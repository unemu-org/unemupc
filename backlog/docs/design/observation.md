# Observing machine output

Design and migration record, 2026-11-09. Typed scanout and semantic byte sinks
are implemented. The inventory and prototype evidence below describe the earlier
implementation; the trace policy and its migration gate are now implemented.
BIOS/FreeDOS monochrome output now comes from an independent MDA card, as
specified in §The MDA card console below. Prompt polling, final captures and
presentation now share the assembly-owned observation facade (migration step 5).

## Audio observation

The adjacent `audio` crate supplies virtual-time PCM sinks and integer mixing.
`OutputConfig.audio_sink` defaults to `AudioSinkKind::None`; capture retains
48 kHz stereo i16 little-endian PCM hashes/counts, with optional bounded frame
retention. Assembly
settles devices and renders after successful execution/persistence checks;
direct machine callers use
`AssembledMachine::capture_audio`. Source-free machines allocate no mixer and
emit no audio. See [audio.md](audio.md) for exact PIT mapping, source history,
resampling, capture lifecycle and the documented nominal-clock discrepancy.

## Interrupt observations

Device IRQ queries observe the current electrical level without consuming a
request. The machine forwards rising and falling transitions at its existing
device deadlines and access/DMA settle points; it never turns a held device
output into a pulse. Intel 8259A datasheet, **Edge and Level Triggered Modes**:
a pending edge-triggered request requires the input to remain high until INTA,
even if masked. Thus reading an 8042 byte before unmasking IRQ1 withdraws the
request; unmasking while the byte remains unread exposes exactly one edge.
PIT IRQ0 is the sustained OUT0 level, including Mode-2 low pulses and Mode-3
half-periods. Status/count observations do not acknowledge it.

