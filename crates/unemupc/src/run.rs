// SPDX-License-Identifier: AGPL-3.0-or-later
//! The PC's machine runs from the command line: `--boot`, `--bios`,
//! `machine check` and `run --machine`; `backlog/docs/design/machine-files.md`
//! § Frontends and `backlog/docs/design/firmware-and-machine-classes.md` §6.
//! They call the runners of `pc-harness`, the same runners its golden tests
//! pin, so the command line and the goldens cannot drift apart.

use anyhow::Context;
use pc_harness::StopReason;
use std::io::Write;
use std::path::PathBuf;

/// The firmware store a `run` command asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum FirmwareStore {
    Default,
    Disabled,
    Root(PathBuf),
}

pub(super) fn check_machine(
    path: &std::path::Path,
    options: assembly::AssemblyOptions,
    rom_dirs: &[PathBuf],
    errors: &mut impl Write,
) -> anyhow::Result<u8> {
    let roots = rom_dirs
        .iter()
        .map(|path| {
            machine_file::firmware_discovery::FirmwareRoot::new(
                path,
                machine_file::firmware_discovery::RootOrigin::CommandLine,
            )
        })
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    let result =
        machine_file::load_for_assembly_with_firmware_roots(path, options, &roots, &mut warnings);
    for warning in warnings {
        writeln!(errors, "warning: {warning}")?;
    }
    let loaded = result?;
    for note in loaded.check()? {
        writeln!(errors, "{note}")?;
    }
    Ok(0)
}

/// Firmware discovery roots in command-line order.
pub fn firmware_roots(rom_dirs: &[PathBuf]) -> Vec<machine_file::firmware_discovery::FirmwareRoot> {
    rom_dirs
        .iter()
        .map(|path| {
            machine_file::firmware_discovery::FirmwareRoot::new(
                path,
                machine_file::firmware_discovery::RootOrigin::CommandLine,
            )
        })
        .collect()
}

/// Load a standalone machine with the command line's firmware discovery and warnings.
/// Both the headless runner and picture window use this preparation path.
pub fn load_machine(
    path: &std::path::Path,
    options: assembly::AssemblyOptions,
    rom_dirs: &[PathBuf],
    errors: &mut (impl Write + ?Sized),
) -> anyhow::Result<machine_file::LoadedMachine> {
    load_named_machine(path, options, rom_dirs, errors).map(|(_, loaded)| loaded)
}

/// The resolved machine name and the same loaded configuration as [`load_machine`].
/// The name is presentation metadata and does not enter emulation configuration.
pub fn load_named_machine(
    path: &std::path::Path,
    options: assembly::AssemblyOptions,
    rom_dirs: &[PathBuf],
    errors: &mut (impl Write + ?Sized),
) -> anyhow::Result<(Option<String>, machine_file::LoadedMachine)> {
    let roots = firmware_roots(rom_dirs);
    let mut warnings = Vec::new();
    let result = machine_file::prepare_for_assembly_with_firmware_packages(
        path,
        &roots,
        &mut warnings,
        None,
    )
    .and_then(|document| {
        let name = document.resolved().0.name.clone();
        document.load(options).map(|loaded| (name, loaded))
    });
    for warning in warnings {
        writeln!(errors, "warning: {warning}")?;
    }
    Ok(result?)
}

