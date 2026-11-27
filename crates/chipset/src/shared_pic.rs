// SPDX-License-Identifier: AGPL-3.0-or-later
//! A single 8259 PIC shared between its two faces (§5.1, §3.3).
//!
//! The 8259 sits on two seams at once: the guest programs it and issues EOI
//! through its **[`Device`] face** (ports `0x20`/`0x21`, decoded by the `bus`),
//! while the machine drives INTA and reads the aggregated INTR through its
//! **[`InterruptController`] face**. Both faces must see the *same* register
//! state — an EOI the guest writes through the bus has to clear the ISR the
//! machine's INTA set.
//!
//! One live PIC therefore backs both faces, shared behind `Rc<RefCell<…>>`.
//! [`SharedPic`] is a cheap handle over that cell: cloned once into the bus's
//! device set (as a [`Device`]) and once into the machine (as an
//! [`InterruptController`]). The machine core still never names `Pic8259` — it
//! sees only the [`InterruptController`] trait object.
//!
//! ## Cascade
//!
//! A PC/AT has two of these chips: a master (`0x20`/`0x21`) carrying IRQ0–IRQ7
//! and a slave (`0xA0`/`0xA1`) carrying IRQ8–IRQ15, the slave's INT output wired
//! into the master's IRQ2 input. [`CascadedPic`] models that pair as one
//! 16-line [`InterruptController`]. It owns a [`SharedPic`] per chip and exposes
//! each as its own [`Device`] face ([`CascadedPic::master_device`] /
//! [`CascadedPic::slave_device`]) so the chipset can decode both port pairs; the
//! single-chip [`SharedPic`] stays usable on its own for chipsets that model
//! only the master.
//!
//! Every input is level-gated per the Intel 8259A datasheet,
//! "Edge and Level Triggered Modes": slave INT falling before INTA withdraws
//! master IRR2 from priority resolution, just as device input falls withdraw
//! their unacknowledged requests.
//! The slave IR7 vector is a defensive fallback for a cascade INTA with no
//! slave request ("Interrupt Sequence"); see `backlog/docs/design/spec.md` §5.1.
//!
//! Each device face saves and validates its underlying PIC snapshot, including
//! input levels and the absolute trace time base. Shared wiring and observation
//! logs remain host state; restore both chip faces when restoring a cascade.
//! See Intel 8259A datasheet, Cascade Mode.

use std::cell::RefCell;
use std::rc::Rc;

use device::{Device, DeviceError, InterruptController, NextEvent, TraceEvent, VClock};
use devices::Pic8259;

/// An observed Intel 8259A interrupt sequence or OCW2 EOI command (§5.1).
///
/// Intel 8259A datasheet, "Interrupt Sequence" and "End of Interrupt": a
/// cascaded INTA supplies one CPU vector, while each chip receives its own EOI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PicObservation {
    /// The vector returned to the CPU, not the internal cascade vector.
    Acknowledge { at: VClock, vector: u8 },
    /// An EOI through a command port (`Some`) or the controller seam (`None`).
    Eoi {
        at: VClock,
        command: u16,
        value: Option<u8>,
    },
}

struct Recording {
    clock: Rc<dyn Fn() -> VClock>,
    events: Vec<PicObservation>,
}

/// Optional, shared PIC observations for a test harness (§5.1, §10).
///
/// Recording allocates events only after [`SharedPic::observe`] is called.
/// It is separate from snapshots and port traces so draining diagnostics cannot
/// alter chip state or fabricate guest port accesses.
#[derive(Clone, Default)]
pub struct PicObservationLog(Rc<RefCell<Option<Recording>>>);

impl PicObservationLog {
    /// Drain observations from both faces of the controller in execution order.
    pub fn take(&self) -> Vec<PicObservation> {
        self.0
            .borrow_mut()
            .as_mut()
            .map(|recording| std::mem::take(&mut recording.events))
            .unwrap_or_default()
    }

    fn acknowledge(&self, vector: u8) {
        if let Some(recording) = self.0.borrow_mut().as_mut() {
            let at = (recording.clock)();
            recording
                .events
                .push(PicObservation::Acknowledge { at, vector });
        }
    }

    fn eoi(&self, command: u16, value: Option<u8>) {
        if let Some(recording) = self.0.borrow_mut().as_mut() {
            let at = (recording.clock)();
            recording
                .events
                .push(PicObservation::Eoi { at, command, value });
        }
    }
}

