#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Run only the tests a change can affect.
#
# The changed files are the difference between the merge base of a base ref
# (default main) and the working tree, plus untracked files. Each file maps to
# the workspace crate whose directory holds it, or, for files outside the
# crates, to the crates that read it (the rules below). Every crate that
# depends on a changed crate, through a normal, build or dev dependency, is
# affected too: that reverse-dependency closure comes from
# `cargo metadata --no-deps`. A change to the workspace manifest, the lock file
# or the toolchain, a path no rule knows, or a `[patch]` in .cargo/config.toml
# (dependencies that are not the pinned ones) selects the whole workspace.
# Documentation, script and repository-configuration changes select no crate;
# they run the checks that can catch their mistakes instead.
#
# Tiers. The fast tier is the ordinary test run of the affected crates. The
# golden tier adds the ignored tests (BIOS POST, DOS boots and sessions,
# fixture archives) of every affected crate; it is chosen when a change can
# alter emulated behaviour: an emulation crate, the machine runners the
# goldens drive (pc-harness), a shipped machine file or a fixture pin changed.
# Without it, a change confined to integration test files selects only their
# ignored tests. Other changes select the ignored tests of each changed crate,
# and those in files of the affected crates that name a changed crate
# (`assembly::`, say). Tests that need user-supplied firmware start with
# `user_rom_` and run only in the user-roms and all tiers. Before a release,
# `--all --tier all` runs the whole workspace in every tier.
#
# The tests run under cargo-nextest when it is on PATH (each test in its own
# process, test binaries side by side, doctests through `cargo test --doc`),
# otherwise under `cargo test`. Either way at most AFFECTED_TEST_THREADS
# tests run at once (default RUST_TEST_THREADS, else 4).
#
# Usage: bash scripts/affected-tests.sh [options]
#   --base REF        compare with the merge base of REF and HEAD (default main)
#   --tier TIER       auto (default), fast, goldens, user-roms (auto plus the
#                     user-ROM tests) or all (goldens plus user-ROM tests)
#   --all             select the whole workspace whatever changed
#   --plan            print the plan and exit; nothing is built
#   --format FORMAT   plan output: text (default) or vars (key=value lines)
#   --no-guard        skip the publication guard
#                     (scripts/bead-gate.sh runs it as its own step)
#   --files FILE      read the changed paths from FILE (- for stdin) instead of git
#   --metadata FILE   read `cargo metadata --no-deps --format-version 1` from FILE
#   --self-test       check the planner against scripts/testdata/affected-tests/
#                     and the crate-reading rules against the sources
#
# Environment: AFFECTED_CARGO is the command that runs cargo (default
# `cargo`; a wrapper such as `nice cargo` fits), AFFECTED_RUNNER picks
# nextest or cargo (default: nextest when installed), AFFECTED_TEST_THREADS
# bounds the tests running at once, AFFECTED_LOG_DIR keeps the logs
# (default: a new temporary directory). Fixture and user-ROM variables
# (UNEMUPC_REQUIRE_FIXTURES, UNEMUPC_USER_ROMS_DIR, ...) pass through to the
# tests. Needs bash, git, jq and, unless --files and --metadata are both
# given, cargo.
set -euo pipefail
export LC_ALL=C
tmp_root=${TMPDIR:-/tmp}
tmp_root=${tmp_root%/}

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  sed -n '/^# Usage:/,/^# Environment:/p' scripts/affected-tests.sh | sed -e '$d' -e 's/^# \{0,1\}//' >&2
  exit "${1:-2}"
}

# The planner: metadata JSON on stdin; as arguments $changed (repo-relative
# paths), $ignored ({path: {tests: [ignored test names], mentions: [crate
# identifiers the file names as ident::]}} for every crate source that holds
# ignored tests), $tier, $all and $patch (why the dependencies are not the
# pinned ones, or ""); prints the plan as JSON.
# shellcheck disable=SC2016
planner='
# Crates whose change can alter emulated behaviour; a change to one of them
# selects the golden tier.
def emulation_crates: ["assembly", "audio", "bus", "chipset", "cpu", "device",
  "devices", "journal", "journal-files", "machine", "machine-file", "pc-config", "testkit", "unemu-pc-core",
  "video"];
