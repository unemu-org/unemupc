---
id: decision-30
title: Published images declare the oldest unemupc that runs them
date: 2026-11-17
status: Accepted
deciders: Mikołaj Mikołajczyk
---

# decision-30 - Published images declare the oldest unemupc that runs them

## Status

Accepted — 2026-11-17 by the maintainer. Design step of
`upc-dfs.15`; it must be settled before the [unemu front end](https://github.com/unemu-org/unemu)'s `image template create`
(`upc-dfs.14`) publishes its first template. Builds on [decision-25 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-25%20-%20Machine%20images%20are%20self-contained%20OCI%20artifacts%20in%20a%20local%20store.md)
(machine images), [decision-26 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-26%20-%20Installation%20media%20sets%20are%20their%20own%20OCI%20artifact%20type.md) (installation media sets) and
[decision-27 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-27%20-%20Firmware%20packages%20are%20their%20own%20OCI%20artifact%20type.md) (firmware packages). The survey of today's readers and the
measured behaviour of release 0.0.2 are in
[`backlog/docs/design/machine-images.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md) §5.6.

**Note, 2026-11-19 (amendment to come).** The maintainer decided, with the
stage B plan of `backlog/docs/design/core-contract.md` (§14.6 Q6, §9),
that before the repositories separate this decision is amended to
re-state the marker as naming the **core's** release
(`CoreDescription::version`). Two things need that amendment: the level a
build compares with is the store crate's own package version, which after
the split is the front end's; and the per-feature table mixes rows the
core reads (the PC config's keys and vocabulary) with rows the front end
reads (envelope media types, compressed layers, media-set and
firmware-package keys). Until the amendment the decision stands as
written; every version is the one workspace's today, so nothing is wrong
yet.

## Context

Templates, installation media sets and firmware packages published on
`registry.unemu.dev` stay there for good, and every release reads them,
older ones included. Nothing in an image says which release it needs.

Every reader fails closed. Manifest and descriptor fields are strict, and so
is each config codec: unknown keys are refused, and so are unknown board,
CPU, card kind, drive type, media format and firmware role values, and system
BIOS or option ROM bytes the release's firmware catalogue does not list.
Unknown annotations are ignored. With a newer image, an older release
therefore:

- refuses it before downloading anything when a media type is new. The
  message names the media type, not a release, and Get templates silently
  leaves out the whole repository;
- downloads the whole closure and then refuses it when a config key, a
  device kind or a firmware catalogue entry is new. It tags nothing and
  shows a decoder message such as "unknown variant `future-device`". In a
  store a newer release shares, the library lists such a machine image as
  if it were fine, and New machine from image… makes a machine that cannot
  start;
- would run it wrongly only if a later release changed what an existing key
  or value means, or if the image depended on an emulation fix.

Releases 0.0.1 and 0.0.2 are out and cannot change. They read images the
same way. Six published artifacts must keep their digests: the three
`unemu/media/freedos` sets (`1.4`, `1.4-1.2m`, `1.4-720k`) and the three
firmware packages (`unemu-at-bios-1.0.2-unemu.3:1`,
`vgabios-0.9d-unemu.1-std:1`, `xub-r638-xtp-ata1f0:1`).

## Decision

1. **The marker is a manifest annotation.** `unemu.min-app-version` holds a
   SemVer 2.0.0 release version (`MAJOR.MINOR.PATCH`, with no pre-release or
   build part). It names the oldest release of the program that provides the
   manifest's `unemu.platform` and can read and run the artifact: unemupc for
   `ibm-pc-at`, whose desktop and command line now live in
   the [unemu front end](https://github.com/unemu-org/unemu). It follows the `unemu.*` keys already in use. The manifest
   alone carries it, so Remotes and Get templates can grey out a row without
   fetching a config. Machine images, installation media sets and firmware
   packages all use the same rule.
2. **No marker means no requirement.** The six published artifacts stay
   as they are. Release 0.0.1 can read all of them.
3. **The marker is a hint, never an authority** (machine-images §2, the
   annotation rule). Readers stay strict and fail closed. The marker lets a
   release refuse early and give a reason a person understands. It is not
   a config field: the config is the machine's identity, and a corrected
   marker must not change machine digests.
4. **Media-type majors move only for a decoder break**, never for each new
   feature. No release changes what an existing key or value means. It adds
   a new key or value instead, which 0.0.1 and 0.0.2 refuse. A change they
   would read silently and wrongly is not allowed.
5. **A per-feature table computes the marker.** The shared store crate and
   the provider record, for each feature they read, the release that first
   reads it. Features include an optional config key, a vocabulary value
   (board, CPU, FPU, card kind, drive type, controller, media format, extra
   kind, firmware role, fit board or class), a payload media type, a
   firmware catalogue entry and an accepted range that grows. Every writer
   stamps the maximum over what the artifact uses: New machine…, Save as new
   image…, import, media and firmware import, and template build and
   create. When that maximum is the baseline 0.0.1, the writer leaves the
   annotation out, so rebuilding an artifact that older releases already
   read gives the same bytes and digest. A recipe, or `--min-app-version` on
   the importing commands, may raise the value for behaviour the table
   cannot see, such as an emulation fix a title needs. It may never lower
   it: a stated value below the computed one is refused. Template creation
   and the push guard of `unemu/` compute the value again and refuse a
   missing or lower marker. Metadata never raises it: new catalogue fields
   go into metadata schema 2, which older releases show read-only.
6. **Readers check the marker first.** A lenient read of the manifest's
   annotations comes before the strict decode, so a future envelope or
   artifact type still produces the message. A release meets a requirement
   when it is no higher than the release's level: its package version, or
   the newest version in its own table if that is higher, so a development
   build reads what it writes. A value that does not parse counts as unmet,
   and the message is "needs a newer unemupc".
7. **The image is disabled, not hidden.** These places all show the same
   reason, "Needs unemupc X or newer": the library's machine images,
   installation media and firmware packages, machines made from such an
   image, Remotes, Get templates, the media picker and the command line.
   They disable every action that needs the config: Start, New machine from
   image…, Insert…, Use in machine…, run and machine new. Details from the
   manifest and Delete tag stay. Get templates offers the newest version
   this release can run, which is `latest`'s version when that one
   qualifies, and says which newer version needs which release. A
   repository with no version this release can run is dimmed. **Pull
   refuses before it fetches a blob.** A local tag is written only for a
   closure the release has validated (machine-images §3.1), and a release
   cannot validate a config it does not read.
8. **0.0.1 and 0.0.2 ignore the marker** (measured on 0.0.2 with unknown
   annotation keys; 0.0.1 reads images with the same code). For them, an
   image that needs a newer release still fails at pull, after the
   download, with the decoder's message, and nothing is tagged. This is
   accepted for preview releases. Templates are small. The untagged blobs
   stay in the store until the planned GC, and a pull by a newer release
   reuses them.

## Consequences

- Each new feature adds one table row in the change that introduces it.
  Exhaustive matches over the vocabulary enums make a missing row a compile
  error. Tests fail when an optional key, a firmware catalogue entry or a
  payload type has no row.
- The summary cache records the requirement and its schema moves up. An
  older release that reads the newer cache rebuilds its own, as it already
  does after any cache change.
- The library and Get templates need a disabled-with-reason row state for
  machine images and firmware packages. Media sets already have one
  ("CD-ROM emulation not available yet").
- Requiring a maximum version (dropping support for something) is out of
  scope. It would be a media-type major with a migration.

## Alternatives considered

- **The writing release's own version.** Simple, but too strict, and it
  breaks reproducibility. Every rebuild with a newer front end changes the
  manifest and its digest. The pinned digests of the published sets and
  packages would no longer match, and an immutable tag would refuse the
  rebuilt bytes.
- **Media-type majors as the marker.** 0.0.2 already refuses a new config
  major before it downloads anything. But the message names no release, and
  0.0.2's Get templates hides the repository, older versions it could run
  included. Using majors as feature levels would also give every new device
  its own codec version, and every release would need to decode every one
  of them.
- **A config field.** 0.0.2 refuses an unknown config key only after the
  download (measured: "unknown field `min_version`"). Remotes would have to
  fetch every config, and the marker would become part of the machine
  digest.

## Amendment (Accepted, 2026-11-21)

Accepted by the maintainer on 2026-11-21, with the new marker named
`unemu.min-frontend-version` and the extraction baseline frozen at the split.
The decision above remains in force until this amendment is implemented.
This addresses the pre-extraction question in core-contract §§9 and 14.6 Q6.

**Two release domains.** `unemu.min-app-version` keeps its spelling and
names the oldest **core** release that reads and runs the artifact for
`unemu.platform`: for the PC, unemupc, described by
`CoreDescription::version`. It never names the contract version or the
front end's package version. Recommend a second manifest-only annotation,
`unemu.min-frontend-version`, for the oldest front-end release that reads
the artifact's envelope and front-end-owned config. Check both leniently
before strict decoding or blob download; either unmet requirement disables
the artifact with a reason naming the component. Missing means no
requirement; malformed means unmet, independently for each annotation.
Unknown platforms remain unsupported, never implicitly compatible.

| Owner and release domain | Rows (current names) |
|---|---|
| Core: `pc-config`, with hardware facts from `assembly` | `MACHINE_KEYS`; machine vocabulary matches (board, CPU/FPU, cards, controllers, drives, channels, write policy, lanes, firmware services, display switch, IRQ/DMA, RTC/NVRAM); widened hardware ranges; `CATALOGUE` entries the core accepts. Accepted platform ids/aliases also need a core-domain rating. |
| Front end: `image-store` | Artifact/config/payload media types, `COMPRESSED`, `RECIPE_TYPES`, descriptor redistribution values, store routing aliases (`PLATFORMS`), `MEDIA_SET_KEYS`, `EXTRA_KEYS`, `FIRMWARE_KEYS` and their shared vocabulary. |
| Front end: PC provider (`pc-image`) | `PAYLOADS` (including disk base/sector compression), `MEDIA_FORMAT_KEYS`, `MEDIA_FORMATS`, `FIT_KEYS`, `ROLES`, `FIT_BOARDS`, `FIT_CLASSES`, `LANES`: these decode packages and prepare content, rather than configure the running core. A lane in a machine config remains a core row. |
| Front end: `recipe` and catalogue metadata | Recipe work-file keys/types use the front-end requirement. `METADATA_SCHEMAS` and `TITLE_FIELDS` remain front-end rows for read-only catalogue fallback; they raise neither runnable-artifact marker. |

**Levels and writers.** Compare core requirements with the selected
platform's core description version, raised to that core's newest table
row for a development build. Compare front-end requirements with the
composing front-end application's version, raised only to its newest
front-end row. Inject both levels; a neutral store must neither link a PC
core nor infer the application's version from its own package. Separate
`NEXT_RELEASE`/`NEWEST_ROW` values and coverage checks in each domain.
Never take a maximum across the two domains. Compute and stamp each
requirement independently, omit its baseline, and preserve inherited or
explicit higher floors within that domain. `--min-app-version` and a
recipe's existing minimum keep meaning the core requirement; recipe parser
features themselves require the front end. Importing a package contributes
its core floor to a machine; its packaging-only requirement does not follow
decoded ROM bytes into a differently packaged machine image.

**Transition.** Keep historical row numbers as literal pre-split releases;
both components shared that release line. Preserve existing manifests,
digests and explicit core floors, including conservative old markers that
also covered front-end features. Do not bulk restamp published artifacts.
Only new front-end features above the extraction baseline need the new
annotation; freeze that baseline and test that rebuilding existing feature
sets keeps their manifest bytes. Old readers ignore the new annotation,
but strict codecs still refuse new keys/types. Neither domain may silently
change the meaning of an existing key/value (point 4 still applies).

**Implied implementation (not part of this document change):**

1. Add `crates/pc-config/src/compat.rs`, export it from `src/lib.rs`, and
   move the core rows/evaluators and coverage tests there from
   [`crates/pc-image/src/compat.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/compat.rs). Use core-owned release/row data without
   an `image-store` dependency; the provider converts it at the boundary.
   Keep package-format rows in `pc-image`; do not move the whole file.
2. In [`crates/image-store/src/compat.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/src/compat.rs), replace global `level()` checks
   in `Requirement`, `raised`, `stamp`, `floor` and `carry` with explicit
   component levels and two requirements. Split `envelope`/`shared_release`
   results by owner. Extend `ConfigValidator` in `src/store.rs` beyond its
   single `required_release` result; update `src/lib.rs` errors and
   `src/summary.rs` fields/cache schema for component-specific reasons.
3. Wire core version/rows and front-end application version through
   `crates/pc-image/src/{compat,provider}.rs`, `crates/platforms/src/` and
   the [unemu front end](https://github.com/unemu-org/unemu)'s
   application composition. Update `image-registry`
   pull/push guards, `recipe/src/compat.rs`, CLI import/template writers
   and library/Remotes readers to use the same pair, before fetching blobs.
   Update machine-images §5.6 when this amendment is accepted.
4. Test unequal core/front-end versions in both directions, missing core,
   malformed/absent markers, development-row maxima, inherited floors,
   early pull refusal and component-specific disabled rows. Retain table
   completeness and published byte/digest pins; test front-end-only changes
   leave the core marker unchanged and metadata raises neither marker.

**Open questions for the maintainer:** Accept the second annotation's name
and the frozen extraction baseline? Confirm that inherited front-end floors
follow retained encoded content only, while explicit core floors survive
repackaging?
