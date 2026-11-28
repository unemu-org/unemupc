// SPDX-License-Identifier: AGPL-3.0-or-later
//! Configuration validation errors (§3.1).

use thiserror::Error;

use crate::device::DeviceId;

/// A way a [`crate::MachineConfig`] is internally inconsistent (§3.1).
///
/// Each variant names one class of failure [`crate::MachineConfig::validate`]
/// catches before any hardware is assembled. Variants derive [`PartialEq`] so a
/// test can assert the exact failure a malformed config produces.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// Jumpers, SRAM population, decode or wiring disagree with the EMS board.
    #[error("invalid Lo-tech EMS resources for {device:?}: require port 260h/264h/268h/26Ch (all aliases), frame C000h/D000h/E000h (64 KiB MMIO), 512/1024/1536/2048 KiB and no IRQ/DMA")]
    InvalidLotechEmsResources { device: DeviceId },
    /// Bochs VBE resources must match the card's fixed decode.
    #[error("invalid Bochs VBE resources for {device:?}: require 4, 8 or 16 MiB of video memory, the VGA and DISPI 1CEh-1D0h ports, the A0000h-BFFFFh window and no IRQ/DMA")]
    InvalidBochsVbeResources { device: DeviceId },
    /// The linear framebuffer sits at E0000000h, beyond a 24-bit bus.
    #[error("the Bochs VBE linear framebuffer of {device:?} at E0000000h needs a 32-bit address bus; board \"{board}\" with this CPU has {bits} bits")]
    BochsVbeAddressSpace {
        /// The card.
        device: DeviceId,
        /// Selected motherboard.
        board: &'static str,
        /// Address width the board decodes.
        bits: u8,
    },
    /// The linear framebuffer must not overlap RAM, a ROM or an MMIO window.
    #[error("the Bochs VBE linear framebuffer of {device:?} overlaps another mapping (RAM, ROM or MMIO) at {addr:#x}")]
    BochsVbeOverlap {
        /// The card.
        device: DeviceId,
        /// First overlapping RAM address.
        addr: u64,
    },
    /// SB16 resources must match its byte/word-DMA ISA wiring and DSP/mixer decode.
    #[error("invalid SB16 resources for {device:?}: bases 220h/240h/260h/280h, IRQ 2/5/7/10, DMA8 0/1/3, DMA16 5/6/7 and DSP/mixer byte ports required")]
    InvalidSb16Resources {
        /// Card with inconsistent wiring.
        device: crate::DeviceId,
    },
    /// Word-controller DMA specs use system channels, never local/cascade channels.
    #[error("invalid system DMA channel {channel} on {controller:?} for {device:?}: word channels must be 5/6/7")]
    InvalidDmaChannel {
        /// Requester with invalid wiring.
        device: crate::DeviceId,
        /// Selected word controller.
        controller: crate::DeviceId,
        /// Invalid system channel.
        channel: u8,
    },
    /// Two devices cannot own the same DMA channel on one controller.
    #[error("DMA channel {channel} on {controller:?} is shared by {a:?} and {b:?}")]
    DmaConflict {
        /// First requester.
        a: crate::DeviceId,
        /// Second requester.
        b: crate::DeviceId,
        /// Shared controller.
        controller: crate::DeviceId,
        /// Channel both requesters own.
        channel: u8,
    },
    /// A forced board switch position needs a matching installed adapter.
    #[error("display switch {switch} requires an installed {}", .switch.required_card_class())]
    DisplaySwitchUnavailable {
        /// Requested physical switch position.
        switch: crate::DisplaySwitch,
    },
    /// Host input must have one unambiguous pointing-device route.
    #[error("only one host mouse target may be configured (PS/2 or serial)")]
    MultipleMouseTargets,
    /// Auxiliary mice require the board's keyboard controller and slave PIC.
    #[error("PS/2 mouse: {reason}")]
    UnsupportedMouse {
        /// Missing controller, missing slave PIC, or unsupported board.
        reason: String,
    },
    /// The IBM AT motherboard requires its original RTC part.
    #[error("board ibm-5170 requires a 64-byte RTC: the IBM AT has an MC146818")]
    Ibm5170RtcRam,
    /// Persistent images retain the original 64-byte format.
    #[error("128-byte NVRAM images are not supported yet")]
    UnsupportedExtendedNvram,
    /// IBM 5170 has no alternate A20/reset control port.
    #[error("sys-ctrl-port-a at 0x92 is absent on ibm-5170; it is allowed only on bare rigs and future generic-386-isa")]
    SysCtrlPortABoard,
    /// Alternate board controls require their fixed byte port and no interrupt/DMA.
    #[error(
        "device {device:?} sys-ctrl-port-a requires one byte-wide port at 0x92 and no IRQ or DMA"
    )]
    InvalidSysCtrlPortAResources {
        /// Role with inconsistent board control resources.
        device: DeviceId,
    },
    /// Both VGA models decode the same ports and video memory.
    #[error("vga and vga-text cards are mutually exclusive (shared ports and MMIO)")]
    ConflictingVgaCards,

    /// Emulator firmware configuration cannot be attached to a period motherboard.
    #[error("fw-cfg port is emulator-only; it is allowed only on board generic-386-isa")]
    FwCfgBoard,
    /// Selector and data decode, IRQ or DMA wiring is inconsistent.
    #[error("device {device:?} fw-cfg requires a word selector at 0x510, byte data at 0x511 and no IRQ or DMA")]
    InvalidFwCfgResources { device: DeviceId },
    /// Inline files cannot be encoded in the firmware directory.
    #[error("device {device:?}: {source}")]
    InvalidFwCfgFiles {
        device: DeviceId,
        source: devices::FwCfgError,
    },
    /// The explicit serial console convenience needs a COM1 UART.
    #[error("device {device:?} fw-cfg sercon_from_com1 requires a UART at DeviceId::Uart")]
    FwCfgCom1Missing { device: DeviceId },
    /// Emulator diagnostics cannot be attached to a period motherboard.
    #[error("debug-console port is emulator-only; it is allowed only on board generic-386-isa")]
    DebugConsoleBoard,
    /// Diagnostic decode must describe exactly one byte-wide port, without IRQ/DMA.
    #[error("device {device:?} debug-console requires one byte-wide port matching DebugConsoleCfg.port and no IRQ or DMA")]
    InvalidDebugConsoleResources {
        /// Role with inconsistent diagnostic resources.
        device: DeviceId,
    },
    /// Invalid CMOS base, digest, retention or output path.
    #[error("NVRAM: {message}")]
    Nvram { message: String },
    /// Invalid interleaved system ROM layout or source role.
    #[error("firmware parts: {message}")]
    FirmwareParts { message: String },
    /// Known chips have been assigned to opposite lanes.
    #[error("firmware parts are swapped for {firmware}: {chips}")]
    FirmwarePartsSwapped {
        firmware: &'static str,
        chips: String,
    },
    /// A recognized layout contains misplaced chips or incorrect dumps.
    #[error("firmware parts do not match {firmware}: {mismatches}")]
    FirmwarePartsMismatch {
        firmware: &'static str,
        mismatches: String,
    },
    /// The file changed after its bytes were verified by the configuration loader.
    #[error("drive {drive_index} image sha256 mismatch: expected {expected}, actual {actual}")]
    DriveImageSha256 {
        /// Index of the drive whose base image changed.
        drive_index: usize,
        /// Verified SHA-256, in lowercase hexadecimal.
        expected: String,
        /// SHA-256 of the bytes assembly would mount, in lowercase hexadecimal.
        actual: String,
    },
    /// A journal mode violates the persistent-media config contract.
    #[error("drive {drive_index} journal configuration: {message}")]
    Journal { drive_index: usize, message: String },
    /// A file-backed drive input violates the persistent-media namespace contract.
    #[error("drive {drive_index} input {path}: {message}", path = path.display())]
    DriveInput {
        drive_index: usize,
        path: std::path::PathBuf,
        message: String,
    },
    /// A loadable image input violates the persistent-media namespace contract.
    #[error("{role} image {path}: {message}", path = path.display())]
    ImageInput {
        image_index: usize,
        role: &'static str,
        path: std::path::PathBuf,
        message: String,
    },
    /// A catalogue board cannot boot without a system ROM.
    #[error("board \"{board}\" requires a system BIOS ROM")]
    MissingSystemBios {
        /// Selected motherboard identifier.
        board: &'static str,
    },
    /// Firmware boards always enter through their hardware reset vector.
    #[error("board \"{board}\" requires power-on ResetVector F000:FFF0; got {actual}", actual = format_reset(.reset))]
    FirmwareReset {
        /// Selected motherboard identifier.
        board: &'static str,
        /// Rejected reset entry.
        reset: crate::ResetSpec,
    },
    /// Writable RAM cannot shadow the checked system ROM.
    #[error("system BIOS for board \"{board}\" overlaps RAM at {addr:#x}")]
    SystemBiosOverlapsRam {
        /// Selected motherboard identifier.
        board: &'static str,
        /// First overlapping address.
        addr: u64,
    },
    /// No reviewed build has these image bytes.
    #[error(
        "bios image (sha256 {sha256}) is not a known image for board \"{board}\"; known: {known}"
    )]
    FirmwareNotListed {
        /// Full SHA-256 of the loaded image.
        sha256: String,
        /// Selected board identifier.
        board: &'static str,
        /// Known entries and their distribution instructions.
        known: String,
    },
    /// A declaration cannot grant services absent from a recognized firmware.
    #[error("firmware \"{firmware}\" declares provides {declared}; catalogue provides {expected}")]
    FirmwareProvidesMismatch {
        /// Firmware image index.
        image_index: usize,
        /// Matched catalogue entry.
        firmware: &'static str,
        /// Canonically ordered declared set.
        declared: String,
        /// Canonically ordered catalogue set.
        expected: String,
    },
    /// The single IDE role cannot identify two independent channels.
    #[error("only one IDE channel is supported: DeviceId::Ide identifies one role, not separate primary and secondary instances")]
    MultipleIdeChannels,
    /// ATA decode must match the configured task/control resources exactly.
    #[error("device {device:?} IDE port windows must match IdeCfg::port_windows(): word data at base, byte task registers at base + 1..=7, and byte control at base + 0x206")]
    InvalidIdePortWindows {
        /// Controller with inconsistent decode.
        device: DeviceId,
    },
    /// The image was built for another motherboard.
    #[error("firmware \"{firmware}\" is built for {boards}, not {board}")]
    FirmwareBoardMismatch {
        /// Catalogue identifier.
        firmware: &'static str,
        /// Compatible boards.
        boards: String,
        /// Selected board.
        board: &'static str,
    },
    /// The current CPU predates the firmware minimum.
    #[error("firmware \"{firmware}\" needs an {minimum} or later; cpu is {cpu}")]
    FirmwareNeedsCpu {
        /// Catalogue identifier.
        firmware: &'static str,
        /// Required generation.
        minimum: crate::FirmwareCpu,
        /// Configured generation.
        cpu: crate::FirmwareCpu,
    },
    /// A required board part is absent.
    #[error("firmware \"{firmware}\" requires {part}; none is configured")]
    FirmwareRequiresDevice {
        /// Catalogue identifier.
        firmware: &'static str,
        /// Missing board part.
        part: crate::FirmwarePart,
    },
    /// This system firmware has no adapter ROM initialization scan.
    #[error("firmware {firmware} does not scan for option ROMs")]
    FirmwareDoesNotScanOptionRoms {
        /// System firmware identifier.
        firmware: &'static str,
        /// Index of the refused option ROM.
        image_index: usize,
    },
    /// This system firmware has no reviewed video ROM initialization scan.
    #[error("system BIOS {firmware} has no known video-ROM scan at C0000h-C7FFFh")]
    FirmwareDoesNotScanVideoRoms {
        /// Identified system firmware, or "unlisted".
        firmware: &'static str,
        /// Index of the refused video ROM.
        image_index: usize,
    },
    /// A video adapter image cannot supply the CPU reset firmware.
    #[error("video BIOS {firmware} cannot be used as the system BIOS")]
    VideoBiosAsSystemBios {
        /// Video firmware catalogue identifier.
        firmware: &'static str,
    },
    /// A reviewed extension ROM must use the extension scan window.
    #[error("{firmware} is an extension option ROM and cannot occupy the video-ROM window C0000h-C7FFFh")]
    OptionRomInVideoWindow {
        /// Index of the refused option ROM.
        image_index: usize,
        /// Extension firmware catalogue identifier.
        firmware: &'static str,
    },
    /// Adapter firmware is not a reviewed build for this board.
    #[error("option ROM image (sha256 {sha256}) is not a known image for board \"{board}\"; known: {known}")]
    OptionRomNotListed {
        /// Index in the image list.
        image_index: usize,
        /// Full image digest.
        sha256: String,
        /// Selected board.
        board: &'static str,
        /// Known adapter firmware builds.
        known: String,
    },
    /// Baked-in ATA resources need matching hardware.
    #[error("option ROM \"{firmware}\" drives ATA at 0x1F0/IRQ14, but no ide card decodes 0x1F0")]
    OptionRomCardMismatch {
        /// Index in the image list.
        image_index: usize,
        /// Matched firmware identifier.
        firmware: &'static str,
    },
    /// The extension scanner visits only 2 KiB boundaries.
    #[error("option ROM at {base:#X} must be 2 KiB aligned")]
    OptionRomMisaligned {
        /// Index in the image list.
        image_index: usize,
        /// Physical address.
        base: u64,
    },
    /// SeaBIOS allocates the first VGA file at a fixed shadow address.
    #[error("video ROM base {base:#X} is unsupported: SeaBIOS places it at C0000h")]
    SeaBiosVideoRomBase {
        /// Index in the image list.
        image_index: usize,
        /// Declared physical address.
        base: u64,
    },
    /// The complete image must fit the selected firmware's scan window.
    #[error(
        "option ROM at {base:#X} is outside the scan window {start:#X}..{end:#X} of \"{firmware}\""
    )]
    OptionRomOutsideScanWindow {
        /// Index in the image list.
        image_index: usize,
        /// Image address.
        base: u64,
        /// First scanned byte.
        start: u64,
        /// Exclusive scan end.
        end: u64,
        /// System firmware identifier.
        firmware: &'static str,
    },
    /// The signature or declared length cannot describe this image.
    #[error("option ROM {name}: {reason}")]
    OptionRomBadHeader {
        /// Index in the image list.
        image_index: usize,
        /// Catalogue identifier or physical address.
        name: String,
        /// Failed format invariant.
        reason: String,
    },
    /// A byte sum over the declared image must be zero.
    #[error("option ROM {name}: byte sum is {sum:#04X}, must be 0 (pad and checksum the image)")]
    OptionRomBadChecksum {
        /// Index in the image list.
        image_index: usize,
        /// Catalogue identifier or physical address.
        name: String,
        /// Modulo-256 byte sum.
        sum: u8,
    },
    /// Adapter address space cannot be writable system RAM.
    #[error("board \"{board}\" adapter ROM space overlaps ram at {addr:#x}")]
    AdapterSpaceOverlapsRam {
        /// Selected motherboard.
        board: &'static str,
        /// First overlapping address.
        addr: u64,
    },

    /// The emulator ISA board's CPU, RAM or fixed firmware wiring is invalid.
    #[error("board \"{board}\" {reason}")]
    BoardConfiguration {
        board: &'static str,
        reason: &'static str,
    },

    /// Configured memory must leave the board's fixed ROM alias to assembly.
    #[error("board \"{board}\" high ROM alias overlaps configured memory at {addr:#x}")]
    HighRomAliasOverlap {
        /// Selected motherboard.
        board: &'static str,
        /// First overlapping address.
        addr: u64,
    },

    /// Extended RAM exceeds the board's expansion-memory range.
    #[error("board \"{board}\" extended RAM exceeds {max_kib} KiB at {addr:#x}")]
    ExtendedRamLimit {
        /// Selected motherboard.
        board: &'static str,
        /// Maximum supported extended memory, in KiB.
        max_kib: u32,
        /// First unsupported address.
        addr: u64,
    },

    /// The board's fixed decode requires a matching CPU address bus.
    #[error("board \"{board}\" requires a 24-bit physical address bus, CPU declares {bits} bits")]
    BoardPhysicalWidth {
        /// Selected motherboard.
        board: &'static str,
        /// Address width declared by the CPU model.
        bits: u8,
    },

    /// A configured mapping is outside the AT's physical bus.
    /// Machine files and presets declare at most 16 MiB, so their bounds check
    /// reports `RegionOutOfSpace` first. This error diagnoses Rust configurations
    /// that declare a larger address space but still select the AT board.
    #[error("board \"{board}\" configured memory at {addr:#x} exceeds the 24-bit physical address space")]
    BoardPhysicalAddressOutOfRange {
        /// Selected motherboard.
        board: &'static str,
        /// First address outside the physical bus.
        addr: u64,
    },

    /// An assembly-time ROM copy cannot reproduce device MMIO reads.
    #[error(
        "board \"{board}\" MMIO at {addr:#x} overlaps the high ROM alias source 0xe0000..0x100000"
    )]
    HighRomAliasSourceMmio {
        /// Selected motherboard.
        board: &'static str,
        /// First overlapping source address.
        addr: u64,
    },

    /// The EMS frame conflicts with the IBM 5170 system-board ROM decode.
    #[error("Lo-tech EMS device {device:?} frame 0xE000 overlaps IBM 5170 system-board ROM; use frame 0xD000")]
    LotechEmsFrameInSystemRom {
        /// Card whose frame must be moved out of system-board ROM space.
        device: DeviceId,
    },

    /// System ROM must have a valid size and end at the top of the first MiB.
    #[error("system BIOS of {size_text} at {base:#x} must have a positive size that is a multiple of 8 KiB and end at 0x100000", size_text = format_bios_size(.size))]
    BiosPlacement {
        /// Guest-physical image base.
        base: u64,
        /// Raw image length in bytes.
        size: u64,
    },

    /// ATA resources must match one standard PIO channel.
    #[error("device {device:?} has invalid IDE resources: base must be 0x1f0 or 0x170, control must be base + 0x206, IRQ must be 14 or 15 respectively, and DMA must be absent")]
    InvalidIdeResources {
        /// Controller with invalid resources.
        device: DeviceId,
    },
    /// Geometry must fit the ATA device's supported CHS range.
    #[error("drive {drive_index} has invalid ATA geometry: expected cylinders 1..=16383, heads 1..=16, sectors 1..=63 and 512-byte sectors")]
    InvalidAtaGeometry {
        /// Drive index in the configuration.
        drive_index: usize,
    },
    /// ATA device 1 cannot be mounted alone.
    #[error("drive {drive_index} on {controller:?} slave requires a master drive")]
    AtaMissingMaster {
        /// Slave drive index.
        drive_index: usize,
        /// Named ATA controller.
        controller: DeviceId,
    },
    /// The configured kind does not declare a word register at this port.
    #[error("device {device:?} has no 16-bit register at port {port:#x}")]
    UnsupportedWordPort {
        /// The device whose register contract rejects the word cycle.
        device: DeviceId,
        /// The unsupported cycle address.
        port: u16,
    },

    /// A word-capable ISA decode must begin at an even cycle address.
    #[error("16-bit port window for {device:?} starts at odd port {start:#x}")]
    UnalignedWordPortWindow {
        /// The device owning the window.
        device: DeviceId,
        /// The unaligned starting port.
        start: u16,
    },

    /// A present device requires a service no configured ROM provides.
    #[error("device {device:?} needs firmware service {service}; no ROM provides it")]
    NoProvider {
        /// Device that needs the service.
        device: DeviceId,
        /// Missing service.
        service: crate::FirmwareService,
    },
    /// IBM table types and QEMU user-defined types interpret the same bytes differently.
    #[error("firmware {firmware} cannot combine cmos-drive-types and cmos-qemu-layout")]
    ConflictingCmosCapabilities {
        /// Catalogue identity with incompatible conventions.
        firmware: &'static str,
    },
    /// The IBM AT table has no type for this geometry; type 47 is reserved.
    #[error("drive {drive_index} geometry {c}/{h}/{s} ({b}-byte sectors) has no IBM AT CMOS fixed-disk type; user-defined type 47 is not supported; use a fixed-disk option ROM", c = .geometry.cylinders, h = .geometry.heads, s = .geometry.sectors_per_track, b = .geometry.bytes_per_sector)]
    UnsupportedCmosDiskGeometry {
        /// Index in the drive list.
        drive_index: usize,
        /// Geometry that cannot be represented by the documented table.
        geometry: crate::HardDiskGeometry,
    },
    /// A RAM-loaded blob cannot declare firmware services.
    #[error("RAM load image {image} cannot provide firmware services")]
    ServicesOnRamLoad {
        /// Index in the machine image list.
        image: usize,
    },
    /// A calendar seed is outside the modelled calendar range.
    #[error("device {device:?} has invalid RTC start {start}: invalid {field}; expected {range}", range = rtc_start_range(.start, .field))]
    InvalidRtcStart {
        /// RTC device with an invalid start.
        device: DeviceId,
        /// Rejected calendar value.
        start: crate::RtcStart,
        /// First invalid calendar or time field.
        field: &'static str,
    },
    /// A device cannot depend on itself for access synchronization.
    #[error("device {device:?} cannot declare itself as an access dependent")]
    SelfAccessCoupling {
        /// The device naming itself.
        device: DeviceId,
    },

    /// An access dependent must be present in the device set.
    #[error("device {device:?} declares absent access dependent {dependent:?}")]
    AccessDependentMissing {
        /// The accessed device.
        device: DeviceId,
        /// The absent dependent.
        dependent: DeviceId,
    },

    /// All eight UART registers must fit in the 16-bit I/O space.
    #[error("uart {device:?} base {base:#x} leaves no room for eight ports")]
    UartPortWindowOutOfSpace {
        /// The configured UART.
        device: DeviceId,
        /// The first register's port.
        base: u16,
    },

    /// The requested role is absent or does not expose the requested capability.
    #[error("unsupported observation output: {device:?} {output:?}")]
    UnsupportedObservationOutput {
        /// Requested device role.
        device: DeviceId,
        /// Missing or mismatched capability.
        output: crate::OutputKind,
    },

    /// Multiple collectors would compete for the same output channel.
    #[error("duplicate observation route: {device:?} {output:?}")]
    DuplicateObservationRoute {
        /// Requested device role.
        device: DeviceId,
        /// Duplicated channel.
        output: crate::OutputKind,
    },

    /// A file source could not be materialised before validation of its bytes.
    #[error("cannot read image file {path}: {message}")]
    ImageRead {
        /// The configured backing file.
        path: std::path::PathBuf,
        /// The underlying I/O error message.
        message: String,
    },

    /// Empty MMIO windows cannot form an inclusive bus decode range.
    #[error("mmio window for {device:?} at {base:#x} must have a nonzero size")]
    EmptyMmioWindow {
        /// The target device.
        device: DeviceId,
        /// The empty window's base.
        base: u64,
    },

    /// The cascaded AT controller exposes only IRQ0 through IRQ15.
    #[error("device {device:?} uses irq line {line}; the model supports irq lines 0 through 15")]
    InvalidIrqLine {
        /// The interrupt source.
        device: DeviceId,
        /// The unsupported PIC input.
        line: u8,
    },

    /// Each face of the shared PIC pair may be exposed only once.
    #[error("duplicate pic face (cascade={cascade}) on {a:?} and {b:?}")]
    DuplicatePicFace {
        /// One device exposing the face.
        a: DeviceId,
        /// The other device exposing the face.
        b: DeviceId,
        /// Whether the duplicated face is the master.
        cascade: bool,
    },

    /// The floppy device's sector size is fixed, rather than configurable.
    #[error("drive {drive_index} uses {bytes_per_sector}-byte sectors; the floppy model supports 512-byte sectors only")]
    UnsupportedFloppySectorSize {
        /// Index in the drive list.
        drive_index: usize,
        /// The requested sector size.
        bytes_per_sector: u32,
    },

    /// A loaded image would occupy a trapped device window.
    #[error("image {image_index} overlaps mmio for {device:?} at {addr:#x}")]
    ImageOverlapsMmio {
        /// Index in the image list.
        image_index: usize,
        /// The device owning the window.
        device: DeviceId,
        /// A shared address.
        addr: u64,
    },

    /// Loading order must not decide which image's bytes survive.
    #[error("images {a} and {b} overlap at {addr:#x}")]
    ImageOverlap {
        /// One index in the image list.
        a: usize,
        /// The other index in the image list.
        b: usize,
        /// A shared address.
        addr: u64,
    },

    /// A RAM load must fit entirely in the declared RAM regions.
    #[error("ram-load image {image_index} covers unmapped ram at {addr:#x}")]
    ImageOutsideRam {
        /// Index in the image list.
        image_index: usize,
        /// The first address without RAM backing.
        addr: u64,
    },

    /// The backend cannot provide exact instruction/REP-iteration step bounds.
    #[error("machine requires a Real16 CPU with step-bounded execution: {0}")]
    StepBoundedCpuRequired(String),

    /// A rational CPU rate must have a positive numerator and denominator.
    #[error(
        "cpu rate must have a positive numerator and denominator (got {numerator}/{denominator})"
    )]
    InvalidCpuRate {
        /// Retired steps in the modelling ratio.
        numerator: u64,
        /// PIT input clocks in the modelling ratio.
        denominator: u64,
    },

    /// The same [`DeviceId`] appears on more than one device.
    #[error("duplicate device id {0:?}")]
    DuplicateDeviceId(DeviceId),

    /// Two devices decode an overlapping port range and neither marks the
    /// overlap as intentional.
    #[error("port overlap at {port:#06x} between {a:?} and {b:?}")]
    PortOverlap {
        /// One of the contending devices.
        a: DeviceId,
        /// The other contending device.
        b: DeviceId,
        /// A port both decode.
        port: u16,
    },

    /// A flagged overlap cannot give every shared port to a narrower window.
    #[error("port overlap at {port:#06x} between {a:?} [{a_start:#06x}..={a_end:#06x}] and {b:?} [{b_start:#06x}..={b_end:#06x}]: overlapping windows must share a start port and differ in width; split the wider window or move one", a_start = .a_window.start, a_end = .a_window.end, b_start = .b_window.start, b_end = .b_window.end)]
    PortOverlapDecodeOrder {
        /// One of the contending devices.
        a: DeviceId,
        /// The other contending device.
        b: DeviceId,
        /// The first device's overlapping window.
        a_window: crate::PortWindow,
        /// The second device's overlapping window.
        b_window: crate::PortWindow,
        /// The first port both windows decode.
        port: u16,
    },

    /// Two MMIO windows cover an overlapping address range.
    #[error("mmio overlap at {addr:#x} between {a:?} and {b:?}")]
    MmioOverlap {
        /// One of the contending windows' devices.
        a: DeviceId,
        /// The other contending window's device.
        b: DeviceId,
        /// An address both cover.
        addr: u64,
    },

    /// A trapped MMIO window overlaps a RAM region — the CPU core forbids a
    /// `Trap` region over `Ram`.
    #[error("mmio window for {device:?} at {addr:#x} overlaps a ram region")]
    MmioOverlapsRam {
        /// The MMIO window's device.
        device: DeviceId,
        /// An address the window and a RAM region share.
        addr: u64,
    },

    /// An MMIO window traps to a device that is not in the device set.
    #[error("mmio window traps to absent device {0:?}")]
    MmioDeviceMissing(DeviceId),

    /// A RAM region, MMIO window, or loaded image falls outside
    /// `[0, address_space_bytes)`.
    #[error("region [{base:#x}, {end:#x}) is outside the {space:#x}-byte address space")]
    RegionOutOfSpace {
        /// Region base address.
        base: u64,
        /// One past the region's last byte.
        end: u64,
        /// The configured address-space size.
        space: u64,
    },

    /// Two devices drive the same PIC input line.
    #[error("irq line {line} driven by both {a:?} and {b:?}")]
    IrqConflict {
        /// One of the contending devices.
        a: DeviceId,
        /// The other contending device.
        b: DeviceId,
        /// The shared PIC input line.
        line: u8,
    },

    /// A device's DMA wiring names a controller that is not in the device set.
    #[error("device {requester:?} wires dma through absent controller {controller:?}")]
    DmaControllerMissing {
        /// The device that requested the wiring.
        requester: DeviceId,
        /// The absent controller it named.
        controller: DeviceId,
    },

    /// A required motherboard role is absent.
    #[error("board {board:?} requires device {device:?}")]
    BoardDeviceMissing {
        /// Selected motherboard.
        board: crate::BoardModel,
        /// Required role.
        device: DeviceId,
    },

    /// Motherboard roles must precede cards in canonical bus order.
    #[error("board {board:?} requires device {device:?} at index {index}")]
    BoardDeviceOrder {
        /// Selected motherboard.
        board: crate::BoardModel,
        /// Role at the wrong position.
        device: DeviceId,
        /// Required bus index.
        index: usize,
    },

    /// A motherboard field differs from its standard configuration.
    #[error("board {board:?} device {device:?} field {field}: expected {expected}, got {actual}")]
    BoardDeviceWiring {
        /// Selected motherboard.
        board: crate::BoardModel,
        /// Role whose configuration differs.
        device: DeviceId,
        /// The first differing field.
        field: &'static str,
        /// Standard value of that field.
        expected: String,
        /// Configured value of that field.
        actual: String,
    },

    /// A card uses an interrupt input reserved by the motherboard.
    #[error("board {board:?} does not offer irq line {line} to card {device:?}")]
    BoardCardIrq {
        /// Selected motherboard.
        board: crate::BoardModel,
        /// Card requesting the input.
        device: DeviceId,
        /// Unavailable interrupt line.
        line: u8,
    },

    /// The motherboard has no modelled DMA channel for this card.
    #[error("board {board:?} does not offer dma channel {channel} to card {device:?}")]
    BoardCardDma {
        /// Selected motherboard.
        board: crate::BoardModel,
        /// Card requesting the channel.
        device: DeviceId,
        /// Unavailable DMA channel.
        channel: u8,
    },

    /// A card names a device other than the motherboard's 8237 for DMA.
    #[error("board {board:?} requires dma controller Dma for card {device:?}, got {controller:?}")]
    BoardCardDmaController {
        /// Selected motherboard.
        board: crate::BoardModel,
        /// Card requesting DMA.
        device: DeviceId,
        /// Incorrect controller role.
        controller: DeviceId,
    },

    /// A drive names an absent controller role.
    #[error("drive {drive_index} binds to absent controller {controller:?}")]
    DriveControllerMissing {
        /// Index in the drive list.
        drive_index: usize,
        /// The missing role.
        controller: DeviceId,
    },

    /// The named device cannot serve this medium kind.
    #[error("drive {drive_index} kind is incompatible with controller {controller:?}")]
    DriveControllerKind {
        /// Index in the drive list.
        drive_index: usize,
        /// The incompatible device.
        controller: DeviceId,
    },

    /// The named controller does not offer this unit.
    #[error("drive {drive_index} binds to controller {controller:?} unit {unit} but only {available} unit(s) exist")]
    DriveUnitUnavailable {
        /// Index in the drive list.
        drive_index: usize,
        /// The named controller.
        controller: DeviceId,
        /// Requested unit.
        unit: u8,
        /// Units supported by both config and hardware.
        available: u8,
    },

    /// Two media cannot occupy the same controller unit.
    #[error("drives {a} and {b} bind to controller {controller:?} unit {unit}")]
    DuplicateDriveUnit {
        /// Earlier drive index.
        a: usize,
        /// Later drive index.
        b: usize,
        /// Shared controller.
        controller: DeviceId,
        /// Shared unit.
        unit: u8,
    },

    /// A drive's backing image does not fit it: a fixed disk needs its
    /// geometry's size, a floppy drive one of the media its type takes.
    #[error("drive {drive_index} image is {actual} bytes but {needs}", needs = image_needs(.expected, .drive_type))]
    ImageGeometryMismatch {
        /// Index of the drive in the config's drive list.
        drive_index: usize,
        /// The size the drive's own geometry requires.
        expected: u64,
        /// The size the backing image actually has.
        actual: u64,
        /// The standard floppy drive type, whose accepted media the message
        /// names; `None` for a fixed disk or a custom geometry.
        drive_type: Option<&'static str>,
    },
}

