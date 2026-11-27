// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open AT privilege-level test: 80286 protected-mode code at CPL 3 and the
//! transitions between ring 0 and ring 3 that a protected-mode kernel
//! makes (Windows 3.x Standard mode, for one).
//!
//! The guest is the project's own floppy,
//! `vendor/at-privilege-test/at-privilege-test.asm`, built by
//! `vendor/at-privilege-test/build.sh`. It enters protected mode with a GDT,
//! an IDT and a 286 TSS, returns to ring 3 with IRET, takes INT 40h back to
//! ring 0 on the TSS's ring-0 stack, returns to ring 3 again with a DPL-0 ES
//! that the return nulls, takes #GP(0) for CLI at ring 3 with IOPL 0, and
//! makes a far CALL from ring 3 through a DPL-3 286 call gate that copies two
//! parameter words to ring 0, then returns with RETF 4. It prints one line
//! per case and a failure count, to COM1 and to the text
//! screen, then halts in protected mode with interrupts disabled. The golden
//! compares the COM1 bytes exactly. See
//! `backlog/docs/design/at-privilege-test.md` for what each line checks.
//!
//! Intel 80286 Programmer's Reference Manual: Protection (DPL, RPL and CPL;
//! descriptor access rights), Interrupts and Exceptions (an interrupt gate
//! to an inner level switches to the TSS's stack and pushes SS, SP, FLAGS,
//! CS and IP, then the #GP error code), Control Transfer (§7.5.1.1 and §7.5.1.4 call gates:
//! an inward CALL switches to the TSS's stack and pushes SS, SP, the copied
//! parameters, CS and IP), Multitasking (the 286 TSS's SS0:SP0) and
//! Appendix B (IRET and RET n to an outer level; CALL through a call gate;
//! CLI at CPL > IOPL raises #GP(0)).

use assembly::{
    AssemblyOptions, DeviceId, ObservationPolicy, ObservationRequest, OutputKind, RunPolicy,
};
use cpu::Cpu;
use device::ByteStreamKind;
use machine::RunOutcome;
use machine_file::test_support::TempDir;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[path = "support/fixtures.rs"]
mod fixtures;

const CPU_REV: &str = "f0785d5a2878c2b9e7392b937aa33dcdcc979f84";
const IMAGE: &str = "at-privilege-test/at-privilege-test-1200.img";
const OPEN_BIOS: &str = "unemu-at-bios/bios-unemu-at.bin";
// The BIOS image of machines/at-games-open.toml, as in the at-pm-test golden.
const OPEN_BIOS_SHA256: &str = "6938dc8b665fa120f33775163ab6bacc559389525db87adedf734416f9ef2442";

// One POST, one floppy boot and a short protected-mode program stay well
// below this.
const CAP: u64 = 100_000_000;
// One POST, one floppy boot and the program with the open AT BIOS. Measured
// at the CPU pin above.
const GOLDEN_RETIRED: u64 = 6_391_138;

// The open AT BIOS reads the display switch with the IBM polarity; the mono
// position selects the preset's MDA, where the screen check reads.
const BOARD: &str = "[board]\ndisplay_switch = \"mono\"";

/// The transcript of a CPU that runs protected-mode code at CPL 3: every
/// case passes.
const EXPECTED: &str = "at-privilege-test\r\n\
     iret to ring 3 ok\r\n\
     int to ring 0 ok\r\n\
     iret nulls es ok\r\n\
     cli gp at ring 3 ok\r\n\
     call gate to ring 0 ok\r\n\
     failures 0\r\n";

fn vendor(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor")
        .join(relative)
}

