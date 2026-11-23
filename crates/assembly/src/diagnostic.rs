// SPDX-License-Identifier: AGPL-3.0-or-later
//! The diagnostic report of a running machine (§3.1).
//!
//! A guest can stop responding without an error: interrupts disabled around
//! a HLT or a loop, the keyboard interrupt masked in the 8259, the keyboard
//! interface disabled in the 8042. The [CPU stop
//! report](cpu::stop_report) exists only when the CPU core refused
//! something. [`MachineDiagnostic`] is the same kind of picture of a machine
//! that still runs, taken by the host at a completed run boundary
//! ([`crate::AssembledMachine::diagnostic_report`]): the retired count,
//! virtual time and status; the processor ([`cpu::CpuDiagnostic`]); the
//! machine's code trail (`machine::CodeTrail`); the last exceptions and
//! interrupts the guest took (`machine::delivery_tap`); the master and slave 8259;
//! the 8042; the three 8254 counters; the A20 line and its sources; the
//! CMOS shutdown byte and NMI mask; and the VGA registers and colour path.
//! [`std::fmt::Display`] renders it as plain text in a fixed order. The format
//! is described in
//! `backlog/docs/design/observation.md` §Diagnostic report of a running
//! machine.
//!
//! Taking a report performs no guest port or memory-mapped access. Guest
//! memory is read from RAM and ROM only. The PIC is read from its snapshot
//! image. The 8042, PIT, RTC and VGA are borrowed through the bus, which
//! first brings each to the bus clock, exactly as the settle at the start of
//! the next run would; their accessors read fields and change nothing.

use crate::{AssembledMachine, DeviceId};
use devices::{CmosDiagnostic, I8042Diagnostic, PicDiagnostic, PitDiagnostic, VgaDiagnostic};
use machine::{RecentDeliveries, TrailEntry};
use std::fmt;

/// The A20 line and its sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct A20Diagnostic {
    /// The gate the CPU applies to its addresses.
    pub cpu: bool,
    /// The 8042 output port's A20 bit.
    pub keyboard: bool,
    /// Port 92h bit 1; `None` when the board has no System Control Port A.
    pub port_92: Option<bool>,
    /// The CPU's A20 is held open by the reset-fetch window after a CPU
    /// reset (`machine::Machine::with_reset_fetch_probe`).
    pub reset_window: bool,
}

/// The diagnostic report of a running machine; see the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineDiagnostic {
    /// The machine's status as its front end names it.
    pub status: String,
    /// Virtual time in instruction-equivalent units.
    pub virtual_time: u64,
    /// The device clock, in virtual ticks.
    pub clock: u64,
    /// Whether the CPU sits in HLT.
    pub halted: bool,
    /// The processor.
    pub cpu: cpu::CpuDiagnostic,
    /// The code trail, oldest entry first; `None` when it was not enabled.
    pub trail: Option<Vec<TrailEntry>>,
    /// Samples the trail took, folded ones included.
    pub trail_samples: u64,
    /// The last exceptions and interrupts the guest took; `None` when the
    /// delivery log was not enabled.
    pub deliveries: Option<RecentDeliveries>,
    /// The master 8259.
    pub master_pic: Option<PicDiagnostic>,
    /// The slave 8259.
    pub slave_pic: Option<PicDiagnostic>,
    /// The 8042.
    pub keyboard: Option<I8042Diagnostic>,
    /// The 8254.
    pub pit: Option<PitDiagnostic>,
    /// The A20 line; `None` on a board without an A20 source.
    pub a20: Option<A20Diagnostic>,
    /// The RTC's CMOS state.
    pub cmos: Option<CmosDiagnostic>,
    /// Register-level VGA state; absent for other display devices.
    pub vga: Option<VgaDiagnostic>,
}

impl AssembledMachine {
    /// Keep the last exceptions and interrupts the guest takes for
    /// [`Self::diagnostic_report`] (`machine::Machine::enable_delivery_log`),
    /// with each IRQ named by its 8259 input. Retired counts and every
    /// guest effect are unchanged.
    pub fn enable_delivery_log(&mut self) -> Result<(), cpu::DeliveryLogError> {
        self.name_irq_inputs();
        self.machine.enable_delivery_log()
    }

    /// Start recording every exception and interrupt the guest takes
    /// (`machine::DeliveryRecording`), with each IRQ named by its 8259
    /// input; its text comes from `machine.delivery_recording()` and
    /// `machine.stop_delivery_recording()`. Retired counts and every guest
    /// effect are unchanged.
    pub fn start_delivery_recording(&mut self) -> Result<(), cpu::DeliveryLogError> {
        self.name_irq_inputs();
        self.machine.start_delivery_recording()
    }

