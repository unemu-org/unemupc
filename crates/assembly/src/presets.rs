// SPDX-License-Identifier: AGPL-3.0-or-later
//! Machine-shape presets (§3.1).
//!
//! A preset is a plain function that emits a [`MachineConfig`] for one machine
//! shape — the ergonomic layer over the flat [`DeviceConfig`] list. Presets own
//! the devices, ports, IRQ/DMA wiring, memory mosaic, reset entry and clock.
//! The BIOS preset uses independent MDA and VGA cards, correcting the shared
//! cursor state of its legacy runner. The frontend picks a preset, assembles it,
//! and runs it; nothing here runs a machine.
//!
//! The old `AtChipset` / `SerialChipset` chipsets are exactly these presets in a
//! different guise: where the chipset trait returned a device set + decode map,
//! a preset returns the pure-data config the assembler realises into the same
//! parts. Board presets and bare rigs with an A20 source use the I286 machine
//! profile; source-free bare rigs retain step-bounded Application(Real16).

use crate::board::BoardModel;
use crate::clock::ClockConfig;
use crate::cpu::{CpuChip, CpuConfig, ResetSpec, SegmentSeed};
use crate::device::{
    CmosCfg, DeviceConfig, DeviceId, DeviceKind, DmaCfg, DmaSpec, FdcCfg, I8042Cfg, MdaTextCfg,
    PicCfg, PitCfg, PortWindow, PostPortCfg, UartCfg, VgaTextCfg,
};
use crate::drive::{DriveConfig, DriveKind, FloppyGeometry, WriteMode};
use crate::image::{ImageKind, ImageSource, LoadImage};
use crate::machine::MachineConfig;
use crate::memory::{MemoryConfig, MmioWindow, RamRegion};

use chipset::{
    COM1_BASE, COM1_TOP, PIC_COMMAND, PIC_DATA, PIT_BASE, PIT_TOP, SLAVE_PIC_COMMAND,
    SLAVE_PIC_DATA,
};
use devices::mda;
use devices::vga_text::{
    CGA_MODE_CONTROL_PORT, CGA_STATUS_PORT, CRTC_DATA_PORT, CRTC_INDEX_PORT, TEXT_BUFFER_BASE,
    TEXT_BUFFER_LEN,
};

// ---------------------------------------------------------------------------
// Shared address-space and port constants (the layout the legacy runners baked
// in). They live here now: a preset owns its machine's memory map.
// ---------------------------------------------------------------------------

/// Flat guest RAM for the small real-mode demos: 64 KiB.
const DEMO_RAM_BYTES: u64 = 0x1_0000;
/// Classic boot load segment (CS = 0) the demo blobs run from.
const DEMO_CS: u16 = 0x0000;
/// Classic boot IP (linear load address, CS = 0): `0x7C00`.
const DEMO_IP: u16 = 0x7C00;
/// Load IP of the floppy-boot loader (CS = 0): `0x7E00`.
const FLOPPY_LOADER_IP: u16 = 0x7E00;
/// Segment the VGA text demo points DS/ES at (`0xB800 << 4 == 0xB8000`).
const VGA_TEXT_SEGMENT: u16 = 0xB800;

/// 8042 keyboard-controller data port (`0x60`).
const KBD_DATA: u16 = 0x60;
/// 8042 keyboard-controller command/status port (`0x64`).
const KBD_CMD: u16 = 0x64;
/// MC146818 CMOS/RTC index port (`0x70`).
const CMOS_INDEX_PORT: u16 = 0x70;
/// MC146818 CMOS/RTC data port (`0x71`).
const CMOS_DATA_PORT: u16 = 0x71;

/// Full 1 MiB guest space for the test386 and BIOS machines.
const MIB: u64 = 0x10_0000;
/// Segment the test386 diagnostic image is loaded at (`0xF0000`).
const TEST386_LOAD_SEG: u16 = 0xF000;
/// Reset IP the test386 image starts at (`F000:FFF0`).
const RESET_IP: u16 = 0xFFF0;
/// Reset CS the BIOS/test386 machines start at (`0xF000`).
const RESET_CS: u16 = 0xF000;
/// Base of the masked 32 KiB BIOS ROM (`0xF8000`).
const BIOS_ROM_BASE: u64 = 0x0F_8000;

