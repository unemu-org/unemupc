#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Write THIRD-PARTY-LICENSES.md for the release binary (unemupc):
# the components inside this repository that keep their own licence, then every
# Rust crate the binary links for the exact Cargo.lock, with the licence files
# each published crate ships. Identical licence texts are written once.
#
# usage: scripts/third-party-licenses.sh [OUTPUT [TARGET]]
#   OUTPUT  file to write, relative to the calling directory;
#           THIRD-PARTY-LICENSES.md at the repository root by default; "-"
#           writes to standard output.
#   TARGET  a target triple such as x86_64-unknown-linux-gnu: list only the
#           crates that platform links. Without it, every platform's crates are
#           listed.
#
# The release workflow runs it with the runner's cargo and jq; the output is generated, so
# it is gitignored rather than committed. It needs only cargo, jq and a SHA-256
# tool (sha256sum or shasum). cargo-deny can list licence expressions but not
# their texts, and cargo-about is not in the devShell. cargo metadata reads the
# crate sources from cargo's cache and downloads missing ones unless
# CARGO_NET_OFFLINE=true; Cargo.lock is never changed (--locked).
#
# A crate's licence files are the ones it publishes; a crate from a git
# repository may keep them at the repository root instead, which is searched
# too. A crate that publishes none gets the standard texts of its licence
# expression from LICENSES/ and its `authors` as the copyright holders.
#
# A crate counts when the binary links it: normal dependencies followed from
# the unemupc package, for the features a release build of that
# package enables (cargo tree). Build and dev dependencies and procedural
# macros run only while compiling and are left out.
set -euo pipefail

caller=$PWD
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

output=${1:-$PWD/THIRD-PARTY-LICENSES.md}
target=${2:-}
if [[ $output != - && $output != /* ]]; then
  output=$caller/$output
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  printf '%s\n' 'third-party-licenses: needs sha256sum or shasum' >&2
  exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

metadata=(cargo metadata --locked --format-version 1)
if [[ -n $target ]]; then
  metadata+=(--filter-platform "$target")
fi
"${metadata[@]}" > "$work/metadata.json"
# cargo metadata unifies the features of every workspace target, tests included;
# cargo tree resolves them as building the binary does. Its "name vX.Y.Z"
# lines are the packages such a build can contain.
cargo tree --locked -e normal -p unemupc --target "${target:-all}" \
  --prefix none --format '{p}' |
  LC_ALL=C awk '{ sub(/^v/, "", $2); print $1 " " $2 }' | LC_ALL=C sort -u > "$work/built.txt"

# One tab-separated line per linked third-party crate:
# name, version, licence, source, crate directory, authors.
jq -r --rawfile built "$work/built.txt" '
  (.packages | map({key: .id, value: .}) | from_entries) as $packages
  | (.resolve.nodes | map({key: .id, value: .}) | from_entries) as $nodes
  | ($built | split("\n") | map(select(length > 0) | {key: ., value: true}) | from_entries)
    as $in_build
  | def proc_macro($id): $packages[$id].targets | any(.kind | index("proc-macro"));
    def in_build($id): $in_build[$packages[$id].name + " " + $packages[$id].version] == true;
    def linked($id):
      [$nodes[$id].deps[]
       | select(any(.dep_kinds[]; .kind == null))
       | .pkg
       | select(in_build(.) and (proc_macro(.) | not))];
  [.packages[] | select(.source == null and (.name == "unemupc")) | .id] as $roots
  | {seen: {}, todo: $roots}
  | until(.todo | length == 0;
      .todo[0] as $id
      | .todo |= .[1:]
      | if .seen[$id] then . else .seen[$id] = true | .todo += linked($id) end)
  | .seen | keys | map($packages[.]) | map(select(.source != null))
  | sort_by(.name, .version)[]
  | [.name, .version,
     (.license // ("see " + (.license_file // "the crate"))),
     (.source | sub("^registry\\+https://github.com/rust-lang/crates.io-index$"; "crates.io")
              | sub("^sparse\\+https://index.crates.io/$"; "crates.io")
              | sub("#.*$"; "")),
     (.manifest_path | sub("/Cargo.toml$"; "")),
     ((.authors // []) | join("; "))]
  | @tsv' "$work/metadata.json" > "$work/crates.tsv"

# The licence files at the top of DIRECTORY and in its LICENSES/ folder.
licence_files() {
  find "$1" -maxdepth 2 -type f \
    \( -path "$1/LICENSES/*" -o -path "$1/licenses/*" \
       -o \( -path "$1/*" ! -path "$1/*/*" \
             \( -iname 'LICEN[CS]E*' -o -iname 'COPYING*' -o -iname 'COPYRIGHT*' \
                -o -iname 'NOTICE*' -o -iname 'UNLICENSE*' \) \) \) \
    -print0 | LC_ALL=C sort -z
}

# The bundled standard text of one SPDX licence identifier, if any. The font
# licences already appear among the components of this repository.
standard_text() {
  case "$1" in
    LGPL-2.1-or-later | LGPL-2.1-only) printf '%s\n' crates/devices/src/opl3/LICENSE ;;
    *) if [[ -f LICENSES/$1.txt ]]; then printf '%s\n' "LICENSES/$1.txt"; fi ;;
  esac
}

