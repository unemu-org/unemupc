#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build the offline FAT16 data-disk fixture described in README.md.
# IBM Personal Computer AT Technical Reference, BIOS listing, fixed-disk
# parameter table: type 2 has 615 cylinders, 4 heads and 17 sectors per track.
set -euo pipefail

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
		echo "Check the fixture contents and toolchain; deliberately review the image before updating the expected output hash." >&2
		exit 1
	fi
}

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$HERE/build"
WORK="$(mktemp -d "$HERE/build/image.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
IMAGE="$WORK/fat16.img"
OUT="$HERE/fat16.img"
SECTORS=$((615 * 4 * 17))
START=17
PARTITION_SECTORS=$((SECTORS - START))
OFFSET=$((START * 512))

# Fix every filesystem timestamp at the FAT epoch. Omitting mcopy -m keeps host
# file mtimes out of the image.
export MTOOLS_SKIP_CHECK=1
# In the C locale mtools adds VFAT long-name entries even for ASCII names
# and volume labels. UTF-8 keeps the directory bytes of the pinned image.
export LC_ALL=C.UTF-8 SOURCE_DATE_EPOCH=315532800 TZ=UTC
if [ "$(locale charmap 2>/dev/null)" != "UTF-8" ]; then
	echo "the C.UTF-8 locale is not available on this host; mtools would write long-name entries" >&2
	exit 1
fi
unset MTOOLSRC MTOOLS_DATE_STRING MTOOLS_NFATS MTOOLS_DIR_LEN
# mtools reads ~/.mtoolsrc through HOME even when MTOOLSRC is set (that only
# adds a further file), so mtools runs with an empty home directory. A
# system-wide mtools.conf, if the host has one, is still read.
mkdir "$WORK/home"
run_mtools() {
	HOME="$WORK/home" "$@"
}

dd if=/dev/zero of="$IMAGE" bs=512 count="$SECTORS" 2>/dev/null
# Write the MBR ourselves instead of mpartition so no boot code or tool-selected
# layout enters the fixture. Start CHS 0/1/1 = LBA 17; end 614/3/17 = LBA 41819.
# Cylinder 614 = 0x266: its top two bits occupy bits 6..7 of the sector byte.
# The MBR code stays zero: the partition is inactive and this disk never boots.
printf '\000\001\001\000\004\003\221\146\021\000\000\000\113\243\000\000' \
	| dd of="$IMAGE" bs=1 seek=446 conv=notrunc 2>/dev/null
printf '\125\252' | dd of="$IMAGE" bs=1 seek=510 conv=notrunc 2>/dev/null

# Microsoft FAT32 File System Specification, Boot Sector and BPB, BS_jmpBoot:
# a valid jump lets DOS recognize the BPB even on this data-only partition.
# Our own stub avoids embedding mtools' boot code. IBM Personal Computer AT
# Technical Reference, BIOS listing, INT 18h: no bootable disk; halt if it returns.
# The extended BPB enables serial and label.
dd if=/dev/zero of="$WORK/boot.bin" bs=512 count=1 2>/dev/null
printf '\353\074\220' | dd of="$WORK/boot.bin" bs=1 seek=0 conv=notrunc 2>/dev/null
printf 'UNEMUPC ' | dd of="$WORK/boot.bin" bs=1 seek=3 conv=notrunc 2>/dev/null
printf '\200\000\051' | dd of="$WORK/boot.bin" bs=1 seek=36 conv=notrunc 2>/dev/null
printf '\315\030\364\353\375' | dd of="$WORK/boot.bin" bs=1 seek=62 conv=notrunc 2>/dev/null
printf '\125\252' | dd of="$WORK/boot.bin" bs=1 seek=510 conv=notrunc 2>/dev/null
run_mtools mformat -i "$IMAGE@@$OFFSET" -T "$PARTITION_SECTORS" -h 4 -s 17 -H "$START" \
	-c 4 -R 1 -d 2 -r 32 -L 41 -m 0xf8 -N 0x28959854 -v UNEMUHDD \
	-B "$WORK/boot.bin" ::
printf 'unemupc98 hard disk fixture\r\n' > "$WORK/MARKER.TXT"
run_mtools mcopy -i "$IMAGE@@$OFFSET" "$WORK/MARKER.TXT" ::MARKER.TXT

# FAT type is determined by data-cluster count, not the BPB's text label.
read_le16() {
	local low high
	read -r low high <<< "$(od -An -tu1 -j "$((OFFSET + $1))" -N 2 "$IMAGE")"
	echo "$((low + high * 256))"
}
BYTES_PER_SECTOR="$(read_le16 11)"
CLUSTER_SECTORS="$(od -An -tu1 -j "$((OFFSET + 13))" -N 1 "$IMAGE" | tr -d '[:space:]')"
RESERVED="$(read_le16 14)"
FATS="$(od -An -tu1 -j "$((OFFSET + 16))" -N 1 "$IMAGE" | tr -d '[:space:]')"
ROOT_ENTRIES="$(read_le16 17)"
TOTAL="$(read_le16 19)"
FAT_SECTORS="$(read_le16 22)"
ROOT_SECTORS=$(((ROOT_ENTRIES * 32 + BYTES_PER_SECTOR - 1) / BYTES_PER_SECTOR))
CLUSTERS=$(((TOTAL - RESERVED - FATS * FAT_SECTORS - ROOT_SECTORS) / CLUSTER_SECTORS))
if [ "$CLUSTERS" -lt 4085 ] || [ "$CLUSTERS" -ge 65525 ]; then
	echo "expected FAT16: data-cluster count $CLUSTERS is outside 4085..65524" >&2
	exit 1
fi
SIZE="$(wc -c < "$IMAGE" | tr -d '[:space:]')"
if [ "$SIZE" -ne "$((SECTORS * 512))" ]; then
	echo "unexpected image size: $SIZE bytes (expected $((SECTORS * 512)))" >&2
	exit 1
fi
check_sha256 "$IMAGE" "e97f1eb42c1dcef1a2fbf9541135b22a955704836088c46bd280d27780e9ba2d"
mv -f "$IMAGE" "$OUT"
echo "built $OUT ($SIZE bytes, $CLUSTERS FAT16 data clusters)"
run_mtools mdir -i "$OUT@@$OFFSET" ::
