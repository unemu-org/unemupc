// SPDX-License-Identifier: AGPL-3.0-or-later
//! Positioned content planning for schema-1 PC hardware; content-bindings § Discovery.
//!
//! Planning and fact resolution never access content. Resolved selectors become
//! logical image keys for the existing lowerer; native zero mounting and prepared
//! envelope writing attach to the resolved rows in later stages.

use crate::{schema::v1::*, ImageReader, JournalBase, LowerError, MachineFile, Origins};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};
use unemu_core_api::{
    BindingKind, BindingSlot, BindingUse, Content, ContentSource, Diagnostic, Error, ErrorKind,
};

fn source_error(origins: &Origins, field: &str, message: impl Into<String>) -> LowerError {
    LowerError {
        origin: origins.get(field).clone(),
        field: field.into(),
        message: message.into(),
        pin_origin: None,
        validation: None,
    }
}
fn positioned(origins: &Origins, prefix: &str, field: &str, message: impl Into<String>) -> Error {
    let message = format!("{}: {field}: {}", origins.get(field), message.into());
    Error::new(ErrorKind::Config, &message)
        .with_diagnostics(vec![Diagnostic::new(format!("{prefix}{field}"), message)])
}
fn fact_error(message: impl Into<String>) -> Error {
    let message = message.into();
    Error::new(ErrorKind::Config, &message).with_diagnostics(vec![Diagnostic::new("", message)])
}
fn name(value: &str) -> bool {
    !value.trim().is_empty()
}
fn pair(
    origins: &Origins,
    field: &str,
    use_id: Option<&str>,
    item: Option<&str>,
    item_key: &str,
) -> Result<bool, LowerError> {
    if use_id.is_none() && item.is_none() {
        return Ok(false);
    }
    if !use_id.is_some_and(name) || !item.is_some_and(name) {
        return Err(source_error(
            origins,
            &format!("{field}.{item_key}"),
            format!(
                "binding requires nonempty use and {item_key}; use set at {}",
                origins.get(&format!("{field}.use"))
            ),
        ));
    }
    Ok(true)
}
fn exclusive(origins: &Origins, field: &str, keys: &[(&str, bool)]) -> Result<(), LowerError> {
    let present: Vec<_> = keys
        .iter()
        .filter(|(_, present)| *present)
        .map(|(key, _)| *key)
        .collect();
    if present.len() > 1 {
        let sites = present
            .iter()
            .map(|key| {
                format!(
                    "{field}.{key} at {}",
                    origins.get(&format!("{field}.{key}"))
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(source_error(
            origins,
            field,
            format!("sources are mutually exclusive: {sites}"),
        ));
    }
    Ok(())
}

/// Check each layer's source groups before inheritance can hide a conflict.
/// Missing sources are allowed here so an override can retain its parent's source.
pub fn validate_sources(document: &MachineFile, origins: &Origins) -> Result<(), LowerError> {
    for (field, rom) in roms(document) {
        let bound = pair(
            origins,
            &field,
            rom.use_id.as_deref(),
            rom.rom.as_deref(),
            "rom",
        )?;
        if !bound && rom.image.is_some() && rom.parts.is_some() {
            return Err(source_error(
                origins,
                &field,
                "image and parts are mutually exclusive",
            ));
        }
        exclusive(
            origins,
            &field,
            &[
                ("image", rom.image.is_some()),
                ("parts", rom.parts.is_some()),
                ("use", bound),
            ],
        )?;
        if !bound
            && (rom.role.is_some() || rom.size.is_some())
            && (rom.image.is_some() || rom.parts.is_some())
        {
            return Err(source_error(
                origins,
                &field,
                "role and size assertions require a package binding",
            ));
        }
        if rom.size == Some(0) {
            return Err(source_error(
                origins,
                &format!("{field}.size"),
                "item size must be positive",
            ));
        }
        if let Some(role) = &rom.role {
            if !["system-bios", "video-bios", "option-rom"].contains(&role.as_str()) {
                return Err(source_error(
                    origins,
                    &format!("{field}.role"),
                    "unknown requested role",
                ));
            }
        }
    }
    for (i, card) in document.card.iter().flatten().enumerate() {
        for (j, drive) in card.drives.iter().flatten().enumerate() {
            let field = format!("card[{i}].drives[{j}]");
            exclusive(
                origins,
                &field,
                &[
                    ("image", drive.image.is_some()),
                    ("disk", drive.disk.is_some()),
                    ("inserted", drive.inserted.is_some()),
                    ("media", drive.media.is_some()),
                ],
            )?;
            if drive.empty == Some(true) && (drive.inserted.is_some() || drive.disk.is_some()) {
                return Err(source_error(
                    origins,
                    &field,
                    "empty conflicts with disk or inserted binding",
                ));
            }
            if let Some(selector) = &drive.inserted {
                if !pair(
                    origins,
                    &format!("{field}.inserted"),
                    selector.use_id.as_deref(),
                    selector.medium.as_deref(),
                    "medium",
                )? {
                    return Err(source_error(
                        origins,
                        &format!("{field}.inserted"),
                        "binding requires use and medium",
                    ));
                }
            }
            for (k, medium) in drive.media.iter().flatten().enumerate() {
                let field = format!("{field}.media[{k}]");
                let bound = pair(
                    origins,
                    &field,
                    medium.use_id.as_deref(),
                    medium.medium.as_deref(),
                    "medium",
                )?;
                exclusive(
                    origins,
                    &field,
                    &[
                        (
                            "image",
                            !medium.image.is_empty()
                                || origins.has_field(&format!("{field}.image")),
                        ),
                        ("use", bound),
                    ],
                )?;
                if !bound
                    && medium.image.is_empty()
                    && !origins.has_field(&format!("{field}.image"))
                {
                    return Err(source_error(
                        origins,
                        &field,
                        "medium requires image or use and medium",
                    ));
                }
            }
        }
    }
    Ok(())
}
/// Whether lowering still needs package composition or native zero preparation.
pub fn has_bindings(document: &MachineFile) -> bool {
    document.disks.is_some()
        || roms(document).any(|(_, r)| r.use_id.is_some() || r.rom.is_some())
        || document
            .card
            .iter()
            .flatten()
            .flat_map(|c| c.drives.iter().flatten())
            .any(|d| {
                d.disk.is_some()
                    || d.inserted.is_some()
                    || d.media
                        .iter()
                        .flatten()
                        .any(|m| m.use_id.is_some() || m.medium.is_some())
            })
}
fn roms(document: &MachineFile) -> impl Iterator<Item = (String, &Rom)> {
    document.firmware.iter().flat_map(|firmware| {
        firmware
            .bios
            .iter()
            .map(|rom| ("firmware.bios".into(), rom))
            .chain(
                firmware
                    .option_roms
                    .iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, rom)| (format!("firmware.option_roms[{i}]"), rom)),
            )
    })
}
fn pin(pin: &Sha256) -> [u8; 32] {
    let mut digest = [0; 32];
    for (byte, pair) in digest
        .iter_mut()
        .zip(pin.as_str().as_bytes().as_chunks::<2>().0)
    {
        let digit = |b: u8| {
            if b.is_ascii_digit() {
                b - b'0'
            } else {
                b.to_ascii_lowercase() - b'a' + 10
            }
        };
        *byte = digit(pair[0]) * 16 + digit(pair[1]);
    }
    digest
}
fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Debug)]
enum Target {
    Rom(Option<usize>),
    Drive(usize, usize, Option<usize>),
}
#[derive(Clone, Debug)]
struct Occurrence {
    slot: BindingSlot,
    target: Target,
    field: String,
    required_role: Option<&'static str>,
    base: Option<u32>,
    offset: Option<usize>,
    drive_kind: Option<assembly::DriveKind>,
    readonly: bool,
}

