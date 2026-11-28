// SPDX-License-Identifier: AGPL-3.0-or-later
//! Typed output routing, resolved before guest execution.
//!
//! The facade observes card-owned state and semantic emissions, never decodes
//! registers or replays accesses. See `backlog/docs/design/observation.md`,
//! §Typed capabilities and route ownership and §Sinks, bounds and lifecycle.
//! IBM VGA/XGA Technical Reference Manual, Graphics and Alphanumeric Modes,
//! and PC16550D datasheet,
//! Transmitter Holding Register, define the two producer contracts.

use std::{
    cell::{Ref, RefCell},
    collections::BTreeMap,
};

use audio::{AudioBlock, AudioError, AudioSink, AudioSource, CaptureAudioSink, Mixer};
use bus::Bus;
use cpu::Cpu;
use device::{ByteCapture, ByteOverflow, ByteStreamKind, Device};
use video::{
    render_graphics, render_planar_text, render_text_mode, CaptureFrameSink, Frame, FrameSink,
};

use crate::{AssembledMachine, AudioSinkKind, ConfigError, DeviceId, FrameSinkKind, OutputConfig};

#[cfg(test)]
#[path = "observation_audio_tests.rs"]
mod audio_tests;

/// An owned display at a completed machine service boundary.
///
/// Both counters come from the same machine as the card. Virtual time includes
/// halted PIT clocks; retired counts only executed instructions. See
/// `backlog/docs/design/observation.md`, §Deterministic boundaries and assertions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayCapture {
    /// Virtual time in instruction equivalents, including halted time.
    pub virtual_time: u64,
    /// Instructions retired by the CPU at this boundary.
    pub retired: u64,
    /// One rendered frame, shared by capture and presentation consumers.
    pub frame: Frame,
}

impl AssembledMachine {
    /// Attach a signal at the current PIT clock, including elapsed halted time.
    /// Future device assembly supplies shared signal history through this seam.
    pub fn add_audio_source(&mut self, source: Box<dyn AudioSource>) {
        self.observation
            .add_audio_source(self.machine.clock(), source);
    }

    /// Assert the CPU reset line as [`::machine::Machine::reset_cpu`] does. With
    /// [`ObservationPolicy::speaker_trace`] on, this also records the reset:
    /// its cause, CMOS byte 0Fh and the restart CS:IP. Without the trace it is
    /// exactly the machine's reset.
    pub fn reset_cpu(&mut self, cause: crate::ResetCause) -> Result<(), ::machine::MachineError> {
        let Some(trace) = self.observation.speaker_trace.clone() else {
            return self.machine.reset_cpu();
        };
        let before =
            crate::speaker_trace::SpeakerTrace::stamp(self.machine.cpu(), self.machine.clock());
        self.machine.reset_cpu()?;
        trace.reset(cause, before, self.machine.cpu());
        Ok(())
    }

    /// Settle lazy devices and deliver PCM through this completed service boundary.
    /// Direct callers of `machine.run_bounded` use this alongside display capture.
    pub fn capture_audio(&mut self) -> Result<(), AudioError> {
        let until = self.machine.clock();
        self.observation.capture_audio(&mut self.machine, until)
    }

    /// Capture a selected card with this machine's execution counters.
    /// Call after a completed service boundary, including an error stop.
    /// This presents the frame to subscribers and replaces the configured sink's
    /// last frame, without advancing devices or executing the CPU. The legacy
    /// method name also covers pixel frames when VGA selects graphics mode.
    pub fn capture_text(&self, device: DeviceId) -> Result<DisplayCapture, ObservationError> {
        Ok(DisplayCapture {
            virtual_time: self.machine.virtual_time(),
            retired: self.machine.cpu().retired(),
            frame: self.observation.capture_display(
                self.machine.bus(),
                Some(self.machine.cpu()),
                device,
            )?,
        })
    }

    /// Capture all selected displays from this machine at the same boundary.
    pub fn capture_displays(&self) -> Result<Vec<(DeviceId, DisplayCapture)>, ObservationError> {
        self.observation
            .routes()
            .iter()
            .filter(|route| route.output == OutputKind::TextScanout)
            .map(|route| {
                self.capture_text(route.device)
                    .map(|capture| (route.device, capture))
            })
            .collect()
    }
}

/// One frame of a Bochs VBE linear framebuffer (Bochs VBE DISPI interface,
/// `vbe.h`), read once from the CPU's video memory: indexed through the DAC
/// `palette` at 8 bpp, the guest's own bytes in a direct-colour frame above.
/// `None` when the CPU cannot read the visible bytes.
pub(crate) fn linear_frame(
    scanout: &devices::LinearScanout,
    memory: &cpu::X86JitCpu,
    palette: &[[u8; 3]; 256],
) -> Option<Frame> {
    let mut bytes = vec![0; usize::try_from(scanout.byte_len()).ok()?];
    memory
        .read_mem(devices::bochs_vbe::LFB_BASE + scanout.offset, &mut bytes)
        .ok()?;
    Some(linear_pixels(scanout, bytes, palette))
}

/// [`linear_frame`] over bytes already read from the display start.
fn linear_pixels(
    scanout: &devices::LinearScanout,
    bytes: Vec<u8>,
    palette: &[[u8; 3]; 256],
) -> Frame {
    let (width, height) = (scanout.width, scanout.height);
    let format = match scanout.bits_per_pixel {
        15 => video::PixelFormat::Rgb555,
        16 => video::PixelFormat::Rgb565,
        24 => video::PixelFormat::Rgb888,
        32 => video::PixelFormat::Xrgb8888,
        _ => {
            let stride = scanout.stride as usize;
            let mut indices = Vec::with_capacity(usize::from(width) * usize::from(height));
            for row in 0..usize::from(height) {
                indices.extend_from_slice(&bytes[row * stride..][..usize::from(width)]);
            }
            return Frame::Pixels(video::IndexedPixels {
                width,
                height,
                repeat: (1, 1),
                indices,
                palette: palette.to_vec().into_boxed_slice(),
            });
        }
    };
    Frame::Direct(video::DirectPixels {
        width,
        height,
        repeat: (1, 1),
        format,
        stride: scanout.stride,
        bytes,
    })
}

/// The requested capability, independent of the device's role or port address.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum OutputKind {
    /// Passive display scanout; VGA selects text or graphics from its AC mode.
    TextScanout,
    /// Accepted bytes with the specified stream semantics.
    Bytes(ByteStreamKind),
}

/// One explicitly selected output and its capture bound.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ObservationRequest {
    /// Stable role to resolve at assembly time.
    pub device: DeviceId,
    /// Capability the configured device must support.
    pub output: OutputKind,
    /// Maximum total stream bytes per capture epoch; ignored for display scanout.
    /// Drains release memory without renewing the budget.
    /// Overflow fails observation until a new epoch, without stopping hardware.
    pub byte_limit: usize,
}

