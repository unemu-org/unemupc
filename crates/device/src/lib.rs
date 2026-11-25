// SPDX-License-Identifier: AGPL-3.0-or-later
//! Device swap seam (§3.2, §7.1).
//!
//! [`Device`] is the fourth of the six swap seams from
//! `backlog/docs/architecture.md`: it is the contract every emulated piece of
//! hardware (16550 UART, 8259 PIC, 8253 PIT, 8042, …) implements. The `machine`
//! core and the `bus` never name a concrete device — they only see this trait.
//!
//! The contract carries **port** and **MMIO** access, [`Device::tick`] on the
//! virtual clock, an IRQ line query, [`Device::reset`], `save`/`restore`, and
//! optional bounded access tracing (see spec §7.1). Trace policy defaults to
//! [`TracePolicy::Off`] independently of display or byte-output observation;
//! snapshots contain hardware state, never trace histories or counters.

#![forbid(unsafe_code)]

pub mod clock;
#[cfg(feature = "host-profile")]
pub mod profile;

pub mod media;
pub use media::{InMemoryMedia, SectorMedia, SectorWrite};

pub mod trace;
pub use trace::{TraceBuffer, TracePolicy};

use std::cell::RefCell;
use std::rc::Rc;

use thiserror::Error;

/// The units and meaning of a device's semantic byte output.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ByteStreamKind {
    /// Accepted UART transmit-holding-register bytes, excluding divisor writes.
    SerialTx,
    /// Accepted POST writes, including repeated checkpoint codes.
    PostCheckpoints,
    /// Accepted emulator-only firmware diagnostic console bytes.
    DebugConsole,
}

/// A host receiver for semantic bytes, never a guest-access decoder.
///
/// Implementations must not block on I/O, call back into the machine, or alter
/// guest timing/interrupts. See `backlog/docs/design/observation.md`, §Sinks,
/// bounds and lifecycle. PC16550D datasheet, Transmitter Holding Register:
/// a byte represents accepted transmission, not a raw write to a port number.
pub trait ByteSink {
    /// Accept one emitted byte; capture failure must not affect guest execution.
    fn push(&mut self, byte: u8);
}

/// The allocation-free default receiver for unobserved semantic output.
#[derive(Default)]
pub struct DiscardByteSink;

impl ByteSink for DiscardByteSink {
    fn push(&mut self, _byte: u8) {}
}

/// Optional semantic producer implemented by UARTs and checkpoint ports.
///
/// Assembly binds one sink before execution. Device reset preserves the binding;
/// restore never re-emits historical bytes. See `backlog/docs/design/observation.md`,
/// §Sinks, bounds and lifecycle; PC16550D datasheet, Transmitter Holding Register.
pub trait ByteOutput {
    /// Describe the emitted bytes so routing can reject a mismatched request.
    fn kind(&self) -> ByteStreamKind;

    /// Install the route's sink. Binding itself emits no bytes.
    fn bind_sink(&mut self, sink: Box<dyn ByteSink>);
}

/// Capture exceeded its explicit byte limit in the current epoch.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Error)]
#[error("byte capture exceeded its {limit}-byte limit")]
pub struct ByteOverflow {
    /// Maximum number of bytes accepted during one capture epoch.
    pub limit: usize,
}

#[derive(Debug)]
struct CapturedBytes {
    bytes: Vec<u8>,
    limit: usize,
    accepted: usize,
    overflow: bool,
}

/// A bounded, single-threaded capture sink and its shared host handle.
///
/// At most `limit` bytes are accepted per epoch, including bytes already drained.
/// Draining releases buffered bytes without replenishing the budget, bounding
/// the concatenated result independently of host drain cadence. Further bytes
/// are discarded and overflow latches: every drain fails until
/// [`ByteCapture::start_epoch`], so a partial stream cannot pass a golden
/// assertion. Hardware continues unchanged.
/// Clones share one history; only the observation facade should drain it and
/// distribute the resulting batch. See `backlog/docs/design/observation.md`,
/// §Sinks, bounds and lifecycle.
#[derive(Clone, Debug)]
pub struct ByteCapture(Rc<RefCell<CapturedBytes>>);

