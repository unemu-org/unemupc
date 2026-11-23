# unemupc

A deterministic, low-level emulator (LLE) of **period IBM-compatible PCs**,
providing the PC core and its standalone command line. The
CPU core is the `x86jit` x86 recompiler, consumed as a pinned git dependency.

The 286 AT is the first machine profile on a longer road: a period PC that runs
DOS games now, and Windows 98 SE games on an emulated 3dfx Voodoo later
(`backlog/decisions/decision-16*`).

## Status

The AT 286 boots open firmware and FreeDOS and runs DOS games. What works, in
detail and with evidence, is in [`backlog/docs/status.md`](backlog/docs/status.md).
Open work is tracked in [beads](https://github.com/steveyegge/beads), whose
database travels with the repository: after cloning, install `bd` (1.3.1 or
newer), run `bd bootstrap`, then `bd ready --exclude-type epic`; see
[`backlog/docs/dev-setup.md`](backlog/docs/dev-setup.md#beads-bd).

Priorities, in order: **testability** (every device tested in isolation on a
mock bus, every machine-level feature behind a deterministic golden test),
**swappability** (CPU, devices and graphics behind traits, machines as
declarative files) and **documentation** (doc-comments that cite the datasheets,
and a knowledge tree under `backlog/docs/`).

## Features

### The machine

- **CPU:** Intel 80286 in real and protected mode through `x86jit`; optional
  80287 socket. A20 through the 8042 and port 92h, CPU reset through the 8042
  and triple fault, with the CMOS shutdown-byte resume.
- **AT system board:** two 8259A PICs, 8254 PIT, two 8237A DMA controllers,
  8042 keyboard controller, MC146818 RTC/CMOS (derived, deterministic contents),
  port 61h speaker and parity control, POST port with page-register readback.
- **Memory:** 640 KiB conventional plus extended memory; Lo-tech hardware EMS
  with code execution from the page frame.
- **Video:** MDA, CGA and a register-level VGA (text, EGA and VGA graphics
  modes including unchained 256-colour), with an open VGA BIOS.
- **Storage:** NEC µPD765 floppy controller with 360K, 720K, 1.2M and 1.44M
  drives; a drive takes every medium real drives of its type read (720K disks
  in a 1.44M drive, 360K disks in a 1.2M drive with double stepping). AT IDE
  (ATA) fixed disks, with the XTIDE Universal BIOS as option ROM. Disks are
  sparse, journalled and roll back by generation.
- **Sound:** PC speaker and a Sound Blaster 16 (DSP 4.05, 8/16-bit DMA) with an
  OPL3 FM synthesizer.
- **Input and serial:** AT keyboard, PS/2 auxiliary mouse, 16550 UART with a
  Microsoft serial mouse.
- **Firmware:** the open **unemu AT BIOS** (a fork of Sergey Kiselev's
  8088_bios), the open Xi 8088 build for the XT-style profiles, and
  user-supplied IBM 5170 BIOSes identified by hash. ROMs are never committed:
  a catalogue recognises them by SHA-256.
- **Determinism:** virtual time is measured in retired instructions; there is
  no wall clock in the execution path, so every run can be a golden test.

### Machine profiles

`machines/*.toml` describe whole machines declaratively: board, CPU, memory,
cards, firmware and drives. `at-games-open.toml` is the open DOS games machine
(unemu AT BIOS, VGA, SB16, serial mouse, EMS, IDE disk, A: 1.2M and B: 1.44M);
`at-games.toml` is the same machine on the user-supplied IBM BIOS.

### Standalone program and front end

`unemupc` runs machines headless, checks machine files and manages persistent
media journals. The desktop, image library, registries and recipes live in
[unemu](https://github.com/unemu-org/unemu). The `setup-unemu` action moved
there with the front end.

## Downloads

Binaries for Linux x86-64 and arm64 (glibc 2.35+), and macOS on Apple silicon and Intel, are attached to each
[release](https://github.com/unemu-org/unemupc/releases), with `SHA256SUMS`;
each archive holds `unemupc` (standalone PC core). The release notes say how to check them and how to open the unsigned macOS
binary. On other platforms, build from source as below.

## Getting started

```sh
nix develop                                   # toolchain and fixture tools (or let direnv load it)
bash scripts/ci-gate.sh fixtures              # build the open firmware and fixture images
cargo run --release -p unemupc -- --boot
cargo run --release -p unemupc -- run --machine machines/at-freedos.toml --print mda
cargo test --workspace                        # unit, golden and integration tests
```

The full command reference is in [`backlog/docs/commands.md`](backlog/docs/commands.md);
the toolchain in [`backlog/docs/dev-setup.md`](backlog/docs/dev-setup.md);
playing DOS games in [`machines/README.md`](machines/README.md#playing-dos-games).
### Building without Nix

- Rust 1.99 or newer (`rust-version` in `Cargo.toml`), for example through
  rustup, and a C compiler.
- `cargo build --release -p unemupc --bin unemupc`.

The firmware and disk fixtures that the tests use are built by
the scripts under `vendor/`, which expect the devShell's tools (`nasm`,
`mtools`, `curl`, `unzip`).

## Documentation

- [`backlog/docs/status.md`](backlog/docs/status.md) — what works, with evidence
- [`backlog/docs/architecture.md`](backlog/docs/architecture.md) — crates, layers and swap seams
- [`backlog/docs/design/`](backlog/docs/design/) — design notes, including hardware, machine files and persistent media
- [`backlog/decisions/`](backlog/decisions/) — decision records
- [`AGENTS.md`](AGENTS.md) — entry point for coding agents and the project's hard rules

## CPU core: x86jit

The CPU is not vendored. `x86jit` lives in its own repository with its own
tracker and gates; this project depends on it as a git dependency pinned to an
exact revision of its `main`, and golden traces are valid only against that
revision. Real-mode and 286 support live there under `CpuMode::Real16`.

## Third-party components

- **Ultimate Oldschool PC Font Pack** by VileR: the MDA and VGA bitmap fonts,
  CC BY-SA 4.0 ([notice](crates/pc-core/assets/fonts/NOTICE.md)).
- **Nuked-OPL3** 1.8 by Nuke.YKT: the OPL3 FM generator is a Rust translation,
  LGPL-2.1-or-later ([licence](crates/devices/src/opl3/LICENSE)).
- **Rust crates**: `scripts/third-party-licenses.sh` writes their licences
  for the exact `Cargo.lock` into `THIRD-PARTY-LICENSES.md`, which ships with
  the release binaries. `unemupc --about` print the notices.

## License

[AGPL-3.0-or-later](LICENSE). Firmware forks and fixtures carry their own
licences (see each `vendor/*/README.md`).
