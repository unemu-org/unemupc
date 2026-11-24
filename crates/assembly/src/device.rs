// SPDX-License-Identifier: AGPL-3.0-or-later
//! Device configuration (§7).
//!
//! One [`DeviceConfig`] describes one modelled chip: its stable role
//! ([`DeviceId`]), the chip-specific parameters ([`DeviceKind`]), the port
//! windows the bus decodes to it, and its IRQ / DMA wiring — all as data. The
//! assembler resolves each [`DeviceId`] to a [`bus`] device index and turns the
//! port windows into a [`bus::DecodeMap`]; nothing here holds a bus index.

/// A stable identifier for a modelled device role (§7).
///
/// The config names devices by *role*, not by chip type or bus index: a caller
/// asks for [`DeviceId::Uart`] and the assembler hands back which bus slot it
/// landed in. Each role appears at most once in a machine (duplication is a
/// [`crate::ConfigError`]), which is why this is a plain enum rather than a
/// string — it is exhaustive over the roles this machine models and is a natural
/// map key ([`Hash`]/[`Eq`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceId {
    /// 16550 serial UART (COM1).
    Uart,
    /// 8253 programmable interval timer.
    Pit,
    /// Master 8259 interrupt controller.
    MasterPic,
    /// Slave 8259 interrupt controller (cascaded into the master).
    SlavePic,
    /// 8042 keyboard controller.
    Keyboard,
    /// MC146818 CMOS RAM / real-time clock.
    Rtc,
    /// VGA text-mode card.
    Vga,
    /// IBM Monochrome Display Adapter (display portion only).
    Mda,
    /// System control port (0x61: speaker gate, refresh, NMI status).
    SysControl,
    /// System Control Port A (92h), absent on IBM 5170.
    SysCtrlPortA,
    /// POST checkpoint port.
    Post,
    /// Emulator-only firmware diagnostic console.
    DebugConsole,
    /// Emulator-only firmware configuration.
    FwCfg,
    /// uPD765 floppy-disk controller.
    Fdc,
    /// Creative Sound Blaster 16 DSP.
    Sb16,
    /// ATA PIO channel.
    Ide,
    /// 8237 DMA controller.
    Dma,
    /// AT word DMA controller, system channels 4–7 (local 0–3).
    DmaWord,
    /// Empty 80287 socket and motherboard coprocessor controls.
    Coprocessor,
    /// Lo-tech expanded-memory board.
    LotechEms,
}

/// A contiguous, inclusive port range decoded to a device (§4.1).
///
/// The config counterpart of [`bus::PortRange`], without the bus index the
/// assembler fills in. A device may decode several windows (a VGA card decodes
/// its CRTC and CGA registers). [`PortWindow::allow_overlap`] permits overlap
/// only when one window strictly contains the other and both start at the same
/// port. Sorting by `(start, end)` then gives every shared port to the narrower
/// window through first-match decoding. Other overlaps require splitting the
/// wider window; unmarked overlaps are always errors.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PortWindow {
    /// First port in the range (inclusive).
    pub start: u16,
    /// Last port in the range (inclusive).
    pub end: u16,
    /// Maximum ISA cycle width; byte cycles unless explicitly selected.
    pub width: bus::PortWidth,
    /// Permit strict containment with another device's window at the same
    /// starting port, so `(start, end)` order gives the narrower window priority.
    /// This may be set on either window; it never permits partial overlap,
    /// identical windows or containment with different starts.
    pub allow_overlap: bool,
}

impl PortWindow {
    /// A non-overlapping window `[start, end]`.
    pub const fn new(start: u16, end: u16) -> Self {
        Self {
            start,
            end,
            width: bus::PortWidth::Byte,
            allow_overlap: false,
        }
    }

    /// A single-port window at `port`.
    pub const fn single(port: u16) -> Self {
        Self::new(port, port)
    }

    /// A window permitting the same-start containment described by [`Self::allow_overlap`].
    pub const fn overlapping(start: u16, end: u16) -> Self {
        Self {
            start,
            end,
            width: bus::PortWidth::Byte,
            allow_overlap: true,
        }
    }

