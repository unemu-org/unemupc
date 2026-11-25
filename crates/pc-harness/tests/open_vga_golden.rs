// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open VGA BIOS goldens on the open game profile: the INT 10h probe floppy,
//! mode 0Dh smooth-scroll scanout and mode 13h scanout driven through the
//! video BIOS.
//!
//! IBM VGA/XGA Technical Reference, Video BIOS (INT 10h AH=00h, 05h, 0Ch,
//! AX=1000h and AX=1010h), Graphics Modes (16-colour planar mode 0Dh and
//! 256-colour chain-4 mode 13h), CRTC Start Address, Attribute Controller
//! Horizontal PEL Panning and the default 256-colour DAC palette; see
//! firmware-and-machine-classes.md §Video ROM placement. CPU revision is
//! pinned by Cargo.lock.

use assembly::{
    AssemblyOptions, DeviceId, ObservationPolicy, ObservationRequest, OutputKind, SpeakerTraceEvent,
};
use cpu::{Cpu, CpuReg};
use machine::RunOutcome;

#[path = "support/fixtures.rs"]
mod fixtures;
#[allow(dead_code)]
#[path = "support/at_games.rs"]
mod support;

const CPU_REV: &str = "8064e28f1a9a2e4af7f1304a61e1eb1c9f9b13af";
const FETCH_SCRIPT: &str = include_str!("../../../vendor/vgabios/fetch.sh");

// Both counts moved with the switch from the Kiselev AT-reset BIOS to the
// open AT BIOS (from 10,239,957 and 10,730,615); screen and frames unchanged.
// Both moved again by one instruction (from 10,240,119 and 10,730,617) when the
// open profile gained its empty 1.44M B: drive; screen and frames unchanged.
// They moved again (from 10,240,118 and 10,730,616) when the open AT BIOS
// began clearing extended memory during POST; screen and frames unchanged.
// The video BIOS based on upstream 0.9d moved them again (from 10,240,139 and
// 10,730,175): the scroll run, whose frames are unchanged, by the new ROM
// alone; the probe also by its BEL check, whose 31/64 s tone the BIOS spends
// polling port 61h (about 356,000 instructions), and one more result line
// (it stopped at 880Ah before).
const PROBE_GOLDEN_STOP: (u64, u64) = (0x0000, 0x88A9);
const PROBE_GOLDEN_LINES: usize = 14;
const PROBE_GOLDEN_RETIRED: u64 = 10_524_099;
const PROBE_GOLDEN_TEXT_SHA256: &str =
    "df1f232b9cd4b8be1c849998ee11b844576611b05f9282a2094bbc70cb8a86d8";
// The mode 13h stage moved this from 10,396,703; the 0Dh frames are unchanged.
const SCROLL_GOLDEN_RETIRED: u64 = 10_688_267;
const SCROLL_FRAME_SHA256: [&str; 5] = [
    "7723e850306f8e31a0fce846b020d7ad4969c9f34093f037a807f33744d44f54",
    "4cd1269be99016a508dd720dd327b6a0cf86f51608954f05582cbf0e6d4ffad8",
    "453cbe69d394c3ab662a6998458f5e9765e35d2b106bce8af60542589129a327",
    "05f0b60dac7807db0725a015102fabe02a0e7ca78901b323d00f33027e27ae70",
    "58ed8b3f9f516112ef66c5e5c9eced93480371468711d294bb598b40a66b73ed",
];

/// The probe's checks, in the order it logs them; each must report "ok".
const PROBE_CHECKS: [&str; 33] = [
    "mode3",
    "dcc",
    "egainfo",
    "bda.mode",
    "bda.cols",
    "bda.rows",
    "bda.char",
    "font.plane2",
    "curshape",
    "curpos",
    "tty.cur",
    "tty.cell",
    "rdchar",
    "tty.bel",
    "scroll",
    "page1",
    "1C.save",
    "1C.rest",
    "1C.vectors",
    "1C.bda",
    "1C.dac",
    "1C.hw",
    "mode",
    "pixel",
    "planes",
    "pan.bios",
    "pan.port",
    "page1",
    "pixel.mem",
    "pixel.bios",
    "dac.rg",
    "dac.b",
    "dac.port",
];