// ---------------------------------------------------------------------------
// Per-chip device builders. Each returns the DeviceConfig for one modelled chip
// with its decoded ports; the caller adds the IRQ / DMA wiring that a given
// machine needs.
// ---------------------------------------------------------------------------

/// The 16550 UART at COM1.
fn uart() -> DeviceConfig {
    DeviceConfig::new(
        DeviceId::Uart,
        DeviceKind::Uart(UartCfg {
            base: COM1_BASE,
            mouse: false,
        }),
        vec![PortWindow::new(COM1_BASE, COM1_TOP)],
    )
}

/// The master 8259 PIC (cascade half of the pair).
fn master_pic() -> DeviceConfig {
    DeviceConfig::new(
        DeviceId::MasterPic,
        DeviceKind::Pic(PicCfg { cascade: true }),
        vec![PortWindow::new(PIC_COMMAND, PIC_DATA)],
    )
}

/// The slave 8259 PIC (carries IRQ8–IRQ15, cascaded into master IRQ2).
fn slave_pic() -> DeviceConfig {
    DeviceConfig::new(
        DeviceId::SlavePic,
        DeviceKind::Pic(PicCfg { cascade: false }),
        vec![PortWindow::new(SLAVE_PIC_COMMAND, SLAVE_PIC_DATA)],
    )
}

/// The 8253 PIT; `irq` wires its counter-0 OUT to a PIC input when set.
fn pit(irq: Option<u8>) -> DeviceConfig {
    let dc = DeviceConfig::new(
        DeviceId::Pit,
        DeviceKind::Pit(PitCfg {}),
        vec![PortWindow::new(PIT_BASE, PIT_TOP)],
    );
    match irq {
        Some(line) => dc.with_irq(line),
        None => dc,
    }
}

/// The 8042 keyboard controller (two non-adjacent ports, `0x60` and `0x64`);
/// `irq` wires its keyboard-byte-ready line to a PIC input when set.
fn keyboard(irq: Option<u8>) -> DeviceConfig {
    let dc = DeviceConfig::new(
        DeviceId::Keyboard,
        DeviceKind::Keyboard(I8042Cfg::default()),
        vec![PortWindow::single(KBD_DATA), PortWindow::single(KBD_CMD)],
    );
    match irq {
        Some(line) => dc.with_irq(line),
        None => dc,
    }
}

/// The MC146818 CMOS/RTC. `seed` writes the AT configuration NVRAM (a stock BIOS
/// checksums it); `timing` overrides the virtual-clock tick ratio; `irq` wires
/// the update-ended output to a PIC input when set.
fn rtc(seed: bool, timing: Option<(u64, u64)>, irq: Option<u8>) -> DeviceConfig {
    let dc = DeviceConfig::new(
        DeviceId::Rtc,
        DeviceKind::Rtc(CmosCfg {
            nvram: if seed {
                crate::NvramSource::Derived
            } else {
                crate::NvramSource::Blank
            },
            timing,
            ..CmosCfg::default()
        }),
        vec![PortWindow::new(CMOS_INDEX_PORT, CMOS_DATA_PORT)],
    );
    match irq {
        Some(line) => dc.with_irq(line),
        None => dc,
    }
}

/// The VGA text card's colour CRTC and CGA compatibility registers.
fn vga() -> DeviceConfig {
    DeviceConfig::new(
        DeviceId::Vga,
        DeviceKind::Vga(VgaTextCfg {}),
        vec![
            PortWindow::new(CRTC_INDEX_PORT, CRTC_DATA_PORT),
            PortWindow::new(CGA_MODE_CONTROL_PORT, CGA_STATUS_PORT),
        ],
    )
}

/// IBM PC Technical Reference, Monochrome Display and Printer Adapter,
/// Programming Considerations: the display decode excludes the printer ports.
fn mda() -> DeviceConfig {
    DeviceConfig::new(
        DeviceId::Mda,
        DeviceKind::Mda(MdaTextCfg {}),
        vec![
            PortWindow::new(mda::DISPLAY_PORT_BASE, mda::DISPLAY_PORT_TOP),
            PortWindow::single(mda::UNASSIGNED_PORT),
        ],
    )
}

