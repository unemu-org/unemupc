# Coding conventions

Generic conventions that apply regardless of stack. Stack-specific rules (language idioms, framework patterns, formatter config) go in the **Stack-specific** section at the bottom — filled at bootstrap.

## File naming

- Pick **one** casing per category and stick with it (e.g. PascalCase for components, kebab-case for scripts, snake_case for modules). Document the choice in Stack-specific.
- One unit per file (one component, one class, one primary export). Co-locate tightly related sibling files (CSS module next to component, test next to source).

## Imports

- Cross-folder imports go through a folder's barrel / public entry, not into its internals. The barrel is the contract; internals are not.
- Prefer path aliases (`@core`, `@services`, ...) over deep relative paths once the project grows past ~3 directory levels.

## Comments

- **Default: no comment.** Names do the work.
- Add only when the *why* is non-obvious: hidden constraint, subtle invariant, workaround for a specific bug, surprising behavior.
- Never explain *what* the code does — well-named identifiers already do that.
- Don't reference the current task / fix / PR ("added for X", "handles case from #123") — that belongs in the commit message, not the source file.
- **Comments must read for a human who has never seen our backlog.** No internal jargon in source: no milestone tags (`M1`, `M2`), no task ids (`task-2`), no decision ids (`decision-5`). Explain the *why* in plain language instead — e.g. not `// TODO(decision-5)` but `// Provisional clock; replace with retired-instruction time once the CPU core exposes an instruction counter.` A doc-comment *may* cite the spec by section (`see spec §6`), since `design/spec.md` is human documentation. Only cite a task id in a comment when it marks a genuine known blocker on that task.

## Commits

