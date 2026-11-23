// SPDX-License-Identifier: AGPL-3.0-or-later
//! Host floppy input at deterministic run boundaries. IBM PC AT Technical
//! Reference, Fixed Disk and Diskette Drive Adapter, Digital Input Register
//! (3F7h) and Interface Lines; `backlog/docs/design/spec.md` §7.8 and
//! `backlog/docs/design/persistent-media.md` § Media changes.

use crate::{AssembledMachine, DeviceId, RunPolicy};
use devices::Upd765;
use machine::RunOutcome;

/// One host input, never delivered through the guest keyboard controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloppyAction {
    /// Open the drive door.
    Eject,
    /// Insert a zero-based catalogue entry.
    Insert(usize),
    /// Select the next entry, wrapping to the first; an empty drive selects zero.
    Next,
}

/// A floppy event scheduled in instruction-equivalent virtual time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloppyEvent {
    /// Earliest completed virtual-time boundary; halted PIT clocks may round up.
    pub at: u64,
    /// Stable controller role.
    pub controller: DeviceId,
    /// Controller-local drive-select unit.
    pub unit: u8,
    /// Door operation.
    pub action: FloppyAction,
}

/// Host-side failure applying a media input; never becomes a guest disk error.
#[derive(Debug, thiserror::Error)]
pub enum FloppyError {
    #[error(transparent)]
    Config(#[from] crate::ConfigError),
    #[error("floppy controller or drive is unavailable")]
    Unavailable,
    #[error("floppy event times must be ordered and within the run interval")]
    Schedule,
    #[error(transparent)]
    Device(#[from] devices::upd765::MediaChangeError),
    #[error(transparent)]
    Persistence(#[from] crate::PersistenceError),
    #[error(transparent)]
    Assembly(#[from] crate::AssemblyError),
    #[error(transparent)]
    Run(#[from] crate::RunError),
}

impl AssembledMachine {
    pub fn floppy(&mut self, controller: DeviceId) -> Result<&mut Upd765, FloppyError> {
        let index = *self
            .device_index
            .get(&controller)
            .ok_or(FloppyError::Unavailable)?;
        self.machine
            .bus_mut()
            .device_mut(index)
            .and_then(|d| d.as_any_mut())
            .and_then(|d| d.downcast_mut::<Upd765>())
            .ok_or(FloppyError::Unavailable)
    }

    /// Apply an input before advancing the CPU again. Eject seals the current
    /// journal using the session finish API; insertion reopens its immutable base
    /// and overlay. Opening the next journal precedes ejection so an opening
    /// failure leaves the current medium inserted.
    pub fn change_floppy(
        &mut self,
        controller: DeviceId,
        unit: u8,
        action: FloppyAction,
    ) -> Result<(), FloppyError> {
        self.check_persistence()?;
        let count = self
            .floppy_names
            .get(&(controller, unit))
            .ok_or(FloppyError::Unavailable)?
            .len();
        let fdc = self.floppy(controller)?;
        fdc.check_media_boundary()?;
        let old = fdc.inserted_medium(usize::from(unit));
        let next = match action {
            FloppyAction::Eject => None,
            FloppyAction::Insert(index) => Some(index),
            FloppyAction::Next if count == 0 => return Ok(()),
            FloppyAction::Next => Some(old.map_or(0, |i| (i + 1) % count)),
        };
        if next.is_some_and(|i| i >= count) {
            return Err(devices::upd765::MediaChangeError::UnknownMedium.into());
        }
        if let Some(index) = next.filter(|_| next != old) {
            for handle in &mut self.persistence {
                if handle.floppy == Some((controller, unit, index)) {
                    handle.insert()?;
                }
            }
        }
        if let Some(index) = old {
            let result = self
                .persistence
                .iter_mut()
                .filter(|handle| handle.floppy == Some((controller, unit, index)))
                .try_for_each(|handle| handle.eject(&mut self.machine));
            if let Err(error) = result {
                if let Some(index) = next.filter(|_| next != old) {
                    for handle in &mut self.persistence {
                        if handle.floppy == Some((controller, unit, index)) {
                            handle.cancel_insert();
                        }
                    }
                }
                return Err(error.into());
            }
        }
        self.floppy(controller)?.eject(usize::from(unit))?;
        if let Some(index) = next {
            if next == old {
                for handle in &mut self.persistence {
                    if handle.floppy == Some((controller, unit, index)) {
                        handle.insert()?;
                    }
                }
            }
            self.floppy(controller)?.insert(usize::from(unit), index)?;
        }
        Ok(())
    }

    /// Supply a host label when lowering retained inline bytes without a path.
    /// Labels never enter the guest or affect the event schedule.
    pub fn set_floppy_name(&mut self, controller: DeviceId, unit: u8, medium: usize, name: String) {
        if let Some(label) = self
            .floppy_names
            .get_mut(&(controller, unit))
            .and_then(|names| names.get_mut(medium))
        {
            *label = name;
        }
    }

    /// Name of the inserted medium for host presentation, or `empty`.
    pub fn floppy_name(&mut self, controller: DeviceId, unit: u8) -> String {
        let index = self
            .floppy(controller)
            .ok()
            .and_then(|fdc| fdc.inserted_medium(usize::from(unit)));
        index
            .and_then(|i| {
                self.floppy_names
                    .get(&(controller, unit))
                    .and_then(|names| names.get(i))
            })
            .cloned()
            .unwrap_or_else(|| "empty".into())
    }

    /// Headless input schedule. Events at the same time retain caller order;
    /// completed caps are checked through the usual session run path. The
    /// returned outcome identifies an early halt, so later events are not applied.
    /// Active caps are exact; halted caps use the machine's whole-PIT-clock rounding.
    pub fn run_floppy_schedule(
        &mut self,
        cap: u64,
        idle_limit: Option<u64>,
        events: &[FloppyEvent],
    ) -> Result<RunOutcome, FloppyError> {
        let mut previous = self.machine.virtual_time();
        for event in events {
            if event.at < previous || event.at > cap {
                return Err(FloppyError::Schedule);
            }
            previous = event.at;
        }
        if cap < previous {
            return Err(FloppyError::Schedule);
        }
        for event in events {
            if event.at > self.machine.virtual_time() {
                let outcome = self.run_session(&RunPolicy::natural(Some(event.at), idle_limit))?;
                if outcome != RunOutcome::RetiredCapReached {
                    return Ok(outcome);
                }
            }
            self.change_floppy(event.controller, event.unit, event.action)?;
        }
        Ok(self.run_session(&RunPolicy::natural(Some(cap), idle_limit))?)
    }
}
