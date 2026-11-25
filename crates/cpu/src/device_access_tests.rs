// SPDX-License-Identifier: AGPL-3.0-or-later
//! The x86jit adapter of the in-process device access seam: served accesses
//! keep the positions, values and final state of the ordinary stops.

use super::*;
use crate::{AccessControl, AccessResult, AccessSpace, DeviceAccess, ServedRange};

const WINDOW: u64 = 0xA0000;

/// A real-mode 80286 machine-profile CPU: RAM below A0000h, a trapped window
/// from A0000h up to `trap_end`, code at 0000:7C00.
fn machine_cpu(code: &[u8], trap_end: u64) -> X86JitCpu {
    let mut cpu =
        X86JitCpu::new_i286_bare(0x100000, 0, 0x7C00, RealSegments::flat(0)).expect("cpu");
    cpu.map_ram(0, WINDOW).expect("ram");
    cpu.map_mmio(WINDOW, trap_end - WINDOW).expect("window");
    cpu.write_mem(0x7C00, code).expect("code");
    cpu.set_served_ranges(&[
        ServedRange::Ports(0x80..=0x80),
        ServedRange::Mmio(0xA0000..=0xBFFFF),
    ])
    .expect("ranges");
    cpu
}

/// One access as both paths see it, with the steps the run had completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    space: AccessSpace,
    write: bool,
    address: u64,
    size: u8,
    value: u64,
    steps: u64,
}

/// A device model both paths share: reads return a value derived from the
/// address and the number of accesses so far.
#[derive(Default)]
struct Model {
    seen: Vec<Seen>,
}

impl Model {
    fn serve(&mut self, seen: Seen) -> u64 {
        self.seen.push(seen);
        if seen.write {
            0
        } else {
            (seen.address ^ self.seen.len() as u64) & 0xFFFF_FFFF
        }
    }
}

struct Handler<F>(F);

impl<F: FnMut(&DeviceAccess, &mut AccessControl) -> AccessResult> DeviceAccessHandler
    for Handler<F>
{
    fn access(&mut self, access: &DeviceAccess, control: &mut AccessControl) -> AccessResult {
        (self.0)(access, control)
    }
}

fn seen(access: &DeviceAccess) -> Seen {
    Seen {
        space: access.space,
        write: access.write,
        address: access.address,
        size: access.size,
        value: access.value,
        steps: access.steps,
    }
}

/// The ordinary path: run until HLT, servicing every stop from the model,
/// with the steps counted from the first run.
fn exit_path(cpu: &mut X86JitCpu, model: &mut Model) -> CpuStop {
    let mut total = 0;
    loop {
        let run = cpu.run_steps(1000, StepRunOptions::default()).expect("run");
        total += run.steps;
        let CpuStepStop::Exit(stop) = run.stop else {
            panic!("{:?}", run.stop)
        };
        let access = |space, write, address, size, value| Seen {
            space,
            write,
            address,
            size,
            value,
            steps: total,
        };
        match stop {
            CpuStop::PortOut { port, size, value } => {
                model.serve(access(AccessSpace::Port, true, port.into(), size, value));
            }
            CpuStop::PortIn { port, size } => {
                let value = model.serve(access(AccessSpace::Port, false, port.into(), size, 0));
                cpu.complete_port_in(value);
            }
            CpuStop::MmioWrite { addr, size, value } => {
                model.serve(access(AccessSpace::Mmio, true, addr, size, value));
                cpu.complete_mmio_write();
            }
            CpuStop::MmioRead { addr, size } => {
                let value = model.serve(access(AccessSpace::Mmio, false, addr, size, 0));
                cpu.complete_mmio_read(value);
            }
            other => return other,
        }
    }
}

/// The served path: one run with the model as handler; stops outside the
/// served ranges are serviced as on the ordinary path.
fn served_path(cpu: &mut X86JitCpu, model: &mut Model) -> (CpuStop, u64) {
    let mut handler = Handler(|access: &DeviceAccess, _: &mut AccessControl| {
        AccessResult::Handled(model.serve(seen(access)))
    });
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
        .expect("run");
    let CpuStepStop::Exit(stop) = run.stop else {
        panic!("{:?}", run.stop)
    };
    (stop, run.steps)
}

