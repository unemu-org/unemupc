// SPDX-License-Identifier: AGPL-3.0-or-later
//! BIOS-independent 80386 ROM on the generic ISA board. Intel 80386
//! Programmer's Reference Manual §6 Protection, §7 Multitasking, §9 Interrupts
//! and Exceptions, §10 Initialization and §14.5 Switching to Real-Address Mode;
//! Intel 8259A datasheet, ICWs and EOI; Intel 8254 datasheet, Mode 2.
//! See `backlog/docs/design/i386-pm-test.md` for the transcript contract.

use assembly::{
    AssemblyOptions, BoardModel, CpuChip, DeviceId, FpuChip, MachineConfig, ObservationPolicy,
    ObservationRequest, OutputKind, RamRegion,
};
use cpu::{Cpu, DeliveryKind};
use device::ByteStreamKind;
use machine::{DeliveryEntry, RecentDeliveries, RunOutcome, SchedulerEngine};

#[path = "support/fixtures.rs"]
mod fixtures;

const CPU_REV: &str = "c09bf414647502f9bd30d7f912e8deae181040a7";
const IMAGE: &str = "i386-pm-test/i386-pm-test.bin";
const CAP: u64 = 500_000;
const GOLDEN_RETIRED: u64 = 31_781;
const EXPECTED: &str = "i386-pm-test\n\
reset dh=03 ok\n\
flags 12-15 0000/7000 ok\n\
a20 off alias ok\n\
a20 port92 on ok\n\
protected32 code stack ok\n\
extended memory ok\n\
limit gp(0) resume ok\n\
interrupt trap gates ok\n\
timer ring0 count=3 ok\n\
ring3 int tss stack ok\n\
ring3 timer tss stack ok\n\
real mode return ok\n\
failures 0000\n";

fn board(rom: Vec<u8>) -> MachineConfig {
    let mut config = assembly::bios(rom, None);
    let uart = config
        .devices
        .iter()
        .find(|device| device.id == DeviceId::Uart)
        .unwrap()
        .clone();
    config.images[0].base = 0xE0000;
    config.board = BoardModel::Generic386Isa;
    config.clock = config.board.default_clock();
    config.cpu.chip = CpuChip::I80386;
    config.cpu.fpu = FpuChip::None;
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

#[derive(Debug, PartialEq, Eq)]
struct Run {
    text: String,
    retired: u64,
    clock: device::VClock,
    deliveries: RecentDeliveries,
}

fn run(rom: Vec<u8>, engine: SchedulerEngine, chunk: u64) -> Run {
    let policy = ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::DebugConsole,
            output: OutputKind::Bytes(ByteStreamKind::DebugConsole),
            byte_limit: 4096,
        }],
        ..ObservationPolicy::default()
    };
    let mut assembled = assembly::assemble_with_observation_and_options(
        &board(rom),
        &policy,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    assembled.machine = assembled.machine.with_scheduler_engine(engine);
    assembled.machine.enable_delivery_log().unwrap();
    let outcome = loop {
        let cap = (assembled.machine.virtual_time() + chunk).min(CAP);
        let outcome = assembled.machine.run_bounded(Some(cap));
        if !matches!(outcome, Ok(RunOutcome::RetiredCapReached)) || cap == CAP {
            break outcome;
        }
    };
    let bytes = assembled
        .observation
        .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let retired = assembled.machine.cpu().retired();
    eprintln!("i386-pm-test: stop={outcome:?} retired={retired}\n{text}");
    assert_eq!(outcome.unwrap(), RunOutcome::Halted);
    assert_eq!(text, EXPECTED);
    assert!(retired < CAP);
    assert_eq!(retired, GOLDEN_RETIRED);
    let deliveries = assembled.machine.recent_deliveries().unwrap();
    assert_eq!(deliveries.dropped, 0);
    assert_eq!(deliveries.total, 9, "one fault, four INTs and four IRQs");
    let records: Vec<_> = deliveries
        .entries
        .iter()
        .flat_map(|entry| match entry {
            DeliveryEntry::Deliveries { count, last, .. } => {
                std::iter::repeat_n(last, *count as usize)
            }
            DeliveryEntry::Dropped(_) => panic!("lost delivery evidence"),
        })
        .collect();
    let irqs: Vec<_> = records
        .iter()
        .filter(|record| record.kind == DeliveryKind::Irq)
        .collect();
    assert_eq!(irqs.len(), 4);
    for (index, irq) in irqs.iter().enumerate() {
        assert_eq!(irq.vector, 0x20);
        assert_eq!(
            (irq.cpl_before, irq.cpl_after),
            (if index < 3 { 0 } else { 3 }, 0)
        );
    }
    let faults: Vec<_> = records
        .iter()
        .filter(|record| record.kind == DeliveryKind::Fault)
        .collect();
    assert_eq!(faults.len(), 1);
    assert_eq!((faults[0].vector, faults[0].error_code), (13, Some(0)));
    assert_eq!(
        records
            .iter()
            .filter(|record| {
                record.kind == DeliveryKind::Software
                    && record.cpl_before == 3
                    && record.cpl_after == 0
                    && record.vector == 0x40
            })
            .count(),
        2
    );
    Run {
        text,
        retired,
        clock: assembled.machine.clock(),
        deliveries,
    }
}

fn validated_fixture() -> Option<Vec<u8>> {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let rom = fixtures::fixture(IMAGE)?;
    assert_eq!(rom.len(), 128 * 1024);
    let pin = fixtures::parse_pin(
        include_str!("../../../vendor/i386-pm-test/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    )
    .unwrap();
    assert_eq!(assembly::test_support::fixture_sha256(&rom), pin);
    Some(rom)
}

#[test]
fn i386_protected_mode_rom_is_deterministic() {
    let Some(rom) = validated_fixture() else {
        return;
    };
    let first = run(rom.clone(), SchedulerEngine::Deadline, CAP);
    let second = run(rom, SchedulerEngine::Deadline, CAP);
    assert_eq!(first, second, "transcript, retired count and virtual time");
}

#[test]
fn i386_protected_mode_schedulers_and_run_chunks_agree() {
    let Some(rom) = validated_fixture() else {
        return;
    };
    let whole = run(rom.clone(), SchedulerEngine::Deadline, CAP);
    for (engine, chunk) in [
        (SchedulerEngine::Deadline, 1),
        (SchedulerEngine::Deadline, 17),
        (SchedulerEngine::PerClock, 257),
    ] {
        assert_eq!(
            whole,
            run(rom.clone(), engine, chunk),
            "{engine:?} chunk={chunk}"
        );
    }
}