fn policy() -> ObservationPolicy {
    ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::Vga,
            output: OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..ObservationPolicy::default()
    }
}

/// The shipped open profile with its real firmware and A: replaced by one
/// read-only image. Its FreeDOS catalogue and blank C: are never booted.
fn open_profile_booting(
    image: &str,
    bytes: &[u8],
    sha256: Option<&str>,
) -> Option<support::Profile> {
    let mut firmware = Vec::new();
    for relative in [
        "unemu-at-bios/bios-unemu-at.bin",
        "vgabios/vgabios-std.bin",
        "xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin",
    ] {
        firmware.push((relative, fixtures::fixture(relative)?));
    }
    let mut profile = support::Profile::new(support::OPEN, false);
    for (relative, bytes) in firmware {
        profile.write(&format!("vendor/{relative}"), &bytes);
    }
    profile.write(&format!("machines/local/{image}"), bytes);
    // A child's single image replaces the inherited A: media catalogue.
    let mut child = format!(
        "schema = 1\nextends = \"../{}\"\n\n[[card]]\nid = \"fdc-765\"\n\n\
         [[card.drives]]\nunit = 0\nimage = \"{image}\"\nwrite = \"readonly\"\n",
        support::OPEN
    );
    if let Some(sha256) = sha256 {
        child.push_str(&format!("sha256 = \"{sha256}\"\n"));
    }
    profile.write("machines/local/boot.toml", child.as_bytes());
    profile.file = profile.tree.path().join("machines/local/boot.toml");
    Some(profile)
}

fn screen_rows(assembled: &mut assembly::AssembledMachine) -> Vec<String> {
    let capture = assembled.capture_text(DeviceId::Vga).unwrap();
    let video::Frame::Text(grid) = &capture.frame else {
        panic!("expected VGA text");
    };
    grid.rows_as_strings()
}

/// A finished probe run. Fields drop in order, so the machine closes its
/// media before the profile's temporary files go.
struct ProbeRun {
    assembled: assembly::AssembledMachine,
    outcome: RunOutcome,
    rows: Vec<String>,
    _profile: support::Profile,
}

/// Boot the INT 10h probe floppy on the open profile until it halts; `None`
/// without the fixtures.
fn run_probe(policy: &ObservationPolicy) -> Option<ProbeRun> {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let probe = fixtures::fixture("vgabios/int10-probe-1200.img")?;
    let pin = fixtures::parse_pin(FETCH_SCRIPT, "PROBE_SHA256=\"").unwrap();
    let profile = open_profile_booting("int10-probe-1200.img", &probe, Some(pin))?;
    let loaded =
        machine_file::load_for_assembly(&profile.file, AssemblyOptions::default()).unwrap();
    let mut assembled = loaded.assemble(policy).unwrap();
    let outcome = assembled
        .run_session(&assembly::RunPolicy::natural(
            Some(100_000_000),
            Some(pc_harness::BIOS_IDLE_LIMIT),
        ))
        .unwrap();
    let rows = screen_rows(&mut assembled);
    Some(ProbeRun {
        assembled,
        outcome,
        rows,
        _profile: profile,
    })
}

/// The probe's checks that report "ok", in the order it logs them.
fn reported_ok(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|pair| pair[1] == "ok")
        .map(|pair| pair[0])
        .collect()
}

