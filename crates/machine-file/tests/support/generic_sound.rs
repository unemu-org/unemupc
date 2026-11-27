// SPDX-License-Identifier: AGPL-3.0-or-later
//! Creative Sound Blaster Series Hardware Programming Guide, §2 DSP ports,
//! §3 DMA output, §4 CT1745, §6 41h/Bxh/Cxh; Yamaha YMF262 datasheet,
//! Register Map and Timers. SeaBIOS boots the AT goldens' PCM and FM sequences
//! on each generic ISA board. See machine-files.md §Generic 386 ISA board.
//! IBM Personal Computer AT Technical Reference, System Board, Direct Memory
//! Access and I/O Address Map; Intel 8259A datasheet, Initialization and EOI.

use super::{budget, freedos_config, games_config};
use assembly::{AudioSinkKind, CpuChip, DeviceId, ObservationPolicy};
use cpu::Cpu;
use device::{AccessDir, Device, TraceEvent, TracePolicy};

fn out(program: &mut Vec<u8>, port: u16, byte: u8) {
    program.extend_from_slice(&[0xBA, port as u8, (port >> 8) as u8, 0xB0, byte, 0xEE]);
}

fn read_to(program: &mut Vec<u8>, port: u16, address: u16) {
    program.extend_from_slice(&[
        0xBA,
        port as u8,
        (port >> 8) as u8,
        0xEC,
        0xA2,
        address as u8,
        (address >> 8) as u8,
    ]);
}

fn fm(program: &mut Vec<u8>, index: u8, value: u8) {
    out(program, 0x388, index);
    out(program, 0x389, value);
}

fn wait(program: &mut Vec<u8>) {
    // At either nominal CPU rate this exceeds the four-sample DMA block and
    // the YMF262's 80 microsecond timer-1 period, using only retired work.
    program.extend_from_slice(&[0xB9, 0, 0x80, 0xE2, 0xFE]);
}

