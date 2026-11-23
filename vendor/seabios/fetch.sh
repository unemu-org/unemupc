#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Build the pinned unemu SeaBIOS fork with the ISA configuration; see README.md.
# Run from the repository root: nix develop .#firmware --command bash vendor/seabios/fetch.sh
# Requires an i686-elf GCC/binutils cross toolchain, native C compiler, Python 3,
# GNU make, git, tar, SHA-256 and standard shell tools. ACPI is off: no IASL.
set -euo pipefail
export LC_ALL=C

if command -v sha256sum >/dev/null 2>&1; then
	SHA256=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
	SHA256=(shasum -a 256)
else
	echo "missing SHA-256 tool: use nix develop .#firmware" >&2
	exit 1
fi

check_sha256() {
	local file="$1" expected="$2" actual
	actual="$("${SHA256[@]}" "$file" | awk '{print $1}')"
	if [ "$actual" != "$expected" ]; then
		echo "SHA-256 mismatch for $file: expected $expected, got $actual" >&2
		echo "Review the pinned inputs and toolchain before deliberately updating the expected hash." >&2
		exit 1
	fi
}

REPO="https://github.com/unemu-org/seabios"
COMMIT="7da2116237ed8916970795b0d8cae5bf6e277d0a"
SOURCE_TREE="81259d7b00535b6a9dd531d60cd907f4d9218c67"
SOURCE_SHA256="f909c5f540c976ac6d323a51658350710f88b2a67afa8b832998dc126c517c94"
CONFIG_SHA256="d1958e10e395c8eb00ea9e99ab1c6fe86cf1acb9a0480d0f7a1b551fa4fe8e2a"
OUTPUT_SHA256="ad50fdb5e12a03dd5c17eac481a4183a7793b84252319eea02b8c878667c994f"
VERSION="1.17.0"
EXTRAVERSION="-unemupc98-isa"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$HERE/src"
OUT="$HERE/bios-1.17.0-isa.bin"
CROSS_PREFIX="${CROSS_PREFIX:-i686-elf-}"
HOSTCC="${HOSTCC:-cc}"
check_sha256 "$HERE/config" "$CONFIG_SHA256"

if [ ! -d "$SRC_DIR/.git" ]; then
	git clone "$REPO" "$SRC_DIR"
fi
# A cached commit needs no network and its working tree can stay untouched.
if ! git -C "$SRC_DIR" cat-file -e "$COMMIT^{commit}" 2>/dev/null; then
	git -C "$SRC_DIR" fetch --depth 1 origin "$COMMIT"
fi
ACTUAL_TREE="$(git -C "$SRC_DIR" rev-parse "$COMMIT^{tree}")"
if [ "$ACTUAL_TREE" != "$SOURCE_TREE" ]; then
	echo "source tree mismatch: expected $SOURCE_TREE, got $ACTUAL_TREE" >&2
	exit 1
fi
mkdir -p "$HERE/build"
BUILD_DIR="$(mktemp -d "$HERE/build/seabios.XXXXXX")"
trap 'rm -rf "$BUILD_DIR"' EXIT
# Cached-repository attributes and user filters must not change exported bytes.
archive_git() {
	GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_ATTR_NOSYSTEM=1 \
		git -c core.autocrlf=false -c core.eol=lf \
		-c core.attributesFile=/dev/null -c tar.umask=0002 "$@"
}
archive_git init --bare --quiet --template= "$BUILD_DIR/archive.git"
# Share only objects; an empty repository excludes the cache's info/attributes.
GIT_ALTERNATE_OBJECT_DIRECTORIES="$(git -C "$SRC_DIR" rev-parse --path-format=absolute --git-path objects)" \
	archive_git --git-dir="$BUILD_DIR/archive.git" archive --format=tar "$COMMIT" > "$BUILD_DIR/source.tar"
check_sha256 "$BUILD_DIR/source.tar" "$SOURCE_SHA256"
mkdir "$BUILD_DIR/tree"
tar -xf "$BUILD_DIR/source.tar" -C "$BUILD_DIR/tree"
cp "$HERE/config" "$BUILD_DIR/tree/.config"
# Use upstream's release-tarball mechanism plus a nonempty distributor suffix.
# Without both, buildversion.py appends the build time and hostname.
printf '%s\n' "$VERSION" > "$BUILD_DIR/tree/.version"
MAKE_ARGS=(
	-C "$BUILD_DIR/tree"
	"CROSS_PREFIX=$CROSS_PREFIX"
	"HOSTCC=$HOSTCC"
	"CPP=${CROSS_PREFIX}cpp"
	PYTHON=python3
	"EXTRAVERSION=$EXTRAVERSION"
)
make "${MAKE_ARGS[@]}" olddefconfig
# Fail if a dependency silently changes any requested Kconfig setting.
while IFS= read -r setting; do
	case "$setting" in
		CONFIG_*=*)
			if ! grep -Fxq -- "$setting" "$BUILD_DIR/tree/.config"; then
				echo "Kconfig did not preserve requested setting: $setting" >&2
				exit 1
			fi
			;;
		'# CONFIG_'*' is not set')
			# Kconfig may omit options hidden by a disabled dependency.
			symbol="${setting#\# }"
			symbol="${symbol% is not set}"
			if grep -q -- "^$symbol=" "$BUILD_DIR/tree/.config"; then
				echo "Kconfig enabled an option requested off: $symbol" >&2
				exit 1
			fi
			;;
	esac
done < "$HERE/config"
make "${MAKE_ARGS[@]}"

IMAGE="$BUILD_DIR/tree/out/bios.bin"
SIZE="$(wc -c < "$IMAGE" | tr -d '[:space:]')"
if [ "$SIZE" -ne 131072 ]; then
	echo "unexpected BIOS size: $SIZE bytes (expected 131072)" >&2
	exit 1
fi
# A mismatched tool-version probe can still make upstream mark a build dirty.
if ! grep -Fxq -- "#define BUILD_VERSION \"$VERSION$EXTRAVERSION\"" \
	"$BUILD_DIR/tree/out/autoversion.h"; then
	echo "non-reproducible SeaBIOS version: expected the fixed release and distributor suffix" >&2
	exit 1
fi
# The exact version is the primary guard. Match the upstream dirty-build form
# too, so short or common hostnames do not reject unrelated firmware strings.
if ! python3 - "$IMAGE" "$EXTRAVERSION" <<'PY'
import re
import socket
import sys
pattern = b"-[0-9]{8}_[0-9]{6}-" + re.escape(
    (socket.gethostname() + sys.argv[2]).encode())
with open(sys.argv[1], "rb") as image:
    sys.exit(1 if re.search(pattern, image.read()) else 0)
PY
then
	echo "BIOS hostname check failed; refusing to install the image" >&2
	exit 1
fi
check_sha256 "$IMAGE" "$OUTPUT_SHA256"
cp "$IMAGE" "$OUT"
echo "built $OUT ($SIZE bytes) from $COMMIT"