PIC observation logs record actual acknowledge calls and their CPU vectors.
An explicit INTA with no serviceable input returns the default IR7 vector and
does not set ISR7 (Intel 8259A datasheet, **Interrupt Sequence** and **Edge and
Level Triggered Modes**). After a slave input falls before acknowledge, master
IR2 is also withdrawn, so an explicit INTA returns master IR7 with both ISRs
unchanged. Normal machine polling and acknowledge remain atomic; this does not
model the physical INTR-to-INTA race interval. See
[scheduler interrupt contract](scheduler.md#pic-state-must-follow-cpu-acceptance).

## Window pacing

Only the host frontend reads `std::time::Instant`: the shell measures cadence
and repaint delay, and `Presenter::advance_with_warnings` wraps the bounded
advance path with a slice clock. The machine scheduler, headless runner, devices,
frame sinks and keyboard encoder never read host time. Tests inject elapsed host
durations and a simulated slice clock into `advance_with_slice_clock`; older
virtual-target tests use `advance_fully`, which supplies zero host cost. The
frontend neither ticks devices nor injects input during a scheduler call.
The pure policy lives in [`crates/machine-view/src/pacing.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/machine-view/src/pacing.rs); the conversion of a
tick deadline to a retired-instruction cap lives with the PC core, in
`crates/pc-core/src/pacing.rs`.

The board oscillator is exactly **OSC = 315/22 MHz**, four times the NTSC
315/88 MHz colour subcarrier. PIT input is OSC/12, or **13,125,000/11 Hz**:
[IBM Personal Computer AT Technical Reference (March 1984), System Board,
System Timers and Oscillator](https://www.minuszerodegrees.net/manuals/IBM_5170_Technical_Reference_1502243_MAR84.pdf).
`device::clock` defines the shared exact ratio, without crystal tolerance.
For elapsed nanoseconds `t`, the absolute target in virtual ticks is
`floor(t * 13,125,000 * TICKS_PER_PIT_CLOCK / 11,000,000,000)`.
Checked `u128` products saturate on exhaustion, including for `Duration::MAX`
at large tick scales. Only the absolute target is rounded, so observation
partitioning does not accumulate drift.
The default guest RTC uses the same ratio, with 11 phase units per virtual
tick at the current scale. Legacy 1- and 3-unit snapshots restore at their saved
ratio. Explicit RTC timing overrides count virtual ticks; see
[machine-config.md](machine-config.md#rtc-start-and-nvram-source).

Guest presentation deadlines follow a **60 Hz** epoch grid,
`ceil(frame * 1e9 / 60)` ns, rather than repeatedly adding a rounded period.
The shell advances a due deadline before requesting work, skips missed paints,
and schedules eframe repaints for the remaining time after host work completes.
Extra UI repaints do not advance or capture the guest again before its next
deadline. Display capture and texture upload happen once per eligible frame;
the platform's display cadence can further limit actual paints. A stopped
machine keeps its last screen and shows the selected speed without a stale ratio.
Stop returns the speed cell to `Speed 100%`.

View > Speed selects 100%, 200%, 400% or Unlimited. Every session starts at 100%.
On macOS, ⌘T holds Unlimited; releasing Command or T restores the selected
mode. Elsewhere Ctrl+Alt+T holds Unlimited until T is released. Focus loss or
opening a menu/dialog also ends either hold. The shortcut's letter make,
repeats and break stay host-owned. macOS Command state comes from winit modifier
changes and survives guest-focus resets; Command never generates guest scan
codes. Other platforms keep their existing Ctrl+Alt modifier balancing.

The host execution budget follows the interval between advances, clamped to
one 60 Hz period through 50 ms. The initial frame and the first frame after
re-anchoring use the 60 Hz period. Paced modes may use that full period to reach
their virtual target. Unlimited has
no virtual target and leaves 3 ms of the period for input/presentation work
(about 13.7 ms of execution at 60 Hz). Thus slower repaint cadences permit
larger slices within the 50 ms bound.
Host rendering costs still limit achievable speed. Budget checks occur between
chunks of at most 1 ms of virtual time; an in-flight chunk can overrun the host
budget. No host duration is supplied to a machine or device.

So what bounds one frame's host work, at every speed, is the slice budget plus
one chunk, and a chunk is bounded only if `Machine::run` returns within bounded
host work whatever the guest does. Three guests do not let virtual time advance
as a chunk expects, and each has its own bound: a guest stuck in exception
delivery retires nothing (the CPU backend's bound, about 7 ms per run in
release, then `CpuError::DeliveryStalled`; §Stuck in exception delivery); a
guest that resets again and again retires a few instructions per reset (the
machine stops the storm inside the run; §The machine resets again and
again); a guest that makes progress only
through exceptions retires one instruction or more per delivery, so its chunk
ends by virtual time as any other. MEASURED (`pc-session-tests`, on the host
clock, debug build): under Unlimited the open `fault_loop_rom` (an invalid
opcode whose #UD handler IRETs straight back) runs 30 frames with the longest
advance about 61 ms, the first frame's 50 ms period included; the open
`reset_storm_rom` stops within the first two frames at 100% and Unlimited, the
longest advance about 13 ms.

The window captures mixed audio at each successful run boundary into a bounded
cpal playback ring. Underruns play silence and overruns discard old frames;
audio never changes pacing targets or virtual time. Outside 100%, playback is
muted without time-stretch. Returning to 100% clears old PCM and interpolation
history and prefills fresh output. ⌘⇧M on macOS (Ctrl+Alt+M elsewhere)
toggles the user's mute preference,
including during speed mute; it cannot override speed mute. The status bar and
View menu distinguish the two reasons and expose the user preference. See
[audio.md §Host output](audio.md#host-output).

Occlusion, minimization, suspension, user pause and modal/menu pause grant no
guest work. On resume or uncovering, the host anchor is set to the current
elapsed duration and the current machine clock, with discarded debt reset.
Every speed change and hold transition also re-anchors. Hidden time is therefore
excluded rather than added to old debt, and switching to a slower mode grants
no catch-up burst. Platform visibility notifications still determine when the
frontend can recognize a hidden window.

For a paced mode, elapsed host time since the anchor is multiplied by its
integer ratio before conversion to virtual ticks. Each paint targets at most
**50 ms times that ratio** of guest time beyond the current clock. Given raw
relative wall target `W`, discarded ticks `D`, and relative machine clock `C`,
choose `T = min(W - D, C + virtual_ticks(50 ms * ratio))` (subtraction saturates).
Add `W - D - T` to `D` and add the anchor clock to `T`. This permanently discards
excess backlog; it does not rewind or fast-forward the machine. The host frame
budget can stop a slice before this virtual target on a slow host.

For configured CPU rate `A/B` steps per PIT clock, current virtual time `V`,
and an absolute chunk target ahead of machine clock `C`, the pure run-cap
function returns `V + ceil((T - C) * A / (B * TICKS_PER_PIT_CLOCK))`, saturating
at `u64::MAX`. The scheduler retains active and halted fractional phases, so the
presenter rechecks `machine.clock()` and computes another cap if required.
A final indivisible instruction or HLT clock can pass the target slightly;
there is no tick outside the scheduler. At exhausted virtual time the presenter
stops. Measured speed is the virtual-clock/host-time ratio over samples from the
last second, restarted on re-anchoring.

The keyboard delivery seam returns only bytes the 8042 actually accepted.
Tests record `(virtual_time_before_run, accepted_bytes)` at every nonempty
delivery; this is a test-owned log, without an unbounded production history or
new CLI flag. Replay runs to each recorded virtual time before delivering those
bytes, splitting work at the input boundary regardless of its normal chunk size.
Live keyboard arrival times vary with the host, but the resulting guest state
depends on the delivered bytes and virtual injection times, not the wall clock.

The CI synthetic guest polls the 8042 and writes each scan byte plus its polling
counter to MDA. Fake elapsed times include repeated timestamps, jitter and a
long stall. Replay with two different headless chunk sizes compares the complete
MDA frame (attributes/cursor included), PIT clock, virtual time, retirement count,
all eight GPRs, IP, CS, exposed FLAGS.IF, a hash of RAM `0..0xA0000` read through
CPU `read_mem`, and every device snapshot at rates 1/1, 5/2 and 2/3. The CPU seam
and adapter do not expose the remaining FLAGS bits. An HLT/IRQ0 fake-clock
regression at rates 5/2, 2/3 and 1/10 checks every frame reaches its target with
at most `ceil(B/A)` clocks of rounding, including frames requiring multiple runs. Shifting logged input by 17 virtual units must
change the frame; this guards against a byte-only oracle insensitive to timing.
The fixture-gated FreeDOS window DIR test also replays its delivery log through
a differently chunked headless run and compares the same state. Fixture handling
matches `crates/cli/tests/support/fixtures.rs`.

Blink remains steady. A later deterministic blink phase can derive from virtual
time; wall-clock blink and further hardware timing changes are outside this policy.

## Register-level VGA text scanout

`Device::planar_text_scanout` is the passive alternative to the existing
interleaved `text_scanout`: separate character, attribute and font planes,
register-derived geometry/addressing/cursor, and a resolved 16-colour RGB palette.
`Observation::capture_text` uses either capability for `TextScanout`, renders
only when requested, and distributes one owned `Frame::Text`. Neither capability
changes latches, traces, snapshots or virtual time.

`video::render_planar_text` preserves raw glyphs and attributes and extends
`TextGrid` with optional `TextAppearance`. It captures both 8 KiB font maps
(256 glyphs, 32 rows each), expands 8/9-dot rows including line-graphics dot nine,
and carries the palette and cursor/underline scan lines. This keeps guest fonts
out of a pixel-frame API: the existing window rasteriser draws them, while
terminal dumps remain character grids. MDA and `vga-text` have no appearance
extension and retain their existing cells, styles, cursor, dumps and raster output.
DAC components expand from six to eight bits by `(v << 2) | (v >> 4)`.
Blink remains a request with no wall-clock phase; the presenter leaves it visible.
A blanked VGA retains its text for inspection but presents black pixels.

READ: IBM VGA/XGA Technical Reference Manual (May 1992), Alphanumeric Modes,
Figs. 2-7–2-8; sequencer, Figs. 2-31 and 2-33–2-36; CRTC, Figs. 2-41,
2-48, 2-50–2-56, 2-59–2-61 and 2-64–2-65; Attribute Controller,
Figs. 2-82–2-83, 2-84, 2-86 and 2-89; DAC, Fig. 2-94.
The isolation fixture programs SeaVGABIOS `stdvgamodes.c` mode 03h using
`sequ_03`, `crtc_03`, `actl_01` and `grdc_01`, then loads synthetic fonts.
The sequencer's divided dot clock (SR01 bit 3) is carried as a horizontal dot
repeat in the appearance. The window doubles every dot, including dot nine,
so SeaVGABIOS modes 00h/01h (`sequ_01 = 08h`) occupy 720 pixels across
40 nine-dot cells at presenter scale one.

Attribute bit 3 remains in the foreground colour index when selecting between
two fonts. This is the maintainer-selected compatibility policy, supported by
[Linux vgacon, vgacon_do_font_op](https://github.com/torvalds/linux/blob/master/drivers/video/console/vgacon.c),
which disables intensity with Color Plane Enable 07h, and by
[DOSBox Staging, draw_text_line_from_dac_palette](https://github.com/dosbox-staging/dosbox-staging/blob/main/src/hardware/video/vga_draw.cpp)
and [86Box, svga_render_text_80](https://github.com/86Box/86Box/blob/master/src/video/vid_svga_render.c),
which keep the four foreground attribute bits. CPE masking remains in the
palette path and does not change the selected font.

ASSUMED deviation: cursor start and underline use the register values directly
as zero-based scan lines, despite the minus-one wording in Figs. 2-51/2-61.
SeaVGABIOS `crtc_07` sets CR09=4Fh and CR14=0Fh: adding one to underline would
put it outside the 16-line cell. Cursor start follows the same convention.
This compatibility choice has not been checked on a physical VGA.

Ordinary text captures render complete character rows using the CRTC offset
independently of displayed width. A frame with pixel/byte panning, preset row
scan or a visible split uses the existing indexed-pixel route instead: partial
cells and splits through glyphs cannot be represented by `TextGrid`. The passive
text capability remains available for cell inspection. ASSUMED compatibility
policy: text stays a cell grid before a usable horizontal total is programmed
(CRTC 00h below 5), including the deterministic reset state. The pixel renderer reads
the guest fonts, ninth-dot line graphics, palette, underline and cursor; blink
and cursor retain a steady visible phase. Cursor skew remains unmodelled.

### VGA CPU decode, monitor sense and polling

The register card now accepts 3C3h (read/write bit 0) and ISA adapter control
46E8h (write-only; bit 4 setup, bit 3 normal decode enable). Setup exposes
write-only 102h bit 0. Reads of 46E8h/102h return FFh; 3C3h returns only
bit 0, with reserved bits zero. The enable gates suppress
CPU register/MMIO access while retaining RAM, DAC, registers and the displayed
image. ROM reads remain board-owned and available. Reset enables decode as a
compatibility choice for direct register programs. Miscellaneous Output bit 0
continues to select CRTC/status/feature ports at 3Bxh or 3Dxh; inactive pairs
float FFh and do not reset the attribute flip-flop.

READ: [IBM VGA/XGA Technical Reference, VGA Function](https://www.ardent-tool.com/docs/pdf/ibm_vgaxga_trm2.pdf),
General Registers, Figs. 2-20–2-27: status 0 bit 4 is display switch sense,
status 1 bit 3 is vertical retrace, bit 0 is display disable, and disabling CPU
decode preserves scanout. The 1992 manual reserves 3C3h rather than specifying
its bit interface. The earlier IBM Personal System/2 Models 50 and 60 Technical Reference,
I/O Controllers, Video Subsystem, Fig. 4-40, p. 4-63, explicitly defines bit 0
as I/O/memory enable and exempts 3C3h access from POS sleep
([original section](https://www.ardent-tool.com/docs/pdf/ps2_50-60_techref_ch4b_system_board_io_controllers_video.pdf),
[searchable transcription](https://manualzz.com/doc/14534054/ibm-video-graphics-array-video-subsystem-manual)).
The model retains that bit interface on ISA. ISA setup is READ from the primary
compatible-device reference, Video Seven
*V7 VGA Technical Reference* (June 1988), Miscellaneous Register Definitions,
ROMMAP/ALTVSE, pp. 3-16–3-17 ([manual](https://www.minuszerodegrees.net/manuals/Video%20Seven/Video%20Seven%20-%20V7%20VGA%20chip%20-%20Technical%20reference%20manual%20-%20JUN88.pdf)).
An original 1987 Display Adapter register description was not available for
independent verification. Sparse 46E8h aliases are not decoded. ROM-bank bits 2–0 are ignored; only the supplied
linear 24 KiB mapping is claimed.

ASSUMED colour-monitor model: the loaded DAC entry zero controls status-0
switch sense. SS=0 if any six-bit component is at least 20h;
otherwise SS=1. This is a deterministic analogue-comparator approximation,
not an established IBM electrical threshold. It distinguishes low-intensity
and high-intensity probes without hardcoding exact RGB triples. The threshold
is an inference consistent with the colour-monitor probes documented by the
primary [PCjs implementation, `inStatus0`](https://github.com/jeffpar/pcjs/blob/master/machines/pcx86/modules/v2/video.js):
low components through 14h retain SS, while a 2Dh component clears it. This
is software compatibility evidence, not physical-card verification. Reserved status
bits and the unimplemented CRT interrupt return zero.

Status 1 now derives from virtual PIT time, never from read count. Programmed
CRTC horizontal/vertical totals, overflow, display ends and vertical retrace
start/end determine the polling raster. Integer 25.175/28.322 MHz dot clocks
are converted through the exact OSC/12 PIT domain (13,125,000/11 Hz); sequencer 8/9-dot
characters and dot-clock division affect horizontal periods. Retrace end wraps
in its four-bit counter. Repeated reads at the same clock return the same
status and still reset the attribute address/data flip-flop.

Before usable totals are programmed, the ASSUMED fallback is a fixed 449-line
frame, 38 PIT clocks per line, 400 active lines, horizontal disable after clock
29 and vertical retrace at lines 412–413 (~70 Hz). Unsupported clock selects
use 25.175 MHz. Beam phase is anchored at virtual clock zero; programming does
not restart it. Cycle-exact blanking, retrace IRQs and mid-frame effects remain
unmodelled. The virtual PIT clock derives from retired instructions and halted
virtual time. Capture/presentation remain passive and share no poll phase.
Version-4 snapshots retain all enable/setup state, display latches and virtual
time; version 3 migrates its live registers into the latches and version 2 is
rejected. Existing text-only MDA/`vga-text` behavior is unchanged.

Font upload remains the normal sequencer map-mask/GC CPU pipeline into plane 2
(IBM RAM Loadable Character Generator, p. 2-101); DAC indexing remains one
shared eight-bit address, a restarted RGB phase on either address write, state
00h/03h and increment/wrap after each triple (Fig. 2-94, pp. 2-104–2-105).
Mock-bus tests exercise both across enable changes. No presenter changes are
needed for the video BIOS. The user-ROM graphics measurement hashes dimensions,
repeat, row-major indices and RGB palette; its text hash covers visible raw
glyph/attribute pairs, never a screenshot or firmware text transcript.

**SUPERVISOR-MEASURED qualification at CPU `fe240a56`:** IBM 5170 BIOS
1985-06-10 plus the IBM Display Adapter VGA BIOS 1987 passes POST with the
video ROM on `ibm-5170`, INT 10h mode 03h (80×25, 16-line characters), and
mode 13h (the complete 320×200 byte-ramp pattern). The ignored
`user_rom_ibm_vga_bios_mode13_pattern` pins stop `Halted` at `0000:7C50`,
23,463,327 retired instructions, and both capture hashes listed in
[Video ROM placement](firmware-and-machine-classes.md#video-rom-placement).
Text sampling requires the boot program's nonuniform magic/stage signature
and CS=0 with IP inside its boot sector, excluding POST's repeated-word RAM
fills. Stage-1/end diagnostics read passive register getters and the same
`PlanarTextState` used by scanout; they print values only and never read guest
status ports or copy firmware screen text. Qualification covers this ROM pair
and these modes; the documented timing and rendering limits remain.


## Register-level VGA graphics scanout

The existing `TextScanout` route is a display request, including graphics on
`vga`. Keeping one route lets a subscribed host follow guest mode switches
without rebinding or inspecting a concrete card. `Device::graphics_scanout`
returns passive plane slices and decoded register state only when AC10 bit 0
selects graphics. Assembly prefers that state over text, renders once and fans
out `Frame::Pixels` to the same sinks. MDA and `vga-text` retain their text paths.
Console substring polling treats pixels as a missing prompt and keeps polling
until text returns, the guest halts or the cap is reached. CLI text dumps report
"display is in graphics mode" for pixel frames. The legacy BIOS result's
text-only screen fields use a blank grid for pixel captures; the observation sink still retains pixels.

`IndexedPixels` owns width, height, row-major DAC indices, dot/scan-line repeat
factors and an RGB888 palette. VGA captures use one byte per pixel plus 768
palette bytes, keeping exact palette indices testable. Generic indexed frames
may carry a shorter palette; every referenced index must exist. Every capture copies memory-derived pixels and palette, so
later writes, reset or restore cannot change a retained frame. Six-bit DAC
components expand using `(v << 2) | (v >> 4)`. The PEL mask is applied to each
palette address when copying the palette. Screen-off and PAS blanking produce
zero indices with a black palette, independent of programmed DAC entry zero.
The window validates the exact pixel count, palette-index range and nonzero
repeat factors before allocating a bounded buffer. Invalid frames return a
`RasterError`. It converts indices to `00RRGGBB`, applies the repeat factors and
then its integer scale. No window or host clock is needed to test presentation.

`Frame::Direct` carries `DirectPixels` for direct-colour modes (VESA BIOS
Extension 3.0, memory model 06h): width, height, repeat factors, a
`PixelFormat` (`Rgb555`, `Rgb565`, packed 24-bit `Rgb888` or `Xrgb8888`), a
byte stride and the raw scan lines, little-endian as the frame buffer stores
them. The buffer needs at least (height − 1) × stride + width × bytes per pixel
bytes; longer buffers are accepted and the bytes past the last visible pixel
ignored, so a card can hand over its scanout from the display start without
copying or padding the invisible tail of the last row. A frame with no width
or no height is empty. The frame keeps guest-format bytes so a card copies its
visible scan lines once per presented frame and does no colour work; the
single conversion to `00RRGGBB` is `DirectPixels::to_xrgb8888`, which every
presenter shares. A stride wider than a row (a virtual width) is honoured and
the bytes past each visible row are skipped. Five- and six-bit fields widen by
bit replication, `(v << 3) | (v >> 2)` and `(v << 2) | (v >> 4)`, so zero and
the field maximum map to 0 and 255; bit 15 of 15-bit pixels and the top byte of
32-bit pixels are ignored. The window rejects a stride shorter than a row
(`InvalidStride`), a buffer that ends before the last visible pixel
(`InvalidPixelCount`), zero repeat factors and oversized output before
converting any pixel, then applies the repeat factors and integer scale as for
indexed frames. The Bochs VBE card produces them while its DISPI interface is
enabled at 15, 16, 24 or 32 bpp, and an indexed frame through the DAC palette
at 8 bpp. Capture reads the visible bytes from the card's video memory in
guest RAM in one read from the display start (Y_OFFSET × line + X_OFFSET ×
bytes per pixel), with the virtual width as stride, and needs the CPU:
`AssembledMachine::capture_text` supplies it; a capture without one sees the
blank screen the VGA scanout reports in DISPI modes. MEASURED on one host:
reading a 1024×768×32 frame takes 0.07 ms (debug and release builds alike);
the shared `to_xrgb8888` conversion of that frame takes 0.4 ms in a release
build (20 ms in a debug build).

READ: IBM VGA/XGA Technical Reference Manual, Graphics Modes (modes D, E, 10,
12 and 13), Video Memory Organization, sequencer Fig. 2-31, CRTC Figs. 2-50,
2-60–2-65, GC Fig. 2-75, AC Figs. 2-84–2-89 and DAC Fig. 2-94.
Planar serializers combine the most-significant bit first from all four planes,
then apply Color Plane Enable, attribute palette, P54S and Color Select.
GC bit 6 takes precedence over its CGA shift bit 5. Both GC bit 6 and AC bit 6
must be set for 256-colour output.

DERIVED/ASSUMED: the 256-colour serializer takes successive bytes from planes
0–3, passing each four-bit sample through Color Plane Enable and the internal
palette's low nibble before pairing the samples. This nonidentity-palette model
is not a READ hardware fact. IBM's Color Select description (Fig. 2-89, p. 2-96)
says "the 8-bit attribute is the digital color value to the video DAC"; VGA
Programming Considerations (p. 2-97) says "should not modify the contents of the
Internal Palette registers". These establish normal mode-13h operation and the
programming restriction, rather than specifying arbitrary palette rewrites.
Color Select, P54S and palette high bits are ignored by the paired output.
The low-nibble interpretation remains unverified on physical hardware.

CRTC start and twice offset advance a wrapping 16-bit counter independently of
visible width. Byte/word/doubleword wiring follows Fig. 2-65, including word
address-wrap selection. DERIVED: its doubleword layout matches the existing
chain-4 CPU mapping, including address bits 14–15 folded into plane offsets
0–1. This is tested beyond the first 16 KiB, without assuming contiguous bytes
inside a plane. Logical dimensions come from horizontal display end times eight
(halved for paired 256-colour samples) and vertical display end with overflow
bits. When CMS0/SRC select counter address bits, logical height removes MSL+1
and DSC repetition. READ: when CR17 bit 0 is clear, row-scan bit 0 replaces
MA13; when bit 1 is clear, row-scan bit 1 replaces MA14, after byte/word/doubleword
wiring (Fig. 2-64). These scan lines address distinct banks, so logical height
removes only DSC. The counter row address advances by twice Offset after MSL+1 such lines.
SeaVGABIOS mode 06h (CR09=C1h, CR17=C2h) therefore has 640×200 logical pixels,
with odd lines displaced by 2000h and a physical line repeat of two.

Mode 13h uses CR09=41h (MSL=1, DSC=0) to produce 320×200 from 400 output lines;
0Dh/0Eh use DSC instead. Frames retain the removed scan-line repetition and a
horizontal repeat from dot clock /2 and paired 256-colour samples. At window
scale one, 0Dh and 13h expand 320×200 to 640×400, 0Eh expands 640×200 to 640×400,
and 12h remains 640×480. Text frames keep their existing raster geometry.
Logical indices remain compact; repeated dots and lines are generated only by
the presenter. No additional monitor aspect correction is applied. Fig. 2-31
specifies that Shift 4 and Shift Load behave as zero in Extended Graphics.

Frame scrolling and page flipping are modelled as described below.
Unmodelled: CGA-interleaved shift (GC bit 5 without bit 6), mismatched GC/AC
serializer settings, mid-frame changes other than the line-compare split,
and smooth-scrolling timing finer than a frame. Unsupported serializer
combinations blank deterministically; other deferred timing effects are ignored. Modes 04h/05h remain unmodelled and blank: GC shift-register mode
(bit 5 without bit 6) is not a supported serializer. Count-by-two/four variations
outside the listed BIOS tables and nonstandard CRTC timing are not claimed.

#### Frame scrolling, split screens and retrace latches

READ: [IBM VGA/XGA Technical Reference Manual (May 1992)](https://bitsavers.trailing-edge.com/pdf/ibm/pc/cards/IBM_VGA_XGA_Technical_Reference_Manual_May92.pdf),
VGA Function, Preset Row Scan (Fig. 2-49), Maximum Scan Line (Fig. 2-50),
Start Address High/Low (Figs. 2-53–2-54), Overflow (Fig. 2-48), Offset
(Fig. 2-60), Line Compare (Fig. 2-66), Attribute Mode Control (Fig. 2-84),
Horizontal PEL Panning and Image Shifting (Figs. 2-87–2-88), and Split Screen
Operation (p. 2-103).

The start address and CRTC 08h (byte panning / preset row scan) latch at the
start of vertical retrace. AC pixel panning latches at the end of vertical
retrace. `tick` detects both crossings in the same integer virtual raster used
by status polling, including a pulse wrapping across vertical total or a tick
that skips several frames. Pending changes schedule the appropriate edge even
without further VGA accesses. Reads and frame capture do not latch anything.

READ: Michael Abrash, Graphics Programming Black Book, Chapter 30, “Video Est
Omnis Divisa”, §The Split Screen and Horizontal Panning: An Example
([chapter text](https://raw.githubusercontent.com/jagregory/abrash-black-book/master/src/chapter-30.md)),
programs start address before vertical sync and panning after sync starts,
because panning takes effect on the next line. ASSUMED frame-level policy:
sample panning at retrace end rather than implementing per-line effects, and
sample CRTC 08h together with the start-address bundle. This keeps a start
written during display paired with panning written during retrace in the next
frame. A panning write during display waits for the next retrace end. IBM
explicitly establishes retrace-start timing for the start address. Other
registers are sampled at capture; mid-frame changes apart from the split remain
outside this model.

AC 13h shifts planar graphics and eight-dot text by 0–7 pixels. Nine-dot text
uses values 0–7 for shifts 1–8 and value 8 for no shift. In 256-colour output,
values 0, 2, 4, 6 shift by 0, 1, 2, 3 logical pixels. IBM leaves odd values and
other table entries undefined: ASSUMED policy ignores bit 0 in 256-colour
output, masks planar/eight-dot values to three bits, and treats nine-dot values
with bit 3 set as unshifted. This applies equally to chained and unchained RAM.
Pixels crossing a byte or cell boundary fetch the next byte/cell; they do not
wrap within the visible row.

CRTC 08h supplies byte panning (bits 5–6) and initial row scan (bits 0–4).
The row scan advances up to MSL and then clears, advancing the address by twice
Offset; DSC repeats each row scan twice. Extended Graphics ignores byte
panning, as Fig. 2-49 specifies. DERIVED/ASSUMED extension to the supported
single-shift modes: byte panning adds a plane-byte displacement after address
wiring in graphics, and a character-counter displacement in text. The manual
describes its use in multiple-shift modes rather than settling these cases.
Initial row scans above MSL use arithmetic continuation (ASSUMED invalid-mode
policy; normal scrolling uses values at or below MSL).

The 10-bit line compare combines CRTC 18h, 07h bit 4 and 09h bit 6 and counts
physical output scan lines, including double scans. The lower screen begins
after the compared line, restarts the memory counter at zero and clears the
preset row scan. AC 10h bit 5 clears the decoded pixel displacement below the
split; when clear the displacement continues. Byte panning remains active
below the split (ASSUMED; IBM explicitly specifies pixel panning only).
ASSUMED boundary convention: the compared line belongs to the upper screen;
the first lower line resets row scan and double-scan phase. IBM describes the
counter match and requires an even compare for double-scanned 200-line modes,
but does not completely specify the boundary phase or odd compares. Tests also
pin a deterministic result for odd boundaries. If preset row scan or a split
would break uniform scan-line repetition, graphics captures retain physical
lines (`repeat.1 = 1`); ordinary frames retain their compact logical geometry.

VGA snapshot version 4 stores the address, CRTC 08h and AC 13h display latches
in formerly reserved header bytes without changing the blob length. Version 3
restores those latches from the corresponding live registers. Invalid latch/reserved
bytes fail atomically. Trace history remains run-scoped.

Isolation tests program six SeaVGABIOS `stdvgamodes.c` mode tables through
ports and draw through CPU MMIO writes. They compare complete 12h and 13h
frames and programmed DAC palettes, covering write modes 0/2, plane masking,
start/stride and Fig. 2-65 addressing. Assembly additionally runs a synthetic
CPU store and checks owned capture, mode switching, sink delivery and passive
observation. Window tests compare the complete scaled host pixel buffer.

MEASURED mutation evidence: 21 independent changes in a disposable copy each
compiled and failed the intended assertion (exit 101); the unmodified copy
passed the six graphics tests, assembly route test and window pixel test.
Disabling the graphics capability reproduces the previous absent-capability
behaviour and fails all six device tests. Forcing assembly's previous text-only
selection fails the route test; suppressing window pixel conversion fails the
raster test. The original checkout without the new API was not tested.

| Mutated rule | Detecting test (short name) |
| --- | --- |
| Absent graphics capability | All six graphics isolation tests |
| Reversed planar plane order | `mode_twelve_exact` |
| Packed plane order; doubleword high address bits; chain-4 CPU high bits | `mode_thirteen_exact` |
| Plane enable; P54S; Color Select; PEL mask | `graphics_attribute_palette` |
| Maximum scan line; paired PEL width; GC serializer precedence | `mode_thirteen_exact` |
| Double scan; vertical overflow bit 9 | `planar_geometry` |
| Start address; row stride; word address wrap | `graphics_start_offset` |
| Screen-off; palette address source | `graphics_screen_off` |
| Assembly text-only selection | `register_vga_display_route_switches` |
| Window pixel conversion suppressed | `indexed_frame_palette` |

The first overflow mutation survived because BIOS modes stay below 512 lines.
The geometry test was strengthened with synthetic 992-line and 224-line
register settings to exercise both overflow bits; the repeated mutation failed.
The disposable build directory was removed after the mutation run.

MEASURED follow-up regressions: before correcting the implementation, mode 06h
failed its 640×200 geometry assertion (height was 100), prompt polling failed on
a pixel frame, and a malformed pixel count panicked. The added mode-13h
Color Select/P54S/high-palette-bit test already passed the low-nibble model;
removing the low sample's `& 15` now fails it, including palette entry 31h
(which distinguishes upper-bit leakage) and assertions with entry 3Fh.

All 16 follow-up mutations compiled and failed their intended test. The
unmodified disposable copy passed 9 graphics tests, 1 prompt-poll test,
1 CLI text-dump test and 6 raster tests; the copy and build directory were deleted.

| Mutated rule | Detecting test (short name) |
| --- | --- |
| CGA logical height; CMS0/MA13; SRC/MA14; clearing counter bits; substitution after word wiring | `mode_six_cga` |
| Mode-13h internal palette low-nibble mask | `mode_thirteen_ignores` |
| Pixel prompt reported as unavailable | `prompt_poll_waits_through` |
| Generic unavailable diagnostic for a graphics dump | `text_dump_reports` |
| Dot or scan-line repeat omitted from frame | `firmware_graphics_modes_retain` |
| Presenter ignores horizontal repeat | `graphics_repeat_factors` |
| Pixel-count validation omitted | `malformed_pixel_counts` |
| Palette-index, nonzero-repeat or scaled-width overflow validation omitted | `pixel_frames_validate` |
| Unmodelled CGA shift rendered as planar | `graphics_screen_off` |

## Host input

The window frontend queues physical winit key transitions independently of
observation subscribers. It encodes a transition into IBM PC scan code set 1
only when its first byte can be accepted at a run boundary. At each completed
run boundary, immediately before the next paced virtual-time run,
`HostKeyboard` attempts delivery through assembly's `try_inject_scancode` seam
and the existing `I8042::push_scancode` path. The side-effect-free
`I8042::host_input_ready` accessor requires an empty output buffer and staging
queue, an enabled interface, and no serial/IRQ hold, pending controller reply
or unfinished LED parameter exchange. No guest-visible port read is used. Backpressure leaves the remaining bytes in
the host FIFO for the next boundary, in order.
The controller's script staging queue itself is unbounded; this additional
conservative frontend backpressure never changes interactive golden scripts.
There is no injection inside a run and no host clock, timer or thread in
guest execution. Host arrival times are therefore quantised to run boundaries;
recorded bytes plus their injection boundaries determine a reproducible run.

[IBM Personal System/2 Hardware Interface Technical Reference — Common
Interfaces (October 1990), Keyboards (101- and 102-Key), Set 1 Scan-Code Tables,
pp. 27–30](https://www.ardent-tool.com/docs/pdf/ibm_hitrc11.pdf) specifies the
make/break codes and E0 extensions. Mapping uses US physical positions; the
guest handles Shift, Ctrl, Alt and locks. Both directions of an extended key
carry E0; breaks set bit 7. Figures 8–9 specify artificial Shift wrappers for
navigation and keypad Divide. Navigation with NumLock alone uses E0 2A before
make and E0 AA after break; with Shift alone it temporarily releases each held
Shift (E0 AA or E0 B6) and restores it after break (E0 2A or E0 36). Shift plus
NumLock uses the base sequence. Divide temporarily releases held Shift keys
regardless of NumLock. The BIOS consumes those wrappers as real Shift state,
keeping grey navigation keys independent of keypad lock translation.

NumLock follows the guest's keyboard LEDs, not host NumLock presses or OS lock
state. The frontend reads bit 1 of the latest Set/Reset Status Indicators
(Hex ED) parameter when it encodes each queued transition at a boundary; it
waits for an unfinished LED exchange to complete. Consequently Ctrl+NumLock
(Pause in the BIOS) does not spuriously toggle a host copy, and guest reboots
and LED updates keep navigation wrappers aligned with guest lock state.
The IBM reference's "Set/Reset Status Indicators (Hex ED)", "Reset (Hex FF)"
and "Basic Assurance Test" sections specify the parameter and LEDs switching
off at the end of BAT. The 8042 records LEDs and an outstanding parameter in
snapshot v4, rejects earlier snapshots without mutation, and clears both on
keyboard/device reset. Physical Shift state remains captured with each event.
On focus loss, all held keys receive breaks; non-Shift keys are released first so their wrappers
restore Shift before its final physical break. Unheld releases are ignored,
absorbing later synthetic releases from X11/Windows. Stopped or errored machines
ignore key events. On macOS, winit 0.30 does not deliver CapsLock key events, so
Caps Lock cannot reach the guest.

On macOS, ⌘⇧M controls host mute while an audio device is active. Press ⌘
first: Shift pressed before Command remains guest input with a balanced break;
Shift pressed while Command is held is suppressed. Ctrl and Alt remain immediate
guest input, including all Ctrl+Alt chords and Ctrl+Alt+Del. Other platforms
retain Ctrl+Alt+M, reserving the M pair while forwarding the modifiers.
macOS shortcut letters and punctuation follow the logical host layout; guest
scan codes always follow physical positions. See [commands](../commands.md)
for the complete Command table.
Letters, digits, punctuation, modifiers, locks, F1–F12, keypad and navigation
are supported. Pause/Break and PrintScreen are intentionally
unmapped because their special modifier-dependent sequences need a separate
protocol; other unsupported keys are ignored. A repeated winit pressed event
emits the plain base make again without a break. Artificial Shift wrappers
appear only on the first make and the final break, avoiding four or six queued
bytes on every repeat of a wrapped key. This follows the IBM set-1 typematic
base-code rule. Guest rate commands do not control the host repeat cadence.

The frontend keeps running beyond the DOS date and time prompts. Natural CPU
stop, errors and exhaustion of the virtual-time range retain the final frame.
Existing HLT idle policy and per-chunk caps remain unchanged. The event loop
uses `ControlFlow::Wait`, but requests another redraw while running; an active
DOS polling loop still consumes host CPU. Detecting idle from prompt text would
freeze a running guest, so further pacing remains separate work.

### Captured PS/2 mouse

When the machine has a PS/2 mouse (`board.mouse = true`) or a UART serial
mouse (`[[card]] kind = "uart-16550", mouse = "microsoft"`), a press inside the focused client area captures the pointer.
Both edges of that initial click are swallowed. Without an attached mouse,
clicks do not capture and the title and keyboard behavior are unchanged.
**⌘G on macOS (Ctrl+Alt+G elsewhere)** releases capture. The shortcut's G
make, repeats and break are host-only. macOS Ctrl+Alt+G remains guest input and
preserves capture; Ctrl and Alt retain their ordinary guest make/break handling.
A G already held by the guest is not stolen by a later modifier press.
Focus loss releases capture and held guest buttons.
Suspend acts like focus loss, releasing keys and the pointer; winit emits it only on mobile and web.
CPU stop/error, window close, application exit (including macOS Cmd+Q) and
frontend failure restore the host cursor.

The window tries winit `CursorGrabMode::Locked`, then `Confined`. Only a
successful grab hides the cursor and updates the shell's status hint to show
⌘G on macOS or Ctrl+Alt+G elsewhere as the release chord. If both fail, it reports the error
and leaves capture disabled. READ from the pinned winit 0.30.13 implementation:
macOS Locked disassociates mouse movement from the cursor position using
Core Graphics; Confined returns NotSupported. The cursor is hidden while the
window has input focus. This host behavior has not been exercised with a GUI.
On a platform supporting confinement only, movement remains relative and does
not depend on cursor coordinates or recentering.

Only `DeviceEvent::MouseMotion` supplies movement; absolute cursor events only
track client-area presence for the initial click. The fixed scale is **1 raw
host motion unit = 1 device count**, with host Y-down negated to device Y-up.
These units come from winit, so no promise is made about physical pixels or
millimetres, OS acceleration or DPI normalization. Finite deltas truncate toward
zero; fractional X/Y remainders survive run boundaries to preserve slow motion.
Non-finite values are ignored per axis. Conversion and coalesced sums saturate
at signed 32-bit limits, without clipping to the PS/2 packet range: the device
owns its -256/+255 counters and overflow bits. Release discards fractional
residue so it cannot leak into a later capture. Left/right/middle buttons map
to packet bits 0/1/2; wheel and extra buttons are ignored by the guest.
READ: IBM PS/2 Hardware Interface Technical Reference, Common Interfaces,
Pointing Device, Data Reporting and Commands (device counts, signs and bits).
The 1:1 host scale and capture controls are frontend policy. For the serial
route, `assembly::inject_mouse` converts the common Y-up counts to Microsoft
Y-down with saturating negation, keeps left/right, and ignores middle. The serial
mouse splits large deltas into -128..127 packets. Configuration rejects multiple
mouse targets, so capture always has one recipient. A UART with no mouse does
not enable capture. Serial motion while DTR or RTS is low is dropped; held host
buttons remain known for the next power-up. See
[machine-files.md §Serial mouse](machine-files.md#serial-mouse).

The session's pointer capture (`machine_view::pointer`) turns host motion
into counts and button changes into one contract event each; the PC core's
`PcMouse` (`unemu_pc_core::mouse`) coalesces motion between button
transitions. A transition starts a new zero-motion segment carrying the new
buttons; subsequent motion joins it. For example, motion A, right press,
motion B, right release becomes `(A, buttons=0), (B, buttons=2),
(0, buttons=0)`. A capture release (the release chord, a menu, Stop) keeps
queued integer motion and releases each held button as its own event, so
with several buttons held it appends one segment per released button, for
example `(0, buttons=1), (0, buttons=0)` with left and right held. Focus loss
of a running session releases everything at once (`InputEvent::ReleaseAll`):
one neutral-button segment, if necessary. Event handlers only update this
FIFO. Immediately before `run_bounded`, after keyboard delivery, `PcMachine`
delivers through `assembly::inject_mouse`; a zero-work paint
injects nothing. Before changing the last delivered button state, it checks
`AssembledMachine::mouse_input_ready`: the attached mouse must have no output
awaiting serial delivery. Otherwise that change and later segments stay queued
in order for later boundaries. In enabled stream mode, a motion packet followed
by a press/release therefore produces separate button-down and button-up packets,
even if the previous packet was unread when the click arrived. Motion with
unchanged buttons can still accumulate/coalesce while output is busy. The query
reads no ports; ordinary bus synchronization settles only the selected device
at the current virtual time before inspecting readiness. The mouse target is
resolved once from configuration. `mouse_device_index` uses a map lookup without
synchronizing the UART or 8042; machines without a mouse touch neither device at
window run boundaries. Neither query advances virtual time. Reporting-disabled, remote
and wrap modes retain the existing device semantics; guest commands may clear
accumulated motion and button transitions in these modes need not form packets.

Keyboard bytes wait for the 8042's input readiness, while mouse transitions wait
for their own peripheral output readiness. These queues are independent: a key
and a click arriving within one boundary can reach the guest in a different order
than on the host. Within each queue, delivery remains ordered. The event sequence,
guest progress and run boundaries determine injections without a clock in
conversion or guest execution.

Pure tests cover capture prerequisites, swallowed clicks, both modifier sides,
release, fractional/sign/large-delta conversion, button bits and segment ordering.
Headless presenter tests check the controller snapshot stays unchanged until a
run, the boundary log contains the expected segments, and a synthetic polling
guest reads exact ACK/PS/2 bytes after a read-data command. They also cover focus
loss, balanced keyboard releases, stopped input and machines without a mouse.
Stream-mode regressions check press/release packets after motion both within one
boundary and with an unread packet from an earlier boundary. Grab-failure retry
and release during the held capture click are tested without a host window.
No CLI, golden, CPU dependency or default machine profile changes are needed.

## CPU stop report

When the CPU core refuses an operation it does not support (x86jit's
`UnsupportedSystemContext` exit, surfaced as `CpuError::UnsupportedSystem`
with the operation's name), meets an instruction it does not lift, or returns
a fault outward, the error states only the operation and CS:IP. The core
refuses without a guest effect, so at that moment the CPU still holds the
refused instruction's state. `X86JitCpu::stop_report` reads it into a
`CpuStopReport` (`crates/cpu/src/stop_report.rs`): plain data, rendered as
plain text by its `Display`.

The report holds CS:IP and its linear address; in the machine profile the
MSW, CPL and A20, the CS and SS caches (selector, base, limit, attributes),
GDTR and IDTR (base, limit), LDTR and TR (selector and cache); SS:SP; the 16
bytes at CS:IP with the first instruction disassembled; and, when the
instruction is a far control transfer, an interrupt or a far return, what it
names:

| Instruction | Shown |
|---|---|
| far CALL/JMP `ptr16:16` (9A, EA) | the target selector's descriptor |
| far CALL/JMP `m16:16` (FF /3, FF /5) | the pointer's address and value, then the target's descriptor |
| INT n, INT 3, INTO | the IDT entry |
| RETF, RETF n | the words at SS:SP (IP, CS, then the outer SP and SS past the released bytes) and the return CS's descriptor, with the return SS's for a return to an outer ring |
| IRET | IP, CS, FLAGS, outer SP and SS at SS:SP, and the descriptors as for RETF |

A gate leads on to the descriptor it names (`gate target`, or `task gate
tss`). Each descriptor shows its 8 bytes and its decoded 80286 fields: type,
DPL, present, base and limit, or for a gate its selector, offset and word
count. When TR holds a 286 TSS, the report shows its back link and SS0:SP0,
SS1:SP1 and SS2:SP2. Descriptors are named only in protected mode. IRET with
NT set is a task return through that back link, so the frame shown at SS:SP
is then not the one the processor uses.

Bytes come from RAM and ROM through the CPU's own map, with the 80286's 24-bit
bus and the A20 gate applied; a trapped (MMIO) or unmapped address is shown as
`??` or "unavailable" and is never read, so a report has no device side effect.
An example, from the synthetic guest `assembly::test_support::refused_gate_call`
(a far CALL from ring 3 through a call gate that copies 31 words, with the
ring-3 stack and the TSS's ring-0 stack in the trapped text buffer: the core
cannot record that frame copy for a replay, so it refuses the call before
touching either stack):

```text
cpu stop report
stop     unsupported cpu system operation LegacyInstruction at 002b:7c2b
cs:ip    002b:7c2b (linear 007c2b)
mode     protected, cpl 3, msw fff1, a20 on
cs       002b base 000000 limit 0000ffff attr 00fb
ss       0033 base 0b8000 limit 00007fff attr 00f3
ss:sp    0033:0400
gdtr     base 007d00 limit 003f
idtr     base 000000 limit 03ff
ldtr     0000 base 000000 limit 00000000 attr 0000
tr       0018 base 007e00 limit 0000002b attr 0083
code     9a 00 00 23 00 f4 00 00 00 00 00 00 00 00 00 00
decoded  lcall $0x23,$0x0
form     far call ptr16:16 0023:0000
desc     target 0023 (gdt) 23 7c 08 00 1f e4 00 00: 286 call gate, selector 0008, offset 7c23, 31 words, dpl 3, present
desc     gate target 0008 (gdt) ff ff 00 00 00 9b 00 00: code segment type b, base 000000, limit ffff, dpl 0, present
tss      link 0000, ss0:sp0 0038:0f00, ss1:sp1 0030:5000, ss2:sp2 0030:4000
```

The PC core keeps the report of the refusal that failed it
(`PcMachine::cpu_stop_report`); the desktop shows it in the Machine error
dialog and saves it as `diagnostics/last-cpu-stop.txt` in the machine's
workspace (a folder that discarding or clearing an image's workspace
removes; a new session always writes its own), and
`unemu run --machine` prints it after its `stopped:` line (commands.md, Build
/ run / test).

### Stuck in exception delivery

A guest whose exception handler faults at its first instruction is
delivered the exception again and again, and no instruction retires in
between. A completed delivery is no step, so a run bounded by steps, and
virtual time, which counts them, would never end. The CPU core returns such
a run after 1024 consecutive deliveries with nothing retired, with steps
below the budget (x86jit's `Vcpu::run_steps` contract); in the machine
profile that is the only budget exit short of the budget.
`X86JitCpu::run_steps` runs on through 11 such returns in one call, 11,264
deliveries, as long as no instruction retires. On an 80286 every delivery
pushes a frame of at least 6 bytes, so within ⌈65,536 / 6⌉ = 10,923
deliveries the frames reach the bottom of a 16-bit stack. In real mode the
frame wraps inside the segment and faults only where a word sits at offset
FFFFh, which from an odd SP happens within the bound (a delivery at SP 1, 3
or 5); in protected mode the whole frame must fit, and when SP is not a
multiple of the frame size, or the stack's limit is below FFFFh, the next
frame overruns the stack. That fault escalates to a double fault and then to
shutdown (Intel 80286 Programmer's Reference Manual, §9.6.2 and Appendix B,
INT), and the run ends in Shutdown, which the AT board answers with a CPU
reset as for any shutdown. From an even SP in real mode, or one that is a
multiple of the frame size in protected mode, a 64 KiB stack wraps without a
fault forever, so a real-mode handler that faults in a loop from an even SP
stalls instead of shutting down; that stall, which neither retires nor shuts
down within the bound, becomes `CpuError::DeliveryStalled` and fails the
machine like a refusal. One call through the whole window
took about 7 ms in a release build (measured on the CPU backend's test), so
the desktop never waits longer than that inside one run. The report is
taken where the core stopped, at the handler's first instruction, so `code`
and `decoded` show the instruction that keeps faulting. The vector is not
known to the report. From the open guest
`assembly::test_support::delivery_stall_rom` (#GP from `MOV DS,AX` with
selector F8h beyond the GDT limit; the handler's first instruction is the
same `MOV DS,AX`; 8-byte frames from SP F000h wrap cleanly);
`delivery_overrun_rom` is the overrunning case, which shuts down and resets:

```text
cpu stop report
stop     the processor is stuck delivering exceptions: 11264 deliveries without an instruction, the last to 0008:0040
cs:ip    0008:0040 (linear 0f8040)
mode     protected, cpl 0, msw fff1, a20 on
cs       0008 base 0f8000 limit 0000ffff attr 009b
ss       0010 base 000000 limit 0000ffff attr 0093
ss:sp    0010:9000
gdtr     base 0f8110 limit 0017
idtr     base 0f8180 limit 006f
ldtr     0000 base 000000 limit 00000000 attr 0000
tr       0000 base 000000 limit 00000000 attr 0000
code     8e d8 f4 00 00 00 00 00 00 00 00 00 00 00 00 00
decoded  mov %ax,%ds
tss      tr not loaded
```

`unemu run --machine` prints `stopped: cpu fault: the processor is stuck
delivering exceptions: ...` and this report, and exits 1.

### The machine resets again and again

A reset takes no virtual time, and the AT board answers every processor
shutdown with one (IBM Personal Computer AT Technical Reference, System
Board, Logic Diagrams: shutdown decoding resets the processor alone). A
guest that shuts down again right after every reset makes a few steps of
virtual time per reset: a 1 ms desktop chunk at 3 steps per reset is about
245 resets, and `unemu run`'s default cap of 20,000,000 steps over 6 million. What never
returns is a loop that makes no virtual-time progress at all, as when the
processor shuts down before the first instruction after each reset
completes: a run capped by virtual time then resets for ever. The machine
ends both.

`machine::Machine` counts the reset requests, shutdowns and the board's
reset line alike. When `cpu::RESET_STORM_RESETS` (64) come in a row, each
fewer than `cpu::RESET_STORM_STEPS` (16) steps of virtual time after the one
before (the machine's start counts as the first reset), the 64th becomes
`CpuError::ResetStorm` instead of a reset, and the machine fails as for a
refusal. Virtual time is the run cap's own unit: completed steps, each REP
iteration one, plus halted time, so a REP MOVSW of 32 KiB counts 16,384,
not the one instruction it retires. The CPU still holds the state of that
request, so the report shows where it shut down. A guest reset still
happens inside `PcMachine::run`, which continues to the same deadline
(core-contract.md §3.5); the stop is what bounds a run's host work: at most
63 resets in a row close together, and every other reset follows at least
16 steps, which count toward the run's cap. Any bound of at least one step
would bound host work; 16 only diagnoses loops that make essentially no
progress. The same stop ends the headless loops (`assembly`'s
`run_session`, `unemu run`, the `pc-harness` runners), which run through
resets to their caps.

A guest that switches modes through resets moves virtual time far more.
MEASURED on the open AT protected-mode test with the open AT BIOS
(`at_pm_test_golden`, 36 resets), in steps between consecutive resets:

| Between | Steps |
|---|---|
| an INT 15h block move and its return to real mode, a reset (shortest) | 855 |
| the 32 rounds through the CMOS shutdown byte | 3,650 to 10,047 |
| power-on and POST's own 8042 reset; the boot and the first round | about 3.6 and 4.3 million |
| the storm ROM below: far jump, CLI, LIDT, then the shutdown | 3 |

A storm stop whose report shows about zero steps per reset (`64 resets
within` a number near 0) with `cs:ip` at the reset vector, F000:FFF0 or the
first instruction it jumps to, points to a problem in the emulator's reset
or first fetch, not in the guest: report it.

From the open guest `assembly::test_support::reset_storm_rom` (NASM listing
and SHA-256 pinned: from the reset vector CLI, LIDT with limit 0, INT 3,
which in real mode is beyond the IDT limit, a double fault and a shutdown;
three steps per reset, so 64 resets span 192):

```text
cpu stop report
stop     the machine resets again and again: 64 resets within 192 steps, the last shutdown at f800:0007
cs:ip    f800:0007 (linear 0f8007)
mode     real, cpl 0, msw fff0, a20 on
cs       f800 base 0f8000 limit 0000ffff attr 009b
ss       0000 base 000000 limit 0000ffff attr 0093
ss:sp    0000:7000
gdtr     base 000000 limit 0000
idtr     base 000000 limit 0000
ldtr     0000 base 000000 limit 00000000 attr 0000
tr       0000 base 000000 limit 00000000 attr 0000
code     cc f4 00 00 00 00 00 00 00 00 00 00 00 00 00 00
decoded  int3
form     int3, vector 03
tss      real mode
```

The desktop shows the stop in its machine error box; `unemu run --machine`
prints `stopped: cpu fault: the machine resets again and again: ...` and the
report, and exits 1. When the reset line rather than a shutdown made the last
request, the stop says `the last reset request at`: a guest that writes FEh to
port 64h right after every reset, which the session once let run on at
thousands of resets a frame, now stops with `64 resets within 191
steps, the last reset request at f800:0004` (`pc-session-tests`).

The reset itself costs about 0.5 µs (MEASURED, release, the storm ROM in a
loop: `reset_cpu` alone 1.98 million per second, 0.55 µs before its legacy
reset image was built once; the whole loop through `PcMachine::run` 878,000 to
898,000 resets per second, 761,000 to 862,000 before). Nine tenths of it is
x86jit's `Vcpu::reset_execution_state` clearing the vCPU's fast block lookup
(`Vcpu::fast_clear`: 2,048 entries of 24 bytes, 48 KiB written per reset).
A reset dominates a storm's profile because the storm does almost nothing
else, not because it is slow.

## Diagnostic report of a running machine

A guest can stop responding without any error: interrupts disabled around a
HLT or a loop, the keyboard interrupt masked in the 8259, the keyboard
interface disabled in the 8042. The machine then keeps its status, nothing
fails, and the CPU stop report above never exists. The diagnostic report is
the same kind of picture of a machine that still runs, taken by the host at a
completed run boundary: `AssembledMachine::diagnostic_report`
(`crates/assembly/src/diagnostic.rs`) returns a `MachineDiagnostic`, plain
data rendered as plain text by its `Display`; `PcMachine::diagnostic_report`
adds the machine's contract status (`ready`, `halted (waits for a reset)`,
`off` or `failed`), which says whether the machine can run, not whether a
front end holds it paused; the desktop appends two last lines, `desktop
running` or `desktop  paused`, and where the exception recording went
(`desktop  exceptions recorded to PATH`, or that Record exceptions is off).
Its lines come in a fixed order:

| Lines | Content |
|---|---|
| `retired`, `time`, `status`, `halted` | retired instructions and completed steps, virtual time and the device clock, the machine's status, whether the CPU sits in HLT and with which IF |
| `cs:ip` … `ss:sp`, `stack` | the CPU (`X86JitCpu::diagnostic_state`, `crates/cpu/src/diagnostic.rs`): CS:IP and its linear address; FLAGS with IF, TF, DF, IOPL and NT decoded (Intel 80286 Programmer's Reference Manual, §3.5.3 and §10.3); AX–SP; MSW with PE and CPL; CS, DS, ES and SS with selector, base, limit and access rights from their caches; GDTR, IDTR, LDTR and TR; whether an injected vector waits and whether an interrupt could be accepted; the HLT a halted CPU executed; the 16 bytes at CS:IP with the first instruction disassembled; the 16 words at SS:SP |
| `trail` | the code trail: CS:IP after the last run slices (below) |
| `delivery` | the last 64 entries of the exceptions and interrupts the guest took, oldest first, consecutive identical deliveries folded into one, with the total and the dropped count (§Exception and interrupt deliveries); `not recorded` when the log was not enabled |
| `pic` | master and slave 8259: IRR, ISR, IMR, vector base, ICW1 and ICW4 with AEOI, an unfinished initialization, the OCW3 read select, the input levels and special mask mode (OCW3 ESMM/SMM); poll mode is not modelled, and the report says so |
| `kbc` | the 8042: status register, command byte with its keyboard and aux enable and interrupt bits, output buffer and its source (keyboard, aux or controller), output and input ports with the inhibit switch, the parameter the next `60h` write is taken as, the last command written to `64h`, queued keyboard bytes, a waiting controller reply, the LED parameter wait, and the IRQ1 and IRQ12 levels |
| `pit` | each 8254 counter: programmed mode, access mode, BCD, count register, counting element, OUT and GATE, and a count not yet loaded or a latch waiting to be read |
| `a20` | the gate the CPU applies, the 8042 output port's A20 bit and port 92h bit 1 when the board has System Control Port A |
| `cmos` | the shutdown status byte at offset 0Fh, the NMI mask of port 70h bit 7 and the selected index |
| `vga` | when register-level VGA is fitted: Miscellaneous Output, sequencer 00h–04h, CRTC 00h–18h, graphics controller 00h–08h, attribute controller 00h–14h, attribute address with PAS and next address/data phase, PEL mask, DAC read/write mode with address and RGB component phase, raw DAC entries 0–15, and each attribute colour’s final DAC index and RGB |

Taking a report performs no guest port or memory-mapped access. Guest memory
is read from RAM and ROM through the CPU's own map, with the 80286's 24-bit
bus and the A20 gate, as for the stop report; a byte in a trapped window is
shown as `dv` (a memory-mapped device, not read) and an unmapped one as
`??`. In the machine profile an offset the segment's hidden cache does not
allow (past the limit of an expand-up segment, at or below the limit of an
expand-down data segment, any offset of an unusable cache) shows as `--` and
is not read, so the report never presents memory the guest cannot reach
through CS or SS (Intel 80286 Programmer's Reference Manual, §6.3 and §7.3).
The PICs are read from their snapshot images, restored into a scratch
chip. The 8042, PIT, RTC and register-level VGA are borrowed through the bus, which first
brings each to the bus clock, as the settle at the start of the next run
does anyway; their `diagnostic_state` accessors only read fields. A report
therefore changes nothing the guest can see. MEASURED
(`assembly::diagnostic` tests): a guest waiting in a STI/HLT loop on the
timer, with two keyboard bytes in the 8042 (one in the output buffer, one
queued) and PIT counter 2's GATE low, takes a report every 997 steps, most at
a halt; after the same run its twin that took none has the same CPU state,
time and snapshot of every device.

The VGA section follows `cmos` and is omitted entirely for other display
models, preserving their report text. All register bytes and DAC indices are
hexadecimal. Each DAC RGB triple uses the stored six-bit components (for
example `00002a`), without expanding them to display bytes. `dac 0-7` and
`dac 8-15` are raw entries, unaffected by PEL mask. The sixteen `shown` lines
map attribute colours 00h–0Fh through the same read-only colour-path helper as
text and planar graphics scanout: Color Plane Enable, internal palette,
P54S, Color Select, then PEL mask. They include RGB from the selected entry,
even when its DAC index is outside 0–15. This describes the attribute path;
packed 256-colour output pairs attribute samples into an eight-bit pixel.
It does not claim that display output is enabled or describe a previously
captured frame. Register coverage follows the IBM VGA/XGA Technical Reference
Manual, General Registers, Figs. 2-81–2-89 and 2-94.

`Vga::diagnostic_state` reads fields directly: no status-port read resets the
attribute flip-flop, no DAC read advances its address or component, and no
VRAM access changes the CPU latches. Isolation tests on `testkit::MockBus`
cover both attribute phases and every DAC read/write component phase, compare
snapshots, and assert that observation emits no guest-access trace. Assembly
tests program registers through the bus, check the complete VGA report
section, take two identical reports, then compare guest reads and writes with
a twin that took no report. The VGA model and palette programming behaviour
are unchanged; this section provides evidence for palette diagnosis.

### Guest RAM image

`PcMachine::memory_image(&self)` returns an owned `assembly::MemoryImage`
(or a CPU memory-read error) beside the machine report. Assembly copies every
system RAM range in the machine's validated memory description, sorted by
physical base: conventional RAM, extended RAM, and any configured upper-memory
or shadow RAM. Each region has a `u64` physical base, a `Ram`/`Rom` kind, and
the bytes as stored. The current capture includes RAM only. ROM, unmapped
gaps and all configured device windows are omitted, including an EMS page
frame whose CPU mapping temporarily holds a device page as RAM.

Capture uses the backend's physical host read API, bypassing CPU address
translation and A20 gating. It neither accesses nor synchronises a device;
in particular, it never reads VGA memory or changes its latches. Retired
counts, virtual time and subsequent execution remain unchanged. The image
owns its bytes, so later guest writes do not change a captured image. Its
`Display` is a short plain-text summary: one line per region with kind, base
and byte length in hexadecimal. The front end decides the dump file format;
the core performs no file I/O and supplies no flattening helper.

Images contain guest bytes, potentially including proprietary software and
private data. They are for local diagnosis only and must **never be added to
a repository**. The image is diagnostic data, not a restorable snapshot.

### The code trail

When enabled (`Machine::enable_code_trail`, `PcMachine::enable_code_trail`),
the machine's run loop records the CPU's CS:IP after every run slice the CPU
returns: a port or memory-mapped access, a HLT, a device deadline, an
interrupt becoming acceptable or a step budget. Consecutive samples at the
same CS:IP fold into one entry with a count (`x5`), so a loop that polls one
port fills one entry. The ring keeps the last 64 entries
(`crates/machine/src/code_trail.rs`). The trail lives in the machine, not in
the CPU: a sample reads two registers the CPU holds after it returned, so
retired counts, virtual time and every guest effect are the same with the
trail on or off. A disabled trail costs one branch per slice. MEASURED: the
open-BIOS FreeDOS boot of `freedos_boot_golden`, run once as the golden runs
it and once with the trail on and a full report taken at each of its ten
poll boundaries (`crates/pc-harness/tests/diagnostic_trail_golden.rs`),
stops at the same retired count and virtual time (8,800,473 and 10,000,000)
with the same screen, POST stream and device snapshots; the trail took
931,221 samples. On a load heavy in interrupts and port accesses
(`assembly::diagnostic`'s `the_delivery_log_cost_on_a_high_irq_load`: PIT
counter 0 at a count of 40h, 937,499 IRQ 0 deliveries in 60,000,000
retired instructions, a slice ending every four instructions; release
build, three interleaved runs each) the guest took 5.22–5.44 s plain and
5.26–5.64 s with the trail, within the noise, but 5.53–5.98 s with the
trail and the report's delivery log, 6 to 10 per cent more: the CPU takes a
slower exit from its compiled code while a delivery log exists, and every
slice drains it. Recording the registers with every delivery adds nothing
measurable: at x86jit `6cb817b8` the same load took 5.56–5.69 s with the
trail and the log with registers, 5.55–5.66 s with the log's registers
option off, and 5.20–5.49 s plain. So the desktop keeps the trail on from every Start and
starts the delivery log only with the first Save machine report or with
Record exceptions (§Exception and interrupt deliveries). On the FreeDOS
boot, with few interrupts, neither shows: 1.04–1.24 s with both over six
runs and 1.04–1.17 s without (test profile, the machine crates at
opt-level 3).

### Taking a report

The desktop takes one from Tools › Diagnostics › Save machine report, or
its chord Ctrl+Alt+Shift+D (⌘⇧D on macOS), whenever a PC runs, paused or
not; with no machine the item is disabled and says so. It writes the
running PC's report to `<data>/unemu/diagnostics/diag-N.txt` in the
platform data directory, N one more than the highest number already there;
with Record exceptions on it also writes the recording (§Exception and
interrupt deliveries) and the report names that file; with Record exceptions
checked but no recording running, it says why. The status line says
`Machine report saved to PATH` (and `; exceptions to PATH`). The machine
keeps its code trail from every Start; the report's deliveries start with
the first report (or with Record exceptions), so the first report of a
session lists none and says `desktop  the delivery log starts with this
report`, and the later ones list the deliveries since. There is no
environment switch. The chord belongs to the host, with the pointer
captured or not: the shell checks it before a key goes to the guest, and
its D never reaches the guest ([`crates/window/src/diagnostics.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/diagnostics.rs)). Tools ›
Diagnostics › Open diagnostics folder shows that folder in the platform's
file manager (`open` on macOS, `explorer` on Windows, `xdg-open`
elsewhere, the opener reaped by a thread), creating it first.
`unemu run --machine FILE --diag-at N` writes the report to stderr when the
run reaches step N, the unit of `--max-steps`, with the trail and the
deliveries enabled from the start, then runs on to its usual stop
(commands.md, Build / run / test).

