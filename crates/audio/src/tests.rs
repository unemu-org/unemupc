// SPDX-License-Identifier: AGPL-3.0-or-later
//! Isolated signal tests; this seam has no port-mapped hardware or bus accesses.
//! See `backlog/docs/design/audio.md` §Validation.

use super::*;

fn period(numerator: u64, denominator: u64) -> SamplePeriod {
    SamplePeriod::new(
        NonZeroU64::new(numerator).unwrap(),
        NonZeroU64::new(denominator).unwrap(),
    )
}

/// Run in release mode with --ignored --exact --nocapture. Alternating periods
/// keep segments distinct even if a producer coalesces equal-rate PCM later.
#[test]
#[ignore]
fn native_segment_capture_scaling() {
    for count in [2048, 4096, 8192, 16384] {
        let build = std::time::Instant::now();
        let buffer = SampleBuffer::default();
        for index in 0..count {
            buffer
                .push_segment(
                    VClock(index * 24),
                    period(48 + index % 2, 2),
                    &[[123, -456]],
                )
                .unwrap();
        }
        let build = build.elapsed();
        let mut mixer = Mixer::new(VClock::ZERO);
        mixer.add_source(VClock::ZERO, Box::new(RateSource::new(buffer.clone())));
        let end = VClock(count * 24);
        let mut capture = CaptureAudioSink::with_frame_limit(count as usize);
        let render = std::time::Instant::now();
        mixer.render_until(end, &mut capture).unwrap();
        let render = render.elapsed();
        assert_eq!(capture.frame_count(), completed_frames(end));
        assert!(capture.frames().iter().all(|frame| *frame == [123, -456]));
        eprintln!(
            "segments={count} frames={} build_s={:.6} render_s={:.6}",
            capture.frame_count(),
            build.as_secs_f64(),
            render.as_secs_f64(),
        );
        std::hint::black_box(capture);
    }
}

#[test]
fn native_render_releases_consumed_prefix_without_an_observation_release() {
    let buffer = SampleBuffer::default();
    for index in 0..256 {
        buffer
            .push_segment(VClock(index * 24), period(24, 1), &[[index as i16; 2]])
            .unwrap();
    }
    let mut reader = RateSource::new(buffer.clone());
    for index in [0, 1, 127, 255] {
        assert_eq!(
            reader.render(VClock(index * 24), VClock((index + 1) * 24)),
            [i128::from(index) * 24; 2]
        );
        assert_eq!(buffer.buffered_segments(), (256 - index) as usize);
        assert_eq!(buffer.buffered_frames(), (256 - index) as usize);
    }
    assert_eq!(
        buffer.push_segment(VClock(256 * 24 - 1), period(24, 1), &[[1; 2]]),
        Err(AudioError::HistoryOrder)
    );
}

struct Signal {
    origin: VClock,
    period: SamplePeriod,
    sample: fn(u128) -> StereoFrame,
}

impl AudioSource for Signal {
    fn render(&mut self, start: VClock, end: VClock) -> SampleArea {
        super::sources::integrate_native(self.origin, self.period, start, end, self.sample)
    }
}

fn source(period: SamplePeriod, sample: fn(u128) -> StereoFrame) -> Box<dyn AudioSource> {
    Box::new(Signal {
        origin: VClock::ZERO,
        period,
        sample,
    })
}

#[test]
fn pit_mapping_is_exact_at_small_and_large_boundaries() {
    for (clock, expected) in [
        (0, 0),
        (24, 0),
        (25, 1),
        (49, 1),
        (50, 2),
        (4374, 175),
        (4375, 176),
        (1_193_181, 47_999),
        (1 << 40, 44_231_782_054),
        (u64::MAX, 742_086_161_593_801_390),
    ] {
        assert_eq!(completed_frames(VClock(clock)), expected, "clock={clock}");
    }
    for n in [0, 1, 2, 175, 176, 44_231_782_054, 742_086_161_593_801_389] {
        let boundary = frame_boundary(n + 1) as u64;
        assert_eq!(completed_frames(VClock(boundary)), n + 1);
        assert_eq!(completed_frames(VClock(boundary - 1)), n);
    }
}