impl ByteCapture {
    /// Start an empty capture with an explicit per-epoch byte bound (zero allowed).
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self(Rc::new(RefCell::new(CapturedBytes {
            bytes: Vec::new(),
            limit,
            accepted: 0,
            overflow: false,
        })))
    }

    /// Drain a complete batch, or report latched overflow without yielding a
    /// misleading partial capture. Draining does not clear an overflow.
    pub fn drain(&self) -> Result<Vec<u8>, ByteOverflow> {
        let mut capture = self.0.borrow_mut();
        if capture.overflow {
            return Err(ByteOverflow {
                limit: capture.limit,
            });
        }
        Ok(std::mem::take(&mut capture.bytes))
    }

    /// Explicitly discard prior capture data and clear overflow for a fresh run.
    pub fn start_epoch(&self) {
        let mut capture = self.0.borrow_mut();
        capture.bytes.clear();
        capture.accepted = 0;
        capture.overflow = false;
    }

    /// Number of retained bytes, always at most the configured limit.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.0.borrow().bytes.len()
    }
}

impl ByteSink for ByteCapture {
    fn push(&mut self, byte: u8) {
        let mut capture = self.0.borrow_mut();
        if capture.overflow || capture.accepted == capture.limit {
            capture.overflow = true;
        } else {
            capture.bytes.push(byte);
            capture.accepted += 1;
        }
    }
}

/// Virtual time base (see spec §6.1).
///
/// A monotonic counter shared by every device's [`Device::tick`], measured in
/// virtual ticks (currently one tick per PIT input clock). The machine advances
/// it from the CPU's retired-instruction delta by a configured rational rate (§6, virtual-clock model), never from wall
/// time — so device timing and every golden are reproducible.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VClock(pub u64);

impl VClock {
    /// The zero instant (machine reset).
    pub const ZERO: VClock = VClock(0);

    /// Convert a duration in PIT input clocks to virtual ticks.
    #[must_use]
    pub const fn from_pit(clocks: u64) -> Self {
        Self(clock::PIT.ticks(clocks))
    }

    /// Advance the clock by `ticks` and return the new value.
    #[must_use]
    pub fn advance(self, ticks: u64) -> VClock {
        VClock(self.0.wrapping_add(ticks))
    }
}

/// A device's next observable autonomous transition in virtual ticks.
///
/// Follows the accepted "Deterministic device scheduler with instruction-exact
/// interrupt boundaries" decision; see `backlog/docs/design/scheduler.md`,
/// §Device and bus contracts and §Review outcome. Reporting deadlines is
/// additive: devices keep the per-clock contract until they opt in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NextEvent {
    /// Tick at the next absolute PIT-clock boundary, preserving legacy calls.
    EveryPitClock,
    /// The earliest transition the machine must observe, or a conservative
    /// earlier deadline. After due work is settled this must be later than the
    /// query's `now`; querying must not consume or mutate the event.
    At(VClock),
    /// No autonomous observable transition until access, reset, restore, or
    /// external/coupled input changes state. Counters may still advance lazily.
    Dormant,
}

/// Direction of a data transfer, shared by port and MMIO access paths.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AccessDir {
    /// Guest reads from the device.
    Read,
    /// Guest writes to the device.
    Write,
}

/// A single observed device event, drained by [`Device::take_trace`] (§3.2
/// tracing seam, §7.1).
///
/// Tracing is a **first-class seam**: golden trace tests read these events from
/// the `Device` contract, never from ad-hoc `println!`s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceEvent {
    /// A serviced port-I/O access.
    Port {
        /// Virtual time at which the access was serviced.
        at: VClock,
        /// 16-bit port number.
        port: u16,
        /// Access width in bytes (1/2/4).
        size: u8,
        /// Read or write.
        dir: AccessDir,
        /// Value written by the guest, or read back to it (low `size` bytes).
        value: u32,
    },
    /// A serviced MMIO access.
    Mmio {
        /// Virtual time at which the access was serviced.
        at: VClock,
        /// Guest physical address.
        addr: u64,
        /// Access width in bytes.
        size: u8,
        /// Read or write.
        dir: AccessDir,
        /// Value written by the guest, or read back to it.
        value: u64,
    },
}

/// Errors a device may raise from the [`Device`] contract (§7.1).
///
/// Stable, `thiserror`-based library error surface: bus faults and device
/// state-restore failures. Library code never `unwrap`s — it returns one of
/// these.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DeviceError {
    /// An access hit a register the device does not decode.
    #[error("unmapped device register: {kind} access at {offset:#x}")]
    UnmappedRegister {
        /// Human-readable access kind ("port"/"mmio").
        kind: &'static str,
        /// Offset within the device's window.
        offset: u64,
    },
    /// [`Device::restore`] was handed a snapshot it cannot interpret.
    #[error("invalid snapshot for device {device}: {reason}")]
    InvalidSnapshot {
        /// The device's [`Device::name`].
        device: &'static str,
        /// Why the blob was rejected.
        reason: &'static str,
    },
}

