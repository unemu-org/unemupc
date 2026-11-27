// SPDX-License-Identifier: AGPL-3.0-or-later
//! IBM VGA register file, four-plane video memory and passive text and graphics scanout.
//!
//! Source: IBM VGA/XGA Technical Reference Manual (May 1992), VGA Function,
//! especially the figures cited below. See `backlog/docs/design/spec.md` §7.1
//! and §9, and `backlog/docs/design/firmware-and-machine-classes.md` §3.6.
//!
//! READ means checked against the manual; DERIVED identifies address mapping
//! inferred from its figures; ASSUMED marks model choices or unspecified cases.
//!
//! | IBM figure / section | Verified coverage | Limit or model choice |
//! | --- | --- | --- |
//! | Figs. 2-19–2-26: general registers | READ: port directions and MSR bit 0 select CRTC, status 1 and feature-control addresses | ASSUMED: inactive pair reads FF; status 0 models a loaded colour monitor from DAC entry 0 with threshold 20h on any component; reserved MSR/feature bits retain writes. RAM-enable gating and IRQ omitted |
//! | Figs. 2-29–2-37: sequencer | READ: five indexed registers, map mask, chain-4 and independent odd/even write control | Reset signalling, CPU extended-memory gating and cycle-exact display timing omitted; undefined read bits return their masked stored values |
//! | Fig. 2-58: CRTC protection | READ: R11 bit 7 protects R0–R7 except R7 bit 4 | Polling uses totals/overflow/display-end/retrace-end; other registers retain full bytes, including reserved bits |
//! | Figs. 2-81–2-89: attribute controller | READ: index/data flip-flop, status reset, PAS, palette widths; R10 mask EF, R11 mask FF, R12 mask 0F | READ: IBM VGA/XGA Technical Reference, Figs. 2-81–2-82, Address Register (IPAS): clear IPAS to load the palette, set for normal display; Type 1 disables PAS=1 palette accesses, Type 2 does not guarantee the addressed colour. COMPATIBILITY POLICY (not verified hardware): accept PAS=1 palette writes because Windows 3.x vga.drv relies on them (driver evidence); PAS=1 palette reads still return stored data. PAS=0 display blanking/overscan is unchanged. ASSUMED: reserved bits read zero |
//! | Fig. 2-94, pp. 2-104–2-105: DAC | READ: one eight-bit palette address, read/write state, RGB order, six-bit components, increment after three bytes and address writes restart the cycle | ASSUMED: 3C8 reads shared address n during a read cycle (some RAMDACs prefetch n+1); components are stored immediately even if an address write aborts the triple; eight-bit wrap and wrong-direction data accesses use the shared component phase without changing state |
//! | Fig. 2-94, p. 2-106: PEL mask | READ: mask register address and mode-set value FF | ASSUMED: writable mask ANDs the displayed palette index only; IBM warns against writes, while firmware programs it |
//! | Fig. 2-25: input status 1 | READ: vertical retrace implies display disable (DE=1) | DERIVED: integer dot clocks from virtual PIT time and programmed totals/retrace; ASSUMED: fixed ~70 Hz fallback before usable CRTC timing, unsupported clock selects use 25.175 MHz |
//! | Figs. 2-77–2-78: memory maps | READ: all four GC memory windows | ASSUMED: unselected bytes read FF without loading latches and discard writes; 128 KiB planar map mirrors at 64 KiB |
//! | Figs. 2-36, 2-74–2-77: odd/even | READ: parity selects maps, GC read-map bit 1 selects the read pair, GR6 bit 1 enables A0 replacement; otherwise A0 passes unchanged | ASSUMED: replacement is gated by odd/even routing (writes with SR4 bit 2 clear; reads with GR5 bit 4 set), not GR6 bit 1 alone; replacement uses CPU A16 in map 00 and inverted MSR bit 5 in other maps |
//! | Figs. 2-37 and 2-65: chain-4 | DERIVED: plane = a & 3; offset = (a & FFFC) OR ((a >> 14) & 3), preserving doubleword scanout placement after unchaining | ASSUMED: both reads and writes use this mapping and the full GC latch/logic/bit-mask pipeline, with sequencer map mask applied |
//! | Figs. 2-69–2-80: read/write modes | READ: all four write modes, rotate, set/reset, logic in modes 0/2/3, bit/map masks, read-map select and colour comparison with don't-care | Memory contention omitted |
//! | Alphanumeric Modes, Figs. 2-31, 2-33–2-36, 2-41, 2-48, 2-50–2-56, 2-59–2-61, 2-64–2-65, 2-82–2-83, 2-84, 2-86, 2-89 and 2-94 | Passive planar text state: geometry, start, stride, cursor shape, font maps, blink, palette and DAC | Ordinary cell grids plus indexed raster output for panning, preset row scan and splits; cursor skew omitted |
//! | Graphics Modes, Figs. 2-31, 2-50, 2-60–2-65, 2-75, 2-84–2-89 | Planar 16-colour and 256-colour serializers, logical geometry, start/stride, palette and screen-off | CGA-interleaved shift, mismatched GC/AC serializer settings, mid-frame changes other than the split and smooth scrolling finer than a frame unmodelled; unsupported serializer settings blank the frame. READ: CMS0/SRC substitute row-scan bits into MA13/MA14 (Fig. 2-64), including mode 06h. Count-by-two/four variations and cycle-exact raster timing not claimed. Dot and scan-line repeats accompany logical pixels for presentation; Shift 4/Shift Load are ignored in Extended Graphics as specified in Fig. 2-31 |
//! | Figs. 2-48–2-50, 2-53–2-54, 2-60, 2-66, 2-84, 2-87–2-88; Split Screen Operation | READ: start-address retrace latch, pixel/byte panning, preset row scan and 10-bit physical-line compare; AC bit 5 suppresses lower-screen pixel panning | ASSUMED frame approximation: CRTC 08h shares the start-address latch; AC panning latches at retrace end; odd packed values ignore bit 0; split begins after the compared line and resets row/double-scan phase. Byte pan in supported single-shift modes is inferred; see observation.md §Frame scrolling |
//! | Mode 13h internal palette, Fig. 2-89 and VGA Programming Considerations, pp. 2-96–2-97 | DERIVED/ASSUMED: each four-bit sample passes through Color Plane Enable and the internal palette low nibble; Color Select, P54S and palette high bits do not affect paired eight-bit output | IBM says "the 8-bit attribute is the digital color value to the video DAC" and "should not modify the contents of the Internal Palette registers". The nonidentity-palette behaviour is a model inference, not verified on physical hardware |
//! | Character Map Select / colour path | Compatibility policy: attribute bit 3 selects the font and remains in the foreground index; Color Plane Enable masks it | Maintainer-selected policy, supported by Linux vgacon.c, vgacon_do_font_op (512-character mode writes CPE=07h), DOSBox Staging vga_draw.cpp, draw_text_line_from_dac_palette, and 86Box vid_svga_render.c, svga_render_text_80 (foreground keeps all four attribute bits) |
//! | Figs. 2-51 and 2-61: cursor start / underline | ASSUMED deviation: register values are zero-based scan lines, without adding one for the manual's minus-one wording | SeaVGABIOS stdvgamodes.c crtc_07 uses CR09=4Fh and CR14=0Fh; adding one would place its underline outside the 16-line cell. Cursor start uses the same zero-based convention; no physical-card verification |
//! | Power-on / model reset | Reset clears graphics registers, indices, latches, palette and RAM, including PEL mask; CPU decode starts enabled | ASSUMED deterministic initialization, not an electrical reset sequence |
//!
//! A literal reading of IBM Fig. 2-77 enables A0 replacement from GR6 bit 1
//! alone. This model calls the replacement logic only after selecting odd/even
//! addressing: SR4 bit 2 clear for writes, GR5 bit 4 set for reads. That gating
//! is ASSUMED and unverified on real hardware. The mixed setting GR6=02h with
//! GR5 bit 4 clear is not used by the reviewed BIOS modes. The test
//! `odd_even_round_trip_pairs_chain_bit_and_independent_read_write_controls`
//! in `vga/tests.rs` records the model's unchanged planar read address in that
//! setting, rather than establishing hardware behaviour.
//!
//! DAC edge cases remain ASSUMED. During a read cycle, 3C8 returns the shared
//! address n; some RAMDACs prefetch and expose n+1. Components change palette
//! RAM immediately, so writing either address port partway through RGB leaves
//! already-written components changed. IBM Fig. 2-94 and pp. 2-104–2-105 say
//! an address write terminates the unfinished cycle and describe concatenating
//! three six-bit values, without establishing this partial-write behaviour.
//!
//! MSR bit 5 is reserved in IBM Fig. 2-21. For chained odd/even accesses in
//! maps 01–11 the compatibility policy injects zero into memory A0 when MSR bit 5
//! is one, and one when it is zero. Thus firmware values 67h/63h/66h select the
//! low page. This polarity is ASSUMED: FreeVGA Miscellaneous Output notes
//! describe the opposite polarity. Map 00 instead injects CPU A16, independent
//! of MSR bit 5. With GR6 bit 1 clear, the original CPU A0 is always preserved.
//!
//! Chain-4 placement is a DERIVATION from IBM Figs. 2-37 and 2-65, rather than
//! an explicit CPU-offset formula in the manual: after the low two CPU bits
//! select the plane, preserving A2–A15 and routing A14–A15 into memory A0–A1
//! gives the doubleword memory layout. CPU accesses use this layout whenever
//! chain-4 is selected; CRTC beam fetches and address outputs are not executed here.
//!
//! Byte I/O and adjacent little-endian word/dword I/O are accepted when every
//! constituent port belongs to the device (including index/data OUTW). MMIO
//! accepts widths 1–8; trailing unselected bytes float FF. Invalid accesses
//! have no effects or traces. Version-4 snapshots preserve address, row and pixel-panning latches, all hardware state
//! and virtual trace time; version 3 initializes display latches from live registers.
//! Trace policy, history and drops remain run-scoped.
//!
//! IBM VGA/XGA Technical Reference, General Registers, Video Subsystem Enable
//! (Fig. 2-27), reserves 3C3h. IBM Personal System/2 Models 50 and 60
//! Technical Reference, I/O Controllers, Video Subsystem, Fig. 4-40 (p. 4-63),
//! defines bit 0 as CPU I/O/memory enable and preserves access to 3C3h even
//! when POS 102h disables the VGA. This model retains that interface on ISA.
//! ISA setup follows Video Seven V7 VGA Technical Reference (June 1988),
//! Miscellaneous Register Definitions, ROMMAP/ALTVSE (IBM Display Adapter
//! compatibility): 46E8h bit 4 selects setup, bit 3 enables normal CPU decode;
//! 102h bit 0 is writable only during setup. Both are write-only (reads FFh).
//! Sparse 46E8h aliases are not decoded. ROM bank bits 2–0 are ignored:
//! only the linear 24 KiB ROM is mapped by the
//! board. All three enable gates affect CPU registers and RAM, preserving
//! stored display state and scanout. ASSUMED reset enables all gates so direct
//! register users need no firmware; adapter setup and subsystem disable remain
//! independent. ROM decode is board-owned and stays readable while disabled.

