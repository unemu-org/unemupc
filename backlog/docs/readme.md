# Docs

Load-on-demand knowledge for coding agents and humans. The pointer table in
[`../../AGENTS.md`](../../AGENTS.md) tells you which file to open for which task — don't read
everything up-front.

Open work is in beads (`bd`), and the shared working rules are beads memories (`bd memories`) —
start there. Create ad-hoc docs with `backlog doc create "<title>"` (→ `doc-N - Title.md`); the
curated files below are hand-maintained.

| File | Purpose |
|------|---------|
| [`architecture.md`](architecture.md) | Repo shape, data flow, module map. |
| [`commands.md`](commands.md) | Build, test, run, typecheck, lint commands. |
| [`test-timings.md`](test-timings.md) | Measured build and test times; what keeps a change from gating in five minutes. |
| [`conventions.md`](conventions.md) | Coding conventions: file naming, imports, comments, commits. |
| [`dev-setup.md`](dev-setup.md) | Toolchain (flake devShell, direnv, pre-commit, static analysis). |
| [`deferred.md`](deferred.md) | Things deliberately not implemented — don't add them unprompted. |
| [`glossary.md`](glossary.md) | Domain terminology. |
| [`status.md`](status.md) | Feature status (what works, what's in flight, what's broken). |
| [`working-on-tasks.md`](working-on-tasks.md) | Beads: statuses, epics, lifecycle, Definition of Done, git flow, session handoff. |
| [`decisions.md`](decisions.md) | Where to capture a decision: `backlog decision` vs bead note. |

The desktop, image library, recipes and `setup-unemu` are documented in
[unemu](https://github.com/unemu-org/unemu). The split records in `design/`
are frozen; `architecture.md` describes the current core.
