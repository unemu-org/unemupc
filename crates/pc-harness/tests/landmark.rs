// SPDX-License-Identifier: AGPL-3.0-or-later
//! User-supplied Landmark AT diagnostics and IBM 5170 FreeDOS boot measurements.
//!
//! Landmark Research International, ROM POST User's Manual v1.2, pp. 16–24:
//! the first twelve tests precede the protected-mode test. IBM Personal
//! Computer Technical Reference, Monochrome Display and Printer Adapter,
//! Programming Considerations: text cells hold glyph/attribute byte pairs.
//! See `backlog/docs/design/compatibility-diagnostics.md` §2–3.

use cpu::{Cpu, CpuError, CpuReg};
use machine::{MachineError, RunOutcome};
use video::{Frame, TextGrid};

use assembly::{
    AssemblyOptions, BoardModel, ConsoleProbe, DeviceId, ImageSource, InjectionSchedule,
    KeyboardStimulus, ObservationPolicy, ObservationRequest, OutputKind, RunError, RunPolicy,
    StopCondition,
};
use machine_file::test_support::FoundUserFirmware;
use machine_file::test_support::TempDir;
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

const ENTRY_ID: &str = "landmark-rom-post-at-1.2.0";
const CPU_REV: &str = "b7c10b6fdda8f84d3eda9c5d7751fcda1c5893ed";
const STEP_BUDGET: u64 = 30_000_000;
// Historical Application Real16 prefix after the PIT refresh calibration.
const REAL16_GOLDEN_RETIRED: u64 = 15_785_137;
const REAL16_GOLDEN_TEXT_SHA256: &str =
    "373ae9a68d11ea20d8da2232d39a90cdbb1b6f573b9fb28ea04e3ab79d3b31d1";
// TODO-MEASURE: supervisor supplies the new I286 stop, count and text hash.
const GOLDEN_RETIRED: Option<u64> = None;
const GOLDEN_TEXT_SHA256: Option<&str> = None;
const GOLDEN_STOP: Option<(u16, u64)> = None;
/// One diagnostic pass completes before this cap. The ROM counts the two CPU
/// resets used to return from its protected-mode tests as errors. Its error
/// counter and failure title reflect its own reset handling, not an emulator
/// fault. This also occurs on real IBM 5170 hardware; see minuszerodegrees,
/// Supersoft/Landmark IBM 5170 notes.
const FULL_PASS_RETIRED_CAP: u64 = 150_000_000;
const FULL_PASS_GOLDEN_STOP_KIND: Option<RunOutcome> = Some(RunOutcome::RetiredCapReached);
const FULL_PASS_GOLDEN_STOP: Option<(u16, u64)> = Some((0x0004, 0x0015));
const FULL_PASS_GOLDEN_RETIRED: Option<u64> = Some(138_208_231);
const FULL_PASS_GOLDEN_TEXT_SHA256: Option<&str> =
    Some("fe45632b8a9c1b762af117b162f56adc523b8aa2e5bda990bad5adb7079e807a");

const ROW_IDS: [&str; 12] = [
    "cpu",
    "rom_a",
    "rom_b",
    "pit_ch0",
    "pit_ch1",
    "pit_ch2",
    "dma1",
    "dma2",
    "dma_page",
    "kbc_parity",
    "mem_16k",
    "mem_refresh",
];

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Passed,
    Failed,
    NotPresent,
    Empty,
    Unrecognized,
}

fn verdict(status: &[u8]) -> Verdict {
    let start = status.iter().position(|byte| !matches!(byte, 0 | b' '));
    let end = status.iter().rposition(|byte| !matches!(byte, 0 | b' '));
    match start.zip(end).map(|(start, end)| &status[start..=end]) {
        None => Verdict::Empty,
        Some(b"PASSED") => Verdict::Passed,
        Some(b"FAILED") => Verdict::Failed,
        Some(b"NOT PRESENT") => Verdict::NotPresent,
        Some(_) => Verdict::Unrecognized,
    }
}

fn row_verdict(grid: &TextGrid, row: u16) -> Verdict {
    // Fixed status columns classify verdicts without reading proprietary row labels.
    let status: Vec<_> = (35..47)
        .map(|col| grid.cell(col, row).expect("status cell in MDA grid").glyph)
        .collect();
    verdict(&status)
}