use device::clock::{ClockDomain, PIT, VGA_25, VGA_28};
use device::{AccessDir, Device, DeviceError, NextEvent, TraceEvent, VClock};

const NAME: &str = "vga";
const VERSION: u8 = 4;
/// First address of the claimed VGA MMIO aperture, including unselected maps.
pub const MEMORY_BASE: u64 = 0xA_0000;
/// Size of the claimed VGA aperture in bytes.
pub const MEMORY_LEN: usize = 0x2_0000;
/// Bytes in each of the four memory planes.
pub const PLANE_LEN: usize = 0x1_0000;
const SEQ_MASKS: [u8; 5] = [0x03, 0x3F, 0x0F, 0x3F, 0x0E];
const GC_MASKS: [u8; 9] = [0x0F, 0x0F, 0x0F, 0x1F, 0x03, 0x7B, 0x0F, 0x0F, 0xFF];
const AC_MASKS: [u8; 21] = [
    0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
    0xEF, 0xFF, 0x0F, 0x0F, 0x0F,
];
const HEADER_LEN: usize = 12 + 4 + 5 + 25 + 9 + 21 + 256 * 3 + 16;
const SNAPSHOT_LEN: usize = HEADER_LEN + 4 * PLANE_LEN;

/// Read-only IBM VGA register and colour-path state.
///
/// IBM VGA/XGA Technical Reference Manual, General Registers, Figs. 2-81–2-89
/// (attribute controller) and Fig. 2-94 (DAC). See
/// `backlog/docs/design/observation.md` §Diagnostic report of a running machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VgaDiagnostic {
    /// Miscellaneous Output register.
    pub misc_output: u8,
    /// Sequencer registers 00h–04h.
    pub sequencer: [u8; 5],
    /// CRT controller registers 00h–18h.
    pub crtc: [u8; 25],
    /// Graphics controller registers 00h–08h.
    pub graphics: [u8; 9],
    /// Attribute controller registers 00h–14h.
    pub attribute: [u8; 21],
    /// Attribute address byte, including Palette Address Source (bit 5).
    pub attribute_address: u8,
    /// Whether the next attribute write supplies data rather than an address.
    pub attribute_expects_data: bool,
    /// PEL mask applied to the final DAC index.
    pub pel_mask: u8,
    /// True in DAC read mode, false in write mode.
    pub dac_read_mode: bool,
    /// Shared DAC read/write address.
    pub dac_address: u8,
    /// Next DAC component (0 = red, 1 = green, 2 = blue).
    pub dac_component: u8,
    /// Raw DAC entries 0–15, with six-bit RGB components.
    pub dac: [[u8; 3]; 16],
    /// Attribute colours 0–15: final masked DAC index and its six-bit RGB.
    /// These describe the attribute path before packed eight-bit pixel pairing.
    pub shown: [(u8, [u8; 3]); 16],
}

