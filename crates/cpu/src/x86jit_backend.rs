// SPDX-License-Identifier: AGPL-3.0-or-later
//! `x86jit`-backed CPU (§2.1, §3.2 seam 1).
//!
//! [`X86JitCpu`] wraps an `x86jit-core` [`Vm`] + [`Vcpu`] and maps the core's
//! [`Exit`] variants onto the backend-neutral [`CpuStop`]. It runs in **Long64**
//! mode (adapter unit tests, [`X86JitCpu::new`]), **Real16** application mode
//! ([`X86JitCpu::new_real16_bare`]), or the **I286 machine** interpreter
//! ([`X86JitCpu::new_i286_bare`] and [`X86JitCpu::new_a20_bare`]), or the
//! **I386 machine** interpreter ([`X86JitCpu::new_i386_bare`]). Only the
//! machine profile consults A20; application profiles retain flat addressing.
//! It uses the
//! `x86jit` integration surface: `Exit::PortIo` (`in`/`out` and each `ins`/`outs`
//! element), `RegionKind::Trap` MMIO, and — in real mode — IVT-vectored IRQ injection.
//!
//! `x86jit-core` is depended on through a git dependency pinned to an exact
//! revision rather than a plain path, so a clean checkout or CI resolves the same
//! CPU without any local x86jit checkout present.

use x86jit_core::{
    CpuMode, CpuModel, CpuState, ExceptionDelivery, ExecutionProfile, Exit, FpuModel,
    NumericErrorEvent, NumericErrorRoute, PortDir, Prot, Reg, RegionKind, ResetOptions,
    StepOptions, StepRunError, StepStop, Vcpu, Vm, VmConfig,
};

use crate::{Cpu, CpuError, CpuReg, CpuStepRun, CpuStepStop, CpuStop, DmaAccess, StepRunOptions};

/// Consecutive deliveries that retire nothing, after which the core ends a
/// machine-profile run with its budget unspent; the value `x86jit-core`
/// documents on `Vcpu::run_steps`.
pub const DELIVERY_STALL_LIMIT: u32 = 1024;

/// Bounded returns in a row, with no instruction retired, that one
/// [`Cpu::run_steps`] call runs through before it reports
/// [`CpuError::DeliveryStalled`]: 11 × 1024 = 11,264 deliveries. A frame on a
/// 16-bit stack takes at least 6 bytes, so within ⌈65,536 / 6⌉ = 10,923
/// deliveries the frames reach the bottom of the stack. In real mode a frame
/// wraps inside the segment and faults only where a word sits at offset
/// FFFFh: from an odd SP a frame soon does (at SP 1, 3 or 5) and the
/// processor shuts down, which the board answers with a reset (Intel 80286
/// Programmer's Reference Manual, §9.6.2 and Appendix B, INT); from an even
/// SP the frames wrap without a fault and the loop has no end. In protected
/// mode the frame must fit: where SP is not a multiple of the frame size (or
/// the limit is below FFFFh) the next frame overruns the stack and the
/// processor shuts down; where it is, the frames wrap and the loop has no
/// end.
pub const STALL_RETURNS: u32 = 11;

/// The linear address of byte `index` after a saved CS:IP whose linear
/// address is `linear`. IP is the 16-bit offset of the next instruction in
/// the code segment on the 80286, the only model this backend selects
/// (Intel 80286 Programmer's Reference Manual, Chapter 2), so the bytes wrap
/// at the end of the segment, back to its base.
fn code_byte_address(linear: u32, ip: u32, index: usize) -> u32 {
    let base = linear.wrapping_sub(ip);
    base.wrapping_add(ip.wrapping_add(index as u32) & 0xFFFF)
}

/// A CPU backed by the `x86jit` interpreter core (§2.1).
///
/// Owns the guest [`Vm`] (its memory map) and the [`Vcpu`]. The machine loop
/// drives it through the [`Cpu`] trait; it never exposes `x86jit` types.
pub struct X86JitCpu {
    vm: Vm,
    cpu: Vcpu,
    dma_regions: Vec<(std::ops::Range<u64>, RegionKind)>,
    completed_steps: u64,
    // Synthetic rigs retain their low reset fetch without the AT high ROM alias.
    low_reset_fetch: bool,
    /// The buffer each read of the core's delivery log reuses.
    delivery_records: Vec<x86jit_core::DeliveryRecord>,
}

/// Guest backing allocation diagnostics, excluding tracking metadata and RSS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBackingStats {
    /// Whether the backing commits mapped regions within a sparse reservation.
    pub sparse: bool,
    /// Host virtual address span reserved for guest memory.
    pub reserved_bytes: u64,
    /// Committed backing bytes, or `None` when the backing owner cannot report them.
    /// Owned flat and sparse backings report a count; sparse regions round to host pages.
    pub committed_bytes: Option<u64>,
}

impl X86JitCpu {
    /// Inspect backing commitment independently of the guest's address span.
    pub fn memory_backing_stats(&self) -> MemoryBackingStats {
        let stats = self.vm.memory_stats();
        MemoryBackingStats {
            sparse: self.vm.mem.is_sparse(),
            reserved_bytes: stats.guest_ram_reserved,
            committed_bytes: stats.guest_ram_committed,
        }
    }

    /// Build a CPU over a flat guest address space of `ram_bytes`, load `code`
    /// at `entry`, and set RIP there.
    ///
    /// The whole space is mapped `RWX` `Ram`; MMIO-trapped windows are added
    /// with [`X86JitCpu::map_mmio`] before execution. Code is written through the
    /// `Vm` API so the core's self-modifying-code tracking sees the write — a raw
    /// buffer poke would silently break translated-block invalidation.
    pub fn new(ram_bytes: u64, entry: u64, code: &[u8]) -> Result<Self, CpuError> {
        let mut vm = Vm::new(VmConfig::flat(ram_bytes));
        vm.map(0, ram_bytes as usize, Prot::RWX, RegionKind::Ram)
            .map_err(|e| CpuError::Memory(format!("{e:?}")))?;
        vm.write_bytes(entry, code)
            .map_err(|e| CpuError::Memory(format!("{e:?}")))?;
        let mut cpu = vm.new_vcpu();
        cpu.set_a20_enabled(true);
        cpu.set_reg(Reg::Rip, entry);
        Ok(Self {
            vm,
            cpu,
            dma_regions: vec![(0..ram_bytes, RegionKind::Ram)],
            completed_steps: 0,
            low_reset_fetch: true,
            delivery_records: Vec::new(),
        })
    }

    /// Build a **real-mode (16-bit)** CPU over a flat guest address space of
    /// `space_bytes` with **no regions mapped** — the embedder lays out the whole
    /// memory map itself with [`X86JitCpu::map_ram`], [`X86JitCpu::map_rom`], and
    /// [`X86JitCpu::map_mmio`].
    ///
    /// This is the constructor a full PC needs: its address space is a mosaic of
    /// conventional RAM, a trapped video window, an option-ROM scan area, and a
    /// masked BIOS ROM at the top — not the single flat RAM block the built-in
    /// demos use. Nothing is loaded here (a BIOS ROM is its own code, mapped with
    /// [`X86JitCpu::map_rom`]); this only seeds the segment and stack registers and
    /// points CS:IP at the reset entry.
    ///
    /// The segment registers are seeded here so that a guest entered at an
    /// arbitrary CS:IP (a test program or a reset vector) has defined DS, ES, SS
    /// and SP before its first instruction; the guest may reload them itself.
    pub fn new_real16_bare(
        space_bytes: u64,
        cs: u16,
        ip: u16,
        segments: RealSegments,
    ) -> Result<Self, CpuError> {
        let mut vm = Vm::new(VmConfig::flat(space_bytes));
        vm.set_cpu_mode(CpuMode::Real16);
        let mut cpu = vm.new_vcpu();
        cpu.set_a20_enabled(true);
        cpu.set_reg(Reg::Cs, u64::from(cs));
        cpu.set_reg(Reg::Ds, u64::from(segments.ds));
        cpu.set_reg(Reg::Es, u64::from(segments.es));
        cpu.set_reg(Reg::Ss, u64::from(segments.ss));
        cpu.set_reg(Reg::Rsp, u64::from(segments.sp));
        cpu.set_reg(Reg::Rip, u64::from(ip));
        Ok(Self {
            vm,
            cpu,
            dma_regions: Vec::new(),
            completed_steps: 0,
            low_reset_fetch: true,
            delivery_records: Vec::new(),
        })
    }

    /// Build an Intel 80286 machine interpreter at a real-mode entry.
    ///
    /// Intel 80286 and 80287 Programmer's Reference Manual, §10.2–10.4:
    /// LMSW can enter protected mode; leaving it requires a processor reset.
    /// Unlike [`Self::new_real16_bare`], this profile retains descriptor caches
    /// and delivers real/protected exceptions in the guest through IVT/IDT.
    /// It enforces 286 instruction and segment limits. See
    /// `backlog/docs/design/protected-mode-gaps.md` §Context.
    /// The address space must include any reset ROM at physical FFFFF0h.
    /// Retains a present FPU for standalone compatibility rigs; board assembly
    /// selects its socket policy through [`Self::new_i286_bare_with_fpu`].
    pub fn new_i286_bare(
        space_bytes: u64,
        cs: u16,
        ip: u16,
        segments: RealSegments,
    ) -> Result<Self, CpuError> {
        Self::new_i286_bare_with_fpu(space_bytes, cs, ip, segments, true)
    }

    /// Build the 80286 machine profile with an optional coprocessor.
    ///
    /// IBM Personal Computer AT Technical Reference, Section 2, Coprocessor:
    /// the 80287 is optional. The core's present model supplies its existing
    /// x87 engine without claiming generation-specific 80287 behaviour.
    /// Intel 80286 and 80287 Programmer's Reference Manual, Part I Appendix A
    /// and Part II Appendix B, item 10: an empty socket supplies an all-high
    /// status/control word under the core's board policy. See
    /// `backlog/docs/design/machine-config.md` §CPU model selection.
    pub fn new_i286_bare_with_fpu(
        space_bytes: u64,
        cs: u16,
        ip: u16,
        segments: RealSegments,
        fpu_present: bool,
    ) -> Result<Self, CpuError> {
        Self::new_machine_bare(
            space_bytes,
            cs,
            ip,
            segments,
            fpu_present,
            false,
            CpuModel::I80286,
        )
    }

    /// Build an Intel 80386DX machine at a real-mode entry with no coprocessor.
    ///
    /// Intel 80386 Programmer's Reference Manual, §10.1 Initialization: the
    /// reset fetch uses the top of the 32-bit physical bus. Sparse backing
    /// reserves 4 GiB but commits only mapped RAM, MMIO and BIOS copies.
    /// See `backlog/docs/design/machine-files.md` §Generic 386 ISA board.
    pub fn new_i386_bare(cs: u16, ip: u16, segments: RealSegments) -> Result<Self, CpuError> {
        Self::new_machine_bare(1 << 32, cs, ip, segments, false, false, CpuModel::I80386)
    }

    /// Build an Intel486 DX with its on-chip FPU and explicit CPUID policy.
    /// Intel486 DX Programmer's Reference Manual, §10.1 Initialization:
    /// both variants reset with CR0.CD/NW/ET set. External FERR routing is
    /// unavailable in the core, so retain internal numeric-error delivery.
    /// See `backlog/docs/design/machine-files.md` §Generic 486 ISA board.
    pub fn new_i486_bare(
        cs: u16,
        ip: u16,
        segments: RealSegments,
        cpuid: bool,
    ) -> Result<Self, CpuError> {
        Self::new_machine_bare(
            1 << 32,
            cs,
            ip,
            segments,
            true,
            false,
            if cpuid {
                CpuModel::I80486DxCpuid
            } else {
                CpuModel::I80486Dx
            },
        )
    }

    fn new_machine_bare(
        space_bytes: u64,
        cs: u16,
        ip: u16,
        segments: RealSegments,
        fpu_present: bool,
        low_reset_fetch: bool,
        model: CpuModel,
    ) -> Result<Self, CpuError> {
        let fpu = if fpu_present {
            FpuModel::Present
        } else {
            FpuModel::None
        };
        let mut vm = if matches!(
            model,
            CpuModel::I80386 | CpuModel::I80486Dx | CpuModel::I80486DxCpuid
        ) {
            Vm::sparse(VmConfig::reserved(space_bytes))
                .map_err(|e| CpuError::Memory(format!("{e:?}")))?
        } else {
            Vm::new(VmConfig::flat(space_bytes))
        };
        vm.set_cpu_model(model);
        vm.set_exception_delivery(ExceptionDelivery::InGuest);
        vm.set_fpu_model(fpu);
        let external = fpu_present && matches!(model, CpuModel::I80286 | CpuModel::I80386);
        if external {
            vm.set_numeric_error_route(NumericErrorRoute::External)
                .map_err(|e| CpuError::UnexpectedExit {
                    what: format!("numeric error route: {e:?}"),
                })?;
        }
        let mut cpu = vm.new_vcpu();
        cpu.cpu = CpuState::legacy_real_entry([cs, segments.ds, segments.es, segments.ss, 0, 0]);
        // Installing the real-mode entry replaces the FPU policy inherited
        // from the VM, so restore the board's choice on the vCPU as well.
        cpu.set_fpu_model(fpu);
        if external {
            cpu.set_numeric_error_route(NumericErrorRoute::External)
                .map_err(|e| CpuError::UnexpectedExit {
                    what: format!("numeric error route: {e:?}"),
                })?;
        }
        cpu.cpu.legacy.cr0 = if matches!(model, CpuModel::I80486Dx | CpuModel::I80486DxCpuid) {
            let reset = CpuState::i486_reset(model);
            cpu.cpu.machine_flags_mask = reset.machine_flags_mask;
            cpu.cpu.gpr[2] = reset.gpr[2];
            reset.legacy.cr0
        } else if model == CpuModel::I80386 {
            0x10
        } else {
            0xFFF0
        };
        cpu.set_reg(Reg::Rsp, u64::from(segments.sp));
        cpu.set_reg(Reg::Rip, u64::from(ip));
        cpu.set_fpu_emulation(false);
        Ok(Self {
            vm,
            cpu,
            dma_regions: Vec::new(),
            completed_steps: 0,
            low_reset_fetch,
            delivery_records: Vec::new(),
        })
    }

    /// Build an A20-capable bare rig using the I80286 machine interpreter.
    ///
    /// Application Real16 ignores A20, so attaching a board line requires a
    /// machine profile. The rig keeps its low FFFF0h reset entry rather than
    /// requiring the AT's high ROM alias (Intel 80286 PRM §5.3 System
    /// Initialization). It retains a present FPU and delivers processor exceptions
    /// in the guest through the IVT/IDT, like the AT machine profile (Intel 80286
    /// PRM §9, Interrupts and Exceptions). See machine-config.md §CPU model selection.
    pub fn new_a20_bare(
        space_bytes: u64,
        cs: u16,
        ip: u16,
        segments: RealSegments,
    ) -> Result<Self, CpuError> {
        Self::new_machine_bare(space_bytes, cs, ip, segments, true, true, CpuModel::I80286)
    }

    /// Map a conventional RAM window `[base, base + size)` as `RWX` `Ram`.
    ///
    /// For an embedder building a mosaic address space with
    /// [`X86JitCpu::new_real16_bare`]: low conventional memory and upper-memory
    /// probe areas can be RAM. Firmware scan space on catalogue boards uses
    /// read-only ROM mappings, including deliberately zero-filled gaps.
    pub fn map_ram(&mut self, base: u64, size: u64) -> Result<(), CpuError> {
        self.map_region(base, size as usize, Prot::RWX, RegionKind::Ram)
    }

    /// Map a masked **ROM** window at `base` sized to `blob` and load `blob` into
    /// it (a BIOS or option ROM).
    ///
    /// The region is `RegionKind::Rom`: fetches and reads run at RAM speed and
    /// code executes straight out of it, while a guest store into it is silently
    /// dropped (the masked-ROM contract) rather than faulting. Contents are loaded
    /// through the `Vm` write path, which bypasses the region kind — the only way
    /// to populate a ROM, which is immutable to the guest thereafter.
    pub fn map_rom(&mut self, base: u64, blob: &[u8]) -> Result<(), CpuError> {
        self.map_region(base, blob.len(), Prot::RX, RegionKind::Rom)?;
        self.write_mem(base, blob)
    }

    /// Map one immutable region filled with a repeated byte, using at most
    /// 4 KiB of temporary storage. One region preserves accesses spanning fill
    /// chunks; mapping each chunk separately would introduce artificial faults.
    /// See `backlog/docs/design/machine-config.md` §MemoryConfig.
    pub fn map_rom_filled(&mut self, base: u64, size: u64, value: u8) -> Result<(), CpuError> {
        self.map_region(base, size as usize, Prot::RX, RegionKind::Rom)?;
        let fill = [value; 4096];
        for offset in (0..size).step_by(fill.len()) {
            let count = (size - offset).min(fill.len() as u64) as usize;
            self.write_mem(base + offset, &fill[..count])?;
        }
        Ok(())
    }

    /// Map one immutable region over the bytes the flat backing already holds
    /// at `[base, base + size)`, without loading anything.
    ///
    /// For a range released with [`Self::unmap`]: the core's flat model keeps
    /// the backing bytes across unmap and map, so an embedder that re-tags a
    /// range it already filled (an unbacked EMS window reading FFh, §4.2) need
    /// not rewrite it. Load new contents with [`Self::write_mem`], which can
    /// write a ROM. Guest stores are dropped, as for [`Self::map_rom`].
    pub fn map_rom_in_place(&mut self, base: u64, size: u64) -> Result<(), CpuError> {
        self.map_region(base, size as usize, Prot::RX, RegionKind::Rom)
    }

    /// Write `bytes` at physical address `addr` through the `Vm` loader API.
    ///
    /// Host edits may replace page tables without a guest CR3 reload. Flush
    /// derived paging state even for equal-byte restores, as well as notifying
    /// SMC tracking. Call only between runs; complete pending device handoffs
    /// before restoring memory. Intel 80386 Programmer's Reference Manual,
    /// §5.2.5 Page-Translation Cache; see protected-mode-gaps.md §Paging restore audit.
    pub fn write_mem(&mut self, addr: u64, bytes: &[u8]) -> Result<(), CpuError> {
        self.vm
            .write_bytes(addr, bytes)
            .map_err(|e| CpuError::Memory(format!("{e:?}")))?;
        self.cpu.invalidate_paging();
        Ok(())
    }