/// A guest-memory handle for DMA transfers (§7.7).
///
/// A DMA controller moves bytes between an I/O device and guest RAM, but no
/// device may hold a reference into that RAM: writes must run through the path
/// that fires the JIT's `note_write` hook, or self-modifying-code tracking stops
/// invalidating translated blocks the transfer overwrites (see the memory model
/// in `backlog/docs/architecture.md`, §Contracts and ownership). The machine implements this trait
/// over the CPU's `Vm` API and hands it to the controller per transfer; unit
/// tests implement it over a plain buffer. Addresses are full 24-bit physical
/// addresses (a channel's page latch combined with its 16-bit offset).
pub trait DmaMemory {
    /// Read one byte of guest memory at physical `addr`.
    fn dma_read(&self, addr: u32) -> u8;

    /// Write one byte of guest memory at physical `addr` (must run `note_write`).
    fn dma_write(&mut self, addr: u32, value: u8);
}

/// Which way a queued DMA transfer moves bytes across the device↔memory seam
/// (§7.7).
///
/// The requester picks the direction; neither variant names a memory address —
/// address authority lives in the DMA controller's channel registers, which the
/// guest programmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DmaOp {
    /// Device→memory: the requester produced these bytes (a floppy read pulls
    /// them from its backing image) and the controller writes them to guest RAM.
    ToMemory(Vec<u8>),
    /// Memory→device: the controller reads `len` bytes from guest RAM and hands
    /// them back to the requester (a floppy write drains them to its image). The
    /// requester names a **length**, never an address.
    FromMemory {
        /// How many bytes the requester wants read out of memory.
        len: usize,
    },
}

/// A DMA transfer a requesting device (the floppy controller) has queued for the
/// machine to run through the DMA controller, in either direction (§7.7).
///
/// The requester names the DMA channel and, via [`DmaOp`], the direction and its
/// payload; it does **not** name the memory address — that lives in the DMA
/// controller's channel registers, which the guest programmed. The machine pulls
/// this with [`Device::take_dma_request`], runs it through the controller
/// ([`Device::dma_service_write`] for [`DmaOp::ToMemory`], [`Device::dma_service_read`]
/// for [`DmaOp::FromMemory`] — each stepping the channel via [`DmaMemory`]), then
/// reports the result back with [`Device::dma_complete`] so the requester can
/// finish its result phase and raise its completion interrupt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmaRequest {
    /// The DMA channel the transfer runs on (channel 2 for the PC floppy).
    pub channel: u8,
    /// The direction and payload of the transfer.
    pub op: DmaOp,
}

/// Bytes moved and the TC signal produced by one DMA service (§7.7).
///
/// Intel 8237A datasheet, Current Word Count Register and End of Process:
/// TC/EOP occurs when the word count underflows, including with autoinitialize.
/// Exhausting the request buffer alone does not assert TC.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DmaTransfer {
    /// Number of bytes moved during this service.
    pub transferred: usize,
    /// Whether the channel reached terminal count during this service.
    pub terminal_count: bool,
}

/// The result the machine hands a requester once its queued [`DmaRequest`] has
/// run through the DMA controller (§7.7).
///
/// The variant matches the request's [`DmaOp`]: a device→memory transfer reports
/// how many bytes reached memory, a memory→device transfer hands back the bytes
/// the controller read out (already truncated to what was actually transferred,
/// never padded with uninitialised tail bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DmaOutcome {
    /// The device→memory ([`DmaOp::ToMemory`]) transfer moved `transferred` bytes.
    ToMemory {
        /// Number of bytes that reached guest memory.
        transferred: usize,
        /// Whether the DMA channel asserted TC during this transfer.
        terminal_count: bool,
    },
    /// The memory→device ([`DmaOp::FromMemory`]) transfer produced these bytes,
    /// in order.
    FromMemory {
        /// Bytes read from guest memory, truncated to the completed transfer.
        bytes: Vec<u8>,
        /// Whether the DMA channel asserted TC during this transfer.
        terminal_count: bool,
    },
}

