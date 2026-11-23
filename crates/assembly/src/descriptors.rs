// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared PC/AT setting domains and effective defaults.
//!
//! IBM Personal Computer AT Technical Reference, System Board, System Memory
//! Map and I/O Address Map; Creative Sound Blaster Series Hardware Programming
//! Guide, §2 DSP I/O Ports and Configuring DMA and Interrupt Settings; Lo-tech
//! 2MB EMS Board schematic r02, address jumpers and SRAM population. These are
//! the existing emulator models, not additional hardware fidelity claims.
//! See `backlog/docs/design/machine-images.md` §6.

use crate::*;

/// An inclusive integer domain, in the field's documented unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntegerDomain {
    pub min: u32,
    pub max: u32,
    pub step: u32,
    pub default: u32,
}

impl IntegerDomain {
    pub const fn contains(self, value: u32) -> bool {
        value >= self.min && value <= self.max && (value - self.min).is_multiple_of(self.step)
    }

    pub fn values(self) -> impl Iterator<Item = u32> {
        (self.min..=self.max).step_by(self.step as usize)
    }
}

pub const CONVENTIONAL_KIB: IntegerDomain = IntegerDomain {
    min: 0,
    max: 640,
    step: 16,
    default: 640,
};
pub const EXTENDED_KIB: IntegerDomain = IntegerDomain {
    min: 0,
    max: IBM5170_MAX_EXTENDED_KIB,
    step: 64,
    default: 0,
};
pub const UART_PORT: IntegerDomain = IntegerDomain {
    min: 0,
    max: 0xFFF8,
    step: 1,
    default: 0x3F8,
};
pub const IRQ: IntegerDomain = IntegerDomain {
    min: 0,
    max: 15,
    step: 1,
    default: 0,
};
pub const DMA: IntegerDomain = IntegerDomain {
    min: 0,
    max: 3,
    step: 1,
    default: 2,
};
pub const SB16_BASES: &[u16] = &[0x220, 0x240, 0x260, 0x280];
pub const SB16_IRQS: &[u8] = &[2, 5, 7, 10];
pub const SB16_DMA8: &[u8] = &[0, 1, 3];
pub const SB16_DMA16: &[u8] = &[5, 6, 7];
pub const IDE_PORTS: &[u16] = &[0x1F0, 0x170];
pub const EMS_PORTS: &[u16] = devices::LotechEms::PORTS;
pub const EMS_FRAMES: &[u16] = devices::LotechEms::FRAMES;
pub const EMS_SIZES_KIB: &[u32] = devices::LotechEms::SIZES_KIB;
pub const SB16_DEFAULT: Sb16Cfg = Sb16Cfg {
    base: 0x220,
    dma16: 5,
};
pub const SB16_DEFAULT_IRQ: u8 = 5;
pub const SB16_DEFAULT_DMA8: u8 = 1;
pub const EMS_DEFAULT: LotechEmsCfg = LotechEmsCfg {
    port: 0x260,
    frame: 0xE000,
    size_kib: 2048,
};

/// The standard COM jumpers supply an IRQ only at these four bases.
pub const fn uart_default_irq(base: u16) -> Option<u8> {
    match base {
        0x3F8 | 0x3E8 => Some(4),
        0x2F8 | 0x2E8 => Some(3),
        _ => None,
    }
}

/// Implemented card models, independent of file syntax and presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    Mda,
    VgaText,
    Vga,
    Uart16550,
    Fdc765,
    Ide,
    Sb16,
    LotechEms,
}

pub const CARD_KINDS: &[CardKind] = &[
    CardKind::Mda,
    CardKind::VgaText,
    CardKind::Vga,
    CardKind::Uart16550,
    CardKind::Fdc765,
    CardKind::Ide,
    CardKind::Sb16,
    CardKind::LotechEms,
];
pub const VIDEO_CARDS: &[CardKind] = &[CardKind::Mda, CardKind::VgaText, CardKind::Vga];