#[test]
#[ignore = "slow: open AT BIOS/VGA/XTIDE POST, then the INT 10h probe floppy"]
fn open_vga_bios_passes_the_int10_probe_floppy() {
    // The profile binds first, so it drops after the machine.
    let Some(ProbeRun {
        _profile,
        mut assembled,
        outcome,
        rows,
    }) = run_probe(&policy())
    else {
        return;
    };
    let text = rows.join("\n");
    let cpu = assembled.machine.cpu();
    let location = (cpu.code_segment(), cpu.reg(CpuReg::Rip));
    let retired = cpu.retired();
    let hash = assembly::test_support::text_plane_sha256(
        &assembled.capture_text(DeviceId::Vga).unwrap().frame,
    );
    eprintln!("int10 probe screen:\n{text}");
    eprintln!(
        "int10 probe: stop={outcome:?} cs={:04x} ip={:04x} retired={retired} text_sha256={hash}",
        location.0, location.1
    );
    // The probe halts with interrupts disabled after printing its log.
    assert_eq!(outcome, RunOutcome::Halted);
    let lines: Vec<_> = rows.iter().map(|row| row.trim_end()).collect();
    let used = lines.iter().rposition(|line| !line.is_empty()).unwrap() + 1;
    assert_eq!(used, PROBE_GOLDEN_LINES, "result screen lines:\n{text}");
    assert_eq!(lines[0], "vgabios INT 10h probe");
    assert_eq!(lines[used - 1], "fails:0000");
    assert!(!text.contains("FAIL"), "{text}");
    assert_eq!(reported_ok(&text), PROBE_CHECKS);
    assert_eq!(location, PROBE_GOLDEN_STOP);
    assert_eq!(retired, PROBE_GOLDEN_RETIRED);
    assert_eq!(hash, PROBE_GOLDEN_TEXT_SHA256);
    assembled.finish_persistence().unwrap();
}

/// The tone a teletype BEL gives: IBM Personal Computer AT Technical
/// Reference, System Board, Speaker (counter 2 sounds while port 61h bits 0
/// and 1 are set), and Intel 8254 datasheet, Mode 3 (square wave at the input
/// clock divided by the count). 1331 is the IBM PC BIOS beep's divisor, about
/// 896 Hz.
const BEL_MODE: u8 = 3;
const BEL_DIVISOR: u32 = 1331;
/// The video BIOS's own segment, where its port 61h writes run.
const VIDEO_BIOS_SEGMENT: u16 = 0xC000;
/// The video BIOS times the tone by counting port 61h bit 4 refresh toggles
/// (one per 18 PIT clocks), 32085 of them, for 31/64 s: 577,530 PIT clocks,
/// 417 short of 31/64 s. The run is deterministic, so the measured length is
/// pinned: 577,819 PIT clocks, 289 longer than the count. Interrupts stay
/// enabled during the tone; while the CPU serves a timer interrupt, bit 4 can
/// flip twice unseen, so the loop misses toggles and counts its 32085 late.
const BEL_GOLDEN_PIT_CLOCKS: u64 = 577_819;
/// The tone's nominal length, and how close the pinned one must stay to it.
const BEL_SECONDS: (u64, u64) = (31, 64);
const BEL_TOLERANCE_PERCENT: u64 = 1;

