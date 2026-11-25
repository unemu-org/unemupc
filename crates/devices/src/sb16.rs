// SPDX-License-Identifier: AGPL-3.0-or-later
//! Creative Sound Blaster 16 DSP, CT1745 mixer and YMF262 native FM output.
//!
//! Hardware source: Creative, Sound Blaster Series Hardware Programming Guide,
//! §2 DSP I/O Ports, Resetting the DSP, Acknowledging DSP Interrupts; §3 8-bit
//! DMA Mode and Auto-initialize DMA; §6 commands 10h, 14h, 1Ch, 40h, 41h,
//! 48h, Bxh/Cxh, D0h/D4h/D5h/D6h, D1h/D3h/D8h, D9h/DAh and E1h;
//! §4 CT1745 Mixer, Register Functions and Figure 4-3; Appendix A, SB16.
//! FM source: Yamaha YMF262 datasheet, §2 CPU Interface and Register Map.
//! See `backlog/docs/design/audio.md` §SB16 8-bit output, §SB16 16-bit output
//! and §CT1745.
//!
//! | Guide section | Coverage |
//! |---|---|
//! | §2 DSP I/O, reset, interrupt acknowledgement | Byte ports, prompt reset-edge reply, latched IRQ |
//! | §3/§6 byte/word DMA | Unsigned legacy mono, signed/unsigned Bxh/Cxh mono/stereo, single/auto-init |
//! | §6 rate and control | Exact rational periods, pause/resume, exit, identification |
//! | §6 speaker control | Status flag only, as specified for DSP 4.xx |
//! | §4 CT1745 | Register masks/defaults/compatibility aliases; master/voice/MIDI attenuation |
//! | §2 resource/status registers | Fixed IRQ/byte/word channel readback; separate latched IRQs |
//! | §4 other mixer controls | Stored only; no switches, gains, AGC or tone processing |
//! | YMF262 CPU interface, register map, timers | Native FM, shared address latch, aliases, status-only timer IRQ |
//! | §5 MIDI, §6 input/ADPCM | Not modelled |
//!
//! Creative's §2 Resetting the DSP recommends a 3 µs hold and describes typical
//! 100 µs initialization. ASSUMED compatibility policy: clear state on writing 1,
//! accept any following 1->0 edge, and expose AAh immediately (zero delay), even
//! when both writes share a PIT clock. Counted driver poll loops on fast boards
//! must not exhaust while instruction time still rounds to the same PIT clock.
//! Busy is asserted only while reset is held; other commands/replies execute
//! immediately without ISA/DSP execution latency. Command 41h is clamped to the
//! guide's 5000–45000 Hz range (§6 41h). Time constants remain unclamped,
//! including 255 (ASSUMED); the explicit-Hz limit applies only to 41h.
//! Reset defaults to 22050 Hz, unsigned mono and a 65536-byte legacy auto-init
//! block (ASSUMED).
//! ASSUMED: E0h returns the inverted argument and F2h immediately asserts the
//! 8-bit IRQ; these compatibility probes are absent from this guide. The read
//! reply queue retains at most 16 oldest bytes and drops excess replies. Empty
//! reply reads return FFh; unused status bits read zero (ASSUMED).
//! Unsupported commands consume their complete known argument payload without a
//! reply; unknown opcodes consume one byte (ASSUMED unsupported-command policy).
//! Mixer +4/+5 and word IRQ acknowledge +F are claimed; OPL ports alias the
//! onboard YMF262 (see §OPL3 in `backlog/docs/design/audio.md`).
//! ASSUMED: +F reads FFh after clearing only the word IRQ. Registers 80h/81h
//! ignore writes: resources come from the machine file and are fixed for the
//! run. Real SB16 software resource selection through these registers is not
//! modelled. Register 82h reflects
//! byte/word IRQs in bits 0/1; MPU-401 bit 2 is always zero.
//!
//! ASSUMED: the first frame is requested one period after the command; subsequent
//! frames use origin + ceil(index * period), never repeated rounded additions.
//! There is one frame of staging (one sample mono, two stereo), fetched exactly
//! at its playback boundary through the DMA seam. Bxh/Cxh FIFO enable is accepted
//! but does not change this depth. IRQ asserts when the block's last sample arrives;
//! the last frame lasts until the following boundary. A masked/exhausted DMA
//! channel produces silence for that frame and retries at the next boundary.
//! An odd stereo block ends with a silent right channel. Pause freezes phase,
//! including the remaining fractional period; resume shifts the origin by the
//! paused duration. Chained blocks retain the sample grid; a live rate change
//! holds the current frame until its old boundary, then starts the new rate.
//! The 48h legacy auto-init size is independent of 14h/Cxh transfer counts.
//! ASSUMED compatibility policy: a new DMA command while paused discards the
//! unfinished transfer, clears pause and starts a fresh block, including an
//! auto-init-to-single replacement. Its first fetch is one period later; only
//! unpaused transfers of the same width defer a replacement to a frame/block boundary.
//! ASSUMED: switching between byte and word DMA replaces the output immediately,
//! dropping any unfinished block and starting a new grid one period later; there
//! is only one DAC stream. Pending interrupts remain separately latched.
//!
//! PCM scaling is unsigned `(byte - 128) * 256`, signed `i8(byte) * 256`;
//! words decode little-endian (unsigned words are offset by 32768); mono
//! duplicates the result. Master and voice attenuation use constant Q24 gains,
//! with one final division toward zero. Levels 0–31 follow the guide's -62 to
//! 0 dB range in 2 dB steps; level zero has Q24 gain 13327, rather than mute.
//! Master, voice and MIDI reset to level 31 (0 dB), not the guide's level 24
//! (-14 dB per stage); see the mixer module for this compatibility deviation.
//! SB Pro writes map nibble n to (n << 1) | 1 (ASSUMED), retaining unity at Fh;
//! alias reads expose upper native volume bits. Unknown
//! indices read FFh and reserved bits read zero (ASSUMED). Other mixer controls
//! are stored only. The board PC speaker is never routed through this mixer.
//! A DSP reset preserves mixer settings; whole-device reset restores them
//! (ASSUMED separation of the DSP and mixer reset domains).
//! Snapshots use SB16-FM4; SB16-MV3 restores with reset OPL3; SB16-8V2
//! also restores with reset mixer and no word state.
//! Direct DAC holds its unsigned level until another
//! DAC write, DMA command or reset (ASSUMED analogue reconstruction). The output
//! observer is installed by assembly; hardware never depends on an audio consumer.

