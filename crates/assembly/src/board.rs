// SPDX-License-Identifier: AGPL-3.0-or-later
//! Motherboard identity and standard wiring, as data.
//!
//! IBM Personal Computer AT Technical Reference, Section 1 "System Board":
//! "System Interrupts", "System Timers", "Direct Memory Access", "I/O Address
//! Map", "Keyboard Controller", and "Real-Time Clock/CMOS RAM Information".
//! Intel 80386 Programmer's Reference Manual, §2.1 and §10.1 define the DX
//! physical bus and reset address. QEMU Firmware Configuration Device
//! specification, Selector and Data Registers defines the emulator-only port.
//! See `backlog/docs/design/machine-config.md` §Board model for the coverage table.

use crate::device::{
    CmosCfg, DeviceConfig, DeviceId, DeviceKind, DmaCfg, I8042Cfg, PicCfg, PitCfg, PortWindow,
    PostPortCfg, SysControlCfg,
};

/// Hardware family derived from a motherboard, never selected independently.
/// See `backlog/docs/design/machine-config.md` §Board model.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MachineClass {
    /// AT interrupt, keyboard and CMOS wiring.
    At,
    /// ISA board with a 32-bit 80386DX bus.
    At386,
    /// ISA board with an Intel486 DX and integrated FPU.
    At486,
}

/// Named motherboard constraints on the flat device list.
///
/// IBM Personal Computer AT Technical Reference, Section 1 "System Board"
/// defines the AT wiring. The generic AT uses the existing 8253 timer model
/// and both 8237 banks, an empty 80287 socket and the keyboard CPU-reset line. See
/// `backlog/docs/design/machine-config.md` §Board model.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BoardModel {
    /// AT with the documented IBM 5170 device coverage.
    Ibm5170,
    /// Emulator ISA board with writable firmware shadow RAM.
    Generic386Isa,
    /// Emulator ISA board with an Intel486 DX and writable firmware shadow RAM.
    Generic486Isa,
    /// Hand-wired Rust test rigs, without motherboard constraints or a class.
    Bare,
}

