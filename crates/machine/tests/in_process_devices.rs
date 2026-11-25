// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serving port 80h and the display window while the CPU runs leaves the
//! machine exactly as returning from the CPU for every access does (§3.1;
//! `backlog/docs/design/scheduler.md` §In-process device access).
//!
//! A real-mode guest on the 80286 machine profile fills the window with REP
//! STOSW, writes and reads port 80h, writes a word to port 80h whose second
//! cycle reaches a DMA page register, programs a window event a few clocks
//! ahead (a deadline earlier than the run's budget) and raises the window's
//! interrupt from a store (a request raised by a served access), while the
//! 8254 interrupts it periodically through the 8259A (Intel 8259A datasheet,
//! Interrupt Sequence; Intel 8254 datasheet, Mode 2). Both ways must stop at
//! the same retired count and virtual time with the same RAM, device logs
//! (each access stamped with its device time) and controller acknowledgements.

use std::cell::Cell;
use std::num::NonZeroU64;
use std::rc::Rc;

use bus::{Bus, DecodeMap, MmioRange, PortRange, PortWidth};
use chipset::{PicObservation, SharedPic};
use cpu::x86jit_backend::{RealSegments, X86JitCpu};
use cpu::Cpu;
use device::{Device, DeviceError, NextEvent, VClock};
use devices::Pit8253;
use machine::{DmaWiring, IrqLine, Machine, MemoryMapSync, RunOutcome, ServedAccesses};

const WINDOW: u64 = 0xA0000;

/// Display-window memory with a control area: a byte written at FFF0h
/// schedules the interrupt that many clocks ahead, FFF1h raises it at once,
/// FFF2h lowers it. Every access is logged with the device's time.
#[derive(Default)]
struct Window {
    memory: Vec<u8>,
    now: VClock,
    due: Option<VClock>,
    level: bool,
    log: Vec<u64>,
}

impl Device for Window {
    fn name(&self) -> &'static str {
        "test-window"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
        Ok(0xFF)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
        Ok(())
    }
    fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let offset = (addr - WINDOW) as usize;
        let mut value = 0;
        for i in (0..usize::from(size)).rev() {
            value = value << 8 | u64::from(self.memory[offset + i]);
        }
        self.log
            .extend([self.now.0, addr, u64::from(size), value, 0]);
        Ok(value)
    }
    fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError> {
        let offset = (addr - WINDOW) as usize;
        for i in 0..usize::from(size) {
            self.memory[offset + i] = (value >> (8 * i)) as u8;
        }
        self.log
            .extend([self.now.0, addr, u64::from(size), value, 1]);
        match offset {
            0xFFF0 => self.due = Some(VClock(self.now.0 + (value & 0xFF).max(1))),
            0xFFF1 => self.level = true,
            0xFFF2 => self.level = false,
            _ => {}
        }
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        self.now = now;
        if self.due.is_some_and(|due| due <= now) {
            self.due = None;
            self.level = true;
        }
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        self.due.map_or(NextEvent::Dormant, NextEvent::At)
    }
    fn irq_pending(&self) -> bool {
        self.level
    }
    fn reset(&mut self) {}
    fn save(&self) -> Vec<u8> {
        let mut bytes = self.memory.clone();
        bytes.extend(self.log.iter().flat_map(|word| word.to_le_bytes()));
        bytes
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
        Ok(())
    }
}

/// A port device that logs every access with its time; reads return the
/// last byte written.
#[derive(Default)]
struct Logger {
    now: VClock,
    last: u32,
    log: Vec<u64>,
}

