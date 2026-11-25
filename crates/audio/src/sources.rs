// SPDX-License-Identifier: AGPL-3.0-or-later
//! Interval helpers for timestamped edges and native-rate PCM history.
//!
//! Intel 8254 datasheet, Mode 3, describes the edge signal a future producer
//! records; no timer hardware is implemented here. See
//! `backlog/docs/design/audio.md` §Mixing and §Source lifecycle.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use super::{AudioError, AudioSource, SampleArea, SamplePeriod, StereoFrame};
use device::VClock;

struct Edges {
    level: StereoFrame,
    retained_from: VClock,
    changes: VecDeque<(VClock, StereoFrame)>,
    end: Option<VClock>,
    covered_until: VClock,
    error: Option<AudioError>,
    needs_segment: bool,
    device_level: StereoFrame,
    output_enabled: bool,
}

impl Edges {
    fn change(&mut self, at: VClock, level: StereoFrame) {
        if let Some((_, last)) = self.changes.back_mut().filter(|(clock, _)| *clock == at) {
            *last = level;
        } else {
            self.changes.push_back((at, level));
        }
    }

    fn release(&mut self, retain_from: VClock) {
        while self
            .changes
            .front()
            .is_some_and(|(at, _)| *at <= retain_from)
        {
            if let Some((_, level)) = self.changes.pop_front() {
                self.level = level;
            }
        }
        self.retained_from = self.retained_from.max(retain_from);
    }

    fn bound_unobserved(&mut self, at: VClock) {
        if !self.output_enabled {
            let retain = super::frame_boundary(super::completed_frames(at)) as u64;
            self.release(VClock(retain));
        }
    }
}

/// Shared producer history for one active mixer consumer. Clones are writer
/// handles, not independent readers: release advances their shared history.
#[derive(Clone)]
pub struct EdgeSource(Rc<RefCell<Edges>>);

impl EdgeSource {
    /// The initial level is active from PIT zero until the first edge.
    #[must_use]
    pub fn new(initial: StereoFrame) -> Self {
        Self(Rc::new(RefCell::new(Edges {
            level: initial,
            retained_from: VClock::ZERO,
            changes: VecDeque::new(),
            end: None,
            covered_until: VClock::ZERO,
            error: None,
            needs_segment: false,
            device_level: initial,
            output_enabled: true,
        })))
    }

    /// Record a device edge; equal timestamps are allowed and the last wins.
    pub fn push(&self, at: VClock, level: StereoFrame) -> Result<(), AudioError> {
        let mut edges = self.0.borrow_mut();
        if edges.end.is_some()
            || at < edges.retained_from
            || at < edges.covered_until
            || edges.needs_segment
            || edges.changes.back().is_some_and(|(last, _)| at < *last)
        {
            return Err(AudioError::HistoryOrder);
        }
        edges.change(at, level);
        Ok(())
    }

    /// Begin a new ordered segment after a rejected device write or a sealed tail.
    /// History already observed cannot be repaired; resume at or after its frontier.
    pub fn start_segment(&self, at: VClock, level: StereoFrame) -> Result<(), AudioError> {
        let mut edges = self.0.borrow_mut();
        if at < edges.retained_from
            || at < edges.covered_until
            || edges.changes.back().is_some_and(|(last, _)| at < *last)
            || edges.end.is_some_and(|end| at < end)
        {
            return Err(AudioError::HistoryOrder);
        }
        if let Some(end) = edges.end.take() {
            edges.change(end, [0; 2]);
        }
        edges.change(at, level);
        edges.needs_segment = false;
        Ok(())
    }

    /// Infallible device callback: latch the first failure for the mixer, then
    /// re-report the current pin at recovery. Device tick/sync cannot return errors.
    pub fn record_level(&self, at: VClock, level: StereoFrame) {
        self.0.borrow_mut().device_level = level;
        let result = if self.0.borrow().needs_segment {
            self.start_segment(at, level)
        } else {
            self.push(at, level)
        };
        if let Err(error) = result {
            let mut edges = self.0.borrow_mut();
            edges.error.get_or_insert(error);
            edges.needs_segment = true;
        } else {
            self.0.borrow_mut().bound_unobserved(at);
        }
    }