/// Character clocks by dot clock (25.175 MHz, then 28.322 MHz) and by
/// Clocking Mode bits 3 and 0 (dot clock halved; eight-dot characters,
/// otherwise nine): IBM VGA/XGA Technical Reference, Miscellaneous Output
/// Register and Clocking Mode Register. Fixed at build time, so status reads
/// and display latches do not reduce the same ratios again.
const CHARACTER_CLOCKS: [[ClockDomain; 10]; 2] = {
    const fn by_mode(dot: ClockDomain) -> [ClockDomain; 10] {
        let mut clocks = [dot; 10];
        let mut mode = 0;
        while mode < 10 {
            let dots = if mode & 1 != 0 { 8 } else { 9 };
            let halved = if mode & 8 != 0 { 2 } else { 1 };
            clocks[mode] = dot.divided_by(dots * halved);
            mode += 1;
        }
        clocks
    }
    [by_mode(VGA_25), by_mode(VGA_28)]
};

struct RasterTiming {
    characters: u128,
    horizontal: (u128, u128),
    vertical: (u128, u128),
    retrace: (u128, u128),
    character_clock: ClockDomain,
}

impl RasterTiming {
    fn edge_count(&self, line: u128) -> u128 {
        let period = self.horizontal.0 * self.vertical.0;
        let edge = (line % self.vertical.0) * self.horizontal.0;
        (self.characters + period - edge) / period
    }

    fn next_edge(&self, line: u128) -> Option<VClock> {
        let edge = (line % self.vertical.0) * self.horizontal.0;
        let next_character = self.edge_count(line) * self.horizontal.0 * self.vertical.0 + edge;
        self.character_clock.checked_edge_at(next_character)
    }
}

/// IBM VGA storage and CPU access pipeline behind [`Device`].
///
/// IBM VGA/XGA Technical Reference Manual, Figs. 2-69–2-80. No board
/// routing or rendering is implied by constructing this device.
pub struct Vga {
    planes: [Vec<u8>; 4],
    sequencer: [u8; 5],
    crtc: [u8; 25],
    graphics: [u8; 9],
    attribute: [u8; 21],
    palette: [[u8; 3]; 256],
    misc: u8,
    feature: u8,
    seq_index: u8,
    crtc_index: u8,
    gc_index: u8,
    ac_address: u8,
    ac_data: bool,
    pel_mask: u8,
    dac_address: u8,
    dac_component: u8,
    dac_state: u8,
    latches: [u8; 4],
    subsystem_enable: u8,
    adapter_control: u8,
    setup_enable: u8,
    display_start: u16,
    display_pan: u8,
    display_row: u8,
    now: VClock,
    trace: device::TraceBuffer,
}

impl Vga {
    /// Deterministic power-on: cleared graphics storage, CPU decode on, mono I/O.
    #[must_use]
    pub fn new() -> Self {
        Self {
            planes: std::array::from_fn(|_| vec![0; PLANE_LEN]),
            sequencer: [0; 5],
            crtc: [0; 25],
            graphics: [0; 9],
            attribute: [0; 21],
            palette: [[0; 3]; 256],
            misc: 0,
            feature: 0,
            seq_index: 0,
            crtc_index: 0,
            gc_index: 0,
            ac_address: 0,
            ac_data: false,
            pel_mask: 0,
            dac_address: 0,
            dac_component: 0,
            dac_state: 0,
            latches: [0; 4],
            subsystem_enable: 1,
            adapter_control: 8,
            setup_enable: 1,
            display_start: 0,
            display_pan: 0,
            display_row: 0,
            now: VClock::ZERO,
            trace: device::TraceBuffer::default(),
        }
    }

    /// Observe fields without guest accesses, latch changes or DAC advancement.
    /// IBM VGA/XGA Technical Reference Manual, Figs. 2-81–2-89 and 2-94.
    #[must_use]
    pub fn diagnostic_state(&self) -> VgaDiagnostic {
        VgaDiagnostic {
            misc_output: self.misc,
            sequencer: self.sequencer,
            crtc: self.crtc,
            graphics: self.graphics,
            attribute: self.attribute,
            attribute_address: self.ac_address,
            attribute_expects_data: self.ac_data,
            pel_mask: self.pel_mask,
            dac_read_mode: self.dac_state == 3,
            dac_address: self.dac_address,
            dac_component: self.dac_component,
            dac: std::array::from_fn(|index| self.palette[index]),
            shown: std::array::from_fn(|index| {
                let address = self.attribute_dac_index(index) & self.pel_mask;
                (address, self.palette_entry(address))
            }),
        }
    }

    /// Attribute colour path shared by text and planar graphics scanout.
    /// IBM VGA/XGA Technical Reference Manual, Figs. 2-84, 2-86 and 2-89:
    /// Color Plane Enable, P54S and Color Select precede the DAC PEL mask.
    fn attribute_dac_index(&self, index: usize) -> u8 {
        let a = &self.attribute;
        let entry = a[index & usize::from(a[0x12])];
        let low = if a[0x10] & 0x80 != 0 {
            (entry & 15) | ((a[0x14] & 3) << 4)
        } else {
            entry
        };
        low | ((a[0x14] & 12) << 4)
    }

    /// Plane storage in plane order, without changing CPU latches.
    #[must_use]
    pub fn planes(&self) -> &[Vec<u8>; 4] {
        &self.planes
    }
    /// Sequencer registers R0–R4.
    #[must_use]
    pub fn sequencer(&self) -> &[u8; 5] {
        &self.sequencer
    }
    /// CRT controller registers R00–R18.
    #[must_use]
    pub fn crtc(&self) -> &[u8; 25] {
        &self.crtc
    }
    /// Graphics controller registers R0–R8.
    #[must_use]
    pub fn graphics(&self) -> &[u8; 9] {
        &self.graphics
    }
    /// Attribute controller registers R00–R14.
    #[must_use]
    pub fn attribute(&self) -> &[u8; 21] {
        &self.attribute
    }
    /// Raw six-bit RGB DAC entries, unaffected by the PEL mask.
    #[must_use]
    pub fn palette(&self) -> &[[u8; 3]; 256] {
        &self.palette
    }
    /// RGB entry after the PEL mask has selected the displayed DAC address.
    #[must_use]
    pub fn palette_entry(&self, index: u8) -> [u8; 3] {
        self.palette[usize::from(index & self.pel_mask)]
    }
    /// Miscellaneous Output register, including clock and sync selections.
    #[must_use]
    pub fn misc_output(&self) -> u8 {
        self.misc
    }
    /// Attribute Palette Address Source bit (true selects normal display).
    #[must_use]
    pub fn palette_address_source(&self) -> bool {
        self.ac_address & 0x20 != 0
    }
    /// Stored PEL mask used for display palette lookup.
    #[must_use]
    pub fn pel_mask(&self) -> u8 {
        self.pel_mask
    }