impl CardKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mda => "mda",
            Self::VgaText => "vga-text",
            Self::Vga => "vga",
            Self::Uart16550 => "uart-16550",
            Self::Fdc765 => "fdc-765",
            Self::Ide => "ide",
            Self::Sb16 => "sb16",
            Self::LotechEms => "lotech-ems",
        }
    }

    pub const fn id(self) -> DeviceId {
        match self {
            Self::Mda => DeviceId::Mda,
            Self::VgaText | Self::Vga => DeviceId::Vga,
            Self::Uart16550 => DeviceId::Uart,
            Self::Fdc765 => DeviceId::Fdc,
            Self::Ide => DeviceId::Ide,
            Self::Sb16 => DeviceId::Sb16,
            Self::LotechEms => DeviceId::LotechEms,
        }
    }

    pub fn of(kind: &DeviceKind) -> Option<Self> {
        Some(match kind {
            DeviceKind::Mda(_) => Self::Mda,
            DeviceKind::Vga(_) => Self::VgaText,
            DeviceKind::VgaRegisters(_) => Self::Vga,
            DeviceKind::Uart(_) => Self::Uart16550,
            DeviceKind::Fdc(_) => Self::Fdc765,
            DeviceKind::Ide(_) => Self::Ide,
            DeviceKind::Sb16(_) => Self::Sb16,
            DeviceKind::LotechEms(_) => Self::LotechEms,
            _ => return None,
        })
    }

    pub fn device(self) -> DeviceConfig {
        let (kind, ports) = match self {
            Self::Mda => (
                DeviceKind::Mda(MdaTextCfg {}),
                vec![PortWindow::new(0x3B0, 0x3BB), PortWindow::single(0x3BF)],
            ),
            Self::VgaText => (
                DeviceKind::Vga(VgaTextCfg {}),
                vec![PortWindow::new(0x3D4, 0x3D5), PortWindow::new(0x3D8, 0x3DA)],
            ),
            Self::Vga => (DeviceKind::VgaRegisters(VgaCfg {}), VgaCfg::port_windows()),
            Self::Uart16550 => {
                let base = UART_PORT.default as u16;
                (
                    DeviceKind::Uart(UartCfg { base, mouse: false }),
                    vec![PortWindow::new(base, base + 7)],
                )
            }
            Self::Fdc765 => (
                DeviceKind::Fdc(FdcCfg { drive_slots: 2 }),
                FdcCfg::port_windows(false),
            ),
            Self::Ide => {
                let cfg = IdeCfg {
                    model: AtaControllerModel::Ata,
                    base: IDE_PORTS[0],
                    control: IDE_PORTS[0] + 0x206,
                };
                (DeviceKind::Ide(cfg), cfg.port_windows())
            }
            Self::Sb16 => (DeviceKind::Sb16(SB16_DEFAULT), SB16_DEFAULT.port_windows()),
            Self::LotechEms => (
                DeviceKind::LotechEms(EMS_DEFAULT),
                EMS_DEFAULT.port_windows(),
            ),
        };
        let mut device = DeviceConfig::new(self.id(), kind, ports);
        device.irq = match self {
            Self::Uart16550 => uart_default_irq(UART_PORT.default as u16),
            Self::Fdc765 => Some(6),
            Self::Ide => Some(14),
            Self::Sb16 => Some(SB16_DEFAULT_IRQ),
            _ => None,
        };
        device.dma = match self {
            Self::Fdc765 => Some(DmaSpec {
                controller: DeviceId::Dma,
                channel: DMA.default as u8,
            }),
            Self::Sb16 => Some(DmaSpec {
                controller: DeviceId::Dma,
                channel: SB16_DEFAULT_DMA8,
            }),
            _ => None,
        };
        device
    }

    pub fn mmio(self, device: &DeviceConfig) -> Option<MmioWindow> {
        let (base, size) = match self {
            Self::Mda => (0xB0000, 0x8000),
            Self::VgaText => (0xB8000, 0x8000),
            Self::Vga => (0xA0000, 0x20000),
            Self::LotechEms => match device.kind {
                DeviceKind::LotechEms(cfg) => (u64::from(cfg.frame) << 4, 0x10000),
                _ => return None,
            },
            _ => return None,
        };
        Some(MmioWindow::new(base, size, device.id))
    }
}

/// Named geometries supported by the diskette model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloppyType {
    pub name: &'static str,
    pub geometry: FloppyGeometry,
}
impl FloppyType {
    pub const HD_1_44: Self = Self {
        name: "1.44M",
        geometry: FloppyGeometry::HD_1_44,
    };
    pub const HD_1_2: Self = Self {
        name: "1.2M",
        geometry: FloppyGeometry::HD_1_2,
    };
    pub const DD_720K: Self = Self {
        name: "720K",
        geometry: FloppyGeometry::DD_720K,
    };
    pub const DD_360K: Self = Self {
        name: "360K",
        geometry: FloppyGeometry::DD_360K,
    };
}
pub const FLOPPY_TYPES: &[FloppyType] = &[
    FloppyType::HD_1_44,
    FloppyType::HD_1_2,
    FloppyType::DD_720K,
    FloppyType::DD_360K,
];

/// The standard name of a floppy geometry, if it has one.
pub fn floppy_type_name(geometry: FloppyGeometry) -> Option<&'static str> {
    FLOPPY_TYPES
        .iter()
        .find(|kind| kind.geometry == geometry)
        .map(|kind| kind.name)
}

