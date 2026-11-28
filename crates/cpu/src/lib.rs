// SPDX-License-Identifier: AGPL-3.0-or-later
//! CPU swap seam (§2, §3.2 seam 1).
//!
//! [`Cpu`] is the first of the six swap seams from
//! `backlog/docs/architecture.md`: it abstracts the CPU core so the `machine`
//! composition root never names a concrete backend. The only backend is
//! [`X86JitCpu`], wrapping the `x86jit-core` recompiler; it runs in
//! **I286 machine** mode for IBM PC/AT machines and **I386 machine** mode for
//! generic ISA 386 boards. **Real16** and **Long64**
//! remain available for standalone adapter tests; assembly uses exact steps.
//!
//! The backend exploits the integration surface `x86jit` exposes: `Exit::PortIo`
//! with `complete_port_in` for ISA port I/O, and `RegionKind::Trap` MMIO (§2.1).
//! In real mode it additionally forwards hardware IRQ injection to the core
//! ([`Cpu::inject_irq`]), which delivers the vector in-guest through the IVT, and
//! exposes completed steps (including REP iterations) for virtual time (§6),
//! alongside whole-instruction retirement counts for diagnostics. REP MOVS/STOS
//! and REP INS/OUTS all charge one step per element, or one for an empty REP.
//! Each string port element exits once; input is charged at handoff, and its
//! deferred ES store adds no step. A nonempty REP port instruction retires once,
//! at its final handoff.
//! This is an engine time convention, not CPU-cycle timing (Intel 80286
//! Programmer's Reference Manual, REP, INS and OUTS instruction descriptions).
//! See `backlog/docs/design/machine-time-profile.md` §String instruction accounting.
//!
//! Warm restart follows Intel 80286 Programmer's Reference Manual, §5.3 System
//! Initialization, Table 5-3 (Processor State after RESET): CS=F000h, IP=FFF0h,
//! FLAGS=0002h and DS=ES=SS=0. General registers are unspecified by that table;
//! this model preserves them. The hardware's hidden CS base FF0000h lasts until
//! a far transfer. I286 restores that base, MSW=FFF0h and IDTR base=0,
//! limit=03FFh (§10.4 Initialization), replacing protected segment caches.
//! The board independently maps its complete FE0000h–FFFFFFh ROM alias.
//! Application Real16 retains its low FFFF0h fetch. Reset first abandons
//! engine-private execution (including pending
//! INS/MMIO and IRQs), then
//! installs these registers, with FNINIT for the core's emulated x87 environment.
//! RAM, mappings, retirement and completed-step accounting survive. The AT's
//! CPU-only reset wiring and retained board A20 line are described in
//! `backlog/docs/design/machine-config.md` §Board model. The A20 gate clears
//! physical bit 20 when disabled (IBM PC AT Technical Reference, Keyboard
//! Controller, Output Port). The machine profile honours it for fetch and CPU
//! data; application profiles ignore it. Source-free bare rigs keep flat
//! Application(Real16); attaching an 8042 or port 92h selects I80286 Machine
//! with a low reset entry. Host memory APIs and ISA DMA bypass the CPU gate.
//! CPU reset retains both an effective line and a pending boundary change.
//!
//! Intel 80386 Programmer's Reference Manual, §10.1 Initialization: the 386
//! reset image uses EIP=FFF0h, hidden CS base FFFF0000h and DH=03h. The core
//! assumes DL=03h and derives CR0.ET from the FPU model. Installing that image
//! precedes reapplying the retained board A20 line; at this bare adapter,
//! A20 disabled makes the reset fetch address FFEFFFF0h, which is unmapped.
//! The assembled board overrides CPU A20 until the first CS reload as an
//! explicit emulator policy, then restores the board gate.
//! See `backlog/docs/design/machine-files.md` §Generic 386 ISA board.

#![forbid(unsafe_code)]

pub mod delivery;
pub mod device_access;
pub mod diagnostic;
pub mod stop_report;
pub mod x86jit_backend;

pub use delivery::{
    Delivery, DeliveryCounts, DeliveryKind, DeliveryLogError, DeliveryName, DeliveryRegisters,
};
pub use device_access::{
    AccessControl, AccessResult, AccessSpace, DeviceAccess, DeviceAccessHandler, ServedRange,
};
pub use diagnostic::CpuDiagnostic;
pub use stop_report::CpuStopReport;
pub use x86jit_backend::X86JitCpu;

