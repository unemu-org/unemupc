# Explicit content bindings in the PC core

Status: accepted with a reduced first scope (maintainer, 2026-11-24).

## Decisions

- First scope: grammar and positioned planning, fact validation and
  logical-key resolution, the two core operations with the prepared launch,
  and a native zero disk. Integration covers the open AT templates.
- A native zero disk stays document-owned, as the contract requires: no zero
  content row and no front-end blob. Its base is a bounded-memory zero sector
  source behind the existing overlay. Journal reopening is covered in the
  first scope. Existing current-reader exports also support zero media;
  they do not need the generated base identity.
- Deferred: passing supplied receipts down to ROM and CMOS image lowering.
  Until then the core keeps its existing digest checks of small ROM bytes
  and supplied CMOS images, including journal-backed CMOS through
  `Context::image` in `crates/pc-config/src/lower.rs`.
  The contract forbids rehashing supplied content, so this is a known gap,
  tracked as follow-up work.
- The three questions below are answered as recommended: a portable prepared
  launch may be stored as a machine config, templates require reviewed
  catalogue firmware, and roles follow from position when the document
  omits them.

Evidence labels: **MEASURED** means exercised by the named tests.
**READ** means inspected source at `file:line`.
**ASSUMED** means a proposal or an estimate that has not been implemented or measured.
Paths prefixed `unemu-core-api:`, `unemu:` or `unemu-media:` name those repositories.
Other paths name this repository. Contract evidence is from revision `086c5c2`.

## Boundary and document types

**READ** (`unemu-core-api:src/binding.rs:169`, `unemu-core-api:src/core.rs:132`):
composition returns an owned document and its validation result.
The document must be host-free and digest-addressed.
Ordinary create must consume it without package facts.
Zero disks remain owned by the document.

**READ** (`crates/pc-config/src/schema/v1.rs:106`,
`crates/pc-config/src/image/mod.rs:139`, `crates/pc-config/src/launch.rs:58`):
there are three representations today.
The machine-file grammar has path-based ROMs and drives.
The stored image envelope contains that hardware grammar in `machine`, plus digest bindings.
The launch envelope adds flat-content receipts and storage-record names.
`PcCore` currently accepts only `application/vnd.unemu.pc.launch.v1+toml`
(`crates/pc-core/src/core.rs:33`, `crates/pc-core/src/core.rs:204`).

**ASSUMED — proposal:** extend the shared schema-1 hardware grammar additively.
Use the existing launch media type for both template input and prepared output.
Do not add another codec or media type for templates.

| Document | Bound forms | Validation and storage |
| --- | --- | --- |
| Host machine file, TOML or YAML | Yes, in its normal hardware positions | A binding-aware loader supplies facts. Existing path-only loading stays available. |
| Image config's `machine` table | Yes, through the same hardware parser | Unresolved hardware is template input, not a valid stored machine image. Normal image validation still requires resolved bindings. |
| Launch document | Yes, inside `machine` before compose | Compose resolves it. The resulting launch document is accepted by validate, create, describe_media and export. |

**ASSUMED:** the front end passes platform `ibm-pc`, the launch media type,
and the recipe's config bytes unchanged to binding_slots and compose.
It does not translate the PC grammar.
Bare stored-image media type `application/vnd.unemu.config.v1+toml` is not a new Core input.
The launch decoder needs a template parse stage before its existing resolved checks.
There is no second binding syntax in the outer `firmware`, `media` or `slots` rows.

**ASSUMED:** a template uses bound items only.
Its config is self-contained: no `extends`, host paths, discovery roots or host journal paths.
A host loader may resolve inheritance before entering the codec.
An unresolved `extends` at the Core seam is Unsupported without filesystem access.
Hardware defaults are frozen during composition using the existing image defaulting code.

**ASSUMED:** mixed documents are valid.
For example, a host file can contain a path BIOS and a bound floppy.
The host loader keeps path resolution in its existing ImageReader and adds bound logical keys to that reader.
At the Core seam, a path-form item must already be a logical `assets/...` key with its existing envelope binding and content receipt.
Such an item can sit beside a bound item in the same launch input.
Compose checks both and opens both through ContentSource.
It cannot open a literal host path: a caller must first import that path through the core-owned host adapter.
This is the same distinction between host paths and launch keys that exists today
(**READ**, `crates/pc-config/src/lower.rs:26`, `crates/pc-config/src/image/codec.rs:113`,
`crates/pc-config/src/image/codec.rs:145`).

## Two complete recipe config files

**MEASURED:** both files below are accepted launch-template inputs, exercised by
`crates/pc-core/tests/content_bindings.rs` through `&dyn Core`.
The `machine.` prefix is the existing envelope nesting.
Removing that prefix and the outer envelope gives the corresponding host machine file.
`machine.disks` declares document-owned zero disks.
The recipe repeats their ids and sizes in its approved `disks` declaration.
The front end compares that declaration with ZeroDisk slots, without parsing PC config.