/// A shared handle to one live 8259 PIC (§5.1).
///
/// Clone it to hand the same PIC to the bus (its [`Device`] face) and to the
/// machine (its [`InterruptController`] face); both clones borrow the one
/// [`Pic8259`] through the shared cell, so register state stays consistent.
#[derive(Clone)]
pub struct SharedPic {
    inner: Rc<RefCell<Pic8259>>,
    cascade_master: Option<Rc<RefCell<Pic8259>>>,
    observations: PicObservationLog,
    command: u16,
}

impl SharedPic {
    /// Wrap a fresh master 8259 (ports `0x20`/`0x21`) in a shared cell.
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Pic8259::new())),
            cascade_master: None,
            observations: PicObservationLog::default(),
            command: 0x20,
        }
    }

    /// Wrap a fresh slave 8259 (ports `0xA0`/`0xA1`) in a shared cell.
    pub fn new_slave() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Pic8259::new_slave())),
            cascade_master: None,
            observations: PicObservationLog::default(),
            command: 0xA0,
        }
    }

    /// Enable diagnostic recording shared with the controller and both chips.
    ///
    /// Pass the bus's read-only observation clock: a dormant PIC receives no
    /// ticks, so its last device access cannot timestamp INTA or EOI. Reading
    /// this clock must not synchronize hardware or change machine state.
    /// Repeated calls update the clock while preserving pending observations.
    pub fn observe(&self, clock: Rc<dyn Fn() -> VClock>) -> PicObservationLog {
        let mut recording = self.observations.0.borrow_mut();
        if let Some(recording) = recording.as_mut() {
            recording.clock = clock;
        } else {
            *recording = Some(Recording {
                clock,
                events: Vec::new(),
            });
        }
        self.observations.clone()
    }

    /// Propagate slave INT to master IR2, including changes from guest writes.
    ///
    /// Intel 8259A datasheet, "Cascade Mode" and "Edge and Level Triggered
    /// Modes": the slave's serviceable requests drive INT, and IR2 must stay
    /// high until INTA. INT and INTA share a machine boundary here, so a low
    /// INT withdraws IRR2 before priority resolution. Repeated high levels
    /// preserve edge detection without inventing requests.
    fn sync_cascade(&self) {
        if let Some(master) = &self.cascade_master {
            let slave_int = self.inner.borrow().irq_pending();
            let mut master = master.borrow_mut();
            master.set_irq_line(CASCADE_IRQ, slave_int);
        }
    }
}

impl Default for SharedPic {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for SharedPic {
    fn name(&self) -> &'static str {
        // A `&'static str` can't be borrowed out of the cell, so return the
        // stable name directly (it matches `Pic8259::name`).
        "8259a-pic"
    }

    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
        self.inner.borrow_mut().port_read(port, size)
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
        self.inner.borrow_mut().port_write(port, size, value)?;
        // ICW1 and OCW3 take precedence over OCW2, even if bit 5 is set.
        if port == self.command && value as u8 & 0x38 == 0x20 {
            self.observations.eoi(port, Some(value as u8));
        }
        self.sync_cascade();
        Ok(())
    }

    fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        self.inner.borrow_mut().mmio_read(addr, size)
    }

    fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError> {
        self.inner.borrow_mut().mmio_write(addr, size, value)
    }

    fn tick(&mut self, now: VClock) {
        self.inner.borrow_mut().tick(now);
    }

    /// Intel 8259A datasheet, Interrupt Sequence and OCW2: only line edges,
    /// register accesses and INTA change requests; no autonomous timer runs.
    fn next_event(&self, _: VClock) -> NextEvent {
        NextEvent::Dormant
    }

    fn irq_pending(&self) -> bool {
        self.inner.borrow().irq_pending()
    }

    fn reset(&mut self) {
        self.inner.borrow_mut().reset();
    }

    fn save(&self) -> Vec<u8> {
        self.inner.borrow().save()
    }

    fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError> {
        self.inner.borrow_mut().restore(snapshot)
    }

    fn set_trace_policy(&mut self, policy: device::TracePolicy) {
        self.inner.borrow_mut().set_trace_policy(policy);
    }

    fn trace_dropped(&self) -> u64 {
        self.inner.borrow().trace_dropped()
    }

    fn take_trace(&mut self) -> Vec<TraceEvent> {
        self.inner.borrow_mut().take_trace()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn core::any::Any> {
        Some(self)
    }
}

