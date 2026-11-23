// SPDX-License-Identifier: AGPL-3.0-or-later
//! National Semiconductor PC16550D UART, Registers §8.1–8.8 and §8.11.
//!
//! Eight port-mapped registers, DLAB banking, a receive holding register or
//! 16-byte FIFO, receive/line-status interrupts and an optional serial endpoint.
//! PC16550D datasheet §8.3 (Programmable Baud Generator), §8.4 (Line Status),
//! §8.5 (FIFO Control), §8.6 (Interrupt Identification), §8.7 (Interrupt Enable)
//! and §8.8 (Modem Control) define these paths. Section 8.11, FIFO Interrupt
//! Mode Operation, defines the character timeout. The board gates IRQ with OUT2.
//! See `backlog/docs/design/spec.md` §7.2 and
//! `backlog/docs/design/observation.md` §UART output ownership and snapshot compatibility.
//!
//! | Function | Coverage |
//! | --- | --- |
//! | RX timing | Endpoint's baud/frame; each frame rounds up to a whole PIT clock |
//! | Guest divisor/LCR | 1.8432 MHz UART clock; mismatch yields byte 0 with FE (ASSUMED) |
//! | RX FIFO | 1/16 bytes; triggers 1/4/8/14; four guest-frame timeout |
//! | IRQ | RLS before RDA/timeout; IER and board OUT2 gate the line |
//! | TX | Instant sink delivery; no timed TX, THRE IRQ or TX FIFO |
//! | Modem | DTR/RTS notify endpoint; loopback disconnects external RX and forces modem outputs inactive |
//! | Internal loopback | TX-to-RX and modem-input feedback remain unmodelled; MSR stays zero |
//! | Snapshot | v4, RX/errors/deadlines and endpoint model/state; no sink/trace history |
//!
//! Mismatch is a deterministic byte-level approximation, not bit sampling:
//! the physical peripheral keeps its rate even if the guest changes the divisor.
//! RX validity is sampled when the start bit begins, after any startup wait.
//! The receiver checks only the first stop bit (PC16550D §8.1, LCR bit 2), so
//! selecting two stop bits accepts one-stop-bit peripheral frames. Without an
//! endpoint, TX retains its instantaneous output-only behaviour. Reset retains
//! attachment and sink; restore checks the endpoint model and effective modem
//! lines before changing state, and never re-emits historical output.
//!
//! Compatibility exception: back-to-back serial frames start at the previous
//! frame's rounded completion time, adding one rounded frame duration at a time.
//! This retains the existing byte-delivery cadence instead of carrying the baud
//! clock's fractional phase across frames. See `backlog/docs/conventions.md`.

use device::clock::{ClockDomain, Ratio, UART, UART_HZ};
use std::{any::Any, collections::VecDeque};

use device::{
    AccessDir, ByteOutput, ByteSink, ByteStreamKind, Device, DeviceError, NextEvent, TraceEvent,
    VClock,
};

/// Stable device name reported by [`Device::name`].
const NAME: &str = "16550-uart";

/// Line Status Register value reported: THRE (`0x20`) and TEMT (`0x40`) are
/// always set because transmit is instantaneous; RX adds DR and error bits.
const LSR_TX_READY: u32 = 0x60;

/// IIR bit 0 means no pending interrupt.
const IIR_NO_IRQ: u32 = 0x01;

/// Modem Status Register value reported: no modem lines are modelled.
const MSR_IDLE: u32 = 0x00;

/// DLAB bit within the Line Control Register (bit 7): when set, offsets +0/+1
/// select the divisor-latch view instead of THR/RBR and IER.
const LCR_DLAB: u8 = 0x80;

/// PC16550D §8.8: loopback disconnects SIN and forces modem output pins inactive.
const MCR_LOOPBACK: u8 = 0x10;

/// Snapshot layout version tag, first byte of [`Device::save`] output.
const SNAPSHOT_VERSION: u8 = 4;

/// Fixed snapshot header; receive bytes and endpoint model/state follow.
const SNAPSHOT_LEN: usize = 38;

/// Serial frame emitted by the far end, independent of guest UART programming.
/// PC16550D datasheet §8.1, Line Control Register; baud is in bits per second.
#[derive(Clone, Copy, Debug)]
pub struct SerialFormat {
    /// Peripheral bit rate, independent of the guest's divisor.
    pub baud: u32,
    /// Number of data bits following the start bit.
    pub data_bits: u8,
    /// Whether the peripheral includes a parity bit in each frame.
    pub parity: bool,
    /// Stop bits emitted by the peripheral; the receiver checks only the first.
    pub stop_bits: u8,
}

impl SerialFormat {
    fn clocks(self) -> u64 {
        let bits =
            1 + u64::from(self.data_bits) + u64::from(self.parity) + u64::from(self.stop_bits);
        ClockDomain::from_hz(Ratio::new(u64::from(self.baud.max(1)), 1)).ticks(bits)
    }
}

