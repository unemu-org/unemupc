// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open AT protected-mode return test: the BIOS extended-memory services
//! (INT 15h AH=88h size, AH=87h block move), 80286 protected-mode entry, the
//! return to real mode through an 8042 pulse reset and through a triple fault,
//! both resumed through the CMOS shutdown byte, and A20 through the 8042.
//!
//! The guest is the project's own floppy, `vendor/at-pm-test/at-pm-test.asm`,
//! built by `vendor/at-pm-test/build.sh`. It prints two INT 15h lines, one
//! line per stage and round, an informational line for a faulting block
//! move, and a failure count, to the BIOS teletype and to COM1, then halts
//! with interrupts disabled. These goldens compare the COM1 bytes exactly.
//! See `backlog/docs/design/at-pm-test.md` for what each line checks.
//!
//! IBM Personal Computer AT Technical Reference: System Board, Keyboard
//! Controller (output port bits 0 and 1; commands D0h, D1h, FEh) and
//! Real-Time Clock (CMOS 0Fh shutdown status byte); BIOS listing, INT 15h
//! AH=87h (BLOCKMOVE) and AH=88h (EXT_MEMORY), and the POST reset path
//! (shutdown codes 05h and 0Ah jump through 0040:0067h, 09h resumes the
//! block move). Intel 80286 Programmer's Reference Manual, Interrupts and
//! Exceptions (double fault and shutdown) and System Control and
//! Initialization (LMSW).

use assembly::{
    AssemblyOptions, BoardModel, DeviceId, ImageSource, ObservationPolicy, ObservationRequest,
    OutputKind, RunPolicy,
};
use cpu::Cpu;
use device::ByteStreamKind;
use machine::RunOutcome;
use machine_file::test_support::TempDir;
use machine_file::test_support::{FoundUserFirmware, UserFirmwareLayout};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[path = "support/fixtures.rs"]
mod fixtures;

const CPU_REV: &str = "dd4316586f1459cec0618008b17817a0d26245a2";
const IMAGE: &str = "at-pm-test/at-pm-test-1200.img";
const OPEN_BIOS: &str = "unemu-at-bios/bios-unemu-at.bin";
// The BIOS image of machines/at-games-open.toml; checked against that file.
const OPEN_BIOS_SHA256: &str = "6938dc8b665fa120f33775163ab6bacc559389525db87adedf734416f9ef2442";
const IBM_ENTRY: &str = "ibm-5170-bios-1985-06-10";
const ROUNDS: u32 = 16;

// The open BIOS also resumes without POST; the whole run stays well below this.
const OPEN_CAP: u64 = 300_000_000;
// One POST (including the 8042 reset of its extended-memory sizing), one
// floppy boot, three INT 15h block moves and 16 rounds that resume without
// POST, with the open AT BIOS. Measured at the CPU pin above; it moved from
// 6,589,934 when the BIOS began clearing extended memory during POST.
const OPEN_GOLDEN_RETIRED: u64 = 6_590_023;
// The IBM BIOS resumes without POST, so one POST and the rounds fit easily.
const IBM_CAP: u64 = 300_000_000;
// One POST, one floppy boot, three INT 15h block moves and 16 rounds that
// resume without POST, with the user-supplied 1985-06-10 BIOS. Measured at
// the CPU pin above.
const IBM_GOLDEN_RETIRED: Option<u64> = Some(31_955_711);

// The open AT BIOS reads the display switch with the IBM polarity; the mono
// position selects the preset's MDA, where the screen check reads.
const OPEN_BOARD: &str = "[board]\ndisplay_switch = \"mono\"";
// The IBM BIOS follows the switch; without the colour card it selects MDA.
// IBM Personal Computer AT Technical Reference, BIOS listing, TEST2 and
// TEST4: on the 5170 the diskette controller shares the Fixed Disk and
// Diskette Drive Adapter, and the POST diskette attachment test (TEST2) also
// probes that adapter's fixed-disk controller (status 1F7h not busy and
// without error bits, the cylinder register at 1F4h reading back). Without
// it the ports float: POST reports 601, its failure path reaches the
// configuration check (TEST4) that leads to 162, and POST waits for F1. An
// empty AT adapter answers the probe.
const IBM_BOARD: &str = "[board]\ndisplay_switch = \"auto\"\n\
     [[card]]\nid = \"vga-text\"\nremove = true\n\
     [[card]]\nkind = \"ide\"\ncontroller_model = \"at-adapter\"\nport = 0x1F0\nirq = 14";
