// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scheduler migration observations for the PC/AT goldens (§10).
//!
//! See `backlog/docs/design/scheduler.md` § Migration and § Review outcome.
//! This stays in CLI test support: it can reuse frontend programs and assembly
//! presets without making either a dependency of the lower-level testkit crate.
//! Device traces are drained at each transaction, preserving equal-time order
//! across devices and Intel 8259A INTA/EOI (datasheet, "Interrupt Sequence" and
//! "End of Interrupt"). Sorting completed per-device traces would lose that order.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fmt,
    rc::Rc,
};

use anyhow::Result;
use assembly::{AssembledMachine, ConsoleProbe, DeviceId, RunPolicy, StopCondition};
use bus::{Bus, DecodeMap, PortRange};
use chipset::{NoInterruptController, PicObservation, PicObservationLog, SharedPic};
use cpu::{Cpu, CpuReg, X86JitCpu};
use device::{
    Device, DeviceError, DmaMemory, DmaOutcome, DmaRequest, NextEvent, TraceEvent, VClock,
};
use devices::{Uart16550, I8042};
use machine::{Machine, RunOutcome};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Halted,
    ResetRequested,
    CapReached,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Access {
        device: DeviceId,
        access: TraceEvent,
    },
    Pic(PicObservation),
    FinalState {
        ram: Vec<(u64, u64, u64)>,
        devices: Vec<(DeviceId, String, u64)>,
        registers: Vec<(CpuReg, u64)>,
        code_segment: u64,
    },
    Stop {
        at: VClock,
        virtual_time: u64,
        reason: Stop,
    },
}

pub type Stream = Vec<Event>;

const SERIAL_REAL16_PROGRAM: &[u8] = &[
    0xBA, 0xF8, 0x03, 0xB0, b'h', 0xEE, 0xB0, b'e', 0xEE, 0xB0, b'l', 0xEE, 0xB0, b'l', 0xEE, 0xB0,
    b'o', 0xEE, 0xF4,
];

/// Both final cap times must be in this inclusive, explicitly reviewed window.
#[derive(Clone, Copy, Debug)]
pub struct CapEndpoint {
    pub cap: u64,
    pub max_overshoot: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Normalisations {
    /// Add to right-hand access times and their OCW2 observations. INTA never
    /// shifts; no offset is inferred.
    pub right_access_shift: i64,
    /// Historical migration evidence: MMIO already stopped before retirement. Override its
    /// shift when ports move to instruction start but MMIO times stay put.
    pub right_mmio_shift: Option<i64>,
    /// Never removes trailing accesses or normalises a halt/error into a cap.
    pub cap_endpoint: Option<CapEndpoint>,
}

fn access_time(event: &Event) -> Option<u64> {
    match event {
        Event::Access {
            access: TraceEvent::Port { at, .. } | TraceEvent::Mmio { at, .. },
            ..
        } => Some(at.0),
        _ => None,
    }
}

fn equivalent(left: &Event, right: &Event, options: Normalisations, final_pair: bool) -> bool {
    if let (Some(left_time), Some(right_time)) = (access_time(left), access_time(right)) {
        let mut shifted = right.clone();
        if let Event::Access {
            access: TraceEvent::Port { at, .. } | TraceEvent::Mmio { at, .. },
            ..
        } = &mut shifted
        {
            *at = VClock(left_time);
        }
        let shift = if matches!(
            right,
            Event::Access {
                access: TraceEvent::Mmio { .. },
                ..
            }
        ) {
            options
                .right_mmio_shift
                .unwrap_or(options.right_access_shift)
        } else {
            options.right_access_shift
        };
        return i128::from(left_time) == i128::from(right_time) + i128::from(shift)
            && left == &shifted;
    }
    // An OCW2 observation belongs to its OUT transaction. Shift it together
    // with that access, while INTA retains its actual acceptance timestamp.
    if let (
        Event::Pic(PicObservation::Eoi {
            at: la,
            command: lc,
            value: Some(lv),
        }),
        Event::Pic(PicObservation::Eoi {
            at: ra,
            command: rc,
            value: Some(rv),
        }),
    ) = (left, right)
    {
        return lc == rc
            && lv == rv
            && i128::from(la.0) == i128::from(ra.0) + i128::from(options.right_access_shift);
    }
    if final_pair {
        if let (
            Some(window),
            Event::Stop {
                at: la,
                virtual_time: lt,
                reason: Stop::CapReached,
            },
            Event::Stop {
                at: ra,
                virtual_time: rt,
                reason: Stop::CapReached,
            },
        ) = (options.cap_endpoint, left, right)
        {
            let in_window =
                |time: u64| time >= window.cap && time - window.cap <= window.max_overshoot;
            return left == right
                || (in_window(la.0) && in_window(ra.0) && in_window(*lt) && in_window(*rt));
        }
    }
    left == right
}

#[derive(Debug)]
pub struct Difference {
    pub index: usize,
    pub left: Option<Event>,
    pub right: Option<Event>,
    context: String,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "first difference at event {}\nleft: {:?}\nright: {:?}\n{}",
            self.index, self.left, self.right, self.context
        )
    }
}

