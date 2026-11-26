// SPDX-License-Identifier: AGPL-3.0-or-later
//! Video-ROM POST integration and opt-in IBM VGA mode-13h measurements.
//!
//! IBM Personal Computer AT Technical Reference, TEST1 video-ROM scan;
//! Kiselev 8088_bios bios.asm, video initialization and extension_scan;
//! IBM VGA/XGA Technical Reference, RAM Loadable Character Generator and
//! 256-Color Graphics Mode (13h). See firmware-and-machine-classes.md
//! §Video ROM placement. CPU revision is pinned by Cargo.lock.

use assembly::{
    AssemblyOptions, BoardModel, DeviceId, FirmwareEntry, ImageSource, ObservationPolicy,
    ObservationRequest, OutputKind,
};
use cpu::{Cpu, CpuReg};
use machine::RunOutcome;
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

#[path = "support/fixtures.rs"]
mod fixtures;

const CPU_REV: &str = "67d6697bf110d36142d78a285637ae404f001a8f";

fn policy() -> ObservationPolicy {
    ObservationPolicy {
        routes: vec![
            ObservationRequest {
                device: DeviceId::Vga,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            },
            ObservationRequest {
                device: DeviceId::Post,
                output: OutputKind::Bytes(device::ByteStreamKind::PostCheckpoints),
                byte_limit: 4096,
            },
        ],
        ..ObservationPolicy::default()
    }
}

fn synthetic_container() -> Vec<u8> {
    let mut bytes = vec![0; 32768];
    let rom = &mut bytes[0x2000..];
    rom[..3].copy_from_slice(&[0x55, 0xAA, 0x30]);
    // Kiselev bios.asm, print: POST uses INT 10h immediately after the ROM
    // returns. An IRET stub avoids the uninitialized fallback console.
    // Preserve DS, install that stub, configure map mask and mark RAM, then RETF.
    let init = [
        0x1E, 0x31, 0xC0, 0x8E, 0xD8, 0xC7, 0x06, 0x40, 0x00, 0x00, 0x01, 0xC7, 0x06, 0x42, 0x00,
        0x00, 0xC0, 0xBA, 0xC4, 0x03, 0xB8, 0x02, 0x04, 0xEF, 0xC7, 0x06, 0x00, 0x06, 0x56, 0x47,
        0x1F, 0xCB,
    ];
    rom[3..3 + init.len()].copy_from_slice(&init);
    rom[0x100] = 0xCF;

    let sum = rom.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    rom[24575] = 0u8.wrapping_sub(sum);
    bytes
}