/// Attribute decoding selected by a text card (§9.2, §9.3).
///
/// IBM PC Technical Reference, Options and Adapters, IBM Monochrome Display
/// and Printer Adapter, Programming Considerations (attribute byte and 3B8);
/// IBM Color/Graphics Monitor Adapter, Programming Considerations, Mode Control
/// Register (bit 5, the colour card's blink enable); IBM VGA Technical
/// Reference, Alphanumeric Modes (the colour attribute byte).
/// See `backlog/docs/design/observation.md`, §Typed capabilities and route ownership.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextAttributeSet {
    /// RGB palette attributes, with bit 3 selecting foreground intensity.
    Colour {
        /// Whether attribute bit 7 requests blink instead of background intensity.
        blink_enabled: bool,
    },
    /// MDA display effects, with bit 3 selecting character intensity.
    Monochrome {
        /// Mode-control register 3B8 bit 5: selects whether attribute bit 7
        /// requests blink instead of background intensity.
        blink_enabled: bool,
    },
}

/// Borrowed text scanout from a card, without guest I/O (§9.2).
///
/// IBM VGA Technical Reference, Alphanumeric Modes and CRT Controller
/// Registers; see `backlog/docs/design/observation.md`, §Typed capabilities
/// and route ownership. Addresses remain in cells, as stored by the card.
///
/// Deliberately plain data (a memory slice + a handful of register-derived
/// scalars) so the renderer depends on no concrete device type — a VGA card, a
/// CGA card, or a test fixture all populate the same struct.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TextModeState<'a> {
    /// Text video memory: two bytes per cell (glyph, then attribute),
    /// little-endian within a 16-bit cell. Read starting at `start_cell`.
    pub vram: &'a [u8],
    /// Visible columns (80 for mode 3).
    pub cols: u16,
    /// Visible rows (25 for mode 3).
    pub rows: u16,
    /// Display start address, in cells (CRTC start-address registers). The
    /// top-left cell is at byte offset `start_cell * 2`.
    pub start_cell: u16,
    /// Absolute cursor location, in cells (CRTC cursor-location registers). Its
    /// grid position is measured relative to `start_cell`.
    pub cursor_cell: u16,
    /// Whether the card is displaying the cursor at all.
    pub cursor_visible: bool,
    /// Meaning of the raw attribute bytes, including the blink selection.
    pub attributes: TextAttributeSet,
}

/// VGA frame addressing after the retrace latch (IBM VGA/XGA Technical
/// Reference Manual, Preset Row Scan, Line Compare and Horizontal PEL Panning).
/// Physical scan lines keep split boundaries independent of line repetition.
#[derive(Clone, Copy, Debug)]
pub struct VgaScroll {
    /// Decoded left shift in logical pixels (or text dots).
    pub pixel_pan: u8,
    /// Byte panning: plane bytes after wiring in graphics, character-counter
    /// units before wiring in text (ASSUMED single-shift interpretation).
    pub byte_pan: u8,
    /// Initial row scan; reset to zero below the split.
    pub preset_row: u8,
    /// First physical scan line below the comparison, target + 1.
    pub split_line: u16,
    /// Clear the decoded pixel displacement below the split.
    pub split_pan_reset: bool,
    /// Physical scan lines per row-scan count (1 or 2).
    pub double_scan: u8,
}

impl Default for VgaScroll {
    fn default() -> Self {
        Self {
            pixel_pan: 0,
            byte_pan: 0,
            preset_row: 0,
            split_line: 1024,
            split_pan_reset: false,
            double_scan: 1,
        }
    }
}

/// Passive planar alphanumeric scanout (IBM VGA/XGA Technical Reference Manual,
/// Alphanumeric Modes, Figs. 2-33–2-35 and 2-60–2-61).
/// Register decoding belongs to the card; glyph rasterisation belongs to
/// renderers. See `backlog/docs/design/observation.md`, §Typed capabilities and route ownership.
#[derive(Clone, Debug)]
pub struct PlanarTextState<'a> {
    /// Frame scrolling and split-screen addressing.
    pub scroll: VgaScroll,
    /// Visible physical scan lines, including partial character rows.
    pub scan_lines: u16,
    /// Character codes, attributes and font bytes, in separate planes.
    pub planes: [&'a [u8]; 3],
    /// Visible character columns and complete character rows.
    pub dimensions: (u16, u16),
    /// Display start and cursor in CRTC address-counter units.
    pub start: u16,
    /// Absolute cursor address.
    pub cursor: u16,
    /// Increment of the address counter between character rows.
    pub row_stride: u16,
    /// Left shift from the CRTC counter to a plane byte address.
    pub address_shift: u8,
    /// High address-counter bit wired to plane address bit zero in word mode.
    pub word_wrap_bit: u8,
    /// Font offsets for attribute bit 3 clear and set, respectively.
    pub font_offsets: [usize; 2],
    /// Whether bit 3 additionally selects a font; colour bits remain unchanged.
    pub dual_font: bool,
    /// Dot width (8 or 9) and character scan-line height (1–32).
    pub cell_size: (u8, u8),
    /// Horizontal repetitions per dot (1 or 2); IBM VGA/XGA TR, Fig. 2-31.
    pub horizontal_repeat: u8,
    /// Repeat dot eight in dot nine for character codes C0h–DFh.
    pub line_graphics: bool,
    /// Attribute bit 7 requests blink instead of background intensity.
    pub blink_enabled: bool,
    /// Cursor enable and inclusive start/end scan lines.
    pub cursor_shape: (bool, u8, u8),
    /// Inclusive underline scan line, or a value beyond the cell height.
    pub underline: u8,
    /// Resolved RGB palette, with six-bit DAC values expanded to eight bits.
    pub palette: [[u8; 3]; 16],
    /// False while the sequencer or attribute controller blanks the display.
    pub enabled: bool,
}

