// SPDX-License-Identifier: AGPL-3.0-or-later
//! The assembler: a validated [`MachineConfig`] becomes a running [`Machine`]
//! (§3.1).
//!
//! [`assemble`] is the single data-driven realiser. It reads the pure-data
//! description and builds the concrete parts the machine core is deliberately
//! blind to: an [`X86JitCpu`] with its memory mosaic laid out, a [`Bus`] with a
//! [`bus::DecodeMap`], the one shared interrupt controller, and the IRQ / DMA
//! wiring tables. The core never names any of these concrete types;
//! naming them is exactly this layer's job, which is why it sits above the
//! `machine` crate.
//!
//! Adjacent MMIO windows share one CPU trap region so cross-device accesses
//! reach the bus instead of failing the CPU's single-region containment check.
//! The bus keeps each device's decode window and splits accesses at their
//! boundaries; gaps between windows stay outside the merged trap regions. See
//! `backlog/docs/design/machine-config.md`, §MemoryConfig.
//!
//! ## Devices are named, not numbered
//!
//! The old hand-written assembly addressed every device by a literal bus index
//! that had to stay in lock-step with the order devices were pushed onto a `Vec`.
//! Here each [`DeviceConfig`] carries a stable [`DeviceId`], and the assembler
//! returns a [`DeviceId`] → bus-index map ([`AssembledMachine::device_index`]) so
//! a later drain/inject path addresses a device by role, not by a fragile count.
//!
//! ## The shared interrupt controller
//!
//! A PC/AT's two 8259 PICs are one [`chipset::CascadedPic`]: it is the machine's
//! single [`device::InterruptController`] and it exposes its master and slave
//! chips as two separate bus [`device::Device`] faces. The config models the pair
//! as two [`DeviceConfig`] entries (a master with cascade set, a slave without);
//! the assembler builds one live `CascadedPic` and hands its master face, its
//! slave face, and its controller face out of that single chip, so a guest EOI
//! through either bus face reaches the state the machine's INTA set.
//!
//! ## CMOS initialization
//!
//! A stock BIOS reads its floppy types, equipment byte, and memory sizes from
//! CMOS and checksums them against the actual machine; a mismatch drops POST into
//! an interactive SETUP prompt a headless run cannot answer. So the seed is a
//! *product* of assembly ([`derive_cmos_layout`]), computed from the memory map
//! and the attached drives. The configured RTC receives 64 or 128 bytes.
//! An explicit NVRAM image bypasses this derivation;
//! the fixed RTC start supplies the live calendar in either case.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::{cell::Cell, rc::Rc};

use bus::{Bus, DecodeMap, MmioRange, PortRange};
use cpu::x86jit_backend::X86JitCpu;
use device::{Device, InterruptController};
use devices::{
    A20Line, Absent80287, Ata, AtaDrive, AtaGeometry, CmosRtc, DebugConsole, Dma8237, Drive, FwCfg,
    MdaText, PostPort, SysControl, SysCtrlPortA, Uart16550, Upd765, VgaText, I8042,
};
use machine::{DmaChannelWiring, DmaWiring, IrqLine, Machine};
use sha2::{Digest, Sha256};

use crate::device::{DeviceId, DeviceKind};
use crate::drive::{DriveKind, FloppyGeometry, WriteMode};
use crate::error::ConfigError;
use crate::image::{ImageKind, ImageSource};
use crate::machine::MachineConfig;
use crate::mount::{DiskBase, InlineMount, MachineMount, Mount, MountError};
use crate::observation::{Observation, ObservationPolicy};

/// Top of conventional (base) memory: 640 KiB. RAM below this is what the BIOS
/// reports as base memory; RAM in `[0xA0000, 0x100000)` is the video/adapter/ROM
/// window, not counted.
const CONVENTIONAL_TOP: u64 = 0xA_0000;

/// Base of extended memory: 1 MiB. RAM at or above this is extended memory.
pub const EXTENDED_BASE: u64 = 0x10_0000;

/// A machine built from a [`MachineConfig`], with the name→index map that lets a
/// caller reach a device by its stable [`DeviceId`] (§3.1).
///
/// The `machine` is ready to run; `device_index` resolves each configured
/// [`DeviceId`] to the bus slot the assembler placed it in, so a drain or inject
/// path never hardcodes an integer index.
#[must_use = "assembled journal sessions require explicit finalization"]
pub struct AssembledMachine {
    /// The running machine, fully wired.
    pub machine: Machine<X86JitCpu>,
    /// Effective board A20 state and transition counter, applied to CPU addressing.
    /// See `backlog/docs/design/machine-config.md` §A20 line and System Control Port A.
    pub a20_line: A20Line,
    /// Stable device role → its index in the bus device set.
    pub device_index: BTreeMap<DeviceId, usize>,
    /// Unique host mouse role, resolved from validated config without device access.
    pub mouse_target: Option<DeviceId>,
    /// Validated observation routes and their bounded, run-scoped collectors.
    pub observation: Observation,
    /// Drive and CMOS journal status, boundary persistence and finalization.
    pub persistence: Vec<crate::PersistenceHandle>,
    /// Host-only catalogue labels; media selection lives in the FDC snapshot.
    pub floppy_names: BTreeMap<(DeviceId, u8), Vec<String>>,
    /// Unknown BIOS hashes accepted through a run-only escape option.
    pub unlisted_firmware: Vec<crate::UnlistedFirmware>,
    /// Option ROM images in ascending physical address, the BIOS scan order.
    /// Includes their resolved bytes and service declarations for diagnostics.
    pub option_roms: Vec<crate::LoadImage>,
    /// The mount the machine was assembled with, kept for the machine's life:
    /// a catalogue medium's journal opens through it at each insert.
    pub mount: MachineMount,
}