    /// Stored Video Subsystem Enable (3C3h), without a guest port transaction.
    /// IBM PS/2 Models 50 and 60 Technical Reference, Video Subsystem, Fig. 4-40.
    #[must_use]
    pub fn subsystem_enable(&self) -> u8 {
        self.subsystem_enable
    }
    /// Latched write-only 46E8h state, including setup and normal-decode bits.
    /// Video Seven V7 VGA Technical Reference, Miscellaneous Register Definitions, ROMMAP.
    #[must_use]
    pub fn adapter_control(&self) -> u8 {
        self.adapter_control
    }
    /// Latched write-only 102h setup enable, without entering setup mode.
    /// Video Seven V7 VGA Technical Reference, Miscellaneous Register Definitions, ALTVSE.
    #[must_use]
    pub fn setup_enable(&self) -> u8 {
        self.setup_enable
    }

    fn colour(&self) -> bool {
        self.misc & 1 != 0
    }
    fn active_crtc(&self) -> u16 {
        if self.colour() {
            0x3D4
        } else {
            0x3B4
        }
    }
    fn active_status(&self) -> u16 {
        if self.colour() {
            0x3DA
        } else {
            0x3BA
        }
    }

    fn owns_port(port: u16) -> bool {
        matches!(port, 0x3B4..=0x3B5 | 0x3BA | 0x102 | 0x46E8 | 0x3C0..=0x3C3 | 0x3C4..=0x3CA | 0x3CC | 0x3CE..=0x3CF | 0x3D4..=0x3D5 | 0x3DA)
    }
    fn check_port(port: u16, size: u8) -> Result<(), DeviceError> {
        if !matches!(size, 1 | 2 | 4)
            || !(0..u16::from(size)).all(|i| Self::owns_port(port.wrapping_add(i)))
        {
            return Err(DeviceError::UnmappedRegister {
                kind: "port",
                offset: u64::from(port),
            });
        }
        Ok(())
    }
    fn check_mmio(addr: u64, size: u8) -> Result<(), DeviceError> {
        if !(1..=8).contains(&size)
            || !(MEMORY_BASE..MEMORY_BASE + MEMORY_LEN as u64).contains(&addr)
        {
            return Err(DeviceError::UnmappedRegister {
                kind: "mmio",
                offset: addr,
            });
        }
        Ok(())
    }

    fn cpu_enabled(&self) -> bool {
        self.subsystem_enable != 0 && self.setup_enable != 0 && self.adapter_control & 0x18 == 8
    }

    /// IBM VGA/XGA Technical Reference, Input Status 1 (Fig. 2-25),
    /// Clocking Mode, and CRT Controller Registers: Horizontal Total, Vertical
    /// Total/Overflow, Vertical Retrace Start/End. Integer dot clocks derive
    /// from virtual PIT time.
    /// Before a usable CRTC is programmed, an ASSUMED ~70 Hz polling raster
    /// (38 PIT clocks/line, 449 lines) permits firmware's initial sync probes.
    fn input_status_one(&self) -> u8 {
        let timing = self.raster_timing(self.now);
        let (htotal, hdisplay) = timing.horizontal;
        let (vtotal, vdisplay) = timing.vertical;
        let (start, width) = timing.retrace;
        let column = timing.characters % htotal;
        let line = timing.characters / htotal % vtotal;
        let retrace = (line + vtotal - start % vtotal) % vtotal < width;
        u8::from(column >= hdisplay || line >= vdisplay || retrace) | (u8::from(retrace) << 3)
    }

    /// Whether the start-address bundle and the pixel panning differ from
    /// what the display last latched at retrace (see [`Device::tick`]).
    fn pending_latches(&self) -> (bool, bool) {
        let address_pending = self.display_start
            != u16::from_be_bytes([self.crtc[0x0c], self.crtc[0x0d]])
            || self.display_row != self.crtc[8] & 0x7f;
        (address_pending, self.display_pan != self.attribute[0x13])
    }

    fn latches_pending(&self) -> bool {
        let (address, pan) = self.pending_latches();
        address || pan
    }

    // Status polling and the display latch must cross the same virtual raster.
    fn raster_timing(&self, now: VClock) -> RasterTiming {
        let c = &self.crtc;
        let htotal = u128::from(c[0]) + 5;
        let hdisplay = u128::from(c[1]) + 1;
        let vtotal =
            u128::from(c[6]) + (u128::from(c[7] & 1) << 8) + (u128::from(c[7] & 0x20) << 4) + 2;
        let vdisplay =
            u128::from(c[0x12]) + (u128::from(c[7] & 2) << 7) + (u128::from(c[7] & 0x40) << 3) + 1;
        if c[0] < 5 || htotal <= hdisplay || vtotal <= vdisplay {
            return RasterTiming {
                characters: PIT.edges_at(now),
                horizontal: (38, 30),
                vertical: (449, 400),
                retrace: (412, 2),
                character_clock: PIT,
            };
        }
        let character_clock = CHARACTER_CLOCKS[usize::from(self.misc & 0x0c == 4)]
            [usize::from(self.sequencer[1] & 9)];
        let start =
            u128::from(c[0x10]) + (u128::from(c[7] & 4) << 6) + (u128::from(c[7] & 0x80) << 2);
        let width = (u128::from(c[0x11] & 15) + 16 - (start & 15) - 1) % 16 + 1;
        RasterTiming {
            characters: character_clock.edges_at(now),
            horizontal: (htotal, hdisplay),
            vertical: (vtotal, vdisplay),
            retrace: (start, width),
            character_clock,
        }
    }

