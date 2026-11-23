// SPDX-License-Identifier: AGPL-3.0-or-later
//! Generic ISA board integration probes. Intel 80386 Programmer's Reference
//! Manual, §2.1 Memory Organization and §10.1 Initialization; SeaBIOS rel-1.17.0,
//! shadow.c qemu_prep_reset. See machine-files.md §Generic 386 ISA board.

use super::*;
use crate::{BoardModel, CpuChip, PortWindow, RamRegion};
use cpu::{Cpu, CpuError, CpuReg, DmaAccess};

fn board() -> MachineConfig {
    let mut config = crate::bios(vec![0xA7; 0x20000], None);
    let uart = config
        .devices
        .iter()
        .find(|d| d.id == DeviceId::Uart)
        .unwrap()
        .clone();
    config.images[0].base = 0xE0000;
    config.board = BoardModel::Generic386Isa;
    config.cpu.chip = CpuChip::I80386;
    config.cpu.fpu = crate::FpuChip::None;
    config.devices = config.board.motherboard_devices();
    config.devices.push(uart);
    config.drives.clear();
    config.memory.address_space_bytes = 0x1000000;
    config.memory.mmio.clear();
    config.memory.ram = vec![
        RamRegion::new(0, 0xA0000),
        RamRegion::new(0xC0000, 0x40000),
        RamRegion::new(0x100000, 15 << 20),
    ];
    config
}

#[test]
fn generic386_board_accepts_its_devices_and_requires_fixed_wiring() {
    let config = board();
    config.validate().unwrap();
    assert_eq!(config.board.class(), Some(crate::MachineClass::At386));
    assert_eq!(config.cpu.chip.physical_address_bits(), 32);
    for id in [
        DeviceId::SysCtrlPortA,
        DeviceId::DebugConsole,
        DeviceId::FwCfg,
    ] {
        let mut missing = config.clone();
        missing.devices.retain(|d| d.id != id);
        assert!(
            matches!(missing.validate(), Err(ConfigError::BoardDeviceMissing { device, .. }) if device == id)
        );
    }
    let mut wrong_cpu = config.clone();
    wrong_cpu.cpu.chip = CpuChip::I80286;
    assert!(matches!(
        wrong_cpu.validate(),
        Err(ConfigError::BoardConfiguration { .. })
    ));
    let mut wrong_port = config.clone();
    wrong_port
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::DebugConsole)
        .unwrap()
        .ports = vec![PortWindow::single(0xE9)];
    assert!(wrong_port.validate().is_err());
    let mut small_cmos = config;
    if let DeviceKind::Rtc(cfg) = &mut small_cmos
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Rtc)
        .unwrap()
        .kind
    {
        cfg.ram = crate::CmosRam::Bytes64;
    }
    assert!(matches!(
        small_cmos.validate(),
        Err(ConfigError::BoardConfiguration { .. })
    ));
}

#[test]
fn generic386_memory_validation_bounds_ram_and_reserves_pristine_copy() {
    let config = board();
    for (size, valid) in [
        (63 << 20, true),
        ((63 << 20) + 65536, false),
        ((15 << 20) + 1, false),
    ] {
        let mut changed = config.clone();
        changed.memory.address_space_bytes = 1 << 32;
        changed.memory.ram[2].size = size;
        assert_eq!(changed.validate().is_ok(), valid);
    }
    let mut missing_shadow = config.clone();
    missing_shadow.memory.ram.remove(1);
    assert!(matches!(
        missing_shadow.validate(),
        Err(ConfigError::BoardConfiguration { .. })
    ));
    let mut high = config;
    high.memory.address_space_bytes = 1 << 32;
    high.memory
        .mmio
        .push(crate::MmioWindow::new(0xFFFE0000, 0x1000, DeviceId::Vga));
    assert!(high.validate().is_err());
}

#[test]
fn generic386_shadow_and_pristine_copy_have_independent_bus_permissions_across_reset() {
    let mut config = board();
    config.system_bios_image = None;
    let mut cpu = build_cpu(&config).unwrap();
    let mut tail = [0; 16];
    cpu.read_mem(0xFFFFFFF0, &mut tail).unwrap();
    assert_eq!(tail, [0xA7; 16]);
    assert_eq!(cpu.guest_dma_write(0xF1234, 0x5A), DmaAccess::Complete(()));
    assert_eq!(cpu.guest_dma_read(0xF1234), DmaAccess::Complete(0x5A));
    assert_eq!(
        cpu.guest_dma_write(0xFFFF1234, 0x00),
        DmaAccess::Complete(())
    );
    assert_eq!(cpu.guest_dma_read(0xFFFF1234), DmaAccess::Complete(0xA7));
    for _ in 0..2 {
        cpu.reset().unwrap();
        assert_eq!(cpu.guest_dma_read(0xF1234), DmaAccess::Complete(0x5A));
        assert_eq!(cpu.guest_dma_read(0xFFFF1234), DmaAccess::Complete(0xA7));
    }
    assert_eq!(cpu.guest_dma_read(0xC0000), DmaAccess::Complete(0));
    assert_eq!(cpu.guest_dma_write(0xC0000, 0x99), DmaAccess::Complete(()));
    assert_eq!(cpu.guest_dma_read(0xC0000), DmaAccess::Complete(0x99));
}