mkdir "$work/texts" "$work/crates-of"
: > "$work/unshipped"
: > "$work/standard"
while IFS=$'\t' read -r name version licence source directory authors; do
  found=0
  # A git dependency's licence may sit at its repository root: the checkout's
  # top directory, which cargo marks with .cargo-ok, up to three levels above
  # the crate (crates/NAME in a workspace of several crates). Every directory
  # up to that root is searched only when the marker is found; without it (a
  # vendored or other source) only the crate's parent is, so a directory
  # outside the dependency, such as this repository, is never taken for it.
  roots=("$directory")
  if [[ $source == git+* ]]; then
    climbed=()
    up=$directory
    checkout=0
    for _ in 1 2 3; do
      up=$(dirname "$up")
      climbed+=("$up")
      if [[ -e $up/.cargo-ok ]]; then
        checkout=1
        break
      fi
    done
    if (( checkout )); then
      roots+=("${climbed[@]}")
    else
      roots+=("$(dirname "$directory")")
    fi
  fi
  for root in "${roots[@]}"; do
    while IFS= read -r -d '' file; do
      found=1
      hash=$(sha256 "$file")
      [[ -f $work/texts/$hash ]] || cp "$file" "$work/texts/$hash"
      if [[ $file == "$directory"/* ]]; then
        shown=${file#"$directory"/}
      else
        shown="repository root: ${file#"$root"/}"
      fi
      printf '%s %s (%s)\n' "$name" "$version" "$shown" >> "$work/crates-of/$hash"
    done < <(licence_files "$root")
  done
  if (( ! found )); then
    ids=$(printf '%s\n' "$licence" | LC_ALL=C grep -oE '[[:alnum:].+-]+' |
      LC_ALL=C grep -vxE 'AND|OR|WITH|see' | LC_ALL=C sort -u | tr '\n' ' ')
    texts=''
    for id in $ids; do
      text=$(standard_text "$id")
      if [[ -n $text ]]; then
        texts+="${texts:+, }$id"
        printf '%s\t%s\n' "$id" "$text" >> "$work/standard"
      else
        printf 'third-party-licenses: no bundled text for %s (%s %s)\n' "$id" "$name" "$version" >&2
        texts+="${texts:+, }$id (text not bundled)"
      fi
    done
    printf '%s %s: %s; copyright holders: %s; standard texts: %s\n' \
      "$name" "$version" "$licence" "${authors:-the authors of $name}" "${texts:-none}" \
      >> "$work/unshipped"
  fi
done < "$work/crates.tsv"

# A fence longer than any backtick run in the texts keeps them verbatim.
fence='`````'

emit_text() {
  printf '%s text\n' "$fence"
  cat "$1"
  # A text without a final newline still closes its block.
  [[ -z $(tail -c 1 "$1") ]] || printf '\n'
  printf '%s\n\n' "$fence"
}

{
  printf '# Third-party licences\n\n'
  printf 'Generated by `scripts/third-party-licenses.sh` from the `Cargo.lock` with\n'
  printf 'SHA-256 `%s`, for %s. Do not edit.\n\n' \
    "$(sha256 Cargo.lock)" "${target:-every platform}"
  printf 'unemupc is licensed under AGPL-3.0-or-later (`LICENSE`). It\n'
  printf 'contains the components below, each under its own licence.\n\n'

  printf '## Components in this repository\n\n'
  printf -- '- Nuked-OPL3 1.8, (c) 2013-2020 Nuke.YKT, LGPL-2.1-or-later: the OPL3 FM\n'
  printf '  generator is a Rust translation (`crates/devices/src/opl3/`).\n'
  printf -- '- Ultimate Oldschool PC Font Pack 2.2, (c) 2016-2020 VileR, CC BY-SA 4.0:\n'
  printf '  the MDA and VGA bitmap fonts, unmodified.\n\n'
  for file in \
    crates/devices/src/opl3/LICENSE \
    crates/pc-core/assets/fonts/NOTICE.md \
    crates/pc-core/assets/fonts/LICENSE.txt; do
    printf '### `%s`\n\n' "$file"
    emit_text "$file"
  done

  printf '## Rust crates\n\n'
  printf '| Crate | Version | Licence | Source |\n| --- | --- | --- | --- |\n'
  while IFS=$'\t' read -r name version licence source _ _; do
    printf '| %s | %s | %s | %s |\n' "$name" "$version" "$licence" "$source"
  done < "$work/crates.tsv"
  printf '\n'

  if [[ -s $work/unshipped ]]; then
    printf '### Crates published without a licence file\n\n'
    printf 'Each is licensed under the SPDX expression shown, by the copyright\n'
    printf 'holders its metadata names as authors; the standard texts follow.\n\n'
    while IFS= read -r line; do
      printf -- '- %s\n' "$line"
    done < "$work/unshipped"
    printf '\n'
    LC_ALL=C sort -u "$work/standard" | while IFS=$'\t' read -r id text; do
      printf '#### %s\n\n' "$id"
      if [[ $text == LICENSES/* ]]; then
        emit_text "$text"
      else
        printf 'The text is `%s` under Components in this repository.\n\n' "$text"
      fi
    done
  fi

  printf '## Licence texts of the Rust crates\n\n'
  # Most-shared texts first, then by crate name, for a stable order.
  for users in "$work"/crates-of/*; do
    [[ -e $users ]] || continue
    printf '%s\t%s\t%s\n' "$(wc -l < "$users" | tr -d ' ')" "$(head -n 1 "$users")" "${users##*/}"
  done | LC_ALL=C sort -t "$(printf '\t')" -k1,1nr -k2,2 |
  while IFS=$'\t' read -r _ _ hash; do
    printf '### Used by\n\n'
    while IFS= read -r user; do
      printf -- '- %s\n' "$user"
    done < "$work/crates-of/$hash"
    printf '\n'
    emit_text "$work/texts/$hash"
  done
} > "$work/out.md"

if [[ $output == - ]]; then
  cat "$work/out.md"
else
  cp "$work/out.md" "$output"
  printf 'third-party-licenses: wrote %s (%s crates)\n' "$output" "$(wc -l < "$work/crates.tsv" | tr -d ' ')" >&2
fi