#[test]
fn consecutive_arbitrary_intervals_keep_every_frame_once() {
    for (start, expected) in [
        (0, [0, 0, 1, 0, 0, 176, 0, 42, 0, 329]),
        ((1 << 40) - 7, [0, 0, 1, 0, 0, 176, 1, 41, 0, 329]),
        (u64::MAX - 100_000, [0, 0, 1, 0, 0, 176, 1, 41, 0, 330]),
    ] {
        let mut mixer = Mixer::new(VClock(start));
        mixer.add_source(VClock::ZERO, source(period(1, 1), |_| [42, -17]));
        let mut capture = CaptureAudioSink::with_frame_limit(10_000);
        let mut at = start;
        for (delta, expected_count) in [0, 1, 24, 1, 2, 4374, 19, 1024, 0, 8191]
            .into_iter()
            .zip(expected)
        {
            let end = at + delta;
            let before = capture.frame_count();
            mixer.render_until(VClock(end), &mut capture).unwrap();
            assert_eq!(capture.frame_count() - before, expected_count);
            at = end;
        }
        assert_eq!(
            capture.frame_count(),
            completed_frames(VClock(at)) - completed_frames(VClock(start))
        );
    }
}

#[test]
fn constant_ramp_and_square_resample_at_native_rates_bit_exactly() {
    for (rate, ramp, square) in [
        (
            period(1, 1),
            [12, 37, 62, 87, 112, 137, 162, 186, 211, 236],
            [360, -360, 360, -360, 360, -360, 360, 0, -360, 360],
        ),
        (
            period(50, 1),
            [0, 0, 1, 1, 2, 2, 3, 3, 3, 4],
            [
                9000, 9000, -9000, -9000, 9000, 9000, -9000, -9000, 8280, 9000,
            ],
        ),
        (
            period(4375, 176),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            [
                9000, -9000, 9000, -9000, 9000, -9000, 9000, -9000, 9000, -9000,
            ],
        ),
        (
            period(4375, 88),
            [0, 0, 1, 1, 2, 2, 3, 3, 4, 4],
            [
                9000, 9000, -9000, -9000, 9000, 9000, -9000, -9000, 9000, 9000,
            ],
        ),
        (
            period(4375, 352),
            [0, 2, 4, 6, 8, 10, 12, 14, 16, 18],
            [360, 360, 360, 360, -360, -360, -360, 0, 360, 360],
        ),
    ] {
        for shape in 0..3 {
            let signal: fn(u128) -> StereoFrame = match shape {
                0 => |_| [1234, -2345],
                1 => |n| [n as i16, -(n as i16)],
                _ => |n| {
                    if n.is_multiple_of(2) {
                        [9000, -9000]
                    } else {
                        [-9000, 9000]
                    }
                },
            };
            let mut mixer = Mixer::new(VClock::ZERO);
            mixer.add_source(VClock::ZERO, source(rate, signal));
            let mut capture = CaptureAudioSink::with_frame_limit(10);
            mixer.render_until(VClock(249), &mut capture).unwrap();
            let expected: Vec<_> = (0..10)
                .map(|i| match shape {
                    0 => [1234, -2345],
                    1 => [ramp[i], -ramp[i]],
                    _ => [square[i], -square[i]],
                })
                .collect();
            assert_eq!(capture.frames(), expected, "rate={rate:?}, shape={shape}");
        }
    }
}

#[test]
fn source_origins_and_fractional_native_periods_use_pit_time() {
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(
        VClock(26),
        Box::new(Signal {
            origin: VClock(26),
            period: period(3, 2),
            sample: |n| [n as i16, 17],
        }),
    );
    let mut capture = CaptureAudioSink::with_frame_limit(10_000);
    mixer.render_until(VClock(125), &mut capture).unwrap();
    assert_eq!(
        capture.frames(),
        [[0, 0], [7, 16], [23, 17], [40, 17], [57, 17]]
    );
}

