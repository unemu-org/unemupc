# Machine description files

Status: **accepted** (decision-17). Schema 1 TOML and YAML parsing and lowering
to an assembly-validated `MachineConfig` are implemented in `crates/machine-file`.
`extends` and embedded presets are implemented, including per-value origins,
per-layer image directories and completeness checks. IDE PIO channels and
volatile/readonly hard disks and explicit floppy/IDE journal attachment are implemented.
NVRAM images support volatile writes and reset/session journals; readonly is rejected.
Drive-indexed assembly diagnostics, including journal configuration errors, map
to the drive's source origin and unit label while preserving the message suffix.
Journal errors are mapped exhaustively from assembly validation.
The window, CLI and FreeDOS goldens load machine files. Frontend assembly
retains file/line origins for byte-dependent firmware catalogue and journal
mount errors through `machine_file::load_for_assembly`.
The design answers one question: how is a `MachineConfig` written as a file,
so that users compose machines without recompiling?

Related: [machine-config.md](machine-config.md) (the Rust data model a file
lowers to), `firmware-and-machine-classes.md` (board models and firmware
compatibility, a parallel spike), `persistent-media.md` (the journal write
mode, a parallel spike), decision-7 (swap seams, the data-plus-assembler
annex), decision-16 (scope; the phase A′ amendment asks for description files).

Evidence labels:

- **MEASURED**: run during this spike on 2026-11-10 with rustc 1.95.0, in a
  scratch crate outside the repository.
- **READ**: source, documentation or registry metadata read on 2026-11-10.
- **ASSUMED**: not verified; each one is listed where it is used.

## Context

The original machine definitions were Rust presets (`assembly::bios`,
`assembly::at_boot`, …). Machine files now supply the window, CLI file commands
and FreeDOS goldens; small synthetic test rigs retain Rust presets.
The phase A′ amendment of decision-16 adds recompile-free hardware composition
through one versioned schema and two syntaxes.

The target shape was agreed with the maintainer on 2026-11-10 (bead notes). On
the same day a scope change replaced the top-level `class` with a **board
model** (`[board].model`, named the way 86Box and PCem name boards). This
document uses that shape:

1. **One serde data model, two syntaxes.** TOML and YAML deserialize into the
   same Rust types. Every key and value is spelled the same in both syntaxes.
2. **`schema = 1`** is required in every file.
3. **The board model supplies the motherboard.** `[board].model` (for example
   `"ibm-5170"`, the generic AT) supplies the motherboard devices, their
   standard wiring and any chipset registers. The machine class (xt or at)
   is derived from the board model. Card defaults and validation depend on the
   board model. The list of board models and firmware compatibility belong to
   the firmware spike.
4. **Everything in an ISA slot is a `[[card]]`.** Each card kind has default
   ports, IRQ, DMA and MMIO. A file can override them where the modelled chip
   allows it.
5. **The file describes the machine only.** RunPolicy (limits, stop
   conditions, input scripts) and presentation (window, scale, display choice)
   stay outside the file. Host-connected endpoints are explicit streams that
   the run side binds.
6. **Paths are relative to the file that names them.** Images may pin a
   `sha256`.
7. **`extends`** inherits from another file or from a built-in preset.
8. **Schema 1 includes IDE, floppy journals, NVRAM, RTC start and the optional
   PS/2 or Microsoft serial mouse.** Other names and shapes are reserved. New hardware uses new
   card kinds within the same format version.
9. **Cycle-exact timing fields are rejected** with an explanation
   (decision-16 §3).

## Inventory

How every current `MachineConfig` field and every `DeviceKind` parameter is
written in a file. "Board" means the board model supplies the value, "type"
means a card-kind default, "user" means the file states it. Model changes are
numbered **M1–M8** and listed after the table.

