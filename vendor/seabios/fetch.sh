#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Build the pinned unemu SeaBIOS, standard-VGA and Bochs VBE ROMs; see README.md.
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
TAG="v1.17.0-unemu1"
COMMIT="ac8cabc4585f4922139a841b1c91cceb432b8728"
SOURCE_TREE="1da2ab6cd26a432bbc2beb998403f8b23120a13a"
SOURCE_SHA256="668d4e73d5664deeda5b695cee481f5fcd538559cb936a99583d20828a77c229"
SOURCE_ARCHIVE_SHA256="f94f8720fc0cc9eedc9681b100ecba0c0d6d30d26f7a0e599c0b86f9138dac0a"
CONFIG_SHA256="d1958e10e395c8eb00ea9e99ab1c6fe86cf1acb9a0480d0f7a1b551fa4fe8e2a"
OUTPUT_SHA256="9aace5de21df74ba42243664454d1f2130c143fc04ce5b4e69c0c71e55e9b699"
VGA_CONFIG_SHA256="370f78d9c30db41bd155d7b360fbf5cd0d6e26e390e19c106c14d32dbf5b5d40"
VGA_OUTPUT_SHA256="1adc6c0fa72b93c5d0659575c992b1bf6f64fd451c444b2a735e22f0a3d6e925"
VGA_BOCHS_CONFIG_SHA256="80d550bdb1ff17be47e8a6d2438fb633edd7ceac65b729312b4d93549d35cc50"
VGA_BOCHS_OUTPUT_SHA256="609be4bab79bf84297c50ef80c070147a2bd71ae1d457e8329b3feb5ec59b8c3"
VERSION="1.17.0"
EXTRAVERSION="-unemupc98-isa"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$HERE/src"
OUT="$HERE/bios-1.17.0-isa.bin"
SOURCE_NAME="seabios-${TAG#v}"
SOURCE_OUT="$HERE/build/$SOURCE_NAME-src.tar"
CROSS_PREFIX="${CROSS_PREFIX:-i686-elf-}"
HOSTCC="${HOSTCC:-cc}"
check_sha256 "$HERE/config" "$CONFIG_SHA256"
check_sha256 "$HERE/vgaconfig" "$VGA_CONFIG_SHA256"
check_sha256 "$HERE/vgaconfig-bochs" "$VGA_BOCHS_CONFIG_SHA256"

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
# The release asset keeps its rebuild recipe and licence notices in unemu/.
GIT_ALTERNATE_OBJECT_DIRECTORIES="$(git -C "$SRC_DIR" rev-parse --path-format=absolute --git-path objects)" \
	archive_git --git-dir="$BUILD_DIR/archive.git" archive --format=tar \
	--prefix="$SOURCE_NAME/" "$COMMIT" > "$BUILD_DIR/release-source.tar"
check_sha256 "$BUILD_DIR/release-source.tar" "$SOURCE_ARCHIVE_SHA256"
mkdir "$BUILD_DIR/tree"
# Match the release recipe: packaging files are not firmware build inputs.
tar --exclude=unemu -xf "$BUILD_DIR/source.tar" -C "$BUILD_DIR/tree"
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
verify_config() {
	# Fail if a dependency silently changes any requested Kconfig setting.
	while IFS= read -r setting; do
		case "$setting" in
			CONFIG_*=*)
				if ! grep -Fxq -- "$setting" "$1/.config"; then
					echo "Kconfig did not preserve requested setting: $setting" >&2
					exit 1
				fi
				;;
			'# CONFIG_'*' is not set')
				# Kconfig may omit options hidden by a disabled dependency.
				symbol="${setting#\# }"
				symbol="${symbol% is not set}"
				if grep -q -- "^$symbol=" "$1/.config"; then
					echo "Kconfig enabled an option requested off: $symbol" >&2
					exit 1
				fi
				;;
		esac
	done < "$2"
}
verify_config "$BUILD_DIR/tree" "$HERE/config"
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
if ! cmp -s "$IMAGE" "$OUT"; then
	mv "$IMAGE" "$OUT"
fi
echo "built $OUT ($SIZE bytes) from $COMMIT"

# Each VGA variant gets its own export, so the system BIOS and every video
# BIOS keep independent configurations and bytes.
build_vgabios() {
	local fragment="$1" variant="$2" output_sha="$3" expected_size="$4" disasm="$5"
	local tree="$BUILD_DIR/$6" image size
	mkdir "$tree"
	tar --exclude=unemu -xf "$BUILD_DIR/source.tar" -C "$tree"
	cp "$HERE/$fragment" "$tree/.config"
	printf '%s\n' "$VERSION" > "$tree/.version"
	MAKE_ARGS[1]="$tree"
	MAKE_ARGS[6]="EXTRAVERSION=-unemupc-$variant"
	make "${MAKE_ARGS[@]}" olddefconfig
	verify_config "$tree" "$HERE/$fragment"
	make "${MAKE_ARGS[@]}" out/vgabios.bin
	image="$tree/out/vgabios.bin"
	size="$(wc -c < "$image" | tr -d '[:space:]')"
	if [ "$size" -ne "$expected_size" ]; then
		echo "unexpected VGA BIOS size: $size bytes (expected $expected_size)" >&2
		exit 1
	fi
	if ! grep -Fxq -- "#define BUILD_VERSION \"$VERSION-unemupc-$variant\"" \
		"$tree/out/autovgaversion.h"; then
		echo "non-reproducible SeaVGABIOS version" >&2
		exit 1
	fi
	# Keep a symbol-aware 16-bit disassembly for the minimum-CPU audit.
	"${CROSS_PREFIX}objdump" -d -M i8086 "$tree/out/vgarom.o" > "$BUILD_DIR/$disasm"
	check_sha256 "$image" "$output_sha"
	# Install locally even when a worktree initially shares fixtures via symlinks.
	mv "$image" "$HERE/build/vgabios-$VERSION-$variant.bin"
	mv "$BUILD_DIR/$disasm" "$HERE/build/$disasm"
	echo "built $HERE/build/vgabios-$VERSION-$variant.bin ($size bytes) from $COMMIT"
}
build_vgabios vgaconfig stdvga "$VGA_OUTPUT_SHA256" 32768 vgabios.disasm vgatree
# The VBE mode tables and DISPI code need 38.5 KiB, past the 32 KiB of the
# standard-VGA build; the option-ROM header's length byte carries the size.
build_vgabios vgaconfig-bochs bochs "$VGA_BOCHS_OUTPUT_SHA256" 39424 vgabios-bochs.disasm vgatree-bochs
mv "$BUILD_DIR/release-source.tar" "$SOURCE_OUT"
# The release recipe rebuilds the system and standard-VGA ROMs. The Bochs ROM
# also needs this script's vgaconfig-bochs until a fork release carries it.
echo "wrote $SOURCE_OUT (source of all three ROMs, LGPL-3.0-only; the Bochs ROM also needs vgaconfig-bochs)"
