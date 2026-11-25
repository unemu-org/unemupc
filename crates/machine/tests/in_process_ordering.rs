// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serving an access while the CPU runs keeps the loop's order of device
//! events and DMA when the access moves a deadline into its own step (§3.1,
//! §7.7; `backlog/docs/design/scheduler.md` §In-process device access).
//!
//! At a CPU rate of one step per three virtual ticks, a write to a register
//! device arms two events one and two ticks later, both inside the step the
//! write belongs to. The first event raises a bus master's DMA request (a
//! wired request line, shared state here); the second logs whether that
//! request was already serviced. The loop services DMA between the two
//! events (Intel 8237A datasheet, DREQ and DACK: a request is served as soon
//! as it is raised), so a served access must leave the same log, RAM and
//! clocks as an access that returns from the CPU.

use std::cell::Cell;
use std::num::NonZeroU64;
use std::rc::Rc;

use bus::{Bus, DecodeMap, MmioRange, PortRange, PortWidth};
use chipset::NoInterruptController;
use cpu::x86jit_backend::{RealSegments, X86JitCpu};
use cpu::Cpu;
use device::{
    Device, DeviceError, DmaMemory, DmaOp, DmaOutcome, DmaRequest, DmaTransfer, NextEvent, VClock,
};
use machine::{DmaWiring, Machine, RunOutcome, ServedAccesses};

const WINDOW: u64 = 0xA0000;

/// The requester's state, shared with the device that raises its request.
const IDLE: u8 = 0;
const REQUESTED: u8 = 1;
const TAKEN: u8 = 2;
const COMPLETE: u8 = 3;

/// A register device at port 80h and in the display window. Any write arms
/// two events, one and two ticks ahead; the first raises the requester's
/// DMA request, the second logs the request's state. Every write is logged
/// with the device time.
struct Kicker {
    now: VClock,
    events: Vec<VClock>,
    request: Rc<Cell<u8>>,
    log: Vec<u64>,
}

impl Kicker {
    fn arm(&mut self) {
        self.log.push(self.now.0);
        self.events = vec![VClock(self.now.0 + 1), VClock(self.now.0 + 2)];
    }
}

impl Device for Kicker {
    fn name(&self) -> &'static str {
        "test-kicker"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
        Ok(0xFF)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
        self.arm();
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
        Ok(0xFF)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), DeviceError> {
        self.arm();
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        self.now = now;
        while self.events.first().is_some_and(|&at| at <= now) {
            self.events.remove(0);
            if self.events.len() == 1 {
                self.request.set(REQUESTED);
            } else {
                self.log.push(u64::from(self.request.get()));
            }
        }
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        self.events
            .first()
            .map_or(NextEvent::Dormant, |&at| NextEvent::At(at))
    }
    fn irq_pending(&self) -> bool {
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

/// A bus master whose request line the kicker raises; it hands one byte to
/// memory per request.
struct Requester(Rc<Cell<u8>>);

impl Device for Requester {
    fn name(&self) -> &'static str {
        "test-requester"
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
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn dma_request_pending(&self) -> bool {
        self.0.get() == REQUESTED
    }
    fn take_dma_request(&mut self) -> Option<DmaRequest> {
        (self.0.get() == REQUESTED).then(|| {
            self.0.set(TAKEN);
            DmaRequest {
                channel: 1,
                op: DmaOp::ToMemory(vec![0xA5]),
            }
        })
    }
    fn dma_complete(&mut self, _: DmaOutcome) {
        self.0.set(COMPLETE);
    }
    fn reset(&mut self) {}
    fn save(&self) -> Vec<u8> {
        vec![self.0.get()]
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
        Ok(())
    }
}

/// A DMA controller that writes each transfer at 600h onwards.
#[derive(Default)]
struct Controller {
    next: u32,
}

impl Device for Controller {
    fn name(&self) -> &'static str {
        "test-controller"
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
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn dma_service_write(&mut self, _: u8, mem: &mut dyn DmaMemory, data: &[u8]) -> DmaTransfer {
        for &byte in data {
            mem.dma_write(0x600 + self.next, byte);
            self.next += 1;
        }
        DmaTransfer {
            transferred: data.len(),
            terminal_count: true,
        }
    }
    fn reset(&mut self) {}
    fn save(&self) -> Vec<u8> {
        self.next.to_le_bytes().to_vec()
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
        Ok(())
    }
}

/// CLI; MOV AX,A000h; MOV ES,AX; OUT 80h,AL; NOP x4; MOV ES:[0],AL;
/// NOP x4; OUT 80h,AL; MOV ES:[1],AL; NOP x4; HLT.
const PROGRAM: &[u8] = &[
    0xFA, 0xB8, 0x00, 0xA0, 0x8E, 0xC0, 0xE6, 0x80, 0x90, 0x90, 0x90, 0x90, 0x26, 0xA2, 0x00, 0x00,
    0x90, 0x90, 0x90, 0x90, 0xE6, 0x80, 0x26, 0xA2, 0x01, 0x00, 0x90, 0x90, 0x90, 0x90, 0xF4,
];

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    outcome: RunOutcome,
    retired: u64,
    steps: u64,
    clock: VClock,
    ram: Vec<u8>,
    devices: Vec<Vec<u8>>,
}

