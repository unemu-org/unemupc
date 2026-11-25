// SPDX-License-Identifier: AGPL-3.0-or-later
//! FreeDOS floppy boot with XTIDE Universal BIOS and an ATA FAT16 hard disk.
//!
//! The I286 model takes the BIOS single-step CPU probe traps per the Intel
//! 80286 Programmer's Reference Manual, §9.7.1 and Appendix B, and delivers
//! real-mode exceptions through the IVT. This changes retired counts; only the
//! CPU detection line, its segment-load warning and shifted rows may change.
//!
//! Port 61h bits follow the PIT since 2026-11-11; delay loops take real refresh time.
//!
//! Keyboard replies now take the host and reply transfer time and the keyboard's
//! basic assurance test delay (IBM PC AT Technical Reference, 101/102-Key
//! Keyboard, "Basic Assurance Test"), so the BIOS keyboard reset finishes later
//! in virtual time; screen content is unchanged apart from the clock rows.
//! Normalized logical hashes and cursors are unchanged.
//!
//! ATA-2 §§6.1–6.2 defines the primary task file and word data register.
//! See `backlog/docs/design/firmware-and-machine-classes.md` §9 and
//! `backlog/docs/design/spec.md` §7.5 for the keyboard input path.
//! Firmware and media are pinned by machines/at-ide.toml; the CPU revision is
//! pinned at 786b5fd04dd895a696f951fcdf26828f55bfb3d6 in Cargo.lock.
//! Prompt polls use the existing DOS session cadence.

use assembly::{
    ConsoleProbe, DeviceId, InjectionSchedule, KeyboardStimulus, ObservationPolicy,
    ObservationRequest, OutputKind, RunPolicy, StopCondition,
};
use cpu::Cpu;
use device::ByteStreamKind;
use machine_file::assemble_with_observation;
use video::Frame;

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/session.rs"]
pub mod session;

fn console(assembled: &assembly::AssembledMachine) -> String {
    let Frame::Text(grid) = assembled.capture_text(DeviceId::Mda).unwrap().frame else {
        panic!("expected text frame")
    };
    grid.rows_as_strings().join("\n")
}