    /// Select the ISA cycle width. Word windows must start at an even port
    /// and cover word registers declared by the device kind.
    /// A single even port may receive a complete word; see
    /// `backlog/docs/design/machine-config.md` §Port window width.
    #[must_use]
    pub const fn with_width(mut self, width: bus::PortWidth) -> Self {
        self.width = width;
        self
    }

    /// Whether this window and `other` share any port.
    pub fn overlaps(&self, other: &PortWindow) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

/// One device's DMA wiring: the controller and channel it transfers through
/// (§7.7).
///
/// A bus-master device (the floppy controller) cannot reach guest memory itself;
/// it queues a transfer that a DMA controller runs. This names the controller by
/// role and the channel it uses; the assembler turns it into a
/// [`machine::DmaWiring`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DmaSpec {
    /// The DMA controller this device masters the bus through.
    pub controller: DeviceId,
    /// System DMA channel: 0–3 for the byte controller, 5–7 for the word controller.
    /// Assembly translates word channels to the controller-local 1–3 numbering.
    pub channel: u8,
}

/// 16550 UART parameters (§7.2).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UartCfg {
    /// Base port (the eight UART registers decode from here). COM1 is `0x3F8`.
    pub base: u16,
    /// Attach a Microsoft two-button serial mouse; no endpoint by default.
    pub mouse: bool,
}

/// 8253 PIT selection (§7.4); port decoding lives in [`DeviceConfig::ports`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PitCfg {}

/// 8259 PIC parameters (§7.3).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PicCfg {
    /// Whether this PIC is cascaded (the master half of a master/slave pair).
    pub cascade: bool,
}

/// 8042 keyboard-controller selection (§7.5); ports live in [`DeviceConfig::ports`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct I8042Cfg {
    /// Attach a standard PS/2 mouse to the auxiliary port; absent by default.
    /// IBM PS/2 Hardware Interface Technical Reference, Common Interfaces,
    /// "Keyboard and Auxiliary Device Controller", Auxiliary Device Interface.
    pub mouse: bool,
}

/// Fixed calendar time for the MC146818, in binary, with a derived weekday.
///
/// Years 1980–2099 match the device's two-digit calendar and leap-year model.
/// Motorola MC146818 datasheet, Time, Calendar, and Alarm Locations, Table 3,
/// defines the time/date fields. See `backlog/docs/design/machine-config.md`
/// § RTC start and NVRAM source.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RtcStart {
    /// Full calendar year, 1980 through 2099.
    pub year: u16,
    /// Month, 1 through 12.
    pub month: u8,
    /// Day of the month, starting at 1.
    pub day: u8,
    /// Hour in 24-hour notation, 0 through 23.
    pub hour: u8,
    /// Minute, 0 through 59.
    pub minute: u8,
    /// Second, 0 through 59.
    pub second: u8,
}

impl Default for RtcStart {
    fn default() -> Self {
        Self {
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }
}

impl RtcStart {
    pub(crate) fn days_in_month(self) -> u8 {
        match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if self.year.is_multiple_of(4) => 29,
            2 => 28,
            _ => 0,
        }
    }

    pub(crate) fn invalid_field(self) -> Option<&'static str> {
        if !(1980..=2099).contains(&self.year) {
            Some("year")
        } else if !(1..=12).contains(&self.month) {
            Some("month")
        } else if !(1..=self.days_in_month()).contains(&self.day) {
            Some("day")
        } else if self.hour >= 24 {
            Some("hour")
        } else if self.minute >= 60 {
            Some("minute")
        } else if self.second >= 60 {
            Some("second")
        } else {
            None
        }
    }

    pub(crate) fn weekday(self) -> u8 {
        let years = self.year - 1980;
        let leap_days = years.div_ceil(4);
        let month_days: u16 = (1..self.month)
            .map(|month| u16::from(Self { month, ..self }.days_in_month()))
            .sum();
        ((2 + years * 365 + leap_days + month_days + u16::from(self.day - 1)) % 7 + 1) as u8
    }
}

