// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bochs VBE DISPI display interface: the register file a VGA-compatible
//! card adds for linear high-colour modes.
//!
//! Sources: the Bochs VBE DISPI interface (`vbe.h`) and the Bochs/VBEBios
//! "VBE Display API" document (interface IDs B0C0h–B0C5h), read for
//! behaviour only. The register file lives in [`crate::Vga`], which keeps its
//! IBM VGA behaviour while DISPI is disabled. See
//! `backlog/docs/design/machine-config.md` §Display cards and
//! `backlog/docs/design/observation.md` §Register-level VGA graphics scanout.
//!
//! Video memory is guest RAM: the board maps it as ordinary CPU RAM at
//! [`LFB_BASE`], so linear-framebuffer stores take the CPU's RAM path and this
//! device never touches them. The A0000h–AFFFFh window is one more view of
//! the same bytes: the machine serves each window access straight from video
//! memory at the bank [`BochsVbe::window_target`] names, so there is one
//! backing and nothing to keep in step.
//!
//! | Interface feature | Model |
//! | --- | --- |
//! | Index port 1CEh, data ports 1CFh–1D0h | READ: 16-bit registers; Bochs (`vbe_write`) takes byte, word and doubleword accesses at 1CEh–1CFh, and a byte write writes the register zero-extended at once. On ISA a word access at the odd data port arrives as two byte cycles (IBM Personal Computer AT Technical Reference, I/O Channel, -I/O CS16): 1CFh carries the low byte, the 1D0h alias the high byte. The card writes a lone 1CFh byte, zero-extended, before its next access or any host query, which no observer can tell apart from at once; a high byte that follows in the same access joins it into one word write. ASSUMED: a 1D0h byte with no low byte before it writes the high byte alone; byte reads return the low byte at 1CFh and the high byte at 1D0h; a word at 1D0h is the whole data register (QEMU decodes 1D0h as data, Bochs does not); a doubleword at 1CEh writes index then data |
//! | ID (0) | READ: writes B0C0h–B0C5h select the interface version and read back; other values are ignored. Power-on B0C5h |
//! | XRES, YRES, BPP (1–3) | READ: GETCAPS (ENABLE bit 1) makes reads return 2560, 1600 and 32. Writes while enabled are ignored (the document calls them illegal). BPP 0 means 8. ASSUMED deviation: larger resolutions clamp to the maxima, where Bochs refuses values outside 320–2560 and 200–1600 |
//! | ENABLE (4) | READ: bits 0, 1, 5, 6, 7 kept, the rest read zero. Enabling sets the virtual width to XRES and clears both offsets; unless NOCLEARMEM (bit 7) is set it clears the whole video memory, as Bochs does. Bit 5 selects the eight-bit DAC whether or not bit 0 is set. ASSUMED: LFB_ENABLED (bit 6) is stored only: the framebuffer and the A0000h window are both always decoded while enabled |
//! | BANK (5) | READ: 64 KiB window at A0000h into video memory. ASSUMED: a bank past the end of memory is ignored, so a read-back shows the refusal; the BANK_RD/BANK_WR split of later Bochs releases is not modelled. B0000h–BFFFFh reads FFh while enabled. An access straddling AFFFFh and B0000h reads FFh and is dropped whole |
//! | VIRT_WIDTH, VIRT_HEIGHT (6, 7) | READ: the virtual width is at least XRES and shrinks to fit memory; VIRT_HEIGHT is read-only: memory / (virtual width × bytes per pixel), at most FFFFh |
//! | X_OFFSET, Y_OFFSET (8, 9) | READ: display start in pixels. ASSUMED: clamped so the visible screen stays inside the virtual screen |
//! | VIDEO_MEMORY_64K (0Ah) | READ: memory size in 64 KiB units, read-only |
//! | Other indices | ASSUMED: read zero, writes ignored |
//! | 4 bpp planar modes | NOT SUPPORTED: BPP 4, which Bochs accepts, is refused (the write is ignored). Planar 4-bit VBE modes need the VGA planes as video memory; the bundled VGA BIOS build must not offer them (SeaVGABIOS lists MM_PLANAR modes 102h, 104h and 106h) |
//! | Instruction fetch from video memory | NOT SUPPORTED: the board maps it read/write and never executable, and the window is trapped, so code cannot run from either |

