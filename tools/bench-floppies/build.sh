#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Build the bootable speed-ceiling floppies: 1.44 MB images that a PC BIOS
# boots from A:, which print a START line, run a fixed number of iterations of
# one ceiling loop and then print an END line and halt. The same image can be
# timed on unemupc and, with a stopwatch, on another emulator. See
# backlog/docs/design/bench-floppies.md.
#
# Usage: build.sh [output directory]
#   default: $TMPDIR/unemupc-bench-floppies (outside the checkout; the images
#   are generated and never committed)
# Needs nasm. Output: <case>.img for every case plus a SHA256SUMS file.
#
# BENCH_PASSES=<n> replaces every case's pass count. Only the self-test uses
# it, to boot a case quickly; the published images keep the counts below.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-${TMPDIR:-/tmp}/unemupc-bench-floppies}"
mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
if [[ -n ${BENCH_PASSES+set} ]]; then
  if [[ ! $BENCH_PASSES =~ ^[1-9][0-9]*$ ]]; then
    echo "build.sh: BENCH_PASSES must be a positive decimal number" >&2
    exit 2
  fi
  echo "build.sh: BENCH_PASSES=$BENCH_PASSES replaces every pass count; these are test images" >&2
fi

# name | source | flags | passes
# ceiling.asm counts passes of 65536 iterations; ceiling-user.asm counts loop
# passes (ten bodies per pass in the V86 cases). The counts give each case
# roughly half a minute on unemupc.
cases=(
  "real16|ceiling.asm||7500"
  "protected32|ceiling.asm|-DPROTECTED -DPROTECTED32|5400"
  "paged32|ceiling.asm|-DPROTECTED -DPROTECTED32 -DPAGED32|4000"
  "flat32|ceiling.asm|-DPROTECTED -DPROTECTED32 -DFLAT32|6000"
  "flat-paged32|ceiling.asm|-DPROTECTED -DPROTECTED32 -DFLAT32 -DPAGED32|4200"
  "flat-paged32-cpl3|ceiling-user.asm|-DFLAT -DPAGED|26000000"
  "flat-paged32-cpl3-farcall|ceiling-user.asm|-DFLAT -DPAGED -DFARCALL|24000000"
  "paged-protected16-cpl3|ceiling-user.asm|-DLDT -DPAGED|22000000"
  "paged-v86|ceiling-user.asm|-DV86 -DPAGED|2600000"
  "paged-v86-iopl0|ceiling-user.asm|-DV86 -DPAGED -DTRAPPED|2200000"
)

for entry in "${cases[@]}"; do
  IFS='|' read -r name source flags passes <<<"$entry"
  passes="${BENCH_PASSES:-$passes}"
  # shellcheck disable=SC2086 # flags is a list of separate options
  nasm -f bin -w+error -I "$here/" $flags "-DPASSES=$passes" "-DCASE_NAME='$name'" \
    "$here/$source" -o "$work/$name.bin"
  nasm -f bin -w+error "-DPAYLOAD='$work/$name.bin'" "$here/boot.asm" -o "$out/$name.img"
done

if command -v sha256sum >/dev/null; then
  sha256=(sha256sum)
else
  sha256=(shasum -a 256)
fi
(cd "$out" && "${sha256[@]}" ./*.img | sed 's| \./| |' >SHA256SUMS)
cat "$out/SHA256SUMS"
