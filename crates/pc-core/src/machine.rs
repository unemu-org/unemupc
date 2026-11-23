// SPDX-License-Identifier: AGPL-3.0-or-later
//! A running PC behind the contract (core-contract.md §4).
//!
//! [`PcMachine`] owns an [`AssembledMachine`] and everything between it and a
//! front end: the scan-code FIFO with the 8042's back-pressure, the pointing
//! device's queue, the rasterised pictures of the observed displays, the
//! mixed sound since the last drain, the drive activity since the last look
//! and the lines the session reports. A run follows the boundary order the
//! desktop has always used: check the journals, deliver keyboard then mouse
//! input, run to a retired-instruction cap, sample the drives, settle the
//! journals. A guest-requested reset happens inside the run, as on the board
//! (IBM Personal Computer AT Technical Reference, System Board, Keyboard
//! Controller Output Port: the 8042's output-port pulse resets only the
//! processor), and the run continues to its deadline.

use crate::{
    pacing::{run_cap, CpuRate},
    HostKeyboard, PcMouse, PixelFrame, RasterSink,
};
use assembly::{AssembledMachine, DeviceId, DeviceKind, MachineConfig, OutputKind};
use audio::{AudioBlock, AudioSink, StereoFrame, FRAMES_PER_SECOND};
use device::clock::{PIT_HZ, TICKS_PER_PIT_CLOCK};
use machine::RunOutcome;
use std::{
    cell::{Cell, RefCell},
    io::{Read, Write},
    num::{NonZeroU16, NonZeroU64},
    rc::Rc,
};
use unemu_core_api::{
    AudioDescription, DisplayDescription, DisplayId, Error, ErrorKind, Features, Finished, Frame,
    InputEvent, InputKind, InputPortDescription, Machine, MachineDescription, MediaAction,
    MediumInfo, Message, MessageLevel, NewMedium, Outputs, PortId, Rate, ResetKind, RunLimit,
    SlotDescription, SlotId, SlotKind, SlotState, Status, Stop, Ticks, WriteKind,
};

/// Integer scale of the rasterised text cells: every picture of a PC is
/// drawn at twice the cell size of its text mode.
pub const RASTER_SCALE: u16 = 2;

/// How many entries an exception recording keeps
/// ([`PcMachine::start_delivery_recording`]).
pub const RECORDED_DELIVERIES: usize = ::machine::RECORDED_ENTRIES;

/// The keyboard's input port.
pub(crate) const KEYBOARD_PORT: PortId = PortId(0);
/// The pointing device's input port.
pub(crate) const POINTER_PORT: PortId = PortId(1);

/// What one PC can do: the reset button and floppy changes while it runs.
pub(crate) const PC_FEATURES: Features = Features::NONE
    .with_resets(&[ResetKind::Soft])
    .with_media_changes(true);

/// One boundary's delivered keyboard bytes, for replay at that virtual time:
/// the virtual time, the bytes and the keyboard-owned sequences among them.
#[cfg(feature = "test-support")]
pub type InputLogEntry = (u64, Vec<u8>, Vec<std::ops::Range<usize>>);

/// The mixed sound since the last drain; the front end decides whether it
/// wants any.
#[derive(Clone, Default)]
struct PulledAudio {
    frames: Rc<RefCell<Vec<StereoFrame>>>,
    enabled: Rc<Cell<bool>>,
}

