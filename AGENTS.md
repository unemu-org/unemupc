# AGENTS.md — unemupc

Repo-specific notes for coding agents (Claude Code, Cursor, Aider, Copilot, …). Generic software-engineering advice is out of scope.

> **CLAUDE.md** at repo root is `@AGENTS.md` plus Claude-only overrides. Other agents read this file directly.

## What this is

`unemupc` is a **low-level emulator (LLE) of an IBM PC/AT-class (Intel 80286)
computer**. The CPU core is the [`x86jit`](https://github.com/unemu-org/x86jit)
x86 recompiler, consumed as a pinned git dependency (see below). It is a
**research project**: the priorities are **testability**, **swappability**
(CPU / device / graphics behind traits, machines as declarative configs), and
**dense documentation** (in-source doc-comments + `backlog/docs/`). The machine
boots an unmodified open BIOS and FreeDOS from a floppy image, headless and
deterministic, captured for golden tests.

The core and its standalone program `unemupc` live here. The desktop and
command front end, including `setup-unemu`, live in
[unemu](https://github.com/unemu-org/unemu) (beads prefix `ufe`). Core changes
reach that repository through an exact git revision pin.

The 286 AT is the first machine profile, not the end target. The goal line is
a period PC that runs Windows 98 SE games on an emulated 3dfx Voodoo, reached
in four phases, each an epic in beads; Windows XP-era machines are a non-goal
(`backlog/decisions/decision-16*`). Check new work against that goal line.

Open work is tracked in **beads** (`bd`, issue prefix `upc`), a Dolt-backed issue tracker — the
same workflow as `x86jit`. Its database lives in `.beads/` (gitignored) and reaches other machines
only through `bd dolt push` → `refs/dolt/data` on `origin`. Knowledge — design, decisions, guides —
is committed markdown under `backlog/`, and the Done history of the earlier Backlog.md tracker stays
there read-only.

## Language

- **Talk to the maintainer in Polish.**
- **Everything committed is English**: code, comments, doc-comments, docs under
  `backlog/docs/`, commit messages, branch names, bead and decision titles, and all
  identifiers. No Polish in the repository or in bead text.

## CPU core: x86jit (pinned dependency — hard rule)

The CPU is **not vendored or copied**. `x86jit` is a separate project with its own
tracker (beads, prefix `xj`), gates and rules; real mode (`CpuMode::Real16`,
interpreter-only) lives on its `main`. This repo depends on `x86jit-core` as a
**git dependency pinned to an exact revision** of x86jit `main`. A bare `path`
dependency in `Cargo.toml` is banned — it breaks `nix build` / CI; develop against a
local checkout through a gitignored `[patch]` override (`backlog/docs/dev-setup.md`).

- A change the machine needs from the CPU is **filed and landed in x86jit**, under its
  workflow: behind `CpuMode::Real16`, `CpuState` / `jit_abi` append-only, the other
  modes' generated code unchanged, x86jit's full gate green, the seam merged whole with
  its tests. Then this repo re-pins.
- **Never stage with `git add -A` in an x86jit checkout** — use explicit paths; leave
  untracked files there alone.
- Pin the `x86jit` rev **per golden test**: golden traces are only valid against a
  known CPU revision, so moving the pin re-runs the slow goldens.
- Cross-repository ids carry a tag: `xj-6vd.9 (x86jit)`, `decision-13 (x86jit)`.

See `ls backlog/decisions/` — the real-mode extension, IRQ-injection contract,
virtual clock, and memory-write (SMC) decisions.

## Engineering hard rules (project-specific)

1. **Testability-first.** No device lands without isolation unit tests on a mock
   bus. No machine-level feature lands without a golden/integration test.
2. **Swappability-first.** New hardware goes behind the `Cpu` / `Device` /
   `InterruptController` / `FrameSink` traits and is composed by a `MachineConfig`
   (the `assembly` crate). The machine core never hardcodes a device or graphics mode.
3. **Docs-first.** Every module and trait carries a doc-comment describing the
   modelled hardware, citing the datasheet by name and section, and cross-referencing
   `backlog/docs/` (`§` style). Datasheet coverage (what is / isn't modelled) is tracked.
4. **Determinism.** No wall-clock in the execution path. Virtual time is measured
   in **retired instructions** (never in JIT blocks — block boundaries are a
   lifter implementation detail), plus PIT clocks that pass while the CPU is
   halted (decision-14). Devices tick off the virtual clock.

## Where things live

| Need | Path | When to load |
|------|------|--------------|
| **Roadmap / task board (source of truth)** | beads — `bd ready --exclude-type epic`, `bd list --type epic`, `bd show <id>` | Always. **Don't read roadmaps from markdown.** One epic per theme; the list is in [`working-on-tasks.md`](backlog/docs/working-on-tasks.md) § Epics. |
| **Shared working rules: roles, bead-to-main flow, worker briefs, publication rule** | beads memories — `bd memories`, `bd recall <key>` | **At session start, before any task work** |
| Bead statuses, epics, lifecycle, Definition of Done, git flow, session handoff | [`backlog/docs/working-on-tasks.md`](backlog/docs/working-on-tasks.md) | Before picking up a task |
| Design spec + machine-config design | [`backlog/docs/design/`](backlog/docs/design/) | Structural or config-layer changes |
| Current repo shape, data flow, file map | [`backlog/docs/architecture.md`](backlog/docs/architecture.md) | When making structural changes or unfamiliar with module layout |
| Coding conventions, file naming, commit style, comment policy | [`backlog/docs/conventions.md`](backlog/docs/conventions.md) | Before writing or modifying code |
| Feature status (what works, what's in flight, what's broken) | [`backlog/docs/status.md`](backlog/docs/status.md) | When user asks "does X work?" or you're picking up work |
| Common dev commands (build, test, run, lint) | [`backlog/docs/commands.md`](backlog/docs/commands.md) | When running build/test/dev loops |
| Toolchain (Nix devShell, direnv, pre-commit, `bd`, the x86jit override) | [`backlog/docs/dev-setup.md`](backlog/docs/dev-setup.md) | When fixing tooling, adding hooks, or onboarding |
| Where to capture decisions (`backlog decision` vs bead note) | [`backlog/docs/decisions.md`](backlog/docs/decisions.md) + `ls backlog/decisions/` | When making a non-trivial decision |
| Project glossary / domain terminology | [`backlog/docs/glossary.md`](backlog/docs/glossary.md) | When you hit an unfamiliar term |
| Things deliberately deferred — do NOT implement unprompted | [`backlog/docs/deferred.md`](backlog/docs/deferred.md) | Before adding features that "seem missing" |
| Backlog skill (`backlog` CLI for **docs and decisions only** — tasks moved to beads) | [`.agents/skills/backlog/SKILL.md`](.agents/skills/backlog/SKILL.md) | Creating a doc or a decision with `backlog doc` / `backlog decision` |
| Code navigation (`repoctx` — structure-aware, token-cheap) | `.claude/skills/repoctx/SKILL.md` | "Where is X?", callers/callees, blast radius — prefer over grep/find |

> **Skills location.** Vendored skills (e.g. `backlog`) live at `.agents/skills/<name>/` (agent-agnostic, **committed**); `scripts/skills-bootstrap.sh` symlinks them into `.claude/skills/` for Claude Code auto-trigger — re-run it after adding one. `repoctx` is **tool-managed**: it lives directly under `.claude/skills/repoctx/`, regenerated by the `repoctx` CLI, not vendored.

## Load-on-demand rule

Don't read every `backlog/docs/` file at session start. Pick the file matching the task — they are sized to be loaded individually. The table above tells you *when* to load *what*.

## Working on tasks

The roles, the path from a bead to `main`, the worker rules and the publication rule are beads memories — read them with `bd memories` (`bd recall <key>` for one; `bd prime` prints them too). Read [`backlog/docs/working-on-tasks.md`](backlog/docs/working-on-tasks.md) for statuses (`open` / `in_progress` / `closed`), epics, the Definition of Done and the git flow. The supervisor picks work with `bd ready --exclude-type epic`, claims it with `bd update <id> --claim` **before** code is written, and closes it only after it lands on `main`, with a reason that names the commit. Workers do not claim or close beads; they report to the supervisor. Append to notes with `--append-notes` — never `--notes` on an existing bead, which replaces the field. Where `bd prime`'s generic advice differs from these docs and the beads memories, these docs and the memories win. Text written before the migration says `task-NN`: `bd list --external-ref backlog:TASK-NN` finds the bead; no match means the task was already Done — read `backlog/completed/task-NN - *.md`.

## Session handoff

Ending a session mid-task, leave the state on the bead itself:

```sh
bd update <id> --append-notes "Session pause $(date -I). Done: <X>. Next: <Y>. Blocker: <Z|none>."
```

Starting a session, read the shared rules (`bd memories`), then the in-progress beads (`bd list --status in_progress`, then `bd show <id>`), before doing anything else. A sandboxed worker that cannot open the beads database (`openat LOCK: operation not permitted`) takes its rules from the supervisor's brief.

## Quick dev loop

```sh
nix develop                 # toolchain — or direnv auto-loads it
bd --version                # beads ≥ 1.3.1, installed on the host (not in the devShell; dev-setup.md)
cargo build
cargo test --workspace                                  # unit + golden + integration
cargo test --workspace -- --ignored                     # slow goldens: BIOS POST, FreeDOS boot, DOS sessions (several minutes; see backlog/docs/commands.md)
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

Full list in [`backlog/docs/commands.md`](backlog/docs/commands.md).

## Hard rules (don't violate)

- **Never commit without explicit user request.** Even mid-task, after accepting a plan, stop and ask. Plan acceptance ≠ commit acceptance. `bd init` and the first `bd dolt push` make a git commit on their own — after any bd setup command, check `git log -1`.
- **Write as if the repository were public.** Code, comments, docs, bead text and commit messages are English and publish-safe. Never commit proprietary bytes: no proprietary firmware, guest software, manuals or screenshots of them — only hashes and our own observations. Open firmware, open fixtures and their licence texts may be committed. Naming real firmware and commercial titles is fine. Never name a private host, a private path, anyone's personal setup or the CPU core's other consumers. `scripts/publication-guard.sh` checks the mechanical part. Push only to the configured `origin`; never create, mirror or publish a repository — publishing is the maintainer's step.
- **Don't add features, refactor, or introduce abstractions beyond what the task requires.** Bug fix = bug fix, not surrounding cleanup.
- **Don't pre-empt later milestones.** Don't half-implement work a later bead or epic owns.
- **All project docs live under `backlog/`.** Docs under `backlog/docs/`, decisions under `backlog/decisions/`; open work lives in beads, never in markdown.

## Code ownership

Maintainer / sole deciding authority: **Mikołaj Mikołajczyk**
(<mikolaj@mikolajczyk.org>).
