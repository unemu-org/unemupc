// SPDX-License-Identifier: AGPL-3.0-or-later
//! Third-party 386 validation ROM on the generic ISA boards. Intel 80386
//! Programmer's Reference Manual §10 Initialization and §6 Protection.
//! The 486 entries intentionally stop at paging row 11: Intel486 Programmer's
//! Reference Manual §9.9.14 requires both A bits on an operand protection fault,
//! whereas this unchanged 386 diagnostic requires them clear. That stop is
//! 486-correct; the pinned image has no switch to accept the 486 A-bit rule.
//! See `backlog/docs/design/protected-mode-gaps.md` for measured coverage.

use assembly::{
    AssemblyOptions, BoardModel, CpuChip, DeviceId, FpuChip, MachineConfig, ObservationPolicy,
    ObservationRequest, OutputKind, RamRegion,
};
use cpu::{Cpu, CpuReg};
use device::ByteStreamKind;

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/host_profile.rs"]
mod host_profile;
#[path = "support/serving.rs"]
mod serving;

const CPU_REV: &str = "fb7bc5b7fea0bef47ddfc31f557dd6b14db34668";
const CAP: u64 = 100_000_000;
const PAGING_STOP_POST: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x08, 0x09, 0x20, 0x21, 0x22, 0x0B, 0x0C, 0x0D, 0x0E,
    0x0F, 0x10, 0x11,
];

fn board(rom: Vec<u8>, chip: CpuChip) -> MachineConfig {
    // The board owns a 128 KiB decode; place the 64 KiB ROM in its upper half.
    let mut bios = vec![0xFF; 65536];
    bios.extend_from_slice(&rom);
    let mut config = assembly::bios(bios, None);
    let uart = config
        .devices
        .iter()
        .find(|device| device.id == DeviceId::Uart)
        .unwrap()
        .clone();
    config.images[0].base = 0xE0000;
    config.board = if chip == CpuChip::I80386 {
        BoardModel::Generic386Isa
    } else {
        BoardModel::Generic486Isa
    };
    config.clock = config.board.default_clock();
    config.cpu.chip = chip;
    config.cpu.fpu = if chip == CpuChip::I80386 {
        FpuChip::None
    } else {
        FpuChip::Integrated
    };
    config.devices = config.board.motherboard_devices();
    config.devices.push(uart);
    config.drives.clear();
    config.memory.address_space_bytes = 0x1000000;
    config.memory.mmio.clear();
    config.memory.ram = vec![
        RamRegion::new(0, 0xA0000),
        RamRegion::new(0xC0000, 0x40000),
        RamRegion::new(0x100000, 15 << 20),
    ];
    config
}

fn observation_policy() -> ObservationPolicy {
    ObservationPolicy {
        routes: vec![
            ObservationRequest {
                device: DeviceId::Post,
                output: OutputKind::Bytes(ByteStreamKind::PostCheckpoints),
                byte_limit: 4096,
            },
            ObservationRequest {
                device: DeviceId::DebugConsole,
                output: OutputKind::Bytes(ByteStreamKind::DebugConsole),
                byte_limit: 65536,
            },
        ],
        ..Default::default()
    }
}