/// Far end of a UART line; no knowledge of ISA ports or IRQ wiring.
/// PC16550D datasheet §8.8, Modem Control Register; see
/// `backlog/docs/design/machine-files.md` §Serial mouse. The UART owns the
/// in-flight frame; implementations own protocol queues and startup readiness.
pub trait SerialPeripheral {
    /// Stable model marker; restore requires the same attached model.
    fn name(&self) -> &'static str;
    /// Physical transmit format used to time received frames.
    fn format(&self) -> SerialFormat;
    /// Observe effective DTR/RTS at `now`, after the UART's loopback masking.
    /// Return true to abort the in-flight frame (for example, on power loss).
    fn modem_control(&mut self, dtr: bool, rts: bool, now: VClock) -> bool;
    /// Earliest start time of pending output; None means no queued output.
    fn ready_at(&self) -> Option<VClock>;
    /// Take one byte at its start bit, after `ready_at`; None means no output.
    fn read_byte(&mut self) -> Option<u8>;
    /// Restore power-on protocol state while retaining the UART attachment.
    fn reset(&mut self);
    /// Save protocol queues, counters, deadlines and effective modem line state.
    fn save(&self) -> Vec<u8>;
    /// Restore protocol state only if valid and consistent with effective DTR/RTS.
    /// Reject without mutation on failure; restoring emits no bytes.
    fn restore(&mut self, data: &[u8], dtr: bool, rts: bool) -> Result<(), DeviceError>;
    /// Expose the endpoint to host input without a guest register transaction.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Clone, Copy)]
struct ReceiveFrame {
    byte: u8,
    bad: bool,
    at: VClock,
}

/// The 16550 UART device (§7.2).
///
/// Constructed with the COM port base address; decodes the eight ports
/// `base..=base+7`. Accepted THR writes emit directly to a bound sink without
/// retaining a second history. Capture limits and drains belong to the host;
/// binding or draining a sink does not change snapshot bytes.
/// See `backlog/docs/design/observation.md`, §Sinks, bounds and lifecycle.
/// When enabled, the bounded trace records port accesses, including divisor-latch
/// writes, so it must not be treated as a transmitted byte stream: the National Semiconductor
/// PC16550D datasheet, Line Control Register (DLAB), assigns offsets +0/+1 to
/// the divisor latches when DLAB is set.
pub struct Uart16550 {
    base: u16,
    /// Interrupt Enable Register (+1, DLAB=0). Receive and line-status interrupt enables.
    ier: u8,
    /// FIFO Control Register (+2 write). Receive FIFO mode and trigger level.
    fcr: u8,
    /// Line Control Register (+3). Bit 7 is DLAB.
    lcr: u8,
    /// Modem Control Register (+4). DTR/RTS endpoint power and OUT2 IRQ gate.
    mcr: u8,
    /// Scratch Register (+7). A byte of RAM; a device-presence probe target.
    scratch: u8,
    /// Divisor latch low (+0, DLAB=1). Determines receiver rate and FIFO timeout.
    dll: u8,
    /// Divisor latch high (+1, DLAB=1). Determines receiver rate and FIFO timeout.
    dlm: u8,
    /// Optional receiver; binding must not change saved state.
    sink: Option<Box<dyn ByteSink>>,
    /// Trace events observed since the last [`Device::take_trace`].
    trace: device::TraceBuffer,
    /// Virtual time of the most recent [`Device::tick`], stamped onto traces.
    now: VClock,
    serial: Option<Box<dyn SerialPeripheral>>,
    rx: VecDeque<(u8, u8)>,
    overrun: u8,
    last_rx: VClock,
    receiving: Option<ReceiveFrame>,
}

impl Uart16550 {
    /// Create a 16550 decoding the eight ports at `base..=base+7`.
    ///
    /// COM1 uses `base == 0x3F8`.
    ///
    /// # Panics
    /// Panics if `base > 0xFFF8`: eight registers must fit in the I/O space.
    /// This is a programmer-error guard; assembly validates user configuration
    /// before constructing the device.
    #[must_use]
    pub fn new(base: u16) -> Self {
        assert!(base <= 0xFFF8, "UART base must leave room for eight ports");
        Self {
            base,
            ier: 0,
            fcr: 0,
            lcr: 0,
            mcr: 0,
            scratch: 0,
            dll: 0,
            dlm: 0,
            sink: None,
            trace: device::TraceBuffer::default(),
            now: VClock::ZERO,
            serial: None,
            rx: VecDeque::new(),
            overrun: 0,
            last_rx: VClock::ZERO,
            receiving: None,
        }
    }

    /// Attach a serial-line peripheral before starting execution.
    #[must_use]
    pub fn with_serial_device(mut self, mut serial: Box<dyn SerialPeripheral>) -> Self {
        let outputs = Self::modem_outputs(self.mcr);
        serial.modem_control(outputs & 1 != 0, outputs & 2 != 0, self.now);
        self.serial = Some(serial);
        self
    }

    /// Host injection seam; the bus synchronizes this UART before mutable access.
    pub fn serial_device_mut(&mut self) -> Option<&mut (dyn SerialPeripheral + '_)> {
        match &mut self.serial {
            Some(serial) => Some(serial.as_mut()),
            None => None,
        }
    }

