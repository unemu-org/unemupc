// SPDX-License-Identifier: AGPL-3.0-or-later
//! Decoding and report tests over a synthetic memory; the run through the
//! real core is in the backend's tests.

use super::*;

const GDT: u32 = 0x800;
const IDT: u32 = 0x1000;
const TSS: u32 = 0x600;

/// Synthetic guest memory: the first `len` bytes answer, the rest are
/// outside RAM and ROM.
struct Memory {
    bytes: Vec<u8>,
}

impl Memory {
    fn new(len: usize) -> Self {
        Self {
            bytes: vec![0; len],
        }
    }

    fn put(&mut self, at: u32, bytes: &[u8]) {
        let at = at as usize;
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn read(&self) -> impl Fn(u32) -> Option<u8> + '_ {
        |linear| self.bytes.get(linear as usize).copied()
    }
}

const RING0_CODE: [u8; 8] = [0xFF, 0xFF, 0, 0, 0, 0x9A, 0, 0];
const RING0_DATA: [u8; 8] = [0xFF, 0xFF, 0, 0, 0, 0x92, 0, 0];
const TSS_DESCRIPTOR: [u8; 8] = [0x2B, 0, 0x00, 0x06, 0, 0x81, 0, 0];
/// A ring-3 call gate to 0008:7C50 copying two parameter words.
const CALL_GATE: [u8; 8] = [0x50, 0x7C, 0x08, 0, 0x02, 0xE4, 0, 0];
const RING3_CODE: [u8; 8] = [0xFF, 0xFF, 0, 0, 0, 0xFA, 0, 0];
const RING3_DATA: [u8; 8] = [0xFF, 0xFF, 0, 0, 0, 0xF2, 0, 0];

/// A protected-mode machine at CPL 0: the GDT above at 800h, an IDT at 1000h
/// and a 286 TSS at 600h whose ring stacks are 0010:7000, 0000:6000 and
/// 0000:5000.
fn protected_memory() -> Memory {
    let mut memory = Memory::new(0x10000);
    for (index, descriptor) in [
        RING0_CODE,
        RING0_DATA,
        TSS_DESCRIPTOR,
        CALL_GATE,
        RING3_CODE,
        RING3_DATA,
    ]
    .iter()
    .enumerate()
    {
        memory.put(GDT + 8 * (index as u32 + 1), descriptor);
    }
    memory.put(
        TSS,
        &[
            0, 0, 0x00, 0x70, 0x10, 0, 0x00, 0x60, 0, 0, 0x00, 0x50, 0, 0,
        ],
    );
    memory
}

fn segment(selector: u16, attributes: u16) -> SegmentView {
    SegmentView {
        selector,
        base: 0,
        limit: 0xFFFF,
        attributes,
        unusable: false,
    }
}

fn protected_system() -> SystemState {
    SystemState {
        msw: 0xFFF1,
        virtual_8086: false,
        cpl: 0,
        a20: true,
        cs: segment(0x08, 0x9B),
        ss: segment(0x10, 0x93),
        gdtr: TableView {
            base: GDT,
            limit: 0x37,
        },
        idtr: TableView {
            base: IDT,
            limit: 0x7FF,
        },
        ldtr: SegmentView {
            unusable: true,
            ..SegmentView::default()
        },
        tr: SegmentView {
            selector: 0x18,
            base: TSS,
            limit: 0x2B,
            attributes: 0x83,
            unusable: false,
        },
    }
}

fn snapshot(ip: u16, sp: u16, system: Option<SystemState>) -> Snapshot {
    Snapshot {
        stop: "unsupported cpu system operation Example at 0008:7c40".into(),
        cs: 0x08,
        ip: u64::from(ip),
        ss: 0x10,
        sp,
        registers: AddressRegisters::default(),
        bases: [0; 4],
        system,
    }
}

fn regs() -> AddressRegisters {
    AddressRegisters {
        bx: 0x1000,
        bp: 0x7000,
        si: 0x0020,
        di: 0x0040,
    }
}

#[test]
fn far_direct_call_and_jump_name_their_pointer() {
    assert_eq!(
        decode_transfer(&[0x9A, 0x34, 0x12, 0x20, 0x00], regs()),
        Some(TransferForm::FarDirect {
            call: true,
            selector: 0x20,
            offset: 0x1234
        })
    );
    assert_eq!(
        decode_transfer(&[0xEA, 0, 0, 0x2B, 0], regs()),
        Some(TransferForm::FarDirect {
            call: false,
            selector: 0x2B,
            offset: 0
        })
    );
    // A pointer cut short by unavailable bytes decodes nothing.
    assert_eq!(decode_transfer(&[0x9A, 0x34, 0x12, 0x20], regs()), None);
}