#[test]
fn mixing_saturates_both_channels_after_summing_all_sources() {
    let mut mixer = Mixer::new(VClock::ZERO);
    for _ in 0..2 {
        mixer.add_source(VClock::ZERO, source(period(1, 1), |_| [30_000, -30_000]));
    }
    let mut capture = CaptureAudioSink::with_frame_limit(10_000);
    mixer.render_until(VClock(25), &mut capture).unwrap();
    assert_eq!(capture.frames(), [[i16::MAX, i16::MIN]]);
    for values in [[30_000, 30_000, -30_000], [-30_000, 30_000, 30_000]] {
        struct Constant(i16);
        impl AudioSource for Constant {
            fn render(&mut self, start: VClock, end: VClock) -> SampleArea {
                [i128::from(self.0) * i128::from(end.0 - start.0); 2]
            }
        }
        let mut mixer = Mixer::new(VClock::ZERO);
        for value in values {
            mixer.add_source(VClock::ZERO, Box::new(Constant(value)));
        }
        let mut capture = CaptureAudioSink::with_frame_limit(10_000);
        mixer.render_until(VClock(25), &mut capture).unwrap();
        assert_eq!(capture.frames(), [[30_000; 2]]);
    }
}

#[test]
fn capture_hash_is_little_endian_pcm_with_no_block_metadata() {
    let frames = [[1, -2], [i16::MIN, i16::MAX]];
    let mut capture = CaptureAudioSink::with_frame_limit(10_000);
    capture.present(AudioBlock::new(VClock(0), VClock(50), &frames).unwrap());
    assert_eq!(capture.frame_count(), 2);
    assert_eq!(capture.pcm_le(), [1, 0, 254, 255, 0, 128, 255, 127]);
    assert_eq!(capture.hash(), 0x82616ee7180d0629);
    let mut split = CaptureAudioSink::with_frame_limit(10_000);
    split.present(AudioBlock::new(VClock(0), VClock(26), &frames[..1]).unwrap());
    split.present(AudioBlock::new(VClock(26), VClock(50), &frames[1..]).unwrap());
    assert_eq!(capture, split);
}

#[test]
fn rendered_capture_is_identical_across_runs_and_observation_splits() {
    fn render(start: u64, ends: &[u64]) -> CaptureAudioSink {
        let mut mixer = Mixer::new(VClock(start));
        mixer.add_source(
            VClock::ZERO,
            source(period(105_000_000, 88 * 44_100), |n| {
                let value = (n % 10_000) as i16;
                [value, -value]
            }),
        );
        let mut capture = CaptureAudioSink::with_frame_limit(10_000);
        for &end in ends {
            mixer.render_until(VClock(end), &mut capture).unwrap();
        }
        capture
    }
    for start in [0, 1 << 40] {
        let end = start + 100_001;
        let whole = render(start, &[end]);
        let splits: Vec<_> = [0, 1, 24, 25, 26, 50, 4375, 65_537, 100_001]
            .map(|n| start + n)
            .into();
        assert_eq!(whole, render(start, &splits));
        assert_eq!(whole, render(start, &[end]));
        assert_eq!(
            whole.frame_count(),
            completed_frames(VClock(end)) - completed_frames(VClock(start))
        );
    }
}

#[test]
fn internal_batches_cover_exact_touching_intervals() {
    struct Check {
        until: VClock,
        calls: usize,
        frames: u64,
    }
    impl AudioSink for Check {
        fn present(&mut self, block: AudioBlock<'_>) {
            assert_eq!(block.start(), self.until);
            AudioBlock::new(block.start(), block.end(), block.frames()).unwrap();
            assert!(block.frames().len() <= 1024);
            self.frames += block.frames().len() as u64;
            self.until = block.end();
            self.calls += 1;
        }
    }
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, source(period(1, 1), |_| [1, 2]));
    let mut sink = Check {
        until: VClock::ZERO,
        calls: 0,
        frames: 0,
    };
    for end in [1, 24, 25, 100_000] {
        mixer.render_until(VClock(end), &mut sink).unwrap();
    }
    assert_eq!(sink.until, VClock(100_000));
    assert_eq!(sink.frames, 4022);
    assert_eq!(sink.calls, 7);
}