pub mod fm;
mod mixer;

use device::clock::{ClockDomain, Ratio, PIT};
use mixer::Mixer;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use device::{
    AccessDir, Device, DeviceError, DmaOp, DmaOutcome, DmaRequest, NextEvent, TraceBuffer,
    TraceEvent, TracePolicy, VClock,
};

/// Timestamped native output, consumed by assembly's device-lifetime RateSource.
/// Creative Sound Blaster Series Hardware Programming Guide, §3 PCM data format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// One frame held for this exact positive rational virtual-tick period.
    Frame {
        /// Absolute start of the native frame.
        at: VClock,
        /// Virtual-tick period numerator.
        numerator: u64,
        /// Virtual-tick period denominator.
        denominator: u64,
        /// Signed left/right levels.
        pcm: [i16; 2],
    },
    /// End the active signal; the gap before another frame is silence.
    Pause(VClock),
}

#[derive(Clone, Copy, Debug)]
struct Transfer {
    count: u32,
    auto: bool,
    mode: u8,
    word: bool,
}

#[derive(Clone, Debug)]
struct State {
    now: u64,
    reset_start: Option<u64>,
    reset_ready: Option<u64>,
    reply: VecDeque<u8>,
    command: u8,
    args: Vec<u8>,
    needed: u8,
    irq: bool,
    irq16: bool,
    word: bool,
    mixer: Mixer,
    speaker: bool,
    rate: u16,
    tc: Option<u8>,
    auto_block: u32,
    reload: u32,
    remaining: u32,
    active: bool,
    auto: bool,
    exit: bool,
    stereo: bool,
    signed: bool,
    paused: Option<u64>,
    origin: u64,
    index: u64,
    tail: bool,
    request: bool,
    inflight: bool,
    staging: Vec<u8>,
    level: [i16; 2],
    direct: bool,
    next_block: Option<Transfer>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            now: 0,
            reset_start: None,
            reset_ready: None,
            reply: VecDeque::new(),
            command: 0,
            args: Vec::new(),
            needed: 0,
            irq: false,
            irq16: false,
            word: false,
            mixer: Mixer::default(),
            speaker: false,
            rate: 22050,
            tc: None,
            auto_block: 65536,
            reload: 65536,
            remaining: 0,
            active: false,
            auto: false,
            exit: false,
            stereo: false,
            signed: false,
            paused: None,
            origin: 0,
            index: 1,
            tail: false,
            request: false,
            inflight: false,
            staging: Vec::new(),
            level: [0; 2],
            direct: false,
            next_block: None,
        }
    }
}

