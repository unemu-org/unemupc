// SPDX-License-Identifier: AGPL-3.0-or-later
//! The processor's part of a diagnostic report of a running machine (§2.1).
//!
//! The [CPU stop report](crate::stop_report) describes a CPU that refused an
//! operation. A machine can also stop responding while the CPU keeps running
//! (a loop with interrupts disabled) or sits halted, and then nothing
//! produces a report. [`CpuDiagnostic`] is the picture of the processor at
//! any completed run boundary: the general registers, FLAGS with IF, TF, DF,
//! IOPL and NT decoded, the machine status word and CPL, the visible
//! selector and hidden descriptor cache of CS, DS, ES and SS, GDTR, IDTR,
//! LDTR and TR, whether an interrupt waits for delivery, the 16 bytes at
//! CS:IP and the 16 words at SS:SP. The FLAGS layout is that of the Intel
//! 80286 Programmer's Reference Manual, §3.5.3 (Flags Word) and §10.3
//! (FLAGS: IOPL in bits 12–13, NT in bit 14); the MSW is §10.1.
//!
//! Memory is read as the stop report reads it, through the CPU's own map with
//! the 80286's 24-bit address bus and the board's A20 gate applied. A byte in
//! a trapped (memory-mapped device) window is never read and shows as
//! [`Peek::Device`]; one where nothing is mapped shows as [`Peek::Unmapped`].
//! In the machine profile an offset the segment's hidden cache does not
//! allow (beyond the limit of an expand-up segment, at or below the limit of
//! an expand-down data segment, or any offset of an unusable cache) shows as
//! [`Peek::BeyondLimit`] and is not read, so the picture never presents
//! memory the guest cannot reach through that segment (Intel 80286
//! Programmer's Reference Manual, §6.3, Segment Descriptors, and §7.3,
//! Limit Checking).
//! Building the picture therefore has no device or guest effect. Machine-level
//! use is described in `backlog/docs/design/observation.md` §Diagnostic report
//! of a running machine.

use crate::stop_report::{SegmentView, TableView, CODE_BYTES};
use std::fmt;

/// How many words at SS:SP the picture shows.
pub const STACK_WORDS: usize = 16;

/// What a host-side read of guest memory found at one address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peek<T> {
    /// RAM or ROM answered with this value.
    Value(T),
    /// The address lies in a trapped window of a memory-mapped device; it was
    /// not read, because a device read can have side effects.
    Device,
    /// No memory answers at the address.
    Unmapped,
    /// The offset lies outside the segment's cached limit (or the cache is
    /// unusable), so the guest cannot reach the address through this
    /// segment; it is not read.
    BeyondLimit,
}

impl<T: Copy> Peek<T> {
    /// The value, when RAM or ROM answered.
    pub fn value(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }
}

/// The system registers of a machine CPU profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemRegisters {
    /// Machine status word (the low 16 bits of CR0); bit 0 is PE.
    pub msw: u16,
    /// EFLAGS.VM: paragraph segments at CPL 3 (Intel 80386 PRM, Chapter 15).
    pub virtual_8086: bool,
    /// Current privilege level.
    pub cpl: u8,
    /// Global descriptor table register.
    pub gdtr: TableView,
    /// Interrupt descriptor table register.
    pub idtr: TableView,
    /// Local descriptor table register.
    pub ldtr: SegmentView,
    /// Task register.
    pub tr: SegmentView,
}

impl SystemRegisters {
    /// Whether the processor runs in protected mode (MSW.PE).
    pub fn protected(&self) -> bool {
        self.msw & 1 != 0
    }
}