/// A frozen plan owns the resolved layer and its original positions. Slots are
/// sorted independently of the hardware-order occurrences used for rewriting.
#[derive(Clone, Debug)]
pub struct BindingPlan {
    document: MachineFile,
    origins: Origins,
    prefix: String,
    occurrences: Vec<Occurrence>,
    slots: Vec<BindingSlot>,
}

/// Discover package and zero occurrences without a reader or filesystem access.
pub fn plan(document: &MachineFile, origins: &Origins) -> Result<BindingPlan, Error> {
    plan_with_prefix(document, origins, "")
}

/// Parse an envelope's hardware directly from the original text. Outer rows are
/// left to the existing envelope codec; their positions are never regenerated.
pub fn plan_envelope(input: &str) -> Result<BindingPlan, Error> {
    #[derive(Deserialize)]
    struct Envelope {
        schema: SchemaVersion,
        platform: String,
        machine: MachineFile,
    }
    crate::check_size(Path::new("config.toml"), input.len())
        .map_err(|e| fact_error(e.to_string()))?;
    let envelope: Envelope = toml::from_str(input).map_err(|e| fact_error(e.to_string()))?;
    let _ = envelope.schema;
    if crate::compat::platform(&envelope.platform).is_err() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "unsupported binding platform",
        ));
    }
    let mut original = Origins::new("config.toml");
    crate::source::record_toml(Path::new("config.toml"), input, &mut original)
        .map_err(|e| fact_error(e.to_string()))?;
    let mut origins = Origins::new("config.toml");
    origins.copy_subtree(&original, "machine", "");
    plan_with_prefix(&envelope.machine, &origins, "machine.")
}

