// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serving accesses while the CPU runs (§3.1; `backlog/docs/design/scheduler.md`
//! §In-process device access).
//!
//! The ordinary loop returns from the CPU at every port access and every
//! access to a trapped window, then charges the steps before it, services
//! the access on the bus, charges the access's own step and settles. The CPU
//! seam can instead call [`InProcess`] synchronously for accesses inside
//! [`SERVED_RANGES`] ([`cpu::Cpu::run_steps_serving`]). The handler does
//! exactly what the loop would do for that stop, in the same order and at the
//! same virtual time, with the same helpers, and lets the run continue. It
//! declines ([`AccessResult::Fallback`]) before changing anything whenever it
//! cannot prove the result equal: an access whose own step would reach the
//! next device deadline (whose ticks may need DMA, which needs the CPU), one
//! that reaches a DMA requester or controller, one the bus would fault, a
//! port write on a machine whose memory-map hook must see every port write
//! with the CPU in hand, and a transfer owned by an exception delivery.
//!
//! What only the loop can do with the CPU after an access ends the run at
//! the next boundary instead (after an MMIO access's instruction completes),
//! where the loop does it before the next instruction, as after the ordinary
//! stop: sync the A20 gate, notice a reset request, and acknowledge a request
//! pending at the interrupt controller, with the loop's own readiness check
//! (the CPU core's interrupt-ready stop is stricter about held single-step
//! traps and waiting events). A deadline the access moves earlier than the
//! run's budget ends the run there. The next deadline's step is kept between
//! accesses, so an access costs one deadline query, made after it.

use std::cell::Cell;
use std::num::NonZeroU64;

use bus::Bus;
use cpu::{AccessControl, AccessResult, AccessSpace, DeviceAccess, ServedRange};
use device::{InterruptController, VClock};

use crate::{
    forward_irq_levels, next_device_deadline, scheduler_error, step_target, steps_until,
    DmaChannelWiring, DmaWiring, IrqLine, MachineError, SchedulerEngine,
};

/// The accesses served while the CPU runs: the diagnostic checkpoint port
/// 80h, which firmware writes at every POST step, and the display memory
/// window A0000h–BFFFFh (IBM Personal Computer AT Technical Reference,
/// Section 1, System Board, I/O Address Map and System Memory Map). Every
/// other port and trapped window still returns from the CPU.
pub const SERVED_RANGES: [ServedRange; 2] = [
    ServedRange::Ports(0x80..=0x80),
    ServedRange::Mmio(0xA0000..=0xBFFFF),
];

/// Accesses served while the CPU ran ([`crate::Machine::served_accesses`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServedAccesses {
    /// Port accesses.
    pub ports: u64,
    /// MMIO accesses.
    pub mmio: u64,
}

impl ServedAccesses {
    pub(crate) fn add(&mut self, other: &Self) {
        self.ports += other.ports;
        self.mmio += other.mmio;
    }
}

/// What the loop finishes after a run served by [`InProcess`].
#[derive(Debug, Default)]
pub(crate) struct ServedRun {
    /// Steps of this run already charged to the clock.
    pub charged: u64,
    /// The board A20 line changed: the loop syncs it before the next
    /// instruction.
    pub sync_a20: bool,
    /// DMA became due, which needs the CPU's memory: the loop settles
    /// before it services the run's stop.
    pub settle: bool,
    /// The error that ended the run, which the loop returns.
    pub error: Option<MachineError>,
    /// Accesses served in this run.
    pub served: ServedAccesses,
}

/// Reads the board A20 line: (transition count, enabled).
type A20Line<'a> = &'a dyn Fn() -> (u64, bool);