#[test]
fn far_indirect_forms_compute_the_16_bit_effective_address() {
    let cases: [(&[u8], bool, SegmentRegister, u16); 7] = [
        // CALL FAR [BP+4]: BP selects SS.
        (&[0xFF, 0x5E, 0x04], true, SegmentRegister::Ss, 0x7004),
        // CALL FAR [BX+SI-2]: a sign-extended 8-bit displacement.
        (&[0xFF, 0x58, 0xFE], true, SegmentRegister::Ds, 0x101E),
        // JMP FAR [0500h] with an ES override.
        (
            &[0x26, 0xFF, 0x2E, 0x00, 0x05],
            false,
            SegmentRegister::Es,
            0x0500,
        ),
        // JMP FAR [BP+DI+1000h] with a CS override.
        (
            &[0x2E, 0xFF, 0xAB, 0x00, 0x10],
            false,
            SegmentRegister::Cs,
            0x8040,
        ),
        // CALL FAR [SI], [DI] and [BX].
        (&[0xFF, 0x1C], true, SegmentRegister::Ds, 0x0020),
        (&[0xFF, 0x1D], true, SegmentRegister::Ds, 0x0040),
        (&[0x3E, 0xFF, 0x1F], true, SegmentRegister::Ds, 0x1000),
    ];
    for (code, call, segment, offset) in cases {
        assert_eq!(
            decode_transfer(code, regs()),
            Some(TransferForm::FarIndirect {
                call,
                segment,
                offset
            }),
            "{code:02x?}"
        );
    }
    // CALL near indirect (FF /2) and a register operand are not far transfers.
    assert_eq!(decode_transfer(&[0xFF, 0x16, 0, 5], regs()), None);
    assert_eq!(decode_transfer(&[0xFF, 0xD8], regs()), None);
}

#[test]
fn software_interrupts_name_their_vector() {
    assert_eq!(
        decode_transfer(&[0xCD, 0x21], regs()),
        Some(TransferForm::Interrupt {
            vector: 0x21,
            kind: InterruptKind::IntN
        })
    );
    assert_eq!(
        decode_transfer(&[0xCC], regs()),
        Some(TransferForm::Interrupt {
            vector: 3,
            kind: InterruptKind::Int3
        })
    );
    assert_eq!(
        decode_transfer(&[0xCE], regs()),
        Some(TransferForm::Interrupt {
            vector: 4,
            kind: InterruptKind::Into
        })
    );
}

#[test]
fn far_returns_and_iret_decode_with_their_release_count() {
    assert_eq!(
        decode_transfer(&[0xCB], regs()),
        Some(TransferForm::FarReturn { release: 0 })
    );
    assert_eq!(
        decode_transfer(&[0xCA, 0x06, 0x00], regs()),
        Some(TransferForm::FarReturn { release: 6 })
    );
    assert_eq!(
        decode_transfer(&[0xF0, 0xCF], regs()),
        Some(TransferForm::InterruptReturn)
    );
}

#[test]
fn other_instructions_and_32_bit_prefixes_are_not_decoded() {
    for code in [
        &[0x90][..],
        &[0xC3],
        &[0x66, 0xCB],
        &[0x67, 0xFF, 0x1C],
        &[0x26],
        &[],
    ] {
        assert_eq!(decode_transfer(code, regs()), None, "{code:02x?}");
    }
}

