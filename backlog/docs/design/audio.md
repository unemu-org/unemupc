# Deterministic audio

The `audio` crate is the audio swap layer beside `video`. It provides an
`AudioSink`, an integer mixer and headless capture, implementing rules 1–4 of
[decision-20](../../decisions/decision-20%20-%20Audio%20through%20a%20virtual-time%20AudioSink%20seam%20with%20host%20output%20in%20the%20window.md).
The AT PC speaker is the first sound-producing device. The window uses cpal
for host playback. An optional SB16 supplies byte/word-DMA PCM through a CT1745 master/voice
volume model and native YMF262 FM synthesis; analogue filtering remains unmodelled. The audio crate itself needs only the existing `device`
and `thiserror` workspace dependencies.

## Output format

The one output format is **48,000 stereo frames/s**, signed i16, left then
right, little endian. A `StereoFrame` is `[i16; 2]`; `CaptureAudioSink::pcm_le`
serializes explicitly, independent of host endianness. One frame is four bytes,
not one channel sample.

## Time base

Audio uses the exact PIT input rate **105,000,000/88 Hz**, hence the output
period is **4375/176 PIT clocks**. This is the clock ratio required by the audio
contract. Intel 8254 Programmable Interval Timer datasheet, Clock Input and
Mode 3, defines the timer's input-clock counting and square-wave behavior; the
board frequency is a machine convention, not an 8254 silicon constant.

Frame `n` covers the whole-clock interval
`[ceil(n * 4375/176), ceil((n+1) * 4375/176))`. It is emitted only when its end
has been reached. At PIT boundary `t`, `F(t) = floor(t * 176/4375)` frames are
complete. Observation `[a, b)` receives frame indices `F(a)..F(b)` (frame ends
in `(a, b]`); a partial frame at `a` was not previously delivered. Empty
intervals or intervals with no complete frame are valid. Every calculation uses
absolute positions and u128 intermediates, including clocks near `u64::MAX`.
There is no carried rounded sample duration and no long-run drift.

