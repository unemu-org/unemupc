// SPDX-License-Identifier: AGPL-3.0-or-later
//! The stored config of a PC machine image: its model, its decode and
//! encode, the binding checks and its lowering; machine-images §2.
//!
//! A stored config is the image's config blob
//! (`application/vnd.unemu.config.v1+toml`): the machine's resolved hardware
//! tables with their defaults frozen in ([`encode`], [`defaults`]), and the
//! bindings that tie each firmware ROM, medium and CMOS image the hardware
//! names to the content that holds it, by digest. This module reads no
//! store: what it knows of content comes from the [`ContentRow`]s a caller
//! hands it, and the bytes of small content (firmware ROMs and the CMOS
//! image) through a [`unemu_core_api::ContentSource`]. The store side
//! (`pc-image`) checks descriptors, kinds, size limits and layer chains, fills
//! the rows and calls [`crate::launch::lower`] to validate. See
//! `backlog/docs/design/core-contract.md` §14.2.1 and §14.3.
//!
//! IBM Personal Computer AT Technical Reference, System Board and Diskette
//! Drive Adapter: controller-local units keep their existing wiring. A fixed
//! disk's medium has the drive's exact geometry; a floppy medium has its
//! payload's exact geometry, one of the media the drive type takes
//! (`FloppyGeometry::accepted_media`).

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{fmt, path::PathBuf, str::FromStr};

pub(crate) mod codec;
pub mod defaults;
pub mod encode;

pub use codec::{check_media_fields, check_slot, lower, ContentRow, Lowered};

/// Why a stored config is refused, with the same words `pc-image` has always
/// used for it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The hardware tables do not parse as a machine document.
    #[error(transparent)]
    Parse(#[from] crate::ParseError),
    /// The hardware tables do not lower.
    #[error(transparent)]
    Lower(#[from] crate::LowerError),
    /// The config breaks a rule of the stored format.
    #[error("invalid PC image: {0}")]
    Invalid(String),
}
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// A SHA-256 content address, written `sha256:` and 64 lowercase hex
/// digits: the text form the image store uses for its digests, so a stored
/// config reads and writes the same bytes whichever crate decodes it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// The digest whose SHA-256 is `sha256`.
    pub fn from_sha256(sha256: [u8; 32]) -> Self {
        Self(sha256)
    }

    /// The 32 bytes of the SHA-256.
    pub fn sha256(&self) -> [u8; 32] {
        self.0
    }

    /// Lowercase hexadecimal without the algorithm prefix.
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// A digest text that is not `sha256:` and 64 lowercase hex digits. Its
/// words are the image store's, so a refusal reads as it always has.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid image: digest must be sha256: followed by 64 lowercase hex digits")]
pub struct DigestError;

impl FromStr for Digest {
    type Err = DigestError;

    fn from_str(value: &str) -> std::result::Result<Self, DigestError> {
        let hex = value
            .strip_prefix("sha256:")
            .filter(|hex| {
                hex.len() == 64
                    && hex
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .ok_or(DigestError)?;
        let mut bytes = [0; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| DigestError)?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.hex())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Digest").field(&self.hex()).finish()
    }
}

impl Serialize for Digest {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A stored config: the envelope, the hardware tables and the bindings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub platform: String,
    pub machine: toml::Table,
    #[serde(default)]
    pub firmware: Vec<Firmware>,
    #[serde(default)]
    pub media: Vec<Medium>,
    #[serde(default)]
    pub slots: Vec<Slot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvram: Option<Nvram>,
}

impl Config {
    /// Decode a stored config and its hardware schema from its TOML table.
    pub fn decode(table: toml::Table) -> Result<Self> {
        let config: Self = toml::Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| invalid(e.to_string()))?;
        if config.media.iter().any(|medium| {
            medium.head.is_some() == medium.zero.is_some()
                || medium.zero.as_ref().is_some_and(|zero| zero.size == 0)
        }) {
            return Err(invalid(
                "medium requires exactly one head or positive zero size",
            ));
        }
        // A future envelope may use another hardware grammar; its version is
        // diagnosed by the platform before interpreting those tables.
        if config.schema == 1 {
            let hardware = toml::to_string(&config.machine).map_err(|e| invalid(e.to_string()))?;
            crate::parse_toml("bundle.toml", &hardware)?;
        }
        Ok(config)
    }

    /// Encode the config as the bytes of its blob.
    pub fn encode(&self) -> Result<String> {
        toml::to_string(self).map_err(|e| invalid(e.to_string()))
    }
}

/// The binding of one firmware ROM (or ROM part) of the hardware tables.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Firmware {
    /// The hardware field the ROM sits at, such as `firmware.bios`.
    pub key: String,
    /// The logical asset name the hardware table names.
    pub source: String,
    pub blob: Digest,
    /// The corresponding source archive embedded with an open copyleft ROM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_archive: Option<Digest>,
}