/// Optional ISA DSP with byte/word DMA inputs and a shared latched interrupt output.
/// Creative Sound Blaster Series Hardware Programming Guide, §2 DSP I/O Ports.
pub struct Sb16 {
    base: u16,
    dma8: u8,
    dma16: u8,
    irq: u8,
    state: State,
    fm: Rc<RefCell<fm::Fm>>,
    output: Option<Box<dyn FnMut(Output)>>,
    trace: TraceBuffer,
}

impl Sb16 {
    /// Build a DSP at the validated ISA base and byte-DMA channel.
    #[must_use]
    pub fn new(base: u16, dma8: u8) -> Self {
        Self {
            base,
            dma8,
            dma16: 5,
            irq: 5,
            state: State::default(),
            fm: Rc::new(RefCell::new(fm::Fm::default())),
            output: None,
            trace: TraceBuffer::default(),
        }
    }

    /// Select fixed resources (Creative Sound Blaster Series Hardware Programming
    /// Guide, §2 Configuring DMA and Interrupt Settings). Assembly validates them.
    #[must_use]
    pub fn with_resources(mut self, dma16: u8, irq: u8) -> Self {
        self.dma16 = dma16;
        self.irq = irq;
        self
    }

    /// Bind native audio once; no host clock or host audio API enters the DSP.
    pub fn observe_output(&mut self, output: impl FnMut(Output) + 'static) {
        self.output = Some(Box::new(output));
    }

    /// Shared native FM state; its lifetime matches the SB16 card.
    pub fn fm(&self) -> Rc<RefCell<fm::Fm>> {
        self.fm.clone()
    }

    fn fm_port(&self, port: u16) -> Option<u16> {
        match port {
            0x388..=0x38b => Some(port - 0x388),
            p if (self.base..=self.base + 3).contains(&p) => Some(p - self.base),
            p if (self.base + 8..=self.base + 9).contains(&p) => Some(p - self.base - 8),
            _ => None,
        }
    }

    fn emit(&mut self, event: Output) {
        if let Some(output) = &mut self.output {
            output(event);
        }
    }

    /// Native stereo-frame period in exact virtual ticks (numerator, denominator).
    /// Creative Sound Blaster Series Hardware Programming Guide, §6 40h/41h:
    /// time constants count interleaved bytes; explicit rates count frames.
    #[must_use]
    pub fn sample_period(&self) -> (u64, u64) {
        let microsecond = ClockDomain::from_hz(Ratio::new(1_000_000, 1)).period();
        if let Some(tc) = self.state.tc {
            (
                microsecond.numerator()
                    * u64::from(256 - u16::from(tc))
                    * if self.state.stereo { 2 } else { 1 },
                microsecond.denominator(),
            )
        } else {
            (
                microsecond.numerator() * 1_000_000,
                microsecond.denominator() * u64::from(self.state.rate),
            )
        }
    }

    fn boundary(&self) -> u64 {
        let (p, q) = self.sample_period();
        let delta = (u128::from(self.state.index) * u128::from(p)).div_ceil(u128::from(q));
        u64::try_from(u128::from(self.state.origin) + delta).unwrap_or(u64::MAX)
    }

    fn frame(&mut self) {
        let (numerator, denominator) = self.sample_period();
        self.emit(Output::Frame {
            at: VClock(self.state.now),
            numerator,
            denominator,
            pcm: self.state.mixer.apply(self.state.level),
        });
    }

    fn clear(&mut self) {
        let now = self.state.now;
        let mixer = self.state.mixer.clone();
        self.state = State {
            now,
            mixer,
            ..State::default()
        };
        self.emit(Output::Pause(VClock(now)));
    }

    fn start(&mut self, count: u32, auto: bool, mode: u8, word: bool) {
        // Unpaused playback drains the current frame before a chained block.
        // Auto-init also defers a single-cycle replacement until the block ends
        // (Creative Sound Blaster Series Hardware Programming Guide, §6 1Ch).
        if self.state.active
            && self.state.word == word
            && self.state.paused.is_none()
            && (self.state.tail || (self.state.auto && !auto))
        {
            self.state.next_block = Some(Transfer {
                count,
                auto,
                mode,
                word,
            });
            self.state.exit = true;
            return;
        }
        self.emit(Output::Pause(VClock(self.state.now)));
        let s = &mut self.state;
        s.word = word;
        s.reload = count;
        s.remaining = count;
        s.auto = auto;
        s.exit = false;
        s.stereo = mode & 0x20 != 0;
        s.signed = mode & 0x10 != 0;
        s.active = true;
        s.paused = None;
        s.origin = s.now;
        s.index = 1;
        s.tail = false;
        s.request = false;
        s.inflight = false;
        s.staging.clear();
        s.direct = false;
        s.level = [0; 2];
        s.next_block = None;
    }

