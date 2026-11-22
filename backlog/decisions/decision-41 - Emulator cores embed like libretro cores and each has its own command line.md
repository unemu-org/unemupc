---
id: decision-41
title: Emulator cores embed like libretro cores and each has its own command line
date: 2026-11-18
status: Accepted
deciders: Mikołaj Mikołajczyk
---

# decision-41 - Emulator cores embed like libretro cores and each has its own command line

## Status

Accepted — 2026-11-18. The maintainer stated the principle, the core's own
command line and the names; this record writes them down with the rules
that follow from them. Design step of `upc-dfs.26`. Builds on
[[decision-40]] (front ends drive cores through one contract crate),
[[decision-5]] and [[decision-14]] (virtual time), [[decision-7]]
(swappability behind traits) and [[decision-22]] (cycle-ready seams). The
inventory of what a core touches of the host today, the allow list, the
storage objects of contract 0.2, the command line of `unemupc` and the
ordered steps are in `backlog/docs/design/core-contract.md` §12. The
questions of §12.13 were answered the same day with their
recommendations: `unemu` is one binary (window and command line); the
front end keeps only `run --machine`, and `--boot`, `--bios` and `machine
check` live in `unemupc` only; a libretro wrapper waits for contract 1.0,
the NES core first; the journal leaves `media` for the PC core now; the
front end's host directories move to `unemu`.

## Context

The maintainer's words, 2026-11-18:

- "The cores themselves should be written to have as few system
  dependencies as possible, so anyone can integrate them easily. A bit like
  RetroArch."
- Each core has its own small command line, so the core can be run and
  tested without the front end.
- The front end's binary is called `unemu`. Each core's binary is called as
  its core: `unemupc` for the PC core, `unemunes` for the NES core. Today
  the desktop binary is named `unemupc`; it becomes `unemu`.

[[decision-40]] put one contract crate between front ends and cores and
kept the picture, the sound, input and time on the front end's side of it.
It left two things with the core: a session directory the core writes
into, and content handed over as an open file. It also said nothing about
what a core's own crates may depend on.

Measured for this decision (core-contract.md §12.2):

- The contract crate has no dependency. Ten of the eleven crates of the
  emulation layer (`device`, `bus`, `cpu`, `video`, `audio`, `devices`,
  `chipset`, `machine`, `testkit`, `unemu-pc-core`) name no file, no
  environment variable, no host clock, no thread and no standard stream
  in production code, and no emulation crate contains `unsafe`.
- `assembly` is the exception. It reads ROM and disk images from host
  paths, opens journals at host paths through `media`, and holds firmware
  discovery: folder scans, environment variables, the host clock and a
  process-wide hash cache.
- The emulation layer links 26 external crates at run time. Thirteen of
  them arrive only through `media`, and `libc` arrives through `media` on
  every Unix target. No windowing, graphics, host audio, network or async
  crate is among the 26.
- The CPU core, `x86jit-core`, runs every PC mode on its interpreter and
  needs the allocator and a few calls of the host's C library; its JIT
  backend, which the PC does not use, needs executable memory and a
  worker thread.

So the core is already close to the principle, but only by habit: nothing
would stop the next dependency or the next `std::fs` call.

## Decision

1. **A core is a library that touches the host only through what its
   embedder hands it.** "The core" is the contract crate, the crates of
   the `emulation` layer and the CPU core they pin. It needs Rust's `std`
   with an allocator and nothing else of the host.
2. **What a core may not depend on or do.**
   - No crate for windowing, graphics, host audio, host input devices,
     network or an async run time.
   - No host clock and no randomness: nothing of `std::time` on any path,
     and no result that depends on the order of a randomly seeded map.
   - No thread it starts. A machine lives on the thread that created it;
     an embedder may give each machine its own thread.
   - No process-wide mutable state. Two machines in one process share
     nothing they can change; immutable tables are fine.
   - No direct file system access: no path it opens, no directory it
     scans, no environment variable, no standard stream, no process exit.
   - No code for one operating system and no `unsafe`: every crate of the
     contract and emulation layers forbids unsafe code. The CPU core is the
     one place that may need either, and only for a JIT backend that is
     optional: every machine runs on the interpreter with identical guest
     results.
3. **Everything else comes through the contract.** Frames and sound are
   pulled, input is pushed as events, content arrives verified by digest,
   time is virtual and the embedder's clock never enters, messages replace
   the standard streams, and what must outlive a run goes into storage
   objects the embedder hands in. Contract 0.2 adds those objects and
   removes the session directory and the open file of 0.1, so that the
   contract names no type of `std::fs` or `std::path`.
4. **Storage objects keep layered disk images possible.** A base is
   read-only content identified by the SHA-256 of all of its bytes, hashed
   by the embedder however large it is. Written sectors go to a separate
   record. Nothing rewrites a base: committing a disk is the embedder's
   operation and writes a new base or a new layer. Inside the core one
   overlay interface reaches sector contents, so a chain of hash-linked
   layers can stand behind a base without the core knowing.
