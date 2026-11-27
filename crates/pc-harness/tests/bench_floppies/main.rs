// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keeps the bootable speed-ceiling floppies in `tools/bench-floppies/`
//! working and tied to the ROM loops they copy. Every case must assemble into
//! a 1.44 MB boot floppy whose loop bytes match its ROM fixture, and two short
//! cases must boot on the generic 486 ISA board with SeaBIOS and run exactly
//! the loop instructions their sources promise.
//!
//! The boot sector checks follow the IBM Personal Computer AT Technical
//! Reference (INT 19h bootstrap: AA55h signature at offset 510) and the
//! Microsoft FAT32 File System Specification (BPB and FAT12 layout). See
//! `backlog/docs/design/bench-floppies.md`.

mod support;

use cpu::Cpu;
use support::{CASES, IMAGE_BYTES, PAYLOAD_BYTES};

fn read_image(dir: &std::path::Path, case: &str) -> Vec<u8> {
    std::fs::read(dir.join(format!("{case}.img"))).unwrap()
}

fn payload(image: &[u8]) -> &[u8] {
    &image[512..512 + PAYLOAD_BYTES]
}

#[test]
fn bench_floppies_build_as_boot_floppies() {
    let temp = machine_file::test_support::TempDir::new("bench-floppies");
    if support::build(temp.path(), None).is_none() {
        return;
    }
    for case in CASES {
        let name = case.name;
        let image = read_image(temp.path(), name);
        assert_eq!(image.len(), IMAGE_BYTES, "{name}");
        assert_eq!(image[510..512], [0x55, 0xaa], "{name}: boot signature");
        assert_eq!(image[0], 0xeb, "{name}: short jump over the BPB");
        // Bytes per sector, reserved sectors (boot sector and 32 payload
        // sectors), total sectors and sectors per track.
        assert_eq!(image[11..13], 512u16.to_le_bytes(), "{name}");
        assert_eq!(image[14..16], 33u16.to_le_bytes(), "{name}");
        assert_eq!(image[19..21], 2880u16.to_le_bytes(), "{name}");
        assert_eq!(image[24..26], 18u16.to_le_bytes(), "{name}");
        // Both FATs start with the media byte and the cluster 1 marker.
        for fat in [33, 42] {
            assert_eq!(
                image[fat * 512..fat * 512 + 3],
                [0xf0, 0xff, 0xff],
                "{name}"
            );
        }
        // The published count, not a test override.
        let start = format!(
            "UNEMU-BENCH {name} START count={}\0",
            case.count(case.passes)
        );
        assert!(
            payload(&image)
                .windows(start.len())
                .any(|window| window == start.as_bytes()),
            "{name}: START text {start:?}"
        );
    }
}

/// Where the branch at `at` lands, for a short JMP/Jcc (`EB`/`7x`) or a near
/// JMP (`E9`) or CALL (`E8`) with a 16-bit or 32-bit displacement.
fn branch_target(code: &[u8], at: usize, wide: bool) -> Option<usize> {
    let (len, disp) = match code[at] {
        0xeb | 0x70..=0x7f => (2, i64::from(code[at + 1] as i8)),
        0xe8 | 0xe9 if wide => (
            5,
            i64::from(i32::from_le_bytes(code[at + 1..at + 5].try_into().unwrap())),
        ),
        0xe8 | 0xe9 => (
            3,
            i64::from(i16::from_le_bytes(code[at + 1..at + 3].try_into().unwrap())),
        ),
        _ => return None,
    };
    usize::try_from(at as i64 + len + disp).ok()
}

/// The ROM's register loop: four body instructions then `JMP` back to them.
/// The floppy's copy closes with `JNZ` instead; the body bytes must be equal.
fn compare_register_loop(name: &str, rom: &[u8], floppy: &[u8]) {
    let entry = 0x1000;
    let close = (entry..entry + 32)
        .find(|&at| floppy[at] == 0x75 && branch_target(floppy, at, false) == Some(entry))
        .unwrap_or_else(|| panic!("{name}: no JNZ closing the floppy loop"));
    let body = &floppy[entry..close];
    let found = (0..rom.len() - body.len() - 2).any(|at| {
        rom[at..at + body.len()] == *body
            && rom[at + body.len()] == 0xeb
            && branch_target(rom, at + body.len(), false) == Some(at)
    });
    assert!(found, "{name}: ROM has no JMP loop with body {body:02x?}");
}

