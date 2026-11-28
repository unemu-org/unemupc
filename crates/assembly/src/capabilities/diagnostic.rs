// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured locations for existing PC/AT validation errors.
//!
//! Resource numbers retain the validator's routed IRQs and decoded aliases.
//! See `backlog/docs/design/machine-images.md` §6 and IBM Personal Computer AT
//! Technical Reference, System Board, System Interrupts and I/O Address Map.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    Port(u16),
    Irq(u8),
    Dma { controller: DeviceId, channel: u8 },
    Memory { start: u64, end: u64 },
}

/// Stable code and paths are separate from human-readable wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub field_paths: Vec<String>,
    pub conflict_partners: Vec<String>,
    pub resource: Option<Resource>,
    pub severity: Severity,
    pub reason: String,
}

fn device_name(id: DeviceId) -> &'static str {
    match id {
        DeviceId::Fdc => "floppy controller",
        DeviceId::Sb16 => "Sound Blaster 16",
        DeviceId::LotechEms => "EMS board",
        DeviceId::Vga => "VGA card",
        DeviceId::Mda => "monochrome adapter",
        DeviceId::Uart => "serial port",
        DeviceId::Ide => "IDE controller",
        DeviceId::Dma | DeviceId::DmaWord => "DMA controller",
        DeviceId::Pit => "system timer",
        DeviceId::Keyboard => "keyboard controller",
        DeviceId::Rtc => "real-time clock",
        _ => "system-board device",
    }
}

fn image_name(draft: &Draft, index: usize) -> &'static str {
    let path = draft.image_path(index);
    if path == "firmware.bios" {
        "system BIOS"
    } else if path == "firmware.video"
        || draft
            .config
            .images
            .get(index)
            .is_some_and(|image| (0xC0000..0xC8000).contains(&image.base))
    {
        "VGA BIOS"
    } else {
        "option ROM"
    }
}

fn memory_field(draft: &Draft, id: DeviceId) -> String {
    let field = if draft
        .config
        .devices
        .iter()
        .any(|device| device.id == id && matches!(device.kind, DeviceKind::LotechEms(_)))
    {
        "frame"
    } else {
        "kind"
    };
    format!("{}.{field}", draft.card_path(id))
}