// The same board without the fixed-disk controller, for the probe test only.
const IBM_BOARD_WITHOUT_FIXED_DISK_ADAPTER: &str =
    "[board]\ndisplay_switch = \"auto\"\n[[card]]\nid = \"vga-text\"\nremove = true";

/// Result word of a reset stage when the BIOS honours the shutdown byte.
const RESUMED: &str = " ok";

/// Result words of the three INT 15h lines: AH=88h and the AH=87h round trip
/// before the rounds, the informational faulting AH=87h after them.
struct Int15 {
    size: &'static str,
    round_trip: &'static str,
    fault: &'static str,
}

/// The IBM listing's behaviour, which the open AT BIOS follows: both counted
/// lines pass, and the faulting move returns status 02h with no other
/// difference.
const INT15_AT: Int15 = Int15 {
    size: " ok",
    round_trip: " ok",
    fault: " ah 02 info 0000",
};

/// The transcript when every `pm` and `a20` stage passes; `int15` gives the
/// result words of the INT 15h lines, `kbc` and `tf` those of the two reset
/// stages in every round.
fn expected_transcript(int15: &Int15, kbc: &str, tf: &str, failures: u32) -> String {
    let mut text = format!("at-pm-test rounds {ROUNDS}\r\n");
    writeln!(text, "int15 88h{}\r", int15.size).unwrap();
    writeln!(text, "int15 87h{}\r", int15.round_trip).unwrap();
    for round in 1..=ROUNDS {
        write!(
            text,
            "r{round:02} pm ok\r\nr{round:02} kbc{kbc}\r\n\
             r{round:02} tf{tf}\r\nr{round:02} a20 ok\r\n"
        )
        .unwrap();
    }
    writeln!(text, "int15 87h fault{}\r", int15.fault).unwrap();
    writeln!(text, "failures {failures}\r").unwrap();
    text
}

fn vendor(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor")
        .join(relative)
}

fn image_sha256() -> &'static str {
    fixtures::parse_pin(
        include_str!("../../../vendor/at-pm-test/build.sh"),
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

/// Both profiles share the game machines' memory map and RTC seed: 640 KiB
/// conventional and 1 MiB extended memory (the program writes above 1 MiB),
/// derived CMOS, no coprocessor, COM1 for the transcript and the floppy as A:.
/// `board` carries the display switch and any preset card removal.
fn machine_file(
    directory: &Path,
    board: &str,
    firmware: &str,
    floppy: &Path,
    floppy_sha256: Option<&str>,
) -> PathBuf {
    let mut text = format!(
        "schema = 1\nextends = \"preset:ibm-5170\"\nname = \"at-pm-test\"\n\
         {board}\n\
         [board.rtc]\nstart = \"2000-01-01T00:00:00\"\nnvram = \"derived\"\n\
         [cpu]\nfpu = \"none\"\n\
         [memory]\nconventional_kib = 640\nextended_kib = 1024\n\
         [firmware.bios]\n{firmware}\n\
         [[card]]\nid = \"uart-16550\"\nport = 0x3F8\n\
         [[card]]\nid = \"fdc-765\"\n\
         [[card.drives]]\nunit = 0\ntype = \"1.2M\"\nimage = {}\nwrite = \"readonly\"\n",
        toml_string(floppy)
    );
    if let Some(sha256) = floppy_sha256 {
        writeln!(text, "sha256 = \"{sha256}\"").unwrap();
    }
    let path = directory.join("at-pm-test.toml");
    std::fs::write(&path, text).unwrap();
    path
}

/// The IBM BIOS provides both disk services, so its catalogue entry requires
/// declaring both; the open BIOS declares only the floppy service. With a
/// catalogue entry, the file pins its SHA-256 identities, taken from the
/// catalogue constants as `machines/at-games.toml` does, never from ROM bytes.
fn ibm_firmware(
    source: &ImageSource,
    identity: Option<(&assembly::FirmwareEntry, &UserFirmwareLayout)>,
) -> String {
    let mut text = String::from("provides = [\"floppy\", \"fixed-disk\"]\n");
    if let Some((entry, _)) = identity {
        writeln!(text, "sha256 = \"{}\"", entry.sha256).unwrap();
    }
    match source {
        ImageSource::File(path) => write!(text, "image = {}", toml_string(path)).unwrap(),
        ImageSource::Interleaved(parts) => {
            let known: &[_] = match identity {
                Some((entry, UserFirmwareLayout::Parts(layout))) => entry.parts[*layout],
                _ => &[],
            };
            text.push_str("parts = [\n");
            for part in parts {
                let lane = match part.lane {
                    assembly::Lane::Even => "even",
                    assembly::Lane::Odd => "odd",
                };
                write!(
                    text,
                    "{{ image = {}, lane = \"{lane}\", offset = {}",
                    toml_string(&part.path),
                    part.offset
                )
                .unwrap();
                if let Some(chip) = known
                    .iter()
                    .find(|chip| chip.lane == part.lane && chip.offset == part.offset)
                {
                    write!(text, ", sha256 = \"{}\"", chip.sha256).unwrap();
                }
                text.push_str(" },\n");
            }
            text.push(']');
        }
        _ => panic!("user firmware discovery must return file-backed sources"),
    }
    text
}

fn ibm_entry() -> &'static assembly::FirmwareEntry {
    BoardModel::Ibm5170
        .firmware_catalogue()
        .iter()
        .find(|entry| entry.id == IBM_ENTRY)
        .expect("catalogued IBM 5170 BIOS")
}

