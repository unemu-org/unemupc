// SPDX-License-Identifier: AGPL-3.0-or-later
//! A Bochs VBE card on the generic 486 board (Bochs VBE DISPI interface,
//! `vbe.h`): a project-owned program detects the interface, sets 640×480 at
//! 32 and 16 bpp, fills the linear framebuffer from protected mode, and the
//! captured frame shows every pixel. See machine-files.md §Display cards.

use std::rc::Rc;

use assembly::{
    AssemblyOptions, BoardModel, BochsVbeCfg, CpuEngine, DeviceConfig, DeviceId, DeviceKind,
    JitBackendFactory, MachineConfig, MmioWindow, ObservationPolicy, ObservationRequest,
    OutputKind,
};
use cpu::Cpu;
use machine::RunOutcome;
use video::{Frame, PixelFormat};

#[path = "support/speed.rs"]
mod speed;

const CPU_REV: &str = "dd4316586f1459cec0618008b17817a0d26245a2";
const LFB32: &[u8] = include_bytes!("fixtures/bochs-vbe-lfb32.bin");
const LFB16: &[u8] = include_bytes!("fixtures/bochs-vbe-lfb16.bin");
const SPEED_RAM: &[u8] = include_bytes!("fixtures/bochs-vbe-speed-ram.bin");
const SPEED_LFB: &[u8] = include_bytes!("fixtures/bochs-vbe-speed-lfb.bin");
const SPEED_BANK: &[u8] = include_bytes!("fixtures/bochs-vbe-speed-bank.bin");
const WINDOW: &[u8] = include_bytes!("fixtures/bochs-vbe-window.bin");

fn cranelift() -> JitBackendFactory {
    JitBackendFactory(|| Box::new(x86jit_cranelift::JitBackend::new()))
}

fn config(rom: &[u8], engine: CpuEngine) -> MachineConfig {
    let mut config = speed::board(rom, BoardModel::Generic486Isa);
    config.cpu.engine = engine;
    config.devices.push(DeviceConfig::new(
        DeviceId::Vga,
        DeviceKind::BochsVbe(BochsVbeCfg { memory_mib: 16 }),
        BochsVbeCfg::port_windows(),
    ));
    config
        .memory
        .mmio
        .push(MmioWindow::new(0xA0000, 0x20000, DeviceId::Vga));
    config
}

fn rig(rom: &[u8], engine: CpuEngine) -> assembly::AssembledMachine {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let policy = ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::Vga,
            output: OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    };
    assembly::assemble_mounted_with_jit(
        &config(rom, engine),
        &policy,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
        Rc::new(assembly::InlineMount),
        Some(cranelift()),
    )
    .unwrap()
}

fn word(rig: &assembly::AssembledMachine, address: u64) -> u16 {
    let mut bytes = [0; 2];
    rig.machine.cpu().read_mem(address, &mut bytes).unwrap();
    u16::from_le_bytes(bytes)
}

fn filled_frame(rom: &[u8], engine: CpuEngine) -> (video::DirectPixels, u64) {
    let mut rig = rig(rom, engine);
    let outcome = loop {
        let cap = rig.machine.virtual_time() + 100_000;
        assert!(cap < 5_000_000, "the program must halt");
        match rig.machine.run_bounded(Some(cap)).unwrap() {
            RunOutcome::RetiredCapReached => {}
            outcome => break outcome,
        }
    };
    assert_eq!(outcome, RunOutcome::Halted);
    assert_eq!(word(&rig, 0x500), 0xB0C0, "ID handshake");
    assert_eq!(word(&rig, 0x502), 256, "VIDEO_MEMORY_64K for 16 MiB");
    let Frame::Direct(frame) = rig.capture_text(DeviceId::Vga).unwrap().frame else {
        panic!("a DISPI mode presents a direct-colour frame");
    };
    (frame, rig.machine.cpu().retired())
}

