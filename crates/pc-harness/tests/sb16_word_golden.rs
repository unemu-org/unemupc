// SPDX-License-Identifier: AGPL-3.0-or-later
//! Creative Sound Blaster Series Hardware Programming Guide, §2 DSP ports,
//! §3 DMA output, §4 CT1745, §6 41h/Bxh. A synthetic AT ROM plays RAM through the real 8237.
//! PCM is pinned against x86jit revision 8064e28f1a9a2e4af7f1304a61e1eb1c9f9b13af.

use assembly::{
    AssemblyOptions, AudioSinkKind, DeviceConfig, DeviceId, DeviceKind, DmaSpec, ObservationPolicy,
    RunPolicy, Sb16Cfg,
};
use device::{AccessDir, TraceEvent, TracePolicy};
use machine_file::assemble_with_observation_and_options;

fn out(program: &mut Vec<u8>, port: u16, byte: u8) {
    program.extend_from_slice(&[0xBA, port as u8, (port >> 8) as u8, 0xB0, byte, 0xEE]);
}

fn rom() -> Vec<u8> {
    let mut program = vec![0xFA, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, 0xBC, 0, 0x70];
    // IRQ5 vector 0Dh points into our ROM; stack and IVT live in guest RAM.
    program.extend_from_slice(&[0xC7, 6, 0x34, 0, 0, 3, 0xC7, 6, 0x36, 0, 0, 0xF8]);
    for (offset, byte) in [0x34, 0x12, 0xCC, 0xED, 0, 0x80, 0xFF, 0x7F]
        .into_iter()
        .enumerate()
    {
        program.extend_from_slice(&[0xC6, 6, offset as u8, 0x10, byte]);
    }
    for (port, byte) in [
        (0x20, 0x11),
        (0x21, 8),
        (0x21, 4),
        (0x21, 1),
        (0x21, 0xDF),
        (0xD4, 5),
        (0xD8, 0),
        (0xC4, 0),
        (0xC4, 0x08),
        (0xC6, 3),
        (0xC6, 0),
        (0x8B, 0),
        (0xD6, 0x49),
        (0xD4, 1),
        (0x226, 1),
    ] {
        out(&mut program, port, byte);
    }
    program.extend_from_slice(&[0x90; 4]);
    out(&mut program, 0x226, 0);
    program.extend_from_slice(&[0x90; 120]);
    // Master at 0 dB, voice at -2 dB left and -4 dB right.
    for (index, value) in [(0x30, 0xF8), (0x31, 0xF8), (0x32, 0xF0), (0x33, 0xE8)] {
        out(&mut program, 0x224, index);
        out(&mut program, 0x225, value);
    }
    // Explicit 8000 Hz, signed stereo B0h, four words.
    for byte in [0x41, 0x1F, 0x40, 0xB0, 0x30, 3, 0] {
        out(&mut program, 0x22C, byte);
    }
    program.extend_from_slice(&[0xFB, 0xEB, 0xFE]);
    assert!(program.len() < 0x300);
    let mut rom = vec![0; 32768];
    rom[..program.len()].copy_from_slice(&program);
    let mut handler = vec![0x50, 0x52];
    out(&mut handler, 0x80, 0x5B);
    handler.extend_from_slice(&[0xBA, 0x2F, 2, 0xEC]);
    out(&mut handler, 0x20, 0x20);
    handler.extend_from_slice(&[0x5A, 0x58, 0xCF]);
    rom[0x300..0x300 + handler.len()].copy_from_slice(&handler);
    rom[32752..32757].copy_from_slice(&[0xEA, 0, 0, 0, 0xF8]);
    rom
}

fn capture_clock(
    splits: &[u64],
    check_irq: bool,
    pit_per_step: u64,
) -> (u64, u64, Vec<[i16; 2]>, Vec<u64>) {
    let mut config = assembly::bios(rom(), Some(vec![0; 1_474_560]));
    config.clock.retired_per_pit_clock = 1;
    config.clock.retired_per_pit_clock_denominator = pit_per_step;
    let cfg = Sb16Cfg {
        base: 0x220,
        dma16: 5,
    };
    config.devices.push(
        DeviceConfig::new(DeviceId::Sb16, DeviceKind::Sb16(cfg), cfg.port_windows())
            .with_irq(5)
            .with_dma(DmaSpec {
                controller: DeviceId::Dma,
                channel: 1,
            }),
    );
    let mut policy = ObservationPolicy {
        trace: TracePolicy::Ring { capacity: 4096 },
        ..ObservationPolicy::default()
    };
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
    let until = 1000;
    for &cap in splits.iter().chain(std::iter::once(&until)) {
        rig.run_session(&RunPolicy::natural(Some(cap), None))
            .unwrap();
        if check_irq && (cap == 530 || cap == 531) {
            assert_eq!(
                rig.machine
                    .bus()
                    .device(rig.device_index[&DeviceId::Sb16])
                    .unwrap()
                    .irq_pending(),
                cap == 531
            );
        }
    }
    let post = rig.device_index[&DeviceId::Post];
    let irqs = rig
        .machine
        .bus_mut()
        .device_mut(post)
        .unwrap()
        .take_trace()
        .into_iter()
        .filter_map(|event| match event {
            TraceEvent::Port {
                at,
                dir: AccessDir::Write,
                value: 0x5B,
                ..
            } => Some(at.0),
            _ => None,
        })
        .collect();
    let audio = rig.observation.captured_audio().unwrap();
    (
        audio.frame_count(),
        audio.hash(),
        audio.frames().to_vec(),
        irqs,
    )
}

// This reference integrates the native levels clock by clock; it shares no
// DSP, DMA, mixer-table or RateSource implementation with the emulator.
fn reference() -> Vec<[i16; 2]> {
    (0..40u64)
        .map(|index| {
            let begin = (index * 4375).div_ceil(176);
            let end = ((index + 1) * 4375).div_ceil(176);
            std::array::from_fn(|channel| {
                let area: i64 = (begin..end)
                    .map(|at| {
                        let level = match at {
                            382..531 => [4660i64, -4660][channel],
                            531..680 => [-32768, 32767][channel],
                            _ => 0,
                        };
                        // round(2^24 * 10^(-2/20)) and round(2^24 * 10^(-4/20)),
                        // then half of the card's voice in the host mix.
                        level * [13326616, 10585708][channel] / (1 << 24) / 2
                    })
                    .sum();
                (area / (end - begin) as i64) as i16
            })
        })
        .collect()
}

#[test]
fn sb16_word_stereo_dma_has_frozen_pcm_irq_time_and_split_invariance() {
    let whole = capture_clock(&[], false, 1);
    assert_eq!(
        whole,
        capture_clock(&[150, 381, 382, 530, 531, 679, 680, 999], true, 1)
    );
    assert_eq!(whole.0, 40);
    assert_eq!(whole.2, reference());
    assert_eq!(whole.3, [535]);
    assert_eq!(whole.1, 0xa6c019b8a1e874ff);
}