#[test]
#[ignore = "requires fetched open Kiselev BIOS; executes POST"]
fn kiselev_post_executes_video_rom_from_eprom_container() {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let Some(bios) = fixtures::fixture("kiselev-bios/bios-xi8088.bin") else {
        return;
    };
    assert!(
        assembly::KISELEV_XI8088.identifies(&bios),
        "open BIOS fixture identity drifted"
    );
    struct Images {
        bios: Vec<u8>,
    }
    impl machine_file::ImageReader for Images {
        fn read(&mut self, path: &std::path::Path) -> std::io::Result<Vec<u8>> {
            Ok(if path.ends_with("bios.bin") {
                self.bios.clone()
            } else {
                synthetic_container()
            })
        }
    }
    let text = "schema=1\n[board]\nmodel='ibm-5170'\n[board.rtc]\nstart='2000-01-01T00:00:00'\nnvram='derived'\n[firmware.bios]\nimage='bios.bin'\n[[firmware.option_roms]]\nimage='video.bin'\nbase=0xC0000\noffset=0x2000\n[[card]]\nkind='vga'\n";
    let path = std::path::Path::new("video-post.toml");
    let document = machine_file::parse_toml(path, text).unwrap();
    let origins = machine_file::Origins::from_toml(path, text).unwrap();
    let options = AssemblyOptions {
        allow_unlisted_firmware: true,
    };
    let config =
        machine_file::lower_with_options(&document, &origins, &mut Images { bios }, options)
            .unwrap();
    assert_eq!(config.images[1].source.known_len(), Some(24576));
    let mut assembled =
        machine_file::assemble_with_observation_and_options(&config, &policy(), options).unwrap();
    let mut post = Vec::new();
    for cap in (10_000..=5_000_000).step_by(10_000) {
        assembled.machine.run_bounded(Some(cap)).unwrap();
        post.extend(
            assembled
                .observation
                .drain_bytes(DeviceId::Post, device::ByteStreamKind::PostCheckpoints)
                .unwrap(),
        );
        if post.contains(&0x20) {
            break;
        }
    }
    assert!(
        post.windows(3).any(|codes| codes == [0x10, 0x11, 0x20]),
        "video init returned to POST: {post:02x?}"
    );
    let mut marker = [0; 2];
    assembled
        .machine
        .cpu()
        .read_mem(0x600, &mut marker)
        .unwrap();
    assert_eq!(marker, *b"VG");
    let index = assembled.device_index[&DeviceId::Vga];
    let vga = assembled
        .machine
        .bus_mut()
        .device_mut(index)
        .unwrap()
        .as_any_mut()
        .unwrap()
        .downcast_mut::<devices::Vga>()
        .unwrap();
    assert_eq!(vga.sequencer()[2], 4);
    let mut header = [0; 3];
    assembled
        .machine
        .cpu()
        .read_mem(0xC0000, &mut header)
        .unwrap();
    assert_eq!(header, [0x55, 0xAA, 0x30]);
}

const GOLDEN_STOP: Option<(u64, u64)> = Some((0x0000, 0x7C50));
// This value moved once when the board oscillator became the exact 315/22 MHz
// constant, shifting timer phase by a fraction of a PIT clock.
const GOLDEN_RETIRED: Option<u64> = Some(23_463_330);
const GOLDEN_TEXT_SHA256: Option<&str> =
    Some("707ba8a5e9a7f27f639e55cbc1a4375c86392f810c2742dc6aa0218ddc1adfab");
const GOLDEN_GRAPHICS_SHA256: Option<&str> =
    Some("825d409a82c363538f60248d866bb8462c28eb7ebdb0499bbd0188c6d0740f27");

const BOOT_STAGE_MAGIC: [u8; 4] = *b"VGA!";