/// The ROM's user loop at offset 2000h against the floppy's, byte for byte,
/// masking only the near-CALL displacement to the near callee (which moves
/// because the loop end differs) and the far-call target (the callee's
/// address, which moves with the load address).
fn compare_user_loop(name: &str, rom: &[u8], floppy: &[u8]) {
    let wide = name.starts_with("flat");
    let entry = 0x2000;
    let near_callee = [0x83, 0xc0, 0x01, 0xc3];
    let jump_len = |code: &[u8], at: usize| match code[at] {
        0xeb => 2,
        0xe9 if wide => 5,
        _ => 3,
    };
    // The ROM loop ends with a JMP back to the entry, right before the callee.
    let rom_end = (entry..entry + 0x400)
        .find(|&at| {
            matches!(rom[at], 0xeb | 0xe9)
                && branch_target(rom, at, wide) == Some(entry)
                && rom[at + jump_len(rom, at)..at + jump_len(rom, at) + 4] == near_callee
        })
        .unwrap_or_else(|| panic!("{name}: no closing JMP in the ROM loop"));
    let rom_callee = rom_end + jump_len(rom, rom_end);
    // The floppy loop ends with CMP EBP, JB, UD2, then the callee.
    let floppy_callee = (entry..entry + 0x400)
        .find(|&at| floppy[at..at + 2] == [0x0f, 0x0b] && floppy[at + 2..at + 6] == near_callee)
        .unwrap_or_else(|| panic!("{name}: no UD2 before the floppy callee"))
        + 2;
    let rel = if wide { 4 } else { 2 };
    let far_target = |code: &[u8], at: usize| {
        let operand = &code[at + 1..at + 1 + rel + 2];
        let (offset, selector) = operand.split_at(rel);
        let offset = offset
            .iter()
            .rev()
            .fold(0u32, |value, byte| value << 8 | u32::from(*byte));
        let selector = u32::from(u16::from_le_bytes(selector.try_into().unwrap()));
        if wide {
            (selector == 0x1b).then_some(offset)
        } else {
            Some(selector * 16 + offset)
        }
    };
    let mut masked = vec![false; rom_end];
    let (mut near, mut far) = (0, 0);
    for at in entry..rom_end {
        if rom[at] == 0xe8
            && floppy[at] == 0xe8
            && branch_target(rom, at, wide) == Some(rom_callee)
            && branch_target(floppy, at, wide) == Some(floppy_callee)
        {
            masked[at + 1..at + 1 + rel].fill(true);
            near += 1;
        }
        // CALL ptr16:16 or ptr16:32 to the far callee, at F9000h in the ROM
        // and at 11000h on the floppy: a V86 segment:0, or flat selector 1Bh.
        if rom[at] == 0x9a
            && floppy[at] == 0x9a
            && at + 1 + rel + 2 <= rom_end
            && far_target(rom, at) == Some(0xf9000)
            && far_target(floppy, at) == Some(0x11000)
        {
            masked[at + 1..at + 1 + rel + 2].fill(true);
            far += 1;
        }
    }
    assert!(near > 0, "{name}: no near call to the callee");
    for at in entry..rom_end {
        assert!(
            masked[at] || rom[at] == floppy[at],
            "{name}: loop byte {at:#x} differs: ROM {:02x?}, floppy {:02x?}",
            &rom[at..(at + 8).min(rom_end)],
            &floppy[at..at + 8]
        );
    }
    // Then the floppy counts passes where the ROM jumped back.
    let counter: &[u8] = if wide {
        &[0x81, 0xfd]
    } else {
        &[0x66, 0x81, 0xfd]
    };
    assert_eq!(
        &floppy[rom_end..rom_end + counter.len()],
        counter,
        "{name}: CMP EBP after the loop body"
    );
    assert_eq!(
        rom[rom_callee..rom_callee + 4],
        floppy[floppy_callee..floppy_callee + 4]
    );
    // The far callee at offset 1000h, through its RET or RETF.
    assert_eq!(
        rom[0x1000..0x1010],
        floppy[0x1000..0x1010],
        "{name}: callee"
    );
    eprintln!("{name}: loop bytes match, {near} near and {far} far call operands masked");
}