/// A POST checkpoint port decoding `port`.
fn post(port: u16) -> DeviceConfig {
    DeviceConfig::new(
        DeviceId::Post,
        DeviceKind::Post(PostPortCfg { port }),
        vec![PortWindow::single(port)],
    )
}

/// The 8237 DMA controller decoding `ports` (its channel registers plus the page
/// latches — the exact latch range differs between machines that overlap the
/// POST port and those that do not).
fn dma(ports: Vec<PortWindow>) -> DeviceConfig {
    DeviceConfig::new(DeviceId::Dma, DeviceKind::Dma(DmaCfg {}), ports)
}

/// A µPD765 floppy controller offering two drive slots, decoding `ports`, with
/// optional IRQ and DMA wiring.
fn fdc(ports: Vec<PortWindow>, irq: Option<u8>, dma_channel: Option<u8>) -> DeviceConfig {
    let mut dc = DeviceConfig::new(
        DeviceId::Fdc,
        DeviceKind::Fdc(FdcCfg { drive_slots: 2 }),
        ports,
    );
    if let Some(line) = irq {
        dc = dc.with_irq(line);
    }
    if let Some(channel) = dma_channel {
        dc = dc.with_dma(DmaSpec {
            controller: DeviceId::Dma,
            channel,
        });
    }
    dc
}

/// A floppy of the given `geometry` in drive slot `slot`, backed by `image`.
///
/// The one place a preset attaches a floppy: geometry and slot are parameters, so
/// a machine with a 1.2 MB drive A: and a 720 KB drive B: is two calls, and the
/// CMOS floppy nibbles + equipment byte follow automatically from
/// [`derive_cmos_config`](crate::assemble::derive_cmos_config) — no hand-edited
/// CMOS. [`floppy_drive`] is the common single-drive, high-density default.
fn floppy_drive_geo(image: ImageSource, geometry: FloppyGeometry, slot: u8) -> DriveConfig {
    DriveConfig {
        media: None,
        kind: DriveKind::Floppy { geometry },
        image,
        controller: DeviceId::Fdc,
        unit: slot,
        write: WriteMode::Volatile,
        expected_sha256: None,
    }
}

/// A single 1.44 MB floppy in drive slot 0 backed by `image` — the default the
/// BIOS and floppy-boot machines take.
fn floppy_drive(image: ImageSource) -> DriveConfig {
    floppy_drive_geo(image, FloppyGeometry::HD_1_44, 0)
}

// ---------------------------------------------------------------------------
// Presets: one function per machine shape.
// ---------------------------------------------------------------------------

/// The PC/AT device set shared by the real-mode demos: UART, cascaded 8259 pair,
/// PIT, 8042, VGA text card, and CMOS/RTC. Each argument wires the one interrupt
/// source a given demo takes (the rest stay unwired, exactly as the demo's
/// hand-assembly left them); `rtc_timing` fast-ticks the RTC for the demo that
/// must see an update-ended interrupt within a bounded run.
fn at_device_set(
    pit_irq: Option<u8>,
    kbd_irq: Option<u8>,
    rtc_irq: Option<u8>,
    rtc_timing: Option<(u64, u64)>,
) -> Vec<DeviceConfig> {
    vec![
        uart(),
        master_pic(),
        pit(pit_irq),
        keyboard(kbd_irq),
        vga(),
        slave_pic(),
        rtc(false, rtc_timing, rtc_irq),
    ]
}