#[test]
fn null_sink_and_source_free_mixer_do_no_sampling_or_delivery() {
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(
        VClock::ZERO,
        source(period(1, 1), |_| panic!("disabled sink sampled a source")),
    );
    mixer
        .render_until(VClock(u64::MAX), &mut NullAudioSink)
        .unwrap();
    assert!(!NullAudioSink.enabled());
    struct NoDelivery;
    impl AudioSink for NoDelivery {
        fn present(&mut self, _: AudioBlock<'_>) {
            panic!("source-free mixer emitted PCM");
        }
    }
    Mixer::new(VClock::ZERO)
        .render_until(VClock(u64::MAX), &mut NoDelivery)
        .unwrap();
}

#[test]
fn malformed_blocks_and_time_rewind_are_rejected_without_losing_position() {
    assert_eq!(
        AudioBlock::new(VClock(10), VClock(9), &[]).unwrap_err(),
        AudioError::TimeReversed
    );
    assert_eq!(
        AudioBlock::new(VClock(0), VClock(25), &[]).unwrap_err(),
        AudioError::FrameCount
    );
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, source(period(1, 1), |_| [1, 2]));
    let mut capture = CaptureAudioSink::with_frame_limit(10_000);
    mixer.render_until(VClock(26), &mut capture).unwrap();
    assert_eq!(
        mixer.render_until(VClock(25), &mut capture),
        Err(AudioError::TimeReversed)
    );
    mixer.render_until(VClock(50), &mut capture).unwrap();
    assert_eq!(capture.frames(), [[1, 2], [1, 2]]);
}

#[test]
fn history_release_runs_for_disabled_and_empty_observations_and_retains_partial_frame() {
    use std::{cell::RefCell, rc::Rc};
    struct History(Rc<RefCell<Vec<VClock>>>);
    impl AudioSource for History {
        fn render(&mut self, _: VClock, _: VClock) -> SampleArea {
            panic!("disabled render");
        }
        fn release_before(&mut self, clock: VClock) {
            self.0.borrow_mut().push(clock);
        }
    }
    let releases = Rc::new(RefCell::new(Vec::new()));
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(History(releases.clone())));
    for end in [0, 1, 26, 26, 49, 50] {
        mixer.render_until(VClock(end), &mut NullAudioSink).unwrap();
    }
    assert_eq!(
        *releases.borrow(),
        [
            VClock(0),
            VClock(0),
            VClock(25),
            VClock(25),
            VClock(25),
            VClock(50)
        ]
    );
    let edges = EdgeSource::new([0; 2]);
    for at in 0..100 {
        edges.push(VClock(at), [at as i16; 2]).unwrap();
    }
    let samples = SampleBuffer::default();
    samples
        .push_segment(VClock::ZERO, period(1, 1), &[[123; 2]; 100])
        .unwrap();
    mixer.add_source(VClock::ZERO, Box::new(edges.clone()));
    mixer.add_source(VClock::ZERO, Box::new(RateSource::new(samples.clone())));
    mixer.render_until(VClock(100), &mut NullAudioSink).unwrap();
    assert_eq!(edges.pending_edges(), 0);
    assert_eq!(samples.buffered_frames(), 0);
}

#[test]
fn finished_sources_are_never_rendered_or_released_again() {
    use std::{cell::Cell, rc::Rc};
    struct Retiring {
        retired: Rc<Cell<bool>>,
        calls: Rc<Cell<usize>>,
    }
    impl AudioSource for Retiring {
        fn render(&mut self, start: VClock, end: VClock) -> SampleArea {
            self.calls.set(self.calls.get() + 1);
            [i128::from(end.0 - start.0); 2]
        }
        fn release_before(&mut self, _: VClock) {
            self.calls.set(self.calls.get() + 1);
        }
        fn finished(&self) -> bool {
            self.retired.get()
        }
    }
    let retired = Rc::new(Cell::new(false));
    let calls = Rc::new(Cell::new(0));
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(
        VClock::ZERO,
        Box::new(Retiring {
            retired: retired.clone(),
            calls: calls.clone(),
        }),
    );
    let mut sink = CaptureAudioSink::new();
    mixer.render_until(VClock(25), &mut sink).unwrap();
    assert_eq!(calls.get(), 2);
    retired.set(true);
    mixer.render_until(VClock(1000), &mut sink).unwrap();
    mixer
        .render_until(VClock(2000), &mut NullAudioSink)
        .unwrap();
    assert_eq!(calls.get(), 2);
    assert!(mixer.sources.is_empty());
    assert_eq!(sink.frame_count(), 40);
}

