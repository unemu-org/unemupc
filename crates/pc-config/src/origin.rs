// SPDX-License-Identifier: AGPL-3.0-or-later
//! Format-neutral field origins for machine-file diagnostics.
//!
//! See `backlog/docs/design/machine-files.md` § Errors. Table positions are
//! supplemented by per-value origins when resolving inheritance.

use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use serde::{de::IgnoredAny, Deserialize};

use crate::ParseError;

/// The defining file and optional one-based line of a field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub file: PathBuf,
    pub line: Option<usize>,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.display())?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        Ok(())
    }
}

/// Field sources and canonical image bases retained across layer merges.
#[derive(Clone, Debug)]
pub struct Origins {
    document: Origin,
    image_file: PathBuf,
    fields: BTreeMap<String, Origin>,
    image_files: BTreeMap<PathBuf, PathBuf>,
    logical_images: BTreeMap<String, PathBuf>,
}

impl Origins {
    /// Origins for a programmatically constructed document without positions.
    pub fn new(file: impl Into<PathBuf>) -> Self {
        let file = file.into();
        Self {
            image_file: file.clone(),
            document: Origin { file, line: None },
            fields: BTreeMap::new(),
            image_files: BTreeMap::new(),
            logical_images: BTreeMap::new(),
        }
    }

    /// Resolve this layer's images against `file`, the layer's own file
    /// once symbolic links are followed, and keep that base when other
    /// layers merge with it.
    pub fn set_image_file(&mut self, file: PathBuf) {
        self.image_files
            .insert(self.document.file.clone(), file.clone());
        self.image_file = file;
    }