    /// Read `buf.len()` bytes at physical address `addr` through the `Vm` API.
    /// Host inspection bypasses guest paging, as does [`Self::write_mem`].
    pub fn read_mem(&self, addr: u64, buf: &mut [u8]) -> Result<(), CpuError> {
        self.vm
            .read_bytes(addr, buf)
            .map_err(|e| CpuError::Memory(format!("{e:?}")))
    }

    /// The current code-segment selector (real mode). The [`Cpu`] trait exposes
    /// only the general-purpose registers and RIP; a runner that wants the linear
    /// CS:IP of the stopping instruction reads the segment here.
    pub fn code_segment(&self) -> u64 {
        self.cpu.reg(Reg::Cs)
    }

    /// Map a memory-mapped-I/O (trapped) window `[base, base + size)`.
    ///
    /// Loads/stores in this range yield `Exit::MmioRead`/`Exit::MmioWrite`,
    /// surfaced as [`CpuStop::MmioRead`]/[`CpuStop::MmioWrite`] (§4.2, §5.2).
    pub fn map_mmio(&mut self, base: u64, size: u64) -> Result<(), CpuError> {
        self.map_region(base, size as usize, Prot::RW, RegionKind::Trap)
    }

    /// Remove the region mapped at exactly `[base, base + size)`, so the range
    /// can be mapped again with another kind (an EMS window that switches
    /// between CPU RAM, ROM and a trapped board window, §4.2).
    ///
    /// The core drops cached translations overlapping the range; its flat
    /// model only removes the region's tag, so the backing bytes stay in place
    /// until they are rewritten. Mapping a trapped region
    /// afterwards also invalidates the whole translation cache, so this is a
    /// port-write-rate operation, not a per-access one. A range that is not
    /// one whole mapped region is refused and the map is left unchanged.
    pub fn unmap(&mut self, base: u64, size: u64) -> Result<(), CpuError> {
        self.vm
            .unmap(base, size as usize)
            .map_err(|e| CpuError::Memory(format!("{e:?}")))?;
        let end = base + size;
        self.dma_regions
            .retain(|(range, _)| range.start != base || range.end != end);
        Ok(())
    }

    fn map_region(
        &mut self,
        base: u64,
        size: usize,
        prot: Prot,
        kind: RegionKind,
    ) -> Result<(), CpuError> {
        self.vm
            .map(base, size, prot, kind)
            .map_err(|e| CpuError::Memory(format!("{e:?}")))?;
        // Vm's host loader API bypasses region kinds. Keep the successful maps
        // here so DMA cycles obey the guest map without involving any devices.
        self.dma_regions.push((base..base + size as u64, kind));
        Ok(())
    }

    /// The diagnosis of a stop this CPU returned, for the errors that name
    /// an instruction: an unsupported system operation, an unsupported
    /// instruction, an outward fault, a stall in exception delivery,
    /// whose instruction is the handler's first, and a reset storm, whose
    /// instruction is the one at the last shutdown, before its reset;
    /// `None` for every other error.
    ///
    /// Call it right after the failed run, before anything else runs the
    /// CPU: the core refuses without a guest effect, so the registers and
    /// descriptor caches are those of the refused instruction. See
    /// [`crate::stop_report`] for the contents. Only RAM and ROM are read,
    /// through the CPU's own map and its address lines: the 80286's 24-bit
    /// bus and, in the machine profile, the A20 gate (IBM PC AT Technical
    /// Reference, Keyboard Controller, Output Port). Trapped windows are
    /// never read, so the report has no device side effect.
    pub fn stop_report(&self, error: &CpuError) -> Option<crate::CpuStopReport> {
        use x86jit_core::legacy::SegmentCache;
        let (cs, ip) = match error {
            CpuError::UnsupportedSystem { cs, ip, .. } => (*cs, *ip),
            CpuError::UnsupportedInstruction { cs, addr, .. } => (*cs, *addr),
            CpuError::Fault { cs, ip, .. } => (*cs, *ip),
            CpuError::DeliveryStalled { cs, ip, .. } => (*cs, *ip),
            CpuError::ResetStorm { cs, ip, .. } => (*cs, *ip),
            _ => return None,
        };
        let state = &self.cpu.cpu;
        let machine = self.vm.execution_profile() == ExecutionProfile::Machine;
        let word = |reg| self.cpu.reg(reg) as u16;
        let registers = crate::stop_report::AddressRegisters {
            bx: word(Reg::Rbx),
            bp: word(Reg::Rbp),
            si: word(Reg::Rsi),
            di: word(Reg::Rdi),
        };
        let legacy = &state.legacy;
        let view = |selector: u16, cache: SegmentCache| crate::stop_report::SegmentView {
            selector,
            base: cache.base,
            limit: cache.limit,
            attributes: cache.attributes,
            unusable: cache.unusable != 0,
        };
        let system = machine.then(|| crate::stop_report::SystemState {
            msw: legacy.cr0 as u16,
            virtual_8086: state.rflags_system & (1 << 17) != 0,
            cpl: legacy.cpl,
            a20: self.cpu.a20_enabled(),
            cs: view(state.cs, legacy.segments[0]),
            ss: view(state.ss, legacy.segments[3]),
            gdtr: crate::stop_report::TableView {
                base: legacy.gdtr.base,
                limit: legacy.gdtr.limit,
            },
            idtr: crate::stop_report::TableView {
                base: legacy.idtr.base,
                limit: legacy.idtr.limit,
            },
            ldtr: view(legacy.ldtr.selector, legacy.ldtr.cache),
            tr: view(legacy.tr.selector, legacy.tr.cache),
        });
        // The machine profile keeps hidden bases; the application profiles
        // address real-mode style from the visible selectors.
        let bases = if machine {
            [2, 0, 3, 1].map(|segment| legacy.segments[segment].base)
        } else {
            [state.es, state.cs, state.ss, state.ds].map(|selector| u32::from(selector) << 4)
        };
        let read = |linear: u32| self.peek_linear(linear).value();
        Some(crate::CpuStopReport::build(
            crate::stop_report::Snapshot {
                stop: error.to_string(),
                cs,
                ip,
                ss: state.ss,
                sp: word(Reg::Rsp),
                registers,
                bases,
                system,
            },
            read,
        ))
    }

    /// What the CPU's own map holds at linear address `linear`: the
    /// model's address lines (24 bits on the 80286) and, in the machine
    /// profile, the A20 gate are applied. RAM and ROM are read; a trapped
    /// window is reported, never read, so the read has no device effect.
    fn peek_linear(&self, linear: u32) -> crate::diagnostic::Peek<u8> {
        use crate::diagnostic::Peek;
        let machine = self.vm.execution_profile() == ExecutionProfile::Machine;
        // The model's formed physical address: 24 bits on the 80286.
        let address_mask = self.vm.cpu_model().map_or(u64::MAX, |model| {
            (1u64 << model.profile().physical_bits) - 1
        });
        let mut physical = u64::from(linear) & address_mask;
        if machine && !self.cpu.a20_enabled() {
            physical &= !0x10_0000;
        }
        let Ok(address) = u32::try_from(physical) else {
            return Peek::Unmapped;
        };
        match self.dma_region_kind(address) {
            Some(RegionKind::Ram | RegionKind::Rom) => {
                let mut byte = [0];
                match self.vm.read_bytes(physical, &mut byte) {
                    Ok(()) => Peek::Value(byte[0]),
                    Err(_) => Peek::Unmapped,
                }
            }
            Some(RegionKind::Trap) => Peek::Device,
            _ => Peek::Unmapped,
        }
    }

    /// One record of the core's delivery log in the backend-neutral shape,
    /// with the bytes at its saved CS:IP read as [`Self::peek_linear`] reads
    /// them; see [`crate::delivery`].
    fn delivery(&self, record: &x86jit_core::DeliveryRecord) -> crate::Delivery {
        use crate::DeliveryKind;
        use x86jit_core::DeliveryKind as Core;
        let kind = match record.kind {
            Core::Fault => DeliveryKind::Fault,
            Core::Trap => DeliveryKind::Trap,
            Core::Software => DeliveryKind::Software,
            Core::Irq => DeliveryKind::Irq,
            Core::Nmi => DeliveryKind::Nmi,
            _ => DeliveryKind::Other,
        };
        crate::Delivery {
            retired: record.retired,
            kind,
            vector: record.vector,
            irq: None,
            cs: record.saved_cs,
            ip: record.saved_ip,
            linear: record.saved_linear,
            error_code: record.error_code,
            cpl_before: record.cpl_before,
            cpl_after: record.cpl_after,
            virtual_8086_before: record.virtual_8086_before,
            virtual_8086_after: record.virtual_8086_after,
            bytes: std::array::from_fn(|index| {
                self.peek_linear(code_byte_address(
                    record.saved_linear,
                    record.saved_ip,
                    index,
                ))
            }),
            regs: record.regs.as_deref().map(|regs| crate::DeliveryRegisters {
                eax: regs.eax,
                ebx: regs.ebx,
                ecx: regs.ecx,
                edx: regs.edx,
                esi: regs.esi,
                edi: regs.edi,
                ebp: regs.ebp,
                esp: regs.esp,
                ds: regs.ds,
                es: regs.es,
                ss: regs.ss,
                fs: regs.fs,
                gs: regs.gs,
                flags: regs.flags,
            }),
        }
    }

    /// The processor at this completed run boundary, for a diagnostic
    /// report of a running machine: registers, FLAGS, the system registers
    /// and descriptor caches of the machine profile, the bytes at CS:IP and
    /// the words at SS:SP. See [`crate::diagnostic`]. It only reads: memory
    /// through the CPU's own map, never a trapped window, and no
    /// register or counter changes, so taking it between runs leaves the
    /// guest and its timing as they were.
    pub fn diagnostic_state(&self) -> crate::diagnostic::CpuDiagnostic {
        use crate::stop_report::{SegmentView, TableView};
        use x86jit_core::legacy::SegmentCache;
        let state = &self.cpu.cpu;
        let machine = self.vm.execution_profile() == ExecutionProfile::Machine;
        let legacy = &state.legacy;
        let view = |selector: u16, cache: SegmentCache| SegmentView {
            selector,
            base: cache.base,
            limit: cache.limit,
            attributes: cache.attributes,
            unusable: cache.unusable != 0,
        };
        // ES, CS, SS, DS; the core's cache order is CS, DS, ES, SS.
        let selectors = [state.es, state.cs, state.ss, state.ds];
        let segments = if machine {
            let order = [2, 0, 3, 1];
            std::array::from_fn(|at| view(selectors[at], legacy.segments[order[at]]))
        } else {
            selectors.map(|selector| SegmentView {
                selector,
                base: u32::from(selector) << 4,
                limit: 0xFFFF,
                attributes: 0,
                unusable: false,
            })
        };
        let system = machine.then(|| crate::diagnostic::SystemRegisters {
            msw: legacy.cr0 as u16,
            virtual_8086: state.rflags_system & (1 << 17) != 0,
            cpl: legacy.cpl,
            gdtr: TableView {
                base: legacy.gdtr.base,
                limit: legacy.gdtr.limit,
            },
            idtr: TableView {
                base: legacy.idtr.base,
                limit: legacy.idtr.limit,
            },
            ldtr: view(legacy.ldtr.selector, legacy.ldtr.cache),
            tr: view(legacy.tr.selector, legacy.tr.cache),
        });
        let word = |reg| self.cpu.reg(reg) as u16;
        crate::diagnostic::CpuDiagnostic::build(
            crate::diagnostic::Registers {
                registers: [
                    Reg::Rax,
                    Reg::Rcx,
                    Reg::Rdx,
                    Reg::Rbx,
                    Reg::Rsp,
                    Reg::Rbp,
                    Reg::Rsi,
                    Reg::Rdi,
                ]
                .map(word),
                ip: state.rip as u16,
                flags: state.rflags() as u16,
                segments,
                system,
                a20: !machine || self.cpu.a20_enabled(),
                irq_queued: self.cpu.has_pending_irq(),
                irq_ready: self.cpu.irq_ready(),
                retired: self.cpu.retired_instructions(),
                completed_steps: self.completed_steps,
            },
            |linear| self.peek_linear(linear),
        )
    }

    fn dma_region_kind(&self, addr: u32) -> Option<RegionKind> {
        self.dma_regions
            .iter()
            .find(|(range, _)| range.contains(&u64::from(addr)))
            .map(|(_, kind)| *kind)
    }
}

/// The data-segment selectors and stack pointer a real-mode guest starts with
/// (§17.6).
///
/// CS and IP are passed separately to [`X86JitCpu::new_real16_bare`] (they fix the
/// code-fetch address); this groups DS/ES/SS and the initial SP. A boot blob that
/// runs from a single flat low-memory segment uses [`RealSegments::flat`], which
/// sets the three data segments equal to the code segment and picks a default
/// stack top clear of the IVT and typical boot code.
#[derive(Copy, Clone, Debug, Default)]
pub struct RealSegments {
    /// Data segment selector.
    pub ds: u16,
    /// Extra segment selector.
    pub es: u16,
    /// Stack segment selector.
    pub ss: u16,
    /// Initial stack pointer (offset within SS).
    pub sp: u16,
}

impl RealSegments {
    /// Default flat stack top: below typical boot code at `0x7C00` and above the
    /// IVT/BIOS data area, giving the guest a working stack without setup code.
    pub const DEFAULT_SP: u16 = 0x7000;

    /// All three data segments equal to `sel` with the default stack top — the
    /// flat low-memory layout a boot blob uses.
    pub fn flat(sel: u16) -> Self {
        Self {
            ds: sel,
            es: sel,
            ss: sel,
            sp: Self::DEFAULT_SP,
        }
    }
}

/// Map the neutral [`CpuReg`] onto the core's [`Reg`].
fn to_core_reg(reg: CpuReg) -> Reg {
    match reg {
        CpuReg::Rax => Reg::Rax,
        CpuReg::Rcx => Reg::Rcx,
        CpuReg::Rdx => Reg::Rdx,
        CpuReg::Rbx => Reg::Rbx,
        CpuReg::Rsp => Reg::Rsp,
        CpuReg::Rbp => Reg::Rbp,
        CpuReg::Rsi => Reg::Rsi,
        CpuReg::Rdi => Reg::Rdi,
        CpuReg::Rip => Reg::Rip,
    }
}

impl X86JitCpu {
    fn map_exit(&self, exit: Exit) -> Result<CpuStop, CpuError> {
        match exit {
            Exit::PortIo {
                port,
                size,
                dir,
                value,
                ..
            } => match dir {
                PortDir::In => Ok(CpuStop::PortIn { port, size }),
                PortDir::Out => Ok(CpuStop::PortOut { port, size, value }),
            },
            // Each MMIO exit is one complete transfer at its own address and
            // width. The fields skipped here only place that transfer inside a
            // wider operand, which the core delivers as several exits at the same
            // instruction, so servicing `addr`/`size` alone is still exact.
            Exit::MmioRead { addr, size, .. } => Ok(CpuStop::MmioRead { addr, size }),
            Exit::MmioWrite {
                addr, size, value, ..
            } => Ok(CpuStop::MmioWrite { addr, size, value }),
            Exit::ExternalNumericError { event, .. } => Ok(match event {
                NumericErrorEvent::Asserted => CpuStop::NumericErrorAsserted,
                NumericErrorEvent::Waiting => CpuStop::NumericErrorWaiting,
            }),
            Exit::Hlt => Ok(CpuStop::Halt),
            Exit::Shutdown { .. } => Ok(CpuStop::Shutdown),
            Exit::Exception {
                addr,
                vector,
                ref metadata,
                ..
            } if self.vm.execution_profile() == ExecutionProfile::Machine => {
                Err(CpuError::Fault {
                    vector,
                    error_code: metadata.as_deref().and_then(|fault| {
                        // The 286 supplies a complete 16-bit error code. The
                        // core's 32-bit projection requires a wider known mask.
                        match fault.error_code() {
                            x86jit_core::fault::ErrorCode::Bits {
                                value, known_mask, ..
                            } if known_mask & 0xFFFF == 0xFFFF => Some(value & 0xFFFF),
                            _ => None,
                        }
                    }),
                    cs: self.cpu.cpu.cs,
                    ip: addr,
                })
            }
            Exit::Exception { vector, .. } => Ok(CpuStop::Exception { vector }),
            Exit::UnsupportedSystemContext {
                addr, operation, ..
            } => Err(CpuError::UnsupportedSystem {
                operation: format!("{operation:?}"),
                cs: self.cpu.cpu.cs,
                ip: addr,
            }),
            Exit::BudgetExhausted => Ok(CpuStop::BudgetExhausted),
            Exit::UnmappedMemory { addr, .. } => Err(CpuError::Unmapped { addr }),
            Exit::UnknownInstruction {
                addr, bytes, len, ..
            } => Err(CpuError::UnsupportedInstruction {
                addr,
                cs: self.cpu.cpu.cs,
                bytes: bytes[..usize::from(len).min(bytes.len())].to_vec(),
            }),
            // Only `syscall`, or `int 0x80` outside real mode, end up
            // here: in real mode every `int n` is delivered inside the guest
            // through the IVT, and the long-mode serial programs make no system
            // calls. The evidence retains the instruction address after RIP
            // has advanced, so diagnostics identify the opcode to implement.
            Exit::Syscall => {
                let evidence =
                    self.cpu
                        .syscall_evidence()
                        .ok_or_else(|| CpuError::UnexpectedExit {
                            what: "syscall exit without instruction evidence".to_string(),
                        })?;
                Err(CpuError::UnsupportedInstruction {
                    addr: evidence.rip(),
                    cs: self.cpu.cpu.cs,
                    bytes: Vec::new(),
                })
            }
            // Nothing in this backend requests a host yield, and the interpreter
            // the machine runs on never fills a JIT code cache; either stop means
            // the embedding is not what this backend expects.
            Exit::CodeSpaceExhausted { addr, .. } => Err(CpuError::UnexpectedExit {
                what: format!("jit code space exhausted at {addr:#x}"),
            }),
            Exit::Interrupted => Err(CpuError::UnexpectedExit {
                what: "interrupted by a host yield request".to_string(),
            }),
            // The core may add exit kinds; report one by name rather than guess.
            other => Err(CpuError::UnexpectedExit {
                what: format!("{other:?}"),
            }),
        }
    }
}

