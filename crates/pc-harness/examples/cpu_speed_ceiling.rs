// SPDX-License-Identifier: AGPL-3.0-or-later
//! Host-only headless throughput measurement. Intel 80286 and 80287
//! Programmer's Reference Manual, System Initialization (protected-mode entry).
//! See `backlog/docs/design/machine-files.md` §Measured speed ceilings.

use assembly::{AssemblyOptions, BoardModel};
use cpu::Cpu;
use machine::RunOutcome;
use std::time::Instant;
#[path = "../tests/support/speed.rs"]
mod speed;

fn main() {
    assert!(
        include_str!("../../../Cargo.lock").contains("5a17667764080c2fe7a33d446584a3e175b7923d")
    );
    for board in [
        BoardModel::Ibm5170,
        BoardModel::Generic386Isa,
        BoardModel::Generic486Isa,
    ] {
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
        for (mode, rom) in [
            (
                "real16",
                include_bytes!("../tests/fixtures/cpu-ceiling-real.bin").as_slice(),
            ),
            protected,
        ] {
            let config = speed::board(rom, board);
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
            assert_eq!(
                rig.machine
                    .cpu()
                    .diagnostic_state()
                    .system
                    .unwrap()
                    .protected(),
                mode != "real16"
            );
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
                println!(
                    "{} {mode} sample={sample}: retired={retired} seconds={elapsed:.6} MIPS={:.3}",
                    board.id(),
                    retired as f64 / elapsed / 1e6
                );
            }
        }
    }
}
