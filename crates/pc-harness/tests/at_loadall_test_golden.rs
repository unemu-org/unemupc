// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open AT LOADALL test: the undocumented 80286 LOADALL (0F 05) copies a
//! block to memory above 1 MiB and back from real mode, through descriptor
//! caches whose base lies above 1 MiB, the way a 286 extended-memory manager
//! does.
//!
//! The guest is the project's own floppy,
//! `vendor/at-loadall-test/at-loadall-test.asm`, built by
//! `vendor/at-loadall-test/build.sh`. It prints one line for the A20 gate,
//! one for each direction of the move, one for a BIOS block move (INT 15h
//! AH=87h) that reads the block back from its physical place, an
//! informational line for 0F 04 and a failure count, to the BIOS teletype
//! and to COM1, then halts with interrupts disabled. The golden compares the COM1 bytes exactly. See
//! `backlog/docs/design/at-loadall-test.md` for what each line checks.
//!
//! Intel, "Undocumented iAPX 286 Test Instruction": LOADALL encoding (p. 2),
//! the 102-byte table at physical 800h and its layout (pp. 1-3), the table
//! requirements (p. 4). IBM Personal Computer AT Technical Reference,
//! System Board, Keyboard Controller (output port bit 1 gates A20), and BIOS
//! listing, INT 15h AH=87h (BLOCKMOVE).

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

const CPU_REV: &str = "aca03742f526a867415ea584fa27c7988b9d344f";
const IMAGE: &str = "at-loadall-test/at-loadall-test-1200.img";
const OPEN_BIOS: &str = "unemu-at-bios/bios-unemu-at.bin";
// The BIOS image of machines/at-games-open.toml, as in the at-pm-test golden.
const OPEN_BIOS_SHA256: &str = "6938dc8b665fa120f33775163ab6bacc559389525db87adedf734416f9ef2442";

// One POST, one floppy boot and two block moves stay well below this.
const CAP: u64 = 100_000_000;
// One POST, one floppy boot and the program with the open AT BIOS. Measured
// at the CPU pin above.
const GOLDEN_RETIRED: u64 = 6_407_714;

// The open AT BIOS reads the display switch with the IBM polarity; the mono
// position selects the preset's MDA, where the screen check reads.
const BOARD: &str = "[board]\ndisplay_switch = \"mono\"";

/// The transcript of a CPU that executes LOADALL: every counted line passes,
/// the BIOS finds the block at 150000h, and 0F 04 raises interrupt 6.
const EXPECTED: &str = "at-loadall-test\r\n\
     a20 ok\r\n\
     loadall write ok\r\n\
     loadall read ok\r\n\
     bios move ok\r\n\
     0f04 int 06\r\n\
     failures 0\r\n";

fn vendor(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor")
        .join(relative)
}

fn image_sha256() -> &'static str {
    fixtures::parse_pin(
        include_str!("../../../vendor/at-loadall-test/build.sh"),
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
/// KiB conventional and 1 MiB extended memory (the program writes at
/// 150000h), no coprocessor, COM1 for the transcript and the floppy as A:,
/// with the at-games-open BIOS.
fn machine_file(directory: &Path) -> PathBuf {
    let text = format!(
        "schema = 1\nextends = \"preset:ibm-5170\"\nname = \"at-loadall-test\"\n\
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
    let path = directory.join("at-loadall-test.toml");
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

/// The open AT BIOS boots the floppy; LOADALL loads an ES cache with base
/// 150000h under the selector 5000h, REP MOVSW writes the block there, a
/// second LOADALL loads a DS cache with that base and the block comes back
/// unchanged, while the conventional memory at 50000h (the selector's
/// real-mode base and the A20-masked alias) keeps its guard words. The BIOS
/// block move then finds the same block at physical 150000h.
#[test]
#[ignore = "slow golden: open BIOS POST, then the LOADALL moves"]
fn open_bios_loadall_moves_a_block_above_one_mebibyte_and_back() {
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    for relative in [OPEN_BIOS, IMAGE] {
        if fixtures::fixture(relative).is_none() {
            return;
        }
    }
    let temporary = TempDir::new("at-loadall-test-open");
    let run = run_program(&machine_file(temporary.path())).unwrap();
    eprintln!(
        "at-loadall-test open: stop={:?} retired={}\n{}",
        run.outcome, run.retired, run.com1
    );
    assert_eq!(run.outcome, RunOutcome::Halted);
    assert_eq!(run.com1, EXPECTED);
    assert!(run.screen.contains("failures 0"));
    assert_eq!(run.retired, GOLDEN_RETIRED);
}