/// The real-mode caches at CS F000h used before applying the board's reset
/// fetch alias (Intel 80286 Programmer's Reference Manual, §5.1).
/// Construct only the legacy fields so reset needs neither a whole CPU state
/// nor a process-wide cache (core-contract.md §12.10).
fn reset_legacy_image() -> x86jit_core::legacy::LegacyCpuState {
    use x86jit_core::legacy::{LegacyCpuState, SegmentCache, TableRegister};

    LegacyCpuState {
        idtr: TableRegister {
            limit: 0x03FF,
            ..TableRegister::default()
        },
        segments: std::array::from_fn(|index| SegmentCache {
            base: if index == 0 { 0xF0000 } else { 0 },
            limit: 0xFFFF,
            attributes: if index == 0 { 0x9B } else { 0x93 },
            ..SegmentCache::default()
        }),
        a20_enabled: 1,
        ..LegacyCpuState::default()
    }
}

impl Cpu for X86JitCpu {
    fn numeric_error_pending(&self) -> bool {
        self.cpu.numeric_error_pending()
    }

    fn ack_external_numeric_error(&mut self) -> Result<(), CpuError> {
        self.cpu
            .ack_external_numeric_error()
            .map_err(|e| CpuError::UnexpectedExit {
                what: format!("numeric error acknowledgement: {e:?}"),
            })
    }

    fn set_a20_enabled(&mut self, enabled: bool) {
        self.cpu.set_a20_enabled(enabled);
    }

    fn a20_enabled(&self) -> bool {
        self.cpu.a20_enabled()
    }

    fn reset(&mut self) -> Result<(), CpuError> {
        self.cpu
            .reset_execution_state(ResetOptions::default().with_x87_init(true));
        let a20_enabled = self.cpu.a20_enabled();
        if let Some(model @ (CpuModel::I80386 | CpuModel::I80486Dx | CpuModel::I80486DxCpuid)) =
            self.vm.cpu_model()
        {
            let fpu = self.cpu.cpu.fpu_model;
            let route = self.cpu.cpu.numeric_error_route;
            let error = self.cpu.cpu.external_numeric_error;
            self.cpu.cpu = if model == CpuModel::I80386 {
                CpuState::i386_reset(fpu)
            } else {
                CpuState::i486_reset(model)
            };
            self.cpu.cpu.numeric_error_route = route;
            self.cpu.cpu.external_numeric_error = error;
            // The reset image enables A20, but a CPU reset retains the board line.
            self.cpu.set_a20_enabled(a20_enabled);
            // RAM survives reset; discard proofs derived from the old CPU image.
            self.cpu.invalidate_paging();
            return Ok(());
        }
        let state = &mut self.cpu.cpu;
        if self.vm.execution_profile() == ExecutionProfile::Machine {
            // Reset must discard protected caches as well as pending handoffs.
            // Preserve GPRs, whose hardware reset contents are unspecified.
            state.legacy = reset_legacy_image();
            state.legacy.a20_enabled = u8::from(a20_enabled);
            state.legacy.cr0 = 0xFFF0;
            if !self.low_reset_fetch {
                state.legacy.segments[0].base = 0xFF0000;
                state.legacy.reset_fetch = 1;
            }
            state.fs = 0;
            state.gs = 0;
        }
        state.cs = 0xF000;
        state.rip = 0xFFF0;
        state.ds = 0;
        state.es = 0;
        state.ss = 0;
        state.flags.set_flags16(0x0002);
        state.rflags_system = 0;
        self.cpu.invalidate_paging();
        Ok(())
    }

    fn run(&mut self, budget: Option<u64>) -> Result<CpuStop, CpuError> {
        let exit = self.cpu.run(&self.vm, budget);
        self.map_exit(exit)
    }

    fn run_steps(
        &mut self,
        max_steps: u64,
        options: StepRunOptions,
    ) -> Result<CpuStepRun, CpuError> {
        let machine = self.vm.execution_profile() == ExecutionProfile::Machine;
        let mut steps = 0;
        // Bounded returns in a row with no instruction retired between them.
        let mut stalls = 0u32;
        let mut stalled_at = 0;
        let stop = loop {
            let run = self
                .cpu
                .run_steps(
                    &self.vm,
                    max_steps - steps,
                    StepOptions::default().with_stop_when_irq_ready(options.stop_when_irq_ready),
                )
                .map_err(|error| match error {
                    StepRunError::UnsupportedMode => CpuError::UnexpectedExit {
                        what: "step-bounded execution is unsupported by this backend or mode"
                            .to_string(),
                    },
                    StepRunError::PendingPortIn => CpuError::UnexpectedExit {
                        what: "pending port input must be completed before step-bounded execution"
                            .to_string(),
                    },
                    other => CpuError::UnexpectedExit {
                        what: format!("{other:?}"),
                    },
                })?;
            steps += run.steps();
            self.completed_steps += run.steps();
            let stop = run.into_stop();
            // In the machine profile a budget stop short of the budget is
            // only the core's bound on deliveries that retired nothing
            // (`Vcpu::run_steps`), resumable at the handler. Running on lets
            // a stack that overruns reach its shutdown, as on hardware;
            // after STALL_RETURNS bounded returns a 16-bit stack has either
            // done so or wraps cleanly forever, and the run stops.
            if machine && steps < max_steps && matches!(stop, StepStop::Exit(Exit::BudgetExhausted))
            {
                let retired = self.cpu.retired_instructions();
                stalls = if stalls != 0 && retired == stalled_at {
                    stalls + 1
                } else {
                    1
                };
                stalled_at = retired;
                if stalls < STALL_RETURNS {
                    continue;
                }
                return Err(CpuError::DeliveryStalled {
                    deliveries: stalls * DELIVERY_STALL_LIMIT,
                    cs: self.cpu.cpu.cs,
                    ip: self.cpu.cpu.rip,
                });
            }
            break stop;
        };
        let stop = match stop {
            StepStop::Exit(exit) => CpuStepStop::Exit(self.map_exit(exit)?),
            StepStop::IrqReady => CpuStepStop::IrqReady,
            other => {
                return Err(CpuError::UnexpectedExit {
                    what: format!("{other:?}"),
                })
            }
        };
        Ok(CpuStepRun { steps, stop })
    }

    fn completed_steps(&self) -> u64 {
        self.completed_steps
    }

    fn irq_ready(&self) -> bool {
        self.cpu.irq_ready()
    }

    fn reg(&self, reg: CpuReg) -> u64 {
        self.cpu.reg(to_core_reg(reg))
    }

    fn set_reg(&mut self, reg: CpuReg, value: u64) {
        self.cpu.set_reg(to_core_reg(reg), value);
        // A host register restore is a paging barrier even for equal values.
        self.cpu.invalidate_paging();
    }

    fn complete_port_in(&mut self, value: u64) {
        self.cpu.complete_port_in(value);
    }

    fn complete_mmio_read(&mut self, value: u64) {
        self.cpu.complete_mmio_read(value);
    }

    fn complete_mmio_write(&mut self) {
        self.cpu.complete_mmio_write();
    }

    /// The core's delivery log (`x86jit-core` spec.md §17.10, Delivery
    /// log), which only its machine profiles keep, with the registers of
    /// the interrupted code in every record.
    fn enable_delivery_log(&mut self, capacity: usize) -> Result<(), crate::DeliveryLogError> {
        self.cpu
            .enable_delivery_log_with(
                capacity,
                x86jit_core::DeliveryLogOptions::default().registers(true),
            )
            .map_err(|error| crate::DeliveryLogError(error.to_string()))
    }

    fn disable_delivery_log(&mut self) {
        self.cpu.disable_delivery_log();
    }

    fn drain_deliveries(
        &mut self,
        records: &mut Vec<crate::Delivery>,
    ) -> Option<crate::DeliveryCounts> {
        let mut drained = std::mem::take(&mut self.delivery_records);
        drained.clear();
        let counts = self
            .cpu
            .delivery_log_mut()
            .map(|log| log.drain_into(&mut drained));
        records.extend(drained.iter().map(|record| self.delivery(record)));
        self.delivery_records = drained;
        let counts = counts?;
        Some(crate::DeliveryCounts {
            dropped: counts.dropped,
            total: counts.total,
        })
    }

    fn peek_deliveries(&self, records: &mut Vec<crate::Delivery>) -> Option<crate::DeliveryCounts> {
        let log = self.cpu.delivery_log()?;
        records.extend(log.iter().map(|record| self.delivery(record)));
        Some(crate::DeliveryCounts {
            dropped: log.dropped(),
            total: log.total(),
        })
    }

    fn inject_irq(&mut self, vector: u8) {
        // The core queues vectors in every mode, but only Real16 delivers them
        // through the IVT once IF, completion and STI/SS-shadow gates allow it.
        self.cpu.inject_irq(vector);
    }

    fn interrupts_enabled(&self) -> bool {
        self.cpu.flags().if_
    }

    fn has_pending_irq(&self) -> bool {
        self.cpu.has_pending_irq()
    }

    fn guest_dma_read(&self, addr: u32) -> DmaAccess<u8> {
        if self.dma_region_kind(addr) == Some(RegionKind::Trap) {
            return DmaAccess::Mmio;
        }
        let mut buf = [0xFF; 1];
        let _ = self.vm.read_bytes(u64::from(addr), &mut buf);
        DmaAccess::Complete(buf[0])
    }

    fn guest_dma_write(&mut self, addr: u32, value: u8) -> DmaAccess<()> {
        match self.dma_region_kind(addr) {
            Some(RegionKind::Trap) => DmaAccess::Mmio,
            Some(RegionKind::Ram) => {
                // Vm records note_write, invalidating code overwritten by DMA.
                let _ = self.vm.write_bytes(u64::from(addr), &[value]);
                DmaAccess::Complete(())
            }
            _ => DmaAccess::Complete(()),
        }
    }

    fn retired(&self) -> u64 {
        self.cpu.retired_instructions()
    }

    fn code_address(&self) -> (u16, u64) {
        (self.cpu.cpu.cs, self.cpu.cpu.rip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Peek;

    fn paged_guest() -> X86JitCpu {
        use x86jit_core::legacy::SegmentCache;
        let mut cpu = X86JitCpu::new_i386_bare(0, 0x2000, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(0x2000, &[0x90, 0xEB, 0xFD]).unwrap();
        cpu.write_mem(0x4000, &0x5003u32.to_le_bytes()).unwrap();
        cpu.write_mem(0x5008, &0x2003u32.to_le_bytes()).unwrap();
        let state = &mut cpu.cpu.cpu;
        state.cs = 8;
        state.ds = 0x10;
        state.ss = 0x10;
        state.legacy.cr0 = 0x8000_0011;
        state.legacy.cr3 = 0x4000;
        state.legacy.segments = std::array::from_fn(|index| SegmentCache {
            base: 0,
            limit: u32::MAX,
            attributes: if index == 0 { 0xC09B } else { 0xC093 },
            ..SegmentCache::default()
        });
        cpu.run_steps(1, StepRunOptions::default()).unwrap();
        assert_eq!(cpu.reg(CpuReg::Rip), 0x2001);
        assert!(cpu.vm.mem.is_code_page(2));
        cpu
    }

    #[test]
    fn host_page_table_restore_discards_paged_fetch_proofs_even_for_equal_bytes() {
        let mut cpu = paged_guest();
        let mut pte = [0; 4];
        cpu.read_mem(0x5008, &mut pte).unwrap();
        let before = cpu.retired();
        cpu.write_mem(0x5008, &pte).unwrap();
        assert!(
            !cpu.vm.mem.is_code_page(2),
            "host restore must release stale fetch proofs"
        );
        assert_eq!(cpu.retired(), before);
        assert_eq!(cpu.reg(CpuReg::Rip), 0x2001);
        cpu.run_steps(1, StepRunOptions::default()).unwrap();
        assert_eq!(cpu.reg(CpuReg::Rip), 0x2000);
    }

    #[test]
    fn host_register_restore_discards_paged_fetch_proofs_even_for_equal_values() {
        let mut cpu = paged_guest();
        let ip = cpu.reg(CpuReg::Rip);
        cpu.set_reg(CpuReg::Rip, ip);
        assert!(!cpu.vm.mem.is_code_page(2));
        assert_eq!(cpu.reg(CpuReg::Rip), ip);
    }

    #[test]
    fn reset_keeps_page_table_ram_and_discards_paged_fetch_proofs() {
        let mut cpu = paged_guest();
        let mut before = [0; 4];
        cpu.read_mem(0x5008, &mut before).unwrap();
        cpu.reset().unwrap();
        assert!(!cpu.vm.mem.is_code_page(2));
        let mut after = [0; 4];
        cpu.read_mem(0x5008, &mut after).unwrap();
        assert_eq!(after, before);
        assert_eq!(cpu.cpu.cpu.legacy.cr0 & 0x8000_0001, 0);
    }

    #[test]
    fn dma_page_table_write_keeps_fetch_proofs_until_the_next_walk() {
        let mut cpu = paged_guest();
        assert_eq!(cpu.guest_dma_write(0x5008, 0x23), DmaAccess::Complete(()));
        assert!(cpu.vm.mem.is_code_page(2));
        cpu.run_steps(1, StepRunOptions::default()).unwrap();
        assert_eq!(cpu.reg(CpuReg::Rip), 0x2000);
    }

    fn i286_guest(code: &[u8]) -> X86JitCpu {
        let mut cpu =
            X86JitCpu::new_i286_bare(0x100_0000, 0, 0x1000, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(0x1000, code).unwrap();
        cpu
    }

    #[test]
    fn numeric_error_routes_follow_supported_machine_models_and_survive_reset() {
        for (model, present, route) in [
            (CpuModel::I80286, true, NumericErrorRoute::External),
            (CpuModel::I80386, true, NumericErrorRoute::External),
            (CpuModel::I80286, false, NumericErrorRoute::Internal),
            (CpuModel::I80386, false, NumericErrorRoute::Internal),
            (CpuModel::I80486Dx, true, NumericErrorRoute::Internal),
            (CpuModel::I80486DxCpuid, true, NumericErrorRoute::Internal),
        ] {
            let mut cpu = X86JitCpu::new_machine_bare(
                0x10000,
                0,
                0x1000,
                RealSegments::flat(0x7000),
                present,
                false,
                model,
            )
            .unwrap();
            assert_eq!(cpu.cpu.numeric_error_route(), route);
            cpu.reset().unwrap();
            assert_eq!(cpu.cpu.numeric_error_route(), route);
        }
    }

    #[test]
    fn external_numeric_assertion_retires_and_wait_acknowledgement_preserves_status() {
        let mut cpu = i286_guest(&[
            0xdb, 0xe3, // FNINIT
            0xd9, 0x2e, 0x00, 0x06, // FLDCW [0600h]
            0xd9, 0xe8, 0xd9, 0xee, // FLD1; FLDZ
            0xde, 0xf9, // FDIVP ST1, ST0
            0x9b, 0xf4, // WAIT; HLT
        ]);
        cpu.write_mem(0x600, &0x037bu16.to_le_bytes()).unwrap();
        let asserted = cpu.run_steps(100, StepRunOptions::default()).unwrap();
        assert_eq!(
            asserted.stop,
            CpuStepStop::Exit(CpuStop::NumericErrorAsserted)
        );
        assert_eq!(asserted.steps, 5);
        assert!(cpu.numeric_error_pending());
        let before = cpu.retired();
        let ip = cpu.reg(CpuReg::Rip);
        let waiting = cpu.run_steps(100, StepRunOptions::default()).unwrap();
        assert_eq!(
            waiting.stop,
            CpuStepStop::Exit(CpuStop::NumericErrorWaiting)
        );
        assert_eq!(waiting.steps, 0);
        assert_eq!(cpu.retired(), before);
        assert_eq!(cpu.reg(CpuReg::Rip), ip);
        cpu.ack_external_numeric_error().unwrap();
        assert!(cpu.numeric_error_pending());
        assert_eq!(cpu.run(None).unwrap(), CpuStop::Halt);
    }

    /// The delivery log: off by default; enabled, a software interrupt and a
    /// #UD come back oldest first with their saved CS:IP, the bytes there
    /// and the registers of the interrupted code (the INT's function number
    /// in AH, SS:SP before the frame); a peek leaves them, a drain takes
    /// them; a twin without the log retires the same instructions to the
    /// same state. An application profile refuses the log.
    #[test]
    fn the_delivery_log_reports_software_interrupts_and_faults_without_changing_the_run() {
        let program = [
            0xB4, 0x09, // 1000 MOV AH,09h
            0xCD, 0x21, // 1002 INT 21h
            0x0F, 0xFF, // 1004 invalid opcode: #UD
        ];
        let run = |log: bool| {
            let mut cpu = i286_guest(&program);
            // INT 21h returns at once; INT 6 halts.
            cpu.write_mem(0x21 * 4, &[0x00, 0x18, 0, 0]).unwrap();
            cpu.write_mem(6 * 4, &[0x00, 0x19, 0, 0]).unwrap();
            cpu.write_mem(0x1800, &[0xCF]).unwrap(); // IRET
            cpu.write_mem(0x1900, &[0xF4]).unwrap(); // HLT
            assert_eq!(cpu.drain_deliveries(&mut Vec::new()), None);
            if log {
                cpu.enable_delivery_log(4).unwrap();
            }
            let stop = cpu.run_steps(100, StepRunOptions::default()).unwrap().stop;
            assert_eq!(stop, CpuStepStop::Exit(CpuStop::Halt));
            cpu
        };
        let mut plain = run(false);
        let mut logged = run(true);
        assert_eq!(plain.retired(), logged.retired());
        assert_eq!(plain.diagnostic_state(), logged.diagnostic_state());
        assert_eq!(plain.peek_deliveries(&mut Vec::new()), None);
        let mut peeked = Vec::new();
        let counts = logged.peek_deliveries(&mut peeked).unwrap();
        assert_eq!((counts.dropped, counts.total), (0, 2));
        let lines: Vec<String> = peeked.iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "2 software INT 21h vector 21 at 0000:1004 linear 001004 error - cpl 0->0 \
                 bytes 0f ff 00 00 00 00 00 00 ax 0900 bx 0000 cx 0000 dx 0000 si 0000 \
                 di 0000 bp 0000 ds 0000 es 0000 ss:sp 0000:7000 flags 0002",
                "3 fault #UD vector 06 at 0000:1004 linear 001004 error - cpl 0->0 \
                 bytes 0f ff 00 00 00 00 00 00 ax 0900 bx 0000 cx 0000 dx 0000 si 0000 \
                 di 0000 bp 0000 ds 0000 es 0000 ss:sp 0000:7000 flags 0002",
            ]
        );
        let mut drained = Vec::new();
        assert_eq!(logged.drain_deliveries(&mut drained), Some(counts));
        assert_eq!(drained, peeked);
        assert!(logged.drain_deliveries(&mut drained).is_some());
        assert_eq!(drained.len(), 2, "a drain takes the records");
        logged.disable_delivery_log();
        assert_eq!(logged.peek_deliveries(&mut Vec::new()), None);
        plain.disable_delivery_log();

        let mut application =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x1000, RealSegments::flat(0)).unwrap();
        assert!(application.enable_delivery_log(4).is_err());
    }

    /// The bytes of a delivery whose saved IP stands near the end of the
    /// segment wrap to the segment's first bytes, as the 80286's IP does,
    /// instead of reading past the segment: a #UD at 0000:FFFEh shows its
    /// opcode, then the IVT at 0000:0000h, not the unmapped 10000h.
    #[test]
    fn delivery_bytes_wrap_at_the_end_of_the_code_segment() {
        let mut cpu = i286_guest(&[0xE9, 0xFB, 0xEF]); // 1000 JMP 0FFFEh
        cpu.write_mem(0xFFFE, &[0x0F, 0xFF]).unwrap(); // #UD
        cpu.write_mem(0, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66])
            .unwrap();
        cpu.write_mem(6 * 4, &[0x00, 0x19, 0, 0]).unwrap();
        cpu.write_mem(0x1900, &[0xF4]).unwrap(); // HLT
        cpu.enable_delivery_log(4).unwrap();
        let stop = cpu.run_steps(100, StepRunOptions::default()).unwrap().stop;
        assert_eq!(stop, CpuStepStop::Exit(CpuStop::Halt));
        let mut records = Vec::new();
        cpu.drain_deliveries(&mut records).unwrap();
        assert_eq!(
            records[0].to_string(),
            "1 fault #UD vector 06 at 0000:fffe linear 00fffe error - cpl 0->0 \
             bytes 0f ff 11 22 33 44 55 66 ax 0000 bx 0000 cx 0000 dx 0000 si 0000 \
             di 0000 bp 0000 ds 0000 es 0000 ss:sp 0000:7000 flags 0002"
        );
        assert_eq!(code_byte_address(0xF8010, 0x10, 3), 0xF8013);
        assert_eq!(code_byte_address(0xFFFFF, 0xFFFF, 1), 0xF0000);
    }