fn plan_with_prefix(
    document: &MachineFile,
    origins: &Origins,
    prefix: &str,
) -> Result<BindingPlan, Error> {
    let error = |field: &str, message: String| positioned(origins, prefix, field, message);
    if document.extends.is_some() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "resolve inheritance before binding discovery",
        ));
    }
    validate_sources(document, origins).map_err(|e| error(&e.field, e.message))?;
    let mut occurrences = Vec::new();
    for (field, rom) in roms(document) {
        let Some(use_id) = &rom.use_id else { continue };
        let bios = field == "firmware.bios";
        if !bios && rom.base.is_none() {
            return Err(error(
                &format!("{field}.base"),
                "option ROM requires base".into(),
            ));
        }
        if bios && rom.offset.is_some() {
            return Err(error(
                &format!("{field}.offset"),
                "offset is only supported by option ROMs".into(),
            ));
        }
        let role = if bios {
            "system-bios"
        } else if rom.base.is_some_and(|b| (0xC0000..0xC8000).contains(&b)) {
            "video-bios"
        } else {
            "option-rom"
        };
        let deferred_video = role == "option-rom" && rom.role.as_deref() == Some("video-bios");
        if rom.role.as_deref().is_some_and(|asserted| asserted != role) && !deferred_video {
            return Err(error(
                &format!("{field}.role"),
                format!(
                    "role conflicts with position requiring {role}; base set at {}",
                    origins.get(&format!("{field}.base"))
                ),
            ));
        }
        let mut slot = BindingSlot::new(
            &field,
            use_id,
            rom.rom.as_deref().unwrap_or_default(),
            BindingKind::FirmwareRom,
        );
        slot.role = rom.role.clone();
        slot.size = rom.size;
        slot.sha256 = rom.sha256.as_ref().map(pin);
        let index = document
            .firmware
            .as_ref()
            .and_then(|f| f.option_roms.as_ref())
            .and_then(|rs| rs.iter().position(|r| std::ptr::eq(r, rom)));
        occurrences.push(Occurrence {
            slot,
            target: Target::Rom(index),
            field: format!("{field}.rom"),
            required_role: Some(role),
            base: rom.base,
            offset: rom.offset,
            drive_kind: None,
            readonly: true,
        });
    }
    let mut zeros = BTreeMap::new();
    for (i, disk) in document.disks.iter().flatten().enumerate() {
        let field = format!("disks[{i}]");
        if disk.id == "cmos"
            || disk.id.is_empty()
            || disk.id.len() > 32
            || !disk
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || disk.size == 0
        {
            return Err(error(&field, "invalid zero disk id or size".into()));
        }
        if let Some((_, first)) = zeros.insert(disk.id.as_str(), (disk.size, field.clone())) {
            return Err(error(
                &field,
                format!(
                    "duplicate zero disk id {}; first declared at {}",
                    disk.id,
                    origins.get(&first)
                ),
            ));
        }
    }
    let mut attached = BTreeMap::new();
    for (i, card) in document.card.iter().flatten().enumerate() {
        for (j, drive) in card.drives.iter().flatten().enumerate() {
            let field = format!("card[{i}].drives[{j}]");
            if drive.disk.is_none()
                && drive.inserted.is_none()
                && !drive.media.iter().flatten().any(|m| m.use_id.is_some())
            {
                continue;
            }
            let kind = drive_kind(card, drive)
                .map_err(|message| error(&format!("{field}.geometry"), message))?;
            if let Some(id) = &drive.disk {
                let (size, declaration) = zeros.get(id.as_str()).ok_or_else(|| {
                    error(
                        &format!("{field}.disk"),
                        format!("undeclared zero disk {id:?}"),
                    )
                })?;
                if card.kind != Some(CardKind::Ide) {
                    return Err(error(&field, "zero disk requires IDE geometry".into()));
                }
                if let Some(first) = attached.insert(id.as_str(), field.clone()) {
                    return Err(error(&field,format!("zero disk {id:?} attached more than once; first at {}; declaration at {}",origins.get(&first),origins.get(declaration))));
                }
                kind.check_image_len(0, *size).map_err(|e| {
                    error(
                        &field,
                        format!("{e}; zero declaration at {}", origins.get(declaration)),
                    )
                })?;
                occurrences.push(Occurrence {
                    slot: BindingSlot::zero_disk(&field, id, *size),
                    target: Target::Drive(i, j, None),
                    field: format!("{field}.disk"),
                    required_role: None,
                    base: None,
                    offset: None,
                    drive_kind: Some(kind),
                    readonly: drive.write == Some(WriteMode::ReadOnly),
                });
            }
            if let Some(selector) = &drive.inserted {
                let mut slot = BindingSlot::new(
                    &field,
                    selector.use_id.as_deref().unwrap_or_default(),
                    selector.medium.as_deref().unwrap_or_default(),
                    BindingKind::Medium,
                );
                slot.size = selector.size;
                slot.sha256 = selector.sha256.as_ref().map(pin);
                if let Some(outer) = &drive.sha256 {
                    if slot.sha256.is_some_and(|p| p != pin(outer)) {
                        return Err(error(
                            &field,
                            "conflicting drive and selector sha256 assertions".into(),
                        ));
                    }
                    slot.sha256 = Some(pin(outer));
                }
                occurrences.push(Occurrence {
                    slot,
                    target: Target::Drive(i, j, None),
                    field: format!("{field}.inserted.medium"),
                    required_role: None,
                    base: None,
                    offset: None,
                    drive_kind: Some(kind),
                    readonly: drive.write == Some(WriteMode::ReadOnly),
                });
            }
            for (k, medium) in drive.media.iter().flatten().enumerate() {
                let Some(use_id) = &medium.use_id else {
                    continue;
                };
                if card.kind != Some(CardKind::Fdc765) {
                    return Err(error(
                        &field,
                        "media catalogue requires a floppy controller".into(),
                    ));
                }
                let id = format!("{field}.media[{k}]");
                let mut slot = BindingSlot::new(
                    &id,
                    use_id,
                    medium.medium.as_deref().unwrap_or_default(),
                    BindingKind::Medium,
                );
                slot.size = medium.size;
                slot.sha256 = medium.sha256.as_ref().map(pin);
                occurrences.push(Occurrence {
                    slot,
                    target: Target::Drive(i, j, Some(k)),
                    field: format!("{id}.medium"),
                    required_role: None,
                    base: None,
                    offset: None,
                    drive_kind: Some(kind),
                    readonly: medium.write == Some(WriteMode::ReadOnly),
                });
            }
        }
    }
    for (id, (_, field)) in zeros {
        if !attached.contains_key(id) {
            return Err(error(&field, format!("unused zero disk {id:?}")));
        }
    }
    let result = BindingPlan {
        document: document.clone(),
        origins: origins.clone(),
        prefix: prefix.into(),
        slots: {
            let mut slots = occurrences
                .iter()
                .map(|o| o.slot.clone())
                .collect::<Vec<_>>();
            slots.sort_by(|a, b| a.id.cmp(&b.id));
            slots
        },
        occurrences,
    };
    for occurrence in &result.occurrences {
        if let Some(size) = occurrence.slot.size {
            result.check_extent(occurrence, size)?;
        }
    }
    let mut declared_lengths = BTreeMap::new();
    for occurrence in &result.occurrences {
        if let (Some(digest), Some(size)) = (occurrence.slot.sha256, occurrence.slot.size) {
            if let Some((first_size, first)) =
                declared_lengths.insert(digest, (size, occurrence.field.clone()))
            {
                if first_size != size {
                    return Err(result.error(
                        occurrence,
                        format!(
                            "sha256 has conflicting declared sizes; first at {}",
                            origins.get(&first)
                        ),
                    ));
                }
            }
        }
    }
    result.check_config()?;
    Ok(result)
}

