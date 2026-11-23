# Open VGA Mode X test

`vga-modex-test.asm` is project-owned, AGPL-3.0-or-later source. It boots
without DOS, sets BIOS mode 13h, then programs unchained 360x240 graphics.
Only source, this README and `build.sh` are committed. Both 1.2M floppies
and `build/` are gitignored. The separate `vga-pas-test.asm` fixture covers
mode 12h palette writes with PAS=1, without changing the Mode X image.

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

## Mode 12h palette compatibility regression

The same build also creates `vga-pas-test-1200.img` (1,228,800 bytes, 55AAh
boot signature), pinned to:

```text
d58e531d4a29e0232484f76f26d4504fee8e9df837b251343fae867ec1286490
```

Its boot sector uses the same four-sector loading convention. After BIOS
mode 12h, it sets DAC entries 0–63 to `(i, 63-i, i XOR 21)` and writes
attribute entries 0–15 to identity with pairs `(index OR 20h, index)` at
3C0h, leaving PAS set throughout. It draws sixteen 40-pixel-wide vertical
bars over all 480 rows through individual plane masks, reports
`vga-pas-test: bars ready` on COM1 and halts. All bytes are project-owned;
this reproduces an observed Windows 3.x vga.drv access sequence without
containing driver code.

The existing CI fixtures lane builds both images through `build.sh`.
`media_fixtures.rs` checks their geometry, signatures and pinned hashes;
the mode-12h golden in `vga_modex_test_golden.rs` checks every displayed RGB
against logical bar coordinates. IBM's IPAS access restriction and our
explicit compatibility policy are documented in the design note.