/// Teletype BEL (INT 10h AX=0E07h, which the probe's tty.bel check issues)
/// sounds the speaker once, from the video BIOS, for 31/64 s of virtual time.
/// The headless speaker trace sees what the window's trace would print.
#[test]
#[ignore = "slow: open AT BIOS/VGA/XTIDE POST, then the INT 10h probe floppy"]
fn open_vga_bios_teletype_bel_sounds_the_speaker_for_31_64_seconds() {
    let policy = ObservationPolicy {
        speaker_trace: true,
        ..policy()
    };
    // The profile binds first, so it drops after the machine.
    let Some(ProbeRun {
        _profile,
        mut assembled,
        outcome,
        rows,
    }) = run_probe(&policy)
    else {
        return;
    };
    let events = assembled
        .observation
        .take_speaker_trace(assembled.machine.clock());
    for event in &events {
        eprintln!("{event}");
    }
    assert_eq!(outcome, RunOutcome::Halted);
    let text = rows.join("\n");
    assert!(reported_ok(&text).contains(&"tty.bel"), "{text}");
    // Every line was kept, so the counts below see every speaker event.
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SpeakerTraceEvent::Suppressed { .. })),
        "{events:#?}"
    );
    let speaker: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                SpeakerTraceEvent::On { .. }
                    | SpeakerTraceEvent::Tone { .. }
                    | SpeakerTraceEvent::Off { .. }
            )
        })
        .collect();
    let bel: Vec<_> = speaker
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            matches!(event, SpeakerTraceEvent::On { divisor, .. } if *divisor == BEL_DIVISOR)
        })
        .collect();
    assert_eq!(bel.len(), 1, "one BEL tone: {speaker:#?}");
    // The open AT BIOS plays its power-on melody from F000h; the video BIOS
    // sounds nothing but the BEL.
    let from_video_bios = speaker
        .iter()
        .filter(|event| {
            matches!(event, SpeakerTraceEvent::On { stamp, .. } if stamp.cs == VIDEO_BIOS_SEGMENT)
        })
        .count();
    assert_eq!(from_video_bios, 1, "{speaker:#?}");
    let (at, on) = bel[0];
    let SpeakerTraceEvent::On { stamp, mode, .. } = on else {
        unreachable!()
    };
    assert_eq!(*mode, BEL_MODE);
    assert_eq!(stamp.cs, VIDEO_BIOS_SEGMENT);
    // The tone neither changes nor stops before the BIOS turns it off.
    let Some(SpeakerTraceEvent::Off { stamp: off, after }) = speaker.get(at + 1) else {
        panic!("the BEL tone never stopped: {speaker:#?}");
    };
    assert_eq!(off.cs, VIDEO_BIOS_SEGMENT);
    let (numerator, denominator) = BEL_SECONDS;
    let pit = device::clock::PIT_HZ;
    let expected = u128::from(numerator)
        * u128::from(pit.numerator())
        * u128::from(device::clock::TICKS_PER_PIT_CLOCK)
        / (u128::from(denominator) * u128::from(pit.denominator()));
    let tolerance = expected * u128::from(BEL_TOLERANCE_PERCENT) / 100;
    eprintln!("BEL tone: {after} ticks, expected {expected} +/- {tolerance}");
    assert!(
        u128::from(*after).abs_diff(expected) <= tolerance,
        "BEL tone lasted {after} ticks, expected {expected} +/- {tolerance}"
    );
    assert_eq!(
        *after,
        BEL_GOLDEN_PIT_CLOCKS * device::clock::TICKS_PER_PIT_CLOCK
    );
    assembled.finish_persistence().unwrap();
}

const STAGE_MAGIC: [u8; 4] = *b"M0D!";
/// Default attribute palette from the video BIOS's mode 0Dh parameter table
/// (its vgatables.h): colours 8–15 use DAC entries 10h–17h.
const MODE_0D_PALETTE: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
];
/// Bytes per plane in one 320×200 page.
const PAGE_BYTES: usize = 8000;
/// Page size in the video BIOS's mode 0Dh table, used by INT 10h AH=05h.
const PAGE_STRIDE: u16 = 0x2000;
/// Per mode 0Dh stage: the CRTC start address in bytes and the PEL panning
/// value. The last stage switches to mode 13h.
const STAGES: [(u16, u8); 4] = [(0, 0), (0, 3), (PAGE_STRIDE, 0), (1, 5)];
const MODE_13_STAGE: u8 = 5;
/// The mode 13h stage's INT 10h AH=0Ch pixel, and the DAC entry it sets with
/// AX=1010h, as 6-bit red, green and blue.
const MODE_13_PIXEL: (usize, usize, u8) = (10, 10, 0x55);
const MODE_13_DAC: [u8; 3] = [0x3F, 0x00, 0x1F];

fn fill_seed(page: usize, plane: u8) -> (u8, u8) {
    if page == 0 {
        (0x0F ^ plane.wrapping_mul(0x35), 2 * plane + 1)
    } else {
        (
            0xC3_u8.wrapping_add(plane.wrapping_mul(0x1D)),
            0xFF - 2 * plane,
        )
    }
}