/// Run-scoped observation choices, separate from guest hardware identity.
///
/// The default selects no routes or presentation subscribers and allocates no
/// stream history. [`Observation::bind`] applies the presentation selection for
/// inline hosts and assembly alike. This policy does not select CPU cooperation
/// size or schedule capture boundaries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObservationPolicy {
    /// Initial display and audio subscribers installed by [`Observation::bind`].
    /// Defaults to Headless video and None audio. This does not add routes.
    pub output: OutputConfig,
    /// Access diagnostics for every device and the open bus, independent of routes.
    /// Defaults to Off, including when semantic output capture is requested.
    pub trace: device::TracePolicy,
    /// Outputs to resolve and bind before guest execution.
    pub routes: Vec<ObservationRequest>,
    /// Host diagnostic: record speaker starts, tone changes and CPU resets with
    /// the guest CS:IP, drained through [`Observation::take_speaker_trace`].
    /// Off by default; off, assembly installs nothing for it.
    pub speaker_trace: bool,
}

/// An output whose device capability was checked during assembly.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ObservationRoute {
    /// Stable device role requested by the host.
    pub device: DeviceId,
    /// Validated output channel.
    pub output: OutputKind,
    /// Resolved bus slot, independent of device ordering in other presets.
    pub device_index: usize,
}

/// A failed observation must not masquerade as an empty frame or stream.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ObservationError {
    /// The caller did not request this output at assembly time.
    #[error("observation route is not bound: {device:?} {output:?}")]
    RouteNotBound {
        /// Requested role.
        device: DeviceId,
        /// Requested channel.
        output: OutputKind,
    },
    /// A bus no longer exposes the capability validated during assembly.
    #[error("text scanout is unavailable for bound device {0:?}")]
    ScanoutUnavailable(DeviceId),
    /// A text-only consumer requested a display currently producing pixels.
    #[error("display is in graphics mode: {0:?}")]
    GraphicsMode(DeviceId),
    /// Captured bytes exceeded the route's explicit limit.
    #[error("observation overflow for {device:?} {kind:?}: {source}")]
    ByteOverflow {
        /// Producing role.
        device: DeviceId,
        /// Stream whose capture is incomplete.
        kind: ByteStreamKind,
        /// Latched capture failure.
        source: ByteOverflow,
    },
}

/// Assembly-owned routes, byte collectors, display subscribers and audio mixer.
///
/// Each display has an optional last-frame capture sink and external boxed
/// `FrameSink` subscribers, delivered in that order and then subscription order.
///
/// Render once at a completed service boundary, then distribute the owned
/// frames/batches to consumers. There is no device drain or output downcast.
/// Hardware resets keep collectors installed; the host explicitly starts a new
/// capture epoch. See `backlog/docs/design/observation.md`, §Typed capabilities
/// and route ownership and §Sinks, bounds and lifecycle.
#[derive(Default)]
pub struct Observation {
    routes: Vec<ObservationRoute>,
    streams: BTreeMap<(DeviceId, ByteStreamKind), ByteCapture>,
    displays: BTreeMap<DeviceId, DisplaySubscribers>,
    /// DISPI interfaces of the bound displays that have one.
    linear: BTreeMap<DeviceId, devices::BochsVbe>,
    audio: RefCell<AudioObservation>,
    pub(crate) speaker_trace: Option<crate::speaker_trace::SpeakerTrace>,
}

#[derive(Default)]
struct AudioObservation {
    mixer: Option<Mixer>,
    capture: Option<CaptureAudioSink>,
    sinks: Vec<Box<dyn AudioSink>>,
}

impl AudioObservation {
    fn update_source_output(&mut self) {
        let enabled = self.enabled();
        if let Some(mixer) = &mut self.mixer {
            mixer.set_output_enabled(enabled);
        }
    }
}

impl AudioSink for AudioObservation {
    fn present(&mut self, block: AudioBlock<'_>) {
        if let Some(capture) = &mut self.capture {
            capture.present(block);
        }
        for sink in &mut self.sinks {
            if sink.enabled() {
                sink.present(block);
            }
        }
    }

    fn enabled(&self) -> bool {
        self.capture.is_some() || self.sinks.iter().any(|sink| sink.enabled())
    }
}

#[derive(Default)]
struct DisplaySubscribers {
    capture: RefCell<Option<CaptureFrameSink>>,
    sinks: RefCell<Vec<Box<dyn FrameSink>>>,
}

impl Observation {
    /// Resolve routes against a manually assembled device set before execution.
    /// This supports inline machines using the same collectors as config presets.
    /// Applies [`ObservationPolicy::output`] to the validated text routes.
    /// Later sink changes use [`Self::configure_output`].
    /// Missing roles, invalid slots and unsupported capabilities are config errors.
    pub fn bind(
        policy: &ObservationPolicy,
        device_index: &BTreeMap<DeviceId, usize>,
        devices: &mut [Box<dyn Device>],
    ) -> Result<Self, ConfigError> {
        let mut observation = Self::default();
        for request in &policy.routes {
            let device = request.device;
            let output = request.output;
            if observation
                .routes
                .iter()
                .any(|route| route.device == device && route.output == output)
            {
                return Err(ConfigError::DuplicateObservationRoute { device, output });
            }
            let index = device_index
                .get(&device)
                .copied()
                .ok_or(ConfigError::UnsupportedObservationOutput { device, output })?;
            let producer_device = devices
                .get_mut(index)
                .ok_or(ConfigError::UnsupportedObservationOutput { device, output })?;
            match output {
                OutputKind::TextScanout => {
                    if let Some(vbe) = producer_device
                        .as_any_mut()
                        .and_then(|any| any.downcast_mut::<devices::Vga>())
                        .and_then(|vga| vga.bochs_vbe())
                    {
                        observation.linear.insert(device, vbe.clone());
                    }
                    if producer_device.text_scanout().is_none()
                        && producer_device.planar_text_scanout().is_none()
                        && producer_device.graphics_scanout().is_none()
                    {
                        return Err(ConfigError::UnsupportedObservationOutput { device, output });
                    }
                }
                OutputKind::Bytes(kind) => {
                    let producer = producer_device
                        .byte_output()
                        .filter(|producer| producer.kind() == kind)
                        .ok_or(ConfigError::UnsupportedObservationOutput { device, output })?;
                    let capture = ByteCapture::new(request.byte_limit);
                    producer.bind_sink(Box::new(capture.clone()));
                    observation.streams.insert((device, kind), capture);
                }
            }
            observation.routes.push(ObservationRoute {
                device,
                output,
                device_index: index,
            });
        }
        observation.configure_output(policy.output);
        Ok(observation)
    }

    /// Select display and audio output, including inline hosts using [`Self::bind`].
    /// Reconfiguration clears configured captures and preserves external subscribers
    /// and audio phase; it does not add routes. Flush audio to the desired epoch
    /// boundary before calling this method.
    pub fn configure_output(&mut self, output: OutputConfig) {
        self.audio.get_mut().capture = match output.audio_sink {
            AudioSinkKind::None => None,
            AudioSinkKind::Capture => Some(CaptureAudioSink::with_frame_limit(
                output.audio_capture_frame_limit,
            )),
        };
        self.audio.get_mut().update_source_output();
        for route in &self.routes {
            if route.output == OutputKind::TextScanout {
                let subscribers = self.displays.entry(route.device).or_default();
                *subscribers.capture.get_mut() = match output.frame_sink {
                    FrameSinkKind::Headless => None,
                    FrameSinkKind::Capture => Some(CaptureFrameSink::new()),
                };
            }
        }
    }

