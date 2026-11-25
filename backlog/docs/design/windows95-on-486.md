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
2. The first local setup run now exposes a CPU blocker: `LegacyEntryFacility`
   at `MOV ES,BX`, protected/unpaged CPL 3, before setup's first screen
   (`user-media.md` §Windows 95 findings). At survey time there was no known
   functional CPU blocker. x86jit ships V86 with paging, 32-bit gates,
   the TSS I/O bitmap at CPL > IOPL, INVLPG and WP. The typed refusals that
   remain are ones Windows 95 is not expected to reach (ASSUMED, section 4).
3. Speed is the real cost. Installed Windows 95 uses paging; the DOS launcher and mini-Windows
   setup front end can run unpaged (ASSUMED). MEASURED paged
   protected32 is 1.9-3.4 MIPS against a virtual 486DX-33 rate of about
   16.5 MIPS (`status.md:71-75`, `:99-117`). The desktop would run at roughly
   0.1-0.2x real time and a full setup would take hours of wall time.
4. SeaBIOS is a proven Windows 95 firmware in QEMU (ASSUMED, common practice),
   and our build keeps the relevant services: INT 15h 88h/E801h/E820h, APM 1.2,
   INT 13h with EDD 3.0, ISA ATA on both channels, INT 15h C2h mouse.
5. Whole-machine snapshots are deferred for the 386/486 boards
   (`deferred.md:69`, `status.md:239`, `:257`). The plan stages work through
   IDE journal generations at setup's own reboots, not through snapshots.

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

| Phase | CPU regime | Host rate today | Against the virtual 16.5 MIPS |
| --- | --- | --- | --- |
| DOS, real-mode stub | Real16 | 80-86 MIPS (`status.md:103`) | faster than real time |
| Setup GUI, detection, file copy | protected16, unpaged (ASSUMED: mini Windows 3.1 in Standard mode) | 8.3-9.0 MIPS (`status.md:103`) | about 0.5x |
| First boot, VMM and VxDs | flat32 CPL 0, paged | 3.0-3.4 MIPS (`status.md:116`) | about 0.2x |
| KRNL386, USER, GDI, setup's second half | protected16 CPL 3, paged | not measured; at most the paged rate | at most 0.2x |
| BIOS and DOS calls under Windows | V86, paged | not measured | at most 0.2x |
| Idle desktop | HLT | free (decision-14) | n/a |

ASSUMED: setup plus first boot on a real 486DX-33 is 30-60 minutes, of which
perhaps 20-30 are CPU bound, so 20-30 billion instructions. At today's rates
that is 2-4 hours of wall time for one full run. Correctness work can proceed
at that speed with staged runs; daily use cannot.

**MEASURED** in the instrumented local stage-1 test (not a CPU microbenchmark):
DOS prompt/command injection at 130,127,767 retired in 40.55 s, about 3.21 MIPS;
protected setup refusal at 249,085,227 retired in 82.03 s, about 3.04 MIPS
across the whole run. These figures include full port/delivery tracing, scanout
polling and host contention. They do not replace the CPU-only rates above or
establish a paged Windows throughput. Setup's first screen was not reached.

x86jit's stated order is Flat32 CPL 3, then paged memory cost, then
protected16. For Windows 95 the order of benefit is different: paged memory
cost first (everything is paged), then paged protected16 at CPL 3 (USER, GDI,
KRNL386 and all of setup), then Flat32 CPL 3 (Explorer and Win32 programs).

## 4. Gap table

Owner: U = unemupc, S = SeaBIOS fork or its config, X = x86jit. Size S/M/L.
"Setup" = blocks reaching the end of setup; "Desktop" = blocks the first
desktop; "Later" = after the first desktop.