An example, from the open guest
`assembly::test_support::keyboard_masked_halt_rom` (it initializes the master
8259, masks IRQ1, disables the 8042's keyboard interface, then executes CLI
and HLT): HLT retires, so CS:IP is the byte after it, and the `hlt` line names
the HLT; each OUT ended a run slice, so the trail passes through the
instruction after every one of them.

```text
machine diagnostic report
retired  15 instructions, 15 steps
time     virtual time 15, clock 15 ticks
status   halted (waits for a reset)
halted   yes, with interrupts disabled
cs:ip    f800:001a (linear 0f801a)
flags    0002: if 0, tf 0, df 0, iopl 0, nt 0; of 0 sf 0 zf 0 af 0 pf 0 cf 0
regs     ax 00ad bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 sp 7000
mode     real, cpl 0, msw fff0 (pe 0), a20 on
cs       f800 base 0f8000 limit 0000ffff attr 009b
ds       f000 base 0f0000 limit 0000ffff attr 0093
es       f000 base 0f0000 limit 0000ffff attr 0093
ss       f000 base 0f0000 limit 0000ffff attr 0093
gdtr     base 000000 limit 0000
idtr     base 000000 limit 03ff
ldtr     0000 base 000000 limit 00000000 attr 0000
tr       0000 base 000000 limit 00000000 attr 0000
irq      no injected vector waits, cannot accept a maskable interrupt now
hlt      at f800:0019, the byte before cs:ip
code     00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
decoded  add %al,(%bx,%si)
ss:sp    f000:7000 (linear 0f7000)
stack    +00 0000 0000 0000 0000 0000 0000 0000 0000
stack    +10 0000 0000 0000 0000 0000 0000 0000 0000
trail    cs:ip after the last 7 run slices, oldest first (7 samples, repeats folded)
trail    f800:0004
trail    f800:0008
trail    f800:000c
trail    f800:0010
trail    f800:0014
trail    f800:0018
trail    f800:001a
pic      master irr 00 isr 00 imr 02 base 08 icw1 11 icw4 01 (auto-eoi off), initialized, reads irr, lines 00, special mask off
pic      slave  irr 00 isr 00 imr 00 base 00 icw1 00 icw4 00 (auto-eoi off), initialized, reads irr, lines 00, special mask off
pic      poll mode is not modelled
kbc      status 18, command byte 10: keyboard disabled, aux enabled, irq1 disabled, irq12 disabled
kbc      output buffer empty, output port 03, input port e0, inhibit switch off
kbc      pending none, last command ad, 0 keyboard bytes queued, controller reply none, awaiting led byte no, aux device none
kbc      irq1 line low, irq12 line low
pit      0: mode 0 lsb/msb, reload 0000, count 0000, out 1, gate 1, no count loaded
pit      1: mode 0 lsb/msb, reload 0000, count 0000, out 1, gate 1, no count loaded
pit      2: mode 0 lsb/msb, reload 0000, count 0000, out 1, gate 0, no count loaded
a20      on: 8042 output port on, port 92h not fitted
cmos     shutdown status (0fh) 00, nmi unmasked, index 00
```