    /// Reset the endpoint without changing the UART's programmed registers.
    /// PC16550D datasheet §8.8, Modem Control Register: the existing DTR/RTS
    /// outputs still power the attached mouse after its protocol reset.
    pub fn reset_serial_device(&mut self) {
        let outputs = Self::modem_outputs(self.mcr);
        if let Some(serial) = &mut self.serial {
            serial.reset();
            serial.modem_control(outputs & 1 != 0, outputs & 2 != 0, self.now);
        }
    }

    /// Whether the serial endpoint and receive shift register have no pending output.
    pub fn serial_input_ready(&self) -> bool {
        self.receiving.is_none() && self.serial.as_ref().is_some_and(|s| s.ready_at().is_none())
    }

    fn modem_outputs(mcr: u8) -> u8 {
        if mcr & MCR_LOOPBACK != 0 {
            0
        } else {
            mcr & 0x0f
        }
    }

    fn trigger(&self) -> usize {
        if self.fcr & 1 == 0 {
            1
        } else {
            [1, 4, 8, 14][usize::from(self.fcr >> 6)]
        }
    }

    fn timeout_at(&self) -> Option<VClock> {
        if self.fcr & 1 == 0 || self.rx.is_empty() || self.rx.len() >= self.trigger() {
            return None;
        }
        let divisor = u64::from(u16::from_le_bytes([self.dll, self.dlm]));
        if divisor == 0 {
            return None;
        }
        let bits = 1
            + u64::from(5 + (self.lcr & 3))
            + u64::from(self.lcr & 8 != 0)
            + if self.lcr & 4 == 0 { 1 } else { 2 };
        let clocks = UART.ticks(4 * bits * divisor * 16);
        Some(VClock(self.last_rx.0.saturating_add(clocks)))
    }

    fn interrupt_id(&self) -> u32 {
        let error = self.overrun | self.rx.front().map_or(0, |(_, error)| *error);
        let id = if self.ier & 4 != 0 && error != 0 {
            6
        } else if self.ier & 1 != 0 && self.rx.len() >= self.trigger() {
            4
        } else if self.ier & 1 != 0 && self.timeout_at().is_some_and(|at| self.now >= at) {
            12
        } else {
            IIR_NO_IRQ
        };
        id | if self.fcr & 1 != 0 { 0xc0 } else { 0 }
    }

    fn receive(&mut self, frame: ReceiveFrame) {
        let capacity = if self.fcr & 1 != 0 { 16 } else { 1 };
        if self.rx.len() == capacity {
            self.overrun = 2;
            if capacity == 16 {
                return;
            }
            self.rx.pop_front();
        }
        self.rx.push_back((
            if frame.bad { 0 } else { frame.byte },
            if frame.bad { 8 } else { 0 },
        ));
        self.last_rx = frame.at;
    }

    fn next_receive_event(&self) -> Option<VClock> {
        if self.mcr & MCR_LOOPBACK != 0 {
            return None;
        }
        if let Some(frame) = self.receiving {
            return Some(frame.at);
        }
        let serial = self.serial.as_ref()?;
        let start = serial.ready_at()?.max(self.now);
        // A future start must be observed before later guest writes can alter LCR.
        Some(if start > self.now {
            start
        } else {
            VClock(start.0.saturating_add(serial.format().clocks()))
        })
    }

    /// Whether the DLAB bit is currently set in the LCR.
    fn dlab(&self) -> bool {
        self.lcr & LCR_DLAB != 0
    }

    /// Byte offset of `port` within this device's window, or an unmapped-port
    /// error if `port` is outside `base..=base+7`.
    fn offset(&self, port: u16) -> Result<u16, DeviceError> {
        if port < self.base || port > self.base + 7 {
            return Err(DeviceError::UnmappedRegister {
                kind: "port",
                offset: u64::from(port.wrapping_sub(self.base)),
            });
        }
        Ok(port - self.base)
    }

    /// Record a serviced port access on the tracing seam.
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

impl ByteOutput for Uart16550 {
    fn kind(&self) -> ByteStreamKind {
        ByteStreamKind::SerialTx
    }