/// A real-mode demo machine: `blob` loaded at `0x7C00` in flat 64 KiB RAM, run
/// from CS:IP `0000:7C00`, on the AT device set `devices`. Used by the boot,
/// keyboard, and RTC demos, which differ only in wiring and segment seed.
fn at_demo(blob: Vec<u8>, devices: Vec<DeviceConfig>) -> MachineConfig {
    MachineConfig {
        system_bios_image: None,
        board: BoardModel::Bare,
        display_switch: crate::DisplaySwitch::Auto,
        cpu: CpuConfig {
            engine: crate::CpuEngine::Interpreter,
            fpu: crate::FpuChip::None,
            chip: CpuChip::I80286,
            reset: ResetSpec::Entry {
                cs: DEMO_CS,
                ip: DEMO_IP,
            },
            segments: SegmentSeed::flat(DEMO_CS),
        },
        clock: ClockConfig::default(),
        memory: MemoryConfig {
            address_space_bytes: DEMO_RAM_BYTES,
            ram: vec![RamRegion::new(0, DEMO_RAM_BYTES)],
            mmio: Vec::new(),
        },
        devices,
        drives: Vec::new(),
        images: vec![LoadImage {
            base: (u64::from(DEMO_CS) << 4) + u64::from(DEMO_IP),
            source: ImageSource::Inline(blob),
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        }],
    }
}

/// The real-mode boot demo: the PIT counter-0 OUT wired to IRQ0, no other source.
pub fn at_boot(blob: Vec<u8>) -> MachineConfig {
    at_demo(blob, at_device_set(Some(0), None, None, None))
}

/// The real-mode keyboard demo: the 8042 keyboard-byte-ready line wired to IRQ1.
///
/// The scancode script is queued into the controller by the frontend after
/// assembly (through the device-index seam), matching the demo's power-on key
/// seeding.
pub fn at_keyboard(blob: Vec<u8>) -> MachineConfig {
    at_demo(blob, at_device_set(None, Some(1), None, None))
}

/// The real-mode RTC demo: the RTC update-ended line wired to IRQ8 (on the slave
/// PIC, cascaded into the master), with the RTC fast-ticked so the update-ended
/// interrupt fires within a bounded run.
pub fn at_rtc(blob: Vec<u8>, second_ticks: u64, uip_ticks: u64) -> MachineConfig {
    at_demo(
        blob,
        at_device_set(None, None, Some(8), Some((second_ticks, uip_ticks))),
    )
}

/// The floppy WRITE demo: the AT device set plus an 8237 and a µPD765 backed by a
/// **blank writable** floppy, with `program` at `0x7C00` and `pattern` preloaded
/// at `pattern_base` in flat 64 KiB RAM.
///
/// The guest drives the controller directly (no BIOS): it programs the 8237 for a
/// memory→device transfer of `pattern`, issues WRITE DATA, then programs the 8237
/// the other way and issues READ DATA to pull the just-written sector back into a
/// separate buffer. The floppy starts blank (all zeroes), so the pattern the guest
/// reads back can only have come from its own write. No interrupt source is wired;
/// the guest polls the controller through the result phase and the machine
/// services the DMA on its run boundaries.
pub fn at_fdc_write(program: Vec<u8>, pattern: Vec<u8>, pattern_base: u64) -> MachineConfig {
    let mut devices = at_device_set(None, None, None, None);
    devices.push(dma(vec![
        PortWindow::new(0x00, 0x0F),
        PortWindow::new(0x80, 0x8F),
    ]));
    devices.push(fdc(FdcCfg::port_windows(false), None, Some(2)));

    let blank = ImageSource::Blank {
        size: FloppyGeometry::DD_360K.image_bytes(),
    };
    MachineConfig {
        system_bios_image: None,
        board: BoardModel::Bare,
        display_switch: crate::DisplaySwitch::Auto,
        cpu: CpuConfig {
            engine: crate::CpuEngine::Interpreter,
            fpu: crate::FpuChip::None,
            chip: CpuChip::I80286,
            reset: ResetSpec::Entry {
                cs: DEMO_CS,
                ip: DEMO_IP,
            },
            segments: SegmentSeed::flat(DEMO_CS),
        },
        clock: ClockConfig::default(),
        memory: MemoryConfig {
            address_space_bytes: DEMO_RAM_BYTES,
            ram: vec![RamRegion::new(0, DEMO_RAM_BYTES)],
            mmio: Vec::new(),
        },
        devices,
        drives: vec![floppy_drive_geo(blank, FloppyGeometry::DD_360K, 0)],
        images: vec![
            LoadImage {
                base: (u64::from(DEMO_CS) << 4) + u64::from(DEMO_IP),
                source: ImageSource::Inline(program),
                kind: ImageKind::RamLoad,
                provides: Vec::new(),
            },
            LoadImage {
                base: pattern_base,
                source: ImageSource::Inline(pattern),
                kind: ImageKind::RamLoad,
                provides: Vec::new(),
            },
        ],
    }
}