use thiserror::Error;

/// How the host executes guest instructions: the CPU engine.
///
/// A host choice, not modelled hardware: both engines retire the same guest
/// instructions in the same order, so virtual time, device timing and every
/// guest-visible result are identical (`backlog/docs/design/machine-files.md`
/// §CPU engine). The JIT runs a backend the embedder supplies
/// ([`JitBackendFactory`]); it compiles hot flat 32-bit code on that backend's
/// compile thread, and everything else, including all 16-bit code, still runs
/// on the interpreter.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CpuEngine {
    /// The `x86jit` interpreter, the default.
    #[default]
    Interpreter,
    /// The `x86jit` Cranelift JIT with background tier-up.
    Jit,
}

impl CpuEngine {
    /// Every engine, in schema order.
    pub const ALL: [Self; 2] = [Self::Interpreter, Self::Jit];

    /// The engine's spelling in machine files and on the command line.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Interpreter => "interpreter",
            Self::Jit => "jit",
        }
    }

    /// The engine a [`Self::name`] spelling selects.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|engine| engine.name() == name)
    }
}

/// Builds the JIT backend for [`CpuEngine::Jit`]; supplied by the embedder.
///
/// The core links no code generator and no executable-memory code of its
/// own: a host program that offers the JIT passes a function creating the
/// CPU core's backend (`x86jit-cranelift`'s `JitBackend`, say). See
/// `backlog/docs/design/core-contract.md` §12.7.
#[derive(Clone, Copy)]
pub struct JitBackendFactory(pub fn() -> Box<dyn x86jit_core::Backend>);

impl std::fmt::Debug for JitBackendFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JitBackendFactory")
    }
}

/// Guest registers the [`Cpu`] seam exposes (§3.1).
///
/// A minimal, backend-neutral subset; it grows with real-mode work (segment
/// registers, flags) later. Values are the full 64-bit register width.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CpuReg {
    /// Accumulator.
    Rax,
    /// Counter.
    Rcx,
    /// Data.
    Rdx,
    /// Base.
    Rbx,
    /// Stack pointer.
    Rsp,
    /// Base pointer.
    Rbp,
    /// Source index.
    Rsi,
    /// Destination index.
    Rdi,
    /// Instruction pointer.
    Rip,
}

/// Why [`Cpu::run`] or [`Cpu::run_steps`] returned control to the machine loop.
///
/// The backend-neutral counterpart of `x86jit`'s `Exit` (§5.2). The `machine`
/// loop matches on this and dispatches to the bus, never seeing the underlying
/// core's own exit type.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CpuStop {
    /// Guest executed `in` or one `ins` element — supply the port value via
    /// [`Cpu::complete_port_in`] before the next [`Cpu::run`].
    PortIn {
        /// 16-bit port number.
        port: u16,
        /// Access width in bytes (1/2/4).
        size: u8,
    },
    /// Guest executed `out` or one `outs` element — `value` holds the low
    /// `size` bytes written.
    PortOut {
        /// 16-bit port number.
        port: u16,
        /// Access width in bytes (1/2/4).
        size: u8,
        /// The value the guest wrote (low `size` bytes).
        value: u64,
    },
    /// Guest read a trapped (MMIO) address — supply the value via
    /// [`Cpu::complete_mmio_read`] before the next run.
    MmioRead {
        /// Guest physical address.
        addr: u64,
        /// Access width in bytes.
        size: u8,
    },
    /// Guest wrote a trapped (MMIO) address.
    MmioWrite {
        /// Guest physical address.
        addr: u64,
        /// Access width in bytes.
        size: u8,
        /// The value the guest wrote.
        value: u64,
    },
    /// The coprocessor asserted ERROR# after a committed instruction.
    /// IBM PC AT Technical Reference, System Board, Coprocessor: board logic
    /// latches this signal onto IRQ13. See machine-config.md §Coprocessor controls.
    NumericErrorAsserted,
    /// WAIT or a waiting x87 instruction is blocked by the board BUSY latch.
    /// Nothing retired; IRQ arbitration remains available on the next run.
    NumericErrorWaiting,
    /// Guest executed `hlt`.
    Halt,
    /// The processor faulted while delivering double fault and requests reset.
    /// No instruction is retired by the shutdown cycle (Intel 80286 PRM §9.6.2).
    Shutdown,
    /// A guest CPU exception (`#DE`, `#UD`, `#BP`, …); `vector` is the x86
    /// exception vector (§5.2).
    Exception {
        /// x86 exception vector.
        vector: u8,
    },
    /// The run budget was exhausted: blocks for [`Cpu::run`], steps for
    /// [`Cpu::run_steps`] — a cooperative yield (§6.2).
    BudgetExhausted,
}

