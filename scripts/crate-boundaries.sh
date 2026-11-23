#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Keep emulation libraries independent of PC host tools. Every workspace
# package declares its layer in package.metadata.unemu. The emulation layer
# may reach only itself and the contract, across every dependency kind.
# Source includes must stay inside this repository.
#
# One copy of the contract. Two revisions of unemu-core-api in one build are
# two different crates, and a core built against one does not implement the
# other's traits; the compiler then fails with a long error far from the
# cause. So the resolved dependency graph (`cargo metadata --offline --locked`,
# every package the lock file pins) must hold exactly one package named
# unemu-core-api; otherwise the check prints each copy's source and the
# packages that depend on it.
#
# The check reads the dependencies each manifest declares (optional and
# target-specific ones included), not a resolved build. From each crate it
# follows the crate's own dependencies of every kind, then the normal and build
# dependencies of the crates it reaches, and reports the first forbidden crate
# on each path once, with the path.
#
# Known violations are listed in scripts/crate-boundaries.allow, one declared
# dependency per line: `FROM → TO (KIND): REASON`, KIND being normal, build or
# dev. The walk never crosses a listed dependency, so a crate that reaches it
# through FROM needs no entry of its own. A listed dependency that is gone or
# no longer forbidden fails, so the list only shrinks.
#
# External crates of the core (core-contract.md §12.1 and §12.3). The contract
# and emulation layers are the library an embedder links, so the crates from
# outside the workspace they may reach are listed one by one in
# scripts/crate-boundaries.external, one per line:
# `LAYER: CRATE (run|build|test): REASON`. The other layers are not limited by
# it. From every workspace package of a limited layer the check walks the
# resolved graph (the lock file's, for every target): the package's own
# dependencies of every kind, then the normal and build dependencies of each
# crate it reaches, workspace crates of any layer included. A crate is of kind
#
#   run    reached through normal dependencies alone, and through no
#          procedural macro;
#   build  otherwise reached through a build dependency or a procedural macro
#          (the macro itself included): it runs on the build host and is not
#          linked;
#   test   reached only through a test (dev) dependency of a package of the
#          layer;
#
# the lowest kind of any path counting. Each external crate the layer reaches
# must be listed with exactly that kind; an unlisted one is reported once,
# with the first path of its kind a breadth-first walk finds. The list names
# crates, not versions: the lock file and cargo deny own those. A listed crate
# that the layer no longer reaches fails, so the list is exact and shrinks as
# crates leave.
#
# The graph is cargo metadata's resolve, read with --all-features, not the
# build of one package: it holds dependencies of weak optional features
# (`dep?/feature`), edges under a cfg no target matches (`cfg(any())`) and the
# features the whole workspace unifies. So a line may name a crate the core's
# own build does not compile (`cargo tree -e normal,build` of the emulation
# packages leaves it out), and workspace feature unification can move
# the list. The list errs on the side of naming too much.
#
# Usage:
#   scripts/crate-boundaries.sh                  check the workspace
#   scripts/crate-boundaries.sh --self-test      run the rules over the fixtures
#   scripts/crate-boundaries.sh --metadata FILE [--allow FILE]
#                                                check `cargo metadata --no-deps` output
#   scripts/crate-boundaries.sh --resolved FILE  check one copy of the contract in
#                                                full `cargo metadata` output
#   scripts/crate-boundaries.sh --external FILE [--list FILE]
#                                                check the external crates of the core
#                                                in full `cargo metadata` output
#
# Needs cargo, jq and python3 (all in the devShell). The workspace check reads one
# `cargo metadata --no-deps`, which resolves nothing, and one resolved
# `cargo metadata --offline --locked --all-features`, which reads the lock file and the local
# dependency cache; both take well under a second.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
default_allow="$root/scripts/crate-boundaries.allow"
default_external="$root/scripts/crate-boundaries.external"
fixtures="$root/scripts/crate-boundaries-fixtures"
# The crate every core and front end must link exactly once.
contract_crate=unemu-core-api