    /// The diagnostic picture of a halted CPU: registers, FLAGS with DF set
    /// and IF clear, the real-mode caches, the HLT before CS:IP, the code
    /// bytes, and a stack in a trapped window that is reported, not read.
    #[test]
    fn a_diagnostic_state_shows_registers_flags_and_memory_without_reading_devices() {
        let mut cpu = i286_guest(&[
            0xB8, 0x34, 0x12, // MOV AX,1234h
            0xBB, 0x00, 0x20, // MOV BX,2000h
            0x8E, 0xD3, // MOV SS,BX
            0xBC, 0xF8, 0x0F, // MOV SP,0FF8h
            0xFD, // STD
            0xFA, // CLI
            0xF4, // HLT
        ]);
        cpu.map_mmio(0x20000, 0x1000).unwrap();
        let run = cpu.run_steps(100, StepRunOptions::default()).unwrap();
        assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Halt));
        let retired = cpu.retired();
        let state = cpu.diagnostic_state();
        assert_eq!(state.register("ax"), Some(0x1234));
        assert_eq!(state.register("bx"), Some(0x2000));
        assert_eq!(state.sp(), 0x0FF8);
        assert!(!state.interrupts_enabled());
        assert_ne!(state.flags & 0x0400, 0, "DF");
        assert_eq!((state.cs().selector, state.cs().base), (0, 0));
        assert_eq!((state.ss().selector, state.ss().base), (0x2000, 0x20000));
        let system = state.system.unwrap();
        assert!(!system.protected());
        assert_eq!(system.cpl, 0);
        // HLT retires: execution continues after it.
        assert_eq!(state.ip, 0x100E);
        assert_eq!(state.hlt_before(), Some((0, 0x100D)));
        assert_eq!(state.code[0], Peek::Value(0));
        // Four words lie in the trapped window, the rest beyond any memory.
        assert!(state.stack[..4].iter().all(|word| *word == Peek::Device));
        assert!(state.stack[4..].iter().all(|word| *word == Peek::Unmapped));
        assert_eq!(state.retired, retired);
        assert_eq!(cpu.retired(), retired, "the picture changes nothing");
        let text = state.to_string();
        for line in [
            "cs:ip    0000:100e (linear 00100e)\n",
            "regs     ax 1234 bx 2000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 sp 0ff8\n",
            "mode     real, cpl 0, msw fff0 (pe 0), a20 on\n",
            "ss       2000 base 020000 limit 0000ffff",
            "hlt      at 0000:100d, the byte before cs:ip\n",
            "ss:sp    2000:0ff8 (linear 020ff8)\n",
            "stack    +00 dvdv dvdv dvdv dvdv ???? ???? ???? ????\n",
            "note     dv: a memory-mapped device, not read; ??: no memory\n",
        ] {
            assert!(text.contains(line), "{line:?} missing from\n{text}");
        }
        assert!(text.contains(": if 0, tf 0, df 1, iopl 0, nt 0;"), "{text}");
    }

    /// Protected-mode entry at 1000h with the GDTR image at 1100h, the IDTR
    /// image at 1106h and the GDT at 1110h: ring-0 code 08h (base 0, limit
    /// FFFFh) and data 10h at 10000h with `data_limit`, so the stack's
    /// frames stay clear of the code and tables. The code then loads SS with
    /// 10h and SP with F000h (at 101Ch), and loads ES with selector F8h,
    /// beyond the GDT limit (at 1021h): #GP(00F8h).
    fn protected_mode_fault(data_limit: u16, idtr: [u8; 6]) -> X86JitCpu {
        let mut cpu = i286_guest(&[
            0xFA, // CLI
            0x0F, 0x01, 0x16, 0x00, 0x11, // LGDT [1100h]
            0x0F, 0x01, 0x1E, 0x06, 0x11, // LIDT [1106h]
            0xB8, 0x01, 0x00, 0x0F, 0x01, 0xF0, // MOV AX,1; LMSW AX
            0xEA, 0x16, 0x10, 0x08, 0x00, // JMP 0008:1016
            0xB8, 0x10, 0x00, 0x8E, 0xD0, // MOV AX,10h; MOV SS,AX
            0xBC, 0x00, 0xF0, // MOV SP,0F000h
            0xB8, 0xF8, 0x00, 0x8E, 0xC0, // MOV AX,0F8h; MOV ES,AX
            0xF4, // HLT
        ]);
        cpu.write_mem(0x1100, &[0x17, 0x00, 0x10, 0x11, 0x00, 0x00])
            .unwrap();
        cpu.write_mem(0x1106, &idtr).unwrap();
        let [low, high] = data_limit.to_le_bytes();
        cpu.write_mem(0x1118, &[0xFF, 0xFF, 0, 0, 0, 0x9B, 0, 0])
            .unwrap();
        cpu.write_mem(0x1120, &[low, high, 0, 0, 0x01, 0x93, 0, 0])
            .unwrap();
        cpu.map_ram(0x10000, 0x10000).unwrap();
        cpu
    }

    /// Point the 14 exception gates of the IDT at 1180h (limit 6Fh) at
    /// 0008:1040h, which holds `handler`.
    fn gates_to(cpu: &mut X86JitCpu, handler: &[u8]) {
        for vector in 0..14 {
            cpu.write_mem(
                0x1180 + 8 * vector,
                &[0x40, 0x10, 0x08, 0x00, 0x00, 0x86, 0, 0],
            )
            .unwrap();
        }
        cpu.write_mem(0x1040, handler).unwrap();
    }

    /// The IDTR image for [`gates_to`]: limit 6Fh, base 1180h.
    const IDTR_1180: [u8; 6] = [0x6F, 0x00, 0x80, 0x11, 0x00, 0x00];

    /// A #GP handler whose first instruction faults again retires nothing
    /// (Intel 80286 Programmer's Reference Manual, §9.5: the handler is
    /// entered, and its MOV ES faults as a new exception, not a double
    /// fault). The core returns every 1024 deliveries with its budget
    /// unspent; before it did, this run never returned. From SP F000h, a
    /// multiple of the 8-byte frame, the frames wrap without a fault, so the
    /// backend stops the run after [`STALL_RETURNS`] such returns with a
    /// typed error naming the handler, and keeps the steps it completed
    /// before the loop.
    #[test]
    fn a_handler_that_faults_at_once_stops_with_a_delivery_stall() {
        let mut cpu = protected_mode_fault(0xFFFF, IDTR_1180);
        gates_to(&mut cpu, &[0x8E, 0xC0, 0xF4]); // MOV ES,AX; HLT
        let error = cpu
            .run_steps(1_000_000, StepRunOptions::default())
            .unwrap_err();
        let CpuError::DeliveryStalled { deliveries, cs, ip } = error else {
            panic!("expected a delivery stall, got {error:?}");
        };
        assert_eq!((deliveries, cs, ip), (11 * 1024, 0x08, 0x1040));
        // CLI to MOV AX,0F8h: the ten instructions before the first fault.
        assert_eq!(cpu.completed_steps(), 10);
        assert_eq!(cpu.retired(), 10);
        assert_eq!(
            error.to_string(),
            "the processor is stuck delivering exceptions: 11264 deliveries without an \
             instruction, the last to 0008:1040"
        );
        let report = cpu.stop_report(&error).expect("a stall has a report");
        assert_eq!(
            (report.cs, report.ip, report.code_linear),
            (0x08, 0x1040, 0x1040)
        );
        assert_eq!(
            &report.code[..3],
            &[Some(0x8E), Some(0xC0), Some(0xF4)],
            "the report shows the handler's first instruction"
        );
        // Each delivery pushed FLAGS, CS, IP and the error code, and the
        // frames wrapped past SP 0 once: F000h - 8 × 11264 = 9000h mod 64 KiB.
        assert_eq!(cpu.reg(CpuReg::Rsp) & 0xFFFF, 0x9000);
    }

    /// A #UD handler that is itself an invalid opcode: each delivery pushes
    /// a 6-byte frame (FLAGS, CS, IP). In real mode, from SP FFFFh, 10,922
    /// frames leave SP at 3, and the next one puts a word at offset FFFFh,
    /// across the top of the segment; in protected mode, from SP FFFEh,
    /// 10,922 frames leave SP at 2 and the next one has no room above the
    /// bottom of the stack. That fault's own frame faults (double fault),
    /// and so does the double fault's: shutdown (Intel 80286 Programmer's
    /// Reference Manual, §9.6.2 and Appendix B, INT, which in real mode
    /// shuts down only at SP 1, 3 or 5). That is within the window the
    /// backend runs through, so one call ends in Shutdown, the stop the
    /// board answers with a reset, and no delivery stall is reported. In
    /// real mode through the vector table and in protected mode through an
    /// interrupt gate.
    #[test]
    fn a_stack_that_overruns_shuts_down_within_the_stall_window() {
        // Real mode: INT 6 at 0000:0018h leads to 0000:3000h; SS:SP is
        // 1000:FFFFh, clear of the code and the vector table. An even SP
        // would wrap inside the segment without a fault.
        let mut real = X86JitCpu::new_i286_bare(
            0x100_0000,
            0,
            0x1000,
            RealSegments {
                ds: 0,
                es: 0,
                ss: 0x1000,
                sp: 0xFFFF,
            },
        )
        .unwrap();
        real.map_ram(0, 0x20000).unwrap();
        real.write_mem(0x1000, &[0x0F, 0xFF]).unwrap();
        real.write_mem(0x18, &[0x00, 0x30, 0x00, 0x00]).unwrap();
        real.write_mem(0x3000, &[0x0F, 0xFF]).unwrap();
        // Protected mode: the faulting MOV ES becomes 0F FF, SP becomes FFFEh.
        let mut protected = protected_mode_fault(0xFFFF, IDTR_1180);
        gates_to(&mut protected, &[0x0F, 0xFF]);
        protected.write_mem(0x101C, &[0xFE, 0xFF]).unwrap();
        protected.write_mem(0x1021, &[0x0F, 0xFF]).unwrap();
        for (label, cpu, retired, handler, sp) in [
            ("real", &mut real, 0, (0, 0x3000), 3),
            ("protected", &mut protected, 10, (0x08, 0x1040), 2),
        ] {
            let run = cpu.run_steps(1_000_000, StepRunOptions::default()).unwrap();
            assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Shutdown), "{label}");
            assert_eq!(cpu.retired(), retired, "{label}");
            assert_eq!(run.steps, retired, "{label}");
            assert_eq!(
                (cpu.code_segment() as u16, cpu.reg(CpuReg::Rip)),
                handler,
                "{label}: the last frame went to the handler"
            );
            // 10,922 frames of 6 bytes from FFFFh leave SP at 3, from FFFEh at 2.
            assert_eq!(cpu.reg(CpuReg::Rsp) & 0xFFFF, sp, "{label}");
        }
    }

    /// After a delivery stall and a reset, a guest whose #UD handler skips
    /// the invalid opcode and returns takes 2,000 deliveries, each followed
    /// by retired instructions, and halts: the reset left no stall count
    /// behind that a healthy guest could trip.
    #[test]
    fn a_reset_after_a_stall_lets_a_healthy_guest_run() {
        let mut cpu = protected_mode_fault(0xFFFF, IDTR_1180);
        gates_to(&mut cpu, &[0x8E, 0xC0, 0xF4]); // MOV ES,AX; HLT
        assert!(matches!(
            cpu.run_steps(1_000_000, StepRunOptions::default()),
            Err(CpuError::DeliveryStalled { .. })
        ));
        cpu.reset().unwrap();
        // The reset vector jumps to 0000:2000h: INT 6 leads to 0000:3000h,
        // SS:SP = 1000:FFFEh, then 2,000 rounds of 0F FF.
        cpu.map_rom(0xFF_FFF0, &[0xEA, 0x00, 0x20, 0x00, 0x00])
            .unwrap();
        cpu.write_mem(0x18, &[0x00, 0x30, 0x00, 0x00]).unwrap();
        cpu.write_mem(
            0x2000,
            &[
                0xB8, 0x00, 0x10, 0x8E, 0xD0, // MOV AX,1000h; MOV SS,AX
                0xBC, 0xFE, 0xFF, // MOV SP,0FFFEh
                0xB9, 0xD0, 0x07, // MOV CX,2000
                0x0F, 0xFF, // #UD
                0xE2, 0xFC, // LOOP 200Bh
                0xF4, // HLT
            ],
        )
        .unwrap();
        cpu.write_mem(
            0x3000,
            &[
                0x5B, 0x83, 0xC3, 0x02, 0x53, // POP BX; ADD BX,2; PUSH BX
                0xCF, // IRET
            ],
        )
        .unwrap();
        let run = cpu.run_steps(1_000_000, StepRunOptions::default()).unwrap();
        assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Halt));
        assert_eq!(cpu.reg(CpuReg::Rcx) & 0xFFFF, 0);
    }

    #[test]
    fn reset_image_matches_the_pinned_real_entry() {
        assert_eq!(
            super::reset_legacy_image(),
            CpuState::legacy_real_entry([0xF000, 0, 0, 0, 0, 0]).legacy
        );
    }

    /// The board's reset after a shutdown writes the complete 80286 reset
    /// image (Intel 80286 Programmer's Reference Manual, §5.3 Table 5-3 and
    /// §10.4): CS:IP F000:FFF0 with the hidden CS base FF0000h, MSW FFF0h,
    /// IDTR base 0 and limit 3FFh, and real-mode caches of limit FFFFh for
    /// every segment register. The guest makes a triple fault with a 256-byte
    /// stack segment and an empty IDT; the code after the reset pushes at
    /// SP 0 and calls INT 20h through the real-mode vector table, which the
    /// old SS cache or IDTR would fault back into shutdown.
    #[test]
    fn reset_after_a_triple_fault_writes_the_complete_reset_image() {
        // An empty IDT: the #GP gate lies beyond the limit (#GP, then #DF),
        // and so does the #DF gate (shutdown, §9.6.2).
        let mut cpu = protected_mode_fault(0x00FF, [0; 6]);
        let run = cpu.run_steps(1000, StepRunOptions::default()).unwrap();
        assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Shutdown));
        let legacy = &cpu.cpu.cpu.legacy;
        assert_eq!(legacy.cr0 & 1, 1, "the shutdown happened in protected mode");
        assert_eq!(legacy.segments[3].limit, 0xFF, "SS holds the small segment");
        cpu.reset().unwrap();
        let state = &cpu.cpu.cpu;
        let legacy = &state.legacy;
        assert_eq!((state.cs, state.rip), (0xF000, 0xFFF0));
        assert_eq!((state.ds, state.es, state.ss), (0, 0, 0));
        assert_eq!(legacy.cr0 & 0xFFFF, 0xFFF0);
        assert_eq!((legacy.idtr.base, legacy.idtr.limit), (0, 0x3FF));
        assert_eq!(legacy.cpl, 0);
        for (index, cache) in legacy.segments.iter().take(4).enumerate() {
            assert_eq!(cache.limit, 0xFFFF, "segment cache {index}");
            let base = if index == 0 { 0xFF_0000 } else { 0 };
            assert_eq!(cache.base, base, "segment cache {index}");
        }
        // The reset vector jumps to 0000:2000h, which pushes at SP 0 (the
        // word at SS:FFFEh) and calls INT 20h, whose vector at 80h leads to
        // a HLT at 0000:3000h.
        cpu.map_rom(0xFF_FFF0, &[0xEA, 0x00, 0x20, 0x00, 0x00])
            .unwrap();
        cpu.write_mem(0x2000, &[0xBC, 0x00, 0x00, 0x50, 0xCD, 0x20, 0xF4])
            .unwrap();
        cpu.write_mem(0x80, &[0x00, 0x30, 0x00, 0x00]).unwrap();
        cpu.write_mem(0x3000, &[0xF4]).unwrap();
        let run = cpu.run_steps(100, StepRunOptions::default()).unwrap();
        assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Halt));
        assert_eq!(
            (cpu.code_segment(), cpu.reg(CpuReg::Rip)),
            (0, 0x3001),
            "INT 20h went through the real-mode vector table"
        );
        assert_eq!(cpu.reg(CpuReg::Rsp) & 0xFFFF, 0xFFF8);
    }

    #[test]
    fn a20_fault_metadata_reports_gated_physical_address() {
        use x86jit_core::fault::Knowledge;
        for enabled in [false, true] {
            let mut cpu =
                X86JitCpu::new_i286_bare(0x200000, 0, 0x1000, RealSegments::flat(0)).unwrap();
            cpu.map_ram(0x1000, 0x1000).unwrap();
            cpu.write_mem(0x1000, &[0xB8, 0xFF, 0xFF, 0x8E, 0xD8, 0xA0, 0x10, 0])
                .unwrap();
            cpu.set_a20_enabled(enabled);
            let exit = cpu.cpu.run(&cpu.vm, Some(100));
            let Exit::UnmappedMemory {
                addr,
                metadata: Some(metadata),
                ..
            } = exit
            else {
                panic!("expected a physical mapping fault, got {exit:?}");
            };
            assert_eq!(addr, if enabled { 0x100000 } else { 0 });
            assert_eq!(metadata.physical_address(), Knowledge::Known(addr));
        }
    }

    #[test]
    fn a20_reset_preserves_pending_line_when_abandoning_mmio() {
        let mut cpu = X86JitCpu::new_i286_bare(0x200000, 0, 0x1000, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x2000).unwrap();
        cpu.map_mmio(0x100000, 0x1000).unwrap();
        cpu.write_mem(0x1000, &[0xB8, 0xFF, 0xFF, 0x8E, 0xD8, 0xA0, 0x10, 0])
            .unwrap();
        assert_eq!(
            cpu.run(Some(100)).unwrap(),
            CpuStop::MmioRead {
                addr: 0x100000,
                size: 1
            }
        );
        cpu.set_a20_enabled(false);
        assert!(cpu.a20_enabled());
        assert_eq!(cpu.cpu.pending_a20_enabled(), Some(false));
        cpu.reset().unwrap();
        assert!(!cpu.a20_enabled());
        assert_eq!(cpu.cpu.pending_a20_enabled(), None);
        assert_eq!(cpu.guest_dma_write(0x10, 0x77), DmaAccess::Complete(()));
        let mut low = [0];
        cpu.read_mem(0x10, &mut low).unwrap();
        assert_eq!(low, [0x77]);
        assert_eq!(cpu.guest_dma_read(0x100000), DmaAccess::Mmio);
    }

    #[test]
    fn i286_single_step_probe_traps_after_pop_ds_and_restores_the_stack() {
        // Intel 80286 Programmer's Reference Manual, §9.7.1 and Appendix B
        // MOV/POP: only SS loads inhibit the next single-step trap.
        let mut cpu = i286_guest(&[]);
        cpu.set_reg(CpuReg::Rip, 0xA7D0);
        cpu.write_mem(4, &[0x35, 0xA8, 0, 0]).unwrap();
        cpu.write_mem(
            0xA7D0,
            &[
                0x9C, 0x9C, 0x58, 0x0D, 0, 1, 0x50, 0x31, 0xC0, 0x9D, 0x90, 0x1E, 0x1F, 0x90, 0x9D,
                0xF4,
            ],
        )
        .unwrap();
        cpu.write_mem(
            0xA835,
            &[
                0x55, 0x89, 0xE5, 0x81, 0x7E, 2, 0xDD, 0xA7, 0x75, 4, 0xB0, 1, 0xEB, 9, 0x81, 0x7E,
                2, 0xDC, 0xA7, 0x75, 2, 0xB0, 2, 0x5D, 0xCF,
            ],
        )
        .unwrap();
        let mut returns = Vec::new();
        let mut halted = false;
        for _ in 0..100 {
            if cpu.reg(CpuReg::Rip) == 0xA835 {
                let mut frame = [0; 6];
                cpu.read_mem(cpu.reg(CpuReg::Rsp), &mut frame).unwrap();
                returns.push(u16::from_le_bytes([frame[0], frame[1]]));
                assert_eq!(cpu.cpu.cpu.rflags_system & 0x100, 0);
                assert!(!cpu.interrupts_enabled());
            }
            let run = cpu.run_steps(1, StepRunOptions::default()).unwrap();
            if run.stop == CpuStepStop::Exit(CpuStop::Halt) {
                halted = true;
                break;
            }
            assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::BudgetExhausted));
        }
        assert!(halted);
        assert_eq!(returns, [0xA7DB, 0xA7DC, 0xA7DD, 0xA7DE, 0xA7DF]);
        assert_eq!(cpu.reg(CpuReg::Rax), 1);
        assert_eq!(cpu.reg(CpuReg::Rsp), u64::from(RealSegments::DEFAULT_SP));
        assert_eq!(cpu.completed_steps(), 54);
        assert_eq!(cpu.retired(), 54);
    }

    #[test]
    fn i286_optional_fpu_policy_survives_entry_and_reset() {
        for (present, expected) in [(false, FpuModel::None), (true, FpuModel::Present)] {
            let mut cpu = X86JitCpu::new_i286_bare_with_fpu(
                0x100_0000,
                0,
                0x1000,
                RealSegments::flat(0),
                present,
            )
            .unwrap();
            assert_eq!(cpu.vm.fpu_model(), expected);
            assert_eq!(cpu.cpu.fpu_model(), expected);
            assert!(!cpu.cpu.cpu.fpu_emulation);
            cpu.reset().unwrap();
            assert_eq!(cpu.vm.fpu_model(), expected);
            assert_eq!(cpu.cpu.fpu_model(), expected);
            assert!(!cpu.cpu.cpu.fpu_emulation);
        }
    }

    #[test]
    fn i286_real_aad_x87_and_rep_ins_resume_at_element_boundaries() {
        let mut cpu = i286_guest(&[
            0xB8, 1, 1, 0xD5, 0x10, 0xA3, 0, 5, // AAD with a nondecimal base
            0xDB, 0xE3, 0xD9, 0x3E, 2, 5, // FNINIT; FNSTCW [0502h]
            0xBA, 0x60, 0, 0xBF, 4, 5, 0xB9, 2, 0, 0xF3, 0x6C, 0xF4,
        ]);
        assert_eq!(cpu.vm.cpu_model(), Some(CpuModel::I80286));
        assert_eq!(
            cpu.vm.exception_delivery(),
            x86jit_core::ExceptionDelivery::InGuest
        );
        assert_eq!(cpu.vm.fpu_model(), FpuModel::Present);
        let options = StepRunOptions::default();
        let input = CpuStepStop::Exit(CpuStop::PortIn {
            port: 0x60,
            size: 1,
        });
        assert_eq!(cpu.run_steps(9, options).unwrap().stop, input);
        assert_eq!(cpu.retired(), 8);
        cpu.complete_port_in(0xA7);
        assert_eq!(cpu.run_steps(1, options).unwrap().stop, input);
        assert_eq!(cpu.retired(), 9);
        cpu.complete_port_in(0x5C);
        assert_eq!(
            cpu.run_steps(1, options).unwrap().stop,
            CpuStepStop::Exit(CpuStop::Halt)
        );
        let mut markers = [0; 6];
        cpu.read_mem(0x500, &mut markers).unwrap();
        assert_eq!(markers, [0x11, 0, 0x7F, 3, 0xA7, 0x5C]);
        assert_eq!(cpu.completed_steps(), 11);
        assert_eq!(cpu.retired(), 10);
    }

    /// A protected-mode guest at CPL 0 with a GDT at 800h (ring-0 code 08h
    /// and data 10h, a 286 TSS 18h at 600h, a ring-3 call gate 20h to
    /// 0008:1100h, ring-3 code 28h and data 30h) that loads TR, then runs
    /// `tail`. The TSS's ring stacks are 0010:6000, 0030:5000 and 0030:4000.
    fn protected_guest(tail: &[u8]) -> X86JitCpu {
        let mut code = vec![
            0x0F, 0x01, 0x16, 0x00, 0x0F, // LGDT [0F00h]
            0xB8, 1, 0, 0x0F, 0x01, 0xF0, // MOV AX,1; LMSW AX
            0xEA, 0x10, 0x10, 0x08, 0x00, // JMP 0008:1010
            0xB8, 0x10, 0x00, 0x8E, 0xD0, 0x8E, 0xD8, // MOV AX,10h; MOV SS,AX; MOV DS,AX
            0xB8, 0x18, 0x00, 0x0F, 0x00, 0xD8, // MOV AX,18h; LTR AX
        ];
        code.extend_from_slice(tail);
        let mut cpu = i286_guest(&code);
        cpu.write_mem(0x0F00, &[0x37, 0x00, 0x00, 0x08, 0x00, 0x00])
            .unwrap();
        let gdt: [[u8; 8]; 6] = [
            [0xFF, 0xFF, 0, 0, 0, 0x9A, 0, 0],
            [0xFF, 0xFF, 0, 0, 0, 0x92, 0, 0],
            [0x2B, 0, 0x00, 0x06, 0, 0x81, 0, 0],
            [0x00, 0x11, 0x08, 0, 0x02, 0xE4, 0, 0],
            [0xFF, 0xFF, 0, 0, 0, 0xFA, 0, 0],
            [0xFF, 0xFF, 0, 0, 0, 0xF2, 0, 0],
        ];
        for (index, descriptor) in gdt.iter().enumerate() {
            cpu.write_mem(0x808 + 8 * index as u64, descriptor).unwrap();
        }
        cpu.write_mem(
            0x600,
            &[
                0, 0, 0x00, 0x60, 0x10, 0, 0x00, 0x50, 0x30, 0, 0x00, 0x40, 0x30, 0,
            ],
        )
        .unwrap();
        cpu
    }

    /// RETF to an outer ring runs: the guest reaches ring 3 on the stack the
    /// return frame names. There a far CALL through the call gate, with 31
    /// parameter words and both the ring-3 stack and the TSS's ring-0 stack in
    /// a device window, is refused: copying that frame would need more
    /// device-transfer records than the core keeps for one instruction. The
    /// report shows CPL 3, the ring-3 stack, the gate, the code segment behind
    /// it and the ring stacks of the TSS.
    #[test]
    fn an_outward_return_reaches_ring_3_where_a_gate_call_is_refused() {
        use crate::stop_report::{Descriptor, DescriptorKind, TransferForm};
        let mut cpu = protected_guest(&[
            0x68, 0x33, 0x00, // PUSH 33h (ring-3 SS)
            0x68, 0x00, 0x04, // PUSH 0400h (ring-3 SP)
            0x68, 0x2B, 0x00, // PUSH 2Bh (ring-3 CS)
            0x68, 0x00, 0x20, // PUSH 2000h (ring-3 IP)
            0xCB, // RETF
        ]);
        // CALL FAR 0023:0000, the call gate with RPL 3.
        cpu.write_mem(0x2000, &[0x9A, 0, 0, 0x23, 0]).unwrap();
        // The gate copies 31 words. Ring-3 data 30h and a new ring-0 data
        // segment 38h start at 10000h, a device window; SS0:SP0 is 0038:0F00.
        cpu.map_mmio(0x1_0000, 0x1000).unwrap();
        let gate = [0x00, 0x11, 0x08, 0, 0x1F, 0xE4, 0, 0];
        cpu.write_mem(0x820, &gate).unwrap();
        cpu.write_mem(0x830, &[0xFF, 0xFF, 0, 0, 0x01, 0xF2, 0, 0])
            .unwrap();
        cpu.write_mem(0x838, &[0xFF, 0xFF, 0, 0, 0x01, 0x92, 0, 0])
            .unwrap();
        cpu.write_mem(0x0F00, &[0x3F, 0x00]).unwrap();
        cpu.write_mem(0x602, &[0x00, 0x0F, 0x38, 0x00]).unwrap();
        let error = cpu.run_steps(20, StepRunOptions::default()).unwrap_err();
        assert!(
            matches!(&error, CpuError::UnsupportedSystem { operation, cs: 0x2B, ip: 0x2000 }
                if operation == "LegacyInstruction"),
            "{error}"
        );
        let report = cpu.stop_report(&error).expect("a refusal has a report");
        let system = report.system.expect("the machine profile");
        assert!(system.protected());
        assert_eq!(
            (system.cs.selector, system.cpl, system.a20),
            (0x2B, 3, true)
        );
        assert_eq!((system.gdtr.base, system.gdtr.limit), (0x800, 0x3F));
        assert_eq!((system.tr.selector, system.tr.base), (0x18, 0x600));
        assert_eq!((report.ss, report.sp), (0x33, 0x0400));
        assert_eq!(report.code_linear, 0x2000);
        assert_eq!(&report.code[..5], &[0x9A, 0, 0, 0x23, 0].map(Some));
        assert_eq!(
            report.form,
            Some(TransferForm::FarDirect {
                call: true,
                selector: 0x23,
                offset: 0
            })
        );
        let named: Vec<_> = report
            .descriptors
            .iter()
            .map(|named| (named.role, named.selector, named.entry.map(|d| d.access())))
            .collect();
        assert_eq!(
            named,
            [("target", 0x23, Ok(0xE4)), ("gate target", 0x08, Ok(0x9B))]
        );
        assert_eq!(report.descriptors[0].entry, Ok(Descriptor(gate)));
        assert_eq!(
            report.descriptors[0].entry.unwrap().kind(),
            DescriptorKind::Gate {
                name: "286 call gate",
                selector: 0x08,
                offset: Some(0x1100),
                words: Some(31)
            }
        );
        let tss = report.tss.expect("TR holds a 286 TSS");
        assert_eq!(tss.stacks, [(0x38, 0x0F00), (0x30, 0x5000), (0x30, 0x4000)]);
        assert!(report.to_string().starts_with(
            "cpu stop report\nstop     unsupported cpu system operation LegacyInstruction at 002b:2000\n"
        ));
    }

    /// The report forms physical addresses as the model does: the 80286's
    /// 24 address lines drop bit 24 of a linear address.
    #[test]
    fn the_report_reads_through_the_models_address_width() {
        let mut cpu = i286_guest(&[0xF4]);
        cpu.cpu.cpu.legacy.segments[0].base = 0x0100_0000;
        let fault = CpuError::Fault {
            vector: 13,
            error_code: Some(0),
            cs: 0,
            ip: 0x1000,
        };
        let report = cpu.stop_report(&fault).unwrap();
        assert_eq!(report.code_linear, 0x0100_1000);
        assert_eq!(report.code[0], Some(0xF4));
    }

    /// Other errors carry no instruction, so they have no report.
    #[test]
    fn only_errors_at_an_instruction_have_a_report() {
        let cpu = i286_guest(&[0xF4]);
        for error in [
            CpuError::Unmapped { addr: 0x9000 },
            CpuError::Memory("refused".into()),
            CpuError::UnexpectedExit {
                what: "interrupted".into(),
            },
        ] {
            assert!(cpu.stop_report(&error).is_none(), "{error}");
        }
        let fault = CpuError::Fault {
            vector: 13,
            error_code: Some(0),
            cs: 0,
            ip: 0x1000,
        };
        let report = cpu.stop_report(&fault).unwrap();
        assert_eq!(report.code[0], Some(0xF4));
        assert!(!report.system.unwrap().protected());
        assert_eq!(report.tss, Err("real mode"));
    }

    #[test]
    fn i286_bad_protected_selector_returns_typed_fault() {
        let mut vm = Vm::new(VmConfig::flat(0x10000));
        vm.set_cpu_model(CpuModel::I80286);
        vm.set_exception_delivery(x86jit_core::ExceptionDelivery::Outward);
        let core = vm.new_vcpu();
        let mut cpu = X86JitCpu {
            vm,
            cpu: core,
            dma_regions: Vec::new(),
            completed_steps: 0,
            low_reset_fetch: true,
            delivery_records: Vec::new(),
        };
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.set_reg(CpuReg::Rip, 0x1000);
        cpu.write_mem(
            0x1000,
            &[
                0xB8, 1, 0, 0x0F, 0x01, 0xF0, // MOV AX,1; LMSW AX
                0xEA, 0, 0x12, 8, 0, // JMP 0008:1200, beyond the empty GDT
            ],
        )
        .unwrap();
        let error = cpu.run_steps(20, StepRunOptions::default()).unwrap_err();
        assert!(
            matches!(
                error,
                CpuError::Fault {
                    vector: 13,
                    error_code: Some(8),
                    cs: 0,
                    ip: 0x1006
                }
            ),
            "{error}"
        );
        assert_eq!(cpu.retired(), 2);
        assert_eq!(cpu.completed_steps(), 2);
        assert_eq!(cpu.reg(CpuReg::Rip), 0x1006);
    }

    #[test]
    fn i286_real_word_at_ffff_delivers_gp_through_ivt_for_ds_and_ss() {
        // Intel 80286 and 80287 Programmer's Reference Manual, §9.6.2:
        // a word crossing a real-mode segment limit reports vector 13 even for SS.
        for code in [&[0xA1, 0xFF, 0xFF][..], &[0x36, 0xA1, 0xFF, 0xFF][..]] {
            let mut cpu = i286_guest(code);
            cpu.write_mem(13 * 4, &[0, 0x18, 0, 0]).unwrap();
            cpu.write_mem(0x1800, &[0xF4]).unwrap();
            let run = cpu.run_steps(1, StepRunOptions::default()).unwrap();
            assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Halt));
            assert_eq!(cpu.reg(CpuReg::Rip), 0x1801);
            assert_eq!(cpu.reg(CpuReg::Rax), 0);
            assert_eq!(
                cpu.reg(CpuReg::Rsp),
                u64::from(RealSegments::DEFAULT_SP) - 6
            );
            let mut frame = [0; 4];
            cpu.read_mem(cpu.reg(CpuReg::Rsp), &mut frame).unwrap();
            assert_eq!(frame, [0, 0x10, 0, 0]);
            assert_eq!(cpu.retired(), 1);
        }
    }

    #[test]
    fn i286_reset_discards_protected_caches_and_fetches_high_rom() {
        let mut cpu = i286_guest(&[0xB8, 1, 0, 0x0F, 0x01, 0xF0]);
        cpu.run_steps(2, StepRunOptions::default()).unwrap();
        assert_eq!(cpu.cpu.cpu.legacy.cr0 & 1, 1);
        cpu.cpu.cpu.legacy.segments[1].base = 0x123000;
        cpu.inject_irq(0x20);
        cpu.map_rom(0xFFFFF0, &[0xEA, 0, 0x15, 0, 0]).unwrap();
        cpu.write_mem(0x1500, &[0x0F, 0x01, 0xE0, 0xF4]).unwrap(); // SMSW AX; HLT
        let retired = cpu.retired();
        cpu.reset().unwrap();
        assert_eq!(cpu.retired(), retired);
        assert_eq!(cpu.cpu.cpu.legacy.cr0, 0xFFF0);
        assert_eq!(cpu.cpu.cpu.legacy.segments[0].base, 0xFF0000);
        assert_eq!(cpu.cpu.cpu.legacy.segments[1].base, 0);
        assert!(!cpu.has_pending_irq());
        assert_eq!(
            cpu.run_steps(3, StepRunOptions::default()).unwrap().stop,
            CpuStepStop::Exit(CpuStop::Halt)
        );
        assert_eq!(cpu.reg(CpuReg::Rax), 0xFFF0);
        assert_eq!(cpu.cpu.cpu.legacy.reset_fetch, 0);
        assert_eq!(cpu.retired(), retired + 3);
    }

    #[test]
    fn filled_rom_keeps_chunk_boundaries_inside_one_immutable_region() {
        let mut cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x1000, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x2000).unwrap();
        cpu.map_rom_filled(0x2000, 0x2003, 0xBA).unwrap();
        cpu.write_mem(
            0x1000,
            &[0xC7, 0x06, 0xFF, 0x2F, 0x34, 0x12, 0xA1, 0xFF, 0x2F, 0xF4],
        )
        .unwrap();
        assert_eq!(cpu.run(Some(100)).unwrap(), CpuStop::Halt);
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFFFF, 0xBABA);
        assert_eq!(cpu.guest_dma_write(0x4002, 0), DmaAccess::Complete(()));
        let mut value = [0];
        cpu.read_mem(0x4002, &mut value).unwrap();
        assert_eq!(value, [0xBA]);
    }

    #[test]
    fn unmapped_window_remaps_as_ram_rom_and_trap_for_fetch_and_dma() {
        // CALL FAR D000:0000; HLT.
        let mut cpu = i286_guest(&[0x9A, 0, 0, 0, 0xD0, 0xF4]);
        cpu.map_mmio(0xD0000, 0x4000).unwrap();
        assert!(matches!(
            cpu.unmap(0xD0000, 0x2000),
            Err(CpuError::Memory(_))
        ));
        assert_eq!(cpu.guest_dma_read(0xD0000), DmaAccess::Mmio);

        cpu.unmap(0xD0000, 0x4000).unwrap();
        cpu.map_ram(0xD0000, 0x4000).unwrap();
        // MOV AX, 1234h; RETF.
        cpu.write_mem(0xD0000, &[0xB8, 0x34, 0x12, 0xCB]).unwrap();
        let run = cpu.run_steps(10, StepRunOptions::default()).unwrap();
        assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Halt));
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFFFF, 0x1234);
        assert_eq!(cpu.guest_dma_write(0xD0004, 0x56), DmaAccess::Complete(()));
        let mut value = [0];
        cpu.read_mem(0xD0004, &mut value).unwrap();
        assert_eq!(value, [0x56]);

        // Re-tagged as ROM, the window keeps its bytes and drops DMA stores.
        cpu.unmap(0xD0000, 0x4000).unwrap();
        cpu.map_rom_in_place(0xD0000, 0x4000).unwrap();
        assert_eq!(cpu.guest_dma_write(0xD0004, 0x77), DmaAccess::Complete(()));
        assert_eq!(cpu.guest_dma_read(0xD0004), DmaAccess::Complete(0x56));
        cpu.set_reg(CpuReg::Rip, 0x1000);
        let run = cpu.run_steps(10, StepRunOptions::default()).unwrap();
        assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Halt));

        cpu.unmap(0xD0000, 0x4000).unwrap();
        cpu.map_mmio(0xD0000, 0x4000).unwrap();
        assert_eq!(cpu.guest_dma_read(0xD0000), DmaAccess::Mmio);
        assert_eq!(cpu.guest_dma_write(0xD0000, 0), DmaAccess::Mmio);
        cpu.set_reg(CpuReg::Rip, 0x1000);
        assert!(matches!(
            cpu.run_steps(10, StepRunOptions::default()),
            Err(CpuError::Unmapped { addr: 0xD0000 })
        ));
    }

    #[test]
    fn reset_discards_pending_ins_and_mid_rep_without_rewinding_accounting() {
        for answered in [true, false] {
            let mut cpu =
                X86JitCpu::new_real16_bare(0x10_0000, 0, 0x7000, RealSegments::flat(0)).unwrap();
            cpu.map_ram(0, 0x10_0000).unwrap();
            cpu.write_mem(0x7000, &[0xF3, 0x6C, 0xF4]).unwrap();
            cpu.write_mem(0xFFFF0, &[0xF4]).unwrap();
            cpu.set_reg(CpuReg::Rcx, 3);
            cpu.set_reg(CpuReg::Rdi, 0x500);
            cpu.set_reg(CpuReg::Rdx, 0x60);
            let run = cpu.run_steps(1, StepRunOptions::default()).unwrap();
            assert_eq!(
                run.stop,
                CpuStepStop::Exit(CpuStop::PortIn {
                    port: 0x60,
                    size: 1
                })
            );
            if answered {
                cpu.complete_port_in(0xA7);
            }
            cpu.inject_irq(0x20);
            cpu.cpu.cpu.fpu_cw = 0;
            cpu.cpu.cpu.fpu_empty = 0;
            let retired = cpu.retired();
            let steps = cpu.completed_steps();
            let gpr = cpu.cpu.cpu.gpr;
            cpu.reset().unwrap();
            assert_eq!(cpu.cpu.cpu.gpr, gpr);
            assert_eq!(cpu.cpu.cpu.flags.to_flags16(), 2);
            assert_eq!((cpu.cpu.cpu.cs, cpu.cpu.cpu.rip), (0xF000, 0xFFF0));
            assert_eq!((cpu.cpu.cpu.ds, cpu.cpu.cpu.es, cpu.cpu.cpu.ss), (0, 0, 0));
            assert_eq!(cpu.retired(), retired);
            assert_eq!(cpu.completed_steps(), steps);
            let run = cpu.run_steps(1, StepRunOptions::default()).unwrap();
            let mut marker = [0xFF; 3];
            cpu.read_mem(0x500, &mut marker).unwrap();
            assert_eq!(marker, [0; 3], "no deferred INS store may survive reset");
            assert_eq!(run.stop, CpuStepStop::Exit(CpuStop::Halt));
            assert_eq!(cpu.reg(CpuReg::Rcx), gpr[1]);
            assert_eq!(cpu.reg(CpuReg::Rdi), gpr[7]);
            assert_eq!(cpu.retired(), retired + 1);
            assert_eq!(cpu.completed_steps(), steps + 1);
            assert!(!cpu.has_pending_irq());
            assert_eq!(cpu.cpu.cpu.fpu_cw, 0x037F);
            assert_eq!(cpu.cpu.cpu.fpu_empty, 0xFF);
        }
    }

    #[test]
    fn answered_ins_store_fault_retries_without_another_port_read() {
        let mut cpu = X86JitCpu::new_real16_bare(
            0x30000,
            0,
            0x7C00,
            RealSegments {
                es: 0x2000,
                ..RealSegments::flat(0)
            },
        )
        .unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        // INSW; HLT. ES points to an unmapped destination until the retry.
        cpu.write_mem(0x7C00, &[0x6D, 0xF4]).unwrap();
        cpu.set_reg(CpuReg::Rdx, 0x300);
        let options = StepRunOptions::default();
        assert_eq!(
            cpu.run_steps(1, options).unwrap(),
            CpuStepRun {
                steps: 1,
                stop: CpuStepStop::Exit(CpuStop::PortIn {
                    port: 0x300,
                    size: 2
                }),
            }
        );
        cpu.complete_port_in(0xABCD);
        assert_eq!(cpu.reg(CpuReg::Rip), 0x7C00);
        assert_eq!(cpu.reg(CpuReg::Rdi), 0);
        assert!(matches!(
            cpu.run_steps(1, options),
            Err(CpuError::Unmapped { addr: 0x20000 })
        ));
        assert_eq!(cpu.completed_steps(), 1);
        cpu.map_ram(0x20000, 0x10000).unwrap();
        assert_eq!(
            cpu.run_steps(1, options).unwrap(),
            CpuStepRun {
                steps: 1,
                stop: CpuStepStop::Exit(CpuStop::Halt),
            }
        );
        let mut word = [0; 2];
        cpu.read_mem(0x20000, &mut word).unwrap();
        assert_eq!(word, [0xCD, 0xAB]);
        assert_eq!(cpu.reg(CpuReg::Rdi), 2);
        assert_eq!(cpu.reg(CpuReg::Rip), 0x7C02);
        assert_eq!(cpu.completed_steps(), 2);
        assert_eq!(cpu.retired(), 2);
    }

    #[test]
    fn readiness_option_stops_after_sti_protected_rep_iteration() {
        let mut cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        // MOV CX, 3; STI; REP STOSB; HLT.
        cpu.write_mem(0x7C00, &[0xB9, 3, 0, 0xFB, 0xF3, 0xAA, 0xF4])
            .unwrap();
        cpu.set_reg(CpuReg::Rdi, 0x600);
        let options = StepRunOptions {
            stop_when_irq_ready: true,
        };
        assert_eq!(
            cpu.run_steps(64, options).unwrap(),
            CpuStepRun {
                steps: 3,
                stop: CpuStepStop::IrqReady,
            }
        );
        assert_eq!(cpu.retired(), 2);
        assert_eq!(cpu.reg(CpuReg::Rcx), 2);
        assert_eq!(cpu.reg(CpuReg::Rip), 0x7C04);
        assert_eq!(
            cpu.run_steps(0, options).unwrap(),
            CpuStepRun {
                steps: 0,
                stop: CpuStepStop::Exit(CpuStop::BudgetExhausted),
            }
        );
        assert_eq!(
            cpu.run_steps(64, options).unwrap(),
            CpuStepRun {
                steps: 0,
                stop: CpuStepStop::IrqReady,
            }
        );
        // Delivery precedes the ready stop and adds no step to the handler's HLT.
        cpu.write_mem(0x20, &[0, 0x80, 0, 0]).unwrap();
        cpu.write_mem(0x8000, &[0xF4]).unwrap();
        cpu.inject_irq(8);
        assert_eq!(
            cpu.run_steps(64, options).unwrap(),
            CpuStepRun {
                steps: 1,
                stop: CpuStepStop::Exit(CpuStop::Halt),
            }
        );
        assert!(!cpu.has_pending_irq());
        assert_eq!(cpu.completed_steps(), 4);
        assert_eq!(cpu.reg(CpuReg::Rcx), 2);
    }

    #[test]
    fn readiness_option_waits_for_port_and_mmio_completion() {
        let options = StepRunOptions {
            stop_when_irq_ready: true,
        };
        // The IN or MMIO load is the instruction protected by STI.
        let mut input = real_mode_cpu(&[0xFB, 0xE4, 0x80, 0xF4]);
        let run = input.run_steps(64, options).unwrap();
        assert_eq!(run.steps, 2);
        assert!(matches!(
            run.stop,
            CpuStepStop::Exit(CpuStop::PortIn { .. })
        ));
        assert!(!input.irq_ready());
        input.complete_port_in(0x42);
        assert_eq!(
            input.run_steps(64, options).unwrap(),
            CpuStepRun {
                steps: 0,
                stop: CpuStepStop::IrqReady,
            }
        );
        assert_eq!(input.reg(CpuReg::Rax), 0x42);

        let mut mmio = real_mode_cpu(&[0xFB, 0xA0, 0, 0, 0xF4]);
        let run = mmio.run_steps(64, options).unwrap();
        assert_eq!(run.steps, 1);
        assert!(matches!(
            run.stop,
            CpuStepStop::Exit(CpuStop::MmioRead { .. })
        ));
        assert!(!mmio.irq_ready());
        mmio.complete_mmio_read(0x43);
        assert_eq!(
            mmio.run_steps(64, options).unwrap(),
            CpuStepRun {
                steps: 1,
                stop: CpuStepStop::IrqReady,
            }
        );
        assert_eq!(mmio.reg(CpuReg::Rax), 0x43);
    }

    /// A stop on an instruction the core does not lift names the code segment
    /// and the instruction bytes, so a field report identifies the opcode.
    #[test]
    fn unsupported_instruction_reports_code_segment_and_bytes() {
        let mut cpu =
            X86JitCpu::new_real16_bare(0x20000, 0x0700, 0x072B, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        // MOV EAX, CR0: a 386 encoding this pin does not lift in Real16, as
        // the next test also relies on; pick another encoding if a future
        // core lifts it.
        cpu.write_mem(0x772B, &[0x0F, 0x20, 0xC0]).unwrap();
        let error = cpu.run(None).unwrap_err();
        match &error {
            CpuError::UnsupportedInstruction { addr, cs, bytes } => {
                assert_eq!((*addr, *cs), (0x072B, 0x0700));
                assert_eq!(bytes, &[0x0F, 0x20, 0xC0]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            error.to_string(),
            "unsupported instruction at 0700:072b (bytes 0f 20 c0)"
        );
    }

    /// A real-mode refusal in the application profile: the report keeps the
    /// bytes at CS:IP and marks the ones in a trapped window, which it never
    /// reads.
    #[test]
    fn a_real_mode_unsupported_instruction_reports_its_bytes_without_reading_mmio() {
        let mut cpu =
            X86JitCpu::new_real16_bare(0x20000, 0x0600, 0x0FFD, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x7000).unwrap();
        cpu.map_mmio(0x7000, 0x1000).unwrap();
        // MOV EAX, CR0, as in the test above, ending at the trapped window.
        cpu.write_mem(0x6FFD, &[0x0F, 0x20, 0xC0]).unwrap();
        let error = cpu.run(None).unwrap_err();
        let report = cpu.stop_report(&error).unwrap();
        assert_eq!(
            (report.cs, report.ip, report.code_linear),
            (0x600, 0xFFD, 0x6FFD)
        );
        assert_eq!(report.system, None);
        assert_eq!(&report.code[..3], &[Some(0x0F), Some(0x20), Some(0xC0)]);
        assert!(report.code[3..].iter().all(Option::is_none));
        let text = report.to_string();
        assert!(text.contains("code     0f 20 c0 ?? ??"), "{text}");
        assert!(text.contains("mode     application profile"), "{text}");
    }

    /// Games swap bytes with video memory (`XCHG ES:[DI], AL`), and a device
    /// register must see exactly one read and one write for an exchange or a
    /// read-modify-write, never a second read after the write.
    #[test]
    fn exchange_and_read_modify_write_on_mmio_read_once_and_write_once() {
        // XCHG ES:[DI], AL; HLT
        let mut cpu = real_mode_cpu(&[0x26, 0x86, 0x05, 0xF4]);
        cpu.set_reg(CpuReg::Rdi, 0x10);
        cpu.set_reg(CpuReg::Rax, 0x5A);
        let addr = VIDEO_BASE + 0x10;
        assert_eq!(cpu.run(None).unwrap(), CpuStop::MmioRead { addr, size: 1 });
        cpu.complete_mmio_read(0x33);
        assert_eq!(
            cpu.run(None).unwrap(),
            CpuStop::MmioWrite {
                addr,
                size: 1,
                value: 0x5A,
            }
        );
        cpu.complete_mmio_write();
        assert_eq!(cpu.run(None).unwrap(), CpuStop::Halt);
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFF, 0x33);

        // ADD WORD ES:[DI], 1; HLT
        let mut cpu = real_mode_cpu(&[0x26, 0x83, 0x05, 0x01, 0xF4]);
        cpu.set_reg(CpuReg::Rdi, 0x20);
        let addr = VIDEO_BASE + 0x20;
        assert_eq!(cpu.run(None).unwrap(), CpuStop::MmioRead { addr, size: 2 });
        cpu.complete_mmio_read(0x1234);
        assert_eq!(
            cpu.run(None).unwrap(),
            CpuStop::MmioWrite {
                addr,
                size: 2,
                value: 0x1235,
            }
        );
        cpu.complete_mmio_write();
        assert_eq!(cpu.run(None).unwrap(), CpuStop::Halt);
    }

    #[test]
    fn completed_steps_survives_error_exits_and_zero_budget_calls() {
        for unsupported in [false, true] {
            let mut cpu =
                X86JitCpu::new_real16_bare(0x10000, 0, 0x7C00, RealSegments::flat(0)).unwrap();
            cpu.map_ram(0, 0x8000).unwrap();
            // MOV CX, 3; REP STOSB; NOP, then a faulting unit with no charge.
            let mut code = vec![0xB9, 3, 0, 0xF3, 0xAA, 0x90];
            code.extend_from_slice(if unsupported {
                // MOV EAX, CR0 does not exist on an 80286 and this pin does not
                // lift it in Real16. If a future core lifts control-register
                // moves, the assertion below must fail loudly; choose another
                // unsupported encoding then.
                &[0x0F, 0x20, 0xC0]
            } else {
                &[0xA0, 0, 0x90]
            });
            cpu.write_mem(0x7C00, &code).unwrap();
            cpu.set_reg(CpuReg::Rdi, 0x600);
            assert_eq!(cpu.completed_steps(), 0);
            assert_eq!(
                cpu.run_steps(2, StepRunOptions::default()).unwrap().steps,
                2
            );
            assert_eq!(cpu.completed_steps(), 2);
            assert_eq!(cpu.retired(), 1);
            let error = cpu.run_steps(64, StepRunOptions::default()).unwrap_err();
            if unsupported {
                assert!(matches!(error, CpuError::UnsupportedInstruction { .. }));
            } else {
                assert!(matches!(error, CpuError::Unmapped { addr: 0x9000 }));
            }
            assert_eq!(cpu.completed_steps(), 5);
            assert_eq!(cpu.retired(), 3);
            assert_eq!(
                cpu.run_steps(0, StepRunOptions::default()).unwrap().steps,
                0
            );
            assert_eq!(cpu.completed_steps(), 5);
        }
    }

    #[test]
    fn run_steps_cuts_inside_cached_real16_block() {
        let mut code = vec![0x40; 100]; // inc ax
        code.extend_from_slice(&[0xE6, 0x80, 0xF4]);
        let mut cpu = real_mode_cpu(&code);
        assert!(matches!(
            run_to_stop(&mut cpu).unwrap(),
            CpuStop::PortOut { .. }
        ));
        cpu.set_reg(CpuReg::Rip, 0x7C00);
        cpu.set_reg(CpuReg::Rax, 0);
        let before = cpu.retired();
        for n in [0, 7, 1, 9] {
            let ip = cpu.reg(CpuReg::Rip);
            let ax = cpu.reg(CpuReg::Rax);
            let run = cpu.run_steps(n, StepRunOptions::default()).unwrap();
            assert_eq!(
                run,
                crate::CpuStepRun {
                    steps: n,
                    stop: crate::CpuStepStop::Exit(CpuStop::BudgetExhausted),
                }
            );
            assert_eq!(cpu.reg(CpuReg::Rip), ip + n);
            assert_eq!(cpu.reg(CpuReg::Rax), ax + n);
        }
        assert_eq!(cpu.retired() - before, 17);
    }

    #[test]
    fn run_steps_attributes_out_to_last_step_and_mmio_to_next_step() {
        let mut out = real_mode_cpu(&[0x90, 0xE6, 0x80, 0xF4]);
        let run = out.run_steps(2, StepRunOptions::default()).unwrap();
        assert_eq!(run.steps, 2);
        assert_eq!(
            run.stop,
            crate::CpuStepStop::Exit(CpuStop::PortOut {
                port: 0x80,
                size: 1,
                value: 0,
            })
        );
        assert_eq!(out.reg(CpuReg::Rip), 0x7C03);
        assert_eq!(out.retired(), 2);

        for (opcode, stop) in [
            (
                0xA0,
                CpuStop::MmioRead {
                    addr: VIDEO_BASE,
                    size: 1,
                },
            ),
            (
                0xA2,
                CpuStop::MmioWrite {
                    addr: VIDEO_BASE,
                    size: 1,
                    value: 0,
                },
            ),
        ] {
            let mut cpu = real_mode_cpu(&[0x90, opcode, 0, 0, 0xF4]);
            let run = cpu.run_steps(2, StepRunOptions::default()).unwrap();
            assert_eq!(run.steps, 1);
            assert_eq!(run.stop, crate::CpuStepStop::Exit(stop));
            assert_eq!(cpu.reg(CpuReg::Rip), 0x7C01);
            assert_eq!(cpu.retired(), 1);
            match stop {
                CpuStop::MmioRead { .. } => cpu.complete_mmio_read(0x42),
                CpuStop::MmioWrite { .. } => cpu.complete_mmio_write(),
                _ => unreachable!(),
            }
            let replay = cpu.run_steps(1, StepRunOptions::default()).unwrap();
            assert_eq!(replay.steps, 1);
            assert_eq!(
                replay.stop,
                crate::CpuStepStop::Exit(CpuStop::BudgetExhausted)
            );
            assert_eq!(cpu.reg(CpuReg::Rip), 0x7C04);
        }
    }

    #[test]
    fn irq_ready_waits_for_instruction_after_mov_ss() {
        let mut cpu = real_mode_cpu(&[0x8E, 0xD0, 0x90, 0xF4]); // mov ss, ax; nop; hlt
        let mut flags = cpu.cpu.flags();
        flags.if_ = true;
        cpu.cpu.set_flags(flags);
        assert!(cpu.irq_ready());
        assert_eq!(
            cpu.run_steps(1, StepRunOptions::default()).unwrap().steps,
            1
        );
        assert!(cpu.interrupts_enabled());
        assert!(!cpu.irq_ready());
        cpu.run_steps(0, StepRunOptions::default()).unwrap();
        assert!(!cpu.irq_ready());
        assert_eq!(
            cpu.run_steps(1, StepRunOptions::default()).unwrap().steps,
            1
        );
        assert!(cpu.irq_ready());
        assert!(!cpu.has_pending_irq());
    }

    #[test]
    fn run_steps_rejects_long64_without_guest_changes() {
        let mut cpu = X86JitCpu::new(0x1000, 0, &[0x90, 0xF4]).unwrap();
        for n in [0, 1] {
            assert!(matches!(
                cpu.run_steps(n, StepRunOptions::default()),
                Err(CpuError::UnexpectedExit { what }) if what == "step-bounded execution is unsupported by this backend or mode"
            ));
            assert_eq!(cpu.reg(CpuReg::Rip), 0);
            assert_eq!(cpu.retired(), 0);
            assert!(!cpu.irq_ready());
        }
    }

    #[test]
    fn run_steps_requires_port_in_completion() {
        let mut cpu = real_mode_cpu(&[0xE4, 0x80, 0xF4]);
        let mut flags = cpu.cpu.flags();
        flags.if_ = true;
        cpu.cpu.set_flags(flags);
        let run = cpu.run_steps(1, StepRunOptions::default()).unwrap();
        assert_eq!(run.steps, 1);
        assert_eq!(
            run.stop,
            crate::CpuStepStop::Exit(CpuStop::PortIn {
                port: 0x80,
                size: 1
            })
        );
        assert!(!cpu.irq_ready());
        assert_eq!(
            cpu.run_steps(0, StepRunOptions::default()).unwrap().steps,
            0
        );
        assert!(
            matches!(cpu.run_steps(1, StepRunOptions::default()), Err(CpuError::UnexpectedExit { what }) if what == "pending port input must be completed before step-bounded execution")
        );
        assert_eq!(cpu.reg(CpuReg::Rip), 0x7C02);
        assert_eq!(cpu.retired(), 1);
        cpu.complete_port_in(0x42);
        assert!(cpu.irq_ready());
        assert_eq!(cpu.retired(), 1);
        let run = cpu.run_steps(1, StepRunOptions::default()).unwrap();
        assert_eq!(run.steps, 1);
        assert_eq!(run.stop, crate::CpuStepStop::Exit(CpuStop::Halt));
        assert_eq!(cpu.reg(CpuReg::Rax), 0x42);
    }

    /// A masked BIOS ROM is fetch/read-executable but immutable to the guest: code
    /// runs straight out of a `map_rom` region, a read from it returns the ROM
    /// bytes, and a guest store aimed back into it is silently dropped rather than
    /// faulting. The program stores a word into its own ROM, reads the same
    /// location back into AX, then halts — if the store landed, AX would hold the
    /// stored value; because it is dropped, AX holds the original ROM bytes.
    #[test]
    fn rom_executes_and_reads_but_a_store_into_it_is_dropped() {
        // DS = CS = 0xF000, so DS:0x0000 resolves to phys 0xF0000 — the ROM's own
        // first bytes, which are this program's opcodes.
        //   C7 06 00 00 34 12   mov word [0x0000], 0x1234   (store into ROM)
        //   A1 00 00            mov ax, [0x0000]            (read it back)
        //   F4                  hlt
        #[rustfmt::skip]
        let code = [
            0xC7, 0x06, 0x00, 0x00, 0x34, 0x12,
            0xA1, 0x00, 0x00,
            0xF4,
        ];
        let mut cpu = X86JitCpu::new_real16_bare(
            0x10_0000,
            0xF000,
            0x0000,
            RealSegments {
                ds: 0xF000,
                es: 0xF000,
                ss: 0x0000,
                sp: 0x0F00,
            },
        )
        .expect("bare real-mode cpu");
        cpu.map_rom(0xF_0000, &code).expect("map rom");

        // Drive to the halt (a small budget per quantum; the program is a handful
        // of instructions, so this converges immediately).
        loop {
            match cpu.run(Some(64)).expect("run") {
                CpuStop::Halt => break,
                CpuStop::BudgetExhausted => continue,
                other => panic!("unexpected stop before hlt: {other:?}"),
            }
        }

        // The store was dropped: AX read back the original ROM bytes at 0xF0000,
        // which are the opcode `C7 06` → little-endian word 0x06C7, not 0x1234.
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFFFF, 0x06C7);

        // The backing is likewise untouched when read directly through the ROM.
        let mut buf = [0u8; 2];
        cpu.read_mem(0xF_0000, &mut buf).expect("read rom");
        assert_eq!(u16::from_le_bytes(buf), 0x06C7);
    }

    #[test]
    fn dma_rom_write_is_dropped_and_reads_return_rom_bytes() {
        let mut cpu = real_mode_cpu(&[0xF4]);
        cpu.map_rom(0xF_0000, &[0xA5, 0x5A]).expect("map rom");

        assert_eq!(cpu.guest_dma_write(0xF_0000, 0x11), DmaAccess::Complete(()));
        assert_eq!(cpu.guest_dma_write(0xF_0001, 0x22), DmaAccess::Complete(()));

        let mut bytes = [0; 2];
        cpu.read_mem(0xF_0000, &mut bytes).expect("read rom");
        assert_eq!(bytes, [0xA5, 0x5A]);
        assert_eq!(cpu.guest_dma_read(0xF_0000), DmaAccess::Complete(0xA5));
        assert_eq!(cpu.guest_dma_read(0xF_0001), DmaAccess::Complete(0x5A));
    }

    #[test]
    fn dma_ram_write_invalidates_cached_code() {
        // mov ax, 0x1234; out 0xE9, al; hlt
        let mut cpu = real_mode_cpu(&[0xB8, 0x34, 0x12, 0xE6, 0xE9, 0xF4]);
        assert!(matches!(
            run_to_stop(&mut cpu).expect("run"),
            CpuStop::PortOut { .. }
        ));
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFFFF, 0x1234);
        cpu.vm.watch_range(0x7C01, 1);

        assert_eq!(cpu.guest_dma_write(0x7C01, 0x78), DmaAccess::Complete(()));
        assert_eq!(cpu.guest_dma_read(0x7C01), DmaAccess::Complete(0x78));
        assert_eq!(cpu.vm.take_dirty_ranges(), vec![(0x7000, 0x1000)]);
        cpu.set_reg(CpuReg::Rip, 0x7C00);

        assert!(matches!(
            run_to_stop(&mut cpu).expect("rerun"),
            CpuStop::PortOut { .. }
        ));
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFFFF, 0x1278);
    }

    /// Base of the trapped colour text window the MMIO tests aim at.
    const VIDEO_BASE: u64 = 0xB_8000;

    /// A real-mode CPU that runs `code` from `0000:7C00` over conventional RAM,
    /// with a trapped window at [`VIDEO_BASE`] and DS/ES pointing into it, so a
    /// near `[disp16]` operand lands at `VIDEO_BASE + disp16`.
    fn real_mode_cpu(code: &[u8]) -> X86JitCpu {
        let mut cpu = X86JitCpu::new_real16_bare(
            0x10_0000,
            0x0000,
            0x7C00,
            RealSegments {
                ds: 0xB800,
                es: 0xB800,
                ss: 0x0000,
                sp: RealSegments::DEFAULT_SP,
            },
        )
        .expect("bare real-mode cpu");
        cpu.map_ram(0, 0xA_0000).expect("map ram");
        cpu.map_mmio(VIDEO_BASE, 0x8000).expect("map mmio");
        cpu.write_mem(0x7C00, code).expect("load code");
        cpu
    }

    /// Run until the first stop that is not a budget yield.
    fn run_to_stop(cpu: &mut X86JitCpu) -> Result<CpuStop, CpuError> {
        loop {
            match cpu.run(Some(64)) {
                Ok(CpuStop::BudgetExhausted) => continue,
                other => return other,
            }
        }
    }

    /// A real-mode word store into a trapped window reaches the machine as one
    /// `MmioWrite` with the guest's linear address, width and value. Once the
    /// machine acknowledges it, the retried store does not trap again and the
    /// guest runs on to its `hlt`.
    #[test]
    fn real_mode_store_to_trapped_window_surfaces_as_one_mmio_write() {
        //   B8 48 1F      mov ax, 0x1F48
        //   89 06 02 00   mov [0x0002], ax    ; DS:0x0002 = 0xB8002
        //   F4            hlt
        let mut cpu = real_mode_cpu(&[0xB8, 0x48, 0x1F, 0x89, 0x06, 0x02, 0x00, 0xF4]);

        assert_eq!(
            run_to_stop(&mut cpu).expect("run"),
            CpuStop::MmioWrite {
                addr: VIDEO_BASE + 2,
                size: 2,
                value: 0x1F48,
            }
        );
        cpu.complete_mmio_write();
        assert_eq!(run_to_stop(&mut cpu).expect("run"), CpuStop::Halt);
    }

    /// A real-mode word load from a trapped window reaches the machine as one
    /// `MmioRead`, and the value the machine supplies is what the guest receives.
    #[test]
    fn real_mode_load_from_trapped_window_surfaces_as_one_mmio_read() {
        //   A1 04 00      mov ax, [0x0004]    ; DS:0x0004 = 0xB8004
        //   F4            hlt
        let mut cpu = real_mode_cpu(&[0xA1, 0x04, 0x00, 0xF4]);

        assert_eq!(
            run_to_stop(&mut cpu).expect("run"),
            CpuStop::MmioRead {
                addr: VIDEO_BASE + 4,
                size: 2,
            }
        );
        cpu.complete_mmio_read(0xBEEF);
        assert_eq!(run_to_stop(&mut cpu).expect("run"), CpuStop::Halt);
        assert_eq!(cpu.reg(CpuReg::Rax) & 0xFFFF, 0xBEEF);
    }

    #[test]
    fn real_mode_mmio_read_and_write_resume_from_nonzero_code_segment() {
        let mut cpu = X86JitCpu::new_real16_bare(
            0x10_0000,
            0x07C0,
            0,
            RealSegments {
                ds: 0xB800,
                es: 0xB800,
                ..RealSegments::flat(0)
            },
        )
        .unwrap();
        cpu.map_ram(0, 0xA_0000).unwrap();
        cpu.map_mmio(VIDEO_BASE, 0x8000).unwrap();
        // mov ax, [4]; inc bx; mov [2], ax; inc cx; hlt
        cpu.write_mem(0x7C00, &[0xA1, 4, 0, 0x43, 0x89, 0x06, 2, 0, 0x41, 0xF4])
            .unwrap();

        assert_eq!(
            run_to_stop(&mut cpu).unwrap(),
            CpuStop::MmioRead {
                addr: VIDEO_BASE + 4,
                size: 2,
            }
        );
        assert_eq!(cpu.reg(CpuReg::Rip), 0);
        cpu.complete_mmio_read(0xBEEF);
        assert_eq!(
            cpu.run_steps(1, StepRunOptions::default()).unwrap(),
            CpuStepRun {
                steps: 1,
                stop: CpuStepStop::Exit(CpuStop::BudgetExhausted),
            }
        );
        assert_eq!(cpu.reg(CpuReg::Rip), 3);
        assert_eq!(cpu.reg(CpuReg::Rax), 0xBEEF);
        assert_eq!(cpu.reg(CpuReg::Rbx), 0);

        assert_eq!(
            run_to_stop(&mut cpu).unwrap(),
            CpuStop::MmioWrite {
                addr: VIDEO_BASE + 2,
                size: 2,
                value: 0xBEEF,
            }
        );
        assert_eq!(cpu.reg(CpuReg::Rip), 4);
        assert_eq!(cpu.reg(CpuReg::Rbx), 1);
        cpu.complete_mmio_write();
        assert_eq!(
            cpu.run_steps(1, StepRunOptions::default()).unwrap(),
            CpuStepRun {
                steps: 1,
                stop: CpuStepStop::Exit(CpuStop::BudgetExhausted),
            }
        );
        assert_eq!(cpu.reg(CpuReg::Rip), 8);
        assert_eq!(cpu.reg(CpuReg::Rcx), 0);
        assert_eq!(run_to_stop(&mut cpu).unwrap(), CpuStop::Halt);
        assert_eq!(cpu.reg(CpuReg::Rip), 10);
        assert_eq!(cpu.reg(CpuReg::Rcx), 1);
        assert_eq!(cpu.code_segment(), 0x07C0);
        assert_eq!(cpu.retired(), 5);
    }

    #[test]
    fn long64_mmio_moves_complete_each_part() {
        const MMIO: u64 = 0x4000;
        let value = 0x1122334455667788_99AABBCCDDEEFF00u128;
        for store in [false, true] {
            // Intel SDM Vol. 2B, MOVDQU, Operation (128-bit load/store).
            // Copy to RAM after the trapped move to observe the completed operand.
            let code = [
                0xF3,
                0x0F,
                if store { 0x7F } else { 0x6F },
                0x00,
                0xF3,
                0x0F,
                0x7F,
                0x03,
                0xF4,
            ];
            let vm = Vm::new(VmConfig::flat(0x10000));
            let vcpu = vm.new_vcpu();
            let mut cpu = X86JitCpu {
                vm,
                cpu: vcpu,
                dma_regions: Vec::new(),
                completed_steps: 0,
                low_reset_fetch: true,
                delivery_records: Vec::new(),
            };
            cpu.map_ram(0, MMIO).unwrap();
            cpu.map_mmio(MMIO, 0x1000).unwrap();
            cpu.write_mem(0x1000, &code).unwrap();
            cpu.set_reg(CpuReg::Rip, 0x1000);
            cpu.set_reg(CpuReg::Rax, MMIO);
            cpu.set_reg(CpuReg::Rbx, 0x2000);
            cpu.cpu.set_xmm(0, if store { value } else { u128::MAX });
            let mut written = [0u8; 16];
            for part in 0..2u8 {
                // Inspect the core metadata before the adapter discards it.
                let exit = cpu.cpu.run(&cpu.vm, None);
                let (addr, size, total_size, actual_part) = match &exit {
                    Exit::MmioRead {
                        addr,
                        size,
                        total_size,
                        part,
                        ..
                    } if !store => (*addr, *size, *total_size, *part),
                    Exit::MmioWrite {
                        addr,
                        size,
                        total_size,
                        part,
                        ..
                    } if store => (*addr, *size, *total_size, *part),
                    other => panic!("unexpected exit: {other:?}"),
                };
                assert_eq!(
                    (addr, size, total_size, actual_part),
                    (MMIO + u64::from(part) * 8, 8, 16, part)
                );
                assert_eq!(cpu.reg(CpuReg::Rip), 0x1000);
                let chunk = (value >> (u32::from(part) * 64)) as u64;
                if store {
                    let stop = cpu.map_exit(exit).unwrap();
                    let CpuStop::MmioWrite {
                        addr: write_addr,
                        size: write_size,
                        value: write_value,
                    } = stop
                    else {
                        panic!("unexpected stop: {stop:?}");
                    };
                    assert_eq!((write_addr, write_size, write_value), (addr, size, chunk));
                    let offset = usize::from(part) * 8;
                    written[offset..offset + 8].copy_from_slice(&write_value.to_le_bytes());
                    cpu.complete_mmio_write();
                } else {
                    assert_eq!(
                        cpu.map_exit(exit).unwrap(),
                        CpuStop::MmioRead { addr, size }
                    );
                    cpu.complete_mmio_read(chunk);
                }
            }
            assert_eq!(cpu.run(None).unwrap(), CpuStop::Halt);
            assert_eq!(cpu.cpu.xmm(0), value);
            let mut memory = [0; 16];
            cpu.read_mem(0x2000, &mut memory).unwrap();
            assert_eq!(memory, value.to_le_bytes());
            if store {
                assert_eq!(written, value.to_le_bytes());
            }
        }
    }

    #[test]
    fn syscall_error_reports_instruction_address_instead_of_resume_rip() {
        for opcode in [[0x0F, 0x05], [0xCD, 0x80]] {
            let mut cpu =
                X86JitCpu::new(0x2000, 0x1000, &[0x90, opcode[0], opcode[1], 0xF4]).unwrap();
            let error = cpu.run(None).unwrap_err();
            assert!(
                matches!(error, CpuError::UnsupportedInstruction { addr: 0x1001, .. }),
                "{opcode:02x?}: {error:?}"
            );
            assert_eq!(cpu.cpu.syscall_evidence().map(|e| e.rip()), Some(0x1001));
            assert_eq!(cpu.reg(CpuReg::Rip), 0x1003);
            assert_eq!(cpu.run(None).unwrap(), CpuStop::Halt);
        }
    }

    /// A host yield request is not something the guest did, so it must not be
    /// reported as a missing opcode. Guest state stays resumable: the next run
    /// carries on to the `hlt`.
    #[test]
    fn host_yield_request_is_an_unexpected_exit() {
        let mut cpu = real_mode_cpu(&[0xF4]);
        cpu.cpu.exit_request().request_exit();

        match cpu.run(None) {
            Err(CpuError::UnexpectedExit { what }) => {
                assert!(what.contains("interrupted"), "{what}")
            }
            other => panic!("expected an unexpected-exit error, got {other:?}"),
        }
        assert_eq!(run_to_stop(&mut cpu).expect("run"), CpuStop::Halt);
    }

    /// A backend whose JIT code storage has run out. It still hands blocks back
    /// for interpretation, which is what a real backend falls back to.
    struct ExhaustedCodeSpace;

    impl x86jit_core::Backend for ExhaustedCodeSpace {
        fn code_space_exhausted(&self) -> bool {
            true
        }

        fn materialize(
            &self,
            ir: &x86jit_core::IrBlock,
            _consistency: x86jit_core::MemConsistency,
            _mmio: Option<(u64, u64)>,
            _guest_base: u64,
        ) -> x86jit_core::CachedBlock {
            x86jit_core::CachedBlock::Interpreted(std::sync::Arc::new(ir.clone()))
        }
    }

    /// Exhausted JIT code space is reported once, as an unexpected exit that
    /// names it rather than as a missing opcode; execution then continues.
    #[test]
    fn exhausted_code_space_is_an_unexpected_exit() {
        let mut vm = Vm::with_backend(VmConfig::flat(0x2000), Box::new(ExhaustedCodeSpace));
        vm.map(0, 0x2000, Prot::RWX, RegionKind::Ram)
            .expect("map ram");
        vm.write_bytes(0x1000, &[0xF4]).expect("load hlt");
        let mut vcpu = vm.new_vcpu();
        vcpu.set_reg(Reg::Rip, 0x1000);
        let mut cpu = X86JitCpu {
            completed_steps: 0,
            low_reset_fetch: true,
            delivery_records: Vec::new(),
            vm,
            cpu: vcpu,
            dma_regions: vec![(0..0x2000, RegionKind::Ram)],
        };

        match cpu.run(None) {
            Err(CpuError::UnexpectedExit { what }) => {
                assert!(what.contains("code space exhausted at 0x1000"), "{what}")
            }
            other => panic!("expected an unexpected-exit error, got {other:?}"),
        }
        assert_eq!(cpu.run(None).expect("run"), CpuStop::Halt);
    }
}

