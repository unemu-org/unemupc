#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Assemble the open VGA Mode X test floppy described in README.md from
# its committed source. Offline: no downloads and no cached inputs. The image
# is gitignored; its SHA-256 is pinned here.
#
# Requires nasm (the flake devShell provides the pinned version).
set -euo pipefail

if ! command -v nasm >/dev/null 2>&1; then
	echo "missing nasm: run this script inside nix develop" >&2
	exit 1
fi

# Both GNU/Linux and macOS provide one of these SHA-256 tools.
if command -v sha256sum >/dev/null 2>&1; then
	SHA256=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
	SHA256=(shasum -a 256)
else
	echo "missing SHA-256 tool: install sha256sum or shasum (or use nix develop)" >&2
	exit 1
fi

check_sha256() {
	local file="$1" expected="$2" actual
	actual="$("${SHA256[@]}" "$file" | awk '{print $1}')"
	if [ "$actual" != "$expected" ]; then
		echo "SHA-256 mismatch for $file: expected $expected, got $actual" >&2
		echo "Check the program source and the nasm version; deliberately re-measure the goldens before updating the expected output hash." >&2
		exit 1
	fi
}

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$HERE/build"
WORK="$(mktemp -d "$HERE/build/image.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
IMAGE="$WORK/vga-modex-test-1200.img"
OUT="$HERE/vga-modex-test-1200.img"

# A flat binary carries no timestamps or host data. Multi-section bin output
# resolves every absolute label as a relocation; that warning class is noise.
nasm -f bin -Ox -w+all -w-reloc-abs-word -Werror -o "$IMAGE" "$HERE/vga-modex-test.asm"

SIZE="$(wc -c < "$IMAGE" | tr -d '[:space:]')"
if [ "$SIZE" -ne 1228800 ]; then
	echo "unexpected image size: $SIZE bytes (expected 1228800, a 1.2M floppy)" >&2
	exit 1
fi
# IBM Personal Computer AT Technical Reference, BIOS listing, bootstrap
# loader: INT 19h reads sector 1 of drive A: to 0000:7C00h. 55AAh marks it.
SIGNATURE="$(od -An -tx1 -j 510 -N 2 "$IMAGE" | tr -d ' \n')"
if [ "$SIGNATURE" != "55aa" ]; then
	echo "missing boot signature 55aa (found $SIGNATURE)" >&2
	exit 1
fi
check_sha256 "$IMAGE" "834038855a9cc34af9b55ca288b086786b37d76209139180a77ba69572b37bc0"
mv -f "$IMAGE" "$OUT"
echo "built $OUT ($SIZE bytes)"

PAS_IMAGE="$WORK/vga-pas-test-1200.img"
PAS_OUT="$HERE/vga-pas-test-1200.img"
nasm -f bin -Ox -w+all -w-reloc-abs-word -Werror -o "$PAS_IMAGE" "$HERE/vga-pas-test.asm"
SIZE="$(wc -c < "$PAS_IMAGE" | tr -d '[:space:]')"
if [ "$SIZE" -ne 1228800 ]; then
    echo "unexpected palette image size: $SIZE bytes (expected 1228800)" >&2
    exit 1
fi
SIGNATURE="$(od -An -tx1 -j 510 -N 2 "$PAS_IMAGE" | tr -d ' \n')"
if [ "$SIGNATURE" != "55aa" ]; then
    echo "missing palette image boot signature 55aa (found $SIGNATURE)" >&2
    exit 1
fi
check_sha256 "$PAS_IMAGE" "d58e531d4a29e0232484f76f26d4504fee8e9df837b251343fae867ec1286490"
mv -f "$PAS_IMAGE" "$PAS_OUT"
echo "built $PAS_OUT ($SIZE bytes)"