#[test]
fn linear_framebuffer_at_32_bpp_shows_every_guest_pixel() {
    for engine in [CpuEngine::Interpreter, CpuEngine::Jit] {
        let (frame, retired) = filled_frame(LFB32, engine);
        assert_eq!(
            (frame.width, frame.height, frame.format, frame.stride),
            (640, 480, PixelFormat::Xrgb8888, 2560)
        );
        let expected: Vec<u32> = (0..640 * 480).map(|n: u32| n & 0xFF_FFFF).collect();
        assert!(frame.to_xrgb8888().unwrap() == expected, "{engine:?}");
        eprintln!("{engine:?}: retired {retired}");
    }
}

#[test]
fn linear_framebuffer_at_16_bpp_shows_every_guest_pixel() {
    let expand5 = |v: u32| (v << 3) | (v >> 2);
    let expand6 = |v: u32| (v << 2) | (v >> 4);
    let (frame, _) = filled_frame(LFB16, CpuEngine::Interpreter);
    assert_eq!(
        (frame.width, frame.height, frame.format, frame.stride),
        (640, 480, PixelFormat::Rgb565, 1280)
    );
    let expected: Vec<u32> = (0..640 * 480)
        .map(|n: u32| {
            let n = n & 0xFFFF;
            expand5(n >> 11) << 16 | expand6((n >> 5) & 63) << 8 | expand5(n & 31)
        })
        .collect();
    assert!(frame.to_xrgb8888().unwrap() == expected);
}

/// What the window program left: its result bytes, the frame and the served
/// port and MMIO accesses.
fn window_run(engine: CpuEngine, slice: Option<u64>) -> (Vec<u8>, Frame, u64, u64) {
    let mut rig = rig(WINDOW, engine);
    let outcome = loop {
        let cap = slice.map(|slice| rig.machine.virtual_time() + slice);
        assert!(
            rig.machine.virtual_time() < 5_000_000,
            "the program must halt"
        );
        match rig.machine.run_bounded(cap.or(Some(5_000_000))).unwrap() {
            RunOutcome::RetiredCapReached if slice.is_some() => {}
            outcome => break outcome,
        }
    };
    assert_eq!(outcome, RunOutcome::Halted);
    let mut results = vec![0; 16];
    rig.machine.cpu().read_mem(0x600, &mut results).unwrap();
    let frame = rig.capture_text(DeviceId::Vga).unwrap().frame;
    let served = rig.machine.served_accesses();
    (results, frame, served.ports, served.mmio)
}

/// The banked window and the framebuffer are one video memory: each view
/// sees the other's stores at once, whatever the order, and the result does
/// not depend on where the host ends its runs.
#[test]
fn banked_window_and_framebuffer_share_one_memory_under_any_run_slicing() {
    let expected = [
        0x5A, 0x00, 0x22, 0x11, 0x33, 0x44, 0x55, 0x55, 0x66, 0x00, 0, 0, 1, 2, 3, 4,
    ];
    let (results, frame, ports, mmio) = window_run(CpuEngine::Interpreter, None);
    assert_eq!(results, expected, "0x601: enabling cleared video memory");
    assert!(ports >= 3, "port 80h writes stay served: {ports}");
    assert!(mmio >= 9, "all nine window accesses are served: {mmio}");
    let Frame::Pixels(pixels) = &frame else {
        panic!("8 bpp presents an indexed frame");
    };
    assert_eq!((pixels.width, pixels.height), (640, 480));
    for slice in [Some(1), Some(7), Some(100_000)] {
        let (sliced, sliced_frame, _, _) = window_run(CpuEngine::Interpreter, slice);
        assert_eq!(sliced, expected, "slice {slice:?}");
        assert!(sliced_frame == frame, "slice {slice:?}");
    }
    let (jit, jit_frame, _, _) = window_run(CpuEngine::Jit, None);
    assert_eq!(jit, expected);
    assert!(jit_frame == frame);
}

