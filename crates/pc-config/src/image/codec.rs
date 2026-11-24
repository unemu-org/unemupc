// SPDX-License-Identifier: AGPL-3.0-or-later
//! The codec half of a stored config's validation: the envelope, its
//! bindings against the hardware tables, the slots, each medium's size
//! against its geometry, and the lowering. It reads nothing of a store:
//! what it knows of content comes from the caller's rows, and the firmware
//! and CMOS bytes through a `ContentSource`, each opened by the SHA-256 and
//! size of its row. See `backlog/docs/design/core-contract.md` §14.3.
//!
//! The work comes in two halves so that the launch document (`crate::launch`)
//! can check its own tables between them: [`bind`] does every check of the
//! config's shape and binds each digest to its role, [`finish`] lowers the
//! bound tables, reading the small content or, for a check without content,
//! in the lowering's metadata mode.

use super::{
    drive_tables, firmware_images, invalid, unit, validate_bios, Config, Digest, Policy, Result,
    Slot,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};
use unemu_core_api::{Content, ContentSource};

/// What the codec knows of one blob or disk head: the SHA-256 of the flat
/// bytes a machine opens, and their length. The codec reads a disk's row
/// for its size and its pin, and opens firmware and CMOS content by their
/// rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRow {
    /// For a raw blob its own digest; for a layered head the chain's
    /// result; for a zero base the digest of that many zero bytes.
    pub sha256: [u8; 32],
    /// The decoded size of a raw blob. For a layered or zero head, the
    /// capacity of the medium's geometry, which the store's chain walk holds
    /// equal to the chain's root.
    pub size: u64,
}

/// A config that passed every check, and the machine it lowers to.
pub struct Lowered {
    pub config: Config,
    pub machine: assembly::MachineConfig,
}

/// What a digest of the config is bound as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// A firmware ROM or a part of one.
    Firmware,
    /// A medium of a floppy drive.
    Floppy,
    /// The medium of a fixed-disk drive.
    FixedDisk,
    /// The CMOS image.
    Cmos,
}

/// A config whose shape passed every check: its hardware tables with
/// placeholders where media sit, each logical asset name with its digest,
/// and each digest with what it is bound as, in binding order.
pub(crate) struct Bound {
    hardware: toml::Table,
    bindings: BTreeMap<String, (Digest, bool)>,
    pub(crate) roles: Vec<(Digest, Role)>,
}

