# test386.asm — third-party CPU-test ROM (test data)

`test386.bin` is built by [`fetch.sh`](fetch.sh) from
[barotto/test386.asm](https://github.com/barotto/test386.asm), pinned at commit
`cfd052d1e64d5375dea5a681c1eadeed64ceda2c`. It is a diagnostic ROM that exercises
an Intel 386 and reports progress by writing a checkpoint ("POST") byte to port
`0x80` at each stage, with direct ASCII output at `0x402`.

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

At CPU pin `5a17667764080c2fe7a33d446584a3e175b7923d`, all three models
produce POST `00 01 02 03 04 05 06 08` and no ASCII bytes. They retire 793,775
instructions, then refuse `MOV CR0,EAX` (`0F 22 C0`) at F000:267C with typed
`LegacyExecution`. The ROM enables PE and PG together in POST 08h, before its
protected-mode tests. No protected-mode or paging pass is claimed. The test
has a 1,000,000-instruction bound and runs normally (about 0.03–0.06 seconds).
The 486 variants retain their different CR0 reset bits but reach the same stop.
When paging is enabled upstream, advance the golden after reviewing progress.
The retained bare scheduler rig also captures POST at 80h.

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
