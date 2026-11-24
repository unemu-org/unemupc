// SPDX-License-Identifier: AGPL-3.0-or-later
//! The launch document: what the PC core reads to build a machine from a
//! stored image.
//!
//! A stored config names its content by the digest of a store object, not
//! by the digest and length of the bytes a machine opens, and it says
//! nothing of the journals a start keeps. The PC provider therefore writes,
//! for each start, a launch document: the stored config's schema unchanged,
//! with the catalogues as this start has them, plus two tables. `content`
//! gives each firmware blob, disk head and CMOS image the config binds the
//! SHA-256 and length of its flat bytes, which is how the contract's
//! `ContentSource` opens content; `record` gives each journalled medium and
//! a journalled CMOS the record of the machine's storage that holds its
//! journal, and its retention. Portable prepared launches may be stored as
//! machine configs; launches augmented with session-kept records are transient. See `backlog/docs/design/core-contract.md` §14.2.2.
//!
//! [`check`] needs no content and is the core's `validate`; [`lower`] reads
//! the firmware and CMOS bytes and names each disk base and each journal by
//! a key, saying what each key stands for. Both run the same checks in the
//! same order, so a document either refuses is refused by the other with
//! the same words, but for what only the bytes of content decide: the
//! pins of the firmware and CMOS bytes, the system BIOS's and the option
//! ROMs' catalogue membership and an option ROM's header and checksum. The
//! store validates a stored config through [`lower`] as well, so one
//! function lowers for the store and for the core.

use crate::image::{self, invalid, Config, Digest, Firmware, Medium, Nvram, Policy, Result, Slot};
use crate::image::{codec, NvramMode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use unemu_core_api::{ContentRef, ContentSource};

mod prepare;
pub use prepare::prepare;

/// The media type of the launch document, the one config type the PC core
/// takes.
pub const MEDIA_TYPE: &str = "application/vnd.unemu.pc.launch.v1+toml";

/// The platform a launch document names: always the current spelling.
/// A retired alias is the store's to accept, before it writes the document.
pub const PLATFORM: &str = "ibm-pc";

/// The longest launch document, in bytes: twice the store's limit for a
/// config blob, room for the two tables beside a config of that size.
pub const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;

/// The largest firmware ROM or ROM part, as the store bounds it. The core
/// reads a ROM whole, so a row may not ask for more.
pub const MAX_FIRMWARE_BYTES: u64 = 16 * 1024 * 1024;
/// The length of a CMOS image: the MC146818's 64 bytes.
pub const CMOS_BYTES: u64 = 64;

/// The `medium` of the record row of a journalled CMOS.
pub const CMOS_MEDIUM: &str = "cmos";
/// The suffix of the record name of a config's own journal: `<id>.upfj`,
/// `cmos.upfj`, the file names a session has always used.
pub const RECORD_SUFFIX: &str = ".upfj";

/// A launch document.
#[derive(Clone, Debug)]
pub struct Launch {
    /// The stored config's schema: its envelope, hardware tables and
    /// bindings, with the catalogues of this start.
    pub config: Config,
    /// One row per firmware blob, disk head and CMOS image `config` binds.
    pub content: Vec<ContentRow>,
    /// One row per journalled medium, and one for a journalled CMOS.
    pub records: Vec<RecordRow>,
}

/// The flat bytes behind one digest of the config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRow {
    /// The blob or head the config names.
    pub digest: Digest,
    /// SHA-256 of the flat bytes the core opens: for a raw blob its own
    /// digest, for a layered head the chain's result. Native zero media
    /// have no content row.
    pub sha256: [u8; 32],
    /// Their length.
    pub size: u64,
}

/// Where the journal of one medium, or of the CMOS, lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordRow {
    /// A medium id of the config, or [`CMOS_MEDIUM`].
    pub medium: String,
    /// The record of the machine's storage: `<medium>.upfj` for the
    /// config's own media and the CMOS, a kept record name
    /// (`unemu_core_api::is_kept_record_name`) for a medium the provider
    /// adds for one start.
    pub name: String,
    /// The closed generations the journal keeps; at least one.
    pub keep_generations: u32,
}

