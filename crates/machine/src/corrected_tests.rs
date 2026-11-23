// SPDX-License-Identifier: AGPL-3.0-or-later
//! Instruction-boundary tests, Intel 80286 Programmer's Reference Manual,
//! §3.11.2 and §9.2; Intel 8259A datasheet, Interrupt Sequence and OCW1/OCW3.

use super::*;
use bus::{DecodeMap, PortRange};
use chipset::{PicObservation, SharedPic};
use cpu::{CpuStepRun, CpuStepStop};
use device::Device;
use testkit::{MockCpu, MockCpuStep};

/// A dormant observer distinguishes access synchronization from unnecessary
/// whole-bus visits at ordinary instruction boundaries.
struct SyncProbe {
    calls: std::rc::Rc<std::cell::RefCell<Vec<VClock>>>,
    queries: std::rc::Rc<std::cell::Cell<usize>>,
}

impl Device for SyncProbe {
    fn name(&self) -> &'static str {
        "sync-probe"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, device::DeviceError> {
        Ok(0)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), device::DeviceError> {
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, device::DeviceError> {
        Ok(0)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), device::DeviceError> {
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        self.calls.borrow_mut().push(now);
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        self.queries.set(self.queries.get() + 1);
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn reset(&mut self) {}
    fn save(&self) -> Vec<u8> {
        Vec::new()
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), device::DeviceError> {
        Ok(())
    }
}

#[test]
fn compute_boundaries_do_not_synchronize_dormant_devices() {
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let cpu = MockCpu::new(Vec::new()).with_step_program(vec![
        step(4, 2, CpuStop::BudgetExhausted, false),
        step(
            2,
            1,
            CpuStop::PortOut {
                port: 0x123,
                size: 1,
                value: 0,
            },
            false,
        ),
        step(1, 1, CpuStop::BudgetExhausted, false),
    ]);
    let mut machine = Machine::new(
        cpu,
        Bus::new(
            vec![Box::new(SyncProbe {
                calls: calls.clone(),
                queries: Default::default(),
            })],
            DecodeMap {
                ports: vec![PortRange {
                    width: bus::PortWidth::default(),
                    ports: 0x123..=0x123,
                    device: 0,
                }],
                mmio: Vec::new(),
            },
        ),
        Box::new(chipset::NoInterruptController),
        Vec::new(),
        None,
    )
    .with_scheduler_engine(SchedulerEngine::Deadline);
    assert_eq!(
        machine.run_bounded(Some(4)).unwrap(),
        RunOutcome::RetiredCapReached
    );
    // Initial state, the accessed device at instruction start, then coupled
    // state after the actual access. Pure computation needs no synchronization.
    assert_eq!(*calls.borrow(), [VClock(0), VClock(2), VClock(3)]);
}

#[test]
fn compute_boundaries_reuse_the_bus_deadline_cache() {
    let queries = std::rc::Rc::new(std::cell::Cell::new(0));
    let cpu = MockCpu::new(Vec::new()).with_step_program(vec![
        step(2, 2, CpuStop::BudgetExhausted, false),
        step(2, 2, CpuStop::BudgetExhausted, false),
    ]);
    let mut machine = Machine::new(
        cpu,
        Bus::new(
            vec![Box::new(SyncProbe {
                calls: Default::default(),
                queries: queries.clone(),
            })],
            DecodeMap::default(),
        ),
        Box::new(chipset::NoInterruptController),
        Vec::new(),
        None,
    )
    .with_step_limit(NonZeroU64::new(2).unwrap())
    .with_scheduler_engine(SchedulerEngine::Deadline);
    machine.run_bounded(Some(4)).unwrap();
    // The initial synchronization invalidates one answer; the replacement
    // remains valid across both CPU slices and both clock advances.
    assert_eq!(queries.get(), 2);
}

#[test]
fn bus_errors_charge_completed_steps_and_settle_before_returning() {
    for engine in [SchedulerEngine::PerClock, SchedulerEngine::Deadline] {
        for stop in [
            CpuStop::PortOut {
                port: 0x123,
                size: 1,
                value: 0,
            },
            CpuStop::PortIn {
                port: 0x123,
                size: 1,
            },
            CpuStop::MmioWrite {
                addr: 0x123,
                size: 1,
                value: 0,
            },
            CpuStop::MmioRead {
                addr: 0x123,
                size: 1,
            },
        ] {
            // Port exits count their stopping step; deferred MMIO exits do not.
            let completed = if matches!(stop, CpuStop::PortIn { .. } | CpuStop::PortOut { .. }) {
                3
            } else {
                2
            };
            let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let cpu = MockCpu::new(Vec::new()).with_step_program(vec![
                step(1, 1, CpuStop::BudgetExhausted, false),
                step(1, 1, CpuStop::BudgetExhausted, false),
                step(1, completed - 2, stop, false),
            ]);
            let bus = Bus::new(
                vec![Box::new(SyncProbe {
                    calls: calls.clone(),
                    queries: Default::default(),
                })],
                DecodeMap::default(),
            )
            .with_undecoded_ports(bus::UndecodedPortAccess::Fault);
            let mut machine = Machine::new(
                cpu,
                bus,
                Box::new(chipset::NoInterruptController),
                Vec::new(),
                None,
            )
            .with_scheduler_engine(engine)
            .with_step_limit(NonZeroU64::new(1).unwrap());
            assert!(matches!(
                machine.run_bounded(Some(3)),
                Err(MachineError::Bus(_))
            ));
            assert_eq!(machine.virtual_time(), completed);
            assert_eq!(machine.clock(), VClock(completed));
            assert_eq!(calls.borrow().last(), Some(&VClock(completed)));
        }
    }
}

/// Two device faces share a gate, so accessing one changes the other's deadline.
struct CoupledDeadline {
    writer: bool,
    armed: std::rc::Rc<std::cell::Cell<bool>>,
    fired: std::rc::Rc<std::cell::RefCell<Vec<VClock>>>,
}

impl Device for CoupledDeadline {
    fn name(&self) -> &'static str {
        "coupled-deadline"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, device::DeviceError> {
        Ok(0)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), device::DeviceError> {
        assert!(self.writer);
        self.armed.set(true);
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, device::DeviceError> {
        Ok(0)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), device::DeviceError> {
        self.port_write(0, 1, 0)
    }
    fn tick(&mut self, now: VClock) {
        if !self.writer && self.armed.get() && now >= VClock(2) {
            self.fired.borrow_mut().push(now);
            self.armed.set(false);
        }
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        if !self.writer && self.armed.get() {
            NextEvent::At(VClock(2))
        } else {
            NextEvent::Dormant
        }
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn reset(&mut self) {}
    fn save(&self) -> Vec<u8> {
        vec![u8::from(self.armed.get())]
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), device::DeviceError> {
        Ok(())
    }
}

#[test]
fn stopping_access_refreshes_coupled_deadlines_before_charging_its_step() {
    for engine in [SchedulerEngine::PerClock, SchedulerEngine::Deadline] {
        let armed = std::rc::Rc::new(std::cell::Cell::new(false));
        let fired = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let devices: Vec<Box<dyn Device>> = [true, false]
            .into_iter()
            .map(|writer| {
                Box::new(CoupledDeadline {
                    writer,
                    armed: armed.clone(),
                    fired: fired.clone(),
                }) as Box<dyn Device>
            })
            .collect();
        let cpu = MockCpu::new(Vec::new()).with_step_program(vec![step(
            1,
            1,
            CpuStop::PortOut {
                port: 0x123,
                size: 1,
                value: 0,
            },
            false,
        )]);
        let mut machine = Machine::new(
            cpu,
            Bus::new(
                devices,
                DecodeMap {
                    ports: vec![PortRange {
                        width: bus::PortWidth::default(),
                        ports: 0x123..=0x123,
                        device: 0,
                    }],
                    mmio: Vec::new(),
                },
            ),
            Box::new(chipset::NoInterruptController),
            Vec::new(),
            None,
        )
        .with_cpu_rate(NonZeroU64::new(1).unwrap(), NonZeroU64::new(4).unwrap())
        .with_scheduler_engine(engine);
        machine.run_bounded(Some(1)).unwrap();
        assert_eq!(*fired.borrow(), [VClock(2)], "{engine:?}");
    }
}

#[test]
fn access_coupling_refreshes_a_relative_deadline_before_step_completion() {
    struct PeriodDevice {
        register: bool,
        now: VClock,
        period: std::rc::Rc<std::cell::Cell<u64>>,
    }

    impl Device for PeriodDevice {
        fn name(&self) -> &'static str {
            "relative-deadline"
        }
        fn port_read(&mut self, _: u16, _: u8) -> Result<u32, device::DeviceError> {
            Ok(self.period.get() as u32)
        }
        fn port_write(&mut self, port: u16, _: u8, value: u32) -> Result<(), device::DeviceError> {
            assert!(self.register);
            assert_eq!(port, 0x61);
            assert_eq!(self.now, VClock(1));
            assert_eq!(value, 1);
            self.period.set(u64::from(value));
            Ok(())
        }
        fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, device::DeviceError> {
            unreachable!("fixture only accesses ports")
        }
        fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), device::DeviceError> {
            unreachable!("fixture only accesses ports")
        }
        fn tick(&mut self, now: VClock) {
            self.now = now;
        }
        fn next_event(&self, _: VClock) -> NextEvent {
            if self.register {
                NextEvent::Dormant
            } else {
                NextEvent::At(VClock(self.now.0 + self.period.get()))
            }
        }
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {
            self.now = VClock::ZERO;
        }
        fn save(&self) -> Vec<u8> {
            self.now.0.to_le_bytes().to_vec()
        }
        fn restore(&mut self, _: &[u8]) -> Result<(), device::DeviceError> {
            unreachable!("fixture does not restore snapshots")
        }
    }

    for coupled in [false, true] {
        let period = std::rc::Rc::new(std::cell::Cell::new(100));
        let devices = [true, false]
            .into_iter()
            .map(|register| {
                Box::new(PeriodDevice {
                    register,
                    now: VClock::ZERO,
                    period: period.clone(),
                }) as Box<dyn Device>
            })
            .collect();
        let mut bus = Bus::new(
            devices,
            DecodeMap {
                ports: vec![PortRange {
                    width: bus::PortWidth::default(),
                    ports: 0x61..=0x61,
                    device: 0,
                }],
                mmio: Vec::new(),
            },
        );
        if coupled {
            bus.add_access_coupling(0, 1).unwrap();
        }
        let cpu = MockCpu::new(Vec::new()).with_step_program(vec![
            step(
                4,
                2,
                CpuStop::PortOut {
                    port: 0x61,
                    size: 1,
                    value: 1,
                },
                false,
            ),
            step(1, 1, CpuStop::BudgetExhausted, false),
            step(1, 1, CpuStop::BudgetExhausted, false),
        ]);
        let mut machine = Machine::new(
            cpu,
            bus,
            Box::new(chipset::NoInterruptController),
            Vec::new(),
            None,
        )
        .with_scheduler_engine(SchedulerEngine::Deadline);
        let outcome = machine.run_bounded(Some(4));
        if coupled {
            assert_eq!(outcome.unwrap(), RunOutcome::RetiredCapReached);
            assert_eq!(machine.clock(), VClock(4));
        } else {
            assert!(
                matches!(outcome, Err(MachineError::Cpu(CpuError::UnexpectedExit { ref what }))
                if what == "scheduler: device deadline did not advance"),
                "{outcome:?}"
            );
        }
    }
}

/// Intel 8259A datasheet, Interrupt Sequence and OCW2: diagnostics observe
/// controller operations without requiring an autonomous PIC clock visit.
#[test]
fn dormant_pic_observations_use_bus_time_without_pic_accesses() {
    let mut pic = SharedPic::new();
    for (port, value) in [(0x20, 0x11), (0x21, 8), (0x21, 4), (0x21, 1)] {
        pic.port_write(port, 1, value).unwrap();
    }
    let mut bus = Bus::new(vec![Box::new(pic.clone())], DecodeMap::default());
    let log = pic.observe(bus.observation_clock());
    let before = pic.save();
    bus.tick_due(VClock(42));
    assert_eq!(pic.save(), before);
    pic.set_irq_line(0, true);
    pic.acknowledge().unwrap();
    bus.tick_due(VClock(47));
    pic.end_of_interrupt();
    let before_drain = pic.save();
    assert_eq!(
        log.take(),
        [
            PicObservation::Acknowledge {
                at: VClock(42),
                vector: 8
            },
            PicObservation::Eoi {
                at: VClock(47),
                command: 0x20,
                value: None
            },
        ]
    );
    assert_eq!(pic.save(), before_drain);
}

fn step(budget: u64, steps: u64, stop: CpuStop, ready: bool) -> MockCpuStep {
    MockCpuStep {
        max_steps: budget,
        run: CpuStepRun {
            steps,
            stop: CpuStepStop::Exit(stop),
        },
        retired: steps,
        irq_ready: ready,
    }
}

fn pending_machine(script: Vec<MockCpuStep>) -> (Machine<MockCpu>, chipset::PicObservationLog) {
    let step_limit = script.iter().map(|step| step.max_steps).max().unwrap_or(1);
    let mut pic = SharedPic::new();
    for (port, value) in [(0x20, 0x11), (0x21, 8), (0x21, 4), (0x21, 1)] {
        pic.port_write(port, 1, value).unwrap();
    }
    pic.set_irq_line(0, true);
    let bus = Bus::new(
        vec![Box::new(pic.clone())],
        DecodeMap {
            ports: vec![PortRange {
                width: bus::PortWidth::default(),
                ports: 0x20..=0x21,
                device: 0,
            }],
            mmio: Vec::new(),
        },
    );
    let log = pic.observe(bus.observation_clock());
    let cpu = MockCpu::new(Vec::new())
        .with_interrupts_enabled(true)
        .with_step_program(script);
    (
        Machine::new(cpu, bus, Box::new(pic), Vec::new(), None)
            .with_step_limit(NonZeroU64::new(step_limit).unwrap())
            .with_scheduler_engine(SchedulerEngine::Deadline),
        log,
    )
}

#[test]
fn mask_in_cli_section_cancels_pending_request_without_inta() {
    let (mut machine, log) = pending_machine(vec![
        step(
            1,
            1,
            CpuStop::PortOut {
                port: 0x21,
                size: 1,
                value: 1,
            },
            false,
        ),
        step(1, 1, CpuStop::BudgetExhausted, false),
    ]);
    machine.cpu = machine.cpu.with_interrupts_enabled(false);
    let outcome = machine.run_bounded(Some(2)).unwrap();
    assert!(machine.cpu.injected.is_empty());
    assert_eq!(outcome, RunOutcome::RetiredCapReached);
    assert_eq!(
        machine.cpu.step_options,
        [
            cpu::StepRunOptions {
                stop_when_irq_ready: true
            },
            cpu::StepRunOptions::default(),
        ]
    );
    assert!(!log
        .take()
        .iter()
        .any(|e| matches!(e, PicObservation::Acknowledge { .. })));
}

#[test]
fn long_cli_section_reaches_irq_readiness_in_one_run() {
    let (mut machine, _) = pending_machine(vec![
        MockCpuStep {
            run: CpuStepRun {
                steps: 4002,
                stop: CpuStepStop::IrqReady,
            },
            ..step(4096, 4002, CpuStop::BudgetExhausted, true)
        },
        step(94, 1, CpuStop::Halt, false),
    ]);
    // No device deadline interrupts the CLI section, STI and protected unit.
    machine.bus = Bus::new(Vec::new(), DecodeMap::default());
    machine.cpu = machine.cpu.with_interrupts_enabled(false);
    assert_eq!(machine.run_bounded(Some(4096)).unwrap(), RunOutcome::Halted);
    assert_eq!(machine.cpu.step_budgets, [4096, 94]);
    assert_eq!(
        machine.cpu.step_options,
        [
            cpu::StepRunOptions {
                stop_when_irq_ready: true
            },
            cpu::StepRunOptions::default(),
        ]
    );
    assert_eq!(machine.cpu.delivered, [8]);
    assert_eq!(machine.virtual_time(), 4003);
}

#[test]
fn pending_irq_run_obeys_deadline_cap_and_step_limit() {
    for (cap, limit, budgets) in [
        (80, 4096, vec![80]),
        (120, 4096, vec![100, 20]),
        (120, 64, vec![64, 36, 20]),
    ] {
        let script = budgets
            .iter()
            .map(|&budget| step(budget, budget, CpuStop::BudgetExhausted, false))
            .collect();
        let (mut machine, log) = pending_machine(script);
        machine.cpu = machine.cpu.with_interrupts_enabled(false);
        machine = machine.with_step_limit(NonZeroU64::new(limit).unwrap());
        let ticks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        machine.bus = Bus::new(
            vec![Box::new(ScheduledLine {
                every_clock: false,
                ticks: ticks.clone(),
                high: false,
            })],
            DecodeMap::default(),
        );
        assert_eq!(
            machine.run_bounded(Some(cap)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.cpu.step_budgets, budgets);
        assert!(machine
            .cpu
            .step_options
            .iter()
            .all(|options| options.stop_when_irq_ready));
        assert_eq!(machine.clock(), VClock(cap));
        assert_eq!(
            *ticks.borrow(),
            if cap >= 100 {
                vec![VClock(100)]
            } else {
                Vec::new()
            }
        );
        assert!(machine.cpu.injected.is_empty());
        assert!(log.take().is_empty());
    }
}

#[test]
fn isr_read_before_sti_has_nothing_in_service() {
    let (mut machine, log) = pending_machine(vec![
        step(
            1,
            1,
            CpuStop::PortOut {
                port: 0x20,
                size: 1,
                value: 0x0B,
            },
            false,
        ),
        step(
            1,
            1,
            CpuStop::PortIn {
                port: 0x20,
                size: 1,
            },
            false,
        ),
    ]);
    machine.cpu = machine.cpu.with_interrupts_enabled(false);
    machine.run_bounded(Some(2)).unwrap();
    assert_eq!(machine.cpu.completions, [0]);
    assert!(log.take().is_empty());
}

#[test]
fn stopping_mask_out_precedes_first_eligible_inta() {
    let (mut machine, log) = pending_machine(vec![
        step(
            1,
            1,
            CpuStop::PortOut {
                port: 0x21,
                size: 1,
                value: 1,
            },
            true,
        ),
        step(1, 1, CpuStop::Halt, true),
    ]);
    machine.run_bounded(Some(2)).unwrap();
    assert!(machine.cpu.injected.is_empty());
    assert!(log.take().is_empty());
}

#[test]
fn sti_and_ss_shadows_wait_for_first_ready_boundary() {
    for (shadow, script, boundary) in [
        (
            "sti",
            vec![
                step(1, 1, CpuStop::BudgetExhausted, false),
                step(1, 1, CpuStop::BudgetExhausted, true),
                step(1, 1, CpuStop::Halt, false),
            ],
            2,
        ),
        (
            "mov ss",
            vec![
                step(1, 1, CpuStop::BudgetExhausted, true),
                step(1, 1, CpuStop::Halt, false),
            ],
            1,
        ),
    ] {
        // STI starts with IF clear; MOV SS starts with IF set and its shadow active.
        let (mut machine, log) = pending_machine(script);
        machine.cpu = machine.cpu.with_interrupts_enabled(shadow == "mov ss");
        machine.run_bounded(Some(boundary + 1)).unwrap();
        assert_eq!(machine.cpu.delivered, [8], "{shadow}");
        assert_eq!(
            log.take(),
            [PicObservation::Acknowledge {
                at: VClock(boundary),
                vector: 8
            }],
            "{shadow}"
        );
    }
}

#[test]
fn exact_cap_wins_over_inta_and_counts_rep_iterations() {
    let (mut machine, log) = pending_machine(vec![MockCpuStep {
        retired: 0,
        ..step(1, 1, CpuStop::BudgetExhausted, true)
    }]);
    machine.run_bounded(Some(1)).unwrap();
    assert_eq!(machine.virtual_time(), 1);
    assert_eq!(machine.clock(), VClock(1));
    assert_eq!(machine.cpu.retired(), 0);
    assert!(log.take().is_empty());
    assert!(machine.cpu.injected.is_empty());
}

#[test]
fn deadline_budget_uses_cpu_rate_and_carried_phase() {
    let cpu = MockCpu::new(Vec::new()).with_step_program(vec![
        step(3, 2, CpuStop::BudgetExhausted, false),
        step(1, 1, CpuStop::BudgetExhausted, false),
        step(2, 2, CpuStop::Halt, false),
    ]);
    let ticks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let clocked = ScheduledLine {
        ticks: ticks.clone(),
        high: false,
        every_clock: true,
    };
    let mut machine = Machine::new(
        cpu,
        Bus::new(vec![Box::new(clocked)], DecodeMap::default()),
        Box::new(chipset::NoInterruptController),
        Vec::new(),
        None,
    )
    .with_scheduler_engine(SchedulerEngine::Deadline)
    .with_cpu_rate(NonZeroU64::new(5).unwrap(), NonZeroU64::new(2).unwrap());
    machine.run_bounded(Some(5)).unwrap();
    assert_eq!(machine.clock(), VClock(2));
    assert_eq!(machine.cpu.step_budgets, [3, 1, 2]);
    assert_eq!(*ticks.borrow(), [VClock(1), VClock(2)]);
}

#[test]
fn real_mode_mov_ss_mov_sp_keeps_irq_off_the_stack_switch() {
    use cpu::x86jit_backend::{RealSegments, X86JitCpu};
    let mut cpu = X86JitCpu::new_real16_bare(0x20000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
    cpu.map_ram(0, 0x20000).unwrap();
    // STI; NOP; MOV SS, AX; MOV SP, BX; CLI; HLT.
    cpu.write_mem(0x7C00, &[0xFB, 0x90, 0x8E, 0xD0, 0x89, 0xDC, 0xFA, 0xF4])
        .unwrap();
    cpu.set_reg(cpu::CpuReg::Rax, 0x1000);
    cpu.set_reg(cpu::CpuReg::Rbx, 0x1000);
    cpu.set_reg(cpu::CpuReg::Rsp, 0x8000);
    // Save the handler's SP to RAM, then return.
    cpu.write_mem(0x8000, &[0x89, 0x26, 0x00, 0x06, 0xCF])
        .unwrap();
    cpu.write_mem(0x20, &[0, 0x80, 0, 0]).unwrap();
    let mut pic = SharedPic::new();
    for (port, value) in [(0x20, 0x11), (0x21, 8), (0x21, 4), (0x21, 1)] {
        pic.port_write(port, 1, value).unwrap();
    }
    let observer = pic.clone();
    let mut machine = Machine::new(
        cpu,
        Bus::new(Vec::new(), DecodeMap::default()),
        Box::new(pic),
        Vec::new(),
        None,
    )
    .with_scheduler_engine(SchedulerEngine::Deadline);
    let log = observer.observe(machine.bus.observation_clock());
    // Return exactly after MOV SS, then assert INTR during its shadow.
    machine.run_bounded(Some(3)).unwrap();
    assert!(!machine.cpu.irq_ready());
    machine.pic.set_irq_line(0, true);
    machine.run_bounded(Some(4)).unwrap();
    assert!(log.take().is_empty());
    assert!(!machine.cpu.has_pending_irq());
    assert_eq!(machine.run_bounded(Some(100)).unwrap(), RunOutcome::Halted);
    let mut saved = [0; 2];
    machine.cpu.read_mem(0x600, &mut saved).unwrap();
    assert_eq!(u16::from_le_bytes(saved), 0x0FFA);
    assert_eq!(machine.cpu.reg(cpu::CpuReg::Rsp), 0x1000);
    assert!(matches!(
        log.take().as_slice(),
        [PicObservation::Acknowledge { vector: 8, .. }]
    ));
}

#[test]
fn real_mode_pending_irq_stops_between_rep_iterations() {
    for split_at_ready_boundary in [false, true] {
        use cpu::x86jit_backend::{RealSegments, X86JitCpu};
        let mut cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        // MOV CX, 3; STI; REP STOSB; CLI; HLT.
        cpu.write_mem(0x7C00, &[0xB9, 3, 0, 0xFB, 0xF3, 0xAA, 0xFA, 0xF4])
            .unwrap();
        cpu.set_reg(cpu::CpuReg::Rdi, 0x600);
        // Save the remaining iteration count before returning to REP.
        cpu.write_mem(0x8000, &[0x89, 0x0E, 0x10, 0x06, 0xCF])
            .unwrap();
        cpu.write_mem(0x20, &[0, 0x80, 0, 0]).unwrap();
        let mut pic = SharedPic::new();
        for (port, value) in [(0x20, 0x11), (0x21, 8), (0x21, 4), (0x21, 1)] {
            pic.port_write(port, 1, value).unwrap();
        }
        let observer = pic.clone();
        let mut machine = Machine::new(
            cpu,
            Bus::new(Vec::new(), DecodeMap::default()),
            Box::new(pic),
            Vec::new(),
            None,
        )
        .with_scheduler_engine(SchedulerEngine::Deadline);
        let log = observer.observe(machine.bus.observation_clock());
        machine.run_bounded(Some(2)).unwrap();
        machine.pic.set_irq_line(0, true);
        if split_at_ready_boundary {
            machine.run_bounded(Some(3)).unwrap();
            assert!(log.take().is_empty());
            assert!(!machine.cpu.has_pending_irq());
        }
        assert_eq!(machine.run_bounded(Some(100)).unwrap(), RunOutcome::Halted);
        let mut remaining = [0; 2];
        machine.cpu.read_mem(0x610, &mut remaining).unwrap();
        assert_eq!(u16::from_le_bytes(remaining), 2);
        assert_eq!(machine.virtual_time(), 9);
        assert_eq!(machine.cpu.retired(), 7);
    }
}

#[test]
fn stopping_eoi_releases_next_request_before_inta() {
    let (mut machine, log) = pending_machine(vec![
        step(
            1,
            1,
            CpuStop::PortOut {
                port: 0x20,
                size: 1,
                value: 0x20,
            },
            true,
        ),
        step(1, 1, CpuStop::Halt, false),
    ]);
    assert_eq!(machine.pic.acknowledge(), Some(8));
    machine.pic.set_irq_line(1, true);
    log.take();
    machine.run_bounded(Some(2)).unwrap();
    assert_eq!(machine.cpu.delivered, [9]);
    assert_eq!(
        log.take(),
        [
            PicObservation::Eoi {
                at: VClock(0),
                command: 0x20,
                value: Some(0x20)
            },
            PicObservation::Acknowledge {
                at: VClock(1),
                vector: 9
            },
        ]
    );
}

#[test]
fn mmio_replays_finish_before_inta_and_are_charged_once() {
    let (mut machine, _) = pending_machine(vec![
        step(
            1,
            0,
            CpuStop::MmioWrite {
                addr: 0xB8000,
                size: 2,
                value: 0x0741,
            },
            false,
        ),
        step(
            1,
            0,
            CpuStop::MmioRead {
                addr: 0xB8000,
                size: 2,
            },
            false,
        ),
        step(1, 1, CpuStop::BudgetExhausted, true),
        step(1, 1, CpuStop::Halt, false),
    ]);
    // Keep the original shared PIC for the controller and its observation log.
    let old = machine.bus.device_mut(0).unwrap();
    let shared = old
        .as_any_mut()
        .unwrap()
        .downcast_mut::<SharedPic>()
        .unwrap()
        .clone();
    machine.bus = Bus::new(
        vec![Box::new(shared.clone()), Box::new(devices::VgaText::new())],
        DecodeMap {
            ports: vec![PortRange {
                width: bus::PortWidth::default(),
                ports: 0x20..=0x21,
                device: 0,
            }],
            mmio: vec![bus::MmioRange {
                addrs: 0xB8000..=0xBFFFF,
                device: 1,
            }],
        },
    );
    machine
        .bus
        .set_trace_policy(device::TracePolicy::Ring { capacity: 16 });
    let log = shared.observe(machine.bus.observation_clock());
    machine.run_bounded(Some(2)).unwrap();
    let trace = machine.bus.device_mut(1).unwrap().take_trace();
    assert!(matches!(
        trace[0],
        device::TraceEvent::Mmio { at: VClock(0), .. }
    ));
    assert!(matches!(
        trace[1],
        device::TraceEvent::Mmio { at: VClock(0), .. }
    ));
    assert_eq!(machine.cpu.completions, [0x0741]);
    assert_eq!(machine.clock(), VClock(2));
    assert_eq!(
        log.take(),
        [PicObservation::Acknowledge {
            at: VClock(1),
            vector: 8
        }]
    );
}

/// A scheduled line with a long dormant prefix exposes whether HLT really skips
/// clocks rather than calling tick once for each idle PIT period (§6).
struct ScheduledLine {
    every_clock: bool,
    ticks: std::rc::Rc<std::cell::RefCell<Vec<VClock>>>,
    high: bool,
}

#[test]
fn halt_delivers_vector_latched_during_halt_without_idle_wait() {
    for engine in [SchedulerEngine::PerClock, SchedulerEngine::Deadline] {
        for idle_limit in [None, Some(0)] {
            let (mut machine, _) = pending_machine(vec![
                step(1, 1, CpuStop::Halt, true),
                step(1, 1, CpuStop::Halt, false),
            ]);
            machine.pic.acknowledge();
            machine.pic.end_of_interrupt();
            machine.pic.set_irq_line(0, false);
            machine.bus = Bus::new(
                vec![Box::new(ScheduledLine {
                    every_clock: true,
                    ticks: Default::default(),
                    high: false,
                })],
                DecodeMap::default(),
            );
            machine.irq_lines = vec![IrqLine {
                device: 0,
                output: 0,
                irq: 0,
            }];
            machine.last_levels = vec![None];
            machine.set_idle_limit(idle_limit);
            machine = machine.with_scheduler_engine(engine);
            assert_eq!(machine.run_bounded(None).unwrap(), RunOutcome::Halted);
            assert_eq!(machine.cpu.delivered, [8]);
            assert_eq!(machine.virtual_time(), 2);
            assert_eq!(machine.halted_clocks, 0);
        }
    }
}
impl Device for ScheduledLine {
    fn name(&self) -> &'static str {
        "scheduled-line"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, device::DeviceError> {
        Ok(0)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), device::DeviceError> {
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, device::DeviceError> {
        Ok(0)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), device::DeviceError> {
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        self.ticks.borrow_mut().push(now);
        self.high = true;
    }
    fn sync(&mut self, _: VClock) {}
    fn next_event(&self, _: VClock) -> NextEvent {
        if self.every_clock {
            NextEvent::EveryPitClock
        } else if self.high {
            NextEvent::Dormant
        } else {
            NextEvent::At(VClock(100))
        }
    }
    fn irq_pending(&self) -> bool {
        self.high
    }
    fn reset(&mut self) {
        self.high = false;
    }
    fn save(&self) -> Vec<u8> {
        Vec::new()
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), device::DeviceError> {
        Ok(())
    }
}

#[test]
fn hlt_jumps_to_deadline_or_exact_cap_and_preserves_idle_span() {
    for cap in [50, 101] {
        let (mut machine, log) = pending_machine(vec![
            step(100.min(cap), 1, CpuStop::Halt, true),
            step(1, 1, CpuStop::Halt, false),
        ]);
        // Remove the initial request: only the scheduled line can wake HLT.
        machine.pic.acknowledge();
        machine.pic.end_of_interrupt();
        machine.pic.set_irq_line(0, false);
        log.take();
        let ticks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let line = ScheduledLine {
            every_clock: false,
            ticks: ticks.clone(),
            high: false,
        };
        machine.bus = Bus::new(vec![Box::new(line)], DecodeMap::default());
        machine.irq_lines = vec![IrqLine {
            device: 0,
            output: 0,
            irq: 0,
        }];
        machine.last_levels = vec![None];
        machine.set_idle_limit(Some(200));
        machine.run_bounded(Some(cap)).unwrap();
        assert_eq!(machine.virtual_time(), cap);
        if cap == 50 {
            assert!(ticks.borrow().is_empty());
            assert_eq!(machine.cpu.retired(), 1);
            assert!(machine.cpu.injected.is_empty());
            assert_eq!(machine.idle_clocks, 49);
            machine.run_bounded(Some(75)).unwrap();
            assert_eq!(machine.idle_clocks, 74);
            machine.set_idle_limit(Some(80));
            assert_eq!(machine.run_bounded(None).unwrap(), RunOutcome::Halted);
            assert_eq!(machine.clock(), VClock(81));
            assert_eq!(machine.idle_clocks, 80);
            assert!(ticks.borrow().is_empty());
            machine.set_idle_limit(Some(200));
            machine.run_bounded(Some(101)).unwrap();
        }
        assert_eq!(machine.cpu.step_budgets, [100.min(cap), 1]);
        assert_eq!(*ticks.borrow(), [VClock(100)]);
        assert_eq!(machine.cpu.delivered, [8]);
        assert_eq!(machine.cpu.retired(), 2);
    }
}

#[test]
fn real_mode_mmio_and_port_effects_settle_at_the_exact_final_cap() {
    use cpu::x86jit_backend::{RealSegments, X86JitCpu};
    for cap in [2, 3, 4] {
        let mut cpu =
            X86JitCpu::new_real16_bare(0x100000, 0, 0x7C00, RealSegments::flat(0xB800)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.map_mmio(0xB8000, 0x8000).unwrap();
        // MOV AX, 0741h; MOV [0], AX; OUT E9h, AL; HLT.
        cpu.write_mem(0x7C00, &[0xB8, 0x41, 0x07, 0xA3, 0, 0, 0xE6, 0xE9, 0xF4])
            .unwrap();
        let mut machine = Machine::new(
            cpu,
            Bus::new(
                vec![Box::new(devices::VgaText::new())],
                DecodeMap {
                    ports: Vec::new(),
                    mmio: vec![bus::MmioRange {
                        addrs: 0xB8000..=0xBFFFF,
                        device: 0,
                    }],
                },
            ),
            Box::new(chipset::NoInterruptController),
            Vec::new(),
            None,
        )
        .with_scheduler_engine(SchedulerEngine::Deadline);
        machine
            .bus
            .set_trace_policy(device::TracePolicy::Ring { capacity: 16 });
        assert_eq!(
            machine.run_bounded(Some(cap)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(machine.virtual_time(), cap);
        assert_eq!(machine.cpu.retired(), cap);
        assert_eq!(
            &machine.bus.device(0).unwrap().text_scanout().unwrap().vram[..2],
            &[0x41, 0x07]
        );
        let ports = machine.bus.take_open_bus_trace();
        assert_eq!(ports.len(), usize::from(cap >= 3));
        if cap >= 3 {
            assert!(matches!(
                ports[0],
                device::TraceEvent::Port {
                    at: VClock(2),
                    value: 0x41,
                    ..
                }
            ));
        }
    }
}

#[test]
fn real_mode_step_limits_do_not_move_observables() {
    use cpu::x86jit_backend::{RealSegments, X86JitCpu};
    let mut expected = None;
    for limit in [1, 64, 65_536] {
        let mut cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        // NOP; MOV AL, 42; OUT E9h, AL; JMP back to NOP.
        cpu.write_mem(0x7C00, &[0x90, 0xB0, 42, 0xE6, 0xE9, 0xEB, 0xF9])
            .unwrap();
        let mut machine = Machine::new(
            cpu,
            Bus::new(Vec::new(), DecodeMap::default()),
            Box::new(chipset::NoInterruptController),
            Vec::new(),
            None,
        )
        .with_scheduler_engine(SchedulerEngine::Deadline)
        .with_step_limit(NonZeroU64::new(limit).unwrap());
        machine.run_bounded(Some(100)).unwrap();
        let actual = (
            machine.clock(),
            machine.cpu.reg(cpu::CpuReg::Rip),
            machine.bus.take_open_bus_trace(),
        );
        assert_eq!(
            actual,
            expected.get_or_insert_with(|| actual.clone()).clone()
        );
        assert_eq!(machine.cpu.retired(), 100);
    }
}

#[test]
fn real_mode_sti_nop_accepts_before_the_next_device_deadline() {
    for split_at_ready_boundary in [false, true] {
        use cpu::x86jit_backend::{RealSegments, X86JitCpu};
        let mut cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.set_reg(cpu::CpuReg::Rdx, 0x3F8);
        cpu.set_reg(cpu::CpuReg::Rsp, 0x7000);
        cpu.write_mem(0x7C00, &[0xFB, 0x90, 0xB0, b'P', 0xEE, 0xFA, 0xF4])
            .unwrap();
        cpu.write_mem(0x8000, &[0xB0, b'I', 0xEE, 0xCF]).unwrap();
        cpu.write_mem(0x20, &[0, 0x80, 0, 0]).unwrap();
        let mut pic = SharedPic::new();
        for (port, value) in [(0x20, 0x11), (0x21, 8), (0x21, 4), (0x21, 1)] {
            pic.port_write(port, 1, value).unwrap();
        }
        pic.set_irq_line(0, true);
        let observer = pic.clone();
        let capture = device::ByteCapture::new(8);
        let mut uart = devices::Uart16550::new(0x3F8);
        uart.byte_output()
            .unwrap()
            .bind_sink(Box::new(capture.clone()));
        let mut machine = Machine::new(
            cpu,
            Bus::new(
                vec![Box::new(uart)],
                DecodeMap {
                    ports: vec![PortRange {
                        width: bus::PortWidth::default(),
                        ports: 0x3F8..=0x3FF,
                        device: 0,
                    }],
                    mmio: Vec::new(),
                },
            ),
            Box::new(pic),
            Vec::new(),
            None,
        )
        .with_scheduler_engine(SchedulerEngine::Deadline);
        let log = observer.observe(machine.bus.observation_clock());
        if split_at_ready_boundary {
            machine.run_bounded(Some(2)).unwrap();
            assert!(log.take().is_empty());
            assert!(!machine.cpu.has_pending_irq());
        }
        assert_eq!(machine.run_bounded(Some(100)).unwrap(), RunOutcome::Halted);
        assert_eq!(capture.drain().unwrap(), b"IP");
        assert!(matches!(
            log.take().as_slice(),
            [PicObservation::Acknowledge { vector: 8, .. }]
        ));
    }
}

/// A bus write or deadline queues DMA; its first completion queues immediate follow-on
/// work. Only the second completion raises the wired line (§7.7).
struct DmaTrigger {
    deadline: Option<VClock>,
    queued: bool,
    completions: std::rc::Rc<std::cell::Cell<u8>>,
    now: VClock,
    completed_at: std::rc::Rc<std::cell::RefCell<Vec<VClock>>>,
}
impl Device for DmaTrigger {
    fn name(&self) -> &'static str {
        "dma-trigger"
    }
    fn port_read(&mut self, _: u16, _: u8) -> Result<u32, device::DeviceError> {
        Ok(0)
    }
    fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), device::DeviceError> {
        self.queued = true;
        Ok(())
    }
    fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, device::DeviceError> {
        Ok(0)
    }
    fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), device::DeviceError> {
        Ok(())
    }
    fn tick(&mut self, now: VClock) {
        self.now = now;
        if self.deadline.is_some_and(|at| at <= now) {
            self.deadline = None;
            self.queued = true;
        }
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        self.deadline.map_or(NextEvent::Dormant, NextEvent::At)
    }
    fn irq_pending(&self) -> bool {
        self.completions.get() == 2
    }
    fn reset(&mut self) {
        self.queued = false;
        self.completions.set(0);
    }
    fn save(&self) -> Vec<u8> {
        Vec::new()
    }
    fn restore(&mut self, _: &[u8]) -> Result<(), device::DeviceError> {
        Ok(())
    }
    fn take_dma_request(&mut self) -> Option<device::DmaRequest> {
        std::mem::take(&mut self.queued).then(|| device::DmaRequest {
            channel: 2,
            op: DmaOp::ToMemory(vec![0xAB]),
        })
    }
    fn dma_complete(&mut self, _: DmaOutcome) {
        self.completed_at.borrow_mut().push(self.now);
        self.completions.set(self.completions.get() + 1);
        self.queued = self.completions.get() < 2;
    }
}

#[test]
fn out_drains_dma_and_follow_on_completion_before_inta() {
    let (mut machine, log) = pending_machine(vec![
        step(
            1,
            1,
            CpuStop::PortOut {
                port: 0x123,
                size: 1,
                value: 1,
            },
            true,
        ),
        step(1, 1, CpuStop::Halt, false),
    ]);
    machine.pic.acknowledge();
    machine.pic.end_of_interrupt();
    machine.pic.set_irq_line(0, false);
    log.take();
    let shared = machine
        .bus
        .device_mut(0)
        .unwrap()
        .as_any_mut()
        .unwrap()
        .downcast_mut::<SharedPic>()
        .unwrap()
        .clone();
    let completions = std::rc::Rc::new(std::cell::Cell::new(0));
    let completed_at = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    machine.bus = Bus::new(
        vec![
            Box::new(shared.clone()),
            Box::new(DmaTrigger {
                deadline: None,
                queued: false,
                completions: completions.clone(),
                now: VClock::ZERO,
                completed_at: completed_at.clone(),
            }),
            Box::new(devices::Dma8237::new()),
        ],
        DecodeMap {
            ports: vec![PortRange {
                width: bus::PortWidth::default(),
                ports: 0x123..=0x123,
                device: 1,
            }],
            mmio: Vec::new(),
        },
    );
    machine.irq_lines = vec![IrqLine {
        device: 1,
        output: 0,
        irq: 0,
    }];
    machine.last_levels = vec![None];
    machine = machine.with_dma_wirings(vec![DmaWiring {
        requester: 1,
        controller: 2,
    }]);
    let log = shared.observe(machine.bus.observation_clock());
    machine.run_bounded(Some(1)).unwrap();
    assert_eq!(completions.get(), 2);
    assert_eq!(*completed_at.borrow(), [VClock(1), VClock(1)]);
    assert!(machine.cpu.injected.is_empty());
    assert!(log.take().is_empty());
    machine.run_bounded(Some(2)).unwrap();
    assert_eq!(machine.cpu.delivered, [8]);
    assert_eq!(
        log.take(),
        [PicObservation::Acknowledge {
            at: VClock(1),
            vector: 8
        }]
    );
}

#[test]
fn zero_step_counted_stops_return_scheduler_errors() {
    for stop in [
        CpuStop::PortIn {
            port: 0x80,
            size: 1,
        },
        CpuStop::PortOut {
            port: 0x80,
            size: 1,
            value: 0,
        },
        CpuStop::Halt,
    ] {
        let (mut machine, _) = pending_machine(vec![step(1, 0, stop, false)]);
        machine
            .bus
            .set_trace_policy(device::TracePolicy::Ring { capacity: 16 });
        let error = machine.run_bounded(Some(1)).unwrap_err();
        assert!(error
            .to_string()
            .contains("scheduler: counted stop completed no steps"));
        assert_eq!(machine.virtual_time(), 0);
        assert_eq!(machine.clock(), VClock::ZERO);
        assert!(machine.cpu.completions.is_empty());
        assert!(machine.bus.take_open_bus_trace().is_empty());
    }
}

#[test]
fn real_mode_error_prefix_is_charged_independently_of_step_splits() {
    use cpu::x86jit_backend::{RealSegments, X86JitCpu};
    for limit in [1, 2, 64] {
        for engine in [SchedulerEngine::PerClock, SchedulerEngine::Deadline] {
            let mut cpu =
                X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
            cpu.map_ram(0, 0x8000).unwrap();
            // MOV CX, 3; REP STOSB; NOP; MOV AL, [9000h] (unmapped).
            cpu.write_mem(0x7C00, &[0xB9, 3, 0, 0xF3, 0xAA, 0x90, 0xA0, 0, 0x90])
                .unwrap();
            cpu.set_reg(cpu::CpuReg::Rdi, 0x600);
            let mut machine = Machine::new(
                cpu,
                Bus::new(
                    vec![Box::new(devices::VgaText::new())],
                    DecodeMap::default(),
                ),
                Box::new(chipset::NoInterruptController),
                Vec::new(),
                None,
            )
            .with_scheduler_engine(engine)
            .with_step_limit(NonZeroU64::new(limit).unwrap());
            machine
                .bus
                .set_trace_policy(device::TracePolicy::Ring { capacity: 16 });
            assert!(matches!(
                machine.run_bounded(Some(100)),
                Err(MachineError::Cpu(CpuError::Unmapped { addr: 0x9000 }))
            ));
            assert_eq!(machine.virtual_time(), 5, "{engine:?}, limit={limit}");
            assert_eq!(machine.clock(), VClock(5));
            machine.bus.port_write(0xE9, 1, 42).unwrap();
            assert!(matches!(
                machine.bus.take_open_bus_trace()[0],
                device::TraceEvent::Port { at: VClock(5), .. }
            ));
        }
    }
}

#[test]
fn halted_fractional_rate_rounds_up_to_reach_the_cap() {
    // A floor-rounded zero jump must fail without hanging the test process.
    struct BoundedPolls(std::cell::Cell<u8>);
    impl InterruptController for BoundedPolls {
        fn set_irq_line(&mut self, _: u8, _: bool) {}
        fn poll(&self) -> Option<u8> {
            self.0.set(self.0.get() + 1);
            assert!(self.0.get() <= 8, "halted cap made no progress");
            None
        }
        fn acknowledge(&mut self) -> Option<u8> {
            None
        }
        fn end_of_interrupt(&mut self) {}
    }
    let cpu = MockCpu::new(Vec::new())
        .with_interrupts_enabled(true)
        .with_step_program(vec![step(2, 1, CpuStop::Halt, true)]);
    let mut machine = Machine::new(
        cpu,
        Bus::new(Vec::new(), DecodeMap::default()),
        Box::new(BoundedPolls(std::cell::Cell::new(0))),
        Vec::new(),
        None,
    )
    .with_cpu_rate(NonZeroU64::new(2).unwrap(), NonZeroU64::new(3).unwrap());
    machine.set_idle_limit(Some(10));
    assert_eq!(
        machine.run_bounded(Some(2)).unwrap(),
        RunOutcome::RetiredCapReached
    );
    assert_eq!(machine.clock(), VClock(3));
    assert_eq!(machine.virtual_time(), 2);
    assert_eq!(machine.idle_clocks, 2);
    assert_eq!(machine.cpu.retired(), 1);
}

/// Intel 80286 and 80287 Programmer's Reference Manual, §§2.3.2 and 2.3.4:
/// instruction fetch uses CS and the 16-bit IP, independently of host call size.
#[test]
fn real_mode_wrapped_fetch_is_independent_of_step_partitions() {
    use cpu::x86jit_backend::{RealSegments, X86JitCpu};

    for cs in [0, 0x100] {
        for limit in [1, 64, 65_536] {
            let mut cpu =
                X86JitCpu::new_real16_bare(0x30000, cs, 0xFFFE, RealSegments::flat(0)).unwrap();
            cpu.map_ram(0, 0x30000).unwrap();
            let base = u64::from(cs) << 4;
            // Each NOP fits inside CS. Linear bytes beyond CS deliberately
            // differ from the instruction at the wrapped IP.
            cpu.write_mem(base + 0xFFFE, &[0x90, 0x90, 0x43, 0xF4])
                .unwrap();
            cpu.write_mem(base, &[0x42, 0xF4]).unwrap();
            let mut steps = 0;
            loop {
                let run = cpu
                    .run_steps(limit.min(4 - steps), Default::default())
                    .unwrap();
                assert!(run.steps > 0);
                steps += run.steps;
                match run.stop {
                    CpuStepStop::Exit(CpuStop::Halt) => break,
                    CpuStepStop::Exit(CpuStop::BudgetExhausted) => assert!(steps < 4),
                    other => panic!("unexpected stop: {other:?}"),
                }
            }
            assert_eq!(steps, 4);
            assert_eq!(cpu.reg(cpu::CpuReg::Rdx), 1, "CS={cs:x}, limit={limit}");
            assert_eq!(cpu.reg(cpu::CpuReg::Rbx), 0, "CS={cs:x}, limit={limit}");
            assert_eq!(cpu.reg(cpu::CpuReg::Rip), 2, "CS={cs:x}, limit={limit}");
        }
    }
}

/// The same contract for a store into the next instruction of the current
/// block: `mov byte [0x7C06], 0x42` patches the immediate of the following
/// `mov al, 0`. Whatever value the instruction sees, it must not depend on the
/// step budget; the core ends the block after a store into a later
/// instruction of it, so the next instruction sees the new byte (AL = 0x42).
#[test]
fn real_mode_store_into_current_block_is_independent_of_step_partitions() {
    use cpu::x86jit_backend::{RealSegments, X86JitCpu};

    let mut results = Vec::new();
    for limit in [1, 64] {
        let mut cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(0x7C00, &[0xC6, 0x06, 0x06, 0x7C, 0x42, 0xB0, 0x00, 0xF4])
            .unwrap();
        let mut steps = 0;
        loop {
            let run = cpu.run_steps(limit, Default::default()).unwrap();
            steps += run.steps;
            match run.stop {
                CpuStepStop::Exit(CpuStop::Halt) => break,
                CpuStepStop::Exit(CpuStop::BudgetExhausted) => assert!(steps < 3),
                other => panic!("unexpected stop: {other:?}"),
            }
        }
        assert_eq!(steps, 3);
        results.push(cpu.reg(cpu::CpuReg::Rax) & 0xFF);
    }
    assert_eq!(
        results[0], results[1],
        "store into the current block: AL depends on the step budget"
    );
}

/// A rejected access can leave DMA work that cannot settle. The access error
/// remains the useful diagnostic even when follow-on work also fails.
#[derive(Default)]
struct FaultingAccess {
    active: bool,
}

impl Device for FaultingAccess {
    fn name(&self) -> &'static str {
        "faulting-access"
    }
    fn port_read(&mut self, port: u16, _: u8) -> Result<u32, device::DeviceError> {
        self.active = true;
        Err(device::DeviceError::UnmappedRegister {
            kind: "port",
            offset: u64::from(port),
        })
    }
    fn port_write(&mut self, port: u16, size: u8, _: u32) -> Result<(), device::DeviceError> {
        self.port_read(port, size).map(|_| ())
    }
    fn mmio_read(&mut self, addr: u64, _: u8) -> Result<u64, device::DeviceError> {
        self.active = true;
        Err(device::DeviceError::UnmappedRegister {
            kind: "mmio",
            offset: addr,
        })
    }
    fn mmio_write(&mut self, addr: u64, size: u8, _: u64) -> Result<(), device::DeviceError> {
        self.mmio_read(addr, size).map(|_| ())
    }
    fn tick(&mut self, _: VClock) {}
    fn next_event(&self, _: VClock) -> NextEvent {
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn reset(&mut self) {
        self.active = false;
    }
    fn save(&self) -> Vec<u8> {
        vec![u8::from(self.active)]
    }
    fn restore(&mut self, snapshot: &[u8]) -> Result<(), device::DeviceError> {
        self.active = snapshot[0] != 0;
        Ok(())
    }
    fn take_dma_request(&mut self) -> Option<device::DmaRequest> {
        self.active.then(|| device::DmaRequest {
            channel: 2,
            op: DmaOp::ToMemory(vec![]),
        })
    }
}

#[test]
fn access_error_survives_follow_on_settle_failure() {
    for stop in [
        CpuStop::PortIn {
            port: 0x123,
            size: 1,
        },
        CpuStop::PortOut {
            port: 0x123,
            size: 1,
            value: 0,
        },
        CpuStop::MmioRead {
            addr: 0x123,
            size: 1,
        },
        CpuStop::MmioWrite {
            addr: 0x123,
            size: 1,
            value: 0,
        },
    ] {
        let counted = matches!(stop, CpuStop::PortIn { .. } | CpuStop::PortOut { .. });
        let cpu = MockCpu::new(Vec::new()).with_step_program(vec![step(
            1,
            u64::from(counted),
            stop,
            false,
        )]);
        let bus = Bus::new(
            vec![Box::<FaultingAccess>::default()],
            DecodeMap {
                ports: vec![PortRange {
                    width: bus::PortWidth::default(),
                    ports: 0x123..=0x123,
                    device: 0,
                }],
                mmio: vec![bus::MmioRange {
                    addrs: 0x123..=0x123,
                    device: 0,
                }],
            },
        );
        let mut machine = Machine::new(
            cpu,
            bus,
            Box::new(chipset::NoInterruptController),
            vec![],
            None,
        )
        .with_step_limit(NonZeroU64::new(1).unwrap())
        .with_dma_wirings(vec![DmaWiring {
            requester: 0,
            controller: 0,
        }]);
        assert!(matches!(
            machine.run_bounded(Some(2)),
            Err(MachineError::Bus(BusError::Device(
                device::DeviceError::UnmappedRegister { offset: 0x123, .. }
            )))
        ));
    }
}

#[test]
fn counter_exhaustion_precedes_an_unexecuted_access() {
    for engine in [SchedulerEngine::PerClock, SchedulerEngine::Deadline] {
        for stop in [
            CpuStop::PortIn {
                port: 0x123,
                size: 1,
            },
            CpuStop::PortOut {
                port: 0x123,
                size: 1,
                value: 0,
            },
        ] {
            let cpu = MockCpu::new(Vec::new()).with_step_program(vec![step(1, 1, stop, false)]);
            let bus = Bus::new(vec![], DecodeMap::default())
                .with_undecoded_ports(bus::UndecodedPortAccess::Fault);
            let mut machine = Machine::new(
                cpu,
                bus,
                Box::new(chipset::NoInterruptController),
                vec![],
                None,
            )
            .with_step_limit(NonZeroU64::new(1).unwrap())
            .with_scheduler_engine(engine);
            machine.clock = VClock(u64::MAX);
            let result = machine.run_bounded(None);
            assert!(
                matches!(
                    result,
                    Err(MachineError::Cpu(CpuError::UnexpectedExit { .. }))
                ),
                "{engine:?}: {result:?}"
            );
            assert_eq!(machine.cpu.completed_steps(), 0);
        }
    }
}

#[test]
fn dma_deadline_inside_a_multi_clock_cpu_step_completes_before_clock_advances() {
    let completed_at = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let cpu = MockCpu::new(Vec::new()).with_step_program(vec![step(
        1,
        1,
        CpuStop::BudgetExhausted,
        false,
    )]);
    let mut controller = devices::Dma8237::new();
    for (port, byte) in [
        (0x0C, 0),
        (4, 0),
        (4, 0),
        (5, 1),
        (5, 0),
        (0x0B, 0x46),
        (0x0A, 2),
    ] {
        controller.port_write(port, 1, byte).unwrap();
    }
    let bus = Bus::new(
        vec![
            Box::new(DmaTrigger {
                deadline: Some(VClock(2)),
                queued: false,
                completions: Default::default(),
                now: VClock::ZERO,
                completed_at: completed_at.clone(),
            }),
            Box::new(controller),
        ],
        DecodeMap::default(),
    );
    let mut machine = Machine::new(
        cpu,
        bus,
        Box::new(chipset::NoInterruptController),
        Vec::new(),
        None,
    )
    .with_cpu_rate(NonZeroU64::new(1).unwrap(), NonZeroU64::new(10).unwrap())
    .with_dma_wirings(vec![DmaWiring {
        requester: 0,
        controller: 1,
    }]);
    machine.run_bounded(Some(1)).unwrap();
    assert_eq!(machine.clock(), VClock(10));
    assert_eq!(*completed_at.borrow(), [VClock(2), VClock(2)]);
    let controller = machine.bus.device_mut(1).unwrap();
    assert_eq!(controller.port_read(8, 1).unwrap() & 4, 4);
    assert_eq!(controller.port_read(4, 1).unwrap(), 2);
}