#[test]
fn fast_square_and_pwm_edges_are_box_filtered_by_pit_clock_duration() {
    let square = EdgeSource::new([0; 2]);
    for at in 1..250 {
        square
            .push(VClock(at), [if at % 2 == 0 { 0 } else { 24_000 }; 2])
            .unwrap();
    }
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(square));
    let mut sink = CaptureAudioSink::with_frame_limit(10);
    mixer.render_until(VClock(249), &mut sink).unwrap();
    assert_eq!(sink.frames()[7], [12_000; 2]);
    assert_eq!(sink.frames()[0], [11_520; 2]);
    let pwm = EdgeSource::new([10_000; 2]);
    for at in [10, 50, 90] {
        pwm.push(VClock(at), [0; 2]).unwrap();
        if at < 90 {
            pwm.push(VClock(at + 30), [10_000; 2]).unwrap();
        }
    }
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(pwm));
    let mut sink = CaptureAudioSink::with_frame_limit(4);
    mixer.render_until(VClock(100), &mut sink).unwrap();
    assert_eq!(sink.frames(), [[4000; 2], [4000; 2], [0; 2], [4000; 2]]);
}

#[test]
fn edges_inside_frame_and_at_its_end_have_exact_clock_weights() {
    let edges = EdgeSource::new([-1000, 2000]);
    edges.push(VClock(7), [2000, -1000]).unwrap();
    edges.push(VClock(19), [-500, 3000]).unwrap();
    edges.push(VClock(25), [123, -123]).unwrap();
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(edges));
    let mut sink = CaptureAudioSink::with_frame_limit(2);
    mixer.render_until(VClock(50), &mut sink).unwrap();
    assert_eq!(sink.frames(), [[560, 800], [123, -123]]);
}

#[test]
fn sealed_rate_and_edge_buffers_release_tails_and_retire() {
    let samples = SampleBuffer::default();
    samples
        .push_segment(VClock::ZERO, period(25, 1), &[[100; 2], [200; 2]])
        .unwrap();
    samples.finish();
    assert_eq!(samples.push(&[[300; 2]]), Err(AudioError::HistoryOrder));
    let edges = EdgeSource::new([1000; 2]);
    edges.finish_at(VClock(25)).unwrap();
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(RateSource::new(samples.clone())));
    mixer.add_source(VClock::ZERO, Box::new(edges.clone()));
    let mut sink = CaptureAudioSink::with_frame_limit(4);
    mixer.render_until(VClock(100), &mut sink).unwrap();
    assert_eq!(sink.frames(), [[1100; 2], [200; 2], [0; 2], [0; 2]]);
    assert!(mixer.sources.is_empty());
    assert_eq!(samples.buffered_frames(), 0);
    assert!(edges.finished());
}

#[test]
fn capture_hash_and_count_cover_full_stream_with_bounded_or_no_retention() {
    let frames = [[1, -2]; 4];
    let mut full = CaptureAudioSink::with_frame_limit(4);
    let mut bounded = CaptureAudioSink::with_frame_limit(1);
    let mut hash_only = CaptureAudioSink::new();
    for sink in [&mut full, &mut bounded, &mut hash_only] {
        sink.present(AudioBlock::new(VClock(0), VClock(100), &frames).unwrap());
        assert_eq!(sink.frame_count(), 4);
    }
    assert_eq!(full.hash(), bounded.hash());
    assert_eq!(full.hash(), hash_only.hash());
    assert_eq!(bounded.frames(), &frames[..1]);
    assert!(hash_only.frames().is_empty());
    let ptr = bounded.frames().as_ptr();
    assert_eq!(ptr, bounded.frames().as_ptr());
    bounded.present(AudioBlock::new(VClock(100), VClock(125), &frames[..1]).unwrap());
    assert_eq!(bounded.frame_count(), 5);
    assert_eq!(bounded.frames().as_ptr(), ptr);
    assert_eq!(bounded.frames().len(), 1);
}