    fn bind_sink(&mut self, sink: Box<dyn ByteSink>) {
        self.sink = Some(sink);
    }
}

impl Device for Uart16550 {
    fn byte_output(&mut self) -> Option<&mut dyn ByteOutput> {
        Some(self)
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
        let offset = self.offset(port)?;
        let value = match offset {
            0 if self.dlab() => u32::from(self.dll),
            0 => {
                let byte = self.rx.pop_front().map_or(0, |(byte, _)| byte);
                self.last_rx = self.now;
                u32::from(byte)
            }
            1 if self.dlab() => u32::from(self.dlm),
            1 => u32::from(self.ier),
            2 => self.interrupt_id(),
            3 => u32::from(self.lcr),
            4 => u32::from(self.mcr),
            5 => {
                let fifo_error = self.fcr & 1 != 0 && self.rx.iter().any(|(_, e)| *e != 0);
                let error = self.overrun
                    | self
                        .rx
                        .front_mut()
                        .map_or(0, |(_, error)| std::mem::take(error));
                self.overrun = 0;
                LSR_TX_READY
                    | u32::from(!self.rx.is_empty())
                    | u32::from(error)
                    | if fifo_error { 0x80 } else { 0 }
            }
            6 => MSR_IDLE,
            7 => u32::from(self.scratch),
            _ => unreachable!("offset is bounded to 0..=7 by Self::offset"),
        };
        self.trace_port(port, size, AccessDir::Read, value);
        Ok(value)
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
        let offset = self.offset(port)?;
        // Registers are byte-wide; operate on the low byte for wider access.
        let byte = value as u8;
        match offset {
            0 if self.dlab() => self.dll = byte,
            0 => {
                if let Some(sink) = &mut self.sink {
                    sink.push(byte);
                }
            }
            1 if self.dlab() => self.dlm = byte,
            1 => self.ier = byte,
            2 => {
                if (self.fcr ^ byte) & 1 != 0 || byte & 3 == 3 {
                    self.rx.clear();
                    self.overrun = 0;
                }
                self.fcr = byte & 0xc9;
            }
            3 => self.lcr = byte,
            4 => {
                self.mcr = byte;
                if byte & MCR_LOOPBACK != 0 {
                    self.receiving = None;
                }
                let outputs = Self::modem_outputs(byte);
                if let Some(serial) = &mut self.serial {
                    if serial.modem_control(outputs & 1 != 0, outputs & 2 != 0, self.now) {
                        self.receiving = None;
                    }
                }
            }
            5 | 6 => {} // LSR/MSR are read-only; writes are ignored on real hw.
            7 => self.scratch = byte,
            _ => unreachable!("offset is bounded to 0..=7 by Self::offset"),
        }
        self.trace_port(port, size, AccessDir::Write, value);
        Ok(())
    }

    fn mmio_read(&mut self, addr: u64, _size: u8) -> Result<u64, DeviceError> {
        // The 16550 is port-mapped on the PC/AT, never memory-mapped.
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
        debug_assert!(now >= self.now, "device clock moved backwards");
        if now < self.now {
            return;
        }
        if self.mcr & MCR_LOOPBACK != 0 {
            self.now = now;
            return;
        }
        loop {
            if self.receiving.is_none() {
                let divisor = u32::from(u16::from_le_bytes([self.dll, self.dlm]));
                let Some(serial) = &mut self.serial else {
                    break;
                };
                let Some(start) = serial.ready_at().map(|at| at.max(self.now)) else {
                    break;
                };
                if start > now {
                    break;
                }
                let format = serial.format();
                let at = VClock(start.0.saturating_add(format.clocks()));
                let valid = u64::from(divisor) * 16 * u64::from(format.baud)
                    == UART_HZ.numerator() / UART_HZ.denominator()
                    && self.lcr & 3 == format.data_bits.saturating_sub(5)
                    && (self.lcr & 8 != 0) == format.parity;
                let Some(byte) = serial.read_byte() else {
                    break;
                };
                self.receiving = Some(ReceiveFrame {
                    byte,
                    bad: !valid,
                    at,
                });
            }
            let Some(frame) = self.receiving.filter(|frame| frame.at <= now) else {
                break;
            };
            self.receiving = None;
            self.now = frame.at;
            self.receive(frame);
        }
        self.now = now;
    }

    fn next_event(&self, now: VClock) -> NextEvent {
        self.next_receive_event()
            .into_iter()
            .chain(self.timeout_at().filter(|at| *at > now))
            .min()
            .map_or(NextEvent::Dormant, NextEvent::At)
    }

    fn irq_pending(&self) -> bool {
        Self::modem_outputs(self.mcr) & 8 != 0 && self.interrupt_id() & 1 == 0
    }

    fn reset(&mut self) {
        let trace_policy = self.trace.policy();
        // Capture epochs belong to the host; a hardware reset keeps the route.
        let base = self.base;
        let sink = self.sink.take();
        let mut serial = self.serial.take();
        if let Some(serial) = &mut serial {
            serial.reset();
        }
        *self = Uart16550::new(base);
        self.sink = sink;
        self.serial = serial;
        self.trace.set_policy(trace_policy);
    }

    fn save(&self) -> Vec<u8> {
        let mut blob = Vec::with_capacity(SNAPSHOT_LEN);
        blob.push(SNAPSHOT_VERSION);
        blob.push(self.ier);
        blob.push(self.fcr);
        blob.push(self.lcr);
        blob.push(self.mcr);
        blob.push(self.scratch);
        blob.push(self.dll);
        blob.push(self.dlm);
        blob.extend_from_slice(&self.now.0.to_le_bytes());
        blob.push(self.rx.len() as u8);
        blob.push(self.overrun);
        blob.extend(self.last_rx.0.to_le_bytes());
        blob.push(u8::from(self.receiving.is_some()));
        blob.push(self.receiving.map_or(0, |f| f.byte));
        blob.push(u8::from(self.receiving.is_some_and(|f| f.bad)));
        blob.extend(self.receiving.map_or(0, |f| f.at.0).to_le_bytes());
        let model = self.serial.as_ref().map_or("", |serial| serial.name());
        blob.push(model.len() as u8);
        for (byte, error) in &self.rx {
            blob.extend([*byte, *error]);
        }
        blob.extend(model.as_bytes());
        if let Some(serial) = &self.serial {
            blob.extend(serial.save());
        }
        blob
    }

    fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError> {
        if snapshot
            .first()
            .is_some_and(|version| *version != SNAPSHOT_VERSION)
        {
            return Err(DeviceError::InvalidSnapshot {
                device: NAME,
                reason: "unsupported snapshot version",
            });
        }
        if snapshot.len() < SNAPSHOT_LEN {
            return Err(DeviceError::InvalidSnapshot {
                device: NAME,
                reason: "snapshot shorter than fixed layout",
            });
        }
        let ier = snapshot[1];
        let fcr = snapshot[2];
        let lcr = snapshot[3];
        let mcr = snapshot[4];
        let scratch = snapshot[5];
        let dll = snapshot[6];
        let dlm = snapshot[7];
        let invalid = || DeviceError::InvalidSnapshot {
            device: NAME,
            reason: "invalid receiver or endpoint state",
        };
        let count = usize::from(snapshot[16]);
        let model_start = SNAPSHOT_LEN + count * 2;
        let model_end = model_start + usize::from(snapshot[37]);
        let now = u64::from_le_bytes(std::array::from_fn(|i| snapshot[8 + i]));
        let last_rx = u64::from_le_bytes(std::array::from_fn(|i| snapshot[18 + i]));
        let at = u64::from_le_bytes(std::array::from_fn(|i| snapshot[29 + i]));
        if count > (if fcr & 1 != 0 { 16 } else { 1 })
            || snapshot.len() < model_end
            || ![0, 2].contains(&snapshot[17])
            || snapshot[26] > 1
            || snapshot[28] > 1
            || last_rx > now
            || (snapshot[26] != 0 && (at <= now || mcr & MCR_LOOPBACK != 0))
            || (snapshot[26] == 0 && snapshot[27..37].iter().any(|b| *b != 0))
        {
            return Err(invalid());
        }
        let mut rx = VecDeque::new();
        for pair in snapshot[SNAPSHOT_LEN..model_start].as_chunks::<2>().0 {
            if ![0, 8].contains(&pair[1]) {
                return Err(invalid());
            }
            rx.push_back((pair[0], pair[1]));
        }
        let model = self.serial.as_ref().map_or("", |s| s.name());
        if snapshot[model_start..model_end] != *model.as_bytes() {
            return Err(invalid());
        }
        if let Some(serial) = &mut self.serial {
            let outputs = Self::modem_outputs(mcr);
            serial.restore(&snapshot[model_end..], outputs & 1 != 0, outputs & 2 != 0)?;
        } else if snapshot.len() != model_end || snapshot[26] != 0 {
            return Err(invalid());
        }
        self.rx = rx;
        self.overrun = snapshot[17];
        self.last_rx = VClock(last_rx);
        self.receiving = (snapshot[26] != 0).then_some(ReceiveFrame {
            byte: snapshot[27],
            bad: snapshot[28] != 0,
            at: VClock(at),
        });
        self.ier = ier;
        self.fcr = fcr;
        self.lcr = lcr;
        self.mcr = mcr;
        self.scratch = scratch;
        self.dll = dll;
        self.dlm = dlm;
        self.now = VClock(u64::from_le_bytes(std::array::from_fn(|i| snapshot[8 + i])));
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

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testkit::MockBus;

    #[test]
    #[should_panic(expected = "UART base must leave room for eight ports")]
    fn fidelity_rejects_wrapping_port_window() {
        let _bus = MockBus::new(Box::new(Uart16550::new(0xFFF9)));
    }

    #[test]
    fn fidelity_last_valid_port_window_includes_scratch() {
        let mut bus = MockBus::new(Box::new(Uart16550::new(0xFFF8)));
        bus.port_write(0xFFFF, 1, 0x5A).unwrap();
        assert_eq!(bus.port_read(0xFFFF, 1).unwrap(), 0x5A);
    }

    const COM1: u16 = 0x3F8;

    fn capture_output(device: &mut dyn Device) -> device::ByteCapture {
        let capture = device::ByteCapture::new(16);
        device
            .byte_output()
            .unwrap()
            .bind_sink(Box::new(capture.clone()));
        capture
    }

    #[test]
    fn endpoint_reset_releases_buttons_and_preserves_powered_uart_state() {
        let mut bus = MockBus::new(Box::new(
            Uart16550::new(COM1).with_serial_device(Box::new(crate::SerialMouse::new())),
        ));
        bus.port_write(COM1 + 4, 1, 3).unwrap();
        bus.port_write(COM1 + 7, 1, 0x5a).unwrap();
        let uart = bus
            .device_mut()
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Uart16550>()
            .unwrap();
        let mouse = uart.serial_device_mut().unwrap();
        mouse
            .as_any_mut()
            .downcast_mut::<crate::SerialMouse>()
            .unwrap()
            .inject_motion(0, 0, 1);
        assert_eq!(mouse.save()[3], 1);
        uart.reset_serial_device();
        let mouse = uart.serial_device_mut().unwrap();
        assert_eq!(mouse.save()[3], 0);
        assert_eq!(mouse.save()[1..3], [1, 1]);
        assert_eq!(mouse.read_byte(), Some(b'M'));
        let snapshot = bus.device().save();
        bus.device_mut().restore(&snapshot).unwrap();
        assert_eq!(bus.port_read(COM1 + 4, 1).unwrap(), 3);
        assert_eq!(bus.port_read(COM1 + 7, 1).unwrap(), 0x5a);
    }

    #[test]
    fn snapshot_is_independent_of_sink_binding_and_capture_history() {
        let mut plain = MockBus::new(Box::new(Uart16550::new(COM1)));
        let mut routed = MockBus::new(Box::new(Uart16550::new(COM1)));
        let capture = device::ByteCapture::new(3);
        routed
            .device_mut()
            .byte_output()
            .unwrap()
            .bind_sink(Box::new(capture.clone()));
        for bus in [&mut plain, &mut routed] {
            bus.port_write(COM1 + 3, 1, 0x80).unwrap();
            bus.port_write(COM1, 1, 12).unwrap();
            bus.port_write(COM1 + 1, 1, 1).unwrap();
            bus.port_write(COM1 + 3, 1, 3).unwrap();
            for byte in [0, 0xFF, 0xFF, 42] {
                bus.port_write(COM1, 1, byte).unwrap();
            }
        }
        let snapshot = plain.device().save();
        assert_eq!(routed.device().save(), snapshot);
        assert_eq!(snapshot[0], SNAPSHOT_VERSION);
        assert_eq!(snapshot.len(), SNAPSHOT_LEN);
        assert_eq!(capture.drain(), Err(device::ByteOverflow { limit: 3 }));
        assert_eq!(routed.device().save(), snapshot);
        capture.start_epoch();
        for bus in [&mut plain, &mut routed] {
            bus.device_mut().reset();
            bus.device_mut().restore(&snapshot).unwrap();
            assert_eq!(bus.device().save(), snapshot);
        }
        assert!(
            capture.drain().unwrap().is_empty(),
            "restore must not emit history"
        );
        for bus in [&mut plain, &mut routed] {
            bus.port_write(COM1, 1, 7).unwrap();
        }
        assert_eq!(capture.drain().unwrap(), [7]);
        assert_eq!(routed.device().save(), plain.device().save());
    }

    #[test]
    fn observation_sink_receives_only_accepted_thr_bytes_and_survives_reset() {
        let capture = device::ByteCapture::new(4);
        let mut bus = MockBus::new(Box::new(crate::trace_policy_tests::traced(Uart16550::new(
            COM1,
        ))));
        let output = bus.device_mut().byte_output().unwrap();
        assert_eq!(output.kind(), device::ByteStreamKind::SerialTx);
        output.bind_sink(Box::new(capture.clone()));
        bus.port_write(COM1, 2, 0x1241).unwrap();
        bus.port_write(COM1 + 3, 1, 0x80).unwrap();
        bus.port_write(COM1, 1, 0xFF).unwrap();
        bus.port_write(COM1 + 1, 1, 0x12).unwrap();
        bus.port_read(COM1, 1).unwrap();
        assert!(bus.port_write(COM1 - 1, 1, 0x42).is_err());
        let snapshot = bus.device().save();
        assert_eq!(snapshot.len(), SNAPSHOT_LEN);
        bus.device_mut().restore(&snapshot).unwrap();
        bus.device_mut().take_trace();
        assert_eq!(capture.drain().unwrap(), b"A");
        bus.device_mut().reset();
        for byte in [0, 0xFF, 0xFF] {
            bus.port_write(COM1, 1, byte).unwrap();
        }
        assert_eq!(capture.drain().unwrap(), [0, 0xFF, 0xFF]);
        assert!(capture.drain().unwrap().is_empty());
    }

    #[test]
    fn dormant_matches_per_clock_seeded_accesses_and_long_jumps() {
        let mut reference = MockBus::new(Box::new(crate::trace_policy_tests::traced(
            Uart16550::new(0x3F8),
        )));
        let mut scheduled = MockBus::new(Box::new(crate::trace_policy_tests::traced(
            Uart16550::new(0x3F8),
        )));
        let mut seed = 0x8042_1468_1816_5501u64;
        for step in 0..128 {
            assert_eq!(
                scheduled.device().next_event(reference.clock()),
                device::NextEvent::Dormant
            );
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let clocks = if step % 31 == 0 {
                100_003
            } else {
                (seed >> 32) % 17
            };
            for _ in 0..clocks {
                reference.tick(1);
                assert!(!reference.device().irq_pending());
            }
            scheduled.device_mut().sync(reference.clock());
            assert_eq!(reference.device().save(), scheduled.device().save());
            assert_eq!(
                reference.device().irq_pending(),
                scheduled.device().irq_pending()
            );
            let port = 0x3F8 + ((seed >> 8) % 8) as u16;
            if seed & 1 == 0 {
                reference.port_write(port, 1, seed as u32).unwrap();
                scheduled.port_write(port, 1, seed as u32).unwrap();
            } else {
                assert_eq!(
                    reference.port_read(port, 1).unwrap(),
                    scheduled.port_read(port, 1).unwrap()
                );
            }
            assert_eq!(reference.device().save(), scheduled.device().save());
            assert_eq!(
                reference.device().irq_pending(),
                scheduled.device().irq_pending()
            );
            assert_eq!(
                reference.device_mut().take_trace(),
                scheduled.device_mut().take_trace()
            );
        }
    }

    #[test]
    fn thr_write_emits_to_sink() {
        let mut uart = Uart16550::new(COM1);
        let capture = capture_output(&mut uart);
        uart.port_write(COM1, 1, u32::from(b'A')).unwrap();
        uart.port_write(COM1, 1, u32::from(b'B')).unwrap();
        assert_eq!(capture.drain().unwrap(), b"AB");
        assert!(capture.drain().unwrap().is_empty());
    }

    #[test]
    fn lsr_reports_thr_empty_no_data() {
        let mut uart = Uart16550::new(COM1);
        assert_eq!(uart.port_read(COM1 + 5, 1).unwrap(), 0x60);
    }

    #[test]
    fn dlab_banks_divisor_latch_then_restores_thr() {
        let mut uart = Uart16550::new(COM1);
        let capture = capture_output(&mut uart);
        // Set DLAB (LCR bit 7) at +3.
        uart.port_write(COM1 + 3, 1, u32::from(LCR_DLAB)).unwrap();
        // With DLAB set, +0/+1 are DLL/DLM.
        uart.port_write(COM1, 1, 0x0C).unwrap();
        uart.port_write(COM1 + 1, 1, 0x01).unwrap();
        assert_eq!(uart.port_read(COM1, 1).unwrap(), 0x0C);
        assert_eq!(uart.port_read(COM1 + 1, 1).unwrap(), 0x01);
        // Clear DLAB: +0 is THR again, so a write transmits rather than banking.
        uart.port_write(COM1 + 3, 1, 0).unwrap();
        uart.port_write(COM1, 1, u32::from(b'Z')).unwrap();
        assert_eq!(capture.drain().unwrap(), b"Z");
        // Divisor latch retained its value.
        assert_eq!(uart.port_read(COM1 + 3, 1).unwrap(), 0);
    }

    #[test]
    fn mock_bus_output_excludes_divisor_writes() {
        let mut bus = MockBus::new(Box::new(crate::trace_policy_tests::traced(Uart16550::new(
            COM1,
        ))));
        let capture = capture_output(bus.device_mut());
        bus.port_write(COM1 + 3, 1, 0x83).unwrap();
        bus.port_write(COM1, 1, 0x42).unwrap();
        bus.port_write(COM1 + 1, 1, 0x01).unwrap();
        assert_eq!(bus.port_read(COM1, 1).unwrap(), 0x42);
        assert_eq!(bus.port_read(COM1 + 1, 1).unwrap(), 0x01);
        bus.port_write(COM1 + 3, 1, 0x03).unwrap();
        for &byte in b"OK\0\xff" {
            bus.port_write(COM1, 1, u32::from(byte)).unwrap();
        }

        assert_eq!(capture.drain().unwrap(), b"OK\0\xff");
        assert!(capture.drain().unwrap().is_empty());
        assert!(bus.device_mut().take_trace().iter().any(|event| matches!(
            event,
            TraceEvent::Port {
                port: COM1,
                dir: AccessDir::Write,
                value: 0x42,
                ..
            }
        )));
    }

    #[test]
    fn scratch_register_round_trips() {
        let mut uart = Uart16550::new(COM1);
        uart.port_write(COM1 + 7, 1, 0x5A).unwrap();
        assert_eq!(uart.port_read(COM1 + 7, 1).unwrap(), 0x5A);
    }

    #[test]
    fn save_restore_round_trip_preserves_state() {
        let mut uart = Uart16550::new(COM1);
        let capture = capture_output(&mut uart);
        uart.port_write(COM1 + 3, 1, u32::from(LCR_DLAB)).unwrap();
        uart.port_write(COM1, 1, 0x03).unwrap(); // DLL
        uart.port_write(COM1 + 1, 1, 0x00).unwrap(); // DLM
        uart.port_write(COM1 + 3, 1, 0).unwrap();
        uart.port_write(COM1 + 4, 1, 0x0B).unwrap(); // MCR
        uart.port_write(COM1 + 7, 1, 0x7E).unwrap(); // scratch
        uart.port_write(COM1, 1, u32::from(b'h')).unwrap();
        uart.port_write(COM1, 1, u32::from(b'i')).unwrap();

        let blob = uart.save();
        let mut restored = Uart16550::new(COM1);
        let restored_capture = capture_output(&mut restored);
        restored.restore(&blob).unwrap();

        assert_eq!(capture.drain().unwrap(), b"hi");
        assert!(restored_capture.drain().unwrap().is_empty());
        assert_eq!(restored.save(), blob);
        restored.port_write(COM1, 1, u32::from(b'!')).unwrap();
        assert_eq!(restored_capture.drain().unwrap(), b"!");
        assert_eq!(restored.port_read(COM1 + 4, 1).unwrap(), 0x0B);
        assert_eq!(restored.port_read(COM1 + 7, 1).unwrap(), 0x7E);
        // DLL survives (read it back through the DLAB view).
        restored
            .port_write(COM1 + 3, 1, u32::from(LCR_DLAB))
            .unwrap();
        assert_eq!(restored.port_read(COM1, 1).unwrap(), 0x03);
    }

    #[test]
    fn restore_truncated_snapshot_is_rejected() {
        let mut uart = Uart16550::new(COM1);
        let err = uart.restore(&[SNAPSHOT_VERSION, 0, 0]).unwrap_err();
        assert_eq!(
            err,
            DeviceError::InvalidSnapshot {
                device: NAME,
                reason: "snapshot shorter than fixed layout",
            }
        );
    }

    #[test]
    fn mmio_is_unmapped() {
        let mut uart = Uart16550::new(COM1);
        assert_eq!(
            uart.mmio_read(0x1000, 1).unwrap_err(),
            DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: 0x1000,
            }
        );
        assert_eq!(
            uart.mmio_write(0x1000, 1, 0).unwrap_err(),
            DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: 0x1000,
            }
        );
    }