The facade passes `Machine::clock()` (`VClock`), the same virtual-tick
timeline the PIT consumes. It does not convert `virtual_time()` or `retired()`
back into clocks: the scheduler already carries the configured CPU rate's
fractional remainder and advances PIT time during HLT. CPU reset does not reset
the device clock or audio phase. See [scheduler.md §Adoption](scheduler.md#adoption).

Audio, window pacing and the default RTC share exact OSC = **315/22 MHz**
and PIT input OSC/12 = **13,125,000/11 Hz** (equivalently 105,000,000/88 Hz).
Output boundaries come from `AUDIO_OUTPUT.period()` in virtual ticks, so no
independent rounded presentation or RTC crystal spelling remains.

## Mixing

`AudioSource::render(start, end)` returns `SampleArea`, two signed i128
**PCM level × PIT clock** integrals over the exact nonempty interval. No source
divides or rounds its contribution. The mixer sums all source areas, divides
once by the full output frame's duration (toward zero), then saturates to
`[-32768,32767]`. Thus two sources at level 1 for 10 and 15 clocks of a 25-clock
frame produce exactly 1, including the symmetric negative case. No fixed-point
approximation loses fractions between sources.

Intervals use absolute timestamps. Source attachment clips the requested start
to the attachment time, so a source joining inside a frame contributes only its
active clocks; the mixer still divides by the full frame duration. Scratch
storage remains bounded at 1024 output frames. No device re-derives the output
grid and no floating point or host timing enters the path.

`EdgeSource` records ordered `(PIT clock, [left, right])` level changes and sums
`level * clocks` between edges. An edge at the start applies immediately; one
at the end applies to the next frame. Same-clock edges are allowed; the last
wins. Box filtering retains the duty cycle of fast square waves and PWM, with
the documented integer-clock quantization, rather than modelling analogue filters.

`RateSource` reads a shared `SampleBuffer` for the **whole device lifetime**.
The writer calls `push_segment(origin, period, frames)` to begin a rate/phase
segment and `push(frames)` to append samples to that segment. Every segment has
its own exact positive `SamplePeriod(P,Q)`. Native sample k covers
`[o + ceil(k*P/Q), o + ceil((k+1)*P/Q))`. A native rate H Hz uses
`P/Q = 105,000,000 / (88 * H)`.

The rate helper weights **every** native sample overlapping an output interval
by its whole PIT-clock overlap. It advances to the next native boundary, not the
next output sample point. Native sample indices and boundary products use u128;
unrepresentable timestamps are rejected on insertion. Periods below one PIT
clock may have zero-duration native samples; the lookup selects the last sample
active at each whole clock and skips those empty intervals.

A new segment cuts the previous held sample at its origin. `pause(at)` ends the
current segment exactly at at; the gap before the next segment is silence.
A rate change or resume appends a new origin/period to the **same buffer and
source**, including a change inside an output frame. Do not seal or attach a
replacement source for those events. Record playback history in time order;
do not pre-assign future sample times that a later device command may change.

`SampleBuffer` rejects a late sample or segment with `AudioError::HistoryOrder`:
push no later than the sample's start time, and devices push before the next
observation boundary. A rejected batch changes no history. The buffer tracks
both the safe history-release watermark and the full observation frontier:
`AudioSource::advance(until)` notifies it even for None/null output or an
observation completing no frame. Once an observation has passed a sample start,
that missing sample cannot silently be filled after silence was already rendered
or time was skipped. A sample starting exactly at the frontier is still allowed.
Sealed, paused, out-of-order and unrepresentable writes use the same error.
Clones are writer handles for one mixer consumer, not independent cursors.

The retained segment queue is that reader's cursor. Before integrating an output
frame, `RateSource` releases the prefix preceding its start, preserving the
native sample active at that start. Rendering stops at the first segment whose
origin is at or beyond the frame's end. Release likewise stops at the first
remaining segment: chronological, nonoverlapping segments put all later ones
in the future. Segment traversal is therefore linear in output frames plus
segments, rather than their product; native-sample integration is unchanged.
This also bounds retained history during a single long capture, before the
mixer's final observation release. The incomplete output frame still survives
that final release.

## PC speaker

IBM Personal Computer AT Technical Reference, September 1985, Section 1
"System Board", "Speaker" and "System-Timer Block Diagram", describes the
speaker drive as **PIT channel 2 OUT AND port 61h bit 1**. Bit 0 drives GATE2;
it does not directly mask the speaker. In Intel 8254 datasheet Modes 2 and 3,
lowering GATE forces OUT high, so software can toggle bit 1 with GATE low.
Mode 0 instead pauses counting while GATE is low. The board reference is
[IBM's Technical Reference](https://www.minuszerodegrees.net/manuals/IBM_5170_Technical_Reference_6280070_SEP85.pdf).

The physical drive maps to **0/+8192** (peak -12 dBFS), duplicated to both
stereo channels: OUT2 AND bit 1 low means no drive, hence exactly zero PCM,
including idle and disabled output. A tone's audible swing is ±4096, 12 dB under
a full-scale sound card voice (§Host mix levels); the level is not a calibrated
voltage or acoustic response. The real speaker's analogue AC coupling, low-pass
filter and mechanics are not modelled, so a sustained tone still carries a DC
component. The existing output-frame box filter integrates square waves and PWM
duty cycles.

`SharedPit8253::observe_speaker` connects a board pin observer. Channel 2 is
advanced through load and OUT boundaries when speaker data is enabled; no
intermediate transition is lost during a lazy synchronization. Direct port 61h
writes, GATE changes and timer programming also observe the resulting pin.
The observer records only level changes; it adds neither a per-instruction
callback nor scheduler deadlines. With bit 1 disabled, channel 2 retains its
constant-time arithmetic advance.

Assembly installs one `EdgeSource` through `Observation::add_audio_source`
when both PIT and SysControl roles exist, including the `ibm-5170` board.
The pin callback uses `EdgeSource::record_level`; devices do not depend on
the audio crate. Default `AudioSinkKind::None` still skips rendering PCM.
Observation propagates consumer availability through `AudioSource::set_output_enabled`
when a source attaches, output changes, a sink subscribes and before capture.
Without an enabled consumer, each device edge releases everything before its
latest incomplete output frame. Same-clock changes coalesce to the last level;
there are at most 25 whole-clock positions in a frame, so the speaker retains
at most 26 edges including a possible segment boundary. This bound holds even
when a host only calls `machine.run_bounded` and never renders audio, including
window construction before playback starts. CLI session boundaries also continue advancing/releasing audio.
No host changes or periodic cleanup calls are required. With a consumer enabled,
the full history between observations is retained. The existing requirement to
flush before changing output epochs preserves the partial frame on late enable.
Inline hosts wire the same
observer explicitly. Existing BIOS and DOS goldens keep their default output;
they do not acquire audio captures or new PCM expectations.

### Device history errors and recovery

Device `tick`/`sync` cannot return audio errors. `record_level` therefore latches
the first `HistoryOrder` failure in the shared source instead of panicking or
discarding it silently. `AudioSource::take_error` lets the mixer report that
failure at the next observation, even for None/null output or a boundary with
no complete frames. That failed render emits no PCM and leaves the mixer cursor
unchanged; history is released only through the prior successful boundary.
A retry can consume the pending interval after the error is reported.

A rejected device write requires **a new segment**. The source remembers the
current device level even when its edge was rejected. When reporting the error,
it starts a segment at the latest of the observation frontier, release watermark,
last accepted edge and any sealed tail, and re-reports that current level there.
No further pin change is needed. An intervening valid `record_level` may also
start the required segment; an invalid attempt keeps recovery pending and
latches an error again. Prior observations cannot be repaired retroactively.
Accepted history remains intact. The speaker source is not sealed at ordinary
disables or rate changes.

`EdgeSource::advance` tracks the full observation frontier, including disabled
output and partial frames. Thus an edge cannot be inserted into already
observed time just because the incomplete frame's history is still retained.

## Source lifecycle

Every valid `Mixer::render_until` call invokes each active source's
`release_before(retain_from)`, including disabled/null output, repeated
boundaries and intervals completing no frame. The safe watermark is the start
of the next incomplete frame, `ceil(F(until) * 4375/176)`, not blindly `until`:
a subsequent observation still needs that frame's beginning. Device histories
may discard older records but must preserve the active level/native sample at
the watermark. Thus default output cannot accumulate history across observations;
producer-side memory between observations remains bounded by the caller's run
chunk size with an active consumer. Unobserved speaker device edges have the
stronger frame-sized bound above, independent of render calls. Invalid
backwards/future requests do not advance the release cursor.

`AudioSource::finished()` permanently retires a source. Finished sources are
removed before rendering and after release, and receive no further callbacks.
`SampleBuffer::finish()` seals a device's final native tail at the end of its
lifetime; rate changes and pauses do not seal it. Closed rate segments and old
native frames are released after they can no longer affect an incomplete output
frame. At most the latest empty appendable segment needs to survive after all
its PCM is released. `EdgeSource::finish_at` retains its existing timestamped
end behavior.

A started stream continues emitting silence after its last source retires,
preserving frame count and hash across observation splits. A mixer that never
had a source still emits nothing. Custom sources may implement the same finished
contract after their final pending tail has been rendered.

## Sinks and observation boundaries

`ObservationPolicy.output.audio_sink` selects `None` (default) or `Capture`,
beside `frame_sink`. Capture's explicit `audio_capture_frame_limit` defaults to
zero: hash and total frame count are always maintained, while PCM retention is
optional. Selecting capture alone creates no producer or mixer. External sinks attach
through `Observation::subscribe_audio`; the configured capture receives each
block first. Sinks return promptly and never reenter or alter machine execution.

`AssembledMachine::add_audio_source` attaches a source at the current PIT clock.
Inline hosts use `Observation::add_audio_source(origin, source)`. Native rate
segments and edge history carry their own absolute timestamps.
The first source creates the mixer at that boundary. Subsequent sources join
the existing stream and contribute only from their attachment times onward.
Sources read timestamped history recorded by the device (edges or DMA blocks),
not merely a device's latest pin/register state.

Rendering takes mutable machine access. `Observation::capture_audio(machine,
until)` rejects `until > machine.clock()` with `AudioError::FutureTime` before
syncing devices or reading sources. For a valid boundary with a mixer, it calls
`Bus::settle()` to bring dormant/lazily scheduled devices to the current clock,
then renders from their history. No mixer means no settle or PCM work. Earlier
completed boundaries are allowed only if they do not rewind the mixer cursor.
Direct machine runners call `AssembledMachine::capture_audio()` at observation
boundaries; that facade always uses the current machine clock.

The assembly run driver first obtains the machine result and performs its
post-run persistence check **even when execution failed**. It propagates a
machine error first, then a persistence error; audio is rendered only if both
succeeded. Thus an audio error cannot hide a machine failure or skip checking
journal failures. Reset handling follows successful capture. Prompt polling and
normal service boundaries use this same path. Audio adds no scheduler events or
per-instruction work. With no producer there is no manufactured silence. Disabled
receivers advance phase and release history without sampling or allocating PCM.

`CaptureAudioSink::new()` hashes/counts without retaining PCM;
`with_frame_limit(N)` retains at most the first N frames. Both keep a streaming
64-bit FNV-1a hash over **all** interleaved little-endian PCM bytes and a total
frame count, independent of retention and block splits. `frames()` borrows the
retained prefix; `pcm_le()` explicitly encodes that prefix only. The facade's
`Observation::captured_audio()` returns a borrowed guard, not a buffer clone.
Drop that guard before rendering or reconfiguring. Hash/length assertions require
no PCM allocation. Existing video hash helpers do not move.

`configure_output` keeps its replace-the-capture behavior, mirroring video:
it starts a fresh configured epoch while retaining mixer phase and external
subscribers. Flush to the desired epoch boundary before reconfiguring. Backwards
rendering returns a typed error and preserves the cursor. Repeated observations
emit no duplicate PCM, but still release source history.

## Host mix levels

The host's full scale is not any one device's full scale. Assembly gives each
analogue output a fixed level so that a normal host volume suits sound cards
and the PC speaker together:

| Source | Host level | Full-scale test signal, peak |
|---|---|---|
| SB16 voice (DSP DAC after CT1745) | card output × 1/2 | 8-bit 00h/FFh square: 16384, **-6.0 dBFS** |
| SB16 FM (YMF262 after CT1745) | card output × 3/4 | one channel sustained at full output: 3063, -20.6 dBFS; nine channels keyed together: ≈18050, -5.2 dBFS |
| PC speaker | 0/+8192 | 1 kHz tone: 8192, **-12.0 dBFS**; swing ±4096 |

Card levels assume the 0 dB CT1745 reset (§CT1745). The voice factor leaves
6 dB of headroom: a full-scale voice square, a full-output FM channel and a
speaker tone together peak at 27511, -1.5 dBFS, without clipping. The speaker
is a one-sided square, so its audible swing is half its peak: ±4096 against
the voice square's ±16384, 12 dB below a full-scale card voice. FM is 3/2 of
the voice level because one YMF262 channel at full output peaks near 4084, an
eighth of the chip's 16-bit range. DOSBox Staging uses the same 1.5 OPL gain;
its source says the earlier 2.0 "was measured to be too high" and that the
exact value depends on the card or clone. Nine full-output FM channels at
their peaks on top of a full-scale voice can still exceed full scale; the
mixer then saturates as before.

The levels are integer factors applied per native sample in assembly
(`sb16_audio::VOICE_GAIN`, `FM_GAIN`) and in the speaker's edge observer;
division rounds toward zero. Hardware snapshots and device output are
unchanged by them. Measured through the window presenter and its host sink
on the open AT games profile before this calibration: voice -28.0 dBFS, one
FM channel -46.3 dBFS, nine FM channels -30.7 dBFS and the speaker -6.0 dBFS.

## Host output

Only the window binary opens the default cpal output device, when it opens its
window. Constructing a presenter or sink is inert; headless CLI runs and tests
never enumerate or open audio devices. The existing observation subscription
API connects `WindowAudioSink`; each successful bounded run captures audio after
persistence checks and before handling a CPU reset. A pre-run capture refreshes
consumer availability before speaker history can be pruned after enable/unmute.
No assembly API change is needed.

The supported-config search first selects **the source's rate, at least two
channels** (48 kHz for the PC, and for a source whose rate is not a whole
number), preferring signed i16 over f32. If unavailable, it accepts the device's default
output configuration when it has at least two channels, a positive sample rate
and i16/f32 samples. Otherwise it chooses the supported rate nearest that rate
(clamped to each range); ties prefer i16, fewer channels, then the higher rate.
An unusable default or failure to query it does not prevent the range fallback.
No compatible configuration or stream creation/playback failure leaves the window
silent with one clear diagnostic. A selected rate other than the source's prints
one informational line: `audio: converting 48000 Hz to <rate> Hz for <device>`
(the source's rate as `N/D Hz` when it is not whole).
On macOS, opening a 48 kHz stream may switch the shared audio device's rate,
affecting other applications using that device.

The emulated stream and headless captures remain **48,000 Hz stereo i16**.
Another machine drains at the exact rate its description states, from 8 000 to
192 000 frames a second (core-contract.md §13.2, "Sound at any rate"); the
window refuses a rate outside those bounds with one diagnostic.
On the producer side, before frames enter the ring, a streaming linear
interpolator converts to the device rate. It keeps the previous stereo frame
and an integer fractional phase with the exact step **(n/d)/device_rate** for a
source at `n/d` Hz across
blocks, including empty blocks; chunking cannot reset the phase or cause drift.
For a 48 000/1 source the arithmetic is the one the 48 kHz-only converter used,
so the PC's frames are unchanged bit for bit.
The first frame is emitted unchanged at time zero. Fractional positions wait for
the next input frame, so the unflushed tail is less than one input-frame interval
(20.84 microseconds at 48 kHz); no synthetic end padding is added. When the
device plays the source's own rate the producer copies frames unchanged. Conversion uses bounded stack scratch with no per-block
allocation and takes place outside the ring lock, never in the real-time callback.
This inexpensive presentation converter has integer quantization and linear
interpolation's high-frequency attenuation; it has **no anti-alias low-pass**,
so downsampling, especially to 16 kHz, can alias content above the host Nyquist
frequency. It is not a high-fidelity band-limited resampler and does not alter the
deterministic mixer. The callback converts i16 to f32 as `sample / 32768` if needed.
Left and right occupy the first two channels; every extra host channel is zero.

cpal's `Xrun`, `DeviceChanged` and `RealtimeDenied` notifications leave playback
active: the backend retains or reroutes the stream. Each kind is reported at
most once per session. Other errors disable output and release the stream on
the event loop. Callbacks only set atomic flags; they never log. Automatic
reconnection after a fatal error is not implemented. A latched audio-source
error is also reported once, but does not disable playback or stop the window;
the next capture retries the recoverable observation interval. CPU and
persistence failures retain their existing behavior. The Nix devShell adds
`pkg-config` and `alsa-lib` on Linux for ALSA; macOS needs no extra system inputs.

The preallocated ring and every queue threshold count **device-rate stereo
frames**: capacity is `floor(device_rate * 200 / 1000)` (200 ms), prefill and
latency target are `floor(device_rate * 50 / 1000)` (50 ms), and trimming starts
above `floor(device_rate * 70 / 1000)` (70 ms). At 48 kHz these remain 9600,
2400 and 3360 frames; at 44.1 kHz they are 8820, 2205 and 3087; at 16 kHz,
3200, 800 and 1120. An underrun plays silence and rearms the same prefill threshold.
A queue exceeding the trim threshold drops its oldest frames down to the target;
capacity overrun also discards the oldest complete frames. These are presentation
policies, independent of emulation.

The mutex is retained because producer-side oldest-frame removal and consumer
reads share a cursor. Each write/read critical section copies PCM in at most
two slices, with no per-sample work under the lock. A callback copies at most
1024 frames per section into stack scratch, then converts and fills host channels
outside the lock. It takes the short lock rather than replacing a whole buffer
with silence on producer contention, and allocates nothing. The existing window
host-clock pacing remains authoritative; neither the queue nor the device clock
advances or delays virtual time. Occlusion/suspension stops production under
that policy, so playback drains to silence.

**⌘⇧M on macOS (Ctrl+Alt+M elsewhere) toggles host mute** while a device is
active. On macOS, press ⌘ first: Shift pressed before Command remains guest
input; Shift pressed while Command is held is suppressed. Command never reaches
the guest, and Ctrl/Alt chords remain guest input. Shortcut letters follow the
logical host layout. Other platforms reserve the M pair and forward modifiers.
Repeats do not toggle again, and an M already held by the guest is not stolen
when modifiers arrive later. F12 reaches the guest normally. The shell's audio
status and View menu describe availability, speed mute and the user preference;
see [commands](../commands.md) for the complete shortcut table. Mute clears
queued audio and interpolation history, rearms prefill and disables this sink's
rendering demand; unmute
refreshes speaker-history demand before the next run so newly generated edges
survive until observation. Automated validation never plays sound; physical
playback and device support require a manual host check.

## SB16 8-bit output

The optional `sb16` ISA card implements Creative's *Sound Blaster Series
Hardware Programming Guide*, §2 DSP I/O Ports, Resetting the DSP,
Acknowledging DSP Interrupts and Configuring DMA and Interrupt Settings;
§3 Digitized Sound I/O Programming; §6 commands 10h, 14h, 1Ch, 40h, 41h,
48h, Cxh, D0h/D4h, D1h/D3h/D8h, DAh and E1h. No preset installs it.
DSP version **4.05** follows the physical SB16 reported in Epson Product
Support Bulletin S-0172, Software, page 4
([primary source](https://files.support.epson.com/pdf/e4p___/e4p___ps.pdf)).

The seven DSP/mixer byte ports are base+4/+5 (mixer index/data), +6 (reset),
+Ah (read reply), +Ch (write command/data, read busy), +Eh (read availability
and acknowledge byte IRQ), and +Fh (acknowledge word IRQ). See
[16-bit output](#sb16-16-bit-output) and [CT1745](#ct1745). OPL ports alias the onboard YMF262; see §OPL3. Creative's §2 Resetting the DSP recommends a
3 µs reset hold and describes typical 100 µs initialization. This model clears
state on writing 1 and accepts **any 1→0 edge**, including two writes at the same
PIT clock. AAh becomes available **immediately (zero initialization delay)** on
that edge. This is an **ASSUMED compatibility policy**: counted driver polling
loops can exhaust before one PIT clock elapses on a fast CPU/PIT ratio. A physical
minimum pulse or fixed 120-clock delay would make those drivers miss the card.
Busy is set only while reset is held; other writes and replies are immediate
(**ASSUMED**, DSP execution latency is unmodelled). Reset clears the parser,
replies, DMA state, speaker flag and IRQ. Initial/reset defaults are 22050 Hz,
unsigned mono and a 65536-byte legacy auto-init block (**ASSUMED**).
Unsupported DSP commands consume all known argument bytes and perform no
operation or reply. The parser recognizes Creative §6's input, ADPCM, silence,
MIDI and generic DMA argument lengths as well as SB16 ASP/test-command lengths
(**ASSUMED** for 04h/05h/08h/0Eh/0Fh/F9h/FAh), cross-checked against
[DOSBox-X's SB16 command-length table](https://github.com/joncampbell123/dosbox-x/blob/master/src/hardware/sblaster.cpp).
For example, recording Bxh consumes and ignores mode and two length bytes;
42h/80h consume two;
38h/E2h/E4h consume one; 2Ch/98h/99h take none. E4h/E8h remain no-ops.
Unknown opcodes consume one byte, and none of these commands emits a diagnostic.
The reply queue keeps 16 oldest bytes
and drops excess replies (**ASSUMED** bounded buffer policy). Empty reply reads
return FFh and unused status bits read zero (**ASSUMED**).

Time constant `tc` gives the exact frame period
`105 * (256 - tc) * channels / 88` PIT clocks, where channels is one for
legacy mono and two for Cxh stereo. The time constant byte remains unclamped,
including 255 (**ASSUMED**); explicit-Hz limits apply only to 41h.
Command 41h accepts 5000–45000 Hz and gives
`105000000 / (88 * Hz)` clocks per frame, without the stereo factor.
Out-of-range explicit rates clamp to 5000–45000 Hz (Creative §6 41h defines
the range; clamping is the model's compatibility policy). A live 40h/41h change
retains the held sample until its existing boundary, then starts the new
rational grid there, without inserting a silent period. Absolute boundaries use
`origin + ceil(index * numerator / denominator)` with integer u128 products.
There is no floating point or repeated addition of a rounded period.

**ASSUMED playback details:** one frame of staging, one byte mono or two bytes
stereo, fetched at the playback boundary. The first frame starts one native
period after the command. Cxh's FIFO bit is decoded but does not enlarge this
staging depth. The machine settles DMA before leaving each device deadline,
including deadlines inside a slow CPU instruction. Short DMA service can finish
a stereo pair in a second service at the same timestamp; a zero-byte service
renders silence and retries at the next native boundary. No bytes are fabricated
into the DSP transfer count. Odd stereo blocks pad their right channel with
silence. The 8237 owns its address/count, terminal count and auto-reload.
DSP block lengths count bytes (the little-endian value plus one). A single-cycle
command received during unpaused auto-init waits for the current block to drain, then
starts on the following frame boundary, as described under 1Ch. A chained
14h/Cxh received while an unpaused completed block's last frame is held also starts on
that frame's existing end boundary. There is no intervening pause or phase
restart when the sample period is unchanged. Command 48h stores a separate
legacy auto-init size for 1Ch: single-cycle counts and Cxh's explicit auto-init
length never overwrite it.

A new DMA command received while paused clears the pause and starts a fresh
block at the command timestamp, with its first fetch one native period later.
This **ASSUMED compatibility policy** also discards an unfinished paused
legacy auto-init block when replacing it with single-cycle playback. Only
unpaused playback waits for the old frame/block to finish; a paused transfer
cannot reach that boundary without another command.

The last byte of each block asserts the configured level IRQ. Reading base+Eh
lowers it; subsequent blocks continue even if an old IRQ remains unacknowledged
(no counted interrupt queue). A single-cycle block or DAh drains the last
frame and then pauses the source at the next exact boundary. Auto-init reloads
the DSP block size independently of 8237 auto-init. D0h silences at its timestamp;
D4h restores the held level and shifts the native origin by the paused duration,
preserving fractional phase (**ASSUMED**, the guide only specifies stopping and
resuming requests). D1h/D3h only update the D8h flag: on SB16 the guide explicitly
says they have no practical effect on the output signal.

Unsigned bytes map to `(byte - 128) * 256`, signed Cxh bytes to `i8(byte) * 256`;
mono duplicates the level and stereo uses left then right. Direct DAC 10h holds
an unsigned level from the data write until another DAC write, DMA command or
reset (**ASSUMED** analogue reconstruction). E0h inversion and immediate F2h
byte IRQ are compatibility probes **ASSUMED** here because this guide does not
specify them.

Assembly attaches one `RateSource`/`SampleBuffer` for the device lifetime.
Every native frame begins a timestamped segment; the next segment clips it at
the exact carried-phase boundary. Pauses, rates and reset keep that same source.
The adapter latches audio-history errors for observation and prunes below the
incomplete output-frame boundary on every unobserved producer event, even
without facade capture calls. `SampleBuffer::push_segment` replaces a segment
starting at the same clock, whose duration would be zero; this small audio-helper
change bounds repeated direct-DAC writes at one clock. Its public API is unchanged.
Hardware snapshots use model marker `SB16-FM4`, preserve partial commands, replies,
phase, separate legacy/active block sizes, queued transfer format and auto-init
flag, block position, staging, reset state and IRQ, and exclude trace/audio
history. Restoring starts presentation at the restored clock without replay.

The synthetic ROM golden programs channel 1 at RAM 1000h and plays bytes
00h/40h/80h/FFh at 8000 Hz. At one retired instruction per PIT clock, the
last DSP argument is written at clock 204, frames begin at 354/503/652/801,
the byte IRQ rises at **801**, the handler emits one marker at **805**, and
the source goes silent at **950**. Capture through clock 1000 produces
**40 frames**, FNV-1a **`d11fa4ee5f6d91f9`**. Earlier hashes were
`0eb2d059128e5341` (unity gain) and `9c71fd79a30a2741` (master and voice reset
to level 24, -14 dB each). Now the CT1745 resets to 0 dB and the host mix takes
half of the card's voice (§Host mix levels). The independent integral halves
each native level before accumulating it and reproduces every output frame; all
IRQ/sample timestamps are unchanged. Whole and split runs match an independent
per-PIT-clock integral. A second ROM run uses three PIT clocks per retired step
to put DMA deadlines inside CPU steps.

Not modelled: recording, ADPCM, high-speed legacy commands, MPU-401/MIDI,
analogue filters, DSP execution latency and ISA contention.

## SB16 16-bit output

Creative's *Sound Blaster Series Hardware Programming Guide*, §3 Digitized Sound
Data Format, §6 Bxh/D5h/D6h/D9h and §2 Sharing of Interrupts define word playback.
B0h/B2h start single-cycle and B4h/B6h auto-init output; command bit 1 (FIFO) is
accepted with the same one-frame staging policy as Cxh. Bit 3 requests recording:
those commands consume all three arguments and produce no playback. Bit 0 must
be zero. The mode byte selects signedness (bit 4) and stereo (bit 5). Length is
the count of **words**, minus one, including both channels in stereo. Little-endian
words decode to signed PCM directly or, for unsigned data, by subtracting 32768.
Stereo order is left then right; an odd word count gives a silent right tail.

The configured system DMA channel (5 by default, 5/6/7 accepted) routes through
the board's word 8237. Its local channel is system channel minus four; the
controller owns word address shifts, page latches, terminal count and auto-init.
The DSP length and controller count remain independent. Both widths use the
same exact rational frame grid, fetch-at-boundary, empty-DMA retry, block/tail,
rate-change and handler-chaining policies described above. **ASSUMED command-width
policy:** D5h/D6h pause/continue only word playback; D9h exits word auto-init at
the current block end. D0h/D4h/DAh operate only on byte playback. The guide lists
separate commands, while some emulators treat each pair alike. Both widths retain
fractional phase over pause.

**ASSUMED one-output policy:** starting the other width immediately discards the
unfinished transfer, including a paused block, and starts its first frame one
period later. Only same-width commands can defer to the old frame/block boundary.
There is no simultaneous byte/word DAC output. Already pending IRQs survive a
replacement independently. Word block completion latches IRQ16; reading base+Fh
clears only IRQ16 (read value FFh, **ASSUMED**). Byte acknowledgement at +Eh clears
only IRQ8. Their logical OR drives the configured IRQ line. Mixer 82h reports
IRQ8/IRQ16 in bits 0/1; MPU-401 bit 2 is always zero.

Snapshots preserve word format, both pending IRQs, word staging and any queued
transfer, mixer index/registers and fixed resources, plus complete FM state. Old `SB16-8V2` snapshots keep
their byte-DMA state and restore a reset mixer with no word state. Presentation
history remains excluded, and restoration applies the restored volumes at the
restored clock.

The synthetic word golden programs DMA5 at physical RAM 1000h (word address
0800h), then plays signed stereo words 1234h/EDCCh/8000h/7FFFh at 8000 Hz.
Master is 0 dB; voice is -2 dB left, -4 dB right. The last DSP argument is at
clock 232, frames start at **382/531**, IRQ16 rises at **531**, the handler marker
is at **535**, and the tail ends at **680**. Capture through 1000 has **40 frames**,
FNV-1a **`a6c019b8a1e874ff`** (`45d03674db163436` before the host mix halved the
card's voice). Split runs around each boundary agree; an independent
clock-by-clock integral with separately specified attenuation constants checks
every PCM frame. Stereo-order and word-fetch mutations must break this golden.

## CT1745

Source: Creative's *Sound Blaster Series Hardware Programming Guide*, §4 CT1745
Mixer, Figure 4-3 and Register Functions (pp. 4-10–4-17); resource/status registers
come from §2 Sharing of Interrupts and Configuring DMA and Interrupt Settings.
Base+4 selects a register and base+5 accesses it. Register 00h writes restore mixer
defaults without stopping the DSP. DSP reset preserves mixer state; whole-device
reset restores both (**ASSUMED** independent reset domains). Index readback,
zero reset-register readback, reserved-bit zeroing and unknown-index FFh readback
are **ASSUMED** where the guide does not specify a read value.

| Registers | Implemented bits | Reset bytes | Effect |
|---|---|---|---|
| 30h–35h master, voice, MIDI L/R | 7–3 | F8h (level 31), **ASSUMED** | Master/voice and master/MIDI applied |
| 36h–3Ah CD/line L/R, mic | 7–3 | 00h | Stored only |
| 3Bh PC speaker | 7–6 | 00h | Stored only; board speaker is independent |
| 3Ch output switches | 4–0 | 1Fh | Stored only |
| 3Dh/3Eh input switches | 6–0 | 15h/0Bh | Stored only |
| 3Fh–42h input/output gains | 7–6 | 00h | Stored only |
| 43h AGC | 0 | 00h | Stored only |
| 44h–47h treble/bass L/R | 7–4 | 80h | Stored only |
| 80h IRQ setup | 3–0 | Configured IRQ bit | Writes ignored |
| 81h DMA setup | 7–5, 3, 1–0 | Configured byte and word bits | Writes ignored |
| 82h IRQ status | 1–0 | 00h | Live pending IRQs; writes ignored |

The guide specifies volume levels 0–31 as -62 to 0 dB in 2 dB steps. Level zero
uses Q24 gain **13327**, not digital mute. For all levels the constant Q24 table is
`round(2^24 * 10^((level - 31)/10))`, generated and checked in a test using floating
point **only in that test**. Each channel computes
`pcm * master_gain * voice_gain / 2^48`, dividing once toward zero. Runtime is all
integer arithmetic. At reset this maps -32768/-16384/0/32512 to themselves; at
the guide's level 24 on both stages they would become -1304/-652/0/1294.

**ASSUMED compatibility deviation:** the guide resets master, voice and MIDI to
level 24 (C0h, -14 dB each). This model resets them to level 31 (F8h, 0 dB),
both at power-on and on a register 00h write. On real cards Creative's
installation runs a mixer set-up at every boot; without it a game that never
programs the mixer plays 28 dB down. 86Box's CT1745 ("changed defaults from
-14dB to 0dB") and DOSBox Staging's `ctmixer_reset` (level 31) make the same
choice. Volume writes alter the held PCM immediately at the port-access
clock, without changing the native sample grid, including direct DAC and paused
playback on resume. Stored-only controls do not affect PCM, even output gain and
tone controls. No board PC-speaker signal passes through this card.

Compatibility registers 04h/22h/26h/28h/2Eh alias native voice/master/MIDI/CD/line.
**ASSUMED common compatibility mapping:** each nibble n writes native level
`(n << 1) | 1`, so Fh reaches native 1Fh (0 dB). Reading returns the upper native
four bits. Register 0Ah aliases
the upper three mic bits and clears the other two on write (**ASSUMED**). The
guide says these are mapped but does not resolve the extra native bits. Native writes are immediately visible through compatibility reads.

Registers 80h/81h expose the fixed machine-file resources, with IRQ2/5/7/10 in
bits 0/1/2/3 and DMA channels in their numbered bits. **ASSUMED model policy:**
resources come from the machine file and stay fixed for the run, so writes are
ignored. A real SB16 can select resources through these registers in software;
that behavior is not modelled. ISA PnP is also not modelled.

## Validation

Mock-bus speaker tests pin Mode 0/2/3 OUT boundaries, independent GATE/data
control and eight direct bit-1 transitions. Audio tests check their exact
box-filtered frames, release and error recovery; observation tests force a
late device callback and verify the next capture reports it with either output
selection. Synthetic AT ROM goldens run the real CPU and board: a Mode 3 divisor
1193 tone enabled for 11930 PIT clocks produces **482 frames**, FNV-1a
`e2a339c1952cc419`, over a 12000-clock capture. A second ROM toggles bit 1 at
clocks 7, 13, 29, 31, 48, 52, 73 and 99; its five frames hash to
`41b2593daa7dfc61`. Both compare whole and split captures; an independent
per-PIT-clock reference checks every PCM frame. An idle/disabled-drive ROM test
checks exactly zero PCM. These speaker hashes replace `59dff6b1a48033ed` and
`d8d6af3ebdc2782d` (drive 0/+16384), which replaced `8ac7ec7fc5f8c7ad` and
`b01b4e720c32724d` (drive −8192/+8192); the drive is now 0/+8192.

A million-instruction bit-1 loop uses both direct `machine.run_bounded` and the
CLI's `AssembledMachine::run_session` API without audio output. At 100 instructions
per PIT clock it also stresses multiple writes at one timestamp; retained history
stays at or below 26 edges. Later capture enable retains full new history.
Recovery tests cover rejected rising and falling transitions without any later
pin change. Mock-bus reprogramming tests pin a new count halfway through a Mode 3
high half (the old half ends at clock 5) and a control word during a low half
(OUT rises immediately at clock 6, with the new count loaded at 7).

The exact time-base, capture-guard, post-error, HLT and device-sync tests remain.
Native constant/ramp/square expectations now use clock-weighted integration:
for example the PIT-rate ramp averages 0..24 to 12 in the first frame, and a
96 kHz square contributes both halves instead of only its initial level.
Dedicated 44.1 kHz and phase-offset 8 kHz cases check boundaries strictly inside
an output frame. The offset matters because a zero-origin 8 kHz grid aligns
with every sixth 48 kHz frame boundary.

A rate change and a pause inside one frame, their tails across later frames,
and a later resume all use one source. Repeated rate/pause segments with output
disabled retain at most two native samples and one segment at the chosen
observation cadence; those samples cover an incomplete frame and must survive.
Late writes fail without changing previously captured silence. The 10+15-clock
unit-level regression proves exact cross-source summation.

The frozen three-source, 176-frame hash changes from `1f672be3ca235cfd` to
`2b19e84103e1b4b2`: 44.1 kHz native transitions within a frame now contribute
clock-weighted blends, and sources no longer round separately in Q32. An
independent per-PIT-clock integer reference confirms the new hash. Whole and
split renders agree. Existing guest/video golden expectations are untouched.

## OPL3

The onboard **Yamaha YMF262** uses the integer native generator ported from
[Nuked-OPL3 1.8](https://github.com/nukeykt/Nuked-OPL3), pinned to
`765ec962e473aeb767e4cba74ffdc8f588ffbfe8` (decision-21). Hardware reference:
[Yamaha YMF262 datasheet](https://www.bitsavers.org/components/yamaha/YMF262_199110.pdf),
FM Sound Generator, §2 CPU Interface, Register Map, Timer and Status
Register; board reference: Creative's *Sound Blaster Series Hardware Programming
Guide*, §2 FM Music I/O Ports and §4 MIDI Volume.

The translated generator and ROM tables preserve **Copyright (C) 2013-2020
Nuke.YKT** and LGPL-2.1-or-later; the upstream licence is included in
`crates/devices/src/opl3/LICENSE`. The rest of the integration remains AGPL.
The combined application remains AGPL-3.0-or-later. There is no copied C core,
FFI, upstream resampler, buffered-write queue or stream helper in the runtime.

| Function | Coverage |
|---|---|
| 36 operators, 18 channels, log-sine/exp ROMs | Native integer generator, C narrowing/wrapping semantics |
| Attack, decay, sustain, release, key scaling | Upstream rate/phase ordering, feedback and key edges |
| NEW, OPL2 waveforms, 4-operator connections | Upstream register decoding and algorithm changes |
| Rhythm, noise, tremolo, vibrato | Upstream integer state and depth controls |
| Output channels | All A/B/C/D generated with upstream channel sample delay |
| Board output | ASSUMED A=left/B=right; C/D disconnected |
| Timers | 8-bit reload, start, mask, overflow latches, status IRQ reset |
| External timer IRQ | ASSUMED disconnected on SB16; never asserts the PIC |
| Analogue DAC, filters, stereo extension | Not modelled |

One native frame is **24 PIT input clocks**, derived from the ISA master clock
(288 master clocks / 12). The grid begins at whole-device reset. The chip catches
up lazily on any SB16 port access or native-source rendering/advancement; CPU
instruction boundaries and audio observation cadence never change its phase.
Generator writes apply in order immediately before the **next** native boundary, including
writes made exactly on a boundary. The first frame begins at origin + 24, with
silence before it. The core continues running during silence, preserving its
noise, LFO and envelope clocks before the first programmed tone.

Timer 1 (02h) ticks every 4 samples (96 PIT clocks); timer 2 (03h) every 16
(384 clocks). Registers 02h–04h act immediately on the data-port write, including
IRQ reset and start; they do not use the generator's next-sample write queue.
A rising start bit in 04h loads the preset. **ASSUMED divider phase:** the
4-/16-sample dividers share the device-origin native grid, independently of
software start time. The first tick is at the next strictly future divider
boundary (origin + a multiple of 96/384 PIT clocks). For example, starting T1
with FFh at clocks 5 or 95 overflows at 96; starting at 96 overflows at 192.
Preset writes during a run affect the next reload. Bits 6/5 mask
and clear their overflow latches. Bit 7 clears both latches without changing the
running/mask state. Overflow flags are suppressed while masked. Status at the
bank-0 address port returns IRQ/T1/T2 in bits 7/6/5, with low bits **zero**,
including NEW=0: this is an OPL3, so classic AdLib detection distinguishes it
from an OPL2. Other reads return FFh.

The card always includes FM. Bank 0 address/data is at **388h/389h**, bank 1 at
**38Ah/38Bh**; base+0..3 aliases both banks, base+8/9 aliases bank 0. A single 9-bit address latch is shared across aliases.
Per Yamaha YMF262 datasheet §2 CPU Interface, A1 selects the bank during
address writes and is ignored during data writes. **ASSUMED silicon compatibility,
following [ymfm's YMF262 address-high handling](https://github.com/aaronsgiles/ymfm/blob/main/src/ymfm_opl.cpp)
and [DOSBox DBOPL](https://dosbox-x.com/doxygen/html/dbopl_8cpp_source.html):** when the last written NEW=0,
bank-1 addresses fall through to bank 0 except 105h, the NEW register itself.
The gate applies when the address is latched, using the last 105h write still
queued for generation, or the generator's current NEW bit when none is pending.
Drivers can enable NEW and immediately program bank 1, including connection
select at 104h, while generator changes still wait for the next sample boundary.
The existing pending-write queue supplies this state, including after snapshot
restore; there is no additional field or snapshot-format change. Dual-OPL2
software with NEW disabled therefore reaches channels 0–8. All are claimed byte ports and participate
in the normal resource-conflict validation. The DSP reset does not reset FM;
a whole-device reset does. No optional machine-file switch is needed.

Native A/B frames feed their own `SampleBuffer` / `RateSource`, at exact period
24/1, beside the voice source. CT1745 MIDI volume 34h/35h and master 30h/31h use
the existing Q24 table and one division toward zero, exactly like voice.
Volume changes immediately update the held FM frame without moving the grid.
FM and voice contribute unrounded areas to the existing saturating output mixer.
An unprogrammed OPL3 contributes exactly zero and preserves the voice goldens.

`SB16-FM4` snapshots include the address latch, queued next-boundary writes, timers,
sample origin/index, held level, every decoded generator register, operator
phase/envelope/feedback, channel connections, output pipeline, LFO and noise.
Connections serialize as indices, with explicit little-endian fields and no
Rust padding or pointers. `SB16-MV3` restores with FM reset at the restored clock;
`SB16-8V2` also resets the mixer/word state as before. Presentation history is
excluded; restored audio resumes at the restored time.

Reference vectors under `crates/devices/tests/data/opl3/` contain full native
A/B/C/D samples in lossless frame run-length encoding, fixed write sequences,
SHA-256 of the decoded PCM, and a small original C driver linking the unmodified
pinned upstream outside the repository. Tests compare every frame, both native
generation APIs, and snapshot continuation during each sequence. They cover a
key-on/off tone and release, all 4-op algorithms/pair changes, rhythm/noise,
OPL2 waveform masking, LFO depths, feedback and all four panning buses.

The synthetic FM ROM programs a 2-op tone through 388h/389h, runs a counted
4000-instruction sustain, then keys off. At one retired instruction per PIT
clock, capture through clock 24000 pins **965 frames**, FNV-1a
**`5f94c755a036c6f7`** (`31fe79c8e4314714` with master and MIDI at level 24 and
no host mix gain). Whole and two differently split runs compare every captured
frame. An unprogrammed OPL3 adds nothing to the byte/word SB16 goldens; their
hashes changed only with the mixer reset and host mix levels.
