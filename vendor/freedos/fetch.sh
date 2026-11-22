#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build bootable FreeDOS boot and tools floppy images used here
# as read-only boot fixtures (see README.md). Nothing is committed — the archives
# are fetched from pinned FreeDOS releases and the image is rebuilt from them, so
# the boot is reproducible against known kernel / shell / boot-sector revisions.
#
# All images are FAT12 with a FreeDOS FAT12 boot sector, the 8086
# KERNEL.SYS, and the 8086 FreeCOM COMMAND.COM in the root. Kernel/SYS and
# FreeCOM are GPL v2 or later; the six utility packages are GPL v2 (see README.md).
# This project reads the images without linking their code.
#
# Requires `git`, `nasm`, `unzip`, `curl`, and mtools (`mformat`/`mcopy`/`mdir`/`mlabel`)
# on PATH. Run it inside `nix develop`: the flake devShell provides all of them
# and the pinned CA roots that the downloads are verified against.
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
		if [ "${3:-output}" = archive ]; then
			echo "Remove the cached archive and fetch the pinned release again; do not change its expected hash." >&2
		else
			echo "Check the pinned sources and toolchain; deliberately rebuild the goldens' expectations before updating the expected output hash." >&2
		fi
		exit 1
	fi
}

# Pinned upstream revisions. A boot is only reproducible against known versions,
# so these are hard-coded rather than tracking moving branches.
#
# 8086 kernel build (FAT12/16, no FAT32, real-mode 8086 — the default installer
# kernel is 386-only and would fault on our real-mode core).
KERNEL_TAG="ke2044"
KERNEL_ZIP="ke2044_86f16.zip"
KERNEL_ZIP_SHA256="367c170c14412fc256b118fcee4906c2ae724c35f28d1496fa6beed1d78237eb"
# 8086 FreeCOM shell build.
FREECOM_TAG="com086"
FREECOM_ZIP="command.zip"
FREECOM_ZIP_SHA256="b0cbfc65f58a2f0f2b88f172bf2a99a306b761ed5b10edcc668f9f047cf42f68"
# Kernel source at the ke2044 tag, for assembling the FAT12 boot sector from
# boot/boot.asm (the release archives ship SYS.COM but no standalone boot binary).
KERNEL_REPO="https://github.com/FDOS/kernel"
KERNEL_COMMIT="8535ddb7a783f94b3aae844c8fe7f5ccce9037eb"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$HERE/build"                     # gitignored scratch (downloads + clone)
SRC_DIR="$WORK/kernel-src"             # gitignored clone (boot-sector source)
OUT="$HERE/freedos.img"                # gitignored 1.44 MB build product
OUT_1200="$HERE/freedos-1200.img"      # gitignored 1.2 MB build product
OUT_TOOLS="$HERE/freedos-tools-1200.img" # gitignored installation floppy

mkdir -p "$WORK"

fetch_release() {
	# $1 repo (kernel|freecom), $2 tag, $3 asset, $4 sha256
	local dest="$WORK/$3"
	if [ ! -f "$dest" ]; then
		curl -fsSL -o "$dest" \
			"https://github.com/FDOS/${1}/releases/download/${2}/${3}"
	fi
	check_sha256 "$dest" "$4" archive
}

fetch_package() {
	local name="$1" expected="$2" dest="$WORK/$1.zip"
	if [ -n "${FREEDOS_PACKAGE_DIR:-}" ]; then
		check_sha256 "$FREEDOS_PACKAGE_DIR/$name.zip" "$expected" archive
		cp "$FREEDOS_PACKAGE_DIR/$name.zip" "$dest"
	elif [ ! -f "$dest" ]; then
		curl -fsSL -o "$dest" \
			"https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/repositories/1.3/base/$name.zip"
	fi
	check_sha256 "$dest" "$expected" archive
}

