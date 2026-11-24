// SPDX-License-Identifier: AGPL-3.0-or-later
//! File-backed generic ISA 386/486 board probes. Intel 80386 Programmer's Reference
//! Manual, §10.1 Initialization; SeaBIOS rel-1.17.0, POST and shadow.c reboot.
//! Intel486 DX Programmer's Reference Manual, §10.1 Initialization.
//! IBM VGA/XGA Technical Reference, Video BIOS and RAM Loadable Character Generator.
//! See machine-files.md §Generic 386 ISA board and §Generic 486 ISA board.

use assembly::{BoardModel, CmosRam, CpuChip, DeviceId, DeviceKind, FirmwareStatus};
use cpu::Cpu;
use std::path::PathBuf;

fn budget(chip: CpuChip, previous_steps: u64) -> u64 {
    let board = if chip == CpuChip::I80386 {
        BoardModel::Generic386Isa
    } else {
        BoardModel::Generic486Isa
    };
    let clock = board.default_clock();
    // Keep the old rate-one virtual-time allowance when nominal MHz increases
    // the instructions completed during SeaBIOS's fixed timer waits.
    (previous_steps * clock.retired_per_pit_clock).div_ceil(clock.retired_per_pit_clock_denominator)
}

fn shipped(chip: CpuChip) -> Option<assembly::MachineConfig> {
    assert!(
        include_str!("../../../../Cargo.lock").contains("5a17667764080c2fe7a33d446584a3e175b7923d")
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if ["bios-1.17.0-isa.bin", "build/vgabios-1.17.0-stdvga.bin"]
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
    let path = root.join("machines").join(if chip == CpuChip::I80386 {
        "generic-386-seabios.toml"
    } else {
        "generic-486-seabios.toml"
    });
    let mut config = machine_file::load_for_assembly(&path, Default::default())
        .unwrap()
        .config;
    config.cpu.chip = chip;
    Some(config)
}

pub fn shipped_file_has_listed_pinned_firmware_and_assembles(chip: CpuChip) {
    let Some(config) = shipped(chip) else {
        return;
    };
    assert_eq!(
        config.board,
        if chip == CpuChip::I80386 {
            BoardModel::Generic386Isa
        } else {
            BoardModel::Generic486Isa
        }
    );
    assert_eq!(config.cpu.chip, chip);
    let entry = config.board.firmware_catalogue()[0];
    assert_eq!(entry.id, "seabios-1.17.0-unemu1-isa");
    assert_eq!(entry.status, FirmwareStatus::Listed);
    assert_eq!(
        entry.sha256,
        "ad50fdb5e12a03dd5c17eac481a4183a7793b84252319eea02b8c878667c994f"
    );
    let bios = &config.images[config.system_bios_image.unwrap()];
    assert_eq!(bios.base, 0xE0000);
    assert_eq!(bios.source.known_len(), Some(131072));
    assert!(config
        .devices
        .iter()
        .any(|d| matches!(d.kind, DeviceKind::VgaRegisters(_))));
    assert!(config
        .devices
        .iter()
        .any(|d| matches!(&d.kind, DeviceKind::Rtc(cfg) if cfg.ram == CmosRam::Bytes128)));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/seabios");
    let input = template(chip)
        .replace(
            "../vendor/seabios/build/vgabios-1.17.0-stdvga.bin",
            "seavgabios-1.17.0-unemu1-stdvga",
        )
        .replace(
            "../vendor/seabios/bios-1.17.0-isa.bin",
            root.join("bios-1.17.0-isa.bin").to_str().unwrap(),
        );
    let temp = machine_file::test_support::TempDir::new("seavgabios-catalogue");
    let path = temp.path().join("catalogue.toml");
    std::fs::write(&path, input).unwrap();
    assert!(machine_file::load_for_assembly(&path, Default::default()).is_err());
    // Discovery intentionally refuses links outside its root; fixture caches
    // may be linked, so give this test its own copy inside the search root.
    let rom_dir = temp.path().join("roms");
    std::fs::create_dir(&rom_dir).unwrap();
    let video = config
        .images
        .iter()
        .find(|image| image.base == 0xC0000)
        .unwrap();
    let assembly::ImageSource::Inline(bytes) = &video.source else {
        panic!("fixture should be inline");
    };
    std::fs::write(rom_dir.join("vgabios-1.17.0-stdvga.bin"), bytes).unwrap();
    let catalogue_named =
        machine_file::load_for_assembly_with_rom_dirs(&path, Default::default(), &[rom_dir])
            .unwrap()
            .config;
    assert_eq!(catalogue_named, config);
    let mut machine = machine_file::assemble(&config).unwrap();
    let mut tail = [0; 16];
    machine
        .machine
        .cpu()
        .read_mem(0xFFFFFFF0, &mut tail)
        .unwrap();
    let assembly::ImageSource::Inline(bytes) = &bios.source else {
        panic!("fixture should be inline")
    };
    assert_eq!(&tail, &bytes[bytes.len() - 16..]);
    for (index, expected) in [
        (0x14, if chip == CpuChip::I80386 { 0 } else { 2 }),
        (0x30, 0),
        (0x31, 0x3C),
        (0x5F, 0),
    ] {
        machine
            .machine
            .bus_mut()
            .port_write(0x70, 1, index)
            .unwrap();
        assert_eq!(
            machine.machine.bus_mut().port_read(0x71, 1).unwrap(),
            expected
        );
    }
    // Signature and SERCON come through the real word-selector/byte-data bus.
    machine.machine.bus_mut().port_write(0x510, 2, 0).unwrap();
    let signature: Vec<_> = (0..4)
        .map(|_| machine.machine.bus_mut().port_read(0x511, 1).unwrap() as u8)
        .collect();
    assert_eq!(signature, b"QEMU");
    machine
        .machine
        .bus_mut()
        .port_write(0x510, 2, 0x19)
        .unwrap();
    let directory: Vec<_> = (0..132)
        .map(|_| machine.machine.bus_mut().port_read(0x511, 1).unwrap() as u8)
        .collect();
    assert_eq!(&directory[..4], &2u32.to_be_bytes());
    assert_eq!(&directory[12..28], b"etc/sercon-port\0");
    let selector = u16::from_be_bytes([directory[8], directory[9]]);
    machine
        .machine
        .bus_mut()
        .port_write(0x510, 2, u32::from(selector))
        .unwrap();
    assert_eq!(machine.machine.bus_mut().port_read(0x511, 1).unwrap(), 0xF8);
    assert_eq!(machine.machine.bus_mut().port_read(0x511, 1).unwrap(), 0x03);
    assert_eq!(&directory[76..98], b"vgaroms/vga-c0000.bin\0");
    let selector = u16::from_be_bytes([directory[72], directory[73]]);
    machine
        .machine
        .bus_mut()
        .port_write(0x510, 2, u32::from(selector))
        .unwrap();
    let video = config
        .images
        .iter()
        .find(|image| image.base == 0xC0000)
        .unwrap();
    let assembly::ImageSource::Inline(bytes) = &video.source else {
        panic!("inline video ROM");
    };
    let streamed: Vec<_> = (0..bytes.len())
        .map(|_| machine.machine.bus_mut().port_read(0x511, 1).unwrap() as u8)
        .collect();
    assert_eq!(&streamed, bytes);
    let mut shadow = [0; 16];
    machine
        .machine
        .cpu()
        .read_mem(0xC0000, &mut shadow)
        .unwrap();
    assert_eq!(
        shadow, [0; 16],
        "SeaBIOS owns deployment into initially empty shadow RAM"
    );
    let mut text_only = config.clone();
    for device in &mut text_only.devices {
        if matches!(device.kind, DeviceKind::VgaRegisters(_)) {
            device.kind = DeviceKind::Vga(assembly::VgaTextCfg {});
        }
    }
    assert!(matches!(
        assembly::check_option_rom_catalogue(&text_only, Default::default()),
        Err(assembly::ConfigError::FirmwareRequiresDevice {
            part: assembly::FirmwarePart::VgaRegisters,
            ..
        })
    ));
    let mut unknown = config.clone();
    let video = unknown
        .images
        .iter_mut()
        .find(|image| image.base == 0xC0000)
        .unwrap();
    let assembly::ImageSource::Inline(bytes) = &mut video.source else {
        unreachable!();
    };
    let previous = bytes.len() - 2;
    bytes[previous] = bytes[previous].wrapping_add(1);
    let last = bytes.len() - 1;
    bytes[last] = bytes[last].wrapping_sub(1);
    assert!(matches!(
        assembly::check_option_rom_catalogue(&unknown, Default::default()),
        Err(assembly::ConfigError::OptionRomNotListed { .. })
    ));
    assert_eq!(
        assembly::check_option_rom_catalogue(
            &unknown,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true
            }
        )
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        machine.machine.run_bounded(Some(1)).unwrap(),
        machine::RunOutcome::RetiredCapReached
    );
    assert_eq!(
        machine.machine.cpu().reg(cpu::CpuReg::Rdx),
        reset_signature(chip)
    );
}

