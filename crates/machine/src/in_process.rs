// SPDX-License-Identifier: AGPL-3.0-or-later
//! Serving accesses while the CPU runs (§3.1; `backlog/docs/design/scheduler.md`
//! §In-process device access).
//!
//! The ordinary loop returns from the CPU at every port access and every
//! access to a trapped window, then charges the steps before it, services
//! the access on the bus, charges the access's own step and settles. The CPU
//! seam can instead call [`InProcess`] synchronously for accesses inside
//! [`SERVED_RANGES`] ([`cpu::Cpu::run_steps_serving`]). The handler does
//! what the loop would do for that stop, in the same order and at the same
//! virtual time, with the same helpers, and lets the run continue. One step
//! is cheaper: after a completed MMIO access it asks only the devices the
//! access reached and their declared access dependents for new deadlines,
//! where the loop asks every device. That gives the loop's answers under the
//! access-coupling contract ([`bus::Bus::add_access_coupling`]): a device
//! whose next event an MMIO access can change through shared state is
//! declared an access dependent. Debug builds check it after every served
//! MMIO access. Port and failed accesses drop every answer, as the loop
//! does. It
//! declines ([`AccessResult::Fallback`]) before changing anything whenever it
//! cannot prove the result equal: an access whose own step would reach the
//! next device deadline (whose visits may need DMA, which needs the CPU), an
//! MMIO access that reaches a DMA requester or controller, any access while
//! a requester already has DMA pending, one the bus would fault, a port write
//! on a machine whose memory-map hook must see every port write with the CPU
//! in hand, and a transfer owned by an exception delivery.
//!
//! DMA itself is left to the loop, which moves the bytes through the CPU's
//! memory with its dirty and self-modifying-code tracking. A port access that
//! leaves a requester with DMA pending (the last command byte of a floppy
//! READ DATA, a DSP command that starts a transfer, a DMA-enable bit) ends
//! the run right after it, and the loop charges its own step and settles as
//! after the ordinary stop, servicing DMA where it would. A request that
//! appears only when the accessed devices settle ends the run at the next
//! boundary, and the loop services it before anything else, as it does right
//! after that settle. An MMIO access is not served when it reaches a DMA
//! party: its instruction's step is charged only after it completes, after
//! the DMA the loop would already have run.
//!
//! The handler never visits a device's timer. The steps before an access
//! lie inside a deadline-free interval, so charging them only moves the
//! clocks, and a port access's own step is charged the same way unless the
//! access moved a deadline into it (a timer reprogrammed to fire within the
//! step). Then the run ends right after the access and the loop charges
//! that step itself, visiting each deadline at its own time with DMA
//! serviced between them, and settles the accessed devices. So no DMA can
//! become due inside a served access, which would need a timer visit or an
//! access to a DMA party: not between two deadlines of one step, and not
//! during an MMIO access, whose instruction completes before the loop could
//! service it. The machine serves only under the deadline engine.
//!
//! What only the loop can do with the CPU after an access ends the run at
//! the next boundary instead (after an MMIO access's instruction completes),
//! where the loop does it before the next instruction, as after the ordinary
//! stop: sync the A20 gate, notice a reset request, and acknowledge a request
//! pending at the interrupt controller, with the loop's own readiness check
//! (the CPU core's interrupt-ready stop is stricter about held single-step
//! traps and waiting events). A run that began with interrupts disabled and
//! a request pending stops by itself at the first boundary able to accept
//! it once an instruction sets IF, which is where the loop's check would
//! first acknowledge it, so it needs no such stop. A run that began with IF
//! set but the CPU not ready (in the shadow of MOV SS or STI) does: the
//! loop's check can acknowledge at the boundary after the shadowed
//! instruction while the CPU, holding that instruction's single-step trap,
//! delivers the trap first and does not stop. A deadline the access moves
//! earlier than the run's budget ends the run there. The next deadline's
//! step is kept between accesses.

use std::cell::Cell;
use std::num::NonZeroU64;
use std::ops::RangeInclusive;

use bus::Bus;
use cpu::{AccessControl, AccessResult, AccessSpace, DeviceAccess, ServedRange};
use device::{InterruptController, VClock};

