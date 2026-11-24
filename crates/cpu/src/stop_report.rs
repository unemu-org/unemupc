// SPDX-License-Identifier: AGPL-3.0-or-later
//! The CPU stop report: the processor state a person needs to diagnose an
//! operation the CPU core refused (§2.1).
//!
//! When the core refuses an operation outside its supported scope, faults
//! outward, or meets an instruction it does not lift, the error names only
//! the operation and CS:IP. The report adds what explains the stop: the
//! machine status word, CPL and A20; the descriptor-table registers GDTR and
//! IDTR; LDTR and TR with their hidden caches; the 16 bytes at CS:IP; and, for
//! a far control transfer, an interrupt or a far return, the operands the
//! instruction names with the 8 bytes of every descriptor it reaches.
//!
//! The decoded forms are those of the Intel 80286 Programmer's Reference
//! Manual, Appendix B: far CALL and JMP with a direct pointer (`ptr16:16`,
//! opcodes 9A and EA) or a memory pointer (`m16:16`, FF /3 and FF /5), whose
//! target selector is named; INT n, INT 3 and INTO, whose IDT entry is named;
//! and RETF, RETF n and IRET, whose return words at SS:SP are shown with the
//! descriptors of the CS and SS they name. Descriptor fields follow the 80286
//! layout of §6.3 (Segment Descriptors) and §7.2 (Gate Descriptors): limit in
//! bytes 0–1, a 24-bit base in bytes 2–4, the access byte in byte 5; a gate
//! holds its offset in bytes 0–1, its selector in bytes 2–3 and its word count
//! in byte 4. When TR holds a 286 task state segment, its ring stacks SS0:SP0,
//! SS1:SP1 and SS2:SP2 (offsets 2–13, §8.2 Task State Segment) and its back
//! link are shown. IRET with NT set is a task return through that back link:
//! the frame shown at SS:SP is then not the one the processor uses.
//!
//! Every byte comes from guest RAM or ROM through the CPU's own memory map; a
//! trapped (memory-mapped I/O) or unmapped address is shown as unavailable and
//! never read, so building a report has no guest-visible effect. The report
//! is data; [`std::fmt::Display`] renders it as plain text for a person to
//! copy. Machine-level use is described in `backlog/docs/design/observation.md`
//! §CPU stop report.

use std::fmt;

/// How many bytes at CS:IP the report shows: more than the longest 80286
/// instruction, so the bytes after a short one show what follows it.
pub const CODE_BYTES: usize = 16;

/// A segment register or a system segment register as the processor holds
/// it: the visible selector and the hidden descriptor cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SegmentView {
    /// Visible selector.
    pub selector: u16,
    /// Cached linear base.
    pub base: u32,
    /// Cached inclusive limit, in bytes.
    pub limit: u32,
    /// Cached access byte (bits 7:0) and the core's flag bits above it.
    pub attributes: u16,
    /// The core marks a cache that holds no usable segment (a null LDTR, say).
    pub unusable: bool,
}

/// GDTR or IDTR: a descriptor table's linear base and inclusive limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableView {
    /// Linear base of the table.
    pub base: u32,
    /// Inclusive limit, in bytes.
    pub limit: u16,
}

/// The system state of a machine CPU profile; absent for the application
/// profiles, which keep no descriptor caches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemState {
    /// Machine status word (the low 16 bits of CR0); bit 0 is PE.
    pub msw: u16,
    /// EFLAGS.VM: paragraph segments at CPL 3 (Intel 80386 PRM, Chapter 15).
    pub virtual_8086: bool,
    /// Current privilege level.
    pub cpl: u8,
    /// Whether the board's A20 gate passes address bit 20.
    pub a20: bool,
    /// Code segment.
    pub cs: SegmentView,
    /// Stack segment.
    pub ss: SegmentView,
    /// Global descriptor table register.
    pub gdtr: TableView,
    /// Interrupt descriptor table register.
    pub idtr: TableView,
    /// Local descriptor table register.
    pub ldtr: SegmentView,
    /// Task register.
    pub tr: SegmentView,
}

impl SystemState {
    /// Whether the processor runs in protected mode (MSW.PE).
    pub fn protected(&self) -> bool {
        self.msw & 1 != 0
    }
}

