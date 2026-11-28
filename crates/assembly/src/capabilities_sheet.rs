// SPDX-License-Identifier: AGPL-3.0-or-later
//! Effective PC/AT property fields and metadata-only draft construction.
//!
//! IBM Personal Computer AT Technical Reference, System Board, System Memory
//! Map and System Interrupts. No image bytes are trusted by this projection;
//! launch still requires normal image verification. See
//! `backlog/docs/design/machine-images.md` §6–7.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    System,
    Memory,
    Display,
    Sound,
    Storage,
    Expansion,
    Firmware,
}

impl Group {
    pub const ALL: [Self; 7] = [
        Self::System,
        Self::Memory,
        Self::Display,
        Self::Sound,
        Self::Storage,
        Self::Expansion,
        Self::Firmware,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Memory => "Memory",
            Self::Display => "Display",
            Self::Sound => "Sound",
            Self::Storage => "Storage",
            Self::Expansion => "Expansion",
            Self::Firmware => "Firmware",
        }
    }
}

/// A presentation field; domains and reasons remain available through `describe` and `choices`.
#[derive(Clone, Debug)]
pub struct Field {
    pub path: String,
    pub label: String,
    pub group: Group,
    pub value: Value,
    pub heading: String,
}

impl Draft {
    /// Construct a pending board description without opening firmware or media.
    pub fn for_board(board: BoardModel) -> Self {
        Self::new(MachineConfig {
            board,
            display_switch: DisplaySwitch::Auto,
            cpu: CpuConfig {
                engine: crate::CpuEngine::Interpreter,
                chip: board.cpu_chip(),
                fpu: if board == BoardModel::Generic486Isa {
                    FpuChip::Integrated
                } else {
                    FpuChip::None
                },
                reset: ResetSpec::reset_vector(),
                segments: SegmentSeed::flat(0xF000),
            },
            clock: board.default_clock(),
            memory: MemoryConfig {
                address_space_bytes: 0x100000 + u64::from(EXTENDED_KIB.default) * 1024,
                ram: ram_regions_for_board(
                    board,
                    u64::from(CONVENTIONAL_KIB.default) * 1024,
                    u64::from(EXTENDED_KIB.default) * 1024,
                    &[],
                    &[],
                ),
                mmio: Vec::new(),
            },
            devices: board.motherboard_devices(),
            drives: Vec::new(),
            images: Vec::new(),
            system_bios_image: None,
        })
    }

    /// Relocate the POST latch while keeping the board's DMA page decode intact.
    /// IBM Personal Computer AT Technical Reference, System Board, I/O Address Map.
    pub fn configure_post(&mut self, port: Option<u16>) {
        if let Some(post) = self
            .config
            .devices
            .iter_mut()
            .find(|device| device.id == DeviceId::Post)
        {
            if let Some(port) = port {
                post.kind = DeviceKind::Post(PostPortCfg { port });
                post.ports = vec![PortWindow::single(port)];
            }
        }
        if port.is_none() {
            self.config
                .devices
                .retain(|device| device.id != DeviceId::Post);
        }
        if port != Some(0x80) {
            if let Some(dma) = self
                .config
                .devices
                .iter_mut()
                .find(|device| device.id == DeviceId::Dma)
            {
                for window in dma.ports.iter_mut().filter(|window| window.start == 0x81) {
                    window.start = 0x80;
                }
            }
        }
    }

    /// Add a file identity using the same hardware defaults as machine lowering.
    pub fn add_card(&mut self, name: String, kind: CardKind) {
        let device = kind.device();
        if let Some(window) = kind.mmio(&device) {
            self.config.memory.mmio.push(window);
        }
        self.card_ids.push((device.id, name));
        self.config.devices.push(device);
    }
}