# Machine runners select the slow golden tier when their behaviour changes.
def runner_files: ["crates/pc-harness/src/", "crates/unemupc/src/"];
# Paths that crates read outside their own directory: test support shared
# with #[path], shipped machine files, fixture scripts and pins, assets.
# The self-test checks that every path a crate source names outside its
# directory is covered here, and that every crate that builds paths from the
# repository root reads both machines/ and vendor/.
def data_rules: [
  {prefix: "crates/pc-harness/tests/support/", goldens: true,
   crates: ["assembly", "machine-file", "pc-config", "pc-harness", "unemupc"],
   why: "golden test support shared by several crates"},
  {prefix: "machines/README.md", goldens: false,
   crates: ["machine-file"], why: "machine-file guide that tests quote"},
  {prefix: "machines/", goldens: true,
   crates: ["machine-file", "pc-harness", "unemupc"], why: "shipped machine file"},
  {prefix: "vendor/", except: "[.]md$", goldens: true,
   crates: ["assembly", "machine-file", "pc-harness", "unemupc"],
   why: "fixture script, pin or notice"}
];
def scan_tests: [
  {package: "pc-config", test: "platform_ids",
   watch: "^crates/(device|bus|cpu|video|audio|devices|chipset|machine|assembly|testkit|journal|pc-config|pc-core|machine-file|journal-files|pc-harness|unemupc)/.+[.]rs$",
   why: "scans the core Rust sources"}
];
def full_files: "^(Cargo[.]toml|Cargo[.]lock|flake[.]nix|flake[.]lock|rust-toolchain([.]toml)?|[.]?rustfmt[.]toml|[.]?clippy[.]toml|[.]cargo/.*)$";
def docs_files: "^(backlog/.*|[^/]+[.]md|[.]agents/.*|[.]claude/.*|[.]serena/.*|LICENSE.*|LICENSES/.*|[.]markdownlint.*|[.]prettier.*|vendor/.+[.]md)$";
def config_files: "^([.]github/.*|[.]beads/.*|[.]pre-commit-config[.]yaml|[.]gitignore|[.]envrc)$";
def runner_self: "^scripts/(affected-tests[.]sh|bead-gate[.]sh|testdata/affected-tests/.*)$";

def has($x): any(.[]; . == $x);
def ident: gsub("-"; "_");
def owner($members):
  . as $f
  | [$members[] | select(.dir as $dir | $f | startswith($dir + "/"))]
  | sort_by(.dir | length) | last;
def closure($rdeps):
  def grow: (. + [.[] | $rdeps[.][]?]) | unique;
  def fix: grow as $next | if $next == . then . else ($next | fix) end;
  unique | fix;