The first file is `open-at.pc.toml`:

```toml
schema = 1
platform = "ibm-pc"

[machine]
schema = 1
disks = [{ id = "system", size = 528482304 }]

[machine.board]
model = "ibm-5170"
display_switch = "colour"

[machine.board.rtc]
start = "2000-01-01T00:00:00"
nvram = "derived"

[machine.cpu]
model = "80286"
rate = "8/13"
fpu = "none"

[machine.memory]
conventional_kib = 640
extended_kib = 1024

[machine.firmware.bios]
use = "bios"
rom = "system-bios"
role = "system-bios"
base = 0xF8000

[[machine.firmware.option_roms]]
use = "vga"
rom = "video-bios"
role = "video-bios"
base = 0xC0000

[[machine.firmware.option_roms]]
use = "ide"
rom = "option-rom"
role = "option-rom"
base = 0xC8000

[[machine.card]]
kind = "vga"

[[machine.card]]
kind = "ide"
controller_model = "ata"
port = 0x1F0
irq = 14

[[machine.card.drives]]
channel = "master"
disk = "system"
geometry = { cylinders = 1024, heads = 16, sectors = 63 }
write = "volatile"

[[machine.card]]
kind = "fdc-765"

[[machine.card.drives]]
unit = 0
type = "1.2M"
empty = true
```

The second file is `open-at-dos.pc.toml`:

```toml
schema = 1
platform = "ibm-pc"

[machine]
schema = 1
disks = [{ id = "system", size = 528482304 }]

[machine.board]
model = "ibm-5170"
display_switch = "colour"

[machine.board.rtc]
start = "2000-01-01T00:00:00"
nvram = "derived"

[machine.cpu]
model = "80286"
rate = "8/13"
fpu = "none"

[machine.memory]
conventional_kib = 640
extended_kib = 1024

[machine.firmware.bios]
use = "bios"
rom = "system-bios"
base = 0xF8000

[[machine.firmware.option_roms]]
use = "vga"
rom = "video-bios"
role = "video-bios"
base = 0xC0000

[[machine.firmware.option_roms]]
use = "ide"
rom = "option-rom"
role = "option-rom"
base = 0xC8000

[[machine.card]]
kind = "vga"

[[machine.card]]
kind = "ide"
controller_model = "ata"
port = 0x1F0
irq = 14

[[machine.card.drives]]
channel = "master"
disk = "system"
geometry = { cylinders = 1024, heads = 16, sectors = 63 }
write = "volatile"

[[machine.card]]
kind = "fdc-765"

[[machine.card.drives]]
unit = 0
type = "1.2M"
inserted = { use = "dos", medium = "disk-1" }
write = "readonly"
```

**ASSUMED:** the recipe selects the corresponding file as its config input.
Both recipes declare `disks = [{ id = "system", size = 528482304 }]`.
Both pin these uses; the DOS recipe additionally pins `dos`:

```toml
[use.bios]
image = "unemu/firmware/ibm-pc/unemu-at-bios-1.0.2-unemu.3:r1"
digest = "sha256:c4c53a6a5c0727d8cccb26b22a92dcbf68e906aed7d9424be80763ea30533ff2"

[use.vga]
image = "unemu/firmware/ibm-pc/vgabios-0.9d-unemu.1-std:r1"
digest = "sha256:c1972247ba5fc03dee5bc011b137e49c547ef1f4fc32dc3f74fa09b205c235e7"

[use.ide]
image = "unemu/firmware/ibm-pc/xub-r638-xtp-ata1f0:r1"
digest = "sha256:4dc25f1c00342e44b2476e84befc9dbbb1817d478cdda9b1fd0a5d8e026026a1"

[use.dos]
image = "unemu/media/ibm-pc/freedos:1.4-1.2m-r1"
digest = "sha256:6508d997b31b3cd673a732cf18591c387dbd3c7e44f5c90f83463af038f1c044"
```

**READ:** these package identities and pins appear in
`unemu-media:recipes/firmware/unemu-at-bios-1.0.2-unemu.3.firmware.toml:13`,
`unemu-media:recipes/firmware/vgabios-0.9d-unemu.1-std.firmware.toml:12`,
`unemu-media:recipes/firmware/xub-r638-xtp-ata1f0.firmware.toml:14`, and
`unemu-media:recipes/media/freedos/freedos-1.4-1.2m.media.toml:14`.
The FreeDOS member `disk-1` is a 1.2M boot disk
(`unemu-media:recipes/media/freedos/freedos-1.4-1.2m.media.toml:84`).
These are package pins, not the flat-content digests supplied to ContentSource.
The examples propose a hardware description; no boot result is claimed.