fn payload(word: bool, tone_steps: u16) -> Vec<u8> {
    let mut program = vec![0xFA, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, 0xBC, 0, 0x70];
    for offset in 0..16 {
        program.extend_from_slice(&[0xC6, 6, offset, 5, 0]);
    }
    read_to(&mut program, 0x21, 0x50A);
    read_to(&mut program, 0xA1, 0x50B);
    // IRQ5 belongs to the master PIC (vector 0Dh); the slave remains in the
    // SeaBIOS cascade configuration. A DOS sound driver only unmasks IRQ5.
    program.extend_from_slice(&[0xC7, 6, 0x34, 0, 0, 0x8E, 0xC7, 6, 0x36, 0, 0, 0]);
    program.extend_from_slice(&[0xE4, 0x21, 0x24, 0xDF, 0xE6, 0x21]);
    out(&mut program, 0x226, 1);
    wait(&mut program);
    out(&mut program, 0x226, 0);
    wait(&mut program);
    read_to(&mut program, 0x22E, 0x50C);
    read_to(&mut program, 0x22A, 0x500);
    out(&mut program, 0x22C, 0xE1);
    read_to(&mut program, 0x22A, 0x501);
    read_to(&mut program, 0x22A, 0x502);

    // The AT golden samples are placed at 21000h. A wrong page latch can
    // therefore produce neither the byte ramp nor the signed stereo pairs.
    program.extend_from_slice(&[0xB8, 0, 0x20, 0x8E, 0xC0]);
    let samples: &[u8] = if word {
        &[0x34, 0x12, 0xCC, 0xED, 0, 0x80, 0xFF, 0x7F]
    } else {
        &[0, 0x40, 0x80, 0xFF]
    };
    for (offset, &byte) in samples.iter().enumerate() {
        program.extend_from_slice(&[0x26, 0xC6, 6, offset as u8, 0x10, byte]);
    }
    let registers: &[(u16, u8)] = if word {
        &[
            (0xD4, 5),
            (0xD8, 0),
            (0xC4, 0),
            (0xC4, 8),
            (0xC6, 3),
            (0xC6, 0),
            (0x8B, 3),
            (0xD6, 0x49),
            (0xD4, 1),
        ]
    } else {
        &[
            (0x0A, 5),
            (0x0C, 0),
            (2, 0),
            (2, 0x10),
            (3, 3),
            (3, 0),
            (0x83, 2),
            (0x0B, 0x49),
            (0x0A, 1),
        ]
    };
    for &(port, value) in registers {
        out(&mut program, port, value);
    }
    if word {
        for (index, value) in [(0x30, 0xF8), (0x31, 0xF8), (0x32, 0xF0), (0x33, 0xE8)] {
            out(&mut program, 0x224, index);
            out(&mut program, 0x225, value);
        }
    }
    for byte in [
        0x41,
        0x1F,
        0x40,
        if word { 0xB0 } else { 0xC0 },
        if word { 0x30 } else { 0 },
        3,
        0,
    ] {
        out(&mut program, 0x22C, byte);
    }
    program.push(0xFB);
    wait(&mut program);
    program.push(0xFA);
    out(&mut program, 0x224, 0x82);
    read_to(&mut program, 0x225, 0x504);
    out(&mut program, 0x20, 0x0B);
    read_to(&mut program, 0x20, 0x505);
    out(&mut program, 0xA0, 0x0B);
    read_to(&mut program, 0xA0, 0x506);

    fm(&mut program, 4, 0x60);
    fm(&mut program, 4, 0x80);
    read_to(&mut program, 0x388, 0x507);
    read_to(&mut program, 0x38A, 0x508);
    fm(&mut program, 2, 0xFF);
    fm(&mut program, 4, 1);
    wait(&mut program);
    read_to(&mut program, 0x388, 0x509);
    fm(&mut program, 4, 0x80);
    fm(&mut program, 4, 0x60);
    for (index, value) in [
        (0x20, 0x21),
        (0x23, 0x21),
        (0x40, 0x10),
        (0x43, 0),
        (0x60, 0x94),
        (0x63, 0x94),
        (0x80, 0x45),
        (0x83, 0x45),
        (0xC0, 0x30),
        (0xA0, 0x98),
        (0xB0, 0x31),
    ] {
        fm(&mut program, index, value);
    }
    // The AT tone holds for 4000 PIT clocks. At nominal MHz the same 4000
    // instructions end before the envelope becomes audible on these boards.
    program.extend_from_slice(&[0xB9, tone_steps as u8, (tone_steps >> 8) as u8, 0xE2, 0xFE]);
    fm(&mut program, 0xB0, 0x11);
    wait(&mut program);
    program.extend_from_slice(&[0xC7, 6, 0x0E, 5, 0xFE, 0xCA, 0xF4]);
    assert!(program.len() < 0x1000);
    program.resize(0x1000, 0);
    let mut handler = vec![0x50, 0x52, 0xFE, 6, 3, 5];
    handler.extend_from_slice(&[0xBA, if word { 0x2F } else { 0x2E }, 2, 0xEC]);
    out(&mut handler, 0x20, 0x20);
    handler.extend_from_slice(&[0x5A, 0x58, 0xCF]);
    program.extend(handler);
    program
}

fn floppy(chip: CpuChip, word: bool) -> Vec<u8> {
    let payload = payload(word, budget(chip, 4000).try_into().unwrap());
    let mut disk = vec![0; 1_474_560];
    // Load the following sectors at 0000:7E00 through SeaBIOS INT 13h.
    let loader = [
        0xFA,
        0x31,
        0xC0,
        0x8E,
        0xD0,
        0xBC,
        0,
        0x7C,
        0x8E,
        0xC0,
        0xFB,
        0xBB,
        0,
        0x7E,
        0xB8,
        payload.len().div_ceil(512) as u8,
        2,
        0xB9,
        2,
        0,
        0x30,
        0xF6,
        0xCD,
        0x13,
        0x72,
        5,
        0xEA,
        0,
        0x7E,
        0,
        0,
        0xFA,
        0xF4,
    ];
    disk[..loader.len()].copy_from_slice(&loader);
    disk[510..512].copy_from_slice(&[0x55, 0xAA]);
    disk[512..512 + payload.len()].copy_from_slice(&payload);
    disk
}