    fn set_rate(&mut self, next: u64) {
        if self.state.active {
            let paused = self.state.paused.map_or(0, |at| self.state.now - at);
            if self.state.paused.is_some() {
                self.state.paused = Some(self.state.now);
            }
            // Finish the held frame on its old boundary. The new rational grid
            // starts there, so rate changes cannot truncate it or insert a gap.
            self.state.origin = next.saturating_add(paused);
            self.state.index = 0;
        }
    }

    fn hold_until_boundary(&mut self) {
        let remaining = self.boundary().saturating_sub(self.state.now);
        self.emit(Output::Frame {
            at: VClock(self.state.now),
            numerator: if remaining == 0 {
                PIT.ticks(1)
            } else {
                remaining
            },
            denominator: 1,
            pcm: self.state.mixer.apply(self.state.level),
        });
    }

    /// Argument bytes in the Creative Sound Blaster Series Hardware Programming
    /// Guide, §6 DSP Commands, including input, ADPCM, MIDI and word DMA commands.
    /// ASSUMED: argument lengths for 04h/05h/08h/0Eh/0Fh/F9h/FAh follow
    /// DOSBox-X's SB16 DSP command-length table. Consuming these unsupported
    /// payloads prevents them becoming output commands.
    fn argument_count(command: u8) -> u8 {
        match command {
            0x04 | 0x08 | 0x0F | 0x10 | 0x38 | 0x40 | 0xE0 | 0xE2 | 0xE4 | 0xF9 => 1,
            0x05 | 0x0E | 0x14..=0x17 | 0x24 | 0x41 | 0x42 | 0x48 | 0x74..=0x77 | 0x80 | 0xFA => 2,
            0xB0..=0xCF => 3,
            _ => 0,
        }
    }

    fn write(&mut self, byte: u8) {
        if self.state.reset_start.is_some() || self.state.reset_ready.is_some() {
            return;
        }
        if self.state.needed != 0 {
            self.state.args.push(byte);
            self.state.needed -= 1;
            if self.state.needed == 0 {
                self.execute();
            }
            return;
        }
        self.state.command = byte;
        self.state.args.clear();
        self.state.needed = Self::argument_count(byte);
        if self.state.needed == 0 {
            self.execute();
        }
    }

    fn execute(&mut self) {
        let args = &self.state.args;
        match self.state.command {
            0x10 => {
                let level = (i16::from(args[0]) - 128) * 256;
                self.state.active = false;
                self.state.origin = self.state.now;
                self.state.index = 1;
                self.state.paused = None;
                self.state.inflight = false;
                self.state.staging.clear();
                self.state.next_block = None;
                self.state.request = false;
                self.state.tail = false;
                self.state.direct = true;
                self.state.level = [level; 2];
                self.emit(Output::Frame {
                    at: VClock(self.state.now),
                    numerator: u64::MAX - self.state.now,
                    denominator: 1,
                    pcm: self.state.mixer.apply([level; 2]),
                });
            }
            0x14 => self.start(
                u32::from(u16::from_le_bytes([args[0], args[1]])) + 1,
                false,
                0,
                false,
            ),
            0x1C => self.start(self.state.auto_block, true, 0, false),
            0x40 => {
                let next = self.boundary();
                self.state.tc = Some(args[0]);
                self.set_rate(next);
            }
            0x41 => {
                let next = self.boundary();
                self.state.rate = u16::from_be_bytes([args[0], args[1]]).clamp(5000, 45000);
                self.state.tc = None;
                self.set_rate(next);
            }
            0x48 => self.state.auto_block = u32::from(u16::from_le_bytes([args[0], args[1]])) + 1,
            0xB0..=0xCF => {
                if self.state.command & 9 == 0 {
                    let word = self.state.command < 0xC0;
                    self.start(
                        (u32::from(u16::from_le_bytes([args[1], args[2]])) + 1)
                            * if word { 2 } else { 1 },
                        self.state.command & 4 != 0,
                        args[0],
                        word,
                    );
                }
            }
            // ASSUMED: pause/continue pairs act only on their named width;
            // Creative's guide lists separate commands, but some emulators alias them.
            0xD0 | 0xD5
                if self.state.active
                    && self.state.word == (self.state.command == 0xD5)
                    && self.state.paused.is_none() =>
            {
                self.state.paused = Some(self.state.now);
                self.emit(Output::Pause(VClock(self.state.now)));
            }
            0xD4 | 0xD6 if self.state.active && self.state.word == (self.state.command == 0xD6) => {
                if let Some(at) = self.state.paused.take() {
                    self.state.origin = self.state.origin.saturating_add(self.state.now - at);
                    self.hold_until_boundary();
                }
            }
            0xD1 => self.state.speaker = true,
            0xD3 => self.state.speaker = false,
            0xD8 => self
                .state
                .reply
                .push_back(if self.state.speaker { 0xFF } else { 0 }),
            // ASSUMED: exit commands are width-specific, like pause/continue.
            0xDA | 0xD9 if self.state.word == (self.state.command == 0xD9) => {
                self.state.exit = true;
                self.state.next_block = None;
            }
            0xE0 => self.state.reply.push_back(!args[0]),
            // Epson Product Support Bulletin S-0172, Software, reports DSP 4.05
            // on a physical SB16; Creative's guide identifies SB16 as 4.xx.
            0xE1 => self.state.reply.extend([4, 5]),
            0xF2 => self.state.irq = true,
            _ => {}
        }
        self.state.reply.truncate(16);
    }