pub(super) fn from_error(draft: &Draft, error: &ConfigError) -> Diagnostic {
    use ConfigError::*;
    let card = |id, field: &str| format!("{}.{field}", draft.card_path(id));
    let drive = |index: usize, field: &str| {
        draft
            .config
            .drives
            .get(index)
            .map(|drive| {
                format!(
                    "{}.drives[unit={}].{field}",
                    draft.card_path(drive.controller),
                    drive.unit
                )
            })
            .unwrap_or_else(|| format!("drives[{index}].{field}"))
    };
    let pair = |a, b, field: &str| vec![card(a, field), card(b, field)];
    let port_field = |id| {
        if draft
            .config
            .devices
            .iter()
            .any(|device| device.id == id && matches!(device.kind, DeviceKind::Sb16(_)))
        {
            "base"
        } else {
            "port"
        }
    };
    let dma_field = |id, controller| {
        if draft
            .config
            .devices
            .iter()
            .any(|device| device.id == id && matches!(device.kind, DeviceKind::Sb16(_)))
        {
            if controller == DeviceId::DmaWord {
                "dma16"
            } else {
                "dma8"
            }
        } else {
            "dma"
        }
    };
    let mut reason = error.to_string();
    let (code, fields, resource) = match error {
        PortOverlap { a, b, port } | PortOverlapDecodeOrder { a, b, port, .. } => {
            reason = format!(
                "{port:03X}h is shared by {} and {}",
                draft.card_path(*a),
                draft.card_path(*b)
            );
            (
                "port-conflict",
                vec![card(*a, port_field(*a)), card(*b, port_field(*b))],
                Some(Resource::Port(*port)),
            )
        }
        IrqConflict { a, b, line } => {
            reason = format!(
                "IRQ {line} is shared by {} and {}",
                draft.card_path(*a),
                draft.card_path(*b)
            );
            (
                "irq-conflict",
                pair(*a, *b, "irq"),
                Some(Resource::Irq(*line)),
            )
        }
        DmaConflict {
            a,
            b,
            controller,
            channel,
        } => {
            reason = format!(
                "DMA {channel} is shared by the {} and the {}",
                device_name(*a),
                device_name(*b)
            );
            (
                "dma-conflict",
                vec![
                    card(*a, dma_field(*a, *controller)),
                    card(*b, dma_field(*b, *controller)),
                ],
                Some(Resource::Dma {
                    controller: *controller,
                    channel: *channel,
                }),
            )
        }
        MmioOverlap { a, b, addr } => {
            reason = format!(
                "Memory at {addr:05X}h is shared by the {} and the {}",
                device_name(*a),
                device_name(*b)
            );
            (
                "memory-conflict",
                vec![memory_field(draft, *a), memory_field(draft, *b)],
                Some(Resource::Memory {
                    start: *addr,
                    end: addr.saturating_add(1),
                }),
            )
        }
        ImageOverlapsMmio {
            image_index,
            device,
            addr,
        } => {
            reason = format!(
                "The {} at {addr:05X}h overlaps the {}",
                image_name(draft, *image_index),
                device_name(*device)
            );
            (
                "rom-mmio-conflict",
                vec![draft.image_path(*image_index), memory_field(draft, *device)],
                Some(Resource::Memory {
                    start: *addr,
                    end: addr.saturating_add(1),
                }),
            )
        }
        ImageOverlap { a, b, addr } => {
            reason = format!(
                "The {} and the {} overlap at {addr:05X}h",
                image_name(draft, *a),
                image_name(draft, *b)
            );
            (
                "rom-conflict",
                vec![draft.image_path(*a), draft.image_path(*b)],
                Some(Resource::Memory {
                    start: *addr,
                    end: addr.saturating_add(1),
                }),
            )
        }
        MmioOverlapsRam { device, addr } => {
            reason = format!("The {} overlaps RAM at {addr:05X}h", device_name(*device));
            (
                "ram-mmio-conflict",
                vec![memory_field(draft, *device), "memory".into()],
                Some(Resource::Memory {
                    start: *addr,
                    end: addr.saturating_add(1),
                }),
            )
        }
        LotechEmsFrameInSystemRom { device } => {
            reason = "E000h is system-board ROM space on the IBM 5170".into();
            (
                "ems-system-rom",
                vec![card(*device, "frame")],
                Some(Resource::Memory {
                    start: 0xE0000,
                    end: 0xF0000,
                }),
            )
        }
        InvalidSb16Resources { device } => ("sb16-resources", vec![draft.card_path(*device)], None),
        InvalidLotechEmsResources { device } => {
            ("ems-resources", vec![draft.card_path(*device)], None)
        }
        InvalidBochsVbeResources { device } => {
            ("vbe-resources", vec![card(*device, "memory_mib")], None)
        }
        BochsVbeAddressSpace { device, .. } => {
            ("vbe-address-space", vec![card(*device, "model")], None)
        }
        BochsVbeOverlap { device, addr } => (
            "vbe-overlap",
            vec![card(*device, "model")],
            Some(Resource::Memory {
                start: *addr,
                end: addr.saturating_add(1),
            }),
        ),
        InvalidIdeResources { device } | InvalidIdePortWindows { device } => {
            ("ide-resources", vec![draft.card_path(*device)], None)
        }
        UartPortWindowOutOfSpace { device, .. } => ("uart-port", vec![card(*device, "port")], None),
        BoardCardIrq { device, line, .. } | InvalidIrqLine { device, line } => (
            "irq-domain",
            vec![card(*device, "irq")],
            Some(Resource::Irq(*line)),
        ),
        InvalidDmaChannel {
            device,
            controller,
            channel,
        } => (
            "dma-domain",
            vec![card(*device, dma_field(*device, *controller))],
            Some(Resource::Dma {
                controller: *controller,
                channel: *channel,
            }),
        ),
        BoardCardDmaController { device, .. } | BoardCardDma { device, .. } => {
            ("board-dma", vec![card(*device, "dma")], None)
        }
        DmaControllerMissing {
            requester,
            controller,
        } => (
            "dma-controller-missing",
            vec![draft.card_path(*requester), draft.card_path(*controller)],
            None,
        ),
        ConflictingVgaCards => ("video-conflict", vec![card(DeviceId::Vga, "kind")], None),
        DuplicateDeviceId(id) => ("duplicate-card", vec![draft.card_path(*id)], None),
        ExtendedRamLimit { .. } => (
            "extended-memory-limit",
            vec!["memory.extended_kib".into()],
            None,
        ),
        AdapterSpaceOverlapsRam { .. }
        | HighRomAliasOverlap { .. }
        | BoardPhysicalAddressOutOfRange { .. }
        | HighRomAliasSourceMmio { .. } => ("board-memory", vec!["memory".into()], None),
        MissingSystemBios { .. } => ("missing-system-bios", vec!["firmware.bios".into()], None),
        BoardDeviceMissing { device, .. }
        | BoardDeviceOrder { device, .. }
        | BoardDeviceWiring { device, .. } => {
            ("board-wiring", vec![draft.card_path(*device)], None)
        }
        ImageGeometryMismatch { drive_index, .. }
        | UnsupportedFloppySectorSize { drive_index, .. }
        | InvalidAtaGeometry { drive_index } => {
            ("drive-geometry", vec![drive(*drive_index, "type")], None)
        }
        DriveControllerMissing { drive_index, .. }
        | DriveControllerKind { drive_index, .. }
        | DriveUnitUnavailable { drive_index, .. }
        | AtaMissingMaster { drive_index, .. } => {
            ("drive-binding", vec![drive(*drive_index, "unit")], None)
        }
        NoProvider { device, .. } => (
            "firmware-service",
            vec![draft.card_path(*device), "firmware".into()],
            None,
        ),
        FirmwareReset { .. } => ("firmware-reset", vec!["cpu".into()], None),
        SystemBiosOverlapsRam { .. } => (
            "bios-ram-conflict",
            vec!["firmware.bios".into(), "memory".into()],
            None,
        ),
        RegionOutOfSpace { .. } => ("address-space", vec!["memory".into()], None),
        _ => ("configuration", vec!["machine".into()], None),
    };
    let conflict_partners = if fields.len() > 1 {
        fields
            .iter()
            .map(|field| {
                if field.starts_with("firmware.") || field.starts_with("images[") {
                    field.clone()
                } else {
                    field
                        .rsplit_once('.')
                        .map_or(field.as_str(), |(owner, _)| owner)
                        .to_string()
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Diagnostic {
        code,
        field_paths: fields,
        conflict_partners,
        resource,
        severity: Severity::Error,
        reason,
    }
}
