// SPDX-License-Identifier: AGPL-3.0-or-later
//! The diagnostic report and its code trail change nothing the guest does
//! (`backlog/docs/design/observation.md` §Diagnostic report of a running
//! machine).
//!
//! The open-BIOS FreeDOS boot of `freedos_boot_golden` runs on the
//! reference `at-freedos` machine, polling for FreeCOM's date prompt in the
//! same one-million-step chunks as `pc_harness::run_freedos_config`: plain,
//! as the golden runs it; with the code trail and the report's delivery
//! log on (the desktop keeps the trail for every machine, the log from its
//! first report or Record exceptions); and with those plus an
//! exception recording and a full diagnostic report taken at every poll
//! boundary (`backlog/docs/design/observation.md` §Exception and interrupt
//! deliveries). Every run must stop at the same retired count and virtual
//! time, show the golden's MDA screen, emit the same POST stream and leave
//! every device with the same snapshot; the two recordings are the same
//! text. The plain and the trail-and-log boots run twice, interleaved, and
//! the wall time of each run is printed. The cost on a load heavy in
//! interrupts is measured by `assembly::diagnostic`'s
//! `the_delivery_log_cost_on_a_high_irq_load`.
//!
//! Marked `#[ignore]` because it boots FreeDOS six times.

use assembly::{
    AssembledMachine, DeviceId, ObservationPolicy, ObservationRequest, OutputKind, RunPolicy,
};
use video::Frame;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/host_profile.rs"]
mod host_profile;
#[path = "support/serving.rs"]
mod serving;
#[path = "support/session.rs"]
#[allow(dead_code)]
mod session;

struct Boot {
    retired: u64,
    virtual_time: u64,
    mda: String,
    post: Vec<u8>,
    devices: Vec<Vec<u8>>,
    reports: usize,
    trail_samples: u64,
    /// The exception trace with its summary; empty without the trace.
    trace: String,
    elapsed: std::time::Duration,
}

fn assembled() -> AssembledMachine {
    let config = machine_file::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../machines/at-freedos.toml"),
    )
    .unwrap();
    let mut policy = ObservationPolicy {
        trace: device::TracePolicy::Off,
        ..Default::default()
    };
    for device in [DeviceId::Vga, DeviceId::Mda] {
        policy.routes.push(ObservationRequest {
            device,
            output: OutputKind::TextScanout,
            byte_limit: 0,
        });
    }
    for (device, kind) in [
        (DeviceId::Uart, device::ByteStreamKind::SerialTx),
        (DeviceId::Post, device::ByteStreamKind::PostCheckpoints),
    ] {
        policy.routes.push(ObservationRequest {
            device,
            output: OutputKind::Bytes(kind),
            byte_limit: 1024 * 1024,
        });
    }
    machine_file::assemble_with_observation(&config, &policy).unwrap()
}

fn mda(assembled: &AssembledMachine) -> String {
    let Frame::Text(grid) = assembled.capture_text(DeviceId::Mda).unwrap().frame else {
        panic!("MDA text required")
    };
    grid.to_dump()
}

/// Boot to the date prompt as `run_freedos_config` polls for it; with
/// `diagnose`, keep the code trail and the report's delivery log; with
/// `record`, also record the deliveries and take a report at every poll
/// boundary.
fn boot(diagnose: bool, record: bool) -> Boot {
    let mut assembled = assembled();
    if diagnose {
        assembled.machine.enable_code_trail();
        assembled.enable_delivery_log().unwrap();
    }
    if record {
        assembled.start_delivery_recording().unwrap();
    }
    let started = std::time::Instant::now();
    let mut reports = 0;
    loop {
        let target = assembled.machine.virtual_time() + pc_harness::DOS_INTERACTIVE_POLL_STEP;
        let cap = target.min(pc_harness::BIOS_MAX_RETIRED);
        let outcome = assembled
            .run_session(&RunPolicy::natural(
                Some(cap),
                Some(pc_harness::BIOS_IDLE_LIMIT),
            ))
            .unwrap();
        if record {
            let report = assembled.diagnostic_report("running").to_string();
            assert!(
                report.contains("\ntrail    cs:ip after the last "),
                "{report}"
            );
            reports += 1;
        }
        if outcome == machine::RunOutcome::Halted
            || mda(&assembled).contains(pc_harness::DOS_DATE_PROMPT)
            || cap >= pc_harness::BIOS_MAX_RETIRED
        {
            break;
        }
    }
    let elapsed = started.elapsed();
    if diagnose {
        let report = assembled.diagnostic_report("running").to_string();
        assert!(report.contains("\ndelivery the last "), "{report}");
    }
    let text = assembled
        .machine
        .stop_delivery_recording()
        .unwrap_or_default();
    let capture = assembled.capture_text(DeviceId::Vga).unwrap();
    let post = assembled
        .observation
        .drain_bytes(DeviceId::Post, device::ByteStreamKind::PostCheckpoints)
        .unwrap();
    let mda = mda(&assembled);
    let trail_samples = assembled
        .machine
        .code_trail()
        .map_or(0, machine::CodeTrail::samples);
    let bus = assembled.machine.bus_mut();
    bus.settle();
    let devices = (0..)
        .map_while(|index| bus.device(index).map(|device| device.save()))
        .collect();
    Boot {
        retired: capture.retired,
        virtual_time: capture.virtual_time,
        mda,
        post,
        devices,
        reports,
        trail_samples,
        trace: text,
        elapsed,
    }
}

