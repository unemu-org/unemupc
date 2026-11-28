// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-process device access: the machine serves chosen port and MMIO
//! accesses while the CPU runs, instead of the CPU returning on each one.
//!
//! The ordinary path returns from [`crate::Cpu::run_steps`] at every port
//! access and every access to a trapped window, and the machine services the
//! stop. A backend that supports it can instead call a
//! [`DeviceAccessHandler`] synchronously for accesses inside the ranges the
//! machine declared with [`crate::Cpu::set_served_ranges`], then continue the
//! same run ([`crate::Cpu::run_steps_serving`]). The handler sees the access
//! exactly as the stop would have carried it, at the same position, and either
//! serves it ([`AccessResult::Handled`]) or declines
//! ([`AccessResult::Fallback`]), in which case the run returns the ordinary
//! stop. Nothing here names a backend; the types are this seam's own. See
//! `backlog/docs/design/scheduler.md` §In-process device access.

use std::ops::RangeInclusive;

/// A range of guest addresses whose accesses the machine offers to serve
/// while the CPU runs. Accesses outside every range always stop the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServedRange {
    /// I/O ports, matched by the access's starting port.
    Ports(RangeInclusive<u16>),
    /// Guest-physical addresses of trapped (MMIO) memory. The backend serves
    /// only the trapped parts of the range, and only accesses that fit wholly
    /// inside one of them; any other access stops the run as before.
    Mmio(RangeInclusive<u64>),
}

/// Which address space an access is in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AccessSpace {
    /// An `in`/`out` port access, or one `ins`/`outs` element.
    Port,
    /// A load or store in a trapped (MMIO) window.
    Mmio,
}

/// One access offered to a [`DeviceAccessHandler`] during a run.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DeviceAccess {
    /// Port or MMIO.
    pub space: AccessSpace,
    /// Whether the guest writes (`out`, store) rather than reads.
    pub write: bool,
    /// Port number or guest-physical address.
    pub address: u64,
    /// Access width in bytes.
    pub size: u8,
    /// The value written (low `size` bytes); zero for a read.
    pub value: u64,
    /// The run's completed steps had it stopped at this access, counted as
    /// [`crate::CpuStepRun::steps`] counts them for the matching stop: a port
    /// access includes its own unit, an MMIO access only the units before
    /// it. Counted from the start of the current
    /// [`crate::Cpu::run_steps_serving`] call.
    pub steps: u64,
    /// An exception or interrupt delivery owns the transfer (a frame pushed
    /// into trapped memory), not an instruction.
    pub delivery: bool,
}

/// What a [`DeviceAccessHandler`] did with an access.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AccessResult {
    /// Served: the read value in its low `size` bytes, ignored for a write.
    /// The run continues.
    Handled(u64),
    /// Not served, and nothing changed: the run returns the ordinary stop for
    /// this access, which the caller services as usual.
    Fallback,
    /// Serving failed after it may have changed device state. The run ends
    /// with an error; the handler keeps whatever describes the failure.
    /// Unlike an ordinary stop whose service failed (after a failed OUT the
    /// CPU can run on), the access stays latched as a fault: every later run
    /// returns the same error with no step until the CPU is reset.
    Fault,
}

/// Effects a handler asks of the running CPU, applied only when it returns
/// [`AccessResult::Handled`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct AccessControl {
    intr: Option<bool>,
    stop_at: Option<u64>,
}

impl AccessControl {
    /// Set the interrupt request level the CPU watches for the rest of the
    /// run: while it is high, the run stops with
    /// [`crate::CpuStepStop::IrqReady`] at the first boundary where a
    /// maskable interrupt can be accepted (IF set, no STI or SS-load shadow,
    /// no pending completion). It affects only the current
    /// [`crate::Cpu::run_steps_serving`] call.
    pub fn set_intr(&mut self, asserted: bool) {
        self.intr = Some(asserted);
    }

    /// End the run at the first completed boundary at or after `steps`
    /// completed steps of the current call, with
    /// [`crate::CpuStop::BudgetExhausted`]. A position already passed ends it
    /// at the next boundary; an unfinished instruction (an MMIO access's own
    /// instruction) completes first. The earliest request wins.
    pub fn stop_at(&mut self, steps: u64) {
        self.stop_at = Some(self.stop_at.map_or(steps, |at| at.min(steps)));
    }

    /// The requested interrupt level, if the handler set one.
    pub fn intr(&self) -> Option<bool> {
        self.intr
    }

    /// The requested stop position, if the handler set one.
    pub fn stop(&self) -> Option<u64> {
        self.stop_at
    }
}

/// Guest RAM as a handler may reach it while serving one access, for a
/// device window whose bytes live in guest RAM (a banked framebuffer
/// window). Reads see the access's own earlier writes; writes take effect,
/// with self-modifying-code tracking, only when the handler returns
/// [`AccessResult::Handled`]. Paging does not apply: addresses are physical.
pub trait GuestRam {
    /// Read `bytes.len()` bytes at `address`; false, with `bytes`
    /// unspecified, when any of them is not RAM.
    fn read(&self, address: u64, bytes: &mut [u8]) -> bool;
    /// Write `bytes` at `address`; false, writing nothing, when any of them
    /// is not RAM.
    fn write(&mut self, address: u64, bytes: &[u8]) -> bool;
}

/// Serves accesses synchronously while the CPU runs (see the module docs).
///
/// Called at most once per access, in guest order, never reentrantly. It must
/// leave the guest-visible world exactly as the ordinary stop's service
/// would, at the same virtual time, or return [`AccessResult::Fallback`]
/// before changing anything. A handler that panics ends the run with an
/// error carrying the panic message, and the backend may refuse to run its
/// CPU again (the x86jit core poisons the whole VM).
pub trait DeviceAccessHandler {
    /// Serve or decline one access.
    fn access(&mut self, access: &DeviceAccess, control: &mut AccessControl) -> AccessResult;

    /// [`Self::access`] with guest RAM at hand, which a backend that can
    /// offer it calls instead. The default ignores the RAM.
    fn access_with_ram(
        &mut self,
        access: &DeviceAccess,
        control: &mut AccessControl,
        ram: &mut dyn GuestRam,
    ) -> AccessResult {
        let _ = ram;
        self.access(access, control)
    }
}
