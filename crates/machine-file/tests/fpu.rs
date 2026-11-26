// SPDX-License-Identifier: AGPL-3.0-or-later
//! IBM Personal Computer AT Technical Reference, Real-Time Clock/CMOS RAM
//! Information, Equipment Byte (Hex 14): coprocessor selection and firmware probes.

use std::{io, path::Path};

use assembly::*;
use machine_file::{lower, parse_toml, parse_yaml, ImageReader, Origins, ParseError, YamlOrigins};

fn probe_rom() -> Vec<u8> {
    let code = [
        0xFA, 0x31, 0xC0, 0x8E, 0xD8, // CLI; zero DS
        0xB0, 0x14, 0xE6, 0x70, 0xE4, 0x71, 0xA2, 0x02, 0x05, // CMOS equipment
        0xB0, 0x2E, 0xE6, 0x70, 0xE4, 0x71, 0xA2, 0x03, 0x05, // checksum high
        0xB0, 0x2F, 0xE6, 0x70, 0xE4, 0x71, 0xA2, 0x04, 0x05, // checksum low
        0xC7, 0x06, 0x00, 0x05, 0x5A, 0x5A, // presence-check sentinel
        0xDB, 0xE3, // FNINIT
        0xD9, 0x3E, 0x00, 0x05, // FNSTCW [0500h]
        0xDD, 0x3E, 0x06, 0x05, // FNSTSW [0506h]
        0xB8, 0x5A, 0xA5, 0xDF, 0xE0, 0xA3, 0x08, 0x05, // FNSTSW AX; save AX
        0x9B, 0xF4, // WAIT; HLT
    ];
    let mut rom = vec![0xF4; 0x8000];
    rom[..code.len()].copy_from_slice(&code);
    rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, 0, 0, 0, 0xF8]);
    rom
}

struct Images;
impl ImageReader for Images {
    fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
        Ok(probe_rom())
    }
}

fn twins(fpu: Option<&str>) -> (MachineConfig, MachineConfig) {
    let toml = format!(
        "schema = 1\n[board]\nmodel = 'ibm-5170'\n[firmware.bios]\nimage = 'bios.bin'\n{}",
        fpu.map(|name| format!("[cpu]\nfpu = '{name}'\n"))
            .unwrap_or_default()
    );
    let yaml = format!(
        "schema: 1\nboard: {{model: ibm-5170}}\nfirmware: {{bios: {{image: bios.bin}}}}\n{}",
        fpu.map(|name| format!("cpu: {{fpu: '{name}'}}\n"))
            .unwrap_or_default()
    );
    let toml_path = Path::new("machines/probe.toml");
    let yaml_path = Path::new("machines/probe.yaml");
    let toml_config = lower(
        &parse_toml(toml_path, &toml).unwrap(),
        &Origins::from_toml(toml_path, &toml).unwrap(),
        &mut Images,
    )
    .unwrap();
    let yaml_config = lower(
        &parse_yaml(yaml_path, &yaml).unwrap(),
        &Origins::from_yaml(yaml_path, &yaml).unwrap(),
        &mut Images,
    )
    .unwrap();
    (toml_config, yaml_config)
}