fn run(chip: CpuChip) {
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin changed: rerun test386 and review the golden stop"
    );
    let Some(rom) = fixtures::fixture("test386/test386.bin") else {
        return;
    };
    assert_eq!(rom.len(), 65536);
    let hash = fixtures::parse_pin(
        include_str!("../../../vendor/test386/fetch.sh"),
        "check_sha256 \"$OUT\" \"",
    )
    .unwrap();
    assert_eq!(assembly::test_support::fixture_sha256(&rom), hash);
    let policy = observation_policy();
    let mut assembled = assembly::assemble_with_observation_and_options(
        &board(rom, chip),
        &policy,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    assembled.machine.enable_delivery_log().unwrap();
    let _profile = host_profile::WorkloadProfile::from_env(&mut assembled, || {
        format!("{} test386", chip.name())
    });
    let started = std::time::Instant::now();
    // A final halt alone cannot distinguish completion from an unrelated
    // failure; sample paging behavior and the conditional ARPL write as well.
    let mut cap = 0;
    let mut checkpoints = vec![
        (1_413_080, 0xA05C, 0x49F000),
        (
            1_413_090,
            0xA06C,
            if chip == CpuChip::I80386 {
                0x3005
            } else {
                0x3025
            },
        ),
    ];
    if chip == CpuChip::I80386 {
        checkpoints.push((1_429_159, 0xB51C, 0));
        checkpoints.push((1_429_160, 0xB522, 0));
    }
    for (target, ip, eax) in checkpoints {
        while assembled.machine.cpu().retired() < target {
            // REP iterations consume steps without each retiring an instruction.
            cap += target - assembled.machine.cpu().retired();
            assert!(cap <= CAP);
            assert_eq!(
                assembled.machine.run_bounded(Some(cap)).unwrap(),
                machine::RunOutcome::RetiredCapReached
            );
        }
        assert_eq!(assembled.machine.cpu().reg(CpuReg::Rip), ip);
        assert_eq!(assembled.machine.cpu().reg(CpuReg::Rax), eax);
        if target == 1_413_090 {
            for (address, expected) in [
                (0x1004, eax as u32),
                (
                    0x327C,
                    if chip == CpuChip::I80386 {
                        0x9F005
                    } else {
                        0x9F025
                    },
                ),
            ] {
                let mut bytes = [0; 4];
                assembled
                    .machine
                    .cpu()
                    .read_mem(address, &mut bytes)
                    .unwrap();
                assert_eq!(u32::from_le_bytes(bytes), expected);
            }
        }
        if target == 1_429_159 {
            let ds = assembled.machine.cpu().diagnostic_state().segments[3];
            assert_eq!(
                (ds.selector, ds.base, ds.limit, ds.attributes),
                (0x1C, 0x2B000, 0xFFFFF, 0x91)
            );
            let mut bytes = [0; 6];
            assembled
                .machine
                .cpu()
                .read_mem(0xFB51C, &mut bytes)
                .unwrap();
            assert_eq!(bytes, [0x63, 0x1D, 0, 0, 2, 0]);
            let mut selector = [0; 2];
            assembled
                .machine
                .cpu()
                .read_mem(0x4B000, &mut selector)
                .unwrap();
            assert_eq!(u16::from_le_bytes(selector), 0xFFF2);
        }
        if target == 1_429_160 {
            assert_eq!(assembled.machine.cpu().diagnostic_state().flags & 0x40, 0);
            let mut selector = [0; 2];
            assembled
                .machine
                .cpu()
                .read_mem(0x4B000, &mut selector)
                .unwrap();
            assert_eq!(u16::from_le_bytes(selector), 0xFFF2);
            assert_eq!(
                assembled.machine.cpu().diagnostic_state().segments[3].attributes,
                0x91
            );
        }
        eprintln!(
            "{} checkpoint: retired={target}; IP={ip:04x}; EAX={eax:08x}",
            chip.name()
        );
    }
    let outcome = assembled.machine.run_bounded(Some(CAP));
    let wall = started.elapsed();
    let post = assembled
        .observation
        .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
        .unwrap();
    let text = String::from_utf8(
        assembled
            .observation
            .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
            .unwrap(),
    )
    .unwrap();
    let retired = assembled.machine.cpu().retired();

    eprintln!(
        "{}: {outcome:?}; retired={retired}; wall={wall:?}; POST={post:02x?}\n{text}",
        chip.name()
    );
    if let Err(machine::MachineError::Cpu(error)) = &outcome {
        eprintln!("{}", assembled.machine.cpu().stop_report(error).unwrap());
    }
    eprintln!("{}", assembled.machine.cpu().diagnostic_state());
    for reg in [
        CpuReg::Rax,
        CpuReg::Rbx,
        CpuReg::Rcx,
        CpuReg::Rdx,
        CpuReg::Rsi,
        CpuReg::Rdi,
        CpuReg::Rsp,
        CpuReg::Rbp,
        CpuReg::Rip,
    ] {
        eprintln!("{reg:?}={:08x}", assembled.machine.cpu().reg(reg));
    }
    let is_386 = chip == CpuChip::I80386;
    let mut expected_post = PAGING_STOP_POST.to_vec();
    if is_386 {
        expected_post.extend_from_slice(&[
            0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0xE0, 0xEE, 0xFF,
        ]);
    }
    assert_eq!(
        post, expected_post,
        "test386 progress changed; review and advance the golden"
    );
    // The pinned printChar uses an immediate OUT: its configured 402h port
    // truncates to 02h, so the arithmetic transcript cannot reach the console.
    assert_eq!(text, "");
    assert_eq!(retired, if is_386 { 79_664_383 } else { 1_413_097 });
    assert_eq!(outcome.unwrap(), machine::RunOutcome::Halted);
    let state = assembled.machine.cpu().diagnostic_state();
    assert_eq!(state.code_linear, if is_386 { 0xFFE7D } else { 0xFFE8A });
    assert_eq!(state.before_code.value(), Some(0xF4));
    let system = state.system.unwrap();
    assert_eq!(system.cr0, if is_386 { 0x8000_0001 } else { 0xE000_0011 });
    assert!(system.protected());
    assert!(!system.virtual_8086);
    assert_eq!(system.cpl, 0);
    assert_eq!(
        assembled.machine.cpu().reg(CpuReg::Rip),
        if is_386 { 0xFE7D } else { 0xFE8A }
    );
    assert_eq!(
        assembled.machine.cpu().reg(CpuReg::Rsi),
        if is_386 { 0xF9DB } else { 0x9DF7 }
    );
    for (address, expected) in [
        (0x1004, if is_386 { 0x3023u32 } else { 0x3025 }),
        (0x327C, if is_386 { 0x9F063 } else { 0x9F025 }),
    ] {
        let mut bytes = [0; 4];
        assembled
            .machine
            .cpu()
            .read_mem(address, &mut bytes)
            .unwrap();
        assert_eq!(u32::from_le_bytes(bytes), expected);
    }
    if is_386 {
        assert_eq!(post.last(), Some(&0xFF), "completion POST code");
        assert_eq!(assembled.machine.cpu().reg(CpuReg::Rax) & 0xFF, 0xFF);
        return;
    }
    let recent = assembled.machine.recent_deliveries().unwrap();
    let machine::DeliveryEntry::Deliveries { last: fault, .. } = recent.entries.last().unwrap()
    else {
        panic!("expected the final fault record");
    };
    eprintln!("last delivery: {fault}");
    assert_eq!(
        (fault.vector, fault.error_code, fault.cs, fault.ip),
        (14, Some(7), 0xAB, 0x9F88)
    );
    assert_eq!(fault.retired, 1_413_070);
    assert_eq!(fault.regs.unwrap().edx, 7);
    assert!(!fault.virtual_8086_before);
}

