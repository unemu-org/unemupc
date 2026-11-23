# Open unchained 360x240 VGA test

The permanent fixture in `vendor/vga-modex-test/` exercises a BIOS mode 13h
followed by a directly programmed 360x240 unchained mode, without DOS or
proprietary guest bytes. Its golden is
`crates/pc-harness/tests/vga_modex_test_golden.rs`. Build details, licence,
image hash and serial transcript are in
[the fixture README](../../../vendor/vga-modex-test/README.md).

Evidence labels: **READ** means inspected source or documentation;
**MEASURED** means executed here with the pinned CPU; **ASSUMED** means a
hypothesis requiring further evidence.

## References

- **READ:** IBM, *VGA/XGA Technical Reference Manual*, May 1992, VGA Function
  ([manual](https://bitsavers.trailing-edge.com/pdf/ibm/pc/cards/IBM_VGA_XGA_Technical_Reference_Manual_May92.pdf)):
  Fig. 2-25 (status 1), Figs. 2-32 and 2-36 (Map Mask and Memory Mode),
  Figs. 2-40–2-50 and 2-58–2-60 (CRTC timing, protection and offset),
  Figs. 2-53–2-54 (start address), Figs. 2-61 and 2-64–2-65 (byte/word/
  doubleword address outputs), and Fig. 2-75 (write mode 1).
- **READ:** Michael Abrash, *Graphics Programming Black Book*, Chapter 32,
  [Listing 32.1](https://www.phatcode.net/res/224/files/html/ch32/32-02.html)
  (John Bridges' published 360-dot timing), and Chapter 47,
  [Listing 47.1](https://www.phatcode.net/res/224/files/html/ch47/47-02.html)
  (240-row vertical timing). The fixture combines these standard parameters:
  MSR E7h; CR00–07 = 6B 59 5A 8E 5E 8A 0D 3E; CR09=41h,
  CR10–17 = EA AC DF 2D 00 E7 06 E3. CR08 and CR0C/0D start at zero;
  CR18=FFh, with overflow bits leaving line compare outside the display.
- **READ:** National Semiconductor *PC16550D* datasheet, Line Control,
  Line Status and Transmitter Holding registers (COM1 reporting).

## Pattern and checks

Each page holds 360x240 logical pixels, 21,600 bytes per plane. Page starts
are 0 and 21,600. At logical coordinates `(x,y)`, page `p` has:

- For `y < 200`, index `((x XOR y) AND 63) + 64*p`.
- For `y >= 200`, index F1h when `x mod 8 < 5`, `y mod 8 < 7`, and either
  `x mod 8 = 0` or `y mod 8` is 0, 3 or 6. Otherwise index `2 + 64*p`.
  These are repeated E-like glyphs with one-pixel strokes and gaps.
- Finally, copy the 64x32 rectangle at `(16,24)` to `(224,112)` on the same
  page. Source and destination do not overlap.

The guest visits every x residue modulo four with Map Mask 1, 2, 4 and 8.
It performs the rectangle copy with all four maps enabled and write mode 1:
read a source byte, invert the CPU byte, then write the destination. Only
latch data can produce the expected copy. All planes are cleared first.
The golden computes the image directly from logical coordinates; it does
not call the VGA renderer, reuse its address calculations or record its
current output hash.

| Check | What a pass establishes | Limit |
| --- | --- | --- |
| CLI/HLT and exact COM1 transcript within 100,000,000 retired instructions | Guest finishes 60 flips; bit 3 of 3DAh does not get stuck for this timing and clock | No assertion of cycle-exact frame rate or physical monitor compatibility |
| 360x240 indices and repeat (2,2), every reported frame | Byte-addressed stride/start, each plane, one-pixel features, and latch copies agree with the logical oracle | No claim for mixed serializers or other address modes |
| 30 captured frames per page | Both nonzero and zero start addresses reach scanout repeatedly | Frame captures cannot prove absence of within-frame tearing |
| Pinned image and CPU revision | Repeatable open input and known CPU | Retired count is reported as evidence, not separately pinned as an assertion |

The harness advances in virtual-time slices of 5,000 ticks and captures on
each new COM1 page digit. It asserts that no report was skipped. A mismatch
reports the first eight coordinates, total differing pixels, and counts
by x modulo four, retaining the first failed frame while still running to
the final report so rendering failures do not hide a retrace hang.

IBM Fig. 2-53 explicitly says the start address latches at retrace start.
The guest writes during one retrace and waits through the next before
reporting that page. There are 60 flips over 120 retrace starts; the counter
at physical 0600h counts flips. No BIOS teletype modifies the pattern.

## Results and scope

**MEASURED, 2026-11-23:** with the unchanged VGA model from main `6e0def18`
and CPU revision `6cb817b86c1fbf463f7aab8c0da1ce0eec3db0b7`, the final
image `834038855a9cc34af9b55ca288b086786b37d76209139180a77ba69572b37bc0`
builds twice identically. The golden passes: 60/60 frames (5,184,000 pixel
comparisons), 30 per page, no stripes or grid corruption, exact final
report, and `Halted` after 10,817,666 retired instructions. This reproducer
does **not** reproduce the reported application's fault. No VGA model fix
or new device isolation tests are justified by this result.

**MEASURED, development check:** the initial harness/guest reported a page
at the end of the same retrace in which its start address was written.
Frame 2 then differed in 83,025 pixels, starting at `(0,0)` (00h versus
40h): the previous page was still displayed. The guest nevertheless
finished all 60 reports. Waiting through the following retrace, as required
by Fig. 2-53, removed this failure without any model change. This was a
reporting-point error in the test, not a reproduced VGA defect. The final
fixture also replaces the initial grid with one-pixel glyph strokes.

## Next discriminating observations

These are hypotheses, not claims about any uninspected guest:

- **READ:** `Vga::read_byte` selects odd/even reads using GR05 bit 4,
  independently of SR04 write addressing. **ASSUMED:** a program that only
  unchains the sequencer, while retaining odd/even reads, could load its
  copy latches from different offsets. Compare GR05 with SR04 at copies.
- **READ:** `Vga::graphics_scanout` derives address shift from CR14 bit 6
  and CR17 bit 6; chain-4 does not select the scanout mode. **ASSUMED:** a
  different CR14/CR17 combination or offset would change spacing. Capture
  these registers together with SR04 and CR13 at the first bad frame.
- **READ:** the module coverage table leaves count-by-two/four variations,
  some mid-frame register changes and mismatched serializers unmodelled.
  `Vga::tick` latches start at retrace start; the renderer observes that
  bundle. **ASSUMED:** a different programming sequence, split or panning
  transition could expose a gap not covered by this whole-frame pattern.
- **READ:** `Vga::raster_timing` uses the selected 28 MHz domain and all
  timing overflow bits, with fallback for unusable totals. **ASSUMED:**
  another programmed total/retrace boundary, or a loop waiting on something
  besides bit 3, could explain a later hang. A short register/I/O trace and
  halted CPU location would distinguish that from this passing case.

Any further open reproducer should use only those observed register and
access sequences, never proprietary bytes. This result does not establish
that the external application's rendering or freeze is fixed.
