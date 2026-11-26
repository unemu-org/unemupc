// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serving a string port transfer while the CPU runs stops at every cap
//! exactly where returning from the CPU for every access stops (§3.1, §6;
//! `backlog/docs/design/scheduler.md` §In-process device access).
//!
//! REP INSW reads a served data port (the fixed-disk task file's data
//! register, ATA-2 §§6.1–6.2) and stores each word to RAM, to the served
//! display window or to a trapped window outside every served range; REP
//! OUTSW goes the other way (Intel 80286 Programmer's Reference Manual, INS
//! and OUTS, REP). An element's port read and its store are one step. When
//! the read falls on the last step of a capped run, the ordinary stop ends
//! the run with the answer pending and the store waits for the next run;
//! the served way must stop there too, with the same registers, memory and
//! device log after every cap, at several CPU rates. A doubleword OUT
//! whose first port is served but whose last is not (8Fh reaching 90h–92h)
//! returns, so ports outside the served ranges stay on the exit path.

use std::cell::RefCell;
use std::num::NonZeroU64;
use std::rc::Rc;

use bus::{Bus, DecodeMap, MmioRange, PortRange, PortWidth};
use chipset::NoInterruptController;
use cpu::x86jit_backend::{RealSegments, X86JitCpu};
use cpu::{Cpu, CpuReg};
use device::{Device, DeviceError, NextEvent, VClock};
use machine::{Machine, RunOutcome, ServedAccesses};

const DATA: u16 = 0x1F0;
/// Served display memory.
const WINDOW: u64 = 0xB8000;
/// Trapped memory outside every served range.
const FAR: u64 = 0x90000;

/// A word data port and two trapped windows; every access is logged with
/// the device time. Reads return a running count.
#[derive(Default)]
struct Script {
    now: VClock,
    next: u64,
    log: Vec<u64>,
}

impl Script {
    fn record(&mut self, kind: u64, address: u64, value: u64) {
        self.log.extend([self.now.0, kind, address, value]);
    }
}

/// The log, shared so it survives the bus.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Script>>);