pub fn seabios_rejects_relocated_video_rom_with_position(chip: CpuChip) {
    if shipped(chip).is_none() {
        return;
    }
    let temp = machine_file::test_support::TempDir::new("seavgabios-base");
    let path = temp.path().join("relocated.toml");
    let mut bytes = vec![0; 512];
    bytes[..3].copy_from_slice(&[0x55, 0xAA, 1]);
    bytes[511] = 0u8.wrapping_sub(bytes.iter().copied().fold(0u8, u8::wrapping_add));
    std::fs::write(temp.path().join("video.bin"), bytes).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pin = "sha256 = \"1adc6c0fa72b93c5d0659575c992b1bf6f64fd451c444b2a735e22f0a3d6e925\"\n";
    let input = template(chip)
        .replace(pin, "")
        .replace(
            "../vendor/seabios/build/vgabios-1.17.0-stdvga.bin",
            "video.bin",
        )
        .replace(
            "../vendor/seabios/bios-1.17.0-isa.bin",
            root.join("vendor/seabios/bios-1.17.0-isa.bin")
                .to_str()
                .unwrap(),
        );
    let options = assembly::AssemblyOptions {
        allow_unlisted_firmware: true,
    };
    let load = |input: &str| {
        std::fs::write(&path, input).unwrap();
        machine_file::load_with_options(&path, options)
    };
    assert!(
        load(&input).is_ok(),
        "checksum-valid unlisted video at C0000h is allowed for a run"
    );
    for base in ["0xC4000", "0xC0800"] {
        let relocated = input.replace("base = 0xC0000", &format!("base = {base}"));
        let error = load(&relocated).expect_err("relocated video ROM must be refused");
        let machine_file::LoadError::Lower(error) = error else {
            panic!("expected a positioned validation error");
        };
        assert_eq!(error.field, "firmware.option_roms[0].base");
        assert_eq!(error.origin.file, path);
        assert_eq!(
            error.origin.line,
            Some(
                relocated
                    .lines()
                    .position(|line| line.starts_with("base ="))
                    .unwrap()
                    + 1
            )
        );
        assert!(
            error.message.contains("SeaBIOS places it at C0000h"),
            "{error}"
        );
    }
}

