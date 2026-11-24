// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transport-neutral PC/AT choices and diagnostics over pending settings edits.
//!
//! The hardware domains are owned by [`crate::descriptors`]; candidate edits
//! run the same checks as [`MachineConfig::validate`]. IBM Personal Computer AT
//! Technical Reference, System Board, I/O Address Map and System Interrupts;
//! Creative Sound Blaster Series Hardware Programming Guide, §2 resource
//! configuration. See `backlog/docs/design/machine-images.md` §6.

#[path = "capabilities_sheet.rs"]
mod sheet;
pub use sheet::{
    fields, firmware_binding, storage_summary, value_label, Field, FirmwareBinding, Group,
};

mod diagnostic;
pub use diagnostic::{Diagnostic, Resource, Severity};

use crate::{descriptors::*, *};

/// A value in a settings field, with no dependency on a file format or UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Integer(u32),
    Name(String),
    /// An unverified file display name, never a catalogue identity.
    FileName(String),
    Disconnected,
}

/// One pending edit. Paths use stable card identities, never vector positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub field_path: String,
    pub value: Value,
}

/// A lowered configuration and atomic pending edits. No image is read here.
#[derive(Clone, Debug)]
pub struct Draft {
    pub config: MachineConfig,
    /// Optional file-level identities. Unnamed cards use their model names.
    pub card_ids: Vec<(DeviceId, String)>,
    /// Optional source field paths keyed by configuration image index.
    pub image_paths: Vec<(usize, String)>,
    pub edits: Vec<Edit>,
    /// Catalogue identities declared by hash, awaiting normal byte verification.
    pub firmware_ids: Vec<(usize, String)>,
    pub firmware_files: Vec<(usize, String)>,
    /// Preserve card roles when the persistence format merges cards by identity.
    pub preserve_card_roles: bool,
    /// Caller-owned revision, echoed in validation and choices.
    pub revision: u64,
}

impl Draft {
    pub fn new(config: MachineConfig) -> Self {
        Self {
            config,
            card_ids: Vec::new(),
            image_paths: Vec::new(),
            edits: Vec::new(),
            firmware_ids: Vec::new(),
            firmware_files: Vec::new(),
            preserve_card_roles: false,
            revision: 0,
        }
    }

    pub fn set(&mut self, field_path: impl Into<String>, value: Value) {
        let field_path = canonical_field_path(self, &field_path.into());
        if value == Value::Name("installed".into()) {
            if let Some(kind) = optional_kind(self, &field_path) {
                if !self
                    .config
                    .devices
                    .iter()
                    .any(|device| device.id == kind.id())
                {
                    self.add_card(kind.name().into(), kind);
                    if kind == CardKind::LotechEms {
                        let frame = format!("{}.frame", self.card_path(kind.id()));
                        if let Some(choice) = choices(self, &frame)
                            .into_iter()
                            .find(|choice| choice.enabled)
                        {
                            self.set(frame, choice.value);
                        }
                    }
                }
            }
        }
        self.edits.retain(|edit| edit.field_path != field_path);
        self.edits.push(Edit { field_path, value });
    }

    fn card_path(&self, id: DeviceId) -> String {
        let name = self
            .card_ids
            .iter()
            .find(|(role, _)| *role == id)
            .map(|(_, name)| name.as_str())
            .or_else(|| {
                self.config
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .and_then(|d| CardKind::of(&d.kind))
                    .map(CardKind::name)
            });
        match name {
            Some(name) => format!("card[{name}]"),
            None => format!("board[{}]", role_name(id)),
        }
    }

    fn image_path(&self, index: usize) -> String {
        if let Some((_, path)) = self.image_paths.iter().find(|(image, _)| *image == index) {
            return path.clone();
        }
        let bios = self.config.system_bios_image.or_else(|| {
            crate::firmware::system_bios(&self.config)
                .ok()
                .and_then(|bios| {
                    self.config
                        .images
                        .iter()
                        .position(|image| std::ptr::eq(image, bios))
                })
        });
        if bios == Some(index) {
            return "firmware.bios".into();
        }
        if self
            .config
            .images
            .get(index)
            .is_some_and(|image| image.kind == ImageKind::Rom)
        {
            let option = self
                .config
                .images
                .iter()
                .take(index)
                .enumerate()
                .filter(|(i, image)| Some(*i) != bios && image.kind == ImageKind::Rom)
                .count();
            format!("firmware.option_roms[{option}]")
        } else {
            format!("images[{index}]")
        }
    }

    fn device_id<'a>(&self, path: &'a str) -> Option<(DeviceId, &'a str)> {
        let (prefix, field) = path.rsplit_once('.')?;
        self.config
            .devices
            .iter()
            .find(|device| self.card_path(device.id) == prefix)
            .map(|device| (device.id, field))
    }
}