build_tools_floppy() {
	fetch_package fdisk 5f6f84fab35132cc7928137e0f350660cb7ff06d23da18e1b5caceed15ea7b8f
	fetch_package format 9bd13d1cfa2084020bf37e8e6b61a05e8413629cf6d7f2d2779819e7b1f32bd4
	fetch_package xcopy 33c43434964d2ae08cf868d5ea54760300588c9c2ea5e50e7936cecebf60fe52
	fetch_package edit 582729981a967493f59f38b7d4464c1b1a6caa25f71c0a2fbf6dd9e3b782037b
	fetch_package mem 4090eb3b08a283cf3fc974c72ab6eb5105bf2b5f27e6bd8dfe6682bd0c3605d1
	fetch_package ctmouse a891124cd5b13e8141778fcae718f3b2667b0a49727ce92b782ab11a8c4bb63a
	mkdir -p "$WORK/tools"
	local name upper
	for name in fdisk format xcopy edit mem ctmouse; do
		upper="$(echo "$name" | tr '[:lower:]' '[:upper:]')"
		unzip -o -j "$WORK/$name.zip" "BIN/$upper.EXE" -d "$WORK/tools" >/dev/null
	done
	unzip -o -j "$WORK/edit.zip" BIN/EDIT.HLP -d "$WORK/tools" >/dev/null
	unzip -o -j "$WORK/$KERNEL_ZIP" bin/sys.com -d "$WORK/tools" >/dev/null
	printf '%s\r\n' \
		"FreeDOS installation tools - thank you to the FreeDOS developers!" \
		"KERNEL.SYS and SYS.COM 2044 (8086 FAT12/16), COMMAND.COM FreeCOM 0.86:" \
		"GNU GPL version 2 or later." \
		"FDISK.EXE 1.3.4a, FORMAT.EXE 0.92, XCOPY.EXE 1.7, EDIT.EXE 0.9b," \
		"MEM.EXE 1.12, CTMOUSE.EXE 2.1b4: GNU GPL version 2 (see upstream archives)." \
		"EDIT.HLP is the editor's help file, under the same licence as EDIT." \
		"FDISK requires 186/286 or later; CTMOUSE requires 286 or later, not 8086." \
		"See the repository's vendor/freedos/README.md for sources and licences." > "$WORK/tools/README.TXT"
	# The verified boot fixture also permits a fully offline tools-only build.
	# Its hash binds the shell, kernel, loader and all FAT timestamps together.
	local boot_image="$OUT_1200"
	check_sha256 "$boot_image" "4ee55e9c44d553041b9bbde99cacae31472f12debf6c9b9c09c5b9343c435d7e"
	rm -f "$OUT_TOOLS"
	cp "$OUT_1200" "$OUT_TOOLS"
	mlabel -i "$OUT_TOOLS" -N 0x28959854 ::FDTOOLS
	for name in FDISK.EXE FORMAT.EXE sys.com XCOPY.EXE EDIT.EXE EDIT.HLP MEM.EXE CTMOUSE.EXE README.TXT; do
		upper="$(echo "$name" | tr '[:lower:]' '[:upper:]')"
		mcopy -i "$OUT_TOOLS" "$WORK/tools/$name" "::$upper"
	done
	check_sha256 "$OUT_TOOLS" "8d91283a1c0d6fc5b5fdb5e40db5b66dd9fabbf014d37262b7892f3b150b6f63"
	mdir -i "$OUT_TOOLS" ::
}

# In the C locale mtools adds VFAT long-name entries even for ASCII names
# and volume labels. UTF-8 keeps the directory bytes of the pinned images.
export LC_ALL=C.UTF-8 MTOOLS_SKIP_CHECK=1 SOURCE_DATE_EPOCH=315532800 TZ=UTC
if [ "$(locale charmap 2>/dev/null)" != "UTF-8" ]; then
	echo "the C.UTF-8 locale is not available on this host; mtools would write long-name entries" >&2
	exit 1
fi
# Offline mode deliberately requires the already pinned boot fixture; it never
# attempts a source clone or download. All supplied archives remain hash-checked.
if [ -n "${FREEDOS_PACKAGE_DIR:-}" ]; then
	check_sha256 "$FREEDOS_PACKAGE_DIR/$KERNEL_ZIP" "$KERNEL_ZIP_SHA256" archive
	cp "$FREEDOS_PACKAGE_DIR/$KERNEL_ZIP" "$WORK/$KERNEL_ZIP"
	check_sha256 "$WORK/$KERNEL_ZIP" "$KERNEL_ZIP_SHA256" archive
	build_tools_floppy
	exit 0
fi