fn run_result_summary(result: &Result<RunOutcome, RunError>) -> String {
    // Free-form backend diagnostics may contain byte dumps. Typed errors retain
    // their variant and address fields; no guest memory is read for this report.
    match result {
        Err(RunError::Machine(MachineError::Cpu(error))) => match error {
            CpuError::UnsupportedInstruction { addr, .. } => {
                format!("Err(Machine(Cpu(UnsupportedInstruction {{ addr: {addr:#x} }})))")
            }
            CpuError::Unmapped { addr } => {
                format!("Err(Machine(Cpu(Unmapped {{ addr: {addr:#x} }})))")
            }
            CpuError::Fault {
                vector,
                error_code,
                cs,
                ip,
            } => {
                format!("Err(Machine(Cpu(Fault {{ vector: {vector}, error_code: {error_code:?}, cs: {cs:#x}, ip: {ip:#x} }})))")
            }
            CpuError::UnsupportedSystem { operation, cs, ip } => {
                format!("Err(Machine(Cpu(UnsupportedSystem {{ operation: {operation}, cs: {cs:#x}, ip: {ip:#x} }})))")
            }
            CpuError::DeliveryStalled { deliveries, cs, ip } => {
                format!("Err(Machine(Cpu(DeliveryStalled {{ deliveries: {deliveries}, cs: {cs:#x}, ip: {ip:#x} }})))")
            }
            CpuError::ResetStorm { resets, cs, ip, .. } => {
                format!("Err(Machine(Cpu(ResetStorm {{ resets: {resets}, cs: {cs:#x}, ip: {ip:#x} }})))")
            }
            CpuError::Memory(_) => "Err(Machine(Cpu(Memory(<details omitted>))))".into(),
            CpuError::UnexpectedExit { .. } => {
                "Err(Machine(Cpu(UnexpectedExit { <details omitted> })))".into()
            }
        },
        Err(RunError::Persistence(_)) => "Err(Persistence(<details omitted>))".into(),
        Err(RunError::ConsoleUnavailable { device, .. }) => {
            format!("Err(ConsoleUnavailable {{ device: {device:?} }})")
        }
        _ => format!("{result:?}"),
    }
}

fn assert_synthetic_ud(
    assembled: &assembly::AssembledMachine,
    result: Result<RunOutcome, RunError>,
) {
    let cpu = assembled.machine.cpu();
    let mut frame = [0; 4];
    cpu.read_mem(cpu.reg(CpuReg::Rsp), &mut frame).unwrap();
    assert!(
        matches!(result, Ok(RunOutcome::Halted))
            && cpu.code_segment() == 0xF000
            && cpu.reg(CpuReg::Rip) == 0xA401
            && frame == [0xCE, 0xA3, 0, 0xF0],
        "expected invalid-opcode IVT frame F000:A3CE; result={}; \
         cs={:#x}; rip={:#x}; return_frame={frame:02x?}; retired={}; completed_steps={}; virtual_time={}",
        run_result_summary(&result),
        cpu.code_segment(),
        cpu.reg(CpuReg::Rip),
        cpu.retired(),
        cpu.completed_steps(),
        assembled.machine.virtual_time(),
    );
}

fn synthetic_stop_machine(program: &[u8]) -> assembly::AssembledMachine {
    let temporary = TempDir::new("landmark-synthetic");
    let mut rom = vec![0xF4; 65536];
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0xCE, 0xA3, 0, 0xF0]);
    rom[0xA3CE..0xA3CE + program.len()].copy_from_slice(program);
    let image = temporary.path().join("synthetic.bin");
    std::fs::write(&image, rom).unwrap();
    let file = write_machine_file(temporary.path(), &ImageSource::File(image)).unwrap();
    let mut loaded = machine_file::load_for_assembly(
        file,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    loaded.config.images.push(assembly::LoadImage {
        base: 6 * 4,
        source: ImageSource::Inline(vec![0, 0xA4, 0, 0xF0]),
        kind: assembly::ImageKind::RamLoad,
        provides: Vec::new(),
    });
    loaded.assemble(&mda_policy()).unwrap()
}

fn run_policy(budget: u64) -> RunPolicy {
    RunPolicy::natural(Some(budget), Some(pc_harness::BIOS_IDLE_LIMIT))
}

fn mda_policy() -> ObservationPolicy {
    ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::Mda,
            output: OutputKind::TextScanout,
            byte_limit: 1_048_576,
        }],
        ..ObservationPolicy::default()
    }
}