fn drive_kind(card: &Card, drive: &Drive) -> Result<assembly::DriveKind, String> {
    match card.kind {
        Some(CardKind::Ide) => {
            let Some(Geometry::Chs(chs)) = &drive.geometry else {
                return Err("explicit IDE geometry is required".into());
            };
            let (Some(c), Some(h), Some(s)) = (chs.cylinders, chs.heads, chs.sectors) else {
                return Err("complete IDE geometry is required".into());
            };
            if u64::from(c)
                .checked_mul(u64::from(h))
                .and_then(|n| n.checked_mul(u64::from(s)))
                .and_then(|n| n.checked_mul(512))
                .is_none()
            {
                return Err("geometry size overflows".into());
            }
            Ok(assembly::DriveKind::HardDisk {
                geometry: assembly::HardDiskGeometry {
                    cylinders: c,
                    heads: h,
                    sectors_per_track: s,
                    bytes_per_sector: 512,
                },
            })
        }
        Some(CardKind::Fdc765) => {
            use assembly::descriptors::FloppyType as F;
            let geometry = match drive.drive_type {
                Some(DriveType::Hd144) => F::HD_1_44.geometry,
                Some(DriveType::Hd12) => F::HD_1_2.geometry,
                Some(DriveType::Dd720) => F::DD_720K.geometry,
                Some(DriveType::Dd360) => F::DD_360K.geometry,
                None => return Err("floppy type is required".into()),
            };
            Ok(assembly::DriveKind::Floppy { geometry })
        }
        _ => Err("medium requires a disk controller".into()),
    }
}

/// One resolved package occurrence. Order is hardware order, independent of facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedItem {
    pub slot_id: String,
    pub key: String,
    pub kind: BindingKind,
    pub size: u64,
    pub sha256: [u8; 32],
}

/// Logical-key hardware and the rows consumed by the prepared-envelope writer.
/// Zero attachments remain in the hardware and slots; they never acquire content rows.
#[derive(Clone, Debug)]
pub struct ResolvedBindings {
    pub document: MachineFile,
    pub origins: Origins,
    pub items: Vec<ResolvedItem>,
    pub zero_slots: Vec<BindingSlot>,
    prefix: String,
    source_fields: BTreeMap<String, (String, String)>,
}
impl BindingPlan {
    pub fn slots(&self) -> &[BindingSlot] {
        &self.slots
    }
    fn error(&self, occurrence: &Occurrence, message: impl Into<String>) -> Error {
        positioned(
            &self.origins,
            &self.prefix,
            &occurrence.field,
            format!(
                "slot {}: use {:?}, item {:?}: {}",
                occurrence.slot.id,
                occurrence.slot.use_id,
                occurrence.slot.item_id,
                message.into()
            ),
        )
    }
    fn assertion_origin(&self, occurrence: &Occurrence, key: &str) -> &crate::Origin {
        let parent = occurrence
            .field
            .rsplit_once('.')
            .map_or(occurrence.slot.id.as_str(), |(parent, _)| parent);
        let nested = format!("{parent}.{key}");
        if self.origins.has_field(&nested) {
            self.origins.get(&nested)
        } else {
            self.origins.get(&format!("{}.{key}", occurrence.slot.id))
        }
    }
    fn check_extent(&self, occurrence: &Occurrence, size: u64) -> Result<(), Error> {
        if size == 0 {
            return Err(self.error(occurrence, "item size must be positive"));
        }
        if let Some(kind) = occurrence.drive_kind {
            kind.check_image_len(0, size)
                .map_err(|e| self.error(occurrence, e.to_string()))?;
        }
        if let Target::Rom(Some(index)) = occurrence.target {
            assembly::check_option_rom_alignment(
                index + 1,
                u64::from(occurrence.base.unwrap_or(0)),
            )
            .map_err(|e| self.error(occurrence, e.to_string()))?;
        }
        if occurrence.required_role.is_some() {
            let mapped = size
                .checked_sub(occurrence.offset.unwrap_or(0) as u64)
                .filter(|n| *n > 0)
                .ok_or_else(|| self.error(occurrence, "offset must leave a nonempty ROM"))?;
            if occurrence.required_role == Some("system-bios") {
                let base = occurrence
                    .base
                    .map(u64::from)
                    .or_else(|| 0x100000_u64.checked_sub(mapped));
                assembly::check_bios_placement(base.unwrap_or(0), mapped).map_err(|e| {
                    self.error(
                        occurrence,
                        format!(
                            "BIOS base versus size: {e}; base at {}",
                            self.origins.get(&format!("{}.base", occurrence.slot.id))
                        ),
                    )
                })?;
            } else if occurrence
                .base
                .map(u64::from)
                .and_then(|base| base.checked_add(mapped))
                .is_none_or(|end| end > 0x100000)
            {
                return Err(self.error(
                    occurrence,
                    "option ROM extent exceeds the reset address space",
                ));
            }
        }
        Ok(())
    }

