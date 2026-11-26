// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serving the ports of a DMA requester while the CPU runs leaves the
//! machine exactly as returning from the CPU for every access does (§3.1,
//! §7.7; `backlog/docs/design/scheduler.md` §In-process device access).
//!
//! A bus master at port 3F5h and in the display window raises its DMA
//! request at once on a port write with bit 0 set (as the last command
//! byte of a floppy READ DATA does, NEC µPD765A datasheet, READ DATA), when
//! it next settles on a port write with bit 0 clear (a request that only
//! the post-access settle shows), and at once on a window store. The loop
//! services a request raised by a port access inside that access's own
//! step when the step crosses a clock, before the accessed devices settle
//! (Intel 8237A datasheet, DREQ and DACK: a request is served as soon as
//! it is raised); a window store's request before its instruction's step.
//! The requester logs every settle and every transfer with its device time
//! and the state of its request, and the guest copies each transferred
//! byte to a logging port right after the access that requested it, so a
//! served access must leave the same logs, RAM and clocks as one that
//! returns from the CPU.

use std::num::NonZeroU64;

use bus::{Bus, DecodeMap, MmioRange, PortRange, PortWidth};
use chipset::NoInterruptController;
use cpu::x86jit_backend::{RealSegments, X86JitCpu};
use cpu::Cpu;
use device::{
    Device, DeviceError, DmaMemory, DmaOp, DmaOutcome, DmaRequest, DmaTransfer, NextEvent, VClock,
};
use machine::{DmaWiring, Machine, RunOutcome, ServedAccesses};

const WINDOW: u64 = 0xA0000;

/// The requester's port: the floppy controller's data register.
const PORT: u16 = 0x3F5;

/// A bus master in a served port range and the display window; see the
/// module docs. Log words: device time, then 0 for a settle, 1 for a taken
/// request, then the number of transfers completed so far and whether a
/// request is pending.
#[derive(Default)]
struct Requester {
    now: VClock,
    pending: bool,
    on_settle: bool,
    completed: u64,
    log: Vec<u64>,
}

impl Requester {
    fn record(&mut self, kind: u64) {
        self.log
            .extend([self.now.0, kind, self.completed, u64::from(self.pending)]);
    }
}