    /// Silence from this clock onward, then retire once its tail is released.
    pub fn finish_at(&self, at: VClock) -> Result<(), AudioError> {
        let mut edges = self.0.borrow_mut();
        if at < edges.retained_from
            || at < edges.covered_until
            || edges.changes.back().is_some_and(|(last, _)| at < *last)
        {
            return Err(AudioError::HistoryOrder);
        }
        edges.end = Some(at);
        Ok(())
    }

    /// Pending timestamped edges, excluding the one retained baseline level.
    #[must_use]
    pub fn pending_edges(&self) -> usize {
        self.0.borrow().changes.len()
    }
}

impl AudioSource for EdgeSource {
    fn set_output_enabled(&mut self, enabled: bool) {
        let mut edges = self.0.borrow_mut();
        edges.output_enabled = enabled;
        let at = edges
            .changes
            .back()
            .map_or(edges.covered_until, |(at, _)| *at);
        edges.bound_unobserved(at);
    }

    fn take_error(&mut self) -> Option<AudioError> {
        let (error, recovery) = {
            let mut edges = self.0.borrow_mut();
            let error = edges.error.take();
            let recovery = (error.is_some() && edges.needs_segment).then(|| {
                let at = edges
                    .covered_until
                    .max(edges.retained_from)
                    .max(edges.changes.back().map_or(VClock::ZERO, |(at, _)| *at))
                    .max(edges.end.unwrap_or(VClock::ZERO));
                (at, edges.device_level)
            });
            (error, recovery)
        };
        if let Some((at, level)) = recovery {
            if let Err(error) = self.start_segment(at, level) {
                self.0.borrow_mut().error.get_or_insert(error);
            }
        }
        error
    }

    fn advance(&mut self, until: VClock) {
        let mut edges = self.0.borrow_mut();
        edges.covered_until = edges.covered_until.max(until);
    }

    fn render(&mut self, start: VClock, end: VClock) -> SampleArea {
        // Completed frames are never revisited, so long PWM histories scan once.
        self.release_before(start);
        let edges = self.0.borrow();
        let stop = edges.end.map_or(end, |stop| stop.min(end));
        let mut at = start;
        let mut level = edges.level;
        let mut area = [0i128; 2];
        for &(clock, next) in &edges.changes {
            if clock <= start {
                level = next;
                continue;
            }
            if clock >= stop {
                break;
            }
            for channel in 0..2 {
                area[channel] += i128::from(level[channel]) * i128::from(clock.0 - at.0);
            }
            at = clock;
            level = next;
        }
        if at < stop {
            for channel in 0..2 {
                area[channel] += i128::from(level[channel]) * i128::from(stop.0 - at.0);
            }
        }
        area
    }

    fn release_before(&mut self, retain_from: VClock) {
        self.0.borrow_mut().release(retain_from);
    }

    fn finished(&self) -> bool {
        let edges = self.0.borrow();
        edges.end.is_some_and(|end| edges.retained_from >= end)
    }
}

fn sample_boundary(origin: VClock, period: SamplePeriod, index: u128) -> Option<u128> {
    index
        .checked_mul(u128::from(period.numerator.get()))?
        .div_ceil(u128::from(period.denominator.get()))
        .checked_add(u128::from(origin.0))
}