fn mode13_floppy() -> Vec<u8> {
    let mut disk = vec![0; assembly::FloppyGeometry::HD_1_2.image_bytes() as usize];
    // BIOS boots at 0000:7C00. The long bounded text-stage delay gives the host
    // a capture boundary before INT 10h switches the same card to pixels.
    let mark_stage = |program: &mut Vec<u8>, stage: u8| {
        for (index, word) in BOOT_STAGE_MAGIC.as_chunks::<2>().0.iter().enumerate() {
            program.extend([0xC7, 0x06, index as u8 * 2, 0x06, word[0], word[1]]);
        }
        program.extend([0xC6, 0x06, 4, 0x06, stage]);
    };
    let mut program = vec![
        0xFA, 0x31, 0xC0, 0x8E, 0xD0, 0xBC, 0x00, 0x7C, 0x8E, 0xD8, 0xFB, 0xB8, 0x03, 0x00, 0xCD,
        0x10,
    ];
    mark_stage(&mut program, 1);
    program.extend([
        0xB9, 0xFF, 0xFF, 0xE2, 0xFE, 0xB8, 0x13, 0x00, 0xCD, 0x10, 0xB8, 0x00, 0xA0, 0x8E, 0xC0,
        0x31, 0xFF, 0xB9, 0x00, 0xFA, 0x31, 0xC0, 0xFC, 0xAA, 0xFE, 0xC0, 0xE2, 0xFB,
    ]);
    mark_stage(&mut program, 2);
    program.extend([0xFA, 0xF4]);
    disk[..program.len()].copy_from_slice(&program);
    disk[510..512].copy_from_slice(&[0x55, 0xAA]);
    disk
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
                write!(quoted, "\\u{:04x}", u32::from(character))?;
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

fn write_vga_post_machine_file(
    directory: &Path,
    bios: &ImageSource,
    video: &ImageSource,
    floppy: &Path,
) -> anyhow::Result<PathBuf> {
    let system = BoardModel::Ibm5170
        .firmware_catalogue()
        .iter()
        .find(|entry| entry.id == "ibm-5170-bios-1985-06-10")
        .unwrap();
    // IBM AT Technical Reference, Equipment Byte (Hex 14): the adapter with
    // its own video ROM uses 00b. Auto selects OwnBios with VGA and no MDA.
    let mut text = format!(
        "schema=1\nextends='preset:ibm-5170'\n\
         [board]\ndisplay_switch='auto'\n\
         [board.rtc]\nstart='2000-01-01T00:00:00'\nnvram='derived'\n\
         [firmware.bios]\nprovides=['floppy','fixed-disk']\nsha256='{}'\n",
        system.sha256
    );
    match bios {
        ImageSource::File(path) => writeln!(text, "image={}", quoted_path(path)?)?,
        ImageSource::Interleaved(parts) => {
            text.push_str("parts=[\n");
            for part in parts {
                writeln!(
                    text,
                    "{{image={},lane='{}',offset={}}},",
                    quoted_path(&part.path)?,
                    part.lane,
                    part.offset
                )?;
            }
            text.push_str("]\n");
        }
        _ => anyhow::bail!("system BIOS discovery must return file-backed firmware"),
    }
    let ImageSource::File(video) = video else {
        anyhow::bail!("video BIOS discovery must return a complete EPROM file");
    };
    writeln!(
        text,
        "\n[[firmware.option_roms]]\nimage={}\nsha256='{}'\nbase=0xC0000\n\
         [[card]]\nid='mda'\nremove=true\n\
         [[card]]\nid='vga-text'\nremove=true\n\
         [[card]]\nkind='vga'\n\
         [[card]]\nid='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\nimage={}\nwrite='readonly'\n\
         [[card]]\nkind='ide'\ncontroller_model='at-adapter'",
        quoted_path(video)?,
        FirmwareEntry::IBM_VGA_1987.sha256,
        quoted_path(floppy)?
    )?;
    let path = directory.join("ibm-vga.toml");
    std::fs::write(&path, text)?;
    Ok(path)
}

#[test]
fn ibm_vga_measurement_machine_keeps_the_ibm_post_hardware() {
    use machine_file::schema::v1;
    let temporary = machine_file::test_support::TempDir::new("ibm-vga-profile");
    let bios = temporary.path().join("system.bin");
    let video = temporary.path().join("video.bin");
    let floppy = temporary.path().join("boot.img");
    for path in [&bios, &video, &floppy] {
        std::fs::write(path, []).unwrap();
    }
    let parts = [assembly::Lane::Even, assembly::Lane::Odd]
        .into_iter()
        .map(|lane| assembly::ImagePart {
            path: bios.clone(),
            lane,
            offset: 0,
        })
        .collect();
    for source in [
        ImageSource::File(bios.clone()),
        ImageSource::Interleaved(parts),
    ] {
        let file = write_vga_post_machine_file(
            temporary.path(),
            &source,
            &ImageSource::File(video.clone()),
            &floppy,
        )
        .unwrap();
        // Resolving checks inheritance without reading or identifying any ROM.
        let (document, _) = machine_file::resolve(file).unwrap();
        let board = document.board.unwrap();
        assert_eq!(board.display_switch, Some(v1::DisplaySwitch::Auto));
        let rtc = board.rtc.unwrap();
        assert_eq!(rtc.start.as_deref(), Some("2000-01-01T00:00:00"));
        assert_eq!(rtc.nvram, Some(v1::Nvram::Derived));
        let cards = document.card.unwrap();
        assert!(!cards
            .iter()
            .any(|card| matches!(card.kind, Some(v1::CardKind::Mda | v1::CardKind::VgaText))));
        assert_eq!(
            cards
                .iter()
                .filter(|card| card.kind == Some(v1::CardKind::Vga))
                .count(),
            1
        );
        let ide = cards
            .iter()
            .find(|card| card.kind == Some(v1::CardKind::Ide))
            .unwrap();
        assert_eq!(ide.controller_model, Some(v1::ControllerModel::AtAdapter));
        let fdc = cards
            .iter()
            .find(|card| card.kind == Some(v1::CardKind::Fdc765))
            .unwrap();
        let drives = fdc.drives.as_ref().unwrap();
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].unit, Some(0));
        assert_eq!(drives[0].drive_type, Some(v1::DriveType::Hd12));
        assert_eq!(drives[0].write, Some(v1::WriteMode::ReadOnly));
        let firmware = document.firmware.unwrap();
        let system = firmware.bios.unwrap();
        match source {
            ImageSource::File(_) => assert!(system.image.is_some() && system.parts.is_none()),
            ImageSource::Interleaved(_) => {
                assert!(system.image.is_none());
                assert_eq!(system.parts.unwrap().len(), 2);
            }
            _ => unreachable!(),
        }
        let video = &firmware.option_roms.as_ref().unwrap()[0];
        assert_eq!(video.base, Some(0xC0000));
        assert_eq!(video.offset, None);
        assert_eq!(
            video.sha256.as_ref().unwrap().as_str(),
            FirmwareEntry::IBM_VGA_1987.sha256
        );
    }
}

