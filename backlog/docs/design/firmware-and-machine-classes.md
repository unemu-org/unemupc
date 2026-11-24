# Firmware, board models and machine classes

Spike for bead `upc-6le.2` (epic `upc-6le`, phase A′ of decision-16), written
2026-11-10 at `main` 99a669e. **Design proposal; implementation notes in §9.** It answers:
which machines a description can select, which open firmware runs each, and
which devices each needs.

Related: [`machine-config.md`](machine-config.md) (the `MachineConfig` this
layer lowers to), decision-12 (Kiselev BIOS choice), decision-8 (A20),
decision-16 and its 2026-11-10 amendments (phases A′ to D), the description-file
spike `upc-6le.1` and the persistent-media spike `upc-6le.3`.

Evidence labels:

- **MEASURED**: observed in this repository, or computed on a fetched file.
- **READ**: read in source code or a document; the source is cited.
- **ASSUMED**: not checked in this spike. Check it before relying on it.

## 1. Context

At the initial survey there was one firmware machine (READ
`crates/assembly/src/presets.rs`, `bios()`). Current media-attached `ibm-5170`
presets use an I286 machine-profile CPU with the 24-bit board memory map and
the following parts; the historical no-controller `bios()` rig remains
Bare/Application Real16:

- a cascaded 8259 pair, an 8253, an 8042, an MC146818 with seeded CMOS, port
  61h, a POST latch at 80h and a COM1 16550;
- MDA and VGA text cards;
- one 8237 with page latches 81h–8Fh, and a µPD765 at 3F0h–3F7h on IRQ6 and
  DMA channel 2;
- Sergey Kiselev's `8088_bios`, built for the Xi 8088 board, as a 32 KiB ROM at
  F8000h.