pub(super) fn integrate_native(
    origin: VClock,
    period: SamplePeriod,
    start: VClock,
    end: VClock,
    mut sample: impl FnMut(u128) -> StereoFrame,
) -> SampleArea {
    let mut at = start.max(origin).0;
    let mut area = [0i128; 2];
    while at < end.0 {
        let index = period.sample_index(at - origin.0);
        let stop = sample_boundary(origin, period, index + 1)
            .map_or(end.0, |next| next.min(u128::from(end.0)) as u64);
        let frame = sample(index);
        for channel in 0..2 {
            area[channel] += i128::from(frame[channel]) * i128::from(stop - at);
        }
        at = stop;
    }
    area
}

/// One device-lifetime PCM reader. Writers change rate or pause by appending
/// timestamped segments to its shared buffer, without replacing this source.
/// Rendering moves retention to the interval's start; completed intervals are
/// never revisited, just as with `EdgeSource`.
pub struct RateSource {
    samples: SampleBuffer,
}

impl RateSource {
    /// Attach the device's timestamped segment history once.
    #[must_use]
    pub fn new(samples: SampleBuffer) -> Self {
        Self { samples }
    }
}

impl AudioSource for RateSource {
    fn render(&mut self, start: VClock, end: VClock) -> SampleArea {
        // The shared queue is this reader's cursor. Release each completed prefix
        // before rendering so a long observation never rescans its past frames.
        self.release_before(start);
        let mut samples = self.samples.0.borrow_mut();
        samples.covered_until = samples.covered_until.max(end);
        let mut area = [0; 2];
        for segment in &samples.segments {
            if segment.origin >= end {
                break;
            }
            let from = start.max(segment.origin);
            let stop = segment.end.map_or(end, |stop| stop.min(end));
            let native_end = sample_boundary(
                segment.origin,
                segment.period,
                segment.first + segment.frames.len() as u128,
            )
            .map_or(stop, |at| VClock(at.min(u128::from(stop.0)) as u64));
            if from >= native_end {
                continue;
            }
            let contribution =
                integrate_native(segment.origin, segment.period, from, native_end, |index| {
                    index
                        .checked_sub(segment.first)
                        .and_then(|offset| usize::try_from(offset).ok())
                        .and_then(|offset| segment.frames.get(offset).copied())
                        .unwrap_or([0; 2])
                });
            for channel in 0..2 {
                area[channel] += contribution[channel];
            }
        }
        area
    }

    fn advance(&mut self, until: VClock) {
        let mut samples = self.samples.0.borrow_mut();
        samples.covered_until = samples.covered_until.max(until);
    }

    fn release_before(&mut self, retain_from: VClock) {
        let mut samples = self.samples.0.borrow_mut();
        while let Some(segment) = samples.segments.front_mut() {
            if retain_from >= segment.origin {
                let index = segment
                    .period
                    .sample_index(retain_from.0 - segment.origin.0);
                let count = index
                    .saturating_sub(segment.first)
                    .min(segment.frames.len() as u128) as usize;
                segment.frames.drain(..count);
                segment.first += count as u128;
            }
            if segment.end.is_some_and(|end| end <= retain_from)
                || (segment.end.is_some() && segment.frames.is_empty())
            {
                samples.segments.pop_front();
            } else {
                // Ordered, nonoverlapping segments make every later one future
                // history once the first still-active segment has been reached.
                break;
            }
        }
    }

    fn finished(&self) -> bool {
        let samples = self.samples.0.borrow();
        samples.sealed
            && samples
                .segments
                .iter()
                .all(|segment| segment.frames.is_empty())
    }
}

struct Segment {
    origin: VClock,
    period: SamplePeriod,
    first: u128,
    frames: VecDeque<StereoFrame>,
    end: Option<VClock>,
}

#[derive(Default)]
struct Samples {
    segments: VecDeque<Segment>,
    covered_until: VClock,
    ordered_from: VClock,
    sealed: bool,
}

/// Shared timestamped native PCM segments for one reader and device writers.
/// Changes of rate and gaps are part of the same history. Push each sample no
/// later than its start; observation advance rejects retroactive PCM with
/// HistoryOrder. See `backlog/docs/design/audio.md` §Mixing and §Source lifecycle.
#[derive(Clone, Default)]
pub struct SampleBuffer(Rc<RefCell<Samples>>);