fn verify_pcm(trace: &[TraceEvent], frames: &[[i16; 2]], word: bool) -> u64 {
    let start = trace
        .iter()
        .filter_map(|event| match event {
            TraceEvent::Port {
                at,
                port: 0x22C,
                dir: AccessDir::Write,
                ..
            } => Some(at.0),
            _ => None,
        })
        .next_back()
        .unwrap();
    // Independent integration of the AT goldens' native levels on the PIT
    // grid, including the signed stereo mixer gains. Neither the DSP nor
    // audio RateSource supplies the expected samples or boundaries.
    let edge = |index: u64| start + (index * 13125).div_ceil(88);
    let count = if word { 2 } else { 4 };
    let mut checked = 0;
    for (index, &actual) in frames.iter().enumerate() {
        let begin = (index as u64 * 4375).div_ceil(176);
        let end = ((index as u64 + 1) * 4375).div_ceil(176);
        if begin < start || end > edge(count + 1) {
            continue;
        }
        let expected = std::array::from_fn(|channel| {
            let area: i64 = (begin..end)
                .map(|at| {
                    let sample = (1..=count).find(|&n| (edge(n)..edge(n + 1)).contains(&at));
                    if word {
                        let level = match sample {
                            Some(1) => [4660i64, -4660][channel],
                            Some(2) => [-32768, 32767][channel],
                            _ => 0,
                        };
                        level * [13326616, 10585708][channel] / (1 << 24) / 2
                    } else {
                        match sample {
                            Some(1) => -16384,
                            Some(2) => -8192,
                            Some(3) | None => 0,
                            Some(4) => 16256,
                            _ => unreachable!(),
                        }
                    }
                })
                .sum();
            (area / (end - begin) as i64) as i16
        });
        assert_eq!(actual, expected, "PCM frame {index}, word={word}");
        checked += 1;
    }
    assert!(checked >= 10, "DMA PCM was not captured");
    start
}

fn verify_fm(trace: &[TraceEvent], frames: &[[i16; 2]]) {
    let written_at = |value| {
        trace
            .iter()
            .find_map(|event| match event {
                TraceEvent::Port {
                    at,
                    port: 0x389,
                    dir: AccessDir::Write,
                    value: written,
                    ..
                } if *written == value => Some(at.0),
                _ => None,
            })
            .unwrap()
    };
    let on = written_at(0x31);
    let off = written_at(0x11);
    let audible = |start, end| {
        frames.iter().enumerate().any(|(index, frame)| {
            let begin = (index as u64 * 4375).div_ceil(176);
            let finish = ((index as u64 + 1) * 4375).div_ceil(176);
            begin >= start && finish <= end && *frame != [0; 2]
        })
    };
    assert!(audible(on, off), "FM note produced no PCM");
    assert!(audible(off, u64::MAX), "FM release produced no PCM");
}

#[derive(Debug, PartialEq, Eq)]
struct Capture {
    golden: (u64, u64, u64),
    irq_delay: u64,
    splits: Vec<u64>,
}

/// The games board with the payload floppy in drive A.
fn sound_config(chip: CpuChip, word: bool) -> Option<assembly::MachineConfig> {
    let mut config = games_config(chip)?;
    let mut inherited = config.clone();
    inherited.devices.retain(|d| d.id != DeviceId::Sb16);
    assert_eq!(inherited, freedos_config(chip, true).unwrap());
    let sb16 = config
        .devices
        .iter()
        .find(|d| d.id == DeviceId::Sb16)
        .unwrap();
    assert_eq!(sb16.irq, Some(5));
    assert_eq!(sb16.dma.unwrap().channel, 1);
    assert!(
        matches!(sb16.kind, assembly::DeviceKind::Sb16(cfg) if cfg.base == 0x220 && cfg.dma16 == 5)
    );
    let drive = config
        .drives
        .iter_mut()
        .find(|d| d.controller == DeviceId::Fdc && d.unit == 0)
        .unwrap();
    drive.image = assembly::ImageSource::Inline(floppy(chip, word));
    drive.expected_sha256 = None;
    drive.write = assembly::WriteMode::ReadOnly;
    Some(config)
}