/// `Ok(())` means identical under exactly the requested normalisations.
pub fn compare(
    left: &[Event],
    right: &[Event],
    options: Normalisations,
) -> Result<(), Box<Difference>> {
    for index in 0..left.len().max(right.len()) {
        if let (Some(l), Some(r)) = (left.get(index), right.get(index)) {
            if equivalent(
                l,
                r,
                options,
                index + 1 == left.len() && index + 1 == right.len(),
            ) {
                continue;
            }
        }
        let mut context = String::new();
        for i in index.saturating_sub(2)..(index + 3).min(left.len().max(right.len())) {
            use fmt::Write;
            writeln!(
                context,
                "[{i}] left={:?} | right={:?}",
                left.get(i),
                right.get(i)
            )
            .expect("writing a string");
        }
        return Err(Box::new(Difference {
            index,
            left: left.get(index).cloned(),
            right: right.get(index).cloned(),
            context,
        }));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub enum Workload {
    SerialReal16,
    Boot,
    Keyboard,
    Rtc,
    VgaText,
    BiosPost,
    BiosFloppy,
    FreeDosPrompt,
    DosDir,
    DosWrite,
    FloppyBoot,
    FdcWrite,
    Test386,
}

pub const FAST_WORKLOADS: [Workload; 8] = [
    Workload::SerialReal16,
    Workload::Boot,
    Workload::Keyboard,
    Workload::Rtc,
    Workload::VgaText,
    Workload::FloppyBoot,
    Workload::FdcWrite,
    Workload::Test386,
];

pub struct Prepared {
    pub machine: Machine<X86JitCpu>,
    pub device_index: BTreeMap<DeviceId, usize>,
    pub policy: RunPolicy,
    pub observation: assembly::Observation,
    ram_regions: Vec<assembly::RamRegion>,
}

#[path = "fixtures.rs"]
pub(super) mod fixtures;

/// Uses the same presets, programs, inputs and policies as CLI goldens.
/// Missing optional firmware/media returns `None`; other assembly/I/O errors fail.
pub fn prepare(workload: Workload) -> Result<Option<Prepared>> {
    prepare_with_bios(workload, None)
}

/// Exercise the same scheduler preparation path with supplied BIOS bytes, so
/// rejection tests can change an image without modifying a shared fixture.
pub fn prepare_with_bios(workload: Workload, bios: Option<Vec<u8>>) -> Result<Option<Prepared>> {
    let bios_override = bios.is_some();
    let mut policy = RunPolicy::natural(None, None);
    let mut ram_regions = vec![assembly::RamRegion::new(0, 0x10000)];
    let mut assemble = |config: &assembly::MachineConfig| {
        ram_regions = config.memory.ram.clone();
        machine_file::assemble_with_observation(
            config,
            &assembly::ObservationPolicy {
                routes: config
                    .devices
                    .iter()
                    .filter(|device| matches!(device.id, DeviceId::Mda | DeviceId::Vga))
                    .map(|device| assembly::ObservationRequest {
                        device: device.id,
                        output: assembly::OutputKind::TextScanout,
                        byte_limit: 0,
                    })
                    .collect(),
                ..Default::default()
            },
        )
    };
    let assembled = match workload {
        Workload::SerialReal16 => {
            let mut cpu = X86JitCpu::new_real16_bare(
                0x10000,
                0,
                0x7C00,
                cpu::x86jit_backend::RealSegments::flat(0),
            )?;
            cpu.map_ram(0, 0x10000)?;
            cpu.write_mem(0x7C00, SERIAL_REAL16_PROGRAM)?;
            let bus = Bus::new(
                vec![Box::new(Uart16550::new(chipset::COM1_BASE))],
                DecodeMap {
                    ports: vec![PortRange {
                        width: bus::PortWidth::default(),
                        ports: chipset::COM1_BASE..=chipset::COM1_TOP,
                        device: 0,
                    }],
                    mmio: Vec::new(),
                },
            );
            AssembledMachine {
                floppy_names: Default::default(),
                machine: Machine::new(cpu, bus, Box::new(NoInterruptController), Vec::new(), None),
                a20_line: Default::default(),
                device_index: BTreeMap::from([(DeviceId::Uart, 0)]),
                mouse_target: None,
                observation: assembly::Observation::default(),
                persistence: Vec::new(),
                unlisted_firmware: Vec::new(),
                option_roms: Vec::new(),
                mount: Default::default(),
            }
        }
        Workload::Boot => {
            policy.idle_limit = Some(pc_harness::BOOT_IDLE_LIMIT);
            assemble(&assembly::at_boot(pc_harness::BOOT_PROGRAM.to_vec()))?
        }
        Workload::Keyboard => {
            policy.idle_limit = Some(pc_harness::KBD_IDLE_LIMIT);
            let mut assembled = assemble(&assembly::at_keyboard(pc_harness::KBD_PROGRAM.to_vec()))?;
            assembled
                .machine
                .bus_mut()
                .device_mut(assembled.device_index[&DeviceId::Keyboard])
                .expect("keyboard")
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<I8042>())
                .expect("8042")
                .push_scancode(pc_harness::KBD_SCANCODE);
            assembled
        }
        Workload::Rtc => {
            policy.idle_limit = Some(pc_harness::RTC_IDLE_LIMIT);
            assemble(&assembly::at_rtc(
                pc_harness::RTC_PROGRAM.to_vec(),
                pc_harness::RTC_SECOND_TICKS,
                pc_harness::RTC_UIP_TICKS,
            ))?
        }
        Workload::VgaText => assemble(&assembly::at_vga_text(
            pc_harness::VGA_TEXT_PROGRAM.to_vec(),
        ))?,
        Workload::FloppyBoot => assemble(&assembly::floppy_boot(
            pc_harness::FLOPPY_LOADER.to_vec(),
            pc_harness::bootable_floppy_image(),
        ))?,
        Workload::FdcWrite => {
            policy.retired_cap = Some(pc_harness::FDC_WRITE_RETIRED_CAP);
            assemble(&assembly::at_fdc_write(
                pc_harness::FDC_WRITE_PROGRAM.to_vec(),
                pc_harness::fdc_write_pattern(),
                pc_harness::FDC_WRITE_SRC,
            ))?
        }
        Workload::Test386 => {
            let Some(image) = fixtures::fixture("test386/test386.bin") else {
                return Ok(None);
            };
            policy.retired_cap = Some(pc_harness::TEST386_MAX_RETIRED);
            assemble(&assembly::test386(image))?
        }
        Workload::BiosPost
        | Workload::BiosFloppy
        | Workload::FreeDosPrompt
        | Workload::DosDir
        | Workload::DosWrite => {
            let Some(rom) = bios.or_else(|| fixtures::fixture("kiselev-bios/bios-xi8088.bin"))
            else {
                return Ok(None);
            };
            anyhow::ensure!(rom.len() == 32768, "Xi8088 BIOS must be 32 KiB");
            let floppy = if matches!(workload, Workload::BiosFloppy) {
                Some(pc_harness::bootable_floppy_image())
            } else if !matches!(workload, Workload::BiosPost) {
                let Some(image) = fixtures::fixture("freedos/freedos.img") else {
                    return Ok(None);
                };
                anyhow::ensure!(image.len() == 1_474_560, "FreeDOS floppy must be 1.44 MB");
                Some(image)
            } else {
                None
            };
            policy = RunPolicy::natural(
                Some(pc_harness::BIOS_GOLDEN_MAX_RETIRED),
                Some(pc_harness::BIOS_IDLE_LIMIT),
            );
            let has_floppy = floppy.is_some();
            let mut config = if matches!(workload, Workload::BiosPost | Workload::BiosFloppy) {
                assembly::bios(rom.clone(), floppy)
            } else {
                machine_file::load(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../machines/at-freedos.toml"),
                )?
            };
            if bios_override && !matches!(workload, Workload::BiosPost | Workload::BiosFloppy) {
                config.images[0].source = assembly::ImageSource::Inline(rom.clone());
            }
            if !has_floppy {
                assembly::check_board_system_bios(
                    &config,
                    assembly::BoardModel::Ibm5170,
                    config.images[0].base,
                    &rom,
                    assembly::AssemblyOptions::default(),
                )?;
            }
            let assembled = assemble(&config)?;
            if has_floppy && !matches!(workload, Workload::BiosFloppy) {
                policy.retired_cap = Some(pc_harness::BIOS_MAX_RETIRED);
                policy.stop = StopCondition::PromptThen {
                    console: ConsoleProbe {
                        device: DeviceId::Mda,
                    },
                    needle: pc_harness::DOS_DATE_PROMPT.to_string(),
                    poll_step: pc_harness::DOS_INTERACTIVE_POLL_STEP,
                    then_budget: 0,
                };
            }
            let interactive = matches!(workload, Workload::DosDir | Workload::DosWrite);
            if interactive {
                policy.stimuli.push(assembly::KeyboardStimulus {
                    keyboard_index: assembled.device_index[&DeviceId::Keyboard],
                    scancodes: if matches!(workload, Workload::DosWrite) {
                        pc_harness::dos_write_file_script()
                    } else {
                        pc_harness::dos_dir_script()
                    },
                    schedule: assembly::InjectionSchedule::AfterPrompt,
                });
                if let StopCondition::PromptThen { then_budget, .. } = &mut policy.stop {
                    *then_budget = pc_harness::DOS_INTERACTIVE_COMMAND_BUDGET;
                }
            }
            assembled
        }
    };
    Ok(Some(Prepared {
        machine: assembled.machine,
        device_index: assembled.device_index,
        policy,
        observation: assembled.observation,
        ram_regions,
    }))
}

type SharedStream = Rc<RefCell<Stream>>;

fn drain_pic(logs: &[PicObservationLog], stream: &SharedStream) {
    for log in logs {
        stream
            .borrow_mut()
            .extend(log.take().into_iter().map(Event::Pic));
    }
}

struct ObservedDevice {
    inner: Box<dyn Device>,
    id: DeviceId,
    stream: SharedStream,
    pic: Vec<PicObservationLog>,
    dma_probes: Rc<Cell<usize>>,
}

impl ObservedDevice {
    fn before(&self) {
        drain_pic(&self.pic, &self.stream);
    }
    fn after(&mut self) {
        assert_eq!(self.inner.trace_dropped(), 0, "transaction trace overflow");
        self.stream
            .borrow_mut()
            .extend(
                self.inner
                    .take_trace()
                    .into_iter()
                    .map(|access| Event::Access {
                        device: self.id,
                        access,
                    }),
            );
        drain_pic(&self.pic, &self.stream);
    }
}

impl Device for ObservedDevice {
    fn text_scanout(&self) -> Option<device::TextModeState<'_>> {
        self.inner.text_scanout()
    }
    fn byte_output(&mut self) -> Option<&mut dyn device::ByteOutput> {
        self.inner.byte_output()
    }
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
        self.before();
        let result = self.inner.port_read(port, size);
        self.after();
        result
    }
    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
        self.before();
        let result = self.inner.port_write(port, size, value);
        self.after();
        result
    }
    fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        self.before();
        let result = self.inner.mmio_read(addr, size);
        self.after();
        result
    }
    fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError> {
        self.before();
        let result = self.inner.mmio_write(addr, size, value);
        self.after();
        result
    }
    fn tick(&mut self, now: VClock) {
        self.inner.tick(now);
    }
    fn sync(&mut self, now: VClock) {
        self.inner.sync(now);
    }
    fn next_event(&self, now: VClock) -> NextEvent {
        self.inner.next_event(now)
    }
    fn irq_pending(&self) -> bool {
        self.inner.irq_pending()
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn save(&self) -> Vec<u8> {
        self.inner.save()
    }
    fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError> {
        self.inner.restore(snapshot)
    }
    fn set_trace_policy(&mut self, policy: device::TracePolicy) {
        self.inner.set_trace_policy(policy);
    }
    fn trace_dropped(&self) -> u64 {
        self.inner.trace_dropped()
    }
    fn take_trace(&mut self) -> Vec<TraceEvent> {
        self.inner.take_trace()
    }
    fn dma_request_pending(&self) -> bool {
        self.inner.dma_request_pending()
    }
    fn take_dma_request(&mut self) -> Option<DmaRequest> {
        self.dma_probes.set(self.dma_probes.get() + 1);
        self.inner.take_dma_request()
    }
    fn dma_complete(&mut self, outcome: DmaOutcome) {
        self.inner.dma_complete(outcome);
    }
    fn dma_service_write(
        &mut self,
        channel: u8,
        mem: &mut dyn DmaMemory,
        data: &[u8],
    ) -> device::DmaTransfer {
        self.inner.dma_service_write(channel, mem, data)
    }
    fn dma_service_read(
        &mut self,
        channel: u8,
        mem: &mut dyn DmaMemory,
        out: &mut [u8],
    ) -> device::DmaTransfer {
        self.inner.dma_service_read(channel, mem, out)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn core::any::Any> {
        self.inner.as_any_mut()
    }
}

#[test]
fn observed_idle_dma_requester_receives_no_probes() {
    let probes = Rc::new(Cell::new(0));
    let requester = ObservedDevice {
        inner: Box::new(devices::Upd765::new()),
        id: DeviceId::Fdc,
        stream: SharedStream::default(),
        pic: Vec::new(),
        dma_probes: probes.clone(),
    };
    let cpu = testkit::MockCpu::new(Vec::new()).with_step_program(vec![
        testkit::MockCpuStep {
            max_steps: 1,
            run: cpu::CpuStepRun {
                steps: 1,
                stop: cpu::CpuStepStop::Exit(cpu::CpuStop::BudgetExhausted),
            },
            retired: 1,
            irq_ready: false,
        };
        50
    ]);
    let mut machine = Machine::new(
        cpu,
        Bus::new(
            vec![Box::new(requester), Box::new(devices::Dma8237::new())],
            DecodeMap::default(),
        ),
        Box::new(NoInterruptController),
        Vec::new(),
        None,
    )
    .with_dma_wirings(vec![machine::DmaWiring {
        requester: 0,
        controller: 1,
    }])
    .with_step_limit(std::num::NonZeroU64::MIN);
    assert_eq!(
        machine.run_bounded(Some(50)).unwrap(),
        RunOutcome::RetiredCapReached
    );
    assert_eq!(machine.cpu().retired(), 50);
    assert_eq!(probes.get(), 0);
}

#[test]
fn observed_device_preserves_scanout_after_draining_traces() {
    let mut display = testkit::MockBus::new(Box::new(ObservedDevice {
        inner: Box::new(devices::VgaText::new()),
        id: DeviceId::Vga,
        stream: SharedStream::default(),
        pic: Vec::new(),
        dma_probes: Default::default(),
    }));
    display
        .device_mut()
        .set_trace_policy(device::TracePolicy::Ring { capacity: 2 });
    display.mmio_write(0xB8000, 2, 0x4E41).unwrap();
    assert!(display.device_mut().take_trace().is_empty());
    assert_eq!(
        &display.device().text_scanout().unwrap().vram[..2],
        &[b'A', 0x4E]
    );
}

#[test]
fn observed_device_preserves_byte_output_after_draining_traces() {
    let mut serial = testkit::MockBus::new(Box::new(ObservedDevice {
        inner: Box::new(Uart16550::new(0x3F8)),
        id: DeviceId::Uart,
        stream: SharedStream::default(),
        pic: Vec::new(),
        dma_probes: Default::default(),
    }));
    serial
        .device_mut()
        .set_trace_policy(device::TracePolicy::Ring { capacity: 2 });
    let capture = device::ByteCapture::new(4);
    serial
        .device_mut()
        .byte_output()
        .unwrap()
        .bind_sink(Box::new(capture.clone()));
    serial.port_write(0x3F8, 1, u32::from(b'A')).unwrap();
    assert!(serial.device_mut().take_trace().is_empty());
    assert_eq!(capture.drain().unwrap(), b"A");
}

// FNV-1a fixes the diagnostic hash across hosts and Rust versions.
fn state_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[test]
fn final_state_detects_untraced_ram_register_and_device_changes() {
    let observe = |changed| {
        let mut cpu = X86JitCpu::new_real16_bare(
            0x10000,
            0,
            0x7C00,
            cpu::x86jit_backend::RealSegments::flat(0),
        )
        .unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(0x7C00, SERIAL_REAL16_PROGRAM).unwrap();
        let mut uart = Uart16550::new(0x3F8);
        match changed {
            1 => cpu.write_mem(0x600, &[42]).unwrap(),
            2 => cpu.set_reg(CpuReg::Rax, 42),
            3 => uart.port_write(0x3FB, 1, 3).unwrap(),
            _ => {}
        }
        observe_with(
            Prepared {
                machine: Machine::new(
                    cpu,
                    Bus::new(vec![Box::new(uart)], DecodeMap::default()),
                    Box::new(NoInterruptController),
                    Vec::new(),
                    None,
                ),
                device_index: BTreeMap::from([(DeviceId::Uart, 0)]),
                policy: RunPolicy::natural(None, None),
                observation: assembly::Observation::default(),
                ram_regions: vec![assembly::RamRegion::new(0, 0x10000)],
            },
            |_, _, _| Ok(RunOutcome::Halted),
        )
    };
    let baseline = observe(0);
    assert!(matches!(
        baseline.as_slice(),
        [Event::FinalState { .. }, Event::Stop { .. }]
    ));
    for changed in 1..=3 {
        let stream = observe(changed);
        let diff = compare(&baseline, &stream, Normalisations::default()).unwrap_err();
        assert_eq!(diff.index, 0);
        match (&baseline[0], &stream[0]) {
            (
                Event::FinalState {
                    ram: left_ram,
                    devices: left_devices,
                    registers: left_regs,
                    ..
                },
                Event::FinalState {
                    ram: right_ram,
                    devices: right_devices,
                    registers: right_regs,
                    ..
                },
            ) => {
                assert_eq!(left_ram == right_ram, changed != 1);
                assert_eq!(left_regs == right_regs, changed != 2);
                assert_eq!(left_devices == right_devices, changed != 3);
            }
            _ => panic!("missing final state"),
        }
    }
}

/// The observer wraps existing bus devices without changing CPU/IRQ semantics.
pub fn observe_with(
    mut prepared: Prepared,
    engine: impl FnOnce(
        &mut Machine<X86JitCpu>,
        &assembly::Observation,
        &RunPolicy,
    ) -> Result<RunOutcome, assembly::RunError>,
) -> Stream {
    let stream = SharedStream::default();
    let clock = prepared.machine.bus().observation_clock();
    let mut pic = Vec::new();
    let mut devices: Vec<_> = prepared
        .device_index
        .iter()
        .map(|(&id, &index)| (id, index))
        .collect();
    devices.sort_unstable_by_key(|&(_, index)| index);
    for &(_, index) in &devices {
        if let Some(shared) = prepared
            .machine
            .bus_mut()
            .device_mut(index)
            .expect("device")
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<SharedPic>())
        {
            // Dormant PICs must observe bus time, independent of device ticks.
            let log = shared.observe(clock.clone());
            pic.push(log);
        }
    }
    for (id, index) in devices {
        let slot = prepared
            .machine
            .bus_mut()
            .device_mut(index)
            .expect("device");
        // A placeholder permits moving the box out without a bus API change.
        let mut inner = std::mem::replace(slot, Box::new(Uart16550::new(0)));
        // Each transaction emits at most one access before the wrapper drains it.
        inner.set_trace_policy(device::TracePolicy::Ring { capacity: 16 });
        *slot = Box::new(ObservedDevice {
            inner,
            id,
            stream: stream.clone(),
            pic: pic.clone(),
            dma_probes: Default::default(),
        });
    }
    let result = engine(
        &mut prepared.machine,
        &prepared.observation,
        &prepared.policy,
    );
    prepared.machine.bus_mut().settle();
    drain_pic(&pic, &stream);
    let reason = match result {
        Ok(RunOutcome::Halted) => Stop::Halted,
        Ok(RunOutcome::ResetRequested) => Stop::ResetRequested,
        Ok(RunOutcome::RetiredCapReached) => Stop::CapReached,
        Err(error) => Stop::Error(error.to_string()),
    };
    let ram = prepared
        .ram_regions
        .iter()
        .map(|region| {
            let mut bytes = vec![0; region.size as usize];
            prepared
                .machine
                .cpu()
                .read_mem(region.base, &mut bytes)
                .unwrap();
            (region.base, region.size, state_hash(&bytes))
        })
        .collect();
    let devices = (0..)
        .map_while(|index| {
            prepared.machine.bus().device(index).map(|device| {
                let id = prepared
                    .device_index
                    .iter()
                    .find_map(|(&id, &slot)| (slot == index).then_some(id))
                    .expect("device id");
                (id, device.name().to_string(), state_hash(&device.save()))
            })
        })
        .collect();
    let registers = [
        CpuReg::Rax,
        CpuReg::Rcx,
        CpuReg::Rdx,
        CpuReg::Rbx,
        CpuReg::Rsp,
        CpuReg::Rbp,
        CpuReg::Rsi,
        CpuReg::Rdi,
        CpuReg::Rip,
    ]
    .into_iter()
    .map(|reg| (reg, prepared.machine.cpu().reg(reg)))
    .collect();
    stream.borrow_mut().push(Event::FinalState {
        ram,
        devices,
        registers,
        code_segment: prepared.machine.cpu().code_segment(),
    });
    stream.borrow_mut().push(Event::Stop {
        at: prepared.machine.clock(),
        virtual_time: prepared.machine.virtual_time(),
        reason,
    });
    drop(prepared);
    Rc::try_unwrap(stream)
        .expect("observers released with the machine")
        .into_inner()
}