    fn record(&mut self, port: u16, size: u8, dir: AccessDir, value: u32) {
        self.trace.push(TraceEvent::Port {
            at: VClock(self.state.now),
            port,
            size,
            dir,
            value,
        });
    }
}

impl Device for Sb16 {
    fn name(&self) -> &'static str {
        "sb16-dsp"
    }
    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
        self.fm.borrow_mut().advance(VClock(self.state.now));
        let value = if let Some(offset) = self.fm_port(port) {
            self.fm.borrow().read(offset)
        } else {
            match port.wrapping_sub(self.base) {
                4 => self.state.mixer.index,
                5 => match self.state.mixer.index {
                    0x80 => match self.irq {
                        2 => 1,
                        5 => 2,
                        7 => 4,
                        10 => 8,
                        _ => 0,
                    },
                    0x81 => (1 << self.dma8) | (1 << self.dma16),
                    0x82 => u8::from(self.state.irq) | (u8::from(self.state.irq16) << 1),
                    _ => self.state.mixer.read(),
                },
                0x0F => {
                    self.state.irq16 = false;
                    0xFF
                }
                0x0A => self.state.reply.pop_front().unwrap_or(0xFF),
                0x0C => {
                    if self.state.reset_start.is_some() || self.state.reset_ready.is_some() {
                        0x80
                    } else {
                        0
                    }
                }
                0x0E => {
                    self.state.irq = false;
                    if self.state.reply.is_empty() {
                        0
                    } else {
                        0x80
                    }
                }
                _ => 0xFF,
            }
        };
        self.record(port, size, AccessDir::Read, value.into());
        Ok(value.into())
    }
    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
        self.record(port, size, AccessDir::Write, value);
        self.fm.borrow_mut().advance(VClock(self.state.now));
        if let Some(offset) = self.fm_port(port) {
            self.fm.borrow_mut().write(offset, value as u8);
            return Ok(());
        }
        match port.wrapping_sub(self.base) {
            4 => self.state.mixer.index = value as u8,
            5 => {
                self.state.mixer.write(value as u8);
                self.fm.borrow_mut().volume(&self.state.mixer);
                if self.state.direct {
                    self.emit(Output::Frame {
                        at: VClock(self.state.now),
                        numerator: u64::MAX - self.state.now,
                        denominator: 1,
                        pcm: self.state.mixer.apply(self.state.level),
                    });
                } else if self.state.active && self.state.paused.is_none() {
                    self.hold_until_boundary();
                }
            }
            6 if value as u8 & 1 != 0 => {
                self.clear();
                self.state.reset_start = Some(self.state.now);
            }
            6 => {
                if self.state.reset_start.take().is_some() {
                    self.state.reset_ready = Some(self.state.now);
                    self.tick(VClock(self.state.now));
                }
            }
            0x0C => self.write(value as u8),
            _ => {}
        }
        Ok(())
    }
    fn mmio_read(&mut self, addr: u64, _: u8) -> Result<u64, DeviceError> {
        Err(DeviceError::UnmappedRegister {
            kind: "mmio",
            offset: addr,
        })
    }
    fn mmio_write(&mut self, addr: u64, _: u8, _: u64) -> Result<(), DeviceError> {
        Err(DeviceError::UnmappedRegister {
            kind: "mmio",
            offset: addr,
        })
    }
    fn tick(&mut self, now: VClock) {
        if now.0 < self.state.now {
            return;
        }
        self.state.now = now.0;
        if self.state.reset_ready.is_some_and(|at| at <= now.0) {
            self.state.reset_ready = None;
            self.state.reply.push_back(0xAA);
        }
        if self.state.active
            && self.state.paused.is_none()
            && !self.state.request
            && !self.state.inflight
            && self.boundary() <= now.0
        {
            if self.state.tail {
                self.state.tail = false;
                if let Some(transfer) = self.state.next_block.take() {
                    let old_period = self.sample_period();
                    self.state.remaining = transfer.count;
                    self.state.reload = transfer.count;
                    self.state.auto = transfer.auto;
                    self.state.word = transfer.word;
                    self.state.exit = false;
                    self.state.stereo = transfer.mode & 0x20 != 0;
                    self.state.signed = transfer.mode & 0x10 != 0;
                    if self.sample_period() != old_period {
                        self.state.origin = now.0;
                        self.state.index = 0;
                    }
                    self.state.request = true;
                } else {
                    self.state.active = false;
                    self.state.origin = now.0;
                    self.state.index = 1;
                    self.emit(Output::Pause(now));
                }
            } else {
                self.state.request = true;
            }
        }
    }
    fn next_event(&self, now: VClock) -> NextEvent {
        let mut at = self.state.reset_ready;
        if self.state.active
            && self.state.paused.is_none()
            && !self.state.request
            && !self.state.inflight
        {
            at = Some(at.map_or(self.boundary(), |at| at.min(self.boundary())));
        }
        at.map_or(NextEvent::Dormant, |at| {
            if at > now.0 {
                NextEvent::At(VClock(at))
            } else {
                // Retain the exhausted-time sentinel for the scheduler to reject.
                NextEvent::At(PIT.next_edge_after(now).unwrap_or(VClock(u64::MAX)))
            }
        })
    }
    fn irq_pending(&self) -> bool {
        self.state.irq || self.state.irq16
    }
    fn reset(&mut self) {
        self.clear();
        self.state.mixer = Mixer::default();
        self.fm.borrow_mut().reset(self.state.now);
        self.trace.clear();
    }
    /// Creative Sound Blaster Series Hardware Programming Guide, §3 DMA and
    /// §6 DSP commands: a paused transfer holds its request until continued.
    fn dma_request_pending(&self) -> bool {
        self.state.request && self.state.paused.is_none()
    }
    fn take_dma_request(&mut self) -> Option<DmaRequest> {
        if !self.state.request || self.state.paused.is_some() {
            return None;
        }
        self.state.request = false;
        self.state.inflight = true;
        Some(DmaRequest {
            channel: if self.state.word {
                self.dma16
            } else {
                self.dma8
            },
            op: DmaOp::FromMemory {
                len: ((if self.state.stereo { 2 } else { 1 })
                    * if self.state.word { 2 } else { 1 }
                    - self.state.staging.len())
                .min(self.state.remaining as usize),
            },
        })
    }
    fn dma_complete(&mut self, outcome: DmaOutcome) {
        if !self.state.inflight {
            return;
        }
        self.state.inflight = false;
        let DmaOutcome::FromMemory { bytes, .. } = outcome else {
            return;
        };
        let expected = ((if self.state.stereo { 2 } else { 1 })
            * if self.state.word { 2 } else { 1 }
            - self.state.staging.len())
        .min(self.state.remaining as usize);
        if bytes.is_empty() {
            self.state.level = [0; 2];
            self.state.staging.clear();
            self.frame();
            self.state.index = self.state.index.saturating_add(1);
            return;
        }
        let count = bytes.len().min(expected);
        self.state.staging.extend_from_slice(&bytes[..count]);
        self.state.remaining -= count as u32;
        let end = self.state.remaining == 0;
        if end {
            if self.state.word {
                self.state.irq16 = true;
            } else {
                self.state.irq = true;
            }
            if self.state.auto && !self.state.exit {
                self.state.remaining = self.state.reload;
            } else {
                self.state.tail = true;
            }
        }
        if self.state.staging.len()
            < (if self.state.stereo { 2 } else { 1 }) * if self.state.word { 2 } else { 1 }
            && !end
        {
            self.state.request = true;
            return;
        }
        let sample = |offset: usize| {
            if self.state.word {
                self.state.staging.get(offset..offset + 2).map_or(0, |b| {
                    let word = u16::from_le_bytes([b[0], b[1]]);
                    if self.state.signed {
                        word as i16
                    } else {
                        (word ^ 0x8000) as i16
                    }
                })
            } else {
                self.state.staging.get(offset).map_or(0, |&byte| {
                    if self.state.signed {
                        i16::from(byte as i8) * 256
                    } else {
                        (i16::from(byte) - 128) * 256
                    }
                })
            }
        };
        let left = sample(0);
        self.state.level = [
            left,
            if self.state.stereo {
                sample(if self.state.word { 2 } else { 1 })
            } else {
                left
            },
        ];
        self.frame();
        self.state.staging.clear();
        self.state.index = self.state.index.saturating_add(1);
    }
    fn save(&self) -> Vec<u8> {
        self.save_state()
    }
    fn restore(&mut self, bytes: &[u8]) -> Result<(), DeviceError> {
        self.restore_state(bytes)
    }
    fn set_trace_policy(&mut self, policy: TracePolicy) {
        self.trace.set_policy(policy);
    }
    fn trace_dropped(&self) -> u64 {
        self.trace.dropped()
    }
    fn take_trace(&mut self) -> Vec<TraceEvent> {
        self.trace.take()
    }
}

