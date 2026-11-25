// SPDX-License-Identifier: AGPL-3.0-or-later
//! CPU configuration (§2, §3.1).

use cpu::x86jit_backend::RealSegments;

/// The modelled CPU chip.
///
/// Selects the board interpreter's instruction generation, system semantics
/// and physical address width, and identifies it in golden traces. Timing is
/// the virtual clock's concern ([`crate::ClockConfig`]).
/// Intel 80286 and 80287 Programmer's Reference Manual, §10.2–10.4:
/// the 80286 starts in real mode and enters protected mode through LMSW.
/// Bare compatibility rigs without an A20 source retain the fixed Real16
/// application interpreter; rigs with an 8042 or port 92h use the machine model.
/// Intel486 DX Programmer's Reference Manual, §2 Basic Architecture and
/// §10.1 Initialization define the integrated FPU and 32-bit bus/reset.
/// See `backlog/docs/design/machine-config.md` §CPU model selection.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CpuChip {
    /// Intel 80286.
    #[default]
    I80286,
    /// Intel 80386DX with a 32-bit physical bus.
    I80386,
    /// Early Intel486 DX without CPUID.
    I80486Dx,
    /// Intel486 DX with EFLAGS.ID and CPUID.
    I80486DxCpuid,
}

impl CpuChip {
    /// Schema spelling for the selected processor chip.
    pub const fn name(self) -> &'static str {
        match self {
            Self::I80286 => "80286",
            Self::I80386 => "80386dx",
            Self::I80486Dx => "80486dx",
            Self::I80486DxCpuid => "80486dx-cpuid",
        }
    }

    /// Physical address bus width. IBM Personal Computer AT Technical Reference,
    /// Section 1 "System Board", I/O Channel: the 80286 AT has 24 address lines.
    /// Width is metadata, not a flat-VM allocation policy. The AT decode is
    /// fixed at 24 bits. Intel 80386 Programmer's Reference Manual, §2.1
    /// Memory Organization: the DX provides 32 physical address lines.
    pub const fn physical_address_bits(self) -> u8 {
        match self {
            Self::I80286 => 24,
            Self::I80386 | Self::I80486Dx | Self::I80486DxCpuid => 32,
        }
    }
}

/// Where the CPU starts executing after reset (§2.1).
///
/// The two forms capture *intent*, not different mechanisms — both fix a
/// real-mode `CS:IP`. A [`ResetSpec::ResetVector`] is the architectural power-on
/// vector, where a masked BIOS ROM ([`crate::LoadImage`] of kind
/// [`crate::ImageKind::Rom`]) supplies the first instruction; a
/// [`ResetSpec::Entry`] points straight at the entry of a blob loaded into RAM
/// (a bare boot blob or a diagnostic image).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResetSpec {
    /// The architectural reset vector; code is fetched from a ROM mapped there.
    ResetVector {
        /// Reset code-segment selector (real mode: base = selector << 4).
        cs: u16,
        /// Reset instruction pointer within the segment.
        ip: u16,
    },
    /// A direct entry into a RAM-loaded blob.
    Entry {
        /// Entry code-segment selector.
        cs: u16,
        /// Entry instruction pointer within the segment.
        ip: u16,
    },
}

impl ResetSpec {
    /// The standard PC/AT reset vector `F000:FFF0`.
    pub const fn reset_vector() -> Self {
        Self::ResetVector {
            cs: 0xF000,
            ip: 0xFFF0,
        }
    }

    /// The reset code-segment selector, whichever form this is.
    pub fn cs(&self) -> u16 {
        match self {
            Self::ResetVector { cs, .. } | Self::Entry { cs, .. } => *cs,
        }
    }

    /// The reset instruction pointer, whichever form this is.
    pub fn ip(&self) -> u16 {
        match self {
            Self::ResetVector { ip, .. } | Self::Entry { ip, .. } => *ip,
        }
    }
}

impl Default for ResetSpec {
    fn default() -> Self {
        Self::reset_vector()
    }
}

/// The real-mode data segments and stack pointer a guest starts with.
///
/// Mirrors [`cpu::x86jit_backend::RealSegments`] as owned, comparable data (the
/// CPU-core type is not [`Eq`], and the whole config must be). The reset `CS:IP`
/// is separate, in [`ResetSpec`]; this groups DS/ES/SS and the initial SP. A
/// [`From`] conversion into the CPU-core type is provided for the assembler.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct SegmentSeed {
    /// Data segment selector.
    pub ds: u16,
    /// Extra segment selector.
    pub es: u16,
    /// Stack segment selector.
    pub ss: u16,
    /// Initial stack pointer (offset within SS).
    pub sp: u16,
}

impl SegmentSeed {
    /// All three data segments equal to `sel` with the CPU-core default stack
    /// top — the flat low-memory layout a boot blob uses.
    pub fn flat(sel: u16) -> Self {
        Self {
            ds: sel,
            es: sel,
            ss: sel,
            sp: RealSegments::DEFAULT_SP,
        }
    }
}

impl From<SegmentSeed> for RealSegments {
    fn from(s: SegmentSeed) -> Self {
        RealSegments {
            ds: s.ds,
            es: s.es,
            ss: s.ss,
            sp: s.sp,
        }
    }
}

/// Optional 80287/80387 math coprocessor or integrated Intel486 DX FPU.
///
/// IBM Personal Computer AT Technical Reference, Section 2, Coprocessor:
/// the 80287 occupies an optional socket. Intel486 DX Programmer's Reference
/// Manual, §2 Basic Architecture defines the on-chip FPU. See
/// `backlog/docs/design/machine-config.md` §CPU model selection.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum FpuChip {
    /// Empty socket (the board default).
    #[default]
    None,
    /// Installed 80287, using the core's existing present x87 engine.
    I80287,
    /// Installed 80387. Intel 387 DX User's Manual, System Interface;
    /// Intel 80386 Programmer's Reference Manual, Chapter 11 Coprocessor Interface.
    I80387,
    /// On-chip Intel486 DX FPU; not an optional socket.
    Integrated,
}

/// The CPU section of a [`crate::MachineConfig`] (§2).
///
/// An execution model, a reset entry, and the seeded segments;
/// the home for future CPU options that must not force a config restructure.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CpuConfig {
    /// The modelled processor and its execution semantics.
    pub chip: CpuChip,
    /// Optional coprocessor on AT boards; mandatory integrated FPU on 486 DX.
    /// Bare rigs retain their constructor's FPU behaviour regardless of this field.
    pub fpu: FpuChip,
    /// Where execution begins after reset.
    pub reset: ResetSpec,
    /// The real-mode data segments and stack the guest starts with.
    pub segments: SegmentSeed,
}
