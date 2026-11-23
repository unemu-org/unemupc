// SPDX-License-Identifier: AGPL-3.0-or-later
//! Generic ISA board integration probes. Intel 80386 Programmer's Reference
//! Manual, §2.1 Memory Organization and §10.1 Initialization; SeaBIOS rel-1.17.0,
//! shadow.c qemu_prep_reset. See machine-files.md §Generic 386 ISA board.

use super::*;
use crate::{BoardModel, CpuChip, PortWindow, RamRegion};
use cpu::{Cpu, CpuReg, DmaAccess};

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
fn generic386_sparse_backing_maps_only_the_populated_regions() {
    for extended in [0, 15 << 20, 63 << 20] {
        for video_window in [false, true] {
            let mut config = board();
            config.memory.address_space_bytes = 1 << 32;
            if extended == 0 {
                config.memory.ram.pop();
            } else {
                config.memory.ram[2].size = extended;
            }
            if video_window {
                config
                    .memory
                    .mmio
                    .push(crate::MmioWindow::new(0xA0000, 0x20000, DeviceId::Vga));
            }
            let mut mappings = Vec::new();
            let cpu = build_cpu_with_mapping_trace(&config, |base, end, kind| {
                mappings.push((base, end, kind));
            })
            .unwrap();
            let stats = cpu.memory_backing_stats();
            assert!(stats.sparse, "{stats:?}");
            assert_eq!(stats.reserved_bytes, 1 << 32);
            let mut expected = vec![
                (0, 0xA0000, "ram"),
                (0xC0000, 0x100000, "ram"),
                (0xFFFE0000, 1 << 32, "rom"),
            ];
            if extended != 0 {
                expected.push((0x100000, 0x100000 + extended, "ram"));
            }
            if video_window {
                expected.push((0xA0000, 0xC0000, "mmio"));
            }
            mappings.sort_unstable();
            expected.sort_unstable();
            assert_eq!(mappings, expected);
            let mapped_bytes: u64 = mappings.iter().map(|(base, end, _)| end - base).sum();
            let populated = 0xA0000 + extended + 0x40000 + 0x20000;
            let video_bytes = if video_window { 0x20000 } else { 0 };
            assert_eq!(mapped_bytes, populated + video_bytes);
            let committed = stats
                .committed_bytes
                .expect("sparse commitment is reported");
            // Every boundary is 64 KiB aligned, so supported host pages fit exactly.
            assert_eq!(committed, mapped_bytes);
            assert!(committed <= populated + video_bytes);
            assert!(committed < stats.reserved_bytes / 32);
            eprintln!(
                "extended={extended} video={video_window}: {stats:?}, mapped_bytes={mapped_bytes}"
            );
            let mut byte = [0];
            assert!(cpu.read_mem(0x40000000, &mut byte).is_err());
            assert!(cpu.read_mem(0xFFFDFFFF, &mut byte).is_err());
        }
    }
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
fn generic386_reset_executes_pristine_high_copy_and_retains_shadow_ram() {
    let mut config = board();
    let ImageSource::Inline(bytes) = &mut config.images[0].source else {
        panic!("expected inline BIOS");
    };
    bytes[0x1FFF0..0x1FFF9]
        .copy_from_slice(&[0xEB, 0x02, 0x0F, 0x0B, 0xEA, 0x00, 0x10, 0x00, 0xF0]);
    bytes[0x11000..0x11005].copy_from_slice(&[0xB0, 0x5A, 0xE6, 0x80, 0xF4]);
    let mut assembled = assemble_with_cpu(
        &config,
        &ObservationPolicy::default(),
        crate::AssemblyOptions {
            allow_unlisted_firmware: true,
        },
        |config| {
            let mut cpu = build_cpu(config)?;
            cpu.write_mem(0xFFFF0, &[0x0F, 0x0B])?;
            cpu.write_mem(0xC1234, &[0x99])?;
            Ok(cpu)
        },
    )
    .unwrap();
    for cap in [10, 20] {
        assert_eq!(
            assembled.machine.run_bounded(Some(cap)).unwrap(),
            machine::RunOutcome::Halted
        );
        assert_eq!(
            assembled.machine.bus_mut().port_read(0x80, 1).unwrap(),
            0x5A
        );
        assert_eq!(assembled.machine.cpu().reg(CpuReg::Rdx), 0x0303);
        let mut byte = [0];
        assembled
            .machine
            .cpu()
            .read_mem(0xC1234, &mut byte)
            .unwrap();
        assert_eq!(byte, [0x99]);
        assembled
            .machine
            .cpu()
            .read_mem(0xFFFF0, &mut byte)
            .unwrap();
        assert_eq!(byte, [0x0F]);
        assembled.machine.reset_cpu().unwrap();
    }
}

#[test]
fn generic386_reset_window_fetches_high_rom_with_both_a20_sources_off() {
    let mut assembled = reset_window_board();
    for cap in [1, 2] {
        assembled.machine.reset_cpu().unwrap();
        assert!(assembled.machine.reset_fetch_window_open());
        assert!(assembled.machine.cpu().a20_enabled());
        assert_a20_sources_off(&mut assembled);
        assert_eq!(
            assembled.machine.run_bounded(Some(cap)).unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        assert_eq!(assembled.machine.cpu().reg(CpuReg::Rip), 0xFFF4);
        assert_eq!(
            assembled.machine.cpu().diagnostic_state().cs().base,
            0xFFFF0000
        );
        assert!(assembled.machine.reset_fetch_window_open());
        assert!(assembled.machine.cpu().a20_enabled());
        assert_a20_sources_off(&mut assembled);
    }
}

#[test]
fn generic386_reset_window_restores_board_a20_after_same_selector_far_jump() {
    let mut assembled = reset_window_board();
    assembled.machine.reset_cpu().unwrap();
    assert_eq!(
        assembled.machine.run_bounded(Some(2)).unwrap(),
        machine::RunOutcome::RetiredCapReached
    );
    let cs = assembled.machine.cpu().diagnostic_state().cs();
    assert_eq!(cs.selector, 0xF000);
    assert_eq!(cs.base, 0xF0000);
    assert_eq!(assembled.machine.cpu().reg(CpuReg::Rip), 0x1000);
    assert!(!assembled.machine.reset_fetch_window_open());
    assert_a20_sources_off(&mut assembled);
    assert!(!assembled.machine.cpu().a20_enabled());
    assert_eq!(
        assembled.machine.run_bounded(Some(20)).unwrap(),
        machine::RunOutcome::Halted
    );
    assert_eq!(
        assembled.machine.bus_mut().port_read(0x80, 1).unwrap(),
        0x5A
    );
}

fn reset_window_board() -> crate::AssembledMachine {
    let mut config = board();
    let ImageSource::Inline(bytes) = &mut config.images[0].source else {
        panic!("expected inline BIOS");
    };
    bytes[0x1FFF0..0x1FFF9]
        .copy_from_slice(&[0xEB, 0x02, 0x0F, 0x0B, 0xEA, 0x00, 0x10, 0x00, 0xF0]);
    // Keeping the selector makes the hidden base, rather than CS alone,
    // decisive. The following read distinguishes wrapped from flat addressing.
    bytes[0x11000..0x1100B].copy_from_slice(&[
        0xB8, 0xFF, 0xFF, 0x8E, 0xD8, 0xA0, 0x10, 0x05, 0xE6, 0x80, 0xF4,
    ]);
    let mut assembled = assemble_with_cpu(
        &config,
        &ObservationPolicy::default(),
        crate::AssemblyOptions {
            allow_unlisted_firmware: true,
        },
        |config| {
            let mut cpu = build_cpu(config)?;
            cpu.write_mem(0xFFFF0, &[0x0F, 0x0B])?;
            cpu.write_mem(0x500, &[0x5A])?;
            cpu.write_mem(0x100500, &[0xA5])?;
            Ok(cpu)
        },
    )
    .unwrap();
    assembled
        .machine
        .bus_mut()
        .port_write(0x64, 1, 0xD1)
        .unwrap();
    assembled.machine.bus_mut().port_write(0x60, 1, 1).unwrap();
    assembled.machine.bus_mut().port_write(0x92, 1, 0).unwrap();
    assert_a20_sources_off(&mut assembled);
    assembled
}

fn assert_a20_sources_off(assembled: &mut crate::AssembledMachine) {
    let bus = assembled.machine.bus_mut();
    assert_eq!(bus.port_read(0x92, 1).unwrap() & 2, 0);
    bus.port_write(0x64, 1, 0xD0).unwrap();
    assert_eq!(bus.port_read(0x60, 1).unwrap() & 3, 1);
}

#[derive(Clone, Copy, Debug)]
enum ResetSource {
    TripleFault,
    Port92,
    Keyboard,
}

impl ResetSource {
    fn instructions(self) -> &'static [u8] {
        match self {
            // Intel 80386 PRM, §9.8.8 Double Fault: no gate fits this one-byte IDT.
            Self::TripleFault => &[
                0xBA, 0xF9, 0x0C, 0xB0, 0x02, 0xEE, 0xB0, 0x06, 0xEE, // CF9h fallback
                0x66, 0xBA, 0xEF, 0xBE, 0xAD, 0xDE, 0x0F, 0x01, 0x1E, 0x00, 0x06, 0xCC,
            ],
            Self::Port92 => &[0xE4, 0x92, 0x24, 0xFE, 0xE6, 0x92, 0x0C, 0x01, 0xE6, 0x92],
            Self::Keyboard => &[0xB0, 0xFE, 0xE6, 0x64],
        }
    }
}

fn guest_reset_board(source: ResetSource, storm: bool) -> crate::AssembledMachine {
    let mut config = board();
    let ImageSource::Inline(bytes) = &mut config.images[0].source else {
        panic!("expected inline BIOS");
    };
    bytes[0x1FFF0..0x1FFF9]
        .copy_from_slice(&[0xEB, 0x02, 0x0F, 0x0B, 0xEA, 0x00, 0x10, 0x00, 0xF0]);
    let mut guest = vec![0xFA]; // CLI keeps the reset probe independent of timer IRQs.
    if !storm {
        guest.extend_from_slice(&[
            0x80, 0x3E, 0x00, 0x05, 0x5A, // CMP byte [0500h],5Ah
            0x74, 0, // JE done
            0xC6, 0x06, 0x00, 0x05, 0x5A, // MOV byte [0500h],5Ah
            0x66, 0xBA, 0xEF, 0xBE, 0xAD, 0xDE, // MOV EDX,DEADBEEFh
        ]);
    }
    guest.extend_from_slice(source.instructions());
    guest.extend_from_slice(&[0x0F, 0x0B]); // A reset must precede the next instruction.
    if !storm {
        guest[7] = (guest.len() - 8) as u8;
        guest.extend_from_slice(&[0xA0, 0x00, 0x05, 0xE6, 0x80, 0xF4]);
    }
    bytes[0x11000..0x11000 + guest.len()].copy_from_slice(&guest);
    assemble_with_cpu(
        &config,
        &ObservationPolicy::default(),
        crate::AssemblyOptions {
            allow_unlisted_firmware: true,
        },
        |config| {
            let mut cpu = build_cpu(config)?;
            // A low reset fetch would execute UD2; the pristine copy must survive.
            cpu.write_mem(0xFFFF0, &[0x0F, 0x0B])?;
            cpu.write_mem(0xC1234, &[0x99])?;
            Ok(cpu)
        },
    )
    .unwrap()
}

fn check_guest_reset(source: ResetSource) {
    for (keyboard_a20, fast_a20) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut assembled = guest_reset_board(source, false);
        let machine = &mut assembled.machine;
        machine.bus_mut().port_write(0x64, 1, 0xD1).unwrap();
        let output_port = if keyboard_a20 { 3 } else { 1 };
        machine.bus_mut().port_write(0x60, 1, output_port).unwrap();
        let fast_bit = if fast_a20 { 2 } else { 0 };
        machine.bus_mut().port_write(0x92, 1, fast_bit).unwrap();
        assert_eq!(machine.bus_mut().port_read(0xCF9, 1).unwrap(), 0xFF);
        machine.bus_mut().port_write(0x70, 1, 0x0F).unwrap();
        machine.bus_mut().port_write(0x71, 1, 0x0A).unwrap();
        machine.reset_cpu().unwrap();
        assert_eq!(
            machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::ResetRequested,
            "{source:?}, keyboard A20={keyboard_a20}"
        );
        assert_eq!(machine.cpu().reg(CpuReg::Rdx), 0xDEADBEEF);
        if matches!(source, ResetSource::TripleFault) {
            assert_eq!(
                machine.cpu().diagnostic_state().system.unwrap().idtr.limit,
                0
            );
        }
        let retired = machine.cpu().retired();
        let time = machine.virtual_time();
        let clock = machine.clock();
        // Resumption performs the pending reset even when the cap allows no step.
        assert_eq!(
            machine.run_bounded(Some(time)).unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.cpu().retired(), retired);
        assert_eq!(machine.virtual_time(), time);
        assert_eq!(machine.clock(), clock);
        assert_eq!(machine.cpu().reg(CpuReg::Rdx), 0x0303);
        assert_eq!(machine.cpu().reg(CpuReg::Rip), 0xFFF0);
        let state = machine.cpu().diagnostic_state();
        assert_eq!(state.cs().selector, 0xF000);
        assert_eq!(state.cs().base, 0xFFFF0000);
        let system = state.system.unwrap();
        assert_eq!(system.msw & 0x1F, 0);
        assert_eq!(system.idtr.limit, 0x3FF);
        assert!(machine.reset_fetch_window_open());
        assert!(machine.cpu().a20_enabled());
        assert_eq!(machine.bus_mut().port_read(0x71, 1).unwrap(), 0x0A);
        assert_eq!(
            machine.bus_mut().port_read(0x92, 1).unwrap(),
            fast_bit | u32::from(matches!(source, ResetSource::Port92))
        );
        machine.bus_mut().port_write(0x64, 1, 0xD0).unwrap();
        assert_eq!(machine.bus_mut().port_read(0x60, 1).unwrap(), output_port);
        for (addr, expected) in [
            (0x500, 0x5A),
            (0xC1234, 0x99),
            (0xFFFF0, 0x0F),
            (0xFFFFFFF0, 0xEB),
        ] {
            let mut byte = [0];
            machine.cpu().read_mem(addr, &mut byte).unwrap();
            assert_eq!(byte, [expected], "{addr:08X}");
        }
        assert_eq!(
            machine.run_bounded(Some(time + 1)).unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.cpu().diagnostic_state().cs().base, 0xFFFF0000);
        assert_eq!(
            machine.run_bounded(Some(time + 2)).unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.cpu().diagnostic_state().cs().base, 0xF0000);
        assert!(!machine.reset_fetch_window_open());
        assert_eq!(machine.cpu().a20_enabled(), keyboard_a20 || fast_a20);
        assert_eq!(
            machine.run_bounded(Some(time + 100)).unwrap(),
            machine::RunOutcome::Halted
        );
        assert_eq!(machine.bus_mut().port_read(0x80, 1).unwrap(), 0x5A);
        if matches!(source, ResetSource::Port92) {
            machine.bus_mut().port_write(0x92, 1, fast_bit | 1).unwrap();
            assert_eq!(
                machine.run_bounded(Some(time + 100)).unwrap(),
                machine::RunOutcome::Halted
            );
            machine.bus_mut().port_write(0x92, 1, fast_bit).unwrap();
            machine.bus_mut().port_write(0x92, 1, fast_bit | 1).unwrap();
            assert_eq!(
                machine.run_bounded(Some(time + 100)).unwrap(),
                machine::RunOutcome::ResetRequested
            );
        }
    }
}