#[cfg(test)]
mod i386_tests {
    use super::*;

    fn v86_guest(model: CpuModel) -> X86JitCpu {
        let mut cpu = X86JitCpu::new_machine_bare(
            1 << 32,
            0x1000,
            0,
            RealSegments::flat(0x8000),
            model != CpuModel::I80386,
            false,
            model,
        )
        .unwrap();
        cpu.map_ram(0, 0x100000).unwrap();
        let state = &mut cpu.cpu.cpu;
        state.legacy.cr0 |= 1;
        state.legacy.cpl = 3;
        state.rflags_system |= 1 << 17;
        state.legacy.segments =
            [state.cs, state.ds, state.es, state.ss, state.fs, state.gs].map(|selector| {
                x86jit_core::legacy::SegmentCache {
                    base: u32::from(selector) << 4,
                    limit: 0xFFFF,
                    attributes: 0x93,
                    ..Default::default()
                }
            });
        state.legacy.segments[0].attributes = 0x9B;
        cpu.write_mem(0x10000, &[0x90, 0xCF]).unwrap();
        cpu
    }

    #[test]
    fn v86_register_restores_preserve_canonical_caches() {
        for model in [
            CpuModel::I80386,
            CpuModel::I80486Dx,
            CpuModel::I80486DxCpuid,
        ] {
            let mut cpu = v86_guest(model);
            let caches = cpu.cpu.cpu.legacy.segments;
            for reg in [CpuReg::Rax, CpuReg::Rsp, CpuReg::Rip] {
                let value = cpu.reg(reg);
                cpu.set_reg(reg, value);
            }
            assert_eq!(cpu.cpu.cpu.legacy.segments, caches);
            assert_eq!(
                cpu.run_steps(1, StepRunOptions::default()).unwrap().steps,
                1
            );
            assert_eq!(cpu.cpu.cpu.legacy.cpl, 3);
            assert_ne!(cpu.cpu.cpu.rflags_system & (1 << 17), 0);
        }
    }