#[test]
fn framebuffer_programs_rebuild_identically() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let temp = machine_file::test_support::TempDir::new("bochs-vbe-fixtures");
    for (name, defines) in [
        ("bochs-vbe-lfb32", &[][..]),
        ("bochs-vbe-lfb16", &["-DBPP16"][..]),
        ("bochs-vbe-speed-ram", &["-DSPEED=0x200000"][..]),
        ("bochs-vbe-speed-lfb", &["-DSPEED=0xe0000000"][..]),
        ("bochs-vbe-speed-bank", &["-DSPEED=0xa0000", "-DBANKED"][..]),
        ("bochs-vbe-window", &[][..]),
    ] {
        let binary = temp.path().join(format!("{name}.bin"));
        let output = std::process::Command::new("nasm")
            .args(["-f", "bin"])
            .args(defines)
            .arg(root.join(if name == "bochs-vbe-window" {
                "bochs-vbe-window.asm"
            } else {
                "bochs-vbe-lfb.asm"
            }))
            .arg("-o")
            .arg(&binary)
            .output();
        let output = match output {
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none() =>
            {
                eprintln!("skipping: nasm is missing");
                return;
            }
            other => other.unwrap(),
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read(&binary).unwrap(),
            std::fs::read(root.join(format!("{name}.bin"))).unwrap(),
            "{name}"
        );
    }
}

/// Wall time for `steps` retired instructions of a store loop, after a
/// warm-up, best of three.
fn store_loop_rate(rom: &[u8], engine: CpuEngine, steps: u64) -> f64 {
    (0..3)
        .map(|_| {
            let mut rig = rig(rom, engine);
            assert_eq!(
                rig.machine.run_bounded(Some(1_000_000)).unwrap(),
                RunOutcome::RetiredCapReached
            );
            let before = rig.machine.cpu().retired();
            let started = std::time::Instant::now();
            assert_eq!(
                rig.machine.run_bounded(Some(1_000_000 + steps)).unwrap(),
                RunOutcome::RetiredCapReached
            );
            let elapsed = started.elapsed().as_secs_f64();
            assert_eq!(rig.machine.cpu().retired() - before, steps);
            steps as f64 / elapsed / 1e6
        })
        .fold(0.0, f64::max)
}

/// Host speed, so ignored by default: framebuffer stores take the CPU's RAM
/// path, so a store loop retires as fast over the framebuffer as over system
/// RAM. Also reports the cost of presenting one 1024×768×32 frame.
#[test]
#[ignore = "wall-clock measurement; run by name"]
fn framebuffer_stores_retire_as_fast_as_ram_stores() {
    for engine in [CpuEngine::Interpreter, CpuEngine::Jit] {
        let steps = 20_000_000;
        let ram = store_loop_rate(SPEED_RAM, engine, steps);
        let lfb = store_loop_rate(SPEED_LFB, engine, steps);
        // Every window store is a device access, as on a banked card.
        let bank = store_loop_rate(SPEED_BANK, engine, steps / 10);
        eprintln!(
            "{engine:?}: RAM {ram:.1} M/s, framebuffer {lfb:.1} M/s, banked window {bank:.1} M/s"
        );
        assert!(lfb > ram * 0.8, "{engine:?}: {lfb:.1} vs {ram:.1} M/s");
    }

    let mut rig = rig(LFB32, CpuEngine::Interpreter);
    let bus = rig.machine.bus_mut();
    for (index, value) in [(4, 0), (3, 32), (1, 1024), (2, 768), (4, 0xC1)] {
        bus.port_write(0x1CE, 2, index).unwrap();
        bus.port_write(0x1CF, 2, value).unwrap();
    }
    let started = std::time::Instant::now();
    let frames = 100;
    for _ in 0..frames {
        let Frame::Direct(frame) = rig.capture_text(DeviceId::Vga).unwrap().frame else {
            panic!("direct-colour frame");
        };
        assert_eq!((frame.width, frame.height), (1024, 768));
    }
    let per_frame = started.elapsed().as_secs_f64() / f64::from(frames) * 1e3;
    eprintln!("1024x768x32 scanout: {per_frame:.2} ms per frame");
}