/// Passive VGA graphics scanout, decoded from registers without guest accesses.
/// IBM VGA/XGA Technical Reference Manual, Video Memory Organization and
/// Figs. 2-50, 2-65, 2-75, 2-84. See `backlog/docs/design/observation.md`.
#[derive(Clone, Debug)]
pub struct GraphicsScanout<'a> {
    /// Frame scrolling and split-screen addressing.
    pub scroll: VgaScroll,
    /// When present, render this scrolled text instead of the graphics fields;
    /// a cell grid cannot represent clipping within a glyph.
    pub text: Option<PlanarTextState<'a>>,
    /// Four independent 64 KiB memory planes.
    pub planes: [&'a [u8]; 4],
    /// Logical pixels, excluding repeated dots and scan lines.
    pub dimensions: (u16, u16),
    /// Dot and scan-line repetitions retained for presentation.
    pub repeat: (u8, u8),
    /// Scan lines per CRTC address-counter row (MSL + 1).
    pub row_scan_height: u8,
    /// Address bits replaced by row scan: MA13 (CMS0) and MA14 (SRC).
    pub row_scan_mask: u16,
    /// Start address in CRTC counter units.
    pub start: u16,
    /// Counter increment per address-counter row.
    pub row_stride: u16,
    /// Byte, word or doubleword counter wiring (0, 1 or 2).
    pub address_shift: u8,
    /// Counter bit routed to address bit zero in word mode.
    pub word_wrap_bit: u8,
    /// Graphics controller 256-colour serializer selection.
    pub packed: bool,
    /// Attribute-controller four-bit input to DAC address mapping.
    pub colours: [u8; 16],
    /// Eight-bit RGB DAC palette, already subject to the PEL mask.
    pub palette: [[u8; 3]; 256],
    /// False for screen-off or unsupported serializer combinations.
    pub enabled: bool,
}