    #[test]
    fn v86_diagnostics_and_stop_report_use_paragraph_segments() {
        let cpu = v86_guest(CpuModel::I80386);
        assert!(cpu
            .diagnostic_state()
            .to_string()
            .contains("mode     virtual-8086, cpl 3"));
        let report = cpu
            .stop_report(&CpuError::UnsupportedSystem {
                operation: "diagnostic probe".into(),
                cs: 0x1000,
                ip: 1,
            })
            .unwrap();
        assert!(report.to_string().contains("mode     virtual-8086, cpl 3"));
        assert!(
            report.descriptors.is_empty(),
            "V86 return selectors do not name descriptors"
        );
        assert_eq!(report.stack.len(), 3, "V86 IRET has only IP, CS and FLAGS");
    }

    #[test]
    fn v86_far_transfer_stop_reports_skip_descriptor_lookup() {
        let mut cpu = v86_guest(CpuModel::I80386);
        cpu.write_mem(0x10000, &[0xEA, 0, 0, 0x34, 0x12]).unwrap();
        let report = cpu
            .stop_report(&CpuError::UnsupportedSystem {
                operation: "diagnostic probe".into(),
                cs: 0x1000,
                ip: 0,
            })
            .unwrap();
        assert!(report.descriptors.is_empty());
    }