/// The loop's state, borrowed apart from the CPU for one run.
pub(crate) struct InProcess<'a> {
    pub bus: &'a mut Bus,
    pub pic: &'a mut dyn InterruptController,
    pub clock: &'a mut VClock,
    pub completed_steps: &'a mut u64,
    pub clock_remainder: &'a mut u64,
    pub engine: SchedulerEngine,
    pub cpu_rate_numerator: NonZeroU64,
    pub cpu_tick_denominator: u128,
    pub irq_lines: &'a [IrqLine],
    pub last_levels: &'a mut [Option<bool>],
    pub dma_wirings: &'a [DmaWiring],
    pub dma_channel_wirings: &'a [DmaChannelWiring],
    /// The board A20 line and the change count the CPU last saw.
    pub a20_line: Option<(A20Line<'a>, u64)>,
    pub reset_signal: Option<&'a Cell<bool>>,
    /// A memory-map hook is attached; it must run after every port write.
    pub memory_map: bool,
    /// The run's step budget.
    pub budget: u64,
    /// The step of this run at which the clock reaches the next device
    /// deadline (`u64::MAX` for none), as last computed with every device's
    /// answer cached and no device touched since; `None` when unknown.
    pub deadline_at: Option<u64>,
    pub outcome: ServedRun,
}

impl InProcess<'_> {
    /// Whether a wired requester has DMA pending, which the loop's settle
    /// would service.
    fn dma_pending(&self) -> bool {
        self.dma_wirings.iter().any(|wiring| {
            self.bus
                .device(wiring.requester)
                .is_some_and(|device| device.dma_request_pending())
        })
    }

    fn sample_irq_lines(&mut self) {
        forward_irq_levels(self.bus, self.pic, self.irq_lines, self.last_levels);
    }

    /// `Machine::charge_steps`, with the loop's DMA settle left to the loop:
    /// [`Self::access`] charges only inside a deadline-free interval, where
    /// no device ticks and no DMA becomes due; should DMA become due anyway,
    /// the loop settles right after the run.
    fn charge(&mut self, steps: u64) -> Result<(), MachineError> {
        self.charge_with(steps, false)
    }

    /// [`Self::charge`]; `quiet` when the caller knows from
    /// [`Self::deadline_at`] that no device is due before the target and
    /// every device's next event is cached: `Machine::advance_to` would then
    /// only move the clocks (`Bus::tick_due` visits nothing due, refills no
    /// cache, and IRQ sampling finds no change), so only the clocks move.
    fn charge_with(&mut self, steps: u64, quiet: bool) -> Result<(), MachineError> {
        *self.completed_steps = self
            .completed_steps
            .checked_add(steps)
            .ok_or_else(|| scheduler_error("step counter exhausted"))?;
        self.outcome.charged += steps;
        let (target, remainder) = step_target(
            *self.clock,
            steps,
            *self.clock_remainder,
            self.cpu_rate_numerator,
            self.cpu_tick_denominator,
        )?;
        *self.clock_remainder = remainder;
        if quiet && self.engine == SchedulerEngine::Deadline {
            if *self.clock < target {
                *self.clock = target;
                self.bus.advance_quiet(target);
            }
            return Ok(());
        }
        // `Machine::advance_to`.
        while *self.clock < target {
            let at = next_device_deadline(self.engine, self.bus, *self.clock)?
                .map_or(target, |at| at.min(target));
            *self.clock = at;
            match self.engine {
                #[cfg(any(test, feature = "reference-scheduler"))]
                SchedulerEngine::PerClock => self.bus.tick_all(at),
                SchedulerEngine::Deadline => {
                    self.bus.tick_due(at);
                }
            }
            self.sample_irq_lines();
            self.outcome.settle |= self.dma_pending();
        }
        Ok(())
    }

    /// Whether the access may be served here at all; see the module docs.
    fn eligible(&mut self, access: &DeviceAccess) -> bool {
        if access.delivery {
            return false;
        }
        let port = access.space == AccessSpace::Port;
        if port && access.write && self.memory_map {
            return false;
        }
        // A DMA requester or controller could start a transfer that only
        // the CPU's memory can carry.
        let dma_wirings = self.dma_wirings;
        let channel_wirings = self.dma_channel_wirings;
        let free = |index: usize| {
            !dma_wirings
                .iter()
                .any(|wiring| wiring.requester == index || wiring.controller == index)
                && !channel_wirings
                    .iter()
                    .any(|route| route.requester == index || route.controller == index)
        };
        let reaches = if port {
            self.bus
                .port_access_reaches_only(access.address as u16, access.size, free)
        } else {
            self.bus
                .mmio_access_reaches_only(access.address, access.size, free)
        };
        if !reaches || self.dma_pending() {
            return false;
        }

        if self.deadline_at.is_none() {
            self.deadline_at = self.deadline_from_now();
        }
        // `access.steps` is the run's position once this access is charged.
        self.deadline_at.is_some_and(|at| access.steps < at)
    }

    /// The step of this run at which the clock reaches the next device
    /// deadline, `u64::MAX` for none; `None` when the query fails, which the
    /// loop then reports at its own point.
    fn deadline_from_now(&mut self) -> Option<u64> {
        match next_device_deadline(self.engine, self.bus, *self.clock) {
            Ok(None) => Some(u64::MAX),
            Ok(Some(deadline)) => Some(self.outcome.charged.saturating_add(steps_until(
                deadline,
                *self.clock,
                *self.clock_remainder,
                self.cpu_rate_numerator,
                self.cpu_tick_denominator,
            ))),
            Err(_) => None,
        }
    }

    /// Record an error that ends the run.
    fn fail(&mut self, error: MachineError) -> AccessResult {
        self.outcome.error.get_or_insert(error);
        AccessResult::Fault
    }
}

