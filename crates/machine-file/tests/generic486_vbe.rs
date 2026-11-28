// SPDX-License-Identifier: AGPL-3.0-or-later
//! The shipped generic 486 board with a Bochs VBE card and the SeaVGABIOS
//! Bochs VBE build. VESA BIOS Extension (VBE) Core Functions Standard 3.0:
//! functions 00h, 01h, 02h, 03h, 05h and 08h; IBM VGA/XGA Technical
//! Reference, Video BIOS: INT 10h AH=0Eh teletype. The guest program is
//! `fixtures/seabios-vbe.asm`. See machine-files.md §Generic 486 ISA board
//! with Bochs VBE.

use assembly::{CpuEngine, DeviceId, DeviceKind, FirmwareStatus, JitBackendFactory};
use cpu::Cpu;
use std::path::{Path, PathBuf};
use video::{Frame, PixelFormat};

const CPU_REV: &str = "dd4316586f1459cec0618008b17817a0d26245a2";
const PROGRAM: &[u8] = include_bytes!("fixtures/seabios-vbe.bin");
const BOCHS_ROM_SHA256: &str = "609be4bab79bf84297c50ef80c070147a2bd71ae1d457e8329b3feb5ec59b8c3";

fn cranelift() -> JitBackendFactory {
    JitBackendFactory(|| Box::new(x86jit_cranelift::JitBackend::new()))
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The shipped machine file `name`, or `None` when a SeaBIOS fixture is
/// missing outside the gate.
fn shipped(name: &str) -> Option<assembly::MachineConfig> {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let root = repository();
    if [
        "bios-1.17.0-isa.bin",
        "build/vgabios-1.17.0-stdvga.bin",
        "build/vgabios-1.17.0-bochs.bin",
    ]
    .iter()
    .any(|name| !root.join("vendor/seabios").join(name).exists())
    {
        assert!(
            std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none(),
            "required SeaBIOS ISA fixture is missing"
        );
        eprintln!("skipping: SeaBIOS ISA fixture is missing");
        return None;
    }
    Some(machine_file::load(root.join("machines").join(name)).unwrap())
}

fn vbe_machine() -> Option<assembly::MachineConfig> {
    shipped("generic-486-seabios-vbe.toml")
}

/// The same retired-instruction allowance as the other generic 486 probes:
/// `steps` at the old one-per-PIT-clock rate, scaled to the board clock.
fn budget(steps: u64) -> u64 {
    let clock = assembly::BoardModel::Generic486Isa.default_clock();
    (steps * clock.retired_per_pit_clock).div_ceil(clock.retired_per_pit_clock_denominator)
}

fn add_floppy(config: &mut assembly::MachineConfig, program: &[u8]) {
    let mut floppy = vec![0; 1_474_560];
    floppy[..program.len()].copy_from_slice(program);
    config.devices.push(
        assembly::DeviceConfig::new(
            DeviceId::Fdc,
            DeviceKind::Fdc(assembly::FdcCfg { drive_slots: 2 }),
            assembly::FdcCfg::port_windows(false),
        )
        .with_irq(6)
        .with_dma(assembly::DmaSpec {
            controller: DeviceId::Dma,
            channel: 2,
        }),
    );
    config.drives.push(assembly::DriveConfig {
        media: None,
        kind: assembly::DriveKind::Floppy {
            geometry: assembly::FloppyGeometry::HD_1_44,
        },
        image: assembly::ImageSource::Inline(floppy),
        expected_sha256: None,
        controller: DeviceId::Fdc,
        unit: 0,
        write: assembly::WriteMode::ReadOnly,
    });
}

fn assemble(config: &assembly::MachineConfig, engine: CpuEngine) -> assembly::AssembledMachine {
    let mut config = config.clone();
    config.cpu.engine = engine;
    let policy = assembly::ObservationPolicy {
        routes: vec![
            assembly::ObservationRequest {
                device: DeviceId::Vga,
                output: assembly::OutputKind::TextScanout,
                byte_limit: 0,
            },
            assembly::ObservationRequest {
                device: DeviceId::Uart,
                output: assembly::OutputKind::Bytes(device::ByteStreamKind::SerialTx),
                byte_limit: 1 << 20,
            },
        ],
        ..Default::default()
    };
    let assembled = assembly::assemble_mounted_with_jit(
        &config,
        &policy,
        Default::default(),
        std::rc::Rc::new(machine_file::FileMount::new()),
        Some(cranelift()),
    )
    .unwrap();
    assert_eq!(assembled.machine.cpu().engine(), engine);
    assembled
}

fn serial(assembled: &assembly::AssembledMachine) -> String {
    let bytes = assembled
        .observation
        .drain_bytes(DeviceId::Uart, device::ByteStreamKind::SerialTx)
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn text_pixels(assembled: &assembly::AssembledMachine) -> Frame {
    let device = assembled
        .machine
        .bus()
        .device(assembled.device_index[&DeviceId::Vga])
        .unwrap();
    let state = device.planar_text_scanout().unwrap();
    Frame::Pixels(video::render_planar_text_pixels(&state))
}

/// SeaVGABIOS's "Removing mode" diagnostics, as mode numbers.
fn removed_modes(serial: &str) -> Vec<u16> {
    serial
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Removing mode "))
        .map(|mode| u16::from_str_radix(mode.trim(), 16).unwrap())
        .collect()
}

#[test]
fn generic486_vbe_shipped_file_has_listed_pinned_firmware_and_assembles() {
    let Some(config) = vbe_machine() else {
        return;
    };
    assert_eq!(config.board, assembly::BoardModel::Generic486Isa);
    assert!(config
        .devices
        .iter()
        .any(|d| matches!(&d.kind, DeviceKind::BochsVbe(cfg) if cfg.memory_mib == 16)));
    let unlisted = assembly::check_option_rom_catalogue(&config, Default::default()).unwrap();
    assert!(unlisted.is_empty(), "{unlisted:?}");
    let entry = config
        .board
        .firmware_catalogue()
        .iter()
        .find(|entry| entry.sha256 == BOCHS_ROM_SHA256)
        .unwrap();
    assert_eq!(entry.id, "seavgabios-1.17.0-unemu1-bochs");
    assert_eq!(entry.status, FirmwareStatus::Listed);
    assert_eq!(entry.size, 39424);
    let video = config
        .images
        .iter()
        .find(|image| image.base == 0xC0000)
        .unwrap();
    assert_eq!(video.source.known_len(), Some(39424));
    // The video ROM is longer than 32 KiB; SeaBIOS still places it at C0000h.
    let mut machine = machine_file::assemble(&config).unwrap();
    assert_eq!(
        machine.machine.run_bounded(Some(1)).unwrap(),
        machine::RunOutcome::RetiredCapReached
    );
}

/// What a diskless POST leaves: the text rows, the font-rendered screen
/// hash, the serial console and the retired count.
fn diskless_post(config: &assembly::MachineConfig) -> (Vec<String>, String, String, u64) {
    let mut assembled = assemble(config, CpuEngine::Interpreter);
    assembled
        .machine
        .run_bounded(Some(budget(10_000_000)))
        .unwrap();
    let serial = serial(&assembled);
    let Frame::Text(grid) = assembled.capture_text(DeviceId::Vga).unwrap().frame else {
        panic!("POST must leave a text screen");
    };
    let hash = assembly::test_support::graphics_frame_sha256(&text_pixels(&assembled));
    (
        grid.rows_as_strings(),
        hash,
        serial,
        assembled.machine.cpu().retired(),
    )
}

/// With the card's DISPI interface found, POST ends at the same text
/// screen as the standard-VGA board, rendered identically.
#[test]
fn generic486_vbe_diskless_post_text_matches_the_standard_vga_board() {
    let (Some(vbe), Some(stdvga)) = (vbe_machine(), shipped("generic-486-seabios.toml")) else {
        return;
    };
    let (vbe_rows, vbe_hash, vbe_serial, vbe_retired) = diskless_post(&vbe);
    let (std_rows, std_hash, std_serial, std_retired) = diskless_post(&stdvga);
    eprintln!("VBE serial console:\n{vbe_serial}");
    eprintln!("retired: bochs {vbe_retired}, stdvga {std_retired}");
    assert!(
        vbe_rows
            .iter()
            .any(|line| line.starts_with("No bootable device.")),
        "{vbe_rows:?}"
    );
    assert_eq!(vbe_rows, std_rows);
    assert_eq!(vbe_hash, std_hash);
    assert_eq!(
        vbe_hash,
        "a9ec051a7369ed33ca6cf8cc59b285469184ec326d0c8284d6eba6cf53cee99d"
    );
    assert!(vbe_serial.contains("Start SeaVGABIOS (version 1.17.0-unemupc-bochs)"));
    assert!(vbe_serial.contains("VBE DISPI: lfb_addr=e0000000, size 16 MB"));
    assert!(!vbe_serial.contains("No VBE DISPI interface detected"));
    assert!(std_serial.contains("Start SeaVGABIOS (version 1.17.0-unemupc-stdvga)"));
    // Only the modes wider than the card's 2560-pixel GETCAPS maximum go;
    // every other mode fits 16 MiB. Add 102h, 104h and 106h when the fork
    // release drops depth-4 modes.
    assert_eq!(
        removed_modes(&vbe_serial),
        [0x199, 0x19A, 0x19B, 0x19C, 0x19D, 0x19E]
    );
}

/// Everything the VBE program leaves for the host to check.
#[derive(PartialEq, Eq)]
struct VbeRun {
    /// Per stage marker: the retired count when the host first saw it and
    /// the frame captured then.
    stages: Vec<(u16, u64, Frame)>,
    /// Result words at 0500h–053Fh.
    results: Vec<u8>,
    /// The VBE controller information block at 1000h, 512 bytes.
    info: Vec<u8>,
    /// The mode information blocks for 101h, 111h and 142h.
    modes: Vec<Vec<u8>>,
    /// The mode list the controller block points to.
    mode_list: Vec<u16>,
    retired: u64,
}

fn read(assembled: &assembly::AssembledMachine, address: u64, len: usize) -> Vec<u8> {
    let mut bytes = vec![0; len];
    assembled
        .machine
        .cpu()
        .read_mem(address, &mut bytes)
        .unwrap();
    bytes
}

fn vbe_run(config: &assembly::MachineConfig, engine: CpuEngine) -> VbeRun {
    let mut config = config.clone();
    add_floppy(&mut config, PROGRAM);
    let mut assembled = assemble(&config, engine);
    let started = std::time::Instant::now();
    let mut stages: Vec<(u16, u64, Frame)> = Vec::new();
    let limit = budget(40_000_000);
    let mut cap = 0;
    let outcome = loop {
        cap += 10_000;
        assert!(
            cap <= limit,
            "the program must halt; stages {}",
            stages.len()
        );
        let outcome = assembled.machine.run_bounded(Some(cap)).unwrap();
        let marker = u16::from_le_bytes(read(&assembled, 0x500, 2).try_into().unwrap());
        if marker != 0 && stages.last().map(|stage| stage.0) != Some(marker) {
            let frame = assembled.capture_text(DeviceId::Vga).unwrap().frame;
            stages.push((marker, assembled.machine.cpu().retired(), frame));
        }
        if outcome != machine::RunOutcome::RetiredCapReached {
            break outcome;
        }
    };
    assert_eq!(outcome, machine::RunOutcome::Halted);
    let info = read(&assembled, 0x1000, 512);
    let pointer = u32::from_le_bytes(info[14..18].try_into().unwrap());
    let list_address = u64::from(pointer >> 16) * 16 + u64::from(pointer & 0xFFFF);
    let mut mode_list = Vec::new();
    for word in read(&assembled, list_address, 512).chunks(2) {
        let mode = u16::from_le_bytes([word[0], word[1]]);
        if mode == 0xFFFF {
            break;
        }
        mode_list.push(mode);
    }
    let run = VbeRun {
        stages,
        results: read(&assembled, 0x500, 0x40),
        info,
        modes: [0x1200, 0x1300, 0x1400]
            .map(|address| read(&assembled, address, 256))
            .to_vec(),
        mode_list,
        retired: assembled.machine.cpu().retired(),
    };
    eprintln!(
        "{engine:?}: retired {}; stages at {:?}; wall {:?}",
        run.retired,
        run.stages
            .iter()
            .map(|stage| (stage.0, stage.1))
            .collect::<Vec<_>>(),
        started.elapsed()
    );
    run
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn dword(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// A VBE 3.0 mode information block: attributes, bytes per scan line,
/// resolution, depth, memory model, the field sizes and positions, the
/// framebuffer address and the linear bytes per scan line.
fn check_mode_block(block: &[u8], depth: u8, model: u8, line: u16, fields: [u8; 8]) {
    // Supported, extended information, colour, graphics, not VGA, LFB.
    assert_eq!(word(block, 0x00) & 0xBB, 0xBB, "attributes");
    assert_eq!(word(block, 0x10), line, "BytesPerScanLine");
    assert_eq!((word(block, 0x12), word(block, 0x14)), (640, 480));
    assert_eq!(block[0x19], depth, "BitsPerPixel");
    assert_eq!(block[0x1B], model, "MemoryModel");
    assert_eq!(&block[0x1F..0x27], &fields, "mask sizes and positions");
    assert_eq!(dword(block, 0x28), 0xE000_0000, "PhysBasePtr");
    assert_eq!(word(block, 0x32), line, "LinBytesPerScanLine");
}

fn indexed(frame: &Frame) -> &video::IndexedPixels {
    let Frame::Pixels(pixels) = frame else {
        panic!("8 bpp presents an indexed frame");
    };
    assert_eq!((pixels.width, pixels.height), (640, 480));
    pixels
}

fn direct(frame: &Frame, format: PixelFormat, stride: u32) -> Vec<u32> {
    let Frame::Direct(pixels) = frame else {
        panic!("a direct-colour mode presents a direct-colour frame");
    };
    assert_eq!(
        (pixels.width, pixels.height, pixels.format, pixels.stride),
        (640, 480, format, stride)
    );
    pixels.to_xrgb8888().unwrap()
}

fn rgb565(n: u32) -> u32 {
    let expand5 = |v: u32| (v << 3) | (v >> 2);
    let expand6 = |v: u32| (v << 2) | (v >> 4);
    expand5(n >> 11) << 16 | expand6((n >> 5) & 63) << 8 | expand5(n & 31)
}

/// The BIOS's VBE functions on the card: controller and mode information,
/// 640×480 at 8, 16 and 32 bpp through the framebuffer, 8 bpp through the
/// banked window with 4F05h, the eight-bit DAC through 4F08h and teletype
/// in a direct-colour mode. Both CPU engines retire the same stream and
/// leave the same frames.
#[test]
fn generic486_vbe_bios_functions_drive_the_card_on_both_engines() {
    let Some(config) = vbe_machine() else {
        return;
    };
    let interpreted = vbe_run(&config, CpuEngine::Interpreter);
    let run = &interpreted;
    let results = &run.results;
    eprintln!("VBE mode list: {:X?}", run.mode_list);

    // 4F00h: VESA signature, version 3.0, 256 × 64 KiB of memory.
    assert_eq!(word(results, 0x02), 0x004F, "4F00h");
    assert_eq!(&run.info[0..4], b"VESA");
    assert_eq!(word(&run.info, 4), 0x0300);
    assert_eq!(word(&run.info, 0x12), 256);
    // The 640×480 modes at 8, 15, 16, 24 and 32 bpp are listed.
    for mode in [0x101, 0x110, 0x111, 0x112, 0x142] {
        assert!(run.mode_list.contains(&mode), "{mode:X}");
    }
    // The 4 bpp planar modes are still offered, although the card refuses
    // depth 4; the widest modes are gone. Flip this when the fork release
    // drops depth-4 modes.
    for mode in [0x102, 0x104, 0x106] {
        assert!(run.mode_list.contains(&mode), "{mode:X}");
    }
    assert!(!run.mode_list.contains(&0x199));

    // 4F01h for 640×480 at 8, 16 and 32 bpp.
    for offset in [0x04, 0x06, 0x08] {
        assert_eq!(word(results, offset), 0x004F, "4F01h at {offset:#x}");
    }
    check_mode_block(&run.modes[0], 8, 4, 640, [0; 8]);
    check_mode_block(&run.modes[1], 16, 6, 1280, [5, 11, 6, 5, 5, 0, 0, 0]);
    check_mode_block(&run.modes[2], 32, 6, 2560, [8, 16, 8, 8, 8, 0, 8, 24]);

    // 4F02h and 4F08h.
    for offset in [0x0A, 0x0C, 0x10, 0x14, 0x16, 0x18, 0x1A, 0x1E, 0x20, 0x24] {
        assert_eq!(word(results, offset), 0x004F, "result at {offset:#x}");
    }
    assert_eq!(
        (results[0x0E], results[0x12]),
        (8, 8),
        "DAC width set, read"
    );
    // 4F05h for banks 0–4, then read back.
    for bank in 0..5 {
        assert_eq!(word(results, 0x30 + 2 * bank), 0x004F, "4F05h bank {bank}");
    }
    assert_eq!(word(results, 0x1C), 4, "4F05h current bank");
    assert_eq!(word(results, 0x22), 0x4111, "4F03h mode and LFB flag");
    // 4F02h for the planar mode 102h reports success, but the card kept
    // the previous 16 bpp: the BIOS never reads the depth back. Flip this
    // when the fork release drops depth-4 modes (4F02h then fails).
    assert_eq!(word(results, 0x26), 16, "DISPI BPP after mode 102h");

    let markers: Vec<u16> = run.stages.iter().map(|stage| stage.0).collect();
    assert_eq!(markers, [1, 2, 3, 4, 5, 6]);

    // Stage 1: 8 bpp, pixel n holds n, eight-bit palette values intact.
    let pixels = indexed(&run.stages[0].2);
    assert!(pixels
        .indices
        .iter()
        .enumerate()
        .all(|(n, &index)| index == n as u8));
    assert_eq!(pixels.palette[1], [0x81, 0x42, 0x03]);
    assert_eq!(pixels.palette[255], [0xFF, 0xFE, 0xFD]);

    // Stage 2: 16 bpp through the framebuffer.
    let expected: Vec<u32> = (0..640 * 480).map(|n: u32| rgb565(n & 0xFFFF)).collect();
    assert!(direct(&run.stages[1].2, PixelFormat::Rgb565, 1280) == expected);

    // Stage 3: 32 bpp through the framebuffer.
    let expected: Vec<u32> = (0..640 * 480).map(|n: u32| n & 0xFF_FFFF).collect();
    assert!(direct(&run.stages[2].2, PixelFormat::Xrgb8888, 2560) == expected);

    // Stage 4: 8 bpp through the banked window.
    let pixels = indexed(&run.stages[3].2);
    assert!(pixels
        .indices
        .iter()
        .enumerate()
        .all(|(n, &index)| index == (n as u8) ^ ((n >> 16) as u8).wrapping_mul(0x25)));

    // Stage 5: "VBE" in white 8×16 glyphs in the first three cells, nothing
    // anywhere else. The BIOS maps attribute 0Fh to full white.
    let pixels = direct(&run.stages[4].2, PixelFormat::Rgb565, 1280);
    let mut lit = 0;
    for (n, &pixel) in pixels.iter().enumerate() {
        let (x, y) = (n % 640, n / 640);
        if x < 24 && y < 16 {
            assert!(pixel == 0 || pixel == 0xFF_FFFF, "({x}, {y}) {pixel:06x}");
            lit += usize::from(pixel != 0);
        } else {
            assert_eq!(pixel, 0, "({x}, {y}) outside the teletype cells");
        }
    }
    let per_cell: Vec<usize> = (0..3)
        .map(|cell| {
            (0..16 * 8)
                .filter(|i| pixels[(i / 8) * 640 + cell * 8 + i % 8] != 0)
                .count()
        })
        .collect();
    eprintln!("teletype lit pixels: {lit}, per cell {per_cell:?}");
    assert_eq!(per_cell, [38, 45, 40], "lit pixels of V, B and E");

    let compiled = vbe_run(&config, CpuEngine::Jit);
    assert_eq!(compiled.retired, interpreted.retired);
    assert_eq!(
        compiled
            .stages
            .iter()
            .map(|stage| (stage.0, stage.1))
            .collect::<Vec<_>>(),
        interpreted
            .stages
            .iter()
            .map(|stage| (stage.0, stage.1))
            .collect::<Vec<_>>()
    );
    assert!(compiled == interpreted, "the JIT run left different state");
}

#[test]
fn vbe_program_rebuilds_identically() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let temp = machine_file::test_support::TempDir::new("seabios-vbe-fixture");
    let binary = temp.path().join("seabios-vbe.bin");
    let output = std::process::Command::new("nasm")
        .args(["-f", "bin"])
        .arg(root.join("seabios-vbe.asm"))
        .arg("-o")
        .arg(&binary)
        .output();
    let output = match output {
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none() =>
        {
            eprintln!("skipping: nasm is missing");
            return;
        }
        other => other.unwrap(),
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(&binary).unwrap(), PROGRAM);
}