fn role_name(id: DeviceId) -> &'static str {
    match id {
        DeviceId::Pit => "pit",
        DeviceId::Keyboard => "keyboard",
        DeviceId::Rtc => "rtc",
        DeviceId::Fdc => "fdc-765",
        DeviceId::Dma => "dma",
        DeviceId::DmaWord => "dma-word",
        DeviceId::Coprocessor => "coprocessor",
        DeviceId::Post => "post",
        DeviceId::MasterPic => "master-pic",
        DeviceId::SlavePic => "slave-pic",
        DeviceId::SysControl => "system-control",
        _ => "device",
    }
}

/// A scalar interval or a finite set; large UART domains can be paged by callers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Domain {
    Integer(IntegerDomain),
    Values(Vec<Value>),
}

/// Field metadata used by lowering defaults and future settings consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDescriptor {
    pub domain: Domain,
    pub default: Option<Value>,
    pub unit: Option<&'static str>,
    /// Fields whose derived defaults can change with this field.
    pub dependent_fields: Vec<String>,
}

fn integers(values: impl IntoIterator<Item = impl Into<u32>>) -> Domain {
    Domain::Values(
        values
            .into_iter()
            .map(|v| Value::Integer(v.into()))
            .collect(),
    )
}

/// Resolve a legacy slot alias to the first existing image, preserving its identity.
pub fn canonical_field_path(draft: &Draft, path: &str) -> String {
    let Some(slot) = FIRMWARE_SLOTS.iter().find(|slot| slot.path == path) else {
        return path.into();
    };
    draft
        .image_paths
        .iter()
        .find(|(index, field)| {
            field.starts_with("firmware.option_roms[") && firmware_kind(draft, *index) == slot.kind
        })
        .map_or_else(|| path.into(), |(_, field)| field.clone())
}

fn firmware_kind(draft: &Draft, index: usize) -> FirmwareKind {
    if draft.config.system_bios_image == Some(index) {
        return FirmwareKind::SystemBios;
    }
    draft
        .firmware_ids
        .iter()
        .find(|(image, _)| *image == index)
        .and_then(|(_, id)| {
            draft
                .config
                .board
                .firmware_catalogue()
                .iter()
                .find(|entry| entry.id == id)
        })
        .map_or_else(
            || {
                if draft
                    .config
                    .images
                    .get(index)
                    .is_some_and(|image| (0xC0000..0xC8000).contains(&image.base))
                {
                    FirmwareKind::VideoBios
                } else {
                    FirmwareKind::OptionRom
                }
            },
            |entry| entry.kind,
        )
}

fn firmware_slot(draft: &Draft, path: &str) -> Option<(FirmwareKind, bool, Option<usize>)> {
    if let Some((index, _)) = draft.image_paths.iter().find(|(_, field)| field == path) {
        let kind = firmware_kind(draft, *index);
        return Some((kind, kind == FirmwareKind::SystemBios, Some(*index)));
    }
    let slot = FIRMWARE_SLOTS.iter().find(|slot| slot.path == path)?;
    let index = if slot.kind == FirmwareKind::SystemBios {
        draft.config.system_bios_image.or_else(|| {
            if draft.image_paths.is_empty() {
                crate::firmware::system_bios(&draft.config)
                    .ok()
                    .and_then(|bios| {
                        draft
                            .config
                            .images
                            .iter()
                            .position(|image| std::ptr::eq(image, bios))
                    })
            } else {
                None
            }
        })
    } else {
        draft
            .config
            .images
            .iter()
            .enumerate()
            .find(|(index, image)| {
                image.kind == ImageKind::Rom && firmware_kind(draft, *index) == slot.kind
            })
            .map(|(index, _)| index)
    };
    Some((slot.kind, slot.required, index))
}

