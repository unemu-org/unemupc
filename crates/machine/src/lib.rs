// SPDX-License-Identifier: AGPL-3.0-or-later
//! Machine composition root (§3.1).
//!
//! [`Machine`] owns a [`Cpu`], a [`Bus`], and an [`InterruptController`] and runs
//! the fetch/dispatch/tick/interrupt loop. It is the composition root: **nothing
//! here names a concrete device, chipset, or CPU backend** — it sees only the
//! [`Cpu`], [`Bus`], and [`InterruptController`] seams (§3.2). A frontend (`cli`)
//! selects an assembly preset; assembly builds the concrete parts and hands them in.
//!
//! ## The loop (§3.1, scheduler design §Interrupt acceptance)
//!
//! Only Real16 step-bounded CPUs are assembled. CPU work stops at the next
//! device deadline, policy cap, or cooperative step limit. One completed
//! instruction is a step; each REP iteration is also a step (Intel 80286
//! Programmer's Reference Manual, String Instructions and §3.11.2).
//!
//! Port/MMIO accesses use instruction-start time. Read/write completion and
//! immediate DMA settle before PIC acknowledgement. INTA occurs only when the
//! CPU reports IF set, no STI/SS shadow, and no pending instruction transaction
//! (Intel 8259A datasheet, Interrupt Sequence; Intel 80286 PRM, §9.2).
//! An attached memory-map hook ([`Machine::with_memory_map`]) is applied after
//! each port write, before that settling, and flushed when a run returns.
//!
//! The production deadline engine visits every observable wired transition;
//! `reference-scheduler` exposes a per-clock test oracle with identical CPU/IRQ
//! semantics. Device IRQ levels reach PIC inputs in timestamp order; the PIC
//! detects edges (Intel 8259A datasheet, Edge and Level Triggered Modes).
//!
//! ## Virtual clock (§6)
//!
//! Completed steps convert to whole virtual ticks at a configured rational
//! rate, with carried remainder. While HLT or coprocessor BUSY suspends execution,
//! time jumps to the
//! next event, idle bound, or cap, without retiring instructions or consuming
//! the running remainder. IF-clear suspension stops immediately because NMI is absent.
//! Virtual-time caps include accumulated halted instruction equivalents; active
//! caps are exact and halted caps round up to a whole virtual tick. No wall time
//! enters execution. See `backlog/docs/design/scheduler.md` §Adoption.

#![forbid(unsafe_code)]

mod code_trail;
pub mod delivery_tap;

pub use code_trail::{CodeTrail, TrailEntry, CODE_TRAIL_LEN};
pub use delivery_tap::{
    DeliveryEntry, DeliveryRecording, DeliveryRing, RecentDeliveries, RECENT_DELIVERIES,
    RECORDED_ENTRIES,
};

use device::clock::{PIT, TICKS_PER_PIT_CLOCK};
use std::num::NonZeroU64;

use bus::{Bus, BusError};
use cpu::{Cpu, CpuError, CpuStepStop, CpuStop};
use device::{DmaMemory, DmaOp, DmaOutcome, InterruptController, NextEvent, VClock};
use thiserror::Error;

/// Default completed guest steps per PIT input clock (§6).
///
/// The default CPU rate is `1/1`: one PIT clock per instruction or REP iteration.
/// This is a modelling approximation, not the real ~1.193182 MHz OSC/12
/// timebase. Non-default rates are set with [`Machine::with_cpu_rate`].
pub const RETIRED_PER_PIT_CLOCK: u64 = 1;

/// Device dispatch policy under the same CPU/IRQ loop (§6).
///
/// Production uses deadlines. The non-default `reference-scheduler` feature
/// exposes a per-clock oracle for integration tests, with the same CPU/IRQ rules.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SchedulerEngine {
    /// Tick every device at every whole PIT clock (the reference behavior).
    #[cfg(any(test, feature = "reference-scheduler"))]
    PerClock,
    /// Tick unmigrated devices every clock and scheduled devices only when due.
    #[default]
    Deadline,
}

/// One wired connection from a device IRQ line to a PIC input (§5.1).
///
/// Declares *which* device (by its index in the configured device set)
/// drives *which* PIC input line. The machine forwards the device's level;
/// the PIC detects edges (Intel 8259A datasheet, Edge and Level Triggered Modes).
#[derive(Copy, Clone, Debug)]
pub struct IrqLine {
    /// Index of the source device in the bus's device set.
    pub device: usize,
    /// Device-local interrupt output pin; zero is the primary output.
    pub output: usize,
    /// The PIC input line this device's OUT is wired to (e.g. `0` for the PIT).
    pub irq: u8,
}

/// One wired coupling between a bus-master device and a DMA controller (§7.7).
///
/// A device such as the floppy controller cannot reach guest memory itself; it
/// queues a [`device::DmaRequest`] and relies on a DMA controller (the 8237) to
/// move the bytes. This declares *which* device masters the bus (`requester`) and
/// *which* controller runs its transfers (`controller`), both by their index in
/// the bus's device set — the machine never names either concrete type, exactly
/// as [`IrqLine`] wires interrupts by index.
#[derive(Copy, Clone, Debug)]
pub struct DmaWiring {
    /// Index of the bus-master device (e.g. the floppy controller).
    pub requester: usize,
    /// Index of the DMA controller that services the requester's transfers.
    pub controller: usize,
}

/// One system-channel route for a requester with controller-local numbering.
///
/// IBM Personal Computer AT Technical Reference, System Board, Direct Memory
/// Access: the word controller serves system channels 5–7 on local channels 1–3.
/// Assembly supplies the translation; the machine treats both numbers as opaque.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DmaChannelWiring {
    /// Bus index of the device issuing requests.
    pub requester: usize,
    /// Channel number carried by the request, in system numbering.
    pub system_channel: u8,
    /// Bus index of the controller that services this channel.
    pub controller: usize,
    /// Channel number passed to that controller's service API.
    pub local_channel: u8,
}

/// Bridges DMA memory cycles to the CPU map (§4.3), keeping RAM writes on the
/// `note_write` path and queuing trapped bytes for bus dispatch. The controller
/// borrows the bus during a transfer, so MMIO is serviced after that borrow ends,
/// before the requester receives completion or any guest execution resumes.
struct CpuDmaMemory<'a, C: Cpu> {
    cpu: &'a mut C,
    read_offset: std::cell::Cell<usize>,
    mmio_reads: std::cell::RefCell<Vec<(usize, u32)>>,
    mmio_writes: Vec<(u32, u8)>,
}

impl<C: Cpu> DmaMemory for CpuDmaMemory<'_, C> {
    fn dma_read(&self, addr: u32) -> u8 {
        let offset = self.read_offset.get();
        self.read_offset.set(offset + 1);
        match self.cpu.guest_dma_read(addr) {
            cpu::DmaAccess::Complete(value) => value,
            cpu::DmaAccess::Mmio => {
                self.mmio_reads.borrow_mut().push((offset, addr));
                0xFF
            }
        }
    }

    fn dma_write(&mut self, addr: u32, value: u8) {
        if self.cpu.guest_dma_write(addr, value) == cpu::DmaAccess::Mmio {
            self.mmio_writes.push((addr, value));
        }
    }
}

/// When the machine calls an attached memory-map hook (§3.1, §4.2).
///
/// A board may let the CPU hold device memory as ordinary RAM, for example an
/// EMS page frame whose mapped pages must stay executable. The machine never
/// learns which device that is; it only calls the hook at these points.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MemoryMapSync {
    /// A run is starting or a guest port write has completed. Apply any
    /// mapping change before the next guest instruction or DMA cycle.
    Apply,
    /// Control is returning to the host. Copy CPU-held memory back to its
    /// device, so device snapshots and host bus reads see current contents.
    Flush,
}

type MemoryMapHook<C> = Box<dyn FnMut(&mut C, MemoryMapSync) -> Result<(), CpuError>>;

/// Errors from running a [`Machine`] (§3.1).
#[derive(Debug, Error)]
pub enum MachineError {
    /// The CPU seam raised an error.
    #[error("cpu: {0}")]
    Cpu(#[from] CpuError),
    /// The bus seam raised an error (undecoded access or device fault).
    #[error("bus: {0}")]
    Bus(#[from] BusError),
    /// The guest raised a CPU exception the machine does not handle. In real
    /// mode most vectors deliver in-guest through the IVT; this surfaces only an
    /// exception the CPU core chose to escape.
    #[error("unhandled guest exception, vector {vector:#x}")]
    Exception {
        /// x86 exception vector.
        vector: u8,
    },
}

fn scheduler_error(what: &str) -> MachineError {
    CpuError::UnexpectedExit {
        what: format!("scheduler: {what}"),
    }
    .into()
}

/// Why [`Machine::run_bounded`] returned control to the caller (§3.1).
///
/// The natural stop is [`RunOutcome::Halted`] — the guest halted and the
/// interrupt source fell quiescent, or there is nothing left to make progress.
/// [`RunOutcome::RetiredCapReached`] means a total virtual-time cap was
/// hit first: a safety net so a guest that spins or repeatedly wakes from HLT
/// cannot run forever. Reaching the cap is a clean stop,
/// not an error, so any output captured up to that point is preserved.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// The guest halted with no more interrupts to service (the natural end).
    Halted,
    /// The total virtual-time cap was reached before a natural stop.
    RetiredCapReached,
    /// A board signal or processor shutdown requested a CPU reset.
    /// The next run resets the CPU unless [`Machine::reset_cpu`] already did so.
    ResetRequested,
}

/// A running machine: a CPU seam, a bus seam, and an interrupt controller (§3.1).
pub struct Machine<C: Cpu> {
    cpu: C,
    bus: Bus,
    pic: Box<dyn InterruptController>,
    clock: VClock,
    engine: SchedulerEngine,
    completed_steps: u64,
    step_limit: NonZeroU64,
    /// HLT or coprocessor WAIT stays suspended across runs until an IRQ can wake it.
    halted: bool,
    /// Halted virtual ticks at the selected rate, charged to the policy cap.
    halted_clocks: u64,
    /// Keep each rate's ticks so repeated switches cannot discard fractional credit.
    inactive_halted_clocks: u64,
    /// Consecutive halted virtual ticks since the CPU last ran or woke.
    idle_clocks: u64,
    /// Device IRQ lines wired to PIC inputs, sampled at scheduler boundaries.
    irq_lines: Vec<IrqLine>,
    /// Last observed level of each wired line (parallel to `irq_lines`), for
    /// change detection. `None` until the first sample (no spurious edge at
    /// reset).
    last_levels: Vec<Option<bool>>,
    /// CPU rate A/B in completed steps per PIT input clock.
    cpu_rate_numerator: NonZeroU64,
    cpu_rate_denominator: NonZeroU64,
    base_cpu_rate: (NonZeroU64, NonZeroU64),
    turbo_cpu_rate: Option<(NonZeroU64, NonZeroU64)>,
    turbo_enabled: bool,
    /// B times the number of virtual ticks per PIT clock.
    cpu_tick_denominator: u128,
    /// Fractional virtual ticks not yet emitted, in units of 1/A.
    clock_remainder: u64,
    /// Consecutive halted PIT clocks with no interrupt injected before stopping.
    /// `None` waits no clocks; only an immediately deliverable IRQ wakes HLT.
    idle_limit: Option<u64>,
    /// Bus-master → DMA-controller couplings drained each boundary (§7.7). Empty
    /// for a machine with no DMA (the serial and timer demos).
    dma_wirings: Vec<DmaWiring>,
    /// Per-requester system-channel routes that override the primary DMA coupling
    /// (§7.7), supplying controller-local numbers for matching requests.
    dma_channel_wirings: Vec<DmaChannelWiring>,
    reset_signal: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    reset_pending: bool,
    a20_line: Option<Box<dyn Fn() -> (u64, bool)>>,
    a20_changes: u64,
    numeric_error_assert: Option<Box<dyn Fn()>>,
    numeric_error_ack: Option<Box<dyn Fn() -> bool>>,
    /// Whether the CPU still fetches through the code base it loaded at
    /// reset; see [`Self::with_reset_fetch_probe`].
    reset_fetch_probe: Option<ResetFetchProbe<C>>,
    /// The board's gate is held open for the reset fetch until the first
    /// code-segment load.
    reset_fetch_window: bool,
    memory_map: Option<MemoryMapHook<C>>,
    /// CS:IP at the last run-slice boundaries, when a diagnostic asked for it.
    code_trail: Option<Box<CodeTrail>>,
    /// The CPU's delivery records, when a diagnostic asked for them.
    deliveries: Option<Box<delivery_tap::DeliveryTap>>,
    /// Reads the interrupt controllers' vector bases, to name an IRQ
    /// delivery's controller input from its vector.
    irq_bases_of: Option<IrqBasesOf>,
    /// The reset requests that came soon after the one before.
    reset_streak: ResetStreak,
}

/// Reset requests in a row, each fewer than [`cpu::RESET_STORM_STEPS`]
/// steps of virtual time after the one before; the start of the machine
/// counts as the first reset.
#[derive(Debug, Default)]
struct ResetStreak {
    /// Virtual time at the last request.
    last: u64,
    /// Virtual time at the request before the streak's first.
    start: u64,
    /// Requests in the streak.
    resets: u32,
}

impl ResetStreak {
    /// Count a request at virtual time `now`; the streak's length and the
    /// virtual time it spans once it reaches [`cpu::RESET_STORM_RESETS`].
    fn record(&mut self, now: u64) -> Option<(u32, u64)> {
        match now.checked_sub(self.last) {
            Some(since) if since < cpu::RESET_STORM_STEPS => {
                if self.resets == 0 {
                    self.start = self.last;
                }
                self.resets += 1;
            }
            _ => self.resets = 0,
        }
        self.last = now;
        (self.resets >= cpu::RESET_STORM_RESETS).then(|| (self.resets, now - self.start))
    }
}

/// Tells whether the CPU still fetches through its reset code base; see
/// [`Machine::with_reset_fetch_probe`].
pub type ResetFetchProbe<C> = Box<dyn Fn(&C) -> bool>;

/// Reads the vector bases of the master and the slave interrupt
/// controller from the bus, `None` for one that is absent.
pub type IrqBasesOf = Box<dyn Fn(&Bus) -> [Option<u8>; 2]>;

impl<C: Cpu> Machine<C> {
    /// Assemble a machine from a CPU backend, a bus, and an interrupt controller.
    ///
    /// `irq_lines` declares which device lines feed which PIC inputs (§5.1);
    /// empty for a machine with no interrupts. `idle_limit` bounds how long the
    /// machine keeps a halted guest alive waiting for an interrupt before
    /// declaring quiescence, in PIT clocks (`None` = no idle wait).
    pub fn new(
        cpu: C,
        bus: Bus,
        pic: Box<dyn InterruptController>,
        irq_lines: Vec<IrqLine>,
        idle_limit: Option<u64>,
    ) -> Self {
        let last_levels = vec![None; irq_lines.len()];
        Self {
            cpu,
            bus,
            pic,
            clock: VClock::ZERO,
            engine: SchedulerEngine::default(),
            completed_steps: 0,
            step_limit: NonZeroU64::new(65_536).unwrap_or(NonZeroU64::MIN),
            halted: false,
            halted_clocks: 0,
            inactive_halted_clocks: 0,
            idle_clocks: 0,
            irq_lines,
            last_levels,
            cpu_rate_numerator: NonZeroU64::MIN,
            cpu_rate_denominator: NonZeroU64::MIN,
            base_cpu_rate: (NonZeroU64::MIN, NonZeroU64::MIN),
            turbo_cpu_rate: None,
            turbo_enabled: false,
            cpu_tick_denominator: u128::from(TICKS_PER_PIT_CLOCK),
            clock_remainder: 0,
            idle_limit,
            dma_wirings: Vec::new(),
            dma_channel_wirings: Vec::new(),
            reset_signal: None,
            reset_pending: false,
            a20_line: None,
            a20_changes: 0,
            numeric_error_assert: None,
            numeric_error_ack: None,
            reset_fetch_probe: None,
            reset_fetch_window: false,
            memory_map: None,
            code_trail: None,
            deliveries: None,
            irq_bases_of: None,
            reset_streak: ResetStreak::default(),
        }
    }