#[test]
fn rendered_edge_and_native_rate_mix_has_frozen_hash_across_block_splits() {
    fn render(ends: &[u64]) -> CaptureAudioSink {
        let edges = EdgeSource::new([1000, -1000]);
        for (at, level) in [
            (7, [-2000, 3000]),
            (31, [4000, 1000]),
            (333, [-123, 456]),
            (2000, [0; 2]),
        ] {
            edges.push(VClock(at), level).unwrap();
        }
        let pcm = SampleBuffer::default();
        let frames: Vec<_> = (0..200)
            .map(|n| [(n * 37 % 2001 - 1000) as i16, (n * 19 % 1001 - 500) as i16])
            .collect();
        pcm.push_segment(VClock::ZERO, period(105_000_000, 88 * 44_100), &frames)
            .unwrap();
        let mut mixer = Mixer::new(VClock::ZERO);
        mixer.add_source(VClock::ZERO, Box::new(edges));
        mixer.add_source(VClock::ZERO, Box::new(RateSource::new(pcm)));
        mixer.add_source(
            VClock::ZERO,
            source(period(105_000_000, 88 * 8000), |n| {
                if n % 2 == 0 {
                    [123, -321]
                } else {
                    [-123, 321]
                }
            }),
        );
        let mut sink = CaptureAudioSink::with_frame_limit(176);
        for &end in ends {
            mixer.render_until(VClock(end), &mut sink).unwrap();
        }
        sink
    }
    let whole = render(&[4375]);
    let split = render(&[1, 24, 25, 26, 100, 1024, 2027, 4375]);
    assert_eq!(whole.frame_count(), 176);
    assert_eq!(whole, split);
    assert_eq!(whole.hash(), 0x2b19e84103e1b4b2);
}

#[test]
fn retired_tail_keeps_silent_stream_hash_and_length_independent_of_splits() {
    fn render(ends: &[u64]) -> CaptureAudioSink {
        let samples = SampleBuffer::default();
        samples
            .push_segment(VClock::ZERO, period(25, 1), &[[100; 2], [200; 2]])
            .unwrap();
        samples.finish();
        let mut mixer = Mixer::new(VClock::ZERO);
        mixer.add_source(VClock::ZERO, Box::new(RateSource::new(samples)));
        let mut sink = CaptureAudioSink::with_frame_limit(4);
        for &end in ends {
            mixer.render_until(VClock(end), &mut sink).unwrap();
        }
        sink
    }
    assert_eq!(render(&[100]), render(&[25, 50, 75, 100]));
    assert_eq!(
        render(&[100]).frames(),
        [[100; 2], [200; 2], [0; 2], [0; 2]]
    );
}

#[test]
fn rate_frames_blend_native_boundaries_at_44100_and_8000() {
    for (origin, hz, frame_index, expected) in
        [(0, 44_100, 1, [1880, 2520]), (7, 8_000, 6, [1720, 1880])]
    {
        let buffer = SampleBuffer::default();
        buffer
            .push_segment(
                VClock(origin),
                period(105_000_000, 88 * hz),
                &[[1000, -1000]],
            )
            .unwrap();
        buffer.push(&[[2000, 3000]]).unwrap();
        let mut mixer = Mixer::new(VClock::ZERO);
        mixer.add_source(VClock::ZERO, Box::new(RateSource::new(buffer)));
        let mut capture = CaptureAudioSink::with_frame_limit(7);
        mixer
            .render_until(
                VClock(frame_boundary(frame_index as u64 + 1) as u64),
                &mut capture,
            )
            .unwrap();
        assert_eq!(capture.frames()[frame_index], expected, "native rate {hz}");
    }
}