impl std::fmt::Display for RtcStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Initial MC146818 / DS12887 storage and optional reset/finish persistence.
///
/// Motorola MC146818 datasheet, RAM and Time, Calendar, and Alarm Locations:
/// the original 64-byte address space contains clock/control registers and RAM.
/// Dallas Semiconductor DS12887 datasheet, Address Map, extends it to 128 bytes.
/// Derived and Blank use the configured capacity; images and journals remain
/// limited to the original 64-byte part.
/// See `backlog/docs/design/machine-config.md` § RTC start and NVRAM source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NvramSource {
    /// Derive firmware-owned CMOS configuration and its AT checksum. A 128-byte
    /// RTC receives the full QEMU layout when the system firmware opts into it.
    #[default]
    Derived,
    /// Leave configuration RAM blank, retaining the chip's control defaults.
    Blank,
    /// Copy writable backing bytes without checksum repair; clear live status.
    Image([u8; 64]),
    /// Immutable file base and journal; only general RAM is persisted.
    Journal {
        /// Original 64-byte image, read and hashed once at assembly.
        image: std::path::PathBuf,
        /// Mandatory binding to the bytes verified during lowering.
        expected_sha256: [u8; 32],
        /// Separate append-only journal output.
        journal: std::path::PathBuf,
        /// Positive closed-generation retention limit.
        keep_generations: u32,
    },
}

pub use devices::cmos_rtc::CmosRam;

/// MC146818 / DS12887 CMOS/RTC parameters (§7.6).
///
/// Motorola MC146818 datasheet, Time, Calendar, and Alarm Locations and
/// Registers A–D; Dallas Semiconductor DS12887 datasheet, Address Map.
/// See `backlog/docs/design/machine-config.md`
/// § RTC start and NVRAM source for initialization precedence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CmosCfg {
    /// Address-space capacity; defaults to the original MC146818 64 bytes.
    pub ram: CmosRam,
    /// Deterministic calendar seed; never read from the host clock.
    pub start: RtcStart,
    /// Configuration RAM seed or a 64-byte image with live status cleared.
    pub nvram: NvramSource,
    /// Virtual-clock timing `(second_ticks, uip_ticks)`; `None` uses the
    /// device's default. Short ratios let bounded test rigs observe updates.
    pub timing: Option<(u64, u64)>,
}

/// VGA text-card parameters (§9.2).
///
/// Port decoding lives in [`DeviceConfig::ports`]; the text buffer is an MMIO
/// window in [`crate::MemoryConfig`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VgaTextCfg {}

/// Register-level IBM VGA (IBM VGA/XGA Technical Reference Manual,
/// General Registers and Memory Organization). No option ROM is installed.
/// See `backlog/docs/design/machine-config.md`, §Display cards.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VgaCfg {}

impl VgaCfg {
    /// Exact register decode, excluding the monochrome printer function.
    #[must_use]
    pub fn port_windows() -> Vec<PortWindow> {
        vec![
            PortWindow::new(0x3B4, 0x3B5),
            PortWindow::single(0x3BA),
            PortWindow::new(0x3C0, 0x3C3),
            PortWindow::new(0x3C4, 0x3CA),
            PortWindow::single(0x3CC),
            PortWindow::new(0x3CE, 0x3CF),
            PortWindow::new(0x3D4, 0x3D5),
            PortWindow::single(0x3DA),
            PortWindow::single(0x102),
            PortWindow::single(0x46E8),
        ]
    }
}

/// IBM Monochrome Display Adapter selection (display portion only).
///
/// IBM PC Technical Reference, Options and Adapters, Monochrome Display and
/// Printer Adapter, Programming Considerations: display ports are 3B0–3BB and
/// 3BF; printer ports 3BC–3BE belong to a separate function. See
/// `backlog/docs/design/machine-config.md` §Presets.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MdaTextCfg {}

/// System-control selection (§7); port decoding lives in [`DeviceConfig::ports`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SysControlCfg {}

/// POST-checkpoint-port parameters (§7).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PostPortCfg {
    /// The checkpoint port the guest writes progress bytes to.
    pub port: u16,
}

