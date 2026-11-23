#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Run one CI gate from the repository root, keeping its status and log.
set -euo pipefail

usage() {
    echo 'Usage: bash scripts/ci-gate.sh {devshell|fmt|guard|clippy|deny|tests|fixtures|goldens|user-roms}' >&2
    exit 2
}

[[ $# -eq 1 ]] || usage
step=$1
gate_command=(nix develop --no-update-lock-file --command)
export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"
case "$step" in
    devshell) gate_command+=(rustc --version) ;;
    fmt) gate_command+=(cargo fmt --check) ;;
    # The publication guard needs no devShell; the crate-boundary check reads
    # `cargo metadata` with the devShell's cargo and jq. Both run even when the
    # other fails, so one log names every finding; either failure fails the gate.
    # The check resolves the graph offline, and a restored cache can lack a
    # git dependency the lock file has moved to (or hold nothing at all), so
    # the lane downloads the locked sources first. Fetching builds nothing.
    guard)
        gate_command=(bash -uc '
            status=0
            bash scripts/uid-guard.sh --self-test || status=1
            bash scripts/publication-guard-test.sh && bash scripts/publication-guard.sh || status=1
            nix develop --no-update-lock-file --command bash -euc "
                cargo fetch --locked
                bash scripts/crate-boundaries.sh --self-test
                bash scripts/crate-boundaries.sh
                bash scripts/core-host-guard.sh --self-test
                bash scripts/core-host-guard.sh
            " || status=1
            exit "$status"
        ')
        ;;
    clippy)
        gate_command+=(bash -euc '
            cargo clippy --locked --all-targets --all-features -- -D warnings
            cargo clippy --locked -p unemupc --bin unemupc -- -D warnings
        ')
        ;;
    deny) gate_command+=(cargo deny --all-features check advisories licenses bans sources) ;;
    tests)
        # Release builds keep a few behaviours that debug builds assert against
        # (devices ignore an earlier clock); run the device tests in both.
        gate_command+=(bash -euc '
            cargo test --locked --profile test --workspace
            cargo test --locked --release -p devices
        ')
        ;;
    fixtures)
        # The goldens gate requires every fixture its tests read. vgabios and
        # unemu-at-bios are built by Nix from their public GitHub repositories
        # over anonymous HTTPS (UNEMU_VGABIOS_FLAKE / UNEMU_VGABIOS_CHECKOUT and
        # UNEMU_AT_BIOS_FLAKE / UNEMU_AT_BIOS_CHECKOUT override the source).
        # SeaBIOS is built in the separate firmware devShell (cross GCC from
        # the Nix binary cache). The Kiselev, XTIDE, vgabios and unemu-at-bios
        # scripts also write the pinned source archives of their open
        # images. XTIDE runs after them: it exports its
        # source over the network with Subversion, which Nix supplies, so an
        # unreachable server cannot keep the other fixtures from being built.
        # The lane stops at the first failing script; the last header names it.
        gate_command+=(bash -euc '
            echo "== fixtures: vendor/kiselev-bios/fetch.sh"
            bash vendor/kiselev-bios/fetch.sh
            echo "== fixtures: vendor/freedos/fetch.sh"
            bash vendor/freedos/fetch.sh
            echo "== fixtures: vendor/test386/fetch.sh"
            bash vendor/test386/fetch.sh
            echo "== fixtures: vendor/hdd/build.sh"
            bash vendor/hdd/build.sh
            echo "== fixtures: vendor/vgabios/fetch.sh"
            bash vendor/vgabios/fetch.sh
            echo "== fixtures: vendor/unemu-at-bios/fetch.sh"
            bash vendor/unemu-at-bios/fetch.sh
            echo "== fixtures: vendor/at-pm-test/build.sh"
            bash vendor/at-pm-test/build.sh
            echo "== fixtures: vendor/i386-pm-test/build.sh"
            bash vendor/i386-pm-test/build.sh
            echo "== fixtures: vendor/at-loadall-test/build.sh"
            bash vendor/at-loadall-test/build.sh
            echo "== fixtures: vendor/vga-modex-test/build.sh"
            bash vendor/vga-modex-test/build.sh
            echo "== fixtures: vendor/at-privilege-test/build.sh"
            bash vendor/at-privilege-test/build.sh
            echo "== fixtures: vendor/seabios/fetch.sh"
            nix develop .#firmware --command bash vendor/seabios/fetch.sh
            echo "== fixtures: vendor/xtide-universal-bios/fetch.sh"
            bash vendor/xtide-universal-bios/fetch.sh
        ')
        ;;
    # The tests lane runs before the fixtures exist, so tests that read a
    # fixture skip there. This lane runs them again with every fixture
    # present, next to the slow ignored goldens.
    goldens) gate_command+=(env UNEMUPC_REQUIRE_FIXTURES=1 cargo test --locked --profile test --workspace -- --include-ignored) ;;
    # Local opt-in lane: CI never calls it and never has user-supplied firmware.
    user-roms) gate_command+=(cargo test --locked --profile test --workspace --no-fail-fast -- --ignored user_rom_) ;;
    *) usage ;;
esac

log_dir=${CI_LOG_DIR:-ci-logs}
mkdir -p "$log_dir"
# pipefail keeps a failed gate red even when tee successfully writes its log.
"${gate_command[@]}" 2>&1 | tee "$log_dir/$step.log"