#[test]
fn timestamped_rate_changes_and_pause_keep_one_source_across_frames() {
    fn render(ends: &[u64]) -> CaptureAudioSink {
        let buffer = SampleBuffer::default();
        buffer
            .push_segment(VClock(0), period(10, 1), &[[100; 2], [300; 2]])
            .unwrap();
        buffer
            .push_segment(VClock(12), period(4, 1), &[[500; 2], [700; 2]])
            .unwrap();
        buffer.pause(VClock(19)).unwrap();
        buffer.pause(VClock(21)).unwrap();
        buffer
            .push_segment(VClock(22), period(8, 1), &[[900; 2], [1100; 2], [1300; 2]])
            .unwrap();
        let mut mixer = Mixer::new(VClock::ZERO);
        mixer.add_source(VClock::ZERO, Box::new(RateSource::new(buffer.clone())));
        let mut capture = CaptureAudioSink::with_frame_limit(5);
        for &end in ends {
            mixer.render_until(VClock(end), &mut capture).unwrap();
        }
        assert_eq!(buffer.buffered_frames(), 0);
        assert_eq!(mixer.sources.len(), 1);
        buffer
            .push_segment(VClock(60), period(50, 1), &[[1700; 2]])
            .unwrap();
        mixer.render_until(VClock(100), &mut capture).unwrap();
        assert_eq!(mixer.sources.len(), 1);
        buffer.finish();
        mixer.render_until(VClock(125), &mut capture).unwrap();
        assert!(mixer.sources.is_empty());
        capture
    }
    let whole = render(&[50]);
    assert_eq!(
        whole.frames(),
        [[336; 2], [948; 2], [1020; 2], [1700; 2], [680; 2]]
    );
    assert_eq!(whole, render(&[1, 13, 24, 25, 26, 37, 50]));
}

#[test]
fn segment_release_stays_bounded_with_disabled_output() {
    let buffer = SampleBuffer::default();
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(RateSource::new(buffer.clone())));
    for cycle in 0..200 {
        let origin = cycle * 50;
        buffer
            .push_segment(VClock(origin), period(7, 1), &[[1; 2], [2; 2], [3; 2]])
            .unwrap();
        buffer.pause(VClock(origin + 17)).unwrap();
        buffer
            .push_segment(VClock(origin + 20), period(5, 1), &[[4; 2], [5; 2], [6; 2]])
            .unwrap();
        mixer
            .render_until(VClock(origin + 50), &mut NullAudioSink)
            .unwrap();
        assert!(buffer.buffered_frames() <= 2);
        assert!(buffer.buffered_segments() <= 1);
        assert_eq!(mixer.sources.len(), 1);
    }
}

#[test]
fn exact_areas_are_summed_before_the_single_frame_division() {
    let first = EdgeSource::new([1, -1]);
    first.push(VClock(10), [0; 2]).unwrap();
    let second = EdgeSource::new([0; 2]);
    second.push(VClock(10), [1, -1]).unwrap();
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(first));
    mixer.add_source(VClock::ZERO, Box::new(second));
    let mut capture = CaptureAudioSink::with_frame_limit(1);
    mixer.render_until(VClock(25), &mut capture).unwrap();
    assert_eq!(capture.frames(), [[1, -1]]);
}