/// Emulator-only QEMU debugcon protocol parameters.
///
/// SeaBIOS uses 402h; a bare test rig may relocate it. See
/// `backlog/docs/design/machine-config.md` §Emulator-only debug console.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DebugConsoleCfg {
    /// The single byte-wide diagnostic port.
    pub port: u16,
}

impl Default for DebugConsoleCfg {
    fn default() -> Self {
        Self {
            port: devices::debug_console::DEBUG_CONSOLE_PORT,
        }
    }
}

/// Emulator-only firmware files, frozen at assembly time.
///
/// See `backlog/docs/design/machine-config.md` §Emulator-only firmware configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FwCfgCfg {
    /// Inline files in directory order; no host I/O occurs during execution.
    pub files: Vec<devices::FwCfgFile>,
    /// Append etc/sercon-port from the UART at DeviceId::Uart explicitly.
    pub sercon_from_com1: bool,
}

impl FwCfgCfg {
    /// Preserve the overlapping word selector without widening the data port.
    pub fn port_windows() -> Vec<PortWindow> {
        vec![
            PortWindow::single(0x510).with_width(bus::PortWidth::Word),
            PortWindow::single(0x511),
        ]
    }
}

/// uPD765 floppy-controller parameters (§7.7).
///
/// Drives are not listed here — they live once in the top-level
/// [`crate::MachineConfig::drives`] and bind by the controller's role and a local
/// `unit`. This declares only how many drive slots the controller offers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FdcCfg {
    /// Number of drive slots this controller offers (a standard FDC drives two,
    /// units 0 and 1).
    pub drive_slots: u8,
}

impl FdcCfg {
    /// AT diskette-controller decode, excluding 3F6h when primary ATA uses it.
    ///
    /// IBM Personal Computer AT Technical Reference, System Board, I/O Address
    /// Map lists 3F0h–3F7h for diskette and 1F0h–1F8h for fixed disk. Its Fixed
    /// Disk and Diskette Drive Adapter section identifies the fixed-disk register
    /// at 3F6h and diskette registers at 3F2h, 3F4h, 3F5h and 3F7h.
    /// The split decode (3F0h–3F5h plus 3F7h) is this model's choice: unused
    /// 3F0h, 3F1h and 3F3h stay with the FDC, as in the unsplit window.
    /// Without a primary ATA control port, retain the legacy 3F0h–3F7h window.
    /// In this model 3F7h belongs entirely to the FDC (digital input register on
    /// read, configuration control register on write). The real adapter shares
    /// its read bits with the fixed disk; that bit-level sharing is not modelled.
    /// See `backlog/docs/design/machine-config.md` §Explicit decode-conflict policy.
    pub fn port_windows(primary_ata_control: bool) -> Vec<PortWindow> {
        if primary_ata_control {
            vec![PortWindow::new(0x3F0, 0x3F5), PortWindow::single(0x3F7)]
        } else {
            vec![PortWindow::new(0x3F0, 0x3F7)]
        }
    }
}

/// ATA-2 PIO channel resources, with two drive positions.
///
/// ATA-2 §§6.1–6.2 defines the task file and word data register; IBM PC AT
/// Technical Reference, Fixed Disk and Diskette Drive Adapter, Programming
/// Interface defines the primary ports. See
/// `backlog/docs/design/machine-config.md` §The fixed-disk (IDE/HDD) attachment seam.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IdeCfg {
    /// ATA device registers or adapter-owned AT task latches.
    pub model: devices::ata::AtaControllerModel,
    /// Task-file base: 1F0h (primary) or 170h (secondary).
    pub base: u16,
    /// ATA alternate status / device control; AT adapter writes only, base plus 206h.
    pub control: u16,
}

impl IdeCfg {
    /// Task-file decode with IOCS16# asserted only for the data register.
    pub fn port_windows(self) -> Vec<PortWindow> {
        vec![
            PortWindow::single(self.base).with_width(bus::PortWidth::Word),
            PortWindow::new(self.base.saturating_add(1), self.base.saturating_add(7)),
            PortWindow::single(self.control),
        ]
    }