fn state(cpu: &X86JitCpu) -> (Vec<u64>, u64, u64, Vec<u8>) {
    let regs = [
        CpuReg::Rax,
        CpuReg::Rcx,
        CpuReg::Rdx,
        CpuReg::Rbx,
        CpuReg::Rsp,
        CpuReg::Rsi,
        CpuReg::Rdi,
        CpuReg::Rip,
    ]
    .map(|reg| cpu.reg(reg))
    .to_vec();
    let mut ram = vec![0; 0x10000];
    cpu.read_mem(0, &mut ram).expect("ram");
    (regs, cpu.retired(), cpu.completed_steps(), ram)
}

/// Run `code` both ways and require the same accesses at the same steps,
/// the same stop and the same final state; the served path must have
/// returned only once.
fn assert_equivalent(code: &[u8]) -> Vec<Seen> {
    let mut exits = machine_cpu(code, 0xC0000);
    let mut exit_model = Model::default();
    let exit_stop = exit_path(&mut exits, &mut exit_model);
    let mut served = machine_cpu(code, 0xC0000);
    let mut served_model = Model::default();
    let (served_stop, steps) = served_path(&mut served, &mut served_model);
    assert_eq!(served_stop, exit_stop);
    assert_eq!(served_model.seen, exit_model.seen);
    assert_eq!(steps, served.completed_steps());
    assert_eq!(state(&served), state(&exits));
    served_model.seen
}

/// MOV AX,A000h; MOV DS,AX; NOP; OUT 80h,AL; MOV [5],AL; MOV AL,[6];
/// IN AL,80h; HLT.
const MIXED: &[u8] = &[
    0xB8, 0x00, 0xA0, 0x8E, 0xD8, 0x90, 0xE6, 0x80, 0xA2, 0x05, 0x00, 0xA0, 0x06, 0x00, 0xE4, 0x80,
    0xF4,
];

#[test]
fn served_port_and_window_accesses_keep_the_stop_positions_values_and_state() {
    let seen = assert_equivalent(MIXED);
    let steps: Vec<_> = seen.iter().map(|seen| (seen.space, seen.steps)).collect();
    // A port access counts its own unit; an MMIO access only those before.
    assert_eq!(
        steps,
        [
            (AccessSpace::Port, 4),
            (AccessSpace::Mmio, 4),
            (AccessSpace::Mmio, 5),
            (AccessSpace::Port, 7),
        ]
    );
}

#[test]
fn rep_outs_and_rep_stos_into_the_window_are_served_element_by_element() {
    // MOV AX,A000h; MOV ES,AX; MOV SI,600h; MOV CX,3; REP OUTSB (DX=80h);
    // MOV CX,3; XOR DI,DI; MOV AL,7; REP STOSB; MOV CX,2; REP INSB; HLT.
    let code = [
        0xB8, 0x00, 0xA0, 0x8E, 0xC0, 0xBE, 0x00, 0x06, 0xBA, 0x80, 0x00, 0xB9, 0x03, 0x00, 0xF3,
        0x6E, 0xB9, 0x03, 0x00, 0x31, 0xFF, 0xB0, 0x07, 0xF3, 0xAA, 0xB9, 0x02, 0x00, 0xF3, 0x6C,
        0xF4,
    ];
    let seen = assert_equivalent(&code);
    let ports: Vec<_> = seen
        .iter()
        .filter(|seen| seen.space == AccessSpace::Port && seen.write)
        .map(|seen| seen.steps)
        .collect();
    assert_eq!(ports, [6, 7, 8]);
    let stores: Vec<_> = seen
        .iter()
        .filter(|seen| seen.space == AccessSpace::Mmio)
        .map(|seen| (seen.address, seen.steps))
        .collect();
    // Three STOSB elements, then two INSB stores into ES:DI in the window.
    assert_eq!(stores.len(), 5);
    assert_eq!(
        &stores[..3],
        &[(WINDOW, 11), (WINDOW + 1, 12), (WINDOW + 2, 13)]
    );
}