/// The processor at one completed run boundary; see the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuDiagnostic {
    /// AX, CX, DX, BX, SP, BP, SI and DI, in encoding order.
    pub registers: [u16; 8],
    /// Instruction pointer: where execution continues.
    pub ip: u16,
    /// The 16-bit FLAGS image, system bits included.
    pub flags: u16,
    /// ES, CS, SS and DS: visible selector and descriptor cache. In an
    /// application profile, which keeps no caches, the base is the selector
    /// times 16 and the limit FFFFh.
    pub segments: [SegmentView; 4],
    /// System registers of a machine profile; `None` for an application one.
    pub system: Option<SystemRegisters>,
    /// Whether the CPU's A20 gate passes address bit 20.
    pub a20: bool,
    /// An injected interrupt vector waits for delivery (IF clear, an
    /// interrupt shadow, or an unfinished access holds it).
    pub irq_queued: bool,
    /// A maskable interrupt could be accepted at this boundary.
    pub irq_ready: bool,
    /// Whole instructions retired since the CPU was built.
    pub retired: u64,
    /// Completed scheduling units (instructions and REP iterations).
    pub completed_steps: u64,
    /// Linear address of CS:IP.
    pub code_linear: u32,
    /// The bytes at CS:IP.
    pub code: [Peek<u8>; CODE_BYTES],
    /// The byte before CS:IP, which tells a halted CPU's HLT (F4h) apart.
    pub before_code: Peek<u8>,
    /// The first instruction at CS:IP, disassembled.
    pub instruction: Option<String>,
    /// Linear address of SS:SP.
    pub stack_linear: u32,
    /// The words at SS:SP, SP first.
    pub stack: [Peek<u16>; STACK_WORDS],
}

/// Encoding-order names of [`CpuDiagnostic::registers`].
const REGISTER_NAMES: [&str; 8] = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di"];

/// Index of a segment register in [`CpuDiagnostic::segments`].
const ES: usize = 0;
const CS: usize = 1;
const SS: usize = 2;
const DS: usize = 3;

impl CpuDiagnostic {
    /// A register by its name: `ax` … `di`.
    pub fn register(&self, name: &str) -> Option<u16> {
        REGISTER_NAMES
            .iter()
            .position(|known| *known == name)
            .map(|index| self.registers[index])
    }

    /// Interrupt enable (FLAGS bit 9).
    pub fn interrupts_enabled(&self) -> bool {
        self.flags & (1 << 9) != 0
    }

    /// The code segment register.
    pub fn cs(&self) -> SegmentView {
        self.segments[CS]
    }

    /// The stack segment register.
    pub fn ss(&self) -> SegmentView {
        self.segments[SS]
    }

    /// Stack pointer.
    pub fn sp(&self) -> u16 {
        self.registers[4]
    }

    /// Where a halted CPU stopped: the CS:IP of the HLT it executed, which
    /// is the byte before CS:IP when that byte is F4h. `None` otherwise.
    pub fn hlt_before(&self) -> Option<(u16, u16)> {
        (self.before_code == Peek::Value(0xF4))
            .then(|| (self.cs().selector, self.ip.wrapping_sub(1)))
    }