    /// IBM VGA/XGA Technical Reference Manual, Figs. 2-49, 2-50, 2-66,
    /// 2-84 and 2-87–2-88. ASSUMED: undefined panning values ignore high
    /// bits, and odd 256-colour values ignore bit zero. Nine-dot text maps
    /// 0–7 to 1–8 dots and 8 to zero; eight-dot text uses 0–7 directly.
    fn scroll(&self, text: bool) -> device::VgaScroll {
        let pixel_pan = if text && self.sequencer[1] & 1 == 0 {
            if self.display_pan & 8 != 0 {
                0
            } else {
                (self.display_pan & 7) + 1
            }
        } else if !text && self.graphics[5] & 0x40 != 0 {
            (self.display_pan & 7) / 2
        } else {
            self.display_pan & 7
        };
        device::VgaScroll {
            pixel_pan,
            // Extended Graphics ignores BP (IBM Fig. 2-49).
            byte_pan: if !text && self.graphics[5] & 0x40 != 0 {
                0
            } else {
                (self.display_row >> 5) & 3
            },
            preset_row: self.display_row & 31,
            split_line: (u16::from(self.crtc[0x18])
                | (u16::from(self.crtc[7] & 0x10) << 4)
                | (u16::from(self.crtc[9] & 0x40) << 3))
                + 1,
            split_pan_reset: self.attribute[0x10] & 0x20 != 0,
            double_scan: if self.crtc[9] & 0x80 != 0 { 2 } else { 1 },
        }
    }

    fn read_port_byte(&mut self, port: u16) -> u8 {
        if port == 0x3C3 {
            return self.subsystem_enable;
        }
        if !self.cpu_enabled() {
            return 0xFF;
        }
        if port == self.active_crtc() {
            return self.crtc_index;
        }
        if port == self.active_crtc() + 1 {
            return self
                .crtc
                .get(usize::from(self.crtc_index))
                .copied()
                .unwrap_or(0);
        }
        if port == self.active_status() {
            self.ac_data = false;
            return self.input_status_one();
        }
        match port {
            0x3C0 => self.ac_address,
            0x3C1 => self
                .attribute
                .get(usize::from(self.ac_address & 0x1F))
                .copied()
                .unwrap_or(0),
            // IBM VGA/XGA Technical Reference, Input Status Register 0,
            // Fig. 2-24: SS detects the display. ASSUMED colour-monitor load:
            // any DAC-0 output at least 20h clears the comparator.
            // Reserved bits and the unimplemented CRT interrupt read zero.
            0x3C2 => {
                if self.palette[0].iter().any(|component| *component >= 0x20) {
                    0
                } else {
                    0x10
                }
            }
            0x3C4 => self.seq_index,
            0x3C5 => self
                .sequencer
                .get(usize::from(self.seq_index))
                .copied()
                .unwrap_or(0),
            0x3C6 => self.pel_mask,
            0x3C7 => self.dac_state,
            0x3C8 => self.dac_address,
            0x3C9 => {
                let value =
                    self.palette[usize::from(self.dac_address)][usize::from(self.dac_component)];
                Self::advance_dac(&mut self.dac_address, &mut self.dac_component);
                value
            }
            0x3CA => self.feature,
            0x3CC => self.misc,
            0x3CE => self.gc_index,
            0x3CF => self
                .graphics
                .get(usize::from(self.gc_index))
                .copied()
                .unwrap_or(0),
            _ => 0xFF,
        }
    }
    fn advance_dac(index: &mut u8, component: &mut u8) {
        *component += 1;
        if *component == 3 {
            *component = 0;
            *index = index.wrapping_add(1);
        }
    }
    fn write_port_byte(&mut self, port: u16, value: u8) {
        match port {
            0x3C3 => {
                self.subsystem_enable = value & 1;
                return;
            }
            0x46E8 => {
                self.adapter_control = value & 0x18;
                return;
            }
            0x102 if self.adapter_control & 0x10 != 0 => {
                self.setup_enable = value & 1;
                return;
            }
            _ => {}
        }
        if !self.cpu_enabled() {
            return;
        }
        if port == self.active_crtc() {
            self.crtc_index = value & 0x1F;
            return;
        }
        if port == self.active_crtc() + 1 {
            let index = usize::from(self.crtc_index);
            if self.crtc[0x11] & 0x80 != 0 && index <= 7 {
                if index == 7 {
                    self.crtc[7] = (self.crtc[7] & !0x10) | (value & 0x10);
                }
            } else if let Some(reg) = self.crtc.get_mut(index) {
                *reg = value;
            }
            return;
        }
        if port == self.active_status() {
            self.feature = value;
            return;
        }
        match port {
            0x3C0 => {
                if self.ac_data {
                    let index = usize::from(self.ac_address & 0x1F);
                    if let Some(reg) = self.attribute.get_mut(index) {
                        *reg = value & AC_MASKS[index];
                    }
                } else {
                    self.ac_address = value & 0x3F;
                }
                self.ac_data = !self.ac_data;
            }
            0x3C2 => self.misc = value,
            0x3C4 => self.seq_index = value & 7,
            0x3C5 => {
                if let Some(reg) = self.sequencer.get_mut(usize::from(self.seq_index)) {
                    *reg = value & SEQ_MASKS[usize::from(self.seq_index)];
                }
            }
            0x3C6 => self.pel_mask = value,
            0x3C7 => {
                self.dac_address = value;
                self.dac_component = 0;
                self.dac_state = 3;
            }
            0x3C8 => {
                self.dac_address = value;
                self.dac_component = 0;
                self.dac_state = 0;
            }
            0x3C9 => {
                self.palette[usize::from(self.dac_address)][usize::from(self.dac_component)] =
                    value & 0x3F;
                Self::advance_dac(&mut self.dac_address, &mut self.dac_component);
            }
            0x3CE => self.gc_index = value & 0x0F,
            0x3CF => {
                if let Some(reg) = self.graphics.get_mut(usize::from(self.gc_index)) {
                    *reg = value & GC_MASKS[usize::from(self.gc_index)];
                }
            }
            _ => {}
        }
    }

    fn window_offset(&self, addr: u64) -> Option<usize> {
        let (base, len) = match (self.graphics[6] >> 2) & 3 {
            0 => (MEMORY_BASE, 0x20000),
            1 => (MEMORY_BASE, 0x10000),
            2 => (0xB0000, 0x8000),
            _ => (0xB8000, 0x8000),
        };
        (base..base + len)
            .contains(&addr)
            .then(|| (addr - base) as usize)
    }
    /// IBM VGA/XGA Technical Reference Manual, Fig. 2-77: GR6 controls A0
    /// replacement independently of the parity used to select maps. Callers
    /// apply the ASSUMED odd/even routing gate described in the coverage matrix.
    fn odd_even_offset(&self, address: usize) -> usize {
        if self.graphics[6] & 2 == 0 {
            return address;
        }
        let replacement = if self.graphics[6] & 0x0C == 0 {
            (address >> 16) & 1
        } else {
            usize::from(self.misc & 0x20 == 0)
        };
        (address & !1) | replacement
    }