#[ignore = "slow: boots FreeDOS six times on the open BIOS"]
#[test]
fn the_code_trail_and_reports_leave_the_freedos_boot_golden_unchanged() {
    let (Some(_rom), Some(_img)) = (
        fixtures::fixture("kiselev-bios/bios-xi8088.bin"),
        fixtures::fixture("freedos/freedos.img"),
    ) else {
        return;
    };
    let plain = boot(false, false);
    let diagnosed = boot(true, false);
    let plain_again = boot(false, false);
    let diagnosed_again = boot(true, false);
    let recorded = boot(true, true);
    let again = boot(true, true);
    eprintln!(
        "plain boot {:?} and {:?}; with the trail and the delivery log {:?} and {:?} \
         ({} trail samples); with a recording and {} reports too {:?} and {:?}",
        plain.elapsed,
        plain_again.elapsed,
        diagnosed.elapsed,
        diagnosed_again.elapsed,
        diagnosed.trail_samples,
        recorded.reports,
        recorded.elapsed,
        again.elapsed
    );
    eprint!(
        "{}",
        recorded
            .trace
            .lines()
            .filter(|line| line.starts_with("summary  "))
            .map(|line| format!("{line}\n"))
            .collect::<String>()
    );
    let runs = [
        &plain,
        &diagnosed,
        &plain_again,
        &diagnosed_again,
        &recorded,
        &again,
    ];
    // The numbers of `freedos_boot_golden`.
    for run in runs {
        assert_eq!((run.retired, run.virtual_time), (8_800_473, 10_000_000));
        assert_eq!(session::screen_hash(&run.mda), 0x83ffbc2844f28889);
        assert_eq!(session::logical_hash(&run.mda), 0xc46f0c147c881837);
        assert_eq!(plain.post, run.post);
        assert_eq!(plain.devices, run.devices);
    }
    assert!(plain.trace.is_empty() && diagnosed.trace.is_empty());
    assert!(
        recorded.trace.starts_with("trace    "),
        "{}",
        recorded.trace
    );
    assert!(recorded.trace.contains("\ndelivery "), "{}", recorded.trace);
    assert!(recorded.trace.contains("\nsummary  "), "{}", recorded.trace);
    assert_eq!(recorded.trace, again.trace);
    assert_eq!(recorded.reports, 10);
    assert!(diagnosed.trail_samples > 0);
}

/// The open-BIOS FreeDOS boot with the machine's served ports and the
/// display window served in process and with every access returning,
/// polled as
/// `run_freedos_config` polls it: the same machine, screens, POST stream,
/// serial output and traces at every poll up to FreeCOM's date prompt.
#[ignore = "slow: boots FreeDOS twice on the open BIOS, in lockstep"]
#[test]
fn serving_in_process_leaves_the_freedos_boot_golden_unchanged() {
    let (Some(_rom), Some(_img)) = (
        fixtures::fixture("kiselev-bios/bios-xi8088.bin"),
        fixtures::fixture("freedos/freedos.img"),
    ) else {
        return;
    };
    let compared = serving::compare(
        &serving::Lockstep {
            label: "freedos boot",
            step: pc_harness::DOS_INTERACTIVE_POLL_STEP,
            cap: pc_harness::BIOS_MAX_RETIRED,
            idle_limit: Some(pc_harness::BIOS_IDLE_LIMIT),
            outputs: vec![
                (DeviceId::Uart, device::ByteStreamKind::SerialTx),
                (DeviceId::Post, device::ByteStreamKind::PostCheckpoints),
            ],
            displays: vec![DeviceId::Vga, DeviceId::Mda],
            profile: Some(|rig, label| {
                Box::new(host_profile::WorkloadProfile::from_env(rig, || label))
            }),
            inject: None,
            audio: false,
        },
        assembled,
        |assembled| mda(assembled).contains(pc_harness::DOS_DATE_PROMPT),
    );
    // The numbers of `freedos_boot_golden`.
    let served = &compared.served.machine;
    assert_eq!(compared.outcome, machine::RunOutcome::RetiredCapReached);
    // The BIOS's port 61h refresh polling and 8042 status reads are served.
    assert!(compared.served_accesses.ports > 800_000 && compared.served_accesses.mmio > 0);
    assert_eq!(
        (cpu::Cpu::retired(served.cpu()), served.virtual_time()),
        (8_800_473, 10_000_000)
    );
    assert_eq!(
        session::screen_hash(&mda(&compared.served)),
        0x83ffbc2844f28889
    );
}