**ASSUMED — end-to-end:** discovery reports three FirmwareRom occurrences and one ZeroDisk for the first file.
The second adds one Medium occurrence.
The front end verifies the packages and every selected blob in full.
It supplies each item's id, kind, size, SHA-256 and package-local roles.
Compose returns resolved config bytes and their required content set.
The front end stores those bytes and retains the required blobs.
Later create needs only that document, ContentSource and session storage.
The second example deliberately omits the system role assertion: its package must still declare `system-bios`.

## One source per item

**ASSUMED:** a ROM accepts either the existing `image`/`parts` source or `use` plus `rom`.
A floppy accepts its existing path/catalogue forms or `inserted = { use, medium }`.
A catalogue entry may likewise use `{ use, medium }` instead of `image`.
An IDE drive accepts its existing path source, `disk = "id"`, or a bound medium using the same `{ use, medium }` selector.
The controller still fixes whether a medium is removable.
Do not add digest-only selectors or positional package references.
Bindings initially select complete ROM items; chip-lane declarations remain the existing path form.

**ASSUMED:** each source selector is exclusive at that occurrence.
Reject incomplete use/item pairs and path-plus-binding combinations.
Reject a drive combining `disk`, `image`, single bound insertion or a catalogue.
Existing catalogue selection rules still apply.
Optional `size` and `sha256` on a bound selector constrain the facts.
ROM `role` is an assertion; it does not change the slot.
An option ROM still requires a base.
A system BIOS base, when supplied, must equal `0x100000 - mapped_size`.
Offsets retain the existing whole-file digest and mapped-suffix semantics.
Check arithmetic and existing placement limits before opening bytes.
The present BIOS placement rule is in `crates/pc-config/src/lower.rs:442` (**READ**).

**ASSUMED:** zero declarations have unique nonempty ids and positive sizes.
Each must be attached exactly once; reject missing, unused or multiply attached ids.
Use the existing safe medium-id spelling for these ids.
Geometry uses checked multiplication: `cylinders * heads * sectors * 512 == size`.
For the examples this is `1024 * 16 * 63 * 512 == 528482304`.
The ATA limits and accepted floppy lengths remain the shared assembly rules.
Do not infer geometry for a declared zero IDE disk.
Existing image geometry checks already use checked multiplication
(**READ**, `crates/pc-config/src/image/codec.rs:346`).

## Prepared representation and storage policy

**READ** (`crates/pc-config/src/launch.rs:5`, `crates/pc-config/src/launch.rs:70`):
a stored head digest may name an object rather than the flat bytes.
A launch content row maps that digest to flat SHA-256 and size.
The earlier launch policy forbade storage. Portable prepared launches now
have `encode_portable` / `decode_portable` codec entry points; kept-record
names remain transient. The contract requires the prepared document to be storable
(`unemu-core-api:src/binding.rs:173`).

**ASSUMED — proposal:** PreparedConfig contains the existing launch representation.
It does not contain the old stored-image envelope alone or a new third wire form.
For raw package ROMs and media, set `blob`/`head` to the flat SHA-256 digest.
Add the corresponding content rows.
Preserve an existing layered head and its verified flat receipt in mixed resolved inputs.
Never replace a layered head identity with its flat digest merely to simplify composition.
The document contains logical keys, size/digest identities, resolved hardware, media policy and record names.
It contains no `use`, item selector, package manifest digest, host path or reader.
Recipe provenance and source/licence attachments stay with the front end.
They are not runtime content and cannot be inferred from BindingFacts.

**ASSUMED:** generate logical keys from occurrence positions, such as `assets/firmware/bios.bin`.
Give distinct occurrences distinct keys even when they share bytes.
Use declared zero disk ids as their medium ids.
Use deterministic position-derived ids for package media.
Reserve the generated namespace against collisions with preserved resolved rows.
ROM bindings keep the existing `key`, `source` and `blob` fields.
Resolved drives remain connected through the existing `slots` and `media` tables.
Do not add another image lookup to the assembler.

### Native zero disks

**READ** (`crates/pc-config/src/image/mod.rs:191`,
`crates/pc-config/src/image/codec.rs:335`, `crates/pc-config/src/image/codec.rs:503`,
`crates/pc-config/src/launch.rs:292`):
before the native-zero extension every medium required a head and content row.
Every medium lowered to ImageSource::File and entered the mount's content-base map.
That could not represent a ZeroDisk without inventing external content.

**ASSUMED — smallest extension:** make a medium's source identity exactly one of
the existing `head` or `zero = { size = N }`.
Keep its id, logical source key, policy, geometry and slot unchanged.
Do not add a zero content row, a zero package item, or a generated blob to Validated.
A prepared zero medium has this shape:

```toml
[[media]]
id = "system"
source = "assets/disks/system"
zero = { size = 528482304 }
policy = "volatile"
geometry = { cylinders = 1024, heads = 16, sectors = 63, sector_bytes = 512 }

[[slots]]
controller = "ide"
unit = 0
catalogue = ["system"]
catalogue_mode = false
inserted = "system"
```

