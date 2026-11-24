// SPDX-License-Identifier: AGPL-3.0-or-later
//! The exceptions and interrupts the guest took (§2.1).
//!
//! A machine CPU delivers its exceptions in the guest, through the IVT or the
//! IDT, so the host never sees them: a guest that faults in a loop, or whose
//! handler fixes things up and returns, looks from outside like any other
//! running guest. The CPU core can keep a log of the last N deliveries it
//! performed (`x86jit-core` spec.md §17.10, Delivery log), outside the CPU
//! state the guest executes on. [`Delivery`] is one record of that log in the
//! backend-neutral shape the machine reads between runs
//! ([`crate::Cpu::drain_deliveries`], [`crate::Cpu::peek_deliveries`]): the
//! retired-instruction count when the handler was entered, the kind of
//! delivery, the vector, the return address the delivery saved (CS:IP and its
//! linear address), the error code it pushed, the privilege level before and
//! after, the eight bytes at the saved CS:IP, and the registers of the
//! interrupted code ([`DeliveryRegisters`]) when the backend records them.
//!
//! What the saved IP names depends on the kind (Intel 80286 Programmer's
//! Reference Manual, Chapter 9, Interrupts and Exceptions; Intel SDM Vol. 3A
//! §7.5): a fault saves
//! the faulting instruction, a software interrupt and the single-step trap
//! the next one, and an external interrupt the instruction it interrupted,
//! which has not run. The bytes are read through the CPU's own memory map
//! (RAM and ROM only, never a trapped device window) when the machine reads
//! the log, at the end of the run slice that delivered: a port access ends a
//! slice before the bus performs it, so the A20 gate and the interrupt
//! controller are as they were at the delivery, while a store the guest made
//! later in the same slice is already visible. With the log off the core's
//! execution, retired counts and state are unchanged. The machine-level use
//! is described in `backlog/docs/design/observation.md` §Exception and
//! interrupt deliveries.

use crate::diagnostic::Peek;
use std::fmt;

/// How many bytes at the saved CS:IP a [`Delivery`] carries.
pub const DELIVERY_BYTES: usize = 8;

/// The largest log the CPU core keeps.
pub const DELIVERY_LOG_MAX: usize = x86jit_core::DeliveryLog::MAX_CAPACITY;

/// What kind of event a delivery was, which also says what its saved IP
/// names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeliveryKind {
    /// A processor exception of the fault or abort class (#DF and BOUND's
    /// #BR included): the saved IP is the faulting instruction.
    Fault,
    /// The single-step trap: the saved IP is the next instruction.
    Trap,
    /// INT n, INT 3 or a taken INTO: the saved IP is the next instruction.
    Software,
    /// A vector from the interrupt controller: the saved IP is the
    /// instruction it interrupted.
    Irq,
    /// The non-maskable interrupt.
    Nmi,
    /// A kind this backend does not know yet.
    Other,
}

impl DeliveryKind {
    /// The lower-case word a trace line uses.
    pub fn word(self) -> &'static str {
        match self {
            Self::Fault => "fault",
            Self::Trap => "trap",
            Self::Software => "software",
            Self::Irq => "irq",
            Self::Nmi => "nmi",
            Self::Other => "other",
        }
    }
}

/// What a delivery is called: the exception's mnemonic, `INT nnh`, `IRQ n`
/// or `NMI`. Orders as the summary of a trace lists the counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeliveryName {
    /// A processor exception (fault, abort or the single-step trap) at this
    /// vector.
    Exception(u8),
    /// The non-maskable interrupt.
    Nmi,
    /// A hardware interrupt on this controller input, when the machine
    /// could tell it.
    Irq(u8),
    /// A hardware interrupt at this vector whose controller input the
    /// machine could not tell.
    IrqVector(u8),
    /// A software interrupt at this vector.
    Int(u8),
    /// A delivery of a kind this backend does not know, at this vector.
    Other(u8),
}

/// The mnemonics of the 80286's exceptions, as Intel SDM Vol. 3A §7.15
/// names the same vectors (the 80286 Programmer's Reference Manual,
/// Chapter 9, describes them without mnemonics). Vector 9, the processor
/// extension segment overrun, has none and shows by number.
fn exception_mnemonic(vector: u8) -> Option<&'static str> {
    Some(match vector {
        0 => "#DE",
        1 => "#DB",
        2 => "NMI",
        3 => "#BP",
        4 => "#OF",
        5 => "#BR",
        6 => "#UD",
        7 => "#NM",
        8 => "#DF",
        10 => "#TS",
        11 => "#NP",
        12 => "#SS",
        13 => "#GP",
        16 => "#MF",
        _ => return None,
    })
}

