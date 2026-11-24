# SeaBIOS 1.17.0 ISA firmware fixture

[`fetch.sh`](fetch.sh) builds `bios-1.17.0-isa.bin` from
[unemu-org/seabios](https://github.com/unemu-org/seabios), pinned at commit
`7da2116237ed8916970795b0d8cae5bf6e277d0a`. The fork is the source of truth:
upstream **rel-1.17.0** (`b52ca86e`) plus the documented fix set in
[`README.unemu.md`](https://github.com/unemu-org/seabios/blob/7da2116237ed8916970795b0d8cae5bf6e277d0a/README.unemu.md).
Its initial code fix (`d98685dd`) replaces `__swab32`'s inline `bswapl` with
C that compiles for `-march=i386`. BSWAP requires a 486, and upstream reaches
it through fw_cfg, ATA and block commands. Fixes are submitted upstream
(`upc-o0j.23`); there are no source patches applied by this build recipe.
Only this README, the build script and our [`config`](config) are committed.
The clone (`src/`), temporary exports (`build/`) and ROM are gitignored.

The catalogue identifies this image as `seabios-1.17.0-unemu1-isa`, separating
its bytes and provenance from the original upstream fixture. The output
filename and version suffix stay unchanged. The board is `generic-386-isa`
(`machines/generic-386-seabios.toml`); existing 286 machines do not use it.
CI builds it in the fixtures lane. Execution qualification is recorded below.

## Execution boundary (historical, CPU pin `deae5bac`)

This section records the former stop. The current pin passes it; see the end
of this section.

**MEASURED on 2026-11-24, CPU pin `deae5bac`:** the shipped diskless machine
passes fw_cfg, reports 16 MiB RAM from CMOS, relocates init, detects no PCI
system and selects COM1 as its primary display. Port 402h ends with
`Turning on vga text mode console`; COM1 contains only the 11-byte terminal
initialization `\x1bc\x1b[?7l\x1b[2J`.

The first `F3 90` (`REP NOP` / `PAUSE`) is reached in big real mode, CPL 0,
at `F000:A72C` (linear `000FA72Ch`), after **1,117,388 retired instructions**
(**1,147,195 completed steps**, including REP iterations). One more bounded
step records `DeliveryKind::Fault`, vector 6, without an error code, and
leaves CS:IP unchanged. At the 10,000,000-step bound, the machine returns
`RetiredCapReached`, with **9,970,193 retired instructions**, still at that
address; #UD deliveries repeat between working IRQ0 timer interrupts.
A 20,000,000-step diagnostic also remained there.

**READ:** fork `src/stacks.c:607–621` (`check_irqs`) emits
`sti; nop; rep; nop; cli; cld`; `src/x86.h:47–49` uses the same REP NOP
encoding in `cpu_relax`. Intel SDM Volume 2B, **PAUSE — Spin Loop Hint**,
specifies NOP behaviour on earlier IA-32 processors, backward compatible
with all IA-32 processors; see the [Intel instruction reference](https://cdrdv2-public.intel.com/835757/325383-sdm-vol-2abcd.pdf).
This was a CPU decoding boundary, not a firmware minimum-CPU violation.
x86jit b7c10b6f executes F3 90 as NOP on every model; no board or firmware
workaround is applied.

The SeaBIOS probes are normal tests: diskless POST to `No bootable device.`,
INT 15h AH=88h/E801h/E820h against the E820 map, and keyboard reset/resume
after complete POST pass. No guest OS boot is qualified; the entry stays
Listed.

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

The first build clones the fork URL into `src/`; a cached clone containing
the pinned commit rebuilds without network. The script never checks out or
modifies a cached working tree: `git archive --format=tar` exports that exact
commit into a temporary directory, whose contents are removed on exit.
The primary source-content check pins that commit's tree object to
`81259d7b00535b6a9dd531d60cd907f4d9218c67` before exporting. Export uses an
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
  retain POST diagnostics at the board-local debug console, port 402h.
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
| Pinned commit's `git archive --format=tar` (umask 0002) | `f909c5f540c976ac6d323a51658350710f88b2a67afa8b832998dc126c517c94` |
| Our `config` fragment | `d1958e10e395c8eb00ea9e99ab1c6fe86cf1acb9a0480d0f7a1b551fa4fe8e2a` |
| `bios-1.17.0-isa.bin` | `ad50fdb5e12a03dd5c17eac481a4183a7793b84252319eea02b8c878667c994f` |

**MEASURED on 2026-11-24:** the input hashes and output above. Two
independent builds in different temporary directories produced identical
131,072-byte ROMs (direct `cmp`), using i686-elf GCC 15.2.0 and GNU binutils
2.46 from the flake-pinned nixpkgs on macOS arm64, with hardening disabled.
Both passed all pinned checks, preserved the Kconfig settings, used version
`1.17.0-unemupc98-isa` and passed the hostname check. An earlier measurement
build stopped at the old output-hash check and reported the same new hash.

**READ:** `scripts/buildversion.py:43–57,114–131` supports release tarballs
through `.version`; `scripts/tarball.sh:23–38` packages it that way. The
script writes `.version` with `1.17.0` and supplies the nonempty make variable
`EXTRAVERSION=-unemupc98-isa`. This uses upstream's supported clean-build path
without changing the exported fork source; `VERSION` alone is not a SeaBIOS make override.
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
the fork carries the documented compatibility fixes, while this recipe adds
only our configuration and fixed release-packaging version metadata.
