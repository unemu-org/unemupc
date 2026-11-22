# User-supplied system ROMs

These images are proprietary. Users supply them separately; this project never
commits or distributes them. The catalogue stores identities and chip metadata
only. Presets contain no proprietary ROM paths. The opt-in
`machines/at-games.toml` selects the IBM 1985-06-10 system BIOS and IBM VGA BIOS
from this directory; users must supply both separately. Most system BIOS
entries remain Listed; execution qualification is specific to a measured machine
and ROM pairing. The IBM Display Adapter VGA entry is Tested for the case below.
Landmark AT has a partial real-mode golden awaiting final local measurement pins,
as described in `backlog/docs/design/compatibility-diagnostics.md` §3.

You may store parts at the repository-relative path
`vendor/user-roms/<entry id>/<part number lowercase>.<chip label lowercase>`.
These are MAME-style names, for example
`vendor/user-roms/ibm-5170-bios-1985-06-10/6480090.u27`.
These filenames are optional labels. Tests search recursively in
`vendor/user-roms/`, then in `UNEMUPC_USER_ROMS_DIR` if set. Set that variable
to an absolute path to a folder of unpacked dumps, rather than a volume root or
home directory. Dumps must be plain files: MAME `.zip` sets and other archives
are not unpacked by discovery. Arbitrary names and nested directories work. Identity is
the catalogue SHA-256, either of a complete pre-interleaved image or of every
part in one known layout. Earlier roots take priority; parts can span both roots.
Only regular files of known sizes are hashed, once per canonical path per test
process; keep files unchanged during a test run. Root, file and directory symlinks
are followed; canonical directory tracking stops loops. A set-but-missing,
relative or non-directory external root is a configuration error, even if vendor
firmware matches. An unreadable subdirectory aborts scanning with a redacted
error instead of silently skipping.
Discovery never prints contents or hashes of user files. Display names and
discovery errors use `<UNEMUPC_USER_ROMS_DIR>/relative/name` instead of the
private root path. Incomplete or wrong layouts are absent, without per-file hash
diagnostics from discovery. Partial layouts append only counts to the skip line,
for example `; layout 0: 1 of 2 parts found`, with no paths or hashes.
A pre-interleaved 65536-byte file works with `firmware.bios.image` or `--bios` too.

Verify each dump with `shasum -a 256 <file>` or `sha256sum <file>`.

| Entry | Board | Assembled size | Assembled SHA-256 |
|---|---|---|---|
| ibm-5170-bios-1984-01-10 | type 1, 6 MHz | 65536 | `74e7b36b4ec0adc5ac3277a887579996c1d2aa755b9892ef3afe7485c10ce04f` |
| ibm-5170-bios-1985-06-10 | type 2, 6 MHz | 65536 | `6fb653b58e678cb69b0f943a105c6c803b18a6db23d13c0abccec5b66c6ee652` |
| ibm-5170-bios-1985-11-15 | type 3, 8 MHz | 65536 | `872febf7d7034bfadf72ed1936e06600eca72456b9ded41d15b7a284de20337d` |
| landmark-rom-post-at-1.2.0 | ibm-5170 diagnostic | 65536 | `a8310f83de0f3e6b71e4a3ff82a01ffe3c3b5a957677aed2fad17b96bce2e577` |

Offsets below are in assembled bytes. Layout A and B of 1984 are alternatives.

| Entry / layout | Chip | Part number | Lane | Offset | Size | Part SHA-256 |
|---|---|---|---|---|---|---|
| 1984-01-10 / A | U27 | 6181028 | even | 0 | 32768 | `a7d0f995c25a29f2556fce563939cbd101f9225f71103debe441e3b6458cf933` |
| 1984-01-10 / A | U47 | 6181029 | odd | 0 | 32768 | `fadb3a5c949b0096b2f753c76bac2edf73040066c1959eeee8f7d092c836b6c5` |
| 1984-01-10 / B | U17 | 6181024 | even | 0 | 16384 | `b9a9aa7a32c4d50ac16034a489962cbd06455ab14cbb78a6d2bbf519b22c8a73` |
| 1984-01-10 / B | U37 | 6181026 | odd | 0 | 16384 | `a54d9546361464489a118e6d7a7f9f959e6678013ab4f4e8a98993ac7bdc916d` |
| 1984-01-10 / B | U27 | 6181025 | even | 32768 | 16384 | `87340797ab2f2eb86185f5289bd0a2bc2e9f14e0633770a87bf4cb0624208a5c` |
| 1984-01-10 / B | U47 | 6181027 | odd | 32768 | 16384 | `a1106cc91a86251c524db72934f7eaa5abb39c2a986a5ac3477a017da7aac6e3` |
| 1985-06-10 / A | U27 | 6480090 | even | 0 | 32768 | `c8654ed7a922644db2b2cce54c540fcfa7cbbae8c514d7eb3dbfbb8187471bfd` |
| 1985-06-10 / A | U47 | 6480091 | odd | 0 | 32768 | `470ab30f9eb7d56c24f5ce10c7d307c539b9f923ea97dd536d2d5ba7e36d471b` |
| 1985-11-15 / A | U27 | 61X9266 | even | 0 | 32768 | `3b921a451d78efcd91232a01cd6936d6ba12dad4374466655aff8ef3a5dc764e` |
| 1985-11-15 / A | U47 | 61X9265 | odd | 0 | 32768 | `aec1759a8245d50be93c9b081ab1ac7de58dd8141f603c7745ae66d2d4c29e4d` |
| landmark-rom-post-at-1.2.0 / A | U27 | 27256 | even | 0 | 32768 | `3677eb9f1b8c39daef82a99337649f43633a46db8726bc6a3805076ba3b25916` |
| landmark-rom-post-at-1.2.0 / A | U47 | 27256 | odd | 0 | 32768 | `6a74a12bc1cda8c67c0526e061c62b8d7e1b622afdf3e9442950355571580313` |

