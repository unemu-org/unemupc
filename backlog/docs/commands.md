# Commands

Commands for the PC core and standalone `unemupc`. The front end
and `setup-unemu` live in [unemu](https://github.com/unemu-org/unemu).

## Standalone PC core command line

```sh
cargo run --offline -p unemupc -- --boot
cargo run --offline -p unemupc -- --bios vendor/kiselev-bios/bios-xi8088.bin
cargo run --offline -p unemupc -- machine check machines/at-freedos.toml
cargo run --offline -p unemupc -- run --machine machines/at-freedos.toml --max-steps 12000000 --idle-limit 1000000 --print mda
cargo run --offline -p unemupc -- media generations --base disk.img --journal disk.upfj
cargo run --offline -p unemupc -- media verify --base disk.img --journal disk.upfj
cargo run --offline -p unemupc -- media rollback --base disk.img --journal disk.upfj --to 0 --backup before.upfj
cargo run --offline -p unemupc -- media commit --base disk.img --journal disk.upfj --to 0 --output restored.img
cargo run --offline -p unemupc -- --version
cargo run --offline -p unemupc -- --about
```

`unemupc` runs without the frontend's store, registry or recipes.
The journal commands use exit codes: 0 success, 1 host/storage failure,
2 usage/configuration refusal, 3 recovery needed. `compact` aliases `commit`;
`verify --repair-tail --backup PATH` preserves the original journal before
repair. Commit writes a new base file and never rewrites the old base.
Journal failure behavior changed with the shared file adapter: a failed append
rolls back to the length before that append. It returns `Storage` if rollback
succeeds and `StorageUncertain` if it cannot undo the write. The journal still
latches either failure and stops output; guest writes remain readable in memory.
Earlier complete frame appends remain, even if their command was not acknowledged.
`media verify` can therefore report a shorter discarded tail, or none, after a
failed append. Actual process-death tails and unsuccessful rollbacks still pass
through normal recovery; the wire format, recovered-command rules and retention
are unchanged.

Journal paths now also refuse the shared engine's reserved sidecar namespace
with the Usage message `reserved sidecar path`: `.lock`, `_lock`, `.create-`
and `.retain-` markers, case variants, and legacy `_N.tmp` temporary suffixes.
Both mounting and offline tools apply this rule before opening journal bytes
or creating lock metadata, including names reached through a symlink. This is
an explicit behavior change. Generated `<sha256>.upfj` and `<set>-<id>.upfj`
names are unaffected. Existing journal-specific `.export-` restrictions remain.

The shared implementation is `unemupc::journal`, using `journal-files`;
The standalone journal regression tests live in this workspace.
`--boot`, `--bios PATH`, `machine check FILE` and `run --machine` select the
standalone runner. Firmware comes from the machine file and repeated `--rom-dir DIR` roots;
`--allow-unlisted-firmware`,
`--diag-at` and `--trace-exceptions` retain the machine runner's semantics.
`--store` and `--no-store` are refused here. Usage names `unemupc`, and
version/about list only the PC core and its notices. Core release archives
ship `unemupc`.

## Build / run / test

```sh
cargo build                                  # build the workspace
cargo run -p unemupc -- --boot                   # Real16 timer demo; no args print usage
cargo run --offline -p unemupc -- --bios vendor/kiselev-bios/bios-xi8088.bin
cargo run --offline -p unemupc -- --bios /path/to/bios.bin --allow-unlisted-firmware
cargo run --offline -p unemupc -- machine check machines/at-freedos.toml
cargo test --workspace                       # unit + golden + integration
cargo test --release -p devices              # release-only device behaviour (CI runs it too)
cargo test --workspace -- --ignored          # slow goldens: BIOS POST, FreeDOS boot, DOS sessions
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --workspace -- --include-ignored # CI goldens lane: every fixture test and the goldens; fail if a fixture is missing
# Corrected per-clock oracle versus production deadlines (no environment gate):
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --offline -p pc-harness --test scheduler_ab -- --include-ignored --nocapture
# Opt-in host timing only, BIOS POST and FreeDOS prompt, median of three per engine:
UNEMUPC_REQUIRE_FIXTURES=1 UNEMUPC_SCHEDULER_WALL_TIMES=1 cargo test --offline --release -p pc-harness --test scheduler_ab scheduler_engine_wall_times -- --ignored --nocapture
# DOS sessions and their final console screens (requires fetched fixtures):
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --offline -p pc-harness --test dos_interactive_golden -- --ignored --nocapture
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --offline -p pc-harness --test dos_write_golden -- --ignored --nocapture
# FreeDOS boots A: with the open AT BIOS and XT+ ROM, then lists readonly C:
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --offline -p pc-harness --test ide_freedos_golden -- --ignored --nocapture
# Open AT protected-mode test floppy: INT 15h 88h/87h, PM entry, 8042/triple-fault resets, A20 (design/at-pm-test.md)
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --offline -p pc-harness --test at_pm_test_golden -- --include-ignored --nocapture
# Open AT LOADALL test floppy: real-mode moves above 1 MiB through LOADALL caches (design/at-loadall-test.md)
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --offline -p pc-harness --test at_loadall_test_golden -- --include-ignored --nocapture
# Open AT privilege-level test floppy: IRET to ring 3, INT, #GP and a call gate back to ring 0 on SS0:SP0 (design/at-privilege-test.md)
UNEMUPC_REQUIRE_FIXTURES=1 cargo test --offline -p pc-harness --test at_privilege_test_golden -- --include-ignored --nocapture
cargo test -p devices                        # one crate's tests
```

The emulation goldens (BIOS POST, FreeDOS boots and sessions, floppy, IDE,
VGA, EMS, the sound cards, the scheduler A/B tests) live in
`crates/pc-harness/tests/`, beside the machine runners they drive, with the
test support other crates include by `#[path]` in
`crates/pc-harness/tests/support/`: run them with `-p pc-harness`.
Ordinary tests and ignored goldens share compiled artifacts because both are
`cargo test` runs in the same profile. The default `test` profile inherits the
workspace's `dev` settings, which lets `cargo build` and `cargo test` also share
library artifacts. Use the same profile for ordinary tests and goldens;
`cargo test --profile test --workspace -- --ignored` makes the choice explicit,
as the CI gate does. Keep the separate release device run to cover behaviour
that intentionally differs when debug assertions are disabled.

Dependencies, including the pinned CPU core, use optimization level 3. The hot
workspace crates (`devices`, `machine`, `bus`, `cpu`, `assembly`, `audio`,
`chipset`) also use level 3. Debug information, debug assertions and
overflow checks remain enabled; frontends and the other workspace crates keep
their unoptimized dev settings.
Optimized code can be harder to step through even with debug information. For a
focused device investigation, temporarily override just that crate:

```sh
cargo --config 'profile.dev.package.devices.opt-level=0' test -p devices
```

## Affected tests and the bead gate

Run only what a change can affect (in the devShell, from any directory):

```sh
bash scripts/affected-tests.sh --plan        # what would run, and why; builds nothing
bash scripts/affected-tests.sh               # changes since the merge base with main, working tree included
bash scripts/affected-tests.sh --base origin/main --tier goldens
bash scripts/affected-tests.sh --all --tier all   # everything, every tier: before a release
bash scripts/affected-tests.sh --self-test   # the planner on its fixtures, the rules against the sources
AFFECTED_RUNNER=cargo bash scripts/affected-tests.sh   # cargo test instead of cargo-nextest
bash scripts/bead-gate.sh                    # fmt, guard, clippy, tests, rustdoc, deny: only what the change affects
AFFECTED_CARGO='nice -n 10 cargo' bash scripts/bead-gate.sh --log-dir "${TMPDIR:-/tmp}/gate-logs"
```

The runner takes the changed files (the merge base of `--base`, default
`main`, against the working tree, plus untracked files) and maps each to the
workspace crate whose directory holds it. Untracked files count: a stray
file in the tree widens the plan, and one that no rule knows selects
everything, so keep logs and scratch files outside the repository. Files
outside the crates map to the crates that read them: shipped machine files,
fixture scripts and pins under `vendor/`, the test support that several
crates include with `#[path]` from `crates/pc-harness/tests/support/`, the shipped machine files and fixture families. Every crate that depends
on a changed crate (normal, build or dev dependency, from `cargo metadata
--no-deps`) is affected too. The root `Cargo.toml`, `Cargo.lock`, the flake
and toolchain files, a `[patch]` section in `.cargo/config.toml` (the local
CPU-core override of [`dev-setup.md`](dev-setup.md), which means the tests do
not run against the pinned core) and any path no rule knows select the whole
workspace with the goldens. Every run starts with the checks that need no
build: the publication guard (unless `--no-guard`), `git diff --check` of the
changed documentation (untracked files included), `bash -n` of every changed
shell script (`scripts/`, `vendor/*/fetch.sh` and `build.sh`) and, when the
runner or its fixtures changed, its self-test. Documentation, script,
repository and tracker configuration (`.github/`, `.beads/`) changes run only
these. A Rust change anywhere under `crates/` also runs pc-config's
`platform_ids`, which reads the core crates' sources, and an affected `devices`
also runs its release-profile tests. The plan lists every file with the rule
that mapped it.

Tiers select among the tests of the affected crates by libtest's ignore flag
and the `user_rom_` name prefix; no test attribute changes for them:

| Tier | Runs | Chosen |
| --- | --- | --- |
| fast | the affected crates' tests that are not ignored, plus some ignored ones by name (below) | by default |
| goldens | plus every ignored test except `user_rom_*` | an emulation crate, `crates/pc-harness/src/`, `crates/unemupc/src/`, a shipped machine file, fixture, shared golden support, workspace manifest, lock file, toolchain, local `[patch]` or unknown path changed; or `--tier goldens` |
| user-roms | auto plus the ignored `user_rom_*` tests | `--tier user-roms`, with `UNEMUPC_USER_ROMS_DIR` set |
| all | goldens and user-ROM tests | `--tier all`; before a release with `--all` |

By default the runner also names ignored tests of a directly changed crate,
and ignored tests in affected files that name that crate. Explicit `--tier fast`
disables those additions. Emulation and runner changes select goldens instead.
The bead gate requires fixtures so missing inputs fail rather than skip.

The tests run under cargo-nextest when the devShell provides it, otherwise
under `cargo test`; the runner says which. Nextest runs each test in its own
process and several test binaries at once, which cargo runs one after
another; it selects the same tests (`--run-ignored` and a filter expression
on `user_rom_` and the names), and doctests, which nextest does not run, go
through `cargo test --doc`. Both runners run at most `AFFECTED_TEST_THREADS`
tests at once, default `RUST_TEST_THREADS`, else 4, never unbounded.
`AFFECTED_RUNNER=cargo` forces `cargo test`. Each failed test is listed by
name in the summary.

`AFFECTED_CARGO` is the command that runs cargo (default `cargo`), so a wrapper
that bounds parallelism on a shared machine goes there; `AFFECTED_LOG_DIR`
keeps the runner's logs. The runner prints each step's time and the slowest
tests (nextest) or test binaries (`cargo test`, from libtest's `finished in`).
`--files FILE` plans from a list of paths instead of git, and `--metadata
FILE` from saved `cargo metadata` output; the self-test uses both with
`scripts/testdata/affected-tests/` (a snapshot of the workspace graph and of
the sources' ignored tests, the scenarios and their expected plans).

`scripts/bead-gate.sh` gates one change: `cargo fmt --check`, the publication
guard and the core host-interface self-test and check, clippy with `-D warnings` on the affected crates (`--all-targets
--all-features`, plus the production `unemupc` binary when `unemupc` is affected),
the affected tests in their tier, rustdoc with `-D warnings` for the
affected crates, and `cargo deny` only when `Cargo.lock`, a manifest or
`deny.toml` changed. Every step runs after a failure; the summary gives each
step's result and seconds. It takes the runner's `--base`, `--tier`,
`--all` and `--files` and `--log-dir DIR`.

CI (`.github/workflows/ci.yml`) still runs every lane on the whole workspace
for pushes to `main` and pull requests, and the release checklist
([Releases](#releases)) runs `--all --tier all`. Measured build and test times, and what keeps a change
from gating within five minutes, are in [`test-timings.md`](test-timings.md).
When the rules miss a path a crate reads, add it to the planner's
`data_rules` in `scripts/affected-tests.sh`; the self-test fails until a
path a crate source names outside its directory is mapped to that crate, and
until every crate whose sources build paths from the repository root reads
both `machines/` and `vendor/`.

## Offline persistent media

```sh
```

The same `media generations`, `verify`, `verify --repair-tail`, `rollback`,
`commit` and `compact` commands accept CMOS NVRAM journals. Pass the original
64-byte `cmos.bin` with `--base` and its separate journal with `--journal`.
Exports remain 64 bytes, preserving the original clock/control prefix and the
selected generation's general RAM (0Eh–3Fh except the base-supplied shutdown
byte 0Fh). See `design/persistent-media.md`
§ CMOS NVRAM journals.

Inspections replay read-only under a shared stable journal lock. An existing
lock is opened read-only, so a 0444 lock file is supported. If the lock is absent
and its directory forbids creation (EACCES, EPERM or EROFS), inspection proceeds
without a lock and prints one line saying so. A writing session must already
have created that lock; an existing lock is never bypassed. Mutating tools still
require an exclusive lock with write access. A live journal session excludes all
tools. Never delete `<journal>.lock`: it is not a stale-PID file; deletion during
a session defeats the single-writer guarantee for both sessions and tools.
`generations` includes the base SHA-256, retained
anchor, closed ids/reasons/end offsets/counts and any unsealed tip. A closed
marker does not certify filesystem consistency or bootability.

`verify` reports binding/frame/grammar failures, recoverable-tail bytes and the
latest reconstructible state. Exit codes: **0** fully valid/closed, **1**
corruption, wrong base or I/O failure (including a missing journal and lock
conflicts, a dangling journal symlink or a
post-lock canonical-path change), **2** usage/config error, **3** recoverable tail
or unsealed commands. Reserved journal temporary names in any input/output,
including the base and resolved symlink targets, are configuration errors (exit 2).
Missing base/journal inputs are checked before locking,
leave no new files and report the input's role and path. Non-regular inputs,
including FIFOs, are refused without blocking. `--repair-tail` takes an exclusive
lock, synchronizes a full backup at an unused path, then performs normal mount
recovery/sealing. Interior damage and incomplete anchors are refused.

Rollback requires a retained closed generation; generation 0 is available only
before retention introduces a positive anchor. An unavailable id is exit 2.
After synchronizing the full backup and its directory, rollback truncates after
the chosen marker (128 bytes for 0) and synchronizes the journal. Later history
and any open tip are removed; their ids can be reused on subsequent mounts.
Keep the immutable base alongside backups. Existing backup paths (including
aliases of inputs) and reserved `.create-` filenames are refused; tools never
delete backups. On Unix a backup inherits the journal's exact permission bits
before any bytes are copied. A copy or backup-sync failure reports the retained
partial, invalid backup's path; do not use it as a valid recovery source. All I/O
errors identify the affected base, journal, lock or backup path. Tail diagnostics name
the safe truncation offset and discarded byte count separately from the failure
offset. Journal and image bytes stay unchanged on inspection.

`media commit` (alias `compact`) exports a retained closed generation, including
available generation 0, to a new raw image. It requires a shared lock and a valid
sealed tip; repair/seal interrupted history first. An occupied output, including
an input alias, is exit 2. The same-directory temporary is read back for size and
SHA-256 verification, synchronized, published by a no-clobber hard link, then its
directory is synchronized. Neither input changes. Select the new base and a fresh
journal path explicitly when continuing from an export; config is never edited.

Journal mode applies `keep_generations` after each successful checkpoint/session
close. The count includes the oldest retained point encoded as an anchor; keep=2
over ids 1–5 retains anchor 4 and id 5. Later ids/reasons survive; sequences restart
at 1. Retention verifies all retained states before file sync, atomic journal
rename under the stable lock, and directory sync. Maintenance failures
produce one warning per affected drive after every drive has been finished; they
do not turn a durable session close into a run failure. Pre-publication failures preserve the old
journal and permit retry at a later close; publication/sync uncertainty requires
reopening and verifying before further persistent work. Retention keeps exact
journal mode bits, but the replacement is owned by its creator. The next exclusive
mount removes this journal's regular decimal-named retention leftovers after safe
identity checks; it never follows a symlink or mounts a leftover. Reserved
`.retain-` and `.export-` names, like `.create-`,
cannot be mounted as journals or used for backups. Temporaries are never imported,
and backups are never pruned.

### Journal session walkthrough

Journal mode is available in Rust `MachineConfig` data and in TOML/YAML
machine-file floppy drive tables with `write = "journal"` and an explicit
`journal` path. The machine file declares the journal path; the standalone runner uses it when
loading that configuration.
This runnable Rust example uses the small WRITE DATA preset: it programs DMA
channel 2, writes one sector, drains the controller result and halts. Run from
the repository root in the devShell. Generated media and the example project's
build output stay in a new temporary directory; `cargo build -p unemupc` uses the
repository's `target/` directory.

```sh
repo_dir="$PWD"
demo="$(mktemp -d)"
mkdir "$demo/src"
cat > "$demo/Cargo.toml" <<EOF
[package]
name = "journal-demo"
version = "0.1.0"
edition = "2021"

[dependencies]
assembly = { path = "$repo_dir/crates/assembly" }
EOF
cat > "$demo/src/main.rs" <<'RS'
use assembly::{assemble, at_fdc_write, ImageSource, RunPolicy, WriteMode};
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let directory = PathBuf::from(args.next().expect("media directory"));
    let value: u8 = args.next().expect("byte value").parse()?;
    let mut guest = vec![0xFA, 0xFC];
    for (port, byte) in [
        (0x3F2u16, 0x1C), (0x0A, 6), (0x0C, 0), (0x0B, 0x4A),
        (4, 0), (4, 6), (0x81, 0), (5, 0xFF), (5, 1), (0x0A, 2),
    ].into_iter().chain([5, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF].map(|b| (0x3F5, b))) {
        guest.extend([0xBA, port as u8, (port >> 8) as u8, 0xB0, byte, 0xEE]);
    }
    guest.extend([0xEC; 7]);
    guest.push(0xF4);
    let mut config = at_fdc_write(guest, vec![value; 512], 0x600);
    let base = directory.join("disk.img");
    if !base.exists() {
        std::fs::write(&base, vec![0; config.drives[0].kind.image_bytes() as usize])?;
    }
    config.drives[0].image = ImageSource::File(base);
    config.drives[0].write = WriteMode::Journal {
        path: directory.join("disk.upfj"), keep_generations: 10,
    };
    let mut machine = assemble(&config)?;
    println!("{:?}", machine.run_session(&RunPolicy::natural(Some(10_000), None))?);
    machine.finish_persistence()?;
    Ok(())
}
RS
cargo run --offline --manifest-path "$demo/Cargo.toml" -- "$demo" 17
cargo run --offline --manifest-path "$demo/Cargo.toml" -- "$demo" 34
cargo build --offline -p unemupc
(
  cd "$demo"
  "$repo_dir/target/debug/unemupc" media generations --base disk.img --journal disk.upfj
  "$repo_dir/target/debug/unemupc" media verify --base disk.img --journal disk.upfj
  "$repo_dir/target/debug/unemupc" media rollback --base disk.img --journal disk.upfj --to 1 --backup before-rollback.upfj
  "$repo_dir/target/debug/unemupc" media commit --base disk.img --journal disk.upfj --to 1 --output restored.img
  "$repo_dir/target/debug/unemupc" media compact --base disk.img --journal disk.upfj --to 1 --output restored-alias.img
)
```

**MEASURED:** both guest runs print `Halted`. These excerpts were captured from
that generated-media run (all commands exited 0); `verify` prints the same
closed-generation report as `generations`:

```text
base sha256: 36bd753facc985aad613c884a2040210d208b1aa520e957b31ba2e1e19cd4185
retained anchor: 0
generation 0: immutable base, end 128, commands 0, sectors 0
generation 1: session end, end 2048, commands 1, sectors 1
generation 2: session end, end 3968, commands 1, sectors 1
recoverable-tail bytes: 0
latest reconstructible state: generation 2 (closed), end 3968
status: fully valid/closed
closed markers do not establish filesystem health or bootability
```

Rollback prints the warning and changes the report to generation 1:

```text
rollback to generation 1: later generations and any open tip will be removed
generation 1: session end, end 2048, commands 1, sectors 1
latest reconstructible state: generation 1 (closed), end 2048
status: fully valid/closed
```

The two export commands print:

```text
exported generation 1 to restored.img
exported generation 1 to restored-alias.img
```

The base stays all zeroes. Both exports contain byte 17 throughout sector 0 and
zeroes elsewhere; `before-rollback.upfj` retains the complete two-generation
journal. Keep the immutable base with that backup. To continue from the export,
select `restored.img` as the new base and choose a fresh journal path in Rust.
See [Supported host guarantees](design/persistent-media.md#supported-host-guarantees)
for the measured macOS/APFS behavior and storage assumptions.

## Typecheck / lint / format

```sh
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --locked -p unemupc --bin unemupc -- -D warnings
cargo fmt --check
cargo deny --all-features check advisories licenses bans sources
# With a cached advisory database:
cargo deny --all-features check --disable-fetch advisories licenses bans sources
```

`deny.toml` permits the core graph's AGPL-3.0-or-later, MIT, Apache-2.0,
Unicode-3.0 and BSD-3-Clause licences. CC-BY-SA-4.0 is a crate-scoped
exception for `unemu-pc-core`'s text fonts. No advisory is ignored. Git
sources are limited to the pinned CPU and contract repositories.

## CI

GitHub Actions runs `.github/workflows/ci.yml` on pushes to `main`, pull requests
and manual dispatches, on a GitHub-hosted x86-64 `ubuntu-latest` runner. Read the
runs with the GitHub CLI:

```sh
gh run list --repo unemu-org/unemupc --workflow ci.yml
gh run view <run-id> --repo unemu-org/unemupc --log-failed
gh run download <run-id> --repo unemu-org/unemupc   # ci-logs-attempt-<n>: ci-logs/<step>.log
```

From the repository root, run the same gates locally:

```sh
bash scripts/ci-gate.sh devshell
bash scripts/ci-gate.sh fmt
bash scripts/ci-gate.sh guard                    # publication, crate boundaries and core host interfaces (self-tests and checks)
bash scripts/ci-gate.sh clippy
bash scripts/ci-gate.sh deny                     # advisories, licences, bans, sources; all features
bash scripts/ci-gate.sh tests
bash scripts/ci-gate.sh fixtures                 # build/hash-check BIOSes, FreeDOS, test386, XTIDE, HDD, VGA BIOS, open AT BIOS, at-pm-test, at-loadall-test, at-privilege-test, source archives
bash scripts/ci-gate.sh goldens                  # slow tests in the test profile; requires fixtures
```

Fixture-dependent tests print a skip message when a fetched vendor file is missing.
Set `UNEMUPC_REQUIRE_FIXTURES=1` to make a missing fixture fail with its path and
fetch script instead. The `goldens` gate always sets this variable, so a failed or
incomplete fixture fetch cannot silently pass CI. The µPD765 FAT12 test follows
the same policy when `mformat` is missing and points to `nix develop`.

### Publication guard

`bash scripts/publication-guard.sh` (the `guard` gate and a pre-commit hook)
scans tracked and untracked, non-ignored text with plain bash, grep, sed and
awk. It fails on private filesystem paths (home directories, macOS volumes and
temporary directories, Windows user profiles, the root user's home and mounts
under mnt; the script lists the exact forms), e-mail addresses outside
`example.org`, `example.com`, `.example`
hosts and the maintainer's and agent trailer addresses (an SSH login such as
`git@github.com:org/repo` is not an address), RFC 1918 and carrier-grade NAT
IPv4 addresses outside the registry tests' `10.0.0.1` (CIDR ranges are not
addresses), `Cargo.lock` sources other than crates.io and github.com git
repositories, LGPL headers outside the OPL3 port and the project's former name.
Private hostnames never appear in the repository: list them, comma-separated,
in `PUBLICATION_GUARD_PRIVATE_HOSTS`, for example
`PUBLICATION_GUARD_PRIVATE_HOSTS=nas.example,lab.example bash scripts/publication-guard.sh`.
That check runs only locally, where someone sets the variable, never in CI, and
a hit prints only the file and line.

### Crate boundaries

```sh
bash scripts/crate-boundaries.sh --self-test
bash scripts/crate-boundaries.sh
```

Emulation may reach only the contract and emulation crates, including test
and build dependencies. The check also enforces the exact external-crate
list and one resolved contract copy. Includes may not escape the repository.

### Core host interfaces

```sh
bash scripts/core-host-guard.sh              # exact host-interface exceptions in the core layers
bash scripts/core-host-guard.sh --self-test  # planted host uses and allow-list changes
bash scripts/core-host-guard.sh --allow FILE # check a candidate exception list
```

The script needs cargo and jq in the devShell. It reads the workspace's
`cargo metadata --offline --no-deps` layer tags, as the crate-boundary guard
does, and scans `src/**/*.rs` in `contract` and `emulation` packages. It
skips separate `tests/`, `tests.rs` and `*_tests.rs` files and stops each other
file at its first real `#[cfg(test)]` attribute ([core contract](design/core-contract.md)
§12.2–12.3). Line, doc and block comments (including nested and multiline
blocks) are ignored before detecting that cutoff or matching host access.
This is a spelling check: it catches common grouped and renamed imports,
including standard-stream handles, and short paths, but does not resolve
aliases at their later uses or scan past that cutoff.
It checks host filesystem, environment, network, process, threads, clocks,
OS modules and cfgs, streams and printing, mutable statics, thread-locals,
lazy caches, foreign functions, random sources and host-seeded `HashMap`s;
in emulation crates it also rejects `libc::` and `extern crate libc`.
Unsafe spellings are scanned too; `#![forbid(unsafe_code)]` in each emulation
crate makes the compiler enforce that rule throughout the crate, including tests.

`scripts/core-host-guard.allow` lists `FILE: MATCHED SOURCE LINE: REASON`.
Matches are literal full source lines with indentation and trailing whitespace
removed, keyed by repository-relative file. Every match must be listed and
every entry must still match; malformed and duplicate entries fail too. One
entry covers repeated identical lines in the same file. The initial list has
41 entries for 42 matched lines. Review and remove exceptions as host code
leaves the core. The self-test reports its case and planted-host counts. It
plants Instant in `devices`, both libc spellings and unsafe in `device`,
other host families, grouped and renamed imports, and production uses after
comments containing `#[cfg(test)]`. It checks comment filtering, separate
`tests/`, `tests.rs`, `*_tests.rs`, benches and examples, guest identifiers,
missing and stale entries, and a moved use, while excluding host-layer packages.
`--metadata FILE --root DIR` runs the same scanner on a fixture tree and its
`cargo metadata --no-deps` document. `AFFECTED_CARGO` selects a cargo wrapper,
as for the affected-tests runner. Both the bead gate's `guard` step and the
CI guard lane run the self-test and the check; CI already invokes that lane
through `scripts/ci-gate.sh`.

### Third-party licences

`bash scripts/third-party-licenses.sh [OUTPUT [TARGET]]` (in the devShell or with any cargo, jq and SHA-256 tool),
writes `THIRD-PARTY-LICENSES.md` (gitignored) for the release binaries: the
repository's own third-party components (Nuked-OPL3, the fonts, the logo) and
every crate `unemupc` link for the exact `Cargo.lock`, with the
licence files of each published crate (and, for a git dependency, of its
repository root), identical texts written once. A crate that publishes no
licence file is listed with its `authors` as copyright holders, and the
standard texts of its licences come from `LICENSES/` (MIT, Apache-2.0, Zlib).
`OUTPUT` is relative to the calling directory. `TARGET` (a target triple)
limits the crates to one platform's build; without it every platform's crates
are listed. It needs only cargo and jq; set `CARGO_NET_OFFLINE=true` to read
the dependency sources from cargo's cache only. The release build sets
`UNEMUPC_SOURCE_REV` to its tag or commit, so `--about` and `--version` name the
exact source (`<repository>/tree/<rev>`); when unset or empty, they name the
repository and the package version. A non-empty `UNEMUPC_SOURCE_COMMIT` adds
`commit: <commit>` immediately after the source line. Both variables are read
at compile time.

### User-supplied ROM tests

These stay local and ignored. CI never has proprietary firmware. Find ROMs
by hash under `vendor/user-roms/` or `UNEMUPC_USER_ROMS_DIR`.

```sh
bash scripts/ci-gate.sh user-roms
```

## Releases

A release is a tag `v<version>` that matches `version` under
`[workspace.package]` in `Cargo.toml`; bump that first. The same commit raises
`NEXT_RELEASE` in `crates/pc-config/src/compat.rs` past the new version
(`next_release_is_above_the_package_version` fails until it does); rows already
written for the release keep their literal version. The maintainer pushes the
tag and creates and publishes the GitHub release for it. Publishing starts
`.github/workflows/release.yml`, which builds `unemupc` and
attaches them to that release; nothing goes to a package registry.
Before tagging, run every test tier on the whole workspace, including the
user-ROM tests CI never has:
`UNEMUPC_REQUIRE_FIXTURES=1 UNEMUPC_USER_ROMS_DIR=/path/to/roms bash scripts/affected-tests.sh --all --tier all`.

```sh
git tag -s v0.1.0 -m 'unemupc 0.1.0' && git push origin v0.1.0
gh release create v0.1.0 --repo unemu-org/unemupc --verify-tag --title 'unemupc 0.1.0' --notes-file NOTES.md
gh run list --repo unemu-org/unemupc --workflow release.yml
gh workflow run release.yml --repo unemu-org/unemupc -f tag=v0.1.0   # build again, replace the assets
```

The release gets `unemupc-<version>-x86_64-unknown-linux-gnu.tar.gz` (built
on Ubuntu 22.04, so it needs glibc 2.35 or newer),
`unemupc-<version>-aarch64-apple-darwin.tar.gz` (macOS on Apple silicon,
unsigned) and `SHA256SUMS`. Each archive unpacks to
`unemupc-<version>-<target>/` with `unemupc`, `LICENSE`, `README.md`,
`THIRD-PARTY-LICENSES.md` from `scripts/third-party-licenses.sh` for that
target, and `notices/`: the repository's own notices that file quotes and
`LICENSES/`, at their repository paths. The workflow appends download notes
to the release text once, including how to clear the macOS quarantine flag
(`xattr -d com.apple.quarantine unemupc`).

The binaries are built outside Nix, with rustup at the devShell's Rust
(`RUSTUP_TOOLCHAIN` in the workflow; a step checks it against
`rust-version`), `--locked --release`, `--remap-path-prefix` for the
workspace, cargo's registry sources and git checkouts and the toolchain, and
`UNEMUPC_SOURCE_REV` set to the tag and `UNEMUPC_SOURCE_COMMIT` to the full
40-hex commit, so `--about` and `--version` name `<repository>/tree/<tag>`
followed by `commit: <commit>`. A check step fails the job when `strings` finds
the runner's home, the workspace, cargo's home or the toolchain in a binary,
or, on macOS, when `otool -L` lists a library outside `/usr/lib` and
`/System/Library`.

Verify a download in the folder that holds it and `SHA256SUMS`:

```sh
sha256sum --ignore-missing -c SHA256SUMS         # Linux
shasum -a 256 --ignore-missing -c SHA256SUMS     # macOS
```

## x86jit (CPU core)

```sh
# develop against a local x86jit checkout: a gitignored override, never a path dep in Cargo.toml
cat > .cargo/config.toml <<'TOML'
[patch."<the git URL x86jit-core is pinned to in Cargo.toml>"]
x86jit-core = { path = "<your x86jit checkout>/x86jit-core" }
TOML
git -C <your x86jit checkout> rev-parse HEAD    # must match the rev in Cargo.lock for goldens to be valid
```

A change the machine needs from the CPU lands in x86jit under its own beads and gate; this repo
then re-pins and re-runs the slow goldens. See `dev-setup.md` § CPU core.

## Guest test-ROMs

```sh
nasm -f bin boot.asm -o boot.bin             # flat binary for the guest
# (or nasm -f elf + ld + objcopy for larger programs)
```

## Pre-commit

```sh
pre-commit install                                  # one-time, per clone
pre-commit run --all-files                          # run active hooks
pre-commit run --all-files --hook-stage manual      # run staged-as-manual hooks too
```

## beads (`bd`) — open work

```sh
bd memories                                      # the shared working rules — read first
bd ready --exclude-type epic                     # unblocked work, highest priority first
bd list --status in_progress                     # what is claimed
bd show <id>                                     # one bead
bd list --type epic                              # the themes
bd create --title "<t>" --description "<d>" --acceptance "<ac>" --type task --priority P2 \
          --parent <epic-id> --no-inherit-labels --labels <labels>
bd update <id> --claim                           # before writing code
bd update <id> --append-notes "<progress>"       # never --notes on an existing bead
bd close <id> --reason "<commit> — <what landed>"
bd list --external-ref backlog:TASK-41           # the bead an old task id became
```

Details: [`working-on-tasks.md`](working-on-tasks.md).

## Backlog.md — docs and decisions only

```sh
backlog doc create "<title>"                      # → docs/doc-N - Title.md
backlog decision create "<title>"                 # → decisions/decision-N - Title.md
```

Never `backlog task create`: tasks moved to beads.
