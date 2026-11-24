#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Fetch and build test386.bin, a third-party 386 CPU-test ROM used here as
# read-only golden test data (see README.md). The image is not committed — it is
# rebuilt from a pinned upstream commit so the smoke test is reproducible.
#
# Requires `git` and `nasm` on PATH (both are in the flake devShell).
set -euo pipefail

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
		echo "Check the pinned sources and toolchain; deliberately rebuild the goldens' expectations before updating the expected output hash." >&2
		exit 1
	fi
}

# Pinned upstream revision. test386.bin is only valid golden data against a known
# source revision, so this is hard-coded rather than tracking a moving branch.
REPO="https://github.com/barotto/test386.asm"
COMMIT="cfd052d1e64d5375dea5a681c1eadeed64ceda2c"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$HERE/test386.asm"        # gitignored clone
OUT="$HERE/test386.bin"            # gitignored build product

if [ ! -d "$SRC_DIR/.git" ]; then
	git clone "$REPO" "$SRC_DIR"
fi
# A cached commit is sufficient for offline builds; fetch only missing objects.
if ! git -C "$SRC_DIR" cat-file -e "$COMMIT^{commit}" 2>/dev/null; then
	git -C "$SRC_DIR" fetch --depth 1 origin "$COMMIT" 2>/dev/null \
		|| git -C "$SRC_DIR" fetch origin
fi
if [ "$(git -C "$SRC_DIR" rev-parse HEAD)" != "$COMMIT" ]; then
	git -C "$SRC_DIR" checkout --quiet "$COMMIT"
fi

# Shadow only the configuration include; never edit the cached upstream tree.
# Both generic ISA boards already decode these diagnostic ports.
build_dir="$(mktemp -d)"
trap 'rm -rf "$build_dir"' EXIT
sed -e 's/^POST_PORT equ 0x190$/POST_PORT equ 0x80/' \
    -e 's/^OUT_PORT equ 0$/OUT_PORT equ 0x402/' \
    "$SRC_DIR/src/configuration.asm" > "$build_dir/configuration.asm"
nasm -i"$build_dir/" -i"$SRC_DIR/src/" -f bin "$SRC_DIR/src/test386.asm" \
    -w-all -o "$build_dir/test386.bin"
# Replace a local fixture symlink rather than modifying another checkout's image.
rm -f "$OUT"
cp "$build_dir/test386.bin" "$OUT"

SIZE="$(wc -c < "$OUT" | tr -d '[:space:]')"
if [ "$SIZE" -ne 65536 ]; then
	echo "unexpected test386.bin size: $SIZE bytes (expected 65536)" >&2
	exit 1
fi
check_sha256 "$OUT" "9cb8dddc091a73a8b7b94e7529671318a957dc645fc152be8dcbd5bb734d057c"
echo "built $OUT ($SIZE bytes) from $COMMIT"