impl Device for Logger {
    fn name(&self) -> &'static str {
        "test-logger"
    }
    fn port_read(&mut self, port: u16, _: u8) -> Result<u32, DeviceError> {
        self.log.extend([self.now.0, u64::from(port), 0]);
        Ok(self.last)
    }
    fn port_write(&mut self, port: u16, _: u8, value: u32) -> Result<(), DeviceError> {
        self.log
            .extend([self.now.0, u64::from(port), u64::from(value) | 0x100]);
        self.last = value;
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
        Ok(0xFF)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), DeviceError> {
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        self.now = now;
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn dma_request_pending(&self) -> bool {
        false
    }
    fn reset(&mut self) {}
    fn save(&self) -> Vec<u8> {
        self.log
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect()
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
        Ok(())
    }
}

/// The guest. IVT: vector 8 (IRQ0) at 0:8000, vector 0Ah (IRQ2) at 0:8100,
/// vector 1 (single step) at 0:8200. With `trace` the loop runs with TF set,
/// so every instruction traps (Intel 80286 Programmer's Reference Manual,
/// §9.5, Single Step Interrupt).
fn program(trace: bool) -> Vec<(u32, Vec<u8>)> {
    let mut main = vec![
        0xFA, // CLI
        0xB8, 0x00, 0xA0, 0x8E, 0xC0, // MOV AX,A000h; MOV ES,AX
        0xB0, 0x13, 0xE6, 0x20, // ICW1: single, edge triggered
        0xB0, 0x08, 0xE6, 0x21, // ICW2: vectors 08h-0Fh
        0xB0, 0x01, 0xE6, 0x21, // ICW4
        0xB0, 0xFA, 0xE6, 0x21, // unmask IRQ0 and IRQ2
        0xB0, 0x34, 0xE6, 0x43, // 8254 counter 0, mode 2
        0xB0, 0xC8, 0xE6, 0x40, 0xB0, 0x00, 0xE6, 0x40, // divisor 200
        0xFB, // STI
    ];
    if trace {
        // PUSHF; POP AX; OR AH,1; PUSH AX; POPF
        main.extend([0x9C, 0x58, 0x80, 0xCC, 0x01, 0x50, 0x9D]);
    }
    main.extend([
        0xBB, 0x40, 0x00, // MOV BX,64
        // loop (7C26h):
        0x31, 0xFF, // XOR DI,DI
        0xB9, 0x20, 0x00, // MOV CX,32
        0x89, 0xD8, // MOV AX,BX
        0xF3, 0xAB, // REP STOSW into the window
        0x88, 0xD8, // MOV AL,BL
        0xE6, 0x80, // OUT 80h,AL
        0xE7, 0x80, // OUT 80h,AX: the second cycle reaches port 81h
        0xE4, 0x80, // IN AL,80h
        0x26, 0xA0, 0x10, 0x00, // MOV AL,ES:[0010h]
        0xB0, 0x03, // MOV AL,3
        0x26, 0xA2, 0xF0, 0xFF, // MOV ES:[FFF0h],AL: interrupt in 3 clocks
        0x90, 0x90, 0x90, 0x90, 0x90, // NOP x5
        0x26, 0xA2, 0xF1, 0xFF, // MOV ES:[FFF1h],AL: interrupt now
        0x90, // NOP
        0x4B, // DEC BX
        0x75, 0xD8, // JNZ loop
        0xFA, 0xF4, // CLI; HLT
    ]);
    let irq0 = vec![
        0x50, // PUSH AX
        0xFF, 0x06, 0x00, 0x05, // INC WORD [0500h]
        0xA0, 0x00, 0x05, // MOV AL,[0500h]
        0xE6, 0x80, // OUT 80h,AL
        0xB0, 0x20, 0xE6, 0x20, // EOI
        0x58, 0xCF, // POP AX; IRET
    ];
    let irq2 = vec![
        0x50, // PUSH AX
        0xFF, 0x06, 0x02, 0x05, // INC WORD [0502h]
        0x26, 0xA2, 0xF2, 0xFF, // MOV ES:[FFF2h],AL: lower the request
        0xB0, 0x20, 0xE6, 0x20, // EOI
        0x58, 0xCF, // POP AX; IRET
    ];
    let step = vec![
        0xFF, 0x06, 0x04, 0x05, // INC WORD [0504h]
        0xCF, // IRET
    ];
    vec![
        (0x7C00, main),
        (0x8000, irq0),
        (0x8100, irq2),
        (0x8200, step),
        (0x04, vec![0x00, 0x82, 0, 0]),
        (0x20, vec![0x00, 0x80, 0, 0]),
        (0x28, vec![0x00, 0x81, 0, 0]),
    ]
}

