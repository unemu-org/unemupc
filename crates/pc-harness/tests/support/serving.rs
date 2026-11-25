// SPDX-License-Identifier: AGPL-3.0-or-later
//! One workload run twice in lockstep, with the machine serving the ranges
//! in `machine::SERVED_RANGES` while the CPU runs and with every access
//! returning from the CPU (`backlog/docs/design/scheduler.md` §In-process
//! device access). At every poll both must report the same outcome, retired count,
//! completed steps, virtual time and device clock, the same observed bytes,
//! the same display frames and the same device traces (every register
//! access with its virtual time); at the end the same device snapshots, RAM
//! and processor state. The host time each way spends running is printed.

use std::time::{Duration, Instant};

use assembly::{AssembledMachine, DeviceId, RunPolicy};
use cpu::Cpu;
use device::{ByteStreamKind, TracePolicy};
use machine::RunOutcome;

/// Device trace ring size; both ways drain it at every poll.
const TRACE_RING: usize = 1 << 18;

/// What one lockstep run polls and compares.
pub struct Lockstep {
    pub label: &'static str,
    /// Virtual-time steps between polls.
    pub step: u64,
    /// Virtual-time cap.
    pub cap: u64,
    /// Halted PIT clocks before a halted guest counts as quiescent.
    pub idle_limit: Option<u64>,
    /// Byte streams drained and compared at every poll.
    pub outputs: Vec<(DeviceId, ByteStreamKind)>,
    /// Displays captured and compared at every poll.
    pub displays: Vec<DeviceId>,
    /// Attach host-profile counters to each way, labelled; the guard
    /// reports when it drops, after the comparison.
    pub profile: Option<Profile>,
}

/// Attaches a host profile to one way of the run.
pub type Profile = fn(&mut AssembledMachine, String) -> Box<dyn std::any::Any>;

/// The served run's machine after the comparison, with what both saw.
/// Each golden reads the parts it pins.
#[allow(dead_code)]
pub struct Compared {
    pub served: AssembledMachine,
    pub outcome: RunOutcome,
    pub outputs: Vec<Vec<u8>>,
    pub polls: u64,
    /// What the served way served; never zero, so the run cannot pass
    /// without serving anything.
    pub served_accesses: machine::ServedAccesses,
}

struct Side {
    assembled: AssembledMachine,
    elapsed: Duration,
}

impl Side {
    fn new(mut assembled: AssembledMachine, in_process: bool) -> Self {
        assembled.machine.set_in_process_devices(in_process);
        assembled
            .machine
            .bus_mut()
            .set_trace_policy(TracePolicy::Ring {
                capacity: TRACE_RING,
            });
        Self {
            assembled,
            elapsed: Duration::ZERO,
        }
    }

    fn run(&mut self, policy: &RunPolicy) -> Result<RunOutcome, String> {
        let started = Instant::now();
        let outcome = self
            .assembled
            .run_session(policy)
            .map_err(|error| error.to_string());
        self.elapsed += started.elapsed();
        outcome
    }

    fn counters(&self) -> (u64, u64, u64, device::VClock) {
        let machine = &self.assembled.machine;
        (
            machine.cpu().retired(),
            machine.cpu().completed_steps(),
            machine.virtual_time(),
            machine.clock(),
        )
    }

    fn traces(&mut self) -> Vec<(Vec<device::TraceEvent>, u64)> {
        let bus = self.assembled.machine.bus_mut();
        let mut traces = vec![(bus.take_open_bus_trace(), bus.open_bus_trace_dropped())];
        let mut index = 0;
        while let Some(device) = bus.device_mut(index) {
            traces.push((device.take_trace(), device.trace_dropped()));
            index += 1;
        }
        traces
    }