fn last_sample_start(
    origin: VClock,
    period: SamplePeriod,
    first: u128,
    count: usize,
) -> Result<VClock, AudioError> {
    sample_boundary(origin, period, first + count.saturating_sub(1) as u128)
        .and_then(|at| u64::try_from(at).ok())
        .map(VClock)
        .ok_or(AudioError::HistoryOrder)
}

impl SampleBuffer {
    /// Start a new rate/phase segment, cutting the previous held sample at origin.
    /// Gaps after a pause are silent. Samples and segments must be chronological.
    pub fn push_segment(
        &self,
        origin: VClock,
        period: SamplePeriod,
        frames: &[StereoFrame],
    ) -> Result<(), AudioError> {
        let mut samples = self.0.borrow_mut();
        if samples.sealed || origin < samples.covered_until || origin < samples.ordered_from {
            return Err(AudioError::HistoryOrder);
        }
        let last = last_sample_start(origin, period, 0, frames.len())?;
        // Several device writes can share one virtual tick. The replaced segment
        // then has no duration, so retaining it would grow history indefinitely.
        if samples
            .segments
            .back()
            .is_some_and(|segment| segment.origin == origin)
        {
            samples.segments.pop_back();
        }
        if let Some(previous) = samples.segments.back_mut() {
            previous.end = Some(previous.end.map_or(origin, |end| end.min(origin)));
        }
        samples.segments.push_back(Segment {
            origin,
            period,
            first: 0,
            frames: frames.iter().copied().collect(),
            end: None,
        });
        samples.ordered_from = last;
        Ok(())
    }

    /// Append native PCM to the current unpaused segment. Reject all of the batch
    /// if its first sample has already been covered; never repair history silently.
    pub fn push(&self, frames: &[StereoFrame]) -> Result<(), AudioError> {
        let mut samples = self.0.borrow_mut();
        if samples.sealed {
            return Err(AudioError::HistoryOrder);
        }
        let segment = samples
            .segments
            .back()
            .filter(|segment| segment.end.is_none())
            .ok_or(AudioError::HistoryOrder)?;
        let first = segment.first + segment.frames.len() as u128;
        let at = last_sample_start(segment.origin, segment.period, first, 1)?;
        let last = last_sample_start(segment.origin, segment.period, first, frames.len())?;
        if at < samples.covered_until || at < samples.ordered_from {
            return Err(AudioError::HistoryOrder);
        }
        if frames.is_empty() {
            return Ok(());
        }
        if let Some(segment) = samples.segments.back_mut() {
            segment.frames.extend(frames);
        }
        samples.ordered_from = last;
        Ok(())
    }

    /// Stop the held signal at this timestamp; resumption starts another segment.
    pub fn pause(&self, at: VClock) -> Result<(), AudioError> {
        let mut samples = self.0.borrow_mut();
        if samples.sealed || at < samples.covered_until || at < samples.ordered_from {
            return Err(AudioError::HistoryOrder);
        }
        if let Some(segment) = samples.segments.back_mut() {
            segment.end = Some(segment.end.map_or(at, |end| end.min(at)));
        }
        samples.ordered_from = at;
        Ok(())
    }

    /// End the device lifetime after the remaining native tail has been released.
    /// Rate changes and pauses do not seal the buffer.
    pub fn finish(&self) {
        self.0.borrow_mut().sealed = true;
    }

    /// Retained native frames, excluding released DMA history.
    #[must_use]
    pub fn buffered_frames(&self) -> usize {
        self.0
            .borrow()
            .segments
            .iter()
            .map(|segment| segment.frames.len())
            .sum()
    }

    /// Retained rate segments; the latest appendable segment may remain empty.
    #[must_use]
    pub fn buffered_segments(&self) -> usize {
        self.0.borrow().segments.len()
    }
}