    /// Attach source history with an explicit PIT origin. The mixer is allocated
    /// only for the first producer; configuring capture alone never creates it.
    pub fn add_audio_source(&mut self, origin: device::VClock, source: Box<dyn AudioSource>) {
        self.audio
            .get_mut()
            .mixer
            .get_or_insert_with(|| Mixer::new(origin))
            .add_source(origin, source);
        self.audio.get_mut().update_source_output();
    }

    /// Subscribe to mixed PCM at explicit observation boundaries. No device or
    /// display route is added. The same nonblocking contract as video applies.
    pub fn subscribe_audio(&mut self, sink: Box<dyn AudioSink>) {
        self.audio.get_mut().sinks.push(sink);
        self.audio.get_mut().update_source_output();
    }

    /// Settle device history, then render through an already completed PIT boundary.
    /// With no producer this is a no-op and does not manufacture silence.
    pub fn capture_audio<C: Cpu>(
        &self,
        machine: &mut ::machine::Machine<C>,
        until: device::VClock,
    ) -> Result<(), AudioError> {
        #[cfg(feature = "host-profile")]
        let _span = machine
            .bus()
            .profile()
            .span(device::profile::ProfilePhase::Observation);
        if until > machine.clock() {
            return Err(AudioError::FutureTime);
        }
        let mut state = self.audio.borrow_mut();
        state.update_source_output();
        if let Some(mut mixer) = state.mixer.take() {
            machine.bus_mut().settle();
            let result = mixer.render_until(until, &mut *state);
            state.mixer = Some(mixer);
            result?;
        }
        Ok(())
    }

    /// Whether assembly built the speaker trace ([`ObservationPolicy::speaker_trace`]).
    #[must_use]
    pub fn speaker_trace_enabled(&self) -> bool {
        self.speaker_trace.is_some()
    }

    /// Take the speaker and reset events recorded so far, in order. `now` is
    /// the machine clock; it closes a finished second's count of left-out
    /// lines. Without the trace this returns an empty list and allocates nothing.
    #[must_use]
    pub fn take_speaker_trace(&self, now: device::VClock) -> Vec<crate::SpeakerTraceEvent> {
        self.speaker_trace
            .as_ref()
            .map_or_else(Vec::new, |trace| trace.take(now))
    }