fn probe(config: &MachineConfig, present: bool, equipment: u8, checksum: [u8; 2]) {
    let mut assembled = assemble_with_options(
        config,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    let outcome = assembled.machine.run_bounded(Some(100));
    let mut result = [0; 10];
    assembled
        .machine
        .cpu()
        .read_mem(0x500, &mut result)
        .unwrap();
    assert_eq!(
        &result[..2],
        if present { &[0x7F, 3] } else { &[0xFF, 0xFF] }
    );
    assert_eq!(result[2], equipment);
    assert_eq!(&result[3..5], &checksum);
    assert_eq!(&result[6..], if present { &[0; 4] } else { &[0xFF; 4] });
    assert_eq!(outcome.unwrap(), machine::RunOutcome::Halted);
}

#[test]
fn ibm_5170_default_probe_reports_absent_coprocessor() {
    let config = bios(probe_rom(), Some(vec![0; 1_474_560]));
    let seed = derive_cmos_config(&config).unwrap();
    assert_eq!(seed[0x14] & 2, 0);
    probe(&config, false, seed[0x14], [seed[0x2E], seed[0x2F]]);
}

#[test]
fn fpu_machine_file_twins_select_socket_and_cmos() {
    for (fpu, present) in [(None, false), (Some("none"), false), (Some("80287"), true)] {
        let (toml, yaml) = twins(fpu);
        assert_eq!(toml, yaml);
        let seed = derive_cmos_config(&toml).unwrap();
        assert_eq!(seed[0x14] & 2, if present { 2 } else { 0 });
        let checksum: u16 = seed[0x10..=0x2D].iter().map(|&v| u16::from(v)).sum();
        assert_eq!([seed[0x2E], seed[0x2F]], checksum.to_be_bytes());
        probe(&toml, present, seed[0x14], checksum.to_be_bytes());
    }
}

#[test]
fn explicit_cmos_image_is_preserved_with_80287() {
    let (mut config, _) = twins(Some("80287"));
    let DeviceKind::Rtc(rtc) = &mut config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Rtc)
        .unwrap()
        .kind
    else {
        panic!("RTC required")
    };
    let mut image = [0; 64];
    image[0x14] = 0x41;
    image[0x2E] = 0xA5;
    image[0x2F] = 0x5A;
    rtc.nvram = NvramSource::Image(image);
    probe(&config, true, 0x41, [0xA5, 0x5A]);
}

#[test]
fn unsupported_fpu_names_have_exact_typed_diagnostics() {
    for name in ["8087", "present", "80288", "NONE"] {
        let toml = format!("schema = 1\n[cpu]\nfpu = '{name}'\n");
        let yaml = format!("schema: 1\ncpu: {{fpu: '{name}'}}\n");
        let errors: [ParseError; 2] = [
            parse_toml("probe.toml", &toml).unwrap_err(),
            parse_yaml("probe.yaml", &yaml).unwrap_err(),
        ];
        for error in errors {
            assert_eq!(
                error.message(),
                format!(
                    "unsupported cpu.fpu `{name}`; allowed values: `none`, `80287`, `80387`, `integrated`"
                )
            );
        }
    }
}

#[test]
fn at_numeric_errors_reach_irq13_before_wait_without_idle_ticks() {
    use cpu::Cpu;
    let mut assembled = immediate_irq13_machine();
    assert_eq!(assembled.machine.idle_limit(), None);
    assert_eq!(
        assembled.machine.run_bounded(Some(500)).unwrap(),
        machine::RunOutcome::RetiredCapReached,
        "an unmasked numeric error must reach the handler before WAIT blocks"
    );
    let mut result = [0; 12];
    assembled
        .machine
        .cpu()
        .read_mem(0x500, &mut result)
        .unwrap();
    assert_eq!(&result[..6], &[2, 0, 0, 0, 0, 0]);
    assert_eq!(&result[10..], &0xa55au16.to_le_bytes());
    assert_eq!(
        assembled.machine.virtual_time(),
        assembled.machine.cpu().completed_steps()
    );
    assert!(!assembled.machine.halted());
}

fn immediate_irq13_machine() -> AssembledMachine {
    struct NumericErrorImage;
    impl ImageReader for NumericErrorImage {
        fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
            Ok(include_bytes!("fixtures/at-irq13-immediate.bin").to_vec())
        }
    }
    let path = Path::new("machines/numeric-error.toml");
    let text = "schema = 1\n[board]\nmodel = 'ibm-5170'\n[cpu]\nfpu = '80287'\n[firmware.bios]\nimage = 'bios.bin'\n";
    let config = lower(
        &parse_toml(path, text).unwrap(),
        &Origins::from_toml(path, text).unwrap(),
        &mut NumericErrorImage,
    )
    .unwrap();
    assemble_with_options(
        &config,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap()
}

