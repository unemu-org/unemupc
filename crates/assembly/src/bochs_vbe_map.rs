// SPDX-License-Identifier: AGPL-3.0-or-later
//! Video memory of a Bochs VBE card as CPU RAM (Bochs VBE DISPI interface,
//! `vbe.h`).
//!
//! Assembly maps the card's whole video memory as read/write CPU RAM at the
//! linear-framebuffer address, so framebuffer stores take the CPU's RAM path
//! on every engine and nothing is trapped. That RAM is the only copy of video
//! memory: the machine serves the banked A0000h window straight from it
//! ([`window`]), at the bank the card's register names at each access, and
//! observation reads the visible screen from it. The one guest action that
//! writes video memory from the card's side is enabling DISPI without
//! NOCLEARMEM, which clears it; [`BochsVbeClear`] performs that clear after
//! the DISPI port write that requested it.

use cpu::{CpuError, X86JitCpu};
use devices::bochs_vbe::{BochsVbe, BANK_SIZE, LFB_BASE};
use machine::MemoryMapSync;

/// The DISPI ports, the only writes after which the card can request a clear.
pub(crate) const DISPI_PORTS: std::ops::RangeInclusive<u16> =
    devices::bochs_vbe::INDEX_PORT..=devices::bochs_vbe::DATA_ALIAS_PORT;

static ZEROS: [u8; 1 << 20] = [0; 1 << 20];

/// The banked window and its target: A0000h–AFFFFh shows video memory at the
/// selected bank while DISPI is enabled.
pub(crate) fn window(
    vbe: BochsVbe,
) -> (std::ops::RangeInclusive<u64>, impl Fn(u64) -> Option<u64>) {
    let base = devices::bochs_vbe::BANK_WINDOW_BASE;
    (base..=base + BANK_SIZE as u64 - 1, move |addr| {
        vbe.window_target(addr)
    })
}

/// Clears one card's video memory each time enabling requests it.
pub(crate) struct BochsVbeClear {
    vbe: BochsVbe,
    /// Clear requests already carried out.
    clears: u64,
}

impl BochsVbeClear {
    pub(crate) fn new(vbe: BochsVbe) -> Self {
        Self {
            clears: vbe.clears(),
            vbe,
        }
    }

    /// The counter moves only after the clear succeeded, so a failed clear
    /// is retried in full at the next DISPI write or run start.
    pub(crate) fn sync(
        &mut self,
        cpu: &mut X86JitCpu,
        event: MemoryMapSync,
    ) -> Result<(), CpuError> {
        if event != MemoryMapSync::Apply {
            return Ok(());
        }
        let clears = self.vbe.clears();
        if clears != self.clears {
            let memory = self.vbe.memory_bytes();
            let mut offset = 0;
            while offset < memory {
                let count = (memory - offset).min(ZEROS.len() as u64) as usize;
                cpu.write_mem(LFB_BASE + offset, &ZEROS[..count])?;
                offset += count as u64;
            }
            self.clears = clears;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "bochs_vbe_map_tests.rs"]
mod tests;