/// What a drive of type `drive` takes, as errors and dialogs state it:
/// `a 1.44M drive takes 1.44M (1474560 bytes) or 720K (737280 bytes) media`.
pub fn floppy_drive_media(drive: FloppyGeometry) -> String {
    let name = |geometry: FloppyGeometry| {
        floppy_type_name(geometry).map_or_else(
            || {
                format!(
                    "{}/{}/{}",
                    geometry.cylinders, geometry.heads, geometry.sectors_per_track
                )
            },
            str::to_owned,
        )
    };
    let media: Vec<String> = drive
        .accepted_media()
        .into_iter()
        .map(|medium| format!("{} ({} bytes)", name(medium), medium.image_bytes()))
        .collect();
    format!("a {} drive takes {} media", name(drive), media.join(" or "))
}

/// Known firmware restriction on drive types, separate from the media a drive
/// type takes. IBM Personal Computer AT Technical Reference, Diskette BIOS,
/// Drive Types: the June 1985 build has no 1.44 MB drive type. Its 720 KB type
/// is not refused: the BIOS listing excerpt checked here lacks the diskette
/// module that sets up drive types (TEST1.ASM's module list names it), so that
/// support is unverified, not assumed absent. Unknown builds are not guessed.
pub fn floppy_disabled_reason(
    bios: Option<&str>,
    geometry: FloppyGeometry,
) -> Option<&'static str> {
    if bios == Some(crate::firmware::IBM_5170_1985_06_10.id) && geometry == FloppyGeometry::HD_1_44
    {
        Some("The June 1985 IBM BIOS has no 1.44M drive type")
    } else {
        None
    }
}

/// Firmware slots refer to catalogue entries; byte availability is a separate check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirmwareSlot {
    pub path: &'static str,
    pub kind: FirmwareKind,
    pub required: bool,
}
pub const FIRMWARE_SLOTS: &[FirmwareSlot] = &[
    FirmwareSlot {
        path: "firmware.bios",
        kind: FirmwareKind::SystemBios,
        required: true,
    },
    FirmwareSlot {
        path: "firmware.option_roms",
        kind: FirmwareKind::OptionRom,
        required: false,
    },
    FirmwareSlot {
        path: "firmware.video",
        kind: FirmwareKind::VideoBios,
        required: false,
    },
];

/// Rebuild the file-format RAM map after a semantic memory or card edit.
/// Expansion scan space stays read-only; adapter RAM fills only the video gap.
pub fn ram_regions(
    conventional: u64,
    extended: u64,
    mmio: &[MmioWindow],
    images: &[LoadImage],
) -> Vec<RamRegion> {
    let mut regions = vec![(0xA0000, 0xC0000)];
    for (base, end) in mmio
        .iter()
        .map(|w| (w.base, w.end()))
        .chain(images.iter().map(|image| {
            (
                image.base,
                image.base + image.source.known_len().unwrap_or(0),
            )
        }))
    {
        regions = regions
            .into_iter()
            .flat_map(|(start, stop)| {
                if base >= stop || end <= start {
                    return vec![(start, stop)];
                }
                vec![(start, base.max(start)), (end.min(stop), stop)]
            })
            .filter(|(start, stop)| start < stop)
            .collect();
    }
    regions.extend([(0, conventional), (0x100000, 0x100000 + extended)]);
    regions.sort_unstable();
    let mut joined: Vec<RamRegion> = Vec::new();
    for (base, end) in regions.into_iter().filter(|(base, end)| base < end) {
        if let Some(previous) = joined.last_mut().filter(|region| region.end() == base) {
            previous.size = end - previous.base;
        } else {
            joined.push(RamRegion::new(base, end - base));
        }
    }
    joined
}

/// RAM population for the selected board. SeaBIOS rel-1.17.0 shadow.c,
/// make_bios_writable and qemu_prep_reset require writable firmware workspace.
/// See `backlog/docs/design/machine-files.md` §Generic 386 ISA board.
pub fn ram_regions_for_board(
    board: BoardModel,
    conventional: u64,
    extended: u64,
    mmio: &[MmioWindow],
    images: &[LoadImage],
) -> Vec<RamRegion> {
    let mut ram = ram_regions(conventional, extended, mmio, images);
    if board == BoardModel::Generic386Isa {
        ram.push(RamRegion::new(0xC0000, 0x40000));
        ram.sort_by_key(|r| r.base);
    }
    ram
}

/// Extended-memory guest population domain. The 386 cap bounds populated low
/// memory at 64 MiB. Sparse backing commits mapped population (plus host-page
/// rounding) within a separately reserved 4 GiB physical address span.
pub const fn extended_kib(board: BoardModel) -> IntegerDomain {
    IntegerDomain {
        max: match board {
            BoardModel::Generic386Isa => GENERIC386_MAX_EXTENDED_KIB,
            _ => IBM5170_MAX_EXTENDED_KIB,
        },
        ..EXTENDED_KIB
    }
}
