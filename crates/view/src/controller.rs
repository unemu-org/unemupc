// SPDX-License-Identifier: AGPL-3.0-or-later
//! Window-independent host controls and session scheduling.
//! Host durations pace presentation only; see backlog/docs/architecture.md § Contracts and ownership.

use machine_view::{
    pacing::{self, Speed},
    Session,
};
use std::{collections::BTreeSet, error::Error, io::Write, time::Duration};
use unemu_core_api::{Machine, ResetKind};
use winit::{
    event::{ElementState, MouseButton},
    keyboard::KeyCode,
};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct Controller {
    pub session: Session,
    pub paused: bool,
    focused: bool,
    occluded: bool,
    keys: BTreeSet<KeyCode>,
    swallowed: BTreeSet<KeyCode>,
    closed: bool,
    error_reported: bool,
}

impl Controller {
    pub fn new(machine: Box<dyn Machine>, speed: Speed, warnings: &mut impl Write) -> Result<Self> {
        let display = machine
            .description()
            .displays
            .first()
            .ok_or("machine has no display")?
            .id;
        let mut session = Session::new(machine, display, warnings)?;
        session.select_speed(speed, Duration::ZERO);
        session.next_paint = Some(Duration::ZERO);
        Ok(Self {
            session,
            paused: false,
            focused: true,
            occluded: false,
            keys: BTreeSet::new(),
            swallowed: BTreeSet::new(),
            closed: false,
            error_reported: false,
        })
    }

    /// True requests a redraw, even if the machine stopped at this boundary.
    pub fn tick(&mut self, elapsed: Duration, warnings: &mut impl Write) -> Result<bool> {
        let (advance, deadline) = pacing::wake(
            elapsed,
            self.session.next_paint,
            self.session.running && !self.paused && !self.occluded,
        );
        self.session.next_paint = deadline;
        if advance {
            self.session.advance_with_warnings(elapsed, warnings)?;
        }
        if !self.error_reported {
            if let Some(error) = &self.session.error {
                writeln!(warnings, "machine stopped: {error}")?;
                self.error_reported = true;
            }
        }
        self.session.audio.check_failure(warnings);
        Ok(advance)
    }

    /// True means quit, after the machine has finished successfully.
    pub fn key(
        &mut self,
        key: KeyCode,
        state: ElementState,
        elapsed: Duration,
        warnings: &mut impl Write,
    ) -> Result<bool> {
        use KeyCode::*;
        let fresh = if state == ElementState::Pressed {
            self.keys.insert(key)
        } else {
            self.keys.remove(&key);
            false
        };
        if self.swallowed.contains(&key) {
            if state == ElementState::Released {
                self.swallowed.remove(&key);
            }
            return Ok(false);
        }
        if state == ElementState::Pressed && !fresh {
            return Ok(false);
        }
        let control = [ControlLeft, ControlRight]
            .iter()
            .any(|k| self.keys.contains(k));
        let alt = [AltLeft, AltRight].iter().any(|k| self.keys.contains(k));
        let command_release = cfg!(target_os = "macos")
            && key == KeyG
            && [SuperLeft, SuperRight]
                .iter()
                .any(|k| self.keys.contains(k))
            && self.session.mouse.captured();
        if fresh && (command_release || (control && alt && matches!(key, KeyR | KeyP | KeyQ))) {
            self.swallowed.insert(key);
            match key {
                KeyG => self.session.release_pointer(),
                KeyR => {
                    self.session.release_pointer();
                    self.session.reset(ResetKind::Soft, elapsed, warnings)?;
                    self.session.next_paint = Some(elapsed);
                    writeln!(warnings, "machine reset")?;
                }
                KeyP => {
                    self.paused = !self.paused;
                    if self.paused {
                        self.session.focus_changed(false);
                        self.session.audio.discard_pending();
                    } else {
                        self.session.focus_changed(self.focused);
                    }
                    self.session
                        .occlusion_changed(self.paused || self.occluded, elapsed);
                    writeln!(
                        warnings,
                        "{}",
                        if self.paused { "paused" } else { "resumed" }
                    )?;
                }
                KeyQ => {
                    self.close(warnings)?;
                    return Ok(true);
                }
                _ => {}
            }
        } else if self.focused && (!self.paused || state == ElementState::Released) {
            self.session.key_event(key, state);
        }
        Ok(false)
    }

    pub fn focus(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.keys.clear();
            self.swallowed.clear();
        }
        self.session.focus_changed(focused && !self.paused);
    }

    pub fn occlude(&mut self, occluded: bool, elapsed: Duration) {
        self.occluded = occluded;
        self.session
            .occlusion_changed(self.paused || occluded, elapsed);
        if occluded {
            self.session.release_pointer();
            self.session.audio.discard_pending();
        }
    }

    pub fn pointer(&mut self, position: Option<(u16, u16)>) {
        self.session.mouse.pointer_inside(position.is_some());
        if !self.paused {
            self.session.pointer_at(position);
        }
    }

    pub fn button(&mut self, button: MouseButton, state: ElementState) {
        if !self.paused && self.focused {
            self.session.mouse_button(button, state);
        }
    }

    pub fn motion(&mut self, dx: f64, dy: f64) {
        if !self.paused && self.focused {
            self.session.mouse_motion(dx, dy);
        }
    }

    pub fn close(&mut self, warnings: &mut impl Write) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        // A failed finish may leave storage uncertain; exit paths must not retry it.
        self.closed = true;
        self.session.release_pointer();
        self.session.audio.discard_pending();
        self.session.finish(warnings)
    }
}

#[cfg(test)]
mod tests;
