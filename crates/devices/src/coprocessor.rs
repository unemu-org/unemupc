// SPDX-License-Identifier: AGPL-3.0-or-later
//! IBM AT coprocessor socket and motherboard error latch.
//!
//! IBM Personal Computer AT Technical Reference (September 1985), System
//! Board, I/O Address Map (p. 1-37), NMI and Coprocessor Controls (p. 1-38),
//! System Interrupts, and Section 2, Coprocessor: ERROR# feeds IRQ13 and an
//! F0h write clears latched BUSY. The CPU supplies assertion events; the board
//! holds IRQ13 until any F0h write, independently of x87 ES. Reads float to FFh.
//! F1h reset and CPU-internal F8h–FFh coprocessor transfers are not modelled;
//! their reads float and writes are ignored. F2h–F7h are not decoded. With an
//! empty socket every output remains inactive. See
//! `backlog/docs/design/machine-config.md` §Coprocessor controls.

use std::{cell::Cell, rc::Rc};

use device::{AccessDir, Device, DeviceError, NextEvent, TraceEvent, VClock};

/// Shared AT board latch, separate from the coprocessor's sticky status.
#[derive(Clone, Default)]
pub struct CoprocessorErrorLine {
    irq: Rc<Cell<bool>>,
    acknowledged: Rc<Cell<bool>>,
}

impl CoprocessorErrorLine {
    /// Latch an ERROR# assertion until a guest F0h write.
    pub fn assert(&self) {
        self.irq.set(true);
    }

    /// Consume a board acknowledgement so assembly can release CPU BUSY.
    pub fn take_acknowledgement(&self) -> bool {
        self.acknowledged.replace(false)
    }
}

/// AT motherboard controls, optionally connected to a populated socket.
#[derive(Default)]
pub struct AtCoprocessor {
    now: VClock,
    trace: device::TraceBuffer,
    error: Option<CoprocessorErrorLine>,
}

/// Compatibility name for the empty-socket constructor.
pub type Absent80287 = AtCoprocessor;

impl AtCoprocessor {
    /// Create an empty socket with inactive BUSY/error outputs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Connect a present coprocessor's external error output to board controls.
    pub fn with_error_line(error: CoprocessorErrorLine) -> Self {
        Self {
            error: Some(error),
            ..Self::default()
        }
    }

    fn check(port: u16) -> Result<(), DeviceError> {
        if matches!(port, 0xF0 | 0xF1 | 0xF8..=0xFF) {
            Ok(())
        } else {
            Err(DeviceError::UnmappedRegister {
                kind: "port",
                offset: u64::from(port),
            })
        }
    }

    fn trace_port(&mut self, port: u16, size: u8, dir: AccessDir, value: u32) {
        self.trace.push(TraceEvent::Port {
            at: self.now,
            port,
            size,
            dir,
            value,
        });
    }
}