#[test]
fn descriptors_decode_in_the_80286_layout() {
    let code = Descriptor(RING0_CODE);
    assert_eq!(
        code.kind(),
        DescriptorKind::Segment {
            code: true,
            base: 0,
            limit: 0xFFFF
        }
    );
    assert_eq!((code.dpl(), code.present()), (0, true));
    let data = Descriptor([0x00, 0x10, 0x00, 0x00, 0x12, 0x72, 0, 0]);
    assert_eq!(
        data.kind(),
        DescriptorKind::Segment {
            code: false,
            base: 0x120000,
            limit: 0x1000
        }
    );
    assert_eq!((data.dpl(), data.present()), (3, false));
    assert_eq!(
        Descriptor(CALL_GATE).kind(),
        DescriptorKind::Gate {
            name: "286 call gate",
            selector: 0x08,
            offset: Some(0x7C50),
            words: Some(2)
        }
    );
    assert_eq!(
        Descriptor([0, 0, 0x18, 0, 0, 0x85, 0, 0]).kind(),
        DescriptorKind::Gate {
            name: "task gate",
            selector: 0x18,
            offset: None,
            words: None
        }
    );
    assert_eq!(
        Descriptor([0x00, 0x20, 0x08, 0, 0, 0xEE, 0x01, 0x00]).kind(),
        DescriptorKind::Gate {
            name: "386 interrupt gate",
            selector: 0x08,
            offset: Some(0x0001_2000),
            words: None
        }
    );
    assert_eq!(
        Descriptor([0x00, 0x20, 0x08, 0, 0, 0x86, 0, 0]).kind(),
        DescriptorKind::Gate {
            name: "286 interrupt gate",
            selector: 0x08,
            offset: Some(0x2000),
            words: None
        }
    );
    assert_eq!(
        Descriptor(TSS_DESCRIPTOR).kind(),
        DescriptorKind::System {
            name: "available 286 tss",
            base: 0x600,
            limit: 0x2B
        }
    );
    assert_eq!(
        Descriptor([0x1F, 0, 0, 0x09, 0, 0x82, 0, 0]).kind(),
        DescriptorKind::System {
            name: "ldt",
            base: 0x900,
            limit: 0x1F
        }
    );
    assert_eq!(
        Descriptor([0, 0, 0, 0, 0, 0x88, 0, 0]).kind(),
        DescriptorKind::Reserved
    );
    assert_eq!(
        Descriptor(CALL_GATE).to_string(),
        "50 7c 08 00 02 e4 00 00: 286 call gate, selector 0008, offset 7c50, 2 words, dpl 3, present"
    );
    assert_eq!(
        code.to_string(),
        "ff ff 00 00 00 9a 00 00: code segment type a, base 000000, limit ffff, dpl 0, present"
    );
}

/// A far CALL through a call gate names the gate and the code segment it
/// leads to, with the TSS's ring stacks for the inward transition.
#[test]
fn a_far_call_through_a_gate_names_the_gate_its_target_and_the_ring_stacks() {
    let mut memory = protected_memory();
    memory.put(0x7C40, &[0x9A, 0x00, 0x00, 0x20, 0x00, 0xF4]);
    let report = CpuStopReport::build(
        snapshot(0x7C40, 0x7BFA, Some(protected_system())),
        memory.read(),
    );
    assert_eq!(
        report.form,
        Some(TransferForm::FarDirect {
            call: true,
            selector: 0x20,
            offset: 0
        })
    );
    assert_eq!(report.code_linear, 0x7C40);
    assert_eq!(&report.code[..6], &[0x9A, 0, 0, 0x20, 0, 0xF4].map(Some));
    assert_eq!(
        report.descriptors,
        [
            NamedDescriptor {
                role: "target",
                selector: 0x20,
                table: DescriptorTable::Gdt,
                entry: Ok(Descriptor(CALL_GATE)),
            },
            NamedDescriptor {
                role: "gate target",
                selector: 0x08,
                table: DescriptorTable::Gdt,
                entry: Ok(Descriptor(RING0_CODE)),
            },
        ]
    );
    assert_eq!(
        report.tss,
        Ok(TssStacks {
            link: 0,
            stacks: [(0x10, 0x7000), (0, 0x6000), (0, 0x5000)]
        })
    );
    let text = report.to_string();
    for line in [
        "cpu stop report\n",
        "stop     unsupported cpu system operation Example at 0008:7c40\n",
        "cs:ip    0008:7c40 (linear 007c40)\n",
        "mode     protected, cpl 0, msw fff1, a20 on\n",
        "cs       0008 base 000000 limit 0000ffff attr 009b\n",
        "ss:sp    0010:7bfa\n",
        "gdtr     base 000800 limit 0037\n",
        "idtr     base 001000 limit 07ff\n",
        "ldtr     0000 base 000000 limit 00000000 attr 0000 unusable\n",
        "tr       0018 base 000600 limit 0000002b attr 0083\n",
        "code     9a 00 00 20 00 f4 00 00 00 00 00 00 00 00 00 00\n",
        "form     far call ptr16:16 0020:0000\n",
        "desc     target 0020 (gdt) 50 7c 08 00 02 e4 00 00: 286 call gate",
        "desc     gate target 0008 (gdt) ff ff 00 00 00 9a 00 00: code segment",
        "tss      link 0000, ss0:sp0 0010:7000, ss1:sp1 0000:6000, ss2:sp2 0000:5000\n",
    ] {
        assert!(text.contains(line), "{line:?} missing from\n{text}");
    }
    assert!(text.contains("decoded  lcall"), "{text}");
}