fn sound_policy() -> ObservationPolicy {
    let mut policy = ObservationPolicy {
        trace: TracePolicy::Ring { capacity: 4096 },
        ..Default::default()
    };
    policy.output.audio_sink = AudioSinkKind::Capture;
    policy.output.audio_capture_frame_limit = 500_000;
    policy
}

/// The golden of [`sb16_opl3`]: retired count, PCM frames and PCM hash.
fn sound_golden(chip: CpuChip, word: bool) -> (u64, u64, u64) {
    match (chip, word) {
        (CpuChip::I80386, false) => (8_432_283, 49_550, 9_224_039_477_004_121_228),
        (CpuChip::I80386, true) => (8_432_311, 49_551, 4_505_727_766_093_319_895),
        (_, false) => (11_033_817, 32_343, 7_145_792_389_143_746_930),
        (_, true) => (11_033_845, 32_343, 9_206_121_455_469_802_097),
    }
}

fn capture(chip: CpuChip, word: bool, splits: &[u64]) -> Option<Capture> {
    let config = sound_config(chip, word)?;
    let policy = sound_policy();
    let clock = config.clock.clone();
    let mut rig = machine_file::assemble_with_observation(&config, &policy).unwrap();
    rig.machine.enable_delivery_log().unwrap();
    let _profile = super::host_profile::WorkloadProfile::from_env(&mut rig, || {
        format!("{} SB16 word={word} splits={}", chip.name(), splits.len())
    });
    let mut audio_nanos = 0;
    for &cap in splits {
        rig.machine.run_bounded(Some(cap)).unwrap();
        let started = _profile.as_ref().map(|_| std::time::Instant::now());
        rig.capture_audio().unwrap();
        if let Some(started) = started {
            audio_nanos += started.elapsed().as_nanos();
        }
    }
    assert_eq!(
        rig.machine
            .run_bounded(Some(budget(chip, 10_000_000)))
            .unwrap(),
        machine::RunOutcome::Halted
    );
    let started = _profile.as_ref().map(|_| std::time::Instant::now());
    rig.capture_audio().unwrap();
    if let Some(started) = started {
        audio_nanos += started.elapsed().as_nanos();
        eprintln!(
            "HOST AUDIO {} word={word} splits={}: nanos={audio_nanos}",
            chip.name(),
            splits.len()
        );
    }
    let mut result = [0; 16];
    rig.machine.cpu().read_mem(0x500, &mut result).unwrap();
    eprintln!("{} word={word} results={result:02X?}", chip.name());
    assert_eq!(&result[..3], &[0xAA, 4, 5]);
    assert_eq!(result[3], 1, "IRQ5 handler must run exactly once");
    assert_eq!(
        &result[4..7],
        &[0; 3],
        "DSP IRQ and both PIC in-service registers must be clear"
    );
    assert_eq!(&result[7..10], &[0, 0xFF, 0xC0]);
    assert_eq!(
        &result[10..12],
        &[0xB8, if chip == CpuChip::I80386 { 0xAE } else { 0x8E }],
        "SeaBIOS PIC masks"
    );
    assert_eq!(result[12] & 0x80, 0x80, "reset reply must be ready");
    assert_eq!(&result[14..], &[0xFE, 0xCA]);
    assert!(rig.machine.recent_deliveries().unwrap().entries.iter().any(|entry| matches!(entry, machine::DeliveryEntry::Deliveries { last, .. } if last.vector == 0x0D)));
    let dma_id = if word {
        DeviceId::DmaWord
    } else {
        DeviceId::Dma
    };
    let snapshot = rig
        .machine
        .bus()
        .device(rig.device_index[&dma_id])
        .unwrap()
        .save();
    let mut dma = if word {
        devices::Dma8237::new_at_word()
    } else {
        devices::Dma8237::new()
    };
    dma.restore(&snapshot).unwrap();
    assert_eq!(dma.current_count(1), 0xFFFF);
    assert_eq!(
        dma.physical_address(1),
        if word { 0x21008 } else { 0x21004 }
    );
    let trace = rig
        .machine
        .bus_mut()
        .device_mut(rig.device_index[&DeviceId::Sb16])
        .unwrap()
        .take_trace();
    assert!(trace.iter().any(|event| matches!(event, TraceEvent::Port { port: 0x22C, dir: AccessDir::Write, value, .. } if *value == if word { 0xB0 } else { 0xC0 })));
    let audio = rig.observation.captured_audio().unwrap();
    let start = verify_pcm(&trace, audio.frames(), word);
    verify_fm(&trace, audio.frames());
    let irq_at = trace
        .iter()
        .find_map(|event| match event {
            TraceEvent::Port {
                at,
                port,
                dir: AccessDir::Read,
                ..
            } if *port == if word { 0x22F } else { 0x22E } && at.0 > start => Some(at.0),
            _ => None,
        })
        .unwrap();
    let irq_delay = irq_at - start;
    let completion = ((if word { 2 } else { 4 }) * 13125u64).div_ceil(88);
    assert!((completion..completion + 16).contains(&irq_delay));
    assert!(!rig
        .machine
        .bus()
        .device(rig.device_index[&DeviceId::Sb16])
        .unwrap()
        .irq_pending());
    let mut splits = vec![1, 17, 999, 100_000, 1_000_000];
    for at in (0..=5)
        .map(|n| start + (n * 13125u64).div_ceil(88))
        .chain(trace.iter().filter_map(|event| match event {
            TraceEvent::Port {
                at,
                port: 0x388 | 0x389,
                dir: AccessDir::Write,
                ..
            } => Some(at.0),
            _ => None,
        }))
    {
        let cap =
            (at * clock.retired_per_pit_clock).div_ceil(clock.retired_per_pit_clock_denominator);
        splits.extend([cap - 1, cap, cap + 1]);
    }
    splits.sort_unstable();
    splits.dedup();
    let golden = (
        rig.machine.cpu().retired(),
        audio.frame_count(),
        audio.hash(),
    );
    eprintln!(
        "{} word={word} golden={golden:?}; IRQ delay={irq_delay} PIT clocks",
        chip.name()
    );
    Some(Capture {
        golden,
        irq_delay,
        splits,
    })
}