impl BoardModel {
    /// Stable identifier used by firmware compatibility data and diagnostics.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Ibm5170 => "ibm-5170",
            Self::Generic386Isa => "generic-386-isa",
            Self::Generic486Isa => "generic-486-isa",
            Self::Bare => "bare",
        }
    }

    /// Instruction-throughput calibration for the board's virtual PIT clock.
    /// The 6 MHz AT model is calibrated to 8/13 steps per PIT clock against
    /// IBM Personal Computer AT Technical Reference, BIOS TEST1 checkpoint 11
    /// (speed/refresh clocks); this is not a hardware instruction-cycle table.
    /// See machine-config.md §CpuConfig and ClockConfig for the measured window.
    pub fn default_clock(self) -> crate::ClockConfig {
        match self {
            Self::Ibm5170 => crate::ClockConfig {
                retired_per_pit_clock: 8,
                retired_per_pit_clock_denominator: 13,
            },
            Self::Bare | Self::Generic386Isa | Self::Generic486Isa => crate::ClockConfig::default(),
        }
    }

    /// Nominal CPU clocks of the board's variants in MHz, with the clock its
    /// default rate is calibrated at. IBM Personal Computer AT Technical
    /// Reference, System Board: type 1 and 2 boards run the 80286 at 6 MHz,
    /// the type 3 board (models 319 and 339) at 8 MHz. Other clocks scale the
    /// calibrated rate by their ratio; none is a cycle-exact clock.
    pub const fn cpu_clocks_mhz(self) -> (u32, &'static [u32]) {
        match self {
            Self::Ibm5170 => (6, &[6, 8]),
            Self::Bare | Self::Generic386Isa | Self::Generic486Isa => (0, &[]),
        }
    }

    /// Reviewed firmware builds owned by this motherboard.
    pub const fn firmware_catalogue(self) -> &'static [crate::FirmwareEntry] {
        match self {
            Self::Ibm5170 => &[
                crate::KISELEV_XI8088,
                crate::firmware::KISELEV_XI8088_ATRESET,
                crate::firmware::UNEMU_AT_BIOS_1_0_2_UNEMU_3,
                crate::firmware::UNEMU_AT_BIOS_1_0_2_UNEMU_2,
                crate::firmware::IBM_5170_1984_01_10,
                crate::firmware::IBM_5170_1985_06_10,
                crate::firmware::IBM_5170_1985_11_15,
                crate::firmware::LANDMARK_ROM_POST_AT_1_2_0,
                crate::FirmwareEntry::IBM_VGA_1987,
                crate::FirmwareEntry::VGABIOS_0_9D_UNEMU_1_STD,
                crate::FirmwareEntry::VGABIOS_0_8A_UNEMU_1_STD,
                crate::FirmwareEntry::XUB_R638_AT,
                crate::FirmwareEntry::XUB_R638_XTP_ATA1F0,
            ],
            Self::Generic386Isa | Self::Generic486Isa => &[crate::firmware::SEABIOS_1_17_0_ISA],
            Self::Bare => &[],
        }
    }

    /// The hardware class implied by this model; a bare rig has no class.
    pub const fn class(self) -> Option<MachineClass> {
        match self {
            Self::Ibm5170 => Some(MachineClass::At),
            Self::Generic386Isa => Some(MachineClass::At386),
            Self::Generic486Isa => Some(MachineClass::At486),
            Self::Bare => None,
        }
    }

    /// Default motherboard roles in bus order, with standard ports and wiring.
    ///
    /// IBM Personal Computer AT Technical Reference, Section 1 "System Board",
    /// I/O Address Map and System Interrupts: slave PIC cascades on IRQ2, PIT
    /// drives IRQ0, keyboard IRQ1, RTC IRQ8 and coprocessor IRQ13. DMA channel 4
    /// is reserved for cascade; motherboard devices do not request transfers.
    /// POST defaults to 80h, so DMA page decoding starts at 81h. Validation also
    /// accepts a relocated/omitted POST latch and the corresponding 80h page.
    pub fn motherboard_devices(self) -> Vec<DeviceConfig> {
        if self == Self::Bare {
            return Vec::new();
        }
        use DeviceId::*;
        let mut devices = vec![
            DeviceConfig::new(
                MasterPic,
                DeviceKind::Pic(PicCfg { cascade: true }),
                vec![PortWindow::new(0x20, 0x21)],
            ),
            DeviceConfig::new(
                SlavePic,
                DeviceKind::Pic(PicCfg { cascade: false }),
                vec![PortWindow::new(0xA0, 0xA1)],
            ),
            DeviceConfig::new(
                Pit,
                DeviceKind::Pit(PitCfg {}),
                vec![PortWindow::new(0x40, 0x43)],
            )
            .with_irq(0),
            DeviceConfig::new(
                Dma,
                DeviceKind::Dma(DmaCfg {}),
                vec![PortWindow::new(0x00, 0x0F), PortWindow::new(0x81, 0x88)],
            ),
            DeviceConfig::new(
                DmaWord,
                DeviceKind::DmaWord(DmaCfg {}),
                vec![PortWindow::new(0xC0, 0xDF), PortWindow::new(0x89, 0x8F)],
            ),
            DeviceConfig::new(
                Keyboard,
                DeviceKind::Keyboard(I8042Cfg::default()),
                vec![PortWindow::single(0x60), PortWindow::single(0x64)],
            )
            .with_irq(1),
            DeviceConfig::new(
                Rtc,
                DeviceKind::Rtc(CmosCfg {
                    ram: crate::CmosRam::Bytes64,
                    nvram: crate::NvramSource::Derived,
                    start: crate::RtcStart::default(),
                    timing: None,
                }),
                vec![PortWindow::new(0x70, 0x71)],
            )
            .with_irq(8),
            DeviceConfig::new(
                Coprocessor,
                DeviceKind::AbsentCoprocessor,
                vec![PortWindow::new(0xF0, 0xF1), PortWindow::new(0xF8, 0xFF)],
            )
            .with_irq(13),
            DeviceConfig::new(
                SysControl,
                DeviceKind::SysControl(SysControlCfg {}),
                vec![PortWindow::single(0x61)],
            ),
            DeviceConfig::new(
                Post,
                DeviceKind::Post(PostPortCfg { port: 0x80 }),
                vec![PortWindow::single(0x80)],
            ),
        ];
        if self.is_generic_isa() {
            if let DeviceKind::Rtc(cfg) = &mut devices[6].kind {
                cfg.ram = crate::CmosRam::Bytes128;
            }
            devices.extend([
                DeviceConfig::new(
                    SysCtrlPortA,
                    DeviceKind::SysCtrlPortA,
                    vec![PortWindow::single(0x92)],
                ),
                DeviceConfig::new(
                    DebugConsole,
                    DeviceKind::DebugConsole(crate::DebugConsoleCfg::default()),
                    vec![PortWindow::single(0x402)],
                ),
                DeviceConfig::new(
                    FwCfg,
                    DeviceKind::FwCfg(crate::FwCfgCfg {
                        sercon_from_com1: true,
                        ..Default::default()
                    }),
                    crate::FwCfgCfg::port_windows(),
                ),
            ]);
        }
        devices
    }

    /// Shared 32-bit ISA memory and firmware services. Intel486 DX Programmer's
    /// Reference Manual, §10.1 Initialization; see machine-files.md §Generic 486 ISA board.
    pub const fn is_generic_isa(self) -> bool {
        matches!(self, Self::Generic386Isa | Self::Generic486Isa)
    }

    /// Default processor for Rust drafts; 486 machine files require an explicit variant.
    pub const fn cpu_chip(self) -> crate::CpuChip {
        match self {
            Self::Generic386Isa => crate::CpuChip::I80386,
            Self::Generic486Isa => crate::CpuChip::I80486Dx,
            Self::Ibm5170 | Self::Bare => crate::CpuChip::I80286,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IBM Personal Computer AT Technical Reference, Section 1 "System Board",
    /// I/O Address Map and System Interrupts. Literal values keep the shared
    /// preset/validator table from becoming its own wiring oracle.
    #[test]
    fn ibm5170_wiring_matches_the_technical_reference() {
        use DeviceId::*;
        let devices = BoardModel::Ibm5170.motherboard_devices();
        let actual: Vec<_> = devices
            .iter()
            .map(|d| (d.id, d.kind.clone(), d.ports.clone(), d.irq, d.dma))
            .collect();
        assert_eq!(
            actual,
            vec![
                (
                    MasterPic,
                    DeviceKind::Pic(PicCfg { cascade: true }),
                    vec![PortWindow::new(0x20, 0x21)],
                    None,
                    None
                ),
                (
                    SlavePic,
                    DeviceKind::Pic(PicCfg { cascade: false }),
                    vec![PortWindow::new(0xA0, 0xA1)],
                    None,
                    None
                ),
                (
                    Pit,
                    DeviceKind::Pit(PitCfg {}),
                    vec![PortWindow::new(0x40, 0x43)],
                    Some(0),
                    None
                ),
                (
                    Dma,
                    DeviceKind::Dma(DmaCfg {}),
                    vec![PortWindow::new(0x00, 0x0F), PortWindow::new(0x81, 0x88)],
                    None,
                    None
                ),
                (
                    DmaWord,
                    DeviceKind::DmaWord(DmaCfg {}),
                    vec![PortWindow::new(0xC0, 0xDF), PortWindow::new(0x89, 0x8F)],
                    None,
                    None
                ),
                (
                    Keyboard,
                    DeviceKind::Keyboard(I8042Cfg::default()),
                    vec![PortWindow::single(0x60), PortWindow::single(0x64)],
                    Some(1),
                    None
                ),
                (
                    Rtc,
                    DeviceKind::Rtc(CmosCfg {
                        ram: crate::CmosRam::Bytes64,
                        nvram: crate::NvramSource::Derived,
                        start: crate::RtcStart::default(),
                        timing: None
                    }),
                    vec![PortWindow::new(0x70, 0x71)],
                    Some(8),
                    None
                ),
                (
                    Coprocessor,
                    DeviceKind::AbsentCoprocessor,
                    vec![PortWindow::new(0xF0, 0xF1), PortWindow::new(0xF8, 0xFF)],
                    Some(13),
                    None
                ),
                (
                    SysControl,
                    DeviceKind::SysControl(SysControlCfg {}),
                    vec![PortWindow::single(0x61)],
                    None,
                    None
                ),
                (
                    Post,
                    DeviceKind::Post(PostPortCfg { port: 0x80 }),
                    vec![PortWindow::single(0x80)],
                    None,
                    None
                ),
            ]
        );
        assert!(devices
            .iter()
            .all(|d| d.access_dependents.is_empty() && !d.allow_port_overlap));
    }
}

/// Position of the AT motherboard's primary-display slide switch.
///
/// IBM Personal Computer AT Technical Reference, System Board, Keyboard
/// Controller, "Input-Port Bit Definitions", bit 6. See
/// `backlog/docs/design/machine-config.md` §Primary display.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum DisplaySwitch {
    /// Prefer MDA, then colour; no adapter leaves the switch at colour.
    #[default]
    Auto,
    /// Select an installed monochrome adapter.
    Mono,
    /// Select an installed colour adapter.
    Colour,
}

