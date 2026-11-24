// SPDX-License-Identifier: AGPL-3.0-or-later
//! The PC config codec below the contract: the machine schema, its TOML
//! decoder and its lowering to `assembly::MachineConfig`.
//!
//! This is the part of a machine description that an embedder links with the
//! PC core, so it touches no host: it opens no file, reads no environment and
//! asks the host nothing. Every image the lowering needs comes through an
//! [`ImageReader`] the caller supplies, and the caller decides what a path
//! means. `machine-file` adds what touches the host on top of it (the YAML
//! reader, `extends` and presets, firmware discovery, the host reader
//! `FileImageReader` and the loaders) and re-exports every name of this crate
//! at its old path. See `backlog/docs/design/core-contract.md` §14.2.1 and
//! `backlog/docs/design/machine-files.md` § Schema.
//!
//! The stored config of a PC machine image, its decode and encode and the
//! codec half of its validation are [`image`]'s: they read content through
//! the contract's `ContentSource` and name nothing of a store. The launch
//! document the PC core builds a machine from, a stored config with the
//! content and records of one start, is [`launch`]'s.
//!
//! Documents decode into schema-versioned layers ([`schema`]) whose omitted
//! fields stay omitted, so the merge of `machine-file` can tell inheritance
//! from an explicit override. Complete documents lower to
//! `assembly::MachineConfig` with board and card defaults and with each image
//! checked against its SHA-256 pin; assembly owns hardware validation.

#![forbid(unsafe_code)]

use std::{fmt, ops::Range, path::Path, path::PathBuf};

use serde::Deserialize;

pub mod bindings;
pub mod compat;
pub mod image;
mod integrity;
pub mod launch;
mod lower;
mod origin;
pub mod pc_names;
pub mod schema;
pub mod source;

// The fixture lookup of the tests of `image` and `launch`.
#[cfg(test)]
#[path = "../../pc-harness/tests/support/fixtures.rs"]
mod fixtures;

pub use integrity::{lower, lower_with_options};
#[cfg(feature = "test-support")]
pub use lower::has_retention_marker;
pub use lower::{lower_tracked, AssemblyDiagnostics, ImageReader, JournalBase, LowerError};
pub use origin::{Origin, Origins};

pub use schema::v1::MachineFile;

/// The largest machine document, in bytes, that either syntax accepts.
pub const MAX_MACHINE_FILE_BYTES: usize = 1_048_576;

/// Refuse a document longer than [`MAX_MACHINE_FILE_BYTES`], naming `path`.
pub fn check_size(path: &Path, length: usize) -> Result<(), ParseError> {
    if length > MAX_MACHINE_FILE_BYTES {
        return Err(ParseError::new(
            path,
            None,
            "machine file exceeds size limit of 1048576 bytes",
            None,
        ));
    }
    Ok(())
}

/// A format-neutral diagnostic with optional source position and snippet.
#[derive(Debug, thiserror::Error)]
pub struct ParseError {
    path: PathBuf,
    line_column: Option<(usize, usize)>,
    message: String,
    snippet: Option<String>,
    span: Option<Range<usize>>,
}

impl ParseError {
    /// Construct a diagnostic without retaining a format-specific error chain.
    pub fn new(
        path: impl Into<PathBuf>,
        line_column: Option<(usize, usize)>,
        message: impl Into<String>,
        snippet: Option<String>,
    ) -> Self {
        Self {
            path: path.into(),
            line_column,
            message: message.into(),
            snippet,
            span: None,
        }
    }

    pub(crate) fn from_toml(path: &Path, input: &str, source: toml::de::Error) -> Self {
        let span = source.span();
        let prefix = span.as_ref().and_then(|span| input.get(..span.start));
        let line_column = prefix.and_then(|prefix| {
            let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
            let column = prefix.rsplit('\n').next()?.chars().count() + 1;
            Some((line, column))
        });
        let snippet = prefix.and_then(|prefix| {
            let start = prefix.rfind('\n').map_or(0, |index| index + 1);
            let line = input.get(start..)?.split('\n').next()?;
            let column = prefix.get(start..)?.chars().count();
            Some(source_snippet(line, column))
        });
        let mut error = Self::new(path, line_column, source.message(), snippet);
        error.span = span;
        error
    }

    /// The path supplied by the caller; parsing never resolves it.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One-based line and character column, when the parser supplies a position.
    pub fn line_column(&self) -> Option<(usize, usize)> {
        self.line_column
    }

    /// The diagnostic without file, position or snippet decoration.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// A bounded source window and a caret, when available.
    pub fn snippet(&self) -> Option<&str> {
        self.snippet.as_deref()
    }

    /// The parser's byte range, when available, including missing-field spans.
    pub fn span(&self) -> Option<Range<usize>> {
        self.span.clone()
    }
}

/// A bounded window of one source line around a zero-based character
/// `column`, and a caret under that column: the snippet of a [`ParseError`]
/// in either syntax.
pub fn source_snippet(line: &str, column: usize) -> String {
    const CONTEXT: usize = 40;

    let line = line.strip_suffix('\r').unwrap_or(line);
    let length = line.chars().count();
    let column = column.min(length);
    let first = column.saturating_sub(CONTEXT);
    let last = column.saturating_add(CONTEXT + 1).min(length);
    let mut text = String::new();
    let mut padding = String::new();
    if first > 0 {
        text.push_str("...");
        padding.push_str("   ");
    }
    for (index, character) in line.chars().enumerate().skip(first).take(last - first) {
        text.push(character);
        if index < column {
            padding.push(if character == '\t' { '\t' } else { ' ' });
        }
    }
    if last < length {
        text.push_str("...");
    }
    format!("{text}\n{padding}^")
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.path.display())?;
        if let Some((line, column)) = self.line_column {
            write!(f, "line {line}, column {column}: ")?;
        }
        f.write_str(&self.message)?;
        if let Some(snippet) = &self.snippet {
            write!(f, "\n{snippet}")?;
        }
        Ok(())
    }
}

/// Parse one TOML layer without I/O, merging, defaults or hardware validation.
///
/// The version peek intentionally ignores other fields, so a newer schema is
/// diagnosed before its fields are interpreted using the current schema.
pub fn parse_toml(path: impl AsRef<Path>, input: &str) -> Result<MachineFile, ParseError> {
    check_size(path.as_ref(), input.len())?;
    #[derive(Deserialize)]
    struct SchemaPeek {
        schema: schema::v1::SchemaVersion,
    }

    let parse = || -> Result<MachineFile, toml::de::Error> {
        let peek: SchemaPeek = toml::from_str(input)?;
        let _version = peek.schema;
        toml::from_str(input)
    };
    parse().map_err(|source| ParseError::from_toml(path.as_ref(), input, source))
}