pub fn run_workload(workload: Workload) -> Result<Option<Stream>> {
    Ok(prepare(workload)?.map(|prepared| observe_with(prepared, assembly::run)))
}

/// Run the same prepared workload and policy with an explicit device engine.
pub fn run_workload_engine(
    workload: Workload,
    engine: machine::SchedulerEngine,
) -> Result<Option<Stream>> {
    Ok(prepare(workload)?.map(|mut prepared| {
        prepared.machine = prepared.machine.with_scheduler_engine(engine);
        let mda = prepared.device_index.get(&DeviceId::Mda).copied();
        observe_with(prepared, |machine, observation, policy| {
            let result = assembly::run(machine, observation, policy);
            let expected = match workload {
                // The bare rig's 8042 selects 80286; its trap after POP DS
                // changes the open BIOS's CPU identification and warning row.
                Workload::BiosPost => Some(0x945fcce568e5d520),
                Workload::BiosFloppy => Some(0x92c581d95567fb07),
                // BIOS century 20h gives FreeDOS Sat 01-01-2000 and stamps new files with year 00.
                Workload::FreeDosPrompt => Some(0xc46f0c147c881837),
                Workload::DosDir => Some(0x65259fa4c26a891c),
                Workload::DosWrite => Some(0x6b0404d06d7532c3),
                _ => None,
            };
            if let Some(expected) = expected {
                let state = machine
                    .bus()
                    .device(mda.unwrap())
                    .unwrap()
                    .text_scanout()
                    .unwrap();
                let console = video::render_text_mode(&state).to_dump();
                eprintln!(
                    "{workload:?} {engine:?}: {result:?}, retired={}, virtual={}\n{console}",
                    machine.cpu().retired(),
                    machine.virtual_time()
                );
                assert_eq!(
                    super::session::logical_hash(&console),
                    expected,
                    "{workload:?}"
                );
            }
            result
        })
    }))
}
