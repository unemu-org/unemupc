// SPDX-License-Identifier: AGPL-3.0-or-later
//! The bootable speed-ceiling floppies from `tools/bench-floppies/`: building
//! them, booting one on the generic 486 ISA board with SeaBIOS from A:, and
//! the facts each case must show. Shared by the `bench_floppy` timing probe
//! and the `bench_floppies` test.
//!
//! The regime checks follow the Intel 80386 Programmer's Reference Manual:
//! §5.1/§5.2 (segment and page translation), §6.3 (privilege levels), §15
//! (virtual-8086 mode and IOPL). See `backlog/docs/design/bench-floppies.md`.

use assembly::{DeviceId, ImageSource, ObservationPolicy, ObservationRequest, OutputKind};
use cpu::{Cpu, CpuReg};
use std::path::{Path, PathBuf};

/// Linear address of the payload; see `tools/bench-floppies/common.inc`.
pub const LOAD: u32 = 0x10000;
/// Payload bytes the boot sector loads: 32 sectors after the boot sector.
pub const PAYLOAD_BYTES: usize = 32 * 512;
/// A 3.5-inch 1.44 MB floppy: 80 cylinders, 2 heads, 18 sectors of 512 bytes.
pub const IMAGE_BYTES: usize = 1_474_560;

/// One case `build.sh` writes as `<name>.img`.
#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub name: &'static str,
    /// The ROM fixture under `tests/fixtures/` that holds the same loop.
    pub rom: &'static str,
    /// The pass count `build.sh` uses unless `BENCH_PASSES` replaces it.
    pub passes: u64,
}

/// Every case, in `build.sh` order.
pub const CASES: [Case; 10] = [
    case("real16", "cpu-ceiling-real.bin", 7500),
    case("protected32", "cpu-ceiling-protected32.bin", 5400),
    case("paged32", "cpu-ceiling-paged32.bin", 4000),
    case("flat32", "cpu-ceiling-flat32.bin", 6000),
    case("flat-paged32", "cpu-ceiling-flat-paged32.bin", 4200),
    case(
        "flat-paged32-cpl3",
        "cpu-ceiling-flat-paged32-cpl3.bin",
        26_000_000,
    ),
    case(
        "flat-paged32-cpl3-farcall",
        "cpu-ceiling-flat-paged32-cpl3-farcall.bin",
        24_000_000,
    ),
    case(
        "paged-protected16-cpl3",
        "cpu-ceiling-paged-protected16-cpl3.bin",
        22_000_000,
    ),
    case("paged-v86", "cpu-ceiling-paged-v86.bin", 2_600_000),
    case(
        "paged-v86-iopl0",
        "cpu-ceiling-paged-v86-iopl0.bin",
        2_200_000,
    ),
];

const fn case(name: &'static str, rom: &'static str, passes: u64) -> Case {
    Case { name, rom, passes }
}

impl Case {
    /// Whether the case comes from `ceiling.asm` (the five-instruction ring-0
    /// loop) rather than `ceiling-user.asm`.
    pub fn register_loop(self) -> bool {
        is_register_loop(self.name)
    }

    /// The count the START and END lines print for `passes`.
    pub fn count(self, passes: u64) -> u64 {
        if self.register_loop() {
            passes * 65536
        } else {
            passes
        }
    }
}

fn is_register_loop(name: &str) -> bool {
    matches!(
        name,
        "real16" | "protected32" | "paged32" | "flat32" | "flat-paged32"
    )
}

