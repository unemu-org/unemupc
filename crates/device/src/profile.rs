// SPDX-License-Identifier: AGPL-3.0-or-later
//! Passive host profiling seam for the PC/AT execution loop.
//!
//! No clock is read here: a host observer owns all timestamps and counters.
//! Callbacks must not affect guest state, access devices, or feed scheduling.
//! This models no additional hardware; see `backlog/docs/status.md` §Speed.

use std::rc::Rc;

/// Exclusive host-time buckets; nested spans charge their innermost bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum ProfilePhase {
    Cpu,
    Scheduler,
    PortIo,
    Mmio,
    Frame,
    Observation,
}

/// Machine-visible CPU return reasons, including failed calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum ProfileExit {
    PortIo,
    Mmio,
    InterruptWindow,
    Halt,
    Budget,
    Other,
    Error,
}

/// Scheduler activity independent of CPU retirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum ProfileEvent {
    DeadlineQuery,
    TickDue,
    DeviceTick,
    HaltedAdvance,
    DeviceSync,
    DmaProbe,
}

/// Host-owned diagnostics, excluded from hardware snapshots and virtual time.
///
/// Implementations must be passive and nonblocking, like `ByteSink` (see
/// `backlog/docs/design/observation.md` §Sinks, bounds and lifecycle). Host
/// timestamps are observations only and must never determine guest execution.
pub trait HostProfiler {
    fn enter(&self, phase: ProfilePhase);
    fn leave(&self, phase: ProfilePhase);
    fn cpu_run(&self, retired: u64, steps: u64, budget: u64, exit: ProfileExit);
    fn event(&self, event: ProfileEvent);
}

/// Optional callback binding. The default performs no host calls or allocation.
#[derive(Clone, Default)]
pub struct ProfileHook(Option<Rc<dyn HostProfiler>>);

impl ProfileHook {
    pub fn new(observer: Rc<dyn HostProfiler>) -> Self {
        Self(Some(observer))
    }

    pub fn enabled(&self) -> bool {
        self.0.is_some()
    }

    pub fn span(&self, phase: ProfilePhase) -> ProfileSpan {
        if let Some(observer) = &self.0 {
            observer.enter(phase);
        }
        ProfileSpan {
            hook: self.clone(),
            phase,
        }
    }

    pub fn cpu_run(&self, retired: u64, steps: u64, budget: u64, exit: ProfileExit) {
        if let Some(observer) = &self.0 {
            observer.cpu_run(retired, steps, budget, exit);
        }
    }

    pub fn event(&self, event: ProfileEvent) {
        if let Some(observer) = &self.0 {
            observer.event(event);
        }
    }
}

/// Ends a diagnostic interval on every return path, including errors.
pub struct ProfileSpan {
    hook: ProfileHook,
    phase: ProfilePhase,
}

impl Drop for ProfileSpan {
    fn drop(&mut self) {
        if let Some(observer) = &self.hook.0 {
            observer.leave(self.phase);
        }
    }
}
