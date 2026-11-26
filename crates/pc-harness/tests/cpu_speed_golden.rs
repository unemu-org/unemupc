// SPDX-License-Identifier: AGPL-3.0-or-later
//! Instruction throughput between IRQ0 ticks (Intel 8254 datasheet, Mode 2;
//! Intel 8259A datasheet, ICWs and EOI). See machine-files.md §Nominal CPU clocks.

use assembly::{AssemblyOptions, BoardModel, ClockConfig, CpuChip};
use machine::RunOutcome;
use std::num::NonZeroU32;
#[path = "support/speed.rs"]
mod speed;
const ROM: &[u8] = include_bytes!("fixtures/cpu-speed.bin");
const CPU_REV: &str = "6758d3d5ce24e7994570214d9ef3e2940d66889b";

fn rig(mhz: u32, turbo: Option<u32>) -> assembly::AssembledMachine {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let mut config = speed::board(ROM, BoardModel::Generic386Isa);
    config.clock = ClockConfig::from_khz(CpuChip::I80386, NonZeroU32::new(mhz * 1000).unwrap());
    if let Some(mhz) = turbo {
        let clock = ClockConfig::from_khz(CpuChip::I80386, NonZeroU32::new(mhz * 1000).unwrap());
        config.clock.turbo = Some((
            clock.retired_per_pit_clock,
            clock.retired_per_pit_clock_denominator,
        ));
    }
    assembly::assemble_with_options(
        &config,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap()
}

fn word(rig: &assembly::AssembledMachine, address: u64) -> u16 {
    let mut bytes = [0; 2];
    rig.machine.cpu().read_mem(address, &mut bytes).unwrap();
    u16::from_le_bytes(bytes)
}

fn ticks(rig: &mut assembly::AssembledMachine, count: u16, chunk: u64) {
    while word(rig, 0x500) < count {
        let cap = rig.machine.virtual_time() + chunk;
        assert!(cap <= 1_000_000);
        assert_eq!(
            rig.machine.run_bounded(Some(cap)).unwrap(),
            RunOutcome::RetiredCapReached
        );
    }
}

fn samples(rig: &assembly::AssembledMachine, from: u64, count: u64) -> Vec<u16> {
    (from..from + count)
        .map(|index| word(rig, 0x502 + 2 * index))
        .collect()
}

#[test]
fn irq0_loop_counts_follow_386_clock_presets() {
    for (mhz, expected) in [
        (16, [6861, 6860, 6861]),
        (25, [10722, 10723, 10723]),
        (33, [14156, 14155, 14156]),
    ] {
        let mut rig = rig(mhz, None);
        ticks(&mut rig, 4, 500);
        let got = samples(&rig, 1, 3);
        // 4096 PIT clocks per tick, two steps per loop and ten ISR steps.
        let clock = ClockConfig::from_khz(CpuChip::I80386, NonZeroU32::new(mhz * 1000).unwrap());
        let interval_steps =
            4096 * clock.retired_per_pit_clock / clock.retired_per_pit_clock_denominator;
        for &count in &got {
            assert!(
                (i64::from(count) - (interval_steps as i64 - 10) / 2).abs() <= 2,
                "{mhz} MHz: {got:?}"
            );
        }
        assert_eq!(got, expected);
    }
}

fn run_to(rig: &mut assembly::AssembledMachine, cap: u64, chunk: u64) {
    while rig.machine.virtual_time() < cap {
        let next = (rig.machine.virtual_time() + chunk).min(cap);
        assert_eq!(
            rig.machine.run_bounded(Some(next)).unwrap(),
            RunOutcome::RetiredCapReached
        );
    }
}

fn switched(chunk: u64) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
    let mut rig = rig(16, Some(33));
    run_to(&mut rig, 50_000, chunk);
    let low = samples(&rig, 1, 2);
    let clock = rig.machine.clock();
    let time = rig.machine.virtual_time();
    assert!(rig.machine.set_turbo(true));
    assert!(rig.machine.turbo_enabled());
    assert_eq!(rig.machine.clock(), clock);
    assert_eq!(rig.machine.virtual_time(), time);
    run_to(&mut rig, 200_000, chunk);
    let high = samples(&rig, 5, 2);
    assert!(rig.machine.set_turbo(false));
    run_to(&mut rig, 300_000, chunk);
    (low, high, samples(&rig, 10, 2))
}

#[test]
fn turbo_button_changes_irq0_loop_counts_deterministically() {
    let run = switched(1);
    assert_eq!(run, switched(1));
    assert_eq!(run, switched(97));
    assert!(run.1.iter().all(|&n| n > run.0[0] * 2));
    assert!(run.2.iter().all(|&n| n.abs_diff(run.0[0]) <= 1));
}