impl AudioSink for PulledAudio {
    fn present(&mut self, block: AudioBlock<'_>) {
        if self.enabled.get() {
            self.frames.borrow_mut().extend_from_slice(block.frames());
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.get()
    }
}

/// One observed display: the card, its rasteriser and its last picture.
struct Display {
    device: DeviceId,
    sink: RasterSink,
    picture: Option<PixelFrame>,
}

/// A running PC behind the contract: an assembled machine and the host-side
/// encoders.
pub struct PcMachine {
    assembled: AssembledMachine,
    config: MachineConfig,
    pub(crate) contract_failure: Option<crate::core::mount::Failures>,
    description: MachineDescription,
    displays: Vec<Display>,
    serial: u64,
    audio: PulledAudio,
    keyboard: HostKeyboard,
    mouse: PcMouse,
    rate: CpuRate,
    status: Status,
    /// Drive busy state at any boundary since the last look, as A:, B:, C:, D:.
    activity: [bool; 4],
    messages: Vec<Message>,
    reset_requests: u64,
    cpu_resets: u64,
    maintenance_reported: bool,
    /// The diagnosis of the CPU refusal that failed the machine.
    cpu_stop_report: Option<cpu::CpuStopReport>,
    guest_failure: Option<machine::MachineError>,
    /// Keyboard bytes delivered at each boundary, for the tests' replays.
    #[cfg(feature = "test-support")]
    pub input_log: Vec<InputLogEntry>,
    /// Mouse segments delivered at each boundary, for the tests' replays.
    #[cfg(feature = "test-support")]
    pub mouse_log: Vec<(u64, Vec<crate::MouseInput>)>,
}

fn error(kind: ErrorKind, error: impl std::fmt::Display) -> Error {
    Error::new(kind, error.to_string())
}

fn display_label(device: DeviceId) -> &'static str {
    match device {
        DeviceId::Mda => "MDA",
        DeviceId::Vga => "VGA",
        _ => "Display",
    }
}

/// The slot id of a drive: its place among A:, B:, C: and D:.
fn slot_of(controller: DeviceId, unit: u8) -> Option<SlotId> {
    match (controller, unit) {
        (DeviceId::Fdc, 0 | 1) => Some(SlotId(unit)),
        (DeviceId::Ide, 0 | 1) => Some(SlotId(2 + unit)),
        _ => None,
    }
}

impl PcMachine {
    /// Wrap a machine the caller assembled, for machine-file starts and
    /// preset-based tests. Stored-image starts use [`crate::PcCore::create_machine`]
    /// over the provider's launch document and content/storage objects.
    ///
    /// Every display the machine's observation routes to a text scanout is a
    /// display of this machine, in route order. HLT waits for an interrupt
    /// without an idle limit: the run's deadline bounds idle time.
    pub fn from_assembled(
        config: &MachineConfig,
        mut assembled: AssembledMachine,
    ) -> Result<Self, Error> {
        let mut messages: Vec<Message> = assembled
            .unlisted_firmware
            .iter()
            .map(|unlisted| Message::new(MessageLevel::Warning, unlisted.to_string()))
            .collect();
        if assembled.observation.speaker_trace_enabled() {
            messages.push(Message::new(
                MessageLevel::Note,
                "speaker trace on: machine assembled, guest time starts at 0",
            ));
        }
        let clock = &config.clock;
        assembled.machine.set_idle_limit(Some(u64::MAX));
        let scale = NonZeroU16::new(RASTER_SCALE)
            .ok_or_else(|| Error::new(ErrorKind::Unsupported, "invalid raster scale"))?;
        let routes: Vec<DeviceId> = assembled
            .observation
            .routes()
            .iter()
            .filter(|route| route.output == OutputKind::TextScanout)
            .map(|route| route.device)
            .collect();
        let mut displays = Vec::new();
        for device in routes {
            let sink = RasterSink::new(scale).map_err(|e| error(ErrorKind::Unsupported, e))?;
            assembled
                .observation
                .subscribe(device, Box::new(sink.clone()))
                .map_err(|e| error(ErrorKind::Config, e))?;
            displays.push(Display {
                device,
                sink,
                picture: None,
            });
        }
        let audio = PulledAudio::default();
        assembled
            .observation
            .subscribe_audio(Box::new(audio.clone()));
        let rate = CpuRate {
            numerator: NonZeroU64::new(clock.retired_per_pit_clock)
                .ok_or_else(|| Error::new(ErrorKind::Config, "zero CPU rate"))?,
            denominator: NonZeroU64::new(clock.retired_per_pit_clock_denominator)
                .ok_or_else(|| Error::new(ErrorKind::Config, "zero CPU rate denominator"))?,
        };
        let mut inputs = Vec::new();
        if config
            .devices
            .iter()
            .any(|device| matches!(device.kind, DeviceKind::Keyboard(_)))
        {
            inputs.push(InputPortDescription::new(
                KEYBOARD_PORT,
                InputKind::Keyboard,
                "Keyboard",
            ));
        }
        if config.devices.iter().any(|device| {
            matches!(&device.kind, DeviceKind::Keyboard(controller) if controller.mouse)
                || matches!(&device.kind, DeviceKind::Uart(uart) if uart.mouse)
        }) {
            inputs.push(InputPortDescription::new(
                POINTER_PORT,
                InputKind::RelativePointer { buttons: 3 },
                "Mouse",
            ));
        }
        let mut slots: Vec<SlotDescription> = config
            .drives
            .iter()
            .filter_map(|drive| {
                let id = slot_of(drive.controller, drive.unit)?;
                let floppy = drive.controller == DeviceId::Fdc;
                Some(SlotDescription::new(
                    id,
                    if floppy {
                        SlotKind::Floppy
                    } else {
                        SlotKind::FixedDisk
                    },
                    format!("{}:", char::from(b'A' + id.0)),
                    floppy,
                ))
            })
            .collect();
        slots.sort_by_key(|slot| slot.id);
        slots.dedup_by_key(|slot| slot.id);
        let description = MachineDescription::new(
            Rate::new(
                PIT_HZ.numerator() * TICKS_PER_PIT_CLOCK,
                PIT_HZ.denominator(),
            ),
            "instruction",
        )
        .with_displays(
            displays
                .iter()
                .enumerate()
                .map(|(index, display)| {
                    DisplayDescription::new(DisplayId(index as u8), display_label(display.device))
                })
                .collect(),
        )
        .with_audio(Some(AudioDescription::new(Rate::new(
            u64::from(FRAMES_PER_SECOND),
            1,
        ))))
        .with_inputs(inputs)
        .with_slots(slots)
        .with_features(PC_FEATURES);
        Ok(Self {
            config: config.clone(),
            contract_failure: None,
            assembled,
            description,
            displays,
            serial: 0,
            audio,
            keyboard: HostKeyboard::default(),
            mouse: PcMouse::default(),
            rate,
            status: Status::Ready,
            activity: [false; 4],
            messages,
            reset_requests: 0,
            cpu_resets: 0,
            maintenance_reported: false,
            cpu_stop_report: None,
            guest_failure: None,
            #[cfg(feature = "test-support")]
            input_log: Vec::new(),
            #[cfg(feature = "test-support")]
            mouse_log: Vec::new(),
        })
    }