impl InterruptController for SharedPic {
    fn set_irq_line(&mut self, irq: u8, level: bool) {
        self.inner.borrow_mut().set_irq_line(irq, level);
    }

    fn poll(&self) -> Option<u8> {
        self.inner.borrow().poll()
    }

    fn acknowledge(&mut self) -> Option<u8> {
        let vector = self.inner.borrow_mut().acknowledge()?;
        self.observations.acknowledge(vector);
        Some(vector)
    }

    fn end_of_interrupt(&mut self) {
        self.inner.borrow_mut().end_of_interrupt();
        self.observations.eoi(self.command, None);
    }
}

/// The master input line the slave's INT output is wired to on every PC/AT.
const CASCADE_IRQ: u8 = 2;

/// A cascaded master + slave 8259 pair presented as one 16-line controller
/// (§5.1, §3.3).
///
/// Owns a master and a slave [`SharedPic`]. The 16 logical lines map as
/// IRQ0–IRQ7 → master lines 0–7 and IRQ8–IRQ15 → slave lines 0–7. Whenever the
/// slave has a serviceable request, its INT output asserts the master's IRQ2
/// input (`IRQ2`); the master then resolves priority across all
/// sixteen lines, with IRQ8–IRQ15 slotting in at IRQ2's priority.
///
/// Hand [`CascadedPic::master_device`] and [`CascadedPic::slave_device`] to the
/// bus as the two [`Device`] faces (decoded at `0x20`/`0x21` and `0xA0`/`0xA1`)
/// and the `CascadedPic` itself to the machine as its [`InterruptController`].
///
/// ## EOI contract
///
/// A cascaded interrupt occupies two in-service bits at once: the actual line on
/// the slave, and IRQ2 on the master (the cascade). Real software ends it with
/// two OCW2 writes — EOI the slave, then EOI the master. [`end_of_interrupt`]
/// collapses that into a single call for the machine's uniform EOI seam: it
/// looks at the master line a non-specific EOI would end (the top in-service
/// line, or in special mask mode the top unmasked one), and **if that is the
/// cascade line (IRQ2) it EOIs the slave first, then the master**; otherwise
/// it EOIs the master alone. This assumes the cascaded interrupt is the one a
/// non-specific EOI on the master ends — which holds because a cascaded
/// [`acknowledge`] nests the master's IRQ2 and the slave's line together. Two
/// slave interrupts nested on top of each other (special fully-nested mode)
/// are not modelled.
///
/// Intel 8259A datasheet, "Special Mask Mode" and "Cascade Mode": each chip
/// keeps its own special mask mode. On the slave it decides which slave
/// requests drive INT while a masked slave level is in service; the master's
/// IR2 nesting still applies on top of that.
///
/// Intel 8259A datasheet, "Edge and Level Triggered Modes": every input
/// is level-gated. Slave INT falling before INTA withdraws master IRR2,
/// allowing another master request to win or leaving no interrupt. Guest port
/// writes therefore cannot cause a spurious cascade INTA. The slave IR7 vector
/// ("Interrupt Sequence") remains a defensive fallback if a cascade INTA finds
/// no slave request; it leaves slave ISR clear while master IR2 needs an EOI.
///
/// [`end_of_interrupt`]: InterruptController::end_of_interrupt
/// [`acknowledge`]: InterruptController::acknowledge
#[derive(Clone)]
pub struct CascadedPic {
    master: SharedPic,
    slave: SharedPic,
}

impl CascadedPic {
    /// Wrap a fresh master + slave pair, each unprogrammed.
    pub fn new() -> Self {
        let master = SharedPic::new();
        let mut slave = SharedPic::new_slave();
        slave.cascade_master = Some(Rc::clone(&master.inner));
        slave.observations = master.observations.clone();
        Self { master, slave }
    }

    /// The master chip's [`Device`] face (decoded at `0x20`/`0x21`).
    ///
    /// A cheap clone sharing the same live chip as this controller, for the bus
    /// device set.
    pub fn master_device(&self) -> SharedPic {
        self.master.clone()
    }

    /// The slave chip's [`Device`] face (decoded at `0xA0`/`0xA1`).
    pub fn slave_device(&self) -> SharedPic {
        self.slave.clone()
    }