    /// Primary IRQ14 or secondary IRQ15.
    pub fn irq(self) -> u8 {
        if self.base == 0x170 {
            15
        } else {
            14
        }
    }
}

/// Creative Sound Blaster 16 ISA resource selection.
///
/// Creative Sound Blaster Series Hardware Programming Guide, §2 DSP I/O Ports
/// and §2 Configuring DMA and Interrupt Settings. ASSUMED: the accepted jumper-style
/// base set is 220h/240h/260h/280h. See
/// `backlog/docs/design/machine-files.md` §SB16.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Sb16Cfg {
    /// Word-DMA channel, one of 5, 6 or 7.
    pub dma16: u8,
    /// DSP, mixer and OPL alias base address.
    pub base: u16,
}

impl Sb16Cfg {
    /// DSP, mixer, FM aliases and both interrupt acknowledgements are byte ports.
    pub fn port_windows(self) -> Vec<PortWindow> {
        [0, 1, 2, 3, 4, 5, 6, 8, 9, 10, 12, 14, 15]
            .into_iter()
            .map(|offset| PortWindow::single(self.base.saturating_add(offset)))
            .chain((0x388..=0x38b).map(PortWindow::single))
            .collect()
    }
}

/// 8237 DMA-controller selection (§7.7); ports live in [`DeviceConfig::ports`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DmaCfg {}

/// Lo-tech 2MB EMS Board schematic, revision r02: HCT688 port jumpers,
/// frame decode and four possible 512 KiB SRAM chips.
/// See `backlog/docs/architecture.md` §x86jit integration.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LotechEmsCfg {
    /// Jumpered I/O base: 260h, 264h, 268h or 26Ch.
    pub port: u16,
    /// Jumpered segment: C000h, D000h or E000h.
    pub frame: u16,
    /// Populated SRAM in KiB: 512, 1024, 1536 or 2048.
    pub size_kib: u32,
}

impl Default for LotechEmsCfg {
    fn default() -> Self {
        crate::descriptors::EMS_DEFAULT
    }
}

impl LotechEmsCfg {
    /// Whether the jumper positions and SRAM population exist on the board.
    pub fn is_valid(self) -> bool {
        self.invalid_field().is_none()
    }

    /// First invalid hardware field, using the shared configuration domains.
    pub fn invalid_field(self) -> Option<&'static str> {
        if !devices::LotechEms::valid_port(self.port) {
            Some("port")
        } else if !devices::LotechEms::valid_frame(self.frame) {
            Some("frame")
        } else if !devices::LotechEms::valid_size_kib(self.size_kib) {
            Some("size_kib")
        } else {
            None
        }
    }

    /// HCT688 ignores A10–A15, so every alias must participate in conflict checks.
    pub fn port_windows(self) -> Vec<PortWindow> {
        (0..64u16)
            .map(|alias| {
                let base = self.port.saturating_add(alias * 0x400);
                PortWindow::new(base, base.saturating_add(3))
            })
            .collect()
    }
}

/// The chip-specific parameters of a [`DeviceConfig`] (§7).
///
/// One variant per modelled chip. The variant fixes *which* chip a device is;
/// the surrounding [`DeviceConfig`] carries the wiring (ports, IRQ, DMA) common
/// to all of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// 16550 UART.
    Uart(UartCfg),
    /// 8253 PIT.
    Pit(PitCfg),
    /// 8259 PIC.
    Pic(PicCfg),
    /// 8042 keyboard controller.
    Keyboard(I8042Cfg),
    /// MC146818 CMOS/RTC.
    Rtc(CmosCfg),
    /// VGA text card.
    Vga(VgaTextCfg),
    /// Register-level IBM VGA with planar memory and alphanumeric scanout.
    VgaRegisters(VgaCfg),
    /// IBM Monochrome Display Adapter (display portion only).
    Mda(MdaTextCfg),
    /// System control port.
    SysControl(SysControlCfg),
    /// PS/2-style alternate A20 and reset controls at 92h, confined to bare rigs.
    SysCtrlPortA,
    /// POST checkpoint port.
    Post(PostPortCfg),
    /// Emulator-only firmware diagnostic console, confined to bare test rigs.
    DebugConsole(DebugConsoleCfg),
    /// Emulator-only traditional firmware configuration, confined to bare rigs.
    FwCfg(FwCfgCfg),
    /// uPD765 floppy controller.
    Fdc(FdcCfg),
    /// Creative Sound Blaster 16 byte-DMA output.
    Sb16(Sb16Cfg),
    /// ATA PIO channel.
    Ide(IdeCfg),
    /// 8237 DMA controller.
    Dma(DmaCfg),
    /// AT word/cascade configuration of the 8237A.
    DmaWord(DmaCfg),
    /// IBM AT socket controls; assembly connects ERROR# when an FPU is fitted.
    AbsentCoprocessor,
    /// Lo-tech 2MB expanded-memory board.
    LotechEms(LotechEmsCfg),
}