use std::cell::Cell;
use std::rc::Rc;

/// The DISPI index port.
pub const INDEX_PORT: u16 = 0x1CE;
/// The DISPI data port.
pub const DATA_PORT: u16 = 0x1CF;
/// The data alias decoded with [`DATA_PORT`], so a word access there stays in
/// the interface.
pub const DATA_ALIAS_PORT: u16 = 0x1D0;
/// Physical address of the linear framebuffer on a board without PCI.
pub const LFB_BASE: u64 = 0xE000_0000;
/// First address of the banked window into video memory.
pub const BANK_WINDOW_BASE: u64 = 0xA_0000;
/// Size of the banked window and of one bank.
pub const BANK_SIZE: usize = 0x1_0000;
/// Largest horizontal resolution reported through GETCAPS.
pub const MAX_XRES: u16 = 2560;
/// Largest vertical resolution reported through GETCAPS.
pub const MAX_YRES: u16 = 1600;
/// Deepest pixel format reported through GETCAPS.
pub const MAX_BPP: u16 = 32;
/// Video memory sizes a board can fit, in MiB.
pub const MEMORY_SIZES_MIB: &[u32] = &[4, 8, 16];
/// Video memory of a card whose machine file names no size, in MiB.
pub const DEFAULT_MEMORY_MIB: u32 = 16;

/// ENABLE bit 0: DISPI modes replace VGA scanout.
pub const ENABLED: u16 = 0x01;
/// ENABLE bit 1: XRES, YRES and BPP read their maxima.
pub const GETCAPS: u16 = 0x02;
/// ENABLE bit 5: the DAC keeps eight bits per component.
pub const DAC_8BIT: u16 = 0x20;
/// ENABLE bit 6: the guest uses the linear framebuffer.
pub const LFB_ENABLED: u16 = 0x40;
/// ENABLE bit 7: enabling keeps video memory.
pub const NO_CLEAR_MEM: u16 = 0x80;
const ENABLE_MASK: u16 = ENABLED | GETCAPS | DAC_8BIT | LFB_ENABLED | NO_CLEAR_MEM;
const ID_FIRST: u16 = 0xB0C0;
const ID_LAST: u16 = 0xB0C5;
const REGISTERS: usize = 10;
/// Bytes [`BochsVbe::save`] appends to a VGA snapshot.
pub(crate) const SNAPSHOT_LEN: usize = 4 + 2 + 2 * REGISTERS;

/// Unsupported video memory size.
#[derive(Debug, thiserror::Error)]
#[error("Bochs VBE video memory must be 4, 8 or 16 MiB")]
pub struct BochsVbeConfigError;

/// The DISPI register file, as the guest last left it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DispiRegisters {
    /// Selected register index.
    pub index: u16,
    /// Interface version, B0C0h–B0C5h.
    pub id: u16,
    /// Visible width in pixels.
    pub xres: u16,
    /// Visible height in lines.
    pub yres: u16,
    /// Bits per pixel: 8, 15, 16, 24 or 32.
    pub bpp: u16,
    /// ENABLE flags.
    pub enable: u16,
    /// Bank shown in the A0000h window.
    pub bank: u16,
    /// Pixels per virtual scan line.
    pub virt_width: u16,
    /// Virtual lines that fit video memory.
    pub virt_height: u16,
    /// Display start column in pixels.
    pub x_offset: u16,
    /// Display start line.
    pub y_offset: u16,
}