fn quoted_path(path: &Path) -> anyhow::Result<String> {
    let absolute = path.canonicalize()?;
    let text = absolute
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF-8 firmware path"))?;
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            character if character.is_control() => {
                write!(quoted, "\\u{:04x}", u32::from(character))?
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

fn write_machine_file(directory: &Path, source: &ImageSource) -> anyhow::Result<PathBuf> {
    let mut text = String::from(
        "schema=1\nextends='preset:ibm-5170'\n\
         [board.rtc]\nstart='2000-01-01T00:00:00'\nnvram='derived'\n\
         [firmware.bios]\nprovides=[]\n",
    );
    match source {
        ImageSource::File(path) => writeln!(text, "image={}", quoted_path(path)?)?,
        ImageSource::Interleaved(parts) => {
            text.push_str("parts=[\n");
            for part in parts {
                let lane = match part.lane {
                    assembly::Lane::Even => "even",
                    assembly::Lane::Odd => "odd",
                };
                writeln!(
                    text,
                    "{{image={},lane='{lane}',offset={}}},",
                    quoted_path(&part.path)?,
                    part.offset
                )?;
            }
            text.push_str("]\n");
        }
        _ => anyhow::bail!("discovery must return file-backed firmware"),
    }
    let path = directory.join("diagnostic.toml");
    std::fs::write(&path, text)?;
    Ok(path)
}

fn write_post_machine_file(
    directory: &Path,
    source: &ImageSource,
    floppy: &Path,
) -> anyhow::Result<PathBuf> {
    write_storage_machine_file(directory, source, floppy, "['floppy','fixed-disk']")
}

fn write_landmark_machine_file(
    directory: &Path,
    source: &ImageSource,
    floppy: &Path,
) -> anyhow::Result<PathBuf> {
    write_storage_machine_file(directory, source, floppy, "[]")
}

fn write_storage_machine_file(
    directory: &Path,
    source: &ImageSource,
    floppy: &Path,
    provides: &str,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        floppy.is_file(),
        "missing fixture vendor/freedos/freedos-1200.img — run vendor/freedos/fetch.sh"
    );
    let file = write_machine_file(directory, source)?;
    let floppy = quoted_path(floppy)?;
    let mut text = std::fs::read_to_string(&file)?
        .replace("provides=[]", &format!("provides={provides}"))
        .replace("[board.rtc]", "[board]\ndisplay_switch='auto'\n[board.rtc]");
    writeln!(text, "\n[[card]]\nid='vga-text'\nremove=true\n[[card]]\nkind='mda'\n[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\nimage={floppy}\nwrite='readonly'\n[[card]]\nkind='ide'\ncontroller_model='at-adapter'")?;
    std::fs::write(&file, text)?;
    Ok(file)
}

#[test]
fn ibm_post_measurement_uses_mda_readonly_floppy_and_volatile_cmos() {
    let temporary = TempDir::new("ibm-post-synthetic");
    let image = temporary.path().join("synthetic.bin");
    let floppy = temporary.path().join("floppy.img");
    std::fs::write(&image, vec![0xF4; 65536]).unwrap();
    std::fs::write(&floppy, vec![0; 1_228_800]).unwrap();
    let file =
        write_post_machine_file(temporary.path(), &ImageSource::File(image), &floppy).unwrap();
    let loaded = machine_file::load_for_assembly(
        &file,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    assert_eq!(loaded.config.board, BoardModel::Ibm5170);
    assert_eq!(loaded.config.display_switch, assembly::DisplaySwitch::Auto);
    assert_eq!(
        loaded.config.primary_display().unwrap(),
        assembly::PrimaryDisplay::Monochrome
    );
    assert_eq!(loaded.config.drives.len(), 1);
    assert_eq!(loaded.config.drives[0].kind.image_bytes(), 1_228_800);
    assert!(loaded.cards.iter().any(|(_, id)| *id == DeviceId::Ide));
    assert!(loaded.config.devices.iter().any(|device| matches!(
        &device.kind,
        assembly::DeviceKind::Ide(ide) if ide.model == devices::ata::AtaControllerModel::AtAdapter
    )));
    assert_eq!(loaded.config.drives[0].write, assembly::WriteMode::ReadOnly);
    assert!(loaded.cards.iter().any(|(_, id)| *id == DeviceId::Mda));
    assert!(!loaded.cards.iter().any(|(_, id)| *id == DeviceId::Vga));
    let rtc = loaded
        .config
        .devices
        .iter()
        .find_map(|device| match &device.kind {
            assembly::DeviceKind::Rtc(rtc) => Some(rtc),
            _ => None,
        })
        .unwrap();
    assert_eq!(rtc.start, assembly::RtcStart::default());
    assert_eq!(rtc.nvram, assembly::NvramSource::Derived);
}

#[test]
fn ibm_post_missing_floppy_reports_fetch_command_without_paths() {
    let temporary = TempDir::new("ibm-post-missing-floppy");
    let image = temporary.path().join("synthetic.bin");
    std::fs::write(&image, vec![0xF4; 65536]).unwrap();
    let error = write_post_machine_file(
        temporary.path(),
        &ImageSource::File(image),
        &temporary.path().join("missing.img"),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "missing fixture vendor/freedos/freedos-1200.img — run vendor/freedos/fetch.sh"
    );
}

#[test]
fn landmark_missing_floppy_reports_fetch_command_without_paths() {
    let temporary = TempDir::new("landmark-missing-floppy");
    let error = write_landmark_machine_file(
        temporary.path(),
        &ImageSource::File(temporary.path().join("synthetic.bin")),
        &temporary.path().join("missing.img"),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "missing fixture vendor/freedos/freedos-1200.img — run vendor/freedos/fetch.sh"
    );
}

fn expect_redacted<T, E: std::fmt::Display>(
    found: &FoundUserFirmware,
    result: Result<T, E>,
    temporary: &Path,
    context: &str,
) -> T {
    found.expect_redacted(
        result.map_err(|error| redact_machine_error(&error.to_string(), temporary)),
        context,
    )
}

fn redact_machine_error(error: &str, temporary: &Path) -> String {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let mut message = error.to_owned();
    for (root, label) in [(repository, "<repository>"), (temporary, "<test-machine>")] {
        for path in [Some(root.to_path_buf()), root.canonicalize().ok()]
            .into_iter()
            .flatten()
        {
            message = message.replace(&path.display().to_string(), label);
            let debug = format!("{path:?}");
            message = message.replace(&debug[1..debug.len() - 1], label);
        }
    }
    message
}

#[test]
fn landmark_machine_errors_redact_temporary_and_repository_paths() {
    let temporary = TempDir::new("landmark-redaction");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    for (path, label) in [
        (temporary.path(), "<test-machine>"),
        (repository, "<repository>"),
    ] {
        let error = format!(
            "could not load {:?} at {}",
            path.join("diagnostic.toml"),
            path.display()
        );
        let redacted = redact_machine_error(&error, temporary.path());
        assert!(redacted.contains(label));
        assert!(!redacted.contains(&path.display().to_string()));
    }
}

#[test]
fn landmark_frontend_path_reads_board_adapter_gap() {
    let mut assembled =
        synthetic_stop_machine(&[0xB8, 0, 0xC0, 0x8E, 0xD8, 0xA0, 1, 0, 0x0F, 0x01, 0x0F]);
    let result = assembled.run_session(&run_policy(16));
    assert!(
        matches!(result, Ok(RunOutcome::Halted)),
        "{}",
        run_result_summary(&result)
    );
    assert_eq!(assembled.machine.cpu().reg(CpuReg::Rax) & 0xFF, 0);
}

#[test]
fn landmark_machine_file_preserves_absolute_parts_and_preset_settings() {
    let temporary = TempDir::new("landmark-parts");
    let even = temporary.path().join("even-\"-\\-\n.bin");
    let odd = temporary.path().join("odd.bin");
    std::fs::write(&even, vec![0x12; 32768]).unwrap();
    std::fs::write(&odd, vec![0x34; 32768]).unwrap();
    let source = ImageSource::Interleaved(vec![
        assembly::ImagePart {
            path: even.clone(),
            lane: assembly::Lane::Even,
            offset: 0,
        },
        assembly::ImagePart {
            path: odd.clone(),
            lane: assembly::Lane::Odd,
            offset: 0,
        },
    ]);
    let floppy = temporary.path().join("floppy.img");
    std::fs::write(&floppy, vec![0; 1_228_800]).unwrap();
    let file = write_landmark_machine_file(temporary.path(), &source, &floppy).unwrap();
    let (document, _) = machine_file::resolve(&file).unwrap();
    let bios = document.firmware.unwrap().bios.unwrap();
    assert_eq!(bios.provides, Some(Vec::new()));
    let parts = bios.parts.unwrap();
    assert_eq!(parts.len(), 2);
    for (part, (expected, lane)) in parts.iter().zip([
        (even, machine_file::schema::v1::Lane::Even),
        (odd, machine_file::schema::v1::Lane::Odd),
    ]) {
        assert_eq!(part.lane, lane);
        assert_eq!(part.offset, 0);
        assert!(Path::new(&part.image).is_absolute());
        assert!(Path::new(&part.image) == expected.canonicalize().unwrap());
    }
    // Synthetic bytes cannot receive the catalogue's diagnostic permission.
    // Supply services only for lowering this fixture's hardware configuration.
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace("provides=[]", "provides=['floppy','fixed-disk']"),
    )
    .unwrap();
    let loaded = machine_file::load_for_assembly(
        file,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    let ImageSource::Inline(bytes) = &loaded.config.images[0].source else {
        panic!("expected lowered synthetic parts")
    };
    assert!(*bytes == [0x12, 0x34].repeat(32768), "part order changed");
    assert_eq!(loaded.config.drives.len(), 1);
    let drive = &loaded.config.drives[0];
    assert_eq!(drive.controller, DeviceId::Fdc);
    assert_eq!(drive.unit, 0);
    assert_eq!(drive.kind.image_bytes(), 1_228_800);
    assert_eq!(drive.image, ImageSource::Inline(vec![0; 1_228_800]));
    assert_eq!(drive.write, assembly::WriteMode::ReadOnly);
    assert_eq!(loaded.config.display_switch, assembly::DisplaySwitch::Auto);
    assert_eq!(
        loaded.config.primary_display().unwrap(),
        assembly::PrimaryDisplay::Monochrome
    );
    assert!(loaded.config.devices.iter().any(|device| matches!(
        &device.kind,
        assembly::DeviceKind::Ide(ide) if ide.model == devices::ata::AtaControllerModel::AtAdapter
    )));
    assert_eq!(loaded.config.images[0].base, 0xF0000);
    assert_eq!(
        loaded.config.memory.ram,
        vec![
            assembly::RamRegion::new(0, 0xB0000),
            assembly::RamRegion::new(0xB8000, 0x8000),
        ]
    );
    assert_eq!(loaded.config.clock.retired_per_pit_clock, 8);
    assert_eq!(loaded.config.clock.retired_per_pit_clock_denominator, 13);
    for id in [DeviceId::Mda, DeviceId::Fdc, DeviceId::Ide] {
        assert!(loaded.config.devices.iter().any(|device| device.id == id));
    }
    assert!(!loaded
        .config
        .devices
        .iter()
        .any(|device| device.id == DeviceId::Vga));
    let rtc = loaded
        .config
        .devices
        .iter()
        .find_map(|device| match &device.kind {
            assembly::DeviceKind::Rtc(rtc) => Some(rtc),
            _ => None,
        })
        .unwrap();
    assert_eq!(rtc.start, assembly::RtcStart::default());
    assert_eq!(rtc.nvram, assembly::NvramSource::Derived);
}

#[test]
fn landmark_invalid_opcode_delivers_real_mode_offset_through_ivt() {
    // 0F FF is undefined on the 80286 (Intel 80286 Programmer's Reference
    // Manual, the invalid-opcode exception, interrupt 6). 0F 05 is not: it is LOADALL.
    let mut assembled = synthetic_stop_machine(&[0x0F, 0xFF]);
    let result = assembled.run_session(&run_policy(8));
    assert_synthetic_ud(&assembled, result);
    assert_eq!(assembled.machine.cpu().retired(), 2);
}

#[test]
fn landmark_unexpected_stop_reports_variant_location_and_counts() {
    for (program, expected_result, count) in [
        (&[0x90, 0x0F, 0xFF][..], "Ok(Halted)", 3),
        (&[0xEB, 0xFE][..], "Ok(RetiredCapReached)", 8),
    ] {
        let mut assembled = synthetic_stop_machine(program);
        let result = assembled.run_session(&run_policy(8));
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_synthetic_ud(&assembled, result);
        }))
        .expect_err("unexpected stop must fail");
        let message = panic
            .downcast_ref::<String>()
            .expect("formatted stop failure");
        for field in [
            format!("result={expected_result}"),
            "cs=0xf000".into(),
            format!("retired={count}"),
            format!("completed_steps={count}"),
            format!("virtual_time={count}"),
        ] {
            assert!(message.contains(&field), "missing stop field: {field}");
        }
        assert!(message.contains("rip=0x"), "missing RIP field");
    }
}

