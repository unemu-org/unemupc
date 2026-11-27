# Windows 95 on generic-486-isa: gap survey and bead split (upc-18x.15)

Scope: what Windows 95 (retail and OSR2) setup and the first boot to the desktop
need from `generic-486-isa` as it stands at `fbea37cd`, CPU pin x86jit
`4bf91c8d`. Original gap survey; local stage-1 findings are now recorded
in `user-media.md` §Windows 95 findings.

Evidence labels:

- **READ** — read in this repository or in x86jit's docs/source at the pin, in this session.
- **MEASURED** — a figure recorded in `backlog/docs/status.md`.
- **ASSUMED** — Windows 95 behaviour recalled from public sources (Windows 95
  Resource Kit, Windows 95 DDK, Schulman "Unauthorized Windows 95", Pietrek
  "Windows 95 System Programming Secrets", Microsoft Knowledge Base, common
  QEMU/86Box practice). Not verified in this session; treat as a hypothesis the
  first run confirms or refutes.

## 1. Headline

1. No device is missing for the cheapest install path. Copy the setup files to
   the IDE disk, boot DOS from a floppy and run `SETUP` from `C:`. Everything
   on that path exists today; what is missing is a local test harness and the
   fixes the first runs will expose.
2. The first local setup run at x86jit `4bf91c8d` stopped on a CPU refusal
   (`LegacyEntryFacility` at `MOV ES,BX`, protected/unpaged CPL 3, before
   setup's first screen; `user-media.md` §Windows 95 findings). **MEASURED** at
   `f0785d5a`: the launcher reaches the first populated setup screen
   (273,257,205 retired, 58–99 s) and that refusal no longer occurs. Later setup
   and VMM startup remain unqualified. x86jit provides V86 with paging, 32-bit
   gates, the TSS I/O bitmap at CPL > IOPL, INVLPG and WP; these still need
   guest-path qualification beyond the first setup screen. The typed refusals
   that remain are ones Windows 95 is not expected to reach (ASSUMED, section 4).
3. Speed is no longer the dominant cost. **MEASURED** at x86jit `f0785d5a`, the
   paged user-mode mixes on the two 486 entries run at 12.3–25.6 MIPS against
   the virtual 486DX-33 rate of about 16.5 MIPS (section 3). The conditional
   CPU-bound wall-time estimate for 20–30 billion instructions is now
   0.22–0.68 hours; the Windows regime mix and instruction budget remain
   ASSUMED. Windows 3.1, the stepping stone, installs and reaches Program
   Manager in 386 Enhanced mode, with a DOS box round trip, in minutes on
   MS-DOS 6.22 (`user-media.md` §Windows 3.1 findings).
4. SeaBIOS is a proven Windows 95 firmware in QEMU (ASSUMED, common practice),
   and our build keeps the relevant services: INT 15h 88h/E801h/E820h, APM 1.2,
   INT 13h with EDD 3.0, ISA ATA on both channels, INT 15h C2h mouse.
5. Whole-machine snapshots are deferred for the 386/486 boards
   (`deferred.md:69`, `status.md:239`, `:257`). The plan stages work through
   IDE journal generations at setup's own reboots, not through snapshots.

Windows 3.1 stepping-stone findings that change the plan (**MEASURED** at
`f0785d5a`, `user-media.md` §Windows 3.1 findings):

- Setup updates the startup files of the drive the system booted from. Booted
  from the read-only DOS floppy it waits in a Retry/Cancel dialog; booted from
  C: it completes. Windows 95 stages after SYS should boot C: as well
  (ASSUMED for Windows 95 setup, which also rewrites the boot drive's files).
- The MS-DOS 7.1 boot floppy cannot host Windows 3.1 Enhanced mode: its kernel
  refuses WIN /3 before paging. With an MS-DOS 6.22 boot disk, WIN /3 runs:
  WIN386's VMM turns paging on, runs the DOS VM in V86 mode, a DOS box opened
  from Program Manager's File/Run reaches its prompt, and after EXIT a
  populated Windows frame holds in paged protected mode, with no CPU or device
  stop. The VMM/V86/VxD path Windows 95 shares is therefore exercised end to end on
  this board.
- Windows 3.1's VPICD sets the 8259A special mask mode once per controller
  (OCW3 6Bh) and re-initialises both controllers at vector bases 50h/58h; the
  PIC model does not implement special mask mode (gap row 17). Nothing failed
  visibly in this run, but the behaviour is unmodelled.
- SeaBIOS never programs PIT counter 1, so the port 61h refresh-detect bit
  (bit 4) never toggles on the generic ISA boards. An MS-DOS 6 startup program
  that times an 8042 wait with that bit spins forever with interrupts off. The
  harness avoids it with MS-DOS 6's clean boot; the board gap stays open (new
  row 26).
- Standard mode works end to end: 16-bit protected mode at CPL 3 with LDT
  segments, PS/2 mouse initialisation through 8042 D4h, VGA mode 12h drawing,
  PIC EOI/ISR reads and heavy PIT read-back (D2h) use. No device or CPU stop.

## 2. Install path options

| Path | What it needs here | Cost | Verdict |
| --- | --- | --- | --- |
| A. ATAPI CD-ROM (with or without El Torito) | An ATAPI device (PACKET, INQUIRY, READ(10), READ CAPACITY, READ TOC, MODE SENSE, REQUEST SENSE, TEST UNIT READY, media change), an ISO media seam, a second IDE channel. READ: the ATA model has "no DMA, packet" (`crates/devices/src/ata.rs:45`). READ: SeaBIOS is ready (`hw/ata.c:729` ATAPI identify, `:1022-1028` both ISA channels, `Kconfig:383` CD boot default on). ASSUMED: Windows 95 CDs are not bootable, so a DOS floppy with a real-mode CD driver is still needed. | L | Later. Phase C needs it for Windows 98 SE anyway. |
| B. DMF 1.68 MB floppies | READ: the FDC knows four formats, none with 21 sectors (`upd765.rs:50-56`). READ: SeaBIOS has no 21-sector entry (`hw/floppy.c:88-100`) and rejects `sector > nls` (`disk.c:142`), so real-mode DOS cannot read DMF without a fork patch. Twelve timed disk swaps on top. ASSUMED: only the retail edition shipped on floppies. | M-L, touches the fork | Not recommended. |
| C. Setup files on the hard disk, `SETUP` from DOS | A FAT16 IDE image of about 500 MB built on the host from a user directory, a DOS boot floppy, keyboard input. READ: IDE geometry to 16383/16/63 with LBA28 and journal write mode (`ata.rs:43`, `machines/at-games.toml:83-84`); the DOOM helper already builds a FAT16 disk from a directory (`backlog/docs/user-media.md` § Temporary disk). | S | **Recommended.** |

Path C runs the real setup end to end: the real-mode stub, the Windows 3.1
mini-kernel GUI, hardware detection, CAB extraction, boot-sector and `IO.SYS`
install, reboot, first-boot configuration, desktop (ASSUMED sequence, Resource
Kit). Windows later finds its CABs on `C:`, so no CD-ROM is needed for adding
drivers.

Two details make path C cheap and repeatable (ASSUMED, Resource Kit):

- `SETUP /is` skips ScanDisk, which is the step most likely to object to a
  non-Microsoft DOS.
- A test-owned `MSBATCH.INF` runs setup unattended (name, time zone, no startup
  disk, express components). It is our own text and safe to commit. The product
  key is the one private value; it comes from an environment variable and is
  written only to the temporary disk, never to logs or repository files.

DOS launcher, in order of preference:

1. The vendored open FreeDOS floppy (`machines/generic-486-freedos.toml`). No
   new private media and the DOOM helper is reused almost as is. ASSUMED risk:
   Windows 95 setup under FreeDOS is reported to work by hobbyists but is not a
   supported configuration. Boot without a memory manager: x86jit refuses V86
   16-bit gates and the mini-kernel wants plain real mode.
2. The maintainer's own MS-DOS or Windows 95 boot floppy, supplied by a second
   environment variable. Fallback if FreeDOS fails.

## 3. Speed budget (ESTIMATE from MEASURED rates)

**MEASURED 2026-11-27**, release, ARM64 macOS host class, exact x86jit pin
`f0785d5a2878c2b9e7392b937aa33dcdcc979f84`, with
`cargo run --offline --release -p pc-harness --example cpu_speed_ceiling`.
Each cell is samples 1 / 2 / 3 in MIPS; every sample retires exactly
10,000,000 instructions. There are 105 samples in total. See
[status.md](../status.md#nominal-cpu-clocks-and-turbo-2026-11-24) for the
fixture method, state checks, monitor counts and paired I/O deltas.

| Case | IBM 5170 / 80286 | Generic 386 / 80386DX | Generic 486 / early 80486DX | Generic 486 / CPUID 80486DX |
| --- | --- | --- | --- | --- |
| `real16` | 111.035, 111.593, 113.569 | 108.451, 108.968, 107.347 | 104.115, 104.348, 101.809 | 91.148, 94.431, 97.321 |
| `protected16` | 70.347, 68.954, 68.507 | — | — | — |
| `protected32` | — | 67.956, 67.134, 67.593 | 63.766, 67.452, 66.453 | 61.274, 61.989, 64.813 |
| `paged32` | — | 46.766, 46.279, 46.237 | 45.971, 46.249, 45.689 | 43.927, 45.282, 45.535 |
| `flat32` | — | 65.402, 66.013, 66.476 | 63.622, 64.066, 63.971 | 65.282, 64.126, 64.438 |
| `flat-paged32` | — | 46.002, 46.341, 46.644 | 44.733, 44.877, 44.916 | 45.289, 45.213, 44.775 |
| `flat32-cpl3` | — | 40.509, 40.662, 40.254 | 39.945, 37.869, 31.571 | 40.256, 37.461, 35.705 |
| `flat-paged32-cpl3` | — | 25.953, 25.824, 25.695 | 25.578, 22.498, 20.213 | 25.560, 23.402, 22.941 |
| `flat-paged32-cpl3-farcall` | — | 18.777, 18.359, 18.472 | 18.297, 15.088, 14.146 | 18.385, 16.211, 15.234 |
| `paged-protected16-cpl3` | — | 15.916, 15.904, 15.903 | 15.287, 12.768, 12.330 | 14.954, 13.817, 13.064 |
| `paged-v86` | — | 17.699, 17.547, 17.629 | 17.355, 13.765, 14.387 | 15.634, 16.180, 14.239 |
| `paged-v86-iopl0` | — | 14.890, 15.014, 14.954 | 14.242, 11.315, 12.676 | 13.335, 12.728, 11.836 |

The mixes are unchanged from the earlier measurement: representative Win32
uses near CALL/RET only (8 pairs per 100 retired), native dword operations and
retained flat data selectors; the separate `flat-paged32-cpl3-farcall` stress
case uses 4 far pairs and 4 near pairs per 100. Protected16 and V86 each
execute 4 far pairs, 4 near pairs and 4 loads of each of DS/ES per 100 retired;
IOPL 0 adds a trapped IN per ten iterations with nine retired CPL 0 monitor
instructions. Other workers shared the host during this run and were not
controlled: **the largest sample spread is 29%** (`(max - min) / min`, the
early 486 entry in the CPL 3 cases); the 386 rows stay within 2%. The table
keeps all three samples; read the lower samples as contention, not as a
regression. At `8f23fdcd` the same paged protected16 and V86 cases ran at
about 1 MIPS, and paged32 at about 3.3 MIPS.

| Phase | CPU regime | Measured host MIPS, both 486 entries | Against virtual 16.5 MIPS |
| --- | --- | --- | --- |
| DOS, real-mode stub | Real16 | 91.148–104.348 | 5.52–6.32x |
| First boot, VMM and VxDs | Flat32 CPL 0, paged; register-only control | 44.733–45.289 | 2.71–2.74x |
| KRNL386, USER, GDI, setup under the VMM | Protected16 CPL 3, LDT, paged | 12.330–15.287 | 0.75–0.93x |
| Win32 programs | Flat32 CPL 3, paged; near calls only | 20.213–25.578 | 1.23–1.55x |
| BIOS and DOS calls under Windows | V86, paged, IOPL 3 control | 13.765–17.355 | 0.83–1.05x |
| Mini-Windows setup front end | Protected16 CPL 3, unpaged | no CPU-only case; the Windows 3.1 setup stage, whole machine with full port tracing, averages 4.5 M instructions/s | n/a |
| Idle desktop | HLT | free (decision-14) | n/a |

| Separate stress case | Measured host MIPS, both 486 entries |
| --- | --- |
| `flat-paged32-cpl3-farcall` | 14.146–18.385 |
| `paged-v86-iopl0` | 11.315–14.242 |

**ASSUMED:** setup plus first boot on a real 486DX-33 is 30–60 minutes,
with 20–30 minutes CPU bound, about 20–30 billion instructions. Dividing that
budget by the representative paged user-mix extremes on the two 486 entries
(12.330–25.578 MIPS: protected16, near-call Flat32 and IOPL-3 V86) gives
**0.22–0.68 hours of host wall time** for the CPU-bound portion. The far-call
Flat32 and trapped-I/O V86 stress cases are excluded. This is a conditional
extrapolation: the instruction budget, regime proportions and actual I/O-trap
frequency are unverified, and the range includes shared-host contention.
Full setup also has storage and instrumentation costs.

The whole-machine stage runs add device and diagnostic costs on top of the CPU.
**MEASURED** at `f0785d5a` in the instrumented stage tests (test profile,
full port and delivery tracing, shared host): Windows 95 stage 1 reaches
setup's first screen at 273,257,205 retired in 58–99 s across runs
(2.8–4.7 M/s). Windows 3.1 setup runs 675,049,255 retired under MS-DOS 6.22 in
178–200 s (3.4–3.8 M/s) and 654,946,401 retired under MS-DOS 7.1 in
186–215 s (3.0–3.5 M/s), with other jobs on the host (`user-media.md`
§Windows 3.1 findings). Capturing the 640x480 screen
after every 10,000-unit run slice had held Windows 3.1's graphical setup to
about 0.3 M/s; polling once per million instructions raised it to about 6 M/s.
Harness polling, not the CPU, was the bottleneck there.

Future CPU throughput comparisons should keep these mixed paged cases: they
cover the data, segment and call traffic that the original register-only
controls leave unmeasured.

## 4. Gap table

Owner: U = unemupc, S = SeaBIOS fork or its config, X = x86jit. Size S/M/L.
"Setup" = blocks reaching the end of setup; "Desktop" = blocks the first
desktop; "Later" = after the first desktop.

| # | Area | Need | Current state and evidence | Owner | Size | Blocks |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Test harness | A local-only test fed by environment variables: build the disk, launch setup, stage at reboots, report stops | READ: the DOOM helper exists (`crates/machine-file/tests/support/user_media.rs:122`), but it takes one flat directory, a 34 MiB 4x32 geometry and DOOM-specific names (`user-media.md` § Temporary disk) | U | M | Setup |
| 2 | Disk image | About 500 MB FAT16, active partition, subdirectories | MEASURED: synthetic 1015/16/63 (523,837,440 bytes) and 2048/16/63 (1,056,964,608 bytes) disks enumerate alongside the probe floppy; native EDD capacity and final-LBA data agree. This qualifies geometry/addressing, not FAT16. READ: nested FAT16 construction has separate disk-builder tests (`status.md` § Local Windows 95 setup launcher). | U | S | Setup |
| 3 | DOS launcher | Setup starts from the open FreeDOS floppy | Untested. ASSUMED risk, fallback in section 2 | U | S | Setup |
| 4 | INT 13h | CHS geometry Windows agrees with; extensions optional | MEASURED: `seabios_int13_disk_services` on all three CPU entries checks AH=08h/41h/48h/42h and a final reported CHS-cylinder read. AH=08h maximum C/H/S: 1013/15/63 and 1022/31/63 (landing cylinder reserved, translation to 32 heads). EDD 3.0 reports native CHS/full capacity; both buffers match synthetic sectors. | U | S | - |
| 5 | Protected-mode IDE driver (ESDI_506.PDR) | IDENTIFY, SET MULTIPLE, READ/WRITE (MULTIPLE), INITIALIZE PARAMETERS, RECALIBRATE with IRQ14; absent slave and absent secondary channel answer cleanly | MEASURED: `seabios_ata_protected_driver` on all three CPU entries runs flat32 IDENTIFY, SET MULTIPLE (2), four-sector READ/WRITE MULTIPLE and readback, INITIALIZE DEVICE PARAMETERS, SET FEATURES (PIO4), RECALIBRATE and A1h ABRT. Twelve IRQ14 deliveries enter the protected IDT, with expected block/completion statuses and data. Absent slave status/error=00h/00h; secondary status/control=FFh. ASSUMED: ESDI_506.PDR uses this subset; the real Windows driver remains unqualified. | U | S-M, fix as found | Desktop only if it hangs |
| 6 | ATAPI CD-ROM | Not needed on path C | READ: absent (`ata.rs:45`); firmware side ready | U | L | Later |
| 7 | PS/2 mouse | 8042 aux port, IRQ12, standard ID 00 mouse | MEASURED: `seabios_mouse_aux_and_bios_handler` lowers `board.mouse = true` on each board/CPU entry. INT 15h C2h initialise/set-handler/enable/disable succeeds; injected motion reaches its BIOS callback. Direct reset gives FAh AAh 00h; sample-rate 100, enable and ID commands ACK; ID=00h. Three IRQ12 bytes preserve signs/buttons and aux OBF without parity/timeout. Windows driver compatibility remains ASSUMED; sample-rate pacing remains unmodelled. | U | S | Later (setup works from the keyboard) |
| 8 | Serial mouse | Alternative to PS/2 | READ: exists (`assemble.rs:367-372`), but COM1 is SeaBIOS's serial console on these boards (`board.rs:222-229`, `fw_cfg.rs:28`), so the two collide. Prefer PS/2 | U | - | - |
| 9 | Keyboard | 8042 command byte, A20, LEDs, typematic | READ: table at `i8042.rs:113-131`; D2h and E0h not modelled. ASSUMED enough | U | S if found | - |
| 10 | Memory size | INT 15h 88h, E801h, E820h | READ: all present (`system.c:170`, `:260`, `:290`); RAM ceiling 64 MiB (`memory.rs:13-17`). Use 16-32 MiB | - | - | - |
| 11 | APM | Connect, CPU idle, power off | MEASURED: `seabios_apm_protected_power_off` checks APM 1.2 install/5303h connect, builds descriptors from returned bases/lengths, calls the 32-bit entry for version negotiation and CPU idle (5305h), wakes through one protected IRQ0, then powers off all devices (5307h). The run ends at the BIOS HLT with IF=0; off does not return. All three CPU entries pass. | U | S (probe) | - |
| 12 | PnP BIOS | Optional | READ: `$PnP` header present, every call returns "not supported" (`pnpbios.c:36-62`, `Kconfig:406`). ASSUMED: Device Manager shows a failing "Plug and Play BIOS" node, cosmetic. Turning `CONFIG_PNPBIOS` off means a firmware rebuild, re-pin and golden re-run | S | S | Later, optional |
| 13 | IRQ routing, PCI | None on ISA | READ: PCI BIOS, PIR, ACPI, MP tables off (`vendor/seabios/config:17-33`). Undecoded ports float to FFh (`crates/bus/src/lib.rs:99`), which is what legacy and ISA PnP detection expects | - | - | - |
| 14 | CMOS/RTC | Time, alarm, periodic, 128 bytes | READ: 128-byte RTC with the QEMU layout (`board.rs:208-210`, `assemble.rs:1811-1860`); periodic partial, alarm and update-ended modelled (`cmos_rtc.rs:64-66`) | - | - | - |
| 15 | VGA 640x480x16 | Planar writes, write modes 0-3, latches, bit mask, set/reset, read mode 1, full register readback | READ: all four write modes, rotate, set/reset, logic, bit/map masks, read-map select, colour compare (`vga.rs:23`); planar 16-colour and 256-colour scanout (`vga.rs:25`); the attribute-controller policy Windows 3.x `vga.drv` needs (`vga.rs:18`). Not modelled: RAM-enable gating, IRQ, mid-frame changes (`vga.rs:13`, `:25`). SeaVGABIOS is plain VGA without VBE (`vendor/seabios/vgaconfig`). ASSUMED: enough for the standard VGA driver and the 320x400 boot logo; the display VxD's save and restore of the whole register set is the untested part | U | S-M, fix as found | Setup (GUI is mode 12h) |
| 16 | More colours or resolution | VBE or an SVGA card | Phase C, upc-18x.2 | U | L | Out of scope |
| 17 | PIC | Whatever VPICD programs | READ: special mask mode, poll mode, level trigger and rotation are not modelled (`pic8259.rs:85-94`). MEASURED (Windows 3.1 Enhanced mode, MS-DOS 6.22): VPICD re-initialises both controllers (bases 50h/58h), sets special mask mode once per controller (OCW3 6Bh), uses specific EOI (60h/61h) and IRR reads (0Ah); no poll. Windows 3.1 ran without a visible fault, but special mask mode is used and unmodelled. ASSUMED: Windows 95's VPICD does the same | U | M | Desktop if it matters |
| 18 | PIT, DMA | VTD and VDMAD program them directly | Present; SB16 DMA goldens pass (`status.md:3-17`). ASSUMED fine | - | - | - |
| 19 | Floppy driver (HSFLOP.PDR) | Controller-type probe | READ: unknown commands answer 80h like a plain uPD765 (`upd765.rs:1041`). ASSUMED: treated as a basic controller | U | S if found | Later |
| 20 | SB16 driver | DSP version, mixer IRQ/DMA readback, playback | READ: DSP 4.05, mixer 80h/81h readback (`sb16.rs:44`), OPL3. MIDI (MPU-401) and input are not modelled (`sb16.rs:23`). ASSUMED: playback works; the MPU-401 child device may fail to start | U | S playback, M MPU-401 | Later |
| 21 | Numeric errors | ASSUMED: Windows 95 may keep CR0.NE = 0 and expect IRQ13 | READ: external routing refused on both 486 entries (`protected-mode-gaps.md:465`); upc-18x.11 and xj-31k.4 (x86jit) already cover it. Unmasked x87 exceptions are rare at the desktop | X then U | M | Later |
| 22 | CPU, functional | V86 under paging, 32-bit gates, I/O bitmap, ARPL as #UD in V86, ring transitions, demand paging, CR3 reloads, INVLPG | READ at the pin: all shipped (x86jit `deferred.md:261-278`, `design/task-switches.md:64-68`, `design/v86.md:131`; ours `protected-mode-gaps.md:462-468`). Remaining typed refusals: V86 16-bit gates, AM+AC, DR7 with enable bits or GD (x86jit `interp/legacy.rs:4117-4128`), TR6 writes (`:4133-4141`), TR3-TR5, 16-bit BSWAP, page tables on device or ROM, the TSS T bit. ASSUMED: Windows 95 reaches none without a debugger | X | unknown | Setup if hit |
| 23 | CPU, speed | Paged execution in all three regimes | MEASURED at `f0785d5a`: all three paged user regimes at 12.3–25.6 MIPS, near or above the virtual 486DX-33 rate; section 3 records every sample | X (bench: U) | M | Practical use |
| 24 | CPUID | Either 486 entry | READ: the CPUID entry reports FPU only (x86jit `model.rs:401`), so no VME is advertised. Start with `80486dx` | - | - | - |
| 25 | Snapshots | A checkpoint after setup would save hours | Deferred (`deferred.md:69`). Disk journal generations at reboots replace it for now | U | L | Not proposed |
| 26 | Port 61h refresh bit | Bit 4 toggles per DRAM refresh request | MEASURED: SeaBIOS programs PIT counter 0 only (READ: `vendor/seabios/src/src/hw/timer.c:274-283`), so counter 1 never runs and bit 4 never toggles; software that waits for toggles hangs. ASSUMED: AT-class BIOSes program counter 1 for refresh during POST (IBM Personal Computer AT Technical Reference, System Timers and the BIOS listing). Fix in the SeaBIOS fork or the board's reset state | S (or U) | S | Any guest delay loop on bit 4 |

Notes on rows where Windows 95 behaviour drives the risk:

- Row 5. ASSUMED: the driver compares BIOS geometry with IDENTIFY words
  1/3/6 and 54-58 before taking the disk over. The open probes measure both
  interfaces, including SeaBIOS's reserved landing cylinder; they do not prove
  how the Windows driver interprets that difference.
- Row 15. Setup's GUI is the Windows 3.1 VGA driver, which the 286 AT profile
  has already run (`status.md:975`); the generic boards with SeaVGABIOS have not.
- Row 22. VMM uses ARPL in V86 as its breakpoint, traps HLT and port I/O from
  V86, patches `INT 20h` VxD calls in place (self-modifying code in ring 0),
  and switches address spaces by rewriting page tables and reloading CR3. It
  uses one TSS for the ring-0 stack and the I/O bitmap and is not expected to
  use hardware task switches. It does not set CR0.WP or CR0.AM.
- Row 23. 16-bit Windows code makes far calls and segment loads constantly, so
  descriptor-load cost under paging matters as much as plain paged access.

## 5. Determinism and tests

Local-only test, following `backlog/docs/user-media.md`:

- `UNEMUPC_USER_WIN95_DIR`: a directory holding the setup files (the `WIN95`
  folder of a CD, or the files of a floppy set). Read only.
- `UNEMUPC_USER_WIN95_KEY`: the product key, written into the generated
  `MSBATCH.INF` on the temporary disk, never printed.
- `UNEMUPC_USER_DOS_FLOPPY`: a private DOS boot floppy image, required by
  the first implemented stage. The FreeDOS alternative above remains untested.
- Tests are `#[ignore]`, named `user_media_win95_*`, and skip when the variable
  is absent. No media bytes, hashes, frame hashes, screenshots or transcripts
  are stored.
- The initial implemented stage ends at setup's first screen or first stop.
  Planned continuation stages end at setup's own reboots: DOS to the end of
  file copy, first boot to the end of first-run configuration, then boot to
  the desktop and APM power-off. Each stage commits an IDE journal generation
  under the temporary directory, so a later stage can be rerun alone.
- Observations: decoded text for DOS prompts; for graphics, the BIOS mode and
  a "non-trivial picture" test as the DOOM probe uses; retired counts and wall
  time per stage; on a stop, CPU state, deliveries and last port I/O in the
  `user-media.md` § Findings format for the x86jit hand-over.
- A per-device port summary (ATA commands seen, 8042 aux commands, PIC OCW3
  writes) turns ASSUMED rows 5, 7 and 17 into READ facts on the first run.

Project-owned probes cover the following gaps in CI. The first four rows
are **MEASURED** at x86jit `ce6bbbee`, with the pinned unemu SeaBIOS fork and
SeaVGABIOS on 80386DX, 80486DX and 80486DX-CPUID. Their sources and frozen
binaries live in `crates/machine-file/tests/fixtures/seabios-{int13,ata-driver,mouse,apm}.*`;
`service_fixtures` rebuilds each with NASM and compares every byte. The shared
`support/generic_services.rs` helper uses a read-only in-memory floppy. Hard
disks use fully SHA-256-identified sparse zero bases. Synthetic sectors are
seeded directly through the media overlay, independently of ATA's address
decoder; guest writes use that same overlay. No device fix or pre-existing
golden change was necessary. Hardware references: ATA-2 (X3T10/0948D rev 4c),
§6.2 task file, §7 command descriptions and §§8.3–8.4 PIO protocols; IBM PS/2
Hardware Interface Technical Reference, Keyboard and Auxiliary Device
Controller / Auxiliary Device Interface and Pointing Device / Data Reporting;
APM BIOS Interface Specification 1.2, §§4.6.1 (installation), 4.6.4 (32-bit
connect), 4.6.6 (CPU Idle), 4.6.8 (Set Power State).
EDD 3.0 §5.8, Tables 3–5 define the separately asserted path length, ISA
interface address, ATA master selection and reserved zero fields; the
checksum is checked as well. IDENTIFY words 57–58 are checked separately
from LBA capacity words 60–61. The first WRITE MULTIPLE block qualifies BSY,
ERR and DRQ before transfer and saves its status for host validation
(ATA-2 §8.4). Mouse phase waits reject premature halt/reset and lack of
virtual-time progress before the expected marker.

| Gap rows | Probe |
| --- | --- |
| 2, 4 | MEASURED: `seabios_int13_disk_services`, both disk geometries, AH=08h/41h/48h/42h, last LBA and final reported CHS cylinder |
| 5 | MEASURED: `seabios_ata_protected_driver`, flat32 IDT/PIC IRQ14, IDENTIFY, multiple-block reads/writes, parameters/features, recalibrate and absent/aborted commands |
| 7 | MEASURED: `seabios_mouse_aux_and_bios_handler`, BIOS C2h callback plus direct aux commands and IRQ12. CuteMouse and the Windows driver remain unqualified |
| 11 | MEASURED: `seabios_apm_protected_power_off`, install/32-bit connect, protected entry, IRQ0 idle wake and terminal BIOS HLT with IF=0 |
| 15 | Mode 12h write modes, latch copies, read mode 1 and an INT 10h AH=1Ch save/restore round trip through SeaVGABIOS |
| 17 | The Windows 3.1 Enhanced-mode trace shows special mask mode (OCW3 6Bh, once per controller) and no poll mode: model special mask mode with isolation tests on the mock bus, then rerun the Enhanced stage |
| 22, 23 | A small V86 monitor with ARPL breakpoints, trapped I/O and CR3 switches already exists as the V86 and paging ROMs; add bench fixtures for paged protected16 CPL 3 and paged V86 |

## 6. Proposed beads, in order

1. **Local Windows 95 setup harness and disk builder.**
   Scope: generalise the user-media disk builder to subdirectories, a chosen
   size and geometry (default 1015/16/63) and an active FAT16 partition; add
   the environment variables of section 5, the generated `MSBATCH.INF`, the
   staged runner with IDE journal generations and the per-device port summary.
   Stage 1 only has to start setup.
   Acceptance: with the variables set, `user_media_win95_stage1` boots DOS,
   launches `SETUP /is` and either reaches the graphical setup or reports the
   first stop in the hand-over format; without them it skips; a synthetic
   builder test runs in CI.
   Depends on: nothing.

2. **Bench fixtures for the paged regimes Windows 95 lives in.**
   Scope: add paged protected16 CPL 3 (LDT selectors, far calls) and paged V86
   cases to the existing speed example and record the rates in `status.md`.
   Send x86jit the measured table and the order of benefit from section 3.
   Acceptance: rates on the 386 and both 486 entries recorded with exact
   retired counts; the x86jit request filed by that repository's owner session.
   Depends on: nothing. Runs beside bead 1.

3. **BIOS-service and driver-style probes on the generic boards.**
   Scope: the open probes of section 5 for rows 2, 4, 5, 7 and 11, as goldens
   on the 386 and both 486 entries. Device fixes only where a probe fails.
   Acceptance: goldens pass; `board.mouse` is exercised on a generic board;
   APM power-off ends a run cleanly.
   Depends on: nothing. Runs beside bead 1.

4. **Windows 3.1 386 Enhanced mode on generic-486-isa (local, optional).**
   Scope: with the maintainer's own Windows 3.1, reach Program Manager in
   Enhanced mode and open a DOS box. Same VMM architecture as Windows 95 (V86,
   ring-0 VxDs, paging, I/O trapping, VPICD, VDD) at a hundredth of the run
   time, so CPU and device findings arrive in minutes.
   Acceptance: Program Manager and a DOS box reached, or each stop handed
   over; findings in `user-media.md`.
   Depends on: bead 1.

5. **Windows 95 setup to its first reboot.**
   Scope: drive stage 1 to completion; fix board and device faults with
   isolation tests; hand CPU stops to x86jit and re-pin.
   Acceptance: setup finishes copying files and requests the reboot; the
   journal generation is kept; retired counts and wall time recorded.
   Depends on: bead 1; bead 4 if taken.

6. **Windows 95 first boot to the desktop.**
   Scope: stages 2 and 3. Record whether the IDE driver runs in protected mode
   or compatibility mode and which devices Device Manager lists.
   Acceptance: the desktop is reached from the keyboard, Shut Down ends the
   run through APM, and a second boot from the kept generation reaches the
   desktop again.
   Depends on: bead 5.

7. **PS/2 mouse at the Windows 95 desktop.**
   Scope: `board.mouse = true` in the test machine; injected motion moves the
   pointer; add sample-rate pacing only if the driver needs it.
   Acceptance: a scripted click opens the Start menu.
   Depends on: beads 3 and 6.

8. **SB16 under Windows 95.**
   Scope: the built-in driver detects the card at 220h/5/1/5 and plays a wave
   file; decide on an MPU-401 UART stub from what the driver does.
   Acceptance: non-silent PCM captured during a system sound.
   Depends on: bead 6. Related: upc-18x.3.

9. **Shipped machine file and user documentation.**
   Scope: `machines/generic-486-win95.toml` (journaled C:, PS/2 mouse, SB16,
   16-32 MiB), a `user-media.md` section, `status.md`, and the decision on
   `CONFIG_PNPBIOS`.
   Acceptance: a user with their own media can install by following the page.
   Depends on: beads 6-8.

10. **ATAPI CD-ROM and ISO media.**
    Scope: ATAPI packet device on the secondary channel, ISO media seam,
    SeaBIOS El Torito boot, the Windows protected-mode CD path.
    Acceptance: a CD-ROM is readable from FreeDOS with an open driver and from
    the installed Windows 95; an El Torito image boots.
    Depends on: bead 6 for the Windows half. Phase C needs it regardless.

Beads 5 and 6 are fix-as-found loops. Budget them as one reviewer round per
finding, not as one landing.

## 7. Open questions for the maintainer

1. Which media do you have: retail full, retail upgrade (its compliance check
   wants a Windows 3.x disk), or OEM OSR2? CD or floppy set? This decides the
   contents of the setup directory and whether FAT32 comes into play.
2. If setup will not start from FreeDOS, is a private DOS boot floppy an
   acceptable launcher, or should the FreeDOS launch be made to work?
3. Product key through an environment variable, never logged: acceptable?
4. Should x86jit be asked to put paged memory cost ahead of Flat32 CPL 3 and
   to add paged protected16 CPL 3 as a measured case?
5. Do you want the Windows 3.1 Enhanced-mode stepping stone (bead 4)?
6. Whole-machine snapshots on the 386/486 boards are deferred. Keep them
   deferred and stage through disk journals, or un-defer them for this work?
7. Is "desktop" for this target the picture plus a clean shutdown, or does it
   include mouse and sound (beads 7 and 8)?
8. ATAPI now, or with phase C?
9. PnP BIOS: accept a cosmetic failing node, or rebuild the firmware without it?
10. Where do these beads live: under upc-18x, or a new epic for Windows 95 on
    the 486 ahead of phase C?

## Windows 3.1 stepping stone (2026-11-27)

The maintainer selected Windows 3.1 in 386 Enhanced mode before the full
Windows 95 installation. **ASSUMED:** its VMM/VxD, paging, V86 and ring-3
16-bit protected execution exercise the same critical seams with a shorter
installation. The shared staged harness and FAT16 builder now accept retail
Windows 3.1 source files. Each stage resumes an IDE journal generation in a
fresh machine; no whole-machine snapshot or CPU workaround is introduced.
See [Windows 3.1](../user-media.md#windows-31) for the generated setup request,
measured boundaries and their proofs. With MS-DOS 6.22, Enhanced mode is
qualified by a Program Manager File/Run DOS prompt round trip in paged V86
mode; with MS-DOS 7.1 the DOS kernel's refusal is asserted instead. Neither
establishes Windows 95 readiness beyond the shared seams.