impl cpu::DeviceAccessHandler for InProcess<'_> {
    fn access(&mut self, access: &DeviceAccess, control: &mut AccessControl) -> AccessResult {
        let port = access.space == AccessSpace::Port;
        // The loop charges a port access's own step after the access, and an
        // MMIO access's when its instruction completes.
        let Some(charge) = access.steps.checked_sub(self.outcome.charged) else {
            return AccessResult::Fallback;
        };
        if (port && charge == 0) || !self.eligible(access) {
            return AccessResult::Fallback;
        }
        // `eligible` proved the clock stays short of the next deadline.
        let quiet = self.engine == SchedulerEngine::Deadline;
        if let Err(error) = self.charge_with(charge - u64::from(port), quiet) {
            return self.fail(error);
        }
        if self.outcome.settle {
            // Charged as the loop would; it settles, then services the stop.
            return AccessResult::Fallback;
        }
        let result = match (access.space, access.write) {
            (AccessSpace::Port, true) => self
                .bus
                .port_write(access.address as u16, access.size, access.value as u32)
                .map(|()| 0),
            (AccessSpace::Port, false) => self
                .bus
                .port_read(access.address as u16, access.size)
                .map(u64::from),
            (AccessSpace::Mmio, true) => self
                .bus
                .mmio_write(access.address, access.size, access.value)
                .map(|()| 0),
            (AccessSpace::Mmio, false) => self.bus.mmio_read(access.address, access.size),
        };
        if port {
            self.outcome.served.ports += 1;
        } else {
            self.outcome.served.mmio += 1;
        }
        #[cfg(feature = "host-profile")]
        self.bus.profile().event(if port {
            device::profile::ProfileEvent::ServedPortIo
        } else {
            device::profile::ProfileEvent::ServedMmio
        });
        // What the loop does after the access, in its order.
        let mut stop = false;
        if port && access.write {
            if let Some((line, seen)) = self.a20_line {
                stop |= line().0 != seen;
                self.outcome.sync_a20 |= stop;
            }
        }
        self.bus.invalidate_deadlines();
        self.deadline_at = None;
        let charged = if port { self.charge(1) } else { Ok(()) };
        if result.is_err() {
            self.bus.settle();
        }
        self.bus.settle_accessed();
        self.sample_irq_lines();
        self.outcome.settle |= self.dma_pending();
        stop |= self.outcome.settle;
        let value = match result {
            Ok(value) => value,
            Err(error) => return self.fail(error.into()),
        };
        if let Err(error) = charged {
            return self.fail(error);
        }
        stop |= self.reset_signal.is_some_and(Cell::get);
        // A request at the controller: the loop's own acceptance check runs
        // at the boundary after this access, as after the ordinary stop.
        stop |= self.pic.poll().is_some();
        // A failed query also stops here: the loop repeats it after its
        // reset and cap checks, where the ordinary stop's loop would fail.
        self.deadline_at = self.deadline_from_now();
        match self.deadline_at {
            Some(at) if !stop => {
                if at < self.budget {
                    control.stop_at(at);
                }
            }
            _ => control.stop_at(self.outcome.charged),
        }
        AccessResult::Handled(value)
    }
}