fn bios_pixel_colour(x: usize) -> u8 {
    ((x + 1) & 15) as u8
}

fn wait_retrace(program: &mut Vec<u8>) {
    program.extend([
        0xBA, 0xDA, 0x03, 0xEC, 0xA8, 0x08, 0x75, 0xFB, // leave the current retrace
        0xEC, 0xA8, 0x08, 0x74, 0xFB, // the start address latches at retrace start
        0xEC, 0xA8, 0x08, 0x75, 0xFB, // PEL panning latches at retrace end
    ]);
}

fn mark_stage(program: &mut Vec<u8>, stage: u8) {
    wait_retrace(program);
    for (index, word) in STAGE_MAGIC.as_chunks::<2>().0.iter().enumerate() {
        program.extend([0xC7, 0x06, index as u8 * 2, 0x06, word[0], word[1]]);
    }
    program.extend([0xC6, 0x06, 0x04, 0x06, stage]);
    // Hold the frame for several refreshes so the host's poll samples it.
    program.extend([0xB9, 0xFF, 0xFF, 0xE2, 0xFE]);
}

fn set_panning(program: &mut Vec<u8>, pan: u8) {
    // INT 10h AX=1000h: set attribute register BL=13h to BH.
    program.extend([0xB8, 0x00, 0x10, 0xBB, 0x13, pan, 0xCD, 0x10]);
}

/// A boot sector in the INT 10h probe's style: mode 0Dh through the BIOS, two
/// pages of planar data, BIOS pixels, then panning and page changes.
fn mode_0d_scroll_floppy() -> Vec<u8> {
    let mut program = vec![
        0xFA, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, 0xBC, 0x00, 0x7C, 0xFB, 0xFC, // stack, DS=0
        0xB8, 0x0D, 0x00, 0xCD, 0x10, // INT 10h AX=000Dh
        0xB8, 0x00, 0xA0, 0x8E, 0xC0, // ES=A000h
    ];
    for page in 0..2 {
        for plane in 0..4u8 {
            let (seed, step) = fill_seed(page, plane);
            let base = page as u16 * PAGE_STRIDE;
            let [base_low, base_high] = base.to_le_bytes();
            let [count_low, count_high] = (PAGE_BYTES as u16).to_le_bytes();
            program.extend([0xBA, 0xC4, 0x03, 0xB8, 0x02, 1 << plane, 0xEF]); // map mask
            program.extend([0xBF, base_low, base_high, 0xB9, count_low, count_high]);
            program.extend([0xB0, seed, 0xAA, 0x04, step, 0xE2, 0xFB]); // stosb; add al
        }
    }
    // Map mask back to all planes, then INT 10h AH=0Ch: colours 1..15, 0 at
    // x=0..15 on the first row.
    program.extend([0xB8, 0x02, 0x0F, 0xEF]);
    program.extend([0x31, 0xC9, 0x31, 0xD2]);
    let pixel_loop = program.len();
    program.extend([
        0x88, 0xC8, 0xFE, 0xC0, 0x24, 0x0F, 0xB4, 0x0C, 0x30, 0xFF, 0x51, 0x52, 0xCD, 0x10, 0x5A,
        0x59, 0x41, 0x83, 0xF9, 0x10, 0x72,
    ]);
    program.push((pixel_loop as isize - program.len() as isize - 1) as u8);
    mark_stage(&mut program, 1);
    set_panning(&mut program, 3);
    mark_stage(&mut program, 2);
    set_panning(&mut program, 0);
    program.extend([0xB8, 0x01, 0x05, 0xCD, 0x10]); // INT 10h AX=0501h: page 1
    mark_stage(&mut program, 3);
    // Byte-granular start address through the CRTC ports, then 5 more pixels.
    program.extend([
        0xBA, 0xD4, 0x03, 0xB8, 0x0C, 0x00, 0xEF, 0xB8, 0x0D, 0x01, 0xEF,
    ]);
    set_panning(&mut program, 5);
    mark_stage(&mut program, 4);
    // INT 10h AX=0013h, then byte i = (i mod 256) XOR (i / 256) through the
    // chain-4 window, one BIOS pixel and one BIOS DAC entry.
    program.extend([0xB8, 0x13, 0x00, 0xCD, 0x10, 0xB8, 0x00, 0xA0, 0x8E, 0xC0]);
    program.extend([
        0x31, 0xFF, 0xB9, 0x00, 0xFA, 0x89, 0xF8, 0x30, 0xE0, 0xAA, 0xE2, 0xF9,
    ]);
    let (x, y, colour) = MODE_13_PIXEL;
    program.extend([
        0xB8, colour, 0x0C, 0x30, 0xFF, 0xB9, x as u8, 0x00, 0xBA, y as u8, 0x00,
    ]);
    program.extend([0xCD, 0x10]);
    let [red, green, blue] = MODE_13_DAC;
    program.extend([
        0xB8, 0x10, 0x10, 0xBB, colour, 0x00, 0xB6, red, 0xB5, green, 0xB1, blue,
    ]);
    program.extend([0xCD, 0x10]);
    mark_stage(&mut program, MODE_13_STAGE);
    program.extend([0xFA, 0xF4]);
    assert!(
        program.len() <= 510,
        "boot program is {} bytes",
        program.len()
    );
    let mut disk = vec![0; assembly::FloppyGeometry::HD_1_2.image_bytes() as usize];
    disk[..program.len()].copy_from_slice(&program);
    disk[510..512].copy_from_slice(&[0x55, 0xAA]);
    disk
}