| # | Area | Need | Current state and evidence | Owner | Size | Blocks |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Test harness | A local-only test fed by environment variables: build the disk, launch setup, stage at reboots, report stops | READ: the DOOM helper exists (`crates/machine-file/tests/support/user_media.rs:122`), but it takes one flat directory, a 34 MiB 4x32 geometry and DOOM-specific names (`user-media.md` § Temporary disk) | U | M | Setup |
| 2 | Disk image | About 500 MB FAT16, active partition, subdirectories | READ: geometry limit 16383/16/63 (`ata.rs:43`); CMOS 39h translation seeded as QEMU does (`assemble.rs:1841`, `:1868`); SeaBIOS reads it (`block.c:84-90`). A disk of at most 1024 cylinders needs no translation | U | S | Setup |
| 3 | DOS launcher | Setup starts from the open FreeDOS floppy | Untested. ASSUMED risk, fallback in section 2 | U | S | Setup |
| 4 | INT 13h | CHS geometry Windows agrees with; extensions optional | READ: EDD 3.0 present (`disk.c:409-414`). No gap known; needs a probe with a 1015/16/63 disk | U | S | - |
| 5 | Protected-mode IDE driver (ESDI_506.PDR) | IDENTIFY, SET MULTIPLE, READ/WRITE (MULTIPLE), INITIALIZE PARAMETERS, RECALIBRATE with IRQ14; absent slave and absent secondary channel answer cleanly | READ: commands ECh, 20h/21h/30h/31h, C4h/C5h, C6h, 91h, 10h-1Fh, 40h/41h, 70h-7Fh, EFh, 90h; everything else ABRT (`ata.rs:640-778`). IDENTIFY words 0-6, 47, 49 (LBA, IORDY, no DMA), 53-61, 64, 67-68 (`ata.rs:780-826`). Data port accepts 16-bit cycles only (`ata.rs:83`). Power commands E0h-E6h and FLUSH E7h abort. ASSUMED sufficient; this is the first real driver to use it. If the driver declines, Windows falls back to MS-DOS compatibility mode through INT 13h in V86: slower, still boots | U | S-M, fix as found | Desktop only if it hangs |
| 6 | ATAPI CD-ROM | Not needed on path C | READ: absent (`ata.rs:45`); firmware side ready | U | L | Later |
| 7 | PS/2 mouse | 8042 aux port, IRQ12, standard ID 00 mouse | READ: device and commands exist (`ps2_mouse.rs:11-19`); `board.mouse` is accepted on the generic boards (`validate.rs:744-776`); SeaBIOS sets the equipment bit and serves INT 15h C2h (`mouse.c:17-23`, `:255`). No generic-board test uses it. Sample rate does not pace packets (`ps2_mouse.rs:27-30`) | U | S | Later (setup works from the keyboard) |
| 8 | Serial mouse | Alternative to PS/2 | READ: exists (`assemble.rs:367-372`), but COM1 is SeaBIOS's serial console on these boards (`board.rs:222-229`, `fw_cfg.rs:28`), so the two collide. Prefer PS/2 | U | - | - |
| 9 | Keyboard | 8042 command byte, A20, LEDs, typematic | READ: table at `i8042.rs:113-131`; D2h and E0h not modelled. ASSUMED enough | U | S if found | - |
| 10 | Memory size | INT 15h 88h, E801h, E820h | READ: all present (`system.c:170`, `:260`, `:290`); RAM ceiling 64 MiB (`memory.rs:13-17`). Use 16-32 MiB | - | - | - |
| 11 | APM | Connect, CPU idle, power off | READ: on by default (`Kconfig:401`); real, 16-bit and 32-bit interfaces (`apm.c:19-64`); power-off without ACPI is `cli; hlt` (`apm.c:89-100`), which stops the machine (`crates/machine/src/lib.rs:2553`). That is a clean end-of-run signal for tests | U | S (probe) | - |
| 12 | PnP BIOS | Optional | READ: `$PnP` header present, every call returns "not supported" (`pnpbios.c:36-62`, `Kconfig:406`). ASSUMED: Device Manager shows a failing "Plug and Play BIOS" node, cosmetic. Turning `CONFIG_PNPBIOS` off means a firmware rebuild, re-pin and golden re-run | S | S | Later, optional |
| 13 | IRQ routing, PCI | None on ISA | READ: PCI BIOS, PIR, ACPI, MP tables off (`vendor/seabios/config:17-33`). Undecoded ports float to FFh (`crates/bus/src/lib.rs:99`), which is what legacy and ISA PnP detection expects | - | - | - |
| 14 | CMOS/RTC | Time, alarm, periodic, 128 bytes | READ: 128-byte RTC with the QEMU layout (`board.rs:208-210`, `assemble.rs:1811-1860`); periodic partial, alarm and update-ended modelled (`cmos_rtc.rs:64-66`) | - | - | - |
| 15 | VGA 640x480x16 | Planar writes, write modes 0-3, latches, bit mask, set/reset, read mode 1, full register readback | READ: all four write modes, rotate, set/reset, logic, bit/map masks, read-map select, colour compare (`vga.rs:23`); planar 16-colour and 256-colour scanout (`vga.rs:25`); the attribute-controller policy Windows 3.x `vga.drv` needs (`vga.rs:18`). Not modelled: RAM-enable gating, IRQ, mid-frame changes (`vga.rs:13`, `:25`). SeaVGABIOS is plain VGA without VBE (`vendor/seabios/vgaconfig`). ASSUMED: enough for the standard VGA driver and the 320x400 boot logo; the display VxD's save and restore of the whole register set is the untested part | U | S-M, fix as found | Setup (GUI is mode 12h) |
| 16 | More colours or resolution | VBE or an SVGA card | Phase C, upc-18x.2 | U | L | Out of scope |
| 17 | PIC | Whatever VPICD programs | READ: special mask mode, poll mode, level trigger and rotation are not modelled (`pic8259.rs:85-94`). ASSUMED: VPICD uses ICW re-init, masks, EOI and ISR/IRR reads only. A port trace settles it | U | M if used | Desktop if used |
| 18 | PIT, DMA | VTD and VDMAD program them directly | Present; SB16 DMA goldens pass (`status.md:3-17`). ASSUMED fine | - | - | - |
| 19 | Floppy driver (HSFLOP.PDR) | Controller-type probe | READ: unknown commands answer 80h like a plain uPD765 (`upd765.rs:1041`). ASSUMED: treated as a basic controller | U | S if found | Later |
| 20 | SB16 driver | DSP version, mixer IRQ/DMA readback, playback | READ: DSP 4.05, mixer 80h/81h readback (`sb16.rs:44`), OPL3. MIDI (MPU-401) and input are not modelled (`sb16.rs:23`). ASSUMED: playback works; the MPU-401 child device may fail to start | U | S playback, M MPU-401 | Later |
| 21 | Numeric errors | ASSUMED: Windows 95 may keep CR0.NE = 0 and expect IRQ13 | READ: external routing refused on both 486 entries (`protected-mode-gaps.md:465`); upc-18x.11 and xj-31k.4 (x86jit) already cover it. Unmasked x87 exceptions are rare at the desktop | X then U | M | Later |
| 22 | CPU, functional | V86 under paging, 32-bit gates, I/O bitmap, ARPL as #UD in V86, ring transitions, demand paging, CR3 reloads, INVLPG | READ at the pin: all shipped (x86jit `deferred.md:261-278`, `design/task-switches.md:64-68`, `design/v86.md:131`; ours `protected-mode-gaps.md:462-468`). Remaining typed refusals: V86 16-bit gates, AM+AC, DR7 with enable bits or GD (x86jit `interp/legacy.rs:4117-4128`), TR6 writes (`:4133-4141`), TR3-TR5, 16-bit BSWAP, page tables on device or ROM, the TSS T bit. ASSUMED: Windows 95 reaches none without a debugger | X | unknown | Setup if hit |
| 23 | CPU, speed | Paged execution in all three regimes | Section 3. Paged protected16 CPL 3 and paged V86 have no benchmark here | X (bench: U) | L | Practical use |
| 24 | CPUID | Either 486 entry | READ: the CPUID entry reports FPU only (x86jit `model.rs:401`), so no VME is advertised. Start with `80486dx` | - | - | - |
| 25 | Snapshots | A checkpoint after setup would save hours | Deferred (`deferred.md:69`). Disk journal generations at reboots replace it for now | U | L | Not proposed |

Notes on rows where Windows 95 behaviour drives the risk (all ASSUMED):

- Row 5. The driver compares the BIOS geometry with IDENTIFY words 1/3/6 and
  54-58 to decide whether it can take the disk over. A disk within 1024
  cylinders removes that question from the first run.
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

Open, project-owned probes that can cover the same gaps in CI:

| Gap rows | Probe |
| --- | --- |
| 2, 4 | SeaBIOS boot with a 1015/16/63 disk and one above 1024 cylinders: INT 13h AH=08h, 41h, 48h and a read at the last cylinder |
| 5 | A driver-style ATA sequence from protected mode with IRQ14: IDENTIFY, SET MULTIPLE, READ/WRITE MULTIPLE, INITIALIZE PARAMETERS, absent slave, absent secondary channel, A1h abort |
| 7 | FreeDOS with the open CuteMouse driver in PS/2 mode and injected motion; a direct 8042 aux exchange with IRQ12 |
| 11 | APM install check, 32-bit connect, CPU idle, power off ending the run |
| 15 | Mode 12h write modes, latch copies, read mode 1 and an INT 10h AH=1Ch save/restore round trip through SeaVGABIOS |
| 17 | Extend the PIC unit tests only if the port trace shows special mask or poll mode |
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