pub fn toml_yaml_defaults_and_memory_limits_match(chip: CpuChip) {
    let Some(config) = shipped(chip) else {
        return;
    };
    let toml = template(chip);
    let file = "machines/generic-386-seabios.toml";
    let yaml = "schema: 1\nboard:\n  model: generic-386-isa\nmemory:\n  extended_kib: 15360\nfirmware:\n  bios:\n    image: bios.bin\n  option_roms:\n    - image: video.bin\n      base: 0xC0000\ncard:\n  - kind: vga\n  - kind: uart-16550\n    irq: 4\n";
    let yaml = if chip == CpuChip::I80386 {
        yaml.to_owned()
    } else {
        yaml.replace("generic-386-isa", "generic-486-isa")
            + &format!("cpu:\n  model: {}\n", chip.name())
    };
    let yaml_config = machine_file::lower(
        &machine_file::parse_yaml("machine.yaml", &yaml).unwrap(),
        &machine_file::YamlOrigins::from_yaml("machine.yaml", &yaml).unwrap(),
        &mut FirmwareImages(&config),
    )
    .unwrap();
    assert_eq!(yaml_config, config);
    for (extended, valid) in [(15360, true), (64512, true), (64576, false), (15361, false)] {
        let input = toml.replace(
            "extended_kib = 15360",
            &format!("extended_kib = {extended}"),
        );
        let result = machine_file::lower(
            &machine_file::parse_toml(file, &input).unwrap(),
            &machine_file::Origins::from_toml(file, &input).unwrap(),
            &mut FirmwareImages(&config),
        );
        assert_eq!(result.is_ok(), valid, "{extended}: {result:?}");
    }
}

pub fn seabios_reaches_diskless_post_end(chip: CpuChip) {
    let Some(config) = shipped(chip) else {
        return;
    };
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
                byte_limit: 65536,
            },
            assembly::ObservationRequest {
                device: DeviceId::DebugConsole,
                output: assembly::OutputKind::Bytes(device::ByteStreamKind::DebugConsole),
                byte_limit: 65536,
            },
        ],
        ..Default::default()
    };
    let mut machine = machine_file::assemble_with_observation(&config, &policy).unwrap();
    machine.machine.enable_delivery_log().unwrap();
    let started = std::time::Instant::now();
    let result = machine.machine.run_bounded(Some(budget(chip, 10_000_000)));
    let serial = machine
        .observation
        .drain_bytes(DeviceId::Uart, device::ByteStreamKind::SerialTx)
        .unwrap();
    let debug = machine
        .observation
        .drain_bytes(DeviceId::DebugConsole, device::ByteStreamKind::DebugConsole)
        .unwrap();
    eprintln!("serial console:\n{}", String::from_utf8_lossy(&serial));
    eprintln!("debug console:\n{}", String::from_utf8_lossy(&debug));
    if let Err(machine::MachineError::Cpu(error)) = &result {
        eprintln!("retired: {}", machine.machine.cpu().retired());
        if let Some(report) = machine.machine.cpu().stop_report(error) {
            eprintln!("{report}");
        }
    }
    result.unwrap();
    // The pinned BIOS emits this PCI-search notice before trying fw_cfg ROMs.
    let debug_text = String::from_utf8_lossy(&debug);
    assert!(debug_text.contains("Running option rom at c000:0003"));
    assert!(debug_text.contains("sercon: configuring in splitmode (vgabios c000:"));
    let serial_text = String::from_utf8_lossy(&serial);
    assert!(serial_text.contains("Start SeaVGABIOS (version 1.17.0-unemupc-stdvga)"));
    assert!(serial_text.contains("VGA stack allocated at"));
    assert!(!serial_text.contains("Failed to initialize VGA hardware"));
    let frame = machine.capture_text(DeviceId::Vga).unwrap().frame;
    let video::Frame::Text(grid) = &frame else {
        panic!("POST must expose text");
    };
    let lines = grid.rows_as_strings();
    eprintln!("POST screen: {}", lines.join("\n"));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("No bootable device.")),
        "POST text: {lines:?}"
    );
    let hash = assembly::test_support::graphics_frame_sha256(&text_pixels(&machine));
    eprintln!("POST font-rendered hash: {hash}");
    assert_eq!(
        hash,
        "a9ec051a7369ed33ca6cf8cc59b285469184ec326d0c8284d6eba6cf53cee99d"
    );
    eprintln!(
        "{} POST retired: {}; wall: {:?}",
        chip.name(),
        machine.machine.cpu().retired(),
        started.elapsed()
    );
    // SeaBIOS rel-1.17.0, boot.c:965–980: boot_fail prints and waits for IRQs.
    for expected in [
        "SeaBIOS (version 1.17.0",
        "Found QEMU fw_cfg",
        "RamSize: 0x01000000 [cmos]",
        "e820 map has 5 items:",
        "0: 0000000000000000 - 000000000009fc00 = 1 RAM",
        "1: 000000000009fc00 - 00000000000a0000 = 2 RESERVED",
        "2: 00000000000f0000 - 0000000000100000 = 2 RESERVED",
        "3: 0000000000100000 - 0000000001000000 = 1 RAM",
        "4: 00000000fffc0000 - 0000000100000000 = 2 RESERVED",
        "No apic - only the main cpu is present.",
        "No bootable device.",
    ] {
        assert!(
            serial
                .windows(expected.len())
                .any(|window| window == expected.as_bytes())
                || debug
                    .windows(expected.len())
                    .any(|window| window == expected.as_bytes()),
            "SeaBIOS did not emit {expected:?} within the retired-instruction budget"
        );
    }
    // The serial console on COM1 must carry the boot failure on its own, so a
    // UART regression cannot hide behind the debug console.
    let marker = b"No bootable device.";
    assert!(
        serial.windows(marker.len()).any(|window| window == marker),
        "SeaBIOS did not print the boot failure on the COM1 serial console"
    );
}