    fn check_config(&self) -> Result<(), Error> {
        let mut shadow = self.document.clone();
        shadow.disks = None;
        for firmware in shadow.firmware.iter_mut() {
            for (target, rom) in firmware
                .bios
                .iter_mut()
                .map(|r| (Target::Rom(None), r))
                .chain(
                    firmware
                        .option_roms
                        .iter_mut()
                        .flatten()
                        .enumerate()
                        .map(|(i, r)| (Target::Rom(Some(i)), r)),
                )
            {
                if rom.use_id.is_some() {
                    rom.image = Some(logical_key(&target));
                    rom.use_id = None;
                    rom.rom = None;
                    rom.role = None;
                    rom.size = None;
                }
            }
        }
        for (i, card) in shadow.card.iter_mut().flatten().enumerate() {
            for (j, drive) in card.drives.iter_mut().flatten().enumerate() {
                drive.disk = None;
                if drive.inserted.take().is_some() {
                    drive.image = Some(logical_key(&Target::Drive(i, j, None)));
                }
                for (k, medium) in drive.media.iter_mut().flatten().enumerate() {
                    if medium.use_id.take().is_some() {
                        medium.image = logical_key(&Target::Drive(i, j, Some(k)));
                        medium.medium = None;
                        medium.size = None;
                    }
                }
            }
        }
        struct NoContent(BTreeMap<PathBuf, u64>);
        impl ImageReader for NoContent {
            fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
                Err(io::Error::other("planning cannot read content"))
            }
            fn image_length(&self, path: &Path) -> io::Result<u64> {
                self.0
                    .get(path)
                    .copied()
                    .ok_or_else(|| io::Error::other("size is pending"))
            }
        }
        let sizes = self
            .occurrences
            .iter()
            .filter_map(|o| {
                o.slot
                    .size
                    .map(|size| (PathBuf::from(logical_key(&o.target)), size))
            })
            .collect();
        crate::lower::lower_metadata(&shadow, &self.origins, &mut NoContent(sizes))
            .map(|_| ())
            .map_err(|e| positioned(&self.origins, &self.prefix, &e.field, e.message))
    }

    /// Validate every supplied fact before rewriting any selector. This operation
    /// cannot open content: package roles are checked at the selected item before
    /// shared digests are considered for identity consistency.
    pub fn resolve(&self, uses: &[BindingUse]) -> Result<ResolvedBindings, Error> {
        let mut facts = BTreeMap::new();
        let mut lengths = BTreeMap::new();
        let mut ordered = uses.iter().collect::<Vec<_>>();
        ordered.sort_by(|a, b| a.id.cmp(&b.id));
        for package in ordered {
            if !name(&package.id) || !name(&package.manifest_digest) {
                return Err(fact_error(format!(
                    "use {:?}: empty name or manifest digest",
                    package.id
                )));
            }
            if facts.insert(package.id.as_str(), package).is_some() {
                return Err(fact_error(format!("duplicate use {:?}", package.id)));
            }
            let mut ids = BTreeSet::new();
            let mut items = package.items.iter().collect::<Vec<_>>();
            items.sort_by(|a, b| a.id.cmp(&b.id));
            for item in items {
                if !name(&item.id) || item.roles.iter().any(|role| !name(role)) {
                    return Err(fact_error(format!(
                        "use {:?}, item {:?}: empty item or role name",
                        package.id, item.id
                    )));
                }
                if !ids.insert(&item.id) {
                    return Err(fact_error(format!(
                        "use {:?}: ambiguous duplicate item {:?}",
                        package.id, item.id
                    )));
                }
                if !matches!(item.kind, BindingKind::FirmwareRom | BindingKind::Medium)
                    || item.size == 0
                {
                    return Err(fact_error(format!(
                        "use {:?}, item {:?}: invalid item kind or size",
                        package.id, item.id
                    )));
                }
                if let Some(first) = lengths.insert(item.sha256, item.size) {
                    if first != item.size {
                        return Err(fact_error(format!(
                            "use {:?}, item {:?}: sha256 has conflicting flat sizes",
                            package.id, item.id
                        )));
                    }
                }
            }
        }
        let used = self
            .occurrences
            .iter()
            .filter_map(|o| o.slot.use_id.as_deref())
            .collect::<BTreeSet<_>>();
        for id in facts.keys() {
            if !used.contains(id) {
                return Err(fact_error(format!("unused supplied use {id:?}")));
            }
        }
        let mut selected = Vec::new();
        let mut writable = BTreeMap::new();
        for occurrence in &self.occurrences {
            let slot = &occurrence.slot;
            let Some(use_id) = slot.use_id.as_deref() else {
                continue;
            };
            let package = facts
                .get(use_id)
                .ok_or_else(|| self.error(occurrence, "unbound use"))?;
            let item = package
                .items
                .iter()
                .find(|item| Some(item.id.as_str()) == slot.item_id.as_deref())
                .ok_or_else(|| self.error(occurrence, "selected item is missing"))?;
            if item.kind != slot.kind {
                return Err(self.error(occurrence, "wrong item kind"));
            }
            if slot.size.is_some_and(|size| size != item.size) {
                return Err(self.error(
                    occurrence,
                    format!(
                        "size conflicts with selected item's facts; assertion at {}",
                        self.assertion_origin(occurrence, "size")
                    ),
                ));
            }
            if slot.sha256.is_some_and(|digest| digest != item.sha256) {
                return Err(self.error(
                    occurrence,
                    format!(
                        "sha256 conflicts with selected item's facts; assertion at {}",
                        self.assertion_origin(occurrence, "sha256")
                    ),
                ));
            }
            let mut role = occurrence.required_role;
            if role.is_some() {
                let catalogue = match self.document.board.as_ref().and_then(|b| b.model) {
                    Some(BoardModel::Generic386Isa) => {
                        assembly::BoardModel::Generic386Isa.firmware_catalogue()
                    }
                    _ => assembly::BoardModel::Ibm5170.firmware_catalogue(),
                };
                if let Some(entry) = catalogue
                    .iter()
                    .find(|entry| entry.sha256 == hex(&item.sha256) && entry.size == item.size)
                {
                    if occurrence
                        .offset
                        .is_some_and(|offset| offset != entry.rom_offset)
                    {
                        return Err(
                            self.error(occurrence, "offset differs from the catalogue ROM layout")
                        );
                    }
                    let mut effective = occurrence.clone();
                    effective.offset = Some(entry.rom_offset);
                    self.check_extent(&effective, item.size)?;
                    let classified = match entry.kind {
                        assembly::FirmwareKind::SystemBios => "system-bios",
                        assembly::FirmwareKind::VideoBios => "video-bios",
                        assembly::FirmwareKind::OptionRom => "option-rom",
                    };
                    if role == Some("system-bios") && classified != "system-bios" {
                        return Err(self.error(
                            occurrence,
                            "catalogue firmware kind conflicts with system position",
                        ));
                    }
                    if role == Some("video-bios") && classified != "video-bios" {
                        return Err(self.error(
                            occurrence,
                            "catalogue firmware kind conflicts with video position",
                        ));
                    }
                    if role != Some("system-bios") {
                        if classified == "system-bios" {
                            return Err(self.error(
                                occurrence,
                                "catalogue system BIOS cannot be an option ROM",
                            ));
                        }
                        role = Some(classified);
                    }
                    if slot
                        .role
                        .as_deref()
                        .is_some_and(|asserted| Some(asserted) != role)
                    {
                        return Err(self.error(
                            occurrence,
                            "role assertion conflicts with reviewed catalogue kind",
                        ));
                    }
                }
            }
            if slot
                .role
                .as_deref()
                .is_some_and(|asserted| Some(asserted) != role)
            {
                return Err(self.error(
                    occurrence,
                    "role assertion conflicts with effective hardware role",
                ));
            }
            if let Some(role) = role {
                if !item.roles.iter().any(|declared| declared == role) {
                    return Err(self.error(occurrence,format!("selected package item does not declare required role {role:?} (implied by position)")));
                }
            }
            self.check_extent(occurrence, item.size)?;
            if slot.kind == BindingKind::Medium {
                if let Some((readonly, first)) =
                    writable.insert(item.sha256, (occurrence.readonly, slot.id.clone()))
                {
                    if !readonly || !occurrence.readonly {
                        return Err(self.error(
                            occurrence,
                            format!("writable medium bound more than once; first slot {first}"),
                        ));
                    }
                }
            }
            selected.push((occurrence, item));
        }
        let mut resolved = ResolvedBindings {
            prefix: self.prefix.clone(),
            source_fields: BTreeMap::new(),
            document: self.document.clone(),
            origins: self.origins.clone(),
            items: Vec::new(),
            zero_slots: self
                .slots
                .iter()
                .filter(|s| s.kind == BindingKind::ZeroDisk)
                .cloned()
                .collect(),
        };
        for (occurrence, item) in selected {
            let key = logical_key(&occurrence.target);
            if path_names(&self.document)
                .iter()
                .any(|name| Path::new(name) == Path::new(&key))
            {
                return Err(self.error(
                    occurrence,
                    format!("generated logical key {key:?} collides with preserved source"),
                ));
            }
            let sha: Sha256 = toml::Value::String(hex(&item.sha256))
                .try_into()
                .map_err(|e| fact_error(format!("invalid resolved digest: {e}")))?;
            let output_field = match occurrence.target {
                Target::Rom(index) => {
                    let firmware = resolved
                        .document
                        .firmware
                        .as_mut()
                        .ok_or_else(|| self.error(occurrence, "missing firmware"))?;
                    let rom = match index {
                        None => firmware.bios.as_mut(),
                        Some(i) => firmware.option_roms.as_mut().and_then(|r| r.get_mut(i)),
                    }
                    .ok_or_else(|| self.error(occurrence, "missing ROM occurrence"))?;
                    rom.image = Some(key.clone());
                    rom.sha256 = Some(sha);
                    rom.use_id = None;
                    rom.rom = None;
                    rom.role = None;
                    rom.size = None;
                    format!("{}.image", occurrence.slot.id)
                }
                Target::Drive(i, j, k) => {
                    let drive = resolved
                        .document
                        .card
                        .as_mut()
                        .and_then(|c| c.get_mut(i))
                        .and_then(|c| c.drives.as_mut())
                        .and_then(|d| d.get_mut(j))
                        .ok_or_else(|| self.error(occurrence, "missing drive occurrence"))?;
                    if let Some(k) = k {
                        let medium = drive
                            .media
                            .as_mut()
                            .and_then(|m| m.get_mut(k))
                            .ok_or_else(|| self.error(occurrence, "missing medium occurrence"))?;
                        medium.image = key.clone();
                        medium.sha256 = Some(sha);
                        medium.use_id = None;
                        medium.medium = None;
                        medium.size = None;
                    } else {
                        drive.image = Some(key.clone());
                        drive.sha256 = Some(sha);
                        drive.inserted = None;
                    }
                    format!("{}.image", occurrence.slot.id)
                }
            };
            let source = (
                occurrence.field.clone(),
                format!(
                    "use {:?}, item {:?}",
                    occurrence.slot.use_id.as_deref().unwrap_or_default(),
                    occurrence.slot.item_id.as_deref().unwrap_or_default(),
                ),
            );
            resolved
                .source_fields
                .insert(output_field.clone(), source.clone());
            if matches!(occurrence.target, Target::Rom(_)) {
                resolved
                    .source_fields
                    .insert(occurrence.slot.id.clone(), source);
            }
            resolved
                .origins
                .insert(&output_field, self.origins.get(&occurrence.field).clone());
            resolved.origins.insert(
                format!("{}.sha256", occurrence.slot.id),
                self.origins.get(&occurrence.field).clone(),
            );
            resolved.origins.bind_image_key(&output_field, &key);
            resolved.items.push(ResolvedItem {
                slot_id: occurrence.slot.id.clone(),
                key,
                kind: item.kind,
                size: item.size,
                sha256: item.sha256,
            });
        }
        if resolved.zero_slots.is_empty() {
            resolved.document.disks = None;
        }
        Ok(resolved)
    }
}
fn logical_key(target: &Target) -> String {
    match target {
        Target::Rom(None) => "assets/firmware/bios.bin".into(),
        Target::Rom(Some(i)) => format!("assets/firmware/option-{i}.bin"),
        Target::Drive(i, j, None) => format!("assets/card-{i}/drive-{j}/image.bin"),
        Target::Drive(i, j, Some(k)) => format!("assets/card-{i}/drive-{j}/media-{k}.bin"),
    }
}
fn path_names(document: &MachineFile) -> Vec<&str> {
    let mut names = Vec::new();
    for (_, rom) in roms(document) {
        names.extend(rom.image.as_deref());
        names.extend(rom.parts.iter().flatten().map(|p| p.image.as_str()));
    }
    for card in document.card.iter().flatten() {
        for drive in card.drives.iter().flatten() {
            names.extend(drive.image.as_deref());
            names.extend(
                drive
                    .media
                    .iter()
                    .flatten()
                    .filter(|m| !m.image.is_empty())
                    .map(|m| m.image.as_str()),
            );
        }
    }
    if let Some(Nvram::Image(nvram)) = document
        .board
        .as_ref()
        .and_then(|b| b.rtc.as_ref())
        .and_then(|r| r.nvram.as_ref())
    {
        names.extend(nvram.image.as_deref());
    }
    names
}