/// Run an already prepared machine. The caller owns host firmware lookup.
pub fn run_machine(
    loaded: machine_file::LoadedMachine,
    limits: (u64, u64, Option<u64>, bool),
    print: Option<&str>,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> anyhow::Result<u8> {
    run_machine_with_turbo(loaded, limits, false, print, output, errors)
}

/// Run a prepared machine with an initial turbo-button position.
pub fn run_machine_with_turbo(
    loaded: machine_file::LoadedMachine,
    limits: (u64, u64, Option<u64>, bool),
    turbo: bool,
    print: Option<&str>,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> anyhow::Result<u8> {
    use assembly::{DeviceId, ObservationPolicy, ObservationRequest, OutputKind, RunPolicy};
    let (max_steps, idle_limit, diag_at, trace_exceptions) = limits;
    let printable =
        |device: DeviceId| matches!(device, DeviceId::Mda | DeviceId::Vga | DeviceId::Uart);
    let selected = if let Some(name) = print {
        let (_, device) = loaded
            .cards
            .iter()
            .find(|(id, _)| id == name)
            .ok_or_else(|| anyhow::anyhow!("output card {name:?} is not configured"))?;
        if !printable(*device) {
            writeln!(errors, "card {name:?} has no printable output")?;
            return Ok(2);
        }
        Some(*device)
    } else {
        loaded
            .cards
            .iter()
            .find(|(_, device)| matches!(device, DeviceId::Mda | DeviceId::Vga))
            .or_else(|| {
                loaded
                    .cards
                    .iter()
                    .find(|(_, device)| *device == DeviceId::Uart)
            })
            .map(|(_, device)| *device)
    };
    let mut observation = ObservationPolicy::default();
    if let Some(device) = selected {
        observation.routes.push(ObservationRequest {
            device,
            output: if device == DeviceId::Uart {
                OutputKind::Bytes(device::ByteStreamKind::SerialTx)
            } else {
                OutputKind::TextScanout
            },
            byte_limit: 1_048_576,
        });
    }
    let mut assembled = loaded.assemble(&observation)?;
    anyhow::ensure!(
        assembled.machine.set_turbo(turbo),
        "--turbo requires cpu.turbo.clock_mhz"
    );
    for warning in &assembled.unlisted_firmware {
        writeln!(errors, "{warning}")?;
    }
    if trace_exceptions {
        if let Err(error) = assembled.start_delivery_recording() {
            writeln!(errors, "warning: exceptions are not traced: {error}")?;
        }
    }
    let policy = RunPolicy::natural(Some(max_steps), Some(idle_limit));
    let result = match diag_at.filter(|at| *at <= max_steps) {
        Some(at) => {
            assembled.machine.enable_code_trail();
            // Without a log the report says the deliveries were not recorded.
            let _ = assembled.enable_delivery_log();
            match assembled.run_session(&RunPolicy::natural(Some(at), Some(idle_limit))) {
                Ok(machine::RunOutcome::RetiredCapReached) => {
                    write!(errors, "{}", assembled.diagnostic_report("running"))?;
                    assembled.run_session(&policy)
                }
                other => other,
            }
        }
        None => assembled.run_session(&policy),
    };
    let result = result.and_then(|outcome| {
        if matches!(
            outcome,
            machine::RunOutcome::Halted | machine::RunOutcome::RetiredCapReached
        ) {
            pc_harness::finish_persistence(&mut assembled, errors)?;
        }
        Ok(outcome)
    });
    let report = cpu_stop_report(&assembled, &result);
    let mut stop = pc_harness::classify_bios_stop(&assembled.machine, result);
    let captured = match selected {
        Some(DeviceId::Uart) => assembled
            .observation
            .drain_bytes(DeviceId::Uart, device::ByteStreamKind::SerialTx),
        Some(device) => assembled
            .capture_text(device)
            .and_then(|capture| display_dump(capture.frame, device)),
        None => Ok(Vec::new()),
    };
    let bytes = match captured {
        Ok(bytes) => bytes,
        Err(error) => {
            stop = StopReason::ObservationFailed {
                detail: error.to_string(),
            };
            Vec::new()
        }
    };
    if let Some(recording) = assembled.machine.stop_delivery_recording() {
        errors.write_all(recording.as_bytes())?;
    }
    let (message, failed) = bios_stop(&stop);
    writeln!(errors, "{}", message.replace(['\r', '\n'], " "))?;
    if let Some(report) = report {
        write!(errors, "{report}")?;
    }
    output.write_all(&bytes)?;
    Ok(u8::from(failed))
}

/// The CPU stop report of a run the CPU core ended by refusing an
/// instruction, read before anything else touches the CPU; see
/// `backlog/docs/design/observation.md` §CPU stop report.
fn cpu_stop_report(
    assembled: &assembly::AssembledMachine,
    result: &Result<machine::RunOutcome, assembly::RunError>,
) -> Option<String> {
    let Err(assembly::RunError::Machine(machine::MachineError::Cpu(error))) = result else {
        return None;
    };
    assembled
        .machine
        .cpu()
        .stop_report(error)
        .map(|report| report.to_string())
}

pub(super) fn run_bios(
    path: &std::path::Path,
    options: assembly::AssemblyOptions,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> anyhow::Result<u8> {
    let run = pc_harness::run_bios_file(path, options, errors)?;
    output
        .write_all(&run.com1)
        .context("writing serial output")?;
    write_bios_stop(&run.stop, errors)
}

fn write_bios_stop(stop: &StopReason, errors: &mut impl Write) -> anyhow::Result<u8> {
    let (message, failed) = bios_stop(stop);
    writeln!(errors, "{}", message.replace(['\r', '\n'], " "))?;
    Ok(u8::from(failed))
}

fn bios_stop(stop: &StopReason) -> (String, bool) {
    use StopReason::*;
    let (reason, failed) = match stop {
        Halt => ("halt".into(), false),
        ResetRequested => ("reset requested".into(), false),
        BudgetExhausted => ("budget exhausted".into(), false),
        Unsupported { detail } => (format!("unsupported instruction: {detail}"), true),
        Unmapped { detail } => (format!("execution error: {detail}"), true),
        Exception { vector } => (format!("exception {vector}"), true),
        Fault { detail } => (format!("cpu fault: {detail}"), true),
        ConfigFailed { detail } => (format!("configuration error: {detail}"), true),
        PolicyFailed { detail } => (format!("run policy error: {detail}"), true),
        ObservationFailed { detail } => (format!("observation error: {detail}"), true),
        PersistenceFailed { detail } => (format!("persistence error: {detail}"), true),
    };
    (format!("stopped: {reason}"), failed)
}

fn display_dump(
    frame: video::Frame,
    device: assembly::DeviceId,
) -> Result<Vec<u8>, assembly::ObservationError> {
    let video::Frame::Text(grid) = frame else {
        return Err(assembly::ObservationError::GraphicsMode(device));
    };
    Ok(format!("{}\n", grid.to_dump()).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assembly::DeviceId;

    #[test]
    fn text_dump_reports_graphics_mode_explicitly_and_preserves_text_bytes() {
        let frame = video::Frame::Pixels(video::IndexedPixels {
            width: 1,
            height: 1,
            repeat: (1, 1),
            indices: vec![0],
            palette: vec![[0; 3]].into_boxed_slice(),
        });
        let error = display_dump(frame, DeviceId::Vga).unwrap_err();
        assert_eq!(
            error,
            assembly::ObservationError::GraphicsMode(DeviceId::Vga)
        );
        assert_eq!(error.to_string(), "display is in graphics mode: Vga");
        let grid = video::render_text_mode(&video::TextModeState {
            vram: &[b'A', 7],
            cols: 1,
            rows: 1,
            start_cell: 0,
            cursor_cell: 0,
            cursor_visible: false,
            attributes: video::TextAttributeSet::Colour {
                blink_enabled: false,
            },
        });
        let expected = format!("{}\n", grid.to_dump()).into_bytes();
        assert_eq!(
            display_dump(video::Frame::Text(grid), DeviceId::Vga).unwrap(),
            expected
        );
    }

    #[test]
    fn synthetic_unmapped_cpu_stop_has_failure_diagnostic() {
        // Inject the stop so reporting does not depend on core memory accesses.
        let assembled = machine_file::assemble(&assembly::at_boot(vec![0xF4])).unwrap();
        let stop = pc_harness::classify_bios_stop(
            &assembled.machine,
            Err(assembly::RunError::Machine(machine::MachineError::Cpu(
                cpu::CpuError::Unmapped { addr: 0x100000 },
            ))),
        );
        let mut errors = Vec::new();
        assert_eq!(write_bios_stop(&stop, &mut errors).unwrap(), 1);
        let message = String::from_utf8(errors).unwrap();
        assert!(message
            .starts_with("stopped: execution error: unmapped guest access at 0x100000; cs:ip "));
        assert_eq!(message.lines().count(), 1);
        assert!(message.ends_with('\n'));
    }

    /// `run` prints the stop report of a CPU refusal after its one-line
    /// stop; other stops print none.
    #[test]
    fn a_cpu_refusal_has_a_stop_report_and_a_halt_has_none() {
        let mut assembled =
            machine_file::assemble(&assembly::test_support::refused_gate_call()).unwrap();
        let result = assembled.run_session(&assembly::RunPolicy::natural(Some(100_000), None));
        let report = cpu_stop_report(&assembled, &result).expect("the refusal has a report");
        assert!(report.starts_with(
            "cpu stop report\nstop     unsupported cpu system operation LegacyInstruction at 002b:7c2b\n"
        ));
        assert!(
            report.contains("desc     target 0023 (gdt) 23 7c 08 00 1f e4 00 00: 286 call gate")
        );
        assert!(report.ends_with("ss2:sp2 0030:4000\n"));

        let mut assembled = machine_file::assemble(&assembly::at_boot(vec![0xFA, 0xF4])).unwrap();
        let result = assembled.run_session(&assembly::RunPolicy::natural(Some(100_000), None));
        assert!(result.is_ok());
        assert_eq!(cpu_stop_report(&assembled, &result), None);
    }

    #[test]
    fn bios_stop_classifies_every_outcome() {
        use StopReason::*;
        for stop in [Halt, ResetRequested, BudgetExhausted] {
            let (message, failed) = bios_stop(&stop);
            assert!(!failed);
            if stop == BudgetExhausted {
                assert_eq!(message, "stopped: budget exhausted");
            }
        }
        for stop in [
            PersistenceFailed {
                detail: "journal".into(),
            },
            Unsupported {
                detail: "opcode".into(),
            },
            Unmapped {
                detail: "address".into(),
            },
            Exception { vector: 13 },
            Fault {
                detail: "vector 13 at 0008:1000".into(),
            },
            ConfigFailed {
                detail: "config".into(),
            },
            PolicyFailed {
                detail: "budget".into(),
            },
            ObservationFailed {
                detail: "route".into(),
            },
        ] {
            assert!(bios_stop(&stop).1);
        }
    }
}