    #[allow(clippy::type_complexity)]
    fn snapshot(
        &mut self,
    ) -> (
        Vec<Vec<u8>>,
        Vec<Option<Vec<u8>>>,
        cpu::CpuDiagnostic,
        Option<machine::RecentDeliveries>,
    ) {
        let bus = self.assembled.machine.bus_mut();
        bus.settle();
        let devices = (0..)
            .map_while(|index| bus.device(index).map(|device| device.save()))
            .collect();
        // Every 4 KiB page of the first 16 MiB the CPU can read.
        let cpu = self.assembled.machine.cpu();
        let ram = (0..4096u64)
            .map(|page| {
                let mut bytes = vec![0; 4096];
                cpu.read_mem(page << 12, &mut bytes).ok().map(|()| bytes)
            })
            .collect();
        (
            devices,
            ram,
            cpu.diagnostic_state(),
            self.assembled.machine.recent_deliveries(),
        )
    }
}

/// Run `build`'s machine both ways until `stop` holds after a poll, the run
/// stops short of its target or the cap is reached, comparing everything
/// listed in the module documentation.
pub fn compare(
    spec: &Lockstep,
    build: impl Fn() -> AssembledMachine,
    stop: impl Fn(&AssembledMachine) -> bool,
) -> Compared {
    let mut served = Side::new(build(), true);
    let mut exits = Side::new(build(), false);
    let profiles: Vec<_> = spec
        .profile
        .iter()
        .flat_map(|attach| {
            [
                attach(&mut served.assembled, format!("{} served", spec.label)),
                attach(&mut exits.assembled, format!("{} returning", spec.label)),
            ]
        })
        .collect();
    let mut outputs = vec![Vec::new(); spec.outputs.len()];
    let mut polls = 0;
    let outcome = loop {
        let target = (served.assembled.machine.virtual_time() + spec.step).min(spec.cap);
        let policy = RunPolicy::natural(Some(target), spec.idle_limit);
        let outcome = served.run(&policy);
        assert_eq!(outcome, exits.run(&policy), "{}: poll {polls}", spec.label);
        polls += 1;
        assert_eq!(
            served.counters(),
            exits.counters(),
            "{}: poll {polls}",
            spec.label
        );
        for (index, &(device, kind)) in spec.outputs.iter().enumerate() {
            let bytes = served.assembled.observation.drain_bytes(device, kind);
            let reference = exits.assembled.observation.drain_bytes(device, kind);
            let bytes = bytes.expect("observed output");
            assert_eq!(bytes, reference.expect("observed output"), "{}", spec.label);
            outputs[index].extend(bytes);
        }
        for &display in &spec.displays {
            assert!(
                served.assembled.capture_text(display).unwrap().frame
                    == exits.assembled.capture_text(display).unwrap().frame,
                "{}: {display:?} frame at poll {polls}",
                spec.label
            );
        }
        assert!(
            served.traces() == exits.traces(),
            "{}: device traces at poll {polls}",
            spec.label
        );
        match outcome {
            Ok(RunOutcome::RetiredCapReached) if target < spec.cap && !stop(&served.assembled) => {}
            Ok(outcome) => break outcome,
            Err(error) => panic!("{}: {error}", spec.label),
        }
    };
    assert!(
        served.snapshot() == exits.snapshot(),
        "{}: final devices, RAM or processor state",
        spec.label
    );
    eprintln!(
        "{}: {outcome:?} after {polls} polls, retired={}, served {:.3}s, every access returning {:.3}s",
        spec.label,
        served.assembled.machine.cpu().retired(),
        served.elapsed.as_secs_f64(),
        exits.elapsed.as_secs_f64(),
    );
    drop(profiles);
    let served_accesses = served.assembled.machine.served_accesses();
    eprintln!("{}: served {served_accesses:?}", spec.label);
    assert_eq!(
        exits.assembled.machine.served_accesses(),
        machine::ServedAccesses::default(),
        "{}: the returning way served accesses",
        spec.label
    );
    assert!(
        served_accesses.ports + served_accesses.mmio > 0,
        "{}: nothing was served",
        spec.label
    );
    Compared {
        served: served.assembled,
        outcome,
        outputs,
        polls,
        served_accesses,
    }
}