/// A segment register an instruction's memory operand uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentRegister {
    Es,
    Cs,
    Ss,
    Ds,
}

impl SegmentRegister {
    fn name(self) -> &'static str {
        match self {
            Self::Es => "es",
            Self::Cs => "cs",
            Self::Ss => "ss",
            Self::Ds => "ds",
        }
    }
}

/// Which software interrupt instruction an [`TransferForm::Interrupt`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptKind {
    /// INT n (CD ib).
    IntN,
    /// INT 3 (CC).
    Int3,
    /// INTO (CE).
    Into,
}

/// The register values a 16-bit effective address can use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AddressRegisters {
    pub bx: u16,
    pub bp: u16,
    pub si: u16,
    pub di: u16,
}

/// A control transfer the report decodes; see the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferForm {
    /// Far CALL (9A) or JMP (EA) with a `ptr16:16` operand.
    FarDirect {
        call: bool,
        selector: u16,
        offset: u16,
    },
    /// Far CALL (FF /3) or JMP (FF /5) through a `m16:16` operand at
    /// `segment`:`offset`.
    FarIndirect {
        call: bool,
        segment: SegmentRegister,
        offset: u16,
    },
    /// INT n, INT 3 or INTO, through IDT entry `vector`.
    Interrupt { vector: u8, kind: InterruptKind },
    /// RETF, or RETF n releasing `release` bytes of parameters.
    FarReturn { release: u16 },
    /// IRET.
    InterruptReturn,
}

/// Decode the transfer at the start of `code`, after any segment-override,
/// LOCK or REP prefixes; `None` for every other instruction, and for an
/// operand- or address-size prefix, whose 32-bit forms the report does not
/// decode. A memory pointer's effective address uses `registers` with the
/// 16-bit addressing forms of the Intel 80286 Programmer's Reference Manual,
/// §3.6 (Addressing Modes); a BP base selects SS unless overridden.
pub fn decode_transfer(code: &[u8], registers: AddressRegisters) -> Option<TransferForm> {
    let mut segment = None;
    let mut at = 0;
    loop {
        match *code.get(at)? {
            0x26 => segment = Some(SegmentRegister::Es),
            0x2E => segment = Some(SegmentRegister::Cs),
            0x36 => segment = Some(SegmentRegister::Ss),
            0x3E => segment = Some(SegmentRegister::Ds),
            0xF0 | 0xF2 | 0xF3 => {}
            _ => break,
        }
        at += 1;
    }
    let word = |offset: usize| -> Option<u16> {
        Some(u16::from_le_bytes([
            *code.get(at + offset)?,
            *code.get(at + offset + 1)?,
        ]))
    };
    let form = match code[at] {
        opcode @ (0x9A | 0xEA) => TransferForm::FarDirect {
            call: opcode == 0x9A,
            offset: word(1)?,
            selector: word(3)?,
        },
        0xFF => {
            let modrm = *code.get(at + 1)?;
            let call = match (modrm >> 3) & 7 {
                3 => true,
                5 => false,
                _ => return None,
            };
            let mode = modrm >> 6;
            let rm = modrm & 7;
            if mode == 3 {
                return None;
            }
            let displacement = match (mode, rm) {
                (0, 6) | (2, _) => word(2)?,
                (1, _) => i16::from(*code.get(at + 2)? as i8) as u16,
                _ => 0,
            };
            let r = registers;
            let (base, stack) = match (mode, rm) {
                (0, 6) => (0, false),
                (_, 0) => (r.bx.wrapping_add(r.si), false),
                (_, 1) => (r.bx.wrapping_add(r.di), false),
                (_, 2) => (r.bp.wrapping_add(r.si), true),
                (_, 3) => (r.bp.wrapping_add(r.di), true),
                (_, 4) => (r.si, false),
                (_, 5) => (r.di, false),
                (_, 6) => (r.bp, true),
                _ => (r.bx, false),
            };
            let default = if stack {
                SegmentRegister::Ss
            } else {
                SegmentRegister::Ds
            };
            TransferForm::FarIndirect {
                call,
                segment: segment.unwrap_or(default),
                offset: base.wrapping_add(displacement),
            }
        }
        0xCD => TransferForm::Interrupt {
            vector: *code.get(at + 1)?,
            kind: InterruptKind::IntN,
        },
        0xCC => TransferForm::Interrupt {
            vector: 3,
            kind: InterruptKind::Int3,
        },
        0xCE => TransferForm::Interrupt {
            vector: 4,
            kind: InterruptKind::Into,
        },
        0xCB => TransferForm::FarReturn { release: 0 },
        0xCA => TransferForm::FarReturn { release: word(1)? },
        0xCF => TransferForm::InterruptReturn,
        _ => return None,
    };
    Some(form)
}