#[test]
fn a_far_indirect_transfer_reads_its_pointer_then_names_the_target() {
    let mut memory = protected_memory();
    memory.put(0x7C40, &[0xFF, 0x5E, 0x04]);
    memory.put(0x7004, &[0x00, 0x01, 0x2B, 0x00]);
    let mut snapshot = snapshot(0x7C40, 0x7BFA, Some(protected_system()));
    snapshot.registers = regs();
    let report = CpuStopReport::build(snapshot, memory.read());
    assert_eq!(
        report.pointer,
        Some(PointerRead {
            linear: 0x7004,
            value: Some((0x0100, 0x002B))
        })
    );
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(report.descriptors[0].role, "target");
    assert_eq!(report.descriptors[0].selector, 0x2B);
    assert_eq!(report.descriptors[0].entry, Ok(Descriptor(RING3_CODE)));
    assert!(report
        .to_string()
        .contains("form     far call m16:16 at ss:7004 (linear 007004) = 002b:0100\n"));
}

#[test]
fn an_interrupt_names_its_idt_entry_and_the_gate_target() {
    let mut memory = protected_memory();
    memory.put(0x7C40, &[0xCD, 0x21]);
    memory.put(IDT + 0x21 * 8, &[0x00, 0x20, 0x08, 0, 0, 0x86, 0, 0]);
    let report = CpuStopReport::build(
        snapshot(0x7C40, 0x7BFA, Some(protected_system())),
        memory.read(),
    );
    assert_eq!(report.descriptors.len(), 2);
    assert_eq!(report.descriptors[0].role, "idt entry");
    assert_eq!(report.descriptors[0].table, DescriptorTable::Idt(0x21));
    assert_eq!(report.descriptors[0].selector, 0x108);
    assert_eq!(report.descriptors[1].role, "gate target");
    assert_eq!(report.descriptors[1].entry, Ok(Descriptor(RING0_CODE)));
    assert!(report
        .to_string()
        .contains("desc     idt entry 0108 (idt vector 21) 00 20 08 00 00 86 00 00: 286 interrupt gate, selector 0008, offset 2000, dpl 0, present\n"));

    // INTO's vector 4 lies beyond an IDT limit of 1Fh.
    memory.put(0x7C40, &[0xCE]);
    let mut system = protected_system();
    system.idtr.limit = 0x1F;
    let report = CpuStopReport::build(snapshot(0x7C40, 0x7BFA, Some(system)), memory.read());
    assert_eq!(report.descriptors[0].entry, Err(Unavailable::BeyondLimit));
}

/// A far return to an outer ring shows the whole frame and the CS and SS it
/// names; RETF n skips the released parameters before the outer SS:SP.
#[test]
fn a_far_return_to_an_outer_ring_names_the_return_cs_and_ss() {
    for (code, release) in [(&[0xCB][..], 0u16), (&[0xCA, 0x04, 0x00], 4)] {
        let mut memory = protected_memory();
        memory.put(0x7C40, code);
        let mut frame = vec![0x00, 0x01, 0x2B, 0x00];
        frame.extend(std::iter::repeat_n(0xEE, usize::from(release)));
        frame.extend([0x00, 0x80, 0x33, 0x00]);
        memory.put(0x7BF0, &frame);
        let report = CpuStopReport::build(
            snapshot(0x7C40, 0x7BF0, Some(protected_system())),
            memory.read(),
        );
        assert_eq!(
            report.stack,
            [
                StackWord {
                    label: "ip",
                    offset: 0x7BF0,
                    value: Some(0x0100)
                },
                StackWord {
                    label: "cs",
                    offset: 0x7BF2,
                    value: Some(0x002B)
                },
                StackWord {
                    label: "outer sp",
                    offset: 0x7BF4 + release,
                    value: Some(0x8000)
                },
                StackWord {
                    label: "outer ss",
                    offset: 0x7BF6 + release,
                    value: Some(0x0033)
                },
            ]
        );
        let roles: Vec<_> = report
            .descriptors
            .iter()
            .map(|named| (named.role, named.selector, named.entry))
            .collect();
        assert_eq!(
            roles,
            [
                ("return cs", 0x2B, Ok(Descriptor(RING3_CODE))),
                ("return ss", 0x33, Ok(Descriptor(RING3_DATA))),
            ]
        );
        assert!(report
            .to_string()
            .contains("stack    ss:7bf2 cs       002b\n"));
    }
}