/// Result of a step-bounded CPU run (§6).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CpuStepRun {
    /// Completed scheduling units in this call; see [`Cpu::run_steps`].
    pub steps: u64,
    /// Why execution returned to the embedder.
    pub stop: CpuStepStop,
}

/// Optional boundary stops for step-bounded execution (§6).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StepRunOptions {
    /// Stop at the first completed boundary where [`Cpu::irq_ready`] is true.
    /// An already-ready CPU returns zero steps; queued IRQ delivery comes first.
    pub stop_when_irq_ready: bool,
}

/// Why a step-bounded run returned (§6).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CpuStepStop {
    /// An ordinary CPU exit, including step-budget exhaustion.
    Exit(CpuStop),
    /// A completed boundary where a maskable IRQ can be accepted.
    /// No instruction is in flight and no vector has been acknowledged.
    IrqReady,
}

/// Errors from the [`Cpu`] seam (§2).
#[derive(Debug, Error)]
pub enum CpuError {
    /// A memory operation on the guest address space failed (mapping/write).
    #[error("guest memory error: {0}")]
    Memory(String),
    /// The guest attempted an unmapped access the machine could not service.
    #[error("unmapped guest access at {addr:#x}")]
    Unmapped {
        /// Faulting guest address.
        addr: u64,
    },
    /// The core hit an instruction it does not lift. The message carries the
    /// visible code selector and the instruction bytes, because a bare offset
    /// cannot tell a missing opcode from a jump into data.
    #[error("unsupported instruction at {cs:04x}:{addr:04x}{}", instruction_bytes(.bytes))]
    UnsupportedInstruction {
        /// Guest RIP of the offending instruction (the IP in real mode).
        addr: u64,
        /// Visible code selector when the core stopped.
        cs: u16,
        /// The instruction bytes the core decoded; empty when the core did
        /// not report them.
        bytes: Vec<u8>,
    },
    /// An exception explicitly returned outward by the machine CPU profile.
    #[error("cpu fault vector {vector:#x}, error code {error_code:?} at {cs:04x}:{ip:04x}")]
    Fault {
        /// Architectural exception vector.
        vector: u8,
        /// Architectural error code, absent for exceptions without one.
        error_code: Option<u32>,
        /// Visible code selector at the fault.
        cs: u16,
        /// Restart or trap instruction pointer supplied by the core.
        ip: u64,
    },
    /// A system operation outside the CPU core's supported execution scope.
    #[error("unsupported cpu system operation {operation} at {cs:04x}:{ip:04x}")]
    UnsupportedSystem {
        /// Core operation name, without guest bytes.
        operation: String,
        /// Visible code selector.
        cs: u16,
        /// Instruction pointer of the refused operation.
        ip: u64,
    },
    /// The core stopped for a reason the machine has no handling for (a host
    /// yield request, exhausted JIT code space, or an exit kind newer than this
    /// backend), or a refused step-bounded call. Kept apart from
    /// [`CpuError::UnsupportedInstruction`] so such a stop is never mistaken for
    /// a missing opcode.
    #[error("unhandled cpu exit: {what}")]
    UnexpectedExit {
        /// Which exit the core returned, with its details.
        what: String,
    },
    /// The processor kept delivering exceptions and retired no instruction
    /// between them, as when a handler's first instruction faults again, and
    /// it neither retired one nor shut down within the backend's bound. A
    /// completed delivery is no step, so a step-bounded run would never reach
    /// its budget, and virtual time would stand still. The core returns such
    /// a run every 1024 deliveries; the backend runs on through 11 of those
    /// returns in one call (`x86jit_backend::STALL_RETURNS`). On the 80286
    /// each delivery pushes a frame of at least 6 bytes. In real mode the
    /// frame wraps inside the segment and faults only where a word sits at
    /// offset FFFFh, which from an odd SP happens within the bound; in
    /// protected mode the frame must fit, and when SP is not a multiple of
    /// the frame size, or the stack's limit is below FFFFh, the frames
    /// overrun the bottom of the stack within the bound. The fault in that
    /// delivery escalates to a double fault and then to shutdown, and the
    /// run ends in [`CpuStop::Shutdown`], the stop the AT board answers with
    /// a reset (Intel 80286 Programmer's Reference Manual, §9.6.2 and
    /// Appendix B, INT). From an even SP in real mode, or one that is a
    /// multiple of the frame size in protected mode, a 64 KiB stack wraps
    /// without a fault and the loop has no end; only that stall is
    /// reported, with the deliveries it took. `cs:ip` is where the last
    /// delivery went: the handler's first instruction, which the CPU still
    /// holds, so the stop report shows its bytes. See
    /// `backlog/docs/design/observation.md` §CPU stop report.
    #[error(
        "the processor is stuck delivering exceptions: {deliveries} deliveries \
         without an instruction, the last to {cs:04x}:{ip:04x}"
    )]
    DeliveryStalled {
        /// Consecutive completed deliveries with no instruction retired.
        deliveries: u32,
        /// Code selector of the last delivery's handler.
        cs: u16,
        /// Instruction pointer of the last delivery's handler.
        ip: u64,
    },
    /// The processor was reset again and again with no progress in between:
    /// [`RESET_STORM_RESETS`] reset requests in a row, each fewer than
    /// [`RESET_STORM_STEPS`] steps of virtual time after the one before, as
    /// when a guest shuts down again at once after every reset. The CPU
    /// backend does not raise it; the machine does, when it counts the
    /// requests (shutdowns and the board's reset line alike), because a
    /// reset takes no virtual time, and a loop that makes none either would
    /// never reach a run's cap. `cs:ip` is where the CPU stood at the last
    /// request, before that reset, so the stop report shows the instruction
    /// that shut down. See `backlog/docs/design/observation.md` §The machine
    /// resets again and again.
    #[error(
        "the machine resets again and again: {resets} resets within {steps} \
         steps, the last {} at {cs:04x}:{ip:04x}",
        if *.shutdown { "shutdown" } else { "reset request" }
    )]
    ResetStorm {
        /// Consecutive reset requests, each soon after the one before.
        resets: u32,
        /// Virtual time over those resets, in steps, from the reset before
        /// the first of them to the last.
        steps: u64,
        /// Whether the last request was a processor shutdown rather than
        /// the board's reset line.
        shutdown: bool,
        /// Code selector at the last request.
        cs: u16,
        /// Instruction pointer at the last request.
        ip: u64,
    },
}