# Prints {"errors": [...], "packages": N, "allowed": N} for one metadata
# document and one allow list.
# Quoted assignments avoid Bash pipe deadlocks while loading these programs.
program='
def layers: ["contract", "emulation", "pc", "application"];
def may_use: {
  contract: [],
  emulation: ["contract", "emulation"],
  pc: ["contract", "emulation", "pc"],
  application: ["contract", "emulation", "pc", "application"]
};
def known_layer: . as $l | any(layers[]; . == $l);
def article: if test("^[aeiou]") then "an \(.)" else "a \(.)" end;
# The rule $from breaks by depending on $to, if any. A crate without a known
# layer is reported on its own, not as a dependency.
def broken_rule($layer; $core; $from; $to):
  $layer[$from] as $fl | $layer[$to] as $tl
  | if $tl == null or ($tl | known_layer | not) then empty
    elif any(may_use | .[$fl][]; . == $tl) | not then
      "\($fl | article) crate may depend only on \(may_use | .[$fl] | join(", "))"
    else empty end;

def listed($valid; $from; $to; $kind):
  any($valid[]; .from == $from and .to == $to and .kind == $kind);

# The forbidden crates $from reaches, each once, by the shortest path, with
# the kind of the starting dependency of $from (normal before
# build before dev). The walk follows every dependency of $from and
# then the normal and build dependencies of the crates it reaches: a
# transitive dependency never builds its own test dependencies. It stops at
# a forbidden crate and never crosses an allow-listed edge.
def walk($graph; $layer; $core; $valid; $from):
  ([("normal", "build", "dev") as $k | ($graph[$from] // [])[] | select(.kind == $k)]
   | map(select(listed($valid; $from; .to; .kind) | not) | {path: [$from, .to], kind: .kind})) as $first
  | {queue: $first, seen: [$from], out: []}
  | until(.queue == [];
      .queue[0] as $item
      | .queue |= .[1:]
      | $item.path[-1] as $node
      | if any(.seen[]; . == $node) then .
        else .seen += [$node]
        | ([broken_rule($layer; $core; $from; $node)] | first // null) as $rule
        | if $rule != null then .out += [$item + {rule: $rule}]
          else .queue += [($graph[$node] // [])[] | select(.kind != "dev")
                          | select(listed($valid; $node; .to; .kind) | not)
                          | {path: ($item.path + [.to]), kind: $item.kind}]
          end
        end)
  | .out[];

($allow_text | split("\n") | to_entries
  | map(select(.value | test("^\\s*(#|$)") | not))
  | map(. as $line
      | (($line.value
          | capture("^(?<from>[^ ]+) → (?<to>[^ ]+) \\((?<kind>normal|build|dev)\\): (?<reason>[^ ].*)$"))
         // {malformed: true})
      | . + {line: ($line.key + 1), text: $line.value})) as $entries
| ($entries | map(select(.malformed | not))) as $valid
| (.packages // []) as $packages
| ($packages | map(.name)) as $names
| ($packages | map({key: .name, value: (.metadata.unemu.layer? // null)}) | from_entries) as $layer
| ($packages | map({key: .name, value: (.metadata.unemu.layer == "emulation"
    or .metadata.unemu.side? == "core")}) | from_entries) as $core
| ($packages
   | map({key: .name, value: ([.dependencies[]?
       | select(.source == null)
       | .name as $n | select(any($names[]; . == $n))
       | {to: .name, kind: (.kind // "normal")}] | unique)})
   | from_entries) as $graph
# An entry stands for one declared dependency that breaks a rule of its own
# crate; anything else is stale.
| ($valid | map(. as $e
    | . + {live: (any(($graph[$e.from] // [])[]; .to == $e.to and .kind == $e.kind)
                  and ([broken_rule($layer; $core; $e.from; $e.to)] | length > 0))})) as $checked
| [ $packages[] | .name as $from | $layer[$from] as $fl
    | select($fl != null and $fl != "contract" and ($fl | known_layer))
    | walk($graph; $layer; $core; $valid; $from) as $hit
    | $hit.path[-1] as $to
    | "forbidden: \($from) (\($fl)) → \($to) (\($layer[$to])), \($hit.kind) dependency, through \($hit.path | join(" → ")); rule: \($hit.rule)"
  ] as $forbidden
| {
    packages: ($packages | length),
    allowed: ([$checked[] | select(.live)] | length),
    errors: (
      (if $packages == [] then ["no packages: the metadata lists no workspace package"] else [] end)
      + [ $packages[] | select($layer[.name] == null)
        | "no layer: \(.name) has no [package.metadata.unemu] layer; set one of \(layers | join(", "))" ]
      + [ $packages[] | select($layer[.name] != null and ($layer[.name] | known_layer | not))
        | "unknown layer: \(.name) declares layer \($layer[.name] | tojson); use one of \(layers | join(", "))" ]
      + [ $packages[] | select($layer[.name] == "contract") | .name as $from | .dependencies[]?
        | "forbidden: \($from) (contract) depends on \(.name) (\(.kind // "normal") dependency); rule: a contract crate depends on nothing" ]
      + $forbidden
      + [ $checked[] | select(.live | not)
          | "stale allow entry, line \(.line): \(.text); that dependency is gone or no longer forbidden, so delete the line" ]
      + [ $entries[] | select(.malformed)
          | "malformed allow entry, line \(.line): \(.text); expected FROM → TO (normal|build|dev): REASON" ]
    )
  }
'

# Prints {"errors": [...], "source": S} for one resolved metadata document:
# an error unless exactly one package is named $crate.
one_copy_program='
(.packages // []) as $packages
| ($packages | map({key: .id, value: .name}) | from_entries) as $name_of
| [$packages[] | select(.name == $crate)] as $copies
| def where: .source // "path \(.manifest_path | rtrimstr("/Cargo.toml"))";
  def users($id):
    [(.resolve.nodes // [])[] | select(any(.deps[]?; .pkg == $id)) | $name_of[.id]]
    | unique | if . == [] then "no package" else join(", ") end;
  . as $doc
| {
    source: (if ($copies | length) == 1 then ($copies[0] | where) else null end),
    errors: (
      if ($copies | length) == 1 then []
      elif $copies == [] then
        ["one copy: the resolved graph has no package named \($crate)"]
      else
        ["one copy: \($copies | length) copies of \($crate) in the resolved graph; every core and front end must link the same one"]
        + [$copies[] | "    \(.version) from \(where), used by \(.id as $id | $doc | users($id))"]
      end)
  }
'

# Prints {"errors": [...], "reached": {LAYER: {"run": N, "build": N,
# "test": N}}} for one resolved metadata document and one external-crate list.
external_program='
# The layers whose external crates are listed; the others are not limited.
def limited: ["contract", "emulation"];
def rank: {run: 0, build: 1, test: 2}[.];
def worse($a; $b): if ($a | rank) >= ($b | rank) then $a else $b end;
def article: if test("^[aeiou]") then "an \(.)" else "a \(.)" end;

($list_text | split("\n") | to_entries
  | map(select(.value | test("^\\s*(#|$)") | not))
  | map(. as $line
      | (($line.value
          | capture("^(?<layer>[a-z]+): (?<crate>[^ ]+) \\((?<kind>run|build|test)\\): (?<reason>[^ ].*)$"))
         // {malformed: true})
      | . + {line: ($line.key + 1), text: $line.value})) as $entries
| ($entries | map(select((.malformed | not) and (.layer as $l | any((limited + ["application"])[]; . == $l) | not))))
  as $unlimited
| ($entries | map(select((.malformed | not) and (.layer as $l | any((limited + ["application"])[]; . == $l))))) as $valid
| (.packages // []) as $packages
| (.workspace_members // []) as $members
| ($packages | map({key: .id, value: {
      name,
      workspace: (.id as $id | any($members[]; . == $id)),
      layer: (.metadata.unemu.layer? // null),
      proc_macro: any(.targets[]?.kind[]?; . == "proc-macro")}})
   | from_entries) as $pkg
| ((.resolve.nodes // []) | map({key: .id, value: [.deps[]? | .pkg as $to
      | ([.dep_kinds[]? | .kind // "normal"] | unique)[] | {to: $to, kind: .}]})
   | from_entries) as $edges
# The crates one step from $item, with the kind of the path through them. A
# package of the layer contributes its test dependencies; nothing else does.
| def step($item):
    ($edges[$item.id] // [])[]
    | select($item.root or .kind != "dev")
    | .to as $to
    | (if .kind == "dev" then "test"
       elif .kind == "build" or $pkg[$to].proc_macro then "build"
       else "run" end) as $k
    | {id: $to, kind: worse($item.kind; $k), path: ($item.path + [$pkg[$to].name]), root: false};
  # Every external crate the workspace packages of $layer reach, once by name,
  # with its lowest kind and the first path of that kind found: the walk
  # drains the run queue before the build queue before the test queue, each
  # breadth first, so a crate is first seen with its lowest kind.
  def reach($layer):
    ([$packages[] | select($pkg[.id].workspace and $pkg[.id].layer == $layer)]
     | sort_by(.name) | map({id, kind: "run", path: [.name], root: true})) as $roots
    | {queues: [$roots, [], []], seen: {}, out: []}
    | until(all(.queues[]; . == []);
        (if .queues[0] != [] then 0 elif .queues[1] != [] then 1 else 2 end) as $q
        | .queues[$q][0] as $item
        | .queues[$q] |= .[1:]
        | if .seen[$item.id] then .
          else .seen[$item.id] = true
          | (if $pkg[$item.id].workspace then . else .out += [$item] end)
          | reduce step($item) as $next (.; .queues[$next.kind | rank] += [$next])
          end)
    | .out
    | group_by($pkg[.id].name)
    | map(min_by(.kind | rank) | {crate: $pkg[.id].name, kind, path});
  ([(limited + ["pc", "application"])[] as $layer | {key: $layer, value: reach($layer)}] | from_entries) as $reached
| def rule($layer):
    "\($layer | article) crate reaches only the external crates scripts/crate-boundaries.external lists for its layer, each with the kind it is reached by";
  def hit($layer):
    "\(.path[0]) (\($layer)) → \(.crate) (external), \(.kind) dependency, through \(.path | join(" → "))";
{
  reached: ($reached | map_values(group_by(.kind) | map({key: .[0].kind, value: length}) | from_entries)),
  errors: (
    [ limited[] as $layer | $reached[$layer][] as $hit
      | [$valid[] | select(.layer == $layer and .crate == $hit.crate)] as $lines
      | if $lines == [] then
          "forbidden: \($hit | hit($layer)); rule: \(rule($layer))"
        elif any($lines[]; .kind == $hit.kind) | not then
          "wrong kind: \($hit | hit($layer)), listed as \($lines[0].kind) on line \($lines[0].line); rule: \(rule($layer))"
        else empty end ]
    + [ $reached.pc[] as $hit
        | select(any($valid[]; .layer == "application" and .crate == $hit.crate))
        | "forbidden: \($hit | hit("pc")); rule: window and audio crates belong only to the application layer" ]
    + [ $valid[] as $e
        | select(any($reached[$e.layer][]; .crate == $e.crate) | not)
        | "stale external entry, line \($e.line): \($e.text); the \($e.layer) layer no longer reaches \($e.crate), so delete the line" ]
    + [ $valid | group_by([.layer, .crate])[] | select(length > 1)
        | "duplicate external entry, lines \(map(.line) | join(", ")): \(.[0].layer): \(.[0].crate); name each crate once" ]
    + [ $unlimited[]
        | "external entry for an unlimited layer, line \(.line): \(.text); the list limits only \(limited | join(", "))" ]
    + [ $entries[] | select(.malformed)
        | "malformed external entry, line \(.line): \(.text); expected LAYER: CRATE (run|build|test): REASON" ]
  )
}
'

# check_external METADATA LIST: print each error and a summary; fail on any
# error.
check_external() {
    local result count
    result=$(jq -c --rawfile list_text "$2" "$external_program" "$1") || {
        echo "crate boundaries: cannot read the resolved metadata $1 with the external list $2" >&2
        return 2
    }
    printf '%s' "$result" | jq -r '.errors[]'
    count=$(printf '%s' "$result" | jq '.errors | length')
    if [[ $count -ne 0 ]]; then
        echo "crate boundaries: $count external-crate error(s)" >&2
        return 1
    fi
    printf '%s' "$result" | jq -r '.reached | to_entries[] | select(.key == "contract" or .key == "emulation")
        | "crate boundaries: the \(.key) layer reaches \(.value.run // 0) run-time, \(.value.build // 0) build-time and \(.value.test // 0) test-only external crate(s), all listed"'
}

# check_one_copy METADATA: print each error, or the contract's source; fail
# unless the graph holds exactly one copy.
check_one_copy() {
    local result
    result=$(jq -c --arg crate "$contract_crate" "$one_copy_program" "$1") || {
        echo "crate boundaries: cannot read the resolved metadata $1" >&2
        return 2
    }
    printf '%s' "$result" | jq -r '.errors[]'
    if [[ $(printf '%s' "$result" | jq '.errors | length') -ne 0 ]]; then
        echo "crate boundaries: not exactly one copy of $contract_crate" >&2
        return 1
    fi
    printf '%s' "$result" | jq -r --arg crate "$contract_crate" '"crate boundaries: one copy of \($crate), from \(.source)"'
}

# check METADATA ALLOW: print each error and a summary; fail on any error.
check() {
    local result count
    result=$(jq -c --rawfile allow_text "$2" "$program" "$1") || {
        echo "crate boundaries: cannot read $1 with the allow list $2" >&2
        return 2
    }
    printf '%s' "$result" | jq -r '.errors[]'
    count=$(printf '%s' "$result" | jq '.errors | length')
    if [[ $count -ne 0 ]]; then
        echo "crate boundaries: $count error(s)" >&2
        return 1
    fi
    printf '%s' "$result" | jq -r '"crate boundaries: \(.packages) crates checked, \(.allowed) allowed exception(s)"'
}

# Keep Python in a file: large here-documents can deadlock Bash pipes.
check_sources() {
    python3 "$root/scripts/crate-boundaries.py" "$@"
}

# Each fixture is metadata with one planted case and its expectation:
# "self_test": {"expect": "pass"|"fail", "message": "text the output contains",
# "errors": N (optional, how many errors it reports)}, and an optional allow
# list NAME.allow beside it.
self_test() {
    local fixture name allow list expect message errors output status failed=0
    for fixture in "$fixtures"/*.json; do
        name=$(basename "$fixture" .json)
        allow="$fixtures/$name.allow"
        [[ -f $allow ]] || allow=/dev/null
        expect=$(jq -r '.self_test.expect' "$fixture")
        message=$(jq -r '.self_test.message // ""' "$fixture")
        errors=$(jq -r '.self_test.errors // ""' "$fixture")
        status=0
        output=$(check "$fixture" "$allow" 2>&1) || status=$?
        if [[ $expect == pass && $status -eq 0 ]] ||
            [[ $expect == fail && $status -ne 0 && $output == *"$message"* &&
                ( -z $errors || $output == *"crate boundaries: $errors error(s)"* ) ]]; then
            echo "self-test $name: ${expect}ed as expected"
        else
            echo "self-test $name: expected $expect${message:+ with \"$message\"}${errors:+ and $errors error(s)}, got status $status:" >&2
            printf '%s\n' "$output" | sed 's/^/    /' >&2
            failed=1
        fi
    done
    # Fixtures of resolved metadata for the one-copy rule, with the same
    # "self_test" expectation (without "errors").
    for fixture in "$fixtures"/resolved/*.json; do
        name=resolved/$(basename "$fixture" .json)
        expect=$(jq -r '.self_test.expect' "$fixture")
        message=$(jq -r '.self_test.message // ""' "$fixture")
        status=0
        output=$(check_one_copy "$fixture" 2>&1) || status=$?
        if [[ $expect == pass && $status -eq 0 ]] ||
            [[ $expect == fail && $status -ne 0 && $output == *"$message"* ]]; then
            echo "self-test $name: ${expect}ed as expected"
        else
            echo "self-test $name: expected $expect${message:+ with \"$message\"}, got status $status:" >&2
            printf '%s\n' "$output" | sed 's/^/    /' >&2
            failed=1
        fi
    done
    # Fixtures of resolved metadata for the external-crate lists, each with an
    # optional list NAME.external beside it and the same "self_test"
    # expectation as the first kind.
    for fixture in "$fixtures"/external/*.json; do
        name=external/$(basename "$fixture" .json)
        list="${fixture%.json}.external"
        [[ -f $list ]] || list=/dev/null
        expect=$(jq -r '.self_test.expect' "$fixture")
        message=$(jq -r '.self_test.message // ""' "$fixture")
        errors=$(jq -r '.self_test.errors // ""' "$fixture")
        status=0
        output=$(check_external "$fixture" "$list" 2>&1) || status=$?
        if [[ $expect == pass && $status -eq 0 ]] ||
            [[ $expect == fail && $status -ne 0 && $output == *"$message"* &&
                ( -z $errors || $output == *"crate boundaries: $errors external-crate error(s)"* ) ]]; then
            echo "self-test $name: ${expect}ed as expected"
        else
            echo "self-test $name: expected $expect${message:+ with \"$message\"}${errors:+ and $errors external-crate error(s)}, got status $status:" >&2
            printf '%s\n' "$output" | sed 's/^/    /' >&2
            failed=1
        fi
    done
    check_sources --self-test || failed=1
    return "$failed"
}

case "${1:-}" in
    "")
        metadata=$(mktemp)
        resolved=$(mktemp)
        trap 'rm -f "$metadata" "$resolved"' EXIT
        (cd "$root" && cargo metadata --format-version 1 --no-deps) >"$metadata"
        (cd "$root" && cargo metadata --format-version 1 --offline --locked --all-features) >"$resolved"
        status=0
        check "$metadata" "$default_allow" || status=$?
        check_sources "$metadata" || { s=$?; [[ $status -ne 0 ]] || status=$s; }
        check_one_copy "$resolved" || { s=$?; [[ $status -ne 0 ]] || status=$s; }
        check_external "$resolved" "$default_external" || { s=$?; [[ $status -ne 0 ]] || status=$s; }
        exit "$status"
        ;;
    --self-test)
        [[ $# -eq 1 ]] || { echo "usage: $0 --self-test" >&2; exit 2; }
        self_test
        ;;
    --metadata)
        [[ $# -eq 2 || ($# -eq 4 && $3 == --allow) ]] ||
            { echo "usage: $0 --metadata FILE [--allow FILE]" >&2; exit 2; }
        check "$2" "${4:-/dev/null}"
        ;;
    --resolved)
        [[ $# -eq 2 ]] || { echo "usage: $0 --resolved FILE" >&2; exit 2; }
        check_one_copy "$2"
        ;;
    --external)
        [[ $# -eq 2 || ($# -eq 4 && $3 == --list) ]] ||
            { echo "usage: $0 --external FILE [--list FILE]" >&2; exit 2; }
        check_external "$2" "${4:-$default_external}"
        ;;
    *)
        echo "usage: $0 [--self-test | --metadata FILE [--allow FILE] | --resolved FILE | --external FILE [--list FILE]]" >&2
        exit 2
        ;;
esac