#[test]
fn a_fallback_returns_the_ordinary_stop_and_serving_resumes_after_it() {
    let mut cpu = machine_cpu(MIXED, 0xC0000);
    let mut offered = Vec::new();
    let mut handler = Handler(|access: &DeviceAccess, _: &mut AccessControl| {
        offered.push(seen(access));
        if access.space == AccessSpace::Port {
            AccessResult::Fallback
        } else {
            AccessResult::Handled(0x5A)
        }
    });
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
        .unwrap();
    assert_eq!(
        run,
        CpuStepRun {
            steps: 4,
            stop: CpuStepStop::Exit(CpuStop::PortOut {
                port: 0x80,
                size: 1,
                value: 0,
            }),
        }
    );
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
        .unwrap();
    assert_eq!(
        run,
        CpuStepRun {
            steps: 3,
            stop: CpuStepStop::Exit(CpuStop::PortIn {
                port: 0x80,
                size: 1
            }),
        }
    );
    assert_eq!(offered.len(), 4);
    assert_eq!(cpu.reg(CpuReg::Rax) & 0xFF, 0x5A);
}

#[test]
fn a_fault_ends_the_run_with_an_error_and_keeps_the_completed_steps() {
    let mut cpu = machine_cpu(MIXED, 0xC0000);
    let mut handler = Handler(|access: &DeviceAccess, _: &mut AccessControl| {
        if access.space == AccessSpace::Mmio {
            AccessResult::Fault
        } else {
            AccessResult::Handled(0)
        }
    });
    let error = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
        .unwrap_err();
    assert!(
        matches!(&error, CpuError::UnexpectedExit { what } if what.contains("device access")),
        "{error}"
    );
    assert_eq!(cpu.completed_steps(), 4);
    assert_eq!(cpu.retired(), 4);
}

#[test]
fn an_interrupt_request_raised_by_a_served_access_stops_at_the_first_ready_boundary() {
    // STI; OUT 80h,AL; NOP; NOP; HLT. OUT is the instruction STI protects,
    // so the boundary after it is the first that accepts an interrupt.
    let code = [0xFB, 0xE6, 0x80, 0x90, 0x90, 0xF4];
    let mut cpu = machine_cpu(&code, 0xC0000);
    let mut raise = Handler(|_: &DeviceAccess, control: &mut AccessControl| {
        control.set_intr(true);
        AccessResult::Handled(0)
    });
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut raise)
        .unwrap();
    assert_eq!(
        run,
        CpuStepRun {
            steps: 2,
            stop: CpuStepStop::IrqReady,
        }
    );
    assert!(cpu.irq_ready());
    // The level belonged to that call: a plain run is not stopped by it.
    assert_eq!(
        cpu.run_steps(1000, StepRunOptions::default()).unwrap(),
        CpuStepRun {
            steps: 3,
            stop: CpuStepStop::Exit(CpuStop::Halt),
        }
    );

    // With IF clear the request waits, as the loop's acknowledgement would.
    let mut cpu = machine_cpu(&[0xFA, 0xE6, 0x80, 0x90, 0x90, 0xF4], 0xC0000);
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut raise)
        .unwrap();
    assert_eq!(
        run,
        CpuStepRun {
            steps: 5,
            stop: CpuStepStop::Exit(CpuStop::Halt),
        }
    );
}

#[test]
fn a_stop_position_moved_earlier_ends_the_run_there() {
    let mut cpu = machine_cpu(&[0x90; 32], 0xC0000);
    cpu.write_mem(0x7C02, &[0xE6, 0x80]).unwrap();
    let mut earlier = Handler(|access: &DeviceAccess, control: &mut AccessControl| {
        control.stop_at(access.steps + 4);
        control.stop_at(access.steps + 9);
        AccessResult::Handled(0)
    });
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut earlier)
        .unwrap();
    // The OUT is the third unit; the earliest request wins.
    assert_eq!(
        run,
        CpuStepRun {
            steps: 7,
            stop: CpuStepStop::Exit(CpuStop::BudgetExhausted),
        }
    );

    // A position already reached lets the MMIO instruction finish first.
    let mut cpu = machine_cpu(MIXED, 0xC0000);
    let mut now = Handler(|access: &DeviceAccess, control: &mut AccessControl| {
        if access.space == AccessSpace::Mmio {
            control.stop_at(access.steps);
        }
        AccessResult::Handled(0)
    });
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut now)
        .unwrap();
    assert_eq!(
        run,
        CpuStepRun {
            steps: 5,
            stop: CpuStepStop::Exit(CpuStop::BudgetExhausted),
        }
    );
    assert_eq!(cpu.reg(CpuReg::Rip), 0x7C0B);
}