pub fn sb16_opl3(chip: CpuChip) {
    assert!(
        include_str!("../../../../Cargo.lock").contains("fb7bc5b7fea0bef47ddfc31f557dd6b14db34668")
    );
    for word in [false, true] {
        let Some(whole) = capture(chip, word, &[]) else {
            return;
        };
        assert_eq!(whole.irq_delay, if word { 299 } else { 597 });
        assert_eq!(whole.golden, sound_golden(chip, word));
        assert_eq!(Some(&whole), capture(chip, word, &whole.splits).as_ref());
    }
}

/// [`sb16_opl3`]'s boots with the machine's served ranges served in process
/// and with every access returning: SeaBIOS reads the payload through the
/// µPD765 and the 8237, the payload programs the DSP, the mixer, the 8237
/// and the YMF262 through served ports, and both ways leave the same
/// machine, traces and PCM, which is the golden's.
#[allow(dead_code)]
pub fn sb16_opl3_serving(chip: CpuChip) {
    for word in [false, true] {
        let Some(config) = sound_config(chip, word) else {
            return;
        };
        let compared = super::serving::compare(
            &super::serving::Lockstep {
                label: if word {
                    "seabios sb16 word"
                } else {
                    "seabios sb16 byte"
                },
                step: 100_000,
                cap: budget(chip, 10_000_000),
                idle_limit: None,
                outputs: Vec::new(),
                displays: Vec::new(),
                profile: Some(|rig, label| {
                    Box::new(super::host_profile::WorkloadProfile::from_env(rig, || {
                        label
                    }))
                }),
                inject: None,
                audio: true,
            },
            || {
                let mut rig =
                    machine_file::assemble_with_observation(&config, &sound_policy()).unwrap();
                rig.machine.enable_delivery_log().unwrap();
                rig
            },
            |_| false,
        );
        assert_eq!(compared.outcome, machine::RunOutcome::Halted);
        assert!(
            compared.served_accesses.ports > 1000,
            "{:?}",
            compared.served_accesses
        );
        let audio = compared.served.observation.captured_audio().unwrap();
        assert_eq!(
            (
                compared.served.machine.cpu().retired(),
                audio.frame_count(),
                audio.hash()
            ),
            sound_golden(chip, word)
        );
    }
}