**READ:** preparation removes template `machine.disks` and drive selectors after creating these rows.
The resolved hardware contains drive wiring and geometry, as today.
The codec lowers a native zero medium to ImageSource::Blank, outside the external-content map.
Assembly resolves fixed-disk blanks to verified zero sector bases before its
byte-materializing fallback; merely constructing a Blank is insufficient.
Supply its immutable sectors through the existing SectorSource and SparseOverlay interfaces.
All write modes use that overlay boundary.
Readonly rejects writes; volatile discards replacements; journal persists replacements.

**READ** (`crates/assembly/src/image.rs:28`, `crates/assembly/src/assemble.rs:938`,
`crates/assembly/src/assemble.rs:1339`): Blank already exists but may allocate a full byte vector.
Reusing its name alone is insufficient.
VerifiedBase already accepts an immutable sector source
(`crates/journal/src/overlay.rs:42`).

**ASSUMED:** add a bounded-memory zero SectorSource and route native zero mounts through it.
Its base identity uses the actual SHA-256 of exactly N zero bytes.
Compute that identity with a bounded streaming buffer when constructing the runtime base.
Do not hash a descriptor, sample sectors, skip hashing large disks or allocate N bytes.
This is hashing generated bytes, not rehashing supplied content receipts.
Composition and discovery need no zero-content open or generated base hash.
The complete generated source is fixed by size; its runtime SHA-256 is derived internally.
Computing that hash is linear work; its cost is not measured here.

**ASSUMED:** journal headers bind that full base hash and length.
Written sectors stay separate from the base in every mode.
Commit/export produces a new base and never rewrites the old one.
Use one overlay interface so a hash-linked chain can later replace the base.
This preserves the present requirement in `backlog/docs/deferred.md:170` (**READ**).
It does not implement layer recipes, deduplication or layer merging.

### What may now be stored

**ASSUMED:** replace the blanket “never stored” launch policy with a portable subset.
A compose result is storable as the machine image's config blob in the launch media type.
It has only document-owned media and deterministic record names such as `system.upfj`.
Journal contents and storage handles are never embedded.
A new session supplies a new storage namespace for those names.
Launch documents augmented with session-kept media or kept-record names remain transient.
The front-end store must dispatch by config media type and retain the listed runtime blobs.
It must not pass a prepared launch document to the old Config decoder.
For package-only templates, those identities name flat blobs.
For a preserved layered input, retain its existing head/layer object graph too.
The front end exposes the resolved flat view through the listed identity.
Create still requests only that identity; descriptor traversal stays inside ContentSource.
That store migration is a coordinated front-end change, not an implementation in this repository.
Old stored-image configs and old transient launch documents remain supported.

## Role and catalogue checks

**READ** (`crates/assembly/src/firmware.rs:23`): the catalogue distinguishes
SystemBios, VideoBios and OptionRom.
Its entries also carry placement, scan windows, services, board requirements and CMOS layout
(`crates/assembly/src/firmware.rs:281`).
These are compatibility facts, not package permissions.

**ASSUMED — role vocabulary:** recognise exactly `system-bios`, `video-bios` and `option-rom` for ROM binding assertions.
Unknown requested roles are Config errors.
Extra unrequested package roles do not grant another role or need interpretation.
Do not invent floppy, hard-disk or CMOS package roles in this change.
Medium kind, attachment, size and geometry establish their compatibility.

| Hardware position | Required package role, even when `role` is omitted |
| --- | --- |
| `firmware.bios` | `system-bios` |
| Option ROM in the separate video window, C0000h through C7FFFh | `video-bios` |
| Other expansion ROM position, unless the reviewed catalogue identifies video firmware | `option-rom` |

**ASSUMED:** a recognised catalogue VideoBios outside the video window still requires `video-bios`.
Existing scan and placement checks decide whether that placement is runnable.
An explicit role must agree with that effective classification and any reviewed catalogue kind.
Require the exact effective role in the selected BindingItem.roles.
Do not treat `option-rom` as a wildcard for video firmware.
Do not infer a role from an item's id, package labels or its digest.
BindingSlot.role remains None when the document omits the assertion.
The implicit requirement is enforced internally.
This distinction is required by the API
(**READ**, `unemu-core-api:src/binding.rs:33`, `unemu-core-api:src/binding.rs:92`).

**READ:** the firmware package model has a role per ROM and an optional catalogue id
(`unemu:crates/image-store/src/firmware.rs:31`).
The shipped BIOS recipe declares `role = "system-bios"` and a catalogue id
(`unemu-media:recipes/firmware/unemu-at-bios-1.0.2-unemu.3.firmware.toml:53`).
The VGA and XTIDE recipes declare `video-bios` and `option-rom`
(`unemu-media:recipes/firmware/vgabios-0.9d-unemu.1-std.firmware.toml:52`,
`unemu-media:recipes/firmware/xub-r638-xtp-ata1f0.firmware.toml:54`).
Their present `[package.fits]` tables declare boards and machine classes, not per-item role lists
(BIOS recipe at line 29; VGA at line 28; XTIDE at line 30).