/// The 8 bytes of a descriptor as they sit in a descriptor table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor(pub [u8; 8]);

/// A descriptor's fields in the 80286 layout; see the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorKind {
    /// A code or data segment (access byte S = 1).
    Segment { code: bool, base: u32, limit: u16 },
    /// A task state segment or local descriptor table (S = 0, type 1, 2, 3,
    /// 9 or 11).
    System {
        name: &'static str,
        base: u32,
        limit: u16,
    },
    /// A call, interrupt, trap or task gate (S = 0, type 4–7 or 12–15).
    /// A task gate has no offset; only a call gate copies parameter words.
    Gate {
        name: &'static str,
        selector: u16,
        offset: Option<u32>,
        words: Option<u8>,
    },
    /// A system type the processor reserves.
    Reserved,
}

impl Descriptor {
    /// The access byte: P, DPL, S and the type.
    pub fn access(&self) -> u8 {
        self.0[5]
    }

    /// Whether the descriptor is present (access bit 7).
    pub fn present(&self) -> bool {
        self.access() & 0x80 != 0
    }

    /// Descriptor privilege level (access bits 6:5).
    pub fn dpl(&self) -> u8 {
        (self.access() >> 5) & 3
    }

    /// The type field (access bits 3:0).
    pub fn kind_bits(&self) -> u8 {
        self.access() & 0xF
    }

    fn word(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.0[at], self.0[at + 1]])
    }

    /// The decoded fields.
    pub fn kind(&self) -> DescriptorKind {
        let base = u32::from(self.word(2)) | (u32::from(self.0[4]) << 16);
        let limit = self.word(0);
        if self.access() & 0x10 != 0 {
            return DescriptorKind::Segment {
                code: self.access() & 0x08 != 0,
                base,
                limit,
            };
        }
        let gate = |name, offset, words| DescriptorKind::Gate {
            name,
            selector: self.word(2),
            offset,
            words,
        };
        let offset16 = Some(u32::from(self.word(0)));
        let offset32 = Some(u32::from(self.word(0)) | (u32::from(self.word(6)) << 16));
        match self.kind_bits() {
            1 => DescriptorKind::System {
                name: "available 286 tss",
                base,
                limit,
            },
            2 => DescriptorKind::System {
                name: "ldt",
                base,
                limit,
            },
            3 => DescriptorKind::System {
                name: "busy 286 tss",
                base,
                limit,
            },
            9 => DescriptorKind::System {
                name: "available 386 tss",
                base,
                limit,
            },
            11 => DescriptorKind::System {
                name: "busy 386 tss",
                base,
                limit,
            },
            4 => gate("286 call gate", offset16, Some(self.0[4] & 0x1F)),
            5 => gate("task gate", None, None),
            6 => gate("286 interrupt gate", offset16, None),
            7 => gate("286 trap gate", offset16, None),
            12 => gate("386 call gate", offset32, Some(self.0[4] & 0x1F)),
            14 => gate("386 interrupt gate", offset32, None),
            15 => gate("386 trap gate", offset32, None),
            _ => DescriptorKind::Reserved,
        }
    }
}

impl fmt::Display for Descriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{byte:02x}")?;
        }
        f.write_str(": ")?;
        match self.kind() {
            DescriptorKind::Segment { code, base, limit } => write!(
                f,
                "{} segment type {:x}, base {base:06x}, limit {limit:04x}",
                if code { "code" } else { "data" },
                self.kind_bits()
            )?,
            DescriptorKind::System { name, base, limit } => {
                write!(f, "{name}, base {base:06x}, limit {limit:04x}")?
            }
            DescriptorKind::Gate {
                name,
                selector,
                offset,
                words,
            } => {
                write!(f, "{name}, selector {selector:04x}")?;
                if let Some(offset) = offset {
                    write!(f, ", offset {offset:04x}")?;
                }
                if let Some(words) = words {
                    write!(f, ", {words} words")?;
                }
            }
            DescriptorKind::Reserved => write!(f, "reserved system type {:x}", self.kind_bits())?,
        }
        write!(
            f,
            ", dpl {}, {}",
            self.dpl(),
            if self.present() {
                "present"
            } else {
                "not present"
            }
        )
    }
}