use crate::{
    forward_irq_levels, next_device_deadline, scheduler_error, step_target, steps_until,
    DmaChannelWiring, DmaWiring, IrqLine, MachineError, SchedulerEngine,
};

/// The accesses served while the CPU runs (IBM Personal Computer AT
/// Technical Reference, Section 1, System Board, I/O Address Map and System
/// Memory Map): the DMA controllers and their page registers (00h–0Fh,
/// 81h–8Fh, C0h–DFh), the interrupt controllers (20h–21h, A0h–A1h), the
/// timer (40h–43h), the keyboard controller and system control port B
/// (60h–64h), the real-time clock and CMOS RAM (70h–71h), the diagnostic
/// checkpoint port 80h, the fixed-disk task file (1F0h–1F7h, ATA-2
/// §§6.1–6.2), the game port (201h), the sound card's base window
/// (220h–22Fh, Creative Sound Blaster Series Hardware Programming Guide,
/// §2), the parallel ports (278h–27Fh, 378h–37Fh) and the serial ports
/// (2F8h–2FFh, 3E8h–3EFh, 2E8h–2EFh, 3F8h–3FFh), the FM synthesizer
/// (388h–38Bh, Yamaha YMF262 datasheet, Register Map), the display
/// adapters' registers (3B0h–3DFh: VGA, monochrome and color adapters), the
/// floppy controller with the fixed-disk control register (3F0h–3F7h), the
/// emulator-only firmware debug port (402h) and firmware configuration ports
/// (510h–511h, QEMU Firmware Configuration Device specification, Guest-side
/// Hardware Interface) that open firmware polls through its boot, and the
/// display memory window A0000h–BFFFFh. A range names addresses, not
/// devices: each access is offered to the machine's handler, which serves it
/// only when it can prove the result equal, whatever device decodes it
/// (`backlog/docs/design/scheduler.md` §In-process device access). Every
/// other port and trapped window still returns from the CPU, among them the
/// coprocessor ports F0h–FFh, whose acknowledgement only the loop can pass
/// to the CPU, and system control port A (92h), which drives the A20 gate
/// and the fast reset.
pub const SERVED_RANGES: [ServedRange; 23] = [
    ServedRange::Ports(0x00..=0x0F),
    ServedRange::Ports(0x20..=0x21),
    ServedRange::Ports(0x40..=0x43),
    ServedRange::Ports(0x60..=0x64),
    ServedRange::Ports(0x70..=0x71),
    ServedRange::Ports(0x80..=0x8F),
    ServedRange::Ports(0xA0..=0xA1),
    ServedRange::Ports(0xC0..=0xDF),
    ServedRange::Ports(0x1F0..=0x1F7),
    ServedRange::Ports(0x201..=0x201),
    ServedRange::Ports(0x220..=0x22F),
    ServedRange::Ports(0x278..=0x27F),
    ServedRange::Ports(0x2E8..=0x2EF),
    ServedRange::Ports(0x2F8..=0x2FF),
    ServedRange::Ports(0x378..=0x37F),
    ServedRange::Ports(0x388..=0x38B),
    ServedRange::Ports(0x3B0..=0x3DF),
    ServedRange::Ports(0x3E8..=0x3EF),
    ServedRange::Ports(0x3F0..=0x3F7),
    ServedRange::Ports(0x3F8..=0x3FF),
    ServedRange::Ports(0x402..=0x402),
    ServedRange::Ports(0x510..=0x511),
    ServedRange::Mmio(0xA0000..=0xBFFFF),
];

/// Whether every port of an access of `size` bytes at `port` lies in the
/// served port range that holds its first port.
fn within_served_ports(port: u64, size: u8) -> bool {
    let last = port + u64::from(size.max(1)) - 1;
    SERVED_RANGES.iter().any(|range| match range {
        ServedRange::Ports(ports) => {
            u64::from(*ports.start()) <= port && last <= u64::from(*ports.end())
        }
        ServedRange::Mmio(_) => false,
    })
}

