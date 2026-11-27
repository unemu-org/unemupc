// SPDX-License-Identifier: AGPL-3.0-or-later
//! Yamaha YMF262 datasheet, Register Map: synthetic FM tone through SB16/mixer.
//! See `backlog/docs/design/audio.md` §OPL3. PCM pins x86jit revision
//! 241c4cba3b3f6828139551314b37cd510889b2b1.
use assembly::{
    AssemblyOptions, AudioSinkKind, DeviceConfig, DeviceId, DeviceKind, DmaSpec, ObservationPolicy,
    RunPolicy, Sb16Cfg,
};
use machine_file::assemble_with_observation_and_options;

#[path = "support/host_profile.rs"]
mod host_profile;
#[path = "support/serving.rs"]
mod serving;

fn out(program: &mut Vec<u8>, port: u16, byte: u8) {
    program.extend_from_slice(&[0xba, port as u8, (port >> 8) as u8, 0xb0, byte, 0xee]);
}
fn reg(program: &mut Vec<u8>, index: u8, value: u8) {
    out(program, 0x388, index);
    out(program, 0x389, value);
}
fn rom() -> Vec<u8> {
    let mut program = vec![0xfa];
    for (index, value) in [
        (0x20, 0x21),
        (0x23, 0x21),
        (0x40, 0x10),
        (0x43, 0),
        (0x60, 0x94),
        (0x63, 0x94),
        (0x80, 0x45),
        (0x83, 0x45),
        (0xc0, 0x30),
        (0xa0, 0x98),
        (0xb0, 0x31),
    ] {
        reg(&mut program, index, value);
    }
    // LOOP retires one instruction per iteration, independent of host speed.
    program.extend_from_slice(&[0xb9, 0xa0, 0x0f, 0xe2, 0xfe]);
    reg(&mut program, 0xb0, 0x11);
    program.extend_from_slice(&[0xeb, 0xfe]);
    let mut rom = vec![0; 32768];
    rom[..program.len()].copy_from_slice(&program);
    rom[32752..32757].copy_from_slice(&[0xea, 0, 0, 0, 0xf8]);
    rom
}
fn assembled() -> assembly::AssembledMachine {
    let mut config = assembly::bios(rom(), Some(vec![0; 1_474_560]));
    config.clock.retired_per_pit_clock = 1;
    config.clock.retired_per_pit_clock_denominator = 1;
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
    let mut policy = ObservationPolicy::default();
    policy.output.audio_sink = AudioSinkKind::Capture;
    policy.output.audio_capture_frame_limit = 2048;
    assemble_with_observation_and_options(
        &config,
        &policy,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap()
}
fn capture(caps: &[u64]) -> (u64, u64, Vec<[i16; 2]>) {
    let mut rig = assembled();
    for &cap in caps.iter().chain(std::iter::once(&24000)) {
        rig.run_session(&RunPolicy::natural(Some(cap), None))
            .unwrap();
    }
    let audio = rig.observation.captured_audio().unwrap();
    (audio.frame_count(), audio.hash(), audio.frames().to_vec())
}
#[test]
fn opl3_two_operator_key_on_release_pcm_and_split_runs() {
    let whole = capture(&[]);
    assert_eq!(
        whole,
        capture(&[1, 23, 24, 25, 149, 997, 4000, 4100, 8191, 12001, 23999])
    );
    assert_eq!(whole, capture(&[7, 800, 4096, 8000, 10000, 18000]));
    assert_eq!(whole.0, 965);
    assert!(whole.2.iter().any(|&p| p != [0; 2]));
    assert_eq!(whole.1, 0x5f94c755a036c6f7);
}

/// The FM golden with the machine's served ranges served in process and with
/// every access returning: the YMF262 address and data ports (Yamaha YMF262
/// datasheet, Register Map) are served, and both ways leave the same
/// machine, traces and PCM.
#[test]
fn opl3_serving_in_process_matches_every_access_returning() {
    let compared = serving::compare(
        &serving::Lockstep {
            label: "opl3",
            step: 997,
            cap: 24000,
            idle_limit: None,
            outputs: Vec::new(),
            displays: Vec::new(),
            profile: Some(|rig, label| {
                Box::new(host_profile::WorkloadProfile::from_env(rig, || label))
            }),
            inject: None,
            audio: true,
        },
        assembled,
        |_| false,
    );
    assert_eq!(compared.outcome, machine::RunOutcome::RetiredCapReached);
    assert!(
        compared.served_accesses.ports >= 24,
        "{:?}",
        compared.served_accesses
    );
    let audio = compared.served.observation.captured_audio().unwrap();
    assert_eq!(
        (audio.frame_count(), audio.hash()),
        (965, 0x5f94c755a036c6f7)
    );
}
