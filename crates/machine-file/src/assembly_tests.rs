// SPDX-License-Identifier: AGPL-3.0-or-later
//! File-backed assembly regression tests; persistent-media.md § Integration.
use crate::{assemble, assemble_with_observation_and_options, assemble_with_options, FileMount};
use assembly::test_support::*;
use assembly::*;
use cpu::x86jit_backend::X86JitCpu;
use device::Device;
use devices::{CmosRtc, Dma8237, Drive, SysControl, I8042};
use machine::DmaChannelWiring;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, rc::Rc};
fn load_image_bytes(source: &ImageSource) -> Result<Vec<u8>, ConfigError> {
    assembly::test_support::load_mounted_bytes(&FileMount::new(), source)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_empty_drive_guest_recalibrates_a_configured_media_list() {
        let mut program = vec![
            0xfa, // CLI
            0xba, 0xf2, 0x03, 0xb0, 0x1c, 0xee, // release FDC reset
            0xba, 0xf5, 0x03, // MOV DX,3F5h
        ];
        for _ in 0..4 {
            program.extend([0xb0, 8, 0xee, 0xec, 0xec]);
        }
        program.extend([
            0xb0, 7, 0xee, 0xb0, 0, 0xee, // RECALIBRATE drive 0
            0xb0, 8, 0xee, // SENSE INTERRUPT STATUS
            0xec, 0xa2, 0x00, 0x05, // store ST0 at 0500h
            0xec, 0xa2, 0x01, 0x05, // store PCN at 0501h
            0xf4,
        ]);
        let mut config = assembly::at_fdc_write(program, vec![], 0x600);
        config.drives[0].kind = DriveKind::Floppy {
            geometry: FloppyGeometry::HD_1_2,
        };
        config.drives[0].media = Some(assembly::FloppyMediaList {
            entries: vec![],
            empty: true,
            inserted: 0,
        });
        let mut assembled = assemble(&config).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        let mut status = [0; 2];
        assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut status)
            .unwrap();
        assert_eq!(status, [0x20, 0]);
    }

    #[test]
    fn guest_refresh_loop_counts_virtual_pit_clocks_at_rate_one() {
        // Synthetic IN/TEST/LOOPZ loop matching IBM AT BIOS TEST1, checkpoint
        // 11. After the count write retires, MOV CX,0 loads CE at t=7.
        // OUT1 rises at t=25 (7+18). At one step per PIT clock,
        // seven INs at 8..26 consume 21 steps, leaving CX = -7.
        let program = vec![
            0xFA, // CLI
            0xB0, 0x74, 0xE6, 0x43, // Mode 2, channel 1, LSB/MSB
            0xB0, 18, 0xE6, 0x41, 0xB0, 0, 0xE6, 0x41, 0xB9, 0, 0, // MOV CX,0
            0xE4, 0x61, 0xA8, 0x10, 0xE1, 0xFA, // IN; TEST; LOOPZ
            0xF4,
        ];
        let mut config = assembly::at_boot(program);
        config.clock = assembly::ClockConfig {
            retired_per_pit_clock: 1,
            retired_per_pit_clock_denominator: 1,
            turbo: None,
        };
        config.devices.push(assembly::DeviceConfig::new(
            DeviceId::SysControl,
            DeviceKind::SysControl(assembly::SysControlCfg {}),
            vec![assembly::PortWindow::single(0x61)],
        ));
        let mut assembled = assemble(&config).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        assert_eq!(
            assembled.machine.cpu().reg(cpu::CpuReg::Rcx),
            0u16.wrapping_sub(7) as u64
        );
    }

    #[test]
    fn guest_refresh_speed_check_counts_256_cycles_at_explicit_rates() {
        // DERIVED from IBM AT BIOS TEST1, checkpoint 11: initial load at 7,
        // first IN at 9, three steps per polling iteration, two outer steps.
        // If n is the total LOOP count, each sampled IN is at 9 + 3*n +
        // 2*completed_cycles. OUT1 rises every 18 clocks from 25. Evaluating
        // that fixed waveform over 256 high/low pairs gives 2903 iterations.
        // At 8/13, the load is at PIT clock 10; sampling floor(steps*13/8)
        // gives 1721 iterations, within the BIOS speed-check window.
        let program = vec![
            0xFA, 0xB0, 0x74, 0xE6, 0x43, 0xB0, 18, 0xE6, 0x41, 0xB0, 0, 0xE6, 0x41, 0xB9, 0, 0,
            0x30, 0xDB, // MOV CX,0; XOR BL,BL
            0xE4, 0x61, 0xA8, 0x10, 0xE1, 0xFA, // IN; TEST; LOOPZ
            0xE4, 0x61, 0xA8, 0x10, 0xE0, 0xFA, // IN; TEST; LOOPNZ
            0xFE, 0xCB, 0x75, 0xF0, // DEC BL; JNZ first IN
            0xF4,
        ];
        for (numerator, denominator, expected_cx) in [(1, 1, 0xF4A9), (8, 13, 0xF947)] {
            let mut config = assembly::at_boot(program.clone());
            config.clock = assembly::ClockConfig {
                retired_per_pit_clock: numerator,
                retired_per_pit_clock_denominator: denominator,
                turbo: None,
            };
            config.devices.push(assembly::DeviceConfig::new(
                DeviceId::SysControl,
                DeviceKind::SysControl(assembly::SysControlCfg {}),
                vec![assembly::PortWindow::single(0x61)],
            ));
            let mut assembled = assemble(&config).unwrap();
            assert_eq!(
                assembled.machine.run_bounded(Some(10_000)).unwrap(),
                machine::RunOutcome::Halted
            );
            assert_eq!(assembled.machine.cpu().reg(cpu::CpuReg::Rcx), expected_cx);
        }
    }

    #[test]
    fn observation_precheck_matches_bind_for_every_device_and_output() {
        let config = assembly::bios(vec![0; 0x8000], Some(vec![0; 1_474_560]));
        let mut assembled = assemble_synthetic(&config).unwrap();
        for dc in &config.devices {
            let slot = assembled
                .machine
                .bus_mut()
                .device_mut(assembled.device_index[&dc.id])
                .unwrap();
            let producer = std::mem::replace(slot, Box::new(I8042::new()));
            let mut devices = vec![producer];
            let device_index = BTreeMap::from([(dc.id, 0)]);
            let mut single = config.clone();
            single.devices = vec![dc.clone()];
            for output in [
                assembly::OutputKind::TextScanout,
                assembly::OutputKind::Bytes(device::ByteStreamKind::SerialTx),
                assembly::OutputKind::Bytes(device::ByteStreamKind::PostCheckpoints),
            ] {
                let request = assembly::ObservationRequest {
                    device: dc.id,
                    output,
                    byte_limit: 1,
                };
                let absent = if dc.id == DeviceId::Keyboard {
                    DeviceId::Uart
                } else {
                    DeviceId::Keyboard
                };
                for routes in [
                    vec![request],
                    vec![request, request],
                    vec![assembly::ObservationRequest {
                        device: absent,
                        ..request
                    }],
                ] {
                    let policy = ObservationPolicy {
                        routes,
                        ..Default::default()
                    };
                    assert_eq!(
                        validate_observation_policy(&single, &policy),
                        Observation::bind(&policy, &device_index, &mut devices).map(|_| ()),
                        "{:?} {:?}",
                        dc.kind,
                        policy.routes
                    );
                }
            }
        }
    }

    fn assemble_synthetic(config: &MachineConfig) -> Result<AssembledMachine, AssemblyError> {
        assemble_with_options(
            config,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
    }

    fn observe_synthetic(
        config: &MachineConfig,
        policy: &ObservationPolicy,
    ) -> Result<AssembledMachine, AssemblyError> {
        assemble_with_observation_and_options(
            config,
            policy,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
    }

    fn rtc_config(config: &mut MachineConfig) -> &mut CmosCfg {
        let device = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Rtc)
            .unwrap();
        let DeviceKind::Rtc(ref mut cfg) = device.kind else {
            panic!("RTC kind")
        };
        cfg
    }

    fn rtc_registers(config: &MachineConfig) -> [u8; 64] {
        let mut assembled = assemble_synthetic(config).unwrap();
        let bus = assembled.machine.bus_mut();
        std::array::from_fn(|index| {
            bus.port_write(0x70, 1, index as u32).unwrap();
            bus.port_read(0x71, 1).unwrap() as u8
        })
    }

    #[test]
    fn bios_preset_assembles_both_dma_banks_and_empty_coprocessor() {
        let config = assembly::bios(vec![0; 0x8000], Some(vec![0; 1_474_560]));
        config.validate_host().unwrap();
        let mut assembled = assemble_synthetic(&config).unwrap();
        let ids = &assembled.device_index;
        assert_eq!(ids[&DeviceId::Dma], 3);
        assert_eq!(ids[&DeviceId::DmaWord], 4);
        assert_eq!(ids[&DeviceId::Coprocessor], 7);
        let bus = assembled.machine.bus_mut();
        bus.port_write(0x0C, 1, 0).unwrap();
        bus.port_write(0x00, 1, 0x56).unwrap();
        bus.port_write(0x00, 1, 0x78).unwrap();
        bus.port_write(0xC0, 1, 0x12).unwrap();
        bus.port_write(0xC0, 1, 0x34).unwrap();
        bus.port_write(0xD8, 1, 0).unwrap();
        assert_eq!(bus.port_read(0xC0, 1).unwrap(), 0x12);
        assert_eq!(bus.port_read(0xC0, 1).unwrap(), 0x34);
        bus.port_write(0x0C, 1, 0).unwrap();
        assert_eq!(bus.port_read(0x00, 1).unwrap(), 0x56);
        assert_eq!(bus.port_read(0x00, 1).unwrap(), 0x78);
        for (port, value) in [(0x81, 0x12), (0x8B, 0x34), (0x89, 0x56), (0x8A, 0x78)] {
            bus.port_write(port, 1, value).unwrap();
            assert_eq!(bus.port_read(port, 1).unwrap(), value);
        }
        for port in [0xF0, 0xF1, 0xF8, 0xFF] {
            bus.port_write(port, 1, 0).unwrap();
            assert_eq!(bus.port_read(port, 1).unwrap(), 0xFF);
        }
        let irq = build_irq_lines(&config, ids);
        assert!(irq
            .iter()
            .any(|line| line.irq == 13 && line.device == ids[&DeviceId::Coprocessor]));
    }

    #[test]
    fn cpu_executes_x87_despite_the_boards_empty_coprocessor_socket() {
        // Keep DS flat so the guest's control-word store and host readback
        // address the same scratch word, independently of the BIOS segment seed.
        let program = [
            0xDB, 0xE3, // FNINIT
            0xC7, 0x06, 0xFE, 0x03, 0, 0, // MOV word [03FEh], 0
            0xD9, 0x3E, 0xFE, 0x03, // FNSTCW [03FEh]
            0xFA, 0xF4, // CLI; HLT
        ];
        let mut rom = vec![0; 0x8000];
        rom[..program.len()].copy_from_slice(&program);
        let mut config = assembly::bios(rom, Some(vec![0; 1_474_560]));
        config.board = assembly::BoardModel::Bare;
        config.cpu.reset = assembly::ResetSpec::Entry { cs: 0xF800, ip: 0 };
        config.cpu.segments = assembly::SegmentSeed::flat(0);
        let mut assembled = assemble_synthetic(&config).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        let mut control_word = [0xFF; 2];
        assembled
            .machine
            .cpu()
            .read_mem(0x3FE, &mut control_word)
            .unwrap();
        assert_eq!(u16::from_le_bytes(control_word), 0x037F);
    }

    #[test]
    fn i286_protected_guest_resumes_through_keyboard_reset_and_cmos() {
        for shutdown in [0x05, 0x0A] {
            let mut rom = vec![0; 0x8000];
            rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, 0, 0x15, 0, 0]);
            let mut config = assembly::bios(rom, Some(vec![0; 1_474_560]));
            config.cpu.segments = assembly::SegmentSeed::flat(0);
            let mut seed = |base, bytes: &[u8]| {
                config.images.push(assembly::LoadImage {
                    base,
                    source: ImageSource::Inline(bytes.to_vec()),
                    kind: ImageKind::RamLoad,
                    provides: Vec::new(),
                });
            };
            seed(
                0x1000,
                &[
                    0xFA, // CLI: no protected interrupt delivery is required.
                    0xB0, 0x0F, 0xE6, 0x70, 0xB0, shutdown, 0xE6, 0x71, 0x0F, 0x01, 0x16, 0,
                    0x21, // LGDT [2100h]
                    0xB8, 1, 0, 0x0F, 0x01, 0xF0, // LMSW AX
                    0xEA, 0, 0x12, 8, 0,
                ],
            );
            seed(
                0x2000,
                &[
                    0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0, 0, 0x9A, 0, 0, 0xFF, 0xFF, 0, 0, 0,
                    0x92, 0, 0,
                ],
            );
            seed(0x2100, &[23, 0, 0, 0x20, 0, 0]);
            seed(
                0x1200,
                &[
                    0xB8, 0x10, 0, 0x8E, 0xD8, 0x8E, 0xD0, 0xC7, 0x06, 0, 5, 0xEF, 0xBE, 0xB0,
                    0xFE, 0xE6, 0x64, // 8042 pulse RESET
                    0xC7, 0x06, 2, 5, 0xAD, 0xDE,
                ],
            );
            // A synthetic reset stub reads live shutdown state and the standard
            // BIOS resume pointer; CMOS and RAM belong to the retained machine.
            seed(
                0x1500,
                &[
                    0x0F, 0x01, 0xE3, // SMSW BX
                    0x81, 0x3E, 0, 5, 0xEF, 0xBE, 0x75, 10, 0xB0, 0x0F, 0xE6, 0x70, 0xE4, 0x71,
                    0xFF, 0x2E, 0x67, 0x04, // JMP FAR [0467h]
                    0xEA, 0, 0x10, 0, 0, // Cold entry before the protected marker exists.
                ],
            );
            seed(0x467, &[0, 0x16, 0, 0]);
            seed(0x1600, &[0xC7, 0x06, 4, 5, 0xFE, 0xCA, 0xFA, 0xF4]);
            let mut assembled = assemble_synthetic(&config).unwrap();
            assert_eq!(
                assembled.machine.run_bounded(Some(100)).unwrap(),
                machine::RunOutcome::ResetRequested
            );
            assert_eq!(assembled.machine.cpu().code_segment(), 8);
            assert_eq!(
                assembled.machine.run_bounded(Some(100)).unwrap(),
                machine::RunOutcome::Halted
            );
            let cpu = assembled.machine.cpu();
            assert_eq!(cpu.code_segment(), 0);
            assert_eq!(cpu.reg(cpu::CpuReg::Rbx), 0xFFF0);
            assert_eq!(cpu.reg(cpu::CpuReg::Rax) & 0xFF, u64::from(shutdown));
            let mut markers = [0; 6];
            cpu.read_mem(0x500, &mut markers).unwrap();
            assert_eq!(markers, [0xEF, 0xBE, 0, 0, 0xFE, 0xCA]);
        }
    }

    #[test]
    fn guest_keyboard_reset_stops_before_the_following_instruction() {
        for (program, reset_time) in [
            (vec![0xB0, 0xFE, 0xE6, 0x64, 0xB0, 0x55, 0xFA, 0xF4], 2),
            (
                vec![
                    0xB0, 0xD1, 0xE6, 0x64, 0xB0, 2, 0xE6, 0x60, 0xB0, 0x55, 0xFA, 0xF4,
                ],
                4,
            ),
        ] {
            let mut rom = vec![0; 0x8000];
            rom[..program.len()].copy_from_slice(&program);
            rom[0x7FF0..0x7FF4].copy_from_slice(&[0xB0, 0x66, 0xFA, 0xF4]);
            let mut config = assembly::bios(rom, Some(vec![0; 1_474_560]));
            config.board = assembly::BoardModel::Bare;
            config.cpu.reset = assembly::ResetSpec::Entry { cs: 0xF800, ip: 0 };
            let mut assembled = assemble_synthetic(&config).unwrap();
            assert_eq!(
                assembled.machine.run_bounded(Some(100)).unwrap(),
                machine::RunOutcome::ResetRequested
            );
            assert_eq!(assembled.machine.virtual_time(), reset_time);
            assert_ne!(assembled.machine.cpu().reg(cpu::CpuReg::Rax) & 0xFF, 0x55);
            assert_eq!(
                assembled.machine.run_bounded(Some(100)).unwrap(),
                machine::RunOutcome::Halted
            );
            assert_eq!(assembled.machine.cpu().reg(cpu::CpuReg::Rax) & 0xFF, 0x66);
        }
    }

    fn a20_probe(program: &mut Vec<u8>, marker: u16) {
        program.extend([
            0x31, 0xC0, 0x8E, 0xD8, 0xC6, 0x06, 0, 0, 0x11, 0xB8, 0xFF, 0xFF, 0x8E, 0xD8, 0xC6,
            0x06, 0x10, 0, 0x77, 0x31, 0xC0, 0x8E, 0xD8, 0xA0, 0, 0, 0x26, 0xA2,
        ]);
        program.extend(marker.to_le_bytes());
        program.extend([
            0xC6, 0x06, 0, 0, 0x99, 0xB8, 0xFF, 0xFF, 0x8E, 0xD8, 0xA0, 0x10, 0, 0x26, 0xA2,
        ]);
        program.extend((marker + 1).to_le_bytes());
    }

    fn a20_rig(
        mut program: Vec<u8>,
        board: assembly::BoardModel,
        keyboard: bool,
        fast: bool,
    ) -> AssembledMachine {
        program.extend([0xFA, 0xF4]);
        let mut rom = vec![0; 0x8000];
        rom[..program.len()].copy_from_slice(&program);
        rom[0x7FF0..0x7FF2].copy_from_slice(&[0xFA, 0xF4]);
        if board == assembly::BoardModel::Ibm5170 {
            rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, 0, 0, 0, 0xF8]);
        }
        let mut config = assembly::bios(rom, Some(vec![0; 1_474_560]));
        config.board = board;
        if board == assembly::BoardModel::Bare {
            config.cpu.reset = ResetSpec::Entry { cs: 0xF800, ip: 0 };
        }
        config.cpu.segments = SegmentSeed::flat(0);
        config.memory.address_space_bytes = 0x200000;
        config.memory.ram.push(RamRegion::new(0x100000, 0x100000));
        if !keyboard {
            config
                .devices
                .retain(|d| !matches!(d.kind, DeviceKind::Keyboard(_)));
        }
        if fast {
            config.devices.push(assembly::DeviceConfig::new(
                DeviceId::SysCtrlPortA,
                DeviceKind::SysCtrlPortA,
                vec![assembly::PortWindow::single(0x92)],
            ));
        }
        assemble_synthetic(&config).unwrap()
    }

    #[test]
    fn a20_keyboard_commands_gate_cpu_reads_and_writes_above_one_mib() {
        use cpu::Cpu;
        for board in [assembly::BoardModel::Ibm5170, assembly::BoardModel::Bare] {
            let mut program = vec![0xB0, 0xD1, 0xE6, 0x64, 0xB0, 1, 0xE6, 0x60];
            a20_probe(&mut program, 0x500);
            program.extend([0xB0, 0xD1, 0xE6, 0x64, 0xB0, 3, 0xE6, 0x60]);
            a20_probe(&mut program, 0x502);
            let mut assembled = a20_rig(program, board, true, false);
            assert!(assembled.machine.cpu().a20_enabled());
            assert_eq!(assembled.a20_line.change_count(), 1);
            assert_eq!(
                assembled.machine.run().unwrap(),
                machine::RunOutcome::Halted
            );
            let mut markers = [0; 4];
            assembled
                .machine
                .cpu()
                .read_mem(0x500, &mut markers)
                .unwrap();
            assert_eq!(markers, [0x77, 0x99, 0x11, 0x77]);
            assert!(assembled.machine.cpu().a20_enabled());
        }
    }

    #[test]
    fn a20_port_92_initial_state_and_writes_gate_cpu_accesses() {
        use cpu::Cpu;
        let mut program = Vec::new();
        a20_probe(&mut program, 0x500);
        program.extend([0xB0, 2, 0xE6, 0x92]);
        a20_probe(&mut program, 0x502);
        program.extend([0xB0, 0, 0xE6, 0x92]);
        a20_probe(&mut program, 0x504);
        let mut assembled = a20_rig(program, assembly::BoardModel::Bare, false, true);
        assert!(!assembled.machine.cpu().a20_enabled());
        assert_eq!(assembled.a20_line.change_count(), 0);
        assert_eq!(
            assembled.machine.run().unwrap(),
            machine::RunOutcome::Halted
        );
        let mut markers = [0; 6];
        assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut markers)
            .unwrap();
        assert_eq!(markers, [0x77, 0x99, 0x11, 0x77, 0x77, 0x99]);
        assert!(!assembled.machine.cpu().a20_enabled());
        let mut high = [0];
        assembled
            .machine
            .cpu()
            .read_mem(0x100000, &mut high)
            .unwrap();
        assert_eq!(high, [0x77], "host physical reads bypass A20");
    }

    #[test]
    fn a20_either_source_keeps_cpu_gate_enabled() {
        use cpu::Cpu;
        let mut program = vec![0xB0, 0, 0xE6, 0x92];
        a20_probe(&mut program, 0x500);
        program.extend([
            0xB0, 2, 0xE6, 0x92, 0xB0, 0xD1, 0xE6, 0x64, 0xB0, 1, 0xE6, 0x60,
        ]);
        a20_probe(&mut program, 0x502);
        program.extend([0xB0, 0, 0xE6, 0x92]);
        a20_probe(&mut program, 0x504);
        program.extend([0xB0, 0xD1, 0xE6, 0x64, 0xB0, 3, 0xE6, 0x60]);
        a20_probe(&mut program, 0x506);
        let mut assembled = a20_rig(program, assembly::BoardModel::Bare, true, true);
        assert_eq!(
            assembled.machine.run().unwrap(),
            machine::RunOutcome::Halted
        );
        assert!(assembled.machine.cpu().a20_enabled());
        let mut markers = [0; 8];
        assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut markers)
            .unwrap();
        assert_eq!(markers, [0x11, 0x77, 0x11, 0x77, 0x77, 0x99, 0x11, 0x77]);
    }

    #[test]
    fn a20_cpu_only_resets_preserve_enabled_and_disabled_gate() {
        use cpu::Cpu;
        for enabled in [false, true] {
            for reset_port in [0x60, 0x64, 0x92] {
                let mut program = vec![
                    0xB0,
                    0xD1,
                    0xE6,
                    0x64,
                    0xB0,
                    if enabled { 3 } else { 1 },
                    0xE6,
                    0x60,
                ];
                if reset_port == 0x60 {
                    program.extend([0xB0, 0xD1, 0xE6, 0x64]);
                }
                program.extend([
                    0xB0,
                    match reset_port {
                        0x60 => {
                            if enabled {
                                2
                            } else {
                                0
                            }
                        }
                        0x64 => 0xFE,
                        _ => {
                            if enabled {
                                3
                            } else {
                                1
                            }
                        }
                    },
                    0xE6,
                    reset_port,
                ]);
                let mut assembled = a20_rig(program, assembly::BoardModel::Bare, true, true);
                assert_eq!(
                    assembled.machine.run().unwrap(),
                    machine::RunOutcome::ResetRequested
                );
                assert_eq!(assembled.machine.cpu().a20_enabled(), enabled);
                let before = assembled.a20_line.change_count();
                let time = assembled.machine.virtual_time();
                assert_eq!(
                    assembled.machine.run_bounded(Some(time)).unwrap(),
                    machine::RunOutcome::RetiredCapReached
                );
                assert_eq!(assembled.machine.cpu().a20_enabled(), enabled);
                assert_eq!(assembled.a20_line.change_count(), before);
                assert_eq!(
                    assembled.machine.run().unwrap(),
                    machine::RunOutcome::Halted
                );
                assert_eq!(assembled.machine.cpu().a20_enabled(), enabled);
            }
        }
    }

    #[test]
    fn a20_bare_divide_error_vectors_through_ivt() {
        for (keyboard, fast) in [(true, false), (false, true)] {
            let mut program = vec![0xFA, 0xC7, 0x06, 0, 0, 0, 6, 0xC7, 0x06, 2, 0, 0, 0];
            for (offset, byte) in [0xC6, 0x06, 0, 5, 0x77, 0xFA, 0xF4].into_iter().enumerate() {
                program.extend([0xC6, 0x06]);
                program.extend((0x600u16 + offset as u16).to_le_bytes());
                program.push(byte);
            }
            program.extend([0x31, 0xC0, 0xF6, 0xF0]);
            let mut assembled = a20_rig(program, assembly::BoardModel::Bare, keyboard, fast);
            assert_eq!(
                assembled.machine.run_bounded(Some(1000)).unwrap(),
                machine::RunOutcome::Halted
            );
            let mut marker = [0];
            assembled
                .machine
                .cpu()
                .read_mem(0x500, &mut marker)
                .unwrap();
            assert_eq!(marker, [0x77], "INT 0 must execute the guest handler");
            assert_eq!(assembled.machine.cpu().code_segment(), 0);
        }
    }

    /// A CPU-only reset with the gate off, through a shutdown and through the
    /// 8042's reset pulse, reaches the ROM's reset vector; the guest reads
    /// back the output port it wrote, the gate still off, and from the first
    /// far jump the gate wraps 100600h onto 000600h again. See
    /// `assembly::test_support::a20_off_reset_rom` and machine-config.md §A20
    /// line and System Control Port A.
    #[test]
    fn a20_ibm_reset_with_gate_off_reaches_the_reset_vector_and_keeps_the_gate_off() {
        use cpu::Cpu;
        let mut assembled = assemble_synthetic(&assembly::test_support::a20_off_reset()).unwrap();
        let mut resets = 0;
        loop {
            match assembled.machine.run_bounded(Some(10_000)).unwrap() {
                machine::RunOutcome::ResetRequested => resets += 1,
                machine::RunOutcome::Halted => break,
                outcome => panic!("unexpected {outcome:?}"),
            }
            assert!(resets <= 2, "more resets than the ROM asks for");
        }
        let cpu = assembled.machine.cpu();
        let byte = |addr: u64| {
            let mut value = [0];
            cpu.read_mem(addr, &mut value).unwrap();
            value[0]
        };
        assert_eq!(byte(0x540), 0, "a reset fetch missed the ROM (#UD)");
        assert_eq!(resets, 2);
        assert_eq!(byte(0x500), 3, "the ROM's reset path ran three times");
        let read = |base: u64| [byte(base + 1), byte(base + 2), byte(base + 3)];
        assert_eq!(
            read(0x510),
            [0x03, 0xDD, 0xDD],
            "8042 output port read back (D0h) at each start"
        );
        assert_eq!(read(0x520), [0x00, 0x05, 0x0A], "CMOS shutdown byte");
        assert_eq!(
            read(0x530),
            [0x00, 0x02, 0x03],
            "FFFF:0610h: extended RAM at the first start, wrapped after each reset"
        );
        assert!(!assembled.a20_line.enabled());
        assert!(!assembled.machine.cpu().a20_enabled());
    }

    /// Pin the reset-fetch window (machine-config.md §A20 line and System
    /// Control Port A, "Reset fetch with the gate off"): the board decodes
    /// nothing at EFFFF0h and its gate stays off, yet right after the CPU-only
    /// reset the CPU's A20 is held open, so the first fetch reaches the high
    /// ROM alias at FFFFF0h; the far jump there closes the window and the
    /// gate applies again. The 8042 output port keeps the value written.
    #[test]
    fn a20_ibm_reset_with_gate_disabled_fetches_rom_until_the_first_cs_load() {
        use cpu::{Cpu, CpuReg};
        let mut program = vec![0xFA, 0xC7, 0x06, 0x18, 0, 0, 6, 0xC7, 0x06, 0x1A, 0, 0, 0];
        for (offset, byte) in [0xC6, 0x06, 0, 5, 0x77, 0xFA, 0xF4].into_iter().enumerate() {
            program.extend([0xC6, 0x06]);
            program.extend((0x600u16 + offset as u16).to_le_bytes());
            program.push(byte);
        }
        program.extend([
            0xB0, 0xD1, 0xE6, 0x64, 0xB0, 1, 0xE6, 0x60, 0xB0, 0xFE, 0xE6, 0x64,
        ]);
        let mut assembled = a20_rig(program, assembly::BoardModel::Ibm5170, true, false);
        let mut bytes = [0; 5];
        assembled
            .machine
            .cpu()
            .read_mem(0xEFFFF0, &mut bytes)
            .unwrap();
        assert_eq!(bytes, [0xFF; 5], "the board decodes nothing at EFFFF0h");
        assert_eq!(
            assembled.machine.run_bounded(Some(1000)).unwrap(),
            machine::RunOutcome::ResetRequested
        );
        let output_port = |assembled: &mut AssembledMachine| {
            let bus = assembled.machine.bus_mut();
            bus.port_write(0x64, 1, 0xD0).unwrap();
            bus.port_read(0x60, 1).unwrap()
        };
        let time = assembled.machine.virtual_time();
        assert_eq!(
            assembled.machine.run_bounded(Some(time)).unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        assert!(!assembled.a20_line.enabled(), "the board gate stays off");
        assert!(
            assembled.machine.cpu().a20_enabled(),
            "the CPU fetches with A20 open"
        );
        assert_eq!(assembled.machine.cpu().code_segment(), 0xF000);
        assert_eq!(assembled.machine.cpu().reg(CpuReg::Rip), 0xFFF0);
        assert_eq!(output_port(&mut assembled), 1, "8042 read-back unchanged");
        const NOTE: &str = "         CPU A20 held open by the reset window\n";
        let report = assembled.diagnostic_report("running").to_string();
        assert!(report.contains(NOTE), "{report}");
        assert_eq!(
            assembled.machine.run_bounded(Some(time + 1)).unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        assert_eq!(assembled.machine.cpu().code_segment(), 0xF800);
        assert!(
            !assembled.machine.cpu().a20_enabled(),
            "the far jump closes the window"
        );
        let report = assembled.diagnostic_report("running").to_string();
        assert!(!report.contains(NOTE), "{report}");
        assert_eq!(
            assembled.machine.run_bounded(Some(time + 1000)).unwrap(),
            machine::RunOutcome::ResetRequested,
            "the ROM runs again and resets again"
        );
        let mut marker = [0];
        assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut marker)
            .unwrap();
        assert_eq!(marker, [0], "no #UD at the reset vector");
    }

    #[test]
    fn a20_ibm_self_test_restores_high_reset_rom_fetch() {
        use cpu::{Cpu, CpuReg};
        let mut program = vec![
            0xFA, 0x80, 0x3E, 1, 5, 1, 0x74, 0, 0xC6, 0x06, 1, 5, 1, 0xC7, 0x06, 0x18, 0, 0, 6,
            0xC7, 0x06, 0x1A, 0, 0, 0,
        ];
        for (offset, byte) in [0xC6, 0x06, 0, 5, 0x77, 0xFA, 0xF4].into_iter().enumerate() {
            program.extend([0xC6, 0x06]);
            program.extend((0x600u16 + offset as u16).to_le_bytes());
            program.push(byte);
        }
        program.extend([
            0xB0, 0xD1, 0xE6, 0x64, 0xB0, 0xDD, 0xE6, 0x60, 0xB0, 0xAA, 0xE6, 0x64, 0xE4, 0x60,
            0xA2, 2, 5, 0xB0, 0xFE, 0xE6, 0x64,
        ]);
        program[7] = (program.len() - 8) as u8;
        program.extend([0xC6, 0x06, 0, 5, 0x55]);
        let mut assembled = a20_rig(program, assembly::BoardModel::Ibm5170, true, false);
        assert_eq!(
            assembled.machine.run_bounded(Some(1000)).unwrap(),
            machine::RunOutcome::ResetRequested
        );
        let time = assembled.machine.virtual_time();
        assert_eq!(
            assembled.machine.run_bounded(Some(time)).unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        assert!(assembled.machine.cpu().a20_enabled());
        assert_eq!(assembled.machine.cpu().code_segment(), 0xF000);
        assert_eq!(assembled.machine.cpu().reg(CpuReg::Rip), 0xFFF0);
        assert_eq!(
            assembled.machine.run_bounded(Some(time + 1000)).unwrap(),
            machine::RunOutcome::Halted
        );
        let mut markers = [0; 3];
        assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut markers)
            .unwrap();
        assert_eq!(
            markers,
            [0x55, 1, 0x55],
            "reset ROM must run instead of #UD; self-test must reply 55h"
        );
        assert_eq!(assembled.machine.cpu().code_segment(), 0xF800);
        assert!(assembled.machine.cpu().a20_enabled());
    }

    #[test]
    fn a20_bare_rig_without_sources_keeps_flat_addressing() {
        use cpu::Cpu;
        let mut program = Vec::new();
        a20_probe(&mut program, 0x500);
        let mut assembled = a20_rig(program, assembly::BoardModel::Bare, false, false);
        assert!(!assembled.a20_line.enabled());
        assert!(assembled.machine.cpu().a20_enabled());
        assert_eq!(
            assembled.machine.run().unwrap(),
            machine::RunOutcome::Halted
        );
        let mut markers = [0; 2];
        assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut markers)
            .unwrap();
        assert_eq!(markers, [0x11, 0x77]);
    }

    #[test]
    fn guest_port_a_reset_uses_the_shared_line_and_resumes_at_reset_vector() {
        for keyboard_first in [false, true] {
            for keep_keyboard in [false, true] {
                for port_a_reset in [false, true] {
                    if !keep_keyboard && !port_a_reset {
                        continue;
                    }
                    let program = [
                        0xB0,
                        if port_a_reset { 3 } else { 0xFE },
                        0xE6,
                        if port_a_reset { 0x92 } else { 0x64 },
                        0xB0,
                        0x55,
                        0xFA,
                        0xF4,
                    ];
                    let mut rom = vec![0; 0x8000];
                    rom[..program.len()].copy_from_slice(&program);
                    rom[0x7FF0..0x7FF4].copy_from_slice(&[0xB0, 0x66, 0xFA, 0xF4]);
                    let mut config = assembly::bios(rom, Some(vec![0; 1_474_560]));
                    config.board = assembly::BoardModel::Bare;
                    config.cpu.reset = assembly::ResetSpec::Entry { cs: 0xF800, ip: 0 };
                    if !keep_keyboard {
                        config
                            .devices
                            .retain(|d| !matches!(d.kind, DeviceKind::Keyboard(_)));
                    }
                    let port = assembly::DeviceConfig::new(
                        DeviceId::SysCtrlPortA,
                        DeviceKind::SysCtrlPortA,
                        vec![assembly::PortWindow::single(0x92)],
                    );
                    if keyboard_first {
                        config.devices.push(port);
                    } else {
                        config.devices.insert(0, port);
                    }
                    let mut assembled = assemble_synthetic(&config).unwrap();
                    assert_eq!(assembled.a20_line.enabled(), keep_keyboard);
                    if keep_keyboard {
                        assembled
                            .machine
                            .bus_mut()
                            .port_write(0x64, 1, 0xD1)
                            .unwrap();
                        assembled.machine.bus_mut().port_write(0x60, 1, 1).unwrap();
                        assert!(!assembled.a20_line.enabled());
                    }
                    let before = assembled.a20_line.change_count();
                    assert_eq!(
                        assembled.machine.run_bounded(Some(100)).unwrap(),
                        machine::RunOutcome::ResetRequested
                    );
                    assert_eq!(assembled.machine.virtual_time(), 2);
                    assert_ne!(assembled.machine.cpu().reg(cpu::CpuReg::Rax) & 0xFF, 0x55);
                    assert_eq!(assembled.a20_line.enabled(), port_a_reset);
                    assert_eq!(
                        assembled.a20_line.change_count(),
                        before + u64::from(port_a_reset)
                    );
                    assert_eq!(
                        assembled.machine.run_bounded(Some(100)).unwrap(),
                        machine::RunOutcome::Halted
                    );
                    assert_eq!(assembled.machine.cpu().reg(cpu::CpuReg::Rax) & 0xFF, 0x66);
                    assert_eq!(assembled.a20_line.enabled(), port_a_reset);
                    assert_eq!(
                        assembled.machine.bus_mut().port_read(0x92, 1).unwrap(),
                        if port_a_reset { 3 } else { 0 }
                    );
                }
            }
        }
    }

    #[test]
    fn run_policies_restart_cpu_before_and_after_prompt() {
        // A RAM counter lets the reset vector request one more reset after input.
        let mut rom = vec![0; 0x8000];
        rom[..5].copy_from_slice(&[0xB0, 0xFE, 0xE6, 0x64, 0xF4]);
        rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, 0, 1, 0, 0xF8]);
        let stub = [
            0xFE, 0x06, 0, 5, 0x80, 0x3E, 0, 5, 2, 0x74, 4, 0xB0, 0xFE, 0xE6, 0x64, 0xB0, 0x55,
            0xFA, 0xF4,
        ];
        rom[0x100..0x100 + stub.len()].copy_from_slice(&stub);
        let mut config = assembly::bios(rom, Some(vec![0; 1_474_560]));
        config.board = assembly::BoardModel::Bare;
        config.cpu.reset = assembly::ResetSpec::Entry { cs: 0xF800, ip: 0 };
        for prompt in [false, true] {
            let mut assembled = observe_synthetic(
                &config,
                &ObservationPolicy {
                    routes: vec![assembly::ObservationRequest {
                        device: DeviceId::Vga,
                        output: assembly::OutputKind::TextScanout,
                        byte_limit: 0,
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
            let mut policy = assembly::RunPolicy::natural(Some(100), None);
            if prompt {
                policy.stop = assembly::StopCondition::PromptThen {
                    console: assembly::ConsoleProbe {
                        device: DeviceId::Vga,
                    },
                    needle: String::new(),
                    poll_step: 2,
                    then_budget: 98,
                };
                policy.stimuli.push(assembly::KeyboardStimulus {
                    keyboard_index: assembled.device_index[&DeviceId::Keyboard],
                    scancodes: vec![0x1E],
                    schedule: assembly::InjectionSchedule::AfterPrompt,
                });
            }
            assert_eq!(
                assembly::run(&mut assembled.machine, &assembled.observation, &policy).unwrap(),
                machine::RunOutcome::Halted
            );
            assert_eq!(assembled.machine.virtual_time(), 15);
            assert_eq!(assembled.machine.cpu().reg(cpu::CpuReg::Rax) & 0xFF, 0x55);
        }
    }

    #[test]
    fn bios_dma_word_out_uses_both_aliased_register_ports() {
        let mut rom = vec![0; 0x8000];
        rom[..7].copy_from_slice(&[0xB8, 0x12, 0x34, 0xE7, 0xC0, 0xFA, 0xF4]);
        let mut config = assembly::bios(rom, Some(vec![0; 1_474_560]));
        config.board = assembly::BoardModel::Bare;
        config.cpu.reset = assembly::ResetSpec::Entry { cs: 0xF800, ip: 0 };
        let mut assembled = assemble_synthetic(&config).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        let bus = assembled.machine.bus_mut();
        assert_eq!(bus.port_read(0xC0, 2).unwrap(), 0x3412);
        bus.port_write(0xC1, 1, 0x56).unwrap();
        bus.port_write(0xC0, 1, 0x78).unwrap();
        assert_eq!(bus.port_read(0xC0, 2).unwrap(), 0x7856);
    }

    #[test]
    fn initialization_default_rtc_matches_legacy_state_and_every_register_except_century() {
        for (config, expected_nvram) in [
            (
                assembly::bios(vec![0; 32768], None),
                assembly::NvramSource::Derived,
            ),
            (
                assembly::bios(vec![0; 32768], Some(vec![0; 1_474_560])),
                assembly::NvramSource::Derived,
            ),
            (
                assembly::at_rtc(vec![0xF4], 100, 10),
                assembly::NvramSource::Blank,
            ),
        ] {
            let rtc_cfg = config
                .devices
                .iter()
                .find_map(|device| match &device.kind {
                    DeviceKind::Rtc(cfg) => Some(cfg),
                    _ => None,
                })
                .unwrap();
            assert_eq!(rtc_cfg.start, assembly::RtcStart::default());
            let mut legacy = CmosRtc::new();
            if let Some((second, uip)) = rtc_cfg.timing {
                legacy = legacy.with_timing(second, uip);
            }
            assert_eq!(rtc_cfg.nvram, expected_nvram);
            if expected_nvram == assembly::NvramSource::Derived {
                legacy = legacy.with_config(devices::CmosConfig {
                    equipment: 0x31,
                    ..devices::CmosConfig::default()
                });
                // CmosConfig leaves 32h at 00h; derived CMOS seeds the 2000 start-year century.
                legacy.port_write(0x70, 1, 0x32).unwrap();
                legacy.port_write(0x71, 1, 0x20).unwrap();
                legacy.port_write(0x70, 1, 0).unwrap();
            }
            let mut assembled = assemble_synthetic(&config).unwrap();
            let index = assembled.device_index[&DeviceId::Rtc];
            let actual = assembled
                .machine
                .bus_mut()
                .device_mut(index)
                .unwrap()
                .save();
            assert_eq!(
                actual,
                legacy.save(),
                "includes every backing byte and calendar field"
            );
            let registers = rtc_registers(&config);
            for (index, value) in registers.into_iter().enumerate() {
                legacy.port_write(0x70, 1, index as u32).unwrap();
                assert_eq!(
                    value,
                    legacy.port_read(0x71, 1).unwrap() as u8,
                    "register {index:#x}"
                );
            }
        }
    }

    #[test]
    fn initialization_non_default_rtc_start_reads_back_through_ports() {
        let mut config = assembly::at_rtc(vec![0xF4], 100, 10);
        rtc_config(&mut config).start = assembly::RtcStart {
            year: 2024,
            month: 2,
            day: 29,
            hour: 23,
            minute: 45,
            second: 56,
        };
        let bytes = rtc_registers(&config);
        for (index, expected) in [
            (0, 0x56),
            (2, 0x45),
            (4, 0x23),
            (6, 5),
            (7, 0x29),
            (8, 0x02),
            (9, 0x24),
        ] {
            assert_eq!(bytes[index], expected, "register {index:#x}");
        }
    }

    #[test]
    fn initialization_period_dates_read_back_and_roll_over_the_century() {
        for (start, next_date) in [
            (
                assembly::RtcStart {
                    year: 1980,
                    month: 1,
                    day: 1,
                    hour: 23,
                    minute: 59,
                    second: 59,
                },
                [4, 0x02, 0x01, 0x80],
            ),
            (
                assembly::RtcStart {
                    year: 1999,
                    month: 12,
                    day: 31,
                    hour: 23,
                    minute: 59,
                    second: 59,
                },
                [7, 0x01, 0x01, 0x00],
            ),
            (
                assembly::RtcStart {
                    year: 1984,
                    month: 2,
                    day: 28,
                    hour: 23,
                    minute: 59,
                    second: 59,
                },
                [4, 0x29, 0x02, 0x84],
            ),
            (
                assembly::RtcStart {
                    year: 2024,
                    month: 2,
                    day: 29,
                    hour: 23,
                    minute: 59,
                    second: 59,
                },
                [6, 0x01, 0x03, 0x24],
            ),
        ] {
            let mut config = assembly::at_rtc(vec![0xF4], 100, 10);
            rtc_config(&mut config).start = start;
            let mut assembled = assemble_synthetic(&config).unwrap();
            let bus = assembled.machine.bus_mut();
            bus.port_write(0x70, 1, 9).unwrap();
            let year = (start.year % 100) as u8;
            assert_eq!(
                bus.port_read(0x71, 1).unwrap() as u8,
                ((year / 10) << 4) | (year % 10)
            );
            bus.tick_due(device::VClock(100));
            for (index, expected) in [6, 7, 8, 9].into_iter().zip(next_date) {
                bus.port_write(0x70, 1, index).unwrap();
                assert_eq!(
                    bus.port_read(0x71, 1).unwrap() as u8,
                    expected,
                    "{start}, register {index}"
                );
            }
        }
    }

    #[test]
    fn initialization_nvram_image_bypasses_derivation_and_uses_fixed_start() {
        let mut config = assembly::bios(vec![0; 32768], None);
        let mut image = std::array::from_fn(|index| (index as u8).wrapping_mul(37));
        // Motorola MC146818 datasheet, Register B: binary, 12-hour encoding.
        image[0x0B] = 0x04;
        let cfg = rtc_config(&mut config);
        cfg.nvram = assembly::NvramSource::Image(image);
        cfg.start = assembly::RtcStart {
            year: 2031,
            month: 12,
            day: 25,
            hour: 13,
            minute: 47,
            second: 59,
        };
        let bytes = rtc_registers(&config);
        for index in (0..64).filter(|index| ![0, 2, 4, 6, 7, 8, 9, 10, 12, 13].contains(index)) {
            assert_eq!(bytes[index], image[index], "register {index:#x}");
        }
        assert_eq!(
            [bytes[0], bytes[2], bytes[4], bytes[6], bytes[7], bytes[8], bytes[9]],
            [59, 47, 0x81, 5, 25, 12, 31]
        );
        // Motorola MC146818 datasheet, Registers A and D: live UIP and VRT.
        assert_eq!(bytes[10], image[10] & 0x7F);
        assert_eq!(bytes[12], 0);
        assert_eq!(bytes[13], 0x80);
    }

    #[test]
    fn observation_rejects_missing_or_mismatched_capabilities() {
        use assembly::{ObservationPolicy, ObservationRequest, OutputKind};
        let config = assembly::at_boot(vec![0xF4]);
        for (device, output) in [
            (DeviceId::Uart, OutputKind::TextScanout),
            (
                DeviceId::Vga,
                OutputKind::Bytes(device::ByteStreamKind::SerialTx),
            ),
            (
                DeviceId::Uart,
                OutputKind::Bytes(device::ByteStreamKind::PostCheckpoints),
            ),
            (
                DeviceId::Post,
                OutputKind::Bytes(device::ByteStreamKind::PostCheckpoints),
            ),
        ] {
            let policy = ObservationPolicy {
                trace: device::TracePolicy::Off,
                routes: vec![ObservationRequest {
                    device,
                    output,
                    byte_limit: 8,
                }],
                ..Default::default()
            };
            assert!(matches!(
                observe_synthetic(&config, &policy),
                Err(AssemblyError::Config(ConfigError::UnsupportedObservationOutput {
                    device: actual_device, output: actual_output,
                })) if actual_device == device && actual_output == output
            ));
        }
    }
    use assembly::BoardModel;
    use assembly::ClockConfig;
    use assembly::DriveConfig;
    use assembly::LoadImage;
    use assembly::{
        CmosCfg, DeviceConfig, DmaCfg, DmaSpec, FdcCfg, I8042Cfg, PicCfg, PitCfg, PortWindow,
        PostPortCfg, SysControlCfg, UartCfg, VgaTextCfg,
    };
    use assembly::{CpuConfig, ResetSpec, SegmentSeed};
    use assembly::{MemoryConfig, MmioWindow, RamRegion};
    use cpu::Cpu;
    use device::{DmaOp, DmaOutcome};

    const SPACE: u64 = 0x10_0000;
    const TEXT_BASE: u64 = 0xB_8000;
    const TEXT_LEN: u64 = 0x8000;
    const ROM_BASE: u64 = 0xF_8000;
    const ROM_LEN: u64 = 0x8000;

    /// A reference AT-class machine: a small stand-in for the real presets
    /// covering the memory map and device wiring. Everything the assembler must build — the memory mosaic, the
    /// full device set with the cascaded PIC, the trapped text buffer, the floppy
    /// controller with its DMA, and a seeded CMOS — is present.
    fn reference_config() -> MachineConfig {
        let devices = vec![
            DeviceConfig::new(
                DeviceId::Uart,
                DeviceKind::Uart(UartCfg {
                    base: 0x3F8,
                    mouse: false,
                }),
                vec![PortWindow::new(0x3F8, 0x3FF)],
            ),
            DeviceConfig::new(
                DeviceId::Pit,
                DeviceKind::Pit(PitCfg {}),
                vec![PortWindow::new(0x40, 0x43)],
            )
            .with_irq(0),
            DeviceConfig::new(
                DeviceId::MasterPic,
                DeviceKind::Pic(PicCfg { cascade: true }),
                vec![PortWindow::new(0x20, 0x21)],
            ),
            DeviceConfig::new(
                DeviceId::SlavePic,
                DeviceKind::Pic(PicCfg { cascade: false }),
                vec![PortWindow::new(0xA0, 0xA1)],
            ),
            DeviceConfig::new(
                DeviceId::Keyboard,
                DeviceKind::Keyboard(I8042Cfg::default()),
                vec![PortWindow::single(0x60), PortWindow::single(0x64)],
            )
            .with_irq(1),
            DeviceConfig::new(
                DeviceId::Rtc,
                DeviceKind::Rtc(CmosCfg {
                    ram: assembly::CmosRam::Bytes64,
                    nvram: assembly::NvramSource::Derived,
                    start: assembly::RtcStart::default(),
                    timing: None,
                }),
                vec![PortWindow::new(0x70, 0x71)],
            )
            .with_irq(8),
            DeviceConfig::new(
                DeviceId::Vga,
                DeviceKind::Vga(VgaTextCfg {}),
                vec![PortWindow::new(0x3D4, 0x3D5)],
            ),
            DeviceConfig::new(
                DeviceId::SysControl,
                DeviceKind::SysControl(SysControlCfg {}),
                vec![PortWindow::single(0x61)],
            ),
            DeviceConfig::new(
                DeviceId::Post,
                DeviceKind::Post(PostPortCfg { port: 0x80 }),
                vec![PortWindow::single(0x80)],
            ),
            DeviceConfig::new(
                DeviceId::Fdc,
                DeviceKind::Fdc(FdcCfg { drive_slots: 2 }),
                vec![PortWindow::new(0x3F0, 0x3F7)],
            )
            .with_irq(6)
            .with_dma(DmaSpec {
                controller: DeviceId::Dma,
                channel: 2,
            }),
            DeviceConfig::new(
                DeviceId::Dma,
                DeviceKind::Dma(DmaCfg {}),
                // 0x81..=0x8F is the page-latch block; it sits above the POST
                // latch at 0x80, so the two never share a byte.
                vec![PortWindow::new(0x00, 0x0F), PortWindow::new(0x81, 0x8F)],
            ),
        ];

        let memory = MemoryConfig {
            address_space_bytes: SPACE,
            ram: vec![
                RamRegion::new(0, TEXT_BASE),
                RamRegion::new(TEXT_BASE + TEXT_LEN, ROM_BASE - (TEXT_BASE + TEXT_LEN)),
            ],
            mmio: vec![MmioWindow::new(TEXT_BASE, TEXT_LEN, DeviceId::Vga)],
        };

        let images = vec![LoadImage {
            base: ROM_BASE,
            source: ImageSource::Inline(vec![0u8; ROM_LEN as usize]),
            kind: ImageKind::Rom,
            provides: Vec::new(),
        }];

        let drives = vec![DriveConfig {
            media: None,
            kind: DriveKind::Floppy {
                geometry: FloppyGeometry::HD_1_44,
            },
            image: ImageSource::Blank {
                size: FloppyGeometry::HD_1_44.image_bytes(),
            },
            controller: DeviceId::Fdc,
            unit: 0,
            write: WriteMode::Volatile,
            expected_sha256: None,
        }];

        MachineConfig {
            system_bios_image: None,
            board: BoardModel::Bare,
            display_switch: assembly::DisplaySwitch::Auto,
            cpu: CpuConfig {
                fpu: assembly::FpuChip::None,
                chip: assembly::CpuChip::I80286,
                reset: ResetSpec::reset_vector(),
                segments: SegmentSeed::flat(0xF000),
            },
            clock: ClockConfig::default(),
            memory,
            devices,
            drives,
            images,
        }
    }

    #[test]
    fn reference_config_assembles() {
        assert!(assemble_synthetic(&reference_config()).is_ok());
    }

    #[test]
    fn access_couplings_resolve_roles_after_device_reordering() {
        for reverse in [false, true] {
            let mut config = reference_config();
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Uart)
                .unwrap()
                .access_dependents = vec![DeviceId::Pit];
            if reverse {
                config.devices.reverse();
            }
            let mut assembled = assemble_synthetic(&config).unwrap();
            let pit_index = assembled.device_index[&DeviceId::Pit];
            let rtc_index = assembled.device_index[&DeviceId::Rtc];
            let bus = assembled.machine.bus_mut();
            let pit_before = bus.device(pit_index).unwrap().save();
            let rtc_before = bus.device(rtc_index).unwrap().save();
            bus.tick_due(device::VClock(100));
            assert_eq!(bus.device(pit_index).unwrap().save(), pit_before);
            bus.port_write(0x3F8, 1, b'A' as u32).unwrap();
            let mut expected = devices::pit8253::SharedPit8253::default();
            let _sys = SysControl::new().with_pit(expected.clone());
            expected.sync(device::VClock(100));
            assert_ne!(expected.save(), pit_before);
            assert_eq!(bus.device(pit_index).unwrap().save(), expected.save());
            assert_eq!(bus.device(rtc_index).unwrap().save(), rtc_before);
        }
    }

    #[test]
    fn non_step_bounded_cpu_is_a_clear_configuration_error() {
        let mut cpu = X86JitCpu::new(0x10000, 0x1000, &[0xF4]).unwrap();
        let error = validate_step_cpu(&mut cpu).unwrap_err();
        assert!(matches!(error, ConfigError::StepBoundedCpuRequired(_)));
        assert!(error
            .to_string()
            .contains("Real16 CPU with step-bounded execution"));
        assert_eq!(cpu.retired(), 0);
    }

    #[test]
    fn assembly_rejects_cpu_without_step_bounds_before_execution() {
        let error = assemble_with_cpu(
            &reference_config(),
            &ObservationPolicy::default(),
            assembly::AssemblyOptions::default(),
            |_| Ok(X86JitCpu::new(0x10000, 0x1000, &[0xF4])?),
        );
        assert!(matches!(
            error,
            Err(AssemblyError::Config(ConfigError::StepBoundedCpuRequired(
                _
            )))
        ));
    }

    #[test]
    fn every_preset_applies_trace_policy_to_every_device() {
        let configs = [
            assembly::at_boot(vec![0xF4]),
            assembly::at_keyboard(vec![0xF4]),
            assembly::at_rtc(vec![0xF4], 64, 8),
            assembly::at_fdc_write(vec![0xF4], vec![0; 512], 0x600),
            assembly::at_vga_text(vec![0xF4]),
            assembly::test386(vec![0; 65536]),
            assembly::floppy_boot(vec![0xF4], vec![0; 1_474_560]),
            assembly::bios(vec![0; 32768], None),
            assembly::bios(vec![0; 32768], Some(vec![0; 1_474_560])),
        ];
        let policy = ObservationPolicy {
            trace: device::TracePolicy::Ring { capacity: 0 },
            ..ObservationPolicy::default()
        };
        for config in configs {
            let mut assembled = observe_synthetic(&config, &policy).unwrap();
            for dc in &config.devices {
                let port = if dc.id == DeviceId::Fdc {
                    0x3F4
                } else {
                    dc.ports[0].start
                };
                let index = assembled.device_index[&dc.id];
                if dc.ports.is_empty() {
                    // The no-media BIOS keeps an undecoded FDC only for CMOS derivation.
                    assembled
                        .machine
                        .bus_mut()
                        .device_mut(index)
                        .unwrap()
                        .port_read(port, 1)
                        .unwrap();
                } else {
                    assembled.machine.bus_mut().port_read(port, 1).unwrap();
                }
                let device = assembled.machine.bus_mut().device_mut(index).unwrap();
                assert!(
                    device.trace_dropped() > 0,
                    "{:?} ignored zero-capacity recording",
                    dc.id
                );
                assert!(device.take_trace().is_empty());
            }
        }
    }

    #[test]
    fn configured_cpu_rate_drives_assembled_clock() {
        for (numerator, denominator, clocks) in [(1, 1, 4), (2, 1, 2), (1, 3, 12), (3, 2, 2)] {
            let mut config = reference_config();
            config.clock.retired_per_pit_clock = numerator;
            config.clock.retired_per_pit_clock_denominator = denominator;
            // Two MOVs and CLI; HLT retire four instructions without device access.
            config.images[0].source = ImageSource::Inline({
                let mut rom = vec![0; ROM_LEN as usize];
                rom[..6].copy_from_slice(&[0xB0, 0, 0xB0, 0, 0xFA, 0xF4]);
                rom
            });
            config.board = assembly::BoardModel::Bare;
            config.cpu.reset = ResetSpec::Entry { cs: 0xF800, ip: 0 };
            config.cpu.segments = SegmentSeed::flat(0xF800);
            let mut assembled = assemble_synthetic(&config).expect("assembles");
            assembled.machine.run().expect("run");
            assert_eq!(assembled.machine.cpu().retired(), 4);
            assert_eq!(assembled.machine.clock(), device::VClock(clocks));
        }
    }

    #[test]
    fn invalid_cpu_rates_are_rejected_before_assembly() {
        for (numerator, denominator) in [(0, 1), (1, 0), (0, 0)] {
            let mut config = reference_config();
            config.clock.retired_per_pit_clock = numerator;
            config.clock.retired_per_pit_clock_denominator = denominator;
            assert!(matches!(
                assemble_synthetic(&config),
                Err(AssemblyError::Config(ConfigError::InvalidCpuRate { .. }))
            ));
        }
    }

    #[test]
    fn device_index_map_follows_config_order() {
        let assembled = assemble_synthetic(&reference_config()).expect("assembles");
        let map = &assembled.device_index;
        // The devices land in config order, each role at its own bus slot; the
        // master and slave PIC faces are two distinct slots of the one live chip.
        assert_eq!(map[&DeviceId::Uart], 0);
        assert_eq!(map[&DeviceId::Pit], 1);
        assert_eq!(map[&DeviceId::MasterPic], 2);
        assert_eq!(map[&DeviceId::SlavePic], 3);
        assert_eq!(map[&DeviceId::Keyboard], 4);
        assert_eq!(map[&DeviceId::Rtc], 5);
        assert_eq!(map[&DeviceId::Vga], 6);
        assert_eq!(map[&DeviceId::SysControl], 7);
        assert_eq!(map[&DeviceId::Post], 8);
        assert_eq!(map[&DeviceId::Fdc], 9);
        assert_eq!(map[&DeviceId::Dma], 10);
        assert_eq!(map.len(), 11);
    }

    fn assert_bus_port_read_owner(
        assembled: &mut AssembledMachine,
        port: u16,
        expected: Option<DeviceId>,
    ) {
        let bus = assembled.machine.bus_mut();
        bus.set_trace_policy(device::TracePolicy::Ring { capacity: 16 });
        bus.port_read(port, 1).expect("port read succeeds");
        // An open-bus value alone cannot distinguish an unused FDC port from
        // the POST latch or an undecoded port; the recipient's trace can.
        let mut owners = Vec::new();
        for (&id, &index) in &assembled.device_index {
            for event in bus.device_mut(index).unwrap().take_trace() {
                if matches!(event, device::TraceEvent::Port {
                    port: actual, size: 1, dir: device::AccessDir::Read, ..
                } if actual == port)
                {
                    owners.push(id);
                }
            }
        }
        assert_eq!(
            owners,
            expected.into_iter().collect::<Vec<_>>(),
            "port {port:#x}"
        );
        let open_reads = bus
            .take_open_bus_trace()
            .into_iter()
            .filter(|event| {
                matches!(event, device::TraceEvent::Port {
                port: actual, size: 1, dir: device::AccessDir::Read, ..
            } if *actual == port)
            })
            .count();
        assert_eq!(
            open_reads,
            usize::from(expected.is_none()),
            "port {port:#x}"
        );
    }

    #[test]
    fn split_fdc_leaves_primary_control_with_its_owner() {
        let mut config = reference_config();
        let post = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Post)
            .unwrap();
        post.kind = DeviceKind::Post(PostPortCfg { port: 0x3F6 });
        post.ports = vec![PortWindow::single(0x3F6)];
        let fdc = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Fdc)
            .unwrap();
        fdc.ports = FdcCfg::port_windows(true);
        assert_eq!(
            fdc.ports,
            vec![PortWindow::new(0x3F0, 0x3F5), PortWindow::single(0x3F7)]
        );
        assert!(config
            .devices
            .iter()
            .all(|d| !d.allow_port_overlap && d.ports.iter().all(|w| !w.allow_overlap)));
        let mut assembled =
            assemble_synthetic(&config).expect("split FDC assembles alongside primary control");
        for (port, owner) in [
            (0x3F5, DeviceId::Fdc),
            (0x3F6, DeviceId::Post),
            (0x3F7, DeviceId::Fdc),
        ] {
            assert_bus_port_read_owner(&mut assembled, port, Some(owner));
        }
    }

    #[test]
    fn same_start_post_dma_overlap_gives_post_priority() {
        for reverse_devices in [false, true] {
            for flag_on_dma in [false, true] {
                let mut config = reference_config();
                let dma = config
                    .devices
                    .iter_mut()
                    .find(|d| d.id == DeviceId::Dma)
                    .unwrap();
                dma.ports[1] = PortWindow::new(0x80, 0x8F);
                let flagged_id = if flag_on_dma {
                    DeviceId::Dma
                } else {
                    DeviceId::Post
                };
                let flagged = config
                    .devices
                    .iter_mut()
                    .find(|d| d.id == flagged_id)
                    .unwrap();
                let window = if flag_on_dma { 1 } else { 0 };
                flagged.ports[window].allow_overlap = true;
                if reverse_devices {
                    config.devices.reverse();
                }
                assert_eq!(config.validate_host(), Ok(()));
                let mut assembled =
                    assemble_synthetic(&config).expect("same-start overlap assembles");
                for (port, owner) in [(0x80, DeviceId::Post), (0x81, DeviceId::Dma)] {
                    assert_bus_port_read_owner(&mut assembled, port, Some(owner));
                }
            }
        }
    }

    #[test]
    fn preset_configs_and_fdc_decode_keep_legacy_windows() {
        let configs = [
            ("at_boot", assembly::at_boot(vec![0xF4])),
            ("at_keyboard", assembly::at_keyboard(vec![0xF4])),
            ("at_rtc", assembly::at_rtc(vec![0xF4], 64, 8)),
            (
                "at_fdc_write",
                assembly::at_fdc_write(vec![0xF4], vec![0; 512], 0x600),
            ),
            ("at_vga_text", assembly::at_vga_text(vec![0xF4])),
            ("test386", assembly::test386(vec![0; 65536])),
            (
                "floppy_boot",
                assembly::floppy_boot(vec![0xF4], vec![0; 1_474_560]),
            ),
            ("bios_without_floppy", assembly::bios(vec![0; 32768], None)),
            (
                "bios_with_floppy",
                assembly::bios(vec![0; 32768], Some(vec![0; 1_474_560])),
            ),
        ];
        for (name, config) in configs {
            let mut legacy = config.clone();
            for fdc in legacy
                .devices
                .iter_mut()
                .filter(|d| matches!(d.kind, DeviceKind::Fdc(_)))
            {
                // Literal legacy wiring is independent of the shared window builder.
                fdc.ports = if name == "bios_without_floppy" {
                    Vec::new()
                } else {
                    vec![PortWindow::new(0x3F0, 0x3F7)]
                };
            }
            assert_eq!(config, legacy, "{name}");
            assert_eq!(config.validate_host(), Ok(()));
            let mut assembled = assemble_synthetic(&config).expect("preset assembles");
            let expected_owner = match name {
                "at_fdc_write" | "floppy_boot" | "bios_with_floppy" => Some(DeviceId::Fdc),
                _ => None,
            };
            for port in 0x3F0..=0x3F7 {
                assert_bus_port_read_owner(&mut assembled, port, expected_owner);
            }
        }
    }

    #[test]
    fn isa_decode_map_preserves_each_windows_width() {
        let mut config = reference_config();
        let index = config
            .devices
            .iter()
            .enumerate()
            .map(|(index, device)| (device.id, index))
            .collect();
        let byte_map = build_decode_map(&config, &index);
        assert!(byte_map
            .ports
            .iter()
            .all(|range| range.width == bus::PortWidth::Byte));
        let post = config
            .devices
            .iter_mut()
            .find(|device| device.id == DeviceId::Post)
            .unwrap();
        post.ports = vec![
            PortWindow::single(0x190).with_width(bus::PortWidth::Word),
            PortWindow::single(0x191),
        ];
        let map = build_decode_map(&config, &index);
        for (port, width) in [(0x190, bus::PortWidth::Word), (0x191, bus::PortWidth::Byte)] {
            let range = map
                .ports
                .iter()
                .find(|range| range.ports.contains(&port))
                .unwrap();
            assert_eq!(range.width, width);
            assert_eq!(range.device, index[&DeviceId::Post]);
        }
    }

    #[test]
    fn decode_map_routes_key_ports_and_mmio() {
        let config = reference_config();
        let index = {
            // Rebuild the index the same way assemble does, to assert the map.
            let assembled = assemble_synthetic(&config).expect("assembles");
            assembled.device_index
        };
        let map = build_decode_map(&config, &index);

        let port_owner = |port: u16| -> Option<usize> {
            map.ports
                .iter()
                .find(|r| r.ports.contains(&port))
                .map(|r| r.device)
        };
        assert_eq!(port_owner(0x3F8), Some(index[&DeviceId::Uart])); // COM1
        assert_eq!(port_owner(0x20), Some(index[&DeviceId::MasterPic]));
        assert_eq!(port_owner(0xA0), Some(index[&DeviceId::SlavePic]));
        assert_eq!(port_owner(0x80), Some(index[&DeviceId::Post])); // POST latch
        assert_eq!(port_owner(0x81), Some(index[&DeviceId::Dma])); // page latch
        assert_eq!(port_owner(0x60), Some(index[&DeviceId::Keyboard]));

        // Ports are sorted ascending, a pure function of the config.
        let starts: Vec<u16> = map.ports.iter().map(|r| *r.ports.start()).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted);

        // The text buffer at 0xB8000 traps to the VGA card.
        let vga = index[&DeviceId::Vga];
        assert!(map
            .mmio
            .iter()
            .any(|r| r.device == vga && r.addrs.contains(&TEXT_BASE)));
    }

    #[test]
    fn wiring_tables_resolve_ids_to_indices() {
        let config = reference_config();
        let assembled = assemble_synthetic(&config).expect("assembles");
        let index = &assembled.device_index;

        let irqs = build_irq_lines(&config, index);
        // PIT→0, keyboard→1, RTC→8, FDC→6, each resolved to its bus slot.
        assert!(irqs
            .iter()
            .any(|l| l.device == index[&DeviceId::Pit] && l.irq == 0));
        assert!(irqs
            .iter()
            .any(|l| l.device == index[&DeviceId::Rtc] && l.irq == 8));
        assert!(irqs
            .iter()
            .any(|l| l.device == index[&DeviceId::Fdc] && l.irq == 6));

        let (dma, channels) = build_dma_wirings(&config, index);
        assert!(channels.is_empty());
        assert_eq!(dma.len(), 1);
        assert_eq!(dma[0].requester, index[&DeviceId::Fdc]);
        assert_eq!(dma[0].controller, index[&DeviceId::Dma]);
    }

    #[test]
    fn all_word_dma_specs_translate_system_channels_at_assembly() {
        let mut config = reference_config();
        config.devices.push(assembly::DeviceConfig::new(
            DeviceId::DmaWord,
            DeviceKind::DmaWord(assembly::DmaCfg {}),
            Vec::new(),
        ));
        let cfg = assembly::Sb16Cfg {
            base: 0x220,
            dma16: 5,
        };
        config.devices.push(
            assembly::DeviceConfig::new(DeviceId::Sb16, DeviceKind::Sb16(cfg), cfg.port_windows())
                .with_irq(5)
                .with_dma(assembly::DmaSpec {
                    controller: DeviceId::Dma,
                    channel: 1,
                }),
        );
        // A generic wiring owner exercises the same rule without relying on
        // a particular device's byte-DMA request format.
        config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Post)
            .unwrap()
            .dma = Some(assembly::DmaSpec {
            controller: DeviceId::DmaWord,
            channel: 5,
        });
        let index: BTreeMap<_, _> = config
            .devices
            .iter()
            .enumerate()
            .map(|(i, d)| (d.id, i))
            .collect();
        for channel in 5..=7 {
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Post)
                .unwrap()
                .dma
                .as_mut()
                .unwrap()
                .channel = channel;
            let (_, routes) = build_dma_wirings(&config, &index);
            assert!(routes.contains(&DmaChannelWiring {
                requester: index[&DeviceId::Post],
                system_channel: channel,
                controller: index[&DeviceId::DmaWord],
                local_channel: channel - 4
            }));
            let route = routes
                .iter()
                .find(|route| route.requester == index[&DeviceId::Post])
                .unwrap();
            struct Memory;
            impl device::DmaMemory for Memory {
                fn dma_read(&self, addr: u32) -> u8 {
                    match addr {
                        0x1000 => 0x34,
                        0x1001 => 0x12,
                        _ => panic!("unexpected DMA address {addr:x}"),
                    }
                }
                fn dma_write(&mut self, _: u32, _: u8) {
                    unreachable!()
                }
            }
            let mut dma = Dma8237::new_at_word();
            let local = route.local_channel;
            for (port, value) in [
                (0xD8, 0),
                (0xC0 + u16::from(local) * 4, 0),
                (0xC0 + u16::from(local) * 4, 8),
                (0xC2 + u16::from(local) * 4, 0),
                (0xC2 + u16::from(local) * 4, 0),
                (0xD6, 0x48 | local),
                (0xD4, local),
            ] {
                dma.port_write(port, 1, value.into()).unwrap();
            }
            let mut bytes = [0; 2];
            let transfer = dma.dma_service_read(local, &mut Memory, &mut bytes);
            assert_eq!(transfer.transferred, 2);
            assert_eq!(bytes, [0x34, 0x12]);
            assert!(routes.contains(&DmaChannelWiring {
                requester: index[&DeviceId::Sb16],
                system_channel: 5,
                controller: index[&DeviceId::DmaWord],
                local_channel: 1
            }));
        }
    }

    #[test]
    fn derived_cmos_reference_colour_text_matches_explicit_equipment() {
        let derived = derive_cmos_config(&reference_config()).unwrap();
        let mut expected_rtc = CmosRtc::new().with_config(devices::CmosConfig {
            equipment: 0x21,
            ..devices::CmosConfig::default()
        });
        // CmosConfig leaves 32h at 00h; derived CMOS seeds the 2000 start-year century.
        expected_rtc.port_write(0x70, 1, 0x32).unwrap();
        expected_rtc.port_write(0x71, 1, 0x20).unwrap();
        let expected: [u8; 64] = std::array::from_fn(|index| {
            expected_rtc.port_write(0x70, 1, index as u32).unwrap();
            expected_rtc.port_read(0x71, 1).unwrap() as u8
        });
        assert_eq!(derived[0x0A..], expected[0x0A..]);
    }

    #[test]
    fn assembled_cpu_maps_are_live() {
        // The ROM is loaded at its base and reads back through the CPU seam; low
        // RAM is writable. This proves the memory mosaic was laid out, without a
        // full boot.
        let mut assembled = assemble_synthetic(&reference_config()).expect("assembles");
        let cpu = assembled.machine.cpu();
        let mut rom = [0xFFu8; 4];
        cpu.read_mem(ROM_BASE, &mut rom).expect("rom readable");
        assert_eq!(rom, [0u8; 4]);

        let bus = assembled.machine.bus_mut();
        // The bus holds the full device set the config named.
        assert!(bus.device_mut(0).is_some());
        assert!(bus.device_mut(10).is_some());
    }

    #[test]
    fn guest_word_access_straddles_mda_and_colour_scanout() {
        for reverse_windows in [false, true] {
            let mut config = assembly::bios(vec![0; 0x8000], None);
            if reverse_windows {
                config.memory.mmio.reverse();
            }
            config.cpu.reset = ResetSpec::Entry { cs: 0, ip: 0x7C00 };
            config.cpu.segments = SegmentSeed::flat(0);
            config.cpu.segments.ds = 0xB000;
            config.images.push(LoadImage {
                base: 0x7C00,
                source: ImageSource::Inline(vec![
                    0xFA, 0xC7, 0x06, 0xFF, 0x7F, 0x34, 0x12, 0xA1, 0xFF, 0x7F, 0xF4,
                ]),
                kind: ImageKind::RamLoad,
                provides: Vec::new(),
            });
            let mut assembled = assemble_synthetic(&config).unwrap();
            assert_eq!(
                assembled.machine.run_bounded(Some(100)).unwrap(),
                machine::RunOutcome::Halted
            );
            assert_eq!(assembled.machine.cpu().reg(cpu::CpuReg::Rax), 0x1234);
            assembled.machine.bus_mut().settle();
            let scanout = |card| {
                assembled
                    .machine
                    .bus()
                    .device(assembled.device_index[&card])
                    .unwrap()
                    .text_scanout()
                    .unwrap()
            };
            let mda = scanout(DeviceId::Mda);
            let colour = scanout(DeviceId::Vga);
            assert_eq!(mda.vram.last(), Some(&0x34));
            assert_eq!(mda.vram[0], 0);
            assert_eq!(&colour.vram[..2], &[0x12, 0]);
        }
    }

    #[test]
    fn i286_reset_fetch_uses_the_single_board_high_alias() {
        use cpu::{Cpu, CpuReg, CpuStepStop, CpuStop, StepRunOptions};
        let mut rom = vec![0; 0x8000];
        rom[0x7FF0..0x7FF3].copy_from_slice(&[0xB0, 0x7B, 0xF4]);
        let config = assembly::bios(rom, Some(vec![0; 1_474_560]));
        assert_eq!(config.cpu.chip.physical_address_bits(), 24);
        let mut regions = Vec::new();
        let mut cpu = build_cpu_with_mapping_trace(&config, |base, end, kind| {
            regions.push((base, end, kind));
        })
        .unwrap();
        let high: Vec<_> = regions
            .into_iter()
            .filter(|(base, _, _)| *base >= 0xFE0000)
            .collect();
        assert_eq!(high, [(0xFE0000, 0x1000000, "rom")]);
        let mut empty = [0];
        cpu.read_mem(0x200000, &mut empty).unwrap();
        assert_eq!(empty, [0xFF]);
        assert!(cpu.read_mem(0x1000000, &mut empty).is_err());
        // Host loading can distinguish the copied alias from its low source;
        // guest stores remain unable to change either ROM mapping.
        cpu.write_mem(0xFFFF0, &[0xB0, 0x11, 0xF4]).unwrap();
        assert_eq!(
            cpu.run_steps(2, StepRunOptions::default()).unwrap().stop,
            CpuStepStop::Exit(CpuStop::Halt)
        );
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFF, 0x7B);
    }

    #[test]
    fn lotech_ems_page_write_reaches_the_next_instruction_and_survives_cpu_reset() {
        use cpu::{Cpu, CpuReg};
        let program = vec![
            0xFA, 0xB8, 0, 0xD0, 0x8E, 0xD8, 0xBA, 0x60, 2, 0xB0, 127, 0xEE, 0xC6, 6, 0, 0, 0x5A,
            0xB0, 0, 0xEE, 0xA0, 0, 0, 0x88, 0xC3, 0xB0, 127, 0xEE, 0xA0, 0, 0, 0x88, 0xC7, 0xB0,
            0xFF, 0xEE, 0xA0, 0, 0, 0xF4,
        ];
        let mut config = assembly::at_boot(program);
        config.memory.address_space_bytes = 0x100000;
        let cfg = assembly::LotechEmsCfg {
            frame: 0xD000,
            ..Default::default()
        };
        config.devices.push(DeviceConfig::new(
            DeviceId::LotechEms,
            DeviceKind::LotechEms(cfg),
            cfg.port_windows(),
        ));
        config
            .memory
            .mmio
            .push(MmioWindow::new(0xD0000, 0x10000, DeviceId::LotechEms));
        let mut rig = assemble(&config).unwrap();
        assert_eq!(
            rig.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        assert_eq!(rig.machine.cpu().reg(CpuReg::Rbx) & 0xFFFF, 0x5A00);
        assert_eq!(rig.machine.cpu().reg(CpuReg::Rax) & 0xFF, 0xFF);
        let mut board = assembly::bios(vec![0xF4; 0x8000], Some(vec![0; 1_474_560]));
        board.devices.push(DeviceConfig::new(
            DeviceId::LotechEms,
            DeviceKind::LotechEms(cfg),
            cfg.port_windows(),
        ));
        board
            .memory
            .mmio
            .push(MmioWindow::new(0xD0000, 0x10000, DeviceId::LotechEms));
        let mut rig = assemble_synthetic(&board).unwrap();
        assert_eq!(
            rig.machine.cpu().guest_dma_read(0xD0000),
            cpu::DmaAccess::Mmio
        );
        assert_eq!(
            rig.machine.cpu().guest_dma_read(0xFE0000),
            cpu::DmaAccess::Complete(0)
        );
        rig.machine.bus_mut().port_write(0xFE60, 1, 0).unwrap();
        rig.machine.bus_mut().mmio_write(0xD0000, 1, 0x42).unwrap();
        assert_eq!(rig.machine.bus_mut().mmio_read(0xD0000, 1).unwrap(), 0x42);
        assert_eq!(
            rig.machine.cpu().guest_dma_read(0xFE0000),
            cpu::DmaAccess::Complete(0)
        );
        let index = rig.device_index[&DeviceId::LotechEms];
        let state = rig.machine.bus().device(index).unwrap().save();
        rig.machine.reset_cpu().unwrap();
        assert_eq!(rig.machine.bus().device(index).unwrap().save(), state);
        assert_eq!(rig.machine.bus_mut().mmio_read(0xD0000, 1).unwrap(), 0x42);
    }

    #[test]
    fn firmware_high_alias_matches_low_memory_and_drops_dma_writes() {
        use cpu::Cpu;
        let mut config = assembly::bios(vec![0xA5; 0x10000], Some(Vec::new()));
        config.images[0].base = 0xF0000;
        config.images.push(LoadImage {
            base: 0xE2000,
            source: ImageSource::Inline(vec![0x5A; 0x2000]),
            kind: ImageKind::Rom,
            provides: Vec::new(),
        });
        config.cpu.reset = ResetSpec::Entry { cs: 0, ip: 0x7C00 };
        config.cpu.segments = SegmentSeed::flat(0);
        let mut cpu = build_cpu(&config).unwrap();
        for low in [
            0xE0000, 0xE1FFF, 0xE2000, 0xE3FFF, 0xE4000, 0xEFFFF, 0xF0000, 0xFFFF0, 0xFFFFF,
        ] {
            let mut expected = [0];
            let mut alias = [0];
            cpu.read_mem(low, &mut expected).unwrap();
            cpu.read_mem(low + 0xF00000, &mut alias).unwrap();
            assert_eq!(alias, expected, "low address {low:#x}");
        }
        let mut boundary = [0; 2];
        cpu.read_mem(0xFE3FFF, &mut boundary).unwrap();
        assert_eq!(boundary, [0x5A, 0]);
        assert_eq!(
            cpu.guest_dma_write(0xFFFFF0, 0),
            cpu::DmaAccess::Complete(())
        );
        let mut value = [0; 2];
        cpu.read_mem(0xFFFFF0, &mut value).unwrap();
        assert_eq!(value, [0xA5; 2]);
    }

    #[test]
    fn firmware_region_boundary_words_span_adjacent_regions() {
        use cpu::{Cpu, CpuError, CpuReg, CpuStop};
        // A guest word whose bytes lie in adjacent mapped regions (ROM or RAM,
        // then the empty-bus fill) completes as one bus access would. The host
        // read path still needs one region holding the whole span.
        for (extended, offset, address) in [(0, 0x000Fu16, 0xFFFFF), (0x20, 0x002F, 0x10001F)] {
            let mut config = assembly::bios(vec![0; 0x8000], Some(Vec::new()));
            config.cpu.reset = ResetSpec::Entry { cs: 0, ip: 0x7C00 };
            config.cpu.segments = SegmentSeed::flat(0);
            if extended > 0 {
                config.memory.ram.push(RamRegion::new(0x100000, extended));
            }
            let mut cpu = build_cpu(&config).unwrap();
            let mut program = vec![0xB8, 0xFF, 0xFF, 0x8E, 0xD8, 0xA1];
            program.extend(offset.to_le_bytes());
            program.push(0xF4);
            cpu.write_mem(0x7C00, &program).unwrap();
            for byte in [address, address + 1] {
                cpu.read_mem(byte, &mut [0]).unwrap();
            }
            let result = cpu.run(Some(100));
            assert!(
                matches!(result, Ok(CpuStop::Halt)),
                "{address:#x}: {result:?}"
            );
            assert_eq!(cpu.reg(CpuReg::Rax) & 0xFFFF, 0xFF00, "{address:#x}");
        }
        let mut config = assembly::bios(vec![0; 0x8000], Some(Vec::new()));
        config
            .memory
            .ram
            .push(RamRegion::new(0x100000, 15232 * 1024));
        let cpu = build_cpu(&config).unwrap();
        cpu.read_mem(0xFDFFFF, &mut [0]).unwrap();
        cpu.read_mem(0xFE0000, &mut [0]).unwrap();
        assert!(
            matches!(cpu.read_mem(0xFDFFFF, &mut [0; 2]), Err(CpuError::Memory(message)) if message == "Unmapped")
        );
    }

    #[test]
    fn firmware_memory_reports_extended_limit_before_alias_overlap() {
        let mut config = assembly::bios(vec![0; 0x8000], Some(Vec::new()));
        config.drives.clear();
        config.memory.address_space_bytes = 0x1000000;
        config.memory.ram.push(RamRegion::new(0x100000, 0));
        for size_kib in [15296, 15360] {
            config.memory.ram.last_mut().unwrap().size = size_kib * 1024;
            let error = config.validate_host().unwrap_err();
            assert!(matches!(
                error,
                ConfigError::ExtendedRamLimit {
                    max_kib: 15232,
                    addr: 0xFE0000,
                    ..
                }
            ));
            assert_eq!(
                error.to_string(),
                "board \"ibm-5170\" extended RAM exceeds 15232 KiB at 0xfe0000"
            );
        }
    }

    #[test]
    fn firmware_memory_rejects_mappings_outside_the_at_bus() {
        for (base, size) in [(0x1000000, 1), (0xFFFFFF, 2)] {
            for kind in ["ram", "rom", "mmio"] {
                let mut config = assembly::bios(vec![0; 0x8000], Some(Vec::new()));
                config.drives.clear();
                config.memory.address_space_bytes = 0x1000010;
                match kind {
                    "ram" => config.memory.ram.push(RamRegion::new(base, size)),
                    "rom" => config.images.push(LoadImage {
                        base,
                        source: ImageSource::Inline(vec![0; size as usize]),
                        kind: ImageKind::Rom,
                        provides: Vec::new(),
                    }),
                    "mmio" => config
                        .memory
                        .mmio
                        .push(MmioWindow::new(base, size, DeviceId::Vga)),
                    _ => unreachable!(),
                }
                assert_eq!(config.validate_host().err().map(|error| error.to_string()), Some("board \"ibm-5170\" configured memory at 0x1000000 exceeds the 24-bit physical address space".into()), "{kind}");
            }
        }
    }

    #[test]
    fn firmware_memory_rejects_mmio_in_the_alias_source() {
        for (base, size, address) in [
            (0xE0000, 1, 0xE0000),
            (0xDFFFF, 2, 0xE0000),
            (0xF0000, 1, 0xF0000),
            (0xF7FFF, 1, 0xF7FFF),
        ] {
            let mut config = assembly::bios(vec![0; 0x8000], Some(Vec::new()));
            config.drives.clear();
            config
                .memory
                .mmio
                .push(MmioWindow::new(base, size, DeviceId::Vga));
            assert_eq!(config.validate_host().err().map(|error| error.to_string()), Some(format!("board \"ibm-5170\" MMIO at {address:#x} overlaps the high ROM alias source 0xe0000..0x100000")));
        }
    }

    #[test]
    fn firmware_memory_respects_board_extended_ram_limit() {
        let mut config = assembly::bios(vec![0; 0x8000], Some(Vec::new()));
        config.drives.clear();
        config.memory.address_space_bytes = 0x1000000;
        config
            .memory
            .ram
            .push(RamRegion::new(0x100000, 14208 * 1024));
        for size_kib in [14208, 14272, 15232] {
            config.memory.ram.last_mut().unwrap().size = size_kib * 1024;
            config.validate_host().unwrap();
        }
        assert_eq!(assembly::IBM5170_MAX_EXTENDED_KIB, 15232);
        assert_eq!(config.memory.ram.last().unwrap().end(), 0xFE0000);
        for size_kib in [15296, 15360] {
            config.memory.ram.last_mut().unwrap().size = size_kib * 1024;
            assert!(matches!(
                config.validate_host(),
                Err(ConfigError::ExtendedRamLimit { max_kib: 15232, .. })
            ));
        }
        config.memory.ram.pop();
        for (base, size) in [(0xFE0000, 1), (0xFDFFFF, 2), (0xFFFFFF, 1)] {
            config.memory.ram.push(RamRegion::new(base, size));
            let error = config.validate_host().unwrap_err();
            if base < 0xFE0000 {
                assert!(matches!(error, ConfigError::ExtendedRamLimit { .. }));
            } else {
                assert!(matches!(error, ConfigError::HighRomAliasOverlap { .. }));
            }
            config.memory.ram.pop();
        }
        config
            .memory
            .mmio
            .push(MmioWindow::new(0xFE0000, 1, DeviceId::Vga));
        assert!(matches!(
            config.validate_host(),
            Err(ConfigError::HighRomAliasOverlap { .. })
        ));
        config.memory.mmio.pop();
        config.images.push(LoadImage {
            base: 0xFF0000,
            source: ImageSource::Inline(vec![0; 0x10000]),
            kind: ImageKind::Rom,
            provides: Vec::new(),
        });
        assert!(matches!(
            config.validate_host(),
            Err(ConfigError::HighRomAliasOverlap { .. })
        ));
    }

    #[test]
    fn firmware_open_bus_reads_ff_and_drops_guest_writes() {
        use cpu::{Cpu, CpuReg, CpuStop};
        let mut config = assembly::bios(vec![0; 0x8000], Some(Vec::new()));
        config.cpu.reset = ResetSpec::Entry { cs: 0, ip: 0x7C00 };
        config.cpu.segments = SegmentSeed::flat(0);
        for address in [0x100000u32, 0x7FFFFE, 0xFDFFFE] {
            let mut cpu = build_cpu(&config).unwrap();
            if address == 0x100000 {
                // FFFF:0010 reaches the first absent byte using 286 addressing.
                let program = [
                    0xB8, 0xFF, 0xFF, 0x8E, 0xD8, 0xC7, 0x06, 0x10, 0, 0x34, 0x12, 0xA1, 0x10, 0,
                    0xF4,
                ];
                cpu.write_mem(0x7C00, &program).unwrap();
                assert_eq!(cpu.run(Some(100)).unwrap(), CpuStop::Halt);
                assert_eq!(cpu.reg(CpuReg::Rax) & 0xFFFF, 0xFFFF);
            } else {
                assert_eq!(
                    cpu.guest_dma_write(address, 0x34),
                    cpu::DmaAccess::Complete(())
                );
                assert_eq!(
                    cpu.guest_dma_write(address + 1, 0x12),
                    cpu::DmaAccess::Complete(())
                );
            }
            let mut value = [0; 2];
            cpu.read_mem(u64::from(address), &mut value).unwrap();
            assert_eq!(value, [0xFF; 2]);
        }
    }

    #[test]
    fn firmware_open_bus_completes_map_without_overwriting_regions() {
        let mut config = assembly::bios(vec![0xA5; 0x8000], Some(Vec::new()));
        config.memory.address_space_bytes = 0x1000000;
        config.memory.ram = vec![
            RamRegion::new(0, 0x80000),
            RamRegion::new(0x100000, 0x100000),
        ];
        config
            .memory
            .mmio
            .push(MmioWindow::new(0x300000, 0x1000, DeviceId::Vga));
        let mut regions = Vec::new();
        let cpu = build_cpu_with_mapping_trace(&config, |base, end, kind| {
            regions.push((base, end, kind));
        })
        .unwrap();
        regions.sort_unstable();
        assert_eq!(
            regions,
            vec![
                (0, 0x80000, "ram"),
                (0x80000, 0xB0000, "empty-bus"),
                (0xB0000, 0xC0000, "mmio"),
                (0xC0000, 0xF8000, "rom"),
                (0xF8000, 0x100000, "rom"),
                (0x100000, 0x200000, "ram"),
                (0x200000, 0x300000, "empty-bus"),
                (0x300000, 0x301000, "mmio"),
                (0x301000, 0xFE0000, "empty-bus"),
                (0xFE0000, 0x1000000, "rom"),
            ]
        );
        assert_eq!(regions.first().unwrap().0, 0);
        assert_eq!(regions.last().unwrap().1, 0x1000000);
        assert!(regions.windows(2).all(|pair| pair[0].1 == pair[1].0));
        for (address, expected) in [
            (0x80000, 0xFF),
            (0xC0000, 0),
            (0xFFFF0, 0xA5),
            (0xFFFFF0, 0xA5),
        ] {
            let mut value = [0];
            cpu.read_mem(address, &mut value).unwrap();
            assert_eq!(value, [expected]);
        }
    }

    #[test]
    fn bare_rigs_do_not_gain_firmware_open_bus() {
        let mut config = assembly::bios(vec![0; 0x8000], None);
        for space in [0x100000, 0x1000000] {
            config.memory.address_space_bytes = space;
            let mut regions = Vec::new();
            let cpu = build_cpu_with_mapping_trace(&config, |base, end, kind| {
                regions.push((base, end, kind));
            })
            .unwrap();
            assert!(!regions.iter().any(|region| region.2 == "empty-bus"));
            assert_eq!(regions.iter().map(|region| region.1).max(), Some(0x100000));
            assert!(cpu.read_mem(0x100000, &mut [0]).is_err());
        }
    }

    #[test]
    fn listed_bios_without_scan_window_has_readonly_zero_adapter_gaps() {
        use cpu::{Cpu, CpuReg, CpuStop};
        use sha2::{Digest, Sha256};
        let mut rom = vec![0xF4; 65536];
        rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0, 1, 0, 0xF0]);
        // Empty sockets must remain readable after guest writes, so exercise
        // ordinary CPU stores as well as the initial gap value.
        let program = [
            0xB8, 0, 0xC0, 0x8E, 0xD8, 0xA0, 1, 0, 0xC6, 0x06, 1, 0, 0xA5, 0x8A, 0x1E, 1, 0, 0xF4,
        ];
        rom[0x100..0x100 + program.len()].copy_from_slice(&program);
        let mut config = assembly::bios(rom.clone(), Some(Vec::new()));
        config.drives.clear();
        config.images[0].base = 0xF0000;
        config.images[0].provides.clear();
        let catalogue = [assembly::FirmwareEntry {
            id: "synthetic-listed-diagnostic",
            sha256: Box::leak(format!("{:x}", Sha256::digest(&rom)).into_boxed_str()),
            parts: &[],
            ..assembly::test_support::LANDMARK_ROM_POST_AT_1_2_0
        }];
        assert_eq!(catalogue[0].scan_window, None);
        assert!(assembly::check_system_bios(
            &config,
            config.board,
            &catalogue,
            0xF0000,
            &rom,
            assembly::AssemblyOptions::default(),
        )
        .unwrap()
        .is_none());
        let mut regions = Vec::new();
        let mut cpu = build_cpu_with_mapping_trace(&config, |base, end, kind| {
            regions.push((base, end, kind));
        })
        .unwrap();
        assert_eq!(
            regions,
            vec![
                (0, 0xB0000, "ram"),
                (0xB0000, 0xC0000, "mmio"),
                (0xC0000, 0xF0000, "rom"),
                (0xF0000, 0x100000, "rom"),
                (0x100000, 0xFE0000, "empty-bus"),
                (0xFE0000, 0x1000000, "rom"),
            ]
        );
        assert_eq!(cpu.run(None).unwrap(), CpuStop::Halt);
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFF, 0);
        assert_eq!(cpu.reg(CpuReg::Rbx) & 0xFF, 0);
        for address in [0xC0000, 0xC0001, 0xEFFFF] {
            let mut value = [0xFF];
            cpu.read_mem(address, &mut value).unwrap();
            assert_eq!(value, [0]);
        }
    }

    #[test]
    fn cpu_trap_regions_merge_chains_but_preserve_ram_gaps() {
        let mut config = assembly::bios(vec![0; 0x8000], None);
        config.memory.mmio = vec![
            MmioWindow::new(0xB0005, 3, DeviceId::Mda),
            MmioWindow::new(0xB8000, 0x8000, DeviceId::Vga),
            MmioWindow::new(0xB0000, 2, DeviceId::Mda),
            MmioWindow::new(0xB0002, 3, DeviceId::Mda),
        ];
        config.memory.ram = vec![
            RamRegion::new(0, 0xB0000),
            RamRegion::new(0xB0008, 0xB8000 - 0xB0008),
            RamRegion::new(0xC0000, 0xF8000 - 0xC0000),
        ];
        config.validate_host().unwrap();
        let mut cpu = build_cpu(&config).unwrap();
        let mut bytes = [0xFF; 8];
        cpu.read_mem(0xB0000, &mut bytes).unwrap();
        assert_eq!(bytes, [0; 8]);
        cpu.write_mem(0xB0008, &[0x5A]).unwrap();
        cpu.read_mem(0xB0008, &mut bytes[..1]).unwrap();
        assert_eq!(bytes[0], 0x5A);
        assert!(cpu.read_mem(0xB0007, &mut bytes[..2]).is_err());
        cpu.read_mem(0xB8000, &mut bytes).unwrap();
        assert_eq!(bytes, [0; 8]);
    }

    #[test]
    fn invalid_config_is_rejected_before_assembly() {
        let mut config = reference_config();
        // A stray UART window over the PIT's 0x40 base is an unallowed overlap.
        config.devices[0].ports.push(PortWindow::single(0x40));
        let result = assemble_synthetic(&config);
        assert!(matches!(
            result,
            Err(AssemblyError::Config(ConfigError::PortOverlap {
                a: DeviceId::Uart,
                b: DeviceId::Pit,
                port: 0x40,
            }))
        ));
    }

    #[test]
    fn a_machine_with_no_pic_gets_a_null_controller() {
        // A minimal machine with a single UART and no interrupt hardware still
        // assembles: the loop is handed a controller that acknowledges nothing.
        let config = MachineConfig {
            system_bios_image: None,
            board: BoardModel::Bare,
            display_switch: assembly::DisplaySwitch::Auto,
            cpu: CpuConfig {
                fpu: assembly::FpuChip::None,
                chip: assembly::CpuChip::I80286,
                reset: ResetSpec::Entry { cs: 0, ip: 0 },
                segments: SegmentSeed::flat(0),
            },
            clock: ClockConfig::default(),
            memory: MemoryConfig {
                address_space_bytes: 0x1_0000,
                ram: vec![RamRegion::new(0, 0x1_0000)],
                mmio: Vec::new(),
            },
            devices: vec![DeviceConfig::new(
                DeviceId::Uart,
                DeviceKind::Uart(UartCfg {
                    base: 0x3F8,
                    mouse: false,
                }),
                vec![PortWindow::new(0x3F8, 0x3FF)],
            )],
            drives: Vec::new(),
            images: Vec::new(),
        };
        let assembled = assemble_synthetic(&config).expect("assembles");
        assert_eq!(assembled.device_index[&DeviceId::Uart], 0);
    }

    #[test]
    fn floppy_type_codes_match_the_standard_nibbles() {
        assert_eq!(floppy_cmos_type(&FloppyGeometry::HD_1_44), 4);
        assert_eq!(floppy_cmos_type(&FloppyGeometry::HD_1_2), 2);
        assert_eq!(floppy_cmos_type(&FloppyGeometry::DD_720K), 3);
        assert_eq!(floppy_cmos_type(&FloppyGeometry::DD_360K), 1);
    }

    /// The reference machine with two floppies attached: a 1.2 MB drive A: in slot
    /// 0 and a 720 KB drive B: in slot 1, each with a correctly sized blank image.
    fn two_floppy_config() -> MachineConfig {
        let mut config = reference_config();
        config.drives = vec![
            DriveConfig {
                media: None,
                kind: DriveKind::Floppy {
                    geometry: FloppyGeometry::HD_1_2,
                },
                image: ImageSource::Blank {
                    size: FloppyGeometry::HD_1_2.image_bytes(),
                },
                controller: DeviceId::Fdc,
                unit: 0,
                write: WriteMode::Volatile,
                expected_sha256: None,
            },
            DriveConfig {
                media: None,
                kind: DriveKind::Floppy {
                    geometry: FloppyGeometry::DD_720K,
                },
                image: ImageSource::Blank {
                    size: FloppyGeometry::DD_720K.image_bytes(),
                },
                controller: DeviceId::Fdc,
                unit: 1,
                write: WriteMode::Volatile,
                expected_sha256: None,
            },
        ];
        config
    }

    #[test]
    fn two_floppies_route_to_slots_and_derive_cmos_with_no_manual_edit() {
        let config = two_floppy_config();

        // (a) Both drives land in the controller's slots, each carrying the exact
        // geometry the config named — slot 0 is the 1.2 MB drive, slot 1 the 720 KB.
        let slots = build_fdc_drives(&config, DeviceId::Fdc).expect("drives mounted");
        assert_eq!(
            slots[0].as_ref().expect("slot 0 mounted").geometry,
            FloppyGeometry::HD_1_2.to_device()
        );
        assert_eq!(
            slots[1].as_ref().expect("slot 1 mounted").geometry,
            FloppyGeometry::DD_720K.to_device()
        );
        assert!(slots[2].is_none() && slots[3].is_none());

        // (b) The CMOS floppy nibbles and equipment byte fall out of the attached
        // drives alone — no caller ever edits CMOS. High nibble = slot 0 (1.2 MB →
        // code 2), low nibble = slot 1 (720 KB → code 3); equipment bit 0 set (a
        // floppy is present) and bits 6-7 = 01 (two drives installed).
        let cmos = derive_cmos_config(&config).unwrap();
        assert_eq!(cmos[0x10], 0x23);
        assert_eq!(cmos[0x14] & 0x01, 0x01);
        assert_eq!(cmos[0x14] >> 6, 0b01);

        // And the whole machine assembles from that config unchanged.
        assert!(assemble_synthetic(&config).is_ok());
    }

    #[test]
    fn write_modes_route_to_the_mounted_drive() {
        // Two drives on one controller: slot 0 writable (the default), slot 1
        // write-protected. The mode must reach the device drive it belongs to.
        let mut config = reference_config();
        config.drives = vec![
            DriveConfig {
                media: None,
                kind: DriveKind::Floppy {
                    geometry: FloppyGeometry::HD_1_44,
                },
                image: ImageSource::Blank {
                    size: FloppyGeometry::HD_1_44.image_bytes(),
                },
                controller: DeviceId::Fdc,
                unit: 0,
                write: WriteMode::Volatile,
                expected_sha256: None,
            },
            DriveConfig {
                media: None,
                kind: DriveKind::Floppy {
                    geometry: FloppyGeometry::HD_1_44,
                },
                image: ImageSource::Blank {
                    size: FloppyGeometry::HD_1_44.image_bytes(),
                },
                controller: DeviceId::Fdc,
                unit: 1,
                write: WriteMode::ReadOnly,
                expected_sha256: None,
            },
        ];

        let slots = build_fdc_drives(&config, DeviceId::Fdc).expect("drives mounted");
        assert!(
            !slots[0].as_ref().expect("slot 0 mounted").write_protected(),
            "the default drive is writable"
        );
        assert!(
            slots[1].as_ref().expect("slot 1 mounted").write_protected(),
            "the flagged drive is mounted write-protected"
        );
    }

    #[test]
    fn cmos_floppy_inventory_uses_only_the_primary_controller() {
        let mut config = two_floppy_config();
        let secondary = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Uart)
            .unwrap();
        secondary.kind = DeviceKind::Fdc(FdcCfg { drive_slots: 2 });
        secondary.ports.clear();
        config.drives[1].controller = DeviceId::Uart;
        config.drives[1].unit = 0;
        assert_eq!(config.validate_host(), Ok(()));
        let cmos = derive_cmos_config(&config).unwrap();
        assert_eq!(cmos[0x10], 0x20);
        assert_eq!(cmos[0x14], 0x21);
        config.drives.remove(0);
        let cmos = derive_cmos_config(&config).unwrap();
        assert_eq!(cmos[0x10], 0);
        assert_eq!(cmos[0x14], 0x20);
    }

    #[test]
    fn named_controllers_keep_their_media_separate() {
        let mut config = two_floppy_config();
        let uart = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Uart)
            .unwrap();
        uart.kind = DeviceKind::Fdc(FdcCfg { drive_slots: 2 });
        uart.ports.clear();
        config.drives[1].controller = DeviceId::Uart;
        config.drives[1].unit = 0;
        for (drive, byte) in config.drives.iter_mut().zip([0x12, 0x72]) {
            drive.image = ImageSource::Inline(vec![byte; drive.kind.image_bytes() as usize]);
        }
        assert_eq!(config.validate_host(), Ok(()));
        let mut assembled = assemble_synthetic(&config).unwrap();
        for (controller, geometry, byte) in [
            (DeviceId::Fdc, FloppyGeometry::HD_1_2, 0x12),
            (DeviceId::Uart, FloppyGeometry::DD_720K, 0x72),
        ] {
            let slots = build_fdc_drives(&config, controller).unwrap();
            assert_eq!(slots[0].as_ref().unwrap().geometry, geometry.to_device());
            assert!(slots[1..].iter().all(Option::is_none));
            let index = assembled.device_index[&controller];
            let fdc = assembled.machine.bus_mut().device_mut(index).unwrap();
            // Direct accesses let the bare rig exercise two controller roles
            // without asking the fixed-port FDC model to relocate its registers.
            fdc.port_write(0x3F2, 1, 0x1C).unwrap();
            let rate = if geometry == FloppyGeometry::DD_720K {
                2
            } else {
                0
            };
            fdc.port_write(0x3F7, 1, rate).unwrap();
            for value in [0x06, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF] {
                fdc.port_write(0x3F5, 1, value).unwrap();
            }
            let DmaOp::ToMemory(data) = fdc.take_dma_request().unwrap().op else {
                panic!("read must queue data");
            };
            assert_eq!(data, vec![byte; 512]);
        }
    }

    #[test]
    fn readonly_guest_write_reports_not_writable_without_mutating_media() {
        assert_eq!(WriteMode::default(), WriteMode::Volatile);
        let mut config = reference_config();
        config.drives[0].write = WriteMode::ReadOnly;
        let original = vec![0; FloppyGeometry::HD_1_44.image_bytes() as usize];
        let mut assembled = assemble_synthetic(&config).unwrap();
        let index = assembled.device_index[&DeviceId::Fdc];
        let fdc = assembled.machine.bus_mut().device_mut(index).unwrap();
        fdc.port_write(0x3F2, 1, 0x1C).unwrap();
        for byte in [0x05, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF] {
            fdc.port_write(0x3F5, 1, byte).unwrap();
        }
        assert!(fdc.take_dma_request().is_none());
        let result: Vec<_> = (0..7).map(|_| fdc.port_read(0x3F5, 1).unwrap()).collect();
        assert_eq!(result, [0x40, 0x02, 0, 0, 0, 1, 2]);
        for byte in [0x06, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF] {
            fdc.port_write(0x3F5, 1, byte).unwrap();
        }
        let DmaOp::ToMemory(data) = fdc.take_dma_request().unwrap().op else {
            panic!("read must queue data");
        };
        assert_eq!(data, original[..512]);
    }

    /// The reference machine carrying a single floppy of `geometry` backed by
    /// `image` in slot 0.
    fn single_floppy_config(geometry: FloppyGeometry, image: ImageSource) -> MachineConfig {
        let mut config = reference_config();
        config.drives = vec![DriveConfig {
            media: None,
            kind: DriveKind::Floppy { geometry },
            image,
            controller: DeviceId::Fdc,
            unit: 0,
            write: WriteMode::Volatile,
            expected_sha256: None,
        }];
        config
    }

    #[test]
    fn a_guest_reads_at_the_configured_geometry_and_faults_past_it() {
        const SECTOR: usize = 512;
        // µPD765 data/DOR ports; the guest drives these through the assembled bus.
        const DOR: u16 = 0x3F2;
        const DATA: u16 = 0x3F5;

        // A 360 KB drive spans 40 cylinders (0..=39). Seed a distinctive first
        // sector so the queued DMA bytes are recognisable.
        let mut image = vec![0u8; FloppyGeometry::DD_360K.image_bytes() as usize];
        for (i, b) in image.iter_mut().take(SECTOR).enumerate() {
            *b = (i % 251) as u8;
        }
        let config =
            single_floppy_config(FloppyGeometry::DD_360K, ImageSource::Inline(image.clone()));
        let mut assembled = assemble_synthetic(&config).expect("assembles");
        let idx = assembled.device_index[&DeviceId::Fdc];
        let bus = assembled.machine.bus_mut();
        let fdc = bus.device_mut(idx).expect("fdc is on the bus");

        // Bring the controller out of reset (not-reset, DMA, motor 0, drive 0) and
        // drain the four post-reset sense polls, exactly as a BIOS would.
        fdc.port_write(DOR, 1, 0x1C).unwrap();
        for _ in 0..4 {
            fdc.port_write(DATA, 1, 0x08).unwrap(); // SENSE INTERRUPT STATUS
            let _ = fdc.port_read(DATA, 1).unwrap();
            let _ = fdc.port_read(DATA, 1).unwrap();
        }

        fdc.port_write(0x3F7, 1, 2).unwrap();

        // READ DATA at C=0 H=0 R=1 — a valid sector on a 360 KB disk. The
        // controller reads the configured drive's sector 0 and queues it for DMA.
        let read = [0x06u8, 0x00, 0x00, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF];
        for &b in &read {
            fdc.port_write(DATA, 1, u32::from(b)).unwrap();
        }
        let request = fdc
            .take_dma_request()
            .expect("an in-geometry read queues a DMA transfer");
        let DmaOp::ToMemory(data) = request.op else {
            panic!("a floppy read queues a device -> memory transfer");
        };
        assert_eq!(
            &data[..],
            &image[..SECTOR],
            "the queued bytes are the configured drive's sector 0"
        );
        fdc.dma_complete(DmaOutcome::ToMemory {
            transferred: SECTOR,
            terminal_count: true,
        });
        let st0 = fdc.port_read(DATA, 1).unwrap() as u8;
        assert_eq!(st0 & 0xC0, 0, "normal termination for an in-geometry read");
        for _ in 0..6 {
            let _ = fdc.port_read(DATA, 1).unwrap(); // drain the rest of the result
        }

        // READ DATA at C=40 — one past a 360 KB disk's last cylinder, though it
        // would be valid on a 1.44 MB disk (80 cylinders). The controller honours
        // the CONFIGURED geometry: there is no data field, so it faults with no
        // transfer instead of reading a 1.44 MB-shaped sector.
        let past = [0x06u8, 0x00, 40, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF];
        for &b in &past {
            fdc.port_write(DATA, 1, u32::from(b)).unwrap();
        }
        assert!(
            fdc.take_dma_request().is_none(),
            "an out-of-geometry read queues no transfer"
        );
        let st0 = fdc.port_read(DATA, 1).unwrap() as u8;
        let st1 = fdc.port_read(DATA, 1).unwrap() as u8;
        assert_eq!(st0 & 0x40, 0x40, "abnormal termination (ST0 IC = 01)");
        assert_eq!(st1 & 0x04, 0x04, "ST1 ND: no data field past the geometry");
    }

    #[test]
    fn unreadable_floppy_file_is_an_error() {
        let mut config = reference_config();
        config.drives[0].image = ImageSource::File(std::path::PathBuf::new());
        assert!(
            matches!(assemble_synthetic(&config), Err(AssemblyError::Config(ConfigError::ImageRead { path, message })) if path.as_os_str().is_empty() && !message.is_empty())
        );
    }

    #[test]
    fn file_backed_floppy_geometry_is_checked() {
        let mut config = reference_config();
        config.drives[0].image =
            ImageSource::File(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").into());
        assert!(
            matches!(assemble_synthetic(&config), Err(AssemblyError::Config(ConfigError::ImageGeometryMismatch { drive_index: 0, expected: 1_474_560, actual, .. })) if actual == include_bytes!("../Cargo.toml").len() as u64)
        );
    }

    #[test]
    fn file_backed_load_image_bounds_are_checked() {
        let mut config = reference_config();
        config.images[0] = LoadImage {
            base: SPACE - 1,
            source: ImageSource::File(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").into()),
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        };
        assert!(
            matches!(assemble_synthetic(&config), Err(AssemblyError::Config(ConfigError::RegionOutOfSpace { base, end, space })) if base == SPACE - 1 && end == base + include_bytes!("../Cargo.toml").len() as u64 && space == SPACE),
            "file bounds must produce a config error"
        );
    }

    fn file_source() -> ImageSource {
        ImageSource::File(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").into())
    }

    #[test]
    fn file_backed_load_over_mmio_is_rejected() {
        let mut config = reference_config();
        config.images[0] = LoadImage {
            base: TEXT_BASE,
            source: file_source(),
            kind: ImageKind::Rom,
            provides: Vec::new(),
        };
        assert!(matches!(
            assemble_synthetic(&config),
            Err(AssemblyError::Config(ConfigError::ImageOverlapsMmio {
                image_index: 0,
                device: DeviceId::Vga,
                addr: TEXT_BASE
            }))
        ));
    }

    #[test]
    fn file_backed_load_over_another_image_is_rejected() {
        let mut config = reference_config();
        config.images.push(LoadImage {
            base: ROM_BASE + 1,
            source: file_source(),
            kind: ImageKind::Rom,
            provides: Vec::new(),
        });
        assert!(
            matches!(assemble_synthetic(&config), Err(AssemblyError::Config(ConfigError::ImageOverlap { a: 0, b: 1, addr })) if addr == ROM_BASE + 1)
        );
    }

    #[test]
    fn file_backed_ram_load_outside_ram_is_rejected() {
        let mut config = reference_config();
        config.images[0] = LoadImage {
            base: ROM_BASE,
            source: file_source(),
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        };
        assert!(matches!(
            assemble_synthetic(&config),
            Err(AssemblyError::Config(ConfigError::ImageOutsideRam {
                image_index: 0,
                addr: ROM_BASE
            }))
        ));
    }

    #[test]
    fn file_backed_ram_load_preserves_bytes_and_source_config() {
        let mut config = reference_config();
        config.images[0] = LoadImage {
            base: 0,
            source: file_source(),
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        };
        let assembled = assemble_synthetic(&config).expect("readable RAM load assembles");
        let expected = include_bytes!("../Cargo.toml");
        let mut actual = vec![0; expected.len()];
        assembled.machine.cpu().read_mem(0, &mut actual).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(config.images[0].source, file_source());
    }
    fn ide_config(base: u16) -> MachineConfig {
        let mut config = reference_config();
        let cfg = assembly::IdeCfg {
            model: assembly::AtaControllerModel::Ata,
            base,
            control: base + 0x206,
        };
        config.devices.push(
            DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(cfg), cfg.port_windows())
                .with_irq(cfg.irq()),
        );
        config
            .devices
            .iter_mut()
            .find(|device| device.id == DeviceId::Fdc)
            .unwrap()
            .ports = FdcCfg::port_windows(base == 0x1F0);
        let geometry = assembly::HardDiskGeometry {
            cylinders: 8,
            heads: 2,
            sectors_per_track: 4,
            bytes_per_sector: 512,
        };
        config.drives.push(DriveConfig {
            media: None,
            controller: DeviceId::Ide,
            unit: 0,
            kind: DriveKind::HardDisk { geometry },
            image: ImageSource::Blank {
                size: geometry.image_bytes(),
            },
            write: WriteMode::Volatile,
            expected_sha256: None,
        });
        config
    }

    #[test]
    fn ide_primary_and_secondary_decode_word_data_byte_registers_and_irq() {
        for (base, irq) in [(0x1F0, 14), (0x170, 15)] {
            let config = ide_config(base);
            assert_eq!(config.validate_host(), Ok(()));
            let mut assembled = assemble(&config).unwrap();
            let map = build_decode_map(&config, &assembled.device_index);
            for offset in 0..8 {
                let window = map
                    .ports
                    .iter()
                    .find(|window| window.ports.contains(&(base + offset)))
                    .unwrap();
                assert_eq!(window.device, assembled.device_index[&DeviceId::Ide]);
                assert_eq!(
                    window.width,
                    if offset == 0 {
                        bus::PortWidth::Word
                    } else {
                        bus::PortWidth::Byte
                    }
                );
            }
            assert!(build_irq_lines(&config, &assembled.device_index)
                .iter()
                .any(
                    |line| line.device == assembled.device_index[&DeviceId::Ide] && line.irq == irq
                ));
            for (port, owner) in [(base + 0x206, DeviceId::Ide), (0x3F7, DeviceId::Fdc)] {
                assert_bus_port_read_owner(&mut assembled, port, Some(owner));
            }
            if base == 0x170 {
                assert_bus_port_read_owner(&mut assembled, 0x3F6, Some(DeviceId::Fdc));
            }
            let bus = assembled.machine.bus_mut();
            bus.port_write(base + 7, 1, 0xEC).unwrap();
            assert_eq!(bus.port_read(base, 2).unwrap(), 0x40);
        }
    }

    #[test]
    fn ide_firmware_board_requires_fixed_disk_even_without_drives() {
        let mut config = assembly::bios(vec![0xF4; 0x8000], Some(vec![0; 1_474_560]));
        let cfg = assembly::IdeCfg {
            model: assembly::AtaControllerModel::Ata,
            base: 0x1F0,
            control: 0x3F6,
        };
        config.devices.push(
            DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(cfg), cfg.port_windows()).with_irq(14),
        );
        config
            .devices
            .iter_mut()
            .find(|device| device.id == DeviceId::Fdc)
            .unwrap()
            .ports = FdcCfg::port_windows(true);
        let error = config.validate_host().unwrap_err();
        assert_eq!(
            error,
            ConfigError::NoProvider {
                device: DeviceId::Ide,
                service: assembly::FirmwareService::FixedDisk
            }
        );
        assert_eq!(
            error.to_string(),
            "device Ide needs firmware service fixed-disk; no ROM provides it"
        );
        config.images[0]
            .provides
            .push(assembly::FirmwareService::FixedDisk);
        let assembled = assemble_synthetic(&config).unwrap();
        assert_eq!(assembled.unlisted_firmware.len(), 1);
        assert!(assembled.device_index.contains_key(&DeviceId::Ide));
    }

    #[test]
    fn ide_resource_validation_has_an_exact_diagnostic() {
        for change in 0..5 {
            let mut config = ide_config(0x1F0);
            let device = config
                .devices
                .iter_mut()
                .find(|device| device.id == DeviceId::Ide)
                .unwrap();
            match change {
                0 => {
                    device.kind = DeviceKind::Ide(assembly::IdeCfg {
                        model: assembly::AtaControllerModel::Ata,
                        base: 0x1E0,
                        control: 0x3E6,
                    })
                }
                1 => {
                    device.kind = DeviceKind::Ide(assembly::IdeCfg {
                        model: assembly::AtaControllerModel::Ata,
                        base: 0x1F0,
                        control: 0x376,
                    })
                }
                2 => device.irq = Some(15),
                3 => device.irq = None,
                _ => {
                    device.dma = Some(DmaSpec {
                        controller: DeviceId::Dma,
                        channel: 2,
                    })
                }
            }
            assert_eq!(config.validate_host().unwrap_err().to_string(), "device Ide has invalid IDE resources: base must be 0x1f0 or 0x170, control must be base + 0x206, IRQ must be 14 or 15 respectively, and DMA must be absent");
        }
    }

    #[test]
    fn ide_drive_validation_has_exact_diagnostics() {
        for geometry in [
            assembly::HardDiskGeometry {
                cylinders: 0,
                heads: 2,
                sectors_per_track: 4,
                bytes_per_sector: 512,
            },
            assembly::HardDiskGeometry {
                cylinders: 16384,
                heads: 2,
                sectors_per_track: 4,
                bytes_per_sector: 512,
            },
            assembly::HardDiskGeometry {
                cylinders: 8,
                heads: 0,
                sectors_per_track: 4,
                bytes_per_sector: 512,
            },
            assembly::HardDiskGeometry {
                cylinders: 8,
                heads: 17,
                sectors_per_track: 4,
                bytes_per_sector: 512,
            },
            assembly::HardDiskGeometry {
                cylinders: 8,
                heads: 2,
                sectors_per_track: 0,
                bytes_per_sector: 512,
            },
            assembly::HardDiskGeometry {
                cylinders: 8,
                heads: 2,
                sectors_per_track: 64,
                bytes_per_sector: 512,
            },
            assembly::HardDiskGeometry {
                cylinders: 8,
                heads: 2,
                sectors_per_track: 4,
                bytes_per_sector: 256,
            },
        ] {
            let mut config = ide_config(0x1F0);
            config.drives[1].kind = DriveKind::HardDisk { geometry };
            assert_eq!(config.validate_host().unwrap_err().to_string(), "drive 1 has invalid ATA geometry: expected cylinders 1..=16383, heads 1..=16, sectors 1..=63 and 512-byte sectors");
        }
        let mut config = ide_config(0x1F0);
        config.drives[1].image = ImageSource::Blank { size: 512 };
        assert_eq!(
            config.validate_host().unwrap_err().to_string(),
            "drive 1 image is 512 bytes but its geometry needs 32768"
        );
        let mut config = ide_config(0x1F0);
        config.drives.push(config.drives[1].clone());
        assert_eq!(
            config.validate_host().unwrap_err().to_string(),
            "drives 1 and 2 bind to controller Ide unit 0"
        );
        config.drives.pop();
        config.drives[1].unit = 2;
        assert_eq!(
            config.validate_host().unwrap_err().to_string(),
            "drive 1 binds to controller Ide unit 2 but only 2 unit(s) exist"
        );
        config.drives[1].unit = 1;
        assert_eq!(
            config.validate_host().unwrap_err().to_string(),
            "drive 1 on Ide slave requires a master drive"
        );
    }
    #[test]
    fn ide_decode_must_equal_the_configured_windows() {
        for malformed in 0..3 {
            let mut config = ide_config(0x1F0);
            let device = config
                .devices
                .iter_mut()
                .find(|device| device.id == DeviceId::Ide)
                .unwrap();
            match malformed {
                0 => device.ports[0].width = bus::PortWidth::Byte,
                1 => {
                    device.ports.pop();
                }
                _ => {
                    device.ports = assembly::IdeCfg {
                        model: assembly::AtaControllerModel::Ata,
                        base: 0x170,
                        control: 0x376,
                    }
                    .port_windows()
                }
            }
            let error = config.validate_host().unwrap_err();
            assert_eq!(error.to_string(), "device Ide IDE port windows must match IdeCfg::port_windows(): word data at base, byte task registers at base + 1..=7, and byte control at base + 0x206");
            assert!(matches!(
                assemble(&config),
                Err(AssemblyError::Config(ConfigError::InvalidIdePortWindows {
                    device: DeviceId::Ide
                }))
            ));
        }
    }

    fn cmos_disk_catalogue() -> [assembly::FirmwareEntry; 1] {
        [assembly::FirmwareEntry {
            sha256: "c35020473aed1b4642cd726cad727b63fff2824ad68cedd7ffb73c7cbd890479",
            provides: &[
                assembly::FirmwareService::Floppy,
                assembly::FirmwareService::FixedDisk,
            ],
            capabilities: &[assembly::FirmwareCapability::CmosDriveTypes],
            ..assembly::KISELEV_XI8088
        }]
    }

    fn cmos_disk_config(cylinders: u32, heads: u32, unit: u8) -> MachineConfig {
        let mut config = assembly::bios(vec![0; 32768], Some(vec![0; 1_474_560]));
        config.drives.push(DriveConfig {
            media: None,
            kind: DriveKind::HardDisk {
                geometry: assembly::HardDiskGeometry {
                    cylinders,
                    heads,
                    sectors_per_track: 17,
                    bytes_per_sector: 512,
                },
            },
            image: ImageSource::Blank {
                size: u64::from(cylinders) * u64::from(heads) * 17 * 512,
            },
            expected_sha256: None,
            controller: DeviceId::Ide,
            unit,
            write: WriteMode::Volatile,
        });
        config
    }

    fn assert_cmos_checksum(seed: &[u8; 64]) {
        let expected: u16 = seed[0x10..=0x2D].iter().map(|&byte| u16::from(byte)).sum();
        assert_eq!(u16::from_be_bytes([seed[0x2E], seed[0x2F]]), expected);
    }

    fn qemu_cmos_catalogue() -> [assembly::FirmwareEntry; 1] {
        [assembly::FirmwareEntry {
            id: "synthetic-qemu-cmos",
            capabilities: &[assembly::FirmwareCapability::CmosQemuLayout],
            ..cmos_disk_catalogue()[0]
        }]
    }

    fn qemu_cmos_config() -> MachineConfig {
        let mut config = cmos_disk_config(615, 4, 0);
        config.drives.truncate(1);
        config.images[0].provides.clear();
        config
    }

    fn expected_at_seed() -> [u8; 128] {
        let mut seed = [0; 128];
        seed[0x0B] = 0x02;
        seed[0x0D] = 0x80;
        seed[0x10] = 0x40;
        seed[0x14] = 0x31;
        seed[0x15..=0x16].copy_from_slice(&[0x80, 0x02]);
        seed[0x2E..=0x2F].copy_from_slice(&[0x00, 0xF3]);
        // Every derived AT layout seeds the BCD start-year century (20h for 2000).
        seed[0x32] = 0x20;
        seed
    }

    fn assert_derived_cmos_century(year: u16, century: u8) {
        let mut config = qemu_cmos_config();
        config.images[0]
            .provides
            .push(assembly::FirmwareService::Floppy);
        rtc_config(&mut config).start.year = year;
        let mut expected = expected_at_seed();
        expected[0x32] = century;

        let mut qemu_expected = expected;
        qemu_expected[0x3D] = 0x21;
        assert_eq!(
            derive_cmos_layout_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap(),
            qemu_expected,
            "QEMU retains every previously seeded byte"
        );

        let mut image_config = config.clone();
        let mut image = [0; 64];
        image[0x32] = 0x18;
        rtc_config(&mut image_config).nvram = assembly::NvramSource::Image(image);
        assert_eq!(rtc_registers(&image_config)[0x32], 0x18);

        let seed = derive_cmos_config(&config).unwrap();
        assert_eq!(&seed[0x2E..=0x2F], &[0x00, 0xF3]);
        assert_cmos_checksum(&seed);
        assert_eq!(derive_cmos_layout(&config).unwrap(), expected);
        assert_eq!(seed, expected[..64]);
        let mut bus = testkit::MockBus::new(Box::new(CmosRtc::new().with_nvram(seed)));
        bus.port_write(0x70, 1, 0x32).unwrap();
        assert_eq!(bus.port_read(0x71, 1).unwrap(), u32::from(century));
        assert_eq!(rtc_registers(&config)[0x32], century);
    }

    #[test]
    fn derived_cmos_century_1999_preserves_checksum_qemu_and_image() {
        assert_derived_cmos_century(1999, 0x19);
    }

    #[test]
    fn derived_cmos_century_2000_preserves_checksum_qemu_and_image() {
        assert_derived_cmos_century(2000, 0x20);
    }

    #[test]
    fn derived_cmos_century_2026_preserves_checksum_qemu_and_image() {
        assert_derived_cmos_century(2026, 0x20);
    }

    /// IBM PC AT Technical Reference, System Board, CMOS RAM address map:
    /// 32h is BIOS-owned RAM, separate from the RTC's two-digit year.
    #[test]
    fn derived_cmos_century_guest_read_stays_seeded_after_year_rollover() {
        let mut config = assembly::at_rtc(
            vec![0xB0, 0x32, 0xE6, 0x70, 0xE4, 0x71, 0xFA, 0xF4],
            100,
            10,
        );
        let rtc = rtc_config(&mut config);
        rtc.nvram = assembly::NvramSource::Derived;
        rtc.start = assembly::RtcStart {
            year: 1999,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
        };
        let mut assembled = assemble_synthetic(&config).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        assert_eq!(assembled.machine.cpu().reg(cpu::CpuReg::Rax) & 0xFF, 0x19);
        let bus = assembled.machine.bus_mut();
        bus.tick_due(device::VClock(100));
        for (index, expected) in [(9, 0x00), (0x32, 0x19)] {
            bus.port_write(0x70, 1, index).unwrap();
            assert_eq!(bus.port_read(0x71, 1).unwrap(), expected);
        }
    }

    #[test]
    fn qemu_cmos_memory_seeds_are_byte_exact_through_four_gib_and_field_limits() {
        for (top, ext, above_16m, high_bytes, high, checksum) in [
            (1 << 20, [0, 0], [0, 0], 0, [0, 0, 0], [0, 0xF3]),
            (15 << 20, [0, 0x38], [0, 0], 0, [0, 0, 0], [1, 0x2B]),
            (16 << 20, [0, 0x3C], [0, 0], 0, [0, 0, 0], [1, 0x2F]),
            (64 << 20, [0, 0xFC], [0, 3], 0, [0, 0, 0], [1, 0xEF]),
            (65 << 20, [0xFF, 0xFF], [0x10, 3], 0, [0, 0, 0], [2, 0xF1]),
            (
                1 << 32,
                [0xFF, 0xFF],
                [0, 0xFF],
                65536,
                [1, 0, 0],
                [2, 0xF1],
            ),
            (
                1 << 32,
                [0xFF, 0xFF],
                [0, 0xFF],
                0xFF_FFFFu64 << 16,
                [0xFF, 0xFF, 0xFF],
                [2, 0xF1],
            ),
            (
                1 << 32,
                [0xFF, 0xFF],
                [0, 0xFF],
                0x100_0000u64 << 16,
                [0, 0, 0],
                [2, 0xF1],
            ),
        ] {
            let mut config = qemu_cmos_config();
            config
                .memory
                .ram
                .retain(|region| region.base < EXTENDED_BASE);
            if top > EXTENDED_BASE {
                config
                    .memory
                    .ram
                    .push(RamRegion::new(EXTENDED_BASE, top - EXTENDED_BASE));
            }
            if high_bytes > 0 {
                config.memory.ram.push(RamRegion::new(1 << 32, high_bytes));
            }
            config.memory.address_space_bytes = top.max((1 << 32) + high_bytes);
            let mut expected = expected_at_seed();
            expected[0x17..=0x18].copy_from_slice(&ext);
            expected[0x30..=0x31].copy_from_slice(&ext);
            expected[0x34..=0x35].copy_from_slice(&above_16m);
            expected[0x5B..=0x5D].copy_from_slice(&high);
            expected[0x2E..=0x2F].copy_from_slice(&checksum);
            expected[0x32] = 0x20;
            expected[0x3D] = 0x21;
            let actual =
                derive_cmos_layout_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap();
            assert_eq!(actual, expected, "top={top}, high={high_bytes}");
            assert_eq!(
                derive_cmos_config_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap(),
                expected[..64]
            );
        }
    }

    #[test]
    fn qemu_cmos_memory_counts_windows_and_whole_units_only() {
        let mut config = qemu_cmos_config();
        config.memory.ram = vec![
            RamRegion::new(0, 640 << 10),
            RamRegion::new(EXTENDED_BASE, 1023),
            RamRegion::new((16 << 20) - 1024, 65536 + 1024 + 7),
            RamRegion::new((1 << 32) - 65536, 2 * 65536 + 9),
        ];
        let seed = derive_cmos_layout_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap();
        assert_eq!(&seed[0x30..=0x31], &[130, 0]);
        assert_eq!(&seed[0x34..=0x35], &[2, 0]);
        assert_eq!(&seed[0x5B..=0x5D], &[1, 0, 0]);
    }

    #[test]
    fn qemu_cmos_century_uses_rtc_start_and_default_when_rtc_is_absent() {
        for (year, expected) in [(1980, 0x19), (1999, 0x19), (2000, 0x20), (2099, 0x20)] {
            let mut config = qemu_cmos_config();
            for device in &mut config.devices {
                if let DeviceKind::Rtc(ref mut rtc) = device.kind {
                    rtc.start.year = year;
                }
            }
            let seed = derive_cmos_config_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap();
            assert_eq!(seed[0x32], expected);
            let mut rtc = CmosRtc::new().with_nvram(seed);
            rtc.port_write(0x70, 1, 0x32).unwrap();
            assert_eq!(rtc.port_read(0x71, 1).unwrap(), u32::from(expected));
        }
        let mut config = qemu_cmos_config();
        config
            .devices
            .retain(|device| !matches!(device.kind, DeviceKind::Rtc(_)));
        assert_eq!(
            derive_cmos_layout_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap()[0x32],
            0x20
        );
    }

    #[test]
    fn qemu_cmos_boot_signature_check_and_single_cpu_are_explicit() {
        let mut seed = [0xA5; 128];
        seed_qemu_cmos(&qemu_cmos_config(), &mut seed);
        assert_eq!([seed[0x38], seed[0x3D], seed[0x5F]], [0, 0x21, 0]);
    }

    #[test]
    fn qemu_cmos_type_47_geometry_and_translation_follow_all_four_positions() {
        for (base, unit, large, translation, disk_type) in [
            (0x1F0, 0, false, 0, 0xF0),
            (0x1F0, 1, false, 0, 0x0F),
            (0x170, 0, false, 0, 0),
            (0x170, 1, false, 0, 0),
            (0x1F0, 0, true, 1, 0xF0),
            (0x1F0, 1, true, 4, 0x0F),
            (0x170, 0, true, 16, 0),
            (0x170, 1, true, 64, 0),
        ] {
            let mut config = qemu_cmos_config();
            let ide = assembly::IdeCfg {
                model: assembly::AtaControllerModel::Ata,
                base,
                control: base + 0x206,
            };
            config.devices.push(
                DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(ide), ide.port_windows())
                    .with_irq(ide.irq()),
            );
            let geometry = assembly::HardDiskGeometry {
                cylinders: if large { 1025 } else { 1024 },
                heads: if large { 16 } else { 8 },
                sectors_per_track: 63,
                bytes_per_sector: 512,
            };
            config.drives.push(DriveConfig {
                media: None,
                controller: DeviceId::Ide,
                unit,
                kind: DriveKind::HardDisk { geometry },
                image: ImageSource::Blank {
                    size: geometry.image_bytes(),
                },
                write: WriteMode::Volatile,
                expected_sha256: None,
            });
            let seed = derive_cmos_layout_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap();
            let mut expected = expected_at_seed();
            expected[0x32] = 0x20;
            expected[0x3D] = 0x21;
            expected[0x39] = translation;
            expected[0x12] = disk_type;
            if base == 0x1F0 {
                expected[0x19 + usize::from(unit)] = 47;
                let offset = 0x1B + usize::from(unit) * 9;
                expected[offset..offset + 9].copy_from_slice(if large {
                    &[1, 4, 16, 0xFF, 0xFF, 0xC8, 1, 4, 63]
                } else {
                    &[0, 4, 8, 0xFF, 0xFF, 0xC0, 0, 4, 63]
                });
                let checksum: u16 = if large { 0x0441 } else { 0x042F };
                let checksum = checksum + u16::from(disk_type);
                expected[0x2E..=0x2F].copy_from_slice(&checksum.to_be_bytes());
            }
            assert_eq!(seed, expected, "base={base:x}, unit={unit}, large={large}");
        }
    }

    #[test]
    fn qemu_cmos_translation_combines_mixed_primary_drives_in_any_order() {
        for (master_cylinders, slave_cylinders, expected) in
            [(1024, 1025, 0x04), (1025, 1024, 0x01)]
        {
            let mut config = qemu_cmos_config();
            let ide = assembly::IdeCfg {
                model: assembly::AtaControllerModel::Ata,
                base: 0x1F0,
                control: 0x3F6,
            };
            config.devices.push(
                DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(ide), ide.port_windows())
                    .with_irq(14),
            );
            for (unit, cylinders) in [(0, master_cylinders), (1, slave_cylinders)] {
                let geometry = assembly::HardDiskGeometry {
                    cylinders,
                    heads: 16,
                    sectors_per_track: 63,
                    bytes_per_sector: 512,
                };
                config.drives.push(DriveConfig {
                    media: None,
                    controller: DeviceId::Ide,
                    unit,
                    kind: DriveKind::HardDisk { geometry },
                    image: ImageSource::Blank {
                        size: geometry.image_bytes(),
                    },
                    write: WriteMode::Volatile,
                    expected_sha256: None,
                });
            }
            for _ in 0..2 {
                let seed =
                    derive_cmos_layout_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap();
                assert_eq!(seed[0x39], expected);
                config.drives.reverse();
            }
        }
    }

    #[test]
    fn rtc128_qemu_catalogue_validates_and_guest_reads_the_full_derived_seed() {
        let entry = assembly::FirmwareEntry {
            boards: &["bare"],
            ..qemu_cmos_catalogue()[0]
        };
        for high_units in [0, 0x03_0201u64] {
            let mut config = qemu_cmos_config();
            config.board = BoardModel::Bare;
            rtc_config(&mut config).ram = assembly::CmosRam::Bytes128;
            config.cpu.reset = ResetSpec::Entry { cs: 0, ip: 0x600 };
            config.cpu.segments = SegmentSeed::flat(0);
            if high_units != 0 {
                config.memory.address_space_bytes = (1 << 32) + (high_units << 16);
                config
                    .memory
                    .ram
                    .push(RamRegion::new(1 << 32, high_units << 16));
            }
            let mut guest = vec![0xFA];
            for index in 0x1B..=0x1F {
                // Poison the aliases: an extended read must never see these bytes.
                guest.extend([0xB0, index, 0xE6, 0x70, 0xB0, 0xA5, 0xE6, 0x71]);
            }
            for (offset, index) in (0x5B..=0x5F)
                .chain((0x5B..=0x5F).map(|index| index | 0x80))
                .chain(0x1B..=0x1F)
                .enumerate()
            {
                let address = (0x500 + offset as u16).to_le_bytes();
                guest.extend([
                    0xB0, index, 0xE6, 0x70, 0xE4, 0x71, 0xA2, address[0], address[1],
                ]);
            }
            guest.push(0xF4);
            config.images.push(LoadImage {
                base: 0x600,
                kind: ImageKind::RamLoad,
                source: ImageSource::Inline(guest),
                provides: Vec::new(),
            });
            config.validate_host().unwrap();
            assembly::check_system_bios(
                &config,
                BoardModel::Bare,
                &[entry],
                config.images[0].base,
                &vec![0; 32768],
                assembly::AssemblyOptions::default(),
            )
            .unwrap();
            let mut assembled = assemble_with_cpu_and_catalogue(
                &config,
                &ObservationPolicy::default(),
                assembly::AssemblyOptions::default(),
                |config| {
                    // The synthetic high-memory map exercises CMOS derivation only.
                    // The 286 guest executes in low RAM; no high RAM is allocated.
                    let mut executable = config.clone();
                    executable.memory.address_space_bytes = SPACE;
                    executable.memory.ram.retain(|region| region.base < SPACE);
                    build_cpu(&executable)
                },
                &[entry],
            )
            .unwrap();
            assert_eq!(
                assembled.machine.run_bounded(Some(200)).unwrap(),
                machine::RunOutcome::Halted
            );
            let mut actual = [0; 15];
            assembled
                .machine
                .cpu()
                .read_mem(0x500, &mut actual)
                .unwrap();
            let high = high_units.to_le_bytes();
            assert_eq!(&actual[..5], &[high[0], high[1], high[2], 0, 0]);
            assert_eq!(&actual[5..10], &[high[0], high[1], high[2], 0, 0]);
            assert_eq!(&actual[10..], &[0xA5; 5]);

            rtc_config(&mut config).ram = assembly::CmosRam::Bytes64;
            let error = assemble_with_cpu_and_catalogue(
                &config,
                &ObservationPolicy::default(),
                assembly::AssemblyOptions::default(),
                |_| panic!("requirements must precede CPU construction"),
                &[entry],
            )
            .err()
            .unwrap();
            assert_eq!(error.to_string(), "invalid configuration: firmware \"synthetic-qemu-cmos\" requires rtc-cmos-128; none is configured");
        }
    }

    #[test]
    fn rtc128_is_rejected_on_ibm5170_before_hardware_construction() {
        let mut config = assembly::bios(vec![0; 32768], Some(vec![0; 1_474_560]));
        assert_eq!(rtc_config(&mut config).ram, assembly::CmosRam::Bytes64);
        rtc_config(&mut config).ram = assembly::CmosRam::Bytes128;
        assert_eq!(
            config.validate_host().unwrap_err(),
            ConfigError::Ibm5170RtcRam
        );
        assert_eq!(
            config.validate_host().unwrap_err().to_string(),
            "board ibm-5170 requires a 64-byte RTC: the IBM AT has an MC146818"
        );
        assert!(matches!(
            assemble(&config),
            Err(AssemblyError::Config(ConfigError::Ibm5170RtcRam))
        ));
    }

    #[test]
    fn rtc128_rejects_nvram_images_and_journals_before_opening_files() {
        for nvram in [
            assembly::NvramSource::Image([0; 64]),
            assembly::NvramSource::Journal {
                image: "not-opened-cmos.bin".into(),
                expected_sha256: [0; 32],
                journal: "not-created-cmos.upfj".into(),
                keep_generations: 1,
            },
        ] {
            let mut config = assembly::bios(vec![0; 32768], None);
            let rtc = rtc_config(&mut config);
            rtc.ram = assembly::CmosRam::Bytes128;
            rtc.nvram = nvram;
            assert_eq!(
                config.validate_host().unwrap_err(),
                ConfigError::UnsupportedExtendedNvram
            );
            assert_eq!(
                config.validate_host().unwrap_err().to_string(),
                "128-byte NVRAM images are not supported yet"
            );
            assert!(matches!(
                assemble(&config),
                Err(AssemblyError::Config(ConfigError::UnsupportedExtendedNvram))
            ));
        }
        for nvram in [assembly::NvramSource::Blank, assembly::NvramSource::Derived] {
            let mut config = assembly::bios(vec![0; 32768], None);
            let rtc = rtc_config(&mut config);
            rtc.ram = assembly::CmosRam::Bytes128;
            rtc.nvram = nvram;
            config.validate_host().unwrap();
            let _assembled = assemble(&config).unwrap();
        }
    }

    #[test]
    fn qemu_cmos_capability_requires_distinct_extended_cmos_addresses() {
        let mut config = qemu_cmos_config();
        let ide = assembly::IdeCfg {
            model: assembly::AtaControllerModel::Ata,
            base: 0x1F0,
            control: 0x3F6,
        };
        config.devices.push(
            DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(ide), ide.port_windows()).with_irq(14),
        );
        let mut disk = cmos_disk_config(1024, 16, 0).drives.remove(1);
        let DriveKind::HardDisk { ref mut geometry } = disk.kind else {
            unreachable!()
        };
        geometry.sectors_per_track = 63;
        disk.image = ImageSource::Blank {
            size: disk.kind.image_bytes(),
        };
        config.drives.push(disk);
        let message = "firmware \"synthetic-qemu-cmos\" requires rtc-cmos-128; none is configured";
        for declared in [false, true] {
            let entry = assembly::FirmwareEntry {
                requires: if declared {
                    &[assembly::FirmwarePart::RtcCmos128]
                } else {
                    &[]
                },
                ..qemu_cmos_catalogue()[0]
            };
            assert_eq!(
                assembly::test_support::check_entry_requirements(
                    &config,
                    config.board,
                    &entry,
                    None
                )
                .unwrap_err()
                .to_string(),
                message
            );
            assert_eq!(
                assembly::check_system_bios(
                    &config,
                    config.board,
                    &[entry],
                    config.images[0].base,
                    &vec![0; 32768],
                    assembly::AssemblyOptions {
                        allow_unlisted_firmware: true
                    }
                )
                .unwrap_err()
                .to_string(),
                message
            );
            let seed = derive_cmos_layout_with_catalogue(&config, &[entry]).unwrap();
            assert_eq!(&seed[0x1B..=0x1D], &[0, 4, 16]);
            assert_eq!(seed[0x1F], 0xFF);
            assert_eq!(&seed[0x5B..=0x5D], &[0, 0, 0]);
            assert_eq!(seed[0x5F], 0);
        }
        config.devices.retain(|device| device.id != DeviceId::Rtc);
        assert_eq!(
            assembly::test_support::check_entry_requirements(
                &config,
                config.board,
                &assembly::FirmwareEntry {
                    requires: &[],
                    ..qemu_cmos_catalogue()[0]
                },
                None
            )
            .unwrap_err()
            .to_string(),
            message
        );
    }

    #[test]
    fn qemu_cmos_type_47_is_independent_of_option_rom_disk_services() {
        let mut config = qemu_cmos_config();
        let ide = assembly::IdeCfg {
            model: assembly::AtaControllerModel::Ata,
            base: 0x1F0,
            control: 0x3F6,
        };
        config.devices.push(DeviceConfig::new(
            DeviceId::Ide,
            DeviceKind::Ide(ide),
            ide.port_windows(),
        ));
        config
            .drives
            .extend(cmos_disk_config(615, 4, 0).drives.into_iter().skip(1));
        let mut slave = config.drives[1].clone();
        slave.unit = 1;
        config.drives.push(slave);
        let expected = derive_cmos_layout_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap();
        assert_eq!(
            [expected[0x12], expected[0x19], expected[0x1A]],
            [0xFF, 47, 47]
        );
        config.images.push(assembly::LoadImage {
            base: 0xC8000,
            kind: ImageKind::Rom,
            source: ImageSource::Inline(vec![0; 2048]),
            provides: vec![assembly::FirmwareService::FixedDisk],
        });
        assert_eq!(
            derive_cmos_layout_with_catalogue(&config, &qemu_cmos_catalogue()).unwrap(),
            expected
        );
    }

    #[test]
    fn qemu_cmos_layout_requires_a_hash_matched_system_capability() {
        let config = qemu_cmos_config();
        for entry in [
            assembly::FirmwareEntry {
                capabilities: &[],
                ..qemu_cmos_catalogue()[0]
            },
            assembly::FirmwareEntry {
                sha256: assembly::KISELEV_XI8088.sha256,
                ..qemu_cmos_catalogue()[0]
            },
            assembly::FirmwareEntry {
                kind: assembly::FirmwareKind::OptionRom,
                ..qemu_cmos_catalogue()[0]
            },
        ] {
            assert_eq!(
                derive_cmos_layout_with_catalogue(&config, &[entry]).unwrap(),
                expected_at_seed()
            );
        }
        assert_eq!(derive_cmos_layout(&config).unwrap(), expected_at_seed());
        assert_eq!(
            derive_cmos_config(&config).unwrap(),
            expected_at_seed()[..64]
        );
        assert_eq!(
            assembly::FirmwareCapability::CmosQemuLayout.to_string(),
            "cmos-qemu-layout"
        );
        assert!(config.board.firmware_catalogue().iter().all(|entry| !entry
            .capabilities
            .contains(&assembly::FirmwareCapability::CmosQemuLayout)));
    }

    #[test]
    fn qemu_cmos_layout_rejects_ibm_type_capability_in_validation_and_derivation() {
        let config = qemu_cmos_config();
        let entry = assembly::FirmwareEntry {
            capabilities: &[
                assembly::FirmwareCapability::CmosDriveTypes,
                assembly::FirmwareCapability::CmosQemuLayout,
            ],
            ..qemu_cmos_catalogue()[0]
        };
        let message =
            "firmware synthetic-qemu-cmos cannot combine cmos-drive-types and cmos-qemu-layout";
        assert_eq!(
            assembly::test_support::check_entry_requirements(&config, config.board, &entry, None)
                .unwrap_err()
                .to_string(),
            message
        );
        assert_eq!(
            assembly::check_system_bios(
                &config,
                config.board,
                &[entry],
                config.images[0].base,
                &vec![0; 32768],
                assembly::AssemblyOptions::default(),
            )
            .unwrap_err()
            .to_string(),
            message
        );
        assert_eq!(
            derive_cmos_layout_with_catalogue(&config, &[entry])
                .unwrap_err()
                .to_string(),
            message
        );
        assembly::test_support::check_cmos_capabilities(&qemu_cmos_catalogue()[0]).unwrap();
    }

    #[test]
    fn ibm_bios_without_fixed_disk_and_with_xtide_obeys_provider_rules() {
        let bytes = vec![0; 65536];
        let ibm = assembly::FirmwareEntry {
            sha256: "de2f256064a0af797747c2b97505dc0b9f3df0de4f489eac731c23ae9ca9cc31",
            ..assembly::test_support::IBM_5170_1985_06_10
        };
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), ibm.sha256);
        let mut config = assembly::bios(bytes.clone(), Some(vec![0; 1_474_560]));
        config.images[0].base = 0xF0000;
        config.images[0].provides.clear();
        assembly::check_system_bios(
            &config,
            BoardModel::Ibm5170,
            &[ibm],
            0xF0000,
            &bytes,
            assembly::AssemblyOptions::default(),
        )
        .unwrap();
        assert!(!config.devices.iter().any(|d| d.id == DeviceId::Ide));
        let seed = derive_cmos_config_with_catalogue(&config, &[ibm]).unwrap();
        assert_eq!([seed[0x12], seed[0x19], seed[0x1A]], [0; 3]);
        let mut disk = cmos_disk_config(615, 4, 0).drives.remove(1);
        disk.image = ImageSource::Blank {
            size: disk.kind.image_bytes(),
        };
        config.drives.push(disk);
        let ide = assembly::IdeCfg {
            model: assembly::AtaControllerModel::Ata,
            base: 0x1F0,
            control: 0x3F6,
        };
        config.devices.push(
            DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(ide), ide.port_windows()).with_irq(14),
        );
        let seed = derive_cmos_config_with_catalogue(&config, &[ibm]).unwrap();
        assert_eq!(seed[0x12], 0x20);
        let mut option = vec![0; 8192];
        option[..3].copy_from_slice(&[0x55, 0xAA, 16]);
        option[8191] = 0xF1;
        let xtide = assembly::FirmwareEntry {
            sha256: Box::leak(format!("{:x}", Sha256::digest(&option)).into_boxed_str()),
            ..assembly::FirmwareEntry::XUB_R638_AT
        };
        config.images.push(assembly::LoadImage {
            base: 0xC8000,
            source: ImageSource::Inline(option),
            kind: ImageKind::Rom,
            provides: vec![],
        });
        let catalogue = [ibm, xtide];
        let seed = derive_cmos_config_with_catalogue(&config, &catalogue).unwrap();
        assert_eq!([seed[0x12], seed[0x19], seed[0x1A]], [0; 3]);
        assert_cmos_checksum(&seed);
        assembly::test_support::validate_option_roms_with_catalogue(&config, &catalogue).unwrap();
        config.devices.retain(|d| d.id != DeviceId::Ide);
        assert!(matches!(
            assembly::test_support::check_entry_requirements(
                &config,
                BoardModel::Ibm5170,
                &xtide,
                Some(1)
            ),
            Err(ConfigError::OptionRomCardMismatch { .. })
        ));
    }

    #[test]
    fn provider_cmos_standard_type_uses_drive_units_and_repairs_checksum() {
        let mut config = cmos_disk_config(615, 4, 0);
        for (unit, expected) in [(0, 0x20), (1, 0x02)] {
            config.drives[1].unit = unit;
            let seed = derive_cmos_config_with_catalogue(&config, &cmos_disk_catalogue()).unwrap();
            assert_eq!([seed[0x12], seed[0x19], seed[0x1A]], [expected, 0, 0]);
            assert_cmos_checksum(&seed);
        }
        let mut second = config.drives[1].clone();
        second.unit = 0;
        if let DriveKind::HardDisk { ref mut geometry } = second.kind {
            geometry.heads = 6;
        }
        config.drives.push(second);
        config.drives.reverse();
        let seed = derive_cmos_config_with_catalogue(&config, &cmos_disk_catalogue()).unwrap();
        assert_eq!(seed[0x12], 0x32);
        assert_cmos_checksum(&seed);
    }

    #[test]
    fn provider_cmos_extended_types_use_the_correct_extension_byte() {
        for (unit, nibble, extensions) in [(0, 0xF0, [16, 0]), (1, 0x0F, [0, 16])] {
            let config = cmos_disk_config(612, 4, unit);
            let seed = derive_cmos_config_with_catalogue(&config, &cmos_disk_catalogue()).unwrap();
            assert_eq!(seed[0x12], nibble);
            assert_eq!(&seed[0x19..=0x1A], &extensions);
            assert_cmos_checksum(&seed);
            let mut rtc = CmosRtc::new().with_nvram(seed);
            for index in [0x12, 0x19, 0x1A, 0x2E, 0x2F] {
                rtc.port_write(0x70, 1, index as u32).unwrap();
                assert_eq!(rtc.port_read(0x71, 1).unwrap() as u8, seed[index]);
            }
        }
    }

    #[test]
    fn provider_cmos_requires_a_matched_capable_system_disk_provider() {
        let config = cmos_disk_config(615, 4, 0);
        let baseline = cmos_disk_catalogue()[0];
        for entry in [
            assembly::FirmwareEntry {
                capabilities: &[],
                ..baseline
            },
            assembly::FirmwareEntry {
                provides: &[assembly::FirmwareService::Floppy],
                ..baseline
            },
            assembly::FirmwareEntry {
                sha256: assembly::KISELEV_XI8088.sha256,
                ..baseline
            },
            assembly::FirmwareEntry {
                kind: assembly::FirmwareKind::OptionRom,
                ..baseline
            },
        ] {
            let seed = derive_cmos_config_with_catalogue(&config, &[entry]).unwrap();
            assert_eq!([seed[0x12], seed[0x19], seed[0x1A]], [0; 3]);
            assert_cmos_checksum(&seed);
        }
    }

    #[test]
    fn provider_cmos_option_rom_overrides_even_a_capable_system_bios() {
        let mut config = cmos_disk_config(615, 4, 0);
        config.images.push(assembly::LoadImage {
            base: 0xC8000,
            source: ImageSource::Inline(vec![0; 2048]),
            kind: ImageKind::Rom,
            provides: vec![assembly::FirmwareService::FixedDisk],
        });
        config.images.reverse();
        let seed = derive_cmos_config_with_catalogue(&config, &cmos_disk_catalogue()).unwrap();
        assert_eq!([seed[0x12], seed[0x19], seed[0x1A]], [0; 3]);
        config.images[0].provides.clear();
        let option_entry = assembly::FirmwareEntry {
            kind: assembly::FirmwareKind::OptionRom,
            sha256: "e5a00aa9991ac8a5ee3109844d84a55583bd20572ad3ffcd42792f3c36b183ad",
            size: 2048,
            provides: &[assembly::FirmwareService::FixedDisk],
            capabilities: &[],
            ..cmos_disk_catalogue()[0]
        };
        let catalogue = [cmos_disk_catalogue()[0], option_entry];
        let seed = derive_cmos_config_with_catalogue(&config, &catalogue).unwrap();
        assert_eq!([seed[0x12], seed[0x19], seed[0x1A]], [0; 3]);
        config.images[0].kind = ImageKind::RamLoad;
        assert_eq!(
            derive_cmos_config_with_catalogue(&config, &catalogue).unwrap()[0x12],
            0x20
        );
    }

    #[test]
    fn provider_cmos_rejects_undocumented_geometry_instead_of_inventing_type_47() {
        for (cylinders, heads, sectors, bytes) in
            [(616, 4, 17, 512), (615, 4, 18, 512), (615, 4, 17, 1024)]
        {
            let mut config = cmos_disk_config(cylinders, heads, 0);
            let DriveKind::HardDisk { ref mut geometry } = config.drives[1].kind else {
                unreachable!()
            };
            geometry.sectors_per_track = sectors;
            geometry.bytes_per_sector = bytes;
            let expected = ConfigError::UnsupportedCmosDiskGeometry {
                drive_index: 1,
                geometry: *geometry,
            };
            assert_eq!(
                derive_cmos_config_with_catalogue(&config, &cmos_disk_catalogue()),
                Err(expected.clone())
            );
            assert_eq!(expected.to_string(), format!("drive 1 geometry {cylinders}/{heads}/{sectors} ({bytes}-byte sectors) has no IBM AT CMOS fixed-disk type; user-defined type 47 is not supported; use a fixed-disk option ROM"));
            assert!(
                derive_cmos_config(&config).is_ok(),
                "unlisted firmware does not opt into CMOS types"
            );
        }
    }

    #[test]
    fn provider_cmos_table_covers_documented_chs_and_chooses_lowest_duplicate() {
        for (cylinders, heads, expected) in [
            (306, 4, 1),
            (615, 4, 2),
            (615, 6, 3),
            (940, 8, 4),
            (940, 6, 5),
            (462, 8, 7),
            (733, 5, 8),
            (900, 15, 9),
            (820, 3, 10),
            (855, 5, 11),
            (855, 7, 12),
            (306, 8, 13),
            (733, 7, 14),
            (612, 4, 16),
            (977, 5, 17),
            (977, 7, 18),
            (1024, 7, 19),
        ] {
            let config = cmos_disk_config(cylinders, heads, 0);
            let seed = derive_cmos_config_with_catalogue(&config, &cmos_disk_catalogue()).unwrap();
            assert_eq!(seed[0x12] >> 4, expected.min(15));
            assert_eq!(seed[0x19], if expected > 15 { expected } else { 0 });
        }
    }

    #[test]
    fn ide_cmos_seed_retains_zero_fixed_disk_types() {
        let baseline = reference_config();
        let config = ide_config(0x1F0);
        assert_eq!(derive_cmos_config(&config), derive_cmos_config(&baseline));
        let mut assembled = assemble(&config).unwrap();
        for (register, expected) in [(0x12, 0), (0x19, 0), (0x1A, 0)] {
            let bus = assembled.machine.bus_mut();
            bus.port_write(0x70, 1, register).unwrap();
            assert_eq!(bus.port_read(0x71, 1).unwrap(), expected);
        }
    }

    #[test]
    fn ide_duplicate_channel_has_an_exact_explanation() {
        let mut config = ide_config(0x1F0);
        let cfg = assembly::IdeCfg {
            model: assembly::AtaControllerModel::Ata,
            base: 0x170,
            control: 0x376,
        };
        config.devices.push(
            DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(cfg), cfg.port_windows()).with_irq(15),
        );
        assert_eq!(config.validate_host().unwrap_err().to_string(), "only one IDE channel is supported: DeviceId::Ide identifies one role, not separate primary and secondary instances");
    }
}