| Rust field (`crates/assembly`) | File form | Source | Model change |
|---|---|---|---|
| `cpu.chip` (`CpuChip::I80286`) | `[cpu] model = "80286"` | board default, user may restate | — |
| `cpu.reset` (`ResetSpec`) | none | board: `ResetVector F000:FFF0` for every firmware board | — (`Entry` stays for Rust test rigs) |
| `cpu.segments` (`SegmentSeed`) | none | board: `SegmentSeed::flat(0xF000)` | — |
| `clock.retired_per_pit_clock`, `…_denominator` | `[cpu] rate = "A/B"` | user, default from `BoardModel::default_clock` (`8/13` on `ibm-5170`) | — |
| `memory.address_space_bytes` | none | derived: 1 MiB + `extended_kib` | — |
| `memory.ram` (`RamRegion` list) | `[memory] conventional_kib`, `extended_kib` | derived by the RAM rule (see [`[memory]`](#memory)) | — |
| `memory.mmio` (`MmioWindow` list) | none | type: each display card owns its window | — (expressible today) |
| `devices[]` for PIC ×2, PIT, 8237, 8042, RTC, port 61h | none | board (`ibm-5170` table below) | — |
| `devices[]` for POST port | `[board] post_port` | board default `0x80`, user may set `"none"` | — |
| `devices[]` for UART, VGA, MDA, FDC | `[[card]] kind = …` | user | — |
| `DeviceConfig.id` (`DeviceId`) | none | derived from the role or card kind; the file-level card `id` is a unique merge key and diagnostic name, not a `DeviceId` | — |
| `DeviceConfig.ports` | `[[card]] port` (base) | type default, user override where relocatable | — |
| `DeviceConfig.irq` | `[[card]] irq` (`0..15` or `"none"`) | type default, user override | — |
| `DeviceConfig.dma` (`DmaSpec`) | `[[card]] dma` (`0..3` or `"none"`) | type default, user override | — |
| `DeviceConfig.access_dependents` | none | board (empty today) | — |
| `DeviceConfig.allow_port_overlap`, `PortWindow.allow_overlap` | none | none | — |
| `UartCfg.base` | `[[card]] port` | type default `0x3F8` | — |
| `PicCfg.cascade`, `PitCfg`, `I8042Cfg`, `SysControlCfg`, `DmaCfg` | none | board | — |
| `PostPortCfg.port` | `[board] post_port` | board default | — |
| `CmosCfg.seed` | `[board.rtc] nvram = "derived"` or `"blank"` | user, default `"derived"` | **M7** |
| `CmosCfg.timing` | none | not writable: a test-rig knob (the RTC demo) | — |
| `VgaTextCfg`, `VgaCfg`, `MdaTextCfg` | `kind = "vga-text"`, `"vga"`, `"mda"` | user | — |
| `FdcCfg.drive_slots` | none | type: 2 units | — |
| `drives[].kind` (`Floppy { geometry }`) | `[[card.drives]] type = "1.44M"` | user | — |
| `drives[].kind` (`HardDisk { geometry }`) | `[[card.drives]] geometry = { … }` on an `ide` card | user | **M5** |
| `drives[].image` (`ImageSource`) | `image` + optional `sha256` | user; volatile/readonly retain verified `Inline` bytes; journals retain `File` plus `DriveConfig.expected_sha256` | — |
| `drives[].controller_slot` | nesting under the card + `unit` / `channel` | derived | **M4** |
| `drives[].write_protect` | `write = "volatile" \| "readonly" \| "journal"` | user, default `"volatile"` | **M3** |
| `images[]` with `ImageKind::Rom` | `[firmware] bios`, `option_roms` | user | **M6** (`provides`) |
| `images[]` with `ImageKind::RamLoad` | none | not writable: test rigs only | — |
| `output` (`OutputConfig`) | none | moved to `ObservationPolicy.output`; presentation stays outside the file | **M8** (implemented) |
| (new) board model | `[board] model` | user, required | **M1** |
| (new) RTC start | `[board.rtc] start` | user, default `"2000-01-01T00:00:00"` | **M7** |
| (new) NVRAM image | `[board.rtc] nvram = { image, … }` | user | **M7** |
| (new) canonical device order | none | derived: board devices, then cards in file order | **M2** (data only) |

Model changes the implementation must make:

- **M1. Board model in the config.** Add `MachineConfig.board: BoardModel`,
  with variants `Ibm5170` and `Bare`. `Bare` is for the Rust test rigs
  (`at_boot`, `test386`, `floppy_boot`, …), which skip board-dependent rules.
  Validation depends on the board model, so the model must be in the data that
  `validate()` sees. It also becomes part of golden identity.
- **M2. Canonical device order.** Bus order is semantic: the bus ticks due
  devices "in bus order at one whole PIT boundary" (READ, `crates/bus/src/lib.rs`).
  The `bios` preset used to interleave cards and board devices; it now
  lists the board devices in the board's fixed order, then the cards, and
  lowering emits the same order. Rewriting the preset was proven neutral by
  the slow goldens and a byte-identical A/B (bead 1). ASSUMED: no golden depends on the
  current tie order. If one does, bead 1 stops and reports it.
- **M3. Write modes.** Replace `DriveConfig.write_protect: bool` with
  `write: WriteMode { Volatile, ReadOnly, Journal { path, keep_generations } }`.
  `false` maps to `Volatile` and `true` to `ReadOnly`. The journal variant
  follows `persistent-media.md`.
- **M4. Drive binding by controller.** Replace the global `controller_slot`
  with `controller: DeviceId` plus `unit: u8`. Today
  `check_drive_controllers` sums FDC slots only, so a `HardDisk` on slot 0
  passes whenever an FDC exists. With an IDE controller present, the binding
  must name the controller.
- **M5. IDE.** Add `DeviceId::Ide`, `DeviceKind::Ide(IdeCfg { base, control })`
  and the device itself. **Implemented:** one primary or secondary PIO channel,
  word data and byte task/control registers, IRQ14/15, and master/slave media
  through `SectorMedia`. The assembler routes `HardDisk` drives by controller
  and unit, with geometry and exact image-size validation. This is the seam that
  [machine-config.md](machine-config.md#the-fixed-disk-idehdd-attachment-seam)
  already reserves.
- **M6. Firmware `provides`.** Add `LoadImage.provides: Vec<FirmwareService>`
  (v1 values `Floppy`, `FixedDisk`), so `validate()` can reject an IDE card that
  no ROM serves.
- **M7. RTC start and NVRAM source.** Replace `CmosCfg.seed: bool` with
  `nvram: NvramSource { Derived, Blank, Image, Journal }`. Add
  `start: RtcStart`, whose default equals today's epoch, 2000-01-01 00:00:00
  (READ, `RtcTime::default_epoch`). Keep `timing`.
- **M8. Presentation out of the machine (implemented).**
  `ObservationPolicy.output` selects frame sinks at assembly time, defaulting to
  Headless. `MachineConfig` describes hardware only; presets and machine-file
  lowering no longer select presentation. Inline hosts retain
  `Observation::configure_output`. Capture boundaries and golden output are
  unchanged.

Not a model change: card-owned MMIO windows. `MemoryConfig.mmio` already names
the owning `DeviceId`; lowering derives the windows from the card kinds.

## Schema (field by field)

### Document rules

- **Syntax by extension:** `.toml` is TOML, `.yaml` and `.yml` are YAML.
  The name supplied by the caller selects the syntax, including a symlink's
  name rather than its target's extension. Any other extension is an error.
  A file contains exactly one document.
- **Keys** are `snake_case`. **Enumerated values** are `kebab-case` strings.
  **Integers** may be decimal or hexadecimal (`0x3F8`); both syntaxes accept
  hex (MEASURED, all three crates in [Crates](#crates)).
- **Size limit:** each file, including parents and embedded preset sources,
  is at most 1 MiB (1,048,576 bytes), inclusive. File reads are bounded to
  limit + 1 bytes; in-memory TOML/YAML parsing enforces the same limit before
  decoding. Larger inputs fail with the path and exact message
  "machine file exceeds size limit of 1048576 bytes". YAML's event, alias and
  nesting budgets remain additional limits.
- **Strictness:** every table rejects unknown fields (`deny_unknown_fields`).
- **No location-losing serde shapes:** no `#[serde(flatten)]`, no internally
  tagged enums, no untagged enums in the document types. With an internally
  tagged `[[card]]` enum, the TOML error for the second card pointed at line 1,
  the first card (MEASURED). With a flat struct, every crate reported the exact
  line and column (MEASURED). So a card is one flat struct with a `kind` field.
  The fields that a given kind does not accept are rejected during lowering,
  with the card id in the message.
- **Two-form values** (`irq = 4` or `irq = "none"`; `nvram = "derived"` or a
  table) use a hand-written `Visitor`. It keeps the value's location in TOML
  and in both YAML crates (MEASURED).
- **Paths** are strings with `/` separators, resolved against the directory of
  the file that contains them. The loader canonicalizes the machine file path
  to find the layer directory: when it is a symlink, relative image, journal
  and `extends` paths use the target file's directory. Diagnostics preserve
  the path supplied by the caller, including relative components and symlink
  spellings. Absolute image and journal paths are accepted. Repository files must use relative paths.
- **Lowering principle:** lowering rejects only what `MachineConfig` cannot
  represent: unsupported values and fields a card kind does not accept.
  Reserved names are recognised and rejected during parsing. Every rule about
  a representable machine lives in `MachineConfig::validate()`. There is no second validation path.

### Top level

| Field | Type | Default | Notes |
|---|---|---|---|
| `schema` | integer | required | `1`. By convention the first key. |
| `name` | string | file stem | Metadata for the window title and messages. Not part of `MachineConfig`. Not inherited. |
| `description` | string | none | Metadata. Not inherited. |
| `extends` | string | none | A relative path or `preset:<name>`. See [extends](#extends). |
| `board` | table | required | |
| `cpu` | table | from the board | |
| `memory` | table | from the board | |
| `firmware` | table | required after merge | |
| `card` | array of tables | empty | |

### `[board]`

| Field | Type | Default | v1 | Lowers to |
|---|---|---|---|---|
| `model` | string | required | `"ibm-5170"`, `"generic-386-isa"` | `MachineConfig.board`, plus the board device set |
| `post_port` | u16 or `"none"` | `0x80` on `ibm-5170` | yes | the `Post` device; also the DMA page-register decode |
| `rtc` | table | see below | yes | the `Rtc` device |
| `mouse` | boolean | `false` | yes | attach a standard PS/2 mouse to the 8042 auxiliary port; reserve IRQ12 |
| `display_switch` | `"auto"`, `"mono"`, `"colour"` | `"auto"` | yes | select the primary display class for the 8042 input switch and derived CMOS video bits |

The `ibm-5170` board model supplies the devices below, in this order (the
canonical order, M2). The values are today's presets, unchanged.

| # | Role | `DeviceId` | Ports | IRQ | Notes |
|---|---|---|---|---|---|
| 1 | master 8259 | `MasterPic` | `0x20–0x21` | — | `cascade = true` |
| 2 | slave 8259 | `SlavePic` | `0xA0–0xA1` | — | cascaded on master IRQ2 |
| 3 | 8254 PIT | `Pit` | `0x40–0x43` | 0 | |
| 4 | byte 8237 DMA | `Dma` | `0x00–0x0F`, page `0x81–0x88` (`0x80–0x88` when `post_port` is not `0x80`) | — | channels 0–3 |
| 5 | word 8237 DMA | `DmaWord` | `0xC0–0xDF`, page `0x89–0x8F` | — | channel 4 is cascade |
| 6 | 8042 KBC | `Keyboard` | `0x60`, `0x64` | 1 | |
| 7 | MC146818 RTC | `Rtc` | `0x70–0x71` | 8 | |
| 8 | optional 80287 socket | `Coprocessor` | `0xF0–0xF1`, `0xF8–0xFF` | 13 | |
| 9 | system control | `SysControl` | `0x61` | — | |
| 10 | POST latch | `Post` | `post_port` | — | omitted when `"none"` |

Board-dependent validation rules (new checks in `validate()`, M1). On
`ibm-5170`, a card IRQ must be one of 3–7, 9–12, 14 or 15: IRQ2 is the cascade,
and 0, 1, 8 and 13 are not on the bus. Card DMA wiring currently names the byte
controller and a channel in 0–3; schema 1 does not expose word-DMA card wiring.
The `Bare` board skips these rules.

`model = "ibm-5160"` (generic XT) is reserved. The list of board models and the
firmware each one accepts belong to the firmware spike.

`board.mouse` is a board peripheral option because the board already supplies
its 8042; keyboard/controller replacement remains reserved. TOML and YAML use
the same boolean, merged as a scalar by `extends` (explicit `false` disables an
inherited mouse):

```toml
[board]
model = "ibm-5170"
mouse = true
```

```yaml
board:
  model: ibm-5170
  mouse: true
```

Omission leaves the mouse off, including all shipped presets and machine files.
This option requires an 8042 and slave PIC (`ibm-5170` today). Assembly attaches
an ID-00 PS/2 mouse and routes its interrupt through IRQ12 (slave IR4, master
IR2 cascade); primary keyboard IRQ1 is unchanged. This is an opt-in auxiliary
extension, not a claim that the original IBM AT shipped a mouse port. Host input
and disabled-reporting accumulation are described in
[machine-config.md §PS/2 mouse](machine-config.md#ps2-mouse).

`board.display_switch` is the physical AT display slide switch. `auto` prefers
MDA, then colour, independently of card order. `mono` requires an installed
MDA; `colour` requires an installed `vga-text` or `vga` card. A forced class
without a matching card is an error at `board.display_switch`. Unknown names
(including `color`) and non-string values are rejected. A child inherits the
parent switch; explicitly setting `auto` clears a parent's forced selection.

Equivalent TOML/YAML overrides for a machine with MDA and `vga-text`:

```toml
[board]
display_switch = "colour"
```

```yaml
board:
  display_switch: colour
```

MDA plus `vga-text` is supported; MDA plus register `vga` still has conflicting
ports/MMIO. With no display card, `auto` leaves the switch at colour and CMOS
video bits at 00 (the equipment byte has no no-adapter code). A ROM-less colour
card derives 80-column colour (10); MDA derives mono (11); a colour card with a
video option ROM at C0000h derives own-BIOS (00). Explicit NVRAM images are not
rewritten, so their equipment byte must agree with the chosen switch and cards.
See [machine-config.md §Primary display](machine-config.md#primary-display)
for the hardware references, no-card policy and snapshot behavior.

### Generic 386 ISA board

`model = "generic-386-isa"` selects `MachineClass::At386` and defaults to
`cpu.model = "80386dx"` (x86jit `CpuModel::I80386`); selecting a 286 or an
80287 is refused. The IBM 5170 retains its 80286 CPU and existing rules.
The 386 board has the same AT IRQ/DMA wiring and device order, with a
128-byte DS12887-class RTC replacing the 64-byte RTC, then port 92h,
the byte debug console at 402h and emulator-only fw_cfg at 510h/511h.
These motherboard devices are required, with fixed resources. fw_cfg serves
`etc/sercon-port` as a little-endian COM1 port number from the installed
`uart-16550` card. A COM1 card is required by that default. VGA uses the
register-level `vga` card with the Listed SeaVGABIOS standard-VGA ROM. The shared 8042/92h A20 line retains OR arbitration; either
source can enable A20. No selectable chipset or shadow-register model is added.

Conventional RAM is 640 KiB. Extended RAM starts at 1 MiB, in 64 KiB
steps, from zero through **64,512 KiB (63 MiB)**. The chosen population cap
bounds guest RAM at 64 MiB and covers the first ISA board's needs; it is an
emulator policy, not an 80386DX bus limit. The CPU has 32 physical address
lines. C0000h–FFFFFh is writable RAM, with the 128 KiB BIOS preloaded at
E0000h–FFFFFh. A separate, guest-read-only copy lives at
FFFE0000h–FFFFFFFFh. CPU resets retain both shadow RAM contents and this
pristine copy; firmware can copy the pristine bytes back during reboot.
`ibm-5170` still refuses writable adapter space and retains its 24-bit map.

The 386 backend reserves a **4 GiB host virtual address span** using x86jit's
sparse backing (`Vm::sparse(VmConfig::reserved(1 << 32))`). Only mapped
regions commit backing: conventional and configured extended RAM, the
256 KiB shadow, the separate 128 KiB high BIOS copy, and configured trapped
device windows. Unpopulated holes remain uncommitted and guest-unmapped;
reservation alone does not populate guest RAM. Host-page rounding can add a
small amount of commitment. The 64 MiB low-memory population cap is independent
of the 32-bit address reservation. Adapter diagnostics expose the sparse backing
kind, reserved span and numeric committed-byte count. Regression tests bound
commitment by the populated regions, including the high ROM copy and device
windows, and verify that unpopulated gaps have no mapping. The tested board
regions are all 64 KiB aligned, so host-page rounding adds no bytes on supported
hosts; their committed-byte count equals the exact mapped-byte total.
Host commitment is separate from RSS and excludes tracking metadata.

Linux/macOS reserve inaccessible pages and commit mapped ranges with page
protection changes; Windows reserves and commits with VirtualAlloc. Reservation
and mapping failures return the adapter's `CpuError::Memory`, without aborting
on a full-span allocation. Sparse VMs use the checked interpreter, preserving
exact retired-instruction counts. The 286 constructors keep their existing
flat backing and memory maps.

[`machines/generic-386-seabios.toml`](../../../machines/generic-386-seabios.toml)
uses 15 MiB of extended RAM and the `seabios-1.17.0-unemu1-isa` catalogue entry.
At CPU pin `b7c10b6f` (x86jit), reset installs the core's
`CpuState::i386_reset`: CS=F000h, hidden base FFFF0000h, EIP=FFF0h and
EDX=0303h. Intel 80386 PRM §10.1 specifies DH=03h; DL=03h is the core's
assumed stepping. CR0.ET reflects the FPU model (clear on this no-FPU board).
The bare CPU adapter reapplies the retained A20 gate after installing the
image: with A20 disabled its reset fetch addresses FFEFFFF0h, deliberately
unmapped. The assembled board explicitly chooses a different **emulator
policy** for CPU-only resets: temporarily force CPU A20 on while CS retains
selector F000h and hidden base FFFF0000h. The reset fetch therefore reaches
the pristine ROM at FFFFFFF0h even when both the 8042 and port 92h sources
are off. This is a board policy, not a universal 386 chipset behaviour;
period chipsets differ. No ROM alias is added at FFEFxxxxh.

Near jumps preserve the hidden base and keep the window open. While it is
open, execution stops after every instruction so the first CS reload closes
it immediately, including a far jump that keeps selector F000h but replaces
the hidden base with F0000h. CPU A20 then equals the current OR of the board
sources before the next instruction; neither source nor its guest read-back
changes during the window. Assembled-machine tests check disabled sources
over repeated resets, the high near jump, same-selector far jump and a
subsequent wrapped data read. Other synthetic tests check retained shadow
RAM. The existing 286 reset path and its window policy remain unchanged.

**CPU-only reset sources.** A typed CPU shutdown exit (triple fault), a
0→1 transition of port 92h bit 0, and the 8042 pulse command FEh all use
the machine's pending CPU-reset path. The request returns `ResetRequested`;
the next run installs `CpuState::i386_reset` before checking its instruction
cap, or the caller can complete it with `reset_cpu`. RAM, shadow RAM, the
pristine high copy, CMOS, device latches and virtual time survive. The A20
reset-fetch window above opens again; after CS reload the retained 8042/92h
OR applies. Port 92h bit 0 stays set across CPU reset, so another pulse needs
a clear followed by a set (IBM Personal System/2 Hardware Interface Technical
Reference, System Control Port A; Intel386 EX User's Manual, PORT92, CPURST).
The shared reset-storm guard covers all three sources and produces a stop
report after 64 consecutive close requests; see observation.md §The machine
resets again and again.

**CF9h remains unmapped.** READ: SeaBIOS rel-1.17.0 `fw/shadow.c:196–208`
tries writes 02h then 06h to the PCI reset port after restoring the pristine
image, then immediately executes `int3`. Its one-byte protected-mode IDT
(`misc.c:134–145`) cannot deliver even #DF. An ISA board needs no PCI reset
register: dropped CF9h writes deliberately fall through to shutdown/reset.
Synthetic guest probes exercise that fallback and both board pulses, retaining
a RAM marker and checking the 386 reset signature, CS hidden base, A20 source
combinations and reset-storm report. No IBM 5170 reset wiring is changed.

The firmware is built from `unemu-org/seabios`, pinned at release
`v1.17.0-unemu1`, commit `e38573ec2d7d43fb7802bc43ddcb3ce2bf892809`: upstream rel-1.17.0 plus the
fork's documented 80386 compatibility fixes. The initial fix replaces inline
`BSWAP` in `__swab32` with 386-compatible C. The fork is the source of truth;
fixes go upstream (`upc-o0j.23`). The catalogue ID distinguishes this fix set
from the former upstream fixture, while its ROM filename stays unchanged.
Both SeaBIOS catalogue entries pin the verified release source archive
`vendor/seabios/build/seabios-1.17.0-unemu1-src.tar` (SHA-256
`b85015a695a9b1b8080c4871335bb68c2ec9aa9c8ba69e88a122a925c580c8c0`).
It includes the rebuild recipe; the release firmware sources and ROM bytes are unchanged.

**MEASURED at CPU pin `b7c10b6f` (2026-11-24):** the shipped diskless
machine completes POST within a 10,000,000-step bound. Port 402h reports
`Found QEMU fw_cfg` and `RamSize: 0x01000000 [cmos]`. COM1 and port 402h
print `No bootable device.`. The probe stops after **5,885,053 retired
instructions**. It asserts these facts and the five E820 lines, rather than
a full console transcript. The E820 map is:

| Start | End (exclusive) | Type |
| --- | --- | --- |
| 00000000h | 0009FC00h | RAM |
| 0009FC00h | 000A0000h | Reserved |
| 000F0000h | 00100000h | Reserved |
| 00100000h | 01000000h | RAM |
| FFFC0000h | 100000000h | Reserved |

The project-owned `seabios-int15` floppy boot sector calls INT 15h after
POST. Its source and 512-byte image live under
`crates/machine-file/tests/fixtures/`; rebuild with the NASM command in the
source. The test pads it into a read-only 1.44 MiB in-memory floppy.
It tests the shipped 16 MiB configuration and a derived 4 MiB configuration.
The guest clears CMOS 30h/31h before the calls. Both sizes still follow
E820, rather than that CMOS word:

| Total memory | AH=88h AX, CF | E801h AX/CX | E801h BX/DX, CF | Highest E820 RAM end |
| --- | --- | --- | --- | --- |
| 16 MiB | 15360 KiB, clear | 15360 KiB | 0, clear | 01000000h |
| 4 MiB | 3072 KiB, clear | 3072 KiB | 0, clear | 00400000h |

The E820 test checks all five entries, SMAP signatures, returned lengths,
CF and continuation tokens through the final zero token. Only the extended
RAM entry changes with memory size. **READ:** SeaBIOS rel-1.17.0
`malloc.c:476–491,565` sets `LegacyRamSize` from the highest RAM/ACPI entry
end below 4 GiB. `system.c:170–180` caps AH=88h at 63 MiB above 1 MiB;
`system.c:260–286` splits E801h at 16 MiB. These two configurations do not
exercise those upper caps.

After diskless POST, CMOS 0Fh=0 and an 8042 FEh pulse reach
`In resume (status=0)` (`resume.c:24–29`). The probe checks complete POST
before resetting. It reaches the resume marker at **5,887,802 retired
instructions**, virtual time **5,997,128**. POST, INT 15h and resume probes
are normal tests; each completes in a few seconds in the optimized test tier.

The former `F3 90` stop at `F000:A72C` on `deae5bac` is passed.
The current CPU executes F3 90 and F2 90 as NOP on every model, so SeaBIOS
`cpu_relax` needs no option. x86jit plans an I80386 option for other REP
prefixes on non-string instructions (default #UD per the 80386 PRM); the
board does not need it for SeaBIOS. No new CPU boundary was observed.
The fixture remains Listed; TSS I/O bitmap and paging remain separate work.
The counts above precede SeaVGABIOS; see below for the current video probes
and FreeDOS floppy/readonly-IDE qualification.

#### SeaVGABIOS delivery and mode-set qualification

Both shipped generic ISA profiles name their video ROM through the ordinary
firmware option-ROM syntax:

```toml
[[firmware.option_roms]]
image = "../vendor/seabios/build/vgabios-1.17.0-stdvga.bin"
sha256 = "1adc6c0fa72b93c5d0659575c992b1bf6f64fd451c444b2a735e22f0a3d6e925"
base = 0xC0000
```

The `image` path is relative to the machine file, and `sha256` pins the complete
video ROM bytes. Build the files with `vendor/seabios/fetch.sh` before loading
these shipped profiles. Alternatively, replace `image` with the catalogue name
`seavgabios-1.17.0-unemu1-stdvga`, keep the SHA-256 pin, and supply firmware
discovery roots containing the ROM (for example, `--rom-dir vendor/seabios/build`).
The name alone does not locate a file: the production frontend resolver searches
the explicit roots by the pinned bytes. There is one content source:
normal loading resolves bytes, verifies pins, header, checksum, catalogue and
register-level VGA requirements. Unknown video bytes are refused unless the
existing run-only `allow_unlisted_firmware` escape is enabled. That escape
still checks ROM format and placement. `vga-text` cannot satisfy this ROM.

On `generic-386-isa` and `generic-486-isa`, assembly puts the resolved video
bytes into the immutable fw_cfg file `vgaroms/vga-c0000.bin`, alongside
`etc/sercon-port`. It leaves C0000h shadow empty. SeaBIOS clears that region,
copies the ROM there and calls its entry point. `base` therefore names the
fixed video deployment address on these boards. Only `base = 0xC0000` is
accepted, including with the run-only escape: SeaBIOS allocates VGA ROMs
sequentially from C0000h and ignores addresses embedded in fw_cfg filenames
(`src/optionroms.c`, `run_file_roms`; `src/malloc.c`, `rom_reserve` and
`rom_confirm`). A different base would misrepresent where the ROM executes.
The IBM 5170 continues to map its video ROM as before. This does not enable arbitrary `genroms/` delivery.

**MEASURED, CPU pin `274d48a8`:** diskless POST shows SeaBIOS boot text in
80×25 mode 3. The golden decodes `No bootable device.` and hashes indexed
pixels rendered from text planes 0/1, the loaded plane-2 font and the DAC.
The project-owned `seabios-int10.asm` floppy sets AX=0003h, writes
`SeaVGABIOS INT 10h mode 3` through AH=0Eh, pauses for a bounded host capture,
sets AX=0013h and fills all 64,000 pixels with `offset & 255`. The probe checks
the decoded text, both pixel-frame hashes and every graphics pixel. The media
base stays read-only, with writes separate under the existing drive policy.
Rebuild the 512-byte sector with `nasm -f bin seabios-int10.asm -o
seabios-int10.bin` in `crates/machine-file/tests/fixtures`.

All three CPU variants produce the same hashes:

| Capture | Indexed pixel-frame SHA-256 |
|---|---|
| Diskless POST (mode 3, loaded font) | `a9ec051a7369ed33ca6cf8cc59b285469184ec326d0c8284d6eba6cf53cee99d` |
| Explicit INT 10h mode 3 and teletype string | `31c39840f5599a9a16471b247e72a01d2c8092d86ba0350c5a0f354f5b4fd231` |
| INT 10h mode 13h pattern | `825d409a82c363538f60248d866bb8462c28eb7ebdb0499bbd0188c6d0740f27` |

POST stops at **6,199,121 retired instructions**, text capture at
**6,065,851**, and the mode probe halts at **6,326,518**. Measured release
wall times were 1.14–1.32 s to POST/text and 1.25–1.40 s for the complete
mode probe. The INT 15h probes now retire 6,036,163 (16 MiB) and 6,036,133
(4 MiB); reset/resume reaches 6,201,870, virtual time 6,365,088. The preceding
serial-only measurements remain historical.

The existing VGA register/memory implementation needed no changes. In
particular, SeaVGABIOS's font loader selects plane 2 through the sequencer map
mask and graphics map while preserving text planes; the rendered-font golden
covers the result. No new VGA register coverage is claimed. These are normal
tests, matching the existing SeaBIOS probes' few-second release tier.

The pinned system BIOS still prints `No VGA found, scan for other display`
before trying fw_cfg video ROMs. The successful probes require the subsequent
option-ROM execution and installed VGA handler. SeaVGABIOS's supported COM1
debug configuration supplies its banner and initialization lines; port 402h
retains system-BIOS diagnostics. See the two pinned-source limitations in
[`vendor/seabios/README.md`](../../../vendor/seabios/README.md#standard-vga-rom).

Headless reproduction (the CLI prints COM1; the test also captures port 402h):

```sh
cargo run --offline -p unemupc -- run --machine machines/generic-386-seabios.toml --max-steps 10000000 --idle-limit 1000000 --print uart-16550
```

The board's default rate is one retired step per PIT clock, an uncalibrated
deterministic policy rather than an instruction-cycle model.

#### FreeDOS floppy and IDE profiles

`machines/generic-386-freedos.toml` extends the diskless SeaBIOS profile
with an FDC at 3F0h (IRQ6, motherboard 8237 channel 2) and the existing
SHA-256-pinned 1.44 MB FreeDOS A: image. `generic-386-ide.toml` adds the
existing readonly FAT16 primary master, geometry 615/4/17, at 1F0h/3F6h,
IRQ14. SeaBIOS provides both disk services; there is no XTIDE ROM. The
`generic-486-freedos.toml` and `generic-486-ide.toml` profiles use the same
storage and the early DX CPU; the shared tests also select `80486dx-cpuid`.
The diskless profiles and their hashes remain unchanged.

The `generic-386-games.toml` and `generic-486-games.toml` children add an
SB16/OPL3 to the matching IDE profile, at 220h, IRQ5, byte DMA 1 and word
DMA 5. Both retain the readonly, SHA-256-pinned C: and FreeDOS A: boot order.
The word channel uses the second 8237 at C0h–DFh and the 8Bh page latch.
IRQ5 goes directly to the master 8259; the slave remains cascaded on IRQ2
for devices such as the inherited IDE IRQ14. The sound guest preserves
SeaBIOS's vector bases and unmasks IRQ5 as a DOS sound driver would.
The shared `*_seabios_sb16_opl3_golden` tests select all three CPU models.
They boot project-owned probes through SeaBIOS and check DSP reset/version,
byte and signed-stereo word DMA above 64 KiB, IRQ acknowledgement, both PIC
in-service registers, OPL detection/timer status and FM key-on/release.
PCM is compared with an independent integration of the AT golden samples;
retired counts and complete audio hashes are pinned, with split runs around
DMA deadlines and FM writes. No proprietary guest is needed for these goldens.
The AT FM patch holds its note for about 4000 PIT clocks; the generic guest
scales that instruction loop to each board's nominal rate, then requires
nonzero PCM both before and after key-off. A literal 4000-instruction hold
ends before the FM envelope becomes audible at the higher CPU rates.
At CPU pin `6a38ae49`, all three models acknowledge the byte/word DSP IRQ
597/299 PIT clocks after the final playback-command byte. SeaBIOS's initial
master/slave masks are B8h/8Eh; after the driver unmasks IRQ5, both in-service
registers and the DSP interrupt status return to zero on completion.

**READ:** both boards already permit these cards and supply the DMA controller.
The shared CMOS derivation advertises A: as type 4 in byte 10h (`40h`),
sets equipment bit 0 at 14h, and preserves the integrated-FPU bit on 486.
Adding a 1.2 MB B: gives byte 10h=`42h` and equipment count bits `40h`;
removing both floppy drives clears these fields. The QEMU layout supplies
boot order `3Dh=21h`, `38h=00h` (floppy then hard disk), primary type 47
geometry and no ATA translation for 615/4/17. Tests verify both derivation
and the assembled RTC reads, card capability choices, and FDC IRQ/DMA wiring.

**MEASURED**, CPU pin `5a176677`, all three models: FreeDOS kernel 2044
and FreeCOM 0.86 reach `A:\>` at 8,518,775 retired instructions without IDE,
or 8,516,488 with IDE. SeaBIOS identifies `ata0-0` as the 20 MiB hard disk;
`DIR C:` lists UNEMUHDD and the 29-byte MARKER.TXT, returning to `A:\>` at
11,191,270 retired instructions. Each normal test polls VGA text every
100,000 virtual steps, injects two Enter keys at the date prompt, and has
a 20,000,000-step cap. VGA text hashes, exact retired counts and serial
prompt output are checked. Release wall time is approximately 1.1–1.8 s.
C: remains the existing data-only fixture; direct C: boot is a follow-up.
No CPU boundary or device change was needed.

### Generic 486 ISA board

`board.model = "generic-486-isa"` requires an explicit `cpu.model`:
`"80486dx"` is the early DX without CPUID, and `"80486dx-cpuid"` is the
DX with toggleable EFLAGS.ID and CPUID. Unknown names retain their TOML/YAML
source position. Omitting the model is a validation error. The FPU domain is
`"integrated"` (also the default for this board); `"none"` and `"80287"` are
rejected. Other boards reject `"integrated"`.

The shipped `machines/generic-486-seabios.toml` explicitly selects the early
DX and integrated FPU. Changing only its CPU model selects the CPUID variant.
Both use the unchanged `seabios-1.17.0-unemu1-isa` catalogue image and hash;
its compatible-board list includes both generic ISA boards.

The 486 shares the 386 board's 640 KiB conventional RAM, up to 63 MiB extended
RAM in 64 KiB steps, 128-byte CMOS, OR-wired 8042/92h A20 sources, debug console,
fw_cfg, writable C0000h–FFFFFh shadow and pristine high reset copy. Sparse
backing reserves 4 GiB while committing only the mapped population. CPU-only
reset from triple fault, port 92h bit 0 or 8042 FEh uses `CpuState::i486_reset`
for both variants, including CR0=60000010h and the model's EDX signature
(0403h or 0410h). The shared reset-storm guard and RAM/device retention apply.

**Design choice:** a separate board identity reuses those services without
making `generic-386-isa` mean two processor generations. It keeps firmware
fits and CPU/FPU validation explicit, and preserves existing 386 defaults.
A CPU choice on the 386 identity would require exceptions to its documented
empty-socket domain and firmware identity. No new chipset or device is needed.

**Known gap (READ, pinned CPU API):** the core refuses external numeric-error
routing for both DX variants with `NumericErrorConfigError::UnsupportedModel`.
The board therefore retains internal routing; its reserved F0h/F1h ports use
the inert absent-socket device, never the AT IRQ13 error latch. This is a
provisional emulator policy, not CR0.NE-driven FERR#/IGNNE# fidelity. See
[486 gaps](protected-mode-gaps.md#486-board-coverage-and-gaps).

### Serial mouse

An AT pointing device can attach to a UART card instead of the optional PS/2
auxiliary port. The IBM 5170 has no physical PS/2 mouse port; serial mouse drivers
use COM1 and IRQ4 without BIOS INT 15h C2h pointing services.

```toml
[[card]]
kind = "uart-16550"
port = 0x3F8
irq = 4
mouse = "microsoft"
```

```yaml
card:
  - kind: uart-16550
    port: 0x3F8
    irq: 4
    mouse: microsoft
```

`mouse` accepts `microsoft` or `none` (default); it is rejected on other card
kinds. A child inherits the attachment and can disable it with `none`.
`UartCfg.mouse` selects the endpoint in Rust. Presets retain no mouse by default.
Config validation refuses multiple host mouse targets, including `board.mouse =
true` together with a serial mouse. The window automatically routes to the unique
configured target, using its existing grab and release controls.

The endpoint implements the two-button Microsoft serial protocol: fixed 1200
bit/s, 7 data bits, no parity, one stop bit; identification `M`; then three-byte
`01LRYYXX`, X-low-six, Y-low-six packets. Y is positive down. Counts accumulate
with i32 saturation and split into successive -128..127 packets without discarding
excess. Middle-button/Logitech `M3` and fourth-byte extensions are unsupported.
The UART remains generic and exposes a serial-peripheral attachment, not a mouse
register bank.

Both DTR and RTS must be high. Either low aborts the in-flight frame, clears
unsent bytes and drops motion; current host button levels remain known.
MCR.LOOP (bit 4) disconnects the external serial input and forces DTR, RTS,
OUT1 and OUT2 inactive. The mouse therefore loses power and the external IRQ
is gated off during a loopback probe. Returning from MCR=1Fh to 0Bh starts a
fresh identification. Internal TX-to-RX and modem-input feedback are not modelled.
A transition from unpowered to both high queues `M` after **ASSUMED 10 ms**
startup (11932 PIT clocks), then a full serial frame. A repeated active MCR write
does not restart identification. Buttons held during power-up report after `M`.
Each byte takes ceil(9 × 13125000 / (11 × 1200)) = **8949 PIT clocks**; rounding
adds less than one PIT clock per frame. These clocks derive from retired
instructions (and halted PIT progress), never host time. A new packet waits for
the preceding packet's last byte to finish; there is no separate sample-rate timer.

The UART uses a nominal 1.8432 MHz input, so divisor **96**, LCR **02h** is
1200 7N1. LCR **06h** (7N2) also receives the mouse correctly: the receiver checks
only the first stop bit. The peripheral still sends at the 7N1 byte rate.
**ASSUMED mismatch model:** another divisor, data length or parity selection
produces byte 00h with LSR.FE at the mouse's physical rate. In particular, 8N1
retains this labelled approximation instead of bit-level resampling.
Validity is sampled when the start bit begins, after startup: the guest may
raise DTR/RTS and then program the UART during the startup delay. Changes after
the start bit affect later frames. A future start is an explicit virtual-time
event, so a lazy synchronization cannot sample the configuration too early.
The receive holding register/FIFO, LSR.DR/OE/FE, receive and line-status IIR
causes, four-frame FIFO timeout and OUT2/IER-gated IRQ are modelled. TX remains
instantaneous, with no THRE IRQ, TX FIFO or modem-input signalling.

Hardware references: Microsoft Mouse Programmer's Reference (serial protocol
requirements); Microchip AN519, Table 1, Microsoft Format; National
Semiconductor PC16550D datasheet §8.1–8.8 and §8.11, Line Control, Programmable Baud
Generator, Line Status, FIFO Control, Interrupt Identification/Enable and Modem
Control and FIFO Interrupt Mode Operation (character timeout). Startup delay
and mismatch bytes are explicit model
assumptions. Primary reference copies: [PC16550D datasheet](https://www.scs.stanford.edu/10wi-cs140/pintos/specs/pc16550d.pdf)
and [Microchip AN519](https://ww1.microchip.com/downloads/en/Appnotes/00519c.pdf).
See [window input](observation.md#captured-ps2-mouse).

### `[board.rtc]`

| Field | Type | Default | v1 | Lowers to |
|---|---|---|---|---|
| `start` | string `"YYYY-MM-DDTHH:MM:SS"` | `"2000-01-01T00:00:00"` | yes | `CmosCfg.start` (M7). The weekday is derived from the date. |
| `nvram` | `"derived"`, `"blank"` or a table | `"derived"` | yes | `CmosCfg.nvram` (M7) |

`start` must be a quoted string in both syntaxes. Unquoted, TOML parses a
local date-time and YAML 1.1 a timestamp. The default start keeps every RTC
register as it is today, so the goldens do not change. A start taken from host
time is not a machine property. It is a run option (see [Outside the
file](#outside-the-file-run-options-presentation-host-streams)).

`nvram` forms:

- `"derived"`: today's `seed = true`. Floppy types, the equipment byte and the
  base and extended memory sizes are derived from the machine, and the
  checksum is written.
- `"blank"`: today's `seed = false`, a battery-dead chip. A stock BIOS enters
  SETUP.
- `{ image = "cmos.bin", sha256 = "…", write = "volatile" }`: exactly
  64 bytes. Calendar reads use `start`; volatile initialization retains image
  control bytes and clears live status, as described in `machine-config.md`.
- `{ image = "cmos.bin", sha256 = "…", write = "journal", journal = "cmos.upfj",
  keep_generations = 10 }`: immutable 64-byte base plus journal replay of general
  RAM (0Eh–3Fh except the transient shutdown byte 0Fh). All clock, alarm and
  status registers 00h–0Dh come from `start`
  and the RTC model. Changed RAM appends only before CPU reset and at session
  finish; reset leaves the generation open and finish closes it once. Run stops,
  prompt polls and window frame slices perform no NVRAM I/O. The journal path
  is required; retention is
  positive and defaults to 10. Paths resolve against their defining layer.
  YAML accepts the same mapping, for example:

  ```yaml
  board:
    rtc:
      nvram: {image: cmos.bin, sha256: "…", write: journal, journal: cmos.upfj, keep_generations: 10}
  ```

Wrong image sizes, pin mismatches, missing journal paths, zero retention and
journal-only fields with volatile writes are errors. `write = "readonly"` is
rejected with `readonly NVRAM is unsupported: the MC146818 has no write-protect
input; use write = "volatile"`. POST must be able to write 0Eh/0Fh. The journal
format, recovery diagnostics, host failures and offline tools follow
`persistent-media.md` § CMOS NVRAM journals.

### `[cpu]`

| Field | Type | Default | v1 | Lowers to |
|---|---|---|---|---|
| `model` | string | from the board on 286/386; required on 486 | `"80286"`, `"80386dx"`, `"80486dx"`, `"80486dx-cpuid"` | `CpuConfig.chip` |
| `fpu` | string | `"integrated"` on 486; `"none"` otherwise | `"none"` or `"80287"` on `ibm-5170`; `"none"` on 386; `"integrated"` on 486 | `CpuConfig.fpu`; derived CMOS 14h bit 1 follows the choice |
| `clock_mhz` | number | 33 on generic boards; unset on IBM AT | yes | Approximate CPU throughput from nominal MHz; see below. |
| `turbo` | `{ clock_mhz = number }` | absent | yes | Alternate button position; selected through a run option, never stored as initial state. |
| `rate` | `"A/B"` string or a positive integer `A` | board default (`"8/13"` on `ibm-5170`) | yes | `ClockConfig` A/B. Both parts must be positive. It is a ratio of completed steps per PIT input clock, not a frequency. |

An omitted `rate` comes from `BoardModel::default_clock` in
`crates/assembly/src/board.rs`: `ibm-5170` uses the 8/13 throughput calibrated
against the IBM AT BIOS speed/refresh check. See
[machine configuration](machine-config.md#cpuconfig-and-clockconfig--minimal-but-the-right-home).
An explicit rate overrides that board default. `rate` and `clock_mhz` in the
same layer are mutually exclusive; the error names the `clock_mhz` value and
its source line. A child layer selecting one form replaces the inherited
other form. Turbo is inherited independently.

`model = "8088"` is rejected: "cpu model 8088 needs an 8088 instruction model,
which the CPU core does not have". A model that does not match the board is
rejected. `fpu = "none"` leaves the optional 80287 socket
empty; `fpu = "80287"` selects the core's existing present x87 engine, which
does not claim generation-specific 80287 fidelity. Other names receive a typed
`ParseError` with the exact message ``unsupported cpu.fpu `80387`; allowed values: `none`, `80287`, `integrated` ``
(substituting the rejected name). TOML and YAML share this enum and diagnostic.
Explicit NVRAM images are preserved, even if their equipment bit disagrees.
Derived CMOS 14h bit 1 is set for every selected FPU, including both 486 DX
models' integrated FPU. It remains clear on `generic-386-isa` with `fpu = "none"`.
SeaBIOS does not consume CMOS 14h: `src/misc.c` `mathcp_setup` sets the BDA FPU
flag independently, and `handle_11` returns the BDA equipment word. The unemu
AT BIOS also returns the BDA word (`src/bios.asm` `int_11`); its POST compares
the CMOS FPU bit with detected hardware (`src/at_cmos.inc` `cmos_check`).
Measured after correcting the 486 seed: both 486 SeaBIOS serial/debug transcripts
and retired counts are unchanged (diskless POST 5,885,053; keyboard-reset resume
5,887,802; INT 15h probes 5,768,863 at 16 MiB and 5,768,833 at 4 MiB).
On the I286 machine profile, an empty socket returns FFFFh for FNSTCW m16
and FNSTSW m16/AX, completing BIOS presence probes; other output stores
leave memory unchanged and WAIT completes (`decision-25 (x86jit)`).
EM/TS/MP retain the architectural #NM controls. A present 80287 uses external
ERROR# routing through the board IRQ13 latch; F0h releases BUSY and clears the
latch without clearing numeric status. F1h reset and F8h–FFh bus transactions
remain unmodelled; see [coprocessor controls](machine-config.md#coprocessor-controls).

### Nominal CPU clocks

`cpu.clock_mhz` is a convenience label for approximate instruction throughput.
It does not add per-opcode cycles or wall time to execution. TOML and YAML
accept numeric integers and decimals in **0.001..=1000 MHz**, at 0.001 MHz
resolution. After the syntax decoder, the number's shortest decimal spelling
must have at most three fractional digits. Trailing zeroes and exponent
notation normalize numerically (`33.000` and `3.3e1` mean 33); quoted strings,
booleans, null, non-finite numbers, zero, negatives and excess precision such
as 4.7701 are errors. Values are stored as integer kHz; there is **no rounding**.
The syntax decoder uses binary64 for decimal input, but conversion to kHz
parses its shortest decimal text with integer arithmetic. All subsequent
clock arithmetic is rational. Resolved machine artifacts retain both clock
positions through encode/decode.

**ASSUMED / approximate CPI**, deliberately independent of the executed code:

| Instruction mix | Share | 80286 clocks | 80386DX clocks | i486 clocks |
|---|---:|---:|---:|---:|
| ADD register, register | 40% | 2 | 2 | 1 |
| MOV register, memory | 20% | 5 | 4 | 1 |
| MOV memory, register | 20% | 3 | 2 | 1 |
| Jcc, half taken | 20% | (7+3)/2 | (8+3)/2 | (3+1)/2 |
| Weighted mean | | 3.4 | 3.1 | 1.2 |
| Selected CPI (ceiling to an integer) | | **4** | **4** | **2** |

The mix is an assumed typical integer program, not a workload survey. The
conservative upward rounding is the only CPI rounding. It leaves some room
for instructions omitted from this small mix; it is not a cache, bus or
wait-state model. The 386 taken branch assumes `m=1` for the next instruction.
The 486 timings assume cache hits and no address-generation interlock.
Both `80486dx` and `80486dx-cpuid` use CPI 2. Floating point, REP iteration
accounting, memory stalls and instruction-specific latencies are not calibrated.

**READ — timing sources:** Intel, [80286 and 80287 Programmer's Reference
Manual (1987)](https://bitsavers.trailing-edge.com/components/intel/80286/210498-005_80286_and_80287_Programmers_Reference_Manual_1987.pdf),
Appendix B, “The 80286 Instruction Set”, ADD, MOV and conditional jumps;
Intel, [80386 Programmer's Reference Manual (1986)](https://pdos.csail.mit.edu/6.828/2018/readings/i386.pdf),
§17.2.2.3 “Clocks” and Chapter 17 ADD, MOV, Jcc entries;
Intel, [i486 Processor Programmer's Reference Manual (1990)](https://bitsavers.org/components/intel/80486/i486_Processor_Programmers_Reference_Manual_1990.pdf),
Chapter 26 instruction-set reference, ADD, MOV and Jcc entries (also tabulated
in Intel i486 Microprocessor, November 1989, §10.1 instruction encoding and
clock-count summary). The mix and rounded CPI are our modelling assumptions,
not Intel's performance claim.

The board oscillator remains `device::clock::OSC = 315/22 MHz`, divided by 12
for PIT input: **105,000,000/88 Hz**, about 1.193182 MHz. For integer kHz K:

```
steps/second = K * 1000 / CPI
rate A/B = (K * 1000 * 88) / (CPI * 105,000,000)
```

Reduce A/B by the greatest common divisor. No frequency or rate rounding is
performed; integer phase carries across CPU calls. `cpu.rate` remains the
exact low-level override for independently calibrated workloads.

| Common part / throughput class | `clock_mhz` | CPU model | Reduced rate A/B | Million steps/s |
|---|---:|---|---|---:|
| 286 nominal example | 4.77 | 80286 | 1749/1750 | 1.1925 |
| 286 nominal example | 6 | 80286 | 44/35 | 1.5 |
| 386DX-16 | 16 | 80386dx | 352/105 | 4 |
| 386DX-20 | 20 | 80386dx | 88/21 | 5 |
| 386DX-25 | 25 | 80386dx | 110/21 | 6.25 |
| 386DX-33 | 33 | 80386dx | 242/35 | 8.25 |
| 386DX-40 | 40 | 80386dx | 176/21 | 10 |
| 486SX-25 throughput class | 25 | either 486 DX model | 220/21 | 12.5 |
| 486SX-33 / 486DX-33 throughput class | 33 | either 486 DX model | 484/35 | 16.5 |
| DX2-50 throughput class | 50 | either 486 DX model | 440/21 | 25 |
| DX2-66 throughput class | 66 | either 486 DX model | 968/35 | 33 |
| DX4-100 throughput class | 100 | either 486 DX model | 880/21 | 50 |

Numbers suffice as presets: aliases would carry no additional timing data.
The SX, DX2 and DX4 labels describe only integer throughput, not new CPU
models, clock multipliers, cache arrangements or FPU choices. Both generic
boards and their shipped SeaBIOS profiles select **33 MHz**. The 486 board
models a DX, so a DX-33 is the default rather than silently naming it a DX2.

An unset IBM AT still means **8/13**, about 0.7343 million steps/s. This
calibration includes the board's effective wait-state/refresh behaviour and
is not equivalent to the nominal 6 MHz/CPI-4 label. Setting `clock_mhz=6`
explicitly selects 1.5 million steps/s; it is not a replacement for the AT's
BIOS calibration. IBM AT fixtures and golden expectations stay unchanged.

Turbo is an alternate clock, for example:

```toml
[cpu]
model = "80386dx"
clock_mhz = 16
turbo = { clock_mhz = 33 }
```

Power-on selects the base clock. `unemupc run --machine FILE --turbo` and
`CreateRequest.options = [("turbo", "true")]` select the alternate position
for that run. `false` is the default. Enabling turbo without its table is an
error. `Machine::set_turbo(bool)` switches internally between machine calls;
`turbo_enabled()` reports the position. Alternate clocks may also be slower,
matching a button's physical alternate position without assuming its polarity.
No host pacing is implied by nominal MHz.

Switching preserves the emitted device clock and completed steps. The running
sub-tick phase `r/A` is converted to `floor(r*A_new/A_old)/A_new` (loss less
than one virtual tick per actual rate change). Halted ticks are retained
separately at each rate, and policy time floors only their combined instruction
equivalents. Fractional halted credit survives any number of switches exactly;
future halted time uses the selected rate.
This prevents retrospective changes to run caps. Repeating the selected
position preserves both remainders. The selected position, elapsed time and
fractional credits survive CPU reset; rebuilding a machine starts again at the
requested run position.

The frontend contract currently has no live option-update operation. A live
button would need a versioned run-control operation carrying a boolean turbo
position, applied at a completed machine-call boundary and returning success
or an unsupported/unconfigured error. No contract operation or GUI control is
added here.

The project-owned `cpu_speed_golden` guest counts INC/JMP iterations between
4096-clock IRQ0 edges, excludes its setup interval, and checks the fixed
counts plus the formula (ten ISR steps per interval). It covers 16, 25 and
33 MHz and repeatable on/off switching at fixed completed-step boundaries,
including different caller chunk sizes. Its CPU pin is
`5a17667764080c2fe7a33d446584a3e175b7923d`.

### Measured speed ceilings

**MEASURED 2026-11-24**, ARM64 macOS development-host class (`uname -sm`:
Darwin arm64), release build, CPU pin
`5a17667764080c2fe7a33d446584a3e175b7923d`. No private host identity is used.
The headless `pc-harness` example `cpu_speed_ceiling` runs the project-owned
`cpu-ceiling.asm` ROM: INC, ADD, XOR, DEC and JMP on registers, interrupts
disabled. Motherboard devices remain attached and clocked; no display is
captured. Protected entry uses LGDT/LMSW and a far jump; the probe verifies
MSW.PE before timing. The 286 runs protected16 and the 386/486 protected32.
Each case warms up 100,000 instructions, then times three successive
10,000,000-retired-instruction intervals with host `Instant`, outside the
emulator execution path. Compilation and assembly are excluded. Both guest
loops and their generated bytes are project-owned and reproducible with NASM.

| Board / configured clock | Real16 million instructions/s (min–max) | Protected million instructions/s (min–max) |
|---|---:|---:|
| IBM 5170 / calibrated 8/13 | 98.772–100.417 | 9.967–10.008 (16-bit) |
| Generic 386 ISA / 33 MHz | 96.295–96.882 | 8.082–8.102 (32-bit) |
| Generic 486 ISA / 33 MHz | 99.158–100.528 | 7.908–7.944 (32-bit) |

**DERIVED from the minimum measured rates:** the real16 loop has headroom
for every listed preset, including 386DX-40 (10 million steps/s) and DX4-100
(50 million steps/s). In the protected loop, the IBM AT calibration and
nominal 6/8 MHz settings fit; the 386 fits 16/20/25 MHz but 33 MHz requires
8.25 million steps/s, just above the measured ceiling, and 40 MHz requires
10 million. None of the listed 486 presets fits this protected32 loop:
even 25 MHz requests 12.5 million steps/s. This is a ceiling for these tight
loops on this shared host, not a guarantee for BIOS boot, games, memory-heavy
code or floating point. The nominal clock controls virtual throughput even
when the host cannot keep real time; no wall-clock pacing is introduced.

**MEASURED 2026-11-24 after the TLB repin**, ARM64 macOS host class
(`Darwin arm64`), release, same three-sample method, CPU pin
`6a38ae4964555495c2b5c82ef72fea34e686c0b4`:

| Board | Real16 MIPS (min–max) | Protected MIPS (min–max) | Paged protected32 MIPS (min–max) |
|---|---:|---:|---:|
| IBM 5170 | 83.929–86.301 | 8.310–9.001 (16-bit) | unavailable on 286 |
| Generic 386 ISA | 80.255–84.340 | 7.502–7.700 (32-bit) | **3.003–3.125** |
| Generic 486 ISA (early DX) | 79.794–80.304 | 6.134–6.744 (32-bit) | **1.900–2.466** |

Each of the 24 samples retires exactly 10,000,000 instructions. Paged 386
sample wall times are 3.244859, 3.199814 and 3.330243 s; paged 486 times
are 4.376621, 5.261918 and 4.055903 s. The paged fixture maps the first MiB
through RAM tables, enables PG and runs the same five register instructions;
the host checks PDE.A after warmup to prove a walk occurred. The core now
has a per-vCPU TLB and reusable paged source proofs; paged interpretation
still uses one-instruction blocks.

For comparison, the previous uncached paging run at
`d8bf798ed05520b6b06bbb787a71f3906a0c65d9` measured **1.832–1.888 MIPS**
on the 386 and **1.837–1.854 MIPS** on the early 486. The new ranges are
about **1.59–1.71×** and **1.02–1.34×** those previous ranges, respectively
(pairing current minimum with previous maximum and vice versa). Unpaged
rates are lower than the earlier ~96–100 real16, ~10 protected16 and ~8
protected32 baseline above. Shared-host load was not controlled; these runs
do not isolate a PG=0 regression or establish how much of the measured
paged gain comes from the CPU changes alone. The wide 486 paged spread is
reported as measured, not smoothed away. The benchmark's nominal clock
ratios and 10-million-instruction sample size are unchanged.

**MEASURED 2026-11-25 with flat segment cases**, ARM64 macOS host class
(`Darwin arm64`), release, CPU pin
`ab193c1591e3876db0e5042282f1bc1871b7d75c`:

| Board | Real16 MIPS (min–max) | Original protected MIPS (min–max) | Original paged32 MIPS (min–max) | Flat32, PG=0 MIPS (min–max) | Flat paged32, PG=1 MIPS (min–max) |
|---|---:|---:|---:|---:|---:|
| IBM 5170 | 92.981–94.750 | 9.887–9.999 (16-bit) | unavailable on 286 | unavailable on 286 | unavailable on 286 |
| Generic 386 ISA | 91.709–92.489 | 7.706–8.291 (32-bit) | 3.163–3.433 | **37.333–37.409** | **3.227–3.442** |
| Generic 486 ISA (early DX) | 90.377–92.560 | 7.663–7.809 (32-bit) | 1.065–2.632 | **23.923–24.535** | **2.962–3.261** |

All **36 samples** retire exactly 10,000,000 instructions after a
100,000-step warmup. The four original ROMs rebuild byte-for-byte unchanged.
Their protected32 code segment remains based at F8000h with a 64 KiB limit,
so it cannot enter the CPU's Flat32 interpreter class. Both new fixtures
instead load CS/DS/ES/SS with base zero, inclusive limit FFFFFFFFh and
32-bit code/stack attributes, at CPL 0. A 32-bit far jump enters the ROM at
its flat linear address F8000h plus the entry offset; no RAM code copy is
needed. The register loop and nominal guest clocks are unchanged.

The example asserts all four cached bases and limits, present ring-0
code/data attributes, CS.D/SS.B, CPL, PE, PG and the full loop EIP before
timing. A mutation replacing the flat ROM with the original protected32
ROM fails the limit assertion (FFFFh versus FFFFFFFFh). PG=0 satisfies the
Flat32 class conditions; PG=1 retains the checked paging path. The flat
paged fixture builds identity mappings for the first MiB in RAM at
1000h/2000h, including the ROM and stack; both paged cases assert PDE.A
following warmup.

Flat32 sample wall times are 0.267564, 0.267863, 0.267316 s on the 386
and 0.416143, 0.407583, 0.418013 s on the 486. Flat paged32 times are
2.928107, 3.099183, 2.905305 s and 3.376415, 3.066769, 3.070002 s,
respectively. The original 486 paged32 times are 9.387537, 4.487987,
3.799765 s; the full spread is retained. Shared-host load was not controlled,
so these samples do not isolate the cause of that spread or a paging
regression. The flat register loop demonstrates a higher unpaged ceiling;
it does not predict game performance or general protected-mode throughput.

Reproduce with `cargo run --offline --release -p pc-harness --example cpu_speed_ceiling`
inside the project devShell (shared hosts use their mandated Cargo wrapper).
Assemble `tests/fixtures/cpu-ceiling.asm` under `crates/pc-harness/` with
`nasm -f bin`, adding `-DPROTECTED` for protected16 and both `-DPROTECTED`
and `-DPROTECTED32` for protected32; additionally use `-DPAGED32` for
`cpu-ceiling-paged32.bin`. Add `-DFLAT32` to the protected32 flags for
`cpu-ceiling-flat32.bin`, and also `-DPAGED32` for
`cpu-ceiling-flat-paged32.bin`. `cpu-speed.asm` uses `nasm -f bin`.

### Clock-related golden changes

Only the generic ISA configurations change clock rate. The IBM AT runtime
traces, retired counts, screen hashes and hardware fingerprints retain their
existing expected values. The hardware fingerprint helper omits the newly
added absent `turbo: None` debug field, so that adding an optional clock does
not change the identity of an unconfigured IBM AT.

**MEASURED at CPU pin `5a17667764080c2fe7a33d446584a3e175b7923d`:**

| Golden observation | Former rate-one value | 386DX-33 | Both 486 DX-33 variants | Reason |
|---|---:|---:|---:|---|
| `i386_pm_test_golden` retired | 8,120 | 31,781 | — | More instructions in IRQ0 wait loops per PIT tick. |
| SeaVGABIOS INT 10h mode 3/13h probe retired | 6,326,518 | 8,528,672 | 11,102,827 | SeaBIOS timer waits and boot loops retire more steps; both pixel hashes are unchanged. |
| FreeDOS floppy prompt retired | 8,518,775 | 10,718,776 | 13,218,823 | Timer waits and prompt polling at the new rate. |
| FreeDOS with IDE prompt retired | 8,516,488 | 10,716,489 | 13,316,536 | Timer waits, ATA boot setup and prompt polling at the new rate. |
| FreeDOS DIR C: returned VGA prompt retired | 11,191,270 | 13,491,318 | 16,391,365 | Faster clock, ATA work and keyboard injection after the initial serial prompt finishes. |

The generic test budgets scale the former rate-one caps by A/B, rounded up,
to retain the same virtual-time allowance. Initial display polls may precede
VGA text-mode initialization and wait for text rather than failing on a pixel
frame. COM1 can still be shifting the prompt after VGA stores finish; the
probe now waits a bounded 100 ms of virtual time for queued serial characters
before typing a command. Retired goldens are captured at the VGA prompt
before that drain. FreeDOS's screen includes the RTC time printed during startup; the
new throughput changes that time and therefore its full-screen hashes:

| Screen | Former SHA-256 | 386DX-33 SHA-256 | Both 486 DX-33 variants SHA-256 |
|---|---|---|---|
| Floppy prompt | `23ae5be81a6c6b1c0059f930424a545d63ec92d548d3f1031b86919e1abf8ef5` | `0fbdd8acac3037c89d098ff1a21d43bd767b0ca7a81c18b1ecaebb70907282b2` | `8456147fac982428ea6abeff04a22770f703af09e4ecfb82f087034826177a7e` |
| IDE prompt | `4a873e897c0d8e8f3d49a6ae2a507f8d5af59b4b507f5a312ce0242c16c9c9de` | `998d3feaae41ac8dd1c9ae964923cc87d4b6f1e353c45c02646780db971920d5` | `84c6ed34f1a68f63adcaf3dd7e25b1ba7d0ead0f299307e80b207ebc8bfdfc7e` |
| DIR C: returned prompt | `b133385b48060a0c721bfdc1f4f572525bea81eb94303d74b25d9a5e0f24b323` | `ea7570aa1cfd5cd6419330d97d46169a947519686e2d92cb26ec099e77315165` | `69417229e1b3356358125a6f9fdbfe8ebb65112b7847f18f32d6b13912bc59be` |

The six generic repository-file hardware fingerprints change because the
clock ratio is part of hardware identity:

| Machine | Former fingerprint | New fingerprint |
|---|---|---|
| `generic-386-freedos.toml` | `9f1c1f819169673c59131c097b33720b71d3f3bb151fee25d42e489ee9b2338c` | `8e5108608b54b5dc7995ca6d0fe2e7d47669c6a50d7c9732ea2027fa2993a212` |
| `generic-386-ide.toml` | `49d6dcf2620f0fceb502015e04f19257874d6dffeb207d0ac7de577c6a7263db` | `8e3d00fb265c98f756cb78af21875862cc487c3123a8a3494315fb3388fee788` |
| `generic-386-seabios.toml` | `fb5592babbaa5a31093d1501e6a070dc6af8ee4634cf713f4446a8a4425a6265` | `61cf0bc019ca5f066c88f38db91de63492fc27b3ea225f95882daa06305b51d0` |
| `generic-486-freedos.toml` | `e71c732d666f2ecb48e4a7655554f9d3227a4db10efc3950aa23c99d7fda988d` | `15d8146732683afe9e9ff865d82aafab9441f35000d378cdafb914893e6cb94e` |
| `generic-486-ide.toml` | `565fc6eb2eca9b325cdd75bf59eadd826308be6efdb3f41dd602dd274339683b` | `ee6e31860a9cb03a4339c3b7696f6f969ffe867d2c9e8ad8954a8bf355dd1efa` |
| `generic-486-seabios.toml` | `18df36806e3236a87a678d52be3e5e5bf8809ba4475f2280d6ad8491899eaf9d` | `d43e0b355235b30aec8244dec0efb47cd7d2c843d38c10a68466329740c2f54a` |

### `[memory]`

| Field | Type | Default | v1 | Lowers to |
|---|---|---|---|---|
| `conventional_kib` | integer, multiple of 16, at most 640 | 640 | yes | RAM `[0, conventional)` |
| `extended_kib` | integer, multiple of 64, at most 15232 on ibm-5170 or 64512 on generic-386-isa | 0 | yes | RAM `[1 MiB, 1 MiB + extended)`. Also sets `address_space_bytes`; assembly uses a fixed 16 MiB AT VM. READ: the AT System Memory Map ends expansion RAM at FE0000h, directly below the high alias. CPU A20 gating remains unwired. |

**The RAM rule (`ibm-5170`).** Upper probe RAM is [A0000h, C0000h), minus
card MMIO and loaded images, joined to adjacent conventional RAM. The model
protects [C0000h, F8000h) from writable system RAM because Kiselev scans it.
The AT System Memory Map distinguishes I/O expansion ROM at C0000h–DFFFFh,
reserved system-board space at E0000h–EFFFFh, and system-board ROM at
F0000h–FFFFFh. MMIO remains valid in the protected interval. Assembly fills
adapter gaps from C0000h to the BIOS base with read-only zeroes, preserving
historical goldens. Other holes in the AT's 24-bit decode read FFh and
discard guest writes (**ASSUMED** floating value). **READ:** IBM Personal
Computer AT Technical Reference, September 1985, Section 1 "System Board",
System Memory Map (p. 1-8), assigns FE0000h–FFFFFFh as an alias of
E0000h–FFFFFh; assembly copies the resolved low ROM/gap bytes read-only there.
Both lowering and assembly enforce the **READ** 15,232 KiB extended-RAM limit
from the same System Memory Map: 100000h–FDFFFFh, directly below the alias.
The FreeDOS preset
has RAM [0, B0000h), MDA/VGA at B0000h–BFFFFh, and BIOS at F8000h–FFFFFh.

### `[firmware]`

| Field | Type | Default | v1 | Lowers to |
|---|---|---|---|---|
| `bios.image` OR `bios.parts` | path OR array of chip tables | required: exactly one | yes | `LoadImage { kind: Rom, source: Inline(assembled_bytes) }` |
| `bios.parts[].image` | path relative to defining layer | required | yes | chip bytes |
| `bios.parts[].lane` | `"even"` or `"odd"` | required | yes | byte lane |
| `bios.parts[].offset` | unsigned integer in assembled bytes | `0` | yes | pair start |
| `bios.parts[].sha256` | 64 hex characters | none | yes | checked against this chip's bytes; retains its pin origin |
| `bios.sha256` | 64 hex characters | none | yes | checked against assembled bytes after chip diagnostics |
| `bios.base` | integer | `0x100000 − size` (the image ends at 1 MiB) | yes | `LoadImage.base`. A 32 KiB Xi8088 image gives `0xF8000`. |
| `bios.provides` | list of strings | `[]` | `"floppy"`, `"fixed-disk"` | `LoadImage.provides` (M6) |
| `option_roms` | array of tables | `[]` | yes | one `LoadImage` (`Rom`) each |
| `option_roms[].image`, `.sha256`, `.provides` | as for `bios` | | yes | |
| `option_roms[].offset` | unsigned byte offset in the file | catalogue-owned for listed containers, otherwise `0` | yes | `ImageSource::RomContainer`; hash covers the full file |
| `option_roms[].base` | integer, 2 KiB aligned, complete mapped image in the selected BIOS video or extension window | required | yes | `LoadImage.base` |

Lowering records the `[firmware.bios]` image index in
`MachineConfig.system_bios_image`. The shared BIOS preset also records this
index for CLI `--bios` loads. A malformed declared BIOS therefore reports
`BiosPlacement` for that image rather than a missing-BIOS diagnostic.

Rules in `validate()` (M6). On a firmware board, an `ide` card needs a ROM
that provides `"fixed-disk"`. An `fdc-765` card with drives needs a ROM that
provides `"floppy"`. Known images inherit their catalogue services; a nonempty
restatement must equal the catalogue. Unknown option ROMs are rejected during
lowering by default. Run-only options can be passed to `lower_with_options`,
`load_with_options` or `load_with_reader_and_options`; `allow_unlisted_firmware`
then permits unknown hashes while retaining source-located errors. The option
is never a machine-file field and does not bypass format or compatibility.
Option ROMs need 55AAh, a nonzero length byte × 512 exactly equal to image
size, and a zero modulo-256 byte sum. The whole image must fit the BIOS scan
window; it cannot overlap another image. Assembly records effective scan order
by ascending address, independent of file order.

### Explicit content selectors

The schema-1 hardware tables also accept package bindings (see
[content bindings](content-bindings.md), § One source per item). Both TOML and
YAML use the same keys, including inside an image or launch envelope's `machine`.
A binding-aware host loader takes verified `BindingUse`/`BindingItem` facts;
ordinary loading of an unresolved template reports that composition is required.
Planning never opens content or searches firmware directories.

- A BIOS or option ROM uses `use = "package-use"`, `rom = "item-id"`, optional
  `role`, `size` and `sha256`, and its normal `base`/`offset`/`provides` fields.
  Exactly one source group is allowed: `image`, `parts`, or `use` plus `rom`.
  Option ROMs require `base`. Roles are exactly `system-bios`, `video-bios` and
  `option-rom`; the package must declare the effective positional role even
  when the document omits `role`. The separate C0000h–C7FFFh window requires
  video firmware; reviewed catalogue video firmware also requires that role
  outside this window. Roles remain local to the selected item and package.
- A drive can use `inserted = { use = "dos", medium = "disk-1" }` instead of
  `image`. The selector also accepts `size` and `sha256` assertions. Controller,
  unit/channel, drive type, geometry and write policy retain their normal meaning.
- A floppy `media` member can use `{ use = "dos", medium = "disk-2" }` instead
  of `image`, with the same assertions and per-medium write policy. Every member
  is resolved, including members not inserted at power-on. Catalogue order stays
  unchanged. `image`, `disk`, `inserted` and `media` are exclusive source groups.
- Top-level `disks = [{ id = "system", size = 528482304 }]` declares zero disks.
  A single IDE drive attaches each with `disk = "system"` and explicit CHS
  geometry. Ids follow the existing safe medium-id spelling; sizes are positive
  and must equal `cylinders * heads * sectors * 512`, without overflow. Duplicate,
  missing, unused and multiply attached declarations are errors. Discovery and
  fact resolution retain native zero attachments without a content row; runtime
  zero mounting is a separate implementation stage.

During inheritance, an omitted `disks` list is inherited; an explicit list
replaces it atomically, retaining the new declaration positions. Selecting a new
ROM source clears the inherited source, digest, role, size, offset and services
unless restated. Selecting a new drive source clears its inherited alternative
selectors and pin, following the existing journal/write override rules. A
metadata-only override retains the original selector and its defining position.

Binding occurrence ids use resolved hardware positions, such as `firmware.bios`,
`firmware.option_roms[0]`, `card[1].drives[0]` or
`card[2].drives[0].media[0]`. Discovery sorts ids by Rust string ordering; facts
never reorder hardware arrays. The envelope's `machine.` prefix appears in
source diagnostics, not in ids. Bound occurrences resolve to distinct generated
logical `assets/...` keys and use the same image reader as path sources.

### VGA video BIOS

The register-level `vga` uses the existing `firmware.option_roms` schema. A
video ROM is still a board-mapped executable image, so it shares the normal
pin, origin, overlap and scan-window checks rather than introducing card-local
firmware loading. On `ibm-5170`, the IBM 5170 and Kiselev system BIOS entries
scan C0000h–C7FFFh separately from their extension windows. A listed BIOS with
no video scan rejects this placement. `vga-text` cannot satisfy the IBM video
ROM's register-level VGA requirement.

```toml
[[card]]
kind = "vga"

[[firmware.option_roms]]
image = "ibm-vga-1987-32k.bin"
sha256 = "51b49761f72780a696c36f3d2e74a8168c1c99b865070f7f90b8e621a8b5ae17"
base = 0xC0000
# Optional restatement: the catalogue already owns this container offset.
offset = 0x2000
```

When extending `preset:ibm-5170` or the existing floppy profiles, remove the
inherited `mda` and `vga-text` cards (`id='mda', remove=true` and
`id='vga-text', remove=true`) before adding `vga`. Its full A0000h–BFFFFh
aperture conflicts with MDA, and the two VGA models are mutually exclusive.
Existing profiles and their default cards are unchanged.

The same fields work in YAML. Pins identify the complete 32 KiB file, including
its unused prefix; only the trailing 24 KiB map at C0000h. A listed image must
use its catalogue offset. Unknown synthetic containers may specify an offset
under the run-only unlisted-firmware option; the suffix must still be a complete,
checksummed option ROM fitting one scan window. `offset` is not supported on
`firmware.bios`; system BIOS `parts[].offset` retains its lane-placement meaning.
Replacing a ROM source during inheritance drops the inherited container offset
unless the child restates it. Only the full 32 KiB IBM form is listed; a cropped
24 KiB dump needs a separately measured identity.

### User-supplied split system BIOS

Only `firmware.bios` accepts `parts`; TOML example (paths relative to the defining file):

```toml
[firmware.bios]
parts = [
  { image = "../vendor/user-roms/ibm-5170-bios-1985-06-10/6480090.u27", lane = "even" },
  { image = "../vendor/user-roms/ibm-5170-bios-1985-06-10/6480091.u47", lane = "odd" },
]
sha256 = "6fb653b58e678cb69b0f943a105c6c803b18a6db23d13c0abccec5b66c6ee652"
```

```yaml
firmware:
  bios:
    parts:
      - {image: ../vendor/user-roms/ibm-5170-bios-1985-06-10/6480090.u27, lane: even}
      - {image: ../vendor/user-roms/ibm-5170-bios-1985-06-10/6480091.u47, lane: odd}
    sha256: 6fb653b58e678cb69b0f943a105c6c803b18a6db23d13c0abccec5b66c6ee652
```

Each part requires `image` and `lane` (`even` or `odd`), accepts `offset`
(default 0, in assembled bytes) and optional `sha256`. Part pins check each
chip's bytes; the BIOS-level pin checks the assembled bytes. Pin mismatches
retain the image and pin origins. A plain pre-interleaved `image` has the same
catalogue identity. `machine check` reports the matched entry as usual.

`image and parts are mutually exclusive`. A child `parts` list replaces the
parent's `image` or entire `parts` list; a child `image` replaces `parts`.
Either source replacement drops inherited `sha256` and `provides` unless
the child restates them. Known images receive their catalogue services;
unlisted images must restate services needed by their cards. Other BIOS
fields continue to merge independently. Inherited part paths retain their
defining layer's directory and per-field origins.

Layout errors start with `firmware parts:` and require at least one even and
one odd part, exactly one of each per offset, equal nonzero chip sizes and
contiguous pairs from offset zero, without gaps or overlap. Option ROMs report
`option ROMs cannot use parts; only firmware.bios supports parts`.
The layout with the most matching chip hashes at any positions (same part
count) reports every misplaced chip and its correct lane/offset. Exchanged
lanes within pairs use the word `swapped`; exchanged halves name the expected
offsets. Unknown chips name the expected label/part number and expected/actual
SHA-256. Chip diagnoses precede the assembled pin check, so a pinned image
still receives the useful chip diagnosis. Part pins are checked first.
Otherwise `FirmwareNotListed` includes known chip layouts and
`vendor/user-roms/README.md`. `--allow-unlisted-firmware` relaxes membership
only; malformed layouts still fail. No new CLI flag or machine-file escape is added.

### `[[card]]`

Common fields:

| Field | Type | Notes |
|---|---|---|
| `id` | string | Unique key for `extends` and messages. Defaults to the kind (`"fdc-765"`); repeated ids are refused. |
| `kind` | string | Required, unless `extends` already defined a card with this `id`. |
| `port` | u16 | The base port, where the kind is relocatable; Lo-tech EMS defaults to 260h and accepts 260h/264h/268h/26Ch with every 400h alias. |
| `frame` | u16 | Required for `lotech-ems`: segment C000h/D000h on `ibm-5170`, the only machine-file board; E000h is rejected. Only Rust bare test rigs accept E000h. Fixed 64 KiB MMIO, must match LTEMM `/p`. |
| `size_kib` | u32 | Lo-tech EMS only: 512/1024/1536/2048 KiB, default 2048. |
| `base` | u16 | SB16-only DSP base; default 220h; see [SB16](#sb16). |
| `dma8` | u8 | SB16-only byte-DMA channel; default 1. |
| `dma16` | u8 | SB16-only word-DMA channel; default 5; 5/6/7 allowed. |
| `controller_model` | `"ata"` or `"at-adapter"` | Only for `ide`; defaults to `"ata"`. Merges by card id like `port`. |
| `irq` | `0..15` or `"none"` | An IRQ on a kind with no interrupt source is an error. |
| `dma` | `0..3` or `"none"` | Only for kinds that master DMA. |
| `drives` | array of tables | Only for disk controllers. |
| `remove` | bool | Only in a file that `extends`. It deletes the inherited card. |

Lowering refuses a repeated card id and names both defining origins. This
includes repeated defaulted kinds and collisions between an explicit id and a
kind-derived id. A missing kind does not synthesize an id from the array index.

Kinds in schema 1. Each card lowers to one `DeviceConfig`, plus any MMIO
windows and drives. A port override is accepted only where the device model
honours it. The MDA, VGA and µPD765 models decode fixed absolute ports (READ,
`devices/src/{mda,vga_text,vga,upd765}.rs`).

| `kind` | `DeviceId` | Default ports | Port override | Default IRQ | Default DMA | MMIO (fixed) | Drives |
|---|---|---|---|---|---|---|---|
| `mda` | `Mda` | `0x3B0–0x3BB`, `0x3BF` | no | — | — | `0xB0000`, 32 KiB | — |
| `vga-text` | `Vga` | `0x3D4–0x3D5`, `0x3D8–0x3DA` | no | — | — | `0xB8000`, 32 KiB | — |
| `vga` | `Vga` | `0x3B4–0x3B5`, `0x3BA`, `0x3C0–0x3C2`, `0x3C4–0x3CA`, `0x3CC`, `0x3CE–0x3CF`, `0x3D4–0x3D5`, `0x3DA` | no | — | — | `0xA0000`, 128 KiB | — |
| `uart-16550` | `Uart` | `base…base+7`, base `0x3F8` | yes, any base ≤ `0xFFF8` | 4 for `0x3F8` and `0x3E8`, 3 for `0x2F8` and `0x2E8`, otherwise required | — | — | — |
| `fdc-765` | `Fdc` | `0x3F0–0x3F7`; with primary IDE: `0x3F0–0x3F5`, `0x3F7` | no (`0x370` is reserved) | 6 | 2 | — | `unit` 0–1 |
| `sb16` | `Sb16` | `base+0..6/8/9/Ah/Ch/Eh/Fh`, `388h–38Bh`; base `0x220` | `base`: 220h/240h/260h/280h | 5 (2/5/7/10 allowed) | `dma8`: 1 (0/1/3 allowed), `dma16`: 5 (5/6/7 allowed) | — | — |
| `ide` | `Ide` | `0x1F0` word data, `0x1F1–0x1F7` byte registers, `0x3F6` byte control | `0x1F0` or `0x170`; the control port is base + `0x206` | 14 (15 for `0x170`) | — (PIO) | — | `channel` master, slave |
| `lotech-ems` | `LotechEms` | base 260h, four byte ports, every 400h alias | 260h/264h/268h/26Ch | — (no IRQ) | — (no DMA) | 64 KiB at required C000h/D000h segment; E000h rejected by `ibm-5170`, the only machine-file board; only Rust bare test rigs accept E000h | — |

The Lo-tech board's physical jumper default is E000h. IBM 5170 decodes
E0000h–FFFFFh as system-board ROM, so its EMS frame must instead be C000h or
D000h and avoid other mappings. A machine file must specify the segment and
use the same LTEMM `/p` switch; for example `frame = 0xD000` and
`DEVICE=LTEMM.EXE /p:D000 /n`. No IRQ or DMA setting is accepted.

`vga` selects register-level VGA with guest fonts and DAC colours, and starts
unprogrammed without an option ROM. `vga-text` remains the existing machine
default. Both kinds are accepted in TOML and YAML; attaching both fails with
`vga and vga-text cards are mutually exclusive (shared ports and MMIO)`.
The full VGA aperture also conflicts with MDA. See
[machine-config.md §Display cards](machine-config.md#display-cards).

When an IDE card uses the primary channel, lowering gives the FDC card
`0x3F0–0x3F5` and `0x3F7`, using `FdcCfg::port_windows(true)`. The IDE control
register at `0x3F6` is a plain window; neither card needs an overlap flag.
Machines without primary IDE keep the unsplit `0x3F0–0x3F7` window through
`FdcCfg::port_windows(false)` (READ, `presets.rs`).

The IBM Personal Computer AT Technical Reference, System Board, **I/O Address
Map** lists `0x3F0–0x3F7` for the diskette controller and `0x1F0–0x1F8` for fixed
disk. Its **Fixed Disk and Diskette Drive Adapter** section identifies the
fixed-disk register at `0x3F6` and the diskette registers at `0x3F2`, `0x3F4`,
`0x3F5` and `0x3F7`. Decoding `0x3F0–0x3F5` plus `0x3F7` is this model's choice:
unused `0x3F0`, `0x3F1` and `0x3F3` stay with the FDC, as in the unsplit window.
This model leaves all of `0x3F7` with the FDC: digital
input register on read and configuration control register on write. The real
adapter shares `0x3F7` read bits with the fixed disk; that bit-level sharing is
not modelled.

To install the fixed-disk half of the AT combination adapter without a disk,
add this card alongside the existing `fdc-765` card on `ibm-5170`:

```toml
[[card]]
kind = "ide"
controller_model = "at-adapter"
```

YAML equivalent: `card: [{kind: ide, controller_model: at-adapter}]` (append
to the existing card list). No `drives` entry is required; the usual fixed-disk
firmware provider is still required. The board and presets do not automatically
add the adapter, preserving their decode maps and golden identities. Omitting
`controller_model`, or selecting `"ata"`, preserves existing machine behaviour.

The adapter latches 1F2h–1F6h without a disk, so cylinder-low 55h/AAh reads
back. Port 1F1h retains its separate write-precompensation/read-error meanings.
The IBM PC AT Technical Reference, **Fixed Disk and Diskette Drive Adapter**,
**Command Register** (p. 7), inhibits drive commands without ready/seek-complete
and clears IRQ14 on every command write. **Diagnose** (p. 10) completes adapter
self-tests with an interrupt: 90h restores the signature/error 01h and raises
IRQ14 even without disks. This controller command is independent of drive
readiness. **I/O address table** (p. 11) makes 3F6h write-only; reads return FFh
without acknowledging IRQ. Status reads acknowledge the shared IRQ14 across
drive selections (pp. 5–6). ASSUMED exact idle status without a drive: 00h.
The option is accepted on every board, including Bare Rust rigs. Attached-drive
PIO remains ATA. See [machine-config.md §The fixed-disk (IDE/HDD) attachment
seam](machine-config.md#the-fixed-disk-idehdd-attachment-seam) for sources,
snapshot compatibility and the WD1003 coverage limits.

The decode map sorts by `(start, end)` and uses the first match. A flagged
overlap is accepted only for strict containment with the same starting port,
so the narrower window wins every shared port, as with a POST latch at `0x80`
over a DMA page range starting at `0x80`. A contained window starting later,
a partial overlap, or identical windows is rejected; split the wider window
or move one (READ, `build_decode_map`, `check_port_overlaps`).

Schema 1 rejects a second card of the same kind with `DuplicateDeviceId`,
because `DeviceId` is one per role. Supporting COM2 later needs instance ids in
`DeviceId`, an additive change.

### Drives

Under an `fdc-765` card:

| Field | Type | Default | Notes |
|---|---|---|---|
| `unit` | 0 or 1 | required | The drive-select line (µPD765 US0/US1). Lowers to `controller = Fdc, unit`. |
| `type` | `"1.44M"`, `"1.2M"`, `"720K"`, `"360K"` | required | the drive type, `FloppyGeometry::{HD_1_44, HD_1_2, DD_720K, DD_360K}`; CMOS 10h encodes it |
| `image` | path | required | read and hashed during lowering; stored `Inline` for volatile/readonly, `File` for journal; its size must be a medium the drive type takes (below) |
| `sha256` | hex | none | pins the base image |
| `write` | `"volatile"`, `"readonly"`, `"journal"` | `"volatile"` | M3 |
| `journal` | path | required if and only if `write = "journal"` | relative to the canonical directory of its defining layer, like every machine-file path; never created implicitly (`persistent-media.md`) |
| `keep_generations` | positive integer | 10 | journal only (`persistent-media.md`) |
| `remove` | bool | | only in a file that `extends` |

The drive type and the medium are separate. A medium's format follows from
its image size, and a drive takes these media (`FloppyGeometry::accepted_media`,
the floppy controller's rule):

| Drive `type` | Media it takes | Rate the medium answers at |
|---|---|---|
| `1.44M` (3.5-inch) | 1.44M, 720K | 500 kbps, 250 kbps |
| `1.2M` (5.25-inch) | 1.2M, 360K | 500 kbps, 300 kbps (double-stepped) |
| `720K` (3.5-inch) | 720K | 250 kbps |
| `360K` (5.25-inch) | 360K | 250 kbps |

An image of any other size fails with `ImageGeometryMismatch`, whose message
names the drive type and its accepted media, for example `drive 0 image is
1228800 bytes but a 1.44M drive takes 1.44M (1474560 bytes) or 720K (737280
bytes) media`; a catalogue entry gets the same text under its `media[N]` field.
The firmware still decides which drive types it supports; see
`floppy_disabled_reason`.

Under an `ide` card, the fields are the same, with two replacements:
`channel = "master" | "slave"` instead of `unit`, and
`geometry = { cylinders, heads, sectors }` (512-byte sectors) instead of
`type`. In schema 1 the geometry is required: cylinders 1–16383, heads 1–16,
sectors 1–63, with 512-byte sectors and an image of exactly the implied size.
A slave requires a master. IDE accepts `volatile` (default), `readonly` and
`journal`, with the same explicit `journal` path and positive `keep_generations`
(default 10) as floppy drives. Paths remain relative to their defining layer.
A journal requires an immutable file-backed image; it replays before guest
execution. ATA journals record complete 512-byte sectors, including completed
sectors from an interrupted multi-sector command; a partial sector is discarded.
The existing offline media tools derive capacity from the base image length and
use the same format and arguments for IDE and floppy journals.

No write mode modifies the base image: `volatile` keeps guest writes in
memory, `readonly` faults them, and `journal` appends them to a separate file.
So a `sha256` pin stays valid in every mode.

Journal bases retain `ImageSource::File` because assembly requires a file path
for alias checks and mounting. Lowering reads and checks the image's hash and
size, and stores the verified digest in `DriveConfig.expected_sha256`, even
without an explicit pin. Assembly checks its single mount-time read against that
digest before opening any journal or creating a lock. A changed base produces
`drive N image sha256 mismatch: expected HASH, actual HASH`, including when an
existing journal would also reject its base binding. The checked buffer supplies
both the journal binding and the mounted bytes; no later base read intervenes.
Volatile/readonly images still carry their verified bytes inline, with no digest
field set. Repository fixture machines keep their existing write modes; persistence
goldens use temporary bases and journals and create no fixture sidecars.

`ImageReader::read_journal_base` refuses journal mode by default. Synthetic
readers can still supply volatile/readonly bytes, with mandatory pin checks;
they cannot silently substitute on-disk bytes when journaling. `FileImageReader`
implements journal reads from the immutable file. A custom file-backed reader
may implement that method under the same reproducible-base contract. This keeps
the journal usable after restart and by offline tools, which verify the original
base's digest as specified in [Persistent media](persistent-media.md).

Before assembly, the loader rejects journal paths that name directories or
alias any image it read or any machine-file layer, including layers whose
values were replaced. Canonical paths catch symlinks; Unix device/inode identity
also catches hard links. These errors carry the drive's defining file and line.
Empty journal paths are rejected as missing explicit paths. Assembly retains
its own output/input and output/output alias checks.
The loader uses the shared media predicates for reserved temporary names.
Image filenames containing `.retain-` are refused in every write mode, including
through symlinks. Journal inputs and layers also cannot occupy the journal's
creation-temporary namespace or use reserved retention names.

### Outside the file: run options, presentation, host streams

The machine file never carries these. A top-level `run`, `presentation` or
`streams` table is rejected with a message that names where the option belongs.

- **RunPolicy** (virtual-time cap, idle limit, stop condition, keyboard
  stimuli) comes from CLI flags, and later from a separate run file (reserved,
  [Reserved names](#reserved-names)).
- **Presentation** (scale, scanlines, fullscreen, which display to show) comes
  from window flags.
- **Host streams.** A host-connected endpoint is addressed by card id: `com1`,
  or `<card>.<endpoint>` for cards with several endpoints. The run side binds
  it, for example `--stream com1=file:serial.log`. An unbound endpoint behaves
  as today: TX is observable through observation, RX is empty. RTC from host
  time is the run option `--rtc-start host`. It samples host time once, before
  the first instruction, and records the value in the run log so a replay can
  reuse it. Schema 1 implements no stream bindings. The addressing rule is
  fixed now so that cards never need a field for it.

Frontends:

- **Window:** `unemu [--machine FILE] [--display CARD] [--allow-unlisted-firmware]`.
  `--machine` defaults to `machines/at-freedos.toml` in the workspace. The
  default display is the first display card in file order, which in the FreeDOS
  file is the MDA the Xi8088 BIOS writes to. The positional
  `[bios-path [floppy-path]]` arguments are removed. A different BIOS or floppy
  is a small file that `extends` the FreeDOS file (see `machines/README.md`).
  The workspace path comes from `CARGO_MANIFEST_DIR` at build time, independent
  of the current directory. Running without the build workspace requires an
  explicit `--machine`. A missing default file is an error.
- **CLI:** `unemu machine check FILE [--allow-unlisted-firmware]` parses,
  merges, lowers, validates and assembles without executing the guest, including
  catalogue checks against image bytes, then prints errors with file and position.
  Journal inspection never writes: missing journals are valid with a creation
  note; recoverable tails/unsealed tips are valid with a next-mount recovery
  warning; interior damage or wrong base binding is invalid at the drive origin.
  Busy journals are valid with a not-inspected warning. Existing locks use offline
  shared inspection; a missing lock uses the same read-only scanner without
  creating a lock and reports that fact. Journal drives assemble as volatile.
  It exits 0 for a valid file,
  1 for an invalid one, 2 for usage errors. `unemu run --machine FILE
  [--max-steps N] [--idle-limit N] [--print CARD]` maps its flags onto
  `RunPolicy::natural` and prints a display card's text screen or a UART's
  stream. The default caps are 20,000,000 completed-step equivalents and 1,000,000
  idle PIT clocks; both flags accept unsigned decimal values, including zero.
  Without `--print`, the first display in file order is printed, falling back to
  the first UART, or no output if neither exists. UART capture is bounded to
  1 MiB; exceeding it is an observation error. Runs report their stop to stderr:
  halt, reset and budget exhaustion exit 0, execution/configuration/observation/
  persistence errors exit 1, and argument errors exit 2 (including `--print`
  selecting a configured card without printable output). Journals finish after
  normal halt or budget exhaustion; failed runs drop without a clean close.
  Maintenance warnings go to stderr. Both file commands accept
  `--allow-unlisted-firmware`; `--boot`, `--bios` and `media` remain available.

## Crates

### TOML: `toml`

`toml` 1.2.1+spec-1.1.0, published 2026-11-06 by toml-rs: MIT OR Apache-2.0,
MSRV 1.85, 124 releases, repository active on 2026-11-06 (READ, crates.io and
GitHub APIs). It implements TOML 1.1. With it, `toml` pulls 7 crates including
itself (MEASURED, `cargo tree -e normal`). Errors carry line, column and a
caret snippet (MEASURED):

```text
TOML parse error at line 6, column 1
  |
6 | mhz = 8
  | ^^^
unknown field `mhz`, expected one of `id`, `kind`, `port`, `irq`
```

There is no real alternative. `toml` is the ecosystem standard.

### YAML: comparison

`serde_yaml` was deprecated and archived on 2024-03-25 (READ). The candidates:

| Crate | Latest release | Maintenance (READ) | License | Engine | Normal deps incl. itself (MEASURED) | Error form (MEASURED) |
|---|---|---|---|---|---|---|
| `serde_yml` | 0.0.13, 2026-05-27 | Repository archived (last push 2026-05-28). RUSTSEC-2025-0068: "unsound and unmaintained", no patched version. Its `libyml` dependency has RUSTSEC-2025-0067. 0.0.13 replaced the engine with a shim over another crate (READ, search results; not inspected). | MIT OR Apache-2.0 | changed between releases | not measured | not measured |
| `serde_norway` | 0.9.42, 2024-12-21 | A maintained fork of `serde_yaml`; repository last pushed 2025-08-04; no release for 23 months | MIT OR Apache-2.0, engine MIT | `unsafe-libyaml-norway` (libyaml transpiled by c2rust, `unsafe`) | 14 | `card[0].irq: invalid type: string "four", expected u8 at line 7 column 10`: field path and position, no snippet |
| `serde_yaml_ng` | 0.10.0, 2024-05-26 | Last push 2025-09-14, 21 open issues. Uses `unsafe-libyaml`, which the RustSec advisory text describes as unmaintained. | MIT (0.10) | `unsafe-libyaml` | not measured | not measured |
| `serde_yaml2` | 0.1.3, 2025-05-12 | 11 stars; RustSec lists it as an "incomplete" pure-Rust alternative | MIT OR Apache-2.0 | `yaml-rust2` | not measured | not measured |
| `serde-saphyr` | 2.1.1, 2026-11-05 | 40 releases since 2025-09-27; repository pushed 2026-11-09; 244 stars; 2 open issues | MIT OR Apache-2.0 | `granit-parser`, pure Rust, a fork of `saphyr-parser` by the same author, targeting YAML 1.2 | 15 (`default-features = false, features = ["deserialize"]`) | `error: line 6 column 5: unknown field` plus a caret snippet; duplicate keys are an error by default |

Further evidence:

- **License policy:** `cargo deny check licenses` and `cargo deny check
  advisories` pass for `toml`, `serde-saphyr` and `serde_norway` under this
  repository's `deny.toml` (MEASURED).
- **Untyped scalars:** both YAML crates deserialize `model: 80286` into a
  `String`, and `0x3F8` into an integer (MEASURED). The "Norway problem" does
  not reach string fields: `model: no` stays `"no"` (MEASURED).
- **`serde-saphyr` hardening (READ, docs.rs):** `unsafe` is denied in the
  crate itself (not in its dependencies). The parser is fuzzed and documented
  not to panic on malformed input. Budgets limit parser events and alias
  replay. A `strict_booleans` option rejects legacy `yes`/`no`/`on`/`off`, but
  accepts case variants of `true` and `false`; the adapter further requires
  lowercase literals. The crate has its own `Spanned<T>`.
- **`serde-saphyr` risks (READ):**
  - **MSRV.** Releases from 1.2.0 (2026-08-30) on declare rust-version 1.89.
    The workspace declares Rust 1.99, pins x86jit `060d446` (rust-version
    1.95, inherited by `x86jit-core` from its workspace; READ), and the Nix
    devShell provides rustc 1.99.1 (MEASURED), so the YAML
    dependency's 1.89 minimum is satisfied. The original investigation used
    x86jit `96a49a9` (rust-version 1.88) and measured a Nix toolchain of 1.95.0;
    those are historical values.
  - **Churn.** Two major versions within three months (1.0.0 on 2026-07-31,
    2.0.0 on 2026-10-13).
  - **Single maintainer.** One author maintains both the serde layer and the
    parser.

### Recommendation

**YAML: `serde-saphyr` 2.x** (`default-features = false, features =
["deserialize"]`), with `strict_booleans`, the default duplicate-key error,
tightened budgets, and single-document input. It is the only candidate that
is actively released, pure Rust, free of advisories, and gives positioned,
snippet-style errors equal to `toml`'s. The churn risk is contained, because
the schema uses only plain serde derive, so the YAML crate sits behind a
syntax adapter. **Fallback: `serde_norway`**, which passed the same probes
(MEASURED). It is unsafe C-derived code with a slow release cadence, but its
errors include the field path. `serde_yml` and `serde_yaml_ng` are excluded on
advisory and maintenance grounds.

The implemented adapter allows bounded anchors: at most 32 anchors and 64
aliases, with at most 16 expansions per anchor, 2,000 replayed events and an
8-level replay stack. Per parse, it permits one document, depth 32, 20,000
events, 10,000 nodes, 1 MiB of scalar text, 2,000 retained anchor events,
64 KiB of retained anchor text and 32 merge keys. Default duplicate-key errors
and strict booleans apply to every parse, including the schema peek and origin
pass. YAML 1.2 leaves timestamps as strings, so the origin pass explicitly
checks that an RTC start's definition is quoted, including through an alias.

YAML is a second syntax for the same serde model, so its extra scalar forms
cannot add values that TOML cannot express. Null is rejected everywhere: mapping
values, sequence items and keys, including `~`, `null`, empty values and
comment-only values. The diagnostic is "null is not allowed; omit the field
instead". Omission remains distinct from explicit empty strings or lists.
All explicit tags, including `!!str` and `!!binary`, are rejected. Integer
fields require integer scalars; quoted numeric text remains a string. Integer
fields reject leading-zero decimal spellings and report overflowing integer
literals as out of range. Radix prefixes are lowercase (`0x`, `0o`, `0b`).
Unquoted decimal-looking string fields retain their original spelling. Boolean
fields require unquoted lowercase `true` or `false`; uppercase plain boolean
spellings are rejected (quote them when a string is intended). The recursive
serde adapter applies the integer and boolean contracts at every table depth,
while typed string fields still accept unquoted text such as `model: 80286`.
The raw syntax pass shares the event, nesting, scalar-byte and document limits
above; serde-saphyr also bounds parsed nodes, anchors and alias replay. Alias-limit
diagnostics name the written anchor or alias when available and state the limit.

Repository syntax equivalence starts from every `machines/**/*.yaml` and
`*.yml`: each must have a `.toml` twin with the same stem and lower to the same
configuration. Having both YAML extensions for one stem is an error. TOML-only
files are permitted.

Other new dependencies: `serde` (derive; already in the lock file
transitively) and `sha2` 0.11.1 (MIT OR Apache-2.0, MSRV 1.88, READ) for
`sha256` pins. All of them live in a new crate, `crates/machine-file`, above
`assembly`. The `assembly` crate stays free of serde and of format crates.

## Versioning, extends and errors

### `schema`

- The loader first reads only `schema` from the file (a small serde struct
  without `deny_unknown_fields`), then parses the whole file with that
  version's document types.
- A missing `schema` is an error. A value above the newest supported version
  is an error: "file uses schema 2; this build reads schema 1".
- **Additive changes stay at schema 1:** a new card kind, a new optional
  field, a new enumerated value. Because unknown fields are rejected, an older
  build rejects a newer file with a positioned "unknown field" or "unknown
  variant". It never misreads the file.
- **Breaking changes** (a rename, a move, a changed meaning) bump the schema.
  Version 1's types are frozen in `schema::v1`, and a pure function
  `v1::Layer → v2::Layer` upgrades them. Every layer of an `extends` chain is
  upgraded to the newest version before merging, so a schema 2 file may extend
  a schema 1 preset. A golden test asserts that every repository file of an
  older schema still lowers to the same `MachineConfig`. Dropping support for
  an old schema needs a decision. A `machine migrate` command, which needs a
  serializer for the document types, is reserved.

### Strictness

- `deny_unknown_fields` everywhere (MEASURED working with `toml`,
  `serde-saphyr` and `serde_norway`).
- **Cycle-exact timing fields** (`mhz`, `frequency`, `clock_hz`, `cycles`,
  `cycle_exact`, `wait_states`, `bus_mhz`, `memory_wait_states`, `timing`, in
  `[cpu]`, `[board]`, `[[card]]` or at the top level) are declared fields
  whose deserializer always fails. The error therefore points at the value and
  explains: "cycle-exact timing is not modelled; `[cpu] rate` is a ratio of
  completed steps per PIT input clock". A plain unknown-field error would not
  explain this.
- **Reserved names** ([Reserved names](#reserved-names)) are recognised and
  rejected during parsing with "reserved, not implemented (schema 1)" and an
  explanation. They never enter the typed document or lowering. Rejection
  fields reject their mere presence, including a null value in YAML.

### `extends`

- **Forms:** `extends = "../machines/at-freedos.toml"` (relative to this file)
  or `extends = "preset:ibm-5170"`. Built-in presets are the files under
  `machines/presets/`, embedded with `include_str!`. Presets carry hardware
  shape only: a path in a preset is a build-time test failure, because an
  embedded file has no directory to resolve against.
- **Syntax is per file:** a YAML file may extend a TOML file and vice versa.
- **Path resolution happens per layer, before merging.** Every path is made
  absolute against the directory of the file that wrote it. A child that
  overrides `image` resolves it against the child's directory.
- **Chain limits:** depth at most 8 layers, including the entry file and any
  preset. Cycles are detected on canonical paths and
  preset names, and the error prints the chain
  (`a.toml → b.yaml → a.toml`). Cycle and completeness chains use the
  caller's spelling for the entry and exactly the string in each declaring
  `extends` line for every parent.
  Filesystem identities and diagnostic spellings are tracked separately.
- **Merge rules** (the child is applied over the parent):

  | Item | Rule |
  |---|---|
  | scalars | the child replaces the parent |
  | tables (`board`, `board.rtc`, `cpu`, `memory`, `firmware.bios`) | merged field by field, recursively |
  | a two-form value whose form changes (`nvram = "derived"` over a table) | replaced whole |
  | `provides`, `firmware.option_roms` | replaced whole when the child sets them |
  | `[[card]]` | keyed by `id`. A matching card merges field by field, and its `kind`, if stated, must equal the parent's. `remove = true` deletes the card. A new `id` is appended. Parent order is kept; new cards follow in child order. |
  | `drives` inside a card | keyed by `unit` or `channel`. A match merges field by field; `remove = true` deletes; a new key is added. Lowering sorts drives by key. |
  | `name`, `description` | not inherited |
  | `schema` | every file declares its own |

- **Completeness is checked after the merge.** A preset may lack
  `[firmware]` or drive images. The final machine must have every required
  field, and the error names the field path and the files of the chain.

Implementation choices where the original merge table left details open:

- Parent file names must be relative; absolute `extends` paths are rejected.
  The canonical directory of the declaring layer is the base, including when
  that layer is a symlink. The joined parent spelling (including `..`) is
  retained in per-value origins; these use the layer's I/O path and may
  include absolute joined prefixes and `..`. The entry's origins preserve
  the caller's path. Cycle and completeness chains instead keep the written
  names. Canonical paths are used only for identity and image bases. Each
  layer still selects syntax by its spelled extension.
- `remove` is a directive, consumed before lowering. Its presence, including
  `remove = false`, requires `extends`; false performs a normal merge.
  A true card tombstone accepts only `id` and `remove`; a drive tombstone
  accepts only its key and `remove`. Deleting an unknown inherited key fails.
- Repeated effective card ids or drive keys within one layer are errors,
  with the first definition's origin. A drive cannot state both `unit` and
  `channel`. Empty keyed card/drive arrays perform no edits; clearing inherited
  entries requires tombstones. New entries require their own identity and a
  new card requires `kind` in its defining layer.
- NVRAM image tables of the same form and drive CHS geometry tables merge
  recursively. Changing NVRAM form replaces the entire value. For firmware,
  `image` OR `parts`, `sha256` and `provides` form one override unit: setting a
  source replaces the inherited source and drops both the inherited pin and
  services unless the child restates them. Known images then use catalogue
  services; an unlisted image under the run-only escape must restate required
  services (otherwise the existing `NoProvider` error names the affected card).
  This applies to BIOS and option ROMs; option ROM arrays still replace whole,
  so their entries restate image/base. A pin-only override re-pins the inherited
  source, and a provides-only override restates its services.
  For NVRAM and drive images, `image` and `sha256` keep their existing override
  unit: a new image clears an omitted pin, and a pin alone re-pins the image.
  For drives and NVRAM, `image`, `sha256` and `journal` form one unit: overriding `image`
  drops the inherited journal and retention count. It clears the inherited write
  mode only if that mode was journal, falling back to volatile unless the child
  sets it again; inherited readonly and volatile modes keep their values and
  origins. A child that restates `write = "journal"` must supply a new journal
  path; otherwise lowering reports `required field is missing` at
  `card[<id>].drives[unit=<n>].journal` for drives or
  `board.rtc.nvram.journal` for NVRAM. This prevents an old journal from being
  bound to a new base. A journal-only override retains the image and resolves
  the journal against the child's directory.
  A child that explicitly sets `write = "volatile"` or `write = "readonly"`
  drops inherited `journal` and `keep_generations` without restating the image.
  Journal-only fields explicitly supplied by that child remain errors.
  A mismatch names the pin's defining file and line, even when the image path
  comes from a different layer.
- Merged scalar origins identify the value's line; table-level diagnostics
  identify the latest layer that edited that table. Inherited values keep
  their original file and line even when cards or drives move after removal.
  Image-directory provenance is separate from diagnostic spelling, so an
  inherited symlinked file still resolves images in its canonical directory.
- Completeness runs once, after the entire chain, before any image reads. It
  lists all missing required field paths with their closest source origins,
  followed by the root-first inheritance chain. Board defaults make `cpu`
  and `memory` optional. The check covers BIOS, option ROM and NVRAM images,
  option ROM bases, floppy keys/types/images, IDE channels/CHS/images and a
  UART IRQ when no port default exists. Hardware consistency remains solely
  assembly's responsibility.

The embedded `ibm-5170` preset supplies the four reference cards in canonical
order (MDA, VGA text, unwired UART, FDC), without a BIOS source or service
declaration. Each child supplies an image or parts; known firmware obtains its
services from the catalogue and unlisted firmware must declare required services.
This preserves the exact FreeDOS device order when images are added. Preset files may inherit other presets but may contain no
file paths, including `extends` file references and journal paths. Registration
and the absence of paths are checked by unit tests; loading checks paths too.

`machine_file::resolve` returns the complete typed document and its origins
without reading images. `load` uses it before lowering; `parse_toml` and
`parse_yaml` continue to parse single, potentially incomplete layers. The
caller's argument to `resolve`/`load` is always a filesystem path, even if its
name begins with `preset:`. That prefix is interpreted only in `extends`;
there is no implicit direct-preset entry point. A future frontend that needs
one must use a separate explicit API.

An existing parent that cannot be opened or read, uses an unsupported file
extension, or exceeds the size limit is reported at the child's `extends`
file and line, followed by the parent path and cause. Source
positions use a precomputed line-start index and ancestor-key lookups. Card
and drive edits keep stable slots and compact/reindex their origins once per
layer, rather than shifting every later entry after each removal.

### Errors

Every error starts with the file path. There are three classes:

1. **Parse errors** include syntax, type, unknown-field, reserved-name and
   rejected-field errors. The parser converts them into a format-neutral
   diagnostic: file path, message, optional line and character column, and an
   optional snippet. It retains no format-specific error source, so displaying
   an error chain does not repeat the diagnostic. TOML also retains its byte
   span.
2. **Lowering errors** (fields a kind does not accept,
   completeness, `sha256` mismatches, unreadable images) name the file and a
   field path such as `card[fdc-765].drives[unit=0].image`. A `sha256`
   mismatch prints both hashes. When an image's spelled path differs from the
   path read, image errors show `spelled/path (resolved to actual/path)`.
3. **`validate()` errors** carry `DeviceId`, drive index and image index. The
   loader keeps an origin table (`DeviceId` → card id and defining file,
   drive index → card and unit, image index → firmware entry) and prints them
   in file terms:

   ```text
   machines/at-ide.toml:6: card[ide]: device Ide needs firmware service fixed-disk; no ROM provides it
   ```

The TOML and YAML loaders keep table and per-value source positions in a
separate, format-neutral origin table. Lowering and validation diagnostics name
the defining file and source line, with card ids and drive units where applicable.
The merger retains per-field origins from each defining layer.
Programmatically constructed documents may omit positions. A diagnostic such
as `card #1.kind` uses a zero-based table index when neither an explicit id nor
a kind names the card: `card #1` is the second `[[card]]` table. In contrast,
`card[1]` names a card whose explicit id is `"1"`.

Validation messages retain assembly's wording, replacing drive/image index
prefixes with file labels. A duplicate-drive message includes each defining
file and line beside its unit label, so repeated units remain distinguishable.

`FirmwareProvidesMismatch` names the matched image's `provides` origin and
prints both declared and catalogue service sets. A recognized BIOS or option ROM uses
the catalogue set when `provides` is omitted or empty. This also applies under
the unlisted-firmware escape option; only unknown images use their declarations.

System BIOS catalogue diagnostics (`FirmwareNotListed`, `FirmwareBoardMismatch`,
`FirmwareNeedsCpu`, `FirmwareRequiresDevice`), `MissingSystemBios` and
`BiosPlacement` map to
`firmware.bios.image`, using a specific field origin when present and otherwise
the containing firmware table's file and line. Their messages retain assembly's
wording unchanged; none of these variants carries an image index.
`FirmwareReset` names the CPU origin; `SystemBiosOverlapsRam` names memory and
relates it to the BIOS image origin.

Option-ROM format, checksum, catalogue and card mismatch diagnostics name
`firmware.option_roms[index]` with its defining file and line; placement errors
name `.base`, and service mismatches name `.provides`. Overlaps include both
image origins. The underlying `ConfigError` remains available unchanged.

## Validation and presets

### One validation path

The loader's output is a function of the bytes of the machine file, its
`extends` chain and the images. It reads no environment variables and no
clock, and resolves no paths against the working directory, except for the
command-line argument itself. The pipeline:

```text
read file → peek schema → typed parse (per layer) → upgrade → merge
  → completeness → lower (board expansion, type defaults, overrides,
    RAM rule, image reads + sha256) → MachineConfig::validate()
  → assemble()  (checks file-source digests on its single read, validates bytes)
```

Volatile/readonly images are read exactly once, hashed, and stored as
`ImageSource::Inline`. Journal bases retain their file paths and verified digests
for the mount-time check described above. Image reading goes through
`ImageReader`, so tests can supply bytes without fixtures; its default journal
method refuses synthetic bases. Lowering computes SHA-256 from the returned
buffer and checks any declared pin; the reader cannot disable integrity checks.

`load()` returns after `MachineConfig::validate()`; it does not assemble hardware
or run the system BIOS catalogue check. Option-ROM membership and compatibility
are checked during lowering with source positions and caller-supplied
`AssemblyOptions` (strict by default). The loader caller passes the same options
to assembly. BIOS membership is
checked by assembly afterwards. A one-byte-changed Kiselev image with a declared
pin fails loading with a SHA-256 mismatch at the BIOS entry's file and line.
Without a pin it loads as inline bytes, then strict assembly returns
`FirmwareNotListed`. That assembly error does not itself retain machine-file
origins; this loader change does not add an assembly wrapper.

### Canonical order

Lowering order is fixed. Devices: board devices in board order, then cards in
merged file order. MMIO windows: sorted by base. Images: the BIOS, then option
ROMs in file order. Drives: card order, then unit or channel. The Rust `bios`
preset is rewritten to the same order (M2, bead 1), so the round trip below
compares equal with `==`.

### Presets as files

Repository layout:

```text
machines/
  at-freedos.toml        the FreeDOS machine (TOML)
  at-freedos.yaml        the same machine in YAML
  at-freedos-from-preset.toml  the same configuration via an embedded preset
  presets/
    ibm-5170.toml        embedded as preset:ibm-5170
```

What stays Rust:

- **The test rigs** (`at_boot`, `at_keyboard`, `at_rtc`, `at_vga_text`,
  `at_fdc_write`, `test386`, `floppy_boot`). They load inline blobs into RAM,
  start at `Entry` CS:IP with seeded segments, and leave IRQs unwired on
  purpose. They are harnesses, not machines a user composes. They use
  `BoardModel::Bare`. A `bare` board model is reserved in case a file ever
  needs them.
- **`bios(rom, None)`**, the no-floppy POST machine. Its FDC decodes nothing
  and it has no 8237, a state that real hardware never has. It stays a rig,
  so its POST golden is untouched.

### Round-trip and golden tests

- **Round trip without fixtures** (runs in plain `cargo test`):
  parse `machines/at-freedos.toml`, remove its real-fixture pins from the typed
  test document, and lower it with an `ImageReader` that returns a 32 KiB dummy
  ROM and a 1,474,560-byte zero-filled floppy. The entire result must equal
  `assembly::bios(rom, Some(floppy))` for the same bytes. Lowering always hashes
  the returned bytes; pins, when present, are always checked.
- **Syntax equivalence:** for every `machines/**/*.toml` with a `.yaml` twin,
  both lower to equal configs.
- **Pinned hashes, fixture-gated** (the existing `fixture()` policy): the real
  file loads with the pinned hashes (BIOS `1ad458ac…`, FreeDOS image
  `b7b08833…`, both READ from `vendor/*/README.md`).
- **Goldens load the files:** the FreeDOS goldens (`freedos_boot`,
  `dos_interactive`, `dos_write`) obtain their machine from
  `machines/at-freedos.toml`, and their expectations do not change.
  `bios_floppy_boot` boots a generated floppy, so it keeps the Rust preset.
- **Diagnostics:** one test per error class, asserting the file, the position
  or field path, and the message.
- **Repository hygiene:** no repository machine file uses an absolute path.

## Examples

### 1. The FreeDOS machine, TOML (`machines/at-freedos.toml`)

```toml
schema = 1
name = "at-freedos"
description = "PC/AT-class machine booting FreeDOS from drive A: with the Xi8088 open BIOS"

[board]
model = "ibm-5170"
post_port = 0x80

[board.rtc]
start = "2000-01-01T00:00:00"
nvram = "derived"

[cpu]
model = "80286"
rate = "8/13"

[memory]
conventional_kib = 640
extended_kib = 0

[firmware.bios]
image = "../vendor/kiselev-bios/bios-xi8088.bin"
sha256 = "1ad458acb1f22dee6b3d9b6bd95e83218a7ddc5389fa2b2c67a4001e689c81a8"
provides = ["floppy"]

[[card]]
kind = "mda"          # the Xi8088 BIOS console; window default: first display card

[[card]]
kind = "vga-text"

[[card]]
kind = "uart-16550"
port = 0x3F8
irq = "none"          # the reference machine leaves COM1 unwired

[[card]]
kind = "fdc-765"

[[card.drives]]
unit = 0
type = "1.44M"
image = "../vendor/freedos/freedos.img"
sha256 = "b7b088333f7f30584d768941fecea8288b63f2d61e13ec6c9472439eda66e51f"
write = "volatile"
```

It lowers to the canonical-order `assembly::bios(rom, Some(floppy))`:

- **devices:** `MasterPic`, `SlavePic`, `Pit` (IRQ 0), `Dma` (`0x00–0x0F`,
  `0x81–0x88`), `DmaWord` (`0xC0–0xDF`, `0x89–0x8F`), `Keyboard` (IRQ 1),
  `Rtc` (IRQ 8, NVRAM derived), `Coprocessor` (IRQ 13), `SysControl`,
  `Post` (`0x80`), `Mda`, `Vga`, `Uart` (`0x3F8`, no IRQ), `Fdc` (IRQ 6,
  DMA channel 2).
- **memory:** 16 MiB CPU address space (24 physical address bits), while the
  lowered installed-memory extent is 1 MiB; RAM `[0, 0xB0000)`; read-only
  zero-filled adapter space `[0xC0000, 0xF8000)`; MMIO `Mda@0xB0000` and
  `Vga@0xB8000`, each 32 KiB; FFh empty bus `[0x100000, 0xFE0000)` with
  discarded guest writes; read-only `[0xFE0000, 0x1000000)` copying the resolved
  low E0000h–FFFFFh bytes, including the BIOS and gap zeroes.
- **images:** ROM at `0xF8000`, 32 KiB.
- **drives:** 1.44 MB, `Fdc` unit 0, `Volatile`.
- **CPU and clock:** 80286, reset `F000:FFF0`, segments `flat(0xF000)`,
  rate 8/13.
- **board:** `Ibm5170`.

### 2. The same machine, YAML (`machines/at-freedos.yaml`)

```yaml
schema: 1
name: at-freedos
description: "PC/AT-class machine booting FreeDOS from drive A: with the Xi8088 open BIOS"

board:
  model: ibm-5170
  post_port: 0x80
  rtc:
    start: "2000-01-01T00:00:00"
    nvram: derived

cpu:
  model: "80286"
  rate: "8/13"

memory:
  conventional_kib: 640
  extended_kib: 0

firmware:
  bios:
    image: ../vendor/kiselev-bios/bios-xi8088.bin
    sha256: "1ad458acb1f22dee6b3d9b6bd95e83218a7ddc5389fa2b2c67a4001e689c81a8"
    provides: [floppy]

card:
  - kind: mda         # the Xi8088 BIOS console; window default: first display card
  - kind: vga-text
  - kind: uart-16550
    port: 0x3F8
    irq: none         # the reference machine leaves COM1 unwired
  - kind: fdc-765
    drives:
      - unit: 0
        type: "1.44M"
        image: ../vendor/freedos/freedos.img
        sha256: "b7b088333f7f30584d768941fecea8288b63f2d61e13ec6c9472439eda66e51f"
        write: volatile
```

YAML style rules for repository files: quote `description` when it contains
`": "`, as this one does. Unquoted, both YAML crates fail with "mapping values
are not allowed in this context" (MEASURED). Quote hashes, the CPU model, drive
types and dates. TOML's `[[card]]` array of tables is YAML's `card:` list.

All six example files in this section parse: the TOML ones with `toml` 1.2.1,
the YAML one with `serde-saphyr` 2.1.1. The TOML and YAML FreeDOS files
produce identical value trees (MEASURED).

### 3. An AT machine with an IDE card (future profile)

```toml
schema = 1
name = "at-ide"
extends = "at-freedos.toml"

[firmware]
# Hard-disk INT 13h comes from the listed XTIDE Universal BIOS AT build.
# Option-ROM identity is checked; the fixed-disk boot profile remains later work.
option_roms = [
  { image = "../vendor/xtide-universal-bios/ide_at-r638-8k.bin", base = 0xC8000, provides = ["fixed-disk"] },
]

[[card]]
id = "fdc-765"                 # inherited card: only its drive changes
[[card.drives]]
unit = 0
write = "readonly"             # keep the boot floppy pristine

[[card]]
kind = "ide"                   # 0x1F0-0x1F7 + 0x3F6, IRQ 14 (type defaults)

[[card.drives]]
channel = "master"
image = "disks/dos-hd.img"
geometry = { cylinders = 615, heads = 4, sectors = 17 }
write = "volatile"
```

Lowering adds `Ide` after `Fdc`, a ROM image at `0xC8000`, and a `HardDisk`
drive bound to `Ide`, master. RAM stays `[0, 0xB0000)`. Assembly creates
read-only zero-filled holes `[0xC0000, 0xC8000)` and `[0xCA000, 0xF8000)`,
assuming an 8 KiB ROM. No RAM backs either hole.
Without a fixed-disk provider, `validate()` returns `NoProvider` with the exact
message `device Ide needs firmware service fixed-disk; no ROM provides it`.
Lowering adds the card's file and table line. No `machines/at-ide.toml` is shipped:
the Kiselev catalogue entry provides only floppy services. The XTIDE fixture
and its `xub-r638-at` entry now exist and declare `fixed-disk`, consistently
with the IDE requirement. Option-ROM bytes now match the catalogue by SHA-256;
XUB inherits fixed-disk services and requires primary ATA at 1F0h/IRQ14.
The fixed-disk boot profile remains later work.

### 4. An XT-class file (rejected in schema 1)

```toml
schema = 1
name = "xt-5160"

[board]
model = "ibm-5160"             # one 8259, one 8237, 8255 PPI keyboard, no RTC
post_port = "none"

[cpu]
model = "8088"

[memory]
conventional_kib = 640

[firmware.bios]
image = "../vendor/xt-bios/bios.rom"   # an open XT BIOS; chosen by the firmware spike
provides = ["floppy"]

[[card]]
kind = "mda"

[[card]]
kind = "fdc-765"
[[card.drives]]
unit = 0
type = "360K"
image = "disks/dos-360k.img"
```

Expected result in schema 1 (the board is checked first, then the CPU):

```text
machines/xt-5160.toml: board model "ibm-5160" is reserved in schema 1: it needs
the 8255 PPI keyboard interface and an 8088 CPU model, neither of which is
modelled yet
```

`[board.rtc]` on this board would be rejected, because an XT board has no RTC.
A card IRQ of 8–15 would be rejected, because there is no slave PIC.

### 5. A preset (`machines/presets/ibm-5170.toml`, embedded)

```toml
schema = 1
name = "ibm-5170"
description = "Generic PC/AT: 80286, 640 KiB, MDA, VGA text, COM1 and floppy controller. No images."

[board]
model = "ibm-5170"

[cpu]
model = "80286"

[memory]
conventional_kib = 640

[firmware.bios]

[[card]]
kind = "mda"

[[card]]
kind = "vga-text"

[[card]]
kind = "uart-16550"
irq = "none"

[[card]]
kind = "fdc-765"
```

A user file then needs only its firmware and a drive:

```toml
schema = 1
extends = "preset:ibm-5170"

[firmware.bios]
image = "my-bios.bin"
provides = ["floppy"]

[[card]]
id = "fdc-765"
[[card.drives]]
unit = 0
type = "1.44M"
image = "dos.img"
```

`machines/at-freedos-from-preset.toml` follows this form with the same pinned
BIOS and floppy paths as `at-freedos.toml`. Both fixture-free and fixture-required
tests compare their complete lowered configurations, including device order.

## Reserved names

These names are recognised and rejected during parsing as "reserved, not
implemented", with an explanation of the missing model or feature. The firmware
spike owns the board-model and firmware entries.

| Where | Reserved |
|---|---|
| top level | `run`, `presentation`, `streams` (rejected with "belongs outside the machine file") |
| `[board] model` | `ibm-5160`, `ibm-5150`, `bare`, `compaq-deskpro`, `tandy-1000`, `i440fx-piix3`, `i440bx-piix4` |
| `[board]` | `a20 = "kbc" \| "port-92" \| "both"`, `kbc = "at" \| "ps2"`, `keyboard = "xt-83" \| "at-84" \| "mf2-101"`, `keyboard_layout`, `dip_switches` (XT), `chipset` (a table of chipset registers), `speaker` |
| `[board.rtc]` | `start = "host"` (rejected: this is the run option `--rtc-start host`) |
| `[cpu] model` | `8086`, `8088`, `v20`, `80386sx`, `80486`, `pentium`, `pentium-ii`, `pentium-iii` |
| `[memory]` | `shadow`, `umb`, `upper = "ram" \| "open-bus"` |
| `[firmware]` | `basic = { image, base }` (ROM BASIC); `provides` values `video`, `serial`, `network-boot` |
| `[[card]] kind`, video | `hercules`, `cga`, `ega`, `svga-et4000`, `svga-s3-trio64`, `voodoo`, `voodoo2`, `banshee` |
| `[[card]] kind`, sound | `adlib`, `sb1`, `sb2`, `sbpro`, `gus`, `mpu-401`, `covox` |
| `[[card]] kind`, disks | `xt-mfm`, `xt-rll`, `xt-ide`, `xt-cf-lite` |
| `[[card]] kind`, ports and others | `lpt`, `game-port`, `bus-mouse`, `ems`, `ne2000` |
| `[[card]]` fields | `monitor = "green" \| "amber" \| "white" \| "color"` (display cards), `rom = { image, base }` (a card-owned ROM), `mmio` (relocation) |
| drives | `type = "2.88M"`, `media = "none"` (an empty drive), `image = "blank"`, `geometry = "auto"`, `type = "atapi-cdrom"` on `ide` |
| `fdc-765` | `port = 0x370` (secondary controller) |
| separate file | a run file (`*.run.toml`, `[run]`) for RunPolicy, and a presentation file |
| rejected forever | `mhz`, `frequency`, `clock_hz`, `cycles`, `cycle_exact`, `wait_states`, `bus_mhz`, `memory_wait_states`, `timing` |

## Open questions for the maintainer

1. **MSRV — resolved.** `serde-saphyr` 2.1.1 needs rustc 1.89; the workspace
   declares 1.99, the Nix devShell provides 1.99.1, and the current x86jit pin
   `060d446` requires 1.95 (`x86jit-core` inherits its workspace's rust-version;
   READ).
2. **Device order.** Accept rewriting the `bios` preset into canonical order
   (bead 1, guarded by the slow goldens)? The alternative is a board model that
   encodes today's interleaved order, which is harder to explain.
3. **Default write mode.** This document uses `volatile`, as
   `persistent-media.md` does. Should `write` be required instead, so every
   drive states its persistence?
4. **Journal path base — resolved.** Every machine-file path, including
   `journal`, is relative to the canonical directory of the layer that wrote
   it, just like images. Diagnostics retain that layer's spelled file path.
   Programmatic Rust configurations may still use paths relative to process cwd.
5. **Card identity — resolved.** `id` defaults to the kind. Lowering rejects
   every repeated effective id and reports both origins. Different ids for a
   second card of the same kind still reach `DuplicateDeviceId` in assembly;
   supporting those instances requires instance ids in `DeviceId`.
6. **Window display — resolved.** Show the first display card in file order
   by default; `--display CARD` selects a display by its effective card id.
   Unknown ids and non-display cards are errors before the event loop opens.
7. **For the firmware spike:** the `provides` vocabulary; whether known ROMs
   are recognised by `sha256`; the XTIDE build for `ide`; the list of board
   models.

## Decision proposal

> **decision-17: machine description files.** A machine is described by a
> versioned file (`schema = 1`) in TOML or YAML. Both syntaxes deserialize
> into one serde data model, using `toml` and `serde-saphyr`, in a
> `machine-file` crate above `assembly`. The file describes hardware only:
> `[board].model` supplies the motherboard devices and wiring, and every ISA
> device is a `[[card]]` with default resources that the file can override.
> `[cpu]`, `[memory]` and `[firmware]` (with `provides`) complete the machine,
> and drives nest under their controller card with a `write` mode of
> `volatile`, `readonly` or `journal`. RunPolicy, presentation and host-stream
> bindings stay outside the file. Paths are relative to the file that names
> them, and images may pin `sha256`. `extends` inherits from a file or from an
> embedded preset, merging tables field by field and cards by `id`. Unknown
> fields are errors, and cycle-exact timing fields are rejected with an
> explanation. A file lowers to a `MachineConfig` that passes through the
> existing `validate()`, with no second validation path. Images are read once,
> hashed and inlined. Additive hardware stays schema 1, and only a breaking
> change bumps the schema, with a migration function per version. Schema 1
> implements today's machine plus IDE, the floppy journal, NVRAM and the RTC
> start; every other name in the reserved list is fixed now and rejected until
> implemented. Rust test rigs remain Rust presets on a `Bare` board.

## Implementation beads

Ordered; each bead is narrow and lands green.

1. **Canonical device order in the `bios` preset.** Reorder `assembly::bios`
   devices to board order then cards (`mda, vga, uart, fdc`), and sort its
   MMIO by base.
   *Accept:* `cargo test --workspace` and `-- --ignored` (slow goldens) pass
   unchanged. machine-config.md states the order rule. If any golden changes,
   stop and report the dependency instead of re-blessing.
   *Depends on:* none.
2. **Board model and drive binding in the model (M1, M3 without journal,
   M4).** Add `MachineConfig.board` (`Ibm5170`, `Bare`),
   `DriveConfig { controller, unit, write: Volatile | ReadOnly }`, and
   board-dependent IRQ and DMA checks in `validate()`, all presets updated.
   *Accept:* every golden is unchanged, and each new `ConfigError` variant has
   a unit test.
   *Depends on:* 1.
3. **Firmware `provides`, RTC start and NVRAM source (M6, M7 without
   journal).**
   *Accept:* the default start and `Derived` NVRAM leave every RTC register
   and NVRAM byte unchanged (goldens unchanged). A unit test reads back a
   non-default start through ports `0x70`/`0x71`. A 64-byte NVRAM image is
   loaded verbatim. The provides rules are unit-tested.
   *Depends on:* 2.
4. **`machine-file` crate: schema 1 types and TOML parsing.** Peek `schema`,
   use flat card and drive structs, two-form visitors, cycle-exact rejection
   fields, reserved names, `toml` 1.x, and errors prefixed with the file path.
   *Accept:* one positioned-error test each for: unknown field, wrong type,
   unknown kind, reserved kind, a cycle-exact field, `schema = 2`, missing
   `schema`, and a `run` table. `cargo deny check` passes.
   *Depends on:* 2.
5. **Lowering to `MachineConfig`.** `ibm-5170` expansion, card type defaults
   and overrides (with the per-kind table), the RAM rule, per-file path
   resolution, `ImageReader` with `sha2`, `Inline` images, the origin table,
   and `validate()`. Add `machines/at-freedos.toml`.
   *Accept:* the fixture-free round trip equals `assembly::bios(…)`.
   Fixture-gated: the real file loads with the pinned hashes. A `sha256`
   mismatch prints both hashes. When an image's spelled path differs from the
   path read, image errors show `spelled/path (resolved to actual/path)`. A `validate()` failure prints the card id and
   the file.
   *Depends on:* 1, 3, 4.
6. **YAML syntax.** `serde-saphyr` (strict booleans, duplicate-key error,
   budget, one document), `machines/at-freedos.yaml`.
   *Accept:* the TOML/YAML twin equality test passes. The YAML error tests
   mirror bead 4's. `cargo deny check` passes. The MSRV question (open
   question 1) is resolved in the same change.
   *Depends on:* 5.
7. **`extends` and presets.** Layering, merge rules, cycle and depth errors,
   `preset:` names embedded from `machines/presets/`, the completeness check,
   and `machines/presets/ibm-5170.toml`.
   *Accept:* a unit test per merge-table row. A cycle error prints the chain.
   YAML extends TOML. A preset containing a path fails. A file extending the
   preset lowers equal to `at-freedos.toml`.
   *Depends on:* 5 (6 for the cross-syntax test).
8. **Frontends and goldens load files — implemented.** The window's `--machine` and
   `--display`; the CLI's `machine check` and `run --machine`; the FreeDOS
   goldens obtain their machine from `machines/at-freedos.toml`. Update
   machine-config.md (non-goal removed), commands.md and status.md.
   *Accept:* the slow goldens are unchanged, the window boots FreeDOS from the
   file, `machine check` exit codes are tested, and no repository machine file
   has an absolute path.
   *Depends on:* 6, 7.
9. **Journal write mode in files.** Add `write = "journal"`, `journal` and
   `keep_generations` on floppy and IDE drives, mapped to `WriteMode::Journal`.
   *Implemented:* TOML/YAML lowering, per-layer journal paths, journal/image
   override units and two-run guest WRITE/READ acceptance with immutable bases.
   NVRAM journals use the same override units and offline tools; readonly is rejected.
   *Accept:* a file-loaded machine journals guest writes and a reload replays
   them. A fixture image is never journaled implicitly. Goldens stay volatile.
   *Depends on:* the persistent-media implementation beads, and 5.
10. **IDE card kind and device (M5).** `DeviceKind::Ide`, the ATA PIO
    register device with isolation tests on a mock bus, channel binding, the
    `0x3F6` control window and split FDC windows.
    *Implemented:* assembly and TOML lowering, readonly/volatile sector media,
    exact validation diagnostics, secondary channel, and assembled IDENTIFY
    with a 256-word `rep insw` transfer. Provider tests use a synthetic BIOS
    through the run-only escape option; CPU integration uses a Bare rig.
    The XTIDE fixture and catalogue entry are available; `machines/at-ide.toml`
    still waits for the fixed-disk boot profile. The missing
    provider diagnostic is covered by an inline TOML test.
    *Accept:* device unit tests pass. `validate()` rejects an IDE card without
    a `fixed-disk` provider. A golden boots from a hard-disk image once the
    firmware spike names the option ROM.
    *Depends on:* 2, 3, 5, and the firmware decision.
11. **M8: `OutputConfig` out of `MachineConfig`.**
    *Implemented:* `ObservationPolicy.output` owns Headless/Capture selection;
    `Observation::bind` applies it to bound text routes for assembly and inline
    hosts. CLI capture and window observation
    use the policy, while inline hosts retain `Observation::configure_output`.
    *Accept:* sink selection moves to the observation setup, and goldens are
    unchanged.
    *Depends on:* 8.

## SB16

An optional schema-1 ISA card supplies 8-bit and 16-bit DMA output; it is absent from all
presets. TOML and YAML use the same fields:

```toml
[[card]]
kind = "sb16"
base = 0x220
irq = 5
dma8 = 1
dma16 = 5
```

Defaults are 220h, IRQ5, DMA1 and word DMA5. Allowed bases are 220h/240h/260h/280h
(**ASSUMED** jumper-style subset). IRQ2/5/7/10 and DMA0/1/3 follow Creative's
*Sound Blaster Series Hardware Programming Guide*, §2 Configuring DMA and
Interrupt Settings. IRQ2 selects the AT slot's IRQ9 route. The `base`, `dma8` and `dma16`
keys are SB16-only; `port` and `dma` are rejected for this card. Word DMA accepts
5/6/7, requires the board's word controller, and participates in channel-conflict
validation independently of byte DMA. Layered files may override base/irq/dma8/dma16 by card id, or remove the
card using the existing keyed-card rules.

Base+4/+5 (mixer), +6/+Ah/+Ch/+Eh (DSP) and +Fh (16-bit acknowledgement)
are claimed. OPL3 additionally claims 388h–38Bh and base+0..3/+8/+9. Port and IRQ conflicts use existing diagnostics; duplicate
DMA ownership on one controller reports both card identities. Invalid resources
also retain the owning card's source location. See [audio.md §SB16 8-bit
output](audio.md#sb16-8-bit-output) for supported commands, exact rates,
FIFO policy, IRQ timing and exclusions.

The `sb16` card always includes a Yamaha YMF262 (OPL3). Besides its DSP/mixer
ports, it claims 388h–38Bh, base+0..3 and base+8/9 as byte ports, including the
standard bank aliases. These participate in normal port-conflict validation;
there is no `opl3` switch. See [audio §OPL3](audio.md#opl3) for sample timing,
timers and the native FM source.

## Removable floppy catalogues

Schema 1 keeps the single `image` form unchanged and adds an alternative
`media = [{ image, sha256?, write?, journal?, keep_generations? }, ...]` on a
floppy drive. A separate `empty = true` starts with no medium; it permits a
zero-entry catalogue too. Otherwise entry zero is mounted at power-on. A child
that replaces `media` also resets inherited `empty` unless it restates that field. YAML
uses the identical entry fields in a sequence. This additive form preserves
existing single-image machine configurations and their golden identities.
Drive-level image/pin/write/journal fields cannot accompany `media`/`empty`.
Every entry must be a medium the declared drive `type` takes (§Drives), and
entries may differ: a 1.44M drive's catalogue may mix 1.44M and 720K disks, a
1.2M drive's 1.2M and 360K disks.

Media have independent write policies and journal paths. Every base is hashed
on the actual mounted bytes and all geometries and path aliases are checked
before any journal opens. `extends` replaces a media list atomically and retains
its defining layer's path origins. A child single `image` replaces an inherited
catalogue, and a child catalogue replaces an inherited single image and policy.
See persistent-media.md § Media changes and machines/README.md § Removable
floppy media for runtime host controls and deterministic headless input.