/// Describe a stable field. Unknown or inapplicable paths have no descriptor.
pub fn describe(draft: &Draft, path: &str) -> Option<FieldDescriptor> {
    let number = |domain: IntegerDomain, unit| FieldDescriptor {
        domain: Domain::Integer(domain),
        default: Some(Value::Integer(domain.default)),
        unit,
        dependent_fields: Vec::new(),
    };
    let names = |values: &[&str]| FieldDescriptor {
        domain: Domain::Values(
            values
                .iter()
                .map(|name| Value::Name((*name).into()))
                .collect(),
        ),
        default: None,
        unit: None,
        dependent_fields: Vec::new(),
    };
    match path {
        "board.model" => return Some(names(&[draft.config.board.id()])),
        "cpu.model" => return Some(names(&[draft.config.cpu.chip.name()])),
        "cpu.fpu" => {
            return Some(names(if draft.config.board == BoardModel::Generic486Isa {
                &["integrated"]
            } else if draft.config.board.is_generic_isa() {
                &["none"]
            } else {
                &["none", "80287"]
            }))
        }
        "board.display_switch" => return Some(names(&["auto", "mono", "colour"])),
        "cpu.rate_steps" | "cpu.rate_pit_clocks" => {
            return Some(number(
                IntegerDomain {
                    min: 1,
                    max: u32::MAX,
                    step: 1,
                    default: 1,
                },
                None,
            ))
        }
        "memory.conventional_kib" => {
            return Some(number(
                IntegerDomain {
                    min: if draft.config.board.is_generic_isa() {
                        640
                    } else {
                        CONVENTIONAL_KIB.min
                    },
                    ..CONVENTIONAL_KIB
                },
                Some("KiB"),
            ))
        }
        "memory.extended_kib" => {
            return Some(number(extended_kib(draft.config.board), Some("KiB")))
        }
        _ => {}
    }
    if let Some((slot_kind, required, _)) = firmware_slot(draft, path) {
        return Some(FieldDescriptor {
            domain: Domain::Values(
                draft
                    .config
                    .board
                    .firmware_catalogue()
                    .iter()
                    .filter(|entry| entry.kind == slot_kind)
                    .map(|entry| Value::Name(entry.id.into()))
                    .chain((!required).then_some(Value::Disconnected))
                    .collect(),
            ),
            default: None,
            unit: None,
            dependent_fields: Vec::new(),
        });
    }
    if let Some(index) = drive_index(draft, path) {
        if !matches!(draft.config.drives[index].kind, DriveKind::Floppy { .. }) {
            return None;
        }
        return Some(FieldDescriptor {
            domain: Domain::Values(
                FLOPPY_TYPES
                    .iter()
                    .map(|kind| Value::Name(kind.name.into()))
                    .collect(),
            ),
            default: None,
            unit: None,
            dependent_fields: Vec::new(),
        });
    }
    if optional_kind(draft, path).is_some() {
        return Some(names(&["installed", "none"]));
    }
    let (id, field) = draft.device_id(path)?;
    let device = draft.config.devices.iter().find(|device| device.id == id)?;
    let kind = draft
        .edits
        .iter()
        .rev()
        .find_map(|edit| {
            if edit.field_path == format!("{}.kind", draft.card_path(id)) {
                if let Value::Name(name) = &edit.value {
                    return CARD_KINDS.iter().copied().find(|kind| kind.name() == name);
                }
            }
            None
        })
        .or_else(|| CardKind::of(&device.kind))?;
    let standard = kind.device();
    let port = draft
        .edits
        .iter()
        .rev()
        .find_map(|edit| {
            if edit.field_path == format!("{}.port", draft.card_path(id)) {
                if let Value::Integer(value) = edit.value {
                    return u16::try_from(value).ok();
                }
            }
            None
        })
        .or(match (&device.kind, kind) {
            (DeviceKind::Uart(cfg), CardKind::Uart16550) => Some(cfg.base),
            (DeviceKind::Ide(cfg), CardKind::Ide) => Some(cfg.base),
            _ => None,
        });
    let default_irq = match &standard.kind {
        DeviceKind::Uart(cfg) => uart_default_irq(port.unwrap_or(cfg.base)),
        DeviceKind::Ide(cfg) => {
            let cfg = IdeCfg {
                base: port.unwrap_or(cfg.base),
                ..*cfg
            };
            Some(cfg.irq())
        }
        _ => standard.irq,
    };
    let mut unit = None;
    let (domain, default) = match (kind, field) {
        (_, "kind") => (
            Domain::Values(
                CARD_KINDS
                    .iter()
                    .filter(|candidate| !draft.preserve_card_roles || candidate.id() == device.id)
                    .map(|kind| Value::Name(kind.name().into()))
                    .collect(),
            ),
            None,
        ),
        (CardKind::Sb16, "base") => (
            integers(SB16_BASES.iter().copied()),
            Some(Value::Integer(SB16_DEFAULT.base.into())),
        ),
        (CardKind::Sb16, "irq") => (
            integers(SB16_IRQS.iter().copied()),
            Some(Value::Integer(SB16_DEFAULT_IRQ.into())),
        ),
        (CardKind::Sb16, "dma8") => (
            integers(SB16_DMA8.iter().copied()),
            Some(Value::Integer(SB16_DEFAULT_DMA8.into())),
        ),
        (CardKind::Sb16, "dma16") => (
            integers(SB16_DMA16.iter().copied()),
            Some(Value::Integer(SB16_DEFAULT.dma16.into())),
        ),
        (CardKind::LotechEms, "port") => (
            integers(EMS_PORTS.iter().copied()),
            Some(Value::Integer(EMS_DEFAULT.port.into())),
        ),
        (CardKind::LotechEms, "frame") => (integers(EMS_FRAMES.iter().copied()), None),
        (CardKind::LotechEms, "size_kib") => {
            unit = Some("KiB");
            (
                integers(EMS_SIZES_KIB.iter().copied()),
                Some(Value::Integer(EMS_DEFAULT.size_kib)),
            )
        }
        (CardKind::Ide, "port") => (
            integers(IDE_PORTS.iter().copied()),
            Some(Value::Integer(IDE_PORTS[0].into())),
        ),
        (CardKind::Uart16550, "port") => (
            Domain::Integer(UART_PORT),
            Some(Value::Integer(UART_PORT.default)),
        ),
        (CardKind::Uart16550 | CardKind::Fdc765 | CardKind::Ide, "irq") => {
            let mut values: Vec<_> = IRQ.values().map(Value::Integer).collect();
            values.push(Value::Disconnected);
            (
                Domain::Values(values),
                default_irq.map(|irq| Value::Integer(irq.into())),
            )
        }
        (CardKind::Fdc765, "dma") => {
            let mut values: Vec<_> = DMA.values().map(Value::Integer).collect();
            values.push(Value::Disconnected);
            (Domain::Values(values), Some(Value::Integer(DMA.default)))
        }
        _ => return None,
    };
    Some(FieldDescriptor {
        domain,
        default,
        unit,
        dependent_fields: if field == "port" && matches!(kind, CardKind::Uart16550 | CardKind::Ide)
        {
            vec![format!("{}.irq", draft.card_path(id))]
        } else {
            Vec::new()
        },
    })
}