MEASURED at CPU `e63cee1` and confirmed at `8bad85d`: the I286 TF probe returns AL=1, including a post-POP DS trap, so Kiselev labels it `Intel 8088 '78` and emits its segment-load warning. At the current pin, real-mode CPU
exceptions also enter the IVT, allowing the synthetic floppy boot sector's
CS-limit fault to reach the BIOS INT 13 handler. **MEASURED at CPU `fd67927`
with locally supplied firmware:** the IBM 5170 BIOS 1985-06-10 passes POST
cleanly and boots a 1.2 MB FreeDOS floppy to the DOS prompt. The measurement
uses MDA with the automatic display switch, an `at-adapter` fixed-disk controller,
derived volatile CMOS and a pinned
RTC. Its regression test accepts FreeCOM's date/time with Enter and polls for
the DOS prompt, recording only verdicts, checkpoints, counts, stop location and
the final MDA-plane hash. IBM regression expectations pin this measurement;
the Landmark AT full-pass regression at CPU `060d446` is also pinned. See
[compatibility diagnostics](compatibility-diagnostics.md#3-golden-capture-rule)
§3 for its media-equipped configuration and measured values. Other firmware
revisions remain unqualified.

Current memory-map policy: `ibm-5170` gets a 16 MiB VM for its fixed 24-bit
decode, with validation of CPU width and mapping bounds. Configured RAM,
ROM and MMIO retain their ranges; adapter gaps keep their historical zeroes.
All remaining holes read FFh and discard guest writes (**ASSUMED** floating
value), including above configured extended RAM and between the end of
conventional RAM and A0000h when less than 640 KiB is installed. **READ:** IBM Personal Computer AT
Technical Reference, September 1985, Section 1 "System Board", System Memory
Map (p. 1-8). The board's FE0000h–FFFFFFh decode is a read-only copy of the
resolved E0000h–FFFFFh bytes, including adapter-gap zeroes. One assembly
function owns this alias independently of CPU reset. Bare rigs retain their old
map and VM size. See [MemoryConfig](machine-config.md#memoryconfig--a-semantic-memory-map).

FreeDOS boots from floppy and runs `DIR` (MEASURED: status.md, slow goldens).
Firmware is hard-wired into the preset. The BIOS has no fixed-disk INT 13h, and
FreeDOS reports no hard disks.

The maintainer agreed a first shape in the `upc-6le.1` notes:

- `[firmware]` with `provides` and `option_roms`;
- an `ide` card that is rejected unless something provides `fixed-disk`;
- `class = "at" | "xt"`;
- `cpu.model = "8088" | "8086" | "v20"` reserved until x86jit models them
  (`xj-50w (x86jit)`, P4).

**A later scope change replaces `class` with a named board model.** The
description says `[board] model = "ibm-5170"`, the way 86Box and PCem name
machines, and the class is derived from the model. Real BIOSes are built per
board or chipset. They program chipset registers and expect chipset-integrated
peripherals. So firmware compatibility is a property of the board, not of an
abstract class.

The forces on the design:

- **Goldens.** Every headless golden pins a firmware image (`fetch.sh` checks
  SHA-256), a CPU revision and a run cadence. Changing the default BIOS changes
  every golden.
- **Phase B** needs A20 control and INT 15h AT services (87h, 88h, 89h). The
  Kiselev BIOS has neither (READ below), so a second firmware is needed anyway.
- **Hard disk.** A hard disk in phase A′ needs a fixed-disk INT 13h provider.
- **XT.** An XT-class machine needs an 8088 CPU model that x86jit does not have
  yet.

## 2. Firmware survey table

"AT services" means INT 15h 87h (block move), 88h (extended memory size), 89h
(enter protected mode), A20 control, and use of AT CMOS.

| Firmware (version, date) | License | CPU min | Board / class | AT services | Fixed disk | Video | Boots FreeDOS | Build | Source |
|---|---|---|---|---|---|---|---|---|---|
| Kiselev `8088_bios`, Xi 8088 config (1.0.2, BIOS date 03/27/26; commit `a4af458`, 2026-07-16) | GPL-3.0-or-later | 8088; runs on our 286 | Xi 8088: AT-like, with 2×8259, 8042, MC146818, **one** 8237 | **None.** INT 15h has 4Fh, 90h/91h, C0h and C2h only; no A20 code; reads AT CMOS and checksums it | None. INT 13h serves DL ≤ 1; IPL falls back to INT 13h DL=80h | MDA/CGA INT 10h built in; EGA/VGA through a ROM at C000h | **Yes** (MEASURED, goldens) | NASM (open) | [skiselev/8088_bios](https://github.com/skiselev/8088_bios) |
| Kiselev `8088_bios`, XT configs (`MACHINE_XT` 16 KiB, Book8088, FE2010A) | GPL-3.0-or-later | 8088 / V20 | XT: 8255 PPI, one 8259, DIP switches | None | None | MDA/CGA, chosen by switches | Not verified | NASM | same; `config.inc` says `MACHINE_XT` is "FIXME: not implemented yet" and the README says "not tested" (READ) |
| GLaBIOS (`VER_NUM 0.4.3` at commit `68c65a5`, 2026-07-23; the README also lists a newer beta) | GPL-3.0-or-later | 8088 (V20 builds need a V20) | XT presets: 5150, 5160, Turbo, several chipsets, `ARCH_EMU` | None. INT 15h is cassette-only on 5150, else AH=86h/CF. No RTC; RTC comes from the GLaTICK option ROM | None. INT 13h is floppy only (READ) | MDA/CGA routines; EGA/VGA through its ROM | Upstream lists no OS tests (ASSUMED yes) | MASM 5 + LINK (proprietary); releases ship ROMs | [640-KB/GLaBIOS](https://github.com/640-KB/GLaBIOS) |
| Super PC/Turbo XT BIOS 3.1 (2017-10-28) | Public domain (bundled XUB is GPL-2, floppy BIOS GPL-3) | 8088 / V20 | XT / 5150 | None. RTC is detected but not served | Needs a controller ROM (bundles XUB) | Tests MDA/Hercules/CGA memory; colour boot screen | Not stated | Open Watcom | [readme](https://www.retrocmp.de/software/pcxtbios/readme.txt) |
| 8086tiny BIOS | ASSUMED MIT | 8086 | Emulator-specific | None | Emulator traps | — | — | — | Rejected in decision-12: it needs emulator-only opcodes |
| **XTIDE Universal BIOS (XUB)** r638 (builds dated 2026-06-09) | GPL-2.0-or-later (source headers; READ) | `ide_xt` 8088, `ide_xtp` 186/V20, `ide_at`/`ide_atl` **286**, `ide_386` 386 | Option ROM, any board | — | **Provides fixed-disk INT 13h** (see §2.2) | Prints through INT 10h | ASSUMED yes | NASM + make (open) | [xtideuniversalbios.org](https://www.xtideuniversalbios.org/), [binaries r638](https://www.xtideuniversalbios.org/binaries/r638/) |
| Bochs `rombios` (`BIOS-bochs-legacy`) (master, © 2001–2026) | LGPL-2.1-or-later | **386** (`#error Only 386+ cpu supported`) | AT / Bochs PCI (i430FX, i440FX, i440BX host IDs) | **Yes**: INT 15h 24h, 87h, 88h, 89h; A20 through **port 92h** | Built-in ATA/ATAPI driver, El Torito | **None.** Needs a VGA ROM at C000h; PCI-BIOS builds halt without one | ASSUMED yes | bcc/as86 (dev86) | [bochs/bios/rombios.c](https://github.com/bochs-emu/Bochs/blob/master/bochs/bios/rombios.c) |
| SeaBIOS 1.17.0 (2025-06-10), QEMU `isapc` config (128 KiB, `PCIBIOS=n`, `USB=n`, `ACPI=n`) | LGPL-3.0-only | **386** (32-bit flat POST) | ISA-only 486-class PC (QEMU `isapc`) | Yes (READ: §3.6), including E801h/E820h; RAM size from CMOS without fw_cfg | Built-in ATA PIO, floppy | SeaVGABIOS through fw_cfg, or SERCON configured by fw_cfg (§3.6) | ASSUMED yes | gcc + binutils | [seabios.org](https://www.seabios.org/), QEMU `roms/config.seabios-128k` |
| SeaBIOS 1.17.0, QEMU `pc` config | LGPL-3.0-only | 386 | i440FX + PIIX3/PIIX4, Q35 | Yes, including E801h/E820h; also PCI BIOS and ACPI | ATA, AHCI, USB, … | SeaVGABIOS: std VGA, Cirrus, Bochs VBE | ASSUMED yes | gcc | as above |
| SeaVGABIOS / LGPL VGABIOS (video option ROMs) | LGPL-3.0 / LGPL-2.1 | ASSUMED 386 | Video option ROM | — | — | VGA; VBE with Bochs VBE | — | gcc / bcc | SeaBIOS `vgasrc/`, Bochs |
| unemu-at-bios, the unemu fork of Kiselev `8088_bios` (`v1.0.2-unemu.3`, `MACHINE_UNEMU_AT`) | GPL-3.0-or-later | **286** (block move and memory sizing in protected mode) | IBM 5170-class: 2×8259, **two** 8237s, 8042, MC146818 | **Yes**: INT 15h 87h, 88h; CMOS shutdown-byte resume (04h, 05h, 09h, 0Ah); extended memory measured into CMOS 30h–31h, and the memory found above 1 MiB cleared on every POST; A20 through the 8042, off after POST; IBM CMOS checks (161–164) | None; through an option ROM (XUB) as Kiselev | As Kiselev; display switch read with the IBM polarity | **Yes** (MEASURED, goldens) | NASM via Nix | <https://github.com/unemu-org/at-bios> (renamed from `unemu-at-bios`); `vendor/unemu-at-bios/README.md` |
| LGPL VGABIOS, unemu fork `v0.9d-unemu.1` (std build, upstream 0.9d; teletype BEL sounds the speaker) | LGPL-2.1-or-later | **286** (fork's 286 check) | Video option ROM, 32 KiB at C0000h | — | — | Standard VGA, no VBE | **Yes** with the unemu AT BIOS (MEASURED, goldens; §Video ROM placement); the 0.8a-based ROM was also measured with the Kiselev AT-reset BIOS | bcc/as86 (dev86) via Nix | <https://github.com/unemu-org/vgabios>; `vendor/vgabios/README.md` |
| fysnet i440fx BIOS 1.0 (2025-02-10) | **Freeware, not open source** | — | i440FX / 440BX | — | — | — | — | NBASM | [fysnet.github.io/i440fx](https://fysnet.github.io/i440fx); excluded on license grounds |
| IBM 5150/5160/5170 ROMs; AMI/Award/Phoenix board BIOSes | Proprietary | per board | per board | per board | per board | per board | per board | — | **User-supplied only.** Hashes are listed, the images are never shipped |

**No open BIOS was found that targets a real 286 AT and provides INT 15h
87h/89h** (READ: searches turned up none; ASSUMED none exists). Both open
AT-service BIOSes, Bochs `rombios` and SeaBIOS, need a 386. Phase B therefore
moves to a 386 board for firmware reasons as well as CPU reasons. The
project's own fork of the Kiselev BIOS, unemu-at-bios (§2.4), has since
added INT 15h 87h and 88h and the shutdown-byte resume for the 286 AT; it
has no 89h.

### 2.1 Kiselev Xi 8088 configuration, in detail (READ: pinned clone, `src/`)

- **Settings.** `MACHINE_XI8088` sets `AT_RTC`, `AT_RTC_NVRAM`, `AT_NMI` (NMI
  mask at 70h), `AT_DELAY` (61h refresh bit), `SECOND_PIC`, `AT_KEYBOARD`,
  `PS2_MOUSE` and model byte FCh. `MAX_RAM_SIZE` is 640 and `EBDA_SIZE` 1
  (`config.inc`).
- **Display switch.** This BIOS reads the AT display switch with inverted
  polarity; in the colour position it selects the monochrome adapter
  (`at_kbc.inc:468–473`). IBM Personal Computer AT Technical Reference, System
  Board, Keyboard Controller, "Input-Port Bit Definitions", specifies bit 6
  set for monochrome. The model keeps that IBM polarity. Kiselev profiles set
  `board.display_switch = "colour"` (A0h input), so the firmware uses MDA.
  The switch still names the IBM primary: derived CMOS 14h video bits are 10b,
  colour, rather than the firmware-selected mono console. Both pinned BIOS
  variants use this explicit setting in `presets::bios()`, and
  `machines/at-freedos.toml`, its YAML twin and `at-freedos-from-preset.toml`
  set it. The open AT BIOS (§2.4) reads the switch with the IBM polarity, so
  `at-ide.toml`, which boots it on the same dual-card board, sets `mono` to
  keep the MDA console. The Rust helper matches the ROM hash, including for the
  historical Bare POST rig; other firmware and the firmware-neutral board
  preset retain Auto. See [primary display](machine-config.md#primary-display).
- **Option-ROM scan.** It scans for a video BIOS from C000h to C800h, then for
  extensions from C800h to F800h (to F000h when a setup flag is set). The step is
  2 KiB. A ROM must have a 55AA header and a size byte, and its checksum must be
  zero. The scan skips zero-sized headers (`bios.asm` `extension_scan`,
  `detect_rom_ext`).
- **IPL.** INT 19h tries floppy 0 four times, then `INT 13h AH=0Dh/02h DL=80h`
  (`bios.asm` `ipl`). An option ROM that hooks INT 13h (or INT 19h) can
  therefore supply the hard disk with no BIOS change.
- **INT 15h.** It handles only 4Fh, 9001h, 90FDh, 9101h, C0h and C2h. Everything
  else returns AH=86h with CF set (`misc.inc`). There is no 87h, 88h, 89h or
  24xxh.
- **CPU detection.** It is written for 8088, 80C88 and V20 (`cpu.inc` header).
  On our 286 it prints an 8088 variant (ASSUMED; cosmetic).

### 2.2 XUB, in detail (READ: SVN trunk sources; MEASURED: r638 `ide_at.bin`)

- **Installation.** The ROM-scan entry `Initialize_FromMainBiosRomSearch`
  installs an **INT 19h boot loader** and returns. Drives are detected when INT
  19h runs (late initialization). `Interrupts.asm` then saves the old INT 13h in
  `RAMVARS.fpOldI13h` and hooks INT 13h, chaining requests it does not own
  (floppies). If no handler is present, it copies the old INT 13h to INT 40h.
- **Defaults of the AT build** (`Main.asm`, `USE_AT`):
  - full mode, which steals **1 KiB** of conventional memory;
  - two controllers: primary 16-bit ATA at **1F0h** (control block at 3F6h)
    and secondary at 170h;
  - IRQ **14/15** when `MODULE_IRQ` is built in.

  So the stock AT build matches a standard AT IDE channel with no configuration
  step.
- **Transfers.** `IdePioBlock.asm` uses **`rep insw`/`rep outsw`** when
  `USE_186`, which every AT build sets. The 8088 build uses unrolled `in ax,dx`
  loops instead.
- **MEASURED: the r638 `ide_at.bin` is 6,646 bytes.** It has the 55AA header,
  a size byte of 16 (8 KiB) and a non-zero checksum (byte sum B0h). **It is not
  a valid option ROM as downloaded.** The upstream flow pads it to 8 KiB and
  fixes the checksum through `XTIDECFG.COM`. A fixture script must do the same
  padding deterministically and pin both hashes. Raw hash:
  `6d5007bca3be0bffeb210969a299b67199368b4845f554b5a00715e2718dfea8`.

**Configured XT+ alternative (READ: r638 source; MEASURED: FreeDOS golden).**
The XT+ build uses 80186 instructions without `USE_AT` or `MODULE_IRQ`.
Its `FloppyDrive_IsInt40hInstalled` checks the vector segment without calling
it; initialization copies the old INT 13h handler into INT 40h when absent.
This avoids the AT build's assumption that the system BIOS already installed
INT 40h. The configured fixture selects `DEVICE_16BIT_ATA`, command base
1F0h, control-block base 3F0h (register 6 at 3F6h), and keeps polling.
It also selects boot drive 00h because the HDD fixture is not bootable.
All other ROMVARS defaults remain unchanged. The recipe retains the AT output
for AT system BIOSes. Exact source references, offsets, bytes and hashes are
in [`vendor/xtide-universal-bios/README.md`](../../../vendor/xtide-universal-bios/README.md).
The new profile uses a separate Kiselev compatibility image: its minimal
`src/floppy2.inc` patch dispatches AH=00h before DL validation; AH=01h already
bypasses it. This mirrors the IBM PC AT Technical Reference BIOS listing,
`DSKETTE.ASM` (`OK_FUNC` / `OK_DRV`, lines 236–237): reset/status accept any
drive number. IBM `DISK.ASM:501–508` and XUB
`Src/Handlers/Int13h/AH0h_HReset.asm:39–44` forward DL=80h reset requests to
the floppy handler. Upstream Kiselev rejects that request before resetting
and sensing the FDC. The patch is GPL-3.0-or-later and changes no CMOS disk
handling. Both BIOS outputs, built from the same pinned commit, have separate
catalogue hashes; existing machines retain the original. See
[`vendor/kiselev-bios/README.md`](../../../vendor/kiselev-bios/README.md).
The resulting floppy boot and `DIR C:` are qualified in §9 item 11.

### 2.3 Bochs `rombios` and SeaBIOS: requirements (READ)

- **Bochs `rombios`.**
  - `rombios.h` sets `BX_CPU 3`, `BX_PCIBIOS 1`, `BX_APM 1`, `BX_USE_ATADRV 1`
    and `BX_ELTORITO_BOOT 1`. `LEGACY` only removes `rombios32`.
  - `PORT_A20` is `0x0092`.
  - It has no INT 10h of its own (`int10_handler: iret`). It scans C000h for a
    VGA ROM, and in PCI-BIOS builds it halts if there is no 55AA there.
  - It scans option ROMs from C800h to E000h.
- **SeaBIOS.**
  - POST runs in 32-bit flat mode (`docs/Memory_Model.md`).
  - Shadowing unlocks the F-segment through **i440FX (82441) or Q35 PAM**
    registers (`src/fw/shadow.c`). Without a bridge it logs "Unable to unlock
    ram". QEMU worked around this for `isapc` with a writable BIOS region, and
    the board model must do the same (READ: pinned QEMU sources, §3.6).
  - Its PCI init knows PIIX3 (82371SB) and PIIX4 (82371AB) ISA bridges, PIIX
    IDE, PIIX4 PM and ICH9 (`src/fw/pciinit.c`).
  - QEMU detection reads PCI 00:00.0 and the fw_cfg signature from data
    port 511h after selection through 510h (`src/fw/paravirt.c:671–674`).
    Both reads return open bus on a plain ISA board, so RAM sizing falls
    back to CMOS (`src/fw/paravirt.c`).

### 2.4 Open AT BIOS candidates

**READ:** SeaBIOS requires a 386; GLaBIOS and the other open BIOS candidates
surveyed here are XT-class. Kiselev targets the Xi8088 board, with an inverted
display switch relative to the IBM AT. The maintainer's decision (2026-11-12)
is to retain Kiselev with an explicit display switch for open-firmware goldens;
AT-faithful firmware behaviour is measured through the user-firmware lane.

**MEASURED:** legacy Bochs `rombios` at `6f8f69e5bb7a`, the last revision
with `BX_CPU < 3` (removed as broken in `ddf4df1dc`), emits 97 instructions
illegal for a 286. POST loops at F000:6107 (`xor eax,eax` in
`timer_tick_post`); MDA/colour, floppy and AT XTIDE + IDE probes never boot
FreeDOS. **READ:** it has no INT 10h implementation, assumes port-92h A20,
and retains 386-only INT 15h AH=87h and disk paths. Reject it for `ibm-5170`.

**ASSUMED:** porting would take about 2–4 weeks for runtime/timers, disk/video paths,
8042 A20, protected-mode services, CMOS and boot qualification; see [deferred work](../deferred.md#open-at-class-80286-system-bios).

**Outcome (2026-11-15): unemu-at-bios.** Instead of a port, the project forked
the Kiselev BIOS (<https://github.com/unemu-org/at-bios>, renamed from
`unemu-at-bios`, a GitHub fork of `skiselev/8088_bios`; GPL-3.0-or-later; Sergey
Kiselev's history kept, the unemu changes behind defines) and added a
`MACHINE_UNEMU_AT` configuration for this board. Release `v1.0.2-unemu.3`
(revision `4548c176`, ROM SHA-256 `6938dc8b…ef2442`, catalogue id
`unemu-at-bios-1.0.2-unemu.3`, **Tested**; `v1.0.2-unemu.2` stays catalogued
as **Superseded** with its own source archive pin, so machines made with it
still start) is the system BIOS of `machines/at-games-open.toml`,
`machines/at-ide.toml` and the `open-at` templates;
`vendor/unemu-at-bios/fetch.sh` builds it with Nix. READ (the fork's
`README.unemu.md`): next to the Xi 8088 settings it resumes a reset through
CMOS 0Fh (05h and 0Ah through 0040:0067h, 09h for the block move and the POST
memory sizing, 04h through POST), answers INT 15h AH=87h and AH=88h as the IBM
listing does, measures extended memory into CMOS 30h–31h in protected mode
(one 8042 CPU reset per POST) and clears every block it finds above 1 MiB on
every POST (since `v1.0.2-unemu.3`; `v1.0.2-unemu.2` kept the contents, so the
VDISK header a DOS=HIGH FreeDOS kernel leaves at 1 MiB survived Ctrl+Alt+Del
and FDXMS286 refused to load), checks the CMOS as the IBM POST does (161–164,
checksum over 10h–2Dh, then the F1/F2 prompt), disables A20 after the 8042
initialization, uses the IBM polarity for the display switch and the port 70h
NMI mask, initializes the second 8237 (scratch port 8Eh instead of C0h) and
identifies the 80286 and an 80287. The ROM scans, the floppy-only INT 13h and
the IPL are the Xi 8088 build's (§2.1). The Kiselev Xi 8088 entries stay
catalogued: the unmodified image still boots `machines/at-freedos.toml` and
the BIOS POST and FreeDOS goldens.

## 3. Board models

### 3.1 What a board model is

A board model is a named, versioned piece of data owned by the `assembly` crate.
The description file selects it with `[board] model = "…"`. It lowers to the
existing flat `MachineConfig`, so the assembler, validator and goldens keep their
contract (machine-config.md). A board model has these parts:

1. **Identity.** An `id`, a display name, the derived **class** (`xt`, `at`,
   `at386`, `pci`), and the reference document (the Technical Reference or
   chipset datasheet, by name and section).
2. **CPU sockets.** The allowed `cpu.model` values and an optional FPU socket. A
   value that is allowed but not yet modelled stays **reserved**.
3. **Motherboard device set and standard wiring.** A `Vec<DeviceConfig>` with
   ports, IRQs, DMA and access couplings, plus the memory map: ROM window, RAM
   holes and shadow regions. This includes chipset-integrated peripherals.
   - For example, the C&T 82C206 integrated peripheral controller of the NEAT
     chipset combines the 8259s, 8237s, 8254 and RTC (ASSUMED part mapping).
   - Such a chip reuses the existing device kinds at their standard ports. It
     is a composition, not a new device.
4. **Configuration store.** Either CMOS NVRAM in a layout derived from the
   description (AT), or DIP switches derived from the description (XT). Neither
   is free-configured, as with CMOS today.
5. **Chipset register model (optional).** Registers that real firmware programs:
   - shadow RAM and PAM;
   - cache control;
   - turbo;
   - fast A20 at port 92h;
   - PCI configuration space;
   - PIRQ routing;
   - ELCR.

   `ibm-5150`, `ibm-5160` and `ibm-5170` have none.
6. **Card bus.** The slot width (ISA-8, ISA-16, PCI) and the IRQ and DMA lines
   left free for cards.
7. **Firmware catalogue.** A list of `FirmwareEntry` values:

   | Field | Meaning |
   |---|---|
   | `id` | Stable name, for example `kiselev-xi8088-1.0.2` |
   | `kind` | `system-bios`, `option-rom` or `video-bios` |
   | `sha256` | Assembled image identity, checked after interleaving any split parts |
   | `parts` | Alternative known chip layouts: label, part number, lane, offset, size and SHA-256; empty for open entries |
   | `size`, `base` | Load rule: a system BIOS is top-aligned under 1 MiB, an option ROM has a suggested base |
   | `provides` | Services the image supplies to the guest (`floppy`, `fixed-disk`, `video`, `at-services`, `pci-bios`, `vbe`) |
   | `requires` | Parts the image needs (`kbc-8042`, `ppi-8255`, `rtc-cmos`, `pic-cascade`, `video-rom`, `port-92h`, `ata@1F0/14`) |
   | `cpu_min` | Minimum CPU model |
   | `distribution` | `open` (a fetch script under `vendor/`, with license and provenance) or `user-supplied` (hash only, never shipped) |
   | `status` | `tested` (a golden runs it) or `listed` |

8. **Default firmware.** The open, tested entry, when there is one.

### 3.2 Board catalogue (proposed)

| `model` | Class | CPU sockets | Chipset registers | Open firmware | User-supplied examples | Status |
|---|---|---|---|---|---|---|
| `ibm-5170` | at | 80286 (80287 later) | none | **Kiselev Xi 8088 1.0.2** (tested; SHA-256 `1ad458ac…81a8`, MEASURED by `fetch.sh`), plus XUB AT r638 option ROM | IBM 5170 BIOS (even/odd pair) | **available now** |
| `ibm-5160` | xt | 8088, v20 (reserved) | none | GLaBIOS `ARCH_5160`, Kiselev `MACHINE_XT`, Super PC/Turbo XT | IBM 5160 BIOS | reserved until `xj-50w.2 (x86jit)` |
| `ibm-5150` | xt | 8088 (reserved) | none | GLaBIOS `ARCH_5150` | IBM 5150 BIOS | optional, later |
| `generic-386-isa` | at386 | 80386DX | port 92h; emulator-only fw_cfg at 510h/511h | The unemu SeaBIOS fork, `CONFIG_QEMU`, serial console first; SeaVGABIOS later; Bochs `rombios` legacy fallback (decision-19, §3.6) | — | board available; diskless POST, INT 15h memory queries and keyboard reset/resume pass at `b7c10b6f` |
| `i440fx-piix3` | pci | Pentium Pro / Pentium II (reserved) | 82441FX PAM/SMRAM, PIIX3 PIRQ/ELCR | SeaBIOS (QEMU `pc` config); Bochs `rombios` | — | candidate for phases C/D |
| `i440bx-piix4` | pci + AGP | Pentium II/III | 82443BX, PIIX4 | none for a real board (see §3.4) | Award/AMI/Phoenix images of named boards | enthusiast |
| `neat-286` | at | 286 / 386SX | C&T CS8221 index/data registers (ASSUMED 22h/23h) | none | AMI/Phoenix NEAT BIOSes | enthusiast |
| `i430tx` | pci | Pentium MMX | 82439TX, PIIX4 | none | Award/AMI | enthusiast |

Today's machine is `ibm-5170` with listed deviations (§4.2). Both 8237
controllers and the empty 80287 socket's ports F0h/F1h, F8h–FFh are assembled
(READ `crates/assembly/src/board.rs:99–130`, `assemble.rs:303–305`). The
8042 reset signal restarts the CPU while retaining RAM and devices (READ
`assemble.rs:261–264,338–339`, `crates/machine/src/lib.rs:305–316`). A20
masking is still missing; empty-socket I/O does not establish CPU-side
absent-FPU behavior (§3.6).

### 3.3 Do the Kiselev BIOS and GLaBIOS use only standard parts? (READ)

The **Kiselev Xi 8088 build on `ibm-5170`** mostly uses 5170 parts:

- 2×8259;
- an 8042 (with PS/2 aux probing);
- the MC146818 with the NMI mask at 70h;
- the 61h refresh bit;
- an 8237;
- POST codes at 80h.

It deviates from a 5170 in these places:

- **Display switch polarity is inverted.** The explicit colour position makes
  this BIOS select MDA; CMOS continues to describe the IBM colour primary
  (10b). See §2.1 for the profile settings.
- **Port 61h bit 2 is the Xi 8088 "turbo" bit** (`set_cpu_clk_turbo`). On a
  5170 that bit enables the parity check. Writes are harmless.
- **Port C0h is written as an "unused" register** for I/O delays and bus
  floating in serial and printer detection (`unused_reg equ 0C0h`). On a 5170,
  C0h is the second 8237's channel-4 address register. Those writes are
  harmless (channel 4 is the cascade), but now change the word-bank state
  (READ `crates/assembly/src/assemble.rs:1215–1228`).
- **Only one 8237 is initialized.** The 16-bit channels stay unprogrammed.
- **No extended memory.** The RAM scan stops at 640 KiB and INT 15h 88h is
  absent.
- **PS/2 aux.** It probes the aux port, which a 5170 8042 lacks. Our 8042
  answers like a PS/2 controller; the BIOS tolerates either.

The **Kiselev `MACHINE_XT` build on `ibm-5160`** uses standard 5160 parts:

- 8255 control word 99h;
- DIP switches through PB3 select and port C;
- NMI mask at A0h.

Its POST code at 80h lands on the XT DMA page-register file (ASSUMED harmless).
It is untested upstream.

**GLaBIOS `ARCH_5160`** uses standard 5160 parts. It sets `TURBO_NONE`, takes
the drive count from IPL switch SW1 and supports the expansion unit. Its default
`ARCH_TURBO` preset toggles a non-standard 61h turbo bit, so the catalogue lists
only `ARCH_5160` (or `ARCH_EMU`) builds for `ibm-5160`. `ARCH_EMU` turns off
DRAM refresh and several POST tests for emulators.

### 3.4 i440FX + PIIX3 with SeaBIOS: candidate board for phases C/D

- **SeaBIOS chipsets (READ `pciinit.c`, `shadow.c`).** The supported host
  bridges are i440FX (82441, PAM0 at 59h) and Q35 (MCH). The ISA bridges are
  PIIX3 (82371SB) and PIIX4 (82371AB), plus ICH9 LPC. IDE setup covers PIIX3
  and PIIX4 function 1. Power management is PIIX4 function 3 or ICH9 SMBus.
  There is **no 440BX support**.
- **Devices the board needs** (ASSUMED register details; datasheets: Intel
  82441FX/82442FX 290549-001, May 1996; Intel 82371SB PIIX3):
  - **Host bridge.** 82441FX PMC with configuration mechanism #1 at CF8h/CFCh,
    PAM0–6 shadow control and SMRAM. The 82442FX DBX has no visible registers.
  - **PIIX3 function 0.** The PCI-to-ISA bridge, with PIRQ routing, the top of
    memory and ELCR. It also embeds the AT parts we already model: 2×8259,
    2×8237, 8254, port 61h, NMI and RTC decode.
  - **PIIX3 function 1.** IDE with legacy 1F0h/170h on IRQ14/15; bus-master IDE
    is optional.
  - **PIIX3 function 2.** UHCI USB, optional.
  - **PIIX4 function 3** (ACPI PM timer and SCI), only for ACPI guests. QEMU
    documents PIIX3 as the default south bridge and PIIX4 as an experimental
    alternative (READ, QEMU `i386/pc` docs).
  - **On ISA:** a super-I/O set (8042, FDC, UARTs, LPT).
  - **On PCI:** a VGA card, later with VBE through SeaVGABIOS's Bochs VBE
    variant, and the Voodoo.
- **Windows 98 SE.**
  - **Bochs** documents Windows 98 SE installing and running on its BIOS with
    `pci: chipset=i440fx` and a Cirrus VGA (READ,
    [Bochs guide](https://bochs.sourceforge.io/doc/docbook/user/guest-win98.html)).
  - **QEMU `pc`** with SeaBIOS is reported to work, but with caveats and version
    regressions (READ, secondary):
    - a community guide pins `pc-i440fx-2.0` and recommends patched installers;
    - a 2019 QEMU bug reports failures after 2.7/2.8;
    - a 2012 SeaBIOS list thread reports a Plug and Play BIOS installer issue.

  This is good evidence that the chipset and firmware pair runs Windows 98 SE.
  The rough edges are in the PnP and ACPI tables, which we would control.
- **Period fit.** The 440FX was released in May 1996. It was the Pentium Pro
  chipset and the first to support the Pentium II, with a 66 MHz FSB and **no
  AGP** (READ, Wikipedia/AnandTech by search). A PCI Voodoo or Voodoo2 fits a
  PCI-only board (ASSUMED card bus facts). A Pentium II on 440FX matches
  decision-16's "Pentium II/III-class" target at its low end; Pentium III
  officially needs 440BX or later.
- **440BX + PIIX4, for comparison.**
  - It dates from April 1998, with a 100 MHz FSB and AGP. It is the
    period-perfect Pentium II/III board.
  - Every real 440BX board BIOS (Award, AMI, Phoenix) is proprietary and would
    be user-supplied.
  - Bochs `rombios` recognizes an i440BX host-bridge ID (READ
    `PCI_FIXED_HOST_BRIDGE3`), but only for Bochs's own 440BX emulation, not a
    real board.
  - SeaBIOS would need 440BX PAM support upstream.
  - Verdict: `i440fx-piix3` is the open critical path. `i440bx-piix4` is an
    enthusiast profile on user ROMs.

### 3.5 Enthusiast boards and user-supplied ROMs

86Box and PCem show the demand: NEAT 286/386SX boards (the Arche AMA-2010 is a
NEAT machine in 86Box; READ by search), 430TX and 440BX boards. These boards are
**extras, not the critical path**. Each one needs its chipset register model,
and its firmware is proprietary.

- **Reference.** The description gives an `image` path (relative to the file)
  and optionally `sha256` and `id`. **Identity is the hash, not the file
  name.**
- **Allowlist.** Each user-supplied entry in a board's catalogue carries only
  `sha256`, `size`, `base`, `provides`, `capabilities`, `requires`, `cpu_min`
  and known part layouts. The repository
  holds hashes of known-good dumps, never the bytes.
- **Split ROMs (implemented).** `ImageSource::Interleaved` assembles complete
  even/odd pairs before identity checks. Even byte i lands at offset + 2*i;
  odd byte i at offset + 2*i + 1. Offsets default to zero; pairs must have equal
  nonzero sizes and tile the full image without gaps or overlaps. Only
  `firmware.bios.parts` supports this source; drives, RAM loads and option ROMs reject it.
  The catalogue pins alternative chip layouts and the assembled SHA-256.
- **IBM 5170 provenance (READ).** MAME
  `src/mame/pc/at.cpp`, commit `cabb909b38` (2026-11-03), sets `ibm5170`
  (01/10/84 and 06/10/85) and `ibm5170a` (11/15/85), loads U27 into even
  bytes and U47 into odd bytes with `ROM_SKIP(1)`. CRC32/SHA-1 of all listed
  27256 parts agree with MAME. The four-27128 1984 layout places U17/U37
  at offset 0 and U27/U47 at 32768; assembling these four 27128 dumps
  reproduces the image of the MAME-listed 27256 pair (MEASURED). All three images contain their date at FFFF5h
  and model FCh at FFFFEh (MEASURED).
  IBM PC AT Technical Reference BIOS listing, TEST2.ASM, "CHECK FOR OPTIONAL
  ROM FROM C800->E000 IN 2K BLOCKS" (`MOV DX,0C800H ... CMP DX,0E000H`),
  establishes the exclusive C8000h..E0000h scan window for all three versions.
  DSKETTE and DISK provide floppy and AT fixed-disk services; the entries
  support CMOS drive types and require 8042, RTC/CMOS and cascaded PICs,
  without requiring an ATA channel when no disk is configured.
  The IBM 3270 PC/AT (5281) 11/15/85 set, 62X0820/62X0821, is a different
  machine and is deliberately not listed. See `vendor/user-roms/README.md`
  for every assembled and chip hash.
- **Unknown hash.** The image is rejected with `FirmwareNotListed` (§6), which
  lists the board's known entries and where each comes from. **Resolved by
  decision-18:** `--allow-unlisted-firmware` is a run-only developer escape
  hatch that warns with the full hash. It is never a machine-file field or part
  of golden identity, and it relaxes only allowlist membership.
- **CMOS.** Vendor BIOSes keep extra CMOS bytes and checksums beyond 2Dh. The
  derived standard bytes may trigger a "CMOS checksum" F1 prompt, so these
  boards need the NVRAM-file feature sketched in `upc-6le.1` (§8).

### 3.6 Phase B firmware requirements (SeaBIOS on generic-386-isa)

The `upc-o0j.8` source spike (2026-11-11) uses **SeaBIOS rel-1.17.0,
`b52ca86e094d19b58e2304417787e96b940e39c6`**. All SeaBIOS file:line
references in this section refer to that revision; basenames identify files
under `src/` (including `fw/`, `hw/`), `vgasrc/` or the named build paths.
QEMU file:line citations refer to `29760efaf96bbe5928f8111881eddbf5ac06439e`
(`29760ef`); Bochs citations refer to
`44104eeaff16107e478ea8bd9c55bd8601a80191` (`44104ee`), under `bochs/bios/`.
**READ** means inspected source, **ASSUMED** means unverified; the spike
built and ran nothing. Prebuilt-image measurements are recorded in Sources.
Machine/core availability below is the spike's source assessment at x86jit
pin `8979fc9`, not a SeaBIOS boot result.

**Firmware source (2026-11-24).** The shipped build now pins the source of
truth, [unemu-org/seabios](https://github.com/unemu-org/seabios), at release
`v1.17.0-unemu1`, commit `e38573ec2d7d43fb7802bc43ddcb3ce2bf892809`: upstream rel-1.17.0 plus the
fix set in the fork's `README.unemu.md`. The initial code fix replaces
`__swab32`'s inline `bswapl` with C compiled for the 80386 (`byteorder.h:15–20`).
Upstream's inline instruction requires a 486 despite `-march=i386`; the
fw_cfg directory, ATA and block commands reach it. The old fixture shut down
on #UD at `0008:000F510B`, after 3,582 retired instructions. Fixes belong in
the fork and go upstream (`upc-o0j.23`), rather than in CPU or board workarounds.
Source citations below retain the spike's upstream revision unless marked fork.

**Execution qualification (2026-11-24, MEASURED).** CPU pin `b7c10b6f`
(x86jit) supplies `CpuState::i386_reset`. The adapter applies
the retained board A20 state after installing that image. Synthetic tests
qualify the high reset fetch, near jumps before CS reload, the low target
after reload, the no-FPU CR0.ET state, EDX=0303h and RAM retention across
resets. DH=03h comes from Intel 80386 PRM §10.1; DL=03h remains the core's
assumed stepping. At the bare adapter, A20 off addresses FFEFFFF0h,
deliberately unmapped. The assembled board's explicit reset-fetch policy
temporarily forces CPU A20 on until the first CS reload, then restores the
board gate; see machine-files.md §Generic 386 ISA board.

**MEASURED with the fork on 2026-11-24:** the shipped diskless machine
passes fw_cfg, reports 16 MiB from CMOS and completes POST. COM1 and port
402h print `No bootable device.`. The probe stops after **5,885,053 retired
instructions**.
The probe asserts the banner, fw_cfg, RamSize and all five E820 lines.
After POST, CMOS 0Fh=0 and an 8042 FEh pulse reach
`In resume (status=0)` at **5,887,802 retired instructions**.

A synthetic floppy boot sector queries INT 15h at 16 MiB and 4 MiB.
AH=88h and E801h return 15,360 KiB and 3,072 KiB above 1 MiB, with CF clear.
E801h BX/DX are zero. E820h returns the expected five entries through its
final zero continuation token. The guest clears CMOS 30h/31h before querying;
the results still follow the highest E820 RAM end. See
[machine-files.md §Generic 386 ISA board](machine-files.md#generic-386-isa-board)
for the map, source sections, budgets and guest rebuild command.
All three probes are normal tests and run in a few seconds each.

The old REP NOP stop at `F000:A72C` on `deae5bac` is passed at this pin.
F3 90 and F2 90 execute as NOP on every model, so SeaBIOS `cpu_relax`
needs no CPU option and no board workaround is applied.
No new CPU boundary was observed in that historical diskless probe. The fixture
remains Listed. Guest DOS boot, TSS I/O bitmap, paging and V86 are now covered
separately; the counts above precede the SeaVGABIOS qualification described below.
At x86jit `0bc16f55`, shared generic-386/486 tests load JemmEx 5.85 from the
cached FreeDOS 1.4 source set and run DIR with live VM=1 and monitor deliveries.
See [V86 coverage and remaining limits](protected-mode-gaps.md#v86-monitor-and-cpu-state-audit-2026-11-24).

**Build target and decision (READ).** `CONFIG_QEMU` is the only target with
ISA fallbacks: ATA at 1F0h/170h without PCI (`ata.c:1020–1030`), RAM size
from CMOS (`paravirt.c:240–254`), boot order at 38h/3Dh
(`boot.c:471–489`), floppy types at 10h (`floppy.c:158–163`), ATA
translation at 39h (`block.c:84–90`) and century at 32h (`clock.c:47–48`).
`CONFIG_COREBOOT` needs coreboot tables/CBFS, otherwise assumes 16 MiB
(`coreboot.c:167–215`); it loses the ISA ATA path and takes floppy types
from romfiles. `CONFIG_CSM` returns from `handle_post` immediately
(`post.c:321ff`). Neither is the board's target.

Without fw_cfg, a read of FFh at 511h ends the probe
(`paravirt.c:664–689`). RAM falls back to 16 MiB + CMOS 34h/35h × 64 KiB
when those bytes are nonzero, otherwise 1 MiB + CMOS 30h/31h KiB; bytes
5Bh–5Dh describe RAM above 4 GiB (`paravirt.c:240–254,588–592`). This
fallback does **not** supply a headless console: `etc/sercon-port` is the
only serial-console port source (`sercon.c:519–521`). `vgarom_setup` clears
C0000h (`optionroms.c:467`) and obtains video ROMs only from PCI or
fw_cfg/CBFS romfiles (`optionroms.c:451–489`); extension ROM setup likewise
uses those sources (`optionroms.c:364–386`). INT 10h is a bare `iretw`
without a provider (`romlayout.S`, `entry_10` at ORG F065h). Missing fw_cfg
also loses `bootorder`, boot-menu control (a fixed 2.5 s "Press ESC" wait,
`boot.c:695–711`), the E820 file and SMBIOS/ACPI data; CPU count falls back
to CMOS 5Fh. These are READ, not measured failures.

[Decision-19](../../decisions/decision-19%20-%20Phase%20B%20firmware%20is%20unmodified%20SeaBIOS%20behind%20an%20emulator-only%20fw_cfg%20device.md)
as amended selects the unemu SeaBIOS fork with an emulator-only, board-local
fw_cfg-compatible selector/data port at 510h/511h, without DMA
(`upc-o0j.13`). Its signature, ID, file directory and legacy keys include
zero defaults and key 0Eh = 0 (no boot menu); files are `vgaroms/*`,
`genroms/*`, `etc/sercon-port` and `bootorder`, derived from the machine
description. ROM catalogue placement must distinguish firmware-loaded ROMs
from pre-mapped ROMs. This distinction is implemented for video ROMs: both
generic ISA boards use the normal `firmware.option_roms` source at C0000h,
with catalogue `seavgabios-1.17.0-unemu1-stdvga`. Assembly freezes its resolved
bytes as `vgaroms/vga-c0000.bin` and leaves C0000h empty for SeaBIOS to deploy.
Normal pins, ROM header/checksum, board/CPU/card requirements and catalogue
membership still apply; only the existing run-only escape relaxes membership.
No arbitrary extension-ROM delivery is added.

SeaVGABIOS builds from the same pinned fork export with a separate hashed
`vgaconfig`: standard IBM VGA, no PCI ROM header or VBE. Its 32 KiB image has
SHA-256 `1adc6c0fa72b93c5d0659575c992b1bf6f64fd451c444b2a735e22f0a3d6e925`
and LGPL-3.0-only provenance. The normal probes exercise diskless POST, mode 3
with plane-2 font loading, AH=0Eh text and mode 13h through the actual INT 10h
handler on 386 and both 486 variants. They assert decoded content and indexed
frame hashes; see machine-files.md §SeaVGABIOS delivery and mode-set
qualification. No VGA register additions were needed.

The source notice `No VGA found` describes the PCI search before the fw_cfg
search, even on success. SeaVGABIOS initialization diagnostics use COM1,
because its weak system-BIOS port-debug variable is discarded by the VGA
linker; see vendor/seabios/README.md. The system BIOS stays byte-identical;
neither diagnostic issue is hidden by emulator aliases or source patches.
QEMU `isapc` works because it supplies fw_cfg (`pc.c:892`) and sends
`vgaroms/vgabios.bin` through it (`vga-isa.c:88`, `loader.c:1232–1240`),
not because SeaBIOS scans a plain ISA ROM window (READ).

The fixture recipe in [`vendor/seabios/`](../../../vendor/seabios/README.md)
(`upc-o0j.9`, build half) supplies a 128 KiB build with its own `.config`:
`CONFIG_QEMU`, `SERCON` and `DEBUG_IO` on; `BOOTMENU`, `MPTABLE`,
`PIRTABLE`, `SMBIOS`, `ACPI`, `FW_ROMFILE_LOAD`, `USB`, `PCIBIOS`,
`USE_SMM`, `TCGBIOS` and `S3_RESUME`
off. MP-table generation reads FEE00030h (`mptable.c:57`), which currently
ends core execution on unmapped memory; PIR tables assume PIIX (READ).
The alternative QEMU prebuilt needs an open-bus page at FEE00000h. Building
uses the separate `nix develop .#firmware` shell: i686-elf GCC with `-m16`,
binutils, a native C compiler for Kconfig, Python 3, GNU make, git, tar and
SHA-256 tools. ACPI is off, so IASL is not needed. PCI storage drivers,
ACPI parsing/PM timer, Xen, MTRRs and TSC timing are also disabled.
The script verifies the source tree object, archive and config hashes,
exports the cached commit without changing its checkout, and fixes `.version` plus
`EXTRAVERSION` using upstream's release-tarball mechanism. It rejects a
wrong image size, then a non-fixed build version, then an upstream dirty-build
hostname suffix, before checking output SHA-256.

| Fixture | Build state (2026-11-24) | Catalogue / board |
|---|---|---|
| `vendor/seabios/bios-1.17.0-isa.bin` | Built from the fork; two independent builds matched by `cmp` (MEASURED); output SHA-256 pinned in the fixture | `seabios-1.17.0-unemu1-isa`, Listed, on `generic-386-isa`; diskless POST, INT 15h memory queries and keyboard reset/resume pass at `b7c10b6f` |

The board and catalogue entry assemble and run diskless POST to the boot
failure message; the FreeDOS profiles below additionally qualify floppy boot.
QEMU's prebuilt
`bios.bin` is not used; its MPTABLE and other platform settings deliberately
differ. Input and output hashes are recorded in the fixture README, together
with the 2026-11-24 two-build measurement on macOS arm64 using i686-elf GCC
15.2.0 and binutils 2.46 from the flake-pinned nixpkgs.

#### ISA floppy and ATA boot contract (2026-11-24)

Source paths below are relative to the cached fork checkout at
`vendor/seabios/src/` (rel-1.17.0, fork pin recorded in the fixture README).

| Evidence | Firmware requirement | Board implementation |
|---|---|---|
| READ `src/hw/floppy.c:147–173`, `:90–99` | CONFIG_QEMU reads CMOS 10h; high/low nibbles describe A:/B: (4 = 1.44 MB, 2 = 1.2 MB); IRQ6 | Shared CMOS derivation and FDC wiring already cover both generic boards |
| READ `src/hw/floppy.c:24–28`, `:197–214`, `:533–545`; `src/hw/dma.c:11–55` | 765 at 3F0h; DOR at 3F2h; FIFO at 3F5h; 8237 byte channel 2, address/count 04h/05h, page 81h, single transfer mode | Existing FDC and motherboard DMA; no device behavior change required |
| READ `src/hw/ata.c:1019–1029` | CONFIG_QEMU with no PCI devices probes ISA primary and secondary ATA with IRQ14/15 (1F0h/3F6h, 170h/376h) | Primary IDE card at its standard resources; absent secondary channel remains open bus |
| READ `src/hw/ata.c:768–814`; `src/block.c:80–90` | IDENTIFY supplies capacity/CHS; QEMU CMOS 39h supplies translation bits | Existing IDENTIFY and type-47 CMOS seed; 615/4/17 needs translation 0; CMOS disk-type bytes do not discover the ATA drive |
| READ `src/boot.c:254`, `:460–495` | Without fw_cfg bootorder, CONFIG_QEMU uses CMOS 3Dh and 38h; static non-QEMU defaults are superseded | Existing `3Dh=21h`, `38h=00h` selects floppy then hard disk with floppy signature checking |

`vendor/seabios/config` enables CONFIG_QEMU and disables PCI BIOS and PCI
storage drivers while retaining ISA floppy/ATA. Both generic boards already
allow FDC/IDE cards through validation and capability choices. Tests now
verify CMOS 10h/14h, boot-order and ATA CMOS bytes through actual RTC ports,
as well as both floppy slots' derived types and equipment count.

**MEASURED:** `generic-386-freedos.toml` and `generic-486-freedos.toml`
boot the existing pinned FreeDOS floppy to `A:\>`; the `-ide.toml` profiles
also expose the existing readonly FAT16 C: through SeaBIOS's native ATA
driver. Shared normal goldens cover 80386DX, 80486DX and 80486DX-CPUID,
assert VGA text, serial prompt, disk identification and a completed
`DIR C:` listing of MARKER.TXT. Counts and budgets are in
[machine-files.md §FreeDOS floppy and IDE profiles](machine-files.md#freedos-floppy-and-ide-profiles).
No CPU boundary was observed. Direct C: boot requires a separate bootable
hard-disk fixture and remains a follow-up; it is not tested by these profiles.
The catalogue remains Listed; this is qualification of these exact profiles
and pins, not a change to the catalogue's general firmware status.

**Implemented catalogue entry.** `seabios-1.17.0-unemu1-isa` is **Listed**, not
Tested: a 131,072-byte system BIOS at E0000h, SHA-256
`ad50fdb5e12a03dd5c17eac481a4183a7793b84252319eea02b8c878667c994f`.
It is open firmware (LGPL-3.0-only), built by `vendor/seabios/fetch.sh`
from release `v1.17.0-unemu1`, fork commit
`e38573ec2d7d43fb7802bc43ddcb3ce2bf892809`. Both SeaBIOS and SeaVGABIOS
pin its corresponding source archive, SHA-256
`b85015a695a9b1b8080c4871335bb68c2ec9aa9c8ba69e88a122a925c580c8c0`;
the release adds rebuild files and documentation without changing firmware sources or ROM bytes.
The catalogue declares an 80386 minimum, AT keyboard/PICs and 128-byte CMOS. It belongs to both
`generic-386-isa` and `generic-486-isa`, provides floppy/fixed-disk services and enables
`cmos-qemu-layout`. It has no pre-mapped ROM scan window. The shipped
`machines/generic-386-seabios.toml` supplies register-level VGA and COM1;
the board supplies fw_cfg/SERCON, port 92h and the 402h console.
SeaVGABIOS is supplied through fw_cfg. See
[machine-files.md §Generic 386 ISA board](machine-files.md#generic-386-isa-board)
for RAM limits, the 4 GiB sparse reservation, numeric commitment and the POST
execution boundary.

**Memory and reset (READ).** C0000h–FFFFFh must be writable RAM at POST,
with the 128 KiB BIOS preloaded at E0000h–FFFFFh. SeaBIOS writes F-segment
variables (`post.c:290–299`), patches runtime code during relocation
(`post.c`, `reloc_preinit` / `updateRelocs`), and allocates C0000h–EFFFFh
(`MALLOC_UPPERMEMORY`, `Kconfig:123–130`). Shadow unlock only knows i440FX
or Q35 PAM and otherwise logs "Unable to unlock ram" (`shadow.c:117–147`).
QEMU `isapc` uses an `isa-bios` alias of the BIOS region
(`x86-common.c:1005–1009`), makes it writable via
`set_readonly(!isapc_ram_fw)` (`x86-common.c:1063`,
`pc_sysfw.c:222` passes true), and maps the same region below 4 GiB
(`x86-common.c:1117–1120`). That high window is not a pristine copy:
SeaBIOS takes the "no pristine copy" branch (`shadow.c:178–186`).

Our board's `upc-o0j.12` supplies writable C0000h–DFFFFh,
the preloaded BIOS RAM and a **separate read-only pristine copy** at
FFFE0000h–FFFFFFFFh on the 386DX bus, as required by decision-19 rule 3.
This is our design for the RAM-preserving CPU reset in `upc-o0j.14`;
with an alias, a second reboot reaches the `HaveRunPost == 3` shutdown
path (`shadow.c:169–208`). Adapter-space validation must permit writable
shadow RAM on this board; `ibm-5170` retains read-only adapter space.

With a pristine image available, reboot copies it from `0xFFF00000 + addr`,
writes CF9h, then executes INT3 with a one-byte IDT to triple-fault
(`shadow.c:21–22,169–208`, `misc.c:134–145`). Ctrl-Alt-Del, failed boot
and an 8042 reset with CMOS 0Fh = 0 take this path (`resume.c:146–157`,
`kbd.c:533`, `boot.c:982`). CPU shutdown and port 92h bit 0 must cause a
386 CPU reset retaining RAM (`upc-o0j.14`), using typed shutdown
`xj-srh.6 (x86jit)` and reset/high fetch `xj-srh.12 (x86jit)`.

**CPU requirements (READ unless labelled).** POST uses 32-bit flat protected
mode. `transition32` disables interrupts, masks NMI through 70h bit 7,
enables A20 through 92h bit 1, loads IDTR/GDTR with 16-bit forms, clears
CR0.PG/CD/NW, sets PE and makes a 16:32 far jump (`romlayout.S:24–66`).
The GDT contains flat 32-bit code/data, 16-bit code/data with 64 KiB limits
and 16-bit code/data with 4 GiB limits (`misc.c:148–169`). Every POST
yield/IRQ check calls 16-bit code with `sti; nop; pause; cli`; IRQs must be
accepted in that window (`stacks.c:607–621`). Returns use
`transition16big`, leaving big real mode for the VGA ROM too
(`romlayout.S:87–117`, `stacks.c:291–292`). INT 15h AH=87h uses 16-bit
protected mode and `rep movsl` (`system.c:72–165`); AH=89h, E801h and
E820h are supported (`system.c:185,260,290`).

Real-mode firmware is built with gcc `-m16 -march=i386`
(`Makefile:64,83–84`): it needs 66h/67h, FS/GS, `pushl`/`pushfl` and
32-bit `calll`/`retl` in 16-bit code. The spike assumed no BSWAP/CMPXCHG/CMOV
from compiler flags. BSWAP is now MEASURED above, emitted by inline assembly
in upstream `byteorder.h:15–17`; the fork replaces that helper. Compiler
flags alone cannot establish CPU compatibility. The CPU
requests sent and filed on 2026-11-11 are:

- **R1 — `xj-srh.16 (x86jit)`:** full 386 real-mode integer ISA under the
  386 model, including SIB, REP with 67h, 66h near CALL/RET and stack forms,
  the 15-byte limit, and the 386 0Fh map (MOVZX/MOVSX, SETcc, Jcc rel16,
  BT*, BSF/BSR, SHLD/SHRD, two-operand IMUL, LSS/LFS/LGS, PUSH/POP FS/GS).
  This extends the partial entry forms of `xj-srh.2 (x86jit)` after
  `xj-srh.4 (x86jit)`; test386.asm (`upc-fro.1`) and a firmware opcode
  census are the proposed oracles.
- **R2 — acceptance added to `xj-srh.4 (x86jit)`:** after PE 1→0, real-mode
  segment loads change only the base and limit checks use the cached limit.
  A PE transition must not flush or relift the other mode's blocks wholesale.
- **R3 — `xj-srh.17 (x86jit)`:** 386 identity: EFLAGS.AC/ID fixed zero;
  CPUID, RDTSC, RDMSR/WRMSR, WBINVD/INVD/INVLPG, BSWAP, CMPXCHG and XADD
  raise #UD under that model. SeaBIOS tests EFLAGS.ID before CPUID
  (`x86.c:10–23`); without it, TSC/MTRR/APIC setup is skipped and timing
  falls back to PIT (`timer.c:101–116`, `mtrr.c:41–44`, `smp.c:104–114`).
  WBINVD/RDMSR use is behind chipset/CPUID checks. ASSUMED reset DX
  signature: 0303h; not qualified by this spike.

Upstream order is `xj-srh.4 (x86jit)`, `xj-srh.15 (x86jit)`, then
`xj-srh.16 (x86jit)` / `xj-srh.17 (x86jit)`. SeaBIOS uses only port 92h
for A20, never the 8042 D1h path (`x86.h:17–19,268–277`,
`system.c:18–35`); an open-bus FFh read falsely looks enabled.

**Devices at POST (READ).** PIC initialization uses ICW 11h and vectors
08h/70h (`pic.c:41–66`). PIT channel 0 uses mode 2, count 0
(`timer.c:274–283`); all non-TSC timing uses 8254 read-back D2h
(`timer.c:171–183`), while 61h bit 5 is used only on the TSC path. RTC
sets status A to 26h, 24-hour mode in B, reads C/D and polls UIP
(`rtc.c:40–72`); INT 15h 83h/86h need periodic IRQ8 (`rtc.c:77–100`).
The 8042 sequence includes ADh/A7h, AAh→55h, ABh→00h, 20h/60h, D4h and
ten FEh attempts; keyboard commands include FFh (ACK then AAh), F5h,
F0h 02h with translation, F4h and EDh (`ps2port.c:135–146,450–524`).
Both DMA controllers are reset via DAh/D6h/D4h (`dma.c:58–67`). FDC
uses DOR, MSR/data and DIR/CCR at 3F7h; commands include READ ID, SEEK,
RECALIBRATE, SPECIFY, READ E6h and WRITE C5h
(`floppy.c:24–28,249–256`). ATA probes both ISA channels, tolerates FFh
for absent hardware and uses IDENTIFY/PACKET IDENTIFY and READ/WRITE
SECTORS (`ata.c:826–847,1020–1030`); runtime disk services run in real
mode (`block.c:600–615`). COM1–4 and LPT are probed; debug output goes to
402h (`serialio.c:105–129`). FPU equipment bit 1 is always set, with an
IRQ13 handler and writes to F0h (`misc.c:62–68`). A dword CF8h read of
FFFFFFFFh selects non-PCI operation (`pci.c:217–225`); HPET/ACPI/PM timer
are off in QEMU's 128 KiB config. Later VGA requires a SeaVGABIOS
`VGA_STANDARD_VGA` or QEMU `isavga` build: floating Bochs VBE ports
1CEh/1CFh fall back to standard VGA (`bochsvga.c:302–313`), but sequencer,
GC, AC, DAC, planar memory and plane-2 font loading still need a register-level
card (`stdvga.c:559–567`) and 386 real-mode execution.

The requirement table is **READ** from the spike's pinned sources; "have"
records existing components, not demonstrated SeaBIOS compatibility.

| Requirement | SeaBIOS source | Have / partial / missing | Bead |
|---|---|---|---|
| Flat 32-bit PM POST, 16↔32 transitions | `romlayout.S:24–117` | Missing | `upc-o0j.2`; `xj-srh.1 (x86jit)` through `xj-srh.5 (x86jit)` |
| gcc `-m16` 386 real-mode ISA | `Makefile:64,83` | Missing: Real16 has 286 policies (`lift/mod.rs:285,374` at the spike CPU pin) | R1: `xj-srh.16 (x86jit)` |
| Big real mode after `transition16big` | `romlayout.S:87–117`; `stacks.c:291` | Missing: fixed real-mode limit (`lift/mod.rs:4962`) | R2: `xj-srh.4 (x86jit)` acceptance |
| Fixed EFLAGS.ID, CPUID #UD | `x86.c:10–23` | Missing: CPUID lifted in every mode (`lift/mod.rs:2371`) | R3: `xj-srh.17 (x86jit)` |
| Triple-fault CPU reset, port 92h bit 0 | `misc.c:134–145`; `resume.c:141` | 92h device reset on Bare uses existing warm-restart path (documented edge/readback; pulse timing ASSUMED); triple-fault reset pending | `upc-o0j.14`; `xj-srh.6 (x86jit)` |
| Writable C0000h–FFFFFh, preloaded images | `post.c:290–299`; `shadow.c:117–147`; `optionroms.c:467` | Implemented on generic-386-isa; IBM 5170 unchanged | `upc-o0j.12` |
| BIOS copy at FFFE0000h–FFFFFFFFh | `shadow.c:22,169–196` | Separate read-only copy implemented; CPU reset fetch still blocked | `upc-o0j.12`; `xj-srh.12 (x86jit)` |
| At least ~1.25 MiB contiguous RAM | `malloc.c:413–456` | Partial | `upc-o0j.6` |
| CMOS 30h/31h, 34h/35h, 5Bh–5Dh | `paravirt.c:240–254,588` | Partial: `cmos-qemu-layout` capability exists; enabled by the SeaBIOS entry with the board. Full derivation and a Rust-configured 128-byte DS12887-compatible RTC exist on Bare rigs; validation requires `rtc-cmos-128`, preventing high RAM fields from aliasing type-47 geometry | `upc-o0j.11`; `upc-o0j.6` |
| CMOS 32h, 38h/3Dh, 39h, 5Fh | `clock.c:48`; `boot.c:471–489`; `block.c:84–90`; `paravirt.c:535` | 32h holds `RtcStart.year / 100` in BCD for every derived AT layout, independently of firmware capabilities; images retain the supplied byte and journals retain the persisted byte. The other fields are partial: `cmos-qemu-layout` capability exists; enabled by the SeaBIOS entry with the board. A Rust-configured 128-byte RTC satisfies `rtc-cmos-128` on Bare rigs, preventing a false CPU count from 5Fh aliasing 1Fh; images/journals remain 64-byte only, without machine-file size syntax | `upc-o0j.11` |
| A20 through port 92h bit 1 | `x86.h:268–277` | Device side complete on Bare: shared 8042/92h OR line (ASSUMED policy), latch/readback and snapshots; CPU masking pending; enabled 8042 prevents 92h-only disable under OR | `upc-o0j.21`; `upc-o0j.6` chooses arbitration and tests SeaBIOS AX=2400h/2402h; CPU gate wiring remains pending |
| fw_cfg at 510h/511h | `paravirt.c:664–727`; `sercon.c:519` | Partial: device exists; board wiring pending | `upc-o0j.13` |
| 8254 read-back command | `timer.c:171–183` | Have: count/status latches, OUT and NULL COUNT | `upc-o0j.10` |
| 8042 command set and keyboard | `ps2port.c:450–524` | Have, without aux | `upc-18x.6` |
| CMOS 0Fh resume 05h/0Ah/0Bh/0Ch | `resume.c:23–84` | Have for 286; 386 reset missing | `upc-6le.25`; `upc-o0j.6`; `upc-o0j.14` |
| Both 8237 controllers | `dma.c:58–67` | Have | `upc-6le.21` |
| RTC periodic IRQ8 | `rtc.c:40–100` | Have; periodic behavior partial | — |
| FDC with READ ID and CCR | `floppy.c:249–256` | Have | — |
| Primary/secondary ISA ATA | `ata.c:1020–1030` | Have | — |
| COM1 UART | `sercon.c` | Have | — |
| VGA ROM and register-level VGA | `optionroms.c:451`; `stdvga.c` | Partial: isolated register file and planar memory; IBM VGA/XGA TR (May 1992), only the module coverage-matrix figures checked: 2-19–2-26, 2-29–2-37, 2-58, 2-69–2-80, 2-81–2-89 and 2-94; chain-4 offset DERIVED from Figs. 2-37/2-65; ASSUMED floating ports/unselected memory FF, 128 KiB mirroring, MSR[5] polarity, DAC edge cases, odd/even gating of A0 replacement and chain-4 pipeline; alphanumeric rendering and opt-in `vga` card wiring implemented (READ: Alphanumeric Modes, Figs. 2-7–2-8, 2-31, 2-33–2-36, 2-41, 2-48, 2-50–2-56, 2-59–2-61, 2-64–2-65, 2-82–2-83, 2-84, 2-86, 2-89 and 2-94: guest fonts, divided dot clock, CRTC scanout/cursor, AC/DAC colours through Frame::Text; ASSUMED zero-based cursor start/underline despite the manual minus-one wording, following SeaVGABIOS crtc_07); CGA-compatible 06h, planar 0Dh/0Eh/10h/12h and chain-4 13h graphics scanout implemented through owned indexed Frame::Pixels (READ: Graphics Modes, Figs. 2-31, 2-50, 2-60–2-65, 2-75 and 2-84–2-89; DERIVED doubleword placement matches CPU chain-4; DERIVED/ASSUMED mode-13h internal palette low-nibble sample handling; logical dot/scan-line repetition retained as presentation factors; READ CMS0/SRC row-scan bank substitution from Fig. 2-64); AC pixel panning, CRTC byte panning and start-address page flips latched at retrace, and the line-compare split are modelled at frame level (status.md, VGA frame scrolling and page flipping); CGA-interleaved shift, smooth-scrolling timing finer than a frame and other mid-frame changes remain unmodelled; the IBM ISA VGA BIOS and the open LGPL VGABIOS fork are qualified on ibm-5170 (§Video ROM placement), while SeaVGABIOS delivery via fw_cfg remains pending for the 386 board | `upc-o0j.15` (`upc-18x.2` is VBE only) |
| FPU always reported | `misc.c:62–68` | Partial: empty socket | `upc-18x.9` (risk) |
| Avoid unmapped FEE00030h MP-table read | `mptable.c:57` | Built fixture disables MPTABLE; Kconfig setting verified | `upc-o0j.9` (build half; catalogue waits for board) |
| Debug console at 402h | `serialio.c:104–129`; `fw/paravirt.h:67` | Device and `DebugConsole` byte observation exist (E9h readback); Bare rigs only, board wiring pending | `upc-o0j.16`; `upc-o0j.6` (board) |

**Risks and fallback.** ASSUMED: thousands of interpreter PE transitions,
relocation and ROM-copy SMC churn may make POST slow; no timing was measured.
READ: non-clean builds embed build time and hostname
(`scripts/buildversion.py:127–130`), so the fixture uses fixed release metadata
and checks the generated version and image for hostname leakage;
the FPU is always claimed (`misc.c:62–68`) despite the empty socket.
INT 15h AH=88h reports the end of the highest E820 RAM entry, not CMOS
(`malloc.c:476–491`); `upc-o0j.6` acceptance therefore compares against E820.
Bochs legacy `rombios` remains the fallback: 16-bit, no 32-bit code
(`rombios.h:24–27`), but 386+ (`rombios.c:122–123`), executable from ROM
with pre-placed option-ROM scanning, a mandatory C0000h VGA ROM or halt
(`rombios.c:11323–11337`), port-92h-only A20 (`rombios.c:2347–2353`),
dev86 builds and no serial console. Its CMOS inputs are 10h, 12h/19h/1Ah,
2Dh, 30h/31h, 34h/35h, 5Bh–5Dh, 38h/3Dh, 3Fh and 32h (READ); fallback
therefore needs register-level VGA from its first run.

### 3.7 Generic 486 ISA board

The `generic-486-isa` identity shares the 386 ISA board's memory map, A20/reset
wiring, shadow RAM, fw_cfg and peripherals (§3.6). A separate identity keeps
machine files and catalogue compatibility explicit while giving the DX its
mandatory integrated FPU. This is smaller than weakening the 386 board's CPU
and empty-socket validation. The machine class is `At486`; the CPU choice is
explicitly `80486dx` or `80486dx-cpuid`, backed by x86jit's `I80486Dx` and
`I80486DxCpuid` at revision `274d48a8ab9dabdfd598c3c3116cf72363ffa049`.

Both use the same Listed SeaBIOS fork build and SHA-256 as §3.6. Only the
catalogue's board fits change; no firmware bytes, source or build options do.
The 486 reset helper installs CR0=60000010h; reset EDX is 0403h for the early
DX and 0410h for the CPUID model. The latter reports GenuineIntel, leaf 0
maximum 1, and leaf 1 EAX=0410h, EDX=1 (FPU). These signatures are the CPU
core's representative stepping choices, not a claim about every physical DX.

**MEASURED before SeaVGABIOS, at `274d48a8` (2026-11-24):** both DX
variants finish diskless POST at 5,885,053 retired instructions and reach keyboard reset/resume at
5,887,802. INT 15h AH=88h/E801h/E820h checks pass at 16 MiB (5,768,863)
and 4 MiB (5,768,833). Both print `No apic - only the main cpu is present.`
and `No bootable device.`; neither prints a vendor/model identification line.
**READ:** the fork's `src/x86.c:10–22` probes EFLAGS.ID before CPUID;
`src/fw/smp.c:104–114` reports the no-APIC message for either an absent
CPUID or a leaf-1 result without APIC. The identical text is expected even
though the CPUID DX advertises its on-chip FPU. The project-owned ROM
separately checks the ID policy and all leaf-0/1 values on the selected model.
No CPU boundary was encountered in these SeaBIOS probes.

External numeric-error routing remains refused by the CPU on both models.
Assembly keeps internal delivery and does not attach the AT F0h/IRQ13 latch.
CR0.NE-driven FERR#/IGNNE# routing remains a gap alongside paging, CPL-3 #AC
and TR3–TR5. The project-owned 486 ROM checks only WP/AM/AC read-back, never
paging protection or alignment exception delivery. See
[machine-file keys and board policy](machine-files.md#generic-486-isa-board)
and [CPU gaps](protected-mode-gaps.md#486-board-coverage-and-gaps).

## 4. Classes and device sets

### 4.1 Standard wiring per class

Sources:

- *IBM Personal Computer XT Technical Reference* (5155/5160, 6280089,
  March 1986), §1 "System Board": "DMA" (p. 1-14), "I/O Address Map"
  (p. 1-24), "8255A I/O Bit Map" (pp. 1-27/28); §2 "Coprocessor";
  §4 "83-Key Keyboard" (p. 4-4) and "Block Diagram" (p. 4-5);
  §5 BIOS listing dated 01/10/86.
- *IBM Personal Computer AT Technical Reference* (1502494, March 1984; 6183355,
  March 1986), Section 1 "System Board". It covers System Interrupts, System
  Timers, Direct Memory Access, the I/O Address Map, "Keyboard Controller"
  (p. 1-42, 1986) and "Real-Time Clock/CMOS RAM Information" (p. 1-56, 1986).
- The fixed-disk interface is in the AT's *Fixed Disk and Diskette Drive
  Adapter* (Options and Adapters).

XT device assignments are READ from the March 1986 reference and BIOS
listing for §9 item 14. Its module matrices distinguish remaining page-wiring,
readback and timing assumptions. Other register values in this comparison
remain ASSUMED until checked in the corresponding device work.

| Part | xt (`ibm-5160`) | at (`ibm-5170`) | at386 (`generic-386-isa`) | In the repo today |
|---|---|---|---|---|
| CPU | 8088 (+8087) | 80286 (+80287 at F0h–FFh, IRQ13) | 80386 (+387) | 80286 Real16 only |
| Interrupts | **one** 8259A, 20h–21h, IRQ0–7, single mode | 8259A pair, 20h/A0h, slave on IRQ2 | as AT | cascaded pair ✓; single-mode mock-bus check ✓ |
| Timer | 8253, 40h–43h. ch0 → IRQ0, ch1 → DRAM refresh (DMA ch0), ch2 → speaker through PPI PB0/PB1 | 8254. ch1 refresh shows in 61h bit 4, ch2 → speaker through 61h | as AT | 8253/8254 ✓; port 61h bit 4 follows PIT1 refresh requests, bit 5 reads OUT2 and bit 0 drives GATE2 |
| DMA | **one** 8237A at 00h–1Fh (upper half aliases lower), ch0 refresh. Page decode 80h–9Fh; 81h–83h (ch2, ch3, ch1), 87h ch0 ASSUMED | **two** 8237A: 00h–1Fh (ch0–3) and C0h–DFh (ch4–7, ch4 cascade). Pages at 80h–8Fh | as AT | both 8237 banks ✓; byte pages 81h–88h, word pages 89h–8Fh (READ `board.rs:99–108`) |
| Keyboard interface | **8255A PPI at 60h–6Fh** (four-register aliases): PA scan code, PB control (PB7 clear, PB6 clock, PB3 switch select), PC switches and status | **8042** at 60h/64h; output port bit 1 = A20, bit 0 = reset | 8042 + port 92h | 8042 ✓ (A20 bit is state only, `FEh` CPU reset pulse ✓); **8255 device model ✓** (not assembled) |
| Keyboard | **83-key**, scan set 1, serial into PPI port A, IRQ1 | 84-key (101 later), set 2 translated to set 1 | as AT | host → set 1 encoder with E0 codes ✓ (an XT has no E0 keys) |
| Config store | **DIP SW1** read through PC0–3, PB3 selects half | MC146818 CMOS at 70h/71h, IRQ8; NMI mask is 70h bit 7 | as AT, plus extended memory bytes | CMOS ✓, derived |
| NMI mask | **A0h–AFh** aliases, bit 7 enables | 70h bit 7 | as AT | AT form ✓ |
| A20 | none (20-bit bus) | 8042 output port | 8042 + **port 92h** bit 1 | shared board line ✓ (OR policy ASSUMED); CPU masking pending, addressing remains enabled |
| CPU reset | — | 8042 `FEh`, CMOS 0Fh shutdown code | + port 92h bit 0 | 8042 and Bare 92h CPU-only warm restart ✓ (pulse timing ASSUMED); RAM, CMOS and devices retained; sustained low reset level not modelled |
| ROM | F0000h–FFFFFh; an 8 KiB BIOS at FE000h | 64 KiB at F0000h (even/odd) | SeaBIOS: 128 KiB preloaded at E0000h in writable C0000h–FFFFFh, pristine read-only copy at FFFE0000h (§3.6) | `ImageKind::Rom` ✓ |
| Card bus | 8-bit, IRQ2–7, DMA1–3 | 16-bit, IRQ3–7, 9–12, 14, 15 | as AT | no slot abstraction (decision-16 §8) |
| Floppy | adapter with DOR 3F2h and MSR/data 3F4h/3F5h; ASSUMED no CCR at 3F7h | 3F0h–3F7h, IRQ6, DMA2 | as AT | µPD765 ✓ |
| Fixed disk | Xebec MFM adapter at 320h with its own ROM; open alternative is an 8-bit XT-IDE card + XUB `ide_xt` | WD1003-style task file at 1F0h–1F7h plus 3F6h, IRQ14. The IBM BIOS reads a CMOS drive type | as AT | ATA-2 PIO channel assembled with volatile/readonly media; firmware boot profile deferred |
| Video | MDA/CGA, chosen by SW1 5–6 | MDA/CGA through CMOS; EGA/VGA by ROM | VGA ROM + register-level VGA | MDA ✓; VGA text (CRTC + CGA registers) ✓ |

### 4.2 What is missing, per board

- **`ibm-5170` completion status (READ):**
  - second 8237 at C0h–DFh with page registers 89h–8Fh: implemented
    (READ `crates/assembly/src/board.rs:104–108`, `assemble.rs:304`);
  - empty 80287 socket ports F0h/F1h, F8h–FFh: implemented (READ
    `board.rs:125–130`); a present 80287 now drives the IRQ13 latch and F0h
    releases BUSY without clearing numeric status. F1h reset and F8h–FFh bus
    transfers remain unmodelled; see [coverage](machine-config.md#coprocessor-controls);
  - 8042 `FEh` reset pulse and CPU-reset seam: implemented; RAM and CMOS
    0Fh survive (READ `assemble.rs:261–264,338–339`,
    `crates/machine/src/lib.rs:305–316`);
  - **ATA device**: implemented and qualified with the new profile (§9 item 11);
  - String port I/O landed at x86jit `73ed5d6`: Real16 INS/OUTS,
    including REP, segment overrides on OUTS and per-element IRQ boundaries.
    The historical `96a49a9` probe did not lift opcodes 6C–6F.
- **`ibm-5160`:**
  - device models for the 8255/SW1, XT keyboard (PB7/PB6, 83-key set 1),
    XT DMA pages and A0h NMI mask are available in isolation (§9 item 14),
    pending board wiring, with remaining assumptions recorded in module matrices;
  - the existing single-mode 8259 honours ICW1 SNGL (mock-bus test);
  - the NMI mask at A0h clashes with an AT slave PIC, so future composition
    must keep it board-specific;
  - an FDC without CCR;
  - an 8-bit card limit and IRQ ≤ 7;
  - the 8088 CPU model (`xj-50w.2 (x86jit)`).
- **`generic-386-isa`, phase B:** board wiring, 128-byte QEMU CMOS,
  extended RAM, port 92h/A20, fw_cfg/SERCON, writable shadow RAM, pristine
  high BIOS copy, 8254 read-back and register-level VGA now assemble.
  SeaVGABIOS delivery, execution of the 386 reset fetch (`xj-srh.12 (x86jit)`),
  the rest of the 386 real-mode integer ISA and boot qualification remain
  pending. See §3.6 for the source-backed requirements.

## 5. Config expression

The description layer (`upc-6le.1`) lowers to `MachineConfig`. The new layer
is `[board]` and `[firmware]`; `class` is never written.

**Illustrative FreeDOS machine (abbreviated; the complete configuration is
`machines/at-freedos.toml`):**

```toml
schema = 1
name = "freedos-floppy"

[board]
model = "ibm-5170"            # class "at" is derived
# This BIOS reads the AT display switch with inverted polarity; in the colour
# position it selects the monochrome adapter. Derived CMOS still declares colour.
display_switch = "colour"

[cpu]
model = "80286"
rate = "8/13"

[memory]
conventional_kib = 640

[firmware]
bios = { image = "../vendor/kiselev-bios/bios-xi8088.bin",
         sha256 = "1ad458acb1f22dee6b3d9b6bd95e83218a7ddc5389fa2b2c67a4001e689c81a8" }

[[card]]
kind = "mda"

[[card]]
kind = "vga-text"

[[card]]
kind = "fdc-765"
drives = [ { slot = 0, type = "1.44M", image = "../vendor/freedos/…", write = "volatile" } ]
```

**AT with a hard disk (new profile; old goldens untouched):**

```toml
[board]
model = "ibm-5170"

[firmware]
bios = { image = "…/bios-xi8088.bin", sha256 = "1ad458ac…81a8" }
option_roms = [
  { image = "…/xub-r638-at.rom", sha256 = "<padded hash>", base = 0xC8000 },
]

[[card]]
kind = "ide"                    # defaults: 0x1F0–0x1F7, 0x3F6, IRQ 14
drives = [ { channel = "master", image = "hd.img",
             geometry = { cylinders = 615, heads = 4, sectors = 17 }, write = "volatile" } ]
```

Rules (the implemented pre-mapped ROM path; SeaBIOS shadow RAM and
firmware-loaded option ROMs are the board-specific extension in §3.6):

- **Identity.** A firmware entry is identified by the SHA-256 of its bytes,
  after split parts are interleaved. `sha256` in the file is optional; when
  present it must match the bytes. `id` is an optional label; when present it
  must name the entry the hash resolves to.
- **`provides` comes from the catalogue.** A file may restate it, and it must
  then equal the catalogue's. It is mandatory only for an unlisted image, if the
  maintainer allows those (§8).
- **System BIOS placement.**
  - A ROM ending at 1 MiB identifies the BIOS before any declaration. If none
    does, the config's declared BIOS index is used; without a declaration, a
    sole ROM is the candidate. These candidates report `BiosPlacement` when
    mis-sized or misplaced. Only zero ROMs or several undeclared candidates
    report `MissingSystemBios`; the highest address is never a fallback.
  - `base = 1 MiB − size`, which puts the reset vector F000:FFF0 at offset
    `size − 16`. The size must be a multiple of 8 KiB.
  - `generic-386-isa` maps a separate read-only pristine copy at
    `4 GiB − size`, not an alias of its writable shadow RAM (§3.6,
    `upc-o0j.12`). PCI boards alias the image at `4 GiB − size`, as SeaBIOS
    expects (`shadow.c`: "the bios at 0xf0000 is also at 0xffff0000").
  - Boards that need a writable shadow map it as RAM preloaded with the image,
    or emulate PAM.
- **Option ROMs.**
  - `base` is required in machine files; catalogue suggestions are informational.
    The size is
    the header's `size byte × 512`, and the image must be exactly that long.
  - Lowering maps each ROM as `ImageKind::Rom`. The IBM PC AT Technical
    Reference, Section 1, **System Memory Map**, labels C0000h–DFFFFh as
    I/O expansion ROM, E0000h–EFFFFh as reserved on the system board, and
    F0000h–FFFFFh as system board ROM. For boards with a firmware catalogue,
    assembly maps gaps from C0000h up to the system BIOS image's base, carving
    around MMIO and ROM windows. This board-owned address policy is independent
    of firmware scan capability: a diagnostic with `scan_window = None` still
    receives gap mappings. With Kiselev at F8000h the mapping is unchanged;
    a 64 KiB BIOS at F0000h leaves gaps only below F0000h. Image/MMIO overlap is
    refused. Empty gaps deliberately read 00h and remain read-only, so guest
    writes and `RamLoad` images cannot introduce uncatalogued extension code.
    Whether absent sockets should read FFh remains an electrical-model question;
    confirm authoritative evidence before changing the established zero value.
  - A pre-mapped video ROM (`kind = video-bios`) sits in C0000h–C7FFFh;
    SeaBIOS instead loads its video ROM through fw_cfg (§3.6).
- **Scan order** for pre-mapped option ROMs is ascending address; these
  BIOSes scan upward in 2 KiB steps:
  - Kiselev C800h–F800h, Bochs C800h–E000h, Super PC/Turbo XT C000h–FE00h
    (READ);
  - GLaBIOS with `ROM_SCAN_ALIGN = 1` (READ);
  - IBM 5170 C800h–E000h (READ, TEST2.ASM above).

  The file's list order carries no meaning. Assembly records resolved ROMs,
  including their effective catalogue services, in ascending address order in
  `AssembledMachine.option_roms` for diagnostics and tests.
- **Derived configuration.**
  - CMOS (AT) and DIP switches (XT) are derived from the memory, drives,
    cards and the fixed-disk provider.
  - The CMOS fixed-disk type bytes (12h, 19h, 1Ah) stay **0** when an option
    ROM provides `fixed-disk`. They are derived from `HardDisk` drives only when
    the system BIOS's entry declares `cmos-drive-types`.
  - The entry must also provide `fixed-disk`, and no option ROM may provide
    that service. Unlisted BIOS declarations cannot opt into this convention.
    The derived seed includes the extended bytes and AT checksum.
  - READ: [IBM Personal Computer AT Technical Reference, September 1985](https://www.minuszerodegrees.net/manuals/IBM_5170_Technical_Reference_6280070_SEP85.pdf),
    Real-Time Clock/CMOS RAM Information, pp. 1-62–1-66: 12h contains drive C/D
    nibbles; nibble F selects 19h/1Ah. The fixed-disk parameter tables define
    types 1–14 and 16–22; 23–255 (including 47) are reserved. Assembly matches
    512-byte, 17-sector CHS and chooses the lowest type for duplicate CHS
    (precompensation and landing zones are not modelled). A geometry outside
    these tables returns `UnsupportedCmosDiskGeometry`; no vendor-specific
    user-defined type-47 layout is assumed.
  - READ: Kiselev `8088_bios` at the catalogue-pinned revision,
    `src/rtc.inc`, `rtc_init` / `nvram_save`, and `src/bios.asm`, `ipl` / `.try_hdd`:
    the BIOS checksums 10h–20h but interprets floppy/equipment settings only;
    hard-disk boot calls INT 13h supplied by an extension. Kiselev and XUB
    therefore declare no `cmos-drive-types` capability.

The description lowers to the existing `LoadImage` seam. Split chips are one
image source, not separately mapped ROMs:

```rust
pub enum ImageSource {
    Inline(Vec<u8>),
    File(PathBuf),
    Blank { size: u64 },
    Interleaved(Vec<ImagePart>),
}
pub struct ImagePart { pub path: PathBuf, pub lane: Lane, pub offset: u64 }
pub enum Lane { Even, Odd }
```

## 6. Validation rules

Description-level checks run before lowering. Hash checks run where file bytes
are read (assembly already validates file lengths after reading). The lowered
`MachineConfig` then passes the existing `validate()`. Each rule is a new
`ConfigError` variant with an exact message under test.

| # | Rule | Error | Example message |
|---|---|---|---|
| 1 | The board model is known | `UnknownBoardModel` | `unknown board model "ibm-5171"; known: ibm-5170, ibm-5160 (reserved), …` |
| 2 | `class` is not written | `ClassIsDerived` | `"class" is derived from [board] model; remove it (ibm-5170 is class "at")` |
| 3 | The CPU model is a socket option of the board | `CpuNotOnBoard` | `board "ibm-5160" takes cpu 8088 or v20, not "80286"` |
| 4 | The CPU model is implemented by the core | `CpuModelUnavailable` | `cpu model "8088" is reserved: the CPU core does not model it yet (x86jit xj-50w.2)` |
| 5 | The board model is not reserved | `BoardUnavailable` | `board "ibm-5160" is reserved until an XT-class CPU model exists` |
| 6 | The firmware hash is in the board's catalogue | `FirmwareNotListed` | `bios image (sha256 6d50…) is not a known image for board "ibm-5170"; known: kiselev-xi8088-1.0.2 (open, vendor/kiselev-bios/fetch.sh), ibm-5170-bios-1985-06-10 (user-supplied: U27 6480090 even, U47 6480091 odd; add its ROM folder in Preferences or Import ROM folder…, or pass --rom-dir)` |
| 7 | The catalogue entry fits this board | `FirmwareBoardMismatch` | `firmware "glabios-0.4.3-5160" is built for ibm-5160, not ibm-5170` |
| 8 | The CPU meets the firmware minimum | `FirmwareNeedsCpu` | `firmware "seabios-1.17.0-isapc" needs an 80386 or later; cpu is 80286` |
| 9 | The firmware's required parts are present | `FirmwareRequiresDevice` | `firmware "bochs-rombios-legacy" requires a video-bios option ROM at 0xC0000; none is configured` |
| 10 | Each card that needs a service has a provider | `NoProvider` | `device Ide needs firmware service fixed-disk; no ROM provides it` |
| 11 | An option ROM's baked resources match a card | `OptionRomCardMismatch` | `option ROM "xub-r638-at" drives ATA at 0x1F0/IRQ14, but no ide card decodes 0x1F0` |
| 12 | An option ROM is aligned and inside the firmware's scan window | `OptionRomMisaligned`, `OptionRomOutsideScanWindow` | `option ROM at 0xF8000 is outside the scan window 0xC8000..0xF8000 of "kiselev-xi8088-1.0.2"` |
| 13 | An option ROM's header, length and checksum are valid | `OptionRomBadHeader`, `OptionRomBadChecksum` | `option ROM xub-r638-at: byte sum is 0xB0, must be 0 (pad and checksum the image)` |
| 14 | No two ROMs share a base | (existing `ImageOverlap`) | — |
| 15 | Each card fits the board bus | `CardNeedsSlot` | `card "ide" is a 16-bit ISA card; board "ibm-5160" has 8-bit slots only` |
| 16 | The IRQ line exists on the board | `InvalidIrqLine` (bounded per board) | `device Fdc uses irq line 9; board "ibm-5160" has irq lines 0 through 7` |
| 17 | A system BIOS is top-aligned with a valid size | `BiosPlacement` | `system BIOS of 24 KiB at 0xf8000 must have a positive size that is a multiple of 8 KiB and end at 0x100000` |


Split-source diagnostics additionally use these exact messages (SHA-256 values
in `EXPECTED`/`ACTUAL` stand for full lowercase hashes):

| Rule | Error | Exact message |
|---|---|---|
| Both lanes exist | `FirmwareParts` | `firmware parts: parts require at least one even and one odd part` |
| One part per lane/offset | `FirmwareParts` | `firmware parts: duplicate even part at offset 0` (or `odd`) |
| Complete pairs | `FirmwareParts` | `firmware parts: offset 0 requires exactly one even and one odd part` |
| Equal chip sizes | `FirmwareParts` | `firmware parts: parts at offset 0 have unequal sizes: even 1, odd 2` |
| Nonempty chips | `FirmwareParts` | `firmware parts: parts at offset 0 must be nonempty` |
| Contiguous pairs | `FirmwareParts` | `firmware parts: parts leave a gap at offset 4; expected offset 2` |
| Disjoint pairs | `FirmwareParts` | `firmware parts: parts overlap at offset 2; expected offset 4` |
| Representable size | `FirmwareParts` | `firmware parts: assembled size overflows` |
| Available allocation | `FirmwareParts` | `firmware parts: assembled image is too large` |
| System ROM source only | `FirmwareParts` | `firmware parts: drives cannot use parts` |
| ROM rather than RAM load | `FirmwareParts` | `firmware parts: RAM load images cannot use parts` |
| No split option ROMs | `FirmwareParts` | `firmware parts: option ROMs cannot use parts; only firmware.bios supports parts` |
| Swapped lanes (including one pair of a larger layout) | `FirmwarePartsSwapped` | `firmware parts are swapped for synthetic-two-layout-bios: even lane offset 0 holds U37 low-odd, which belongs in the odd lane at offset 0; odd lane offset 0 holds U17 low-even, which belongs in the even lane at offset 0` |
| Misplaced halves | `FirmwarePartsMismatch` | `firmware parts do not match synthetic-two-layout-bios: even lane offset 0 holds U27 high-even, which belongs in the even lane at offset 4; odd lane offset 0 holds U47 high-odd, which belongs in the odd lane at offset 4; even lane offset 4 holds U17 low-even, which belongs in the even lane at offset 0; odd lane offset 4 holds U37 low-odd, which belongs in the odd lane at offset 0` |
| Duplicate and missing chips | `FirmwarePartsMismatch` | `firmware parts do not match synthetic-user-bios: U27 synthetic-even appears 2 times; U47 synthetic-odd (odd lane offset 0) is missing` |
| Unexpected position | `FirmwarePartsMismatch` | `even lane offset 49152 is not part of synthetic-two-layout-bios layout` (after the error prefix) |
| Bad dump or wrong chip | `FirmwarePartsMismatch` | `firmware parts do not match synthetic-two-layout-bios: odd lane offset 0: expected U37 low-odd sha256 EXPECTED, actual ACTUAL (bad dump or wrong chip)` |

The same-part-count layout with the most supplied chip hashes in any position
wins; catalogue order breaks ties. Duplicate and missing known chips are named
first in layout order, followed by part-position errors in input order, separated
by `; `. The word `swapped` is used only for a permutation in which each known
chip appears exactly once and every misplaced chip changed lane within its pair.
Unexpected lane/offset positions are reported even when no expected chip occupies
that slot. Layout validation and per-part pins run first, chip
identity diagnostics next, then the assembled pin. A recognized assembled image
bypasses chip diagnoses, so a pin for a different listed version still fails.
The membership escape also bypasses chip identity diagnoses, but retains layout
and pin checks. Source-labelled errors name `firmware.bios.parts` when parts
define the BIOS. Multi-pair layouts in `FirmwareNotListed` include `lane@offset`;
alternative layouts are separated with ` or `.

Rule 10 is implemented through `LoadImage.provides` and kind-owned requirements:
IDE requires `FixedDisk` even without drives; FDC requires `Floppy` when a drive
is bound. `Bare` rigs skip these rules. The stable assembly diagnostic is
`device Ide needs firmware service fixed-disk; no ROM provides it`; machine-file
lowering adds the card id and defining file/line. A recognized system BIOS takes its services from the catalogue; an omitted or
empty declaration uses that set, and a nonempty restatement must match it.
A mismatch reports both sets as `FirmwareProvidesMismatch`, even with the
unlisted-firmware escape enabled. Unlisted firmware retains declared services.
A diagnostic system ROM can instead declare the catalogue-only capability
`DirectHardwareTest(service)` for each controller family it tests directly.
Landmark AT declares it for `Floppy` and `FixedDisk`: an FDC with a drive and
an IDE card therefore need no INT 13h provider while that diagnostic runs.
This waives only the corresponding device requirement; it supplies no guest
service and changes neither CMOS fixed-disk conventions nor option-ROM scanning.
Only the system ROM matched by SHA-256 and size receives this permission.
Unlisted images never inherit it from the board's scan-window fallback, and
ordinary BIOS entries without it retain the provider check. Machine files
cannot declare this capability or opt into it through `provides`.

Listed option ROMs use the same comparison and SHA-256 allowlist. Their
board, CPU minimum and required parts are mandatory, including under the escape.
An ATA resource mismatch reports `OptionRomCardMismatch`; other missing parts
report `FirmwareRequiresDevice`. CMOS fixed-disk types follow the provider
and the system BIOS's explicit `cmos-drive-types` capability, as described
under Derived configuration above.

The scan window governs firmware discovery and option-ROM placement, not the
board's physical gap decoding below the system BIOS. A system BIOS entry with
neither an extension nor a video scan window rejects every option ROM with
`FirmwareDoesNotScanOptionRoms`: `firmware X does not scan for option ROMs`.
Machine-file loaders accept run-only `AssemblyOptions` through their
`*_with_options` / `load_with_reader_and_options` APIs. Defaults stay strict;
allowing unlisted option ROMs retains the defining file/line and does not
bypass format, placement or compatibility checks.

Implementation diagnostics additionally pin these exact examples:

- `option ROM at 0xC8001 must be 2 KiB aligned` (`OptionRomMisaligned`).
- `option ROM at 0xC8000: expected 55AAh signature and a length byte`
  (`OptionRomBadHeader`).
- `option ROM at 0xC8000: length byte declares 1536 bytes, image has 2048 (must match and be nonzero)`
  (`OptionRomBadHeader`). Exact length prevents trailing bytes from hiding
  additional executable images outside the declared, checksummed module.
- `board "ibm-5170" adapter ROM space overlaps ram at 0xc8000`
  (`AdapterSpaceOverlapsRam`); a RAM load without RAM backing retains
  `ImageOutsideRam`.
- `option ROM image (sha256 …) is not a known image for board "ibm-5170"; known: xub-r638-at (open, vendor/xtide-universal-bios/fetch.sh)`
  (`OptionRomNotListed`).

**READ hardware reference:** *IBM Personal Computer AT Technical Reference*
(March 1984), System BIOS, **Adapters with System-Accessible ROM Modules**,
p. 5-11, specifies the 55h/AAh signature, length in 512-byte blocks,
modulo-256 byte sum of zero, 2 KiB scan steps and FAR CALL at offset 3.
It documents **C8000h through E0000h**, not the XT's F4000h limit.
[Original IBM scan](https://ftp3.us.freebsd.org/pub/misc/bitsavers/pdf/ibm/pc/at/1502494_PC_AT_Technical_Reference_Mar84.pdf).
**READ firmware:** pinned Kiselev `8088_bios/src/bios.asm`, `detect_rom_ext`
(lines 1390–1435), scans **[C8000h, F8000h)** with default CMOS; a setup bit
can shorten it to F0000h. `extension_scan` (lines 1532–1571) verifies the
signature, nonzero length and checksum, advances past the declared module,
rounds upward to 2 KiB, and returns offset 3 for FAR CALL. The earlier video
scan covers [C0000h, C8000h); that whole interval is isolated from RAM too.
Catalogue placement uses the default extension window. Custom NVRAM that
shortens the scan can make an otherwise valid F0000h–F7FFFh ROM undiscoverable;
`option_roms` records the default scan order, not execution under altered NVRAM.

Rule 13 rejects the raw r638 `ide_at.bin` (MEASURED byte sum B0h), which is the
intended behaviour: the fixture must pad and checksum it.

### Video ROM placement

The system catalogue stores **two distinct POST windows**, both with exclusive
ends. IBM 5170 BIOS entries scan `[C0000h, C8000h)` for video firmware and
`[C8000h, E0000h)` for extensions. READ by the supervisor: IBM AT BIOS listing
for the 06/10/85 edition, TEST1, “CHECK FOR THE ADVANCED VIDEO CARD” / “CHECK FOR
VIDEO ROM”, scans for 55AAh in 2 KiB steps through C7FFFh. The same video window
on the 1984-01-10 and 1985-11-15 catalogue entries is an inference from that
listing, not a separately read listing for those editions. READ directly in the
pinned Kiselev
`src/bios.asm:1127–1137`: DX=C000h, BX=C800h, `extension_scan`, followed by
`call far [67h]`; `extension_scan:1532–1571` verifies length/checksum and returns
offset 3. Kiselev's separate extension window remains `[C8000h, F8000h)`.
Landmark has neither window and rejects adapter ROMs in both areas. An unlisted
experimental BIOS retains only the previous extension-window fallback; it does
not establish a video scan capability. A missing video scan reports the actual
system BIOS identity, or “unlisted”, rather than the fallback extension window.
Unresolved system files defer the video
placement check until their exact loaded identity is available.

ROMs starting in the video interval use **only** the video's declared window;
there is no union that permits an image to straddle C8000h. Alignment, complete
mapped length, 55AAh header, exact size byte and checksum checks still apply.
A listed video BIOS must fit inside the system BIOS's video window even when
placed at an extension address. A listed extension option ROM cannot occupy
the video window. The board's read-only gap decode does not imply a firmware
scan capability.

`FirmwareEntry::IBM_VGA_1987` (`ibm-vga-bios-1987-32k`) identifies a user-supplied
IBM Display Adapter (ISA VGA, 1987) EPROM. SUPERVISOR-MEASURED: the full 32 KiB
file SHA-256 is `51b49761f72780a696c36f3d2e74a8168c1c99b865070f7f90b8e621a8b5ae17`;
bytes 0000h–1FFFh are zero and bytes 2000h–7FFFh form a valid 24 KiB option
ROM (size byte 30h, checksum zero). The catalogue owns `rom_offset=2000h`;
`ImageSource::RomContainer` retains all bytes for identity/pins and maps only
the suffix at C0000h–C5FFFh. It requires the register-level `vga`, not `vga-text`.
The entry is **Tested** for the IBM 5170 BIOS 1985-06-10 pairing below; this
does not qualify other system BIOS revisions or video modes.
Only this container is listed. A plain 24 KiB representation needs its own
measured SHA-256 and a second catalogue entry with size 24576 and offset zero;
the full-file hash cannot identify a cropped dump.

`FirmwareEntry::VGABIOS_0_9D_UNEMU_1_STD` (`vgabios-0.9d-unemu.1-std`) is the
open counterpart: the unemu fork of the LGPL VGABIOS, based on upstream 0.9d,
std build, release `v0.9d-unemu.1` (source revision
`40522278d5b703e637271696783721f4b5ea541f`, <https://github.com/unemu-org/vgabios>),
LGPL-2.1-or-later. Unlike upstream, its teletype output (INT 10h AH=0Eh and
AH=13h) sounds the speaker for BEL (07h), as the IBM PC/AT BIOS does: PIT
counter 2 in mode 3 with divisor 1331 (about 896 Hz) for 31/64 s, timed by
counting the port 61h bit 4 refresh toggles, with interrupts enabled during
the tone and port 61h and IF restored. `vendor/vgabios/fetch.sh` builds it
with Nix and pins the raw 32 KiB image, SHA-256
`b867f3846f44be63b1b10febc1ce79c9435f5d1233c67a0b63164ab48359a582`, header
`55 AA 40`, byte sum zero. It has no container offset, fills C0000h–C7FFFh and
requires the register-level `vga`. `machines/at-games-open.toml` maps it with
the open AT BIOS (unemu-at-bios, §2.4; the Kiselev AT-reset BIOS before
2026-11-15) and XTIDE at C8000h. The entry is **Tested**:
**MEASURED at CPU `060d446a883a9b2fc94b3ef91258cb4e6e98d916`** (2026-11-16), the
open AT BIOS POST calls the ROM, prints its sign-on ("Bochs VGABios
0.9d-unemu") and reports "EGA/VGA (Video BIOS Present)", and FreeDOS reaches
`A:\>`; the fork's INT 10h probe floppy reports all 33 checks `ok`, `tty.bel`
among them, and `fails:0000`; the headless speaker trace sees one tone from
the video BIOS (C000h) for that BEL, mode 3, divisor 1331, ending after
577,819 PIT clocks (0.484267 s; 31/64 s is 0.484375 s); a synthetic boot
sector sets mode 0Dh through INT 10h, then panning (AX=1000h), page 1
(AH=05h) and a CRTC byte start, and each scanned-out frame matches a decode of
planar memory pixel for pixel; it then sets mode 13h, and that frame matches
the chain-4 byte stream, one BIOS pixel and the default 256-colour DAC palette
with one AX=1010h entry.

`FirmwareEntry::VGABIOS_0_8A_UNEMU_1_STD` (`vgabios-0.8a-unemu.1-std`, the
fork's 0.8a-based ROM, `7570ef9c…5389`, last built by release `v0.8a-unemu.3`)
stays catalogued as **Superseded**, superseded by 0.9d-unemu.1: teletype BEL
sounds the speaker. It keeps the board contract and the source archive pin of
its own release (`vgabios-0.8a-unemu.3-src.tar`), so machines and images made
with it still start. Its `fetch` names `vendor/vgabios/fetch.sh` at revision
`4f73f3d06e120b42a93bbbc0f857f3fb9a1cb0ea` of this repository, the last one
that pinned `v0.8a-unemu.3`; that checkout rebuilds its ROM and archive.
New machine… lists it under its package name but never chooses it by
default, and the capabilities and settings sheet labels it "(superseded)".

| Open pinned result (`at_games.rs`, `open_vga_golden.rs`) | Value |
|---|---|
| FreeDOS prompt | `RetiredCapReached`, `15,216,991` retired, text SHA-256 `41ff081e1227af8f1a053df1b627b734de4032963cc0357c488cbbe67e01469e` |
| INT 10h probe | `Halted`, `0000:88A9`, `10,524,099` retired (`10,240,139` with the 0.8a-based ROM and its probe), text SHA-256 `df1f232b9cd4b8be1c849998ee11b844576611b05f9282a2094bbc70cb8a86d8` |
| Teletype BEL | one speaker start at C000:5866, mode 3, divisor 1331; off at C000:588F after 577,819 PIT clocks, within 1% of 31/64 s |
| Mode 0Dh scroll, then mode 13h | `Halted`, `10,688,267` retired (`10,730,175` with the 0.8a-based ROM); five frame SHA-256 values pinned in the test, unchanged |

The synthetic CLI POST test loads an offset-2000h container through the machine
file, proves FAR CALL/RETF with POST 10h/11h/20h, pins a RAM marker and checks
the programmed VGA sequencer. Its IRET-only INT 10h stub lets Kiselev's
immediate console calls return before the RTC checkpoint. The ignored
`user_rom_ibm_vga_bios_mode13_pattern`
uses both user ROM identities and a synthetic 1.2 MB boot disk: mode 03h, a
bounded text capture stage, INT 10h mode 13h, then a 320×200 byte-ramp pattern.
**SUPERVISOR-MEASURED at CPU `fe240a56ac2040e3deaebb2537e642137ee02c71`:**
with IBM 5170 BIOS 1985-06-10 on `preset:ibm-5170`, POST initializes this video
ROM and both INT 10h mode calls pass. The register-level card produces 80×25
text with 16-line characters, followed by the complete 320×200 graphics pattern.
The fixture uses derived CMOS, automatic display selection with the VGA ROM,
an `at-adapter` IDE controller, and its synthetic read-only 1.2 MB boot disk.
Only the boot program's `VGA!` magic/stage marker with CS=0 and IP in
7C00h–7DFFh accepts a sample; POST memory fill cannot impersonate the stage.

| Pinned result | Value |
|---|---|
| Stop | `Halted`, `0000:7C50` |
| Retired instructions | `23,463,327` |
| Text-plane SHA-256 | `707ba8a5e9a7f27f639e55cbc1a4375c86392f810c2742dc6aa0218ddc1adfab` |
| Graphics-frame SHA-256 | `825d409a82c363538f60248d866bb8462c28eb7ebdb0499bbd0188c6d0740f27` |

All four expectations are populated in the ignored user-ROM test. Cheap,
passive register/derived-geometry dumps remain value-only at stage 1 and at
the end. No proprietary bytes, screen text or images are stored. Missing user
ROMs follow the shared skip/require policy.

## 7. Recommendation

**(a) A hard disk soon on the AT machine.** Keep the Kiselev Xi 8088 BIOS as
it is. Add the **XUB AT build (`ide_at`, r638, GPL-2.0-or-later) as an option
ROM at C8000h** on a **new profile** next to the old one. Add an **ATA device**
at 1F0h–1F7h and 3F6h on IRQ14. This works for four reasons:

- the Kiselev IPL already falls back to INT 13h DL=80h;
- XUB installs its own INT 19h loader and chains INT 13h (READ);
- the AT build's defaults are exactly the standard primary channel (READ);
- upstream itself ships combined Xi 8088 + XUB ROMs.

The original investigation identified two prerequisites:

1. **x86jit string port I/O** (`INS`/`OUTS` with `REP`) in Real16 is now
   available at the current pin `060d446`, with machine integration tests.
   The historical measurement at `96a49a9` found it missing; an XUB boot
   remains a separate qualification.
2. **Port split at 3F6h.** The FDC's decode shrinks from 3F0h–3F7h to
   3F0h–3F5h plus 3F7h, so the IDE owns 3F6h.

The fallback, if the CPU seam is late, is the 8088 build `ide_xt`, which uses
`in ax,dx` loops. It defaults to an 8-bit XT-IDE at 300h, so it would need a
source build with AT defaults. Prefer waiting for the seam.

**(b) Phase B.** Decision-19 selects **the unemu SeaBIOS fork, `CONFIG_QEMU`**
on `generic-386-isa`. The board uses the pinned 128 KiB build from
`upc-o0j.9` and emulator-only fw_cfg for serial-console goldens first.
Register-level VGA is wired; SeaVGABIOS delivery follows. Writable shadow RAM, the high
BIOS copy, firmware-owned CMOS and CPU/device prerequisites are detailed in
§3.6. It provides AT services and builds with gcc (READ). The firmware
family is also planned for phases C/D on `i440fx-piix3`. Bochs legacy
`rombios` is the fallback, requiring VGA from its first run (§3.6). Neither
runs on the 286 board, so `ibm-5170` keeps the Kiselev BIOS permanently as
the smallest test bed.

**(c) An XT-class machine later.** Use an `ibm-5160` board with **GLaBIOS
`ARCH_5160`** as its default firmware. It explicitly targets the standard 5160
and emulators, and it is GPL-3.0. Pin the upstream release ROM by SHA-256,
because the build needs MASM. List **Kiselev `MACHINE_XT`** as a second entry
for differential runs; it builds with NASM like today's fixture, but upstream
has not tested it. The board waits for `xj-50w.2 (x86jit)`. The 8255, the XT
keyboard and the DIP-switch device work can land before the CPU model.

**Goldens.**

- Never switch a profile's firmware. **A new firmware is a new profile with new
  goldens.** The old `presets::bios()` profile stays byte-identical.
- The first bead re-expresses `presets::bios()` as `ibm-5170` + the Kiselev
  catalogue entry. It proves `MachineConfig` equality. Config is `Eq`, so this
  is a plain assertion, not an A/B run.
- The firmware hash becomes part of the machine identity, alongside the
  x86jit pin and the run policy.
- Board completion work (DMA2, reset line) proves the old goldens unchanged, or
  re-baselines deliberately with the reason stated.

## 8. Risks and open questions

Risks:

- **CPU qualification for (a).** The INS/OUTS prerequisite landed at
  `73ed5d6`. The XUB AT build may use another unlifted opcode, which only a
  boot will show.
- **XUB OS hooks.** XUB calls INT 15h 90h/91h with AL=00h. The Kiselev BIOS
  answers AH=86h with CF set for those (READ). ASSUMED: XUB ignores the result.
- **Conventional memory.** XUB full mode takes 1 KiB and the Kiselev BIOS
  reserves a 1 KiB EBDA. Upstream lists sharing them as a TODO (READ README).
  `MEM` output will differ in the new profile, which is expected.
- **The 3F6h/3F7h split** changes the FDC's decode in the new profile only.
  On real hardware 3F7h is shared: bit 7 is the floppy disk change, bits 0–6
  belong to the fixed-disk adapter. Bits 0–6 are not modelled.
- **SeaBIOS ISA requirements — resolved by the source spike.** fw_cfg,
  writable shadow/high copy, CMOS layout and 8254 read-back are required
  work, while debug port 402h is optional (READ, §3.6; decision-19). Runtime
  compatibility and interpreter performance remain unmeasured.
- **Windows 98 SE on SeaBIOS** has PnP and ACPI quirks and QEMU-version
  regressions (READ, secondary sources).
- **Section citations.** The Technical Reference section titles were read from
  tables of contents, not from the full scans.
- **GLaBIOS builds need MASM.** Pinning release binaries trades
  source-reproducibility for availability. An open assembler (JWasm/UASM) is
  unverified.
- **User-supplied ROMs** may trip vendor CMOS checksums without an NVRAM file.

Open questions for the maintainer:

1. **Naming — resolved (decision-18).** The board is `ibm-5170`; both DMA
   banks, empty-socket coprocessor I/O and the CPU-reset seam are now
   implemented, with remaining deviations recorded in §4.2.
2. **Unlisted firmware — resolved (decision-18).** A strict allowlist by
   default, with the run-only `--allow-unlisted-firmware` developer flag and
   a full-hash warning. The flag is absent from machine files and goldens;
   it does not bypass BIOS presence, placement, reset or memory isolation.
3. **XUB fixture.** Should the fixture build from a pinned SVN revision with
   NASM, or download r638 and pad and checksum it deterministically? Both pin
   two hashes.
4. **Phase B console — resolved (decision-19).** SeaBIOS SERCON first,
   configured through emulator-only fw_cfg; register-level VGA later (§3.6).
5. **XT default firmware.** GLaBIOS (tested on a 5160; MASM-built binary) or
   Kiselev `MACHINE_XT` (NASM; untested upstream)?
6. **NVRAM file.** Should a CMOS NVRAM file (from the `upc-6le.1` notes) be in
   schema v1 for user-supplied BIOSes, or wait for the first enthusiast board?
7. **386 board name.** The proposed name is `generic-386-isa`.

## 9. Implementation beads

Each bead is narrow; dependencies are in brackets. Board models replace `class`.
The generic AT comes first.

1. **Board model `ibm-5170` as data (generic AT).** `presets::bios()` becomes
   `ibm-5170` + firmware. The class `at` is derived. The board doc table lists
   modelled and missing 5170 parts with AT Technical Reference citations.
   - *Acceptance:* `MachineConfig` equality with today's preset for both floppy
     variants; every headless and slow golden unchanged.
   - *Depends on:* none.
2. **Firmware catalogue with a SHA-256 allowlist.** Add `FirmwareEntry` (id,
   kind, sha256, size, base rule, provides, requires, cpu_min, boards,
   distribution, status) and the `kiselev-xi8088-1.0.2` entry. Hashes are
   checked at image read. Errors 6–9 and 17.
   - *Acceptance:* each error has an exact-message test; a one-byte-changed ROM
     is rejected with the list of known entries; the goldens load through the
     catalogue.
   - *Implemented:* board-owned system BIOS catalogue in `assembly`, checked
     after image resolution and before ROM mapping; rules 6–9 and 17 have exact
     diagnostic tests. The CLI and window support the run-only
     `--allow-unlisted-firmware` escape hatch with a full-hash warning. Bare rigs
     skip assembly checks; the historical POST runner checks its intended BIOS
     explicitly. System BIOS and option ROM service claims now come from catalogue entries.
   - *Depends on:* 1.
3. **Description `[board]` and `[firmware]` sections.** `model` drives
   everything; `class` is rejected with a hint. Reserved CPU models (`8088`,
   `8086`, `v20`, `80386`, …) and reserved boards are rejected (rules 1–5).
   - *Acceptance:* the TOML and YAML of today's FreeDOS machine lower to a
     config equal to the preset; reserved values give tested messages.
   - *Depends on:* 2, and the description-loader bead from `upc-6le.1`.
4. **Option-ROM placement and scan.** Alignment, scan window, header, length
   and checksum (rules 12–14); RAM holes; ROM mapping; the effective scan order
   in `Assembled`.
   - *Acceptance:* a synthetic 2 KiB test ROM (inline bytes) is called by the
     Kiselev BIOS (the extension-init POST code is seen) in a slow test; old
     goldens unchanged.
   - *Implemented:* rules 11–14, catalogue-owned services and strict option-ROM
     membership, system-BIOS identification by its end at 1 MiB, read-only
     adapter space, and `AssembledMachine.option_roms`. A fixture-gated slow test
     checks a synthetic C8000h ROM's RAM marker and POST code between extension
     detection and completion. Existing golden expectations are unchanged.
   - *Depends on:* 2.
5. **x86jit request: Real16 string port I/O.** `INS`/`OUTS` bytes and words
   with `REP`, restartable per element, filed and landed in x86jit, then
   re-pinned here. Support landed at `73ed5d6`; machine tests
   cover short transfers. The 256-word sector transfer remains this later
   bead's acceptance test.
   - *Acceptance (here):* a machine test where `rep insw` reads 256 words from
     a mock port device; all goldens re-run green at the new pin.
   - *Depends on:* the x86jit bead.
6. **ATA device, isolated.** The task file at 1F0h–1F7h, device control and
   alternate status at 3F6h, IRQ14, master and slave, PIO. Commands: IDENTIFY
   DEVICE (CHS + LBA28), READ/WRITE SECTORS, READ/WRITE MULTIPLE, SET MULTIPLE
   MODE, INITIALIZE DEVICE PARAMETERS, SET FEATURES, RECALIBRATE, SEEK, EXECUTE
   DEVICE DIAGNOSTIC and SRST. Writes are volatile; journaling follows
   `upc-6le.3`. The coverage matrix cites ATA-2 and the AT *Fixed Disk and
   Diskette Drive Adapter*.
   - *Acceptance:* mock-bus tests per command and error path; a snapshot round
     trip.
   - *Depends on:* none.
7. **FDC/IDE port split.** With an `ide` card on the primary channel, the FDC
   decodes 3F0h–3F5h and 3F7h, and the IDE decodes 3F6h.
   - *Acceptance:* the unsplit overlap is rejected; presets without `ide` are
     unchanged.
   - *Depends on:* 6.
8. **XUB fixture.** Add `vendor/xtide-universal-bios/fetch.sh`, which builds or
   fetches a pinned AT build, pads it to 8 KiB, fixes the checksum and checks
   both hashes. Its README records the binary download centre's exact GPL
   version 2 statement (without an "or later" grant), separately from the
   source-header survey above. Add the catalogue
   entry `xub-r638-at` (provides `fixed-disk`; requires a CPU ≥ 80286 and
   `ata@1F0/14`). Only the script and README are committed in the vendor directory.
   - *Acceptance:* deterministic output and an offline rebuild when cached.
   - *Implemented:* r638 `ide_at.bin` is hash-checked, padded with 00h to
     8 KiB and checksummed at the last byte (50h). The output SHA-256 is
     `6409de912eab620ea144b79e723bed478814047dc2766fc19fd6e2e3690662d0`.
     `FirmwareEntry::XUB_R638_AT` records the `Listed` option-ROM identity,
     minimum CPU, fixed-disk service and primary ATA requirement. It remains
     part of the board's enforced firmware catalogue for option-ROM validation. Primary IDE assembly now satisfies the ATA requirement
     with the 1F0h task file, 3F6h control and IRQ14 wiring. Catalogue/recipe and
     fixture-gated hash tests cover the entry.
   - *Depends on:* 2.
9. **Fixed-disk provider rules.** Rules 10–11, and the CMOS fixed-disk type
   follows the provider (0 when it is an option ROM).
   - *Acceptance:* tests; the existing CMOS seed is byte-identical.
   - *Status:* implemented. Rules 10–11 retain their existing validation;
     catalogue-owned `cmos-drive-types` gates IBM AT standard/extended types.
     Unsupported geometry is refused, option-ROM providers leave types zero,
     and preset seed equality plus the existing machine-file/preset equivalence
     cover the unchanged FreeDOS seed. Provider and checksum tests cover the
     synthetic system-BIOS path; XUB is tested with an IDE disk.
   - *Depends on:* 2, 6, 8.
10. **Hard-disk image fixture.** A script builds a partitioned FAT16 image (MBR,
    one partition, a marker file) with open tools; the hash is pinned and the
    image is gitignored.
    - *Acceptance:* identical bytes across runs.
    - *Depends on:* none.
11. **Profile `ibm-5170` + XUB + IDE: FreeDOS sees `C:`.** A new description or
    preset next to the old one, with a new slow golden (floppy boot, `DIR C:`
    lists the marker).
    - *Status:* the new ignored `ide_freedos_golden` boots FreeDOS from A:
      and types `DIR C:` through the keyboard. `machines/at-ide.toml` extends
      the original floppy profile with the separately pinned Kiselev AT-reset
      image (since 2026-11-15 the open AT BIOS, unemu-at-bios, with the
      `mono` display switch; §2.4), configured XT+ at C8000h and a readonly
      615/4/17 FAT16 master.
      The completed listing contains `UNEMUHDD` and `MARKER.TXT`, 29 bytes.
      MEASURED with PIT-driven refresh, rate 8/13 and CPU `44403dd`: prompt
      retired 15,777,577, prompt FNV-1a `49bbec2d36c0bbf0`; after DIR retired
      19,748,107 at virtual time 21,000,000, final FNV-1a `fb811e1f40836da7`.
      The golden checks the completed listing, retired instruction counts,
      screen hashes and POST, with port tracing disabled.
      POST is `01 02 03 04 05 06 07 08 09 10 12 20 21 22 24 25 30 31 40 41 42 43 00`.
      MEASURED with the open AT BIOS at CPU `060d446a`: the same retired
      counts (prompt 15,775,575; after DIR 19,746,105 at virtual time
      21,000,000), logical hashes and POST codes as the Kiselev AT-reset image
      at that pin; the screen FNV-1a values move with the clock row at the
      prompt, `Current time is 12:00:18.78 am` before and `12:00:18.83 am`
      after (prompt `a0e8f388c37aa180`, final `7aae65a75645f427`).
      XT+ detects `UNEMU ATA-2 PIO DISK`; IDENTIFY/READ MULTIPLE and floppy
      sector reads complete without unsupported instructions.
      No CPU or device implementation changes are included.
      Mutations fail with XUB removed (no C:), a byte data window (XUB's
      `REP INSW` fails at 1F0h), a renamed marker in a separately pinned image,
      and the original unpatched BIOS (no DOS prompt). Removing IRQ14 routing
      preserves all pins: this XT+ build polls and has no `MODULE_IRQ`.
    - *Acceptance:* the new golden is green; the old goldens are byte-identical.
    - *Depends on:* 4, 5, 7, 8, 9, 10.
12. **`ibm-5170` completion: second 8237** at C0h–DFh with pages 89h–8Fh.
    - *Status:* implemented (READ `crates/assembly/src/board.rs:104–108`,
      `assemble.rs:304`): the AT word bank is constructed with
      `Dma8237::new_at_word`. Existing assembly tests check both controllers
      and their page latches (`assemble.rs:1064–1096`); isolated word-bank
      tests cover port decoding and pages
      (`crates/devices/src/dma8237.rs:724–760`).
    - *Acceptance:* device tests; goldens unchanged, or re-baselined with a
      stated reason (the BIOS writes C0h for I/O delays).
    - *Depends on:* 1.
13. **Phase B firmware spike (`upc-o0j.8`).**
    - *Status:* done (source investigation, 2026-11-11). §3.6 records the
      requirements, evidence labels, pinned sources and decision-19. Follow-up
      work is filed as `upc-o0j.9`–`upc-o0j.16`, with `upc-o0j.5`/`.6`
      amended; R1/R3 are `xj-srh.16 (x86jit)` / `xj-srh.17 (x86jit)`,
      R2 extends `xj-srh.4 (x86jit)`. No firmware build or boot was run.
    - *Depends on:* the phase B gap analysis, 1–3.
14. **XT device set, device level.** The 8255 PPI with DIP-switch derivation,
    the XT keyboard interface (83-key, set 1, PB6/PB7), a single-mode 8259
    check, the XT page-register map and the NMI mask at A0h, citing XT Technical
    Reference §1. The `ibm-5160` board stays reserved.
    - **Implemented at device level (2026-11-11; reviewed against manuals):**
      `Ppi8255` models Mode 0, directions, mode-set latch reset and C bit
      set/reset; Modes 1/2 are ignored atomically. `XtSystemPpi` composes it
      with SW1 derivation, 83 physical set-1 positions, a receive latch and IRQ1,
      PB6 reset/AAh and PB7 receiver acknowledgement/clear. PA has no SW1 path:
      it reads the keyboard shift register, or output latch in diagnostic mode.
      SW1 is accessible only through PC0–3, with PB3 selecting the nibble.
      SW1 bit 1 means an installed 8087; bit 0 means normal POST, with an explicit
      `loop_on_post` flag clearing it. Drive counts must be one through four;
      zero is rejected with `XT floppy drive count must be between one and four
      (SW1 cannot encode zero drives)`. Default SW1 is 7Dh. `XtNmiMask` stores
      bit 7 at all A0h–AFh aliases (high enables); no NMI source or CPU delivery
      is modelled. `XtDma` wraps the existing byte 8237, masks pages to 20
      address bits and decodes 80h–9Fh. XT controller ports 10h–1Fh alias
      00h–0Fh, and XT PPI ports 64h–6Fh alias 60h–63h. The shared AT controller
      and generic four-port PPI retain their existing decode. Keyboard reset
      keeps an unread LS322 byte until PB7 clears it, with AAh queued behind
      that byte. The existing single PIC is tested with
      BIOS ICW1/2/4 values 13h/08h/09h, and with ICW4 omitted.
    - **READ coverage:** *IBM Personal Computer XT Technical Reference*
      (6280089, March 1986), §1 “System Board”: “DMA” (1-14), “I/O Address
      Map” (1-24), “8255A I/O Bit Map” (1-27/28); §2 “Coprocessor”; §4
      “83-Key Keyboard” (4-4) and “Block Diagram” (4-5); §5 BIOS listing
      (01/10/86), POST and `KBD_RESET`. These confirm the single SW1 pack
      (no SW2), PC0–3 switch mux, PB0/PB1 speaker/timer controls, PB2 spare,
      PB4/PB5 active-low check enables, PB6 clock hold, PB7 clear, PC4 spare,
      PC5 OUT2 and PC6/PC7 check status. The switch table defines motherboard
      banks, display and drive-count-plus-one encodings. The BIOS confirms
      08h / ~20 ms / C8h / 48h reset and AAh/IRQ1, NMI enable/power-on mask,
      channel-2 page at 81h and a POST write to 83h. The Intel 8255A datasheet,
      Mode Selection/Mode 0/Bit Set-Reset/Basic Operation, confirms input reset,
      mode-set output clearing, output-latch reads and illegal control reads.
    - **Coverage limits (ASSUMED):** exact cleared PA and spare PC4 read
      values are 00h/zero; illegal control and NMI reads return FFh. The 74LS670
      details are absent from the extracted text: conventional page registers
      81h/82h/83h/87h serve channels 2/3/1/0; 90h–9Fh mirror 80h–8Fh and unused
      page latches retain nibbles. Hardware uses a 4×4 74LS670 register file
      decoded on A0–A1, so board wiring may replace this 16-latch compatibility
      model. The 83-key buffer holds 16 queued codes, excluding the system
      receive latch. The FFh overrun code is READ in both BIOS INT 9 listings
      (11/08/82 and 01/10/86), which branch to the buffer-full beep. The exact
      position is ASSUMED: overflow replaces the last queued byte with FFh,
      retaining the oldest 15, and repeated overflow coalesces that marker.
      Clock low must last 1,193 virtual PIT clocks (~1 ms) before release queues
      AAh. This is an ASSUMED compatibility threshold: the 83-key text gives
      no minimum, and 12.5 ms in §4 “Reset (Hex FF)” (4-26) applies to 101/102
      keys. CPU steps are not cycles: at rate 1/1 the BIOS's 10,582-iteration
      LOOP yields 10,582 clocks, below the former 14,915-clock threshold.
      The chosen threshold rejects immediate mode-set/release and permits that
      loop. Qualification must ensure the BIOS hold exceeds it at every
      supported XT CPU rate. No serial bit timing or physical BAT delay is
      modelled.
      Queued bytes wait one PIT clock after PB7 release so IRQ1 has a low
      interval. PIT/status pins and control getters are unwired; refresh,
      audio and NMI generation remain outside this device-only scope.
    - **Isolation:** mock-bus tests cover literal SW1 values and POST/FPU bits,
      direction and BSR effects, all 83 make/break pairs, threshold boundaries,
      the BIOS reset/clear sequence, queue overflow and snapshot bounds,
      literal BIOS hold at rate 1/1, unread receive-latch preservation,
      PPI/controller register aliases, control/status pins, page aliases and
      transfer wrap, NMI aliases, single PIC initialization, snapshots and trace Off/Ring membership. No assembly,
      board catalogue, parser, CPU or frontend implementation is changed.
    - *Acceptance:* mock-bus tests; validation keeps rejecting the board and
      the 8088 model with tested messages.
    - *Depends on:* 1, 3.
15. **XT firmware and first XT golden.** The GLaBIOS `ARCH_5160` release ROM
    pinned by hash; the Kiselev `MACHINE_XT` NASM build as a second entry. The
    golden boots the FreeDOS 8086 floppy on `ibm-5160`.
    - *Depends on:* 14, `xj-50w.2 (x86jit)` and its re-pin.
16. **User-supplied ROM entries.** Hash-only catalogue entries, even/odd parts
    and the `FirmwareNotListed` listing. No chipset models are implied.
    - *Implemented:* three IBM 5170 entries (1984-01-10, 1985-06-10,
      1985-11-15), alternative 1984 layouts, TOML/YAML parts with origins,
      assembled identity and swapped/wrong-chip diagnostics. All stay Listed;
      defaults are unchanged. Synthetic tests cover the implementation; ignored
      user-ROM tests check identity and measure 1985-06-10 POST execution on the
      I286 machine-profile CPU. The POST test uses pinned RTC, volatile derived
      NVRAM, MDA, automatic display selection, an AT fixed-disk adapter and a
      read-only 1.2 MB FreeDOS A: floppy. It accepts date/time with Enter, polls
      for the DOS prompt and prints stop/checkpoints/retired count/prompt verdict/
      MDA-plane hash without screen text. Pinned expectations qualify clean POST
      and floppy boot for 1985-06-10 at CPU `fd67927` with locally supplied
      firmware; other revisions remain unqualified.

## Sources

- Kiselev `8088_bios`: <https://github.com/skiselev/8088_bios>, pinned at
  `a4af458` (2026-07-16): `src/config.inc`, `bios.asm`, `misc.inc`,
  `cpu.inc`, `floppy2.inc`, README.
- GLaBIOS: <https://github.com/640-KB/GLaBIOS>, `src/GLABIOS.ASM` and
  `MAKE.BAT` at `68c65a5` (2026-07-23).
- Super PC/Turbo XT BIOS 3.1: <https://www.retrocmp.de/software/pcxtbios/readme.txt>,
  <https://www.phatcode.net/downloads.php?id=101>.
- XTIDE Universal BIOS: <https://www.xtideuniversalbios.org/> and SVN trunk
  `Src/Main.asm`, `Src/Initialization/Initialize.asm`,
  `Src/Initialization/Interrupts.asm`, `Src/Handlers/Int13h.asm`,
  `Src/Device/IDE/IdePioBlock.asm`; binaries
  <https://www.xtideuniversalbios.org/binaries/r638/> (2026-06-09).
- Bochs `rombios`: [pinned BIOS sources](https://github.com/bochs-emu/Bochs/tree/44104eeaff16107e478ea8bd9c55bd8601a80191/bochs/bios),
  commit `44104eeaff16107e478ea8bd9c55bd8601a80191` (2026-11-08),
  `rombios.c`, `rombios.h`, `Makefile.in`; Windows 98 SE guide
  <https://bochs.sourceforge.io/doc/docbook/user/guest-win98.html>.
- SeaBIOS: [rel-1.17.0 sources](https://github.com/coreboot/seabios/tree/b52ca86e094d19b58e2304417787e96b940e39c6),
  commit `b52ca86e094d19b58e2304417787e96b940e39c6` (2025-06-10), also
  QEMU's `roms/seabios` pin. §3.6 cites this release, including
  `docs/Memory_Model.md`, `src/fw/`, `src/hw/`, `src/romlayout.S`,
  `src/Kconfig`, `vgasrc/` and `scripts/buildversion.py` (READ).
- QEMU: [pinned sources](https://github.com/qemu/qemu/tree/29760efaf96bbe5928f8111881eddbf5ac06439e),
  commit `29760efaf96bbe5928f8111881eddbf5ac06439e` (2026-11-10,
  v11.2.0-rc0). `roms/config.seabios-128k`, `roms/config.vga-isavga`,
  `hw/i386/isapc.c:30–186`, `pc.c:403–485,880–892`, `pc_sysfw.c:222`,
  `x86-common.c:1005–1009,1063,1114–1120`, `vga-isa.c:88`, `loader.c:1232–1240`,
  `docs/specs/fw_cfg.rst` (READ); [PC documentation](https://www.qemu.org/docs/master/system/i386/pc.html).
  Its `isapc` supplies fw_cfg (including DMA at 514h), writable BIOS/ROM RAM,
  port 92h and 8042 A20, 8254, ISA VGA, two IDE channels and FDC (READ).
  ASSUMED: default 486 CPU has CPUID.
- QEMU prebuilt images at that commit (MEASURED by the source spike,
  fetched-file hashes; not executed): `pc-bios/bios.bin`, 131072 bytes,
  SHA-256 `3dfd946d0c03ab0e022f84f10c3eb5f1dd507761f73e7d8067511ba35a10f776`;
  version `rel-1.17.0-0-gb52ca86e094d-prebuilt.qemu.org`, GCC 15.1.1,
  binutils 2.44. `pc-bios/vgabios.bin` (`isavga`), 38912 bytes,
  SHA-256 `a7ea86bb06a58ff969fd0942e73b8eae00cb58e4c90bccbd900f6a3a01f54fbb`.
- Windows 98 SE on QEMU: <https://bugs.launchpad.net/bugs/1819289> (2019),
  <https://mail.coreboot.org/pipermail/seabios/2012-May/003887.html>.
- fysnet i440fx BIOS: <https://fysnet.github.io/i440fx>.
- Intel 440FX: <https://en.wikipedia.org/wiki/Intel_440FX>; 440BX:
  <https://en.wikipedia.org/wiki/Intel_440BX>; datasheet 290549-001 (May 1996).
- IBM Technical References: AT 1502494 (March 1984) and 6183355 (March 1986) on
  bitsavers; XT 6139821 (1986) tables of contents.
- x86jit pin `96a49a9`: `x86jit-core/src/lift/mod.rs`, port I/O note on
  `ins`/`outs`.