/// A machine lowered from a launch document. Its config names every disk
/// base and journal by a key no host opens: a base by the medium's logical
/// asset name, a journal by `<id>.upfj` or `cmos.upfj`.
#[derive(Debug)]
pub struct Lowered {
    pub machine: assembly::MachineConfig,
    /// Key of an `ImageSource::File` or an NVRAM base: the SHA-256 and the
    /// length of its content.
    pub bases: BTreeMap<PathBuf, ([u8; 32], u64)>,
    /// Key of a `WriteMode::Journal` or the NVRAM journal: its record.
    pub records: BTreeMap<PathBuf, String>,
}

/// The document as it is written: the stored config's fields in their
/// order, then the two tables.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: u32,
    platform: String,
    machine: toml::Table,
    #[serde(default)]
    firmware: Vec<Firmware>,
    #[serde(default)]
    media: Vec<Medium>,
    #[serde(default)]
    slots: Vec<Slot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nvram: Option<Nvram>,
    #[serde(default)]
    content: Vec<ContentLine>,
    #[serde(default)]
    record: Vec<RecordLine>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentLine {
    digest: Digest,
    /// 64 lowercase hex digits.
    sha256: String,
    size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordLine {
    medium: String,
    name: String,
    keep_generations: u32,
}

/// Read a launch document. Only its form is checked here: [`check`] and
/// [`lower`] check what it says.
pub fn decode(bytes: &[u8]) -> Result<Launch> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid(format!(
            "launch document exceeds size limit of {MAX_DOCUMENT_BYTES} bytes"
        )));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?;
    let mut table: toml::Table = toml::from_str(text).map_err(|e| invalid(e.to_string()))?;
    let document: Document = table
        .clone()
        .try_into()
        .map_err(|e: toml::de::Error| invalid(e.to_string()))?;
    if document.schema != 1 {
        return Err(invalid("unsupported PC config envelope"));
    }
    table.remove("content");
    table.remove("record");
    let config = Config::decode(table)?;
    let content = document
        .content
        .into_iter()
        .map(|line| {
            Ok(ContentRow {
                digest: line.digest,
                sha256: sha256_from_hex(&line.sha256)?,
                size: line.size,
            })
        })
        .collect::<Result<_>>()?;
    let records = document
        .record
        .into_iter()
        .map(|line| RecordRow {
            medium: line.medium,
            name: line.name,
            keep_generations: line.keep_generations,
        })
        .collect();
    Ok(Launch {
        config,
        content,
        records,
    })
}

/// Write a launch document in its canonical wire form, which round-trips
/// through [`decode`] byte for byte.
pub fn encode(launch: &Launch) -> Result<Vec<u8>> {
    let config = launch.config.clone();
    let document = Document {
        schema: config.schema,
        platform: config.platform,
        machine: config.machine,
        firmware: config.firmware,
        media: config.media,
        slots: config.slots,
        nvram: config.nvram,
        content: launch
            .content
            .iter()
            .map(|row| ContentLine {
                digest: row.digest,
                sha256: Digest::from_sha256(row.sha256).hex(),
                size: row.size,
            })
            .collect(),
        record: launch
            .records
            .iter()
            .map(|row| RecordLine {
                medium: row.medium.clone(),
                name: row.name.clone(),
                keep_generations: row.keep_generations,
            })
            .collect(),
    };
    let text = toml::to_string(&document).map_err(|e| invalid(e.to_string()))?;
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid(format!(
            "launch document exceeds size limit of {MAX_DOCUMENT_BYTES} bytes"
        )));
    }
    Ok(text.into_bytes())
}

/// Read the portable launch subset used as a stored machine config. Session
/// augmentation uses `decode` instead because kept records belong to one session.
pub fn decode_portable(bytes: &[u8]) -> Result<Launch> {
    let launch = decode(bytes)?;
    check_portable(&launch)?;
    Ok(launch)
}

