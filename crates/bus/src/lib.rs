// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bus swap seam (§3.2 seam 3, §3.3, §4.1).
//!
//! The [`Bus`] **executes** a decode map assembly **derives from configuration** — the
//! routing contract (§3.3). It owns the set of
//! [`Device`]s, a table of port ranges → device index, and a table of MMIO
//! ranges → device index, and dispatches guest accesses to the right device. It
//! holds **no** decode *policy*: which ports/addresses map where is the
//! assembler's business.
//!
//! Port windows default to 8-bit cycles. A 16-bit window models IOCS16#
//! (IBM Personal Computer AT Technical Reference, Section 1, "System Board",
//! "I/O Channel", "I/O Channel Signal Description", "-I/O CS16"). An aligned
//! word reaches that device once with size 2. The starting port selects the
//! cycle width: a single even-port window can serve a word register without
//! accessing a different register decoded at the next port. An odd-aligned
//! word uses two independently decoded byte cycles. An aligned 32-bit access uses two
//! word cycles at port and port+2 when both select 16-bit windows; mixed
//! windows are decoded anew after each cycle, low bytes first. Port arithmetic
//! wraps at 16 bits. Wait states and physical byte-lane signals are not modelled.
//! See `backlog/docs/design/spec.md` §4.1 and
//! `backlog/docs/design/machine-config.md` §Port window width.
//! Device and open-bus traces record actual cycle sizes when enabled.
//! Recording defaults to Off; both
//! use [`device::TraceBuffer`] and the same bounded-ring/drop-count contract.
//! See `backlog/docs/design/observation.md`, §Traces and later audio.
//!
//! MMIO accesses within one decode window retain their original size. Accesses
//! crossing windows use contiguous sub-accesses in ascending address order,
//! composed little-endian. An undecoded address faults with
//! [`BusError::UnmappedMmio`]; earlier sub-accesses remain completed and later
//! ones are not attempted. MMIO never uses the port open-bus policy.
//!
//! [`Bus::tick_due`] dispatches due transitions; [`Bus::tick_all`] supports the
//! per-clock test oracle. IRQ levels reach the machine through [`Bus::device_irq_output`].
//! Before a port/MMIO access or mutable borrow, the bus synchronizes the target,
//! then its declared access dependents in ascending bus-index order, before any
//! register side effect. This keeps a dependent timer's next deadline based on
//! current time. Couplings are directed and do not expand transitively. Unrelated
//! devices receive no access synchronization; [`Bus::settle`] visits every
//! device once in bus order. `EveryPitClock` devices need no extra sync in either
//! path. See `backlog/docs/design/scheduler.md`, §Adoption.

#![forbid(unsafe_code)]

use std::ops::RangeInclusive;

use device::{AccessDir, Device, DeviceError, NextEvent, TraceEvent, VClock};
use thiserror::Error;

/// ISA port cycle width; IOCS16# opts a window into word cycles.
///
/// IBM Personal Computer AT Technical Reference, Section 1, System Board,
/// I/O Channel, I/O Channel Signal Description, -I/O CS16. See
/// `backlog/docs/design/machine-config.md` §Port window width.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PortWidth {
    /// No IOCS16# assertion: wider accesses use byte cycles.
    #[default]
    Byte,
    /// IOCS16# asserted: even ports accept word cycles.
    Word,
}

/// A contiguous port range routed to one device (§4.1).
pub struct PortRange {
    /// Inclusive 16-bit port range this device decodes.
    pub ports: RangeInclusive<u16>,
    /// Index into the [`Bus`]'s device vector.
    pub device: usize,
    /// Maximum cycle width selected by the starting port.
    pub width: PortWidth,
}

/// A contiguous MMIO address range routed to one device (§4.1).
pub struct MmioRange {
    /// Inclusive guest-physical address range this device decodes.
    pub addrs: RangeInclusive<u64>,
    /// Index into the [`Bus`]'s device vector.
    pub device: usize,
}

/// The decode map an assembler hands the bus (§3.3).
///
/// This is pure data: assembly derives it, the [`Bus`] executes it. There is
/// no decode logic here — only the ranges and the device indices they resolve
/// to.
#[derive(Default)]
pub struct DecodeMap {
    /// Port-space routing table.
    pub ports: Vec<PortRange>,
    /// MMIO-space routing table.
    pub mmio: Vec<MmioRange>,
}

/// What the bus does with a port access no device decodes (§4.1).
///
/// On a real ISA machine the data bus is not actively driven for an undecoded
/// port: a read floats to all-ones (0xFF per byte — "open bus"), and a write
/// simply has no listener and is dropped. A stock BIOS relies on this — its POST
/// pokes many ports that a minimally-populated board never decodes (the DMA page
/// latches, the CGA registers on a machine with no CGA, a floppy-controller
/// probe, spare serial/parallel probes) and must not fault on any of them.
///
/// [`OpenBus`](UndecodedPortAccess::OpenBus) models that hardware behaviour and
/// is the default. [`Fault`](UndecodedPortAccess::Fault) instead raises
/// [`BusError::UnmappedPort`], for diagnostics or tests that want every port
/// access to resolve to a real device.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum UndecodedPortAccess {
    /// Read returns all-ones, write is dropped; both are traced. The hardware
    /// default an unmodified BIOS depends on.
    #[default]
    OpenBus,
    /// Any undecoded port access raises [`BusError::UnmappedPort`].
    Fault,
}

/// Errors raised while executing the decode (§4.1).
#[derive(Debug, Error)]
pub enum BusError {
    /// No device decodes this port.
    #[error("unmapped port {port:#06x}")]
    UnmappedPort {
        /// The undecoded 16-bit port.
        port: u16,
    },
    /// No device decodes this MMIO address.
    #[error("unmapped mmio address {addr:#x}")]
    UnmappedMmio {
        /// The undecoded guest-physical address.
        addr: u64,
    },
    /// The target device raised an error servicing the access.
    #[error("device error: {0}")]
    Device(#[from] DeviceError),
}

/// Invalid directed access coupling: both indices must exist and be distinct.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error(
    "invalid access coupling from device {device} to {dependent}: indices must exist and differ"
)]
pub struct AccessCouplingError {
    /// Index of the accessed device.
    pub device: usize,
    /// Index of the device whose timer can change.
    pub dependent: usize,
}

/// The bus: owns devices + the decode map and dispatches accesses (§3.3).
pub struct Bus {
    #[cfg(feature = "host-profile")]
    profile: device::profile::ProfileHook,
    devices: Vec<Box<dyn Device>>,
    map: DecodeMap,
    deadlines: Vec<Option<NextEvent>>,
    access_dependents: Vec<Vec<usize>>,
    /// Accesses may change a pin even when no autonomous timer is due. Sample
    /// those changes at the next clock, preserving the reference loop's order.
    pins_dirty: bool,
    /// Policy for a port access no device decodes (§4.1). Open-bus by default.
    undecoded_ports: UndecodedPortAccess,
    /// Virtual time of the most recent [`Bus::tick_all`] or [`Bus::tick_due`],
    /// stamped onto open-bus traces and shared with diagnostic observers, even
    /// when their devices are dormant.
    now: std::rc::Rc<std::cell::Cell<VClock>>,
    /// Open-bus accesses observed since the last [`Bus::take_open_bus_trace`].
    ///
    /// An undecoded port has no device to record it on the [`Device`] tracing
    /// seam, so explicitly enabled diagnostics use the same bounded recorder
    /// as devices.
    open_bus_trace: device::TraceBuffer,
}

impl Bus {
    /// Build a bus from a device set and the configuration-derived decode map.
    ///
    /// The `map`'s device indices must reference `devices`; the assembler is
    /// responsible for keeping them consistent (§3.3). Undecoded port accesses
    /// default to open-bus ([`UndecodedPortAccess::OpenBus`]); use
    /// [`Bus::with_undecoded_ports`] to select strict faulting instead.
    pub fn new(devices: Vec<Box<dyn Device>>, map: DecodeMap) -> Self {
        Self {
            #[cfg(feature = "host-profile")]
            profile: Default::default(),
            deadlines: vec![None; devices.len()],
            access_dependents: vec![Vec::new(); devices.len()],
            pins_dirty: true,
            devices,
            map,
            undecoded_ports: UndecodedPortAccess::default(),
            now: std::rc::Rc::new(std::cell::Cell::new(VClock::ZERO)),
            open_bus_trace: device::TraceBuffer::default(),
        }
    }

    /// Bind passive host diagnostics; hardware state and deadlines are unchanged.
    #[cfg(feature = "host-profile")]
    pub fn set_profile(&mut self, profile: device::profile::ProfileHook) {
        self.profile = profile;
    }

    /// Shared passive diagnostics for rendering and observation at bus boundaries.
    #[cfg(feature = "host-profile")]
    pub fn profile(&self) -> &device::profile::ProfileHook {
        &self.profile
    }

    /// Declare that an access to `device` can change `dependent`'s next event.
    ///
    /// Before access, synchronize the target first, then its dependents in
    /// ascending bus-index order. Repeated declarations are idempotent. Edges
    /// are directed: declare every affected timer explicitly, including indirect
    /// dependents. Legacy every-clock devices receive no additional sync calls.
    /// Invalid indices and self-coupling are rejected without changing the bus.
    ///
    /// The machine probes each DMA requester through a mutable borrow on every
    /// settle pass, even when no transfer is pending. Declaring dependents on a
    /// requester therefore re-synchronizes them on each probe. This only costs
    /// host time: synchronization must be idempotent at the same virtual clock.
    pub fn add_access_coupling(
        &mut self,
        device: usize,
        dependent: usize,
    ) -> Result<(), AccessCouplingError> {
        if device >= self.devices.len() || dependent >= self.devices.len() || device == dependent {
            return Err(AccessCouplingError { device, dependent });
        }
        let dependents = &mut self.access_dependents[device];
        if let Err(position) = dependents.binary_search(&dependent) {
            dependents.insert(position, dependent);
        }
        Ok(())
    }

    /// Set how the bus handles a port access no device decodes, returning the
    /// bus for chaining. Defaults to [`UndecodedPortAccess::OpenBus`].
    #[must_use]
    pub fn with_undecoded_ports(mut self, policy: UndecodedPortAccess) -> Self {
        self.undecoded_ports = policy;
        self
    }

    /// The current undecoded-port policy (§4.1).
    pub fn undecoded_ports(&self) -> UndecodedPortAccess {
        self.undecoded_ports
    }