/// Video memory as the boot program leaves it, one 64 KiB array per plane.
fn expected_planes() -> [Vec<u8>; 4] {
    let mut planes: [Vec<u8>; 4] = std::array::from_fn(|_| vec![0; 0x10000]);
    for page in 0..2 {
        for (plane, memory) in planes.iter_mut().enumerate() {
            let (seed, step) = fill_seed(page, plane as u8);
            let base = page * usize::from(PAGE_STRIDE);
            for (offset, byte) in memory[base..base + PAGE_BYTES].iter_mut().enumerate() {
                *byte = seed.wrapping_add((offset as u8).wrapping_mul(step));
            }
        }
    }
    for x in 0..16 {
        let mask = 0x80 >> (x % 8);
        for (plane, memory) in planes.iter_mut().enumerate() {
            if bios_pixel_colour(x) >> plane & 1 == 1 {
                memory[x / 8] |= mask;
            } else {
                memory[x / 8] &= !mask;
            }
        }
    }
    planes
}

/// IBM VGA/XGA Technical Reference, Graphics Modes: each 40-byte row starts
/// at the CRTC start address; PEL panning shifts the row's pixel stream left.
fn expected_indices(planes: &[Vec<u8>; 4], start: u16, pan: u8) -> Vec<u8> {
    let mut indices = Vec::with_capacity(320 * 200);
    for y in 0..200 {
        for x in 0..320 {
            let dot = x + usize::from(pan);
            let address = (usize::from(start) + y * 40 + dot / 8) & 0xFFFF;
            let bit = 7 - dot % 8;
            let colour = (0..4).fold(0, |colour, plane| {
                colour | ((planes[plane][address] >> bit) & 1) << plane
            });
            indices.push(MODE_0D_PALETTE[usize::from(colour)]);
        }
    }
    indices
}