    /// The files of every merged layer, without presets: the inputs a
    /// journal must not overwrite.
    pub fn layer_files(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.image_file.as_path())
            .chain(self.image_files.values().map(PathBuf::as_path))
            .filter(|file| !file.to_string_lossy().starts_with("preset:"))
    }

    /// Resolve an asset against its defining layer, including preset and symlink origins.
    pub fn image_path(&self, field: &str, image: &str) -> PathBuf {
        if let Some(key) = self.logical_images.get(field) {
            return key.clone();
        }
        let origin = self.get(field);
        let file = if let Some(file) = self.image_files.get(&origin.file) {
            file
        } else if origin.file == self.document.file {
            &self.image_file
        } else {
            &origin.file
        };
        file.parent().unwrap_or(Path::new(".")).join(image)
    }

    /// Keep a generated logical key independent of its selector's source directory.
    pub fn bind_image_key(&mut self, field: &str, key: &str) {
        self.logical_images.insert(field.into(), key.into());
    }

    /// Take the child layer's document and image bases when merging it over
    /// this one.
    pub fn use_document(&mut self, child: &Self) {
        self.document = child.document.clone();
        self.image_file = child.image_file.clone();
        self.image_files.extend(child.image_files.clone());
    }

    /// Whether an origin is recorded for exactly `field`.
    pub fn has_field(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }

    /// Forget the origins of `prefix` and of everything under it.
    pub fn remove_subtree(&mut self, prefix: &str) {
        let keys = self
            .fields
            .range(prefix.to_owned()..)
            .take_while(|(key, _)| within(key, prefix))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.fields.remove(&key);
        }
    }

    /// Replace the origins under `to` with those `source` holds under `from`.
    pub fn copy_subtree(&mut self, source: &Self, from: &str, to: &str) {
        self.remove_subtree(to);
        self.insert(to, source.get(from).clone());
        for (key, origin) in source
            .fields
            .range(from.to_owned()..)
            .take_while(|(key, _)| within(key, from))
        {
            let suffix = &key[from.len()..];
            let mapped = if to.is_empty() {
                suffix.trim_start_matches('.').to_owned()
            } else {
                format!("{to}{suffix}")
            };
            self.insert(mapped, origin.clone());
        }
    }

    /// Renumber the origins of the array `prefix`: entry `i` moves to
    /// `indices[i]`, or is forgotten when that is `None`.
    pub fn remap_entries(&mut self, prefix: &str, indices: &[Option<usize>]) {
        let prefix = format!("{prefix}[");
        let entries = self
            .fields
            .range(prefix.clone()..)
            .take_while(|(key, _)| key.starts_with(&prefix))
            .map(|(key, origin)| (key.clone(), origin.clone()))
            .collect::<Vec<_>>();
        for (key, _) in &entries {
            self.fields.remove(key);
        }
        for (key, origin) in entries {
            let tail = &key[prefix.len()..];
            let Some((index, suffix)) = tail.split_once(']') else {
                continue;
            };
            if let Some(new_index) = index
                .parse::<usize>()
                .ok()
                .and_then(|index| indices.get(index))
                .copied()
                .flatten()
            {
                self.insert(format!("{prefix}{new_index}]{suffix}"), origin);
            }
        }
    }

    /// Record a defining source, including a different file for inherited fields.
    pub fn insert(&mut self, field: impl Into<String>, origin: Origin) {
        self.fields.insert(field.into(), origin);
    }

    /// Look up the closest containing field, falling back to the document.
    pub fn get(&self, field: &str) -> &Origin {
        let mut end = field.len();
        loop {
            if let Some(origin) = self.fields.get(&field[..end]) {
                return origin;
            }
            let Some(parent) = field[..end].rfind(['.', '[']) else {
                return &self.document;
            };
            end = parent;
        }
    }

    fn record_pins(&mut self, input: &str) -> Result<(), ParseError> {
        let mut values = Self::new(self.document.file.clone());
        crate::source::record_toml(&self.document.file, input, &mut values)?;
        self.keep_pins(values);
        Ok(())
    }

    /// Keep, of the per-value origins of one whole layer, those that
    /// diagnostics name beside a table's position: pins, firmware offsets
    /// and parts, CPU clocks, coprocessor and engine, mouse attachments and the display switch. Each syntax's
    /// table-origins decoder ends with it.
    pub fn keep_pins(&mut self, values: Origins) {
        for (field, origin) in values.fields {
            if field.ends_with(".sha256")
                || (field.starts_with("firmware.") && field.ends_with(".offset"))
                || field.starts_with("firmware.bios.parts")
                || field.ends_with(".mouse")
                || field == "board.display_switch"
                || field == "cpu.fpu"
                || field == "cpu.engine"
                || field == "cpu.clock_mhz"
                || field == "cpu.turbo.clock_mhz"
                || field == "cpu.rate"
                || field == "disks"
                || field.starts_with("disks[")
                || field.ends_with(".use")
                || field.ends_with(".rom")
                || field.ends_with(".role")
                || field.ends_with(".size")
                || field.ends_with(".disk")
                || field.contains(".inserted")
                || field.contains(".media[")
            {
                self.insert(field, origin);
            }
        }
    }

    /// Retain TOML table spans separately from the format-independent schema.
    pub fn from_toml(file: impl Into<PathBuf>, input: &str) -> Result<Self, ParseError> {
        #[derive(Deserialize, Default)]
        struct Tables {
            board: Option<toml::Spanned<Board>>,
            cpu: Option<toml::Spanned<IgnoredAny>>,
            memory: Option<toml::Spanned<IgnoredAny>>,
            firmware: Option<toml::Spanned<Firmware>>,
            #[serde(default)]
            card: Vec<toml::Spanned<Card>>,
        }
        #[derive(Deserialize)]
        struct Board {
            rtc: Option<toml::Spanned<IgnoredAny>>,
        }
        #[derive(Deserialize)]
        struct Firmware {
            bios: Option<toml::Spanned<IgnoredAny>>,
            #[serde(default)]
            option_roms: Vec<toml::Spanned<IgnoredAny>>,
        }
        #[derive(Deserialize)]
        struct Card {
            #[serde(default)]
            drives: Vec<toml::Spanned<IgnoredAny>>,
        }
        let mut origins = Self::new(file);
        let tables: Tables = toml::from_str(input)
            .map_err(|error| ParseError::from_toml(&origins.document.file, input, error))?;
        let lines = crate::source::LineIndex::new(input);
        let mut record = |field: String, start: usize| {
            let start = start + input[start..].len() - input[start..].trim_start().len();
            origins.insert(
                field,
                Origin {
                    file: origins.document.file.clone(),
                    line: Some(lines.line(start)),
                },
            );
        };
        if let Some(board) = tables.board {
            record("board".into(), board.span().start);
            if let Some(rtc) = board.into_inner().rtc {
                record("board.rtc".into(), rtc.span().start);
            }
        }
        if let Some(cpu) = tables.cpu {
            record("cpu".into(), cpu.span().start);
        }
        if let Some(memory) = tables.memory {
            record("memory".into(), memory.span().start);
        }
        if let Some(firmware) = tables.firmware {
            record("firmware".into(), firmware.span().start);
            let firmware = firmware.into_inner();
            if let Some(bios) = firmware.bios {
                record("firmware.bios".into(), bios.span().start);
            }
            for (index, rom) in firmware.option_roms.into_iter().enumerate() {
                record(format!("firmware.option_roms[{index}]"), rom.span().start);
            }
        }
        for (index, card) in tables.card.into_iter().enumerate() {
            record(format!("card[{index}]"), card.span().start);
            for (drive_index, drive) in card.into_inner().drives.into_iter().enumerate() {
                record(
                    format!("card[{index}].drives[{drive_index}]"),
                    drive.span().start,
                );
            }
        }
        origins.record_pins(input)?;
        Ok(origins)
    }
}

fn within(key: &str, prefix: &str) -> bool {
    key == prefix
        || key
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('.') || tail.starts_with('['))
}