    /// Share a read-only view of virtual bus time with diagnostic observers
    /// whose devices may be dormant. Reading it does not synchronize hardware.
    pub fn observation_clock(&self) -> std::rc::Rc<dyn Fn() -> VClock> {
        let now = self.now.clone();
        std::rc::Rc::new(move || now.get())
    }

    /// Configure every device and the open-bus recorder with one explicit policy.
    /// Clears diagnostic histories and drop counts without changing hardware.
    pub fn set_trace_policy(&mut self, policy: device::TracePolicy) {
        self.open_bus_trace.set_policy(policy);
        for device in &mut self.devices {
            device.set_trace_policy(policy);
        }
    }

    /// Open-bus events overwritten since trace configuration, preserved on drain.
    pub fn open_bus_trace_dropped(&self) -> u64 {
        self.open_bus_trace.dropped()
    }

    /// Drain the open-bus accesses absorbed since the last call.
    ///
    /// Mirrors [`Device::take_trace`] for accesses that never reached a device:
    /// with recording enabled, each undecoded port read/write serviced under
    /// [`UndecodedPortAccess::OpenBus`] appears here as a [`TraceEvent::Port`].
    pub fn take_open_bus_trace(&mut self) -> Vec<TraceEvent> {
        self.open_bus_trace.take()
    }

    fn port_chunk(&self, port: u16, size: u8) -> (Option<usize>, u8) {
        let range = self
            .map
            .ports
            .iter()
            .find(|range| range.ports.contains(&port));
        let word = range.is_some_and(|range| range.width == PortWidth::Word);
        let chunk = if size >= 2 && port.is_multiple_of(2) && word {
            2
        } else {
            size.min(1)
        };
        (range.map(|range| range.device), chunk)
    }

    /// Resolve the leading sub-access without changing widths inside a window.
    fn mmio_chunk(&self, addr: u64, size: u8) -> Result<(usize, u8), BusError> {
        let range = self
            .map
            .mmio
            .iter()
            .find(|r| r.addrs.contains(&addr))
            .ok_or(BusError::UnmappedMmio { addr })?;
        // Invalid widths belong to the device's validation, never to a set of
        // smaller, apparently valid accesses with partial side effects.
        if !(1..=8).contains(&size) {
            return Ok((range.device, size));
        }
        addr.checked_add(u64::from(size) - 1)
            .ok_or(BusError::UnmappedMmio { addr })?;
        let last_offset = *range.addrs.end() - addr;
        let chunk = if u64::from(size) - 1 <= last_offset {
            size
        } else {
            (last_offset + 1) as u8
        };
        Ok((range.device, chunk))
    }

    /// Dispatch a guest `in` at `port`, `size` bytes wide.
    ///
    /// Cycles use the starting window's width and combine little-endian, as
    /// described in the module documentation. Port numbers wrap in the 16-bit
    /// port space. A fault stops the access; earlier cycles remain completed.
    ///
    /// An undecoded port under [`UndecodedPortAccess::OpenBus`] returns all-ones
    /// (0xFF per byte), matching a real ISA bus that no device drives, and is
    /// recorded on the open-bus trace when its trace policy records; under [`UndecodedPortAccess::Fault`] it
    /// raises [`BusError::UnmappedPort`].
    pub fn port_read(&mut self, port: u16, size: u8) -> Result<u32, BusError> {
        #[cfg(feature = "host-profile")]
        let _span = self.profile.span(device::profile::ProfilePhase::PortIo);
        let (device, chunk) = self.port_chunk(port, size);
        let value = match device {
            Some(idx) => {
                self.sync_access(idx);
                Ok(self.devices[idx].port_read(port, chunk)?)
            }
            None => match self.undecoded_ports {
                UndecodedPortAccess::OpenBus => {
                    let value = open_bus_value(chunk);
                    self.trace_open_bus(port, chunk, AccessDir::Read, value);
                    Ok(value)
                }
                UndecodedPortAccess::Fault => Err(BusError::UnmappedPort { port }),
            },
        }?;
        if size == chunk {
            return Ok(value);
        }
        let rest = self.port_read(port.wrapping_add(u16::from(chunk)), size - chunk)?;
        let rest_mask = u32::MAX >> (8 * (4 - (size - chunk)));
        Ok((value & open_bus_value(chunk)) | ((rest & rest_mask) << (8 * chunk)))
    }

    /// Dispatch a guest `out` of `value` at `port`, `size` bytes wide.
    ///
    /// Uses the same cycle widths, ordering, wrapping and fault rules as
    /// [`Bus::port_read`], slicing the value little-endian for each cycle.
    ///
    /// An undecoded port under [`UndecodedPortAccess::OpenBus`] drops the write
    /// (no listener drives the bus) and optionally records it on the bounded
    /// trace; under
    /// [`UndecodedPortAccess::Fault`] it raises [`BusError::UnmappedPort`].
    pub fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), BusError> {
        #[cfg(feature = "host-profile")]
        let _span = self.profile.span(device::profile::ProfilePhase::PortIo);
        let (device, chunk) = self.port_chunk(port, size);
        let cycle_value = if size > chunk {
            value & open_bus_value(chunk)
        } else {
            value
        };
        match device {
            Some(idx) => {
                self.sync_access(idx);
                Ok(self.devices[idx].port_write(port, chunk, cycle_value)?)
            }
            None => match self.undecoded_ports {
                UndecodedPortAccess::OpenBus => {
                    self.trace_open_bus(port, chunk, AccessDir::Write, cycle_value);
                    Ok(())
                }
                UndecodedPortAccess::Fault => Err(BusError::UnmappedPort { port }),
            },
        }?;
        if size == chunk {
            return Ok(());
        }
        self.port_write(
            port.wrapping_add(u16::from(chunk)),
            size - chunk,
            (value >> (8 * chunk)) & (u32::MAX >> (8 * (4 - (size - chunk)))),
        )
    }

    /// Record an open-bus port access on the bus's own trace, stamped with the
    /// current virtual time (§6) — never wall time, so goldens stay reproducible.
    fn trace_open_bus(&mut self, port: u16, size: u8, dir: AccessDir, value: u32) {
        self.open_bus_trace.push(TraceEvent::Port {
            at: self.now.get(),
            port,
            size,
            dir,
            value,
        });
    }

    /// Dispatch a guest MMIO read at `addr`, `size` bytes wide.
    ///
    /// The open-bus policy is deliberately **port-only**. An undecoded MMIO
    /// address is a memory-map concern, not a floating ISA data line: a BIOS
    /// machine covers its physical space with mapped RAM and ROM regions so
    /// there is no undecoded memory to float, and a stray MMIO trap that reaches
    /// the bus with no device is a real wiring error worth surfacing.
    ///
    /// Cross-window accesses run in ascending address order and combine results
    /// little-endian. A fault stops the access without undoing earlier chunks.
    /// An address span overflowing `u64` faults before reaching a device.
    /// Widths outside 1–8 bytes are passed unchanged to the first device for
    /// validation.
    pub fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, BusError> {
        #[cfg(feature = "host-profile")]
        let _span = self.profile.span(device::profile::ProfilePhase::Mmio);
        let (idx, chunk) = self.mmio_chunk(addr, size)?;
        self.sync_access(idx);
        let value = self.devices[idx].mmio_read(addr, chunk)?;
        if chunk == size {
            return Ok(value);
        }
        let shift = 8 * chunk;
        let rest = self.mmio_read(addr + u64::from(chunk), size - chunk)?;
        Ok((value & ((1u64 << shift) - 1)) | (rest << shift))
    }

    /// Dispatch a guest MMIO write of `value` at `addr`, `size` bytes wide.
    ///
    /// Uses the same ordering, fault and width rules as [`Bus::mmio_read`],
    /// slicing the value little-endian at each decode-window boundary.
    pub fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), BusError> {
        #[cfg(feature = "host-profile")]
        let _span = self.profile.span(device::profile::ProfilePhase::Mmio);
        let (idx, chunk) = self.mmio_chunk(addr, size)?;
        self.sync_access(idx);
        if chunk == size {
            return Ok(self.devices[idx].mmio_write(addr, size, value)?);
        }
        let shift = 8 * chunk;
        self.devices[idx].mmio_write(addr, chunk, value & ((1u64 << shift) - 1))?;
        let remaining = size - chunk;
        let rest = (value >> shift) & ((1u64 << (8 * remaining)) - 1);
        self.mmio_write(addr + u64::from(chunk), remaining, rest)
    }

    /// Advance every device to virtual time `now` (§6).
    pub fn tick_all(&mut self, now: VClock) {
        #[cfg(feature = "host-profile")]
        self.profile.event(device::profile::ProfileEvent::TickDue);
        self.now.set(now);
        self.deadlines.fill(None);
        for dev in &mut self.devices {
            #[cfg(feature = "host-profile")]
            self.profile
                .event(device::profile::ProfileEvent::DeviceTick);
            dev.tick(now);
        }
    }

    /// Tick due devices in bus order at any virtual-tick deadline (§6).
    ///
    /// Scheduled deadlines are refreshed after ticks; EveryPitClock answers
    /// stay cached until access or mutation invalidates them. Returning true
    /// requests IRQ sampling, including pin changes caused by accesses since
    /// the previous visit. Dormant devices receive no clock visits; the scheduler
    /// gives legacy EveryPitClock devices one visit per elapsed PIT input edge.
    ///
    /// `now` must not be earlier than the bus time. A same-instant call visits
    /// only due At deadlines; no PIT edge has elapsed for legacy devices.
    pub fn tick_due(&mut self, now: VClock) -> bool {
        #[cfg(feature = "host-profile")]
        self.profile.event(device::profile::ProfileEvent::TickDue);
        let mut changed = std::mem::take(&mut self.pins_dirty);
        let previous = self.now.get();
        debug_assert!(now >= previous, "bus time must not move backwards");
        let pit_edge = now > previous && device::clock::PIT.edges_between(previous, now) > 0;
        for (dev, slot) in self.devices.iter_mut().zip(&mut self.deadlines) {
            let event = *slot.get_or_insert_with(|| dev.next_event(self.now.get()));
            let due = match event {
                NextEvent::EveryPitClock => pit_edge,
                NextEvent::At(at) => at <= now,
                NextEvent::Dormant => false,
            };
            if due {
                #[cfg(feature = "host-profile")]
                self.profile
                    .event(device::profile::ProfileEvent::DeviceTick);
                dev.tick(now);
                if event != NextEvent::EveryPitClock {
                    *slot = Some(dev.next_event(now));
                }
                changed = true;
            }
        }
        self.now.set(now);
        changed
    }

    /// Earliest autonomous event, reusing the same per-device answers as
    /// [`Bus::tick_due`]. Access, mutation and synchronization invalidate those
    /// answers; querying a CPU budget does not. Whole-clock arithmetic is widened
    /// so an exhausted clock still returns EveryPitClock for the caller to reject.
    pub fn next_event(&mut self) -> NextEvent {
        #[cfg(feature = "host-profile")]
        self.profile
            .event(device::profile::ProfileEvent::DeadlineQuery);
        let now = self.now.get();
        self.devices
            .iter()
            .zip(&mut self.deadlines)
            .map(|(dev, slot)| *slot.get_or_insert_with(|| dev.next_event(now)))
            .min_by_key(|event| match event {
                NextEvent::EveryPitClock => device::clock::PIT
                    .next_edge_after(now)
                    .map_or(u128::from(u64::MAX) + 1, |at| u128::from(at.0)),
                NextEvent::At(at) => u128::from(at.0),
                NextEvent::Dormant => u128::MAX,
            })
            .unwrap_or(NextEvent::Dormant)
    }

    /// Forget cached deadlines after an access that may affect coupled devices.
    /// This only invalidates scheduling answers; it does not synchronize hardware.
    pub fn invalidate_deadlines(&mut self) {
        self.deadlines.fill(None);
    }

    fn sync_access(&mut self, index: usize) {
        self.sync_device(index);
        for offset in 0..self.access_dependents[index].len() {
            self.sync_device(self.access_dependents[index][offset]);
        }
    }

    fn sync_device(&mut self, index: usize) {
        let dev = &mut self.devices[index];
        let event = self.deadlines[index]
            .take()
            .unwrap_or_else(|| dev.next_event(self.now.get()));
        if event != NextEvent::EveryPitClock {
            #[cfg(feature = "host-profile")]
            self.profile
                .event(device::profile::ProfileEvent::DeviceSync);
            dev.sync(self.now.get());
        }
        self.pins_dirty = true;
    }

    /// Bring every scheduled device's lazy state to the current bus clock.
    ///
    /// Call before reading device snapshots through [`Bus::device`]. Devices
    /// using [`NextEvent::EveryPitClock`] are already current and receive no
    /// extra tick. Synchronization invalidates deadlines and requests pin
    /// sampling at the next clock, just like a device access.
    pub fn settle(&mut self) {
        for index in 0..self.devices.len() {
            self.sync_device(index);
        }
    }

    /// The current IRQ-line level of the device at `index`, or `None` if the
    /// index is out of range (§5.1).
    ///
    /// This is the non-consuming pin level (`irq_pending`). The machine forwards
    /// both transitions; the PIC detects edges and withdraws pending requests
    /// when an input falls (Intel 8259A datasheet, Edge and Level Triggered Modes).
    /// Wiring uses the configured device index, without a concrete device type.
    pub fn device_irq_line(&self, index: usize) -> Option<bool> {
        self.device_irq_output(index, 0)
    }

    /// Raw level of a device-local interrupt output pin (§5.1).
    /// Board wiring selects the pin; the machine forwards level changes
    /// from every output. Devices without that pin return a low level.
    pub fn device_irq_output(&self, index: usize, output: usize) -> Option<bool> {
        self.devices.get(index).map(|d| d.irq_output(output))
    }

    /// Borrow a device by index (for tests and CLI serial draining).
    ///
    /// State reflects the device's last synchronization, which may precede the
    /// bus clock. Call [`Bus::settle`] first when current state is needed, such
    /// as when saving a snapshot.
    pub fn device(&self, index: usize) -> Option<&dyn Device> {
        self.devices.get(index).map(|b| b.as_ref())
    }

    /// Mutably borrow a device by index after synchronizing it to the bus clock.
    pub fn device_mut(&mut self, index: usize) -> Option<&mut Box<dyn Device>> {
        if index < self.devices.len() {
            self.sync_access(index);
        }
        self.devices.get_mut(index)
    }
}