/// The video BIOS's mode 13h DAC load, the IBM VGA default 256-colour
/// palette: the 16 EGA colours, 16 grey levels, nine 24-step hue wheels
/// (three intensities at three saturations) and eight black entries.
fn default_dac_palette() -> Vec<[u8; 3]> {
    let mut palette = vec![
        [0x00, 0x00, 0x00],
        [0x00, 0x00, 0x2A],
        [0x00, 0x2A, 0x00],
        [0x00, 0x2A, 0x2A],
        [0x2A, 0x00, 0x00],
        [0x2A, 0x00, 0x2A],
        [0x2A, 0x15, 0x00],
        [0x2A, 0x2A, 0x2A],
        [0x15, 0x15, 0x15],
        [0x15, 0x15, 0x3F],
        [0x15, 0x3F, 0x15],
        [0x15, 0x3F, 0x3F],
        [0x3F, 0x15, 0x15],
        [0x3F, 0x15, 0x3F],
        [0x3F, 0x3F, 0x15],
        [0x3F, 0x3F, 0x3F],
    ];
    for grey in [
        0x00, 0x05, 0x08, 0x0B, 0x0E, 0x11, 0x14, 0x18, 0x1C, 0x20, 0x24, 0x28, 0x2D, 0x32, 0x38,
        0x3F,
    ] {
        palette.push([grey; 3]);
    }
    for levels in [
        [0x00, 0x10, 0x1F, 0x2F, 0x3F],
        [0x1F, 0x27, 0x2F, 0x37, 0x3F],
        [0x2D, 0x31, 0x36, 0x3A, 0x3F],
        [0x00, 0x07, 0x0E, 0x15, 0x1C],
        [0x0E, 0x11, 0x15, 0x18, 0x1C],
        [0x14, 0x16, 0x18, 0x1A, 0x1C],
        [0x00, 0x04, 0x08, 0x0C, 0x10],
        [0x08, 0x0A, 0x0C, 0x0E, 0x10],
        [0x0B, 0x0C, 0x0D, 0x0F, 0x10],
    ] {
        let (low, high) = (levels[0], levels[4]);
        // Blue to magenta to red to yellow to green to cyan, four steps each:
        // one channel rises from low, or falls from high.
        let rising = &levels[..4];
        let falling: Vec<u8> = levels[1..].iter().rev().copied().collect();
        palette.extend(rising.iter().map(|&level| [level, low, high]));
        palette.extend(falling.iter().map(|&level| [high, low, level]));
        palette.extend(rising.iter().map(|&level| [high, level, low]));
        palette.extend(falling.iter().map(|&level| [level, high, low]));
        palette.extend(rising.iter().map(|&level| [low, high, level]));
        palette.extend(falling.iter().map(|&level| [low, level, high]));
    }
    palette.resize(256, [0; 3]);
    palette
}

/// The DAC's 6-bit levels as RGB888, replicating the top bits as the
/// capture does.
fn rgb888(entry: [u8; 3]) -> [u8; 3] {
    entry.map(|level| (level << 2) | (level >> 4))
}

/// IBM VGA/XGA Technical Reference, 256-Color Graphics Mode: chain-4 maps
/// CPU byte i to plane i mod 4, and doubleword scanout reads the planes back
/// in CPU byte order, so pixel i is CPU byte i. The BIOS's mode 13h attribute
/// palette is the identity, so each byte is its DAC index.
fn expected_mode_13() -> (Vec<u8>, Vec<[u8; 3]>) {
    let mut indices: Vec<u8> = (0..64000_usize)
        .map(|offset| (offset as u8) ^ ((offset >> 8) as u8))
        .collect();
    let (x, y, colour) = MODE_13_PIXEL;
    indices[y * 320 + x] = colour;
    let mut palette = default_dac_palette();
    palette[usize::from(colour)] = MODE_13_DAC;
    (indices, palette.into_iter().map(rgb888).collect())
}

fn boot_stage(cpu: &cpu::X86JitCpu) -> Option<u8> {
    // POST memory tests can leave arbitrary bytes at 0600h: accept a marker
    // only while the boot sector itself is executing.
    if cpu.code_segment() != 0 || !(0x7C00..0x7E00).contains(&cpu.reg(CpuReg::Rip)) {
        return None;
    }
    let mut marker = [0; 5];
    cpu.read_mem(0x600, &mut marker).unwrap();
    (marker[..4] == STAGE_MAGIC).then_some(marker[4])
}

