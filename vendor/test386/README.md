# test386.asm — third-party CPU-test ROM (test data)

`test386.bin` is built by [`fetch.sh`](fetch.sh) from
[barotto/test386.asm](https://github.com/barotto/test386.asm), pinned at commit
`cfd052d1e64d5375dea5a681c1eadeed64ceda2c`. It is a diagnostic ROM that exercises
an Intel 386 and reports progress by writing a checkpoint ("POST") byte to port
`0x80` at each stage. The configuration requests ASCII at `0x402`, but the
pinned print routine truncates that immediate port to `0x02` (see below).

## Provenance and licence

- Upstream: <https://github.com/barotto/test386.asm> (originally by Jeff Parsons /
  pcjs.org, extended by Marco Bortolin).
- Licence: **GPL-3.0-or-later** (see the upstream `COPYING`).
- Upstream source is unchanged. A generated configuration include selects the
  board diagnostic ports; the ROM is used as read-only test data only. It is
  assembled with `nasm` and loaded by the emulator's integration test. It is not
  linked into, copied into, or distributed with this project's source.

## Not committed

Neither the upstream clone (`test386.asm/`) nor the build product
(`test386.bin`) is checked in — both are `.gitignore`d. Run `fetch.sh` (which
needs `git` and `nasm`, both in the flake devShell) to produce `test386.bin`
locally. The 65,536-byte image is checked against SHA-256
`9cb8dddc091a73a8b7b94e7529671318a957dc645fc152be8dcbd5bb734d057c`.
The integration test skips itself when the image is absent unless
`UNEMUPC_REQUIRE_FIXTURES=1`.

## Board configuration

`fetch.sh` shadows only `configuration.asm` in a temporary include directory:
`POST_PORT=0x80` and `OUT_PORT=0x402`. All other upstream settings stay at their
pinned defaults: `CPU_FAMILY=3`, `TEST_UNDEF=0`, `LPT_PORT=0`, `COM_PORT=0`,
`LPT_STROBING=0`, `COM_PORT_DIV=1`, `BOCHS=0`, `IBM_PS1=0`, `DEBUG=0`,
`ROM128=0`. The output file replaces a local symlink instead of rewriting its
target. No upstream source, generated include, listing or ROM is committed.

The same 386-defined-behavior image runs on `generic-386-isa` (80386DX) and
`generic-486-isa` (early DX and CPUID DX). There is no upstream 486 family
configuration; no 486-specific AC, CPUID or undefined-flag qualification is
claimed. The 64 KiB ROM occupies F0000h–FFFFFh with the board's high reset copy
at FFFF0000h–FFFFFFFFh. Tests fill E0000h–EFFFFh with FFh to satisfy the
board's 128 KiB decode. Tests use the explicit unlisted-fixture assembly option,
leaving shipped firmware catalogue and scan policy intact. No SeaBIOS runs.

## Current measured result (2026-11-25)

At CPU pin `ab193c1591e3876db0e5042282f1bc1871b7d75c`, the 80386DX
completes with POST
`00 01 02 03 04 05 06 08 09 20 21 22 0b 0c 0d 0e 0f 10 11 12 13 14 15 16 17 18 19 1a 1b 1c e0 ee ff`,
**79,664,383** retired instructions and the completion HLT at **00D0:FE7C**
(next EIP FE7Dh). The previous five-million-step budget ended during EE;
the golden now permits 100 million steps. Both 486 variants retain their
486-correct row-11 stop, stream and **1,413,097** retired instructions
recorded below. No board change or ROM patch was needed.

The ARPL at `src/test386.asm:1487`, 00D0:B51C, now continues to B522h
without a fault: FFF2h and BX=2 have equal RPLs, so read-only DS is accepted,
the selector stays unchanged and ZF clears. Checkpoints pin both sides.
The full stream and FFh completion are asserted separately from the 486 stops.

POST 18–1C covers BOUND, XCHG, ENTER, LEAVE and VERR/VERW. E0 skips
TEST_UNDEF=0; EE runs arithmetic/logical cases whose output upstream asks
the user to compare manually. FFh proves execution completion, not that
manual comparison. `src/print_p.asm:81–82` encodes `out OUT_PORT,al` with
an eight-bit immediate: the pinned bytes at C104h are `E6 02`, so configured
402h becomes DMA port 02h. Zero debug-console bytes are expected throughout;
no arithmetic transcript is claimed. ROM128-only checks remain excluded.

## Historical result (2026-11-24)

At CPU pin `6a38ae4964555495c2b5c82ef72fea34e686c0b4` (2026-11-24),
the 80386DX completes all 74 paging rows and the subsequent PTE.D check,
then passes POST 12–16 and stops in POST 17. Its stream is
`00 01 02 03 04 05 06 08 09 20 21 22 0b 0c 0d 0e 0f 10 11 12 13 14 15 16 17`,
with zero ASCII bytes and **1,429,164** retired instructions. The next
CPU-policy/diagnostic mismatch is `src/test386.asm:1487`: at 00D0:B51C,
`63 1D 00 00 02 00` (`ARPL word [20000h],BX`) raises #GP(0), although
the selector's RPL already equals BX's and the diagnostic expects no write
or fault in the read-only DS. The CPU deliberately checks write permission
before comparing RPLs. Upstream's `BOCHS=1` skips this check; the pinned
image retains **BOCHS=0**. No physical-chip result is claimed for this case.

Both 486 entries retain POST
`00 01 02 03 04 05 06 08 09 20 21 22 0b 0c 0d 0e 0f 10 11`, zero ASCII bytes
and **1,413,097** retired instructions. Row index 11 at
`src/tests/paging_p.asm:204` faults correctly with #PF(7) on
`MOV dword [0049F000h],DEADBEEFh`, 00AB:9F88. PDE 1004h=00003025h and
PTE 327Ch=0009F025h: both A bits set, D clear. This is **486-correct** under
Intel486 PRM §9.9.14; the unchanged 386 diagnostic requires both A bits clear.
On the 386 those fault-time values are now 00003005h/0009F005h and the
handler continues. There is **no configuration knob for 486 A-bit behavior**:
`CPU_FAMILY` only controls undefined-behavior tests, and the handler's A-bit
checks are unconditional. BOCHS does not change them.

Both distinct stops reach the shared error HLT at 00D0:FE89. The goldens
sample the cause before the handler, pin the final delivery and tables,
and retain the ROM hash. No board defect, ROM patch or FFh completion is
claimed. ROM128-only task-switch and 16-bit V86-gate tests remain excluded.
The bound is 5,000,000 virtual-time units. Page-flag updates explicitly
reload CR3, so the new per-vCPU TLB needs no diagnostic workaround.
The bare scheduler rig still captures POST at 80h.

See [measured coverage and CPU findings](../../backlog/docs/design/protected-mode-gaps.md)
and `crates/pc-harness/tests/test386_golden.rs` for the pinned stop contract.

## Reproducible and offline builds

`fetch.sh` checks the output hash, so changes in assembler output cannot silently
change golden fixtures. A mismatch reports the file and both hashes. Check the
pinned sources and toolchain; deliberately rebuild and review the goldens'
expectations before updating the expected output hash.

The script uses portable size checks and either `sha256sum` or `shasum -a 256`
(on macOS); the flake devShell supplies the build tools. When the clone already
contains the pinned commit, rebuilding skips the network. A missing clone or
commit is fetched as before.