    #[test]
    fn v86_interrupt_return_stop_report_has_only_three_slots() {
        let cpu = v86_guest(CpuModel::I80386);
        let report = cpu
            .stop_report(&CpuError::UnsupportedSystem {
                operation: "diagnostic probe".into(),
                cs: 0x1000,
                ip: 1,
            })
            .unwrap();
        assert_eq!(report.stack.len(), 3);
    }

    #[test]
    fn v86_delivery_adapter_preserves_vm_markers_and_pushfd_flags() {
        let mut cpu = v86_guest(CpuModel::I80386);
        cpu.write_mem(0x10000, &[0xCC]).unwrap();
        cpu.write_mem(0x808, &[0xFF, 0xFF, 0, 0, 0, 0x9B, 0xCF, 0])
            .unwrap();
        cpu.write_mem(0x810, &[0xFF, 0xFF, 0, 0, 0, 0x93, 0xCF, 0])
            .unwrap();
        cpu.write_mem(0x1018, &[0, 0x20, 8, 0, 0, 0xEE, 0, 0])
            .unwrap();
        cpu.write_mem(0x2000, &[0x90]).unwrap();
        cpu.write_mem(0x5004, &0x9000u32.to_le_bytes()).unwrap();
        cpu.write_mem(0x5008, &0x10u16.to_le_bytes()).unwrap();
        let legacy = &mut cpu.cpu.cpu.legacy;
        legacy.gdtr.base = 0x800;
        legacy.gdtr.limit = 0x17;
        legacy.idtr.base = 0x1000;
        legacy.idtr.limit = 0x7FF;
        legacy.tr.selector = 0x18;
        legacy.tr.cache.base = 0x5000;
        legacy.tr.cache.limit = 0x67;
        legacy.tr.cache.attributes = 0x8B;
        legacy.tr.cache.unusable = 0;
        cpu.enable_delivery_log(8).unwrap();
        cpu.run_steps(1, StepRunOptions::default()).unwrap();
        let mut records = Vec::new();
        cpu.peek_deliveries(&mut records).unwrap();
        assert_eq!(records.len(), 1);
        let event = records[0];
        assert_eq!(
            (event.virtual_8086_before, event.virtual_8086_after),
            (true, false)
        );
        assert_eq!((event.cpl_before, event.cpl_after), (3, 0));
        assert_eq!(event.regs.unwrap().flags & (1 << 17), 0);
        assert!(event.to_string().contains("vm 1->0"));
    }

