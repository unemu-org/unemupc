// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open unchained 360x240 VGA golden, booted through the open AT and VGA BIOSes.
//! IBM VGA/XGA Technical Reference Manual (May 1992), Figs. 2-25, 2-32,
//! 2-36, 2-60, 2-61, 2-64, 2-65 and 2-75: retrace, plane selection,
//! display addressing and write mode 1. See design/vga-modex-test.md.

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

const CPU_REV: &str = "b7c10b6fdda8f84d3eda9c5d7751fcda1c5893ed";
const IMAGE: &str = "vga-modex-test/vga-modex-test-1200.img";
const PAS_IMAGE: &str = "vga-modex-test/vga-pas-test-1200.img";
const OPEN_BIOS: &str = "unemu-at-bios/bios-unemu-at.bin";
const OPEN_BIOS_SHA256: &str = "6938dc8b665fa120f33775163ab6bacc559389525db87adedf734416f9ef2442";
const VGA_BIOS: &str = "vgabios/vgabios-std.bin";
const VGA_BIOS_SHA256: &str = "b867f3846f44be63b1b10febc1ce79c9435f5d1233c67a0b63164ab48359a582";
const CAP: u64 = 100_000_000;

fn vendor(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor")
        .join(relative)
}

fn image_sha256() -> &'static str {
    fixtures::parse_pin(
        include_str!("../../../vendor/vga-modex-test/build.sh"),
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

fn machine_file(directory: &Path, image: &str, image_hash: &str) -> PathBuf {
    let text = format!(
        "schema = 1\nextends = \"preset:ibm-5170\"\nname = \"vga-modex-test\"\n\
         [board]\ndisplay_switch = \"colour\"\n\
         [board.rtc]\nstart = \"2000-01-01T00:00:00\"\nnvram = \"derived\"\n\
         [cpu]\nfpu = \"none\"\n\
         [memory]\nconventional_kib = 640\nextended_kib = 1024\n\
         [firmware.bios]\nprovides = [\"floppy\"]\nimage = {}\nsha256 = \"{OPEN_BIOS_SHA256}\"\n\
         [[firmware.option_roms]]\nimage = {}\nsha256 = \"{VGA_BIOS_SHA256}\"\nbase = 0xC0000\n\
         [[card]]\nid = \"mda\"\nremove = true\n\
         [[card]]\nid = \"vga-text\"\nremove = true\n\
         [[card]]\nkind = \"vga\"\n\
         [[card]]\nid = \"uart-16550\"\nport = 0x3F8\n\
         [[card]]\nid = \"fdc-765\"\n\
         [[card.drives]]\nunit = 0\ntype = \"1.2M\"\nimage = {}\nwrite = \"readonly\"\n\
         sha256 = \"{}\"\n",
        toml_string(&vendor(OPEN_BIOS)),
        toml_string(&vendor(VGA_BIOS)),
        toml_string(&vendor(image)),
        image_hash
    );
    let path = directory.join("vga-modex-test.toml");
    std::fs::write(&path, text).unwrap();
    path
}

// This is a logical pixel definition, without VGA addresses or model helpers.
fn expected_pixel(page: u8, mut x: usize, mut y: usize) -> u8 {
    if (224..288).contains(&x) && (112..144).contains(&y) {
        x -= 208;
        y -= 88;
    }
    if y >= 200 {
        if x % 8 < 5 && y % 8 < 7 && (x.is_multiple_of(8) || matches!(y % 8, 0 | 3 | 6)) {
            0xF1
        } else {
            2 + page * 64
        }
    } else {
        ((x ^ y) & 63) as u8 + page * 64
    }
}

fn compare_frame(frame: &video::IndexedPixels, page: u8) -> Result<(), String> {
    if (frame.width, frame.height, frame.repeat) != (360, 240, (2, 2)) {
        return Err(format!(
            "geometry {:?}, expected 360x240 repeat (2, 2)",
            (frame.width, frame.height, frame.repeat)
        ));
    }
    let mut count = 0;
    let mut residues = [0; 4];
    let mut first = Vec::new();
    for y in 0..240 {
        for x in 0..360 {
            let expected = expected_pixel(page, x, y);
            let actual = frame.indices[y * 360 + x];
            if actual != expected {
                count += 1;
                residues[x % 4] += 1;
                if first.len() < 8 {
                    first.push(format!(
                        "({x},{y}): got {actual:02x}, expected {expected:02x}"
                    ));
                }
            }
        }
    }
    if count == 0 {
        Ok(())
    } else {
        Err(format!(
            "page {page}: {count}/86400 pixels differ, counts by x mod 4 {residues:?}; {}",
            first.join("; ")
        ))
    }
}

#[test]
#[ignore = "slow golden: open AT/VGA BIOS POST, then 60 Mode X retraces and flips"]
fn open_bios_modex_pixels_latch_copies_and_sixty_retrace_flips() {
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    for relative in [OPEN_BIOS, VGA_BIOS, IMAGE] {
        if fixtures::fixture(relative).is_none() {
            return;
        }
    }
    let temporary = TempDir::new("vga-modex-test");
    let file = machine_file(temporary.path(), IMAGE, image_sha256());
    let loaded = machine_file::load_for_assembly(&file, AssemblyOptions::default()).unwrap();
    let observation = ObservationPolicy {
        routes: vec![
            ObservationRequest {
                device: DeviceId::Uart,
                output: OutputKind::Bytes(ByteStreamKind::SerialTx),
                byte_limit: 65536,
            },
            ObservationRequest {
                device: DeviceId::Vga,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            },
        ],
        ..Default::default()
    };
    let mut assembled = loaded.assemble(&observation).unwrap();
    let mut transcript = String::new();
    let mut checked = [0; 2];
    let mut failure = None;
    let mut reported = 0;
    let mut outcome = RunOutcome::RetiredCapReached;
    while assembled.machine.virtual_time() < CAP {
        let next = (assembled.machine.virtual_time() + 5000).min(CAP);
        outcome = assembled
            .run_session(&RunPolicy::natural(
                Some(next),
                Some(pc_harness::BIOS_IDLE_LIMIT),
            ))
            .unwrap();
        let bytes = assembled
            .observation
            .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
            .unwrap();
        transcript.push_str(std::str::from_utf8(&bytes).unwrap());
        if let Some((_, progress)) = transcript.split_once("pattern ready\r\n") {
            let frames = progress
                .bytes()
                .take_while(|b| matches!(b, b'0' | b'1'))
                .count();
            if frames > reported {
                assert_eq!(frames, reported + 1, "poll skipped a frame");
                let page = ((frames - 1) & 1) as u8;
                let video::Frame::Pixels(frame) =
                    assembled.capture_text(DeviceId::Vga).unwrap().frame
                else {
                    panic!("expected graphics at frame {frames}");
                };
                if let Err(error) = compare_frame(&frame, page) {
                    failure.get_or_insert_with(|| format!("frame {frames}: {error}"));
                }
                checked[usize::from(page)] += 1;
                reported = frames;
            }
        }
        if outcome != RunOutcome::RetiredCapReached {
            break;
        }
    }
    eprintln!(
        "vga-modex-test: stop={outcome:?}, retired={}, checked={checked:?}\n{transcript}",
        assembled.machine.cpu().retired()
    );
    assembled.finish_persistence().unwrap();
    assert_eq!(
        outcome,
        RunOutcome::Halted,
        "guest did not finish: {failure:?}"
    );
    assert_eq!(
        transcript,
        format!(
            "vga-modex-test\r\npattern ready\r\n{}\r\nframes 60 done\r\n",
            "01".repeat(30)
        )
    );
    assert_eq!(checked, [30, 30]);
    assert!(failure.is_none(), "{}", failure.unwrap_or_default());
}

#[test]
#[ignore = "slow golden: open AT/VGA BIOS POST, then mode 12h PAS=1 palette writes"]
fn open_bios_mode_twelve_palette_writes_with_pas_set_show_sixteen_colours() {
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    for relative in [OPEN_BIOS, VGA_BIOS, PAS_IMAGE] {
        if fixtures::fixture(relative).is_none() {
            return;
        }
    }
    let image_hash = fixtures::parse_pin(
        include_str!("../../../vendor/vga-modex-test/build.sh"),
        "check_sha256 \"$PAS_IMAGE\" \"",
    )
    .unwrap();
    let temporary = TempDir::new("vga-pas-test");
    let file = machine_file(temporary.path(), PAS_IMAGE, image_hash);
    let loaded = machine_file::load_for_assembly(&file, AssemblyOptions::default()).unwrap();
    let observation = ObservationPolicy {
        routes: vec![
            ObservationRequest {
                device: DeviceId::Uart,
                output: OutputKind::Bytes(ByteStreamKind::SerialTx),
                byte_limit: 65536,
            },
            ObservationRequest {
                device: DeviceId::Vga,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            },
        ],
        ..Default::default()
    };
    let mut assembled = loaded.assemble(&observation).unwrap();
    let outcome = assembled
        .run_session(&RunPolicy::natural(
            Some(CAP),
            Some(pc_harness::BIOS_IDLE_LIMIT),
        ))
        .unwrap();
    let transcript = assembled
        .observation
        .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
        .unwrap();
    let video::Frame::Pixels(frame) = assembled.capture_text(DeviceId::Vga).unwrap().frame else {
        panic!("expected mode 12h graphics");
    };
    eprintln!(
        "vga-pas-test: stop={outcome:?}, retired={}",
        assembled.machine.cpu().retired()
    );
    assembled.finish_persistence().unwrap();
    assert_eq!(outcome, RunOutcome::Halted);
    assert_eq!(transcript, b"vga-pas-test: bars ready\r\n");
    assert_eq!(
        (frame.width, frame.height, frame.repeat),
        (640, 480, (1, 1))
    );
    assert_eq!(frame.indices.len(), 640 * 480);
    // Logical bars and RGB values are independent of VGA address/scanout helpers.
    for y in 0..480 {
        for x in 0..640 {
            let colour = (x / 40) as u8;
            let expected =
                [colour, 63 - colour, colour ^ 21].map(|component| component * 4 + component / 16);
            let index = frame.indices[y * 640 + x];
            let actual = frame.palette[usize::from(index)];
            assert_eq!(
                actual, expected,
                "displayed RGB at ({x},{y}), index {index:02x}"
            );
        }
    }
}