impl DispiRegisters {
    /// Bochs initial mode: 640×480 at 8 bpp, disabled.
    const POWER_ON: Self = Self {
        index: 0,
        id: ID_LAST,
        xres: 640,
        yres: 480,
        bpp: 8,
        enable: 0,
        bank: 0,
        virt_width: 640,
        virt_height: 0,
        x_offset: 0,
        y_offset: 0,
    };

    fn bytes_per_pixel(self) -> u64 {
        u64::from(self.bpp).div_ceil(8)
    }

    fn line_bytes(self) -> u64 {
        u64::from(self.virt_width) * self.bytes_per_pixel()
    }

    /// Keep the virtual screen within `memory` and the visible screen within
    /// the virtual screen.
    fn fit(&mut self, memory: u64) {
        let pixel = self.bytes_per_pixel();
        let lines = u64::from(self.yres).max(1);
        self.virt_width = self.virt_width.max(self.xres);
        if self.line_bytes() * lines > memory {
            let widest = (memory / (pixel * lines)).min(u64::from(u16::MAX)) as u16;
            self.virt_width = widest.max(self.xres);
        }
        self.virt_height = match self.line_bytes() {
            0 => 0,
            line => (memory / line).min(u64::from(u16::MAX)) as u16,
        };
        self.x_offset = self.x_offset.min(self.virt_width - self.xres);
        self.y_offset = self
            .y_offset
            .min(self.virt_height.saturating_sub(self.yres));
    }
}

/// Where the visible screen sits in video memory, in the guest's pixel format.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LinearScanout {
    /// Byte offset of the first visible pixel from the start of video memory.
    pub offset: u64,
    /// Visible pixels per line.
    pub width: u16,
    /// Visible lines that lie wholly inside video memory.
    pub height: u16,
    /// Bits per pixel: 8, 15, 16, 24 or 32.
    pub bits_per_pixel: u16,
    /// Bytes from one scan line to the next.
    pub stride: u32,
}

impl LinearScanout {
    /// Bytes from [`Self::offset`] to the end of the last visible pixel.
    #[must_use]
    pub fn byte_len(&self) -> u64 {
        if self.width == 0 || self.height == 0 {
            return 0;
        }
        u64::from(self.height - 1) * u64::from(self.stride)
            + u64::from(self.width) * u64::from(self.bits_per_pixel).div_ceil(8)
    }
}

/// Host handle on one card's DISPI state; clones share the card.
///
/// The machine serves the banked window through [`Self::window_target`] and
/// performs the clears enabling requests ([`Self::clears`]); observation
/// reads [`Self::linear_scanout`]. Guest traffic reaches the registers only
/// through the VGA's bus faces.
#[derive(Clone)]
pub struct BochsVbe(Rc<Shared>);

struct Shared {
    memory: u64,
    regs: Cell<DispiRegisters>,
    clears: Cell<u64>,
    /// A low data byte that may yet be joined by its high byte (see the
    /// coverage matrix).
    staged: Cell<Option<u8>>,
}

impl BochsVbe {
    /// A powered-on interface over `memory_mib` MiB of video memory.
    ///
    /// # Errors
    /// [`BochsVbeConfigError`] unless the size is one of [`MEMORY_SIZES_MIB`].
    pub fn new(memory_mib: u32) -> Result<Self, BochsVbeConfigError> {
        if !MEMORY_SIZES_MIB.contains(&memory_mib) {
            return Err(BochsVbeConfigError);
        }
        let memory = u64::from(memory_mib) << 20;
        let mut regs = DispiRegisters::POWER_ON;
        regs.fit(memory);
        Ok(Self(Rc::new(Shared {
            memory,
            regs: Cell::new(regs),
            clears: Cell::new(0),
            staged: Cell::new(None),
        })))
    }