/// The path a fixed disk lowers from while its size is checked: a disk
/// payload is never read in validation.
const VALIDATION_DISK: &str = "__pc_image_validation_disk__";
struct Reader<'a> {
    /// `None` for a check without content, which lowers in the metadata
    /// mode and so reads nothing.
    content: Option<&'a dyn ContentSource>,
    rows: &'a BTreeMap<Digest, ContentRow>,
    /// Logical asset name to its digest, and whether it is small content.
    bindings: BTreeMap<String, (Digest, bool)>,
}
impl Reader<'_> {
    /// The bytes of a firmware ROM or CMOS image, opened by its row. Content
    /// of another length than the row's is refused before it is read, so a
    /// source can neither make the codec allocate a size of its choosing nor
    /// hand it other bytes than the row names.
    fn small(&self, digest: &Digest) -> io::Result<Vec<u8>> {
        let row = self
            .rows
            .get(digest)
            .ok_or_else(|| io::Error::other("unbound logical asset"))?;
        let content = self
            .content
            .ok_or_else(|| io::Error::other("a check reads no content"))?;
        match content
            .open(&row.sha256, row.size)
            .map_err(io::Error::other)?
        {
            Content::Bytes(bytes) if bytes.len() as u64 == row.size => Ok(bytes),
            Content::Bytes(_) => Err(io::Error::other("content size differs from its row")),
            Content::Reader(reader) => {
                if reader.size() != row.size {
                    return Err(io::Error::other("content size differs from its row"));
                }
                let len = usize::try_from(row.size)
                    .map_err(|_| io::Error::other("small content exceeds memory"))?;
                let mut bytes = vec![0; len];
                reader.read_at(0, &mut bytes).map_err(io::Error::other)?;
                Ok(bytes)
            }
            _ => Err(io::Error::other("unsupported content")),
        }
    }
}
impl crate::ImageReader for Reader<'_> {
    fn validate_machine(
        &self,
        config: &assembly::MachineConfig,
    ) -> std::result::Result<(), assembly::ConfigError> {
        config.validate_logical()
    }

    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        if path == Path::new(VALIDATION_DISK) {
            return Ok(vec![0; 512]);
        }
        let (digest, small) = self
            .bindings
            .get(&path.to_string_lossy().to_string())
            .ok_or_else(|| io::Error::other("unbound logical asset"))?;
        if !small {
            return Err(io::Error::other("disk payload must stay file-backed"));
        }
        let digest = *digest;
        self.small(&digest)
    }

    /// A ROM's length from its row, for the metadata mode of a check.
    fn image_length(&self, path: &Path) -> io::Result<u64> {
        self.bindings
            .get(&path.to_string_lossy().to_string())
            .and_then(|(digest, _)| self.rows.get(digest))
            .map(|row| row.size)
            .ok_or_else(|| io::Error::other("unbound logical asset"))
    }
}
fn logical(name: &str) -> Result<()> {
    if !name.starts_with("assets/")
        || name.len() > 200
        || name
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
    {
        return Err(invalid("invalid logical asset name"));
    }
    Ok(())
}
fn bind_name(
    bindings: &mut BTreeMap<String, (Digest, bool)>,
    source: &str,
    digest: &Digest,
    small: bool,
) -> Result<()> {
    logical(source)?;
    if bindings.contains_key(source) {
        return Err(invalid("duplicate logical asset name"));
    }
    bindings.insert(source.into(), (*digest, small));
    Ok(())
}
fn row<'a>(rows: &'a BTreeMap<Digest, ContentRow>, digest: &Digest) -> Result<&'a ContentRow> {
    rows.get(digest)
        .ok_or_else(|| invalid(format!("missing content row for {digest}")))
}

/// A stored drive table names no medium: the slots do.
pub fn check_media_fields(drive: &toml::Table) -> Result<()> {
    if [
        "disk",
        "inserted",
        "image",
        "sha256",
        "media",
        "empty",
        "write",
        "journal",
        "keep_generations",
    ]
    .iter()
    .any(|k| drive.contains_key(*k))
    {
        return Err(invalid("bundle hardware contains media fields"));
    }
    Ok(())
}
/// A slot sits at its drive's position and has the shape its drive takes.
/// The store's walk stops where this refuses, so that the codec's sentence
/// is the one a person reads.
pub fn check_slot(controller: &str, kind: &str, drive: &toml::Table, slot: &Slot) -> Result<()> {
    if slot.controller != controller || slot.unit != unit(kind, drive)? {
        return Err(invalid("slot binding differs from hardware position"));
    }
    if slot
        .inserted
        .as_ref()
        .is_some_and(|id| slot.catalogue.first() != Some(id))
    {
        return Err(invalid("unsupported initial selection"));
    }
    if kind == "ide"
        && (slot.catalogue_mode || slot.catalogue.len() != 1 || slot.inserted.is_none())
    {
        return Err(invalid("IDE needs exactly one inserted medium"));
    }
    if !slot.catalogue_mode && (slot.catalogue.len() != 1 || slot.inserted.is_none()) {
        return Err(invalid("single image slot requires one inserted medium"));
    }
    Ok(())
}

/// Lower `config`. `rows` holds one row per firmware blob, disk head and
/// CMOS image the config binds; `content` serves the firmware and CMOS rows.
/// `platform_known` is whether the config's platform names the PC, the
/// alias check the caller makes; `options` carries the permission for
/// unlisted firmware.
pub fn lower(
    config: Config,
    rows: &BTreeMap<Digest, ContentRow>,
    content: &dyn ContentSource,
    platform_known: bool,
    options: assembly::AssemblyOptions,
) -> Result<Lowered> {
    if config.media.iter().any(|medium| {
        medium
            .name
            .as_ref()
            .is_some_and(|name| name.contains(['/', '\\', '\0']) || name.is_empty())
    }) {
        return Err(invalid("invalid medium name"));
    }
    let bound = bind_with_selection(&config, rows, platform_known, false)?;
    let machine = finish(&config, rows, bound, Some(content), options, None)?;
    Ok(Lowered { config, machine })
}

