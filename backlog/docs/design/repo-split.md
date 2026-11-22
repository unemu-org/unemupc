# Repository split: the extraction plan

Frozen record of the repository split; the living description is in [architecture.md](../architecture.md), and the normative contract text is in [unemu-core-api](https://github.com/unemu-org/unemu-core-api).

Status: **Accepted** by the maintainer on 2026-11-22 (bead `upc-dfs.53`), with
every recommended answer of §14. Proposed 2026-11-21. Preparation steps E1
and E2 are recorded below; no new repository exists yet. The steps of §12
track the extraction. It continues
[core contract §9](core-contract.md#9-towards-two-repositories), whose
preconditions are met, and it follows the maintainer's decision of 2026-11-21
to separate the repositories **before** contract 1.0.

Markers: **[REPO]** = read in this repository at `main` `af41c2f7`, with the
path; **MEASURED** = produced for this document by a command, which is named;
**ASSUMED** = not verified, and the step that verifies it is named.

## 0. The plan in ten lines

1. Two repositories: the **PC core** and the **front end**. The contract
   (`unemu-core-api`), the CPU core (`x86jit`) and the NES core (`unemunes`)
   already have their own.
2. The core keeps the repository `unemu-org/unemupc`, its history, its
   release line, its tags and its beads (`upc`). It holds 17 crates and the
   binary `unemupc`.
3. The front end is a new repository, proposed name `unemu-org/unemu`, with
   22 crates and the binary `unemu`. It starts from the same history, so every
   commit named in a document or a bead stays valid in both.
4. The front end takes the core as a **git dependency pinned to an exact
   revision**, the way this repository takes `x86jit` today, with a gitignored
   `[patch]` override for local work. A core change reaches the front end by a
   re-pin, which is its own bead and re-runs the front end's slow goldens.
5. The core's dependency closure, test dependencies included, already holds
   no front-end crate (§2.2). What still crosses the line is outside Cargo:
   test support shared with `#[path]` and paths built from the repository
   root (§2.4). Two preparation steps remove both, inside this repository,
   under the normal gate.
6. Fixtures keep one home and one set of pins: the core. The front end's
   tests read them from a checkout of the core at the pinned revision (§5).
7. Each repository gets its own workspace, lock file, devShell, dependency
   policy, CI, release workflow, guards and gate scripts (§6 to §8).
8. Releases separate: `unemupc-<version>-<target>.tar.gz` from the core,
   `unemu-<version>-<target>.tar.gz` from the front end. The two markers of
   decision-30 already have separate version sources.
9. The order makes the cut reversible until late: the front end is built and
   gated against the unchanged core repository first; only then does the core
   repository drop the front-end crates (§12).
10. Contract 1.0, the redesign of the GUI and a shared provider layer are not
    part of this plan and are not prepared by it.

## 1. Where the work stands

The preconditions of core contract §9, as of `af41c2f7`:

| # | Precondition | State |
|---|---|---|
| 1 | The boundary check passes with an empty core-side allow list | Met. `scripts/crate-boundaries.allow` holds one line, the application-composition edge `window` → `upcli`, which stays inside the front end [REPO] |
| 2 | Stage B: stored PC images start through `core_launch` | Met (core contract §14) |
| 3 | `machine-file` does not depend on `image-store` | Met (step 7 of core contract §12.10) |
| 4 | `media` is placed; the journal engine is the core's | Met: `media` stays with the front end, `journal` and `journal-files` with the core |
| 5 | The contract is 1.0 | **Superseded** by the decision of 2026-11-21: the contract lives in `unemu-org/unemu-core-api` (crates `unemu-core-api` and `unemu-core-files`), is pinned by revision, and reaches 1.0 after the split |
| — | The core opens no file; the host-interface allow list is empty | Met. `scripts/core-host-guard.allow` has no entry [REPO] |
| — | Decision-30 names two release domains | Met: the amendment is accepted and implemented; the extraction baseline is 0.0.3 |

Figures used below. MEASURED with
`cargo metadata --offline --format-version 1 --no-deps`, `git ls-files`,
`git rev-list --count HEAD`, `git tag -l` and `grep` over `crates/`:

- 39 workspace packages; 1,156 tracked files; 566 commits; a 23 MiB pack.
- Tags `v0.0.1`, `v0.0.2`, `v0.0.3`. The workspace version is `0.0.3`.
  `NEXT_RELEASE` is `0.0.4` in both `pc_config::compat` (core) and
  `image_store::compat` (front end); `EXTRACTION_BASELINE` is `0.0.3`.
- No release ships the binary `unemu` yet: the `setup-unemu` action installs
  `v0.0.4` or newer [REPO: [`.github/actions/setup-unemu/README.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/.github/actions/setup-unemu/README.md)].

## 2. Where every crate goes

### 2.1. The two sets

The rule is the one `scripts/crate-boundaries.sh` already checks: a package is
on the core's side when its layer is `emulation` or its manifest says
`side = "core"` [REPO: `[package.metadata.unemu]` in each `Cargo.toml`].
Everything else is the front end's.

**Core repository (17 packages).**

| Package (directory under `crates/`) | Layer | Notes |
|---|---|---|
| `device`, `bus`, `cpu`, `video`, `audio`, `devices`, `chipset`, `machine`, `assembly` | `emulation` | The machine, its devices and the assembler. `cpu` is the only crate that names `x86jit-core` |
| `testkit` | `emulation` | Mock bus for device tests. A test dependency inside the core only; no front-end crate names it |
| `journal` | `emulation` | Journal format, recovery, overlay model |
| `pc-config` | `emulation` | PC config codec, launch document, the core's compatibility rows (`compat`) |
| `unemu-pc-core` (`pc-core`) | `emulation` | The contract adapter, the text rasteriser and its fonts |
| `machine-file` | `pc`, `side = "core"` | Machine description files, host firmware discovery |
| `journal-files` | `pc`, `side = "core"` | File-backed journals, offline tools, raw-image publication, disk-layer format |
| `pc-harness` | `pc`, `side = "core"` | Machine runners and the emulation goldens |
| `unemupc` | `pc`, `side = "core"` | The core's command line: library and binary `unemupc` |

**Front-end repository (22 packages).**

| Package (directory under `crates/`) | Layer | Names a core crate |
|---|---|---|
| `media`, `image-store`, `image-registry` (`registry`), `recipe`, `core-launch`, `cli-core`, `ui-kit`, `machine-view`, `library-model`, `library-ui`, `player-ui`, `unemu-test-core` (`test-core`) | `neutral` | No, of any kind |
| `pc-image`, `cli-pc`, `pc-media`, `pc-library`, `media-ui`, `pc-ui` | `pc` | Yes (core contract §11 Q3 (a): a provider may use its own core's crates) |
| `pc-session-tests` | `pc` | Yes, test dependencies only |
| `platforms`, `upcli`, `window` (binary `unemu`) | `application` | Yes |

This is the table of core contract §9 with the crates added since:
`journal`, `journal-files`, `unemupc`, `pc-config` to the core; `core-launch`,
`recipe`, `unemu-test-core`, `platforms` and the window crates by name to the
front end.

### 2.2. The core set closes

MEASURED: over the manifests' declared dependencies of every kind (normal,
build, dev, optional and target-specific), the 17 core packages have **no
edge to a package outside the 17**. Their direct dependencies from outside
the workspace are ten: `x86jit-core`, `unemu-core-api`, `unemu-core-files`,
`thiserror`, `anyhow`, `sha2`, `serde`, `serde-saphyr`, `toml`, `libc`. None
is a window, audio, network or compression crate; the exact list of what the
`emulation` layer may reach stays in `scripts/crate-boundaries.external`.

The same check runs on every push today: it is the core-side rule of
`scripts/crate-boundaries.sh`, with no exception listed.

### 2.3. What the front end takes from the core

MEASURED from the same metadata: front-end packages name **13 core packages**
directly.

| Core package | As a normal dependency of | Only as a test dependency of |
|---|---|---|
| `assembly` | `cli-pc`, `media-ui`, `pc-image`, `pc-library`, `pc-media`, `pc-ui`, `platforms`, `upcli`, `window` | `pc-session-tests` |
| `machine-file` | `cli-pc`, `pc-image`, `pc-library`, `pc-media`, `pc-ui`, `upcli`, `window` | `media-ui`, `pc-session-tests` |
| `journal-files` | `cli-pc`, `pc-image`, `pc-library`, `pc-media`, `pc-ui`, `window` | `pc-session-tests` |
| `unemu-pc-core` | `cli-pc`, `pc-library`, `platforms`, `window` | `pc-image`, `pc-session-tests`, `pc-ui` |
| `devices` | `pc-media`, `upcli`, `window` | `cli-pc` |
| `pc-config` | `pc-image` | `pc-session-tests`, `platforms` |
| `device`, `video` | `cli-pc` | `pc-session-tests`, `pc-ui`, `window` |
| `machine` | `cli-pc` | `pc-image`, `pc-session-tests`, `upcli`, `window` |
| `pc-harness` | `cli-pc` | `upcli`, `window` |
| `unemupc` (its library) | `cli-pc` | — |
| `cpu` | — | `cli-pc`, `pc-image`, `pc-session-tests`, `pc-ui`, `upcli`, `window` |
| `audio` | — | `pc-session-tests` |

`bus`, `chipset`, `journal` and `testkit` are never named by the front end.
Features the front end asks of core crates: `test-support` on `assembly`,
`machine-file` and `unemu-pc-core`, in test dependencies only. §2.4 adds
`test-support` on `pc-harness`.

This surface is wide and it is not the contract. It is permitted by core
contract §11 Q3 (a), and the exact pin makes it safe: an incompatible core
change shows when the pin moves, never before. Narrowing it belongs to the
later GUI and provider work, not to this split.

### 2.4. What crosses the line outside Cargo

At the plan baseline, the boundary script read Cargo dependencies only.
Three kinds of reach outside that graph must be cut before the repositories
separate. The E1 source-include check now covers the first kind.

**(a) Test support shared with `#[path]`.** MEASURED by
`grep '#\[path' crates`: front-end test sources include files of
`crates/pc-harness/tests/support/` 30 times.

| Front-end crate | `fixtures.rs` | [`session.rs`](../../../crates/pc-harness/tests/support/session.rs) | [`at_games.rs`](../../../crates/pc-harness/tests/support/at_games.rs) | `fat12.rs` |
|---|---|---|---|---|
| `cli-pc` | 12 | 2 | 1 | 2 |
| `pc-image` | 10 | — | — | — |
| `pc-library`, `upcli`, `window` | 1 each | — | — | — |

Core crates include the same files too (`assembly` 3, `machine-file` 8,
`pc-config` 1, `unemupc` 1); those stay as they are, inside one repository.
`scheduler.rs` is used by the core only. The one include between front-end
crates (`cli-pc` → [`crates/pc-image/tests/support/open_image.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/support/open_image.rs)) stays.

Answer: `pc-harness` gains a `test-support` feature that exposes these four
modules as `pc_harness::test_support::{fixtures, session, at_games, fat12}`,
the way `machine-file` and `assembly` already expose theirs. Front-end tests
`use` them. `pc-harness` is already a normal dependency of `cli-pc` and a
test dependency of `upcli` and `window`; `pc-image` and `pc-library` gain it
as a test dependency. Step E1.

**(b) Paths built from the repository root.** `fixtures.rs` and [`at_games.rs`](../../../crates/pc-harness/tests/support/at_games.rs)
resolve `vendor/` and `machines/` from the *including* crate's
`CARGO_MANIFEST_DIR` joined with `../..` [REPO:
`crates/pc-harness/tests/support/fixtures.rs:17`, `at_games.rs:13`].
MEASURED by `grep CARGO_MANIFEST_DIR` with `../..`: front-end crates build
root paths themselves in `cli-pc` (9 places), `pc-image` (6), `pc-library`
(4), `window` (4) and `image-store` (1, the lock file). Compile-time reads of
core files from front-end tests: `vendor/freedos/fetch.sh`
([`crates/cli-pc/tests/media_files_golden.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/tests/media_files_golden.rs)) and `machines/README.md`
([`crates/cli-pc/tests/media_create.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/tests/media_create.rs)).

Answer: one function names the tree that holds `machines/` and `vendor/`,
`pc_harness::test_support::root()`: the directory in `UNEMUPC_FIXTURE_ROOT`
when it is set (absolute and existing, else a configuration error, the rule
`UNEMUPC_USER_ROMS_DIR` already follows), otherwise the repository of
`pc-harness` itself. In this repository both are the same directory, so
behaviour does not change. Front-end tests take every `machines/` and
`vendor/` path of the core from it and read the two files above at run time.
Paths to the front end's own data stay relative to the crate. Step E2.

| Path family | Owner | Read by front-end tests through |
|---|---|---|
| `machines/*.toml`, `machines/README.md`, `machines/presets/` | core | the fixture root |
| `vendor/` firmware and disk fixtures built by `fetch.sh` / `build.sh`: `kiselev-bios`, `unemu-at-bios`, `vgabios`, `xtide-universal-bios`, `freedos` (boot floppies), `hdd`, `user-roms` | core | the fixture root |
| `vendor/` fixtures only the core reads: `test386`, `at-pm-test`, `at-loadall-test`, `at-privilege-test`, `seabios`, `ltemm` | core | not read |
| [`vendor/kenslab/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/vendor/kenslab/), [`vendor/shareware/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/vendor/shareware/), [`vendor/freedos/fetch-floppy-1.4.sh`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/vendor/freedos/fetch-floppy-1.4.sh) with [`floppy-1.4-licenses.txt`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/vendor/freedos/floppy-1.4-licenses.txt) and [`iso-extract.py`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/vendor/freedos/iso-extract.py) | front end (recipes) | its own `vendor/` |
| [`backlog/docs/design/recipe-corpus/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-corpus/), [`recipe-schema-1.0.json`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-schema-1.0.json), [`mockups/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/mockups/), [`crates/pc-image/data/recipes/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/data/recipes/), [`assets/brand/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/assets/brand/) | front end | crate-relative, as today |
| `Cargo.lock` (the pin assertions of the goldens) | each repository's own | crate-relative, as today |

**(c) Tests that scan the sources of every crate.**
[`crates/image-store/tests/platform_ids.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/tests/platform_ids.rs) walks `crates/` and its allow
lists name three core files (`pc-config/src/compat.rs`,
`pc-config/src/launch/tests.rs`, `pc-core/src/machine/tests.rs`) [REPO].
After the split it scans the front end's crates; the core gets the same scan
over its own crates, with those three entries, in `pc-config`. ASSUMED: no
other test needs a core source file; step E3's ownership check and the
planner's self-test, which already lists every path a crate source names
outside its directory, verify it.

Core sources also read files outside their crate at compile time
(`machines/presets/ibm-5170.toml` in `machine-file`, fixture scripts in
`assembly` tests). These stay inside the core repository and resolve in a git
checkout of it, which holds the whole tree.

### 2.5. Files outside the crates

| Destination | Paths |
|---|---|
| Core | `machines/`, `vendor/` except the front-end rows of §2.4, `scripts/core-host-guard.*`, `scripts/crate-boundaries.external`, `devShells.firmware` of `flake.nix`, `backlog/completed/`, `backlog/archive/`, `backlog/tasks/`, `backlog/drafts/`, the core's docs and decisions (§9) |
| Front end | The front-end rows of §2.4, [`assets/brand/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/assets/brand/), [`.github/actions/setup-unemu/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/.github/actions/setup-unemu/), [`.github/workflows/setup-unemu.yml`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/.github/workflows/setup-unemu.yml), [`scripts/export-layout.sh`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/scripts/export-layout.sh), the front end's docs and decisions (§9) |
| Both, each with its own edited copy | `Cargo.toml`, `Cargo.lock`, `flake.nix`, `flake.lock`, `deny.toml`, `LICENSE`, `LICENSES/`, `README.md`, `AGENTS.md`, `CLAUDE.md`, `.gitignore`, `.envrc`, `.pre-commit-config.yaml`, the lint configurations, `.agents/`, `.claude/`, `.serena/`, `.beads/` configuration, `backlog/config.yml`, `.github/workflows/ci.yml` and `release.yml`, the gate and guard scripts of §8, `vendor/freedos/README.md`, the shared docs of §9 |

Step E3 turns this table into a committed list that a check reads: every
tracked file has exactly one owner or is listed as shared.

## 3. Names, history and trackers

### 3.1. The core keeps `unemupc`

Recommended: the PC core keeps the repository `unemu-org/unemupc`, the
history, the tags `v0.0.1` to `v0.0.3`, the release page, the CI history and
the beads database with prefix `upc`.

- The name already means the core everywhere it is load-bearing: the binary
  `unemupc`, the core's id in `CoreDescription`, "Needs unemupc X or newer"
  and `unemu.min-app-version` (decision-30), the firmware index directory, the
  `UNEMUPC_*` variables, core contract §9 and §12.9.
- The project's goal line (decision-16) and nine of the twelve open epics are
  core work: `cpu-core`, `scheduler`, `validation`, `storage`,
  `consolidation`, `protected-mode`, `win9x-base`, `voodoo` and the
  cycle-ready seams. Two concern both sides (`tests-ci` and the release
  epic); one, `ux`, is the front end's (MEASURED: `bd list --type epic`).
- The amendment of decision-30 keeps the pre-split release numbers as the
  line both components shared; the core continues that line, so no published
  manifest changes meaning.
- Every existing link to the repository (the pinned `setup-upcli` and
  `setup-unemu` commits, release assets, the x86jit cross references) keeps
  resolving.

### 3.2. The front end's name

Proposed: **`unemu-org/unemu`**. It is the binary's name, the name of its host
directories, the name of the action (`setup-unemu`), and it names no single
platform, which fits a front end that registers every core. Alternatives:
`unemu-frontend` (says what it is, but the product is called `unemu`),
`unemu-app`. The maintainer decides (§14 Q2).

### 3.3. How history is carried

| Option | What the new repository holds | For | Against |
|---|---|---|---|
| **A. Shared history** (recommended) | The same commits as `unemupc` up to the split point, then one commit that removes the core's paths | Every commit hash named in `status.md`, in design documents and in bead close reasons resolves in both repositories. Signatures stay valid. `git log --follow` and `git blame` work through the earlier crate splits (`cli`, `window`, `media`). No tool, nothing rewritten, reversible | The front end's history contains the core's past (a 23 MiB pack, MEASURED). The release tags must not be pushed there |
| B. `git filter-repo` over a path set | Rewritten commits that touch only the front end's paths | A smaller history that shows only front-end files | Every hash changes, so the hashes cited in docs and beads resolve only in `unemupc`. Signatures are dropped. History through the crate splits is lost unless the old paths (`crates/cli`, [`crates/window`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window), [`crates/media`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/media)) are kept, and those bring the core's code back. Squash commits that touched both sides keep messages about files that are gone. One-way |
| C. Fresh start | One commit: "import from unemupc at COMMIT" | Simplest to explain | No blame and no log for any file; every reference resolves only in `unemupc` |

Recommended: **A**. The history is already public, so sharing it publishes
nothing new, and the documents of this project cite commits heavily. With A
the new repository is created empty and the history is pushed to it; it is
**not** made with the hosting service's fork function, which would tie its
visibility and its search to the parent. Tags and `refs/dolt/data` are not pushed.

The contract was exported from one directory with its history (core contract
§13.1). That fitted one crate with no history through renames; it does not
fit 22 crates that were carved out of three.

### 3.4. One layout in both

Both repositories keep every file at the path it has today: `crates/<name>/`,
`scripts/`, `backlog/`, and `vendor/` for the fixture families each owns. A
patch made in one applies in the other by path, which is what makes the
rollback of §12.3 cheap.

## 4. Pins and the path of a change

### 4.1. The pin

The front end's workspace manifest names each of the 13 core packages of §2.3
as a git dependency on `https://github.com/unemu-org/unemupc` at one exact
40-digit revision, the same revision in every line:

```toml
assembly = { git = "https://github.com/unemu-org/unemupc", rev = "<40 hex>" }
machine-file = { git = "https://github.com/unemu-org/unemupc", rev = "<40 hex>" }
# ... one line per core package the front end names
```

The rules are those of decision-13 for `x86jit`: never a bare `path`
dependency, never a branch or a tag name, and the revision is a commit of the
core's `main` whose CI is green. The front end does not name `x86jit-core`:
its revision comes with the core's manifest and appears in the front end's
lock file.

**One copy of the contract.** The front end pins `unemu-core-api` and
`unemu-core-files` itself, and so do the PC core and the NES core. Cargo
treats two revisions as two crates, so all three must name the same one. The
resolved-graph check of `scripts/crate-boundaries.sh` (exactly one package
named `unemu-core-api`) moves to the front end unchanged and fails a re-pin
that breaks this.

**Profiles.** A dependency's profile settings are ignored; the front end's
manifest keeps `debug-assertions`, `overflow-checks` and `opt-level = 3` for
`[profile.dev.package."*"]`, which now covers the core crates. The per-crate
`opt-level` entries for core crates go; the one for `media` stays.

### 4.2. Local work against a core checkout

A gitignored `.cargo/config.toml`, as for `x86jit` today
([dev-setup](../dev-setup.md)):

```toml
[patch."https://github.com/unemu-org/unemupc"]
assembly = { path = "<unemupc checkout>/crates/assembly" }
# ... one line per core package in the graph (16; `testkit` is never built)
```

Cargo wants one line per package, so the front end carries a small script
that writes this file for a given checkout. With the override the fixture
root of §2.4 is that checkout without any variable. The planner already
treats a `[patch]` as "not the pinned dependencies" and selects the whole
workspace [REPO: `scripts/affected-tests.sh`, header].

### 4.3. How a change travels

| Change | Where it lands first | Then |
|---|---|---|
| The front end needs something of the PC core | A bead in `upc`, landed in `unemupc` under the core's gate | A re-pin bead in the front end |
| The machine needs something of the CPU | `x86jit`, as today | `unemupc` re-pins (its slow goldens); the front end re-pins `unemupc` |
| The contract changes | A release of `unemu-core-api` | `unemupc` and `unemunes` each re-pin and land; the front end moves the contract pin and every core pin in **one** commit |
| A front-end change that needs no core change | The front end | — |

A re-pin is its own bead: the Definition of Done plus the front end's slow
goldens. The planner needs no new rule for it: a change to `Cargo.toml` or
`Cargo.lock` already selects the whole workspace with the golden tier [REPO:
`scripts/affected-tests.sh`, `full_files`].

### 4.4. The pin rule of each golden

Today every golden asserts that `Cargo.lock` holds the `x86jit` revision it
was recorded against [REPO: `include_str!("../../../Cargo.lock")` in 20
files].

- **Core goldens** keep that assertion unchanged.
- **Front-end goldens** assert the **`unemupc` revision** in the front end's
  lock file, one constant per test file, so that moving the pin makes every
  golden's author look at it. The `x86jit` revision follows from the core's
  and is not asserted a second time.
- The NES golden keeps asserting the `unemunes` revision
  ([`crates/platforms/tests/nes_cartridge.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/platforms/tests/nes_cartridge.rs)).

### 4.5. Which core revisions a release may pin

Development may pin any green commit of the core's `main`. A front-end
**release** pins the commit of a core release tag: decision-30 compares
`unemu.min-app-version` with `CoreDescription::version`, and an unreleased
core commit reports a development level. The front end's release workflow
checks it (§7.5).

## 5. Tests, goldens and fixtures

| What | Core | Front end |
|---|---|---|
| Unit and isolation tests | With their crates | With their crates |
| Emulation goldens (BIOS POST, FreeDOS boots, DOS sessions, test ROMs, scheduler A/B) | `pc-harness`, `assembly`, `machine-file`: 39 ignored tests in `pc-harness` alone (MEASURED) | — |
| Goldens that drive a machine through the front end's command line or store | — | `cli-pc` (`at_games`, `boot_sector_golden`, `freedos_install_golden`, `ide_journal_golden`, `machine_freedos_golden`, `media_files_golden`), `pc-image`, `pc-library`, `pc-session-tests`, `window`, `platforms` (NES) |
| Volume-builder suite `user_media_volumes` | — | `pc-image` (core contract §11 Q4) |
| Recipe conformance | — | CI and release lane |
| Tests that need user-supplied firmware (`user_rom_*`) | `pc-harness` (4 files), `machine-file` (1) | `cli-pc` (4 files), `pc-image` (2), `pc-library` (1) |

**User-supplied firmware.** Unchanged in both: never in CI, never committed,
found by hash under `vendor/user-roms/` of the fixture root and then in
`UNEMUPC_USER_ROMS_DIR`. Each repository keeps the local `user-roms` lane of
`scripts/ci-gate.sh` for its own tests. The catalogue of identities stays in
`assembly`, so the core owns the hardware qualification of a ROM and the
front end only uses the result.

**Fixtures.** Only scripts, pins and notices are committed; the built images
are gitignored [REPO: `.gitignore`, `vendor/`]. After the split:

- The core keeps every fixture script it reads and builds them in its own
  devShell, as today. It no longer downloads the FreeDOS 1.4 Floppy Edition
  (about 330 MB) or the Ken's Labyrinth archive: only recipes read those.
- The front end reads the core's fixtures and machine files from **a checkout
  of `unemupc` at the pinned revision**, named by `UNEMUPC_FIXTURE_ROOT`. A
  script reads the revision from `Cargo.lock`, checks the commit out into a
  gitignored directory and builds the fixtures there **in the core's own
  devShell**, so the assembler and the disk tools are the versions the pinned
  hashes were made with. CI does the same with a second checkout step and
  caches the built outputs by core revision.
- The front end keeps its own `vendor/` for the three recipe families of
  §2.4 and builds them in its own devShell.

Rejected: copying the core's fetch scripts into the front end (two sets of
pins that drift), and building inside Cargo's own checkout of the dependency
(a directory Cargo manages). Later, not now: taking built fixtures from the
images that `unemu-media` publishes.

## 6. Workspace, lock file, toolchain

| Item | Core (`unemupc`) | Front end |
|---|---|---|
| Workspace members | The 17 of §2.1 | The 22 of §2.1 |
| `[workspace.package]` | `version` continues from `0.0.3`; `repository` unchanged | `version` starts at `0.0.3`, the extraction baseline; `repository` is the new URL; same licence, edition and `rust-version` |
| `[workspace.dependencies]` | `x86jit-core`, `unemu-core-api`, `unemu-core-files`, `thiserror`, `anyhow`, `sha2`, `serde`, `serde-saphyr`, `toml`, `libc`, the core's own crates by path | As today without the core's path entries, plus the 13 git lines of §4.1; keeps `unemu-nes-core`, `egui`, `eframe`, `winit`, `cpal` |
| `Cargo.lock` | Regenerated; a check shows no remaining package changes version | Regenerated; the only difference allowed is the source of the 17 core packages (path → git) |
| Rust toolchain | `rust-version` and the flake's stable toolchain, as today | The same `rust-version` at the split. Afterwards the rule is: the front end's toolchain is never older than the pinned core's `rust-version` |
| `flake.nix` default devShell | Rust, `cargo-deny`, `cargo-nextest`, `nasm`, `binutils`, `mtools`, `dosfstools`, `curl`, `unzip`, `qemu`, `jq`, `backlog`, `pre-commit`. No ALSA: nothing in the core links host audio | Rust, `cargo-deny`, `cargo-nextest`, `jq`, `zstd`, what its own fixture scripts and [`export-layout.sh`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/scripts/export-layout.sh) call (`curl`, `unzip`, `mtools`), `backlog`, `pre-commit`, and on Linux `pkg-config` with ALSA. The assembler, the linker tools and QEMU go: the core's fixtures are built in the core's devShell (§5) |
| `devShells.firmware` (SeaBIOS) | Stays | Removed |
| `deny.toml` | Licence list reduced to what the core's graph holds; the CC BY-SA exception for `unemu-pc-core` stays; no advisory is ignored (all four are in the window stack); `allow-git`: `x86jit`, `unemu-core-api` | As today, with `unemupc` added to `allow-git`; both exceptions stay, because `unemu-pc-core` is still in the graph |
| `publish = false` | Stays | Stays |

ASSUMED: Cargo resolves `*.workspace = true` fields of the core's manifests
from the core repository's own workspace when the packages come in as git
dependencies. Step E5 verifies it with the first build.

## 7. CI and releases

### 7.1. Workflows

| Workflow | Core | Front end |
|---|---|---|
| `ci.yml` | Stays, with the lanes `devshell`, `fmt`, `guard`, `clippy`, `deny`, `tests`, `fixtures`, `goldens`. Drops `recipe-conformance` and the FreeDOS 1.4 cache. `clippy` checks the production binary `unemupc` instead of `unemu` | A copy with all lanes, plus: the second checkout of the core at the pinned revision, its fixture cache, the front end's own fixture cache (FreeDOS 1.4), `recipe-conformance` |
| `release.yml` | Builds `unemupc` only; no ALSA step; no recipe job | Builds `unemu` only; keeps the ALSA checks, the recipe conformance job and the [`recipe-schema-1.0.json`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-schema-1.0.json) asset |
| [`setup-unemu.yml`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/.github/workflows/setup-unemu.yml) and [`.github/actions/setup-unemu/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/.github/actions/setup-unemu/) | Removed, with a line in the README naming the new home | Moves here |

Neither CI needs a secret: both run with a read-only token, and the upload
job of a release uses the job's own token with `contents: write` [REPO: the
`permissions` blocks]. Every action stays pinned to a full commit.

### 7.2. Release archives

| | Core | Front end |
|---|---|---|
| Archive | `unemupc-<version>-<target>.tar.gz` | `unemu-<version>-<target>.tar.gz` |
| Holds | `unemupc`, `LICENSE`, `README.md`, `THIRD-PARTY-LICENSES.md`, `notices/` | `unemu`, the same companions |
| Tag | `v<version>` of the core's workspace | `v<version>` of the front end's workspace |
| First after the split | `v0.0.4` | `v0.0.4`, the first release that ships `unemu` |

The front end's archive does not carry `unemupc`: it is the core's program
and comes from the core's release (§14 Q6). ASSUMED: Cargo does not build a
binary of a package that is only a dependency; step E8 confirms it or the
question is moot.

`THIRD-PARTY-LICENSES.md` in the front end must still quote the notices of
components that now sit in the pinned core: the Nuked-OPL3 licence
(`crates/devices/src/opl3/LICENSE`) and the font notices under
`crates/pc-core/assets/fonts/` [REPO: `scripts/third-party-licenses.sh`,
"Components in this repository"]. The script finds them through the manifest
paths `cargo metadata` reports for `devices` and `unemu-pc-core`.

`unemu --about` names where the source of the build is. After the split that
is two places for the PC part: the front end's revision and the pinned core's.
The notice lists, for each compiled-in core, its repository and the revision
in the front end's lock file. This is a licence obligation of a combined
AGPL program, so it lands with the cut (step E8), not later.

### 7.3. `setup-unemu`

The action moves to the front end and installs from the front end's
releases: `unemu-org/unemu/.github/actions/setup-unemu@<commit>`, archive
name `unemu-<version>-<target>.tar.gz`. No release with `unemu` exists
before the split (§1), so there is nothing to stay compatible with; the old
`setup-upcli` remains reachable at old commits of `unemupc` for releases up
to `v0.0.3`. Workflows of `unemu-media` that install the command line change
one `uses:` line after the front end's first release; that change is made in
that repository, by its own session (step E16).

### 7.4. The markers of decision-30 after the split

| Marker | Compared with | Version source after the split |
|---|---|---|
| `unemu.min-app-version` | `CoreDescription::version` of the selected platform's core | The **core** repository's workspace version (`unemu-pc-core`'s package version); rows and `NEXT_RELEASE` in `pc_config::compat` |
| `unemu.min-frontend-version` | The composing application's version | The **front end** repository's workspace version (`window`, `upcli`); rows, `NEXT_RELEASE` and the frozen `EXTRACTION_BASELINE` (`0.0.3`) in `image_store::compat`, `pc_image::compat` and `recipe::compat` |

Both sources exist today as package versions of crates on the right side
[REPO: `crates/pc-core/src/core.rs:44`, `crates/upcli/src/command.rs:92`,
`crates/window/src/shell.rs:2739`]; the split only makes the two numbers free
to differ. Nothing is restamped. "unemupc" in the reason a reader prints
keeps naming the core.

### 7.5. Release checks

| Check | Core | Front end |
|---|---|---|
| The tag names the workspace version; the toolchain matches `rust-version` | Yes | Yes |
| The binaries hold no path of the build host and link only system libraries | `unemupc` | `unemu` |
| Command words run on a headless Linux with and without ALSA | — | Yes |
| Recipe schema equals the generated one; every pinned recipe reproduces | — | Yes |
| The pinned core revision is the commit of a core release tag, and that tag's version is the `unemu-pc-core` version in the lock file | — | **New** |
| One copy of the contract in the resolved graph | Yes | Yes |

## 8. Scripts

| Script | Core | Front end |
|---|---|---|
| `publication-guard.sh`, `publication-guard-test.sh` | Stays | A copy. The rule is the same in both (§11) |
| `core-host-guard.sh`, `.allow` | Stays; it is the core's check | Not needed: no `emulation` crate lives here |
| `crate-boundaries.sh` | Keeps the layer rule (`emulation` may not reach `pc`), the external-crate list (`crate-boundaries.external`) and the one-copy rule. The core-side rule becomes trivial: every package is the core's. The window and command-line families go | Keeps the layer rule for `neutral`, `pc` and `application`, the family rule, the `window` → `upcli` allow line and the one-copy rule. **New:** a package that comes from a core's repository counts as that platform's layer, so a `neutral` crate that names one fails. Today the walk stops at the workspace's edge |
| `crate-boundaries-fixtures/` | The fixtures of the rules it keeps | Likewise |
| `ci-gate.sh` | Without `recipe-conformance`; `fixtures` without the two recipe downloads | With the core checkout before `fixtures`, and `UNEMUPC_FIXTURE_ROOT` exported for `goldens` |
| `bead-gate.sh` | Stays; the production-binary clippy step names `unemupc` | Stays; without the host guard |
| `affected-tests.sh`, `scripts/testdata/affected-tests/` | Its tables shrink to the core: `emulation_crates`, `runner_files` (`crates/pc-harness/src/`, `crates/unemupc/src/`), the data rules for `crates/pc-harness/tests/support/`, `machines/`, `vendor/` | `emulation_crates` holds `media` only; `runner_files` keeps the front end's entries; the data rules keep recipes, corpus, schema, mock-ups and assets. The rules for `machines/` and `vendor/` of the core disappear: those files change only with the pin, which already selects everything with the golden tier |
| `third-party-licenses.sh` | For `unemupc` | For `unemu`, reading the core's notices from the pinned sources (§7.2) |
| [`export-layout.sh`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/scripts/export-layout.sh) | — | Moves here |
| `skills-bootstrap.sh`, `uid-guard.sh` | Stay | Copies |
| New | — | The core checkout for fixtures (§5) and the writer of the `[patch]` file (§4.2) |

The `side` key of `[package.metadata.unemu]` has done its work once the
repositories exist; removing it is a later clean-up in each.

## 9. Docs and decisions

Rule: a document lives where the code it describes lives. A document about
both is kept in both, with one line at its top that says which repository
maintains which part. A link across repositories is a full URL to the other
repository; a reference in prose carries the repository's name, as cross
references to `x86jit` do today.

| Document | Destination |
|---|---|
| `design/spec.md`, `machine-config.md`, `machine-files.md`, `firmware-and-machine-classes.md`, `scheduler.md`, `observation.md`, `audio.md`, `persistent-media.md`, `protected-mode-gaps.md`, `at-pm-test.md`, `at-loadall-test.md`, `at-privilege-test.md`, `compatibility-diagnostics.md`, `machine-time-profile.md`, `testing.md` | Core |
| [`design/machine-images.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md), [`recipes.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipes.md), `recipe-schema-*.json`, [`recipe-corpus/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-corpus/), [`registry-library-plan.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/registry-library-plan.md), [`about-text.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/about-text.md), [`icons.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/icons.md), [`mockups/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/mockups/), `docs/user/` | Front end |
| `design/core-contract.md`, this document | **Both, frozen.** They are the record of the split. After the cut neither is edited except for a closing status line; the living description is each repository's `architecture.md`, and the contract's normative text is in its own repository |
| [`design/machine-images.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md) §5.6 (compatibility) | Front end, with the core's rows described where they live: a short section in the core's docs that points at `pc_config::compat` and decision-30 |
| `architecture.md` | Split. The crate map and the layer table by row; "From configuration to a run", "Virtual clock", "x86jit integration" and the snapshot clock to the core; the store, library, registry and desktop paragraphs to the front end. The layer table is corrected on the way: it still lists `journal` and `journal-files` as `neutral` while their manifests say `emulation` and `pc` [REPO] |
| `commands.md` | Split by section. Core: "Standalone PC core command line", "Offline persistent media", "x86jit (CPU core)", "Guest test-ROMs", and the core's part of "Build / run / test", "CI", "Releases". Front end: "Front end: window and command words", "Local machine images", "Minimum release of an image", "Registry push and pull", "Official template images", "Shareware installation media", every recipe section, "Exporting an image", "Creating disk images" |
| `status.md` | Both keep the entries up to the split as they are: it is a dated log and its links stay valid. One marker line records the split; above it each repository writes only its own entries |
| `dev-setup.md`, `conventions.md`, `decisions.md`, `glossary.md`, `working-on-tasks.md`, `readme.md`, `test-timings.md` | Both, each edited to its own crates, epics and pins. The front end's `dev-setup.md` gains "The PC core: unemupc" beside the contract's section |
| `deferred.md` | Split by entry. Hardware entries to the core; "Machine-image daemon" and "Reusing Docker and oras registry logins" to the front end; "Layered disk images" in both (§11) |
| `backlog/completed/`, `archive/`, `tasks/`, `drafts/` | Core: the history of the earlier tracker |

**Decisions.** Records are not renumbered and not rewritten.

| Destination | Decisions |
|---|---|
| Core | 1 to 10, 12 to 19, 21 to 23, 28, 42 |
| Front end | 24 to 27, 29, 33 to 39, 43 |
| Both | 11 (licence), 20 (audio seam and host output), 30 (the two markers), 32 (platform id), 40 and 41 (the contract and embeddable cores) |

New decisions continue each repository's own series. A reference across
repositories carries the name: `decision-44 (unemu)`, as `decision-13
(x86jit)` does today.

## 10. Beads

- **`unemupc` keeps the database and the prefix `upc`**, pushed to
  `refs/dolt/data` on its `origin` as today.
- **The front end gets a new database.** Proposed prefix: **`ufe`** (§14
  Q4). It is created with `bd init` in the new repository and pushed to
  `refs/dolt/data` on the new `origin`. `bd init` and the first
  `bd dolt push` make a git commit on their own (AGENTS.md); `git log -1` is
  checked after each.
- **The six shared memories** (`roles`, `bead-to-main`, `bead-conventions`,
  `worker-prompts`, `sandbox-workers`, `publication-rule`) are copied into
  the new database and edited for the front end's gate.
- **Which beads move.** A bead goes where the crates it changes go. Closed
  beads stay in `upc`: they are history and their close reasons name commits
  both repositories hold. Open front-end beads are re-created in the new
  database with their text, priority and labels and with
  `external_ref unemupc:<old id>`, the way the earlier migration kept
  `backlog:TASK-NN`; the old bead is closed in `upc` with a reason that names
  the new id. A bead that needs both sides becomes a core bead and a
  front-end bead that waits for the re-pin. At the time of writing 24 beads
  are open (MEASURED: `bd list --status open`), so the triage is done by hand
  in one sitting.
- **Epics.** `ux` moves whole. `tests-ci` and the release epic exist in both:
  each repository has its own gate and its own first release. Every other
  epic stays in `upc`.
- **Ids across repositories** carry the repository's name:
  `upc-sk3.16 (unemupc)`, `ufe-12 (unemu)`, `xj-6vd.9 (x86jit)`. Beads cannot
  depend on a bead of another database; the waiting bead says so in its
  description.

## 11. Publication, licences and constraints that stay

- **One publication rule, in both.** Everything committed is English and
  safe to publish; no proprietary firmware, guest software, manuals or
  screenshots of them, only hashes and our own observations; no private host,
  path or personal setup; the CPU core's other consumers stay anonymous. Each
  repository runs `scripts/publication-guard.sh` in its guard lane and as a
  pre-commit hook. Pushing goes only to the configured `origin`; creating or
  publishing a repository is the maintainer's step.
- **Licences.** Both repositories are AGPL-3.0-or-later; the contract and
  `x86jit` keep their permissive licences. The font assets under CC BY-SA
  stay in `unemu-pc-core` with their notices; the logo (CC0) and the DejaVu
  subset go with `ui-kit`. The source notices of the open firmware
  (`vendor/*/source-notice.txt`) stay with the scripts that build it, in the
  core.
- **Fixtures.** Neither repository commits a built image. The fixture
  scripts, their pins and the licence index of the FreeDOS edition are what
  is committed, each in the repository that owns the family (§2.4).
- **Layered disk images stay possible.** The sector overlay interface, the
  journal format and the disk-layer format remain in the core (`journal`,
  `journal-files`); the store, the OCI envelope and the disk closure remain
  in the front end (`image-store`, `pc-image`). The seam between them is the
  one that exists: storage objects of the contract and
  `journal_files::{create, layer}` called by `pc-image`, now across the pin.
  No codec is copied to the other side, so there is still one implementation
  to extend. An image keeps describing a whole machine: the manifest is the
  front end's, the PC config inside it the core's (`pc-config`).
- **User documentation** waits for the split, as decided. Its plan may start
  after step E15; it is not part of this one.
- **Feature freeze.** The split adds no feature. The three small additions it
  needs are named where they appear: the fixture root (§2.4), the per-core
  source line of `--about` (§7.2) and the release check of §7.5.

## 12. Steps

M = the maintainer's step (anything outward: a repository, its settings, a
push, a tag, a release, visibility). S = the supervisor with workers, under
the normal path from bead to `main`; every landing still waits for the
maintainer's word, as always.

### 12.1. Order

| # | Who | Step | Verified by |
|---|---|---|---|
| E0 | M | Accept this plan and answer §14 | The answers recorded here; status becomes Accepted |
| | | **Preparation, in `unemupc`, one bead each** | |
| E1 | S | `pc-harness` exposes its shared test support behind `test-support`; the 30 front-end includes become `use` (§2.4 a). The boundary script fails when a crate's source includes a file of a crate on the other side | Full gate with goldens. `grep` finds no `#[path]` from a front-end crate into a core crate. A planted include fails the script's self-test |
| E2 | S | One fixture root, `UNEMUPC_FIXTURE_ROOT` (§2.4 b); front-end tests read the core's `machines/` and `vendor/` only through it | Full gate with the variable unset. The front end's tests and goldens green again with the variable naming a second checkout that holds the built fixtures while the first checkout holds none |
| E3 | S | The ownership list of §2.5 as a committed file and a check in the guard lane; the source scan of §2.4 c split in two | The check passes; a planted unowned file and a planted doubly owned file fail its self-test |
| E4 | M | Choose the split point: a commit of `main` after E1 to E3 with green hosted CI. From here front-end changes land only on the front-end branch; core changes land on `main` as usual | The commit is named in the bead and in this document |
| | | **The front end, built locally: nothing is published** | |
| E5 | S | A branch from the split point: the core's paths removed; the workspace of §6 with the 13 git lines at the split point's revision; the lock file regenerated | `cargo metadata --locked` resolves. The lock-file difference is only the source of the 17 core packages. Build and the fast tests green |
| E6 | S | Fixtures through the core checkout (§5); front-end goldens assert the core revision (§4.4) | `goldens` green with every fixture required. Changing the constant in one golden fails it |
| E7 | S | Scripts, guards, planner, `deny.toml`, `flake.nix` as §6 and §8 | Every self-test passes. `cargo deny` clean. A `neutral` crate that names a core package fails the boundary check |
| E8 | S | Workflows, release packaging, `setup-unemu`, `third-party-licenses.sh`, the per-core source line of `--about`, README, AGENTS.md | The whole gate run locally, lane by lane, with the results recorded on the bead. The setup-unemu test archive installs and runs |
| E9 | S | Docs and decisions pruned and split as §9 | The publication guard passes. No relative link points at a removed file |
| E10 | M | Create the repository with the accepted name: empty, not a fork; default branch `main`; the same protection and Actions settings as `unemupc`; no secret | The repository exists and is empty |
| E11 | M | Push `main`: the shared history and the commits of E5 to E9. No tags, no `refs/dolt/data` | Hosted CI green on a cold runner, every lane. An anonymous `cargo fetch --locked` works |
| E12 | S, M agrees to the push | `bd init` with the accepted prefix; memories copied; open front-end beads re-created and the old ones closed with pointers (§10); `bd dolt push` | `git log -1` after each `bd` set-up command. `bd ready` in both databases shows the right work |
| | | **Abort point: see §12.2** | |
| | | **The cut in `unemupc`** | |
| E13 | S | A branch that removes the 22 front-end crates and the front end's files; workspace, lock file, `deny.toml`, scripts, workflows, README, AGENTS.md and docs as §6 to §9 | Full gate with goldens and the local user-ROM lane. `git diff <split point>..HEAD -- crates/<each core crate>` shows no source change, so no golden is re-recorded. The lock file moves no version of a remaining package |
| E14 | M | Land and push E13 | Hosted CI green |
| E15 | S | The front end re-pins to the commit of E14: the first ordinary re-pin. The freeze of E4 ends | The front end's gate with goldens. When no core change landed since the split point, the core crates are byte-identical between the two revisions and only the pin constants change their value |
| | | **Afterwards** | |
| E16 | M; S prepares | Releases: core `v0.0.4`; the front end pins that tag's commit and releases `v0.0.4`. Then the `uses:` line in `unemu-media`, changed in that repository by its own session | Both release workflows green; the checks of §7.5. `setup-unemu` installs the front end's `v0.0.4` |
| E17 | S | Close the record: a closing status line in `core-contract.md` and here, in both repositories; the split marker in each `status.md` | The documents say the split is done and name both commits |

**As landed — E1 and E2 (2026-11-22).** `pc-harness/test-support`
exposes `fixtures`, `session`, `at_games` and `fat12`. All 30 front-end
`#[path]` includes of harness support became imports: 17 in `cli-pc`, 10
in `pc-image`, and one each in `pc-library`, `upcli` and `window`.
Core-side includes and the same-side `cli-pc` → `pc-image` include stay.
`pc-image`, `pc-library` and `pc-session-tests` gain a harness test dependency;
`pc-library` also activates it optionally for its exported `test-support` helpers.

`pc_harness::test_support::root()` selects `UNEMUPC_FIXTURE_ROOT` when set,
requiring an existing absolute directory; an empty, relative, missing or
non-directory value is a configuration error, including optional fixture
reads. Without the variable it uses the harness repository. Front-end
machine and core-vendor paths, including session tests, now use it; the FreeDOS
fetch script and the machine guide are read at run time. Front-end recipe data,
scripts, FreeDOS Floppy Edition media and licence index, and lock-file checks
keep their own paths. Process-isolated tests
exercise the public API, external fixture bytes, default selection and
invalid overrides. The boundary self-test plants all four source-include
forms across the sides, including source-relative and manifest-relative
`concat!` arguments with balanced delimiters and whitespace before `!`.
Rust string literals and nested comments are tokenized so include examples
stay inert; same-side includes still pass. The scanner shares the shell
boundary check's ownership in §2.5.

The supervisor verifies the second-checkout run with no built fixtures in
the first tree and reruns the full gate with user ROMs before landing.

**As landed — E3 (2026-11-22).** `scripts/repo-ownership.paths` records
core, front-end and shared ownership by path pattern, including the separate
FreeDOS recipe files. `scripts/repo-ownership.sh` requires exactly one
matching row per tracked or new, non-ignored file. Its self-test rejects an
unowned path and overlapping ownership, including duplicate rows and a
shared row overlapping a core row. Both CI's guard lane and the bead gate
run the check and its self-test.

The platform-id scan is split between [`image-store/tests/platform_ids.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/tests/platform_ids.rs)
(22 front-end crates) and `pc-config/tests/platform_ids.rs` (17 core crates).
The core retains the three retired-id allow entries from §2.4 (c); neither
scan reads sources on the other side. The affected-tests planner watches
the corresponding crate set for each scan, with regression scenarios for
both sides. No crate dependency changes. The ownership check passes on all 1,157 paths
of the preparation baseline; its failure cases are exercised without
changing the git index. The planner self-test covers 50 external paths.

**E4 — the split point (2026-11-22).** The maintainer chose
`0f2795402e947e235a7b892c931910b05f05d61f` (`0f279540`), the landing of E1
and E2, with hosted CI green on that commit. From here front-end changes
land only on the front-end branch; core changes land on `main` as usual.
The 13 git lines of §4.1 pin this revision until the re-pin of E15.

### 12.2. The abort point

Until E13 lands, `unemupc` holds everything and builds everything. E1 to E3
are improvements that stand on their own. Aborting before E13 costs: the
maintainer archives or deletes the new repository, the freeze of E4 is
lifted, commits made on the front-end side since the split point are
applied to `unemupc` by path (§3.4), and beads moved in E12 are reopened in
`upc` from their pointers. No history is rewritten at any point, before or
after.

### 12.3. Rollback after the cut

After E14, going back is one `git revert` of the removal commit in `unemupc`
(its history still holds every file), followed by applying the front end's
later commits by path. It is possible but it grows with every front-end
commit; the decision to keep the split is effectively made at E14.

## 13. Risks

| Risk | Answer |
|---|---|
| The front end uses 13 core crates beyond the contract; a core change breaks it unseen | It breaks at the re-pin, in the front end's gate, never on the core's `main`. The exact pin is the protection. The `test-support` features are listed in the core's `architecture.md` as used from outside |
| Two copies of the contract after a partial re-pin | The one-copy check in the front end; the rule that the contract and every core move in one commit (§4.3) |
| The front end's lock file resolves a registry crate to another version than the core's lock file, so the core's goldens and the front end's run slightly different graphs | Each repository's goldens are valid for its own lock file, which is what it ships. E5 and E13 start both from identical versions; exact (`=`) requirements stay where behaviour depends on them |
| Fixture bytes depend on the tools that build them | The front end builds the core's fixtures in the core's devShell at the pinned revision (§5) |
| A front-end release pinned to an unreleased core stamps images with a development level | The release check of §7.5 |
| Generic package names (`cpu`, `device`, `machine`, `bus`, `video`, `audio`) meet a same-named package of another core in one graph | Cargo allows same names from different sources; a collision would only make `-p NAME` ambiguous. No core has one today (MEASURED: the NES core's packages are `unemu-nes-core`, `nes`, `x65`). Renaming waits for a real collision (§14 Q9) |
| Front-end work continues in two places between E4 and E13 | The freeze of E4: one side only. The window is short because E5 to E9 are prepared before the repository exists |
| Docs kept in both drift apart | Only frozen records and dated logs are kept in both; living documents are split, not copied (§9) |
| The cold hosted runner fails where the local gate passed | E11 and E14 are verified on the hosted runner before the next step starts |
| Commit hashes in docs stop resolving | They do not with option A of §3.3. With B or C they resolve only in `unemupc`, and every such reference would need the repository's name |
| The split disturbs the layered-image path | No codec or format moves across the line; §11 |

## 14. Questions for the maintainer

**Q1. Which repository keeps the name `unemupc`, the history and the beads?**
Recommended: **the core** (§3.1).

**Q2. The front end's repository name.** `unemu`, `unemu-frontend` or
`unemu-app`. Recommended: **`unemu`** (§3.2).

**Q3. How is history carried?** Recommended: **A, shared history**, pushed to
an empty repository, without tags (§3.3). B (`git filter-repo`) if a history
with front-end files only matters more than resolvable hashes and signatures.

**Q4. The beads prefix of the front end.** Recommended: **`ufe`**. Open
front-end beads are re-created with a reference to the old id; closed ones
stay in `upc` (§10).

**Q5. Is there one more combined release before the cut?** Recommended:
**no**. The baseline is frozen at 0.0.3; the first release with `unemu` is
the front end's `v0.0.4` from its own repository, beside the core's `v0.0.4`.

**Q6. Does the front end's archive also carry `unemupc`?** Recommended:
**no**: each archive holds its own repository's program (§7.2).

**Q7. Must a front-end release pin a tagged core release?** Recommended:
**yes**, checked by the release workflow; development pins any green commit
of the core's `main` (§4.5).

**Q8. Where do the front end's tests get the core's fixtures?** Recommended:
**a checkout of the core at the pinned revision**, built in the core's
devShell (§5). Taking them from published images of `unemu-media` is a later
option, not part of the split.

**Q9. Do the core's generic package names change before the cut?**
Recommended: **no**. Nothing collides, the pin works by name within one
repository, and a rename touches every golden's imports for no gain now.

**Q10. `core-contract.md` and this document: kept whole and frozen in both,
or split by section?** Recommended: **frozen in both** (§9).

**Q11. Is the new repository public from its first push?** Recommended:
**private until E11's hosted CI is green, then made public by the
maintainer**. Its content is already public in `unemupc`, so this protects
only against a half-working repository being seen, and it keeps deletion
clean if the plan is aborted. ASSUMED: the hosted runner's minutes for a
private repository suffice for the few runs needed.

**Q12. Variable names.** The front end keeps reading `UNEMUPC_REQUIRE_FIXTURES`,
`UNEMUPC_USER_ROMS_DIR` and `UNEMUPC_FIXTURE_ROOT`, which name the core's
fixtures. Its own build variables (`UNEMUPC_SOURCE_REV`,
`UNEMUPC_SOURCE_COMMIT`) could become `UNEMU_*`. Recommended: **keep every
spelling for the cut** (core contract §12.9 left them alone on purpose) and
rename the front end's two in a later bead of its own.