/// The device swap seam (§3.2 seam 4, §7.1).
///
/// Every emulated chip implements this. Method contracts:
///
/// - [`Device::port_read`] / [`Device::port_write`]: ISA port-space access at a
///   16-bit `port`, `size` bytes wide (1/2/4). Backs `IN`/`OUT`.
/// - [`Device::mmio_read`] / [`Device::mmio_write`]: memory-mapped access at a
///   guest physical `addr`, `size` bytes. Backs `RegionKind::Trap` loads/stores.
/// - [`Device::tick`]: advance internal timers/state to virtual time `now`
///   (§6). Deadline dispatch visits due transitions; accesses synchronize lazy state.
/// - [`Device::irq_pending`]: the device's IRQ level (§5). The machine forwards
///   both transitions to the interrupt controller, which detects edges and gates
///   requests by the current level (Intel 8259A datasheet, Edge and Level Triggered
///   Modes). The machine injects the acknowledged vector into the CPU.
/// - [`Device::irq_output`]: query a device-local interrupt pin (§5.1). Pin 0
///   defaults to `irq_pending`; additional pins default low. Board wiring maps
///   pins to interrupt-controller input lines.
/// - [`Device::reset`]: return to power-on register state.
/// - [`Device::save`] / [`Device::restore`]: snapshot/replay (§7).
/// - [`Device::take_trace`]: drain observed events for golden tests.
/// - [`Device::text_scanout`] / [`Device::graphics_scanout`] /
///   [`Device::byte_output`]: optional, side-effect-free
///   observation capabilities (the screen a card displays; the bytes a device
///   emits as output), routed by the assembly crate. Default: unsupported.
///
/// Before each port or MMIO access, the bus calls [`Device::sync`] at its current
/// whole virtual tick for devices that opt out of [`NextEvent::EveryPitClock`].
/// Synchronization catches counters up and stamps traces without consuming an
/// access or changing byte/latch phases. It must be idempotent at the same time.
/// Legacy devices receive no extra ticks. Accesses and mutable device borrows
/// invalidate the bus's cached deadline; the next query sees the resulting state.
/// See `backlog/docs/design/scheduler.md`, §Access synchronization and invalidation.
pub trait Device {
    /// Passive graphics capability, selected only while the card is in graphics
    /// mode. Observation must not update latches, traces or virtual time.
    fn graphics_scanout(&self) -> Option<GraphicsScanout<'_>> {
        None
    }

    /// Passive planar text capability; unsupported by default. No guest access,
    /// latch update, trace or virtual-time advance is permitted during observation.
    fn planar_text_scanout(&self) -> Option<PlanarTextState<'_>> {
        None
    }

    /// Borrow current text memory and register-derived scanout together.
    ///
    /// This is a passive host query: no guest accesses, synchronization, trace
    /// recording or drains. See `backlog/docs/design/observation.md`,
    /// §Typed capabilities and route ownership. Unsupported by default.
    fn text_scanout(&self) -> Option<TextModeState<'_>> {
        None
    }

    /// Optional semantic byte producer, used by assembly to bind a sink once.
    /// Querying the capability must not change guest-visible state.
    fn byte_output(&mut self) -> Option<&mut dyn ByteOutput> {
        None
    }

    /// Stable, human-readable chip name (e.g. `"16550-uart"`). Used in traces
    /// and errors.
    fn name(&self) -> &'static str;

    /// Read `size` bytes from port `port`. Returns the value in the low bytes.
    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError>;

    /// Write the low `size` bytes of `value` to port `port`.
    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError>;

    /// Read `size` bytes from MMIO address `addr`.
    fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, DeviceError>;

    /// Write the low `size` bytes of `value` to MMIO address `addr`.
    fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError>;

    /// Advance internal state to virtual time `now` (§6).
    ///
    /// Times earlier than the device's current time violate the caller contract:
    /// debug builds assert, release builds ignore them. Restore the bus clock
    /// alongside device snapshots so counters cannot rewind or replay intervals.
    fn tick(&mut self, now: VClock);

    /// Synchronize a scheduled device before access at `now` (§6).
    ///
    /// The default delegates to `tick`; scheduled implementations must support
    /// arithmetic catch-up, including a zero elapsed interval. The bus never
    /// calls this adapter for an `EveryPitClock` device.
    fn sync(&mut self, now: VClock) {
        self.tick(now);
    }

    /// Report the next observable autonomous transition without changing state.
    ///
    /// The accepted scheduler contract (`backlog/docs/design/scheduler.md`,
    /// §Review outcome) keeps unmigrated devices on every PIT clock. A scheduled
    /// device may return a future deadline or declare itself dormant; the
    /// deadline engine consults this hint; the reference engine ticks every clock.
    fn next_event(&self, now: VClock) -> NextEvent {
        let _ = now;
        NextEvent::EveryPitClock
    }

    /// Whether the device is currently asserting its IRQ line (§5.1).
    ///
    /// This is a non-consuming pin-level query, not a pending-event queue.
    /// Intel 8259A datasheet, Edge and Level Triggered Modes: the integration
    /// forwards both transitions; the controller owns request edge detection.
    fn irq_pending(&self) -> bool;

    /// Query an interrupt output by its device-local pin index (§5.1).
    /// Output 0 is the existing primary line; other outputs default low.
    /// IBM PS/2 Hardware Interface Technical Reference, Common Interfaces,
    /// "Keyboard and Auxiliary Device Controller", Output Port, defines the
    /// 8042's separate keyboard and auxiliary outputs. The board maps pins to
    /// IRQ numbers; this query neither wires nor samples the PIC.
    fn irq_output(&self, output: usize) -> bool {
        output == 0 && self.irq_pending()
    }

    /// Return to power-on register state.
    fn reset(&mut self);

    /// Serialize device state for snapshot/replay (§7).
    ///
    /// The caller must first synchronize the device to the current machine
    /// clock with [`Device::sync`] (or settle all scheduled devices on the bus).
    /// Devices advance lazily, so saving without synchronization can retain
    /// stale counters or deadlines. Restore the corresponding global clock
    /// alongside the device: snapshots retain their absolute time base.
    fn save(&self) -> Vec<u8>;

    /// Restore device state from a [`Device::save`] blob.
    ///
    /// Reject malformed or inconsistent input with [`DeviceError::InvalidSnapshot`]
    /// without changing the device. The blob must never cause a panic or an
    /// unbounded transition when the restored device resumes execution.
    /// The caller must restore a bus clock at least as late as the snapshot's
    /// absolute time before resuming accesses. Earlier sync/tick calls are ignored.
    /// Keep the current trace history, drop count and recording policy: these
    /// run-scoped diagnostics are excluded from hardware snapshots.
    fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError>;

    /// A queued DMA transfer, if this device (a bus master such as the floppy
    /// controller) has one pending (§7.7).
    ///
    /// The machine drains this each boundary and, for a device wired to a DMA
    /// controller, runs the transfer through the controller in the direction the
    /// [`DmaRequest`]'s [`DmaOp`] names — [`Device::dma_service_write`] for
    /// device→memory, [`Device::dma_service_read`] for memory→device — before
    /// delivering the [`DmaOutcome`] back with [`Device::dma_complete`]. Default
    /// `None`: a device that never masters the bus has nothing to hand over.
    fn take_dma_request(&mut self) -> Option<DmaRequest> {
        None
    }

    /// Whether a DMA probe may return work after due transitions are dispatched.
    ///
    /// False skips the mutable borrow and its access synchronization. An opted-in
    /// requester must report every timed request through `next_event`, and this
    /// query must include requests queued by accesses and immediate completions.
    /// The conservative default keeps probing devices without a readiness hint.
    /// This query must neither consume requests nor advance device state (§7.7).
    fn dma_request_pending(&self) -> bool {
        true
    }

    /// Finish a DMA transfer the machine ran on this device's behalf (§7.7): the
    /// [`DmaOutcome`] reports how many bytes reached memory (device→memory) or
    /// carries the bytes read out of memory (memory→device). The requester
    /// advances its transfer state, fills its result phase, and raises its
    /// completion interrupt. Default no-op.
    fn dma_complete(&mut self, outcome: DmaOutcome) {
        let _ = outcome;
    }

    /// Act as the DMA controller for a device→memory transfer: write `data` to
    /// guest memory through `mem` on `channel`, advancing the channel's address
    /// and count, and return the byte count and TC signal (§7.7).
    ///
    /// Only a DMA controller (the 8237) overrides this; every other device
    /// inherits the no-op default that moves nothing.
    fn dma_service_write(
        &mut self,
        channel: u8,
        mem: &mut dyn DmaMemory,
        data: &[u8],
    ) -> DmaTransfer {
        let _ = (channel, mem, data);
        DmaTransfer::default()
    }

    /// Act as the DMA controller for a memory→device transfer: read bytes from
    /// guest memory through `mem` on `channel` into `out`, advancing the channel's
    /// address and count, and return the byte count and TC signal (§7.7). The
    /// transfer stops at terminal count, so fewer than `out.len()` bytes may be
    /// read; the machine truncates its buffer to the returned count before handing
    /// it to the requester.
    ///
    /// Only a DMA controller (the 8237) overrides this; every other device
    /// inherits the no-op default that moves nothing.
    fn dma_service_read(
        &mut self,
        channel: u8,
        mem: &mut dyn DmaMemory,
        out: &mut [u8],
    ) -> DmaTransfer {
        let _ = (channel, mem, out);
        DmaTransfer::default()
    }

    /// Select access recording independently of semantic output. Defaults to Off.
    /// Reconfiguration clears history and drop counts; hardware reset preserves
    /// the policy. Neither policy nor diagnostics belong in hardware snapshots.
    fn set_trace_policy(&mut self, _policy: TracePolicy) {}

    /// Events overwritten since configuration or hardware reset. Draining does
    /// not clear this saturating counter. Off records nothing and counts no drops.
    fn trace_dropped(&self) -> u64 {
        0
    }

    /// Drain and return the trace events observed since the last call (§3.2
    /// tracing seam), oldest retained first. Does not reset [`Device::trace_dropped`].
    /// Empty by default and whenever recording is Off.
    fn take_trace(&mut self) -> Vec<TraceEvent> {
        Vec::new()
    }

    /// Downcast escape hatch for host-side driving of a concrete device, when the
    /// machine core only holds `&mut dyn Device`. It exists for out-of-band traffic
    /// the bus cannot express — e.g. the frontend injecting keyboard scancodes
    /// into the 8042 mid-run, mirroring bytes arriving from a real keyboard, or
    /// draining the bytes a UART actually transmitted.
    ///
    /// Defaults to `None`: a device opts in by returning `Some(self)`. It is not a
    /// guest-visible seam and must never be used from the emulated execution path;
    /// only the embedding frontend reaches for it.
    fn as_any_mut(&mut self) -> Option<&mut dyn core::any::Any> {
        None
    }
}