/// The VGA text demo: `blob` runs with DS/ES pointed at the colour text page, the
/// `0xB8000` buffer trapped as MMIO to the card, and no interrupt source.
pub fn at_vga_text(blob: Vec<u8>) -> MachineConfig {
    let space = TEXT_BUFFER_BASE + TEXT_BUFFER_LEN as u64;
    MachineConfig {
        system_bios_image: None,
        board: BoardModel::Bare,
        display_switch: crate::DisplaySwitch::Auto,
        cpu: CpuConfig {
            engine: crate::CpuEngine::Interpreter,
            fpu: crate::FpuChip::None,
            chip: CpuChip::I80286,
            reset: ResetSpec::Entry {
                cs: DEMO_CS,
                ip: DEMO_IP,
            },
            segments: SegmentSeed {
                ds: VGA_TEXT_SEGMENT,
                es: VGA_TEXT_SEGMENT,
                ss: DEMO_CS,
                sp: SegmentSeed::flat(DEMO_CS).sp,
            },
        },
        clock: ClockConfig::default(),
        memory: MemoryConfig {
            address_space_bytes: space,
            ram: vec![RamRegion::new(0, TEXT_BUFFER_BASE)],
            mmio: vec![MmioWindow::new(
                TEXT_BUFFER_BASE,
                TEXT_BUFFER_LEN as u64,
                DeviceId::Vga,
            )],
        },
        devices: at_device_set(None, None, None, None),
        drives: Vec::new(),
        images: vec![LoadImage {
            base: (u64::from(DEMO_CS) << 4) + u64::from(DEMO_IP),
            source: ImageSource::Inline(blob),
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        }],
    }
}

/// The test386 diagnostic machine: the whole 1 MiB is RAM, the 64 KiB `image` is
/// loaded at `0xF0000`, and execution starts at the reset vector `F000:FFF0`.
///
/// Just two devices — the POST checkpoint port at `0x80` and a COM1 UART — and no
/// interrupt controller: the diagnostic runs `cli`, so any wired IRQ could never
/// be delivered and would keep the run from terminating.
pub fn test386(image: Vec<u8>) -> MachineConfig {
    MachineConfig {
        system_bios_image: None,
        board: BoardModel::Bare,
        display_switch: crate::DisplaySwitch::Auto,
        cpu: CpuConfig {
            engine: crate::CpuEngine::Interpreter,
            fpu: crate::FpuChip::None,
            chip: CpuChip::I80286,
            reset: ResetSpec::Entry {
                cs: RESET_CS,
                ip: RESET_IP,
            },
            segments: SegmentSeed::flat(TEST386_LOAD_SEG),
        },
        clock: ClockConfig {
            retired_per_pit_clock: machine::RETIRED_PER_PIT_CLOCK,
            retired_per_pit_clock_denominator: 1,
            turbo: None,
        },
        memory: MemoryConfig {
            address_space_bytes: MIB,
            ram: vec![RamRegion::new(0, MIB)],
            mmio: Vec::new(),
        },
        devices: vec![post(0x80), uart()],
        drives: Vec::new(),
        images: vec![LoadImage {
            base: u64::from(TEST386_LOAD_SEG) << 4,
            source: ImageSource::Inline(image),
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        }],
    }
}

