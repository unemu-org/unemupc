---
id: decision-19
title: Phase B firmware is unmodified SeaBIOS behind an emulator-only fw_cfg device
date: 2026-11-11
status: Accepted
deciders: Mikołaj Mikołajczyk
---

# decision-19 - Phase B firmware is unmodified SeaBIOS behind an emulator-only fw_cfg device

## Status

Accepted — 2026-11-11. Amends rule 5 of [[decision-18]] (phase B firmware).
Board wiring, shadow RAM, the separate pristine high copy and the Listed
SeaBIOS catalogue entry are implemented for `generic-386-isa` (2026-11-23).
Execution/boot qualification still waits for `xj-srh.12 (x86jit)` and a re-pin.
Part of phase B of [[decision-16]]. The requirement list is in
[`backlog/docs/design/firmware-and-machine-classes.md`](../docs/design/firmware-and-machine-classes.md)
(phase B firmware requirements); the CPU-core side is in
[`backlog/docs/design/protected-mode-gaps.md`](../docs/design/protected-mode-gaps.md).

## Context

[[decision-18]] chose SeaBIOS for the ISA-only 386 board `generic-386-isa`,
"with its serial console first". A source survey of SeaBIOS rel-1.17.0
(`b52ca86e`, the revision QEMU ships) showed that an unmodified build cannot
do that on a plain ISA board:

- The only build target with ISA fallbacks is `CONFIG_QEMU`. Without the QEMU
  firmware configuration port (fw_cfg, 510h/511h) it sizes RAM from CMOS, but
  it has no serial console (the port comes only from the `etc/sercon-port`
  file), and it loads no video or option ROM: it clears C0000h and takes ROMs
  only from PCI or from fw_cfg files.
- POST runs in 32-bit flat protected mode and writes into its own
  F-segment, so C0000h–FFFFFh must be writable RAM preloaded with the image
  (QEMU's `isapc` maps the BIOS as RAM because there is no chipset to unlock
  shadowing).
- Reboot copies a pristine image from just below 4 GiB, then triple-faults the
  CPU.

QEMU's `isapc` machine runs this build because it provides fw_cfg and writable
shadow RAM. Its window below 4 GiB is an alias of that same writable RAM, so
SeaBIOS there finds no pristine copy to restore on reboot.

## Decision

1. **Phase B firmware is an unmodified SeaBIOS `CONFIG_QEMU` build**, built
   reproducibly from a pinned release with our own `.config` (serial console
   on; boot menu, MP table, PIR table, SMBIOS, ACPI, PCI BIOS, USB, SMM off) and
   listed in the catalogue by hash, like the other open firmware.
2. **The `generic-386-isa` board gets an fw_cfg-compatible configuration port**
   at 510h/511h (no DMA interface), modelled on QEMU's documented protocol.
   It is documented as an emulator-only device, not period hardware. It serves
   the files SeaBIOS needs (`etc/sercon-port`, `vgaroms/*`, `genroms/*`,
   `bootorder`) from the machine description, plus the legacy keys it reads.
   No other board gets it.
3. **The board maps the firmware as writable shadow RAM plus a pristine high
   copy:** writable RAM at C0000h–FFFFFh with the BIOS image preloaded at the
   top, as QEMU `isapc` does, and, unlike `isapc`, a separate read-only copy of
   the image just below 4 GiB, so a reboot that keeps RAM can restore the
   image. Adapter-space validation allows this for this board only; `ibm-5170`
   keeps read-only adapter space.
4. **Serial console first, register-level VGA later**, as decision-18 rule 5
   intended; the VGA ROM then arrives through fw_cfg.
5. **Bochs rombios stays the fallback**, but it needs a VGA ROM and a
   register-level VGA from the first run, because it has no serial console.

## Alternatives considered

- **Bochs rombios (legacy build) as the phase B firmware.** Fewer machine
  changes (16-bit, pre-placed option ROMs, no fw_cfg, no high copy), but no
  serial console, so the register-level VGA would be needed first; kept as the
  fallback.
- **SeaBIOS with a local patch** (serial port from Kconfig, ISA ROM scan)
  instead of fw_cfg. Keeps period hardware only, but adds a patch we must
  maintain against every SeaBIOS release; rejected.

## Consequences

- (+) An unmodified, widely tested firmware; QEMU's prebuilt images can serve
  as a cross-check.
- (+) Headless serial-console goldens before any VGA work.
- (−) One device that no real board had; it must stay clearly labelled and
  confined to this board.
- (−) Board work: shadow RAM, the high BIOS copy, a CMOS layout owned by the
  firmware entry (QEMU's RAM-size bytes), the 8254 read-back command, a CPU
  reset on triple fault, and A20 through port 92h (SeaBIOS never uses the 8042
  for A20).
- (−) CPU-core work beyond 386 protected mode: the full 386 real-mode
  instruction set, big real mode, and a 386 identity without CPUID.

## Trigger to revisit

SeaBIOS needs more fw_cfg features than a small port can serve; the
interpreter cannot run SeaBIOS's POST mode switches at a usable speed; or a
period-accurate phase B board becomes a goal.