/// The interrupt-controller seam between the device IRQ lines and the CPU (§5).
///
/// A real machine routes device IRQ lines through an 8259 PIC, which aggregates
/// them into a single INTR to the CPU and answers the CPU's INTA cycle with an
/// interrupt vector (§5.1, §5.4). This trait is that seam: devices (the PIT, the
/// keyboard controller, …) drive input lines with [`InterruptController::set_irq_line`],
/// and the CPU integration polls, acknowledges and end-of-interrupts through the
/// remaining methods.
///
/// The trait is deliberately minimal and lives here, next to [`Device`], so both
/// the concrete controller device and the `chipset` seam can name it without a
/// heavier dependency. The `machine`/`cpu` wiring — asking for a vector on INTR
/// and injecting it — is later integration work; this trait only fixes the shape
/// that wiring will call.
pub trait InterruptController {
    /// Drive input IRQ line `irq` to `level` (`true` = asserted).
    ///
    /// This is how devices raise and lower their request lines. Whether a rising
    /// level actually latches a request depends on the controller's trigger mode.
    /// The mask controls serviceability, not the electrical input. Forward falls
    /// too: Intel 8259A datasheet, Edge and Level Triggered Modes requires IR to
    /// remain high until INTA, even for an edge-triggered request.
    fn set_irq_line(&mut self, irq: u8, level: bool);