    /// Name an IRQ vector's input from the vector bases the 8259s hold,
    /// read from their snapshot images once per run slice that delivered
    /// an IRQ: the master's eight vectors are IRQ 0 to 7, the slave's IRQ 8
    /// to 15 (Intel 8259A datasheet, ICW2; IBM Personal Computer AT
    /// Technical Reference, System Board, Interrupts).
    fn name_irq_inputs(&mut self) {
        let master = self.device_index.get(&DeviceId::MasterPic).copied();
        let slave = self.device_index.get(&DeviceId::SlavePic).copied();
        self.machine
            .set_irq_bases_of(Box::new(move |bus: &bus::Bus| {
                [master, slave].map(|index| {
                    let snapshot = bus.device(index?)?.save();
                    Some(devices::Pic8259::diagnostic_from_snapshot(&snapshot)?.vector_base)
                })
            }));
    }

    /// The diagnostic report of this machine at its completed run boundary;
    /// `status` is how the front end names the machine's state. See the
    /// [module documentation](crate::diagnostic) for what it reads and why
    /// that changes nothing the guest can see.
    pub fn diagnostic_report(&mut self, status: &str) -> MachineDiagnostic {
        let machine = &self.machine;
        let cpu = machine.cpu().diagnostic_state();
        let trail = machine.code_trail();
        let pic = |id| {
            let index = *self.device_index.get(&id)?;
            let snapshot = machine.bus().device(index)?.save();
            devices::Pic8259::diagnostic_from_snapshot(&snapshot)
        };
        let master_pic = pic(DeviceId::MasterPic);
        let slave_pic = pic(DeviceId::SlavePic);
        let a20 = self.device_index.contains_key(&DeviceId::Keyboard)
            || self.device_index.contains_key(&DeviceId::SysCtrlPortA);
        let (keyboard_source, fast_source) = self.a20_line.sources();
        let a20 = a20.then(|| A20Diagnostic {
            cpu: cpu.a20,
            keyboard: keyboard_source,
            port_92: self
                .device_index
                .contains_key(&DeviceId::SysCtrlPortA)
                .then_some(fast_source),
            reset_window: machine.reset_fetch_window_open(),
        });
        let mut report = MachineDiagnostic {
            status: status.to_string(),
            virtual_time: machine.virtual_time(),
            clock: machine.clock().0,
            halted: machine.halted(),
            trail: trail.map(machine::CodeTrail::entries),
            trail_samples: trail.map_or(0, machine::CodeTrail::samples),
            deliveries: machine.recent_deliveries(),
            cpu,
            master_pic,
            slave_pic,
            keyboard: None,
            pit: None,
            a20,
            cmos: None,
            vga: None,
        };
        report.keyboard =
            self.device::<devices::I8042, _>(DeviceId::Keyboard, |kbc| kbc.diagnostic_state());
        report.pit = self.device::<devices::pit8253::SharedPit8253, _>(DeviceId::Pit, |pit| {
            pit.diagnostic_state()
        });
        report.cmos =
            self.device::<devices::CmosRtc, _>(DeviceId::Rtc, |rtc| rtc.diagnostic_state());
        report.vga = self.device::<devices::Vga, _>(DeviceId::Vga, |vga| vga.diagnostic_state());
        report
    }

    /// Read a concrete device through the bus, which brings it to the bus
    /// clock first; `None` when the role is absent or another type.
    fn device<T: 'static, R>(&mut self, id: DeviceId, read: impl FnOnce(&T) -> R) -> Option<R> {
        let index = *self.device_index.get(&id)?;
        let device = self
            .machine
            .bus_mut()
            .device_mut(index)?
            .as_any_mut()?
            .downcast_ref::<T>()?;
        Some(read(device))
    }
}

fn yes(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn level(value: bool) -> &'static str {
    if value {
        "high"
    } else {
        "low"
    }
}

fn pic_line(f: &mut fmt::Formatter<'_>, name: &str, pic: &Option<PicDiagnostic>) -> fmt::Result {
    let Some(pic) = pic else {
        return writeln!(f, "pic      {name} not fitted");
    };
    write!(
        f,
        "pic      {name} irr {:02x} isr {:02x} imr {:02x} base {:02x} icw1 {:02x} icw4 {:02x} (auto-eoi {}), ",
        pic.irr,
        pic.isr,
        pic.imr,
        pic.vector_base,
        pic.icw1,
        pic.icw4,
        if pic.auto_eoi() { "on, not performed" } else { "off" },
    )?;
    match pic.awaiting {
        Some(word) => write!(f, "initialization waits for {word}")?,
        None => f.write_str("initialized")?,
    }
    writeln!(
        f,
        ", reads {}, lines {:02x}",
        if pic.reads_isr { "isr" } else { "irr" },
        pic.lines
    )
}