/// The value an open-bus read of `size` bytes floats to: all-ones in every
/// byte, which a stock BIOS reads as "nothing here" (0xFF/0xFFFF/0xFFFFFFFF).
fn open_bus_value(size: u8) -> u32 {
    match size {
        1 => 0x0000_00FF,
        2 => 0x0000_FFFF,
        _ => 0xFFFF_FFFF,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type MmioCalls = std::rc::Rc<std::cell::RefCell<Vec<TraceEvent>>>;

    struct MmioRegisters {
        base: u64,
        bytes: Vec<u8>,
        calls: MmioCalls,
        read_junk: u64,
    }

    impl Device for MmioRegisters {
        fn name(&self) -> &'static str {
            "mmio-registers"
        }
        fn port_read(&mut self, port: u16, _: u8) -> Result<u32, DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "port",
                offset: u64::from(port),
            })
        }
        fn port_write(&mut self, port: u16, _: u8, _: u32) -> Result<(), DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "port",
                offset: u64::from(port),
            })
        }
        fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, DeviceError> {
            if !(1..=8).contains(&size) {
                return Err(DeviceError::UnmappedRegister {
                    kind: "mmio",
                    offset: addr,
                });
            }
            let mut bytes = [0; 8];
            for (i, byte) in bytes.iter_mut().take(usize::from(size)).enumerate() {
                *byte = self
                    .bytes
                    .get((addr - self.base) as usize + i)
                    .copied()
                    .unwrap_or(0);
            }
            let value = u64::from_le_bytes(bytes) | self.read_junk;
            self.calls
                .borrow_mut()
                .push(mmio_event(addr, size, AccessDir::Read, value));
            Ok(value)
        }
        fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError> {
            if !(1..=8).contains(&size) {
                return Err(DeviceError::UnmappedRegister {
                    kind: "mmio",
                    offset: addr,
                });
            }
            for (i, byte) in value
                .to_le_bytes()
                .iter()
                .take(usize::from(size))
                .enumerate()
            {
                if let Some(slot) = self.bytes.get_mut((addr - self.base) as usize + i) {
                    *slot = *byte;
                }
            }
            self.calls
                .borrow_mut()
                .push(mmio_event(addr, size, AccessDir::Write, value));
            Ok(())
        }
        fn tick(&mut self, _: VClock) {}
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {
            self.bytes.fill(0);
        }
        fn save(&self) -> Vec<u8> {
            self.bytes.clone()
        }
        fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError> {
            self.bytes.copy_from_slice(snapshot);
            Ok(())
        }
    }

    fn mmio_event(addr: u64, size: u8, dir: AccessDir, value: u64) -> TraceEvent {
        TraceEvent::Mmio {
            at: VClock::ZERO,
            addr,
            size,
            dir,
            value,
        }
    }

    fn mmio_bus(windows: &[(u64, &[u8])]) -> (Bus, MmioCalls) {
        mmio_bus_with_read_junk(windows, &[])
    }

    fn mmio_bus_with_read_junk(windows: &[(u64, &[u8])], read_junk: &[u64]) -> (Bus, MmioCalls) {
        let calls = MmioCalls::default();
        let mut devices: Vec<Box<dyn Device>> = Vec::new();
        let mut map = DecodeMap::default();
        for (base, bytes) in windows {
            map.mmio.push(MmioRange {
                addrs: *base..=(*base + (bytes.len() as u64 - 1)),
                device: devices.len(),
            });
            devices.push(Box::new(MmioRegisters {
                base: *base,
                bytes: bytes.to_vec(),
                calls: calls.clone(),
                read_junk: read_junk.get(devices.len()).copied().unwrap_or(0),
            }));
        }
        (Bus::new(devices, map), calls)
    }

    #[test]
    fn mmio_straddling_word_write_reaches_both_devices() {
        let (mut bus, calls) = mmio_bus(&[(0xB7FFE, &[0, 0]), (0xB8000, &[0, 0])]);
        bus.mmio_write(0xB7FFF, 2, 0x1234).unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                mmio_event(0xB7FFF, 1, AccessDir::Write, 0x34),
                mmio_event(0xB8000, 1, AccessDir::Write, 0x12)
            ]
        );
        assert_eq!(bus.device(0).unwrap().save(), [0, 0x34]);
        assert_eq!(bus.device(1).unwrap().save(), [0x12, 0]);
    }

    #[test]
    fn mmio_straddling_word_read_composes_both_devices() {
        let (mut bus, calls) = mmio_bus(&[(0xB7FFE, &[0, 0x34]), (0xB8000, &[0x12, 0])]);
        assert_eq!(bus.mmio_read(0xB7FFF, 2).unwrap(), 0x1234);
        assert_eq!(
            *calls.borrow(),
            [
                mmio_event(0xB7FFF, 1, AccessDir::Read, 0x34),
                mmio_event(0xB8000, 1, AccessDir::Read, 0x12)
            ]
        );
    }

    #[test]
    fn mmio_eight_byte_access_spans_three_windows() {
        let (mut bus, calls) = mmio_bus(&[(0x100, &[0; 2]), (0x102, &[0; 3]), (0x105, &[0; 3])]);
        let value = 0x8877_6655_4433_2211;
        bus.mmio_write(0x100, 8, value).unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                mmio_event(0x100, 2, AccessDir::Write, 0x2211),
                mmio_event(0x102, 3, AccessDir::Write, 0x554433),
                mmio_event(0x105, 3, AccessDir::Write, 0x887766)
            ]
        );
        calls.borrow_mut().clear();
        assert_eq!(bus.mmio_read(0x100, 8).unwrap(), value);
        assert_eq!(
            *calls.borrow(),
            [
                mmio_event(0x100, 2, AccessDir::Read, 0x2211),
                mmio_event(0x102, 3, AccessDir::Read, 0x554433),
                mmio_event(0x105, 3, AccessDir::Read, 0x887766)
            ]
        );
    }

    #[test]
    fn mmio_read_masks_junk_bits_from_nonfinal_fragments() {
        let (mut bus, calls) = mmio_bus_with_read_junk(
            &[(0x100, &[0x12]), (0x101, &[0x34]), (0x102, &[0x56])],
            &[!0xFF, !0xFF],
        );
        assert_eq!(bus.mmio_read(0x100, 3).unwrap(), 0x563412);
        assert_eq!(
            *calls.borrow(),
            [
                mmio_event(0x100, 1, AccessDir::Read, !0xFF | 0x12),
                mmio_event(0x101, 1, AccessDir::Read, !0xFF | 0x34),
                mmio_event(0x102, 1, AccessDir::Read, 0x56),
            ]
        );
    }

    #[test]
    fn mmio_write_masks_every_fragment_including_the_last() {
        let (mut bus, calls) = mmio_bus(&[(0x100, &[0; 2]), (0x102, &[0]), (0x103, &[0])]);
        bus.mmio_write(0x100, 4, 0xFFFF_FFFF_4433_2211).unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                mmio_event(0x100, 2, AccessDir::Write, 0x2211),
                mmio_event(0x102, 1, AccessDir::Write, 0x33),
                mmio_event(0x103, 1, AccessDir::Write, 0x44),
            ]
        );
        assert_eq!(bus.mmio_read(0x100, 4).unwrap(), 0x4433_2211);
    }

    #[test]
    fn mmio_invalid_zero_width_reaches_device_validation_without_side_effects() {
        assert_invalid_mmio_width(0);
    }

    #[test]
    fn mmio_invalid_oversized_widths_are_not_split_into_valid_fragments() {
        for size in [9, 16, u8::MAX] {
            assert_invalid_mmio_width(size);
        }
    }

    fn assert_invalid_mmio_width(size: u8) {
        let (mut bus, calls) = mmio_bus(&[(0x100, &[0x12]), (0x101, &[0x34; 8])]);
        assert!(matches!(
            bus.mmio_read(0x100, size),
            Err(BusError::Device(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: 0x100
            }))
        ));
        assert!(matches!(
            bus.mmio_write(0x100, size, u64::MAX),
            Err(BusError::Device(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: 0x100
            }))
        ));
        assert!(calls.borrow().is_empty());
        assert_eq!(bus.device(0).unwrap().save(), [0x12]);
        assert_eq!(bus.device(1).unwrap().save(), [0x34; 8]);
    }

    #[test]
    fn mmio_overflow_faults_at_original_address_before_any_fragment() {
        let addr = u64::MAX - 1;
        let (mut bus, calls) = mmio_bus(&[(addr, &[0x12, 0x34]), (0, &[0x56, 0x78])]);
        assert!(matches!(
            bus.mmio_read(addr, 4),
            Err(BusError::UnmappedMmio { addr: fault }) if fault == addr
        ));
        assert!(matches!(
            bus.mmio_write(addr, 4, u64::MAX),
            Err(BusError::UnmappedMmio { addr: fault }) if fault == addr
        ));
        assert!(calls.borrow().is_empty());
        assert_eq!(bus.device(0).unwrap().save(), [0x12, 0x34]);
        assert_eq!(bus.device(1).unwrap().save(), [0x56, 0x78]);
    }

    #[test]
    fn mmio_within_one_window_keeps_original_size_and_value() {
        for size in [1, 2, 4, 8] {
            let (mut bus, calls) = mmio_bus(&[(0x100, &[0; 9])]);
            let value = 0x8877_6655_4433_2211;
            bus.mmio_write(0x101, size, value).unwrap();
            let read = bus.mmio_read(0x101, size).unwrap();
            let mut expected = [0; 8];
            expected[..usize::from(size)]
                .copy_from_slice(&value.to_le_bytes()[..usize::from(size)]);
            assert_eq!(read, u64::from_le_bytes(expected));
            assert_eq!(
                *calls.borrow(),
                [
                    mmio_event(0x101, size, AccessDir::Write, value),
                    mmio_event(0x101, size, AccessDir::Read, read)
                ]
            );
        }
    }

    #[test]
    fn mmio_partially_unmapped_stops_after_completed_prefix() {
        let (mut bus, calls) = mmio_bus(&[(0x100, &[0x11]), (0x102, &[0x33, 0x44])]);
        assert!(matches!(
            bus.mmio_write(0x100, 4, 0x8877_6655),
            Err(BusError::UnmappedMmio { addr: 0x101 })
        ));
        assert_eq!(
            *calls.borrow(),
            [mmio_event(0x100, 1, AccessDir::Write, 0x55)]
        );
        assert_eq!(bus.device(0).unwrap().save(), [0x55]);
        assert_eq!(bus.device(1).unwrap().save(), [0x33, 0x44]);
        calls.borrow_mut().clear();
        assert!(matches!(
            bus.mmio_read(0x100, 4),
            Err(BusError::UnmappedMmio { addr: 0x101 })
        ));
        assert_eq!(
            *calls.borrow(),
            [mmio_event(0x100, 1, AccessDir::Read, 0x55)]
        );
        calls.borrow_mut().clear();
        assert!(matches!(
            bus.mmio_read(0x101, 2),
            Err(BusError::UnmappedMmio { addr: 0x101 })
        ));
        assert!(matches!(
            bus.mmio_write(0x101, 2, 0xFFFF),
            Err(BusError::UnmappedMmio { addr: 0x101 })
        ));
        assert!(calls.borrow().is_empty());
    }

    type PortCalls = std::rc::Rc<std::cell::RefCell<Vec<TraceEvent>>>;

    struct ByteRegisters {
        base: u16,
        values: Vec<u8>,
        trace: device::TraceBuffer,
        calls: PortCalls,
        now: VClock,
        read_junk: u32,
    }

    impl Device for ByteRegisters {
        fn name(&self) -> &'static str {
            "byte-registers"
        }
        fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
            let offset = usize::from(port - self.base);
            let mut bytes = [0; 4];
            bytes[..usize::from(size)]
                .copy_from_slice(&self.values[offset..offset + usize::from(size)]);
            let value = u32::from_le_bytes(bytes) | self.read_junk;
            let event = TraceEvent::Port {
                at: self.now,
                port,
                size,
                dir: AccessDir::Read,
                value,
            };
            self.calls.borrow_mut().push(event.clone());
            self.trace.push(event);
            Ok(value)
        }
        fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
            let offset = usize::from(port - self.base);
            self.values[offset..offset + usize::from(size)]
                .copy_from_slice(&value.to_le_bytes()[..usize::from(size)]);
            let event = TraceEvent::Port {
                at: self.now,
                port,
                size,
                dir: AccessDir::Write,
                value,
            };
            self.calls.borrow_mut().push(event.clone());
            self.trace.push(event);
            Ok(())
        }
        fn mmio_read(&mut self, addr: u64, _size: u8) -> Result<u64, DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            })
        }
        fn mmio_write(&mut self, addr: u64, _size: u8, _value: u64) -> Result<(), DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            })
        }
        fn tick(&mut self, now: VClock) {
            self.now = now;
        }
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {
            self.values.fill(0);
            self.trace.clear();
        }
        fn save(&self) -> Vec<u8> {
            self.values.clone()
        }
        fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError> {
            self.values.copy_from_slice(snapshot);
            Ok(())
        }
        fn set_trace_policy(&mut self, policy: device::TracePolicy) {
            self.trace.set_policy(policy);
        }
        fn trace_dropped(&self) -> u64 {
            self.trace.dropped()
        }
        fn take_trace(&mut self) -> Vec<TraceEvent> {
            self.trace.take()
        }
    }

    type CallLog = std::rc::Rc<std::cell::RefCell<Vec<(&'static str, VClock)>>>;

    type CoupledCalls = std::rc::Rc<std::cell::RefCell<Vec<(usize, &'static str, VClock, u64)>>>;

    struct CoupledTimer {
        index: usize,
        now: VClock,
        period: std::rc::Rc<std::cell::Cell<u64>>,
        calls: CoupledCalls,
    }

    impl Device for CoupledTimer {
        fn name(&self) -> &'static str {
            "coupled-timer"
        }
        fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
            self.port_write(0, 1, 3)?;
            Ok(3)
        }
        fn port_write(&mut self, _: u16, _: u8, value: u32) -> Result<(), DeviceError> {
            self.calls
                .borrow_mut()
                .push((self.index, "access", self.now, self.period.get()));
            self.period.set(u64::from(value));
            Ok(())
        }
        fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
            self.port_read(0, 1).map(u64::from)
        }
        fn mmio_write(&mut self, _: u64, _: u8, value: u64) -> Result<(), DeviceError> {
            self.port_write(0, 1, value as u32)
        }
        fn tick(&mut self, now: VClock) {
            self.now = now;
            self.calls
                .borrow_mut()
                .push((self.index, "sync", now, self.period.get()));
        }
        fn next_event(&self, _: VClock) -> NextEvent {
            if self.index == 0 {
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
        fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
            Ok(())
        }
    }

    #[test]
    fn coupled_deadline_starts_at_access_time() {
        for access in 0..5 {
            for cached in [false, true] {
                let period = std::rc::Rc::new(std::cell::Cell::new(1000));
                let calls = CoupledCalls::default();
                let devices = (0..2)
                    .map(|index| {
                        Box::new(CoupledTimer {
                            index,
                            now: VClock::ZERO,
                            period: period.clone(),
                            calls: calls.clone(),
                        }) as Box<dyn Device>
                    })
                    .collect();
                let mut bus = Bus::new(
                    devices,
                    DecodeMap {
                        ports: vec![PortRange {
                            width: PortWidth::default(),
                            ports: 0x100..=0x100,
                            device: 0,
                        }],
                        mmio: vec![MmioRange {
                            addrs: 0x1000..=0x1000,
                            device: 0,
                        }],
                    },
                );
                bus.add_access_coupling(0, 1).unwrap();
                bus.tick_due(VClock(100));
                assert_eq!(bus.next_event(), NextEvent::At(VClock(1000)));
                if !cached {
                    bus.invalidate_deadlines();
                }
                match access {
                    0 => {
                        bus.port_read(0x100, 1).unwrap();
                    }
                    1 => bus.port_write(0x100, 1, 3).unwrap(),
                    2 => {
                        bus.mmio_read(0x1000, 1).unwrap();
                    }
                    3 => bus.mmio_write(0x1000, 1, 3).unwrap(),
                    _ => bus.device_mut(0).unwrap().port_write(0x100, 1, 3).unwrap(),
                }
                assert_eq!(bus.next_event(), NextEvent::At(VClock(103)));
                bus.invalidate_deadlines();
                assert_eq!(bus.next_event(), NextEvent::At(VClock(103)));
                assert_eq!(
                    calls.borrow().as_slice(),
                    [
                        (0, "sync", VClock(100), 1000),
                        (1, "sync", VClock(100), 1000),
                        (0, "access", VClock(100), 1000),
                    ]
                );
            }
        }
    }

    struct TimedDevice {
        now: VClock,
        event: NextEvent,
        calls: CallLog,
        before_access: Option<CallLog>,
    }

    #[test]
    fn access_couplings_are_directed_sorted_and_deduplicated() {
        let calls = CoupledCalls::default();
        let period = std::rc::Rc::new(std::cell::Cell::new(1000));
        let devices = (0..4)
            .map(|index| {
                Box::new(CoupledTimer {
                    index,
                    now: VClock::ZERO,
                    period: period.clone(),
                    calls: calls.clone(),
                }) as Box<dyn Device>
            })
            .collect();
        let mut bus = Bus::new(devices, DecodeMap::default());
        for (source, dependent) in [(0, 3), (0, 1), (0, 3), (3, 2), (1, 0)] {
            bus.add_access_coupling(source, dependent).unwrap();
        }
        bus.tick_due(VClock(100));
        bus.device_mut(0).unwrap().port_write(0, 1, 1000).unwrap();
        assert_eq!(
            calls.borrow().as_slice(),
            [
                (0, "sync", VClock(100), 1000),
                (1, "sync", VClock(100), 1000),
                (3, "sync", VClock(100), 1000),
                (0, "access", VClock(100), 1000),
            ]
        );
        calls.borrow_mut().clear();
        bus.device_mut(1).unwrap();
        assert_eq!(
            calls.borrow().as_slice(),
            [
                (1, "sync", VClock(100), 1000),
                (0, "sync", VClock(100), 1000),
            ]
        );
        calls.borrow_mut().clear();
        bus.device_mut(2).unwrap();
        assert_eq!(calls.borrow().as_slice(), [(2, "sync", VClock(100), 1000)]);
        calls.borrow_mut().clear();
        bus.settle();
        assert_eq!(
            calls.borrow().as_slice(),
            [
                (0, "sync", VClock(100), 1000),
                (1, "sync", VClock(100), 1000),
                (2, "sync", VClock(100), 1000),
                (3, "sync", VClock(100), 1000),
            ]
        );
    }

    #[test]
    fn access_skips_uncoupled_and_every_clock_devices() {
        for coupled in [false, true] {
            let calls = [CallLog::default(), CallLog::default(), CallLog::default()];
            let devices = [
                NextEvent::Dormant,
                NextEvent::EveryPitClock,
                NextEvent::At(VClock(1000)),
            ]
            .into_iter()
            .zip(&calls)
            .enumerate()
            .map(|(index, (event, device_calls))| {
                Box::new(TimedDevice {
                    now: VClock::ZERO,
                    event,
                    calls: device_calls.clone(),
                    before_access: (coupled && index == 0).then(|| calls[1].clone()),
                }) as Box<dyn Device>
            })
            .collect();
            let mut bus = Bus::new(
                devices,
                DecodeMap {
                    ports: vec![PortRange {
                        width: PortWidth::default(),
                        ports: 0x100..=0x100,
                        device: 0,
                    }],
                    mmio: vec![MmioRange {
                        addrs: 0x1000..=0x1000,
                        device: 0,
                    }],
                },
            );
            if coupled {
                bus.add_access_coupling(0, 1).unwrap();
            }
            bus.tick_due(VClock(1));
            for calls in &calls {
                calls.borrow_mut().clear();
            }
            bus.invalidate_deadlines();
            bus.port_read(0x100, 1).unwrap();
            bus.port_write(0x100, 1, 500).unwrap();
            bus.mmio_read(0x1000, 1).unwrap();
            bus.mmio_write(0x1000, 1, 0).unwrap();
            bus.device_mut(0).unwrap();
            assert_eq!(
                calls[0]
                    .borrow()
                    .iter()
                    .filter(|(call, _)| *call == "sync")
                    .count(),
                5
            );
            assert!(calls[1].borrow().iter().all(|(call, _)| *call == "query"));
            if coupled {
                assert!(!calls[1].borrow().is_empty());
            } else {
                assert!(calls[1].borrow().is_empty());
            }
            assert!(calls[2].borrow().is_empty());
        }
    }

    #[test]
    fn access_coupling_rejects_self_and_invalid_indices() {
        let (mut bus, _) = timed_bus(NextEvent::Dormant);
        for (device, dependent) in [(0, 0), (0, 1), (1, 0), (usize::MAX, 0), (0, usize::MAX)] {
            assert_eq!(
                bus.add_access_coupling(device, dependent),
                Err(AccessCouplingError { device, dependent })
            );
        }
        assert!(bus.access_dependents.iter().all(Vec::is_empty));
    }

    impl TimedDevice {
        fn record(&self, call: &'static str) {
            if matches!(
                call,
                "port_read" | "port_write" | "mmio_read" | "mmio_write"
            ) {
                if let Some(calls) = &self.before_access {
                    assert!(
                        !calls.borrow().is_empty(),
                        "dependent must be visited before access"
                    );
                }
            }
            self.calls.borrow_mut().push((call, self.now));
        }
    }

    impl Device for TimedDevice {
        fn name(&self) -> &'static str {
            "timed"
        }
        fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
            self.record("port_read");
            self.event = NextEvent::Dormant;
            Ok(self.now.0 as u32)
        }
        fn port_write(&mut self, _: u16, _: u8, value: u32) -> Result<(), DeviceError> {
            self.record("port_write");
            self.event = NextEvent::At(VClock(u64::from(value)));
            Ok(())
        }
        fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
            self.record("mmio_read");
            Ok(self.now.0)
        }
        fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), DeviceError> {
            self.record("mmio_write");
            Ok(())
        }
        fn tick(&mut self, now: VClock) {
            self.now = now;
            self.record("tick");
            if self.event != NextEvent::EveryPitClock {
                self.event = NextEvent::Dormant;
            }
        }
        fn sync(&mut self, now: VClock) {
            self.now = now;
            self.record("sync");
        }
        fn next_event(&self, _: VClock) -> NextEvent {
            self.record("query");
            self.event
        }
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {
            self.event = NextEvent::At(VClock(self.now.0 + 1));
        }
        fn save(&self) -> Vec<u8> {
            self.now.0.to_le_bytes().to_vec()
        }
        fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
            self.reset();
            Ok(())
        }
    }

    fn timed_bus(event: NextEvent) -> (Bus, CallLog) {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let bus = Bus::new(
            vec![Box::new(TimedDevice {
                now: VClock::ZERO,
                event,
                calls: calls.clone(),
                before_access: None,
            })],
            DecodeMap {
                ports: vec![PortRange {
                    width: PortWidth::default(),
                    ports: 0x100..=0x101,
                    device: 0,
                }],
                mmio: vec![MmioRange {
                    addrs: 0x1000..=0x1001,
                    device: 0,
                }],
            },
        );
        (bus, calls)
    }

    #[test]
    fn mmio_read_syncs_each_straddling_fragment_before_access() {
        assert_straddling_mmio_sync(AccessDir::Read);
    }

    #[test]
    fn mmio_write_syncs_each_straddling_fragment_before_access() {
        assert_straddling_mmio_sync(AccessDir::Write);
    }

    fn assert_straddling_mmio_sync(dir: AccessDir) {
        let calls = [CallLog::default(), CallLog::default()];
        let devices = calls
            .iter()
            .map(|calls| {
                Box::new(TimedDevice {
                    now: VClock::ZERO,
                    event: NextEvent::Dormant,
                    calls: calls.clone(),
                    before_access: None,
                }) as Box<dyn Device>
            })
            .collect();
        let mut bus = Bus::new(
            devices,
            DecodeMap {
                ports: vec![],
                mmio: vec![
                    MmioRange {
                        addrs: 0x1000..=0x1000,
                        device: 0,
                    },
                    MmioRange {
                        addrs: 0x1001..=0x1001,
                        device: 1,
                    },
                ],
            },
        );
        bus.tick_due(VClock(12));
        bus.invalidate_deadlines();
        for calls in &calls {
            calls.borrow_mut().clear();
        }
        let access = match dir {
            AccessDir::Read => {
                assert_eq!(bus.mmio_read(0x1000, 2).unwrap(), 0x0C0C);
                "mmio_read"
            }
            AccessDir::Write => {
                bus.mmio_write(0x1000, 2, 0x1234).unwrap();
                "mmio_write"
            }
        };
        for calls in &calls {
            assert_eq!(
                calls.borrow().as_slice(),
                [
                    ("query", VClock::ZERO),
                    ("sync", VClock(12)),
                    (access, VClock(12)),
                ]
            );
        }
    }

    #[test]
    fn lazy_sync_precedes_each_port_byte_and_mmio_access() {
        for event in [NextEvent::Dormant, NextEvent::At(VClock(99))] {
            let (mut bus, calls) = timed_bus(event);
            for clock in 1..=7 {
                bus.tick_due(VClock(clock));
            }
            assert_eq!(calls.borrow().as_slice(), [("query", VClock::ZERO)]);
            assert_eq!(bus.port_read(0x100, 2).unwrap(), 0x0707);
            bus.port_write(0x100, 2, 0x6363).unwrap();
            assert_eq!(bus.mmio_read(0x1000, 1).unwrap(), 7);
            bus.mmio_write(0x1000, 1, 0).unwrap();
            let actual: Vec<_> = calls
                .borrow()
                .iter()
                .copied()
                .filter(|(call, _)| *call != "query")
                .collect();
            assert_eq!(
                actual,
                [
                    ("sync", VClock(7)),
                    ("port_read", VClock(7)),
                    ("sync", VClock(7)),
                    ("port_read", VClock(7)),
                    ("sync", VClock(7)),
                    ("port_write", VClock(7)),
                    ("sync", VClock(7)),
                    ("port_write", VClock(7)),
                    ("sync", VClock(7)),
                    ("mmio_read", VClock(7)),
                    ("sync", VClock(7)),
                    ("mmio_write", VClock(7)),
                ]
            );
        }
    }

    #[test]
    fn access_and_mutable_borrows_invalidate_cached_deadlines() {
        let (mut bus, calls) = timed_bus(NextEvent::Dormant);
        bus.tick_due(VClock(1));
        bus.port_write(0x100, 1, 4).unwrap();
        bus.tick_due(VClock(2));
        bus.tick_due(VClock(3));
        bus.tick_due(VClock(4));
        bus.port_write(0x100, 1, 9).unwrap();
        bus.tick_due(VClock(5));
        bus.port_read(0x100, 1).unwrap(); // Cancels the cached event at 9.
        for clock in 6..=9 {
            bus.tick_due(VClock(clock));
        }
        bus.device_mut(0).unwrap().restore(&[]).unwrap();
        assert_eq!(calls.borrow().last(), Some(&("sync", VClock(9))));
        bus.tick_due(VClock(10));
        let ticks: Vec<_> = calls
            .borrow()
            .iter()
            .copied()
            .filter(|(call, _)| *call == "tick")
            .collect();
        assert_eq!(ticks, [("tick", VClock(4)), ("tick", VClock(10))]);
    }

    #[test]
    fn budget_queries_share_dispatch_cache_and_refresh_after_mutation() {
        let (mut bus, calls) = timed_bus(NextEvent::Dormant);
        let clock = bus.observation_clock();
        let queries = || {
            calls
                .borrow()
                .iter()
                .filter(|(call, _)| *call == "query")
                .count()
        };
        for _ in 0..3 {
            assert_eq!(bus.next_event(), NextEvent::Dormant);
        }
        assert_eq!(queries(), 1);
        bus.tick_due(VClock(7));
        assert_eq!(clock(), VClock(7));
        assert_eq!(bus.device(0).unwrap().save(), 0u64.to_le_bytes());
        assert_eq!(queries(), 1);
        bus.port_write(0x100, 1, 12).unwrap();
        for _ in 0..3 {
            assert_eq!(bus.next_event(), NextEvent::At(VClock(12)));
        }
        assert_eq!(queries(), 2);
        bus.tick_due(VClock(11));
        assert_eq!(bus.next_event(), NextEvent::At(VClock(12)));
        assert_eq!(queries(), 2);
        bus.tick_due(VClock(12));
        assert_eq!(bus.next_event(), NextEvent::Dormant);
        assert_eq!(queries(), 3);
        bus.device_mut(0).unwrap().reset();
        assert_eq!(bus.next_event(), NextEvent::At(VClock(13)));
        assert_eq!(queries(), 4);
        bus.tick_all(VClock(13));
        assert_eq!(bus.next_event(), NextEvent::Dormant);
        assert_eq!(queries(), 5);
        bus.settle();
        assert_eq!(bus.next_event(), NextEvent::Dormant);
        assert_eq!(queries(), 6);
    }

    #[test]
    fn minimum_deadline_preserves_every_clock_dormancy_and_invalid_deadlines() {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let devices = [
            NextEvent::Dormant,
            NextEvent::At(VClock(17)),
            NextEvent::EveryPitClock,
        ]
        .into_iter()
        .map(|event| {
            Box::new(TimedDevice {
                now: VClock::ZERO,
                event,
                calls: calls.clone(),
                before_access: None,
            }) as Box<dyn Device>
        })
        .collect();
        let mut bus = Bus::new(devices, DecodeMap::default());
        assert_eq!(bus.next_event(), NextEvent::EveryPitClock);
        bus.device_mut(0).unwrap().port_write(0, 1, 0).unwrap();
        assert_eq!(bus.next_event(), NextEvent::At(VClock::ZERO));
        bus.device_mut(0).unwrap().port_read(0, 1).unwrap();
        bus.device_mut(2).unwrap().port_read(0, 1).unwrap();
        assert_eq!(bus.next_event(), NextEvent::At(VClock(17)));
        bus.device_mut(1).unwrap().port_read(0, 1).unwrap();
        assert_eq!(bus.next_event(), NextEvent::Dormant);
        assert_eq!(
            Bus::new(Vec::new(), DecodeMap::default()).next_event(),
            NextEvent::Dormant
        );
        let (mut bus, _) = timed_bus(NextEvent::EveryPitClock);
        bus.tick_all(VClock(u64::MAX));
        assert_eq!(bus.next_event(), NextEvent::EveryPitClock);
    }

    #[test]
    fn every_clock_answer_is_cached_until_access_or_mutation() {
        let (mut bus, calls) = timed_bus(NextEvent::EveryPitClock);
        for clock in 1..=10 {
            bus.tick_due(VClock(clock));
        }
        let queries = || {
            calls
                .borrow()
                .iter()
                .filter(|(call, _)| *call == "query")
                .count()
        };
        assert_eq!(queries(), 1);
        bus.mmio_read(0x1000, 1).unwrap();
        assert_eq!(queries(), 1);
        bus.tick_due(VClock(11));
        assert_eq!(queries(), 2);
        bus.device_mut(0).unwrap();
        assert_eq!(queries(), 2);
        bus.tick_due(VClock(12));
        assert_eq!(queries(), 3);
        bus.port_write(0x100, 1, 14).unwrap();
        bus.tick_due(VClock(13));
        assert_eq!(queries(), 4);
        bus.tick_due(VClock(14));
        assert_eq!(queries(), 5);
    }

    #[test]
    fn settle_updates_all_lazy_snapshots_without_extra_every_clock_ticks() {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let devices: Vec<Box<dyn Device>> = [
            NextEvent::Dormant,
            NextEvent::At(VClock(100)),
            NextEvent::EveryPitClock,
        ]
        .into_iter()
        .map(|event| {
            Box::new(TimedDevice {
                now: VClock::ZERO,
                event,
                calls: calls.clone(),
                before_access: None,
            }) as Box<dyn Device>
        })
        .collect();
        let mut bus = Bus::new(devices, DecodeMap::default());
        for clock in 1..=50 {
            bus.tick_due(VClock(clock));
        }
        assert_eq!(bus.device(0).unwrap().save(), 0u64.to_le_bytes());
        assert_eq!(bus.device(1).unwrap().save(), 0u64.to_le_bytes());
        calls.borrow_mut().clear();
        bus.settle();
        for index in 0..3 {
            assert_eq!(bus.device(index).unwrap().save(), 50u64.to_le_bytes());
        }
        assert_eq!(
            calls.borrow().as_slice(),
            [("sync", VClock(50)), ("sync", VClock(50))]
        );
    }

    #[test]
    fn every_clock_devices_receive_no_access_sync_or_duplicate_ticks() {
        let (mut bus, calls) = timed_bus(NextEvent::EveryPitClock);
        for clock in 1..=3 {
            bus.tick_due(VClock(clock));
            bus.mmio_read(0x1000, 1).unwrap();
            bus.mmio_write(0x1000, 1, 0).unwrap();
        }
        let calls = calls.borrow();
        assert!(!calls.iter().any(|(call, _)| *call == "sync"));
        assert_eq!(calls.iter().filter(|(call, _)| *call == "tick").count(), 3);
    }

    fn register_bus(registers: &[(u16, &[u8])]) -> Bus {
        let mut devices: Vec<Box<dyn Device>> = Vec::new();
        let mut map = DecodeMap::default();
        for &(base, values) in registers {
            map.ports.push(PortRange {
                width: PortWidth::default(),
                ports: base..=base + (values.len() - 1) as u16,
                device: devices.len(),
            });
            devices.push(Box::new(ByteRegisters {
                base,
                values: values.to_vec(),
                trace: device::TraceBuffer::default(),
                calls: PortCalls::default(),
                now: VClock::ZERO,
                read_junk: 0,
            }));
        }
        let mut bus = Bus::new(devices, map);
        bus.set_trace_policy(device::TracePolicy::Ring { capacity: 100 });
        bus
    }

    fn isa_bus(windows: &[(u16, u16, PortWidth, &[u8])]) -> (Bus, PortCalls) {
        let calls = PortCalls::default();
        let mut devices: Vec<Box<dyn Device>> = Vec::new();
        let mut map = DecodeMap::default();
        for &(start, end, width, values) in windows {
            map.ports.push(PortRange {
                ports: start..=end,
                device: devices.len(),
                width,
            });
            devices.push(Box::new(ByteRegisters {
                base: start,
                values: values.to_vec(),
                trace: device::TraceBuffer::default(),
                calls: calls.clone(),
                now: VClock::ZERO,
                read_junk: 0,
            }));
        }
        (Bus::new(devices, map), calls)
    }

    fn isa_exchange(
        bus: &mut Bus,
        calls: &PortCalls,
        port: u16,
        size: u8,
        value: u32,
        expected: &[(u16, u8, u32)],
    ) {
        bus.port_write(port, size, value).unwrap();
        assert_eq!(bus.port_read(port, size).unwrap(), value);
        let expected: Vec<_> = [AccessDir::Write, AccessDir::Read]
            .into_iter()
            .flat_map(|dir| {
                expected
                    .iter()
                    .map(move |&(port, size, value)| TraceEvent::Port {
                        at: VClock::ZERO,
                        port,
                        size,
                        dir,
                        value,
                    })
            })
            .collect();
        assert_eq!(*calls.borrow(), expected);
    }

    #[test]
    fn isa_word_window_receives_one_word_cycle() {
        let (mut bus, calls) = isa_bus(&[
            (0x1F0, 0x1F0, PortWidth::Word, &[0, 0]),
            (0x1F1, 0x1F1, PortWidth::Byte, &[0xEE]),
        ]);
        isa_exchange(&mut bus, &calls, 0x1F0, 2, 0x3412, &[(0x1F0, 2, 0x3412)]);
        assert_eq!(bus.device(1).unwrap().save(), [0xEE]);
    }

    #[test]
    fn isa_byte_window_receives_two_byte_cycles() {
        let (mut bus, calls) = isa_bus(&[(0x100, 0x101, PortWidth::Byte, &[0, 0])]);
        isa_exchange(
            &mut bus,
            &calls,
            0x100,
            2,
            0x3412,
            &[(0x100, 1, 0x12), (0x101, 1, 0x34)],
        );
    }

    #[test]
    fn isa_split_read_masks_unused_bits_in_every_byte() {
        let calls = PortCalls::default();
        let mut bus = Bus::new(
            vec![Box::new(ByteRegisters {
                base: 0x100,
                values: vec![0x12, 0x12],
                trace: device::TraceBuffer::default(),
                calls: calls.clone(),
                now: VClock::ZERO,
                read_junk: 0xDEAD_0000,
            })],
            DecodeMap {
                ports: vec![PortRange {
                    ports: 0x100..=0x101,
                    device: 0,
                    width: PortWidth::Byte,
                }],
                mmio: Vec::new(),
            },
        );
        assert_eq!(bus.port_read(0x100, 2).unwrap(), 0x1212);
        assert_eq!(
            *calls.borrow(),
            [
                port_event(0x100, AccessDir::Read, 0xDEAD_0012),
                port_event(0x101, AccessDir::Read, 0xDEAD_0012),
            ]
        );
    }

    #[test]
    fn isa_split_word_write_masks_each_byte_cycle_value() {
        let (mut bus, calls) = isa_bus(&[(0x100, 0x101, PortWidth::Byte, &[0, 0])]);
        bus.port_write(0x100, 2, 0xDEAD_1234).unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                port_event(0x100, AccessDir::Write, 0x34),
                port_event(0x101, AccessDir::Write, 0x12),
            ]
        );
        assert_eq!(bus.device(0).unwrap().save(), [0x34, 0x12]);
    }

    #[test]
    fn isa_odd_word_uses_two_byte_cycles() {
        let (mut bus, calls) = isa_bus(&[(0x100, 0x103, PortWidth::Word, &[0; 4])]);
        isa_exchange(
            &mut bus,
            &calls,
            0x101,
            2,
            0x3412,
            &[(0x101, 1, 0x12), (0x102, 1, 0x34)],
        );
    }

    #[test]
    fn isa_dword_uses_two_word_cycles() {
        let (mut bus, calls) = isa_bus(&[(0x100, 0x103, PortWidth::Word, &[0; 4])]);
        isa_exchange(
            &mut bus,
            &calls,
            0x100,
            4,
            0x7856_3412,
            &[(0x100, 2, 0x3412), (0x102, 2, 0x7856)],
        );
    }

    #[test]
    fn isa_mixed_windows_redecode_after_each_cycle() {
        let (mut bus, calls) = isa_bus(&[
            (0x100, 0x100, PortWidth::Word, &[0, 0]),
            (0x102, 0x103, PortWidth::Byte, &[0, 0]),
        ]);
        isa_exchange(
            &mut bus,
            &calls,
            0x100,
            4,
            0x7856_3412,
            &[(0x100, 2, 0x3412), (0x102, 1, 0x56), (0x103, 1, 0x78)],
        );
        let (mut bus, calls) = isa_bus(&[
            (0x100, 0x101, PortWidth::Byte, &[0, 0]),
            (0x102, 0x102, PortWidth::Word, &[0, 0]),
        ]);
        isa_exchange(
            &mut bus,
            &calls,
            0x100,
            4,
            0x7856_3412,
            &[(0x100, 1, 0x12), (0x101, 1, 0x34), (0x102, 2, 0x7856)],
        );
    }

    #[test]
    fn isa_overlap_uses_first_matching_windows_width() {
        let (mut bus, calls) = isa_bus(&[
            (0x100, 0x100, PortWidth::Byte, &[0]),
            (0x100, 0x103, PortWidth::Word, &[0; 4]),
        ]);
        isa_exchange(
            &mut bus,
            &calls,
            0x100,
            4,
            0x7856_3412,
            &[(0x100, 1, 0x12), (0x101, 1, 0x34), (0x102, 2, 0x7856)],
        );
        assert_eq!(bus.device(1).unwrap().save(), [0, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn isa_dword_wraps_between_word_cycles() {
        let (mut bus, calls) = isa_bus(&[
            (0xFFFE, 0xFFFE, PortWidth::Word, &[0, 0]),
            (0, 0, PortWidth::Word, &[0, 0]),
        ]);
        isa_exchange(
            &mut bus,
            &calls,
            0xFFFE,
            4,
            0x7856_3412,
            &[(0xFFFE, 2, 0x3412), (0, 2, 0x7856)],
        );
    }

    #[test]
    fn isa_fault_keeps_completed_word_cycle() {
        let (bus, calls) = isa_bus(&[(0x100, 0x100, PortWidth::Word, &[0x12, 0x34])]);
        let mut bus = bus.with_undecoded_ports(UndecodedPortAccess::Fault);
        assert!(matches!(
            bus.port_read(0x100, 4),
            Err(BusError::UnmappedPort { port: 0x102 })
        ));
        assert!(matches!(
            bus.port_write(0x100, 4, 0x7856_3412),
            Err(BusError::UnmappedPort { port: 0x102 })
        ));
        assert_eq!(
            *calls.borrow(),
            [
                TraceEvent::Port {
                    at: VClock::ZERO,
                    port: 0x100,
                    size: 2,
                    dir: AccessDir::Read,
                    value: 0x3412
                },
                TraceEvent::Port {
                    at: VClock::ZERO,
                    port: 0x100,
                    size: 2,
                    dir: AccessDir::Write,
                    value: 0x3412
                },
            ]
        );
    }

    #[test]
    fn isa_trace_records_cycle_width_value_and_virtual_time() {
        let (mut bus, calls) = isa_bus(&[(0x100, 0x100, PortWidth::Word, &[0, 0])]);
        bus.port_write(0x100, 2, 0x3412).unwrap();
        assert!(bus.device_mut(0).unwrap().take_trace().is_empty());
        bus.set_trace_policy(device::TracePolicy::Ring { capacity: 4 });
        bus.tick_all(VClock(7));
        assert_eq!(bus.port_read(0x100, 4).unwrap(), 0xFFFF_3412);
        bus.port_write(0x100, 4, 0x7856_ABCD).unwrap();
        assert_eq!(
            bus.device_mut(0).unwrap().take_trace(),
            [
                TraceEvent::Port {
                    at: VClock(7),
                    port: 0x100,
                    size: 2,
                    dir: AccessDir::Read,
                    value: 0x3412
                },
                TraceEvent::Port {
                    at: VClock(7),
                    port: 0x100,
                    size: 2,
                    dir: AccessDir::Write,
                    value: 0xABCD
                },
            ]
        );
        assert_eq!(
            bus.take_open_bus_trace(),
            [
                TraceEvent::Port {
                    at: VClock(7),
                    port: 0x102,
                    size: 1,
                    dir: AccessDir::Read,
                    value: 0xFF,
                },
                TraceEvent::Port {
                    at: VClock(7),
                    port: 0x103,
                    size: 1,
                    dir: AccessDir::Read,
                    value: 0xFF,
                },
                TraceEvent::Port {
                    at: VClock(7),
                    port: 0x102,
                    size: 1,
                    dir: AccessDir::Write,
                    value: 0x56
                },
                TraceEvent::Port {
                    at: VClock(7),
                    port: 0x103,
                    size: 1,
                    dir: AccessDir::Write,
                    value: 0x78
                },
            ]
        );
        assert_eq!(bus.open_bus_trace_dropped(), 0);
        assert_eq!(calls.borrow().len(), 3);
    }

    fn port_event(port: u16, dir: AccessDir, value: u32) -> TraceEvent {
        TraceEvent::Port {
            at: VClock::ZERO,
            port,
            size: 1,
            dir,
            value,
        }
    }

    #[test]
    fn word_out_sets_crtc_index_then_data() {
        let mut bus = register_bus(&[(0x3D4, &[0, 0])]);
        bus.port_write(0x3D4, 2, 0x120E).unwrap();
        assert_eq!(bus.device(0).unwrap().save(), [0x0E, 0x12]);
        assert_eq!(
            bus.device_mut(0).unwrap().take_trace(),
            [
                port_event(0x3D4, AccessDir::Write, 0x0E),
                port_event(0x3D5, AccessDir::Write, 0x12),
            ]
        );
    }

    #[test]
    fn word_write_decodes_each_device() {
        let mut bus = register_bus(&[(0x100, &[0]), (0x101, &[0])]);
        bus.port_write(0x100, 2, 0xABCD).unwrap();
        assert_eq!(bus.device(0).unwrap().save(), [0xCD]);
        assert_eq!(bus.device(1).unwrap().save(), [0xAB]);
    }

    #[test]
    fn word_read_combines_devices_little_endian() {
        let mut bus = register_bus(&[(0x100, &[0x34]), (0x101, &[0x12])]);
        assert_eq!(bus.port_read(0x100, 2).unwrap(), 0x1234);
        for (index, port, value) in [(0, 0x100, 0x34), (1, 0x101, 0x12)] {
            assert_eq!(
                bus.device_mut(index).unwrap().take_trace(),
                [port_event(port, AccessDir::Read, value)]
            );
        }
    }

    #[test]
    fn word_access_applies_open_bus_to_undecoded_high_byte() {
        let mut bus = register_bus(&[(0x100, &[0x34])]);
        assert_eq!(bus.port_read(0x100, 2).unwrap(), 0xFF34);
        bus.port_write(0x100, 2, 0xABCD).unwrap();
        assert_eq!(bus.device(0).unwrap().save(), [0xCD]);
        assert_eq!(
            bus.take_open_bus_trace(),
            [
                port_event(0x101, AccessDir::Read, 0xFF),
                port_event(0x101, AccessDir::Write, 0xAB),
            ]
        );
    }

    #[test]
    fn word_access_faults_on_undecoded_high_byte_after_low_byte() {
        let mut bus =
            register_bus(&[(0x100, &[0x34])]).with_undecoded_ports(UndecodedPortAccess::Fault);
        assert!(matches!(
            bus.port_read(0x100, 2),
            Err(BusError::UnmappedPort { port: 0x101 })
        ));
        assert!(matches!(
            bus.port_write(0x100, 2, 0xABCD),
            Err(BusError::UnmappedPort { port: 0x101 })
        ));
        assert_eq!(bus.device(0).unwrap().save(), [0xCD]);
        assert_eq!(
            bus.device_mut(0).unwrap().take_trace(),
            [
                port_event(0x100, AccessDir::Read, 0x34),
                port_event(0x100, AccessDir::Write, 0xCD),
            ]
        );
        assert!(bus.take_open_bus_trace().is_empty());
    }

    #[test]
    fn dword_access_uses_four_byte_cycles_in_order() {
        let mut bus = register_bus(&[(0x100, &[0, 0, 0, 0])]);
        bus.port_write(0x100, 4, 0x7856_3412).unwrap();
        assert_eq!(bus.port_read(0x100, 4).unwrap(), 0x7856_3412);
        let mut expected = Vec::new();
        for dir in [AccessDir::Write, AccessDir::Read] {
            for (offset, value) in [0x12, 0x34, 0x56, 0x78].into_iter().enumerate() {
                expected.push(port_event(0x100 + offset as u16, dir, value));
            }
        }
        assert_eq!(bus.device_mut(0).unwrap().take_trace(), expected);
    }

    #[test]
    fn word_access_wraps_at_end_of_port_space() {
        let mut bus = register_bus(&[(0xFFFF, &[0]), (0, &[0])]);
        bus.port_write(0xFFFF, 2, 0x1234).unwrap();
        assert_eq!(bus.device(0).unwrap().save(), [0x34]);
        assert_eq!(bus.device(1).unwrap().save(), [0x12]);
        assert_eq!(bus.port_read(0xFFFF, 2).unwrap(), 0x1234);
    }

    #[test]
    fn byte_access_keeps_value_width_and_neighbor_unchanged() {
        let mut bus = register_bus(&[(0x3D4, &[0x0E, 0x12])]);
        assert_eq!(bus.port_read(0x3D5, 1).unwrap(), 0x12);
        bus.port_write(0x3D4, 1, 0x0F).unwrap();
        assert_eq!(bus.device(0).unwrap().save(), [0x0F, 0x12]);
        assert_eq!(
            bus.device_mut(0).unwrap().take_trace(),
            [
                port_event(0x3D5, AccessDir::Read, 0x12),
                port_event(0x3D4, AccessDir::Write, 0x0F),
            ]
        );
    }

    /// A device decoding a single port, so the decoded path can be exercised
    /// alongside the undecoded open-bus path.
    struct OnePortDevice;

    impl Device for OnePortDevice {
        fn name(&self) -> &'static str {
            "one-port"
        }
        fn port_read(&mut self, _port: u16, _size: u8) -> Result<u32, DeviceError> {
            Ok(0x42)
        }
        fn port_write(&mut self, _port: u16, _size: u8, _value: u32) -> Result<(), DeviceError> {
            Ok(())
        }
        fn mmio_read(&mut self, addr: u64, _size: u8) -> Result<u64, DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            })
        }
        fn mmio_write(&mut self, addr: u64, _size: u8, _value: u64) -> Result<(), DeviceError> {
            Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            })
        }
        fn tick(&mut self, _now: VClock) {}
        fn irq_pending(&self) -> bool {
            false
        }
        fn irq_output(&self, output: usize) -> bool {
            output == 1
        }
        fn reset(&mut self) {}
        fn save(&self) -> Vec<u8> {
            Vec::new()
        }
        fn restore(&mut self, _snapshot: &[u8]) -> Result<(), DeviceError> {
            Ok(())
        }
    }

    fn bus_decoding(port: u16) -> Bus {
        let devices: Vec<Box<dyn Device>> = vec![Box::new(OnePortDevice)];
        let map = DecodeMap {
            ports: vec![PortRange {
                width: PortWidth::default(),
                ports: port..=port,
                device: 0,
            }],
            mmio: Vec::new(),
        };
        Bus::new(devices, map)
    }

    #[test]
    fn device_irq_outputs_select_independent_pins_and_preserve_the_primary_query() {
        let bus = bus_decoding(0x60);
        assert_eq!(bus.device_irq_line(0), Some(false));
        assert_eq!(bus.device_irq_output(0, 0), Some(false));
        assert_eq!(bus.device_irq_output(0, 1), Some(true));
        assert_eq!(bus.device_irq_output(0, 2), Some(false));
        assert_eq!(bus.device_irq_output(1, 1), None);
    }

    #[test]
    fn open_bus_trace_is_off_by_default_and_ring_retains_newest_events() {
        let mut bus = bus_decoding(0x60);
        for i in 0..5 {
            bus.port_write(0x80, 1, i).unwrap();
        }
        assert!(bus.take_open_bus_trace().is_empty());
        assert_eq!(bus.open_bus_trace_dropped(), 0);
        bus.set_trace_policy(device::TracePolicy::Ring { capacity: 2 });
        for i in 0..5 {
            bus.port_write(0x80, 1, i).unwrap();
        }
        assert_eq!(
            bus.take_open_bus_trace(),
            vec![
                port_event(0x80, AccessDir::Write, 3),
                port_event(0x80, AccessDir::Write, 4),
            ]
        );
        assert_eq!(bus.open_bus_trace_dropped(), 3);
        assert!(bus.take_open_bus_trace().is_empty());
        assert_eq!(bus.open_bus_trace_dropped(), 3);
        bus.set_trace_policy(device::TracePolicy::Off);
        assert_eq!(bus.open_bus_trace_dropped(), 0);
    }

    #[test]
    fn open_bus_is_the_default_policy() {
        assert_eq!(
            bus_decoding(0x60).undecoded_ports(),
            UndecodedPortAccess::OpenBus
        );
    }

    #[test]
    fn undecoded_read_floats_to_all_ones() {
        let mut bus = bus_decoding(0x60);
        assert_eq!(bus.port_read(0x80, 1).unwrap(), 0xFF);
        assert_eq!(bus.port_read(0x80, 2).unwrap(), 0xFFFF);
        assert_eq!(bus.port_read(0x80, 4).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn undecoded_write_is_a_no_op() {
        let mut bus = bus_decoding(0x60);
        // No device to fault; the write is absorbed.
        bus.port_write(0x3D8, 1, 0xAB).unwrap();
    }

    #[test]
    fn open_bus_accesses_are_traced_with_the_virtual_clock() {
        let mut bus = bus_decoding(0x60);
        bus.set_trace_policy(device::TracePolicy::Ring { capacity: 2 });
        bus.tick_all(VClock(7));
        bus.port_write(0x80, 1, 0xAB).unwrap();
        let _ = bus.port_read(0x3DA, 1).unwrap();

        let trace = bus.take_open_bus_trace();
        assert_eq!(
            trace,
            vec![
                TraceEvent::Port {
                    at: VClock(7),
                    port: 0x80,
                    size: 1,
                    dir: AccessDir::Write,
                    value: 0xAB,
                },
                TraceEvent::Port {
                    at: VClock(7),
                    port: 0x3DA,
                    size: 1,
                    dir: AccessDir::Read,
                    value: 0xFF,
                },
            ]
        );
        // The trace is drained on take.
        assert!(bus.take_open_bus_trace().is_empty());
    }

    #[test]
    fn decoded_ports_bypass_open_bus_and_are_not_traced() {
        let mut bus = bus_decoding(0x60);
        bus.set_trace_policy(device::TracePolicy::Ring { capacity: 2 });
        assert_eq!(bus.port_read(0x60, 1).unwrap(), 0x42);
        bus.port_write(0x60, 1, 0x99).unwrap();
        assert!(bus.take_open_bus_trace().is_empty());
    }

    #[test]
    fn strict_mode_still_faults_on_undecoded_ports() {
        let mut bus = bus_decoding(0x60).with_undecoded_ports(UndecodedPortAccess::Fault);
        bus.set_trace_policy(device::TracePolicy::Ring { capacity: 2 });
        assert!(matches!(
            bus.port_read(0x80, 1),
            Err(BusError::UnmappedPort { port: 0x80 })
        ));
        assert!(matches!(
            bus.port_write(0x80, 1, 0),
            Err(BusError::UnmappedPort { port: 0x80 })
        ));
        // A fault leaves nothing on the open-bus trace.
        assert!(bus.take_open_bus_trace().is_empty());
    }
    #[test]
    fn clock_domain_bus_orders_subedge_deadlines() {
        use device::clock::PIT;
        let x = VClock((PIT.ticks(1) - 1).max(1));
        let calls = CallLog::default();
        let devices: Vec<Box<dyn Device>> = [NextEvent::EveryPitClock, NextEvent::At(x)]
            .into_iter()
            .map(|event| {
                Box::new(TimedDevice {
                    now: VClock::ZERO,
                    event,
                    calls: calls.clone(),
                    before_access: None,
                }) as Box<dyn Device>
            })
            .collect();
        let mut bus = Bus::new(
            devices,
            DecodeMap {
                ports: Vec::new(),
                mmio: Vec::new(),
            },
        );
        assert!(x > VClock::ZERO);
        let expected = if x < PIT.edge_at(1) {
            NextEvent::At(x)
        } else {
            NextEvent::EveryPitClock
        };
        assert_eq!(bus.next_event(), expected);
    }

    #[test]
    fn clock_domain_bus_visits_legacy_only_on_pit_edges() {
        use device::clock::PIT;
        let (mut bus, calls) = timed_bus(NextEvent::EveryPitClock);
        let before = VClock(PIT.ticks(1) - 1);
        bus.tick_due(before);
        assert!(!calls.borrow().iter().any(|(call, _)| *call == "tick"));
        bus.tick_due(PIT.edge_at(1));
        assert_eq!(
            calls
                .borrow()
                .iter()
                .filter(|(call, _)| *call == "tick")
                .map(|(_, at)| *at)
                .collect::<Vec<_>>(),
            [PIT.edge_at(1)]
        );
    }

    #[test]
    fn clock_domain_bus_same_instant_visits_only_due_deadlines() {
        use device::clock::PIT;
        let now = PIT.edge_at(1);
        for event in [
            NextEvent::EveryPitClock,
            NextEvent::Dormant,
            NextEvent::At(now),
            NextEvent::At(PIT.edge_at(2)),
        ] {
            let (mut bus, calls) = timed_bus(event);
            assert_eq!(bus.next_event(), event);
            // Preserve a previously queried deadline while moving bus time.
            bus.now.set(now);
            calls.borrow_mut().clear();
            bus.tick_due(now);
            let ticks: Vec<_> = calls
                .borrow()
                .iter()
                .copied()
                .filter(|(call, _)| *call == "tick")
                .collect();
            let expected = if event == NextEvent::At(now) {
                vec![("tick", now)]
            } else {
                Vec::new()
            };
            assert_eq!(ticks, expected);
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "bus time must not move backwards")]
    fn clock_domain_bus_backwards_time_checks_debug_precondition() {
        use device::clock::PIT;
        let (mut bus, _) = timed_bus(NextEvent::EveryPitClock);
        bus.tick_due(PIT.edge_at(2));
        bus.tick_due(PIT.edge_at(1));
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn clock_domain_bus_backwards_time_skips_legacy_in_release() {
        use device::clock::PIT;
        let (mut bus, calls) = timed_bus(NextEvent::EveryPitClock);
        bus.tick_due(PIT.edge_at(2));
        calls.borrow_mut().clear();
        bus.tick_due(PIT.edge_at(1));
        assert!(!calls.borrow().iter().any(|(call, _)| *call == "tick"));
    }
}
