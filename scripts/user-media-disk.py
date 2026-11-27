#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Private FAT16 media; Microsoft FAT Specification, BPB and FAT type determination.

See backlog/docs/user-media.md §Temporary disk, §Windows 95 and §Windows 3.1.
"""
import argparse
import math
import os
import pathlib
import re
import shutil
import struct
import subprocess
import sys
import tempfile

NAME = re.compile(r"[A-Za-z0-9_!#$%&()@^`{}~-]{1,8}(\.[A-Za-z0-9_!#$%&()@^`{}~-]{1,3})?")

RESERVED = {"CON", "PRN", "AUX", "NUL"} | {
    prefix + str(number) for prefix in ("COM", "LPT") for number in range(1, 10)
}


def short_name(value):
    # GNU mtools, VFAT-style long file names: avoid renaming and LFN fallback.
    return NAME.fullmatch(value) and value.split(".")[0].upper() not in RESERVED


def dos_path(value):
    parts = value.replace("\\", "/").strip("/").split("/") if value.strip("/\\") else []
    if any(not short_name(part) for part in parts):
        raise ValueError("expected a DOS 8.3 target path")
    return tuple(part.upper() for part in parts)


def msbatch(key):
    # Windows 95 Resource Kit, Appendix D, Setup/NameAndOrg/Network/System.
    # Device descriptions and OSR2 ProductKey support are hypotheses until run.
    if key is not None and not re.fullmatch(r"[A-Za-z0-9-]{1,64}", key):
        raise ValueError("invalid product key format")
    lines = ["[Setup]", "Express=1", r"InstallDir=C:\WINDOWS", "EBD=0",
             'TimeZone="GMT"', "Verify=0", "PenWinWarning=0"]
    if key:
        lines.append("ProductKey=" + key)
    lines += ["", "[NameAndOrg]", 'Name="User"', 'Org="Local test"', "",
              "[Network]", "Display=0", "Clients=", "Protocols=", "",
              "[System]", 'Display="Standard Display Adapter (VGA)"',
              'Monitor="Standard VGA 640x480"', 'Mouse="Standard PS/2 Port Mouse"', ""]
    return "\r\n".join(lines).encode("ascii")


def win31_settings():
    # Windows 3.1 Resource Kit, Chapter 1, Creating the System Settings File.
    # Keep the request explicit; the guest run must establish which choices stick.
    return ("[sysinfo]\r\nshowsysinfo=no\r\n[configuration]\r\nmachine=ibm_compatible\r\n"
            "display=vga\r\nmouse=ps2mouse\r\nkeyboard=t4s0enha\r\nnetwork=nonet\r\n"
            "[windir]\r\nC:\\WINDOWS\r\n[userinfo]\r\nUser\r\n\"Local test\"\r\n"
            "[dontinstall]\r\ngames\r\n[options]\r\n[endinstall]\r\nconfigfiles=modify\r\nendopt=exit\r\n").encode("ascii")


def win31_files():
    return {
        "SETUP.SHH": win31_settings(),
        "SYS31.BAT": ("@ECHO OFF\r\nSYS C:\r\nIF ERRORLEVEL 1 GOTO END\r\n"
                      "IF NOT EXIST C:\\COMMAND.COM COPY A:\\COMMAND.COM C:\\ >NUL\r\n"
                      "IF NOT EXIST C:\\IO.SYS GOTO END\r\n"
                      "IF NOT EXIST C:\\COMMAND.COM GOTO END\r\n"
                      # Setup rewrites the boot drive's startup files, so the
                      # install stage boots C:, which needs an XMS driver before
                      # Windows exists. Windows' own expander and driver serve
                      # every DOS boot floppy alike.
                      "C:\\WIN31SRC\\EXPAND C:\\WIN31SRC\\HIMEM.SY_ C:\\HIMEM.SYS >NUL\r\n"
                      "IF NOT EXIST C:\\HIMEM.SYS GOTO END\r\n"
                      # Only DOS 7 keeps startup options as text in MSDOS.SYS;
                      # MS-DOS 6 keeps part of its kernel there, so it is never
                      # rewritten below version 7.
                      "C:\\DOSVER.COM\r\nIF NOT ERRORLEVEL 7 GOTO DONE\r\n"
                      "ATTRIB -R -S -H C:\\MSDOS.SYS >NUL\r\n"
                      "ECHO [Options]>C:\\MSDOS.SYS\r\n"
                      "ECHO BootGUI=0>>C:\\MSDOS.SYS\r\n"
                      "ECHO Logo=0>>C:\\MSDOS.SYS\r\n"
                      "ECHO BootDelay=0>>C:\\MSDOS.SYS\r\n"
                      "ECHO UM31DOS7\r\n"
                      ":DONE\r\nECHO UM31SYSOK\r\n:END\r\n").encode("ascii"),
        # Our program: INT 21h AH=30h returns the DOS major version in AL, and
        # INT 21h AH=4Ch exits with it as the ERRORLEVEL.
        "DOSVER.COM": bytes.fromhex("b430cd21b44ccd21"),
        "INST31.BAT": ("@ECHO OFF\r\nC:\r\nCD \\WIN31SRC\r\n"
                       "SETUP /H:C:\\SETUP.SHH\r\nIF ERRORLEVEL 1 GOTO END\r\n"
                       "IF NOT EXIST C:\\WINDOWS\\SYSTEM\\WIN386.EXE GOTO END\r\n"
                       "IF NOT EXIST C:\\WINDOWS\\SYSTEM.INI GOTO END\r\n"
                       "ECHO UM31INSTALLED\r\n:END\r\n").encode("ascii"),
        "CONFIG.SYS": b"DEVICE=C:\\HIMEM.SYS\r\nDOS=HIGH\r\n",
        "AUTOEXEC.BAT": b"@ECHO OFF\r\nPATH C:\\WINDOWS;C:\\WINDOWS\\SYSTEM;C:\\\r\n",
    }


def win31_mbr():
    # Our CHS chain loader relocates all 512 bytes from 7C00h to 0600h,
    # preserving DL, then INT 13h/AH=02h loads the first partition's VBR.
    # SYS installs only the DOS VBR; leaving the MBR empty cannot boot C:.
    return bytes.fromhex(
        "fa31c08ed88ec08ed0bc007cfcbe007cbf0006b90001f3a5ea1d060000fb"
        "bebe078a74018b4c02bb007cb80102cd137205ea007c0000faf4ebfc")


def build(args):
    doom = args.source is None and not (args.win95 or args.win31)
    source = pathlib.Path(args.source or os.environ[
        "UNEMUPC_USER_WIN95_DIR" if args.win95 else
        "UNEMUPC_USER_WIN31_DIR" if args.win31 else "UNEMUPC_USER_DOOM_DIR"])
    if source.is_symlink() or not source.is_dir():
        raise ValueError("expected a regular source directory")
    target = dos_path(args.target if args.target is not None else ("WIN95" if args.win95 else "WIN31SRC" if args.win31 else ""))
    entries = {}

    def add(path, value):
        if path in entries:
            raise ValueError("case-insensitive filename collision")
        entries[path] = value

    def parents(path):
        for length in range(1, len(path) + 1):
            parent = path[:length]
            if parent in entries and entries[parent] is not None:
                raise ValueError("file conflicts with target directory")
            entries[parent] = None

    def tree(directory, prefix):
        children = sorted(directory.iterdir(), key=lambda p: p.name.upper())
        seen = set()
        for child in children:
            if not short_name(child.name) or child.is_symlink():
                raise ValueError("expected regular files/directories with DOS 8.3 names")
            name = child.name.upper()
            if name in seen:
                raise ValueError("case-insensitive filename collision")
            seen.add(name)
            path = prefix + (name,)
            if child.is_dir():
                add(path, None)
                tree(child, path)
            elif child.is_file():
                add(path, child)
            else:
                raise ValueError("expected regular files/directories with DOS 8.3 names")

    parents(target)
    tree(source, target)
    if doom and not {("DOOM.EXE",), ("DOOM.WAD",)}.issubset(entries):
        raise ValueError("expected DOOM.EXE and DOOM.WAD (any case)")
    for filename, destination in args.extra:
        path = dos_path(destination)
        if not path or not pathlib.Path(filename).is_file() or pathlib.Path(filename).is_symlink():
            raise ValueError("expected a regular extra file and a DOS target filename")
        parents(path[:-1])
        add(path, pathlib.Path(filename))
    if args.win95:
        add(("MSBATCH.INF",), msbatch(os.environ.get("UNEMUPC_USER_WIN95_KEY")))

    if args.win31:
        for name, data in win31_files().items():
            add((name,), data)

    heads = args.heads if args.heads is not None else (4 if doom else 16)
    spt = args.sectors if args.sectors is not None else (32 if doom else 63)
    if not (1 <= heads <= 16 and 1 <= spt <= 63):
        raise ValueError("invalid CHS geometry")
    required = sum((value.stat().st_size if isinstance(value, pathlib.Path) else len(value))
                   for value in entries.values() if value is not None)
    cylinders = args.cylinders if args.cylinders is not None else (max(544, math.ceil(required * 2 / 65536)) if doom else 1015)
    if args.size_mib is not None:
        needed = math.ceil(args.size_mib * 1024 * 1024 / (heads * spt * 512))
        if needed > cylinders:
            raise ValueError("chosen size exceeds CHS geometry")
        cylinders = needed
    if not (1 <= cylinders <= (8192 if doom else 1024) and 1 <= heads <= 16 and 1 <= spt <= 63):
        raise ValueError("invalid CHS geometry")
    sectors = cylinders * heads * spt
    start = spt
    total = sectors - start
    # Include both FATs and the fixed root when applying the cluster-count rule.
    cluster = 16 if doom else 1
    while True:
        fat = math.ceil(((total // cluster) + 2) * 2 / 512)
        clusters = (total - 1 - 2 * fat - 32) // cluster
        if clusters < 65525:
            break
        cluster *= 2
    if cluster > 64 or clusters < 4085:
        raise ValueError("chosen size cannot hold FAT16")
    allocation = sum(math.ceil((value.stat().st_size if isinstance(value, pathlib.Path) else len(value))
                               / (cluster * 512)) for value in entries.values() if value is not None)
    for path, value in entries.items():
        if value is None:
            count = sum(other[:-1] == path for other in entries) + 2
            allocation += math.ceil(count * 32 / (cluster * 512))
    if allocation > clusters or sum(len(path) == 1 for path in entries) > 511:
        raise ValueError("media exceeds the builder FAT16 capacity")

    work = pathlib.Path(tempfile.mkdtemp(prefix="unemupc-user-media-", dir="/tmp"))
    try:
        image = work / "data.img"
        def chs(lba):
            cylinder, remainder = divmod(lba, heads * spt)
            head, sector = divmod(remainder, spt)
            cylinder = min(cylinder, 1023)
            return bytes([head, sector + 1 | ((cylinder >> 8) << 6), cylinder & 255])
        with image.open("wb") as disk:
            disk.truncate(sectors * 512)
            if args.win31:
                disk.write(win31_mbr())
            disk.seek(446)
            disk.write(bytes([0x80]) + chs(start) + bytes([6]) + chs(sectors - 1)
                       + struct.pack("<II", start, total))
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
        for name in ("MTOOLSRC", "MTOOLS_DATE_STRING", "MTOOLS_NFATS", "MTOOLS_DIR_LEN",
                     "UNEMUPC_USER_WIN95_KEY"):
            env.pop(name, None)
        env.update(HOME=str(work / "home"), MTOOLSRC="/dev/null", MTOOLS_SKIP_CHECK="1",
                   LC_ALL="C.UTF-8", SOURCE_DATE_EPOCH="315532800", TZ="UTC")
        def run(*command, data=None):
            subprocess.run(command, input=data if data is not None else b"", env=env, check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        device = str(image) + "@@" + str(start * 512)
        run("mformat", "-i", device, "-T", str(total), "-h", str(heads), "-s", str(spt),
            "-H", str(start), "-c", str(cluster), "-R", "1", "-d", "2", "-r", "32",
            "-m", "0xf8", "-N", "0x12345678", "-v", "USERMEDIA", "-B", str(work / "boot.bin"), "::")
        for path, value in sorted(entries.items()):
            destination = "::/" + "/".join(path)
            if value is None:
                run("mmd", "-D", "s", "-D", "S", "-i", device, destination)
            else:
                run("mcopy", "-D", "s", "-D", "S", "-i", device, "-" if isinstance(value, bytes) else str(value),
                    destination, data=value if isinstance(value, bytes) else None)
        with image.open("rb") as disk:
            disk.seek(start * 512)
            bpb = disk.read(512)
        size = struct.unpack_from("<H", bpb, 19)[0] or struct.unpack_from("<I", bpb, 32)[0]
        reserved, = struct.unpack_from("<H", bpb, 14)
        root, = struct.unpack_from("<H", bpb, 17)
        fat, = struct.unpack_from("<H", bpb, 22)
        clusters = (size - reserved - bpb[16] * fat - math.ceil(root * 32 / 512)) // bpb[13]
        if not 4085 <= clusters < 65525:
            raise ValueError("builder did not produce FAT16")
        (work / "boot.bin").unlink()
        (work / "home").rmdir()
        print(image)
    except BaseException:
        shutil.rmtree(work)
        raise


def main():
    # argparse's normal error includes user input, which can be private.
    class Parser(argparse.ArgumentParser):
        def error(self, message):
            raise ValueError("invalid builder arguments")
    parser = Parser(description=__doc__)
    parser.add_argument("--source")
    parser.add_argument("--target")
    parser.add_argument("--cylinders", type=int)
    parser.add_argument("--heads", type=int)
    parser.add_argument("--sectors", type=int)
    parser.add_argument("--size-mib", type=int)
    parser.add_argument("--extra", nargs=2, action="append", default=[], metavar=("FILE", "DOSPATH"))
    windows = parser.add_mutually_exclusive_group()
    windows.add_argument("--win95", action="store_true")
    windows.add_argument("--win31", action="store_true")
    try:
        build(parser.parse_args())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        # Neither tool arguments, source names nor key material belong in logs.
        print("user-media disk build failed: " + (str(error) if isinstance(error, ValueError)
              else "input or tool failure"), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
