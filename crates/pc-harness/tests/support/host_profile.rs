// SPDX-License-Identifier: AGPL-3.0-or-later
//! Opt-in host counters shared by whole-machine workload tests. These callbacks
//! never supply guest time; see `backlog/docs/status.md` §Speed.

#[cfg(feature = "host-profile")]
pub use enabled::*;

#[cfg(not(feature = "host-profile"))]
pub struct WorkloadProfile;

#[cfg(not(feature = "host-profile"))]
impl WorkloadProfile {
    pub fn from_env(
        _: &mut assembly::AssembledMachine,
        _: impl FnOnce() -> String,
    ) -> Option<Self> {
        None
    }

    #[allow(dead_code)]
    pub fn checkpoint(&self, _: &str) {}
}

#[cfg(feature = "host-profile")]
mod enabled {
    use device::profile::{HostProfiler, ProfileEvent, ProfileExit, ProfileHook, ProfilePhase};
    use std::{cell::RefCell, rc::Rc, time::Instant};

    #[derive(Clone, Debug, Default)]
    pub struct Counts {
        pub nanos: [u128; 6],
        pub calls: u64,
        pub retired: u64,
        pub steps: u64,
        pub budget: u64,
        pub single: u64,
        pub exits: [u64; 7],
        pub events: [u64; 6],
    }

    struct State {
        counts: Counts,
        stack: Vec<ProfilePhase>,
        last: Instant,
    }

    impl State {
        fn charge(&mut self) {
            let now = Instant::now();
            if let Some(phase) = self.stack.last() {
                self.counts.nanos[*phase as usize] += now.duration_since(self.last).as_nanos();
            }
            self.last = now;
        }
    }

    pub struct Collector(RefCell<State>);

    impl Collector {
        pub fn new() -> Rc<Self> {
            Rc::new(Self(RefCell::new(State {
                counts: Counts::default(),
                stack: Vec::with_capacity(16),
                last: Instant::now(),
            })))
        }

        pub fn counts(&self) -> Counts {
            let mut state = self.0.borrow_mut();
            state.charge();
            state.counts.clone()
        }
    }

    impl HostProfiler for Collector {
        fn enter(&self, phase: ProfilePhase) {
            let mut state = self.0.borrow_mut();
            state.charge();
            state.stack.push(phase);
        }

        fn leave(&self, phase: ProfilePhase) {
            let mut state = self.0.borrow_mut();
            state.charge();
            assert_eq!(state.stack.pop(), Some(phase));
        }

        fn cpu_run(&self, retired: u64, steps: u64, budget: u64, exit: ProfileExit) {
            let mut state = self.0.borrow_mut();
            let counts = &mut state.counts;
            counts.calls += 1;
            counts.retired += retired;
            counts.steps += steps;
            counts.budget += budget;
            counts.single += u64::from(budget == 1);
            counts.exits[exit as usize] += 1;
        }

        fn event(&self, event: ProfileEvent) {
            self.0.borrow_mut().counts.events[event as usize] += 1;
        }
    }

    pub struct WorkloadProfile {
        collector: Rc<Collector>,
        label: String,
        started: Instant,
    }

    impl WorkloadProfile {
        /// Absent or zero disables all callbacks; an explicit API is used by tests.
        pub fn from_env(
            rig: &mut assembly::AssembledMachine,
            label: impl FnOnce() -> String,
        ) -> Option<Self> {
            Self::from_switch(
                rig,
                std::env::var("UNEMUPC_HOST_PROFILE").as_deref() == Ok("1"),
                label,
            )
        }

        fn from_switch(
            rig: &mut assembly::AssembledMachine,
            enabled: bool,
            label: impl FnOnce() -> String,
        ) -> Option<Self> {
            enabled.then(|| Self::attach(rig, label()))
        }

        pub fn attach(rig: &mut assembly::AssembledMachine, label: String) -> Self {
            eprintln!("HOST PROFILE START {label}: pid={}", std::process::id());
            let collector = Collector::new();
            collector.enter(ProfilePhase::Observation);
            rig.machine
                .bus_mut()
                .set_profile(ProfileHook::new(collector.clone()));
            Self {
                collector,
                label,
                started: Instant::now(),
            }
        }
    }

    impl WorkloadProfile {
        pub fn checkpoint(&self, suffix: &str) {
            self.report(suffix);
        }

        fn report(&self, suffix: &str) {
            let c = self.collector.counts();
            eprintln!("HOST PROFILE {}{suffix}: wall_s={:.6} nanos(cpu,scheduler,port,mmio,frame,observation)={:?} calls={} retired={} steps={} mean_retired={:.3} mean_budget={:.3} budget_one={} exits(port,mmio,irq,hlt,budget,other,error)={:?} events(deadline,tick_due,device_tick,halted_advance,device_sync,dma_probe)={:?}",
            self.label, self.started.elapsed().as_secs_f64(), c.nanos, c.calls,
            c.retired, c.steps, c.retired as f64 / c.calls.max(1) as f64,
            c.budget as f64 / c.calls.max(1) as f64, c.single, c.exits, c.events);
        }
    }