/// Where a named descriptor comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorTable {
    Gdt,
    Ldt,
    /// The IDT entry of this vector.
    Idt(u8),
}

/// Why a named descriptor's bytes are not shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// A null selector names no descriptor.
    NullSelector,
    /// The entry lies beyond the table's limit.
    BeyondLimit,
    /// The selector names the LDT and LDTR holds none.
    NoLdt,
    /// The entry's bytes are not in RAM or ROM.
    NotInMemory,
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NullSelector => "null selector",
            Self::BeyondLimit => "beyond the table limit",
            Self::NoLdt => "no ldt loaded",
            Self::NotInMemory => "not in ram or rom",
        })
    }
}

/// A descriptor the decoded instruction reaches, with its 8 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedDescriptor {
    /// What the instruction uses it for: `target`, `gate target`, `return cs`.
    pub role: &'static str,
    /// The selector that names it; for an IDT entry, the vector times 8.
    pub selector: u16,
    pub table: DescriptorTable,
    pub entry: Result<Descriptor, Unavailable>,
}

/// One labelled word of a return frame at SS:SP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackWord {
    pub label: &'static str,
    /// Offset in the stack segment.
    pub offset: u16,
    /// `None` when the word is not in RAM or ROM.
    pub value: Option<u16>,
}

/// The `m16:16` pointer of a far indirect transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerRead {
    /// Linear address of the pointer.
    pub linear: u32,
    /// Offset and selector, `None` when not in RAM or ROM.
    pub value: Option<(u16, u16)>,
}

/// The ring stacks of a 286 task state segment, with its back link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TssStacks {
    /// The selector of the task that called this one.
    pub link: u16,
    /// SS:SP for privilege levels 0, 1 and 2.
    pub stacks: [(u16, u16); 3],
}

/// The diagnosis of one CPU stop; see the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuStopReport {
    /// The stop as the error states it.
    pub stop: String,
    /// Visible code selector.
    pub cs: u16,
    /// Instruction pointer of the stopping instruction.
    pub ip: u64,
    /// Visible stack selector.
    pub ss: u16,
    /// Stack pointer.
    pub sp: u16,
    /// Linear address of CS:IP.
    pub code_linear: u32,
    /// The bytes at CS:IP; `None` where not in RAM or ROM.
    pub code: [Option<u8>; CODE_BYTES],
    /// The first instruction, disassembled.
    pub instruction: Option<String>,
    /// System registers of a machine profile; `None` for an application one.
    pub system: Option<SystemState>,
    /// The decoded control transfer, if the instruction is one.
    pub form: Option<TransferForm>,
    /// The memory pointer of a far indirect transfer.
    pub pointer: Option<PointerRead>,
    /// The return frame of RETF or IRET.
    pub stack: Vec<StackWord>,
    /// Every descriptor the instruction reaches, in the order it does.
    pub descriptors: Vec<NamedDescriptor>,
    /// The current task's ring stacks, or why they are not shown.
    pub tss: Result<TssStacks, &'static str>,
}

/// What the report is built from: the stopping CPU's registers and a reader
/// of its memory by linear address.
pub(crate) struct Snapshot {
    pub stop: String,
    pub cs: u16,
    pub ip: u64,
    pub ss: u16,
    pub sp: u16,
    pub registers: AddressRegisters,
    /// Linear bases of ES, CS, SS and DS.
    pub bases: [u32; 4],
    pub system: Option<SystemState>,
}