/// Reset requests in a row, each soon after the one before, after which the
/// machine stops with [`CpuError::ResetStorm`].
pub const RESET_STORM_RESETS: u32 = 64;

/// Fewer steps of virtual time than this between two reset requests count
/// toward [`CpuError::ResetStorm`]: virtual time is the run cap's own unit
/// (completed steps, each REP iteration one, plus halted time), so a loop
/// below it makes essentially no progress. Any bound of at least one keeps a
/// run's host work bounded; a small one only diagnoses such loops and never
/// stops a guest that switches modes through resets. MEASURED on the open
/// AT protected-mode test with the open AT BIOS: its shortest distance
/// between two resets is 855 steps (an INT 15h block move, whose return to
/// real mode is a reset), its rounds take 3,650 to 10,047, and POST about
/// 3.6 million; a guest that shuts down right after the reset vector's far
/// jump takes 3.
pub const RESET_STORM_STEPS: u64 = 16;

/// The " (bytes 0f 0b)" tail of an unsupported-instruction message, or
/// nothing when the core reported no bytes.
fn instruction_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(" (bytes {})", hex.join(" "))
}

/// Result of one DMA byte cycle in the guest physical memory map (§4.3).
///
/// Intel 8237A datasheet, Pin Description (MEMR/MEMW): DMA drives memory bus
/// cycles independently of CPU instruction execution.
/// The CPU backend has no devices: a trapped address must be dispatched by the
/// machine through its MMIO bus before completing the request to the bus master.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum DmaAccess<T> {
    /// Memory serviced the cycle, including a dropped ROM or unmapped write.
    Complete(T),
    /// A trapped window requires an MMIO byte cycle at the same address.
    Mmio,
}

