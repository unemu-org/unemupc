#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Assemble the project-owned unbound LE DPMI client offline.
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
IMAGE="$WORK/dpmitest.exe"
OUT="$HERE/dpmitest.exe"

# LE header offsets are file-relative; the code is position independent.
nasm -f bin -Ox -w+all -Werror -o "$IMAGE" "$HERE/dpmitest.asm"

SIZE="$(wc -c < "$IMAGE" | tr -d '[:space:]')"
if [ "$SIZE" -ne 4336 ]; then
	echo "unexpected image size: $SIZE bytes (expected 4336, an LE header and one page)" >&2
	exit 1
fi
check_sha256 "$IMAGE" "91a5b15f1a47e7d5651663e36b66e56190d79b7fd553d775e60b9dd60113a105"
mv -f "$IMAGE" "$OUT"
echo "built $OUT ($SIZE bytes)"