/// Enumerate effective fields, including fields with no alternative hardware model.
pub fn fields(draft: &Draft) -> Vec<Field> {
    let config = validate_draft(draft).config;
    let mut fields = Vec::new();
    let mut push = |path: String, label: &str, group, value| {
        fields.push(Field {
            path,
            label: label.into(),
            group,
            value,
            heading: String::new(),
        })
    };
    push(
        "board.model".into(),
        "Board",
        Group::System,
        Value::Name(config.board.id().into()),
    );
    push(
        "cpu.model".into(),
        "Processor",
        Group::System,
        Value::Name(config.cpu.chip.name().into()),
    );
    push(
        "cpu.fpu".into(),
        "Coprocessor",
        Group::System,
        Value::Name(
            if config.cpu.fpu == FpuChip::None {
                "none"
            } else if config.cpu.fpu == FpuChip::I80387 {
                "80387"
            } else if config.cpu.fpu == FpuChip::Integrated {
                "integrated"
            } else {
                "80287"
            }
            .into(),
        ),
    );
    push(
        "cpu.rate_steps".into(),
        "Retired instructions",
        Group::System,
        Value::Integer(config.clock.retired_per_pit_clock as u32),
    );
    push(
        "cpu.rate_pit_clocks".into(),
        "Per PIT clocks",
        Group::System,
        Value::Integer(config.clock.retired_per_pit_clock_denominator as u32),
    );
    push(
        "board.display_switch".into(),
        "Display switch",
        Group::Display,
        Value::Name(
            match config.display_switch {
                DisplaySwitch::Auto => "auto",
                DisplaySwitch::Mono => "mono",
                DisplaySwitch::Colour => "colour",
            }
            .into(),
        ),
    );
    let (conventional, extended) = sizes(&config);
    push(
        "memory.conventional_kib".into(),
        "Conventional memory",
        Group::Memory,
        Value::Integer((conventional / 1024) as u32),
    );
    push(
        "memory.extended_kib".into(),
        "Extended memory",
        Group::Memory,
        Value::Integer((extended / 1024) as u32),
    );
    for old in &draft.config.devices {
        let replacement = draft
            .edits
            .iter()
            .find(|edit| edit.field_path == format!("{}.kind", draft.card_path(old.id)))
            .and_then(|edit| {
                if let Value::Name(name) = &edit.value {
                    CARD_KINDS
                        .iter()
                        .find(|kind| kind.name() == name)
                        .map(|kind| kind.id())
                } else {
                    None
                }
            })
            .unwrap_or(old.id);
        let Some(device) = config
            .devices
            .iter()
            .find(|device| device.id == replacement)
        else {
            continue;
        };
        let Some(kind) = CardKind::of(&device.kind) else {
            continue;
        };
        let prefix = draft.card_path(old.id);
        let group = match kind {
            CardKind::Mda | CardKind::Vga | CardKind::VgaText => Group::Display,
            CardKind::Sb16 => Group::Sound,
            CardKind::Ide | CardKind::Fdc765 => Group::Storage,
            _ => Group::Expansion,
        };
        let mut field = |suffix: &str, label: &str, value| {
            push(format!("{prefix}.{suffix}"), label, group, value)
        };
        field("kind", "Card", Value::Name(kind.name().into()));
        match device.kind {
            DeviceKind::Sb16(cfg) => {
                field("base", "I/O port", Value::Integer(cfg.base.into()));
                field("dma16", "16-bit DMA", Value::Integer(cfg.dma16.into()));
            }
            DeviceKind::LotechEms(cfg) => {
                field("port", "I/O port", Value::Integer(cfg.port.into()));
                field("frame", "Page frame", Value::Integer(cfg.frame.into()));
                field("size_kib", "Memory", Value::Integer(cfg.size_kib));
            }
            DeviceKind::VgaRegisters(_) => field("model", "Model", Value::Name("ibm".into())),
            DeviceKind::BochsVbe(cfg) => {
                field("model", "Model", Value::Name("bochs-vbe".into()));
                field("memory_mib", "Video memory", Value::Integer(cfg.memory_mib));
            }
            DeviceKind::Uart(cfg) => field("port", "I/O port", Value::Integer(cfg.base.into())),
            DeviceKind::Ide(cfg) => field("port", "I/O port", Value::Integer(cfg.base.into())),
            _ => {}
        }
        if describe(draft, &format!("{prefix}.irq")).is_some() {
            field(
                "irq",
                "IRQ",
                device
                    .irq
                    .map_or(Value::Disconnected, |irq| Value::Integer(irq.into())),
            );
        }
        for (suffix, label) in [("dma", "DMA"), ("dma8", "8-bit DMA")] {
            if describe(draft, &format!("{prefix}.{suffix}")).is_some() {
                field(
                    suffix,
                    label,
                    device.dma.map_or(Value::Disconnected, |dma| {
                        Value::Integer(dma.channel.into())
                    }),
                );
            }
        }
    }
    for (kind, group, label) in [
        (CardKind::Sb16, Group::Sound, "Sound card"),
        (CardKind::LotechEms, Group::Expansion, "EMS board"),
        (CardKind::Mda, Group::Display, "Monochrome adapter"),
        (CardKind::VgaText, Group::Display, "VGA adapter"),
    ] {
        let owner = if draft
            .config
            .devices
            .iter()
            .any(|device| device.id == kind.id())
        {
            draft.card_path(kind.id())
        } else {
            format!("card[{}]", kind.name())
        };
        let installed = config.devices.iter().any(|device| device.id == kind.id());
        push(
            format!("{owner}.enabled"),
            label,
            group,
            Value::Name(if installed { "installed" } else { "none" }.into()),
        );
    }
    for drive in &config.drives {
        if let DriveKind::Floppy { geometry } = drive.kind {
            if let Some(kind) = FLOPPY_TYPES.iter().find(|kind| kind.geometry == geometry) {
                push(
                    format!(
                        "{}.drives[unit={}].type",
                        draft.card_path(drive.controller),
                        drive.unit
                    ),
                    &format!("Drive {}:", char::from(b'A' + drive.unit)),
                    Group::Storage,
                    Value::Name(kind.name.into()),
                );
            }
        }
    }
    let mut paths: Vec<_> = draft
        .image_paths
        .iter()
        .map(|(_, path)| path.clone())
        .collect();
    for slot in FIRMWARE_SLOTS {
        if !draft
            .image_paths
            .iter()
            .any(|(index, _)| firmware_kind(draft, *index) == slot.kind)
        {
            paths.push(slot.path.into());
        }
    }
    for path in paths {
        let index = draft
            .image_paths
            .iter()
            .find(|(_, field)| field == &path)
            .map(|(index, _)| *index);
        let kind = firmware_slot(draft, &path).map_or(FirmwareKind::OptionRom, |(kind, _, _)| kind);
        let value = draft
            .edits
            .iter()
            .find(|edit| edit.field_path == path)
            .map(|edit| edit.value.clone())
            .or_else(|| {
                index.and_then(|index| {
                    draft
                        .firmware_ids
                        .iter()
                        .find(|(image, _)| *image == index)
                        .map(|(_, id)| Value::Name(id.clone()))
                        .or_else(|| {
                            draft
                                .firmware_files
                                .iter()
                                .find(|(image, _)| *image == index)
                                .map(|(_, name)| Value::FileName(name.clone()))
                        })
                })
            })
            .unwrap_or(Value::Disconnected);
        push(
            path,
            match kind {
                FirmwareKind::SystemBios => "System BIOS",
                FirmwareKind::VideoBios => "Video BIOS",
                FirmwareKind::OptionRom => "Option ROM",
            },
            Group::Firmware,
            value,
        );
    }
    for field in &mut fields {
        if let Some(edit) = draft
            .edits
            .iter()
            .find(|edit| edit.field_path == field.path)
        {
            field.value = edit.value.clone();
        }
    }
    for field in &mut fields {
        field.heading = field_heading(draft, field);
    }
    fields
}

