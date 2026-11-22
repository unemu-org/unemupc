# Working on tasks

Open work is tracked in **beads** (`bd`, a Dolt-backed issue tracker; issue prefix `upc`). Knowledge
— design, decisions, guides — stays as markdown under `backlog/`; a bead links to a doc section and
never copies it. Roles, the path from a bead to `main`, review and the worker rules are beads
memories (`bd memories`, keys `roles`, `bead-to-main`, `bead-conventions`, `worker-prompts`,
`sandbox-workers`, `publication-rule`; the publication rule is also in
[`conventions.md` § Publication](conventions.md#publication)); this page is the day-to-day reference. `bd <cmd> --help`
gives the flags. `bd prime` prints the tool's generic workflow; where it differs from these docs and
the beads memories (who claims and closes, what goes into `bd remember` — see
`bd recall bead-conventions` — pushing), these docs and the memories win.

This is the same workflow the CPU core (`x86jit`, prefix `xj`) uses. The front end uses its own repository (unemu, prefix `ufe`). These projects keep separate
bead databases; a reference across them carries a tag — `xj-6vd.9 (x86jit)`.

## Statuses

| Status | Meaning |
|--------|---------|
| `open` | Filed, scoped, not started. Default for every new bead. |
| `in_progress` | Claimed and actively worked. Claim it **before** you start writing code. |
| `closed` | Landed on `main`. The close reason names the commit and the doc section changed. |

`bd` derives "blocked" from dependencies: a bead with an open blocker does not show in `bd ready`.

```sh
bd ready --exclude-type epic               # unblocked work, highest priority first (plain `bd ready` lists the epics too)
bd list --status in_progress               # what is claimed right now
bd show <id>                               # one bead with its dependencies and parent
bd list --parent <epic-id>                 # the children of one epic
bd list --external-ref backlog:TASK-41     # the bead an old Backlog.md id became
```

Priorities are `P0`…`P4` (`P0` highest). The import mapped Backlog.md high/medium/low to
`P1`/`P2`/`P3`; unset became `P2`.

## Epics

Each theme is an epic (`bd list --type epic`); every task bead belongs to one, through `--parent`
(housekeeping chores may stand alone). The epic carries the theme's label; children keep their own
labels (create them with `--no-inherit-labels`). A dotted id (`upc-sk3.1`) shows the parent a bead
was created under; after a reparent `bd show` is authoritative.

| Epic key | Theme |
|----------|-------|
| `cpu-core` | the x86jit CPU core as a pinned dependency — the pin, API adaptation, real-mode gaps the machine needs |
| `code-review` | defects from the whole-code review |
| `validation` | machine-level validation ROMs and oracles (test386 and similar) |
| `storage` | disk media, controllers and image persistence |
| `tests-ci` | test infrastructure, CI lanes and tooling gates |
| `scheduler` | event-driven device scheduling, clock domains, CPU speed |
| `observation` | observable output from device state, not replayed traces |
| `skeleton` | phase A of decision-16: the display through a real card to a window, host keyboard |
| `consolidation` | phase A′: machine description files (TOML/YAML), firmware and machine class as configuration, recoverable floppy persistence |
| `protected-mode` | phase B: 386 protected mode, paging, A20, a 386 profile — DOS-extended programs |
| `win9x-base` | phase C: PCI, VBE, audio, IDE, mouse, Pentium features — Windows 98 SE to the desktop |
| `voodoo` | phase D, the target: Windows 98 SE with a 3dfx Voodoo card |

The four phase epics follow the scope decision (decision-16). Each ends in a milestone bead
(`bd list --type milestone`); the last one is the project's goal line. Later phases stay coarse
until the phase before them is close to done. Before any phase, the foundation milestone (label
`foundation`) gathers the open work that finishes the 286 machine across the other epics; it
blocks the first bead of phases A and B.

A new theme gets a new epic; do not stretch an existing one to fit.

## Bead lifecycle

```sh
bd create --title "<title>" --description "<why and what>" --acceptance "<criteria>" \
          --type task --priority P2 --parent <epic-id> --no-inherit-labels --labels <theme labels>
bd update <id> --claim                              # before writing code
bd update <id> --append-notes "MEASURED: <command> → <result>"
bd close <id> --reason "<commit> — <what landed; doc section changed>"   # only once it is on main
```

- **Never `--notes` on an existing bead**: it replaces the whole notes field. Use `--append-notes`.
- Label evidence: **MEASURED** (command run, give it), **READ** (`file:line`), **ASSUMED** (not
  checked yet).
- Work found along the way is a new bead with `--deps discovered-from:<id>`, not folded into the
  current one.
- Do not close until `main` actually contains the work — an early close misleads `bd ready`.

## Definition of Done

A bead closes only when all of these hold on `main`:

1. `cargo test --workspace` is green.
2. `cargo clippy --all-targets --all-features -- -D warnings` is clean.
3. `cargo fmt --check` is clean.

The CI lane runs these checks via `scripts/ci-gate.sh`, plus the publication
guard, `cargo deny --all-features check licenses bans sources` on every run,
and the slow goldens after building their fixtures. Use
`bash scripts/ci-gate.sh <step>` to run the same gate locally; see
[`commands.md` § CI](commands.md#ci). Before merging one change,
`bash scripts/bead-gate.sh` runs the same kinds of checks on only what the change
can affect, with the slow goldens when it touches emulation; see
[`commands.md` § Affected tests and the bead gate](commands.md#affected-tests-and-the-bead-gate).

Plus, when the change warrants it:

- `cargo deny --all-features check licenses bans sources` whenever the dependency
  set changes (including features, sources, and licence policy), following x86jit's
  conditional Definition of Done;
- the slow goldens, `cargo test --workspace -- --ignored` (BIOS POST, FreeDOS boot, the DOS
  sessions; about 75 s; the gitignored fixtures under `vendor/` must be fetched — a missing
  fixture skips) —
  whenever the `cpu`, `machine`, `assembly` or `devices` crates change or the x86jit pin moves;
- an A/B equality test when a refactor claims to change nothing observable;
- a mutation check on a new golden: break the feature, watch the golden fail.

## Git flow

One bead, one branch (`<type>/<bead-or-topic>`, [Conventional Branch](https://conventionalbranch.org/)),
one worktree outside the main checkout. The supervisor squash-merges each bead into `main` as one
commit: **Conventional Commits** (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `test:`, `ci:`,
`build:`), **GPG-signed**. Commits and pushes happen only at the maintainer's request. Never bypass
hooks. Details: `bd recall bead-to-main`.

The configured `origin` is the only push destination. The issue database lives in `.beads/`
(gitignored); `bd dolt push` stores it in `refs/dolt/data` on `origin` and it goes nowhere else.
Push it after a batch of bead changes, with the maintainer's agreement like any push. Until a remote
exists the database lives on one machine only — back it up with `bd backup` if it matters.

## Decision capture

- Bead-bound decision → on the bead: `bd update <id> --append-notes "Decided: X over Y — <reason>."`
- Cross-cutting / architecture-grade → `backlog decision create "<summary>"` (decisions stay
  markdown under `backlog/decisions/`). See [`decisions.md`](decisions.md).

## Session handoff

```sh
bd update <id> --append-notes "Session pause $(date -I). Done: <X>. Next: <Y>. Blocker: <Z|none>."
```

The next session runs `bd list --status in_progress`, then `bd show <id>`, and picks up without
rediscovering state from the diff.

## History

Backlog.md was the tracker until 2026-11-09. Its Done history stays read-only under
`backlog/completed/`; the task files migrated to beads are archived under `backlog/archive/tasks/`.
Every migrated bead carries its old id as `external_ref` (`backlog:TASK-NN`) and its old file name
and dates in `metadata`. Text that mentions `task-NN` resolves through
`bd list --external-ref backlog:TASK-NN` (exact, upper case). No bead means the task was Done before
the migration: read `backlog/completed/task-NN - *.md`.

What the migration did, so it can be checked:

- The board held 41 tasks: 39 `Done`, 1 `In Progress` (`task-7`), 1 `To Do` (`task-41`). The two
  open ones became beads — `task-7` → `upc-fro.1`, `task-41` → `upc-sk3.1`. Title, Description,
  Acceptance Criteria (numbering and checked state kept) and Implementation Notes were copied
  verbatim; labels were kept; priority low became `P3`, unset became `P2`.
- `task-7` had sat `In Progress` since 2026-07-17 with nobody working it; it was imported as
  `open`, with a note saying so.
- Neither open task had a dependency on another open task, so no `blocks` edges were needed.
- The 39 Done tasks moved from `backlog/tasks/` to `backlog/completed/`, not imported. `task-9`
  never existed on the board.
- The working rules that lived in `handoff.md` became beads memories; the project snapshot it
  carried is in [`status.md`](status.md).