fn ibm_identity(
    found: &FoundUserFirmware,
) -> (&'static assembly::FirmwareEntry, &UserFirmwareLayout) {
    (ibm_entry(), &found.layout)
}

struct Run {
    outcome: RunOutcome,
    com1: String,
    screen: String,
    retired: u64,
}

fn run_program(file: &Path, cap: u64) -> anyhow::Result<Run> {
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
        Some(cap),
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
fn open_profile_uses_the_game_profile_bios() {
    let profile = include_str!("../../../machines/at-games-open.toml");
    assert!(profile.contains("../vendor/unemu-at-bios/bios-unemu-at.bin"));
    assert!(profile.contains(&format!("sha256 = \"{OPEN_BIOS_SHA256}\"")));
}

#[test]
fn ibm_machine_file_lowers_to_mda_com1_extended_memory_and_a_readonly_floppy() {
    let temporary = TempDir::new("at-pm-test-synthetic");
    let bios = temporary.path().join("synthetic.bin");
    let floppy = temporary.path().join("floppy.img");
    std::fs::write(&bios, vec![0xF4; 65536]).unwrap();
    std::fs::write(&floppy, vec![0; 1_228_800]).unwrap();
    let file = machine_file(
        temporary.path(),
        IBM_BOARD,
        &ibm_firmware(&ImageSource::File(bios), None),
        &floppy,
        None,
    );
    let loaded = machine_file::load_for_assembly(
        &file,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    assert_eq!(loaded.config.board, BoardModel::Ibm5170);
    assert_eq!(
        loaded.config.primary_display().unwrap(),
        assembly::PrimaryDisplay::Monochrome
    );
    assert!(loaded.cards.iter().any(|(_, id)| *id == DeviceId::Mda));
    assert!(loaded.cards.iter().any(|(_, id)| *id == DeviceId::Uart));
    assert!(!loaded.cards.iter().any(|(_, id)| *id == DeviceId::Vga));
    assert_eq!(loaded.config.drives.len(), 1);
    assert_eq!(loaded.config.drives[0].kind.image_bytes(), 1_228_800);
    assert_eq!(loaded.config.drives[0].write, assembly::WriteMode::ReadOnly);
    assert_eq!(image_sha256().len(), 64);
}

/// A discovered chip pair gets the catalogue's assembled and per-chip pins.
#[test]
fn ibm_firmware_pins_catalogue_hashes_for_each_discovered_chip() {
    let entry = ibm_entry();
    for (layout, chips) in entry.parts.iter().enumerate() {
        let source = ImageSource::Interleaved(
            chips
                .iter()
                .map(|chip| assembly::ImagePart {
                    path: PathBuf::from(format!("/chip-{}", chip.offset)),
                    lane: chip.lane,
                    offset: chip.offset,
                })
                .collect(),
        );
        let text = ibm_firmware(&source, Some((entry, &UserFirmwareLayout::Parts(layout))));
        assert!(text.contains(&format!("sha256 = \"{}\"\n", entry.sha256)));
        for chip in *chips {
            assert!(text.contains(&format!("sha256 = \"{}\" }}", chip.sha256)));
        }
    }
    let text = ibm_firmware(
        &ImageSource::File(PathBuf::from("/assembled.bin")),
        Some((entry, &UserFirmwareLayout::Assembled)),
    );
    assert!(text.starts_with(&format!(
        "provides = [\"floppy\", \"fixed-disk\"]\nsha256 = \"{}\"\nimage = ",
        entry.sha256
    )));
}

/// The IBM POST probe that failed without the adapter (see `IBM_BOARD`) reads
/// the assembled ports, not the ROM, so a synthetic BIOS shows it. The derived
/// diskette (10h) and equipment (14h) bytes do not depend on a catalogue hash
/// match: a synthetic, unlisted BIOS gets the same 1.2M drive A: and
/// monochrome, one-diskette equipment byte as the real one.
#[test]
fn ibm_machine_file_answers_the_post_fixed_disk_adapter_probe() {
    let temporary = TempDir::new("at-pm-test-probe");
    let bios = temporary.path().join("synthetic.bin");
    let floppy = temporary.path().join("floppy.img");
    std::fs::write(&bios, vec![0xF4; 65536]).unwrap();
    std::fs::write(&floppy, vec![0; 1_228_800]).unwrap();
    let options = AssemblyOptions {
        allow_unlisted_firmware: true,
    };
    let firmware = ibm_firmware(&ImageSource::File(bios), None);
    let probe = |board: &str| {
        let directory = temporary.path().join(if board == IBM_BOARD {
            "with"
        } else {
            "without"
        });
        std::fs::create_dir_all(&directory).unwrap();
        let file = machine_file(&directory, board, &firmware, &floppy, None);
        let loaded = machine_file::load_for_assembly(&file, options).unwrap();
        let cmos = assembly::derive_cmos_config(&loaded.config).unwrap();
        assert_eq!(cmos[0x10], 0x20, "drive A: is CMOS type 2, 1.2M");
        assert_eq!(cmos[0x12], 0, "no fixed disk");
        assert_eq!(cmos[0x14], 0x31, "monochrome display, one diskette drive");
        let mut assembled = loaded.assemble(&ObservationPolicy::default()).unwrap();
        let bus = assembled.machine.bus_mut();
        let status = bus.port_read(0x1F7, 1).unwrap() as u8;
        let cylinder = [0x55u8, 0xAA].map(|value| {
            bus.port_write(0x1F4, 1, u32::from(value)).unwrap();
            bus.port_read(0x1F4, 1).unwrap() as u8
        });
        (status, cylinder)
    };
    let (status, cylinder) = probe(IBM_BOARD);
    assert_eq!(status & 0x80, 0, "fixed-disk controller not busy");
    assert_eq!(status & 0x0C, 0, "no DRQ or corrected-data bits");
    assert_eq!(cylinder, [0x55, 0xAA], "cylinder register reads back");
    let (status, cylinder) = probe(IBM_BOARD_WITHOUT_FIXED_DISK_ADAPTER);
    assert_eq!(status, 0xFF, "floating status: POST sees a busy controller");
    assert_eq!(cylinder, [0xFF, 0xFF]);
}

/// IBM Personal Computer AT Technical Reference, BIOS listing, BLOCKMOVE and
/// SHUT9: INT 15h AH=87h writes its status to port 80h, resets the processor
/// through the 8042 and reads the status back once POST resumes it. On the
/// assembled board port 80h reads FFh after power-on, and a written byte
/// survives the 8042 processor reset. The synthetic BIOS is all HLT bytes,
/// so the CPU stays parked at the reset vector.
#[test]
fn ibm_machine_port_80h_keeps_its_latch_across_an_8042_processor_reset() {
    let temporary = TempDir::new("at-pm-test-port-80h");
    let bios = temporary.path().join("synthetic.bin");
    let floppy = temporary.path().join("floppy.img");
    std::fs::write(&bios, vec![0xF4; 65536]).unwrap();
    std::fs::write(&floppy, vec![0; 1_228_800]).unwrap();
    let file = machine_file(
        temporary.path(),
        IBM_BOARD,
        &ibm_firmware(&ImageSource::File(bios), None),
        &floppy,
        None,
    );
    let options = AssemblyOptions {
        allow_unlisted_firmware: true,
    };
    let loaded = machine_file::load_for_assembly(&file, options).unwrap();
    let mut assembled = loaded.assemble(&ObservationPolicy::default()).unwrap();
    let machine = &mut assembled.machine;
    assert_eq!(machine.bus_mut().port_read(0x80, 1).unwrap(), 0xFF);
    machine.bus_mut().port_write(0x80, 1, 0x02).unwrap();
    // 8042 command FEh pulses output-port bit 0, the processor reset line.
    machine.bus_mut().port_write(0x64, 1, 0xFE).unwrap();
    assert_eq!(
        machine.run_bounded(Some(1_000_000)).unwrap(),
        RunOutcome::ResetRequested
    );
    // The next run performs the reset, then halts at the reset vector.
    assert_eq!(
        machine.run_bounded(Some(2_000_000)).unwrap(),
        RunOutcome::Halted
    );
    assert_eq!(machine.bus_mut().port_read(0x80, 1).unwrap(), 0x02);
}

/// The open profile: the at-games-open BIOS (unemu-at-bios, the unemu fork of
/// Kiselev 8088_bios) on the same board, memory and RTC seed, without the
/// game cards (journaled IDE disk, Sound Blaster, EMS, mouse) that this test
/// does not exercise and CI cannot provide.
///
/// Every counted line passes, as with the IBM BIOS: the open BIOS answers
/// INT 15h AH=88h from CMOS 31h:30h (which its POST copies from 17h:18h) and
/// moves the block with AH=87h, ending it as the IBM listing does (status in
/// AH, CF and ZF from it, IF set, A20 off); both reset stages resume through
/// the shutdown byte (codes 05h and 0Ah, through 0040:0067h) without POST.
/// The faulting move reports status 02h.
#[test]
#[ignore = "slow golden: open BIOS POST, then 32 resets resumed through the shutdown byte"]
fn open_bios_resumes_the_protected_mode_test_through_the_shutdown_byte() {
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    for relative in [OPEN_BIOS, IMAGE] {
        if fixtures::fixture(relative).is_none() {
            return;
        }
    }
    let temporary = TempDir::new("at-pm-test-open");
    let firmware = format!(
        "provides = [\"floppy\"]\nimage = {}\nsha256 = \"{OPEN_BIOS_SHA256}\"",
        toml_string(&vendor(OPEN_BIOS))
    );
    let file = machine_file(
        temporary.path(),
        OPEN_BOARD,
        &firmware,
        &vendor(IMAGE),
        Some(image_sha256()),
    );
    let run = run_program(&file, OPEN_CAP).unwrap();
    eprintln!(
        "at-pm-test open: stop={:?} retired={}\n{}",
        run.outcome, run.retired, run.com1
    );
    assert_eq!(run.outcome, RunOutcome::Halted);
    assert_eq!(
        run.com1,
        expected_transcript(&INT15_AT, RESUMED, RESUMED, 0)
    );
    assert!(run.screen.contains("failures 0"));
    assert_eq!(run.retired, OPEN_GOLDEN_RETIRED);
}

/// IBM 5170 BIOS of 1985-06-10: INT 15h AH=88h returns CMOS 31h:30h, AH=87h
/// moves the block above 1 MiB and back, and both reset stages resume through
/// the shutdown byte without POST, so every counted line passes. The faulting
/// move reports status 02h.
#[test]
#[ignore = "requires the user-supplied IBM 5170 system ROM and the at-pm-test floppy"]
fn user_rom_ibm_5170_at_pm_test_resumes_through_the_shutdown_byte() {
    let catalogue = BoardModel::Ibm5170.firmware_catalogue();
    let found = machine_file::require_user_firmware!(IBM_ENTRY, catalogue);
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    if fixtures::fixture(IMAGE).is_none() {
        return;
    }
    let temporary = TempDir::new("at-pm-test-ibm");
    let file = machine_file(
        temporary.path(),
        IBM_BOARD,
        &ibm_firmware(&found.source, Some(ibm_identity(&found))),
        &vendor(IMAGE),
        Some(image_sha256()),
    );
    let run = found.expect_redacted(run_program(&file, IBM_CAP), "running the test floppy");
    // Only the program's own COM1 output is printed, never the firmware's screen.
    eprintln!(
        "at-pm-test {IBM_ENTRY}: stop={:?} retired={}\n{}",
        run.outcome, run.retired, run.com1
    );
    assert_eq!(run.outcome, RunOutcome::Halted);
    assert_eq!(
        run.com1,
        expected_transcript(&INT15_AT, RESUMED, RESUMED, 0)
    );
    assert!(run.screen.contains("failures 0"));
    if let Some(expected) = IBM_GOLDEN_RETIRED {
        assert_eq!(run.retired, expected);
    }
}