fn field_heading(draft: &Draft, field: &Field) -> String {
    if field.path.starts_with("cpu.") {
        return "Processor".into();
    }
    if field.path.starts_with("memory.") {
        return "System memory".into();
    }
    if field.path.starts_with("board.") {
        return "System board".into();
    }
    if let Some(index) = drive_index(draft, &field.path) {
        return format!(
            "Floppy drive {}:",
            char::from(b'A' + draft.config.drives[index].unit)
        );
    }
    if let Some((id, _)) = draft.device_id(&field.path) {
        if let Some(kind) = draft
            .config
            .devices
            .iter()
            .find(|device| device.id == id)
            .and_then(|device| CardKind::of(&device.kind))
        {
            return card_label(kind).into();
        }
    }
    if field.path.contains("sb16]") {
        return card_label(CardKind::Sb16).into();
    }
    if field.path.contains("lotech-ems]") {
        return card_label(CardKind::LotechEms).into();
    }
    field.group.label().into()
}

fn card_label(kind: CardKind) -> &'static str {
    match kind {
        CardKind::Mda => "Monochrome display adapter",
        CardKind::VgaText => "VGA text adapter",
        CardKind::Vga => "VGA adapter",
        CardKind::Uart16550 => "Serial port",
        CardKind::Fdc765 => "Floppy controller",
        CardKind::Ide => "IDE controller",
        CardKind::Sb16 => "Sound Blaster 16",
        CardKind::LotechEms => "Lo-tech EMS board",
    }
}

/// Human labels are shared by every capabilities consumer.
pub fn value_label(draft: &Draft, path: &str, value: &Value) -> String {
    match value {
        Value::Integer(value)
            if path.ends_with(".base") || path.ends_with(".port") || path.ends_with(".frame") =>
        {
            format!("{value:X}h")
        }
        Value::Integer(value) => value.to_string(),
        Value::Disconnected => "None".into(),
        Value::FileName(name) => format!("File: {name} (unverified)"),
        Value::Name(name) => {
            if let Some(entry) = draft
                .config
                .board
                .firmware_catalogue()
                .iter()
                .find(|entry| entry.id == name)
            {
                return firmware_label(entry);
            }
            if let Some(kind) = CARD_KINDS.iter().find(|kind| kind.name() == name) {
                return card_label(*kind).into();
            }
            match name.as_str() {
                "ibm-5170" => "IBM 5170 (AT)",
                "80286" => "Intel 80286",
                "80287" => "Intel 80287",
                "80387" => "Intel 80387",
                "installed" => "Installed",
                "none" => "None",
                "auto" => "Automatic",
                "mono" => "Monochrome",
                "colour" => "Colour",
                "1.44M" => "3.5 inch 1.44 MB",
                "1.2M" => "5.25 inch 1.2 MB",
                "720K" => "3.5 inch 720 KB",
                "360K" => "5.25 inch 360 KB",
                _ => name,
            }
            .into()
        }
    }
}
/// A superseded catalogue entry keeps its label, marked as superseded.
fn firmware_label(entry: &FirmwareEntry) -> String {
    let label = catalogue_label(entry);
    if entry.status == crate::FirmwareStatus::Superseded {
        format!("{label} (superseded)")
    } else {
        label
    }
}