#[test]
fn landmark_stop_summary_omits_free_form_cpu_payloads() {
    for error in [
        CpuError::Memory("synthetic private payload".into()),
        CpuError::UnexpectedExit {
            what: "synthetic private payload".into(),
        },
    ] {
        let summary = run_result_summary(&Err(RunError::Machine(MachineError::Cpu(error))));
        assert!(!summary.contains("synthetic private payload"));
        assert!(summary.contains("<details omitted>"));
    }
    assert_eq!(
        run_result_summary(&Err(RunError::Machine(MachineError::Cpu(
            CpuError::Unmapped { addr: 0x12345 }
        )))),
        "Err(Machine(Cpu(Unmapped { addr: 0x12345 })))",
    );
}

#[test]
fn landmark_status_classification_uses_only_status_tokens() {
    assert_eq!(verdict(b" PASSED "), Verdict::Passed);
    assert_eq!(verdict(b" FAILED "), Verdict::Failed);
    assert_eq!(verdict(b" NOT PRESENT "), Verdict::NotPresent);
    assert_eq!(verdict(b" \0      "), Verdict::Empty);
    assert_eq!(verdict(b"PASS"), Verdict::Unrecognized);
    assert_eq!(verdict(b"XPASSED "), Verdict::Unrecognized);
}