    /// Wire a latched board reset signal without naming its source device.
    /// IBM Personal Computer AT Technical Reference, System Board, Keyboard
    /// Controller: System Reset resets the microprocessor, not peripheral state.
    /// See `backlog/docs/design/machine-config.md` §Board model. A pending
    /// signal reports one stop at a settled instruction boundary. CPU reset
    /// remains pending until [`Self::reset_cpu`] succeeds, explicitly or at the
    /// start of the next run. A host-side failure cannot lose the request.
    #[must_use]
    pub fn with_reset_signal(mut self, signal: std::rc::Rc<std::cell::Cell<bool>>) -> Self {
        self.reset_signal = Some(signal);
        self
    }

    /// Attach a board A20 line reader returning (transition count, enabled).
    ///
    /// IBM PC AT Technical Reference, Keyboard Controller, Output Port: A20 is
    /// a board signal retained across CPU-only reset. Sample once at assembly,
    /// then after port writes and before each run (including host-side device
    /// restore). The counter avoids redundant CPU calls. Without a line, the
    /// CPU keeps its constructor's flat addressing. See
    /// `backlog/docs/design/machine-config.md` §A20 line and System Control Port A.
    #[must_use]
    pub fn with_a20_line(mut self, line: impl Fn() -> (u64, bool) + 'static) -> Self {
        let (changes, enabled) = line();
        self.cpu.set_a20_enabled(enabled);
        self.a20_changes = changes;
        self.a20_line = Some(Box::new(line));
        self
    }

    /// Connect board numeric-error assertion and acknowledgement signals.
    /// IBM PC AT Technical Reference, NMI and Coprocessor Controls: the board
    /// owns IRQ13 and F0h; the CPU owns its wait interlock. See
    /// `backlog/docs/design/machine-config.md` §Coprocessor controls.
    pub fn with_numeric_error_lines(
        mut self,
        assert: impl Fn() + 'static,
        acknowledge: impl Fn() -> bool + 'static,
    ) -> Self {
        self.numeric_error_assert = Some(Box::new(assert));
        self.numeric_error_ack = Some(Box::new(acknowledge));
        self
    }

    fn sync_a20(&mut self) {
        if let Some(line) = &self.a20_line {
            let (changes, enabled) = line();
            if changes != self.a20_changes {
                self.cpu.set_a20_enabled(enabled || self.reset_fetch_window);
                self.a20_changes = changes;
            }
        }
    }

    /// Attach a probe that tells whether the CPU still fetches through the
    /// code-segment base it loaded at reset (FF0000h on the 80286).
    ///
    /// Intel 80286 Programmer's Reference Manual, Appendix C, Hardware
    /// Compatibility Considerations, "Address after Reset": A23–A20 are high
    /// for CS-relative bus cycles after reset until the first far JMP or far
    /// CALL. ASSUMED board behaviour (decision-42; machine-config.md §A20
    /// line and System Control Port A, "Reset fetch with the gate off"): a
    /// CPU reset with the board's A20 gate off still reaches the system ROM
    /// at FFFFF0h, so while the probe holds after a CPU reset the machine
    /// drives the CPU's A20 enabled; the first code-segment load ends that
    /// window and the board line applies again. The gate's sources (8042 output port, port 92h) never change,
    /// so the value the guest reads back is the one it wrote. Without a
    /// probe, or without an A20 line, resets keep the board line as it is.
    #[must_use]
    pub fn with_reset_fetch_probe(mut self, probe: impl Fn(&C) -> bool + 'static) -> Self {
        self.reset_fetch_probe = Some(Box::new(probe));
        self
    }

    /// Open the reset-fetch window after a CPU reset when the CPU fetches
    /// through its reset base, whatever the board line says: the line
    /// applies again when the window closes, so a gate the reset code
    /// changes before its first CS load takes effect from that load.
    fn open_reset_fetch_window(&mut self) {
        let (Some(_), Some(probe)) = (&self.a20_line, &self.reset_fetch_probe) else {
            return;
        };
        if probe(&self.cpu) {
            self.reset_fetch_window = true;
            self.cpu.set_a20_enabled(true);
        }
    }

    /// Whether the CPU's A20 is held open by the reset-fetch window
    /// ([`Self::with_reset_fetch_probe`]), for a host diagnostic.
    pub fn reset_fetch_window_open(&self) -> bool {
        self.reset_fetch_window
    }

    /// Close the reset-fetch window once the CPU has loaded a new code
    /// segment, giving the CPU the board line's current state.
    fn close_reset_fetch_window(&mut self) {
        if !self.reset_fetch_window
            || self
                .reset_fetch_probe
                .as_ref()
                .is_some_and(|probe| probe(&self.cpu))
        {
            return;
        }
        self.reset_fetch_window = false;
        self.apply_board_a20();
    }

    /// Give the CPU the board line's current state, changed or not.
    fn apply_board_a20(&mut self) {
        if let Some(line) = &self.a20_line {
            let (changes, enabled) = line();
            self.cpu.set_a20_enabled(enabled);
            self.a20_changes = changes;
        }
    }

    /// Attach a hook that keeps part of the CPU memory map in step with a
    /// device (§4.2).
    ///
    /// It receives [`MemoryMapSync::Apply`] when every run starts and right
    /// after every guest port write, before that instruction's DMA and the
    /// next instruction; a port write is the only guest action expected to
    /// change such a map, and host changes between runs are applied when the
    /// next run starts. It receives [`MemoryMapSync::Flush`] whenever
    /// [`Self::run_bounded`] returns, including with an error. A machine has
    /// one hook; attaching a second would silently drop the first.
    #[must_use]
    pub fn with_memory_map(
        mut self,
        hook: impl FnMut(&mut C, MemoryMapSync) -> Result<(), CpuError> + 'static,
    ) -> Self {
        debug_assert!(
            self.memory_map.is_none(),
            "a memory-map hook is already attached"
        );
        self.memory_map = Some(Box::new(hook));
        self
    }

    fn sync_memory_map(&mut self, event: MemoryMapSync) -> Result<(), MachineError> {
        if let Some(hook) = self.memory_map.as_mut() {
            hook(&mut self.cpu, event)?;
        }
        Ok(())
    }

    /// Configure the construction-time CPU rate A/B (§6).
    ///
    /// `numerator` is A and `denominator` is B, in completed steps per PIT
    /// input clock. Both must be positive; the default is `1/1`. This is a
    /// modelling ratio, not a measured CPU frequency. Configure it before the
    /// first run; changing rate after time has elapsed has no defined meaning.
    #[must_use]
    pub fn with_cpu_rate(mut self, numerator: NonZeroU64, denominator: NonZeroU64) -> Self {
        self.base_cpu_rate = (numerator, denominator);
        self.cpu_rate_numerator = numerator;
        self.cpu_rate_denominator = denominator;
        self.cpu_tick_denominator =
            u128::from(self.cpu_rate_denominator.get()) * u128::from(TICKS_PER_PIT_CLOCK);
        self
    }

    /// Configure the alternate turbo-button rate before the first run.
    /// Both button positions are throughput ratios; see
    /// `backlog/docs/design/machine-files.md` §Nominal CPU clocks.
    #[must_use]
    pub fn with_turbo_rate(mut self, numerator: NonZeroU64, denominator: NonZeroU64) -> Self {
        self.turbo_cpu_rate = Some((numerator, denominator));
        self
    }

    /// Switch button positions at a completed machine-call boundary.
    /// Returns false when enabling turbo without a configured alternate clock.
    /// Device time and accumulated policy time never move backwards. The
    /// sub-tick phase is rescaled down to the new denominator (loss < one
    /// virtual tick). Halted instruction-equivalent fractions survive every switch
    /// exactly. No-op switches preserve both fractions.
    pub fn set_turbo(&mut self, enabled: bool) -> bool {
        let rate = if enabled {
            let Some(rate) = self.turbo_cpu_rate else {
                return false;
            };
            rate
        } else {
            self.base_cpu_rate
        };
        if self.turbo_enabled == enabled {
            return true;
        }
        if rate != (self.cpu_rate_numerator, self.cpu_rate_denominator) {
            std::mem::swap(&mut self.halted_clocks, &mut self.inactive_halted_clocks);
            self.clock_remainder = (u128::from(self.clock_remainder) * u128::from(rate.0.get())
                / u128::from(self.cpu_rate_numerator.get()))
                as u64;
            self.cpu_rate_numerator = rate.0;
            self.cpu_rate_denominator = rate.1;
            self.cpu_tick_denominator = u128::from(rate.1.get()) * u128::from(TICKS_PER_PIT_CLOCK);
        }
        self.turbo_enabled = enabled;
        true
    }

    /// The currently selected turbo-button position.
    pub fn turbo_enabled(&self) -> bool {
        self.turbo_enabled
    }

    /// Select device timer dispatch before running the machine (§6).
    #[must_use]
    #[cfg(any(test, feature = "reference-scheduler"))]
    pub fn with_scheduler_engine(mut self, engine: SchedulerEngine) -> Self {
        self.engine = engine;
        self
    }

    /// Bound host cooperation in steps. By contract, smaller calls cannot move
    /// device or IRQ boundaries; device-free CPU regressions hold the core to it
    /// (scheduler design §Adoption).
    #[must_use]
    pub fn with_step_limit(mut self, limit: NonZeroU64) -> Self {
        self.step_limit = limit;
        self
    }

    /// The selected device timer dispatch policy.
    pub fn scheduler_engine(&self) -> SchedulerEngine {
        self.engine
    }

    /// Declare the bus-master → DMA-controller couplings this machine services
    /// (§7.7), returning the machine for chaining. A machine with no bus-master
    /// device leaves this empty (the default).
    #[must_use]
    pub fn with_dma_wirings(mut self, wirings: Vec<DmaWiring>) -> Self {
        self.dma_wirings = wirings;
        self
    }

    /// Override a requester's controller for a selected system DMA channel.
    /// IBM Personal Computer AT Technical Reference, System Board, Direct Memory
    /// Access: byte and word channels belong to distinct controllers (§7.7).
    /// Routes match both requester and system channel; other requests keep the
    /// controller and channel from the primary wiring.
    #[must_use]
    pub fn with_dma_channel_wirings(mut self, wirings: Vec<DmaChannelWiring>) -> Self {
        self.dma_channel_wirings = wirings;
        self
    }

    /// The CPU backend (for post-run inspection).
    pub fn cpu(&self) -> &C {
        &self.cpu
    }

    /// Assert the board's CPU reset line, retaining RAM, devices and virtual time.
    ///
    /// IBM Personal Computer AT Technical Reference, System Board, Keyboard
    /// Controller Output Port: bit 0 drives the CPU reset line. Shutdown uses
    /// the same CPU-only reset (System Board, Logic Diagrams). Media sessions
    /// survive (backlog/docs/design/machine-config.md §Board model).
    /// The 8042 output port and A20 gate keep their state; with a reset-fetch
    /// probe attached, a gate that is off is held open for the CPU until its
    /// first code-segment load ([`Self::with_reset_fetch_probe`]). A frontend may also
    /// assert it to restart a guest that halted with interrupts disabled. That
    /// is a CPU-only reset, unlike a hardware reset button, which also resets
    /// the board: POST reinitialises the board instead. A running guest
    /// reboots through Ctrl+Alt+Del.
    pub fn reset_cpu(&mut self) -> Result<(), MachineError> {
        if self.reset_fetch_window {
            // A reset inside the window: the board line applies until the
            // new window decision below.
            self.reset_fetch_window = false;
            self.apply_board_a20();
        }
        self.sync_a20();
        self.cpu.reset()?;
        self.open_reset_fetch_window();
        self.reset_pending = false;
        self.halted = false;
        self.idle_clocks = 0;
        Ok(())
    }

    /// Whether HLT or coprocessor BUSY suspends the CPU across runs.
    pub fn halted(&self) -> bool {
        self.halted
    }

    /// Start recording the CPU's CS:IP after every run slice, for a
    /// diagnostic report; see [`CodeTrail`]. The trail
    /// stays outside the CPU's execution path, so retired counts and every
    /// guest effect are unchanged. Calling it again keeps the trail.
    pub fn enable_code_trail(&mut self) {
        self.code_trail.get_or_insert_with(Box::default);
    }

    /// The code trail, when [`Self::enable_code_trail`] enabled it.
    pub fn code_trail(&self) -> Option<&CodeTrail> {
        self.code_trail.as_deref()
    }

    /// The bus (to drain device output / traces after a run).
    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    /// Mutable bus access.
    pub fn bus_mut(&mut self) -> &mut Bus {
        &mut self.bus
    }

    /// The current device clock, in virtual ticks (§6).
    pub fn clock(&self) -> VClock {
        self.clock
    }

    /// Whole inactive credit and its fractional part in current-rate units.
    /// Flooring only this conversion preserves all integer cap comparisons;
    /// the original ticks remain available for an exact conversion after a switch.
    fn inactive_halted_credit(&self) -> (u64, u128) {
        let (a, b) = if self.turbo_enabled {
            self.base_cpu_rate
        } else {
            self.turbo_cpu_rate.unwrap_or(self.base_cpu_rate)
        };
        let denominator = u128::from(b.get()) * u128::from(TICKS_PER_PIT_CLOCK);
        let numerator = u128::from(self.inactive_halted_clocks) * u128::from(a.get());
        (
            (numerator / denominator).min(u128::from(u64::MAX)) as u64,
            (numerator % denominator) * self.cpu_tick_denominator / denominator,
        )
    }

    /// Total virtual time in instruction-equivalent units (§6).
    ///
    /// This is the active count plus the floor of the sum of halted instruction
    /// equivalents at both rates, saturating at `u64::MAX`. Each rate contributes
    /// `halted_ticks * A / (B * ticks_per_PIT)`; fractional credits survive separate
    /// halts, runner calls and turbo switches. Halted clocks do not consume the running
    /// CPU's fractional virtual-tick remainder, measured in units of 1/A. Unlike
    /// [`Cpu::retired`], this advances during HLT too, so run caps and scheduled
    /// targets bound busy and idle time.
    /// Active time counts completed steps, including individual REP iterations.
    pub fn virtual_time(&self) -> u64 {
        let (inactive, credit) = self.inactive_halted_credit();
        let halted = (u128::from(self.halted_clocks) * u128::from(self.cpu_rate_numerator.get())
            + credit)
            / self.cpu_tick_denominator;
        let active = self.completed_steps;
        (u128::from(active) + u128::from(inactive) + halted).min(u128::from(u64::MAX)) as u64
    }

    /// The configured idle limit (halted PIT clocks before quiescence).
    pub fn idle_limit(&self) -> Option<u64> {
        self.idle_limit
    }

    /// Set the idle limit after construction — the "how we run" bound on how many
    /// consecutive halted PIT clocks pass with no injected interrupt before the
    /// machine stops (`None` = no idle wait). The count resets on each wake-up.
    ///
    /// The idle limit is a run concern, not part of what the machine *is*, so the
    /// run-policy layer above owns it and stamps it here before driving the loop.
    /// The assembler starts without an idle wait; the run policy applies it here.
    pub fn set_idle_limit(&mut self, idle_limit: Option<u64>) {
        self.idle_limit = idle_limit;
    }