**ASSUMED:** the front end extracts roles faithfully from that bound package's item declaration or item-scoped package catalogue entry.
If a future package fits form declares item-specific roles, preserve that item scope.
Package-wide compatibility labels must not be copied to every ROM as role permissions.
The core sees only the resulting BindingItem.roles.
It does not parse packages or authenticate the opaque manifest digest.
It checks roles before deduplicating shared content identities.
A second package with the same SHA-256 cannot repair a missing role in the selected package.

**READ** (`crates/assembly/src/firmware.rs:817`, `crates/assembly/src/firmware.rs:933`):
video scan classification already uses placement and catalogue kind.
The run-only unlisted-firmware option relaxes membership alone.
Headers, checksums and compatibility still apply.
The option is not part of machine identity (`crates/assembly/src/firmware.rs:16`).

**ASSUMED — recommendation:** compose retains the baseline strict catalogue policy.
A package role does not admit an unlisted ROM or establish a BIOS scan policy.
For a standard PC board, reject an unlisted bound ROM as Unsupported.
No compose argument can carry the current run-only escape.
A maintainer must add the reviewed catalogue entry before using that ROM in a template.
Keep existing host-file and explicit create escape behaviour unchanged.
Do not add a persistent permission flag or guess scan bounds.

**ASSUMED:** known firmware keeps its catalogue scan windows, CPU/board requirements,
ROM-container offset, services and CMOS layout.
Check supplied `provides` against catalogue services as today.
For option ROM bytes, retain 55AAh signature, nonzero declared length, checksum,
alignment, overlap and scan-range checks.
The existing model cites IBM Personal Computer AT Technical Reference,
System BIOS, “Adapters with System-Accessible ROM Modules”
(**READ**, `crates/assembly/src/firmware.rs:790`, `crates/assembly/src/firmware.rs:900`).
Roles never bypass these checks.

## Discovery, composition and diagnostics

**ASSUMED:** use one binding planner in pc-config for all entry points.
Its config pass parses the document and records constraints without content access.
Its facts pass produces logical-key mappings and resolved envelope rows.
Then use the existing launch check/lower path and ImageReader in lower.rs.
Do not add a second ROM resolver, geometry validator or assembler.
Validate/create on an unresolved template return Config: composition is required.
They never silently scan for a missing bound item.

| Diagnostic | binding_slots | compose before any open |
| --- | --- | --- |
| Malformed selector, path plus binding, unknown requested role | Config, at the conflicting fields | Same check |
| Missing/duplicate zero id, unused zero declaration, invalid geometry or size | Config, at declaration and attachment | Same check |
| Unbound use; selected item missing | Cannot know package availability | Config naming slot, use and item |
| Unused supplied use | Cannot know supplied uses | Config naming use; no fabricated config position |
| Duplicate use ids; empty names or manifest digest | No facts | Config naming the offending fact |
| Duplicate item ids within a use, including unselected duplicates | No facts | Config: ambiguous id; duplicates across different uses are allowed |
| Wrong item kind; size/digest conflict | Check syntax and document-only constraints | Config against the selected item's facts |
| Role omitted by the selected package | Enforce positional assertion consistency only | Config, including implicit system/video/option role |
| BIOS base versus size; medium size versus geometry | Check when document gives the size | Config using the fact size before opening bytes |
| Extra items in a used package | No facts | Allowed |

**READ** (`unemu-core-api:src/core.rs:139`, `unemu-core-api:docs/conformance.md:379`):
all config/fact consistency checks must precede content opens.
Discovery allows only Config and Unsupported.
Compose additionally allows Content and Storage.

**ASSUMED:** first reject unsupported operation/platform/media type.
Then finish the complete config and facts passes, including checks on unused facts.
Run resolved config-only validation before opening anything.
Open every distinct required `(sha256, size)`, including uninserted catalogue media.
Check each returned reader length; trust the full frontend receipt.
Read only what the shared lowerer needs for firmware and CMOS checks.
Return Content for missing/mismatched receipts or detected changes.
Return Storage for read I/O.
Return Unsupported for correctly received ROM bytes that are malformed or unsupported.
Internal out-of-bounds parser reads must not escape as Usage.
Apply the same byte-dependent classification to ordinary create.
No storage acquisition, machine creation, base writes or virtual-time changes occur in compose.

**READ — implementation gap:** current lowering hashes images and catalogue identification hashes ROM bytes
(`crates/pc-config/src/lower.rs:288`, `crates/assembly/src/firmware.rs:1023`).
The current Core adapter maps most lowering errors to Config
(`crates/pc-core/src/core.rs:214`).
Calling these unchanged would violate the new receipt/error contract.