/// One medium of the image, in a slot's catalogue.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Medium {
    #[serde(deserialize_with = "medium_id")]
    pub id: String,
    pub source: String,
    /// Exactly one external head or document-owned zero source is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zero: Option<Zero>,
    pub name: Option<String>,
    pub policy: Policy,
    pub geometry: Geometry,
}

/// A document-owned generated source; content-bindings § Native zero disks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zero {
    pub size: u64,
}
impl Medium {
    /// Source length without opening content or hashing generated bytes.
    pub fn size(&self, rows: &std::collections::BTreeMap<Digest, ContentRow>) -> Result<u64> {
        match (self.head, &self.zero) {
            (Some(head), None) => rows
                .get(&head)
                .map(|r| r.size)
                .ok_or_else(|| invalid(format!("missing content row for {head}"))),
            (None, Some(zero)) if zero.size > 0 => Ok(zero.size),
            _ => Err(invalid(
                "medium requires exactly one head or positive zero size",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Local media persistence intent, independent of a chosen recovery generation.
pub enum Policy {
    Readonly,
    Volatile,
    Journal,
}
impl Policy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Readonly => "readonly",
            Self::Volatile => "volatile",
            Self::Journal => "journal",
        }
    }
    /// The policy of an assembly write mode.
    pub fn from_write(write: &assembly::WriteMode) -> Self {
        match write {
            assembly::WriteMode::ReadOnly => Self::Readonly,
            assembly::WriteMode::Volatile => Self::Volatile,
            assembly::WriteMode::Journal { .. } => Self::Journal,
        }
    }
    /// The assembly write mode of this policy, journalling to `path`.
    pub fn write(self, path: PathBuf) -> assembly::WriteMode {
        match self {
            Self::Readonly => assembly::WriteMode::ReadOnly,
            Self::Volatile => assembly::WriteMode::Volatile,
            Self::Journal => assembly::WriteMode::journal(path),
        }
    }
}

/// A medium's geometry, in 512-byte sectors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub cylinders: u32,
    pub heads: u32,
    pub sectors: u32,
    pub sector_bytes: u32,
}
impl Geometry {
    /// The geometry of a drive of `kind`.
    pub fn of(kind: assembly::DriveKind) -> Self {
        match kind {
            assembly::DriveKind::Floppy { geometry: g } => Self {
                cylinders: g.cylinders,
                heads: g.heads,
                sectors: g.sectors_per_track,
                sector_bytes: g.bytes_per_sector,
            },
            assembly::DriveKind::HardDisk { geometry: g } => Self {
                cylinders: g.cylinders,
                heads: g.heads,
                sectors: g.sectors_per_track,
                sector_bytes: g.bytes_per_sector,
            },
        }
    }

    /// The geometry of a `size`-byte medium in a drive of `kind`: a fixed
    /// disk's own geometry, or the floppy medium of that size the drive type
    /// takes (`FloppyGeometry::accepted_media`: 720K in a 1.44M drive, 360K in
    /// a 1.2M drive).
    pub fn of_medium(kind: assembly::DriveKind, size: u64) -> Result<Self> {
        match kind {
            assembly::DriveKind::Floppy { geometry } => geometry
                .medium_for_len(size)
                .map(|medium| Self::of(assembly::DriveKind::Floppy { geometry: medium }))
                .ok_or_else(|| {
                    invalid(format!(
                        "floppy medium is {size} bytes but {}",
                        assembly::descriptors::floppy_drive_media(geometry)
                    ))
                }),
            assembly::DriveKind::HardDisk { .. } => Ok(Self::of(kind)),
        }
    }

    /// Whether a medium of this geometry fits a drive of `kind` by the same
    /// rule: a fixed disk needs its own geometry, a floppy drive any medium
    /// its type takes.
    pub fn fits(&self, kind: assembly::DriveKind) -> bool {
        match kind {
            assembly::DriveKind::Floppy { geometry } => geometry
                .accepted_media()
                .into_iter()
                .any(|medium| Self::of(assembly::DriveKind::Floppy { geometry: medium }) == *self),
            assembly::DriveKind::HardDisk { .. } => Self::of(kind) == *self,
        }
    }
}

/// The media of one drive: a single image, or a catalogue a person picks from.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub controller: String,
    pub unit: u8,
    pub catalogue: Vec<String>,
    pub catalogue_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inserted: Option<String>,
}

/// The binding of the RTC's CMOS image.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nvram {
    pub mode: NvramMode,
    pub source: String,
    pub head: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NvramMode {
    Image,
    Journal,
}