    /// Video memory size in bytes.
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        self.0.memory
    }

    /// The current registers, without a guest access.
    #[must_use]
    pub fn registers(&self) -> DispiRegisters {
        self.settle();
        self.0.regs.get()
    }

    /// Write a staged low data byte: its high byte did not follow.
    fn settle(&self) {
        if let Some(low) = self.0.staged.take() {
            self.write_data(u16::from(low));
        }
    }

    /// Whether DISPI modes replace VGA scanout and the A0000h window.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.registers().enable & ENABLED != 0
    }

    /// Whether the DAC keeps eight bits per component.
    #[must_use]
    pub fn dac_8bit(&self) -> bool {
        self.registers().enable & DAC_8BIT != 0
    }

    /// Counts the clears of the whole video memory that enabling requested.
    #[must_use]
    pub fn clears(&self) -> u64 {
        self.settle();
        self.0.clears.get()
    }

    /// The bank the A0000h window shows; `None` while DISPI is disabled.
    #[must_use]
    pub fn window_bank(&self) -> Option<u16> {
        self.enabled().then(|| self.registers().bank)
    }

    /// The physical address in video memory that window address `addr`
    /// shows: [`LFB_BASE`] + bank × 64 KiB + its offset. `None` outside
    /// A0000h–AFFFFh or while DISPI is disabled, when the VGA serves it.
    #[must_use]
    pub fn window_target(&self, addr: u64) -> Option<u64> {
        let offset = addr.checked_sub(BANK_WINDOW_BASE)?;
        if offset >= BANK_SIZE as u64 {
            return None;
        }
        let bank = self.window_bank()?;
        Some(LFB_BASE + u64::from(bank) * BANK_SIZE as u64 + offset)
    }

    /// The visible screen while DISPI is enabled.
    #[must_use]
    pub fn linear_scanout(&self) -> Option<LinearScanout> {
        let regs = self.registers();
        if regs.enable & ENABLED == 0 {
            return None;
        }
        let pixel = regs.bytes_per_pixel();
        let stride = regs.line_bytes();
        let offset = u64::from(regs.y_offset) * stride + u64::from(regs.x_offset) * pixel;
        let row = u64::from(regs.xres) * pixel;
        // Only a mode larger than memory clips: the registers keep every
        // offset inside the virtual screen.
        let height = if row == 0 {
            0
        } else {
            let room = self.0.memory.saturating_sub(offset);
            let rows = if room < row {
                0
            } else {
                (room - row) / stride + 1
            };
            rows.min(u64::from(regs.yres)) as u16
        };
        Some(LinearScanout {
            offset,
            width: regs.xres,
            height,
            bits_per_pixel: regs.bpp,
            stride: stride as u32,
        })
    }

    pub(crate) fn read_index(&self) -> u16 {
        self.registers().index
    }

    /// A byte cycle at the data port: the low byte, staged.
    pub(crate) fn write_data_low(&self, value: u8) {
        self.settle();
        self.0.staged.set(Some(value));
    }

    /// A byte cycle at the data alias: the high byte, completing a staged low
    /// byte.
    pub(crate) fn write_data_high(&self, value: u8) {
        let low = self.0.staged.take().unwrap_or(0);
        self.write_data(u16::from_le_bytes([low, value]));
    }

    pub(crate) fn write_index(&self, value: u16) {
        let mut regs = self.registers();
        regs.index = value;
        self.0.regs.set(regs);
    }

    pub(crate) fn read_data(&self) -> u16 {
        let regs = self.registers();
        let caps = regs.enable & GETCAPS != 0;
        match regs.index {
            0 => regs.id,
            1 if caps => MAX_XRES,
            2 if caps => MAX_YRES,
            3 if caps => MAX_BPP,
            1 => regs.xres,
            2 => regs.yres,
            3 => regs.bpp,
            4 => regs.enable,
            5 => regs.bank,
            6 => regs.virt_width,
            7 => regs.virt_height,
            8 => regs.x_offset,
            9 => regs.y_offset,
            0x0A => (self.0.memory / BANK_SIZE as u64) as u16,
            _ => 0,
        }
    }

    pub(crate) fn write_data(&self, value: u16) {
        self.0.staged.set(None);
        let mut regs = self.0.regs.get();
        let enabled = regs.enable & ENABLED != 0;
        match regs.index {
            0 if (ID_FIRST..=ID_LAST).contains(&value) => regs.id = value,
            1 if !enabled => regs.xres = value.min(MAX_XRES),
            2 if !enabled => regs.yres = value.min(MAX_YRES),
            3 if !enabled => {
                let bpp = if value == 0 { 8 } else { value };
                if matches!(bpp, 8 | 15 | 16 | 24 | 32) {
                    regs.bpp = bpp;
                }
            }
            4 => {
                let value = value & ENABLE_MASK;
                if value & ENABLED != 0 && !enabled {
                    regs.virt_width = regs.xres;
                    regs.x_offset = 0;
                    regs.y_offset = 0;
                    regs.enable = value;
                    regs.fit(self.0.memory);
                    if value & NO_CLEAR_MEM == 0 {
                        self.0.clears.set(self.0.clears.get() + 1);
                    }
                }
                regs.enable = value;
            }
            5 if u64::from(value) < self.0.memory / BANK_SIZE as u64 => regs.bank = value,
            6 => regs.virt_width = value,
            8 => regs.x_offset = value,
            9 => regs.y_offset = value,
            _ => {}
        }
        regs.fit(self.0.memory);
        self.0.regs.set(regs);
    }

    /// Power-on registers; video memory is guest RAM and keeps its contents.
    pub(crate) fn reset(&self) {
        self.0.staged.set(None);
        let mut regs = DispiRegisters::POWER_ON;
        regs.fit(self.0.memory);
        self.0.regs.set(regs);
    }

    pub(crate) fn save(&self) -> Vec<u8> {
        let r = self.registers();
        let mut blob = Vec::with_capacity(SNAPSHOT_LEN);
        blob.extend_from_slice(&(self.0.memory as u32).to_le_bytes());
        blob.extend_from_slice(&r.index.to_le_bytes());
        for value in [
            r.id,
            r.xres,
            r.yres,
            r.bpp,
            r.enable,
            r.bank,
            r.virt_width,
            r.virt_height,
            r.x_offset,
            r.y_offset,
        ] {
            blob.extend_from_slice(&value.to_le_bytes());
        }
        blob
    }

    /// Registers from a [`Self::save`] blob, refused unless the guest could
    /// have produced them on a card of this memory size.
    pub(crate) fn decode(&self, blob: &[u8]) -> Option<DispiRegisters> {
        if blob.len() != SNAPSHOT_LEN
            || u64::from(u32::from_le_bytes(blob[..4].try_into().ok()?)) != self.0.memory
        {
            return None;
        }
        let word = |at: usize| u16::from_le_bytes([blob[at], blob[at + 1]]);
        let regs = DispiRegisters {
            index: word(4),
            id: word(6),
            xres: word(8),
            yres: word(10),
            bpp: word(12),
            enable: word(14),
            bank: word(16),
            virt_width: word(18),
            virt_height: word(20),
            x_offset: word(22),
            y_offset: word(24),
        };
        let mut fitted = regs;
        fitted.fit(self.0.memory);
        let valid = (ID_FIRST..=ID_LAST).contains(&regs.id)
            && regs.xres <= MAX_XRES
            && regs.yres <= MAX_YRES
            && matches!(regs.bpp, 8 | 15 | 16 | 24 | 32)
            && regs.enable & !ENABLE_MASK == 0
            && u64::from(regs.bank) < self.0.memory / BANK_SIZE as u64
            && fitted == regs;
        valid.then_some(regs)
    }

    /// Install decoded registers; video memory is guest RAM and is restored
    /// with it.
    pub(crate) fn restore(&self, regs: DispiRegisters) {
        self.0.staged.set(None);
        self.0.regs.set(regs);
    }
}

#[cfg(test)]
#[path = "bochs_vbe_tests.rs"]
mod tests;