/// One device in a [`crate::MachineConfig`] (§7).
///
/// The `id` is the role callers and the assembler address the device by; `kind`
/// carries its chip parameters; `ports` are the windows the bus decodes to it;
/// `irq` is the PIC input line it drives (if any); `dma` is its DMA wiring (if
/// it masters the bus). Marking `allow_port_overlap` permits this device's
/// windows to overlap another device's under the same-start strict containment
/// rule of [`PortWindow::allow_overlap`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceConfig {
    /// The device's stable role.
    pub id: DeviceId,
    /// The chip and its parameters.
    pub kind: DeviceKind,
    /// The port windows the bus decodes to this device.
    pub ports: Vec<PortWindow>,
    /// The PIC input line this device drives, if it raises interrupts.
    pub irq: Option<u8>,
    /// This device's DMA wiring, if it masters the bus.
    pub dma: Option<DmaSpec>,
    /// Devices whose next event an access to this device can change. The bus
    /// synchronizes them before the access, after this device, in config order.
    /// Directed edges; list indirect dependents explicitly. Empty by default.
    pub access_dependents: Vec<DeviceId>,
    /// Permit overlaps under the same-start containment rule of [`PortWindow::allow_overlap`].
    pub allow_port_overlap: bool,
}

impl DeviceConfig {
    /// A device with the given role, kind, and decoded ports, no IRQ or DMA
    /// wiring and no overlap allowance.
    pub fn new(id: DeviceId, kind: DeviceKind, ports: Vec<PortWindow>) -> Self {
        Self {
            id,
            kind,
            ports,
            irq: None,
            dma: None,
            access_dependents: Vec::new(),
            allow_port_overlap: false,
        }
    }

    /// System DMA inputs, including the SB16's independent word input (§7.7).
    /// IBM Personal Computer AT Technical Reference, System Board, Direct Memory Access.
    pub(crate) fn dma_inputs(&self) -> impl Iterator<Item = DmaSpec> {
        self.dma.into_iter().chain(match self.kind {
            DeviceKind::Sb16(cfg) => Some(DmaSpec {
                controller: DeviceId::DmaWord,
                channel: cfg.dma16,
            }),
            _ => None,
        })
    }

    /// Set the PIC input line this device drives.
    #[must_use]
    pub fn with_irq(mut self, irq: u8) -> Self {
        self.irq = Some(irq);
        self
    }