#[test]
fn scanout_models_shift_the_pixel_stream_and_build_the_bios_palette() {
    let planes = expected_planes();
    let base = expected_indices(&planes, 0, 0);
    assert_eq!(
        &base[..16],
        &(0..16)
            .map(|x| MODE_0D_PALETTE[usize::from(bios_pixel_colour(x))])
            .collect::<Vec<_>>()[..]
    );
    for (start, pan) in [(0, 3), (1, 5)] {
        let shift = usize::from(start) * 8 + usize::from(pan);
        let shifted = expected_indices(&planes, start, pan);
        assert_eq!(shifted[..64000 - shift], base[shift..]);
    }
    assert_ne!(expected_indices(&planes, PAGE_STRIDE, 0), base);
    assert_eq!(&mode_0d_scroll_floppy()[510..512], &[0x55, 0xAA]);
    let palette = default_dac_palette();
    assert_eq!(palette.len(), 256);
    assert_eq!(palette[32], [0x00, 0x00, 0x3F]);
    assert_eq!(palette[247], [0x0B, 0x0C, 0x10]);
    assert_eq!(rgb888([0x3F, 0x15, 0x00]), [0xFF, 0x55, 0x00]);
}

#[test]
#[ignore = "slow: open AT BIOS/VGA/XTIDE POST, then a mode 0Dh/13h scanout boot sector"]
fn open_vga_bios_mode_0d_scrolling_and_mode_13h_reach_scanout() {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let Some(profile) = open_profile_booting("m0d-scroll-1200.img", &mode_0d_scroll_floppy(), None)
    else {
        return;
    };
    let loaded =
        machine_file::load_for_assembly(&profile.file, AssemblyOptions::default()).unwrap();
    let mut assembled = loaded.assemble(&policy()).unwrap();
    let planes = expected_planes();
    let mut hashes = Vec::new();
    let mut outcome = RunOutcome::RetiredCapReached;
    let cap = 100_000_000;
    while assembled.machine.virtual_time() < cap {
        let next = assembled.machine.virtual_time() + 5_000;
        outcome = assembled
            .run_session(&assembly::RunPolicy::natural(
                Some(next),
                Some(pc_harness::BIOS_IDLE_LIMIT),
            ))
            .unwrap();
        let stage = boot_stage(assembled.machine.cpu());
        if let Some(stage) = stage.filter(|stage| usize::from(*stage) == hashes.len() + 1) {
            let capture = assembled.capture_text(DeviceId::Vga).unwrap();
            let video::Frame::Pixels(frame) = &capture.frame else {
                panic!("graphics modes must capture pixels at stage {stage}");
            };
            assert_eq!(
                (frame.width, frame.height, frame.repeat),
                (320, 200, (2, 2))
            );
            if stage == MODE_13_STAGE {
                let (indices, palette) = expected_mode_13();
                assert!(
                    frame.indices == indices,
                    "mode 13h differs from the chain-4 byte stream"
                );
                assert_eq!(&frame.palette[..], &palette[..], "mode 13h DAC palette");
            } else {
                let (start, pan) = STAGES[usize::from(stage) - 1];
                assert!(
                    frame.indices == expected_indices(&planes, start, pan),
                    "stage {stage}: start={start:#06x} pan={pan} differs from planar memory"
                );
            }
            hashes.push(assembly::test_support::graphics_frame_sha256(
                &capture.frame,
            ));
        }
        if outcome != RunOutcome::RetiredCapReached {
            break;
        }
    }
    let retired = assembled.machine.cpu().retired();
    eprintln!("mode 0Dh scroll: stop={outcome:?} retired={retired} frames={hashes:#?}");
    assert_eq!(outcome, RunOutcome::Halted);
    assert_eq!(boot_stage(assembled.machine.cpu()), Some(MODE_13_STAGE));
    assert_eq!(hashes, SCROLL_FRAME_SHA256);
    assert_eq!(retired, SCROLL_GOLDEN_RETIRED);
    assembled.finish_persistence().unwrap();
}