/// Accesses served while the CPU ran ([`crate::Machine::served_accesses`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServedAccesses {
    /// Port accesses.
    pub ports: u64,
    /// MMIO accesses.
    pub mmio: u64,
}

impl ServedAccesses {
    pub(crate) fn add(&mut self, other: &Self) {
        self.ports += other.ports;
        self.mmio += other.mmio;
    }
}

/// What the loop finishes after a run served by [`InProcess`].
#[derive(Debug, Default)]
pub(crate) struct ServedRun {
    /// Steps of this run already charged to the clock.
    pub charged: u64,
    /// The board A20 line changed: the loop syncs it before the next
    /// instruction.
    pub sync_a20: bool,
    /// DMA became pending when the accessed devices settled after a served
    /// port access, which needs the CPU's memory: the run ended at the next
    /// boundary, and the loop services it before the run's stop, as right
    /// after that settle.
    pub settle: bool,
    /// The run ended right after an access whose remaining work the loop
    /// does itself, as after the ordinary stop: it charges the access's own
    /// step (a port access's), visiting every deadline in it with DMA
    /// serviced between them, then settles the accessed devices. A port
    /// access that leaves DMA pending ends the run this way.
    pub finish: bool,
    /// That access failed on the bus: the loop settles every device before
    /// the accessed ones, as after a failed ordinary stop.
    pub failed: bool,
    /// The error that ended the run, which the loop returns.
    pub error: Option<MachineError>,
    /// Accesses served in this run.
    pub served: ServedAccesses,
}

/// The port writes a memory-map hook observes.
#[derive(Clone, Copy)]
pub(crate) enum ObservedPorts<'a> {
    /// No hook is attached.
    None,
    /// The hook runs after every port write.
    All,
    /// The hook runs after writes to these ports.
    Only(&'a [RangeInclusive<u16>]),
}

impl ObservedPorts<'_> {
    fn observes(self, port: u64, size: u8) -> bool {
        let last = port + u64::from(size.max(1)) - 1;
        match self {
            Self::None => false,
            Self::All => true,
            Self::Only(ranges) => ranges
                .iter()
                .any(|range| u64::from(*range.start()) <= last && port <= u64::from(*range.end())),
        }
    }
}

/// Reads the board A20 line: (transition count, enabled).
type A20Line<'a> = &'a dyn Fn() -> (u64, bool);

/// The loop's state, borrowed apart from the CPU for one run.
pub(crate) struct InProcess<'a> {
    pub bus: &'a mut Bus,
    pub pic: &'a mut dyn InterruptController,
    pub clock: &'a mut VClock,
    pub completed_steps: &'a mut u64,
    pub clock_remainder: &'a mut u64,
    pub cpu_rate_numerator: NonZeroU64,
    pub cpu_tick_denominator: u128,
    pub irq_lines: &'a [IrqLine],
    pub last_levels: &'a mut [Option<bool>],
    pub dma_wirings: &'a [DmaWiring],
    pub dma_channel_wirings: &'a [DmaChannelWiring],
    /// The board A20 line and the change count the CPU last saw.
    pub a20_line: Option<(A20Line<'a>, u64)>,
    pub reset_signal: Option<&'a Cell<bool>>,
    /// The port writes an attached memory-map hook must observe; those are
    /// left to the loop, which applies the hook after each.
    pub memory_map: ObservedPorts<'a>,
    /// A trapped window served from guest RAM.
    pub ram_window: Option<&'a crate::RamWindow>,
    /// The run stops by itself at the first boundary that can accept a
    /// maskable interrupt, and began with IF clear, a request pending at the
    /// controller and none queued in the CPU: a request a served access
    /// leaves pending needs no stop of its own (see the module docs).
    pub irq_stop_armed: bool,
    /// The run's step budget.
    pub budget: u64,
    /// The step of this run at which the clock reaches the next device
    /// deadline (`u64::MAX` for none), as last computed with every device's
    /// answer cached and no device touched since; `None` when unknown.
    pub deadline_at: Option<u64>,
    pub outcome: ServedRun,
}