/// The fast floppy-boot machine: a COM1 UART, an 8237 DMA controller, and a µPD765
/// floppy controller backed by `floppy`, with `loader` loaded at `0x7E00` in flat
/// 64 KiB RAM. The loader drives the controller to read the boot sector over DMA
/// and jumps to it; there is no BIOS and no interrupt controller (the DMA transfer
/// is synchronous, so the loader never waits on IRQ6).
pub fn floppy_boot(loader: Vec<u8>, floppy: Vec<u8>) -> MachineConfig {
    let devices = vec![
        uart(),
        // The DMA page latches here run from 0x80 (no POST port competes for it).
        dma(vec![
            PortWindow::new(0x00, 0x0F),
            PortWindow::new(0x80, 0x8F),
        ]),
        fdc(FdcCfg::port_windows(false), None, Some(2)),
    ];
    MachineConfig {
        system_bios_image: None,
        board: BoardModel::Bare,
        display_switch: crate::DisplaySwitch::Auto,
        cpu: CpuConfig {
            engine: crate::CpuEngine::Interpreter,
            fpu: crate::FpuChip::None,
            chip: CpuChip::I80286,
            reset: ResetSpec::Entry {
                cs: DEMO_CS,
                ip: FLOPPY_LOADER_IP,
            },
            segments: SegmentSeed::flat(DEMO_CS),
        },
        clock: ClockConfig::default(),
        memory: MemoryConfig {
            address_space_bytes: DEMO_RAM_BYTES,
            ram: vec![RamRegion::new(0, DEMO_RAM_BYTES)],
            mmio: Vec::new(),
        },
        devices,
        drives: vec![floppy_drive(ImageSource::Inline(floppy))],
        images: vec![LoadImage {
            base: (u64::from(DEMO_CS) << 4) + u64::from(FLOPPY_LOADER_IP),
            source: ImageSource::Inline(loader),
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        }],
    }
}

