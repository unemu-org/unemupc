// SPDX-License-Identifier: AGPL-3.0-or-later
//! File-backed generic ISA 386 board probes. Intel 80386 Programmer's Reference
//! Manual, §10.1 Initialization; SeaBIOS rel-1.17.0, POST and shadow.c reboot.
//! See machine-files.md §Generic 386 ISA board.

use assembly::{BoardModel, CmosRam, CpuChip, DeviceId, DeviceKind, FirmwareStatus};
use cpu::Cpu;
use std::path::PathBuf;

fn shipped() -> Option<assembly::MachineConfig> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("vendor/seabios/bios-1.17.0-isa.bin").exists() {
        assert!(
            std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none(),
            "required SeaBIOS ISA fixture is missing"
        );
        eprintln!("skipping: SeaBIOS ISA fixture is missing");
        return None;
    }
    Some(machine_file::load(root.join("machines/generic-386-seabios.toml")).unwrap())
}

#[test]
fn generic386_shipped_file_has_listed_pinned_firmware_and_assembles() {
    let Some(config) = shipped() else {
        return;
    };
    assert_eq!(config.board, BoardModel::Generic386Isa);
    assert_eq!(config.cpu.chip, CpuChip::I80386);
    let entry = config.board.firmware_catalogue()[0];
    assert_eq!(entry.id, "seabios-1.17.0-isa");
    assert_eq!(entry.status, FirmwareStatus::Listed);
    assert_eq!(
        entry.sha256,
        "07add88892bb47898ec712322870b8de7bdb6016dc7f4c3dd169d748be004a13"
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
    for (index, expected) in [(0x30, 0), (0x31, 0x3C), (0x5F, 0)] {
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
    assert_eq!(machine.machine.cpu().reg(cpu::CpuReg::Rdx), 0x0303);
}

#[test]
fn generic386_toml_yaml_defaults_and_memory_limits_match() {
    let Some(config) = shipped() else {
        return;
    };
    let toml = include_str!("../../../machines/generic-386-seabios.toml");
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
    let yaml_config = machine_file::lower(
        &machine_file::parse_yaml("machine.yaml", yaml).unwrap(),
        &machine_file::YamlOrigins::from_yaml("machine.yaml", yaml).unwrap(),
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

/// SeaBIOS rel-1.17.0, byteorder.h:15–17 and paravirt.c:703–705 use BSWAP
/// for the fw_cfg directory count. Its empty protected-mode IDT (misc.c:134–145)
/// cannot deliver #UD or #DF. Intel 80386 PRM §9.8.6 (Invalid Opcode) and §9.8.8
/// (Double Fault): shutdown requests a board CPU reset instead of retiring BSWAP.
fn assert_seabios_bswap_shutdown(machine: &machine::Machine<cpu::X86JitCpu>) {
    assert_eq!(machine.cpu().code_address(), (8, 0xF510B));
    assert_eq!(machine.cpu().retired(), 3582);
    let system = machine.cpu().diagnostic_state().system.unwrap();
    assert!(system.protected());
    assert_eq!(system.cpl, 0);
    assert_eq!(system.idtr.limit, 0);
    let mut bytes = [0; 2];
    machine.cpu().read_mem(0xF510B, &mut bytes).unwrap();
    assert_eq!(bytes, [0x0F, 0xC8]);
    eprintln!(
        "blocked: protected mode CPL 0, 0008:000F510B, linear 000F510B, \
         bytes {bytes:02X?}, BSWAP EAX; #UD -> #DF -> shutdown -> ResetRequested; \
         retired: 3582"
    );
}

#[test]
#[ignore = "SeaBIOS BSWAP EAX (0F C8) at 0008:000F510B raises #UD on the 386 and shuts down; POST is not qualified"]
fn generic386_seabios_reaches_diskless_post_end() {
    let Some(config) = shipped() else {
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
    if matches!(result, Ok(machine::RunOutcome::ResetRequested)) {
        assert_seabios_bswap_shutdown(&machine.machine);
        assert!(debug.starts_with(b"SeaBIOS (version 1.17.0"));
        assert!(debug.ends_with(b"Found QEMU fw_cfg\n"));
        assert!(serial.is_empty());
        return;
    }
    result.unwrap();
    // SeaBIOS rel-1.17.0, boot.c:965–980: boot_fail prints and waits for IRQs.
    for expected in ["SeaBIOS (version 1.17.0", "No bootable device."] {
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
}

#[test]
#[ignore = "SeaBIOS BSWAP EAX at 0008:000F510B shuts down before POST completes; reset/resume is not qualified"]
fn generic386_seabios_keyboard_reset_reaches_resume() {
    let Some(config) = shipped() else {
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
    let result = assembled.machine.run_bounded(Some(10_000_000));
    if matches!(result, Ok(machine::RunOutcome::ResetRequested)) {
        assert_seabios_bswap_shutdown(&assembled.machine);
        return;
    }
    result.unwrap();
    assembled
        .observation
        .drain_bytes(DeviceId::DebugConsole, device::ByteStreamKind::DebugConsole)
        .unwrap();
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
            break;
        }
        assert_eq!(
            outcome,
            machine::RunOutcome::ResetRequested,
            "SeaBIOS did not reach resume within the retired-instruction budget"
        );
    }
}
