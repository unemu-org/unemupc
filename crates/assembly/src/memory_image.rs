// SPDX-License-Identifier: AGPL-3.0-or-later
//! Read-only images of system RAM at a completed run boundary.
//!
//! The assembly memory description owns the system RAM ranges (IBM Personal
//! Computer AT Technical Reference, System Board, System Memory Map). Images
//! copy those ranges through the CPU backend's physical host read API, without
//! applying A20 or accessing a device. Configured device windows, including an
//! EMS page frame temporarily held as CPU RAM, are omitted. ROM is omitted.
//! See `backlog/docs/design/observation.md` §Guest RAM image.
//!
//! An image contains guest bytes and is for local diagnosis only. It must
//! never be added to a repository. Capturing or formatting it changes no
//! guest state, retired count or virtual time.

use crate::{AssembledMachine, MemoryConfig};
use std::fmt;

/// The storage kind of a region in a [`MemoryImage`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// Writable system memory.
    Ram,
    /// Read-only memory; the current capture API omits ROM regions.
    Rom,
}

/// An owned copy of one physical memory region, without CPU address gating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    /// Physical base address, before any CPU A20 aliasing.
    pub base: u64,
    /// Whether the region holds RAM or ROM.
    pub kind: MemoryKind,
    /// Bytes as stored at the capture boundary.
    pub bytes: Vec<u8>,
}

/// System RAM in ascending physical address order, with gaps left out.
///
/// This is diagnostic data, not a restorable machine snapshot. Its display
/// lists only each region's kind, physical base and byte length in hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryImage {
    /// Owned regions; device windows are never included by capture.
    pub regions: Vec<MemoryRegion>,
}

impl AssembledMachine {
    /// Copy every system RAM range in the memory description used to assemble
    /// this machine, including conventional, extended and upper-memory RAM.
    ///
    /// Reads use physical host addresses, independent of A20. No device is
    /// borrowed, synchronised or read, so VGA latches and all other guest state
    /// stay unchanged. Device windows and ROM are omitted. The front end owns
    /// the on-disk format; the bytes are for local diagnosis only and must
    /// never be added to a repository. See [`crate::memory_image`].
    ///
    /// Returns the CPU's memory error if a supplied range cannot be read.
    pub fn memory_image(&self, memory: &MemoryConfig) -> Result<MemoryImage, cpu::CpuError> {
        let mut ram = memory.ram.clone();
        ram.sort_by_key(|region| region.base);
        let mut regions = Vec::with_capacity(ram.len());
        for region in ram {
            let mut bytes = vec![0; region.size as usize];
            self.machine.cpu().read_mem(region.base, &mut bytes)?;
            regions.push(MemoryRegion {
                base: region.base,
                kind: MemoryKind::Ram,
                bytes,
            });
        }
        Ok(MemoryImage { regions })
    }
}

impl fmt::Display for MemoryImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "memory image")?;
        for region in &self.regions {
            let kind = match region.kind {
                MemoryKind::Ram => "ram",
                MemoryKind::Rom => "rom",
            };
            writeln!(
                f,
                "{kind:<8} base {:#010x}, length {:#x}",
                region.base,
                region.bytes.len()
            )?;
        }
        Ok(())
    }
}