#[test]
fn generic386_guest_triple_fault_resets_and_keeps_ram() {
    check_guest_reset(ResetSource::TripleFault);
}

#[test]
fn generic386_guest_port92_edge_resets_and_keeps_ram() {
    check_guest_reset(ResetSource::Port92);
}

#[test]
fn generic386_guest_keyboard_pulse_resets_and_keeps_ram() {
    check_guest_reset(ResetSource::Keyboard);
}

#[test]
fn generic386_guest_reset_storm_reports_each_source() {
    for source in [
        ResetSource::TripleFault,
        ResetSource::Port92,
        ResetSource::Keyboard,
    ] {
        let mut assembled = guest_reset_board(source, true);
        for _ in 1..cpu::RESET_STORM_RESETS {
            assert_eq!(
                assembled.machine.run_bounded(Some(10_000)).unwrap(),
                machine::RunOutcome::ResetRequested
            );
        }
        let error = assembled.machine.run_bounded(Some(10_000)).unwrap_err();
        assert!(
            matches!(&error, machine::MachineError::Cpu(cpu::CpuError::ResetStorm { resets, shutdown, cs: 0xF000, .. })
            if *resets == cpu::RESET_STORM_RESETS && *shutdown == matches!(source, ResetSource::TripleFault)),
            "{source:?}: {error}"
        );
        let machine::MachineError::Cpu(error) = error else {
            unreachable!()
        };
        assert!(assembled
            .machine
            .cpu()
            .stop_report(&error)
            .unwrap()
            .to_string()
            .contains("the machine resets again and again"));
    }
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
