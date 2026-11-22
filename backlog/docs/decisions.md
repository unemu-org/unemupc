# Where to capture a decision

This PC core project records decisions in **two** places. Pick the right one — don't invent a third.

| Surface | Use when | Lifetime |
|---------|----------|----------|
| **Backlog decision** (`backlog/decisions/`) | The choice constrains app shape, public contracts, layering, error/test/runtime semantics, or is a cross-cutting tool/library/process call. Hard to reverse, or affects future contributors. This is the home for what other projects call ADRs. | Project-lifetime, append-only in spirit |
| **Bead note** (`bd update <id> --append-notes`) | The decision is bound to one bead. Example: "for `upc-f7j.1` we chose X over Y because of constraint Z." | Bound to that bead |

If a decision outlives the immediate bead, it's a `backlog decision`. If it only explains one
bead's implementation, it's a bead note. There is no separate ADR folder — architecture-grade
decisions live in `backlog/decisions/` alongside lighter ones (tag or title them so they're easy
to find).

## Recording a decision

```sh
backlog decision create "<one-line decision summary>"   # → backlog/decisions/decision-N - <Title>.md
ls backlog/decisions/                                    # list existing (no `decision list` subcommand)
```

Fill the generated file. Suggested sections (keep it short — a durable trace, not a research
paper; if it grows past ~250 lines, scope it down):

- **Status** — Proposed / Accepted / Superseded by decision-NNN
- **Date** — ISO date of acceptance
- **Deciders** — names
- **Context** — the situation
- **Decision** — what we picked + why
- **Alternatives considered** — briefly, with why each lost
- **Consequences** — positive and negative
- **Trigger to revisit** — what would re-open this

To reverse an accepted decision, write a new one that supersedes it and update the old file's
Status line to `Superseded by decision-NNN` with a back-link. Editing typos is fine; editing the
substance of an accepted decision is not.

## Recording a bead-bound decision

Attach it to the bead instead — it stays next to the work:

```sh
bd update <id> --append-notes "Decided: <choice> over <alt> — <one-sentence reason>. Revisit if <condition>."
```

## When it's weighty enough to record at all

Write a `backlog decision` when the choice:

- **Constrains the shape of the app or public contracts** — downstream code will depend on it.
- **Is hard to reverse** — undoing it needs a migration, not a refactor.
- **Is cross-cutting** — touches multiple layers/modules/milestones.
- **Was contested or non-obvious** — there were real alternatives and future-you will want the why.
- **Has stakeholder implications** — onboarding, distribution, licensing, hosting.

Skip it (a bead note or commit message is enough) when the choice:

- Is a **tool swap** doable in a day (formatter, linter, package manager, devShell tech).
- Is **DX convenience** with no behavioral effect (editor config, direnv, shell aliases).
- Is a **library swap** in a single layer with no contract change.
- Is a **status update or roadmap item** — those are beads, not decisions.

### Edge cases — record a decision if the answer is "yes"

- **Tool choice with lock-in:** "Build *requires* Nix" → record. "Nix is primary, npm works as
  fallback" → skip.
- **Library swap that changes a public interface:** downstream notices → record; purely internal →
  skip.
- **Process/workflow choice** (e.g. "release from `main` only"): durable contract with
  collaborators → record; personal preference → skip.

Front-end decisions live in [unemu](https://github.com/unemu-org/unemu).
Cross-repository references name the owner, for example [decision-24 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-24%20-%20Desktop%20shell%20is%20a%20classic%20in-window%20menu%20UI%20built%20with%20egui.md).
