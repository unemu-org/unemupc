// SPDX-License-Identifier: AGPL-3.0-or-later
//! IBM Personal Computer AT Technical Reference, System Board, Speaker;
//! Intel 8254 datasheet, Mode 3. Synthetic ROMs exercise the actual AT wiring.
//! PCM is pinned against x86jit revision b7c10b6fdda8f84d3eda9c5d7751fcda1c5893ed.

use assembly::{AssemblyOptions, AudioSinkKind, ObservationPolicy, RunPolicy};
use machine_file::assemble_with_observation_and_options;

fn capture(program: &[u8], until: u64, splits: &[u64]) -> (u64, u64, Vec<[i16; 2]>) {
    let mut rom = vec![0; 32768];
    rom[..program.len()].copy_from_slice(program);
    rom[32752..32757].copy_from_slice(&[0xEA, 0, 0, 0, 0xF8]);
    let mut config = assembly::bios(rom, Some(vec![0; 1_474_560]));
    config.clock.retired_per_pit_clock = 1;
    config.clock.retired_per_pit_clock_denominator = 1;
    let mut policy = ObservationPolicy::default();
    policy.output.audio_sink = AudioSinkKind::Capture;
    policy.output.audio_capture_frame_limit = 1024;
    let mut rig = assemble_with_observation_and_options(
        &config,
        &policy,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    assert_eq!(config.board, assembly::BoardModel::Ibm5170);
    for &cap in splits.iter().chain(std::iter::once(&until)) {
        rig.run_session(&RunPolicy::natural(Some(cap), None))
            .unwrap();
    }
    assert_eq!(rig.machine.clock().0, until);
    let audio = rig.observation.captured_audio().unwrap();
    (audio.frame_count(), audio.hash(), audio.frames().to_vec())
}

fn reference(until: u64, high: impl Fn(u64) -> bool) -> Vec<[i16; 2]> {
    (0..until * 176 / 4375)
        .map(|n| {
            let start = (n * 4375).div_ceil(176);
            let end = ((n + 1) * 4375).div_ceil(176);
            // The driven speaker enters the host mix at 8192 (-12 dBFS).
            let area: i64 = (start..end).map(|at| if high(at) { 8192 } else { 0 }).sum();
            [(area / (end - start) as i64) as i16; 2]
        })
        .collect()
}

#[test]
fn at_speaker_mode3_tone_has_frozen_pcm() {
    // The reset jump contributes one instruction: enable is at clock 9,
    // load at 10, then each high half lasts 597 clocks and each low half 596.
    let mut program = vec![
        0xFA, 0xB0, 0xB6, 0xE6, 0x43, 0xB0, 0xA9, 0xE6, 0x42, 0xB0, 4, 0xE6, 0x42, 0xB0, 3, 0xE6,
        0x61,
    ];
    // Two final instructions put disable at clock 11939: ten full periods later.
    program.extend(std::iter::repeat_n(0x90, 11928));
    program.extend_from_slice(&[0xB0, 0, 0xE6, 0x61, 0xEB, 0xFE]);
    let whole = capture(&program, 12000, &[]);
    let split = capture(&program, 12000, &[10, 24, 607, 1203, 6000, 11939]);
    assert_eq!(whole, split);
    assert_eq!(whole.0, 482);
    assert_eq!(
        whole.2,
        reference(12000, |at| (9..11939).contains(&at)
            && (at < 10 || (at - 10) % 1193 < 597))
    );
    assert_eq!(whole.1, 0xe2a3_39c1_952c_c419);
}

#[test]
fn at_speaker_eight_bit1_toggles_have_exact_box_filtered_positions() {
    let mut program = vec![0xFA];
    let mut clock = 2;
    for (at, value) in [
        (7, 2),
        (13, 0),
        (29, 2),
        (31, 0),
        (48, 2),
        (52, 0),
        (73, 2),
        (99, 0),
    ] {
        while clock + 1 < at {
            program.push(0x90);
            clock += 1;
        }
        program.extend_from_slice(&[0xB0, value, 0xE6, 0x61]);
        clock += 2;
    }
    program.extend_from_slice(&[0xEB, 0xFE]);
    let whole = capture(&program, 125, &[]);
    assert_eq!(whole, capture(&program, 125, &[8, 14, 30, 49, 74, 99]));
    assert_eq!(whole.0, 5);
    assert_eq!(whole.1, 0x41b2_593d_aa7d_fc61);
    assert_eq!(
        whole.2,
        vec![[1966; 2], [1310; 2], [1310; 2], [7864; 2], [0; 2]]
    );
    assert_eq!(
        whole.2,
        reference(125, |at| (7..13).contains(&at)
            || (29..31).contains(&at)
            || (48..52).contains(&at)
            || (73..99).contains(&at))
    );
}

#[test]
fn at_speaker_idle_and_disabled_drive_are_exact_silence() {
    for program in [
        vec![0xFA, 0xEB, 0xFE],
        vec![0xFA, 0xB0, 1, 0xE6, 0x61, 0xEB, 0xFE],
        vec![
            0xFA, 0xB0, 0xB0, 0xE6, 0x43, 0xB0, 4, 0xE6, 0x42, 0xB0, 0, 0xE6, 0x42, 0xB0, 2, 0xE6,
            0x61, 0xEB, 0xFE,
        ],
    ] {
        let (_, _, frames) = capture(&program, 125, &[]);
        assert_eq!(frames, vec![[0; 2]; 5]);
    }
}