    /// DERIVED from IBM VGA/XGA Technical Reference Manual, Figs. 2-37 and
    /// 2-65: retain doubleword memory placement when software unchains planes.
    fn chain_four_offset(address: usize) -> usize {
        (address & 0xFFFC) | ((address >> 14) & 3)
    }
    fn read_byte(&mut self, addr: u64) -> u8 {
        if !self.cpu_enabled() {
            return 0xFF;
        }
        let Some(address) = self.window_offset(addr) else {
            return 0xFF;
        };
        let (offset, plane) = if self.sequencer[4] & 8 != 0 {
            (Self::chain_four_offset(address), address & 3)
        } else if self.graphics[5] & 0x10 != 0 {
            (
                self.odd_even_offset(address),
                usize::from(self.graphics[4] & 2) | (address & 1),
            )
        } else {
            (address, usize::from(self.graphics[4] & 3))
        };
        for p in 0..4 {
            self.latches[p] = self.planes[p][offset & 0xFFFF];
        }
        if self.graphics[5] & 8 == 0 {
            return self.latches[plane];
        }
        let mut result = 0xFF;
        for p in 0..4 {
            if self.graphics[7] & (1 << p) != 0 {
                let compare = if self.graphics[2] & (1 << p) != 0 {
                    0xFF
                } else {
                    0
                };
                result &= !(self.latches[p] ^ compare);
            }
        }
        result
    }
    fn write_byte(&mut self, addr: u64, value: u8) {
        if !self.cpu_enabled() {
            return;
        }
        let Some(address) = self.window_offset(addr) else {
            return;
        };
        let (offset, select) = if self.sequencer[4] & 8 != 0 {
            (Self::chain_four_offset(address), 1 << (address & 3))
        } else if self.sequencer[4] & 4 == 0 {
            (
                self.odd_even_offset(address),
                if address & 1 == 0 { 5 } else { 10 },
            )
        } else {
            (address, 15)
        };
        let rotated = value.rotate_right(u32::from(self.graphics[3] & 7));
        let mode = self.graphics[5] & 3;
        for p in 0..4 {
            if self.sequencer[2] & select & (1 << p) == 0 {
                continue;
            }
            let latch = self.latches[p];
            let set_reset = if self.graphics[0] & (1 << p) != 0 {
                0xFF
            } else {
                0
            };
            let data = match mode {
                0 if self.graphics[1] & (1 << p) != 0 => set_reset,
                0 => rotated,
                1 => {
                    self.planes[p][offset & 0xFFFF] = latch;
                    continue;
                }
                2 => {
                    if value & (1 << p) != 0 {
                        0xFF
                    } else {
                        0
                    }
                }
                _ => set_reset,
            };
            let data = match (self.graphics[3] >> 3) & 3 {
                0 => data,
                1 => data & latch,
                2 => data | latch,
                _ => data ^ latch,
            };
            let mask = self.graphics[8] & if mode == 3 { rotated } else { 0xFF };
            self.planes[p][offset & 0xFFFF] = (data & mask) | (latch & !mask);
        }
    }
}

impl Default for Vga {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for Vga {
    /// IBM VGA/XGA Technical Reference Manual, Graphics Modes, Figs. 2-31,
    /// 2-50, 2-60–2-65, 2-75 and 2-84–2-89. Logical scanout excludes repeated
    /// dots/lines; unsupported serializers blank as recorded in the matrix.
    fn graphics_scanout(&self) -> Option<device::GraphicsScanout<'_>> {
        let c = &self.crtc;
        let a = &self.attribute;
        let text = if a[0x10] & 1 == 0 {
            // Preserve cell inspection before horizontal timing is configured;
            // the power-on nine-dot setting is not a programmed scrolling mode.
            if c[0] < 5 {
                return None;
            }
            let state = self.planar_text_scanout()?;
            let scroll = state.scroll;
            if scroll.pixel_pan == 0
                && scroll.byte_pan == 0
                && scroll.preset_row == 0
                && scroll.split_line >= state.scan_lines
            {
                return None;
            }
            Some(state)
        } else {
            None
        };
        let scroll = self.scroll(text.is_some());
        let packed = self.graphics[5] & 0x40 != 0;
        let wide_pel = a[0x10] & 0x40 != 0;
        let lines = u16::from(c[0x12]) | (u16::from(c[7] & 2) << 7) | (u16::from(c[7] & 0x40) << 3);
        let row_scan_height = (c[9] & 31) + 1;
        let row_scan_mask = (if c[0x17] & 1 == 0 { 0x2000 } else { 0 })
            | (if c[0x17] & 2 == 0 { 0x4000 } else { 0 });
        // CGA-compatible row scans address separate banks instead of repeating pixels.
        let scan_repeat = if scroll.preset_row != 0 || scroll.split_line <= lines {
            1
        } else {
            (if row_scan_mask == 0 {
                row_scan_height
            } else {
                1
            }) * scroll.double_scan
        };
        let repeat = (
            (if self.sequencer[1] & 8 != 0 { 2 } else { 1 }) * if wide_pel { 2 } else { 1 },
            scan_repeat,
        );
        let colours = std::array::from_fn(|index| self.attribute_dac_index(index));
        Some(device::GraphicsScanout {
            scroll,
            text,
            planes: std::array::from_fn(|p| self.planes[p].as_slice()),
            dimensions: (
                (u16::from(c[1]) + 1) * 8 / if wide_pel { 2 } else { 1 },
                (lines + 1) / u16::from(scan_repeat),
            ),
            repeat,
            row_scan_height,
            row_scan_mask,
            start: self.display_start,
            row_stride: u16::from(c[0x13]) * 2,
            address_shift: if c[0x14] & 0x40 != 0 {
                2
            } else {
                u8::from(c[0x17] & 0x40 == 0)
            },
            word_wrap_bit: if c[0x17] & 0x20 != 0 { 15 } else { 13 },
            packed,
            colours,
            palette: std::array::from_fn(|index| {
                self.palette_entry(index as u8).map(|v| (v << 2) | (v >> 4))
            }),
            enabled: self.sequencer[1] & 0x20 == 0
                && self.palette_address_source()
                && packed == wide_pel
                && (packed || self.graphics[5] & 0x20 == 0),
        })
    }

