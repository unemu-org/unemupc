// SPDX-License-Identifier: AGPL-3.0-or-later
//! Windows-style instruction mixes: Intel 80386 Programmer's Reference Manual
//! §5.1/§5.2 (LDT segments and paging), §6.3 (privilege transitions) and §15
//! (V86 and I/O trapping). See `backlog/docs/design/windows95-on-486.md` §3.

use super::speed;
use assembly::{AssemblyOptions, BoardModel, CpuChip};
use cpu::{Cpu, CpuReg};
use machine::RunOutcome;

pub const CASES: [Case; 6] = [
    Case {
        name: "flat32-cpl3",
        rom: include_bytes!("../fixtures/cpu-ceiling-flat32-cpl3.bin"),
        paged: false,
        segmented: false,
        v86: false,
        trapped: false,
    },
    Case {
        name: "flat-paged32-cpl3",
        rom: include_bytes!("../fixtures/cpu-ceiling-flat-paged32-cpl3.bin"),
        paged: true,
        segmented: false,
        v86: false,
        trapped: false,
    },
    Case {
        name: "flat-paged32-cpl3-farcall",
        rom: include_bytes!("../fixtures/cpu-ceiling-flat-paged32-cpl3-farcall.bin"),
        paged: true,
        segmented: false,
        v86: false,
        trapped: false,
    },
    Case {
        name: "paged-protected16-cpl3",
        rom: include_bytes!("../fixtures/cpu-ceiling-paged-protected16-cpl3.bin"),
        paged: true,
        segmented: true,
        v86: false,
        trapped: false,
    },
    Case {
        name: "paged-v86",
        rom: include_bytes!("../fixtures/cpu-ceiling-paged-v86.bin"),
        paged: true,
        segmented: true,
        v86: true,
        trapped: false,
    },
    Case {
        name: "paged-v86-iopl0",
        rom: include_bytes!("../fixtures/cpu-ceiling-paged-v86-iopl0.bin"),
        paged: true,
        segmented: true,
        v86: true,
        trapped: true,
    },
];

#[derive(Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    rom: &'static [u8],
    paged: bool,
    segmented: bool,
    v86: bool,
    trapped: bool,
}