impl std::fmt::Display for DisplaySwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Mono => "mono",
            Self::Colour => "colour",
        })
    }
}

impl DisplaySwitch {
    pub(crate) const fn required_card_class(self) -> &'static str {
        match self {
            Self::Auto => "display card",
            Self::Mono => "MDA card",
            Self::Colour => "colour card (vga-text / vga)",
        }
    }
}

/// Primary adapter selected by the board switch and installed display cards.
///
/// IBM Personal Computer AT Technical Reference, System Board, Real-Time
/// Clock/CMOS RAM Information, "Equipment Byte (Hex 14)", bits 5–4.
/// See `backlog/docs/design/machine-config.md` §Primary display.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrimaryDisplay {
    /// No installed adapter; retain CMOS video code 00 and the colour switch.
    None,
    /// Monochrome Display and Printer Adapter (CMOS video code 11).
    Monochrome,
    /// ROM-less colour adapter in 80-column mode (CMOS video code 10).
    Colour80,
    /// Colour adapter with a video option ROM (CMOS video code 00).
    OwnBios,
}

impl PrimaryDisplay {
    /// AT input-port bit 6 is set only for a monochrome primary display.
    pub const fn monochrome(self) -> bool {
        matches!(self, Self::Monochrome)
    }

    /// Video bits in the CMOS equipment byte; the AT has no no-adapter code.
    pub const fn cmos_video_bits(self) -> u8 {
        match self {
            Self::None | Self::OwnBios => 0,
            Self::Colour80 => 0x20,
            Self::Monochrome => 0x30,
        }
    }
}