/// A way assembling a validated [`MachineConfig`] can fail (§3.1).
///
/// Configuration inconsistencies and unreadable backing files surface as
/// [`AssemblyError::Config`] before hardware is built. The remaining failures
/// come from the CPU core rejecting a memory operation or the bus rejecting
/// resolved access-coupling indices.
#[derive(Debug, thiserror::Error)]
pub enum AssemblyError {
    /// CMOS journal could not be locked, validated or replayed.
    #[error("NVRAM journal mount: {source}")]
    NvramJournal {
        #[source]
        source: journal::OpenError,
    },
    /// The configuration is internally inconsistent (validation ran first).
    #[error("invalid configuration: {0}")]
    Config(#[from] ConfigError),

    /// A drive could not lock, validate, repair or replay its journal.
    #[error("drive {drive_index} journal mount: {source}")]
    Journal {
        drive_index: usize,
        #[source]
        source: journal::OpenError,
    },

    /// Resolved access-coupling indices were rejected by the bus.
    #[error("bus access coupling: {0}")]
    AccessCoupling(#[from] bus::AccessCouplingError),

    /// The CPU core rejected a memory-map operation (a region that does not fit,
    /// a `Trap` window overlapping RAM the config did not catch).
    #[error("cpu memory map: {0}")]
    Cpu(#[from] cpu::CpuError),
}

/// Turn an inline [`MachineConfig`] into a running [`Machine`] and its device-index map
/// (§3.1). File-backed callers use `machine_file::assemble`; other mounts use
/// [`assemble_mounted`].
///
/// The steps are: validate the config; build the CPU and lay out its memory
/// mosaic (RAM regions, MMIO windows, then loaded images); instantiate each
/// device in config order, recording its [`DeviceId`] → index; build the decode
/// map, the interrupt controller, and the IRQ / DMA wiring from the device data;
/// derive and seed the CMOS from the memory map and drives; and assemble the
/// machine.
///
/// The **idle limit** — how long a halted guest is kept alive waiting for an
/// interrupt before the machine declares the source quiescent — is *not* set here.
/// It is a "how we run" concern that belongs to [`RunPolicy`](crate::RunPolicy),
/// so the assembled machine is built without one and the run driver
/// ([`run`](crate::run)) stamps it on before the loop runs.
pub fn assemble(config: &MachineConfig) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_observation(config, &ObservationPolicy::default())
}

/// Assemble hardware and resolve explicitly requested observation routes.
///
/// Missing or mismatched capabilities are [`ConfigError`]s before execution.
/// Capture choices are separate from guest hardware configuration; existing
/// [`assemble`] callers retain their current capture paths.
pub fn assemble_with_observation(
    config: &MachineConfig,
    observation_policy: &ObservationPolicy,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_observation_and_options(
        config,
        observation_policy,
        crate::AssemblyOptions::default(),
    )
}

/// Assemble with run-only firmware permissions and default observation.
pub fn assemble_with_options(
    config: &MachineConfig,
    options: crate::AssemblyOptions,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_observation_and_options(config, &ObservationPolicy::default(), options)
}

/// Host mix level of the driven PC speaker (IBM Personal Computer AT Technical
/// Reference, System Board, Speaker): the drive is 0 or this level, a peak of
/// -12 dBFS. The audible swing of that one-sided square is half the level,
/// ±4096, 12 dB under the ±16384 swing of a full-scale sound-card voice, as a
/// case speaker sounds next to a card's line output. See
/// `backlog/docs/design/audio.md` §Host mix levels.
const SPEAKER_LEVEL: i16 = 8192;

/// Assemble with explicit capture routes and run-only firmware permissions.
pub fn assemble_with_observation_and_options(
    config: &MachineConfig,
    observation_policy: &ObservationPolicy,
    options: crate::AssemblyOptions,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_cpu(config, observation_policy, options, build_cpu)
}

/// Assemble with everything the config names by path taken from `mount`:
/// image bytes, disk bases and journals (see [`Mount`]). The machine keeps
/// the mount for its life, so a catalogue's journal opens through it at each
/// insert. A medium appended by path later is a host file all the same
/// (`machine_file::FileMachineExt::append_floppy_verified`). See
/// `backlog/docs/design/core-contract.md` §12.5 and §14.4.
pub fn assemble_mounted(
    config: &MachineConfig,
    observation_policy: &ObservationPolicy,
    options: crate::AssemblyOptions,
    mount: Rc<dyn Mount>,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_preverified(
        config,
        observation_policy,
        options,
        build_cpu,
        config.board.firmware_catalogue(),
        mount.clone(),
        MachineMount::new(mount),
    )
}

fn assemble_with_cpu(
    config: &MachineConfig,
    observation_policy: &ObservationPolicy,
    options: crate::AssemblyOptions,
    build: impl FnOnce(&MachineConfig) -> Result<X86JitCpu, AssemblyError>,
) -> Result<AssembledMachine, AssemblyError> {
    assemble_with_cpu_and_catalogue(
        config,
        observation_policy,
        options,
        build,
        config.board.firmware_catalogue(),
    )
}

fn assemble_with_cpu_and_catalogue(
    config: &MachineConfig,
    observation_policy: &ObservationPolicy,
    options: crate::AssemblyOptions,
    build: impl FnOnce(&MachineConfig) -> Result<X86JitCpu, AssemblyError>,
    catalogue: &[crate::FirmwareEntry],
) -> Result<AssembledMachine, AssemblyError> {
    assemble_preverified(
        config,
        observation_policy,
        options,
        build,
        catalogue,
        Rc::new(InlineMount),
        MachineMount::default(),
    )
}

pub fn assemble_preverified(
    config: &MachineConfig,
    observation_policy: &ObservationPolicy,
    options: crate::AssemblyOptions,
    build: impl FnOnce(&MachineConfig) -> Result<X86JitCpu, AssemblyError>,
    catalogue: &[crate::FirmwareEntry],
    mount: Rc<dyn Mount>,
    machine_mount: MachineMount,
) -> Result<AssembledMachine, AssemblyError> {
    // A validated config does not fail assembly for configuration reasons; run
    // the checks first so any inconsistency surfaces as a precise ConfigError.
    config.validate_mounted(|media| mount.check_outputs(media))?;
    validate_observation_policy(config, observation_policy)?;

    // Validate the bytes that will actually be mounted, without a second file
    // read that could observe a different length or contents.
    let floppy_names = config
        .drives
        .iter()
        .filter(|d| matches!(d.kind, DriveKind::Floppy { .. }))
        .map(|d| {
            let names = d.media.as_ref().map_or_else(
                || {
                    vec![match &d.image {
                        ImageSource::File(p) => p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        _ => "disk 1".into(),
                    }]
                },
                |list| list.entries.iter().map(|m| m.name.clone()).collect(),
            );
            ((d.controller, d.unit), names)
        })
        .collect();
    let nvram_bases = load_nvram_bases(config, mount.as_ref())?;
    let (resolved, base_metadata) =
        resolve_sources_preverified(config, options, catalogue, mount.as_ref())?;
    let config = resolved.as_ref();
    config.validate_resolved()?;

    let mut unlisted_firmware = Vec::new();
    if !catalogue.is_empty() {
        let image = crate::firmware::system_bios(config)?;
        let bytes = load_image_bytes(&image.source)?;
        if let Some(unlisted) =
            crate::check_system_bios(config, config.board, catalogue, image.base, &bytes, options)?
        {
            unlisted_firmware.push(unlisted);
        }
    }

    unlisted_firmware.extend(crate::check_option_rom_catalogue(config, options)?);
    let mut option_roms = Vec::new();
    if !config.board.firmware_catalogue().is_empty() {
        let bios = crate::firmware::system_bios(config)?;
        for (index, image) in config.images.iter().enumerate() {
            if image.kind == ImageKind::Rom && !std::ptr::eq(image, bios) {
                let mut image = image.clone();
                image.provides = crate::firmware::image_services(config, index)?
                    .unwrap_or_default()
                    .to_vec();
                option_roms.push(image);
            }
        }
        option_roms.sort_by_key(|image| image.base);
    }

    let mut cpu = build(config)?;
    validate_step_cpu(&mut cpu)?;
    let cmos = if config.devices.iter().any(|device| {
        matches!(
            device.kind,
            DeviceKind::Rtc(crate::CmosCfg {
                nvram: crate::NvramSource::Derived,
                ..
            })
        )
    }) {
        derive_cmos_layout_with_catalogue(config, catalogue)?
    } else {
        [0; 128]
    };

    // Instantiate devices in config order, recording each role's bus index. The
    // one shared PIC is created lazily on the first PIC entry and reused for the
    // second face and the controller seam.
    let mut devices: Vec<Box<dyn Device>> = Vec::with_capacity(config.devices.len());
    let mut device_index: BTreeMap<DeviceId, usize> = BTreeMap::new();
    let mut pic: Option<chipset::CascadedPic> = None;
    let reset_signal = Rc::new(Cell::new(false));
    let a20_line = A20Line::default();
    let pit = devices::pit8253::SharedPit8253::default();
    let mut persistence = Vec::new();
    let mut native_audio = Vec::new();
    let mut ems_frame = None;
    let mut trace_cmos = None;

    for dc in &config.devices {
        let boxed: Box<dyn Device> = match &dc.kind {
            DeviceKind::Uart(c) => {
                let mut uart = Uart16550::new(c.base);
                if c.mouse {
                    uart = uart.with_serial_device(Box::new(devices::SerialMouse::new()));
                }
                Box::new(uart)
            }
            DeviceKind::Pit(_) => Box::new(pit.clone()),
            DeviceKind::Pic(c) => {
                // One live master/slave pair backs both faces and the controller.
                let shared = pic.get_or_insert_with(chipset::CascadedPic::new);
                if c.cascade {
                    Box::new(shared.master_device())
                } else {
                    Box::new(shared.slave_device())
                }
            }
            DeviceKind::Keyboard(c) => {
                let mut keyboard = I8042::new()
                    .with_primary_display(config.primary_display()?.monochrome())
                    .with_board_lines(Rc::clone(&reset_signal), a20_line.clone());
                if c.mouse {
                    keyboard = keyboard.with_aux_device(Box::new(devices::Ps2Mouse::new()));
                }
                Box::new(keyboard)
            }
            DeviceKind::Rtc(c) => {
                let mut rtc = CmosRtc::new().with_ram(c.ram);
                if let Some((second_ticks, uip_ticks)) = c.timing {
                    rtc = rtc.with_timing(second_ticks, uip_ticks);
                }
                rtc = match &c.nvram {
                    crate::NvramSource::Derived => match c.ram {
                        crate::CmosRam::Bytes64 => {
                            let mut seed = [0; 64];
                            seed.copy_from_slice(&cmos[..64]);
                            rtc.with_nvram(seed)
                        }
                        crate::CmosRam::Bytes128 => rtc.with_extended_nvram(cmos),
                    },
                    crate::NvramSource::Blank => rtc,
                    crate::NvramSource::Image(bytes) => rtc.with_nvram(*bytes),
                    crate::NvramSource::Journal {
                        journal,
                        keep_generations,
                        ..
                    } => {
                        let (base, origin) = &nvram_bases[&devices.len()];
                        let handle = crate::PersistenceHandle::mount_nvram(
                            devices.len(),
                            mount.as_ref(),
                            journal,
                            base,
                            *keep_generations,
                            origin.as_ref(),
                            &mut rtc,
                        )?;
                        persistence.push(handle);
                        rtc
                    }
                };
                let start = c.start;
                rtc.set_time(
                    start.second,
                    start.minute,
                    start.hour,
                    start.weekday(),
                    start.day,
                    start.month,
                    (start.year % 100) as u8,
                );
                if observation_policy.speaker_trace {
                    trace_cmos = Some(rtc.nvram_store());
                }
                Box::new(rtc)
            }
            DeviceKind::LotechEms(c) => {
                let board = devices::LotechEms::new(c.port, c.frame, c.size_kib)
                    .map_err(|_| ConfigError::InvalidLotechEmsResources { device: dc.id })?;
                ems_frame = Some((dc.id, board.frame_memory(), u64::from(c.frame) << 4));
                Box::new(board)
            }
            DeviceKind::Vga(_) => Box::new(VgaText::new()),
            DeviceKind::VgaRegisters(_) => Box::new(devices::Vga::new()),
            DeviceKind::Mda(_) => Box::new(MdaText::new()),
            DeviceKind::SysControl(_) => Box::new(SysControl::new().with_pit(pit.clone())),
            DeviceKind::Post(c) => Box::new(PostPort::with_port(c.port)),
            DeviceKind::SysCtrlPortA => Box::new(SysCtrlPortA::with_board_lines(
                Rc::clone(&reset_signal),
                a20_line.clone(),
            )),
            DeviceKind::DebugConsole(c) => Box::new(DebugConsole::with_port(c.port)),
            DeviceKind::FwCfg(c) => Box::new(
                FwCfg::with_files(crate::fw_cfg::files(config, dc.id, c)?).map_err(|source| {
                    ConfigError::InvalidFwCfgFiles {
                        device: dc.id,
                        source,
                    }
                })?,
            ),
            DeviceKind::Fdc(_) => Box::new(mount_removable_fdc(
                config,
                dc.id,
                &mut persistence,
                &base_metadata,
                &mount,
            )?),
            DeviceKind::Ide(cfg) => Box::new(
                Ata::with_drives(mount_ide_drives(
                    config,
                    dc.id,
                    &mut persistence,
                    &base_metadata,
                    devices.len(),
                    mount.as_ref(),
                )?)
                .map(|ata| ata.with_model(cfg.model))
                .and_then(|ata| ata.with_resources(cfg.base, cfg.control, cfg.irq()))
                .map_err(|_| ConfigError::InvalidIdeResources { device: dc.id })?,
            ),
            DeviceKind::Sb16(cfg) => {
                let (dsp, source) = crate::sb16_audio::build(
                    cfg.base,
                    dc.dma.map_or(1, |dma| dma.channel),
                    cfg.dma16,
                    dc.irq.unwrap_or(5),
                );
                native_audio.push(source);
                Box::new(dsp)
            }
            DeviceKind::Dma(_) => Box::new(Dma8237::new()),
            DeviceKind::DmaWord(_) => Box::new(Dma8237::new_at_word()),
            DeviceKind::AbsentCoprocessor => Box::new(Absent80287::new()),
        };
        let index = devices.len();
        devices.push(boxed);
        device_index.insert(dc.id, index);
    }

    let mut observation = Observation::bind(observation_policy, &device_index, &mut devices)?;
    if device_index.contains_key(&DeviceId::SysControl) && device_index.contains_key(&DeviceId::Pit)
    {
        let source = audio::EdgeSource::new([0; 2]);
        let writer = source.clone();
        pit.observe_speaker(move |at, high| {
            writer.record_level(at, [if high { SPEAKER_LEVEL } else { 0 }; 2]);
        });
        observation.add_audio_source(device::VClock::ZERO, Box::new(source));
    }

    for source in native_audio {
        observation.add_audio_source(device::VClock::ZERO, source);
    }
    let speaker_trace = observation_policy.speaker_trace.then(|| {
        crate::speaker_trace::SpeakerTrace::new(
            pit.clone(),
            has_a20_source(config).then(|| Rc::clone(&reset_signal)),
            trace_cmos,
        )
    });
    observation.speaker_trace = speaker_trace.clone();

    // The machine needs exactly one controller. If the config has no PIC (an
    // interrupt-free machine), a null controller keeps the loop total.
    let controller: Box<dyn InterruptController> = match pic {
        Some(p) => Box::new(p),
        None => Box::new(chipset::NoInterruptController),
    };

    let map = build_decode_map(config, &device_index);
    let irq_lines = build_irq_lines(config, &device_index);
    let (dma_wirings, dma_channel_wirings) = build_dma_wirings(config, &device_index);

    // The machine is built with no idle limit; the run policy stamps it on before
    // the loop runs (idle_limit is a "how we run" concern, not machine identity).
    let (rate_numerator, rate_denominator) = config.clock.cpu_rate()?;
    let mut bus = Bus::new(devices, map);
    if let (Some(&sys), Some(&timer)) = (
        device_index.get(&DeviceId::SysControl),
        device_index.get(&DeviceId::Pit),
    ) {
        bus.add_access_coupling(sys, timer)?;
    }
    for device in &config.devices {
        for dependent in &device.access_dependents {
            bus.add_access_coupling(device_index[&device.id], device_index[dependent])?;
        }
    }
    bus.set_trace_policy(observation_policy.trace);
    let mut machine = Machine::new(cpu, bus, controller, irq_lines, None)
        .with_cpu_rate(rate_numerator, rate_denominator)
        .with_dma_wirings(dma_wirings)
        .with_dma_channel_wirings(dma_channel_wirings);
    if has_a20_source(config) {
        let line = a20_line.clone();
        machine = machine
            .with_reset_signal(reset_signal)
            .with_a20_line(move || (line.change_count(), line.enabled()))
            // Reset fetches reach the ROM before the first CS load, even with
            // the board gate off (Intel 80286 PRM, Appendix C, Address after
            // Reset; Intel 80386 PRM, §10.1 Initialization). Bare low entries
            // never open this window. Read the hidden base only for reset CS.
            .with_reset_fetch_probe(|cpu: &cpu::X86JitCpu| {
                cpu.code_segment() == 0xF000
                    && matches!(cpu.diagnostic_state().cs().base, 0xFF_0000 | 0xFFFF_0000)
            });
    }
    let mut ems_map = None;
    if let Some((device, board, frame)) = ems_frame {
        // The CPU cannot fetch from a trapped region; let it hold mapped pages.
        let run = trap_runs(config)
            .into_iter()
            .find(|&(base, end)| base <= frame && frame + 0x10000 <= end)
            .ok_or(ConfigError::InvalidLotechEmsResources { device })?;
        ems_map = Some(crate::ems_frame::EmsFrameMap::new(board, frame, run));
    }
    match (ems_map, speaker_trace) {
        (Some(mut map), None) => {
            machine = machine.with_memory_map(move |cpu, event| map.sync(cpu, event));
        }
        // The trace shares the one memory-map hook, which runs after every
        // guest port write; it only reads state after the EMS map is applied.
        (mut map, Some(trace)) => {
            machine = machine.with_memory_map(move |cpu, event| {
                if let Some(map) = &mut map {
                    map.sync(cpu, event)?;
                }
                if event == ::machine::MemoryMapSync::Apply {
                    trace.after_port_write(cpu);
                }
                Ok(())
            });
        }
        (None, None) => {}
    }

    let mouse_target = config.devices.iter().find_map(|device| match &device.kind {
        DeviceKind::Uart(cfg) if cfg.mouse => Some(device.id),
        DeviceKind::Keyboard(cfg) if cfg.mouse => Some(device.id),
        _ => None,
    });
    Ok(AssembledMachine {
        mouse_target,
        machine,
        a20_line,
        device_index,
        observation,
        persistence,
        floppy_names,
        unlisted_firmware,
        option_roms,
        mount: machine_mount,
    })
}

// Reject policy errors before any mount can create files or recover a tip.
pub fn validate_observation_policy(
    config: &MachineConfig,
    policy: &ObservationPolicy,
) -> Result<(), ConfigError> {
    use crate::OutputKind;
    use device::ByteStreamKind;

    for (index, request) in policy.routes.iter().enumerate() {
        let device = request.device;
        let output = request.output;
        if policy.routes[..index]
            .iter()
            .any(|r| r.device == device && r.output == output)
        {
            return Err(ConfigError::DuplicateObservationRoute { device, output });
        }
        let supported = config
            .devices
            .iter()
            .find(|dc| dc.id == device)
            .is_some_and(|dc| {
                matches!(
                    (&dc.kind, output),
                    (
                        DeviceKind::Vga(_) | DeviceKind::VgaRegisters(_) | DeviceKind::Mda(_),
                        OutputKind::TextScanout
                    ) | (
                        DeviceKind::Uart(_),
                        OutputKind::Bytes(ByteStreamKind::SerialTx)
                    ) | (
                        DeviceKind::Post(_),
                        OutputKind::Bytes(ByteStreamKind::PostCheckpoints)
                    ) | (
                        DeviceKind::DebugConsole(_),
                        OutputKind::Bytes(ByteStreamKind::DebugConsole)
                    )
                )
            });
        if !supported {
            return Err(ConfigError::UnsupportedObservationOutput { device, output });
        }
    }
    Ok(())
}

/// A zero-step probe rejects unsupported backends without executing guest code.
pub fn validate_step_cpu(cpu: &mut impl cpu::Cpu) -> Result<(), ConfigError> {
    cpu.run_steps(0, cpu::StepRunOptions::default())
        .map(|_| ())
        .map_err(|error| ConfigError::StepBoundedCpuRequired(error.to_string()))
}

/// Build the CPU with the whole memory mosaic laid out and the reset entry seeded
/// (§4).
///
/// The bare real-mode constructor maps nothing; the assembler lays out every RAM
/// region, MMIO window, and loaded image from the config. On a firmware board,
/// empty adapter addresses below the system BIOS remain read-only zeroes.
/// Other holes in the 24-bit physical space read FFh (ASSUMED floating value)
/// and drop guest stores, using the backend's immutable ROM contract.
/// IBM Personal Computer AT Technical Reference, September 1985, Section 1
/// "System Board", System Memory Map (p. 1-8) assigns memory ranges but does
/// not establish the floating value. Outside reserved board aliases, configured
/// regions take priority. See
/// `backlog/docs/design/machine-config.md` §MemoryConfig for this model choice.
pub fn build_cpu(config: &MachineConfig) -> Result<X86JitCpu, AssemblyError> {
    build_cpu_with_mapping_trace(config, |_, _, _| {})
}

/// The trapped CPU regions covering the configured MMIO windows.
///
/// The CPU requires each access to fit one region, while device ownership
/// remains a bus concern. Only touching windows can share a trap region.
pub fn trap_runs(config: &MachineConfig) -> Vec<(u64, u64)> {
    let mut mmio: Vec<_> = config.memory.mmio.iter().collect();
    mmio.sort_unstable_by_key(|window| window.base);
    mmio.chunk_by(|left, right| left.end() == right.base)
        .map(|run| (run[0].base, run[run.len() - 1].end()))
        .collect()
}

pub fn has_a20_source(config: &MachineConfig) -> bool {
    config.devices.iter().any(|device| {
        matches!(
            device.kind,
            DeviceKind::Keyboard(_) | DeviceKind::SysCtrlPortA
        )
    })
}

pub fn build_cpu_with_mapping_trace(
    config: &MachineConfig,
    mut record: impl FnMut(u64, u64, &'static str),
) -> Result<X86JitCpu, AssemblyError> {
    let firmware_board = !config.board.firmware_catalogue().is_empty();
    let board_memory = config.board == crate::BoardModel::Ibm5170;
    let physical_end = match config.board {
        crate::BoardModel::Ibm5170 => crate::memory::AT_PHYSICAL_END,
        crate::BoardModel::Generic386Isa => crate::memory::I386_PHYSICAL_END,
        crate::BoardModel::Bare => config.memory.address_space_bytes,
    };
    let mut cpu = match (config.board, config.cpu.chip) {
        (crate::BoardModel::Ibm5170, crate::CpuChip::I80286) => X86JitCpu::new_i286_bare_with_fpu(
            physical_end,
            config.cpu.reset.cs(),
            config.cpu.reset.ip(),
            config.cpu.segments.into(),
            config.cpu.fpu == crate::FpuChip::I80287,
        )?,
        (_, crate::CpuChip::I80386) => X86JitCpu::new_i386_bare(
            config.cpu.reset.cs(),
            config.cpu.reset.ip(),
            config.cpu.segments.into(),
        )?,
        (crate::BoardModel::Generic386Isa, crate::CpuChip::I80286) => {
            return Err(ConfigError::BoardConfiguration {
                board: config.board.id(),
                reason: "requires an 80386DX CPU",
            }
            .into());
        }
        (crate::BoardModel::Bare, crate::CpuChip::I80286) if has_a20_source(config) => {
            X86JitCpu::new_a20_bare(
                physical_end,
                config.cpu.reset.cs(),
                config.cpu.reset.ip(),
                config.cpu.segments.into(),
            )?
        }
        (crate::BoardModel::Bare, crate::CpuChip::I80286) => X86JitCpu::new_real16_bare(
            physical_end,
            config.cpu.reset.cs(),
            config.cpu.reset.ip(),
            config.cpu.segments.into(),
        )?,
    };
    let mut mapped = Vec::new();
    let mut record_mapping = |base, end, kind| {
        mapped.push((base, end));
        record(base, end, kind);
    };

    // RAM and MMIO first so a RAM-loaded image writes into an already-mapped
    // region; a ROM image maps and populates its own region.
    for region in &config.memory.ram {
        cpu.map_ram(region.base, region.size)?;
        record_mapping(region.base, region.end(), "ram");
    }
    for (base, end) in trap_runs(config) {
        cpu.map_mmio(base, end - base)?;
        record_mapping(base, end, "mmio");
    }
    if firmware_board && config.board != crate::BoardModel::Generic386Isa {
        // Physical adapter decoding belongs to the board, independently of
        // whether this firmware searches for option ROMs.
        let end = crate::firmware::system_bios(config)?.base;
        let mut cursor = 0xC0000;
        let mut occupied: Vec<_> = config
            .images
            .iter()
            .filter(|image| image.kind == ImageKind::Rom)
            .map(|image| {
                (
                    image.base,
                    image.base + image.source.known_len().unwrap_or(0),
                )
            })
            .chain(
                config
                    .memory
                    .mmio
                    .iter()
                    .map(|window| (window.base, window.end())),
            )
            .filter(|(base, stop)| *base < end && *stop > cursor)
            .collect();
        occupied.sort_unstable();
        for (base, stop) in occupied {
            if base > cursor {
                cpu.map_rom_filled(cursor, base - cursor, 0)?;
                record_mapping(cursor, base, "rom");
            }
            cursor = cursor.max(stop);
        }
        if cursor < end {
            // Preserve historical zero-filled reads while keeping
            // adapter space isolated from guest RAM and RAM-loaded code.
            cpu.map_rom_filled(cursor, end - cursor, 0)?;
            record_mapping(cursor, end, "rom");
        }
    }
    for image in &config.images {
        let bytes = load_image_bytes(&image.source)?;
        match image.kind {
            ImageKind::Rom if config.board == crate::BoardModel::Generic386Isa => {
                cpu.write_mem(image.base, &bytes)?;
                if std::ptr::eq(image, crate::firmware::system_bios(config)?) {
                    cpu.map_rom(crate::memory::I386_HIGH_BIOS_BASE, &bytes)?;
                    record_mapping(
                        crate::memory::I386_HIGH_BIOS_BASE,
                        crate::memory::I386_PHYSICAL_END,
                        "rom",
                    );
                }
            }
            ImageKind::Rom => {
                cpu.map_rom(image.base, &bytes)?;
                record_mapping(image.base, image.base + bytes.len() as u64, "rom");
            }
            ImageKind::RamLoad => cpu.write_mem(image.base, &bytes)?,
        }
    }
    if board_memory {
        // Reserve the board decode before filling holes. Its source may itself
        // contain holes, which must be filled before the alias is copied.
        if config.board == crate::BoardModel::Ibm5170 {
            mapped.push((crate::memory::AT_HIGH_ALIAS_BASE, physical_end));
        }
        mapped.sort_unstable();
        let mut cursor = 0;
        let mut gaps = Vec::new();
        for &(base, end) in &mapped {
            let base = base.min(physical_end);
            if cursor < base {
                cpu.map_rom_filled(cursor, base - cursor, 0xFF)?;
                record(cursor, base, "empty-bus");
                gaps.push((cursor, base));
            }
            cursor = cursor.max(end).min(physical_end);
        }
        if cursor < physical_end {
            cpu.map_rom_filled(cursor, physical_end - cursor, 0xFF)?;
            record(cursor, physical_end, "empty-bus");
            gaps.push((cursor, physical_end));
        }
        if config.board == crate::BoardModel::Ibm5170 {
            mapped.extend(gaps);
            map_at_high_alias(&mut cpu, &mapped)?;
            record(crate::memory::AT_HIGH_ALIAS_BASE, physical_end, "rom");
        }
    }
    if (board_memory || config.board == crate::BoardModel::Generic386Isa)
        && matches!(config.cpu.reset, crate::ResetSpec::ResetVector { .. })
    {
        cpu::Cpu::reset(&mut cpu)?;
    }
    Ok(cpu)
}

/// Map the IBM AT board's complete high ROM decode, independently of CPU reset.
/// IBM Personal Computer AT Technical Reference, September 1985, Section 1
/// "System Board", System Memory Map (p. 1-8): FE0000h–FFFFFFh duplicates
/// E0000h–FFFFFh. Copy the resolved low map, including gap fill, in bounded
/// chunks that never cross source-region boundaries. This is the single owner
/// of the high alias; the I286 reset-origin fetch at FFFFF0h reads its copied
/// bytes before the first far transfer replaces the hidden CS base.
/// See machine-config.md §MemoryConfig.
fn map_at_high_alias(cpu: &mut X86JitCpu, mapped: &[(u64, u64)]) -> Result<(), AssemblyError> {
    let high = crate::memory::AT_HIGH_ALIAS_BASE;
    cpu.map_rom_filled(high, 0x20000, 0)?;
    let mut bytes = [0; 4096];
    for &(base, end) in mapped {
        let mut cursor = base.max(0xE0000);
        let end = end.min(0x100000);
        while cursor < end {
            let size = (end - cursor).min(bytes.len() as u64) as usize;
            cpu.read_mem(cursor, &mut bytes[..size])?;
            cpu.write_mem(cursor + 0xF00000, &bytes[..size])?;
            cursor += size as u64;
        }
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn cpu_mapping_regions(
    config: &MachineConfig,
    options: crate::AssemblyOptions,
) -> Result<Vec<(u64, u64, &'static str)>, AssemblyError> {
    let (resolved, _) = resolve_file_sources(config, options)?;
    let config = resolved.as_ref();
    config.validate_resolved()?;
    let mut regions = Vec::new();
    build_cpu_with_mapping_trace(config, |base, end, kind| {
        regions.push((base, end, kind));
    })?;
    Ok(regions)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn mapped_cpu(
    config: &MachineConfig,
    options: crate::AssemblyOptions,
) -> Result<X86JitCpu, AssemblyError> {
    let (resolved, _) = resolve_file_sources(config, options)?;
    resolved.validate_resolved()?;
    build_cpu(&resolved)
}

/// Materialise an image's bytes: inline bytes verbatim, a blank run of zeroes, or
/// a file read from disk at load time.
pub fn load_image_bytes(source: &ImageSource) -> Result<Vec<u8>, ConfigError> {
    load_mounted_bytes(&InlineMount, source)
}

/// Materialise an image's bytes, asking `mount` for what the source names by
/// path.
pub fn load_mounted_bytes(mount: &dyn Mount, source: &ImageSource) -> Result<Vec<u8>, ConfigError> {
    match source {
        ImageSource::Inline(bytes) => Ok(bytes.clone()),
        ImageSource::RomContainer { bytes, offset } => bytes
            .get(*offset..)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| ConfigError::FirmwareParts {
                message: "ROM container offset exceeds image length".into(),
            }),
        ImageSource::Blank { size } => Ok(vec![0u8; *size as usize]),
        ImageSource::Interleaved(parts) => {
            let loaded = read_parts(mount, parts)?;
            crate::interleave_parts(&loaded)
        }
        ImageSource::File(path) => read_image(mount, path).map(|image| image.into_parts().0),
    }
}

/// The bytes `mount` holds for `path`; a host failure is the path's read error.
fn read_image(
    mount: &dyn Mount,
    path: &std::path::Path,
) -> Result<crate::mount::ImageBytes, ConfigError> {
    mount.image(path).map_err(|error| match error {
        MountError::Config(error) => error,
        MountError::Host(error) => ConfigError::ImageRead {
            path: path.into(),
            message: error.to_string(),
        },
    })
}

fn read_parts(
    mount: &dyn Mount,
    parts: &[crate::ImagePart],
) -> Result<Vec<(crate::Lane, u64, Vec<u8>)>, ConfigError> {
    parts
        .iter()
        .map(|part| {
            Ok((
                part.lane,
                part.offset,
                load_mounted_bytes(mount, &ImageSource::File(part.path.clone()))?,
            ))
        })
        .collect()
}

/// The 64-byte CMOS image of each NVRAM journal, by device index, with the
/// identity of the host file it came from when it came from one.
pub type NvramBases = BTreeMap<usize, ([u8; 64], Option<Rc<dyn std::any::Any>>)>;

pub fn load_nvram_bases(
    config: &MachineConfig,
    mount: &dyn Mount,
) -> Result<NvramBases, ConfigError> {
    let mut bases = BTreeMap::new();
    for (index, device) in config.devices.iter().enumerate() {
        let DeviceKind::Rtc(crate::CmosCfg {
            nvram:
                crate::NvramSource::Journal {
                    image,
                    expected_sha256,
                    ..
                },
            ..
        }) = &device.kind
        else {
            continue;
        };
        let (bytes, origin) = mount
            .image(image)
            .map_err(|error| match error {
                MountError::Config(error) => error,
                MountError::Host(error) => ConfigError::Nvram {
                    message: format!("image {}: {error}", image.display()),
                },
            })?
            .into_parts();
        let actual: [u8; 32] = Sha256::digest(&bytes).into();
        if &actual != expected_sha256 {
            return Err(ConfigError::Nvram {
                message: format!(
                    "image sha256 mismatch: expected {:x}, actual {:x}",
                    sha2::digest::Output::<Sha256>::from(*expected_sha256),
                    sha2::digest::Output::<Sha256>::from(actual)
                ),
            });
        }
        let base = bytes.try_into().map_err(|b: Vec<u8>| ConfigError::Nvram {
            message: format!("image must contain 64 bytes, got {}", b.len()),
        })?;
        bases.insert(index, (base, origin));
    }
    Ok(bases)
}

/// The verified base of a journalled drive or catalogue medium, as its mount
/// answered it.
#[derive(Clone, Debug)]
pub struct DriveBase {
    verified: Option<DiskBase>,
}

/// Resolve file lengths before validating placement and disk geometry. Inline
/// presets borrow their config so the common path needs no additional copy.
#[cfg(any(test, feature = "test-support"))]
pub fn resolve_file_sources(
    config: &MachineConfig,
    options: crate::AssemblyOptions,
) -> Result<(Cow<'_, MachineConfig>, Vec<Option<DriveBase>>), ConfigError> {
    resolve_file_sources_with_catalogue(config, options, config.board.firmware_catalogue())
}

#[cfg(any(test, feature = "test-support"))]
pub fn resolve_file_sources_with_catalogue<'a>(
    config: &'a MachineConfig,
    options: crate::AssemblyOptions,
    catalogue: &[crate::FirmwareEntry],
) -> Result<(Cow<'a, MachineConfig>, Vec<Option<DriveBase>>), ConfigError> {
    resolve_sources_preverified(config, options, catalogue, &InlineMount)
}

pub fn resolve_sources_preverified<'a>(
    config: &'a MachineConfig,
    options: crate::AssemblyOptions,
    catalogue: &[crate::FirmwareEntry],
    mount: &dyn Mount,
) -> Result<(Cow<'a, MachineConfig>, Vec<Option<DriveBase>>), ConfigError> {
    config.check_part_sources()?;
    if config
        .images
        .iter()
        .filter(|image| matches!(image.source, ImageSource::Interleaved(_)))
        .count()
        > 1
    {
        return Err(ConfigError::FirmwareParts {
            message: "option ROMs cannot use parts; only firmware.bios supports parts".into(),
        });
    }
    if !config.drives.iter().any(|d| d.media.is_some())
        && !config
            .images
            .iter()
            .map(|image| &image.source)
            .chain(config.drives.iter().map(|drive| &drive.image))
            .any(|source| matches!(source, ImageSource::File(_) | ImageSource::Interleaved(_)))
    {
        return Ok((Cow::Borrowed(config), Vec::new()));
    }
    let bios_index = crate::firmware::system_bios(config).ok().and_then(|bios| {
        config
            .images
            .iter()
            .position(|image| std::ptr::eq(image, bios))
    });
    let mut resolved = config.clone();
    for (index, image) in resolved.images.iter_mut().enumerate() {
        if matches!(image.source, ImageSource::File(_)) {
            let bytes = load_mounted_bytes(mount, &image.source)?;
            let system_bios = bios_index == Some(index)
                || image.base.checked_add(bytes.len() as u64) == Some(0x100000);
            image.source = crate::firmware::rom_source(bytes, image.kind, system_bios, catalogue);
        }
    }
    resolved.check_part_sources()?;
    for image in &mut resolved.images {
        if let ImageSource::Interleaved(parts) = &image.source {
            let loaded = read_parts(mount, parts)?;
            let bytes = crate::interleave_parts(&loaded)?;
            crate::check_firmware_parts(&loaded, &bytes, catalogue, options)?;
            image.source = ImageSource::Inline(bytes);
        }
    }
    if let Ok(bios) = crate::firmware::system_bios(&resolved) {
        for (index, image) in config.images.iter().enumerate() {
            if matches!(image.source, ImageSource::Interleaved(_))
                && !std::ptr::eq(&resolved.images[index], bios)
            {
                return Err(ConfigError::FirmwareParts {
                    message: "option ROMs cannot use parts; only firmware.bios supports parts"
                        .into(),
                });
            }
        }
    }
    let mut base_metadata = vec![None; resolved.drives.len()];
    for (index, drive) in resolved.drives.iter_mut().enumerate() {
        if drive.media.is_some() {
            continue;
        }
        if let ImageSource::File(path) = &drive.image {
            if matches!(drive.write, WriteMode::Journal { .. }) {
                let verified =
                    verify_mounted_base(mount, path, drive.expected_sha256, &drive.kind, index)?;
                base_metadata[index] = Some(DriveBase {
                    verified: Some(verified),
                });
                continue;
            }
            let bytes = read_image(mount, path)?.into_parts().0;
            if let Some(expected) = drive.expected_sha256 {
                let actual: [u8; 32] = Sha256::digest(&bytes).into();
                if actual != expected {
                    return Err(ConfigError::DriveImageSha256 {
                        drive_index: index,
                        expected: format!("{:x}", sha2::digest::Output::<Sha256>::from(expected)),
                        actual: format!("{:x}", sha2::digest::Output::<Sha256>::from(actual)),
                    });
                }
            }
            base_metadata[index] = Some(DriveBase { verified: None });
            drive.image = ImageSource::Inline(bytes);
        }
    }
    for (drive_index, drive) in resolved.drives.iter_mut().enumerate() {
        if let Some(list) = &mut drive.media {
            for medium in &mut list.entries {
                let metadata = if let ImageSource::File(path) = &medium.image {
                    if matches!(medium.write, WriteMode::Journal { .. }) {
                        let verified = verify_mounted_base(
                            mount,
                            path,
                            medium.expected_sha256,
                            &drive.kind,
                            drive_index,
                        )?;
                        base_metadata.push(Some(DriveBase {
                            verified: Some(verified),
                        }));
                        continue;
                    }
                    let bytes = read_image(mount, path)?.into_parts().0;
                    if let Some(expected) = medium.expected_sha256 {
                        let actual: [u8; 32] = Sha256::digest(&bytes).into();
                        if actual != expected {
                            return Err(ConfigError::DriveImageSha256 {
                                drive_index,
                                expected: format!(
                                    "{:x}",
                                    sha2::digest::Output::<Sha256>::from(expected)
                                ),
                                actual: format!(
                                    "{:x}",
                                    sha2::digest::Output::<Sha256>::from(actual)
                                ),
                            });
                        }
                    }
                    medium.image = ImageSource::Inline(bytes);
                    Some(DriveBase { verified: None })
                } else {
                    None
                };
                if let Some(actual) = medium.image.known_len() {
                    drive.kind.check_image_len(drive_index, actual)?;
                }
                base_metadata.push(metadata);
            }
        }
    }
    Ok((Cow::Owned(resolved), base_metadata))
}

/// The base of a journalled drive from `mount`: its length is checked before
/// the mount reads a byte, and its SHA-256 against the config's pin.
pub fn verify_mounted_base(
    mount: &dyn Mount,
    path: &std::path::Path,
    expected: Option<[u8; 32]>,
    kind: &DriveKind,
    drive_index: usize,
) -> Result<DiskBase, ConfigError> {
    let base = mount
        .disk_base(path, &|length| kind.check_image_len(drive_index, length))
        .map_err(|error| match error {
            MountError::Config(error) => error,
            MountError::Host(error) => ConfigError::ImageRead {
                path: path.into(),
                message: error.to_string(),
            },
        })?;
    let actual = base.sha256();
    if let Some(expected) = expected {
        if actual != expected {
            return Err(ConfigError::DriveImageSha256 {
                drive_index,
                expected: format!("{:x}", sha2::digest::Output::<Sha256>::from(expected)),
                actual: format!("{:x}", sha2::digest::Output::<Sha256>::from(actual)),
            });
        }
    }
    kind.check_image_len(drive_index, base.len())?;
    Ok(base)
}

/// Build the port + MMIO decode map from the device windows, sorted so the map is
/// a pure function of the config, not of device build order (§4.1).
///
/// Ports sort by `(start, end)`, not by width. Validation accepts a flagged
/// overlap only for strict containment with the same start, so the narrower
/// window sorts first and owns every shared port. A POST latch at `0x80` can
/// therefore win over a DMA page range starting at `0x80`. A window starting
/// later inside a wider range would lose; callers must split the wider window.
pub fn build_decode_map(
    config: &MachineConfig,
    device_index: &BTreeMap<DeviceId, usize>,
) -> DecodeMap {
    let mut ports: Vec<PortRange> = Vec::new();
    for dc in &config.devices {
        let index = device_index[&dc.id];
        for window in &dc.ports {
            ports.push(PortRange {
                width: window.width,
                ports: window.start..=window.end,
                device: index,
            });
        }
    }
    ports.sort_by_key(|r| (*r.ports.start(), *r.ports.end()));

    let mut mmio: Vec<MmioRange> = config
        .memory
        .mmio
        .iter()
        .map(|window| MmioRange {
            addrs: window.base..=window.end() - 1,
            device: device_index[&window.device],
        })
        .collect();
    mmio.sort_by_key(|r| (*r.addrs.start(), *r.addrs.end()));

    DecodeMap { ports, mmio }
}

/// Build the device-IRQ → PIC-input wiring table from the config, resolving each
/// source [`DeviceId`] to its bus index (§5.1).
pub fn build_irq_lines(
    config: &MachineConfig,
    device_index: &BTreeMap<DeviceId, usize>,
) -> Vec<IrqLine> {
    config
        .devices
        .iter()
        .flat_map(|dc| {
            dc.interrupt_outputs().map(|(output, irq)| IrqLine {
                device: device_index[&dc.id],
                output,
                irq,
            })
        })
        .collect()
}

/// Build the bus-master → DMA-controller coupling table, resolving both the
/// requester and its named controller to their bus indices (§7.7).
pub fn build_dma_wirings(
    config: &MachineConfig,
    device_index: &BTreeMap<DeviceId, usize>,
) -> (Vec<DmaWiring>, Vec<DmaChannelWiring>) {
    let mut primary = Vec::new();
    let mut channels = Vec::new();
    for dc in &config.devices {
        let requester = device_index[&dc.id];
        for (input, dma) in dc.dma_inputs().enumerate() {
            let controller = device_index[&dma.controller];
            if input == 0 {
                primary.push(DmaWiring {
                    requester,
                    controller,
                });
            }
            if dma.controller == DeviceId::DmaWord {
                debug_assert!(
                    matches!(dma.channel, 5..=7),
                    "check_dma_controllers_present validates word DMA system channels 5..=7"
                );
                // IBM Personal Computer AT Technical Reference, System Board,
                // Direct Memory Access: word-controller channels are local 1–3.
                channels.push(DmaChannelWiring {
                    requester,
                    system_channel: dma.channel,
                    controller,
                    local_channel: dma.channel - 4,
                });
            }
        }
    }
    (primary, channels)
}

pub fn mount_ide_drives(
    config: &MachineConfig,
    controller: DeviceId,
    persistence: &mut Vec<crate::PersistenceHandle>,
    base_metadata: &[Option<DriveBase>],
    controller_index: usize,
    mount: &dyn Mount,
) -> Result<[Option<AtaDrive>; 2], AssemblyError> {
    let mut slots = [None, None];
    for (drive_index, drive) in config
        .drives
        .iter()
        .enumerate()
        .filter(|(_, drive)| drive.controller == controller)
    {
        let DriveKind::HardDisk { geometry } = drive.kind else {
            continue;
        };
        let loaded = base_metadata.get(drive_index).and_then(Option::as_ref);
        let verified = loaded.and_then(|b| b.verified.as_ref());
        let bytes = if verified.is_some() {
            Vec::new()
        } else {
            load_image_bytes(&drive.image)?
        };
        let capacity = (drive.kind.image_bytes() / 512) as u32;
        let media: Box<dyn device::SectorMedia> = match &drive.write {
            WriteMode::Journal {
                path,
                keep_generations,
            } => {
                let base = match verified {
                    Some(base) => base.clone(),
                    None => journal::VerifiedBase::from_bytes(&bytes)
                        .map_err(|source| AssemblyError::Journal {
                            drive_index,
                            source: source.into(),
                        })?
                        .into(),
                };
                let (mut handle, media) = crate::PersistenceHandle::mount_journal(
                    drive_index,
                    mount,
                    path,
                    &base,
                    drive.kind.image_bytes(),
                    *keep_generations,
                )?;
                handle.ata_index = Some(controller_index);
                persistence.push(handle);
                media
            }
            mode => Box::new(device::InMemoryMedia::new(
                bytes,
                u64::from(capacity),
                *mode == WriteMode::ReadOnly,
            )),
        };
        let geometry = AtaGeometry {
            cylinders: geometry.cylinders as u16,
            heads: geometry.heads as u8,
            sectors_per_track: geometry.sectors_per_track as u8,
        };
        slots[usize::from(drive.unit)] = Some(
            AtaDrive::with_media(geometry, capacity, media)
                .map_err(|_| ConfigError::InvalidAtaGeometry { drive_index })?,
        );
    }
    Ok(slots)
}

/// Route the config's floppy drives into the controller's four drive positions by
/// their `controller` and `unit` (§7.7).
///
/// Only floppy drives are mounted here; a slot past the fourth position is
/// ignored (validation bounds units to both declared count and hardware). The
/// mounted medium's geometry is the format its image size gives among the
/// media the drive type takes ([`FloppyGeometry::medium_for_len`]).
///
pub fn mount_fdc_drives(
    config: &MachineConfig,
    controller: DeviceId,
    persistence: &mut Vec<crate::PersistenceHandle>,
    base_metadata: &[Option<DriveBase>],
    mount: &Rc<dyn Mount>,
) -> Result<[Option<Drive>; 4], AssemblyError> {
    let mut slots: [Option<Drive>; 4] = [None, None, None, None];
    for (drive_index, drive) in config.drives.iter().enumerate() {
        if drive.controller != controller {
            continue;
        }
        let DriveKind::Floppy { geometry } = &drive.kind else {
            continue;
        };
        let slot = drive.unit as usize;
        if slot >= slots.len() {
            continue;
        }
        if drive.media.is_some() {
            continue;
        }
        let loaded = base_metadata.get(drive_index).and_then(Option::as_ref);
        let verified = loaded.and_then(|b| b.verified.as_ref());
        let image = if verified.is_some() {
            Vec::new()
        } else {
            load_image_bytes(&drive.image)?
        };
        let length = verified.map_or(image.len() as u64, DiskBase::len);
        let geometry = medium_geometry(geometry, length, drive_index)?;
        slots[slot] = Some(match &drive.write {
            WriteMode::ReadOnly => Drive::new_read_only(geometry, image),
            WriteMode::Volatile => Drive::new(geometry, image),
            WriteMode::Journal {
                path,
                keep_generations,
            } => {
                let (mut handle, media) = crate::PersistenceHandle::prepare_mounted(
                    drive_index,
                    mount.clone(),
                    path,
                    match verified {
                        Some(base) => base.clone(),
                        None => journal::VerifiedBase::from_bytes(&image)
                            .map_err(|source| AssemblyError::Journal {
                                drive_index,
                                source: source.into(),
                            })?
                            .into(),
                    },
                    *keep_generations,
                );
                handle.floppy = Some((controller, drive.unit, 0));
                handle.insert()?;
                persistence.push(handle);
                Drive::with_journaled_media(geometry, media)
            }
        });
    }
    Ok(slots)
}

pub fn mount_removable_fdc(
    config: &MachineConfig,
    controller: DeviceId,
    persistence: &mut Vec<crate::PersistenceHandle>,
    metadata: &[Option<DriveBase>],
    mount: &Rc<dyn Mount>,
) -> Result<Upd765, AssemblyError> {
    let mut slots = mount_fdc_drives(config, controller, persistence, metadata, mount)?;
    let mut fdc = Upd765::new();
    let mut metadata_index = config.drives.len();
    for (drive_index, drive) in config.drives.iter().enumerate() {
        let start = metadata_index;
        metadata_index += drive.media.as_ref().map_or(0, |list| list.entries.len());
        if drive.controller != controller {
            continue;
        }
        let DriveKind::Floppy {
            geometry: drive_geometry,
        } = drive.kind
        else {
            continue;
        };
        let unit = usize::from(drive.unit);
        let install_error = |e: devices::upd765::MediaChangeError| ConfigError::Journal {
            drive_index,
            message: e.to_string(),
        };
        let Some(list) = &drive.media else {
            // A single image is a one-entry catalogue with its disk inserted.
            if let Some(medium) = slots.get_mut(unit).and_then(Option::take) {
                fdc.set_media_list(unit, drive_geometry.drive_type(), vec![medium], false)
                    .map_err(install_error)?;
            }
            continue;
        };
        let mut entries = Vec::new();
        for (index, medium) in list.entries.iter().enumerate() {
            let verified = metadata
                .get(start + index)
                .and_then(Option::as_ref)
                .and_then(|b| b.verified.as_ref());
            let image = if verified.is_some() {
                Vec::new()
            } else {
                load_image_bytes(&medium.image)?
            };
            let length = verified.map_or(image.len() as u64, DiskBase::len);
            let geometry = medium_geometry(&drive_geometry, length, drive_index)?;
            entries.push(match &medium.write {
                WriteMode::Volatile => Drive::new(geometry, image),
                WriteMode::ReadOnly => Drive::new_read_only(geometry, image),
                WriteMode::Journal {
                    path,
                    keep_generations,
                } => {
                    let (mut handle, media) = crate::PersistenceHandle::prepare_mounted(
                        drive_index,
                        mount.clone(),
                        path,
                        match verified {
                            Some(base) => base.clone(),
                            None => journal::VerifiedBase::from_bytes(&image)
                                .map_err(|source| AssemblyError::Journal {
                                    drive_index,
                                    source: source.into(),
                                })?
                                .into(),
                        },
                        *keep_generations,
                    );
                    handle.floppy = Some((controller, drive.unit, index));
                    if index == list.inserted && !list.empty {
                        handle.insert()?;
                    }
                    persistence.push(handle);
                    Drive::with_journaled_media(geometry, media)
                }
            });
        }
        fdc.set_media_list_from(
            unit,
            drive_geometry.drive_type(),
            entries,
            list.empty,
            list.inserted,
        )
        .map_err(install_error)?;
    }
    Ok(fdc)
}

/// The controller geometry of the medium whose image is `length` bytes in a
/// drive of type `drive`: one of the media the drive type takes.
pub fn medium_geometry(
    drive: &FloppyGeometry,
    length: u64,
    drive_index: usize,
) -> Result<devices::FloppyGeometry, ConfigError> {
    DriveKind::Floppy { geometry: *drive }.check_image_len(drive_index, length)?;
    Ok(drive.medium_for_len(length).unwrap_or(*drive).to_device())
}

/// Derive the CMOS seed the BIOS expects from the memory map and the attached
/// drives and selected primary display (§7.6).
///
/// The BIOS checksums floppy types, the equipment byte, and the base/extended
/// memory sizes against the real machine, so these are computed here rather than
/// supplied by the caller: base memory is the RAM below 640 KiB, extended memory
/// is the RAM at or above 1 MiB, and the floppy nibbles and equipment byte come
/// from the primary display and floppy drives on the primary controller
/// (DeviceId::Fdc). Secondary
/// controllers have no entries in the AT CMOS diskette inventory. For a plain AT — 640 KiB base, one 1.44 MB
/// floppy, no display, hard disk or coprocessor — this reproduces [`devices::CmosConfig::default`]
/// byte-for-byte apart from the BIOS century byte at 32h, seeded in BCD from
/// the RTC start year. The returned 64-byte seed includes extended disk types
/// and the checksum; explicit NVRAM images bypass it during assembly.
///
/// IBM Personal Computer AT Technical Reference (September 1985), System Board,
/// Real-Time Clock/CMOS RAM Information: Fixed Disk Type Byte (Hex 12), Drive C/D
/// Extended Bytes (Hex 19/1A), BIOS Fixed Disk Parameters, Equipment Byte
/// (Hex 14, bits 5–4: display, bit 1: math coprocessor installed), and
/// Checksum (Hex 2E/2F); CMOS RAM address map, Date Century Byte (Hex 32),
/// outside the checksum range. The March 1984 edition defines the 40/80-column
/// colour and monochrome equipment codes.
/// Only a hash-matched system BIOS with `cmos-drive-types` and `fixed-disk`
/// enables these types. Any fixed-disk option ROM keeps them zero. Geometries
/// outside the documented table are refused: this reference reserves type 47,
/// and does not define a user-geometry layout. The separate `cmos-qemu-layout`
/// convention uses type 47 instead; see [`derive_cmos_layout`].
pub fn derive_cmos_config(config: &MachineConfig) -> Result<[u8; 64], ConfigError> {
    derive_cmos_config_with_catalogue(config, config.board.firmware_catalogue())
}

pub fn derive_cmos_config_with_catalogue(
    config: &MachineConfig,
    catalogue: &[crate::FirmwareEntry],
) -> Result<[u8; 64], ConfigError> {
    let layout = derive_cmos_layout_with_catalogue(config, catalogue)?;
    let mut seed = [0; 64];
    seed.copy_from_slice(&layout[..64]);
    Ok(seed)
}

/// Derive a complete QEMU-compatible CMOS address space when the hash-matched
/// system BIOS opts into `cmos-qemu-layout`; otherwise extend the AT seed with zeros.
///
/// READ: QEMU hw/i386/pc.c (29760ef), pc_cmos_init_late, cmos_init_hd and
/// set_boot_dev; hw/i386/x86-common.c, x86_rtc_set_cpus_count;
/// hw/rtc/mc146818rtc.c, rtc_set_cmos. SeaBIOS src/fw/paravirt.c,
/// qemu_preinit, qemu_cfg_e820 and qemu_get_present_cpus_count;
/// src/boot.c, boot_init; src/block.c, get_translation; src/clock.c, clock_setup;
/// src/hw/floppy.c, floppy_setup. See the firmware design §3.6.
/// The existing equipment/floppy bytes and AT checksum (10h–2Dh) are retained.
/// The fixed boot policy is floppy, then hard disk, with floppy signature checks
/// enabled (QEMU's fd_bootchk default with boot order ac, rather than its default
/// cad). One CPU is modelled. Type 47 describes
/// primary IDE drives even if an option ROM supplies fixed-disk services: these
/// bytes describe hardware, independently of the IBM BIOS provider convention.
///
/// Assembly loads all bytes into a 128-byte RTC and only the low 64 bytes into
/// an MC146818. The NVRAM image API remains 64-byte only. Firmware validation requires a
/// 128-byte RTC for this capability: otherwise 5Bh–5Dh and 5Fh alias type-47
/// geometry and falsely report high RAM and extra CPUs. Pure derivation remains
/// available for synthetic maps without validating device requirements.
pub fn derive_cmos_layout(config: &MachineConfig) -> Result<[u8; 128], ConfigError> {
    derive_cmos_layout_with_catalogue(config, config.board.firmware_catalogue())
}

pub fn derive_cmos_layout_with_catalogue(
    config: &MachineConfig,
    catalogue: &[crate::FirmwareEntry],
) -> Result<[u8; 128], ConfigError> {
    let base_memory_kb = clamp_kb(ram_bytes_in(config, 0, CONVENTIONAL_TOP));
    let extended_memory_kb = clamp_kb(ram_bytes_in(config, EXTENDED_BASE, u64::MAX));

    // Floppy nibbles: high = drive 0 (slot 0), low = drive 1 (slot 1).
    let mut floppy_types = 0u8;
    let mut floppy_count = 0u8;
    for drive in &config.drives {
        match &drive.kind {
            DriveKind::Floppy { geometry } => {
                if drive.controller != DeviceId::Fdc {
                    continue;
                }
                floppy_count += 1;
                let code = floppy_cmos_type(geometry);
                match drive.unit {
                    0 => floppy_types |= code << 4,
                    1 => floppy_types |= code,
                    _ => {}
                }
            }
            DriveKind::HardDisk { .. } => {}
        }
    }

    let mut equipment = config.primary_display()?.cmos_video_bits();
    if config.board == crate::BoardModel::Ibm5170 && config.cpu.fpu == crate::FpuChip::I80287 {
        equipment |= 0x02;
    }
    if floppy_count > 0 {
        equipment |= 0x01;
        equipment |= (floppy_count - 1).min(0x03) << 6;
    }

    let mut seed = [0; 128];
    seed[0x0B] = 0x02;
    seed[0x0D] = 0x80;
    seed[0x10] = floppy_types;
    seed[0x14] = equipment;
    seed[0x15..=0x16].copy_from_slice(&base_memory_kb.to_le_bytes());
    seed[0x17..=0x18].copy_from_slice(&extended_memory_kb.to_le_bytes());

    let start = config
        .devices
        .iter()
        .find_map(|device| match &device.kind {
            DeviceKind::Rtc(rtc) => Some(rtc.start),
            _ => None,
        })
        .unwrap_or_default();
    let century = (start.year / 100) as u8;
    seed[0x32] = ((century / 10) << 4) | (century % 10);

    let bios = crate::firmware::system_bios(config).ok();
    let entry = bios.and_then(|image| crate::firmware::matched_entry(image, catalogue));
    if let Some(entry) = entry {
        crate::firmware::check_cmos_capabilities(entry)?;
        if entry
            .capabilities
            .contains(&crate::FirmwareCapability::CmosQemuLayout)
        {
            seed_qemu_cmos(config, &mut seed);
        }
    }
    let system_disk_types = entry.is_some_and(|entry| {
        entry
            .capabilities
            .contains(&crate::firmware::FirmwareCapability::CmosDriveTypes)
            && entry.provides.contains(&crate::FirmwareService::FixedDisk)
    });
    let mut option_disk_provider = false;
    for image in &config.images {
        if image.kind != ImageKind::Rom || bios.is_some_and(|bios| std::ptr::eq(image, bios)) {
            continue;
        }
        let services = crate::firmware::matched_entry(image, catalogue)
            .map_or(image.provides.as_slice(), |entry| entry.provides);
        option_disk_provider |= services.contains(&crate::FirmwareService::FixedDisk);
    }
    if system_disk_types && !option_disk_provider {
        for (drive_index, drive) in config.drives.iter().enumerate() {
            if let DriveKind::HardDisk { geometry } = drive.kind {
                if drive.controller != DeviceId::Ide || drive.unit > 1 {
                    continue;
                }
                let disk_type = fixed_disk_cmos_type(&geometry).ok_or(
                    ConfigError::UnsupportedCmosDiskGeometry {
                        drive_index,
                        geometry,
                    },
                )?;
                let nibble = disk_type.min(15);
                seed[0x12] |= nibble << if drive.unit == 0 { 4 } else { 0 };
                if disk_type > 15 {
                    seed[0x19 + usize::from(drive.unit)] = disk_type;
                }
            }
        }
    }
    let checksum: u16 = seed[0x10..=0x2D].iter().map(|&byte| u16::from(byte)).sum();
    seed[0x2E..=0x2F].copy_from_slice(&checksum.to_be_bytes());
    Ok(seed)
}

/// READ: QEMU hw/i386/pc.c, pc_cmos_init_late, cmos_init_hd and set_boot_dev;
/// hw/rtc/mc146818rtc.c, rtc_set_cmos. RAM is counted separately below and above
/// 4 GiB so remapped high RAM cannot inflate the low-memory fields.
pub fn seed_qemu_cmos(config: &MachineConfig, seed: &mut [u8; 128]) {
    const FOUR_GIB: u64 = 1 << 32;
    let extended = clamp_kb(ram_bytes_in(config, EXTENDED_BASE, FOUR_GIB));
    seed[0x17..=0x18].copy_from_slice(&extended.to_le_bytes());
    seed[0x30..=0x31].copy_from_slice(&extended.to_le_bytes());
    let above_16m =
        (ram_bytes_in(config, 16 << 20, FOUR_GIB) / 65536).min(u64::from(u16::MAX)) as u16;
    seed[0x34..=0x35].copy_from_slice(&above_16m.to_le_bytes());
    let above_4g = ram_bytes_in(config, FOUR_GIB, u64::MAX) / 65536;
    seed[0x5B..=0x5D].copy_from_slice(&above_4g.to_le_bytes()[..3]);
    seed[0x3D] = 0x21;
    seed[0x38] = 0;
    seed[0x5F] = 0;

    for drive in &config.drives {
        let DriveKind::HardDisk { geometry } = drive.kind else {
            continue;
        };
        let Some(channel) = config.devices.iter().find_map(|device| {
            if device.id != drive.controller {
                return None;
            }
            match device.kind {
                DeviceKind::Ide(ide) if ide.base == 0x1F0 => Some(0),
                DeviceKind::Ide(ide) if ide.base == 0x170 => Some(1),
                _ => None,
            }
        }) else {
            continue;
        };
        if drive.unit > 1 {
            continue;
        }
        let slot = channel * 2 + usize::from(drive.unit);
        seed[0x39] |= qemu_ata_translation(&geometry) << (slot * 2);
        if channel != 0 {
            continue;
        }
        let unit = usize::from(drive.unit);
        seed[0x12] |= 0x0F << if unit == 0 { 4 } else { 0 };
        seed[0x19 + unit] = 47;
        let offset = 0x1B + unit * 9;
        let cylinders = (geometry.cylinders as u16).to_le_bytes();
        seed[offset..offset + 9].copy_from_slice(&[
            cylinders[0],
            cylinders[1],
            geometry.heads as u8,
            0xFF,
            0xFF,
            0xC0 | if geometry.heads > 8 { 8 } else { 0 },
            cylinders[0],
            cylinders[1],
            geometry.sectors_per_track as u8,
        ]);
    }
}

/// READ: QEMU hw/block/block.c, blkconf_geometry, selects automatic translation
/// for explicit CHS through hw/block/hd-geometry.c, hd_bios_chs_auto_trans:
/// none within 1024/16/63, LBA otherwise. Validation limits geometry to
/// 16383/16/63, so only cylinders above 1024 select LBA for valid configs.
pub fn qemu_ata_translation(geometry: &crate::HardDiskGeometry) -> u8 {
    u8::from(geometry.cylinders > 1024 || geometry.heads > 16 || geometry.sectors_per_track > 63)
}

/// IBM Personal Computer AT Technical Reference (September 1985), Real-Time
/// Clock/CMOS RAM Information, BIOS Fixed Disk Parameters (pp. 1-63 and 1-66),
/// and System BIOS listing, FD_TBL: 512-byte sectors, 17 sectors per track.
/// CHS cannot distinguish write-precompensation/landing-zone variants, so the
/// lowest matching type wins. Types 23–255 are reserved, including type 47.
pub fn fixed_disk_cmos_type(geometry: &crate::HardDiskGeometry) -> Option<u8> {
    const TYPES: &[(u8, u32, u32)] = &[
        (1, 306, 4),
        (2, 615, 4),
        (3, 615, 6),
        (4, 940, 8),
        (5, 940, 6),
        (6, 615, 4),
        (7, 462, 8),
        (8, 733, 5),
        (9, 900, 15),
        (10, 820, 3),
        (11, 855, 5),
        (12, 855, 7),
        (13, 306, 8),
        (14, 733, 7),
        (16, 612, 4),
        (17, 977, 5),
        (18, 977, 7),
        (19, 1024, 7),
        (20, 733, 5),
        (21, 733, 7),
        (22, 733, 7),
    ];
    if geometry.bytes_per_sector != 512 || geometry.sectors_per_track != 17 {
        return None;
    }
    TYPES.iter().find_map(|&(kind, cylinders, heads)| {
        (geometry.cylinders == cylinders && geometry.heads == heads).then_some(kind)
    })
}

/// Total bytes of RAM lying within `[low, high)`, clamping each region to the
/// window so a region that straddles the boundary contributes only its
/// in-window part.
pub fn ram_bytes_in(config: &MachineConfig, low: u64, high: u64) -> u64 {
    config
        .memory
        .ram
        .iter()
        .map(|r| {
            let start = r.base.clamp(low, high);
            let end = r.end().clamp(low, high);
            end.saturating_sub(start)
        })
        .sum()
}

/// Convert a byte count to whole kilobytes, saturating at the CMOS field's 16-bit
/// range.
pub fn clamp_kb(bytes: u64) -> u16 {
    (bytes / 1024).min(u64::from(u16::MAX)) as u16
}

/// The CMOS floppy-type code for a geometry: 1.44 MB → 4, 1.2 MB → 2, 720 KB → 3,
/// 360 KB → 1 (the standard AT nibble values); an unrecognised geometry reports 0
/// (no drive).
pub fn floppy_cmos_type(geometry: &FloppyGeometry) -> u8 {
    if *geometry == FloppyGeometry::HD_1_44 {
        4
    } else if *geometry == FloppyGeometry::HD_1_2 {
        2
    } else if *geometry == FloppyGeometry::DD_720K {
        3
    } else if *geometry == FloppyGeometry::DD_360K {
        1
    } else {
        0
    }
}

#[cfg(test)]
#[path = "generic386_tests.rs"]
mod generic386_tests;