    /// The effective hardware and catalogues this machine started with.
    pub fn config(&self) -> &MachineConfig {
        &self.config
    }

    /// The assembled machine, for PC code that needs more than the
    /// contract: the text grid, the floppy catalogue, device state in tests.
    pub fn assembled(&mut self) -> &mut AssembledMachine {
        &mut self.assembled
    }

    /// The assembled machine, read only.
    pub fn assembled_ref(&self) -> &AssembledMachine {
        &self.assembled
    }

    /// The keyboard's FIFO: what waits for the 8042.
    pub fn keyboard(&self) -> &HostKeyboard {
        &self.keyboard
    }

    /// Drive busy state seen at the boundaries since the slots were last
    /// read, as A:, B:, C:, D:, without taking it.
    pub fn drive_activity(&self) -> [bool; 4] {
        self.activity
    }

    /// Deliver the queued keyboard bytes the 8042 takes now, as a run
    /// boundary does, without running.
    #[cfg(feature = "test-support")]
    pub fn deliver_keys_for_test(&mut self) -> Vec<u8> {
        self.keyboard.inject_at_boundary(&mut self.assembled)
    }

    /// Deliver the queued mouse segments the device takes now, as a run
    /// boundary does, without running.
    #[cfg(feature = "test-support")]
    pub fn deliver_pointer_for_test(&mut self) -> Vec<crate::MouseInput> {
        self.mouse.inject_at_boundary(&mut self.assembled)
    }

    /// Queue the keyboard-owned Ctrl+Alt+Del reset chord, which reaches the
    /// keyboard buffer whole even while the interface is inhibited (see
    /// [`HostKeyboard::ctrl_alt_del`]); three key events would not.
    pub fn ctrl_alt_del(&mut self) {
        self.keyboard.ctrl_alt_del();
    }

    /// The catalogue entry a floppy drive holds now.
    pub fn inserted_floppy(&mut self, unit: usize) -> Option<usize> {
        self.fdc()?.inserted_medium(unit)
    }

    /// Guest-requested resets seen so far.
    pub fn reset_requests(&self) -> u64 {
        self.reset_requests
    }

    /// CPU resets performed so far, guest-requested and from the host.
    pub fn cpu_resets(&self) -> u64 {
        self.cpu_resets
    }