#[test]
fn at_suspended_wait_survives_device_restore_and_resumes_without_idle_ticks() {
    use cpu::{Cpu, CpuReg};
    let mut assembled = immediate_irq13_machine();
    for cap in 1..500 {
        assert_eq!(
            assembled.machine.run_bounded(Some(cap)).unwrap(),
            machine::RunOutcome::RetiredCapReached
        );
        if assembled.machine.cpu().numeric_error_pending() {
            break;
        }
    }
    assert!(assembled.machine.cpu().numeric_error_pending());
    assembled
        .machine
        .bus_mut()
        .port_write(0xa1, 1, 0xff)
        .unwrap();
    assert_eq!(
        assembled.machine.run().unwrap(),
        machine::RunOutcome::Halted
    );
    assert!(assembled.machine.halted());
    let held = (
        assembled.machine.cpu().reg(CpuReg::Rip),
        assembled.machine.cpu().retired(),
        assembled.machine.clock(),
    );
    let index = assembled.device_index[&DeviceId::Coprocessor];
    let snapshot = assembled.machine.bus().device(index).unwrap().save();
    let device = assembled.machine.bus_mut().device_mut(index).unwrap();
    device.reset();
    assert!(!device.irq_pending());
    device.restore(&snapshot).unwrap();
    assert!(device.irq_pending());
    assert_eq!(
        assembled.machine.run().unwrap(),
        machine::RunOutcome::Halted
    );
    assert_eq!(
        (
            assembled.machine.cpu().reg(CpuReg::Rip),
            assembled.machine.cpu().retired(),
            assembled.machine.clock(),
        ),
        held,
        "device restoration must preserve the suspended WAIT and its time"
    );
    assembled
        .machine
        .bus_mut()
        .port_write(0xa1, 1, 0xdf)
        .unwrap();
    assert_eq!(
        assembled.machine.run_bounded(Some(500)).unwrap(),
        machine::RunOutcome::RetiredCapReached,
        "the already latched IRQ must wake WAIT on the next run"
    );
    let mut result = [0; 12];
    assembled
        .machine
        .cpu()
        .read_mem(0x500, &mut result)
        .unwrap();
    assert_eq!(&result[..6], &[2, 0, 0, 0, 0, 0]);
    assert_eq!(&result[10..], &0xa55au16.to_le_bytes());
    assert_eq!(assembled.machine.idle_limit(), None);
    assert!(!assembled.machine.halted());
}

#[test]
fn at_numeric_errors_reach_irq13_and_wait_keeps_device_time_running() {
    use cpu::Cpu;
    assert!(
        include_str!("../../../Cargo.lock").contains("#be0b80d9e13e3ba23da6b540915173488cd1c879")
    );
    let started = std::time::Instant::now();
    let mut config = bios(
        include_bytes!("fixtures/at-irq13.bin").to_vec(),
        Some(vec![0; 1_474_560]),
    );
    config.cpu.fpu = FpuChip::I80287;
    let mut assembled = assemble_with_options(
        &config,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    assembled.machine.set_idle_limit(Some(2000));
    assert_eq!(
        assembled.machine.run_bounded(Some(500)).unwrap(),
        machine::RunOutcome::RetiredCapReached
    );
    assert!(
        assembled.machine.halted(),
        "WAIT remains suspended across runner calls"
    );
    assert_eq!(
        assembled.machine.run_bounded(Some(100_000)).unwrap(),
        machine::RunOutcome::Halted
    );
    let mut bytes = [0; 12];
    assembled.machine.cpu().read_mem(0x500, &mut bytes).unwrap();
    let words: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect();
    assert_eq!(words[0], 2, "one IRQ13 for each unmasked error");
    assert_eq!(words[1], 0, "AT numeric errors never deliver exception 16");
    assert_eq!(words[2], 1, "the timer unmasked IRQ13 while WAIT was held");
    assert_ne!(words[3] & 0x80, 0, "ES was asserted in the handler");
    assert_eq!(
        words[3], words[4],
        "F0h must preserve the complete status word"
    );
    assert_eq!(
        words[5], 0xa55a,
        "both WAIT retries and the masked error completed"
    );
    assert!(
        assembled.machine.virtual_time() > assembled.machine.cpu().retired(),
        "blocked WAIT advances virtual time without retiring instructions"
    );
    assert_eq!(assembled.machine.cpu().retired(), 96);
    eprintln!(
        "IRQ13 guest: retired={}, virtual={}, wall={:?}",
        assembled.machine.cpu().retired(),
        assembled.machine.virtual_time(),
        started.elapsed()
    );
}

fn generic386_config(fpu: &str) -> MachineConfig {
    struct ProbeImage;
    impl ImageReader for ProbeImage {
        fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
            let mut bytes = vec![0xf4; 0x18000];
            bytes.extend_from_slice(include_bytes!("fixtures/i386-387.bin"));
            Ok(bytes)
        }
    }
    let text = format!("schema = 1\n[board]\nmodel = 'generic-386-isa'\n[cpu]\nfpu = '{fpu}'\n[firmware.bios]\nimage = 'probe.bin'\n[[card]]\nkind = 'uart-16550'\n");
    let yaml = format!("schema: 1\nboard: {{model: generic-386-isa}}\ncpu: {{fpu: '{fpu}'}}\nfirmware: {{bios: {{image: probe.bin}}}}\ncard: [{{kind: uart-16550}}]\n");
    let path = Path::new("probe.toml");
    let toml = lower(
        &parse_toml(path, &text).unwrap(),
        &Origins::from_toml(path, &text).unwrap(),
        &mut ProbeImage,
    )
    .unwrap();
    let path = Path::new("probe.yaml");
    let yaml = lower(
        &parse_yaml(path, &yaml).unwrap(),
        &Origins::from_yaml(path, &yaml).unwrap(),
        &mut ProbeImage,
    )
    .unwrap();
    assert_eq!(toml, yaml);
    toml
}