Between a CPU-only reset and the processor's first code-segment load, the
`a20` line is followed by `         CPU A20 held open by the reset window`: the
CPU's A20 then reads on whatever the 8042 output port and port 92h say
([decision-42](<../../decisions/decision-42 - CPU A20 is held open from a CPU-only reset until the first CS load.md>);
machine-config.md §A20 line and System
Control Port A, "Reset fetch with the gate off").

## Exception and interrupt deliveries

A machine CPU delivers exceptions and interrupts in the guest, through the
IVT or the IDT (`ExceptionDelivery::InGuest`), so the host never sees them: a
guest that faults in a loop, or whose #GP handler fixes things up and returns,
looks from outside like any running guest. x86jit keeps, on request, a
per-vCPU log of the last N deliveries (`x86jit-core` spec.md §17.10, Delivery
log; pinned since `e63c1321`, with the registers of the interrupted code
added in `42a7f272` and pinned since `6cb817b8`), outside `CpuState` and checkpoints, and with it off
nothing in execution, retirement or generated code changes; the registers
option changes none of them either. The backend
exposes it as `Cpu::enable_delivery_log`, `drain_deliveries` and
`peek_deliveries` (`crates/cpu/src/delivery.rs`, `X86JitCpu`); only the
machine profiles keep one, an application profile refuses.

