// SPDX-License-Identifier: AGPL-3.0-or-later
//! IBM PC/AT optional coprocessor detection with the open Kiselev BIOS.
//! See `backlog/docs/design/machine-config.md` §CPU model selection.

use assembly::{bios, FpuChip};
use cpu::Cpu;

#[path = "support/fixtures.rs"]
mod fixtures;

#[test]
#[ignore = "requires the open BIOS fixture and runs its coprocessor detection twice"]
fn kiselev_empty_socket_finishes_the_coprocessor_probe() {
    let Some(rom) = fixtures::fixture("kiselev-bios/bios-xi8088.bin") else {
        return;
    };
    let mut screens = Vec::new();
    for fpu in [FpuChip::I80287, FpuChip::None] {
        let mut config = bios(rom.clone(), Some(pc_harness::bootable_floppy_image()));
        config.cpu.fpu = fpu;
        let policy = assembly::ObservationPolicy {
            routes: vec![assembly::ObservationRequest {
                device: assembly::DeviceId::Mda,
                output: assembly::OutputKind::TextScanout,
                byte_limit: 0,
            }],
            ..Default::default()
        };
        let mut assembled = machine_file::assemble_with_observation(&config, &policy).unwrap();
        assert_eq!(
            assembled
                .run_session(&assembly::RunPolicy::natural(
                    Some(pc_harness::BIOS_GOLDEN_MAX_RETIRED),
                    Some(pc_harness::BIOS_IDLE_LIMIT),
                ))
                .unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        let mut equipment = [0; 2];
        assembled
            .machine
            .cpu()
            .read_mem(0x410, &mut equipment)
            .unwrap();
        assembled
            .machine
            .bus_mut()
            .port_write(0x70, 1, 0x14)
            .unwrap();
        let cmos = assembled.machine.bus_mut().port_read(0x71, 1).unwrap();
        let equipment = u16::from_le_bytes(equipment);
        // The absent branch prints its result without clearing the equipment
        // bit inherited from CMOS, even for the present engine's 037Fh word.
        assert_eq!(equipment & 2, cmos as u16 & 2);
        assert_eq!(cmos & 2, if fpu == FpuChip::None { 0 } else { 2 });
        eprintln!(
            "FPU {fpu:?}: BDA equipment={equipment:04x}; CMOS14={cmos:02x}; retired={}",
            assembled.machine.cpu().retired()
        );
        let video::Frame::Text(grid) = assembled
            .capture_text(assembly::DeviceId::Mda)
            .unwrap()
            .frame
        else {
            panic!("text frame required")
        };
        let screen = grid.rows_as_strings();
        eprintln!("FPU {fpu:?}:\n{}", screen.join("\n"));
        screens.push(screen);
    }
    // The generic present engine already fails this BIOS's 03FFh control-word
    // comparison, so both policies retain its existing "Absent" report.
    assert_eq!(screens[0], screens[1]);
    let report = screens[0]
        .iter()
        .find(|line| line.starts_with("Mathematics Co-processor:"))
        .expect("coprocessor report");
    assert_eq!(report, "Mathematics Co-processor:   Absent");
    eprintln!("Coprocessor report: {report:?} -> {report:?}");
}

#[test]
#[ignore = "requires the open BIOS fixture and boots the project-owned IRQ13 guest"]
fn kiselev_irq13_chains_to_int2_for_numeric_errors() {
    let Some(rom) = fixtures::fixture("kiselev-bios/bios-xi8088.bin") else {
        return;
    };
    assert!(
        include_str!("../../../Cargo.lock").contains("#aca03742f526a867415ea584fa27c7988b9d344f")
    );
    // Kiselev 8088 BIOS, bios.asm int_75: both EOIs precede software INT 02h.
    // The guest keeps that firmware vector and replaces only INT 02h.
    let mut floppy = vec![0; 1_474_560];
    floppy[..512].copy_from_slice(include_bytes!(
        "../../machine-file/tests/fixtures/at-irq13-boot.bin"
    ));
    let mut config = bios(rom, Some(floppy));
    config.cpu.fpu = FpuChip::I80287;
    let started = std::time::Instant::now();
    let mut assembled = machine_file::assemble(&config).unwrap();
    assembled.machine.set_idle_limit(Some(2000));
    assert_eq!(
        assembled
            .machine
            .run_bounded(Some(pc_harness::BIOS_GOLDEN_MAX_RETIRED))
            .unwrap(),
        machine::RunOutcome::Halted
    );
    let mut result = [0; 12];
    assembled
        .machine
        .cpu()
        .read_mem(0x500, &mut result)
        .unwrap();
    assert_eq!(
        u16::from_le_bytes([result[0], result[1]]),
        2,
        "two BIOS INT 75h entries reached the guest INT 02h handler"
    );
    assert_eq!(&result[2..4], &[0; 2], "no exception 16");
    assert_eq!(&result[10..], &0xa55au16.to_le_bytes(), "guest finished");
    eprintln!(
        "BIOS IRQ13 chain: retired={}, wall={:?}",
        assembled.machine.cpu().retired(),
        started.elapsed()
    );
}
