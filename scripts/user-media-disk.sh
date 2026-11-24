#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Build private data media outside the checkout; stdout is the temporary image.
set -euo pipefail
exec python3 -c '
import os
import pathlib
import re
import shutil
import struct
import subprocess
import tempfile


def build():
    source = pathlib.Path(os.environ["UNEMUPC_USER_DOOM_DIR"])
    files = sorted(source.iterdir(), key=lambda p: p.name.upper())
    names = [p.name.upper() for p in files]
    if not {"DOOM.EXE", "DOOM.WAD"}.issubset(names):
        raise ValueError("expected DOOM.EXE and DOOM.WAD (any case)")
    if len(names) != len(set(names)):
        raise ValueError("case-insensitive filename collision")
    if any(p.is_symlink() or not p.is_file() or
           not re.fullmatch(r"[A-Za-z0-9_!#$%&\x27()@^`{}~-]{1,8}(\.[A-Za-z0-9_!#$%&\x27()@^`{}~-]{1,3})?", p.name)
           for p in files):
        raise ValueError("expected regular files with DOS 8.3 names, without subdirectories")
    # ATA-2, IDENTIFY DEVICE words 1/3/6; Microsoft FAT Specification,
    # FAT type determination: use a FAT16 cluster count, including on growth.
    required = sum(((p.stat().st_size + 2047) // 2048) * 2048 for p in files)
    # A little over 32 MiB keeps 8 KiB clusters above the FAT16 threshold.
    cylinders = max(544, (required * 2 + 65535) // 65536)
    if cylinders > 8192 or len(files) > 510:
        raise ValueError("media exceeds the builder FAT16 capacity")
    sectors = cylinders * 4 * 32
    start = 32
    cluster = 16
    while (sectors - start) // cluster >= 65500:
        cluster *= 2
    work = pathlib.Path(tempfile.mkdtemp(prefix="unemupc-user-media-", dir="/tmp"))
    try:
        image = work / "data.img"
        with image.open("wb") as disk:
            disk.truncate(sectors * 512)
            # The inactive partition boots only through the FreeDOS floppy.
            end_c = min(cylinders - 1, 1023)
            entry = struct.pack("<8BII", 0, 1, 1, 0, 6, 3,
                                32 | ((end_c >> 8) << 6), end_c & 255,
                                start, sectors - start)
            disk.seek(446)
            disk.write(entry)
            disk.seek(510)
            disk.write(b"\x55\xaa")
        boot = bytearray(512)
        boot[:11] = b"\xeb\x3c\x90UNEMUPC "
        boot[36:39] = b"\x80\x00\x29"
        boot[62:67] = b"\xcd\x18\xf4\xeb\xfd"
        boot[510:] = b"\x55\xaa"
        (work / "boot.bin").write_bytes(boot)
        (work / "home").mkdir()
        env = dict(os.environ)
        for key in ("MTOOLSRC", "MTOOLS_DATE_STRING", "MTOOLS_NFATS", "MTOOLS_DIR_LEN"):
            env.pop(key, None)
        env.update(HOME=str(work / "home"), MTOOLS_SKIP_CHECK="1", LC_ALL="C.UTF-8",
                   SOURCE_DATE_EPOCH="315532800", TZ="UTC")
        def run(*args):
            subprocess.run(args, env=env, check=True, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL)
        target = str(image) + "@@16384"
        run("mformat", "-i", target, "-T", str(sectors - start), "-h", "4", "-s", "32",
            "-H", "32", "-c", str(cluster), "-R", "1", "-d", "2", "-r", "32",
            "-m", "0xf8", "-N", "0x12345678", "-v", "USERMEDIA", "-B", str(work / "boot.bin"), "::")
        for file in files:
            run("mcopy", "-i", target, str(file), "::" + file.name.upper())
        with image.open("rb") as disk:
            disk.seek(16384)
            bpb = disk.read(512)
        total = struct.unpack_from("<H", bpb, 19)[0] or struct.unpack_from("<I", bpb, 32)[0]
        reserved, = struct.unpack_from("<H", bpb, 14)
        root, = struct.unpack_from("<H", bpb, 17)
        fat, = struct.unpack_from("<H", bpb, 22)
        clusters = (total - reserved - bpb[16] * fat - (root * 32 + 511) // 512) // bpb[13]
        if not 4085 <= clusters < 65525:
            raise ValueError("builder did not produce FAT16")
        (work / "boot.bin").unlink()
        (work / "home").rmdir()
        print(image)
    except BaseException:
        shutil.rmtree(work)
        raise

try:
    build()
except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
    # Tool errors can contain private filenames; never echo their arguments.
    print("user-media disk build failed: " + (str(error) if isinstance(error, ValueError)
          else type(error).__name__), file=__import__("sys").stderr)
    raise SystemExit(1)
'