- Conventional Commits by default (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`, `release:`). If your project uses a different convention, document in Stack-specific.
- GPG-signed. The `gpg-uid-guard` pre-commit hook refuses to sign if `user.email` has no matching UID on `user.signingkey`.
- **Never commit without explicit user request.** This rule supersedes any plan acceptance.

## Phase / scope discipline

- Don't pre-empt later milestones. If a task is scoped to a later milestone, don't half-implement it during earlier work.
- If a refactor would be cleaner alongside a bug fix but isn't required, defer it — create a backlog task instead.
- Don't add error handling, fallbacks, or validation for scenarios that can't happen at the call site. Trust internal code; validate only at system boundaries (user input, external APIs).

## UI / output text (if applicable)

- Terse. Lowercase. No emoji in UI text or logs unless the project explicitly opts in.

## Publication

The repository is public. Everything committed (code, comments, docs, bead
text, commit messages) follows the publication rule in `AGENTS.md`:

- **No proprietary bytes.** Proprietary firmware, guest software, manuals,
  scans and screenshots of them stay out of the tree; tests and docs keep
  hashes and our own observations. Open firmware, open fixtures and their
  licence texts (the FreeDOS licence index, say) may be committed.
- **Names are fine.** Real firmware (an IBM BIOS, say) and commercial titles
  may be named in code, docs and tests.
- **Nothing private.** No private hosts, private paths or anyone's personal
  setup, and the CPU core's other consumers stay unnamed. Examples use
  `example.org`, `example.com` or `.example` hosts.

`scripts/publication-guard.sh` checks the mechanical part on every commit and
in CI; see [`commands.md` § CI](commands.md#ci).

## When in doubt

- Read the relevant decision: `ls backlog/decisions/` (see [`decisions.md`](decisions.md)).
- Check active work: `bd ready --exclude-type epic`.
- Ask the user. Solo project — they're the only deciding authority.

---

## Stack-specific (Rust)

- **Edition**: 2021. **MSRV**: the latest stable Rust the devShell provides
  (currently 1.99; decision-17), never below x86jit's. **Workspace**: one Cargo workspace, one crate per swap-layer
  (`cpu`, `bus`, `chipset`, `device`, `devices/*`, `video`, `machine`, `unemupc`, `pc-harness`,
  `testkit`).
- **Clippy**: `cargo clippy --all-targets --all-features -- -D warnings` is the
  gate. **Format**: `cargo fmt`, default rustfmt.
- **Error handling**: `thiserror` for library error enums with stable variants
  (bus faults, device errors); `anyhow` only in the host tools (`unemupc`, `pc-harness`) and tests. Never `unwrap`/
  `expect` in library code outside tests — return a typed error.
- **Naming**: model real hardware — device types named after the chip
  (`Uart16550`, `Pic8259`, `Pit8253`, `I8042`). One device per module.
- **Language**: all code, comments, and identifiers in **English** (Polish only in
  chat with the maintainer — see AGENTS.md).
- **License headers**: each source file carries an SPDX line
  `// SPDX-License-Identifier: AGPL-3.0-or-later`.
  Exception: `crates/devices/src/opl3/mod.rs` and `opl3/tables.rs` translate
  Nuked-OPL3 1.8, commit `765ec962`, and preserve Nuke.YKT's copyright with
  `// SPDX-License-Identifier: LGPL-2.1-or-later` (decision-21).
  `opl3/LICENSE` contains the upstream licence. Board timing, serialization and
  tests remain AGPL. The publication guard enforces the exact two-file SPDX
  exception. Cargo-deny checks crate metadata: `devices` and the combined
  program remain AGPL-3.0-or-later; it does not enforce per-file licences.
  URLs inside a verbatim upstream licence notice are allowed.
- **Doc-comments (docs-first rule)**: every public module and trait has a `//!` /
  `///` doc-comment naming the modelled hardware and cross-referencing
  `backlog/docs/design/spec.md` (`§` style). Datasheet deviations are documented.
- **Determinism**: no `std::time`, `Instant`, `SystemTime`, or RNG in the
  execution path. Virtual time = retired instructions.
- **Memory writes**: guest RAM is mutated only through the x86jit `Vm` API that
  runs `note_write` (SMC safety) — never a raw buffer write. See the memory decision.
- **x86jit dependency**: git dep pinned to an exact rev of x86jit `main`; a gitignored
  `[patch]` onto a local checkout for development; never a bare `path` dep (breaks
  `nix build`/CI). See decision-13.

## Test strategy

See [`testing.md`](testing.md): device unit tests on a mock bus, fuzz +
property-based, golden/trace tests (deterministic, pin the x86jit rev),
boot/integration ROMs, SingleStepTests differential for real mode, snapshot/replay.
No device or machine feature merges without its tests.

Proprietary-firmware goldens store hashes and our own interpretation (POST codes,
which tests passed, retired-instruction counts), never firmware bytes or copies
of the firmware's screen text beyond short identifiers. Do not commit screenshots
or full console transcripts from that firmware. User-ROM tests are ignored and
start with `user_rom_`; those that need user firmware use the shared
`machine_file::require_user_firmware!` helper (the user-media tests keep their
own skip); see `commands.md` §User-supplied ROM tests. Discovery tests use synthetic
bytes only.

Keep process-spawning tests in separate test binaries from tests holding journal
locks or pipe/FIFO descriptors whose lifetime affects another test. On Unix,
fork duplicates every thread's open descriptors: `O_CLOEXEC` closes them only at
exec, so a child can briefly retain a sibling's released lock or postpone EOF.
Separate binaries prevent that inheritance without serializing the fast suite.

## Device time and clock domains

Decision-22 §5 applies to new code:

- Durations come from `device::clock::ClockDomain`, never bare tick literals.
  No device assumes one `VClock` tick is one PIT input clock. Use
  `VClock::from_pit` when converting a PIT duration at a boundary.
- Schedule the next input edge with `domain.next_edge_after(now)`, never
  `now + 1`. Handle exhaustion of the virtual-time range.
- Define each crystal once as an exact integer ratio. The board oscillator is
  `device::clock::OSC` (315/22 MHz); derive its divided clocks from it.
  Round edges up and carry epoch phase. Never accumulate rounded periods or
  use floating point for clock arithmetic.
- New devices report deadlines and catch up idempotently in `sync(at)` for any
  time. Do not introduce devices requiring a visit on every PIT input edge.
- Latch picture-affecting state at a raster position in a stateful beam.
  Do not derive new beam state from absolute time or read live state at capture.
- Status phases never advance per read.
- DMA requesters accept partial transfers.
- New run-policy and pacing features count virtual time, not instruction
  equivalents. Existing policy units retain their documented contracts.
- Bus width and wait states belong in the decode map or machine file, never
  hidden inside a device.

The clock-domain seam does not itself change VGA raster anchoring, privatize
`VClock`, or add a cycle-exact CPU profile; those remain separate work.

Known compatibility exceptions in the existing serial models: the 8042 ACK
adds two separately rounded frame durations (host frame plus reply frame),
and the UART starts each back-to-back receive frame at the previous rounded
completion time. Both can accumulate fractional rounding. They deliberately
retain existing guest timing and are not patterns for new clocked hardware.