#[test]
#[ignore = "requires separately supplied IBM 5170 and IBM Display Adapter ROMs"]
fn user_rom_ibm_vga_bios_mode13_pattern() {
    let catalogue = BoardModel::Ibm5170.firmware_catalogue();
    let bios = machine_file::require_user_firmware!("ibm-5170-bios-1985-06-10", catalogue);
    let video = machine_file::require_user_firmware!(FirmwareEntry::IBM_VGA_1987.id, catalogue);
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let temporary = machine_file::test_support::TempDir::new("ibm-vga-post-machine");
    let redact = |message: &str| {
        video
            .redact(&bios.redact(message))
            .replace(&temporary.path().display().to_string(), "<temporary>")
    };
    let floppy = temporary.path().join("boot.img");
    std::fs::write(&floppy, mode13_floppy())
        .unwrap_or_else(|error| panic!("writing boot disk: {}", redact(&error.to_string())));
    let file = write_vga_post_machine_file(temporary.path(), &bios.source, &video.source, &floppy)
        .unwrap_or_else(|error| panic!("writing IBM VGA machine: {}", redact(&error.to_string())));
    let loaded = machine_file::load_for_assembly(&file, AssemblyOptions::default())
        .unwrap_or_else(|error| panic!("loading IBM VGA machine: {}", redact(&error.to_string())));
    assert_eq!(
        loaded.config.primary_display().unwrap(),
        assembly::PrimaryDisplay::OwnBios
    );
    let mut observation = policy();
    observation
        .routes
        .iter_mut()
        .find(|route| route.device == DeviceId::Post)
        .unwrap()
        .byte_limit = 1_048_576;
    let mut assembled = loaded
        .assemble(&observation)
        .unwrap_or_else(|error| panic!("assembling IBM VGA: {}", redact(&error.to_string())));
    assembled
        .machine
        .set_idle_limit(Some(pc_harness::BIOS_IDLE_LIMIT));
    let VgaRun {
        outcome,
        text_hash,
        post,
        text_dimensions,
    } = run_vga_program(&mut assembled);
    let cpu = assembled.machine.cpu();
    let location = (cpu.code_segment(), cpu.reg(CpuReg::Rip));
    let retired = cpu.retired();
    eprintln!(
        "ibm-vga: stop={outcome:?} cs={:04x} ip={:04x} retired={retired}",
        location.0, location.1
    );
    eprintln!("ibm-vga: post={post:02x?}");
    assert_eq!(
        outcome,
        RunOutcome::Halted,
        "program must finish before the cap"
    );
    assert_eq!(
        boot_program_stage(assembled.machine.cpu()),
        Some(2),
        "boot program graphics stage"
    );
    assert_eq!(text_dimensions, Some((80, 25)), "mode 03h text geometry");
    let text_hash = text_hash.expect("text stage before graphics mode");
    let capture = assembled.capture_text(DeviceId::Vga).unwrap();
    let video::Frame::Pixels(pixels) = &capture.frame else {
        panic!("mode 13h must capture pixels");
    };
    assert_eq!((pixels.width, pixels.height), (320, 200));
    for (offset, index) in pixels.indices.iter().enumerate() {
        assert_eq!(*index, offset as u8, "pattern pixel {offset}");
    }
    let graphics_hash = assembly::test_support::graphics_frame_sha256(&capture.frame);
    eprintln!("ibm-vga: text_sha256={text_hash} graphics_sha256={graphics_hash}");
    if let Some(expected) = GOLDEN_STOP {
        assert_eq!(location, expected);
    }
    if let Some(expected) = GOLDEN_RETIRED {
        assert_eq!(retired, expected);
    }
    if let Some(expected) = GOLDEN_TEXT_SHA256 {
        assert_eq!(text_hash, expected);
    }
    if let Some(expected) = GOLDEN_GRAPHICS_SHA256 {
        assert_eq!(graphics_hash, expected);
    }
}