/// Visit each firmware ROM, or each part of a ROM in parts, of the hardware
/// tables in their binding order, with the field it sits at.
pub fn firmware_images<E: From<Error>>(
    table: &mut toml::Table,
    mut visit: impl FnMut(&str, &mut toml::Table) -> std::result::Result<(), E>,
) -> std::result::Result<(), E> {
    let Some(firmware) = table
        .get_mut("firmware")
        .and_then(toml::Value::as_table_mut)
    else {
        return Ok(());
    };
    let mut roms = Vec::new();
    if let Some(bios) = firmware.get_mut("bios") {
        roms.push(("firmware.bios".to_owned(), bios));
    }
    for (field, rom) in roms {
        visit_rom(&field, rom, &mut visit)?;
    }
    if let Some(roms) = firmware
        .get_mut("option_roms")
        .and_then(toml::Value::as_array_mut)
    {
        for (i, rom) in roms.iter_mut().enumerate() {
            visit_rom(&format!("firmware.option_roms[{i}]"), rom, &mut visit)?;
        }
    }
    Ok(())
}
fn visit_rom<E: From<Error>>(
    field: &str,
    rom: &mut toml::Value,
    visit: &mut impl FnMut(&str, &mut toml::Table) -> std::result::Result<(), E>,
) -> std::result::Result<(), E> {
    let rom = rom
        .as_table_mut()
        .ok_or_else(|| invalid("ROM must be a table"))?;
    if let Some(parts) = rom.get_mut("parts").and_then(toml::Value::as_array_mut) {
        for (i, part) in parts.iter_mut().enumerate() {
            visit(
                &format!("{field}.parts[{i}]"),
                part.as_table_mut()
                    .ok_or_else(|| invalid("ROM part must be a table"))?,
            )?;
        }
    } else {
        visit(field, rom)?;
    }
    Ok(())
}
/// Visit each drive table of the hardware tables in card order, with its
/// controller's id and kind.
pub fn drive_tables<E: From<Error>>(
    machine: &mut toml::Table,
    mut visit: impl FnMut(&str, &str, &mut toml::Table) -> std::result::Result<(), E>,
) -> std::result::Result<(), E> {
    if let Some(cards) = machine.get_mut("card").and_then(toml::Value::as_array_mut) {
        for card in cards {
            let card = card
                .as_table_mut()
                .ok_or_else(|| invalid("card must be a table"))?;
            let kind = card
                .get("kind")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| invalid("missing card kind"))?
                .to_owned();
            let id = card
                .get("id")
                .and_then(toml::Value::as_str)
                .unwrap_or(&kind)
                .to_owned();
            if let Some(drives) = card.get_mut("drives").and_then(toml::Value::as_array_mut) {
                for drive in drives {
                    visit(
                        &id,
                        &kind,
                        drive
                            .as_table_mut()
                            .ok_or_else(|| invalid("drive must be a table"))?,
                    )?;
                }
            }
        }
    }
    Ok(())
}
/// A drive's controller-local unit: an IDE channel's 0 or 1, or the `unit`
/// field of any other drive.
pub fn unit(kind: &str, drive: &toml::Table) -> Result<u8> {
    if kind == "ide" {
        match drive.get("channel").and_then(toml::Value::as_str) {
            Some("master") => Ok(0),
            Some("slave") => Ok(1),
            _ => Err(invalid("invalid IDE channel")),
        }
    } else {
        drive
            .get("unit")
            .and_then(toml::Value::as_integer)
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(|| invalid("missing drive unit"))
    }
}

/// Check the lowered machine's system BIOS against its board, with the
/// permission for unlisted firmware in `options`.
pub fn validate_bios(
    config: &assembly::MachineConfig,
    options: assembly::AssemblyOptions,
) -> Result<()> {
    let bios = config
        .images
        .iter()
        .find(|image| {
            image.kind == assembly::ImageKind::Rom
                && image
                    .source
                    .known_len()
                    .and_then(|n| image.base.checked_add(n))
                    == Some(0x100000)
        })
        .ok_or_else(|| invalid("missing resolved system BIOS"))?;
    let assembly::ImageSource::Inline(bytes) = &bios.source else {
        return Err(invalid("system BIOS must already be resolved"));
    };
    assembly::check_board_system_bios(config, config.board, bios.base, bytes, options)
        .map_err(|e| invalid(e.to_string()))?;
    Ok(())
}

fn medium_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    let id = String::deserialize(deserializer)?;
    if id == "cmos"
        || id.is_empty()
        || id.len() > 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(serde::de::Error::custom("invalid or reserved medium id"));
    }
    Ok(id)
}

#[cfg(test)]
mod tests;