    /// Drive the master's IRQ2 (cascade) input from the slave's INT output.
    ///
    /// The slave asserts its INT whenever it has a serviceable request. The
    /// master latches IRQ2 on the rising edge, exactly as edge-triggered wiring
    /// would; when the slave's request moves into service (or is masked away)
    /// the line drops again and any pending master IRR2 is withdrawn per the
    /// Intel 8259A datasheet, "Edge and Level Triggered Modes".
    ///
    /// Intel 8259A datasheet, "Cascade Mode": this runs on slave line,
    /// acknowledge and EOI operations; the shared slave handle also propagates
    /// INT after every guest port write.
    fn sync_cascade(&mut self) {
        self.slave.sync_cascade();
    }
}

impl Default for CascadedPic {
    fn default() -> Self {
        Self::new()
    }
}

impl InterruptController for CascadedPic {
    fn set_irq_line(&mut self, irq: u8, level: bool) {
        if irq < 8 {
            self.master.set_irq_line(irq, level);
        } else if irq < 16 {
            self.slave.set_irq_line(irq - 8, level);
            // A change on any slave line may raise or drop the cascade.
            self.sync_cascade();
        }
    }

    fn poll(&self) -> Option<u8> {
        // The master resolves priority first. A winning cascade line means the
        // vector comes from the slave's own highest-priority pending line.
        if self.master.inner.borrow().highest_pending() == Some(CASCADE_IRQ) {
            Some(
                self.slave
                    .poll()
                    .unwrap_or_else(|| self.slave.inner.borrow().vector_base() + 7),
            )
        } else {
            self.master.poll()
        }
    }

    fn acknowledge(&mut self) -> Option<u8> {
        let master_line = self.master.inner.borrow().highest_pending();
        if master_line == Some(CASCADE_IRQ) {
            // Cascaded INTA: the slave supplies the vector and moves its line
            // into service; the master's IRQ2 moves into service in step.
            let vector = self.slave.inner.borrow_mut().acknowledge()?;
            self.master.inner.borrow_mut().acknowledge();
            self.master.observations.acknowledge(vector);
            // The slave's INT typically drops now that its line is in service.
            self.sync_cascade();
            Some(vector)
        } else {
            self.master.acknowledge()
        }
    }