#[test]
fn landmark_status_cells_ignore_surrounding_text() {
    let mut vram = vec![0; 80 * 25 * 2];
    for cell in vram.as_chunks_mut::<2>().0 {
        cell[0] = b' ';
    }
    for (row, col, token) in [
        (1, 0, &b"|"[..]),
        (1, 2, b"FAILED"),
        (1, 34, b"X"),
        (1, 35, b"PASSED"),
        (1, 47, b"|"),
        (1, 49, b"FAILED"),
        (2, 35, b"FAILED"),
        (2, 47, b"|"),
        (3, 35, b"NOT PRESENT"),
        (3, 47, b"|"),
        (13, 0, b"|"),
        (13, 2, b"PASSED"),
        (13, 47, b"|"),
        (13, 49, b"PASSED"),
    ] {
        for (offset, glyph) in token.iter().enumerate() {
            vram[(row * 80 + col + offset) * 2] = *glyph;
        }
    }
    let grid = video::render_text_mode(&device::TextModeState {
        vram: &vram,
        cols: 80,
        rows: 25,
        start_cell: 0,
        cursor_cell: 0,
        cursor_visible: false,
        attributes: device::TextAttributeSet::Monochrome {
            blink_enabled: true,
        },
    });
    assert_eq!(row_verdict(&grid, 1), Verdict::Passed);
    assert_eq!(row_verdict(&grid, 2), Verdict::Failed);
    assert_eq!(row_verdict(&grid, 3), Verdict::NotPresent);
    assert_eq!(row_verdict(&grid, 13), Verdict::Empty);
}

