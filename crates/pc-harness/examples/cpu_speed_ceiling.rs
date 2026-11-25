// SPDX-License-Identifier: AGPL-3.0-or-later
//! Host-only headless throughput measurement. Intel 80286 and 80287
//! Programmer's Reference Manual, System Initialization (protected-mode entry).
//! Intel 80386 Programmer's Reference Manual §5.1 (segment translation) and §5.2
//! (page translation) define the additional 32-bit cases.
//! See `backlog/docs/design/machine-files.md` §Measured speed ceilings.

use assembly::{AssemblyOptions, BoardModel, CpuChip};
use cpu::{Cpu, CpuReg};
use machine::RunOutcome;
use std::time::Instant;
#[path = "../tests/support/speed.rs"]
mod speed;
#[path = "../tests/support/user_speed.rs"]
mod user_speed;

fn main() {
    assert!(
        include_str!("../../../Cargo.lock").contains("786b5fd04dd895a696f951fcdf26828f55bfb3d6")
    );
    for (board, chip) in [(BoardModel::Ibm5170, CpuChip::I80286)]
        .into_iter()
        .chain(user_speed::CHIPS)
    {
        let label = if chip == CpuChip::I80486DxCpuid {
            "generic-486-isa/80486dx-cpuid"
        } else {
            board.id()
        };
        let protected = if board == BoardModel::Ibm5170 {
            (
                "protected16",
                include_bytes!("../tests/fixtures/cpu-ceiling-protected.bin").as_slice(),
            )
        } else {
            (
                "protected32",
                include_bytes!("../tests/fixtures/cpu-ceiling-protected32.bin").as_slice(),
            )
        };
        let mut cases = vec![
            (
                "real16",
                include_bytes!("../tests/fixtures/cpu-ceiling-real.bin").as_slice(),
            ),
            protected,
        ];
        if board != BoardModel::Ibm5170 {
            cases.extend_from_slice(&[
                (
                    "paged32",
                    include_bytes!("../tests/fixtures/cpu-ceiling-paged32.bin").as_slice(),
                ),
                (
                    "flat32",
                    include_bytes!("../tests/fixtures/cpu-ceiling-flat32.bin").as_slice(),
                ),
                (
                    "flat-paged32",
                    include_bytes!("../tests/fixtures/cpu-ceiling-flat-paged32.bin").as_slice(),
                ),
            ]);
        }
        for (mode, rom) in cases {
            let mut config = speed::board(rom, board);
            config.cpu.chip = chip;
            let mut rig = assembly::assemble_with_options(
                &config,
                AssemblyOptions {
                    allow_unlisted_firmware: true,
                },
            )
            .unwrap();
            assert_eq!(
                rig.machine.run_bounded(Some(100_000)).unwrap(),
                RunOutcome::RetiredCapReached
            );
            let state = rig.machine.cpu().diagnostic_state();
            let system = state.system.unwrap();
            assert_eq!(system.protected(), mode != "real16");
            assert_eq!(system.paging(), mode.contains("paged32"));
            if mode.starts_with("flat") {
                assert_eq!(system.cpl, 0);
                assert!(!system.virtual_8086);
                for segment in state.segments {
                    assert!(!segment.unusable);
                    assert_eq!(segment.base, 0, "{mode}: segment must be flat");
                    assert_eq!(segment.limit, u32::MAX);
                    assert_eq!(
                        segment.attributes & 0xC000,
                        0xC000,
                        "32-bit, page granularity"
                    );
                    assert_eq!(segment.attributes & 0xF0, 0x90, "present, ring 0 code/data");
                }
                assert_ne!(state.segments[1].attributes & 8, 0, "CS executable");
                for index in [0, 2, 3] {
                    assert_eq!(state.segments[index].attributes & 0xA, 2, "writable data");
                }
                assert!((0xF8000..0xFFFF0).contains(&rig.machine.cpu().reg(CpuReg::Rip)));
            }
            if system.paging() {
                let mut pde = [0; 4];
                rig.machine.cpu().read_mem(0x1000, &mut pde).unwrap();
                assert_ne!(u32::from_le_bytes(pde) & 0x20, 0, "paging must set PDE.A");
            }
            for sample in 1..=3 {
                let before = rig.machine.cpu().retired();
                let start = Instant::now();
                assert_eq!(
                    rig.machine
                        .run_bounded(Some(100_000 + sample * 10_000_000))
                        .unwrap(),
                    RunOutcome::RetiredCapReached
                );
                let elapsed = start.elapsed().as_secs_f64();
                let retired = rig.machine.cpu().retired() - before;
                assert_eq!(retired, 10_000_000);
                println!(
                    "{} {mode} sample={sample}: retired={retired} seconds={elapsed:.6} MIPS={:.3}",
                    label,
                    retired as f64 / elapsed / 1e6
                );
            }
        }
        if board != BoardModel::Ibm5170 {
            let mut rigs = Vec::new();
            for case in user_speed::CASES {
                let mut rig = case.rig(board, chip);
                let start_retired = rig.machine.cpu().retired();
                let warm_cap = rig.machine.virtual_time() + 100_000;
                assert_eq!(
                    rig.machine.run_bounded(Some(warm_cap)).unwrap(),
                    RunOutcome::RetiredCapReached
                );
                case.check(&rig, case.iterations(100_000));
                rigs.push((case, rig, start_retired));
            }
            // Keep the V86 control and trapped samples adjacent to limit host-load drift.
            for sample in 1..=3 {
                for (case, rig, start_retired) in &mut rigs {
                    let before = rig.machine.cpu().retired();
                    let cap = rig.machine.virtual_time() + 10_000_000;
                    let start = Instant::now();
                    assert_eq!(
                        rig.machine.run_bounded(Some(cap)).unwrap(),
                        RunOutcome::RetiredCapReached
                    );
                    let elapsed = start.elapsed().as_secs_f64();
                    let retired = rig.machine.cpu().retired() - before;
                    assert_eq!(retired, 10_000_000);
                    case.check(
                        rig,
                        case.iterations(rig.machine.cpu().retired() - *start_retired),
                    );
                    println!("{label} {} sample={sample}: retired={retired} seconds={elapsed:.6} MIPS={:.3}", case.name, retired as f64 / elapsed / 1e6);
                }
            }
        }
    }
}
