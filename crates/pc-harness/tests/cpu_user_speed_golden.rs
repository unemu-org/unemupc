// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deterministic throughput fixtures for Intel 80386 PRM §5 (segments/pages),
//! §6.3 (ring transitions) and §15 (V86). See windows95-on-486.md §3.

use cpu::{Cpu, CpuReg};
use machine::RunOutcome;
#[path = "support/speed.rs"]
mod speed;
#[path = "support/user_speed.rs"]
mod user_speed;

fn qualify(case: user_speed::Case) {
    for (board, chip) in user_speed::CHIPS {
        let mut rig = case.rig(board, chip);
        eprintln!(
            "{} {} setup retired={} time={}",
            chip.name(),
            case.name,
            rig.machine.cpu().retired(),
            rig.machine.virtual_time()
        );
        let before = rig.machine.cpu().retired();
        let cap = rig.machine.virtual_time() + 10_000;
        if case.name == "paged-protected16-cpl3" {
            // Observe the far target before RETF hides its selector at the loop boundary.
            let checkpoint = rig.machine.virtual_time() + 14;
            assert_eq!(
                rig.machine.run_bounded(Some(checkpoint)).unwrap(),
                RunOutcome::RetiredCapReached
            );
            assert_eq!(rig.machine.cpu().retired() - before, 14);
            check_far_callee(&rig);
        }
        assert_eq!(
            rig.machine.run_bounded(Some(cap)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(rig.machine.cpu().retired() - before, 10_000);
        case.check(&rig, case.iterations(10_000));
    }
}

#[test]
fn windows_regime_speed_flat32_cpl3() {
    qualify(user_speed::CASES[0]);
}
#[test]
fn windows_regime_speed_flat_paged32_cpl3() {
    qualify(user_speed::CASES[1]);
}
#[test]
fn windows_regime_speed_paged_protected16_cpl3() {
    qualify(user_speed::CASES[3]);
}
#[test]
fn windows_regime_speed_paged_v86() {
    qualify(user_speed::CASES[4]);
}
#[test]
fn windows_regime_speed_paged_v86_iopl0() {
    qualify(user_speed::CASES[5]);
}

#[test]
fn windows_regime_speed_flat_paged32_cpl3_farcall() {
    qualify(user_speed::CASES[2]);
}

fn check_far_callee(rig: &assembly::AssembledMachine) {
    let cpu = rig.machine.cpu();
    let state = cpu.diagnostic_state();
    let system = state.system.unwrap();
    let cs = state.segments[1];
    assert_eq!(
        cs.selector, 0x17,
        "far callee must use the second LDT code segment"
    );
    assert_eq!(cs.base, 0xf9000);
    assert_eq!(cs.attributes & 0x4000, 0, "callee must have D=0");
    assert_eq!(cs.limit, 0xffff);
    assert_eq!(cpu.reg(CpuReg::Rip), 0);
    assert_eq!(system.cpl, 3);
    assert!(system.protected() && system.paging());
    assert!(!system.virtual_8086);
}
