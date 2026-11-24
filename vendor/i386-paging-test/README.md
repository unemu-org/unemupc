# Open 386/486 paging ROM

`i386-paging-test.asm` is project-owned AGPL-3.0-or-later source. It uses no
BIOS, DOS or external guest bytes. Build the 128 KiB image offline with the
project devShell's NASM 3.01:

```sh
bash vendor/i386-paging-test/build.sh
```

The builder verifies the exact size and SHA-256 before replacing its output;
flat binaries have no timestamps. Output and temporary builds are ignored.
Use the same Bash/coreutils requirements as `../i386-pm-test/build.sh`.
Deliberately remeasure the hash and transcript after any source change.

The ROM runs on `generic-386-isa` and both `generic-486-isa` CPU variants,
with the ordinary motherboard devices, 16 MiB map, port-92h A20 and port-402h
ASCII. It enters USE32 from the high reset copy via writable F0000h shadow.
Page directories, page tables, descriptors, IDT and TSS all live in RAM.
No shipped machine file or catalogue rule changes.

Checks cover identity/alias mappings, PDE/PTE A and PTE D, CR2 and page-fault
error codes, ring-3 PDE/PTE U/S and R/W checks, writable user memory, equal
CR3 reload and a different directory. The 486 variants additionally execute
INVLPG after a PTE change and fault supervisor writes to read-only user and
supervisor pages with CR0.WP=1. The 386 explicitly reports those two 486-only
facilities unavailable. All variants finish with `failures 00000000` and HLT.

`crates/pc-harness/tests/i386_paging_test_golden.rs` pins the image hash,
CPU revision, exact transcript and retired count, with whole and 17-step
bounded runs. Missing images skip locally and fail with
`UNEMUPC_REQUIRE_FIXTURES=1`. `scripts/ci-gate.sh fixtures` builds the image;
the existing vendor-file rule in `affected-tests.sh` selects its consumers
and the golden tier.

See [transcripts, memory layout and limitations](../../backlog/docs/design/i386-pm-test.md#two-level-paging-rom).