impl fmt::Display for DeliveryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Exception(vector) => match exception_mnemonic(vector) {
                Some(mnemonic) => f.write_str(mnemonic),
                None => write!(f, "exception {vector:02X}h"),
            },
            Self::Nmi => f.write_str("NMI"),
            Self::Irq(line) => write!(f, "IRQ {line}"),
            Self::IrqVector(vector) => write!(f, "IRQ at {vector:02X}h"),
            Self::Int(vector) => write!(f, "INT {vector:02X}h"),
            Self::Other(vector) => write!(f, "vector {vector:02X}h"),
        }
    }
}

/// One exception or interrupt the guest took; see the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// Instructions retired when the handler was entered: a software
    /// interrupt's own instruction is counted, a faulting one is not.
    pub retired: u64,
    /// What kind of event it was.
    pub kind: DeliveryKind,
    /// The vector the delivery used.
    pub vector: u8,
    /// The controller input of an [`DeliveryKind::Irq`], when the machine
    /// could tell it from its interrupt controller's vector bases.
    pub irq: Option<u8>,
    /// The code selector the delivery saved.
    pub cs: u16,
    /// The instruction pointer the delivery saved.
    pub ip: u32,
    /// The CS base in effect before the delivery plus [`Self::ip`], before
    /// the A20 gate.
    pub linear: u32,
    /// The error code the delivery pushed; `None` when it pushed none, as
    /// every real-mode delivery.
    pub error_code: Option<u32>,
    /// The privilege level before the delivery; 0 outside protected mode.
    pub cpl_before: u8,
    /// The privilege level of the handler; 0 outside protected mode.
    pub cpl_after: u8,
    /// EFLAGS.VM before delivery and in the entered handler.
    pub virtual_8086_before: bool,
    pub virtual_8086_after: bool,
    /// The bytes at the saved CS:IP, read when the machine read the log, at
    /// the end of the run slice; the offset wraps at the end of a 16-bit
    /// segment as the 80286's IP does.
    pub bytes: [Peek<u8>; DELIVERY_BYTES],
    /// The registers of the interrupted code at the delivery; `None` when
    /// the backend did not record them.
    pub regs: Option<DeliveryRegisters>,
}

/// The registers of the interrupted code at one delivery, as the CPU core's
/// delivery log records them (`x86jit-core` spec.md §17.10, Delivery log):
/// each holds its value when the delivery began, after the raising
/// instruction's own effects and before the frame push, the stack switch
/// and the FLAGS changes the delivery makes. For a software interrupt these
/// are the INT's inputs (AH holding a DOS or BIOS function number, say);
/// for a fault, the state the handler is entered from; for an IRQ or NMI,
/// the state after the last instruction that completed. SS:SP is the
/// interrupted stack, before the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeliveryRegisters {
    /// The general registers. The 80286 fills the low 16 bits (AX, BX, ...)
    /// and leaves the high 16 zero; the 80386 fills all 32.
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
    /// The interrupted code's stack pointer: on a 16-bit stack its low 16
    /// bits are SP.
    pub esp: u32,
    pub ds: u16,
    pub es: u16,
    pub ss: u16,
    /// FS and GS, which only the 80386 has; `None` on the 80286.
    pub fs: Option<u16>,
    pub gs: Option<u16>,
    /// The FLAGS image (EFLAGS on the 80386) as PUSHF would form it.
    pub flags: u32,
}

impl DeliveryRegisters {
    /// Whether these are an 80386's 32-bit registers, which is when FS and
    /// GS are present.
    fn wide(&self) -> bool {
        self.fs.is_some() || self.gs.is_some()
    }
}

