# Local user-media qualification

DOOM is commercial software. Supply your own legal copy; this repository ships
no game files. No game file or disk-image digest, frame digest, screenshot,
console transcript or private path belongs in the repository. The temporary
base is identified in memory by SHA-256 for the existing sector-overlay
interface; that value is never logged or stored here.

## Prepare the directory

Copy an installed registered DOOM 1.2 directory, or extract the installer's LHA
archive using `lhasa` from nixpkgs (`nix shell nixpkgs#lhasa`, then `lha x` on
the archive). Extraction of multi-disk installers may require joining their
archive segments first. Keep the result outside the checkout. The directory
must directly contain `DOOM.EXE`, `DOOM.WAD` and any other installed files.
Names are case insensitive; use ordinary DOS 8.3 filenames and no symbolic
links. Subdirectories are supported. Set `UNEMUPC_USER_DOOM_DIR` to that directory.

Inside the project devShell:

```sh
cargo test --offline -p machine-file user_media_doom_386 -- --ignored --nocapture
# Optional: both 486 CPU profiles use the same helper.
cargo test --offline -p machine-file user_media_doom_486 -- --ignored --nocapture
```

The tests are ignored by default. With the variable absent they explicitly
skip, including when the golden lane includes ignored tests. `user_media_doom_`
is the opt-in name filter; the existing `user_rom_` firmware filter does not
select commercial game media. CI must not supply this variable.

The existing open SeaBIOS, SeaVGABIOS and FreeDOS fixtures are required. The
helper uses the shipped generic IDE profile, boots FreeDOS from A:, types `C:`
and `DOOM` through the emulated keyboard, and gives C: a volatile sector overlay.
No configuration writes reach the immutable base or the supplied directory.

## Sound variant

`user_media_doom_sound_386` uses `machines/generic-386-games.toml` with SB16
at 220h, IRQ5 and DMA1/DMA5. It is separately ignored and requires the same
`UNEMUPC_USER_DOOM_DIR` opt-in:

```sh
cargo test --offline -p machine-file --test generic386 user_media_doom_sound_386 -- --ignored --nocapture
```

Before hashing or mounting the generated data disk, the test replaces its
`DEFAULT.CFG` with these test-owned DOOM 1.2 settings:

```text
snd_sfxdevice 3
snd_musicdevice 3
snd_sbport 544
snd_sbirq 5
snd_sbdma 1
snd_mport -1
snd_channels 3
```

These select Sound Blaster effects and OPL music; 544 is decimal 220h.
The source directory is only read. The disk base receives its full SHA-256
after the generated configuration is copied; subsequent guest writes stay
in the volatile sector overlay. The probe requires the same non-trivial
mode 13h picture plus DSP command writes, OPL register-data writes and
channel-1 address/count advancement during a poll with no DMA-controller
register writes. It observes the existing bounded port traces and restores
device snapshots into a separate 8237 for side-effect-free inspection.
Trace overflow is a failure. No audio or game-derived hash is recorded.

**MEASURED**, DOOM 1.2, x86jit `6a38ae49`: all selected startup identifiers
and the DOS/4G banner appear. The qualifying mode 13h picture has 243 palette
indices at 160,240,957 retired instructions (43.7 s in this probe). At that
point the SB16 has seen 15 DSP command writes and 528 OPL register-data writes,
and the DMA advancement check has passed. This qualifies sound initialization
and DMA activity at startup; it does not claim attract-mode endurance or pin
the game's audio content. The sound trace rings hold 65,536 events per device
and are drained every 100,000 scheduling units; other diagnostics keep their
small rings. A synthetic-media test checks generated configuration replacement,
deterministic disk bytes and preservation of the supplied directory's defaults.

## Temporary disk

`scripts/user-media-disk.sh` with no arguments reads `UNEMUPC_USER_DOOM_DIR` and prints the
path to a newly built image under the system temporary directory. It requires
Python 3 and mtools from the devShell. The caller owns that temporary directory;
the Rust probe removes it on success and unwind.

The disk starts at 34 MiB and grows with the input, with four heads, 32 sectors
per track and 512-byte sectors. One active type-06h FAT16 partition begins at LBA 32;
the builder verifies its data-cluster count. The minimum size allows 8 KiB
clusters, limiting DOS FAT-chain traversal during WAD seeks. Filenames are sorted and uppercased,
FAT timestamps use the FAT epoch in UTC, and the volume serial is fixed. The
builder never computes a digest. Synthetic tests check byte-for-byte
reproducibility despite changed host mtimes, MBR/BPB layout and copied contents.

