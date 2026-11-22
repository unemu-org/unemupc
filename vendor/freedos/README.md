# vendor/freedos — bootable FreeDOS floppy fixtures

`fetch.sh` builds three **bootable FAT12 FreeDOS floppy images** in one run:

- `freedos.img`: 1.44 MB, 1,474,560 bytes (2880 sectors), used by the existing
  open-BIOS boot and DOS session goldens.
- `freedos-1200.img`: 1.2 MB, 1,228,800 bytes (2400 sectors), used by the
  user-supplied IBM 5170 BIOS boot measurement. The 1985-06-10 BIOS supports
  360K, 1.2M and 720K drive types, so this measurement uses a 1.2M drive.
- `freedos-tools-1200.img`: 1.2 MB, 1,228,800 bytes (2400 sectors), an open
  installation and utility floppy for the AT. It extends `freedos-1200.img`
  with the programs below and `README.TXT`. All tools fit; none is omitted.

All are read-only boot fixtures. Images and downloaded sources are not
committed; pinned FreeDOS inputs rebuild them reproducibly, like
`vendor/kiselev-bios` and `vendor/test386`.

## What it produces

The two minimal boot floppies contain the same files in the root:

- **`KERNEL.SYS`** — the FreeDOS kernel, **8086 build** (`ke2044_86f16`, FAT12/16,
  no FAT32). The default installer kernel is 386-only and would fault on our
  real-mode 286-class core; the 8086 build boots in pure real mode without
  A20/HMA.
- **`COMMAND.COM`** — FreeCOM, **8086 build** (`com086`).
- A **FreeDOS FAT12 boot sector**, assembled from the kernel source
  (`boot/boot.asm`, `-dISFAT12`) and installed over the formatted image's boot
  code while preserving the geometry-specific BPB `mformat -f 1440` or
  `mformat -f 1200` wrote.

No `FDCONFIG.SYS`/`AUTOEXEC.BAT` is placed: with none present the kernel loads
`COMMAND.COM` as the default shell. FreeCOM asks for the date and time;
two Enter keypresses accept them and reach the DOS prompt.

The tools floppy also contains the following programs. Versions and licence
labels come from `APPINFO/*.LSM` in each package (kernel metadata is
`docs/fdkernel.lsm`). The FreeDOS project and each program's authors supply
these freely redistributable tools; this repository does not author them.

| Program | Package version | Licence | Compatibility evidence in the pinned archive |
|---------|-----------------|---------|----------------------------------------------|
| `KERNEL.SYS` | 2044 FAT12/FAT16 | GPL v2 or later | `docs/readme.txt`: 8086 FAT16 binaries |
| `COMMAND.COM` | FreeCOM 0.86 (`com086`) | GPL v2 or later | The existing pinned 8086 shell fixture |
| `SYS.COM` | Kernel 2044 | GPL v2 or later | Same `ke2044_86f16.zip`; `docs/readme.txt` describes 8086 binaries |
| `FDISK.EXE` | 1.3.4a | GPL v2 | Included `SOURCE/FDISK/SOURCES.ZIP`, `FDISK/MAKEFILE`: `-1` selects 80186/286 instructions; 186/286 or later, not 8086 |
| `FORMAT.EXE` | 0.92 | GNU GPL (v2 text in `DOC/FORMAT/LICENSE.TXT`) | Included `SOURCE/FORMAT/SOURCES.ZIP`, `makefile`: `-0` (8086) and UPX `--8086` |
| `XCOPY.EXE` | 1.7 | GPL v2 | Included `SOURCE/XCOPY/SOURCES.ZIP`, `makefile`: `-0` (8086) and UPX `--8086` |
| `EDIT.EXE` | 0.9b | GPL v2 | LSM: DOS/Borland C; included `SOURCE/EDIT/SOURCES.ZIP`, `EDIT.MAK`: 16-bit Borland build, optional `-1` targets 186/286, UPX `--8086` |
| `MEM.EXE` | 1.12 | GPL v2 | Included `SOURCE/MEM/SOURCES.ZIP`, `MEM/MKFILES/GENERIC.MAK`: UPX `--8086`; 16-bit small-model compiler settings in `WATCOM.MAK` |
| `CTMOUSE.EXE` | 2.1b4 | GPL v2 | Included `SOURCE/CTMOUSE/SOURCES.ZIP`, `CTMOUSE.ASM`: `.286`; `DOC/CTMOUSE/HISTORY.TXT` records 286/386 support; 286 or later, not 8086 |

`EDIT.HLP` accompanies EDIT under its licence. The LSMs mostly declare DOS or
FreeDOS without a minimum CPU, so the more specific history, technical notes
and bundled build instructions above establish the per-program CPU requirements.
FDISK requires 186/286 or later, not 8086; CTMOUSE requires 286 or later.
The complete tools set therefore requires a 286. CTMOUSE TECHNOTE item 5
describes the older 2.0 implementation and does not establish 2.1b4 compatibility.
These are compatibility evidence, not a claim that every optional tool has
been exercised end to end in the emulator. No included tool requires a 386.
Full licence texts and sources remain in the pinned upstream archives under
`DOC/` and `SOURCE/` (the kernel uses `docs/`).