impl Device for Shared {
    fn name(&self) -> &'static str {
        "test-script"
    }
    fn port_read(&mut self, port: u16, _: u8) -> Result<u32, DeviceError> {
        let mut script = self.0.borrow_mut();
        script.next += 1;
        let value = 0x1000 + script.next;
        script.record(0, u64::from(port), value);
        Ok(value as u32)
    }
    fn port_write(&mut self, port: u16, _: u8, value: u32) -> Result<(), DeviceError> {
        self.0
            .borrow_mut()
            .record(1, u64::from(port), u64::from(value));
        Ok(())
    }
    fn mmio_read(&mut self, addr: u64, _: u8) -> Result<u64, DeviceError> {
        let mut script = self.0.borrow_mut();
        script.next += 1;
        let value = 0x2000 + script.next;
        script.record(2, addr, value);
        Ok(value)
    }
    fn mmio_write(&mut self, addr: u64, _: u8, value: u64) -> Result<(), DeviceError> {
        self.0.borrow_mut().record(3, addr, value);
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        self.0.borrow_mut().now = now;
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
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

/// MOV AX,segment; MOV ES,AX; MOV DS,AX; MOV DX,1F0h; MOV CX,40;
/// XOR SI,SI; MOV DI,0700h; CLD; REP INSW or REP OUTSW; HLT.
fn program(segment: u16, input: bool) -> Vec<u8> {
    let [lo, hi] = segment.to_le_bytes();
    vec![
        0xB8,
        lo,
        hi,
        0x8E,
        0xC0,
        0x8E,
        0xD8,
        0xBA,
        0xF0,
        0x01,
        0xB9,
        40,
        0,
        0x31,
        0xF6,
        0xBF,
        0x00,
        0x07,
        0xFC,
        0xF3,
        if input { 0x6D } else { 0x6F },
        0xF4,
    ]
}

/// The state after one capped run.
#[derive(Debug, PartialEq, Eq)]
struct Poll {
    outcome: RunOutcome,
    virtual_time: u64,
    steps: u64,
    retired: u64,
    clock: VClock,
    registers: Vec<u64>,
    ram: Vec<u8>,
    log: Vec<u64>,
}

fn run(
    segment: u16,
    input: bool,
    rate: (u64, u64),
    caps: &[u64],
    in_process: bool,
) -> (Vec<Poll>, ServedAccesses) {
    let mut cpu = X86JitCpu::new_i286_bare(0x100000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
    cpu.map_ram(0, FAR).unwrap();
    cpu.map_mmio(FAR, 0x8000).unwrap();
    cpu.map_mmio(WINDOW, 0x8000).unwrap();
    cpu.write_mem(0x7C00, &program(segment, input)).unwrap();
    let script = Shared::default();
    let bus = Bus::new(
        vec![Box::new(script.clone())],
        DecodeMap {
            ports: vec![PortRange {
                ports: DATA..=DATA + 1,
                device: 0,
                width: PortWidth::Word,
            }],
            mmio: vec![
                MmioRange {
                    addrs: FAR..=FAR + 0x7FFF,
                    device: 0,
                },
                MmioRange {
                    addrs: WINDOW..=WINDOW + 0x7FFF,
                    device: 0,
                },
            ],
        },
    );
    let mut machine = Machine::new(cpu, bus, Box::new(NoInterruptController), Vec::new(), None)
        .with_cpu_rate(
            NonZeroU64::new(rate.0).unwrap(),
            NonZeroU64::new(rate.1).unwrap(),
        )
        .with_in_process_devices(in_process);
    let mut polls = Vec::new();
    for &cap in caps {
        let outcome = machine.run_bounded(Some(cap)).unwrap();
        let mut ram = vec![0; 0x60];
        machine
            .cpu()
            .read_mem((u64::from(segment) << 4) + 0x700, &mut ram)
            .unwrap_or_else(|_| ram.fill(0));
        polls.push(Poll {
            outcome,
            virtual_time: machine.virtual_time(),
            steps: machine.cpu().completed_steps(),
            retired: machine.cpu().retired(),
            clock: machine.clock(),
            registers: [CpuReg::Rcx, CpuReg::Rsi, CpuReg::Rdi, CpuReg::Rip]
                .map(|reg| machine.cpu().reg(reg))
                .to_vec(),
            ram,
            log: std::mem::take(&mut script.0.borrow_mut().log),
        });
        if outcome == RunOutcome::Halted {
            break;
        }
    }
    (polls, machine.served_accesses())
}

const RATES: [(u64, u64); 4] = [(1, 1), (1, 3), (3, 2), (1, 7)];

/// Element stores to RAM, to the served window and to the trapped window
/// outside every served range.
const DESTINATIONS: [(&str, u16); 3] = [("ram", 0x1000), ("window", 0xB800), ("far", 0x9000)];

#[test]
fn a_cap_inside_a_served_string_transfer_stops_where_the_loop_stops() {
    for rate in RATES {
        for (name, segment) in DESTINATIONS {
            for input in [true, false] {
                let label = format!("{name} input={input} {rate:?}");
                // A cap at every step (each run's only port read is in its
                // last step, so nothing is served), a cap every third step,
                // and one cap at each step of a fresh machine's transfer
                // followed by the rest of the run.
                let every: Vec<u64> = (1..=60).collect();
                let strided: Vec<u64> = (1..=20).map(|n| 3 * n).collect();
                let mut served_total = 0;
                let splits = (1..=50).map(|cap| vec![cap, 200]);
                for caps in [every, strided].into_iter().chain(splits) {
                    let (exits, none) = run(segment, input, rate, &caps, false);
                    let (served, count) = run(segment, input, rate, &caps, true);
                    assert_eq!(none, ServedAccesses::default(), "{label}");
                    assert_eq!(
                        exits.last().unwrap().outcome,
                        RunOutcome::Halted,
                        "{label}: caps {caps:?}"
                    );
                    assert_eq!(served, exits, "{label}: caps {:?}", &caps[..2]);
                    served_total += count.ports;
                }
                // Into the far window every store returns as an MMIO write
                // and the next element's read starts a one-step completion
                // run, in whose last step it returns too: one read per run.
                let floor = if name == "far" && input { 40 } else { 50 * 20 };
                assert!(served_total > floor, "{label}: {served_total} served");
            }
        }
    }
}

/// A logger for 80h–8Fh and one for 90h–92h, by bus index 0 and 1.
fn port_logger_machine(program: &[u8], in_process: bool) -> (Machine<X86JitCpu>, [Shared; 2]) {
    let mut cpu = X86JitCpu::new_i386_bare(0, 0x7C00, RealSegments::flat(0)).unwrap();
    cpu.map_ram(0, 0x10000).unwrap();
    cpu.write_mem(0x7C00, program).unwrap();
    let loggers = [Shared::default(), Shared::default()];
    let port = |ports: std::ops::RangeInclusive<u16>, device| PortRange {
        ports,
        device,
        width: PortWidth::Byte,
    };
    let bus = Bus::new(
        vec![Box::new(loggers[0].clone()), Box::new(loggers[1].clone())],
        DecodeMap {
            ports: vec![port(0x80..=0x8F, 0), port(0x90..=0x92, 1)],
            mmio: Vec::new(),
        },
    );
    let machine = Machine::new(cpu, bus, Box::new(NoInterruptController), Vec::new(), None)
        .with_in_process_devices(in_process);
    (machine, loggers)
}

#[test]
fn an_access_running_past_its_served_range_returns() {
    // MOV EAX,44332211h; MOV DX,8Fh; OUT DX,EAX (reaches 90h-92h);
    // OUT DX,AL; HLT.
    let program = [
        0x66, 0xB8, 0x11, 0x22, 0x33, 0x44, 0xBA, 0x8F, 0x00, 0x66, 0xEF, 0xEE, 0xF4,
    ];
    let run = |in_process: bool| {
        let (mut machine, loggers) = port_logger_machine(&program, in_process);
        assert_eq!(machine.run_bounded(Some(100)).unwrap(), RunOutcome::Halted);
        let logs = loggers.map(|logger| std::mem::take(&mut logger.0.borrow_mut().log));
        (
            logs,
            machine.virtual_time(),
            machine.cpu().retired(),
            machine.served_accesses(),
        )
    };
    let (exits, time, retired, none) = run(false);
    let (served, served_time, served_retired, count) = run(true);
    assert_eq!(none, ServedAccesses::default());
    // 90h, 91h and 92h each saw one byte of the doubleword.
    assert_eq!(exits[1].len(), 3 * 4, "{exits:?}");
    assert_eq!(
        (served, served_time, served_retired),
        (exits, time, retired)
    );
    // Only the byte OUT at 8Fh is served.
    assert_eq!(count, ServedAccesses { ports: 1, mmio: 0 });
}