    fn end_of_interrupt(&mut self) {
        // See the EOI contract on the type: a single call ends the interrupt a
        // non-specific EOI ends on the master, and if that is the cascade line
        // it also ends the matching slave interrupt.
        let cascaded = self.master.inner.borrow().non_specific_eoi_line() == Some(CASCADE_IRQ);
        if cascaded {
            self.slave.end_of_interrupt();
        }
        self.master.end_of_interrupt();
        // Ending a slave interrupt may re-expose a lower pending slave line.
        self.sync_cascade();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloned_pic_faces_share_trace_policy_history_and_drop_count() {
        let pic = SharedPic::new();
        let mut bus = testkit::MockBus::new(Box::new(pic.clone()));
        bus.port_read(0x21, 1).unwrap();
        let mut host = pic.clone();
        assert!(host.take_trace().is_empty());
        host.set_trace_policy(device::TracePolicy::Ring { capacity: 2 });
        for _ in 0..5 {
            bus.port_read(0x21, 1).unwrap();
        }
        assert_eq!(host.trace_dropped(), 3);
        assert_eq!(bus.device().trace_dropped(), 3);
        assert_eq!(host.take_trace().len(), 2);
        assert!(bus.device_mut().take_trace().is_empty());
        assert_eq!(bus.device().trace_dropped(), 3);
        host.set_trace_policy(device::TracePolicy::Off);
        bus.port_read(0x21, 1).unwrap();
        assert!(host.take_trace().is_empty());
        assert_eq!(host.trace_dropped(), 0);
    }

    #[test]
    fn shared_faces_validate_snapshots_and_preserve_held_inputs() {
        for slave in [false, true] {
            let make = || {
                if slave {
                    SharedPic::new_slave()
                } else {
                    SharedPic::new()
                }
            };
            let command = if slave { SLAVE_CMD } else { MASTER_CMD };
            let mut source = make();
            source.sync(VClock(251));
            source.set_irq_line(7, true);
            source.acknowledge();
            source.end_of_interrupt();
            let mut bus = testkit::MockBus::new(Box::new(source.clone()));
            let valid = bus.device().save();
            let mut restored = make();
            restored.restore(&valid).unwrap();
            restored.set_irq_line(7, true);
            assert_eq!(restored.poll(), source.poll());
            assert_eq!(restored.poll(), None);
            source.take_trace();
            for at in [251, 252, 1000] {
                source.sync(VClock(at));
                restored.sync(VClock(at));
                assert_eq!(source.port_read(command, 1), restored.port_read(command, 1));
                assert_eq!(source.save(), restored.save());
                assert_eq!(source.take_trace(), restored.take_trace());
            }
            for length in 0..valid.len() {
                assert!(matches!(
                    bus.device_mut().restore(&valid[..length]),
                    Err(DeviceError::InvalidSnapshot { .. })
                ));
            }
            for index in 0..valid.len() {
                for value in [0, 1, 0x7F, 0xFF] {
                    let mut blob = valid.clone();
                    blob[index] = value;
                    bus.device_mut().restore(&valid).unwrap();
                    match bus.device_mut().restore(&blob) {
                        Ok(()) => {
                            bus.device_mut().sync(VClock(u64::MAX));
                            bus.device().next_event(VClock(u64::MAX));
                            bus.port_read(command, 1).unwrap();
                            bus.port_write(command, 1, 0x20).unwrap();
                        }
                        Err(DeviceError::InvalidSnapshot { .. }) => {
                            assert_eq!(bus.device().save(), valid)
                        }
                        Err(error) => panic!("unexpected restore error: {error}"),
                    }
                }
            }
            let mut blob = valid.clone();
            blob[4] = 0xFF;
            assert!(matches!(
                bus.device_mut().restore(&blob),
                Err(DeviceError::InvalidSnapshot { .. })
            ));
            blob = valid;
            blob.push(0);
            assert!(matches!(
                bus.device_mut().restore(&blob),
                Err(DeviceError::InvalidSnapshot { .. })
            ));
        }
    }

    /// Intel 8259A datasheet, Interrupt Sequence and OCW2: elapsed time cannot
    /// change IRR/ISR; line edges, INTA and EOI do, on either device face.
    #[test]
    fn dormant_faces_match_per_clock_interrupts_and_traces() {
        for slave in [false, true] {
            let mut reference = if slave {
                SharedPic::new_slave()
            } else {
                SharedPic::new()
            };
            let mut scheduled = if slave {
                SharedPic::new_slave()
            } else {
                SharedPic::new()
            };
            reference.set_trace_policy(device::TracePolicy::Ring { capacity: 100 });
            scheduled.set_trace_policy(device::TracePolicy::Ring { capacity: 100 });
            let command = if slave { SLAVE_CMD } else { MASTER_CMD };
            let base = if slave { SLAVE_BASE } else { MASTER_BASE };
            init_chip(&mut reference, command, command + 1, base, 2);
            init_chip(&mut scheduled, command, command + 1, base, 2);
            let clock = Rc::new(std::cell::Cell::new(VClock::ZERO));
            let observation_clock: Rc<dyn Fn() -> VClock> = {
                let clock = clock.clone();
                Rc::new(move || clock.get())
            };
            let reference_log = reference.observe(observation_clock.clone());
            let scheduled_log = scheduled.observe(observation_clock);
            let mut reference_bus = testkit::MockBus::new(Box::new(reference.clone()));
            let mut scheduled_bus = testkit::MockBus::new(Box::new(scheduled.clone()));
            for target in [1, 31, 1000, 65537] {
                clock.set(VClock(target));
                while reference_bus.clock().0 < target {
                    reference_bus.tick(1);
                }
                assert_eq!(
                    scheduled_bus.device().next_event(VClock(target)),
                    device::NextEvent::Dormant
                );
                scheduled_bus.device_mut().sync(VClock(target));
                for pic in [&mut reference, &mut scheduled] {
                    pic.set_irq_line(3, true);
                    assert_eq!(pic.acknowledge(), Some(base + 3));
                    pic.set_irq_line(3, false);
                }
                assert_eq!(
                    reference_bus.port_read(command + 1, 1).unwrap(),
                    scheduled_bus.port_read(command + 1, 1).unwrap()
                );
                for bus in [&mut reference_bus, &mut scheduled_bus] {
                    bus.port_write(command, 1, 0x20).unwrap();
                }
                assert_eq!(reference_bus.device().save(), scheduled_bus.device().save());
                assert_eq!(
                    reference_bus.device_mut().take_trace(),
                    scheduled_bus.device_mut().take_trace()
                );
                assert_eq!(reference_log.take(), scheduled_log.take());
            }
        }
    }

    // Master ports and the PC BIOS vector base (IRQ0 → 0x08).
    const MASTER_CMD: u16 = 0x20;
    const MASTER_DATA: u16 = 0x21;
    const MASTER_BASE: u8 = 0x08;
    // Slave ports and vector base (IRQ8 → 0x70).
    const SLAVE_CMD: u16 = 0xA0;
    const SLAVE_DATA: u16 = 0xA1;
    const SLAVE_BASE: u8 = 0x70;

    // ICW1 = init (bit4) + ICW4-needed (bit0); ICW4 = 8086 mode (bit0).
    const ICW1: u32 = 0x11;
    const ICW4_8086: u32 = 0x01;

    /// Program a chip's ICW1–ICW4 through its `Device` face at `base` port.
    fn init_chip(dev: &mut SharedPic, cmd: u16, data: u16, vector_base: u8, icw3: u32) {
        dev.port_write(cmd, 1, ICW1).unwrap();
        dev.port_write(data, 1, u32::from(vector_base)).unwrap();
        dev.port_write(data, 1, icw3).unwrap();
        dev.port_write(data, 1, ICW4_8086).unwrap();
    }

    /// A fully initialized cascaded pair (master base 0x08, slave base 0x70).
    fn init_pair() -> CascadedPic {
        let pic = CascadedPic::new();
        let mut master = pic.master_device();
        let mut slave = pic.slave_device();
        // ICW3: master has a slave on IRQ2 (bit 2); slave identity is 2.
        init_chip(&mut master, MASTER_CMD, MASTER_DATA, MASTER_BASE, 0x04);
        init_chip(&mut slave, SLAVE_CMD, SLAVE_DATA, SLAVE_BASE, 0x02);
        pic
    }

    #[test]
    fn cascaded_acknowledge_returns_slave_vector_and_nests_irq2() {
        let mut pic = init_pair();
        // IRQ8 (the RTC line) goes through the slave.
        pic.set_irq_line(8, true);
        // The controller offers the slave's vector, not a master vector.
        assert_eq!(pic.poll(), Some(SLAVE_BASE));
        assert_eq!(pic.acknowledge(), Some(SLAVE_BASE));
        // The slave's line and the master's IRQ2 are both in service.
        assert_eq!(pic.slave.inner.borrow().highest_in_service(), Some(0));
        assert_eq!(
            pic.master.inner.borrow().highest_in_service(),
            Some(CASCADE_IRQ)
        );
        // With IRQ2 in service, nothing lower on either chip is offered.
        assert_eq!(pic.poll(), None);
    }

    #[test]
    fn cascaded_irq15_uses_high_slave_vector() {
        let mut pic = init_pair();
        pic.set_irq_line(15, true);
        // IRQ15 is slave line 7 → vector 0x77.
        assert_eq!(pic.acknowledge(), Some(SLAVE_BASE + 7));
    }

    #[test]
    fn direct_cascade_input_without_slave_request_uses_defensive_ir7_vector() {
        let mut pic = init_pair();
        // The controller face permits driving IR2 directly, bypassing slave INT.
        pic.set_irq_line(CASCADE_IRQ, true);
        assert_eq!(pic.poll(), Some(SLAVE_BASE + 7));
        assert_eq!(pic.acknowledge(), Some(SLAVE_BASE + 7));
        assert_eq!(pic.slave.inner.borrow().isr(), 0);
        assert_eq!(pic.master.inner.borrow().isr(), 1 << CASCADE_IRQ);
        pic.end_of_interrupt();
        assert_eq!(pic.master.inner.borrow().isr(), 0);
        assert_eq!(pic.poll(), None);
    }

    #[test]
    fn single_end_of_interrupt_clears_both_chips_for_cascade() {
        let mut pic = init_pair();
        pic.set_irq_line(8, true);
        pic.acknowledge().unwrap();
        // One EOI ends the cascaded interrupt on both chips.
        pic.end_of_interrupt();
        assert_eq!(pic.master.inner.borrow().highest_in_service(), None);
        assert_eq!(pic.slave.inner.borrow().highest_in_service(), None);
    }

    #[test]
    fn cascade_outranks_lower_master_lines_but_not_higher() {
        let mut pic = init_pair();
        // A slave interrupt enters at IRQ2's priority: it outranks IRQ7 …
        pic.set_irq_line(7, true);
        pic.set_irq_line(8, true);
        assert_eq!(pic.poll(), Some(SLAVE_BASE));
        assert_eq!(pic.acknowledge(), Some(SLAVE_BASE));
        // … while the lower-priority master IRQ7 stays blocked underneath it.
        assert_eq!(pic.poll(), None);
        pic.end_of_interrupt();
        assert_eq!(pic.poll(), Some(MASTER_BASE + 7));
    }

    #[test]
    fn higher_master_line_outranks_the_cascade() {
        let mut pic = init_pair();
        // IRQ1 is higher priority than the IRQ2 cascade.
        pic.set_irq_line(1, true);
        pic.set_irq_line(8, true);
        assert_eq!(pic.poll(), Some(MASTER_BASE + 1));
        assert_eq!(pic.acknowledge(), Some(MASTER_BASE + 1));
        // Once IRQ1 is done, the cascaded slave interrupt surfaces.
        pic.end_of_interrupt();
        assert_eq!(pic.poll(), Some(SLAVE_BASE));
    }

    #[test]
    fn within_slave_priority_picks_lower_line_first() {
        let mut pic = init_pair();
        // IRQ10 (slave line 2) then IRQ8 (slave line 0); line 0 is higher.
        pic.set_irq_line(10, true);
        pic.set_irq_line(8, true);
        assert_eq!(pic.acknowledge(), Some(SLAVE_BASE));
        // IRQ8 in service blocks the lower slave line until EOI.
        assert_eq!(pic.poll(), None);
        pic.end_of_interrupt();
        assert_eq!(pic.poll(), Some(SLAVE_BASE + 2));
    }

    #[test]
    fn masked_slave_line_does_not_surface() {
        let mut pic = init_pair();
        // Mask IRQ8 (slave line 0) through the slave's data port.
        pic.slave_device().port_write(SLAVE_DATA, 1, 0x01).unwrap();
        pic.set_irq_line(8, true);
        // The slave latched the request but does not assert its INT, so the
        // master's IRQ2 never rises and nothing is offered.
        assert_eq!(pic.poll(), None);
        assert_eq!(pic.acknowledge(), Some(MASTER_BASE + 7));
        assert_eq!(pic.master.inner.borrow().highest_pending(), None);
    }

    /// Intel 8259A datasheet, Special Mask Mode, programmed through the bus
    /// face: with the mode on, a lower level is delivered past a masked level
    /// in service; with it off, it is not.
    #[test]
    fn special_mask_mode_on_mock_bus_delivers_past_a_masked_in_service_level() {
        for special_mask in [true, false] {
            let mut pic = SharedPic::new();
            let mut bus = testkit::MockBus::new(Box::new(pic.clone()));
            for (port, value) in [(0x20, 0x11), (0x21, 0x50), (0x21, 0x04), (0x21, 0x01)] {
                bus.port_write(port, 1, value).unwrap();
            }
            if special_mask {
                bus.port_write(MASTER_CMD, 1, 0x6B).unwrap();
            }
            pic.set_irq_line(3, true);
            assert_eq!(pic.acknowledge(), Some(0x53));
            bus.port_write(MASTER_DATA, 1, 0x08).unwrap();
            pic.set_irq_line(5, true);
            let expected = special_mask.then_some(0x55);
            assert_eq!(pic.poll(), expected);
            assert_eq!(bus.device().irq_pending(), special_mask);
        }
    }

    /// Slave line 0 in service and masked on the slave, its master IR2
    /// ended with a specific EOI, then slave line 3 requested.
    fn slave_request_under_masked_slave_service(slave_special_mask: bool) -> Option<u8> {
        let mut pic = init_pair();
        let mut master = pic.master_device();
        let mut slave = pic.slave_device();
        if slave_special_mask {
            slave.port_write(SLAVE_CMD, 1, 0x68).unwrap();
        }
        pic.set_irq_line(8, true);
        assert_eq!(pic.acknowledge(), Some(SLAVE_BASE));
        slave.port_write(SLAVE_DATA, 1, 0x01).unwrap();
        master.port_write(MASTER_CMD, 1, 0x62).unwrap();
        pic.set_irq_line(11, true);
        pic.poll()
    }

    /// Intel 8259A datasheet, Special Mask Mode and Cascade Mode: the slave's
    /// own special mask mode lets a lower slave level drive its INT past a
    /// masked slave level in service, once the master has ended IR2.
    #[test]
    fn slave_special_mask_mode_drives_the_cascade() {
        assert_eq!(
            slave_request_under_masked_slave_service(true),
            Some(SLAVE_BASE + 3)
        );
        assert_eq!(slave_request_under_masked_slave_service(false), None);
    }

    /// A slave request already pending behind an unmasked slave level in
    /// service: masking that level in special mask mode raises slave INT with
    /// no new request, through the bus face's cascade sync after the write,
    /// and the master delivers it (Intel 8259A datasheet, Special Mask Mode
    /// and Cascade Mode).
    #[test]
    fn masking_the_slave_level_in_service_releases_a_pending_slave_request() {
        let mut pic = init_pair();
        let mut master = pic.master_device();
        let mut slave = pic.slave_device();
        slave.port_write(SLAVE_CMD, 1, 0x68).unwrap();
        pic.set_irq_line(8, true);
        assert_eq!(pic.acknowledge(), Some(SLAVE_BASE));
        master.port_write(MASTER_CMD, 1, 0x62).unwrap();
        pic.set_irq_line(11, true);
        // Slave line 0 is in service and unmasked: line 3 waits, INT is low.
        assert!(!slave.irq_pending());
        assert_eq!(pic.poll(), None);
        let mut bus = testkit::MockBus::new(Box::new(slave.clone()));
        bus.port_write(SLAVE_DATA, 1, 0x01).unwrap();
        assert!(slave.irq_pending());
        assert_eq!(
            pic.master.inner.borrow().highest_pending(),
            Some(CASCADE_IRQ)
        );
        assert_eq!(pic.poll(), Some(SLAVE_BASE + 3));
        assert_eq!(pic.acknowledge(), Some(SLAVE_BASE + 3));
        assert_eq!(pic.slave.inner.borrow().isr(), 0x09);
    }

    /// While the master still has IR2 in service, a second slave request
    /// waits even with the slave in special mask mode: the master's nesting
    /// is its own.
    #[test]
    fn master_ir2_in_service_still_holds_a_special_mask_slave() {
        let mut pic = init_pair();
        let mut slave = pic.slave_device();
        slave.port_write(SLAVE_CMD, 1, 0x68).unwrap();
        pic.set_irq_line(8, true);
        pic.acknowledge().unwrap();
        slave.port_write(SLAVE_DATA, 1, 0x01).unwrap();
        pic.set_irq_line(11, true);
        assert!(slave.irq_pending());
        assert_eq!(pic.poll(), None);
    }

    /// In special mask mode the cascade wrapper's single EOI follows the
    /// master line a non-specific EOI ends, skipping a masked IR2.
    #[test]
    fn cascaded_eoi_follows_the_special_mask_non_specific_line() {
        let mut pic = init_pair();
        let mut master = pic.master_device();
        master.port_write(MASTER_CMD, 1, 0x68).unwrap();
        pic.set_irq_line(8, true);
        pic.acknowledge().unwrap();
        // Mask IR2 on the master: the cascade in service stops nesting.
        master.port_write(MASTER_DATA, 1, 0x04).unwrap();
        pic.set_irq_line(5, true);
        assert_eq!(pic.acknowledge(), Some(MASTER_BASE + 5));
        pic.end_of_interrupt();
        // IRQ5 ended; the masked cascade and its slave line stay in service.
        assert_eq!(pic.master.inner.borrow().isr(), 1 << CASCADE_IRQ);
        assert_eq!(pic.slave.inner.borrow().isr(), 0x01);
    }

    #[test]
    fn master_lines_are_unaffected_by_the_slave() {
        let mut pic = init_pair();
        // A plain master interrupt behaves exactly as the single-chip PIC does.
        pic.set_irq_line(3, true);
        assert_eq!(pic.poll(), Some(MASTER_BASE + 3));
        assert_eq!(pic.acknowledge(), Some(MASTER_BASE + 3));
        assert_eq!(pic.master.inner.borrow().highest_in_service(), Some(3));
        pic.end_of_interrupt();
        assert_eq!(pic.master.inner.borrow().highest_in_service(), None);
    }
}
