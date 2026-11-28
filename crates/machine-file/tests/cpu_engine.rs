// SPDX-License-Identifier: AGPL-3.0-or-later
//! The `cpu.engine` machine-file key, and equal guest results from both CPU
//! engines on an open-firmware board. The core links no JIT backend; these
//! tests supply x86jit-cranelift's, as the host programs do.
//! See machine-files.md §CPU engine.

use assembly::{CpuEngine, DeviceId, JitBackendFactory};
use cpu::{Cpu, CpuReg, CpuStepStop, CpuStop, StepRunOptions, X86JitCpu};
use std::path::{Path, PathBuf};

fn cranelift() -> JitBackendFactory {
    JitBackendFactory(|| Box::new(x86jit_cranelift::JitBackend::new()))
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// An IBM 5170 machine with a synthetic 32 KiB BIOS, whose `[cpu]` table
/// holds `cpu` (one key per line). Returns the file and its contents.
fn at_machine(temp: &machine_file::test_support::TempDir, cpu: &str) -> (PathBuf, String) {
    let mut rom = vec![0; 32768];
    rom[0x7FF0..0x7FF2].copy_from_slice(&[0xFA, 0xF4]);
    std::fs::write(temp.path().join("bios.bin"), rom).unwrap();
    let text = format!(
        "schema = 1\n[board]\nmodel = \"ibm-5170\"\n[cpu]\n{cpu}\n[firmware.bios]\nimage = \"bios.bin\"\n[[card]]\nkind = \"mda\"\n"
    );
    let path = temp.path().join("machine.toml");
    std::fs::write(&path, &text).unwrap();
    (path, text)
}

fn unlisted() -> assembly::AssemblyOptions {
    assembly::AssemblyOptions {
        allow_unlisted_firmware: true,
    }
}

/// The 1-based line of `text` that starts with `prefix`.
fn line_of(text: &str, prefix: &str) -> usize {
    text.lines()
        .position(|line| line.starts_with(prefix))
        .unwrap()
        + 1
}

#[test]
fn engine_defaults_to_the_interpreter_and_accepts_both_spellings() {
    let temp = machine_file::test_support::TempDir::new("cpu-engine-default");
    for (cpu, engine) in [
        ("", CpuEngine::Interpreter),
        ("engine = \"interpreter\"", CpuEngine::Interpreter),
    ] {
        let (path, _) = at_machine(&temp, cpu);
        let config = machine_file::load_with_options(&path, unlisted()).unwrap();
        assert_eq!(config.cpu.engine, engine, "{cpu:?}");
    }
    let document =
        machine_file::parse_toml("machine.toml", "schema = 1\n[cpu]\nengine = \"jit\"\n").unwrap();
    assert_eq!(
        document.cpu.unwrap().engine,
        Some(machine_file::schema::v1::CpuEngine::Jit)
    );
}

#[test]
fn unknown_engine_is_refused_at_its_value() {
    let temp = machine_file::test_support::TempDir::new("cpu-engine-unknown");
    let (path, text) = at_machine(&temp, "model = \"80286\"\nengine = \"cranelift\"");
    let error = machine_file::load_with_options(&path, unlisted()).unwrap_err();
    let machine_file::LoadError::Parse(error) = error else {
        panic!("expected a parse error, got {error:?}");
    };
    let expected = "unsupported cpu.engine `cranelift`; allowed values: `interpreter`, `jit`";
    assert_eq!(error.message(), expected);
    assert_eq!(error.path(), path);
    assert_eq!(
        error.line_column().map(|(line, _)| line),
        Some(line_of(&text, "engine"))
    );
    for name in ["cranelift", "JIT", "Interpreter", ""] {
        let yaml = format!("schema: 1\ncpu: {{engine: '{name}'}}\n");
        assert_eq!(
            machine_file::parse_yaml("probe.yaml", &yaml)
                .unwrap_err()
                .message(),
            format!("unsupported cpu.engine `{name}`; allowed values: `interpreter`, `jit`")
        );
    }
}

#[test]
fn jit_on_the_80286_is_refused_at_the_key() {
    let temp = machine_file::test_support::TempDir::new("cpu-engine-80286");
    let (path, text) = at_machine(&temp, "model = \"80286\"\nengine = \"jit\"");
    let error = machine_file::load_with_options(&path, unlisted()).unwrap_err();
    let machine_file::LoadError::Lower(error) = error else {
        panic!("expected a positioned validation error, got {error:?}");
    };
    assert_eq!(error.field, "cpu.engine");
    assert_eq!(error.origin.file, path);
    assert_eq!(error.origin.line, Some(line_of(&text, "engine")));
    assert!(
        error.message.contains(
            "cpu engine jit needs an 80386 or newer; the 80286 runs only on the interpreter"
        ),
        "{error}"
    );
}

/// A copy of the shipped SeaBIOS 486 machine file with `cpu` added to its
/// `[cpu]` table, or `None` when the SeaBIOS fixture is missing outside the
/// gate. Parents must be relative, so the copy names the fixtures absolutely.
fn seabios_486(
    temp: &machine_file::test_support::TempDir,
    name: &str,
    cpu: &str,
) -> Option<PathBuf> {
    let root = repository();
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
    let shipped = std::fs::read_to_string(root.join("machines/generic-486-seabios.toml")).unwrap();
    let vendor = format!("{}/", root.join("vendor").to_str().unwrap());
    let text = shipped
        .replace("\"../vendor/", &format!("\"{vendor}"))
        .replacen("[cpu]\n", &format!("[cpu]\n{cpu}\n"), 1);
    let path = temp.path().join(name);
    std::fs::write(&path, text).unwrap();
    Some(path)
}

#[test]
fn jit_lowers_on_a_32_bit_board_and_a_child_can_switch_back() {
    let temp = machine_file::test_support::TempDir::new("cpu-engine-486");
    let Some(jit) = seabios_486(&temp, "jit.toml", "engine = \"jit\"") else {
        return;
    };
    let config = machine_file::load(&jit).unwrap();
    assert_eq!(config.cpu.engine, CpuEngine::Jit);
    let child = temp.path().join("child.toml");
    std::fs::write(
        &child,
        "schema = 1\nextends = \"jit.toml\"\n[cpu]\nengine = \"interpreter\"\n",
    )
    .unwrap();
    assert_eq!(
        machine_file::load(&child).unwrap().cpu.engine,
        CpuEngine::Interpreter
    );
    let grandchild = temp.path().join("grandchild.toml");
    std::fs::write(&grandchild, "schema = 1\nextends = \"jit.toml\"\n").unwrap();
    assert_eq!(
        machine_file::load(&grandchild).unwrap().cpu.engine,
        CpuEngine::Jit,
        "an absent key inherits the parent's engine"
    );
}

/// What one run observes: per checkpoint the outcome, the retired count and
/// a few registers, then the serial and debug-console bytes and the text screen.
#[derive(Debug, PartialEq, Eq)]
struct Observed {
    checkpoints: Vec<(machine::RunOutcome, u64, u64, u64, u64)>,
    serial: Vec<u8>,
    debug: Vec<u8>,
    screen: Vec<String>,
}

/// The guest-visible results, and the compiled blocks the JIT published (host-only).
fn diskless_post(path: &Path, engine: CpuEngine) -> (Observed, u64) {
    let mut config = machine_file::load(path).unwrap();
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
    let mut assembled = assembly::assemble_mounted_with_jit(
        &config,
        &policy,
        Default::default(),
        std::rc::Rc::new(machine_file::FileMount::new()),
        Some(cranelift()),
    )
    .unwrap();
    assert_eq!(assembled.machine.cpu().engine(), engine);
    // SeaBIOS reaches "No bootable device." and halts within 24M steps at
    // the board's 33 MHz rate; 2M-step checkpoints split POST, the 32-bit
    // firmware set-up and the end.
    let mut checkpoints = Vec::new();
    for slice in 1..=12 {
        let outcome = assembled
            .machine
            .run_bounded(Some(slice * 2_000_000))
            .unwrap();
        let cpu = assembled.machine.cpu();
        checkpoints.push((
            outcome,
            cpu.retired(),
            cpu.reg(CpuReg::Rip),
            cpu.reg(CpuReg::Rsp),
            cpu.reg(CpuReg::Rax),
        ));
        if outcome == machine::RunOutcome::Halted {
            break;
        }
    }
    let serial = assembled
        .observation
        .drain_bytes(DeviceId::Uart, device::ByteStreamKind::SerialTx)
        .unwrap();
    let debug = assembled
        .observation
        .drain_bytes(DeviceId::DebugConsole, device::ByteStreamKind::DebugConsole)
        .unwrap();
    let video::Frame::Text(grid) = assembled.capture_text(DeviceId::Vga).unwrap().frame else {
        panic!("POST must leave a text screen");
    };
    let compiled = assembled.machine.cpu().published_compiled_blocks();
    (
        Observed {
            checkpoints,
            serial,
            debug,
            screen: grid.rows_as_strings(),
        },
        compiled,
    )
}

/// Shows the JIT produced compiled blocks for SeaBIOS's flat 32-bit code on
/// its background thread and that the retired stream is unchanged: equal
/// retired counts and registers at every checkpoint and equal serial, debug
/// and screen output. It does not prove compiled code ran: the core exposes
/// no count of compiled executions outside its test hooks.
#[test]
fn both_engines_retire_the_same_seabios_post() {
    let temp = machine_file::test_support::TempDir::new("cpu-engine-equal");
    let Some(path) = seabios_486(&temp, "machine.toml", "") else {
        return;
    };
    let (interpreted, none) = diskless_post(&path, CpuEngine::Interpreter);
    let (compiled, blocks) = diskless_post(&path, CpuEngine::Jit);
    eprintln!("compiled blocks published: interpreter {none}, jit {blocks}");
    assert_eq!(none, 0);
    // SeaBIOS runs its 32-bit set-up long enough for hot blocks to compile
    // and be published while the run goes on.
    assert!(blocks > 0, "the JIT published no compiled block");
    eprintln!("checkpoints (outcome, retired, rip, rsp, rax):");
    for (left, right) in interpreted.checkpoints.iter().zip(&compiled.checkpoints) {
        eprintln!("  interpreter {left:?}  jit {right:?}");
    }
    eprintln!(
        "serial {} bytes, debug {} bytes",
        interpreted.serial.len(),
        interpreted.debug.len()
    );
    assert!(
        interpreted
            .screen
            .iter()
            .any(|line| line.starts_with("No bootable device.")),
        "{:?}",
        interpreted.screen
    );
    assert!(String::from_utf8_lossy(&interpreted.debug).contains("SeaBIOS (version 1.17.0"));
    assert_eq!(compiled, interpreted);
}

/// Without a supplied backend the JIT is an error at the key, never a quiet
/// fall-back to the interpreter.
#[test]
fn jit_without_a_supplied_backend_is_refused_at_the_key() {
    let temp = machine_file::test_support::TempDir::new("cpu-engine-no-backend");
    let Some(path) = seabios_486(&temp, "jit.toml", "engine = \"jit\"") else {
        return;
    };
    let config = machine_file::load(&path).unwrap();
    let expected = "cpu engine jit: this build has no JIT backend; the embedder must supply one";
    let Err(error) = machine_file::assemble(&config) else {
        panic!("the JIT assembled without a backend");
    };
    assert_eq!(error.to_string(), expected);
    let mut loaded = machine_file::load_for_assembly(&path, Default::default()).unwrap();
    let Err(error) = loaded.assemble(&Default::default()) else {
        panic!("the JIT assembled without a backend");
    };
    assert_eq!(error.field, "cpu.engine");
    assert_eq!(error.origin.file, path);
    assert_eq!(error.message, expected);
    assert!(loaded.check().is_err());
    loaded.jit_backend = Some(cranelift());
    loaded.check().unwrap();
    let assembled = loaded.assemble(&Default::default()).unwrap();
    assert_eq!(assembled.machine.cpu().engine(), CpuEngine::Jit);
}

/// A real-mode stub enters flat 32-bit protected mode (Intel 80386
/// Programmer's Reference Manual, §10.3 Switching to Protected Mode) and runs
/// a loop summing 10_000_000..1 into EAX before HLT: hot enough for the JIT
/// to compile and publish the loop body while it runs.
fn summing_loop(jit: Option<JitBackendFactory>) -> X86JitCpu {
    let mut cpu = X86JitCpu::new_i386_bare_with_fpu(
        0,
        0x1000,
        cpu::x86jit_backend::RealSegments::flat(0),
        false,
        jit,
    )
    .unwrap();
    cpu.map_ram(0, 0x10000).unwrap();
    let real_mode = [
        0xFA, // cli
        0x0F, 0x01, 0x16, 0x00, 0x11, // lgdt [1100h]
        0x0F, 0x20, 0xC0, // mov eax, cr0
        0x0C, 0x01, // or al, 1
        0x0F, 0x22, 0xC0, // mov cr0, eax
        0x66, 0xEA, 0x00, 0x12, 0x00, 0x00, 0x08, 0x00, // jmp dword 0008:00001200
    ];
    let gdtr = [0x17, 0x00, 0x08, 0x11, 0x00, 0x00];
    let gdt = [
        [0; 8],
        [0xFF, 0xFF, 0, 0, 0, 0x9A, 0xCF, 0], // flat 32-bit code
        [0xFF, 0xFF, 0, 0, 0, 0x92, 0xCF, 0], // flat data
    ];
    let protected = [
        0x66, 0xB8, 0x10, 0x00, // mov ax, 10h
        0x8E, 0xD8, // mov ds, ax
        0x8E, 0xD0, // mov ss, ax
        0x8E, 0xC0, // mov es, ax
        0x8E, 0xE0, // mov fs, ax
        0x8E, 0xE8, // mov gs, ax
        0xB9, 0x80, 0x96, 0x98, 0x00, // mov ecx, 10000000
        0x31, 0xC0, // xor eax, eax
        0x01, 0xC8, // add eax, ecx
        0x49, // dec ecx
        0x75, 0xFB, // jnz add
        0xF4, // hlt
    ];
    cpu.write_mem(0x1000, &real_mode).unwrap();
    cpu.write_mem(0x1100, &gdtr).unwrap();
    cpu.write_mem(0x1108, gdt.as_flattened()).unwrap();
    cpu.write_mem(0x1200, &protected).unwrap();
    cpu
}

/// Run in fixed slices to the halt, recording what each slice completed,
/// the retired count and the registers after it.
fn slices(mut cpu: X86JitCpu) -> (Vec<(u64, u64, u64, u64)>, u64) {
    let mut seen = Vec::new();
    loop {
        let run = cpu.run_steps(4099, StepRunOptions::default()).unwrap();
        seen.push((
            run.steps,
            cpu.retired(),
            cpu.reg(CpuReg::Rax),
            cpu.reg(CpuReg::Rcx),
        ));
        if run.stop == CpuStepStop::Exit(CpuStop::Halt) {
            return (seen, cpu.published_compiled_blocks());
        }
    }
}

/// Shows the JIT produced compiled blocks and the retired stream is
/// unchanged; it does not prove compiled code ran.
#[test]
fn jit_and_interpreter_retire_the_same_stream() {
    assert_eq!(summing_loop(None).engine(), CpuEngine::Interpreter);
    assert_eq!(summing_loop(Some(cranelift())).engine(), CpuEngine::Jit);
    let (interpreted, none) = slices(summing_loop(None));
    let (compiled, blocks) = slices(summing_loop(Some(cranelift())));
    let last = *interpreted.last().unwrap();
    // 6 real-mode and 8 set-up instructions, 3 per iteration, then the halt.
    assert_eq!(last.1, 6 + 8 + 3 * 10_000_000 + 1);
    let sum = (1..=10_000_000u64).sum::<u64>() & 0xFFFF_FFFF;
    assert_eq!(last.2 & 0xFFFF_FFFF, sum);
    assert_eq!(none, 0);
    eprintln!("compiled blocks published: interpreter {none}, jit {blocks}");
    assert!(blocks > 0, "the JIT published no compiled block");
    assert_eq!(compiled, interpreted);
}

/// The engine is a local run choice, not hardware: a stored config of a
/// machine file that says jit has no engine key, so it keeps the identity of
/// the same hardware without one (its encoded table is byte-identical).
#[test]
fn stored_config_never_carries_the_engine() {
    let base = "schema = 1\n[board]\nmodel = \"generic-486-isa\"\n[cpu]\nmodel = \"80486dx\"\n";
    let encode = |text: &str| {
        let document = machine_file::parse_toml("machine.toml", text).unwrap();
        pc_config::image::encode::machine(&document)
    };
    let plain = encode(base);
    for engine in ["jit", "interpreter"] {
        let stored = encode(&format!("{base}engine = \"{engine}\"\n"));
        let cpu = stored["cpu"].as_table().unwrap();
        assert!(!cpu.contains_key("engine"), "{engine}: {cpu:?}");
        assert_eq!(stored, plain, "{engine}");
        assert_eq!(
            toml::to_string(&stored).unwrap(),
            toml::to_string(&plain).unwrap()
        );
    }
    assert_eq!(
        pc_config::compat::lookup(&[pc_config::compat::MACHINE_KEYS], "machine.cpu.engine"),
        None
    );
}