    /// Run until the guest halts with no more interrupts to service (§3.1).
    ///
    /// Unbounded: equivalent to [`Machine::run_bounded`] with no cap. A reset
    /// request is reported once; the next run performs its pending CPU reset
    /// before executing from the reset vector.
    pub fn run(&mut self) -> Result<RunOutcome, MachineError> {
        self.run_bounded(None)
    }

    fn next_deadline(&mut self) -> Result<Option<VClock>, MachineError> {
        #[cfg(any(test, feature = "reference-scheduler"))]
        let event = if self.engine == SchedulerEngine::PerClock {
            if self.bus.device(0).is_none() {
                return Ok(None);
            }
            NextEvent::EveryPitClock
        } else {
            self.bus.next_event()
        };
        #[cfg(not(any(test, feature = "reference-scheduler")))]
        let event = self.bus.next_event();
        let at = match event {
            NextEvent::Dormant => return Ok(None),
            NextEvent::EveryPitClock => PIT
                .next_edge_after(self.clock)
                .ok_or_else(|| scheduler_error("clock exhausted"))?,
            NextEvent::At(at) => at,
        };
        if at <= self.clock {
            return Err(scheduler_error("device deadline did not advance"));
        }
        Ok(Some(at))
    }

    // Visit transitions at their own times so a short low pulse cannot vanish
    // inside a slow CPU step or a halted jump.
    fn advance_to(&mut self, target: VClock) -> Result<(), MachineError> {
        while self.clock < target {
            let at = self.next_deadline()?.map_or(target, |at| at.min(target));
            self.clock = at;
            match self.engine {
                #[cfg(any(test, feature = "reference-scheduler"))]
                SchedulerEngine::PerClock => self.bus.tick_all(at),
                SchedulerEngine::Deadline => {
                    self.bus.tick_due(at);
                }
            }
            self.sample_irq_lines();
            // A native audio deadline can fall inside one slow CPU step. Move
            // its DMA bytes before advancing time again, preserving sample times.
            self.settle(false)?;
        }
        Ok(())
    }

    fn charge_steps(&mut self, steps: u64) -> Result<(), MachineError> {
        self.completed_steps = self
            .completed_steps
            .checked_add(steps)
            .ok_or_else(|| scheduler_error("step counter exhausted"))?;
        let denominator = self.cpu_tick_denominator;
        let total = u128::from(steps)
            .checked_mul(denominator)
            .and_then(|ticks| ticks.checked_add(u128::from(self.clock_remainder)))
            .ok_or_else(|| scheduler_error("clock interval exhausted"))?;
        let rate = u128::from(self.cpu_rate_numerator.get());
        let clocks =
            u64::try_from(total / rate).map_err(|_| scheduler_error("clock interval exhausted"))?;
        self.clock_remainder = (total % rate) as u64;
        let target = self
            .clock
            .0
            .checked_add(clocks)
            .ok_or_else(|| scheduler_error("clock exhausted"))?;
        self.advance_to(VClock(target))
    }

    fn settle(&mut self, accessed: bool) -> Result<(), MachineError> {
        if accessed {
            // An access can change a coupled device as well as its own pins.
            // Ordinary instruction boundaries have no such side effects.
            self.bus.settle();
            self.sample_irq_lines();
        }
        for _ in 0..1024 {
            if !self.service_dma_pass()? {
                return Ok(());
            }
            self.bus.settle();
            self.sample_irq_lines();
        }
        Err(scheduler_error("DMA completion made no progress"))
    }

    /// Run to a natural halt or an absolute virtual-time cap (§3.1, §6).
    ///
    /// The stopping instruction completes all bus/DMA effects before a ready
    /// interrupt is acknowledged. Active caps count exact completed steps;
    /// halted caps round up to a whole virtual tick at fractional CPU rates.
    /// `None` imposes no cap. Captured output survives either stopping outcome.
    /// A previously reported reset is performed before any execution or cap
    /// check, unless [`Self::reset_cpu`] already completed it. An attached
    /// memory-map hook is flushed on every return, see [`Self::with_memory_map`].
    pub fn run_bounded(&mut self, cap: Option<u64>) -> Result<RunOutcome, MachineError> {
        let outcome = self.run_until(cap);
        let flushed = self.sync_memory_map(MemoryMapSync::Flush);
        let outcome = outcome?;
        flushed?;
        Ok(outcome)
    }

    /// Report a reset request, or stop the machine when the requests come
    /// in a storm: [`cpu::RESET_STORM_RESETS`] in a row, each fewer than
    /// [`cpu::RESET_STORM_STEPS`] steps of virtual time after the one
    /// before. A reset takes no virtual time, so a guest that shuts down
    /// again at once after every reset makes almost none, and one that
    /// shuts down before its first step makes none: a run capped by virtual
    /// time would never end; [`CpuError::ResetStorm`] ends that loop. The CPU still holds
    /// the state of the request, so a stop report shows where it shut down.
    /// See `backlog/docs/design/observation.md` §The machine resets again
    /// and again.
    fn reset_requested(&mut self, shutdown: bool) -> Result<RunOutcome, MachineError> {
        let Some((resets, steps)) = self.reset_streak.record(self.virtual_time()) else {
            return Ok(RunOutcome::ResetRequested);
        };
        let (cs, ip) = self.cpu.code_address();
        Err(CpuError::ResetStorm {
            resets,
            steps,
            shutdown,
            cs,
            ip,
        }
        .into())
    }

    fn run_until(&mut self, cap: Option<u64>) -> Result<RunOutcome, MachineError> {
        self.sync_a20();
        self.sync_memory_map(MemoryMapSync::Apply)?;
        if self.reset_pending {
            self.reset_cpu()?;
        }
        let mut completing = false;
        let mut no_progress = 0;
        self.settle(true)?;
        loop {
            if !completing
                && self
                    .reset_signal
                    .as_ref()
                    .is_some_and(|signal| signal.replace(false))
            {
                self.reset_pending = true;
                return self.reset_requested(false);
            }
            if !completing {
                // Policy observations and hard caps win over a new INTA.
                if cap.is_some_and(|cap| self.virtual_time() >= cap) {
                    return Ok(RunOutcome::RetiredCapReached);
                }
                if self.cpu.irq_ready() && !self.cpu.has_pending_irq() && self.pic.poll().is_some()
                {
                    if let Some(vector) = self.pic.acknowledge() {
                        self.cpu.inject_irq(vector);
                    }
                }
                if self.halted {
                    if !self.cpu.interrupts_enabled() {
                        return Ok(RunOutcome::Halted);
                    }
                    if self.cpu.has_pending_irq() {
                        self.halted = false;
                        self.idle_clocks = 0;
                    } else {
                        let Some(limit) = self.idle_limit else {
                            return Ok(RunOutcome::Halted);
                        };
                        let limit = PIT.ticks(limit);
                        if self.idle_clocks >= limit {
                            return Ok(RunOutcome::Halted);
                        }
                        let mut clocks = limit - self.idle_clocks;
                        if let Some(deadline) = self.next_deadline()? {
                            clocks = clocks.min(deadline.0 - self.clock.0);
                        }
                        let (inactive, inactive_credit) = self.inactive_halted_credit();
                        if let Some(cap) = cap {
                            let a = u128::from(self.cpu_rate_numerator.get());
                            let b = self.cpu_tick_denominator;
                            let credit = (u128::from(self.halted_clocks) * a + inactive_credit) % b;
                            let room = u128::from(cap - self.virtual_time())
                                .checked_mul(b)
                                .and_then(|ticks| ticks.checked_sub(credit))
                                .ok_or_else(|| scheduler_error("clock interval exhausted"))?;
                            clocks =
                                clocks.min(u64::try_from(room.div_ceil(a)).unwrap_or(u64::MAX));
                        }
                        let maximum_halted =
                            (u128::from(u64::MAX - self.completed_steps.saturating_add(inactive))
                                + 1)
                            .checked_mul(self.cpu_tick_denominator)
                            .ok_or_else(|| scheduler_error("clock interval exhausted"))?
                                - 1
                                - inactive_credit;
                        let halted_room = (maximum_halted
                            / u128::from(self.cpu_rate_numerator.get()))
                        .checked_sub(u128::from(self.halted_clocks))
                        .ok_or_else(|| scheduler_error("step counter exhausted"))?;
                        clocks = clocks
                            .min(u64::MAX - self.clock.0)
                            .min(u64::MAX - self.halted_clocks)
                            .min(u64::try_from(halted_room).unwrap_or(u64::MAX));
                        if clocks == 0 {
                            return Err(scheduler_error("virtual counter exhausted"));
                        }
                        let target = self
                            .clock
                            .0
                            .checked_add(clocks)
                            .ok_or_else(|| scheduler_error("clock exhausted"))?;
                        self.advance_to(VClock(target))?;
                        self.halted_clocks = self
                            .halted_clocks
                            .checked_add(clocks)
                            .ok_or_else(|| scheduler_error("halted clock counter exhausted"))?;
                        self.idle_clocks += clocks;
                        self.settle(false)?;
                        continue;
                    }
                }
            }

            let mut budget = cap
                .map_or(u64::MAX, |cap| cap - self.virtual_time())
                .min(self.step_limit.get());
            let options = cpu::StepRunOptions {
                stop_when_irq_ready: self.pic.poll().is_some() && !self.cpu.irq_ready(),
            };
            if completing {
                budget = 1;
            } else if self.reset_fetch_window {
                // In the reset-fetch window, stop after every step so the
                // board gate applies again right after the first CS load.
                budget = budget.min(1);
            } else if let Some(deadline) = self.next_deadline()? {
                let clocks = u128::from(deadline.0 - self.clock.0);
                let phase = clocks * u128::from(self.cpu_rate_numerator.get())
                    - u128::from(self.clock_remainder);
                let steps = phase.div_ceil(self.cpu_tick_denominator);
                budget = budget.min(u64::try_from(steps).unwrap_or(u64::MAX));
            }
            // Bound execution before touching the CPU: detecting overflow when
            // charging completed steps would leave an unaccounted guest effect.
            let clock_room = (u128::from(u64::MAX - self.clock.0) + 1)
                * u128::from(self.cpu_rate_numerator.get())
                - 1
                - u128::from(self.clock_remainder);
            let clock_steps = clock_room / self.cpu_tick_denominator;
            budget = budget
                .min(u64::MAX - self.virtual_time())
                .min(u64::try_from(clock_steps).unwrap_or(u64::MAX));
            if budget == 0 {
                return Err(scheduler_error("virtual counter exhausted"));
            }
            let before = self.cpu.completed_steps();
            let run = self.cpu.run_steps(budget, options);
            self.close_reset_fetch_window();
            self.read_deliveries();
            let run = match run {
                Ok(run) => run,
                Err(error) => {
                    self.charge_steps(self.cpu.completed_steps() - before)?;
                    self.settle(false)?;
                    return Err(error.into());
                }
            };
            if let Some(trail) = &mut self.code_trail {
                let (cs, ip) = self.cpu.code_address();
                trail.record(cs, ip as u16);
            }
            if run.steps == 0 {
                no_progress += 1;
                if no_progress > 1024 {
                    return Err(scheduler_error("instruction completion made no progress"));
                }
            } else {
                no_progress = 0;
            }
            let stop = match run.stop {
                CpuStepStop::IrqReady => CpuStop::BudgetExhausted,
                CpuStepStop::Exit(stop) => stop,
            };
            let counted_stop = matches!(
                stop,
                CpuStop::PortOut { .. } | CpuStop::PortIn { .. } | CpuStop::Halt
            );
            let prefix = if counted_stop {
                run.steps
                    .checked_sub(1)
                    .ok_or_else(|| scheduler_error("counted stop completed no steps"))?
            } else {
                run.steps
            };
            self.charge_steps(prefix)?;
            let accessed = matches!(
                stop,
                CpuStop::PortOut { .. }
                    | CpuStop::PortIn { .. }
                    | CpuStop::MmioWrite { .. }
                    | CpuStop::MmioRead { .. }
            );
            completing = false;
            let mut map_result = Ok(());
            let access_result = match stop {
                CpuStop::PortOut { port, size, value } => {
                    let result = self.bus.port_write(port, size, value as u32);
                    self.sync_a20();
                    map_result = self.sync_memory_map(MemoryMapSync::Apply);
                    result
                }
                CpuStop::PortIn { port, size } => self.bus.port_read(port, size).map(|value| {
                    self.cpu.complete_port_in(u64::from(value));
                }),
                CpuStop::MmioWrite { addr, size, value } => {
                    self.bus.mmio_write(addr, size, value).map(|()| {
                        self.cpu.complete_mmio_write();
                        completing = true;
                    })
                }
                CpuStop::MmioRead { addr, size } => self.bus.mmio_read(addr, size).map(|value| {
                    self.cpu.complete_mmio_read(value);
                    completing = true;
                }),
                CpuStop::NumericErrorAsserted => {
                    if let Some(assert) = &self.numeric_error_assert {
                        assert();
                        // The next WAIT can retire nothing, so no later clock
                        // advance is guaranteed to expose this IRQ to the PIC.
                        self.sample_irq_lines();
                    }
                    Ok(())
                }
                CpuStop::NumericErrorWaiting => {
                    // BUSY suspends instruction retirement just like HLT. Keep
                    // device clocks and interrupt arbitration alive while held.
                    self.halted = true;
                    no_progress = 0;
                    Ok(())
                }
                CpuStop::Exception { vector } => return Err(MachineError::Exception { vector }),
                // IBM Personal Computer AT Technical Reference, System Board,
                // Logic Diagrams: shutdown decoding resets the processor alone.
                CpuStop::Shutdown => {
                    self.reset_pending = true;
                    self.settle(false)?;
                    return self.reset_requested(true);
                }
                CpuStop::Halt => {
                    self.halted = true;
                    Ok(())
                }
                CpuStop::BudgetExhausted => Ok(()),
            };
            if self.numeric_error_ack.as_ref().is_some_and(|ack| ack()) {
                self.cpu.ack_external_numeric_error()?;
            }
            if accessed {
                // A register can gate another device's timer. Refresh every
                // deadline before advancing through the stopping instruction.
                self.bus.invalidate_deadlines();
            }
            let charge_result = if counted_stop {
                self.charge_steps(1)
            } else {
                Ok(())
            };
            // Even a rejected access can have device side effects. Charge and
            // settle its completed prefix, but preserve the access error if
            // clock accounting or coupled-device work also fails. An unsuccessful
            // deferred MMIO operation earns no extra step.
            let settle_result = self.settle(accessed);
            access_result?;
            map_result?;
            charge_result?;
            settle_result?;
        }
    }

    /// Propagate device IRQ level changes to the PIC (§5.1).
    ///
    /// Intel 8259A datasheet, Edge and Level Triggered Modes: inputs must stay
    /// high until INTA. The PIC owns edge detection; forwarding falls lets it
    /// withdraw requests whose device no longer needs service. The first sample
    /// establishes reset levels without fabricating a rising edge.
    fn sample_irq_lines(&mut self) {
        for (i, line) in self.irq_lines.iter().enumerate() {
            let Some(level) = self.bus.device_irq_output(line.device, line.output) else {
                continue;
            };
            if self.last_levels[i].is_some_and(|previous| previous != level) {
                self.pic.set_irq_line(line.irq, level);
            }
            self.last_levels[i] = Some(level);
        }
    }