#[test]
fn an_access_crossing_the_served_window_edge_is_not_offered() {
    // MOV AX,BFFFh; MOV DS,AX; MOV [000Fh],AX (BFFFFh-C0000h); HLT. The
    // trapped region runs on past C0000h, beyond the served range.
    let code = [0xB8, 0xFF, 0xBF, 0x8E, 0xD8, 0xA3, 0x0F, 0x00, 0xF4];
    let mut cpu = machine_cpu(&code, 0xD0000);
    let mut offered = 0;
    let mut handler = Handler(|_: &DeviceAccess, _: &mut AccessControl| {
        offered += 1;
        AccessResult::Handled(0)
    });
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
        .unwrap();
    assert_eq!(offered, 0);
    assert_eq!(
        run,
        CpuStepRun {
            steps: 2,
            stop: CpuStepStop::Exit(CpuStop::MmioWrite {
                addr: 0xBFFFF,
                size: 2,
                value: 0xBFFF,
            }),
        }
    );
}

#[test]
fn ranges_wait_for_a_run_start_without_an_outstanding_access() {
    let mut cpu = machine_cpu(MIXED, 0xC0000);
    cpu.set_served_ranges(&[]).unwrap();
    let mut offered = 0;
    let mut handler = Handler(|_: &DeviceAccess, _: &mut AccessControl| {
        offered += 1;
        AccessResult::Handled(0)
    });
    // OUT, then the MMIO store, return with no range served.
    for _ in 0..2 {
        let run = cpu
            .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
            .unwrap();
        assert!(matches!(run.stop, CpuStepStop::Exit(_)));
    }
    // The answered store still awaits its replay: the ranges wait for it.
    cpu.complete_mmio_write();
    cpu.set_served_ranges(&[ServedRange::Mmio(0xA0000..=0xBFFFF)])
        .unwrap();
    let run = cpu
        .run_steps_serving(1, StepRunOptions::default(), &mut handler)
        .unwrap();
    assert_eq!(run.steps, 1);
    let run = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
        .unwrap();
    assert_eq!(offered, 1);
    assert!(matches!(
        run.stop,
        CpuStepStop::Exit(CpuStop::PortIn { port: 0x80, .. })
    ));
}

#[test]
fn a_range_the_core_refuses_is_an_error_not_a_silent_omission() {
    let mut cpu = machine_cpu(MIXED, 0xC0000);
    cpu.set_served_ranges(&[
        ServedRange::Ports(0x80..=0x81),
        ServedRange::Ports(0x81..=0x82),
    ])
    .unwrap();
    let mut handler = Handler(|_: &DeviceAccess, _: &mut AccessControl| AccessResult::Handled(0));
    let error = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
        .unwrap_err();
    assert!(
        matches!(&error, CpuError::UnexpectedExit { what } if what.contains("refused")),
        "{error}"
    );
    assert_eq!(cpu.completed_steps(), 0);
}

#[test]
fn a_handler_panic_keeps_its_message_in_the_error() {
    let mut cpu = machine_cpu(MIXED, 0xC0000);
    let mut handler = Handler(|_: &DeviceAccess, _: &mut AccessControl| -> AccessResult {
        panic!("window model broke")
    });
    let error = cpu
        .run_steps_serving(1000, StepRunOptions::default(), &mut handler)
        .unwrap_err();
    assert!(
        matches!(&error, CpuError::UnexpectedExit { what } if what.contains("window model broke")),
        "{error}"
    );
}
