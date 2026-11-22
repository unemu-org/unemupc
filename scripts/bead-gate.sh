#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Gate one change with the checks it can affect, each step timed.
#
# scripts/affected-tests.sh plans the change; this gate then runs formatting,
# the publication guard, clippy on the affected crates (and on the production
# unemupc binary when its command composition is affected), the affected tests in their tier,
# rustdoc of the affected crates, and cargo deny when the lock file, a
# manifest or the dependency policy changed. Documentation, script and
# configuration changes build nothing: they run the guard and their own
# checks. Every step runs even after an earlier one fails; the summary gives
# each step's result and time. CI still runs the whole workspace on main.
#
# Usage: bash scripts/bead-gate.sh [--base REF] [--tier TIER] [--all] [--files FILE] [--log-dir DIR]
#   --base, --tier, --all and --files as for scripts/affected-tests.sh
#   --log-dir DIR   keep each step's log in DIR (default: a new temporary directory)
#
# Environment: AFFECTED_CARGO runs cargo, as for scripts/affected-tests.sh.
# The tests require every fixture (UNEMUPC_REQUIRE_FIXTURES=1) unless the
# variable is already set; the user-ROM variables pass through.
set -euo pipefail
export LC_ALL=C
tmp_root=${TMPDIR:-/tmp}
tmp_root=${tmp_root%/}

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  sed -n '/^# Usage:/,/^# Environment:/p' scripts/bead-gate.sh | sed -e '$d' -e 's/^# \{0,1\}//' >&2
  exit "${1:-2}"
}

plan_args=()
log_dir=
files_from_stdin=0
while (( $# )); do
  case "$1" in
    --base|--tier) [[ $# -ge 2 ]] || usage; plan_args+=("$1" "$2"); shift 2 ;;
    --all) plan_args+=(--all); shift ;;
    --files)
      [[ $# -ge 2 ]] || usage
      if [[ $2 == - ]]; then files_from_stdin=1; else plan_args+=(--files "$2"); fi
      shift 2 ;;
    --log-dir) [[ $# -ge 2 ]] || usage; log_dir=$2; shift 2 ;;
    -h|--help) usage 0 ;;
    *) usage ;;
  esac
done
log_dir=${log_dir:-$(mktemp -d "$tmp_root/bead-gate.XXXXXX")}
mkdir -p "$log_dir"
# Every step plans again, so standard input is read once, into the log directory.
if (( files_from_stdin )); then
  cat > "$log_dir/changed-files"
  plan_args+=(--files "$log_dir/changed-files")
fi
export UNEMUPC_REQUIRE_FIXTURES=${UNEMUPC_REQUIRE_FIXTURES-1}
read -r -a cargo_cmd <<< "${AFFECTED_CARGO:-cargo}"

gate_start=$(date +%s)
summary=()
failed=()

# step NAME COMMAND...: run, keep the output in NAME.log, record the time.
step() {
  local name=$1 start status
  shift
  echo "== $name $(date +%H:%M:%S): $*"
  start=$(date +%s)
  set +e
  "$@" > "$log_dir/$name.log" 2>&1
  status=$?
  set -e
  if (( status == 0 )); then
    summary+=("$(printf 'PASS %-14s %5ds' "$name" $(( $(date +%s) - start )))")
  else
    summary+=("$(printf 'FAIL %-14s %5ds  exit %d, see %s' "$name" $(( $(date +%s) - start )) "$status" "$log_dir/$name.log")")
    failed+=("$name")
    tail -n 20 "$log_dir/$name.log" | sed 's/^/   | /'
  fi
}

skip() { summary+=("$(printf 'skip %-14s        %s' "$1" "$2")"); }

step plan bash scripts/affected-tests.sh --plan ${plan_args[@]+"${plan_args[@]}"}
cat "$log_dir/plan.log"
if (( ${#failed[@]} )); then
  echo "bead gate: the plan failed"
  exit 1
fi
vars=$(bash scripts/affected-tests.sh --plan --format vars ${plan_args[@]+"${plan_args[@]}"})
var() { printf '%s\n' "$vars" | sed -n "s/^$1=//p"; }
mode=$(var mode)

packages=()
if [[ $mode == full ]]; then
  packages=(--workspace)
else
  for crate in $(var affected); do packages+=(-p "$crate"); done
fi

if [[ $mode == crates || $mode == full ]]; then
  step fmt "${cargo_cmd[@]}" fmt --check
else
  skip fmt "no crate affected"
fi

guard_lane() {
  local status=0
  bash scripts/uid-guard.sh --self-test || status=1
  bash scripts/publication-guard-test.sh && bash scripts/publication-guard.sh || status=1
  bash scripts/core-host-guard.sh --self-test && bash scripts/core-host-guard.sh || status=1
  return "$status"
}
step guard guard_lane

if [[ $mode == crates || $mode == full ]]; then
  step clippy "${cargo_cmd[@]}" clippy --locked "${packages[@]}" --all-targets --all-features -- -D warnings
  if [[ $(var cli) == 1 ]]; then
    # The production binary without the dev-dependency features tests unify in.
    step clippy-unemupc "${cargo_cmd[@]}" clippy --locked -p unemupc --bin unemupc -- -D warnings
  fi
else
  skip clippy "no crate affected"
fi

if [[ $mode != none ]]; then
  step tests env AFFECTED_LOG_DIR="$log_dir/tests" \
    bash scripts/affected-tests.sh --no-guard ${plan_args[@]+"${plan_args[@]}"}
  # The runner used, the slowest tests and every step with its failed tests.
  sed -n -e '/^runner:/p' -e '/^slowest test/,$p' "$log_dir/tests.log" | sed 's/^/   /'
else
  skip tests "nothing changed"
fi

if [[ $mode == crates || $mode == full ]]; then
  step rustdoc env RUSTDOCFLAGS='-D warnings' "${cargo_cmd[@]}" doc --locked --no-deps "${packages[@]}"
else
  skip rustdoc "no crate affected"
fi

if [[ ${BEAD_GATE_SKIP_DENY:-0} == 1 ]]; then
  skip deny "reserved for supervisor (BEAD_GATE_SKIP_DENY=1)"
elif [[ $(var deny) == 1 ]]; then
  step deny "${cargo_cmd[@]}" deny --all-features check advisories licenses bans sources
else
  skip deny "lock file, manifests and dependency policy unchanged"
fi

echo
echo "bead gate: $(( $(date +%s) - gate_start ))s in all, logs in $log_dir"
printf '  %s\n' "${summary[@]}"
if (( ${#failed[@]} )); then
  echo "bead gate: failed: ${failed[*]}"
  exit 1
fi
echo "bead gate: passed"