fn screen_hash(text: &str) -> u64 {
    // FNV-1a makes the canonical row text independent of Rust's hash seed.
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

#[test]
#[ignore = "slow: BIOS POST, XTIDE detection and an interactive FreeDOS DIR C: session"]
fn freedos_lists_the_readonly_ide_marker() {
    assert!(
        include_str!("../../../Cargo.lock").contains("#786b5fd04dd895a696f951fcdf26828f55bfb3d6")
    );
    for path in [
        "unemu-at-bios/bios-unemu-at.bin",
        "freedos/freedos.img",
        "xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin",
        "hdd/fat16.img",
    ] {
        if fixtures::fixture(path).is_none() {
            return;
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../machines/at-ide.toml");
    let config = machine_file::load(path).unwrap();
    let policy = ObservationPolicy {
        routes: vec![
            ObservationRequest {
                device: DeviceId::Mda,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            },
            ObservationRequest {
                device: DeviceId::Post,
                output: OutputKind::Bytes(ByteStreamKind::PostCheckpoints),
                byte_limit: 256,
            },
        ],
        ..ObservationPolicy::default()
    };
    let mut assembled = assemble_with_observation(&config, &policy).unwrap();
    let keyboard_index = assembled.device_index[&DeviceId::Keyboard];
    let run_policy = RunPolicy {
        retired_cap: Some(100_000_000),
        idle_limit: Some(pc_harness::BIOS_IDLE_LIMIT),
        stop: StopCondition::PromptThen {
            console: ConsoleProbe {
                device: DeviceId::Mda,
            },
            needle: pc_harness::DOS_DATE_PROMPT.into(),
            poll_step: pc_harness::DOS_INTERACTIVE_POLL_STEP,
            then_budget: pc_harness::DOS_INTERACTIVE_COMMAND_BUDGET,
        },
        stimuli: vec![KeyboardStimulus {
            keyboard_index,
            scancodes: [pc_harness::KEY_ENTER, pc_harness::KEY_ENTER].concat(),
            schedule: InjectionSchedule::AfterPrompt,
        }],
    };
    let result = assembly::run(&mut assembled.machine, &assembled.observation, &run_policy);
    eprintln!(
        "boot result: {result:?}; retired={} virtual={}\n{}",
        assembled.machine.cpu().retired(),
        assembled.machine.virtual_time(),
        console(&assembled)
    );
    let post = assembled
        .observation
        .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
        .unwrap();
    eprintln!("POST: {post:02x?}");
    assert!(result.is_ok(), "boot failed: {result:?}");
    assert!(
        console(&assembled).contains("A:\\>"),
        "FreeDOS did not reach A:\\>"
    );
    let prompt_retired = assembled.machine.cpu().retired();
    let prompt_hash = screen_hash(&console(&assembled));
    let Frame::Text(prompt_frame) = assembled.capture_text(DeviceId::Mda).unwrap().frame else {
        panic!("expected MDA text frame");
    };
    eprintln!("OBS_IDE prompt_cursor={:?}", prompt_frame.cursor());
    let prompt_logical_hash = session::logical_hash(&console(&assembled));
    // Shift-semicolon produces a colon in the BIOS's set-1 keyboard table.
    let script = [
        0x20, 0xa0, 0x17, 0x97, 0x13, 0x93, 0x39, 0xb9, 0x2e, 0xae, 0x2a, 0x27, 0xa7, 0xaa, 0x1c,
        0x9c,
    ];
    assembly::inject_scancodes(&mut assembled.machine, keyboard_index, &script);
    let cap = assembled.machine.virtual_time() + pc_harness::DOS_INTERACTIVE_COMMAND_BUDGET;
    let result = assembled.machine.run_bounded(Some(cap));
    assert!(result.is_ok(), "DIR failed: {result:?}");
    // A FAT16 free-space scan takes longer than the floppy DIR budget. Poll
    // until FreeCOM has finished the listing and returned to its next prompt.
    while !console(&assembled)
        .lines()
        .any(|line| line.trim() == "A:\\>")
    {
        let cap = assembled.machine.virtual_time() + pc_harness::DOS_INTERACTIVE_POLL_STEP;
        assert!(cap <= 40_000_000, "DIR did not return to the prompt");
        assembled.machine.run_bounded(Some(cap)).unwrap();
    }
    let console = console(&assembled);
    let Frame::Text(final_frame) = assembled.capture_text(DeviceId::Mda).unwrap().frame else {
        panic!("expected MDA text frame");
    };
    eprintln!(
        "OBS_IDE final_cursor={:?}, final_logical_hash={:016x}",
        final_frame.cursor(),
        session::logical_hash(&console)
    );
    eprintln!("DIR result: {result:?}\n{console}");
    eprintln!("OBS prompt_retired={prompt_retired} prompt_hash={prompt_hash:016x} final_retired={} final_virtual={} screen_hash={:016x}", assembled.machine.cpu().retired(), assembled.machine.virtual_time(), screen_hash(&console));
    assert!(console.contains("Volume in drive C is UNEMUHDD"));
    assert!(console
        .lines()
        .any(|line| line.split_whitespace().take(3).eq(["MARKER", "TXT", "29"])));
    // The BIOS reads 32h=20h, so FreeDOS reports Sat 01-01-2000.
    assert_eq!(prompt_logical_hash, 0xfbd3caf6e77b879d);
    assert_eq!(prompt_retired, 15_775_575);
    assert_eq!(prompt_hash, 0xa0e8f388c37aa180);
    assert_eq!(assembled.machine.cpu().retired(), 19_746_105);
    assert_eq!(assembled.machine.virtual_time(), 21_000_000);
    assert_eq!(screen_hash(&console), 0x7aae65a75645f427);
    assert_eq!(session::logical_hash(&console), 0x48d2b9699fbe5cc2);
    assert_eq!(
        post,
        [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x10, 0x12, 0x20, 0x21, 0x22,
            0x24, 0x25, 0x30, 0x31, 0x40, 0x41, 0x42, 0x43, 0x00,
        ]
    );
}
