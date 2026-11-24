# Machine descriptions

`generic-386-seabios.toml` and `generic-486-seabios.toml` describe diskless
ISA boards with the pinned unemu SeaBIOS 1.17.0 fork, SeaVGABIOS,
register-level VGA and COM1. Both have 640 KiB conventional plus 15 MiB
extended RAM. The board supplies 128-byte CMOS, port 92h, fw_cfg with
`etc/sercon-port`, and the 402h debug console. Build the pinned firmware
with `vendor/seabios/fetch.sh` in the firmware devShell. The diskless
profiles reach `No bootable device.`. Both select `cpu.clock_mhz = 33`: a
386DX-33 class (8.25 million steps/s) or 486DX-33 class (16.5 million steps/s).
These are approximate CPI-based throughput labels; the CPU models stay
`80386dx`, `80486dx` and `80486dx-cpuid`. Set `cpu.turbo = { clock_mhz = 8 }`
for an alternate clock and select it with `unemupc run --machine FILE --turbo`.
The button can select either a slower or faster alternate clock. `cpu.rate`
remains the exact override and conflicts with `clock_mhz` in the same layer.
The IBM AT default remains its calibrated 8/13 rate. See
[clock rules and presets](../backlog/docs/design/machine-files.md#nominal-cpu-clocks).

`generic-386-freedos.toml` and `generic-486-freedos.toml` extend those
profiles with the pinned FreeDOS 1.44 MB floppy in A:. Their `-ide.toml`
counterparts extend the floppy profiles with the readonly `vendor/hdd/fat16.img`
primary master (615 cylinders, 4 heads, 17 sectors per track). SeaBIOS supplies
both floppy and ATA INT 13h services directly; no XTIDE option ROM is needed.
Fetch media with `vendor/freedos/fetch.sh` and `vendor/hdd/build.sh`.
All three CPU choices (80386DX, early 486DX, CPUID 486DX) reach `A:\>` and
list the UNEMUHDD volume's 29-byte MARKER.TXT with `DIR C:`. The shipped 486
files select `80486dx`; overriding `cpu.model` to `80486dx-cpuid` selects
the other variant. These profiles boot from A:, while C: is a data disk;
booting directly from C: needs a separate bootable fixture and qualification.
See [the board description](../backlog/docs/design/machine-files.md#generic-386-isa-board).

`at-freedos.toml` and its equivalent `at-freedos.yaml` describe the current
IBM PC/AT-class FreeDOS machine:
Xi8088 open BIOS, MDA and VGA text cards, an unwired COM1, and a 1.44 MB
floppy on the µPD765 controller. Its pinned images are fetched with
`vendor/kiselev-bios/fetch.sh` and `vendor/freedos/fetch.sh`.

`at-ide.toml` extends `at-freedos.toml`, selecting the open AT BIOS
(`vendor/unemu-at-bios/bios-unemu-at.bin`, the unemu fork of the Kiselev BIOS;
[fixture notes](../vendor/unemu-at-bios/README.md)) and the pinned XTIDE
Universal BIOS r638 XT+ option ROM configured for 16-bit ATA polling at C8000h. A primary IDE channel uses
1F0h/3F6h and IRQ14.
Its master is `vendor/hdd/fat16.img`, geometry 615/4/17, containing the
UNEMUHDD FAT16 volume and 29-byte `MARKER.TXT`. The disk is readonly so
the directory-listing test cannot depend on accepted writes.
The open AT BIOS accepts XUB's AH=00h, DL=80h reset request using IBM AT
semantics; the original BIOS and floppy profile keep their existing bytes. It
reads the display switch with the IBM polarity, so `at-ide.toml` sets
`display_switch = "mono"` to keep the inherited MDA as the console (the
Kiselev profiles set `colour`, which that BIOS reads the other way).
The ignored `ide_freedos_golden` boots A:, types `DIR C:` through the keyboard
and checks the completed listing, prompt instruction count, screen hashes
and POST sequence. Run it with:

```sh
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --offline -p pc-harness --test ide_freedos_golden -- --ignored --nocapture
```

Fetch the Kiselev BIOS images with `vendor/kiselev-bios/fetch.sh`, the open AT
BIOS with `vendor/unemu-at-bios/fetch.sh`, the option ROMs with
`vendor/xtide-universal-bios/fetch.sh`, and build the HDD with
`vendor/hdd/build.sh`. See
[`firmware-and-machine-classes.md` §9 item 11](../backlog/docs/design/firmware-and-machine-classes.md).

Every path, including `journal`, resolves relative to the layer that declares
it. For example,
`../vendor/freedos/freedos.img` starts in this directory, independently of the
process working directory. The loader canonicalizes the machine file path
for layer resolution: a symlink uses its target file's directory for
images, journals and relative parent files.
Diagnostics keep the path supplied by the caller. The supplied name also
selects the format, even when the symlink target has another extension.
Image errors show both spelled and resolved paths when they differ.
The loader reads each image once and checks its SHA-256 pin. Single-image
volatile/readonly bytes stay inline; catalogues and journal bases retain
a file path for assembly's alias checks and are read again at mounting; keep the
base unchanged between loading and assembly.

Complete TOML (`.toml`) and YAML (`.yaml` or `.yml`) files load through
`machine_file::load`. YAML requires one document, unique keys and strict
booleans (`true`/`false`); anchors and aliases have bounded parser budgets.
RTC start times must be quoted. Quoting CPU model names, hashes and drive
types is optional.

`at-freedos-from-preset.toml` lowers to the same configuration by extending
`preset:ibm-5170`. The preset in `presets/ibm-5170.toml` is embedded in the
binary and contains board, CPU, memory, firmware services and default cards,
with no image or journal paths. The child supplies its pinned BIOS and floppy.

`extends` also accepts a relative machine-file path, such as
`extends = "at-freedos.toml"`. Each file independently chooses TOML or YAML.
Tables merge field by field, cards by `id` (defaulting to `kind`), and drives
by `unit` or `channel`. Parent order is retained and new cards append. Use
`remove = true` with an inherited key to delete a card or drive. Cycles and
chains longer than eight layers fail. A final completeness check reports
missing fields and their defining files before images are read. The caller's
argument is always a filesystem path; `preset:` is interpreted only in an
`extends` field. Cycle and completeness chains retain the entry spelling and each written
parent name. Per-value origins retain the layer I/O path, including joined
relative components.

Setting an image path also replaces its hash pin: omit `sha256` to remove an
inherited pin, or supply a new pin. A `sha256`-only override re-pins an inherited
image. Mismatches identify the file and line that set the pin. Each machine
file is limited to 1 MiB, including parent files.

## Selecting a machine

The schema and
defaults are in
[`machine-files.md`](../backlog/docs/design/machine-files.md).

Validate and run a machine from the repository root with the core's standalone
program:

```sh
cargo run --offline -p unemupc --bin unemupc -- machine check machines/at-freedos.toml
cargo run --offline -p unemupc --bin unemupc -- run --machine machines/at-freedos.toml --max-steps 100000000 --print mda
```

The installed binary accepts the same command words. Run `unemupc` without
arguments to print its usage. `run` accepts `--max-steps`, `--idle-limit`,
`--diag-at`, `--trace-exceptions` and `--print CARD`; output selection and run
limits stay outside the hardware description. `--print` selects a configured
card id, such as `mda` or `vga`.

For user-supplied firmware, pass repeatable ROM directories explicitly:

```sh
unemupc machine check machines/at-games.toml --rom-dir /path/to/roms
unemupc run --machine machines/at-games.toml --rom-dir /path/to/roms --print vga
```

Without `--rom-dir`, the standalone program performs no ROM discovery. Firmware
is selected by full-file SHA-256 pins, including split chips and assembled ROMs.
Unknown development firmware requires `--allow-unlisted-firmware` on either
command. See [the ROM guide](../vendor/user-roms/README.md).

The desktop, library, firmware preferences and image management live in the
[unemu front end](https://github.com/unemu-org/unemu). Follow its documentation
for interactive runs, settings and host input.

For different images, save a local file beside
`at-freedos.toml`, for example:

```toml
schema = 1
extends = "at-freedos.toml"

[firmware.bios]
image = "images/custom-bios.bin" # replaces the inherited BIOS and its pin
# sha256 = "..."               # optionally pin the replacement

[[card]]
id = "fdc-765"                 # merge with the inherited controller
[[card.drives]]
unit = 0
image = "images/work.img"      # replaces drive A and removes its inherited pin
```

Run `unemupc machine check machines/local.toml` before booting with
`unemupc run --machine machines/local.toml`. Unknown development firmware
requires `--allow-unlisted-firmware` on either command. For a headless run, use
`unemupc run --machine machines/local.toml --print mda`. Run limits and host
output selection stay outside the hardware description.

## Preparing disks

Keep generated disks, local machine files and journals outside the repository
or in the gitignored `machines/local/` directory used by the game profiles.
A mounted image is an immutable base. Guest writes belong in an overlay;
`media commit` exports a new base and never changes its input. SHA-256 binding
covers the entire base, including large disks. Never edit a mounted base with
host tools or replace a base that still has a journal bound to it.

### Creating images and a local profile

Disk creation, formatting and image import belong to the
[unemu front end](https://github.com/unemu-org/unemu). Create new raw disks
there, then bind their paths and geometry in a local machine file. A blank
fixed disk still needs partitioning and formatting in the guest.

For the IBM AT BIOS, use a supported fixed-disk type, such as 615 cylinders,
4 heads and 17 sectors (type 2). With the open AT BIOS and XTIDE from
`at-ide.toml`, use the geometry reported by the disk-creation tool. The image
length must match the geometry exactly.

Save local overrides beside the images, with a relative parent path:

```toml
schema = 1
extends = "../at-ide.toml"

[[card]]
id = "ide"
[[card.drives]]
channel = "master"
image = "disk.img"
geometry = { cylinders = 615, heads = 4, sectors = 17 }
write = "journal"
journal = "disk.upfj"
```

Check the file with `unemupc machine check FILE`, then use
`unemupc run --machine FILE --print mda`. See the
[machine-file schema](../backlog/docs/design/machine-files.md) for drive fields.

### Write modes and journals

The IDE journal lives next to the immutable disk base. Guest writes survive
restarting the emulator, and mounting the same file replays the journal before
execution. Journal directories must already exist. The same write modes and
offline commands apply to floppies and IDE disks; ATA persists full 512-byte
sectors, and offline capacity comes from the complete image length.

Drives accept `write = "volatile"`, `"readonly"` or `"journal"`; NVRAM supports
volatile and journal modes. `keep_generations` is positive and defaults to 10.
`journal` and `keep_generations` are errors with volatile/readonly modes. Journal
paths must be nonempty, must not name a directory, and must not alias an image
or machine-file layer; diagnostics identify the drive's file and line.

Overriding a drive's `image` discards its inherited journal and retention count.
An inherited journal write mode falls back to volatile unless restated;
readonly and volatile modes retain their values. Restating journal mode needs
a new journal path. Setting volatile or readonly without repeating the image
also discards inherited journal options; explicit journal options remain errors.

Completed IDE commands are journaled at completion. For an unfinished command,
completed sectors reach the journal only at a checkpoint, session finish, CPU
reset or run error; partial sectors are excluded. Changing a floppy closes only
that floppy's journal.

### Adding a data floppy

For drive B:, the same journal policy applies. For a local data floppy, use this
complete card override and keep the journal beside its base:

```toml
[[card]]
id = "fdc-765"
[[card.drives]]
unit = 1
type = "1.44M"
image = "transfer.img"
write = "journal"
journal = "transfer.upfj"
keep_generations = 10
```

### Removable floppy media

A floppy keeps its existing single `image`, `sha256`, `write`, `journal` and
`keep_generations` form, or uses an ordered `media` list with those fields on
each entry. For multiple installation/data disks, replace the single-image B:
override above with this card override in `local.toml`. Supply `install-1.img`
beside that file; `transfer.img` is the formatted image created earlier:

```toml
[[card]]
id = "fdc-765"
[[card.drives]]
unit = 1
type = "1.44M"
media = [
  { image = "install-1.img", write = "readonly" },
  { image = "transfer.img", write = "journal", journal = "transfer.upfj" },
]
# empty = true
```

The first entry is inserted at power-on unless `empty = true`, which also permits
an empty catalogue. A configured drive with `empty = true` starts with its
disk-change line asserted; unconfigured positions and drives starting with a
medium keep the line clear. YAML uses the same fields and a sequence for `media`.
List and drive-level single-image fields cannot be combined. Every medium must be
one the drive type takes, by its image size, and entries may differ:

| Drive `type` | Images it takes |
|---|---|
| `1.44M` (3.5-inch) | 1.44M (1,474,560 bytes), 720K (737,280 bytes) |
| `1.2M` (5.25-inch) | 1.2M (1,228,800 bytes), 360K (368,640 bytes; read at 300 kbps, double-stepped) |
| `720K` (3.5-inch) | 720K |
| `360K` (5.25-inch) | 360K |

Any other image fails with a message naming the drive type and the images it
takes. The CMOS drive types follow `type`, not the inserted disk.
Paths belong to the layer that declares them, and `extends` replaces a media
list as a whole; a replacement list clears inherited `empty = true` unless the
child restates it. The inherited A: boot floppy stays unchanged in this example.

Interactive insertion, ejection and media catalogues are provided by the
[unemu front end](https://github.com/unemu-org/unemu). The standalone program
starts with the machine file's initial media; embedders can schedule changes
through the assembly API described below.

Ejected volatile media retain their in-memory writes for the session. Ejecting
journaled media flushes and seals that medium's generation and releases its
lock; insertion reopens and replays it. The base is never rewritten, and a
persistence failure stops execution. The journal API always closes a generation,
even after an insert with no writes; it has no clean no-op close. Dropping it
would use the recovery path instead. Each eject therefore uses a retention slot,
so repeated swaps can prune older rollback states after ten closes by default.
Export important states first or increase each medium's `keep_generations`.
Host tools still require stopping the
emulator and writing a new image, as described below.

Headless embedders and tests use `AssembledMachine::run_floppy_schedule` with
ordered `FloppyEvent { at, controller, unit, action }` inputs. `at` is cumulative
virtual time in instruction equivalents, including halted PIT progress. Halted
caps follow the scheduler's whole-PIT-clock rounding. `change_floppy` applies
one event at an already reached boundary; no host timer enters the device path.

### Exporting and rolling back

After installing or copying software, exit DOS cleanly and stop the emulator.
Inspect the HDD's closed generation numbers, export the installed state as a
new base, or roll back with a fresh backup path:

```sh
cd "$HOME/unemupc-disks"
unemupc media generations --base disk.img --journal disk.upfj
unemupc media verify --base disk.img --journal disk.upfj
# Use a retained closed generation reported above (1 is an example).
unemupc media commit --base disk.img --journal disk.upfj --to 1 --output disk-next.img
unemupc media rollback --base disk.img --journal disk.upfj --to 1 --backup disk-before-rollback.upfj
```

Choose an available closed generation from `generations`. Rollback backs up the
journal and discards later history; commit exports a new immutable base without
changing either input. Keep the original base with its original journal. To
adopt `disk-next.img`, override the drive's `image`, optionally set its new
SHA-256 pin, and explicitly select `write = "journal"` with a **new** journal
path. Journal tools preserve
sector state, not filesystem health. Use the same commands with `transfer.img`
and `transfer.upfj` for the floppy; for example, export it to `transfer-next.img`
before using the host-copy example below.

### Building a floppy from host files

Use the [unemu front end](https://github.com/unemu-org/unemu) to build new
FAT12 data floppies from host files or split a directory tree across disks.
Bind the resulting raw images in a local machine file or media catalogue.

### Create from files… in the window

The disk-creation dialog and library retention policies are documented with the
[unemu front end](https://github.com/unemu-org/unemu).

### Copying host files

To put host files into existing FAT media, mtools is optional and runs while the
emulator is stopped. Always create a new working image first; never modify an
immutable base with a journal bound to it:

```sh
cp transfer-next.img transfer-host.img
mcopy -i transfer-host.img file ::
mdir -i transfer-host.img ::
# For an already partitioned disk, use its partition-start LBA times 512.
# Example only: a partition starting at LBA 63 has byte offset 32256.
cp partitioned.img partitioned-host.img
mcopy -i partitioned-host.img@@32256 file ::
```

Do not guess the partition offset or use `@@32256` on an unpartitioned blank
image. Obtain the start LBA from its partition table; the fixture in
`vendor/hdd/README.md` instead starts at LBA 17 (`@@8704`). Adopt the host-edited
copy with a fresh journal path and, optionally, a new SHA-256 pin. Host mtools
operations may use host file timestamps.

## Lo-tech EMS card reference

```toml
[[card]]
kind = "lotech-ems"
port = 0x260
frame = 0xD000
size_kib = 2048
```

`frame` is required and must match the driver's `/p` switch. It is a segment
(for example 0xD000), not a physical address. The card's jumper default is
E000h, but E0000h–FFFFFh is system-board ROM space on `ibm-5170`: use C000h or
D000h there. IBM 5170 is the only implemented machine-file board; only Rust
bare test rigs accept E000h. `port` defaults to 260h and accepts 260h, 264h,
268h or 26Ch; `size_kib` defaults to 2048 and accepts 512, 1024, 1536 or 2048
(one to four 512 KiB SRAM chips). The 64 KiB frame must not overlap RAM,
another MMIO window or a loaded ROM. All four byte ports and every 400h alias
participate in conflict checking. The card has no IRQ or DMA setting.

The r02 schematic's write-only page latches return FFh on reads. Values
80h–FFh or unpopulated chips leave their 16 KiB windows unbacked. Deterministic
power-on uses FFh latches and zero SRAM; a warm reset preserves both latches
and SRAM. Code can run from the frame: a window whose latch selects a
populated page that no other latch selects is held as CPU RAM, so overlay
managers and EMS-aware programs execute from mapped pages. Unbacked windows
read and execute FFh and drop writes, as on the floating bus; a word that
straddles a mapped and an unbacked window reads FFh for the unbacked byte.
When two latches select the same page, both windows keep the board's coherent
data path but cannot be executed from. One known limit remains: a word operand
that crosses from such a shared window into a neighbouring window that is
mapped or unbacked (for example `MOV AX,[3FFFh]` in the frame with window 0
mapped and windows 1 and 2 on one page), or from another card's memory-mapped
window directly below or above the frame into a mapped or unbacked window,
stops the run with a CPU error.
Hardware coverage is recorded in `crates/devices/src/lotech_ems.rs`.

## Playing DOS games

`at-games.toml` combines the IBM 5170 BIOS dated 1985-06-10, register-level
VGA with the IBM Display Adapter video BIOS, SB16/OPL3, the Lo-tech 2 MiB EMS
board, a Microsoft serial mouse on COM1/IRQ4, and a journaled IDE C:. It
extends `preset:ibm-5170` and removes both default text cards. It is a
configuration for compatibility experiments, not a claim that individual games
or mouse drivers have passed.

Supply the two system BIOS chips and the complete 32 KiB VGA EPROM under the
names in the file; [the ROM guide](../vendor/user-roms/README.md) gives their
identities and layout. The pins cover both chips, the assembled system BIOS,
and the complete video container; its 2000h prefix is skipped when mapping
C0000h. Nothing downloads these ROMs. Unlike test discovery, machine-file paths
use these exact filenames; a local child can override them.

`at-games-open.toml` needs no user ROMs. Fetch/build the open AT BIOS, open VGA
BIOS and XTIDE fixtures with `vendor/unemu-at-bios/fetch.sh`,
`vendor/vgabios/fetch.sh` and `vendor/xtide-universal-bios/fetch.sh`. The open
AT BIOS ([fixture notes](../vendor/unemu-at-bios/README.md)) is the unemu fork
of the Kiselev BIOS with the IBM PC/AT's shutdown-byte resume, INT 15h AH=87h
and AH=88h, measured extended memory, CMOS checks and display-switch
polarity; the colour switch selects the VGA card, as with the IBM BIOS. It
removes both preset text cards and adds the register-level `vga` card with
the open VGA BIOS, the unemu fork of the LGPL VGABIOS built for the 286
([fixture notes](../vendor/vgabios/README.md)), at C0000h–C7FFFh; XTIDE
follows at C8000h. POST, FreeDOS and the fork's INT 10h probe floppy are
measured on this profile, and a mode 0Dh golden pins panned and page-flipped
frames. As with the IBM variant, no individual game is qualified. The IBM
variant needs user ROMs for both system and video firmware.

Both shipped game profiles enable the 2 MiB board at D000h, port 260h. For
another machine, add the `lotech-ems` card above to a local child file. Fetch
the local-only driver using `vendor/ltemm/fetch.sh` (see [its terms and
pins](../vendor/ltemm/README.md)); copy LTEMM.EXE to your own working DOS
disk. Add this line to CONFIG.SYS:

```text
DEVICE=LTEMM.EXE /p:D000 /n
```

Use `DEVICE=C:\LTEMM.EXE /p:D000 /n` if it is installed in C:\ instead. `/n`
skips only the keyboard-polled memory test; the RAM probe still writes port
61h. Match `/p` to the configured frame, and pass `/i:264` when using port
264h. Reboot and run `MEM /E` from the open FreeDOS tools disk to check the
total and free EMS pages. Never put the driver into a committed or published
disk image. Extended RAM and XMS are separate from this board's expanded RAM.

Both profiles use 640 KiB conventional plus **1024 KiB extended RAM**, derived
CMOS, a fixed RTC start of 2000-01-01, the preset's instruction/PIT ratio, and
an empty 80287 socket. The 1 MiB extended-memory default shortens the POST memory
test. To raise it, set `memory.extended_kib` in a local child profile; allowed
values are 0–15232 KiB in steps of 64 KiB.
The I286 model supports LMSW entry, 16-bit protected execution and task switches;
the pinned IBM POST tests all 640 KiB + 1 MiB, and the open AT BIOS measures
the extended memory at POST. Both BIOSes pass the open protected-mode test
floppy's INT 15h block move and reset checks
([at-pm-test](../backlog/docs/design/at-pm-test.md)). No memory manager is
supplied; XMS and DOS-extender use are not yet qualified. This CPU is an
80286, with no 386 instruction set.

A: is **1.2M**, with the minimal FreeDOS boot fixture, the `FDTOOLS`
installation floppy, then a local transfer floppy; it also takes 360K disks.
In `at-games-open.toml`, B: is an empty **3.5-inch 1.44M** drive: insert 1.44M
or 720K game disks through the unemu front end, or list
them under unit 1 in a local child profile. `at-games.toml` keeps A: only: the
June 1985 BIOS has no 1.44M drive type, and whether it has a 720K one is not
verified from the IBM listing this project has (its diskette module is not
included), so no B: is configured there.

Create a blank type-2 hard disk (615/4/17) and a formatted 1.2M transfer
floppy with the [unemu front end](https://github.com/unemu-org/unemu), at the
local paths declared by the selected profile. Then fetch the FreeDOS fixtures
and check or run the profile with the standalone program:

```sh
vendor/freedos/fetch.sh
unemupc machine check machines/at-games.toml
unemupc run --machine machines/at-games.toml --print vga
# Open firmware, no user ROMs (also run vendor/unemu-at-bios/fetch.sh,
# vendor/vgabios/fetch.sh and vendor/xtide-universal-bios/fetch.sh):
# unemupc machine check machines/at-games-open.toml
# unemupc run --machine machines/at-games-open.toml --print vga
```

The disk is blank, 21,411,840 bytes (about 20.42 MiB), IBM fixed-disk type 2.
A 32 MiB disk rounded to 16 heads and 63 sectors has 66/16/63 geometry;
that needs the XTIDE path and a matching geometry override, as described in
[Preparing disks](#preparing-disks). The IBM profile deliberately uses the exact
615/4/17 mapping its system BIOS supports, with `controller_model = "at-adapter"`.
The open profile uses ATA task-file semantics for XTIDE. Both profiles share
`games.img` and `games.upfj`; run only one at a time.

### macOS system shortcuts in games

Host keyboard capture and macOS shortcut handling belong to the
[unemu front end](https://github.com/unemu-org/unemu).

### First DOS install

Use the [unemu front end](https://github.com/unemu-org/unemu) for interactive
DOS installation, disk swaps and game input. The standalone program supports
bounded headless runs and printed display output.

The supplied tools floppy contains FDISK, FORMAT, SYS, XCOPY, EDIT, MEM and
CTMOUSE. See [versions, licences and CPU requirements](../vendor/freedos/README.md).
The game profiles use a type-2 disk; the IBM profile's fixed-disk installation
and C: boot remain unqualified by the open-firmware installation golden.

Stop the emulator before using `unemupc media generations`, `unemupc media verify`,
`unemupc media commit` or `unemupc media rollback`; follow
[Exporting and rolling back](#exporting-and-rolling-back) with base
`machines/local/games.img` and journal `machines/local/games.upfj`. Commit
exports a new base, preserving the original; adopting it requires a fresh
journal path in a local child. Rollback uses a retained generation and a new
backup path. The full base is SHA-256-bound even without an explicit disk pin.

Compatibility targets are experiments, not measured passes. Suggested 286
starting points are Prince of Persia, Commander Keen and Wolfenstein 3D;
version-specific graphics, CPU, memory and driver requirements still apply.
The maintainer's named **Tyrian 2000** and **Duke Nukem 3D** targets are
**not yet**: both require a 386 or later and cannot run on this 80286 profile.
No memory manager is supplied; XMS, INT 15h block move and DOS-extender use
remain unqualified. A20 and installed extended RAM do not supply a 386.

The synthetic profile smoke covers configuration and `machine check` without
proprietary bytes. It checks C0000h in the original document, then relocates
only the synthetic video ROM to C8000h because an unknown system BIOS cannot
establish the catalogue's video scan. The ignored
`user_rom_at_games_freedos_prompt` exercises the unmodified IBM profile's
C0000h layout with discovered user ROMs, the 1.2M fixture and a temporary blank
HDD. Its stop/counter/hash and POST pins record a measured FreeDOS boot with
both user-supplied IBM ROMs.

The fixture-gated open-profile check uses the real pinned open AT BIOS, open
VGA BIOS, XTIDE and FreeDOS 1.2M images with temporary local media created by
`media create`. The ignored `open_game_profile_boots_freedos` checks the open
AT BIOS sign-on and its 80286 line on the POST screen and pins its FreeDOS A:
prompt's retired count and VGA text hash. Missing open fixtures skip unless
`UNEMUPC_REQUIRE_FIXTURES=1`; this open-firmware boot does not qualify the
IBM fixed-disk BIOS's C: operations.