/// Execute the listed firmware's real-mode diagnostic prefix.
///
/// The refresh bit follows PIT channel 1 (IBM Personal Computer AT Technical
/// Reference, System Timers) and the board runs at 8/13 steps per PIT clock, so
/// the former Application Real16 prefix retires fewer instructions with the
/// same text hash. I286's new stop and final screen still require measurement.
#[test]
#[ignore = "requires separately supplied proprietary firmware; I286 stop awaits measurement"]
fn user_rom_landmark_at_real_mode_prefix() {
    let catalogue = BoardModel::Ibm5170.firmware_catalogue();
    let found = machine_file::require_user_firmware!(ENTRY_ID, catalogue);
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    let temporary = TempDir::new("landmark-machine");
    let file = expect_redacted(
        &found,
        write_machine_file(temporary.path(), &found.source),
        temporary.path(),
        "writing diagnostic machine file",
    );
    let loaded = expect_redacted(
        &found,
        machine_file::load_for_assembly(&file, AssemblyOptions::default()),
        temporary.path(),
        "loading diagnostic machine file",
    );
    assert!(loaded
        .cards
        .iter()
        .any(|(id, device)| id == "mda" && *device == DeviceId::Mda));
    let mut assembled = expect_redacted(
        &found,
        loaded.assemble(&mda_policy()),
        temporary.path(),
        "assembling diagnostic",
    );
    eprintln!("{ENTRY_ID}: real16_prefix_retired={REAL16_GOLDEN_RETIRED}, real16_prefix_text_sha256={REAL16_GOLDEN_TEXT_SHA256}");
    let result = assembled.run_session(&run_policy(STEP_BUDGET));
    let cpu = assembled.machine.cpu();
    eprintln!("{ENTRY_ID}: stop={}, cs={:04x}, ip={:04x}, retired={}, completed_steps={}, virtual_time={}",
        run_result_summary(&result), cpu.code_segment(), cpu.reg(CpuReg::Rip),
        cpu.retired(), cpu.completed_steps(), assembled.machine.virtual_time());
    if let Some((cs, ip)) = GOLDEN_STOP {
        assert_eq!(
            (cpu.code_segment(), cpu.reg(CpuReg::Rip)),
            (u64::from(cs), ip)
        );
    }
    let retired = cpu.retired();
    let capture = found.expect_redacted(assembled.capture_text(DeviceId::Mda), "capturing MDA");
    let Frame::Text(grid) = capture.frame else {
        panic!("diagnostic golden requires an MDA text frame, received pixels");
    };
    assert_eq!((grid.cols(), grid.rows()), (80, 25));
    let text_sha256 = assembly::test_support::text_plane_sha256(&video::Frame::Text(grid.clone()));
    eprintln!("{ENTRY_ID}: retired={retired}, text_sha256={text_sha256}");
    for (index, id) in ROW_IDS.iter().enumerate() {
        eprintln!("{id}={:?}", row_verdict(&grid, index as u16 + 1));
    }
    eprintln!("protected_mode={:?}", row_verdict(&grid, 13));
    if let Some(expected) = GOLDEN_RETIRED {
        assert_eq!(retired, expected, "retired count");
    }
    if let Some(expected) = GOLDEN_TEXT_SHA256 {
        assert_eq!(text_sha256, expected, "MDA text plane hash");
    }
}