def classify($members):
  . as $f
  | ($f | owner($members)) as $owner
  | ([data_rules[]
      | select(.prefix as $prefix | .except as $except
               | ($f | startswith($prefix)) and ($except == null or ($f | test($except) | not)))]
     | first) as $data
  | if ($f | test(full_files)) then
      {class: "full", crates: [], goldens: true, deny: true,
       why: "workspace manifest, lock file or toolchain: everything"}
    elif $f == "deny.toml" then
      {class: "deny", crates: [], goldens: false, deny: true, why: "dependency policy"}
    elif $data != null then
      {class: "crate", crates: ($data.crates + [$owner.name // empty] | unique),
       goldens: $data.goldens, deny: false, why: $data.why}
    elif $owner != null then
      (emulation_crates | has($owner.name)) as $emulation
      | (any(runner_files[]; . as $r | $f == $r or ($r | endswith("/")) and ($f | startswith($r)))) as $runner
      | {class: "crate", crates: [$owner.name], goldens: ($emulation or $runner),
         deny: ($f == $owner.dir + "/Cargo.toml"),
         why: (if $emulation then "emulation crate"
               elif $runner then "machine runner the goldens drive"
               else "crate" end)}
    elif ($f | test(docs_files)) then
      {class: "docs", crates: [], goldens: false, deny: false, why: "documentation"}
    elif ($f | startswith("scripts/")) then
      {class: "scripts", crates: [], goldens: false, deny: false, why: "script"}
    elif ($f | test(config_files)) then
      {class: "config", crates: [], goldens: false, deny: false,
       why: "repository, CI or tracker configuration"}
    else
      {class: "full", crates: [], goldens: true, deny: false,
       why: "no rule maps this path: everything"}
    end
  | {path: $f} + .;

if $tier == "rules" then
  {emulation_crates: emulation_crates, runner_files: runner_files,
   data_rules: data_rules, scan_tests: scan_tests}
else
.workspace_root as $root
| .workspace_members as $ids
| [.packages[] | select(.id as $id | $ids | has($id))
   | {name, dir: (.manifest_path | ltrimstr($root + "/") | rtrimstr("/Cargo.toml")),
      deps: [.dependencies[] | select(.path != null) | .path | ltrimstr($root + "/")]}]
  as $packages
| ($packages | map({(.dir): .name}) | add) as $by_dir
| [$packages[] | .deps = [.deps[] | $by_dir[.] // empty]] as $members
| (reduce $members[] as $m ({}; reduce $m.deps[] as $d (.; .[$d] += [$m.name]))) as $rdeps
| ([$changed[] | classify($members)]
   + (if $patch == "" then [] else
       [{path: ".cargo/config.toml", class: "full", crates: [], goldens: true, deny: false,
         why: $patch}] end)) as $files
| ($all or ($files | any(.class == "full"))) as $full
| ([$files[] | select(.class == "crate") | .crates[]] | unique) as $seeds
| (if $full then [$members[].name] | sort else $seeds | closure($rdeps) end) as $affected
| ($full or ($files | any(.class == "crate" and .goldens))) as $golden_auto
| (if $tier == "fast" then false elif $tier == "goldens" or $tier == "all" then true
   else $golden_auto end) as $goldens_on
| (if $full then "full" elif ($seeds | length) > 0 then "crates"
   elif ($files | length) > 0 then "checks" else "none" end) as $mode
# Integration test files do not change the crate implementation or sibling
# tests. Support files and other changes retain the broader crate selection.
| (if $goldens_on or $tier == "fast" or $mode != "crates" then [] else
    [$ignored | to_entries[] | .key as $path | .value as $v
     | ($path | owner($members) | .name) as $crate
     | select($crate != null and ($affected | has($crate)))
     | [$seeds[] | select(ident as $id | ($v.mentions // []) | has($id))] as $named
     | if ($seeds | has($crate)) then
         select(any($files[]; (.crates | has($crate)) and
           (.path == $path or (.path | test("^crates/[^/]+/tests/[^/]+[.]rs$") | not))))
         | {path: $path, crate: $crate, tests: $v.tests, why: "in changed crate \($crate)"}
       elif ($named | length) > 0 then
         {path: $path, crate: $crate, tests: $v.tests,
          why: "names \($named | map(ident + "::") | join(" "))"}
       else empty end]
  end) as $targeted
| {
    mode: $mode,
    seeds: $seeds,
    affected: (if $mode == "crates" or $mode == "full" then $affected else [] end),
    tier: (if $goldens_on then "goldens" else "fast" end),
    golden_reasons: [$files[] | select(.goldens) | "\(.path): \(.why)"],
    user_roms: ($tier == "user-roms" or $tier == "all"),
    targeted: ([$targeted[].tests[]] | unique),
    targeted_files: [$targeted[] | "\(.path): \(.why)"],
    targeted_crates: ([$targeted[].crate] | unique),
    scan_tests: (if $mode != "crates" then [] else
      [scan_tests[] | select(.package as $p | ($affected | has($p)) | not)
       | select(.watch as $w | $changed | any(test($w)))
       | "\(.package):\(.test)"] end),
    release_devices: ($affected | has("devices") and ($mode == "crates" or $mode == "full")),
    cli: ($affected | has("unemupc") and ($mode == "crates" or $mode == "full")),
    deny: ($files | any(.deny)),
    docs: [$files[] | select(.class == "docs") | .path],
    shell: [$changed[] | select(test("[.]sh$"))],
    self_test: ($changed | any(test(runner_self))),
    files: $files
  }
end
'

base=main
tier=auto
all=false
plan_only=0
format=text
guard=1
files_from=
metadata_from=
self_test=0
while (( $# )); do
  case "$1" in
    --base) [[ $# -ge 2 ]] || usage; base=$2; shift 2 ;;
    --tier) [[ $# -ge 2 ]] || usage; tier=$2; shift 2 ;;
    --all) all=true; shift ;;
    --plan) plan_only=1; shift ;;
    --format) [[ $# -ge 2 ]] || usage; format=$2; shift 2 ;;
    --no-guard) guard=0; shift ;;
    --files) [[ $# -ge 2 ]] || usage; files_from=$2; shift 2 ;;
    --metadata) [[ $# -ge 2 ]] || usage; metadata_from=$2; shift 2 ;;
    --self-test) self_test=1; shift ;;
    -h|--help) usage 0 ;;
    *) usage ;;
  esac
done
case "$tier" in auto|fast|goldens|user-roms|all) ;; *) usage ;; esac
case "$format" in text|vars) ;; *) usage ;; esac

read -r -a cargo_cmd <<< "${AFFECTED_CARGO:-cargo}"

# The names of the ignored tests in one Rust file, except user-ROM tests:
# the first function after each #[ignore line.
ignored_tests() {
  awk '/#\[ignore/ { want = 1; next }
       want && match($0, /fn [A-Za-z0-9_]+/) {
         name = substr($0, RSTART + 3, RLENGTH - 3); want = 0
         if (name !~ /^user_rom_/) print name
       }' "$1"
}

# The planner's $ignored for the sources under crates/: every file that holds
# ignored tests other than user-ROM ones, with the workspace crates it names.
# ignored_json METADATA_FILE
ignored_json() {
  local idents file tests mentions
  idents=$(jq -r '[.packages[].name | gsub("-"; "_")] | join("|")' "$1")
  { grep -rlE '#\[ignore' crates --include='*.rs' || true; } | sort | while IFS= read -r file; do
    tests=$(ignored_tests "$file" | tr '\n' ' ')
    [[ -n $tests ]] || continue
    mentions=$({ grep -oE "(^|[^A-Za-z0-9_])($idents)::" "$file" || true; } \
      | sed -E 's/^[^a-z]//; s/::$//' | sort -u | tr '\n' ' ')
    printf '%s\t%s\t%s\n' "$file" "$tests" "$mentions"
  done | jq -Rn '[inputs | split("\t")
    | {key: .[0], value: {tests: (.[1] | split(" ") | map(select(. != ""))),
                          mentions: (.[2] | split(" ") | map(select(. != "")))}}]
    | from_entries'
}

# Why the dependencies are not the pinned ones (a local [patch] override,
# backlog/docs/dev-setup.md), or nothing.
patch_reason() {
  local config
  for config in .cargo/config.toml .cargo/config; do
    if [[ -f $config ]] && grep -qE '^[[:space:]]*\[patch' "$config"; then
      echo "$config has a [patch] section: a dependency is not the pinned one, so everything"
      return
    fi
  done
}

workspace_metadata() {
  if [[ -n $metadata_from ]]; then
    cat "$metadata_from"
  else
    # Reads the manifests only: no dependency resolution, nothing is built.
    "${cargo_cmd[@]}" metadata --no-deps --format-version 1 --offline
  fi
}

changed_files() {
  if [[ -n $files_from ]]; then
    if [[ $files_from == - ]]; then cat; else cat "$files_from"; fi | sed '/^[[:space:]]*$/d'
    return
  fi
  local merge_base
  merge_base=$(git merge-base "$base" HEAD)
  # Unquoted paths, so a name with non-ASCII characters still matches its rule.
  { git -c core.quotePath=false diff --name-only --no-renames "$merge_base"
    git -c core.quotePath=false ls-files --others --exclude-standard; } | sort -u
}

# plan METADATA_FILE CHANGED_FILE IGNORED_JSON TIER ALL PATCH
plan() {
  jq --argjson changed "$(jq -Rn '[inputs]' < "$2")" --argjson ignored "$3" \
    --arg tier "$4" --argjson all "$5" --arg patch "$6" "$planner" < "$1"
}

print_plan_text() {
  printf '%s' "$1" | jq -r '
    def list: if length == 0 then "none" else join(" ") end;
    "changed files: \(.files | length)",
    (.files[] | "  \(.path) -> \(if .class == "crate" then (.crates | join(" ")) else .class end) (\(.why))"),
    "mode: \(.mode)",
    (if .mode == "crates" or .mode == "full" then
      "changed crates: \(.seeds | list)",
      "affected crates (changed crates and their dependents): \(.affected | list)",
      "tier: \(.tier)\(if .user_roms then " plus user-ROM tests" else "" end)"
        + (if .tier == "goldens" and (.golden_reasons | length) > 0
           then ", because:" else "" end),
      (if .tier == "goldens" then .golden_reasons[] | "  \(.)" else empty end),
      "ignored tests run by name: \(.targeted | length)"
        + (if (.targeted | length) > 0 then ", from:" else "" end),
      (.targeted_files[] | "  \(.)"),
      "workspace scan tests: \(.scan_tests | list)",
      "release-profile devices tests: \(if .release_devices then "yes" else "no" end)"
     else empty end),
    "dependency policy (cargo deny): \(if .deny then "yes" else "no" end)",
    (if .mode != "none" then "checks: publication guard"
      + (if (.docs | length) > 0 then ", whitespace of the changed documentation" else "" end)
      + (if (.shell | length) > 0 then ", bash -n of the changed shell scripts" else "" end)
      + (if .self_test then ", the runner self-test" else "" end)
     else empty end)
  '
}

print_plan_vars() {
  printf '%s' "$1" | jq -r '
    "mode=\(.mode)", "seeds=\(.seeds | join(" "))", "affected=\(.affected | join(" "))",
    "tier=\(.tier)", "user_roms=\(if .user_roms then 1 else 0 end)",
    "targeted=\(.targeted | join(" "))", "scan_tests=\(.scan_tests | join(" "))",
    "release_devices=\(if .release_devices then 1 else 0 end)",
    "cli=\(if .cli then 1 else 0 end)", "deny=\(if .deny then 1 else 0 end)",
    "docs=\(.docs | join(" "))", "shell=\(.shell | join(" "))",
    "self_test=\(if .self_test then 1 else 0 end)"
  '
}

field() { printf '%s' "$plan_json" | jq -r "$1"; }

# The failed tests in one step's log, in the format of the runner that wrote it.
failed_tests() {
  if [[ $1 == nextest ]]; then
    sed -nE 's/^ *(FAIL|TIMEOUT|ABORT|LEAK-FAIL|SIG[A-Z]+) +\[[^]]*\]( +\( *[0-9]+\/ *[0-9]+\))? +(.*)$/\3/p' "$2"
  else
    sed -nE 's/^test (.*) \.\.\. FAILED$/\1/p' "$2"
  fi | sort -u
}

# ---------------------------------------------------------------- self-test

# Lexically normalise a relative path ("a/b/../c" -> "a/c"); a path that
# climbs above the repository keeps its leading "..".
normalise() {
  awk -v path="$1" 'BEGIN {
    n = split(path, part, "/"); depth = 0
    for (i = 1; i <= n; i++) {
      if (part[i] == "" || part[i] == ".") continue
      if (part[i] == ".." && depth > 0 && out[depth] != "..") { depth--; continue }
      out[++depth] = part[i]
    }
    s = ""; for (i = 1; i <= depth; i++) s = s (i > 1 ? "/" : "") out[i]
    print s
  }'
}

# The crate of a repository path, from "dir<TAB>name" lines.
crate_of() {
  local file=$1 dirs=$2 best= crate= dir name
  while IFS=$'\t' read -r dir name; do
    if [[ $file == "$dir"/* && ${#dir} -gt ${#best} ]]; then best=$dir; crate=$name; fi
  done < <(printf '%s\n' "$dirs")
  [[ -n $best ]] && printf '%s\t%s\n' "$crate" "$best"
}

# "crate<TAB>path" for every existing repository path that a crate source
# names outside the crate's own directory: #[path] modules and include_*!
# files (relative to the naming file) and "../.." string literals (relative
# to the crate directory, which is how the tests join them to
# CARGO_MANIFEST_DIR).
outside_references() {
  local dirs=$1 line file match literal base owner crate crate_dir resolved
  while IFS= read -r line; do
    file=${line%%:*}
    match=${line#*:}
    match=${match#*:}
    literal=${match#*\"}
    literal=${literal%\"*}
    owner=$(crate_of "$file" "$dirs") || continue
    crate=${owner%%$'\t'*}
    crate_dir=${owner#*$'\t'}
    case "$match" in
      '#['*|include*) base=$(dirname "$file") ;;
      *) base=$crate_dir ;;
    esac
    resolved=$(normalise "$base/$literal")
    case "$resolved" in ..*|"$crate_dir"|"$crate_dir"/*) continue ;; esac
    [[ -e $resolved ]] || continue
    [[ -d $resolved ]] && resolved=$resolved/
    printf '%s\t%s\n' "$crate" "$resolved"
  done < <(grep -rnoE '(#\[path *= *|include_str!\(|include_bytes!\(|include!\()?"\.\./[^"]*"' crates --include='*.rs' || true)
}

# The crates whose sources build paths from the repository root at run time:
# join("../..") or a "/../.." literal (the root itself, not one file under
# it), or parent() / ancestors() within a few lines of CARGO_MANIFEST_DIR. Such a crate may read anything under the root,
# so the rules must count it among the readers of machines/ and vendor/.
root_builders() {
  local dirs=$1 dir name
  while IFS=$'\t' read -r dir name; do
    # No grep -q: under pipefail its early exit would fail the pipeline.
    if [[ -n $(find "$dir" -name '*.rs' -exec awk '
        /CARGO_MANIFEST_DIR/ { near = 4 }
        /join\("\.\.\/\.\.\/?"\)/ || /"\/\.\.\/\.\.\/?"/ { found = 1 }
        near > 0 && (/\.parent\(\)/ || /\.ancestors\(\)/) { found = 1 }
        { if (near > 0) near-- }
        END { exit !found }' {} \; -print) ]]; then
      echo "$name"
    fi
  done < <(printf '%s\n' "$dirs")
}

run_self_test() {
  local data=scripts/testdata/affected-tests failures=0 count i name expect got mismatch tmp
  tmp=$(mktemp -d "$tmp_root/affected-self-test.XXXXXX")
  count=$(jq length "$data/scenarios.json")
  for (( i = 0; i < count; i++ )); do
    name=$(jq -r ".[$i].name" "$data/scenarios.json")
    jq -r ".[$i].changed[]" "$data/scenarios.json" > "$tmp/changed"
    # A scenario uses the snapshot of the sources' ignored tests unless it
    # brings its own.
    got=$(plan "$data/metadata.json" "$tmp/changed" \
      "$(jq -c --slurpfile snapshot "$data/ignored.json" ".[$i].ignored // \$snapshot[0]" "$data/scenarios.json")" \
      "$(jq -r ".[$i].tier // \"auto\"" "$data/scenarios.json")" false \
      "$(jq -r ".[$i].patch // \"\"" "$data/scenarios.json")")
    expect=$(jq -c ".[$i].expect" "$data/scenarios.json")
    # Compare only the fields the scenario names.
    if mismatch=$(jq -rn --argjson got "$got" --argjson expect "$expect" '
        [$expect | to_entries[] | select($got[.key] != .value)
         | "    \(.key): expected \(.value | tojson), got \($got[.key] | tojson)"] | .[]') \
        && [[ -z $mismatch ]]; then
      echo "ok   $name"
    else
      echo "FAIL $name"
      printf '%s\n' "$mismatch"
      failures=$((failures + 1))
    fi
  done

  # Exercise both the pipe-sized deadlock window on macOS and larger inputs.
  local size padding plan_json dirs
  printf '%s\n' 'env!("CARGO_MANIFEST_DIR").join("../..");' > "$tmp/root-builder.rs"
  for size in 32768 131072; do
    padding=$(jq -rn --argjson size "$size" '"x" * $size')
    plan_json=$(jq -n --argjson size "$size" '{mode: "checks", seeds: [], affected: [],
      tier: "fast", user_roms: false, targeted: [], scan_tests: [],
      release_devices: false, cli: false, deny: false, docs: [], shell: [],
      self_test: false, files: [{path: ("x" * $size), class: "scripts", why: "script"}]}')
    dirs=$(printf '%s\t%s\n' "$tmp" "$padding")
    if [[ $(print_plan_text "$plan_json") == *"$padding -> scripts (script)"* &&
          $(print_plan_vars "$plan_json") == *'mode=checks'* &&
          $(field '.files[0].path') == "$padding" &&
          $(crate_of "$tmp/root-builder.rs" "$dirs") == "$padding"$'\t'"$tmp" &&
          $(root_builders "$dirs") == "$padding" ]]; then
      echo "ok   plan and directory helpers stream $size-byte payloads"
    else
      echo "FAIL plan and directory helpers with $size-byte payloads"
      failures=$((failures + 1))
    fi
  done

  got=$(ignored_tests "$data/ignored.rs" | tr '\n' ' ')
  expect='slow_golden_is_listed attribute_after_ignore_is_skipped_over nested_golden_is_listed '
  if [[ $got == "$expect" ]]; then
    echo "ok   ignored tests of a changed file, without the user-ROM ones"
  else
    echo "FAIL ignored tests of a changed file"
    echo "    expected: $expect"
    echo "    got:      $got"
    failures=$((failures + 1))
  fi

  # Both runners' failures are reported by test name.
  printf '%s\n' '        PASS [   0.010s] (1/3) unemupc::quantum passes' \
    '        FAIL [   1.250s] (2/3) unemupc::quantum real16_spin_reaches_cap' \
    '     SIGSEGV [   0.300s] (3/3) unemupc::bin/unemupc shell::tests::crashes' \
    '     Summary [   1.300s] 3 tests run: 1 passed, 2 failed' \
    '        FAIL [   1.250s] (2/3) unemupc::quantum real16_spin_reaches_cap' > "$tmp/nextest.log"
  printf '%s\n' 'test passes ... ok' 'test shell::tests::breaks ... FAILED' \
    'test result: FAILED. 1 passed; 1 failed' > "$tmp/cargo.log"
  got=$( { failed_tests nextest "$tmp/nextest.log"; failed_tests cargo "$tmp/cargo.log"; } | tr '\n' '|')
  expect='unemupc::bin/unemupc shell::tests::crashes|unemupc::quantum real16_spin_reaches_cap|shell::tests::breaks|'
  if [[ $got == "$expect" ]]; then
    echo "ok   failed tests are reported by name for both runners"
  else
    echo "FAIL failed tests by name"
    echo "    expected: $expect"
    echo "    got:      $got"
    failures=$((failures + 1))
  fi

  # Every path a crate source names outside its directory must select that
  # crate (or everything) when it changes.
  if command -v cargo > /dev/null; then
    local metadata dirs refs rules missing file builders
    metadata="$tmp/metadata.json"
    "${cargo_cmd[@]}" metadata --no-deps --format-version 1 --offline > "$metadata"
    dirs=$(jq -r '.workspace_root as $r | .packages[]
      | "\(.manifest_path | ltrimstr($r + "/") | rtrimstr("/Cargo.toml"))\t\(.name)"' "$metadata")
    refs=$(outside_references "$dirs" | sort -u)
    printf '%s\n' "$refs" | cut -f2 | sort -u > "$tmp/paths"
    got=$(plan "$metadata" "$tmp/paths" '{}' auto false "")
    missing=$(jq -rn --argjson plan "$got" --arg refs "$refs" '
      ($plan.files | map({(.path): .}) | add) as $by_path
      | $refs | split("\n")[] | select(. != "") | split("\t") as [$crate, $path]
      | $by_path[$path] as $f
      | select($f.class != "full" and ($f.crates | index($crate)) == null)
      | "    \($crate) reads \($path), which maps to \($f.class) \($f.crates | join(" "))"')
    if [[ -z $missing ]]; then
      echo "ok   every path a crate names outside its directory selects that crate ($(wc -l < "$tmp/paths" | tr -d ' ') paths)"
    else
      echo "FAIL paths read by a crate that the rules do not map to it:"
      printf '%s\n' "$missing"
      failures=$((failures + 1))
    fi
    rules=$(jq -n --argjson changed '[]' --argjson ignored '{}' --arg tier rules \
      --argjson all false --arg patch "" "$planner")
    builders=$(root_builders "$dirs" | sort -u | tr '\n' ' ')
    missing=$(jq -rn --argjson rules "$rules" --arg builders "$builders" '
      $builders | split(" ")[] | select(. != "") as $crate
      | ("machines/", "vendor/") as $prefix
      | select([$rules.data_rules[] | select(.prefix == $prefix) | .crates[]] | index($crate) | not)
      | "    \($crate) builds paths from the repository root but is not a reader of \($prefix)"')
    if [[ -z $missing ]]; then
      echo "ok   every crate that builds repository-root paths reads machines/ and vendor/ ($builders)"
    else
      echo "FAIL crates that build repository-root paths:"
      printf '%s\n' "$missing"
      failures=$((failures + 1))
    fi
    # The crates and files the rules name must exist, so a rename cannot
    # silently turn a rule off.
    missing=$(jq -rn --argjson rules "$rules" --argjson meta "$(cat "$metadata")" '
      [$meta.packages[].name] as $names
      | ($rules.emulation_crates + [$rules.data_rules[].crates[]] + [$rules.scan_tests[].package])
      | unique[] | select(. as $n | $names | index($n) | not)
      | "    crate \(.) named by a rule is not in the workspace"')
    while IFS= read -r file; do
      [[ -e $file ]] || missing+=$'\n'"    $file named by a rule does not exist"
    done < <(printf '%s' "$rules" | jq -r '.runner_files[], .data_rules[].prefix,
      (.scan_tests[] | "crates/\(.package)/tests/\(.test).rs")')
    if [[ -z $missing ]]; then
      echo "ok   every crate and file the rules name exists"
    else
      echo "FAIL rules name what does not exist:"
      printf '%s\n' "${missing#$'\n'}"
      failures=$((failures + 1))
    fi
  else
    echo "skip source checks: cargo is not on PATH"
  fi
  rm -rf "$tmp"
  if (( failures )); then
    echo "affected-tests self-test: $failures failed"
    return 1
  fi
  echo "affected-tests self-test: all passed"
}

if (( self_test )); then
  run_self_test
  exit
fi

# --------------------------------------------------------------------- plan

work=$(mktemp -d "$tmp_root/affected-tests.XXXXXX")
trap 'rm -rf "$work"' EXIT
workspace_metadata > "$work/metadata.json"
changed_files > "$work/changed"
plan_json=$(plan "$work/metadata.json" "$work/changed" "$(ignored_json "$work/metadata.json")" \
  "$tier" "$all" "$(patch_reason)")

if (( plan_only )); then
  if [[ $format == vars ]]; then print_plan_vars "$plan_json"; else print_plan_text "$plan_json"; fi
  exit 0
fi

if [[ -n $files_from ]]; then
  echo "affected-tests: changed paths from $files_from"
else
  echo "affected-tests: changes since $(git rev-parse --short "$(git merge-base "$base" HEAD)") (merge base with $base), working tree included"
fi
print_plan_text "$plan_json"

# ---------------------------------------------------------------------- run

log_dir=${AFFECTED_LOG_DIR:-$(mktemp -d "$tmp_root/affected-tests-logs.XXXXXX")}
mkdir -p "$log_dir"
mode=$(field .mode)
failed=()
summary=()

# Never unbounded: the machine may be shared with other builds.
threads=${AFFECTED_TEST_THREADS:-${RUST_TEST_THREADS:-4}}
if [[ ! $threads =~ ^[1-9][0-9]*$ ]]; then
  echo "affected-tests: test threads must be a positive number, not '$threads'" >&2
  exit 2
fi
export RUST_TEST_THREADS=$threads
runner=${AFFECTED_RUNNER:-auto}
case "$runner" in
  auto) if command -v cargo-nextest > /dev/null; then runner=nextest; else runner=cargo; fi ;;
  nextest|cargo) ;;
  *) echo "affected-tests: AFFECTED_RUNNER is nextest, cargo or auto, not '$runner'" >&2; exit 2 ;;
esac

# step NAME FORMAT COMMAND...: run, tee the output to NAME.log, record the
# time and, on failure, the failed tests (FORMAT: nextest, cargo or none).
step() {
  local name=$1 format=$2 start status test
  shift 2
  echo
  echo "== $name: $*"
  start=$(date +%s)
  set +e
  "$@" 2>&1 | tee "$log_dir/$name.log"
  status=${PIPESTATUS[0]}
  set -e
  if (( status == 0 )); then
    summary+=("PASS $name $(( $(date +%s) - start ))s")
  else
    summary+=("FAIL $name $(( $(date +%s) - start ))s (exit $status)")
    failed+=("$name")
    if [[ $format != none ]]; then
      while IFS= read -r test; do summary+=("       failed: $test"); done < <(failed_tests "$format" "$log_dir/$name.log")
    fi
  fi
}

# run_tests NAME SELECTION CARGO_ARGS...: the selected tests of the packages
# and targets in CARGO_ARGS. SELECTION is fast (not ignored), goldens (also
# the ignored ones but user-ROM tests), all (everything), user-roms (only
# ignored user-ROM tests) or names (only the ignored tests named in $names,
# by substring, user-ROM tests excepted). Both runners select the same tests.
run_tests() {
  local name=$1 selection=$2 filter=() libtest=() expr= test
  shift 2
  if [[ $runner == nextest ]]; then
    case "$selection" in
      fast) ;;
      goldens) filter=(--run-ignored all -E 'not test(/user_rom_/)') ;;
      all) filter=(--run-ignored all) ;;
      user-roms) filter=(--run-ignored only -E 'test(/user_rom_/)') ;;
      names)
        for test in "${names[@]}"; do expr+="${expr:+ or }test($test)"; done
        filter=(--run-ignored only -E "not test(/user_rom_/) and ($expr)") ;;
    esac
    step "$name" nextest "${cargo_cmd[@]}" nextest run --locked "$@" --no-fail-fast \
      --test-threads "$threads" --no-tests warn ${filter[@]+"${filter[@]}"}
  else
    case "$selection" in
      fast) ;;
      goldens) libtest=(--include-ignored --skip user_rom_) ;;
      all) libtest=(--include-ignored) ;;
      user-roms) libtest=(--ignored user_rom_) ;;
      names) libtest=(--ignored --skip user_rom_ "${names[@]}") ;;
    esac
    step "$name" cargo "${cargo_cmd[@]}" test --locked "$@" --no-fail-fast -- ${libtest[@]+"${libtest[@]}"}
  fi
}

if [[ $mode == none ]]; then
  echo "nothing changed: no tests to run"
  exit 0
fi

# git diff --check of the changed documentation, untracked files included.
check_whitespace() {
  local merge_base status=0 file rc tracked=()
  merge_base=$(git merge-base "$base" HEAD)
  for file in "$@"; do
    if git ls-files --error-unmatch -- "$file" > /dev/null 2>&1; then
      tracked+=("$file")
    elif [[ -f $file ]]; then
      # Against /dev/null git reports a difference (bit 0) and, for
      # whitespace errors, bit 1.
      rc=0
      git diff --no-index --check -- /dev/null "$file" || rc=$?
      if (( rc & 2 )); then status=1; fi
    fi
  done
  if (( ${#tracked[@]} )); then
    git diff --check "$merge_base" -- "${tracked[@]}" || status=1
  fi
  return "$status"
}

# Checks that need no build, for every change.
if (( guard )); then step guard none bash scripts/publication-guard.sh; fi
docs=()
while IFS= read -r path; do [[ -n $path ]] && docs+=("$path"); done < <(field '.docs[]')
if (( ${#docs[@]} )); then step whitespace none check_whitespace "${docs[@]}"; fi
while IFS= read -r script; do
  [[ -n $script && -f $script ]] || continue
  step "syntax-$(basename "$(dirname "$script")")-$(basename "$script" .sh)" none bash -n "$script"
done < <(field '.shell[]')
if [[ $(field .self_test) == true ]]; then step self-test none bash scripts/affected-tests.sh --self-test; fi

if [[ $mode == crates || $mode == full ]]; then
  if [[ $runner == nextest ]]; then
    echo
    echo "runner: $(cargo-nextest nextest --version 2>/dev/null | head -n 1 || echo cargo-nextest), at most $threads tests at once"
  else
    echo
    echo "runner: cargo test (cargo-nextest is not on PATH or AFFECTED_RUNNER=cargo), $threads test threads per binary"
  fi
  if [[ $mode == full ]]; then
    packages=(--workspace)
  else
    packages=()
    while IFS= read -r crate; do packages+=(-p "$crate"); done < <(field '.affected[]')
  fi
  selection=fast
  if [[ $(field .tier) == goldens ]]; then
    if [[ $(field .user_roms) == true ]]; then selection=all; else selection=goldens; fi
  fi
  run_tests tests "$selection" "${packages[@]}"

  # nextest does not run doctests. Only a library has doctests, and cargo
  # refuses `--doc` for a selection of binary-only packages (the desktop).
  if [[ $mode == full ]]; then
    doc_packages=(--workspace)
  else
    doc_packages=()
    while IFS= read -r crate; do doc_packages+=(-p "$crate"); done < <(
      jq -r --argjson affected "$(field '.affected' | jq -c .)" '.packages[]
        | select(.name as $n | $affected | index($n))
        | select(any(.targets[]?; .kind | index("lib")))
        | .name' "$work/metadata.json")
  fi
  if [[ $runner == nextest && ${#doc_packages[@]} -gt 0 ]]; then
    libtest=()
    case "$selection" in
      goldens) libtest=(--include-ignored --skip user_rom_) ;;
      all) libtest=(--include-ignored) ;;
    esac
    step doctests cargo "${cargo_cmd[@]}" test --locked "${doc_packages[@]}" --doc --no-fail-fast \
      -- ${libtest[@]+"${libtest[@]}"}
  fi

  # Same packages, so the build is the one above; only the named tests run.
  names=()
  while IFS= read -r test; do [[ -n $test ]] && names+=("$test"); done < <(field '.targeted[]')
  if (( ${#names[@]} )); then
    run_tests changed-goldens names "${packages[@]}" --tests
  fi

  if [[ $(field .user_roms) == true && $(field .tier) != goldens ]]; then
    run_tests user-roms user-roms "${packages[@]}"
  fi

  while IFS= read -r scan; do
    [[ -n $scan ]] || continue
    run_tests "scan-${scan#*:}" fast -p "${scan%%:*}" --test "${scan#*:}"
  done < <(field '.scan_tests[]')

  # Release builds keep behaviours that debug builds assert against.
  if [[ $(field .release_devices) == true ]]; then
    run_tests devices-release fast --release -p devices
  fi

  echo
  if [[ $runner == nextest ]]; then
    echo "slowest tests (seconds):"
    cat "$log_dir"/*.log | sed -nE 's/^ *PASS +\[ *([0-9.]+)s\]( +\( *[0-9]+\/ *[0-9]+\))? +(.*)$/\1 \3/p' \
      | sort -rn | head -n 10 | awk '{ t = $1; $1 = ""; printf "%9.2f %s\n", t, $0 }' || true
    sed -nE 's/^ *Summary +\[ *([0-9.]+)s\] (.*)$/  \2 in \1 s/p' "$log_dir"/*.log || true
  else
    # Test binaries run one after another, so their times add up.
    echo "slowest test binaries (seconds, from libtest's 'finished in'):"
    cat "$log_dir"/*.log | awk '
      /^ *Running /{ p = $NF; gsub(/[()]/, "", p); sub(/.*\//, "", p); sub(/-[0-9a-f]+$/, "", p); bin = p " " $2 }
      /^ *Doc-tests /{ bin = $2 " doc-tests" }
      /test result:/ && match($0, /finished in [0-9.]+s/) {
        printf "%9.2f  %s\n", substr($0, RSTART + 12, RLENGTH - 13), bin }' | sort -rn | head -n 10 || true
  fi
fi

echo
echo "summary (logs in $log_dir):"
printf '  %s\n' ${summary[@]+"${summary[@]}"}
if (( ${#failed[@]} )); then
  echo "affected-tests: failed: ${failed[*]}"
  exit 1
fi
echo "affected-tests: passed"