#[test]
fn late_native_samples_and_segments_are_rejected_without_rewriting_silence() {
    let buffer = SampleBuffer::default();
    buffer
        .push_segment(VClock::ZERO, period(10, 1), &[[1000; 2]])
        .unwrap();
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(RateSource::new(buffer.clone())));
    let mut capture = CaptureAudioSink::with_frame_limit(2);
    mixer.render_until(VClock(25), &mut capture).unwrap();
    assert_eq!(capture.frames(), [[400; 2]]);
    assert_eq!(buffer.push(&[[2000; 2]]), Err(AudioError::HistoryOrder));
    assert_eq!(
        buffer.push_segment(VClock(24), period(10, 1), &[[2000; 2]]),
        Err(AudioError::HistoryOrder)
    );
    assert_eq!(buffer.pause(VClock(24)), Err(AudioError::HistoryOrder));
    assert_eq!(buffer.buffered_frames(), 0);
    buffer
        .push_segment(VClock(25), period(10, 1), &[[3000; 2]])
        .unwrap();
    mixer.render_until(VClock(50), &mut capture).unwrap();
    assert_eq!(capture.frames(), [[400; 2], [1200; 2]]);
    let empty = SampleBuffer::default();
    empty
        .push_segment(VClock::ZERO, period(10, 1), &[])
        .unwrap();
    let mut disabled = Mixer::new(VClock::ZERO);
    disabled.add_source(VClock::ZERO, Box::new(RateSource::new(empty.clone())));
    disabled
        .render_until(VClock(1), &mut NullAudioSink)
        .unwrap();
    assert_eq!(empty.push(&[[1; 2]]), Err(AudioError::HistoryOrder));
    empty
        .push_segment(VClock(1), period(10, 1), &[[3; 2]])
        .unwrap();
    disabled
        .render_until(VClock(26), &mut NullAudioSink)
        .unwrap();
    assert_eq!(empty.push(&[[4; 2]]), Err(AudioError::HistoryOrder));
}

#[test]
fn device_edge_error_is_latched_and_recovery_starts_a_segment() {
    let edges = EdgeSource::new([0; 2]);
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(edges.clone()));
    mixer.render_until(VClock(10), &mut NullAudioSink).unwrap();
    edges.record_level(VClock(7), [16384; 2]);
    edges.record_level(VClock(8), [16384; 2]);
    assert_eq!(
        mixer.render_until(VClock(10), &mut NullAudioSink),
        Err(AudioError::HistoryOrder)
    );
    edges.record_level(VClock(10), [16384; 2]);
    let mut sink = CaptureAudioSink::with_frame_limit(10);
    mixer.render_until(VClock(25), &mut sink).unwrap();
    assert_eq!(sink.frames(), &[[9830; 2]]);
    assert_eq!(edges.pending_edges(), 0);
}

#[test]
fn speaker_pwm_history_is_box_filtered_and_released() {
    let edges = EdgeSource::new([0; 2]);
    for (at, high) in [
        (7, true),
        (13, false),
        (29, true),
        (31, false),
        (48, true),
        (52, false),
        (73, true),
        (99, false),
    ] {
        edges.record_level(VClock(at), [if high { 16384 } else { 0 }; 2]);
    }
    let mut mixer = Mixer::new(VClock::ZERO);
    mixer.add_source(VClock::ZERO, Box::new(edges.clone()));
    let mut sink = CaptureAudioSink::with_frame_limit(10);
    mixer.render_until(VClock(100), &mut sink).unwrap();
    assert_eq!(
        sink.frames(),
        &[[3932; 2], [2621; 2], [2621; 2], [15728; 2]]
    );
    assert_eq!(edges.pending_edges(), 0);
}

#[test]
fn rejected_device_edge_recovers_current_level_without_another_pin_change() {
    for (initial, current, expected) in [(0, 16384, 9830), (16384, 0, 6553)] {
        let edges = EdgeSource::new([initial; 2]);
        let mut mixer = Mixer::new(VClock::ZERO);
        mixer.add_source(VClock::ZERO, Box::new(edges.clone()));
        mixer.render_until(VClock(10), &mut NullAudioSink).unwrap();
        edges.record_level(VClock(7), [current; 2]);
        assert_eq!(
            mixer.render_until(VClock(10), &mut NullAudioSink),
            Err(AudioError::HistoryOrder)
        );
        let mut sink = CaptureAudioSink::with_frame_limit(1);
        mixer.render_until(VClock(25), &mut sink).unwrap();
        assert_eq!(sink.frames(), &[[expected; 2]]);
    }
}

#[test]
fn clock_domain_output_frames_follow_exact_sample_edges() {
    use device::clock::AUDIO_OUTPUT;
    for n in [1, 2, 176, 1_000_000_000] {
        let end = AUDIO_OUTPUT.edge_at(n);
        assert_eq!(completed_frames(end), n);
        assert_eq!(completed_frames(VClock(end.0 - 1)), n - 1);
    }
}