When a diagnostic asks, the machine enables the log with the registers on,
at its largest capacity of 65,536 records (a slice with at most one delivery
per step loses none, and
any loss is counted as dropped; the log grows only with the records it
holds), and moves its records after every run slice into its delivery tap
(`crates/machine/src/delivery_tap.rs`): a ring of the last 64 entries for the
report (`Machine::enable_delivery_log`) and, while one runs, a recording
(`Machine::start_delivery_recording`), a ring of the last 10,000 entries with
exact totals per vector since it began. Both rings fold consecutive identical
deliveries (the same kind, vector, saved CS:IP and error code; the bytes and
the registers are not compared) into one entry with a count and the retired range, so a guest
that loops on one fault takes one entry and what came before the loop stays
in the ring. A recording started after the report's log counts from its own
start. The slice that fails with a delivery stall is read too. The tap lives
in the machine, outside the CPU, so retired counts, virtual time and every
guest effect are the same with it on or off; off, it costs one branch per
slice.

Each delivery is one line:

```text
delivery 54 fault #GP vector 0d at 0008:007b linear 0f807b error 00f8 cpl 0->0 bytes 8e d8 f4 55 89 e5 83 46 ax 00f8 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es 0000 ss:sp 0010:7000 flags 0046
```

| Field | Content |
|---|---|
| `54` | `Vcpu::retired_instructions` when the handler was entered: a software interrupt's own instruction is counted, a faulting one is not |
| `fault` | the kind: `fault` (a fault or abort, #DF and BOUND's #BR included; the saved IP is the faulting instruction), `trap` (single step; the next instruction), `software` (INT n, INT 3, a taken INTO; the next instruction), `irq` (the interrupted instruction, not yet run), `nmi` |
| `#GP` | the name: an exception's mnemonic (`#DE #DB NMI #BP #OF #BR #UD #NM #DF #TS #NP #SS #GP #MF`, others `exception NNh`), `INT NNh` (`INT 1Ch`), `IRQ n` or `NMI`; the hexadecimal digits after a name are upper case |
| `vector 0d` | the vector used |
| `at 0008:007b` | the return address the delivery saved |
| `linear 0f807b` | the CS base before the delivery plus the saved IP, before the A20 gate |
| `error 00f8` | the error code pushed; `-` when none (every real-mode delivery) |
| `cpl 0->0` | the privilege level before and after; 0 outside protected mode |
| `bytes …` | the 8 bytes at the saved CS:IP, read at the end of the run slice from RAM and ROM through the CPU's own map, the offset wrapping at the end of the 16-bit segment; `dv` is a device window (not read), `--` unmapped |
| `ax … flags …` | the registers of the interrupted code when the delivery began, after the raising instruction's own effects and before the frame push, stack switch and FLAGS changes of the delivery: `ax bx cx dx si di bp ds es`, `ss:sp` (the interrupted stack, before the frame) and `flags` (the image PUSHF would form); for a software interrupt the INT's inputs, for a fault the state its handler is entered from, for an IRQ or NMI the state after the last instruction that completed. On an 80386 the general registers are `eax …` with 8 digits, `fs` and `gs` follow `es`, and `ss:esp` and `flags` have 8 digits. Absent when the CPU did not record them |

A folded entry gives the retired counts of the first and the last delivery
of the run and their number, then the latest delivery's fields, bytes and
registers included. A range such as `12..12 x10921` is a run of deliveries at one
retired count: each faulted again in its handler before any instruction
retired.

```text
delivery 90952030..90955747 x64 fault #GP vector 0d at 17f9:ffff linear 027b8f error 0000 cpl 0->0 bytes …
```

Where the CPU's log overflowed before the machine read it, an entry
`dropped  N deliveries (the CPU's log overflowed before the machine read
it)` stands where the loss happened and ends any fold.

An IRQ is named by its 8259 input from the vector bases the master and slave
hold (Intel 8259A datasheet, ICW2); `IRQ at nnh` when neither base covers the
vector. The machine reads the log when a slice ends, and a slice ends at every
port access before the bus performs it, so the 8259s and the A20 gate are as
they were at the delivery; a store the guest made later in the same slice is
already in the bytes. A delivery whose own delivery faults is not recorded,
the exception it raised is (a #DF, or the next fault); a #DF whose delivery
faults ends in shutdown, which is not a delivery.

**The recording.** Tools › Diagnostics › Record exceptions, a check item off
at every launch, records the running PC from the moment it is checked and
every PC started while it stays checked. Each recording goes to one
`<data>/unemu/diagnostics/trace-N.txt`, created new when it is first
written (never over an existing file) and rewritten with the latest ring,
while it still holds what the recording wrote last (otherwise the text takes
the next free number), at each of: Save machine report, Stop,
unchecking the item, and closing the window or quitting; unchecking and
Stop end the recording. Nothing is written while the guest runs, so an app
that is killed writes nothing. `unemu run --machine FILE --trace-exceptions`
keeps the same recording from the start of the run and writes it to stderr
when the run ends, before the `stopped: ...` line. The text: two header
lines, the entries oldest first, and the summary, which gives the total, the
deliveries the kept entries stand for, the deliveries the log dropped before
the machine read them, and a line per name with its count (exceptions, NMI,
IRQs, software interrupts, each by number):

```text
trace    exceptions and interrupts the guest took from retired 0 to retired 61
trace    the last 4 entries, oldest first; an entry with a range and xN stands for N identical deliveries in a row
delivery 12 fault #UD vector 06 at f800:002e linear 0f802e error - cpl 0->0 bytes 0f ff cd 60 b0 11 e6 20 ax 0000 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es 0000 ss:sp 0000:7000 flags 0046
…
summary  4 deliveries, 4 of them in the entries kept, 0 dropped before they were read
summary  #UD: 1
summary  #GP: 1
summary  IRQ 0: 1
summary  INT 60h: 1
```

**The report.** The report's `delivery` lines follow the trail: `delivery
the last K of T exceptions and interrupts in E entries, oldest first (D
dropped before they were read)`, then the E entries in the line format
above. The desktop keeps the log from its first report or from Record
exceptions, whichever comes first (§Diagnostic report of a running machine,
The code trail, for the measured cost); `unemu --diag-at` from the start of
the run.

MEASURED. The open guest `assembly::test_support::deliveries_rom` (its NASM
listing and SHA-256 are pinned beside it) takes one delivery of each kind and
halts; its exact trace, in [`crates/cli-pc/tests/exception_trace.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/tests/exception_trace.rs):

```text
delivery 12 fault #UD vector 06 at f800:002e linear 0f802e error - cpl 0->0 bytes 0f ff cd 60 b0 11 e6 20 ax 0000 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es 0000 ss:sp 0000:7000 flags 0046
delivery 18 software INT 60h vector 60 at f800:0032 linear 0f8032 error - cpl 0->0 bytes b0 11 e6 20 b0 08 e6 21 ax 0000 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es 0000 ss:sp 0000:7000 flags 0046
delivery 37 irq IRQ 0 vector 08 at f800:0054 linear 0f8054 error - cpl 0->0 bytes fa b0 ff e6 21 2e 0f 01 ax 0010 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es 0000 ss:sp 0000:7000 flags 0246
delivery 54 fault #GP vector 0d at 0008:007b linear 0f807b error 00f8 cpl 0->0 bytes 8e d8 f4 55 89 e5 83 46 ax 00f8 bx 0000 cx 0000 dx 0000 si 0000 di 0000 bp 0000 ds 0000 es 0000 ss:sp 0010:7000 flags 0046
```

The #UD saves the invalid opcode `0f ff` itself, INT 60h the instruction
after it, IRQ0 the CLI after the HLT it ended, the #GP the faulting
`MOV DS,AX` in protected mode with selector F8h as its error code. The
registers follow the listing: `XOR AX,AX` leaves ZF and PF set and CLI
clears IF (`flags 0046`) on the stack 0000:7000 the guest set, for the #UD
and INT 60h alike; the IRQ0 tick ends the HLT after `MOV AL,10h` and STI
(`ax 0010`, `flags 0246`); the #GP leaves DS as it was and shows the selector
in AX and the protected-mode stack 0010:7000. The report of the same ROM
lists the same four, with `es f000` where the machine starts from
`assembly::bios`'s seed, which sets ES to F000h (the ROM never loads ES)
(`assembly::diagnostic` tests). `delivery_overrun_rom` (a #UD handler that
faults until its stack overruns and the CPU shuts down, from SP FFFFh: a
real-mode frame wraps inside the segment and the 80286 shuts down only where
a frame word would sit at offset FFFFh, at SP 1, 3 or 5) records two entries,
the first #UD and the 10,921 at the handler folded, the folded one with the
registers of its last delivery (`ss:sp 1000:0009`), and a summary of 10,922
#UDs; `delivery_stall_rom` two entries and a summary of 11,264 #GPs before its
stall stop; both stop as without the recording. `delivery_ring_rom` takes
12,000 software interrupts that never fold, alternating INT 60h and INT 61h:
the recording holds exactly the last 10,000, deliveries 2,001 to 12,000 at
their retired counts, and the summary counts 6,000 of each, from the [unemu front end](https://github.com/unemu-org/unemu)
command line and desktop alike ([`crates/window/src/shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs),
`record_exceptions_writes_the_last_entries_at_every_trigger`, which also
writes it at each trigger and finds the guest halting at the same retired
count with the recording on or off). On a bare 80286, fifty distinct INT 21h
and then a loop of 100,000 identical #UDs leave 51 entries, the fifty and one
folded entry `x100000` with the loop's retired range and the registers of
its last #UD (CX and DX change on every pass and do not break the fold), in
the report and the
recording alike (`machine::delivery_tap` tests). The twin test of the report
keeps the log and a recording on one of two machines and finds the same CPU,
time and device snapshots. The open-BIOS FreeDOS boot of
`freedos_boot_golden`, run twice more with the log, a recording and the
trail on and a report at each of its ten poll boundaries
(`diagnostic_trail_golden`), stops at the same retired count and virtual
time (8,800,473 and 10,000,000) with the same screen, POST stream and device
snapshots as without them, and both recordings are the same text: 7,120
deliveries, none dropped (INT 10h 3,367, INT 16h 1,766, INT 29h 662, INT 15h
311, IRQ 0 174, IRQ 6 155, INT 13h 160, INT 21h 136, five #DB among them);
the boot took 1.06–1.11 s with them and 1.04–1.17 s without (test profile,
the machine crates at opt-level 3).

## Implemented access trace policy

`device::TracePolicy` has two modes: `Off` (the default) and
`Ring { capacity }`. Every concrete device and the open bus use the same
`TraceBuffer`. Off retains no events, counts no drops and allocates no history.
A ring retains the newest events in access order, overwriting the oldest and
incrementing a saturating `u64` drop counter. Capacity zero retains nothing and
counts every event as dropped. `Device::take_trace` and
`Bus::take_open_bus_trace` drain retained events without clearing the counters;
`Device::trace_dropped` and `Bus::open_bus_trace_dropped` report lost events.
A drain is bounded by the configured capacity.

`ObservationPolicy.trace` explicitly selects the policy for all devices and the
open bus during assembly. `assemble`, presets and all production CLI capture
policies leave it Off. Selecting serial/POST output or a display never enables
tracing. `Bus::set_trace_policy` can also configure an existing machine; both
this operation and device-level reconfiguration discard prior history and reset
the counters. Hardware reset preserves policy and starts a new diagnostic
history. `SharedPic` forwards policy, drain and counter to its one live chip.

Machine timing tests in `crates/machine/src/corrected_tests.rs` also explicitly
select rings when asserting port/MMIO timestamps. Shared-PIC tests verify that
cloned faces configure and drain one recorder and expose the same drop count.

Snapshots exclude trace events, policy and counters. All ten device snapshot
formats already excluded access traces, so no version bump is necessary.
Restore neither emits nor restores diagnostics; the host's existing trace epoch
continues. Per-device isolation tests check that draining a trace cannot change
snapshot bytes and restoring into a fresh device cannot import history.

Trace-consuming unit tests opt into rings sized above their bounded access
counts. The scheduler A/B harness uses a 16-event ring per device, drains after
each transaction and asserts zero drops. Its complete comparison stream remains
a test-owned oracle. A slow CLI regression runs BIOS POST to its golden cap,
BIOS floppy boot, FreeDOS prompt, DIR and file-write sessions with Off and a
two-event overflowing ring. It checks every device/open-bus retained length and
observed drops, and compares the complete `BiosRun`: serial, POST, both screens
(including attributes/cursors), capture stamps, final media and stop reason.

No new environment variable or CLI option is needed; the policy is a Rust
observation/configuration choice.

## UART output ownership and snapshot compatibility

The UART sends accepted THR bytes directly to its bound `ByteSink`, without
retaining a device-owned output history. With no sink, transmission discards
the byte without allocating. Assembly already binds COM1 to one explicitly
bounded `ByteCapture`; consumers drain that collector through
`Observation::drain_bytes`. No assembly wiring change is needed. Divisor-latch
writes still emit nothing, and sink overflow does not change device state.

UART snapshot v4 starts with the former 16-byte register/time layout: version, IER, FCR, LCR, MCR, scratch,
DLL, DLM, then the little-endian 64-bit absolute trace clock. A 38-byte fixed
header also stores RX count, overrun, last receive/read time and the in-flight
byte's error/deadline; queued RX byte/error pairs, endpoint model name and
endpoint snapshot follow. The serial mouse body includes DTR/RTS, startup time,
pending protocol bytes, accumulated counts and current/reported buttons.
Versions 1–3 are rejected. Restore validates the complete snapshot and matching
endpoint model, plus endpoint DTR/RTS against the restored MCR's effective outputs
(including loopback masking), before changing state. It retains the receiving port base and sink,
and emits no historical output. Hardware reset retains and resets the endpoint,
while capture epochs remain host-owned. No snapshot contains transmitted bytes,
access traces or host input queues.

An isolated mock-bus regression transmits 1 MiB each with no sink, an
order-checking counting sink, and an overflowing three-byte capture. It checks
unchanged snapshot bytes and length, no retained access traces, complete ordered
sink delivery, and bounded capture retention. Snapshot tests separately cover
legacy-version rejection without mutation and continuation through the sink.

## Recommendation

Observe **state for displays** and **semantic emissions for streams**. A display
snapshot comes from the card's current memory and registers. Serial and POST
bytes come from the device at the point it accepts an output byte. Render each
snapshot once through `video::render_text_mode`, then distribute the same
`video::Frame` to capture and presentation sinks. Device access traces remain a
separate, explicitly enabled, bounded debugging facility.

Keep the machine core independent of VGA, UART, POST port numbers, and host
presenters. Add optional, typed observation capabilities to `device::Device` and
resolve their routes in `assembly`, using its existing device-role map. A future
window and a golden runner consume those same routes. A GUI must never implement
CRTC decoding or replay guest writes.

**Measured limitation of compatibility:** the current VGA text golden is fully
identical under state capture and trace replay. The BIOS colour-page text and
attributes are identical, but its cursor is not: state capture gives `(0, 18)`,
replay gives `(0, 0)`. Both are visible. The replay omits monochrome CRTC writes.
Existing golden assertions still pass; full `BiosRun.screen` equality cannot be
promised. See § Prototype evidence and § Migration.

## Inventory of the current paths

This section describes the code before the disposable prototype. `TraceEvent`
has only `Port` and `Mmio` variants (`crates/device/src/lib.rs`); neither is a
semantic output event. All ten concrete devices keep `Vec<TraceEvent>` buffers.
The bus separately records undecoded port accesses. A drain removes history;
there is no trace capacity or enable policy.

| Output | Producer | How it is read today / consumers | Problems |
| --- | --- | --- | --- |
| Colour text screen, glyphs, attributes, start address, cursor | `crates/devices/src/vga_text.rs`, `VgaText`: 32 KiB VRAM and CRTC; the existing `vram`, `start_cell`, `cursor_cell`, `cursor_visible` getters already expose state | `crates/cli/src/lib.rs`, `render_vga`, drains `take_trace`, reconstructs VRAM and a second CRTC register file, calls `render_text_mode`; used by `run_vga_text` and every successful `run_bios_config` assembly | Replay ignores `0x3B4/0x3B5`, unlike the preset and card of that time (the colour card's monochrome aliases were later removed; the MDA card owns `0x3Bx`); uses 256 independent index slots, unlike the card's modulo-25 indexing. A second capture loses prior writes. Restored state cannot be reconstructed from subsequent traces. |
| Card-level text golden | Same `VgaText` state | `crates/devices/tests/vga_text_golden.rs`, `hello_renders_to_captured_grid`, already uses getters → `TextModeState` → `CaptureFrameSink`; checks committed `golden/vga_text_hello.txt`, attribute and cursor | This is the desired path, but CLI duplicates it with replay. |
| MDA console text (done) | `MdaText` owns the MMIO aperture at `0xB0000` and its independent CRTC | CLI captures `DeviceId::Mda` through `capture_text`; FreeDOS/DOS goldens read `BiosRun.mda` | Card scanout supplies memory, start address and cursor; capture no longer reads CPU backing memory. |
| Prompt detection (done) | Same MDA card scanout | `ConsoleProbe.device` selects a bound `DeviceId` route; `console_contains` calls the observation facade used by final capture | One render per capture is distributed to subscribers before prompt detection. Poll cadence and injection schedule stay fixed. |
| COM1 transmitted byte stream | `crates/devices/src/uart16550.rs`, THR branch in `port_write`; `output: Vec<u8>` | CLI `drain_serial`: `Device::as_any_mut` → `Uart16550` → `take_output`. Six serial-returning runners, `BiosRun.com1`, CLI stdout; device tests also inspect `output()` / `take_output()` | Semantic source is correct and excludes DLAB divisor writes. Concrete downcast and silent empty fallback do not validate route/type mismatches. Output and access traces are both unbounded; `save` currently includes pending output. |
| POST checkpoints | `crates/devices/src/post_port.rs`, `PostPort` at configured port (`0x80` or `0x190`) | CLI `drain_post_at` filters `take_trace` for writes to the selected port, casts each value to `u8`; `BiosRun.post`, `Test386Run.post` | No semantic stream independent of trace. (Since 2026-11-15 a read returns the device's last written byte, guest-visible port state for the IBM block move, not an observation.) Trace policy cannot change until this consumer moves. Reads must never become checkpoints. |
| Port/MMIO diagnostics | UART, VGA, POST, `Pic8259`, `Pit8253`, `I8042`, `CmosRtc`, `Dma8237`, `Upd765`, `SysControl` in `crates/devices/src/` | `Device::take_trace`; all callers enumerated below. `SharedPic` forwards the drain to the same live PIC (`crates/chipset/src/shared_pic.rs`) | Every serviced access allocates retained history. Most machine runs never drain PIC/PIT/keyboard/RTC/DMA/FDC/system-control traces; UART output draining does not drain its trace. Access traffic is not a display/audio model. |
| Undecoded-port diagnostics | `crates/bus/src/lib.rs`, `open_bus_trace` | `Bus::take_open_bus_trace`, only bus unit tests currently consume it | Unbounded recording also occurs outside devices; fixing device buffers alone leaves this path. No current BIOS runner returns these diagnostics despite its source comment mentioning goldens. |
| Run termination / diagnostic site | CPU, bus and machine run results | CLI `StopReason` classification, returned in `Test386Run.stop` / `BiosRun.stop`; errors in other runners remain `anyhow::Result` | Orthogonal to device output. Preserve capture on halt, cap and fault. `Unsupported`/`Unmapped` detail strings include addresses/bytes for diagnosis, not stable golden text; `Exception` carries a vector. |
| Presentation (done) | `Observation::capture_text`: one owned `Frame::Text` per capture | Facade fans out each prompt/final frame to its route subscribers. `ObservationPolicy.output.frame_sink` selects Headless or one `CaptureFrameSink` per requested display; the VGA runner selects Capture | `Observation::subscribe(DeviceId, Box<dyn FrameSink>)` is the presenter seam. Each configured capture sink retains only its last frame; ordered history is test-owned. |
| Speaker/audio (future) | Potentially PIT channel 2 plus `SysControl` gate/data-enable | No audio frame, sample sink or runner field exists. `SysControl` bit 0 drives PIT channel-2 GATE, bit 5 samples OUT2 and bit 4 follows channel-1 refresh requests in virtual PIT time (`crates/devices/src/sys_control.rs`, coverage matrix). PIT exposes `out(n)` (`crates/devices/src/pit8253.rs`) | There is no valid sound output to capture yet. Replaying port writes would implement another timer. Audio needs actual emulated signal production before an observer can consume it. |
| Floppy writes and machine-test capture bytes | `Upd765` / `Drive` mutate mounted image bytes; machine unit tests use mock capture devices | FDC tests inspect transfer/read-back results, CLI WRITE golden observes a serial marker; `crates/machine/src/lib.rs` tests inspect mock devices through `save()` | These are storage effects / test doubles, not additional frontend output fields. Host-file write-back and whole-machine snapshots remain deferred (`backlog/docs/deferred.md`). Do not turn `Device::save` blobs into a display API. |

### Complete runner result and consumer map

Only `Test386Run` and `BiosRun` are public run-result structs in
`crates/cli/src/lib.rs`. There are no hidden audio, disk-image, trace, retired-count
or clock fields in them.

| Runner / result | Every output field or successful return value | Current golden / frontend consumers |
| --- | --- | --- |
| `run_test386` → `Test386Run` | `post: Vec<u8>`, `stop: StopReason` | `crates/cli/tests/test386_golden.rs`: exact POST bytes and unsupported-stop variant |
| `run_bios`, `run_bios_with_cap`, `run_bios_boot`, `run_freedos`, `run_freedos_interactive` → `BiosRun` via `run_bios_config` | `post: Vec<u8>`, `screen: TextGrid`, `screen_stamp: Option<ScreenStamp>`, `mda: TextGrid`, `com1: Vec<u8>`, `floppy_image: Option<Vec<u8>>`, `stop: StopReason` | `bios_boot_golden.rs`: exact POST and cap-stop variant; `bios_floppy_boot_golden.rs`: exact COM1 marker and diagnostic stop print; `freedos_boot_golden.rs`, `dos_interactive_golden.rs`, `dos_write_golden.rs`: MDA console substrings and cap-stop variant. All paths under `crates/cli/tests/`. BIOS goldens also assert display cursor and capture stamps; snapshot tests cover the colour grid. |
| `run_boot` → `Result<Vec<u8>>` | COM1 bytes | `boot_golden.rs`: exact timer markers; `main.rs --boot`: stdout |
| `run_keyboard` → `Result<Vec<u8>>` | COM1 bytes | `keyboard_golden.rs`: exact queued scancode |
| `run_rtc` → `Result<Vec<u8>>` | COM1 bytes | `rtc_golden.rs`: exact RTC marker |
| `run_fdc_write` → `Result<Vec<u8>>` | COM1 bytes | `fdc_write_golden.rs`: exact write/read-back success marker |
| `run_floppy_boot` → `Result<Vec<u8>>` | COM1 bytes | `floppy_boot_golden.rs`: exact boot-sector marker |
| `run_vga_text` → `Result<TextGrid>` | Captured final colour grid | `vga_text_golden.rs`: dimensions, first cells/attributes, untouched cell, first-row string; separate program-byte assertion |

`crates/cli/tests/quantum.rs` additionally checks CPU retired counts and
`Machine::virtual_time` directly; those are timing controls, not output streams.
Hand-encoded program-byte assertions in these tests do not consume device traces.
`TextGrid::rows_as_strings` / `to_dump` preserve the existing Latin-1-like byte to
character conversion, null-to-space mapping and trailing-whitespace trimming.
Those dumps are intentionally distinct from a future presenter's CP437/font
rasterisation (`crates/video/src/lib.rs`).

The remaining direct state/output consumers are unit tests. In
`crates/devices/src/uart16550.rs`, `take_output()` is read by
`thr_write_appends_to_output`, `dlab_banks_divisor_latch_then_restores_thr` and
`mock_bus_output_excludes_divisor_writes`; `output()` is read by
`save_restore_round_trip_preserves_state`, `reset_clears_state` and
`hello_via_output_and_trace`. CLI `drain_serial` is the only production caller
of these UART methods. In `crates/devices/src/vga_text.rs`, the state getters are
read by `mmio_word_write_stores_glyph_and_attribute`,
`mmio_byte_writes_hit_glyph_then_attribute`, `crtc_cursor_location_round_trips`,
`crtc_start_address_round_trips`, `cursor_disable_bit_hides_cursor`,
`reset_clears_buffer_and_registers` and
`save_restore_round_trip_preserves_state`. The pre-spike card golden above is
their only consumer outside that module. `crates/video/src/lib.rs` unit tests
exercise synthetic state, dump conversion and last-frame capture; they do not
reach a machine or device. No further frontend presents frames today.

### Every current `take_trace` caller

Paths and containing symbols identify all call sites, including test-only
consumers. Multiple drains in one symbol are noted. Implementations returning a
buffer are not additional consumers.

| Path | Callers / purpose |
| --- | --- |
| `crates/cli/src/lib.rs` | `drain_post_at`; `render_vga`; `tests::drain_serial_excludes_divisor_writes` (drains trace to prove UART semantic output survives) |
| `crates/chipset/src/shared_pic.rs` | `SharedPic::take_trace` forwards to `inner.borrow_mut().take_trace()`; not a second recording buffer |
| `crates/devices/src/post_port.rs` | `writes_surface_on_trace_in_order`; `trace_is_drained_on_take` (two calls); `with_port_decodes_the_bios_post_port` |
| `crates/devices/src/uart16550.rs` | `mock_bus_output_excludes_divisor_writes`; `hello_via_output_and_trace`; `mock_bus_isolation_harness` |
| `crates/devices/src/vga_text.rs` | `mmio_write_is_traced_with_virtual_time` |
| `crates/devices/src/sys_control.rs` | `writes_and_reads_surface_on_trace` (two calls) |
| `crates/devices/src/pic8259.rs` | `mock_bus_isolation_harness` |
| `crates/devices/src/pit8253.rs` | `trace_seam_records_accesses` |
| `crates/bus/src/lib.rs` | `word_out_sets_crtc_index_then_data`; `word_read_combines_devices_little_endian` (one call per device); `word_access_faults_on_undecoded_high_byte_after_low_byte`; `dword_access_uses_four_byte_cycles_in_order`; `byte_access_keeps_value_width_and_neighbor_unchanged` |

No other production or test caller drains device traces. In particular RTC,
keyboard, FDC and DMA implement the method but have no callers that drain it.
The default implementation lives in `crates/device/src/lib.rs`; all ten concrete
devices implement it. The mock devices in bus and machine tests also implement
it; machine tests do not call it.

For completeness, `take_open_bus_trace` is called only in
`crates/bus/src/lib.rs` tests:
`word_access_applies_open_bus_to_undecoded_high_byte`,
`word_access_faults_on_undecoded_high_byte_after_low_byte`,
`open_bus_accesses_are_traced_with_the_virtual_clock` (including a second empty
drain), `decoded_ports_bypass_open_bus_and_are_not_traced`, and
`strict_mode_still_faults_on_undecoded_ports`.

The disposable prototype renames the old CLI consumer to `render_vga_trace` and
adds drains in `vga_snapshot_survives_trace_drain` and
`host_snapshot_reads_live_state_without_guest_accesses`. These are experimental
verification, not additional production requirements.

## Proposed observation contract

### Typed capabilities and route ownership

Use a small optional `Device` capability for a **borrowed text scanout state**:
VRAM slice, dimensions, display start, cursor location and visibility. A query
takes `&self`, has no guest I/O side effects, and does not drain anything. The
card populates the scalars from its existing getters; only the card knows its
register aliases. Repeated queries without guest execution produce the same
frame, regardless of trace consumption or recording policy.

The neutral text-state data can live in `device`; `video::TextModeState` can
re-export it and the renderer can continue using it. This introduces only the
acyclic dependency `video → device`, not `device → devices` or
`devices → video`. An owned `Frame` is created before the card borrow ends and
before execution resumes. Do not export mutable VRAM or a register-file mirror.
Keep the current renderer's fixed mode dimensions, blank-past-end behavior and
cursor calculation during this migration; hardware rendering changes are a
separate task.

For **semantic byte streams**, attach a typed byte sink when assembly binds an
output route. The UART emits only from its accepted THR path with DLAB clear;
POST emits the low byte of each accepted write, in order, including repetitions,
zero and `0xFF`. Reads emit nothing. The sink receives bytes directly; it never
decodes ports. A byte-output capability describes its stream kind (serial TX or
POST checkpoints or emulator debug-console bytes) and allows attaching a sink; devices without it return no
capability. Later audio uses a separate signal/sample contract, not a byte-stream
kind with ambiguous units.

`AssembledMachine` should expose an observation facade that owns the route table
and collectors. Resolve a display or stream by stable `DeviceId` and output
channel at assembly time; validate capabilities then. An unavailable or
mismatched requested output is an explicit configuration error, not a blank
screen or empty serial stream. The former Long64 `run_serial` demo and
`golden_serial.rs` are removed; all machine runners use assembled Real16 CPUs.
`main.rs` offers `--boot`; no arguments print usage.

The facade can call a hardware-neutral machine/device visitor to borrow display
capabilities. The machine core only enumerates configured slots and stamps
observation boundaries; it never switches on `DeviceId::Vga` or downcasts to
concrete chips. The assembly/frontend adapter renders card state into `Frame`
and resolves display routes by device role. The exact Rust method names are left to the
implementation; required operations are:

| Operation | Owner / contract |
| --- | --- |
| Query text state | `Device`, optional read-only capability; memory and register-derived state borrowed together |
| Bind semantic byte sink | Device byte-output capability; binding done once by assembly, producer calls the sink at emission |
| Capture selected displays at a boundary | Assembly observation facade, calls renderer once per route, returns/presents owned frames |
| Read captured stream bytes | One collector per route, explicit drain or final result; a frontend and test do not race to drain the device |
| Fan out frames and byte batches | Observation facade/hub, feeds capture and presentation subscribers from the same values |

Keep `as_any_mut` as a pragmatic bridge in the prototype and initial migration,
and for existing host keyboard injection. Replace output downcasts with the
capabilities once two producers are wired. Requiring every presenter to know
`VgaText`, `Uart16550` and `PostPort` would spread concrete-device coupling;
putting those downcasts into `Machine` would violate its swap seam. A save-state
blob is also unsuitable: it is versioned restoration data, not a scanout format.

### Debug console stream

`devices::DebugConsole` is an emulator-only diagnostic aid, not period hardware.
The protocol reference is QEMU System Emulator's `-debugcon` option and
`isa-debugcon` device (`hw/char/debugcon.c`: byte-only valid accesses, one-port
region, E9h default readback). SeaBIOS rel-1.17.0's
`src/hw/serialio.c:104–129` probes 402h for E9h before sending debug bytes
with `outb`; `src/fw/paravirt.h:67` defines the probe value.

Rust hosts request `ObservationRequest { device: DeviceId::DebugConsole,
output: OutputKind::Bytes(ByteStreamKind::DebugConsole), byte_limit: ... }`.
The assembly facade binds the device's semantic sink and
`drain_bytes(DeviceId::DebugConsole, ByteStreamKind::DebugConsole)` returns
the accepted bytes in order, including repetitions, zero and FFh. Reads emit
nothing. This uses the same bounded capture/overflow/epoch contract as UART TX
and POST; unobserved output allocates no history. Reset preserves the sink;
restore never replays history. The device does no host I/O and uses no host clock.

Only byte cycles are accepted directly by the device; word/dword calls return
`UnmappedRegister` with kind `port width`. **ASSUMED integration policy:** the
existing ISA bus splits wider guest accesses into consecutive byte cycles. Only
the cycle at the configured port emits a byte or returns E9h; other cycles use
their own decode/open-bus policy. With undecoded neighbours and open-bus mode,
word/dword reads return FFE9h/FFFFFFE9h and writes emit only the low byte.
This policy is tested; no claim is made about QEMU's wider-access dispatch.

The device is currently available only to `BoardModel::Bare` test rigs.
`ibm-5170` rejects it; decision-19 confines emulator extensions to the future
`generic-386-isa` board. No machine-file syntax or CLI selection is introduced
until that board exists; its wiring belongs to `upc-o0j.6`.

### Sinks, bounds and lifecycle

The default byte sink discards output without allocating history. A capture sink
records the requested stream under an explicit byte limit; overflow is latched
and causes observation/golden failure, while guest hardware execution continues
unchanged. A live presenter can use a bounded host queue and report overflow.
No sink calls back into the machine, blocks on host I/O, changes interrupts, or
feeds host timing into execution. The hub can buffer or copy already captured
data for host I/O after a run boundary. Dropping intermediate **presentation
frames** is allowed; silently dropping captured serial/POST bytes is not.

Output buffering therefore cannot simply move from an unbounded trace `Vec` to
an unbounded device queue. Direct sink emission removes that queue; captures
remain explicitly bounded, selected and run-scoped. The UART has no
`output`/`take_output` buffer or compatibility adapter. Its snapshot v3 excludes
host output history and rejects the earlier formats, as specified in
§ UART output ownership and snapshot compatibility. Whole-machine snapshots
remain outside this output-ownership change.

Starting a fresh run clears capture state. Device reset preserves installed
routes/sink bindings; the facade explicitly decides when to start a new capture
epoch so earlier POST progress is not accidentally erased. A display restore is
observable immediately from restored state, without replay. Do not retroactively
emit already captured stream bytes on restore. Machine-wide replay remains
deferred; an eventual replay design must specify stream epochs/cursors.

`ObservationPolicy.output.frame_sink` selects the initial facade/presenter wiring
at assembly, defaulting to Headless. Presentation is run-scoped observation setup;
`MachineConfig`, presets and machine-file lowering describe hardware only.
`Observation::bind` applies the policy's presentation selection for assembly and
inline hosts alike. Hosts can later call public
`Observation::configure_output(OutputConfig)`. Reconfiguration replaces the
configured last-frame sinks, clears their frames and preserves external
subscriptions without adding routes.
`Headless` installs no presentation subscriber; it must not disable state observation
or explicitly requested stream capture. `Capture` connects one `CaptureFrameSink`
per requested text route to the same rendered frames a window would receive.
`Observation::captured_frame(DeviceId)` returns its last frame, and
`subscriber_count` exposes the wiring. A new capture epoch clears these last frames.
`Observation::subscribe(DeviceId, Box<dyn FrameSink>)` validates a bound text route
and adds a subscriber owned by the facade. External subscribers own their epoch
lifecycle. No window dependency is needed: a future presenter supplies the box.
Each capture renders once, then synchronously presents that owned frame to the
configured capture sink first, then external subscribers in registration order.
A failed capture feeds no subscriber and leaves the last captured frame intact.
The VGA runner returns the configured sink's frame and explicitly fails if the
sink received nothing, so its golden exercises delivery as well as rendering.
Subscribers receive only `&Frame`, never a machine reference. Headless routes
allocate no frame history. The facade keeps mutable subscriber state separate
from the passive card query; observing still changes no hardware state. Store
collection limits, display-selection requests and capture cadence in an
observation/run policy beside `RunPolicy`, not as guest hardware identity.
Do not change the guest's step bounds to service a window's repaint requests.

### Deterministic boundaries and assertions

Capture displays after a completed machine service boundary (including DMA),
and at every terminal path, including a CPU/bus error. At present the BIOS runner
already extracts output after classifying a failed run; preserve that behavior.
Initial assembly failure returns an explicitly absent/blank compatibility
result, not a claim that a card emitted a frame.

Use retired-instruction virtual time, including the existing halted PIT-clock
extension, to select capture boundaries. The current machine advances devices
before dispatching the CPU stop (`crates/machine/src/lib.rs`, `run_bounded`,
`advance_clock`, `tick_clock`); the spike does not change this order. Stamp a
capture with both retired count and virtual time. If streams need timestamps,
use the existing emission boundary's `VClock` and a stable sequence number to
disambiguate multiple emissions at one time. Do not invent instruction-precise
timestamps by interpolating between CPU blocks. Byte-only goldens ignore batch
boundaries and timestamps and compare concatenated bytes in emission order.

A frame query must not read guest status ports: VGA status reads advance its
retrace phase, and system-control reads advance a status phase. Repeated
observation must not change `Device::save`, traces, virtual time, retired count,
or subsequent guest execution. A UI may repaint the last owned frame on its own
host schedule; only deterministic emulator boundaries produce new observations.

Screen goldens should assert the typed `TextGrid` (dimensions, all glyph and
attribute bytes, cursor), plus the existing human-readable dump where useful.
Test `FrameSink` capture against that same frame. Existing dump encoding is kept
byte-identical. Stream goldens assert exact byte vectors; POST tests also prove
reads do not emit and multi-byte bus accesses retain existing byte-cycle
semantics. Unit tests explicitly enable debugging traces only when asserting
access behavior. An optional trace overflow fails a trace-specific assertion,
never changes screen or serial output.

### The MDA card console

The BIOS/FreeDOS preset now routes `0xB0000..=0xB7FFF` to `MdaText`, rather than
the CPU's RAM. The MDA card owns its CRTC ports (`0x3B0..=0x3B7`), mode/status
ports and unassigned display ports through `0x3BB` plus `0x3BF`. The printer
ports `0x3BC..=0x3BE` stay undecoded. VGA no longer receives monochrome aliases,
so writes to the two cards cannot share a CRTC register file. See IBM PC
Technical Reference, Monochrome Display and Printer Adapter, Programming
Considerations, and the device module's coverage matrix for assumptions.

`BiosRun.mda` captures the MDA route through `capture_text`; `BiosRun.screen`
remains the colour card. Prompt polling's plain-data, comparable `ConsoleProbe`
selects `DeviceId::Mda`. `run(machine, observation, policy)` calls the same
`Observation::capture_text` as `AssembledMachine::capture_text` and
`capture_displays`; only the latter wrapper adds execution stamps. Both paths
honour the card's start address and cursor and present exactly their captured
frame. Missing routes and lost scanout produce
`ConsoleUnavailable { device, source: ObservationError }` during polling and
`ObservationError` during final capture; CLI classifies either as an observation
failure. The source preserves `RouteNotBound` versus `ScanoutUnavailable` and is
included in the diagnostic. A missing text route fails validation before the
first poll executes any guest instruction. A bound route that later loses its
scanout fails at capture, including after a natural halt.

`poll_to_prompt` returns the last outcome and whether that poll found the needle;
`run` reuses that result without capturing again before injecting input. The
ordered synthetic workload now emits three frames (two polls and final capture),
rather than four. This removes the redundant post-poll presentation without
adding or removing a `run_bounded` call. Poll steps, execution bounds and input
injection stay fixed; final capture still runs after halt, cap and CPU errors.

The old RAM probe used cell zero and hid the cursor. Card capture exposes the
hardware cursor and the independent VGA CRTC keeps its own reset state. These
cursor changes are the intended correction to the shared-register artefact. MDA
status preserves the old per-read bit-0/bit-3 phases while its other bits read
high according to the card's documented open-bus assumption. BIOS writes now
trap as MMIO; the slow goldens' screen stamps, stop points, POST and COM1
streams are unchanged, so the execution boundaries did not move.

#### Changed expectations when the MDA card was wired

Cursor tuples below are `(column, row, visible)`. In every scenario the colour
card keeps its power-on cursor because the BIOS only programs MDA; MDA capture
now exposes the BIOS cursor that the RAM probe hid. Only cursor expectations
change; console text, attributes, POST, COM1, media, execution counters and stop
points retain their existing checks.

- BIOS POST: colour `(0, 18, true)` → `(0, 0, true)`;
  MDA `(0, 0, false)` → `(0, 18, true)`.
- BIOS floppy boot: colour `(0, 17, true)` → `(0, 0, true)`;
  MDA `(0, 0, false)` → `(0, 17, true)`.
- FreeDOS prompt: colour `(31, 24, true)` → `(0, 0, true)`;
  MDA `(0, 0, false)` → `(31, 24, true)`.
- DIR: colour `(4, 24, true)` → `(0, 0, true)`;
  MDA `(0, 0, false)` → `(4, 24, true)`.
- File write: colour `(4, 24, true)` → `(0, 0, true)`;
  MDA `(0, 0, false)` → `(4, 24, true)`.

The text-attribute decoding rebase required explicit decoded styles in assembly
unit-test cells, but no further golden expectation changes. The cursor changes
listed above remain the only changed golden expectations for this wiring.

### Traces and later audio

Implemented: `TracePolicy::Off` by default, plus an explicitly sized ring with
deterministic overwrite-oldest behavior, retained-order drain and a dropped-event
count. This covers both device access buffers and `Bus::open_bus_trace`;
`SharedPic` delegates to its existing inner device rather than recording again.
The shared recorder enforces the same capacity/overflow contract everywhere.
Trace tests explicitly select sufficient capacity or assert zero drops before
checking sequences. Off avoids recording allocations altogether. Debugger
filters remain deferred until needed.

The trace bound must not affect semantic streams or current scanout. Turning
tracing off, draining it early or overflowing its ring must yield the same
frames and byte streams. Diagnostic traffic is allowed to be incomplete when
its dropped count says so; golden output is not reconstructed from it.

Later sound needs a deterministic producer from the *actual* speaker/PIT state,
with fixed rational virtual-time conversion and either timestamped signal edges
or fixed-format PCM blocks. Capture and host audio presentation must share that
producer. Pin sample rate, channel count, sample encoding and rounding/resampler
phase in the observation identity; preserve phase across block boundaries.
Host playback may underrun or discard stale presentation blocks without changing
emulated time or the golden samples. None of the speaker hardware or audio
format is implemented by this spike.

## Migration with A/B gates

1. Record current outputs under unchanged ROM/media bytes, CPU revision, machine
   configuration and run policy. Characterise all result fields, not just current
   assertions: the BIOS test currently leaves `screen` unchecked. Pin both full
   frames and legacy dumps so cursor/attribute changes are visible.
2. Add observation capabilities and wire one facade, retaining the old path as
   an A/B oracle. Compare *within the same run*, taking the non-destructive state
   snapshot before the old trace drain. Keep full trace recording only for this
   temporary comparison. Do not run a second guest with different stop timing.
3. Switch VGA capture to current state after checking full grid equality on the
   VGA golden. For the BIOS record the measured cursor discrepancy separately:
   preserve all existing golden bytes and assertions, and add a new explicit
   state-based cursor expectation. Do not reproduce the replay bug in the card.
   If a caller requires exact legacy `BiosRun.screen` compatibility, expose the
   canonical frame separately and retain a labelled legacy cursor projection
   until that caller migrates. That adapter is not used by new presenters.
4. Bind UART and POST semantic sinks; A/B exact concatenated streams against
   existing UART `take_output` and POST trace filtering. Exercise DLAB writes,
   repeated bytes, zero/`0xFF`, POST reads, resets, early drains and error stops.
   Do not switch bounded tracing off while POST still depends on it.
5. **Done:** prompt polling and final capture use one facade by `DeviceId`, as
   described in §The MDA card console.
   Compare frames at the same existing polls, with no extra `run_bounded` calls. Re-run BIOS,
   FreeDOS boot, interactive DIR and DOS write-session goldens, checking unchanged
   text, markers, POST and stop kinds. Route `FrameSinkKind` through the facade;
   ensure headless and capture modes yield identical machine results. Unit tests
   compare complete `BiosRun` results for halt, CPU error and cap stops, verify
   ordered poll/final frames, route selection, CRTC start address and capture epochs.
   Additional tests cover Headless with zero subscribers, inline output selection,
   two external sinks after the capture sink, Bytes-only subscription rejection,
   failed captures without delivery and preservation of the polling error source.
6. **Trace migration done:** production output no longer uses replay/downcasts;
   access traces are opt-in and bounded. The slow regression compares tracing
   Off against tiny overflowing rings across all five BIOS/DOS golden workloads.
   Keep bounded-oracle mode only in tests if it still serves a diagnostic
   purpose. No committed golden fixture is rewritten merely to accommodate an
   observation implementation change.

This is a gate on **existing asserted bytes**, not a claim that the old and new
observer are equivalent for arbitrary histories. Monochrome/index aliases (the
monochrome ones since removed from the colour card),
repeated capture and restore deliberately expose errors the old path cannot
represent. Correct-state tests supplement the unchanged golden corpus.

## Prototype evidence

MEASURED against the CPU revision `b0c9ae52820dffc1e2cb75c4928a670788a916d0`
already pinned in `Cargo.lock`. No dependency pin, preset, guest input, run budget
or golden fixture was changed.

The disposable `crates/devices/src/vga_text.rs` change only opts into
`Device::as_any_mut`. In `crates/cli/src/lib.rs`, `render_vga` builds
`TextModeState` from the live card's getters. The original function is retained
as `render_vga_trace`, called after capture to compare all cells, dump and cursor.
The returned grid is the state-based one. Thus the prototype still allocates and
drains traces for measurement; it does **not** solve trace memory growth.

| Run | Full grid equality | Text dump equality | Differing glyph/attribute cells | State cursor / replay cursor |
| --- | --- | --- | --- | --- |
| `crates/cli/tests/vga_text_golden.rs` | Yes | Yes | 0 of 2,000 | `(0, 0, visible)` / `(0, 0, visible)` |
| `bios_boot_golden::bios_boot_stop_point_matches_golden`, fetched 32 KiB BIOS, existing golden cap | **No** | Yes | 0 of 2,000 | `(0, 18, visible)` / `(0, 0, visible)` |

The BIOS rows are blank in the colour window; its console is in the separate
RAM probe. The cursor difference does not indicate missing colour glyphs.

Three regression tests were added **before changing production behavior** and
run against the original implementation:

| Test | Before | After / what it proves |
| --- | --- | --- |
| CLI `vga_snapshot_survives_trace_drain` | Fails: glyph 0 instead of `A` | Passes: glyph/attribute survive an earlier trace drain and two snapshots agree |
| CLI `vga_snapshot_uses_card_crtc_aliases` | Fails: glyph 0 instead of `A` | Passes: mono ports plus the card's index alias set start=1, cursor=3 and disabled cursor; the rendered glyph is `A`, cursor `(2, 0, hidden)` |
| Device `host_snapshot_reads_live_state_without_guest_accesses` | Fails: VGA host observation capability absent | Passes on `testkit::MockBus`: writes through mono ports and an aliased index are visible via getters after trace drain; observing generates no guest-access trace |

This table records the tests as they were added; the colour card's monochrome
aliases were later removed when the MDA card was wired, and the fixtures were
renamed or reduced to colour ports and index aliasing. The alias fixture combined
monochrome port and register-index aliasing; it did
not independently isolate each alias rule. Existing device tests cover register
and port behavior, while this fixture checks that the frontend uses their result.

Validation commands use the supplied Nix devShell environment and offline Cargo.
Results and remaining unmeasured migration gates are recorded below; no broad
slow-suite success is inferred from a named BIOS run.

| Command (after loading the supplied devShell environment) | MEASURED result |
| --- | --- |
| `which cc cargo` | Both resolve under `/nix/store` |
| `cargo test --offline -p cli --lib vga_snapshot` before implementation | Expected failure: 0 passed, 2 failed, 1 filtered out; both failed glyph assertions read 0 instead of 65 |
| `cargo test --offline -p devices --lib host_snapshot_reads_live_state_without_guest_accesses` before implementation | Expected failure: 0 passed, 1 failed, 209 filtered out; downcast capability absent |
| `cargo fmt --all` | Exit 0; formats prototype only |
| `cargo test --offline -p cli --lib vga_snapshot -- --nocapture` after implementation | 2 passed, 0 failed, 1 filtered out |
| `cargo test --offline --workspace` | Exit 0: 329 passed, 0 failed, 5 ignored; additionally 2 passing watchdog subprocess cases (331 passing result entries if those are counted twice) |
| `cargo test --offline -p cli --test bios_boot_golden bios_boot_stop_point_matches_golden -- --ignored --exact --nocapture` | 1 passed, 0 failed; ROM present, A/B executed; test duration 2.82 s |
| `cargo test --offline -p cli --test vga_text_golden -- --nocapture` | 2 passed, 0 failed; A/B executed |
| `cargo build --offline --all-targets` | Exit 0 |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | Exit 0, no warnings |
| `cargo fmt --check` | Exit 0 |
| `git diff --check` | Exit 0 |

This is historical prototype evidence. At that stage the complete ignored suite
had not run, and FreeDOS/session byte equivalence, restore equivalence and
tracing-off/overflow independence were still gates. The implemented trace policy
and regression above now cover the trace gate; audio remains deferred. No
prototype timing number above measures the bounded recorder.

## Proposed implementation beads

These are titles and scopes for the supervisor to file or fold into existing
work. They are a dependency order, not a second task board. Existing bead scopes
here come from the spike brief; the tracker was not opened by this worker.

| Order | Proposed title | One-line scope / relationship |
| --- | --- | --- |
| 1 | Define typed output capabilities and observation routes | Add read-only text state and byte-sink capabilities, resolve/validate routes by role, and test unsupported routes and passive observation. |
| 2 | Capture VGA goldens from the card's scanout state | Absorb `upc-x83.1`: remove CLI CRTC/VRAM replay, A/B existing VGA and BIOS outputs, add full-grid/cursor and repeat-capture regressions. Depends on 1. |
| 3 | Capture UART and POST through semantic byte sinks | Preserve exact COM1/POST streams, DLAB and byte-cycle rules; bound explicit captures, define reset/overflow ownership and retain UART snapshot compatibility. Depends on 1. |
| 4 | Share console observation across run policies and presenters | Unify MDA card observation across prompt polling and final capture, wire `OutputConfig`/`FrameSinkKind`, fan out the same frames to capture and presentation. Depends on 2 and 3. |
| 5 | Bound and disable access traces independently of output | Expand `upc-kk2.7` to all devices plus open-bus recording, add off/ring policies and drop counts, opt trace tests in, prove identical output with tracing off/overflowing. Depends on 2 and 3; final end-to-end gate also uses 4. |

Audio hardware and a window implementation remain follow-up consumers, not
prerequisites for this seam. Do not add a speaker model or GUI as part of these
observation fixes.

### Floppy host input

⌘F/⌘E on macOS (Ctrl+Alt+F/E elsewhere) queue insertion of the next A:
catalogue entry or ejection. Shift selects B:; on macOS press Command first. Repeated presses and releases of the shortcut key
are consumed by the host, even if modifiers are released first. Next wraps to
entry zero and from an empty drive selects zero; empty is reached only by eject.
The Media menu shows each drive's current file name or `empty`. No GUI image or guest
screen capture is needed to test these controls.

The presenter drains media events before keyboard and mouse injection at the
next completed run boundary, then advances execution. Journal sealing/reopening
uses assembly's persistence handles synchronously there. A host failure stops
execution. Headless runs use `run_floppy_schedule` and `FloppyEvent.at` in the
same cumulative instruction-equivalent virtual time as run caps; equal-time
events retain input order. This is input data, independent of observation
subscriptions. Hardware semantics and snapshot compatibility are described in
persistent-media.md § Media changes; the source is the IBM PC AT Technical
Reference, Fixed Disk and Diskette Drive Adapter, Digital Input Register
(3F7h) and Interface Lines.