The general form is `bash scripts/user-media-disk.sh --source SOURCE --target
'\\WIN95'`, with optional `--cylinders`, `--heads`, `--sectors`, `--size-mib`, and
repeatable `--extra FILE DOSPATH`. The default geometry is 1015/16/63 with
512-byte sectors (523,837,440 bytes, about 500 MiB). The partition starts at the
second track, LBA 63. `--size-mib` rounds up to a cylinder within the requested
CHS capacity; geometry without a size fills that capacity. General images stay
within 1024 cylinders, 16 heads and 63 sectors per track. The DOOM preset keeps
its existing smaller geometry and growth policy.

The Python helper validates the entire directory tree before creating output:
only regular files/directories with 8.3 names, no symlinks, no case-insensitive
collisions within a directory or with extra files. Apostrophes and DOS device
basenames (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, including
extensions) are rejected for source entries, target components and extra-file
destinations. This avoids mtools' renaming/LFN fallback (GNU mtools manual,
[VFAT-style long file names](https://www.gnu.org/software/mtools/manual/mtools.html#VFAT_002dstyle-long-file-names)).
Both mtools name-clash actions are set to skip, and unused stdin is closed, so
an unexpected backend clash cannot wait for an invisible interactive answer.
Target directories and extra
file parents are created on the image. Generated configuration enters mcopy on
stdin. No temporary host copy of MSBATCH.INF is made. The builder selects the
cluster size using the Microsoft FAT Specification's data-cluster rule and
checks the actual BPB afterwards: 4085 <= CountOfClusters < 65525. Both FATs,
the reserved sector and the fixed root directory are excluded from that count.
The partition is active but has no installed operating system or MBR bootstrap;
DOS still boots from A:. Later setup may install those boot records through the
journal. Synthetic CI tests independently parse the MBR/BPB, both FAT copies,
directory chains and a multi-cluster file, using only generated open data.
Every live directory entry is inspected for unwanted attribute-0Fh LFN records.
Subdirectories must have correct `.` and `..` links, including cluster zero for
a parent in the root. Nested trees with generated MSBATCH.INF and extra files
must reproduce identical images after host mtimes change.

The bounded probe reports retired instructions and host elapsed time for the
FreeDOS prompt, command injection, short startup identifiers, any DOS/4G banner,
graphics transition and a 320×200 picture with at least 32 distinct palette
indices. It checks BIOS mode 13h and requires all selected startup identifiers.
It does not save pictures or hash screens. The instruction allowance is fixed;
wall time is diagnostic only. A stop reports CPU state, deliveries and the last
observed port I/O. A budget failure reports throughput rather than starting an
unbounded run.

## Findings

Qualification results are recorded after running the local probe. A CPU stop
must be handed to the x86jit maintainers with CS:IP, linear address, the faulting
instruction's own bytes only, mode, CPL, retired count, delivery state and last
port I/O. Board/device failures require isolation tests before a fix. Neither
case permits publishing game bytes, media hashes or frame hashes.

MEASURED on the pinned CPU revision `fc07c81f`, registered DOOM 1.2 on
`generic-386-isa` boots from the open FreeDOS floppy and reaches the game picture.
No board fix or CPU boundary was needed. The 34 MiB temporary FAT16 disk uses
8 KiB clusters; an earlier 2 KiB-cluster probe spent its 250-million-instruction
launch allowance in DOS/BIOS disk service before graphics. Increasing cluster
size avoids unnecessary FAT-chain traversal without changing the supplied files.

| 386 milestone | Retired instructions | Host elapsed |
| --- | ---: | ---: |
| FreeDOS prompt | 10,716,384 | 3.44 s |
| DOOM command injected | 10,815,206 | 3.46 s |
| DOS/4G banner | 14,080,125 | 4.51 s |
| DOOM / V_Init | 23,045,406 | 7.53 s |
| M_LoadDefaults | 23,145,189 | 7.55 s |
| Z_Init / W_Init | 23,244,159 | 7.58 s |
| R_Init | 24,806,794 | 7.94 s |
| 320×200 graphics capture | 153,366,005 | 36.07 s |
| Mode 13h, 243 distinct indices | 159,571,631 | 37.67 s |

The first 320×200 capture precedes the BIOS data-area mode update; qualification
waits for the non-trivial picture and checks mode 13h there. Sampling is every
100,000 machine scheduling units, so these counts are observed checkpoints,
not instruction-exact transition locations. Host elapsed times are diagnostic
and vary by host load. The limit is 250 million retired instructions after
command injection, plus a bounded boot allowance. No attract-mode endurance
claim is made by this startup probe.

Both 486 profiles also pass the same probe (run together):

| CPU profile | DOS/4G banner | R_Init | 320×200 graphics | Qualifying picture | Host elapsed at picture |
| --- | ---: | ---: | ---: | ---: | ---: |
| 80486DX | 19,180,424 | 35,107,182 | 219,462,780 | 226,874,445 | 98.78 s |
| 80486DX with CPUID | 19,180,424 | 35,107,210 | 219,462,808 | 226,874,473 | 98.67 s |

Counts are retired instructions. Both pictures have 243 distinct indices and
BIOS mode 13h. Different virtual clock rates change timer/disk-wait work; host
contention makes these concurrent wall times unsuitable for comparing CPU speed.

## Windows 95

`user_media_win95_stage1` is an ignored, local test. It requires both
`UNEMUPC_USER_WIN95_DIR` (the extracted WIN95 setup directory) and
`UNEMUPC_USER_DOS_FLOPPY` (a private 1.44 MB DOS boot floppy). Missing variables
cause an explicit skip. The optional `UNEMUPC_USER_WIN95_KEY` is never required
to launch setup. Run inside the devShell:

```sh
cargo test --offline -p machine-file --test user_media_win95 user_media_win95_stage1 -- --ignored --nocapture
```

Keep all inputs outside the checkout. The setup directory is read-only. The
floppy mounts read-only. `--win95` builds a 1015/16/63 disk, copies the source to
C:\WIN95, and generates C:\MSBATCH.INF. Both bases are fully SHA-256 identified
at runtime; their hashes are never printed. IDE writes use the existing sector
overlay interface with a journal beside the generated base. Neither the base
nor source media is rewritten. The random temporary directory is retained on
success and on a guest stop; its path is printed only locally. It holds private
media and journal data and must never be added to the repository. Delete it when
finished. Future publication of a new base must use a new file, never overwrite
the old one.

Stage 1 uses `generic-486-isa`, 80486DX with integrated FPU, 16 MiB RAM,
SeaBIOS plus SeaVGABIOS, primary IDE master, floppy A:, PS/2 mouse, and no sound.
It selects the no-CD-ROM option on the supplied OSR2 startup menu, then
waits for a decoded A: prompt and types
`C:\WIN95\SETUP /IS /IQ C:\MSBATCH.INF` through the existing emulated keyboard
injection seam. It ends at the first setup screen: a populated full-screen
text dialog with short setup identifiers, or BIOS mode 12h, 640x480 scanout,
at least three palette indices and 4096 pixels outside the dominant index.
No image, screen hash or commercial text is kept in the repository.
The fixed budgets are 200 million retired instructions before the command,
300 million after it, and a 600-million virtual-time ceiling for HLT waits.
Host elapsed time is diagnostic only. The machine runs in slices of 10,000
virtual units; traces are drained after every slice, and the screen is captured
and the budgets checked after every 100 slices (about one million instructions).

A clean stage or budget boundary finishes a journal generation. An execution
error checkpoints only completed sectors, preserving a diagnostic generation
without marking the session successfully finished. This is a disk checkpoint,
not a whole-machine snapshot: a later stage boots anew from the same immutable
base plus journal. Continuation to file-copy completion, setup's reboots and
first desktop are follow-up work; no later-stage runner is claimed here.

The stop report prints mode, CPL, CR0, CS:EIP, linear address, retired count,
a payload-free error category, per-device access counts, the last port access
(port, direction, size), command/register-write histograms (ATA, 8042
auxiliary, PIC ICW/OCW, PIT, DMA, FDC), PIC special-mask and poll requests, PIT
channel/mode counts, VGA indexed-register sets, and the delivery totals with
the kind, vector, saved address, error code and privilege/VM transition of the
latest deliveries. Trace rings are drained at every slice and overflow is an
error. Guest bytes stay out of the report: no instruction bytes, registers,
screen text or data-port payloads. Values written to command and control
registers are printed: ATA command, 8042 command and auxiliary-device bytes,
PIC and PIT control words, DMA registers, and for the floppy controller the
digital-output and data-rate registers plus the first byte of each command
(its parameter bytes are counted, not kept). ATA data-port values (1F0h and
170h) are also omitted from the retained port history. Synthetic tests compare all
port and delivery diagnostics for different payload bytes and registers. The
reports in this document contain only our observations, no media hashes,
product keys, commercial frame hashes or transcripts. CI must not set the
private-media variables.

### MSBATCH.INF evidence

**READ:** Microsoft Windows 95 Resource Kit, Appendix D,
[[Setup]](https://techshelps.github.io/MSDN/DNWIN95/HTML/S735E.HTM),
[[Network]](https://techshelps.github.io/MSDN/DNWIN95/HTML/S6869.HTM) and the
[sample script](https://techshelps.github.io/MSDN/DNWIN95/HTML/S7376.HTM).
The generator requests express installation to C:\WINDOWS, suppresses boot-disk
creation and the Pen Windows warning, selects full installation and GMT, sets
a generic name/organization, hides network pages, and specifies VGA, a VGA
monitor and PS/2 mouse. The sample spells the organization key `Org`.
**ASSUMED:** empty Clients/Protocols disable networking for this source version;
the English display/monitor/mouse descriptions match its INF database. These
choices have not been qualified through a completed installation.

**READ:** the Resource Kit documents `ProductID`, whereas the optional generator
emits `ProductKey` as requested for OSR2; acceptance of that spelling is
**ASSUMED** until a keyed run. Invalid key characters, including newlines, are
refused without logging their value. Synthetic tests check the key is present
only in the on-disk generated file, absent from builder stdout/stderr, omitted
when unset, and never staged as another host file. The key's acceptance is
separate from reaching the first screen.

**READ:** Resource Kit chapter 3,
[setup switches](https://bitsavers.trailing-edge.com/pdf/microsoft/resource_kits/Windows_95_Resource_Kit_1995.pdf),
distinguishes `/iq` for a DOS launch and `/is` for a Windows launch, both bypassing
a quick disk check. Microsoft KB Q129260,
[setup initialization](https://techshelps.github.io/MSDN/KB/Source/win95x/q129260.htm),
also describes `/is` as suppressing ScanDisk. The runner supplies both switches;
the narrower claim that `/iq` specifically skips cross-links is **ASSUMED**.

### Windows 95 recheck at the current pin

**MEASURED**, x86jit `f0785d5a`, shared runner with screen polling once per
100 slices: the unchanged stage-1 command and first-screen criterion pass.
With OSR2 source and the DOS 7.1 floppy, the no-CD option is chosen at
43,791,849 retired (12.6 s), the setup command is typed at 130,677,747
(42.5 s), and a populated 640x480 frame with three palette indices and BDA mode
12h is seen at 273,257,205 retired, 58–99 s after assembly across three runs
on the shared host. The coarser polling
moves each boundary by up to one million instructions. At `8f23fdcd`, with
per-slice polling, the same screen was seen at 271,888,056 retired after
120.96 s. The earlier refusal below is historical at `4bf91c8d`. This recheck
still qualifies neither full installation nor the product-key prompt.

### Windows 95 findings (2026-11-25)

**MEASURED**, Windows 95 OSR2 source and MS-DOS 7.1 boot floppy, CPU pin
`4bf91c8d`, generic-486-isa / 80486DX / integrated FPU / 16 MiB RAM:

| Boundary | Retired instructions | Host elapsed |
| --- | ---: | ---: |
| No-CD-ROM boot-menu selection | 43,751,849 | 12.78 s |
| DOS prompt and setup command injection | 130,127,767 | 40.55 s |
| First CPU refusal during protected setup initialization | 249,085,227 | 82.03 s |

The initial 50-million boot allowance was insufficient: the startup menu and
DOS preparation consume more than that before the shell is ready. The final
boot allowance is 200 million. The first full-screen setup dialog and key
prompt were **not reached**. Setup ran from C: and performed disk reads/writes,
then stopped during its protected initialization. No CPU workaround or device
change was attempted.

CPU hand-over (our observations, no commercial transcript):

- Refusal: `LegacyEntryFacility`.
- Mode: protected, unpaged, CPL 3, CR0=00000011h.
- CS:EIP=0367:00003D87h; linear=000342E7h.
- Refused instruction: `MOV ES,BX` (`mov %bx,%es` in AT&T syntax);
  architectural opcode bytes: `8E C3` (`MOV ES,BX`).
- BX=0317h, A20 enabled, EFLAGS low word=3202h. GDTR=00510800h/011Fh,
  IDTR=00510000h/07FFh, LDTR=0038h (base=00514C50h), TR=0088h
  (base=0002AE70h).
- Retired=249,085,227. No reset or budget exhaustion ended this run.
- Last I/O: read 92h -> 02h. Most recent delivery: software INT 2Ah;
  delivery log total=196,726, dropped=0. IRQ queued=false, ready=true.
- The local log contains the last 64 port accesses, full recent deliveries and
  decoded screen. They remain private. The screen was text mode during setup
  initialization, before a qualifying graphical picture.

| Device | Port accesses | Observed command/register summary |
| --- | ---: | --- |
| ATA | 1,392,607 | READ SECTORS 20h: 114; WRITE SECTORS 30h: 146; IDENTIFY ECh: 1; IDENTIFY PACKET A1h: 2 |
| 8042 | 17,898 | command-byte writes 60h: 12; A7h/AAh/ABh/ADh: once each; AEh: 84; no D4h auxiliary command |
| PIC | 1,444 | ICW1=11h once per controller; vector bases 08h/70h, cascade 04h/02h, ICW4=01h; master OCW3=0Bh: 12 |
| DMA | 7,924 | channel 2 programmed 792 times (mode 46h); no sound DMA requested |
| FDC | 30,360 | READ DATA command byte E6h seen 792 times; source floppy mounted read-only |

All evidence rings had zero dropped events. The IDE journal checkpoint was
written in the retained temporary stage directory, with the original base
unchanged. Later work must address the CPU refusal in x86jit under that
repository's workflow; this harness does not bypass it. A keyed run remains
unqualified: the location and acceptance of the key prompt are not observed.

The instruction bytes above were read by the stage-1 runner of that time. The
shared runner no longer prints guest bytes; a stop is handed over by its
address, mode and error category, and its bytes are read separately for the
CPU tracker.

## Windows 3.1

The local `user_media_win31_stage1`, `user_media_win31_stage2`,
`user_media_win31_standard_mode`, `user_media_win31_stage3` and
`user_media_win31_enhanced_refused_by_dos7` tests require
`UNEMUPC_USER_WIN31_DIR` (merged retail floppy files) and
`UNEMUPC_USER_DOS_FLOPPY`. They explicitly skip when either variable is
missing. Run each by name with `--ignored --nocapture` in the project
devShell. CI must leave the variables unset. Stage 1 records whether the floppy
is DOS 7 (its batch prints a marker in the DOS 7 branch). `stage3` then skips
under DOS 7 and `enhanced_refused_by_dos7` skips under any other DOS, so the
whole ignored set passes with either floppy and each one is qualified by the
test that applies to it.

The `--win31` builder reuses the FAT16 tree builder and 1015/16/63 geometry.
It stages the supplied files under C:\WIN31SRC and generates its own
SETUP.SHH, batch files, CONFIG.SYS and AUTOEXEC.BAT. Its small project-owned
MBR relocates to 0600h, preserves the BIOS boot drive, and loads the boot
sector named by the first partition-table entry, using its CHS start; it does
not search for the active entry. This preset has exactly one partition, and it
is marked active.
The supplied DOS floppy's SYS installs the proprietary boot sector and system
files only through guest writes to the journal. No proprietary boot code is
embedded in the builder.

The Windows 95 and Windows 3.1 tests share one runner, port diagnostics,
keyboard encoder and persistence boundary implementation. Each test creates
its own private disk; a later-stage test executes its prerequisites, closes
their journal generations, drops the machine, and mounts the same immutable
base plus journal in a fresh machine. Every mount hashes the full base and
floppy, including sparse extents, without printing either digest. Successful
boundaries finish a generation; CPU errors checkpoint completed sectors.
Disks and logs remain private and outside the checkout, including on failure.
At an installation stop, the runner reads SYSTEM.INI through the ATA drive's
merged overlay view and mtools on a new, disposable private inspection image.
It reports only derived configuration flags; the inspection image is deleted.

### Stage plan and evidence

1. Boot A: from the DOS floppy and reach its shell. On the MS-DOS 7.1 floppy,
   select the no-CD option. On an MS-DOS 6 setup disk, press F5 at the
   startup message (MS-DOS 6 clean boot), which skips the disk's startup files
   and with them DOS Setup. Then type C:\SYS31.BAT. It runs SYS C: (from the
   current directory, or through the RAM-drive PATH on DOS 7.1), copies
   COMMAND.COM itself if SYS did not, and requires IO.SYS and COMMAND.COM on C:.
   It expands Windows 3.1's own HIMEM.SYS to C:\ with Windows' EXPAND, so every
   DOS gets the same XMS driver. Our 8-byte DOSVER.COM (INT 21h function 30h,
   then exit with the major version as ERRORLEVEL) gates the MSDOS.SYS rewrite:
   only on DOS 7 or later are the startup options `BootGUI=0`, `Logo=0` and
   `BootDelay=0` written. MS-DOS 6 keeps part of its kernel in MSDOS.SYS, so
   that file is never touched there. Only then is our marker printed.
2. Boot C: with no floppy mounted, then type C:\INST31.BAT. Run
   SETUP /H:C:\SETUP.SHH from C:\WIN31SRC, installing to C:\WINDOWS. Request
   VGA, PS/2 mouse, an enhanced US keyboard and no network or printers.
   CONFIG.SYS loads the HIMEM expanded in stage 1; setup may rewrite it. Once the
   graphical setup phase has been painted for five million retired
   instructions, press ENTER once. Setup returns to DOS; our completion marker
   additionally requires WIN386.EXE and SYSTEM.INI.
3. Standard mode: boot C: with no floppy and type WIN /S with PROGMAN.EXE as the
   explicit shell. Mode evidence counts only after the command was typed. Once
   the CPU is in protected mode (paging never on) and a populated 640x480 frame
   has stayed identical at every poll for ten million retired instructions
   (keys typed while Windows still shows its hourglass are lost), type Alt+F,
   R and C:\COMMAND.COM: Program Manager's File/Run command. Require a decoded C: prompt with the CPU back in real mode (standard
   mode runs DOS programs in real mode), type EXIT, then require a populated
   frame at every screen poll for five million retired instructions in
   protected mode without paging. Only the shell's own menu can start that
   prompt, so the round trip proves Program Manager is running.
4. Enhanced mode: boot C: with no floppy and type WIN /3 with PROGMAN.EXE. After
   the command, require paging, V86 execution and at least one delivery that
   leaves V86 for the monitor (VM flag set before, clear after); with the CPU
   in paged protected mode and the same settled populated frame, send the same
   File/Run keys. COMMAND.COM opens full screen (ASSUMED: Windows' default
   program settings; MEASURED: its prompt is decoded as text). Require a C: prompt while the CPU is in paged V86 mode, type EXIT, then require a
   populated frame at every poll for five million retired instructions in
   paged protected mode. Graphics alone cannot qualify the stage.
5. Enhanced mode under DOS 7 (`user_media_win31_enhanced_refused_by_dos7`):
   the same WIN /3 command; the stage passes only when the DOS kernel's
   incompatibility message is decoded before paging is ever enabled.

Booting stage 2 from A: does not work: setup updates the startup files of the
drive the system booted from, finds the floppy read-only and waits in a
Retry/Cancel dialog (**MEASURED**, first attempt). Booting C: is therefore part
of the plan, which is why stage 1 provides an XMS driver before Windows exists.

**READ:** [Microsoft Windows 3.1 Resource Kit, chapter 1, pp. 75–81](https://bitsavers.trailing-edge.com/pdf/microsoft/resource_kits/0030-31645_Windows_3.1_Resource_Kit_199202.pdf)
confirms `/H`, profile identifiers, positional user information, printer
omission, and `endopt=exit` with `configfiles=modify`. The generated file uses
these documented sections rather than copying the retail template. The
requested device identifiers, including the no-network identifier, were
checked for presence in the supplied SETUP.INF; its text is not reproduced.
`/H` works: setup ran unattended apart from the one ENTER above, and no
Express keystroke script was needed (**MEASURED**).

Boot allows 200 million retired instructions. SYS and WIN startup allow another
300 million after command injection; installation allows one billion. Virtual
time also has a fixed ceiling (600 million, or two billion for installation),
so HLT cannot make a probe unbounded. Installation also stops after 150 million
retired instructions without a new ATA write command. Host elapsed time is
diagnostic only. The shared Findings report adds PIC OCW2/OCW3, special-mask
and poll requests, PIT channel/mode counts and VGA indexed-register sets to the
ATA, 8042 auxiliary, DMA and FDC summaries. ATA payload redaction remains in
force.

### Windows 3.1 findings (2026-11-27)

**MEASURED**, x86jit `f0785d5a`, retail Windows 3.1 floppy set merged into one
directory (`UNEMUPC_USER_WIN31_DIR`), generic-486-isa / 80486DX / 16 MiB RAM /
SeaBIOS + SeaVGABIOS / PS/2 mouse. Two DOS boot floppies were used as
`UNEMUPC_USER_DOS_FLOPPY`: MS-DOS 6.22 setup disk 1 and the MS-DOS 7.1 boot
floppy from Windows 95 OSR2. Retired counts are per stage (each stage is a
fresh machine) and were identical across repeated runs; host time is the
test-profile build on a shared host.

With **MS-DOS 6.22** (all five tests pass; `enhanced_refused_by_dos7` skips
after stage 1 found no DOS 7 marker):

| Stage | Command typed (retired) | File/Run sent / DOS prompt and EXIT (retired) | Boundary (retired) | Host elapsed | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| 1, SYS C: (F5 clean boot at 11,870,969) | 12,785,258 | — | 35,378,166 | 9–12 s | passes |
| 2, SETUP /H (one ENTER at 233,545,430) | 49,682,682 | — | 675,049,255 | 178–200 s | passes |
| 3, WIN /S (standard mode) | 52,582,590 | 146,034,692 / 162,839,154 | 172,525,865 | 62.7 s | passes |
| 4, WIN /3 (386 Enhanced mode) | 52,582,590 | 141,761,180 / 156,475,486 | 163,644,543 | 73.1 s | passes |

With **MS-DOS 7.1** (all five tests pass; `stage3` skips after stage 1 found
the DOS 7 marker, and `enhanced_refused_by_dos7` asserts the refusal):

| Stage | Command typed (retired) | File/Run sent / DOS prompt and EXIT (retired) | Boundary (retired) | Host elapsed | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| 1, SYS C: (no-CD option at 43,791,849) | 130,677,743 | — | 145,195,015 | 34–53 s | passes |
| 2, SETUP /H (one ENTER at 213,244,393) | 24,411,480 | — | 654,946,401 | 186–215 s | passes |
| 3, WIN /S (standard mode) | 28,318,246 | 118,850,515 / 141,574,544 | 149,271,851 | 47.0 s | passes |
| 5, WIN /3 refused by the DOS 7 kernel | 28,318,246 | — | 34,177,828 | 9.4 s | passes: refusal decoded before paging |

Host times in these runs include contention from other jobs on the shared
host; an earlier, quieter run of the same stages took 8–12 s and 144–151 s for
MS-DOS 6.22 stages 1 and 2.

Setup's graphical phase runs in 16-bit protected mode at CPL 3 without paging
(CR0=00000011h). Derived SYSTEM.INI flags after installation, identical with
both DOS versions, no values quoted: standard VGA display driver, PS/2 mouse
description, Program Manager shell, network driver empty, `[386Enh]` section
present. Setup therefore detected or accepted the requested machine, display,
mouse and no-network choices.

Standard mode runs Program Manager with either DOS: after WIN /S the CPU is in
protected mode at CPL 3 with paging never on; File/Run opens COMMAND.COM, whose
C: prompt is decoded with the CPU back in real mode; after EXIT a populated
640x480 frame holds for five million instructions in protected mode
(CR0=00000011h).

**386 Enhanced mode with MS-DOS 6.22.** After WIN /3 the harness saw, all after
the command: paging on, V86 execution, and a delivery leaving V86 for the
monitor. A populated frame then stayed identical at every poll for ten million
instructions, and File/Run was sent at 141,761,180 retired. COMMAND.COM's C:
prompt was decoded at 156,475,486 retired while the CPU was in paged V86 mode;
EXIT was typed, and a populated frame then held at every poll for five million
instructions in paged protected mode (boundary 163,644,543). The final state is
protected mode, CR0=80000013h (PG, ET, MP, PE), with the CPU in the VMM at
CPL 0. The latest deliveries are V86-to-CPL 0 transitions: #GP faults with
error code 025Ah from V86 code, #UD faults at linear FFB17h, and software
INT 13h reflected from V86. These are the VMM's V86 monitor at work (ASSUMED:
#UD at that fixed BIOS address is WIN386's V86 breakpoint mechanism). No
deliveries were dropped. No CPU stop and no device stop. A first attempt that
sent File/Run at the first populated frame lost the keys to Windows' hourglass
screen, which is why the harness waits for a settled frame (**MEASURED**).

**MS-DOS 7.1 refuses Enhanced mode.** After WIN /3 the CPU never
enables paging; the screen shows the DOS kernel's own message that this DOS
version is incompatible with this Windows version, and the system halts in
real mode (CS:EIP=0B01:00001513h, CPL 0, CR0=00000010h, timer IRQ 0 and
INT 1Ch still delivered). The message text is present in the boot floppy's
IO.SYS and absent from the Windows 3.1 files and COMMAND.COM (**MEASURED**, by
searching the files; the text is not reproduced). It is the MS-DOS 7.x kernel
refusing the Windows 3.1 Enhanced-mode launcher, not an emulator stop.

**MS-DOS 6.22 setup disk boot.** Its startup files load keyboard support and
start DOS Setup. Without F5, the boot never reaches a prompt: a program
spins with interrupts disabled in a loop that waits for port 61h bit 4
(refresh detect) to change before testing the 8042 input-buffer flag. Bit 4
never changes because SeaBIOS programs only PIT counter 0 and leaves counter 1,
which drives refresh detect, unprogrammed. Retired instructions kept growing
while CS:EIP and every device counter other than port 61h stayed unchanged
(**MEASURED**). The harness uses MS-DOS
6's clean boot (F5), which skips the startup files; the board gap is recorded
for a follow-up (windows95-on-486.md, gap row 26).

Per-device port summary, MS-DOS 6.22 runs (the MS-DOS 7.1 runs match the
setup and standard-mode columns apart from counts):

| Device | Setup (stage 2) | Standard mode (stage 3) | 386 Enhanced mode (stage 4) |
| --- | --- | --- | --- |
| ATA | READ SECTORS 20h: 13,420; WRITE SECTORS 30h: 2,232; IDENTIFY ECh: 1; IDENTIFY PACKET A1h: 2 | 20h: 239; 30h: 9; ECh: 1; A1h: 2 | 20h: 660; 30h: 25; ECh: 1; A1h: 2 |
| 8042 | D4h (to mouse): 24; aux bytes reset FFh, sample rate F3h, resolution E8h, scaling E6h, ID F2h, enable F4h, disable F5h, status E9h | D4h: 28; same aux set without F5h/E9h | D4h: 11; same aux set as standard mode; command byte read 20h once; keyboard disable ADh: 4 |
| PIC | ICW1=11h once per controller; EOI 20h: 781; specific EOI 61h: 3; OCW3 read ISR 0Bh: 513 | EOI 20h: 292; specific EOI 61h: 38; OCW3 0Bh: 266 | ICW1 twice per controller (re-initialised at vector bases 50h/58h); EOI 20h: 172; specific EOI 60h: 100, 61h: 50; OCW3 read IRR 0Ah: 17,298 master, 4 slave; **special mask mode set (OCW3 6Bh) once per controller**; no poll |
| PIT | channel 0 mode 2 (34h) once; read-back latch of counter 0 (D2h): 1,914,389 | 34h three times, channel 0 mode 3 (36h) once; D2h: 546,440 | 34h four times; counter 0 latch (00h): 17,296; D2h: 648,050 |
| DMA | master clear and second-controller setup only; no transfer | same | clear byte pointer on both controllers, mask writes; no transfer |
| VGA | attribute 00h–14h and 16h, sequencer 00h–04h, graphics 00h–08h, CRTC 00h–18h and F9h | attribute 00h–14h, sequencer 00h–04h, graphics 00h–08h, CRTC 00h–18h | attribute 00h–14h and 16h, sequencer 00h–04h and 06h, graphics 00h–08h, 0Ah and 0Fh, CRTC 00h–18h |

All trace rings had zero dropped events. PIT read-back (D2h) dominates timer
traffic in every Windows phase, which makes it a throughput factor (Intel 8254
datasheet, Read-Back Command). Setup also selects CRTC index F9h, and the
Enhanced-mode display driver selects sequencer 06h and graphics 0Ah/0Fh, all
outside the IBM VGA register ranges; whether they read back a value was not
examined. VPICD's special mask mode was not implemented by the PIC model at
the time of these runs (Intel 8259A datasheet, OCW3 ESMM/SMM); Windows 3.1 ran
without a visible fault regardless.

**MEASURED** rerun with special mask mode modelled (MS-DOS 6.22 floppy,
`user_media_win31_stage1` to `stage3` pass; the PIC counts of the Enhanced
column are unchanged). The runner now also prints the PIC command writes that
follow each controller's first special-mask write, and the 8259 lines of the
diagnostic report at the stage end. No ICW1 follows the 6Bh on either
controller, so VPICD sets the mode after re-initialising at 50h/58h. After the
6Bh the master sees specific EOIs 60h (100) and 61h (50) and IRR reads 0Ah
(17,297), the slave 0Ah (3); all 172 non-specific 20h EOIs come before it. At
the end of the Enhanced stage both controllers report special mask on, at
bases 50h and 58h.

Screen polling costs: capturing the 640x480 frame after every 10,000-unit run
slice held setup's graphical phase to about 0.3 million instructions per host
second. The shared runner now drains traces every slice but captures the screen
once per 100 slices (about one million instructions), which raised the same
phase to about 6 million per second (**MEASURED**, same stage, same pin).