#[test]
fn bench_floppy_loops_match_the_rom_fixtures() {
    let temp = machine_file::test_support::TempDir::new("bench-floppies-roms");
    if support::build(temp.path(), None).is_none() {
        return;
    }
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for case in CASES {
        let rom = std::fs::read(fixtures.join(case.rom)).unwrap();
        let image = read_image(temp.path(), case.name);
        if case.register_loop() {
            compare_register_loop(case.name, &rom, payload(&image));
        } else {
            compare_user_loop(case.name, &rom, payload(&image));
        }
    }
}

/// Boot `name` built with `passes`, check START, run from the first loop
/// instruction exactly the loop's own instruction count and check that the CPU
/// then stands on the first instruction after the loop; then run to the END
/// line and check what the loop left behind.
fn run_exactly(name: &str, passes: u64) {
    let temp = machine_file::test_support::TempDir::new(&format!("bench-floppy-{name}"));
    if support::build(temp.path(), Some(passes)).is_none() {
        return;
    }
    let Some(config) = support::config(read_image(temp.path(), name)) else {
        return;
    };
    let count = support::find(name).count(passes);
    let mut assembled = support::assemble(&config);
    // Slices shorter than the case setup, so START is seen before the loop.
    let found = support::boot_to_start(&mut assembled, 1000);
    assert_eq!(found, (name.to_owned(), count));

    let entry = support::loop_entry(name);
    while support::code_linear(assembled.machine.cpu()) != entry {
        support::assert_running(&assembled, "before the loop");
        assert!(
            assembled.machine.cpu().retired() < 100_000_000,
            "the loop was not entered: {}",
            assembled.machine.cpu().diagnostic_state()
        );
        let cap = assembled.machine.virtual_time() + 1;
        support::run(&mut assembled, cap);
    }
    assert!(
        support::counters_at_start(name, passes, assembled.machine.cpu()),
        "{name}: entry reached after the first pass: {}",
        assembled.machine.cpu().diagnostic_state()
    );
    let entered = assembled.machine.cpu().retired();
    let expected = support::loop_retired(name, count);
    for target in [expected / 2, expected] {
        while assembled.machine.cpu().retired() - entered < target {
            support::assert_running(&assembled, "inside the loop");
            let cap = assembled.machine.virtual_time()
                + (target - (assembled.machine.cpu().retired() - entered));
            support::run(&mut assembled, cap);
        }
        assert_eq!(assembled.machine.cpu().retired() - entered, target);
        if target != expected {
            support::regime(name, assembled.machine.cpu()).unwrap();
        }
    }
    // Exactly the promised instructions after entry, the CPU stands on the
    // first instruction after the last pass.
    let exit = support::exit_bytes(name);
    let mut code = vec![0; exit.len()];
    let at = support::code_linear(assembled.machine.cpu());
    assembled
        .machine
        .cpu()
        .read_mem(u64::from(at), &mut code)
        .unwrap();
    assert_eq!(
        code, exit,
        "{name}: first instruction after the loop at {at:#x}"
    );
    assert!(at > entry);

    let chunk = support::steps_per_ms(&config);
    while !assembled.machine.halted() {
        assert!(
            assembled.machine.cpu().retired() - entered < expected + 100_000,
            "no halt after the loop"
        );
        let cap = assembled.machine.virtual_time() + chunk;
        support::run(&mut assembled, cap);
    }
    assert_eq!(
        support::marker(&support::text(&assembled), "END"),
        Some((name.to_owned(), count))
    );
    support::end_state(name, count, assembled.machine.cpu());
}

/// The V86 case with the trapped port read exercises the most of the user
/// setup: paging, the TSS, the monitor's #GP handler and the UD2 exit.
#[test]
fn bench_floppy_runs_the_exact_v86_loop_on_seabios() {
    run_exactly("paged-v86-iopl0", 1000);
}

/// The ring-0 register loop under protected mode and paging, one pass of
/// 65536 iterations.
#[test]
fn bench_floppy_runs_the_exact_paged32_loop_on_seabios() {
    run_exactly("paged32", 1);
}