impl CpuStopReport {
    /// Build the report; `read` returns the byte at a linear address, or
    /// `None` where no RAM or ROM answers.
    pub(crate) fn build(snapshot: Snapshot, read: impl Fn(u32) -> Option<u8>) -> Self {
        let Snapshot {
            stop,
            cs,
            ip,
            ss,
            sp,
            registers,
            bases,
            system,
        } = snapshot;
        let [_, code_base, stack_base, _] = bases;
        let in_segment = |base: u32, offset: u16| base.wrapping_add(u32::from(offset));
        let word_at = |linear: u32| -> Option<u16> {
            Some(u16::from_le_bytes([
                read(linear)?,
                read(linear.wrapping_add(1))?,
            ]))
        };
        let code: [Option<u8>; CODE_BYTES] = std::array::from_fn(|index| {
            read(in_segment(
                code_base,
                (ip as u16).wrapping_add(index as u16),
            ))
        });
        let fetched: Vec<u8> = code.iter().map_while(|byte| *byte).collect();
        let instruction = x86jit_core::disasm::disassemble(
            &fetched,
            ip & 0xFFFF,
            x86jit_core::lift::CpuMode::Real16,
        )
        .into_iter()
        .next()
        .map(|decoded| decoded.text);
        let form = decode_transfer(&fetched, registers);
        let protected = system.is_some_and(|system| system.protected());
        let paragraph = system.is_some_and(|system| system.virtual_8086);
        let mut pointer = None;
        let mut stack = Vec::new();
        let mut descriptors = Vec::new();
        let name = |role, selector: u16, descriptors: &mut Vec<NamedDescriptor>| {
            if let Some(system) = &system {
                let named = named_descriptor(system, role, selector, &read);
                push_following_gate(descriptors, system, named, &read);
            }
        };
        match form {
            Some(TransferForm::FarDirect { selector, .. }) if protected && !paragraph => {
                name("target", selector, &mut descriptors);
            }
            Some(TransferForm::FarIndirect {
                segment, offset, ..
            }) => {
                let base = bases[segment as usize];
                let linear = in_segment(base, offset);
                let value = word_at(linear).zip(word_at(in_segment(base, offset.wrapping_add(2))));
                pointer = Some(PointerRead { linear, value });
                if let (true, Some((_, selector))) = (protected && !paragraph, value) {
                    name("target", selector, &mut descriptors);
                }
            }
            Some(TransferForm::Interrupt { vector, .. }) if protected => {
                if let Some(system) = &system {
                    let named = idt_entry(system, vector, &read);
                    push_following_gate(&mut descriptors, system, named, &read);
                }
            }
            Some(TransferForm::FarReturn { release }) => {
                let labels: [(&str, u16); 4] = [
                    ("ip", 0),
                    ("cs", 2),
                    ("outer sp", 4u16.wrapping_add(release)),
                    ("outer ss", 6u16.wrapping_add(release)),
                ];
                stack = return_frame(
                    if paragraph { &labels[..2] } else { &labels },
                    sp,
                    stack_base,
                    &read,
                );
            }
            Some(TransferForm::InterruptReturn) => {
                let labels: [(&str, u16); 5] = [
                    ("ip", 0),
                    ("cs", 2),
                    ("flags", 4),
                    ("outer sp", 6),
                    ("outer ss", 8),
                ];
                stack = return_frame(
                    if paragraph { &labels[..3] } else { &labels },
                    sp,
                    stack_base,
                    &read,
                );
            }
            _ => {}
        }
        if protected && !paragraph && !stack.is_empty() {
            let word = |label| {
                stack
                    .iter()
                    .find(|word| word.label == label)
                    .and_then(|word| word.value)
            };
            let cpl = system.map_or(0, |system| system.cpl);
            if let Some(return_cs) = word("cs") {
                name("return cs", return_cs, &mut descriptors);
                // Only a return to an outer ring pops SS:SP.
                if u8::try_from(return_cs & 3).unwrap_or(0) > cpl {
                    if let Some(outer_ss) = word("outer ss") {
                        name("return ss", outer_ss, &mut descriptors);
                    }
                }
            }
        }
        let tss = match &system {
            None => Err("no system registers in this cpu profile"),
            Some(system) if !system.protected() => Err("real mode"),
            Some(system) => tss_stacks(system, &read),
        };
        Self {
            stop,
            cs,
            ip,
            ss,
            sp,
            code_linear: in_segment(code_base, ip as u16),
            code,
            instruction,
            system,
            form,
            pointer,
            stack,
            descriptors,
            tss,
        }
    }
}