    /// The legend of the markers the picture uses, `None` when it uses none.
    fn legend(&self) -> Option<String> {
        let code = |peek: Peek<u8>| self.code.contains(&peek);
        let stack = |peek: Peek<u16>| self.stack.contains(&peek);
        let parts: Vec<&str> = [
            (
                code(Peek::Device) || stack(Peek::Device),
                "dv: a memory-mapped device, not read",
            ),
            (
                code(Peek::Unmapped) || stack(Peek::Unmapped),
                "??: no memory",
            ),
            (
                code(Peek::BeyondLimit) || stack(Peek::BeyondLimit),
                "--: beyond the segment limit, not reachable through it",
            ),
        ]
        .into_iter()
        .filter_map(|(used, text)| used.then_some(text))
        .collect();
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

fn segment_line(f: &mut fmt::Formatter<'_>, name: &str, view: &SegmentView) -> fmt::Result {
    write!(
        f,
        "{name:<9}{:04x} base {:06x} limit {:08x} attr {:04x}",
        view.selector, view.base, view.limit, view.attributes
    )?;
    if view.unusable {
        f.write_str(" unusable")?;
    }
    writeln!(f)
}

fn bit(flags: u16, at: u16) -> u16 {
    (flags >> at) & 1
}

/// The picture as plain text, one labelled line per item in a fixed order;
/// the retired and step counts are left to the report that embeds it.
impl fmt::Display for CpuDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cs = self.cs();
        let ss = self.ss();
        writeln!(
            f,
            "cs:ip    {:04x}:{:04x} (linear {:06x})",
            cs.selector, self.ip, self.code_linear
        )?;
        let flags = self.flags;
        writeln!(
            f,
            "flags    {flags:04x}: if {}, tf {}, df {}, iopl {}, nt {}; of {} sf {} zf {} af {} pf {} cf {}",
            bit(flags, 9),
            bit(flags, 8),
            bit(flags, 10),
            (flags >> 12) & 3,
            bit(flags, 14),
            bit(flags, 11),
            bit(flags, 7),
            bit(flags, 6),
            bit(flags, 4),
            bit(flags, 2),
            bit(flags, 0),
        )?;
        f.write_str("regs    ")?;
        for name in ["ax", "bx", "cx", "dx", "si", "di", "bp", "sp"] {
            write!(f, " {name} {:04x}", self.register(name).unwrap_or(0))?;
        }
        writeln!(f)?;
        match &self.system {
            None => writeln!(
                f,
                "mode     application profile, no system registers, a20 {}",
                on(self.a20)
            )?,
            Some(system) => writeln!(
                f,
                "mode     {}, cpl {}, msw {:04x} (pe {}), a20 {}",
                if system.virtual_8086 {
                    "virtual-8086"
                } else if system.protected() {
                    "protected"
                } else {
                    "real"
                },
                system.cpl,
                system.msw,
                system.msw & 1,
                on(self.a20)
            )?,
        }
        for (name, index) in [("cs", CS), ("ds", DS), ("es", ES), ("ss", SS)] {
            segment_line(f, name, &self.segments[index])?;
        }
        if let Some(system) = &self.system {
            writeln!(
                f,
                "gdtr     base {:06x} limit {:04x}",
                system.gdtr.base, system.gdtr.limit
            )?;
            writeln!(
                f,
                "idtr     base {:06x} limit {:04x}",
                system.idtr.base, system.idtr.limit
            )?;
            segment_line(f, "ldtr", &system.ldtr)?;
            segment_line(f, "tr", &system.tr)?;
        }
        writeln!(
            f,
            "irq      {}, {}",
            if self.irq_queued {
                "an injected vector waits for delivery"
            } else {
                "no injected vector waits"
            },
            if self.irq_ready {
                "accepts a maskable interrupt now"
            } else {
                "cannot accept a maskable interrupt now"
            }
        )?;
        if let Some((selector, offset)) = self.hlt_before() {
            writeln!(
                f,
                "hlt      at {selector:04x}:{offset:04x}, the byte before cs:ip"
            )?;
        }
        f.write_str("code    ")?;
        for byte in &self.code {
            match byte {
                Peek::Value(byte) => write!(f, " {byte:02x}")?,
                Peek::Device => f.write_str(" dv")?,
                Peek::Unmapped => f.write_str(" ??")?,
                Peek::BeyondLimit => f.write_str(" --")?,
            }
        }
        writeln!(f)?;
        if let Some(text) = &self.instruction {
            writeln!(f, "decoded  {text}")?;
        }
        writeln!(
            f,
            "ss:sp    {:04x}:{:04x} (linear {:06x})",
            ss.selector,
            self.sp(),
            self.stack_linear
        )?;
        for (row, words) in self.stack.chunks(8).enumerate() {
            write!(f, "stack    +{:02x}", row * 16)?;
            for word in words {
                match word {
                    Peek::Value(word) => write!(f, " {word:04x}")?,
                    Peek::Device => f.write_str(" dvdv")?,
                    Peek::Unmapped => f.write_str(" ????")?,
                    Peek::BeyondLimit => f.write_str(" ----")?,
                }
            }
            writeln!(f)?;
        }
        if let Some(legend) = self.legend() {
            writeln!(f, "note     {legend}")?;
        }
        Ok(())
    }
}

fn on(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
}

/// What the picture is built from: the CPU's registers and a reader of its
/// memory by linear address.
pub(crate) struct Registers {
    pub registers: [u16; 8],
    pub ip: u16,
    pub flags: u16,
    pub segments: [SegmentView; 4],
    pub system: Option<SystemRegisters>,
    pub a20: bool,
    pub irq_queued: bool,
    pub irq_ready: bool,
    pub retired: u64,
    pub completed_steps: u64,
}