#[test]
fn an_iret_within_the_ring_names_only_the_return_cs() {
    let mut memory = protected_memory();
    memory.put(0x7C40, &[0xCF]);
    memory.put(0x7BF0, &[0x00, 0x02, 0x08, 0x00, 0x02, 0x02]);
    let report = CpuStopReport::build(
        snapshot(0x7C40, 0x7BF0, Some(protected_system())),
        memory.read(),
    );
    assert_eq!(report.form, Some(TransferForm::InterruptReturn));
    assert_eq!(
        report
            .stack
            .iter()
            .map(|word| word.label)
            .collect::<Vec<_>>(),
        ["ip", "cs", "flags", "outer sp", "outer ss"]
    );
    assert_eq!(report.stack[2].value, Some(0x0202));
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(report.descriptors[0].role, "return cs");
    assert_eq!(report.descriptors[0].entry, Ok(Descriptor(RING0_CODE)));
}

/// Bytes outside RAM and ROM, a null selector, an LDT selector without an
/// LDT and an entry outside memory are each marked, never guessed.
#[test]
fn unavailable_bytes_and_descriptors_are_marked() {
    let mut memory = Memory::new(0x7C43);
    memory.put(0x7C40, &[0xEA, 0x00, 0x00]);
    let report = CpuStopReport::build(
        snapshot(0x7C40, 0x7BFA, Some(protected_system())),
        memory.read(),
    );
    assert_eq!(report.code[3], None);
    assert_eq!(report.form, None, "the pointer is cut short");
    assert!(report.to_string().contains("code     ea 00 00 ?? ?? ??"));

    let mut memory = Memory::new(0x8000);
    for (selector, why) in [
        (0x0000u16, Unavailable::NullSelector),
        (0x0003, Unavailable::NullSelector),
        (0x000C, Unavailable::NoLdt),
        (0x0040, Unavailable::BeyondLimit),
    ] {
        let [low, high] = selector.to_le_bytes();
        memory.put(0x7C40, &[0xEA, 0, 0, low, high]);
        let report = CpuStopReport::build(
            snapshot(0x7C40, 0x7BFA, Some(protected_system())),
            memory.read(),
        );
        assert_eq!(report.descriptors[0].entry, Err(why), "{selector:04x}");
    }
    let mut system = protected_system();
    system.gdtr.base = 0xF000;
    memory.put(0x7C40, &[0xEA, 0, 0, 0x08, 0]);
    let report = CpuStopReport::build(snapshot(0x7C40, 0x7BFA, Some(system)), memory.read());
    assert_eq!(report.descriptors[0].entry, Err(Unavailable::NotInMemory));
    assert!(report
        .to_string()
        .contains("desc     target 0008 (gdt) unavailable: not in ram or rom\n"));
}

#[test]
fn an_ldt_selector_reads_the_local_table() {
    let mut memory = protected_memory();
    memory.put(0x900 + 8, &RING3_CODE);
    memory.put(0x7C40, &[0xEA, 0, 0, 0x0F, 0]);
    let mut system = protected_system();
    system.ldtr = SegmentView {
        selector: 0x38,
        base: 0x900,
        limit: 0x0F,
        attributes: 0x82,
        unusable: false,
    };
    let report = CpuStopReport::build(snapshot(0x7C40, 0x7BFA, Some(system)), memory.read());
    assert_eq!(report.descriptors[0].table, DescriptorTable::Ldt);
    assert_eq!(report.descriptors[0].entry, Ok(Descriptor(RING3_CODE)));
}

/// Real mode and the application profiles have no descriptors: the report
/// keeps the bytes, the form and the return frame.
#[test]
fn real_mode_and_application_reports_name_no_descriptors() {
    let mut memory = protected_memory();
    memory.put(0x7C40, &[0xCB]);
    memory.put(0x7BF0, &[0x00, 0x01, 0x2B, 0x00]);
    let mut real = protected_system();
    real.msw = 0xFFF0;
    for system in [Some(real), None] {
        let report = CpuStopReport::build(snapshot(0x7C40, 0x7BF0, system), memory.read());
        assert_eq!(report.form, Some(TransferForm::FarReturn { release: 0 }));
        assert_eq!(report.stack[1].value, Some(0x002B));
        assert!(report.descriptors.is_empty());
        let text = report.to_string();
        if system.is_some() {
            assert_eq!(report.tss, Err("real mode"));
            assert!(text.contains("mode     real, cpl 0, msw fff0, a20 on\n"));
        } else {
            assert!(report.tss.is_err());
            assert!(text.contains("mode     application profile, no system registers\n"));
            assert!(!text.contains("gdtr"));
        }
    }
}
