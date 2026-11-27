// SPDX-License-Identifier: AGPL-3.0-or-later
//! Host-only throughput probe for the bootable speed-ceiling floppies.
//!
//! Build the images with `tools/bench-floppies/build.sh <directory>`, then run
//! `cargo run --release -p pc-harness --example bench_floppy -- <directory>/*.img`
//! from the repository root. Each image boots on the generic 486 ISA board with
//! SeaBIOS from A: (no hard disk) and runs uncapped in 1 ms slices of virtual
//! time, the way an unthrottled front end slices a run. For each image the
//! probe prints the iteration count, the instructions retired between the
//! START and END lines on the text screen, the host seconds between them and
//! the resulting rate.
//!
//! Samples taken during the run check the processor regime the case is meant to
//! measure (Intel 80386 Programmer's Reference Manual §5.1/§5.2 segments and
//! pages, §6.3 privilege levels, §15 virtual-8086 mode and IOPL). The retired
//! count between the markers must match the loop's own instruction count, and
//! the registers left behind must match the iteration count. See
//! `backlog/docs/design/bench-floppies.md`.

#[path = "../tests/bench_floppies/support.rs"]
#[allow(dead_code)]
mod support;

use cpu::Cpu;
use std::time::Instant;

fn main() {
    let images: Vec<String> = std::env::args().skip(1).collect();
    if images.is_empty() {
        eprintln!(
            "usage: bench_floppy <image>...  (build them with tools/bench-floppies/build.sh)"
        );
        std::process::exit(2);
    }
    for path in images {
        let Some(config) = support::config(std::fs::read(&path).unwrap()) else {
            std::process::exit(1);
        };
        measure(&path, &config);
    }
}

fn measure(label: &str, config: &assembly::MachineConfig) {
    let mut assembled = support::assemble(config);
    let chunk = support::steps_per_ms(config);
    // Boot: BIOS POST, the floppy read and the case's own real-mode prelude.
    let (case, count) = support::boot_to_start(&mut assembled, chunk);
    let expected = support::loop_retired(&case, count);
    let start_retired = assembled.machine.cpu().retired();
    let start = Instant::now();

    // Timed window. The text screen is read only when the CPU halts, which the
    // payload does right after the END line, and at the regime samples.
    let (mut matched, mut samples, mut chunks) = (0u64, 0u64, 0u64);
    let mut first_miss = None;
    let wall = loop {
        let cap = assembled.machine.virtual_time() + chunk;
        support::run(&mut assembled, cap);
        chunks += 1;
        if assembled.machine.halted() {
            let elapsed = start.elapsed().as_secs_f64();
            if let Some(end) = support::marker(&support::text(&assembled), "END") {
                assert_eq!(end, (case.clone(), count), "{label}");
                break elapsed;
            }
            // A halt without the END line cannot end, and no slice advances it.
            support::assert_running(&assembled, &format!("in {label} without the END line"));
        }
        if chunks % 1000 == 2 {
            samples += 1;
            match support::regime(&case, assembled.machine.cpu()) {
                Ok(()) => matched += 1,
                Err(why) => {
                    first_miss.get_or_insert(why);
                }
            }
        }
        assert!(
            assembled.machine.cpu().retired() - start_retired
                < expected + expected / 4 + 10_000_000,
            "{label}: no END line\n{}",
            support::text(&assembled)
        );
    };
    let retired = assembled.machine.cpu().retired() - start_retired;
    // The window is slice aligned: it can miss up to a slice of the loop's start
    // and holds the case setup, the END store and up to a slice after the halt.
    // Anything more means the loop ran a different number of times.
    assert!(
        (expected.saturating_sub(chunk)..expected + 2 * chunk + 100_000).contains(&retired),
        "{label}: retired {retired}, loop alone {expected}"
    );
    // In the trapped V86 case a sample can land in the ring-0 port monitor.
    // A run shorter than two slices (a test build with a tiny count) takes no
    // sample; the published counts take dozens.
    assert!(
        samples == 0 || matched > 0,
        "{label}: no sample in the intended regime ({samples} samples): {first_miss:?}"
    );
    support::end_state(&case, count, assembled.machine.cpu());
    println!(
        "{case}: count={count} retired={retired} loop={expected} seconds={wall:.3} MIPS={:.3} regime={matched}/{samples}",
        retired as f64 / wall / 1e6
    );
    if let Some(why) = first_miss {
        println!("  first sample outside the loop regime: {why}");
    }
}