impl InProcess<'_> {
    /// Whether a wired requester has DMA pending, which the loop's settle
    /// would service.
    fn dma_pending(&self) -> bool {
        self.dma_wirings.iter().any(|wiring| {
            self.bus
                .device(wiring.requester)
                .is_some_and(|device| device.dma_request_pending())
        })
    }

    fn sample_irq_lines(&mut self) {
        forward_irq_levels(self.bus, self.pic, self.irq_lines, self.last_levels);
    }

    /// `Machine::charge_steps` inside a deadline-free interval: the caller
    /// knows from [`Self::deadline_at`] that no device is due before the
    /// target and every device's next event is cached, so
    /// `Machine::advance_to` would only move the clocks (`Bus::tick_due`
    /// visits nothing due and refills no answer, IRQ sampling finds no change
    /// since the last sample, and no DMA becomes due).
    fn advance(&mut self, steps: u64) -> Result<(), MachineError> {
        *self.completed_steps = self
            .completed_steps
            .checked_add(steps)
            .ok_or_else(|| scheduler_error("step counter exhausted"))?;
        self.outcome.charged += steps;
        let (target, remainder) = step_target(
            *self.clock,
            steps,
            *self.clock_remainder,
            self.cpu_rate_numerator,
            self.cpu_tick_denominator,
        )?;
        *self.clock_remainder = remainder;
        if *self.clock < target {
            *self.clock = target;
            self.bus.advance_quiet(target);
        }
        Ok(())
    }

    /// Charge a served port access's own step as the loop does right after
    /// the access, when no device is due within the step; false, with
    /// nothing charged, when the access moved a deadline into its own step
    /// or a counter or the deadline query fails. The loop then charges the
    /// step itself ([`ServedRun::finish`]): it visits each deadline at its
    /// own time and services DMA between them, which only it can do, since
    /// DMA needs the CPU's memory.
    fn own_step(&mut self) -> bool {
        let Ok((target, remainder)) = step_target(
            *self.clock,
            1,
            *self.clock_remainder,
            self.cpu_rate_numerator,
            self.cpu_tick_denominator,
        ) else {
            return false;
        };
        // The query `Machine::advance_to` makes first, with every answer
        // the access invalidated computed again.
        match next_device_deadline(SchedulerEngine::Deadline, self.bus, *self.clock) {
            Ok(Some(deadline)) if deadline <= target => return false,
            Err(_) => return false,
            Ok(_) => {}
        }
        let Some(completed) = self.completed_steps.checked_add(1) else {
            return false;
        };
        *self.completed_steps = completed;
        *self.clock_remainder = remainder;
        self.outcome.charged += 1;
        if *self.clock < target {
            *self.clock = target;
            self.bus.advance_quiet(target);
            // `Machine::advance_to` samples once the clock moves, before the
            // accessed devices settle; the access may have moved a line.
            self.sample_irq_lines();
        }
        true
    }

    /// Whether the access may be served here at all; see the module docs.
    fn eligible(&mut self, access: &DeviceAccess) -> bool {
        if access.delivery {
            return false;
        }
        let port = access.space == AccessSpace::Port;
        if port && access.write && self.memory_map.observes(access.address, access.size) {
            return false;
        }
        // The CPU core matches an access by its first port only; a wider
        // access running past the end of its served range (a doubleword OUT
        // at 8Fh reaches 92h) returns, so the ports outside stay on the exit
        // path.
        if port && !within_served_ports(access.address, access.size) {
            return false;
        }
        // A port access that leaves DMA pending ends the run for the loop
        // to service it (see the module docs). An MMIO access to a DMA
        // requester or controller could start a transfer the loop would run
        // before the access's instruction completes and its step is charged.
        let dma_wirings = self.dma_wirings;
        let channel_wirings = self.dma_channel_wirings;
        let free = |index: usize| {
            !dma_wirings
                .iter()
                .any(|wiring| wiring.requester == index || wiring.controller == index)
                && !channel_wirings
                    .iter()
                    .any(|route| route.requester == index || route.controller == index)
        };
        let reaches = if port {
            // Only whether the bus would fault.
            self.bus
                .port_access_reaches_only(access.address as u16, access.size, |_| true)
        } else {
            self.bus
                .mmio_access_reaches_only(access.address, access.size, free)
        };
        if !reaches || self.dma_pending() {
            return false;
        }

        if self.deadline_at.is_none() {
            self.deadline_at = self.deadline_from_now();
        }
        // `access.steps` is the run's position once this access is charged.
        self.deadline_at.is_some_and(|at| access.steps < at)
    }

    /// The step of this run at which the clock reaches the next device
    /// deadline, `u64::MAX` for none; `None` when the query fails, which the
    /// loop then reports at its own point.
    fn deadline_from_now(&mut self) -> Option<u64> {
        match next_device_deadline(SchedulerEngine::Deadline, self.bus, *self.clock) {
            Ok(None) => Some(u64::MAX),
            Ok(Some(deadline)) => Some(self.outcome.charged.saturating_add(steps_until(
                deadline,
                *self.clock,
                *self.clock_remainder,
                self.cpu_rate_numerator,
                self.cpu_tick_denominator,
            ))),
            Err(_) => None,
        }
    }

    /// Record an error that ends the run.
    fn fail(&mut self, error: MachineError) -> AccessResult {
        self.outcome.error.get_or_insert(error);
        AccessResult::Fault
    }
}