impl Device for Requester {
    fn name(&self) -> &'static str {
        "test-requester"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
        Ok(self.completed as u32)
    }
    fn port_write(&mut self, _: u16, _: u8, value: u32) -> Result<(), DeviceError> {
        if value & 1 != 0 {
            self.pending = true;
        } else {
            self.on_settle = true;
        }
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
        Ok(0xFF)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), DeviceError> {
        self.pending = true;
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        self.now = now;
    }
    fn sync(&mut self, now: VClock) {
        self.now = now;
        self.record(0);
        if std::mem::take(&mut self.on_settle) {
            self.pending = true;
        }
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn dma_request_pending(&self) -> bool {
        self.pending
    }
    fn take_dma_request(&mut self) -> Option<DmaRequest> {
        self.record(1);
        std::mem::take(&mut self.pending).then(|| DmaRequest {
            channel: 2,
            op: DmaOp::ToMemory(vec![0xA0 + self.completed as u8]),
        })
    }
    fn dma_complete(&mut self, _: DmaOutcome) {
        self.completed += 1;
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

/// Port 80h: logs every write with the device time.
#[derive(Default)]
struct Logger {
    now: VClock,
    log: Vec<u64>,
}

impl Device for Logger {
    fn name(&self) -> &'static str {
        "test-logger"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
        Ok(0xFF)
    }
    fn port_write(&mut self, _: u16, _: u8, value: u32) -> Result<(), DeviceError> {
        self.log.extend([self.now.0, u64::from(value)]);
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

/// Eight rounds of: a port write requesting at once, the byte it moved
/// copied to port 80h; a port write requesting when the requester settles,
/// its byte copied; a served read of the requester; a window store
/// requesting at once, its byte copied. Then HLT.
fn program() -> Vec<u8> {
    let mut main = vec![
        0xFA, // CLI
        0xB8, 0x00, 0xA0, 0x8E, 0xC0, // MOV AX,A000h; MOV ES,AX
        0xBA, 0xF5, 0x03, // MOV DX,3F5h
        0x31, 0xF6, // XOR SI,SI
        0xB9, 0x08, 0x00, // MOV CX,8
    ];
    let body = [
        0xB0, 0x01, 0xEE, // MOV AL,1; OUT DX,AL: request at once
        0x8A, 0x84, 0x00, 0x06, // MOV AL,[SI+0600h]
        0xE6, 0x80, // OUT 80h,AL
        0x46, // INC SI
        0xB0, 0x00, 0xEE, // MOV AL,0; OUT DX,AL: request when settled
        0x8A, 0x84, 0x00, 0x06, // MOV AL,[SI+0600h]
        0xE6, 0x80, // OUT 80h,AL
        0x46, // INC SI
        0xEC, // IN AL,DX
        0xE6, 0x80, // OUT 80h,AL
        0x26, 0xA2, 0x00, 0x00, // MOV ES:[0000h],AL: request at once
        0x8A, 0x84, 0x00, 0x06, // MOV AL,[SI+0600h]
        0xE6, 0x80, // OUT 80h,AL
        0x46, // INC SI
    ];
    main.extend(body);
    main.extend([0xE2, (-(body.len() as i8) - 2) as u8]); // LOOP body
    main.extend([0xF4]); // HLT
    main
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    outcome: RunOutcome,
    retired: u64,
    steps: u64,
    clock: VClock,
    ram: Vec<u8>,
    devices: Vec<Vec<u8>>,
}

/// Bus indices: requester, controller, port 80h.
fn run(in_process: bool, rate: (u64, u64), cap: u64) -> (Outcome, ServedAccesses) {
    let mut cpu = X86JitCpu::new_i286_bare(0x100000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
    cpu.map_ram(0, WINDOW).unwrap();
    cpu.map_mmio(WINDOW, 0x20000).unwrap();
    cpu.write_mem(0x7C00, &program()).unwrap();
    let port = |ports: std::ops::RangeInclusive<u16>, device| PortRange {
        ports,
        device,
        width: PortWidth::Byte,
    };
    let bus = Bus::new(
        vec![
            Box::new(Requester::default()),
            Box::new(Controller::default()),
            Box::new(Logger::default()),
        ],
        DecodeMap {
            ports: vec![port(PORT..=PORT, 0), port(0x80..=0x80, 2)],
            mmio: vec![MmioRange {
                addrs: WINDOW..=0xBFFFF,
                device: 0,
            }],
        },
    );
    let mut machine = Machine::new(cpu, bus, Box::new(NoInterruptController), Vec::new(), None)
        .with_dma_wirings(vec![DmaWiring {
            requester: 0,
            controller: 1,
        }])
        .with_cpu_rate(
            NonZeroU64::new(rate.0).unwrap(),
            NonZeroU64::new(rate.1).unwrap(),
        )
        .with_in_process_devices(in_process);
    let outcome = machine.run_bounded(Some(cap)).unwrap();
    let mut ram = vec![0; 0x20];
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

/// One step per three clocks, so every step's charge visits the clock, one
/// step per clock and two steps per three clocks, where some steps move no
/// clock at all.
const RATES: [(u64, u64); 3] = [(1, 3), (1, 1), (3, 2)];

#[test]
fn dma_raised_by_a_served_port_access_runs_where_the_loop_runs_it() {
    for rate in RATES {
        let (exits, none) = run(false, rate, 100_000);
        let (served, count) = run(true, rate, 100_000);
        assert_eq!(none, ServedAccesses::default());
        assert_eq!(exits.outcome, RunOutcome::Halted, "{rate:?}");
        // Every request was serviced before the guest read its byte.
        let copied: Vec<u64> = exits.devices[2]
            .chunks(8)
            .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
            .skip(1)
            .step_by(2)
            .collect();
        let expected: Vec<u64> = (0..8u64)
            .flat_map(|round| {
                let first = 0xA0 + 3 * round;
                [first, first + 1, 3 * round + 2, first + 2]
            })
            .collect();
        assert_eq!(copied, expected, "{rate:?}");
        // The requester's port accesses and port 80h's are all served.
        assert!(count.ports >= 8 * 7, "{rate:?}: {count:?}");
        assert_eq!(served, exits, "{rate:?}");
    }
}

#[test]
fn every_stopping_point_of_a_capped_dma_run_matches() {
    for rate in RATES {
        for cap in (1..200).step_by(3) {
            let (exits, _) = run(false, rate, cap);
            let (served, _) = run(true, rate, cap);
            assert_eq!(served, exits, "{rate:?} cap {cap}");
        }
    }
}