pub fn find(name: &str) -> Case {
    CASES
        .into_iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("unknown case {name}"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run `build.sh` into `out`, with every pass count replaced by `passes` when
/// given. `None` when nasm is missing and fixtures are not required.
pub fn build(out: &Path, passes: Option<u64>) -> Option<()> {
    match std::process::Command::new("nasm").arg("-v").output() {
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none() =>
        {
            eprintln!("skipping: nasm is missing");
            return None;
        }
        other => {
            other.unwrap();
        }
    }
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/bench-floppies/build.sh");
    let mut command = std::process::Command::new("bash");
    command.arg(script).arg(out);
    match passes {
        Some(passes) => command.env("BENCH_PASSES", passes.to_string()),
        // The published counts, whatever the caller's environment says.
        None => command.env_remove("BENCH_PASSES"),
    };
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(())
}

/// The generic 486 ISA board with SeaBIOS, the floppy controller and `image`
/// in A:, and no hard disk. `None` when the SeaBIOS or FreeDOS fixtures are
/// missing and fixtures are not required; the FreeDOS machine file supplies
/// the floppy drive, whose image is replaced.
pub fn config(image: Vec<u8>) -> Option<assembly::MachineConfig> {
    let root = root();
    for name in [
        "seabios/bios-1.17.0-isa.bin",
        "seabios/build/vgabios-1.17.0-stdvga.bin",
        "freedos/freedos.img",
    ] {
        if !root.join("vendor").join(name).exists() {
            assert!(
                std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none(),
                "required fixture is missing: {name}"
            );
            eprintln!("skipping: fixture is missing: {name}");
            return None;
        }
    }
    let mut config = machine_file::load_for_assembly(
        root.join("machines/generic-486-freedos.toml"),
        Default::default(),
    )
    .unwrap()
    .config;
    config.drives[0].image = ImageSource::Inline(image);
    config.drives[0].expected_sha256 = None;
    Some(config)
}

/// Assemble with the VGA text screen observable.
pub fn assemble(config: &assembly::MachineConfig) -> assembly::AssembledMachine {
    let policy = ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::Vga,
            output: OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    };
    machine_file::assemble_with_observation(config, &policy).unwrap()
}

/// Virtual-time steps in one millisecond at the board's nominal CPU rate.
pub fn steps_per_ms(config: &assembly::MachineConfig) -> u64 {
    let pit = device::clock::PIT_HZ;
    let numerator = u128::from(config.clock.retired_per_pit_clock) * u128::from(pit.numerator());
    let denominator = u128::from(config.clock.retired_per_pit_clock_denominator)
        * u128::from(pit.denominator())
        * 1000;
    (numerator / denominator) as u64
}

pub fn text(assembled: &assembly::AssembledMachine) -> String {
    match assembled.capture_text(DeviceId::Vga).unwrap().frame {
        video::Frame::Text(grid) => grid.rows_as_strings().join("\n"),
        _ => String::new(),
    }
}

/// The `UNEMU-BENCH <case> <marker> count=<n>` line, as (case, count).
pub fn marker(screen: &str, marker: &str) -> Option<(String, u64)> {
    screen.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == "UNEMU-BENCH").then_some(())?;
        let case = words.next()?.to_owned();
        (words.next()? == marker).then_some(())?;
        let count = words.next()?.strip_prefix("count=")?.parse().ok()?;
        // The START line ends with the A20 state; until that is printed in
        // full the count may still be growing digit by digit.
        if marker == "START" && !matches!(words.next()?, "A20=on" | "A20=off") {
            return None;
        }
        Some((case, count))
    })
}

/// Fail when the CPU sits in a halt it cannot leave. With interrupts off a run
/// returns at once, so a loop waiting for progress would otherwise never end.
pub fn assert_running(assembled: &assembly::AssembledMachine, what: &str) {
    let cpu = assembled.machine.cpu();
    assert!(
        !(assembled.machine.halted() && !cpu.interrupts_enabled()),
        "halted with interrupts off {what}, after {} retired instructions\n{}\n{}",
        cpu.retired(),
        text(assembled),
        cpu.diagnostic_state()
    );
}

/// Run in `chunk`-step slices until the START line is complete; (case, count).
pub fn boot_to_start(assembled: &mut assembly::AssembledMachine, chunk: u64) -> (String, u64) {
    // SeaBIOS POST and the floppy read take about 11 million steps.
    while assembled.machine.virtual_time() < 100_000_000 {
        let cap = assembled.machine.virtual_time() + chunk;
        run(assembled, cap);
        if let Some(found) = marker(&text(assembled), "START") {
            let screen = text(assembled);
            assert!(screen.contains(" A20=on"), "{screen}");
            assert!(assembled.machine.cpu().diagnostic_state().a20);
            return found;
        }
        assert_running(assembled, "before the START line");
    }
    panic!("no START line\n{}", text(assembled));
}

/// Run to the absolute virtual time `cap`; the guest must not reset.
pub fn run(assembled: &mut assembly::AssembledMachine, cap: u64) {
    match assembled.machine.run_bounded(Some(cap)).unwrap() {
        machine::RunOutcome::RetiredCapReached | machine::RunOutcome::Halted => {}
        machine::RunOutcome::ResetRequested => panic!("the guest reset the machine"),
    }
}

/// Linear address of CS:EIP. The diagnostic's own CS:IP is 16 bits wide.
pub fn code_linear(cpu: &cpu::X86JitCpu) -> u32 {
    cpu.diagnostic_state().segments[1]
        .base
        .wrapping_add(cpu.reg(CpuReg::Rip) as u32)
}

/// Linear address of the first loop instruction.
pub fn loop_entry(case: &str) -> u32 {
    if is_register_loop(case) {
        LOAD + 0x1000
    } else {
        LOAD + 0x2000
    }
}

/// Whether the loop counters hold their starting values, so that reaching
/// [`loop_entry`] is the first pass and not a later one.
pub fn counters_at_start(case: &str, passes: u64, cpu: &cpu::X86JitCpu) -> bool {
    if is_register_loop(case) {
        cpu.reg(CpuReg::Rcx) & 0xffff == 0 && cpu.reg(CpuReg::Rsi) & 0xffff_ffff == passes
    } else {
        cpu.reg(CpuReg::Rbp) == 0
    }
}

/// The first bytes after the loop: UD2 for the user cases; for the register
/// loop the `mov ax, imm16` that starts the END store (operand-size prefixed
/// in the 32-bit code segments).
pub fn exit_bytes(case: &str) -> &'static [u8] {
    match case {
        "real16" => &[0xb8],
        _ if is_register_loop(case) => &[0x66, 0xb8],
        _ => &[0x0f, 0x0b],
    }
}