/// The full PC/AT BIOS machine: the 1 MiB memory mosaic with the 32 KiB `rom`
/// masked at `0xF8000`, the cascaded 8259 pair, PIT, 8042, VGA and MDA text cards,
/// CMOS/RTC (seeded), the port-`0x61` latch, and a POST port at `0x80`, booting
/// from the reset vector. CPU work uses exact steps and interrupts are accepted
/// only at completed instruction boundaries.
///
/// For either pinned Kiselev Xi8088 image, this BIOS reads the AT display
/// switch with inverted polarity; in the colour position it selects the
/// monochrome adapter. The preset selects Colour, and derived CMOS video bits
/// still describe the switch-selected colour primary (10b). Other firmware
/// keeps Auto. IBM Personal Computer AT Technical Reference, System Board,
/// Keyboard Controller, "Input-Port Bit Definitions", defines the AT polarity;
/// see `backlog/docs/design/machine-config.md` §Primary display.
///
/// `floppy` chooses the disk story:
///
/// - `Some(image)` attaches a real µPD765 floppy controller (ports `0x3F0..=0x3F7`,
///   IRQ6) driven by an 8237 (`0x00..=0x0F`, page latches `0x81..=0x8F`) backed by
///   `image` — the path INT 13h / INT 19h use to load a boot sector.
/// - `None` models the pre-floppy-controller spike: the BIOS's floppy-probe ports
///   stay undecoded (open-bus `0xFF`), so INT 19h spins in its retry loop. The
///   machine still declares a 1.44 MB drive in CMOS (a real AT always does, and the
///   original spike seeded the same CMOS), so a floppy drive is present for the
///   CMOS derivation but the controller decodes no ports and takes no interrupt.
///   This partial motherboard is a Bare rig, not a complete Ibm5170 board.
pub fn bios(rom: Vec<u8>, floppy: Option<Vec<u8>>) -> MachineConfig {
    let board = if floppy.is_some() {
        BoardModel::Ibm5170
    } else {
        BoardModel::Bare
    };
    let mut devices = BoardModel::Ibm5170.motherboard_devices();
    if floppy.is_none() {
        devices.retain(|d| {
            !matches!(
                d.id,
                DeviceId::Dma | DeviceId::DmaWord | DeviceId::Coprocessor
            )
        });
    }
    devices.extend([mda(), vga(), uart()]);

    let drives = match &floppy {
        Some(_) => {
            // A decoded floppy controller uses the board's 8237 for the boot path.
            devices.push(fdc(FdcCfg::port_windows(false), Some(6), Some(2)));
            vec![floppy_drive(ImageSource::Inline(floppy.unwrap()))]
        }
        None => {
            // No controller decoded (INT 19h reads open-bus and spins), but the AT
            // still declares its drive A: in CMOS. The FDC entry decodes nothing and
            // is unwired; it exists only so the drive binds and the CMOS derivation
            // sees a 1.44 MB floppy, exactly as the original no-floppy spike did.
            devices.push(fdc(Vec::new(), None, None));
            vec![floppy_drive(ImageSource::Blank {
                size: FloppyGeometry::HD_1_44.image_bytes(),
            })]
        }
    };

    let display_switch = crate::bios_display_switch(&rom);
    let image = LoadImage {
        base: BIOS_ROM_BASE,
        source: ImageSource::Inline(rom),
        kind: ImageKind::Rom,
        provides: vec![crate::FirmwareService::Floppy],
    };

    MachineConfig {
        system_bios_image: Some(0),
        board,
        display_switch,
        cpu: CpuConfig {
            engine: crate::CpuEngine::Interpreter,
            fpu: crate::FpuChip::None,
            chip: CpuChip::I80286,
            reset: ResetSpec::ResetVector {
                cs: RESET_CS,
                ip: RESET_IP,
            },
            segments: SegmentSeed::flat(RESET_CS),
        },
        clock: board.default_clock(),
        memory: MemoryConfig {
            address_space_bytes: MIB,
            ram: {
                let mut ram = vec![RamRegion::new(0, mda::TEXT_BUFFER_BASE)];
                if board == BoardModel::Bare {
                    // The historical partial POST rig retains its original map.
                    ram.push(RamRegion::new(0xC0000, BIOS_ROM_BASE - 0xC0000));
                }
                ram
            },
            mmio: vec![
                MmioWindow::new(
                    mda::TEXT_BUFFER_BASE,
                    mda::TEXT_BUFFER_LEN as u64,
                    DeviceId::Mda,
                ),
                MmioWindow::new(TEXT_BUFFER_BASE, TEXT_BUFFER_LEN as u64, DeviceId::Vga),
            ],
        },
        devices,
        drives,
        images: vec![image],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ibm_at_preset_uses_the_calibrated_rate_and_bare_rigs_keep_their_rate() {
        let at = bios(vec![0; 0x8000], Some(vec![0; 1_474_560]));
        assert_eq!(
            (
                at.clock.retired_per_pit_clock,
                at.clock.retired_per_pit_clock_denominator
            ),
            (8, 13)
        );
        let bare = bios(vec![0; 0x8000], None);
        assert_eq!(
            (
                bare.clock.retired_per_pit_clock,
                bare.clock.retired_per_pit_clock_denominator
            ),
            (1, 1)
        );
    }

    #[test]
    fn bios_board_identity_distinguishes_the_no_media_rig() {
        for floppy in [None, Some(vec![0; 1_474_560])] {
            assert_eq!(bios(vec![0; 32768], floppy).system_bios_image, Some(0));
        }
        assert_eq!(bios(vec![0; 0x8000], None).board, BoardModel::Bare);
        assert_eq!(
            bios(
                vec![0; 0x8000],
                Some(vec![0; FloppyGeometry::HD_1_44.image_bytes() as usize])
            )
            .board,
            BoardModel::Ibm5170
        );
    }

    #[test]
    fn bios_devices_follow_board_then_card_order() {
        for has_floppy in [false, true] {
            let floppy =
                has_floppy.then(|| vec![0; FloppyGeometry::HD_1_44.image_bytes() as usize]);
            let config = bios(vec![0; 0x8000], floppy);
            let expected: Vec<_> = [
                DeviceId::MasterPic,
                DeviceId::SlavePic,
                DeviceId::Pit,
                DeviceId::Dma,
                DeviceId::DmaWord,
                DeviceId::Keyboard,
                DeviceId::Rtc,
                DeviceId::Coprocessor,
                DeviceId::SysControl,
                DeviceId::Post,
                DeviceId::Mda,
                DeviceId::Vga,
                DeviceId::Uart,
                DeviceId::Fdc,
            ]
            .into_iter()
            .filter(|id| {
                has_floppy
                    || !matches!(
                        id,
                        DeviceId::Dma | DeviceId::DmaWord | DeviceId::Coprocessor
                    )
            })
            .collect();
            assert_eq!(
                config.devices.iter().map(|d| d.id).collect::<Vec<_>>(),
                expected,
                "floppy attached: {has_floppy}"
            );
        }
    }

    #[test]
    fn bios_mmio_windows_follow_base_order() {
        for floppy in [
            None,
            Some(vec![0; FloppyGeometry::HD_1_44.image_bytes() as usize]),
        ] {
            let config = bios(vec![0; 0x8000], floppy);
            assert_eq!(
                config.memory.mmio,
                vec![
                    MmioWindow::new(0xB0000, 0x8000, DeviceId::Mda),
                    MmioWindow::new(0xB8000, 0x8000, DeviceId::Vga),
                ]
            );
        }
    }

    #[test]
    fn bios_mda_decode_excludes_ram_vga_and_printer_ports() {
        for floppy in [
            None,
            Some(vec![0; FloppyGeometry::HD_1_44.image_bytes() as usize]),
        ] {
            let config = bios(vec![0; 0x8000], floppy);
            let window = config
                .memory
                .mmio
                .iter()
                .find(|w| w.base == 0xB0000)
                .expect("the monochrome aperture must be card-owned MMIO");
            assert_eq!(window.size, 0x8000);
            assert_eq!(window.device, DeviceId::Mda);
            let card = config
                .devices
                .iter()
                .find(|d| d.id == window.device)
                .unwrap();
            assert_eq!(card.kind, DeviceKind::Mda(MdaTextCfg {}));
            assert_eq!(
                card.ports,
                vec![PortWindow::new(0x3B0, 0x3BB), PortWindow::single(0x3BF)]
            );
            for port in 0x3B0..=0x3BF {
                let owners: Vec<_> = config
                    .devices
                    .iter()
                    .filter(|d| d.ports.iter().any(|w| (w.start..=w.end).contains(&port)))
                    .collect();
                if (0x3BC..=0x3BE).contains(&port) {
                    assert!(owners.is_empty(), "printer port {port:#x}");
                } else {
                    assert_eq!(owners, vec![card], "display port {port:#x}");
                }
            }
            assert!(config
                .memory
                .ram
                .iter()
                .all(|r| r.end() <= 0xB0000 || r.base >= 0xB8000));
            assert_eq!(config.validate(), Ok(()));
        }
    }

    #[test]
    fn floppy_drive_default_is_high_density_slot_zero() {
        // The BIOS and floppy-boot machines take this default; it must stay a
        // single 1.44 MB drive A: so their goldens are untouched.
        let drive = floppy_drive(ImageSource::Blank {
            size: FloppyGeometry::HD_1_44.image_bytes(),
        });
        assert_eq!(
            drive.kind,
            DriveKind::Floppy {
                geometry: FloppyGeometry::HD_1_44
            }
        );
        assert_eq!(drive.unit, 0);
    }

    #[test]
    fn floppy_drive_geo_honours_its_geometry_and_slot() {
        // A different drive type in a different slot — the capability the maintainer
        // wants: attach, say, a 720 KB drive B: with no CMOS hand-editing.
        let drive = floppy_drive_geo(
            ImageSource::Blank {
                size: FloppyGeometry::DD_720K.image_bytes(),
            },
            FloppyGeometry::DD_720K,
            1,
        );
        assert_eq!(
            drive.kind,
            DriveKind::Floppy {
                geometry: FloppyGeometry::DD_720K
            }
        );
        assert_eq!(drive.unit, 1);
    }
    #[test]
    fn every_preset_accepts_valid_images() {
        let floppy = vec![0; FloppyGeometry::HD_1_44.image_bytes() as usize];
        for config in [
            at_boot(vec![0; 512]),
            at_keyboard(vec![0; 512]),
            at_rtc(vec![0; 512], 100, 10),
            at_vga_text(vec![0; 512]),
            at_fdc_write(vec![0; 512], vec![0; 512], 0x8000),
            test386(vec![0; 0x1_0000]),
            floppy_boot(vec![0; 512], floppy.clone()),
            bios(vec![0; 0x8000], None),
            bios(vec![0; 0x8000], Some(floppy)),
        ] {
            assert_eq!(config.validate(), Ok(()));
            assert!(crate::assemble_with_options(
                &config,
                crate::AssemblyOptions {
                    allow_unlisted_firmware: true
                }
            )
            .is_ok());
        }
    }
}