    /// All wired `(device output, PIC input)` pairs, primary output first (§5.1).
    /// Assembly and resource validation share this description so auxiliary
    /// outputs cannot bypass IRQ ownership checks. IBM PS/2 Hardware Interface
    /// Technical Reference, Common Interfaces, "Keyboard and Auxiliary Device
    /// Controller", Interrupts: the auxiliary port has a separate IRQ12 output.
    /// IBM Personal Computer AT Technical Reference, System Board, Interrupts:
    /// IRQ2 is the cascade; the compatible ISA slot pin drives IRQ9 instead.
    pub fn interrupt_outputs(&self) -> impl Iterator<Item = (usize, u8)> + '_ {
        // The AT slot's IRQ2 pin is wired to IRQ9. SB16 is currently the only
        // card whose configuration offers IRQ2; other IRQ selections pass through.
        let primary = self.irq.map(|irq| {
            (
                0,
                if matches!(self.kind, DeviceKind::Sb16(_)) && irq == 2 {
                    9
                } else {
                    irq
                },
            )
        });
        let auxiliary =
            matches!(&self.kind, DeviceKind::Keyboard(cfg) if cfg.mouse).then_some((1, 12));
        primary.into_iter().chain(auxiliary)
    }

    /// Set this device's DMA wiring.
    #[must_use]
    pub fn with_dma(mut self, dma: DmaSpec) -> Self {
        self.dma = Some(dma);
        self
    }

    /// Permit overlaps under the same-start containment rule of [`PortWindow::allow_overlap`].
    #[must_use]
    pub fn allowing_port_overlap(mut self) -> Self {
        self.allow_port_overlap = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupt_outputs_include_the_primary_and_optional_auxiliary_routes() {
        let mut controller = DeviceConfig::new(
            DeviceId::Keyboard,
            DeviceKind::Keyboard(I8042Cfg::default()),
            vec![],
        )
        .with_irq(1);
        assert_eq!(controller.interrupt_outputs().collect::<Vec<_>>(), [(0, 1)]);
        controller.kind = DeviceKind::Keyboard(I8042Cfg { mouse: true });
        assert_eq!(
            controller.interrupt_outputs().collect::<Vec<_>>(),
            [(0, 1), (1, 12)]
        );
        controller.irq = None;
        assert_eq!(
            controller.interrupt_outputs().collect::<Vec<_>>(),
            [(1, 12)]
        );
        controller.kind = DeviceKind::Keyboard(I8042Cfg::default());
        assert_eq!(controller.interrupt_outputs().count(), 0);
    }

    #[test]
    fn isa_port_window_constructors_default_to_byte_cycles() {
        assert_eq!(bus::PortWidth::default(), bus::PortWidth::Byte);
        for window in [
            PortWindow::new(0x100, 0x103),
            PortWindow::single(0x100),
            PortWindow::overlapping(0x100, 0x103),
        ] {
            assert_eq!(window.width, bus::PortWidth::Byte);
            assert_eq!(
                window.with_width(bus::PortWidth::Word).width,
                bus::PortWidth::Word
            );
        }
    }

    #[test]
    fn initialization_weekdays_match_gregorian_reference_for_every_supported_date() {
        // Zeller's congruence uses a March-based year, independently of the
        // production January anchor and cumulative month-day calculation.
        for year in 1980..=2099 {
            for month in 1..=12 {
                let max_day = match month {
                    4 | 6 | 9 | 11 => 30,
                    2 if year % 4 == 0 => 29,
                    2 => 28,
                    _ => 31,
                };
                for day in 1..=max_day {
                    let start = RtcStart {
                        year,
                        month,
                        day,
                        ..RtcStart::default()
                    };
                    assert_eq!(start.invalid_field(), None, "{start}");
                    let (y, m) = if month <= 2 {
                        (u32::from(year) - 1, u32::from(month) + 12)
                    } else {
                        (u32::from(year), u32::from(month))
                    };
                    let h = (u32::from(day) + 13 * (m + 1) / 5 + y + y / 4 - y / 100 + y / 400) % 7;
                    let weekday = ((h + 6) % 7 + 1) as u8;
                    assert_eq!(start.weekday(), weekday, "{start}");
                }
            }
        }
    }

    #[test]
    fn initialization_weekdays_include_dos_epoch_and_current_year_leap_day() {
        for (year, month, day, expected) in [
            (1980, 1, 1, 3),
            (1984, 3, 1, 5),
            (1999, 12, 31, 6),
            (2000, 1, 1, 7),
            (2024, 2, 29, 5),
            (2024, 3, 1, 6),
        ] {
            let start = RtcStart {
                year,
                month,
                day,
                ..RtcStart::default()
            };
            assert_eq!(start.weekday(), expected, "{start}");
        }
    }
}