/// Measure a complete diagnostic pass without retaining proprietary screen text.
///
/// Landmark Research International, ROM POST User's Manual v1.2, pp. 16–36:
/// the ROM drives the controllers directly and repeats the tests. A fixed total
/// virtual-time budget includes completed CPU steps and halted PIT progress,
/// making the final text-plane hash reproducible.
#[test]
#[ignore = "requires separately supplied proprietary firmware"]
fn user_rom_landmark_at_full_pass() {
    let catalogue = BoardModel::Ibm5170.firmware_catalogue();
    let found = machine_file::require_user_firmware!(ENTRY_ID, catalogue);
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    let temporary = TempDir::new("landmark-full-pass-machine");
    let floppy =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/freedos/freedos-1200.img");
    let file = expect_redacted(
        &found,
        write_landmark_machine_file(temporary.path(), &found.source, &floppy),
        temporary.path(),
        "writing diagnostic machine file",
    );
    let loaded = expect_redacted(
        &found,
        machine_file::load_for_assembly(&file, AssemblyOptions::default()),
        temporary.path(),
        "loading diagnostic machine file",
    );
    let mut assembled = expect_redacted(
        &found,
        loaded.assemble(&mda_policy()),
        temporary.path(),
        "assembling diagnostic",
    );
    let result = assembled.run_session(&run_policy(FULL_PASS_RETIRED_CAP));
    let cpu = assembled.machine.cpu();
    let location = (cpu.code_segment(), cpu.reg(CpuReg::Rip));
    let retired = cpu.retired();
    let virtual_time = assembled.machine.virtual_time();
    let capture = found.expect_redacted(assembled.capture_text(DeviceId::Mda), "capturing MDA");
    let Frame::Text(grid) = capture.frame else {
        panic!("diagnostic golden requires an MDA text frame, received pixels");
    };
    assert_eq!((grid.cols(), grid.rows()), (80, 25));
    let text_sha256 = assembly::test_support::text_plane_sha256(&Frame::Text(grid));
    eprintln!(
        "{ENTRY_ID}: stop={}, cs={:04x}, ip={:04x}, retired={retired}, virtual_time={virtual_time}, text_sha256={text_sha256}",
        run_result_summary(&result),
        location.0,
        location.1,
    );
    assert_eq!(virtual_time, FULL_PASS_RETIRED_CAP, "total virtual time");
    if let Some(expected) = FULL_PASS_GOLDEN_STOP_KIND {
        assert!(
            matches!(result, Ok(actual) if actual == expected),
            "unexpected stop: {}",
            run_result_summary(&result),
        );
    }
    if let Some((cs, ip)) = FULL_PASS_GOLDEN_STOP {
        assert_eq!(location, (u64::from(cs), ip));
    }
    if let Some(expected) = FULL_PASS_GOLDEN_RETIRED {
        assert_eq!(retired, expected, "retired count");
    }
    if let Some(expected) = FULL_PASS_GOLDEN_TEXT_SHA256 {
        assert_eq!(text_sha256, expected, "MDA text plane hash");
    }
}

// This value moved once when the board oscillator became the exact 315/22 MHz
// constant, shifting timer phase by a fraction of a PIT clock.
const IBM_GOLDEN_STOP: Option<(u16, u64)> = Some((0x0070, 0x02aa));
const IBM_GOLDEN_RETIRED: Option<u64> = Some(24_872_018);
const IBM_GOLDEN_POST: Option<&[u8]> = Some(&[
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
    0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
    0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47,
    0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57,
    0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67,
    0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77,
    0x78, 0x79, 0x7A, 0x7B, 0x7C, 0x7D, 0x7E, 0x7F, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8A, 0x8B, 0x8C, 0x8D, 0x8E, 0x8F, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97,
    0x98, 0x99, 0x9A, 0x9B, 0x9C, 0x9D, 0x9E, 0x9F, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
    0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF, 0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7,
    0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xC1, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7,
    0xC8, 0xC9, 0xCA, 0xCB, 0xCC, 0xCD, 0xCE, 0xCF, 0xD0, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7,
    0xD8, 0xD9, 0xDA, 0xDB, 0xDC, 0xDD, 0xDE, 0xDF, 0xE0, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7,
    0xE8, 0xE9, 0xEA, 0xEB, 0xEC, 0xED, 0xEE, 0xEF, 0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7,
    0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0x09, 0x0A, 0x0B, 0x0C, 0x0E, 0x0F, 0x32, 0x11, 0x12, 0x13,
    0x14, 0x15, 0x16, 0x18, 0x19, 0x81, 0x85, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x32,
    0x22, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23, 0x23,
    0x23, 0x24, 0x25, 0x26, 0x27, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2F, 0x30, 0x81, 0x85, 0x31, 0x31,
    0x32, 0x31, 0x32, 0x31, 0x32, 0x31, 0x32, 0x31, 0x32, 0x31, 0x32, 0x31, 0x32, 0x31, 0x32, 0x31,
    0x32, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x34, 0xF0, 0x81, 0x85, 0xF1, 0xB0,
    0xF2, 0x9D, 0xF3, 0xF4, 0x95, 0x95, 0xF5, 0xF6, 0x9D, 0xF7, 0xF8, 0xF9, 0xFA, 0x35, 0x34, 0x35,
    0x36, 0x37, 0x38, 0x3A, 0x3C, 0x3D, 0x3E, 0x3B, 0x40, 0x41, 0x43,
]);
const IBM_GOLDEN_DOS_PROMPT: Option<bool> = Some(true);
const IBM_GOLDEN_TEXT_SHA256: Option<&str> =
    Some("c02b5aeabf65118af62136535b8637d42a4792378d0e407463c3865debc484e7");