5. **The rule is checked, not remembered.** The boundary check gets a list
   of the external crates the `contract` and `emulation` layers may link:
   none for the contract, a reviewed list with a reason per crate for the
   emulation layer, read from the resolved dependency graph so that a crate
   arriving through another crate is caught too. A crate that is not listed
   fails CI; a listed crate that is gone fails too. A second check scans
   the production sources of those layers for the host interfaces of `std`
   that item 2 forbids, with a list of today's exceptions that only
   shrinks.
6. **Code that needs the host lives outside the core.** Opening files,
   locks, temporary files, folder scans, caches in the user's directories
   and the offline journal tools are host-side code: in the front end, in
   the core's own command line, or in a small crate that implements the
   contract's objects over files. The journal's format and its recovery
   logic stay in the core, over records.
7. **Names.** The front end's binary is `unemu`. A core's binary, its
   `CoreDescription::id` and its repository carry the core's name:
   `unemupc`, `unemunes`. The marker of [[decision-30]] keeps naming the
   core's release.
8. **Each core has its own command line.** It lives with the core, runs
   and checks machines without the front end, and depends on the core's
   crates only: no image store, no registry, no recipes, no window
   toolkit. For the PC it is `unemupc`: headless runs of machine files and
   BIOS ROMs, the machine check and the journal tools, with the words,
   output and exit codes those commands have in the [unemu front end](https://github.com/unemu-org/unemu), and room
   for the debugger, tracing and the exception trace.
9. **Not required now**, each with the event that would make it worth its
   cost:
   - **`no_std`.** When a target without `std` is wanted: bare metal or a
     console SDK. Until then `std` without the host interfaces is the
     rule, and it is what the checks enforce.
   - **A C ABI.** When an embedder that is not written in Rust exists. It
     is a wrapper crate over the contract, which is object-safe and
     generic-free so that this stays a wrapper.
   - **A libretro wrapper.** When someone wants to run a core in a
     libretro front end, and not before the contract is 1.0. It is a
     separate crate over the contract, first for the NES core, whose
     machine fits libretro's model; the PC fits it poorly (keyboard and
     mouse, media catalogues, journals).

## Consequences

- `assembly` stops opening files. Images and journals reach it as content
  and records; the loaders that know host paths (`machine-file`, the PC
  provider) open them. Firmware discovery leaves `assembly` for host-side
  code.
- `media` splits: the journal format and its sink become a core crate
  without file system access; the file-backed records, the offline tools,
  the FAT12 builder and image creation are host-side. The `assembly` →
  `media` exception of the boundary check goes away, and with it `libc`,
  `serde` and `toml` leave the core.
- `libc` stays in the core in one place: SHA-256's CPU feature detection
  on a few targets that have a C library by definition. It is listed with
  that reason.
- `x86jit-core` is built without its default features, and x86jit is
  asked for a way to create a machine for one thread without its
  process-wide barrier calls.
- The offline journal commands are the PC core's: they read the core's
  own format. They move from the neutral command line to the PC's part of
  it and to `unemupc`.
- The desktop binary is renamed `unemu`, and `unemupc` becomes the core's
  command line. Release files, scripts and docs follow.
- A core can be embedded by a program that has no file system: a browser,
  a test, a libretro wrapper. Tests can run a machine with storage in
  memory.
- More small crates, and one more list to keep. Each new external crate
  of the core costs a reviewed line.

## Alternatives considered

- **Keep it a convention in the docs.** That is how `assembly` came to
  hold file reads, environment variables and the host clock. A check is
  cheap; the boundary check already reads the dependency graph.
- **`no_std` cores now.** The strongest form of the rule. It would replace
  `thiserror`, the maps and the I/O traits in every crate and in the CPU
  core, for no embedder that exists.
- **libretro itself as the contract.** It would give a front end for
  free. It assumes one core instance per process with global state, a
  frame-stepped console and a save file; it has no catalogue of media, no
  virtual time and no description of a machine as data. The contract
  keeps Rust types, and a wrapper can map it to libretro later.
- **A virtual file system in the contract**, as libretro offers: open,
  rename, remove, stat and directory listing by path. It would keep the
  journal code as it is, and it would bring paths, rename semantics and
  locks into the contract. Records with append, sync and atomic
  replacement are what the journal needs and no more.
- **Keep the session directory of contract 0.1.** No work now. It keeps
  per-OS code and `libc` in the core and rules out an embedder without a
  file system.
- **No command line in the core**, only the front end's. Then a core
  cannot be run or tested without the repository that depends on it.

## Trigger to revisit

- The allow list or the exception list of the source check grows instead
  of shrinking.
- A second core cannot keep the rule: it needs a host interface the
  contract does not offer.
- A protected-mode profile needs the JIT to be usable. Then executable
  memory and a compile thread become an optional capability the embedder
  grants, and the rule on threads is restated for it.
- An embedder appears that is not Rust, or a target without `std`.