fetch_release "kernel" "$KERNEL_TAG" "$KERNEL_ZIP" "$KERNEL_ZIP_SHA256"
fetch_release "freecom" "$FREECOM_TAG" "$FREECOM_ZIP" "$FREECOM_ZIP_SHA256"

# --- assemble the FAT12 boot sector from pinned kernel source ---------------
if [ ! -d "$SRC_DIR/.git" ]; then
	git clone "$KERNEL_REPO" "$SRC_DIR"
fi
# A cached commit is sufficient for offline builds; fetch only missing objects.
if ! git -C "$SRC_DIR" cat-file -e "$KERNEL_COMMIT^{commit}" 2>/dev/null; then
	git -C "$SRC_DIR" fetch --depth 1 origin "$KERNEL_COMMIT" 2>/dev/null \
		|| git -C "$SRC_DIR" fetch origin
fi
if [ "$(git -C "$SRC_DIR" rev-parse HEAD)" != "$KERNEL_COMMIT" ]; then
	git -C "$SRC_DIR" checkout --quiet "$KERNEL_COMMIT"
fi
# boot.asm selects FAT12 with -dISFAT12; it includes only the local magic.mac and
# has its own `org`, so it assembles standalone into a 512-byte sector image.
nasm -dISFAT12 -f bin -I"$SRC_DIR/boot/" "$SRC_DIR/boot/boot.asm" -o "$WORK/fat12com.bin"
BOOT_SIZE="$(wc -c < "$WORK/fat12com.bin" | tr -d '[:space:]')"
if [ "$BOOT_SIZE" -ne 512 ]; then
	echo "unexpected boot-sector size: $BOOT_SIZE bytes (expected 512)" >&2
	exit 1
fi

# --- lay down the FAT12 filesystem ------------------------------------------
unzip -o -j "$WORK/$KERNEL_ZIP" bin/kernel.sys -d "$WORK/extract" >/dev/null
unzip -o -j "$WORK/$FREECOM_ZIP" BIN/COMMAND.COM -d "$WORK/extract" >/dev/null

export MTOOLS_SKIP_CHECK=1
# mtools uses this epoch for volume-label, creation, access and modification
# timestamps. Do not use mcopy -m: it would preserve archive/host file mtimes.
# UTC avoids host timezone differences; the FAT epoch is representable everywhere.
export SOURCE_DATE_EPOCH=315532800 TZ=UTC
build_floppy() {
	local output="$1" format="$2" sectors="$3" size signature
	rm -f "$output"
	dd if=/dev/zero of="$output" bs=512 count="$sectors" 2>/dev/null
	mformat -i "$output" -f "$format" -N 0x28959853 -v FREEDOS ::
	mcopy -i "$output" "$WORK/extract/kernel.sys" ::KERNEL.SYS
	mcopy -i "$output" "$WORK/extract/COMMAND.COM" ::COMMAND.COM

	# Preserve the formatted geometry in the BPB (0x0B..0x3D). Only the
	# FreeDOS loader (0x3E..0x1FD) replaces mformat's boot stub; the formatted
	# 0x55AA signature remains in place for either floppy geometry.
	dd if="$WORK/fat12com.bin" of="$output" bs=1 skip=62 seek=62 count=448 conv=notrunc 2>/dev/null

	size="$(wc -c < "$output" | tr -d '[:space:]')"
	if [ "$size" -ne "$((sectors * 512))" ]; then
		echo "unexpected image size: $size bytes (expected $((sectors * 512)))" >&2
		exit 1
	fi
	signature="$(dd if="$output" bs=1 skip=510 count=2 2>/dev/null | od -An -tx1 | tr -d ' \n')"
	if [ "$signature" != "55aa" ]; then
		echo "boot signature missing: got 0x$signature (expected 0x55aa)" >&2
		exit 1
	fi
	echo "built $output ($size bytes), boot signature 0x$signature"
	mdir -i "$output" ::
}

build_floppy "$OUT" 1440 2880
check_sha256 "$OUT" "b7b088333f7f30584d768941fecea8288b63f2d61e13ec6c9472439eda66e51f"
build_floppy "$OUT_1200" 1200 2400
check_sha256 "$OUT_1200" "4ee55e9c44d553041b9bbde99cacae31472f12debf6c9b9c09c5b9343c435d7e"

build_tools_floppy