    /// Borrow the configured capture epoch without cloning retained PCM.
    /// None means capture is disabled; a producer-free machine has an empty capture.
    #[must_use]
    pub fn captured_audio(&self) -> Option<Ref<'_, CaptureAudioSink>> {
        Ref::filter_map(self.audio.borrow(), |state| state.capture.as_ref()).ok()
    }

    /// Subscribe a host presenter to a bound display, without lending it the
    /// machine. Sinks must return promptly, use no host timing in execution and
    /// never call back into the machine. They receive only the captured frame,
    /// after the configured capture sink, in subscription order.
    pub fn subscribe(
        &mut self,
        device: DeviceId,
        sink: Box<dyn FrameSink>,
    ) -> Result<(), ObservationError> {
        self.text_route(device)?;
        self.displays
            .entry(device)
            .or_default()
            .sinks
            .get_mut()
            .push(sink);
        Ok(())
    }

    /// Number of presentation subscribers, including the configured capture sink.
    ///
    /// Do not call this from inside a sink's `present`: the subscriber list is
    /// borrowed while frames are delivered.
    #[must_use]
    pub fn subscriber_count(&self, device: DeviceId) -> usize {
        self.displays.get(&device).map_or(0, |subscribers| {
            subscribers.sinks.borrow().len() + usize::from(subscribers.capture.borrow().is_some())
        })
    }

    /// Last frame delivered to the configured capture sink; absent in headless
    /// mode or before the first capture. Each display retains at most one frame.
    #[must_use]
    pub fn captured_frame(&self, device: DeviceId) -> Option<Frame> {
        self.displays
            .get(&device)?
            .capture
            .borrow()
            .as_ref()?
            .last()
            .cloned()
    }

    pub(crate) fn text_route(
        &self,
        device: DeviceId,
    ) -> Result<&ObservationRoute, ObservationError> {
        let output = OutputKind::TextScanout;
        self.routes
            .iter()
            .find(|route| route.device == device && route.output == output)
            .ok_or(ObservationError::RouteNotBound { device, output })
    }

    /// The stable role/output table resolved during assembly, in request order.
    #[must_use]
    pub fn routes(&self) -> &[ObservationRoute] {
        &self.routes
    }

    /// Render one selected display and fan out the owned frame to its capture
    /// sink, then external subscribers in subscription order. Failed captures
    /// feed no sink. This does not execute or synchronize hardware.
    #[cfg(test)]
    pub(crate) fn capture_text(
        &self,
        bus: &Bus,
        device: DeviceId,
    ) -> Result<Frame, ObservationError> {
        self.capture_display(bus, None, device)
    }

    /// [`Self::capture_text`], reading a linear framebuffer from `memory`
    /// while the card shows one. Without `memory` such a card shows the
    /// blank screen its VGA scanout reports.
    pub(crate) fn capture_display(
        &self,
        bus: &Bus,
        memory: Option<&cpu::X86JitCpu>,
        device: DeviceId,
    ) -> Result<Frame, ObservationError> {
        #[cfg(feature = "host-profile")]
        let _span = bus.profile().span(device::profile::ProfilePhase::Frame);
        let route = self.text_route(device)?;
        let producer = bus
            .device(route.device_index)
            .ok_or(ObservationError::ScanoutUnavailable(device))?;
        let linear = self
            .linear
            .get(&device)
            .and_then(|vbe| vbe.linear_scanout())
            .zip(memory);
        let frame = if let Some((scanout, memory)) = linear {
            let palette = producer
                .graphics_scanout()
                .ok_or(ObservationError::ScanoutUnavailable(device))?
                .palette;
            linear_frame(&scanout, memory, &palette)
                .ok_or(ObservationError::ScanoutUnavailable(device))?
        } else if let Some(state) = producer.graphics_scanout() {
            Frame::Pixels(render_graphics(&state))
        } else if let Some(state) = producer.planar_text_scanout() {
            Frame::Text(render_planar_text(&state))
        } else {
            let state = producer
                .text_scanout()
                .ok_or(ObservationError::ScanoutUnavailable(device))?;
            Frame::Text(render_text_mode(&state))
        };
        if let Some(subscribers) = self.displays.get(&device) {
            if let Some(capture) = &mut *subscribers.capture.borrow_mut() {
                capture.present(&frame);
            }
            for sink in &mut *subscribers.sinks.borrow_mut() {
                sink.present(&frame);
            }
        }
        Ok(frame)
    }

    /// Drain one route's complete byte batch for distribution by the host.
    /// Overflow remains a failure on every drain until a new capture epoch.
    pub fn drain_bytes(
        &self,
        device: DeviceId,
        kind: ByteStreamKind,
    ) -> Result<Vec<u8>, ObservationError> {
        self.streams
            .get(&(device, kind))
            .ok_or(ObservationError::RouteNotBound {
                device,
                output: OutputKind::Bytes(kind),
            })?
            .drain()
            .map_err(|source| ObservationError::ByteOverflow {
                device,
                kind,
                source,
            })
    }

    /// Start a fresh capture epoch without resetting guest hardware.
    /// Configured last-frame sinks are cleared; external subscribers own their lifecycle.
    pub fn start_epoch(&self) {
        for subscribers in self.displays.values() {
            let mut capture = subscribers.capture.borrow_mut();
            if capture.is_some() {
                *capture = Some(CaptureFrameSink::new());
            }
        }
        for capture in self.streams.values() {
            capture.start_epoch();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assemble, assemble_with_observation, AssemblyError, DeviceConfig, DeviceKind, PortWindow,
        PostPortCfg,
    };

    fn request(device: DeviceId, output: OutputKind, byte_limit: usize) -> ObservationRequest {
        ObservationRequest {
            device,
            output,
            byte_limit,
        }
    }

    #[test]
    fn register_vga_display_route_switches_to_owned_pixels_and_back() {
        let mut config = crate::at_vga_text(vec![0xFA, 0xB0, 0x2A, 0xA2, 0, 0, 0xF4]);
        config.cpu.segments.ds = 0xA000;
        let card = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Vga)
            .unwrap();
        card.kind = DeviceKind::VgaRegisters(crate::VgaCfg {});
        card.ports = crate::VgaCfg::port_windows();
        config.memory.ram = vec![crate::RamRegion::new(0, 0xA0000)];
        config.memory.mmio = vec![crate::MmioWindow::new(0xA0000, 0x20000, DeviceId::Vga)];
        let policy = ObservationPolicy {
            output: OutputConfig {
                frame_sink: FrameSinkKind::Capture,
                ..Default::default()
            },
            routes: vec![request(DeviceId::Vga, OutputKind::TextScanout, 0)],
            ..Default::default()
        };
        let mut rig = assemble_with_observation(&config, &policy).unwrap();
        let text = rig.capture_text(DeviceId::Vga).unwrap();
        let index = rig.observation.routes()[0].device_index;
        let saved = rig.machine.bus().device(index).unwrap().save();
        let bus = rig.machine.bus_mut();
        bus.port_write(0x3C2, 1, 0x63).unwrap();
        for (port, index, value) in [
            (0x3C4, 1, 1),
            (0x3C4, 2, 15),
            (0x3C4, 4, 14),
            (0x3CE, 5, 0x40),
            (0x3CE, 6, 5),
            (0x3CE, 8, 255),
            (0x3D4, 1, 0x4f),
            (0x3D4, 7, 0x1f),
            (0x3D4, 9, 0x41),
            (0x3D4, 0x12, 0x8f),
            (0x3D4, 0x13, 0x28),
            (0x3D4, 0x14, 0x40),
            (0x3D4, 0x17, 0xa3),
        ] {
            bus.port_write(port, 2, index | (value << 8)).unwrap();
        }
        for index in 0..16 {
            bus.port_read(0x3DA, 1).unwrap();
            bus.port_write(0x3C0, 1, index).unwrap();
            bus.port_write(0x3C0, 1, index).unwrap();
        }
        for (index, value) in [(0x10, 0x41), (0x12, 15)] {
            bus.port_read(0x3DA, 1).unwrap();
            bus.port_write(0x3C0, 1, index).unwrap();
            bus.port_write(0x3C0, 1, value).unwrap();
        }
        bus.port_read(0x3DA, 1).unwrap();
        bus.port_write(0x3C0, 1, 0x20).unwrap();
        bus.port_write(0x3C6, 1, 255).unwrap();
        bus.port_write(0x3C8, 1, 0x2a).unwrap();
        for v in [63, 1, 2] {
            bus.port_write(0x3C9, 1, v).unwrap();
        }
        assert_eq!(
            rig.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        let before = rig.machine.bus().device(index).unwrap().save();
        let capture = rig.capture_text(DeviceId::Vga).unwrap();
        let Frame::Pixels(pixels) = &capture.frame else {
            panic!("expected pixels")
        };
        assert_eq!((pixels.width, pixels.height), (320, 200));
        let mut expected = vec![0; 64000];
        expected[0] = 0x2a;
        assert_eq!(pixels.indices, expected);
        assert_eq!(pixels.palette[0x2a], [255, 4, 8]);
        assert_eq!(rig.machine.bus().device(index).unwrap().save(), before);
        assert_eq!(
            rig.observation.captured_frame(DeviceId::Vga),
            Some(capture.frame.clone())
        );
        assert_eq!(
            rig.capture_displays().unwrap(),
            vec![(DeviceId::Vga, capture.clone())]
        );
        rig.machine
            .bus_mut()
            .device_mut(index)
            .unwrap()
            .restore(&saved)
            .unwrap();
        assert_eq!(rig.capture_text(DeviceId::Vga).unwrap().frame, text.frame);
        assert_eq!(pixels.indices[0], 0x2a);
    }

    #[test]
    fn register_vga_routes_planar_text_on_a_bare_rig() {
        let mut config = crate::at_vga_text(vec![0xF4]);
        let card = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Vga)
            .unwrap();
        card.kind = crate::DeviceKind::VgaRegisters(crate::VgaCfg {});
        card.ports = crate::VgaCfg::port_windows();
        config.memory.ram = vec![crate::RamRegion::new(0, 0xA0000)];
        config.memory.mmio = vec![crate::MmioWindow::new(0xA0000, 0x20000, DeviceId::Vga)];
        let policy = ObservationPolicy {
            routes: vec![request(DeviceId::Vga, OutputKind::TextScanout, 0)],
            ..Default::default()
        };
        let mut rig = assemble_with_observation(&config, &policy).unwrap();
        let before = rig.capture_text(DeviceId::Vga).unwrap();
        let Frame::Text(grid) = before.frame else {
            panic!("expected text frame")
        };
        assert!(!grid.appearance().unwrap().enabled);
        let bus = rig.machine.bus_mut();
        bus.port_write(0x3C2, 1, 0x67).unwrap();
        for (port, index, value) in [
            (0x3C4, 2, 3),
            (0x3C4, 4, 2),
            (0x3CE, 5, 0x10),
            (0x3CE, 6, 0x0E),
            (0x3CE, 8, 255),
            (0x3D4, 1, 1),
            (0x3D4, 9, 15),
            (0x3D4, 0x12, 15),
            (0x3D4, 0x13, 1),
        ] {
            bus.port_write(port, 1, index).unwrap();
            bus.port_write(port + 1, 1, value).unwrap();
        }
        bus.port_read(0x3DA, 1).unwrap();
        bus.port_write(0x3C0, 1, 0x20).unwrap();
        bus.mmio_write(0xB8000, 2, 0x0741).unwrap();
        let frame = rig.capture_text(DeviceId::Vga).unwrap();
        let Frame::Text(grid) = &frame.frame else {
            panic!("expected text frame")
        };
        assert_eq!((grid.cols(), grid.rows()), (2, 1));
        assert_eq!(
            (
                grid.cell(0, 0).unwrap().glyph,
                grid.cell(0, 0).unwrap().attr
            ),
            (b'A', 7)
        );
        assert!(grid.appearance().unwrap().enabled);
        assert_eq!(rig.capture_text(DeviceId::Vga).unwrap(), frame);
        assert_eq!((frame.retired, frame.virtual_time), (0, 0));
    }

    #[test]
    fn debug_console_captures_guest_bytes_on_a_bare_rig() {
        // mov dx,0402h; in al,dx; cmp al,E9h; jne halt;
        // mov al,'H'; out dx,al; mov al,'i'; out dx,al; mov al,10; out dx,al; hlt
        let program = vec![
            0xBA, 0x02, 0x04, 0xEC, 0x3C, 0xE9, 0x75, 0x09, 0xB0, b'H', 0xEE, 0xB0, b'i', 0xEE,
            0xB0, b'\n', 0xEE, 0xF4,
        ];
        let mut config = crate::at_boot(program);
        assert_eq!(config.board, crate::BoardModel::Bare);
        config.devices.push(DeviceConfig::new(
            DeviceId::DebugConsole,
            DeviceKind::DebugConsole(crate::DebugConsoleCfg::default()),
            vec![PortWindow::single(0x402)],
        ));
        config.devices.reverse();
        let policy = ObservationPolicy {
            routes: vec![request(
                DeviceId::DebugConsole,
                OutputKind::Bytes(ByteStreamKind::DebugConsole),
                3,
            )],
            ..Default::default()
        };
        let mut assembled = assemble_with_observation(&config, &policy).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(32)).unwrap(),
            machine::RunOutcome::Halted
        );
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
                .unwrap(),
            b"Hi\n"
        );
        assert!(assembled
            .observation
            .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn debug_console_route_requires_its_device_and_stream_kind() {
        let mut config = crate::at_boot(vec![0xF4]);
        let mut policy = ObservationPolicy {
            routes: vec![request(
                DeviceId::DebugConsole,
                OutputKind::Bytes(ByteStreamKind::DebugConsole),
                8,
            )],
            ..Default::default()
        };
        assert!(matches!(
            assemble_with_observation(&config, &policy),
            Err(AssemblyError::Config(
                ConfigError::UnsupportedObservationOutput {
                    device: DeviceId::DebugConsole,
                    output: OutputKind::Bytes(ByteStreamKind::DebugConsole),
                }
            ))
        ));
        config.devices.push(DeviceConfig::new(
            DeviceId::DebugConsole,
            DeviceKind::DebugConsole(crate::DebugConsoleCfg::default()),
            vec![PortWindow::single(0x402)],
        ));
        policy.routes[0].output = OutputKind::Bytes(ByteStreamKind::SerialTx);
        assert!(matches!(
            assemble_with_observation(&config, &policy),
            Err(AssemblyError::Config(
                ConfigError::UnsupportedObservationOutput {
                    device: DeviceId::DebugConsole,
                    output: OutputKind::Bytes(ByteStreamKind::SerialTx),
                }
            ))
        ));
    }

    #[test]
    fn debug_console_wide_bus_accesses_use_consecutive_byte_cycles() {
        let mut config = crate::at_boot(vec![0xF4]);
        config.devices.push(DeviceConfig::new(
            DeviceId::DebugConsole,
            DeviceKind::DebugConsole(crate::DebugConsoleCfg::default()),
            vec![PortWindow::single(0x402)],
        ));
        let mut assembled = assemble_with_observation(
            &config,
            &ObservationPolicy {
                routes: vec![request(
                    DeviceId::DebugConsole,
                    OutputKind::Bytes(ByteStreamKind::DebugConsole),
                    8,
                )],
                ..Default::default()
            },
        )
        .unwrap();
        let bus = assembled.machine.bus_mut();
        assert_eq!(bus.port_read(0x402, 2).unwrap(), 0xFFE9);
        assert_eq!(bus.port_read(0x402, 4).unwrap(), 0xFFFFFFE9);
        bus.port_write(0x402, 2, 0x4241).unwrap();
        bus.port_write(0x402, 4, 0x46454443).unwrap();
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)
                .unwrap(),
            b"AC"
        );
    }

    #[test]
    fn bind_applies_policy_capture_without_reconfiguration() {
        let mut devices: Vec<Box<dyn Device>> = vec![Box::new(devices::VgaText::new())];
        devices[0].mmio_write(0xB8000, 2, 0x7049).unwrap();
        let observation = Observation::bind(
            &ObservationPolicy {
                output: OutputConfig {
                    frame_sink: FrameSinkKind::Capture,
                    ..Default::default()
                },
                routes: vec![request(DeviceId::Vga, OutputKind::TextScanout, 0)],
                ..Default::default()
            },
            &BTreeMap::from([(DeviceId::Vga, 0)]),
            &mut devices,
        )
        .unwrap();
        assert_eq!(observation.subscriber_count(DeviceId::Vga), 1);
        assert_eq!(observation.captured_frame(DeviceId::Vga), None);
        let bus = Bus::new(devices, bus::DecodeMap::default());
        let frame = observation.capture_text(&bus, DeviceId::Vga).unwrap();
        let Frame::Text(grid) = &frame else {
            panic!("expected text frame")
        };
        assert_eq!(grid.cell(0, 0).unwrap().glyph, b'I');
        assert_eq!(grid.cell(0, 0).unwrap().attr, 0x70);
        assert_eq!(observation.captured_frame(DeviceId::Vga), Some(frame));
        assert_eq!(observation.subscriber_count(DeviceId::Vga), 1);
    }

    #[test]
    fn display_fanout_updates_capture_before_external_subscription_order() {
        use std::rc::{Rc, Weak};

        struct OrderedSink {
            observation: Weak<Observation>,
            log: Rc<RefCell<Vec<(u8, Frame)>>>,
            id: u8,
        }
        impl FrameSink for OrderedSink {
            fn present(&mut self, frame: &Frame) {
                let observation = self.observation.upgrade().unwrap();
                assert_eq!(
                    observation.captured_frame(DeviceId::Vga).as_ref(),
                    Some(frame)
                );
                self.log.borrow_mut().push((self.id, frame.clone()));
            }
        }

        let log = Rc::new(RefCell::new(Vec::new()));
        let mut devices: Vec<Box<dyn Device>> = vec![Box::new(devices::VgaText::new())];
        let observation = Rc::new_cyclic(|weak| {
            let mut observation = Observation::bind(
                &ObservationPolicy {
                    routes: vec![request(DeviceId::Vga, OutputKind::TextScanout, 0)],
                    ..Default::default()
                },
                &BTreeMap::from([(DeviceId::Vga, 0)]),
                &mut devices,
            )
            .unwrap();
            observation.configure_output(OutputConfig {
                frame_sink: FrameSinkKind::Capture,
                ..Default::default()
            });
            for id in [1, 2] {
                observation
                    .subscribe(
                        DeviceId::Vga,
                        Box::new(OrderedSink {
                            observation: weak.clone(),
                            log: log.clone(),
                            id,
                        }),
                    )
                    .unwrap();
            }
            observation
        });
        let mut bus = Bus::new(devices, bus::DecodeMap::default());
        let mut expected = Vec::new();
        for word in [0x0741, 0x7042] {
            bus.device_mut(0)
                .unwrap()
                .mmio_write(0xB8000, 2, word)
                .unwrap();
            let frame = observation.capture_text(&bus, DeviceId::Vga).unwrap();
            expected.extend([(1, frame.clone()), (2, frame)]);
        }
        assert_eq!(*log.borrow(), expected);
        assert_eq!(observation.subscriber_count(DeviceId::Vga), 3);
    }

    #[test]
    fn display_subscription_rejects_bytes_only_route() {
        struct NoFrames;
        impl FrameSink for NoFrames {
            fn present(&mut self, _: &Frame) {
                panic!("byte routes cannot emit frames");
            }
        }
        let config = crate::at_boot(vec![0xF4]);
        let mut assembled = assemble_with_observation(
            &config,
            &ObservationPolicy {
                routes: vec![request(
                    DeviceId::Uart,
                    OutputKind::Bytes(ByteStreamKind::SerialTx),
                    8,
                )],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            assembled
                .observation
                .subscribe(DeviceId::Uart, Box::new(NoFrames)),
            Err(ObservationError::RouteNotBound {
                device: DeviceId::Uart,
                output: OutputKind::TextScanout
            })
        );
        assert_eq!(assembled.observation.subscriber_count(DeviceId::Uart), 0);
    }

    #[test]
    fn failed_display_captures_preserve_sinks_without_delivery() {
        use std::rc::Rc;

        struct Recorder(Rc<RefCell<Vec<Frame>>>);
        impl FrameSink for Recorder {
            fn present(&mut self, frame: &Frame) {
                self.0.borrow_mut().push(frame.clone());
            }
        }
        let config = crate::at_vga_text(vec![0xF4]);
        let mut assembled = assemble_with_observation(
            &config,
            &ObservationPolicy {
                output: OutputConfig {
                    frame_sink: FrameSinkKind::Capture,
                    ..Default::default()
                },
                routes: vec![request(DeviceId::Vga, OutputKind::TextScanout, 0)],
                ..Default::default()
            },
        )
        .unwrap();
        let frames = Rc::new(RefCell::new(Vec::new()));
        assembled
            .observation
            .subscribe(DeviceId::Vga, Box::new(Recorder(frames.clone())))
            .unwrap();
        let frame = assembled.capture_text(DeviceId::Vga).unwrap().frame;
        let index = assembled.device_index[&DeviceId::Vga];
        *assembled.machine.bus_mut().device_mut(index).unwrap() = Box::new(devices::I8042::new());
        assert_eq!(
            assembled.capture_text(DeviceId::Vga),
            Err(ObservationError::ScanoutUnavailable(DeviceId::Vga))
        );
        assert_eq!(
            assembled.capture_text(DeviceId::Mda),
            Err(ObservationError::RouteNotBound {
                device: DeviceId::Mda,
                output: OutputKind::TextScanout,
            })
        );
        assert_eq!(
            assembled.observation.captured_frame(DeviceId::Vga),
            Some(frame.clone())
        );
        assert_eq!(*frames.borrow(), vec![frame]);
    }

    #[test]
    fn output_selection_routes_subscribers_and_resets_capture_epochs() {
        use std::rc::Rc;

        struct Recorder(Rc<RefCell<Vec<Frame>>>);
        impl FrameSink for Recorder {
            fn present(&mut self, frame: &Frame) {
                self.0.borrow_mut().push(frame.clone());
            }
        }

        for kind in [FrameSinkKind::Headless, FrameSinkKind::Capture] {
            let mut config = crate::bios(vec![0; 0x8000], None);
            config.cpu.reset = crate::ResetSpec::Entry { cs: 0xF800, ip: 0 };
            config.images[0].source = crate::ImageSource::Inline({
                let mut rom = vec![0; 0x8000];
                rom[..4].copy_from_slice(&[0x90, 0x90, 0xFB, 0xF4]);
                rom
            });
            config.board = crate::BoardModel::Bare;
            config.devices.reverse();
            let policy = ObservationPolicy {
                output: OutputConfig {
                    frame_sink: kind,
                    ..Default::default()
                },
                routes: vec![
                    request(DeviceId::Mda, OutputKind::TextScanout, 0),
                    request(DeviceId::Vga, OutputKind::TextScanout, 0),
                ],
                ..Default::default()
            };
            let mut assembled = assemble_with_observation(&config, &policy).unwrap();
            let count = usize::from(kind == FrameSinkKind::Capture);
            for device in [DeviceId::Mda, DeviceId::Vga] {
                assert_eq!(assembled.observation.subscriber_count(device), count);
                assert_eq!(assembled.observation.captured_frame(device), None);
            }
            let frames = Rc::new(RefCell::new(Vec::new()));
            assert!(matches!(
                assembled
                    .observation
                    .subscribe(DeviceId::Keyboard, Box::new(Recorder(frames.clone()))),
                Err(ObservationError::RouteNotBound { .. })
            ));
            assembled
                .observation
                .subscribe(DeviceId::Mda, Box::new(Recorder(frames.clone())))
                .unwrap();
            assert_eq!(
                assembled.observation.subscriber_count(DeviceId::Mda),
                count + 1
            );
            assembled.machine.set_idle_limit(Some(20));
            assert_eq!(
                assembled.machine.run_bounded(Some(7)).unwrap(),
                machine::RunOutcome::RetiredCapReached
            );
            assert_eq!(assembled.machine.cpu().retired(), 4);
            assert_eq!(assembled.machine.virtual_time(), 7);
            let index = assembled.device_index[&DeviceId::Mda];
            assembled
                .machine
                .bus_mut()
                .mmio_write(0xB0000, 2, 0x7041)
                .unwrap();
            let saved = assembled.machine.bus().device(index).unwrap().save();
            let first = assembled.capture_text(DeviceId::Mda).unwrap();
            assert_eq!((first.virtual_time, first.retired), (7, 4));
            assembled.machine.run_bounded(Some(9)).unwrap();
            assembled
                .machine
                .bus_mut()
                .mmio_write(0xB0000, 2, 0x0742)
                .unwrap();
            let final_captures = assembled.capture_displays().unwrap();
            assert_eq!(final_captures[0].0, DeviceId::Mda);
            assert_eq!(final_captures[1].0, DeviceId::Vga);
            assert_ne!(first.frame, final_captures[0].1.frame);
            assert_eq!(
                *frames.borrow(),
                vec![first.frame, final_captures[0].1.frame.clone()]
            );
            for (device, capture) in final_captures {
                assert_eq!(capture.virtual_time, 9);
                assert_eq!(capture.retired, 4);
                assert_eq!(
                    assembled.observation.captured_frame(device),
                    (kind == FrameSinkKind::Capture).then_some(capture.frame)
                );
            }
            assembled
                .machine
                .bus_mut()
                .device_mut(index)
                .unwrap()
                .restore(&saved)
                .unwrap();
            assert_eq!(
                assembled.capture_text(DeviceId::Mda).unwrap().frame,
                frames.borrow()[0]
            );
            assembled.observation.start_epoch();
            for device in [DeviceId::Mda, DeviceId::Vga] {
                assert_eq!(assembled.observation.captured_frame(device), None);
            }
            assert_eq!(
                assembled.observation.subscriber_count(DeviceId::Mda),
                count + 1
            );
            assert_eq!(frames.borrow().len(), 3);
        }
    }

    #[test]
    fn bios_mda_and_vga_have_independent_crtc_and_status_state() {
        let config = crate::bios(vec![0; 0x8000], None);
        let policy = ObservationPolicy {
            trace: device::TracePolicy::Off,
            routes: vec![
                request(DeviceId::Mda, OutputKind::TextScanout, 0),
                request(DeviceId::Vga, OutputKind::TextScanout, 0),
            ],
            ..Default::default()
        };
        let mut assembled = assemble_with_observation(&config, &policy).unwrap();
        let bus = assembled.machine.bus_mut();
        bus.mmio_write(0xB0000, 4, 0x70420741).unwrap();
        for (index, value) in [(13, 1), (15, 3), (10, 0x20)] {
            bus.port_write(0x3B4, 1, index).unwrap();
            bus.port_write(0x3B5, 1, value).unwrap();
        }
        let Frame::Text(mda) = assembled.capture_text(DeviceId::Mda).unwrap().frame else {
            panic!("expected text frame")
        };
        let Frame::Text(vga) = assembled.capture_text(DeviceId::Vga).unwrap().frame else {
            panic!("expected text frame")
        };
        assert_eq!(
            mda.cell(0, 0),
            Some(&video::TextCell {
                glyph: b'B',
                attr: 0x70,
                style: video::TextStyle::Monochrome {
                    effect: video::MonochromeEffect::Reverse,
                    intense: false,
                    blink: false,
                    background_intense: false
                }
            })
        );
        assert_eq!(
            mda.cursor(),
            video::Cursor {
                col: 2,
                row: 0,
                visible: false
            }
        );
        assert_eq!(
            vga.cell(0, 0),
            Some(&video::TextCell {
                glyph: 0,
                attr: 0,
                style: video::TextStyle::Colour {
                    foreground: 0,
                    background: 0,
                    blink: false
                }
            })
        );
        assert_eq!(
            vga.cursor(),
            video::Cursor {
                col: 0,
                row: 0,
                visible: true
            }
        );
        let bus = assembled.machine.bus_mut();
        for status in [0xF6, 0xF7, 0xFE, 0xFF] {
            assert_eq!(bus.port_read(0x3BA, 1).unwrap(), status);
        }
        assert_eq!(bus.port_read(0x3DA, 1).unwrap(), 0);
        for port in [0x3BC, 0x3BD, 0x3BE] {
            assert_eq!(bus.port_read(port, 1).unwrap(), 0xFF);
        }
    }

    #[test]
    fn bios_mda_guest_store_reaches_scanout_instead_of_ram() {
        let mut config = crate::bios(vec![0; 0x8000], None);
        let card = config
            .memory
            .mmio
            .iter()
            .find(|w| w.base == 0xB0000)
            .expect("the monochrome aperture must be card-owned MMIO")
            .device;
        assert_eq!(card, DeviceId::Mda);
        config.cpu.reset = crate::ResetSpec::Entry { cs: 0, ip: 0x7C00 };
        config.cpu.segments = crate::SegmentSeed::flat(0);
        config.cpu.segments.ds = 0xB000;
        config.images.push(crate::LoadImage {
            base: 0x7C00,
            source: crate::ImageSource::Inline(vec![0xFA, 0xB8, b'A', 7, 0xA3, 0, 0, 0xF4]),
            kind: crate::ImageKind::RamLoad,
            provides: Vec::new(),
        });
        let policy = ObservationPolicy {
            trace: device::TracePolicy::Ring { capacity: 16 },
            routes: vec![request(card, OutputKind::TextScanout, 0)],
            ..Default::default()
        };
        let mut assembled = assemble_with_observation(&config, &policy).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(100)).unwrap(),
            machine::RunOutcome::Halted
        );
        let capture = assembled.capture_text(card).unwrap();
        let Frame::Text(grid) = capture.frame else {
            panic!("expected text frame")
        };
        assert_eq!(
            grid.cell(0, 0),
            Some(&video::TextCell {
                glyph: b'A',
                attr: 7,
                style: video::TextStyle::Monochrome {
                    effect: video::MonochromeEffect::Normal,
                    intense: false,
                    blink: false,
                    background_intense: false
                }
            })
        );
        let mut bytes = [0; 2];
        assembled
            .machine
            .cpu()
            .read_mem(0xB0000, &mut bytes)
            .unwrap();
        assert_eq!(bytes, [0, 0], "guest MMIO must not mutate CPU backing RAM");
        let index = assembled.device_index[&card];
        assert!(assembled
            .machine
            .bus_mut()
            .device_mut(index)
            .unwrap()
            .take_trace()
            .iter()
            .any(|event| matches!(
                event,
                device::TraceEvent::Mmio {
                    addr: 0xB0000,
                    dir: device::AccessDir::Write,
                    value: 0x0741,
                    ..
                }
            )));
    }

    #[test]
    fn observation_routes_follow_device_ids_and_capture_one_running_machine() {
        let program = vec![
            0xBA, 0xF8, 0x03, 0xB0, b'A', 0xEE, 0xBA, 0x90, 0x01, 0xB0, 0xFF, 0xEE, 0xB8, b'B',
            0x1F, 0xA3, 0, 0, 0xF4,
        ];
        let mut config = crate::at_vga_text(program);
        config.devices.push(DeviceConfig::new(
            DeviceId::Post,
            DeviceKind::Post(PostPortCfg { port: 0x190 }),
            vec![PortWindow::single(0x190)],
        ));
        config.devices.reverse();
        let policy = ObservationPolicy {
            trace: device::TracePolicy::Off,
            routes: vec![
                request(DeviceId::Vga, OutputKind::TextScanout, 0),
                request(
                    DeviceId::Post,
                    OutputKind::Bytes(ByteStreamKind::PostCheckpoints),
                    8,
                ),
                request(
                    DeviceId::Uart,
                    OutputKind::Bytes(ByteStreamKind::SerialTx),
                    8,
                ),
            ],
            ..Default::default()
        };
        let mut assembled = assemble_with_observation(&config, &policy).unwrap();
        assert_eq!(assembled.observation.routes().len(), 3);
        for route in assembled.observation.routes() {
            assert_eq!(route.device_index, assembled.device_index[&route.device]);
        }
        assembled.machine.run().unwrap();
        let retired = assembled.machine.cpu().retired();
        let time = assembled.machine.virtual_time();
        let capture = assembled.capture_text(DeviceId::Vga).unwrap();
        assert_eq!(capture.virtual_time, time);
        assert_eq!(capture.retired, retired);
        assert_eq!(assembled.capture_text(DeviceId::Vga).unwrap(), capture);
        let frame = capture.frame;
        let Frame::Text(grid) = &frame else {
            panic!("expected text frame")
        };
        assert_eq!(
            grid.cell(0, 0).unwrap(),
            &video::TextCell {
                glyph: b'B',
                attr: 0x1F,
                style: video::TextStyle::Colour {
                    foreground: 15,
                    background: 1,
                    blink: false
                },
            }
        );
        assert_eq!((grid.cols(), grid.rows()), (80, 25));
        let mut sink = CaptureFrameSink::new();
        sink.present(&frame);
        assert_eq!(sink.last(), Some(&frame));
        assert_eq!(
            assembled.capture_displays().unwrap(),
            vec![(
                DeviceId::Vga,
                DisplayCapture {
                    virtual_time: time,
                    retired,
                    frame
                }
            )]
        );
        assert_eq!(assembled.machine.cpu().retired(), retired);
        assert_eq!(assembled.machine.virtual_time(), time);
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
                .unwrap(),
            b"A"
        );
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
                .unwrap(),
            [0xFF]
        );
        assert!(assembled
            .observation
            .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn stream_facade_preserves_divisor_banking_byte_cycles_and_reset_history() {
        let mut config = crate::at_boot(vec![0xF4]);
        config.devices.push(DeviceConfig::new(
            DeviceId::Post,
            DeviceKind::Post(PostPortCfg { port: 0x190 }),
            vec![PortWindow::single(0x190)],
        ));
        let policy = ObservationPolicy {
            trace: device::TracePolicy::Ring { capacity: 100 },
            routes: vec![
                request(
                    DeviceId::Uart,
                    OutputKind::Bytes(ByteStreamKind::SerialTx),
                    4,
                ),
                request(
                    DeviceId::Post,
                    OutputKind::Bytes(ByteStreamKind::PostCheckpoints),
                    5,
                ),
            ],
            ..Default::default()
        };
        let mut assembled = assemble_with_observation(&config, &policy).unwrap();
        let bus = assembled.machine.bus_mut();
        bus.port_write(0x3FB, 1, 0x83).unwrap();
        bus.port_write(0x3F8, 2, 0x0142).unwrap();
        bus.port_write(0x3FB, 1, 3).unwrap();
        bus.port_write(0x3F8, 2, 0x004F).unwrap();
        bus.port_write(0x3F8, 1, u32::from(b'K')).unwrap();
        for byte in [0, 0xFF] {
            bus.port_write(0x3F8, 1, byte).unwrap();
        }
        let checkpoints = [0x1234, 0x1234, 0, 0xFF];
        for value in checkpoints {
            bus.port_write(0x190, 4, value).unwrap();
        }
        // Reads add no checkpoint. 0x190 returns its latch, the low byte of
        // the last write; the undecoded 0x191 floats high.
        let latch = checkpoints[3] & 0xFF;
        assert_eq!(bus.port_read(0x190, 2).unwrap(), 0xFF00 | latch);
        for device in [DeviceId::Uart, DeviceId::Post] {
            let dev = bus.device_mut(assembled.device_index[&device]).unwrap();
            let state = dev.save();
            dev.take_trace();
            dev.reset();
            dev.restore(&state).unwrap();
        }
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
                .unwrap(),
            b"OK\0\xff"
        );
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
                .unwrap(),
            [0x34, 0x34, 0, 0xFF]
        );
        assembled
            .machine
            .bus_mut()
            .port_write(0x190, 1, 42)
            .unwrap();
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
                .unwrap(),
            [42]
        );
        assembled
            .machine
            .bus_mut()
            .port_write(0x190, 1, 43)
            .unwrap();
        assert!(matches!(
            assembled
                .observation
                .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints),
            Err(ObservationError::ByteOverflow { .. })
        ));
        assembled.observation.start_epoch();
        assembled.machine.bus_mut().port_write(0x190, 1, 7).unwrap();
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
                .unwrap(),
            [7]
        );
    }

    #[test]
    fn display_capture_stamps_halted_virtual_time_separately_from_retired() {
        let config = crate::at_vga_text(vec![0xFB, 0xF4]);
        let policy = ObservationPolicy {
            trace: device::TracePolicy::Off,
            routes: vec![request(DeviceId::Vga, OutputKind::TextScanout, 0)],
            ..Default::default()
        };
        let mut assembled = assemble_with_observation(&config, &policy).unwrap();
        crate::run(
            &mut assembled.machine,
            &assembled.observation,
            &crate::RunPolicy::natural(Some(10), Some(20)),
        )
        .unwrap();
        let capture = assembled.capture_text(DeviceId::Vga).unwrap();
        assert_eq!(capture.retired, assembled.machine.cpu().retired());
        assert_eq!(capture.virtual_time, assembled.machine.virtual_time());
        assert!(capture.virtual_time > capture.retired);
        assert_eq!(assembled.capture_text(DeviceId::Vga).unwrap(), capture);
        assert_eq!(
            assembled.capture_displays().unwrap(),
            vec![(DeviceId::Vga, capture)]
        );
    }

    #[test]
    fn observation_duplicate_route_is_a_configuration_error() {
        let config = crate::at_boot(vec![0xF4]);
        let route = request(
            DeviceId::Uart,
            OutputKind::Bytes(ByteStreamKind::SerialTx),
            8,
        );
        let policy = ObservationPolicy {
            trace: device::TracePolicy::Off,
            routes: vec![route, route],
            ..Default::default()
        };
        assert!(matches!(assemble_with_observation(&config, &policy),
            Err(AssemblyError::Config(ConfigError::DuplicateObservationRoute { device: DeviceId::Uart, output }))
                if output == route.output));
    }

    #[test]
    fn observation_unrequested_output_is_explicit_and_defaults_allocate_no_collectors() {
        let assembled = assemble(&crate::at_boot(vec![0xF4])).unwrap();
        assert!(assembled.observation.routes().is_empty());
        assert!(assembled.observation.streams.is_empty());
        assert!(matches!(
            assembled.capture_text(DeviceId::Vga),
            Err(ObservationError::RouteNotBound { .. })
        ));
        assert!(matches!(
            assembled
                .observation
                .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx),
            Err(ObservationError::RouteNotBound { .. })
        ));
        assert!(assembled
            .machine
            .bus()
            .device(assembled.device_index[&DeviceId::Pit])
            .unwrap()
            .text_scanout()
            .is_none());
    }

    #[test]
    fn observation_byte_overflow_fails_drains_until_explicit_epoch() {
        let config = crate::at_boot(vec![0xF4]);
        let policy = ObservationPolicy {
            trace: device::TracePolicy::Off,
            routes: vec![request(
                DeviceId::Uart,
                OutputKind::Bytes(ByteStreamKind::SerialTx),
                1,
            )],
            ..Default::default()
        };
        let mut assembled = assemble_with_observation(&config, &policy).unwrap();
        for byte in [0, 0xFF] {
            assembled
                .machine
                .bus_mut()
                .port_write(0x3F8, 1, byte)
                .unwrap();
        }
        let expected = Err(ObservationError::ByteOverflow {
            device: DeviceId::Uart,
            kind: ByteStreamKind::SerialTx,
            source: ByteOverflow { limit: 1 },
        });
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx),
            expected
        );
        assembled
            .machine
            .bus_mut()
            .device_mut(assembled.device_index[&DeviceId::Uart])
            .unwrap()
            .reset();
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx),
            expected
        );
        assembled.observation.start_epoch();
        assembled
            .machine
            .bus_mut()
            .port_write(0x3F8, 1, 42)
            .unwrap();
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
                .unwrap(),
            [42]
        );
    }
}