/// The CPU swap seam (§3.2 seam 1).
///
/// A backend implements step-bounded execution ([`Cpu::run_steps`]), register
/// access, port/MMIO completion handshakes, the IRQ-injection contract (§5.2),
/// and completed-step accounting for virtual time (§6).
pub trait Cpu {
    /// Raw x87 error level, independent of the board IRQ latch (Intel 80286
    /// and 80287 PRM, Part II, NPX Status Word, ES). See machine-config.md
    /// §Coprocessor controls. Backends without a coprocessor report false.
    fn numeric_error_pending(&self) -> bool {
        false
    }

    /// Release the external error wait after the board's F0h write, preserving
    /// x87 status. IBM PC AT Technical Reference, NMI and Coprocessor Controls;
    /// see machine-config.md §Coprocessor controls.
    fn ack_external_numeric_error(&mut self) -> Result<(), CpuError> {
        Ok(())
    }

    /// Drive the board's A20 gate on every vCPU owned by this backend.
    ///
    /// IBM PC AT Technical Reference, Keyboard Controller, Output Port: a low
    /// gate clears physical address bit 20. Apply at the next architectural
    /// interruption point; a completed OUT allows immediate application.
    /// CPU reset preserves the line, including a pending request. DMA and host
    /// physical-memory access bypass it. See
    /// `backlog/docs/design/machine-config.md` §A20 line and System Control Port A.
    /// Backends without a gate retain flat addressing by default.
    fn set_a20_enabled(&mut self, _enabled: bool) {}

    /// Effective A20 gate, excluding a change waiting for an in-flight access.
    /// Backends without a gate report enabled (flat addressing).
    fn a20_enabled(&self) -> bool {
        true
    }

    /// Reset the processor without changing memory or monotone accounting.
    ///
    /// Intel 80286 Programmer's Reference Manual, §5.3, Table 5-3 (Processor
    /// State after RESET). Backends without a reset model return an error.
    fn reset(&mut self) -> Result<(), CpuError> {
        Err(CpuError::UnexpectedExit {
            what: "CPU reset is unsupported by this backend".to_string(),
        })
    }

    /// Run guest code until an [`CpuStop`] is hit or `budget` blocks execute.
    ///
    /// `budget` counts `x86jit` blocks; `None` runs until a non-budget stop.
    /// This adapter API remains for standalone CPU callers. The machine uses
    /// [`Cpu::run_steps`] and schedules devices at completed-step boundaries.
    fn run(&mut self, budget: Option<u64>) -> Result<CpuStop, CpuError>;

    /// Run at most `max_steps` scheduling units (§6).
    ///
    /// One unit is a completed instruction or a completed REP iteration. An
    /// empty REP counts once. This is separate from [`Cpu::retired`], which
    /// counts whole instructions. A supported zero-budget call has no guest
    /// effects, including interrupt delivery or device completion consumption.
    /// The default returns [`CpuError::UnexpectedExit`] without execution.
    /// Unsupported modes and an outstanding IN value also return that error.
    /// With `options.stop_when_irq_ready`, return [`CpuStepStop::IrqReady`] at
    /// the first eligible boundary, including between REP iterations. Budget
    /// exhaustion wins over readiness, and a queued eligible IRQ is delivered
    /// before the readiness check, without consuming a step. The readiness
    /// stop never splits an unfinished IN or MMIO unit or an interrupt shadow
    /// (Intel 80286 Programmer's Reference Manual, §3.11.2 and §9.2).
    ///
    /// The stopping instruction's share of [`CpuStepRun::steps`] is:
    ///
    /// - Port IN/OUT, each INS/OUTS element, and HLT: the last counted unit;
    ///   its start is `steps - 1`.
    ///   IN counts at handoff even though its value is still pending; supplying
    ///   that value adds no step. For INS the core retains the answer until its
    ///   faultable ES store completes on the next positive run; that store adds
    ///   no step and blocks IRQ acceptance until complete. Complete the input
    ///   handshake before another positive-budget run.
    /// - MMIO, faults, fetch failures and unsupported instructions: zero; all
    ///   counted units precede the incomplete unit, whose start is `steps`.
    ///   A REP exiting mid-iteration counts only its completed iterations.
    /// - Budget exhaustion or IRQ readiness: no unit is in flight; the next
    ///   unit starts at `steps`. An instruction's own exit wins on the last unit.
    ///
    /// Complete bus effects before acknowledging an interrupt. On error, charge
    /// the delta of [`Cpu::completed_steps`] before propagating the error.
    /// REP restart follows Intel SDM Vol. 3A §7.6 (Program or Task Restart);
    /// the port-handoff count is an engine convention, not hardware timing.
    fn run_steps(
        &mut self,
        _max_steps: u64,
        _options: StepRunOptions,
    ) -> Result<CpuStepRun, CpuError> {
        Err(CpuError::UnexpectedExit {
            what: "step-bounded execution is unsupported by this backend or mode".to_string(),
        })
    }

