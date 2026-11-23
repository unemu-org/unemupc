# Open VGA Mode X test

`vga-modex-test.asm` is project-owned, AGPL-3.0-or-later source. It boots
without DOS, sets BIOS mode 13h, then programs unchained 360x240 graphics.
Only source, this README and `build.sh` are committed. The 1.2M floppy
`vga-modex-test-1200.img` and `build/` are gitignored.

The boot sector loads four sectors (2–5 on track 0, head 0) at `0800:0000h`.
The program selects the 28 MHz clock, unlocks CRTC R0–R7, programs 90 bytes
per row and 240 logical rows, disables chain-4 and odd/even writes, selects
byte addressing, and clears all four planes with Map Mask 0Fh. It draws two
distinct pages through individual Map Mask bits, including one-pixel glyph
strokes, then copies a rectangle on each page with write mode 1. The byte
written by the CPU is deliberately inverted after loading the latches.

COM1 (3F8h, 115200 baud, 8N1) reports:

```text
vga-modex-test
pattern ready
010101010101010101010101010101010101010101010101010101010101
frames 60 done
```

Each digit names a displayed page. Each iteration waits for bit 3 of 3DAh
to clear then set, increments the word at physical `0600h`, and writes
CR0C/CR0D. IBM Fig. 2-53 latches the start address at retrace **start**, so
a write during that pulse takes effect at the following retrace. The guest
waits for that following pulse to end before reporting the page. Thus 60
reported flips span 120 retrace starts, with no wall-clock dependency. It
ends with CLI/HLT and the frame counter at 60. Only COM1 is used for output;
BIOS teletype output would change graphics memory.

## Build and reproducibility

```sh
bash vendor/vga-modex-test/build.sh
```

Run in the project devShell. Requires Bash, nasm (measured with 3.01), `od`,
`awk`, `tr`, `wc`, `mktemp`, `mv`, and `sha256sum` or `shasum -a 256`.
The build is offline. NASM flat output carries no timestamps; padding is
zero, the boot signature is 55AAh, and the size is exactly 1,228,800 bytes.
Assembly fails if the program exceeds its four loaded sectors. The script
checks size, signature and this SHA-256 before replacing the output:

```text
834038855a9cc34af9b55ca288b086786b37d76209139180a77ba69572b37bc0
```

The CI fixtures lane builds it. The permanent golden is
`crates/pc-harness/tests/vga_modex_test_golden.rs`; the pattern definition,
evidence and limitations are in
[the design note](../../backlog/docs/design/vga-modex-test.md).

## Register sources

- Michael Abrash, *Graphics Programming Black Book*, Chapter 32,
  [Listing 32.1](https://www.phatcode.net/res/224/files/html/ch32/32-02.html):
  John Bridges' public-domain 360x480 timing parameters, including the
  28 MHz clock and 360-dot horizontal timing/offset.
- Abrash, Chapter 47,
  [Listing 47.1](https://www.phatcode.net/res/224/files/html/ch47/47-02.html):
  240-row Mode X vertical timing (CR09=41h repeats each logical row twice).
  We combine that vertical setup with the 360-dot horizontal parameters;
  the assembly implementation and drawing pattern are our own.
- IBM, *VGA/XGA Technical Reference Manual*, May 1992, VGA Function,
  Figs. 2-25, 2-32, 2-36, 2-50, 2-53–2-54, 2-58, 2-60–2-65 and 2-75:
  status, plane selection, addressing, timing, start latch and write modes.