#[test]
fn test386_i386_golden() {
    run(CpuChip::I80386);
}
#[test]
fn test386_i486_dx_golden() {
    run(CpuChip::I80486Dx);
}
#[test]
fn test386_i486_cpuid_golden() {
    run(CpuChip::I80486DxCpuid);
}

#[test]
fn test386_board_captures_post_and_ascii_from_reset() {
    let mut rom = vec![0xFF; 65536];
    let program = [
        0xB0, 0x42, 0xE6, 0x80, 0xBA, 0x02, 0x04, 0xB0, b'O', 0xEE, 0xB0, b'K', 0xEE, 0xB0, b'\n',
        0xEE, 0xF4,
    ];
    rom[..program.len()].copy_from_slice(&program);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0, 0, 0, 0xF0]);
    for chip in [CpuChip::I80386, CpuChip::I80486Dx, CpuChip::I80486DxCpuid] {
        let mut assembled = assembly::assemble_with_observation_and_options(
            &board(rom.clone(), chip),
            &observation_policy(),
            AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
                .unwrap(),
            [0x42]
        );
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
                .unwrap(),
            b"OK\n"
        );
    }
}

#[test]
fn test386_legacy_scheduler_rig_captures_configured_post() {
    let mut rom = vec![0xFF; 65536];
    rom[..5].copy_from_slice(&[0xB0, 0x42, 0xE6, 0x80, 0xF4]);
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0, 0, 0, 0xF0]);
    let policy = ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::Post,
            output: OutputKind::Bytes(ByteStreamKind::PostCheckpoints),
            byte_limit: 16,
        }],
        ..Default::default()
    };
    let mut assembled =
        assembly::assemble_with_observation(&assembly::test386(rom), &policy).unwrap();
    assert_eq!(
        assembled.machine.run_bounded(Some(100)).unwrap(),
        machine::RunOutcome::Halted
    );
    assert_eq!(
        assembled
            .observation
            .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
            .unwrap(),
        [0x42]
    );
}

/// The 386 run with the machine's served ranges served in process and with
/// every access returning: the same machine at every poll and at the halt.
/// Its port traffic is mostly the console write that truncates to port 02h,
/// a register of the DMA controller, served in process like its POST codes
/// and keyboard-controller reads, so it pins serving a register device on a
/// long protected-mode run.
#[test]
#[ignore = "slow: test386 on the 386 board twice, in lockstep"]
fn test386_i386_serving_in_process_matches_every_access_returning() {
    let Some(rom) = fixtures::fixture("test386/test386.bin") else {
        return;
    };
    let config = board(rom, CpuChip::I80386);
    let compared = serving::compare(
        &serving::Lockstep {
            label: "test386 i386",
            step: 1_000_000,
            cap: CAP,
            idle_limit: None,
            outputs: vec![
                (DeviceId::Post, ByteStreamKind::PostCheckpoints),
                (DeviceId::DebugConsole, ByteStreamKind::DebugConsole),
            ],
            displays: Vec::new(),
            profile: Some(|rig, label| {
                Box::new(host_profile::WorkloadProfile::from_env(rig, || label))
            }),
            inject: None,
            audio: false,
        },
        || {
            let mut assembled = assembly::assemble_with_observation_and_options(
                &config,
                &observation_policy(),
                AssemblyOptions {
                    allow_unlisted_firmware: true,
                },
            )
            .unwrap();
            assembled.machine.enable_delivery_log().unwrap();
            assembled
        },
        |_| false,
    );
    assert_eq!(compared.outcome, machine::RunOutcome::Halted);
    assert_eq!(compared.served.machine.cpu().retired(), 79_664_383);
    assert_eq!(compared.outputs[0].last(), Some(&0xFF));
    assert!(compared.served_accesses.ports > 0);
}