fn optional_kind(draft: &Draft, path: &str) -> Option<CardKind> {
    let owner = path.strip_suffix(".enabled")?;
    [
        CardKind::Sb16,
        CardKind::LotechEms,
        CardKind::Mda,
        CardKind::VgaText,
    ]
    .into_iter()
    .find(|kind| owner == draft.card_path(kind.id()) || owner == format!("card[{}]", kind.name()))
}

fn drive_index(draft: &Draft, path: &str) -> Option<usize> {
    draft.config.drives.iter().position(|drive| {
        format!(
            "{}.drives[unit={}].type",
            draft.card_path(drive.controller),
            drive.unit
        ) == path
    })
}

/// The result of applying the entire edit set before any hardware check runs.
#[derive(Clone, Debug)]
pub struct Validation {
    pub config: MachineConfig,
    pub diagnostics: Vec<Diagnostic>,
    pub revision: u64,
}

impl Validation {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
    /// Pending image checks also prevent launch; assembly must revalidate bytes.
    pub fn ready(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

fn field_error(path: &str, code: &'static str, reason: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code,
        field_paths: vec![path.into()],
        conflict_partners: Vec::new(),
        resource: None,
        severity: Severity::Error,
        reason: reason.into(),
    }
}

fn sizes(config: &MachineConfig) -> (u64, u64) {
    (
        config
            .memory
            .ram
            .iter()
            .map(|r| r.end().min(0xA0000).saturating_sub(r.base))
            .sum(),
        config
            .memory
            .ram
            .iter()
            .map(|r| r.end().saturating_sub(r.base.max(0x100000)))
            .sum(),
    )
}

/// Apply semantic edits while maintaining the derived port and MMIO decodes.
/// Resource validation happens only after all edits, permitting atomic swaps.
pub fn validate_draft(draft: &Draft) -> Validation {
    let mut config = draft.config.clone();
    let mut diagnostics = Vec::new();
    let (mut conventional, mut extended) = sizes(&config);
    let mut remap = false;
    let mut pending_images = Vec::new();
    let mut removed_images = Vec::new();
    let mut selected_bios = draft
        .firmware_ids
        .iter()
        .find(|(index, _)| Some(*index) == config.system_bios_image)
        .map(|(_, id)| id.clone())
        .or_else(|| {
            crate::firmware::system_bios(&config)
                .ok()
                .and_then(|image| {
                    crate::firmware::matched_entry(image, config.board.firmware_catalogue())
                })
                .map(|entry| entry.id.to_string())
        });
    let mut edits: Vec<_> = draft.edits.iter().collect();
    edits.sort_by_key(|edit| !edit.field_path.ends_with(".kind"));
    for edit in edits {
        let path = edit.field_path.as_str();
        let Some(descriptor) = describe(draft, path) else {
            diagnostics.push(field_error(
                path,
                "unknown-field",
                "Unknown or inapplicable settings field",
            ));
            continue;
        };
        let in_domain = match &descriptor.domain {
            Domain::Integer(domain) => {
                matches!(edit.value, Value::Integer(value) if domain.contains(value))
            }
            Domain::Values(values) => values.contains(&edit.value),
        };
        if !in_domain {
            diagnostics.push(field_error(
                path,
                "field-domain",
                "Value is outside the supported field domain",
            ));
            continue;
        }
        if optional_kind(draft, path).is_some() {
            remap = true;
            continue;
        }
        match (path, &edit.value) {
            ("board.model" | "cpu.model", _) => continue,
            ("cpu.fpu", Value::Name(name)) => {
                config.cpu.fpu = if name == "none" {
                    FpuChip::None
                } else if name == "integrated" {
                    FpuChip::Integrated
                } else {
                    FpuChip::I80287
                };
                continue;
            }
            ("board.display_switch", Value::Name(name)) => {
                config.display_switch = match name.as_str() {
                    "mono" => DisplaySwitch::Mono,
                    "colour" => DisplaySwitch::Colour,
                    _ => DisplaySwitch::Auto,
                };
                continue;
            }
            ("cpu.rate_steps", Value::Integer(value)) => {
                config.clock.retired_per_pit_clock = u64::from(*value);
                continue;
            }
            ("cpu.rate_pit_clocks", Value::Integer(value)) => {
                config.clock.retired_per_pit_clock_denominator = u64::from(*value);
                continue;
            }
            _ => {}
        }
        if let Some((slot_kind, _, index)) = firmware_slot(draft, path) {
            if edit.value == Value::Disconnected {
                removed_images.extend(index);
            }
            if let Value::Name(name) = &edit.value {
                let entry = config
                    .board
                    .firmware_catalogue()
                    .iter()
                    .find(|entry| entry.id == name);
                if let Some(entry) = entry {
                    let system = slot_kind == FirmwareKind::SystemBios;
                    if index.is_some_and(|index| {
                        crate::firmware::matched_entry(
                            &config.images[index],
                            config.board.firmware_catalogue(),
                        )
                        .is_some_and(|current| current.id == entry.id)
                            || draft
                                .firmware_ids
                                .iter()
                                .any(|(image, id)| *image == index && id == name)
                    }) {
                        if system {
                            selected_bios = Some(name.clone());
                        }
                        continue;
                    }
                    let base = match entry.base {
                        FirmwareBase::Suggested(base) => base,
                        FirmwareBase::TopOfFirstMib => 0x100000 - entry.size,
                    };
                    let image = LoadImage {
                        base,
                        source: ImageSource::File("pending-firmware".into()),
                        kind: ImageKind::Rom,
                        provides: entry.provides.to_vec(),
                    };
                    let index = if let Some(index) = index {
                        config.images[index] = image;
                        index
                    } else {
                        let index = config.images.len();
                        config.images.push(image);
                        index
                    };
                    if system {
                        selected_bios = Some(name.clone());
                        config.system_bios_image = Some(index);
                    }
                    pending_images.push((
                        index,
                        entry.size - entry.rom_offset as u64,
                        path.to_string(),
                    ));
                    diagnostics.push(Diagnostic {
                        severity: Severity::Pending,
                        ..field_error(
                            path,
                            "firmware-pending",
                            "Selected firmware bytes must be supplied and verified",
                        )
                    });
                }
            }
            continue;
        }
        if let Some(index) = drive_index(draft, path) {
            if let Value::Name(name) = &edit.value {
                if let Some(kind) = FLOPPY_TYPES.iter().find(|kind| kind.name == name) {
                    config.drives[index].kind = DriveKind::Floppy {
                        geometry: kind.geometry,
                    };
                }
            }
            continue;
        }
        if let Value::Integer(value) = edit.value {
            match path {
                "memory.conventional_kib" => {
                    conventional = u64::from(value) * 1024;
                    remap = true;
                    continue;
                }
                "memory.extended_kib" => {
                    extended = u64::from(value) * 1024;
                    remap = true;
                    continue;
                }
                _ => {}
            }
        }
        if let Some((id, field)) = draft.device_id(path) {
            let irq_path = format!("{}.irq", draft.card_path(id));
            let follow_default_irq = descriptor.dependent_fields.contains(&irq_path)
                && !draft.edits.iter().any(|edit| edit.field_path == irq_path);
            if let Some(device) = draft
                .config
                .devices
                .iter()
                .position(|device| device.id == id)
                .and_then(|index| config.devices.get_mut(index))
            {
                match (&edit.value, field) {
                    (Value::Name(name), "kind") => {
                        if let Some(kind) = CARD_KINDS.iter().find(|kind| kind.name() == name) {
                            *device = kind.device();
                            remap = true;
                        }
                    }
                    (Value::Disconnected, "irq") => device.irq = None,
                    (Value::Disconnected, "dma") => device.dma = None,
                    (Value::Integer(value), "irq") => device.irq = Some(*value as u8),
                    (Value::Integer(value), "dma" | "dma8") => {
                        device.dma = Some(DmaSpec {
                            controller: DeviceId::Dma,
                            channel: *value as u8,
                        })
                    }
                    (Value::Integer(value), field) => match &mut device.kind {
                        DeviceKind::Sb16(cfg) => {
                            match field {
                                "base" => cfg.base = *value as u16,
                                "dma16" => cfg.dma16 = *value as u8,
                                _ => {}
                            }
                            device.ports = cfg.port_windows();
                        }
                        DeviceKind::LotechEms(cfg) => {
                            match field {
                                "port" => cfg.port = *value as u16,
                                "frame" => {
                                    cfg.frame = *value as u16;
                                    remap = true;
                                }
                                "size_kib" => cfg.size_kib = *value,
                                _ => {}
                            }
                            device.ports = cfg.port_windows();
                        }
                        DeviceKind::Uart(cfg) => {
                            let old_irq = uart_default_irq(cfg.base);
                            cfg.base = *value as u16;
                            if follow_default_irq && old_irq.is_some() && device.irq == old_irq {
                                device.irq = uart_default_irq(cfg.base);
                            }
                            device.ports =
                                vec![PortWindow::new(cfg.base, cfg.base.saturating_add(7))];
                        }
                        DeviceKind::Ide(cfg) => {
                            let old_irq = Some(cfg.irq());
                            cfg.base = *value as u16;
                            if follow_default_irq && device.irq == old_irq {
                                device.irq = Some(cfg.irq());
                            }
                            cfg.control = cfg.base.saturating_add(0x206);
                            device.ports = cfg.port_windows();
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }
    if remap {
        let edited_ids: Vec<_> = draft
            .edits
            .iter()
            .filter_map(|e| draft.device_id(&e.field_path))
            .filter(|(_, field)| *field == "kind" || *field == "frame")
            .map(|(id, _)| id)
            .collect();
        config
            .memory
            .mmio
            .retain(|window| !edited_ids.contains(&window.device));
        for (original, device) in draft.config.devices.iter().zip(&config.devices) {
            if edited_ids.contains(&original.id) {
                if let Some(window) = CardKind::of(&device.kind).and_then(|kind| kind.mmio(device))
                {
                    config.memory.mmio.push(window);
                }
            }
        }
        config.memory.mmio.sort_by_key(|window| window.base);
        config.memory.ram = ram_regions_for_board(
            config.board,
            conventional,
            extended,
            &config.memory.mmio,
            &config.images,
        );
        config.memory.address_space_bytes = 0x100000 + extended;
    }
    let primary_ata = config
        .devices
        .iter()
        .any(|device| matches!(device.kind, DeviceKind::Ide(cfg) if cfg.base == IDE_PORTS[0]));
    for device in &mut config.devices {
        if matches!(device.kind, DeviceKind::Fdc(_)) {
            device.ports = FdcCfg::port_windows(primary_ata);
        }
    }
    let mut located = draft.clone();
    located.card_ids = draft
        .config
        .devices
        .iter()
        .zip(&config.devices)
        .map(|(old, new)| {
            let prefix = draft.card_path(old.id);
            let name = prefix
                .strip_prefix("card[")
                .and_then(|tail| tail.strip_suffix(']'))
                .unwrap_or(role_name(old.id));
            (new.id, name.to_string())
        })
        .collect();
    if draft.preserve_card_roles {
        // A replacement receives a new file identity, so it appends to the inherited cards.
        for edit in &draft.edits {
            let Some((id, "kind")) = draft.device_id(&edit.field_path) else {
                continue;
            };
            let Value::Name(name) = &edit.value else {
                continue;
            };
            let old = draft
                .config
                .devices
                .iter()
                .find(|device| device.id == id)
                .and_then(|device| CardKind::of(&device.kind));
            if old.is_some_and(|kind| kind.name() != name) {
                if let Some(index) = config.devices.iter().position(|device| {
                    device.id == id
                        && CardKind::of(&device.kind).is_some_and(|kind| kind.name() == name)
                }) {
                    let device = config.devices.remove(index);
                    config.devices.push(device);
                }
            }
        }
    }
    let removed: Vec<_> = draft
        .edits
        .iter()
        .filter(|edit| edit.value == Value::Name("none".into()))
        .filter_map(|edit| optional_kind(draft, &edit.field_path).map(CardKind::id))
        .collect();
    if !removed.is_empty() {
        config
            .devices
            .retain(|device| !removed.contains(&device.id));
        config
            .memory
            .mmio
            .retain(|window| !removed.contains(&window.device));
        config.memory.ram = ram_regions_for_board(
            config.board,
            conventional,
            extended,
            &config.memory.mmio,
            &config.images,
        );
    }
    located.config = config.clone();
    for (index, _, path) in &pending_images {
        located.image_paths.retain(|(image, _)| image != index);
        located.image_paths.push((*index, path.clone()));
    }
    let image_indices: Vec<_> = (0..config.images.len())
        .filter(|index| !removed_images.contains(index))
        .collect();
    let image_index = |old: usize| image_indices.iter().position(|index| *index == old);
    if !removed_images.is_empty() {
        config.images = image_indices
            .iter()
            .map(|index| config.images[*index].clone())
            .collect();
        config.system_bios_image = config.system_bios_image.and_then(image_index);
        located.image_paths = located
            .image_paths
            .iter()
            .filter_map(|(index, path)| image_index(*index).map(|index| (index, path.clone())))
            .collect();
        located.config = config.clone();
        pending_images = pending_images
            .into_iter()
            .filter_map(|(index, size, path)| image_index(index).map(|index| (index, size, path)))
            .collect();
    }
    let mut placement = config.clone();
    for (index, size, _) in &pending_images {
        placement.images[*index].source = ImageSource::Blank { size: *size };
    }
    for (original_index, id) in &draft.firmware_ids {
        let Some(index) = image_index(*original_index) else {
            continue;
        };
        if !draft
            .edits
            .iter()
            .any(|edit| edit.field_path == draft.image_path(*original_index))
        {
            if let Some(entry) = config
                .board
                .firmware_catalogue()
                .iter()
                .find(|entry| entry.id == id)
            {
                placement.images[index].source = ImageSource::Blank {
                    size: entry.size - entry.rom_offset as u64,
                };
                if let Err(error) =
                    crate::firmware::check_entry_requirements(&config, config.board, entry, None)
                {
                    diagnostics.push(diagnostic::from_error(&located, &error));
                }
            }
        }
    }
    let catalogue = config.board.firmware_catalogue();
    let mut known = Vec::new();
    for (old_index, id) in &draft.firmware_ids {
        let Some(index) = image_index(*old_index) else {
            continue;
        };
        let path = draft.image_path(*old_index);
        let name = draft
            .edits
            .iter()
            .find(|edit| edit.field_path == path)
            .map_or(Some(id.as_str()), |edit| {
                if let Value::Name(name) = &edit.value {
                    Some(name.as_str())
                } else {
                    None
                }
            });
        if let Some(entry) = name.and_then(|name| catalogue.iter().find(|entry| entry.id == name)) {
            known.push((index, entry));
        }
    }
    for (index, _, path) in &pending_images {
        if !known.iter().any(|(image, _)| image == index) {
            if let Some(entry) = draft
                .edits
                .iter()
                .find(|edit| &edit.field_path == path)
                .and_then(|edit| {
                    if let Value::Name(name) = &edit.value {
                        catalogue.iter().find(|entry| entry.id == name)
                    } else {
                        None
                    }
                })
            {
                known.push((*index, entry));
            }
        }
    }
    if !known.is_empty() {
        if let Some(error) = config.draft_firmware_error(&known) {
            let mut diagnostic = diagnostic::from_error(&located, &error);
            if let ConfigError::FirmwareProvidesMismatch { image_index, .. } = error {
                diagnostic.code = "firmware-provides";
                diagnostic.field_paths = vec![located.image_path(image_index)];
            }
            diagnostics.push(diagnostic);
        }
    }
    for error in placement.draft_image_errors() {
        diagnostics.push(diagnostic::from_error(&located, &error));
    }
    let firmware_pending = config.board != BoardModel::Bare
        && (crate::firmware::system_bios(&config).is_err()
            || crate::firmware::unresolved_roms(&config));
    for edit in &draft.edits {
        if firmware_slot(draft, &edit.field_path).is_some() {
            if let Value::Name(name) = &edit.value {
                if let Some(entry) = config
                    .board
                    .firmware_catalogue()
                    .iter()
                    .find(|entry| entry.id == name)
                {
                    if let Err(error) = crate::firmware::check_entry_requirements(
                        &config,
                        config.board,
                        entry,
                        None,
                    ) {
                        let mut diagnostic = diagnostic::from_error(&located, &error);
                        diagnostic.field_paths = vec![edit.field_path.clone()];
                        diagnostics.push(diagnostic);
                    }
                }
            }
        }
    }
    diagnostics.extend(config.draft_errors().into_iter().map(|error| {
        let pending = firmware_pending
            && matches!(
                error,
                ConfigError::NoProvider { .. } | ConfigError::MissingSystemBios { .. }
            );
        let mut diagnostic = diagnostic::from_error(&located, &error);
        if pending {
            diagnostic.severity = Severity::Pending;
        }
        diagnostic
    }));
    for (index, drive) in config.drives.iter().enumerate() {
        if matches!(drive.image, ImageSource::File(_))
            || matches!(drive.write, WriteMode::Journal { .. })
            || drive.media.as_ref().is_some_and(|list| {
                list.entries.iter().any(|medium| {
                    matches!(medium.image, ImageSource::File(_))
                        || matches!(medium.write, WriteMode::Journal { .. })
                })
            })
        {
            diagnostics.push(Diagnostic {
                severity: Severity::Pending,
                ..field_error(
                    &format!(
                        "{}.drives[unit={}].image",
                        located.card_path(drive.controller),
                        drive.unit
                    ),
                    "media-pending",
                    "Media bytes and persistent paths must be verified before launch",
                )
            });
        }
        if let DriveKind::Floppy { geometry } = drive.kind {
            if let Some(reason) = floppy_disabled_reason(selected_bios.as_deref(), geometry) {
                diagnostics.push(field_error(
                    &format!(
                        "{}.drives[unit={}].type",
                        draft.card_path(drive.controller),
                        drive.unit
                    ),
                    "bios-drive-type",
                    reason,
                ));
            }
        }
        if let Err(error) = drive_image_check(drive, index) {
            diagnostics.push(diagnostic::from_error(&located, &error));
        }
    }
    if config.board != BoardModel::Bare && crate::firmware::system_bios(&config).is_err() {
        diagnostics.push(Diagnostic {
            severity: Severity::Pending,
            ..field_error(
                "firmware.bios",
                "firmware-missing",
                "System firmware must be supplied",
            )
        });
    }
    for (index, image) in config.images.iter().enumerate() {
        if image.kind == ImageKind::Rom
            && matches!(
                image.source,
                ImageSource::File(_) | ImageSource::Interleaved(_)
            )
        {
            let path = located.image_path(index);
            if !diagnostics
                .iter()
                .any(|d| d.field_paths.contains(&path) && d.severity == Severity::Pending)
            {
                diagnostics.push(Diagnostic {
                    severity: Severity::Pending,
                    ..field_error(
                        &path,
                        "firmware-pending",
                        "Firmware bytes must be supplied and verified",
                    )
                });
            }
        }
    }
    // Missing input cannot be a resource conflict, and unrelated choices remain editable.
    diagnostics.retain(|d| !(d.code == "missing-system-bios" && d.severity == Severity::Error));
    let mut unique = Vec::new();
    for diagnostic in diagnostics {
        if !unique.contains(&diagnostic) {
            unique.push(diagnostic);
        }
    }
    Validation {
        config,
        diagnostics: unique,
        revision: draft.revision,
    }
}

fn drive_image_check(drive: &DriveConfig, index: usize) -> Result<(), ConfigError> {
    if let Some(list) = &drive.media {
        for medium in &list.entries {
            if let Some(actual) = medium.image.known_len() {
                drive.kind.check_image_len(index, actual)?;
            }
        }
    }
    Ok(())
}

/// An option's reason is the same structured diagnostic returned on apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub value: Value,
    /// Existing errors in unrelated fields do not constrain this choice.
    pub enabled: bool,
    pub reason: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub revision: u64,
}

/// Enumerate a field's domain. Use [`choices_page`] for large scalar domains.
pub fn choices(draft: &Draft, field_path: &str) -> Vec<Choice> {
    choices_page(draft, field_path, 0, usize::MAX)
}

/// Page an integer domain without allocating or validating all UART bases.
pub fn choices_page(draft: &Draft, path: &str, offset: usize, limit: usize) -> Vec<Choice> {
    let Some(descriptor) = describe(draft, path) else {
        return Vec::new();
    };
    let values: Box<dyn Iterator<Item = Value>> = match descriptor.domain {
        Domain::Integer(domain) => Box::new(domain.values().map(Value::Integer)),
        Domain::Values(values) => Box::new(values.into_iter()),
    };
    let baseline = validate_draft(draft).diagnostics;
    values
        .skip(offset)
        .take(limit)
        .map(|value| {
            let mut candidate = draft.clone();
            candidate.set(path, value.clone());
            let diagnostics = validate_draft(&candidate).diagnostics;
            let error = diagnostics.iter().find(|d| {
                d.severity == Severity::Error
                    && (!baseline.contains(d) || diagnostic_affects_field(d, path))
            });
            Choice {
                value,
                enabled: error.is_none(),
                reason: error.map(|d| d.reason.clone()),
                revision: draft.revision,
                diagnostics,
            }
        })
        .collect()
}

/// Consumers share candidate validation's path relation, including owner-level errors.
pub fn diagnostic_affects_field(diagnostic: &Diagnostic, path: &str) -> bool {
    let related = |field: &str| {
        field == path
            || path
                .strip_prefix(field)
                .is_some_and(|tail| tail.starts_with('.') || tail.starts_with('['))
            || field
                .strip_prefix(path)
                .is_some_and(|tail| tail.starts_with('.') || tail.starts_with('['))
    };
    diagnostic.field_paths.iter().any(|field| related(field))
        || diagnostic.conflict_partners.iter().any(|partner| {
            // Owner identities identify a partner; only a field-level partner
            // constrains a sibling field during an unfinished multi-field edit.
            let owner = (partner.starts_with("card[") || partner.starts_with("board["))
                && partner.ends_with(']');
            partner == path || (!owner && related(partner))
        })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "capabilities_conflict_tests.rs"]
mod conflict_tests;
