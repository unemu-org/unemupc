# SeaBIOS and SeaVGABIOS 1.17.0 ISA firmware fixtures

[`fetch.sh`](fetch.sh) builds `bios-1.17.0-isa.bin` from
[unemu-org/seabios](https://github.com/unemu-org/seabios), pinned at release
**v1.17.0-unemu1**, commit
`e38573ec2d7d43fb7802bc43ddcb3ce2bf892809`. The fork is the source of truth:
upstream **rel-1.17.0** (`b52ca86e`) plus the documented fix set in
[`README.unemu.md`](https://github.com/unemu-org/seabios/blob/e38573ec2d7d43fb7802bc43ddcb3ce2bf892809/README.unemu.md).
Its initial code fix (`d98685dd`) replaces `__swab32`'s inline `bswapl` with
C that compiles for `-march=i386`. BSWAP requires a 486, and upstream reaches
it through fw_cfg, ATA and block commands. Fixes are submitted upstream
(`upc-o0j.23`); there are no source patches applied by this build recipe.
Only this README, the build script and our [`config`](config) and
[`vgaconfig`](vgaconfig) fragments are committed.
The clone (`src/`), temporary exports and release source archive (`build/`),
and ROM are gitignored. The release adds LGPLv3-noticed rebuild files in
`unemu/` and README updates; its firmware sources are unchanged.

The catalogue identifies this image as `seabios-1.17.0-unemu1-isa`, separating
its bytes and provenance from the original upstream fixture. The output
filename and version suffix stay unchanged. The board is `generic-386-isa`
(`machines/generic-386-seabios.toml`); existing 286 machines do not use it.
CI builds both ROMs in the fixtures lane. Execution qualification is recorded below.

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
`66f06eca7df3167ba2f2aef72f2a23dc057a37c3` before exporting. Export uses an
empty temporary bare repository sharing only the cache's objects, so cached
`.git/info/attributes` cannot affect it. System/global Git configuration and
system attributes are disabled; `core.attributesFile=/dev/null`,
`core.autocrlf=false`, `core.eol=lf` and `tar.umask=0002` fix the relevant
conversion and archive settings. Attributes committed in the pinned tree
still apply. The archive SHA-256 is checked in addition to the independent
tree-object pin. Both temporary build trees exclude `unemu/`, as the release
recipe does: packaging files are not firmware build inputs. The script copies
our fragment to `.config`, runs
`make olddefconfig`, checks that the requested settings survived dependency
resolution, then runs `make`. Options hidden by disabled dependencies count
as disabled.

The script also writes `build/seabios-1.17.0-unemu1-src.tar`, exactly the
2,856,960-byte source asset of release `v1.17.0-unemu1`. It archives the same
commit with prefix `seabios-1.17.0-unemu1/` and checks
`SOURCE_ARCHIVE_SHA256` before installing it. This is the byte stream of:

```sh
GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null git -c core.autocrlf=false -c tar.umask=0002 archive --format=tar --prefix=seabios-1.17.0-unemu1/ e38573ec
```

Both catalogue entries pin this archive. It contains `COPYING`,
`COPYING.LESSER`, the firmware sources and the LGPLv3-noticed `unemu/config`,
`unemu/vgaconfig`, `unemu/build.sh` and `unemu/BUILD.md`. Extracting it and
running `bash unemu/build.sh <output-directory>` rebuilds both release ROMs
with the documented toolchain. The fixture tests check its hash, Git commit
header, prefix, fixed metadata, configuration fragments, rebuild recipe and
licence files. The local ROM filenames and their SHA-256 pins stay unchanged.

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
| Pinned commit's `git archive --format=tar` (umask 0002) | `7dc0759caf2ff46e68bec9f2df8222c718cd0627d6969b3ec70ab0c753a6b5d0` |
| `build/seabios-1.17.0-unemu1-src.tar` (release source asset) | `b85015a695a9b1b8080c4871335bb68c2ec9aa9c8ba69e88a122a925c580c8c0` |
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

## Standard VGA ROM

The same command also builds `build/vgabios-1.17.0-stdvga.bin` from the same
pinned source archive, in a separate temporary export. The stable output lives
under the already ignored `build/` directory; no ROM binary is committed.
The catalogue ID is `seavgabios-1.17.0-unemu1-stdvga`, with role video BIOS
and fits for `generic-386-isa` and `generic-486-isa`. The system BIOS config
and its pinned output bytes are unchanged.

**READ:** `vgasrc/Kconfig` directly offers `CONFIG_VGA_STANDARD_VGA` for the
original IBM 256 KiB VGA. `vgasrc/vgahw.h` dispatches it to `stdvga_*`.
`CONFIG_VGA_PCI=n` removes the PCI data pointer from the option-ROM header;
`CONFIG_VGA_VBE=n` avoids unsupported extensions. The Bochs variant is
unnecessary: `bochsvga_setup` would probe ports 1CEh/1CFh and fall back to
standard VGA when its DISPI ID readback fails. The chosen variant does not
probe those ports. The existing extra INT 10h stack remains enabled.

| Input / output | SHA-256 |
|---|---|
| Our `vgaconfig` fragment | `370f78d9c30db41bd155d7b360fbf5cd0d6e26e390e19c106c14d32dbf5b5d40` |
| `build/vgabios-1.17.0-stdvga.bin` | `1adc6c0fa72b93c5d0659575c992b1bf6f64fd451c444b2a735e22f0a3d6e925` |

**MEASURED on 2026-11-24:** two independent builds in different temporary
exports produced identical images (`cmp`) with the hashes above, using the
same GCC 15.2.0 / binutils 2.46 firmware toolchain as the system BIOS. Both
builds also passed the unchanged system BIOS hash. A review of the linked
16-bit disassembly found no post-386 instructions: in particular no BSWAP,
WBINVD/INVD/INVLPG, CPUID, timestamp/MSR, XADD/CMPXCHG, CMOV or SIMD
instructions. This is an instruction audit, not exhaustive execution coverage.
The previously noted system-BIOS WBINVD in an unreachable PCI path is not
part of this VGA ROM.

The expected image size is 32,768 bytes. Its version is fixed at
`1.17.0-unemupc-stdvga`; its 55AAh header, 40h length byte and checksum
are checked by normal firmware loading. `fetch.sh` checks the config, source,
output size, version and output hash before installing the image. It leaves
`build/vgabios.disasm`, made with `i686-elf-objdump -d -M i8086` on the
linked `vgarom.o`, for a symbol-aware 16-bit minimum-CPU audit.

**READ:** two diagnostic details matter when interpreting the probes.
`src/optionroms.c:429–483` prints `No VGA found, scan for other display`
after the PCI search and **before** loading `vgaroms/` files. This notice
persists on a successful non-PCI machine; it does not mean fw_cfg deployment
failed. Removing it requires a separate fork change, since the system BIOS
bytes are pinned here. Tests instead require option-ROM execution, an
installed VGA INT 10h handler, the VGA initialization diagnostics and the
font-rendered POST screen.

SeaVGABIOS's port-debug path also cannot use the system BIOS's
`DebugOutputPort VARFSEG`: the VGA linker discards that weak section
(`src/types.h:70`, `vgasrc/vgalayout.lds.S`). **MEASURED:** the initial
port-debug build loaded the port from CS:0000h, the ROM signature, and emitted
no banner at 402h. `vgaconfig` therefore selects the supported serial-debug
path on COM1 instead. The system BIOS still logs at 402h and mirrors its
console to COM1; SeaVGABIOS initialization lines appear on COM1. No emulator
port alias or firmware-source patch hides either diagnostic issue.

## Provenance and licence

Upstream is [SeaBIOS](https://github.com/coreboot/seabios), with project-level
licence **LGPL-3.0-only**. **READ at the pinned revision:**
`docs/Download.md:1–2` permits distribution under GNU LGPLv3;
`src/post.c:1–6`, `src/sercon.c:1–5`, `src/hw/ata.c:1–6`,
`vgasrc/vgainit.c:1–7`, `vgasrc/stdvga.c:1–7` and
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
its licence files remain obtainable at the pinned commit in the cached clone
and in the verified release source archive.
The ROM is an external firmware fixture, not linked into the emulator;
the fork carries the documented compatibility fixes, while this recipe adds
only our configuration and fixed release-packaging version metadata.