impl CpuDiagnostic {
    /// Build the picture; `read` returns what a linear address holds.
    pub(crate) fn build(registers: Registers, read: impl Fn(u32) -> Peek<u8>) -> Self {
        let Registers {
            registers: words,
            ip,
            flags,
            segments,
            system,
            a20,
            irq_queued,
            irq_ready,
            retired,
            completed_steps,
        } = registers;
        let at = |base: u32, offset: u16| base.wrapping_add(u32::from(offset));
        // Only the machine profile keeps caches whose limits the processor checks.
        let checked = system.is_some();
        let peek = |view: &SegmentView, offset: u16| {
            if checked && !reachable(view, offset) {
                Peek::BeyondLimit
            } else {
                read(at(view.base, offset))
            }
        };
        let (cs, ss) = (&segments[CS], &segments[SS]);
        let code: [Peek<u8>; CODE_BYTES] =
            std::array::from_fn(|index| peek(cs, ip.wrapping_add(index as u16)));
        let before_code = peek(cs, ip.wrapping_sub(1));
        let sp = words[4];
        let stack: [Peek<u16>; STACK_WORDS] = std::array::from_fn(|index| {
            let offset = sp.wrapping_add(2 * index as u16);
            match (peek(ss, offset), peek(ss, offset.wrapping_add(1))) {
                (Peek::Value(low), Peek::Value(high)) => {
                    Peek::Value(u16::from_le_bytes([low, high]))
                }
                (Peek::BeyondLimit, _) | (_, Peek::BeyondLimit) => Peek::BeyondLimit,
                (Peek::Device, _) | (_, Peek::Device) => Peek::Device,
                _ => Peek::Unmapped,
            }
        });
        let code_base = cs.base;
        let stack_base = ss.base;
        let fetched: Vec<u8> = code.iter().map_while(|byte| byte.value()).collect();
        let instruction = x86jit_core::disasm::disassemble(
            &fetched,
            u64::from(ip),
            x86jit_core::lift::CpuMode::Real16,
        )
        .into_iter()
        .next()
        .map(|decoded| decoded.text);
        Self {
            registers: words,
            ip,
            flags,
            segments,
            system,
            a20,
            irq_queued,
            irq_ready,
            retired,
            completed_steps,
            code_linear: at(code_base, ip),
            code,
            before_code,
            instruction,
            stack_linear: at(stack_base, sp),
            stack,
        }
    }
}

/// Whether the segment's cache allows `offset`: a usable cache, and an
/// offset within the limit, or above it for an expand-down data segment
/// (Intel 80286 Programmer's Reference Manual, §6.3 and §7.3).
fn reachable(view: &SegmentView, offset: u16) -> bool {
    if view.unusable {
        return false;
    }
    let offset = u32::from(offset);
    let data = view.attributes & 0x18 == 0x10;
    let expand_down = data && view.attributes & 0x04 != 0;
    if expand_down {
        offset > view.limit
    } else {
        offset <= view.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(selector: u16, base: u32, limit: u32, attributes: u16) -> SegmentView {
        SegmentView {
            selector,
            base,
            limit,
            attributes,
            unusable: false,
        }
    }

    /// In the machine profile, code past the code segment's limit and stack
    /// words at or below an expand-down stack's limit are marked, not read.
    #[test]
    fn offsets_the_segment_does_not_allow_are_marked_and_not_read() {
        let flat = segment(0x10, 0, 0xFFFF, 0x93);
        let registers = |system| Registers {
            // SP = 0FEEh
            registers: [0, 0, 0, 0, 0x0FEE, 0, 0, 0],
            ip: 0x0002,
            flags: 0x0002,
            // ES, CS (limit 5), SS (expand-down, limit 0FF0h), DS
            segments: [
                flat,
                segment(0x08, 0x1000, 0x0005, 0x9B),
                segment(0x18, 0x2000, 0x0FF0, 0x97),
                flat,
            ],
            system,
            a20: true,
            irq_queued: false,
            irq_ready: false,
            retired: 0,
            completed_steps: 0,
        };
        let read = |linear: u32| Peek::Value(linear as u8);
        let state = CpuDiagnostic::build(registers(Some(SystemRegisters::default())), read);
        // Offsets 2..=5 are inside the limit, 6 and up are not.
        assert!(state.code[..4].iter().all(|byte| byte.value().is_some()));
        assert!(state.code[4..]
            .iter()
            .all(|byte| *byte == Peek::BeyondLimit));
        // 0FEE–0FF1: one byte or both at or below the limit; 0FF2 on is valid.
        assert_eq!(state.stack[..2], [Peek::BeyondLimit; 2]);
        assert_eq!(state.stack[2], Peek::Value(0xF3F2));
        let text = state.to_string();
        assert!(text.contains("stack    +00 ---- ---- f3f2"), "{text}");
        assert!(
            text.contains("note     --: beyond the segment limit, not reachable through it\n"),
            "{text}"
        );
        // An application profile keeps no caches, so nothing is marked.
        let plain = CpuDiagnostic::build(registers(None), read);
        assert!(plain.code.iter().all(|byte| byte.value().is_some()));
        assert!(plain.stack.iter().all(|word| word.value().is_some()));
    }
}