/// `ax 0000 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es 0000
/// ss:sp 0000:0000 flags 0000` with the 80286's 16-bit registers; with the
/// 80386's, `eax 00000000` and so on, `fs` and `gs` after `es`, `ss:esp
/// 0000:00000000` and `flags 00000000`.
impl fmt::Display for DeliveryRegisters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let wide = self.wide();
        let general = [
            ("ax", self.eax),
            ("bx", self.ebx),
            ("cx", self.ecx),
            ("dx", self.edx),
            ("si", self.esi),
            ("di", self.edi),
            ("bp", self.ebp),
        ];
        for (index, (name, value)) in general.into_iter().enumerate() {
            let space = if index == 0 { "" } else { " " };
            if wide {
                write!(f, "{space}e{name} {value:08x}")?;
            } else {
                write!(f, "{space}{name} {:04x}", value as u16)?;
            }
        }
        write!(f, " ds {:04x} es {:04x}", self.ds, self.es)?;
        for (name, selector) in [("fs", self.fs), ("gs", self.gs)] {
            if let Some(selector) = selector {
                write!(f, " {name} {selector:04x}")?;
            }
        }
        if wide {
            write!(
                f,
                " ss:esp {:04x}:{:08x} flags {:08x}",
                self.ss, self.esp, self.flags
            )
        } else {
            write!(
                f,
                " ss:sp {:04x}:{:04x} flags {:04x}",
                self.ss, self.esp as u16, self.flags as u16
            )
        }
    }
}

impl Delivery {
    /// What the delivery is called.
    pub fn name(&self) -> DeliveryName {
        match self.kind {
            DeliveryKind::Fault | DeliveryKind::Trap => DeliveryName::Exception(self.vector),
            DeliveryKind::Software => DeliveryName::Int(self.vector),
            DeliveryKind::Irq => match self.irq {
                Some(line) => DeliveryName::Irq(line),
                None => DeliveryName::IrqVector(self.vector),
            },
            DeliveryKind::Nmi => DeliveryName::Nmi,
            DeliveryKind::Other => DeliveryName::Other(self.vector),
        }
    }
}

/// One line, without a label: `retired kind name vector at cs:ip linear
/// error cpl bytes`, for example `1234 fault #UD vector 06 at f800:0024
/// linear 0f8024 error - cpl 0->0 bytes 0f ff f4 00 00 00 00 00`, then the
/// [`DeliveryRegisters`] when the record has them (` ax 0000 bx 0000 ...
/// ss:sp 0000:7000 flags 0046`). A byte in a device window reads `dv`, one
/// where nothing is mapped `--`. The format is described in
/// `backlog/docs/design/observation.md` §Exception and interrupt
/// deliveries.
impl fmt::Display for Delivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {} vector {:02x} at {:04x}:{:04x} linear {:06x} error ",
            self.retired,
            self.kind.word(),
            self.name(),
            self.vector,
            self.cs,
            self.ip,
            self.linear
        )?;
        match self.error_code {
            Some(code) => write!(f, "{code:04x}")?,
            None => f.write_str("-")?,
        }
        write!(f, " cpl {}->{}", self.cpl_before, self.cpl_after)?;
        if self.virtual_8086_before || self.virtual_8086_after {
            write!(
                f,
                " vm {}->{}",
                u8::from(self.virtual_8086_before),
                u8::from(self.virtual_8086_after)
            )?;
        }
        f.write_str(" bytes")?;
        for byte in self.bytes {
            match byte {
                Peek::Value(value) => write!(f, " {value:02x}")?,
                Peek::Device => f.write_str(" dv")?,
                Peek::Unmapped | Peek::BeyondLimit => f.write_str(" --")?,
            }
        }
        match &self.regs {
            Some(regs) => write!(f, " {regs}"),
            None => Ok(()),
        }
    }
}

/// The log's counts at a read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryCounts {
    /// Records the full ring overwrote since the previous consuming read,
    /// which no read returned.
    pub dropped: u64,
    /// Every delivery since the log was enabled, dropped ones included.
    pub total: u64,
}