pub fn seabios_keyboard_reset_reaches_resume(chip: CpuChip) {
    let Some(config) = shipped(chip) else {
        return;
    };
    let policy = assembly::ObservationPolicy {
        routes: vec![assembly::ObservationRequest {
            device: DeviceId::DebugConsole,
            output: assembly::OutputKind::Bytes(device::ByteStreamKind::DebugConsole),
            byte_limit: 65536,
        }],
        ..Default::default()
    };
    let mut assembled = machine_file::assemble_with_observation(&config, &policy).unwrap();
    assembled.machine.enable_delivery_log().unwrap();
    let started = std::time::Instant::now();
    let result = assembled
        .machine
        .run_bounded(Some(budget(chip, 10_000_000)));
    result.unwrap();
    let post = assembled
        .observation
        .drain_bytes(DeviceId::DebugConsole, device::ByteStreamKind::DebugConsole)
        .unwrap();
    assert!(String::from_utf8_lossy(&post).contains("No bootable device."));
    eprintln!(
        "before reset retired: {}",
        assembled.machine.cpu().retired()
    );
    // SeaBIOS rel-1.17.0, resume.c handle_resume: CMOS 0Fh=0 selects reboot.
    assembled
        .machine
        .bus_mut()
        .port_write(0x70, 1, 0x0F)
        .unwrap();
    assembled.machine.bus_mut().port_write(0x71, 1, 0).unwrap();
    assembled
        .machine
        .bus_mut()
        .port_write(0x64, 1, 0xFE)
        .unwrap();
    assert_eq!(
        assembled
            .machine
            .run_bounded(Some(budget(chip, 10_000_000)))
            .unwrap(),
        machine::RunOutcome::ResetRequested
    );
    let mut log = Vec::new();
    loop {
        let outcome = assembled
            .machine
            .run_bounded(Some(budget(chip, 20_000_000)))
            .unwrap();
        assert!(assembled.machine.cpu().retired() <= budget(chip, 20_000_000));
        log.extend(
            assembled
                .observation
                .drain_bytes(DeviceId::DebugConsole, device::ByteStreamKind::DebugConsole)
                .unwrap(),
        );
        if log
            .windows(b"In resume (status=0)".len())
            .any(|window| window == b"In resume (status=0)")
        {
            eprintln!(
                "{} resume retired: {}; virtual time: {}; wall: {:?}",
                chip.name(),
                assembled.machine.cpu().retired(),
                assembled.machine.virtual_time(),
                started.elapsed()
            );
            break;
        }
        assert_eq!(
            outcome,
            machine::RunOutcome::ResetRequested,
            "SeaBIOS did not reach resume within the retired-instruction budget"
        );
    }
}