fn vga_register_state(vga: &devices::Vga) -> String {
    use device::Device;
    let text = vga.planar_text_scanout().unwrap();
    format!(
        "misc={:02x} sequencer={:02x?} crtc={:02x?} graphics={:02x?} attribute={:02x?} \
         dac_mask={:02x} subsystem_3c3={:02x} adapter_46e8={:02x} setup_102={:02x} \
         cols={} rows={} char_height={} start={:04x} offset={:02x} row_stride={} address_shift={} enabled={} pas={}",
        vga.misc_output(), vga.sequencer(), vga.crtc(), vga.graphics(), vga.attribute(),
        vga.pel_mask(), vga.subsystem_enable(), vga.adapter_control(), vga.setup_enable(),
        text.dimensions.0, text.dimensions.1, text.cell_size.1, text.start, vga.crtc()[0x13],
        text.row_stride, text.address_shift, text.enabled, vga.palette_address_source(),
    )
}

fn print_vga_register_state(assembled: &mut assembly::AssembledMachine, boundary: &str) {
    let index = assembled.device_index[&DeviceId::Vga];
    // The keyboard scheduling probe shares this loop with a legacy text card.
    if let Some(vga) = assembled
        .machine
        .bus_mut()
        .device_mut(index)
        .unwrap()
        .as_any_mut()
        .and_then(|device| device.downcast_mut::<devices::Vga>())
    {
        eprintln!("ibm-vga: {boundary} {}", vga_register_state(vga));
    }
}

fn boot_program_stage(cpu: &cpu::X86JitCpu) -> Option<u8> {
    // IBM AT Technical Reference, TEST1 memory test: repeated fill words can
    // imitate a stage byte. Require a nonuniform signature and execution in
    // the boot sector before accepting a sample.
    if cpu.code_segment() != 0 || !(0x7C00..0x7E00).contains(&cpu.reg(CpuReg::Rip)) {
        return None;
    }
    let mut marker = [0; 5];
    cpu.read_mem(0x600, &mut marker).unwrap();
    (marker[..4] == BOOT_STAGE_MAGIC).then_some(marker[4])
}

struct VgaRun {
    outcome: RunOutcome,
    text_hash: Option<String>,
    post: Vec<u8>,
    text_dimensions: Option<(u16, u16)>,
}

