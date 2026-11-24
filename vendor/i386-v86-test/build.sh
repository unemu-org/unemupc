#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Assemble the project-owned 128 KiB 80386 firmware ROM offline.
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
IMAGE="$WORK/i386-v86-test.bin"
OUT="$HERE/i386-v86-test.bin"

# Flat-binary absolute relocations resolve to the fixed shadow addresses.
nasm -f bin -Ox -w+all -w-reloc-abs-dword -Werror -o "$IMAGE" "$HERE/i386-v86-test.asm"

SIZE="$(wc -c < "$IMAGE" | tr -d '[:space:]')"
if [ "$SIZE" -ne 131072 ]; then
	echo "unexpected image size: $SIZE bytes (expected 131072, a 128 KiB ROM)" >&2
	exit 1
fi
check_sha256 "$IMAGE" "c21066bfcd0a1d5a39458de7f0c8b656a578d16dd4db97e1672f7755220e348b"
mv -f "$IMAGE" "$OUT"
echo "built $OUT ($SIZE bytes)"