impl Case {
    pub fn rig(self, board: BoardModel, chip: CpuChip) -> assembly::AssembledMachine {
        assert!(include_str!("../../../../Cargo.lock")
            .contains("3d793fc281debe02d998eb13ece3bec658a58edb"));
        let mut config = speed::board(self.rom, board);
        config.cpu.chip = chip;
        let mut rig = assembly::assemble_with_options(
            &config,
            AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap();
        // Enter through guest setup, keeping its REP work outside timed intervals.
        while u64::from(rig.machine.cpu().diagnostic_state().segments[1].base)
            + rig.machine.cpu().reg(CpuReg::Rip)
            != 0xfa000
        {
            assert!(
                rig.machine.virtual_time() < 100_000,
                "{}: setup did not enter user code: {}",
                self.name,
                rig.machine.cpu().diagnostic_state()
            );
            let cap = rig.machine.virtual_time() + 1;
            assert_eq!(
                rig.machine.run_bounded(Some(cap)).unwrap(),
                RunOutcome::RetiredCapReached,
                "{}: {}",
                self.name,
                rig.machine.cpu().diagnostic_state()
            );
        }
        self.check(&rig, 0);
        rig
    }

    pub fn iterations(self, retired: u64) -> u64 {
        let period = if self.v86 { 250 } else { 25 };
        assert_eq!(retired % period, 0);
        retired / period * if self.v86 { 10 } else { 1 }
    }

    pub fn check(self, rig: &assembly::AssembledMachine, iterations: u64) {
        let cpu = rig.machine.cpu();
        let state = cpu.diagnostic_state();
        let system = state.system.unwrap();
        assert!(system.protected());
        assert_eq!(system.paging(), self.paged);
        assert_eq!(system.cpl, 3);
        assert_eq!(system.virtual_8086, self.v86);
        assert_eq!(
            state.flags & 0x3000,
            if self.v86 && !self.trapped { 0x3000 } else { 0 }
        );
        assert_eq!(
            u64::from(state.segments[1].base) + cpu.reg(CpuReg::Rip),
            0xfa000,
            "{}: loop boundary",
            self.name
        );
        let selector = if self.v86 {
            0x4000
        } else if self.segmented {
            0x1f
        } else {
            0x23
        };
        if iterations != 0 || !self.segmented {
            for index in [0, 2, 3] {
                assert_eq!(state.segments[index].selector, selector);
                assert_eq!(
                    state.segments[index].base,
                    if self.segmented { 0x40000 } else { 0 }
                );
            }
        }
        assert_eq!(
            state.segments[1].selector,
            if self.v86 {
                0xfa00
            } else if self.segmented {
                0x0f
            } else {
                0x1b
            }
        );
        if !self.v86 {
            for segment in state.segments {
                if iterations == 0 && self.segmented && segment.selector == 0 {
                    continue;
                }
                assert_eq!(
                    segment.attributes & 0x4000,
                    if self.segmented { 0 } else { 0x4000 }
                );
                assert_eq!(
                    segment.limit,
                    if self.segmented { 0xffff } else { u32::MAX }
                );
            }
        }
        if self.segmented && !self.v86 {
            assert_eq!(system.ldtr.selector, 0x28);
            assert_eq!(system.ldtr.base, 0x900);
        }
        let mut memory = [0; 8];
        cpu.read_mem(0x40100, &mut memory).unwrap();
        let mask = if self.segmented { 0xffff } else { u32::MAX };
        let value = (iterations as u32).wrapping_mul(3) & mask;
        let result = if iterations == 0 {
            0
        } else {
            value.wrapping_add(1) & mask
        };
        if self.segmented {
            assert_eq!(
                u16::from_le_bytes(memory[..2].try_into().unwrap()) as u32,
                value
            );
            assert_eq!(
                u16::from_le_bytes(memory[2..4].try_into().unwrap()) as u32,
                result
            );
        } else {
            assert_eq!(u32::from_le_bytes(memory[..4].try_into().unwrap()), value);
            assert_eq!(u32::from_le_bytes(memory[4..].try_into().unwrap()), result);
        }
        let mut si = 0u32;
        let mut di = 0u32;
        let mut dx = 0u32;
        let mut cx = 0u32;
        for index in 1..=iterations {
            let ax = (index as u32).wrapping_mul(3).wrapping_add(1) & mask;
            si = si.wrapping_add(ax) & mask;
            di ^= si;
            dx = if self.segmented {
                ((dx ^ ax) as u16).rotate_left(1) as u32
            } else {
                (dx ^ ax).rotate_left(1)
            };
            cx = (cx.wrapping_add(7) ^ 0x1234) & mask;
            if !self.segmented {
                si = si.wrapping_add(cx);
                di ^= dx;
                cx = cx.rotate_right(1);
            }
        }
        let ax = if self.v86 && iterations != 0 {
            (result & 0xff00) | 0x5a
        } else {
            result
        };
        for (reg, expected) in [
            (CpuReg::Rax, ax as u64),
            (CpuReg::Rcx, cx as u64),
            (CpuReg::Rdx, dx as u64),
            (CpuReg::Rsi, si as u64),
            (CpuReg::Rdi, di as u64),
            (CpuReg::Rbp, iterations),
        ] {
            assert_eq!(cpu.reg(reg), expected, "{}: {reg:?}", self.name);
        }
        if iterations != 0 {
            assert_eq!(
                cpu.reg(CpuReg::Rbx),
                if self.segmented {
                    u64::from(selector + 1)
                } else {
                    0x40101
                }
            );
        }
        assert_eq!(
            cpu.reg(CpuReg::Rsp),
            if self.segmented { 0x8000 } else { 0x8800 }
        );
        let mut count = [0; 4];
        cpu.read_mem(0x6000, &mut count).unwrap();
        assert_eq!(
            u32::from_le_bytes(count),
            if self.trapped {
                (iterations / 10) as u32
            } else {
                0
            }
        );
        if self.paged && iterations != 0 {
            let mut pte = [0; 4];
            cpu.read_mem(0x2000 + 0xfa * 4, &mut pte).unwrap();
            assert_eq!(
                u32::from_le_bytes(pte) & 0x27,
                0x27,
                "user code page accessed"
            );
            for page in [0x40, if self.segmented { 0x47 } else { 8 }] {
                cpu.read_mem(0x2000 + page * 4, &mut pte).unwrap();
                assert_eq!(
                    u32::from_le_bytes(pte) & 0x67,
                    0x67,
                    "user data/stack page accessed and dirty"
                );
            }
            cpu.read_mem(0x2000, &mut pte).unwrap();
            assert_eq!(
                u32::from_le_bytes(pte) & 7,
                3,
                "tables stay supervisor-only"
            );
        }
    }
}

// Keep the chip list explicit in the test; the example also retains its 286 case.
pub const CHIPS: [(BoardModel, CpuChip); 3] = [
    (BoardModel::Generic386Isa, CpuChip::I80386),
    (BoardModel::Generic486Isa, CpuChip::I80486Dx),
    (BoardModel::Generic486Isa, CpuChip::I80486DxCpuid),
];
