// SPDX-License-Identifier: AGPL-3.0-or-later
//! Third-party 386 validation ROM on the generic ISA boards. Intel 80386
//! Programmer's Reference Manual §10 Initialization and §6 Protection.
//! See `backlog/docs/design/protected-mode-gaps.md` for measured coverage.

use assembly::{
    AssemblyOptions, BoardModel, CpuChip, DeviceId, FpuChip, MachineConfig, ObservationPolicy,
    ObservationRequest, OutputKind, RamRegion,
};
use cpu::{Cpu, CpuReg};
use device::ByteStreamKind;

#[path = "support/fixtures.rs"]
mod fixtures;

const CPU_REV: &str = "d8bf798ed05520b6b06bbb787a71f3906a0c65d9";
const CAP: u64 = 1_000_000;
const GOLDEN_POST: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x08, 0x09, 0x20, 0x21,
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
    let started = std::time::Instant::now();
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
    assert_eq!(
        post, GOLDEN_POST,
        "test386 progress changed; review and advance the golden"
    );
    assert_eq!(text, "", "printing follows the virtual-8086 tests");
    assert_eq!(
        retired, 796_780,
        "retired count changed: review newly reached test386 sections and advance the golden"
    );
    assert!(
        matches!(&outcome,
        Err(machine::MachineError::Cpu(cpu::CpuError::UnsupportedSystem {
            operation, cs: 0xD0, ip: 0x2CA0
        })) if operation == "InvalidLegacyContext(Virtual8086Unsupported)"),
        "expected the virtual-8086 boundary; review further progress: {outcome:?}"
    );
    let state = assembled.machine.cpu().diagnostic_state();
    assert_eq!(state.code_linear, 0xF2CA0);
    assert_eq!(
        state.code[..1]
            .iter()
            .map(|byte| byte.value().unwrap())
            .collect::<Vec<_>>(),
        [0xCF]
    );
    let system = state.system.unwrap();
    assert!(system.protected());
    assert_eq!(system.cpl, 0);
    assert_eq!(assembled.machine.cpu().reg(CpuReg::Rax), 0x118);
    let mut frame = [0; 36];
    assembled
        .machine
        .cpu()
        .read_mem(0x1FFD7, &mut frame)
        .unwrap();
    let words: Vec<_> = frame
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| u32::from_le_bytes(*bytes))
        .collect();
    assert_eq!(
        words,
        [0x5630, 0xF000, 0x20246, 0x7FFF, 0x1000, 0x3040, 0x2010, 0x5060, 0x7080]
    );
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
