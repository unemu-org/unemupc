# Status

2026-11-22 — The front end moved to [unemu](https://github.com/unemu-org/unemu) at `0f279540`; entries above this marker are the core's own.

Snapshot of what works, what's in flight, what's broken. **Not the roadmap** — open work lives in
beads (`bd ready --exclude-type epic`, `bd list --type epic`).

Update this when a feature lands, breaks, or gets pulled. Stale status is worse than no status.

- **2026-11-23 — Minimal PC picture window:** `unemupc-view --machine FILE`
  reuses the standalone machine loader, with shared `unemu-host` sessions for
  keyboard, pointer capture, pacing and host audio. A winit/softbuffer
  application shows the aspect-correct picture without GUI chrome. Reset,
  pause and quit chords and clean `Finished` shutdown are covered without a
  window by the scripted test core. Window/audio dependencies remain confined
  to the application layer. All four release archives include the viewer;
  desktop/audio and hosted release smoke checks remain manual. See
  [Viewing a machine](commands.md#viewing-a-machine).
- **2026-11-23 — Offline PC media description and export:** contract 0.5's
  optional operations describe configured floppy/fixed-disk slots and their
  fixed lengths without creating a PC. `raw`, `vhd-fixed` and `machine` export
  current readers through streamed sinks. VHD footers and machine documents
  are deterministic; machine files use relative image names, full output
  SHA-256 pins, volatile writes and hash-only firmware references. Reports
  disclose settings omitted or changed by the format. Tests cover the shipped
  floppy/IDE configurations, the existing machine-file loader, an independent
  VHD parser and fixed footer vector, request-order determinism, bounded reads,
  validation and reader/sink failures. Emulation and golden expectations are
  unchanged.

- **2026-11-23 — PC contract 0.4 migration, pinned at 0.5:** `unemu-core-api` and
  `unemu-core-files` use release v0.5.0 at `316dd2a1` (0.5 only adds optional
  offline media description and export, implemented by the PC core below the
  frontend boundary).
  The PC core and About output state contract 0.5. Successful finish enters `Finished`; terminal
  mutations and invalid input return `Usage`. Soft reset works in Ready and
  Halted, clears queued input and held controls, and preserves counters and
  media. Unbounded runs and reached-limit input delivery follow the matrix;
  counter exhaustion fails before an overflowing instruction executes.
  Media selection stages opens before ejecting the current disk; additions
  stay unsupported. File adapters use typed stages and constructor factories.
  Storage regressions cover partial tails/reads, destructive cuts and escaped
  named records; journal tools retain the separate path-lock domain. Guest
  programs, device timing and golden expectations remain unchanged.

- **2026-11-23 — VGA palette writes with PAS set:** attribute palette writes
  now take effect with either PAS value, matching the sequence reported from
  Windows 3.x vga.drv. This is an explicit compatibility policy, not verified
  original-hardware behaviour. PAS=1 reads still return stored data; register
  masks, flip-flop sequencing and PAS=0 display blanking/overscan are unchanged.
  A separate open mode-12h floppy draws sixteen bars and checks displayed RGB
  against an independent oracle; it and the mock-bus test fail with the old
  write gate. See [the evidence and policy](design/vga-modex-test.md#mode-12h-pas-palette-compatibility-regression).
- **2026-11-23 — VGA diagnostic registers and colour path:** the machine
  diagnostic report now appends an optional register-level VGA section with
  sequencer, CRTC, graphics and attribute registers, PAS/flip-flop phase,
  PEL mask, DAC mode/address/component, raw DAC 0–15 and the sixteen attribute
  colours mapped to their final DAC indices and RGB. Observation uses fields
  directly and shares scanout’s colour-path calculation. Reports for other
  display models retain their text. This adds diagnostic evidence without
  changing palette programming or rendering. See
  [observation §Diagnostic report of a running machine](design/observation.md#diagnostic-report-of-a-running-machine).
- **2026-11-23 — Four release targets:** the release workflow builds
  `unemupc` for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`
  on Ubuntu 22.04 runners (glibc 2.35 or newer), for `aarch64-apple-darwin`
  on `macos-latest` and for `x86_64-apple-darwin` on `macos-15-intel`; every
  target runs `--version` and keeps the static checks, notices and packaging.
  A manual run with `upload` false builds and checks an existing tag without
  touching its assets. The first such run must still verify the new targets'
  hosted builds. See [Releases](commands.md#releases).
- **2026-11-23 — Guest RAM diagnostic image:** `PcMachine::memory_image`
  copies every configured system RAM region in physical address order,
  including extended and upper-memory RAM, independently of A20. Device
  windows and ROM are omitted; capture leaves time, CPU and devices unchanged.
  The owned regions and plain-text summary let front ends save a local dump
  beside the report. Guest bytes must never enter a repository. See
  [observation §Guest RAM image](design/observation.md#guest-ram-image).
- **2026-11-23 — Open 360x240 VGA regression fixture:** the bootable
  `vga-modex-test` programs unchained byte addressing, single-plane drawing,
  one-pixel glyphs, write-mode-1 copies, 28 MHz retrace polling and two-page
  flips. Its independent pixel golden passes all 60 displayed frames on the
  unchanged VGA model. The reported application fault is not reproduced;
  no model correction is claimed. See [the evidence](design/vga-modex-test.md).
- **2026-11-23 — Release 0.0.4:** the first release of the core repository
  after the split. The workspace version is 0.0.4 and
  `pc_config::compat::NEXT_RELEASE` is 0.0.5, so a core row for a feature
  this release does not read names 0.0.5. The release archive holds the
  standalone `unemupc` only; the desktop and command line `unemu` are
  released from [unemu](https://github.com/unemu-org/unemu), which pins this
  release's commit.
- **2026-11-22 — Repository split point chosen (E4):** the maintainer chose
  `0f279540` as the split point, with hosted CI green. Front-end changes now
  land only on the front-end branch until the re-pin; core changes land on
  `main` as usual. No new repository exists yet. See
  [repository split §12](design/repo-split.md#12-steps).
- **2026-11-22 — Repository split preparation: shared tests and fixture root:**
  front-end tests import the four shared harness modules through
  `pc-harness/test-support`; all 30 cross-side source includes are removed.
  The boundary guard rejects source includes across the two sides, with
  planted self-tests. Core fixture paths use `UNEMUPC_FIXTURE_ROOT` when set
  (an existing absolute directory), otherwise the harness repository;
  front-end data paths stay local. The machine guide and FreeDOS fetch script
  are read at run time. The split plan is accepted; no new repository exists
  yet. See [repository split §2.4](design/repo-split.md#24-what-crosses-the-line-outside-cargo).
- **2026-11-22 — Repository split ownership guard and source scans:**
  the extraction inventory assigns every tracked path to the core, front end
  or both. The guard rejects missing and overlapping ownership. Platform-id
  scans now stay on their own side: `image-store` scans 22 front-end crates,
  `pc-config` scans 17 core crates, and the planner selects each by its source
  set. Ownership and planner self-tests pass. See
  [repository split §12](design/repo-split.md#12-steps).
- **2026-11-21 — Repository split planned (Proposed):** the extraction plan
  for a core repository and a front-end repository is written and waits for
  the maintainer. The core keeps `unemupc` with 17 crates; the front end is
  a new repository with 22 crates that pins the core by revision. Nothing is
  implemented and no repository exists yet. See
  [repository split](design/repo-split.md).
- **2026-11-21 — Independent core and front-end compatibility:**
  `unemu.min-app-version` names the selected platform core's release;
  `unemu.min-frontend-version` independently rates front-end formats.
  Core rows and completeness tests live in `pc-config::compat`; package
  and recipe rows remain on the front end. Application composition supplies
  both release levels; readers refuse either unmet marker before strict
  decoding or pull blobs and give a component-specific disabled reason.
  The front-end extraction baseline stays 0.0.3, preserving existing
  manifest bytes, digest pins and explicit core floors. See
  [machine images §5.6](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md#56-compatibility-of-published-images).
- **2026-11-21 — FAT12 tests stay with the front end:** the complete
  user-file volume-builder suite moves from `pc-harness` to `pc-image`,
  retaining both test names, corpus, full SHA-256 checks, skip/required-file
  rules and byte-equality assertions. Recipe data changes select front-end
  readers. The core-side closure, including dev dependencies, no longer
  reaches `media` or a front-end crate; its boundary allow list is empty.
  The separate `window` → `upcli` application-composition exception remains.
  See [core contract §9](design/core-contract.md#9-towards-two-repositories).
- **2026-11-21 — Core host-interface allow list empty:** the four temporary
  directory exceptions move from assembly to host-side `machine-file` test
  support, with all callers following them. The CPU builds only its legacy
  reset image, replacing the process-wide once cell without changing x86jit.
  The host guard has zero exceptions. See
  [core contract §9](design/core-contract.md#9-towards-two-repositories).
- **2026-11-21 — One front-end binary:** `unemu` opens the window with no
  arguments or desktop options and runs the composed command line when its
  first argument is a word (`image`, `media`, `recipe`, `library`, `run`).
  `--help` lists both paths; `--version` and `--about` use the desktop notice.
  The `upcli` executable is removed; its application-layer library remains.
  Archives contain `unemu` and the standalone PC core `unemupc`.
  `setup-unemu` replaces `setup-upcli`; its workflow installs and runs the
  current binary from a checked test archive. Linux still requires ALSA for
  command words: the release workflow checks bare Ubuntu before and after
  installing `libasound2`. The container result remains unmeasured locally.
  See [core contract §12.10 step 13](design/core-contract.md#1210-steps).
- **2026-11-21 — Final journal and file-mount separation:**
  `journal` is emulation/core and `journal-files` is PC/core on the host.
  `machine-file` owns `FileMount`, file assembly entry points, output-path
  validation, floppy append-by-path and the real-file assembly tests.
  Assembly uses only the journal's byte/record logic and mount-supplied
  sessions; protected input identities are opaque tokens, interpreted by
  the file mount. The default lookup maps are ordered maps without a
  host-seeded hasher. Both assembly/media boundary exceptions and the
  assemble/mount/observation host exceptions are removed.
  `media` retains neutral FAT12 codecs; atomic image publication and file
  layer chains move to `journal-files` with their tests, closing the neutral
  CLI's transitive path to the PC file engine. No media journal facade remains.
  Disk bases remain read-only and fully SHA-256 identified; writes remain
  separate overlays, and commit publishes a new base.
  All 192 saved CLI cases remain byte-identical. Open-fixture guest goldens
  pass; the sandbox's loopback/clipboard failures and supervisor-only checks
  are recorded in the core-contract step's validation note.

- **2026-11-21 — Shared journal file engine, extraction still partial:**
  `journal-files` now uses contract 0.3.1's `FileRecord` for session records,
  header publication, retention and cleanup; offline tools transfer `PathLock`
  into recovery. `unemu-core-api` and `unemu-core-files` use release 0.3.1
  at `afba6e6b`; the NES core uses `0dbf0d56` with the same contract.
  Contract 0.3.1 fixes first mount when umask removes write permission by
  keeping the writable creation descriptor; the regression now runs by default.
  Accepted behavior changes: failed appends roll back to their
  prior length (`Storage`, or `StorageUncertain` if rollback fails), while the
  journal still latches the failure and stops output; `media verify` may find
  a shorter or no discarded tail. Real crash tails are still discarded.
  Mounting and offline tools now refuse reserved sidecar names with the same
  Usage diagnostic; generated journal names remain valid. Unused media facade
  exports are removed, but live ones and the assembly file mount/metadata remain.
  Final layer tags and the dependency/host-list proofs remain partial. See
  [core contract §12.10 step 11](design/core-contract.md#1210-steps).
- **2026-11-21 — Recipe format 1.0 frozen:** [decision-43 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-43%20-%20Recipe-format-1.0-frozen.md) is
  accepted. Recipes and work files use 1.0;
  experimental 0.1 is refused, while historical syntax snapshots remain.
  Existing output pins and lock bytes are preserved. The generated 1.0
  schema has a drift check and release asset; CI and release upload require
  conformance of all seven pinned open recipes without allowing drafts.
  Ken's exact licence-index input is missing locally; the complete Ken's
  input set still needs release-fixture provisioning. No recipe is published by this change.
  `commands.md` carries §9's promises until user documentation follows the
  front-end split. See [`design/recipes.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipes.md) §19.2 B17 and [decision-43 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-43%20-%20Recipe-format-1.0-frozen.md).

- **2026-11-20 — Contract 0.3 re-pin:** `unemu-core-api` and
  `unemu-core-files` use release 0.3.0 at `0d490fcb`; the NES core uses
  `1b2b1128`. The PC and scripted test cores state contract 0.3, and both
  programs' About lines follow. The contract's only API addition is the
  arbitrary-path `unemu-core-files::FileRecord`; this re-pin does not adopt
  it. Emulation goldens stay unchanged, including the NES picture digest
  `0xFCDF_D186_11FE_D325` and eight frames at 80 000 retired instructions.
- **2026-11-20 — Journal crate extraction, partly done:** `journal` owns
  the byte format, replay and sparse overlays; `journal-files` owns file
  sessions and offline tools; `media` temporarily re-exports the old paths.
  Both new crates remain neutral/core until the contract file adapter and
  mount extraction complete. The four journal commands now share one
  implementation in `unemupc`, and remain available in `upcli` unchanged.
  Existing `FileContent` supplies base reads; arbitrary-path records,
  publication/retention and fault injection need contract-repository work.
  Assembly's boundary and host exceptions remain; step 11 is not complete.
  See `design/core-contract.md` §12.10 step 11.
- **2026-11-20 — Firmware discovery is host-side:** directory scanning,
  environment roots, the persistent hash index and discovery sessions now live
  in `machine_file::firmware_discovery`, with all 60 existing tests. The
  user-ROM test helper moves to `machine-file`'s `test-support` feature;
  firmware catalogues and byte identity checks stay in `assembly`. The host
  guard loses 21 discovery entries (41 → 20); its remaining mount, map,
  temporary-directory and CPU-cache entries are outside this move. See
  `design/core-contract.md` §12.10 step 12.
- **2026-11-20 — Front-end host directories use `unemu`:** only shared
  preferences copy once from `unemupc`, retaining the original-directory fallback.
  Machines and journals, diagnostics/trace logs, saves and states each keep using
  their old `unemupc` tree while it exists, including new writes; `unemu` is used
  only when that old tree is absent. Old trees take precedence even over an
  existing new tree. No tree is copied, traversed or removed at startup: this
  preserves absolute journal identity, target-relative symlink paths and private
  permissions without copying multi-GB data. The user can move a tree explicitly.
  This is the safe interpretation of “a one-time read of the old place”.
  The image store, caches, workspaces, explicit roots, source-tree paths,
  `UNEMUPC_*` and the PC firmware index stay unchanged. Tests cover concurrent
  starts, interrupted preferences and abandoned tree staging, links, permissions
  and writes to a single journal. See `design/core-contract.md` §12.10 step 14
  and `commands.md`.
- **2026-11-20 — Raw commands belong to the standalone PC core:**
  `--boot`, `--bios` and `machine check` run only in `unemupc`;
  `upcli` rejects them with its normal usage error (exit 2). The frontend
  keeps `run --machine` and its store-backed library call. Raw-command
  integration tests follow the standalone binary; release archives now
  include it alongside `unemu` and `upcli`.
  See `design/core-contract.md` §12.10 step 6.
- **2026-11-20 — Recipe CLI exit policies:** work-file drift remains a
  warning by default, as recipes §3.8 specifies. `recipe check` and
  `recipe build` accept `--deny-drift` for CI (exit 1; build refuses before
  inputs or publication). `recipe conformance` exits 1 for drafts unless
  `--allow-drafts` is given; failed pinned builds and runs with no pinned
  recipe still fail. Usage text and the command-line pin are unchanged.
  See `commands.md` and [`design/recipes.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipes.md) §3.8's dated decision.
- **2026-11-20 — Machinefile syntax corpus:** three documented installation
  routes (FreeDOS 1.4, private MS-DOS 3.30, private Windows 3.1) now have
  equal TOML/HCL pairs with proposed input steps, waits, disk capture and
  lock snapshots. The pre-freeze review recommends keeping TOML, with
  concrete nested-input and multiline examples. These are design documents
  with explicit assumptions and placeholder pins; no guest installation or
  Machinefile implementation is claimed. See [`design/recipe-corpus/README.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-corpus/README.md)
  and [`design/recipes.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipes.md) §14.

- **2026-11-20 — Standalone PC core command line:** the new `unemupc`
  package supplies a library and binary for `--boot`, `--bios`,
  `machine check`, `run --machine`, `--version` and `--about`, without
  frontend store/registry dependencies. `cli-pc` delegates the four machine
  commands and retains store-backed preparation, keeping `upcli`'s usage
  and parse pin unchanged. The raw commands stay in `upcli` for the next
  move. The boundary check now enforces the core side; its original store
  exception is already unnecessary after the firmware-package seam.
  Release packaging is unchanged. See `design/core-contract.md` §12.10 step 5.

- **2026-11-20 — The desktop binary is `unemu`:** the package remains
  `window`. Desktop titles, application menus, native application identity,
  help/version/about, own-name messages, release contents, licence output and
  command examples use `unemu`; `upcli` is unchanged. The PC core still identifies
  itself as `unemupc`. Repository/archive names, compatibility requirements,
  environment variables, host directories and file formats stay unchanged.
  Name pins, the affected-test runner self-test, workflow syntax and local
  packing are checked without opening windows. The hosted release check on a
  test tag remains for the maintainer. See `design/core-contract.md` §12.10 step 4.

- **2026-11-20 — Stage B's launch migration is complete:** both front ends
  start stored PC images through `core_launch` and the PC factory. The final
  cleanup removes `pc_image::resolve`, `workspace::start` and `start_new`, whose
  callers were tests only. Those tests retain their names and assertions through
  preparation/inspection or factory boots. As an explicit S14 deviation,
  `SessionTransfer` still supplies verified content to the launch plans; the derived
  `PreparedSession::machine` and path mapping serve pending inserts, CMOS
  resets, catalogue editors and desktop host bindings. The transfer can go when
  launch plans take verified content directly; the view and rewriting can go
  when those PC tools use launch identities directly. No test is deleted;
  independent A/B comparisons retain their assembly reference path.
  Factory comments and the architecture/spec describe these live roles.
  Failed-run persistence still uses the typed PC assembly adapter, desktop
  host bindings remain beside the launch, and `upcli` retains its historical
  Debug parser output. See `design/core-contract.md` §4, §9 item 2 and §14.3 S14.

- **2026-11-20 — PC CPU dependency disables host-environment APIs:** the
  workspace uses `x86jit-core` with `default-features = false` and no optional
  features. The PC needs neither `environment` nor `test-hooks`; the exact CPU
  revision and lock file stay unchanged. The whole workspace builds; feature
  trees for `cpu`, `upcli` and `window` contain no x86jit feature. All 44 selected
  slow goldens pass; BIOS and DOS goldens preserve their exact retired counts.
  See `design/core-contract.md` §12.10 step 3.
- **2026-11-20 — Stored-image run commands are platform-neutral:** `image run`
  and `image machine run` now parse and execute in `cli-core` through the
  binary's `Cores`, with store aliases canonicalized before core selection.
  `CliPlatform::own_run` retains platform session pins, migration and ownership;
  `prepare_run` supplies the launch and confirmation after creation. The PC
  adapter preserves failed-run CMOS/journal persistence and diagnostics.
  Usage and all pinned parses are unchanged. S11's 131-call set and 11 binary
  invocations retain their output; the binary matrix also keeps every written
  file byte. A toy stored image runs and durably saves through the test core
  with no PC crate in its build. The old start APIs remain for the next step.
  See `design/core-contract.md` §14.3 S12.

- **2026-11-20 — Stored-image CLI runs use the core contract:**
  `image run` and `image machine run` remain in `cli-pc`, prepare through
  `pc_image::launch`, start with `core_launch::start`, run to
  `RunLimit::steps` and finish through the contract on success. Failed runs
  still persist CMOS and close journals through the PC assembly path: the
  contract forbids finishing a failed machine. Output now names the contract
  stop and the former idle cutoff is removed. Guest resets already continued
  in the old assembly loop; the core now performs and reports them. Core
  messages and finish warnings go to standard error. Pins,
  insert confirmation, existing journal files and retired counts are preserved
  outside those stated policy changes; the retired platform spelling still
  runs. The previous start API remains available for other callers.
  See `design/core-contract.md` §14.3 S11 and `commands.md`.
- **2026-11-20 — Desktop library Start uses the core launch contract:** image,
  machine and Gaming-copy starts freeze their launch after created disks and
  kept catalogues are restored. The UI calls `start_with` and
  `PcCore::create_machine`, retaining the typed PC Presenter and its media
  tools. Catalogue hardware comes from the core config; host media bindings
  remain beside the launch so Stop/Start keeps restored disks and journal
  paths. Display selection and run-only firmware/speaker options are retained.
  Headless comparisons cover exact pictures, audio, drive state, messages,
  retired counts and journal bytes, plus created/kept disks, unfinished drops,
  finished-machine reopens and Gaming power cycles. The old start API remains.
  See `design/core-contract.md` §14.3 S13.

- **2026-11-20 — Stored PC launch objects are available:** `pc_image::launch`
  prepares an image workspace, a machine or a disposable copy as a
  `core_launch::Prepared`, with the edited launch document, verified content
  and session storage plans. Plans reopen at every start; sparse disks stay
  sparse, changed files fail with `Content`, and records retain existing
  journal paths and `FILE.lock` exclusion with sessions and offline tools.
  Five-session comparisons alternate existing and contract starts and compare
  frames, audio, drive activity, messages, sector reads/writes and disk/CMOS
  journal bytes through retention. CLI and desktop starts remain unchanged.
  See `design/core-contract.md` §14.3 S10.

- **2026-11-20 — Created disks and kept catalogues derive their start from the launch:**
  (`upc-dfs.43.7`, stage B step S7; design/core-contract.md §14.3).
  Both catalogue editors now update launch bindings transactionally before the
  machine is derived. Short kept record names map to the existing journal files,
  including journals outside the workspace; hashes, retention, catalogue order,
  pending-insert precedence and selected disks are preserved. Launch lowering
  now restores a non-first selected disk without reordering the catalogue;
  stored configs retain their existing restriction. Restart tests keep journal
  bytes and base files unchanged, and re-lowering the edited document reproduces
  the prepared machine. No crate-map change or object-based launch plans.
- **2026-11-20 — PC creation through the core contract:** `PcCore::create`
  and its typed `create_machine` accept the bounded PC launch document;
  `validate` checks it without host access and lists its exact flat content.
  Assembly mounts the supplied content readers and storage records, keeps
  journals for later catalogue inserts, and exposes the effective config via
  `PcMachine::config()`. Displays follow MDA/VGA order. Run-only options are
  `allow-unlisted-firmware` and `speaker-trace` (`true` / `false`); duplicate,
  unknown and invalid options are refused. Contract errors preserve content,
  storage and uncertain-write kinds. Tests compare frames, sound, drive lights,
  messages and journal bytes with the verified-file path across two sessions,
  and cover replay, retries, media appended by path and starts from parts.
  Existing CLI and desktop starts are unchanged; routing those through this
  factory is later work. See `design/core-contract.md` §14.3 S9.

- **2026-11-20 — Stored PC starts derive their machine from a launch:**
  (`upc-dfs.43.6`, stage B step S6; design/core-contract.md §14.3).
  `pc-image` builds the effective launch after Settings, with verified flat-content
  identities and session retention. One lowering and path translation derives the
  assembly config; pending inserts edit the launch, and edits install the document
  and machine together only after successful lowering. Thread transfer preserves both.
  Stored-image starts keep their machine, base keys, journal paths and retired counts;
  unjournalled CMOS recomputation is represented by launch-only content. Invalid core
  record ids (including the reserved `kept-` prefix) are refused at start even for
  read-only media. Catalogue names retain media-set display labels with slashes, and
  compressed floppies select inline bytes or a journal key from the launch's policy.
  Assembly and front-end start callers are unchanged. Created disks and kept catalogues
  still use their existing path; their launch edits belong to S7. No crate-map change.
- **2026-11-20 — Templates reproduce the published recipe-built packages and sets:**
  `pc_image::template` builds its three firmware packages and three FreeDOS
  sets through the recipe builder from the recipes, work file and locks in
  `crates/pc-image/data/recipes/` (moved there from the docs corpus), offline
  from `vendor/`. This preserves the software repository's
  published metadata instead of regenerating it with today's importer.
  Firmware pins move: AT BIOS `e3c226ae…` → `c4c53a6a…`, VGA BIOS
  `179a11db…` → `c1972247…`, XUB `7d42e80e…` → `4dc25f1c…`.
  FreeDOS pins move: 1.44M `2ee5d72d…` → `77060710…`, 1.2M
  `1320a287…` → `6508d997…`, 720K `bd3a3007…` → `d8c593a8…`.
  The pre-release 1.2.0 template pins move: `open-at` `750981c2…` →
  `e821e8e8…`, `open-at-games` `01028f9c…` → `95a9030d…`.
  Packages gain their locked dates and README text; sets gain their locked
  work id, synopsis and needs (`video=vga,ega` in the manifest). Template
  machines change only their `unemu.firmware.sources` package digests:
  the machine configs and every payload descriptor remain byte-identical,
  pinned by a regression test against the pre-change manifests. ROMs,
  sources, floppy contents, blank disks and hardware settings are unchanged.
  Guards still run before tags; existing differing revision tags are refused.
  See [`design/machine-images.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md) §5.5 and [`design/recipes.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipes.md) §19.2.

- **2026-11-20 — The PC launch document:** (`upc-dfs.43.5`, stage B step S5;
  design/core-contract.md §14.2.2, §14.3). `pc_config::launch` holds the document the PC core
  is to build a stored image's machine from (`application/vnd.unemu.pc.launch.v1+toml`): the
  stored config unchanged, plus a `content` table (each bound digest's flat SHA-256 and length)
  and a `record` table (each journal's record and retention). `decode` and `encode` (pinned
  encoding, byte-exact round trip), `check` without content (the core's `validate` to be), and
  `lower`, which reads firmware and CMOS and names each disk base and journal by a key. A
  firmware row is at most 16 MiB and a CMOS row 64 bytes before anything is opened; missing,
  doubled and unused rows of either table are refused. The store now validates a stored config
  through `launch::lower`. A journalled CMOS lowers to an NVRAM journal over its image in the
  validated machine; the start path already replaced that field, so the start's machine is
  unchanged. A stored journalled medium whose id cannot form a record name now fails validation.
  Validation treats media and journal names as logical keys and never normalizes host paths;
  dangling host symlinks cannot invalidate a document. Metadata validation retains ROM lengths
  and refuses overlapping ROMs with the full lowering's sentence. Decode rejects unsupported
  document and hardware versions and unknown keys, using the shared stored-config hardware
  decoder. Nothing starts a machine from the document yet.
- **2026-11-20 — The recipe corpus tracks the migrated software recipes:**
  The seven real recipes, their two work files and their locks are refreshed
  from `unemu-org/software` at `3c493f7`, under `recipes/media/freedos/`,
  `recipes/media/kenslab/` and `recipes/firmware/`. Design-only recipes and
  first-round HCL snapshots are preserved; current GitHub links use the
  repository's new name. See [`design/recipe-corpus/README.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-corpus/README.md) for the mapping.

- **2026-11-20 — The core's host interfaces are guarded:** (`upc-dfs.29`,
  embeddability step 2; design/core-contract.md §12.10). No guest behaviour change.
  `scripts/core-host-guard.sh` checks the production source spellings of the contract
  and emulation layers against 41 exact exceptions (42 matched lines), rejects stale
  exceptions and new host access, including direct libc use, and runs with its planted
  self-tests in the bead and CI guard lanes. It uses the specified first-`#[cfg(test)]`
  cutoff, so it is a coarse source check rather than Rust name resolution. All twelve
  emulation crates now forbid unsafe code; ten gained the attribute, while `pc-config`
  and `unemu-pc-core` already had it.

- **2026-11-20 — `machine-file` no longer depends on the image store:** (`upc-dfs.34`,
  embeddability step 7; design/core-contract.md §12.10 and §9 item 3). No behaviour change.
  The firmware-package lookup is the trait `machine_file::FirmwarePackages`, which
  `pc_image::firmware::StorePackages` implements over the store; the PC package rules and the
  lookup moved to `pc-image/src/firmware_package.rs`, and the preparations that take a store or
  a store root are now `pc_image::firmware::prepare_for_assembly_with_firmware_store` and
  `prepare_for_assembly_with_store_root`. `PreparedMachineFile::firmware_sources` gives the
  packages' ids as text (`sha256:<hex>`); `pc_image::firmware::firmware_sources` parses them.
  `machine-file`'s closure loses `image-store`, `zstd-sys`, `cc` and `serde_json` (62 to 54
  run-time packages). `upcli run --machine` gives the same output with and without a store.

- **2026-11-20 — A launch from parts in `core-launch`:** (`upc-dfs.43.8`, stage B step S8;
  design/core-contract.md §14.3). No behaviour change for a file opened with Open… or
  `--open`. `core_launch::Prepared::from_parts` makes a start from a platform's own `Parts`: the
  core, the platform, a title, notes, the config, and a `ContentPlan` and a `StoragePlan` made
  on a worker and opened on the machine's thread at every start; `with_options` adds run-only
  options; `start_with` builds a platform's own machine type from the same request, and `start`
  is `start_with` over `Core::create`; `Cores::for_platform` finds a platform's core. The
  file-only accessors of `Prepared` (`kind`, `file_name`, `sha256`, `size`, `saves_dir`,
  `states_dir`) now answer `Option`, `None` for parts, and the player offers Save and Load state
  only for a start with a states directory. Nothing starts from parts yet; the PC's store side
  comes with S10.

- **2026-11-20 — The stored PC image config and its codec move below the contract:**
  (`upc-dfs.43.4`, stage B step S4; design/core-contract.md §14.3). No behaviour change.
  `pc_config::image` now holds the stored config's model (`Config`, `Firmware`, `Medium`,
  `Slot`, `Nvram`, `Policy`, `Geometry`) with its decode and encode, the hardware-table encoding
  and frozen defaults of an import, the binding walks and the codec half of validation
  (`image::lower`), which opens firmware and CMOS bytes through the contract's `ContentSource`
  by the SHA-256 and size of their content rows. `pc-config` has its own `Digest` in the store's
  text form (`sha256:` and 64 hex digits); it still declares no store, `media` or
  `machine-file` and reaches `media` only through `assembly`, the allowed crossing. `pc-image` keeps the store's half and the store operations, serves the codec a
  `ContentSource` over its blobs and converts digests at its edge; `pc_image::Policy` and every
  refusal's words are unchanged. Every config the workspace's tests encode or decode has the same
  bytes before and after, and the pinned template digests hold.

- **2026-11-20 — x86jit moves to `6cb817b8`; the exception trace shows the registers:**
  (`upc-f7j.21`). Two x86jit changes: the delivery log can record the interrupted code's
  registers with each delivery (xj-ir6.40 (x86jit)), and on the 80286/80386 machine models a
  real-mode frame (a delivery, PUSHA, far CALL, IRET/RET/POPA) wraps inside the segment and
  faults only where a word sits at offset FFFFh, so a delivery at an even SP no longer shuts
  down; SP 1, 3 or 5 still does (xj-srh.27 (x86jit)). The machine enables the log with the
  registers on whenever it enables it (Record exceptions, the report's log, `upcli
  --trace-exceptions`), and every trace and report delivery line ends in `ax bx cx dx si di bp
  ds es ss:sp flags` as the delivery began (for INT n the INT's inputs, SS:SP before the frame;
  e-registers and FS/GS on an 80386); folding is unchanged and a folded entry shows its last
  delivery's registers. Cost on the high-IRQ benchmark: none measurable (5.56–5.69 s with the
  trail and the log against 5.55–5.66 s with the registers off). The ignored goldens pass
  unchanged. Three tests relied on a shutdown at an even real-mode SP and now start from SP
  FFFFh: pc-core's `a_stall_that_overruns_its_stack_shuts_down_and_the_board_resets` and the
  exception trace's `a_fault_loop_folds_into_one_entry`, both on `delivery_overrun_rom` (its
  SHA-256 re-pinned), and the backend's
  `a_stack_that_overruns_shuts_down_within_the_stall_window`; the
  open-BIOS A20 reset test leaves SP at 1 instead of 2, since at SP 2 the #UD of a missed reset
  fetch is now delivered and a returning handler reaches the low ROM copy; with the window
  disabled it again stops with "64 resets within 0 steps", and decision-42 records why.
- **2026-11-20 — The PC provider's validation splits into a store half and a codec half:**
  (`upc-dfs.43.3`, stage B step S3; design/core-contract.md §14.3). No behaviour change.
  `pc-image`'s `prepare` first reads the store (descriptors, size limits, a ROM's source link,
  a fixed disk's chain, the zero-base rule, the closure) and hands the codec half one row per
  bound blob or disk head (SHA-256 of the flat bytes and their length) and a reader of the
  firmware and CMOS bytes; the codec half checks the config against its hardware tables and
  lowers it without naming any store type but the digest, ready to move below the contract in
  S4. Every outcome of `prepare` in the `pc-image`, `pc-library` and `cli-pc` tests is the same
  before and after, refusal words and closure order included; new tests run the codec half over
  inputs in memory and pin the closure order.

- **2026-11-20 — The assembler mounts through one seam:** (`upc-dfs.36`; design/core-contract.md
  §12.5 move 2, step 10 of §12.10, §14.4). Assembly asks the machine's `assembly::Mount` for
  what a config names by path (the bytes of an image, the verified base of a journalled disk, the
  journal of a disk or of CMOS RAM) instead of opening it. `FileMount` answers with the files the
  paths name, exactly as before; every `assemble*` function but `assemble_mounted` uses it, as do
  a floppy appended by path and the byte loads of an already resolved config, so no caller and nothing a user sees changed: the same journal bytes, host
  calls and messages. Two journal outputs with one key are now refused for every mount (a file
  mount refused them before, as aliases). `assemble_mounted` takes any other mount; one in memory serves a base as the
  contract's `ContentReader` and a journal as its `Record`, so the PC core can mount a machine
  from a `ContentSource` and a `Storage` without naming the media crate (stage B, S9). The
  machine keeps its mount for its life, so a catalogue's journal opens through it at insert, and
  a floppy appended by path is still a host file with its own journal and lock. A new test runs
  a machine with floppy, fixed-disk and CMOS journals over records in memory beside the same
  machine over files: equal guest results, and the records hold the journal files' bytes.
  The workspace tests and the ignored goldens pass, the journal goldens among them.

- **2026-11-20 — `pc-config`: the machine schema and its lowering move below the contract:**
  (`upc-dfs.43.2`, stage B step S2; design/core-contract.md §14.3). No behaviour change. The new
  `emulation` crate `pc-config` holds the machine schema, its TOML decoder, the source origins
  with their TOML half and the lowering to `assembly::MachineConfig`; it opens no file and
  forbids unsafe code. `machine-file` depends on it, re-exports every moved name at its old path
  and keeps what touches the host (YAML, `extends`, firmware discovery, `FileImageReader`, the
  loaders). Four things changed shape because another crate's type cannot gain methods or a
  foreign trait impl there: `Origins::from_yaml` and `MachineFile::describe` are the traits
  `YamlOrigins` and `Describe` of `machine-file` (callers import them),
  `JournalBase::from(&VerifiedBase)` is `machine_file::journal_base`, and the merge helpers of
  `Origins` and the lowering's diagnostics context became public for `machine-file`. The boundary check places `pc-config` in the emulation layer
  with the allow list unchanged; two self-test fixtures plant `pc-config → machine-file` and
  `pc-config → media`. `pc-config`'s dependency tree still holds `media` through `assembly`
  until the journal split.
- **2026-11-19 — The journal sits on a record of the contract:** (`upc-dfs.35`;
  design/core-contract.md §12.5 move 1, step 9 of §12.10, §14.4). `media::JournalMedia` keeps a
  floppy, IDE or CMOS journal in one `unemu_core_api::Record` and opens no file: it reads,
  appends, cuts, synchronizes, creates a journal by committing a replacement that holds only the
  header, and rewrites retained history through a replacement it verifies before the commit. The
  journal's file code (the stable `FILE.lock`, header publication through `FILE.create-N.tmp`,
  retention through `FILE.retain-N.tmp`, the leftover cleanup, `sync_all`) moved unchanged into
  `media::journal_file`, which is the journal file as a record: `media::JournalFile`, for the
  storage the PC's provider will build (a table from record name to journal file, stage B).
  `JournalMedia::open_record` mounts any record, also the contract's storage in memory. The path
  constructors are thin wrappers, so no caller changed and nothing a user sees changed: the same
  bytes (a scripted session's four stages are pinned to the SHA-256 the previous build wrote),
  the same host calls in the same order (the journal's model tests are unchanged), the same
  failure names, and the same lock, so a running machine and `media verify`, `rollback` and
  `commit` still exclude each other (new test). Through a record, which no caller uses yet, a
  failed header commit is named "publish header" with the file's step in the message.
  The workspace tests and the 63 ignored goldens pass, the journal goldens among them.

- **2026-11-19 — The machine-file lowering asks its reader for everything of the host:**
  (`upc-dfs.43.1`, stage B step S1; design/core-contract.md §14.3). No behaviour change.
  `ImageReader` answers an image's length without reading it, whether a path is a reserved
  retention name, and the check of journal paths against the inputs; `verify_journal_base`
  answers a length and a digest instead of the media crate's verified base. `lower.rs` now names
  no `std::fs`, `std::env` or `media`; `FileImageReader` and the metadata reader of
  `describe` live in `lib.rs`. A test lowers a machine with journalled media from a reader in
  memory whose files exist nowhere. Assembly's `MachineConfig::validate` still resolves journal
  folders on the host (§12.10 steps 10 and 11).
- **2026-11-19 — The player's status line no longer clips its light's word:** (`upc-6cj`;
  design/mockups/player.html "Status line"). The light cell had a fixed 68 px, narrower than its
  light, gap and "Running" together, so the platform name drew over the last letter ("Runninc"),
  and a line too narrow for its cells scaled every cell down, the light and the message too. Now
  the light cell is as wide as its light, the mock-up's 5 px gap and the widest of Running,
  Paused, Halted and Stopped, and never shrinks; cells inset their text by the mock-up's 6 px;
  sound and speed grow past their usual widths when their text needs it. When the line is too
  narrow the platform and the content's name give way first (they ellipsize, down to 60 px),
  then the slot chips, and only then the message, which otherwise keeps 240 px as the machine
  status line's does. The machine status line still insets its text by 3 px: at 6 px its
  92 px EMS cell would cut "EMS unknown", so aligning it means resizing its cells. New tests: every
  cell holds its text and the light's cell is as wide in every state; at 760 px with the NES
  platform name and a long title the names ellipsize, the light's word is whole and the message
  keeps 240 px; a long speed text widens its cell.
- **2026-11-19 — A CPU-only reset with A20 off reaches the BIOS:** (`upc-f7j.20`;
  decision-42; design/machine-config.md §A20 line and System Control Port A, "Reset fetch with
  the gate off"). Before, a guest that turned the gate off through the 8042 and then reset the
  processor (shutdown or command FEh) fetched its reset vector from EFFFF0h, where the IBM 5170
  board decodes nothing; with SS:SP 0000:0002 the #UD could not be delivered and the machine
  stopped with "64 resets within 0 steps, the last shutdown at f000:fff0" (MEASURED by the
  maintainer with Windows 3.1 Standard mode under both the open AT BIOS and the IBM 5170 BIOS;
  that hardware reaches the BIOS here is INFERRED from the guest running on hardware, and an
  earlier divergence leaving the gate off only here is not ruled out). Now the processor's A20
  is held open from a CPU-only reset until its first code-segment load (the 80286 drives A23–A20
  high for CS cycles until the first far jump); the 8042 output port, port 92h, the board line
  and the memory map are untouched, so the guest reads back the gate it wrote and 1 MiB wraps
  again right after the reset vector's far jump. The documents do not settle what the 5170's ROM
  decode does here; decision-42 records the choice, its uncertainty and the trigger to revisit.
  Supersedes the older entry below that pinned the #UD at EFFFF0h. New tests: an open test ROM
  (`assembly::test_support::a20_off_reset_rom`, hash-pinned listing) resetting through a triple
  fault and through FEh, a machine-level window test, and an open boot sector resumed through
  shutdown code 0Ah by the open AT BIOS (fixture) and by the IBM 5170 BIOS (user ROM, not run
  here). The ROM test, the window test and the open-BIOS boot-sector test failed before the
  change, the last with the reported reset storm.

- **2026-11-19 — The boundary check lists the external crates the core may reach:**
  (`upc-dfs.28`; design/core-contract.md §12.3, step 1 of §12.10; commands.md § Crate
  boundaries). `scripts/crate-boundaries.external` names, with a reason each, the crates from
  outside the workspace that the `contract` and `emulation` layers reach: today 27 at run time
  (the contract, `thiserror`, `x86jit-core` with `iced-x86` and `lazy_static`, `sha2` and its
  seven, `libc`, and the thirteen that come only through `media` and leave with the journal
  split) and 7 at build time, none for the contract. `scripts/crate-boundaries.sh` walks the
  resolved graph from each package of those layers and fails on a crate the list does not name
  or names with another kind (run, build, test), with the package, the crate, the kind and the
  path, and on a line whose crate is no longer reached, so the list only shrinks. Eleven new
  self-test fixtures; it runs in the `guard` gate as before. No code or behaviour changed.
- **2026-11-19 — The about text of a work is specified, not implemented:** (`upc-miu`;
  design/about-text.md, mock-up design/mockups/about.html, both accepted by the maintainer on
  2026-11-19 with every recommendation). A work gets a long Markdown text per language, with a
  source per language (`own`, or `licensed` with a title, authors, an `https` URL and one of
  five Creative Commons licences), and its synopsis may be stated per language. Specified:
  metadata schema 3 (`synopsis` as localized text, `about`, `about_source`, language tags, the
  canonical form and limits, the writer's oldest-schema rule, the migration from schema 2 and
  what 0.0.3 shows) in machine-images §2.1, its 0.0.4 compatibility rows in §5.6; the
  work-file keys (`[title.about]` inline or `{ file = "NAME" }`, `about_source`, the synopsis
  table), the text files, rule 11 of `recipe check`, the guard rules R1 to R5 and warnings W1
  to W6, and the text layer `application/vnd.unemu.recipe.text.v1` in recipes §6.1, §6.11,
  §6.13, §6.16 and §7.1. The mock-up joined the icon inventory. No code reads or writes any
  of it yet; about-text.md §15 lists the implementation steps in order.
- **2026-11-19 — Stage B of the core contract planned:** (design/core-contract.md §14; no code).
  The maintainer accepted the plan with every recommendation: the PC config codec (the machine
  schema and its lowering from `machine-file`, the stored image config from `pc-image`) moves to
  a new `emulation` crate `pc-config`; the PC provider writes a launch document (the stored
  config plus flat content digests and sizes and storage records) that `PcCore::create` reads;
  `PcCore::create_machine` and `core_launch::start_with` let the desktop keep a typed
  `PcMachine`; `image run` runs through the contract, with the contract's stop in its result
  line, and moves to `cli-core` last and droppable; the journal keeps its lock `FILE.lock`.
  Fourteen steps, S1 to S14: S1 to S8 can start now, S9 on after `upc-dfs.35` and `upc-dfs.36`.
  Corrected in the spec: `disk.rs` and `zero.rs` stay in `pc-image`, the PC's storage is the
  provider's own and not `FileStorage` over the workspace, stage B does not need step 12, and
  decision-30 gets a note that it is amended before the repositories separate.
- **2026-11-19 — A guest that resets again and again no longer holds the desktop's frame;
  a storm of resets stops the machine with a report:** (`upc-f7j.19`;
  design/observation.md §The machine resets again and again, §Window pacing).
  Before, a guest that shut down right after every reset kept one run resetting until its
  virtual-time cap (a run of one second of guest time did not return within 60 s, debug).
  The machine now stops 64 reset requests in a row, each fewer than 16 steps of virtual time
  (the run cap's unit) after the one before, with `CpuError::ResetStorm` and a CPU stop report
  at the last shutdown: "the machine resets again and again: 64 resets within 192 steps, the
  last shutdown at f800:0007"; the desktop shows it in its machine error box and `upcli run`
  prints it and exits 1. A guest reset still continues the run to its deadline; the stop
  bounds a run to 63 close resets in a row, and every other reset follows at least 16 steps
  toward the cap. 16 only diagnoses loops that make essentially no progress: the open AT
  protected-mode test's resets are at least 855 steps apart (an INT 15h block move), its
  rounds 3,650 to 10,047. A reset costs about 0.5 µs (release), nine tenths of it in x86jit's
  fast-lookup clear; building the legacy reset image once took 10 % off. MEASURED: the open
  `reset_storm_rom` and `fault_loop_rom` (NASM listings and SHA-256 pinned) in `pc-core`
  (a run of one second of guest time returns after 63 resets with the stop and its
  report), `pc-session-tests` (on the host
  clock at 100 % and Unlimited every advance returns within its slice plus one chunk and the
  storm stops;
  under Unlimited the fault loop runs 30 frames, the longest advance about 61 ms in debug),
  `cli-pc` `reset_storm`, and `machine`'s streak test. Changed on purpose: the session test
  of a guest that pulses the 8042 reset line right after every reset, which ran on at
  thousands of resets a frame, now stops with "64 resets within 191 steps, the last
  reset request at f800:0004". The open AT protected-mode golden and every other golden are
  unchanged.
- **2026-11-19 — Diagnostics move into Tools › Diagnostics, and the exception recording keeps
  the last deliveries, folded:** (`upc-f7j.18`; design/observation.md §Diagnostic report of a
  running machine and §Exception and interrupt deliveries; commands.md). The desktop no longer
  reads `UNEMUPC_DIAG` or `UNEMUPC_TRACE_EXCEPTIONS`. Tools › Diagnostics › Save machine report
  (Ctrl+Alt+Shift+D, ⌘⇧D on macOS, with the pointer captured or not) writes `diag-N.txt`
  whenever a PC runs, paused too, and is disabled without one; Record exceptions, a check item
  off at every launch, keeps the last 10,000 entries with exact totals per vector and writes
  one `trace-N.txt` per recording at Save machine report (the report names it), Stop,
  unchecking and closing the window (a killed app writes nothing); Open diagnostics folder
  shows the folder in the file manager. The code trail runs for every machine; the report's
  delivery log starts with the first report or Record exceptions, since on an interrupt-heavy
  open guest (937,499 IRQs in 60 million instructions, release) it costs 6–10 % (5.53–5.98 s
  against 5.22–5.44 s; the trail alone 5.26–5.64 s, within the noise). Both the
  report's last 64 and the recording fold consecutive identical deliveries (same kind, vector,
  saved CS:IP and error code) into one entry with a count and the retired range, so a guest
  looping on one fault no longer pushes out how it got there. `upcli --trace-exceptions`
  writes the same recording at the end of the run instead of the first 10,000 lines.
  MEASURED: `machine::delivery_tap` (fifty distinct deliveries and then 100,000 identical
  #UDs leave 51 entries, one of them `x100000`), `cli-pc` `exception_trace` (the open
  `delivery_ring_rom`'s 12,000 unfoldable deliveries leave exactly the last 10,000, 6,000 per
  vector), the shell tests (the menu items and their enablement over AccessKit, the chord with
  the pointer captured on both platforms, the recording written at each trigger with the same
  retired count on and off), and `diagnostic_trail_golden` (the FreeDOS boot golden with the
  trail and the log, and with a recording, unchanged).
- **2026-11-19 — Every exception and interrupt the guest takes can be traced, and the
  diagnostic report lists the last 64; x86jit moves to `e63c1321`:** (`upc-x83.17`, for
  `upc-f7j.17`; design/observation.md §Exception and interrupt deliveries). x86jit
  `e63c1321` (xj-ir6.37 (x86jit)) keeps an opt-in log of the deliveries a machine vCPU
  performs in the guest, outside its state. `upcli run --machine FILE --trace-exceptions`
  writes a line per delivery to stderr (retired count, kind, mnemonic and vector, saved
  CS:IP and linear address, error code, CPL before and after, the 8 bytes at the saved
  CS:IP from memory only), bounded to the first 10,000 lines, then a count line every
  1,000,000 deliveries, and a summary (total, dropped, a count per mnemonic) before the stop
  line; the window does the same with `UNEMUPC_TRACE_EXCEPTIONS=1`, one
  `<data>/unemupc/diagnostics/trace-N.txt` per session, appended every frame and finished
  at Stop. IRQs are named by their 8259 input. With the diagnostic trail on
  (`UNEMUPC_DIAG=1`, `--diag-at`) the report gains a `delivery` section with the last 64
  records and the counts. Off by default; on or off the run is the same. MEASURED:
  `cli-pc` `exception_trace` (the exact trace of the open `deliveries_rom`, NASM listing
  and SHA-256 pinned: a #UD stepped over by its IVT handler, INT 60h, an IRQ0 tick and a
  protected-mode #GP(00F8h) handled by a gate; `--diag-at` on it; the bound on
  `delivery_overrun_rom`, 10,922 #UDs, and `delivery_stall_rom`, 11,264 #GPs read from the
  slice that failed), the report section and the extended twin-device test in
  `assembly::diagnostic`, the backend's log test in `cpu`, the trace's bounds in
  `machine::delivery_tap`, the desktop's trace file in the shell tests, and
  `diagnostic_trail_golden` (the open-BIOS FreeDOS boot with the log and the trace on
  stops at 8,800,473 retired and 10,000,000 virtual time with the golden's screen, POST
  stream and device snapshots; two traced boots give the same 7,120-delivery trace). Every
  golden is unchanged at the new pin.
- **2026-11-19 — Push from the library: Push to… of one item or several, Push here… of a
  registry or namespace, as the approved remotes-push mock-up (Remotes revision 3):**
  (`upc-5pw.64.5`; design/machine-images.md §5.1, mockups/remotes-push.html). Image › Push…
  became Image › Push to…, with Image › Push here…, the toolbar's Push… beside Pull… (the Push
  icon, now painted) and the Push to… / Push here… buttons of the panes. One item's destination
  is preselected by the Remotes rule (a host-qualified tag goes to its registry, a short tag
  pulled from the default registry back there, a short tag made here where it was pushed last,
  else nothing), shown as text and in a chooser of registries and namespaces, repository and
  tag; the verdict comes before anything is sent, reading this session's listing ("As listed":
  a version or revision tag taken is refused with the next tag suggested, a moving tag asks
  with both digests first) and the login (a registry without a login waits for Log in…).
  Several items, selected with Ctrl/Shift in Images, Installation media or Firmware, go one
  after another under one bar, each with its verdict; refused ones are skipped with the
  reason, a failure marks its row and the next one goes. A registry's or namespace's node
  shows its own pane until a tag is selected. Each push is recorded on the local tag
  (`unemu.pushed.to`, `Store::record_push`; the tag keeps its name), Source reads `local →
  host` and the pane lists the references; Remotes gains the pushed tag without a request, a
  tag pushed from a local tag of another name reading *Pushed from here · as …*. `upcli` is
  unchanged and records no push. The registry/namespace context menu waits for the library
  context-menu work (upc-5pw.82). MEASURED: `library_view::push_tests` (AccessKit tests of
  the whole frame against the mock registry: preselection, refusal, login, as-listed taken and
  move, Push here…, several with skipped and failed rows, Remotes and Source after the push),
  the shell's toolbar and menu gating test, `image-store` store test of the push record, and
  model tests of the rule and of Source.
- **2026-11-19 — Every set and package exports to plain files:** step B11 of
  design/recipes.md §19.2 ([decision-37 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-37%20-%20Every%20artifact%20exports%20to%20plain%20files%20without%20loss.md)). `upcli image export NAME:TAG --to
  DIR [--files] [--store DIR]` writes every layer's decoded bytes under its
  file name (the title of A3, without `.zst`), `config.toml` and
  `manifest.json` as stored, `EXPORT-VERSION` (`1`) and `SHA256SUMS`;
  `--files` also writes the files of the set's FAT12 disks into `files`,
  the volumes merged in the set's order, long names where a disk has them,
  each file's and folder's time its entry's taken as UTC. Nothing in `DIR`
  is overwritten: a file with the same bytes is kept, anything else
  refuses before a byte is written. The FAT12 reader is product code now
  (`media::fat12::reader`, with `Volume::tree`): it refuses truncated
  images, loops, chains outside the data area, cross-linked clusters and
  FAT16 sizes with a reason, and reads long names. `recipe build` names
  the exported files that keep each input (`kept as files in data-1.img,
  data-2.img`) and says that an unpacked archive itself is not kept.
  [`scripts/export-layout.sh`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/scripts/export-layout.sh) writes the same files from any OCI Image
  Layout with jq, sha256sum, zstd and mtools. MEASURED (`cli-pc` test
  `image_export`): command and script give the same files and times for a
  synthetic split set, a firmware package and the FreeDOS 1.4 1.44M set;
  Azuramoon built from the maintainer's folder exports with `--files` to
  that folder byte for byte (561 files). zstd joins the devShell for the
  script. commands.md, Exporting an image.
- **2026-11-19 — A diagnostic report of a running machine, on demand:** (`upc-f7j.16`;
  design/observation.md §Diagnostic report of a running machine). A guest that hangs without
  an error (interrupts disabled around a HLT or a loop, IRQ1 masked, the 8042's keyboard
  interface disabled) kept the machine running with nothing to show why; the CPU stop report
  exists only when the CPU core refuses something. With `UNEMUPC_DIAG=1` in the desktop's
  environment, Ctrl+Alt+Shift+D (⌘⇧D on macOS) writes the running PC's report to
  `<data>/unemupc/diagnostics/diag-N.txt` and the status line names the file; without the
  switch the chord does nothing, and its D never reaches the guest. `upcli run --machine FILE
  --diag-at N` writes the same report to stderr at step N and runs on. The report shows the
  retired count, virtual time, status and halt; every CPU register, FLAGS with IF, TF, DF,
  IOPL and NT decoded, MSW, CPL, the CS/DS/ES/SS caches, GDTR, IDTR, LDTR and TR, the 16 bytes
  at CS:IP and 16 words at SS:SP (RAM and ROM only; a device window is named, never read);
  the code trail, CS:IP after the last 64 run slices with repeats folded, kept by the machine
  outside the CPU; both 8259s; the 8042 (status, command byte, output buffer and source,
  pending parameter, last command, inhibit); each PIT counter; A20 and its sources; the CMOS
  shutdown byte and NMI mask. New read-only accessors: `X86JitCpu::diagnostic_state`,
  `Pic8259::diagnostic_state`, `I8042::diagnostic_state` (with the last command, host-only),
  `Pit8253::diagnostic_state`, `CmosRtc::diagnostic_state`, `A20Line::sources`. MEASURED with
  the open ROM `assembly::test_support::keyboard_masked_halt_rom` (masks IRQ1, sends 8042
  ADh, CLI, HLT): the report shows IF 0, halted after the HLT at F800:0019, IMR 02, the
  keyboard interface disabled and the trail ending at F800:001A. The open-BIOS FreeDOS boot
  with the trail on and a report at each of its ten poll boundaries stops at the golden's
  retired count and virtual time (8,800,473; 10,000,000) with the same screen, POST stream
  and device snapshots as without, and takes the same time with and without the trail. A
  report taken every 997 steps of a timer-driven STI/HLT guest, with 8042 bytes queued and
  PIT GATE2 low, leaves every device snapshot equal to a twin that took none. Code and stack
  bytes the segment's cache limit does not allow show as `--`; the desktop adds whether it
  holds the machine paused. Every golden keeps its transcript and retired count.
- **2026-11-19 — GitHub CI is green again after the NES core landed:** (`upc-dfs.42`). Three
  gates had failed on `main` since `fca6b542`. The guard lane's crate-boundary check reads the
  resolved graph with `cargo metadata --offline`, which fails on a cold Cargo cache or one
  restored from an older lock file; the lane now runs `cargo fetch --locked` first (it builds
  nothing). The goldens lane runs the ignored `user_rom_` tests too: the Ken's Labyrinth
  recipe build gave its builder `--fetch` consent, so without the user's files it tried to
  download LABFULL.ZIP and panicked; the user-file builds now have no fetcher, see the input
  missing and print the SKIPPED line (a failure under `UNEMUPC_REQUIRE_USER_ROMS=1`). The other
  `user_rom_` tests already skipped before reading anything. The large sparse-disk test was
  refused by the store's free-space check because the runner ran short of disk: the cached
  `target` directory, restored from older lock files, kept every earlier build and grew with
  each pin. The workflow now caches Cargo's downloads (reused across locks) apart from
  `target` (reused only for the same locks), builds without incremental data, and prints the
  disk left in the gate summary.
- **2026-11-19 — Release 0.0.3:** the workspace version is 0.0.3 and
  `image_store::compat::NEXT_RELEASE` is 0.0.4, so a row for a feature this
  release does not read now names 0.0.4. The release carries the `ibm-pc`
  platform id, the repository paths and revision tags, the frozen Zstandard
  profile, export file names on every layer, recipes through the lock and on
  a registry, the second platform (the NES core) and the delivery-stall stop.
  The maintainer tags and publishes it; then the six published artifacts are
  pushed again under the new paths.
- **2026-11-19 — A guest stuck delivering exceptions stops the machine with a report instead
  of freezing it; x86jit moves to `7a9ea390`:** (`upc-f7j.15`; design/observation.md §CPU
  stop report). On the 286 AT a protected-mode guest whose exception handler faulted at its
  first instruction was delivered the exception again and again; a delivery retires nothing,
  virtual time counts retired instructions, so the run's deadline was never reached and the
  desktop froze at 100% CPU. x86jit `7a9ea390` (xj-srh.26 (x86jit)) returns such a run after
  1024 deliveries in a row with no instruction retired, with its step budget unspent. The CPU
  backend runs on through 11 such returns (11,264 deliveries, about 7 ms in a release build):
  enough for the frames of a 16-bit stack to reach its bottom. A stack whose SP is not a
  multiple of the frame size (or whose limit is below FFFFh) then overruns, the processor
  shuts down and the AT board resets it, as on hardware, so a guest that leaves protected mode
  by a triple fault still gets its reset. A stack that wraps cleanly loops forever; that stall
  becomes the new `CpuError::DeliveryStalled`: "the processor is stuck delivering exceptions:
  11264 deliveries without an instruction, the last to CS:IP", CS:IP being the handler's first
  instruction. It fails the machine like any CPU refusal: the desktop shows it in the Machine
  error dialog with the CPU stop report (whose code bytes are the handler's faulting
  instruction), and `upcli run --machine` prints `stopped: cpu fault: ...` and the report and
  exits 1. The machine's run loop was the only loop that ran the CPU's `run_steps` in
  production; the PC core, the desktop session and `upcli` above it already stop on its error.
  MEASURED with two open ROMs in `assembly::test_support`: `delivery_stall_rom` (protected
  mode, #GP whose handler's `MOV DS,AX` faults again, 8-byte frames from SP F000h) stops with
  the report in the CPU backend, PC core, session and `upcli` tests, which hang past their 60 s
  deadline at the old pin; `delivery_overrun_rom` (a real-mode #UD handler that is itself an
  invalid opcode, 6-byte frames from SP FFFEh) shuts down after 10,922 deliveries, the board
  resets and the ROM's second start halts, where stopping at the first bounded return reported
  a stall instead. The reset after a shutdown was already complete (CS:IP F000:FFF0 with base
  FF0000h, MSW FFF0h, IDTR 0/3FFh, caches of limit FFFFh, engine state cleared); a CPU test now
  pins it, and the open at-pm-test golden's triple-fault stage resumes through the BIOS. The
  pin also brings x86jit's locked operand that spans RAM and a device region, now performed
  instead of refused (`e29dc1ae`). Every golden keeps its transcript and retired count at the
  new pin.
- **2026-11-19 — Recipes on a registry:** step B10 of design/recipes.md
  §19.2 ([decision-35 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-35%20-%20Recipes%20are%20OCI%20artifacts%20in%20a%20recipes%20namespace%20beside%20their%20outputs.md)). `upcli recipe push FILE REF` publishes a pinned
  recipe as an artifact of its own type (`application/vnd.unemu.recipe.media.v1`
  or `…firmware.v1`): the recipe as config, its work file, bundled inputs
  and lock files as titled layers, `unemu.recipe.output` and its digest,
  CC0-1.0, and `unemu.min-app-version` from the new rows for recipe
  versions and steps and the locked output's marker. The recipe guard
  refuses any other part, a bundled input that is not text, a recipe
  without `[license]` naming a file, file-set entry or `unpack` member
  under 256 bytes not marked `disclosable` (members take the key now,
  the maintainer's decision), another place than `NAMESPACE/recipes/…` at the output's
  tag, and a tag naming a recipe of another output digest (the same
  digest replaces it). `upcli recipe pull REF [--to DIR]` refuses a recipe
  that needs a newer unemupc before fetching any part, checks it and
  writes the folder it was pinned in; `upcli recipe plan REF` judges it
  from manifest and config alone. Tags a recipe build creates record the
  recipe's digest (`unemu.build.recipe`, index only; no digest moves).
  MEASURED (`cli-pc` test `recipe_registry`, mock registry): pushed,
  pulled into an empty folder, planned and built into an empty store to
  the pinned digest; each refusal tested; a row is two requests. No
  Remotes UI yet (B13). commands.md, Recipes on a registry.
- **2026-11-19 — Remotes says Newer available across releases again, from the title
  records:** (`upc-5pw.64.7`; design/machine-images.md §3). A selected set or package tag whose
  metadata record was read reads "Newer available · you have 1.3" when the library holds, under
  the local name a pull writes, records of the same title, edition, regions, languages and
  formats, all with an older `version` field. The rule is conservative
  (`image_store::tag::compare_versions`): only number segments decide, by value (1.3 < 1.4 <
  3.11 < 95, 6.2 < 6.20), trailing `.0` segments do not count, and a word or mixed segment that
  decides (1.4_beta and 1.4_rc, 4.00.950b and 4.00.950, 98_se and 98) makes no claim. Tags
  still never order two releases: a tag named like a higher version whose record says lower
  gets no hint. Another edition, format, region or language, a version that does not split
  into segments, a library record of the same software whose version cannot be compared, or a
  record that cannot be read makes no claim, with no error row. Revisions of one release keep
  the tag rule. The hint only appears once the tag is selected, since the list reads no
  metadata record. It is a hint only; nothing is replaced or pulled.
- **2026-11-19 — Recipes are scaffolded from the author's files; their
  inputs list for ROM managers:** step B9 of design/recipes.md §19.2.
  `upcli recipe scaffold FILE|FOLDER...` prints the `[input]` tables of
  the files named, with sizes and SHA-256; a zip archive also gets an
  `unpack` step listing every member in the archive's order with its size,
  hash and the archive's time, and a folder becomes a file set in the
  `fat` step's name order, without times, at most 100,000 files. `upcli
  recipe inputs FILE --format dat|csv|sha256sums` lists a recipe's inputs
  as a Logiqx DAT, a CSV or `sha256sum -c` lines, one folder's layout.
  MEASURED (`cli-pc` test `recipe_scaffold`): synthetic files scaffold
  byte for byte to text that passes `recipe check`, the three forms are
  byte for byte, `sha256sum -c` accepts the files. The B9 proof, on the
  maintainer's own files: LABFULL.ZIP scaffolds to the corpus's input
  table and 19 members with their times, the AZURAMON folder to its 561
  entries, both byte for byte. commands.md,
  Scaffolding recipes and listing their inputs.
- **2026-11-19 — Local images that need a newer unemupc:** the desktop's part of decision-30
  (`upc-dfs.15` part 2; design/machine-images.md §5.6). The summary cache, now schema 8, keeps
  each artifact's `unemu.min-app-version` and reads it before any config. An image, set or
  package this release does not read is summarised from its manifest and metadata and listed in
  its collection; one whose manifest, layers or metadata do not decode is still a ⚠ failure row,
  with the needs text as its reason. Every other cache schema is rebuilt. The library and the
  media picker show "Needs unemupc X or newer" in states they already have: a set's
  Availability, a package's Status and its misfit entry in New machine… and Settings › Firmware,
  the ⚠ of a machine made from such an image, an image's readiness line and name hover, and the
  picker's set line. Everything that reads or rewrites the config is disabled with that reason
  in the panes, menus, toolbar and keys, and refused in code: Start, New machine from image…,
  Settings…, Layers…, Push…, Save as new image…, Insert, Insert into…, Insert media… and the
  drives' Insert…, Use in machine… and Edit metadata…. Details and Delete tag… stay. Start's
  refusal names the machine's own tag, not the digest it keeps. The Images list's rows have no
  dimmed look yet; that waits for a mock-up (`upc-dfs.15.1`).
- **2026-11-19 — Pinned recipes find their outputs; a lost lock is rebuilt:**
  the two follow-ups of step B8 of design/recipes.md §19.2. A pinned
  recipe's build looks every output a step makes up by the payload its
  lock names, in the store, the download cache and the libraries (§3.4,
  "found instead of made"), before it plans the steps; a found output is
  read and checked by its SHA-256, and the steps and inputs only it needed
  are not run or read. `recipe plan` lists `outputs found instead of
  made:` (JSON `payloads`), `recipe build` prints `KIND ID (FILE): found
  WHERE, not made`; `recipe conformance` never finds instead of making.
  A pinned recipe with nothing at its lock's path (§7.2) is built with
  this build's writers in a scratch store: the pinned digest goes on as a
  locked build, another digest names both and tags nothing; `recipe
  plan` plans it so and says the lock is lost, conformance rebuilds it the
  same way, and `recipe check` still refuses it (rule 8). MEASURED (`pc-image` test
  `recipe_lock`): with its inputs gone and the outputs in the store or in
  a library, the synthetic pinned recipe builds its pinned digest; files
  with the outputs' names and sizes and other bytes are not taken; a lost
  lock rebuilds the pinned digest, and a build with another manifest
  writer is refused. No pinned digest changes; the FreeDOS sets now find
  their licence index with DOS line ends (`floppy-1.4/LICENSES.TXT` of
  the fixtures) instead of making it. commands.md, Planning recipes,
  Building recipes, Pinning recipes.
- **2026-11-19 — The session's PC tests have their own crate:**
  `crates/pc-session-tests` (layer `pc`, tests only, no library code)
  holds the 57 tests that drive a real PC through `machine_view::Session`
  and test no code of the desktop (design/core-contract.md §8.1.10): the
  session tests of the window's `main.rs`, the host mix levels, host
  audio, the mouse shim and the key table, under the same module paths.
  A change to the desktop's shell no longer runs them; a change to the
  session or the PC core does. MEASURED: the window's tests take 9.2 s of
  wall time (261 tests) where they took 30.1 s (318); no behaviour
  changes and no test is lost.
- **2026-11-19 — NES colours of the 2C02G:** unemunes re-pinned to `3aa95580`, which only
  changes its palette: each palette row is rotated by -5° of hue per row, the differential phase
  distortion of a 2C02G (NESdev "NTSC video"). Colour `$22`, a common sky colour, goes from a
  purple `B17BFF` to `968AFF`; row 0 is unchanged. The cartridge golden in
  [`crates/platforms/tests/nes_cartridge.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/platforms/tests/nes_cartridge.rs) moves to `0xFCDFD18611FED325` (its picture uses rows
  1 and 2), the same digest unemunes pins for that picture.
- **2026-11-19 — The NES core, compiled in:** step 16 of
  design/core-contract.md §13.6. Both programs carry `unemu-nes-core`
  from unemunes, pinned to revision `7c5abb52` (built against the same
  `unemu-core-api` v0.2.0 commit, so one copy of the contract stays in the
  build; `deny.toml` allows that git source and no other new one). The NES
  core follows the PC in `platforms::cores()`, so `--version` and `--about`
  of both programs gain the line `core unemunes 0.0.1 (unemunes), contract
  0.2`, and the desktop's File › Open… is enabled in the release: its dialog
  offers "NES cartridge images (*.nes)", and `--open FILE.nes` starts one.
  Only NROM (mapper 0) images run; another mapper or a damaged image is
  refused with the core's reason. The third-party licence list names `nes`,
  `unemu-nes-core` and `x65` (MIT OR Apache-2.0) with the licence files at
  the root of their repository, which `scripts/third-party-licenses.sh`
  now finds in a workspace checkout of several crates. A golden in
  `platforms` builds a checkerboard cartridge from our own bytes, opens it
  through `core_launch::prepare` and `start`, and pins 8 frames within
  80 000 instructions and the picture's FNV-1a digest
  `0x64CB5BF7C2F11325` for the pinned revision.
- **2026-11-19 — Saved state in the player:** step 15 of
  design/core-contract.md §13.6. A machine opened with Open… whose core
  lists saved state has Save state (Ctrl+Alt+S) and Load state
  (Ctrl+Alt+L) in its Machine menu after Stop: one quick slot per content
  and core, `unemupc/states/CORE/aa/DIGEST/quick.state` (the line
  `unemu-state 1`, the machine's state identity, then the core's bytes),
  written through a temporary file and a rename so a failed save keeps the
  state saved before. Load state is greyed until a state exists and refuses
  a damaged file or a state of another identity with a Load state box,
  leaving the machine running as it was. A machine without saved state has
  neither item. The PC's stay disabled and its guest still receives
  Ctrl+Alt+S and Ctrl+Alt+L. No released core opens files yet, so only the
  test core shows it.
- **2026-11-19 — Open… in the application:** step 14 of
  design/core-contract.md §13.6. The desktop takes `--open FILE [--core ID]`
  beside `--machine FILE` (the usage line changes by that, and `--core`
  alone or `--open` with `--machine` is a usage error) and has File ›
  Open… (Ctrl+O) above Open machine file…, which loses its key. The file
  is prepared by `core_launch::prepare` on a worker thread, opens in the
  host's dialog on "All supported files", and a file that does not start
  (no core opens it, a core cannot run it, several cores run it, it needs
  more) shows its refusal in the player's message box and leaves a running
  machine alone; one that starts asks to stop a running machine first with
  the stop prompt, as starting the PC does while a file's machine runs.
  The shell holds a `player_ui::Player` beside the PC's presenter, one
  machine at a time: Pause, Stop, Reset, Power cycle, Mute, Speed, Fast
  forward, Release and Full screen, the tick, the central panel, the title
  (content — platform — unemupc), the status line and raw keys and pointer
  go to whichever runs; the player's Machine, Media and View menus replace
  the PC's, the PC's commands are greyed and the toolbar's Ctrl+Alt+Del
  button reads Reset. Pad keys are read from and written to `pads` in the
  preferences; a file's machine keeps its records under `unemupc/saves`
  (states later under `unemupc/states`) in the data directory. With the
  release's cores of this step (the PC alone, which opens no files) File ›
  Open… was greyed with "No core in this release opens files"; the `test-core`
  feature of `platforms` adds the scripted test core for trying it by hand.
- **2026-11-19 — The player, the view of a machine started from a file:**
  step 13 of design/core-contract.md §13.6. The new crate `player-ui`
  (neutral, a window crate) holds one `dyn Machine` started from a
  `core_launch::Prepared` in a `machine_view::Session` and draws it as the
  approved `mockups/player.html`: the Machine menu (Pause, Reset when the
  machine lists a soft reset, Power cycle, Stop, Input… for a pad port),
  one Media menu with a section per removable slot (Eject, Next in
  catalogue, the catalogue's entries), the View menu (scale, Show overscan
  when the picture is cropped, full screen, mute, speed), the status line
  (message, Running or Paused, a chip per removable slot, platform,
  content, sound, speed), the screen in the machine's shape with overscan
  hidden by default and a cross-hair for an absolute pointer, the Input
  dialog (click a row, press a key; Esc leaves it, Delete clears it,
  Defaults) and its message boxes. A control the description does not
  offer is absent. Power cycle is a hard reset when listed, otherwise the
  machine finishes and starts again from its file. No behaviour changes:
  nothing shows the player yet; the application wires it in at step 14,
  and Save state and Load state come with step 15.
- **2026-11-18 — The launch path from a file to a machine:** step 12 of
  design/core-contract.md §13.6. `core-launch` gains `prepare`, `start`,
  `HostDirs`, `Prepared`, `Outcome`, `LaunchError` and `Cores::filters`
  (§13.3). `prepare` opens the file once and hashes all of it through the
  `unemu-core-files` reader the machine gets later (a file whose length or
  time changes is refused as changed), asks the named core alone or the
  cores that list the file's extension first, then the others, and checks
  that `validate` on the config the core wrote names the file and nothing
  else; a core that fails is left out and heard only when no core is
  left. `start` creates the machine with a content source that answers the
  file's digest alone and a `FileStorage` over `saves/CORE/aa/DIGEST/`. The
  refusals carry the mock-up's sentences (no core opens it, with what the
  cores open; recognised and cannot run it; several cores, with `--core`;
  needs more than itself; already running elsewhere). No behaviour changes:
  nothing calls it yet. New dependencies of `core-launch`:
  `unemu-core-files` (the contract's repository at the pinned revision)
  and `sha2`.
- **2026-11-18 — The 80286 runs calls through call gates; the CPU pin
  moves to `a11353e6`:** Windows 3.1 Standard mode stopped with
  `LegacyGateOrTaskTransfer` at a far CALL from ring 3 through a DPL-3 286
  call gate to DPL-0 code. x86jit `a11353e6` runs far CALL and JMP through
  call gates: an inward CALL onto the TSS stack of the target ring with the
  outer SS:SP, the gate's parameter words and CS:IP pushed, RETF n back,
  same-level and conforming targets without a stack switch, JMP to the same
  level only, the PRMs' faults; a 386 gate on the 80286 is #GP(selector).
  `LegacyGateOrTaskTransfer` is no longer produced; an inward CALL whose
  frame copy needs more than the core's 64 retry-ledger entries is refused
  as `LegacyInstruction`. Cargo.toml, Cargo.lock (only x86jit-core moves)
  and every golden's CPU revision follow; the 80286's identity is
  unchanged. The open privilege-level floppy gains case (e)
  (design/at-privilege-test.md): from ring 3, a far CALL through a DPL-3
  call gate copying two words to a DPL-0 handler, which checks SS0:SP0 and
  the frame order (outer SS, SP, the words, CS, IP), then RETF 4 back.
  MEASURED with the open AT BIOS: every line passes, 6,391,138 retired
  instructions, image SHA-256 `ac7ba0ab…f79f04`; at the previous pin it stops
  with `LegacyGateOrTaskTransfer at 0033:00f1`. Every other open golden keeps its outcome and retired count (the user-ROM goldens are run separately). The CPU
  stop report drops its note for a refusal during interrupt delivery (no
  refusal the core still makes is ascribed to it), and its test guest
  (`refused_gate_call`) now reaches ring 3 and calls through a gate with 31
  words with both stacks in the trapped text buffer, a frame the core
  refuses.
- **2026-11-18 — The library frame leaves the window; what is left is the
  application:** moves 6.4 and 6.5 of the window split
  (design/core-contract.md §8.1.9). `pc-ui` takes the machine library's
  frame (`library_view` with the Machines, Firmware and Installation media
  collections, Edit metadata…, Remotes and the layout tests) and the tests
  of the push, pull and Pull… models that drive it (`registry_transfer`,
  `remote_pull`); the fixtures the shell's tests drive the frame with are
  behind `pc-ui`'s `test-support`. The window keeps the shell, its menus,
  the preferences and `main`, the media dialogs' session half, re-exports
  under the old paths, and the seven tests that name the application (its
  preferences, the shell's hotkeys, the program's provider list). No
  behaviour changes: the workspace lists the same 4132 test names
  (MEASURED); 172 tests moved to `pc-ui`. The window's own tests take 26 s
  of wall time where they took 50 s (MEASURED, on a loaded host).
- **2026-11-18 — Core registration:** step 8 of design/core-contract.md
  §13.6. `platforms::cores()` is the one list of the cores compiled into
  both programs: the PC core alone, plus `unemu-test-core` with the
  `test-core` feature of `platforms`, which is off by default and which
  no release build sets. A new neutral crate, `core-launch`, starts with
  `Cores`: each core checked against the contract this program drives
  (`usable`, `refused` with "core NAME implements contract A.B; this
  program needs C.D", `by_id`) and the About lines. `upcli` and `unemupc`
  each make one `Cores` from the list, and their `--version` and
  `--about` print one line per core after the program's own lines:
  `core unemupc 0.0.2 (unemupc), contract 0.2`; a refused core's line
  ends with `, unusable: ` and the reason. Nothing else in either text
  changed, and `--help` and the usage text are unchanged (MEASURED: the
  binaries' output before and after differs by that one line). The test
  that every compiled-in core states a contract this program drives moved
  from the desktop to `platforms`, runs over `cores()` and also checks
  that core ids, platform ids and each platform's content-kind ids are
  unique. The About dialog of the desktop's menu is unchanged.
- **2026-11-18 — The PC dialogs and the command vocabulary leave the
  window:** moves 6.1 to 6.3 of the window split
  (design/core-contract.md §8.1.9). `pc-library` takes the models of File ›
  New machine… (`new_machine`) and of a machine file's Settings…
  (`settings_model`, which now opens with the library's machine-file
  directory that the shell passes), and the machine-file list with its
  readiness check (`machine_files`). A new `pc` crate, `pc-ui` (egui),
  holds Settings… with the image review (`settings_sheet`), the New
  machine wizard's view (`new_machine_view`), the disk-layer dialog
  (`layers_dialog`), the PC parts of the details panes (`pane`) and the
  desktop's command vocabulary (`commands`); the egui test helpers of the
  library fixture are `ui_kit::test_support`. The window re-exports
  everything under its old paths. No behaviour changes: every test keeps
  its name (4117; MEASURED) but for ten tests of `shell_model` that went
  with their code to `machine_files` and `commands`; 51 tests moved to
  `pc-library`, 29 to `pc-ui` and the picker's size test to `media-ui`.
- **2026-11-18 — `machine-view` on contract 0.2:** step 10 of
  design/core-contract.md §13.6. A new module, `machine_view::screen`,
  is the geometry every view but the PC's will show a picture with:
  `layout` gives the shown part (the visible part, or the whole picture
  with overscan shown) and its rectangle in a panel, in the shape
  `aspect × (shown width / width) ÷ (shown height / height)`, with the
  height a whole multiple of the shown rows under integer scaling;
  `ScreenLayout::pointer_position` maps a host point back to a place on
  the whole picture, or `None` off the screen. The session aims at a
  machine's absolute pointer instead of capturing (`Session::pointer_at`,
  buttons through `mouse_button`, `None` at focus loss), resets by kind
  (`Session::reset`, which paces a halted machine from the reset) and
  saves and loads a state (`Session::save_state`,
  `load_state`, which re-anchors pacing, drops the sound queued before
  the load, shows the loaded picture at once and releases held input; a
  state of another identity is refused with the core's message). Tested
  against `unemu-test-core`, the `machine-view` row of §13.3's test
  table. No behaviour changes: the PC keeps its 4:3 screen, nothing calls
  the new calls yet, and no PC test or pin changed.
- **2026-11-18 — The PC media views leave the window:** moves 5.1 and 5.2
  of the window split (design/core-contract.md §8.1.8). A drive's
  Catalogue submenu now answers with the disk chosen, or All N disks…, and
  the shell turns that into its command, so the submenu no longer names
  the shell. A new `pc` crate, `media-ui` (egui, `pc-media`, `ui-kit` and
  the PC crates the moved views already named),
  holds the media picker (`media_picker`), Create from files…
  (`media_create`) and the Catalogue submenu (`catalogue_menu`); the
  window re-exports them under their old paths. No behaviour changes:
  every test keeps its name (4078; MEASURED); 41 tests moved to
  `media-ui`, and the picker's size test, which uses the library's modal
  helper, stays with the window.
- **2026-11-18 — A scripted test core, `unemu-test-core`:** step 9 of
  design/core-contract.md §13.6. A new neutral crate, `crates/test-core`,
  whose only dependency is the contract: `TestCore` implements all of
  contract 0.2 and emulates nothing. Its content is a `utc` text script
  (first line `unemu-test-core 1`) that sets the picture's size, aspect
  and visible part, the clock, the sound's rate and tone, the input ports
  (keyboard, relative and absolute pointers, pads), slots with a
  catalogue, the features, battery memory and events at a virtual time
  (halt, power-off, a guest reset, a failure, a message); the grammar is
  in the crate's documentation. `identify` answers `None` without the
  first line, `Unsupported` for an `unsupported REASON` line and otherwise
  a config naming the script by digest. Its picture shows the frame
  count, the held pad controls and a cross per pointer; its sound is a
  square wave with exactly `floor(now × rate / clock)` frames; its battery
  record counts the machine's starts; a saved state is its counters. It
  is in no program's list of cores, so nothing a program does changes;
  the front end's crates will test against it. The boundary check gains
  the rule that the test core depends on the contract alone.
- **2026-11-18 — The 80286 runs code at CPL 1–3; the CPU pin moves to
  `b4e64dff`:** Windows 3.1 Standard mode stopped with `unsupported cpu
  system operation LegacyPrivilegeTransition` at its first IRET from ring 0
  to ring 3. x86jit `b4e64dff` runs protected-mode code at CPL 1–3 on its
  80286 model: RETF and IRET to an outer ring (a data segment register of a
  lower DPL becomes null), interrupts, exceptions, IRQs and NMI through a
  gate to an inner ring on the TSS's ring stack, task switches into rings
  1–3, and the CPL and IOPL checks (CLI/STI, POPF/IRET, port I/O, the
  system instructions, LOCK). A far CALL or JMP through a call gate is
  still refused (`LegacyGateOrTaskTransfer`). Cargo.toml, Cargo.lock (only
  x86jit-core moves) and every golden's CPU revision follow. A new open
  floppy, `vendor/at-privilege-test` (design/at-privilege-test.md), IRETs
  to ring 3 with IOPL 3, takes INT 40h back to ring 0 on SS0:SP0, IRETs to
  ring 3 again with a DPL-0 ES that the return nulls, and takes #GP(0) for
  CLI at ring 3 with IOPL 0. MEASURED with the open AT BIOS: every line
  passes, 6,390,307 retired instructions; at the previous pin it stops with
  `LegacyPrivilegeTransition` at the first IRET. Every other open golden
  keeps its outcome and retired count (the user-ROM goldens are run
  separately). Two tests that used a refused outward
  RETF as their CPU failure changed: the CPU stop report test now reaches
  ring 3 and is refused at a call gate there, and two `upcli run` tests
  fail on a LOADALL table the core refuses.
- **2026-11-18 — This repository on contract 0.2:** step 7 of
  design/core-contract.md §13.6. The workspace pins release v0.2.0 of
  `unemu-core-api` (commit `56c4640`), where content and storage are
  objects a front end hands in and the contract names no path. The PC core
  states contract 0.2, lists no content kind a person opens (a PC starts
  from a config) and still refuses `create` and `add_medium`, so the PC is
  started by its provider as before and nothing it does changes. The
  desktop's picture now carries the visible part a machine states (the
  whole picture for the PC); nothing shows it yet. MEASURED: one copy of
  the contract in the build, and none of the contract's old path
  signatures in it or in this workspace.
- **2026-11-18 — The library's neutral views leave the window:** moves 4.1
  to 4.3 of the window split (design/core-contract.md §8.1.7). A new
  neutral crate, `library-ui` (egui, no PC crate), holds the list tables
  every collection and Remotes draw with (`table`), the Save as new image…
  form and its questions (`save_image`), the Push…, Pull by reference… and
  Pull… dialogs (`registry_transfer_view`) and the Remotes group with its
  details pane (`remotes`). Remotes no longer reaches into the library
  frame: the frame lends it the platform facts, the skin, the local tags
  and the stopped machines' drives for one call and applies, in their
  order, the browsing requests and the frame changes it asks for. About
  3 600 lines of registry views have no PC dependency by Cargo. No
  behaviour changes: every test keeps its name (4074; MEASURED); the save
  form's drawing test moved to `library-ui`, and the tests of Remotes and
  the dialogs, which drive the frame or plan PC images, stay with the
  window.
- **2026-11-18 — Recipe locks and pinning:** step B8 of design/recipes.md
  §19.2. `upcli recipe pin FILE` builds a draft, then writes its lock,
  `FILE.lock/sha256/<hex>` (the manifest, the config and every layer no
  step made, the bytes the store holds), and `digest = "sha256:…"` after
  `revision`. A pinned recipe's build checks the lock before reading any
  input (also in `recipe check`, rule 8: each file's hash, the descriptors,
  and the lock's decoded fields against the recipe), runs the steps,
  checks every output against the locked manifest (decoded, then stored
  SHA-256 and size under encoder profile 1) and publishes the lock's bytes
  unchanged; a mismatch names the output and both hashes and tags nothing
  (`--keep-as` keeps this build's result under its own tag). A work file
  edited since the pin is drift: listed by `check`, `build` and `show`,
  never published; `recipe show` of a pinned recipe prints the lock's
  record. `upcli recipe conformance FILE|FOLDER... --library DIR` builds
  every pinned recipe into a scratch store and compares digests. The
  corpus's six open recipes are pinned: FreeDOS `1.4-r1` `1edc14e3…`,
  `1.4-1.2m-r1` `c8129f6c…`, `1.4-720k-r1` `dd65877d…`, and the firmware
  packages `unemu-at-bios` `e3c226ae…`, `vgabios` `179a11db…`, `xub`
  `7d42e80e…` (the template packages' digests). MEASURED: a pinned recipe
  builds its digest, the lock's manifest byte for byte, with a publisher
  whose manifest writer differs (another annotation, another order, other
  whitespace), whose own draft of the recipe gets another digest; the
  corpus pins conform from the fixtures. (Locked payloads found instead of
  made: 2026-11-19, above.) commands.md, Pinning recipes and Recipe
  conformance.
- **2026-11-18 — The contract has its own repository:** step 3 of
  design/core-contract.md §13.6. `unemu-core-api` is no longer a workspace
  crate: the workspace pins release v0.1.0 of
  [unemu-org/unemu-core-api](https://github.com/unemu-org/unemu-core-api)
  (`MIT OR Apache-2.0`) by its exact commit, as it pins x86jit, and
  `crates/core-api` is gone. Its source there equals the old directory but
  for the licence lines, so nothing behaves differently; its seven unit tests
  run in that repository's CI. `scripts/crate-boundaries.sh` now also fails
  when a build holds more or fewer than one copy of the contract, naming each
  copy's source and users. A local checkout is used through a gitignored
  `[patch]` (dev-setup.md, "The contract: unemu-core-api").
- **2026-11-18 — Sound at any rate, pads from the keyboard:** steps 1 and 2
  of design/core-contract.md §13.6, in `machine-view`. The session plays a
  machine's sound at the exact rate its description states, any rate from
  8 000 to 192 000 frames a second (984 375/22 Hz, the NES's, included),
  converted to the device's rate; only a rate outside those bounds disables
  audio, with one warning. A 48 kHz source converts with the arithmetic it
  had, so the PC's sound is unchanged bit for bit (MEASURED against digests
  recorded before the change), and the PC holds the contract's frame-count
  rule (MEASURED). A machine without a keyboard port gets its pads from
  bound keys: by default the first player has the arrow keys, X = A, Z = B, S = X,
  A = Y, Q = L, W = R, Enter = Start and right Shift = Select
  (`machine_view::pads`); keys have stable names for a preferences file
  (`keys::name`, `keys::by_name`). The PC has a keyboard port: its keys are
  unchanged. Nothing in the desktop shows a pad machine yet.
- **2026-11-18 — The library's worker leaves the window:** moves 3.11 and
  3.12 of the window split (design/core-contract.md §8.1.6). `pc-library`
  (layer pc, no toolkit) now holds the library's worker thread
  (`library_worker`), which wakes the frame through a plain function
  instead of an egui context, the PC's part of the registry models
  (`pc_desktop`) and the image a workspace pins for the save form
  (`layers_dialog::SourceImage`); behind its `test-support` feature, the
  library's fixtures, the synthetic firmware catalogue and the tests' hang
  guard. Cargo now keeps the worker from naming the window. The worker's
  tests and those of `image_settings`, `hardware` and `remote_browse` that
  draw nothing moved with it: 67 tests leave the window's test binary.
  No behaviour changes: every test keeps its name (4044; MEASURED).
- **2026-11-18 — Recipe builds:** step B7 of design/recipes.md §19.2.
  `upcli recipe build FILE [--fetch] [--library DIR]... [--store DIR]
  [--keep-as TAG]` builds a draft: the plan, the downloads `--fetch`
  consents to (HTTPS only, listed first, kept in the store's
  `cache/downloads/` by SHA-256 only with the pinned size and hash), every
  value read and checked again, the steps `unpack` (zip, stored and
  deflate, names checked, never guessed), `text` (ASCII, CR LF) and `fat`
  (the FAT12 builder) as pure functions of the bytes, then the importers'
  own plan-and-publish functions, which tag last; nothing is tagged when an
  input is missing or anything fails. The installation-set import now gives
  a set's title record the id of its repository's last component (recipes
  §6.4) instead of `title`; older sets keep theirs until published again
  with catalogue edits, which moves their personal data to the new id.
  The metadata layer of every imported set changed with it, so the FreeDOS
  set pins moved: `1.4-r1` `545f287b…` → `2ee5d72d…`, `1.4-1.2m-r1`
  `53e81e4e…` → `1320a287…`, `1.4-720k-r1` `29c2d9d2…` → `bd3a3007…`
  (the firmware packages and templates keep theirs). The published sets
  must be rebuilt and pushed again with the release. MEASURED: the open firmware recipes (the corpus gained the video BIOS and
  the option ROM) build the template packages digest for digest; the three
  FreeDOS recipes build sets with the importer's configs byte for byte;
  Ken's Labyrinth and AZURAMON build from a user's own files, Ken's disk the
  one the FAT goldens pin. Attachments are not built yet. commands.md,
  Building recipes.
- **2026-11-18 — CPU stop report:** when the CPU core refuses an operation
  (`unsupported cpu system operation LegacyPrivilegeTransition`, say), an
  instruction it does not lift or a fault it returns outward, the stop now
  carries a report: CS:IP, MSW, CPL, A20, the CS and SS caches, GDTR, IDTR,
  LDTR, TR, the 16 bytes at CS:IP, and for far CALL/JMP, INT n and RETF/IRET
  the descriptors they reach with their 8 bytes (a gate and its target), the
  return frame at SS:SP and the 286 TSS's ring stacks. Only RAM and ROM are
  read. The Machine error dialog shows it in monospace; every message dialog
  now has Copy beside OK. It is saved as `diagnostics/last-cpu-stop.txt` in
  the machine's workspace (the status line names the path), and a refusal
  the instruction shown cannot make is noted as one that may have come while
  delivering an interrupt or exception. `upcli run --machine`
  prints it on stderr. No change for running guests. design/observation.md
  §CPU stop report; commands.md, Build / run / test.
- **2026-11-18 — The library's PC models leave the window:** the rest of
  move 3.7 and moves 3.8 to 3.10 of the window split
  (design/core-contract.md §8.1.6). `pc-library` (layer pc, no toolkit)
  now also holds the frame's query and the worker's snapshot of every
  list (`snapshot`), the Machines, Firmware and Installation media
  collections' rows, facets, labels, scans and imports (`machines`,
  `firmware`, `install_media`), a firmware package's statement and the
  worker's sides of Edit metadata… (`firmware_metadata`), the details
  panes' hardware facts (`hardware`) and Settings… on an image
  (`image_settings`). The window keeps the views and re-exports the models
  under their old paths; the statement editor's and a machine row's
  drawing methods became window traits. Every item the library worker
  names of the frame now lives below it. No behaviour changes: every test
  keeps its name and place (3962; MEASURED).
- **2026-11-18 — Recipe libraries, the index and the plan:** step B6 of
  design/recipes.md §19.2. `upcli recipe plan FILE [--json]` resolves every
  input of a recipe that passes the checks: from the store (by the decoded
  digests of its layers), beside the recipe (bundled inputs) and from the
  user's libraries, including members of zip archives (stored and deflate,
  our own central-directory reader and `miniz_oxide`, decoding no further
  than the size the archive states), with members of an
  `unpack` step found instead of made; it reports where each input was
  found or why not, near misses, the steps that would run and the outputs;
  exit 0 when everything resolves, 1 when something is missing.
  `upcli library ls|add|remove|scan` keeps the list (the preferences key
  `libraries`) and the index, a rebuildable cache in the store's
  `cache/libraries/` that keeps each file's hash under its size and change
  times and hashes only files of a wanted size. CSV and `SHA256SUMS`
  catalogues are hints. MEASURED: AZURAMON's plan finds all 561 entries in
  204 files of the user's copy, 204 hashes, and none on a second plan.
  commands.md, Planning recipes and libraries.
- **2026-11-18 — Push, pull and Remotes' models leave the window:** moves
  3.4 and 3.5 of the window split (design/core-contract.md §8.1.6).
  `library-model` (layer neutral) now also holds Image › Push… and Pull…
  (`registry_transfer`), browsing Remotes (`remote_browse`) and Pull… from
  them (`remote_pull`), and the desktop's provider seam (`DesktopPlatform`,
  `PlatformFacts`, §8.1.4): what the models ask of a platform (validators,
  the blobs it restricts, its name and id, its media format names and
  payload labels) the caller hands in. The PC's answer, `PcDesktop`, stays
  in the window until the worker moves. The window re-exports the models
  under their old paths. No behaviour changes: every test keeps its name;
  11 tests moved to `library-model` and 4 are new (3962; MEASURED).
- **2026-11-18 — The desktop's PC media models and firmware choices
  leave the window:** moves 3.6 and 3.7 of the window split
  (design/core-contract.md §8.1.6). `pc-media` (layer pc, no toolkit)
  holds the floppy catalogue drafts, the blank-disk form and the pending
  floppies of `media_dialog`, `created_media`, `kept_catalogue`,
  `drive_chips`, and the sets and short disk names of the picker and the
  Catalogue submenu they use; `pc-library` (layer pc, no toolkit) starts
  with `firmware_choices`. The window re-exports both under the old module
  paths; the Insert dialog's library form and Start's insert of pending
  floppies stay in the window. `hardware` and `image_settings` wait for the
  listing types of move 3.3. No behaviour changes: every test keeps its
  name (3958 before and after, 40 now in the two new crates; MEASURED).
- **2026-11-18 — The desktop library's first neutral models:** moves 3.1
  to 3.3 of the window split (design/core-contract.md §8.1.6). A new crate,
  `library-model` (layer `neutral`, no toolkit, no PC crate), holds the
  Preferences › Registries page model, the rows of the image lists and the
  Save as new image… form's model; the window re-exports them under their
  old paths and keeps the views. The version-tag order (`precedence`) moved
  from `pc-image` to `image_store::tag`, and `image-store`'s
  `test-support` feature gained a `TempDir`. The boundary check counts
  `library-model` with the window crates. No behaviour changes: the
  workspace lists the same test names apart from the crate (25 tests moved
  to `library-model`).
- **2026-11-18 — The command line in three crates:** steps 5 and 6 of
  the CLI split (design/core-contract.md §7.1, §7.5). `cli` is gone:
  `cli-core` (layer neutral) holds the platform-neutral command line, its
  provider seam and the journal tools' tests; `cli-pc` (layer pc) the PC's
  platform, every PC command, the PC command tests and the front-end
  goldens; `upcli` the binary, its `--version` and `--about`, the tests that
  run the binary and the pin of the documented command lines. The crate
  boundary check now keeps the neutral command line free of PC and
  emulator crates, so the source scan shrank to `#[path]` and `include!`.
  The PC's provider list is `pc_image::with_providers`, which `platforms`
  calls. Build and lint with `-p upcli`. No behaviour changes: every test
  keeps its name and the binary's output over 319 invocations is
  byte-identical (MEASURED).
- **2026-11-18 — The desktop's machine window drives a `dyn Machine`:**
  second step of the window split (design/core-contract.md §8). Three new
  crates: `unemu-core-api`, the front-end contract of §3 (layer
  `contract`, no dependency at all); `unemu-pc-core`, the PC behind it in
  stage A (`PcMachine::from_assembled` over the provider's assembled
  machine; `PcCore::create` answers `Unsupported`), which took the scan-code
  and mouse encoders, the 8042 back-pressure, the deadline-to-cap
  conversion, drive-activity sampling, the CP437 rasteriser and its CC
  BY-SA fonts from the window; and `machine-view` (layer `neutral`), the
  paced session over the contract with host audio, the HID key map,
  pointer capture and the macOS system-key tap. The window's machine
  window is a `machine_view::Session<PcMachine>`; its media dialogs still
  reach the PC catalogue through `PcMachine::assembled`. No behaviour
  changes beyond two edge cases listed in core-contract.md §8 (a capture
  release with several guest buttons held, media keys on a machine with a
  B: but no A:). The workspace lists the same 3902 test names apart from
  the crate, plus 26 new tests; the licence list differs only in the
  fonts' paths; `deny.toml` allows CC-BY-SA-4.0 for `unemu-pc-core`
  instead of `window`.
- **2026-11-18 — The command line's provider seam:** step 4 of the CLI
  split (design/core-contract.md §7.2, §7.5). Inside the `cli` crate the
  platform-neutral command line (`cli::cli_core`: the top-level parser,
  the usage text, the exit codes, the neutral image, registry, recipe and
  journal commands) reaches a platform only through the `CliPlatform` and
  `PlatformCommand` traits, and the PC's part (`cli::cli_pc`) owns every
  PC command and answers the neutral commands' questions: validators,
  catalogue restrictions, compression, pinned images, the recipe provider
  and media format names. A source test keeps the neutral modules free of
  emulator crates, and a pin test holds the usage text and the parse of
  every documented command line. No behaviour changes: output, exit codes
  and written files are byte-identical over 319 invocations (MEASURED).
- **2026-11-18 — Work files merge, and `upcli recipe show`:** a media
  recipe and the work file it names merge into one description record,
  field by field as design/recipes.md §6.16 rules (B5): a single value is
  the recipe's when it states one; a recipe's list of publishers,
  developers or genres replaces the work's whole; labels are the union, the
  work's first; `[ids]` and `[needs]` merge per key; the description falls
  back to the synopsis. `recipe::merge_path` checks the file and merges it,
  and `Merged::title` and `Merged::metadata` give the title metadata of
  schema 2 a build will write, which encodes and decodes unchanged.
  `upcli recipe show FILE` prints the record in the lines of `upcli image
  media show`, each value from the work file marked, or the check's
  findings with exit 1. The seven corpus recipes of the three folders with
  work files that had a first round give its full records field by field;
  the labels of Ken's Labyrinth 2.01 differ in order only (MEASURED).
- **2026-11-18 — The desktop's skin and widget helpers move to a `ui-kit`
  crate:** first step of the window split (design/core-contract.md §8).
  The new crate (layer `neutral`; the boundary check counts it as a window
  crate, so it may not depend on a CLI crate) holds the skins, fonts, title
  strips, bevels and text marks (`ui_kit::skin`), the mock-up icons and
  their inventory, the library's details-pane layout, the brand logo, the
  host file dialogs and the clipboard shortcuts, with the DejaVu symbol
  font and the egui font notices. The desktop uses them from there and
  keeps what needs the shell or PC firmware. No behaviour changes: the
  workspace's 3902 test names are the same apart from the crate of the 22
  moved tests (MEASURED), and a change to the kit, an approved mock-up or
  the brand assets now selects `ui-kit` and the window.
- **2026-11-17 — Recipe crate and `upcli recipe check`:** the new
  platform-neutral crate `recipe` reads Mediafiles, Firmwarefiles and work
  files of recipe version 0.1 into a typed model and runs every static
  check of design/recipes.md §6.11 without reading an input (B4): TOML 1.0
  only (the TOML 1.1 forms the `toml` crate accepts are refused in a pass
  over `toml_parser`'s events), unknown keys, types and values with their
  key paths, the kind's tables, references and unused values, the open
  statement, the provider's vocabularies, digests without a lock, the
  limits, and sizes, formats, names and ROMs the provider refuses. Every
  finding is reported with file, line, column, rule and key path.
  `pc_image::recipe::PcRecipes` is the `ibm-pc` provider. `upcli recipe
  check FILE|FOLDER...` passes the corpus's `ibm-pc` files; the NES and
  Amiga recipes each get the one rule-7 finding that this build has no
  provider for their platform, and with test providers for the two
  platforms all 18 files pass (MEASURED). [`design/recipe-schema-0.1.json`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipe-schema-0.1.json) is now
  written from the parser's key tables and the providers, and a test
  compares it. Building, fetching and planning are later beads.
- **2026-11-17 — A listing no longer reads another listing's probe as a
  running machine:** a machine's state probe took the workspace lock
  exclusively for an instant, so two listings at the same moment (the
  window's relist after Discard and a read of the same machine, or the
  window and `upcli image machine ls`) could show a stopped machine as
  Running. The probe now asks for a shared lock; a session's exclusive
  lock still reads as running. The flaky window test that caught it failed 6 of 400
  runs under load before and 0 of 400 after (MEASURED).
- **2026-11-17 — The 80286 executes LOADALL; the CPU pin moves to
  `cf32ec2e`:** x86jit `cf32ec2e` runs the undocumented LOADALL (0F 05) on
  its 80286 model, which HIMEM.SYS 3.x uses to copy extended memory from
  real mode; the previous pin raised interrupt 6 there, so DOS stopped with
  "Invalid Opcode" at the first XMS move. Cargo.toml, Cargo.lock (only
  x86jit-core moves) and every golden's CPU revision follow. A new open
  floppy, `vendor/at-loadall-test` (design/at-loadall-test.md), moves a
  block to `150000h` and back through LOADALL-loaded ES and DS caches and
  checks the guard at their A20 and selector alias. MEASURED with the open
  AT BIOS: every line passes, 6,404,154 retired instructions; at the
  previous pin the same image reports `loadall write FAIL 0005` and
  `loadall read FAIL 0015`. Every other golden keeps its outcome and
  retired count. Three synthetic-firmware tests (`firmware_catalogue` and
  two in `landmark`) used 0F 05 as an undefined 286 opcode; they now use
  0F FF.
- **2026-11-17 — Machine runners and emulation goldens out of the
  command-line crate:** a new `pc-harness` crate (layer `pc`) holds what
  was `cli`'s library: the machine runners, the hand-encoded guest
  programs, the key scripts and `StopReason`. Its `tests/` hold the 33
  emulation goldens that never use the command line, the binary or the
  store, and `tests/support/`, which the other crates still include by
  `#[path]` (a dependency would put a `pc` crate under `assembly`'s
  tests). `upcli`'s `--boot`, `--bios` and `run` call the same runners
  through `pc-harness`; the command tests and the eight goldens that need
  `upcli`, the command line or the store stay in `cli`. No behaviour
  changed: the workspace lists the same 3 844 tests (51 ignored) under the
  same names, upcli's output is byte-identical over 67 invocations (usage,
  every subcommand's errors, `--boot`, `--bios` and `run --machine`), and
  no golden changed. A CLI-only change no longer builds or runs the
  emulation goldens: 24 test binaries instead of 57 and 7 ignored tests by
  name instead of 32, 57 s instead of 95 s (MEASURED, loaded machine).
- **2026-11-17 — FAT builder additions for recipes, and a data-disk boot
  sector:** `media::fat12::plan_volumes` builds FAT12 diskettes from files
  given in memory, as a recipe's `fat` step will (design/recipes.md §6.9,
  B3): listed or name order, a time per entry (creation, access and write
  fields), a chosen serial, a label with its time, names refused with the
  rule they break (lower case included) and never renamed, folders deeper
  than DOS allows refused, and a split to exactly N volumes whose error
  names the number it needs. Name order now writes every folder's files
  first, then its folders, each by the stored 8.3 names, for recipes,
  `media create --add` and Create from files alike; with the default keys
  a volume equals `media create --add`'s. Every disk the builder makes,
  `media create --format` included, now carries our own boot code
  ([`crates/media/src/fat12/boot.asm`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/media/src/fat12/boot.asm), 8086 instructions only) in place of
  the halt loop: started from the disk, the machine prints that it is not
  a system disk, waits for a key and boots again through INT 19h. MEASURED
  on the Xi8088 BIOS and on the open AT BIOS without option ROMs
  (`boot_sector_golden`, the message twice, a key between). Against the
  previous `main`, blank images and a split changed in sector 0 only
  (bytes 63 to 160) and filled disks also in their folders' entry order,
  with the same files, times and contents (MEASURED); the pins were taken
  again, and synthetic volumes pin each addition. A test assembles the
  source with nasm and compares it with the bytes on the disk. Ken's
  Labyrinth from the author's LABFULL.ZIP builds twice to the same, now
  pinned, image (MEASURED, user-file lane); AZURAMON's split test needs
  the user's files and was not run here.
- **2026-11-17 — Crate layers and the desktop off the command-line crate:**
  every workspace crate names its layer (`contract`, `neutral`, `emulation`,
  `pc` or `application`) in `[package.metadata.unemu]`, and
  `scripts/crate-boundaries.sh` checks the rules of core-contract §7.3 over
  normal, build and test dependencies, with a self-test on fixture metadata
  and a shrink-only allow list (`scripts/crate-boundaries.allow`, today only
  `assembly` → `media`). It runs in the guard lane of CI. The desktop no
  longer depends on `cli`: registry clients moved to
  `image_registry::accounts`, the preferences path to
  `image_store::location::preferences`, image creation to
  `pc_image::media_create`, and the provider list, the licence notice and the
  push plan to the new `platforms` crate; `platforms::push_plan` binds the
  neutral `image_registry::push::plan_with_catalogue` to the PC firmware
  catalogue (`pc_image::firmware::restricted`), so neither program can plan a
  push without it. `cli` re-exports them; `upcli`'s usage, output and
  exit codes are unchanged. A change to `cli` no longer rebuilds or tests the
  desktop.
- **2026-11-17 — A library machine's drive catalogue survives Stop:** disks
  that went into a running library machine (a set's disk from Insert…, an
  image file from Browse for an image file…, the disks Create from files…
  made) joined the drive's catalogue for that run only, so a 10-disk split
  made on a running machine had to be inserted again after every restart. At
  Stop the window now hands each floppy drive's catalogue, with the media
  changes queued for the next run boundary counted, to the library worker,
  which keeps it in the machine's workspace, `media/created/catalogue.toml`
  beside the created disks' pending insert, never in `local/machines.toml`,
  which released versions read with unknown fields refused. Each disk is kept
  by identity: the image's own disk by name and SHA-256, a set's disk by the
  set's manifest, medium id and the tag it came from (never a moving release
  alias), any other image by its SHA-256 among the machine's created images.
  A browsed file is copied there at Stop as a read-only base named by its
  SHA-256, and a journal Insert put beside it is copied into the machine's
  media folder, so the record holds no host path and Duplicate and Gaming
  mode never share that journal. The record keeps each disk's write setting,
  format and place and the disk in each drive; the same disk inserted again
  is kept once, and a catalogue that is the image's own leaves no record.
  The next Start puts the catalogue back into the prepared session through
  the same path as pending inserts and created disks, with the disk that was
  in the drive in it from power-on: assembly's `FloppyMediaList` gained an
  `inserted` index (0 for machine files) and the uPD765 model
  `set_media_list_from`, so the guest sees that disk with the change line
  clear, exactly as a catalogue's first disk. Conflict rule: a pending insert
  for a drive wins that drive (its disks lead and its chosen disk goes in),
  and the kept catalogue follows in its order without the insert's disks
  twice; the image's own disks the record does not name come last. A disk
  that cannot come back, and a record that cannot be read or that a newer
  version wrote, stay out with a Start note; the Start goes on, and a Stop
  leaves a newer version's record as it is, with a note. Duplicate and Gaming
  mode carry the record (a Gaming-mode copy keeps its own at its Stop unless
  it is discarded), Reset to image… drops it and its confirmation lists it,
  and Save as new image… keeps it as it keeps created disks, with fresh
  journals: the writes to kept set, created and browsed disks start afresh,
  and the new image's own disks are matched by name. On a stopped machine the
  Catalogue submenu and the picker's This machine list the kept catalogue as
  waiting for Start, after a pending insert's disks, each with its own
  format; choosing a disk there makes it the one that goes in (a kept one
  takes the pending insert out). The stopped machine's drive chips still show
  only pending and created disks. `crates/window/src/kept_catalogue.rs` holds
  the record, the restore and their tests; library worker, shell, assembly
  and uPD765 tests cover it.
- **2026-11-17 — Metadata schema 2: needs, requirements and the work:**
  a title record may now state what it needs from its platform family
  (`needs`: kinds of feature with alternatives, best first), the box's own
  `requirements` text, and the work it is a release of (`work_id`,
  `synopsis`, and `work_ids` in Wikidata, MobyGames or IGDB), as recipes
  §6.4, §6.7 and §6.16 specify (machine-images §2.1). The writer uses schema
  2 only when a title sets one of these, chosen from the new
  decision-30 rows (`image_store::compat::TITLE_FIELDS` and
  `METADATA_SCHEMAS`, both 0.0.3 for schema 2), so every schema-1 artifact
  keeps its bytes and the pinned FreeDOS and firmware digests do not move;
  metadata never raises `unemu.min-app-version`. Readers read schemas 1 and
  2 strictly; a schema-1 blob naming a new field is refused and shown
  read-only; release 0.0.2 shows a schema-2 catalogue read-only and still
  runs the artifact (measured with its decoder and store). The provider seam
  `image_store::needs::Vocabulary` is data (ordered, quantity and choice
  kinds, `none` in every choice kind); `pc_image::needs::PcNeeds` holds the
  accepted, append-only `ibm-pc` vocabulary, and an unknown kind or value is
  refused when a set is planned, never at run time; a quantity states one
  minimum. Providers reach the vocabulary through `ConfigValidator::needs`
  and `MediaFormats::needs`. Readers likewise keep a work-id catalogue a
  later release adds; only writers name just Wikidata, MobyGames and IGDB.
  The manifest carries the needs as `unemu.needs`
  (`cpu=286+;memory=640K;sound=sb,adlib,none`). `upcli image media import`
  takes `--need`, `--requirements`, `--work` (which must equal the last
  component of the tag's repository), `--synopsis` and `--work-id`, and
  `image media show` prints them (commands.md). The summary cache moves to
  schema 7, so rows an older build summarized read-only are rebuilt. The library's mismatch warning and template suggestion,
  and editing the new fields in the library, are later work.
- **2026-11-17 — Every layer of a set or package names its export file:**
  each layer descriptor of an installation set and of a firmware package
  carries `org.opencontainers.image.title` with the file it unpacks to
  ([decision-37 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-37%20-%20Every%20artifact%20exports%20to%20plain%20files%20without%20loss.md) point 3 and its amendment, recipes.md §3.10 and §19.1 A3), so
  an ORAS pull names the files: a disk, ROM image or chip, source archive or
  extra its input's file name as given, with `.zst` after it on a layer
  stored compressed (`x86BOOT.img.zst`, `bios-unemu-at.bin.zst`), so `zstd -d`
  gives back the original file; a set's metadata `metadata.json`, a package's
  README `README.md` as before, the config none (machine-images §2, Export
  file names). Disks that Create from files keeps in the library export as
  `<top folder>-NN.img` (`AZURAMON-03.img`). `image media import`,
  `image firmware import` and `edit`, the window's imports and the template
  and media builders refuse, before anything is stored and naming both
  files, a name that one of macOS, Linux and Windows would not take
  unchanged, a name the export writes itself (`config.toml`,
  `metadata.json`, `manifest.json`, `SHA256SUMS`, `EXPORT-VERSION`), or two
  files whose names are equal ignoring case (an extra called `readme.md`
  beside a package's README, two `DISK.IMG` from two folders); nothing is
  renamed. The next revision of a package imported before ROM layers had
  names calls them `<rom id>.bin`, or `<rom id>-rom.bin` when that is taken.
  A set or package stored without the names, or under other names, is
  another one to a re-import with a statement, and the refusal says that
  only the names differ. The names are manifest bytes, so every pin moved
  once under the same tags: the three firmware packages
  (`unemu-at-bios-1.0.2-unemu.3:r1` `e3c226ae…`,
  `vgabios-0.9d-unemu.1-std:r1` `179a11db…`, `xub-r638-xtp-ata1f0:r1`
  `7d42e80e…`), the three FreeDOS sets (`1.4-r1` `545f287b…`, `1.4-1.2m-r1`
  `53e81e4e…`, `1.4-720k-r1` `29c2d9d2…`; superseded by the title-id
  change of 2026-11-18, see "Recipe builds"), the two templates, which carry
  the packages' named ROM and source layers and their digests (`open-at`
  `750981c2…`, `open-at-games` `01028f9c…`), and the pinned uncompressed
  synthetic set of the import tests (`889392e9…`). The six published
  artifacts keep their old digests until unemu-media pushes them again.
  Releases 0.0.1 and 0.0.2 read sets and packages with a name on every
  layer: measured with scratch builds of both tags reading titled copies of
  the artifacts each builds itself (manifest decode, store resolution and
  publication check, library summaries, a machine from the named packages,
  a firmware edit, a set re-import and a registry pull), so the ASSUMED
  point of recipes.md §3.10 holds.
- **2026-11-17 — Text fields in the desktop window cut, copy and paste
  with the host clipboard:** the window keeps egui's built-in clipboard
  shortcuts off, and eframe was built without its `clipboard` feature, so
  no text field took Command+V (Ctrl+V elsewhere), a registry password
  could not be pasted into Log in, and egui's clipboard was a string inside
  the program. eframe now has `clipboard` (arboard; smithay-clipboard on
  Linux, new in `Cargo.lock` with its Wayland toolkit crates; on Windows
  arboard's BSL-1.0 backend, now allowed in `deny.toml`). A focused text
  field gets the platform's cut, copy and paste chords (Command on macOS,
  Ctrl elsewhere, the dedicated keys, and on Windows Shift+Delete,
  Ctrl+Insert and Shift+Insert, as egui maps them). Paste reads the host
  clipboard at once through one arboard clipboard the shell opens at the
  first paste and keeps, and puts the text where the chord was, so Enter
  right after a paste submits the pasted password and Tab moves on only
  after the text is in. Copy and cut leave through egui's copy output, which
  egui-winit writes to the host clipboard. The password field takes paste
  only: copy and cut do nothing there (egui's own cut would delete the
  selection without copying it). With no text field focused the chords
  stay ordinary keys and no clipboard is opened or read; while the machine
  has the keyboard its keys still go only to the guest, so Ctrl+C remains
  the guest's in both shortcut styles. The native macOS menu has no Edit
  menu, so no key equivalent takes Command+V before the window.
  [`crates/ui-kit/src/text_clipboard.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/ui-kit/src/text_clipboard.rs) holds the mapping and its tests
  with a fake clipboard, plus a check that eframe is built with
  `clipboard` and that arboard opens the host clipboard (skipped on Linux
  without an X11 or Wayland display, as on the headless CI runner); the
  shell tests paste into the registry Log in fields and log in to a
  registry on this computer, and check that Ctrl+C still reaches the guest
  without a clipboard read.
- **2026-11-17 — Media › Catalogue: every disk of a drive's catalogue one
  click away:** each floppy drive in the Media menu, and its drive chip's
  menu, has Catalogue ▸ after Create from files… and before Next disk, as
  revision 3 of the approved media-picker mock-up draws it. The submenu
  lists the drive's catalogue in order under a line with the drive and the
  count, the disk in the drive marked ● and "in A:"; it lists the same disks
  as the picker's This machine, and choosing one does what Insert there does:
  running or halted it goes in now (halted, the status bar adds that
  Ctrl+Alt+Del boots from it); on a stopped machine file it waits for the
  next Start and says "at Start", and choosing the disk the drive holds at
  power-on takes the waiting one out again; a waiting disk not in the
  catalogue yet is named under the list. A stopped library machine and an
  image know their own disks only at Start: their submenu lists the disks
  waiting for it (a library set's disk, every disk of the set when all join,
  the disks Create from files… made for the machine, or what an Insert put
  into an image), the chosen one "at Start", with a line saying the image's
  own disks join then; choosing another makes it the one that goes in (a
  set's pending insert names that disk; created disks keep the chosen one in
  their record, `first`, and it leads the drive at Start). With nothing
  waiting the item is disabled as "Catalogue · known at Start". Past 12
  disks the submenu lists 12, two before the disk in the drive (or the
  waiting one) and nine after, then All N disks…, which opens Insert… on This
  machine with the disk in the drive chosen and scrolled into view (on a
  waiting list, Insert… with its set chosen); the menu never scrolls. With
  one disk or none the item is disabled and says why ("Catalogue · one
  disk", "Catalogue · empty"). The keyboard needs no new key: ↓ to
  Catalogue, → or Enter opens it on the disk in the drive, ↑↓ choose, Enter
  puts the disk in, ← or Escape go back; a drive chip's menu opens with Tab
  and Enter. The submenu opens on the side with room, and in a drive chip's
  menu it grows upward from the status bar, measured again when its lines
  change. One short name now names a disk on the drive chip, the Media
  menu's drive line, This machine, the submenu, a waiting disk's details
  line and the picker's drive button: a disk of a set is "TITLE N of M"
  (the title without its edition), with the disk's own name after a dash
  when it says more than its number ("FreeDOS 1.4 · 1 of 8 — Packages"; a
  title that ends in a number takes a middle dot; "Disk #2", "Diskette 2"
  or "2/7" add nothing), a one-disk set its title, any other disk its own
  name; the picker's disk buttons use the same rule. Create from files…
  names a split's disks the same way ("AZURAMON 3 of 10", a parenthesis in
  the title kept), and a library machine's set disks keep their short names
  after Start, as the disks the Start put in lead its drive's catalogue.
- **2026-11-17 — Repository paths carry the platform; sets and packages are
  tagged by release and revision:** media sets live under
  `unemu/media/<platform>/<work>` and firmware packages under
  `unemu/firmware/<platform>/<catalogue id>` when official, and under the same
  paths in `private/` for everything imported here; machine templates keep
  `unemu/<name>` (recipes.md §8.3, [decision-35 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-35%20-%20Recipes%20are%20OCI%20artifacts%20in%20a%20recipes%20namespace%20beside%20their%20outputs.md)). A set or package tag is
  `[<release>-]rN` (`1.4-r1`, `usa-rev1-r1`, `r1`), which never moves, locally
  or by push, with or without `--move`, and a deleted one names no other
  image; the release alone (`1.4`, also a release shaped like a version,
  `1.0.0`) is an alias for the newest revision of its release: the store
  creates it only at that newest revision and moves it forward in the same
  compare-and-swap that writes a newer revision, never backward and never to
  another release; sets and packages have no `latest` and take no new tag
  outside the scheme; the digit-first set tag rule is retired ([decision-38 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-38%20-%20Tags%20name%20a%20release%20and%20a%20revision%20and%20only%20revision%20tags%20are%20immutable.md)).
  A push creates or moves an alias only at the newest revision of its release
  that the registry already has, checked against the registry's tag list and
  the digest of each revision. A pull by an alias writes the revision it
  names, the local alias follows, and the output says so; a registry alias
  published before revisions is pulled as it is. Local imports (`image
  firmware import`, `image media import`, the window's import and Create from
  files) name their result under `private/` (`private/firmware/ibm-pc/<id>:r1`,
  `private/media/ibm-pc/<title>:1.0-r1`) and never take the official name a
  pull writes; `image firmware edit` goes to the next revision (`r2`). Only
  the template and media builders write `unemu/firmware/ibm-pc/<id>:r1` and
  `unemu/media/ibm-pc/freedos:1.4-r1` with its alias, the names the replaced
  published artifacts will take; no manifest byte changes, so no pinned digest
  moved. Remotes lists a set's or package's tags by release, each release's
  alias first, then its revisions highest first, and says Newer available only
  between revisions of one release; a local release alias at another image
  than the registry's reads Newer available when the registry lists a higher
  revision than the one the alias names, "Local is newer: …-r2" when the
  library's is higher, and otherwise Local tag names another image, as does a
  0.0.2 alias that names no revision. A store written by 0.0.2
  (`freedos:1.4`, `firmware/<id>:1`, paths without the platform) still lists,
  resolves and runs; nothing rewrites or moves those tags.
- **2026-11-17 — The Zstandard encoder is frozen as profile 1:** the
  encoder every `+zstd` payload is written with (zstd 0.13.5, zstd-safe
  7.4.1, zstd-sys 2.1.3 building the bundled libzstd 1.5.7, level 3, one
  thread, one frame with the size pledged, checksum and content size on, no
  dictionary, libzstd's 2 MiB level-3 window), together with the rules
  that choose which payloads it encodes (the 4 KiB threshold and each
  payload type's size limit), is named encoder profile 1;
  the `+zstd` suffix of the version-1 media types is its marker, so no
  manifest byte changes, and a later encoder is a new profile beside it
  (machine-images §2, the `image_store::codec` documentation).
  [`crates/image-store/tests/zstd_profile.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/tests/zstd_profile.rs) pins seven synthetic test
  vectors, one per compressed payload kind (floppy, tar source archive,
  ROM, optical, sector pack, extra, a 3 MiB disk base), by frame length and
  SHA-256, plus payloads at the edges of the threshold and the floppy limit,
  checks their frame headers, shows that changing any one encoder
  setting moves a pinned digest, and fails if `Cargo.lock` resolves another
  version, source or backend of the zstd crates or another crate named
  after zstd. The three FreeDOS 1.4
  installation sets and the three firmware packages still rebuild to their
  pinned digests.
- **2026-11-17 — Remotes: the details pane of a remote tag and Pull from
  the list; Get templates opens Remotes:** selecting a tag in Remotes shows,
  as the approved remotes mock-up draws it, the manifest's annotations,
  digest and type, the edition and version its metadata record adds, a media
  set's disks, the local tag a pull writes and its source, its local state,
  what a pull downloads (blobs the store holds left out) and the
  repository's tags; buttons Show in library, Pull…, Pull and create
  machine… (machine images), Pull and insert… (media sets, into a stopped
  machine with a drive that takes the set) and Contents… (every descriptor of
  the manifest, read without fetching). Ctrl (⌘) and Shift select several
  tags. Pull… (pane, Image › Pull…, toolbar Pull…, Enter on a row) pulls by
  the listed digest, writes the local tag last, never moves a local tag (a
  clash is pulled under another tag the dialog proposes, and a typed tag
  that names another image is refused), and pulls several one after another
  under one progress bar, skipping with the reason tags in the library, tags
  whose local name holds another image, tags this release cannot run and
  tags of a registry that no longer answers; Cancel stops at the next read
  and the waiting pulls before they start. After a pull the row turns In
  library from the store, without a new listing. A repository row now stands
  for the newest version this release runs (`latest`'s version when it is one
  of the 16 newest and runs here; at most the 16 newest are read), names the
  newer versions that cannot run here and why, and is dimmed only when no
  version runs here. Only IBM PC compatible artifacts (`ibm-pc`, or the
  retired `ibm-pc-at`) of the PC provider's three types run here: another
  family's image ("A c64 machine image: this release runs IBM PC
  compatibles") or another tool's artifact is muted with the reason and
  cannot be pulled. Machine template is said only of a repository one name
  under `unemu/`. A typed local tag that already names the image makes Pull
  say Done; Pull and insert… names what waiting disk it replaces in a drive
  and offers no drive to a CD set. File › Get
  templates… and the empty library's button select Remotes › the default
  registry › `unemu/` › Machine images; the Get templates dialog is retired,
  its rules ported (the `latest` rule, `unemu/test-…` never listed, Add
  registry… for a registry preferences do not list). Image › Pull by
  reference… is the former Image › Pull…. While the library shows Remotes
  the toolbar shows Pull…, its icon drawn, in the room of the speed control,
  which applies to a running machine only, so the toolbar keeps to one line
  at the default window width.
  Not yet: the registry node's own details pane, a search over remotes, and
  the library's disabled rows of local images that need a newer release.
- **2026-11-17 — The PC platform id is `ibm-pc`; `ibm-pc-at` stays a
  read-only alias:** the id names the IBM PC compatible family, from the PC
  and XT through the AT to the 386 and later machines, and displays call it
  "IBM PC compatible" (the installation-set details and import form, the
  generated firmware README). Every writer writes `ibm-pc`: New machine…,
  machine import, Save as new image…, a machine's Save, installation-set
  and firmware import, firmware edits, compression, catalogue edits and the
  template builds, also when they derive from an artifact that names
  `ibm-pc-at`; compression and catalogue edits keep the source's config, so
  the machine digest stays. Every reader treats both ids as one platform
  through one function of the shared store crate (`image_store::platform`,
  where the alias table lives so that comparisons without a provider at
  hand agree): manifest annotation, config and metadata `platform` fields,
  firmware packages offered to New machine and machine files, provider
  routing for pull and push, and the media-set drive checks. An `ibm-pc-at`
  image, set or package is read, listed, pulled unchanged, run and combined
  with `ibm-pc` firmware and sets; a store that holds only `ibm-pc-at`
  artifacts is read without a rewrite; any other id is still refused. The
  new id has a row of its own: release 0.0.3 first reads it, so every
  artifact written now declares `unemu.min-app-version` 0.0.3 and this
  build reads 0.0.3; a stated `--min-app-version` below 0.0.3 is refused.
  `image media show` prints the current id. A test scans every crate's
  source for the comparison forms the code base uses (`==`, `!=`,
  `matches!`, `.eq`/`.ne`, `starts_with`, `contains`, map lookups and match
  arms on a platform operand or literal, continued lines joined); it catches
  those forms, not every conceivable one. The pinned digests of the
  templates, firmware packages and FreeDOS sets moved once under the same
  tags: the six published `ibm-pc-at` artifacts are replaced by these
  rebuilds before the first public release.
- **2026-11-17 — Remotes in the library window (browse only), and where
  each local tag came from:** the sidebar lists Remotes under the Library
  group, as the approved remotes mock-up draws it: each registry of Tools ›
  Preferences › Registries, the default one first and `registry.unemu.dev`
  even when not configured, with its kind (official, public / private /
  public) or its state (a spinner while listing, unreachable, error). An
  open registry shows its namespaces (those its entry names, or the first
  path components a whole-registry entry's catalog returns), a private one
  with a lock while no account is stored or the registry asks for a login;
  an open namespace shows one group per artifact type that has repositories
  (Machine images, Installation media, Firmware), with counts. Nothing
  contacts a registry until its node is expanded or selected; every request
  runs on the library worker. The list is the library's grid with the
  mock-up's columns (Name, Source, Title, Latest, Type, Licence, Size, Local
  state): a repository row stands for the tag the model chose, a repository
  of several tags expands into them newest first (its other tags are read
  then), and a tag this release cannot run is muted with "Needs unemupc X or
  newer". Strips over the rows say a listing runs (Stop), stopped (Refresh)
  or no longer answers (the earlier rows stay, Try again); messages instead
  of the rows cover a listing before its first row, a listing stopped before
  it, nothing published, unreachable, server errors (404 included), login
  required (Log in… only for a Basic challenge; Preferences › Registries
  opens on the registry, and it lists again when they close) and local
  failures, including a library worker that went away. Next 50 › reads the
  catalog's next page. F5, View › Refresh remote and the toolbar's Refresh
  list the shown registry again; the toolbar's Registries… opens Preferences.
  Every start of a machine (Start, Gaming mode, Start…, a power cycle, a
  machine file) stops every listing first, also before the readiness check,
  so a slow registry never holds a machine back; an expanded repository
  whose reads stop or whose listing is replaced collapses. The arrow keys
  move between rows and Enter expands a repository, as in the library's
  lists. The store now records where every tag written from now on came
  from: a pull its registry (`unemu.pull.registry`), any other write
  `unemu.made-here`; the Images, Installation media and Firmware lists show
  that as a Source column (the host, `local`, or a dash for a tag written
  before either record, never guessed). Not yet: the details pane of a
  remote tag and Pull from the list (File › Get templates… keeps its dialog,
  which pulls the templates, and Image › Pull… pulls by reference),
  multiple selection, a search over remotes, and dimming a repository only
  when none of its versions runs on this release (each tag row is dimmed on
  its own today).
- **2026-11-17 — Browsing remotes, without drawing yet:** the library worker
  can list a registry without pulling, for the Remotes group of the approved
  mock-up, also while a machine runs. The roots are the configured
  registries, the default one first, with `registry.unemu.dev` listed even
  when not configured. A listing reads the catalog 50 repositories a page
  (the next page on demand; an entry that names namespaces reads only
  theirs), each repository's tags as the registry pages them, and the record
  of the tag a row stands for: the newest SemVer release, else the newest
  set tag (`1.4` before `1.4-720k`) or firmware revision, whatever order the
  registry lists them in. Expanding a repository reads its other tags. A
  record comes from the manifest and, for media sets and firmware packages,
  the small config record: artifact type, title, version, a set's formats,
  size and the raw `unemu.min-app-version`. Selecting a tag also reads its
  small metadata record, for the edition and version; no other layer, cover,
  disk or ROM, is fetched, and nothing is written to disk. Refresh replaces
  the session's listing, and Stop takes effect before the next request. The
  model has the mock-up's states (listing, stopped, unreachable, unreachable
  with the earlier rows kept and marked stale, server error with status and
  reason, login required with the registry's challenge, Log in… only for
  Basic); a failed next page keeps the pages before it. A whole-registry
  entry is grouped by first path component. Each tag is compared with the
  library within its host-qualified local name: in library, not pulled (or
  the same image under another tag), newer available, or local tag names
  another image, again whenever the library changes. "Newer" follows the
  artifact type: SemVer for machine images, `<version>[-variant][-rN]` within
  one variant for media sets (`1.4` is newer than `1.3`, `1.4-r2` than
  `1.4`, `1.4-720k` is a sibling), the revision number for firmware; aliases
  never count. The window does not draw Remotes yet.
- **2026-11-17 — Images declare the oldest unemupc that runs them (core
  and command line):** a manifest may carry `unemu.min-app-version`, the
  oldest release that reads and runs it. Writers compute it from per-feature
  tables (shared rows in `image-store`, PC rows in `pc-image`: config keys,
  vocabulary values, payload types, firmware catalogue entries) and leave it
  out at 0.0.1, so the published FreeDOS sets, firmware packages and
  templates keep their digests; every row is 0.0.1 today. `--min-app-version`
  on `image import`, `image media import` and `image firmware import` and
  `edit` raises it, never lowers it, and a machine takes the value of the
  firmware packages it binds. Pull refuses an image that needs a newer release
  before fetching any blob, with "REFERENCE needs unemupc X or newer (this is
  Y)"; `image run`, `image machine new` and Start refuse it with the same
  sentence before reading its config; the `image`, `machine`, `media` and
  `firmware` listings show the requirement. A push into `unemu/` computes the value
  again and refuses a missing or lower one. The window's disabled rows and
  Get templates' choice of a runnable version come next.
- **2026-11-17 — Create from files… in the media picker:** the picker's
  footer has **Create from files…** beside Browse for an image file…, and
  Media › Create from files… under each drive's Insert… and the same item in
  a drive chip's menu open the picker straight in its Create view, as revision
  2 of the media-picker mock-up draws it. The view keeps the Into row; Add
  files…, Add folder… (the asynchronous system dialogs) and files dropped on
  it fill a list of where each item lands, its files, folders and size, with ×
  to remove; the same item twice is one. The library worker reads only names
  and sizes until Create; the window then lays the list out again for every
  edit without reading the host, through a new `media::fat12::HostItem` and
  `Survey` that share the command line's builder (a survey's images equal
  `fat12::build`'s). A single folder becomes the top folder, cut to 8.3; the
  label follows the top folder until edited; the formats are the drive's,
  its own first. The space meter counts clusters, folders and root entries
  and says live whether the files fit, need N bytes beyond one disk (with a
  link to a larger format the drive takes that holds them) or hold a file too
  large for any disk, which disables Create; DOS names and skipped files are
  collapsible lists. Too big for one disk shows Split across N disks with its
  plan (label, files, bytes used, first and last file), the Next disk note and
  the XCOPY command, and Copy into C: greyed, "later". Create and insert
  builds on the library worker and stores each image once, named by its
  SHA-256, as a read-only base among the machine's own media (guest writes go
  to its journals or are refused): a running or halted machine takes disk 1
  at once with every disk in its catalogue in order; a stopped library machine
  keeps the disks and their pending insert in its workspace
  (`media/created/`), never in the store's machine list, which released
  versions read with unknown fields refused; Start puts them in and the record
  goes once the machine assembled; a record that cannot be read, or a disk
  whose image is gone, stays out with a note and never stops the Start, and
  the next Take out again or Insert replaces such a record. Take out again,
  another Insert into the drive and Reset to image… drop them; Duplicate,
  Gaming mode and Save as new image… with fresh journals keep them (the PC
  provider's publication now carries `media/created/` to the new journals). A
  running machine takes the images as the worker hashed them, without
  hashing them again on the window's thread. "Also keep in the library" (on
  for a split, off for one disk until changed) also publishes the disks as a
  private installation media set. Splits, here and in `upcli media create
  floppy --split`, now fill disks in folder order, as the mock-up draws it: a
  folder's own files before its subfolders', so a game's loose SETUP.EXE and
  README go on disk 1; directory entries stay sorted. The mock-up's DOS-name
  sentence now states the builder's rule (cut to 8.3, a `~N` tail only on a
  clash). machines/README.md § Create from files… in the window describes it,
  with the XCOPY recipe and how to make the same disk on the command line.
- **2026-11-17 — Pulled images keep their registry host:** an image pulled
  from any registry other than `registry.unemu.dev` gets the full reference
  as its local tag, Docker-style (`registry.lan.example/private/games/dune2:1.0.0`,
  `nas.example:5000/private/media/x:1.1`), so the same repository path on two
  registries no longer collides. Pulls from `registry.unemu.dev` keep the
  short tags (`open-at:1.2.0`, `unemu/media/freedos:1.4`), and images made
  here get no host. The store's tag grammar accepts an optional lowercase
  `host[:port]/` first component; each pull records its registry on the tag
  (`unemu.pull.registry` in `index.json`), while older tags stay as they are,
  unrenamed, with an unknown source. Push of a host-qualified tag goes to its
  own host unless a reference is given, and namespace policy reads the path
  after the host. Save as new image and Publish revision from a pulled image
  suggest a name without the host. Browsing remotes and the library's Source
  column come later.
- **2026-11-16 — Floppies built from host files:** `upcli media create
  floppy --add PATH` builds a new FAT12 data floppy (360K, 720K, 1.2M or
  1.44M) from host files and whole folders, for DOS programs that come as
  loose files; `--top NAME` puts them under one root folder and `--split`
  spreads a larger tree over `<stem>-1ofN.<ext>`, `<stem>-2ofN.<ext>`…
  labelled `LABEL 1`, `LABEL 2`…, each disk repeating the folder path so
  XCOPY /S /E from every disk rebuilds the tree on C:. Names become plain DOS
  8.3 names (no long-name entries) with `~1` tails on clashes and Latin
  diacritics folded to their base letters; every change and every skipped
  host file inside the folders (Finder, archive and Explorer metadata,
  symbolic links, devices, the images being written) is printed. A named
  source that is a link, metadata or a device is refused, as is the same
  host entry named twice, and so is a folder whose DOS path would pass the 64
  characters DOS keeps for a current directory. Identical trees give
  identical bytes whatever the host times or directory order. The fit is
  checked first, counting whole clusters and folders: a misfit names the
  shortfall in bytes and writes nothing, and a file larger than a disk is
  named and never split. The formatter moved into the media crate, where the
  window can reuse it; blank `--format` images are byte-identical to before.
  Two slow goldens boot FreeDOS with such disks in B:: one checks DIR and
  TYPE, the other enters a two-disk 720K set with CD and rebuilds it with
  XCOPY on A:.
- **2026-11-16 — `image template build` builds the templates only:** the
  open firmware packages and the FreeDOS 1.4 Floppy Edition installation sets
  are built and published by the unemu-media repository
  (github.com/unemu-org/unemu-media); get them with `upcli image pull
  unemu/firmware/<catalogue id>:1` and `upcli image pull
  unemu/media/freedos:1.4` (`:1.4-1.2m`, `:1.4-720k`). The command's output,
  dry-run report and guard checks cover `open-at` and `open-at-games` alone;
  it still imports the three packages the templates bind from the firmware
  fixtures, as local copies under the published tags (they stay even when a
  template is refused), and no longer reads the installation-set fixtures. A
  package tag of the store that names another image stops the build and its
  dry run alike. The template digests are unchanged. The local
  builders stay for tests: `pc_image::template::build_firmware` (packages
  under `unemu/firmware/<catalogue id>:1`, no `latest`) and `build_media` (sets
  under `unemu/media/freedos:1.4`, `:1.4-1.2m`, `:1.4-720k`, no `latest`,
  titled "FreeDOS" with the release and edition in their own fields, as
  unemu-media publishes them; formerly `unemu/media/freedos-floppy-*:1.4.0`).
  Evidence: the pin tests give the six published digests (three packages,
  three sets) from this repository's fixtures, and unemu-media's exact import
  argument lists, run with this `upcli` in a scratch store, give the same six
  digests; the narrowed command's tests fail when the old package and set
  output is restored, and the dry-run package-tag test fails without its
  check.
- **2026-11-16 — The Insert media picker looks like its mock-up:** the
  picker takes the approved media-picker mock-up's size, 1040 by about
  640, inside the window's content area with room around it; below 720
  wide or a 240-high body its content scrolls inside the window rather
  than squeezing. The body takes the height the rows above it and the
  footer leave, measured in the same pass, never from the frame before,
  so the picker never grows and the footer stays inside it when the Into
  row wraps. The Into row shows the drives as toggle buttons ("A: 1.2M ·
  empty", what a drive holds cut to one line) and when the disk goes in;
  the From row shows the sources as tabs and greys a tab per configured
  registry, the default first, until after the MVP; the search field spans
  the picker. The facets, results and details panes sit on the field
  colour inside the library panes' inset outline. Facets are flat sections
  under small uppercase headings, one line per value with its check box
  and a right-aligned muted count. A result is a full-width row: a cover
  whose title breaks between words and stays clipped inside it, the title
  with its edition in muted text, the subtitle, and the disks, extras and
  licence mark at the right; selection fills the whole row, and the line
  of a selected set that does not fit stays readable on it. The details
  pane has the cover, title and reference with the licence mark, "Disk to
  insert" and "When a program writes to the disk" group boxes with compact
  one-line radios ("1. Boot disk and installer · 1.2M": the label without
  a file name such as "(x86BOOT.img)", which goes in the tooltip, while
  "(1.2M)" or "(v2.0)" stay), the catalogue check box and the extras line;
  it scrolls on its own for a long set. The footer right-aligns the
  summary on one line, which names the disk as its radio button does,
  shows the keys with key caps, and frames Browse for an image file…,
  Insert (the default) and Cancel at the right in that order. Classic and
  modern skins, light and dark. Not yet shown: the set's one-line
  description, which the library's set summaries do not carry. Behaviour
  and keys are unchanged; every control answers a click in a window test.
  Evidence: window tests for the size at a big and two small windows, the
  panes' fixed height across frames, the footer under wrapped top rows,
  single-line facet values, covers clipped and broken between words, the
  radio column, the framed right-aligned footer buttons, the label rules,
  the registry tabs and a click on every control; the layout tests fail
  on the previous picker.
- **2026-11-16 — Library details panes start both columns at the top:**
  in every collection whose details pane has sections side by side
  (machines, images, installation media, firmware), the pane's head (the
  kind tile, the title and the facts under it) now opens the first column,
  and the other columns start beside it at the top of the pane instead of
  under the head; the actions and the note under them stay full width below
  the columns, and a narrow pane still stacks everything in one column.
  Machine files keep their single column. The approved machines mock-up
  shows the same layout now, at the maintainer's request. The details pane's
  first height grows from 420 to 460 px, so on a window 1280 px wide or
  wider the pane of a machine or an image without hints or warnings shows
  whole, the note under its actions included; longer panes, such as a
  machine's with a newer-firmware hint, scroll with their scroll bar shown,
  in the smallest window too. The labels field of a machine or an image
  keeps its focus while the window is resized across the width where the
  columns stack. Evidence: headless layout tests for level column tops at
  the pane's top on wide panes, the third column under the first two on a
  middle-width pane and stacking on narrow ones, the whole note at the first
  height on a 1280 px window, the note scrolled into view for a machine with
  a hint and in the smallest window, and the labels field's focus across
  the stacking width.
- **2026-11-16 — Version 0.0.2, a preview release:** published tags of
  installation media sets never move and the push preflight sees every
  manifest type; `upcli --version` names the full commit of a release build;
  the setup-upcli GitHub Action installs a pinned upcli release; the binaries
  describe the project as an emulator of period IBM-compatible PCs.
- **2026-11-16 — Published tags are safe before the first media push:** the
  push preflight and its final recheck ask for the tag with every manifest
  and index type in `Accept` (OCI image manifest and index, Docker schema 2
  manifest and manifest list), so a tag that another tool pointed at an image
  index is no longer taken for a new tag and replaced without `--move`; pull
  still asks for the OCI image manifest only. Tags of installation media sets
  that start with a digit (`1.4`, `1.4-720k`, `1.4-registered-720k-r2`) are
  now immutable like version tags, in the local store and on push, even with
  `--move`, whichever side the set is on; the refusal suggests the next `-rN`
  revision. Aliases (tags that do not start with a digit, `latest` included)
  still move with confirmation, and machine images and firmware packages keep
  the version-tag rule alone. A deleted set tag is not remembered locally as a
  deleted version tag is. Evidence: mock-registry tests for an image index, a
  Docker manifest and a Docker manifest list hidden from the narrow `Accept`,
  and for set tags with and without `--move`, plus a store test; each fails
  when its change is reverted.
- **2026-11-16 — setup-upcli action:** `.github/actions/setup-upcli` is a
  composite action that installs a released `upcli` in other repositories'
  workflows (the open media pipeline and the firmware forks), so they import
  and push with this project's CLI and push guard. Inputs: `version` (a
  release tag), `sha256` (of the runner's archive) and an optional `commit`
  that must match the `commit:` line of `upcli --version`. It supports Linux
  X64 and macOS ARM64 runners, downloads with curl over HTTPS only, checks
  the SHA-256 before unpacking, installs only `upcli` with `LICENSE` and
  `THIRD-PARTY-LICENSES.md` under `$RUNNER_TEMP`, adds it to `PATH` and
  outputs its `path`. It holds no secrets and uses no other actions; the
  inputs reach its bash script only through the environment and are
  validated first. Its README shows a media-set import and push with the
  registry credentials on the push step only.
  `.github/workflows/setup-upcli.yml` tests it on `ubuntu-latest` and
  `macos-14` with v0.0.1, including a wrong SHA-256 that must fail.
  Evidence: actionlint (with shellcheck), shellcheck of the script and
  zizmor report nothing. On an Apple-silicon Mac the script installed v0.0.1
  under bash 3.2 (with `sha256sum` and with `shasum`) and ran `upcli
  --version`; it refused a wrong SHA-256 with nothing unpacked or added to
  `PATH`, and refused malformed inputs, an unsupported platform and a
  `commit` for a release without a commit line. The Linux archive hashed and
  unpacked there too. The workflow's first GitHub run follows the push.
- **2026-11-16 — Version 0.0.1, a preview release:** the workspace version
  moves to 0.0.1 for the first public preview on GitHub, so the release
  workflow publishes upcli and unemupc binaries (Linux x86-64, macOS on Apple
  silicon) that other projects, such as the open media pipeline, can download
  instead of building. The official templates, media sets and firmware
  packages are not on registry.unemu.dev yet; they come with a later release.
- **2026-11-16 — Release binaries:** `.github/workflows/release.yml` builds
  `upcli` and `unemupc` when a GitHub release is published (or by hand for a
  tag) and attaches `unemupc-<version>-<target>.tar.gz` for
  x86_64-unknown-linux-gnu (Ubuntu 22.04) and aarch64-apple-darwin (macOS 15),
  plus `SHA256SUMS`. It builds outside Nix with rustup at the devShell's Rust,
  `--locked --release`, path prefixes remapped and `UNEMUPC_SOURCE_REV` set to
  the tag. A check step fails on the runner's home, workspace, cargo home or
  toolchain in a binary (`strings`, whole file) and, on macOS, on a library
  outside the system (`otool -L`). Each archive carries the licence, the
  README, `THIRD-PARTY-LICENSES.md` for its target and the in-tree notices.
  The macOS binaries are unsigned; the workflow appends how to clear the
  quarantine flag to the release notes. Evidence: actionlint (with
  shellcheck) and zizmor report nothing. A dry run of the macOS steps outside
  Nix on an Apple-silicon Mac, on the tree with the release-readiness changes,
  built both binaries with no home, workspace, user name or `/nix/store` path
  in them, `otool -L` listing only `/usr/lib` and `/System/Library`, and an
  archive whose `upcli --about` names `.../tree/v0.1.0`. The Linux leg and
  the upload job have not run yet; the first GitHub release is their test.
- **2026-11-16 — CI moves to GitHub Actions:** the repository moves to
  `github.com/unemu-org/unemupc`, and `.github/workflows/ci.yml` becomes the
  only CI; the Forgejo workflow is deleted. It runs the same
  `scripts/ci-gate.sh` gates (devShell, formatting, publication guard,
  Clippy, cargo deny, tests, fixtures, slow goldens; the user-ROM lane stays
  local) on pushes to `main`, pull requests and manual dispatches, on
  GitHub-hosted `ubuntu-latest`. The job frees disk space first, installs
  Nix with the multi-user daemon and the build sandbox on, and keeps the
  Cargo and FreeDOS 1.4 caches. Every action is pinned to a commit SHA; the
  job has read-only permissions and no secrets, and keeps the job token out
  of `.git/config` and `nix.conf`. Instead of a posted commit status, any
  failed gate fails the job, a final Gate summary step names the failed
  steps, and `ci-logs/` is uploaded as a run artifact. Read runs with
  `gh run list --repo unemu-org/unemupc`. Evidence: actionlint (with
  shellcheck) and zizmor report nothing, and the `fmt` and `guard` gates
  pass locally; the first GitHub run follows the move and is not yet
  observed.
- **2026-11-16 — Release readiness of the tree:** the binaries no longer carry
  or use the path of the tree they were built in. The desktop's library lists
  machine files from `UNEMUPC_MACHINES_DIR`; else, when cargo started it, the
  tree's `machines/` (from `CARGO_MANIFEST_DIR` at run time, never compiled
  in); else `unemupc/machines` in the platform data directory. `--help` prints
  that directory, and a fresh install selects no machine instead of a missing
  default. Its ROM folders are its preferences, `--rom-dir`,
  `UNEMUPC_USER_ROMS_DIR` and, under cargo, the tree's `vendor/user-roms`. A
  Settings… child of a machine file outside any `machines` directory goes to
  the `local/` folder of the library's machines directory; with no data
  directory Settings refuses with a message. The user-ROM test lane exists only
  with `test-support`. Tests check that none of these defaults points into the
  build tree. Panic locations of registry and git dependencies still name
  cargo's cache in a release build; remapping them belongs to the release
  build flags. `upcli --version` and `--about` now print the licence
  (AGPL-3.0-or-later), the source of the build (the tag or commit a release
  names in `UNEMUPC_SOURCE_REV`), the Nuked-OPL3 notice and where the full
  licence texts are;
  `unemupc --version`, `--about`, `--help` and the About dialog add the
  Nuked-OPL3 notice next to the fonts. `scripts/third-party-licenses.sh`
  writes `THIRD-PARTY-LICENSES.md` (generated, gitignored) from `cargo
  metadata`, `cargo tree` and the crates' own licence files for the exact
  `Cargo.lock`, with the standard MIT, Apache-2.0 and Zlib texts in
  `LICENSES/` and the authors as copyright holders for crates that publish no
  licence file. The workspace points at its public repository and no crate is
  publishable to crates.io (`publish = false`). The publication rule is
  relaxed: no proprietary firmware, software or manuals, but real firmware and
  commercial titles may be named; private hosts, paths and personal setup stay
  out. The publication guard also catches more private path forms, e-mail
  addresses outside an allowlist, private and carrier-grade NAT IPv4
  addresses, `Cargo.lock` sources other than crates.io and github.com, and
  private hostnames given only through `PUBLICATION_GUARD_PRIVATE_HOSTS`
  (local only; hits show file and line). Installed users missing a
  user-supplied BIOS are pointed to Preferences or Import ROM folder… instead
  of the source tree's ROM guide. Mock-ups use `nas.example`, scripts and
  configuration files carry SPDX headers, and the README covers downloads,
  building without Nix and the third-party components.
- **2026-11-16 — File dialogs no longer abort the app on macOS:** with a
  machine running, opening a file dialog ended the process. Each dialog
  was synchronous: it ran its own modal event loop inside the frame that
  opened it, that loop dispatched winit's queued events while winit was
  still handling the frame, and winit aborted. A running machine wakes the
  event loop all the time, which is likely why it happened nearly every
  time with one running. Every file dialog of the desktop app (the eight
  Browse…, Save as… and Import pickers) now goes through one helper that
  starts rfd's asynchronous dialog and returns at once: a sheet on the
  window on macOS, the XDG desktop portal on Linux as before. The shell
  polls it each frame without blocking, and the chosen path reaches the
  place that asked on a later frame. Insert floppy's Browse… and Save as…
  answer only while that dialog is open for the same drive; Create blank
  disk and ROM folders only while open. A media picker's Browse for an
  image file… inserts only into the same running session or stopped
  machine and only while no other dialog is open; a source archive for
  Edit metadata… goes only to the same package; Import ROM folder… starts
  only while the import is still allowed. Those last three say in the
  status bar why a chosen path was not used. Import installation media…
  and a firmware import's source archive ignore answers once their
  dialog has closed. While a file dialog is open the status bar says
  "Waiting for the file dialog…", a running machine keeps running, and a
  second file dialog is refused with a status message naming it, also
  when Edit metadata… opens its source archive picker by itself. Titles,
  filters, start folders and proposed file names are unchanged. Headless
  tests pin them per call site in a table, cover each delivery rule in a
  second table, drive the answer through the frame's logic step, and
  check the helper's request, answer, cancel and refusal steps with a
  scripted dialog. The real dialog cannot run headless and still needs a
  check on macOS with a running machine. The app shows no message boxes,
  so no other modal path was affected.
- **2026-11-16 — Disk layers follows its approved mock-up:** the dialog
  is one sunken tree in the code font, inside the classic bevel or the
  modern 1 px outline the library's panes draw. Each fixed disk goes by
  letter, IDE master or slave and size (`C: IDE master (20.4 MiB)`), with
  its name and write mode, then its chain: each member by kind (a zero base
  reads `base  blank`), a short digest (`abcd…wxyz`), its size and how many
  images share it, down to the working changes, which take the selection
  highlight. CMOS keeps its own small chain. Floppy drives follow by letter
  and type (`A: 1.2M catalogue`), each medium by name with its write mode,
  the one in the drive marked and unsaved sectors highlighted, or `empty`;
  a floppy medium is one blob, so its digest is no longer listed. Firmware
  goes by the names and open or user-supplied marks the image's details
  pane gives it: the catalogue's names, a library package's for an unlisted
  ROM, else the start of its digest. Settings name the image tag they come
  from, as images record no settings revision number, and the stored,
  exclusive and dedup sizes stay as one quiet line. No controller id,
  medium id or configuration key is shown, neither in the discard question
  nor in the Save as new image… form's list of what is saved. Under the
  tree is the mock-up's notice, and the footer is one row, read and tabbed
  in the same order: Save as new image… (the default), Discard changes,
  Close. Save as layer… and New image from here… both opened the Save as
  new image… form, the second only adding *Flatten disk chains into new
  bases*; the one button opens the form with that option, unchecked.
  Gaming mode keeps Keep as new image…, without the option. When the media
  arrive the dialog takes its new size in one frame instead of growing over
  three. Saving and discarding are unchanged. The design's other *New image
  from here…*, forking a workspace from an earlier manifest, is marked as
  not built. Details panes and Disk layers now call a ROM open only when
  the firmware catalogue does not list its bytes as proprietary, whatever
  the layer's own mark says, as the push guard already ruled.
- **2026-11-16 — The Insert disk picker keeps its size:** opened from a
  drive chip's Insert…, the media picker grew taller every frame, about
  63 px a frame, until its rows and buttons left the window and nothing
  could be chosen. The vertical separators between its facets, results and
  details columns took all the height on offer, and a modal offers the
  height it measured the frame before, so the footer's height was added
  again on every frame. The three columns now share one fixed 300 px body,
  as the approved media-picker mock-up's body has, and each scrolls inside
  it; the modal's rectangle holds from its third frame, from This library
  and from This machine. Headless tests draw the picker for thirty frames,
  directly and through the status bar's Insert…. The desktop app's other
  modals were checked for the same pattern; none grows.
- **2026-11-16 — Library list headers sit edge to edge, labels on the left:**
  the column headers of the Machines, Images, Installation media and
  Firmware lists now follow the library mock-up's list header instead of a
  row of separate centred buttons. The cells sit edge to edge on the face
  colour with no gap between them: each starts where its column's cells
  start and reaches the next column, and the last runs on over the scroll
  bar's room to the list's edge. Labels are left-aligned in the mock-up's
  6 px padding and truncated with an ellipsis; a truncated label shows in
  full on hover. In the classic skin each cell is a raised two-pixel bevel
  (highlight and second face at the top and left, dark shadow and shadow at
  the bottom and right) that sinks into the inset bevel while pressed; in
  the modern skin each cell has a flat 1 px line in the shadow colour on
  square corners, neighbours sharing one line. The Images list's Machines
  header, which cannot sort, looks the same but takes no click. Sort marks
  and clicks are unchanged. A column's narrowest width now keeps the
  header's 12 px of padding rather than 10 px, so on a narrow window a
  column may be 2 px wider. Rows are unchanged.
- **2026-11-16 — The project is renamed unemupc:** the name unemupc98 no
  longer fits a line that may end with an optional Windows XP phase, and a
  later NEC PC-98 emulator in the family would collide with it; the logo has
  shown "PC" since 2026-11-14. The desktop app's binary is now `unemupc`
  (was `unemupc98-window`) and the command line's is `upcli` (was
  `unemupc98`): `cargo run -p window --bin unemupc`, `cargo clippy -p cli
  --bin upcli`, and the usage text says `upcli`. The window title, About
  box, macOS application menu and messages say unemupc. Two effects reach
  users: on macOS an Accessibility grant made to the old binary does not
  carry over (started from a terminal, the grant belongs to the terminal),
  and on Linux the window class and Wayland app id change to `unemupc`.
  Every developer and CI environment variable drops the 98:
  `UNEMUPC_REQUIRE_FIXTURES`, `UNEMUPC_USER_ROMS_DIR`,
  `UNEMUPC_REQUIRE_USER_ROMS`, `UNEMUPC_OPEN_AT_BIOS`,
  `UNEMUPC_TRACE_SPEAKER`, `UNEMUPC_SCHEDULER_WALL_TIMES`,
  `UNEMUPC_SCHEDULER_STREAMS`, `UNEMUPC_RECORD_STREAMS`,
  `UNEMUPC_FIXTURE_PROBE` and `UNEMUPC_QUANTUM_CHILD_TEST`; the old names
  are no longer read. The shared preferences file moves from
  `unemupc98/preferences.toml` to `unemupc/preferences.toml` in the platform
  config directory. The first time the desktop or the command line asks for
  it, and the new file does not exist yet, the old file is copied there
  under the preferences lock before anything reads it; the old file stays.
  Both frontends now take the path from one function, `cli::preferences`.
  The firmware hash index moves to `<cache dir>/unemupc/firmware-index.v2`
  and is rebuilt on first use rather than copied. Pinned fixture inputs keep
  the old name where it is part of their bytes (the hard-disk fixture's
  MARKER.TXT, the SeaBIOS version string, the source-archive notices and the
  FreeDOS LICENSES.TXT), so no golden or digest moves and the guest sees no
  change. Repository URLs still name unemupc98 until the remote repositories
  are renamed. `scripts/publication-guard.sh` now fails on the old name
  outside those places, the history records and the status entries from
  this one down.
- **2026-11-16 — The CPU core comes from its public GitHub repository:** x86jit
  is now published at github.com/unemu-org/x86jit, its primary forge, with its
  history rewritten (third-party binaries filtered out, every commit re-signed),
  so every revision hash changed. The pin moves to `ae12d43e`, the public commit
  whose source tree is identical to the previous pin `80b54f8a`; Cargo.toml,
  Cargo.lock, the cargo-deny git allow list and every golden's CPU revision
  follow. No behaviour changes: every golden, the slow ignored set and the
  user-ROM goldens keep their retired counts and traces.
- **2026-11-16 — Library panes on the field and the second face, in an inset
  outline:** as in the approved library mock-ups, the library's tree and list
  panes now sit on the field colour (#ffffff light, #26282b dark) and the
  details pane on the second face colour (#e7e4dc light, #3e4045 dark),
  instead of the face colour, in the Machines, Images, Installation media
  and Firmware collections alike. Each pane has the mock-ups' inset outline:
  in the classic skin a sunken two-pixel bevel (shadow and dark shadow at
  the top and left, highlight and second face at the bottom and right), in
  the modern skin a flat 1 px line in the shadow colour around 6 px rounded
  corners. The face colour shows in a 3 px gap around the panes and in a
  6 px split between the list and the details pane, which still drags.
  Dark stripes now read as lighter bands on the dark field rather than dark
  ones on the face, and the modern light panes are white rather than beige.
  The list's search field, which shares the field colour, is sunk into the
  same bevel in the classic skin. Each pane takes the room its panel has
  and keeps its contents inside its outline, so in a short window nothing
  runs over the gaps or the status bar; a tree taller than its pane now
  scrolls, as the mock-ups' panes do. Content stays 8 px from the panel's
  edge across, and now 8 px rather than 2 px at the top and bottom of the
  tree and the details pane. Row heights are unchanged. On the new grounds,
  modern text, strong and muted text read at 4.5:1 or more, except the
  mock-ups' own muted text on the second face in the dark theme (4.27:1),
  and so do links, warnings, errors and open marks on the second face; the
  classic skin's text keeps 3:1, as its icons need. The guest screen,
  menus, toolbar, status bar and dialogs are unchanged.
- **2026-11-16 — An unavailable firmware entry says why:** in Settings ›
  Firmware, on a machine or an image, an entry the list greys as
  "unavailable" now shows its reason when hovered (not in your store,
  user-supplied ROMs are never downloaded, or the validator's reason); egui
  gives a disabled entry no hover text of its own, so the open list said
  "unavailable" and nothing more. When the current entry is superseded and
  its replacement cannot be chosen, the greyed Use link says why on hover,
  including that the store's packages are still being read (the list itself
  stays disabled until they are). Once they are read, a line right under
  "Superseded by …" names the replacement with its reason and, when its ROM
  is not in the store, Import ROM folder…, the sheet's existing import. That
  line is not repeated in the "Fit this board but cannot be chosen now" list
  further down, which may be scrolled out of view. An entry that the image or
  the machine's applied settings already bind no longer claims to be "not in
  your store": it can be chosen without a package, as Use the image's value
  always could bind it. New machine…'s greyed options in its lists say why
  on hover too; their lines under each field already did. Headless tests
  open the lists, hover greyed and available entries and read the painted
  tooltips and lines; each fails without the change.
- **2026-11-16 — Get templates lists only template repositories:** a template
  is a repository one level under `unemu/` (`unemu/open-at`,
  `unemu/open-at-games`). Nested repositories (firmware packages under
  `unemu/firmware/…`, media sets under `unemu/media/…`, staging repositories
  under `unemu/staging/…`) and the live registry test's `unemu/test-…`
  repositories are left out by name before any request. Before, only the
  firmware and live-test names were, so a machine image in a media or staging
  repository would have been offered as a template. A dialog test seeds the
  same machine image in all six kinds of repository and expects only
  `open-at` and `open-at-games`, with the others never read; it fails without
  the fix.
- **2026-11-16 — The modern skin takes the mock-ups' colours; New machine…
  never suggests a template's repository name:** as in the approved
  mock-ups, the modern skin now shares the classic skin's colours and differs
  only in its rounded corners and flat outlines. Panels, dialogs and resting
  buttons take the face colour (#34363a dark, #d6d3cb light); hovered buttons
  and buttons with an open menu the second face colour, which also fills the
  boxes of check boxes and radio buttons; text takes the ink and muted
  colours, selected rows the selection colour with white text, links the
  link colour, and dialogs, menus, separators and resting controls a 1 px
  outline in the shadow colour. Main-toolbar buttons have no frame at rest
  and show that outline only on hover; the classic skin keeps its bevel.
  Text fields are no longer lighter than the panels in the dark theme. In
  the modern skin's light and dark themes, every widget state's text on its
  fills, and plain, weak, strong, link and error text on the face, fields
  and stripes, read at 4.5:1 or more; so do warnings on fields and stripes,
  while on the face they take the mock-ups' warn colour, which reads 4.23:1
  there in the light theme. Both skins take the mock-ups' warn and error
  colours, and in both a focused text field is outlined in its own text
  colour, at 3:1 or more against the field, as the mock-ups' focus rule
  draws it (egui's white outline vanished on the light field). The guest
  screen keeps its black surround. New machine… names the image's
  repository after its title, as a lowercase slug in which letters with
  diacritics lose them ("Gra Żółw" gives `gra-zolw`), and after its firmware
  while it has none (`open-at` for open firmware). The suggestion is never an
  official template's name, which made the user's own image look like the
  template (a `-local.N` tag suggestion and the local-only note), nor a
  repository the store already has: it takes the first free `-2`, `-3`, …
  suffix (`open-at-2`). The user can still type any name; the Name and tag
  step's example now reads `my-at-286`, in the app and in the new-machine
  mock-up.
- **2026-11-16 — Exchange and read-modify-write instructions on memory-mapped
  devices:** the CPU core pin moves to x86jit `80b54f8a`. `XCHG` with a memory
  operand on a memory-mapped window (video memory included), with or without a
  segment override, now does one device read and then one device write; before,
  the core stopped with "unsupported instruction" (a game swapping bytes with
  video memory, `XCHG ES:[DI], AL`, stopped right after its title screen). The
  same holds for `XADD`, `CMPXCHG` and their `LOCK` forms, and every
  read-modify-write form on a device window (ALU operations, `INC`/`DEC`/`NOT`/
  `NEG`, shifts and rotates) now does exactly one read and one write, without
  the second read after the write. A locked operand that lies partly in RAM and
  partly in a device window still stops with the instruction bytes. The pin also
  brings the faster RAM/ROM region lookup. A CPU adapter test pins one read and
  one write for `XCHG` and for a word `ADD` on video memory; it fails at the
  previous pin. Every golden, the slow ignored set and the user-ROM goldens
  keep their retired counts and traces.
- **2026-11-16 — Firmware is editable in Settings… on machines and images;
  "newer version available" for superseded firmware:** implements the approved
  [firmware-settings mock-up](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/mockups/firmware-settings.html) (now in
  the repository; the icon inventory counts 12 approved mock-ups). The
  catalogue gives every system BIOS a CMOS layout (`CmosLayout`: open AT
  BIOS, Kiselev Xi 8088, IBM AT, Landmark ROM POST) and every superseded entry
  its replacement and a short reason as data (`Supersession`): the open AT
  BIOS 1.0.2-unemu.2 → 1.0.2-unemu.3 ("clears memory above 1 MiB on POST:
  XMS after Ctrl+Alt+Del"), vgabios 0.8a-unemu.1 → 0.9d-unemu.1 ("teletype
  BEL sounds the speaker"). The Firmware tab of Settings… offers the System
  BIOS and, for a VGA, the Video BIOS among the catalogue entries that fit the
  board, grouped by family, each unavailable one with its reason (the
  validator's, or not in the store with Import ROM folder… over the sheet); a
  superseded entry is marked, listed after its replacement, never preselected
  and offers "Use <replacement>". On a machine the choice applies from its
  next start (C:, its layers and inserted media stay); the same CMOS layout
  keeps its CMOS, another layout resets a stored CMOS to the bytes derived
  from the configuration (one warning in the sheet). The overlay binds the
  package ROM by digest at every start and in Save as new image….
  On an image, Publish revision never overwrites a tag (`latest` included),
  suggests the next minor version, or `<version>-local.N` for an official
  template (local only, said in the review), and says when user-supplied
  firmware makes the revision user-supplied. Machine and image details panes
  show "Newer version available" with the stored reason, a link to a newer
  tag of the image's repository that already uses the replacement, and
  "Switch to <release>…", which opens Settings on the Firmware tab with the
  replacement chosen; rows carry a quiet "↑ newer BIOS" badge with the hint
  as tooltip. The Firmware collection has a Status column ("Superseded by
  1.0.2-unemu.3"); its Role summary header reads Role. A machine's settings
  record the package digest and layer descriptor of each ROM they choose, so
  it keeps starting, saving and resetting after the package loses its tag;
  the Firmware pane's "Used by" and its Delete tag… confirmation (now offered
  for a selected package) name such machines. A crash between the record and
  the CMOS journal leaves a pending reset that Start finishes. Revisions and
  Save as new image… write `unemu.firmware.sources` from the packages the
  closure uses. Official templates are known by their bare repository name
  (so New machine…'s default `open-at` counts as one). Not yet: pulling a
  firmware package that a configured registry offers when OK is pressed
  (entries not in the store are unavailable with Import ROM folder…), and the
  reduced Machine files collection (Create image…, Open machine file… adding
  to the list); machine files keep their property sheets for now.
- **2026-11-16 — Open BIOS sources from their public GitHub repositories;
  open VGA BIOS `v0.9d-unemu.1` sounds the teletype BEL; templates 1.2.0:**
  `vendor/unemu-at-bios/fetch.sh` and `vendor/vgabios/fetch.sh` now build
  from <https://github.com/unemu-org/at-bios> (renamed from `unemu-at-bios`,
  a GitHub fork of `skiselev/8088_bios` with the same history, tags and
  revisions) and <https://github.com/unemu-org/vgabios>, read anonymously over
  HTTPS; deriving the address from this repository's `origin` is gone, and the
  `*_FLAKE` and `*_CHECKOUT` overrides stay. Both source notices name the
  public repositories. The AT BIOS keeps release `v1.0.2-unemu.3` and its ROM
  (`6938dc8b…ef2442`); its source archive keeps its name and root directory,
  and only its `NOTICE` changed: new pin `b928cee4…32ee` (from `59d3d921…9d67`).
  The open VGA BIOS moves to vgabios `v0.9d-unemu.1` (signed tag, revision
  `40522278`), based on upstream 0.9d: ROM `b867f384…a582`, INT 10h probe
  `7c928cec…f1b4`, source archive `vgabios-0.9d-unemu.1-src.tar`
  (`0f0890b6…6c48`). **MEASURED** on aarch64-darwin: three runs of each script
  from GitHub and one from a local clone build byte-identical archives, the
  extracted vgabios archive rebuilds both pinned files, and the staged AT BIOS
  tree rebuilds its ROM. Teletype output (INT 10h AH=0Eh, and AH=13h) now
  sounds the speaker for BEL: PIT counter 2 in mode 3, divisor 1331 (about
  896 Hz), for 31/64 s timed by port 61h bit 4 refresh toggles, interrupts on
  during the tone, port 61h and IF restored. **MEASURED headless** with the
  speaker trace (`ObservationPolicy::speaker_trace`; the window's
  `UNEMUPC_TRACE_SPEAKER` prints the same events): on the open profile the
  probe's AX=0E07h gives one start at C000:5866, mode 3, divisor 1331, and the
  stop at C000:588F after 577,819 PIT clocks (0.484267 s; 31/64 s is
  0.484375 s), and the probe reports `tty.bel ok`; the open AT BIOS's
  power-on melody (five tones from F000h) is unchanged. The new ignored golden
  `open_vga_bios_teletype_bel_sounds_the_speaker_for_31_64_seconds` pins this
  within 1%; with the 0.8a-based ROM it fails (`tty.bel FAIL`, no tone).
  Catalogue: new `vgabios-0.9d-unemu.1-std` (**Tested**);
  `vgabios-0.8a-unemu.1-std` is **Superseded** ("superseded by 0.9d-unemu.1:
  teletype BEL sounds the speaker") with its own archive pin
  (`vgabios-0.8a-unemu.3-src.tar`, `2d93fcaa…f8eb`), so machines and images
  made with it still start (**MEASURED**: `at-games-open.toml` pinned to the
  0.8a ROM, with that ROM, loads without the unlisted-firmware option and
  boots FreeDOS to `A:\>`); the catalogue self-test now accepts a superseded
  entry whose provenance tag is its id's release or a later release of the
  same upstream base (`v0.8a-unemu.3` for `vgabios-0.8a-unemu.1-std`), and
  requires the revision in its `fetch`. The AT BIOS entries' provenance names
  `unemu-org/at-bios`. `machines/at-games-open.toml` and the
  templates use the 0.9d ROM. New machine… lists a superseded video BIOS
  package under its package name but never chooses it by default; the
  capabilities and settings sheet labels the entry "unemu VGA BIOS
  0.8a-unemu.1 (std) (superseded)" (new tests for the video slot and the
  label). A superseded entry's `fetch` now names its script at the last
  revision of this repository that pinned its release (`4f73f3d0` for
  vgabios `v0.8a-unemu.3`, `bcb375e6` for unemu-at-bios `v1.0.2-unemu.2`):
  check out that revision and run it to rebuild the ROM and its archive.
  Goldens re-measured: the INT 10h probe 10,524,099 retired (from
  10,240,139), stop 0000:88A9 (from 880Ah), 14 screen lines with the BEL line,
  text `df1f232b…`; the mode 0Dh/13h run 10,688,267 (from 10,730,175), frames
  unchanged; the open FreeDOS boot and the 720K/360K run keep 15,216,991 and
  23,463,925 retired, with new screen hashes `41ff081e…` and `b4e1ac64…`
  (FreeCOM shows the time 12:00:21.30 instead of 12:00:20.98). The `at-ide`
  and at-pm-test goldens carry no video BIOS and are unchanged. The official
  templates and firmware packages move once, to version 1.2.0 (1.1.1 was
  never published but may be in a local store): pinned digests in
  [`crates/pc-image/tests/template.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/template.rs); the XTIDE package keeps its digest.
  `image template build --dry-run` against a scratch store reports every tag
  `new` and `guard:allowed`.

- **2026-11-16 — Library panes take the mock-ups' colours:** text fields
  and mini tables use the mock-ups' field colour in both skins and themes
  (the dark field was near black), striped rows their stripe colour, and an
  Images group row its own ground. Media per drive and the settings table
  draw as the mock-up's mini table: a thin outline, a muted header, a first
  column in the strong text colour (the mock-up's bold; no bold face is
  loaded) that wraps only past 35 % of the table, a 1 px rule under every
  row instead of stripes, and changed rows tinted across the table; on a
  narrow pane every cell stays inside the outline. The kind tile wraps its
  title between words ("Open AT" / "286"). A details pane taller than its
  area shows its scroll bar at rest, so the note under the actions reads as
  further down rather than cut off. Text fields, striped grids (the
  firmware import review, the installation media import tables, the
  firmware metadata editor's layer grid, Preferences › Registries, the push
  dialog's tables), scroll-bar troughs and progress bars take the same
  field and stripe colours. Headless tests check the colours, the table's
  rules, tint and narrow widths, the tile's word breaks, and that every
  pane's note is in view at the first height or scrolls into view.

- **2026-11-16 — Library machine and image panes and the library lists
  follow the approved mock-ups:** the machine and image details panes have
  the machines mock-up's kind tile beside the title, label and value rows,
  sections side by side on a wide pane and stacked under 720 px, and the
  actions as framed buttons in one wrapping row at the bottom with Start as
  the default; the details pane opens 420 px tall so they show. A machine's
  Media per drive is a table: header row, drives by letter (A:, B:, C:) with
  their type and media, a CMOS row, changed rows tinted with "61 030 sectors
  written · 29.8 MiB"-style lines and pending inserts. Firmware rows name the
  System BIOS, Video BIOS and Option ROM from the board catalogue or the
  library's firmware packages, with their open or user-supplied mark (a ROM
  in chips is open only when every chip is); the hardware line reads as the
  mock-up ("IBM 5170-class board · Intel 80286, 6 MHz (8/13) · 640 KiB + 1
  MiB · ATA at 1F0h"). The image pane adds the manifest's description and
  its base image. Controller keys (`ide:0`), medium ids, firmware keys and
  the board id no longer reach these panes. An image's redistribution mark
  (list, pane, tile) covers its configuration and every layer, as the push
  guard does, so an open BIOS with a user-supplied disk is user-supplied. The
  Machines, Images, Firmware and Installation media lists use their mock-ups'
  columns; each column keeps at least its header and a typical value whole,
  the rest of the area is shared out, and a list narrower than that scrolls
  sideways in its own area; digests and tags are monospace, counts and sizes
  right-aligned, the Images redistribution a chip. The Firmware and
  Installation media panes use the same type sizes and draw without overlap
  or truncated text. Headless tests draw the panes and lists in 760, 1000,
  1400 and 2000 px windows and check overlap, cut or truncated text, raw ids,
  type sizes, the drive table, the action row and the list headers. Not
  shown yet: an image's origin (pull source) and an official-template mark,
  which the store does not record.

- **2026-11-15 — `image media import` reproduces the official FreeDOS sets:**
  the command line could not express three fields the template build gives
  the FreeDOS 1.4 Floppy Edition sets, so a set imported by hand was always
  another set digest. New set-level flags: `--version V`, `--edition E` (the
  title record's, once each) and a repeatable `--set-label L` (the set's
  labels, in the order given; a repeated label is refused). The preview and
  `image media show` print them. A fixture-gated test imports each of the
  three sets with the command line and compares its digest with
  `build_media`'s: all three are byte-identical, and without the new flags
  the 1.44M set is another digest. The three documented commands give the
  published sets' digests. Without the flags nothing changes; the shareware
  import commands give the same digests as before. The commands are in
  [commands](commands.md) (`image media import`).
- **2026-11-15 — FreeDOS keeps its XMS after Ctrl+Alt+Del: open AT BIOS
  `v1.0.2-unemu.3`, open VGA BIOS `v0.8a-unemu.3`, templates 1.1.1:** with
  FreeDOS 1.4 loaded high over FDXMS286, a restart through guest
  Ctrl+Alt+Del or an 8042 CPU reset used to boot into "VDISK has been
  detected. Driver won't be installed.": MEM showed Extended (XMS) 1,024K
  total and 0K free, with the kernel and FreeCOM in conventional memory,
  until power off. **MEASURED** cause: the FreeDOS kernel marks the HMA in use
  with a VDISK boot record at 100000h ("VDISK" at 100003h, kernel/inithma.c
  InstallVDISK); RAM keeps it across both restarts, as on the AT, and POST of
  `v1.0.2-unemu.2` never wrote memory above 1 MiB, so FDXMS286's check_vdisk
  found the old session's record. CMOS 30h–31h and INT 15h AH=88h were
  correct; the emulator was not at fault. unemu-at-bios `v1.0.2-unemu.3`
  (revision `4548c176`, ROM `6938dc8b…ef2442`, source archive
  `unemu-at-bios-1.0.2-unemu.3-src.tar`, `59d3d921…9d67`) clears every
  64 KiB block its POST sizing finds above 1 MiB, cold or warm. Its catalogue
  entry `unemu-at-bios-1.0.2-unemu.3` is the one `machines/at-games-open.toml`,
  `machines/at-ide.toml` and the templates use; `unemu-at-bios-1.0.2-unemu.2`
  stays listed as a superseded open entry (new `FirmwareStatus::Superseded`)
  with its own source archive pin (`6c7aaace…a47e`), so machines and images
  made with it still start. The catalogue self-test holds the entries a fetch
  script builds to that script's archive pin; a superseded entry needs a
  well-formed pin of its own release. MEASURED with `v1.0.2-unemu.3`: 865K
  XMS free after Ctrl+Alt+Del and after an 8042 reset, as after power-on, on
  the open game profile and on `at-ide` with 1 MiB. The ignored golden
  `xms_warm_boot_golden` leaves the kernel's record at 1 MiB, restarts both
  ways and checks that it is gone
  (it failed with `v1.0.2-unemu.2`); `UNEMUPC_OPEN_AT_BIOS` runs it against
  a candidate image instead. The open goldens moved with the BIOS:
  `open_game_profile_boots_freedos` 15,216,991 retired (from 15,241,264),
  `open_game_profile_reads_720k_in_b_and_360k_in_a` 23,463,925 (from
  22,988,197), both with new screen hashes (the banner names the new
  release), the at-pm-test open run 6,590,023 (from 6,589,934, every line
  still passing), the INT 10h probe 10,240,139 (from 10,240,118) and the mode
  0Dh/13h run 10,730,175 (from 10,730,616), screens and frames unchanged. The
  `at-ide` goldens and the 0 KiB extended-memory window tests are unchanged.
  The open VGA BIOS moves to vgabios `v0.8a-unemu.3` (revision `72bf5d98`):
  ROM and probe bytes unchanged, new source archive
  `vgabios-0.8a-unemu.3-src.tar` (`2d93fcaa…f8eb`, nothing left out now that
  the fork no longer tracks its issue tracker's settings). The official
  templates and firmware packages move once, to version 1.1.1 (pinned
  digests in [`crates/pc-image/tests/template.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/template.rs); the XTIDE package keeps its
  digest). Machines made from templates 1.1.0 keep the `v1.0.2-unemu.2` BIOS,
  without the fix; a machine made from 1.1.1, or a machine file, gets the
  fix. **MEASURED** with that ROM rebuilt from its release: a machine file
  pinned to its hash and an image made with its package both start and show
  its banner; with the superseded entry removed from the catalogue both are
  refused. New machine… still offers the superseded package but never
  chooses it by default, and firmware labels mark the entry "(superseded)".
  **READ:** Settings on an image or on a machine made from one does not edit
  firmware ("Firmware stays as the image stores it",
  `window/src/image_settings.rs`); firmware edits in Settings are separate,
  planned work.

- **2026-11-15 — Sound card and PC speaker host levels:** the SB16 was audible
  only at a high host volume while the PC speaker was startlingly loud. Measured
  through the window presenter and its host sink on the open AT games machine
  (machine file and `open-at-games` template image alike): a full-scale 8-bit
  DSP square peaked at -28.0 dBFS, one FM channel at full output at -46.3 dBFS,
  nine FM channels at -30.7 dBFS and the speaker at -6.0 dBFS. Nothing was
  detached or muted: the CT1745 reset of master, voice and MIDI to level 24 (-14
  dB per stage, -28 dB together, as Creative's guide specifies) met a host full
  scale equal to the card's maximum, and the speaker entered unattenuated at
  0/+16384. The CT1745 now resets to 0 dB (**ASSUMED** deviation, as 86Box and
  DOSBox Staging do), and assembly sets fixed host mix levels: card voice × 1/2,
  card FM × 3/4 (1.5 times the voice, the OPL gain DOSBox Staging uses after
  finding 2.0 too high), speaker 0/+8192. Now a full-scale voice peaks at -6.0
  dBFS, one FM channel at -20.6 dBFS, nine at -5.2 dBFS, and the speaker at
  -12.0 dBFS; the speaker's audible swing, ±4096, is 12 dB under the voice's
  ±16384. All three sources together peak at -1.5 dBFS without clipping. The
  speaker, SB16 byte/word and OPL3 goldens have new PCM hashes. AdLib detection
  (timer status 00h then C0h) passes; headless, Duke Nukem II wrote no SB16
  mixer register before its title screen. Tests pin the levels for a synthetic
  machine, the shipped machine file and the template image. See [audio design
  §Host mix levels](design/audio.md#host-mix-levels).

- **2026-11-15 — Tests no longer play the POST melody on the host's
  speakers:** the window's shell tests start real machines, and starting a
  machine opened the host audio output, so every test run on a developer's
  Mac played the open AT BIOS POST melody at random moments (the
  maintainer heard it at normal and at slowed speed, even with the emulator
  closed). It sounded like stray beeps from a running guest; it was never
  the emulator in use. In test builds the host audio output now behaves as
  on a host without an output device; a test pins that opening it starts no
  stream.

- **2026-11-15 — macOS system shortcuts can go to the guest (opt-in):**
  macOS acts on Ctrl+arrows (Mission Control, move a Space), Ctrl+digits,
  Ctrl+F1–F8, Ctrl+Space and F11 before the window sees them, so games that
  fire with Ctrl and move with the arrows switched Spaces. Preferences ›
  General › "Send macOS system shortcuts to the guest while it has the
  keyboard" (off by default, saved as `send_system_shortcuts`) asks for
  Accessibility access when turned on and then installs an active Quartz
  event tap (HID level, session level as fallback) only while a running,
  unpaused guest has the keyboard and no dialog is open. The tap takes Ctrl
  with an arrow, navigation key, digit, Space or function key, and every
  function key, never a Command chord; it hands them to the window's ordinary
  key queue, so focus checks and the focus-loss release of held keys are
  unchanged. Focus loss, pause, dialogs, stop and quit remove the tap; a tap
  macOS switched off for a slow answer is reinstalled; a refused tap leaves
  one status hint. Without the option the shortcuts stay with macOS; Help and
  `machines/README.md` § macOS system shortcuts in games give the System
  Settings › Keyboard Shortcuts › Mission Control workaround. Decision logic
  and lifetime are tested headless; the tap itself is checked by hand
  (Duke Nukem II with Ctrl+arrows, ASSUMED until the maintainer's check).
  NSApplication presentation options cannot block Mission Control, so they
  are not used.

- **2026-11-15 — An unsupported-instruction stop names CS:IP and the
  instruction bytes:** the CPU core reports the bytes of an instruction it
  does not lift, but the stop message used to show only the offset
  ("unsupported instruction at 0x72b"), so a field report could not tell a
  missing opcode from a jump into data. `CpuError::UnsupportedInstruction`
  now carries the visible code selector and the bytes, and the message reads
  `unsupported instruction at 0700:072b (bytes 0f 20 c0)`; the window's
  machine error shows it as is. Found with Commander Keen 4 1.4, which stops
  this way after its title screen on the open AT games profile. The cause is
  still open.

- **2026-11-15 — Machines in the desktop library and the media picker
  (approved machines and media-picker mock-ups):** the library opens on
  **Machines**. The sidebar is one Library group in the mock-up's order —
  Machines (Recently played, Favourites, which list machines), Images,
  Installation media, Firmware, Machine files — with the selected
  collection's facets under it and a greyed, collapsed Remotes group. The
  library worker runs the machine migration when it opens the store and the
  Machines list shows a one-time notice of the adopted sessions. The Machines
  table has the mock-up's columns (name with the machine icon and ★, source
  image with its short digest and the ⚑ tag-moved and detached badges, the
  backend's State with "A: insert pending", last run, size of the changes)
  and sorts by column; Recently played lists the last run first. The details
  pane names the workspace id and the command line, links the source image,
  lists media per drive (the image's against the machine's writes and pending
  disk), the settings that differ from the image by the property sheet's
  field names, and the image's hardware, with Start, Gaming mode, Settings…,
  Insert media…, Save as new image…, Disk layers…, Duplicate…, Rename…,
  Reset to image… and Delete machine…; Enter and a double-click start it.
  Start on an image makes a machine named after it and starts it, or with
  machines opens Start… (last run first, New machine last). An image's pane
  lists Machines from this image (N) with links and adds New machine from
  image… (Ctrl+Shift+N), Gaming mode and Delete tag…. Settings… on a machine
  opens the approved property sheet in machine mode (banner, differences
  against the image, OK/Cancel/Apply, read-only while it runs) and keeps the
  tables as the machine's overlay; Image › Settings… still publishes a
  revision. Save as new image… of a machine asks what happens to the machine
  (moves onto the new image by default, or keeps its changes), with the
  repository defaulting to the machine name at 1.0.0. Gaming mode on a
  machine plays a disposable copy of it as it is; the end dialog offers
  Discard, Keep as new machine… and Keep as new image…, as does the
  Unfinished sessions banner. File › New machine… also makes a machine from
  the new image unless its Name and tag step's check box is cleared. Menus:
  Edit (Rename… F2, Duplicate… Ctrl+D, Delete machine… / Delete tag…
  Delete), Image (named target, Start…, New machine from image…, Gaming
  mode, Settings… (new image revision), Layers…, Push…, Pull…), Machine
  (named target, Insert media…, Duplicate…, Rename…, Reset to image…, Delete
  machine…, Show source image); disabled items say why in one phrase; the
  library keys act outside guest focus, as the mock-up says, and never
  while a text field is edited; Help › Keyboard shortcuts lists them. **Media picker:** every
  Insert… (Media menu, drive chip, toolbar A: and B:, Ctrl+Alt+I and
  Ctrl+Alt+Shift+I) opens it, replacing Browse / catalogue… and From
  library…. It shows the target (the running machine, or any stopped machine
  with a floppy drive), the drive with its format and contents, and when the
  disk goes in; sources This machine and This library, greyed remotes;
  search with free words and `field:value` words (format, publisher, genre,
  platform, label, year and year ranges) ranked title first; Format,
  Platform, Genre, Year, Publisher and Labels facets with counts over the
  result; the accepted-media fit rule with "N don't fit — Show" and where a
  dimmed set would fit; the disk to insert (the next disk when the drive
  holds one of the set), all disks in order, and the write setting in plain
  words; ↑↓, Enter, Alt+A/Alt+B, Escape. Into a stopped machine Insert
  stores a pending insert (replacing the drive's earlier one), shown in its
  details, its State, the drive chips ("· at Start") and the Media menu
  (Take out again); into a running or halted machine the disk goes in at
  once through the existing library insert; a refused disk only says why
  and the machine keeps running. Browse for an image file… opens the system
  file chooser, then that file's own Insert floppy step. The Installation media pane's Insert into…
  ▾ lists the drives of every machine that take the set, or one short
  reason. pc-image gains `machine::prepare_copy`, `keep_copy` and
  `publish_copy` (the copy, keep and publish steps of Gaming mode on a
  machine, which need its crate-private journal copy, insert and overlay
  publication). **MEASURED headless:** worker tests run a machine (its run
  is confirmed only after the shell reports assembly), play and keep a gaming
  copy, save a machine as a new image (and refuse a taken version), migrate
  once; view tests draw the tree order, the list and details pane, the
  Start… chooser, name rules; picker tests cover fit, search, facets, the
  default disk, the summary, the pending insert and the keys' row walk; a
  pc-image test copies, keeps and publishes a gaming copy. **Manual
  checks:** the whole flow in a window (both skins, light and dark), on
  macOS for ⌘D, ⌘⌫ and ⌘⇧N.
  - **Mock-up departures, awaiting maintainer approval:**
    - Add to machine… stays disabled ("Not available yet"); the mock-up item
      is open, not done. Adding a set to a drive's catalogue as machine
      state kept by Save as new image… needs a stored catalogue in the
      machine record, applied at Start and written into the new image's
      configuration by the workspace publication; upc-5pw.71 tracks that
      backend work.
    - The set pane keeps its running-machine Insert button beside Insert
      into… ▾. It is enabled only while a machine runs, and the library is
      not shown then, so in practice Insert into… is the way in.
    - This machine in the picker lists a stopped machine's catalogue as
      empty with a note (the image's own disks are in the drive at Start;
      Next disk swaps them once it runs): a pending insert can name only a
      set disk. Browse for an image file… into a stopped machine says to use
      a set of the library instead.
    - Configured remotes are not listed one by one in the picker: one greyed
      "Remotes · after the MVP" tab, and the remote preview is a short text
      with a disabled Pull and insert instead of a sample row.
    - The details pane puts the two rows of actions above the media and
      settings columns (the pane is short; below they would need scrolling),
      and keeps a Favourite check box and a labels field where the mock-up
      shows only label chips.
    - Save as new image… of a machine lists what is saved as lines (drive and
      sectors written, CMOS, settings, pending inserts), not the mock-up's
      four-column table with new bytes per part.
    - Settings… on a machine marks a differing field "Image: … · ● differs
      from the image" without the "Use the image's value" link.
    - Disk layers › Discard changes on a machine resets it (as Reset to
      image… does) after the Disk layers dialog's own confirmation, not the
      Reset to image… dialog.
    - The running status line keeps today's text ("Running — click the
      screen…") instead of "Running · NAME · REF at sha256:… · workspace ID";
      the window title names the machine and its image as the mock-up does.
    - The Images table keeps the library mock-up's columns; the machines
      mock-up's Machines count column is not added (an image's details pane
      lists Machines from this image (N)).

- **2026-11-15 — Speaker and CPU-reset trace (opt-in diagnostic):** with
  `UNEMUPC_TRACE_SPEAKER=1` in its environment (read once at startup; any
  other value leaves it off) the window prints one stderr line per event:
  `speaker on` / `speaker tone` / `speaker off` when port 61h GATE and data
  become both set, counter 2 is reprogrammed while they are, or one drops
  again. Each line carries the guest time, PIT clock, retired count, the
  CS:IP after the OUT, and counter 2's mode, divisor and frequency, or how
  long the speaker was on. A `cpu reset` line carries its source (`reset-pulse`
  from the 8042 or port 92h, `cpu-shutdown`, or `host` for the halted-machine
  restart), CMOS byte 0Fh and the restart CS:IP. Run it with
  `UNEMUPC_TRACE_SPEAKER=1 nix develop --command cargo run --release
  --offline -p window --bin unemupc 2> ~/unemupc-speaker.log`.
  `ObservationPolicy::speaker_trace` builds it in assembly; it only reads
  timer, CMOS and CPU state after guest port writes and around frontend
  resets (`AssembledMachine::reset_cpu`). With it off, assembly installs
  nothing and a headless test shows identical clocks, retired counts and PCM.
  A virtual second prints at most 32 speaker lines and 32 resume resets
  (shutdown 05h, 09h or 0Ah; FDXMS286 memory moves cause many of these), then
  one count of what it left out; resets that run POST always print. Measured
  on the open AT: the power-on melody is five `speaker on` lines at
  `F000:A66A` with divisors 2153, 4307, 3224, 4307 and 2875, each off after
  0.185 s. The open VGA BIOS ignores BEL (vgabios.c, teletype case 7), so
  on VGA machines DOS's BEL character makes no sound. Investigation of
  spurious POST-like sounds: headless runs through the window presenter
  (pauses, stalls up to 10 s, speed changes, mute, XMS block-move resets,
  copying to C:) gave silence after POST; the trace is there to catch the
  maintainer's case.
- **2026-11-15 — FreeDOS 1.4 Floppy Edition installation sets, open with
  their source; copyleft media need their source:** `image template build`
  now also builds `unemu/media/freedos-floppy-1.44m`, `-1.2m` and `-720k`
  (version tag `1.4.0` and `latest`; `pc_image::template::build_media`,
  separate from the firmware and machine build, whose `1.1.0` pins are
  unchanged): the official edition's 7, 8 and 12 disks, byte for byte, in
  installer order, with the licence index `LICENSES.TXT` (CC0-1.0; every
  program's licence with the notice it rests on) and a deterministic source
  tar as extras, open under the conjunction of the programs' licences
  (including `Zlib`) and judged by the `unemu/` guard before they are
  tagged. The release documents state no licence and are not carried. The
  build checks every set fixture before its first tag.
  [`vendor/freedos/fetch-floppy-1.4.sh`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/vendor/freedos/fetch-floppy-1.4.sh) (the last step of the CI fixtures
  lane; CI caches its outputs, and a run whose outputs match their pins stops
  at once) fetches the edition (`FD14-FloppyEdition.zip`, the SHA-256 the
  release's `verify.txt` lists, ibiblio fallback), checks its member list,
  pins all 27 disks, takes the official source packages of the 69 programs
  from the release LiveCD (a small ISO 9660 reader; the LiveCD is deleted
  once the packages verify) plus FDBANNER, and the installer, its
  translations and the release builder at their `FreeDOS-v1.4` tags. `image
  media import --open` also takes an SPDX expression with `--source EXTRA`
  (an archive extra other than the licence text), and push-guard rule 4
  covers copyleft media and linked set extras. See [machine images §5.2 and
  §5.5](design/machine-images.md#55-official-template-images) and
  `vendor/freedos/README.md`.
- **2026-11-15 — Desktop icons follow the approved mock-ups:** the app now
  draws the mock-ups' icons ([icons note](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/icons.md)). The inline SVGs
  are painted with egui shapes from their own coordinates, in the text colour
  where the SVG says `currentColor` and in the mock-up's fixed colours
  elsewhere, so no SVG renderer or icon font enters the build: the
  application monitor in the classic title strip; the toolbar's green Start,
  Pause, red Stop, floppy, Eject, gear and blue-topped Layers (Ctrl+Alt+Del is
  code text without an icon, as in the mock-up); the image, media-set,
  firmware and machine-file icons in the tree, the list rows and an image's
  details heading; ⓘ in the image Settings and Edit metadata notes; the ?
  and ✕ discs of Move tag? and Version tag already exists; the empty
  library's monitor-and-keyboard picture. Text marks: ▶ Ⅱ ■ in the Machine
  menu's mark column, ⏏ on the Media and drive menus' Eject, ⚠ after the
  name of a row with a problem (instead of a `[!]` prefix), ★ after a
  favourite's name, 🔊/🔇 in place of the word "Audio" in the status bar,
  ⚠ before Unfinished sessions, ✓ on finished wizard steps and free tags.
  **Fixed tofu:** egui's bundled fonts have no ✓, ✗, ✕, Ⅱ or ⌫, so every ✓
  and ✗ the app wrote (registry checks, Push… and Get templates… results,
  Corresponding source attached, the publish validator line) drew as an
  empty box; a five-glyph DejaVu Sans 2.37 subset (Bitstream Vera licence,
  16 KiB, reproducible command and hashes in
  `crates/window/assets/fonts/dejavu/NOTICE.md`) now follows egui's fonts.
  About, `--help` and `--version` credit it, and `deny.toml` allows
  `Bitstream-Vera` for the `window` crate only. The library's name column is
  left-aligned now and a repository group's rows are set in, as in the
  mock-ups (a centred egui label dropped the room the icon needs).
  **Inventory:** 34 SVG uses and 29 distinct icon characters across the
  eleven approved mock-ups (media-picker included); 23 SVG uses were painted
  then (25 with the machine icon, entry above), and ⌫, ⚑, ▾ and □
  were the only characters drawn nowhere (□ alone since). **Not drawn, with reasons:** the library and
  machines mock-ups' other toolbar buttons (New, Properties, Browse
  registry…, Push, Delete, Gaming mode, New machine from image…, Save as new
  image…), because the app has only the machine toolbar; the Machines
  collection's icons (monitor, ⚑ tag moved, ● differs, its ⓘ notes, ⌫
  shortcuts) and the media picker's own marks (the toolbar's B: floppy
  button, ▾ on Insert into…, the ↑↓ keys of its note) were drawn later with
  those screens (entry above); View's ✓/● marks,
  which egui's check boxes and selection show; the drive menu's ● write mode
  (named as text, an accepted departure); the library mock-up's ★ rating (no
  rating field); □ window buttons (the system title bar). **MEASURED
  headless:** tests read every mock-up and fail on an icon without an
  inventory entry or an entry without its icon; check that every non-ASCII
  character in the desktop's string literals and every mock-up text mark has
  a glyph in the proportional and monospace families (before the subset:
  ✓ U+2713 and ✗ U+2717 had none); paint every icon in both skins and both
  themes; and check the toolbar, title strip, menus, rows, empty card and
  audio cell. **Manual checks:** look at the icons in both skins and in light
  and dark themes, on macOS and on Windows or Linux (the title strip icon is
  drawn only there).
- **2026-11-15 — Machines as named local instances of images (backend and
  command line):** the store keeps machine records in `local/machines.toml`
  (schema 1, atomic, index-locked; machine-images §3.2): a fixed workspace
  id, a name unique ignoring case, the pinned image and the tag it came from,
  labels, favourite, last run, pending inserts and a settings overlay.
  `pc_image::machine` creates machines (default name the image title, then
  `title (2)`) and lists them with derived state (Never started, Same as
  image, Changed per drive + inserted media + CMOS + settings, Running;
  detached and tag-moved; a per-machine problem). It renames, duplicates,
  resets and deletes them, changes settings (SETUP fields of CMOS recomputed
  as for image Settings) and saves as a new image (by default moving the
  machine onto it with fresh journals, or keeping it on the old image).
  Start is `prepare` plus `confirm` after assembly, so a failed assembly
  keeps pending inserts and the last run. Opening a store for machine work
  migrates desktop `regular-` workspaces to machines named after their image
  title, keeps command-line workspaces' names and leaves Gaming-mode
  workspaces alone; a desktop workspace made again later is adopted as
  `title (2)`. `image machine ls|new|rename|duplicate|reset|rm|settings|commit|run`
  mirrors the desktop actions. Tests cover every operation, migration from an
  old-layout store (including a busy and a re-created desktop workspace),
  reserved ids, an unjournaled CMOS and pending inserts. The slow golden
  `machine_freedos_golden` (ignored) boots FreeDOS through a machine twice,
  checking that its floppy journal keeps a written file, then resets it.
  The desktop starts machines itself (entry above); rolling back to an older
  build: §3.2.
- **2026-11-15 — A halted machine takes a floppy and Ctrl+Alt+Del restarts
  it; a stopped machine takes floppies for Start:** a guest that halts with
  interrupts off (the open AT BIOS after "No ROM BASIC", when the boot code
  of an empty disk gives up through INT 18h) no longer ends the session. The
  status line reads "Machine halted — insert a boot disk and press
  Ctrl+Alt+Del, or Stop to return to the library"; Media › Insert, Browse…,
  From library…, Eject, Next image, the drive chips and the host media keys
  act on its drives as on a running machine; Pause/Resume is disabled.
  Ctrl+Alt+Del (toolbar, Ctrl+Alt+End and Machine › Reset halted machine)
  restarts a halted machine with a CPU-only reset, the line the 8042 pulse and
  the shutdown cycle use: unlike a hardware reset button it resets no board
  device; RAM, devices and the inserted media are kept, POST reinitialises the
  board and runs again in the same session, keys held at the halt are
  dropped, and the reset takes no virtual time. On a running machine
  Ctrl+Alt+Del still sends the keys to the guest. A halted session's journals
  and CMOS stay open for the reset: at the halt, changed CMOS RAM and
  completed ATA sectors are made durable as before a reset, and Stop finishes
  the session (before, the halt finished it). An error dialog ends a halted
  session's input but Stop still finishes it. Stop, Library and Exit still
  end a halted session without the stop prompt, and Gaming mode still asks
  Keep or Discard. While no session runs, Media › Insert, Browse… and From
  library… (and the A: toolbar button) put floppies into the selected
  machine file's or machine image's drives, under the usual accepted-media
  and write-policy rules; the drive chips show them, Eject takes one out, and
  Start inserts them before the first instruction, so POST boots from them.
  They are kept only in the shell, never written to the machine file or the
  stored image, and last until Start, until another machine is selected, or
  until the library reads the machine file again (Settings save, firmware
  check), which takes them out. At Start a disk the machine file lists goes
  in only if the file still lists the same image there, and a Browse… disk
  only if its SHA-256 is unchanged; Start names each drive left out and
  inserts the others.
  Before an image starts, From library… disks go in write-protected, since
  their journals belong to the workspace Start opens; an image's own catalogue
  joins at Start. **MEASURED headless (shell, open AT BIOS
  `unemu-at-bios-1.0.2-unemu.2`, 1.2M A:):** a disk whose boot sector runs
  INT 18h reaches "No ROM BASIC" and halts; Insert of `freedos-1200.img` and
  Ctrl+Alt+Del then boot to the FreeDOS kernel banner; with A: empty, the same
  disk inserted before Start boots FreeDOS too (both `#[ignore]`d, about 22 s
  and 11 s in a debug build).
- **2026-11-15 — File › New machine… keeps one size:** an egui modal lays its
  content out inside last frame's size, and the wizard's vertical separator
  between the step list and the page filled all the height on offer, so the
  dialog grew 32 px on every repaint without bound and its content left the
  window. The body between the title and the buttons now has a fixed height,
  the window height minus the dialog chrome, between 240 and 460 px; the page
  scrolls inside it. **MEASURED headless (30 frames, every step, both skins,
  1040×700 and 1040×500):** before, the wizard grew from 432 to 1360 px at
  1040×700; now it holds 532 px at 1040×700 and 382 px at 1040×500 from the
  first visible frame and stays inside the viewport. The same check found
  bounded growth in Import installation media…, Import ROM folder…, Publish
  new image revision (image Settings › Review), firmware Edit metadata and
  README: each grew for 2 to 7 repaints, up to 56 px each, until its scroll
  area reached its cap. Those scroll areas now take their full height at once
  (`min_scrolled_height` equal to the cap). Machine Settings, image Settings
  (every tab), Save as new image (Disk layers form and its prompts),
  Preferences (both tabs), Push…, Pull… and Get templates… (every stage the
  tests reach) already held one size. Known, unchanged: at the minimum 500 px
  window height, Publish new image revision (538 px) and one Push stage
  (516 px) are taller than the window.
- **2026-11-15 — Selected rows are legible in every skin and theme:** the
  classic skin paints selections navy but kept the light theme's dark-blue
  selection text, and library table rows drew selected text in the ordinary
  label grey. Library rows now take the selection stroke colour, as egui's
  selectable items do, and the classic skin's selection text is white.
  **MEASURED (WCAG 2.x contrast, selected text on selection fill):** classic
  light was 1.21:1 for table rows and 1.10:1 for selectable items; now every
  skin × theme is at least 5.03:1 (modern light 5.03, modern dark 5.32,
  classic light and dark 9.09). Side effect: egui outlines a focused text
  field in the selection stroke colour, so in the classic skin that outline is
  now white.
- **2026-11-15 — Library rows are one per manifest; machine files only under
  Machine files; resizable details pane; no logo entry:** tags of one
  repository that name the same manifest (a version tag and its `latest`
  alias, as `Get templates…` and pulls leave them) are now one row in the
  machine list, the Installation media collection and the Firmware
  collection. The row lists every tag, version tags first (`1.1.0, latest`)
  in the Tag column and the details pane; a search for any of its tags finds
  it. Row, pending and footer counts and the tree's Firmware and Installation
  media counts count these rows, so a package under a version tag and
  `latest` counts once. A repository group header still counts tags. Different manifests stay
  separate rows (firmware `:1` and `:2` after Edit metadata…), and so does one
  manifest in two repositories. Start, Settings…, Layers…, Save as new
  image…, Push… and Edit metadata… act on the row's version tag. Delete tag…
  names the tag it removes, offers the row's other tags as choices and keeps
  its confirmation; the store still records a deleted version tag as retired.
  Deleting one tag of a row leaves the row and its selection under the other
  tags, and the selection follows the row across refreshes. The store and CLI
  output are unchanged. All images now lists stored machine images only:
  machine files from disk list only under Machine files, whose footer reads
  `N of M machine files`; Recently played and Favourites list stored images,
  as before. The split between the list and the details pane drags (the
  library mock-up's splitter), shared by every collection, starting at
  232 px; the pane keeps at least 120 px and the list 160 px, the height
  lasts for the session, and the pane scrolls only when its content is
  taller. The sidebar's collapsible `unemupc98` logo entry is gone; the
  approved main-window and library mock-ups have no such entry. Its
  animation went with it: the About dialog keeps the static logo, and the
  file-only `reduced_motion` preference now affects only the indeterminate
  firmware-scan progress bar.
- **2026-11-15 — Template firmware packages describe themselves:** `image
  template build` now gives each of its three firmware packages a title
  (`unemu AT BIOS 1.0.2-unemu.2`, `unemu VGA BIOS 0.8a-unemu.2, standard VGA`,
  `XTIDE Universal BIOS r638, XT+ build for ATA at 1F0h`), a one-paragraph
  description (what it is, the machines it serves, its licence and that its
  source archive is included; the system BIOS names Sergey Kiselev as the
  original author of 8088_bios), a vendor, a version (`1.0.2-unemu.2`,
  `0.8a-unemu.2`, `r638`) and labels. They go through the same statement as
  the metadata flags of `image firmware import`, so the config, the generated
  README and the standard `org.opencontainers.image.title`, `.description`,
  `.vendor` and `.version` annotations carry them; before, registry and
  library views showed the catalogue id and no description. No source URL or
  revision annotation is added (the catalogue provenance text is unchanged).
  Generation `1.1.0` is not published yet, so it keeps its version with new
  pins: the packages `9c482af2…7daa`, `770f76c1…ac1c`, `240d2d4f…89ec`,
  `open-at` `e2618810…7e0f` and `open-at-games` `8dfa958d…273b` (the machines
  record their package manifests, so they move too). See
  [machine images §5.5](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md#55-official-template-images).
- **2026-11-15 — CI on Linux: pinned download tools and CA roots, no worker
  race in window tests:** the devShell now provides `curl` and Info-ZIP
  `unzip` 6.00 and points `NIX_SSL_CERT_FILE` and `CURL_CA_BUNDLE` at the
  pinned nixpkgs CA bundle. The CI container's own bundle could not verify the
  FreeDOS package mirror's current root, so the `fixtures` gate stopped in
  `vendor/freedos/fetch.sh` and the goldens then missed the later fixtures.
  The gate now prints a header before each script. **MEASURED on
  aarch64-darwin:** with the pinned bundle minus that root, the devShell's
  curl fails as CI did (`self-signed certificate in certificate chain`); with
  the devShell's bundle it verifies. In a fresh tree every fixture script (the
  `fixtures` gate, LTEMM and the three shareware floppies) reproduces all
  pinned hashes with the devShell's tools. `vendor/hdd/build.sh` runs mtools
  with an empty `HOME`, so no `~/.mtoolsrc` applies. Window tests that wait for
  a background worker no longer fail after 10–60 s on a loaded runner: a
  five-minute hang guard is their only time bound. A Linux CI run is pending.
- **2026-11-15 — Floppy drive and medium types are separate; open AT B: is
  3.5-inch 1.44M:** the µPD765 model installs a `DriveType` per unit
  (360K, 720K, 1.2M, 1.44M, or a custom geometry taking only itself),
  separate from the inserted medium's geometry. A 1.44M drive takes 1.44M and
  720K media, a 1.2M drive 1.2M and 360K media, 720K and 360K drives their own.
  The medium sets the rate it answers at (1.44M/1.2M 500 kbps, 720K 250 kbps,
  360K 300 kbps in a 1.2M drive and 250 kbps in a 360K drive); other CCR rates
  fault with ST1 MA as before. 360K media in a 1.2M drive are double-stepped:
  the PCN is the physical cylinder, track `PCN/2` lies under the head at an
  even PCN and carries that track as its ID cylinder; an odd PCN finds no ID
  field (MA) and a C other than the track's ID ends with ST1 ND + ST2 WC.
  ASSUMED, unchanged: single-stepped media address the command's C without
  comparing it with the PCN. TRACK 0 follows the physical cylinder; two-side
  and write protect follow the medium; the change line stays per drive. The
  3.5-inch rates are ASSUMED (the IBM AT reference predates those drives).
  Drive types are wiring, not snapshot state, so the snapshot stays v5.
  Assembly, the machine file, Media › Insert, Browse…, From library… and the
  drive chips share one rule (`FloppyGeometry::accepted_media`): a drive image
  or catalogue entry is accepted when the drive takes its size-derived format,
  else `ImageGeometryMismatch` names the drive type and its accepted media
  (for example `a 1.44M drive takes 1.44M (1474560 bytes) or 720K (737280
  bytes) media`). A catalogue may mix the formats a drive takes; chips show the
  medium's own format. CMOS 10h still encodes the drive types (a 720K disk in
  a 1.44M B: leaves type 4). `machines/at-games-open.toml` and the open
  templates (still `1.1.0`, re-pinned: `open-at` `e3bccef9…3b75`,
  `open-at-games` `b5c48e53…10c9`) gain an empty 3.5-inch 1.44M B: (unit 1).
  `machines/at-games.toml` keeps A: only: READ, the local IBM 06/10/85 listing
  excerpt lacks `DSKETTE.ASM`, which holds the drive-type support (TEST1.ASM
  lines 67–70 name it), so whether that BIOS has a 720K type 3 is not
  verified, and `floppy_disabled_reason` still refuses only 1.44M on it.
  **MEASURED at CPU `060d446a`:** at-games-open FreeDOS still reaches `A:\>`
  at 15,241,264 retired with text hash `41ff081e…469e` (unchanged with B:);
  a new golden inserts a 720K FAT12 disk into B: through Media › Insert,
  lists it and TYPEs a file at cylinder 33, then swaps a 360K disk into the
  1.2M A:, lists it and TYPEs a file on track 11 (the head ends at physical
  cylinder 22): 22,988,197 retired, text hash `b9e3a1cb…f16a`. The halting
  open VGA goldens on this profile move by one instruction with B: (INT 10h
  probe 10,240,119 → 10,240,118; mode 0Dh/13h 10,730,617 → 10,730,616; screen
  and frames unchanged; with the old profile on the new code they keep their
  old counts). ASSUMED explanation, as for the BIOS switch: the BIOS's extra
  second-drive setup shortens its own timer busy-waits.
- **2026-11-15 — Media sets carry the standard OCI annotations:** an
  installation set's manifest now carries `org.opencontainers.image.description`,
  `org.opencontainers.image.vendor` (the publishers) and
  `org.opencontainers.image.authors` (the developers), lists joined with `, `
  in the order stated, beside the title (and `org.opencontainers.image.licenses`
  of an open set), all from the set's title record, so registry web UIs that
  read the standard annotations show them ([`design/machine-images.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md) §5.2).
  An empty field adds nothing: a set without catalogue fields keeps its digest
  (the shared and CLI pin tests are unchanged). There is no creation time (no
  clock) and no URL (the title record has no such field). The existing-tag
  comparison the library's Import installation media… dialog uses checks the
  four descriptive annotations, so an identical re-import is already imported
  and a set stored without them is a tag conflict; the dialog shares the plan,
  so its sets get the annotations too. The shareware import commands
  (`commands.md`, each `vendor/shareware/*/README.md`) add a one-paragraph
  `--description`; that changes those sets' digests again, so a store that
  already holds the tags needs a fresh store or new tags.
- **2026-11-15 — Open AT BIOS: unemu-at-bios `v1.0.2-unemu.2`:**
  `vendor/unemu-at-bios/fetch.sh` builds release `v1.0.2-unemu.2` (revision
  `a2cf362d`) of the unemu fork of Sergey Kiselev's 8088_bios
  (GPL-3.0-or-later; `MACHINE_UNEMU_AT`) with Nix from the sibling
  `unemu-at-bios` repository, as for vgabios, checks the ROM's SHA-256
  (`f8ecd4a2…c8db6`) and writes its source archive
  `unemu-at-bios-1.0.2-unemu.2-src.tar` (`6c7aaace…a47e`, the tree without
  upstream's prebuilt `binaries/` and `xtide_r625/`), after rebuilding the ROM
  from the staged tree's own flake. **MEASURED on aarch64-darwin:** two origin
  runs and one local-checkout run give the same ROM and archive. The CI
  `fixtures` gate runs it before XTIDE. Catalogue entry
  `unemu-at-bios-1.0.2-unemu.2` (open, **Tested**; `ibm-5170`; floppy; 8042,
  RTC/CMOS, cascaded PICs; 80286) is now the system BIOS of
  `machines/at-games-open.toml`, `machines/at-ide.toml` and the templates.
  READ (the fork's `README.unemu.md`): CMOS shutdown-byte resume (05h/0Ah
  through 0040:0067h, 09h, 04h via POST), INT 15h AH=87h/88h as IBM, extended
  memory measured into CMOS 30h–31h (one more 8042 CPU reset per POST), IBM
  CMOS checks (161–164, checksum over 10h–2Dh), A20 off after POST, IBM
  display-switch and port 70h NMI polarity, second 8237 initialised, 80286 and
  80287 identified, banner "unemu AT BIOS, Version 1.0.2-unemu.2". Because of
  the IBM switch polarity `at-ide.toml` now sets `display_switch = "mono"` to
  keep its MDA console (derived CMOS now declares monochrome); at-games-open
  keeps `colour` for its VGA. **MEASURED at CPU `060d446a`:** at-pm-test on
  the open profile now passes everything: `int15 88h ok`, `int15 87h ok`, all
  `kbc`/`tf` lines resumed, `int15 87h fault ah 02 info 0000`, `failures 0`,
  6,589,934 retired (was `failures 34`, 211,167,574 retired, a reboot on every
  reset). at-games-open POST shows the new banner and "Intel 80286", and
  FreeDOS reaches `A:\>` with the retired count and text hash unchanged
  (`15,241,264`): the goldens that stop at a virtual-time poll keep their
  counts. ASSUMED explanation: virtual time is retired instructions plus
  halted PIT time, and the BIOS's extra POST work only shortens its own
  busy-wait delays. at-ide keeps 15,775,575 / 19,746,105 retired and its
  logical hashes and POST codes; its screen hashes move with the time FreeDOS
  shows at the prompt, `Current time is 12:00:18.78 am` with the Kiselev
  AT-reset BIOS and `Current time is 12:00:18.83 am` with the open AT BIOS
  (MEASURED; prompt `b7d8e6cd…` -> `a0e8f388…`, final `a045a62f…` ->
  `7aae65a7…`). The INT 10h probe (10,240,119) and the mode 0Dh/13h scanout
  (10,730,617) halt slightly later with the same screen and frames. The EMS,
  journal, FreeDOS install and media goldens pass unchanged. No golden waits at
  F1: the derived CMOS of every open profile passes the new checks. The
  templates change their system BIOS package, so `pc_image::template::VERSION`
  is `1.1.0` with new pinned digests. The Kiselev entries stay: the unmodified
  Xi 8088 image boots `machines/at-freedos.toml` and its BIOS POST, FreeDOS
  and DOS goldens; no machine uses the AT-reset build any more. See
  [the fixture notes](../../vendor/unemu-at-bios/README.md) and
  [open AT BIOS candidates](design/firmware-and-machine-classes.md#24-open-at-bios-candidates).
- **2026-11-15 — Set-level metadata on `image media import`:** `--publisher P`,
  `--developer D` and `--genre G` (repeatable, order kept), `--year YYYY`,
  `--platform PLATFORM` (the software platform, one of the library dialog's
  choices: `DOS`, `Windows 3.x`, … `other`) and `--description TEXT` fill the
  set's one §2.1 title record, the same fields the Import installation media…
  dialog writes (plus developers, which the dialog has no field for). The year
  (1 to 9999) and platform rules now live in `pc_image::media_import`, so the
  dialog's worker and the command line refuse the same values with the same
  message; the dialog keeps its own inline year hint. Values are trimmed and
  blank ones refused. The preview and `image media show` print one line per
  stated value. The metadata stays generated content: CC0-1.0 in an open set.
  Without the flags the set digest is unchanged (a CLI test pins the default
  set's digest, measured before the flags existed, equal to the shared
  import's pin). The shareware import examples (`commands.md`, each
  `vendor/shareware/*/README.md`) now name Apogee Software as publisher, the
  developer (id Software for Keen 4 and Wolfenstein 3-D, Apogee Software for
  Duke Nukem II), the year (1991, 1992, 1993), the genre and `DOS`. The
  metadata changes those sets' digests, and the command line never moves an
  existing tag, so a store that already holds the tags needs new ones or a
  fresh store.
- **2026-11-15 — Registry privacy per namespace:** one registry can serve a
  public `unemu/` and a private `private/`, as [decision-25 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-25%20-%20Machine%20images%20are%20self-contained%20OCI%20artifacts%20in%20a%20local%20store.md)'s amendment "privacy
  per namespace" and the approved registry-accounts mock-up describe.
  - **Preferences:** a registry entry either keeps one `private` flag for the
    whole registry (older entries read and write unchanged) or lists
    `namespaces = [{ name = "unemu", private = false }, { name = "private",
    private = true }]`; an entry with namespaces may not also say
    `private = true`. `unemu/` is never private: an entry that marks it so is
    refused when preferences are read, with an error saying why.
  - **Guard:** user-supplied blobs go only into a namespace marked private, or
    to an entry without namespaces marked private as a whole; `unemu/` takes
    open content only on every host; the anonymous-read probe still runs on
    the target repository. An entry that lists namespaces refuses every push
    outside them ("X/ is not a configured namespace on this registry"), open
    content included, as the mock-up does. A repository is in a namespace only
    below it. The other guard rules are unchanged.
  - **Command line:** `image registry set HOST [--private | --namespace
    NAME[:private|:public]...]` writes an entry's privacy under the
    preferences lock and keeps its user and every other key. `image push
    --dry-run` prints a `scope:` line.
  - **Desktop shell:** Preferences › Registries lists a child row per
    namespace with "Per namespace" in the registry's Private column, the
    details pane lists the namespaces, and Add/Edit has the Namespaces field
    (empty = the whole registry; `NAME:public` or `NAME:private` sets one
    namespace on its own, `unemu` stays public). Making a private namespace
    public asks for confirmation, and a dialog opened before another writer
    changed the namespaces is refused. Push… names the namespace in its first
    check and gives the unlisted-namespace refusal; Pull… shows the same line
    under the resolved reference. Every rule comes from the registry crate.
  - **Tests:** preferences round trip of both forms, the guard per namespace
    (private allowed, `unemu/` and unlisted refused, a private namespace that
    answers anonymous reads refused through the mock registry, whole-registry
    privacy unchanged, `unemu/` marked private refused), `image registry set`
    and the dry run's namespace, and headless presenter and rendering tests
    of the rows, the Add/Edit field, the confirmation and Push…/Pull….
  - **Manual checks (maintainer):** configure `registry.unemu.dev` with
    `unemu/` public and `private/` private, then push a user-supplied image to
    `private/` and see the refusal for `unemu/` and for another namespace.
  - **Mock-up departures, not yet reviewed by the maintainer:**
    - The Namespaces field also takes `NAME:public` and `NAME:private`, so
      Edit can show and keep an entry whose namespaces differ; the mock-up
      gives every listed namespace the one Private registry checkbox.
    - Child rows derive their description from the mark (official,
      proprietary or public) instead of a free-text label.
    - `unemu/` takes open content only on every host, as the guard already
      did; the mock-up applies that rule to `registry.unemu.dev` only.
    - Pull… shows the namespace line; the mock-up has no Pull… dialog.
- **2026-11-15 — Redistributable installation media sets:** `image media
  import … --open --license LicenseRef-NAME --license-text EXTRA` states that a
  set may be redistributed under the vendor's own terms ([decision-26 (unemu)](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/decisions/decision-26%20-%20Installation%20media%20sets%20are%20their%20own%20OCI%20artifact%20type.md)
  amendment). `--open` needs both flags; the licence must be one SPDX
  `LicenseRef-` and the terms text exactly one of the set's `--extra` files,
  so nothing is guessed. An open set marks every medium and extra open under
  that licence, each linking the terms text's decoded digest in the new
  `unemu.terms` annotation (the counterpart of `unemu.source`); the generated
  config and metadata are open under CC0-1.0 and the manifest carries
  `org.opencontainers.image.licenses`. Without the statement a set is
  user-supplied exactly as before (a test pins a default set's manifest
  digest, measured with the earlier import). Re-importing the same bytes
  under the same tag with other marks is a tag conflict, so a set is opened
  under a new tag; the refusal hints say so, and say that a user-supplied
  record in another local tag is cleared only by deleting that tag or pushing
  from a store without it. Push-guard rule 5 lets an open set into `unemu/`
  or onto a registry not marked private only when every open medium and extra
  links one terms text that is an open extra of the same closure; a missing or
  mismatched link, or an absent terms text, is refused with the layers named,
  and a private registry outside `unemu/` keeps the user's own copy. Rights
  follow the bytes: a machine image with an open set's media stays private
  while any other layer is user-supplied (machine import still marks every
  floppy user-supplied), and another local tag recording one of the set's
  disks user-supplied keeps the set private. `image media show` prints the
  terms extra, each layer's class, licence and link, and the guard's verdict.
  Synthetic floppies and terms texts in the tests. **MEASURED:** the three
  `vendor/shareware/` sets, imported with their statements (entry below)
  into a scratch store, pass the guard for `unemu/` in `image push
  --dry-run`. The GUI for open sets is separate work; set-level metadata
  followed (entry above).
- **2026-11-15 — Open VGA BIOS re-pinned to `v0.8a-unemu.2`:**
  `vendor/vgabios/fetch.sh` builds fork release `v0.8a-unemu.2` (revision
  `d883bbac`), which drops upstream's prebuilt `VGABIOS-lgpl-latest*.bin`
  images and a developer's personal git hooks, and preprocesses with the same
  gcc 14.3.0 on every system. **MEASURED on aarch64-darwin:** the ROM and probe
  bytes are unchanged (`7570ef9c…5389`, `21b9dd1d…eb58`), so the catalogue id
  `vgabios-0.8a-unemu.1-std`, the machine files and the goldens stay. The source
  archive is now `vgabios-0.8a-unemu.2-src.tar` (SHA-256 `6304d0c2…d1d9`); it
  leaves out only `.beads/config.yaml`, whose sync remote names a developer's
  own forge. Origin and local-checkout builds write the same archive, and the
  extracted archive rebuilds both pinned hashes. The vgabios firmware package
  and both template machines change, so `pc_image::template::VERSION` is
  `1.0.1` with new pinned digests; the Kiselev and XTIDE packages keep theirs.
- **2026-11-15 — Shareware installation media (local only):**
  - **`vendor/shareware/{keen4,wolf3d,duke2}/fetch.sh`** build installation
    floppies from the original Apogee shareware archives: Commander Keen 4
    1.4 (`4keen.zip`), Wolfenstein 3-D episode 1 1.4 (`1wolf14.zip`) and
    Duke Nukem II episode 1 1.0 (`4duke.zip`). Pinned URL (FUNET's mirror of
    Apogee's file area) with a byte-identical 1994 CD-ROM copy as fallback,
    archive SHA-256 and member list checked, offline once cached or with
    `SHAREWARE_ARCHIVE`. Each image is a 1.44M FAT12 floppy with every
    archive member unmodified, the archive's DOS timestamps, a fixed label
    and serial and the project's non-bootable volume sector; its SHA-256 is
    pinned (measured on macOS). Two runs (download, then cached) give
    identical images. The vendor and licence texts go to `extras/`; each
    README summarises them. Everything under `vendor/shareware/` except the
    scripts and READMEs is gitignored; the scripts are not in CI.
  - **Media sets:** `image media import` of each floppy with its extras and
    the licence statement (`commands.md`, each README) gives an open set:
    Wolfenstein 3-D under `LicenseRef-apogee-vendor-v11.01.92` with
    `VENDOR.DOC`, Duke Nukem II under `LicenseRef-apogee-license-v12.01.93`
    with `LICENSE.DOC`, Commander Keen 4 under
    `LicenseRef-apogee-readme-keen4-v1.4` with `README.DOC` (its only
    distribution text, without a revision mark). Imported without the
    statement a set stays user-supplied. Since the set-level metadata entry
    above, the imports also record publisher, developer, year, genre and the
    DOS software platform.
  - **Publication:** the maintainer approved on 2026-11-15 that these public
    shareware sets may be published. Imported with the statement they are
    open, and the push guard lets them into `unemu/` with their terms texts
    (entry above); pushing them to the registry is the maintainer's step.
- **2026-11-15 — INT 15h AH=88h and AH=87h on the AT test floppy:** before
  its rounds, `vendor/at-pm-test` now calls INT 15h AH=88h (AX must equal
  CMOS 31h:30h) and AH=87h through a caller GDT: 128 words to `10C000h`,
  read back directly at `FFFF:C010h`, back to a second buffer, with a guard
  at the A20 alias. Each AH=87h call starts with A20 off (so the BIOS must
  turn it on; the guard catches one that does not), IF set and CMOS 0Fh
  cleared, and checks AH=00h, CF=0, ZF=1, IF=1, the other registers, DS/ES
  and SS:SP, A20 off afterwards (8042 output port bit 1) and CMOS 0Fh =
  00h. After the rounds, a move from a limit-0 source, outside the
  documented entry contract, prints its status and the differences from
  the IBM result (02h, CF=1, ZF=0) on an informational line that is never
  counted. The image SHA-256 is re-pinned. **MEASURED (open golden):** the
  Kiselev BIOS has neither function (AH=86h, CF=1): `int15 88h FAIL 0003`,
  `int15 87h FAIL 0607`, pinned as expected to fail, and
  `int15 87h fault ah 86 info 0005`; `failures 34`, 211,167,574 retired
  instructions. The emulator's port 80h now reads back its last write, as
  the AT's 74LS612 page latch does, also across an 8042 processor reset
  (default-suite test); the IBM block move reads its status there after the
  reset and saw FFh before. **MEASURED:** the IBM 5170 user-ROM golden
  gives `int15 88h ok`, `int15 87h ok`, `int15 87h fault ah 02 info 0000`,
  `failures 0`, 31,955,711 retired instructions (pinned). See
  [`design/at-pm-test.md`](design/at-pm-test.md#int-15h-ah88h-and-ah87h).
- **2026-11-15 — Official template images and File › Get templates…:**
  - **`image template build --store DIR [--vendor DIR] [--dry-run]`** builds
    the open firmware packages (`unemu/firmware/<catalogue-id>`: Kiselev
    AT-reset BIOS, unemu VGA BIOS, XTIDE r638, each with the source archive the
    catalogue pins) and two template machines created through the New machine
    create API, never from machine files: `open-at` (IBM 5170-class, the three
    ROMs, 640 KiB + 1 MiB, VGA, one empty 1.2M drive, a blank zero-base 504 MiB
    ATA disk, no sound) and `open-at-games` (plus SB16, COM1 mouse, Lo-tech EMS,
    as `machines/at-games-open.toml` without media). Tags `1.0.1` (`1.0.0`
    before the VGA BIOS re-pin, above) and `latest`;
    no operating system inside. Same fixtures, same digests on every run and
    host (pinned by a test). The push guard for `unemu/` judges each manifest
    before its tags are written, only the judged manifest is tagged, and a
    refusal stops the build. `latest` moves only away from lower versions of
    its repository. `--dry-run` builds in a scratch store, judges with the
    marks of `--store` too, and reports what a build would do to its tags.
    The build never pushes; commands.md lists the maintainer's push commands,
    dry runs first.
  - **File › Get templates…** follows the approved get-templates mock-up;
    the behaviour is described once, in
    [`design/machine-images.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md) §5.5. In
    short: the library offers it while it holds no machine image (firmware
    packages and media sets do not count); it lists the machine images under
    `unemu/` of the default registry and pulls the chosen ones one after
    another by digest on the library worker, under one progress bar that
    counts blobs already in the store as done. Cancel stops a queued pull
    before it contacts the registry. An unreachable registry, a refused or
    missing login and a registry without a catalog each end the listing with
    their own message; a login failure offers Log in…, or Add registry… when
    the registry has no entry, and the dialog lists again once Preferences
    closes.
  - **Code:** `pc_image::template` (definitions, the build with a guard
    callback, tag planning for dry runs, version precedence),
    `firmware_import::publish_as` and `create::create_judged` (only the judged
    manifest is tagged), `NewImage::description` (manifest description),
    `cli::build_templates`, `cli::registry_probe_client`; `templates.rs` in the
    window crate.
  - **Tests:** two builds from different fixture copies give identical
    manifests and digests, and a test pins the published digests; a rebuild
    keeps every tag; the guard sees each image before its tags and a refusal
    tags nothing of it; only the judged manifest is tagged; a store recording
    the open BIOS as user-supplied refuses the build and its dry run; each tag
    passes the `unemu/` push plan with every blob open; a version tag naming
    another image refuses the build; `latest` stays on a higher version and on
    a foreign image and moves from a lower one; the dry run reports conflict,
    moves and kept; the templates validate and lower with their cards; both
    templates POST with the open BIOS (checkpoint stream through the XTIDE
    scan) and XTIDE finds their blank disk. Get templates… against the mock
    registry lists only machines under `unemu/` (the version `latest` names,
    releases before pre-releases; firmware, test and broken repositories
    skipped, and a repository this account may not read), marks templates in
    the library and local tags naming another image, and pulls the chosen ones
    by digest in turn under one progress whose second template starts with the
    shared firmware counted; Cancel stops a queued pull before any request,
    the summary counts what was added and the status bar stays quiet; an
    unreachable registry, a missing or refused login and a registry without a
    catalog end with their messages; Log in… routes to that registry's Log in
    dialog, a registry without an entry gets Add registry… with no other row
    selected, and the dialog lists again with the stored login, which goes
    with the catalog request; closing the dialog stops the listing before the
    next repository; every dialog stage renders headless, including the
    dimmed rows and the click anywhere on a row; and a library with only
    firmware and media offers the command in place of the empty table.
  - **Mock-up departures, awaiting maintainer approval:**
    - No "Writing tag" stage: a pull reports only blobs arriving and
      verified, so its row reads Pulling until the pull answers.
    - The dialog has no title-bar ×; Close, Cancel and Escape do its work.
    - No default-button emphasis on the empty state's Get templates… or on
      the dialog's Get, Try again and Close: egui buttons have no default
      style.
    - The empty state has no illustration. While machine files are listed,
      it sits above the table instead of replacing it, so those files stay
      reachable.
    - A login failure for a registry with no Preferences entry (a default
      set by `UNEMU_REGISTRY` or a bare `default_registry`) offers Add
      registry…, opening Preferences on the Add dialog prefilled with the
      host and no row selected, instead of the mock-up's Log in….
  - **Not done here:** publishing the templates to `registry.unemu.dev`
    (maintainer).
- **2026-11-15 — Image › Push… and Image › Pull…:**
  - **Push…** works on any local image: machine image, media set or firmware
    package. It opens on the selected image, or the Push… button of the
    selected image.
    - The destination resolves Docker-style against the default registry
      (preferences, overridden by `UNEMU_REGISTRY` as on the command line; an
      invalid one stops both dialogs with its error). One-click destinations:
      the local name, `private/` on the default registry and each other
      configured registry.
    - The dry run is the command line's plan (`cli::push_plan`): local marks
      plus the firmware catalogue, built from one reading of the tag. It lists
      every blob with its type, size, effective class and where that class was
      recorded (this image, local tag NAME or the firmware catalogue). The
      guard's checks and verdict follow the mock-up, with no override. The tag
      grammar and digest destinations are refused before the guard runs.
    - The push carries the reviewed image digest. When the local tag names
      another image by then, nothing is sent and the dry run is built again
      with a notice.
    - A non-version tag moves only after a confirmation that shows the old
      and the new digest. The move applies only while the registry's tag
      still names the digest shown; otherwise the dialog asks again.
    - Progress is per blob (uploaded, already there, mounted) and per
      accepted chunk ("chunk N of M · bytes"), then the manifest and the tag.
  - **Pull…** takes a reference and an optional local tag (default: the short
    form). It shows bytes per downloading blob and each verified blob, and
    resumes an interrupted blob by range. A tag conflict, an unknown reference
    or a digest mismatch shows in the dialog.
  - **Threads and errors:** both dialogs run on the library worker, never on
    the frame loop, and are disabled while a machine runs. Progress is shared
    state with a repaint request, so a slow frame never stalls a transfer;
    only the final answer goes through the worker's queue. Logins come from
    the account store through the command line's client
    (`cli::registry_client`). Auth, refusal and network errors show in the
    dialog; no secret is ever shown.
  - **Cancel** is checked before each blob, between the chunks of a large
    upload and before publication; a single upload request of up to 64 MiB
    finishes first. A cancelled push never publishes its manifest and keeps
    accepted chunks for the next push. A cancelled pull never writes the local
    tag; the next pull fetches only the missing blobs. A pull cancelled while
    its job still waits behind another worker job contacts no registry and
    reads "Cancelled before it started; nothing was fetched." Escape and
    closing the dialog cancel too. Quitting while a transfer runs asks "A push is running.
    Cancel it and quit?" first; a transfer whose dialog closed reports its
    cancel in the status bar.
  - **Registry crate:** gains `push_cancellable` (blob and chunk progress),
    `pull_cancellable` (received bytes and verified blobs), and
    `PushOptions::move_from`. The mock registry can hold one request until a
    test releases it. The CLI and the shell share the providers list, the
    push plan and the client (`cli::with_image_providers`, `cli::push_plan`,
    `cli::registry_client`).
  - **Tests:** headless tests against the in-process mock registry cover an
    allowed push (progress shown while the manifest is held, manifest last),
    every guard refusal including a forged job and a registry that answers
    anonymous reads, the local-tag and catalogue sources of a class (a package
    marking a catalogued proprietary ROM open cannot leave), a tag moved since
    the dry run, tag-move confirmation with a race, version-tag refusal, a pull
    resumed by range, cancel during push and during pull, closing the dialog
    and shutting down during a push, the quit question, the default-registry
    override and errors, chunk status text, rendering of every stage, and that
    registry requests and secret reads never run on the frame loop.
  - **Manual checks (maintainer):** a real push of a user-supplied image to
    `registry.unemu.dev/private/`, a refused push of it to `unemu/`, a pull of
    it into a fresh store, Cancel during a multi-chunk upload followed by a
    resumed push, quitting during a push, and the native look in both skins.
  - **Mock-up departures, accepted for now by the maintainer on 2026-11-15:**
    - The dry run has no "On the registry" column, because whether a blob is
      present is known only while pushing; the progress list shows it. A
      "Recorded in" column shows where each class comes from.
    - No Pause: Cancel stops at the next blob or chunk, and the next push
      resumes the accepted chunks.
    - Chunk progress reads "chunk N of M · sent of size" instead of "chunk N
      of M · K accepted".
    - The login line only informs: there is no Log in… inside the dialog,
      and Push stays enabled without a recorded login. A refusal from the
      registry points to Preferences › Registries.
    - Pull… had no mock-up of its own (the mock-up shows it disabled); its
      layout follows Push….
- **2026-11-15 — Settings on an image and Save as new image…:** the library's Settings… (and
  Machine › Settings… with an image selected) opens a property sheet over the stopped image's
  flattened hardware; a running image opens read-only. Every option and reason comes from the
  capabilities API; each edit is written into the hardware tables and described again. Firmware
  stays as stored; diskette drive B: can be installed with an empty catalogue, a drive whose
  catalogue holds media keeps its type and stays, and fixed disks are not added or removed here.
  Review changes… shows the flattened configuration diff, the CMOS bytes recomputed as SETUP would
  (only the fields whose derived value changes — 10h, 12h/19h–1Ah, the 14h diskette, coprocessor
  and display bits, 15h–18h, 30h–31h — then the 2Eh–2Fh checksum over 10h–2Dh; clock and other
  options kept; images without a stored CMOS list the bytes Start will derive), every binding, the
  old and new machine digests, and the name and tag. Publish writes a configuration blob, a CMOS
  blob when one changes, and a manifest that binds every disk, firmware, catalogue and metadata
  blob of the source again; the source and its tags are untouched. One **Save as new image…**
  dialog (repository, OCI-grammar tag, title; flatten for New image from here…) serves Disk
  layers' Save as layer…, New image from here… and Keep as new image…, the new Machine › Save as
  new image…, and the revision review. The tag defaults to the next minor version after the
  source's version tag (else the repository's newest, else 1.0.0); taken version tags are refused
  with Use <next>, other tags move only after Move tag? and then compare-and-swap from the shown
  digest; a refused save re-reads the store's tags and asks again (Move tag? with the current
  digest, or the version refusal for a tag that appeared meanwhile). The store refuses to move a
  version tag on every tag update (create, save, revision, pull) and keeps a record of deleted
  version tags (`local/retired-version-tags.toml`), so a deleted version tag can later name only
  the digest it named before. Publication keeps the source manifest's annotations (title,
  `unemu.firmware.sources`, others; per-build ones such as `created` are dropped), may retitle,
  and records `org.opencontainers.image.base.digest`; a save that changes nothing produces the
  source manifest itself, so the new tag aliases the same digest. An interrupted save is offered
  in Disk layers and Save as new image… as "Finish saving <tag>", which repeats it with its own
  tag and expected digest. The library worker reads every image, tag and plan for these dialogs;
  the UI thread opens no store for them. A stored CMOS image that SETUP values cannot be derived
  for (an unlisted fixed-disk geometry) is kept byte for byte, and the review says so. Revision
  publication still verifies every blob of the closure once, like every publication.
  **MEASURED:** headless model, dialog and worker tests; pc-image tests publish memory, drive,
  display and coprocessor revisions of a stored-CMOS image and compare every CMOS byte. Manual checks still open: the look of the sheet, the review and both tag
  questions against the mock-up in both skins and themes; keyboard focus and Escape in the
  stacked dialogs; Machine › Save as new image… on a running machine (it stops first, then the
  workspace runs the new image); a revision that adds drive B: boots and the BIOS sees the
  drive; a CMOS-revision image boots without a CMOS checksum error. Firmware changes, a new fixed
  disk D:, catalogue editing and the "Keep the inserted floppies" choice remain open.
- **2026-11-15 — Library Installation media collection and Import installation media…:** the
  library tree has an Installation media node with the set count and Format (floppy sizes,
  optical), Labels, Genre, Platform and Year facets; its virtualized rows show the set with its
  disks ("4 x 1.44M"), tag, platform, extras count, stored/decoded size, redistribution, labels and
  availability, with title search (disk labels and extra names are searchable too) and its own
  sort. Everything comes from summaries, which now carry the set's ordered media (label, format,
  decoded digest) and extras; older cached rows rebuild once. The set pane lists the disks in
  catalogue order with format, geometry and short SHA-256, and the extras with kind, size, DOS
  name and redistribution. Insert opens the running machine's matching drive's From library…
  view with the set chosen, so insertion uses the existing verified path; optical, mixed-format
  and mismatched sets show the reason. The shell draws the library only while no machine runs,
  so in the app Insert currently always shows "No machine is running…"; the routing is covered by
  a headless shell test with a running presenter. Add to machine…, Make transfer media…, Export
  and the per-extra Open/Export… are disabled with "not available yet" reasons; sets have no
  Start. File › Import installation media… (and the collection's toolbar) picks files or a
  folder on the host and lists them on the library worker (a folder by the CLI's rule: hidden
  files skipped, any other non-candidate refuses), reviews order (up/down), labels, the detected
  format with an override, extras (kind, optional DOS 8.3 name, remove), title, tag (derived
  from the title until edited), software platform, publisher, year, genre, labels and
  description, then imports on the worker through `pc_image::media_import`, the code path that
  `image media import` now uses too. Progress counts files; Cancel import stops before the next
  file and writes no tag. Refusals read as the CLI prints them; re-importing the same review is
  "Already imported" without writing, and a different set under an existing tag is a tag
  conflict. A missing or stopped worker fails the dialog, which can then be closed. The command
  line still refuses any existing tag before reading sources. Headless tests cover rows, facets,
  search with hidden blobs, the pane, availability and Insert routing, the dialog model,
  the worker import, refusals, cancel, re-import and worker failure. Manual checks: import a real
  multi-disk folder and a file list with an odd-sized image (format override), add a PDF and a
  zip as extras, re-import the same review, cancel a large import mid-way, browse the facets and
  the pane at the default window size in both skins, and confirm Start stays disabled with a set
  selected.
- **2026-11-15 — New machine… wizard and zero bases:** File › New machine… (⌘N/Ctrl+N) opens a
  seven-step wizard that builds a new empty image in the local store, never a machine file:
  machine class (the closed list of seven; only AT 286 is selectable), motherboard with its
  system BIOS picked from stored firmware packages whose fit metadata names the board (incompatible
  packages under "Show incompatible" with reasons; an empty state offers the library's Import ROM folder…),
  CPU and memory, video (video BIOS per card) / sound / COM1 / EMS, drives (floppies, the disk
  adapter's option ROM such as XTIDE, blank C:/D:), name and tag, summary. Options, limits and
  reasons come from the capabilities API over the draft machine document; video BIOS and option ROM
  fit comes from the catalogue entries the packages name, and a package without a catalogue id never
  fits. Tags follow the OCI grammar; version tags are refused when taken, other tags move only after
  a confirmation (compare-and-swap). The generated configuration and metadata are open; a
  user-supplied ROM makes the image private and the wizard says so. Create binds the package ROM
  blobs by digest and gives every fixed disk a **zero base** (`application/vnd.unemu.disk.zero.v1+toml`):
  a descriptor of a few hundred bytes whose identity is the SHA-256 of that many zeros. Zero-rooted
  heads are read straight from their chain, never materialized; saves seal only written sectors and
  flatten keeps one sparse layer. **MEASURED:** a 504 MiB C: allocates under 2 MiB and writes no file
  over 16 KiB from Create through Start; a created open image (Kiselev AT BIOS, open VGA BIOS, XTIDE)
  lists in the library, validates and POSTs through checkpoints 01h–08h. COM2, registry browsing for
  packages (Get from registry…), the sample boards and speeds of the mock-up beyond the IBM 5170 at
  6/8 MHz, and native visual acceptance remain open. Save as new image… keeps the manifest
  title annotation and `unemu.firmware.sources` (see the Settings on an image entry). Generated
  configurations and metadata are open in import, Create and publication alike; ROMs, disks,
  floppies and CMOS stay user-supplied.
- **2026-11-15 — Status-bar drive chips:** while a machine runs, the status
  bar shows one chip per drive it has, as in the approved
  [main-window mock-up](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/mockups/main-window.html) (revision 2):
  `A: FreeDOS 1.2 boot · 1.2M · read-only`, `B: empty`,
  `C: open-at-dos:1.0.0 · journal`, each with an amber activity light.
  - **Hover and click:** hover shows the drive, the full image name, its source
    (machine file or image, library media set with disk position, or a file
    chosen with Browse…), a short SHA-256 (or "not pinned"), the format and the
    write mode. A click opens that drive's menu: Eject (disabled while the
    drive is empty), Insert…, From library… and the write mode, which for a
    session disk changes through its Insert dialog with the disk preselected; a
    fixed disk offers Disk layers… and says it is part of the image or the
    machine. The menu items are the Media menu's own commands, and the Media
    menu now names each drive's image and write mode too and disables Eject
    for an empty drive.
  - **Lights:** they read each drive's own busy state at every completed run
    boundary — the µPD765's selected, motor-on drive or a pending DMA transfer
    (so a BIOS motor-off delay keeps A: lit for a moment after an access), and
    the ATA disk's BSY or DRQ — through two new read-only device accessors; a
    200 ms hold in host UI time keeps a short access visible and never reaches
    the machine. A fast test runs a guest repeating multi-sector IDE reads and
    floppy DMA reads through the sampling presenter and replays it headlessly
    in 137- and 997-tick chunks with identical guest and device state. Any IDE
    command whose data phase starts and ends between two 1 ms boundaries goes
    unseen; writes have no BSY interval in this model.
  - **Layout:** cell widths depend only on the window width, so chips
    appearing or changing never move Power, CPU, EMS, audio or speed. CPU, EMS
    and speed are narrower (100, 92 and 124 px) to fit their usual text, which
    gives the drive area 298 px at the default 1040 px window (80 px at the
    760 px minimum, at most 900 px) while the message keeps 240 px. A chip
    shortens only its image name, so an A: and a C: chip keep format and write
    mode at the default window; with three drives there the whole line can
    still be cut. The hover always has the full text.
  - **Tests:** headless tests cover chip text per drive state, tooltip rows,
    the click menu and its commands, Media menu labels, the layout, default
    width text, and the light's on/off transitions with the hold.
  - **Manual checks:** look at the chips, tooltip and drive menu in both skins
    and in light and dark themes; boot FreeDOS from a real floppy image and
    watch A:'s light while it reads; watch C: during a DOS directory listing on
    an IDE machine; open a library media set from a chip and switch a browsed
    floppy between read-only and journal from its chip.
  - **Mock-up departures, accepted for now by the maintainer on 2026-11-15:**
    - C:'s write mode is a label in its drive menu, not a choice between
      Journal and Read-only.
    - An empty floppy drive's menu has no "Write mode for the next floppy"
      section; the mode is chosen in the Insert dialog.
    - The light is steady while the drive is busy, not flickering.
- **2026-11-15 — Code runs from the EMS frame:** a Lo-tech EMS window whose
  latch selects a populated page that no other latch selects is held as CPU
  RAM, so overlay managers and EMS-aware programs can execute from mapped
  pages; the CPU core is unchanged. After every guest port write, before that
  instruction's DMA and the next instruction, the assembly-side frame map
  writes held windows back into SRAM, retiles the frame's CPU regions and
  loads newly held pages through the self-modifying-code-safe write path.
  Unbacked windows are CPU ROM of FFh: they read and execute FFh and drop
  guest and DMA writes, and a word straddling a held and an unbacked window
  reads FFh for the unbacked byte. Pages selected by two latches stay trapped
  on the board's coherent data path, without fetch. Operands and instructions
  crossing two held windows work. Known limit: an operand crossing a trapped
  region (a shared window, or another card's window beside the frame) and a
  held or unbacked window stops the run with an unmapped-access CPU error,
  because the core does not split one operand between RAM or ROM and a
  trapped region. Held pages are written back whenever a run returns, so
  device snapshots see current SRAM; a restore drops them and reloads. ISA DMA
  in both directions uses the held CPU copy. Run chunking leaves registers,
  time, device state and traces unchanged (synthetic rigs). MEASURED: the
  write-back costs under 1 µs per 1 ms run chunk with four held windows (see
  architecture). EMSCHECK in the open FreeDOS MEM golden now also copies a
  routine into an EMS page, moves the page to another window and far-calls it
  there. MEASURED: the previous EMSCHECK keeps its endpoint (18,681,813) under
  the new map; the extended program re-pins it to 18,681,761 with unchanged
  text `0eda968da7052433`, and the page-alias mutant moves by the same amount
  (16,355,236, `df1f6dc3129c84ce`; the mutant wraps the assembled board, so
  its aliased latches drive the frame map). Unbacked windows as ROM leave both
  endpoints unchanged. Skipping the write-back fails eight synthetic rigs and
  the golden's DOS page check. See
  [architecture](architecture.md#x86jit-integration).

- **2026-11-14 — macOS application menu:** the global menu bar has one
  application menu: About unemupc98, Hide unemupc98 (⌘H), Hide Others (⌥⌘H),
  Show All and Quit unemupc98 (⌘Q). About opens the shell's About dialog; Quit
  runs the shell's Exit, never NSApplication `terminate:`. Both bring a
  minimized window forward; an open shell dialog holds back About, never Quit.
  AppKit takes ⌘H, ⌥⌘H and ⌘Q as menu key equivalents, so each acts once and
  the letter never reaches the guest; the shell's own ⌘H handling is gone. An
  Option pressed while Command is held no longer reaches the guest either, so
  ⌥⌘H sends no Alt tap. Dock › Quit, logout, restart and shutdown answer
  "terminate later" through a method added to winit's application delegate, run
  the same Exit and reply once the shell decides: at once with no machine
  running, YES after Stop machine and persistence, NO on Cancel. A stop whose
  persistence fails replies NO and keeps the error dialog up; File › Exit still
  closes. While an application-modal panel (a file dialog) is open, or when the
  shell cannot take the request, the system quit is cancelled. The stop prompt
  and Gaming mode's Keep/Discard question bring a hidden or minimized window
  forward. AppKit ends the process on YES, so a stop error is printed to stderr
  and the exit status is 0. If a later winit answers the quit question itself,
  startup prints a warning and Dock › Quit quits at once again. Headless tests
  cover the menu model, the reply state machine and the shell paths. The native
  menu needs manual checks:
  1. the menu's items and shortcuts (its bold title is the process name for a
     bare binary);
  2. About, also from a minimized window;
  3. ⌘H, ⌥⌘H and Show All hide or show once, typing nothing, not even an Alt
     tap, in a focused guest;
  4. ⌘Q and Quit with and without a running machine (Cancel resumes, Stop
     machine quits);
  5. Dock › Quit with and without a running machine, also after ⌘H and from a
     minimized window;
  6. logout with a running machine (Cancel makes macOS report the cancelled
     logout, Stop machine lets it continue) and with none (no prompt);
  7. Dock › Quit during Gaming mode, also hidden or minimized, keeps its
     Keep/Discard question;
  8. Dock › Quit twice while the prompt is up, then Cancel: the application
     stays responsive and a later quit still works;
  9. Dock › Quit while a file dialog (Browse…) is open is cancelled, and works
     again once the dialog is closed.
- **2026-11-15 — Firmware licences and metadata in the library window**
  (mock-up revision 3): Import ROM folder… has a Redistribution choice per set
  (catalogue-open preset open, proprietary fixed user-supplied, others open
  only with a named SPDX licence, free expressions checked by the parser),
  Source archive… with the user's statement that it is the corresponding
  source and the archive's own licence statement, and a per-set details editor
  (title, description, notes, labels, narrowing-only catalogue fits,
  provenance, layers and licences, editable README preview). A set whose tag
  already holds its ROM bytes is already imported, whatever statement made it.
  The package pane shows description, labels, provenance, every layer's
  licence, the guard's Config and Push lines (`guard::reach`, shared with
  `image firmware show`), the source archive, the README with Open README, and
  Edit metadata… (also File › Edit metadata…), which publishes `:N+1` with the
  same ROM blobs. A copyleft open ROM without its source layer gets the warning
  badge in its row and the push-guard notice with Add source archive…, and the
  status line counts such packages as private only. Every verdict, reason and
  README text is the library's preview of the statement, run on the library
  worker; no ROM or archive byte is read in the frame loop, and a stopped
  worker fails the dialogs, which can then be closed.
  - **Library and CLI changes:** a statement's README is Keep, Generate or a
    hand-written text; a hand-written README layer carries
    `unemu.readme = "hand-written"`, and every edit, `image firmware edit`
    included, keeps it unless asked to regenerate (`--regenerate-readme`). Empty
    texts clear fields. A tag holding the same ROM and document bytes is
    already imported (GUI and `image firmware import`); a statement that would
    change it is refused, naming the edit. New helpers: `preview_edit`,
    `generated_readme`, `current_rom_license`, `locked_by`,
    `catalogue_rights`, `catalogue_fit`, `license::identifiers`,
    `guard::reach`; `image_store::firmware::valid_date` is public.
  - **Licence policy:** `license::ACTIVE` is now `Policy::STRICT`, as the
    approved mock-up asks: any copyleft identifier, even after `OR`
    (`MIT OR GPL-2.0-only`), needs the source archive, for import, edit, the
    README and the push guard alike. The maintainer can switch back to
    ACCEPTED in one line.
  - **Mock-up departures, awaiting maintainer approval:**
    1. Source archive… opens the host file picker in the scanned folder; the
       mock-up's in-program dialog listing archive candidates (tars found in
       the scanned folder, a package's own layer) is not built, because the
       scan does not report archives.
    2. The archive's own licence is an explicit Archive licence choice ("Not
       stated" by default, as the CLI's `--source-license`); the mock-up shows
       only the ROM licence for the archive.
    3. Publish new revision is enabled with no field changed when the library
       says the package would change (a package written before the current
       rules gets a new config and README); the mock-up disables it.
    4. The layers table has no document licence choice: an edit keeps the
       documents as they are, and the GUI cannot add documents yet.
    5. Boards and machine classes of an unlisted ROM are added from the
       provider's vocabulary in a list, not typed as free ids.
    6. While a statement is refused and none was accepted yet (a copyleft set
       when the review opens), the layers table says "Checking the layers…"
       and the README editor is empty and disabled; the mock-up always shows
       both.
    7. The Labels facet in the tree is not built.
    8. The already-imported set's details omit its stored redistribution
       ("Open · …, with its source archive"), which the scan does not read.
  Headless tests cover the redistribution rules, SPDX validation, reasons
  worded with the dialog's controls, the details round trip, README equality
  and hand-written README keeping, the rescan after a statement import, Edit
  metadata…'s revision, the badge and notice, and worker failure. Manual
  checks: run Import ROM folder… on a folder with the Kiselev AT-reset BIOS and
  its tar (Import stays disabled with the reason until Source archive… picks
  the tar), an IBM BIOS (Open disabled) and XTIDE; edit fields and the README
  and check the layers table; scan the folder again (all already imported);
  publish Edit metadata… on an imported package and open its README; check
  both skins at the default window size.
- **2026-11-15 — Firmware licences, source archives and descriptive metadata:**
  every firmware package layer carries an SPDX licence (`unemu.license`) next to
  its redistribution mark, and an open mark without a valid licence counts as
  user-supplied everywhere, in the push guard too. Rights follow the bytes: a
  ROM, chip, document or archive the catalogue lists as proprietary by SHA-256
  and length can never be opened, whatever its catalogue id or a hand-built
  candidate says, and `image push` adds those digests to the guard's local
  marks, so a pulled package cannot carry them out. Catalogue-open ROMs take
  the catalogue licence; other ROMs stay user-supplied unless `--open
  --license SPDX` states otherwise. A GPL, LGPL, AGPL or MPL family ROM is open
  only with an open source archive (`--source`, zip or tar, its own `source`
  layer linked from the ROM, tar stored `+zstd` from 4 KiB): the archive the
  catalogue pins (`source_archive_sha256`, from the Kiselev, vgabios and XTIDE
  fetch scripts) is open as is, any other needs `--source-license SPDX`, and a
  catalogue-open copyleft ROM without one is imported user-supplied with a note
  naming its fetch script. `image firmware import --unlisted ID --role ROLE`
  adds a ROM the catalogue does not know (never under a catalogue id). Imports
  record title, description, notes and quirks, sorted labels and fits (boards,
  classes, required device kinds, prefilled from the catalogue) and
  provenance (dumped, downloaded or built from, revision, address, supplied
  date), write a generated UTF-8 Markdown README layer and the standard
  `org.opencontainers.image.*` title, description, vendor, version, source,
  revision and licenses annotations (no creation time). The README and config
  are open (CC0-1.0) only when every other layer is. `image firmware edit`
  publishes a new revision with the same ROM and document blobs; packages
  written before licences stay readable, count as already imported when they
  hold the same bytes, and are user-supplied for the guard where they lack a
  licence. The push guard refuses an open copyleft ROM layer without its
  linked source archive to a registry not marked private or into `unemu/`; an
  all-open package with its source may enter `unemu/`. A machine image whose
  ROM comes from such a package with an open source archive embeds the source
  layer (the same blob) and marks both open; otherwise its ROM stays
  user-supplied. New machine… binds package ROMs the same way, and its
  generated content is open under CC0-1.0, so its preview tells truthfully
  whether the image may enter `unemu/`. Summaries carry licences, labels, description, provenance,
  sources and the README, with a licence facet; `image firmware show` prints
  them with each layer's class and the guard's verdict. Stricter `OR`
  handling is one switch, `image_store::license::ACTIVE` (now `STRICT`, see the
  previous entry). The GUI is the previous entry.
- **2026-11-15 — Library Firmware collection:** the library tree has a
  Firmware node with the package count and Role, Board, Machine class and
  Redistribution facets; its virtualized rows show name, roles, chip count,
  vendor, version/date, fits, catalogue ids, open/user-supplied and size, with
  search and sorting. The package pane shows the chip layout (socket, part
  number, lane, offset, size, short SHA-256), fits, documents and Used by (local
  machine images that embed the ROM blobs, by digest). Everything comes from
  summaries, which now carry the package's vendor/version/date, ROMs and extras;
  older cached rows rebuild once. Import ROM folder… (File, Tools and the
  collection) picks a folder, scans it on the library worker with progress
  (Cancel stops the scan), and reviews complete, already imported, ambiguous
  and refused sets, the count of unmatched files by cause and the unreadable
  files by name. It imports the picks with the CLI's matching and ambiguity
  rule, but refuses per set: a tag conflict or changed source refuses only that
  set, whereas `image firmware import` stops the whole run at a tag conflict.
  Re-import reports "already imported". A missing or stopped worker fails the
  dialog, which can then be closed. Packages have no Start; Use in
  machine… and Export are disabled, documents have no Open, and incomplete
  imports are not stored as records. Used by and the redistribution badge
  compare raw identities, so `+zstd` ROM layers match too. Headless tests cover
  rows, facets, search, the pane, virtualization, the worker import flow,
  refusals, index reuse and worker failure. Manual checks: open Tools ›
  Import ROM folder… on a real ROM folder (progress, Cancel scan, review
  layout, a re-import), browse the Firmware facets and the pane at the default
  window size in both skins, and confirm Start stays disabled with a package
  selected.
- **2026-11-15 — Registry accounts:**
  - **Preferences dialog:** Tools › Preferences… opens a tabbed dialog. General
    links to ROM folders; Registries follows the approved mock-up.
  - **Registries page:** lists the configured registries and the built-in
    `registry.unemu.dev`, each with its user, signed-in state and private flag.
    Actions: Add…, Edit…, Log in…, Log out, Remove, Set as default and Test
    connection.
  - **Validation:**
    - host names need a dot, a port or `localhost`;
    - plain HTTP is accepted only for loopback test registries;
    - an edit of an entry that changed meanwhile is refused;
    - un-marking private asks for confirmation, and there is no push override;
    - the default registry cannot be removed.
  - **Login:** checked against the registry (anonymous `GET /v2/`, then Basic,
    short timeouts) before anything is kept. A registry that answers without
    login cannot verify it, and the page says so. Cancel, Escape or closing
    Preferences cancels a login in flight. The password field's undo history is
    dropped.
  - **Background work:** sign-in states, logins, logouts and removals run on
    background threads, never on the UI thread.
  - **Credential store:** chosen at run time. On macOS it is the Keychain
    through `security`: the secret is hex-encoded on the tool's standard input,
    never in arguments; printable ASCII, at most 1536 characters, checked before
    any request. Without a system store the page keeps a session-only memory
    store whose text points to the `UNEMU_REGISTRY_*` variables, and `image
    login` refuses.
  - **`image login`** reads the password in termios raw mode, where Ctrl+C
    cancels and restores the terminal, and uses the same accounts functions,
    store and preferences lock as the page.
  - **Tests:** headless tests cover the store trait and every page action
    against the in-process mock registry, including cancel, stale edits and the
    thread the store runs on. They also cover the preferences merge between
    page, CLI and shell, the raw-mode prompt on a pseudo-terminal, and that no
    secret reaches Debug output, status text, drawn text, undo history or the
    preferences file. The Keychain backend has an opt-in test on a throwaway
    keychain.
  - **Out of scope:** wiping secrets from memory. Separate later work: Secret
    Service, Windows Credential Manager and Bearer tokens. The GUI Push… and
    Pull… dialogs landed separately (entry above).
  - **Manual checks:** native look and the real Keychain.
  - **Mock-up departures, accepted for now by the maintainer on 2026-11-15:**
    - Only the General and Registries tabs exist; the mock-up also shows
      Library, Display and Input.
    - The Add dialog has no Display name or "Allow access without login"
      fields, and registry rows show no display name. Namespaces landed later
      (privacy per namespace, entry above).
    - "Allow plain HTTP" is limited to a test registry on this computer, not
      the local network.
    - The Log in dialog says Basic authentication over TLS (or plain HTTP to a
      loopback test registry), not a token service with refreshing tokens. It
      also states the Keychain limit.
    - Edit… and Test connection are added. The details pane shows the
      connection test instead of the mock-up's access-check line.
    - Every action applies at once: OK and Cancel both close, and Apply stays
      disabled.
    - Sign-in states show "Checking…" while the store is asked.
- **2026-11-15 — Source archives for the open fixture firmware:** the
  Kiselev, XTIDE Universal BIOS and open VGA BIOS fixture scripts now also write
  the corresponding source of their GPL/LGPL images as deterministic,
  SHA-256-pinned POSIX ustar archives next to them (gitignored; the CI
  `fixtures` gate builds them): `kiselev-bios-a4af458-atreset-src.tar`
  (upstream at the pinned commit with `at-reset.patch` applied, without
  upstream's prebuilt `binaries/` and `xtide_r625/`, plus the patch and a
  NOTICE; GPL-3.0-or-later), `xtide-universal-bios-r638-src.tar` (SVN r638
  trunk export plus NOTICE; GPL-2.0-only) and `vgabios-0.8a-unemu.1-src.tar`
  (the fork's pinned tree, the one Nix builds from, without upstream's prebuilt
  `VGABIOS-lgpl-latest*.bin` and `.beads/hooks/`, plus NOTICE;
  LGPL-2.1-or-later; re-pinned 2026-11-15 as `vgabios-0.8a-unemu.2-src.tar`,
  above). Before writing them, the Kiselev and XTIDE scripts
  rebuild a shipped image (AT reset BIOS) or both downloads from the staged
  source and compare the pinned hashes.
  **MEASURED on aarch64-darwin:** repeated runs reproduce each archive hash;
  the NOTICE rebuild commands run in each extracted archive reproduce every
  pinned image or download hash, including XTIDE r638 `ide_at.bin` and
  `ide_xtp.bin` byte for byte with NASM 3.01 once `__DATE__` is the commit
  date. Linux hashes are not measured. Carrying the archives as a firmware
  package's `source` layer is separate work. See the vendor READMEs and
  [`commands.md`](commands.md).
- **2026-11-14 — No silent stop, session write modes, toolbar speed:** with a
  machine running, Machine library, Open machine file… (once a path is
  confirmed), Exit, ⌘Q/⌘W and the window's close button pause it and ask
  "Stop it and continue?" (Stop machine / Cancel); Cancel resumes without
  catch-up and restores guest focus and pointer capture. Quitting works through
  open menus and dialogs, closing them first. Gaming mode keeps its
  Keep/Discard question. Library-disk journals of image sessions live in the
  image workspace and Gaming Discard removes them. The macOS build then
  installed no platform application menu so ⌘Q reached the shell; the macOS
  application menu entry above replaces that.
  The Insert dialog fixes the write policy only for machine-file entries;
  Browse… and From library… disks switch between read-only and a journal on
  re-insert, and browsing the same file and SHA-256 reuses its row. A toolbar
  Speed combobox after Layers mirrors View › Speed, shows Unlimited during the
  fast-forward hold and re-anchors pacing like the menu. Headless tests cover
  each prompt path, the policy switch and the toolbar selection; native visual
  and Dock/menu-bar behaviour need manual checking.
- **2026-11-14 — Open VGA BIOS on the open AT profile:** `vendor/vgabios/fetch.sh` builds the unemu fork of the LGPL VGABIOS (release `v0.8a-unemu.1`, revision `8e154f04`; re-pinned 2026-11-15 to `v0.8a-unemu.2` with the same ROM and probe bytes, above) and its INT 10h probe floppy with Nix from the sibling `vgabios` repository, deriving the forge from this repository's origin remote, and checks both SHA-256 pins; neither file is committed. Catalogue entry `vgabios-0.8a-unemu.1-std` is an open (LGPL-2.1-or-later), **Tested** video BIOS: raw 32 KiB at C0000h, requiring the register-level `vga`. `machines/at-games-open.toml` replaces its MDA/VGA-text console with that card and ROM; XTIDE stays at C8000h. **MEASURED at CPU `fe240a56`:** Kiselev POST runs the ROM, shows its sign-on and "EGA/VGA (Video BIOS Present)", and FreeDOS reaches `A:\>` (`retired=15241264`); the probe reports all 32 checks `ok`, `fails:0000` (13 screen lines, `Halted` at `0000:880A`, `retired=10239957`); a boot sector drives mode 0Dh panning, page 1 and a CRTC byte start through INT 10h and ports, then mode 13h with a BIOS pixel and DAC entry; all four 0Dh frames match planar memory and the 13h frame matches the chain-4 byte stream and the BIOS default palette, pixel for pixel (`retired=10730615`). The CI `fixtures` gate now also builds XTIDE, the FAT16 disk and these fixtures; from CI's credential-less HTTPS origin the VGA BIOS builds only while `vgabios` is anonymously readable (MEASURED readable from the LAN), otherwise the gate fails naming `UNEMU_VGABIOS_FLAKE`. Registry delivery from `registry.unemu.dev/unemu/` is later work. No game is qualified. See [the fixture notes](../../vendor/vgabios/README.md) and [Video ROM placement](design/firmware-and-machine-classes.md#video-rom-placement).
- **2026-11-14 — Registry push and pull:** the platform-neutral `image-registry` crate pushes and pulls tagged machine images, media sets and firmware packages to OCI distribution registries, with `image push/pull/remote ls/login` CLI commands. References resolve Docker-style against `registry.unemu.dev`; preferences list registries (host, private flag, user, no secrets); secrets come from the environment (bound to one host) or the macOS Keychain via the `security` tool. TLS is mandatory except for loopback test registries; authentication is anonymous first, then Basic, and credentials never follow a redirect to another origin. Push uploads only missing blobs, chunked and resumable above one chunk, publishes the manifest last, keeps version tags immutable and moves other tags only with `--move`. A push guard classifies each blob by the most restrictive mark any local manifest records for it and refuses user-supplied content outside private registries, anything but open content in `unemu/`, unlisted registries, and targets that are anonymously readable or answer with a Bearer challenge; `--dry-run` prints each blob's effective class and where it came from. Pull resumes interrupted blob downloads by range, verifies every digest through the store import and tags only a complete closure. Mock-registry tests cover resume, dedup, ordering, auth, digest mismatch, tag rules, paging and every guard refusal; the live round trip against registry.unemu.dev is opt-in. Firmware packages (2026-11-15) travel through the same CLI commands and guard: CLI mock-registry tests push and pull the open Xi8088 package with identical manifest and blob digests, list it in `image firmware ls` with its facets and as `firmware` (no Start) in `image ls`, show a repeat pull fetching nothing, refuse a user-supplied package to a non-private registry and to `unemu/`, and show each ROM layer's effective class in `--dry-run`. Since the licence work (entry above, 2026-11-15) a package whose every layer is open has an open config, and the guard adds the copyleft source rule. Bearer-token authentication, the GUI login and Browse registry remain separate.
- **2026-11-14 — Open AT protected-mode return test:** `vendor/at-pm-test/`
  holds a project-owned NASM program and an offline build script for a
  bootable 1.2M floppy (SHA-256 pinned, built by the CI `fixtures` gate). For
  16 rounds it enters 80286 protected mode (GDT/IDT, LMSW, far jump, memory
  above 1 MiB and its A20 alias, INT through the IDT, its own #GP handler),
  returns through an 8042 pulse reset (CMOS 0Fh = 0Ah) and a triple fault
  (IDT limit 0, CMOS 0Fh = 05h), and switches A20 through 8042 D1h/D0h with
  the FFFF:0010h wrap check. Results go line by line to COM1 and the BIOS
  teletype. **MEASURED (open golden, at-games-open BIOS and memory map):**
  every `pm` and `a20` stage passes; every reset stage reports
  `FAIL reboot` (`failures 32`, 211,149,814 retired instructions).
  **READ (pinned Kiselev source):** that BIOS cold-starts on every reset and
  never reads the shutdown byte. The resets, the shutdown and the survival of
  memory above 1 MiB are confirmed; the 0040:0067h resume needs an open BIOS
  with the AT shutdown dispatch. The golden pins this as expected to fail.
  The IBM 5170 user-ROM golden passes: every stage `ok`, including the
  8259-mask and triple-fault-handler checks, `failures 0`, 31,935,006 retired
  instructions (MEASURED with the user-supplied 1985-06-10 BIOS). Its first run stopped in POST at 601/162 because the machine had no
  AT fixed-disk controller, which the 1985 POST probes while testing the diskette adapter.
  The test machine now has an empty `at-adapter` IDE card. The private
  Windows 3.x Standard-mode acceptance steps are in
  [`design/at-pm-test.md`](design/at-pm-test.md). The counts in this entry
  predate the INT 15h lines (2026-11-15 entry above).
- **2026-11-14 — Desktop image library and disk layers:** local-store summaries join machine files in an Explorer-style tree, virtualized details list and selected-image pane. Search, metadata facets, repository/tag groups, sorting, favorites and local labels use cached summaries. Store loading, bounded cache rebuilds, verification, imports and publication run in a background worker. Image launches pin the selected descriptor and reuse a regular workspace; Gaming mode isolates all writable media and CMOS, with explicit Keep as new image or Discard and startup recovery. Selection uses cancellable descriptor checks; full disk verification happens at Start. Disk layers exposes chains, sharing, working-sector counts, whole-machine publication/flatten and confirmed discard. Tag deletion preserves blobs. The store root follows platform data-directory conventions and can be changed under Tools. GUI acceptance remains manual; media-set browsing stays separate.
- **2026-11-14 — Image blob compression (`+zstd`):** disk bases, sector packs,
  floppies, firmware, media-set payloads and firmware-package ROMs and extras
  of at least 4 KiB are stored as streaming single-threaded zstd level 3
  frames, because the platform provider (or the media-set or firmware-package
  envelope) declares their `+zstd` types. Frames are deterministic, so a ROM
  embedded in a machine image reuses its package's blob. Metadata, layer
  descriptors, CMOS and tiny payloads stay raw; `--no-compress` keeps raw
  fixtures. OCI descriptors name the stored bytes; `unemu.decoded.digest` and
  `unemu.decoded.size` carry the raw identity that configs and layer parents
  use, so machine digests do not change. Every reader accepts raw and `+zstd`
  blobs and fails closed, naming the blob, on a corrupt stream or decoded
  mismatch. Resolution verifies by streaming and writes nothing; only a mounted
  hard-disk base or inserted media-set medium is decoded into
  `cache/materialized/`, after a free-space check. Machine floppies, firmware
  and sector packs decode in memory, and layered heads stream their members.
  `image sweep` removes decoded files no tag or workspace pin reaches;
  `image ls` shows stored and decoded sizes and the cache size. Store-first
  firmware resolution decodes a compressed package ROM into memory; a raw ROM
  from an older store is still read in place. No filesystem sparse files are
  used. `image compress` writes a new tag for existing machine images, media
  sets and firmware packages, and keeps a machine's import record (journal
  retention). Registry push and pull move the stored frames unchanged (no decode
  on push), and the push guard also matches a compressed blob by its decoded
  digest, so a raw and a compressed copy of one ROM share one class.
- **2026-11-14 — macOS host shortcuts:** Command chords stay in the shell,
  with no guest modifier or letter scan codes. Ctrl/Alt and Ctrl+Alt+Del remain
  guest input. Mute uses ⌘⇧M and Stop uses ⌘K, preserving ⌘M minimize and
  ⌘Q quit; ⌘W closes, ⌘H hides and ⌘Tab switches applications. Hold ⌘T for
  Unlimited; releasing either key or losing focus restores the selected speed.
  Menus and Help show macOS bindings; other platforms retain their shortcuts.
  Headless tests inject either platform and cover actions, guest bytes, repeats
  and focus loss. Native visual acceptance remains manual.
- **2026-11-14 — Disk publication review fixes:** failed CAS releases publication intent; strict artwork/metadata checks precede intent, and abandon preserves journals. Starts enforce pinned tags and fresh-session requests, retain one raw verification plus one logical-head/cache verification for layered disks, and reuse captured preparation. Discard uses a durable tombstone and generic sidecar locks, lists affected media and requires explicit confirmation. Flatten streams new raw heads at the layer depth limit; published local records preserve retention. Regression coverage includes process exits with layered parents and both CAS modes.
- **2026-11-14 — Immutable disk layers and workspaces:** fixed disks seal deterministic sparse sector packs into parent-bound descriptors; chain reads use the existing sector interface and verify complete logical SHA-256. Named isolated workspaces retain journal policies, with a shared materialized cache for layered heads. Offline whole-machine publication includes all catalogue media and CMOS, verifies the closure, publishes by tag CAS and rebases the workspace through a durable retryable intent. Opt-in CLI commands run, list, discard and publish workspaces. Fault injection covers publication steps; tag deletion leaves shared blobs intact. GUI integration, registry transport and GC remain separate.
- **2026-11-14 — Installation media sets:** Neutral deterministic OCI media-set codec, extras with provider-validated canonical DOS names, PC floppy/ISO storage formats, `image media import/ls/show`, and summary facets are implemented. Media > Insert offers From library, ordered catalogue loading, an initial disk selection and read-only or machine-local journals. Optical, incompatible and damaged sets remain visible with reasons. Imports preview their order and offer dry runs; preparation runs in background jobs, and transactional catalogue insertion reuses disks and verified receipts. The default store uses the shared unemu family data root directly, with store.toml and images/ inside; Preferences can override it. The library collection and Import dialog landed on 2026-11-15 (entry above); the transfer-medium generator remains deferred.
- **2026-11-14 — Machine settings:** Machine > Settings… (Ctrl+, outside guest
  focus) opens the classic seven-tab property sheet from effective inherited
  values, including user-ROM machines without their image bytes. Assembly owns
  the fields, domains, disabled-choice reasons and all resource-conflict pairs.
  Declared catalogue firmware also constrains services before ROM bytes are read.
  Running/paused sessions are read-only. OK/Apply previews and writes a sparse
  `machines/local/<parent>-custom.toml` child with explicit replacement consent,
  then selects it in the library. Shipped files and inherited media pins/policies
  stay unchanged; pending image checks still gate Start. Headless model tests
  cover discovery, round-trips, preview bytes, invalid edits and save guards;
  visual acceptance remains manual. Metadata uses the normal machine-file
  lowering rules; indexed ROM edits preserve other bindings and symlink spellings.
  Saved local children edit in place with a diff and overwrite consent; firmware
  edits defer discovery to readiness checks.
- **2026-11-14 — Firmware packages:** The neutral strict OCI codec, shared extras, PC roles/chip layouts/fit validation, ROM-folder import and `image firmware ls/show` are implemented. Import matches full hashes and lengths, rejects incomplete or ambiguous sets, preserves sources and publishes by CAS. Frontends explicitly select the store: desktop readiness honors Preferences, CLI run accepts --store/--no-store, and plain library loaders disable store lookup. Fully pinned declarations keep their image/chip shape; identical package re-import is idempotent and preselection store failures warn and fall back to declared paths and ROM folders; damaged selected packages block use. Machine-image import embeds ROMs and records package provenance. Firmware summaries expose role, board/class and catalogue facets without reading ROMs. The library's Firmware collection landed on 2026-11-15 (entry above); see [design §5.3](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md#53-firmware-packages).
- **2026-11-14 — Installation media sets:** the design section and [mock-up](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/mockups/install-media.html) were approved by the maintainer. The codec, CLI and From library dialog are implemented; the library collection and Import dialog followed on 2026-11-15; the transfer-medium generator remains deferred.
- **2026-11-14 — PC machine image import:** `pc-image` imports resolved TOML/YAML machines into self-contained OCI blobs and local tags, validates PC bundle bindings and resolves them back to assembly input. Firmware, full media catalogues and CMOS are copied; journal generations freeze to whole raw bases through private recovery copies. `image import` requires an explicit store and tag, with `--create` for initialization; interrupted journals require named per-medium recovery choices. Validation and resolution keep large disks file-backed, and local import records retain policies and generation choices. Source files and journals remain unchanged. Layers, workspaces and registry transport remain separate.
- **2026-11-14 — PC machine image import:** `pc-image` imports resolved TOML/YAML machines into self-contained OCI blobs and local tags, validates PC bundle bindings and resolves them back to assembly input. Firmware, full media catalogues and CMOS are copied; journal generations freeze to whole raw bases through private recovery copies. `image import` requires an explicit store and tag; interrupted journals require a recovery choice. Source files and journals remain unchanged. Layers, workspaces and registry transport remain separate.
- **2026-11-14 — Firmware folder index and progress:** desktop readiness uses a
  local, bounded, atomically replaced SHA-256 cache with fresh metadata checks;
  invalid indexes rebuild and completed scans drop deleted files from scanned
  folders without discarding hashes for other folders or pin sizes. All pins in a
  chip layout share one folder walk, and declared paths obey the size filter.
  The status bar and Locate firmware dialog show listing/hashing progress,
  all found lines plus bounded skip results, Cancel scan, cached Check again and
  an uncached Rescan action. Progress stays in the fixed-height message pane;
  listing and index are shared across machine checks for the desktop session.
  Start reuses the checked document, verifies images while showing “Loading machine…”,
  and retries discovery once after a located firmware file fails verification.
  Folder changes and Rescan refresh listings; selection alone does not.
  Pending machine rows show “checking firmware…” with Start disabled. Synthetic
  headless regressions cover cache reuse, edits, deletion, invalid indexes,
  cancellation and rescans; native visual acceptance remains manual.

- **2026-11-14 — Machine image catalogue:** `image-store` now provides the strict,
  deterministic schema-1 publication codec, read-only degraded display projections,
  atomic local catalogue overlays and personal ratings/notes, and explicit overlay
  publication preserving the machine digest. Personal fields never enter blobs.
  Local per-title facet/search summaries rebuild in bounded lazy batches; warm
  startup reads the index, library and cache without opening payload blobs.
  Per-image failure rows prevent rebuild starvation; content hashes handle restored
  local files. Corrupt libraries permit read-only listing, cache writers merge rows,
  and capacity pressure is reported with reduced search data. Publication consumes
  overlays and moves local personal data, sanitizes artwork and reuses unchanged
  metadata bytes. Overlay artwork is exposed as GC roots. GUI wiring, provider
  launch compilation, import and GC remain separate.
- **2026-11-14 — Host speed controls:** View > Speed offers 100%, 200%, 400%
  and Unlimited; every session starts at 100%. Ctrl+Alt+T temporarily holds
  Unlimited and restores the selected mode on release or focus loss. The status
  cell measures virtual-time progress against host time over the last second.
  Paced modes may use the full observed frame period; Unlimited leaves 3 ms
  for presentation. Periods track repaint cadence within 16.7–50 ms, with at
  most one guest frame on each 60 Hz deadline. Mode changes and resume re-anchor
  without catch-up. Audio is muted outside 100%, without time-stretch; the UI
  distinguishes speed mute from the user preference toggled by Ctrl+Alt+M.
  Returning to 100% uses fresh buffering and preserves that preference.
  Guest clocks and headless execution are unchanged. Headless pacing, keyboard,
  audio and synthetic-machine determinism tests cover these paths; no GUI
  acceptance was performed. See [window commands](commands.md).
- **2026-11-14 — Machine image store:** platform-neutral OCI layout, streamed immutable SHA-256 blobs, deterministic manifests, provider-validated complete closures, paged local tags, provider-declared payload codecs, foreign OCI index preservation, missing-artwork reporting and atomic compare-and-swap updates are implemented; application wiring, import, layers, workspaces and registry remain separate.
- **2026-11-14 — Application logo:** the logo now shows "PC"; the project name remains unemupc98. Embedded window/Dock icon, GIF-timed welcome animation (file-only `reduced_motion` preference), static About logo and README branding. The welcome logo is centred above the heading
  at up to 128 px; welcome/About frames key only the exact uniform border colour
  connected to the perimeter by four-neighbour paths. Enclosed regions of that
  colour remain opaque, preserving the logo interior. The window/Dock icon is unchanged; visual
  acceptance remains manual.

- **2026-11-14 — Desktop media controls:** the Media menu and Ctrl+Alt+I
  expose a per-drive Insert dialog, catalogue next/eject, disabled absent drives
  and CD-ROM, and the fixed disk's write policy. Browse checks geometry and full
  SHA-256, then appends through assembly's validated media boundary. Existing
  catalogue policies stay fixed; browsed media permit read-only or a separately
  named journal with the drive's retention policy (ten generations by default).
  Cancel preserves the medium; paused insertion advances no guest time.
  Browse validation errors
  remain editable in Insert without ending the session. Mount/persistence failures
  open an error dialog and stop execution. Tools creates blank CHS/size fixed disks
  or formatted standard FAT12 floppies using the existing media-create code. Relative
  journal paths use the base directory; relative creation paths use the machine
  directory. Creation prefills the failing drive's path and geometry. Journals
  default beside their base with a Save picker. Occupied creation outputs ask for
  a new name. Status cells clip independently; library warning markers exclude
  pending checks
  and skipped firmware roots. Details wrap left, shorten machine-folder paths,
  offer cause-specific recovery hints and report EMS unknown after failed loading.
  Reset chords account for remaining typed-buffer capacity, discarding stale host
  backlogs and waiting when the complete chord cannot yet fit;
  replay retains typed-chord ownership. Headless tests cover these paths; native
  picker and visual acceptance remain manual. Live journal-sector counts are not
  exposed by the frontend and are omitted.

- **2026-11-14 — Buffered guest warm-reset input:** the shell's menu,
  toolbar and Ctrl+Alt+End transfer the whole Ctrl/Alt/Del make-and-break chord
  to the keyboard in one call. Its 16-byte typed buffer accepts keys while the
  controller interface is disabled; FFh/F5h/F4h/F6h discard all unsent typed bytes,
  preserving the controller output and ACK/BAT order (IBM PC AT Technical
  Reference, Keyboard, Keyboard Buffer and Commands from the System). The
  bounded overflow policy replaces the last buffered byte with 00h. Whole-script
  golden injection retains its existing order. Snapshot version 9 saves and
  validates the typed buffer. Host shortcuts now pass Ctrl/Alt taps to the guest
  because modifier deferral was removed; Ctrl or Alt held alone reach games.
  Consuming a host chord releases every already-forwarded modifier, and plain
  Ctrl+C retains its make/break sequence. Mock-bus and headless tests cover
  buffering under ADh, reset flushing, delivery, overflow and snapshot restore.
  Both `user_rom_at_games_ctrl_alt_del_reboot` and its frame-chunk variant use
  one-call typed injection; their existing pins await supervisor measurement.

- **2026-11-14 — ATA read-block timing:** PIO reads now enter BSY with DRQ
  cleared between blocks and schedule the next block from a nominal 3600 rpm,
  17-sector service clock (about 1 ms). A Status acknowledgement after consuming
  a block can lower IRQ14 before the next block raises it, preserving separate
  edges at the cascaded PIC. Writes and the initial read block remain immediate.
  Snapshot formats 3 (ATA) and 4 (AT adapter) include the pending deadline and
  reject older formats. Mock-bus timing, reset/replacement, malformed-snapshot
  and continuation checks accompany an open guest IRQ14 regression; both IRQ
  regressions fail against the previous implementation.

- **2026-11-13 — Machine settings capabilities:** assembly owns shared card,
  RAM, SB16/EMS/UART/IDE resource domains and defaults, video decodes, named
  floppy geometries and firmware slots. `capabilities::describe`, `choices`
  (with paging for large integer domains) and `validate_draft` expose options
  and structured diagnostics without UI dependencies. Drafts replace resources
  atomically, retain card identity paths and report unavailable firmware as
  pending. Known firmware metadata also restricts floppy types and reserves
  ROM ranges before bytes are available. Settings presentation remains separate.
  Repository-file synthetic parity snapshots and the existing lowering and
  validation corpus cover unchanged legacy behavior and source diagnostics.

- **2026-11-13 — egui desktop shell:** `unemupc98-window` opens a classic
  in-window File/Machine/Media/View/Tools/Help menu, toolbar, status bar and
  machine library. Start changes the same client area to a nearest-filtered
  4:3 guest texture; Pause/Resume, guest Ctrl+Alt+Del, media next/eject and Stop
  reuse the existing session boundaries. Stop finishes persistence. Missing
  firmware/media disables Start using the existing validator. View offers
  modern skin and system/light/dark themes. File Open accepts a machine-file
  path; Tools › Locate firmware/Preferences save ROM folders and refresh
  readiness using the shared explicit-roots SHA-256 discovery. The CLI accepts
  repeatable --rom-dir. The macOS shell uses only its native title bar and the
  toolbar paints its icons without font symbols. Future editors/import/layers are placeholder dialogs, save
  state/CD-ROM remain disabled. Drive activity lights landed on 2026-11-15
  with the status-bar drive chips (entry above). Last-run metadata stays blank. Existing window
  tests remain headless; new command/discovery/texture/input tests also open no
  windows. Visual appearance and physical capture/audio need manual checking.

- **2026-11-13 — AT controller self-test and warm start:** 8042 command AAh
  reinitializes the output port to its power-on value and drives A20 enabled,
  while retaining the SYS flag/55h reply and issuing no CPU-reset request.
  This is an AT compatibility policy: IBM PC AT Technical Reference, Keyboard
  Controller, Commands, documents the self-test reply but not that output-port
  initialization. The IBM POST depends on it before protected-mode memory
  sizing after DOS has disabled A20. Mock-bus and synthetic IBM 5170 ROM tests
  cover D1h/DDh, AAh and the subsequent high reset fetch. The existing test
  without AAh still pins the empty-bus/#UD result with A20 disabled.
  `Machine::reset_cpu` remains a board CPU-line reset, preserving RAM, devices,
  the 8042 output port and A20. Host reboots use guest Ctrl+Alt+Del or relaunch.
  `user_rom_at_games_ctrl_alt_del_reboot` follows the guest path back to the
  FreeDOS A: prompt. **MEASURED (supervisor user-ROM release run):** reboot
  returns to `A:\>` at `CS:IP=05E3:15C4`, with 81,853,092 retired instructions
  and VGA text SHA-256
  `8810a375732aec052068b73542b7a9a9185e8be8d5224b75a7834aa5a02dc8b4`.
  The test pins those values. Its polling and the open install golden reject
  terminal run outcomes and a non-advancing virtual clock, so a failed warm
  start cannot spin forever below a virtual-time cap. Transient non-text VGA
  frames during POST are allowed. The open installation golden injects only
  Ctrl/Alt/Del makes for both warm starts:
  queued breaks would precede the keyboard reset ACK/BAT during POST.
  **MEASURED (before the installed-empty-drive correction, unchanged CPU pin):**
  guest warm starts move the C: prompt from
  screen FNV-1a `1429bef60948e1d5`, 102,208,270 retired instructions and virtual
  time 106,800,000 to `5d0c917f3ab08d17`, 98,075,818 and 100,800,000.
  Guest warm POST replaces the former host CPU resets, changing the executed
  path and the time displayed by DOS. This measurement predates the installed
  empty-drive correction; the combined endpoint is recorded under Open FreeDOS
  installation media below.
  **MEASURED mutation:** reverting only AAh output-port initialization fails
  both synthetic regression tests, but the open install golden still passes
  with the same endpoint. That golden qualifies the guest reboot path, not
  the IBM-specific dependency on AAh. With the user-supplied IBM BIOS, the
  same mutation never returns to the prompt after Ctrl+Alt+Del (measured
  before the polling guards existed; the guards now turn that into a prompt
  test failure).

- **2026-11-13 — Exact clock domains:** `device::clock` centralizes OSC =
  315/22 MHz and named PIT, CGA/VGA, UART, RTC, OPL3, 48 kHz output and
  12/16 MHz AT CPU crystal domains. Device durations, PIT input-edge counting,
  legacy scheduler ordering and visits, audio grids and host pacing use exact
  ratios. Legacy devices run only when a PIT input edge has elapsed, including
  when a scheduled transition occurs between PIT edges.
  One virtual tick remains one PIT clock. Rounded RTC/UART/VGA/pacing crystal
  spellings now derive from OSC; the existing RTC UIP window remains 1193 PIT
  clocks. CPU instruction-rate calibration and policy units are unchanged.
  See [conventions](conventions.md#device-time-and-clock-domains) for the rules
  for new code. This implements decision-22 §1a–b; the private time field,
  permanent multi-tick test build and stateful VGA beam remain separate seams.

- **2026-11-13 — Lo-tech hardware EMS:** the r02 board model owns 512 KiB
  to 2 MiB SRAM behind a 64 KiB MMIO frame at C000h/D000h, or E000h on Bare. Four
  write-only page latches decode 260h/264h/268h/26Ch and all 400h aliases;
  values 80h–FFh and unpopulated chips float high and drop writes. Power-on
  latches are deterministically FFh; CPU and device reset retain latches and
  SRAM. Ordinary RAM keeps its existing CPU path. On IBM 5170, E0000h–FFFFFh
  is system-board ROM space, so EMS uses C000h or D000h; Bare also permits E000h.
  Coverage and model choices are in `crates/devices/src/lotech_ems.rs`.
  Machine files require `frame` for `kind = "lotech-ems"`; `port` and `size_kib`
  have defaults. Both shipped game profiles enable 2 MiB at D000h, port 260h.
  The CPU-region snapshot includes the card. MEASURED: the open-profile
  behaviour golden is unchanged. The supervisor's gate provides the user-ROM
  at-games golden measurement before merge. LTEMM r01 is a fetched,
  double-hash-pinned, gitignored fixture, never redistributed or placed in a published disk image; the project
  emulates the board only and LTEMM is the user's own driver (decision-23, accepted). The ignored open-firmware
  FreeDOS MEM golden uses a disposable in-memory tools disk and a synthetic DOS
  program that checks all 128 pages through all four windows before MEM /E,
  including an aliased-page negative control. MEASURED: MEM reports 2,048 KiB
  total and free EMS (2,097,152 bytes). The D000h endpoint first retired
  18,681,813 instructions with text FNV-1a `0eda968da7052433`; the page-alias
  mutant failed the DOS check at 16,355,288 instructions with text
  `df1f6dc3129c84ce` (both re-pinned 2026-11-15, above). See
  [the machine reference](../../machines/README.md#lo-tech-ems-card-reference)
  and [fixture terms](../../vendor/ltemm/README.md).

- **2026-11-13 — Game-ready AT profiles:** `machines/at-games.toml` combines
  the user-supplied June 1985 IBM 5170 BIOS and IBM VGA EPROM, register-level
  VGA, 640 KiB + 1 MiB RAM for a shorter POST memory test, SB16 with OPL3
  (220h/IRQ5/DMA1/HDMA5), a COM1
  Microsoft serial mouse, a type-2 IDE disk with a separate journal, and a
  swappable 1.2M FreeDOS/transfer catalogue. B: is omitted because this BIOS
  has no 1.44M drive type. `at-games-open.toml` uses Kiselev AT-reset and XTIDE;
  since 2026-11-14 it also has the register-level VGA with the open VGA BIOS
  (entry above) instead of the MDA/VGA-text console. Both keep the empty 80287
  socket and fixed virtual RTC.
  The [game guide](../../machines/README.md#playing-dos-games) covers supplying
  ROMs/tools, disk preparation, copying files, BLASTER, mouse drivers and
  journal export/rollback. The FreeDOS fixture supplies only kernel and shell.
  Synthetic tests check resolved firmware pins/device selection, lowering,
  resource wiring, full-base hashing and non-writing `machine check`.
  Synthetic C0000h validation correctly refuses an unidentified system BIOS;
  the rest of that smoke uses a relocated C8000h stand-in. The opt-in
  `user_rom_at_games_freedos_prompt` keeps the actual C0000h mapping and uses
  a disposable blank HDD. **MEASURED (supervisor user-ROM run, earlier 4 MiB extended RAM):** this game-ready
  AT boots FreeDOS with the user-supplied IBM 1985-06-10 system BIOS and IBM VGA
  BIOS: `RetiredCapReached`, `CS:IP=0070:02CB`, `retired=68093462`,
  `dos_prompt=true`, VGA text SHA-256
  `a2b73fae1c7ee569829051dea41b7c997aaeace94f3d28a8d9ee7ab89e364452`.
  The test pins these values and the complete POST checkpoint sequence against
  CPU revision `060d446a883a9b2fc94b3ef91258cb4e6e98d916`.
  The IBM profile's measured boots are POST and guest Ctrl+Alt+Del with a blank
  type-2 disk, ending at `A:\>`. FDISK, FORMAT, copying and running from C:
  through the IBM fixed-disk BIOS on the AT adapter remain unmeasured, as does
  C: boot. No game or real
  mouse-driver session is qualified. The I286 supports LMSW entry, 16-bit
  protected execution and task switches; the pinned POST tests all 640 KiB +
  1 MiB. No memory manager is supplied; XMS, INT 15h block move and DOS-extender
  use remain unqualified. There is no 386 instruction set.
  Tyrian 2000 and Duke Nukem 3D require a 386 or later and remain **not yet**.
  **MEASURED (open fixtures):** `at-games-open.toml` passes non-writing
  `machine check` with the real pinned Kiselev AT-reset, open VGA BIOS, XTIDE
  and FreeDOS 1.2M fixtures and `media create` local disks. Its ignored FreeDOS
  prompt golden samples the POST screen, then records `RetiredCapReached`,
  `retired=15241264`, `dos_prompt=true`, VGA text SHA-256
  `41ff081e1227af8f1a053df1b627b734de4032963cc0357c488cbbe67e01469e`
  (previously, on MDA: `retired=13772948`, text
  `f016e6af8c82493b1a489b8753d1ad9c34f31da6af744e177984405ca1f4ead6`).

- **2026-11-13 — Sparse persistent media:** journal drives read their immutable
  base on demand and retain only written sectors, including removable floppies.
  CLI/window journal-disk launches retain the verified read-only descriptor from
  machine-file lowering through assembly: one streamed full-base SHA-256 and
  no whole-disk buffer. Independent assembly/offline opens verify once each.
  Metadata checks reject changes after verification through the host
  failure channel. Retention, readable finish, verify/rollback and atomic export
  use sparse states and bounded buffers; the version-1 format is unchanged.
  Counting-allocator tests cover a sparse 512 MiB media lifecycle and 504 MiB
  frontend launch. Seeded keep=2 workloads restore base sectors and compare every
  retained generation export with dense snapshots.
  `SectorSource` is the parent seam for future hash-linked layers; actual layer
  files, deduplication and FAT optimizations remain deferred. See
  [persistent media](design/persistent-media.md#memory-and-hashing).

- **2026-11-13 — Microsoft serial mouse:** opt-in `mouse = "microsoft"` on a
  UART card supplies a two-button 1200 7N1 endpoint. DTR/RTS power-up produces
  `M` after an assumed 10 ms startup plus a byte frame; inactive lines cancel
  transmission and drop motion. Signed motion splits across three-byte packets,
  with positive Y down. The generic UART now receives timed bytes, models RX
  FIFO/overrun, receive/line-status IRQs and FIFO timeout, and gates its board IRQ
  through OUT2. Loopback forces external modem outputs inactive and isolates
  external RX; a driver probe therefore powers the mouse down and restarting it
  identifies again. 7N2 receives the 7N1 stream; other divisor/data/parity
  mismatches receive 00h with framing error (an explicit approximation, including
  8N1). Line settings are sampled at the start bit, after startup, so raise-then-
  program works.
  Snapshots include endpoint identity, partial transmission and accumulated input.
  Machine files reject simultaneous PS/2 and serial host targets; the window
  routes its existing captured input to either target. Target lookup is resolved
  from configuration without synchronizing unrelated devices. Restore rejects
  endpoint/MCR line mismatches without mutation. Presets are unchanged.
  Logitech extensions, bit sampling, TX timing/interrupts, modem inputs and
  internal loopback feedback remain unmodelled. Synthetic IRQ4 ROM and mock-bus tests qualify the
  protocol. Manual check (maintainer, 2026-11-15): CuteMouse 2.1b4 loads on
  the open AT games profile and the mouse works in FreeDOS EDIT; MOUSE.COM,
  games and Windows driver sessions have not been exercised. See [serial mouse configuration](design/machine-files.md#serial-mouse).

- **2026-11-12 — CPU A20 wiring:** Assembly applies the shared 8042/92h OR
  line at construction; the machine forwards effective transitions after port
  writes and before runs using the change counter. CPU-only resets, including
  shutdown, preserve the line. IBM 5170's I80286 Machine model honours the gate.
  Bare rigs with an A20 source use that model too, retaining a low reset entry;
  source-free rigs keep flat Application(Real16). The initial transition count
  remains 1 after 8042 attachment, 0 for port 92h alone. CPU physical bit 20 is
  gated; host memory access and ISA DMA bypass it. Source policy remains OR
  (ASSUMED), with SeaBIOS qualification still owned by the generic ISA board.
  Synthetic tests cover bidirectional 1 MiB aliasing through both controls,
  OR combinations, CPU-only reset, gated fault metadata and redundant-call
  suppression. Bare A20 rigs vector divide error through INT 0 in the guest.
  The IBM 5170 reset test pins the assumed external-gate/ROM interaction:
  with A20 off, FFFFF0h becomes empty-bus EFFFF0h and FFh bytes enter #UD.
  See [the board contract](design/machine-config.md#a20-line-and-system-control-port-a).

  **MEASURED CPU-detection re-baseline, with the same CPU pin:** the bare POST
  rig has an 8042 and therefore runs the I80286 Machine model. The single-step
  trap after POP DS changes Kiselev's 8088-identification result from
  `Harris-designed 80C88` to `Intel 8088 '78` and adds its segment-load warning.
  The maintainer accepted this model change. Only this CPU-detection report
  changes; the floppy, FreeDOS and IDE screen expectations stay unchanged.

  | Bare POST expectation | Previous | Current |
  | --- | --- | --- |
  | MDA FNV64 (`bios_boot_golden`) | `bfa9b5ab3f8508a4` | `b59246d76d78f3fc` |
  | Logical FNV64 (`bios_boot_golden`, volatile media POST, scheduler POST) | `e36ef0102a7cbc14` | `945fcce568e5d520` |
  | MDA cursor row | 18 | 19 |
  | Retired instructions | 19,015,022 | 19,015,022 |
  | Virtual time | 20,000,000 | 20,000,000 |

  **Proof:** in the captured 25-row MDA console, restoring only the CPU
  identification text, removing only the added warning row and appending the
  resulting empty final row reproduces the previous raw MDA and logical FNV64
  values exactly. Removing that row also restores cursor `(0,18,true)` from
  `(0,19,true)`. RTC, other text, stop kind and POST bytes require no changes.
  No A20-wrapped guest access explains this drift; it is the model's TF behavior
  (Intel 80286 Programmer's Reference Manual, §9.7.1).

- **2026-11-12 — Derived AT CMOS century:** every derived layout seeds 32h
  from the RTC start year in BCD, including the standard AT layout. The IBM
  PC AT Technical Reference, System Board, CMOS RAM address map, defines this
  date century byte outside the checksum range. Images retain their supplied
  byte; journals retain their persisted byte. The live two-digit year still
  rolls from 99 to 00 without updating the century seed. The open Kiselev BIOS
  displays `2000-01-01` instead of `0000-01-01`; FreeDOS keeps
  `Sat 01-01-2000` instead of falling back to `Tue 01-01-1980`, and new DOS
  files receive the 2000 date. Screen goldens retain these date differences. See
  [machine-config.md §RTC start and NVRAM source](design/machine-config.md#rtc-start-and-nvram-source).

  **MEASURED against `20cd1da`, with the same CPU pin:** seeding CMOS 32h
  with 20h gives the Kiselev BIOS the century for its RTC line and date service.
  FreeDOS accepts that date, and FreeCOM displays `Sat 01-01-2000`;
  the DOS write golden's new `ZZ` entry is dated `01-01-00` rather than
  `01-01-80`. Reversing only the RTC date, FreeCOM date line and `ZZ` year in
  the captured consoles exactly recovers every previous raw and logical hash.
  The raw MDA hash re-baseline is:

  | Golden | Previous hash | Current hash |
  | --- | --- | --- |
  | BIOS POST | `0b55c39cf21513e6` | `bfa9b5ab3f8508a4` |
  | BIOS floppy boot | `9a13654e51c09b99` | `bfa3c76f8f761e6b` |
  | DOS DIR | `20d57080f986a38c` | `806c1c1c475339a2` |
  | DOS write | `e4c4f5de9a7f0d49` | `71f290c7a24fb363` |
  | FreeDOS prompt | `c95b91cec87e1379` | `83ffbc2844f28889` |
  | IDE prompt | `b93bd1a24305e490` | `b7d8e6cd59c50bf2` |
  | IDE final | `ada1634e25758753` | `a045a62f7781ffa1` |

  Logical hashes normalize RTC/time rows but retain the DOS date and directory
  entries. BIOS POST and floppy logical hashes remain unchanged; the moved
  constants are:

  | Golden | Previous logical hash | Current logical hash |
  | --- | --- | --- |
  | FreeDOS prompt; media fixture; scheduler prompt | `af201c7e8acb071d` | `c46f0c147c881837` |
  | DOS DIR; scheduler DIR | `d330b78c4533697e` | `65259fa4c26a891c` |
  | DOS write; media fixture; scheduler write | `a6601fff99236a61` | `6b0404d06d7532c3` |
  | IDE prompt | `26cb04d76a1ee3c7` | `fbd3caf6e77b879d` |
  | IDE final | `bf790c95b531f8e8` | `48d2b9699fbe5cc2` |

  With the valid 2000 date, the measured retired endpoints move as follows;
  virtual-time caps, cursors, POST/COM1 and stop kinds remain unchanged.
  Both scheduler engines match complete access/state streams with these
  date expectations.

  | Golden / endpoint | Previous retired count | Current retired count |
  | --- | --- | --- |
  | FreeDOS prompt | 8,800,477 | 8,800,473 |
  | DOS DIR | 11,772,930 | 11,772,926 |
  | DOS write | 11,766,599 | 11,766,596 |
  | IDE prompt | 15,775,579 | 15,775,575 |
  | IDE final | 19,746,109 | 19,746,105 |

- **2026-11-12 — Open AT BIOS:** legacy Bochs rejected; keep Kiselev with an explicit display switch for open goldens and use the user-firmware lane for AT fidelity; an open 286 AT BIOS is [deferred](deferred.md#open-at-class-80286-system-bios).

- **2026-11-12 — Audio seam:** the `audio` crate provides exact PIT-to-48-kHz
  mapping, interval box filtering of edge histories, timestamped native-rate integration,
  saturating stereo mixing and bounded headless PCM capture with streaming hashes. Run-scoped
  `AudioSinkKind` defaults to no output; source-free machines create no mixer.
  Sources release history even with output disabled and retire after their tails.
  Assembly settles devices before audio observation, including halted time.
  The AT speaker records OUT2 AND port 61h bit 1 at exact PIT clocks, with
  independent GATE2 control, direct bit-banged/PWM output and 0/+16384 stereo
  drive levels with exactly zero silence. SysControl/PIT pairs attach it
  automatically; default None output bounds device history to an incomplete
  frame even without capture calls. Synthetic AT tone and toggle goldens pin PCM,
  while late device-history errors surface at the next observation and restore
  the current level in a new segment without waiting for another pin change.
  The window plays through cpal (i16 or f32, stereo plus silent extra host channels),
  preferring 48 kHz, then a compatible device default, then the supported rate
  nearest 48 kHz. Producer-side linear interpolation carries exact rational phase
  across blocks; the deterministic stream stays at 48 kHz. Downsampling has no
  anti-alias filter and can alias high frequencies. Device-rate ring frames keep
  a 200 ms capacity, 50 ms prefill/latency target and oldest-frame drops above
  70 ms. Underruns play silence. Ctrl+Alt+M toggles host mute; unavailable or
  fatally failed output reports once and leaves
  emulation running silently. Transient cpal notifications keep playback active;
  recoverable source errors never stop the window. Tests stay device-free; physical playback is
  unmeasured. Optional SB16 DSP 4.05 now supports 8-bit and 16-bit mono/stereo DMA output
  (single-cycle and auto-init), exact programmable rates, direct DAC, pause/resume,
  independent byte/word IRQ acknowledgement and backward-compatible hardware snapshots. The `sb16` machine-file card
  defaults to base 220h, IRQ5, DMA1 and word DMA5; no preset adds it. CT1745
  registers and SB Pro aliases read back with documented reset values; master
  and voice volumes attenuate PCM. Other mixer controls are stored only.
  The onboard YMF262 now supplies native integer FM (Nuked-OPL3 1.8 port),
  virtual timers and OPL aliases, with CT1745 MIDI/master attenuation. Recording
  remains unimplemented. See
  [audio design](design/audio.md).

- **2026-11-12 — 8042 keyboard command transmission:** writing a keyboard
  byte to port 60h releases the keyboard clock and clears command-byte bit 4;
  controller parameters and D3/D4 writes retain their routing, and the aux inhibit
  bit is unchanged. ACK readiness waits two 11-bit serial frames (**DERIVED**:
  2626 PIT clocks at the model's **ASSUMED** 10 kHz clock). Reset BAT adds
  300 ms (**READ**: IBM PC AT Technical Reference, March 1986, 101/102-Key
  Keyboard, Basic Assurance Test, p. 4-39; lower end of 300–500 ms) plus a reply
  frame. Unread output remains first; draining it cannot shorten reply deadlines.
  Snapshots preserve queued receive deadlines and use device version 7.
  E0h read-test-inputs remains unmodelled because serial pin levels are not tracked.
  Keyboard replies now take the host and reply transfer time and the keyboard's
  basic assurance test delay (IBM PC AT Technical Reference, 101/102-Key Keyboard,
  Basic Assurance Test), so the BIOS keyboard reset finishes later in virtual
  time; screen content is unchanged apart from the clock rows. The six BIOS/DOS
  goldens retain their normalized logical hashes and cursors. Only BIOS floppy
  changes retired instructions: 19,008,614 → 18,991,006 (−17,608) within the same
  20,000,000-PIT-clock cap; all other retired counts and virtual-time caps stay fixed.
  The measured raw MDA hash re-baseline is:

  | Golden | Previous hash | Current hash |
  | --- | --- | --- |
  | BIOS POST | `809829e50bc06fbd` | `0b55c39cf21513e6` |
  | BIOS floppy boot | `938e07668f3b0b34` | `9a13654e51c09b99` |
  | DOS DIR | `bb926924f4d7efa1` | `20d57080f986a38c` |
  | DOS write | `153da3e002975834` | `e4c4f5de9a7f0d49` |
  | FreeDOS prompt | `526086b917baa58e` | `c95b91cec87e1379` |
  | IDE prompt | `49bbec2d36c0bbf0` | `b93bd1a24305e490` |
  | IDE final | `fb811e1f40836da7` | `ada1634e25758753` |

- **2026-11-11 — Absent physical memory on firmware boards:** assembly covers
  the AT's fixed 24-bit extent (16 MiB) around configured RAM, ROM,
  MMIO and existing zero-filled adapter gaps. Remaining holes read FFh
  (**ASSUMED** floating value) and discard guest writes, so memory sizing can
  probe above installed RAM without an unmapped CPU error. Configured extended
  RAM remains writable up to the shared 15,232 KiB board-map limit (**READ**,
  IBM PC AT Technical Reference, Section 1, System Memory Map); Bare
  rigs retain their maps. FE0000h–FFFFFFh mirrors the resolved low 128 KiB
  read-only as a board decode. Fill/copy scratch is bounded to 8 KiB; VM extent
  is fixed by AT decoding; CPU width and mapping bounds are validated. With
  A20 enabled, accesses above 1 MiB read FFh or extended RAM instead of Unmapped;
  the CPU wiring follow-up now makes a disabled gate alias physical bit 20. See
  [memory-map policy](design/machine-config.md#memoryconfig--a-semantic-memory-map).

- **2026-11-11 — Register-level VGA graphics:** opt-in `vga` cards capture
  planar 16-colour 0Dh/0Eh/10h/12h and chain-4 13h through owned indexed
  pixel frames with a captured DAC palette and presentation repeat factors.
  CGA-compatible mode 06h uses row-scan bits to select its memory banks. The existing display route follows
  AC text/graphics selection; the window presents pixels with integer scaling.
  CRTC logical geometry, start/stride, double scanning and Fig. 2-65 addressing,
  plane enable, AC colours and PEL mask are covered. CGA-interleaved shift,
  mid-frame changes other than the split and smooth-scrolling timing finer than
  a frame remain unmodelled. Frame-level panning and splits are covered below.
  Existing text presets and golden expectations are unchanged. See
  [graphics scanout](design/observation.md#register-level-vga-graphics-scanout).

- **2026-11-11 — XT devices (device level):** Mode 0 8255, XT SW1/PPI and
  83-key set-1 receiver, NMI enable latch at A0h, and single 8237 with XT page
  decode are available as isolated devices, with snapshots and trace policy
  tests. Review against the March 1986 XT Technical Reference corrected SW1
  POST/FPU bits (default 7Dh), removed the PC-only PA/SW1 path, added the reset
  hold threshold, bounded the keyboard queue to 16, and covered DMA/NMI aliases.
  The queue emits FFh on overrun. The reset hold uses a documented 1,193-clock
  compatibility threshold and preserves an unread board byte until PB7 clears
  it; tests cover the BIOS's 10,582-iteration hold at rate 1/1 and PPI/controller
  register aliases. The PIC test uses BIOS ICWs 13h/08h/09h. Module matrices
  distinguish READ facts from remaining wiring/timing assumptions; keyboard
  transmission is byte-level.
  No board is wired: `ibm-5160` and `8088` remain reserved and rejected.

- **2026-11-11 — Debug console:** emulator-only QEMU-style byte port (default
  402h, E9h readback) and bounded `DebugConsole` observation are available to
  bare Rust test rigs. IBM 5170 validation rejects the device; future
  `generic-386-isa` wiring and machine-file exposure remain pending.

- **2026-11-11 — PS/2 auxiliary channel and mouse:** the 8042
  can attach an `AuxDevice` at construction, routes D4h to it, tags aux output
  with status bits 0/5 and exposes IRQ12 as interrupt output 1 (command-byte
  bit 1). D3h injects an aux byte even without a peripheral. Clock inhibit
  retains peripheral output; ready keyboard/aux bytes alternate after controller
  replies. `Ps2Mouse` implements standard ID-00 commands, stream/remote/wrap,
  signed motion, three buttons, overflow and stream-only 2:1 scaling. Snapshots
  include partial exchanges and all peripheral state. Packets are emitted at
  input injection boundaries and coalesced while busy; sample-rate pacing in
  virtual time is not modelled. Machine configuration now opts in through `I8042Cfg.mouse` or
  `board.mouse` in TOML/YAML; slave IR4 / IRQ12 uses the same level propagation as
  IRQ1. `assembly::inject_mouse` deterministically accepts motion/buttons at
  run boundaries, including accumulation with reporting disabled. A synthetic
  AT ROM checks reset/enable replies and exact packets through a guest IRQ12
  handler with both PIC EOIs. Existing profiles attach no mouse. The window now
  captures a configured mouse on a client-area click, swallows that click, and
  releases with Ctrl+Alt+G, focus loss or suspension. It tries Locked then
  Confined, hides the cursor and shows the release chord in the title; macOS
  supports Locked only. Raw relative input uses a fixed 1:1 scale with Y-up
  conversion and carried fractions. Motion coalesces between button changes
  and enters only at run boundaries. Pure and headless presenter tests cover
  conversion, capture, button ordering and guest-visible packets; host GUI grab
  behavior remains unmeasured. See [window input](design/observation.md#captured-ps2-mouse).

## VGA frame scrolling and page flipping (2026-11-13)

Register-level VGA now applies AC horizontal pixel panning in planar, chained
and unchained 256-colour graphics and in text; CRTC byte panning and preset row
scan; and the 10-bit line-compare split with the AC panning-compatibility flag.
Start address and CRTC 08h latch at vertical retrace start; pixel panning
latches at retrace end using virtual time. This pairs a start written during
display with panning written during retrace for the next frame.
Scrolled text captures use indexed pixels for partial cells; ordinary text
captures retain their cell grid. Snapshots use version 4 and restore version 3
with latches initialized from the live registers. Mock-bus pixel tests and a
synthetic planar ROM cover scrolling and page flips without external firmware.

IBM establishes start-address latch timing. Abrash documents programming
panning after sync starts; sampling it at retrace end and CRTC 08h at retrace
start are explicit frame-level assumptions. Undefined panning values and split
boundary phase remain ASSUMED. CGA-interleaved shift, mid-frame changes other than the split, and
smooth-scrolling timing finer than a frame remain unmodelled. See
[frame scrolling](design/observation.md#frame-scrolling-split-screens-and-retrace-latches).

## IBM ISA VGA video ROM integration (2026-11-12)

- Firmware supports a separate C0000h–C7FFFh video scan for the IBM 5170 BIOS
  catalogue and both Kiselev builds. Extension-ROM checks retain their separate
  windows and checksums. The `ibm-vga-bios-1987-32k` entry is **Tested** for
  the qualified IBM 5170 BIOS 1985-06-10 case below. Its supplied full EPROM
  identity maps offset 2000h onward as 24 KiB at C0000h and requires register-level `vga`. A plain 24 KiB dump is unlisted.
- Machine files use `firmware.option_roms` plus `kind='vga'`; the catalogue owns
  the container offset, full-file SHA-256 pins include the unmapped prefix,
  and TOML/YAML also support explicit offsets for synthetic/unlisted ROMs.
- VGA adds subsystem and ISA setup/enable decode (3C3h/46E8h/102h), a
  deterministic colour-monitor sense approximation and virtual-time retrace
  polling. Existing mono/colour port select, plane-2 font upload and DAC index
  protocols are covered across enable changes. Version-3 snapshots include
  enable state. See [observation coverage and assumptions](design/observation.md#vga-cpu-decode-monitor-sense-and-polling).
- A synthetic 24 KiB ROM inside a 32 KiB container tests the open Kiselev POST
  path. **SUPERVISOR-MEASURED at CPU `fe240a56`:** the ignored
  `user_rom_ibm_vga_bios_mode13_pattern` qualifies the user-supplied IBM VGA
  BIOS with IBM 5170 BIOS 1985-06-10 on `ibm-5170`: POST initializes the video
  ROM, INT 10h mode 03h produces 80×25 text with 16-line characters, and mode
  13h produces the complete 320×200 byte-ramp pattern. The test pins Halted at
  `0000:7C50`, **23,463,327** retired instructions, and both capture SHA-256
  values in the [qualification table](design/firmware-and-machine-classes.md#video-rom-placement).
  Sampling requires the boot program's magic/stage marker and CS/IP inside
  its sector. Missing ROMs skip cleanly. No ROM or firmware screen text is
  stored or fetched; other BIOS revisions and VGA modes are not qualified.

## Works (as of 2026-11-10, main @ 479f8ed — the 286 foundation is finished)

- **An unmodified open BIOS boots FreeDOS from a floppy, headless and deterministic.** The Kiselev
  Xi8088 BIOS completes POST and its INT 19h loads a FreeDOS 8086 floppy over the µPD765 floppy
  controller + 8237 DMA; FreeCOM reaches its prompt, and a scancode script drives it to `A:\>`,
  runs `DIR`, and creates a file (`echo x>zz`) that the controller commits to the in-memory image.
- **Firmware catalogue:** IBM 5170 BIOS bytes are checked against the pinned
  Kiselev Xi 8088 1.0.2 SHA-256 before mapping. Unknown images list known builds
  and fetch instructions; listed firmware checks board, CPU and required parts.
  Declared or sole mis-sized ROMs receive a precise BIOS-placement diagnostic;
  ambiguous undeclared ROM sets report a missing BIOS. System BIOS presence,
  placement, power-on reset and isolation from writable
  RAM/other images are checked independently of allowlist membership. Option
  ROMs are also SHA-256 checked, aligned, bounded by the BIOS scan window,
  header/length/checksum validated and mapped read-only. Adapter space has no
  writable system RAM; MMIO is carved out of the zero-filled gaps. Assembly
  reports effective ROM scan order. Machine-file loader APIs accept the run-only
  unlisted escape while preserving source diagnostics. A synthetic 2 KiB
  extension has a fixture-gated BIOS POST acceptance test. CLI and window
  accept the run-only `--allow-unlisted-firmware` flag and report the unknown hash
  to stderr. Bare assembly remains unrestricted; the historical POST runner
  explicitly checks the real BIOS against the intended board catalogue.
- **Devices:** 16550 UART, 8259 PIC pair (master + slave cascade), 8253 PIT, 8042 keyboard
  controller, MC146818 CMOS/RTC (seeded NVRAM), 8237 DMA, µPD765 floppy (read + write), port 0x61,
  POST port 0x80, independent VGA/CGA and MDA text cards, all behind the `Device` trait with mock-bus
  tests; text output reaches a `FrameSink`.
- **2026-11-11 — 8254 read-back:** PIT count/status latches support single and
  multiple counters, status-first reads in all RW formats, programmed mode bits
  (including aliases), OUT and NULL COUNT. NULL COUNT follows the initial load
  clock and running Mode 2/3 reload boundaries; BCD counting remains unsupported.
- **Device fidelity:** PIT Modes 2/3 adopt complete reloads at period/half-period
  boundaries and isolate incomplete LSB writes. RTC periodic PF/PIE uses a
  rational 32,768 Hz divider and rounded-up PIT deadlines, including IRQ8 wake
  from HLT; periodic and calendar phases remain an explicitly documented
  approximation. CRTC indices decode five bits; cursor comparison uses the full
  16-bit address while scanout wraps at the CPU aperture by assumption. FDC
  IRQ6 and DMA requests obey DOR bit 3. With RDY active, SEEK succeeds with or
  without a disk or installed drive. RECALIBRATE reports equipment check with
  PCN=0 only for absent drives; installed empty drives complete normally and
  report T0 at PCN=0 while disk change remains asserted. Without a disk,
  ID/data searches wait for reset without an interrupt. SENSE DRIVE STATUS
  (04h) accepts the drive/head parameter and returns ST3 without raising or
  acknowledging an interrupt or changing the present cylinders. ST3 echoes
  unit/head, reports two-sided geometry, installed-drive track 0 and media
  write protection, and keeps READY active for every select. ASSUMED: no disk
  clears TS/WP; absent drives also clear T0. FT is always clear because drive
  faults are not modelled.
  ASSUMED for silicon: SEEK/RECALIBRATE completion supersedes unread reset
  polls (Bochs-style, limited to seek and recalibrate completion). MAME instead
  retains pending ST0 per drive and senses the lowest pending drive; NEC µPD765
  and Intel 8272A SENSE INTERRUPT STATUS descriptions do not define the interaction.
  SIS reports the pending seek/recalibrate completion and then idle 80h.
  All four C0h–C3h/00h reset polls remain available until a seek/recalibrate
  completion supersedes them. Overlapped seek completions are not modelled:
  one latch holds the latest completion, even across drives.
  CCR writes at 3F7h select 500/300/250 kbps (00/01/10); reserved 11 matches
  no medium. READ ID and READ/WRITE DATA
  report abnormal termination with ST1 missing-address-mark when the selected
  rate differs from the rate the medium answers at in its drive: 500 kbps for
  1.44/1.2 MB, 250 kbps for 720 KB and for 360 KB in a 360K drive, 300 kbps
  for 360 KB in a 1.2M drive. Fault result/IRQ occurs on the final command byte, without DMA
  or rotational delay; DIR reads and seek/sense commands are unchanged.
  IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
  Diskette Control Register (3F7h), does not specify its reset value.
  ASSUMED: device reset selects 500 kbps; DOR reset preserves the adapter
  rate. Snapshot v4 stores it and restores v3 using 500 kbps. Drive and
  medium types are separate (see the 2026-11-15 entry); rate-dependent
  transfer timing is not modelled.
  WRITE DATA reports not-writable before rate or CHS errors on protected media.
  Per-device module matrices record the remaining unsupported cases and
  assumptions; changed expectations are listed in
  [scheduler adoption](design/scheduler.md#adoption).
- **AT fixed-disk adapter registers:** `ide` cards accept
  `controller_model = "at-adapter"` on every board, keeping task latches
  accessible without a disk (including cylinder-low 55h/AAh echo). Port 1F1h
  reads Error and writes Precompensation. IBM's adapter reference, **Command
  Register** (p. 7), inhibits drive commands without readiness and clears IRQ14
  on every command write; **Diagnose** (p. 10) runs controller self-tests and
  interrupts even without a disk, returning error 01h. Status acknowledges the
  shared controller IRQ (pp. 5–6); write-only 3F6h reads FFh (p. 11 I/O table).
  ASSUMED exact empty-drive idle status: 00h. Snapshots carry an explicit model
  tag and preserve pending diagnostic IRQs without media. Default `"ata"` and
  historical presets retain their behaviour. The adapter is opt-in alongside
  the FDC, which owns all 3F7h bits; MFM and full WD1003 commands/timing remain
  unmodelled. See
  [machine-config.md](design/machine-config.md#the-fixed-disk-idehdd-attachment-seam)
  for reference sections and coverage. MEASURED externally: an empty adapter
  removes POST error 601 with an original IBM 5170 BIOS; this worker checks the
  register probe using synthetic firmware rather than that BIOS.
- **Open FreeDOS installation media:** `vendor/freedos/fetch.sh` builds the
  pinned `freedos-tools-1200.img` (1.2 MB FAT12) with FDISK, FORMAT, SYS,
  XCOPY, EDIT, MEM and CTMOUSE. FDISK needs 186/286 or later, not 8086;
  CTMOUSE needs a 286 or later, so the complete tools set requires a 286.
  Package versions, licences and compatibility evidence are in `vendor/freedos/README.md`; a local-package
  mode rebuilds it without network access from the verified boot fixture.
  `freedos_install_golden` creates a blank 32 MiB disk through `media create`,
  partitions it as active FAT16, warm-starts through guest Ctrl+Alt+Del, runs
  FORMAT with system transfer, ejects A: and warm-starts again to boot C:.
  Before combining the paths, CPU resets with installed-empty-drive behavior
  ended at screen FNV-1a `8f1e57cd86975e25`, 104,210,281 retired instructions
  and virtual time 108,800,000. Guest warm starts before that drive correction
  ended at `5d0c917f3ab08d17`, 98,075,818 instructions and virtual time
  100,800,000. **MEASURED with both changes:** the final C: prompt has screen
  FNV-1a `040490b8724fbaba`, 99,077,816 retired instructions and virtual time
  101,800,000, with the same open `at-ide.toml` firmware and CPU pin `fe240a56`.
  The new pin combines guest Ctrl+Alt+Del POST with successful recalibration
  and T0 on the installed empty A: drive after eject, which reaches READ ID
  instead of failing recalibration. Both POST work and the displayed DOS time
  differ from the earlier baselines.
  MEASURED on the CPU-reset path: normal empty-drive recalibration reaches READ ID;
  without index pulses the BIOS exhausts one IRQ wait (726,351 virtual steps)
  and three command-ready waits (786,864 total). NEC uPD765A datasheet,
  READ ID command and Status Register 1 (MA): an ID search ends only on an ID
  field or after two index pulses. Intel 82077AA datasheet, §8.1 Command and
  Result Phase Handshaking, corroborates the required software timeout. The
  motor start is unchanged; the first C: boot read moves 1,507,019 virtual
  steps later, rounded to two prompt-polling chunks at the endpoint. XTIDE now
  sees timeout 80h and omits the old error 40h line; the retained top banner
  and later RTC time change the screen hash. With guest warm starts, the
  correction instead moves the endpoint by one polling chunk: 1,000,000
  virtual steps and 1,001,998 retired instructions beyond the warm-start
  baseline. The polling cadence and POST path determine the combined endpoint.
  Omitting FORMAT makes the same C: boot fail at the partition boot signature
  check. Writes stay in a hash-bound journal; the base remains unchanged.
  See `machines/README.md` §Playing DOS games for the same installation steps.
- **IDE PIO:** one primary (1F0h/3F6h, IRQ14) or secondary (170h/376h,
  IRQ15) channel assembles from Rust configs and TOML `ide` cards. Master/slave
  fixed disks use explicit CHS geometry and volatile/readonly/journal `SectorMedia`.
  Primary IDE splits the FDC decode at 3F6h. An assembled Bare rig transfers all
  256 IDENTIFY words with `rep insw`; firmware boards require a ROM declaring
  fixed-disk services, even for an empty IDE card. The fixture recipe retains
  the AT build and adds configured XT+ (`xub-r638-xtp-ata1f0`), using 16-bit
  ATA polling. `machines/at-ide.toml` loads XT+ at C8000h with a readonly
  615/4/17 FAT16 master. **FreeDOS now boots from A: and lists C:** the separate
  Kiselev `bios-xi8088-atreset.bin` accepts XUB's DL=80h reset request using
  IBM AT semantics. The upstream BIOS remains unchanged for existing machines.
  The ignored `ide_freedos_golden` types `DIR C:` through the keyboard and
  checks label `UNEMUHDD`, `MARKER.TXT` (29 bytes), the completed prompt,
  screen hashes and POST. Three runs before pinning matched: 14,777,577 retired
  at the first shell prompt, 17,748,107 after DIR, final screen FNV-1a
  `ef5a10891b9ef196` (rechecked unchanged at CPU pin `a92b676`, including the
  corrected FDC INT acknowledgements). XT+ detects `UNEMU ATA-2 PIO DISK`.
  No emulator device or CPU fix is included. The golden rejects missing XUB,
  a byte data window, a renamed marker
  in a copied HDD, and the original BIOS; disconnecting IRQ14 preserves its
  pins because this build polls. CMOS
  fixed-disk types follow the provider: option ROMs (including XUB)
  leave 12h/19h/1Ah zero; a hash-matched system BIOS that provides fixed-disk
  and declares `cmos-drive-types` derives IBM AT types 1–14/16–22 and the
  checksum. Unsupported geometries are refused rather than inventing a type-47
  convention. Kiselev declares no such capability; existing seeds are unchanged.
  Catalogued BIOS service
  declarations must match the catalogue, and omitted declarations inherit it. ATA write batches preserve complete sectors on interruption and
  reject readonly writes with ERR/ABRT. ATA submits completed commands independently of host pacing. Assembly flushes
  in-flight prefixes only for finish, CPU reset, run errors and explicit
  snapshots/checkpoints; partial sectors stay in controller state. Media capacity is checked at
  construction. Primary/secondary IDENTIFY interrupts reach the cascaded PIC.
  Existing ATA behavior tests are unchanged.
- **Floppy and IDE write modes:** volatile is the default (in memory only), readonly
  rejects writes, and explicit journal mode persists command-complete writes
  beside an immutable file-backed base and replays them on the next mount.
  Journal selection is available in Rust `MachineConfig` data and TOML/YAML
  floppy and IDE drive tables, with explicit per-layer journal paths and positive
  retention counts (default 10). Changing a drive image drops inherited journal
  settings while preserving readonly/volatile modes; a child can turn journaling
  off without repeating the image. Journal outputs cannot alias loaded images or
  machine-file layers, with source-positioned refusals before assembly.
  Fixture machines keep their existing modes; persistence goldens use temporary journals.
  NVRAM journals persist changed CMOS RAM (0Eh–3Fh except 0Fh) only before CPU
  reset and at finish. Reset records keep one session generation open; finish
  closes it and runs retention. Run stops and host frame slices perform no NVRAM
  I/O. The journal codec, recovery and offline tools are shared with drives.
  TOML/YAML accept explicit journal paths and retention with drive-style image
  override units. Readonly NVRAM is rejected with a volatile-mode hint because
  the MC146818 has no write-protect input. IDE shares the drive journal format and
  offline tools (512-byte sectors; capacity from the base image length). Offline CLI
  `media generations` and `verify` inspect under shared locks, with an explicit
  no-lock report when read-only storage has no lock entry;
  `verify --repair-tail` and `rollback --to N` require synchronized full backups and
  exclusive locks. `media commit`/`compact` exports a verified immutable image;
  configured retention keeps a verified anchor and the requested closed history.
  Acceptance tests cover full volatile/journal guest results and replay,
  failure isolation and rollback through fresh guest DMA. The ignored fixture
  guard checks pinned hashes and vendor directory entries across volatile
  BIOS/FreeDOS/DOS-write runs; golden expectations remain unchanged.
  macOS/APFS sync/lock/publication paths were measured; Linux storage behavior
  and physical power-loss durability remain assumptions under the documented
  storage contract. See [persistence evidence](design/persistent-media.md#acceptance-evidence-appendix)
  and the [runnable walkthrough](commands.md#journal-session-walkthrough).
  Closed/recovered markers do not imply a healthy filesystem.
- **Machine files in frontends:** TOML/YAML, inheritance and embedded presets
  feed the window and CLI through `machine-file`, with source-positioned
  assembly catalogue diagnostics. `machine check FILE` validates and assembles
  without guest execution or journal writes (exit 0/1/2). Missing, recoverable
  and busy journals produce notes/warnings; corruption or a wrong base is invalid.
  `run --machine FILE` accepts retired/idle caps and a display/UART card id for
  final output. Normal halt/budget exhaustion finishes persistence; failed runs
  leave unsealed commands for the next mount.
  The window defaults to the build workspace's `machines/at-freedos.toml`,
  chooses the first display or `--display CARD`, and no longer accepts positional
  BIOS/floppy paths. FreeDOS goldens load that same file; BIOS goldens and
  synthetic fast tests keep their presets.
- **Machines are declarative.** A `MachineConfig` (the `assembly` crate) is built by one
  `assemble()` and run under a `RunPolicy`; presets replace the old hand-written runners.
- **CPU pin:** `060d446a883a9b2fc94b3ef91258cb4e6e98d916` (x86jit `main`).
  READ: real-mode I286/I386 machine execution now accepts the Application Real16
  instruction set, including AAD/AAM, BCD/SALC, x87 with MSW.EM/TS checks and
  resumable REP INS/OUTS. I286 rejects 386-only 66h/67h forms with #UD.
  IDE and Landmark pin assertions name this revision. Open-firmware goldens
  use the measured I286/IVT baseline below; proprietary runs remain unmeasured.
  Historical validation by the supervisor at `44403dd` on 2026-11-11: the fast
  suite, slow set and user-ROM lane passed with unchanged golden values on
  Application Real16. At `a92b676`, 1,666 fast tests, 611 release device tests
  and all existing slow golden expectations passed.
  The `44403dd` update added `xj-srh.11 (x86jit)` A20 gating for LegacySystem.
  Board A20 integration now forwards the retained shared line into this CPU
  profile; the accepted no-media bare POST re-baseline is recorded above.
- **CPU:** `ibm-5170` with model `80286` selects the I286 machine profile, with
  real-mode entry, LMSW protected entry and same-CPL 16-bit protected execution.
  `set_cpu_model(I80286)` fixes the VM model and defaults to InGuest delivery:
  real exceptions use the IVT; CPL-0 protected interrupts, exceptions, IRQs,
  TF and IRET use the IDT. LDT selectors and descriptor-query instructions are
  available. READ: `xj-srh.18 (x86jit)` adds CPL-0 hardware task switches
  (TSS/task-gate JMP/CALL, IRET with NT, IDT task gates), busy bits and MSW.TS
  under `decision-24 (x86jit)`; privilege transitions and V86 remain unsupported. Explicit
  outward faults retain typed vector/error-code/CS:IP diagnostics. A fault
  during #DF delivery requests the same CPU-only reset as the 8042, retaining
  RAM, CMOS, the A20 latch, devices and accounting. The standalone Real16 constructor
  and source-free bare rigs retain Application Real16; bare rigs with an A20
  source select I80286 with low reset fetch and in-guest processor exceptions.
  Both profiles have exact step bounds,
  per-iteration REP time, STI/SS-load inhibition and interrupt-readiness stops.
  Real16 INS/OUTS support byte/word elements, REP, OUTS segment overrides,
  and IRQ delivery between elements; answered INS stores retain their value on retry.
- **Scheduler:** deadline dispatch is the sole production engine. INTA follows
  completed instruction bus/DMA effects and only occurs when the CPU can accept
  it. Active caps are exact; HLT jumps to the next event or bound. The per-clock
  reference is test-only (`reference-scheduler`); config has no timing or block
  quantum selection. The Long64 machine demo/blob path is intentionally retired.
- **Observation:** screens come from the text card's scanout state, COM1 and POST
  bytes through byte sinks; access traces are off in production and opt-in as a
  bounded ring with drop counts. Snapshots carry each device's time base and
  reject malformed input.
- **Optional window presenter:** the `window` leaf crate embeds the original
  VileR v2.2 CP437 MDA/VGA fonts (CC BY-SA 4.0), parses NE/FNT resources and
  rasterises decoded text styles to softbuffer pixels. `unemupc98-window` boots
  `machines/at-freedos.toml` with paced runs and observation subscribers; it retains
  the last delivered frame while continuing past date/time prompts. Physical
  host keys map to set-1 make/break bytes, including E0 navigation and right
  modifiers, queued only at run boundaries with backpressure. Presenter,
  keyboard and FreeDOS input tests run headlessly. The maintainer confirmed the
  FreeDOS prompt, font and colours in a window. MDA underline uses row 12; cursors use BIOS default shapes rather
  than programmed CRTC start/end registers. HLT waits for IRQs within run caps.
  The event loop alone reads a host clock: integer-rational PIT targets use the
  configured CPU rate, 60 Hz paints and a 50 ms guest-work bound per paint.
  Excess backlog is discarded; `WaitUntil` idles between paints. Occlusion uses
  `Wait` and pauses the guest; resuming discards the hidden interval. Fake-clock
  synthetic and fixture-gated FreeDOS tests replay accepted input at recorded
  virtual times with different headless chunking and compare MDA/time/device
  state, CPU registers/retirement and conventional RAM hashes. Blink remains steady; its phase can later derive from virtual time.
  Pause/Break and PrintScreen are unmapped. Typematic uses host repeated presses.
  Manual desktop acceptance passed for date/time input, `c:` and `dir`.
  E0 navigation/Divide use IBM artificial Shift wrappers on first make/final
  break; repeat makes are plain. Encoding at boundaries follows guest LED NumLock
  state, preserving navigation through Ctrl+NumLock and reboots. The 8042
  snapshot v4 stores LEDs and pending LED parameters; reset clears both.
  Focus loss releases held keys. macOS winit 0.30 does not deliver CapsLock.
- **Tests:** MEASURED 2026-11-10 at `479f8ed`: `cargo test --workspace` 610
  passed, 0 failed; `cargo test --release -p devices` 358 passed; the 13 slow
  goldens and engine comparisons pass (BIOS POST, floppy boot, FreeDOS prompt,
  `DIR`, file write, per-clock versus deadline on every workload with final
  state); clippy for the full and the production feature sets, fmt, publication
  guard and cargo deny are clean; `cargo doc` has no warnings. Forgejo CI runs
  the same gate. See `commands.md`.
- **Dependency gate (2026-11-11):** the all-features `deny` lane also checks
  RustSec advisories. Vulnerabilities and yanked crates fail; unmaintained crates
  require a dated, justified ignore with a review date. RUSTSEC-2026-0192
  (`ttf-parser` 0.25.1, unmaintained, not a known vulnerability) is accepted to
  retain winit's Wayland client-side decorations; review on 2027-05-11.
- **Speed (MEASURED 2026-11-11):** the release BIOS POST, FreeDOS boot and
  `DIR` test has a median wall time of 0.436899 s at `a92b676`, versus
  0.449007 s at `8979fc9` (three alternating pairs, 2.7% less time under shared
  host load). Current-pin sample shares await the supervisor; see
  [`design/machine-time-profile.md`](design/machine-time-profile.md).

- **CPU warm restart:** 8042 reset requests restart the CPU at F000:FFF0 after
  the persistence boundary check. CLI natural, prompt, post-prompt and
  machine-file runs, and the window, resume with 80286 reset registers and
  cleared execution latches. A reported reset remains pending until performed;
  direct machine resumption resets before execution, including after a host
  persistence failure. Window reset diagnostics count only performed resets.
  RAM, CMOS shutdown byte 0Fh, devices, the 8042
  output-port/A20 latch, retirement counters and virtual time survive. Reset
  does not end a journal session. I286 reset restores MSW/IDTR and real caches,
  including hidden CS base FF0000h; the system ROM has a 24-bit reset mirror.
  A20 address masking follows the shared board line; see
  [machine-config.md, Board model](design/machine-config.md#board-model).

### User-supplied system ROM catalogue

- **2026-11-11 — User-ROM test lane:** shared test-only SHA-256 discovery accepts
  arbitrary filenames in `vendor/user-roms/` and optional `UNEMUPC_USER_ROMS_DIR`,
  including nested chip layouts or assembled images. `ci-gate.sh user-roms` is a
  local opt-in lane; absent identities skip visibly unless
  `UNEMUPC_REQUIRE_USER_ROMS=1`. CI never calls it. Synthetic tests cover
  discovery and policy, including symlinks and loops, external-root configuration
  failures, partial-layout counts and downstream path redaction. External roots
  must be absolute folders of unpacked dumps. Proprietary firmware execution
  remains limited to the measurements below.

IBM 5170 BIOS versions 1984-01-10, 1985-06-10 and 1985-11-15 are Listed by
assembled SHA-256. Single interleaved images and even/odd parts (including
both 1984 chip layouts) support identity checks, TOML/YAML inheritance,
per-part pins and chip diagnostics (including partial pair swaps and exchanged
halves). Child source overrides drop stale service declarations; known images
use catalogue services. No proprietary bytes are distributed.
No default firmware changes. **MEASURED at CPU `fd67927` with locally supplied
firmware:** the 1985-06-10 BIOS passes POST cleanly and boots FreeDOS from a
1.2 MB floppy to the DOS prompt.
This does not qualify other BIOS revisions or broader guest compatibility.
`user_rom_ibm_5170_1985_post` uses I286 with MDA, `display_switch='auto'`, an
`at-adapter` fixed-disk controller, read-only `freedos-1200.img`, derived
volatile CMOS and a pinned RTC. It waits for FreeCOM's date prompt, sends Enter
for date and time, then polls for `A:\>` or `A:>`. Reports contain a safe stop,
port-80h checkpoints, retirement count, DOS-prompt verdict and final MDA-plane
SHA-256, without screen text. Regression expectations pin that measurement;
identity tests remain available.
See `vendor/user-roms/README.md` and `vendor/freedos/README.md`.

## Port 61h timing re-baseline (2026-11-11)

Port 61h bits follow the PIT; delay loops in the Kiselev BIOS now take real
refresh time. Bit 4 toggles on PIT1 OUT rising edges, bit 5 samples OUT2, and
bit 0 drives GATE2. The shared PIT snapshot owns the refresh flip-flop and gate.

**MEASURED by the supervisor:** the IBM 5170 BIOS speed/refresh check passes
at rates `4/7` through `2/3`; `11/20` stops at checkpoint `04`, and `27/40`
stops at checkpoint `11`. The 6 MHz `ibm-5170` default is **8/13 steps per PIT
clock**. This applies to Rust presets, machine-file board defaults and the
shipped FreeDOS TOML/YAML files; IDE inherits the FreeDOS rate. Explicit rates
remain authoritative. Bare rigs retain their own rates, including 1/1 for the
historical no-controller POST rig. See
[machine configuration](design/machine-config.md#cpuconfig-and-clockconfig--minimal-but-the-right-home).

**MEASURED comparison against main:** the old baseline is main `889601b`,
with CPU `44403dd85077c0db29ad09780003ebd3b193ecdd`, per-read refresh and rate 1/1.
The new side uses the same CPU pin, PIT-driven status and calibrated AT rate.
No intermediate 1/1 refresh baseline is used in the table below. Complete MDA
and colour dumps, COM1, POST, cursors and final data were compared. Every
logical screen matches main after replacing only displayed RTC/time fields.
There are no unexplained category (d) changes: (a) guest stages occur at different
steps, (b) guest clock text advances differently, and (c) safety/command windows
must reserve enough time for completed commands rather than stop early.

Goldens again use the shipped CLI BIOS/FreeDOS runners. Their production boot
and POST caps are 20,000,000 steps, with a 3,000,000-step command window.
At 8/13 the date prompt is first observed at the 10,000,000-step poll; DIR and
write/DIR finish their windows at 13,000,000, leaving seven million steps before
the safety cap (one more prompt poll plus six million of margin). Poll cadence
remains one million steps. IDE keeps its 100,000,000 safety cap, three-million
command windows and 40,000,000 DIR completion bound. The trace-overflow test
also asserts completed command output, final prompts and successful stop kinds.
The shared test helper contains passive screen assertions only, not execution
policies, captures or a duplicate floppy reader.

**MEASURED final endpoints:** R is CPU retired instructions; T is virtual
instruction-equivalent time, including halted time. Hashes are FNV-1a over all
25 canonical MDA rows. Exact raw hashes and counters are pinned separately from
whole-console logical hashes, which replace only `RTC (Real Time Clock):` and
`Current time is` fields. Both scheduler engines match complete access/state
streams at the production-policy endpoints.

| Golden / endpoint | Category | Main R -> new R | Main T -> new T | Main MDA hash -> new MDA hash |
| --- | --- | --- | --- | --- |
| BIOS POST; scheduler BIOS POST | a/b/c | 9,015,022 -> 19,015,022 | 10,000,000 -> 20,000,000 | `cf7334eb70c4c237` -> `809829e50bc06fbd` |
| BIOS floppy boot | a/b | 10,996,320 -> 18,993,723 | 12,000,000 -> 20,000,000 | `ba49c0fe9e88293e` -> `0744aaf03bcc6424` |
| Scheduler BIOS floppy | a/b | 8,996,320 -> 18,993,723 | 10,000,000 -> 20,000,000 | `ba49c0fe9e88293e` -> `0744aaf03bcc6424` |
| FreeDOS date prompt; scheduler prompt | a/b | 7,802,475 -> 8,802,475 | 9,000,000 -> 10,000,000 | `1da6d49437180ba0` -> `526086b917baa58e` |
| DOS DIR; both scheduler engines | a/b/c | 9,774,929 -> 11,774,928 | 11,000,000 -> 13,000,000 | `56d8215daf926acd` -> `bb926924f4d7efa1` |
| DOS write; both scheduler engines | a/b/c | 9,768,598 -> 11,768,597 | 11,000,000 -> 13,000,000 | `616104befb7e4588` -> `153da3e002975834` |
| IDE initial A: prompt | a/b | 14,777,577 -> 15,777,577 | 16,000,000 -> 17,000,000 | `922f327d9dd4db6b` -> `49bbec2d36c0bbf0` |
| IDE DIR C: and final prompt | a/b | 17,748,107 -> 19,748,107 | 19,000,000 -> 21,000,000 | `ef5a10891b9ef196` -> `fb811e1f40836da7` |
| Media fixture BIOS | a/b/c | not pinned | 10,000,000 -> 20,000,000 | `cf7334eb70c4c237` -> `809829e50bc06fbd` |
| Media fixture FreeDOS prompt | a/b | not pinned | 9,000,000 -> 10,000,000 | `1da6d49437180ba0` -> `526086b917baa58e` |
| Media fixture write | a/b/c | not pinned | 11,000,000 -> 13,000,000 | `616104befb7e4588` -> `153da3e002975834` |

Final cursors match main: BIOS failure `(0,18)`, boot-sector console `(0,17)`,
date prompt `(31,24)`, completed commands `(4,24)`. Colour text remains blank.
BIOS clock text changes from `0000-01-01 00:00:01` to `0000-01-01 00:00:03`;
floppy DOS time from `12:00:07.41 am` to `12:00:12.90 am`; IDE time from
`12:00:11.36 am` to `12:00:18.12 am` (b). Dates, banners, directory and volume
identities, file/free-space counts and final prompts match. DIR lists KERNEL
and COMMAND; the write session adds ZZ with `x\r\n` (three bytes) verified
from volatile floppy media; IDE DIR C: lists the 29-byte MARKER.TXT.

POST remains `01 02 03 04 05 06 07 08 09 10 12 20 21 22 24 25 30 31 40 43 00`.
IDE retains `41 42` between `40` and `43`. COM1 remains empty except for the
boot sector's `B`. Fixture SHA-256 hashes and vendor namespaces remain intact.

**DERIVED / MEASURED calibration:** including `DEC BL / JNZ`, iterations per
complete refresh-bit cycle are `n = (36R - 2)/3`. Centring 256 cycles at 1710
iterations gives `R = (3 × 1710/256 + 2)/36 ≈ 0.6122`. The synthetic guest
counts 2903 iterations at 1/1 (CX=`F4A9h`) and 1721 at 8/13 (CX=`F947h`, inside
the BIOS acceptance range). Actual firmware window measurements above belong
to the supervisor; complete proprietary POST/boot compatibility remains
unqualified. See [scheduler calibration](design/scheduler.md#device-migration-inventory).

## I286 CPU-probe and exception-delivery re-baseline (2026-11-12)

**MEASURED against main `5931d32`, CPU `44403dd`:** the new CPU pin `8bad85d`
and `set_cpu_model(I80286)` take the BIOS single-step CPU probe traps per the
Intel 80286 Programmer's Reference Manual §9.7.1 and Appendix B, and deliver
real-mode exceptions through the IVT. This is the approved reason for the
following golden changes. The synthetic boot sector reaches the normal
20,000,000-step cap through the BIOS INT 13 handler rather than stopping at
the segment-limit exception. Exception delivery itself retires no instruction.

Full raw MDA and colour dumps, POST and COM1 were compared without normalizing
away text differences. The sole visible change is:

- `Main Processor:             Harris-designed 80C88 @ 4.77 MHz` ->
  `Main Processor:             Intel 8088 '78 @ 4.77 MHz`;
- the added line `WARNING: This CPU does not disable interrupts after loading segment registers!`;
- following rows move down one; the final blank row leaves the 25-row display.

This remains visible only in the floppy-boot endpoint. DOS and IDE scroll it
off the screen, leaving their final screens byte-identical. The historical
no-controller BIOS POST rig is Bare/Application Real16 and stays unchanged.
All guest-rendered clock lines, boot messages, directory entries, prompts and
file contents match main. T is virtual instruction-equivalent time, R is
retired instructions, and hashes are FNV-1a over 25 canonical MDA rows.

| Endpoint | Main R -> I286 R | Main T -> I286 T | Main MDA hash -> I286 hash | Main cursor -> I286 cursor |
| --- | --- | --- | --- | --- |
| Bare BIOS POST | 19,015,022 -> 19,015,022 | 20,000,000 -> 20,000,000 | `809829e50bc06fbd` -> same | (0,18) -> same |
| BIOS floppy boot | 18,993,723 -> 19,008,614 | 20,000,000 -> 20,000,000 | `0744aaf03bcc6424` -> `938e07668f3b0b34` | (0,17) -> (0,18) |
| FreeDOS date prompt | 8,802,475 -> 8,800,477 | 10,000,000 -> 10,000,000 | `526086b917baa58e` -> same | (31,24) -> same |
| DOS DIR | 11,774,928 -> 11,772,930 | 13,000,000 -> 13,000,000 | `bb926924f4d7efa1` -> same | (4,24) -> same |
| DOS write/DIR | 11,768,597 -> 11,766,599 | 13,000,000 -> 13,000,000 | `153da3e002975834` -> same | (4,24) -> same |
| IDE initial A: prompt | 15,777,577 -> 15,775,579 | 17,000,000 -> 17,000,000 | `49bbec2d36c0bbf0` -> same | (4,24) -> same |
| IDE DIR C: and final prompt | 19,748,107 -> 19,746,109 | 21,000,000 -> 21,000,000 | `fb811e1f40836da7` -> same | (4,24) -> same |

The floppy logical hash changes `69d62210a69ffe15` -> `92c581d95567fb07`;
all other logical hashes are unchanged. Both scheduler engines use the same
new floppy screen baseline. Colour text stays blank with visible cursor (0,0).
MDA cursors remain visible. POST remains
`01 02 03 04 05 06 07 08 09 10 12 20 21 22 24 25 30 31 40 43 00`;
IDE retains `41 42` between `40` and `43`. COM1 remains empty except for
floppy boot's `42` (`B`). No firmware or media fixture is changed.

IBM POST and the Landmark full-pass golden now pin their measured regression
values. The Landmark values below come from the supervisor's user-ROM run,
not these open-firmware runs.

## IRQ input level golden baseline (2026-11-12)

**MEASURED against main `3f529bf`:** the BIOS floppy endpoint changes from
R **18,991,006** to **18,990,963** at unchanged T **20,000,000**. The old model
acknowledged one stale keyboard vector **09h at PIT clock 4,896,203**, after POST
read FAh from port 60h at **4,896,116**. The new model withdraws that request when
the line falls, as Intel 8259A datasheet, **Edge and Level Triggered Modes**
requires. Main has 427 acknowledges; corrected inputs have 426, with identical
remaining vector order. Subsequent acceptance times can shift because the stale
handler no longer executes. Restoring only stale IRQ1 latching reproduces all
427 vectors and timestamps and the old retired endpoint, isolating the cause.
The trace's IMR is still its initialization value at the polled read; the first
later OCW1 is B8h at 4,896,164. This is withdrawal before CPU acceptance, not a
change in keyboard transfer timing or PIT arithmetic.

Both MDA raw hash `9a13654e51c09b99` and logical hash `92c581d95567fb07` are
unchanged. MDA cursor stays visible at (0,18); the blank colour screen keeps raw
and logical hash `c1fdc54ce4885c65` and visible cursor (0,0). Only the retired
count expectation in the floppy golden is updated. The other endpoints retain
their main baselines.

PIC snapshots now use version 3 and reject pending IRR bits on low inputs
atomically; version 2 is rejected because it allowed requests to outlive their
input levels. A low input may still have an ISR bit from an earlier INTA.

## Empty 80287 socket integration (2026-11-12)

ibm-5170 has an empty 80287 socket by default (maintainer decision 2026-11-12);
the core reports no coprocessor (x86jit decision-25).
The pin is `060d446a883a9b2fc94b3ef91258cb4e6e98d916`. The choice is carried by
`cpu.fpu = "none"` (default) or `"80287"`, and derived CMOS 14h combines the
selected display bits with coprocessor bit 1 and the drive inventory. Explicit
NVRAM images retain their contents.

**MEASURED:** the synthetic machine-file guest reads FFFFh from FNSTCW and
FNSTSW m16/AX with no FPU, completes WAIT and reaches HLT. Selecting `"80287"`
keeps the generic present engine, with control word 037Fh and CMOS bit 1 set.
The isolated RTC test checks MDA equipment bytes 31h/33h, colour bytes 21h/23h
and the AT checksum.

**MEASURED:** a paired Kiselev BIOS floppy run with Present and None reaches
the normal 20,000,000-step boundary with byte-identical raw screens. The exact
coprocessor report is unchanged:

- `Mathematics Co-processor:   Absent` -> `Mathematics Co-processor:   Absent`.

**READ:** this BIOS first compares the stored control word with 03FFh;
the generic Present engine returns 037Fh, so it already takes the absent branch.
The empty socket's FFFFh takes that same branch. This observation does not
qualify generation-specific 80287 behaviour or the board's FERR#/IRQ13 wiring.
`decision-24 (x86jit)` / `xj-srh.18 (x86jit)` additionally provide CPL-0 task
switches; that core capability is source-inspected, not a new guest-OS claim.

**MEASURED final validation:** 1,911 fast tests, 694 release device tests and
22 slow tests passed. Build, Clippy, formatting, publication guard and rustdoc
with warnings denied passed. All existing golden text/logical hashes,
retired/virtual counts, POST streams and cursor expectations remain unchanged;
no golden re-baseline was needed.

## In flight

- **2026-11-12 — Device IRQ levels and PIC withdrawal:** the machine forwards
  both device pin transitions at existing scheduler sampling points. Falling
  inputs withdraw unacknowledged edge-triggered IRR bits, including masked
  requests and slave INT on master IR2. Explicit empty INTA supplies default
  IR7 without setting ISR7; normal poll/INTA remains atomic. PIT already exposes
  sustained OUT0 in Modes 0/2/3; no timer pulse synthesis remains in the wiring.
  8042, FDC, ATA and RTC retain their held-output acknowledgement rules; UART
  IRQs and ICW1 LTIM remain unmodelled. See
  [scheduler interrupt contract](design/scheduler.md#pic-state-must-follow-cpu-acceptance).

- **2026-11-12 — AT primary-display configuration:** board display selection now
  derives the 8042 C0h input bit 6 and CMOS 14h video bits from the installed
  cards. Auto prefers MDA, then colour; `board.display_switch` accepts
  `auto`, `mono` or `colour`, with unavailable adapter classes rejected.
  ROM-less colour derives 80-column colour; a video ROM at C0000h selects
  own-BIOS configuration. No adapter retains CMOS 00 and the colour switch
  as an explicit compatibility policy. Explicit NVRAM is not rewritten.
  Kiselev profiles explicitly select colour: that BIOS interprets the AT switch
  with inverted polarity and uses MDA there, while derived CMOS video bits stay
  colour (10b). Other firmware retains automatic selection.
  Synthetic isolation, assembly and TOML/YAML tests cover these paths. The
  goldens did not change, and the user-firmware lane passed (supervisor-measured).
  See [primary display](design/machine-config.md#primary-display).

- **2026-11-11 — A20 / System Control Port A device side:** Bare rigs can
  opt into port 92h with versioned snapshots, latch readback and CPU reset on
  bit-0 0→1; readback remains set after CPU reset until cleared by a write or
  system reset (IBM ThinkPad 560 Technical Reference, "System Control Port A
  (Hex 0092)"; Intel386 EX User's Manual, PORT92, CPURST). Bit 1 is zero after
  system reset. Pulse timing and ordinary security/light latches remain ASSUMED;
  real bit-3 writes are permitted only while zero, an unmodelled restriction.
  Bit 4 is read-only watchdog-timeout status, modelled as zero (Bochs PORTS.LST,
  port 0092). The 8042 and 92h share one reset line and one observable A20 line;
  effective A20 is their logical OR (ASSUMED board policy). The enabled 8042
  prevents SeaBIOS from disabling A20 through 92h alone; `upc-o0j.6` must choose
  arbitration and test INT 15h AX=2400h/2402h. Attaching the 8042 starts the
  change counter at 1; no source reports disabled despite flat CPU addressing.
  The CPU wiring follow-up below retains those source defaults and counter baseline.
  IBM 5170 rejects port 92h. See
  [machine-config.md §A20 line and System Control Port A](design/machine-config.md#a20-line-and-system-control-port-a).

- **2026-11-11 — Firmware-owned QEMU CMOS layout:** opt-in catalogue capability
  and byte-exact synthetic tests derive RAM, century, boot order, type-47 IDE
  geometry, translation and single-CPU fields. No production entry enables it;
  the SeaBIOS entry will do so with `generic-386-isa`. The complete 128-byte
  derivation is available and Bare Rust rigs can expose it through a
  DS12887-compatible 128-byte RTC. The IBM AT retains its 64-byte MC146818 and version-4 snapshots;
  the extended part uses version 5. Restore rejects snapshots whose capacity
  differs from the configured part. NVRAM images/journals
  stay 64-byte only; machine files have no size selector. Board selection
  remains pending.

- **2026-11-13, diagnostic-ROM validation and full-pass golden:** user-supplied
  `landmark-rom-post-at-1.2.0` remains Listed for `ibm-5170`, with pinned
  assembled and U27/U47 hashes. **MEASURED by the supervisor at main `9bd422d`,
  x86jit `fe240a5`:** full diagnostic passes complete; every test passes except
  floppy read on a rig without a drive or medium. The earlier twelve-test
  Real16 prefix and unsupported-SIDT stop remain historical evidence.
  Catalogue-only, per-service direct-hardware-test permissions now allow this
  diagnostic's FDC with media and fixed-disk adapter without claiming boot
  services. Ordinary BIOS entries and unlisted images keep the provider rule.
  The new ignored `user_rom_landmark_at_full_pass` uses MDA, automatic display
  switch, a read-only 1.2 MB FreeDOS floppy, an `at-adapter` IDE card, fixed RTC
  and derived volatile CMOS. After the floppy-controller fix, the supervisor
  measured a passing floppy read and one complete pass before the 150,000,000
  cap. The ROM counts two reset-based returns from protected mode as errors per
  pass, matching its real IBM 5170 behaviour; the golden pins this state.
  **MEASURED by the supervisor and pinned:** stop `Ok(RetiredCapReached)` at
  `0004:0015`, retired instructions `138,208,231`, MDA text-plane SHA-256
  `fe45632b8a9c1b762af117b162f56adc523b8aa2e5bda990bad5adb7079e807a`.
  The existing diskless prefix configuration and constants remain unchanged.
  The capture records only stop metadata, retired count and the text-plane
  hash, with no screen text. Adapter gaps remain board-owned independently of
  the absent option-ROM scan window. Ruud XT v5.7 is a future `ibm-5160`
  fixture (`upc-fro.6`). See
  [compatibility diagnostics](design/compatibility-diagnostics.md#3-golden-capture-rule).

- **2026-11-11, fw_cfg device:** traditional read-only selector/data interface,
  inline file directory and explicit COM1 serial-console file are available
  through Rust assembly configuration on Bare rigs. Full-word selector decode
  at 510h preserves architecture keys; 511h is byte-wide. No DMA, machine-file
  syntax or firmware-board wiring; `generic-386-isa` remains pending. See
  [machine-config.md §Emulator-only firmware configuration](design/machine-config.md#emulator-only-firmware-configuration).

- **2026-11-11 — Register-level VGA:** isolated registers, DAC, planar memory,
  CPU read/write modes and version-4 snapshots (including ISA enable state and
  display latches, with version-3 migration). Register protocols, mode-3 logic,
  attribute widths and retrace status are checked against the IBM VGA/XGA
  Technical Reference Manual (May 1992), limited to the module coverage matrix:
  Figs. 2-19–2-26, 2-29–2-37, 2-58, 2-69–2-80, 2-81–2-89 and 2-94.
  Chain-4 offsets are DERIVED from Figs. 2-37 and 2-65. ASSUMED: floating ports
  and unselected memory read FF, 128 KiB mirroring, MSR[5] page polarity, DAC
  edge cases, odd/even gating of A0 replacement and the chain-4 pipeline.
  Alphanumeric rendering and opt-in `kind = "vga"` wiring now use guest fonts,
  CRTC geometry/start/stride/cursor and AC/DAC colours through `Frame::Text`.
  The existing text-only defaults and goldens are unchanged. No option ROM is
  installed by default; the user-supplied IBM video ROM is opt-in as described
  above. Graphics scanout is available, while
  SeaVGABIOS delivery via fw_cfg remains pending.

- **2026-11-11, SeaBIOS fixture:** pinned rel-1.17.0 ISA build recipe and
  configuration added under `vendor/seabios/`, with a separate firmware
  devShell. Two independent builds matched on macOS arm64 with the flake-pinned
  i686-elf GCC 15.2.0 and binutils 2.46 (MEASURED by the supervisor); the
  output SHA-256 is pinned. No board or catalogue entry is added;
  the entry waits for `generic-386-isa` and the x86jit 386 CPU model.
- **2026-11-11, phase B:** firmware decided (decision-19): unmodified SeaBIOS
  with emulator-only fw_cfg; requirements in
  [firmware-and-machine-classes.md §3.6](design/firmware-and-machine-classes.md#36-phase-b-firmware-requirements-seabios-on-generic-386-isa).

The 286 foundation milestone is closed. Next is phase A of decision-16 (the
display through a real MDA card to a window, host keyboard input): epic
`skeleton` in beads. The DOS-session profile records the current paired-pin
wall-time comparison and historical CPU sample shares.

## Broken / known defects

- **Coprocessor error/control wiring remains unimplemented.**
  `ibm-5170` now defaults to no coprocessor; machine files can select
  `cpu.fpu = "none"` or `"80287"`. Derived CMOS 14h bit 1 follows the choice,
  with its checksum updated; explicit NVRAM images are preserved. Bare rigs
  retain their present FPU. Under `decision-25 (x86jit)`, the I286 `None`
  policy accepts FNINIT and WAIT, returns FFFFh for FNSTCW m16 and FNSTSW
  m16/AX, and preserves other output destinations, subject to EM/TS/MP #NM
  controls. The `"80287"`
  option uses `Present`, with no generation-specific fidelity claim, and
  FNINIT/FNSTCW stores 037Fh. Socket ports still read FFh and ignore writes;
  FERR#/IRQ13 and F0h/F1h control wiring remain unimplemented.

- Multi-byte scalar accesses crossing a RAM/MMIO boundary in either direction
  ended the run with `Exit::UnmappedMemory` in historical x86jit
  `a92b676e081ce862dcb48c82645b3d3a8aa2364f` (READ). Its
  [`Memory::region_at`](https://github.com/unemu-org/x86jit/blob/a92b676e081ce862dcb48c82645b3d3a8aa2364f/x86jit-core/src/memory.rs#L2512-L2534)
  requires the whole transfer inside one region before read/write paths check
  for MMIO (`memory.rs:2730`, `memory.rs:2864`). Neither part of the scalar
  transfer is performed. RAM/ROM crossings, empty-bus fill included, are fixed
  at x86jit `060d446a` (MEASURED: guest boundary words complete). A span that
  includes an MMIO (Trap) byte keeps the old exit (READ, x86jit `060d446a`
  commit message). Assembly coalesces adjacent MMIO device windows so
  those crossings reach the bus, but mixed RAM/MMIO accesses remain unsupported,
  tracked as `xj-ir6.30 (x86jit)`.
- Journal selection is available in machine-file floppy tables. The CLI and
  window frontends do not load machine files yet; that integration is in progress.
  Immutable-image export (`media commit`/`compact`) and atomic retention pruning
  are implemented. Retention runs only after a durable closed generation, keeps
  a verified anchor bound to the original base, and reports maintenance failures
  separately from command persistence. Physical power-cut durability is assumed
  under the documented storage contract, not measured.

## Recently landed

- **Presentation outside machine identity:** `ObservationPolicy.output` now owns
  Headless/Capture selection, defaulting to Headless. `MachineConfig`, presets
  and machine-file lowering describe hardware only. Inline hosts retain
  `Observation::configure_output`; capture frames and boundaries are unchanged.

- **BIOS/FreeDOS console through MDA:** the profiles route `0xB0000..=0xB7FFF`
  and display ports `0x3B0..=0x3BB` / `0x3BF` to the MDA card, excluding printer
  ports. Console capture and prompt detection use its scanout. VGA has independent
  CRTC state; golden cursor expectations now reflect the two cards.
  Prompt polling and final capture share the observation facade by `DeviceId`.
  `ObservationPolicy.output` selects Headless or per-display Capture sinks; boxed
  `FrameSink` subscribers receive the same frames at the existing capture boundaries.

- **Interactive FreeDOS — keyboard to a running command** (`task-26`): a scancode
  script fed through the 8042 drives FreeCOM past its date/time prompts to the
  `A:\` prompt and runs `DIR`, listing `KERNEL.SYS` + `COMMAND.COM` — the full
  input path (scancode → 8042 → IRQ1 → BIOS INT 09h → BDA ring → INT 16h → DOS)
  end to end. Frontend-only driver (`run_freedos_interactive`); the models are
  untouched. Two genuine interrupt-model limitations were found and worked around
  in the driver (IRQ0 starving IRQ1 under a coarse quantum; the 8042 refilling its
  output buffer so IRQ1's level never re-edges) — faithful fixes tracked in
  `task-27`. Interactive golden reaches the prompt in ~25 s (`#[ignore]`).
- **🏆 FreeDOS boots to a shell prompt — the north star** (`task-20`–`task-25`):
  the unmodified Kiselev Xi8088 BIOS completes its entire POST, renders its
  hardware-detection screen via INT 10h, and its INT 19h loads a real **FreeDOS**
  8086 floppy over the modelled **µPD765 floppy controller + 8237 DMA**;
  `KERNEL.SYS` loads and inits and hands off to `COMMAND.COM` (FreeCOM), which
  reaches its prompt:
  ```text
  FreeDOS kernel 2044 (build 2044 OEM:0xfd) [compiled Jan 25 2026]
  FreeCom version 0.86 - WATCOMC - XMS_Swap
  Current date is Sat 01-01-2000
  Enter new date (mm-dd-[cc]yy):
  ```
  It then idles headless in the INT 16h keyboard wait. The whole stack runs under
  real firmware + a real OS. Key steps: x87 `FNINIT` lifted (`task-24`, cleared
  BIOS FPU detect); the µPD765 + a machine-orchestrated DMA-bus-master seam
  (`Cpu::guest_dma_write` → `note_write`, SMC-safe, `task-21`); BIOS ROM at
  `0xF8000` (`task-20`); and the final fix (`task-25`) — the BIOS runs its console
  in **MDA** mode, so its INT 10h read-char busy-waits on the mono status port
  `0x3BA`, which the machine only decoded for CGA (`0x3DA`); aliasing the mono
  ports (`0x3B4/0x3B5/0x3BA`) onto the text card cleared the spin. Console output
  lands in the MDA window at `0xB0000` (`BiosRun.mda`). Workspace: **226 tests**
  (+3 slow boot goldens `#[ignore]`; the FreeDOS one reaches the prompt in ~9 min).
- **The real Kiselev Xi8088 BIOS boots and runs 14 POST checkpoints** (`task-20`,
  folds in ROM mapping `task-12` + POST-port `task-15`): the unmodified BIOS ROM,
  mapped read-only at `0xF8000`, far-jumps from the reset vector and drives our
  hardware through CPU test → chipset → DMAC → low-RAM test → IVT → PIT → PIC → KBC
  → enable-interrupts → video-BIOS scan → MDA/CGA fallback → **RTC init** (passes —
  CMOS is seeded) → CPU-type → **FPU detect**, where it stops at x87 `FNINIT`
  (`DB E3`, linear `0xFA85B`) — the next real-mode lift gap (`task-24`). A 1 MiB
  memory mosaic (RAM / trapped VGA MMIO / zeroed option-ROM area / masked BIOS ROM)
  composes every M5a device with IRQ0/1/8 wired; `run_bios` captures the port-0x80
  POST stream + VGA screen on every path. Emulator-only; x86jit untouched.
  Workspace at that point: **208 tests**. The colour screen was blank because
  the BIOS fallback console then lived in plain RAM at `0xB0000`; it is now
  the independent MDA card, captured in `BiosRun.mda`.
- **DOS-boot groundwork — M4.5 + M5a machine hardening** (tasks 13–19, 23): the
  push toward booting FreeDOS on the AT via the Kiselev Xi8088 open BIOS
  (decision-12). **Real-mode CPU driven to 99.63% decided on the SST/80286 corpus**
  (was 96.20%), lifting LES/LDS, LAHF/SAHF, PUSHA/POPA, ENTER, real-mode LEAVE, and
  fixing real-mode string ops to use `ds<<4`/`es<<4` bases instead of a flat base 0
  (the biggest fix — ~45k corpus fails; string ops are everywhere in a BIOS/DOS).
  No regression in the engine's other modes; x86jit pin bumped to 68e0ee1.
  Machine hardened for a real BIOS: open-bus default for undecoded ports
  (0xFF read, no-op write), CGA mode/colour/status ports with a per-read retrace
  bit, the full 8042 AT command set (self-test/interface-test/input-port/aux/BAT),
  port 0x61 with PIT-driven refresh detect and timer-2 OUT, and seeded CMOS NVRAM with a valid AT
  checksum. All device-level;
  the BIOS machine assembly (task-20) composes them. BIOS fact-check (task-19)
  confirmed the Xi8088 config: master+slave cascade (no 0xA0 collision), 32 KiB ROM
  at 0xF8000, built-in µPD765 floppy INT 13h, 6845 CGA. Workspace: **205 tests**.
- **test386.asm boots as a smoke test** (`task-7`, spike): the third-party 386
  CPU-test ROM boots from RAM at `0xF0000` (reset `F000:FFF0`), reaches **POST
  `[0x00, 0x01]`**, and stops at `SAHF` (`0x9E`) in the conditional-jump test — the
  next unlifted real-mode op. This surfaced (and fixed) a **stale x86jit pin**: the
  emulator was 13 commits behind `pc-emulator`, before the far-jmp/seg-load lift;
  bumped to `3ff42a5` (also brings in-guest `#UD`/`#GP` + `RegionKind::Rom`), no
  regression. New `PostPort` (0x190 capture), `Machine::run_bounded` + a retired
  cap, and `run_test386` (captures POST on every exit path). `vendor/test386`
  builds the image from a pinned commit (GPL-3.0, gitignored). To push further:
  lift `SAHF`/`LAHF`, then POST 3 needs FS/GS, POST 8 needs protected mode.
- **M4 integration — the full AT interrupt/graphics wiring** (`task-5`): the M4
  devices are now wired into the machine. Keyboard scancodes raise **IRQ1** through
  the master PIC and a guest handler reads them; guest stores into the `0xB8000`
  text window are trapped as MMIO and rendered into a `TextGrid` through the
  `FrameSink` seam; and the **8259 slave/cascade** now carries **IRQ8–IRQ15**, with
  the MC146818 RTC's update-ended interrupt reaching the CPU across the
  slave→master-IRQ2 cascade. `AtChipset` device set is
  `[UART, master PIC, PIT, 8042, VGA, slave PIC, RTC]`. Added `Cpu::complete_mmio_write`
  (a trapped store is now acknowledged so the guest advances past it). Golden tests
  for keyboard, VGA text, and the RTC cascade path. Workspace: **163 tests pass**.
- **`RegionKind::Rom` on x86jit `main`** @ 3ff42a5: read-only executable memory for
  a masked BIOS/option ROM — reads and fetches take the inlined RAM path at full
  speed, stores are silently dropped on the interpreter path. Purely additive to the
  region-kind match arms; Long64/Compat32 codegen byte-identical (jit 203/0,
  smc 15/0), `jit_abi` offsets unchanged. Not yet consumed by the emulator.
- **Real-mode fault delivery on x86jit `main`** (`task-11`) @ cbba7d9: in-guest
  `#UD` (invalid-on-286 opcodes), `BOUND`/`#BR`, and the 80286 segment-limit /
  instruction-length `#GP` — all Real16-gated. 80286 corpus jumped to **95.98%
  decided / 92.91% exception delivery**; no Long64 regression.
- **M4 devices + first graphics** (`task-5`, device level): 8042 keyboard, MC146818
  CMOS/RTC, 8237 DMA (via a `note_write`-safe `DmaMemory` handle), and VGA text
  mode as a `Device` behind the new `FrameSink` seam (`video` crate) with a
  rendered-frame golden. All isolated + unit/golden tested. (Now wired into the
  machine — see the M4 integration entry above.)
- **Real mode graduated to x86jit `main`** (`task-3`) @ 40acdbc: the full
  real-mode (16-bit) seam — addressing, INT/IRET/IVT/IF, IRQ injection, the
  complete instruction set — now lives on x86jit main, entirely gated so the
  engine's other modes (Long64/Compat32) are byte-identical. `pc-emulator` is
  fast-forwarded to main; no drift. 80286 corpus 89.7%/95.2%, cf16 34/34,
  JIT suite for the engine's other modes intact.
- SingleStepTests/80286 authoritative silicon oracle (`task-10`) — our exact
  target CPU, now the **sole** real-mode oracle on x86jit `main`: 1,478,000 tests,
  **89.7% overall / 95.2% decided** after the lift-gap work. The redundant 8088
  oracle (`task-6`) was retired — it produced false failures on 8088-vs-286
  differences and the 286 corpus supersedes it.

- Timer-interrupt boot (`task-4`): the real-mode machine takes an IRQ0 timer
  interrupt via the in-guest IVT and prints to serial. `AtChipset` = 16550 UART +
  8259 PIC + 8253 PIT; PIT OUT level → PIC edge detection → `inject_irq`; deterministic
  virtual clock from the retired-instruction counter. Boot golden passes.

## Not started

See `bd ready --exclude-type epic` and `bd list --type epic`. Don't duplicate the bead list here.