/// Route bound logical keys and ordinary paths through the same ImageReader seam.
/// The caller supplies path interpretation; this adapter never probes the host.
pub struct BindingImageReader<'a, R> {
    pub fallback: &'a mut R,
    pub content: &'a dyn ContentSource,
    pub resolved: &'a ResolvedBindings,
}
impl<R: ImageReader> ImageReader for BindingImageReader<'_, R> {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let Some(item) = self
            .resolved
            .items
            .iter()
            .find(|item| Path::new(&item.key) == path)
        else {
            return self.fallback.read(path);
        };
        match self
            .content
            .open(&item.sha256, item.size)
            .map_err(io::Error::other)?
        {
            Content::Bytes(bytes) if bytes.len() as u64 == item.size => Ok(bytes),
            Content::Reader(reader) if reader.size() == item.size => {
                let length = usize::try_from(item.size)
                    .map_err(|_| io::Error::other("content exceeds address space"))?;
                let mut bytes = vec![0; length];
                reader.read_at(0, &mut bytes).map_err(io::Error::other)?;
                Ok(bytes)
            }
            _ => Err(io::Error::other("content size differs from selected fact")),
        }
    }
    fn image_length(&self, path: &Path) -> io::Result<u64> {
        self.resolved
            .items
            .iter()
            .find(|item| Path::new(&item.key) == path)
            .map(|item| Ok(item.size))
            .unwrap_or_else(|| self.fallback.image_length(path))
    }
    fn validate_machine(
        &self,
        config: &assembly::MachineConfig,
    ) -> Result<(), assembly::ConfigError> {
        self.fallback.validate_machine(config)
    }
    fn verify_journal_base(&mut self, path: &Path) -> io::Result<JournalBase> {
        if let Some(item) = self
            .resolved
            .items
            .iter()
            .find(|item| Path::new(&item.key) == path)
        {
            let actual = match self
                .content
                .open(&item.sha256, item.size)
                .map_err(io::Error::other)?
            {
                Content::Bytes(bytes) => bytes.len() as u64,
                Content::Reader(reader) => reader.size(),
                _ => return Err(io::Error::other("unsupported content")),
            };
            if actual != item.size {
                return Err(io::Error::other("content size differs from selected fact"));
            }
            Ok(JournalBase {
                length: item.size,
                sha256: item.sha256,
            })
        } else {
            self.fallback.verify_journal_base(path)
        }
    }
    fn is_reserved_retention_name(&self, path: &Path) -> bool {
        if self
            .resolved
            .items
            .iter()
            .any(|item| Path::new(&item.key) == path)
        {
            false
        } else {
            self.fallback.is_reserved_retention_name(path)
        }
    }
    fn read_journal_base(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        self.fallback.read_journal_base(path)
    }
    fn validate_journal_paths(
        &self,
        config: &assembly::MachineConfig,
        origins: &Origins,
        images: &[PathBuf],
    ) -> Result<(), assembly::ConfigError> {
        self.fallback
            .validate_journal_paths(config, origins, images)
    }
}

