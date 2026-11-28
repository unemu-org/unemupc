// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serving port 80h and the display window while the CPU runs leaves the
//! machine exactly as returning from the CPU for every access does (§3.1;
//! `backlog/docs/design/scheduler.md` §In-process device access).
//!
//! A real-mode guest on the 80286 machine profile fills the window with REP
//! STOSW, writes and reads port 80h, writes a word to port 80h whose second
//! cycle reaches a DMA page register (the controller of an idle requester,
//! served since nothing is pending), programs a window event a few clocks
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
    program_rig(in_process, program(trace))
}

fn program_rig(in_process: bool, program: Vec<(u32, Vec<u8>)>) -> Rig {
    extended_rig(in_process, program, Extra::default())
}

/// Devices appended after the requester (bus index 6 on), with display
/// window ranges decoded ahead of the window's and access couplings.
#[derive(Default)]
struct Extra {
    devices: Vec<Box<dyn Device>>,
    mmio: Vec<MmioRange>,
    couplings: Vec<(usize, usize)>,
}

fn extended_rig(in_process: bool, program: Vec<(u32, Vec<u8>)>, extra: Extra) -> Rig {
    let mut cpu = X86JitCpu::new_i286_bare(0x100000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
    cpu.map_ram(0, WINDOW).unwrap();
    cpu.map_mmio(WINDOW, 0x20000).unwrap();
    for (addr, bytes) in program {
        cpu.write_mem(u64::from(addr), &bytes).unwrap();
    }
    let pic = SharedPic::new();
    let port = |ports: std::ops::RangeInclusive<u16>, device| PortRange {
        ports,
        device,
        width: PortWidth::Byte,
    };
    let mut devices: Vec<Box<dyn Device>> = vec![
        Box::new(pic.clone()),
        Box::new(Pit8253::new()),
        Box::new(Window {
            memory: vec![0; 0x20000],
            ..Window::default()
        }),
        Box::new(Logger::default()),
        Box::new(Logger::default()),
        Box::new(Logger::default()),
    ];
    devices.extend(extra.devices);
    let mut mmio = extra.mmio;
    mmio.push(MmioRange {
        addrs: WINDOW..=0xBFFFF,
        device: 2,
    });
    let mut bus = Bus::new(
        devices,
        DecodeMap {
            ports: vec![
                port(0x20..=0x21, 0),
                port(0x40..=0x43, 1),
                port(0x80..=0x80, 3),
                port(0x81..=0x81, 4),
            ],
            mmio,
        },
    );
    for (device, dependent) in extra.couplings {
        bus.add_access_coupling(device, dependent).unwrap();
    }
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
    let devices = (2..)
        .map_while(|i| bus.device(i).map(|device| device.save()))
        .collect();
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
fn a_hook_observing_some_ports_keeps_every_other_port_write_served() {
    let hooked = |ports: std::ops::RangeInclusive<u16>| {
        let applied = Rc::new(Cell::new(0u64));
        let count = applied.clone();
        let rig = rig(true);
        let rig = Rig {
            machine: rig.machine.with_memory_map_for_ports(
                std::iter::once(ports),
                move |_, event| {
                    if event == MemoryMapSync::Apply {
                        count.set(count.get() + 1);
                    }
                    Ok(())
                },
            ),
            ..rig
        };
        let (outcome, _, served) = run(rig, 1_000_000);
        (outcome, served, applied.get())
    };
    let (plain, _, plain_served) = run(rig(true), 1_000_000);
    assert!(plain_served.ports > 64 * 2, "{plain_served:?}");
    // A hook for ports the guest never writes (the DISPI ports of a VBE
    // card) leaves every port write served.
    let (outcome, served, applied) = hooked(0x1CE..=0x1D0);
    assert_eq!(outcome, plain);
    assert_eq!(served, plain_served);
    assert!(applied >= 1, "run starts still apply the hook");
    // A hook for port 80h sees each of its writes, which return to the loop.
    let (outcome, served, applied) = hooked(0x80..=0x80);
    assert_eq!(outcome, plain);
    assert!(served.ports < plain_served.ports, "{served:?}");
    assert!(applied > 64 * 2, "{applied}");
}

#[test]
fn a_ram_window_serves_its_bytes_from_ram_both_ways() {
    let windowed = |in_process: bool| {
        let rig = rig(in_process);
        let rig = Rig {
            machine: rig.machine.with_ram_window(WINDOW..=WINDOW + 0xFF, |addr| {
                Some(0xE000 + (addr - WINDOW))
            }),
            ..rig
        };
        run(rig, 1_000_000)
    };
    let (exits, _, exit_served) = windowed(false);
    let (served, _, in_process) = windowed(true);
    assert_eq!(served, exits);
    assert_eq!(exit_served.mmio, 0);
    assert!(in_process.mmio > 64 * 32, "{in_process:?}");
    // The last fill stored BX = 1 into each word of the window's RAM.
    assert_eq!(&exits.ram[0xE000..0xE040], [1, 0].repeat(32).as_slice());
    // The window's device saw none of the fill, only its control area.
    assert!(exits.devices[0][..0x100].iter().all(|&b| b == 0));
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

/// A device no access reaches, counting how often the machine asks for its
/// next event; its one deadline lies past every run here.
struct Bystander {
    queries: Rc<Cell<u64>>,
}

impl Device for Bystander {
    fn name(&self) -> &'static str {
        "test-bystander"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
        Ok(0xFF)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
        Ok(0xFF)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), DeviceError> {
        Ok(())
    }
    fn tick(&mut self, _: VClock) {}
    fn next_event(&self, _: VClock) -> NextEvent {
        self.queries.set(self.queries.get() + 1);
        NextEvent::At(VClock(1 << 40))
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn dma_request_pending(&self) -> bool {
        false
    }
    fn reset(&mut self) {}
    fn save(&self) -> Vec<u8> {
        Vec::new()
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
        Ok(())
    }
}

/// A served store to the display window leaves every device it did not
/// reach with the deadline it had, so the machine does not ask them again
/// for each store, and the run still matches the ordinary one.
#[test]
fn a_served_window_store_does_not_ask_unreached_devices_again() {
    let bystander_rig = |in_process: bool| {
        let queries = Rc::new(Cell::new(0));
        let extra = Extra {
            devices: vec![Box::new(Bystander {
                queries: queries.clone(),
            })],
            ..Extra::default()
        };
        (extended_rig(in_process, program(false), extra), queries)
    };
    let (rig, _) = bystander_rig(false);
    let (exits, _, _) = run(rig, 1_000_000);
    let (rig, queries) = bystander_rig(true);
    let (served, _, count) = run(rig, 1_000_000);
    assert_eq!(served, exits);
    assert!(count.mmio > 64 * 32, "{count:?}");
    // Port accesses and the loop's own stops still ask every device. Debug
    // builds also ask every device once after each served MMIO access, to
    // check that the answers kept are current.
    let checks = if cfg!(debug_assertions) {
        count.mmio
    } else {
        0
    };
    let queries = queries.get() - checks;
    assert!(queries * 4 < count.mmio, "{queries} queries for {count:?}");
}

/// The interval a store to [`Arm`] schedules for [`Alarm`] through state
/// they share, the kind of link an access coupling declares.
type SharedDue = Rc<Cell<Option<VClock>>>;

/// Sixteen bytes at the start of the display window; a store schedules the
/// alarm a few clocks ahead unless one is already pending.
struct Arm {
    due: SharedDue,
    now: VClock,
    memory: [u8; 16],
}

impl Device for Arm {
    fn name(&self) -> &'static str {
        "test-arm"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
        Ok(0xFF)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
        Ok(())
    }
    fn mmio_read(&mut self, addr: u64, _: u8) -> Result<u64, DeviceError> {
        Ok(u64::from(self.memory[(addr - WINDOW) as usize]))
    }
    fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError> {
        for i in 0..u64::from(size) {
            self.memory[(addr - WINDOW + i) as usize] = (value >> (8 * i)) as u8;
        }
        if self.due.get().is_none() {
            self.due
                .set(Some(VClock(self.now.0 + 5 + (addr - WINDOW) % 7)));
        }
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
        self.memory.to_vec()
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
        Ok(())
    }
}

/// Fires at the time [`Arm`] scheduled and logs the time it was visited.
struct Alarm {
    due: SharedDue,
    log: Vec<u64>,
}

impl Device for Alarm {
    fn name(&self) -> &'static str {
        "test-alarm"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
        Ok(0xFF)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
        Ok(0xFF)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), DeviceError> {
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        if self.due.get().is_some_and(|due| due <= now) {
            self.due.set(None);
            self.log.push(now.0);
        }
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        self.due.get().map_or(NextEvent::Dormant, NextEvent::At)
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn dma_request_pending(&self) -> bool {
        false
    }
    fn reset(&mut self) {}
    fn save(&self) -> Vec<u8> {
        self.log.iter().flat_map(|at| at.to_le_bytes()).collect()
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
        Ok(())
    }
}

/// A served store reaching a device whose access coupling names a timer
/// schedules that timer as the ordinary stop does: the coupled device's
/// deadline is asked again after the store, while unreached ones are not.
#[test]
fn a_served_window_store_reschedules_its_coupled_timer() {
    let coupled_rig = |in_process: bool| {
        let due = SharedDue::default();
        let extra = Extra {
            devices: vec![
                Box::new(Arm {
                    due: due.clone(),
                    now: VClock::ZERO,
                    memory: [0; 16],
                }),
                Box::new(Alarm {
                    due,
                    log: Vec::new(),
                }),
            ],
            mmio: vec![MmioRange {
                addrs: WINDOW..=WINDOW + 15,
                device: 6,
            }],
            couplings: vec![(6, 7)],
        };
        extended_rig(in_process, program(false), extra)
    };
    let (exits, _, _) = run(coupled_rig(false), 1_000_000);
    let (served, _, count) = run(coupled_rig(true), 1_000_000);
    assert!(count.mmio > 64 * 32, "{count:?}");
    let alarms = exits.devices.last().unwrap().len() / 8;
    assert!(alarms >= 64, "{alarms} alarms");
    assert_eq!(served, exits);
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

/// The code trail samples the boundaries at which the CPU returned, so it
/// has fewer samples with serving on; for one setting it is the same on
/// every run, so a stop report stays reproducible.
#[test]
fn the_code_trail_with_serving_on_is_the_same_on_every_run() {
    let trail = |mut rig: Rig| {
        rig.machine.run_bounded(Some(1_000_000)).unwrap();
        let trail = rig.machine.code_trail().unwrap();
        (trail.entries(), trail.samples())
    };
    let served = trail(rig(true));
    assert_eq!(trail(rig(true)), served);
    let exits = trail(rig(false));
    assert_eq!(trail(rig(false)), exits);
    assert!(served.1 < exits.1, "{} {}", served.1, exits.1);
}

/// The IRQ0 handler of [`renaming_program`] at 0:8000h: EOI, then the
/// controller initialised again with the other vector base (08h or 50h,
/// toggled in [0504h]), then IRET (Intel 8259A datasheet, Initialization
/// Command Words).
const RENAMING_HANDLER: &[u8] = &[
    0x50, // PUSH AX
    0xFF, 0x06, 0x00, 0x05, // INC WORD [0500h]
    0xB0, 0x20, 0xE6, 0x20, // EOI
    0xB0, 0x13, 0xE6, 0x20, // ICW1: single, edge triggered, ICW4
    0xA0, 0x04, 0x05, // MOV AL,[0504h]
    0x34, 0x58, // XOR AL,58h: 08h <-> 50h
    0xA2, 0x04, 0x05, // MOV [0504h],AL
    0xE6, 0x21, // ICW2: the new base
    0xB0, 0x01, 0xE6, 0x21, // ICW4
    0xB0, 0xFE, 0xE6, 0x21, // unmask IRQ0
    0x58, 0xCF, // POP AX; IRET
];

/// Twenty IRQ0 interrupts from the 8254 (Intel 8254 datasheet, Mode 2),
/// each handled by [`RENAMING_HANDLER`] under vector 08h or 50h.
fn renaming_program() -> Vec<(u32, Vec<u8>)> {
    let main = vec![
        0xFA, // CLI
        0xB0, 0x13, 0xE6, 0x20, // ICW1
        0xB0, 0x08, 0xE6, 0x21, // ICW2: vectors 08h-0Fh
        0xB0, 0x01, 0xE6, 0x21, // ICW4
        0xB0, 0xFE, 0xE6, 0x21, // unmask IRQ0
        0xB0, 0x34, 0xE6, 0x43, // 8254 counter 0, mode 2
        0xB0, 0xC8, 0xE6, 0x40, 0xB0, 0x00, 0xE6, 0x40, // divisor 200
        0xFB, // STI
        // loop:
        0xFF, 0x06, 0x02, 0x05, // INC WORD [0502h]
        0x83, 0x3E, 0x00, 0x05, 0x14, // CMP WORD [0500h],20
        0x72, 0xF5, // JB loop
        0xFA, 0xF4, // CLI; HLT
    ];
    vec![
        (0x7C00, main),
        (0x8000, RENAMING_HANDLER.to_vec()),
        (0x20, vec![0x00, 0x80, 0, 0]),
        (0x50 * 4, vec![0x00, 0x80, 0, 0]),
        (0x504, vec![0x08]),
    ]
}

/// An IRQ is named by its controller input from the vector bases in
/// effect when it was acknowledged, also when the handler's served port
/// writes give the controller a new base before the slice ends.
#[test]
fn an_irq_is_named_from_the_bases_it_was_taken_under() {
    let run = |in_process: bool| {
        let mut cpu = X86JitCpu::new_i286_bare(0x100000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, WINDOW).unwrap();
        for (addr, bytes) in renaming_program() {
            cpu.write_mem(u64::from(addr), &bytes).unwrap();
        }
        let pic = SharedPic::new();
        let port = |ports: std::ops::RangeInclusive<u16>, device| PortRange {
            ports,
            device,
            width: PortWidth::Byte,
        };
        let bus = Bus::new(
            vec![Box::new(pic.clone()), Box::new(Pit8253::new())],
            DecodeMap {
                ports: vec![port(0x20..=0x21, 0), port(0x40..=0x43, 1)],
                mmio: Vec::new(),
            },
        );
        let lines = vec![IrqLine {
            device: 1,
            output: 0,
            irq: 0,
        }];
        let mut machine =
            Machine::new(cpu, bus, Box::new(pic), lines, None).with_in_process_devices(in_process);
        machine.enable_delivery_log().unwrap();
        machine.set_irq_bases_of(Box::new(|bus: &Bus| {
            let master = bus
                .device(0)
                .and_then(|pic| devices::Pic8259::diagnostic_from_snapshot(&pic.save()))
                .map(|pic| pic.vector_base);
            [master, None]
        }));
        assert_eq!(
            machine.run_bounded(Some(1_000_000)).unwrap(),
            RunOutcome::Halted
        );
        let mut ram = vec![0; 0x10000];
        machine.cpu().read_mem(0, &mut ram).unwrap();
        (
            machine.recent_deliveries().unwrap(),
            machine.cpu().retired(),
            ram,
            machine.served_accesses(),
        )
    };
    let (exits, retired, ram, _) = run(false);
    let (served, served_retired, served_ram, count) = run(true);
    assert!(count.ports > 20 * 4, "{count:?}");
    let irqs: Vec<_> = exits
        .entries
        .iter()
        .filter_map(|entry| match entry {
            machine::DeliveryEntry::Deliveries { last, .. }
                if last.kind == cpu::DeliveryKind::Irq =>
            {
                Some((last.vector, last.irq))
            }
            _ => None,
        })
        .collect();
    assert_eq!(irqs.len(), 20, "{irqs:?}");
    assert!(irqs.iter().all(|&(_, irq)| irq == Some(0)), "{irqs:?}");
    assert!(irqs.iter().any(|&(vector, _)| vector == 0x50), "{irqs:?}");
    assert_eq!(served.entries, exits.entries);
    assert_eq!((served_retired, served_ram), (retired, ram));
}

/// A timer request pending behind CLI while the guest polls served ports,
/// then taken after STI, also when the next access sits in the STI or
/// MOV SS shadow (Intel 80286 Programmer's Reference Manual, §9.5,
/// Interrupt Priorities; Intel 8259A datasheet, Interrupt Sequence).
fn masked_program() -> Vec<(u32, Vec<u8>)> {
    let main = vec![
        0xFA, // CLI
        0xB0, 0x13, 0xE6, 0x20, // ICW1
        0xB0, 0x08, 0xE6, 0x21, // ICW2: vectors 08h-0Fh
        0xB0, 0x01, 0xE6, 0x21, // ICW4
        0xB0, 0xFE, 0xE6, 0x21, // unmask IRQ0
        0xB0, 0x34, 0xE6, 0x43, // 8254 counter 0, mode 2
        0xB0, 0x32, 0xE6, 0x40, 0xB0, 0x00, 0xE6, 0x40, // divisor 50
        0xB9, 0xF4, 0x01, // MOV CX,500
        0xE4, 0x80, // poll: IN AL,80h
        0xE2, 0xFC, // LOOP poll
        0xFB, // STI
        0xE4, 0x80, // IN AL,80h in the STI shadow
        0xB9, 0xF4, 0x01, // MOV CX,500
        // again:
        0xFA, // CLI
        0xE4, 0x80, 0xE4, 0x80, // IN AL,80h twice
        0x8C, 0xD0, 0x8E, 0xD0, // MOV AX,SS; MOV SS,AX
        0xE4, 0x80, // IN AL,80h in the MOV SS shadow
        0xFB, // STI
        0xE4, 0x80, // IN AL,80h in the STI shadow
        0xE2, 0xF0, // LOOP again
        0xFA, 0xF4, // CLI; HLT
    ];
    let irq0 = vec![
        0x50, // PUSH AX
        0xFF, 0x06, 0x00, 0x05, // INC WORD [0500h]
        0xB0, 0x20, 0xE6, 0x20, // EOI
        0x58, 0xCF, // POP AX; IRET
    ];
    vec![
        (0x7C00, main),
        (0x8000, irq0),
        (0x20, vec![0x00, 0x80, 0, 0]),
    ]
}

#[test]
fn a_request_pending_behind_cli_is_acknowledged_where_the_loop_would() {
    let (exits, _, _) = run(program_rig(false, masked_program()), 1_000_000);
    let (served, _, count) = run(program_rig(true, masked_program()), 1_000_000);
    assert_eq!(exits.outcome, RunOutcome::Halted);
    assert!(count.ports > 2_000, "{count:?}");
    let [timer, _, _] = counters(&exits);
    assert!(timer > 20, "{timer}");
    assert_eq!(served, exits);
    // Caps land inside the masked poll, in the shadows and at entries.
    for cap in (1..3_000).step_by(37).chain([4_999, 7_001, 9_973]) {
        let (exits, _, _) = run(program_rig(false, masked_program()), cap);
        let (served, _, _) = run(program_rig(true, masked_program()), cap);
        assert_eq!(served, exits, "cap {cap}");
    }
}

/// Timer divisors that land the 8254's request at every phase of the
/// short loops below.
const PHASE_DIVISORS: [u16; 12] = [19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67];

/// The 8259 and 8254 set up for IRQ0 at `divisor` (Intel 8254 datasheet,
/// Mode 2) with interrupts disabled, then `prologue` and a loop of `body`
/// run 4000h times by LOOP, then CLI; HLT. IRQ0 counts in [0500h], the
/// single-step trap in [0504h] (Intel 80286 Programmer's Reference Manual,
/// §9.5, Single Step Interrupt).
fn traced_loop_program(divisor: u16, prologue: &[u8], body: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let [lo, hi] = divisor.to_le_bytes();
    let mut main = vec![
        0xFA, // CLI
        0xB0, 0x13, 0xE6, 0x20, // ICW1
        0xB0, 0x08, 0xE6, 0x21, // ICW2: vectors 08h-0Fh
        0xB0, 0x01, 0xE6, 0x21, // ICW4
        0xB0, 0xFE, 0xE6, 0x21, // unmask IRQ0
        0xB0, 0x34, 0xE6, 0x43, // 8254 counter 0, mode 2
        0xB0, lo, 0xE6, 0x40, 0xB0, hi, 0xE6, 0x40, // the divisor
    ];
    main.extend_from_slice(prologue);
    main.extend([0xB9, 0x00, 0x40]); // MOV CX,4000h
    main.extend_from_slice(body);
    main.extend([0xE2, (-(body.len() as i8) - 2) as u8]); // LOOP body
    main.extend([0xFA, 0xF4]); // CLI; HLT
    let irq0 = vec![
        0x50, // PUSH AX
        0xFF, 0x06, 0x00, 0x05, // INC WORD [0500h]
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
        (0x8200, step),
        (0x04, vec![0x00, 0x82, 0, 0]),
        (0x20, vec![0x00, 0x80, 0, 0]),
    ]
}

/// STI, then TF set through PUSHF; POP AX; OR AH,1; PUSH AX; POPF.
const STI_AND_TRACE: &[u8] = &[0xFB, 0x9C, 0x58, 0x80, 0xCC, 0x01, 0x50, 0x9D];

/// Both ways at every divisor must match, including every acknowledgement
/// time in the controller's log.
fn assert_traced_loop_matches(body: &[u8], prologue: &[u8]) {
    let mut diverged = Vec::new();
    for divisor in PHASE_DIVISORS {
        let program = || traced_loop_program(divisor, prologue, body);
        let (exits, _, _) = run(program_rig(false, program()), 200_000);
        let (served, _, count) = run(program_rig(true, program()), 200_000);
        let [timer, _, steps] = counters(&exits);
        assert!(timer > 10 && steps > 1000, "{divisor}: {timer} {steps}");
        assert!(count.ports > 1000, "{divisor}: {count:?}");
        if served != exits {
            diverged.push(divisor);
        }
    }
    assert!(diverged.is_empty(), "diverged at divisors {diverged:?}");
}

/// With TF and IF set, a served IN in the shadow of MOV SS: a run that
/// starts in the shadow has a request pending and the CPU not ready, and
/// the boundary after the IN holds the IN's single-step trap, which the
/// CPU delivers before it would stop for the request.
#[test]
fn a_request_behind_a_traced_access_in_a_mov_ss_shadow_is_acknowledged_where_the_loop_would() {
    // MOV AX,SS; MOV SS,AX; IN AL,80h.
    assert_traced_loop_matches(&[0x8C, 0xD0, 0x8E, 0xD0, 0xE4, 0x80], STI_AND_TRACE);
}

/// The same with TF set and the served IN in the shadow of STI, after a
/// served IN with interrupts disabled.
#[test]
fn a_request_behind_a_traced_access_in_an_sti_shadow_is_acknowledged_where_the_loop_would() {
    // CLI; IN AL,80h; STI; IN AL,80h.
    assert_traced_loop_matches(&[0xFA, 0xE4, 0x80, 0xFB, 0xE4, 0x80], STI_AND_TRACE);
}

/// TF and IF set together by POPF in a run that began with both clear,
/// then a served IN whose single-step trap is held at the next boundary.
#[test]
fn a_request_behind_a_traced_access_after_popf_is_acknowledged_where_the_loop_would() {
    assert_traced_loop_matches(
        &[
            0xFA, // CLI
            0xE4, 0x80, 0xE4, 0x80, // IN AL,80h twice
            0x68, 0x02, 0x03, // PUSH 0302h: IF and TF
            0x9D, // POPF
            0xE4, 0x80, // IN AL,80h, traced
            0x9C, 0x58, 0x80, 0xE4, 0xFE, 0x50, 0x9D, // clear TF again
        ],
        &[],
    );
}

/// A run that began with interrupts disabled, then STI directly followed
/// by MOV SS and a served IN in its shadow, all traced.
#[test]
fn a_request_behind_a_traced_access_in_a_mov_ss_shadow_after_sti_is_acknowledged_where_the_loop_would(
) {
    // CLI; IN AL,80h; MOV AX,SS; STI; MOV SS,AX; IN AL,80h.
    assert_traced_loop_matches(
        &[0xFA, 0xE4, 0x80, 0x8C, 0xD0, 0xFB, 0x8E, 0xD0, 0xE4, 0x80],
        STI_AND_TRACE,
    );
}

/// TF and IF set together by POPF in a run that began with both clear,
/// then a served IN in the shadow of MOV SS.
#[test]
fn a_request_behind_a_traced_access_in_a_mov_ss_shadow_after_popf_is_acknowledged_where_the_loop_would(
) {
    assert_traced_loop_matches(
        &[
            0xFA, // CLI
            0xE4, 0x80, 0xE4, 0x80, // IN AL,80h twice
            0x68, 0x02, 0x03, // PUSH 0302h: IF and TF
            0x9D, // POPF
            0x8C, 0xD0, 0x8E, 0xD0, // MOV AX,SS; MOV SS,AX
            0xE4, 0x80, // IN AL,80h in the shadow, traced
            0x9C, 0x58, 0x80, 0xE4, 0xFE, 0x50, 0x9D, // clear TF again
        ],
        &[],
    );
}