fn image_sha256() -> &'static str {
    fixtures::parse_pin(
        include_str!("../../../vendor/at-privilege-test/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    )
    .unwrap()
}

fn toml_string(path: &Path) -> String {
    let text = path.to_str().expect("UTF-8 fixture path");
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            character if character.is_control() => {
                write!(quoted, "\\u{:04x}", u32::from(character)).unwrap()
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

/// The at-pm-test machine: the game machines' memory map and RTC seed, 640
/// KiB conventional and 1 MiB extended memory, no coprocessor, COM1 for the
/// transcript and the floppy as A:, with the at-games-open BIOS.
fn machine_file(directory: &Path) -> PathBuf {
    let text = format!(
        "schema = 1\nextends = \"preset:ibm-5170\"\nname = \"at-privilege-test\"\n\
         {BOARD}\n\
         [board.rtc]\nstart = \"2000-01-01T00:00:00\"\nnvram = \"derived\"\n\
         [cpu]\nfpu = \"none\"\n\
         [memory]\nconventional_kib = 640\nextended_kib = 1024\n\
         [firmware.bios]\nprovides = [\"floppy\"]\nimage = {}\nsha256 = \"{OPEN_BIOS_SHA256}\"\n\
         [[card]]\nid = \"uart-16550\"\nport = 0x3F8\n\
         [[card]]\nid = \"fdc-765\"\n\
         [[card.drives]]\nunit = 0\ntype = \"1.2M\"\nimage = {}\nwrite = \"readonly\"\n\
         sha256 = \"{}\"\n",
        toml_string(&vendor(OPEN_BIOS)),
        toml_string(&vendor(IMAGE)),
        image_sha256()
    );
    let path = directory.join("at-privilege-test.toml");
    std::fs::write(&path, text).unwrap();
    path
}

struct Run {
    outcome: RunOutcome,
    com1: String,
    screen: String,
    retired: u64,
}

fn run_program(file: &Path) -> anyhow::Result<Run> {
    let loaded = machine_file::load_for_assembly(file, AssemblyOptions::default())?;
    let policy = ObservationPolicy {
        routes: vec![
            ObservationRequest {
                device: DeviceId::Uart,
                output: OutputKind::Bytes(ByteStreamKind::SerialTx),
                byte_limit: 65_536,
            },
            ObservationRequest {
                device: DeviceId::Mda,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            },
        ],
        ..ObservationPolicy::default()
    };
    let mut assembled = loaded.assemble(&policy)?;
    let outcome = assembled.run_session(&RunPolicy::natural(
        Some(CAP),
        Some(pc_harness::BIOS_IDLE_LIMIT),
    ))?;
    let com1 = assembled
        .observation
        .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)?;
    let video::Frame::Text(grid) = assembled.capture_text(DeviceId::Mda)?.frame else {
        anyhow::bail!("expected MDA text");
    };
    let retired = assembled.machine.cpu().retired();
    assembled.finish_persistence()?;
    Ok(Run {
        outcome,
        com1: String::from_utf8_lossy(&com1).into_owned(),
        screen: grid.rows_as_strings().join("\n"),
        retired,
    })
}

#[test]
fn uses_the_game_profile_bios_and_the_pinned_image() {
    let profile = include_str!("../../../machines/at-games-open.toml");
    assert!(profile.contains("../vendor/unemu-at-bios/bios-unemu-at.bin"));
    assert!(profile.contains(&format!("sha256 = \"{OPEN_BIOS_SHA256}\"")));
    assert_eq!(image_sha256().len(), 64);
}

/// The open AT BIOS boots the floppy; the program reaches ring 3 with IRET,
/// comes back to ring 0 through a DPL-3 interrupt gate on SS0:SP0 with the
/// ring-3 frame, returns to ring 3 with ES nulled and IOPL 0, takes #GP(0)
/// for CLI there, and calls ring 0 through a call gate with two parameter
/// words and returns with RETF 4. The text screen shows the same lines as
/// COM1.
#[test]
#[ignore = "slow golden: open BIOS POST, then the ring transitions"]
fn open_bios_program_runs_at_ring_3_and_returns_to_ring_0() {
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    for relative in [OPEN_BIOS, IMAGE] {
        if fixtures::fixture(relative).is_none() {
            return;
        }
    }
    let temporary = TempDir::new("at-privilege-test-open");
    let run = run_program(&machine_file(temporary.path())).unwrap();
    eprintln!(
        "at-privilege-test open: stop={:?} retired={}\n{}\n{}",
        run.outcome, run.retired, run.com1, run.screen
    );
    assert_eq!(run.outcome, RunOutcome::Halted);
    assert_eq!(run.com1, EXPECTED);
    for line in EXPECTED.split("\r\n").filter(|line| !line.is_empty()) {
        assert!(
            run.screen.contains(line),
            "{line:?} missing from the screen"
        );
    }
    assert_eq!(run.retired, GOLDEN_RETIRED);
}