/// Retired instructions of the timed loop alone, from the loop sources.
pub fn loop_retired(case: &str, count: u64) -> u64 {
    match case {
        // INC, ADD, XOR, DEC, JNZ per iteration; DEC ESI, JNZ per 65536.
        _ if is_register_loop(case) => count * 5 + count / 65536 * 2,
        // The ROM's 25-instruction period plus the counting CMP.
        "flat-paged32-cpl3" | "flat-paged32-cpl3-farcall" | "paged-protected16-cpl3" => count * 26,
        // Ten 24-instruction bodies, nine for the port read or its stand-in,
        // the counting CMP and the branch. The faulting IN does not retire.
        "paged-v86" | "paged-v86-iopl0" => count * 251,
        _ => panic!("unknown case {case}"),
    }
}

/// Whether the processor is where the case's loop should run; `Err` says why not.
pub fn regime(case: &str, cpu: &cpu::X86JitCpu) -> Result<(), String> {
    let state = cpu.diagnostic_state();
    let system = state.system.unwrap();
    let [es, cs, ss, ds] = state.segments;
    let code = code_linear(cpu);
    let mut problems = Vec::new();
    let mut expect = |ok: bool, what: &str| {
        if !ok {
            problems.push(what.to_owned());
        }
    };
    let iopl = (state.flags >> 12) & 3;
    let flat =
        |segment: cpu::stop_report::SegmentView| segment.base == 0 && segment.limit == u32::MAX;
    if is_register_loop(case) {
        expect(
            (LOAD + 0x1000..LOAD + 0x100c).contains(&code),
            "EIP in the loop",
        );
        expect(system.protected() == (case != "real16"), "PE");
        expect(system.paging() == case.ends_with("paged32"), "PG");
        expect(system.cpl == 0 && !system.virtual_8086, "CPL 0, no VM");
        match case {
            "real16" => expect(cs.selector == 0x1000 && cs.base == LOAD, "real CS"),
            "flat32" | "flat-paged32" => {
                expect([cs, ss, ds, es].into_iter().all(flat), "flat segments");
                expect(cs.attributes & 0x4000 != 0, "32-bit CS");
            }
            _ => {
                expect(
                    cs.selector == 8 && cs.base == LOAD && cs.limit == 0xffff,
                    "64 KiB CS at the load address",
                );
                expect(cs.attributes & 0x4000 != 0, "32-bit CS");
                expect(case != "paged32" || flat(ds), "flat DS");
            }
        }
    } else {
        let v86 = case.starts_with("paged-v86");
        expect(
            (LOAD + 0x1000..LOAD + 0x3000).contains(&code),
            "EIP in the loop or callee",
        );
        expect(system.protected() && system.paging(), "PE and PG");
        expect(system.cpl == 3, "CPL 3");
        expect(system.virtual_8086 == v86, "VM");
        expect(iopl == if case == "paged-v86" { 3 } else { 0 }, "IOPL");
        match case {
            "flat-paged32-cpl3" | "flat-paged32-cpl3-farcall" => {
                expect(cs.selector == 0x1b && flat(cs), "flat ring-3 CS");
                expect(
                    ds.selector == 0x23 && flat(ds) && flat(ss),
                    "flat ring-3 data",
                );
            }
            "paged-protected16-cpl3" => {
                expect(
                    matches!(cs.selector, 0x0f | 0x17) && cs.limit == 0xffff,
                    "16-bit LDT code segment",
                );
                expect(ss.selector == 0x1f && ss.base == 0x40000, "LDT stack");
                expect(system.ldtr.selector == 0x28, "LDTR");
            }
            _ => {
                expect(matches!(cs.selector, 0x1100 | 0x1200), "V86 CS");
                expect(ss.selector == 0x4000 && ss.base == 0x40000, "V86 SS");
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{}: {}", problems.join(", "), state))
    }
}

/// Registers and memory the loop leaves behind, read at the final halt.
pub fn end_state(case: &str, count: u64, cpu: &cpu::X86JitCpu) {
    if is_register_loop(case) {
        assert_eq!(cpu.reg(CpuReg::Rcx) & 0xffff, 0, "{case}: inner counter");
        return;
    }
    let v86 = case.starts_with("paged-v86");
    let bodies = count * if v86 { 10 } else { 1 };
    assert_eq!(cpu.reg(CpuReg::Rbp), bodies, "{case}: EBP counts bodies");
    let mut word = [0; 4];
    cpu.read_mem(0x40100, &mut word).unwrap();
    let mask = if case.starts_with("flat") {
        u32::MAX
    } else {
        0xffff
    };
    assert_eq!(
        u32::from_le_bytes(word) & mask,
        (bodies as u32).wrapping_mul(3) & mask,
        "{case}: user data"
    );
    cpu.read_mem(0x6000, &mut word).unwrap();
    assert_eq!(
        u64::from(u32::from_le_bytes(word)),
        if case == "paged-v86-iopl0" { count } else { 0 },
        "{case}: monitor traps"
    );
}
