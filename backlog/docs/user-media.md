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
Host elapsed time is diagnostic only. Polling is every 10,000 virtual units.

A clean stage or budget boundary finishes a journal generation. An execution
error checkpoints only completed sectors, preserving a diagnostic generation
without marking the session successfully finished. This is a disk checkpoint,
not a whole-machine snapshot: a later stage boots anew from the same immutable
base plus journal. Continuation to file-copy completion, setup's reboots and
first desktop are follow-up work; no later-stage runner is claimed here.

The stop report prints mode, CPL, CR0, CS:EIP, linear address, instruction bytes,
retired count, deliveries, the last 64 port accesses and complete per-device
access counts and command/register-write histograms (ATA, 8042 auxiliary,
PIC ICW/OCW, DMA, FDC). Trace rings are drained at every poll and overflow is an
error. ATA data-port payload values (1F0h and 170h) are omitted from diagnostic
history; their port, direction, transfer size and virtual timestamp remain.
Command/register histograms never include ATA data-port payloads. A synthetic
test compares all port diagnostics for different key-like transfer bytes.
Decoded text is printed only to the local process output, with an optional
key redacted. Do not publish that output: reports in this document contain only
our observations, no media hashes, product keys, commercial frame hashes or
transcripts. CI must not set the private-media variables.

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

The current CPU diagnostic snapshot exposes a 16-bit IP and bypasses paging
when reading code. This harness computes the full CS-base + EIP and reads the
unpaged code bytes itself; under paging it explicitly reports bytes unavailable
instead of showing bytes from the wrong physical address. A future paged-stage
diagnostic needs a CPU seam that can inspect translated code without side
effects. That limitation does not affect this unpaged refusal.