    /// Run each wired bus master's pending DMA transfer through its DMA
    /// controller (§7.7), the cross-device path a floppy transfer needs.
    ///
    /// The seam is symmetric. For a device→memory ([`DmaOp::ToMemory`]) request —
    /// what a floppy READ queues — the requester hands over the bytes it read from
    /// its backing image; the machine streams them through the controller's
    /// channel, writing RAM via the `note_write` path (§4.3), dropping ROM
    /// writes, dispatching trapped windows through the MMIO bus, and reporting
    /// the byte count and TC signal back. For a memory→device request
    /// ([`DmaOp::FromMemory`]) — what a floppy WRITE queues — the machine reads
    /// the requested length from guest RAM, ROM or MMIO through the same channel,
    /// truncates the buffer to what the channel actually transferred (terminal
    /// count may cut it short, and the requester must never see uninitialised
    /// tail bytes), and hands the bytes
    /// back with the TC signal. Either way the controller advances the channel's
    /// address and count, then the requester finishes its result phase and raises
    /// its interrupt.
    ///
    /// This is the only place a device's bytes cross to or from memory without the
    /// device holding a memory reference: the requester never sees an address, the
    /// controller never sees the disk, and the SMC-safe seam is preserved.
    /// Bus faults propagate just like CPU MMIO faults; a failed transfer does
    /// not report a successful completion to the requester.
    #[cfg(test)]
    fn service_dma(&mut self) -> Result<(), BusError> {
        self.service_dma_pass().map(|_| ())
    }

    fn service_dma_pass(&mut self) -> Result<bool, BusError> {
        let mut transferred = false;
        for i in 0..self.dma_wirings.len() {
            let wiring = self.dma_wirings[i];
            let request = self
                .bus
                .device_mut(wiring.requester)
                .and_then(|dev| dev.take_dma_request());
            let Some(request) = request else {
                continue;
            };
            let (controller, channel) = self
                .dma_channel_wirings
                .iter()
                .find(|route| {
                    route.requester == wiring.requester && route.system_channel == request.channel
                })
                .map_or((wiring.controller, request.channel), |route| {
                    (route.controller, route.local_channel)
                });
            transferred = true;
            let mut mem = CpuDmaMemory {
                cpu: &mut self.cpu,
                read_offset: std::cell::Cell::new(0),
                mmio_reads: std::cell::RefCell::new(Vec::new()),
                mmio_writes: Vec::new(),
            };
            let outcome = match request.op {
                DmaOp::ToMemory(data) => {
                    let transfer = self
                        .bus
                        .device_mut(controller)
                        .map(|ctrl| ctrl.dma_service_write(channel, &mut mem, &data))
                        .unwrap_or_default();
                    for (addr, value) in mem.mmio_writes {
                        self.bus.mmio_write(u64::from(addr), 1, u64::from(value))?;
                    }
                    DmaOutcome::ToMemory {
                        transferred: transfer.transferred,
                        terminal_count: transfer.terminal_count,
                    }
                }
                DmaOp::FromMemory { len } => {
                    let mut buf = vec![0u8; len];
                    let transfer = self
                        .bus
                        .device_mut(controller)
                        .map(|ctrl| ctrl.dma_service_read(channel, &mut mem, &mut buf))
                        .unwrap_or_default();
                    buf.truncate(transfer.transferred);
                    for (offset, addr) in mem.mmio_reads.into_inner() {
                        buf[offset] = self.bus.mmio_read(u64::from(addr), 1)? as u8;
                    }
                    DmaOutcome::FromMemory {
                        bytes: buf,
                        terminal_count: transfer.terminal_count,
                    }
                }
            };
            if let Some(dev) = self.bus.device_mut(wiring.requester) {
                dev.dma_complete(outcome);
            }
        }
        Ok(transferred)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bus::{DecodeMap, PortRange};
    use device::{Device, DeviceError, DmaRequest, TraceEvent};
    use testkit::MockCpu;

    /// Only requests in a row, each fewer than the bound after the one
    /// before, make a storm; one longer distance, or a retired count that
    /// went back (a restored state), starts the count again.
    #[test]
    fn a_reset_storm_needs_its_requests_in_a_row_and_close_together() {
        let near = cpu::RESET_STORM_STEPS - 1;
        let resets = cpu::RESET_STORM_RESETS;
        let mut streak = ResetStreak::default();
        let mut retired = 0;
        for _ in 1..resets {
            retired += near;
            assert_eq!(streak.record(retired), None);
        }
        // A request as far from the last as the bound is not part of it.
        retired += cpu::RESET_STORM_STEPS;
        assert_eq!(streak.record(retired), None);
        for _ in 1..resets {
            retired += near;
            assert_eq!(streak.record(retired), None);
        }
        assert_eq!(streak.record(retired - 1), None, "went back");
        let start_again = retired - 1;
        let mut retired = start_again;
        for _ in 1..resets {
            retired += 1;
            assert_eq!(streak.record(retired), None);
        }
        retired += 1;
        assert_eq!(
            streak.record(retired),
            Some((resets, retired - start_again))
        );
    }

    #[test]
    fn i286_shutdown_resets_to_high_rom_preserving_ram_cmos_a20_and_time() {
        use cpu::{x86jit_backend::RealSegments, CpuReg, X86JitCpu};
        use devices::{A20Line, CmosRtc, I8042};
        use std::{cell::Cell, rc::Rc};

        let mut cpu =
            X86JitCpu::new_i286_bare(0x100_0000, 0, 0x1000, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(
            0x1000,
            &[
                0xFA, 0xC6, 0x06, 0, 5, 0x77, 0xB0, 0x0F, 0xE6, 0x70, 0xB0, 0x0A, 0xE6, 0x71, 0xB0,
                0xD1, 0xE6, 0x64, 0xB0, 1, 0xE6, 0x60, 0x0F, 0x01, 0x16, 0,
                0x21, // LGDT [2100h]
                0x0F, 0x01, 0x1E, 6, 0x21, // LIDT [2106h], no valid gates
                0xB8, 1, 0, 0x0F, 0x01, 0xF0, 0xEA, 0, 0x12, 8, 0,
            ],
        )
        .unwrap();
        cpu.write_mem(0x2008, &[0xFF, 0xFF, 0, 0, 0, 0x9A, 0, 0])
            .unwrap();
        cpu.write_mem(0x2010, &[0xFF, 0xFF, 0, 0, 0, 0x92, 0, 0])
            .unwrap();
        cpu.write_mem(0x2100, &[23, 0, 0, 0x20, 0, 0, 0, 0, 0, 0x30, 0, 0])
            .unwrap();
        cpu.write_mem(
            0x1200,
            &[
                0xB8, 0x10, 0, 0x8E, 0xD0, 0x8E, 0xD8, 0xB8, 0x18, 0, 0x8E,
                0xD8, // #GP; invalid #GP and #DF gates
            ],
        )
        .unwrap();
        cpu.map_rom(0xFFFFF0, &[0xEA, 0, 0x15, 0, 0]).unwrap();
        // This synthetic rig supplies code at the gated reset fetch as well.
        cpu.map_rom(0xEFFFF0, &[0xEA, 0, 0x15, 0, 0]).unwrap();
        cpu.write_mem(
            0x1500,
            &[
                0x0F, 0x01, 0xE3, // SMSW BX: reset left protected mode
                0xB0, 0x0F, 0xE6, 0x70, 0xE4, 0x71, 0xA2, 1, 5, 0xF4,
            ],
        )
        .unwrap();
        let reset = Rc::new(Cell::new(false));
        let a20 = A20Line::default();
        let keyboard = I8042::new().with_board_lines(reset.clone(), a20.clone());
        let bus = Bus::new(
            vec![Box::new(keyboard), Box::new(CmosRtc::new())],
            DecodeMap {
                ports: vec![
                    PortRange {
                        ports: 0x60..=0x64,
                        device: 0,
                        width: bus::PortWidth::Byte,
                    },
                    PortRange {
                        ports: 0x70..=0x71,
                        device: 1,
                        width: bus::PortWidth::Byte,
                    },
                ],
                mmio: Vec::new(),
            },
        );
        let mut machine = Machine::new(cpu, bus, Box::new(NoIc), Vec::new(), None)
            .with_reset_signal(reset.clone())
            .with_a20_line({
                let line = a20.clone();
                move || (line.change_count(), line.enabled())
            });
        assert_eq!(
            machine.run_bounded(Some(100)).unwrap(),
            RunOutcome::ResetRequested
        );
        assert_eq!(machine.cpu().code_segment(), 8);
        assert_eq!(machine.cpu().reg(CpuReg::Rip), 0x120A);
        assert!(
            !reset.get(),
            "shutdown must not pulse the keyboard reset latch"
        );
        assert!(!a20.enabled());
        assert!(!machine.cpu().a20_enabled());
        let changes = a20.change_count();
        let time = machine.virtual_time();
        let clock = machine.clock();
        let retired = machine.cpu().retired();
        assert_eq!(
            retired, time,
            "fault delivery and shutdown retire no instruction"
        );
        assert_eq!(
            machine.run_bounded(Some(time)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.cpu().code_segment(), 0xF000);
        assert_eq!(machine.cpu().reg(CpuReg::Rip), 0xFFF0);
        assert!(!machine.cpu().a20_enabled());
        assert_eq!(machine.cpu().retired(), retired);
        assert_eq!(machine.virtual_time(), time);
        assert_eq!(machine.clock(), clock);
        assert_eq!(machine.bus_mut().port_read(0x71, 1).unwrap(), 0x0A);
        assert_eq!(machine.run_bounded(Some(100)).unwrap(), RunOutcome::Halted);
        assert_eq!(machine.cpu().reg(CpuReg::Rbx), 0xFFF0);
        let mut markers = [0; 2];
        machine.cpu().read_mem(0x500, &mut markers).unwrap();
        assert_eq!(markers, [0x77, 0x0A]);
        assert!(!a20.enabled());
        assert_eq!(a20.change_count(), changes);
        assert_eq!(machine.bus_mut().port_read(0x71, 1).unwrap(), 0x0A);
    }

    /// With a reset-fetch probe, a CPU-only reset with the board gate off
    /// fetches the reset vector at FFFFF0h (nothing answers at EFFFF0h),
    /// and the first CS load gives the CPU the board's gate again; the 8042
    /// output port keeps the value the guest wrote.
    #[test]
    fn reset_fetch_window_opens_the_gate_until_the_first_cs_load() {
        use cpu::{x86jit_backend::RealSegments, X86JitCpu};
        use devices::{A20Line, I8042};
        use std::{cell::Cell, rc::Rc};

        let mut cpu =
            X86JitCpu::new_i286_bare(0x100_0000, 0, 0x1000, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(
            0x1000,
            &[
                0xFA, 0xFE, 0x06, 0, 5, // CLI; INC BYTE [0500h]
                0xB0, 0xD1, 0xE6, 0x64, 0xB0, 1, 0xE6, 0x60, // output port 01h: A20 off
                0xB0, 0xFE, 0xE6, 0x64, 0xF4, // pulse the reset line; HLT
            ],
        )
        .unwrap();
        // The reset vector jumps to 0000:1500h, which marks and halts.
        cpu.map_rom(0xFFFFF0, &[0xEA, 0, 0x15, 0, 0]).unwrap();
        cpu.write_mem(0x1500, &[0xFE, 0x06, 1, 5, 0xF4]).unwrap();
        let reset = Rc::new(Cell::new(false));
        let a20 = A20Line::default();
        let keyboard = I8042::new().with_board_lines(reset.clone(), a20.clone());
        let bus = Bus::new(
            vec![Box::new(keyboard)],
            DecodeMap {
                ports: vec![PortRange {
                    ports: 0x60..=0x64,
                    device: 0,
                    width: bus::PortWidth::Byte,
                }],
                mmio: Vec::new(),
            },
        );
        let mut machine = Machine::new(cpu, bus, Box::new(NoIc), Vec::new(), None)
            .with_reset_signal(reset)
            .with_a20_line({
                let line = a20.clone();
                move || (line.change_count(), line.enabled())
            })
            .with_reset_fetch_probe(|cpu: &X86JitCpu| {
                let cs = cpu.diagnostic_state().cs();
                cs.selector == 0xF000 && cs.base == 0xFF_0000
            });
        assert_eq!(
            machine.run_bounded(Some(100)).unwrap(),
            RunOutcome::ResetRequested
        );
        let time = machine.virtual_time();
        machine.reset_cpu().unwrap();
        assert!(!a20.enabled());
        assert!(machine.cpu().a20_enabled(), "the window holds A20 open");
        assert_eq!(
            machine.run_bounded(Some(time + 1)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.cpu().code_segment(), 0);
        assert!(!machine.cpu().a20_enabled(), "the far jump closes it");
        assert_eq!(machine.run_bounded(Some(100)).unwrap(), RunOutcome::Halted);
        let mut markers = [0; 2];
        machine.cpu().read_mem(0x500, &mut markers).unwrap();
        assert_eq!(markers, [1, 1]);
        machine.bus_mut().port_write(0x64, 1, 0xD0).unwrap();
        assert_eq!(machine.bus_mut().port_read(0x60, 1).unwrap(), 1);
    }

    #[test]
    fn memory_map_hook_applies_after_each_port_write_and_flushes_on_return() {
        use cpu::{x86jit_backend::RealSegments, CpuReg, X86JitCpu};
        use std::{cell::RefCell, rc::Rc};

        let guest = || {
            let mut cpu =
                X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
            cpu.map_ram(0, 0x10000).unwrap();
            // MOV DX, 3F8h; MOV AL, 41h; OUT DX, AL; MOV AL, 42h; OUT DX, AL; HLT.
            cpu.write_mem(
                0x7C00,
                &[0xBA, 0xF8, 0x03, 0xB0, 0x41, 0xEE, 0xB0, 0x42, 0xEE, 0xF4],
            )
            .unwrap();
            cpu
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let log = events.clone();
        let mut machine = Machine::new(
            guest(),
            bus_with_capture(),
            Box::new(NoIc),
            Vec::new(),
            None,
        )
        .with_memory_map(move |cpu: &mut X86JitCpu, event| {
            log.borrow_mut().push((event, cpu.reg(CpuReg::Rax) & 0xFF));
            Ok(())
        });
        assert_eq!(machine.run_bounded(Some(100)).unwrap(), RunOutcome::Halted);
        // Each Apply sees the value just written and not the next MOV AL.
        assert_eq!(
            *events.borrow(),
            [
                (MemoryMapSync::Apply, 0),
                (MemoryMapSync::Apply, 0x41),
                (MemoryMapSync::Apply, 0x42),
                (MemoryMapSync::Flush, 0x42),
            ]
        );

        let events = Rc::new(RefCell::new(Vec::new()));
        let log = events.clone();
        let mut failing = Machine::new(
            guest(),
            bus_with_capture(),
            Box::new(NoIc),
            Vec::new(),
            None,
        )
        .with_memory_map(move |_: &mut X86JitCpu, event| {
            log.borrow_mut().push(event);
            if log.borrow().len() == 2 {
                return Err(CpuError::Memory("refused".into()));
            }
            Ok(())
        });
        assert!(matches!(
            failing.run_bounded(Some(100)),
            Err(MachineError::Cpu(CpuError::Memory(_)))
        ));
        assert_eq!(
            *events.borrow(),
            [
                MemoryMapSync::Apply,
                MemoryMapSync::Apply,
                MemoryMapSync::Flush
            ]
        );
        assert_eq!(failing.bus().device(0).unwrap().save(), [0x41]);
    }

    /// A minimal capture device: records port writes, so the loop can be tested
    /// without a concrete `devices` chip.
    #[derive(Default)]
    struct CaptureDevice {
        written: Vec<u8>,
        ticks: u64,
        reset_signal: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    }

    impl Device for CaptureDevice {
        fn name(&self) -> &'static str {
            "capture"
        }
        fn port_read(&mut self, _port: u16, _size: u8) -> Result<u32, DeviceError> {
            Ok(0xAB)
        }
        fn port_write(&mut self, _port: u16, _size: u8, value: u32) -> Result<(), DeviceError> {
            self.written.push(value as u8);
            if let Some(signal) = &self.reset_signal {
                if value == 0xFE {
                    signal.set(true);
                }
            }
            Ok(())
        }
        fn mmio_read(&mut self, addr: u64, _size: u8) -> Result<u64, DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            })
        }
        fn mmio_write(&mut self, addr: u64, _size: u8, _value: u64) -> Result<(), DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            })
        }
        fn tick(&mut self, _now: VClock) {
            self.ticks += 1;
        }
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {
            self.written.clear();
            self.ticks = 0;
        }
        fn save(&self) -> Vec<u8> {
            self.written.clone()
        }
        fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError> {
            self.written = snapshot.to_vec();
            Ok(())
        }
        fn take_trace(&mut self) -> Vec<TraceEvent> {
            Vec::new()
        }
    }