fn run(in_process: bool) -> (Outcome, ServedAccesses) {
    let mut cpu = X86JitCpu::new_i286_bare(0x100000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
    cpu.map_ram(0, WINDOW).unwrap();
    cpu.map_mmio(WINDOW, 0x20000).unwrap();
    cpu.write_mem(0x7C00, PROGRAM).unwrap();
    let request = Rc::new(Cell::new(IDLE));
    let bus = Bus::new(
        vec![
            Box::new(Kicker {
                now: VClock::ZERO,
                events: Vec::new(),
                request: request.clone(),
                log: Vec::new(),
            }),
            Box::new(Requester(request)),
            Box::new(Controller::default()),
        ],
        DecodeMap {
            ports: vec![PortRange {
                ports: 0x80..=0x80,
                device: 0,
                width: PortWidth::Byte,
            }],
            mmio: vec![MmioRange {
                addrs: WINDOW..=0xBFFFF,
                device: 0,
            }],
        },
    );
    let mut machine = Machine::new(cpu, bus, Box::new(NoInterruptController), Vec::new(), None)
        .with_dma_wirings(vec![DmaWiring {
            requester: 1,
            controller: 2,
        }])
        .with_cpu_rate(NonZeroU64::new(1).unwrap(), NonZeroU64::new(3).unwrap())
        .with_in_process_devices(in_process);
    let outcome = machine.run_bounded(Some(10_000)).unwrap();
    let mut ram = vec![0; 0x10];
    machine.cpu().read_mem(0x600, &mut ram).unwrap();
    let bus = machine.bus_mut();
    bus.settle();
    let devices = (0..3).map(|i| bus.device(i).unwrap().save()).collect();
    (
        Outcome {
            outcome,
            retired: machine.cpu().retired(),
            steps: machine.cpu().completed_steps(),
            clock: machine.clock(),
            ram,
            devices,
        },
        machine.served_accesses(),
    )
}

#[test]
fn dma_raised_inside_a_served_access_step_runs_between_its_events() {
    let (exits, none) = run(false);
    let (served, count) = run(true);
    assert_eq!(none, ServedAccesses::default());
    assert_eq!(exits.outcome, RunOutcome::Halted);
    // Four writes, each followed by its second event, which saw the
    // request already complete.
    let log: Vec<u64> = exits.devices[0]
        .chunks(8)
        .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
        .collect();
    assert_eq!(log.len(), 8, "{log:?}");
    assert!(
        log.iter()
            .skip(1)
            .step_by(2)
            .all(|&state| state == u64::from(COMPLETE)),
        "{log:?}"
    );
    assert_eq!(&exits.ram[..4], &[0xA5; 4]);
    assert!(count.ports > 0 && count.mmio > 0, "{count:?}");
    assert_eq!(served, exits);
}