**ASSUMED:** carry the trusted full identity from the launch ImageReader into shared ROM/catalogue checks.
Match reviewed entries by that identity and length.
Continue reading bytes for header, checksum and layout validation.
The host-file reader still establishes identity by hashing every byte.
Use an explicit trusted identity path in the shared helpers; do not disable pins globally.
Preserve reader error kinds through the adapter, following ContentCache's existing error capture
(**READ**, `crates/pc-core/src/mount.rs:131`).
This is part of implementation acceptance, not an optional optimisation.

### Source positions

**READ** (`crates/pc-config/src/origin.rs:17`, `crates/pc-config/src/origin.rs:159`,
`crates/pc-config/src/source.rs:98`, `crates/machine-file/src/origin.rs:15`):
Origins already carries defining files and one-based lines.
The recursive walk covers array members and nested values.
Lookup falls back to the closest enclosing field.
The YAML adapter supplies the same representation.
The public Diagnostic contains a field path and message, not a line field
(`unemu-core-api:src/error.rs:78`).

**ASSUMED:** reuse that walk and Origins for the added fields.
For Core input, record positions from the original launch text, before extracting or serialising `machine`.
Use a synthetic document name such as `config.toml`, since ConfigDocument carries no filename.
Store envelope-prefixed paths such as `machine.firmware.bios.rom` in Diagnostic.path.
Put `config.toml:line`, slot, use and item in its message.
Host loading retains the actual defining-layer origin through inheritance.
Preserve the original-to-generated field map while lowering so ROM byte errors point back to the selector.
Do not report the rewritten `bundle.toml` position as if it were the user's source.
Fact-only errors use a fact name in the message and an empty config path.
Report both sites for conflicts where both have origins.

Example message shape (**ASSUMED**, illustrative line):

```text
config.toml:31: firmware.bios: use "bios", rom "video-bios":
package item does not declare required role "system-bios" (implied by position)
```

## Stable slots and bytes

**READ** (`unemu-core-api:src/binding.rs:18`, `unemu-core-api:src/binding.rs:153`):
slot ids are stable config-local occurrence ids.
Slots are sorted by Rust string ordering.
Facts use/item/role order has no meaning.

**ASSUMED:** ids follow hardware positions, without the envelope's `machine.` prefix:

| Occurrence | Id |
| --- | --- |
| System ROM | `firmware.bios` |
| Expansion ROM | `firmware.option_roms[0]`, `firmware.option_roms[1]` |
| Single bound or zero drive | `card[1].drives[0]` |
| Bound catalogue member | `card[2].drives[0].media[0]` |

**ASSUMED:** array indexes are zero-based and rendered without padding.
They refer to the supplied resolved document's array positions.
Reordering the config may change ids; reordering facts cannot.
Repeated references to one package item have distinct occurrence ids.
Sort the final ids lexicographically: `[10]` sorts before `[2]`.
Do not use source line numbers, labels, digest order or card ids as occurrence identity.
Media SlotId values remain the existing controller/unit ids; they are not BindingSlot ids.

**ASSUMED:** ZeroDisk slots have no use_id, the declared disk id as item_id,
the exact size, and no role or digest assertion.
Each attachment yields its occurrence slot.
Prepared documents report no package slots and may report native zero attachments with those same ids.
Path items produce no package slot.
Their resolved content rows still contribute to validation.

**ASSUMED:** canonical output preserves hardware/card/drive/catalogue array order.
Sort facts for lookup and diagnostics, not for hardware reordering.
Freeze defaults, strip binding syntax and template declarations, and use the existing canonical launch encoder.
Emit firmware bindings and media in deterministic hardware occurrence order.
Emit one content row per referenced head/blob identity in deterministic first-occurrence order.
Deduplicate Validated by `(sha256, size)` with a fixed label policy.
Reject one SHA-256 associated with conflicting flat lengths.
Emit record rows in medium order with fixed retention defaults.
Never serialize fact order, package role order, host paths, timestamps or randomness.
Equal config and facts therefore give identical prepared bytes and content lists.
Zero identity derivation does not enter the prepared serialization.

**READ** (`crates/pc-config/src/launch.rs:196`, `crates/pc-config/src/launch.rs:259`):
launch encoding already has a canonical round trip.
Launch checking already deduplicates the required identity set.

## Existing features and create

**ASSUMED:** preserve floppy catalogue ordering, insertion and write policy.
Bind every catalogue entry, including entries not inserted at power-on.
Repeated references to one writable medium obey the existing no-sharing rule
(**READ**, `crates/pc-config/src/image/codec.rs:329`).
ROM catalogue membership and package role permission remain independent checks.
Unknown bound firmware does not gain a scan policy from its package label.

