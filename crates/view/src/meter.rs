// SPDX-License-Identifier: AGPL-3.0-or-later
//! How fast the emulation runs, for the window title: the share of the
//! configured clock kept and the guest's picture changes per second, each
//! over a window of about one host second. See backlog/docs/commands.md
//! § Viewing a machine.
//!
//! Host durations come from the presenter and only describe the machine; the
//! meter reads the machine's counters and never drives it.

use machine_view::pacing::virtual_ticks;
use std::{fmt, time::Duration};
use unemu_core_api::Rate;

/// The machine's counters at one presented frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Host time since the session started.
    pub elapsed: Duration,
    /// Virtual time, in ticks of the machine's clock. Time the guest spends
    /// halted waiting for an interrupt still passes on this clock, so it
    /// counts as kept time.
    pub ticks: u64,
    /// The serial of the presented picture, which changes only when its
    /// pixels do; `None` before the first picture.
    pub picture: Option<u64>,
}

/// The rates of the latest completed window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// Virtual time advanced over host time elapsed, in whole percent of the
    /// configured clock.
    pub speed_percent: u64,
    /// Presented frames whose picture differed from the previous presented
    /// frame, per host second.
    pub fps: u64,
}

impl fmt::Display for Reading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}% · {} fps", self.speed_percent, self.fps)
    }
}

pub struct RateMeter {
    rate: Rate,
    start: Option<Sample>,
    last_picture: Option<u64>,
    changed_frames: u64,
    reading: Option<Reading>,
}

impl RateMeter {
    /// Long enough to smooth a guest that redraws in bursts, short enough
    /// that the title follows a scene change within a second or two.
    pub const WINDOW: Duration = Duration::from_secs(1);

    /// A meter for a machine whose ticks run at `rate`.
    pub fn new(rate: Rate) -> Self {
        Self {
            rate,
            start: None,
            last_picture: None,
            changed_frames: 0,
            reading: None,
        }
    }

    /// Record one presented frame; closes the window once it spans
    /// [`Self::WINDOW`], and the next window starts at this sample.
    pub fn observe(&mut self, sample: Sample) {
        let Some(start) = self.start else {
            self.start = Some(sample);
            self.last_picture = sample.picture;
            self.changed_frames = 0;
            return;
        };
        if sample.picture != self.last_picture {
            self.last_picture = sample.picture;
            self.changed_frames += 1;
        }
        let window = sample.elapsed.saturating_sub(start.elapsed);
        if window < Self::WINDOW {
            return;
        }
        let nanos = window.as_nanos();
        let kept = virtual_ticks(window, self.rate);
        let ticks = u128::from(sample.ticks.saturating_sub(start.ticks));
        self.reading = (kept > 0).then(|| Reading {
            speed_percent: rounded_ratio(ticks.saturating_mul(100), kept),
            fps: rounded_ratio(
                u128::from(self.changed_frames).saturating_mul(1_000_000_000),
                nanos,
            ),
        });
        self.start = Some(sample);
        self.changed_frames = 0;
    }

    /// Forget the current window and the shown rates, when the machine stops
    /// advancing (paused, hidden, stopped) or its pace changes, so the next
    /// reading covers only time it ran at one pace.
    pub fn reset(&mut self) {
        self.start = None;
        self.last_picture = None;
        self.changed_frames = 0;
        self.reading = None;
    }

    /// The latest completed window's rates; `None` before the first one.
    pub fn reading(&self) -> Option<Reading> {
        self.reading
    }
}

/// `numerator / denominator` rounded half up; `denominator` is nonzero.
fn rounded_ratio(numerator: u128, denominator: u128) -> u64 {
    let quotient = numerator / denominator;
    let rounded = if (numerator % denominator).saturating_mul(2) >= denominator {
        quotient + 1
    } else {
        quotient
    };
    u64::try_from(rounded).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