    fn bus_with_capture() -> Bus {
        let devices: Vec<Box<dyn Device>> = vec![Box::new(CaptureDevice::default())];
        let map = DecodeMap {
            ports: vec![PortRange {
                width: bus::PortWidth::default(),
                ports: 0x3F8..=0x3FF,
                device: 0,
            }],
            mmio: Vec::new(),
        };
        Bus::new(devices, map)
    }

    /// The code trail records CS:IP after each slice: each OUT ends one, a
    /// polling loop folds into one entry, and the trail ends at the HLT. A
    /// twin machine without the trail retires the same instructions in the
    /// same virtual time.
    #[test]
    fn the_code_trail_records_slice_boundaries_without_changing_the_run() {
        use cpu::{x86jit_backend::RealSegments, X86JitCpu};
        let program = [
            0xE6, 0x80, // 1000 OUT 80h,AL
            0xE6, 0x80, // 1002 OUT 80h,AL
            0xB9, 0x05, 0x00, // 1004 MOV CX,5
            0xE6, 0x80, // 1007 OUT 80h,AL (the loop)
            0xE2, 0xFC, // 1009 LOOP 1007h
            0xFA, // 100B CLI
            0xF4, // 100C HLT
        ];
        let run = |trail: bool| {
            let mut cpu =
                X86JitCpu::new_i286_bare(0x100_0000, 0, 0x1000, RealSegments::flat(0)).unwrap();
            cpu.map_ram(0, 0x10000).unwrap();
            cpu.write_mem(0x1000, &program).unwrap();
            let bus = Bus::new(Vec::new(), DecodeMap::default());
            let mut machine = Machine::new(cpu, bus, Box::new(NoIc), Vec::new(), None);
            if trail {
                machine.enable_code_trail();
            }
            let outcome = machine.run_bounded(Some(1000)).unwrap();
            assert_eq!(outcome, RunOutcome::Halted);
            assert!(machine.halted());
            (
                machine.cpu().retired(),
                machine.virtual_time(),
                machine.code_trail().map(|trail| trail.entries()),
            )
        };
        let (retired, time, entries) = run(true);
        assert_eq!(run(false), (retired, time, None));
        let entries = entries.unwrap();
        let addresses: Vec<(u16, u16, u32)> = entries
            .iter()
            .map(|entry| (entry.cs, entry.ip, entry.samples))
            .collect();
        // After each OUT the CPU stands on the next instruction; the loop's
        // OUT returns to the same place five times; HLT retires.
        assert_eq!(
            addresses,
            [
                (0, 0x1002, 1),
                (0, 0x1004, 1),
                (0, 0x1009, 5),
                (0, 0x100D, 1)
            ]
        );
    }

    /// A no-op controller for the interrupt-free loop tests.
    pub(crate) struct NoIc;
    impl InterruptController for NoIc {
        fn set_irq_line(&mut self, _irq: u8, _level: bool) {}
        fn poll(&self) -> Option<u8> {
            None
        }
        fn acknowledge(&mut self) -> Option<u8> {
            None
        }
        fn end_of_interrupt(&mut self) {}
    }

    fn instruction_cpu(program: Vec<CpuStop>, enabled: bool) -> MockCpu {
        let script = program
            .iter()
            .copied()
            .chain([CpuStop::Halt; 4])
            .map(|stop| {
                let steps = u64::from(!matches!(stop, CpuStop::Exception { .. }));
                testkit::MockCpuStep {
                    max_steps: 1,
                    run: cpu::CpuStepRun {
                        steps,
                        stop: CpuStepStop::Exit(stop),
                    },
                    retired: steps,
                    irq_ready: enabled,
                }
            })
            .collect();
        MockCpu::new(program)
            .with_interrupts_enabled(enabled)
            .with_step_program(script)
    }

    fn machine_with(program: Vec<CpuStop>) -> Machine<MockCpu> {
        Machine::new(
            instruction_cpu(program, false),
            bus_with_capture(),
            Box::new(NoIc),
            Vec::new(),
            Some(1),
        )
        .with_step_limit(NonZeroU64::MIN)
    }

    #[test]
    fn turbo_rescales_phase_and_keeps_past_halted_time() {
        let mut machine = machine_with(Vec::new())
            .with_cpu_rate(NonZeroU64::new(3).unwrap(), NonZeroU64::new(2).unwrap())
            .with_turbo_rate(NonZeroU64::new(7).unwrap(), NonZeroU64::new(3).unwrap());
        machine.charge_steps(1).unwrap();
        let old_phase = machine.clock_remainder;
        machine.halted_clocks = TICKS_PER_PIT_CLOCK * 100;
        let before = machine.virtual_time();
        let clock = machine.clock();
        assert!(machine.set_turbo(true));
        assert_eq!(machine.virtual_time(), before);
        assert_eq!(machine.clock(), clock);
        assert_eq!(machine.clock_remainder, old_phase * 7 / 3);
        let phase = machine.clock_remainder;
        assert!(machine.set_turbo(true));
        assert_eq!(machine.clock_remainder, phase);
        machine.halted_clocks = TICKS_PER_PIT_CLOCK * 30;
        let before = machine.virtual_time();
        assert!(machine.set_turbo(false));
        assert_eq!(machine.virtual_time(), before);
        machine.charge_steps(3).unwrap();
        assert_eq!(machine.clock().0 - clock.0, 2 * TICKS_PER_PIT_CLOCK);
    }

