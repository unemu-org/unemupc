// SPDX-License-Identifier: AGPL-3.0-or-later
//! File-backed generic ISA 386/486 board probes. Intel 80386 Programmer's Reference
//! Manual, §10.1 Initialization; SeaBIOS rel-1.17.0, POST and shadow.c reboot.
//! Intel486 DX Programmer's Reference Manual, §10.1 Initialization.
//! See machine-files.md §Generic 386 ISA board and §Generic 486 ISA board.

use assembly::{BoardModel, CmosRam, CpuChip, DeviceId, DeviceKind, FirmwareStatus};
use cpu::Cpu;
use std::path::PathBuf;

fn shipped(chip: CpuChip) -> Option<assembly::MachineConfig> {
    assert!(
        include_str!("../../../../Cargo.lock").contains("5a17667764080c2fe7a33d446584a3e175b7923d")
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("vendor/seabios/bios-1.17.0-isa.bin").exists() {
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
    let input = template(chip);
    Some(
        machine_file::lower(
            &machine_file::parse_toml(&path, &input).unwrap(),
            &machine_file::Origins::from_toml(&path, &input).unwrap(),
            &mut machine_file::FileImageReader,
        )
        .unwrap(),
    )
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
        (0x14, if chip == CpuChip::I80386 { 0x20 } else { 0x22 }),
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
    let directory: Vec<_> = (0..68)
        .map(|_| machine.machine.bus_mut().port_read(0x511, 1).unwrap() as u8)
        .collect();
    assert_eq!(&directory[..4], &1u32.to_be_bytes());
    assert_eq!(&directory[12..28], b"etc/sercon-port\0");
    let selector = u16::from_be_bytes([directory[8], directory[9]]);
    machine
        .machine
        .bus_mut()
        .port_write(0x510, 2, u32::from(selector))
        .unwrap();
    assert_eq!(machine.machine.bus_mut().port_read(0x511, 1).unwrap(), 0xF8);
    assert_eq!(machine.machine.bus_mut().port_read(0x511, 1).unwrap(), 0x03);
    assert_eq!(
        machine.machine.run_bounded(Some(1)).unwrap(),
        machine::RunOutcome::RetiredCapReached
    );
    assert_eq!(
        machine.machine.cpu().reg(cpu::CpuReg::Rdx),
        reset_signature(chip)
    );
}

pub fn toml_yaml_defaults_and_memory_limits_match(chip: CpuChip) {
    let Some(config) = shipped(chip) else {
        return;
    };
    let toml = template(chip);
    let file = "machines/generic-386-seabios.toml";
    let yaml = "schema: 1\nboard:\n  model: generic-386-isa\nmemory:\n  extended_kib: 15360\nfirmware:\n  bios:\n    image: bios.bin\ncard:\n  - kind: vga\n  - kind: uart-16550\n    irq: 4\n";
    struct Image(Vec<u8>);
    impl machine_file::ImageReader for Image {
        fn read(&mut self, _: &std::path::Path) -> std::io::Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }
    let bytes = match &config.images[0].source {
        assembly::ImageSource::Inline(bytes) => bytes.clone(),
        _ => panic!("fixture should be inline"),
    };
    let yaml = if chip == CpuChip::I80386 {
        yaml.to_owned()
    } else {
        yaml.replace("generic-386-isa", "generic-486-isa")
            + &format!("cpu:\n  model: {}\n", chip.name())
    };
    let yaml_config = machine_file::lower(
        &machine_file::parse_yaml("machine.yaml", &yaml).unwrap(),
        &machine_file::YamlOrigins::from_yaml("machine.yaml", &yaml).unwrap(),
        &mut Image(bytes.clone()),
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
            &mut Image(bytes.clone()),
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
    let result = machine.machine.run_bounded(Some(10_000_000));
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
    let result = assembled.machine.run_bounded(Some(10_000_000));
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
        assembled.machine.run_bounded(Some(10_000_000)).unwrap(),
        machine::RunOutcome::ResetRequested
    );
    let mut log = Vec::new();
    loop {
        let outcome = assembled.machine.run_bounded(Some(20_000_000)).unwrap();
        assert!(assembled.machine.cpu().retired() <= 20_000_000);
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
    struct Image(Vec<u8>);
    impl machine_file::ImageReader for Image {
        fn read(&mut self, _: &std::path::Path) -> std::io::Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }
    let assembly::ImageSource::Inline(bios) =
        &shipped.images[shipped.system_bios_image.unwrap()].source
    else {
        panic!("fixture should be inline")
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
            &mut Image(bios.clone()),
        )
        .unwrap();
        let mut floppy = vec![0; 1_474_560];
        floppy[..512].copy_from_slice(include_bytes!("../fixtures/seabios-int15.bin"));
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
        let mut assembled = machine_file::assemble(&config).unwrap();
        assembled.machine.enable_delivery_log().unwrap();
        let started = std::time::Instant::now();
        let outcome = assembled.machine.run_bounded(Some(10_000_000));
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