fn run_vga_program(assembled: &mut assembly::AssembledMachine) -> VgaRun {
    let mut text_hash = None;
    let mut text_dimensions = None;
    let mut outcome = RunOutcome::RetiredCapReached;
    let mut post = Vec::new();
    for cap in (10_000..=200_000_000).step_by(10_000) {
        let result = assembled.machine.run_bounded(Some(cap));
        post.extend(
            assembled
                .observation
                .drain_bytes(DeviceId::Post, device::ByteStreamKind::PostCheckpoints)
                .unwrap(),
        );
        outcome = result.unwrap_or_else(|_| {
            print_vga_register_state(assembled, "end");
            panic!(
                "IBM VGA execution failed at cs={:04x} ip={:04x} retired={} post={post:02x?}",
                assembled.machine.cpu().code_segment(),
                assembled.machine.cpu().reg(CpuReg::Rip),
                assembled.machine.cpu().retired()
            )
        });
        if boot_program_stage(assembled.machine.cpu()) == Some(1) && text_hash.is_none() {
            print_vga_register_state(assembled, "stage-1");
            let capture = assembled.capture_text(DeviceId::Vga).unwrap();
            let video::Frame::Text(grid) = &capture.frame else {
                panic!("mode 03h must capture text");
            };
            text_dimensions = Some((grid.cols(), grid.rows()));
            text_hash = Some(assembly::test_support::text_plane_sha256(&capture.frame));
        }
        if outcome == RunOutcome::Halted {
            break;
        }
    }
    print_vga_register_state(assembled, "end");
    VgaRun {
        outcome,
        text_hash,
        post,
        text_dimensions,
    }
}

#[test]
fn vga_measurement_does_not_inject_keys_during_controller_self_test() {
    let mut rom = vec![0; 32768];
    // IBM AT Technical Reference, TEST1 keyboard-controller self-test:
    // no unsolicited scancode may precede the expected 55h reply.
    // Delay past the measurement loop's former periodic input boundaries.
    let program = [
        0xFA, 0x31, 0xC0, 0x8E, 0xD8, 0xB0, 0xAE, 0xE6, 0x64, 0xB9, 0x10, 0x00, 0xBA, 0xFF, 0xFF,
        0x4A, 0x75, 0xFD, 0xE2, 0xF8, 0xB0, 0xAA, 0xE6, 0x64, 0xE4, 0x60, 0xA2, 0x00, 0x05, 0xFA,
        0xF4,
    ];
    rom[..program.len()].copy_from_slice(&program);
    rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, 0, 0, 0, 0xF8]);
    let config = assembly::bios(rom, None);
    let mut assembled = machine_file::assemble_with_observation(&config, &policy()).unwrap();
    let run = run_vga_program(&mut assembled);
    assert_eq!(run.outcome, RunOutcome::Halted);
    assert!(assembled.machine.cpu().retired() > 2_000_000);
    let mut reply = [0];
    assembled.machine.cpu().read_mem(0x500, &mut reply).unwrap();
    assert_eq!(reply, [0x55]);
}

#[test]
fn vga_register_dump_preserves_state_and_reports_scanout_geometry() {
    let mut bus = testkit::MockBus::new(Box::new(devices::Vga::new()));
    bus.port_write(0x3C2, 1, 0x67).unwrap();
    for (index, value) in [
        (1u8, 79u8),
        (7, 2),
        (9, 15),
        (0x12, 0x8F),
        (0x0C, 0x12),
        (0x0D, 0x34),
        (0x13, 40),
    ] {
        bus.port_write(0x3D4, 2, u32::from(index) | (u32::from(value) << 8))
            .unwrap();
    }
    bus.port_write(0x3C6, 1, 0x7F).unwrap();
    bus.tick(40_000);
    let before = bus.device().save();
    let vga = bus
        .device_mut()
        .as_any_mut()
        .unwrap()
        .downcast_mut::<devices::Vga>()
        .unwrap();
    let dump = vga_register_state(vga);
    assert!(dump.starts_with("misc=67 sequencer=["));
    for (label, count) in [
        ("sequencer", 5),
        ("crtc", 25),
        ("graphics", 9),
        ("attribute", 21),
    ] {
        let values = dump
            .split(&format!("{label}=["))
            .nth(1)
            .unwrap()
            .split(']')
            .next()
            .unwrap();
        assert_eq!(values.split(", ").count(), count, "{label}");
    }
    assert!(dump.contains("dac_mask=7f subsystem_3c3=01 adapter_46e8=08 setup_102=01"));
    assert!(dump.contains("cols=80 rows=25 char_height=16 start=1234 offset=28 row_stride=80"));
    assert_eq!(bus.device().save(), before);
}