struct Rig {
    machine: Machine<X86JitCpu>,
    log: chipset::PicObservationLog,
}

/// Bus indices: PIC, PIT, window, port 80h, port 81h (a DMA page register,
/// the controller of an idle requester), the requester.
fn rig(in_process: bool) -> Rig {
    traced_rig(in_process, false)
}

fn traced_rig(in_process: bool, trace: bool) -> Rig {
    let mut cpu = X86JitCpu::new_i286_bare(0x100000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
    cpu.map_ram(0, WINDOW).unwrap();
    cpu.map_mmio(WINDOW, 0x20000).unwrap();
    for (addr, bytes) in program(trace) {
        cpu.write_mem(u64::from(addr), &bytes).unwrap();
    }
    let pic = SharedPic::new();
    let port = |ports: std::ops::RangeInclusive<u16>, device| PortRange {
        ports,
        device,
        width: PortWidth::Byte,
    };
    let bus = Bus::new(
        vec![
            Box::new(pic.clone()),
            Box::new(Pit8253::new()),
            Box::new(Window {
                memory: vec![0; 0x20000],
                ..Window::default()
            }),
            Box::new(Logger::default()),
            Box::new(Logger::default()),
            Box::new(Logger::default()),
        ],
        DecodeMap {
            ports: vec![
                port(0x20..=0x21, 0),
                port(0x40..=0x43, 1),
                port(0x80..=0x80, 3),
                port(0x81..=0x81, 4),
            ],
            mmio: vec![MmioRange {
                addrs: WINDOW..=0xBFFFF,
                device: 2,
            }],
        },
    );
    let log = pic.observe(bus.observation_clock());
    let lines = vec![
        IrqLine {
            device: 1,
            output: 0,
            irq: 0,
        },
        IrqLine {
            device: 2,
            output: 0,
            irq: 2,
        },
    ];
    let mut machine = Machine::new(cpu, bus, Box::new(pic.clone()), lines, None)
        .with_dma_wirings(vec![DmaWiring {
            requester: 5,
            controller: 4,
        }])
        .with_in_process_devices(in_process);
    machine.enable_code_trail();
    Rig { machine, log }
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    outcome: RunOutcome,
    retired: u64,
    steps: u64,
    virtual_time: u64,
    clock: VClock,
    ram: Vec<u8>,
    devices: Vec<Vec<u8>>,
    pic: Vec<PicObservation>,
}

fn run(mut rig: Rig, cap: u64) -> (Outcome, u64, ServedAccesses) {
    let outcome = rig.machine.run_bounded(Some(cap)).unwrap();
    let mut ram = vec![0; 0x10000];
    rig.machine.cpu().read_mem(0, &mut ram).unwrap();
    let bus = rig.machine.bus_mut();
    bus.settle();
    let devices = (2..6).map(|i| bus.device(i).unwrap().save()).collect();
    let slices = rig.machine.code_trail().unwrap().samples();
    let served = rig.machine.served_accesses();
    (
        Outcome {
            outcome,
            retired: rig.machine.cpu().retired(),
            steps: rig.machine.cpu().completed_steps(),
            virtual_time: rig.machine.virtual_time(),
            clock: rig.machine.clock(),
            ram,
            devices,
            pic: rig.log.take(),
        },
        slices,
        served,
    )
}

fn counters(outcome: &Outcome) -> [u16; 3] {
    [0x500, 0x502, 0x504].map(|at| u16::from_le_bytes([outcome.ram[at], outcome.ram[at + 1]]))
}

#[test]
fn serving_in_process_matches_returning_for_every_access() {
    let (exits, exit_slices, none) = run(rig(false), 1_000_000);
    let (served, served_slices, count) = run(rig(true), 1_000_000);
    assert_eq!(none, ServedAccesses::default());
    assert!(count.ports > 64 && count.mmio > 64 * 32, "{count:?}");
    assert_eq!(exits.outcome, RunOutcome::Halted);
    let [timer, window, _] = counters(&exits);
    assert!(timer > 4 && window >= 64, "{timer} {window}");
    assert_eq!(served, exits);
    // Most accesses no longer end a slice.
    assert!(
        served_slices * 3 < exit_slices,
        "{served_slices} served slices, {exit_slices} with every access returning"
    );
}

#[test]
fn every_stopping_point_of_a_capped_run_matches() {
    // Caps land on served accesses, REP iterations and interrupt entries.
    for cap in (1..400).step_by(7).chain([999, 2_001, 3_337]) {
        let (exits, _, _) = run(rig(false), cap);
        let (served, _, _) = run(rig(true), cap);
        assert_eq!(served, exits, "cap {cap}");
    }
}

#[test]
fn a_fractional_cpu_rate_keeps_the_carried_phase() {
    let rate = |rig: Rig| Rig {
        machine: rig
            .machine
            .with_cpu_rate(NonZeroU64::new(3).unwrap(), NonZeroU64::new(2).unwrap()),
        ..rig
    };
    let (exits, _, _) = run(rate(rig(false)), 1_000_000);
    let (served, _, _) = run(rate(rig(true)), 1_000_000);
    assert_eq!(served, exits);
}

#[test]
fn a_memory_map_hook_still_sees_every_port_write() {
    let with_hook = |in_process: bool| {
        let applied = Rc::new(Cell::new(0u64));
        let count = applied.clone();
        let rig = rig(in_process);
        let rig = Rig {
            machine: rig.machine.with_memory_map(move |_, event| {
                if event == MemoryMapSync::Apply {
                    count.set(count.get() + 1);
                }
                Ok(())
            }),
            ..rig
        };
        let (outcome, _, _) = run(rig, 1_000_000);
        (outcome, applied.get())
    };
    let exits = with_hook(false);
    assert!(exits.1 > 64 * 2, "{}", exits.1);
    assert_eq!(with_hook(true), exits);
}

#[test]
fn turning_serving_off_between_runs_keeps_the_machine_the_same() {
    let mut toggled = rig(true);
    let mut reference = rig(false);
    let mut cap = 0;
    for slice in 0..12 {
        cap += 311;
        toggled.machine.set_in_process_devices(slice % 2 == 0);
        let a = toggled.machine.run_bounded(Some(cap)).unwrap();
        let b = reference.machine.run_bounded(Some(cap)).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            toggled.machine.cpu().retired(),
            reference.machine.cpu().retired()
        );
    }
    assert_eq!(run(toggled, 1_000_000).0, run(reference, 1_000_000).0);
}

/// With TF set every instruction traps, and a request raised by a served
/// store waits behind the trap the CPU holds for that store: the loop's
/// acknowledgement must land where it lands after the ordinary stop.
#[test]
fn a_request_raised_under_single_step_is_acknowledged_where_the_loop_would() {
    // Capped: TF is still set at the final HLT, so HLT retires, the trap is
    // taken and IRET resumes past it; the guest never halts either way and
    // the comparison needs only the same run.
    for cap in [997, 20_011, 60_000] {
        let (exits, _, _) = run(traced_rig(false, true), cap);
        let (served, _, count) = run(traced_rig(true, true), cap);
        assert!(count.mmio > 0, "{count:?}");
        assert_eq!(served, exits, "cap {cap}");
        if cap == 60_000 {
            let [timer, window, steps] = counters(&exits);
            assert!(
                timer > 4 && window >= 64 && steps > 1000,
                "{timer} {window} {steps}"
            );
        }
    }
}