    /// The vector the controller would dispatch on INTA right now, if any: the
    /// highest-priority pending, unmasked, not-already-in-service request
    /// resolved to its vector (§5.4). Does not change controller state.
    fn poll(&self) -> Option<u8>;

    /// Perform an INTA (interrupt-acknowledge) cycle: latch the highest-priority
    /// serviceable request into the in-service state and return its vector, or
    /// `None` if the controller supplies no vector (§5.4). The Intel 8259A
    /// returns its default IR7 vector when INTA finds no serviceable request,
    /// without setting ISR7 (datasheet, Interrupt Sequence and Edge and Level
    /// Triggered Modes). Callers normally poll first; an explicit INTA after
    /// a disappearing request can therefore return a spurious vector.
    fn acknowledge(&mut self) -> Option<u8>;

    /// End-of-interrupt: clear the highest-priority in-service request, allowing
    /// lower-priority requests to be serviced again (§5.4).
    fn end_of_interrupt(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_capture_limit_covers_the_whole_epoch_across_drains() {
        let capture = ByteCapture::new(2);
        let mut producer = capture.clone();
        producer.push(0);
        assert_eq!(capture.drain().unwrap(), [0]);
        producer.push(0xFF);
        assert_eq!(capture.drain().unwrap(), [0xFF]);
        producer.push(42);
        assert_eq!(capture.buffered_len(), 0);
        assert_eq!(capture.drain(), Err(ByteOverflow { limit: 2 }));
        assert_eq!(producer.drain(), Err(ByteOverflow { limit: 2 }));
        capture.start_epoch();
        producer.push(7);
        producer.push(8);
        assert_eq!(capture.drain().unwrap(), [7, 8]);
    }

    #[test]
    fn byte_capture_zero_limit_latches_overflow_until_new_epoch() {
        let mut capture = ByteCapture::new(0);
        assert_eq!(capture.drain(), Ok(Vec::new()));
        capture.push(0);
        assert_eq!(capture.buffered_len(), 0);
        assert_eq!(capture.drain(), Err(ByteOverflow { limit: 0 }));
        capture.start_epoch();
        assert_eq!(capture.drain(), Ok(Vec::new()));
    }

    #[test]
    fn byte_capture_drains_complete_batches_with_one_shared_history() {
        let capture = ByteCapture::new(3);
        let mut producer = capture.clone();
        producer.push(0);
        producer.push(0xFF);
        assert_eq!(capture.drain().unwrap(), [0, 0xFF]);
        assert_eq!(producer.buffered_len(), 0);
        producer.push(42);
        assert_eq!(capture.drain().unwrap(), [42]);
    }

    #[test]
    fn vclock_advances_monotonically() {
        let t0 = VClock::ZERO;
        let t1 = t0.advance(5);
        let t2 = t1.advance(3);
        assert!(t0 < t1 && t1 < t2);
        assert_eq!(t2, VClock(8));
    }
}