/// SeaBIOS rel-1.17.0, malloc.c calcRamSize and system.c handle_1588,
/// handle_15e801 and handle_15e820 derive legacy sizes from the E820 map.
/// A synthetic floppy boot sector queries the installed handlers after POST.
pub fn seabios_int15_memory_matches_e820(chip: CpuChip) {
    let Some(shipped) = shipped(chip) else {
        return;
    };
    for extended in [15360u32, 3072] {
        let file = "machines/generic-386-seabios.toml";
        let input = template(chip).replace(
            "extended_kib = 15360",
            &format!("extended_kib = {extended}"),
        );
        let mut config = machine_file::lower(
            &machine_file::parse_toml(file, &input).unwrap(),
            &machine_file::Origins::from_toml(file, &input).unwrap(),
            &mut FirmwareImages(&shipped),
        )
        .unwrap();
        add_floppy(&mut config, include_bytes!("../fixtures/seabios-int15.bin"));
        let mut assembled = machine_file::assemble(&config).unwrap();
        assembled.machine.enable_delivery_log().unwrap();
        let started = std::time::Instant::now();
        let outcome = assembled
            .machine
            .run_bounded(Some(budget(chip, 10_000_000)));
        eprintln!(
            "{} INT 15h {extended} KiB extended: {outcome:?}; retired: {}; wall: {:?}",
            chip.name(),
            assembled.machine.cpu().retired(),
            started.elapsed()
        );
        assert_eq!(outcome.unwrap(), machine::RunOutcome::Halted);
        let mut results = [0; 20];
        assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut results)
            .unwrap();
        let word = |offset| u16::from_le_bytes(results[offset..offset + 2].try_into().unwrap());
        assert_eq!(word(18), 0xCAFE, "guest did not complete INT 15h calls");
        let end = usize::from(word(16));
        assert!((0x624..=0xA80).contains(&end));
        assert_eq!((end - 0x600) % 36, 0);
        let mut entries = vec![0; end - 0x600];
        assembled
            .machine
            .cpu()
            .read_mem(0x600, &mut entries)
            .unwrap();
        let mut map = Vec::new();
        let count = entries.len() / 36;
        for (index, record) in entries.as_chunks::<36>().0.iter().enumerate() {
            let dword = |offset| u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
            assert_eq!(dword(20), 0x534D4150);
            assert_eq!(
                dword(24),
                if index + 1 == count {
                    0
                } else {
                    index as u32 + 1
                }
            );
            assert_eq!(dword(28), 20);
            assert_eq!(record[32] & 1, 0, "E820 returned CF");
            let start = u64::from_le_bytes(record[..8].try_into().unwrap());
            let size = u64::from_le_bytes(record[8..16].try_into().unwrap());
            map.push((start, size, dword(16)));
        }
        eprintln!("E820: {map:X?}; legacy words: {results:02X?}");
        let ram_end = map
            .iter()
            .filter(|entry| entry.2 == 1)
            .map(|entry| entry.0 + entry.1)
            .max()
            .unwrap();
        let configured_end = (u64::from(extended) + 1024) * 1024;
        assert_eq!(
            map,
            vec![
                (0, 0x9FC00, 1),
                (0x9FC00, 0x400, 2),
                (0xF0000, 0x10000, 2),
                (0x100000, configured_end - 0x100000, 1),
                (0xFFFC0000, 0x40000, 2),
            ]
        );
        assert_eq!(ram_end, configured_end);
        let expected_kib = ((ram_end - 1024 * 1024) / 1024) as u16;
        assert_eq!(word(0), expected_kib);
        assert_eq!(word(2) & 1, 0, "AH=88h returned CF");
        assert_eq!(
            [word(4), word(6), word(8), word(10)],
            [expected_kib, 0, expected_kib, 0]
        );
        assert_eq!(word(12) & 1, 0, "E801h returned CF");
        for index in [0x30, 0x31] {
            assembled
                .machine
                .bus_mut()
                .port_write(0x70, 1, index)
                .unwrap();
            assert_eq!(assembled.machine.bus_mut().port_read(0x71, 1).unwrap(), 0);
        }
    }
}

struct FirmwareImages<'a>(&'a assembly::MachineConfig);
impl machine_file::ImageReader for FirmwareImages<'_> {
    fn read(&mut self, path: &std::path::Path) -> std::io::Result<Vec<u8>> {
        let image = if matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("vgabios-1.17.0-stdvga.bin" | "seavgabios-1.17.0-unemu1-stdvga" | "video.bin")
        ) {
            self.0
                .images
                .iter()
                .find(|image| image.base == 0xC0000)
                .unwrap()
        } else if matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("bios-1.17.0-isa.bin" | "bios.bin")
        ) {
            &self.0.images[self.0.system_bios_image.unwrap()]
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "unknown test image",
            ));
        };
        let assembly::ImageSource::Inline(bytes) = &image.source else {
            panic!("fixture should be inline")
        };
        Ok(bytes.clone())
    }
}

