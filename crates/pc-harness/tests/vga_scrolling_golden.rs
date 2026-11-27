// SPDX-License-Identifier: AGPL-3.0-or-later
//! Synthetic VGA scrolling ROM: IBM VGA/XGA Technical Reference Manual,
//! Graphics Modes, CRTC Start Address and Attribute Controller PEL Panning.
//! Frame hashes pin x86jit fb7bc5b7fea0bef47ddfc31f557dd6b14db34668.

use assembly::{
    AssemblyOptions, DeviceId, ObservationPolicy, ObservationRequest, OutputKind, RunPolicy,
};

const CPU_REV: &str = "fb7bc5b7fea0bef47ddfc31f557dd6b14db34668";

#[path = "support/serving.rs"]
mod serving;

const FRAME_HASHES: [&str; 7] = [
    "542e315e0ac2a357e8debffd2636765cb37d50542f923826656b76a7e5f22a1b",
    "483976e4a820889295d384081974a322b0fac19957934b1c27c7c960a676b22d",
    "2c3cc36c6ec94b1d8de65d9c7525065d3d0880ee396b60a16106086fe82cc43c",
    "bb2fb40860513c397224bd1ec5350b87282591a024912a9196ed5c9dfc00726c",
    "f94461088325d4a6b1ad3ff39520d48c9e1682ad45499f46b2476b588d391cbd",
    "d383e390797ec0c6af0123c58075b19acdea82bc86a1cd8d4ba343b2d311de7b",
    "542e315e0ac2a357e8debffd2636765cb37d50542f923826656b76a7e5f22a1b",
];

fn out(program: &mut Vec<u8>, port: u16, value: u8) {
    program.extend([0xba, port as u8, (port >> 8) as u8, 0xb0, value, 0xee]);
}
fn reg(program: &mut Vec<u8>, port: u16, index: u8, value: u8) {
    program.extend([
        0xba,
        port as u8,
        (port >> 8) as u8,
        0xb8,
        index,
        value,
        0xef,
    ]);
}
fn ac(program: &mut Vec<u8>, index: u8, value: u8) {
    program.extend([0xba, 0xda, 3, 0xec]);
    out(program, 0x3c0, index);
    out(program, 0x3c0, value);
    program.extend([0xba, 0xda, 3, 0xec]);
    out(program, 0x3c0, 0x20);
}
fn wait_retrace(program: &mut Vec<u8>) {
    program.extend([
        0xba, 0xda, 3, 0xec, 0xa8, 8, 0x75, 0xfb, // leave retrace
        0xec, 0xa8, 8, 0x74, 0xfb, // enter next retrace
        0xec, 0xa8, 8, 0x75, 0xfb, // reach the pixel-panning latch at its end
    ]);
}

fn rom() -> Vec<u8> {
    let mut program = vec![0xfa, 0xfc, 0xb8, 0, 0xa0, 0x8e, 0xd8];
    out(&mut program, 0x3c2, 0x63);
    for (i, value) in [3, 9, 15, 0, 6].into_iter().enumerate() {
        reg(&mut program, 0x3c4, i as u8, value);
    }
    // Mode 0Dh geometry, byte addressing, 40 plane bytes per memory row.
    for (i, value) in [
        0x2d, 0x27, 0x28, 0x90, 0x2b, 0x80, 0xbf, 0x1f, 0, 0xc0, 0x20, 0, 0, 0, 0, 0, 0x9c, 0x0e,
        0x8f, 0x14, 0, 0x96, 0xb9, 0xe3, 0xff,
    ]
    .into_iter()
    .enumerate()
    {
        reg(&mut program, 0x3d4, i as u8, value);
    }
    for (i, value) in [0, 0, 0, 0, 0, 0, 5, 15, 255].into_iter().enumerate() {
        reg(&mut program, 0x3ce, i as u8, value);
    }
    for i in 0..16 {
        ac(&mut program, i, i);
    }
    for (i, value) in [(0x10, 1), (0x12, 15), (0x13, 0)] {
        ac(&mut program, i, value);
    }
    out(&mut program, 0x3c6, 255);
    out(&mut program, 0x3c8, 0);
    for i in 0..16 {
        for value in [i * 4, 63 - i * 4, i * 2] {
            out(&mut program, 0x3c9, value);
        }
    }
    // Different stripes on three pages, written through the planar CPU path.
    for (page, seed) in [(0_u16, 0x55_u8), (0x2000, 0x33), (0x4000, 0x0f)] {
        for plane in 0..4 {
            reg(&mut program, 0x3c4, 2, 1 << plane);
            for (offset, value) in [
                (0, seed.rotate_left(plane)),
                (1, !seed.rotate_left(plane)),
                (40, seed),
            ] {
                let address = page + offset;
                program.extend([0xc6, 6, address as u8, (address >> 8) as u8, value]);
            }
        }
    }
    for (stage, (page, pan)) in [
        (0_u16, 0),
        (0, 1),
        (0, 3),
        (0, 7),
        (0x2000, 0),
        (0x4000, 2),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        // Wait for enabled display so the pending capture precedes both edges.
        program.extend([0xba, 0xda, 3, 0xec, 0xa8, 9, 0x75, 0xfb]);
        reg(&mut program, 0x3d4, 0x0c, (page >> 8) as u8);
        reg(&mut program, 0x3d4, 0x0d, page as u8);
        ac(&mut program, 0x13, pan);
        out(&mut program, 0x80, 0x80 | (stage as u8 + 1));
        // Keep each marker observable by the host's bounded capture cadence.
        program.extend([0xb9, 0, 1, 0xe2, 0xfe]);
        if stage == 4 {
            // No VGA access services the pending latch during this full-frame
            // delay: the device deadline must deliver the page flip itself.
            program.extend([0xb9, 0x20, 0x4e, 0xe2, 0xfe]);
        } else {
            wait_retrace(&mut program);
        }
        out(&mut program, 0x80, stage as u8 + 1);
        program.extend([0xb9, 0, 1, 0xe2, 0xfe]);
    }
    program.extend([0xeb, 0xfe]);
    let mut rom = vec![0; 32768];
    rom[..program.len()].copy_from_slice(&program);
    rom[32752..32757].copy_from_slice(&[0xea, 0, 0, 0, 0xf8]);
    rom
}