    #[test]
    fn port_outside_window_is_unmapped() {
        let mut uart = Uart16550::new(COM1);
        let err = uart.port_read(COM1 + 8, 1).unwrap_err();
        assert_eq!(
            err,
            DeviceError::UnmappedRegister {
                kind: "port",
                offset: 8,
            }
        );
    }

    #[test]
    fn no_irq_is_asserted() {
        let uart = Uart16550::new(COM1);
        assert!(!uart.irq_pending());
    }

    #[test]
    fn reset_clears_registers_and_preserves_host_output() {
        let mut uart = Uart16550::new(COM1);
        let capture = capture_output(&mut uart);
        uart.port_write(COM1, 1, u32::from(b'x')).unwrap();
        uart.port_write(COM1 + 7, 1, 0x42).unwrap();
        uart.reset();
        assert_eq!(capture.drain().unwrap(), b"x");
        assert_eq!(uart.port_read(COM1 + 7, 1).unwrap(), 0);
    }

    #[test]
    fn hello_via_output_and_trace() {
        let mut uart = crate::trace_policy_tests::traced(Uart16550::new(COM1));
        let capture = capture_output(&mut uart);
        for &byte in b"hello" {
            uart.port_write(COM1, 1, u32::from(byte)).unwrap();
        }
        assert_eq!(capture.drain().unwrap(), b"hello");

        // The tracing seam must yield one Port write to `base` per THR byte.
        let writes: Vec<u32> = uart
            .take_trace()
            .into_iter()
            .filter_map(|ev| match ev {
                TraceEvent::Port {
                    port,
                    dir: AccessDir::Write,
                    value,
                    ..
                } if port == COM1 => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(
            writes,
            b"hello".iter().map(|&b| u32::from(b)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mock_bus_isolation_harness() {
        let mut uart = Uart16550::new(COM1);
        let capture = capture_output(&mut uart);
        let mut bus = MockBus::new(Box::new(uart));
        bus.tick(3);
        for &byte in b"hello" {
            bus.port_write(COM1, 1, u32::from(byte)).unwrap();
        }
        assert_eq!(bus.port_read(COM1 + 5, 1).unwrap(), 0x60);

        assert_eq!(capture.drain().unwrap(), b"hello");
        assert!(bus.device_mut().take_trace().is_empty());
    }
}

#[cfg(test)]
mod clock_domain_regressions {
    use super::*;

    use testkit::MockBus;
    #[test]
    fn clock_domain_uart_frames_and_fifo_timeout_use_exact_rates() {
        let mut bus = MockBus::new(Box::new(Uart16550::new(0x3f8)));
        let uart: &mut Uart16550 = bus
            .device_mut()
            .as_any_mut()
            .unwrap()
            .downcast_mut()
            .unwrap();
        uart.fcr = 0x41;
        uart.dll = 0xff;
        uart.dlm = 0xff;
        uart.lcr = 3;
        uart.rx.push_back((0x41, 0));
        let expected = UART.ticks(4 * 10 * 65535 * 16);
        assert_eq!(
            bus.device().next_event(bus.clock()),
            NextEvent::At(VClock(expected))
        );
        assert_eq!(
            SerialFormat {
                baud: 1,
                data_bits: 8,
                parity: false,
                stop_bits: 1
            }
            .clocks(),
            ClockDomain::from_hz(Ratio::new(1, 1)).ticks(10)
        );
    }
}