impl Sb16 {
    fn save_state(&self) -> Vec<u8> {
        let s = &self.state;
        let optional = |v: Option<u64>| v.unwrap_or(u64::MAX);
        let values = [
            u64::from(self.base),
            u64::from(self.dma8),
            s.now,
            optional(s.reset_start),
            optional(s.reset_ready),
            u64::from(s.command),
            u64::from(s.needed),
            u64::from(s.irq),
            u64::from(s.speaker),
            u64::from(s.rate),
            s.tc.map_or(256, u64::from),
            u64::from(s.auto_block),
            u64::from(s.remaining),
            u64::from(s.active),
            u64::from(s.auto),
            u64::from(s.exit),
            u64::from(s.stereo),
            u64::from(s.signed),
            optional(s.paused),
            s.origin,
            s.index,
            u64::from(s.tail),
            u64::from(s.request),
            u64::from(s.inflight),
            u64::from(s.level[0] as u16),
            u64::from(s.level[1] as u16),
            u64::from(s.direct),
            s.next_block.map_or(0, |transfer| u64::from(transfer.count)),
            s.next_block.map_or(0, |transfer| u64::from(transfer.mode)),
            u64::from(s.reload),
            s.next_block.map_or(0, |transfer| u64::from(transfer.auto)),
        ];
        let mut bytes = b"SB16-FM4".to_vec();
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
        for values in [
            s.reply.iter().copied().collect::<Vec<_>>(),
            s.args.clone(),
            s.staging.clone(),
        ] {
            bytes.push(values.len() as u8);
            bytes.extend(values);
        }
        bytes.extend([
            self.dma16,
            self.irq,
            u8::from(s.word),
            u8::from(s.irq16),
            s.next_block.map_or(0, |t| u8::from(t.word)),
            s.mixer.index,
        ]);
        bytes.extend(s.mixer.registers);
        self.fm.borrow_mut().advance(VClock(s.now));
        bytes.extend(self.fm.borrow().save());
        bytes
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceError> {
        let bad = || DeviceError::InvalidSnapshot {
            device: "sb16-dsp",
            reason: "invalid SB16 DSP/mixer state",
        };
        let legacy = bytes.get(..8) == Some(b"SB16-8V2");
        let fm_version = bytes.get(..8) == Some(b"SB16-FM4");
        if !legacy && !fm_version && bytes.get(..8) != Some(b"SB16-MV3") {
            return Err(bad());
        }
        let mut rest = &bytes[8..];
        let mut values = [0u64; 31];
        for value in &mut values {
            let raw: [u8; 8] = rest
                .get(..8)
                .ok_or_else(bad)?
                .try_into()
                .map_err(|_| bad())?;
            *value = u64::from_le_bytes(raw);
            rest = &rest[8..];
        }
        let v = values;
        let optional = |value| if value == u64::MAX { None } else { Some(value) };
        let mut lists = Vec::new();
        for limit in [16, 3, if legacy { 1 } else { 3 }] {
            let len = usize::from(*rest.first().ok_or_else(bad)?);
            rest = &rest[1..];
            if len > limit {
                return Err(bad());
            }
            lists.push(rest.get(..len).ok_or_else(bad)?.to_vec());
            rest = &rest[len..];
        }
        let mut mixer = Mixer::default();
        let (word, irq16, queued_word) = if legacy {
            (false, false, false)
        } else {
            if (if fm_version {
                rest.len() < 30
            } else {
                rest.len() != 30
            }) || rest[0] != self.dma16
                || rest[1] != self.irq
                || rest[2..5].iter().any(|&v| v > 1)
            {
                return Err(bad());
            }
            mixer.index = rest[5];
            mixer.registers.copy_from_slice(&rest[6..30]);
            if mixer
                .registers
                .iter()
                .enumerate()
                .any(|(i, &v)| v & !Mixer::mask(i as u8 + 0x30) != 0)
            {
                return Err(bad());
            }
            let result = (rest[2] != 0, rest[3] != 0, rest[4] != 0);
            rest = &rest[30..];
            result
        };
        let fm = if fm_version {
            let fm = fm::Fm::decode(rest, mixer.clone(), v[2]).ok_or_else(bad)?;
            rest = &[];
            fm
        } else {
            let mut fm = fm::Fm::default();
            fm.reset(v[2]);
            fm.volume(&mixer);
            fm
        };
        let maximum = if word { 131072 } else { 65536 };
        if !rest.is_empty()
            || v[0] != u64::from(self.base)
            || v[1] != u64::from(self.dma8)
            || v[5] > 255
            || v[6] > 3
            || !(5000..=45000).contains(&v[9])
            || v[10] > 256
            || !(1..=65536).contains(&v[11])
            || v[12] > maximum
            || v[12] > v[29]
            || (v[19] > v[2] && (v[13] == 0 || v[20] != 0))
            || (v[20] == 0 && v[13] == 0)
            || !(1..=maximum).contains(&v[29])
            || v[30] > 1
            || (v[30] != 0 && v[27] == 0)
            || v[27] > if queued_word { 131072 } else { 65536 }
            || v[28] > 255
            || (v[27] != 0 && (v[13] == 0 || (v[21] == 0 && (v[14] == 0 || v[15] == 0))))
            || v[24] > 65535
            || v[25] > 65535
            || [7, 8, 13, 14, 15, 16, 17, 21, 22, 23, 26]
                .iter()
                .any(|&i| v[i] > 1)
            || [3, 18]
                .iter()
                .any(|&i| optional(v[i]).is_some_and(|at| at > v[2]))
            || v[4] != u64::MAX
            || (v[13] != 0 && v[12] == 0 && v[21] == 0)
            || (v[22] != 0 && v[23] != 0)
        {
            return Err(bad());
        }
        let expected_args = usize::from(Self::argument_count(v[5] as u8));
        if lists[1].len() + v[6] as usize != expected_args
            || (v[13] == 0 && (v[21] != 0 || v[22] != 0 || v[23] != 0 || v[18] != u64::MAX))
            || lists[2].len() >= (if v[16] != 0 { 2 } else { 1 }) * if word { 2 } else { 1 }
            || (word && (v[12] % 2 != 0 || v[29] % 2 != 0))
            || (queued_word && (v[27] == 0 || v[27] % 2 != 0))
        {
            return Err(bad());
        }
        let s = State {
            now: v[2],
            reset_start: optional(v[3]),
            reset_ready: optional(v[4]),
            reply: lists[0].iter().copied().collect(),
            command: v[5] as u8,
            args: lists[1].clone(),
            needed: v[6] as u8,
            irq: v[7] != 0,
            irq16,
            word,
            mixer,
            speaker: v[8] != 0,
            rate: v[9] as u16,
            tc: (v[10] < 256).then_some(v[10] as u8),
            auto_block: v[11] as u32,
            reload: v[29] as u32,
            remaining: v[12] as u32,
            active: v[13] != 0,
            auto: v[14] != 0,
            exit: v[15] != 0,
            stereo: v[16] != 0,
            signed: v[17] != 0,
            paused: optional(v[18]),
            origin: v[19],
            index: v[20],
            tail: v[21] != 0,
            request: v[22] != 0,
            inflight: v[23] != 0,
            staging: lists[2].clone(),
            level: [v[24] as u16 as i16, v[25] as u16 as i16],
            direct: v[26] != 0,
            next_block: (v[27] != 0).then_some(Transfer {
                count: v[27] as u32,
                auto: v[30] != 0,
                mode: v[28] as u8,
                word: queued_word,
            }),
        };
        self.state = s;
        self.fm.borrow_mut().restore(fm);
        // Presentation history is not hardware state. Restore resumes this source
        // at the restored clock; it never replays previously captured PCM.
        if self.state.direct {
            self.emit(Output::Frame {
                at: VClock(self.state.now),
                numerator: u64::MAX - self.state.now,
                denominator: 1,
                pcm: self.state.mixer.apply(self.state.level),
            });
        } else if self.state.active && self.state.paused.is_none() {
            self.hold_until_boundary();
        } else {
            self.emit(Output::Pause(VClock(self.state.now)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