impl fmt::Display for MachineDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "machine diagnostic report")?;
        writeln!(
            f,
            "retired  {} instructions, {} steps",
            self.cpu.retired, self.cpu.completed_steps
        )?;
        writeln!(
            f,
            "time     virtual time {}, clock {} ticks",
            self.virtual_time, self.clock
        )?;
        writeln!(f, "status   {}", self.status)?;
        let halted = match (self.halted, self.cpu.interrupts_enabled()) {
            (false, _) => "no",
            (true, true) => "yes, waiting for an interrupt",
            (true, false) => "yes, with interrupts disabled",
        };
        writeln!(f, "halted   {halted}")?;
        write!(f, "{}", self.cpu)?;
        match &self.trail {
            None => writeln!(f, "trail    not recorded (the code trail was not enabled)")?,
            Some(entries) => {
                writeln!(
                    f,
                    "trail    cs:ip after the last {} run slices, oldest first ({} samples, repeats folded)",
                    entries.len(),
                    self.trail_samples
                )?;
                for entry in entries {
                    write!(f, "trail    {:04x}:{:04x}", entry.cs, entry.ip)?;
                    if entry.samples > 1 {
                        write!(f, " x{}", entry.samples)?;
                    }
                    writeln!(f)?;
                }
            }
        }
        match &self.deliveries {
            None => writeln!(
                f,
                "delivery not recorded (the delivery log was not enabled)"
            )?,
            Some(deliveries) => {
                writeln!(
                    f,
                    "delivery the last {} of {} exceptions and interrupts in {} entries, oldest first ({} dropped before they were read)",
                    deliveries.kept,
                    deliveries.total,
                    deliveries.entries.len(),
                    deliveries.dropped
                )?;
                for entry in &deliveries.entries {
                    writeln!(f, "{entry}")?;
                }
            }
        }
        pic_line(f, "master", &self.master_pic)?;
        pic_line(f, "slave ", &self.slave_pic)?;
        if self.master_pic.is_some() {
            writeln!(
                f,
                "pic      poll mode and special mask mode are not modelled"
            )?;
        }
        match &self.keyboard {
            None => writeln!(f, "kbc      not fitted")?,
            Some(kbc) => {
                writeln!(
                    f,
                    "kbc      status {:02x}, command byte {:02x}: keyboard {}, aux {}, irq1 {}, irq12 {}",
                    kbc.status,
                    kbc.command_byte,
                    if kbc.keyboard_disabled() { "disabled" } else { "enabled" },
                    if kbc.aux_disabled() { "disabled" } else { "enabled" },
                    if kbc.keyboard_interrupt() { "enabled" } else { "disabled" },
                    if kbc.aux_interrupt() { "enabled" } else { "disabled" },
                )?;
                write!(f, "kbc      output buffer ")?;
                match kbc.output_buffer {
                    Some((byte, source)) => write!(f, "{byte:02x} from {source}")?,
                    None => f.write_str("empty")?,
                }
                writeln!(
                    f,
                    ", output port {:02x}, input port {:02x}, inhibit switch {}",
                    kbc.output_port,
                    kbc.input_port,
                    if kbc.inhibited() { "on" } else { "off" }
                )?;
                write!(f, "kbc      pending ")?;
                match kbc.pending {
                    Some(pending) => f.write_str(pending)?,
                    None => f.write_str("none")?,
                }
                f.write_str(", last command ")?;
                match kbc.last_command {
                    Some(command) => write!(f, "{command:02x}")?,
                    None => f.write_str("none")?,
                }
                writeln!(
                    f,
                    ", {} keyboard bytes queued, controller reply {}, awaiting led byte {}, aux device {}",
                    kbc.queued_keyboard,
                    kbc.controller_reply
                        .map_or_else(|| "none".to_string(), |byte| format!("{byte:02x}")),
                    yes(kbc.awaiting_leds),
                    if kbc.aux_attached { "attached" } else { "none" },
                )?;
                writeln!(
                    f,
                    "kbc      irq1 line {}, irq12 line {}",
                    level(kbc.irq1),
                    level(kbc.irq12)
                )?;
            }
        }
        match &self.pit {
            None => writeln!(f, "pit      not fitted")?,
            Some(pit) => {
                for (index, channel) in pit.channels.iter().enumerate() {
                    write!(
                        f,
                        "pit      {index}: mode {} {}{}, reload {:04x}, count {:04x}, out {}, gate {}",
                        channel.mode,
                        channel.access,
                        if channel.bcd { " bcd" } else { "" },
                        channel.reload,
                        channel.count,
                        u8::from(channel.out),
                        u8::from(channel.gate),
                    )?;
                    if !channel.armed {
                        f.write_str(", no count loaded")?;
                    } else if channel.null_count {
                        f.write_str(", new count not loaded yet")?;
                    }
                    if channel.count_latched {
                        f.write_str(", count latched")?;
                    }
                    if channel.status_latched {
                        f.write_str(", status latched")?;
                    }
                    writeln!(f)?;
                }
            }
        }
        match &self.a20 {
            None => writeln!(f, "a20      no a20 source, flat addressing")?,
            Some(a20) => {
                write!(
                    f,
                    "a20      {}: 8042 output port {}, ",
                    if a20.cpu { "on" } else { "off" },
                    if a20.keyboard { "on" } else { "off" }
                )?;
                match a20.port_92 {
                    Some(fast) => writeln!(f, "port 92h {}", if fast { "on" } else { "off" })?,
                    None => writeln!(f, "port 92h not fitted")?,
                }
                if a20.reset_window {
                    writeln!(f, "         CPU A20 held open by the reset window")?;
                }
            }
        }
        match &self.cmos {
            None => writeln!(f, "cmos     not fitted")?,
            Some(cmos) => writeln!(
                f,
                "cmos     shutdown status (0fh) {:02x}, nmi {}, index {:02x}",
                cmos.shutdown_status,
                if cmos.nmi_disabled {
                    "masked"
                } else {
                    "unmasked"
                },
                cmos.index
            )?,
        }
        if let Some(vga) = &self.vga {
            write!(f, "vga      misc {:02x}, seq", vga.misc_output)?;
            for byte in vga.sequencer {
                write!(f, " {byte:02x}")?;
            }
            writeln!(
                f,
                ", ac addr {:02x} (pas {}, next {})",
                vga.attribute_address,
                u8::from(vga.attribute_address & 0x20 != 0),
                if vga.attribute_expects_data {
                    "data"
                } else {
                    "address"
                },
            )?;
            for (label, bytes) in [
                ("crtc", vga.crtc.as_slice()),
                ("gc", vga.graphics.as_slice()),
            ] {
                write!(f, "vga      {label}")?;
                for byte in bytes {
                    write!(f, " {byte:02x}")?;
                }
                writeln!(f)?;
            }
            write!(f, "vga      ac")?;
            for byte in &vga.attribute[..16] {
                write!(f, " {byte:02x}")?;
            }
            let a = &vga.attribute;
            writeln!(
                f,
                ", mode {:02x} overscan {:02x} plane-enable {:02x} pan {:02x} colour-select {:02x}",
                a[16], a[17], a[18], a[19], a[20]
            )?;
            writeln!(
                f,
                "vga      pel-mask {:02x}, dac {} mode, address {:02x}, component {}",
                vga.pel_mask,
                if vga.dac_read_mode { "read" } else { "write" },
                vga.dac_address,
                vga.dac_component
            )?;
            for (group, entries) in vga.dac.chunks(8).enumerate() {
                write!(f, "vga      dac {}-{}", group * 8, group * 8 + 7)?;
                for [r, g, b] in entries {
                    write!(f, " {r:02x}{g:02x}{b:02x}")?;
                }
                writeln!(f)?;
            }
            for (colour, (index, [r, g, b])) in vga.shown.iter().enumerate() {
                writeln!(
                    f,
                    "vga      shown {colour:02x} -> dac {index:02x} {r:02x}{g:02x}{b:02x}"
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::DeviceId;

    /// A guest that points IRQ0 at an EOI-and-IRET handler, programs PIT
    /// counter 0 (mode 2, 1000h) and counter 2 (mode 3, 0800h) with port
    /// 61h untouched, so counter 2's GATE stays low, then waits in a
    /// STI/HLT loop: halted most of the time, woken by the timer.
    fn timer_wait_rom() -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        let code: &[u8] = &[
            0xFA, // CLI
            0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, // XOR AX,AX; MOV DS,AX; MOV SS,AX
            0xBC, 0x00, 0x70, // MOV SP,7000h
            0xC7, 0x06, 0x20, 0x00, 0x80, 0x00, // MOV WORD [0020h],0080h
            0xC7, 0x06, 0x22, 0x00, 0x00, 0xF8, // MOV WORD [0022h],0F800h
            0xB0, 0x11, 0xE6, 0x20, 0xB0, 0x08, 0xE6, 0x21, // ICW1, ICW2
            0xB0, 0x04, 0xE6, 0x21, 0xB0, 0x01, 0xE6, 0x21, // ICW3, ICW4
            0xB0, 0xFE, 0xE6, 0x21, // OCW1: only IRQ0
            0xB0, 0x34, 0xE6, 0x43, 0xB0, 0x00, 0xE6, 0x40, 0xB0, 0x10, 0xE6, 0x40, 0xB0, 0xB6,
            0xE6, 0x43, 0xB0, 0x00, 0xE6, 0x42, 0xB0, 0x08, 0xE6, 0x42, 0xFB, // STI
            0xF4, // HLT
            0xEB, 0xFD, // JMP to the HLT
        ];
        rom[..code.len()].copy_from_slice(code);
        rom[0x80..0x85].copy_from_slice(&[0xB0, 0x20, 0xE6, 0x20, 0xCF]); // EOI; IRET
        rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF8]);
        rom
    }

    fn machine() -> crate::AssembledMachine {
        let config = crate::bios(timer_wait_rom(), None);
        let mut assembled = crate::assemble_with_observation_and_options(
            &config,
            &crate::ObservationPolicy::default(),
            crate::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap();
        // As the PC core runs it: a halted guest waits for its next interrupt.
        assembled.machine.set_idle_limit(Some(u64::MAX));
        assembled
    }

    fn vga_machine() -> crate::AssembledMachine {
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
        crate::assemble_with_observation_and_options(
            &config,
            &crate::ObservationPolicy::default(),
            crate::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap()
    }

    fn vga_out(machine: &mut crate::AssembledMachine, port: u16, value: u8) {
        machine
            .machine
            .bus_mut()
            .port_write(port, 1, u32::from(value))
            .unwrap();
    }

    #[test]
    fn vga_report_renders_programmed_registers_raw_dac_and_shown_colours() {
        let mut machine = vga_machine();
        vga_out(&mut machine, 0x3C2, 0x63);
        for (port, values) in [
            (0x3C4, [3, 1, 15, 0, 14].as_slice()),
            (
                0x3D4,
                [
                    0x5F, 0x4F, 0x50, 0x82, 0x54, 0x80, 0xBF, 0x1F, 0, 0x41, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0, 0, 0,
                ]
                .as_slice(),
            ),
            (0x3CE, [0, 0, 0, 0, 0, 0x10, 0x0E, 0x0F, 0xFF].as_slice()),
            (
                0x3C0,
                [
                    0, 1, 2, 3, 4, 5, 0x14, 7, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 1,
                    0x12, 0x0F, 3, 0,
                ]
                .as_slice(),
            ),
        ] {
            for (index, value) in values.iter().enumerate() {
                vga_out(&mut machine, port, index as u8);
                vga_out(
                    &mut machine,
                    if port == 0x3C0 { port } else { port + 1 },
                    *value,
                );
            }
        }
        vga_out(&mut machine, 0x3C6, 0xFF);
        vga_out(&mut machine, 0x3C8, 0);
        for index in 0..16 {
            for value in [index, index + 1, index + 2] {
                vga_out(&mut machine, 0x3C9, value);
            }
        }
        vga_out(&mut machine, 0x3C8, 0x14);
        for value in [0x2A, 0x15, 0x3F] {
            vga_out(&mut machine, 0x3C9, value);
        }
        vga_out(&mut machine, 0x3C8, 0x10);
        vga_out(&mut machine, 0x3C0, 0x20);
        machine.machine.bus_mut().port_read(0x3DA, 1).unwrap();
        let report = machine.diagnostic_report("paused");
        assert_eq!(report, machine.diagnostic_report("paused"));
        let text = report.to_string();
        let section = text
            .lines()
            .filter(|line| line.starts_with("vga "))
            .collect::<Vec<_>>()
            .join("\n");
        let mut expected = concat!(
            "vga      misc 63, seq 03 01 0f 00 0e, ac addr 20 (pas 1, next address)\n",
            "vga      crtc 5f 4f 50 82 54 80 bf 1f 00 41 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00\n",
            "vga      gc 00 00 00 00 00 10 0e 0f ff\n",
            "vga      ac 00 01 02 03 04 05 14 07 38 39 3a 3b 3c 3d 3e 3f, mode 01 overscan 12 plane-enable 0f pan 03 colour-select 00\n",
            "vga      pel-mask ff, dac write mode, address 10, component 0\n",
            "vga      dac 0-7 000102 010203 020304 030405 040506 050607 060708 070809\n",
            "vga      dac 8-15 08090a 090a0b 0a0b0c 0b0c0d 0c0d0e 0d0e0f 0e0f10 0f1011"
        ).to_string();
        for (colour, (index, rgb)) in [
            (0, "000102"),
            (1, "010203"),
            (2, "020304"),
            (3, "030405"),
            (4, "040506"),
            (5, "050607"),
            (0x14, "2a153f"),
            (7, "070809"),
            (0x38, "000000"),
            (0x39, "000000"),
            (0x3A, "000000"),
            (0x3B, "000000"),
            (0x3C, "000000"),
            (0x3D, "000000"),
            (0x3E, "000000"),
            (0x3F, "000000"),
        ]
        .iter()
        .enumerate()
        {
            expected.push_str(&format!(
                "\nvga      shown {colour:02x} -> dac {index:02x} {rgb}"
            ));
        }
        assert_eq!(section, expected);
        assert!(text.ends_with(&format!("{expected}\n")));
        let mut legacy = self::machine();
        assert!(legacy.diagnostic_report("paused").vga.is_none());
        assert!(!legacy
            .diagnostic_report("paused")
            .to_string()
            .contains("vga "));
    }

    #[test]
    fn vga_reports_preserve_guest_ac_writes_and_partial_dac_reads_and_writes() {
        for read_mode in [false, true] {
            for expects_data in [false, true] {
                let mut diagnosed = vga_machine();
                let mut plain = vga_machine();
                for machine in [&mut diagnosed, &mut plain] {
                    vga_out(machine, 0x3C8, 7);
                    for value in [11, 22, 33] {
                        vga_out(machine, 0x3C9, value);
                    }
                    vga_out(machine, if read_mode { 0x3C7 } else { 0x3C8 }, 7);
                    machine.machine.bus_mut().port_read(0x3C9, 1).unwrap();
                    vga_out(machine, 0x3C0, 0x33);
                    if !expects_data {
                        vga_out(machine, 0x3C0, 5);
                    }
                }
                let before = diagnosed
                    .device::<devices::Vga, _>(DeviceId::Vga, device::Device::save)
                    .unwrap();
                let report = diagnosed.diagnostic_report("paused");
                assert_eq!(report, diagnosed.diagnostic_report("paused"));
                assert_eq!(
                    before,
                    diagnosed
                        .device::<devices::Vga, _>(DeviceId::Vga, device::Device::save)
                        .unwrap()
                );
                let vga = report.vga.unwrap();
                assert_eq!(
                    (vga.dac_address, vga.dac_component, vga.dac_read_mode),
                    (7, 1, read_mode)
                );
                assert_eq!(
                    (vga.attribute_address, vga.attribute_expects_data),
                    (0x33, expects_data)
                );
                for port in [0x3C0, 0x3C9, 0x3C9, 0x3C8] {
                    assert_eq!(
                        diagnosed.machine.bus_mut().port_read(port, 1).unwrap(),
                        plain.machine.bus_mut().port_read(port, 1).unwrap()
                    );
                }
                for machine in [&mut diagnosed, &mut plain] {
                    vga_out(machine, 0x3C0, 9);
                    vga_out(machine, 0x3C9, 44);
                }
                assert_eq!(
                    diagnosed.diagnostic_report("paused"),
                    plain.diagnostic_report("paused")
                );
            }
        }
    }

    /// The report's deliveries on the open delivery ROM, run to its final
    /// halt: the #UD, INT 60h, IRQ 0 and the protected-mode #GP, oldest
    /// first, each with the registers of the interrupted code (ES as
    /// [`crate::bios`] seeds it, F000h, which the ROM never loads); a twin
    /// without the log halts at the same retired count and virtual time
    /// with the same CPU, and its report says the deliveries were not
    /// recorded.
    #[test]
    fn the_report_lists_the_deliveries_of_the_open_delivery_rom() {
        let run = |log: bool| {
            let config = crate::bios(crate::test_support::deliveries_rom(), None);
            let mut assembled = crate::assemble_with_observation_and_options(
                &config,
                &crate::ObservationPolicy::default(),
                crate::AssemblyOptions {
                    allow_unlisted_firmware: true,
                },
            )
            .unwrap();
            assembled.machine.set_idle_limit(Some(u64::MAX));
            if log {
                assembled.enable_delivery_log().unwrap();
            }
            let outcome = assembled.machine.run_bounded(Some(1_000_000)).unwrap();
            assert_eq!(outcome, machine::RunOutcome::Halted);
            assembled.diagnostic_report("halted")
        };
        let plain = run(false);
        let logged = run(true);
        assert_eq!(
            (plain.cpu.retired, plain.virtual_time, &plain.cpu),
            (logged.cpu.retired, logged.virtual_time, &logged.cpu)
        );
        let text = plain.to_string();
        assert!(
            text.contains("\ndelivery not recorded (the delivery log was not enabled)\n"),
            "{text}"
        );
        let text = logged.to_string();
        let section: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("delivery "))
            .collect();
        assert_eq!(
            section,
            [
                "delivery the last 4 of 4 exceptions and interrupts in 4 entries, oldest first (0 dropped before they were read)",
                "delivery 12 fault #UD vector 06 at f800:002e linear 0f802e error - cpl 0->0 bytes 0f ff cd 60 b0 11 e6 20 ax 0000 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es f000 ss:sp 0000:7000 flags 0046",
                "delivery 18 software INT 60h vector 60 at f800:0032 linear 0f8032 error - cpl 0->0 bytes b0 11 e6 20 b0 08 e6 21 ax 0000 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es f000 ss:sp 0000:7000 flags 0046",
                "delivery 37 irq IRQ 0 vector 08 at f800:0054 linear 0f8054 error - cpl 0->0 bytes fa b0 ff e6 21 2e 0f 01 ax 0010 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es f000 ss:sp 0000:7000 flags 0246",
                "delivery 54 fault #GP vector 0d at 0008:007b linear 0f807b error 00f8 cpl 0->0 bytes 8e d8 f4 55 89 e5 83 46 ax 00f8 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es f000 ss:sp 0010:7000 flags 0046",
            ],
            "{text}"
        );
        // The section stands after the trail and before the 8259s.
        let at = text.find("\ndelivery the last").unwrap();
        assert!(text.find("\ntrail    ").unwrap() < at, "{text}");
        assert!(at < text.find("\npic      master").unwrap(), "{text}");
    }

    /// Taking reports changes nothing: a twin that took none ends with the
    /// same CPU, time and device snapshots. The reports fall at halted
    /// moments, with keyboard bytes queued in the 8042 and counter 2's GATE
    /// low, the states whose lazy device timing a report's borrow could
    /// disturb. The diagnosed machine also keeps the delivery log and a
    /// delivery recording, which see every timer interrupt, named IRQ 0.
    #[test]
    fn taking_reports_leaves_every_device_as_a_twin_without_them() {
        let mut plain = machine();
        let mut diagnosed = machine();
        diagnosed.start_delivery_recording().unwrap();
        diagnosed.enable_delivery_log().unwrap();
        assert!(plain.diagnostic_report("running").deliveries.is_none());
        let mut halted = 0;
        for (step, cap) in (20_000..60_000).step_by(997).enumerate() {
            for machine in [&mut plain, &mut diagnosed] {
                machine.machine.run_bounded(Some(cap)).unwrap();
                if step == 3 {
                    // Two bytes: one fills the output buffer, one waits.
                    assert!(machine.type_scancodes(&[0x1E, 0x9E]));
                }
            }
            let report = diagnosed.diagnostic_report("running");
            halted += usize::from(report.halted);
            let pit = report.pit.unwrap();
            assert!(!pit.channels[2].gate);
            assert_eq!((pit.channels[0].mode, pit.channels[0].reload), (2, 0x1000));
            if step > 3 {
                let kbc = report.keyboard.unwrap();
                assert_eq!(kbc.output_buffer, Some((0x1E, "keyboard")));
                assert_eq!(kbc.queued_keyboard, 1);
            }
        }
        assert!(halted > 10, "only {halted} reports at a halt");
        let deliveries = diagnosed.diagnostic_report("running").deliveries.unwrap();
        assert!(deliveries.total >= 10, "{}", deliveries.total);
        assert_eq!(deliveries.kept, deliveries.total);
        assert_eq!(deliveries.dropped, 0);
        for entry in &deliveries.entries {
            let machine::DeliveryEntry::Deliveries { last, .. } = entry else {
                panic!("{entry}");
            };
            assert_eq!(last.name(), cpu::DeliveryName::Irq(0), "{entry}");
            assert_eq!((last.vector, last.cs), (0x08, 0xF800), "{entry}");
        }
        let recording = diagnosed.machine.delivery_recording().unwrap();
        assert!(
            recording.contains(&format!(
                "\nsummary  {0} deliveries, {0} of them in the entries kept, 0 dropped before they were read\nsummary  IRQ 0: {0}\n",
                deliveries.total
            )),
            "{recording}"
        );
        let snapshots = |machine: &mut crate::AssembledMachine| {
            let bus = machine.machine.bus_mut();
            bus.settle();
            (0..)
                .map_while(|index| bus.device(index).map(|device| device.save()))
                .collect::<Vec<_>>()
        };
        for machine in [&mut plain, &mut diagnosed] {
            machine.machine.run_bounded(Some(80_000)).unwrap();
        }
        assert_eq!(snapshots(&mut plain), snapshots(&mut diagnosed));
        let mut report = diagnosed.diagnostic_report("running");
        assert!(report.deliveries.take().is_some());
        assert_eq!(plain.diagnostic_report("running"), report);
        assert!(plain.device_index.contains_key(&DeviceId::Keyboard));
    }

    /// The cost of the code trail and the report's delivery log on a load
    /// heavy in interrupts and port accesses: an open guest programs PIT
    /// counter 0 to count 40h (about 18,600 IRQ 0 a second of virtual time,
    /// one every few hundred instructions) and loops on `IN AL,61h`, so a
    /// run slice ends every four instructions. Machines without either,
    /// with the trail, and with the trail and the log run interleaved, three
    /// of each, to the same virtual time; all end at the same retired
    /// count, and the wall times are printed. Run it in release:
    /// `cargo test --release -p assembly --lib high_irq -- --ignored --nocapture`.
    ///
    /// ```text
    /// 0000 FA               cli
    /// 0001 31C0 8ED8 8ED0   xor ax, ax; mov ds, ax; mov ss, ax
    /// 0007 BC0070           mov sp, 7000h
    /// 000A C70620004000     mov word [0020h], 0040h   ; INT 8 offset
    /// 0010 C706220000F8     mov word [0022h], 0F800h
    /// 0016 B011E620 ...     ICW1 11h, ICW2 08h, ICW3 04h, ICW4 01h
    /// 0026 B0FEE621         OCW1: only IRQ0
    /// 002A B034E643         counter 0, mode 2
    /// 002E B040E640 B000E640 count 0040h
    /// 0036 FB               sti
    /// 0037 41               inc cx
    /// 0038 01CB             add bx, cx
    /// 003A E461             in al, 61h
    /// 003C EBF9             jmp 0037h
    /// 0040 50 B020 E620 58 CF  push ax; mov al, 20h; out 20h, al; pop ax; iret
    /// 7FF0 EA000000F8       jmp F800:0000h
    /// ```
    #[ignore = "a measurement: run in release with --nocapture"]
    #[test]
    fn the_delivery_log_cost_on_a_high_irq_load() {
        let mut rom = vec![0; 0x8000];
        let code: &[u8] = &[
            0xFA, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, 0xBC, 0x00, 0x70, //
            0xC7, 0x06, 0x20, 0x00, 0x40, 0x00, 0xC7, 0x06, 0x22, 0x00, 0x00, 0xF8, //
            0xB0, 0x11, 0xE6, 0x20, 0xB0, 0x08, 0xE6, 0x21, 0xB0, 0x04, 0xE6, 0x21, //
            0xB0, 0x01, 0xE6, 0x21, 0xB0, 0xFE, 0xE6, 0x21, 0xB0, 0x34, 0xE6, 0x43, //
            0xB0, 0x40, 0xE6, 0x40, 0xB0, 0x00, 0xE6, 0x40, 0xFB, //
            0x41, 0x01, 0xCB, 0xE4, 0x61, 0xEB, 0xF9,
        ];
        rom[..code.len()].copy_from_slice(code);
        rom[0x40..0x46].copy_from_slice(&[0x50, 0xB0, 0x20, 0xE6, 0x20, 0x58]);
        rom[0x46] = 0xCF;
        rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF8]);
        let run = |trail: bool, log: bool| {
            let config = crate::bios(rom.clone(), None);
            let mut assembled = crate::assemble_with_observation_and_options(
                &config,
                &crate::ObservationPolicy::default(),
                crate::AssemblyOptions {
                    allow_unlisted_firmware: true,
                },
            )
            .unwrap();
            if trail {
                assembled.machine.enable_code_trail();
            }
            if log {
                assembled.enable_delivery_log().unwrap();
            }
            let started = std::time::Instant::now();
            assembled.machine.run_bounded(Some(60_000_000)).unwrap();
            let elapsed = started.elapsed();
            let total = assembled.machine.recent_deliveries().map_or(0, |d| d.total);
            (cpu::Cpu::retired(assembled.machine.cpu()), elapsed, total)
        };
        let (mut off, mut trail, mut on) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..3 {
            off.push(run(false, false));
            trail.push(run(true, false));
            on.push(run(true, true));
        }
        for runs in [&trail, &on] {
            for (plain, diagnosed) in off.iter().zip(runs) {
                assert_eq!(plain.0, diagnosed.0);
            }
        }
        assert!(on[0].2 > 10_000, "{} deliveries", on[0].2);
        let times = |runs: &[(u64, std::time::Duration, u64)]| {
            runs.iter().map(|run| run.1).collect::<Vec<_>>()
        };
        eprintln!(
            "retired {}, {} IRQ deliveries; plain {:?}; with the trail {:?}; with the trail and the log {:?}",
            on[0].0,
            on[0].2,
            times(&off),
            times(&trail),
            times(&on)
        );
    }
}