/// The scrolling ROM on a register-level VGA board.
fn rig() -> assembly::AssembledMachine {
    let mut config = assembly::bios(rom(), None);
    config.devices.retain(|d| d.id != DeviceId::Mda);
    config.memory.mmio.retain(|w| w.device != DeviceId::Mda);
    let card = config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Vga)
        .unwrap();
    card.kind = assembly::DeviceKind::VgaRegisters(assembly::VgaCfg {});
    card.ports = assembly::VgaCfg::port_windows();
    config.memory.ram = vec![assembly::RamRegion::new(0, 0xa0000)];
    config.memory.mmio.retain(|w| w.device != DeviceId::Vga);
    config
        .memory
        .mmio
        .push(assembly::MmioWindow::new(0xa0000, 0x20000, DeviceId::Vga));
    config.clock.retired_per_pit_clock = 1;
    config.clock.retired_per_pit_clock_denominator = 1;
    let policy = ObservationPolicy {
        routes: vec![
            ObservationRequest {
                device: DeviceId::Vga,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            },
            ObservationRequest {
                device: DeviceId::Post,
                output: OutputKind::Bytes(device::ByteStreamKind::PostCheckpoints),
                byte_limit: 16,
            },
        ],
        ..ObservationPolicy::default()
    };
    machine_file::assemble_with_observation_and_options(
        &config,
        &policy,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap()
}

#[test]
fn synthetic_planar_scrolling_and_page_flips_have_pinned_frame_hashes() {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let mut rig = rig();
    let mut hashes = Vec::new();
    let mut pending_captures = 0;
    for cap in (100..=160_000).step_by(100) {
        rig.run_session(&RunPolicy::natural(Some(cap), None))
            .unwrap();
        let stages = rig
            .observation
            .drain_bytes(DeviceId::Post, device::ByteStreamKind::PostCheckpoints)
            .unwrap();
        if stages.is_empty() {
            continue;
        }
        let pending = stages[0] & 0x80 != 0;
        let stage = hashes.len();
        assert_eq!(stages, [(stage as u8 + 1) | if pending { 0x80 } else { 0 }]);
        if pending {
            assert_eq!(pending_captures, stage);
            pending_captures += 1;
        } else {
            assert_eq!(pending_captures, stage + 1);
        }
        let capture = rig.capture_text(DeviceId::Vga).unwrap();
        let video::Frame::Pixels(frame) = &capture.frame else {
            panic!("planar pixels")
        };
        assert_eq!(
            (frame.width, frame.height, frame.repeat),
            (320, 200, (2, 2))
        );
        let expected = [
            [10, 5, 10, 5, 10, 5, 10, 5],
            [5, 10, 5, 10, 5, 10, 5, 5],
            [5, 10, 5, 10, 5, 5, 10, 5],
            [5, 5, 10, 5, 10, 5, 10, 5],
            [12, 6, 3, 9, 12, 6, 3, 9],
            [12, 14, 15, 7, 3, 1, 15, 7],
            [10, 5, 10, 5, 10, 5, 10, 5],
        ];
        let expected_stage = if pending {
            stage.saturating_sub(1)
        } else {
            stage
        };
        assert_eq!(
            &frame.indices[..8],
            &expected[expected_stage],
            "pending={pending}, stage={stage}"
        );
        let hash = assembly::test_support::graphics_frame_sha256(&capture.frame);
        assert_eq!(
            hash, FRAME_HASHES[expected_stage],
            "pending={pending}, stage={stage}"
        );
        if !pending {
            hashes.push(hash);
        }
        if hashes.len() == 7 {
            break;
        }
    }
    assert_eq!(hashes.len(), 7);
    assert_eq!(pending_captures, 7);
    eprintln!("scrolling frames: {hashes:#?}");
    assert_eq!(hashes, FRAME_HASHES);
}

/// The scrolling run, display-memory, VGA-register and port-80h heavy, with
/// all of them served in process and with every access returning: the same
/// machine, frames and traces at every 100-step poll.
#[test]
fn synthetic_scrolling_serving_in_process_matches_every_access_returning() {
    let compared = serving::compare(
        &serving::Lockstep {
            label: "vga scrolling",
            step: 100,
            cap: 160_000,
            idle_limit: None,
            outputs: vec![(DeviceId::Post, device::ByteStreamKind::PostCheckpoints)],
            displays: vec![DeviceId::Vga],
            profile: None,
            inject: None,
            audio: false,
        },
        rig,
        |_| false,
    );
    assert_eq!(compared.outcome, machine::RunOutcome::RetiredCapReached);
    assert_eq!(compared.polls, 1600);
    assert_eq!(compared.outputs[0].len(), 14);
    // Its VGA register accesses are served too.
    assert!(compared.served_accesses.ports > 10_000 && compared.served_accesses.mmio > 0);
}
