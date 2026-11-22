# Test timings

Measurements below predate the repository split and describe the combined workspace. They are retained as historical evidence, not current core build times. The current core has 17 crates; front-end timing belongs to [unemu](https://github.com/unemu-org/unemu).

Where a change's build and test time goes, measured on 2026-11-17 to decide
what a change must run ([`commands.md`](commands.md#affected-tests-and-the-bead-gate)
has the commands). The first rounds ran the tests under `cargo test`; the
last one under cargo-nextest 0.9.138 ([cargo-nextest](#cargo-nextest)).
Repeat the measurements before a crate split is judged by them.

Setup: a shared development machine with 18 cores, macOS, the Nix devShell
(Rust 1.99.1). Every cargo command ran through a cargo wrapper that limits
jobs on a shared machine: 6 build jobs (`CARGO_BUILD_JOBS=6`), 4 test threads
(`RUST_TEST_THREADS=4`), `nice -n 10`, `CARGO_INCREMENTAL=0`, one target
directory per worktree, in the test profile (optimized hot crates, debug
assertions on). Other builds and tests shared the machine; each number
carries the one-minute load average at its start and end (18 keeps every
core busy). The machine never went idle between 19:08 and 21:54 (one-minute
load 4–56), so there is no idle repeat yet. Load moves the times by up to
half: under `cargo test` the window's binary unit tests took between 82 s
and 124 s over eight runs.

## Build

`cargo test --workspace --no-run --timings` into an empty target directory:
**200 s** (load 5 → 26). The pinned CPU core, `x86jit-core`, built at
opt-level 3, takes **147 s** of it alone, and `cpu` and every crate above it
wait for it. The other 260 dependency units take 208 s of compile time and the
17 workspace crates 262 s, most of it in the 47 s after the CPU core
finishes.

Compile time per workspace crate (the sum of its library and test-target
units, so it exceeds the crate's share of the wall time):

| Crate | Seconds | Largest unit |
| --- | ---: | --- |
| cli | 62 | 56 test binaries, about 1 s each; the library 1 s |
| machine-file | 43 | library tests 11 s, library 11 s |
| assembly | 40 | library tests 18 s |
| window | 37 | the `unemupc` binary's tests 22 s, the binary 13 s |
| pc-image | 25 | library 7 s, library tests 5 s |
| devices | 14 | library tests 9 s |
| image-store | 10 | library tests 3 s |
| media | 8 | library tests 4 s |
| machine | 6 | library tests 2 s |
| image-registry | 4 | library tests 3 s |
| chipset, cpu, bus, audio, device, video, testkit | 1–2 each | |

A warm target directory rebuilds only the changed crate and its dependents
(the scenarios below). Selecting a subset with `-p` can rebuild a few shared
crates once: cargo unifies features over the selected packages only, so
testing the desktop package builds `machine`, `assembly`, `machine-file`,
`pc-image` and `cli` without the reference scheduler that `cli`'s
dev-dependencies switch on in a workspace build (24 s, once per target
directory).

A copy-on-write clone of a warm target directory (`cp -c -R` on APFS,
3 s for 8 GB) serves a copy of the tree at another path without building
anything (0 s, load 9); touching one window source then rebuilt the window
only (20 s). Cargo hashes workspace crates by their path inside the
workspace, and the clone keeps file times. A gate that starts each new
worktree from such a clone of an up-to-date main target skips the 200 s clean
build.

Setting `RUSTC_BOOTSTRAP` for cargo rebuilds 58 crates (the proc-macro stack
and everything above it: 110 s, then 51 s to return). Pass libtest's unstable
`--report-time` by running a built test binary directly instead.

## Tests

Cargo runs test binaries one after another, so libtest's `finished in` times
add up to the run; parallelism exists only inside one binary, up to the 4
test threads.

| Run (whole workspace) | Wall | Load |
| --- | ---: | --- |
| fast tier: `cargo test` | 399 s | 5 → 9 |
| golden tier: `-- --include-ignored --skip user_rom_`, fixtures required | 464 s | 31 → 5 |
| every tier with user-supplied ROMs (a gate run earlier the same day) | 671 s of binaries | not recorded |

Per crate, the sum of its test binaries (fast tier, golden tier). The two
runs saw different load, which is why `pc-image`, with no ignored tests, reads
149 s and 99 s:

| Crate | Fast | Golden |
| --- | ---: | ---: |
| cli | 38 s | 178 s |
| pc-image | 149 s | 99 s |
| window | 131 s | 94 s |
| media | 45 s | 31 s |
| image-store | 18 s | 38 s |
| image-registry | 7 s | 10 s |
| machine-file, assembly, devices | under 4 s each | under 5 s each |
| the other eight | under 1 s each | under 1 s each |

The slowest binaries and what bounds them (per-test times from libtest's
`--report-time`, run on the built binaries directly):

- The window binary's 769 unit tests: 82 s at 4 threads (load 8 → 7),
  111 s at 2 and 203 s at 1, with 294 s of test time at 4 threads. Two
  sound-level tests boot the open games machine and take 21 s each, so more
  threads could bring the binary to about 25–35 s, not lower. Its ignored
  tests add 13 s and 21 s (shell boots) and 3 s (headless window driver).
- `pc-image`'s `publication_death`: 34–71 s in one test that runs a child
  process at every publication boundary, one after another; threads do not
  help.
- `media`'s library tests: 28–42 s, of which two overlay tests take 39 s and
  19 s.
- `pc-image`'s `workspaces` 18–39 s, `template` 12–18 s and `large_disk`
  7–14 s; `image-store`'s `metadata` 8–15 s and `store` 6–13 s.
- The CLI goldens: `freedos_install_golden` 22 s, `scheduler_ab` 16 s,
  `media_files_golden` 14 s, `ide_journal_golden` 8 s,
  `machine_freedos_golden` 8 s, `xms_warm_boot_golden` 7 s; about 140 s
  together.
- The user-ROM tests add about 210 s: `landmark` 58 s, `at_games` 52 s on
  top of its open goldens, `vga_bios` 45 s, `machine-file`'s `parts` 42 s,
  `at_pm_test_golden` 12 s.

## Shared-machine limits

The 4 test threads cost the window binary the most: 82 s where its test time
and longest tests allow about 30 s. Most other slow binaries are bound by a
single long test, which more threads do not shorten. Under `cargo test` the
bigger loss is that the binaries run serially: about 130 test binaries, most
of them a few seconds of one or two tests, leave the cores idle. Nextest
overlaps them within the same 4 tests at a time (below).

The 6 build jobs matter little to the clean build: its critical path is one
crate, the CPU core (147 s). They were not raised for these measurements,
since the machine was shared.

## cargo-nextest

Nextest runs every test in its own process, at most 4 at a time across all
binaries here, so a binary's long tests overlap other binaries' tests. The
whole workspace, warm, one at a time through the cargo wrapper:

| Run (whole workspace) | `cargo test` | nextest | Load during nextest |
| --- | ---: | ---: | --- |
| fast tier | 399 s | 254 s (tests), 267 s with guard, doctests and release-profile devices | 10 → 4 |
| golden tier (`user_rom_*` excluded) | 464 s and 427 s | 321 s, 337 s in all | 6 → 11 |
| release profile, `--workspace --release` | 522 s (with the build) | 317 s (80 s build, 235 s tests) | 4 → 7 |

A process per test costs about 75 ms of start-up for the window binary, whose
tests are mostly short: its 769 unit tests take 537 s of test time under
nextest against 294 s in one process (median 93 ms against 18 ms), so a
window-only change runs slower under nextest (see the scenarios). Two library
tests that build a thousand-image catalogue took 12–34 s each under nextest
against 11 s in one process, depending on load. Everything else gains from
the overlap.

The same tests run and pass under both runners. Comparing every test of the
fast-tier, golden-tier and release runs one by one: no test passes under
`cargo test` and fails or is missing under nextest, or the other way round.
The totals differ only where `cargo test` also counts a child process's own
`test result` line: `cli`'s `quantum` tests run themselves again under a
watchdog, so `cargo test` reports 3 or 4 passes for its 2 tests. Nextest
reports one test as leaking a handle past its end in the release run only,
`window`'s `library_view::layout_tests::a_narrow_mini_table_keeps_every_cell_inside_its_outline`
(passed, 0.26 s), and flags `publication_death` as slow (over 60 s) in every
run. Doctests, which nextest does not run, go through `cargo test --doc`; the
workspace has one.

## Scenarios

`scripts/affected-tests.sh` on one changed file each, in a warm target
directory (the file was touched so that cargo rebuilt what an edit would),
one at a time through the cargo wrapper, fixtures required. Every run also
spent 5–13 s on the publication guard.

With the final plan (a changed crate's own ignored tests, and those that name
a changed crate, run by name in the fast tier) under nextest:

| Change | Plan | Build | Tests | Ignored tests by name | Total | Load |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| docs only | checks | — | — | — | 12 s | 13 → 16 |
| window only | window, fast | 41 s | 143 s | 3 tests, 22 s | 221 s | 16 → 7 |
| cli only | cli and window, fast | 15 s | 138 s | 35 tests (cli's own, the window's that name `cli::`), 71 s | 232 s | 7 → 7 |
| image-store | 6 crates, fast | 24 s | 211 s | 5 tests that name `image_store::`, 22 s | 271 s | 7 → 6 |
| devices | 8 crates, goldens | 33 s | 286 s, then 11 s for the release-profile devices tests | — | 340 s | 6 → 6 |
| `Cargo.lock` | everything, goldens | 0 s | 319 s, then 9 s release-profile devices | — | 337 s | 6 → 11 |
| `Cargo.lock` with `--tier fast` | everything, fast | 0 s | 252 s | — | 267 s | 10 → 4 |

The same final plan under `cargo test`, for comparison:

| Change | Build | Tests | Ignored tests by name | Total | Load |
| --- | ---: | ---: | ---: | ---: | --- |
| window only | 11 s | 109 s | 39 s | 165 s | 6 → 36 |
| cli only | 15 s | 154 s | 153 s (the cli goldens one binary after another) | 329 s | 36 → 5 |

The first plan (no ignored tests by name) under `cargo test`:

| Change | Plan | Build | Tests | Total | Load |
| --- | --- | ---: | ---: | ---: | --- |
| docs only (`backlog/docs/commands.md`, `README.md`) | checks | — | — | 7 s | 14 |
| window only (`crates/window/src/pacing.rs`) | window, fast; image-store `platform_ids` | 24 s, of which most is the once-per-target desktop package feature set; 8 s for `platform_ids` | 96 s | 134 s | 14 → 11 |
| cli only (`crates/cli/src/image.rs`) | cli and window, fast | 15 s | 131 s, the window binary 94 s of it | 154 s | 11 → 17 |
| image-store ([`crates/image-store/src/library.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/src/library.rs)) | image-store, image-registry, machine-file, pc-image, cli, window; fast | 30 s | 252 s | 291 s | 17 → 7 |
| devices (`crates/devices/src/pit8253.rs`) | devices, chipset, machine, assembly, machine-file, pc-image, cli, window; goldens; release-profile devices | 32 s | 437 s, then 118 s for the release-profile devices tests (their first release build) | 595 s | 7 → 8 |
| cli golden test file and [`crates/window/src/shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) | cli and window, fast; their three ignored tests by name, in the same build | 16 s | 142 s, then 32 s for the three ignored tests | 180 s | 14 → 21 |
| `Cargo.lock` | everything; goldens; release-profile devices | 0 s here (the lock did not really change; a new lock rebuilds everything, 200 s) | 427 s | 434 s | 8 → 11 |

`scripts/bead-gate.sh` on the front-end changes (fmt, guard, clippy, tests,
rustdoc; no deny), one at a time through the cargo wrapper:

| Change | Target directory | fmt | guard | clippy | tests | rustdoc | Total | Load |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| cli only | warm for tests, first clippy and rustdoc | 1 s | 5 s | 29 s + 7 s front-end command line | 185 s | 6 s | 234 s | 8 → 10 |
| window only | warm | 1 s | 5 s | 8 s | 127 s | 5 s | 147 s | 10 → 7 |
| cli only | warm | 2 s | 5 s | 6 s | 174 s | 1 s | 188 s | 7 → 7 |
| cli only | empty, as for every new worktree today | 2 s | 8 s | 42 s + 8 s front-end command line | 318 s, of which 174 s build | 10 s | 388 s | 7 → 18 |

The rows above ran the first plan under `cargo test`. With the final plan
under nextest, warm:

| Change | fmt | guard | clippy | tests | rustdoc | Total | Load |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| cli only | 1 s | 5 s | 6 s + 1 s front-end command line | 202 s (132 s tests, 68 s for 35 ignored tests by name) | 2 s | 218 s | 5 → 7 |
| window only | 1 s | 6 s | 5 s | 129 s (106 s tests, 22 s for 3 ignored tests by name) | 1 s | 143 s | 7 → 5 |

Clippy and rustdoc build their dependencies without code generation, so even
from an empty target directory they cost under a minute; the test build is
what a new worktree pays for.

## What keeps a change from gating in five minutes

A window-only or CLI-only change gates in 2.5–4 minutes once the target
directory is warm: 143 s and 218 s with the final plan under nextest. Under
`cargo test` the CLI-only change would take more than five minutes, because
its 35 ignored tests run one binary after another (153 s instead of 71 s).
What still stands in the way:

1. **A clean build in every new worktree.** Each worktree gets its own empty
   target directory, so its first gate pays the clean build (200 s, 147 s of
   it the CPU core) before any test runs. Seeding the new directory with a
   copy-on-write clone of an up-to-date main target removed all of it in the
   experiment above. That is a change to how the cargo wrapper creates
   target directories, not to the repository.
2. **The window depends on `cli`.** A cli change retests the window, whose
   769 unit tests take 82–124 s at 4 test threads and dominate every
   front-end gate. The window calls `cli::command::run`, `cli::create`, the
   image providers, the push plan, the registry client, the about text and
   the preferences path. Splitting the CLI core from the PC provider, then
   the front end, keeps a command-line change out of the window's tests only
   if the window stops depending on the crate the change lands in; a change
   to the shared core will still retest it.
3. **Four test threads.** Nextest now overlaps the test binaries (the
   whole fast tier 399 → 254 s, the golden tier 464 → 321 s), but at most 4
   tests run at once, and the window's short tests pay a process start-up
   each. The window binary would take about 30 s on all cores.
4. **Single long tests.** `publication_death` (34–71 s), media's
   sparse-overlay comparison (39 s), the window's two sound-level boots (21 s
   each) and pc-image's `workspaces` (18–39 s) set the time of every
   image-store, media or pc-image change: an image-store change took 291 s
   without clippy and rustdoc.
5. **Emulation changes need the goldens.** The goldens of the crates a
   devices change reaches take 286 s under nextest (437 s under `cargo
   test`), plus 118 s for the first release-profile devices build in a
   target directory: a devices change gated in 340 s with a warm target,
   above five minutes until the goldens get shorter or run on more threads.
