// SPDX-License-Identifier: AGPL-3.0-or-later
//! The meter fed with synthetic host durations and counters; nothing sleeps.

use super::*;

/// The AT's timer input, the clock a PC machine paces by.
const PIT: Rate = Rate::new(105_000_000, 88);

/// Feed the 61 presented frames of one host second starting at `from`, at
/// 60 Hz: the guest keeps `percent` of the clock and changes its picture on
/// the frames `changed` picks.
fn second(meter: &mut RateMeter, from: Duration, percent: u128, changed: impl Fn(u64) -> bool) {
    let mut picture = 0;
    for frame in 0..=60 {
        let elapsed = from + Duration::from_nanos(frame * 1_000_000_000 / 60);
        if frame > 0 && changed(frame) {
            picture += 1;
        }
        meter.observe(Sample {
            elapsed,
            ticks: (virtual_ticks(elapsed, PIT) * percent / 100) as u64,
            picture: Some(picture),
        });
    }
}

#[test]
fn keeping_the_configured_clock_reads_100_percent() {
    let mut meter = RateMeter::new(PIT);
    second(&mut meter, Duration::ZERO, 100, |f| f % 10 < 3);
    assert_eq!(
        meter.reading(),
        Some(Reading {
            speed_percent: 100,
            fps: 18
        })
    );
}

#[test]
fn a_host_behind_the_clock_reads_below_100_percent() {
    let mut meter = RateMeter::new(PIT);
    second(&mut meter, Duration::ZERO, 52, |f| f % 20 < 3);
    assert_eq!(
        meter.reading(),
        Some(Reading {
            speed_percent: 52,
            fps: 9
        })
    );
}

#[test]
fn unlimited_speed_reads_above_100_percent() {
    let mut meter = RateMeter::new(PIT);
    second(&mut meter, Duration::ZERO, 214, |f| f % 2 == 0);
    assert_eq!(
        meter.reading(),
        Some(Reading {
            speed_percent: 214,
            fps: 30
        })
    );
}

#[test]
fn fps_counts_only_frames_whose_picture_changed() {
    let mut meter = RateMeter::new(PIT);
    second(&mut meter, Duration::ZERO, 100, |_| false);
    assert_eq!(meter.reading().map(|r| r.fps), Some(0));
    let mut meter = RateMeter::new(PIT);
    second(&mut meter, Duration::ZERO, 100, |_| true);
    assert_eq!(meter.reading().map(|r| r.fps), Some(60));
}

#[test]
fn no_reading_before_the_first_full_window() {
    let mut meter = RateMeter::new(PIT);
    assert_eq!(meter.reading(), None);
    for millis in [0u64, 500, 999] {
        let elapsed = Duration::from_millis(millis);
        meter.observe(Sample {
            elapsed,
            ticks: virtual_ticks(elapsed, PIT) as u64,
            picture: Some(millis),
        });
        assert_eq!(meter.reading(), None, "at {millis} ms");
    }
}

#[test]
fn reset_forgets_the_reading_and_the_paused_interval() {
    let mut meter = RateMeter::new(PIT);
    second(&mut meter, Duration::ZERO, 100, |_| true);
    assert!(meter.reading().is_some());
    meter.reset();
    assert_eq!(meter.reading(), None);
    // Resuming after nine paused seconds: the clock stood still meanwhile,
    // but the first window after the reset starts at the resume.
    let resumed = Duration::from_secs(10);
    let stood = virtual_ticks(Duration::from_secs(9), PIT);
    let mut picture = 0;
    for frame in 0..=60 {
        let elapsed = resumed + Duration::from_nanos(frame * 1_000_000_000 / 60);
        picture += 1;
        meter.observe(Sample {
            elapsed,
            ticks: (virtual_ticks(elapsed, PIT) - stood) as u64,
            picture: Some(picture),
        });
        if frame < 60 {
            assert_eq!(meter.reading(), None, "frame {frame}");
        }
    }
    assert_eq!(
        meter.reading(),
        Some(Reading {
            speed_percent: 100,
            fps: 60
        })
    );
}

#[test]
fn the_next_window_starts_where_the_last_one_closed() {
    let mut meter = RateMeter::new(PIT);
    second(&mut meter, Duration::ZERO, 100, |_| true);
    let first = meter.reading();
    // The next second runs at half speed; the reading holds until it closes.
    let start = Duration::from_secs(1);
    let base = virtual_ticks(start, PIT);
    let mut picture = 60;
    for frame in 1..=60 {
        let elapsed = start + Duration::from_nanos(frame * 1_000_000_000 / 60);
        if frame % 2 == 0 {
            picture += 1;
        }
        meter.observe(Sample {
            elapsed,
            ticks: (base + virtual_ticks(elapsed - start, PIT) / 2) as u64,
            picture: Some(picture),
        });
        if frame < 60 {
            assert_eq!(meter.reading(), first, "frame {frame}");
        }
    }
    assert_eq!(
        meter.reading(),
        Some(Reading {
            speed_percent: 50,
            fps: 30
        })
    );
}

#[test]
fn reading_formats_whole_percent_and_fps() {
    let reading = Reading {
        speed_percent: 100,
        fps: 18,
    };
    assert_eq!(reading.to_string(), "100% · 18 fps");
    let reading = Reading {
        speed_percent: 3,
        fps: 0,
    };
    assert_eq!(reading.to_string(), "3% · 0 fps");
}
