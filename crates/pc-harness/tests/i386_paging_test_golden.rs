// SPDX-License-Identifier: AGPL-3.0-or-later
//! Two-level paging on generic ISA boards. Intel 80386 Programmer's Reference
//! Manual §5.2 Page Translation, §6.4 Page-Level Protection, §9.8.14 Page Fault;
//! Intel486 Family Programmer's Reference Manual §4.1.3 CR0.WP and §26 INVLPG.
//! See `backlog/docs/design/i386-pm-test.md` for the transcript contract.

use assembly::{
    AssemblyOptions, BoardModel, CpuChip, DeviceId, FpuChip, MachineConfig, ObservationPolicy,
    ObservationRequest, OutputKind, RamRegion,
};
use cpu::Cpu;
use device::ByteStreamKind;

#[path = "support/fixtures.rs"]
mod fixtures;

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

const COMMON: &str = "i386-paging-test\n\
identity alias ok\n\
pde pte accessed ok\n\
alias write dirty ok\n\
#pf cr2=00401000 error=00000000\n\
not present resume ok\n\
cr3 equal reload ok\n\
cr3 new directory ok\n";
const USER: &str = "#pf cr2=00402000 error=00000005\n\
#pf cr2=00403000 error=00000007\n\
#pf cr2=00800000 error=00000005\n\
#pf cr2=00C00000 error=00000007\n\
ring3 pte pde us rw ok\n\
ring3 writable ok\n\
supervisor wp=0 write ok\n";

fn run(chip: CpuChip) {
    assert!(
        include_str!("../../../Cargo.lock").contains("641d4d9698c79dfd351716c9f7fac8ed0bf00c90")
    );
    let Some(rom) = fixtures::fixture("i386-paging-test/i386-paging-test.bin") else {
        return;
    };
    assert_eq!(rom.len(), 131072);
    let hash = fixtures::parse_pin(
        include_str!("../../../vendor/i386-paging-test/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    )
    .unwrap();
    assert_eq!(assembly::test_support::fixture_sha256(&rom), hash);
    let policy = ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::DebugConsole,
            output: OutputKind::Bytes(ByteStreamKind::DebugConsole),
            byte_limit: 4096,
        }],
        ..Default::default()
    };
    let is_386 = chip == CpuChip::I80386;
    let expected = format!(
        "{COMMON}{}{USER}{}failures 00000000\n",
        if is_386 {
            "invlpg unavailable on 386\n"
        } else {
            "invlpg pte change ok\n"
        },
        if is_386 {
            "wp unavailable on 386\n"
        } else {
            "#pf cr2=00403000 error=00000003\n#pf cr2=00402000 error=00000003\nsupervisor wp=1 user supervisor ro ok\n"
        }
    );
    for chunk in [500_000, 17] {
        let mut rig = assembly::assemble_with_observation_and_options(
            &board(rom.clone(), chip),
            &policy,
            AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap();
        let start = std::time::Instant::now();
        let outcome = loop {
            let cap = (rig.machine.virtual_time() + chunk).min(500_000);
            let outcome = rig.machine.run_bounded(Some(cap));
            if !matches!(outcome, Ok(machine::RunOutcome::RetiredCapReached)) || cap == 500_000 {
                break outcome;
            }
        };
        let wall = start.elapsed();
        let text = String::from_utf8(
            rig.observation
                .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
                .unwrap(),
        )
        .unwrap();
        eprintln!(
            "{}: {outcome:?}; retired={}; wall={wall:?}\n{text}\n{}",
            chip.name(),
            rig.machine.cpu().retired(),
            rig.machine.cpu().diagnostic_state()
        );
        assert_eq!(outcome.unwrap(), machine::RunOutcome::Halted);
        assert_eq!(text, expected);
        assert_eq!(
            rig.machine.cpu().retired(),
            if is_386 { 8_666 } else { 9_373 }
        );
    }
}
#[test]
fn paging_i386_golden() {
    run(CpuChip::I80386);
}
#[test]
fn paging_i486_dx_golden() {
    run(CpuChip::I80486Dx);
}
#[test]
fn paging_i486_cpuid_golden() {
    run(CpuChip::I80486DxCpuid);
}