**ASSUMED:** derived or blank CMOS stays generated.
The effective firmware catalogue entry still selects its CMOS layout.
An existing logical-key CMOS image and its receipt remain usable in mixed inputs.
Keep 64-byte validation and existing NVRAM journal behaviour.
Do not introduce a CMOS package binding kind.
Core create already lowers firmware/CMOS and maps NVRAM journal bases separately
(**READ**, `crates/pc-config/src/launch.rs:279`, `crates/pc-config/src/launch.rs:296`).

**ASSUMED:** a bound disk with `write = "journal"` gets a stable generated medium id
and a document-owned `<id>.upfj` record row.
No host `journal` path is accepted for a new bound Core selector.
Use existing keep_generations validation and session storage for its record.
Only create opens that storage.
The immutable external base uses its full received SHA-256; a zero base uses its generated full SHA-256.
The record remains separate from the base.
Existing records already validate names, ownership and retention
(**READ**, `crates/pc-config/src/launch.rs:391`).

**ASSUMED:** create decodes the prepared launch document and obtains its required set through the shared check.
ContentCache opens exactly that set, never a package id, a manifest or a host path.
Lower through the existing logical-key ImageReader and mount the existing sector overlays.
Native zero sources add no ContentSource request.
Ordinary content/storage failures can still prevent creation after successful composition.
This extends the present check/cache/lower/mount sequence
(**READ**, `crates/pc-core/src/core.rs:140`, `crates/pc-core/src/mount.rs:136`).

**ASSUMED:** describe_media on prepared output uses resolved drive and medium geometry.
It reports the same runtime SlotId, size and label as create.
A zero IDE disk has its exact declared size.
A selected DOS floppy reports its accepted medium size, not an assumed drive maximum.
No bytes are opened.
The existing implementation already derives these descriptions from launch rows
(**READ**, `crates/pc-core/src/export.rs:23`).

**ASSUMED:** raw, fixed-VHD and machine exports keep taking current sectors through ExportRequest readers.
They do not resolve package bindings again or mutate bases.
Machine export writes ordinary path-based schema-1 media references to the new outputs.
It exports actual current sectors for a zero base too.
It does not need to preserve recipe use names or zero shorthand.
The current export formats and slot selection are in
`crates/pc-core/src/core.rs:38` and `crates/pc-core/src/export.rs:123` (**READ**).

**ASSUMED:** old path documents, stored configs, transient launches, ROM parts and offsets retain their behaviour.
Old documents gain no implicit package requirements.
Unsupported binding capability remains the default until both operations and their tests land.
Advertise content_bindings only with both methods implemented.
Coordinate the released contract pin and literal implements version with release preparation.
Do not commit a local patch-generated Cargo.lock change as a dependency update.
The migration requires that coordinated release step
(**READ**, `unemu-core-api:docs/migration-0.6.md:3`).

## Implementation handoffs and estimate

**ASSUMED:** the supervisor can assign the following sequential deliverables.
The grammar/planning and fact-resolution steps are implemented. The native
zero and core-operation steps now have implementation and contract tests.
Receipt propagation and the later integration work remain separate.
Each step keeps the existing path and logical-key lowering as the single resolution route.

| Step | Files | Tests and acceptance | Estimate |
| --- | --- | --- | --- |
| Grammar and positioned planning | `crates/pc-config/src/schema/v1.rs`, `schema/v1/projection.rs`, `schema/v1/values.rs`, `image/encode.rs`, `origin.rs`, a new `bindings.rs`; `crates/machine-file/src/origin.rs`, `resolve.rs`, `paths.rs` | TOML/YAML parsing, source exclusivity, inheritance positions, declaration merging and stable slots. Both complete examples discover without I/O. Path-only fixtures are unchanged. | 2 days |
| Fact validation and logical-key resolution | `crates/pc-config/src/bindings.rs`, `image/mod.rs`, `image/codec.rs`, `lower.rs`; `crates/machine-file/src/lib.rs`, `source.rs`, `firmware_resolve.rs` | Every diagnostic row above. A counting source records zero opens for every config/fact rejection. Mixed path/bound files lower through one reader. Same digest in two packages never shares role permission. | 2–3 days |
| Trusted ROM identities and error mapping | `crates/pc-config/src/lower.rs`, `image/codec.rs`; `crates/assembly/src/image.rs`, `firmware.rs`, `assemble.rs`; `crates/pc-core/src/core.rs`, `mount.rs` | Received identities replace core rehashing. Existing host hashing and ROM checks still work. Wrong receipt/size is Content; read I/O is Storage; correct-receipt bad header/checksum is Unsupported. No bypass of placement or catalogue checks. | 2–3 days |
| Native zero source and overlay mounting | `crates/pc-config/src/image/mod.rs`, `image/codec.rs`, `launch.rs`; `crates/assembly/src/mount.rs`, `assemble.rs`; `crates/journal/src/overlay.rs`; `crates/pc-core/src/mount.rs` | Head XOR zero, size/geometry rejection and no zero content row/open. First/last sector reads, sparse writes, readonly, journal reopen and full zero-stream hash. Memory grows with replacements, not disk capacity. Base bytes remain unchanged. | 2–3 days |
| Core operations and prepared persistence contract | `crates/pc-core/src/core.rs`, a small binding adapter; `crates/pc-config/src/launch.rs`, `image/defaults.rs`; `crates/pc-core/tests/bindings.rs` | Drive as `&dyn Core`. Permuted facts give identical bytes. check_prepared passes. validate identity set equals compose's. Create succeeds with only those identities. Prepared native zeros remain discoverable. No storage open or machine execution in compose. | 2 days |
| Integration, export and documentation | `crates/pc-core/tests/export.rs`, binding tests; `crates/machine-file/tests/`, `crates/pc-harness/tests/`; relevant docs under `backlog/docs/` | Old documents regressions. Prepared describe_media parity. Bound DOS boot matches the path-based golden at the same CPU pin. Zero disk guest writes persist only by policy. All export formats round-trip current sectors. Mixed catalogues and CMOS/NVRAM are covered. | 2 days |

