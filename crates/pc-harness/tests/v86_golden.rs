// SPDX-License-Identifier: AGPL-3.0-or-later
//! Intel 80386 Programmer's Reference Manual, Chapter 15 Virtual 8086 Mode;
//! Intel486 DX Programmer's Reference Manual, Chapter 23 Virtual 8086 Mode.
//! See `backlog/docs/design/protected-mode-gaps.md` §V86 monitor and CPU-state audit.

use assembly::{
    AssemblyOptions, BoardModel, CpuChip, DeviceId, FpuChip, MachineConfig, ObservationPolicy,
    ObservationRequest, OutputKind, RamRegion,
};
use cpu::{Cpu, DeliveryKind};
use device::ByteStreamKind;
use machine::{DeliveryEntry, RunOutcome, SchedulerEngine};
#[path = "support/fixtures.rs"]
mod fixtures;

const CPU_REV: &str = "5bdb7a4d9a9a65e6f545e632788a50cdf68cda5c";
const CAP: u64 = 50_000;
const EXPECTED: &str = "i386-v86-test\n\
v86 real code ok\n\
reflected int ok\n\
v86 resumed ok\n\
iopl gp(0) ok\n\
bitmap allow deny ok\n\
ring0 return ok\n\
failures 0000\n";

fn board(rom: Vec<u8>, chip: CpuChip) -> MachineConfig {
    let mut config = assembly::bios(rom, None);
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

fn run(
    chip: CpuChip,
    engine: SchedulerEngine,
    chunk: u64,
) -> Option<(u64, device::VClock, machine::RecentDeliveries)> {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let rom = fixtures::fixture("i386-v86-test/i386-v86-test.bin")?;
    assert_eq!(rom.len(), 131072);
    let pin = fixtures::parse_pin(
        include_str!("../../../vendor/i386-v86-test/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    )
    .unwrap();
    assert_eq!(assembly::test_support::fixture_sha256(&rom), pin);
    let policy = ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::DebugConsole,
            output: OutputKind::Bytes(ByteStreamKind::DebugConsole),
            byte_limit: 4096,
        }],
        ..Default::default()
    };
    let mut assembled = assembly::assemble_with_observation_and_options(
        &board(rom, chip),
        &policy,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    assembled.machine = assembled.machine.with_scheduler_engine(engine);
    assembled.machine.enable_delivery_log().unwrap();
    let started = std::time::Instant::now();
    let outcome = loop {
        let cap = (assembled.machine.virtual_time() + chunk).min(CAP);
        let outcome = assembled.machine.run_bounded(Some(cap));
        if !matches!(outcome, Ok(RunOutcome::RetiredCapReached)) || cap == CAP {
            break outcome;
        }
    };
    let text = String::from_utf8(
        assembled
            .observation
            .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
            .unwrap(),
    )
    .unwrap();
    let retired = assembled.machine.cpu().retired();
    eprintln!(
        "{} V86 ROM {engine:?} chunk={chunk}: {outcome:?}; retired={retired}; wall={:?}\n{text}",
        chip.name(),
        started.elapsed()
    );
    if !matches!(outcome, Ok(RunOutcome::Halted)) {
        eprintln!("{}", assembled.machine.cpu().diagnostic_state());
    }
    assert_eq!(outcome.unwrap(), RunOutcome::Halted);
    assert_eq!(text, EXPECTED);
    assert_eq!(retired, 867);
    let deliveries = assembled.machine.recent_deliveries().unwrap();
    assert_eq!(deliveries.total, 5);
    assert_eq!(deliveries.dropped, 0);
    let records: Vec<_> = deliveries
        .entries
        .iter()
        .map(|entry| match entry {
            DeliveryEntry::Deliveries { count: 1, last, .. } => last,
            _ => panic!("unexpected folded or dropped delivery"),
        })
        .collect();
    for record in &records {
        assert_eq!(
            (record.virtual_8086_before, record.virtual_8086_after),
            (true, false)
        );
        assert_eq!((record.cpl_before, record.cpl_after), (3, 0));
        assert_eq!(record.regs.unwrap().flags & (1 << 17), 0);
    }
    assert_eq!(
        records
            .iter()
            .map(|record| (record.kind, record.vector, record.error_code))
            .collect::<Vec<_>>(),
        [
            (DeliveryKind::Software, 0x40, None),
            (DeliveryKind::Software, 0x42, None),
            (DeliveryKind::Fault, 13, Some(0)),
            (DeliveryKind::Fault, 13, Some(0)),
            (DeliveryKind::Software, 0x41, None)
        ]
    );
    assert!(
        !assembled
            .machine
            .cpu()
            .diagnostic_state()
            .system
            .unwrap()
            .virtual_8086
    );
    Some((retired, assembled.machine.clock(), deliveries))
}

fn qualify(chip: CpuChip) {
    let whole = run(chip, SchedulerEngine::Deadline, CAP);
    for (engine, chunk) in [
        (SchedulerEngine::Deadline, 17),
        (SchedulerEngine::PerClock, 257),
    ] {
        assert_eq!(whole, run(chip, engine, chunk));
    }
}
#[test]
fn i386_v86_monitor_rom() {
    qualify(CpuChip::I80386);
}
#[test]
fn i486_dx_v86_monitor_rom() {
    qualify(CpuChip::I80486Dx);
}
#[test]
fn i486_cpuid_v86_monitor_rom() {
    qualify(CpuChip::I80486DxCpuid);
}