fn assemble_boot_probe(program: Vec<u8>, cs: u16, ip: u16) -> assembly::AssembledMachine {
    let mut rom = vec![0; 32768];
    let [ip_low, ip_high] = ip.to_le_bytes();
    let [cs_low, cs_high] = cs.to_le_bytes();
    rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, ip_low, ip_high, cs_low, cs_high]);
    let mut config = assembly::bios(rom, None);
    for (base, bytes) in [
        (u64::from(cs) * 16 + u64::from(ip), program),
        (0x40, vec![0, 0x20, 0, 0]),
        (0x2000, vec![0xCF]),
    ] {
        config.images.push(assembly::LoadImage {
            base,
            source: ImageSource::Inline(bytes),
            kind: assembly::ImageKind::RamLoad,
            provides: vec![],
        });
    }
    machine_file::assemble_with_observation(&config, &policy()).unwrap()
}

#[test]
fn text_sampling_rejects_post_patterns_and_locations_outside_the_boot_sector() {
    let magic = [
        BOOT_STAGE_MAGIC[0],
        BOOT_STAGE_MAGIC[1],
        BOOT_STAGE_MAGIC[2],
        BOOT_STAGE_MAGIC[3],
        1,
    ];
    for (cs, ip, marker, captured) in [
        (0, 0x7C00, [1; 5], false),
        (0, 0x7C00, [0; 5], false),
        (0, 0x7C00, [0x55; 5], false),
        (0, 0x7C00, [0xAA; 5], false),
        (0, 0x7C00, [1, b'G', b'A', b'!', 1], false),
        (0x10, 0x7C00, magic, false),
        (0, 0x1000, magic, false),
        (0, 0x7E00, magic, false),
        (0, 0x7C00, [b'V', b'G', b'A', b'!', 2], false),
        (0, 0x7C00, magic, true),
        (0, 0x7DE0, magic, true),
    ] {
        let mut program = vec![0xFA, 0x31, 0xC0, 0x8E, 0xD8];
        for (offset, byte) in marker.into_iter().enumerate() {
            program.extend([0xC6, 0x06, offset as u8, 0x06, byte]);
        }
        program.push(0xF4);
        let mut assembled = assemble_boot_probe(program, cs, ip);
        let run = run_vga_program(&mut assembled);
        assert_eq!(run.outcome, RunOutcome::Halted);
        assert_eq!(
            run.text_hash.is_some(),
            captured,
            "cs={cs:04x} ip={ip:04x} marker={marker:02x?}"
        );
    }
}

#[test]
fn boot_program_publishes_magic_for_text_and_graphics_stages() {
    // An IRET-only INT 10h keeps this marker-protocol test independent of video firmware.
    let mut assembled = assemble_boot_probe(mode13_floppy()[..512].to_vec(), 0, 0x7C00);
    let run = run_vga_program(&mut assembled);
    assert_eq!(run.outcome, RunOutcome::Halted);
    assert!(
        run.text_hash.is_some(),
        "the actual boot program must expose its text stage"
    );
    let mut marker = [0; 5];
    assembled
        .machine
        .cpu()
        .read_mem(0x600, &mut marker)
        .unwrap();
    assert_eq!(&marker[..4], &BOOT_STAGE_MAGIC);
    assert_eq!(marker[4], 2);
    assert_eq!(boot_program_stage(assembled.machine.cpu()), Some(2));
    assert_eq!(assembled.machine.cpu().code_segment(), 0);
    assert!((0x7C00..0x7E00).contains(&assembled.machine.cpu().reg(CpuReg::Rip)));
}
