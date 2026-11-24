# Open Intel486 DX protected-mode ROM

`i486-pm-test.asm` is project-owned source under AGPL-3.0-or-later.
`build.sh` assembles a 128 KiB firmware image entirely offline with nasm 3.01
from the project devShell. Generated `i486-pm-test.bin` and `build/` are
ignored. No BIOS, guest OS or external source is used.

```sh
bash vendor/i486-pm-test/build.sh
```

Requires Bash, nasm, awk, wc, tr, mktemp, mv and sha256sum or shasum.
The script checks size and SHA-256 before replacing the output. The flat
binary has no timestamps. Source or assembler changes require deliberately
remeasuring the image hash and golden transcripts.

The target is `generic-486-isa`, with either `80486dx` or `80486dx-cpuid`,
the integrated FPU, 640 KiB conventional and 15 MiB extended RAM, normal
board devices and COM1 for fw_cfg. The same ROM handles both CPU variants.
It executes the pristine high reset copy, preserves reset EDX, then enters
writable shadow RAM and flat USE32 protected mode.

Checks cover reset EDX/CR0, 32-bit BSWAP, register/memory XADD, successful and
failed CMPXCHG, CR0.WP/AM and EFLAGS.AC read-back, EFLAGS.ID presence detection,
CPUID leaves 0/1 when present, and INVD/WBINVD/INVLPG. WP protection needs
paging; the ROM tests no paging, CPL-3 alignment exception or FERR routing.
NASM groups CPUID and CMPXCHG under its Pentium CPU directive; the source
emits only the documented 486 instructions and common 386 instructions.

The fixtures CI lane builds this ROM. The fast golden in
`crates/pc-harness/tests/i486_pm_test_golden.rs` checks the exact port-402h
transcript, retired count, no deliveries, repeatability and scheduler/run-chunk
agreement. Missing output skips locally and fails with
`UNEMUPC_REQUIRE_FIXTURES=1`. No catalogue entry is needed for this test ROM.
See [the transcript contract](../../backlog/docs/design/i386-pm-test.md#486-extension).