Landmark uses EPROM type `27256` for the part number because no separate chip
part numbers are established. Its MAME `ibm5170` selector is `landmark`; the
filenames below contain a space before `27256`:

| Entry | Chip | Original MAME filename | Optional repository filename |
|---|---|---|---|
| landmark-rom-post-at-1.2.0 | U27 even | `5170_even_u27_ 27256.bin` | `landmark-rom-post-at-1.2.0/27256.u27` |
| landmark-rom-post-at-1.2.0 | U47 odd | `5170_odd_u47_ 27256.bin` | `landmark-rom-post-at-1.2.0/27256.u47` |

The diagnostic occupies the BIOS socket but supplies no boot services. Use an
FDC with no bound drives for the real-mode-prefix test; no disk is needed.
The assembled 64 KiB image contains two identical 32 KiB halves.

Create a child layer in `machines/` (the parent `at-freedos.toml` stays unchanged):

```toml
schema = 1
extends = "at-freedos.toml"

[firmware.bios]
parts = [
  { image = "../vendor/user-roms/ibm-5170-bios-1985-06-10/6480090.u27", lane = "even" },
  { image = "../vendor/user-roms/ibm-5170-bios-1985-06-10/6480091.u47", lane = "odd" },
]
sha256 = "6fb653b58e678cb69b0f943a105c6c803b18a6db23d13c0abccec5b66c6ee652"
```

Each part also accepts `offset` (default 0) and an optional `sha256` pin.
The BIOS-level pin checks the assembled bytes. `image` and `parts` are mutually
exclusive. A child source replaces the complete inherited source and drops
inherited `sha256` and `provides` unless the child restates them. Known images
then use their catalogue services. Editing an existing BIOS table in place
requires removing `image` before adding `parts`, and removing `provides` or
setting it to `["floppy", "fixed-disk"]` for these IBM images.
Relative paths belong to
the layer that defines them. Only the system BIOS supports parts.

Pairs must have equal nonzero chip sizes and tile the assembled image without
gaps or overlaps. The layout with the most matching chip hashes (and the same
part count) diagnoses chips in wrong lanes or halves and names every bad dump
with its expected chip and expected/actual hashes. Exchanged lanes within pairs
are reported as swapped only when each known chip occurs exactly once.
Duplicated and missing chips are named explicitly, as are positions outside the
chosen layout. These diagnoses precede the assembled pin check;
individual part pins are checked first. A pin for another listed BIOS still
reports the assembled pin mismatch.
Unknown assembled identities report the board catalogue. The run-only
`--allow-unlisted-firmware` option relaxes membership and chip-identity diagnoses, retaining layout and pin checks.

The ignored `user_rom_ibm_1985_identity` checks the 1985-06-10 identity and chip
diagnostics when present, without executing it. It accepts parts or a complete
image. `user_rom_landmark_at_identity` checks the diagnostic's assembled and
per-part hashes, including the mirrored halves. The ignored CLI test
`user_rom_landmark_at_real_mode_prefix` checks twelve Passed verdicts and the
protected-mode instruction stop using a headless MDA capture. Its final retired
count and text-plane hash are TODO-MEASURE until a new local run. Adapter gaps
now have board-owned read-only zero mappings, fixing the previous C0001h fault
for the Listed entry. See the diagnostic design document for details. It never
prints the screen. Run all such tests with
`bash scripts/ci-gate.sh user-roms`; CI never calls this lane and never has proprietary firmware. Missing dumps print:

```text
SKIPPED: user-supplied firmware <entry id> not found (set UNEMUPC_USER_ROMS_DIR or see vendor/user-roms/README.md)
```