impl Device for AtCoprocessor {
    fn name(&self) -> &'static str {
        if self.error.is_some() {
            "at-coprocessor"
        } else {
            "absent-80287"
        }
    }
    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
        Self::check(port)?;
        self.trace_port(port, size, AccessDir::Read, 0xFF);
        Ok(0xFF)
    }
    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
        Self::check(port)?;
        if port == 0xF0 {
            if let Some(error) = &self.error {
                error.irq.set(false);
                error.acknowledged.set(true);
            }
        }
        self.trace_port(port, size, AccessDir::Write, value);
        Ok(())
    }
    fn mmio_read(&mut self, addr: u64, _: u8) -> Result<u64, DeviceError> {
        Err(DeviceError::UnmappedRegister {
            kind: "mmio",
            offset: addr,
        })
    }
    fn mmio_write(&mut self, addr: u64, _: u8, _: u64) -> Result<(), DeviceError> {
        Err(DeviceError::UnmappedRegister {
            kind: "mmio",
            offset: addr,
        })
    }
    fn tick(&mut self, now: VClock) {
        debug_assert!(now >= self.now, "device clock moved backwards");
        if now >= self.now {
            self.now = now;
        }
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
        self.error.as_ref().is_some_and(|error| error.irq.get())
    }
    fn reset(&mut self) {
        self.now = VClock::ZERO;
        self.trace.clear();
        if let Some(error) = &self.error {
            error.irq.set(false);
            error.acknowledged.set(false);
        }
    }
    fn save(&self) -> Vec<u8> {
        let mut blob = vec![2];
        blob.extend_from_slice(&self.now.0.to_le_bytes());
        blob.push(u8::from(self.irq_pending()));
        blob.push(u8::from(
            self.error.as_ref().is_some_and(|e| e.acknowledged.get()),
        ));
        blob
    }
    fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError> {
        if snapshot.len() != 11
            || snapshot[0] != 2
            || snapshot[9] > 1
            || snapshot[10] > 1
            || (self.error.is_none() && snapshot[9..].iter().any(|&b| b != 0))
        {
            return Err(DeviceError::InvalidSnapshot {
                device: "absent-80287",
                reason: "invalid snapshot",
            });
        }
        self.now = VClock(u64::from_le_bytes(std::array::from_fn(|i| snapshot[i + 1])));
        if let Some(error) = &self.error {
            error.irq.set(snapshot[9] != 0);
            error.acknowledged.set(snapshot[10] != 0);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use testkit::MockBus;

    #[test]
    fn populated_socket_latches_irq_until_any_f0_write_and_rearms() {
        let error = CoprocessorErrorLine::default();
        let mut bus = MockBus::new(Box::new(AtCoprocessor::with_error_line(error.clone())));
        for value in [0, 0x55, 0xff] {
            error.assert();
            assert!(bus.device().irq_pending());
            for port in [0xf1, 0xf8, 0xff] {
                bus.port_write(port, 1, value).unwrap();
                assert_eq!(bus.port_read(port, 1).unwrap(), 0xff);
                assert!(bus.device().irq_pending());
                assert!(!error.take_acknowledgement());
            }
            bus.tick(10);
            assert!(bus.device().irq_pending());
            bus.port_write(0xf0, 1, value).unwrap();
            assert!(!bus.device().irq_pending());
            assert!(error.take_acknowledgement());
            assert!(!error.take_acknowledgement());
            bus.port_write(0xf0, 1, value).unwrap();
            assert!(error.take_acknowledgement());
        }
    }

    #[test]
    fn populated_socket_snapshot_and_reset_preserve_the_board_latch_contract() {
        let error = CoprocessorErrorLine::default();
        let mut bus = MockBus::new(Box::new(AtCoprocessor::with_error_line(error.clone())));
        error.assert();
        let latched = bus.device().save();
        bus.port_write(0xf0, 1, 0).unwrap();
        let acknowledged = bus.device().save();
        bus.device_mut().restore(&latched).unwrap();
        assert!(bus.device().irq_pending());
        assert!(!error.take_acknowledgement());
        bus.device_mut().restore(&acknowledged).unwrap();
        assert!(!bus.device().irq_pending());
        assert!(error.take_acknowledgement());
        error.assert();
        bus.device_mut().reset();
        assert!(!bus.device().irq_pending());
        assert!(!error.take_acknowledgement());
    }

    #[test]
    fn empty_socket_reads_open_bus_discards_writes_and_never_interrupts() {
        let mut bus = MockBus::new(Box::new(Absent80287::new()));
        for port in [0xF0, 0xF1, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, 0xFF] {
            for value in [0, 0x55, 0xFF] {
                bus.port_write(port, 1, value).unwrap();
                bus.tick(1);
                assert_eq!(bus.port_read(port, 1).unwrap(), 0xFF);
                assert!(!bus.device().irq_pending());
            }
        }
    }

    #[test]
    fn unassigned_ports_and_memory_are_not_decoded() {
        let mut bus = MockBus::new(Box::new(Absent80287::new()));
        for port in [0xEF, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0x100] {
            assert!(matches!(
                bus.port_read(port, 1),
                Err(DeviceError::UnmappedRegister { .. })
            ));
            assert!(matches!(
                bus.port_write(port, 1, 0),
                Err(DeviceError::UnmappedRegister { .. })
            ));
        }
        assert!(bus.mmio_read(0xF0, 1).is_err());
        assert!(bus.mmio_write(0xF0, 1, 0).is_err());
    }
}