    /// The diagnosis of the stop when the CPU core refused an operation, met
    /// an instruction it does not lift or faulted outward and so failed the
    /// machine: registers, descriptor tables, the bytes at CS:IP and the
    /// descriptors the instruction reaches; see [`cpu::stop_report`]. `None`
    /// while the machine runs and after any other failure. A front end shows
    /// it beside the error, which states only the operation and CS:IP.
    pub fn cpu_stop_report(&self) -> Option<&cpu::CpuStopReport> {
        self.cpu_stop_report.as_ref()
    }

    /// Take the typed run failure so PC commands can preserve its error
    /// source chain, which the contract's flat diagnostic does not carry.
    pub fn take_guest_failure(&mut self) -> Option<machine::MachineError> {
        self.guest_failure.take()
    }

    /// Record the CPU's CS:IP after every run slice from now on, for the
    /// trail of [`Self::diagnostic_report`]; see `machine::CodeTrail`. The
    /// trail stays outside the CPU's execution path: retired counts, virtual
    /// time and every guest effect are the same with it on or off.
    pub fn enable_code_trail(&mut self) {
        self.assembled.machine.enable_code_trail();
    }

    /// Keep the last exceptions and interrupts the guest takes, for the
    /// deliveries of [`Self::diagnostic_report`]; see
    /// `machine::delivery_tap`. The CPU's delivery log stays outside the
    /// state the guest executes on: retired counts, virtual time and every
    /// guest effect are the same with it on or off.
    pub fn enable_delivery_log(&mut self) -> Result<(), cpu::DeliveryLogError> {
        self.assembled.enable_delivery_log()
    }

    /// Whether [`Self::enable_delivery_log`] enabled the report's
    /// deliveries.
    pub fn delivery_log_enabled(&self) -> bool {
        self.assembled.machine.recent_deliveries().is_some()
    }

    /// Start recording every exception and interrupt the guest takes, the
    /// last `machine::RECORDED_ENTRIES` entries with exact totals; see
    /// `machine::DeliveryRecording`. Like [`Self::enable_delivery_log`], it
    /// leaves the run unchanged.
    pub fn start_delivery_recording(&mut self) -> Result<(), cpu::DeliveryLogError> {
        self.assembled.start_delivery_recording()
    }

    /// Whether a recording runs.
    pub fn delivery_recording_active(&self) -> bool {
        self.assembled.machine.delivery_recording_active()
    }

    /// The recording's text so far; `None` while none runs.
    pub fn delivery_recording(&self) -> Option<String> {
        self.assembled.machine.delivery_recording()
    }

    /// End the recording; its text, `None` when none ran.
    pub fn stop_delivery_recording(&mut self) -> Option<String> {
        self.assembled.machine.stop_delivery_recording()
    }

    /// The diagnostic report of the machine at its completed run boundary:
    /// the retired count, virtual time and status, the CPU, the code trail,
    /// the 8259s, the 8042, the PIT, A20 and the CMOS shutdown byte and NMI
    /// mask; see `assembly::diagnostic`. It performs no guest access and
    /// changes nothing the guest can see, so a front end may take it while
    /// the machine runs, between two runs. The status is the machine's
    /// contract status; whether a front end holds the machine paused is the
    /// front end's to add.
    pub fn diagnostic_report(&mut self) -> assembly::MachineDiagnostic {
        let status = match self.status {
            Status::Ready => "ready",
            Status::Halted => "halted (waits for a reset)",
            Status::Off => "off",
            Status::Failed => "failed",
            _ => "unknown",
        };
        self.assembled.diagnostic_report(status)
    }

    /// An owned image of every system RAM region at this completed run boundary,
    /// in physical address order, independent of A20. ROM and device windows
    /// are omitted; no guest access or device synchronisation occurs, so the
    /// run stays unchanged. Contains guest bytes for local diagnosis only;
    /// never add an image to a repository. See `assembly::memory_image` and
    /// `backlog/docs/design/observation.md` §Guest RAM image.
    pub fn memory_image(&self) -> Result<assembly::MemoryImage, cpu::CpuError> {
        self.assembled.memory_image(&self.config.memory)
    }