#[cfg(test)]
mod base_length_tests {
    #[test]
    fn wrong_disk_geometry_is_rejected_before_hashing() {
        let directory = crate::test_support::TempDir::new("geometry-before-hash");
        let path = directory.path().join("base.img");
        std::fs::write(&path, [0; 4096]).unwrap();
        let before = journal_files::hash_passes();
        let kind = assembly::DriveKind::HardDisk {
            geometry: assembly::HardDiskGeometry {
                cylinders: 1,
                heads: 1,
                sectors_per_track: 16,
                bytes_per_sector: 512,
            },
        };
        let error = super::verify_drive_base(&path, None, &kind, 0, None).unwrap_err();
        assert!(matches!(
            error,
            assembly::ConfigError::ImageGeometryMismatch {
                actual: 4096,
                expected: 8192,
                ..
            }
        ));
        assert_eq!(journal_files::hash_passes(), before);
    }
}

pub(super) trait HostValidate {
    fn validate_host(&self) -> Result<(), ConfigError>;
}
impl HostValidate for MachineConfig {
    fn validate_host(&self) -> Result<(), ConfigError> {
        crate::validate(self)
    }
}
fn assemble_with_cpu(
    config: &MachineConfig,
    policy: &ObservationPolicy,
    options: AssemblyOptions,
    build: impl FnOnce(&MachineConfig) -> Result<X86JitCpu, AssemblyError>,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_cpu_and_catalogue(
        config,
        policy,
        options,
        build,
        config.board.firmware_catalogue(),
    )
}
fn assemble_with_cpu_and_catalogue(
    config: &MachineConfig,
    policy: &ObservationPolicy,
    options: AssemblyOptions,
    build: impl FnOnce(&MachineConfig) -> Result<X86JitCpu, AssemblyError>,
    catalogue: &[FirmwareEntry],
) -> Result<AssembledMachine, AssemblyError> {
    let mount = Rc::new(FileMount::new());
    assembly::test_support::assemble_preverified(
        config,
        policy,
        options,
        build,
        catalogue,
        mount.clone(),
        MachineMount::new(mount),
    )
}
fn resolve_file_sources(
    config: &MachineConfig,
    options: AssemblyOptions,
) -> Result<(std::borrow::Cow<'_, MachineConfig>, Vec<Option<DriveBase>>), ConfigError> {
    resolve_file_sources_with_catalogue(config, options, config.board.firmware_catalogue())
}
fn resolve_file_sources_with_catalogue<'a>(
    config: &'a MachineConfig,
    options: AssemblyOptions,
    catalogue: &[FirmwareEntry],
) -> Result<(std::borrow::Cow<'a, MachineConfig>, Vec<Option<DriveBase>>), ConfigError> {
    resolve_sources_preverified(config, options, catalogue, &FileMount::new())
}
/// [`verify_mounted_base`] over host files, with an optional base the caller
/// verified already.
fn verify_drive_base(
    path: &std::path::Path,
    expected: Option<[u8; 32]>,
    kind: &DriveKind,
    drive_index: usize,
    verified: Option<&journal_files::VerifiedBase>,
) -> Result<journal_files::VerifiedBase, ConfigError> {
    let mount = FileMount::with_verified_bases(
        verified
            .map(|base| (path.to_path_buf(), base.clone()))
            .into_iter()
            .collect(),
    );
    verify_mounted_base(&mount, path, expected, kind, drive_index).map(|base| {
        journal_files::VerifiedBase::from_verified_source(
            Rc::new(base.verified().clone()),
            base.verified().identity().clone(),
        )
        .unwrap()
    })
}

/// Test wrapper for drive construction without retaining session handles.
fn build_fdc_drives(
    config: &MachineConfig,
    controller: DeviceId,
) -> Result<[Option<Drive>; 4], AssemblyError> {
    mount_fdc_drives(
        config,
        controller,
        &mut Vec::new(),
        &[],
        &(Rc::new(InlineMount) as Rc<dyn Mount>),
    )
}

#[path = "image_source_tests.rs"]
mod image_source_tests;