## Pinned versions (provenance)

| Component | Source | Pin |
|-----------|--------|-----|
| Kernel (8086) `KERNEL.SYS` | `github.com/FDOS/kernel` release | tag `ke2044`, asset `ke2044_86f16.zip` |
| Shell (8086) `COMMAND.COM` | `github.com/FDOS/freecom` release | tag `com086`, asset `command.zip` |
| Boot sector `boot/boot.asm` | `github.com/FDOS/kernel` source | commit `8535ddb7a783f94b3aae844c8fe7f5ccce9037eb` (tag `ke2044`) |

Archive SHA-256 sums are checked in `fetch.sh`; the assembled boot sector is
size-checked (512 bytes). Each image is checked for its size, the `0x55AA`
signature and SHA-256:

| Image | Bytes | SHA-256 |
|-------|-------|---------|
| `freedos.img` | 1,474,560 | `b7b088333f7f30584d768941fecea8288b63f2d61e13ec6c9472439eda66e51f` |
| `freedos-1200.img` | 1,228,800 | `4ee55e9c44d553041b9bbde99cacae31472f12debf6c9b9c09c5b9343c435d7e` |
| `freedos-tools-1200.img` | 1,228,800 | `8d91283a1c0d6fc5b5fdb5e40db5b66dd9fabbf014d37262b7892f3b150b6f63` |

**MEASURED:** `fetch.sh` reproduces all three pinned image hashes from the pinned archives and source with mtools 4.0.49, whose version appears in the boot-sector OEM name.

The six utility archives come from the FreeDOS 1.3 repository at
`https://www.ibiblio.org/pub/micro/pc-stuff/freedos/files/repositories/1.3/base/<name>.zip`.
`fetch.sh` pins each archive's full SHA-256, including FORMAT 0.92 and XCOPY 1.7
as supplied in that repository; the repository name alone is not a version pin.

**MEASURED:** Two offline tools builds with mtools 4.0.49 compare byte for byte,
produce the pinned SHA-256, and leave 854,016 bytes free. The system files and
loader code come from the verified 1.2 MB boot fixture; only
the tools volume identity fields and filesystem contents change.

A hash mismatch fails the build and reports both hashes. Check sources and tools;
changing a pinned output hash requires deliberately rebuilding and reviewing the
goldens' expectations.

The minimal boot floppies use label `FREEDOS` and serial `2895-9853`.
The tools floppy uses label `FDTOOLS` and serial `2895-9854`, so DOS can
distinguish it from the boot floppy when switching media in the same drive.
`SOURCE_DATE_EPOCH=315532800` and `TZ=UTC` fix all mtools
volume-label and file creation, access and modification timestamps at the FAT
epoch, 1980-01-01 00:00:00 UTC. `mcopy` deliberately omits `-m`, so archive and
host file mtimes do not enter the image. Use mtools with `SOURCE_DATE_EPOCH`
support (the devShell provides 4.0.49); an incompatible build fails the image hash
check.

## License

FreeDOS kernel and FreeCOM are **GNU GPL v2 or later**, freely redistributable.
This project only *reads* the image at runtime (a boot fixture); it does not link
or incorporate FreeDOS code. Upstream license text ships inside each archive
(`docs/` in the kernel zip, sources in the FreeCOM zip). Anyone passing the
built image on takes on the applicable GPL source obligations: provide the
corresponding source as required by each package's licence. The pinned utility
archives ship it in `SOURCE/*/SOURCES.ZIP`; retain those archives and the pinned
kernel/FreeCOM sources with any redistribution.

## Usage

```sh
vendor/freedos/fetch.sh             # builds all three images
```

Requires `git`, `nasm`, `unzip`, `curl`, and mtools (`mformat`/`mcopy`/`mdir`/`mlabel`) —
all in the flake devShell. SHA-256 uses `sha256sum` or macOS's `shasum -a 256`;
size and boot-signature checks also use portable utilities. The images, archives
and source clone are gitignored. Once `build/ke2044_86f16.zip`,
`build/command.zip` and the pinned commit in `build/kernel-src/` are cached,
rebuilding needs no network. Cached archives are still hash-checked; missing
archives or commits are fetched as before.

For a tools-only offline build, first provide the already pinned
`freedos-1200.img` and a directory containing `ke2044_86f16.zip` plus
`fdisk.zip`, `format.zip`, `xcopy.zip`, `edit.zip`, `mem.zip`, `ctmouse.zip`:

```sh
FREEDOS_PACKAGE_DIR=/path/to/cached-packages vendor/freedos/fetch.sh
```

When set, this variable disables downloads and source-clone operations and
builds only `freedos-tools-1200.img`. It checks the boot fixture and every
archive before copying any tools. Missing inputs fail; it never silently
falls back to the network. Without the variable, the original source build
runs first, followed by the tools build. All three outputs retain the same
FAT timestamp rules and fixed per-volume serials.