**ASSUMED:** total 12–15 engineering days, about 3 working weeks with review.
Expect roughly 1,000–1,600 production lines and 1,200–1,800 test lines across the steps.
The receipt propagation and zero mount are the main uncertainties.
The coordinated front-end store/media-type migration is additional work.
No CPU, device register model, new external crate or layer-chain implementation is expected.

**ASSUMED:** write regression tests before changing each behaviour.
Record an observed failure against the original behaviour or an explicit mutation.
For new entry points, asserting Unsupported before implementation proves the missing capability;
it does not prove the individual diagnostic branches.
Use targeted mutations for role borrowing, early opens, zero allocation and identity-set omissions.
Hardware integration tests use open fixtures; selector/role tests use synthetic bytes.
Retain existing golden CPU pins and run the implementation gate appropriate to each step.
This design-only change adds no tests and claims no runtime acceptance result.

## Questions for the maintainer

1. **May compose's portable launch subset become a stored machine config?**
   Recommended: yes, under its existing launch media type.
   Keep session-augmented launches transient.
   This requires coordinated front-end config dispatch and replaces the blanket “never stored” policy.
2. **May template composition require reviewed catalogue firmware?**
   Recommended: yes.
   Package roles are necessary but do not define safe scan or CMOS policy.
   Keep the existing run-only escape for explicit legacy launches.
3. **Is positional video-role enforcement acceptable without an explicit role?**
   Recommended: require `video-bios` in the separate video window, `option-rom` in the extension window,
   and always `system-bios` at reset firmware.
   Never accept a package's broader label as a substitute.

Related: [core contract](core-contract.md), [machine files](machine-files.md), [persistent media](persistent-media.md), [deferred layers](../deferred.md#layered-disk-images-base-image-plus-content-addressed-layers), unemu-core-api `docs/decisions/0002-content-bindings.md`, `docs/conformance.md`, `docs/migration-0.6.md`.

## Implemented first-scope details

The core now advertises both binding operations for its launch media type.
`launch::prepare` performs config and fact checks, resolves logical keys,
freezes defaults, preserves existing heads and receipts, and writes native
zero rows. `PcCore::compose` opens the resulting identity set and checks small
firmware bytes before returning the owned portable document. No machine or
storage handle is acquired during composition. Bound firmware must match a
reviewed board-catalogue entry. Receipt propagation into ROM and CMOS image
lowering remains deferred; the existing digest checks of small ROM bytes
and supplied CMOS images (including journal-backed CMOS) still run.

The portable launch codec accepts document-owned record names and refuses
session-kept record names. The front-end store still needs its coordinated
media-type dispatch change in its own repository; it must use this launch
codec rather than the old stored-image Config decoder. This workspace has
no front-end image store to migrate.

Zero media require exactly one positive `zero.size` or external `head`,
matching geometry. Native zero media currently attach to IDE drives.
`VerifiedBase::zero` hashes the complete generated stream with a fixed 64 KiB
buffer at runtime. Volatile and readonly mounts use sparse replacements;
journals bind the generated full hash and length and replay replacements on
reopen. No disk-capacity buffer, content row, receipt or Validated identity
is generated for zero media. The explicit host binding loader still reports
Unsupported for zero selectors; the prepared launch codec is the native-zero
entry point in this scope.

Raw, fixed-VHD and machine exports work unchanged through current-sector
readers. Their tests cover a native-zero base with modified current bytes;
exports write new outputs and do not preserve the zero shorthand.

Contract tests cover both complete examples, fact permutations, config-only
rejections without opens, receipt/error kinds, portable round trips, preserved
layered heads and an open AT POST banner through the existing text observation.
Sector and mock-bus tests cover sparse writes, readonly policy, large declared
capacity, the complete generated SHA-256 and journal reopening.