    /// Declare the ranges whose accesses [`Cpu::run_steps_serving`] offers
    /// to its handler, replacing any earlier set; an empty slice serves none.
    /// See [`device_access`]. A backend may apply the change only at a later
    /// run start where no access is outstanding; until then the earlier set
    /// stays in force. The default ignores the call, for a backend that never
    /// serves accesses in process.
    fn set_served_ranges(&mut self, ranges: &[ServedRange]) -> Result<(), CpuError> {
        let _ = ranges;
        Ok(())
    }

    /// [`Cpu::run_steps`], offering each access inside the served ranges to
    /// `handler` before stopping for it (see [`device_access`]).
    ///
    /// Steps, stops and every guest effect are those of [`Cpu::run_steps`]
    /// with the handled accesses serviced at their stops, except that a
    /// handled access does not end the run. A handler's
    /// [`AccessControl::set_intr`] can end it with
    /// [`CpuStepStop::IrqReady`] and [`AccessControl::stop_at`] with
    /// [`CpuStop::BudgetExhausted`] short of the budget.
    /// [`AccessResult::Fault`] ends it with an error, after charging the
    /// completed steps to [`Cpu::completed_steps`]. The default serves
    /// nothing and runs [`Cpu::run_steps`].
    fn run_steps_serving(
        &mut self,
        max_steps: u64,
        options: StepRunOptions,
        handler: &mut dyn DeviceAccessHandler,
    ) -> Result<CpuStepRun, CpuError> {
        let _ = handler;
        self.run_steps(max_steps, options)
    }

    /// Completed scheduling units across step-bounded runs, including error exits (§6).
    ///
    /// Uses the same units as [`CpuStepRun::steps`], including REP iterations.
    /// Read the delta around [`Cpu::run_steps`] to retain completed work when
    /// the call returns an error. The default is zero for unsupported backends.
    fn completed_steps(&self) -> u64 {
        0
    }

    /// Whether a maskable IRQ can be accepted at this completed boundary (§5.2).
    ///
    /// Requires IF, no STI or SS-load shadow, and no pending port/MMIO completion
    /// (including an answered MMIO operation awaiting replay). This query does
    /// not acknowledge, deliver or require a queued vector. Intel SDM Vol. 3A
    /// §§7.8.1, 7.8.3 describe IF and interrupt inhibition after loading SS.
    /// The default is `false` for backends without this capability.
    fn irq_ready(&self) -> bool {
        false
    }

    /// Read a guest register (full 64-bit width).
    fn reg(&self, reg: CpuReg) -> u64;

    /// Write a guest register (full 64-bit width).
    fn set_reg(&mut self, reg: CpuReg, value: u64);

    /// Supply the value for a pending [`CpuStop::PortIn`] before the next
    /// [`Cpu::run`].
    fn complete_port_in(&mut self, value: u64);

    /// Supply the value for a pending [`CpuStop::MmioRead`] before the next
    /// [`Cpu::run`].
    fn complete_mmio_read(&mut self, value: u64);

    /// Acknowledge a serviced [`CpuStop::MmioWrite`] before the next [`Cpu::run`].
    ///
    /// Unlike a port `out` (which retires immediately), a trapped store leaves the
    /// instruction pointer on the faulting instruction and the block re-executes
    /// from it. This tells the retried store the side effect is already done — so
    /// it does not trap a second time and execution moves on. Without this call a
    /// guest that writes MMIO spins on the same store forever.
    fn complete_mmio_write(&mut self);

    /// Whether guest FLAGS.IF permits maskable interrupts (Intel 80286
    /// Programmer's Reference Manual, HLT and CLI/STI instruction descriptions;
    /// §5.2). The machine uses this to decide whether HLT can wake.
    fn interrupts_enabled(&self) -> bool;

