# Architecture

This describes the implementation on `main`: a deterministic, headless IBM
PC/AT-class emulator with a real-mode CPU, device models, declarative assembly,
and BIOS/FreeDOS integration tests. Testability, swappability, and documented
hardware coverage guide the crate boundaries. Corrected timing and deadline
dispatch are adopted; design documents distinguish implementation from proposals.
The hardware specification is [`design/spec.md`](design/spec.md).

## Generic 386 ISA board

`assembly::BoardModel::Generic386Isa` composes the AT peripherals with an
80386DX CPU selected through x86jit's `CpuModel::I80386`, a 128-byte CMOS RTC,
port 92h and emulator-only fw_cfg and debug console. Its own validation
requires these resources, 640 KiB conventional RAM, and up to 63 MiB of
extended RAM in 64 KiB steps. Firmware shadow RAM covers C0000h–FFFFFh;
the Listed SeaBIOS ISA image is preloaded at E0000h and separately mapped
read-only at FFFE0000h–FFFFFFFFh. Reset retains both memories independently.
The backend uses a 4 GiB flat virtual span without mapping the address gap;
the pinned core's sparse region allocator is not implemented. IBM 5170
assembly keeps its original 24-bit map and read-only adapter space.

The shipped `machines/generic-386-seabios.toml` validates and assembles with
register-level VGA and COM1. CPU pin `99ed385e` supplies the 386 reset image;
the adapter reapplies the retained board A20 line after installing it. Tests
qualify the high reset fetch, CS reload and RAM retention. SeaBIOS emits its
402h banner, then after 3,081 retired instructions the core refuses
protected-mode `BT EDX,21` (`0F BA E2 15`) at `0008:000F5067` with
`UnsupportedSystem(LegacyInstruction)`. POST and boot remain unqualified.
See [machine files](design/machine-files.md#generic-386-isa-board) and
[firmware requirements](design/firmware-and-machine-classes.md#36-phase-b-firmware-requirements-seabios-on-generic-386-isa).

## Crate map and dependencies

The 18 workspace packages implement the PC core, its host tools and tests.
The root [`Cargo.toml`](../../Cargo.toml) owns the exact CPU and contract pins.
The desktop and command front end live in [unemu](https://github.com/unemu-org/unemu).

| Crate | Responsibility | Runtime dependencies | Test-only dependencies |
| --- | --- | --- | --- |
| `journal` | Emulation/core: version-1 disk/CMOS codec, scanner, sparse overlays and record-backed lifecycle, recovery and verified retention. `JournalSession` keeps host publication policy outside the assembler | `sha2`, `thiserror`, `unemu-core-api` | `unemu-core-api/test-support` |
| `journal-files` | PC/core host tools: verified file bases, journal path constructors over shared `FileRecord`, offline inspection/export/rollback, atomic image creation (`create`) and immutable sparse disk layers (`layer`). File sessions retain the same lock, recovery, retention and failure policy | `journal`, `sha2`, `serde`, `toml`, `thiserror`, `unemu-core-api`, `unemu-core-files`, `libc` (Unix) | `unemu-core-api/test-support` |
| `device` | Hardware-neutral contracts: `Device`, `InterruptController`, `DmaMemory`, `VClock`, `NextEvent`, access traces, DMA messages, and the observation capabilities (text scanout, byte output, `ByteCapture`) | `thiserror` | None |
| `cpu` | `Cpu`/`CpuStop` seam and `X86JitCpu`, which owns the core's `Vm` and `Vcpu` | `x86jit-core`, `thiserror` | None |
| `bus` | Owns boxed devices and executes a port/MMIO `DecodeMap`; open-bus policy and ticking | `device`, `thiserror` | None |
| `devices` | UART 16550, PIC 8259, PIT 8253, 8042, CMOS/RTC, DMA 8237, uPD765 floppy controller, VGA and MDA text cards, system control, POST port and Lo-tech EMS board | `device`, `thiserror` | `testkit`, `video` |
| `chipset` | Shared/cascaded PIC composition, null interrupt controller, AT port constants; no `Chipset` trait | `device`, `devices` | `testkit` |
| `machine` | Generic `Machine<C: Cpu>` and execution/clock/IRQ/DMA loop | `cpu`, `bus`, `device`, `thiserror` | `testkit`, `devices` |
| `video` | `FrameSink`, text frames/grids, text-mode renderer and capture sink | `device` | None |
| `assembly` | Emulation/core: `MachineConfig`, logical validation, presets, image/device construction, wiring, and `RunPolicy`; firmware catalogue and byte identity checks. `Mount` supplies bytes, verified bases and journal sessions, with opaque origin tokens interpreted only by the host. Inline configs assemble directly; host entry points live in `machine-file` | `machine`, `cpu`, `bus`, `device`, `devices`, `video`, `audio`, `chipset`, `journal`, `unemu-core-api`, `sha2`, `thiserror` | `journal/test-support`, `testkit`, `unemu-core-api/test-support` |
| `unemu-core-api` (external) | Not a workspace crate: a git dependency pinned to release v0.5.0 (contract 0.5) at `316dd2a1` of its own repository, [unemu-org/unemu-core-api](https://github.com/unemu-org/unemu-core-api) (`MIT OR Apache-2.0`). The contract between front ends and emulator cores ([core contract](design/core-contract.md) §3, §13.1): `Core` with its `CoreDescription`, the `Machine` a front end drives (run to a limit, picture and sound, input by USB HID usage, media slots, reset, saved state, finish, messages), the `CONTRACT` version and its `drives` rule; content and storage are objects the front end hands in (`ContentReader`, `Storage`, `Record`), never a path, and a core can identify a file a person opens. `std` only, object-safe, every type `#[non_exhaustive]` | None | None |
| `unemu-pc-core` | The PC core behind the contract (§4, §14.2.3): `PcMachine` wraps an assembled machine and runs it to tick deadlines, with the set-1 scan-code encoder and the 8042 back-pressure (`keyboard`), the pointing-device queue (`mouse`), the deadline-to-cap conversion (`pacing`), the CP437 text rasteriser and its CC BY-SA fonts (`raster`, `font`) and drive-activity sampling; `PcCore::validate` checks a launch document without content, and `create` / `create_machine` lower it through `pc-config` and assemble over contract content readers and storage records (`mount`); fixed MDA/VGA display order, run-only firmware permission and speaker trace, and the effective config through `PcMachine::config()`; offline media descriptions and streamed raw, fixed-VHD and schema-1 machine exports | `unemu-core-api`, `assembly`, `pc-config`, `audio`, `device`, `devices`, `machine`, `video`, `sha2`, `toml` | `unemu-core-api/test-support` |
| `pc-config` | Core-owned compatibility rows and evaluators (`compat`: machine keys, hardware vocabulary, accepted platform spellings and firmware catalogue, independent release/row types without an image-store dependency); the PC config codec below the contract ([core contract](design/core-contract.md) §14.2.1): the machine schema 1 (`schema`), its TOML decoder (`parse_toml`, `ParseError`), the source origins and their TOML half (`Origins`, `source`), the guest attachment names (`pc_names`) and the lowering to `assembly::MachineConfig` through assembly validation (`lower`, `lower_tracked`, `MachineFile::describe_with`), which asks its `ImageReader` for every image and everything of the host; and the stored PC image config (`image`: its model, decode and encode, its own `Digest` in the store's text form, the hardware-table encoding `image::encode` and frozen defaults `image::defaults`, and the codec half of validation `image::lower` over the store's `ContentRow`s and a contract `ContentSource`); and the launch document the PC core is to read (`launch`, `application/vnd.unemu.pc.launch.v1+toml`: the stored config plus a `content` table of each bound digest's flat SHA-256 and size and a `record` table of each journal's record and retention; `decode`, `encode`, `check` without content, and `lower`, which reads firmware and CMOS and names each disk base and journal by a key). Opens no file and reads no environment; `#![forbid(unsafe_code)]`. Behind `test-support`: the lowering's copy of the journal's retention-name rule and an empty diagnostics context, for the tests `machine-file` keeps | `assembly`, `unemu-core-api`, `serde`, `sha2`, `toml`, `thiserror` | None |
| `machine-file` | Machine description files on the host ([schema](design/machine-files.md#schema-field-by-field)): re-exports every name of `pc-config` at its old path and adds the YAML reader and the YAML half of the origins (`YamlOrigins`), `extends` and presets (`resolve`), journal path checks, host firmware discovery (`firmware_discovery`, with `frontend` and `persistent`, and the user-ROM helper behind `test-support`), machine-file firmware resolution (`firmware_resolve`), the seam of the firmware-package lookup (`FirmwarePackages`, which `pc-image` implements over the store; the prepared document keeps the packages' ids as text), the host's image reader (`FileImageReader`), `describe` over the host's file metadata (`Describe`) and the loaders (`load*`, `LoadedMachine`, `PreparedMachineFile`), `FileMount`, file-backed `assemble*`, host output validation and floppy append-by-path (`FileMachineExt`). Real-file assembly regressions live here | `assembly`, `pc-config`, `devices`, `cpu` (test support), `journal`, `journal-files`, `unemu-core-api`, `serde`, `sha2`, `toml`, `serde-saphyr`, `thiserror` | `machine-file/test-support` (self), `pc-config/test-support`, `assembly/test-support`, `machine` |
| `testkit` | Single-device `MockBus`, scripted `MockCpu`, golden assertions | `device`, `cpu` | None |
| `pc-harness` | Machine runners for the PC/AT goldens and the command line: hand-encoded guest programs, key scripts, assembly/policy selection, output capture and stop diagnostics (`StopReason`). Its `tests/` hold the emulation goldens (BIOS POST, FreeDOS boots and sessions, floppy, IDE, VGA, EMS, sound, the scheduler A/B tests) and `tests/support/`. Behind `test-support`, it exports `test_support::{fixtures, session, at_games, fat12}` for front-end tests; `test_support::root()` selects the core fixture checkout via `UNEMUPC_FIXTURE_ROOT` (an existing absolute directory), defaulting to its own repository | `machine-file`, `journal-files`, `assembly`, `device`, `cpu`, `chipset`, `machine`, `devices`, `video`, `anyhow` | `bus`, `testkit`, `toml`, `machine-file/test-support`, `machine/reference-scheduler`, `assembly/reference-scheduler`, `assembly/test-support` |
| `unemupc` | The standalone PC core command line ([core contract](design/core-contract.md) §12.8): library parser, machine usage lines, dispatch, runner and core-only notices; binary `unemupc` offers `--boot`, `--bios`, `machine check`, `run --machine`, `--version`, `--about`, and `media generations`, `verify`, `rollback`, `commit` (alias `compact`). No store or registry. `execute_with_loader` shares execution with the frontend while leaving its store-backed preparation in `cli-pc` | `pc-harness`, `machine-file`, `unemu-pc-core`, `unemu-core-api`, `assembly`, `machine`, `device`, `video`, `anyhow`, `journal-files` | `cpu`, `assembly/test-support`, `pc-config`, `toml`, `unemu-core-api/test-support` |
| `unemupc-view` (`crates/view`) | Application: bare winit picture window over `machine-view::Session`; softbuffer nearest sampling and letterboxing, host controls, clean finish. Reuses the standalone loader and machine-file assembler; no GUI toolkit | `unemupc`, `machine-file`, `assembly`, `unemu-pc-core`, `unemu-core-api`, `machine-view` (exact unemu-host pin), `winit`, `softbuffer`, `anyhow` | `unemu-test-core` (same pin), contract and session test support |
| --- | --- | --- |

| Layer | Workspace crates | May depend on |
| --- | --- | --- |
| `emulation` | `device`, `bus`, `cpu`, `video`, `audio`, `devices`, `chipset`, `machine`, `testkit`, `assembly`, `journal`, `unemu-pc-core`, `pc-config` | `contract`, `emulation` |
| `pc` | `unemupc`, `machine-file`, `journal-files`, `pc-harness` | `contract`, `emulation`, `pc` |
| `application` | `unemupc-view` | `contract`, `emulation`, `pc`, `application` |

Only the application links window and host-audio crates. `machine-view`
comes from `unemu-host` at `1692a1fcfd6c9900d3074786a54f51c9de3b30f0`,
sharing this workspace's contract 0.5 pin. Host elapsed time paces the session;
it never enters the emulation layer. The window reuses `unemupc::run`'s
loader, including resolved names and firmware warnings, then calls
`LoadedMachine::assemble` and wraps `PcMachine` as `dyn Machine`.

The contract is external. `scripts/crate-boundaries.sh` checks every declared
dependency kind, the exact external-crate list and one resolved copy of
`unemu-core-api`. Source includes must remain inside the repository.
`scripts/core-host-guard.sh` keeps host interfaces out of emulation crates.
Concrete hardware selection belongs to `assembly`; the generic machine loop
uses the CPU, device and interrupt-controller seams.

The PC adapter follows the contract's call/status matrix (0.4, unchanged in 0.5): successful finish
enters `Finished`, terminal mutations return `Usage`, and input validates
advertised ports before queueing. Runs accept an unbounded request, apply input
before already-reached limits, and return immediately in `Halted` or `Off`.
Soft reset accepts `Ready` and `Halted`, clears host input and held controls,
and preserves time, absolute steps and inserted media. Counter exhaustion
fails with `Guest` before another instruction can overflow the counters.
Floppy selection completes its opens before changing the insertion; catalogue
additions and saved state remain unadvertised.

Journal tools and file-backed sessions use only the path-record lock domain
(`FILE.lock`). A host supplying named `FileStorage` records must keep those
files outside all path-record/tool access, including aliases: the named lock
(`FILE_lock`) does not exclude a path lock. Named record filenames use the
contract's reversible escaping, including sidecar-shaped names. Recovery scans
current bytes after ordinary reopen, cuts invalid append tails and synchronizes
before resuming; reopening never promises rollback of destructive cuts.
Retention uses replacement, whose parent barrier precedes publication. Disk
bases remain read-only, fully SHA-256 identified and separate from overlays.

`assembly`, `machine-file`, `unemu-pc-core` and `pc-harness` expose
`test-support` features used by the front end's tests across its exact pin.
`pc_harness::test_support::root()` uses `UNEMUPC_FIXTURE_ROOT` when supplied
(an existing absolute directory), otherwise this checkout. The front end
reads core `machines/` and `vendor/` fixtures through that API; fixture pins
and build scripts have one home here.

## From configuration to a run

`machine_file::load` reads a complete TOML or YAML machine description, expands the
assembly-owned board data and card defaults, resolves images against their
defining files, checks optional SHA-256 pins, and retains the bytes inline.
It derives the RAM mosaic and calls `MachineConfig::validate()` before returning.
The reference file is `machines/at-freedos.toml`; callers may also use Rust presets.

`assembly::descriptors` owns the implemented card names, video decodes, RAM
and resource domains, effective defaults, named floppy geometries and firmware
slots. EMS domains reference the device model's public jumper/population
constants, so the runtime constructor and configuration cannot drift.
Machine-file parsing/lowering consumes these definitions. The
transport-neutral `assembly::capabilities` API describes fields, pages large
integer domains, offers choices and validates a lowered `Draft` with atomic
pending edits. Paths use card identities (`card[sound].irq`) and drive units,
with optional source-level card names and image-field paths supplied by the
caller. Image paths exclude the system BIOS when numbering option ROMs.
Port descriptors expose IRQ dependencies; edits follow a changed port's derived
IRQ default when the old default is still selected, preserving explicit IRQ edits.
Choices retain unrelated existing errors as diagnostics without disabling other
fields, so staged resource swaps remain editable. Responses echo
the caller's revision so a frontend can discard stale choices.

`validate_draft` shares the ordered checks behind `MachineConfig::validate`,
collecting one error per independent check. Diagnostics include stable codes,
field paths, conflict partners, decoded ports/routed IRQs/DMA ownership and
severity. Candidate edits replace existing resources and rebuild derived
port/MMIO/RAM maps; several fields are applied before checking conflicts.
Selected firmware uses catalogue requirements and sizes for placement checks,
while absent bytes remain pending. The known June 1985 IBM BIOS drive-type
restriction is exposed to drafts; legacy lowering's first-error behavior and
source-origin mapping remain unchanged. No host image reads, UI types or
execution clocks enter the API. Launch must still load/verify inputs and run
assembly validation, including persistent-path checks. `Validation::ready`
means no reported error or pending check, not permission to bypass assembly.

Host firmware discovery lives in `machine_file::firmware_discovery`. Its `frontend`
module reuses one listing for the union of BIOS and option-ROM pins in a check, enforces the same
size bounds on declared paths, and hashes only eligible files. The `persistent`
module supplies a scoped host-worker session to existing machine-file resolver
calls: an explicit cache path, metadata-validated SHA-256 index, cancellation flag
and bounded progress/result snapshots. No guest configuration or execution state
contains cache settings. Tests explicitly disable persistence or use temporary
cache paths. A host caller polls snapshots without waiting for the worker to join. Non-racy cache hits avoid
candidate file reads, while the ordinary loader still verifies the selected ROM
bytes before launch. See [machine descriptions](../../machines/README.md#selecting-a-machine)
for local cache locations, privacy and Rescan/Cancel behavior.

The normal real-mode path is:

```text
MachineConfig (preset or caller data)
  -> assemble(&config)
  -> AssembledMachine { Machine<X86JitCpu>, device_index, observation }
  -> RunPolicy + assembly::run(&mut machine, &observation, &policy)
  -> Machine::run_bounded(...) -> output capture / stop classification
```

[`MachineConfig`](../../crates/assembly/src/machine.rs) describes CPU/reset state,
clock rate, RAM/MMIO layout, devices, drives and loadable images.
[`assemble()`](../../crates/assembly/src/assemble.rs) validates
it before construction, maps CPU memory, loads images, creates devices in config
order, derives CMOS contents from memory/drives, and resolves device roles into
bus indices. It builds the decode map, IRQ lines and DMA couplings. One shared
master/slave PIC pair backs both bus faces and the interrupt-controller seam;
a configuration without a PIC receives a null controller.

[`RunPolicy`](../../crates/assembly/src/run.rs) describes how to drive that machine:
virtual-time cap, halted idle limit, natural stop or console-prompt polling, and
keyboard stimuli after the prompt. `run()` applies the idle limit and drives
`run_bounded`; prompt polling captures a bound `DeviceId` display route through
`ConsoleProbe` and the observation facade. Policy cadence affects guest execution and golden identity.
The assembled machine starts without an idle wait. Some simple runners call
`Machine::run()` directly. The assembler constructs Real16 machines and rejects a CPU without step-bounded
execution. The Long64 machine demo has been intentionally removed.

### Current execution order

The authoritative loop is
[`Machine::run_bounded`](../../crates/machine/src/lib.rs). For a running CPU:

1. Settle instruction effects, check the absolute virtual-time cap, then
   acknowledge a serviceable PIC request only if `Cpu::irq_ready()` permits it.
2. Bound `Cpu::run_steps` by the next device deadline, remaining cap and host
   cooperation limit. A pending INTR requests a stop at the first eligible
   boundary, respecting IF and STI/SS-load inhibition.
3. Charge completed prefix steps, dispatch the stopping port/MMIO access at
   instruction-start time, and finish pending read/write handshakes. Each REP
   iteration is a step, so long string instructions advance device time.
4. Charge the completed stopping step once, settle bus/DMA effects and sample
   wired IRQ transitions before the next acceptance check. Errors retain the
   completed prefix's device time. Exact active caps win over a new INTA.

Production uses deadline dispatch. Devices without a deadline implementation
retain per-clock visits, while scheduled devices synchronize lazily on access.
The non-default `reference-scheduler` feature exposes per-clock dispatch to
integration tests as an oracle with the same CPU/IRQ rules. No production
`MachineConfig` selects it, and no legacy outer loop remains.

[`Bus`](../../crates/bus/src/lib.rs) dispatches port cycles according to each
window's `PortWidth`: `Byte` by default, or `Word` for IOCS16# devices. An even
port in a word window receives one size-2 call for a word access. Width is
selected by the cycle's starting port, so a single-port word window consumes
both data bytes even if the next port decodes to a byte device. Odd word accesses
use two byte cycles. An aligned 32-bit access to word windows uses two size-2 cycles at
port and port+2; each new cycle independently selects its window and width.
Mixed windows therefore use byte or word cycles as selected, low bytes first,
with port arithmetic wrapping at 16 bits. A fault preserves completed cycles.
Device traces record those actual cycle sizes. This models the IBM Personal
Computer AT Technical Reference, Section 1, System Board, I/O Channel,
I/O Channel Signal Description, -I/O CS16 (IOCS16#); wait states and physical
byte-lane signalling remain outside the model. Existing presets retain byte
windows. Assembly checks word windows against a kind-based register support
table, currently empty; ATA card wiring will declare its word data register.
Port dispatch resolves the device and cycle width in one first-match lookup.
See [Port window width](design/machine-config.md#port-window-width).
Undecoded ports default to
all-ones reads/dropped writes; optional bounded tracing and strict faulting are
selectable. Assembly coalesces touching MMIO windows into one CPU trap region
while retaining per-device bus decoding. MMIO accesses contained in one decode
window reach the device once with their original size. Accesses crossing windows
are split into contiguous sub-accesses in ascending address order, with
little-endian read composition and write slicing. An undecoded MMIO address
faults immediately: earlier sub-accesses remain completed and later ones are not
attempted. Read results use `complete_port_in` or
`complete_mmio_read`; trapped writes require `complete_mmio_write` so retrying
the instruction does not repeat the device side effect.

While halted, the loop stops immediately with IF clear (NMI is not modelled).
With IF set it first tries IRQ service; otherwise, subject to idle/cap bounds,
it jumps to the next device event, idle expiry or cap, settles DMA and samples
edges before retrying IRQ acceptance. A queued IRQ permits CPU re-entry. Halt state and accumulated idle time
survive bounded runner calls.

## Contracts and ownership

- **[`Cpu`](../../crates/cpu/src/lib.rs)** exposes `run_steps`, completed-step
  and whole-instruction counters, general-purpose registers/RIP, port/MMIO
  completions, IF and IRQ readiness, IRQ injection/pending status and DMA byte
  access. It has no segment-register API,
  bus-binding API or `irq_ack()` method. Segment seeds and `code_segment()` are
  concrete `X86JitCpu` setup/inspection APIs.
- **[`Device`](../../crates/device/src/lib.rs)** supplies port/MMIO access,
  `tick(VClock)`, IRQ level queries, reset, per-device save/restore, trace
  drains and optional DMA request/service/completion hooks. Host-side
  `as_any_mut` is an optional downcast for input/output glue, not execution-loop
  hardware selection. `NextEvent::{EveryPitClock, At(VClock), Dormant}` is an
  scheduling contract consulted by the production deadline loop; `next_event`
  defaults to `EveryPitClock` for unmigrated devices. Timestamped access wrappers
  synchronize the target device at instruction start; deadline dispatch settles
  due transitions without ticking dormant devices at each CPU boundary.
- **[`SectorMedia`](../../crates/device/src/media.rs)** is owned by each uPD765
  drive and covers its configured geometry. It serves 512-byte sectors, accepts
  one ordered batch of replacements per nonempty completed WRITE DATA, and
  reports write protection that remains constant while mounted. Partial sectors
  are zero-filled by the controller; abnormal completions include sectors already
  committed. The contract exposes no clock or host error result. Default drives
  use `InMemoryMedia`; reset retains the medium and device snapshots exclude it.
  The host-side `journal_files::JournalMedia` owns reconstructed bytes, synchronous
  per-command output, recovery and checkpoint/finish under a stable sidecar lock.
  Its shared `SinkStatus` latches the first failure outside guest-visible state;
  reads and later memory writes continue after output stops. Assembly adapts
  journals to `SectorMedia` and exposes per-drive persistence handles. CLI and
  window frontends poll status at run boundaries and explicitly finish normal
  sessions; execution errors leave the tip unsealed.
  See [persistent media](design/persistent-media.md#integration-layers-flush-points-determinism-argument).
- **`InterruptController`**, defined in `device`, supplies `set_irq_line`,
  non-mutating `poll`, `acknowledge`, and `end_of_interrupt`. Machine wiring
  forwards rising and falling device pin levels to PIC inputs; the PIC detects
  edges and withdraws unacknowledged requests when their inputs fall.
  Acknowledgement requires CPU IRQ readiness and no already queued vector,
  after the stopping instruction's bus/DMA effects have settled. Guest EOI port
  writes reach the same shared PIC state. See the adopted ordering in
  [scheduler design](design/scheduler.md#adoption).
  Known limitation: cascaded-board validation does not reserve master IRQ2 for
  slave INT, so a device route can still conflict with that internal connection.
- **[`FrameSink`](../../crates/video/src/lib.rs)** consumes `&Frame` via
  `present`; the current frame is `Frame::Text(TextGrid)`. Rendering turns VRAM
  and display-start/cursor state into cells with glyph bytes and attributes.
  `CaptureFrameSink` retains the last frame. Presentation is driven by the assembly observation facade at capture
  boundaries, outside the machine loop.

The bus owns devices; `X86JitCpu` owns guest memory. Devices do not retain RAM
references. DMA requesters supply a channel and payload/length, the configured
controller supplies addresses/counts, and the machine lends a temporary
`DmaMemory` adapter over the CPU seam. `DmaOutcome` returns a transfer count or
the bytes read, truncated to the controller's actual transfer length.

### Snapshot clock and interrupt state

Device restore keeps the current trace history, drop counts and recording policy.
A restored bus clock must be at least as late as each device snapshot's absolute
clock; devices ignore earlier sync/tick times to avoid rewinding or replaying an
interval. Restoring an earlier epoch requires restoring its global clock too.

A future machine-level snapshot must also save `Machine::last_levels`, the IRQ
IRQ sampler's last observed levels. No device snapshot contains these samples;
losing them can suppress or invent an interrupt edge after restore. CPU state,
the global clock and its fractional remainder, halted state and idle counters
also belong to that machine-level snapshot.

### Observation and presentation

Assembly owns the observation routes. The contract adapter in `unemu-pc-core`
turns device output into frames, sound, input ports and media slots. It accepts
content and storage through the contract; host files belong to `machine-file`,
`journal-files` and `unemu-core-files`. Desktop presentation and host audio
output live in [unemu](https://github.com/unemu-org/unemu).

`PcCore::describe_media` and `export_slots` check launch documents without
opening content or creating a PC. The platform advertises `raw` (`.img`, one
slot), `vhd-fixed` (`.vhd`, one fixed disk) and `machine` (`.toml`, all inserted
media). Exports read the front end's current-byte readers in 64 KiB chunks;
bases, overlay layers and journals stay read-only. Slot and length checks
precede output creation, and canonical drive order makes request order
irrelevant. Fixed VHDs preserve configured CHS, use timestamp zero and derive
their UUID from the full current-byte SHA-256. The machine writer retains the
schema-1 hardware tables, refers to relative sink output names with the sink's
SHA-256 receipts, defaults media writes to volatile, and names firmware by its
SHA-256 for the existing loader's firmware discovery, without copying ROMs.
Reports name omitted catalogue entries and custom labels, replaced media
identities/write policies, omitted journal records/retention and NVRAM images,
and firmware source-archive provenance. Raw/VHD reports also list omitted
machine settings. The front end owns output cleanup on failure.


### Core compatibility rows

`pc_config::compat` owns the PC config and launch-document requirements and
`NEXT_RELEASE`. Its marker `unemu.min-app-version` is compared with the PC
core's release (`unemu-pc-core`'s package version). The front end owns
`unemu.min-frontend-version`. Existing images retain their markers across the
split; see [decision-30](../decisions/decision-30%20-%20Published%20images%20declare%20the%20oldest%20unemupc%20that%20runs%20them.md).

## Virtual clock

[Decision-5](../decisions/decision-5%20-%20Virtual%20clock%20in%20retired%20instructions.md)
sets retired instructions as the deterministic time source; block counts and
wall time do not define device time. `VClock(u64)` counts virtual ticks;
`device::clock::TICKS_PER_PIT_CLOCK` remains **1**. `VClock::from_pit` converts
PIT input clocks at unit boundaries. The `device` crate owns clock domains
because it already owns the common device-time seam; no additional crate or
runtime dependency is needed.

`device::clock::OSC` is exactly **315/22 MHz**. Named `ClockDomain` constants
cover PIT (OSC/12), CGA horizontal dots (OSC), VGA dots (25.175/28.322 MHz),
UART (1.8432 MHz), RTC (32.768 kHz), OPL3 samples (OSC/288), 48 kHz output,
and AT 12/16 MHz CPU crystals (6/8 MHz 286 after division by two). The CPU
crystals describe hardware frequencies, not the calibrated instruction rate.
A domain stores an exact ratio of virtual ticks per edge. `edges_between(a,b)`
counts edges in `(a,b]`; `edge_at(n)` rounds up once; `next_edge_after(t)` uses
the absolute epoch to retain phase, including when several fast edges share a
tick. Checked edge methods report exhaustion; duration conversion saturates.
Devices keep local reset/programming origins where the existing model anchors them.
Audio integration remains in virtual-tick areas. PIT counters advance by input
edges, while deadlines, UART/keyboard durations and the VGA clock conversion use
domains. The public `VClock` field and existing stateless VGA raster remain
unchanged by this seam (decision-22 §1a–b).
[`ClockConfig`](../../crates/assembly/src/clock.rs) supplies the positive rational
CPU rate A/B (`retired_per_pit_clock` / `retired_per_pit_clock_denominator`), in
completed steps per PIT clock. Assembly passes
the rate into `Machine::with_cpu_rate`; the default is 1/1. Running time emits
`floor((remainder + completed_steps_delta * B * ticks_per_PIT) / A)` virtual ticks and carries the remainder
across CPU exits. This is an instruction-timed approximation, not cycle timing.

[Decision-14](../decisions/decision-14%20-%20Virtual%20time%20advances%20while%20the%20CPU%20is%20halted.md)
extends time through HLT: halted clocks tick devices without retiring CPU
instructions or consuming the running clock remainder. `Machine::virtual_time`
is completed steps plus `floor(total_halted_ticks * A / (B * ticks_per_PIT))`, saturating
at `u64::MAX`. Caps and policy targets use these instruction equivalents;
device clocks use `VClock`. Golden runs depend on the CPU revision, config and
policy cadence. Host cooperation size cannot change timing by contract;
device-free CPU regressions and the per-clock-versus-deadline comparisons hold
the CPU core to it (see [scheduler adoption](design/scheduler.md#adoption)).

## x86jit integration

[`X86JitCpu`](../../crates/cpu/src/x86jit_backend.rs) adapts a pinned external
`x86jit-core`; machines use its Real16 interpreter. Long64 remains an
adapter unit-test concern only. CPU changes are developed upstream and consumed by re-pinning,
not copied into this workspace
([decision-13](../decisions/decision-13%20-%20Consume%20x86jit%20main%20at%20an%20exact%20revision.md)).

- `Exit::PortIo`, `MmioRead`, `MmioWrite`, `Hlt`, `Exception` and
  `BudgetExhausted` map to `CpuStop`. `UnmappedMemory` and `UnknownInstruction`
  become typed CPU errors; `Syscall` is unsupported. Host interruption, code
  space exhaustion and unrecognized exits become `UnexpectedExit` errors.
- The memory mosaic uses `RegionKind::Ram` (RWX), `Trap` (MMIO), and `Rom`
  (RX, executable/readable with guest stores dropped). Host image loading uses
  `Vm::write_bytes`, which can populate ROM. Real-mode `INT n` already vectors
  through the guest IVT; it is not an outstanding embedder implementation.
- The Lo-tech EMS board owns its four latches and SRAM behind `Device` and a
  64 KiB MMIO frame, and shares them with assembly through a `LotechEmsFrame`
  handle. Port writes update the latches synchronously before the next CPU
  instruction. Every 400h I/O alias is represented in the decode map and
  conflict validation. The IBM 5170's E0000h–FFFFFh system-board ROM decode
  rejects an E000h frame; the shipped profiles use D000h. Only Rust bare test
  rigs accept E000h. The normal high ROM alias is unchanged. CPU reset
  preserves latches and SRAM.
  See the coverage matrix in `crates/devices/src/lotech_ems.rs` and
  [machine fields](../../machines/README.md#lo-tech-ems-card-reference).
- The CPU cannot fetch instructions from a `Trap` region, so assembly maps
  each EMS frame window in one of three forms
  (`crates/assembly/src/ems_frame.rs`). A window whose latch selects a
  populated page that no other latch selects is `Ram`, and its CPU copy is
  authoritative. An unbacked window is `Rom` filled with FFh: reads and
  fetches see the floating bus, guest and DMA stores are dropped. Only a page
  selected by two latches stays `Trap` and reaches the board, data-correct but
  not executable. `Machine::with_memory_map` calls the map when every run
  starts and after every guest port write, before that instruction's DMA and
  the next instruction. It writes held copies back into SRAM (all of them
  before a retile), retiles the frame with `X86JitCpu::unmap` and
  `map_ram`/`map_rom_in_place`/`map_mmio` (neighbouring windows of one form
  share a region; the DMA region table follows; the core's flat backing keeps
  bytes across unmap and map), then loads newly held pages, or the FFh fill of
  a newly unbacked window, with `Vm::write_bytes`, whose `note_write` drops
  any translation of the previous contents. Its change counters advance last,
  so a failed apply is retried whole. When a run returns, held pages are
  written back, so device snapshots read current SRAM; a restore makes the
  next run drop the held copies unwritten and reload them. The core joins RAM
  and ROM regions for one operand, so a word straddling a held and an unbacked
  window reads FFh for the unbacked byte; an operand crossing a `Trap` region
  (a shared window, or another device's window beside the frame) and a held
  or unbacked window stops with `CpuError::Unmapped`. A latch
  write costs one or two 16 KiB copies; a retile, whose `Trap` map invalidates
  the whole translation cache, happens only when a window changes form.
  The write-back on every run return copies at most 64 KiB. MEASURED once in
  release on the development host: 1 ms run chunks (1,193 steps) of a spinning
  guest took 26.2–26.6 µs per return with no held window and 26.9–27.3 µs with
  four, under 1 µs and about 3% of the chunk, so held windows carry no
  per-window dirty tracking.
- `inject_irq` forwards a queued vector; the core gates delivery on IF, pending
  completions and the STI/SS-load shadows, then enters the guest IVT handler.
  `completed_steps()` drives machine time; `retired()` remains available for
  whole-instruction diagnostics.
- DMA byte access uses `Vm::read_bytes` / `write_bytes`; writes run `note_write`
  for self-modifying-code invalidation
  ([decision-6](../decisions/decision-6%20-%20Guest%20memory%20writes%20via%20note_write%20for%20SMC%20safety.md)).
  The backend remembers the regions it mapped: a DMA write into ROM is
  dropped and a DMA read from ROM returns ROM bytes, while a byte in a trapped
  MMIO window is handed back to the machine (`DmaAccess::Mmio`), which runs it
  as a bus MMIO cycle on the owning device. Read errors return `0xFF`; write
  errors are discarded by the current byte seam.

## In flight and further reading

- **Board validation:** [`design/compatibility-diagnostics.md`](design/compatibility-diagnostics.md)
  records the diagnostic-ROM spike, user-supplied firmware policy, expected
  Landmark AT coverage and the golden capture rule; execution is not qualified.
- **Scheduler:** [decision-15](../decisions/decision-15%20-%20Deterministic%20device%20scheduler%20with%20instruction-exact%20interrupt%20boundaries.md)
  amends [`design/scheduler.md`](design/scheduler.md). Instruction-boundary
  timing and deadline dispatch are adopted. Per-clock dispatch survives only as
  a feature-gated test oracle; the harness compares ordered accesses, INTA/EOI,
  RAM, registers, saved device state and stop endpoints without normalization
  between the two engines. The post-adoption profile is in
  [`design/machine-time-profile.md`](design/machine-time-profile.md).
- **Observation:** [`design/observation.md`](design/observation.md) describes
  typed device capabilities and assembly-owned observation routes. Access traces
  are opt-in, bounded and independent of semantic output; the document retains
  the historical inventory. Prompt polling, final capture and presentation now share one route-based facade.

[`design/machine-config.md`](design/machine-config.md) explains composition;
[`design/spec.md`](design/spec.md) specifies hardware coverage and seams;
[`design/machine-time-profile.md`](design/machine-time-profile.md) records timing
measurements. Historical decisions are under [`../decisions/`](../decisions/);
accepted designs may precede implementation. Deferred hardware and fidelity
work is listed in [`deferred.md`](deferred.md).