#[test]
fn generic386_80387_reset_probe_initialization_arithmetic_and_irq13() {
    generic386_fpu_probe(true);
}

#[test]
fn generic386_no_fpu_reset_probe_all_ones_and_et_clear() {
    generic386_fpu_probe(false);
}

fn generic386_fpu_probe(present: bool) {
    use cpu::Cpu;
    let config = generic386_config(if present { "80387" } else { "none" });
    let mut assembled = assemble_with_options(
        &config,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap();
    assert_eq!(
        assembled.machine.run_bounded(Some(500)).unwrap(),
        machine::RunOutcome::Halted
    );
    let mut result = [0; 22];
    assembled
        .machine
        .cpu()
        .read_mem(0x500, &mut result)
        .unwrap();
    let words: Vec<u16> = result
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| u16::from_le_bytes(*v))
        .collect();
    assert_eq!(words[0], if present { 0 } else { 0xffff }, "FNSTSW AX");
    assert_eq!(words[1], if present { 0x037f } else { 0xffff }, "FNSTCW");
    let et = if present { 0x10 } else { 0 };
    assert_eq!(words[2] & 0x10, et, "SMSW ET");
    let cr0 = u32::from_le_bytes(result[6..10].try_into().unwrap());
    assert_eq!(cr0, u32::from(et), "reset CR0 model");
    assert_eq!(words[5], if present { 42 } else { 0 }, "7 * 6");
    assert_eq!(words[6], u16::from(present), "INT 75h count");
    assert_eq!(words[7], 0, "no internal #MF");
    if present {
        assert_eq!(words[8] & 0x84, 0x84, "unmasked zero divide status");
        assert_eq!(words[8], words[9], "F0h preserves exception status");
    }
    assert_eq!(words[10], 0xa55a, "WAIT resumed after F0h");
    assert!(!assembled.machine.cpu().numeric_error_pending());
}

#[test]
fn wrong_board_80387_has_positioned_diagnostic() {
    for board in ["ibm-5170", "generic-486-isa"] {
        let model = if board == "ibm-5170" {
            "80286"
        } else {
            "80486dx"
        };
        let text = format!("schema = 1\n[board]\nmodel = '{board}'\n[cpu]\nmodel = '{model}'\nfpu = '80387'\n[firmware.bios]\nimage = 'probe.bin'\n[[card]]\nkind = 'uart-16550'\n");
        let path = Path::new("probe.toml");
        let error = lower(
            &parse_toml(path, &text).unwrap(),
            &Origins::from_toml(path, &text).unwrap(),
            &mut Images,
        )
        .unwrap_err();
        assert_eq!(error.field, "cpu.fpu");
        assert_eq!(error.origin.file, path);
        assert_eq!(error.origin.line, Some(6));
        assert!(
            error
                .message
                .contains("80387 requires board generic-386-isa"),
            "{error}"
        );
    }
}