    /// The session's reset total, for its end; nothing when none happened.
    pub fn reset_summary(&self) -> Option<String> {
        (self.cpu_resets != 0).then(|| format!("cpu resets in session: {}", self.cpu_resets))
    }

    /// Count one performed CPU reset; only the first is reported.
    pub fn record_cpu_reset(&mut self) {
        self.cpu_resets = self.cpu_resets.saturating_add(1);
        if self.cpu_resets == 1 {
            self.messages
                .push(Message::new(MessageLevel::Note, "cpu reset"));
        }
    }

    fn fdc(&mut self) -> Option<&mut devices::Upd765> {
        let index = *self.assembled.device_index.get(&DeviceId::Fdc)?;
        self.assembled
            .machine
            .bus_mut()
            .device_mut(index)?
            .as_any_mut()?
            .downcast_mut()
    }

    fn ata(&mut self) -> Option<&mut devices::Ata> {
        let index = *self.assembled.device_index.get(&DeviceId::Ide)?;
        self.assembled
            .machine
            .bus_mut()
            .device_mut(index)?
            .as_any_mut()?
            .downcast_mut()
    }

    /// Read each drive's busy state at a completed run boundary, for the
    /// front end's drive lights. `device_mut` syncs each controller to the
    /// bus clock and clears its cached deadline, which only repeats the
    /// settle the next run starts with; the accessors themselves change no
    /// device state.
    fn sample_drive_busy(&mut self) {
        let floppy: [bool; 2] =
            std::array::from_fn(|unit| self.fdc().is_some_and(|fdc| fdc.drive_active(unit)));
        let fixed: [bool; 2] =
            std::array::from_fn(|unit| self.ata().is_some_and(|ata| ata.activity(unit)));
        for (seen, busy) in self
            .activity
            .iter_mut()
            .zip(floppy.into_iter().chain(fixed))
        {
            *seen |= busy;
        }
    }

    /// One message per traced speaker or reset event; nothing without the trace.
    fn take_speaker_trace(&mut self) {
        let clock = self.assembled.machine.clock();
        for event in self.assembled.observation.take_speaker_trace(clock) {
            self.messages
                .push(Message::new(MessageLevel::Note, event.to_string()));
        }
    }

    fn storage(&mut self, failure: assembly::PersistenceError) -> Error {
        self.status = Status::Failed;
        self.contract_error(ErrorKind::Storage, failure)
    }

    fn contract_error(&self, kind: ErrorKind, failure: impl std::error::Error + 'static) -> Error {
        self.contract_failure
            .as_ref()
            .and_then(|errors| errors.error_for(&failure))
            .unwrap_or_else(|| error(kind, failure))
    }

    /// Deliver the queued keyboard bytes and mouse segments the devices take
    /// now, keyboard first.
    fn deliver_input(&mut self) {
        let delivered = self.keyboard.inject_at_boundary(&mut self.assembled);
        #[cfg(feature = "test-support")]
        if !delivered.is_empty() {
            self.input_log.push((
                self.assembled.machine.virtual_time(),
                delivered,
                self.keyboard.typed_ranges().to_vec(),
            ));
        }
        #[cfg(not(feature = "test-support"))]
        drop(delivered);
        let mouse_input = self.mouse.inject_at_boundary(&mut self.assembled);
        #[cfg(feature = "test-support")]
        if !mouse_input.is_empty() {
            self.mouse_log
                .push((self.assembled.machine.virtual_time(), mouse_input));
        }
        #[cfg(not(feature = "test-support"))]
        drop(mouse_input);
    }
}

impl Machine for PcMachine {
    fn description(&self) -> &MachineDescription {
        &self.description
    }

    fn now(&self) -> Ticks {
        Ticks(self.assembled.machine.clock().0)
    }

    fn steps(&self) -> u64 {
        self.assembled.machine.virtual_time()
    }

    fn status(&self) -> Status {
        self.status
    }