    /// Inject a hardware interrupt request for `vector` (§5.2).
    ///
    /// The backend queues the vector and delivers it at the next `run()`
    /// boundary once the guest's `IF` is set, no memory/port completion is
    /// outstanding, and the one-instruction STI or SS-load shadow has elapsed
    /// (that gating lives in the CPU core). Delivery is through the real-mode IVT: it
    /// pushes FLAGS/CS/IP, clears `IF`, and vectors the handler. A queued vector
    /// is held (never dropped) until it can be delivered, which is what wakes a
    /// `hlt`ed guest. The default is a no-op so a backend without an interrupt
    /// path still runs.
    fn inject_irq(&mut self, _vector: u8) {}

    /// Whether an injected vector is still queued and not yet delivered.
    ///
    /// The machine loop consults this so it only performs one PIC INTA →
    /// [`Cpu::inject_irq`] at a time: while a vector is in flight (masked by
    /// `IF`, held by an STI/SS-load shadow, or waiting on a completion) no new
    /// one is acknowledged. The default is `false` for a backend that takes no
    /// interrupts.
    fn has_pending_irq(&self) -> bool {
        false
    }

    /// Read one byte of guest physical memory for a DMA transfer (§4.3).
    ///
    /// RAM and ROM return their bytes; a trapped window returns
    /// [`DmaAccess::Mmio`] so the machine can call its bus's MMIO read. The
    /// default returns open-bus `0xFF` for a backend with no guest memory.
    fn guest_dma_read(&self, addr: u32) -> DmaAccess<u8> {
        let _ = addr;
        DmaAccess::Complete(0xFF)
    }

    /// Write one DMA byte according to the guest physical memory map (§4.3).
    ///
    /// RAM writes must run the core's `note_write` self-modifying-code hook.
    /// ROM writes are dropped; a trapped window returns [`DmaAccess::Mmio`]
    /// without modifying backing memory, so the machine can dispatch the write.
    /// The default drops the write for a backend with no guest memory.
    fn guest_dma_write(&mut self, addr: u32, value: u8) -> DmaAccess<()> {
        let _ = (addr, value);
        DmaAccess::Complete(())
    }

    /// Whole-instruction retirement count since construction (§6).
    ///
    /// A monotone diagnostic counter. Machine time uses [`Cpu::completed_steps`]
    /// instead so individual REP iterations consume time before the whole
    /// instruction retires. Neither counter depends on wall time.
    /// The default is `0` for a backend that does not count.
    fn retired(&self) -> u64 {
        0
    }

    /// The visible code selector and the instruction pointer: where
    /// execution continues at this completed boundary. A host diagnostic
    /// samples it between runs; reading it changes nothing. The default is
    /// selector 0 with [`CpuReg::Rip`], for a backend without segments.
    fn code_address(&self) -> (u16, u64) {
        (0, self.reg(CpuReg::Rip))
    }

    /// Start a log of the last `capacity` exceptions and interrupts the CPU
    /// delivers in the guest (1 to [`delivery::DELIVERY_LOG_MAX`]); see
    /// [`delivery`]. A backend that can records the registers of the
    /// interrupted code with each delivery ([`Delivery::regs`]). A log
    /// already kept is replaced, unread. The log stays
    /// outside the state the guest executes on: execution, retired counts
    /// and every guest effect are the same with it on or off. The default
    /// refuses, for a backend that delivers nothing in the guest.
    fn enable_delivery_log(&mut self, capacity: usize) -> Result<(), DeliveryLogError> {
        let _ = capacity;
        Err(DeliveryLogError(
            "this CPU backend keeps no delivery log".to_string(),
        ))
    }

    /// Stop the delivery log and discard what it holds.
    fn disable_delivery_log(&mut self) {}

    /// Move the log's records, oldest first, to the end of `records`, and
    /// return its counts, the dropped count then starting again from 0;
    /// `None`, with nothing appended, while no log is kept.
    fn drain_deliveries(&mut self, records: &mut Vec<Delivery>) -> Option<DeliveryCounts> {
        let _ = records;
        None
    }

    /// Append the log's records, oldest first, to `records` without
    /// removing them, and return its counts; `None` while no log is kept.
    fn peek_deliveries(&self, records: &mut Vec<Delivery>) -> Option<DeliveryCounts> {
        let _ = records;
        None
    }
}