fn add_floppy(config: &mut assembly::MachineConfig, sector: &[u8; 512]) {
    let mut floppy = vec![0; 1_474_560];
    floppy[..512].copy_from_slice(sector);
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

fn text_pixels(assembled: &assembly::AssembledMachine) -> video::Frame {
    let device = assembled
        .machine
        .bus()
        .device(assembled.device_index[&DeviceId::Vga])
        .unwrap();
    let state = device.planar_text_scanout().unwrap();
    video::Frame::Pixels(video::render_planar_text_pixels(&state))
}

pub fn seabios_int10_modes_have_golden_frames(chip: CpuChip) {
    let Some(mut config) = shipped(chip) else {
        return;
    };
    add_floppy(&mut config, include_bytes!("../fixtures/seabios-int10.bin"));
    let policy = assembly::ObservationPolicy {
        routes: vec![assembly::ObservationRequest {
            device: DeviceId::Vga,
            output: assembly::OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    };
    let mut assembled = machine_file::assemble_with_observation(&config, &policy).unwrap();
    let started = std::time::Instant::now();
    let mut marker = [0; 2];
    for cap in (10_000..=budget(chip, 10_000_000)).step_by(10_000) {
        assembled.machine.run_bounded(Some(cap)).unwrap();
        assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut marker)
            .unwrap();
        if marker == [1, 0] {
            break;
        }
    }
    assert_eq!(marker, [1, 0], "guest did not reach mode 3 text capture");
    let text = assembled.capture_text(DeviceId::Vga).unwrap().frame;
    let video::Frame::Text(grid) = &text else {
        panic!("mode 3 must expose text");
    };
    assert_eq!((grid.cols(), grid.rows()), (80, 25));
    assert_eq!(grid.rows_as_strings()[0], "SeaVGABIOS INT 10h mode 3");
    assert!(grid.rows_as_strings()[1..].iter().all(String::is_empty));
    let text_hash = assembly::test_support::graphics_frame_sha256(&text_pixels(&assembled));
    assert_eq!(
        text_hash,
        "31c39840f5599a9a16471b247e72a01d2c8092d86ba0350c5a0f354f5b4fd231"
    );
    eprintln!(
        "{} INT 10h text hash: {text_hash}; retired: {}; wall: {:?}",
        chip.name(),
        assembled.machine.cpu().retired(),
        started.elapsed()
    );
    assert_eq!(
        assembled
            .machine
            .run_bounded(Some(budget(chip, 10_000_000)))
            .unwrap(),
        machine::RunOutcome::Halted
    );
    assembled
        .machine
        .cpu()
        .read_mem(0x500, &mut marker)
        .unwrap();
    assert_eq!(marker, [2, 0]);
    let graphics = assembled.capture_text(DeviceId::Vga).unwrap().frame;
    let video::Frame::Pixels(pixels) = &graphics else {
        panic!("mode 13h must expose pixels");
    };
    assert_eq!((pixels.width, pixels.height), (320, 200));
    for (offset, index) in pixels.indices.iter().enumerate() {
        assert_eq!(*index, offset as u8, "mode 13h pixel {offset}");
    }
    let graphics_hash = assembly::test_support::graphics_frame_sha256(&graphics);
    assert_eq!(
        assembled.machine.cpu().retired(),
        if chip == CpuChip::I80386 {
            8_528_672
        } else {
            11_102_827
        }
    );
    assert_eq!(
        graphics_hash,
        "825d409a82c363538f60248d866bb8462c28eb7ebdb0499bbd0188c6d0740f27"
    );
    eprintln!(
        "{} INT 10h graphics hash: {graphics_hash}; retired: {}; wall: {:?}",
        chip.name(),
        assembled.machine.cpu().retired(),
        started.elapsed()
    );
}

fn reset_signature(chip: CpuChip) -> u64 {
    match chip {
        CpuChip::I80486Dx => 0x0403,
        CpuChip::I80486DxCpuid => 0x0410,
        _ => 0x0303,
    }
}
fn template(chip: CpuChip) -> String {
    if chip == CpuChip::I80386 {
        include_str!("../../../../machines/generic-386-seabios.toml").into()
    } else {
        include_str!("../../../../machines/generic-486-seabios.toml").replace(
            "model = \"80486dx\"",
            &format!("model = \"{}\"", chip.name()),
        )
    }
}

fn freedos_config(chip: CpuChip, ide: bool) -> Option<assembly::MachineConfig> {
    shipped(chip)?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["freedos/freedos.img", "hdd/fat16.img"] {
        if name.starts_with("hdd/") && !ide {
            continue;
        }
        if !root.join("vendor").join(name).exists() {
            assert!(
                std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none(),
                "required storage fixture is missing: {name}"
            );
            eprintln!("skipping: storage fixture is missing: {name}");
            return None;
        }
    }
    let bits = if chip == CpuChip::I80386 { 386 } else { 486 };
    let storage = if ide { "ide" } else { "freedos" };
    let path = root.join(format!("machines/generic-{bits}-{storage}.toml"));
    let mut config = machine_file::load_for_assembly(path, Default::default())
        .unwrap()
        .config;
    config.cpu.chip = chip;
    Some(config)
}

/// IBM Personal Computer AT Technical Reference, System Board, CMOS RAM
/// Information: drive-type nibbles at 10h and drive count at 14h. SeaBIOS
/// rel-1.17.0, floppy.c floppy_setup consumes those same type nibbles.
pub fn seabios_storage_configuration(chip: CpuChip) {
    for ide in [false, true] {
        let Some(mut config) = freedos_config(chip, ide) else {
            return;
        };
        config.validate().unwrap();
        let fdc = config
            .devices
            .iter()
            .find(|d| d.id == DeviceId::Fdc)
            .unwrap();
        assert_eq!(fdc.irq, Some(6));
        assert_eq!(
            fdc.dma,
            Some(assembly::DmaSpec {
                controller: DeviceId::Dma,
                channel: 2
            })
        );
        assert!(config
            .devices
            .iter()
            .any(|d| d.id == DeviceId::Dma && matches!(d.kind, DeviceKind::Dma(_))));
        let draft = assembly::capabilities::Draft::new(config.clone());
        let descriptor = assembly::capabilities::describe(&draft, "card[fdc-765].kind").unwrap();
        let assembly::capabilities::Domain::Values(kinds) = descriptor.domain else {
            panic!("expected card kinds");
        };
        for kind in ["fdc-765", "ide"] {
            assert!(kinds.contains(&assembly::capabilities::Value::Name(kind.into())));
        }
        let fpu = if chip == CpuChip::I80386 { 0 } else { 2 };
        let seed = assembly::derive_cmos_layout(&config).unwrap();
        assert_eq!((seed[0x10], seed[0x14]), (0x40, 1 | fpu));
        assert_eq!((seed[0x3D], seed[0x38]), (0x21, 0));
        assert_eq!(
            (seed[0x12], seed[0x19], seed[0x39]),
            if ide { (0xF0, 47, 0) } else { (0, 0, 0) }
        );
        let mut machine = machine_file::assemble(&config).unwrap();
        for index in [0x10u8, 0x14, 0x12, 0x19, 0x3D, 0x38, 0x39] {
            machine
                .machine
                .bus_mut()
                .port_write(0x70, 1, u32::from(index))
                .unwrap();
            assert_eq!(
                machine.machine.bus_mut().port_read(0x71, 1).unwrap(),
                u32::from(seed[usize::from(index)])
            );
        }
        // The second slot must contribute its own type and the equipment count.
        let mut second = config.drives[0].clone();
        second.unit = 1;
        second.kind = assembly::DriveKind::Floppy {
            geometry: assembly::FloppyGeometry::HD_1_2,
        };
        config.drives.push(second);
        let seed = assembly::derive_cmos_layout(&config).unwrap();
        assert_eq!((seed[0x10], seed[0x14]), (0x42, 0x41 | fpu));
        config
            .drives
            .retain(|d| !matches!(d.kind, assembly::DriveKind::Floppy { .. }));
        let seed = assembly::derive_cmos_layout(&config).unwrap();
        assert_eq!((seed[0x10], seed[0x14]), (0, fpu));
    }
}

fn freedos_text(assembled: &assembly::AssembledMachine) -> String {
    let video::Frame::Text(grid) = assembled.capture_text(DeviceId::Vga).unwrap().frame else {
        // Earlier instruction polls can precede SeaVGABIOS's text-mode setup.
        return String::new();
    };
    let text = grid.rows_as_strings().join("\n");
    if text.contains("A:\\>") {
        assert_eq!((grid.cols(), grid.rows()), (80, 25));
    }
    text
}

fn text_sha256(text: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(text.as_bytes()))
}

/// PC16550D datasheet, transmitter operation and character timing.
fn serial_until(
    assembled: &mut assembly::AssembledMachine,
    chip: CpuChip,
    needle: &[u8],
) -> Vec<u8> {
    use device::clock::{ClockDomain, Ratio, TICKS_PER_PIT_CLOCK};
    let clocks = ClockDomain::from_hz(Ratio::new(1000, 1))
        .ticks(1)
        .div_ceil(TICKS_PER_PIT_CLOCK);
    let idle = assembled.machine.idle_limit();
    assembled.machine.set_idle_limit(Some(clocks));
    let mut bytes = Vec::new();
    // VGA stores are immediate, whereas COM1 still has a timed shift register.
    // Allow up to 100 ms of guest time for the final queued characters.
    for _ in 0..100 {
        bytes.extend(
            assembled
                .observation
                .drain_bytes(DeviceId::Uart, device::ByteStreamKind::SerialTx)
                .unwrap(),
        );
        if bytes.windows(needle.len()).any(|window| window == needle) {
            assembled.machine.set_idle_limit(idle);
            return bytes;
        }
        let cap = assembled.machine.virtual_time() + budget(chip, clocks);
        assembled.machine.run_bounded(Some(cap)).unwrap();
    }
    panic!(
        "serial output did not finish: {}",
        String::from_utf8_lossy(&bytes)
    );
}

/// SeaBIOS rel-1.17.0, floppy.c floppy_read and ata.c ata_scan; ATA-2,
/// §§6.1–6.2, legacy task file. The immutable fixtures use the same media
/// pins as the AT profiles, exercising BIOS disk services without XTIDE.
pub fn seabios_freedos_reaches_prompt(chip: CpuChip, ide: bool) {
    let Some(config) = freedos_config(chip, ide) else {
        return;
    };
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
                byte_limit: 65536,
            },
            assembly::ObservationRequest {
                device: DeviceId::DebugConsole,
                output: assembly::OutputKind::Bytes(device::ByteStreamKind::DebugConsole),
                byte_limit: 65536,
            },
        ],
        ..Default::default()
    };
    let mut assembled = machine_file::assemble_with_observation(&config, &policy).unwrap();
    assembled.machine.enable_delivery_log().unwrap();
    let started = std::time::Instant::now();
    let mut entered = false;
    let mut prompt = None;
    for cap in (100_000..=budget(chip, 20_000_000)).step_by(100_000) {
        let result = assembled.machine.run_bounded(Some(cap));
        if let Err(machine::MachineError::Cpu(error)) = &result {
            eprintln!(
                "CPU BOUNDARY: retired={}\n{}",
                assembled.machine.cpu().retired(),
                freedos_text(&assembled)
            );
            if let Some(report) = assembled.machine.cpu().stop_report(error) {
                eprintln!("{report}");
            }
            eprintln!("deliveries: {:#?}", assembled.machine.recent_deliveries());
        }
        result.unwrap();
        let screen = freedos_text(&assembled);
        if screen.contains("A:\\>") {
            prompt = Some((cap, screen));
            break;
        }
        if !entered && screen.contains("Enter new date") {
            let keyboard = assembled.device_index[&DeviceId::Keyboard];
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x1c, 0x9c, 0x1c, 0x9c]);
            entered = true;
        }
    }
    let (cap, screen) = prompt.unwrap_or_else(|| {
        panic!(
            "FreeDOS prompt budget exhausted\n{}",
            freedos_text(&assembled)
        )
    });
    eprintln!(
        "{} FreeDOS ide={ide}: retired: {}; wall: {:?}; text SHA-256: {}\n{screen}",
        chip.name(),
        assembled.machine.cpu().retired(),
        started.elapsed(),
        text_sha256(&screen)
    );
    assert_eq!(
        text_sha256(&screen),
        match (chip == CpuChip::I80386, ide) {
            (true, false) => "0fbdd8acac3037c89d098ff1a21d43bd767b0ca7a81c18b1ecaebb70907282b2",
            (true, true) => "998d3feaae41ac8dd1c9ae964923cc87d4b6f1e353c45c02646780db971920d5",
            (false, false) => "8456147fac982428ea6abeff04a22770f703af09e4ecfb82f087034826177a7e",
            (false, true) => "84c6ed34f1a68f63adcaf3dd7e25b1ba7d0ead0f299307e80b207ebc8bfdfc7e",
        }
    );
    assert!(screen.contains("FreeDOS kernel 2044"));
    assert!(screen.contains("FreeCom version 0.86"));
    assert_eq!(
        assembled.machine.cpu().retired(),
        match (chip == CpuChip::I80386, ide) {
            (true, false) => 10_718_776,
            (true, true) => 10_716_489,
            (false, false) => 13_218_823,
            (false, true) => 13_316_536,
        }
    );
    let serial = serial_until(&mut assembled, chip, b"A:\\>");
    let debug = assembled
        .observation
        .drain_bytes(DeviceId::DebugConsole, device::ByteStreamKind::DebugConsole)
        .unwrap();
    let serial = String::from_utf8_lossy(&serial);
    let debug = String::from_utf8_lossy(&debug);
    eprintln!("serial:\n{serial}\ndebug:\n{debug}");
    assert!(serial.contains("FreeDOS kernel 2044"));
    assert!(serial.contains("A:\\>"));
    assert!(debug.contains("Booting from Floppy..."));
    if !ide {
        return;
    }
    assert!(debug.contains("ata0-0: UNEMU ATA-2 PIO DISK ATA-0 Hard-Disk (20 MiBytes)"));
    assert!(!debug.contains("Booting from Hard Disk..."));
    let keyboard = assembled.device_index[&DeviceId::Keyboard];
    assembly::inject_scancodes(
        &mut assembled.machine,
        keyboard,
        &[
            0x20, 0xa0, 0x17, 0x97, 0x13, 0x93, 0x39, 0xb9, 0x2e, 0xae, 0x2a, 0x27, 0xa7, 0xaa,
            0x1c, 0x9c,
        ],
    );
    for dir_cap in ((cap + 100_000)..=budget(chip, 20_000_000)).step_by(100_000) {
        assembled.machine.run_bounded(Some(dir_cap)).unwrap();
        let text = freedos_text(&assembled);
        if text.contains("MARKER") && text.lines().any(|line| line.trim() == "A:\\>") {
            let retired = assembled.machine.cpu().retired();
            eprintln!(
                "{} DIR C: retired: {}; wall: {:?}; text SHA-256: {}\n{text}",
                chip.name(),
                assembled.machine.cpu().retired(),
                started.elapsed(),
                text_sha256(&text)
            );
            assert!(text.contains("Volume in drive C is UNEMUHDD"));
            assert!(text
                .lines()
                .any(|line| line.split_whitespace().take(3).eq(["MARKER", "TXT", "29"])));
            assert_eq!(
                text_sha256(&text),
                if chip == CpuChip::I80386 {
                    "ea7570aa1cfd5cd6419330d97d46169a947519686e2d92cb26ec099e77315165"
                } else {
                    "69417229e1b3356358125a6f9fdbfe8ebb65112b7847f18f32d6b13912bc59be"
                }
            );
            let serial = serial_until(&mut assembled, chip, b"A:\\>");
            let serial = String::from_utf8_lossy(&serial);
            assert!(serial.contains("MARKER"));
            assert!(serial.contains("A:\\>"));
            assert_eq!(
                retired,
                if chip == CpuChip::I80386 {
                    13_491_318
                } else {
                    16_391_365
                }
            );
            return;
        }
    }
    panic!("DIR C: budget exhausted\n{}", freedos_text(&assembled));
}