impl crate::MachineConfig {
    /// Resolve the board's primary display independently of card order.
    ///
    /// Auto prefers MDA so a dual-card AT retains its monochrome console.
    /// A ROM at C0000h identifies a video BIOS, matching firmware validation's
    /// video-ROM requirement; unrelated option ROMs do not change the display.
    /// IBM Personal Computer AT Technical Reference, System Board, Display
    /// Adapter Switch and "Equipment Byte (Hex 14)". See machine-config.md
    /// §Primary display for the no-card and ROM-less VGA policies.
    pub fn primary_display(&self) -> Result<PrimaryDisplay, crate::ConfigError> {
        let mono = self
            .devices
            .iter()
            .any(|d| matches!(d.kind, DeviceKind::Mda(_)));
        let colour = self
            .devices
            .iter()
            .any(|d| matches!(d.kind, DeviceKind::Vga(_) | DeviceKind::VgaRegisters(_)));
        let select_mono = match self.display_switch {
            DisplaySwitch::Auto => mono,
            DisplaySwitch::Mono if mono => true,
            DisplaySwitch::Colour if colour => false,
            switch => return Err(crate::ConfigError::DisplaySwitchUnavailable { switch }),
        };
        Ok(if select_mono {
            PrimaryDisplay::Monochrome
        } else if !colour {
            PrimaryDisplay::None
        } else if self.images.iter().enumerate().any(|(index, image)| {
            image.kind == crate::ImageKind::Rom
                && image.base == 0xC0000
                && self.system_bios_image != Some(index)
        }) {
            PrimaryDisplay::OwnBios
        } else {
            PrimaryDisplay::Colour80
        })
    }
}