fn image_needs(expected: &u64, drive_type: &Option<&'static str>) -> String {
    drive_type
        .and_then(|name| {
            crate::descriptors::FLOPPY_TYPES
                .iter()
                .find(|kind| kind.name == name)
        })
        .map_or_else(
            || format!("its geometry needs {expected}"),
            |kind| crate::descriptors::floppy_drive_media(kind.geometry),
        )
}

fn rtc_start_range(start: &crate::RtcStart, field: &str) -> String {
    match field {
        "year" => "1980-2099".into(),
        "month" => "1-12".into(),
        "day" => format!(
            "1-{} for the selected month and year",
            start.days_in_month()
        ),
        "hour" => "0-23".into(),
        "minute" | "second" => "0-59".into(),
        _ => "a valid calendar date and time".into(),
    }
}

fn format_bios_size(size: &u64) -> String {
    if size.is_multiple_of(1024) {
        format!("{} KiB", size / 1024)
    } else {
        format!("{size} bytes")
    }
}

fn format_reset(reset: &crate::ResetSpec) -> String {
    let name = match reset {
        crate::ResetSpec::Entry { .. } => "Entry",
        crate::ResetSpec::ResetVector { .. } => "ResetVector",
    };
    format!("{name} {:04X}:{:04X}", reset.cs(), reset.ip())
}
