---
id: decision-40
title: Front ends drive emulator cores through one contract crate
date: 2026-11-17
status: Accepted
deciders: Mikołaj Mikołajczyk
---

# decision-40 - Front ends drive emulator cores through one contract crate

## Status

Accepted — 2026-11-17, by the maintainer, with the recommended answers to
the open questions of `backlog/docs/design/core-contract.md` §11: the
contract crate is a member of this workspace now, its own small repository
once unemunes starts and published at 1.0; it is named `unemu-core-api`;
`pc`-layer crates on the front-end side may depend on PC core crates;
`machine-file` and the journal half of `media` stay with the PC core for
now; the PC text rasteriser and its fonts move into the PC adapter. Design
step of `upc-dfs.24`, with `upc-3bm`. Builds on [[decision-7]]
(swappability behind traits), [decision-25 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-25%20-%20Machine%20images%20are%20self-contained%20OCI%20artifacts%20in%20a%20local%20store.md) (machine images in a
platform-neutral store), [[decision-30]] (the minimum release of an image)
and [[decision-32]] (platform ids). The inventory of today's coupling, the
contract's signatures and the ordered steps are in
`backlog/docs/design/core-contract.md`.

## Context

The desktop GUI and the command line are to leave unemupc for one front-end
repository that pins emulator cores by exact version, as unemupc pins
x86jit. A NES emulator, unemunes, comes next and takes the same library,
registry and shell.

Today nothing separates a front end from a running machine. Both front
ends hold an `assembly::AssembledMachine` and reach into it: the window
turns key codes into set-1 scan codes, feeds the 8042 at run boundaries,
rasterises text with CP437 fonts, downcasts to the floppy and ATA
controllers for the drive lights and calls the journal API itself. The
command-line crate holds three things in one library: the commands, the
golden runners with 32 emulator test binaries that use nothing else, and
helpers the desktop needs, so `window` depends on `cli`. A change to one
command rebuilds and re-tests the desktop and relinks every golden.

What is already separate: `image-store` and `image-registry` depend on no
emulator crate, platform providers plug into the store through
`ConfigValidator`, `MediaFormats` and `FirmwareRules`, and platform ids
are compared through one function of the store crate.

The maintainer set the shape: a small, stable, versioned contract crate
that every emulator implements and the front end depends on; no grab-bag
core crate; the neutral store and library crates stay separate on the
front-end side; shared emulation building blocks only when a second
emulator needs them; the command line split into a neutral core and
per-platform provider commands; platform-only GUI parts in per-platform
modules. The measure of success is that a CLI-only change builds and tests
in under five minutes.

## Decision

1. **One contract crate.** `unemu-core-api` (working name) is the whole
   contract between a front end and a core. It has two traits: `Core`
   describes itself and creates machines; `Machine` is driven. It has no
   dependencies, no generics and no platform vocabulary, and it forbids
   unsafe code.
2. **A machine is driven by five kinds of call**: run to a limit, read the
   picture and the sound, send input, change media, and control (reset,
   save and load state, finish). A machine advances only inside `run`;
   pause is the front end not calling it. Output is pulled: pixels as
   `0x00RRGGBB` with the display's aspect, and stereo `i16` frames at a
   rate the machine declares.
3. **Time belongs to the core.** Virtual time is ticks at an exact
   rational rate the machine declares; work is counted in steps of the
   core's own unit. No call takes or returns host time. The same create
   request and the same sequence of calls give the same guest and the same
   output.
4. **Capabilities are data.** A core describes itself, each platform it
   provides and each machine: displays, sound, input ports, media slots,
   resets, saved state. A front end enables a control from a description,
   never by trying a call.
5. **Media are slots with catalogues.** A slot holds an ordered catalogue
   and at most one inserted medium; the front end ejects, inserts by
   index, takes the next, or appends a medium. A medium is a read-only
   base, identified by its SHA-256 and hashed in full by the front end,
   plus a separate place for written sectors below the session directory.
   Nothing in the contract rewrites a base, so layered disk images stay
   possible.
6. **Creation takes neutral inputs**: the platform's stored config
   document, verified content by digest, a session directory the front end
   owns, and run-only options outside machine identity. Any other way to
   describe a machine, such as a PC machine file, is the provider's.
7. **The contract is versioned and additive.** Every type is
   non-exhaustive; a minor version adds fields, variants and methods with
   a default and changes nothing; a major version is a change a core or a
   front end must be edited for. While the major is 0 every minor may
   break. A core states the version it implements as a literal. The front
   end refuses a core of another major, lists its platforms disabled with
   the reason, and has a test that every compiled-in core passes.