/// Validate the complete config and all facts before the shared lowering reader
/// can open anything. Native zero mounting is deliberately left to its own stage.
pub fn lower_with_bindings<R: ImageReader>(
    document: &MachineFile,
    origins: &Origins,
    uses: &[BindingUse],
    content: &dyn ContentSource,
    reader: &mut R,
    options: assembly::AssemblyOptions,
) -> Result<assembly::MachineConfig, Error> {
    let plan = plan(document, origins)?;
    let resolved = plan.resolve(uses)?;
    resolved.check(reader)?;
    if !resolved.zero_slots.is_empty() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "native zero mounting is required before lowering zero attachments",
        ));
    }
    let mut routed = BindingImageReader {
        fallback: reader,
        content,
        resolved: &resolved,
    };
    crate::lower_with_options(&resolved.document, &resolved.origins, &mut routed, options)
        .map_err(|e| resolved.lower_error(e))
}
impl ResolvedBindings {
    fn lower_error(&self, error: LowerError) -> Error {
        let (field, message) = match self.source_fields.get(&error.field) {
            Some((field, context)) => (
                field.as_str(),
                format!(
                    "{}: {field}: {context}: {}",
                    self.origins.get(field),
                    error.message
                ),
            ),
            None => (
                error.field.as_str(),
                format!("{}: {}: {}", error.origin, error.field, error.message),
            ),
        };
        Error::new(ErrorKind::Config, &message).with_diagnostics(vec![Diagnostic::new(
            format!("{}{field}", self.prefix),
            message,
        )])
    }
    /// Run the existing config-only lowerer with fact lengths. This is also the
    /// attachment point for the later prepared envelope's combined row checks.
    pub fn check<R: ImageReader>(&self, reader: &mut R) -> Result<(), Error> {
        struct Metadata<'a, R> {
            fallback: &'a mut R,
            items: &'a [ResolvedItem],
        }
        impl<R: ImageReader> ImageReader for Metadata<'_, R> {
            fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
                Err(io::Error::other("planning cannot read content"))
            }
            fn image_length(&self, path: &Path) -> io::Result<u64> {
                self.items
                    .iter()
                    .find(|i| Path::new(&i.key) == path)
                    .map(|i| Ok(i.size))
                    .unwrap_or_else(|| self.fallback.image_length(path))
            }
        }
        let mut document = self.document.clone();
        document.disks = None;
        for card in document.card.iter_mut().flatten() {
            for drive in card.drives.iter_mut().flatten() {
                if drive.disk.is_some() {
                    drive.disk = None;
                }
            }
        }
        crate::lower::lower_metadata(
            &document,
            &self.origins,
            &mut Metadata {
                fallback: reader,
                items: &self.items,
            },
        )
        .map(|_| ())
        .map_err(|e| self.lower_error(e))
    }
}