/// The first half of [`lower`]: the envelope, the hardware tables' form,
/// every firmware, medium and CMOS binding against the hardware, the slots
/// and each medium's size against its geometry. Reads nothing.
pub(crate) fn bind(
    config: &Config,
    rows: &BTreeMap<Digest, ContentRow>,
    platform_known: bool,
) -> Result<Bound> {
    bind_with_selection(config, rows, platform_known, true)
}

fn bind_with_selection(
    config: &Config,
    rows: &BTreeMap<Digest, ContentRow>,
    platform_known: bool,
    launch_selection: bool,
) -> Result<Bound> {
    if config.schema != 1 || !platform_known {
        return Err(invalid("unsupported PC config envelope"));
    }
    let mut hardware = config.machine.clone();
    let raw_text = toml::to_string(&hardware).map_err(|e| invalid(e.to_string()))?;
    let document = crate::parse_toml("bundle.toml", &raw_text)?;
    crate::bindings::validate_sources(&document, &crate::Origins::new("bundle.toml"))?;
    if crate::bindings::has_bindings(&document) {
        return Err(invalid(
            "unresolved content bindings; composition is required",
        ));
    }
    if ["extends", "name", "description"]
        .iter()
        .any(|k| hardware.contains_key(*k))
    {
        return Err(invalid(
            "bundle hardware must be resolved and contain no descriptive fields",
        ));
    }
    let mut bindings = BTreeMap::new();
    let mut roles = Vec::new();
    let mut firmware_count = 0;
    firmware_images(&mut hardware, |field, rom| {
        let binding = config
            .firmware
            .get(firmware_count)
            .ok_or_else(|| invalid("missing firmware binding"))?;
        if binding.key != field
            || rom.get("image").and_then(toml::Value::as_str) != Some(&binding.source)
        {
            return Err(invalid("firmware binding differs from hardware"));
        }
        bind_name(&mut bindings, &binding.source, &binding.blob, true)?;
        roles.push((binding.blob, Role::Firmware));
        if rom.get("sha256").and_then(toml::Value::as_str) != Some(binding.blob.hex().as_str()) {
            return Err(invalid("firmware hash differs from binding"));
        }
        firmware_count += 1;
        Ok(())
    })?;
    if firmware_count != config.firmware.len() {
        return Err(invalid("unreferenced firmware binding"));
    }
    let mut used = BTreeSet::new();
    let mut slot_index = 0;
    let mut ids = BTreeSet::new();
    if config
        .media
        .iter()
        .any(|m| !ids.insert(m.id.clone()) || m.id.is_empty())
    {
        return Err(invalid("duplicate or empty medium id"));
    }
    drive_tables(&mut hardware, |controller, kind, drive| {
        check_media_fields(drive)?;
        let slot = config
            .slots
            .get(slot_index)
            .ok_or_else(|| invalid("missing slot binding"))?;
        let mut checked_slot = slot.clone();
        // A kept catalogue restores its selected disk without moving it.
        if launch_selection {
            if slot
                .inserted
                .as_ref()
                .is_some_and(|id| !slot.catalogue.contains(id))
            {
                return Err(invalid("unsupported initial selection"));
            }
            checked_slot.inserted = slot.inserted.as_ref().and(slot.catalogue.first()).cloned();
        }
        check_slot(controller, kind, drive, &checked_slot)?;
        for id in &slot.catalogue {
            let medium = config
                .media
                .iter()
                .find(|m| &m.id == id)
                .ok_or_else(|| invalid("unknown catalogue medium"))?;
            if used.insert(id.clone()) {
                bind_name(&mut bindings, &medium.source, &medium.head, false)?;
                let role = if kind == "ide" {
                    Role::FixedDisk
                } else {
                    Role::Floppy
                };
                roles.push((medium.head, role));
            } else if !matches!(medium.policy, Policy::Readonly) {
                return Err(invalid("writable medium bound more than once"));
            }
            let disk_size = row(rows, &medium.head)?.size;
            let g = &medium.geometry;
            if g.sector_bytes != 512
                || u64::from(g.cylinders)
                    .checked_mul(u64::from(g.heads))
                    .and_then(|n| n.checked_mul(u64::from(g.sectors)))
                    .and_then(|n| n.checked_mul(512))
                    != Some(disk_size)
            {
                return Err(invalid("medium geometry differs from payload length"));
            }
            // Launch-only labels keep the text accepted by live inserts.
            if medium
                .name
                .as_ref()
                .is_some_and(|n| n.contains('\0') || n.is_empty())
            {
                return Err(invalid("invalid medium name"));
            }
            if slot.catalogue_mode != medium.name.is_some() {
                return Err(invalid("medium name differs from catalogue shape"));
            }
            if kind == "ide" {
                let expected = toml::Table::from_iter([
                    ("cylinders".into(), i64::from(g.cylinders).into()),
                    ("heads".into(), i64::from(g.heads).into()),
                    ("sectors".into(), i64::from(g.sectors).into()),
                ]);
                if drive.get("geometry").and_then(toml::Value::as_table) != Some(&expected) {
                    return Err(invalid("medium geometry differs from hardware"));
                }
            }
        }
        // Lower only a bounded placeholder; the medium's geometry is checked
        // above against the row's size, and a disk is never read here. The
        // store verifies raw payloads; its resolved callback verifies a
        // layered head once.
        if kind == "ide" {
            drive.insert("image".into(), VALIDATION_DISK.into());
            drive.insert("write".into(), "volatile".into());
            drive.insert(
                "geometry".into(),
                toml::Table::from_iter([
                    ("cylinders".into(), 1.into()),
                    ("heads".into(), 1.into()),
                    ("sectors".into(), 1.into()),
                ])
                .into(),
            );
        } else {
            drive.insert("media".into(), Vec::<toml::Value>::new().into());
            drive.insert("empty".into(), true.into());
        }
        slot_index += 1;
        Ok(())
    })?;
    if used.len() != config.media.len() || slot_index != config.slots.len() {
        return Err(invalid("unreferenced medium or slot binding"));
    }
    let rtc_nvram = hardware
        .get_mut("board")
        .and_then(toml::Value::as_table_mut)
        .and_then(|b| b.get_mut("rtc"))
        .and_then(toml::Value::as_table_mut)
        .and_then(|rtc| rtc.get_mut("nvram"));
    if let Some(nvram) = &config.nvram {
        let target = rtc_nvram.ok_or_else(|| invalid("missing hardware NVRAM"))?;
        let expected = toml::Table::from_iter([("image".into(), nvram.source.clone().into())]);
        if target.as_table() != Some(&expected) {
            return Err(invalid("NVRAM hardware differs from binding"));
        }
        bind_name(&mut bindings, &nvram.source, &nvram.head, true)?;
        roles.push((nvram.head, Role::Cmos));
        let mut table = expected;
        table.insert("sha256".into(), nvram.head.hex().into());
        table.insert("write".into(), "volatile".into());
        *target = table.into();
    } else if rtc_nvram.is_some_and(|n| n.as_str().is_none()) {
        return Err(invalid("unbound hardware NVRAM"));
    }
    Ok(Bound {
        hardware,
        bindings,
        roles,
    })
}