8. **Platform knowledge stays behind providers.** Artifact validation,
   media formats, firmware rules, machine instances, import, publication,
   machine files, hardware choices for settings, and platform-only
   commands and dialogs stay in provider code on the front-end side. A
   provider crate may depend on its own core's crates beyond the contract;
   a neutral crate may not.
9. **The PC core implements the contract in an adapter crate**,
   `unemu-pc-core` (working name), over `assembly`. Scan-code encoding,
   the keyboard-controller queue, mouse packets and the text rasteriser
   with its fonts move below the contract into it. Golden tests keep using
   the emulation crates directly.
10. **The command line splits in four**: `cli-core` (neutral commands and
    the provider seam), `cli-pc` (PC provider commands), the [unemu front end](https://github.com/unemu-org/unemu)
    binary, and `pc-harness` (the golden runners and their tests). The
    helpers the desktop takes from `cli` move into the libraries they
    belong to, and `window` stops depending on `cli`. A small `platforms`
    crate is the one place that names every compiled-in platform.
11. **The desktop splits by part**: shared widgets, the machine window
    over the contract, the library and Remotes, the media picker and
    dialogs, the PC-only views, and the application.
12. **Layers are declared and checked.** Every workspace crate declares
    one of `contract`, `neutral`, `emulation`, `pc`, `application`. A
    script over `cargo metadata` fails CI on a dependency the layers
    forbid, test dependencies included, and on a crate with no layer.
    Known violations are listed, and a listed violation that is gone
    fails too, so the list only shrinks.
13. **Every step is small and changes no behaviour.** Each lands on `main`
    with the full gate, moves tests with the code they test and weakens
    none.

## Consequences

- A CLI-only change stops rebuilding and re-testing the desktop and the
  emulator goldens. The first two steps (the boundary check, then `window`
  off `cli`) deliver most of the time; the rest deliver the boundary.
- Seventeen crates become about twenty-eight. Clean builds are expected to
  stay level; each step measures it.
- The CC BY-SA font assets move from `window` to the PC adapter, with
  their notices, the licence field and the `deny.toml` exception.
- Package names in CI, the release workflow, the licence script and the
  docs change when `cli` is renamed.
- Before the repositories separate, three more cuts are needed:
  `machine-file` loses its dependency on `image-store`, the PC config
  codec moves below the contract so `Core::create` reads stored configs,
  and the home of the journal half of `media` is settled.
- After the split the program [[decision-30]]'s marker names is the core:
  `unemu.min-app-version` keeps meaning the unemupc release, and the front
  end has its own version that images never name.
- The contract crate must reach every consumer from one source. Cargo
  treats one crate from two git revisions as two crates, so cores and the
  front end cannot each pin it by revision.
- unemunes implements the contract from its first day and needs no change
  to it for a first version; a light gun and validation through the core
  are known later additions.

## Alternatives considered

- **A shared `unemu-core` crate with scheduler, clock, device traits and
  the machine loop.** Rejected by the maintainer's direction: nothing but
  the PC uses them yet, and a crate of everything two emulators might
  share would freeze the PC's internals as a public interface.
- **The front end generic over a machine type** instead of trait objects.
  Monomorphises the shell per core, puts core types in front-end
  signatures and rules out a later process boundary. The calls are few and
  per frame; dynamic dispatch costs nothing that matters.
- **Everything through the contract as data**, including settings,
  firmware catalogues and machine files. It would be the cleanest
  boundary, and it means designing three neutral models for one platform.
  They are added when a second platform needs them.
- **A text grid in the contract** so the desktop keeps the rasteriser.
  The grid's attributes and fonts are PC vocabulary, and every other front
  end would need the fonts again.
- **Storage handed to the core as objects** from the first version, so the
  core does no host I/O. It is the better end state and the contract
  leaves room for it, but it re-plumbs the machine's persistence and
  belongs with layered disk images.
- **Only the affected-set test runner, no split.** It already brings a
  CLI-only change under five minutes by selecting crates, but it selects
  `window` for every `cli` change because the dependency is real, and it
  does nothing for the move to a front-end repository.

## Trigger to revisit

- The PC adapter or the NES core cannot implement a call without knowing
  the front end, or the front end needs a core's type to drive it.
- A second platform needs a settings dialog, validation without a
  provider crate, or storage without a file system: each is a planned
  minor addition, and a reason to check the whole shape again.
- The boundary check's list of exceptions grows instead of shrinking.