/// Why a CPU keeps no delivery log.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("no delivery log: {0}")]
pub struct DeliveryLogError(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(kind: DeliveryKind, vector: u8) -> Delivery {
        Delivery {
            retired: 1234,
            kind,
            vector,
            irq: None,
            cs: 0xF800,
            ip: 0x24,
            linear: 0xF8024,
            error_code: None,
            cpl_before: 0,
            cpl_after: 0,
            virtual_8086_before: false,
            virtual_8086_after: false,
            bytes: [Peek::Value(0); DELIVERY_BYTES],
            regs: None,
        }
    }

    fn registers() -> DeliveryRegisters {
        DeliveryRegisters {
            eax: 0x0900,
            ebx: 0x1234,
            ecx: 0x0003,
            edx: 0x0105,
            esi: 0x0080,
            edi: 0xFFFE,
            ebp: 0x7FF0,
            esp: 0x6FFA,
            ds: 0x1000,
            es: 0xB800,
            ss: 0x2000,
            fs: None,
            gs: None,
            flags: 0x0246,
        }
    }

    #[test]
    fn v86_delivery_line_keeps_vm_separate_from_the_pushfd_image() {
        let mut record = delivery(DeliveryKind::Fault, 13);
        record.virtual_8086_before = true;
        record.cpl_before = 3;
        record.regs = Some(registers());
        assert_eq!(record.regs.unwrap().flags & (1 << 17), 0);
        assert!(record.to_string().contains("cpl 3->0 vm 1->0 bytes"));
        record.virtual_8086_before = false;
        record.virtual_8086_after = true;
        assert!(record.to_string().contains("vm 0->1"));
    }

    /// A record with registers appends them after its bytes: the 80286's
    /// 16-bit names and SS:SP; the 80386's e-registers, FS and GS and
    /// SS:ESP. A record without them ends at its bytes.
    #[test]
    fn a_line_with_registers_appends_them_after_the_bytes() {
        let mut record = delivery(DeliveryKind::Software, 0x21);
        record.regs = Some(registers());
        assert_eq!(
            record.to_string(),
            "1234 software INT 21h vector 21 at f800:0024 linear 0f8024 error - cpl 0->0 \
             bytes 00 00 00 00 00 00 00 00 \
             ax 0900 bx 1234 cx 0003 dx 0105 si 0080 di fffe bp 7ff0 ds 1000 es b800 \
             ss:sp 2000:6ffa flags 0246"
        );
        let wide = DeliveryRegisters {
            eax: 0x8000_0900,
            esp: 0x0001_6FFA,
            fs: Some(0x0030),
            gs: Some(0),
            flags: 0x0004_0246,
            ..registers()
        };
        assert_eq!(
            wide.to_string(),
            "eax 80000900 ebx 00001234 ecx 00000003 edx 00000105 esi 00000080 \
             edi 0000fffe ebp 00007ff0 ds 1000 es b800 fs 0030 gs 0000 \
             ss:esp 2000:00016ffa flags 00040246"
        );
        assert!(delivery(DeliveryKind::Fault, 6)
            .to_string()
            .ends_with(" bytes 00 00 00 00 00 00 00 00"));
    }

    #[test]
    fn a_line_names_the_delivery_and_marks_unread_bytes() {
        let mut record = delivery(DeliveryKind::Fault, 6);
        record.bytes[0] = Peek::Value(0x0F);
        record.bytes[1] = Peek::Value(0xFF);
        record.bytes[6] = Peek::Device;
        record.bytes[7] = Peek::Unmapped;
        assert_eq!(
            record.to_string(),
            "1234 fault #UD vector 06 at f800:0024 linear 0f8024 error - cpl 0->0 \
             bytes 0f ff 00 00 00 00 dv --"
        );
        let mut gp = delivery(DeliveryKind::Fault, 13);
        (gp.error_code, gp.cpl_before) = (Some(0xF8), 3);
        assert!(gp
            .to_string()
            .contains(" fault #GP vector 0d at f800:0024 linear 0f8024 error 00f8 cpl 3->0 "));
        let names = [
            (delivery(DeliveryKind::Fault, 0x0E), "exception 0Eh"),
            (delivery(DeliveryKind::Trap, 1), "#DB"),
            (delivery(DeliveryKind::Software, 0x21), "INT 21h"),
            (delivery(DeliveryKind::Software, 0x1C), "INT 1Ch"),
            (delivery(DeliveryKind::Irq, 0x08), "IRQ at 08h"),
            (
                Delivery {
                    irq: Some(0),
                    ..delivery(DeliveryKind::Irq, 0x08)
                },
                "IRQ 0",
            ),
            (delivery(DeliveryKind::Nmi, 2), "NMI"),
        ];
        for (record, name) in names {
            assert_eq!(record.name().to_string(), name);
        }
    }
}
