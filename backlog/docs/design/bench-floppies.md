# Speed-ceiling boot floppies

`tools/bench-floppies/` builds ten 1.44 MB boot floppies. Each one runs one of
the speed-ceiling loops from
[machine-files.md §Measured speed ceilings](machine-files.md#measured-speed-ceilings)
a fixed number of times, between a START line and an END line on the text
screen. They boot through the BIOS like any boot floppy, so the same guest work
can be timed on unemupc and on another emulator, for example 86Box. The ROM
fixtures only run on unemupc's bare boards.

Evidence labels: **READ** means inspected source or documentation;
**MEASURED** means executed here with the pinned CPU; **ASSUMED** means a
hypothesis requiring further evidence.

**MEASURED:** they boot with SeaBIOS on generic-486-isa. The maintainer has
also booted and timed an earlier build on 86Box with an Award BIOS on a
Pentium II board. **ASSUMED:** they boot with other PC BIOSes too; the boot
sector and prelude use only the BIOS services listed under
[What the images do and need](#what-the-images-do-and-need).

## Building

```sh
bash tools/bench-floppies/build.sh [output directory]
```

The script needs NASM (it is in the devShell). It writes `<case>.img` for every
case plus `SHA256SUMS`. Without an argument it writes to
`${TMPDIR:-/tmp}/unemupc-bench-floppies`. The images are generated and are
never committed. The build is reproducible: the boot sector carries a fixed
volume serial, and two builds give identical bytes.

`BENCH_PASSES=<n>` (a positive decimal number) replaces every case's pass count
and prints a warning on standard error. Only the tests use it, to keep their
boots short.

The test `crates/pc-harness/tests/bench_floppies/` covers the images four
ways:

- **Build.** Every case builds with the published counts. The test checks the
  image size, the boot signature, the BPB, both FATs and the START text with
  its count.
- **Tie to the ROM fixtures.** Each floppy loop is compared byte for byte with
  the loop in its ROM fixture under `crates/pc-harness/tests/fixtures/`.
  - For the user-mode cases only two things are masked: the near-CALL
    displacement to the near callee, which moves because the loop end differs,
    and the far-call target, which moves with the load address.
  - For the register loop the four body instructions must be equal. The ROM
    closes them with JMP and the floppy with JNZ.
- **Exact V86 run.** `paged-v86-iopl0` is booted with 1000 passes on
  generic-486-isa with SeaBIOS. The test checks:
  - the START line;
  - that the loop counters are at their start values on loop entry;
  - the regime halfway through;
  - that the CPU stands on the first instruction after the loop after exactly
    the promised retired count;
  - the END line, and the registers, user data and trap counter left behind.
- **Exact paged32 run.** `paged32` gets the same checks with one pass of
  65,536 iterations, so the protected and paged register-loop setup also runs
  in CI.

Every loop that waits for progress fails, naming the retired count and showing
the screen, when the guest halts with interrupts off instead of hanging. The
tests skip with a message when NASM or the SeaBIOS/FreeDOS fixtures are
missing, unless `UNEMUPC_REQUIRE_FIXTURES` is set.

## Running on unemupc

```sh
cargo run --offline --release -p pc-harness --example bench_floppy -- <output directory>/*.img
```

The probe loads `machines/generic-486-freedos.toml` (SeaBIOS) and replaces the
A: image with each floppy in turn. There is no hard disk. It runs uncapped in
1 ms slices of virtual time, as an unthrottled front end would. For each image
it prints:

- the iteration count;
- the instructions retired between seeing START and seeing END;
- the loop instructions alone;
- the host seconds and the rate;
- how many regime samples landed inside the case's loop regime.

The probe asserts every check the tests make, except the exact count. The
window is slice aligned. It begins up to one slice (16,500 instructions at the
board's nominal clock) after START, holds the case setup at its start, and ends
up to one slice after the END line. Its retired count therefore differs from the
loop instructions by at most about one slice. In `paged-v86-iopl0` a regime sample
can land in the ring-0 port monitor, which is expected; at least one sample per
case must be in the loop regime.

## Timing on another emulator

**ASSUMED** procedure, apart from the Award BIOS / Pentium II boot noted
above:

1. Configure a 486 or Pentium II class machine with any BIOS, a VGA card,
   640 KiB conventional memory and a 1.44 MB drive A:. Boot from A: first. A
   hard disk is not needed.
2. Attach `<case>.img` as A: and power on.
3. Start a stopwatch when row 0 shows
   `UNEMU-BENCH <case> START count=<n> A20=on`.
4. Stop it when row 2 shows `UNEMU-BENCH <case> END count=<n>`. The machine
   then halts with interrupts off; reset it to run again.
5. The emulator's rate is the loop instructions from the table below divided by
   the stopwatch seconds.

The floppy motor is switched off at START, so the drive stopping is a second
cue. An emulator that limits the guest to its configured clock measures that
clock, not its host limit, unless the configured clock is faster than the host
can deliver. A slow configuration can take several minutes per case.

## Cases

| Image | Count (iterations) | Retired per iteration | Loop instructions |
|---|---:|---|---:|
| `real16.img` | 491,520,000 | 5, plus 2 per 65,536 | 2,457,615,000 |
| `protected32.img` | 353,894,400 | 5, plus 2 per 65,536 | 1,769,482,800 |
| `paged32.img` | 262,144,000 | 5, plus 2 per 65,536 | 1,310,728,000 |
| `flat32.img` | 393,216,000 | 5, plus 2 per 65,536 | 1,966,092,000 |
| `flat-paged32.img` | 275,251,200 | 5, plus 2 per 65,536 | 1,376,264,400 |
| `flat-paged32-cpl3.img` | 26,000,000 | 26 | 676,000,000 |
| `flat-paged32-cpl3-farcall.img` | 24,000,000 | 26 | 624,000,000 |
| `paged-protected16-cpl3.img` | 22,000,000 | 26 | 572,000,000 |
| `paged-v86.img` | 2,600,000 | 251 (ten bodies) | 652,600,000 |
| `paged-v86-iopl0.img` | 2,200,000 | 251 (ten bodies, one trapped IN) | 552,200,000 |

`flat-paged32` and `flat-paged32-cpl3` (near calls only) are the two mixes the
Windows 95 plan measures ([windows95-on-486.md](windows95-on-486.md) §3).

**READ** (the sources, checked by the ROM comparison test): the loop bodies are
the ROM fixtures' own. A ROM can loop forever, but a timed boot has to stop, so
each loop gets a counter:

- **`ceiling.asm`** (real16, protected32, paged32, flat32, flat-paged32). The
  closing JMP becomes JNZ on the body's own DEC CX. An outer DEC ESI/JNZ counts
  passes of 65,536 iterations.
- **`ceiling-user.asm`** (the CPL 3 and V86 cases). EBP already counts bodies,
  so the closing JMP becomes CMP EBP/JB, which adds one instruction per pass.
  The faulting IN of `paged-v86-iopl0` does not retire; the monitor's nine
  instructions do.

The code moves from ROM at F8000h to RAM at the load address 10000h. The setup
stays at offset 0, the callee at +1000h and the loop at +2000h, and segment
bases, page-table entries and V86 code segments move with it. **ASSUMED:**
running from RAM instead of ROM does not change the rate on unemupc. Both
are cached guest memory to the CPU core, and nothing in the loop writes to
code.

## What the images do and need

- **Boot sector.** It carries a FAT12 BPB; the payload sits in the volume's
  reserved sectors, so the disk is also an empty FAT12 floppy. It loads 32
  sectors to 1000:0000h with INT 13h function 02h, one sector per call, so no
  call crosses a track or a 64 KiB DMA page. A failed read is followed by a
  reset (function 00h) and a retry, up to five attempts.
- **Geometry.** The geometry is that of the 1.44 MB medium (18 sectors, 2
  heads). It is not asked of the BIOS, because INT 13h function 08h reports the
  drive's largest format, which for a 2.88 MB drive would be 36 sectors per
  track.
- **Real-mode prelude, before START.** This is the only time the BIOS is used.
  1. INT 10h mode 03h.
  2. A20 is opened through the 8042 (command D1h, data DFh, then the null
     command FFh) and through port 92h. The port 92h write is skipped when bit
     1 already reads as set, which also covers boards without the port (they
     read FFh).
  3. A20 is checked by comparing 0000:0500h with FFFF:0510h, both restored
     afterwards. The START line reports the result.
  4. The floppy motor is switched off (port 3F2h = 0Ch), both 8259 mask
     registers are set to FFh, NMI is masked (port 70h bit 7, selecting the
     read-only CMOS status register D) and FS and GS are cleared.
  5. The START line is printed with INT 10h teletype output.
- **After START.** CLI, and no BIOS calls. The END line is stored straight
  into B8000h text memory, then CLI/HLT.
- **CPU.** A 386 or later. The CPL 3 and V86 cases exit through UD2 (invalid
  opcode), which reaches a ring-0 interrupt gate.
- **Video.** A colour text adapter at B8000h (VGA or EGA). Monochrome
  adapters show nothing.
- **Memory used.**
  - 0500h, the A20 check, restored afterwards.
  - 0800h–8FFFh: GDT, LDT, page tables, IDT, TSS and stacks.
  - 10000h–13FFFh: the payload.
  - 40000h–4FFFFh: user data and the user stack.
  - 100500h, the A20 check, only when the gate is open. Without memory there
    the check still tells the two addresses apart.

  The flat cases point ESP at 90000h. The EBDA, video memory and ROMs are not
  touched, and paging identity-maps only the first MiB.
- **Interrupts the count does not show.** Maskable interrupts and NMI are
  masked. System management interrupts are not: on the other emulator, for
  example a board BIOS's USB legacy emulation, they add time there that no
  instruction count shows.

## Measured on unemupc

**MEASURED 2026-11-27**, ARM64 macOS host class (`Darwin arm64`), release
build, CPU pin `75ef58a2eee181c148d2c7689eacfea7d946fbf2`, generic-486-isa with
SeaBIOS, 1 ms slices. The host was shared and its load was not controlled.

The current images, one run each. Uptime load averages (1/5/15 minutes) were
5.52/7.66/7.82 before the run and 4.98/6.04/7.04 after it. Every regime sample
landed in the loop regime.

| Case | Window retired | Seconds | Million instructions/s |
|---|---:|---:|---:|
| real16 | 2,457,615,247 | 21.868 | 112.384 |
| protected32 | 1,769,469,383 | 22.828 | 77.512 |
| paged32 | 1,310,715,151 | 24.017 | 54.574 |
| flat32 | 1,966,092,258 | 25.274 | 77.790 |
| flat-paged32 | 1,376,254,383 | 24.572 | 56.009 |
| flat-paged32-cpl3 | 675,996,531 | 22.398 | 30.181 |
| flat-paged32-cpl3-farcall | 624,001,059 | 28.744 | 21.709 |
| paged-protected16-cpl3 | 571,999,358 | 28.309 | 20.206 |
| paged-v86 | 652,591,438 | 26.670 | 24.469 |
| paged-v86-iopl0 | 552,194,834 | 28.066 | 19.675 |

An earlier build had the same loops but a prelude without the NMI mask and the
FS/GS clear, and lacked the two flat-paged cases. It was measured three times
per case. Uptime load averages were 8.53/10.91/10.89 before the first run and
5.57/7.28/8.63 after the third. Retired counts were identical in all three runs.
One regime sample of 34 per run in `paged-v86-iopl0` landed in the port
monitor.

| Case | Seconds (3 runs) | Million instructions/s (3 runs) |
|---|---|---|
| real16 | 25.296 / 21.655 / 23.622 | 97.154 / 113.491 / 104.038 |
| protected32 | 26.754 / 22.619 / 27.743 | 66.139 / 78.229 / 63.781 |
| paged32 | 25.562 / 26.851 / 29.407 | 51.277 / 48.815 / 44.572 |
| flat32 | 27.858 / 29.198 / 31.199 | 70.575 / 67.336 / 63.017 |
| flat-paged32-cpl3-farcall | 29.623 / 30.051 / 33.638 | 21.065 / 20.765 / 18.551 |
| paged-protected16-cpl3 | 27.351 / 27.716 / 30.018 | 20.913 / 20.638 / 19.055 |
| paged-v86 | 26.677 / 27.566 / 27.643 | 24.462 / 23.674 / 23.608 |
| paged-v86-iopl0 | 26.687 / 28.418 / 26.633 | 20.692 / 19.431 / 20.733 |