    /// One bounded run per pass, as the desktop always ran its chunks; the PC
    /// lists no frame stops, so `stop_on_frame` is refused. A
    /// deadline becomes a retired-instruction cap at the machine's configured
    /// rate; reaching the cap ends the run with [`Stop::Deadline`], even when
    /// halted time leaves the clock a fraction of a tick short of `until` (the
    /// front end runs again). Exhausted virtual time stops the machine as if
    /// it were switched off.
    ///
    /// A guest-requested reset happens inside the run, which continues to
    /// the same deadline. A reset retires nothing, so a guest that resets
    /// again right after every reset would barely move virtual time; the
    /// machine stops such a storm (`CpuError::ResetStorm`, a `Guest` error),
    /// so a run does at most 63 resets in a row that are close together, and
    /// every other reset follows at least 128 retired instructions, which
    /// count toward the deadline's cap.
    fn run(&mut self, limit: RunLimit) -> Result<Stop, Error> {
        if limit.stop_on_frame {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "a PC run cannot stop at the end of a frame",
            ));
        }
        match self.status {
            Status::Halted => return Ok(Stop::Halted),
            Status::Off => return Ok(Stop::PoweredOff),
            Status::Failed => {
                return Err(Error::new(
                    ErrorKind::Usage,
                    "the machine failed and cannot run",
                ))
            }
            _ => {}
        }
        loop {
            let clock = self.assembled.machine.clock().0;
            let time = self.assembled.machine.virtual_time();
            let mut cap = u64::MAX;
            if let Some(until) = limit.until {
                if clock >= until.0 {
                    return Ok(Stop::Deadline);
                }
                cap = run_cap(until.0, clock, time, self.rate);
            }
            if let Some(steps) = limit.steps {
                if time >= steps {
                    return Ok(Stop::Steps);
                }
                cap = cap.min(steps);
            }
            if cap == time {
                self.status = Status::Off;
                return Ok(Stop::PoweredOff);
            }
            if let Err(failure) = self.assembled.check_persistence() {
                return Err(self.storage(failure));
            }
            self.deliver_input();
            let outcome = self.assembled.machine.run_bounded(Some(cap));
            self.sample_drive_busy();
            if outcome.is_err() {
                if let Err(failure) = self.assembled.flush_persistence() {
                    return Err(self.storage(failure));
                }
            }
            if matches!(outcome, Ok(RunOutcome::ResetRequested)) {
                self.reset_requests = self.reset_requests.saturating_add(1);
            }
            if let Err(failure) = self.assembled.check_persistence() {
                return Err(self.storage(failure));
            }
            match outcome {
                Ok(RunOutcome::RetiredCapReached) => {
                    self.take_speaker_trace();
                    if cap == u64::MAX {
                        self.status = Status::Off;
                        return Ok(Stop::PoweredOff);
                    }
                    return Ok(
                        if limit
                            .steps
                            .is_some_and(|steps| self.assembled.machine.virtual_time() >= steps)
                        {
                            Stop::Steps
                        } else {
                            Stop::Deadline
                        },
                    );
                }
                Ok(RunOutcome::ResetRequested) => {
                    if let Err(failure) = self.assembled.before_cpu_reset() {
                        return Err(self.storage(failure));
                    }
                    if let Err(failure) = self.assembled.reset_cpu(assembly::ResetCause::Requested)
                    {
                        self.status = Status::Failed;
                        let error = error(ErrorKind::Guest, &failure);
                        self.guest_failure = Some(failure);
                        return Err(error);
                    }
                    self.record_cpu_reset();
                    self.take_speaker_trace();
                }
                Ok(RunOutcome::Halted) => {
                    self.status = Status::Halted;
                    // The session stays open for a reset, but what the guest
                    // wrote up to the halt becomes durable now, as before a
                    // reset: changed CMOS RAM and completed ATA sectors.
                    // A failure here leaves the machine halted, so Stop can
                    // still finish its media.
                    self.assembled
                        .before_cpu_reset()
                        .map_err(|failure| self.contract_error(ErrorKind::Storage, failure))?;
                    self.take_speaker_trace();
                    return Ok(Stop::Halted);
                }
                Err(failure) => {
                    self.status = Status::Failed;
                    // The core refuses without a guest effect, so the CPU still
                    // holds the state of the refused instruction.
                    if let machine::MachineError::Cpu(refusal) = &failure {
                        self.cpu_stop_report = self.assembled.machine.cpu().stop_report(refusal);
                    }
                    self.take_speaker_trace();
                    let error = error(ErrorKind::Guest, &failure);
                    self.guest_failure = Some(failure);
                    return Err(error);
                }
            }
        }
    }

    /// Captures the display at the completed boundary and rasterises it with
    /// the bundled fonts; after an error the last picture stays readable.
    fn frame(&mut self, display: DisplayId) -> Result<Frame<'_>, Error> {
        let index = usize::from(display.0);
        let device = self
            .displays
            .get(index)
            .ok_or_else(|| Error::new(ErrorKind::Unsupported, "no such display"))?
            .device;
        self.assembled
            .capture_text(device)
            .map_err(|e| error(ErrorKind::Unsupported, e))?;
        let display = &mut self.displays[index];
        if let Some(picture) = display.sink.latest.borrow_mut().take() {
            // An idle screen rasterises to the same pixels on every capture;
            // only a changed picture gets a new serial.
            if display.picture.as_ref() != Some(&picture) {
                display.picture = Some(picture);
                self.serial = self.serial.wrapping_add(1);
            }
        }
        if let Some(failure) = display.sink.error() {
            return Err(error(ErrorKind::Unsupported, failure));
        }
        let picture = display
            .picture
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Usage, "no picture yet"))?;
        Ok(Frame::new(
            picture.width,
            picture.height,
            &picture.pixels,
            (4, 3),
            self.serial,
        ))
    }

    /// Checks the journals first, then renders the sound up to `now()`.
    fn drain_audio(&mut self, out: &mut Vec<[i16; 2]>) -> Result<(), Error> {
        if let Err(failure) = self.assembled.check_persistence() {
            return Err(self.storage(failure));
        }
        let captured = self.assembled.capture_audio();
        out.append(&mut self.audio.frames.borrow_mut());
        captured.map_err(|e| error(ErrorKind::Unsupported, e))
    }

    fn set_outputs(&mut self, outputs: Outputs) {
        self.audio.enabled.set(outputs.audio);
    }

    fn input(&mut self, event: InputEvent) {
        match event {
            InputEvent::Key { port, key, state } if port == KEYBOARD_PORT => {
                self.keyboard.key_event(key, state);
            }
            InputEvent::PointerMotion { port, dx, dy } if port == POINTER_PORT => {
                self.mouse.motion(dx, dy);
            }
            InputEvent::PointerButton {
                port,
                button,
                pressed,
            } if port == POINTER_PORT => self.mouse.button(button, pressed),
            InputEvent::ReleaseAll => {
                self.mouse.release();
                self.keyboard.release_all();
            }
            _ => {}
        }
    }

    /// The drives with their floppy catalogues: each entry's name and what
    /// becomes of the guest's writes, as the controller holds them. Digests,
    /// sizes and formats stay with the provider's machine config in this
    /// stage, so they are reported empty.
    fn slot_states(&mut self) -> Vec<SlotState> {
        let activity = std::mem::take(&mut self.activity);
        let takes_changes = matches!(self.status, Status::Ready | Status::Halted);
        let slots: Vec<(SlotId, bool)> = self
            .description
            .slots
            .iter()
            .map(|slot| (slot.id, slot.removable))
            .collect();
        slots
            .into_iter()
            .map(|(id, removable)| {
                let mut state = SlotState::new(id)
                    .with_active(activity[usize::from(id.0)])
                    .with_accepts_changes(removable && takes_changes);
                if id.0 < 2 {
                    let unit = usize::from(id.0);
                    let names = self
                        .assembled
                        .floppy_names
                        .get(&(DeviceId::Fdc, id.0))
                        .cloned()
                        .unwrap_or_default();
                    let writes: Vec<WriteKind> = self
                        .fdc()
                        .map(|fdc| {
                            fdc.catalogue(unit)
                                .map(|drive| {
                                    if drive.write_protected() {
                                        WriteKind::ReadOnly
                                    } else if drive.journaled() {
                                        WriteKind::Kept
                                    } else {
                                        WriteKind::Discarded
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    state = state
                        .with_catalogue(
                            names
                                .into_iter()
                                .zip(writes)
                                .map(|(name, writes)| MediumInfo::new(name, [0; 32], 0, "", writes))
                                .collect(),
                        )
                        .with_inserted(self.inserted_floppy(unit));
                }
                state
            })
            .collect()
    }

    /// Media join a running PC through the PC provider's own code in this
    /// stage; see [`PcMachine::assembled`].
    fn add_medium(&mut self, _slot: SlotId, _medium: NewMedium<'_>) -> Result<usize, Error> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "media join a running PC through its provider",
        ))
    }

    /// Changes a floppy drive now: between runs the machine is at a boundary.
    fn change_medium(&mut self, slot: SlotId, action: MediaAction) -> Result<(), Error> {
        let unit = slot.0;
        if unit > 1 {
            return Err(Error::new(ErrorKind::Unsupported, "not a floppy drive"));
        }
        let action = match action {
            MediaAction::Eject => assembly::FloppyAction::Eject,
            MediaAction::Insert(index) => assembly::FloppyAction::Insert(index),
            MediaAction::Next => assembly::FloppyAction::Next,
            _ => return Err(Error::new(ErrorKind::Unsupported, "unknown media action")),
        };
        self.assembled
            .change_floppy(DeviceId::Fdc, unit, action)
            .map_err(|failure| {
                let kind = match failure {
                    assembly::FloppyError::Unavailable => ErrorKind::Usage,
                    assembly::FloppyError::Persistence(_) => ErrorKind::Storage,
                    assembly::FloppyError::Config(_) => ErrorKind::Config,
                    _ => ErrorKind::Guest,
                };
                self.contract_error(kind, failure)
            })
    }

    /// The reset button of a halted PC. This asserts only the CPU reset line,
    /// the one the 8042 output-port pulse and the shutdown cycle assert (IBM
    /// Personal Computer AT Technical Reference, System Board, Keyboard
    /// Controller Output Port); unlike a hardware reset button it resets no
    /// board device. The CPU restarts at its reset vector, RAM, the devices
    /// and the inserted media keep their state, and POST initialises the
    /// board again. The reset takes no virtual time. Keys held or queued
    /// before the halt belong to the old boot; none of them reach the new
    /// POST.
    fn reset(&mut self, kind: ResetKind) -> Result<(), Error> {
        if kind != ResetKind::Soft {
            return Err(Error::new(ErrorKind::Unsupported, "only the reset button"));
        }
        if self.status != Status::Halted {
            return Err(Error::new(
                ErrorKind::Usage,
                "the reset button restarts only a halted machine",
            ));
        }
        // A failed reset leaves the machine halted, so its media can still
        // be finished.
        self.assembled
            .before_cpu_reset()
            .map_err(|failure| self.contract_error(ErrorKind::Storage, failure))?;
        self.assembled
            .reset_cpu(assembly::ResetCause::Host)
            .map_err(|failure| error(ErrorKind::Guest, failure))?;
        self.record_cpu_reset();
        self.take_speaker_trace();
        self.keyboard = HostKeyboard::default();
        self.status = Status::Ready;
        Ok(())
    }

    fn state_identity(&self) -> Option<String> {
        None
    }

    fn save_state(&mut self, _out: &mut dyn Write) -> Result<(), Error> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "a PC has no saved state yet",
        ))
    }

    fn load_state(&mut self, _state: &mut dyn Read) -> Result<(), Error> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "a PC has no saved state yet",
        ))
    }

    /// Seals every journal. Dropping the machine instead never writes a
    /// clean generation marker, and a failed execution stays unsealed.
    /// Journal maintenance that could not run is reported once, as warnings
    /// of a clean end or as messages of a failed one.
    fn finish(&mut self) -> Result<Finished, Error> {
        if self.status == Status::Failed {
            return Err(Error::new(
                ErrorKind::Usage,
                "a failed machine cannot finish cleanly",
            ));
        }
        let result = self.assembled.finish_persistence();
        let mut warnings = Vec::new();
        if !self.maintenance_reported {
            for handle in &self.assembled.persistence {
                if let Some(outcome) = handle.maintenance_failure() {
                    warnings.push(format!("{}: {outcome}", handle.owner()));
                }
            }
            self.maintenance_reported = true;
        }
        match result {
            Ok(()) => Ok(Finished::new().with_warnings(warnings)),
            Err(failure) => {
                for warning in warnings {
                    self.messages.push(Message::new(
                        MessageLevel::Warning,
                        format!("warning: {warning}"),
                    ));
                }
                Err(if self.contract_failure.is_some() {
                    self.storage(failure)
                } else {
                    error(ErrorKind::Storage, failure)
                })
            }
        }
    }

    fn drain_messages(&mut self, out: &mut Vec<Message>) {
        out.append(&mut self.messages);
    }
}

#[cfg(test)]
mod tests;