This also skips with `UNEMUPC_REQUIRE_FIXTURES=1`. Set
`UNEMUPC_REQUIRE_USER_ROMS=1` to panic with the same text on absence.
See `backlog/docs/commands.md` for the commands and shared test helper.

Goldens for proprietary firmware may store hashes and our own interpretation:
POST codes, which tests passed, retired-instruction counts and similar measured
outcomes. Never store proprietary firmware bytes or copies of its screen text
beyond short identifiers. Screenshots and full console transcripts do not belong
in these goldens. All committed discovery tests use synthetic bytes.


## IBM Display Adapter video BIOS (ISA VGA, 1987)

Catalogue id: `ibm-vga-bios-1987-32k`. Supply one **32768-byte EPROM file** in
this ignored directory or under `UNEMUPC_USER_ROMS_DIR`. Discovery is recursive
and hash-based; an optional descriptive name is
`ibm-vga-bios-1987-32k/ibm-vga-1987-32k.bin`. No separate socket or byte-lane
parts are required; this is a single eight-bit EPROM, not an even/odd pair.

- SHA-256: `51b49761f72780a696c36f3d2e74a8168c1c99b865070f7f90b8e621a8b5ae17`
- SHA-1 (identification reference): `7b8b0e22ecd002d43d5ed1bb9f9a63d9b9948032`
- Supervisor-measured layout: zero prefix 0000h–1FFFh, checksummed 24 KiB
  option ROM at file offsets 2000h–7FFFh, signature 55AAh, size byte 30h.
  The adapter presents that suffix at C0000h–C5FFFh.

Keep the complete file: catalogue identity and machine-file pins cover all
32 KiB. The loader applies the known 2000h offset automatically. A plain 24 KiB
dump is **not listed** because its SHA-256 has not been measured. To add it,
measure that dump, then add a separate catalogue entry with size 24576 and
`rom_offset=0`; do not reuse the container hash or guess it from SHA-1.
See `backlog/docs/design/machine-files.md` §VGA video BIOS for the existing
`firmware.option_roms` declaration alongside `kind='vga'`.

The entry requires `ibm-5170`, a system BIOS with a known video-window scan and
the register-level VGA card. Validation requires its mapped bytes to fit inside
that BIOS's video window; a video BIOS cannot be used as `firmware.bios`.
The entry is Tested for this pairing. The ignored
`user_rom_ibm_vga_bios_mode13_pattern` additionally requires the catalogued
1985-06-10 IBM 5170 system BIOS (whole image or its U27/U47 pair above). It boots
a synthetic program, captures the text plane before INT 10h mode 13h, draws a
pattern and reports only stop/counters/hashes and numeric register state.
The supervisor qualified POST with this ROM and INT 10h modes 03h/13h on
`ibm-5170`; all four regression pins are populated. See
`backlog/docs/design/firmware-and-machine-classes.md` §Video ROM placement
for the measured stop, retired count and capture SHA-256 values. Missing ROMs
skip cleanly even when ordinary open fixtures are required, unless `UNEMUPC_REQUIRE_USER_ROMS=1`.
No ROM is downloaded or distributed.

## Game files for the FAT builder's user-file tests

The ignored tests in `crates/pc-harness/tests/user_media_volumes.rs` build diskettes
from a user's own copy of two games, as a recipe's `fat` step will, and check
that each builds twice to the same bytes. They look in the same two roots as
the ROM tests (this folder, then `UNEMUPC_USER_ROMS_DIR`) for the games' files,
unpacked, each found by its size and SHA-256 under any name or folder. Only
names, sizes and hashes are in the repository; the tests print nothing about
your files but counts.

| Test | Files | Where the list is |
|---|---|---|
| `user_rom_kenslab_disk_builds_twice_to_the_same_bytes` | the 19 members of Ken's Labyrinth 2.01's `LABFULL.ZIP` (940242 bytes, SHA-256 `643aa0ca8d1a7d2c88afecd952da00f3b912be8b83c4c1d2f4ab9b50491fadfd`), freeware from the author's site; unpack it, the tests do not read the archive | [`backlog/docs/design/recipe-corpus/kenslab-2.01/kenslab-2.01.media.toml`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-corpus/kenslab-2.01/kenslab-2.01.media.toml), the `unpack` step's `members` |
| `user_rom_azuramoon_split_builds_twice_to_the_same_bytes` | the 561 files (204 distinct contents) of the Azuramoon chapter I folder | [`backlog/docs/design/recipe-corpus/azuramon/azuramoon-2026.media.toml`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-corpus/azuramon/azuramoon-2026.media.toml), `[input.game]` `files` |

Run them with `cargo test --offline -p pc-harness --test user_media_volumes -- --ignored`.