/// Write a launch for storage without retaining session storage names.
pub fn encode_portable(launch: &Launch) -> Result<Vec<u8>> {
    check_portable(launch)?;
    encode(launch)
}

fn check_portable(launch: &Launch) -> Result<()> {
    check(launch, assembly::AssemblyOptions::default())?;
    if launch.records.iter().any(|row| {
        unemu_core_api::is_kept_record_name(&row.name)
            || row.name != format!("{}{RECORD_SUFFIX}", row.medium)
    }) {
        return Err(invalid(
            "session-kept records are transient and cannot be stored in a portable launch",
        ));
    }
    Ok(())
}

fn sha256_from_hex(text: &str) -> Result<[u8; 32]> {
    let malformed = || invalid("content sha256 must be 64 lowercase hex digits");
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(malformed());
    }
    let mut bytes = [0; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|_| malformed())?;
    }
    Ok(bytes)
}

/// Everything that needs no content: the tables, the envelope, every
/// binding, the slots, each size against its geometry, the lowering in its
/// metadata mode, `MachineConfig::validate_logical` and the option-ROM catalogue.
/// Answers the content a machine of this document opens, which is every
/// `content` row: the firmware and CMOS that [`lower`] reads, and the disk
/// bases and NVRAM base it names for the machine to open. One entry per
/// distinct SHA-256 and length, in the order of the rows.
pub fn check(launch: &Launch, options: assembly::AssemblyOptions) -> Result<Vec<ContentRef>> {
    let checked = run(launch, None, options)?;
    let mut seen = BTreeSet::new();
    Ok(launch
        .content
        .iter()
        .filter(|row| seen.insert((row.sha256, row.size)))
        .map(|row| {
            // Every row is bound: `run` refuses an unused one.
            let label = match checked.roles[&row.digest] {
                codec::Role::Firmware => "firmware",
                codec::Role::Floppy => "floppy",
                codec::Role::FixedDisk => "fixed disk",
                codec::Role::Cmos => "CMOS",
            };
            ContentRef::new(row.sha256, row.size, label)
        })
        .collect())
}

/// The machine of a launch document. Reads each firmware ROM and the CMOS
/// image through `content`, by the SHA-256 and length of its row, and
/// checks them against their pins; opens no disk. `options` carries the
/// permission for unlisted firmware.
pub fn lower(
    launch: &Launch,
    content: &dyn ContentSource,
    options: assembly::AssemblyOptions,
) -> Result<Lowered> {
    let checked = run(launch, Some(content), options)?;
    let config = &launch.config;
    let mut bases = BTreeMap::new();
    let mut records = BTreeMap::new();
    for medium in &config.media {
        if let Some(head) = medium.head {
            let row = checked.rows[&head].clone();
            bases.insert(PathBuf::from(&medium.source), (row.sha256, row.size));
        }
    }
    if let Some(nvram) = config
        .nvram
        .as_ref()
        .filter(|n| matches!(n.mode, NvramMode::Journal))
    {
        let row = checked.rows[&nvram.head].clone();
        bases.insert(PathBuf::from(&nvram.source), (row.sha256, row.size));
    }
    for row in &launch.records {
        records.insert(
            PathBuf::from(format!("{}{RECORD_SUFFIX}", row.medium)),
            row.name.clone(),
        );
    }
    Ok(Lowered {
        machine: checked.machine,
        bases,
        records,
    })
}

/// What [`check`] and [`lower`] share.
struct Checked {
    machine: assembly::MachineConfig,
    rows: BTreeMap<Digest, image::ContentRow>,
    roles: BTreeMap<Digest, codec::Role>,
}