fn catalogue_label(entry: &FirmwareEntry) -> String {
    let words = entry.id.replace('-', " ");
    if words.starts_with("ibm 5170 bios ") {
        format!(
            "IBM 5170 BIOS {}",
            entry.id.trim_start_matches("ibm-5170-bios-")
        )
    } else if words.starts_with("kiselev ") {
        if entry.id.ends_with("-atreset") {
            "Kiselev Xi 8088 BIOS 1.0.2 (AT disk reset)".into()
        } else {
            "Kiselev Xi 8088 BIOS 1.0.2".into()
        }
    } else if words.starts_with("xub ") {
        format!(
            "XTIDE Universal BIOS {}",
            entry.id.trim_start_matches("xub-")
        )
    } else if words.starts_with("ibm vga") {
        "IBM VGA BIOS (1987)".into()
    } else if words.starts_with("unemu at bios ") {
        format!(
            "unemu AT BIOS {}",
            entry.id.trim_start_matches("unemu-at-bios-")
        )
    } else if words.starts_with("vgabios ") {
        let version = entry.id.trim_start_matches("vgabios-");
        match version.strip_suffix("-std") {
            Some(version) => format!("unemu VGA BIOS {version} (std)"),
            None => format!("unemu VGA BIOS {version}"),
        }
    } else {
        words
    }
}

/// Read-only drive summaries use the same effective configuration as field choices.
pub fn storage_summary(draft: &Draft) -> Vec<String> {
    validate_draft(draft)
        .config
        .drives
        .iter()
        .map(|drive| {
            let name = match drive.kind {
                DriveKind::Floppy { .. } => {
                    format!("Floppy drive {}:", char::from(b'A' + drive.unit))
                }
                DriveKind::HardDisk { .. } => {
                    format!("IDE {}", if drive.unit == 0 { "master" } else { "slave" })
                }
            };
            let policy = if let Some(media) = &drive.media {
                format!("{} catalogue images", media.entries.len())
            } else {
                match drive.write {
                    WriteMode::Volatile => "Volatile".into(),
                    WriteMode::ReadOnly => "Read-only".into(),
                    WriteMode::Journal { .. } => "Journal".into(),
                }
            };
            format!(
                "{name} · {} KiB · {policy}",
                drive.kind.image_bytes() / 1024
            )
        })
        .collect()
}

/// A selected catalogue image's file binding, without host discovery.
pub struct FirmwareBinding {
    pub filename: String,
    pub sha256: &'static str,
    pub base: u64,
    pub offset: usize,
    pub provides: &'static [FirmwareService],
}

pub fn firmware_binding(entry: &FirmwareEntry) -> FirmwareBinding {
    FirmwareBinding {
        filename: format!("{}.bin", entry.id),
        sha256: entry.sha256,
        base: match entry.base {
            FirmwareBase::Suggested(base) => base,
            FirmwareBase::TopOfFirstMib => 0x100000 - entry.size,
        },
        offset: entry.rom_offset,
        provides: entry.provides,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_firmware_ids_get_readable_labels() {
        assert_eq!(
            firmware_label(&crate::firmware::UNEMU_AT_BIOS_1_0_2_UNEMU_3),
            "unemu AT BIOS 1.0.2-unemu.3"
        );
        assert_eq!(
            firmware_label(&crate::firmware::UNEMU_AT_BIOS_1_0_2_UNEMU_2),
            "unemu AT BIOS 1.0.2-unemu.2 (superseded)"
        );
        assert_eq!(
            firmware_label(&FirmwareEntry::VGABIOS_0_9D_UNEMU_1_STD),
            "unemu VGA BIOS 0.9d-unemu.1 (std)"
        );
        assert_eq!(
            firmware_label(&FirmwareEntry::VGABIOS_0_8A_UNEMU_1_STD),
            "unemu VGA BIOS 0.8a-unemu.1 (std) (superseded)"
        );
        assert_eq!(
            firmware_label(&crate::KISELEV_XI8088),
            "Kiselev Xi 8088 BIOS 1.0.2"
        );
    }
}