#[test]
#[ignore = "requires separately supplied proprietary firmware; measurement only"]
fn user_rom_ibm_5170_1985_post() {
    const IBM_ENTRY: &str = "ibm-5170-bios-1985-06-10";
    let catalogue = BoardModel::Ibm5170.firmware_catalogue();
    let found = machine_file::require_user_firmware!(IBM_ENTRY, catalogue);
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    let temporary = TempDir::new("ibm-post-machine");
    let floppy =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/freedos/freedos-1200.img");
    let file = expect_redacted(
        &found,
        write_post_machine_file(temporary.path(), &found.source, &floppy),
        temporary.path(),
        "writing POST machine file",
    );
    let loaded = expect_redacted(
        &found,
        machine_file::load_for_assembly(&file, AssemblyOptions::default()),
        temporary.path(),
        "loading POST machine",
    );
    let mut policy = mda_policy();
    policy.routes.push(ObservationRequest {
        device: DeviceId::Post,
        output: OutputKind::Bytes(device::ByteStreamKind::PostCheckpoints),
        byte_limit: 1_048_576,
    });
    let mut assembled = expect_redacted(
        &found,
        loaded.assemble(&policy),
        temporary.path(),
        "assembling POST machine",
    );
    let cap = 200_000_000;
    let mut result = assembled.run_session(&RunPolicy {
        retired_cap: Some(cap),
        idle_limit: Some(pc_harness::BIOS_IDLE_LIMIT),
        stop: StopCondition::PromptThen {
            console: ConsoleProbe {
                device: DeviceId::Mda,
            },
            needle: pc_harness::DOS_DATE_PROMPT.into(),
            poll_step: pc_harness::DOS_INTERACTIVE_POLL_STEP,
            then_budget: 0,
        },
        stimuli: vec![KeyboardStimulus {
            keyboard_index: assembled.device_index[&DeviceId::Keyboard],
            scancodes: [pc_harness::KEY_ENTER, pc_harness::KEY_ENTER].concat(),
            schedule: InjectionSchedule::AfterPrompt,
        }],
    });
    let (grid, dos_prompt) = loop {
        let capture = found.expect_redacted(assembled.capture_text(DeviceId::Mda), "capturing MDA");
        let Frame::Text(grid) = capture.frame else {
            panic!("POST measurement requires an MDA text frame, received pixels");
        };
        let dos_prompt = (0..grid.rows()).any(|row| {
            let glyphs: Vec<_> = (0..grid.cols())
                .map(|col| grid.cell(col, row).unwrap().glyph)
                .collect();
            glyphs.windows(3).any(|window| window == b"A:>")
                || glyphs.windows(4).any(|window| window == b"A:\\>")
        });
        if dos_prompt
            || !matches!(result, Ok(RunOutcome::RetiredCapReached))
            || assembled.machine.virtual_time() >= cap
        {
            break (grid, dos_prompt);
        }
        let next =
            (assembled.machine.virtual_time() + pc_harness::DOS_INTERACTIVE_POLL_STEP).min(cap);
        result = assembled.run_session(&run_policy(next));
    };
    let cpu = assembled.machine.cpu();
    let location = (cpu.code_segment(), cpu.reg(CpuReg::Rip));
    let retired = cpu.retired();
    eprintln!("{IBM_ENTRY}: stop={}, cs={:04x}, ip={:04x}, retired={retired}, completed_steps={}, virtual_time={}",
        run_result_summary(&result), location.0, location.1, cpu.completed_steps(), assembled.machine.virtual_time());
    let post = found.expect_redacted(
        assembled
            .observation
            .drain_bytes(DeviceId::Post, device::ByteStreamKind::PostCheckpoints),
        "reading POST checkpoints",
    );
    let text_sha256 = assembly::test_support::text_plane_sha256(&Frame::Text(grid));
    eprintln!("{IBM_ENTRY}: post={post:02x?}, dos_prompt={dos_prompt}, text_sha256={text_sha256}");
    assert!(
        matches!(result, Ok(RunOutcome::RetiredCapReached)),
        "unexpected stop: {}",
        run_result_summary(&result)
    );
    if let Some((cs, ip)) = IBM_GOLDEN_STOP {
        assert_eq!(location, (u64::from(cs), ip));
    }
    if let Some(expected) = IBM_GOLDEN_RETIRED {
        assert_eq!(retired, expected);
    }
    if let Some(expected) = IBM_GOLDEN_POST {
        assert_eq!(post, expected);
    }
    if let Some(expected) = IBM_GOLDEN_DOS_PROMPT {
        assert_eq!(dos_prompt, expected);
    }
    if let Some(expected) = IBM_GOLDEN_TEXT_SHA256 {
        assert_eq!(text_sha256, expected, "MDA text plane hash");
    }
}