impl cpu::DeviceAccessHandler for InProcess<'_> {
    fn access(&mut self, access: &DeviceAccess, control: &mut AccessControl) -> AccessResult {
        self.serve(access, control, None)
    }

    fn access_with_ram(
        &mut self,
        access: &DeviceAccess,
        control: &mut AccessControl,
        ram: &mut dyn cpu::GuestRam,
    ) -> AccessResult {
        self.serve(access, control, Some(ram))
    }
}

impl InProcess<'_> {
    /// Serve one access as the loop would; see the module docs.
    fn serve(
        &mut self,
        access: &DeviceAccess,
        control: &mut AccessControl,
        ram: Option<&mut dyn cpu::GuestRam>,
    ) -> AccessResult {
        let port = access.space == AccessSpace::Port;
        // An access the RAM window maps reaches guest RAM, as in the loop;
        // without RAM at hand it returns to the loop.
        let in_ram = (!port)
            .then(|| {
                self.ram_window
                    .and_then(|w| w.route(access.address, access.size))
            })
            .flatten();
        if in_ram.is_some() && ram.is_none() {
            return AccessResult::Fallback;
        }
        // The loop charges a port access's own step after the access, and an
        // MMIO access's when its instruction completes.
        let Some(charge) = access.steps.checked_sub(self.outcome.charged) else {
            return AccessResult::Fallback;
        };
        // A port read in the run's last step returns: the ordinary stop
        // ends the run with the read's answer pending, so a string input's
        // store and everything after it wait for the next run, after the
        // loop's cap check. Served, the store and the next element would run
        // first, past the cap when the store exits to the loop.
        if port && !access.write && access.steps >= self.budget {
            return AccessResult::Fallback;
        }
        if (port && charge == 0) || !self.eligible(access) {
            return AccessResult::Fallback;
        }
        // `eligible` proved the clock stays short of the next deadline.
        if let Err(error) = self.advance(charge - u64::from(port)) {
            return self.fail(error);
        }
        let result = match (access.space, access.write) {
            (AccessSpace::Mmio, write) if in_ram.is_some() => {
                let (Some(base), Some(ram)) = (in_ram, ram) else {
                    return AccessResult::Fallback;
                };
                let len = usize::from(access.size.min(8));
                if write {
                    // As in the loop, bytes outside RAM are dropped.
                    let _ = ram.write(base, &access.value.to_le_bytes()[..len]);
                    Ok(0)
                } else {
                    let mut bytes = [0xFF; 8];
                    if !ram.read(base, &mut bytes[..len]) {
                        bytes = [0xFF; 8];
                    }
                    Ok(u64::from_le_bytes(bytes) & (u64::MAX >> (64 - 8 * len)))
                }
            }
            (AccessSpace::Port, true) => self
                .bus
                .port_write(access.address as u16, access.size, access.value as u32)
                .map(|()| 0),
            (AccessSpace::Port, false) => self
                .bus
                .port_read(access.address as u16, access.size)
                .map(u64::from),
            (AccessSpace::Mmio, true) => self
                .bus
                .mmio_write(access.address, access.size, access.value)
                .map(|()| 0),
            (AccessSpace::Mmio, false) => self.bus.mmio_read(access.address, access.size),
        };
        if port {
            self.outcome.served.ports += 1;
        } else {
            self.outcome.served.mmio += 1;
        }
        #[cfg(feature = "host-profile")]
        {
            use device::profile::{ProfileEvent, ProfileSpace};
            let profile = self.bus.profile();
            let (event, space) = if port {
                (ProfileEvent::ServedPortIo, ProfileSpace::Port)
            } else {
                (ProfileEvent::ServedMmio, ProfileSpace::Mmio)
            };
            profile.event(event);
            profile.access(space, access.address, true);
        }
        // What the loop does after the access, in its order.
        let mut stop = false;
        if port && access.write {
            if let Some((line, seen)) = self.a20_line {
                stop |= line().0 != seen;
                self.outcome.sync_a20 |= stop;
            }
        }
        // The loop asks every device again after any access. By the
        // access-coupling contract (`Bus::add_access_coupling`), a completed
        // MMIO access changes the timers only of the devices it reached and
        // their declared access dependents, whose answers the bus dropped
        // when it synchronized them. Every other device keeps the state its
        // cached answer came from, and an answer does not change while the
        // clock moves towards it, so asking those devices again would return
        // the same deadlines. A port access can reach board lines and timers
        // shared beyond its devices, and a failed access ends the run, so
        // both still drop every answer.
        if port || result.is_err() {
            self.bus.invalidate_deadlines();
        }
        self.deadline_at = None;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                // The run ends here; the loop charges the own step and
                // settles, as after a failed ordinary stop.
                self.outcome.finish = true;
                self.outcome.failed = true;
                return self.fail(error.into());
            }
        };
        // A request the access raised: the loop services DMA inside the
        // access's own step, at the first deadline visit in it, or else
        // right after the accessed devices settle, so it charges that step
        // and settles itself.
        if port && (self.dma_pending() || !self.own_step()) {
            self.outcome.finish = true;
            control.stop_at(access.steps);
            return AccessResult::Handled(value);
        }
        self.bus.settle_accessed();
        self.sample_irq_lines();
        self.outcome.settle |= self.dma_pending();
        stop |= self.outcome.settle;
        stop |= self.reset_signal.is_some_and(Cell::get);
        // A request at the controller: the loop's own acceptance check runs
        // at the boundary after this access, as after the ordinary stop.
        stop |= !self.irq_stop_armed && self.pic.poll().is_some();
        // A failed query also stops here: the loop repeats it after its
        // reset and cap checks, where the ordinary stop's loop would fail.
        self.deadline_at = self.deadline_from_now();
        // The answers kept across an MMIO access must be the ones the loop's
        // full re-query would get, which holds only while every device an
        // MMIO access can retime through shared state is declared an access
        // dependent of the accessed device.
        #[cfg(debug_assertions)]
        if !port {
            let stale = self.bus.stale_deadlines();
            debug_assert!(
                stale.is_empty(),
                "an MMIO access changed the next event of devices {stale:?} \
                 that are not declared access dependents of the devices it reached"
            );
        }
        match self.deadline_at {
            Some(at) if !stop => {
                if at < self.budget {
                    control.stop_at(at);
                }
            }
            _ => control.stop_at(self.outcome.charged),
        }
        AccessResult::Handled(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bus::{DecodeMap, MmioRange};
    use cpu::DeviceAccessHandler;
    use device::{Device, DeviceError, NextEvent};
    use std::rc::Rc;

    const WINDOW: u64 = 0xA0000;

    /// A display window whose stores fail once `reject` is set.
    struct Window {
        reject: bool,
    }

    impl Device for Window {
        fn name(&self) -> &'static str {
            "test-window"
        }
        fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
            Ok(0xFF)
        }
        fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
            Ok(())
        }
        fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
            Ok(0xFF)
        }
        fn mmio_write(&mut self, addr: u64, _: u8, _: u64) -> Result<(), DeviceError> {
            if self.reject {
                return Err(DeviceError::UnmappedRegister {
                    kind: "mmio",
                    offset: addr,
                });
            }
            Ok(())
        }
        fn tick(&mut self, _: VClock) {}
        fn next_event(&self, _: VClock) -> NextEvent {
            NextEvent::Dormant
        }
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {}
        fn save(&self) -> Vec<u8> {
            Vec::new()
        }
        fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
            Ok(())
        }
    }

    /// A device no access reaches, counting how often it is asked for its
    /// next event.
    struct Bystander(Rc<Cell<u64>>);

    impl Device for Bystander {
        fn name(&self) -> &'static str {
            "test-bystander"
        }
        fn port_read(&mut self, _: u16, _: u8) -> Result<u32, DeviceError> {
            Ok(0xFF)
        }
        fn port_write(&mut self, _: u16, _: u8, _: u32) -> Result<(), DeviceError> {
            Ok(())
        }
        fn mmio_read(&mut self, _: u64, _: u8) -> Result<u64, DeviceError> {
            Ok(0xFF)
        }
        fn mmio_write(&mut self, _: u64, _: u8, _: u64) -> Result<(), DeviceError> {
            Ok(())
        }
        fn tick(&mut self, _: VClock) {}
        fn next_event(&self, _: VClock) -> NextEvent {
            self.0.set(self.0.get() + 1);
            NextEvent::At(VClock(1 << 40))
        }
        fn irq_pending(&self) -> bool {
            false
        }
        fn reset(&mut self) {}
        fn save(&self) -> Vec<u8> {
            Vec::new()
        }
        fn restore(&mut self, _: &[u8]) -> Result<(), DeviceError> {
            Ok(())
        }
    }

    /// Serve one store to the window, rejected or not, and report whether
    /// it was handled and how often the bystander was asked from the store
    /// until the next deadline query after it.
    fn store(reject: bool) -> (bool, u64) {
        let queries = Rc::new(Cell::new(0));
        let mut bus = Bus::new(
            vec![
                Box::new(Window { reject }),
                Box::new(Bystander(queries.clone())),
            ],
            DecodeMap {
                ports: Vec::new(),
                mmio: vec![MmioRange {
                    addrs: WINDOW..=0xBFFFF,
                    device: 0,
                }],
            },
        );
        bus.next_event();
        let mut pic = chipset::NoInterruptController;
        let mut clock = VClock::ZERO;
        let (mut completed, mut remainder) = (0, 0);
        let mut handler = InProcess {
            bus: &mut bus,
            pic: &mut pic,
            clock: &mut clock,
            completed_steps: &mut completed,
            clock_remainder: &mut remainder,
            cpu_rate_numerator: NonZeroU64::MIN,
            cpu_tick_denominator: 1,
            irq_lines: &[],
            last_levels: &mut [],
            dma_wirings: &[],
            dma_channel_wirings: &[],
            a20_line: None,
            reset_signal: None,
            memory_map: ObservedPorts::None,
            ram_window: None,
            irq_stop_armed: false,
            budget: 100,
            deadline_at: None,
            outcome: ServedRun::default(),
        };
        let before = queries.get();
        let result = handler.access(
            &DeviceAccess {
                space: AccessSpace::Mmio,
                write: true,
                address: WINDOW,
                size: 1,
                value: 0x55,
                steps: 2,
                delivery: false,
            },
            &mut AccessControl::default(),
        );
        let handled = matches!(result, AccessResult::Handled(_));
        assert_eq!(handled, !handler.outcome.failed);
        bus.next_event();
        (handled, queries.get() - before)
    }

    /// A completed store keeps the deadlines of devices it did not reach;
    /// a failed one drops every answer before the loop charges its step,
    /// as after a failed ordinary stop.
    #[test]
    fn a_failed_served_store_drops_every_cached_deadline() {
        // Debug builds ask every device once more after a completed store,
        // to check the answers kept.
        assert_eq!(store(false), (true, u64::from(cfg!(debug_assertions))));
        let (handled, queries) = store(true);
        assert!(!handled);
        assert!(queries >= 1, "{queries} queries after the failed store");
    }
}