    /// IBM VGA/XGA Technical Reference Manual, Alphanumeric Modes:
    /// Figs. 2-31, 2-33–2-36 (clock/fonts); 2-41, 2-48, 2-50–2-56,
    /// 2-59–2-61, 2-64–2-65 (geometry/cursor/address wiring);
    /// 2-82–2-83, 2-84, 2-86, 2-89 and 2-94 (palette/attributes/DAC).
    /// Cursor start and underline use the ASSUMED zero-based convention
    /// documented with SeaVGABIOS evidence in the coverage matrix.
    fn planar_text_scanout(&self) -> Option<device::PlanarTextState<'_>> {
        let c = &self.crtc;
        let a = &self.attribute;
        let s = &self.sequencer;
        let font_offset =
            |low: u8, high: u8| usize::from(low) * 0x4000 + usize::from(high) * 0x2000;
        let font_offsets = [
            font_offset(s[3] & 3, (s[3] >> 4) & 1),
            font_offset((s[3] >> 2) & 3, (s[3] >> 5) & 1),
        ];
        let height = (c[9] & 31) + 1;
        let lines = u16::from(c[0x12]) | (u16::from(c[7] & 2) << 7) | (u16::from(c[7] & 0x40) << 3);
        let rows = (lines + 1) / u16::from(height) / if c[9] & 0x80 != 0 { 2 } else { 1 };
        let palette = std::array::from_fn(|index| {
            self.palette_entry(self.attribute_dac_index(index))
                .map(|value| (value << 2) | (value >> 4))
        });
        Some(device::PlanarTextState {
            scroll: self.scroll(true),
            scan_lines: lines + 1,
            planes: [&self.planes[0], &self.planes[1], &self.planes[2]],
            dimensions: (u16::from(c[1]) + 1, rows),
            start: self.display_start,
            cursor: u16::from_be_bytes([c[0x0E], c[0x0F]]),
            row_stride: u16::from(c[0x13]) * 2,
            address_shift: if c[0x14] & 0x40 != 0 {
                2
            } else {
                u8::from(c[0x17] & 0x40 == 0)
            },
            word_wrap_bit: if c[0x17] & 0x20 != 0 { 15 } else { 13 },
            font_offsets,
            dual_font: s[4] & 2 != 0 && font_offsets[0] != font_offsets[1],
            cell_size: (if s[1] & 1 != 0 { 8 } else { 9 }, height),
            horizontal_repeat: if s[1] & 8 != 0 { 2 } else { 1 },
            line_graphics: a[0x10] & 4 != 0,
            blink_enabled: a[0x10] & 8 != 0,
            cursor_shape: (c[0x0A] & 0x20 == 0, c[0x0A] & 31, c[0x0B] & 31),
            underline: c[0x14] & 31,
            palette,
            enabled: s[1] & 0x20 == 0 && self.palette_address_source() && a[0x10] & 1 == 0,
        })
    }

    fn name(&self) -> &'static str {
        NAME
    }
    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
        Self::check_port(port, size)?;
        let mut value = 0;
        for i in 0..size {
            value |= u32::from(self.read_port_byte(port + u16::from(i))) << (8 * i);
        }
        self.trace.push(TraceEvent::Port {
            at: self.now,
            port,
            size,
            dir: AccessDir::Read,
            value,
        });
        Ok(value)
    }
    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
        Self::check_port(port, size)?;
        for i in 0..size {
            self.write_port_byte(port + u16::from(i), (value >> (8 * i)) as u8);
        }
        self.trace.push(TraceEvent::Port {
            at: self.now,
            port,
            size,
            dir: AccessDir::Write,
            value,
        });
        Ok(())
    }
    fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        Self::check_mmio(addr, size)?;
        let mut value = 0;
        for i in 0..size {
            value |= u64::from(self.read_byte(addr + u64::from(i))) << (8 * i);
        }
        self.trace.push(TraceEvent::Mmio {
            at: self.now,
            addr,
            size,
            dir: AccessDir::Read,
            value,
        });
        Ok(value)
    }
    fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError> {
        Self::check_mmio(addr, size)?;
        for i in 0..size {
            self.write_byte(addr + u64::from(i), (value >> (8 * i)) as u8);
        }
        self.trace.push(TraceEvent::Mmio {
            at: self.now,
            addr,
            size,
            dir: AccessDir::Write,
            value,
        });
        Ok(())
    }
    /// IBM VGA/XGA Technical Reference Manual, Start Address High/Low:
    /// the start address latches at vertical retrace start. ASSUMED frame
    /// approximation: sample CRTC 08h with that address bundle.
    /// Michael Abrash, Graphics Programming Black Book, Chapter 30, The Split
    /// Screen and Horizontal Panning: An Example, programs panning after sync
    /// starts because its hardware effect begins on the next line. ASSUMED
    /// frame-level approximation: sample AC panning at vertical retrace end.
    fn tick(&mut self, now: VClock) {
        debug_assert!(now >= self.now, "device clock moved backwards");
        if now >= self.now && !self.latches_pending() {
            // Every latch already holds its register: crossing retrace would
            // store the same values, so the raster need not be located.
            self.now = now;
        } else if now >= self.now {
            let previous = self.raster_timing(self.now);
            let current = self.raster_timing(now);
            let start = current.retrace.0;
            let end = start + current.retrace.1;
            if current.edge_count(start) > previous.edge_count(start) {
                self.display_start = u16::from_be_bytes([self.crtc[0x0c], self.crtc[0x0d]]);
                self.display_row = self.crtc[8] & 0x7f;
            }
            if current.edge_count(end) > previous.edge_count(end) {
                self.display_pan = self.attribute[0x13];
            }
            self.now = now;
        }
    }
    fn next_event(&self, now: VClock) -> NextEvent {
        let (address_pending, pan_pending) = self.pending_latches();
        if !address_pending && !pan_pending {
            return NextEvent::Dormant;
        }
        // Both deadlines invert the status raster, so neither rounding nor a
        // retrace pulse wrapping across vertical total can mix the frame pair.
        let timing = self.raster_timing(now);
        [
            (address_pending, timing.retrace.0),
            (pan_pending, timing.retrace.0 + timing.retrace.1),
        ]
        .into_iter()
        .filter(|(pending, _)| *pending)
        .filter_map(|(_, line)| timing.next_edge(line))
        .min()
        .map_or(NextEvent::Dormant, NextEvent::At)
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn reset(&mut self) {
        let policy = self.trace.policy();
        *self = Self::new();
        self.trace.set_policy(policy);
    }
    fn save(&self) -> Vec<u8> {
        let mut blob = Vec::with_capacity(SNAPSHOT_LEN);
        blob.extend_from_slice(&[
            VERSION,
            self.misc,
            self.feature,
            self.seq_index,
            self.crtc_index,
            self.gc_index,
            self.ac_address,
            u8::from(self.ac_data),
            self.pel_mask,
            self.dac_address,
            self.dac_component,
            self.dac_state,
        ]);
        blob.extend_from_slice(&self.latches);
        blob.extend_from_slice(&self.sequencer);
        blob.extend_from_slice(&self.crtc);
        blob.extend_from_slice(&self.graphics);
        blob.extend_from_slice(&self.attribute);
        for rgb in self.palette {
            blob.extend_from_slice(&rgb);
        }
        blob.extend_from_slice(&[
            self.subsystem_enable,
            self.adapter_control,
            self.setup_enable,
            self.display_start as u8,
            (self.display_start >> 8) as u8,
            self.display_pan,
            self.display_row,
            0,
        ]);
        blob.extend_from_slice(&self.now.0.to_le_bytes());
        for plane in &self.planes {
            blob.extend_from_slice(plane);
        }
        blob
    }
    fn restore(&mut self, blob: &[u8]) -> Result<(), DeviceError> {
        let invalid = |reason| DeviceError::InvalidSnapshot {
            device: NAME,
            reason,
        };
        if blob.len() != SNAPSHOT_LEN {
            return Err(invalid("snapshot length mismatch"));
        }
        if !matches!(blob[0], 3 | VERSION) {
            return Err(invalid("unsupported snapshot version"));
        }
        if blob[3] > 7
            || blob[4] > 31
            || blob[5] > 15
            || blob[6] > 63
            || blob[7] > 1
            || blob[10] > 2
            || !matches!(blob[11], 0 | 3)
        {
            return Err(invalid("invalid register or access phase"));
        }
        fn take<const N: usize>(blob: &[u8], cursor: &mut usize) -> [u8; N] {
            let value = std::array::from_fn(|i| blob[*cursor + i]);
            *cursor += N;
            value
        }
        let mut candidate = Self::new();
        candidate.misc = blob[1];
        candidate.feature = blob[2];
        candidate.seq_index = blob[3];
        candidate.crtc_index = blob[4];
        candidate.gc_index = blob[5];
        candidate.ac_address = blob[6];
        candidate.ac_data = blob[7] != 0;
        candidate.pel_mask = blob[8];
        candidate.dac_address = blob[9];
        candidate.dac_component = blob[10];
        candidate.dac_state = blob[11];
        let mut cursor = 12;
        candidate.latches = take(blob, &mut cursor);
        candidate.sequencer = take(blob, &mut cursor);
        candidate.crtc = take(blob, &mut cursor);
        candidate.graphics = take(blob, &mut cursor);
        candidate.attribute = take(blob, &mut cursor);
        for rgb in &mut candidate.palette {
            *rgb = take(blob, &mut cursor);
        }
        let control: [u8; 8] = take(blob, &mut cursor);
        if control[0] > 1
            || control[1] & !0x18 != 0
            || control[2] > 1
            || (blob[0] == 3 && control[3..].iter().any(|v| *v != 0))
            || (blob[0] == VERSION && (control[5] > 15 || control[6] > 0x7f || control[7] != 0))
        {
            return Err(invalid("invalid subsystem enable or adapter setup state"));
        }
        candidate.subsystem_enable = control[0];
        candidate.adapter_control = control[1];
        candidate.setup_enable = control[2];
        (candidate.display_start, candidate.display_pan) = if blob[0] == 3 {
            (
                u16::from_be_bytes([candidate.crtc[0x0c], candidate.crtc[0x0d]]),
                candidate.attribute[0x13],
            )
        } else {
            (u16::from_le_bytes([control[3], control[4]]), control[5])
        };
        candidate.display_row = if blob[0] == 3 {
            candidate.crtc[8] & 0x7f
        } else {
            control[6]
        };
        candidate.now = VClock(u64::from_le_bytes(take(blob, &mut cursor)));
        if candidate
            .sequencer
            .iter()
            .zip(SEQ_MASKS)
            .any(|(v, m)| v & !m != 0)
            || candidate
                .graphics
                .iter()
                .zip(GC_MASKS)
                .any(|(v, m)| v & !m != 0)
            || candidate
                .attribute
                .iter()
                .zip(AC_MASKS)
                .any(|(v, m)| v & !m != 0)
            || candidate.palette.iter().flatten().any(|v| *v > 63)
        {
            return Err(invalid("invalid reserved register bits or DAC component"));
        }
        for plane in &mut candidate.planes {
            plane.copy_from_slice(&blob[cursor..cursor + PLANE_LEN]);
            cursor += PLANE_LEN;
        }
        std::mem::swap(&mut candidate.trace, &mut self.trace);
        *self = candidate;
        Ok(())
    }
    fn set_trace_policy(&mut self, policy: device::TracePolicy) {
        self.trace.set_policy(policy);
    }
    fn trace_dropped(&self) -> u64 {
        self.trace.dropped()
    }
    fn take_trace(&mut self) -> Vec<TraceEvent> {
        self.trace.take()
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn core::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod planar_tests;

#[cfg(test)]
mod graphics_tests;

#[cfg(test)]
mod clock_domain_regressions {
    use super::*;

    use testkit::MockBus;
    #[test]
    fn clock_domain_vga_fallback_and_selected_dot_clocks_keep_phase() {
        let mut bus = MockBus::new(Box::new(Vga::new()));
        for (misc, dots) in [(0, VGA_25), (4, VGA_28)] {
            bus.port_write(0x3c2, 1, misc).unwrap();
            let vga: &mut Vga = bus
                .device_mut()
                .as_any_mut()
                .unwrap()
                .downcast_mut()
                .unwrap();
            let fallback = vga.raster_timing(VClock::from_pit(123));
            assert_eq!(fallback.characters, 123);
            vga.crtc[0] = 95;
            vga.crtc[1] = 79;
            vga.crtc[6] = 255;
            vga.crtc[0x12] = 199;
            let now = VClock::from_pit(1_000_000_000);
            let timing = vga.raster_timing(now);
            let character = dots.divided_by(9);
            assert_eq!(timing.characters, character.edges_at(now));
            let next = timing.next_edge(0).unwrap();
            assert_eq!(
                next,
                character
                    .checked_edge_at(timing.edge_count(0) * timing.horizontal.0 * timing.vertical.0)
                    .unwrap()
            );
            vga.crtc[0] = 0;
        }
    }

    /// The prebuilt character clocks are the ones dividing the selected dot
    /// clock at run time would give, for every clock select and every
    /// Clocking Mode dot-width and halving combination.
    #[test]
    fn clock_domain_character_clock_table_matches_runtime_division() {
        let mut vga = Vga::new();
        vga.crtc[0] = 95;
        vga.crtc[1] = 79;
        vga.crtc[6] = 255;
        vga.crtc[0x12] = 199;
        for misc in [0x00, 0x04, 0x08, 0x0c] {
            for clocking in [0x00, 0x01, 0x08, 0x09, 0xf6, 0xff] {
                vga.misc = misc;
                vga.sequencer[1] = clocking;
                let dot = if misc & 0x0c == 4 { VGA_28 } else { VGA_25 };
                let dots =
                    if clocking & 1 != 0 { 8 } else { 9 } * if clocking & 8 != 0 { 2 } else { 1 };
                assert_eq!(
                    vga.raster_timing(VClock::from_pit(7)).character_clock,
                    dot.divided_by(dots),
                    "misc {misc:#04x} clocking mode {clocking:#04x}"
                );
            }
        }
    }
}