#[test]
fn generic386_guest_writes_and_reads_f_segment_through_machine_bus() {
    let config = board();
    let mut assembled = assemble_with_cpu(
        &config,
        &ObservationPolicy::default(),
        crate::AssemblyOptions {
            allow_unlisted_firmware: true,
        },
        |config| {
            let mut entry = config.clone();
            entry.cpu.reset = crate::ResetSpec::Entry { cs: 0, ip: 0x1000 };
            let mut cpu = build_cpu(&entry)?;
            // CLI; MOV AX,F000h; MOV ES,AX; MOV BYTE ES:[1234h],5Ah;
            // MOV AL,ES:[1234h]; OUT 80h,AL; HLT.
            cpu.write_mem(
                0x1000,
                &[
                    0xFA, 0xB8, 0, 0xF0, 0x8E, 0xC0, 0x26, 0xC6, 0x06, 0x34, 0x12, 0x5A, 0x26,
                    0xA0, 0x34, 0x12, 0xE6, 0x80, 0xF4,
                ],
            )?;
            Ok(cpu)
        },
    )
    .unwrap();
    assert_eq!(
        assembled.machine.run_bounded(Some(100)).unwrap(),
        machine::RunOutcome::Halted
    );
    assert_eq!(assembled.machine.cpu().reg(CpuReg::Rax) & 0xFF, 0x5A);
    assert_eq!(
        assembled.machine.bus_mut().port_read(0x80, 1).unwrap(),
        0x5A
    );
}

#[test]
fn generic386_start_reports_the_reset_fetch_blocker_as_a_typed_error() {
    let mut cpu = build_cpu(&board()).unwrap();
    let error = cpu
        .run_steps(1, cpu::StepRunOptions::default())
        .unwrap_err();
    assert!(matches!(error, CpuError::UnsupportedSystem { .. }));
    assert!(error
        .to_string()
        .contains("80386 reset fetch at 0xfffffff0"));
}

#[test]
fn generic386_ibm5170_adapter_space_remains_read_only() {
    let mut config = board();
    config.board = BoardModel::Ibm5170;
    config.cpu.chip = CpuChip::I80286;
    config.devices = config.board.motherboard_devices();
    config.memory.ram.remove(1);
    config.memory.ram[1].size = u64::from(crate::IBM5170_MAX_EXTENDED_KIB) * 1024;
    config.validate().unwrap();
    let mut cpu = build_cpu(&config).unwrap();
    assert_eq!(cpu.guest_dma_write(0xC0000, 0x55), DmaAccess::Complete(()));
    assert_eq!(cpu.guest_dma_read(0xC0000), DmaAccess::Complete(0));
    assert_eq!(cpu.guest_dma_write(0xF1234, 0x55), DmaAccess::Complete(()));
    assert_eq!(cpu.guest_dma_read(0xF1234), DmaAccess::Complete(0xA7));
    config.memory.ram.push(RamRegion::new(0xC0000, 0x1000));
    assert!(matches!(
        config.validate(),
        Err(ConfigError::AdapterSpaceOverlapsRam { .. })
    ));
}

#[test]
fn generic386_coprocessor_choices_match_board_validation() {
    use crate::capabilities::{describe, Domain, Draft, Value};

    let draft = Draft::new(board());
    assert_eq!(
        describe(&draft, "cpu.fpu").unwrap().domain,
        Domain::Values(vec![Value::Name("none".into())])
    );
    let ibm = Draft::new(crate::bios(vec![0xF4; 0x10000], None));
    assert_eq!(
        describe(&ibm, "cpu.fpu").unwrap().domain,
        Domain::Values(vec![
            Value::Name("none".into()),
            Value::Name("80287".into())
        ])
    );
}

#[test]
fn generic386_conventional_memory_choices_match_board_validation() {
    use crate::capabilities::{describe, Domain, Draft, Value};

    let draft = Draft::new(board());
    let descriptor = describe(&draft, "memory.conventional_kib").unwrap();
    let Domain::Integer(domain) = descriptor.domain else {
        panic!("expected conventional memory integer domain");
    };
    assert_eq!(domain.values().collect::<Vec<_>>(), vec![640]);
    assert_eq!(descriptor.default, Some(Value::Integer(640)));
    assert_eq!(descriptor.unit, Some("KiB"));

    let ibm = Draft::new(crate::bios(vec![0xF4; 0x10000], None));
    assert_eq!(
        describe(&ibm, "memory.conventional_kib").unwrap().domain,
        Domain::Integer(crate::descriptors::CONVENTIONAL_KIB)
    );
}