    impl Drop for WorkloadProfile {
        fn drop(&mut self) {
            self.collector.leave(ProfilePhase::Observation);
            self.report("");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use cpu::Cpu;

        #[test]
        fn profiling_preserves_cpu_memory_devices_trace_and_frame() {
            let run = |enabled: bool| {
                let program = vec![
                    0xFA, // CLI keeps this synthetic probe independent of timer IRQs.
                    0xC7, 0x06, 0, 0, 0x41, 7, // MOV WORD [DS:0],0741h (MMIO)
                    0xA1, 0, 0, // MOV AX,[DS:0]
                    0xE6, 0x80, 0xE4, 0x81, // OUT / IN
                    0xB9, 20, 0, 0xE2, 0xFE, // LOOP
                    0xF4,
                ];
                let policy = assembly::ObservationPolicy {
                    trace: device::TracePolicy::Ring { capacity: 128 },
                    routes: vec![assembly::ObservationRequest {
                        device: assembly::DeviceId::Vga,
                        output: assembly::OutputKind::TextScanout,
                        byte_limit: 0,
                    }],
                    ..Default::default()
                };
                let mut rig =
                    assembly::assemble_with_observation(&assembly::at_vga_text(program), &policy)
                        .unwrap();
                rig.machine = rig
                    .machine
                    .with_step_limit(std::num::NonZeroU64::new(3).unwrap());
                assert!(!rig.machine.bus().profile().enabled());
                let profile = enabled.then(|| WorkloadProfile::attach(&mut rig, "identity".into()));
                assert_eq!(
                    rig.machine.run_bounded(Some(100)).unwrap(),
                    machine::RunOutcome::Halted
                );
                let frame = rig.capture_text(assembly::DeviceId::Vga).unwrap();
                let mut ram = vec![0; 0x10000];
                rig.machine.cpu().read_mem(0, &mut ram).unwrap();
                let mut hardware = Vec::new();
                let mut trace = rig.machine.bus_mut().take_open_bus_trace();
                for index in 0..rig.device_index.len() {
                    let dev = rig.machine.bus_mut().device_mut(index).unwrap();
                    hardware.push(dev.save());
                    trace.extend(dev.take_trace());
                }
                let state = (
                    rig.machine.cpu().diagnostic_state(),
                    rig.machine.cpu().retired(),
                    rig.machine.virtual_time(),
                    rig.machine.clock(),
                    ram,
                    hardware,
                    trace,
                    frame,
                );
                if let Some(profile) = profile {
                    profile.checkpoint(" checkpoint");
                    let c = profile.collector.counts();
                    assert!(c.calls > 10);
                    assert_eq!(c.retired, rig.machine.cpu().retired());
                    assert_eq!(c.steps, rig.machine.cpu().completed_steps());
                    assert_eq!(c.exits.iter().sum::<u64>(), c.calls);
                    for exit in [
                        ProfileExit::PortIo,
                        ProfileExit::Mmio,
                        ProfileExit::Halt,
                        ProfileExit::Budget,
                    ] {
                        assert!(c.exits[exit as usize] > 0, "{exit:?}");
                    }
                    for phase in [
                        ProfilePhase::Cpu,
                        ProfilePhase::Scheduler,
                        ProfilePhase::PortIo,
                        ProfilePhase::Mmio,
                        ProfilePhase::Frame,
                        ProfilePhase::Observation,
                    ] {
                        assert!(c.nanos[phase as usize] > 0, "{phase:?}");
                    }
                }
                state
            };
            assert_eq!(run(false), run(true));
        }

        #[test]
        fn disabled_workload_does_not_build_label() {
            let mut rig = assembly::assemble(&assembly::at_vga_text(vec![0xf4])).unwrap();
            assert!(WorkloadProfile::from_switch(&mut rig, false, || panic!(
                "disabled label constructed"
            ))
            .is_none());
        }

        #[test]
        fn nested_spans_do_not_double_count_time() {
            let collector = Collector::new();
            let hook = ProfileHook::new(collector.clone());
            let start = Instant::now();
            {
                let _scheduler = hook.span(ProfilePhase::Scheduler);
                let _port = hook.span(ProfilePhase::PortIo);
                let _recursive_port = hook.span(ProfilePhase::PortIo);
            }
            let c = collector.counts();
            assert!(c.nanos[ProfilePhase::PortIo as usize] > 0);
            assert!(c.nanos.iter().sum::<u128>() <= start.elapsed().as_nanos());
            assert!(collector.0.borrow().stack.is_empty());
        }
    }
}

#[cfg(all(test, not(feature = "host-profile")))]
#[test]
fn disabled_workload_does_not_build_label() {
    let mut rig = assembly::assemble(&assembly::at_vga_text(vec![0xf4])).unwrap();
    assert!(WorkloadProfile::from_env(&mut rig, || panic!("disabled label constructed")).is_none());
}
