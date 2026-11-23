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
    startup_speed: Speed,
    unlimited: bool,
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
            startup_speed: speed,
            unlimited: false,
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
        } else if self.keys.remove(&key) {
            false
        } else {
            return Ok(false);
        };
        if self.swallowed.contains(&key) {
            if state == ElementState::Released {
                self.swallowed.remove(&key);
                if key == KeyT {
                    self.session.hold_fast_forward(false, elapsed);
                }
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
        let command = cfg!(target_os = "macos")
            && [SuperLeft, SuperRight]
                .iter()
                .any(|k| self.keys.contains(k));
        if state == ElementState::Released
            && matches!(
                key,
                ControlLeft | ControlRight | AltLeft | AltRight | SuperLeft | SuperRight
            )
            && !(control && alt || command)
        {
            self.session.hold_fast_forward(false, elapsed);
        }
        let command_chord = command
            && (matches!(key, KeyT | KeyU) || (key == KeyG && self.session.mouse.captured()));
        if fresh
            && (command_chord
                || (control && alt && matches!(key, KeyR | KeyP | KeyQ | KeyT | KeyU)))
        {
            self.swallowed.insert(key);
            match key {
                KeyG => self.session.release_pointer(),
                KeyT => self.session.hold_fast_forward(true, elapsed),
                KeyU => {
                    self.unlimited = !self.unlimited;
                    self.session.select_speed(
                        if self.unlimited {
                            Speed::Unlimited
                        } else {
                            self.startup_speed
                        },
                        elapsed,
                    );
                }
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

    pub fn window_title(&self, name: &str) -> String {
        let speed = self.session.pacing.effective();
        if speed == Speed::Normal {
            name.to_owned()
        } else {
            format!("{name} — {}", speed.label().to_lowercase())
        }
    }

    #[cfg(target_os = "macos")]
    pub fn modifiers_changed(
        &mut self,
        modifiers: winit::keyboard::ModifiersState,
        elapsed: Duration,
    ) {
        // macOS can report Command release without a keyboard event.
        if !modifiers.super_key() {
            for key in [KeyCode::SuperLeft, KeyCode::SuperRight] {
                if self.keys.remove(&key) {
                    self.session.key_event(key, ElementState::Released);
                }
            }
            if !(modifiers.control_key() && modifiers.alt_key()) {
                self.session.hold_fast_forward(false, elapsed);
            }
        }
    }

    pub fn focus(&mut self, focused: bool, elapsed: Duration) {
        self.focused = focused;
        if !focused {
            self.session.hold_fast_forward(false, elapsed);
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