/// Push `named`, and after a gate the descriptor the gate names.
fn push_following_gate(
    descriptors: &mut Vec<NamedDescriptor>,
    system: &SystemState,
    named: NamedDescriptor,
    read: &impl Fn(u32) -> Option<u8>,
) {
    let next = match named.entry.map(|descriptor| descriptor.kind()) {
        Ok(DescriptorKind::Gate { name, selector, .. }) => Some((
            if name == "task gate" {
                "task gate tss"
            } else {
                "gate target"
            },
            selector,
        )),
        _ => None,
    };
    descriptors.push(named);
    if let Some((role, selector)) = next {
        descriptors.push(named_descriptor(system, role, selector, read));
    }
}

fn return_frame(
    labels: &[(&'static str, u16)],
    sp: u16,
    stack_base: u32,
    read: &impl Fn(u32) -> Option<u8>,
) -> Vec<StackWord> {
    labels
        .iter()
        .map(|&(label, delta)| {
            let offset = sp.wrapping_add(delta);
            let low = stack_base.wrapping_add(u32::from(offset));
            let high = stack_base.wrapping_add(u32::from(offset.wrapping_add(1)));
            StackWord {
                label,
                offset,
                value: read(low)
                    .zip(read(high))
                    .map(|(low, high)| u16::from_le_bytes([low, high])),
            }
        })
        .collect()
}

fn table_entry(
    base: u32,
    limit: u32,
    index: u16,
    read: &impl Fn(u32) -> Option<u8>,
) -> Result<Descriptor, Unavailable> {
    if u32::from(index) + 7 > limit {
        return Err(Unavailable::BeyondLimit);
    }
    let mut bytes = [0; 8];
    for (at, byte) in bytes.iter_mut().enumerate() {
        *byte = read(base.wrapping_add(u32::from(index) + at as u32))
            .ok_or(Unavailable::NotInMemory)?;
    }
    Ok(Descriptor(bytes))
}

fn named_descriptor(
    system: &SystemState,
    role: &'static str,
    selector: u16,
    read: &impl Fn(u32) -> Option<u8>,
) -> NamedDescriptor {
    let local = selector & 4 != 0;
    let index = selector & !7;
    let entry = if local {
        if system.ldtr.unusable || system.ldtr.selector & !3 == 0 {
            Err(Unavailable::NoLdt)
        } else {
            table_entry(system.ldtr.base, system.ldtr.limit, index, read)
        }
    } else if index == 0 {
        Err(Unavailable::NullSelector)
    } else {
        table_entry(system.gdtr.base, u32::from(system.gdtr.limit), index, read)
    };
    NamedDescriptor {
        role,
        selector,
        table: if local {
            DescriptorTable::Ldt
        } else {
            DescriptorTable::Gdt
        },
        entry,
    }
}

fn idt_entry(
    system: &SystemState,
    vector: u8,
    read: &impl Fn(u32) -> Option<u8>,
) -> NamedDescriptor {
    let index = u16::from(vector) * 8;
    NamedDescriptor {
        role: "idt entry",
        selector: index,
        table: DescriptorTable::Idt(vector),
        entry: table_entry(system.idtr.base, u32::from(system.idtr.limit), index, read),
    }
}

fn tss_stacks(
    system: &SystemState,
    read: &impl Fn(u32) -> Option<u8>,
) -> Result<TssStacks, &'static str> {
    let tr = system.tr;
    if tr.unusable || tr.selector & !3 == 0 {
        return Err("tr not loaded");
    }
    if !matches!(tr.attributes & 0xF, 1 | 3) {
        return Err("tr does not hold a 286 tss");
    }
    if tr.limit < 13 {
        return Err("tss limit below its stack fields");
    }
    let word = |offset: u32| -> Result<u16, &'static str> {
        let low = read(tr.base.wrapping_add(offset));
        let high = read(tr.base.wrapping_add(offset + 1));
        low.zip(high)
            .map(|(low, high)| u16::from_le_bytes([low, high]))
            .ok_or("tss not in ram or rom")
    };
    Ok(TssStacks {
        link: word(0)?,
        stacks: [
            (word(4)?, word(2)?),
            (word(8)?, word(6)?),
            (word(12)?, word(10)?),
        ],
    })
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

impl fmt::Display for CpuStopReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "cpu stop report")?;
        writeln!(f, "stop     {}", self.stop)?;
        writeln!(
            f,
            "cs:ip    {:04x}:{:04x} (linear {:06x})",
            self.cs, self.ip, self.code_linear
        )?;
        match &self.system {
            None => writeln!(f, "mode     application profile, no system registers")?,
            Some(system) => {
                writeln!(
                    f,
                    "mode     {}, cpl {}, msw {:04x}, a20 {}",
                    if system.virtual_8086 {
                        "virtual-8086"
                    } else if system.protected() {
                        "protected"
                    } else {
                        "real"
                    },
                    system.cpl,
                    system.msw,
                    if system.a20 { "on" } else { "off" }
                )?;
                segment_line(f, "cs", &system.cs)?;
                segment_line(f, "ss", &system.ss)?;
            }
        }
        writeln!(f, "ss:sp    {:04x}:{:04x}", self.ss, self.sp)?;
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
        f.write_str("code    ")?;
        for byte in &self.code {
            match byte {
                Some(byte) => write!(f, " {byte:02x}")?,
                None => f.write_str(" ??")?,
            }
        }
        writeln!(f)?;
        if let Some(text) = &self.instruction {
            writeln!(f, "decoded  {text}")?;
        }
        if let Some(form) = &self.form {
            f.write_str("form     ")?;
            let kind = |call: bool| if call { "call" } else { "jmp" };
            match *form {
                TransferForm::FarDirect {
                    call,
                    selector,
                    offset,
                } => writeln!(f, "far {} ptr16:16 {selector:04x}:{offset:04x}", kind(call))?,
                TransferForm::FarIndirect {
                    call,
                    segment,
                    offset,
                } => {
                    write!(
                        f,
                        "far {} m16:16 at {}:{offset:04x}",
                        kind(call),
                        segment.name()
                    )?;
                    match self.pointer {
                        Some(PointerRead {
                            linear,
                            value: Some((offset, selector)),
                        }) => writeln!(f, " (linear {linear:06x}) = {selector:04x}:{offset:04x}")?,
                        Some(PointerRead {
                            linear,
                            value: None,
                        }) => writeln!(f, " (linear {linear:06x}) = ??")?,
                        None => writeln!(f)?,
                    }
                }
                TransferForm::Interrupt { vector, kind } => {
                    let name = match kind {
                        InterruptKind::IntN => "int",
                        InterruptKind::Int3 => "int3",
                        InterruptKind::Into => "into",
                    };
                    writeln!(f, "{name}, vector {vector:02x}")?
                }
                TransferForm::FarReturn { release: 0 } => writeln!(f, "retf")?,
                TransferForm::FarReturn { release } => writeln!(f, "retf {release:04x}")?,
                TransferForm::InterruptReturn => writeln!(f, "iret")?,
            }
        }
        for word in &self.stack {
            write!(f, "stack    ss:{:04x} {:<9}", word.offset, word.label)?;
            match word.value {
                Some(value) => writeln!(f, "{value:04x}")?,
                None => writeln!(f, "??")?,
            }
        }
        for named in &self.descriptors {
            let table = match named.table {
                DescriptorTable::Gdt => "gdt".to_string(),
                DescriptorTable::Ldt => "ldt".to_string(),
                DescriptorTable::Idt(vector) => format!("idt vector {vector:02x}"),
            };
            write!(
                f,
                "desc     {} {:04x} ({table}) ",
                named.role, named.selector
            )?;
            match &named.entry {
                Ok(descriptor) => writeln!(f, "{descriptor}")?,
                Err(why) => writeln!(f, "unavailable: {why}")?,
            }
        }
        match &self.tss {
            Ok(tss) => {
                write!(f, "tss      link {:04x}", tss.link)?;
                for (ring, (ss, sp)) in tss.stacks.iter().enumerate() {
                    write!(f, ", ss{ring}:sp{ring} {ss:04x}:{sp:04x}")?;
                }
                writeln!(f)
            }
            Err(why) => writeln!(f, "tss      {why}"),
        }
    }
}

#[cfg(test)]
#[path = "stop_report_tests.rs"]
mod tests;