    #[test]
    fn v86_reset_discards_vm_and_caches_on_all_models() {
        for model in [
            CpuModel::I80386,
            CpuModel::I80486Dx,
            CpuModel::I80486DxCpuid,
        ] {
            let mut cpu = v86_guest(model);
            cpu.map_ram(0xFFFFF000, 0x1000).unwrap();
            cpu.write_mem(0xFFFFFFF0, &[0x90]).unwrap();
            cpu.reset().unwrap();
            let state = cpu.diagnostic_state();
            assert!(!state.system.unwrap().virtual_8086);
            assert!(!state.system.unwrap().protected());
            assert_eq!(state.system.unwrap().cpl, 0);
            assert_eq!(state.code_linear, 0xFFFFFFF0);
            assert_eq!(
                cpu.run_steps(1, StepRunOptions::default()).unwrap().steps,
                1
            );
        }
    }

    #[test]
    fn i386_sparse_backing_reserves_the_physical_span_without_mapping_memory() {
        let cpu = X86JitCpu::new_i386_bare(0, 0x1000, RealSegments::flat(0)).unwrap();
        assert_eq!(
            cpu.memory_backing_stats(),
            MemoryBackingStats {
                sparse: true,
                reserved_bytes: 1 << 32,
                committed_bytes: Some(0),
            }
        );
    }

    #[test]
    fn i386_invalid_sparse_reservation_returns_a_memory_error() {
        let result = X86JitCpu::new_machine_bare(
            0,
            0,
            0,
            RealSegments::flat(0),
            false,
            false,
            CpuModel::I80386,
        );
        assert!(matches!(result, Err(CpuError::Memory(message)) if message == "InvalidSpan"));
    }

    #[test]
    fn i286_keeps_flat_backing_before_any_region_is_mapped() {
        let cpu = X86JitCpu::new_i286_bare(1 << 24, 0, 0, RealSegments::flat(0)).unwrap();
        assert_eq!(
            cpu.memory_backing_stats(),
            MemoryBackingStats {
                sparse: false,
                reserved_bytes: 1 << 24,
                committed_bytes: Some(1 << 24),
            }
        );
    }

    #[test]
    fn i386_constructor_selects_machine_model_and_executes_real_entry() {
        let mut cpu = X86JitCpu::new_i386_bare(0, 0x1000, RealSegments::flat(0)).unwrap();
        assert_eq!(cpu.vm.cpu_model(), Some(CpuModel::I80386));
        assert_eq!(cpu.vm.execution_profile(), ExecutionProfile::Machine);
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(0x1000, &[0x66, 0xB8, 0x78, 0x56, 0x34, 0x12, 0xF4])
            .unwrap();
        assert_eq!(
            cpu.run_steps(10, StepRunOptions::default()).unwrap().stop,
            CpuStepStop::Exit(CpuStop::Halt)
        );
        assert_eq!(cpu.reg(CpuReg::Rax), 0x12345678);
    }

    /// Intel 80386 Programmer's Reference Manual, §10.1 Initialization:
    /// near jumps retain the reset base; the first CS reload returns to low ROM.
    #[test]
    fn i386_reset_executes_high_near_jump_then_reloads_cs_and_keeps_ram() {
        let mut cpu = X86JitCpu::new_i386_bare(0, 0x1000, RealSegments::flat(0)).unwrap();
        cpu.map_ram(0, 0x10000).unwrap();
        cpu.write_mem(0x600, &[0xA5]).unwrap();
        cpu.write_mem(0x1000, &[0xB0, 0x5A, 0xF4]).unwrap();
        let mut rom = vec![0x0F; 0x20000];
        rom[0x1FFF0..0x1FFF9]
            .copy_from_slice(&[0xEB, 0x02, 0x0F, 0x0B, 0xEA, 0x00, 0x10, 0x00, 0x00]);
        cpu.map_rom(0xFFFE0000, &rom).unwrap();
        for expected_retired in [4, 8] {
            cpu.reset().unwrap();
            assert_eq!(cpu.cpu.cpu.cs, 0xF000);
            assert_eq!(cpu.cpu.cpu.rip, 0xFFF0);
            assert_eq!(cpu.cpu.cpu.legacy.segments[0].base, 0xFFFF0000);
            assert_eq!(cpu.cpu.cpu.legacy.cr0, 0, "the board has no 80387");
            assert_eq!(cpu.reg(CpuReg::Rdx), 0x0303);
            assert_eq!(cpu.cpu.cpu.fpu_model, FpuModel::None);
            assert_eq!(
                cpu.run_steps(0, StepRunOptions::default()).unwrap().steps,
                0
            );
            assert_eq!(
                cpu.run_steps(1, StepRunOptions::default()).unwrap().steps,
                1
            );
            assert_eq!(cpu.cpu.cpu.rip, 0xFFF4);
            assert_eq!(cpu.cpu.cpu.legacy.segments[0].base, 0xFFFF0000);
            assert_eq!(
                cpu.run_steps(1, StepRunOptions::default()).unwrap().steps,
                1
            );
            assert_eq!(cpu.cpu.cpu.legacy.segments[0].base, 0);
            assert_eq!(cpu.cpu.cpu.legacy.reset_fetch, 0);
            assert_eq!(
                cpu.run_steps(2, StepRunOptions::default()).unwrap().stop,
                CpuStepStop::Exit(CpuStop::Halt)
            );
            assert_eq!(cpu.reg(CpuReg::Rax) & 0xFF, 0x5A);
            assert_eq!(cpu.retired(), expected_retired);
            let mut byte = [0];
            cpu.read_mem(0x600, &mut byte).unwrap();
            assert_eq!(byte, [0xA5]);
        }
    }

    #[test]
    fn i386_reset_reapplies_disabled_a20_after_installing_the_image() {
        let mut cpu = X86JitCpu::new_i386_bare(0, 0x1000, RealSegments::flat(0)).unwrap();
        cpu.map_rom(0xFFFE0000, &vec![0xF4; 0x20000]).unwrap();
        cpu.set_a20_enabled(false);
        cpu.reset().unwrap();
        assert!(!cpu.a20_enabled());
        assert!(matches!(
            cpu.run_steps(1, StepRunOptions::default()),
            Err(CpuError::Unmapped { addr: 0xFFEFFFF0 })
        ));
        assert_eq!(cpu.retired(), 0);
        cpu.set_a20_enabled(true);
        cpu.reset().unwrap();
        assert!(cpu.a20_enabled());
        assert_eq!(
            cpu.run_steps(1, StepRunOptions::default()).unwrap().stop,
            CpuStepStop::Exit(CpuStop::Halt)
        );
    }
}

#[cfg(test)]
mod i486_tests {
    use super::*;

    #[test]
    fn i486_reset_uses_model_signature_control_bits_and_internal_fpu_route() {
        for (cpuid, signature) in [(false, 0x0403), (true, 0x0410)] {
            let mut cpu =
                X86JitCpu::new_i486_bare(0, 0x1000, RealSegments::default(), cpuid).unwrap();
            assert!(cpu.memory_backing_stats().sparse);
            for a20 in [false, true] {
                cpu.cpu.cpu.legacy.cr0 = 1;
                cpu.cpu.set_a20_enabled(a20);
                cpu.reset().unwrap();
                assert_eq!(cpu.cpu.cpu.legacy.cr0, 0x6000_0010);
                assert_eq!(cpu.cpu.cpu.gpr[2], signature);
                assert_eq!(cpu.cpu.cpu.fpu_model, FpuModel::Present);
                assert_eq!(cpu.cpu.numeric_error_route(), NumericErrorRoute::Internal);
                assert_eq!(cpu.cpu.cpu.legacy.segments[0].base, 0xFFFF0000);
                assert_eq!(cpu.cpu.cpu.rip, 0xFFF0);
                assert_eq!(cpu.cpu.a20_enabled(), a20);
            }
        }
    }
}