/// The second half of [`lower`]: lower the bound tables, put each slot's
/// media where its drive is, and check the machine. With `content` the
/// firmware and CMOS are read and checked against their pins and the
/// system BIOS against its board; without it the lowering runs in its
/// metadata mode, which reads nothing and sizes each ROM from its row, and
/// what only those bytes decide is left to a lowering with content.
///
/// `retention`, which a launch document gives, holds each journal's
/// retention by medium id and `cmos`; with it, a journalled CMOS lowers to
/// an NVRAM journal over its image instead of a copy of its bytes. Every
/// key the machine names is the config's own: a medium's logical asset name
/// for its image, `<id>.upfj` for its journal, the CMOS binding's logical
/// asset name and `cmos.upfj`.
pub(crate) fn finish(
    config: &Config,
    rows: &BTreeMap<Digest, ContentRow>,
    bound: Bound,
    content: Option<&dyn ContentSource>,
    options: assembly::AssemblyOptions,
    retention: Option<&BTreeMap<String, u32>>,
) -> Result<assembly::MachineConfig> {
    let mut reader = Reader {
        content,
        rows,
        bindings: bound.bindings,
    };
    let text = toml::to_string(&bound.hardware).map_err(|e| invalid(e.to_string()))?;
    let document = crate::parse_toml("bundle.toml", &text)?;
    let origins = crate::Origins::new("bundle.toml");
    let mut machine = if content.is_some() {
        crate::lower(&document, &origins, &mut reader)?
    } else {
        crate::lower::lower_metadata(&document, &origins, &mut reader)?
    };
    let keep = |id: &str| {
        retention
            .and_then(|map| map.get(id).copied())
            .unwrap_or(assembly::WriteMode::DEFAULT_KEEP_GENERATIONS)
    };
    for (drive, slot) in machine.drives.iter_mut().zip(&config.slots) {
        if drive.unit != slot.unit {
            return Err(invalid("slot order differs from effective drive order"));
        }
        let mut entries = Vec::new();
        for id in &slot.catalogue {
            let m = config
                .media
                .iter()
                .find(|m| &m.id == id)
                .ok_or_else(|| invalid("missing medium"))?;
            if matches!(drive.kind, assembly::DriveKind::HardDisk { .. }) {
                drive.kind = assembly::DriveKind::HardDisk {
                    geometry: assembly::HardDiskGeometry {
                        cylinders: m.geometry.cylinders,
                        heads: m.geometry.heads,
                        sectors_per_track: m.geometry.sectors,
                        bytes_per_sector: m.geometry.sector_bytes,
                    },
                };
            }
            if !m.geometry.fits(drive.kind) {
                return Err(invalid("medium geometry differs from hardware"));
            }
            let mut write = m.policy.write(format!("{}.upfj", m.id).into());
            if let assembly::WriteMode::Journal {
                keep_generations, ..
            } = &mut write
            {
                *keep_generations = keep(&m.id);
            }
            entries.push(assembly::FloppyMedium {
                image: assembly::ImageSource::File(m.source.clone().into()),
                expected_sha256: Some(row(rows, &m.head)?.sha256),
                write,
                name: m.name.clone().unwrap_or_default(),
            });
        }
        if slot.catalogue_mode {
            drive.media = Some(assembly::FloppyMediaList {
                entries,
                empty: slot.inserted.is_none(),
                inserted: slot
                    .inserted
                    .as_ref()
                    .and_then(|id| slot.catalogue.iter().position(|m| m == id))
                    .unwrap_or(0),
            });
            drive.expected_sha256 = None;
            drive.write = assembly::WriteMode::Volatile;
            drive.image = assembly::ImageSource::Blank {
                size: drive.kind.image_bytes(),
            };
        } else {
            let entry = entries
                .pop()
                .ok_or_else(|| invalid("empty single image slot"))?;
            drive.media = None;
            drive.image = entry.image;
            drive.expected_sha256 = entry.expected_sha256;
            drive.write = entry.write;
        }
    }
    if let Some(nvram) = config
        .nvram
        .as_ref()
        .filter(|n| retention.is_some() && matches!(n.mode, super::NvramMode::Journal))
    {
        let sha256 = row(rows, &nvram.head)?.sha256;
        for device in &mut machine.devices {
            if let assembly::DeviceKind::Rtc(rtc) = &mut device.kind {
                rtc.nvram = assembly::NvramSource::Journal {
                    image: nvram.source.clone().into(),
                    expected_sha256: sha256,
                    journal: "cmos.upfj".into(),
                    keep_generations: keep("cmos"),
                };
            }
        }
    }
    machine
        .validate_logical()
        .map_err(|e| invalid(e.to_string()))?;
    assembly::check_option_rom_catalogue(&machine, assembly::AssemblyOptions::default())
        .map_err(|e| invalid(e.to_string()))?;
    if content.is_some() {
        validate_bios(&machine, options)?;
    }
    Ok(machine)
}
