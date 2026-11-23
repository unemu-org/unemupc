# Open 80386 protected-mode ROM

`i386-pm-test.asm` is project-owned source under AGPL-3.0-or-later.
`build.sh` assembles a 128 KiB firmware image entirely offline, using nasm
3.01 from the project devShell. Generated `i386-pm-test.bin` and `build/`
are gitignored. No BIOS, DOS, external source or proprietary bytes are used.

```sh
bash vendor/i386-pm-test/build.sh
```

Requires Bash, nasm, awk, wc, tr, mktemp, mv and sha256sum or shasum.
The script checks the exact size and SHA-256 before replacing the output.
Flat binary output contains no timestamps. Deliberately remeasure the golden
when changing the source or assembler; the image hash is pinned in the script.

The target is `generic-386-isa`, with 640 KiB conventional and 15 MiB extended
RAM, no FPU, the normal board devices and a COM1 card. The image is preloaded
in writable shadow RAM at E0000h and copied into a separate read-only region
at FFFE0000h. Reset starts at FFFFFFF0h, makes a near jump in the high copy,
saves EDX and far jumps into the F0000h shadow. All subsequent execution and
descriptor writes use the shadow or ordinary RAM.

The program checks the reset signature, identification flags without CPUID,
port-92h A20, USE32 segmentation and a limit #GP, interrupt/trap gates,
8259A-remapped PIT IRQ0, and ring-3 INT/IRQ transitions using a 386 TSS.
It returns through a 16-bit protected segment, clears PE, reloads the real
segments and reports the failure count before CLI/HLT. Each check prints one
line on port 402h. COM1 is present for the board's normal fw_cfg configuration;
the ROM does not transmit on it.

The fixture lane builds this ROM. The fast golden
`crates/pc-harness/tests/i386_pm_test_golden.rs` uses a harness-owned
`MachineConfig` and explicit permission for unlisted firmware. No catalogue
entry or normal machine-file validation rule is changed. Missing output skips
locally and fails when `UNEMUPC_REQUIRE_FIXTURES=1`.

See [the design and measured evidence](../../backlog/docs/design/i386-pm-test.md)
for the complete transcript, memory layout, limits and CPU revision.
