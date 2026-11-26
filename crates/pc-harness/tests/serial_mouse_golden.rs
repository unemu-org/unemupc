// SPDX-License-Identifier: AGPL-3.0-or-later
//! Synthetic AT ROM: initialize COM1 to 1200 7N1, then receive M and motion
//! through IRQ4 / vector 0Ch. Microsoft Mouse Programmer's Reference, serial
//! protocol requirements; PC16550D datasheet §8.1–8.8; Intel 8259A,
//! Interrupt Sequence. See observation.md § Host input.
//!
//! CPU pin: x86jit-core 67d6697bf110d36142d78a285637ae404f001a8f (Cargo.lock).
//! The ROM uses no BIOS pointing services. It echoes each received byte through
//! the UART's independent TX sink; IRQ entry requires the real PIC and CPU path.

use assembly::{DeviceId, DeviceKind, ObservationPolicy, ObservationRequest, OutputKind};
use device::{AccessDir, ByteStreamKind, TraceEvent, TracePolicy};

#[path = "support/host_profile.rs"]
mod host_profile;
#[path = "support/serving.rs"]
mod serving;

fn rom() -> Vec<u8> {
    let mut program = vec![
        0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xd0, 0xbc, 0x00, 0x70, 0xc7, 0x06, 0x30, 0x00, 0x00,
        0x01, // IVT[0Ch] = F800:0100
        0xc7, 0x06, 0x32, 0x00, 0x00, 0xf8,
    ];
    for (port, value) in [
        (0x20u8, 0x11),
        (0x21, 8),
        (0x21, 4),
        (0x21, 1),
        (0x21, 0xef),
    ] {
        program.extend([0xb0, value, 0xe6, port]);
    }
    for (port, value) in [
        (0x3fbu16, 0x80),
        (0x3f8, 96),
        (0x3f9, 0),
        (0x3fb, 2),
        (0x3f9, 1),
        (0x3fc, 0x0b),
    ] {
        program.extend([0xba, port as u8, (port >> 8) as u8, 0xb0, value, 0xee]);
    }
    program.extend([0xfb, 0xf4, 0xeb, 0xfd]);
    let handler = [
        0x50, 0x52, 0xba, 0xf8, 0x03, 0xec, 0xee, 0xb0, 0x20, 0xe6, 0x20, 0x5a, 0x58, 0xcf,
    ];
    let mut image = vec![0xf4; 0x8000];
    image[..program.len()].copy_from_slice(&program);
    image[0x100..0x100 + handler.len()].copy_from_slice(&handler);
    image[0x7ff0..0x7ff5].copy_from_slice(&[0xea, 0, 0, 0, 0xf8]);
    image
}

fn assembled() -> assembly::AssembledMachine {
    let mut config = assembly::bios(rom(), Some(vec![0; 1_474_560]));
    config.clock = assembly::ClockConfig::default();
    for device in &mut config.devices {
        if let DeviceKind::Uart(uart) = &mut device.kind {
            uart.mouse = true;
            device.irq = Some(4);
        }
    }
    let policy = ObservationPolicy {
        trace: TracePolicy::Ring { capacity: 128 },
        routes: vec![ObservationRequest {
            device: DeviceId::Uart,
            output: OutputKind::Bytes(ByteStreamKind::SerialTx),
            byte_limit: 16,
        }],
        ..Default::default()
    };
    machine_file::assemble_with_observation_and_options(
        &config,
        &policy,
        assembly::AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap()
}

#[test]
fn serial_mouse_rom_receives_identification_and_motion_via_irq4_at_pinned_times() {
    let mut assembled = assembled();
    assembled.machine.set_idle_limit(Some(100_000));
    let uart = assembled.device_index[&DeviceId::Uart];
    assert_eq!(assembled.mouse_device_index(), Some(uart));
    assert_eq!(
        assembled.machine.run_bounded(Some(30_000)).unwrap(),
        machine::RunOutcome::RetiredCapReached
    );
    assert_eq!(
        assembled
            .observation
            .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
            .unwrap(),
        b"M"
    );
    assert!(assembly::inject_mouse(
        &mut assembled.machine,
        uart,
        5,
        -7,
        3
    ));
    assert_eq!(
        assembled.machine.run_bounded(Some(60_000)).unwrap(),
        machine::RunOutcome::RetiredCapReached
    );
    assert_eq!(
        assembled
            .observation
            .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
            .unwrap(),
        [0x70, 5, 7]
    );
    let trace = assembled
        .machine
        .bus_mut()
        .device_mut(uart)
        .unwrap()
        .take_trace();
    let received: Vec<_> = trace
        .iter()
        .filter_map(|event| match event {
            TraceEvent::Port {
                at,
                port: 0x3f8,
                dir: AccessDir::Read,
                value,
                ..
            } => Some((at.0, *value)),
            _ => None,
        })
        .collect();
    assert_eq!(
        received,
        [(20_919, 0x4d), (38_952, 0x70), (47_901, 5), (56_850, 7)]
    );
}

/// The mouse ROM with the machine's served ranges served in process and with
/// every access returning: the 16550's registers (PC16550D datasheet,
/// §8.1–8.8) are served, the same motion is injected into both ways at the
/// same virtual time, and both leave the same machine, echo and traces.
#[test]
fn serial_mouse_serving_in_process_matches_every_access_returning() {
    let compared = serving::compare(
        &serving::Lockstep {
            label: "serial mouse",
            step: 1000,
            cap: 60_000,
            idle_limit: Some(100_000),
            outputs: vec![(DeviceId::Uart, ByteStreamKind::SerialTx)],
            displays: Vec::new(),
            profile: Some(|rig, label| {
                Box::new(host_profile::WorkloadProfile::from_env(rig, || label))
            }),
            inject: Some(|assembled, poll| {
                if poll == 30 {
                    let uart = assembled.device_index[&DeviceId::Uart];
                    assert!(assembly::inject_mouse(
                        &mut assembled.machine,
                        uart,
                        5,
                        -7,
                        3
                    ));
                }
            }),
            audio: false,
        },
        assembled,
        |_| false,
    );
    assert_eq!(compared.outcome, machine::RunOutcome::RetiredCapReached);
    assert_eq!(compared.outputs[0], [b'M', 0x70, 5, 7]);
    assert!(
        compared.served_accesses.ports > 10,
        "{:?}",
        compared.served_accesses
    );
}