    #[test]
    fn turbo_toggles_preserve_fractional_halted_credit_and_caps() {
        let mut machine = machine_with(Vec::new())
            .with_cpu_rate(
                NonZeroU64::new(11).unwrap(),
                NonZeroU64::new(52_500).unwrap(),
            )
            .with_turbo_rate(
                NonZeroU64::new(11).unwrap(),
                NonZeroU64::new(26_250).unwrap(),
            );
        machine.bus = Bus::new(Vec::new(), DecodeMap::default());
        machine.cpu = instruction_cpu(Vec::new(), true);
        machine.halted = true;
        let mut elapsed = 0;
        for interval in 1..=2000 {
            let clocks = if machine.turbo_enabled() { 2000 } else { 4000 };
            elapsed += clocks;
            machine.set_idle_limit(Some(elapsed));
            assert_eq!(machine.run_bounded(None).unwrap(), RunOutcome::Halted);
            let expected = interval * 88 / 105;
            assert_eq!(machine.virtual_time(), expected, "interval {interval}");
            assert!(machine.set_turbo(!machine.turbo_enabled()));
            assert_eq!(machine.virtual_time(), expected, "toggle {interval}");
            assert_eq!(machine.clock(), VClock::from_pit(elapsed));
        }
        let before = machine.clock();
        machine.set_idle_limit(Some(elapsed + 10_000));
        let cap = machine.virtual_time() + 1;
        assert_eq!(
            machine.run_bounded(Some(cap)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        let expected_ticks = PIT.ticks(cap * 52_500 - 2000 * 44_000).div_ceil(11);
        assert_eq!(machine.clock().0 - before.0, expected_ticks);
        assert_eq!(machine.virtual_time(), cap);
    }

    #[test]
    fn turbo_halted_caps_combine_coprime_denominators_in_both_positions() {
        for final_turbo in [false, true] {
            let mut machine = machine_with(Vec::new())
                .with_cpu_rate(NonZeroU64::new(2).unwrap(), NonZeroU64::new(3).unwrap())
                .with_turbo_rate(NonZeroU64::new(3).unwrap(), NonZeroU64::new(7).unwrap());
            machine.bus = Bus::new(Vec::new(), DecodeMap::default());
            machine.cpu = instruction_cpu(Vec::new(), true);
            machine.halted = true;
            let mut expected_credit = 0;
            for interval in 1..=2000 {
                expected_credit += if machine.turbo_enabled() { 9 } else { 14 };
                machine.set_idle_limit(Some(interval));
                assert_eq!(machine.run_bounded(None).unwrap(), RunOutcome::Halted);
                assert_eq!(machine.virtual_time(), expected_credit / 21);
                assert!(machine.set_turbo(!machine.turbo_enabled()));
                assert_eq!(machine.virtual_time(), expected_credit / 21);
            }
            assert!(machine.set_turbo(final_turbo));
            let cap = machine.virtual_time() + 1;
            let credit_per_tick = if final_turbo { 9 } else { 14 };
            let ticks = PIT
                .ticks(cap * 21 - expected_credit)
                .div_ceil(credit_per_tick);
            let before = machine.clock();
            machine.set_idle_limit(Some(2100));
            assert_eq!(
                machine.run_bounded(Some(cap)).unwrap(),
                RunOutcome::RetiredCapReached
            );
            assert_eq!(machine.clock().0 - before.0, ticks);
            assert_eq!(machine.virtual_time(), cap);
        }
    }

    #[test]
    fn cpu_reset_keeps_turbo_selection_and_timing_state() {
        let cpu = cpu::X86JitCpu::new_real16_bare(
            0x10_0000,
            0,
            0,
            cpu::x86jit_backend::RealSegments::flat(0),
        )
        .unwrap();
        let mut machine = Machine::new(cpu, bus_with_capture(), Box::new(NoIc), Vec::new(), None)
            .with_cpu_rate(NonZeroU64::new(3).unwrap(), NonZeroU64::new(2).unwrap())
            .with_turbo_rate(NonZeroU64::new(7).unwrap(), NonZeroU64::new(3).unwrap());
        machine.halted_clocks = PIT.ticks(1);
        assert!(machine.set_turbo(true));
        machine.charge_steps(8).unwrap();
        machine.halted_clocks = PIT.ticks(1);
        let before = (
            machine.clock(),
            machine.virtual_time(),
            machine.clock_remainder,
            machine.halted_clocks,
            machine.inactive_halted_clocks,
        );
        machine.reset_cpu().unwrap();
        assert!(machine.turbo_enabled());
        assert_eq!(
            (
                machine.cpu_rate_numerator.get(),
                machine.cpu_rate_denominator.get()
            ),
            (7, 3)
        );
        assert_eq!(
            (
                machine.clock(),
                machine.virtual_time(),
                machine.clock_remainder,
                machine.halted_clocks,
                machine.inactive_halted_clocks
            ),
            before
        );
        assert!(machine.set_turbo(false));
        assert_eq!(machine.virtual_time(), before.1);
    }

    #[test]
    fn turbo_without_an_alternate_rate_is_rejected_without_change() {
        let mut machine = machine_with(Vec::new());
        assert!(!machine.set_turbo(true));
        assert!(!machine.turbo_enabled());
        assert!(machine.set_turbo(false));
        assert_eq!(machine.clock(), VClock::ZERO);
    }

    #[test]
    fn unsupported_reset_blocks_resumption_and_remains_pending() {
        for bounded in [false, true] {
            let signal = std::rc::Rc::new(std::cell::Cell::new(false));
            let capture = CaptureDevice {
                reset_signal: Some(signal.clone()),
                ..Default::default()
            };
            let bus = Bus::new(
                vec![Box::new(capture)],
                DecodeMap {
                    ports: vec![PortRange {
                        width: bus::PortWidth::Byte,
                        ports: 0x64..=0x64,
                        device: 0,
                    }],
                    mmio: Vec::new(),
                },
            );
            let cpu = instruction_cpu(
                vec![
                    CpuStop::PortOut {
                        port: 0x64,
                        size: 1,
                        value: 0xFE,
                    },
                    CpuStop::PortOut {
                        port: 0x64,
                        size: 1,
                        value: 0x55,
                    },
                    CpuStop::Halt,
                ],
                false,
            );
            let mut machine = Machine::new(cpu, bus, Box::new(NoIc), Vec::new(), None)
                .with_reset_signal(signal.clone());
            let outcome = if bounded {
                machine.run_bounded(Some(100))
            } else {
                machine.run()
            };
            assert_eq!(outcome.unwrap(), RunOutcome::ResetRequested);
            assert_eq!(machine.virtual_time(), 1);
            assert!(!signal.get());
            assert_eq!(machine.bus().device(0).unwrap().save(), [0xFE]);
            for _ in 0..2 {
                assert!(matches!(
                    machine.run(),
                    Err(MachineError::Cpu(CpuError::UnexpectedExit { .. }))
                ));
                assert!(machine.reset_pending);
                assert_eq!(machine.virtual_time(), 1);
                assert_eq!(machine.bus().device(0).unwrap().save(), [0xFE]);
            }
        }
    }

    #[test]
    fn reset_request_wins_over_a_coincident_instruction_cap() {
        let signal = std::rc::Rc::new(std::cell::Cell::new(false));
        let mut machine = machine_with(vec![
            CpuStop::PortOut {
                port: 0x3F8,
                size: 1,
                value: 0xFE,
            },
            CpuStop::Halt,
        ])
        .with_reset_signal(signal.clone());
        *machine.bus_mut().device_mut(0).unwrap() = Box::new(CaptureDevice {
            reset_signal: Some(signal.clone()),
            ..Default::default()
        });
        assert_eq!(
            machine.run_bounded(Some(1)).unwrap(),
            RunOutcome::ResetRequested
        );
        assert_eq!(machine.virtual_time(), 1);
        assert_eq!(machine.bus().device(0).unwrap().save(), [0xFE]);
        assert!(!signal.get());
        assert!(matches!(
            machine.run_bounded(Some(1)),
            Err(MachineError::Cpu(CpuError::UnexpectedExit { .. }))
        ));
        assert!(machine.reset_pending);
        assert_eq!(machine.virtual_time(), 1);
    }

    #[test]
    fn explicit_reset_clears_the_obligation_before_the_next_run() {
        let cpu = cpu::X86JitCpu::new_real16_bare(
            0x10_0000,
            0,
            0,
            cpu::x86jit_backend::RealSegments::flat(0),
        )
        .unwrap();
        let signal = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut machine = Machine::new(cpu, bus_with_capture(), Box::new(NoIc), Vec::new(), None)
            .with_reset_signal(signal);
        assert_eq!(
            machine.run_bounded(Some(0)).unwrap(),
            RunOutcome::ResetRequested
        );
        machine.reset_cpu().unwrap();
        machine.cpu.inject_irq(0x20);
        assert_eq!(
            machine.run_bounded(Some(0)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert!(
            machine.cpu.has_pending_irq(),
            "a completed reset must not repeat on resumption"
        );
    }

    #[test]
    fn halted_counter_exhaustion_does_not_saturate_or_advance_devices() {
        let mut machine =
            machine_with(Vec::new()).with_cpu_rate(NonZeroU64::new(2).unwrap(), NonZeroU64::MIN);
        machine.bus = Bus::new(
            Vec::new(),
            DecodeMap {
                ports: Vec::new(),
                mmio: Vec::new(),
            },
        );
        machine.cpu = instruction_cpu(Vec::new(), true);
        machine.halted = true;
        machine.halted_clocks = u64::MAX / 2;
        machine.set_idle_limit(Some(u64::MAX));
        assert_eq!(machine.virtual_time(), u64::MAX - 1);
        assert!(machine.run_bounded(Some(u64::MAX)).is_err());
        assert_eq!(machine.virtual_time(), u64::MAX - 1);
        assert_eq!(machine.clock(), VClock(0));
    }

    #[test]
    fn counter_exhaustion_refuses_the_next_instruction() {
        for ticks in [false, true] {
            let mut machine = machine_with(vec![CpuStop::Halt]);
            machine.bus = Bus::new(
                Vec::new(),
                DecodeMap {
                    ports: Vec::new(),
                    mmio: Vec::new(),
                },
            );
            if ticks {
                machine = machine.with_cpu_rate(NonZeroU64::MIN, NonZeroU64::new(2).unwrap());
                machine.clock = VClock(u64::MAX - 1);
            } else {
                machine.completed_steps = u64::MAX;
            }
            let before = (machine.clock(), machine.virtual_time());
            assert!(machine.run_bounded(None).is_err());
            assert_eq!(
                machine.cpu.completed_steps(),
                0,
                "an overflowing instruction must not execute"
            );
            assert_eq!((machine.clock(), machine.virtual_time()), before);
        }
    }

    #[test]
    fn cpu_rate_conversion_is_independent_of_step_partitioning() {
        const TOTAL: u64 = 4095;
        for (numerator, denominator) in [(1, 1), (2, 1), (1, 3), (3, 2), (7, 3), (6, 4)] {
            for stride in [1, 3, 7, 15, 63, TOTAL] {
                let mut machine = machine_with(Vec::new()).with_cpu_rate(
                    NonZeroU64::new(numerator).unwrap(),
                    NonZeroU64::new(denominator).unwrap(),
                );
                for retired in (stride..=TOTAL).step_by(stride as usize) {
                    machine.charge_steps(stride).unwrap();
                    assert_eq!(machine.clock(), VClock(retired * denominator / numerator));
                    assert_eq!(machine.clock_remainder, retired * denominator % numerator);
                    assert_eq!(machine.virtual_time(), retired);
                    let before = machine.clock();
                    machine.charge_steps(0).unwrap();
                    assert_eq!(
                        machine.clock(),
                        before,
                        "zero retirement must not age devices"
                    );
                }
            }
        }
    }

    #[test]
    fn cpu_rate_remainder_survives_halts_and_wakeups() {
        const CYCLES: usize = 64;
        for (numerator, denominator) in [(1, 1), (2, 1), (1, 3), (3, 2), (7, 3)] {
            let program = [CpuStop::Halt, CpuStop::BudgetExhausted].repeat(CYCLES);
            let mut machine = machine_with(program.clone()).with_cpu_rate(
                NonZeroU64::new(numerator).unwrap(),
                NonZeroU64::new(denominator).unwrap(),
            );
            machine.cpu = instruction_cpu(program, true);
            machine.set_idle_limit(Some(1));

            for cycle in 0..CYCLES as u64 {
                assert_eq!(machine.run_bounded(None).expect("halt"), RunOutcome::Halted);
                let retired = 2 * cycle + 1;
                let halted = cycle + 1;
                assert_eq!(machine.cpu().retired(), retired);
                assert_eq!(
                    machine.clock(),
                    VClock(retired * denominator / numerator + halted)
                );
                assert_eq!(machine.clock_remainder, retired * denominator % numerator);
                assert_eq!(
                    machine.virtual_time(),
                    retired + halted * numerator / denominator
                );

                machine.cpu.inject_irq(0x08);
                let cap = machine.virtual_time() + 1;
                assert_eq!(
                    machine.run_bounded(Some(cap)).expect("wake"),
                    RunOutcome::RetiredCapReached
                );
                assert_eq!(machine.cpu().retired(), retired + 1);
                assert_eq!(
                    machine.clock(),
                    VClock((retired + 1) * denominator / numerator + halted)
                );
                assert_eq!(machine.virtual_time(), cap);
            }
        }
    }

    #[test]
    fn halted_cpu_rate_caps_preserve_fraction_across_runner_calls() {
        for (numerator, denominator) in [(2, 1), (1, 3), (3, 2)] {
            let mut machine = machine_with(vec![CpuStop::Halt]).with_cpu_rate(
                NonZeroU64::new(numerator).unwrap(),
                NonZeroU64::new(denominator).unwrap(),
            );
            machine.cpu = machine.cpu.with_interrupts_enabled(true);
            machine.set_idle_limit(Some(1000));
            for cap in [2, 3, 7, 19] {
                assert_eq!(
                    machine.run_bounded(Some(cap)).expect("cap"),
                    RunOutcome::RetiredCapReached
                );
                let halted = ((cap - 1) * denominator).div_ceil(numerator);
                assert_eq!(machine.cpu().retired(), 1);
                assert_eq!(machine.halted_clocks, halted);
                assert_eq!(machine.clock(), VClock(denominator / numerator + halted));
                assert_eq!(machine.clock_remainder, denominator % numerator);
                assert_eq!(machine.virtual_time(), 1 + halted * numerator / denominator);
                let before = machine.clock();
                machine.run_bounded(Some(cap)).expect("same cap");
                assert_eq!(machine.clock(), before);
            }
        }
    }

    #[test]
    fn cpu_rate_large_factors_do_not_overflow() {
        let mut machine = machine_with(vec![CpuStop::BudgetExhausted; 10]).with_cpu_rate(
            NonZeroU64::new(u64::MAX).unwrap(),
            NonZeroU64::new(u64::MAX - 1).unwrap(),
        );
        for retired in (2..=20).step_by(2) {
            machine.charge_steps(2).unwrap();
            assert_eq!(machine.clock(), VClock(PIT.ticks(retired) - 1));
            assert_eq!(machine.clock_remainder, u64::MAX - PIT.ticks(retired));
        }

        let mut machine = machine_with(Vec::new()).with_cpu_rate(
            NonZeroU64::new(u64::MAX).unwrap(),
            NonZeroU64::new(u64::MAX / 2 + 1).unwrap(),
        );
        machine.halted_clocks = PIT.ticks(2);
        assert_eq!(machine.virtual_time(), 3, "divide before saturating");
        machine = machine.with_cpu_rate(NonZeroU64::new(u64::MAX).unwrap(), NonZeroU64::MIN);
        assert_eq!(machine.virtual_time(), u64::MAX);
    }

    #[test]
    fn unmigrated_device_next_event_defaults_to_every_pit_clock() {
        let machine = machine_with(Vec::new());
        let device = machine.bus().device(0).expect("capture device");
        for now in [VClock::ZERO, VClock(17), VClock(u64::MAX)] {
            assert_eq!(device.next_event(now), device::NextEvent::EveryPitClock);
            assert!(device.save().is_empty());
        }
    }

    #[test]
    fn loop_dispatches_port_out_then_halts() {
        let program = vec![
            CpuStop::PortOut {
                port: 0x3F8,
                size: 1,
                value: b'h' as u64,
            },
            CpuStop::PortOut {
                port: 0x3F8,
                size: 1,
                value: b'i' as u64,
            },
            CpuStop::Halt,
        ];
        let mut machine = machine_with(program);
        machine.run().expect("run");

        // Each MockCpu run() retires one instruction and the ratio is 1:1, so
        // three boundaries advance the clock to 3.
        assert_eq!(machine.clock(), VClock(3));
    }

    #[test]
    fn loop_completes_port_in() {
        let program = vec![CpuStop::PortIn {
            port: 0x3F8,
            size: 1,
        }];
        let mut machine = machine_with(program);
        machine.run().expect("run");
        // The capture device returns 0xAB for reads; the machine completes the
        // pending in with it.
        assert_eq!(machine.cpu().completions, vec![0xAB]);
    }

    #[test]
    fn loop_surfaces_exception() {
        let program = vec![CpuStop::Exception { vector: 6 }];
        let mut machine = machine_with(program);
        let err = machine.run().unwrap_err();
        assert!(matches!(err, MachineError::Exception { vector: 6 }));
    }

    #[test]
    fn retired_cap_terminates_a_spinning_guest() {
        // A guest that keeps retiring without ever halting: a long run of
        // budget-exhausted yields (far more than the cap). Without a cap the
        // machine would keep looping; the cap must stop it cleanly at the bound.
        let cap = 5;
        let program = vec![CpuStop::BudgetExhausted; 1000];
        let mut machine = machine_with(program);
        let outcome = machine.run_bounded(Some(cap)).expect("run");
        assert_eq!(outcome, RunOutcome::RetiredCapReached);
        // 1:1 retired-per-run, so the machine stops the first boundary at which
        // the retired count reaches the cap — exactly `cap` instructions in.
        assert_eq!(machine.clock(), VClock(cap));
    }

    /// A device with a scriptable IRQ line and EOI observation, standing in for
    /// the PIT/PIC pairing to test the wiring in isolation.
    struct LineDevice {
        level: bool,
        stop_after: Option<u64>,
        period: Option<u64>,
    }
    // Initial sampling now precedes the first clock: start on the steady high
    // level so this mock's first periodic rising edge remains at clock five.
    impl Default for LineDevice {
        fn default() -> Self {
            Self {
                level: true,
                stop_after: None,
                period: None,
            }
        }
    }
    impl Device for LineDevice {
        fn name(&self) -> &'static str {
            "line"
        }
        fn port_read(&mut self, _port: u16, _size: u8) -> Result<u32, DeviceError> {
            Ok(0)
        }
        fn port_write(&mut self, _port: u16, _size: u8, _value: u32) -> Result<(), DeviceError> {
            Ok(())
        }
        fn mmio_read(&mut self, _addr: u64, _size: u8) -> Result<u64, DeviceError> {
            Ok(0)
        }
        fn mmio_write(&mut self, _addr: u64, _size: u8, _value: u64) -> Result<(), DeviceError> {
            Ok(())
        }
        fn tick(&mut self, now: VClock) {
            // Drive the line low for one clock every period, else high — a
            // mode-2-like OUT so the rising edge recurs once per period.
            self.level = (now == VClock::ZERO || !now.0.is_multiple_of(self.period.unwrap_or(4)))
                && self.stop_after.is_none_or(|end| now.0 <= end);
        }
        fn irq_pending(&self) -> bool {
            self.level
        }
        fn reset(&mut self) {}
        fn save(&self) -> Vec<u8> {
            Vec::new()
        }
        fn restore(&mut self, _s: &[u8]) -> Result<(), DeviceError> {
            Ok(())
        }
    }

    /// A counting interrupt controller: records rising input transitions and
    /// hands out one serviceable vector per rise, cleared by acknowledge.
    #[derive(Default)]
    struct CountingIc {
        pulses: u32,
        pending: u32,
        acknowledgements: std::rc::Rc<std::cell::Cell<u64>>,
    }
    impl InterruptController for CountingIc {
        fn set_irq_line(&mut self, _irq: u8, level: bool) {
            if level {
                self.pulses += 1;
                self.pending += 1;
            }
        }
        fn poll(&self) -> Option<u8> {
            (self.pending > 0).then_some(0x08)
        }
        fn acknowledge(&mut self) -> Option<u8> {
            self.acknowledgements.set(self.acknowledgements.get() + 1);
            if self.pending > 0 {
                self.pending -= 1;
                Some(0x08)
            } else {
                None
            }
        }
        fn end_of_interrupt(&mut self) {}
    }

    #[test]
    fn irq_sampling_produces_one_request_per_period() {
        // Drive the machine so the line device is ticked across several periods
        // and assert exactly one rising PIC input per period. The line goes low at
        // ticks 4, 8, 12, … and high in between, so a rising edge occurs once per
        // 4-tick period.
        let program = vec![CpuStop::BudgetExhausted; 20];
        // Each run() retires one instruction (1 tick). We need the LineDevice at
        // a known bus index and a CountingIc to observe rising inputs. Rebuild a bus.
        let devices: Vec<Box<dyn Device>> = vec![Box::new(LineDevice::default())];
        let map = DecodeMap {
            ports: Vec::new(),
            mmio: Vec::new(),
        };
        let bus = Bus::new(devices, map);
        let mut machine = Machine::new(
            instruction_cpu(program, true),
            bus,
            Box::<CountingIc>::default(),
            vec![IrqLine {
                device: 0,
                output: 0,
                irq: 0,
            }],
            Some(1),
        )
        .with_step_limit(NonZeroU64::MIN);
        machine.run().expect("run");
        // The line device starts high, then is low at ticks 4, 8, … and high in between, so a
        // rising edge (one PIC request per 4-tick period) occurs at ticks 5, 9,
        // 13, 17, 21. The final `Halt` boundary advances the clock to 21, giving
        // five rising edges → five injected vectors. We can't read the boxed IC's
        // private counter, so assert the CPU received the matching injections.
        assert_eq!(machine.cpu().injected.len(), 5);
        assert!(machine.cpu().injected.iter().all(|&v| v == 0x08));
    }

    #[test]
    fn deadline_and_per_clock_preserve_every_pit_irq0_edge() {
        use devices::Pit8253;

        fn edges(engine: SchedulerEngine, mode: u8, reload: u16) -> Vec<VClock> {
            let bus = Bus::new(
                vec![Box::new(Pit8253::new())],
                DecodeMap {
                    ports: vec![PortRange {
                        width: bus::PortWidth::default(),
                        ports: 0x40..=0x43,
                        device: 0,
                    }],
                    mmio: Vec::new(),
                },
            );
            let mut machine = Machine::new(
                MockCpu::new(Vec::new()),
                bus,
                Box::<CountingIc>::default(),
                vec![IrqLine {
                    device: 0,
                    output: 0,
                    irq: 0,
                }],
                None,
            )
            .with_scheduler_engine(engine);
            let mut edges = Vec::new();
            for clock in 1..=140000 {
                // Reprogram after a long dormant interval and midway through
                // an active period; no legacy device supplies extra samples.
                if [20, 97, 70000].contains(&clock) {
                    machine
                        .bus_mut()
                        .port_write(0x43, 1, 0x30 | u32::from(mode << 1))
                        .unwrap();
                    machine
                        .bus_mut()
                        .port_write(0x40, 1, u32::from(reload & 255))
                        .unwrap();
                    machine
                        .bus_mut()
                        .port_write(0x40, 1, u32::from(reload >> 8))
                        .unwrap();
                }
                machine.advance_to(machine.clock().advance(1)).unwrap();
                if machine.pic.acknowledge().is_some() {
                    edges.push(machine.clock());
                }
            }
            edges
        }

        for mode in [0, 2, 3] {
            for reload in [0, 1, 2, 3, 5, 8, 1193] {
                let reference = edges(SchedulerEngine::PerClock, mode, reload);
                assert_eq!(
                    reference,
                    edges(SchedulerEngine::Deadline, mode, reload),
                    "mode {mode}, reload {reload}"
                );
                if reload != 1 || mode != 2 {
                    assert!(!reference.is_empty());
                }
            }
        }
    }

    fn halted_machine(program: Vec<CpuStop>, limit: Option<u64>) -> Machine<MockCpu> {
        let bus = Bus::new(
            vec![
                Box::new(LineDevice {
                    level: true,
                    stop_after: Some(9),
                    period: None,
                }),
                Box::<CaptureDevice>::default(),
            ],
            DecodeMap {
                ports: vec![PortRange {
                    width: bus::PortWidth::default(),
                    ports: 0x3F8..=0x3FF,
                    device: 1,
                }],
                mmio: Vec::new(),
            },
        );
        Machine::new(
            instruction_cpu(program, true),
            bus,
            Box::<CountingIc>::default(),
            vec![IrqLine {
                device: 0,
                output: 0,
                irq: 0,
            }],
            limit,
        )
        .with_step_limit(NonZeroU64::MIN)
    }

    #[test]
    fn numeric_error_assertion_samples_irq_without_advancing_clock() {
        let error = devices::CoprocessorErrorLine::default();
        let ack = error.clone();
        let bus = Bus::new(
            vec![Box::new(devices::AtCoprocessor::with_error_line(
                error.clone(),
            ))],
            DecodeMap::default(),
        );
        let mut machine = Machine::new(
            instruction_cpu(vec![CpuStop::NumericErrorAsserted], true),
            bus,
            Box::<CountingIc>::default(),
            vec![IrqLine {
                device: 0,
                output: 0,
                irq: 13,
            }],
            None,
        )
        .with_step_limit(NonZeroU64::MIN)
        .with_cpu_rate(NonZeroU64::new(u64::MAX).unwrap(), NonZeroU64::MIN)
        .with_numeric_error_lines(move || error.assert(), move || ack.take_acknowledgement());
        assert_eq!(
            machine.run_bounded(Some(1)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.pic.poll(), Some(0x08));
        assert_eq!(machine.clock(), VClock::ZERO);
        assert!(machine.cpu().injected.is_empty(), "the cap wins over INTA");
    }

    #[test]
    fn numeric_error_waiting_resumes_for_a_latched_irq_without_idle_ticks() {
        let cpu = MockCpu::new(Vec::new())
            .with_interrupts_enabled(true)
            .with_step_program(vec![
                testkit::MockCpuStep {
                    max_steps: 1,
                    run: cpu::CpuStepRun {
                        steps: 0,
                        stop: CpuStepStop::Exit(CpuStop::NumericErrorWaiting),
                    },
                    retired: 0,
                    irq_ready: true,
                },
                testkit::MockCpuStep {
                    max_steps: 1,
                    run: cpu::CpuStepRun {
                        steps: 1,
                        stop: CpuStepStop::Exit(CpuStop::BudgetExhausted),
                    },
                    retired: 1,
                    irq_ready: true,
                },
            ]);
        let mut machine = Machine::new(
            cpu,
            Bus::new(Vec::new(), DecodeMap::default()),
            Box::<CountingIc>::default(),
            Vec::new(),
            None,
        )
        .with_step_limit(NonZeroU64::MIN);
        assert_eq!(machine.run().unwrap(), RunOutcome::Halted);
        assert!(machine.halted());
        assert_eq!(machine.virtual_time(), 0);
        machine.pic.set_irq_line(13, true);
        assert_eq!(
            machine.run_bounded(Some(1)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert!(!machine.halted());
        assert_eq!(machine.cpu().delivered, [0x08]);
        assert_eq!(machine.virtual_time(), 1);
    }

    #[test]
    fn halt_waits_without_running_cpu_until_irq_delivery() {
        let mut machine = halted_machine(
            vec![
                CpuStop::Halt,
                CpuStop::PortOut {
                    port: 0x3F8,
                    size: 1,
                    value: b'P' as u64,
                },
            ],
            Some(4),
        );
        assert_eq!(
            machine.run_bounded(Some(6)).expect("run"),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.cpu().retired(), 2);
        assert_eq!(machine.clock(), VClock(6));
        assert_eq!(machine.cpu().delivered, vec![0x08]);
        assert_eq!(machine.bus().device(1).expect("capture").save(), b"P");
    }

    #[test]
    fn halt_with_if_clear_stops_without_retiring_post_halt_code() {
        let mut machine = machine_with(vec![CpuStop::Halt, CpuStop::BudgetExhausted]);
        machine.set_idle_limit(Some(100));
        assert_eq!(
            machine.run_bounded(Some(2)).expect("run"),
            RunOutcome::Halted
        );
        assert_eq!(machine.cpu().retired(), 1);
        assert_eq!(machine.clock(), VClock(1));
        // Re-entering the runner must preserve HLT too.
        assert_eq!(
            machine.run_bounded(Some(2)).expect("rerun"),
            RunOutcome::Halted
        );
        assert_eq!(machine.cpu().retired(), 1);
    }

    #[test]
    fn halt_with_pending_irq_and_if_clear_stops_immediately() {
        let mut machine = machine_with(vec![CpuStop::Halt, CpuStop::BudgetExhausted]);
        machine.cpu.inject_irq(0x08);
        machine.set_idle_limit(Some(100));
        assert_eq!(
            machine.run_bounded(Some(2)).expect("run"),
            RunOutcome::Halted
        );
        assert_eq!(machine.cpu().retired(), 1);
        assert!(machine.cpu().has_pending_irq());
        assert!(machine.cpu().delivered.is_empty());
    }

    #[test]
    fn halt_idle_limit_counts_exactly_idle_pit_clocks() {
        for limit in [0, 1, 3] {
            let mut machine = halted_machine(vec![CpuStop::Halt], Some(limit));
            assert_eq!(
                machine.run_bounded(Some(20)).expect("run"),
                RunOutcome::Halted
            );
            assert_eq!(machine.cpu().retired(), 1);
            assert_eq!(machine.clock(), VClock(1 + limit));
            assert!(machine.cpu().injected.is_empty());
        }
    }

    #[test]
    fn halt_idle_counter_resets_after_each_wake() {
        let mut machine = halted_machine(
            vec![
                CpuStop::Halt,
                CpuStop::BudgetExhausted,
                CpuStop::Halt,
                CpuStop::PortOut {
                    port: 0x3F8,
                    size: 1,
                    value: b'P' as u64,
                },
                CpuStop::Halt,
            ],
            Some(4),
        );
        assert_eq!(
            machine.run_bounded(Some(20)).expect("run"),
            RunOutcome::Halted
        );
        assert_eq!(machine.cpu().retired(), 5);
        assert_eq!(machine.clock(), VClock(15));
        assert_eq!(machine.cpu().delivered, vec![0x08, 0x08]);
    }

    #[test]
    fn halt_without_idle_wait_only_resumes_for_an_immediate_irq() {
        let mut machine = halted_machine(vec![CpuStop::Halt, CpuStop::BudgetExhausted], None);
        assert_eq!(
            machine.run_bounded(Some(20)).expect("run"),
            RunOutcome::Halted
        );
        assert_eq!(machine.clock(), VClock(1));
        assert_eq!(
            machine.run_bounded(Some(2)).expect("still halted"),
            RunOutcome::Halted
        );
        assert_eq!(machine.cpu().retired(), 1);
        machine.cpu.inject_irq(0x08);
        assert_eq!(
            machine.run_bounded(Some(2)).expect("wake"),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.cpu().delivered, vec![0x08]);
    }

    #[test]
    fn virtual_time_cap_bounds_a_periodic_halt_loop() {
        let bus = Bus::new(
            vec![Box::new(LineDevice {
                period: Some(65_536),
                ..LineDevice::default()
            })],
            DecodeMap::default(),
        );
        let mut machine = Machine::new(
            instruction_cpu(vec![CpuStop::Halt; 1000], true),
            bus,
            Box::<CountingIc>::default(),
            vec![IrqLine {
                device: 0,
                output: 0,
                irq: 0,
            }],
            Some(100_000),
        )
        .with_step_limit(NonZeroU64::MIN);
        let cap = 1000;
        assert_eq!(
            machine.run_bounded(Some(cap)).expect("run"),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.clock(), VClock(cap));
        assert_eq!(machine.virtual_time(), cap);
        assert_eq!(machine.cpu().retired(), 1);
        assert!(machine.cpu().delivered.is_empty());
    }

    #[test]
    fn halt_idle_counter_persists_across_runner_calls() {
        let mut machine = halted_machine(vec![CpuStop::Halt], Some(3));
        for _ in 0..2 {
            assert_eq!(machine.run_bounded(None).expect("run"), RunOutcome::Halted);
            assert_eq!(machine.clock(), VClock(4));
            assert_eq!(machine.cpu().retired(), 1);
        }
    }

    #[test]
    fn halted_cap_preserves_idle_count_across_runner_calls() {
        let mut machine = halted_machine(vec![CpuStop::Halt], Some(3));
        assert_eq!(
            machine.run_bounded(Some(3)).expect("first chunk"),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.virtual_time(), 3);
        assert_eq!(machine.cpu().retired(), 1);
        assert_eq!(
            machine.run_bounded(Some(3)).expect("same cap"),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.virtual_time(), 3);
        assert_eq!(
            machine.run_bounded(Some(20)).expect("rest"),
            RunOutcome::Halted
        );
        assert_eq!(machine.virtual_time(), 4);
        assert_eq!(machine.cpu().retired(), 1);
    }

    #[test]
    fn halted_reentry_with_if_clear_neither_ticks_nor_acknowledges() {
        for pending in [false, true] {
            let controller = CountingIc::default();
            let acknowledgements = controller.acknowledgements.clone();
            let mut machine =
                halted_machine(vec![CpuStop::Halt, CpuStop::BudgetExhausted], Some(100));
            machine.pic = Box::new(controller);
            machine.cpu = instruction_cpu(vec![CpuStop::Halt, CpuStop::BudgetExhausted], false);
            assert_eq!(
                machine.run_bounded(Some(20)).expect("run"),
                RunOutcome::Halted
            );
            if pending {
                machine.pic.set_irq_line(0, true);
            }
            let before = acknowledgements.get();
            assert_eq!(
                machine.run_bounded(Some(20)).expect("rerun"),
                RunOutcome::Halted
            );
            assert_eq!(machine.clock(), VClock(1));
            assert_eq!(machine.cpu().retired(), 1);
            assert_eq!(acknowledgements.get(), before);
            assert!(machine.cpu().injected.is_empty());
        }
    }

    /// Intel 80286 Programmer's Reference Manual, STI and HLT instruction
    /// descriptions: STI defers the pending interrupt until after HLT (§5.2).
    #[test]
    fn real_mode_sti_halt_delivers_preinjected_vector_without_idle_wait() {
        use cpu::x86jit_backend::{RealSegments, X86JitCpu};
        let mut cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).expect("cpu");
        cpu.map_ram(0, 0x10000).expect("ram");
        // Use DX for the capture port, since an immediate port is only 8 bits.
        cpu.write_mem(0x7C00, &[0xFB, 0xF4, 0xB0, b'P', 0xEE, 0xFA, 0xF4])
            .expect("program");
        cpu.set_reg(cpu::CpuReg::Rdx, 0x3F8);
        cpu.write_mem(0x8000, &[0xB0, b'I', 0xEE, 0xCF])
            .expect("handler");
        cpu.write_mem(0x20, &[0, 0x80, 0, 0]).expect("IVT vector 8");
        cpu.inject_irq(0x08);
        assert!(!cpu.interrupts_enabled());
        let mut machine = Machine::new(cpu, bus_with_capture(), Box::new(NoIc), Vec::new(), None);
        assert_eq!(
            machine.run_bounded(Some(100)).expect("run"),
            RunOutcome::Halted
        );
        assert_eq!(machine.bus().device(0).expect("capture").save(), b"IP");
        assert!(!machine.cpu().has_pending_irq());
    }

    /// Intel 80286 Programmer's Reference Manual, HLT: the interrupt handler
    /// must execute before the instruction following HLT. Intel 8254 datasheet,
    /// Mode 2: counter 0 produces the rising edge that wakes the processor (§5).
    #[test]
    fn real_mode_halt_takes_pit_irq_before_post_halt_out() {
        use cpu::x86jit_backend::{RealSegments, X86JitCpu};
        let mut cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).expect("cpu");
        cpu.map_ram(0, 0x10000).expect("ram");
        // CLI; program PIT mode 2, divisor 64; STI; HLT; OUT marker P; CLI; HLT.
        cpu.write_mem(
            0x7C00,
            &[
                0xFA, 0xB0, 0x34, 0xE6, 0x43, 0xB0, 64, 0xE6, 0x40, 0xB0, 0, 0xE6, 0x40, 0xFB,
                0xF4, 0xB0, b'P', 0xE6, 0xE9, 0xFA, 0xF4,
            ],
        )
        .expect("program");
        cpu.write_mem(0x8000, &[0xB0, b'I', 0xE6, 0xE9, 0xCF])
            .expect("handler");
        cpu.write_mem(0x20, &[0, 0x80, 0, 0]).expect("IVT vector 8");
        let bus = Bus::new(
            vec![
                Box::new(devices::Pit8253::new()),
                Box::<CaptureDevice>::default(),
            ],
            DecodeMap {
                ports: vec![
                    PortRange {
                        width: bus::PortWidth::default(),
                        ports: 0x40..=0x43,
                        device: 0,
                    },
                    PortRange {
                        width: bus::PortWidth::default(),
                        ports: 0xE9..=0xE9,
                        device: 1,
                    },
                ],
                mmio: Vec::new(),
            },
        );
        let mut machine = Machine::new(
            cpu,
            bus,
            Box::<CountingIc>::default(),
            vec![IrqLine {
                device: 0,
                output: 0,
                irq: 0,
            }],
            Some(128),
        );
        let outcome = machine.run_bounded(Some(1000)).expect("run");
        assert_eq!(machine.bus().device(1).expect("capture").save(), b"IP");
        assert_eq!(outcome, RunOutcome::Halted);
    }

    /// A minimal CPU backend with a flat guest-memory buffer, so the DMA seam can
    /// be exercised end to end without x86jit. The DMA path reads and writes guest
    /// RAM through the `guest_dma_*` methods, which back onto this buffer.
    struct MemCpu {
        mem: Vec<u8>,
    }

    impl MemCpu {
        fn new(size: usize) -> Self {
            Self { mem: vec![0; size] }
        }

        fn seed(&mut self, addr: u32, bytes: &[u8]) {
            let at = addr as usize;
            self.mem[at..at + bytes.len()].copy_from_slice(bytes);
        }
    }

    impl Cpu for MemCpu {
        fn run(&mut self, _budget: Option<u64>) -> Result<CpuStop, CpuError> {
            Ok(CpuStop::Halt)
        }
        fn interrupts_enabled(&self) -> bool {
            false
        }
        fn reg(&self, _reg: cpu::CpuReg) -> u64 {
            0
        }
        fn set_reg(&mut self, _reg: cpu::CpuReg, _value: u64) {}
        fn complete_port_in(&mut self, _value: u64) {}
        fn complete_mmio_read(&mut self, _value: u64) {}
        fn complete_mmio_write(&mut self) {}
        fn guest_dma_read(&self, addr: u32) -> cpu::DmaAccess<u8> {
            cpu::DmaAccess::Complete(self.mem[addr as usize])
        }
        fn guest_dma_write(&mut self, addr: u32, value: u8) -> cpu::DmaAccess<()> {
            self.mem[addr as usize] = value;
            cpu::DmaAccess::Complete(())
        }
        fn retired(&self) -> u64 {
            0
        }
    }

    /// A bus master that queues a single memory→device transfer and captures the
    /// outcome the machine hands back, so a test can assert the seam delivered the
    /// right bytes without inspecting a concrete controller.
    #[derive(Default)]
    struct FromMemoryRequester {
        pending: Option<DmaRequest>,
        received: Option<DmaOutcome>,
    }

    impl Device for FromMemoryRequester {
        fn name(&self) -> &'static str {
            "from-memory-requester"
        }
        fn port_read(&mut self, _port: u16, _size: u8) -> Result<u32, DeviceError> {
            Ok(0)
        }
        fn port_write(&mut self, _port: u16, _size: u8, _value: u32) -> Result<(), DeviceError> {
            Ok(())
        }
        fn mmio_read(&mut self, addr: u64, _size: u8) -> Result<u64, DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            })
        }
        fn mmio_write(&mut self, addr: u64, _size: u8, _value: u64) -> Result<(), DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            })
        }
        fn tick(&mut self, _now: VClock) {}
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {}
        fn save(&self) -> Vec<u8> {
            Vec::new()
        }
        fn restore(&mut self, _snapshot: &[u8]) -> Result<(), DeviceError> {
            Ok(())
        }
        fn take_dma_request(&mut self) -> Option<DmaRequest> {
            self.pending.take()
        }
        fn dma_complete(&mut self, outcome: DmaOutcome) {
            self.received = Some(outcome);
        }
        fn as_any_mut(&mut self) -> Option<&mut dyn core::any::Any> {
            Some(self)
        }
    }

    /// DMA channel the mock requester runs on (the PC floppy channel).
    const READ_CHANNEL: u8 = 2;

    /// Build a real 8237 programmed for a memory→device (read) transfer on
    /// channel 2 at physical `address`, transferring `count + 1` bytes.
    fn programmed_read_dma(address: u16, count: u16) -> devices::Dma8237 {
        let mut dma = devices::Dma8237::new();
        dma.port_write(0x0A, 1, 0x06).unwrap(); // mask channel 2
        dma.port_write(0x0C, 1, 0).unwrap(); // clear flip-flop
        dma.port_write(0x0B, 1, 0x4A).unwrap(); // mode: ch2, read, single
        dma.port_write(0x04, 1, u32::from(address & 0xFF)).unwrap(); // addr low
        dma.port_write(0x04, 1, u32::from(address >> 8)).unwrap(); // addr high
        dma.port_write(0x81, 1, 0x00).unwrap(); // page latch (channel 2)
        dma.port_write(0x05, 1, u32::from(count & 0xFF)).unwrap(); // count low
        dma.port_write(0x05, 1, u32::from(count >> 8)).unwrap(); // count high
        dma.port_write(0x0A, 1, 0x02).unwrap(); // unmask channel 2
        dma
    }

    /// Assemble a machine whose bus holds the mock requester (index 0) and the
    /// programmed 8237 (index 1), wired together, over `cpu`'s guest memory.
    fn dma_machine(
        cpu: MemCpu,
        requester: FromMemoryRequester,
        dma: devices::Dma8237,
    ) -> Machine<MemCpu> {
        let devices: Vec<Box<dyn Device>> = vec![Box::new(requester), Box::new(dma)];
        let map = DecodeMap {
            ports: Vec::new(),
            mmio: Vec::new(),
        };
        Machine::new(
            cpu,
            Bus::new(devices, map),
            Box::new(NoIc),
            Vec::new(),
            Some(1),
        )
        .with_dma_wirings(vec![DmaWiring {
            requester: 0,
            controller: 1,
        }])
    }

    #[test]
    fn channel_routes_translate_only_the_selected_requester_and_system_channel() {
        use std::{cell::RefCell, rc::Rc};
        struct ControllerProbe {
            bank: u8,
            calls: Rc<RefCell<Vec<(u8, u8)>>>,
        }
        impl Device for ControllerProbe {
            fn name(&self) -> &'static str {
                "dma-controller-probe"
            }
            fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
                Ok(0)
            }
            fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
                Ok(())
            }
            fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
                Ok(0)
            }
            fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), DeviceError> {
                Ok(())
            }
            fn tick(&mut self, _: VClock) {}
            fn irq_pending(&self) -> bool {
                false
            }
            fn reset(&mut self) {}
            fn save(&self) -> Vec<u8> {
                Vec::new()
            }
            fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
                Ok(())
            }
            fn dma_service_read(
                &mut self,
                channel: u8,
                _: &mut dyn DmaMemory,
                bytes: &mut [u8],
            ) -> device::DmaTransfer {
                self.calls.borrow_mut().push((self.bank, channel));
                bytes.fill(0x55);
                device::DmaTransfer {
                    transferred: bytes.len(),
                    terminal_count: true,
                }
            }
        }
        for (requester, channel, expected) in [(0, 5, (1, 1)), (0, 2, (0, 2)), (1, 5, (0, 5))] {
            let calls = Rc::new(RefCell::new(Vec::new()));
            let mut devices: Vec<Box<dyn Device>> = (0..2)
                .map(|i| {
                    Box::new(FromMemoryRequester {
                        pending: (i == requester).then_some(DmaRequest {
                            channel,
                            op: DmaOp::FromMemory { len: 2 },
                        }),
                        received: None,
                    }) as Box<dyn Device>
                })
                .collect();
            for bank in 0..2 {
                devices.push(Box::new(ControllerProbe {
                    bank,
                    calls: calls.clone(),
                }));
            }
            let bus = Bus::new(
                devices,
                DecodeMap {
                    ports: Vec::new(),
                    mmio: Vec::new(),
                },
            );
            let mut machine =
                Machine::new(MemCpu::new(16), bus, Box::new(NoIc), Vec::new(), Some(1))
                    .with_dma_wirings(vec![
                        DmaWiring {
                            requester: 0,
                            controller: 2,
                        },
                        DmaWiring {
                            requester: 1,
                            controller: 2,
                        },
                    ])
                    .with_dma_channel_wirings(vec![DmaChannelWiring {
                        requester: 0,
                        system_channel: 5,
                        controller: 3,
                        local_channel: 1,
                    }]);
            machine.service_dma().unwrap();
            assert_eq!(*calls.borrow(), [expected]);
        }
    }

    /// Read the outcome the requester captured after a serviced transfer.
    fn captured_outcome<C: Cpu>(machine: &mut Machine<C>) -> DmaOutcome {
        machine
            .bus
            .device_mut(0)
            .and_then(|dev| dev.as_any_mut())
            .and_then(|any| any.downcast_mut::<FromMemoryRequester>())
            .and_then(|req| req.received.take())
            .expect("the requester received a completion")
    }

    /// A byte-wide memory-mapped device with observable read side effects.
    struct DmaMmioDevice {
        bytes: [u8; 2],
        reads: u8,
    }

    impl Device for DmaMmioDevice {
        fn name(&self) -> &'static str {
            "dma-mmio"
        }
        fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
            Ok(0)
        }
        fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
            Ok(())
        }
        fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, DeviceError> {
            assert_eq!(size, 1);
            self.reads += 1;
            Ok(u64::from(self.bytes[(addr - 0xB8000) as usize]))
        }
        fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError> {
            assert_eq!(size, 1);
            self.bytes[(addr - 0xB8000) as usize] = value as u8;
            Ok(())
        }
        fn tick(&mut self, _: VClock) {}
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {}
        fn save(&self) -> Vec<u8> {
            vec![self.bytes[0], self.bytes[1], self.reads]
        }
        fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
            Ok(())
        }
    }

    fn mapped_dma_machine(op: DmaOp, count: u16) -> Machine<cpu::X86JitCpu> {
        use cpu::x86jit_backend::RealSegments;
        let mut cpu = cpu::X86JitCpu::new_real16_bare(0x10_0000, 0, 0x7C00, RealSegments::flat(0))
            .expect("cpu");
        cpu.map_ram(0xB7FFE, 2).expect("ram before mmio");
        cpu.map_mmio(0xB8000, 2).expect("mmio");
        cpu.map_rom(0xB8002, &[0xA5, 0x5A]).expect("rom");
        cpu.map_ram(0xB8004, 2).expect("ram after rom");
        cpu.write_mem(0xB7FFE, &[0x11, 0x22]).expect("seed ram");
        cpu.write_mem(0xB8000, &[0xEE, 0xEE])
            .expect("seed mmio backing");
        cpu.write_mem(0xB8004, &[0x33, 0x44]).expect("seed ram");
        let mut dma = programmed_read_dma(0x7FFE, count);
        dma.port_write(0x81, 1, 0x0B).expect("page");
        if matches!(op, DmaOp::ToMemory(_)) {
            dma.port_write(0x0B, 1, 0x46).expect("write mode");
        }
        let requester = FromMemoryRequester {
            pending: Some(DmaRequest {
                channel: READ_CHANNEL,
                op,
            }),
            received: None,
        };
        let bus = Bus::new(
            vec![
                Box::new(requester),
                Box::new(dma),
                Box::new(DmaMmioDevice {
                    bytes: [0x66, 0x77],
                    reads: 0,
                }),
            ],
            DecodeMap {
                ports: Vec::new(),
                mmio: vec![bus::MmioRange {
                    addrs: 0xB8000..=0xB8001,
                    device: 2,
                }],
            },
        );
        Machine::new(cpu, bus, Box::new(NoIc), Vec::new(), Some(1)).with_dma_wirings(vec![
            DmaWiring {
                requester: 0,
                controller: 1,
            },
        ])
    }

    #[test]
    fn service_dma_write_routes_mmio_and_preserves_rom_across_boundaries() {
        let mut machine = mapped_dma_machine(DmaOp::ToMemory(vec![1, 2, 3, 4, 5, 6, 7, 8]), 6);
        machine.service_dma().expect("dma");
        assert_eq!(
            machine.bus().device(2).expect("mmio device").save(),
            vec![3, 4, 0]
        );
        let mut bytes = [0; 2];
        machine
            .cpu()
            .read_mem(0xB8000, &mut bytes)
            .expect("backing");
        assert_eq!(bytes, [0xEE, 0xEE]);
        machine.cpu().read_mem(0xB8002, &mut bytes).expect("rom");
        assert_eq!(bytes, [0xA5, 0x5A]);
        machine.cpu().read_mem(0xB7FFE, &mut bytes).expect("ram");
        assert_eq!(bytes, [1, 2]);
        machine.cpu().read_mem(0xB8004, &mut bytes).expect("ram");
        assert_eq!(bytes, [7, 0x44]);
        assert!(matches!(
            captured_outcome(&mut machine),
            DmaOutcome::ToMemory {
                transferred: 7,
                terminal_count: true
            }
        ));
    }

    #[test]
    fn service_dma_read_routes_mmio_and_reads_rom_across_boundaries() {
        let mut machine = mapped_dma_machine(DmaOp::FromMemory { len: 8 }, 6);
        machine.service_dma().expect("dma");
        assert_eq!(
            machine.bus().device(2).expect("mmio device").save(),
            vec![0x66, 0x77, 2]
        );
        match captured_outcome(&mut machine) {
            DmaOutcome::FromMemory {
                bytes,
                terminal_count,
            } => {
                assert!(terminal_count);
                assert_eq!(bytes, [0x11, 0x22, 0x66, 0x77, 0xA5, 0x5A, 0x33]);
            }
            other => panic!("expected memory read, got {other:?}"),
        }
    }

    #[test]
    fn service_dma_from_memory_delivers_read_bytes() {
        const ADDRESS: u16 = 0x0010;
        let pattern: Vec<u8> = (0..8u8).map(|i| 0xB0 ^ i).collect();

        let mut cpu = MemCpu::new(0x1000);
        cpu.seed(u32::from(ADDRESS), &pattern);

        // count+1 == pattern.len() so the whole request transfers.
        let dma = programmed_read_dma(ADDRESS, (pattern.len() - 1) as u16);
        let requester = FromMemoryRequester {
            pending: Some(DmaRequest {
                channel: READ_CHANNEL,
                op: DmaOp::FromMemory { len: pattern.len() },
            }),
            received: None,
        };

        let mut machine = dma_machine(cpu, requester, dma);
        machine.service_dma().expect("dma");

        match captured_outcome(&mut machine) {
            DmaOutcome::FromMemory {
                bytes,
                terminal_count,
            } => {
                assert!(terminal_count);
                assert_eq!(
                    bytes, pattern,
                    "the requester gets exactly the seeded bytes"
                );
            }
            other => panic!("expected a FromMemory outcome, got {other:?}"),
        }
    }

    #[test]
    fn service_dma_from_memory_truncates_at_terminal_count() {
        const ADDRESS: u16 = 0x0010;
        let pattern: Vec<u8> = (0..8u8).map(|i| 0xC0 ^ i).collect();
        // The requester asks for 8 bytes but the channel is programmed for only 4
        // (count 3 -> 4 bytes), so terminal count cuts the transfer short.
        const REQUESTED: usize = 8;
        const AVAILABLE: usize = 4;

        let mut cpu = MemCpu::new(0x1000);
        cpu.seed(u32::from(ADDRESS), &pattern);

        let dma = programmed_read_dma(ADDRESS, (AVAILABLE - 1) as u16);
        let requester = FromMemoryRequester {
            pending: Some(DmaRequest {
                channel: READ_CHANNEL,
                op: DmaOp::FromMemory { len: REQUESTED },
            }),
            received: None,
        };

        let mut machine = dma_machine(cpu, requester, dma);
        machine.service_dma().expect("dma");

        match captured_outcome(&mut machine) {
            DmaOutcome::FromMemory {
                bytes,
                terminal_count,
            } => {
                assert!(terminal_count);
                assert_eq!(
                    bytes,
                    pattern[..AVAILABLE],
                    "the buffer is truncated to what the channel transferred; no uninitialised tail"
                );
            }
            other => panic!("expected a FromMemory outcome, got {other:?}"),
        }
    }
    #[test]
    fn clock_domain_machine_active_halted_and_legacy_deadlines() {
        let mut machine = machine_with(vec![CpuStop::Halt]);
        machine.charge_steps(7).unwrap();
        assert_eq!(machine.clock(), VClock::from_pit(7));
        assert_eq!(machine.next_deadline().unwrap(), Some(PIT.edge_at(8)));
        machine.set_idle_limit(Some(10));
        machine.halted = true;
        machine.cpu = instruction_cpu(vec![CpuStop::Halt], true);
        assert_eq!(
            machine.run_bounded(Some(12)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.clock(), VClock::from_pit(12));
        assert_eq!(machine.virtual_time(), 12);
    }

    #[test]
    fn clock_domain_legacy_deadline_uses_next_pit_input_edge() {
        let mut machine = machine_with(vec![CpuStop::Halt]);
        machine.clock = VClock::from_pit(7);
        assert_eq!(machine.next_deadline().unwrap(), Some(PIT.edge_at(8)));
    }
    #[test]
    fn clock_domain_halted_limits_and_caps_use_pit_durations() {
        for (idle, cap, end) in [(10, 12, 10), (20, 12, 12)] {
            let mut machine = machine_with(vec![CpuStop::Halt]);
            machine.cpu = instruction_cpu(vec![CpuStop::Halt], true);
            machine.set_idle_limit(Some(idle));
            machine.halted = true;
            machine.run_bounded(Some(cap)).unwrap();
            assert_eq!(machine.clock(), VClock::from_pit(end));
            assert_eq!(machine.virtual_time(), end);
        }
    }
    #[test]
    fn clock_domain_active_product_overflow_is_a_scheduler_error() {
        let mut machine = machine_with(Vec::new()).with_cpu_rate(
            NonZeroU64::new(u64::MAX).unwrap(),
            NonZeroU64::new(u64::MAX).unwrap(),
        );
        machine.bus = Bus::new(
            Vec::new(),
            DecodeMap {
                ports: Vec::new(),
                mmio: Vec::new(),
            },
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            machine.charge_steps(u64::MAX)
        }));
        assert!(result.is_ok(), "clock arithmetic must not panic");
        if TICKS_PER_PIT_CLOCK > 1 {
            assert!(
                result.unwrap().is_err(),
                "an unrepresentable interval must be rejected"
            );
        } else {
            assert!(result.unwrap().is_ok());
        }
    }
    #[test]
    fn clock_domain_halted_product_overflow_is_a_scheduler_error() {
        let mut machine = machine_with(Vec::new())
            .with_cpu_rate(NonZeroU64::MIN, NonZeroU64::new(u64::MAX).unwrap());
        machine.bus = Bus::new(
            Vec::new(),
            DecodeMap {
                ports: Vec::new(),
                mmio: Vec::new(),
            },
        );
        machine.cpu = instruction_cpu(vec![CpuStop::Halt], true);
        machine.halted = true;
        machine.set_idle_limit(Some(u64::MAX));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            machine.run_bounded(Some(u64::MAX))
        }));
        assert!(result.is_ok(), "halted clock arithmetic must not panic");
        if TICKS_PER_PIT_CLOCK > 1 {
            assert!(
                result.unwrap().is_err(),
                "an unrepresentable halted interval must be rejected"
            );
        } else {
            assert!(result.unwrap().is_ok());
        }
    }
}

#[cfg(test)]
mod corrected_tests;

#[cfg(test)]
mod a20_tests;
