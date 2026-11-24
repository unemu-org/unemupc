// SPDX-License-Identifier: AGPL-3.0-or-later
//! Intel486 DX instruction and identification ROM. Intel486 DX Programmer's
//! Reference Manual, §4 System Architecture, §10 Initialization and Chapter 26;
//! Embedded Intel486 Processor Family Developer's Manual, Appendix D CPUID.
//! See `backlog/docs/design/i386-pm-test.md` §486 extension for the contract.

use assembly::{
    AssemblyOptions, BoardModel, CpuChip, DeviceId, FpuChip, MachineConfig, ObservationPolicy,
    ObservationRequest, OutputKind, RamRegion,
};
use cpu::Cpu;
use device::ByteStreamKind;
use machine::{RunOutcome, SchedulerEngine};

#[path = "support/fixtures.rs"]
mod fixtures;
const CPU_REV: &str = "6a38ae4964555495c2b5c82ef72fea34e686c0b4";
const CAP: u64 = 50_000;
const COMMON: &str = "i486-pm-test\n\
reset signature ok\n\
reset cr0=60000010 ok\n\
bswap32 ok\n\
xadd register memory ok\n\
cmpxchg success failure ok\n\
cr0 wp am readback ok\n\
eflags ac readback ok\n";

fn run(
    chip: CpuChip,
    engine: SchedulerEngine,
    chunk: u64,
) -> Option<(String, u64, device::VClock)> {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let rom = fixtures::fixture("i486-pm-test/i486-pm-test.bin")?;
    assert_eq!(rom.len(), 131072);
    let pin = fixtures::parse_pin(
        include_str!("../../../vendor/i486-pm-test/build.sh"),
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
    let bytes = assembled
        .observation
        .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let retired = assembled.machine.cpu().retired();
    eprintln!(
        "{} {engine:?} chunk={chunk}: {outcome:?}; retired={retired}; wall={:?}\n{text}",
        chip.name(),
        started.elapsed()
    );
    if let Err(machine::MachineError::Cpu(error)) = &outcome {
        eprintln!("{}", assembled.machine.cpu().stop_report(error).unwrap());
    }
    if !matches!(outcome, Ok(RunOutcome::Halted)) {
        eprintln!("state: {:?}", assembled.machine.cpu().diagnostic_state());
        eprintln!("deliveries: {:?}", assembled.machine.recent_deliveries());
    }
    assert_eq!(outcome.unwrap(), RunOutcome::Halted);
    let identification = if chip == CpuChip::I80486Dx {
        "eflags id fixed; cpuid absent\n"
    } else {
        "eflags id toggle; cpuid present\ncpuid 0 GenuineIntel max=1 ok\ncpuid 1 eax=0410 edx=0001 ok\n"
    };
    assert_eq!(
        text,
        format!("{COMMON}{identification}invd wbinvd invlpg ok\nfailures 0000\n")
    );
    assert_eq!(
        retired,
        if chip == CpuChip::I80486Dx {
            1694
        } else {
            2119
        }
    );
    assert_eq!(assembled.machine.recent_deliveries().unwrap().total, 0);
    Some((text, retired, assembled.machine.clock()))
}

#[test]
fn i486_dx_protected_mode_rom() {
    assert_eq!(
        run(CpuChip::I80486Dx, SchedulerEngine::Deadline, CAP),
        run(CpuChip::I80486Dx, SchedulerEngine::Deadline, CAP)
    );
}
#[test]
fn i486_cpuid_protected_mode_rom() {
    assert_eq!(
        run(CpuChip::I80486DxCpuid, SchedulerEngine::Deadline, CAP),
        run(CpuChip::I80486DxCpuid, SchedulerEngine::Deadline, CAP)
    );
}
#[test]
fn i486_protected_mode_schedulers_and_run_chunks_agree() {
    for chip in [CpuChip::I80486Dx, CpuChip::I80486DxCpuid] {
        let whole = run(chip, SchedulerEngine::Deadline, CAP);
        for (engine, chunk) in [
            (SchedulerEngine::Deadline, 1),
            (SchedulerEngine::Deadline, 17),
            (SchedulerEngine::PerClock, 257),
        ] {
            assert_eq!(whole, run(chip, engine, chunk));
        }
    }
}
fn board(rom: Vec<u8>, chip: CpuChip) -> MachineConfig {
    let mut config = assembly::bios(rom, None);
    let uart = config
        .devices
        .iter()
        .find(|device| device.id == DeviceId::Uart)
        .unwrap()
        .clone();
    config.images[0].base = 0xE0000;
    config.board = BoardModel::Generic486Isa;
    config.clock = config.board.default_clock();
    config.cpu.chip = chip;
    config.cpu.fpu = FpuChip::Integrated;
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