/// Every check of a launch document, in one order for [`check`] (`content`
/// is `None`) and [`lower`]: the doubled content rows, the config's shape
/// and bindings, then the content table against the bindings, then the
/// record table, then the lowering.
fn run(
    launch: &Launch,
    content: Option<&dyn ContentSource>,
    options: assembly::AssemblyOptions,
) -> Result<Checked> {
    let mut rows = BTreeMap::new();
    let mut lengths = BTreeMap::new();
    for row in &launch.content {
        if lengths
            .insert(row.sha256, row.size)
            .is_some_and(|size| size != row.size)
        {
            return Err(invalid("content sha256 has conflicting flat sizes"));
        }
        let flat = image::ContentRow {
            sha256: row.sha256,
            size: row.size,
        };
        if rows.insert(row.digest, flat).is_some() {
            return Err(invalid(format!("doubled content row for {}", row.digest)));
        }
    }
    let config = &launch.config;
    let bound = codec::bind(config, &rows, config.platform == PLATFORM)?;
    let mut roles = BTreeMap::new();
    for (digest, role) in &bound.roles {
        let row = rows
            .get(digest)
            .ok_or_else(|| invalid(format!("missing content row for {digest}")))?;
        check_size(*role, row.size)?;
        roles.entry(*digest).or_insert(*role);
    }
    if let Some(row) = launch
        .content
        .iter()
        .find(|row| !roles.contains_key(&row.digest))
    {
        return Err(invalid(format!("unused content row for {}", row.digest)));
    }
    let retention = check_records(config, &launch.records)?;
    let machine = codec::finish(config, &rows, bound, content, options, Some(&retention))?;
    Ok(Checked {
        machine,
        rows,
        roles,
    })
}

/// A row's length within the PC's limit for what it is bound as, before
/// anything is opened, in the store's words: the core reads firmware and
/// CMOS whole. A medium's row needs no limit of its own: the codec holds its
/// length equal to the medium's geometry, and assembly holds that geometry to
/// what the drive takes (the ATA limit of 16383 cylinders, 16 heads and 63
/// sectors, or a medium of the floppy drive's type), all before a disk is
/// opened.
fn check_size(role: codec::Role, size: u64) -> Result<()> {
    match role {
        codec::Role::Cmos if size != CMOS_BYTES => {
            Err(invalid("CMOS must contain exactly 64 bytes"))
        }
        codec::Role::Firmware if size > MAX_FIRMWARE_BYTES => {
            Err(invalid("payload exceeds PC size limit"))
        }
        _ => Ok(()),
    }
}

/// One record row per journalled medium and per journalled CMOS, each a
/// record name of its own that keeps at least one generation. Answers each
/// journal's retention by medium id or [`CMOS_MEDIUM`].
fn check_records(config: &Config, records: &[RecordRow]) -> Result<BTreeMap<String, u32>> {
    let mut journalled: Vec<&str> = config
        .media
        .iter()
        .filter(|m| m.policy == Policy::Journal)
        .map(|m| m.id.as_str())
        .collect();
    if config
        .nvram
        .as_ref()
        .is_some_and(|n| matches!(n.mode, NvramMode::Journal))
    {
        journalled.push(CMOS_MEDIUM);
    }
    let mut retention = BTreeMap::new();
    let mut names = BTreeSet::new();
    for row in records {
        if !journalled.contains(&row.medium.as_str()) {
            return Err(invalid(format!("unused record row for {}", row.medium)));
        }
        if retention.contains_key(&row.medium) {
            return Err(invalid(format!("doubled record row for {}", row.medium)));
        }
        let own = format!("{}{RECORD_SUFFIX}", row.medium);
        if !unemu_core_api::is_record_name(&row.name)
            || (!unemu_core_api::is_kept_record_name(&row.name) && row.name != own)
        {
            return Err(invalid(format!(
                "the journal of {} cannot be the record {:?}: it is {own} or a kept record name",
                row.medium, row.name
            )));
        }
        if !names.insert(row.name.as_str()) {
            return Err(invalid(format!(
                "record {} holds more than one journal",
                row.name
            )));
        }
        if row.keep_generations == 0 {
            return Err(invalid(format!(
                "the journal of {} must keep at least one generation",
                row.medium
            )));
        }
        retention.insert(row.medium.clone(), row.keep_generations);
    }
    if let Some(missing) = journalled.iter().find(|id| !retention.contains_key(**id)) {
        return Err(invalid(format!("missing record row for {missing}")));
    }
    Ok(retention)
}

#[cfg(test)]
mod tests;
