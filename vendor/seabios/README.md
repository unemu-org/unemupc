# SeaBIOS 1.17.0 ISA firmware fixture

[`fetch.sh`](fetch.sh) builds `bios-1.17.0-isa.bin` from unmodified
[coreboot/seabios](https://github.com/coreboot/seabios), **rel-1.17.0**, pinned
at commit `b52ca86e094d19b58e2304417787e96b940e39c6` (2025-06-10).
Only this README, the build script and our [`config`](config) are committed.
The clone (`src/`), temporary exports (`build/`) and ROM are gitignored.

The firmware catalogue lists this image as `seabios-1.17.0-isa` for the
`generic-386-isa` board (`machines/generic-386-seabios.toml`). Existing 286
machines do not use it. CI builds it in the fixtures lane. There is no SeaBIOS
boot qualification yet: the pinned CPU core still refuses the 386 reset fetch;
see [firmware requirements §3.6](../../backlog/docs/design/firmware-and-machine-classes.md#36-phase-b-firmware-requirements-seabios-on-generic-386-isa).

## Build and tools

From the repository root:

```sh
nix develop .#firmware --command bash vendor/seabios/fetch.sh
```

The separate firmware devShell supplies the i686-elf cross GCC and GNU
binutils, Python 3, GNU make, git, tar, coreutils (`sha256sum`), grep and awk.
Kconfig also needs a **native C compiler**: the shell supplies `HOSTCC` from
its native stdenv, while `CROSS_PREFIX` selects the firmware toolchain.
The script passes both on make's command line because SeaBIOS's Makefile
sets `HOSTCC := $(CC)` internally. It also selects cross `cpp` and
`PYTHON=python3`. Equivalent tools outside Nix can use `CROSS_PREFIX` and
`HOSTCC` environment overrides. The default Rust devShell is unchanged.
**IASL is not required**: ACPI table generation and loading are disabled.

The first build clones the upstream URL into `src/`; a cached clone containing
the pinned commit rebuilds without network. The script never checks out or
modifies a cached working tree: `git archive --format=tar` exports that exact
commit into a temporary directory, whose contents are removed on exit.
The primary source-content check pins that commit's tree object to
`00ca2288a3318f220d235c2e2cb9ca4f6f430874` before exporting. Export uses an
empty temporary bare repository sharing only the cache's objects, so cached
`.git/info/attributes` cannot affect it. System/global Git configuration and
system attributes are disabled; `core.attributesFile=/dev/null`,
`core.autocrlf=false`, `core.eol=lf` and `tar.umask=0002` fix the relevant
conversion and archive settings. Attributes committed in the pinned tree
still apply. The archive SHA-256 is checked in addition to the independent
tree-object pin. The script copies our fragment to `.config`, runs
`make olddefconfig`, checks that the requested settings survived dependency
resolution, then runs `make`. Options hidden by disabled dependencies count
as disabled.

## ISA configuration

Every setting has a rationale in `config`; option names were **READ** in the
pinned `src/Kconfig`. Unspecified settings take that revision's defaults.

- `CONFIG_QEMU=y` retains ISA ATA, CMOS memory sizing and fw_cfg support;
  `CONFIG_ROM_SIZE=128` produces a 131,072-byte image intended for
  E0000h–FFFFFh. This choice alone does not provide the board's writable
  shadow RAM or pristine high BIOS copy.
- `SERCON=y` enables the headless console, whose port still needs the
  board-local fw_cfg `etc/sercon-port` file. `DEBUG_IO=y` and `DEBUG_LEVEL=1`
  retain POST diagnostics at port 402h; a device for this port is separate work.
- `BOOTMENU=n` avoids the interactive wait. Upstream boot ordering, ATA,
  floppy, PS/2 and serial BIOS services remain at their defaults.
- `MPTABLE=n` avoids the APIC read on a single-CPU ISA machine. `PIRTABLE`,
  `PCIBIOS`, PCI storage drivers and chipset-specific `VGAHOOKS` are off
  because there is no PCI bus or chipset interrupt router.
- `SMBIOS`, `ACPI`, `FW_ROMFILE_LOAD`, `ACPI_PARSE`, `PMTIMER`, `USB`,
  `USE_SMM`, `TCGBIOS` and `S3_RESUME` are off: this board has none of their
  platform tables, controllers, power-management facilities or TPM.
  `FW_ROMFILE_LOAD` controls the firmware-table relocation loader, **not**
  ordinary fw_cfg file access: serial configuration and option-ROM delivery
  still need fw_cfg.
- `XEN`, `MTRR_INIT` and `TSC_TIMER` are off because this is an ISA 386
  machine, without Xen, MTRRs or a timestamp counter. Firmware timing uses
  the PIT path.

## Reproducibility and hashes

| Input / output | SHA-256 |
|---|---|
| Pinned commit's `git archive --format=tar` (umask 0002) | `886288c05ea3854fe3e9fcee9a2e23fdfebdb2502abc6a7dc259f9f29f9a9275` |
| Our `config` fragment | `d1958e10e395c8eb00ea9e99ab1c6fe86cf1acb9a0480d0f7a1b551fa4fe8e2a` |
| `bios-1.17.0-isa.bin` | `07add88892bb47898ec712322870b8de7bdb6016dc7f4c3dd169d748be004a13` |

The two input hashes were **MEASURED** locally. **MEASURED by the supervisor
on 2026-11-11:** two independent builds in different temporary directories
produced the same output SHA-256 above on macOS arm64, using i686-elf GCC
15.2.0 and GNU binutils 2.46 from the flake-pinned nixpkgs, with the firmware
devShell's hardening disabled. Both preserved the requested Kconfig settings,
produced 131,072 bytes, used version `1.17.0-unemupc98-isa` and passed the
hostname check. They stopped only at the then-placeholder output-hash check;
the measured hash is now pinned in this README and `fetch.sh`. A third build
passed the pinned output-hash check (MEASURED by the supervisor).

**READ:** `scripts/buildversion.py:43–57,114–131` supports release tarballs
through `.version`; `scripts/tarball.sh:23–38` packages it that way. The
script writes `.version` with `1.17.0` and supplies the nonempty make variable
`EXTRAVERSION=-unemupc98-isa`. This uses upstream's supported clean-build path
without patching source; `VERSION` alone is not a SeaBIOS make override.
`buildversion.py` additionally requires recognized, consistent GCC/binutils
versions. A failed probe would append build time and hostname, so `fetch.sh`
requires the generated `BUILD_VERSION` to be exactly
`1.17.0-unemupc98-isa` as the primary guard. A second check greps the raw
image for the upstream dirty-build suffix `-YYYYMMDD_HHMMSS-<hostname>`
followed by our distributor suffix, failing without printing that identity.
This avoids rejecting unrelated strings when a hostname is short or common.
The fixed version contains no time, user or host name. The ROM also records
compiler/binutils versions, so the pinned flake toolchain and final output
hash remain part of reproducibility.
Updating nixpkgs with `nix flake update` can change GCC/binutils and the
embedded `BUILD_TOOLS` string. The output hash then fails until the new
toolchain's output is reviewed and deliberately re-pinned; there is no
separate nixpkgs input for firmware.
The two matching builds above establish byte-for-byte reproducibility with
that toolchain on macOS arm64.

The script checks the tree object and both input hashes, image size, version,
absence of a dirty-build hostname suffix and output hash before copying
`out/bios.bin` to the fixture path. A mismatch requires reviewing inputs and
toolchain before deliberately changing a pin.

QEMU's prebuilt rel-1.17.0 `bios.bin`, SHA-256
`3dfd946d0c03ab0e022f84f10c3eb5f1dd507761f73e7d8067511ba35a10f776`,
**is not used**. It differs on purpose: its configuration enables MPTABLE
and other QEMU platform features that this ISA build disables. Its hash is
recorded as comparison provenance in the firmware design document's Sources,
not as the expected output of this recipe.

## Provenance and licence

Upstream is [SeaBIOS](https://github.com/coreboot/seabios), with project-level
licence **LGPL-3.0-only**. **READ at the pinned revision:**
`docs/Download.md:1–2` permits distribution under GNU LGPLv3;
`src/post.c:1–6`, `src/sercon.c:1–5`, `src/hw/ata.c:1–6` and
`Makefile:1–5` likewise name GNU LGPLv3 without an "or later" grant.
`COPYING.LESSER:1–16` contains LGPL version 3 and incorporates GPLv3.
Its later-version provision (`COPYING.LESSER:151–156`) requires the library
to grant "or any later version"; it does not itself grant that permission.

Other licence notices **READ** in that same source tree are:

- BSD: `src/sha256.c:1–11`, `src/sha512.c:1–11`,
  `src/std/LegacyBios.h:12–19` and the ACPI table-structure notice in
  `src/std/acpi.h:32–33`.
- LZMA SDK 4.40: `src/fw/lzmadecode.c:1–20` and
  `src/fw/lzmadecode.h:1–20` offer a choice of GNU LGPL or Common Public
  License (CPL), with a special linking exception.
- The build-time `scripts/buildversion.py:6` header names GNU GPLv3.

These notices are not a complete per-file licence audit.
No upstream licence text or source bytes are committed here. The source and
its licence files remain obtainable at the pinned commit in the cached clone.
The ROM is an external firmware fixture, not linked into the emulator;
there is no local SeaBIOS source patch, only our configuration and fixed
release-packaging version metadata.
