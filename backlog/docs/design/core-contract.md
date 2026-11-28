# Core contract and the front-end split

Frozen record of the repository split; the living description is in [architecture.md](../architecture.md), and the normative contract text is in [unemu-core-api](https://github.com/unemu-org/unemu-core-api).

Proposed PC content binding design: [explicit content bindings](content-bindings.md).

Closed on 2026-11-23: the split is done. The core repository was cut at [a28a4a5b](https://github.com/unemu-org/unemupc/commit/a28a4a5b084a376a9a02baef891318231b308797) and the front end first pinned it at [ccbce16a](https://github.com/unemu-org/unemu/commit/ccbce16ae0c3ac6f7dc9247098874fcaee43dc0a); both released `v0.0.4` (unemupc `8a319d70`, unemu `ff37615b`, pinned to unemupc `v0.0.4`). The software repository (formerly unemu-media) installs unemu with its own pinned fetcher, not `setup-unemu`.

Status: **Accepted** by the maintainer on 2026-11-17 (design step of
`upc-dfs.24`, with `upc-3bm`), with the recommended answer to each question
of §11. Steps 1 to 3 of §7.5 and steps 1 to 6 of §8 are implemented (the
contract crate of §3 and the PC core of §4 in its stage A came with §8 step
2; steps 3 to 6 as §8.1 plans them, which leaves the session's crate of
§8.1.10); the other steps land one at a time. §8.1, the plan of steps 3 to 6 of
§8, was accepted by the maintainer on 2026-11-18. The decision record is
[decision-40](<../../decisions/decision-40 - Front ends drive emulator cores through one contract crate.md>).
§12 (embeddable cores, storage objects as contract 0.2, the core's own
command line `unemupc` and the front end's binary `unemu`) was
**accepted** on 2026-11-18 under
[decision-41](<../../decisions/decision-41 - Emulator cores embed like libretro cores and each has its own command line.md>),
with the recommended answers of §12.13. §13 (the second platform: the
contract's own repository, what a NES core adds to the contract, the front
end's generic launch path and the registration of cores) was **accepted**
on 2026-11-18 (design step of `upc-dfs.27`), with the recommended answers
of §13.9. §14 (§4's stage B in fourteen steps: the PC config codec in the
new crate `pc-config`, the launch document, `PcCore::create`, and both
front ends starting stored PC images through `core_launch`) was
**accepted** on 2026-11-19, with the recommended answers of §14.6; the
corrections it made to earlier sections are marked "(corrected
2026-11-19, stage B plan)". Stage B has now landed, with the live helpers
retained in §4 and §14.3 S14.

Markers: **[REPO]** = read in this repository at `896c5d9`, with the path;
**COUNTED** = counted for this document by a throwaway script over the
sources (reference counts are approximate: the script reads `crate::path`
uses and cuts each file at its first test module); **PRELIMINARY** = read
from the measurement logs of the change-targeted testing work (`upc-x83.19`)
on 2026-11-17, taken on a shared development host under load, and replaced
by that work's own write-up when it lands; **ASSUMED** = not verified.

## 0. The design in ten lines

1. One small crate, working name **`unemu-core-api`**, is the whole contract
   between a front end and an emulator core: a `Core` that describes itself
   and creates machines, and a `Machine` the front end drives.
2. A machine is driven by five kinds of call: run to a limit, read the
   picture and the sound, send input, change media, and control (reset,
   save and load state, finish). Pause is the absence of a call.
3. Time is the core's own: virtual **ticks** at an exact rational rate, and
   **steps** in the core's own unit. The contract has no wall clock.
4. What a core can do is **described as data**: the core, each platform it
   provides, and each machine (displays, sound, input ports, media slots,
   resets, saved state). A front end enables a control from the description.
5. The contract has **no dependencies**, no generics and no platform words.
   Every type is `#[non_exhaustive]`, so a minor version only adds.
6. A core states the contract version it **implements**; the front end
   refuses another major, and shows the platform disabled with the reason.
7. Everything a platform knows about artifacts stays where it is: the
   provider traits of `image-store` (`ConfigValidator`, `MediaFormats`,
   `FirmwareRules`) and the provider crate (`pc-image`). Platform-only
   commands and dialogs stay in per-platform modules.
8. The PC core implements the contract in an adapter crate, working name
   **`unemu-pc-core`**, over today's `assembly`; scan codes, the 8042 queue
   and the CP437 rasteriser move below the contract with it.
9. `cli` splits into **`cli-core`** (neutral), **`cli-pc`** (PC provider
   commands), the **`upcli`** binary, and **`pc-harness`** (the golden
   runners and the 32 test binaries that use nothing else); `window` stops
   depending on `cli`.
10. A script reading `cargo metadata` keeps the layers apart in CI. Every
    step lands on `main` with the full gate and no behaviour change.

## 1. Direction and scope

The maintainer's direction is binding and restated here. The desktop GUI
and the command line leave unemupc for one front-end repository that pins
emulator cores by exact version, as unemupc pins x86jit. A NES emulator
(unemunes) comes next. The agreed shape:

1. A small, stable, versioned **core contract crate**: the machine trait
   the front end drives, and a description of the core's capabilities. The
   front end depends only on it; every emulator implements it. There is no
   grab-bag `unemu-core` crate.
2. The platform-neutral library and store crates that exist (`image-store`,
   `image-registry`, the media builders, the coming recipe crate) stay
   separate crates on the front-end side.
3. Shared emulation building blocks (scheduler, clock, device traits,
   `testkit`) are shared only when a second emulator needs them.

The command line splits into a platform-neutral core and per-platform
provider commands, both on the front-end side. Platform-specific GUI parts
stay in per-platform modules and are generalised only when a second
platform needs it. **Success criterion: a CLI-only change builds and tests
in under five minutes.**

In scope: the inventory of today's coupling (§2), the contract (§3), the
PC core on it (§4), the front end on it (§5), a sanity check against a NES
core (§6), the CLI split (§7), the window split (§8), the road to two
repositories (§9), and the questions the maintainer answered (§11).

Out of scope: designing the NES emulator; a plug-in system or cores loaded
at run time (cores are compiled in, at pinned versions); sharing device or
scheduler crates; a transport for the contract (it is a Rust API; nothing
in it prevents a later process boundary, and nothing here needs one); the
recipe engine ([recipes.md](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipes.md)); new features of any kind.

## 2. Inventory: today's coupling

### 2.1. Crate graph

Seventeen workspace crates, about 300 000 lines with tests (COUNTED). The
workspace dependencies below are the normal dependencies of each
`Cargo.toml` [REPO: `crates/*/Cargo.toml`]; test-only ones are named in the
text where they matter.

| Crate | Lines | Workspace dependencies | Layer today |
|---|---:|---|---|
| `device` | 1 571 | none | emulation |
| `bus` | 2 325 | `device` | emulation |
| `cpu` | 2 240 | none (`x86jit-core`) | emulation |
| `video` | 1 000 | `device` | emulation |
| `audio` | 1 627 | `device` | emulation |
| `devices` | 38 179 | `device` | emulation |
| `chipset` | 1 147 | `device`, `devices` | emulation |
| `machine` | 5 785 | `device`, `cpu`, `bus` | emulation |
| `testkit` | 446 | `device`, `cpu` | emulation (tests) |
| `assembly` | 36 796 | `machine`, `cpu`, `bus`, `device`, `devices`, `video`, `audio`, `chipset`, **`media`** | emulation |
| `media` | 12 079 | none | neutral |
| `image-store` | 17 283 | none | neutral |
| `image-registry` ([`crates/registry`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/registry)) | 12 124 | `image-store` | neutral |
| `machine-file` | 17 184 | `assembly`, `media`, **`image-store`** | PC |
| `pc-image` | 26 792 | `assembly`, `machine-file`, `media`, `image-store` | PC provider |
| `cli` | 29 019 | `pc-image`, `image-store`, `image-registry`, `machine-file`, `assembly`, `device`, `bus`, `cpu`, `chipset`, `machine`, `media`, `devices`, `video` | front end |
| `window` | 94 972 | `image-store`, `image-registry`, `pc-image`, **`cli`**, `media`, `audio`, `device`, `devices`, `machine-file`, `video`, `assembly`, `machine` | front end |

Read as layers, bottom to top:

```text
emulation   device  bus  cpu  video  audio  devices  chipset  machine  testkit
            assembly ───────────────────────────────┐ (journals at run time)
neutral     media   image-store   image-registry    │
PC          machine-file (assembly, media, image-store)
            pc-image     (assembly, machine-file, media, image-store)
front end   cli     (everything above)
            window  (everything above, and cli)
```

Three edges cross the lines the split needs, and are marked bold in the
table:

- `assembly` → `media`: the running machine owns its journal sessions
  [REPO: `crates/assembly/src/persistence.rs`].
- `machine-file` → `image-store`: a machine file may take firmware from a
  package in the store [REPO: `crates/machine-file/src/firmware_package.rs`,
  `crates/machine-file/src/lib.rs`].
- `window` → `cli`: the desktop takes shared helpers from the command-line
  crate (§2.5).

### 2.2. What is already neutral, and the seams that exist

- **`image-store`** and **`image-registry`** depend on no emulator crate
  [REPO: their `Cargo.toml`]. The store's module documentation states the
  rule: "Platform providers own config validation and ordered graph
  traversal; this crate owns no emulator, registry, disk reconstruction,
  or GC" [REPO: [`crates/image-store/src/lib.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/src/lib.rs)]. Some of their vocabulary
  is PC-shaped (disk and floppy payload media types), but it carries no
  emulator behaviour.
- **Provider traits** already route by platform, all in `image-store`
  [REPO: [`crates/image-store/src/store.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/src/store.rs), `media_set.rs`, `firmware.rs`]:
  - `ConfigValidator`: `artifact_type`, `platform`, `payload_types`,
    `validate(config, descriptors, blobs) -> Vec<Digest>`,
    `validate_resolved`, `required_release` (decision-30).
  - `MediaFormats`: `platform`, `validate_name`, `validate_format`.
  - `FirmwareRules`: `platform`, `validate`, `validate_name`.
  - `BlobSource`: `read_verified`, `open_verified`: verified content by
    descriptor, the store's side of every provider call.
- **Platform ids** are compared through `image_store::platform::same`
  only; the alias table lives in the shared crate
  ([machine-images.md](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md) §1, decision-32).
- **The provider list** is one function,
  `with_image_providers(|&[&dyn ConfigValidator]| …)`, which both front
  ends call "so they accept the same artifacts" [REPO:
  `crates/cli/src/image/registry.rs`]. It lives in `cli`, which is why
  `window` depends on `cli`.
- **`media`** depends on no workspace crate. It holds two things: the
  journal and overlay format with its offline tools (`sink`, `overlay`,
  `scan`, `offline`, `layer`), and the FAT12 builder and image creation
  (`fat12`, `create`).
- **Inside the emulator** the swap seams of decision-7 hold: `Cpu`,
  `Device`, `InterruptController`, `FrameSink`, `AudioSink`, and
  `device::SectorMedia`, the hardware-neutral mounted medium a controller
  reads and writes [REPO: `crates/device/src/media.rs`].
- **The capabilities sketch** of machine-images §6 exists as PC code:
  `assembly::descriptors` (setting domains) and `assembly::capabilities`
  (choices and diagnostics over pending edits).

What does not exist: any seam between a front end and a *running* machine.
Both front ends hold an `assembly::AssembledMachine` and reach into it.

### 2.3. `cli`: module by module

One library and the `upcli` binary. Line counts are COUNTED; the uses are
[REPO] in the named file.

| Module | Lines | PC crates it uses, and why | Kind |
|---|---:|---|---|
| `lib.rs` | 2 471 | `assembly`, `machine`, `cpu`, `device`, `devices`, `video`: the golden runners (`run_bios*`, `run_freedos*`, `run_boot`, `run_keyboard`, `run_rtc`, `run_fdc_write`, `run_vga_text`, `run_test386`), the hand-encoded guest programs, key scripts, `StopReason`, and `finish_persistence` | test harness |
| `command.rs` | 1 037 | Usage text and the top-level parser. PC: `--boot`, `--bios`, `machine check`, `run --machine` (`assembly`, `machine_file`, `machine`, `cpu`, `video` for `--print`). Neutral: `media generations`, `verify`, `rollback`, `commit` (`media::offline` only) | mixed |
| [`create.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/create.rs) | 427 | `media create floppy`, `hd`: `media::fat12`, `media::create`, and `assembly::HardDiskGeometry` for the CHS policy | PC |
| `image.rs` | 611 | Neutral: `image ls`, `workspace ls`, `workspace discard`. Through a provider: `compress` (picks a validator by artifact type), `sweep` (`pc_image::workspace::sweep_materialized`). PC: `import` (machine file to image), `run` (`assembly::assemble_with_verified_bases`), `publish`, `abandon-publication` | mixed |
| `image/media.rs` | 701 | Neutral: `image media ls`, `show`. PC: `image media import` through `pc_image::media_import` | mixed |
| `image/firmware.rs` | 870 | Neutral: `image firmware ls`, `show`. PC: `import`, `edit` match ROMs against the firmware catalogue and the ROM index (`assembly::FirmwareEntry`, `assembly::firmware_discovery`, `pc_image::firmware_import`) | mixed |
| [`image/registry.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-core/src/image/registry.rs) | 1 066 | `push`, `pull`, `remote ls`, `login`, `registry set` are neutral. PC enters in two places only: the provider list (`with_image_providers`) and the catalogue marks of the push guard (`guard_marks`, `pc_image::firmware::proprietary`) | neutral with two hooks |
| [`image/template.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/src/image/template.rs) | 331 | `image template build`: the official PC templates (`pc_image::template`) | PC |
| [`image/machine.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/src/image/machine.rs) | 425 | `image machine *`: every action is a `pc_image::machine` function; `run` assembles | PC |
| `preferences.rs` | 245 | none: the path of the shared preferences file | neutral |
| [`about.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/upcli/src/about.rs) | 77 | `devices::OPL3_NOTICE` in the notices | neutral with one notice |
| `main.rs` | 17 | host paths: `assembly::firmware_discovery::firmware_cache_path`, `image_store::location::default_store_root` | binary |

`bus` and `chipset` appear only in the test code of `cli` (COUNTED), though
they are declared as normal dependencies.

### 2.4. `cli` tests

53 integration test binaries plus the library's unit tests (COUNTED from
`crates/cli/tests`). They fall into three groups:

- **Emulation goldens, 32 binaries**: BIOS POST, FreeDOS boot and
  sessions, floppy, IDE, VGA, EMS, the sound cards, the scheduler A/B
  tests. They use the runners of `lib.rs` or the emulation crates directly,
  never the command line, the binary or the store: `at_pm_test_golden`,
  `bios_boot_golden`, `bios_floppy_boot_golden`, `boot_golden`,
  `dos_interactive_golden`, `dos_write_golden`, `fdc_write_golden`,
  `fixture_policy`, `floppy_boot_golden`, `floppy_swap_golden`, `fpu_bios`,
  `freedos_boot_golden`, `ide_freedos_golden`, `keyboard_golden`,
  `landmark`, `lotech_ems_golden`, `media_device`, `media_fixtures`,
  `open_vga_golden`, `opl3_golden`, `quantum`, `rtc_golden`, `sb16_golden`,
  `sb16_word_golden`, `scheduler_ab`, `serial_mouse_golden`,
  `speaker_golden`, `test386_golden`, `vga_bios`, `vga_scrolling_golden`,
  `vga_text_golden`, `xms_warm_boot_golden`. Eleven of them load machine
  files through `machine_file`.
- **Goldens that need a front end, 7 binaries**: `at_games`,
  `freedos_install_golden`, `ide_journal_golden` and `media_files_golden`
  call `cli::command` beside the emulator; `firmware_catalogue` and
  `timing_adoption` run the `upcli` binary; `machine_freedos_golden` runs a
  machine from the image store through the provider.
- **Command tests, 14 binaries**: `image_commands`,
  `image_firmware_commands`, `image_machine_commands`,
  `image_media_commands`, `image_registry_commands`,
  `image_template_commands`, `platform_alias`, `firmware_host_guard`,
  `machine_check`, `machine_commands`, `media_create`,
  `media_create_files`, `media_fifo`, `media_tools`.

So 32 of the 53 binaries a change to a command module rebuilds and relinks
today are emulator goldens that it cannot affect.

### 2.5. `window`: module by module

One small library (`font`, `raster`, `keyboard`, `mouse`, `WindowSink`) and
the `unemupc` binary, which holds everything else as modules of `main.rs`:
the whole desktop is **one compilation unit and one test binary** [REPO:
[`crates/window/src/main.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/main.rs), [`crates/window/Cargo.toml`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/Cargo.toml)].

| Group | Modules (lines) | PC crates used in production code, and why |
|---|---|---|
| Machine window | `main.rs` `Presenter` (3 387 with tests), `pacing` (501), `host_audio` (504), `keyboard` (1 035), `mouse` (524), `raster` (511), `font` (269), `lib.rs` (157), `drive_chips` (1 030), `system_keys`, `macos_keys` | `assembly::AssembledMachine` and `machine::RunOutcome` drive the run loop (§2.6); `devices::I8042` and the set-1 tables in `keyboard`; `video::Frame` in `raster`; `device::clock::PIT_HZ` in `pacing`; `audio` sink types in `host_audio` |
| Shell | `shell` (14 527, of which about 8 800 are tests), `shell_model` (1 321), `shell_icons`, `app_menu`, `macos_menu`, `logo`, `file_dialog`, `text_clipboard`, `preferences`, `preferences_sheet` | `assembly`: assembling on Start (`assemble_with_verified_bases`, `ObservationPolicy`), `FloppyAction`, `WriteMode`, `ImageSource`, `DeviceId`, `firmware_discovery`; `machine_file::PreparedMachineFile` for machine files; `pc_image::machine::State`; `cli::preferences`, `cli::about`; `devices::OPL3_NOTICE` |
| Library and Remotes | `library_view` (6 307) with `machines`, `firmware`, `firmware_metadata`, `hardware`, `install_media`, `pane`, `remotes`, `remotes/details`; `library_worker` (3 667); `remote_browse` (3 989); `remote_pull`; `registry_transfer`, `registry_transfer_view`; `registries_model`; `save_image`; `layers_dialog`; `image_settings`; `settings_model`, `settings_sheet`; `new_machine`, `new_machine_view`; `firmware_choices` | `pc_image` throughout: `machine::*` (machine instances), `workspace`, `revision`, `inspection`, `create`, `template`, `firmware`, `firmware_import`, `media_set`, `is_pc`, `platform_name`, `media_repository`. `assembly::capabilities` and `descriptors` in settings and New machine; `assembly::FirmwareEntry` in the firmware views; `machine_file::parse_toml` and `resolve` in settings. `cli::with_image_providers`, `push_plan`, `registry_client`, `registry_probe_client` |
| Media picker and dialogs | `media_picker` (4 161), `media_dialog` (2 571), `media_create` (1 718), `created_media` (910), `catalogue_menu` (1 190) | `assembly::FloppyGeometry`, `FloppyMedium`, `FloppyMediaList`, `DriveKind`, `WriteMode`, `ImageSource`, `descriptors`; `media::fat12`, `VerifiedBase`; `pc_image::machine`, `workspace`, `media_import`; `machine_file::schema`, `resolve`; `cli::create` (the image-creation request and its executor) |

Fifteen modules name no emulation or PC crate in production code:
`app_menu`, `file_dialog`, `font`, `logo`, `macos_keys`, `macos_menu`,
`shell_icons`, `system_keys`, `text_clipboard`, `library_view/pane`,
`registries_model`, `preferences_sheet`, `registry_transfer_view`,
`save_image` and `library_view/remotes/details` (COUNTED). Every other
module names at least `assembly`, `pc_image`, `machine_file` or an
emulation crate.

The modules reference each other in cycles: `library_view` uses 20 sibling
modules and `shell` 22, and `library_worker`, `media_picker`,
`registry_transfer` and `new_machine` each point back at `library_view` or
`shell` (COUNTED). Most arrows into `shell` are for six skin helpers:
`skin_visuals`, `configure_context`, `gradient_title`, `escape_pressed`,
`bevel`, `bevel_ring` (95 references with tests, COUNTED).

Production uses of `cli` in `window` [REPO]:

| Use | Where | What it is |
|---|---|---|
| `cli::with_image_providers` | `library_worker.rs` | the provider list |
| `cli::push_plan` | `registry_transfer.rs`, [`library_view/firmware_metadata.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/firmware_metadata.rs) | push plan with the catalogue marks |
| `cli::registry_client`, `registry_probe_client` | `registry_transfer.rs`, `remote_browse.rs` | a registry client for a host and its account |
| `cli::create::{CreateCommand, parse, execute, …}` | `media_dialog.rs` | blank and formatted images |
| `cli::preferences::path` | [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) | the shared preferences file |
| `cli::about::licence_notice` | [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs), `main.rs` | the About text |

The tests of `window` also call the whole command line
(`cli::command::run`) to prepare fixtures in six places [REPO:
`library_test_support.rs`, [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs), `media_dialog.rs`,
[`library_view/install_media.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/install_media.rs)].

### 2.6. How the window drives a machine today

The session is `Presenter` in [`crates/window/src/main.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/main.rs). Per run chunk it
makes these calls [REPO]; the contract of §3 is cut along them.

| Purpose | Today's call |
|---|---|
| Create | The library worker thread prepares the session (`pc_image::machine::prepare` or `workspace::prepare_pinned`: the configuration and its verified bases) and hands it over as a `SessionTransfer`; the UI thread calls `assembly::assemble_with_verified_bases(config, policy, options, bases)`. A machine file goes through `machine_file::prepare_for_assembly_with_store_root` |
| Time | `machine.clock()` (ticks at the PIT input rate), `machine.virtual_time()` (retired instructions plus halted clocks) |
| Run | `pacing::run_cap(target, clock, time, rate)`, then `machine.run_bounded(Some(cap))` → `RunOutcome::{RetiredCapReached, Halted, ResetRequested}` |
| Guest reset request | `assembled.before_cpu_reset()`, `assembled.reset_cpu(ResetCause::Requested)` |
| Picture | `observation.subscribe(display, sink)`, then `assembled.capture_text(display)`; `WindowSink` rasterises the `video::Frame` with the bundled CP437 fonts |
| Sound | `observation.subscribe_audio(sink)`, `assembled.capture_audio()`; 48 kHz stereo `i16` |
| Keyboard | `HostKeyboard::key_event` (winit key code to set-1 scan codes, typematic, NumLock-aware wrappers), then `inject_at_boundary` (8042 back-pressure) |
| Mouse | `HostMouse` (counts, button queue), `assembly::inject_mouse` at the boundary |
| Media | `assembled.change_floppy(DeviceId::Fdc, unit, FloppyAction::{Eject, Insert(i), Next})`, `append_floppy_verified`, `floppy_name`; slots from `media_dialog::FloppySlot::from_config(&MachineConfig)` |
| Drive lights | downcast to `devices::Upd765` and `devices::Ata`: `drive_active(unit)`, `activity(unit)` |
| Host reset | `reset_halted`: `before_cpu_reset`, `reset_cpu(ResetCause::Host)`, only while halted |
| Power cycle | the shell stops the session and starts it again |
| Persistence | `check_persistence`, `flush_persistence`, `finish_persistence`, `PersistenceHandle::maintenance_failure` |
| Diagnostics | `unlisted_firmware`, `observation.take_speaker_trace`, lines on standard error |
| Pause | the shell stops calling `advance` and re-anchors `pacing` on resume |
| Save state | a disabled menu item; the machine has no saved state yet |

### 2.7. What crosses the future repository line

For two repositories, each of these must be cut or pointed the other way.
None of them blocks the splits inside the workspace (§7, §8).

| Edge | Why it exists | What removes it |
|---|---|---|
| `window` → `cli` | shared helpers live in the command-line crate | §7 step 2 |
| `cli` runners used by goldens and by command code | one library holds both | §7 step 3 |
| `window` library (fonts, raster, scan codes) → `video`, `devices`, `assembly` | PC presentation and input encoding live in the desktop | §4, with the adapter |
| `window` and `cli` → `assembly::AssembledMachine` | no run-time seam | the contract, §3 |
| `pc-image` → `assembly`, `machine-file` | the PC config codec, the firmware catalogue, capabilities | stays: a provider crate may use its own core's crates (§3.12, §11 Q3) |
| `machine-file` → `image-store` | firmware packages from the store | a small lookup trait in `machine-file` that the provider implements over the store (§9). Removed by step 7 of §12.10 |
| `assembly` → `media` | journals written while the machine runs | §9 and §11 Q4 |

### 2.8. Build and test time today

All figures here are PRELIMINARY (shared host under load, six compile jobs,
four test threads).

- A clean build of every test binary took 200 s. The largest single units
  were the `unemupc` test binary (22 s), `assembly`'s tests (18 s), the
  `unemupc` binary (13 s) and `machine-file` (11 s each for the library
  and its tests). Each `cli` test binary took about 1.3 s; there are 53.
- Running every test binary, slow goldens included, took 460 s of test
  time. The `unemupc` binary's tests took 82 to 88 s with four threads
  (203 s with one); the per-test times add up to 294 s, of which the
  library views with their layout tests and the library worker are about
  150 s, the settings, New machine and transfer dialogs about 60 s, the
  three sound-level tests 44 s, and the shell 18 s.
- With the affected-set runner of `upc-x83.19` at its fast tier, a change
  to `crates/window/src/pacing.rs` took 134 s, a change to
  `crates/cli/src/image.rs` took 154 s (it selects `cli` and `window`),
  and a change to [`crates/image-store/src/library.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/src/library.rs) took 291 s.

So the runner already brings a CLI-only change under five minutes at the
fast tier, but most of that time is the desktop being rebuilt and
re-tested for a change that cannot affect it. The split removes that work
by construction, where the runner can only select around it.

## 3. The contract crate

This section describes release v0.1.0 of
[unemu-org/unemu-core-api](https://github.com/unemu-org/unemu-core-api)
(commit `4e2dd83`), where the contract has lived since §13.6 step 3 (this
workspace pins v0.2.0 since §13.6 step 7; the 0.2 additions are §12.5 and
§13.2); the
crate's documentation comments and its `CHANGELOG.md` are now the normative
text, and this section stays as the record of the design.

Working name `unemu-core-api` (§11 Q2); directory `crates/core-api` while it
lived in this workspace (§11 Q1).

### 3.1. Rules of the crate

- **No dependencies.** `std` only. Errors implement `std::error::Error` by
  hand. This departs from the `thiserror` convention for library errors on
  purpose: nothing in the contract may force a version of another crate on
  a core.
- **Object-safe, no generics.** The front end holds `&dyn Core` and
  `Box<dyn Machine>`. Cores are compiled in; the traits also describe a
  later process boundary without change.
- **Additive.** Every struct and enum is `#[non_exhaustive]`. A struct a
  core constructs has a `new` with its required fields and `with_…`
  setters. A minor version adds fields, variants, and trait methods with a
  default body; it removes and changes nothing.
- **No platform words.** No port, IRQ, scan code, floppy geometry,
  cartridge mapper or firmware catalogue appears in it.
- **No host clock, no randomness, no threads.** No `std::time` type
  appears in any signature.
- **No presentation policy.** Pacing, speed, pause, frame rate, window
  size, fonts, key bindings and audio devices are the front end's.
- `#![forbid(unsafe_code)]`.

### 3.2. Version

```rust
/// A version of the contract between front ends and cores.
///
/// `major` changes when a core or a front end has to be edited to keep
/// working. `minor` changes when something is added: a method with a default
/// body, a field, an enum variant. While `major` is 0 every minor may break.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContractVersion {
    pub major: u16,
    pub minor: u16,
}

/// The contract this source defines.
pub const CONTRACT: ContractVersion = ContractVersion { major: 0, minor: 1 };

impl ContractVersion {
    /// Whether a front end written for `self` may drive a core that
    /// implements `core`: the same major, and during major 0 the same minor.
    /// Within one major, a core that implements an older minor is driven
    /// with the newer additions treated as absent.
    pub const fn drives(self, core: ContractVersion) -> bool {
        self.major == core.major && (self.major != 0 || self.minor == core.minor)
    }
}
```

How the front end checks a core:

1. **At build time, by Cargo.** The front end and every core name the
   crate with a SemVer requirement. Compatible requirements resolve to one
   copy, so the trait a core implements is the trait the front end calls.
   Requirements of two majors give two copies, and the core's
   `impl Core` does not satisfy the front end's trait: the build fails at
   the line that lists the core. This only works when every consumer takes
   the crate from **one source** (§11 Q1): Cargo treats the same crate from
   two git revisions as two crates.
2. **At start-up, by value.** `CoreDescription::implements` is a literal
   the core's author writes, never the `CONTRACT` constant, because the
   constant follows whichever copy Cargo resolved. The front end calls
   `CONTRACT.drives(core.description().implements)`. A core that fails is
   listed with its platforms disabled and the reason "core NAME implements
   contract A.B; this program needs C.D", the disabled-with-reason pattern
   of decision-30.
3. **In the front end's tests.** One test asserts that every compiled-in
   core passes the check, so a wrong pin fails CI and not a user.
4. **For additions.** Before the front end relies on behaviour a minor
   added, it compares `implements.minor`. A method added later has a
   default body that returns `ErrorKind::Unsupported`.

### 3.3. The core and its capabilities

```rust
/// One emulator core: a stateless factory for machines of the platforms it
/// provides.
///
/// A front end holds every compiled-in core for the life of the program and
/// may call it from any thread.
pub trait Core: Send + Sync {
    /// What this core is and what it can do. Constant for the life of the
    /// program.
    fn description(&self) -> &CoreDescription;

    /// Build a machine, stopped at virtual time zero.
    ///
    /// Reads the config document, takes every blob it names from
    /// `request.content`, and writes only below `request.session`. Fails
    /// with `ErrorKind::Config`, `Content` or `Unsupported`; a failed create
    /// leaves nothing running and nothing half-written that a later create
    /// could mistake for a session.
    fn create(&self, request: &CreateRequest<'_>) -> Result<Box<dyn Machine>, Error>;
}

/// A core, as data. A front end lists cores and enables controls from this
/// alone, before any machine exists.
#[non_exhaustive]
pub struct CoreDescription {
    /// The contract version the core's author wrote the core against: a
    /// literal, not [`CONTRACT`].
    pub implements: ContractVersion,
    /// Stable id of the core, such as `unemupc`.
    pub id: &'static str,
    /// The core's name for people.
    pub name: &'static str,
    /// The core's release, `MAJOR.MINOR.PATCH`.
    pub version: &'static str,
    /// The machine families the core provides.
    pub platforms: &'static [PlatformDescription],
    /// Licence notices of the third-party components compiled into the core,
    /// for the front end's About text.
    pub notices: &'static [Notice],
}

/// One machine family of a core.
#[non_exhaustive]
pub struct PlatformDescription {
    /// The platform id artifacts name, in its current spelling. Retired
    /// spellings are the front end's concern; the core sees the current one.
    pub id: &'static str,
    /// The platform's name for people.
    pub name: &'static str,
    /// Media types of the config documents `create` reads.
    pub config_types: &'static [&'static str],
    /// The most a machine of this platform can do; one machine may do less
    /// and says so in its [`MachineDescription`].
    pub features: Features,
    /// Run-only options `create` accepts, outside machine identity.
    pub options: &'static [OptionDescription],
}

/// What machines can do. Every field has a "no" value, so a newer field is
/// "no" for an older core.
#[non_exhaustive]
pub struct Features {
    /// Kinds of reset the machine accepts.
    pub resets: &'static [ResetKind],
    /// Whether state can be saved and loaded.
    pub saved_state: bool,
    /// Whether removable media can change while the machine runs.
    pub media_changes: bool,
    /// Whether a run can stop when a display finishes a frame.
    pub frame_stops: bool,
}

/// A third-party component inside a core and its licence notice.
#[non_exhaustive]
pub struct Notice {
    pub component: &'static str,
    pub text: &'static str,
}

/// A run-only option: a permission or a diagnostic switch that never enters
/// a machine's identity, its config or a stored artifact.
#[non_exhaustive]
pub struct OptionDescription {
    pub key: &'static str,
    pub summary: &'static str,
}
```

Capabilities are described at three levels, each narrower than the last:
the **core** (which platforms), the **platform** (`Features`: what any of
its machines may do), and the **machine** (`MachineDescription`, §3.5: what
this one has). A front end enables a control from a description and never
by trying a call. A call the description promised that answers
`Unsupported` is a core bug.

### 3.4. Creating a machine

```rust
/// Everything a core needs to build one machine.
#[non_exhaustive]
pub struct CreateRequest<'a> {
    /// The platform id, in its current spelling.
    pub platform: &'a str,
    /// The resolved machine configuration: the document a stored image keeps
    /// as its config, after any local settings were applied to it.
    pub config: ConfigDocument<'a>,
    /// Verified content by digest: firmware, disk bases, media.
    pub content: &'a dyn ContentSource,
    /// A directory the front end owns for this machine. The core keeps
    /// there what must outlive the run: written sectors, non-volatile
    /// memory, saved battery memory. The core never writes outside it and
    /// never changes a file it took from `content`.
    pub session: &'a Path,
    /// Run-only options, by the keys the platform describes. An unknown key
    /// is refused.
    pub options: &'a [(&'a str, &'a str)],
}

/// A config document with its media type.
#[non_exhaustive]
pub struct ConfigDocument<'a> {
    pub media_type: &'a str,
    pub bytes: &'a [u8],
}

/// Content addressed by SHA-256.
///
/// The source has hashed the whole content, never a prefix and never a
/// sample, before it hands it over, however large it is. The core trusts
/// the receipt and does not hash again.
pub trait ContentSource {
    /// The exact bytes with this digest and length.
    fn open(&self, sha256: &[u8; 32], size: u64) -> Result<Content, Error>;
}

/// Verified read-only content.
#[non_exhaustive]
pub enum Content {
    /// Small content, in memory.
    Bytes(Vec<u8>),
    /// Large content, opened for random access. The core reads it and never
    /// writes it.
    File(std::fs::File),
}
```

The shape follows today's flow (§2.6). The front end verifies the closure
on a worker thread, as `prepare` does now, and its `ContentSource` then
answers from receipts, so `create` on the UI thread does not hash a disk.
The list of digests to verify comes from the provider's
`ConfigValidator::validate`, which already returns it.

`create` takes one input form: the platform's stored config document. Any
other way to describe a machine is a provider matter (§3.12): a PC machine
file with host paths, ROM folders and `extends` is turned into a machine
by the PC provider, which hands the front end the same `Box<dyn Machine>`.
For the PC the document `create` reads is not the stored config as it is:
the stored config names firmware without a size and a layered or blank
disk by its head descriptor, not by the flat bytes `ContentSource::open`
takes, and the store's closure is not the list of content the core opens.
The PC provider writes a **launch document** from the stored config, with
the flat digests, sizes and records added, and `create` reads that (§14.2.2)
(corrected 2026-11-19, stage B plan).

**Media design constraint.** The contract keeps layered disk images
possible ([deferred.md](../deferred.md), Layered disk images): a base is
read-only content identified by its SHA-256, hashed in full by the source;
written sectors go to the session directory and never into the base;
nothing in the contract rewrites a base in place (committing a disk is a
store operation on the front-end side that writes a new base); and a
medium reaches the core as one `Content` plus one write target (§3.8), so a
chain of hash-linked layers can stand behind the same two values later.

### 3.5. The machine: description, time and run

`Machine` is one trait. It is shown here in parts, by subject, over this
and the next five sections.

```rust
/// One running machine.
///
/// A machine lives on the thread that created it; it need not be `Send`.
/// It advances only inside [`Machine::run`]. Between calls nothing happens:
/// pause is the front end not calling `run`.
pub trait Machine {
    /// What this machine has. Constant for the machine's life.
    fn description(&self) -> &MachineDescription;

    /// Virtual time now, in ticks at `description().clock`.
    fn now(&self) -> Ticks;

    /// Work done so far, in the core's own step unit
    /// (`description().step_unit`), for budgets that must not depend on the
    /// clock rate.
    fn steps(&self) -> u64;

    /// Whether the machine can run.
    fn status(&self) -> Status;

    /// Advance until a limit or an event, whichever comes first.
    ///
    /// Input and media changes made since the last return take effect
    /// first, at `now()`. A deadline is met at the core's next boundary, so
    /// `now()` may pass `limit.until` by less than one step. A core that
    /// turns the deadline into a budget of steps may also answer
    /// `Stop::Deadline` with `now()` short of `limit.until` by less than
    /// one step; the front end runs again while it is short. Otherwise a
    /// run never returns before a limit without an event. Fails with
    /// `ErrorKind::Guest` when the emulated machine cannot continue and
    /// with `ErrorKind::Storage` when a host write failed; either leaves
    /// the status `Failed`, except that a `Storage` failure while the guest
    /// halts may leave it `Halted`, so the session can still be finished.
    /// `stop_on_frame` on a machine whose features do not list
    /// `frame_stops` fails with `ErrorKind::Unsupported`.
    fn run(&mut self, limit: RunLimit) -> Result<Stop, Error>;

    // Output: §3.6. Input: §3.7. Media: §3.8. Control: §3.9.
}

/// What one machine has.
#[non_exhaustive]
pub struct MachineDescription {
    /// The rate of [`Ticks`], exact.
    pub clock: Rate,
    /// A word for one step, for messages: `instruction`, `cycle`.
    pub step_unit: &'static str,
    pub displays: Vec<DisplayDescription>,
    pub audio: Option<AudioDescription>,
    pub inputs: Vec<InputPortDescription>,
    pub slots: Vec<SlotDescription>,
    /// What this machine can do, within its platform's `Features`.
    pub features: Features,
}

/// A point in virtual time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticks(pub u64);

/// An exact rate per second, as a fraction in lowest terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    pub numerator: u64,
    pub denominator: u64,
}

/// Where a run stops if nothing else stops it first. Built with
/// `RunLimit::until`, `RunLimit::steps` and `with_…` setters.
#[non_exhaustive]
pub struct RunLimit {
    /// Stop at this virtual time.
    pub until: Option<Ticks>,
    /// Stop when `steps()` reaches this value.
    pub steps: Option<u64>,
    /// Stop when a display finishes a frame.
    pub stop_on_frame: bool,
}

/// Why a run returned.
#[non_exhaustive]
pub enum Stop {
    /// `limit.until` was reached.
    Deadline,
    /// `limit.steps` was reached.
    Steps,
    /// A display finished a frame.
    Frame(DisplayId),
    /// The guest stopped executing and nothing scheduled can wake it; the
    /// status is `Halted`. A reset or a media change may still follow.
    Halted,
    /// The guest switched the machine off.
    PoweredOff,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// `run` advances the machine.
    Ready,
    /// The guest waits for a reset; `run` returns `Stop::Halted` at once.
    Halted,
    /// The guest switched the machine off.
    Off,
    /// A `Guest` or `Storage` error ended the run; only messages, the last
    /// frame and dropping the machine remain.
    Failed,
}
```

Two units, because today's machine already has both and they mean
different things [REPO: `crates/machine/src/lib.rs`, `clock`,
`virtual_time`]. **Ticks** are the clock that pacing, sound and the picture
follow: the front end turns elapsed host time into a tick deadline with
`description().clock`, as `pacing::virtual_ticks` does with `PIT_HZ` today.
**Steps** are the budget of a headless run (`--max-steps`), in the unit the
core counts work in. A core for which they coincide says so by using one
rate for both.

Guest-requested resets are not events. Today the window answers
`RunOutcome::ResetRequested` by calling the reset itself; under the
contract the core performs it inside `run` and reports it as a message
(§3.10), because no front end has a choice to make there.

### 3.6. Picture and sound out

```rust
pub trait Machine {
    /// The picture of a display at the last completed boundary.
    ///
    /// After an error the last picture stays readable. The borrow ends
    /// before the next call on the machine.
    fn frame(&mut self, display: DisplayId) -> Result<Frame<'_>, Error>;

    /// Move the sound rendered up to `now()` since the last call to the end
    /// of `out`, at `description().audio` rate.
    fn drain_audio(&mut self, out: &mut Vec<[i16; 2]>) -> Result<(), Error>;

    /// Say which outputs the front end consumes now. A core need not render
    /// or keep history for an output nobody takes.
    fn set_outputs(&mut self, outputs: Outputs);
}

/// One picture.
#[non_exhaustive]
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    /// `width * height` pixels, rows top to bottom, each `0x00RRGGBB`.
    pub pixels: &'a [u32],
    /// The shape of the whole picture on the emulated display, width to
    /// height, such as 4:3. The front end scales to it.
    pub aspect: (u32, u32),
    /// Changes whenever the pixels changed, so an unchanged picture is not
    /// uploaded again.
    pub serial: u64,
}

#[non_exhaustive]
pub struct DisplayDescription {
    pub id: DisplayId,
    /// The display's name for people, such as `VGA`.
    pub label: String,
}

#[non_exhaustive]
pub struct AudioDescription {
    /// Stereo frames per second, exact.
    pub rate: Rate,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub audio: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayId(pub u8);
```

The picture is **pixels**, in the format the window already uses
("pixels are `0x00RRGGBB` u32 values" [REPO: `crates/window/src/lib.rs`]).
A text mode is a PC matter: its cell grid, attributes and fonts stay below
the contract, and the PC adapter rasterises (§4). Tests and `--print` that
want the text grid use the PC crates directly.

Output is **pulled**. Today's sinks are `Rc`-shared callbacks
(`WindowSink`, `WindowAudioSink`); a pull needs no shared ownership across
the contract and keeps the machine free of the front end's types. Sound
keeps its bounded host ring, mute, speed rules and resampling in the front
end [REPO: `crates/window/src/host_audio.rs`]; `set_outputs` carries what
`AudioSink::enabled` tells the machine today.

### 3.7. Input

```rust
pub trait Machine {
    /// Queue one input event. It reaches the guest at the start of the next
    /// `run`, in order, at `now()`. The core owns everything after that:
    /// encoding, repeat, device back-pressure, coalescing and overflow.
    fn input(&mut self, event: InputEvent);
}

#[non_exhaustive]
pub enum InputEvent {
    /// A key of a keyboard port, by physical position.
    Key { port: PortId, key: Key, state: KeyState },
    /// Relative pointer movement in device counts, positive `dy` up.
    PointerMotion { port: PortId, dx: i32, dy: i32 },
    /// A pointer button, numbered from 0: left, right, middle.
    PointerButton { port: PortId, button: u8, pressed: bool },
    /// A control of a game pad.
    Pad { port: PortId, control: PadControl, pressed: bool },
    /// The front end lost focus: every held key, button and control of
    /// every port is released.
    ReleaseAll,
}

/// A key by its position: the usage id of the USB HID keyboard page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key(pub u16);

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    /// The host repeated a held key. A core with its own repeat ignores it.
    Repeated,
    Released,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadControl {
    Up, Down, Left, Right,
    A, B, X, Y,
    L, R,
    Start, Select,
}

#[non_exhaustive]
pub struct InputPortDescription {
    pub id: PortId,
    pub kind: InputKind,
    /// The port's name for people, such as `Keyboard` or `Controller 1`.
    pub label: String,
}

#[non_exhaustive]
pub enum InputKind {
    Keyboard,
    /// A mouse or similar; the front end offers pointer capture when a
    /// machine has one.
    RelativePointer { buttons: u8 },
    Pad { controls: Vec<PadControl> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(pub u8);
```

The front end maps its toolkit's key codes to `Key` and decides what is a
host shortcut and what goes to the guest. Pointer capture, the release
chord and the swallowed first click stay in the front end. Sending
Ctrl+Alt+Del is three `Key` events and their releases.

### 3.8. Media slots and catalogues

A **slot** is a place a medium goes: a floppy drive, a fixed disk, a
cartridge port, a tape deck. A slot holds an ordered **catalogue** of
media and at most one of them is inserted. The front end changes what is
inserted; the core tells the guest in the way its hardware does.

```rust
pub trait Machine {
    /// Every slot's state now. Reading it takes the activity seen since the
    /// last call, for drive lights.
    fn slot_states(&mut self) -> Vec<SlotState>;

    /// Add a medium to a slot's catalogue and return its index. It is not
    /// inserted by this call.
    fn add_medium(&mut self, slot: SlotId, medium: NewMedium<'_>) -> Result<usize, Error>;

    /// Change what a slot holds, at the next run boundary.
    fn change_medium(&mut self, slot: SlotId, action: MediaAction) -> Result<(), Error>;
}

#[non_exhaustive]
pub struct SlotDescription {
    pub id: SlotId,
    pub kind: SlotKind,
    /// The slot's name for people, in the platform's words: `A:`, `C:`,
    /// `Cartridge`.
    pub label: String,
    /// Whether the catalogue and the inserted medium can change at all.
    pub removable: bool,
    /// Format ids of the media the slot takes, in the vocabulary of the
    /// platform's media formats. The front end offers only these.
    pub formats: Vec<String>,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Floppy,
    FixedDisk,
    Optical,
    Cartridge,
    Tape,
}

#[non_exhaustive]
pub struct SlotState {
    pub id: SlotId,
    pub catalogue: Vec<MediumInfo>,
    /// Index into `catalogue` of the inserted medium.
    pub inserted: Option<usize>,
    /// Whether the slot was busy at any boundary since the last call.
    pub active: bool,
    /// Whether `change_medium` is accepted now.
    pub accepts_changes: bool,
}

#[non_exhaustive]
pub struct MediumInfo {
    /// A name for people; never a host path.
    pub name: String,
    pub sha256: [u8; 32],
    pub size: u64,
    pub format: String,
    pub writes: WriteKind,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    /// The guest sees a write-protected medium.
    ReadOnly,
    /// Guest writes are kept in memory and lost with the session.
    Discarded,
    /// Guest writes are kept in the session, separate from the base.
    Kept,
}

/// A medium the front end hands to a running machine.
#[non_exhaustive]
pub struct NewMedium<'a> {
    pub name: &'a str,
    /// The read-only base, verified in full by the front end.
    pub content: Content,
    pub sha256: [u8; 32],
    pub size: u64,
    pub format: &'a str,
    pub writes: WriteTarget<'a>,
}

#[non_exhaustive]
pub enum WriteTarget<'a> {
    ReadOnly,
    Discarded,
    /// Keep written sectors in this file below the session directory. The
    /// core owns its format; the base is never changed.
    Kept { file: &'a Path },
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaAction {
    Eject,
    /// Insert a catalogue entry.
    Insert(usize),
    /// Insert the next entry, wrapping to the first; an empty slot takes
    /// the first.
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId(pub u8);
```

The three actions and the append are exactly today's
`assembly::FloppyAction` and `append_floppy_verified` [REPO:
`crates/assembly/src/floppy.rs`], with the controller role and unit folded
into a `SlotId`. A fixed disk is a slot that is not removable; it still
reports activity.

The contract models the media of a **running machine**. The catalogue of a
stored image, a machine's pending inserts, installation media sets and the
picker's library are store data (`image-store`) and provider rules
(`MediaFormats`); they do not pass through the core.

### 3.9. Reset, pause, saved state, finish

```rust
pub trait Machine {
    /// Reset the machine. Fails with `Unsupported` for a kind the
    /// description does not list, and with `Usage` when the kind is not
    /// possible in the current status. A `Storage` or `Guest` failure of the
    /// reset itself leaves the status as it was (a halted machine stays
    /// `Halted`, so its session can still be finished).
    fn reset(&mut self, kind: ResetKind) -> Result<(), Error>;

    /// A token naming everything a saved state depends on: the core's
    /// release, the machine's config and the identity of its media. A state
    /// loads only into a machine with an equal token. `None` when the
    /// machine cannot save state.
    fn state_identity(&self) -> Option<String>;

    /// Write the whole machine state at the current boundary.
    fn save_state(&mut self, out: &mut dyn Write) -> Result<(), Error>;

    /// Replace the machine state with a saved one. On failure the machine
    /// is unchanged.
    fn load_state(&mut self, state: &mut dyn Read) -> Result<(), Error>;

    /// End the session cleanly: make everything written durable and close
    /// it as complete. Not valid once the status is `Failed`. Dropping a
    /// machine without `finish` is the equivalent of a power cut: what was
    /// already durable stays, and nothing is marked complete.
    fn finish(&mut self) -> Result<Finished, Error>;
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetKind {
    /// What the machine's reset line or button does: the processor
    /// restarts, memory, devices and media keep their state. Takes no
    /// virtual time.
    Soft,
    /// Power off and on: everything volatile starts fresh; media stay
    /// inserted and non-volatile memory stays.
    Hard,
}

/// What a clean end reports.
#[non_exhaustive]
pub struct Finished {
    /// Problems that did not fail the session, such as a retention pass
    /// that could not run.
    pub warnings: Vec<String>,
}
```

- **Pause** has no call. The front end stops calling `run` and re-anchors
  its pacing when it resumes, as the shell does today.
- **Reset.** The PC lists `Soft` and accepts it while `Halted`, which is
  today's `reset_halted`; a power cycle stays a front-end composition
  (finish, then create) until a core lists `Hard`.
- **Saved state.** The PC lists none yet; the three calls return
  `Unsupported`. `state_identity` is the `compatibility` field
  machine-images §2 reserves for a stored state. The state's bytes are the
  core's own format.
- **Finish** carries today's rule that "dropping this handle never writes a
  clean generation marker" [REPO: `crates/assembly/src/persistence.rs`]
  and that a failed execution is left unsealed.

### 3.10. Errors and messages

```rust
pub trait Machine {
    /// Move the core's messages since the last call to the end of `out`. A
    /// core never writes to the process's standard streams.
    fn drain_messages(&mut self, out: &mut Vec<Message>);
}

/// The one error type of the contract.
#[derive(Debug)]
#[non_exhaustive]
pub struct Error {
    pub kind: ErrorKind,
    /// One sentence a person can read, without a host path the user did
    /// not supply.
    pub message: String,
    /// For `Config`: where in the document, when the core knows.
    pub diagnostics: Vec<Diagnostic>,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The config document is malformed or names something this core does
    /// not have.
    Config,
    /// Content is missing or is not what its digest promised.
    Content,
    /// The core, the platform or this machine cannot do what was asked.
    Unsupported,
    /// The emulated machine cannot continue.
    Guest,
    /// A host write or read below the session directory failed. The guest
    /// never sees it.
    Storage,
    /// A saved state does not fit this machine.
    State,
    /// The call is not valid in the machine's current status.
    Usage,
}

#[non_exhaustive]
pub struct Diagnostic {
    /// A field path in the config document.
    pub path: String,
    pub message: String,
}

#[non_exhaustive]
pub struct Message {
    pub level: MessageLevel,
    pub text: String,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageLevel {
    Note,
    Warning,
}
```

One error type with a kind keeps every signature the same and lets a minor
version add kinds. A core keeps its own typed errors inside
(`AssemblyError`, `PersistenceError`, `RunError`) and maps them at the
adapter; the message is the typed error's text. Host failures stay out of
guest-visible state, as persistent-media § Host errors and determinism
requires: they surface as `Storage` at a run boundary.

Messages carry what the window prints on standard error today: firmware
accepted under the unlisted escape, a guest-requested reset, speaker trace
lines, journal maintenance warnings.

### 3.11. Determinism

- No call takes or returns host time. A core reads no clock, no random
  source and starts no thread on the execution path.
- A machine advances only inside `run`. Input and media changes take
  effect at run boundaries, at a virtual time the front end can read.
- **Replay rule.** The same `CreateRequest` (config bytes, content by
  digest, an equal session directory, options) followed by the same
  sequence of calls with the same arguments gives the same guest and the
  same pictures, sound, messages and stops. A front end that records its
  calls with `now()` and `steps()` at each boundary can replay a session.
- The contract does not promise that two different ways of slicing a span
  into runs give the same guest. Today's machine is exact on active steps
  and rounds a halted stop to a whole tick [REPO:
  `crates/machine/src/lib.rs`, `run_bounded`]; a front end that needs
  slice independence drives by `steps`.
- Sound and picture never pace execution. Host failures never reach the
  guest.

### 3.12. What stays platform-specific, behind providers

The contract is the run-time seam. Everything a platform knows beyond it
stays in **provider** code on the front-end side, which may use its own
core's crates directly (§11 Q3):

| Platform-specific today | Stays where |
|---|---|
| Config schema, validation, the closure walk | `ConfigValidator` implementation (`pc-image`) |
| Media formats and names, firmware rules and catalogue | `MediaFormats`, `FirmwareRules` implementations (`pc-image`, over `assembly`'s catalogue) |
| Machine instances: settings, pending inserts, commit, duplicate | `pc_image::machine` |
| Import of machine files, publication of sessions, templates | `pc_image::import`, `workspace`, `template` |
| Machine files with host paths, ROM folders and discovery | `machine-file`, `assembly::firmware_discovery`; the PC provider builds the machine and returns `Box<dyn Machine>` |
| Hardware choices and diagnostics for Settings and New machine | `assembly::capabilities`, `descriptors` (machine-images §6), used by the PC dialogs |
| The text grid, `--print`, console probes, scripted key stimuli, golden runners | PC crates and `pc-harness` |
| PC commands and PC dialogs | `cli-pc` (§7), the PC module of the desktop (§8) |

Three things are deliberately left out of version 0.1, each with the event
that would bring it in:

- **Validation through the core** (`Core::validate`), so a simple core
  needs no provider crate. Add when the NES provider's `ConfigValidator`
  proves to be a thin wrapper around its core (§6). §13.2 brings it in
  with contract 0.2, beside `Core::identify`.
- **Settings as data** (the `describe`, `choices`, `validate_draft`,
  `preview` calls of machine-images §6). Add when a second platform needs
  a settings dialog.
- **Storage handed in as objects** instead of a session directory, so a
  core does no host I/O. Add with layered disk images or a build without a
  file system (§11 Q4). Decision-41 brings it forward: §12.5 designs it as
  contract 0.2.

## 4. The PC core on the contract

The **`unemu-pc-core`** crate (`crates/pc-core`) is on the
emulation side. It depends on `unemu-core-api`, `pc-config`, `assembly`, `machine`,
`cpu`, `device`, `devices`, `video` and `audio`, and on nothing from the
store.

```rust
/// The PC core's entry in a front end's list of cores.
pub struct PcCore;

/// A running PC behind the contract.
pub struct PcMachine { /* an AssembledMachine and the host-side encoders */ }

impl PcMachine {
    /// Wrap a machine the caller assembled, for machine-file starts and
    /// preset-based tests. Stored images use `PcCore::create_machine`.
    pub fn from_assembled(
        config: &assembly::MachineConfig,
        assembled: assembly::AssembledMachine,
    ) -> Result<Self, unemu_core_api::Error>;

    /// The assembled machine, for PC code that needs more than the
    /// contract: the text grid, device state in tests.
    pub fn assembled(&mut self) -> &mut assembly::AssembledMachine;
}

impl unemu_core_api::Machine for PcMachine { /* … */ }
impl unemu_core_api::Core for PcCore { /* … */ }
```

How each call maps onto today's code, and what moves:

| Contract | PC implementation | Moves from |
|---|---|---|
| `description().clock` | `device::clock::PIT_HZ` times `TICKS_PER_PIT_CLOCK` | `window/pacing.rs` (the constant use) |
| `now`, `steps` | `machine.clock()`, `machine.virtual_time()` | |
| `run` | deadline to cap (`run_cap`), `check_persistence`, inject queued input, `run_bounded`, guest reset (`before_cpu_reset`, `reset_cpu`), sample drive activity | `window/main.rs` `advance_with_slice_clock`, `window/pacing.rs` `run_cap` |
| `frame` | `capture_text(display)` and the rasteriser with the bundled fonts | `window/lib.rs` `WindowSink`, `raster.rs`, `font.rs`, the font assets |
| `drain_audio`, `set_outputs` | `capture_audio` into an internal sink | the sink half of `window/host_audio.rs` |
| `input` `Key` | HID usage to set-1 scan codes, typematic, NumLock-aware wrappers, the 8042 queue | `window/keyboard.rs` minus its winit mapping |
| `input` pointer | count carry and button queue, `inject_mouse` | the device half of `window/mouse.rs` |
| `slot_states` | drives of the config, `inserted_medium`, `floppy_name`, `drive_active`, `activity` | `window/main.rs` `drive_views`, `sample_drive_busy`; `media_dialog::FloppySlot::from_config` |
| `add_medium`, `change_medium` | `append_floppy_verified`, `change_floppy` | |
| `reset(Soft)` | `reset_halted` | `window/main.rs` |
| `finish` | `finish_persistence`, `maintenance_failure` | `window/main.rs`, `cli/lib.rs` |
| `drain_messages` | unlisted firmware, speaker trace, reset lines | `window/main.rs` |
| `notices` | `devices::OPL3_NOTICE` | `cli/about.rs` |

`PcCore::create` reads the PC launch document through `pc-config`, which
now owns the machine schema and lowering, the stored-image model and
codec, encoding and defaults. The provider supplies verified content and
storage objects. [`disk.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/disk.rs) and [`zero.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/zero.rs) stay in `pc-image`: they walk the
manifest's layer chain, build files in the store's cache and describe the
zero base, the store's side of a `ContentSource` (§12.5). The two stages:

1. **Stage A.** `PcMachine` and its tests land. The desktop session drives
   `dyn Machine`. Every start goes through the PC provider as today
   (`pc_image::machine::prepare`, then `assemble_with_verified_bases`),
   wrapped by `PcMachine::from_assembled`. `PcCore::create` answers
   `Unsupported` and the PC's `PlatformDescription` lists no
   `config_types`, so no front end offers it.
2. **Stage B landed.** Both front ends start stored PC images through
   `core_launch` and `PcCore::create_machine`, the typed implementation of
   `Core::create`. The codec lives below the contract in `pc-config`, over
   `ContentSource`; `pc-image` keeps store work and validates through the
   same codec. Its launch document adds flat content identities and record
   names without changing the stored format. §14 records the fourteen steps.

   Deviations retained after S14's caller audit:
   - `PreparedSession::machine` and `provider::lower_session` still supply
     host paths to pending inserts, pending CMOS resets, catalogue editors
     and the desktop's media bindings. They are not the front ends' assembly
     path. The core mounts the document's logical keys through objects.
   - `SessionTransfer` still carries verified file, memory and zero-chain
     inputs into `launch::from_session`; its `restore` also supports desktop
     test inspection and the old/new session comparisons. The unused production
     APIs `pc_image::resolve`, `workspace::start` and `start_new` are removed;
     their test callers use retained preparation APIs for inspection and lifecycle
     checks, or `pc_image::launch` plus `core_launch` and `PcCore` to boot.
     The transfer can go when launch plans capture verified content directly;
     the host-path view and rewriting can go when editors, resets, inserts and
     desktop bindings consume launch identities without a `MachineConfig`.
   - The desktop keeps a typed `PcMachine` and host bindings beside the
     launch, preserving PC media tools and kept catalogues (§14.3 S13).
   - The neutral CLI uses the PC's typed starter for failed-run persistence
     through assembly: contract `finish` is forbidden after failure. The
     contract stop word, absence of an idle cutoff and core reset messages
     are the stated CLI policy changes. The historical Debug parser output
     remains in `upcli` (§14.3 S11, S12).

Consequences to plan for:

- **Licences.** The MDA and VGA fonts are CC BY-SA 4.0 and move with the
  rasteriser. The crate-scoped exception in `deny.toml`, the licence field
  and `scripts/third-party-licenses.sh` name `window` today and must name
  the adapter crate for those assets (§11 Q5).
- **Byte-identical input.** The tests that pin the scan-code stream, the
  typematic repeats and the mouse packets move with the encoders and must
  pass unchanged; the HID mapping gets a table test against today's winit
  mapping.
- **Goldens do not use the contract.** They keep `assembly` and
  `pc-harness`. The adapter gets its own tests: one session through
  `dyn Machine` compared with the same session through `AssembledMachine`.

## 5. The front end on the contract

**The desktop session** (`Presenter`) becomes a platform-neutral crate that
holds a `Box<dyn Machine>`. Its loop, in today's terms:

```rust
fn advance(&mut self, elapsed: Duration) -> Result<(), Error> {
    let target = self.pacing.target(elapsed, self.machine.now());
    while self.running && self.below(target) && self.pacing.slice_open(/* … */) {
        for (slot, action) in self.media_events.drain(..) {
            self.machine.change_medium(slot, action)?;
        }
        let until = self.pacing.chunk_target(self.machine.now(), target);
        match self.machine.run(RunLimit::until(until))? {
            Stop::Halted => { self.running = false; self.halted = true; }
            Stop::PoweredOff => self.running = false,
            _ => {}
        }
        self.machine.drain_audio(&mut self.audio)?;
        self.machine.drain_messages(&mut self.messages);
    }
    let frame = self.machine.frame(self.display)?;
    /* upload when frame.serial changed; update drive lights from slot_states() */
    Ok(())
}
```

Pacing, speed, the paint cadence, the host audio ring, mute, pointer
capture and host shortcuts stay as they are; `pacing` takes the tick rate
from the description instead of `PIT_HZ`. The host clock is still read
only here.

**Starting.** The front end asks the platform's provider for a machine.
For a core whose `create` reads stored configs, the neutral path is:
verify the closure on the worker thread (digests from
`ConfigValidator::validate`), then `core.create(&request)` with a
`ContentSource` over the store's receipts and the machine's workspace as
the session directory. For the PC in stage A the provider returns the
`PcMachine` it built itself. From stage B the PC's provider adds one
store-side step between the image and `create`: it writes the launch
document and hands over a `core_launch::Prepared` made from parts, a
content plan and a storage plan of its own (§14.2.4, §14.2.5); the
store's closure and the core's content list differ for the PC (corrected
2026-11-19, stage B plan).

**The command line** drives no machine through the contract at first: the
commands that run a machine (`run`, `image run`, `image machine run`,
`--boot`, `--bios`) are PC provider commands with PC run policies
(`RunPolicy`, idle limits, `--print`). When a second platform has a
runnable image, `image run` and `image machine run` move to `cli-core` as
`core.create`, `run(RunLimit::steps(max))`, `finish`. They move with
stage B instead, as §12.9 has it: through the contract in `cli-pc` first,
then to `cli-core` with one start hook on `CliPlatform`, last and
droppable (§14.2.7, §14.6 Q4) (corrected 2026-11-19, stage B plan).

**The lists of cores and providers** are data the two binaries pass down.
No neutral crate names a platform.

## 6. Sanity check: a NES core

Not a design of unemunes; a check that the contract does not assume a PC.

| Need of a NES core | In the contract |
|---|---|
| Create from a config (region, expansion) and a cartridge image by digest | `create`: config document plus `ContentSource`. The cartridge's mapper comes from the image's own header, read by the core |
| Battery-backed cartridge memory | the session directory |
| Run frame by frame | `RunLimit::stop_on_frame`, `Stop::Frame` |
| 256×240 picture with 8:7 pixels | `Frame` with `aspect` |
| Sound at its own rate | `AudioDescription::rate`; the front end already resamples |
| Two controllers | two `InputKind::Pad` ports, `InputEvent::Pad` |
| Reset button, power | `ResetKind::Soft`, `Hard` |
| Cartridge port | a `SlotKind::Cartridge` slot with `accepts_changes` false while running |
| Disk system sides | a slot catalogue and `MediaAction::Next` |
| Saved state from the first day | `save_state`, `load_state`, `state_identity` |
| Exact master clock (236 250 000 / 11 Hz, NTSC) | `Rate` |
| No firmware catalogue, no settings dialog | nothing asked of it |

Gaps this found, none blocking 0.1:

- **A light gun** needs an absolute pointer position on the picture: one
  more `InputEvent` variant and `InputKind`, a minor addition.
- **Validation.** A NES provider's `ConfigValidator` would only parse the
  config and name the cartridge digest. If that holds, `Core::validate`
  (§3.12) lets a simple platform ship with no provider crate at all.
- **Rewind and run-ahead** want cheap repeated saves; `save_state` to a
  `Write` in memory serves, and a faster path can be added later.

## 7. The CLI split

Current names (2026-11-21, §12.10 step 13): `upcli` is the composition
library only; the `window` package builds the combined `unemu` executable.
The dated split records below retain the executable name they measured.

### 7.1. Target crates

| Crate (directory) | Layer | Contents |
|---|---|---|
| `cli-core` ([`crates/cli-core`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-core)) | neutral | The command model and top-level parser; usage assembly; exit-code policy. Commands: `image ls`, `image sweep`, `image compress`, `image workspace ls`, `image workspace discard`, `image media ls`, `show`, `image firmware ls`, `show`, `image push`, `pull`, `remote ls`, `login`, `registry set`, `recipe check`, `show`, `media generations`, `verify`, `rollback`, `commit`. The `CliPlatform` trait (§7.2). Depends on `image-store`, `image-registry`, `media`, `recipe`, `anyhow`, `toml` |
| `cli-pc` ([`crates/cli-pc`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc)) | PC | The PC's `CliPlatform`. Commands: `--boot`, `--bios`, `machine check`, `run --machine`, `image import`, `image run`, `image publish`, `image workspace abandon-publication`, `image machine *`, `image media import`, `image firmware import`, `edit`, `image template build`, `media create`. Depends on `cli-core`, `pc-image`, `machine-file`, `assembly`, `pc-harness`, and the emulation crates the run commands need |
| `upcli` ([`crates/upcli`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/upcli)) | application | Library-only command composition, the host-default runner and command usage/parse pins. `window` calls it for command words; the executable is `unemu` (§12.10 step 13) |
| `pc-harness` (`crates/pc-harness`) | PC | Today's `cli/src/lib.rs`: runners, guest programs, key scripts, `StopReason`, and `tests/support`, a directory the other crates' tests include with `#[path]` (a `test-support` feature would need `assembly` to depend on a `pc` crate). Its `tests/` are the 33 emulation golden binaries (§2.4 plus `user_media_volumes`). Depends on `assembly` and the emulation crates and, as a test dependency for the eleven goldens that load machine files, on `machine-file` |
| `platforms` ([`crates/platforms`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/platforms)) | application | The one place that names every compiled-in platform, for both binaries: the provider list (`with_image_providers`), the program's licence notice with each core's notices. In the front-end repository this is the crate whose manifest pins the cores |

Moves into existing libraries, so that `window` needs nothing from a CLI
crate:

| Today in `cli` | Goes to | Why there |
|---|---|---|
| `registry_client`, `registry_probe_client` | `image-registry` (`accounts`) | They use registry types only |
| `push_plan`, `guard_marks` | `image-registry::push` takes the catalogue-restricted digests as an argument (`LocalMarks` already has a catalogue mark); `pc-image` states them | Splits the neutral plan from the PC knowledge |
| `preferences::path` | `image_store::location` | Beside `default_store_root`, the other host location |
| `create` (request model, `parse`, `execute`) | `pc-image` | PC diskette formats and CHS policy; a library both front ends call |
| `finish_persistence` | `cli-pc` (and gone with `Machine::finish`) | |

### 7.2. The provider seam of the command line

```rust
/// One platform's part of the command line.
pub trait CliPlatform {
    /// The platform id, in its current spelling.
    fn platform(&self) -> &str;
    /// Validators for every artifact type of the platform, for transfer and
    /// listing.
    fn validators(&self) -> Vec<&dyn ConfigValidator>;
    /// Digests of a manifest's blobs the platform knows must not be
    /// redistributed, for the push guard.
    fn restricted(&self, store: &Store, manifest: &Manifest) -> anyhow::Result<Vec<Digest>>;
    /// Publish `target` as a compressed copy of `source`, with the local
    /// records the platform keeps beside an image.
    fn compress(&self, store: &Store, source: &str, target: &str) -> anyhow::Result<Descriptor>;
    /// The images this platform's workspaces and machines pin, which a
    /// sweep of the materialized cache keeps.
    fn pinned(&self, store: &Store) -> anyhow::Result<Vec<Digest>>;
    /// The usage lines of the platform's own commands.
    fn usage(&self) -> &str;
    /// Parse the platform's own command, or `None` when the words are not
    /// this platform's.
    fn parse(&self, args: &[OsString]) -> Option<Result<Box<dyn PlatformCommand>, &'static str>>;
}

/// A parsed platform command.
pub trait PlatformCommand {
    fn execute(self: Box<Self>, io: &mut Io<'_>, host: &HostSources) -> anyhow::Result<u8>;
}
```

The hooks are the places §2.3 found: the provider list, the catalogue
marks of the push guard, `pc_image::compress_image` (which copies the PC
import record to the compressed image) and
`pc_image::workspace::sweep_materialized` (which reads what PC sessions
pin).

`cli_core::run(args, io, host, platforms: &[&dyn CliPlatform])` parses the
neutral words first, in today's order, then offers the rest to each
platform. With one platform every command keeps its exact words, output
and exit code. How a command names its platform once there are two (by the
artifact it names, where it names one) is decided with the second
platform, not here.

### 7.3. Dependency rules

Each workspace package declares its layer in its manifest:

```toml
[package.metadata.unemu]
layer = "neutral"   # contract | neutral | emulation | pc | application
```

| Layer | Crates (after the split) | May depend on |
|---|---|---|
| `contract` | `unemu-core-api` | nothing at all, also outside the workspace |
| `neutral` | `image-store`, `image-registry`, `media`, `cli-core`, later the recipe crate and the neutral window crates | `contract`, `neutral` |
| `emulation` | `device`, `bus`, `cpu`, `video`, `audio`, `devices`, `chipset`, `machine`, `testkit`, `assembly`, `unemu-pc-core`, `pc-config` | `contract`, `emulation` |
| `pc` | `machine-file`, `pc-image`, `pc-harness`, `cli-pc`, later the PC window module | `contract`, `neutral`, `emulation`, `pc` |
| `application` | `upcli` (composition library), `platforms`, `window` (`unemu` binary) | anything |

`pc-config`, the PC config codec of §4 stage B (§14.2.1), joins the
`emulation` row (corrected 2026-11-19, stage B plan).

Two more rules:

- No window library depends on a CLI crate, and no CLI crate on a window
  crate. The combined application permits one explicit normal `window` →
  `upcli` composition edge (§12.10 step 13); UI libraries keep the boundary.
- The rules cover normal, build **and** test dependencies, because a
  neutral crate whose tests need a PC crate is rebuilt by every emulator
  change and cannot leave the repository without it. Test fixtures that
  need a real platform live in a `pc` or application crate.

### 7.4. The boundary check

`scripts/crate-boundaries.sh`, bash with `cargo metadata` and `jq` (both
in the devShell; no new crate):

1. Reads `cargo metadata --format-version 1 --no-deps` once: the
   dependencies each workspace manifest declares, optional and
   target-specific ones included, not the resolved closure.
2. Fails when a workspace package has no `layer`, so a new crate cannot
   dodge the rules.
3. For every workspace package, walks those declared dependencies inside
   the workspace (its own of every kind, then normal and build ones of the
   crates it reaches, whose test dependencies it never builds) and fails on
   the first crate on each path that the table of §7.3 forbids, printing
   the path once: `cli-core → image-registry → … → assembly`. The walk
   does not cross an allow-listed edge, so a crate depending on `assembly`
   needs no entry of its own for `media`.
4. Fails when the `contract` package has any dependency.
5. Reads `scripts/crate-boundaries.allow`, a list of known violations, one
   `from → to (kind): reason` per line. A listed edge passes; **a listed
   edge that no longer exists fails**, so the list only shrinks. It starts
   with today's one forbidden edge, `assembly → media` (`emulation` →
   `neutral`), and the test dependencies found when the layers are first
   declared. `machine-file → image-store` is `pc` → `neutral` and allowed;
   it matters only for the extraction (§9). Step 13 of §12.10 adds one
   deliberate application-composition edge, `window` → `upcli` (normal),
   with a passing fixture; the existing library-crossing fixtures still fail.
6. `--self-test` runs the rules over fixture metadata files, each with one
   planted violation, and expects each to fail: the same approach the
   affected-set runner takes for its own mapping.

It runs in the `guard` lane of `scripts/ci-gate.sh`, on every push and
pull request, and in the supervisor's gate. It takes about a second
(measured: about 0.1 s for the check, a few seconds with the devShell's
start).

### 7.5. Steps

Each step is one bead: it lands on `main` with the full gate, changes no
behaviour, and moves tests with the code they test, weakening none. The
expected effects are ASSUMED, derived from the PRELIMINARY figures of
§2.8, for a CLI-only change measured as `upc-x83.19` measures it.

| # | Step | Expected effect |
|---|---|---|
| 1 | **Done.** **Boundary check.** Add `layer` to every manifest, `scripts/crate-boundaries.sh` with its self-test and the allow list of today's violations; wire it into the `guard` lane. No code moves | Gate +1 s. Later steps cannot regress |
| 2 | **Done.** **`window` off `cli`.** Move the shared helpers of §2.5 to `image-registry`, `image_store::location`, `pc-image` and the new `platforms` crate (§7.1). Replace the six `cli::command::run` fixture calls in `window`'s tests with the library calls they stand for. Remove `cli` from `window`'s manifest; add the "no window crate depends on a CLI crate" rule | The largest win. A CLI-only change no longer rebuilds or tests the desktop: from about 154 s to about 30–45 s. Clean build unchanged |
| 3 | **Done.** **Runners and goldens out of `cli`.** Create `pc-harness` from `cli/src/lib.rs` and move the 32 golden binaries and `tests/support` with it. `cli` depends on `pc-harness` for `--boot`, `--bios` and `run`. The seven goldens that need a front end stay with `cli` and stay selected by tier. As landed: 33 binaries moved (`user_media_volumes` was added since §2.4) and eight front-end goldens stay (`boot_sector_golden` too); `tests/support` stays a directory the other crates include by `#[path]`, because a `test-support` feature would make `assembly` (layer `emulation`) depend on a `pc` crate | A CLI-only change links 21 test binaries instead of 53: about 10–20 s less, and about 60–70 s less at the tier that includes the slow goldens, by construction and not by tier marks. MEASURED with the affected-test runner: 22 test binaries instead of 55 (24 instead of 57 with the unit tests), 7 ignored tests run by name instead of 32, 57 s instead of 95 s on a loaded machine |
| 4 | **Done.** **The provider seam, inside `cli`.** Add `CliPlatform` and `PlatformCommand`; route every PC command through one PC implementation; the neutral commands take providers as arguments. Still one crate. A test asserts the usage text and the parse of every documented command line are unchanged. As landed: the neutral command line is `cli::cli_core` (the traits, `HostSources`, `Io`, `parse`, `usage`, `run`, and the neutral commands with their tests), the PC's is `cli::cli_pc` (`PcPlatform`, built by `with_platform` from the provider list of `platforms`, and every PC command), and `cli::command` is the program that composes them; `upcli`'s `main` passes the platform list. Deviations from the sketch above: `restricted` returns `image_store::Result`, the type `push::plan_with_catalogue` takes; two more hooks, `recipes` (the recipe provider of `recipe check` and `show`) and `medium_format` (the format names of `image media show`); `usage` takes a `UsageSlot`, because the neutral and the PC lines interleave and the text keeps its order byte for byte; `PlatformCommand` is `Debug`, for the parse pin; `--version` and `--about` stay with the program (`cli::command`), which owns the notice; the platforms are offered a command line after `image` and before the journal commands, since a PC option such as `--rom-dir` on a `media` line was always the PC's refusal; a command that names one artifact (`image compress`, `media show`, `firmware show`) asks the first platform, the choice among several being left to the second platform. Neutrality inside `cli` is a source test ([`tests/neutral_boundary.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-core/tests/neutral_boundary.rs)): with comments and literals blanked, no file under `src/cli_core` starts a path or a `use` tree (also a leading `::` one, or one in a `use {…}` group over several lines) at an emulation, PC or application crate, names a `crate::` path outside `cli_core`, climbs out of `cli_core` with `super` (by the file's module depth and its inline modules), or uses `extern crate`, `#[path]`, `include!` or `#[macro_export]`. [`tests/command_line_pin.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/upcli/tests/command_line_pin.rs) pins the usage texts and the parse of 215 lines: every usage form, every `upcli` example of commands.md (a copy kept with the test), and the refused lines whose words the two parts split; each of the 36 refusals reads as the binary before the split printed it (MEASURED). MEASURED: the binary's output, exit codes and written files are byte-identical before and after over 319 invocations | None in time. Makes step 5 a move |
| 5 | **Done.** **`cli-core`.** Move the neutral modules and their tests (`media_tools`, `media_fifo`, the neutral halves of the image command tests, with the store's synthetic providers). The rest of `cli` keeps working through re-exports for one step. As landed, together with step 6 in one change: [`crates/cli-core`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-core) (layer `neutral`) is what was `cli::cli_core`, with its unit tests (the toy platform included); `media_tools` and `media_fifo` moved and run the neutral command line with no platform at all (`cli_core::run(…, &[])`). Deviations: no re-exports, since step 6 landed with it; the image command tests were not split, because every one of them builds its images through the PC import and the shared PC fixtures, which §7.3 keeps in a `pc` or application crate: they moved whole to `cli-pc`, and the neutral image commands keep their unit tests with the toy platform in `cli-core`; `media_tools`'s `usage_errors_are_two_and_boot_still_runs` runs `--boot`, so it moved to `cli-pc`, in a test binary of the same name. [`tests/neutral_boundary.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-core/tests/neutral_boundary.rs) shrank to what Cargo cannot see: no file under `cli-core/src` loads another file with `#[path]` or `include!` (`include_str!`, `include_bytes!`); the crate names, `crate::` paths and `super` climbs it checked before are now the manifest's (the boundary check) and the compiler's | A neutral command change rebuilds `cli-core` and its dependents; a PC command change no longer runs the neutral tests. A few seconds. MEASURED: see step 6 |
| 6 | **Done.** **`cli-pc` and `upcli`.** Rename what is left to `cli-pc`; create the `upcli` binary crate. The four test binaries that run `upcli` (`firmware_catalogue`, `timing_adoption`, `machine_commands`, `image_registry_commands`) move to the `upcli` package, because Cargo gives the binary's path only to the package that builds it. Update the package names in CI, the release workflow, `scripts/third-party-licenses.sh`, `scripts/ci-gate.sh`, `commands.md` and `architecture.md`. As landed: [`crates/cli-pc`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc) (layer `pc`) is what was `cli::cli_pc`, plus `cli_pc::command`, the command line with the PC platform alone (`parse`, `run`, `run_with_firmware_sources`), which the PC command tests and the front-end goldens drive (none of them passes `--version` or `--about`); [`crates/upcli`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/upcli) (package `upcli`, layer `application`) builds the binary and has a small library, `about` and `command` (the composed command line with the program's notices), for `main`, its unit tests and `command_line_pin`, which moved there with its pinned files. Deviations: `cli-pc` may not depend on `platforms` (application), so the PC's provider list moved from `platforms::with_image_providers` to `pc_image::with_providers`, which `platforms` now calls, and `cli-pc` asks `pc_image::firmware::restricted` directly; `image_template_commands` plans its push with `image_registry::push::plan_with_catalogue` and the PC catalogue's restrictions, the body of `platforms::push_plan`. The affected-test runner treats all of [`crates/cli-core/src/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-core/src/) as command line the front-end goldens drive, not only the dispatch, because `ide_journal_golden` runs the journal tools; in one crate those goldens ran by name on any CLI change. `scripts/bead-gate.sh` and `ci-gate.sh` lint `-p upcli --bin upcli`; the release workflow builds `-p upcli -p window`; the licence script starts from `upcli` and `window` (the same 352 crates, MEASURED). MEASURED: every test keeps its name (3929 before and after; `cli::cli_core::X` and `cli::cli_pc::X` became `cli-core`'s and `cli-pc`'s `X`); the binary's output, exit codes and written files over the 319 invocations of step 4 are byte-identical but for the firmware index, which records the ROM file's times and inode and differs between two runs of the same binary too | None in time. The neutral command line is PC-free by the boundary check (MEASURED: a planted `cli-core → pc-image` or `cli-pc → platforms` dependency fails it). MEASURED with the affected-test runner on a shared host, a one-line change to the journal tools of the neutral command line: before the split it ran the `cli` crate's 196 fast tests and its 7 front-end goldens by name in a second step, 77 s in all; after it, the goldens tier of `cli-core`, `cli-pc` and `upcli`, the same 203 tests in one run, 48 s |

After step 3 the criterion holds with a wide margin and without the
desktop: build and test of a CLI-only change in well under a minute, and a
whole bead gate (format, clippy on the affected crates, tests, guard) in
about two minutes (ASSUMED). Steps 4 to 6 are for the boundary, not for
time.

Order: 2 before 3 because it is the larger win and unblocks the window
split; 3 before 4 to 6 because it shrinks the crate they reshape.

## 8. The window split

Shorter, because it follows the same rules. Today a window-only change
already takes about 134 s with the affected-set runner (PRELIMINARY), so
the aim here is that a change to one part of the desktop builds and tests
only that part, and that the neutral parts are provably PC-free.

Target crates:

| Crate | Layer | From today's modules |
|---|---|---|
| `ui-kit` | neutral | The skin and widget helpers now in [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) (`skin_visuals`, `configure_context`, `gradient_title`, `escape_pressed`, `bevel`, `bevel_ring`, the marks), `shell_icons`, `logo`, `file_dialog`, `text_clipboard`, `library_view/pane` |
| `machine-view` | neutral | The session over `dyn Machine` (`Presenter`), `pacing`, `host_audio`, `system_keys`, `macos_keys`, the key mapping to `Key`, pointer capture, drive lights. PC encoders and the rasteriser go to `unemu-pc-core` (§4) |
| `library-ui` | neutral, with PC views split off | The library model and worker (`library_worker`, `registries_model`, `registry_transfer`, `remote_browse`, `remote_pull`, `save_image`), then the views (`library_view` with `machines`, `install_media`, `remotes`, `registry_transfer_view`) |
| `media-ui` | neutral, with PC pieces split off | `media_picker`, `media_dialog`, `media_create`, `created_media`, `catalogue_menu`, `drive_chips` |
| `pc-ui` | PC | `new_machine`, `new_machine_view`, `settings_model`, `settings_sheet`, `image_settings`, `firmware_choices`, `layers_dialog`, `library_view/hardware`, `library_view/firmware`, `firmware_metadata` |
| `unemupc` | application | `shell`, `shell_model`, `app_menu`, `macos_menu`, `preferences`, `preferences_sheet`, `main` |

§8.1 plans steps 3 to 6 move by move. It changes the last four rows of
this table, for reasons counted there, and moves the PC dialogs and the
library's frame together in step 6; the maintainer accepted it on
2026-11-18 (§8.1.11), so §8.1.3's crates replace the last four rows above.

Order, each a series of small beads under the same rules as §7.5:

1. **Done.** **`ui-kit`.** Almost every arrow into `shell` is one of six helpers
   (§2.5); moving them out cuts most module cycles. No time win; it is
   the prerequisite for every later step. As landed: the six helpers,
   `window_title` (it shares the title strip) and the eject and speaker
   marks form `ui_kit::skin`; `shell_icons`, `logo`, `file_dialog`,
   `text_clipboard` and the layout half of `library_view/pane` move whole,
   with their tests. The DejaVu symbol font and the egui font notices move
   with `configure_context`; the kit reads the brand logo from
   [`assets/brand/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/assets/brand/). Staying in `window`: `machine_menu_mark` (it maps the
   shell's own commands), `outline_focused_field` and `toolbar_button`
   (not in the list), the skin tests of [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) (they share the mock-up
   tokens and contrast helpers with the focus and toolbar tests there),
   the PC firmware parts of the pane (the Firmware section, the newer
   version hint and the short digest, which read the firmware views; the
   window's `library_view/pane` re-exports the kit's pane for the library
   views) and the desktop-wide glyph and mark tests, in a test-only
   `shell_icons` module, because they read the window's sources and menu
   marks. Test names are unchanged apart from the crate (3902 tests).
2. **Done.** **The contract, `unemu-pc-core` (stage A) and `machine-view`.** The
   session has few inbound arrows (the shell and the media dialogs), so it
   comes out early, and it is where the contract first carries real
   traffic. Its tests go with it: the sound-level tests (44 s of test
   time), the host-audio and session tests. A library change stops
   running them, and a session change runs only them: about 80 s of
   desktop tests down to about 15–20 s. As landed:
   - `unemu-core-api` (`crates/core-api`, layer `contract`, no dependency of
     any kind) holds §3 as written, with `CONTRACT` 0.1 and `drives`, unit
     tests of the version rule and a scripted core and machine driven only
     as `&dyn Core` and `Box<dyn Machine>`.
   - `unemu-pc-core` (`crates/pc-core`, layer `emulation`) holds
     `PcMachine::from_assembled`, `PcMachine::assembled` and `PcCore`, whose
     platform lists no config type and whose `create` answers
     `Unsupported`. From `window` came the set-1 encoder with the 8042
     back-pressure and the reset chord (`keyboard`, now keyed by HID
     usage), the device half of the mouse (`mouse::PcMouse`: button
     transitions, coalescing, `inject_mouse` back-pressure), `run_cap`
     (`pacing`), the rasteriser, `font`, the CC BY-SA fonts with their
     notice and `FONT_NOTICE`, the sink (`RasterSink`, formerly
     `WindowSink`) and drive-activity sampling, with their tests and the
     `presentation` test binary. Its own tests compare a session through
     `dyn Machine` with the same keys and caps on the assembled machine.
   - `machine-view` ([`crates/machine-view`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/machine-view), layer `neutral`, counted as a
     window crate by the boundary check) holds `Session`, the former
     `Presenter` loop over the contract, `pacing` (now at the rate the
     machine describes), `host_audio` (fed by `drain_audio`),
     `system_keys`, `macos_keys`, `keys` (winit key code to HID usage) and
     `pointer` (capture, release chord, swallowed click, count carry). Its
     tests: the host-audio ring, pacing, system-key and macOS-key tests,
     the key map, and session tests against a scripted machine.
   - `window` keeps a `Presenter` that is a `Session<PcMachine>` plus the
     PC catalogue model (the machine config and the floppy slots) that the
     media dialogs, catalogue menus and drive chips edit, and the hold of
     the status-bar drive lights, which belongs to the drive chips (step
     5). All 3902 test names are unchanged apart from the crate (47 tests
     moved to `machine-view`, 39 to `unemu-pc-core`), and 26 tests were
     added, among them a table test that composes the key map with the
     encoder and gives the window's former winit table key for key.

   Deviations from §3 to §5, each forced by keeping the step free of
   behaviour change or by the layer rules:
   - **Additions to the contract**, all additive: named HID usages as
     associated constants (`Key::A`, `Key::ARROW_UP`, …), so the key map
     and the encoder cannot disagree on a number; `new`/`with_…`
     constructors (`const` where a core builds a static description),
     `Features::NONE`, `Outputs::NONE`, `ContractVersion::new` and its
     `Display`; `Rate::new` reduces to lowest terms; the usual derives
     (`Clone`, `Debug`, `PartialEq`) on the data types.
   - **`Session<M: Machine + ?Sized = dyn Machine>`** instead of a session
     that can hold only `Box<dyn Machine>`. The session's code calls the
     contract alone, and its tests drive a scripted machine, but the
     window names `Session<PcMachine>`: the media dialogs, catalogue menus
     and drive chips still append floppies and read the catalogue through
     `PcMachine::assembled` until step 5 moves them behind `add_medium`.
     For the same reason `PcMachine::add_medium` answers `Unsupported` in
     stage A, and its slot catalogues carry each floppy's name and write
     kind (read-only, discarded or kept, as the controller holds the
     medium) but no digest, size or format yet: those stay in the
     provider's machine config.
   - **The PC core names its platform id** (`ibm-pc`) in its
     `PlatformDescription`; the platform-id scan of `image-store`
     ([`tests/platform_ids.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/tests/platform_ids.rs)) lists `pc-core/src/core.rs` among the files
     that may spell it out.
   - **PC calls beyond §4**: `assembled_ref`, `keyboard`, `ctrl_alt_del`
     (the keyboard-owned reset chord reaches an inhibited keyboard whole;
     three key events would not), `inserted_floppy`, `drive_activity`, the
     reset counters and summary, `RASTER_SCALE` for integer scaling, and
     test hooks behind a `test-support` feature (the delivered-input logs
     the replay tests read).
   - **What a core may answer** (contract text of §3.5 and §3.9 changed to
     match, the contract being 0.1 and unpublished): `Stop::Deadline` may
     come back with `now()` short of `until` by less than one step (the
     PC's cap rounds halted time down; the session runs again while it is
     short); a `Storage` failure while the guest halts, and a failure of
     `reset` itself, leave the status `Halted` rather than `Failed`, which
     the session relies on to still finish a halted machine's media.
   - **`Features::frame_stops`**, a new field: `RunLimit::stop_on_frame`
     is honoured only by a machine that lists it and refused with
     `Unsupported` otherwise. The PC does not list it.
   - **Where a run ends.** `PcMachine::run` ends at the retired-instruction
     cap a deadline gives, as each window chunk did; when halted time leaves
     `now()` a fraction of a tick short of `until`, it still answers
     `Stop::Deadline` and the session runs the next chunk, as the window
     loop always did. Exhausted virtual time answers `Stop::PoweredOff`. A
     guest-requested reset happens inside the run, which then continues to
     the same deadline, so after a guest reset the chunk ends where it was
     going to end rather than a millisecond after the reset.
   - **Media keys** change the first and the second removable slot of the
     description, for the PC A: and B:; a machine with a B: but no A: would
     now change B: with the unshifted chord.
   - **Pointer release** with several guest buttons held sends one packet
     per released button where one packet released them all; focus loss of
     a running session still releases everything at once
     (`InputEvent::ReleaseAll`).
   - **Tests that need a real PC stay in `window`**: the three sound-level
     tests, the six host-audio tests that run a PC, the session tests of
     `main.rs` and the mouse tests, which now join `pointer` with
     `PcMouse` in a test shim. A `neutral` crate may not have a PC crate
     among its test dependencies (§7.3), so `machine-view` cannot run a PC.
     `caps_respect_configured_rates_and_saturate_without_overflow` moved
     to `unemu-pc-core` with `run_cap`, and its one `frame_target` check
     joined `overload_drops_debt_once_and_an_ahead_guest_does_no_work`.
   - **Sound at 48 kHz only**: the session plays what `drain_audio` returns
     as 48 kHz stereo, the rate the PC describes, and resamples only to the
     host device's rate. A machine that describes another rate gets no host
     device and one `audio disabled` warning; resampling such a source
     comes with the core that has one.
   - **Pictures** are copied out of the machine only when `Frame::serial`
     moved, and the PC moves it only when the rasterised pixels changed:
     MEASURED over 30 paints of an idle screen, 30 copies before, 1 after
     (`an_unchanged_screen_is_copied_once_across_paints`).
   - **The lights' hold** (`drive_chips::DriveLights`) stays with the drive
     chips in `window`; `machine-view` takes the activity
     (`Session::take_slot_activity`) and the window feeds its lights.

   The expected time effect did not come with this step, because the tests
   that need a real PC stay in `window` and `window` depends on
   `machine-view`. MEASURED with the affected-test runner on a loaded
   development host: a change to [`crates/machine-view/src/session.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/machine-view/src/session.rs) runs
   `machine-view` and `window` (787 tests, one ignored test by name) in
   114 s; a change to [`crates/window/src/shell_model.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell_model.rs) runs `window` (731
   tests, three ignored tests by name) in 114 s. What moved out is the 47
   fast tests that need no machine, so a window change no longer builds or
   runs them; the 44 s of sound-level tests move only with a PC-side crate
   of their own, or once the media dialogs leave the window.
3. **Done** (§8.1.6). **The library model and worker**, without egui:
   types the views and the worker share move down first, which cuts the
   `library_worker` ↔ `library_view` cycle.
4. **Done** (§8.1.7; the frame moved with step 6). **The library and
   Remotes views.** The largest part: with the model and worker about
   150 s of the 294 s of per-test time. After this a change to the shell
   or to the session no longer runs them.
5. **Done** (§8.1.8). **The media picker and dialogs.**
6. **Done** (§8.1.9). **The PC views** into `pc-ui`. What remains in
   `unemupc` is the shell and the composition.

`library-ui` and `media-ui` reach PC knowledge through provider objects
passed in by the application, as the command line does. A view that only
one platform has stays whole in `pc-ui`; nothing is generalised before a
second platform needs it.

The contract crate and the adapter can start as soon as this design is
approved, in parallel with §7: they add crates and move nothing until
`machine-view` exists.

### 8.1. Steps 3 to 6 in detail

Status: **Accepted** by the maintainer on 2026-11-18 (planning step
`upc-dfs.25`), with the recommended answers of §8.1.11: Q1 (a) and the
crate names of Q2. It keeps the rules of §7.3 and of §8 and changes the
last four rows of §8's table, for the reasons counted in §8.1.1 and
§8.1.2. The provider list already moved to `pc-image` with the CLI split
(`pc_image::with_providers`, step 6 of §7.5), so move 3.4 only uses it.

Markers in this section: **[REPO]** = read on `main` at `705b1af`, with
the path; **COUNTED** = counted for this section by throwaway scripts over
[`crates/window/src`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src) (a file's production part ends at its first test
module; a reference is a path that starts at a crate's name, so a `use`
that brings in several names counts once and the figures are lower
bounds); **MEASURED** = one `cargo nextest run -p window` on this commit,
four tests at once, on a loaded development host: 732 tests and 4 ignored,
75.8 s of wall time, 287.7 s when each test's own time is added up. 57 of
them failed at once because one firmware fixture was not fetched in that
checkout (38 of the 57 are tests of the PC dialogs, 11 of the frame, 8
elsewhere), so every figure marked `*` is a lower bound; **ASSUMED** = not
verified.

#### 8.1.1. Inventory of the library, media and PC modules

"PC references" are references in production code to `assembly`,
`pc_image`, `machine_file`, `devices`, `device`, `machine`, `video`,
`audio`, `cpu`, `unemu_pc_core` or `platforms`. `media`, `image_store`,
`image_registry`, `unemu_core_api`, `machine_view` and `ui_kit` are
neutral or the contract and are not counted. "View lines" are the lines of
the top-level items that name `egui`, `eframe`, `ui_kit` or `winit`; a
file with none builds without a toolkit. Seconds are each module's tests
added up (MEASURED); the lines and references are COUNTED.

| Group | Module | Lines with tests | Production | View lines | PC references | Tests | Seconds |
|---|---|---:|---:|---:|---:|---:|---:|
| Registries and Remotes | `registries_model` | 1 978 | 1 145 | 0 | 0 | 16 | 1.6 |
| | `registry_transfer` | 2 889 | 1 548 | 0 | 7 | 18 | 15.2 |
| | `remote_browse` | 3 993 | 1 939 | 0 | 6 | 28 | 9.8 |
| | `remote_pull` with `remote_pull/tests` | 1 790 | 947 | 0 | 3 | 9 | 0.6 |
| | `save_image` | 879 | 629 | 158 | 0 | 11 | 0.7 |
| | `registry_transfer_view` | 1 078 | 789 | 763 | 0 | 1 | 0.6 |
| | `library_view/remotes` with its `tests` | 4 577 | 1 803 | 1 046 | 2 | 39 | 5.3 |
| | `library_view/remotes/details` | 896 | 896 | 705 | 0 | 0 | 0 |
| | **sum** | 18 080 | 9 696 | 2 672 | **18** | 122 | 33.8 |
| Local library | `library_worker` | 4 632 | 2 207 | (6 uses of `egui::Context`) | 124 | 30 | 44.4 |
| | `library_view` (the frame) | 6 320 | 3 998 | 3 093 | 10 | 37 | 48.0 `*` |
| | `library_view/layout_tests` | 2 517 | 0 | | | 27 | 40.1 |
| | `library_view/machines` | 2 499 | 1 962 | 1 249 | 14 | 12 | 1.2 |
| | `library_view/firmware` | 2 871 | 1 806 | 1 065 | 13 | 14 | 9.6 |
| | `library_view/firmware_metadata` | 2 585 | 1 521 | 847 | 10 | 9 | 6.9 |
| | `library_view/install_media` | 2 844 | 1 665 | 929 | 15 | 16 | 12.6 |
| | `library_view/hardware` | 710 | 504 | 0 | 15 | 5 | 1.8 |
| | `library_view/pane` (the PC half; the rest is in `ui-kit`) | 135 | 135 | 109 | 0 | 0 | 0 |
| | **sum** | 25 113 | 13 798 | | **201** | 150 | 164.6 `*` |
| PC dialogs | `new_machine` | 2 295 | 1 416 | 0 | 20 | 21 | 1.2 `*` |
| | `new_machine_view` | 1 207 | 1 207 | 1 167 | 5 | 0 | 0 |
| | `settings_model` | 1 782 | 735 | 0 | 6 | 25 | 2.8 `*` |
| | `settings_sheet` | 1 461 | 1 067 | 939 | 13 | 6 | 0.4 `*` |
| | `image_settings` | 1 536 | 805 | 0 | 13 | 15 | 1.1 `*` |
| | `firmware_choices` | 511 | 447 | 0 | 9 | 2 | 0.1 |
| | `layers_dialog` | 1 919 | 957 | 450 | 8 | 14 | 4.1 `*` |
| | **sum** | 10 711 | 6 634 | | **74** | 83 | 9.7 `*` |
| Media | `media_picker` | 4 161 | 2 445 | 1 567 | 1 | 27 | 3.7 |
| | `media_dialog` | 2 559 | 561 | 0 | 10 | 22 | 8.3 |
| | `media_create` | 1 718 | 1 451 | 837 | 8 | 4 | 0.4 |
| | `created_media` | 916 | 744 | 0 | 26 | 5 | 0.7 |
| | `kept_catalogue` | 1 394 | 1 083 | 0 | 18 | 7 | 0.6 |
| | `catalogue_menu` | 1 196 | 624 | 172 | 6 | 11 | 0.9 |
| | `drive_chips` | 1 033 | 545 | 0 | 1 | 14 | 1.0 |
| | **sum** | 12 977 | 7 453 | | **70** | 90 | 15.6 |
| Shell and session | `shell`, `shell_model`, `main`, `app_menu`, `macos_menu`, `preferences`, `preferences_sheet` and the test-only `host_audio`, `mouse`, `key_table_tests`, `sound_levels_tests`, `shell_icons` | 22 349 | 8 258 | | 84 | 287 | 64.0 `*` |

`kept_catalogue` is newer than §2.5 and belongs to the media group. The
library's test fixture, `library_test_support` (471 lines), is in no row.
The preliminary figures of §2.8 put the three sound-level tests at 44 s;
here two of the three failed at once.

What each group takes from the PC crates, and why [REPO, in the named
file]:

- **Registries and Remotes: 18 references of six kinds.** None of them is
  PC behaviour; each is a name or a list a platform can state.
  - The push guard's catalogue: `registry_transfer::run`, `plan` and
    `run_push` take `catalogue: &[assembly::FirmwareEntry]` and call
    `platforms::push_plan` (5 references).
  - Two labels of blob kinds, `pc_image::CMOS` and `pc_image::DISK_ZERO`,
    in the transfer's list of blobs (2, `registry_transfer.rs`).
  - Which platform this program runs: `pc_image::PLATFORM_NAME` and
    `pc_image::is_pc` in `TagRecord::other_platform` (2,
    `remote_browse.rs`).
  - The name of a medium's format: `pc_image::media_set::format_name` (1,
    `remote_browse.rs`).
  - The order of version tags: `pc_image::template::precedence` (3 in
    `remote_browse.rs`, 2 in [`library_view/remotes.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/remotes.rs)). It is a SemVer
    comparison over `Store::is_version_tag` and names nothing of a PC
    [REPO: [`crates/pc-image/src/template.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/template.rs)].
  - Which drives take a pulled set: `assembly::FloppyGeometry` and
    `assembly::descriptors::FLOPPY_TYPES` in `remote_pull::Drive` and
    `drives_for` (3).
- **The library worker: 124 references to about 70 distinct PC items.** Of
  its 33 requests, 24 are answered by PC code, and so is each of the 16
  machine jobs; 21 of its 41 events carry a PC type (`ImageDetails`,
  `SessionTransfer`, `revision::Plan`, `machine::Settings`, the firmware
  and media-import reports, the created and kept disks). What is neutral
  in it is the loop: the thread, the channels, the cancel counter, the
  summary cache and the snapshot after each job. The nine neutral requests
  are `Query`, `Delete`, `Edit`, `Finish`, `Assembled`, `ReadHost`,
  `Shutdown`, and `Transfer` and `Browse` through the modules above. Its
  three start flows (an image, a machine, a Gaming-mode copy of a machine)
  each call PC code before, during and after the session they serve
  (`prepare_pinned`, `machine::prepare`, `prepare_copy`, `resume_copy`,
  `created_media::apply`, `kept_catalogue::restore`, `machine::confirm`).
- **The frame, `LibraryView`: 10 references of its own, PC by what it
  owns.** Of its 64 fields, 28 are typed by a PC crate or by a PC module
  of the window (the readiness of the selected image, the launched
  `SessionTransfer`, the wizard, the settings sheet, the layers dialog,
  the firmware and media dialogs, the machine rows) and 4 by the registry
  dialogs and Remotes. It is implemented in six files: `library_view.rs`,
  `machines.rs`, `firmware.rs`, `install_media.rs`, `remotes.rs` and
  [`remotes/details.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/library-ui/src/remotes/details.rs) each hold an `impl LibraryView` block.
- **The collections.** `machines` holds `pc_image::machine::Listing` rows
  with the floppy drives, created and kept disks of each machine;
  `firmware` and `firmware_metadata` are the PC firmware catalogue and
  `pc_image::firmware_import`; `install_media` is
  `pc_image::media_import` (15 of its references); `hardware` reads
  `assembly::capabilities` and has no view line at all.
- **The media group: 70 references, no neutral module.** `media_dialog`,
  `created_media`, `kept_catalogue` and `drive_chips` have no view line
  and are the PC's floppy catalogue (`assembly::FloppyMedium`,
  `WriteMode`, `ImageSource`, `pc_image::machine`,
  `pc_image::workspace::PreparedSession`). `media_picker` names a PC crate
  once, but stands on `media_create` and `created_media`.
- **The PC dialogs** are what §8 says they are; three of them
  (`new_machine`, `settings_model`, `image_settings`) and
  `firmware_choices` have no view line.

How the modules point at each other (COUNTED, production code):

- `library_worker` takes from `library_view`: `ImageRow`, `Query`,
  `snapshot`, `delete_tag`, `host_last_run`, and from its children the
  model halves of `firmware` (`ScanReport`, `Outcome`, `scan_folder`,
  `preview_set`, `import_selected`), `firmware_metadata` (`Verdict`,
  `EditSource`, `PackageDetails`, `open_edit`, `preview_edit`,
  `publish_edit`, `read_package`), `install_media` (`Pick`, `Control`,
  `Sources`, `Outcome`, `inspect`, `import`) and `machines`
  (`MachineRow`). `library_view` takes back `Request`, `Event`,
  `MachineJob`, `Worker`, `Recovery`, `Finish`, `WorkspaceKind`,
  `CheckedImage`, `regular_name`. That is the cycle of §8 step 3.
- The worker also takes `created_media` and `kept_catalogue` (22
  references), `image_settings::describe`, `layers_dialog::SourceImage`,
  `save_image::{Target, local_tags}`, `registry_transfer::run` and
  `remote_browse::run`.
- The media models form their own knot: `created_media` and
  `kept_catalogue` name `media_dialog`, and model items that live in two
  view files (`catalogue_menu::{SetDisk, set_disk, short_name}`,
  `media_picker::{SetEntry, CatalogueDisk, drive_name}`); `media_dialog`
  names `created_media::Verified`.
- `human_size` (in `library_view`) is named by ten modules.
- The library views and `catalogue_menu` push the shell's `Command` (they
  name it 36 times, tests included), and the frame lists the shell's
  `LibraryEntry`; both are defined in `shell_model`.
- The session, `Presenter` in `main.rs`, names `drive_chips` in two
  methods (`drive_views`, `observe_drive_lights`); no media or library
  module names `Presenter` outside its tests.

#### 8.1.2. What the counts decide

1. **The registry side is neutral and gets a small seam.** 9 696
   production lines with 18 references that a platform can answer as
   data. This is the part of the desktop that matches the neutral half of
   the command line (`image push`, `pull`, `remote ls`, `login`,
   `registry set`), and its seam shares four hooks with `CliPlatform`:
   the platform, `validators`, `restricted` and `medium_format`.
2. **The worker's loop is neutral, its jobs are not, and they are not
   pulled apart now.** Making the worker neutral means one of two things.
   Either its PC requests and events become values the worker cannot read
   (the pattern of `PlatformCommand`), with the three start flows turned
   inside out so that PC code calls the neutral loop while it holds a
   session; or machines, settings, firmware import, media import and
   readiness become neutral data, which is option (b) of §11 Q3, refused
   there, and the "settings as data" that §3.12 leaves out of version 0.1.
   Both are generalisation for one platform, with nothing but the PC's
   own tests (the worker's 30, the frame's 64) to check the new seam
   against. The worker stays whole and moves to the `pc` layer; §8.1.4
   says how it grows a seam when a second platform has a library.
3. **The frame is PC and moves last.** It owns the PC dialogs as fields,
   so it cannot sit below them, and a neutral frame would need every
   collection behind a trait. `machines` and `install_media`, which §8
   listed with the neutral views, follow it: their counts are 14 and 15.
4. **Nothing in the media group is neutral.** §8's `media-ui` keeps its
   name and its contents and changes its layer to `pc`.
5. **Model and view are mixed inside files**, not only between them.
   Step 3 therefore splits nine files (`save_image`, `library_view`,
   `firmware`, `firmware_metadata`, `install_media`, `machines`,
   `layers_dialog`, `catalogue_menu`, `media_picker`), and the compiler
   draws the line: an item is model when it builds without `egui` and
   `ui-kit`.
6. **Most tests of the neutral modules need a real PC.** The test modules
   of `registry_transfer`, `remote_browse`, `remote_pull`, Remotes and the
   transfer view (95 of the 122 tests of the registry side) take their
   fixtures from `pc_image` (`PcProvider`, `PLATFORM`, the synthetic
   firmware), from the worker or from the frame: the fixture of
   `registry_transfer`'s tests holds a `LibraryView`. §7.3 forbids a
   neutral crate a PC test dependency, so they stay in `pc` crates, as the
   tests that need a real PC stayed in `window` in step 2.

#### 8.1.3. Target crates

These rows replace the rows `library-ui`, `media-ui`, `pc-ui` and
`unemupc` of §8's table; `ui-kit` and `machine-view` stand as landed.

| Crate | Layer | Toolkit | From today's modules |
|---|---|---|---|
| `library-model` (new) | neutral | none | `registries_model`, `registry_transfer`, `remote_browse`, `remote_pull`, the model half of `save_image`, the listing types of `library_view` (`ImageRow`, `Listed`, `listed`, `Source`, `source`, `human_size`, `delete_tag`, `host_last_run`), and the provider seam (`DesktopPlatform`, `PlatformFacts`, §8.1.4) |
| `library-ui` | neutral | egui | The table helpers of `library_view` (`Cell`, `Columns`, `fit_columns`, `table_width`, `sort_header`, `draw_cells`, `draw_cells_with_hints`, `group_row`, `name_cell`, `Lead`), the view half of `save_image`, `registry_transfer_view`, `library_view/remotes` and `remotes/details` |
| `pc-media` (new) | pc | none | `media_dialog`, `created_media`, `kept_catalogue`, `drive_chips`, and the model items of `catalogue_menu` and `media_picker` those four name |
| `pc-library` (new) | pc | none | `library_worker`, `firmware_choices`, `hardware`, `image_settings`, the model halves of `firmware`, `firmware_metadata`, `install_media` and `machines`, `Query`, `Filter`, `Snapshot` and `snapshot`, `new_machine`, `settings_model`, the machine-file list of `shell_model` (`LibraryEntry` and what builds it), the PC's `DesktopPlatform`, and the library's test support behind a `test-support` feature |
| `media-ui` | pc | egui | `media_picker`, `media_create`, the view of `catalogue_menu` |
| `pc-ui` | pc | egui | The frame (`library_view` with the views of `machines`, `firmware`, `firmware_metadata`, `install_media`, the PC half of `pane`, `layout_tests`), `settings_sheet`, `new_machine_view`, `layers_dialog`, and the shell's command vocabulary (`Command`, `State`, `Enablement`) |
| `unemupc` (today the package `window`) | application | egui | `shell`, the rest of `shell_model`, `app_menu`, `macos_menu`, `preferences`, `preferences_sheet`, `main`; the session (`Presenter`), which stays here; its tests that drive a real PC moved to the test-only `pc-session-tests` (`upc-dfs.25.7`, §8.1.10) |

What each depends on inside the workspace:

```text
library-model   image-store, image-registry, unemu-core-api
library-ui      library-model, ui-kit
pc-media        pc-image, assembly, machine-file, media, image-store
pc-library      pc-media, library-model, pc-image, assembly, machine-file,
                unemu-pc-core (the platform's description)
media-ui        pc-media, ui-kit
pc-ui           pc-library, pc-media, library-ui, library-model, ui-kit,
                machine-view (the `Speed` a command carries)
unemupc         all of the above, machine-view, unemu-pc-core, platforms
```

What changed against §8's table, and why:

| §8 | Here | Why |
|---|---|---|
| `library-ui`, neutral, holds the model, the worker and the views | Two neutral crates, `library-model` without a toolkit and `library-ui` with egui | Step 3 is "without egui". A model crate that links no toolkit keeps the registry models and their tests out of every view change, and lets the `pc` model crates below the views depend on it |
| `library_worker` in neutral `library-ui` | `pc-library`, layer `pc` | 124 PC references; 24 of 33 requests and 21 of 41 events are PC (§8.1.2, 2) |
| `library_view`, `machines`, `install_media` in neutral `library-ui` | `pc-ui` and `pc-library` | 28 of the frame's 64 fields are PC; 14 and 15 PC references (§8.1.2, 3) |
| `media-ui`, neutral with PC pieces split off | `media-ui`, layer `pc`, and its models in `pc-media` | 70 PC references; the one module that names a PC crate only once, `media_picker`, stands on two that name them 8 and 26 times. The worker and the session need the media models below them, without a toolkit |
| `drive_chips`, `created_media` in `media-ui` | `pc-media` | No view line; the worker names `created_media` and `kept_catalogue`, the session names `drive_chips` |
| `image_settings`, `firmware_choices`, `library_view/hardware`, `new_machine`, `settings_model` in `pc-ui` | `pc-library` | No view line; the worker names the first three |
| The PC views move in step 6, the library views in step 4 | The PC dialogs and the frame move together in step 6; step 4 moves the neutral views | The frame owns the dialogs as fields, so it cannot land in a crate below them |

Six crates come out of the window instead of three. Each line between
them is one the rules force: neutral against `pc` (§7.3), with a toolkit
against without (step 3), and the media views beside the library views,
which do not name each other (§8).

#### 8.1.4. The desktop's provider seam

The neutral crates ask a platform five things. The trait lives in
`library-model`, and whoever calls a neutral function hands the platforms
in, as `upcli` hands `&[&dyn CliPlatform]` to the neutral command line. In
steps 3 to 6 the caller is the PC library, whose worker runs the transfers
and whose frame owns Remotes, so the list holds the PC alone; a list the
application builds arrives with a neutral worker (the last part of this
section).

```rust
use image_store::{ConfigValidator, Digest, Manifest, Store};
use unemu_core_api::PlatformDescription;

/// One platform's part of the desktop library: what the platform-neutral
/// registry and Remotes code asks about artifacts it cannot read itself.
///
/// The desktop's counterpart of the command line's `CliPlatform`
/// (core-contract.md §7.2): `validators`, `restricted` and `medium_format`
/// mean what they mean there. The caller hands the platforms in; no
/// neutral crate names one.
pub trait DesktopPlatform {
    /// The platform as its core describes it: the id artifacts name, in its
    /// current spelling, and the name people read. Ids are compared with
    /// `image_store::platform::same` only.
    fn description(&self) -> &'static PlatformDescription;

    /// Validators for every artifact type of the platform. Push and pull
    /// pick the one whose artifact type and platform match the manifest.
    fn validators(&self) -> Vec<&dyn ConfigValidator>;

    /// Digests of a manifest's blobs the platform knows must not be
    /// redistributed, whatever the image's own marks say: the input of the
    /// push guard, for the dry run and for the push.
    fn restricted(&self, store: &Store, manifest: &Manifest) -> image_store::Result<Vec<Digest>>;

    /// The name of a medium's format, from its table in a media set's
    /// config, as Remotes lists a set's media; `None` for a table the
    /// platform does not read.
    fn medium_format(&self, format: &toml::Table) -> Option<&'static str>;

    /// The platform's own payload media types with a name for each, as a
    /// transfer lists the blobs of an image. A type no platform names is
    /// shown by its media type.
    fn payload_labels(&self) -> &'static [(&'static str, &'static str)];
}

/// What the views and the registry models read of the compiled-in
/// platforms without calling them. Plain data: the frame makes it once and
/// the dialogs clone it, so nothing on the UI thread borrows a platform.
#[derive(Clone, Debug, Default)]
pub struct PlatformFacts {
    platforms: Vec<&'static PlatformDescription>,
    payload_labels: Vec<(&'static str, &'static str)>,
}

impl PlatformFacts {
    pub fn of(platforms: &[&dyn DesktopPlatform]) -> Self;

    /// Whether a compiled-in platform runs artifacts that name `id`.
    pub fn runs(&self, id: &str) -> bool;

    /// The name of the machines this program runs, as "this release runs
    /// …s" says it: the first platform's name.
    pub fn family(&self) -> &str;

    /// The name of a payload media type, when a platform states one.
    pub fn payload_label(&self, media_type: &str) -> Option<&'static str>;
}
```

The functions that take it, in `library-model`:

```rust
// registry_transfer: `platforms` replaces `providers` and `catalogue`.
pub fn run(
    store: &Store,
    request: TransferRequest,
    platforms: &[&dyn DesktopPlatform],
    send: &(dyn Fn(TransferEvent) + Sync),
    repaint: &(dyn Fn() + Sync),
);
pub fn kind_label(facts: &PlatformFacts, descriptor: &Descriptor) -> String;

// remote_browse
pub fn run(
    store: &Store,
    request: BrowseRequest,
    platforms: &[&dyn DesktopPlatform],
    send: &dyn Fn(BrowseEvent),
);
impl TagRecord {
    pub fn other_platform(&self, facts: &PlatformFacts) -> Option<String>;
    pub fn not_runnable(&self, facts: &PlatformFacts) -> Option<String>;
}

// remote_pull: a drive of a stopped machine, as data.
pub struct Drive {
    /// The machine's id and name.
    pub machine: String,
    pub name: String,
    pub unit: u8,
    /// The drive as its platform names it, such as `A:`.
    pub slot: String,
    /// The drive's own format name, such as `1.44M`.
    pub format: String,
    /// Format names of the media the drive takes, its own first: what
    /// `SlotDescription::formats` (§3.8) says of a running machine's slot.
    pub takes: Vec<String>,
    pub holds: Option<String>,
}
```

How each hook is cut, with the words on the screen unchanged:

| Today | Becomes |
|---|---|
| `platforms::push_plan(store, tag, target, entry, catalogue)` | `image_registry::push::plan_with_catalogue(store, tag, target, entry, \|manifest\| …)` with the digests every platform's `restricted` returns, joined |
| `providers: &[&dyn ConfigValidator]` | every platform's `validators`, in the order of the list |
| `pc_image::CMOS => "CMOS"`, `pc_image::DISK_ZERO => "disk base (zero)"` | `PlatformFacts::payload_label`, after the neutral media types |
| `pc_image::is_pc(id)`, `pc_image::PLATFORM_NAME` | `PlatformFacts::runs(id)`, `PlatformFacts::family()`. The PC core already describes itself as `ibm-pc`, "IBM PC compatible" [REPO: `crates/pc-core/src/core.rs`]; a test in `pc-library` asserts that these equal `pc_image::PLATFORM` and `PLATFORM_NAME` |
| `pc_image::media_set::format_name(table).ok()` | the first platform's `medium_format(table)` |
| `pc_image::template::precedence` | `image_store::tag::precedence`: the function moves to the neutral crate that already holds `tag::compare` and `tag::newest_first`, and `pc_image::template` re-exports it |
| `Drive::geometry` with `accepted_media()` and `FLOPPY_TYPES` | `Drive::format` and `Drive::takes`, filled by the PC code that lists a stopped machine's drives ([`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs), two places, and `stopped_drives` of the frame) |

With one platform every text is byte for byte today's. A hook that names
one artifact asks the first platform, as the command line does (§7.5
step 4); the choice among several is made with the second platform. No
new file spells the platform id out: the neutral crates read it from the
description, so the platform-id scan of `image-store` needs no new entry.

The PC's implementation, in `pc-library`:

```rust
/// The PC's part of the desktop library.
pub struct PcDesktop<'a> {
    validators: &'a [&'a dyn ConfigValidator],
    catalogue: &'a [assembly::FirmwareEntry],
}

impl<'a> PcDesktop<'a> {
    /// `validators` is the PC's provider list; `catalogue` is the firmware
    /// catalogue whose proprietary images restrict a push.
    pub fn new(
        validators: &'a [&'a dyn ConfigValidator],
        catalogue: &'a [assembly::FirmwareEntry],
    ) -> Self;

    /// The PC's facts, for the frame and its dialogs.
    pub fn facts() -> PlatformFacts;
}

impl DesktopPlatform for PcDesktop<'_> { /* description: PcCore's platform;
    restricted: pc_image::firmware::restricted(manifest, self.catalogue);
    medium_format: pc_image::media_set::format_name(table).ok();
    payload_labels: (pc_image::CMOS, "CMOS"), (pc_image::DISK_ZERO, "disk base (zero)") */ }
```

The worker builds it where it runs a transfer, from the two things its
`Backend` already supplies there, so the tests' synthetic catalogue and
the injected crash keep working and a transfer still looks the catalogue
up once [REPO: `library_worker.rs`, `Request::Transfer`]:

```rust
Request::Transfer(job) => backend.with_providers(|providers| {
    let pc = PcDesktop::new(providers, backend.firmware_catalogue());
    library_model::registry_transfer::run(&store, job, &[&pc], &send_transfer, &repaint)
}),
```

Two supporting moves keep the layers legal:

- **The PC's provider list moves from `platforms` to `pc-image`**
  (`pc_image::with_providers`, landed with the CLI split), and `platforms` forwards to it. A
  `pc` crate may not depend on `platforms`, which is `application`, in
  its tests either; the worker and the tests of the registry models need
  the list. The future `cli-pc` needs the same move (§7.1).
- **The worker's wake-up is a function, not an `egui::Context`**:
  `pub type Wake = Arc<dyn Fn() + Send + Sync>`. The frame passes
  `move || ctx.request_repaint()`. This is the worker's only use of the
  toolkit (6 references).

**What the seam does not cover, and what brings each part in.** A neutral
library would also list local images, show their details, act on them and
start them. In steps 3 to 6 the neutral crates do none of the four; the
PC library does, as today.

| Part | Stays PC because | Enters the seam when |
|---|---|---|
| Listing local images | `snapshot` folds the firmware and installation-media collections and the firmware replacement hints into one answer (`firmware_choices`, the catalogue). The row type and the table are neutral already (`library-model`, `library-ui`) | a second platform lists images: `snapshot` splits into the neutral rows and one hook for the hints |
| Details | a readiness check answers `pc_image::inspection::ImageDetails` and a `capabilities::Draft` | a second platform has a details pane: the check returns the platform's own value, and the platform draws it |
| Actions | 24 requests and 16 machine jobs with PC arguments | a second platform has a worker job: the platform's requests and events become associated types the loop passes through |
| Launch | the worker's three flows prepare PC sessions and keep host media bindings; the presenter keeps a typed `Session<PcMachine>` and PC catalogue model | Stage B landed: `DesktopLaunch` carries `pc_image::launch::Prepared`; the shell calls `core_launch::start_with` and `PcCore::create_machine`. Host bindings remain beside the launch for PC media tools (§14.3 S13, S14); a neutral library worker is still separate work |

For that later step the shape is recorded here so that nothing in steps 3
to 6 works against it; it is **not** part of them:

```rust
/// A platform whose library the neutral worker serves. Sketch.
pub trait LibraryPlatform: DesktopPlatform + Send + 'static {
    /// The platform's own requests and answers; the worker passes them
    /// through unread, as the command line passes a `PlatformCommand`.
    type Job: Send + 'static;
    type Event: Send + 'static;
    /// A verified start, made on the worker thread. A `Machine` need not
    /// be `Send` (§3.5), so the machine itself is made on the UI thread
    /// from this value: by `Core::create` over the receipts it carries, or
    /// by the provider, which returns its own session.
    type Prepared: Send + 'static;

    fn run(&self, store: &Store, job: Self::Job, send: &dyn Fn(Self::Event)) -> Result<(), String>;
    fn prepare(&self, session: &Workspace<'_>, image: &Descriptor) -> Result<Self::Prepared, String>;
}
```

#### 8.1.5. Rules of every move

Each move below is one bead under the rules of §7.5. In addition:

1. **Names stay.** A module that moves whole keeps its name in its new
   crate. A module that is split leaves its view half in its file, which
   re-exports the model half (`pub use`), so no other file changes its
   paths in the same bead. Renaming is never part of a move.
2. **A test moves with its module when it builds there.** A test that
   names a module still above its module's new crate, or, in a neutral
   crate, any PC crate, stays where it is, in a test-only module of the
   same path (as `shell_icons` stayed in step 1), and follows in the move
   that brings the last module it names down. No test is rewritten,
   dropped or weakened. A test's body changes only where a function it
   calls gained a parameter the move states.
3. **The name proof.** `cargo nextest list --workspace` before and after:
   with the package and binary prefix removed, and the renames the move
   states applied, the two sorted lists are equal. Today the window has
   732 tests and 4 ignored ones (MEASURED).
4. **The layer proof.** The new crate declares its layer;
   `scripts/crate-boundaries.sh` passes with the allow list unchanged. No
   move adds a line to the allow list: a test that would need one stays
   behind by rule 2. The move that creates a crate adds its name to the
   window family of the script (the `family` function, the comment above
   it and a self-test fixture), so the rule against CLI crates covers it.
5. **The runner.** A move whose tests read `vendor/`, `machines/` or
   `crates/pc-harness/tests/support/` adds the new crate to those data
   rules of `scripts/affected-tests.sh`; the runner's self-test fails on a
   path no rule covers. The bead's report gives
   `scripts/affected-tests.sh --plan` for one file of the moved module,
   before and after.
6. **Visibility.** `pub(crate)` and `pub(super)` become `pub` only for
   what another crate names. What only a test left behind names goes
   behind the crate's `test-support` feature, as in `ui-kit`,
   `machine-view` and `unemu-pc-core`.
7. **Time is reported, not promised.** Every time below is ASSUMED from
   the MEASURED per-module seconds of §8.1.1; at four tests at once the
   wall time is about a quarter of the sum. A move that adds more than a
   few seconds to the clean build says why (§10).
8. **A new crate** follows `conventions.md`: the SPDX line, a `//!` that
   says what the crate holds and names this section, and a `description`
   in its manifest.

#### 8.1.6. Step 3: the models and the worker, without egui

Twelve moves. 3.1 to 3.5 make the registry side neutral; 3.6 to 3.12 take
the PC models and the worker out from under the views, which cuts the
cycle.

| # | Move | Cut before it moves | Tests | Proof and effect |
|---|---|---|---|---|
| 3.1 | **Done.** **Two helpers into `image-store`.** `Precedence` and `precedence` go from `pc_image::template` to `image_store::tag`, re-exported where they were. A `TempDir` like `assembly::test_support::TempDir` (which stays for the emulation crates) goes behind `image-store`'s `test-support` feature | none | the tests that call only `precedence` move with it | The five call sites in `remote_browse` and `remotes` name `image_store::tag`. Plan: `image-store` and its dependents, as any `image-store` change |
| 3.2 | **Done.** **`library-model` starts with `registries_model`** (1 145 lines, no sibling module, no PC reference). The window re-exports it | none | 16 move, with the temp directory of 3.1 in place of `assembly`'s (6 uses); the one test of `merge_tests` names the application's `preferences` and stays | The first neutral desktop model by Cargo. The script's window family gains `library-model` |
| 3.3 | **Done.** **Listing and the save form.** `ImageRow`, `Listed`, `listed`, `Source`, `source`, `human_size`, `delete_tag`, `host_last_run` and the short digest (`firmware::short`) go to `library_model::listing`; the model of `save_image` (457 of 629 lines: `Target`, `TagLevel`, `TagCheck`, `check_tag`, `next_version`, `local_tags`, `SaveAsImage`) to `library_model::save_image`. `library_view` and `save_image` re-export them | none | the 11 tests of `save_image` move; the listing has none of its own | Cuts ten arrows into `library_view` (`human_size`) and the arrows of `image_settings` and the worker to `ImageRow` |
| 3.4 | **Done.** **`registry_transfer` on the seam, into `library-model`.** `DesktopPlatform`, `PlatformFacts` and `PcDesktop` appear (the last in the window until 3.11); the provider list moves to `pc-image` | the catalogue and the two labels (7 references), §8.1.4 | its 18 tests (15.2 s) hold a `LibraryView` and build stores with `PcProvider`; they stay, in a test-only `registry_transfer` module | The tests that pin the plan's lines, the refusals and the blob labels pass unchanged; the test that the PC core's platform equals `pc-image`'s is new. As landed: see below the table |
| 3.5 | **Done.** **`remote_browse` and `remote_pull`, into `library-model`** | the platform's name and id, the format name, the tag order (6); the drive's geometry (3), §8.1.4 | the 28 tests of `remote_browse` name the worker and stay; the 9 of `remote_pull/tests` draw the transfer view and stay with it | After this move about 6 100 production lines of registry models have no PC dependency. Plan for a file of them: `library-model`, `window`. As landed: see below the table |
| 3.6 | **Done.** **`pc-media` starts.** `media_dialog`, `created_media`, `kept_catalogue`, `drive_chips` move whole, with the model items of `catalogue_menu` (`SetDisk`, `set_disk`, `short_name`) and of `media_picker` (`Disk`, `SetEntry`, `CatalogueDisk`, `format_name`, `drive_name`, `title_parts`, `label_without_file`) that they name; both view files re-export them. The rest of `catalogue_menu` that builds without a toolkit (`Catalogue`, its rows and marks) may come in the same move | `BlankDisk::default()` becomes a constructor that takes the machine-file directory, and the shell passes `machines_dir()` (1 use) | `created_media` 5, `kept_catalogue` 7, `drive_chips` 14 move; the 22 of `media_dialog` name `Presenter` and the library fixture and stay | The four move together because they name each other. Plan for a media model file: `pc-media`, `window`. As landed: [`crates/pc-media`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-media) (layer `pc`, no toolkit) holds `created_media`, `kept_catalogue` and `drive_chips` whole, `media_dialog` but for two methods, the named items of `media_picker` (`SetEntry`'s `words` and `facet` became `pub` for the picker's search) and of `catalogue_menu` with `own_name`, which `short_name` calls and the picker's `disk_name` names; the rest of `catalogue_menu` stayed. `BlankDisk::new` takes the directory as an `Option`; `for_machine` passes none, since it sets its own. Deviations: the two methods of `media_dialog` that need the desktop stay in the window's `media_dialog.rs`, which re-exports the model: `InsertDialog::library_ui` draws with egui (a view line §8.1.1 missed, as it sits after the file's first test module) and `PendingMedia::apply` takes the session's `Presenter`; they are methods of two window traits (`LibraryInsertView`, `ApplyPending`), so their two callers in [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) keep their calls and import the traits. Of the 22 tests of `media_dialog`, the 12 of `media_dialog::tests` name neither `Presenter` nor the fixture and moved by rule 2; the 10 of `media_set_tests` and `review_tests` stay, and for them `LibraryMessage`, `LibraryInsert::{refresh, snapshot, pending}` and `PendingMedia::inserts` are `pub`, and the test-only `LibraryInsert::entries` is behind `test-support`. The glyph test of the window reads `pc_media::source_dir()` too (`test-support`, as in `ui-kit`). MEASURED: 38 tests in `pc-media`; plan for [`crates/pc-media/src/drive_chips.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-media/src/drive_chips.rs): `pc-media`, `window`, where it was `window` alone |
| 3.7 | **Done.** **`pc-library` starts** with `firmware_choices`, `hardware` (with `CheckedImage` from the worker and `firmware::is_rom`) and `image_settings` | none: after 3.3 they name only each other and `library-model` | `firmware_choices` 2 move; `hardware` 5 and `image_settings` 15 name the worker, the fixture or `settings_sheet` and stay | As landed: [`crates/pc-library`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library) (layer `pc`, no toolkit) holds `firmware_choices` with its 2 tests; the window's glyph test reads `pc_library::source_dir()` too. Deviation: `hardware` and `image_settings` stay in the window, with `CheckedImage` and `firmware::is_rom`: they name `human_size`, `ImageRow` and `SaveAsImage`, which reach `library-model` with 3.3, carried out beside this move and not yet under it; they follow once 3.3 has landed. MEASURED: plan for [`crates/pc-library/src/firmware_choices.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/firmware_choices.rs): `pc-library`, `window`. The rest, `hardware` and `image_settings`, landed with 3.8 to 3.10: see below the table |
| 3.8 | **Done.** **`Query` and `Filter`, then the model halves of `firmware` and `firmware_metadata`** (712 and 650 lines: the rows, facets and labels, `ScanReport`, `Outcome`, `scan_folder`, `preview_set`, `import_selected`, `Verdict`, `EditSource`, `PackageDetails`, `open_edit`, `preview_edit`, `publish_edit`, `read_package`) into `pc-library` | `firmware_metadata` plans its push with `plan_with_catalogue` and `pc_image::firmware::restricted` in place of `platforms::push_plan` (1) | stay with the views: they drive the frame | The fields of `Query` become `pub`. As landed: see below the table |
| 3.9 | **Done.** **The model halves of `install_media` and `machines`** (703 and 690 lines: `MediaRow`, `Availability`, `rows`, `all_rows`, `picker_set`, `Control`, `Pick`, `Sources`, `inspect`, `import`; `MachineRow`, `facets`, `visible`, `name_problem`, `id_for`, `setting_diffs`) into `pc-library` | none | stay with the views | As landed: see below the table |
| 3.10 | **Done.** **`Snapshot` and `snapshot`** (with `facets_for`, `sort_value`) into `pc-library` | none | stay with the frame | After this move the worker names nothing of `library_view` but through re-exports. As landed: see below the table |
| 3.11 | **Done.** **The worker.** `library_worker`, `layers_dialog::SourceImage`, `PcDesktop` and the test support (`library_test_support` without its three egui helpers, the synthetic firmware catalogue, `test_wait`) into `pc-library`; the support goes behind `test-support` | `Wake` in place of `egui::Context` (6) | its 30 tests (44.4 s) move, but for those that call test helpers of `layers_dialog` and `image_settings`, which follow in 6.2 | The cycle is cut by Cargo: `pc-library` cannot name the window. `crates/pc-harness/tests/support/` and `vendor/` gain `pc-library` in the runner's rules. As landed: see below the table |
| 3.12 | **Done.** **The tests left behind whose modules are now all in or below `pc-library`** move there, under the same paths: those of `remote_browse` (20 after 3.5 moved 8 of the 28 to `library-model`; the 28 took 9.8 s), and of `hardware` and `image_settings` where they name no view. The 15 of `registry_transfer` left after 3.4 hold a `LibraryView` in their fixture and wait for the frame (6.4) | none | about 40 tests, about 12 s | No production code changes. As landed: see below the table |

As landed, 3.1 to 3.3 (one bead each, no behaviour change; the workspace
lists the same 3958 test names apart from the crate, MEASURED with
`cargo nextest list --workspace --run-ignored all`):

- **3.1.** `Precedence` and `precedence` are in `image_store::tag`, and
  `pc_image::template` re-exports them; the five named call sites in
  `remote_browse.rs` and [`library_view/remotes.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/remotes.rs) name `image_store::tag`.
  The two other callers (the frame in `library_view.rs`, `image_settings.rs`)
  keep the old path through the re-export; both move to `pc` crates. The
  temporary directory is `image_store::test_support::TempDir`, behind the
  feature. Deviation: no test moved. The one precedence test,
  `template::tests::versions_order_by_semantic_version_precedence`, also
  names `template::ALIAS`, so by rule 2 it stays in `pc-image`, where it
  runs through the re-export.
- **3.2.** `library-model` ([`crates/library-model`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/library-model), layer `neutral`,
  counted as a window crate by the boundary check, with a self-test
  fixture) holds `registries_model`; the window's `registries_model.rs`
  re-exports it and keeps `merge_tests`. 15 tests moved, not 16: the
  sixteenth is the one of `merge_tests`, which stays as planned. All six
  uses of the temporary directory, the stayed test's included, are the
  store's. The stayed test imports by name what it took from the old
  module's own `use` lines; its body is unchanged. `RegistriesPage::wait`
  is behind `library-model`'s `test-support` feature, since the window's
  tests call it. `ENV_HINT` and `checks` stay private to the crate.
- **3.3.** `library_model::listing` holds `ImageRow`, `Listed`, `listed`,
  `Source`, `source`, `human_size`, `delete_tag`, `host_last_run` and
  `short`, with the private `tags` and `last_run_utc` they call;
  `library_model::save_image` holds the form's model with `Prompt`, its
  own `short` and its private helpers. `library_view.rs` re-exports the
  listing but `Source`, which no window file names (the import would be
  unused); [`library_view/firmware.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/firmware.rs) re-exports `short`; `save_image.rs`
  keeps `form`, `prompts` and their helpers and re-exports the model.
  Visibility: `ImageRow::repository`, `Listed::references` and
  `SaveAsImage::source_repo` became `pub`, since the frame and the form
  name them. 10 tests of `save_image` moved, not 11:
  `the_form_and_both_questions_render_without_a_window` draws the form with
  egui and stays in the window's `save_image::tests`, with copies of its
  two fixture helpers (`digest`, `tags`). The window's test that formats a
  last run reaches `last_run_utc` through
  `library_model::listing::test_support`, behind the feature.

Time of 3.1 to 3.3 (MEASURED on a loaded development host): the 25 moved tests take
2.4 s added up (`cargo nextest run -p library-model`). The runner's plan
for a change to [`crates/library-model/src/registries_model.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/library-model/src/registries_model.rs) or
`save_image.rs` is `library-model` and `window`; for the window's
`save_image.rs` it is `window` alone, which no longer builds or runs them.

As landed, 3.4 and 3.5 (one bead each, no behaviour change; MEASURED with
`cargo nextest list --workspace --run-ignored all`: the workspace lists the
same 3958 test names apart from the crate, plus the four new tests below,
3962 in all; `library-model` has 38 tests, the window 662):

- **3.4.** `library_model::platform` holds `DesktopPlatform` and
  `PlatformFacts` as §8.1.4 has them, with two helpers that join every
  platform's `validators` and `restricted` in list order;
  `library_model::registry_transfer` holds the model, whose `run` takes
  `platforms` and plans a push with `plan_with_catalogue` over the joined
  restrictions, and whose `kind_label` takes the facts. `PcDesktop` is the
  window's `pc_desktop` module; the worker builds it per transfer from its
  `Backend`'s providers and catalogue, as planned. The provider list was
  already `pc_image::with_providers`; the worker still reaches it through
  `platforms::with_image_providers`, which forwards to it. Deviations: the
  push and pull dialogs keep the facts for their blob labels, so
  `PushDialog::open` and `PullDialog::open` take them; the frame makes them
  once (`LibraryView::platform_facts`) and its dialogs clone them. 3 of the
  18 tests moved, not none, by rule 2: they name neither the window nor a
  PC crate (`a_refused_local_tag_says_what_is_wrong_with_it`,
  `blob_status_names_the_chunk_and_what_a_cancel_waits_for`, and
  `the_default_registry_follows_the_environment_and_a_bad_one_stops_both_dialogs`
  with `image-store`'s temporary directory and empty facts); the texts the
  15 stayed tests compare with are behind `test-support`
  (`registry_transfer::test_support`). The new tests: the PC core's platform
  equals `pc-image`'s, and the PC's payload blobs keep their names, both in
  the window beside `PcDesktop` until it moves (3.11); and two of the facts
  and the joined restrictions in `library-model`, over two toy platforms.
- **3.5.** `library_model::remote_browse` and `remote_pull` hold the two
  models. `remote_browse::run` takes `platforms`: a read tag's media
  formats come from the first platform's `medium_format`, and
  `TagRecord::other_platform` and `not_runnable` take the facts. Remotes
  browses with a `PcDesktop` over no validators and no catalogue: browsing
  asks only the description and the format names, and reading the
  catalogue would count against the worker tests' injected crash. `Drive`
  is the data of §8.1.4; the window's `remote_pull::pc_drive` fills it from
  a PC drive, `takes` with the standard format names of the drive's
  `accepted_media`, so `drives_for` answers as before. Deviations: `Pick`
  carries the facts (`refusal` and `RemotePull::open` keep their
  signatures; the dialog keeps a clone for its progress line); the views
  that state a tag's local state (`state_parts`, `repository_row`,
  `tag_row`, `state_text`) take the facts. The only PC code that lists a
  stopped machine's drives for this dialog is `stopped_drives` of the
  frame: the two places in [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) build the media picker's own `Drive`.
  8 of the 28 tests of `remote_browse` moved, not none, by rule 2, with the
  helpers only they use (`closed_port`, `row`, `view`, `grouped`, `digest`)
  and copies of `Setup` (over `image-store`'s temporary directory, browsing
  with no platform), `plain`, `open_registry`, `ended` and `names`:
  `roots_put_the_default_registry_first_and_contact_none`,
  `the_catalog_is_read_fifty_repositories_a_page`,
  `an_unreachable_registry_lists_nothing`,
  `log_in_is_offered_only_for_a_basic_challenge`,
  `whole_registry_entries_group_repositories_by_their_first_path_component`,
  `local_names_carry_the_host_except_for_the_family_default_registry`,
  `local_states_compare_within_one_host_qualified_name` and
  `a_failed_next_page_keeps_the_pages_before_it`. The 20 others build PC
  artifacts or browse through the worker and stay; so do the 9 of
  `remote_pull/tests`, which all build records of PC artifacts. For them
  `representative` and `Remotes::touch` are behind `test-support`, with the
  page sizes and annotation keys (`remote_browse::test_support`). Test
  bodies changed only where a function gained the stated parameter
  (`run`'s platforms, the facts), where a `Pick` gained its field and where
  a `Drive` literal became a `pc_drive` call (2).

Time of 3.4 and 3.5 (MEASURED on a loaded development host, `cargo nextest run -p
library-model`): the 38 tests of the crate take 3.9 s added up, under a
second of wall time. The runner's plan for [`crates/library-model/src/remote_browse.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/library-model/src/remote_browse.rs),
`registry_transfer.rs`, `remote_pull.rs` or `platform.rs` is `library-model`
and `window`; for the window's `registry_transfer.rs` it is `window` alone.

As landed, the rest of 3.7 and 3.8 to 3.10 (one bead, four moves, each
built and tested before the next; no behaviour change; no test moved, as
planned, so the workspace lists the same 3962 test names, the window 660
and `pc-library` 2, MEASURED with `cargo nextest list --workspace
--run-ignored all`). Every split file keeps its view half and re-exports
the model with `pub(crate) use pc_library::<module>::*`; the stayed test
modules import by name what they took from the old module's own `use`
lines, and their bodies are unchanged. An item that only a view uses
stays with its view, even where it builds without a toolkit.

- **3.7, the rest.** `pc_library::hardware` holds the module whole, with
  `CheckedImage`, which the worker fills and re-exports;
  `pc_library::image_settings` holds the model whole, and the window's
  `image_settings.rs` keeps its test module (whose helpers other tests
  use). `is_rom` went to `pc_library::firmware`, which 3.8 filled.
  Visibility: `ImageSettings::{source, machine, floppies}` became `pub`,
  since the stayed tests read them.
- **3.8.** `Query` and `Filter` are in `pc_library::snapshot` (`Filter`
  and its three predicates `pub`). `pc_library::firmware` holds the
  facets and labels, `Redistribution`, `FirmwareRow`, `MachineRoms`,
  `add_facets`, `rows`, the cells and their sort, and the scan and import
  (`ScanReport`, `CandidateView`, `CandidateState`, `Outcome`,
  `scan_folder`, `preview_set`, `import_selected`); the columns, the row
  lead, the pane with its helpers, `PaneAction`, `SourceTarget` (which
  the shell names) and the import dialog stay.
  `pc_library::firmware_metadata` holds the statement (`Rights`,
  `Catalogue`, `LicenseChoice`, `ProvenanceFields`, `SourceFile`,
  `Facts`, `Fields`), the verdict and the reason texts, `PackageDetails`
  and the worker's sides (`read_package`, `open_edit`, `preview_edit`,
  `publish_edit`); `read_package` plans with `plan_with_catalogue` over
  `pc_image::firmware::restricted`, as planned. Deviation: `EditSource`
  holds a `Draft`, so the editor's state, `Draft`, moved too, with its
  model methods; its four drawing methods and the licence picker of
  `LicenseChoice` became two window traits (`DraftView`, `LicensePicker`),
  so their callers keep their calls. For the views, four fields of
  `Draft` and the private methods and items the views call became `pub`;
  `provenance_hint`, which only the editor names, stays with it. For the stayed
  tests, `ProvenanceFields::{revision, url, statement}`,
  `MachineRoms::users` and `Redistribution::value` became `pub`, and
  `USER_SUPPLIED` and `class` are behind `test-support`.
- **3.9.** `pc_library::install_media` holds the facets and format
  labels, `MediaRow`, `Availability` with `availability`, `add_facets`,
  `rows`, `all_rows`, `picker_set`, the cells and their sort, and the
  import (`Control`, `Pick`, `Sources`, `inspect`, `Outcome`, `import`);
  `formats` and `is_set_payload` are behind `test-support` for the stayed
  tests. `pc_library::machines` holds `MachineRow` (but its lead),
  `FACETS`, `facets`, `visible`, `name_problem`, `id_for`, `SettingDiff`,
  `setting_diffs` and `pending_name`. Deviation: `MachineRow::lead`
  draws the toolkit's icon in the table's `Lead`, so it is a window trait
  (`MachineLead`). `pc-library` now depends on `pc-media`, as §8.1.3
  lists, for the media picker's `SetEntry` and a machine's created and
  kept disks.
- **3.10.** `pc_library::snapshot` holds `ViewRow`, `Snapshot` (fields
  `pub`), `facets_for`, `sort_value` and `snapshot`, with `image_rights`
  and `redistribution`, which the sort names. Every item the worker names
  through `library_view` now lives in `pc-library` or `library-model`; the
  worker's paths change when it moves (3.11).

`platforms::push_plan` has no caller in the programs after 3.8 (the
command line's test names it in a comment); removing it is left to the
move that empties `platforms`. Time (MEASURED, `cargo nextest run -p
window -p pc-library` after each move, four tests at once on a loaded
development host): 660 tests in 60 to 66 s of wall time, as before. The
runner's plan for [`crates/pc-library/src/hardware.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/hardware.rs) (or any other
`pc-library` file) is `pc-library` and `window`, where the plan for the
window's [`library_view/hardware.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/hardware.rs) was `window` alone.

As landed, 3.11 and 3.12 (one bead, two moves, each built and tested
before the next; no behaviour change). MEASURED with `cargo nextest list
--workspace --run-ignored all`: the workspace lists the same 4044 test
names apart from the crate, with one rename the move implies: the five
tests of the window's `library_view::hardware::tests` are
`hardware::tests` in `pc-library`, the path the module has had there
since 3.7. `pc-library` has 69 tests where it had 2, the window 599 where
it had 666. Moved test bodies changed only in paths to modules the new
crate names otherwise (`crate::created_media` is `pc_media::created_media`
there, `crate::kept_catalogue`, `crate::remote_pull` and
`crate::registry_transfer` likewise, `crate::library_view::hardware` is
`crate::hardware`) and where a worker is spawned with a `Wake`.

- **3.11.** `pc_library::library_worker` holds the worker whole, and the
  window's `library_worker.rs` re-exports it. `Worker::spawn` takes a
  `Wake` (`Arc<dyn Fn() + Send + Sync>`): the frame passes a closure that
  asks its `egui::Context` for a repaint; the tests that spawned a worker
  with a fresh context pass one that does nothing (three in the frame's
  tests, three in the worker's). The worker reaches the provider list
  through `pc_image::with_providers` instead of `platforms`, which only
  forwarded to it. `SourceImage` is `pc_library::layers_dialog`'s, which
  the dialog re-exports. `PcDesktop` is `pc_library::pc_desktop` with its
  two tests, which the window imports by that name. Behind `test-support`
  (which turns on `assembly`'s and `pc-image`'s, which the fixtures use):
  `pc_library::library_test_support` holds `Fixture`, `FirmwareStore`,
  `Medium`, `import_media_set` and `source_archive`, the synthetic
  firmware catalogue with the helpers that build it (`single`, `two_chip`,
  `tar`, `pinned_tar`) and `warm` (3.12); the window's
  `library_test_support.rs` keeps the three egui helpers and re-exports
  the rest, and `library_view::firmware::tests` re-exports the catalogue's
  helpers. `pc_library::test_wait` is the hang guard; the window imports
  it. The worker's test constructors and backends (`Worker::synthetic`,
  `recording`, `firmware_crash`, `media_crash`, `checked`) are behind the
  feature too. Visibility: the worker's items were crate-wide
  (`pub(super)` at the crate root, or `pub(crate)`) and the window names
  them, so they are `pub`. Deviation: 29 of the 30 tests moved, not "all
  but those that call test helpers of `layers_dialog` and
  `image_settings`": `the_save_form_starts_from_the_image_the_workspace_session_pins`
  calls `image_settings`' helpers, which 3.12 brings down, so it moved
  there by rule 2. `keep_publishes_then_removes_disposable_but_regular_publication_keeps_workspace`
  reviews the publication with `layers_dialog::tests` and stays in the
  window's `library_worker::tests`, with copies of the five helpers it
  calls (`until`, `worker`, `restarted`, `launch`, `finish`). The
  runner's data rules gain `pc-library` for `machines/` as well: the
  self-test requires both `machines/` and `vendor/` of a crate that builds
  paths from the repository root, as the firmware store does for the
  source archives.
- **3.12.** The 12 tests of `image_settings` that draw no sheet are
  `pc_library::image_settings::tests`; the three that call
  `settings_sheet` stay in the window's `image_settings::tests`, with the
  two drawing helpers. The helpers that build images for other tests
  (`open_new_image`, `open_image`, `write_c`, `opened`, `reviewed`) are
  `pc_library::image_settings::test_support`; both test modules re-export
  them under the old path. The five tests of `hardware` are
  `pc_library::hardware::tests`; their summary helper takes the cache
  from `library_test_support::warm`, which `library_view::firmware::tests`
  re-exports. 19 of the 20 tests of `remote_browse` are in a test-only
  `remote_browse` module of `pc-library`, as `shell_icons` stayed in step
  1. Deviation: `a_tag_needing_a_newer_release_is_refused_before_any_blob`
  pulls with `platforms::with_image_providers`, an `application` crate,
  so by rule 2 it stays in the window's `remote_browse::tests`, with the
  helpers it calls (`Image`, `blob`, `machine`, `metadata_of`, `machine_of`,
  `Setup`, `plain`, `open_registry`, `blob_requests`). No production code
  changed in this move.

Time (MEASURED, `cargo nextest run -p pc-library -p window`, four tests at
once on a loaded development host): 664 tests in 62 s of wall time; the 69
of `pc-library` take 159 s added up, the 595 the window runs 621 s (both
sums inflated by the load). The runner's plan for
[`crates/pc-library/src/library_worker.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/library_worker.rs) (or `remote_browse.rs`) is
`pc-library` and `window`; for the window's `library_worker.rs` it stays
`window` alone. The whole runner over this change (full plan, since
`Cargo.lock` changed): 4028 tests passed in 317 s.

Effect of step 3 (ASSUMED from §8.1.1): about 115 tests and 55 to 60 s of
the 288 s leave the application's test binary (the worker 44 s, the tests
of `remote_browse` 10 s, the small models). A change to the shell or to a
view then runs about 230 s of tests added up, about 60 s of wall time
where it is 76 s now. A change to a model runs its crate and everything
above it, as now. Four test binaries are linked in place of one.

#### 8.1.7. Step 4: the neutral library views

| # | Move | Cut before it moves | Tests | Proof and effect |
|---|---|---|---|---|
| 4.1 | **Done.** **`library-ui` starts** with the table helpers of `library_view`, the view of `save_image` (`form`, `prompts`) and `registry_transfer_view` (789 lines) | `kind_label` takes the facts, which the push and pull dialogs keep | the one test of `registry_transfer_view` builds a PC store and stays | The frame and the collections draw their tables from the crate. As landed: see below the table |
| 4.2 | **Done.** **Remotes off the frame, in place.** The `impl LibraryView` blocks of `remotes.rs` and [`remotes/details.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/library-ui/src/remotes/details.rs) (about 1 150 lines) become methods of `RemotesView`. What they read of the frame comes in as one borrowed value (the facts, the skin, whether the worker lives, the local tags, the stopped machines' drives as `remote_pull::Drive`, a function that sends a `BrowseRequest`); what they did to the frame goes out as a list of actions the frame applies (select an image, show a tag in the library, New machine from a pulled image, begin a pull, open a registry's login, open the picker on a set) | this move is the cut: the two blocks touch 19 fields of the frame besides `self.remote`, and four of its methods defined elsewhere (`send`, `select_image`, `pending_label`, `new_machine_named`) (COUNTED) | the 39 tests of `library_view::remotes::tests` drive the frame and pass unchanged | No crate changes; the diff is confined to the two files and the frame's calls into them. As landed: see below the table |
| 4.3 | **Done.** **Remotes into `library-ui`**: `remotes` and `remotes/details` | none after 4.2; the kit's pane items come from `ui_kit::pane` | stay with the frame, which re-exports the module | About 3 600 lines of registry views (Remotes, push, pull, the save form) have no PC dependency by Cargo. As landed: see below the table |

As landed, 4.1 to 4.3 (one bead, three moves, each built and tested
before the next; no behaviour change). MEASURED with `cargo nextest list
--workspace --run-ignored all`: the workspace lists the same 4074 test
names apart from the crate; one test moved (below), so `library-ui` has 1
and the window 598 where it had 599. `library-ui` ([`crates/library-ui`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/library-ui),
layer `neutral`, egui, `ui-kit`, `library-model`, `image-store`,
`image-registry`) is counted as a window crate by the boundary check, with
a self-test fixture, and the window's glyph test reads
`library_ui::source_dir()` too (`test-support`, as in `ui-kit`).

- **4.1.** `library_ui::table` holds the table helpers whole: `Lead`,
  `Cell`, `Columns`, `sort_header`, `fit_columns`, `table_width`,
  `group_row`, `draw_cells`, `draw_cells_with_hints`, `name_cell` and their
  private helpers and figures; the frame's `image_lead` and `tree_icon`
  name its rows and stay. `library_view.rs` imports what the frame, the
  collections and Remotes name; `CHILD_INDENT`, `NAME_ICON_ROOM` and `SCROLL_ROOM`, which only the
  frame's tests and `layout_tests` name, are behind `test-support`
  (`table::test_support`). `library_ui::save_image` holds `form` and
  `prompts` with their two helpers; the window's `save_image.rs` re-exports
  them beside the model. `library_ui::registry_transfer_view` holds the
  dialogs whole but the test module, which stays in the window's
  `registry_transfer_view::tests` as planned (it plans a push of a PC image)
  and imports by name the dialog types it took from the old module's own
  `use` lines. The cut was already made: `kind_label` took the facts in
  3.4. Deviation: `the_form_and_both_questions_render_without_a_window`
  moved, by rule 2: it draws the form over the model alone, so it is
  `save_image::tests` in `library-ui`. The two tests of the frame that
  draw only table cells (`selected_rows_and_items_are_legible_in_every_skin_and_theme`,
  `the_name_cell_draws_its_icon_and_badges`) stay in `library_view::tests`,
  whose module stays, as the tests of the split files did in 3.8 to 3.10.
- **4.2.** `RemotesView` holds every method of the two `impl LibraryView`
  blocks under its old name. What they read of the frame comes in as
  `RemotesContext` (the facts, the skin, the local tags, the stopped
  machines' drives); what they did to the frame goes out as an ordered
  `Vec<RemotesAction>` (`Browse`, `Select` a node, `ShowInLibrary`,
  `NewMachine`, `BeginPull`, `LogIn`, `Insert`), which the frame's
  `apply_remotes` carries out in that order. The frame keeps thin methods
  of the same names (`attach_remotes`, `select_remote`, `remote_list`,
  `apply_list` for the tests, …), so the shell and the 39 tests of
  `library_view::remotes::tests` are unchanged, and it took the three
  methods that are its own: `stopped_drives`, `show_in_library` and
  `new_machine_from`. Deviations: the browsing requests go out as
  `Browse` actions instead of a function in the context, and "whether the
  worker lives" stays with the frame: the frame's `send` fails the imports
  and transfers of the whole frame when the worker is gone, which a
  function lent beside `&mut self.remote` cannot do, so the frame sends
  each request and, with no worker, hands Remotes the ended event at once
  (`send_browse`, with `unsent`), as before. Showing a node is an action
  too (`Select`), so the frame clears its selection and the shell's
  selected file before Remotes shows the node, after any request the same
  frame sent before it, as before. `begin_remote_pull` writes the dialog
  into a slot it is lent, so a frame without logins keeps the dialog it
  has, as before. The frame lists the drives for each call that lends the
  context while Remotes is shown or Contents… is open, where before only
  the details pane and Pull… listed them; `can_pull_remote`, which the
  shell asks every frame, lends no drives, since whether a tag can be
  pulled does not depend on them.
- **4.3.** `library_ui::remotes` and its `details` hold the module whole,
  with the kit's pane items from `ui_kit::pane` and the short digest from
  `library_model::listing::short`; the window's [`library_view/remotes.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/remotes.rs)
  re-exports it and keeps `mod tests` (the 39 tests, unchanged), with the
  names they took from the old module's own `use` lines. Visibility: the
  types and methods the frame names became `pub`; `RemotesView` gained a
  `Default` (clippy's `new_without_default` for a public type). For the
  stayed tests, `RemotesView::{open, expanded, selected, multi, contents}`,
  `selection`, `Node::registry`, every field of `ListAnswer` (a test builds
  one with `..ListAnswer::default()`), `TreeLine::{label, tail}` and `Tail`
  became `pub`, and `ROW`, `COLUMNS`, `tree_lines`, `action_label`,
  `namespace_key`, `type_label` and `newest_first` are behind
  `test-support` (`remotes::test_support`). Deviation from rule 6 of
  §8.1.5: the fields and methods only the stayed tests name
  (`RemotesView::{open, expanded, selected, multi}`, `selection`,
  `Node::registry`, `ListAnswer` and its fields, `RemotesView::apply_list`,
  `TreeLine::{label, tail}`, `Tail`) are `pub` for every build, not behind
  `test-support`: a field, or a method the frame also calls, cannot be
  made public behind a feature without wrapping it under another name, and
  the tests would then change their bodies, which rule 2 forbids. The
  fields that became `pub` for stayed tests in 3.7 to 3.10 are the
  precedent. No test moved: 38 of them
  drive a `LibraryView`, and
  `expanded_tags_list_by_release_with_the_alias_before_its_revisions`
  builds its records with `pc_image::PLATFORM`.

Time (MEASURED, `cargo nextest run -p library-ui -p window`, four tests at
once on a loaded development host): 595 tests in 51 to 53 s of wall time
after each move, as before. The runner's plan for any `library-ui` file
([`table.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/library-ui/src/table.rs), `remotes.rs`, `registry_transfer_view.rs`) is `library-ui` and
`window`, where the plan for the window's `registry_transfer_view.rs`,
`save_image.rs` or [`library_view/remotes.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/remotes.rs) was `window` alone; it still
is for the window's [`library_view/remotes.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/remotes.rs), which now holds the tests.

Effect: none in time, since the tests of these views drive the frame. The
step is for the boundary.

#### 8.1.8. Step 5: the media picker and dialogs

| # | Move | Cut before it moves | Tests | Proof and effect |
|---|---|---|---|---|
| 5.1 | **Done.** **The catalogue menu answers with its own choice.** `catalogue_menu::item` returns which disk was chosen, or "all disks", and the shell pushes `Command::CatalogueDisk` and `CatalogueAll` | the two uses of the shell's `Command` | the 11 tests of `catalogue_menu` pass unchanged or gain the returned value where they read the pushed command | In place. As landed: see below the table |
| 5.2 | **Done.** **`media-ui` starts** with `media_picker` (the picker's state and view), `media_create` and the view of `catalogue_menu`; the pane colours come from `ui_kit::pane` | none after 5.1 | `media_picker` 27, `media_create` 4, `catalogue_menu` 11 move, but for the one that names the library fixture | Plan for a media view file: `media-ui`, `window`: about 5 s of its own tests, then the shell's. As landed: see below the table |

The 22 tests of `media_dialog` stay with the session until
`upc-dfs.25.7`.

As landed, 5.1 and 5.2 (one bead, two moves, each built and tested before
the next; no behaviour change). MEASURED with `cargo nextest list
--workspace --run-ignored all`: the workspace lists the same 4078 test
names apart from the crate; `media-ui` has 41 and the window 557 where it
had 598. `media-ui` ([`crates/media-ui`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/media-ui), layer `pc`, egui, `pc-media`,
`ui-kit`) is counted as a window crate by the boundary check, with a
self-test fixture, and the window's glyph test reads `media_ui::source_dir()`
too (`test-support`, as in `ui-kit`).

- **5.1.** `catalogue_menu::item` returns `Option<Chosen>`: `Chosen::Disk`
  with the catalogue index of the disk chosen, or `Chosen::All` for All N
  disks…, and takes no command list. The shell's `catalogue_item`, which
  both callers (the drive chip's menu and the Media menu) now call, pushes
  `Command::CatalogueDisk(unit, index)` or `Command::CatalogueAll(unit)`
  with the unit it asked about, which is the catalogue's own. The 11 tests
  pass unchanged but for the one call of `item` in the submenu-placement
  test, which loses the command list; none of them reads a pushed command.
  The shell's tests that click a disk or All N disks… and compare the
  pushed command pass unchanged.
- **5.2.** `media_ui::media_picker`, `media_create` and `catalogue_menu`
  hold the three modules whole, with the pane colours from `ui_kit::pane`
  and the media models from `pc_media` (`created_media`, `media_dialog`).
  The window's `media_picker.rs` re-exports the picker
  (`pub(crate) use media_ui::media_picker::*`); `catalogue_menu` is
  `media_ui`'s under its old name (`use media_ui::catalogue_menu` at the
  crate root, as `pc_media`'s modules are), and the window's
  `media_create` is gone, since no window file names it. Visibility: what
  the window names became `pub` (the picker's `Picker`, `Target`, `Drive`,
  `When`, `Source`, `Choice`, `Outcome`, `Mode`, `remotes`, `show` and its
  field `reveal`; Create from files…'s `Create` and `Item`; the
  submenu's `Catalogue`, `Pending`, `Row` with `Mark`, `Chosen`, `item`,
  `entry_name`, `slot_catalogue`, `waiting_catalogue`, `IN_DRIVE_MARK` and
  `OWN_DISKS_AT_START`; and the re-exported `drive_name`, `format_name`,
  `CatalogueDisk`, `SetEntry`, `set_disk` and `short_name`).
  `Picker::results` and `Create::outlook`, which no other crate calls,
  became `pub(crate)`, since their answer types stay crate-wide; `CAP`
  and `BEFORE` stay crate-wide too, so the public docs that name them
  (the module's, `Row::All`'s and `Catalogue::window`'s) name them
  without a link, which rustdoc refuses to a private item. The
  picker's fixtures that the shell's and the Installation media tests
  build with (`set`, `set_outcome`), and those the stayed test calls
  (`library`, `machine`, the three formats), are
  `media_picker::test_support` behind `test-support`; the media-ui tests
  import them, and the window's `media_picker::tests` re-exports `set` and
  `set_outcome` under the old path. 41 tests moved: the 11 of
  `catalogue_menu`, the 4 of `media_create` and 26 of the 27 of
  `media_picker`. `the_picker_keeps_its_size_frame_after_frame` calls the
  library fixture's `settled_modal`, which goes to `ui-kit` only in 6.2,
  so by rule 2 it stays in the window's `media_picker::tests`, unchanged.
  Moved test bodies changed only in paths the new crate names otherwise
  (`crate::created_media` is `pc_media::created_media`, the test-only
  `Disk` is `pc_media::media_picker::Disk`). Deviation: `media-ui` also
  depends on `assembly`, `image-store`, `image-registry`, `media` and
  `pc-image`, which the three modules name directly (floppy geometries and
  write modes, digests, the registry configuration of the From row, the
  FAT12 builder, the private repository's name); §8.1.3 lists only
  `pc-media` and `ui-kit`. Deviation from rule 6 of §8.1.5: some items
  are `pub` in every build although only the window's tests name them —
  `IN_DRIVE_MARK`, `OWN_DISKS_AT_START`, `Row` and `Mark`, and the
  already-public `Catalogue::disabled` and `Catalogue::rows` — because
  `Row` is the return type of a public function (the private-interfaces
  lint) and the tests that stayed name these paths in their bodies, which
  rule 2 forbids changing; the 3.x and 4.3 moves set the precedent.

Time (MEASURED on a loaded development host): the 41 tests of `media-ui`
take 0.4 s of wall time (`cargo nextest run -p media-ui`); `media-ui` and
the window together run the same 594 tests as the window did before (598
listed, 557 + 41, of which 4 are ignored), in
51.9 s, where the window alone took 50.5 s after 5.1. The runner's plan for
[`crates/media-ui/src/media_picker.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/media-ui/src/media_picker.rs) (or `catalogue_menu.rs`) is
`media-ui` and `window`; for the window's `media_picker.rs` or
`catalogue_menu.rs` it was `window` alone, and it still is for the window's
`media_picker.rs`, which now holds the stayed test.

#### 8.1.9. Step 6: the PC views, and what is left

| # | Move | Cut before it moves | Tests | Proof and effect |
|---|---|---|---|---|
| 6.1 | **Done.** **The dialog models into `pc-library`**: `new_machine` (1 416 lines) and `settings_model` (735) | `SettingsModel::open` takes the machine-file directory, as `open_with_library` does already; the shell passes `machines_dir()` | `settings_model` 25 move, but for the one that names `machines_dir_from`; of `new_machine`'s 21 those that draw `new_machine_view` stay | As landed: see below the table |
| 6.2 | **Done.** **`pc-ui` starts with the dialogs**: the PC half of `pane`, `settings_sheet`, `new_machine_view`, `layers_dialog`. The three egui test helpers of the library fixture (`settled_modal`, `Pointer`, `painted_at`) go to `ui-kit`'s `test-support`, since they name no PC | none | `settings_sheet` 6, `layers_dialog` 14, and the tests of `image_settings`, `new_machine` and the worker that waited for these views; those that drive the frame wait for 6.4 | As landed: see below the table |
| 6.3 | **Done.** **Commands and machine files down.** `State`, `Command`, `Enablement` (159 lines, which name only `machine_view::pacing::Speed`) go to `pc_ui::commands`; `LibraryEntry` and what builds it (`spawn_library_check`, `machine_paths`, `select_machine`, the diagnostics) go to `pc_library::machine_files`. `shell_model` keeps the hotkeys, the status-bar widths and the texture, and re-exports the rest | none | the 17 tests of `shell_model` go with what they test; the move lists the paths that change | After this move the frame names nothing of the application. As landed: see below the table |
| 6.4 | **Done.** **The frame.** `library_view` with the views of `machines`, `firmware`, `firmware_metadata`, `install_media`, `layout_tests`, and every test left behind whose modules are now in or below `pc-ui` | none: a move of whole files, about 9 000 production lines and 20 000 lines of tests | `library_view` 37, `layout_tests` 27, the collections 51, Remotes 39, `remote_pull` 9, `registry_transfer` 18 and its view's 1; the few that name `preferences` or the shell's hotkeys stay | **This is where the desktop's test time splits.** 140 s of tests move to `pc-ui` with this move (the frame 48.0, its layout tests 40.1, the collections 30.3, the registry and Remotes tests that drive the frame 21.7; MEASURED, `*`), beside the dialogs' of 6.1 and 6.2. A shell change then runs the application alone: about 70 s added up as measured here, about 110 s with the sound-level tests at their preliminary 44 s, so 20 to 30 s of wall time where it is 76 s now. A change to a library view runs `pc-ui` and the application. As landed: see below the table |
| 6.5 | **Done.** **What is left is the application**: `shell`, `shell_model`, `app_menu`, `macos_menu`, `preferences`, `preferences_sheet`, `main`. Optionally the package takes the name `unemupc` (`unemu` since decision-41, where `unemupc` is the core's command line: §12.9 and step 4 of §12.10), with the changes §7.5 step 6 lists for the command line (CI, the release workflow, `scripts/third-party-licenses.sh`, the reasons in `deny.toml`, the runner's rules, the script's window family, `commands.md`, `architecture.md`) | none | none move | No time effect. As landed: see below the table |

As landed, 6.1 to 6.3 (one bead, three moves, each built and tested before
the next; no behaviour change). MEASURED with `cargo nextest list
--workspace --run-ignored all`: the workspace lists the same 4117 test
names apart from the crate, with the renames of 6.3 below; the window has
476 where it had 557, `pc-library` 120 where it had 69, `media-ui` 42 where
it had 41, and the new `pc-ui` 29. `pc-ui` ([`crates/pc-ui`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui), layer `pc`,
egui) is counted as a window crate by the boundary check, with a self-test
fixture, and the window's glyph test reads `pc_ui::source_dir()` too
(`test-support`, as in `ui-kit`). No data rule of the runner changed: no
`pc-ui` source names a path outside its directory, and the rules that name
`pc-library` reach `pc-ui` as its dependent.

- **6.1.** `pc_library::new_machine` and `pc_library::settings_model` hold
  the two models whole; the window's files re-export them and keep the
  tests that stay. `SettingsModel::open` takes the machine-file directory
  (`Option<PathBuf>`) and the shell passes `machines_dir()`; the tests that
  open a file pass none, since every file they open lies in a `machines`
  directory, which is all the directory is asked for. 18 of the 22 tests of
  `new_machine` moved (the plan's 21 and the ignored user-ROM test); the
  four that draw `new_machine_view` stayed, with copies of the three
  helpers they call (`wizard`, `enabled`, `reason`). 24 of the 25 of
  `settings_model` moved; `custom_layer_of_a_file_outside_machines_stays_out_of_the_build_tree`
  names the application's `machines_dir_from` and stays in the window's
  `settings_model::tests`. Visibility: what the window names became `pub`
  (the wizard's steps, `Slot`, `RomRef`, `Offer`, `Selection`, `Wizard`
  and the re-exported `short` and `TagLevel`; `SettingsModel`); `Preview`
  stays crate-wide. `Wizard` gained a `Default` (clippy's
  `new_without_default` for a public type). Deviations: the model opened
  itself again in two places (the staged child it checks before a save,
  and the saved child after it), which asked `machines_dir()` each time;
  it now keeps the directory it was opened with in a private field and
  passes that. `open_with_library`, which only the stayed test calls, is a
  name for `open` behind `test-support`, so the test body is unchanged. The
  two tests of `settings_sheet` that open a model pass none too (a function
  they call gained the stated parameter).
- **6.2.** `pc-ui` holds `settings_sheet`, `new_machine_view` and
  `layers_dialog` whole, and the PC half of the panes as `pc_ui::pane`,
  which re-exports `ui_kit::pane` as the window's module did; the window's
  [`library_view/pane.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/pane.rs) and `layers_dialog.rs` re-export them, and the
  window names `settings_sheet` and `new_machine_view` at its crate root
  (`use pc_ui::{…}`), as it now names `library_worker` and `new_machine`
  (`use pc_library::{…}`), since no test stays in those four files. In
  `pc-ui`, `image_settings`, `library_worker` and `new_machine` are private
  modules that re-export the model beside the tests that came down, under
  the paths they had; `save_image` joins the form's model and view, and a
  test-only `library_test_support` joins the library's fixtures and the
  kit's helpers. `ui_kit::test_support` (behind `test-support`) holds
  `settled_modal`, `Pointer` and `painted_at`, which the window's
  `library_test_support` re-exports (only `settled_modal` is still named
  there). Tests moved: the 6 of `settings_sheet`, the 14 of
  `layers_dialog`, the 3 of `image_settings` that draw the sheet, the 4 of
  `new_machine` that draw the wizard, and the worker's
  `keep_publishes_then_removes_disposable_but_regular_publication_keeps_workspace`;
  and, by rule 2, `the_picker_keeps_its_size_frame_after_frame`, which
  waited only for `settled_modal`, to `media-ui`'s `media_picker::tests`.
  The disk-layer dialog's helpers that the frame's tests call (`finish`,
  `reviewed_publication`, and `source`, which the worker's test calls) are
  `layers_dialog::test_support` behind `test-support`; both crates'
  `layers_dialog::tests` re-export them under the old path. Moved test
  bodies changed only in paths the new crate names otherwise
  (`crate::library_view::ImageRow` is `library_model::listing::ImageRow`,
  `crate::library_view::firmware::tests::warm` is the fixture's `warm`, the
  picker's `settled_modal` is `ui_kit::test_support`'s). Visibility: what
  the window names became `pub` (the sheet's `show`, `show_image`,
  `show_review`, `firmware_tab` with `FirmwareEdit`, and their actions; the
  wizard's `show` and `Action`; `LayersDialog`, its `Action` and the
  re-exported `SourceImage`; the pane's `short_digest`, `newer_hint` with
  `HintAction` and `firmware_section`); `Sheet`, `why_disabled` and
  `capitalized`, which only `pc-ui` names now, stay crate-wide. The
  window's `library_view::hardware` re-export dropped the four names only
  the dialog used. Deviations: `firmware_tab` is `pub` for every build,
  although only the window's layout tests name it outside the crate, since
  the image sheet calls it too (the precedent of 4.3 and 5.2); `pc-ui`
  also depends on `assembly`, `image-store`, `image-registry` and
  `pc-image`, which the moved views name directly, and `media-ui` gains
  `ui-kit/test-support` as a test dependency.
- **6.3.** `pc_ui::commands` holds `State`, `Command` and `Enablement`
  (the speed is `machine_view::pacing::Speed`, so `pc-ui` depends on
  `machine-view`); `pc_library::machine_files` holds `LibraryEntry`,
  `PreparedEntry`, `LibraryCheck`, `spawn_library_check`, `machine_paths`,
  `select_machine` and the diagnostics (`relative_diagnostic`,
  `failed_firmware_paths`, `source_revision`). `shell_model` keeps the
  hotkeys, the status-bar widths and the texture and re-exports the rest,
  so the frame, the menus and the shell keep their paths. The 17 tests went
  with what they test; the paths that change: the nine of the machine-file
  list are `machine_files::tests::*` in `pc-library`
  (`a_missing_default_machine_is_neither_listed_nor_selected`,
  `cause_specific_hints_distinguish_media_firmware_and_integrity`,
  `discovery_includes_local_and_yaml_but_not_nested_or_non_machine_files`,
  `firmware_readiness_is_lazy_and_runs_on_a_host_worker`,
  `fix_round_library_ignores_dotfiles_and_pending_files`,
  `invalid_machine_is_visible_with_start_blocked`,
  `missing_local_media_names_the_media_cause_and_shortens_resolved_paths`,
  `row_warnings_exclude_pending_checks_and_skipped_roots`,
  `selection_keeps_a_missing_requested_machine_instead_of_starting_another`),
  `enablement_requires_ready_firmware_and_live_capabilities` is
  `commands::tests::*` in `pc-ui`, and the seven of the hotkeys, the
  status bar and the texture stay. `LibraryEntry::inspect_with_rom_dirs`,
  which the shell's tests call, is behind `pc-library`'s `test-support`;
  `inspect` stays test-only and `relative_diagnostic` crate-wide. What the
  frame names of `shell_model` now lives in `pc-ui` and `pc-library`; its
  paths change to those crates when it moves (6.4).

Time (MEASURED, `cargo nextest run` of `ui-kit`, `media-ui`, `pc-ui`,
`pc-library` and the window, four tests at once on a loaded development
host): 685 tests in 64 to 73 s of wall time after each move (622 of
`pc-library` and the window after 6.1). The runner's plan for a `pc-ui`
file is `pc-ui` and `window`; for [`crates/pc-library/src/new_machine.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/new_machine.rs)
or [`machine_files.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/machine_files.rs) it is `pc-library`, `pc-ui` and `window`; for the
window's [`settings_sheet.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/settings_sheet.rs) or [`shell_model.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell_model.rs) it was `window` alone.

As landed, 6.4 and 6.5 (one bead, two moves, each built and tested before
the next; no behaviour change). MEASURED with `cargo nextest list
--workspace --run-ignored all`: the workspace lists the same 4132 test
names apart from the crate, with no rename; the window has 304 where it
had 476 and `pc-ui` 201 where it had 29. No data rule of the runner and no
line of the boundary check's allow list changed: no moved source names a
path outside its crate, and the rules that name `pc-library` reach `pc-ui`
as its dependent.

- **6.4.** `pc_ui::library_view` holds the frame with its collections
  (`machines`, `firmware`, `firmware_metadata`, `install_media`, `hardware`,
  `pane`, `remotes`) and `layout_tests`, whole; `pc_ui::registry_transfer`
  and `pc_ui::remote_pull` re-export the neutral models with the tests that
  drive them through the frame (`remote_pull` with `pc_drive`, the PC's
  description of a stopped machine's drive). The frame names
  `pc_ui::commands::Command` and `pc_library::machine_files::LibraryEntry`
  in place of the window's `shell_model` re-exports, and its two tests that
  read `crate::shell_model::Command::Start` read `crate::commands` (a path
  the new crate names otherwise). In `pc-ui`, `media_picker` and
  `registry_transfer_view` are private modules under the paths the frame
  names, as `image_settings` and `save_image` are: the first re-exports
  `pc_media::media_picker` with `media-ui`'s fixture `set`, the second
  `library_ui::registry_transfer_view` with copies of the five drawing
  helpers (`texts`, `render`, `escape`, `shows`, `click`) the Remotes
  tests use. The window's `library_view.rs` and `registry_transfer.rs`
  re-export the moved modules and keep the tests that stay, and its
  `registry_transfer_view.rs` keeps its test without `click`, which only
  the moved tests called; `image_settings.rs`,
  `layers_dialog.rs`, `save_image.rs`, `remote_pull.rs` and their tests are
  gone from the window, since nothing there names them any more
  (`remote_pull` is `use pc_ui::remote_pull` at the crate root), and
  `registry_transfer_view` and `remote_browse` are test-only modules. 172
  tests moved: 36 of `library_view::tests`, the 27 layout tests, the 51 of
  the collections, 38 of Remotes, the 9 of `remote_pull` and 11 of
  `registry_transfer`. Seven stayed, by rule 2:
  `library_view::tests::platform_default_store_paths_and_preference_round_trip`
  (the application's `preferences`),
  `library_view::remotes::tests::refresh_lists_the_shown_registry_again`
  (the shell's `hotkey`), four of `registry_transfer::tests`
  (`the_guard_refuses_without_override_and_a_forged_job_reaches_no_registry`,
  `push_and_pull_name_the_namespace_and_the_guard_decides_per_namespace`,
  `a_package_marking_catalogued_proprietary_firmware_open_cannot_leave`,
  `a_move_or_stale_answer_after_cancel_shows_the_cancel`), which run a
  transfer through their helper `run_here` with
  `platforms::with_image_providers`, an `application` crate, and
  `registry_transfer_view::tests::push_and_pull_render_every_stage_without_a_window`,
  which calls it in its body (the precedent of 3.12). The fixtures that the
  stayed tests and the shell's tests build with are behind `pc-ui`'s
  `test-support` (which turns on `pc-library`'s, `library-model`'s and
  `image-registry`'s): `registry_transfer::test_support` (the push `Lab`
  with `accounts_for`, `login_policy`, `open_push`, `push_settled`,
  `wait_pumping` and the two constants), `library_view::remotes::test_support`
  (the `Rig`, its `preferences`, the sample listing and its helpers),
  `firmware::test_support` (`publish_open_package`, `poll_until`,
  `warm`, `select_first_package`, `firmware_edit_source`),
  `install_media::test_support` (the sets `library`, `disk`, `warm`,
  `FLOPPY_1440`, `select_set`, `reviewed_media`) and
  `machines::test_support` (`image`, `row`); the test modules re-export
  them under the old paths, and their bodies are unchanged. The test-only
  frame methods the shell's tests call (`synthetic`, `pump_until`,
  `pump_queued`, `set_machines`, `use_snapshot`) are `pub` behind the
  feature too. Visibility: what the window names became `pub` (`LibraryView`, `Request`,
  `default_store_path`, the four collection modules
  with their re-exported models, `SourceTarget`, `show_machine`,
  `stop_remotes`, `recheck`, and `remote_pull`'s re-export). The module
  `registry_transfer` holds the model's re-export for the frame, which
  names it; the window names only its `test_support`, in its tests. Deviations: `LibraryView::remote` is `pub` for every build,
  although only the shell's tests read it, since it is a field (the
  precedent of 4.3); `pc-ui` also depends on `media` (the frame reads host
  files for Create from files…), and as test dependencies on `cpu`,
  `device`, `video`, `media`, `image-store`, `image-registry`,
  `library-model` and `library-ui` with their test support, and on
  `media-ui`'s `test-support` for the picker's `set`, which one
  Installation media test builds a set with (a `pc` crate beside `pc-ui`,
  not above it); §8.1.3 lists none of these.
- **6.5.** What is left in the window is the application: `shell`,
  `shell_model`, `app_menu`, `macos_menu`, `preferences`,
  `preferences_sheet` and `main`, with the test-only `host_audio`, `mouse`,
  `key_table_tests`, `sound_levels_tests`, `shell_icons` and
  `library_test_support`. Besides them it holds only re-exports under old
  paths with the tests that stay (`library_view`, `media_picker`,
  `registries_model`, `registry_transfer`, `registry_transfer_view`,
  `remote_browse`, `settings_model`) and `media_dialog`, whose session half
  stays with `Presenter` until `upc-dfs.25.7` (§8.1.10). The package keeps
  the name `window`: the rename goes with the step that names the desktop
  binary `unemu` (§12.11, step 4 of §12.10).

Time (MEASURED, `cargo nextest run -p window` and `-p pc-ui` before and
after, four tests at once on a loaded development host): before, the
window ran 473 tests in 49.7 s of wall time (551 s added up) and `pc-ui`
29 in 3.9 s (47 s); after, the window runs 301 in 26.2 s (141 s added up)
and `pc-ui` 201 in 29.2 s (388 s). A change to the shell now runs the
window's tests alone. The runner's plan for
[`crates/pc-ui/src/library_view/machines.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/machines.rs) is `pc-ui` and `window`,
where the plan for the window's [`library_view/machines.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view/machines.rs) was `window`
alone. The whole runner over this change (full plan, since `Cargo.lock`
changed, goldens tier): 4116 tests passed in 420 s.

Order. Step 3 comes first and step 4 needs only 3.1 to 3.5. Steps 5 and 6
do not depend on each other; step 6 brings the time, so it may go before
step 5. Inside step 3 the two halves (3.1 to 3.5, 3.6 to 3.11) are
independent until 3.11, which needs both, so two workers can hold one
each.

#### 8.1.10. Where `upc-dfs.25.7` fits

`upc-dfs.25.7` plans a `pc` crate for the session tests that need a real
PC and stayed in the window in step 2: the session tests of `main.rs` (50
tests, 16.8 s), the three sound-level tests (44 s, PRELIMINARY), the six
host-audio tests and the mouse tests. They are the tests of `Presenter`,
so that crate is where `Presenter` lives.

- It depends on this plan in one place: `Presenter` names `drive_chips`
  (`drive_views`, `observe_drive_lights`), which is in `pc-media` after
  3.6. So it can start after 3.6 and needs nothing else of steps 3 to 6.
- It should sit **below** `pc-media`, with those two methods becoming
  functions of `drive_chips` over the session. The other way round, every
  change to a media model would run the sound-level tests.
- The 22 tests of `media_dialog` that put pending media into a
  `Presenter`, and the shell's start of a session (`poll_images` in
  [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs): `SessionTransfer::restore`, `assemble_with_verified_bases`,
  `Presenter::from_assembled`), are its natural next contents; neither is
  moved by steps 3 to 6.
- With it, the application's own tests after 6.4 are the shell's: about
  45 s added up, about 12 s of wall time.

As landed (`upc-dfs.25.7`): a test-only crate,
[`crates/pc-session-tests`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-session-tests) (layer `pc`, no library code), holds the 57
tests that drive a real PC through the session and test no code of the
window: 35 session tests of `main.rs`, the three sound-level tests, the
six host-audio tests, the eleven mouse shim tests and the two key-table
tests, under the same module paths. They drive `Session<PcMachine>`
directly; `Presenter` stays in the window with its media dialogs and drive
lights, so the crate depends on `machine-view`, the PC core, `assembly`
and the library's `pc-image` and `pc-library`, not on the window. The
window keeps the tests of its own code: the command line, the setup from a
machine file (and the FreeDOS golden through it), the drive lights and
views, the program's notices and the fixture policy, with the fixtures the
shell's tests share. MEASURED (`cargo nextest run`, four tests at once):
the window ran 318 tests in 30.1 s of wall time (92.5 s added up) and now
runs 261 in 9.2 s (36.4 s); the new crate runs 57 in 21.2 s (56.0 s), two
thirds of it the two open games level tests. The runner's plan for
[`crates/window/src/shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) is the window alone; for a file of
`machine-view` it includes `pc-session-tests`.

One deviation from the plan above: the crate does not sit below
`pc-media`. One sound-level test names the template image's workspace
with `pc_library::library_worker::regular_name`, so the crate has
`pc-library` as a test dependency, and with it `pc-media`,
`library-model`, `image-registry` and `media`. A change to a media model
alone therefore still runs these tests, as it ran them in the window
before; nothing regresses. Moving the function that names an image's
workspace down to `pc-image` lets the crate drop that dependency.

#### 8.1.11. Questions answered by the maintainer

Answered on 2026-11-18: Q1 (a), neutral by count; Q2, the names as
proposed. The options are kept for the record.

**Q1. How much of the local library becomes neutral now?** §8 placed the
library model, the worker and the views of machines and installation
media in a neutral `library-ui`. Counted, they are PC (§8.1.2).

- (a) **Neutral by count.** The registry models and views, the listing
  types, the tables and the save form become neutral, behind the five
  hooks of §8.1.4. The worker, the frame and the collections move to `pc`
  crates as they are. The worker gets its seam when a second platform has
  a library (§8.1.4, last part). This is the plan above.
- (b) **The worker's loop neutral now.** As (a), and after 3.11 the loop
  moves to `library-model` with the PC requests and events as associated
  types and the three start flows calling it. About four more beads in
  the worker and its 30 tests; no gain in time; the frame stays PC.
- (c) **A neutral frame now.** As (b), and every collection, pane and
  dialog of the frame behind a platform trait.

Recommendation: **(a)**. It is the direction as stated: platform GUI stays
in per-platform modules and is generalised when a second platform needs
it. (b) and (c) design a library seam against one platform, which is how
the CLI seam was *not* designed: there the neutral half existed first and
the hooks were found by counting. Under (a) every `pc` crate still leaves
for the front-end repository with the rest (§9), so nothing here blocks
the extraction.

**Q2. The names** `library-model`, `pc-media`, `pc-library`, and
`media-ui` for a crate of layer `pc`. They are working names; a rename is
one bead at any time.

Recommendation: keep them. `pc-media-ui` would say the layer, but §8
named the crate, and the layer is in its manifest.

## 9. Towards two repositories

Not part of the first beads; recorded so that no step works against it.

The extraction plan is [repository split](repo-split.md) (Proposed,
2026-11-21): the destination of every crate and file, names and history,
pins, CI and releases, and the ordered steps. The maintainer decided on
2026-11-21 to separate the repositories before contract 1.0, which
supersedes item 5 of the list below.

| Destination | Crates |
|---|---|
| The contract's own source (§11 Q1) | `unemu-core-api` |
| The PC core repository (unemupc) | `device`, `bus`, `cpu`, `video`, `audio`, `devices`, `chipset`, `machine`, `testkit`, `assembly`, `unemu-pc-core`, `pc-config` (added 2026-11-19, stage B plan), `pc-harness`, `machine-file`, `journal` and `journal-files` (§11 Q4, accepted 2026-11-21) |
| The front-end repository | `image-store`, `image-registry`, `media` (all FAT12 formats, readers and builders), the recipe crate, `cli-core`, `cli-pc`, `upcli`, `platforms`, `pc-image`, the window crates |

What must be true first:

1. The boundary check passes with an empty allow list.
2. **Done: stage B landed.** `PcCore::create` reads the launch document
   the PC provider writes from a stored config, and both front ends start
   stored PC images through `core_launch`. The typed PC adapters and live
   host-binding/compatibility helpers are the deviations listed in §4
   and §14.3 S14.
3. `machine-file` no longer depends on `image-store`: its firmware-package
   lookup takes a small trait the provider implements over the store.
   Done: step 7 of §12.10 (`machine_file::FirmwarePackages`, implemented by
   `pc_image::firmware::StorePackages`).
4. **Done (2026-11-21):** `media` stays with the front end (§11 Q4).
   `journal` and `journal-files` own the core's journal and file engine.
   The volume-builder suite `user_media_volumes` lives in `pc-image`, with
   its names, corpus, full SHA-256 checks and byte-equality assertions
   unchanged; `pc-harness` no longer has a media dev-dependency. The
   core-side dependency closure, including dev dependencies, has no media
   or front-end crate and needs no core-side boundary exception.
5. The contract is 1.0 and comes from one source.

§12 adds to this list: the core opens no file (§12.5; the journal engine
now lives in `journal` and `journal-files`), and the crates that go with the core
depend on nothing of the front end (the core-side rule of §12.8).

As of 2026-11-21, the host-interface allow list is empty: the guard scans
zero matched lines. Temporary-directory fixtures live in the host-side
`machine-file` test support, and the CPU constructs its small legacy reset
image directly, without a process-wide cache. This completes the remaining
six exceptions after step 11; the historical counts below describe their
respective landings. MEASURED in release mode: 27 samples of 200,000
resets per version give pooled medians of 546.492 ns/reset with the once
cell and 546.474 ns/reset with direct legacy construction (no measurable
regression). The timing probe is `crates/cpu/tests/reset_timing.rs`; it
checks that resets retire no instructions. A unit test also compares every
legacy field with the pinned CPU's real-entry constructor.

Decision-30 then needs one sentence of care: `unemu.min-app-version` names
the oldest release of the program that provides the manifest's platform.
After the split that program is the **core**: the marker keeps meaning the
unemupc release, `CoreDescription::version`, and the front end has its own
version that images never name.

Two things in the code say otherwise today, and stage B does not fix them:
the level a build compares with is `image-store`'s own package version,
which after the split is the front end's; and the table mixes rows the
core reads (the PC config's keys and vocabulary) with rows the front end
reads (envelope media types, compressed layers, media-set and
firmware-package keys). The maintainer decided on 2026-11-19 to settle
this as an amendment of decision-30 before the repositories separate,
re-stating the marker as naming the core's release (§14.6 Q6; corrected
2026-11-19, stage B plan).

## 10. Risks

| Risk | Answer |
|---|---|
| The contract is frozen too early and is wrong | It stays 0.x until the PC adapter and a second core both run on it. 0.x minors may break, by rule |
| A move changes behaviour unnoticed | Every step moves tests with code, weakens none, and runs the full gate. The encoders' byte-level tests move unchanged. The adapter is compared against direct `AssembledMachine` sessions |
| More crates, longer clean builds and links | Expected to be neutral: the same code in more units compiles more in parallel. Measured per step with the figures of `upc-x83.19`; a step that adds more than a few seconds to the clean build says why |
| The neutral crates quietly regain PC dependencies | The boundary check, with a list of exceptions that can only shrink |
| Two versions of the contract in one build | One source for the crate; the front end's test over every compiled-in core |
| Pulling pictures and sound costs a copy | One frame borrow and one `Vec` append per paint, as today's sinks do; measured in the adapter's step |
| Font assets under CC BY-SA in a core crate | Licence metadata and the `deny.toml` exception move with them (§11 Q5) |

## 11. Questions answered by the maintainer

Answered on 2026-11-17: the recommendation of each question below was
accepted — Q1 (a), Q2 `unemu-core-api`, Q3 (a), Q4 (a) for both, Q5 (a).
The options are kept for the record.

**Q1. Where does the contract crate live, and how do cores and the front
end get the same copy?** Cargo treats one crate from two git revisions as
two crates, so pinning by revision in each repository breaks the trait.

- (a) A member of this workspace now; its own small repository once
  unemunes starts; published to crates.io at 1.0, with SemVer
  requirements.
- (b) Its own repository from the first day, every consumer pinning the
  same tag, with a CI check that they agree.
- (c) Inside the front-end repository for good, cores depending on that
  repository.

Recommendation: **(a)**. It costs nothing now, and SemVer through a
registry is the only arrangement in which Cargo itself guarantees one
copy. (c) makes every core depend on the repository that depends on it.

**Q2. The name.** `unemu-core-api` or `unemu-contract`?

Recommendation: **`unemu-core-api`**. It says whose API it is; "contract"
alone is also what we call the x86jit ABI rules and the IRQ injection
contract.

**Q3. May a provider crate on the front-end side depend on its core's
crates beyond the contract?** `pc-image`, `cli-pc` and the PC dialogs need
`assembly` (firmware catalogue, capabilities, config types) and
`machine-file`.

- (a) Yes, for crates in the `pc` layer only, checked by the boundary
  script; the neutral front end depends only on the contract.
- (b) No: everything a provider needs crosses the contract as data.

Recommendation: **(a)**. It matches "generalise only when a second
platform needs it": (b) means designing settings, firmware catalogues and
machine files as neutral data now, for one platform.

**Q4. Where do `machine-file` and the journal half of `media` live once
there are two repositories?**

- `machine-file`: (a) with the PC core, its store lookup behind a trait;
  (b) with the front end, and the goldens that load machine files with it.
- `media` (journals and overlays): (a) with the PC core for now, the
  front end taking it from the pinned core like any other PC crate, and
  the FAT12 builder and image creation split off to the front end;
  (b) with the front end, storage handed to the core as objects through
  the contract; (c) its own repository.

Recommendation: **(a) for both**, decided finally at extraction. The
goldens need machine files, and the config codec is built on
`machine-file`. Handing storage in as objects is the cleaner end state and
the contract leaves room for it (§3.12), but it re-plumbs `assembly`'s
persistence and belongs with layered disk images, not with this split.

**Q4 follow-up — Recommendation (Accepted, 2026-11-21)**

Accepted by the maintainer on 2026-11-21, together with the decision to
separate the repositories before contract 1.0 (the contract already lives
in its own repository and is pinned by revision).
§12.5 and step 11's final extraction (2026-11-21) have already superseded
its journal timing: storage now crosses the contract as objects. Recommend
keeping the remaining `media` crate **with the front end**, without another
crate or a duplicate FAT12 reader.

| Destination | Crate / contents |
|---|---|
| Core, `emulation` | `journal`: wire format, recovery, `SectorSource`, verified-base/overlay model and record-backed durable sink (`src/sink.rs`). `device::SectorMedia` and assembly's controller/geometry adapters stay here too. None needs FAT12 or a host path. |
| Core, `pc`, `side = "core"` | `journal-files`: file-backed bases and sink constructors, locks, offline tools, raw-image publication ([`create.rs`](../../../crates/journal-files/src/create.rs)) and disk-layer descriptor/sector format (`layer.rs`). These already share the file engine after step 11; keep them together. `machine-file` owns file mounts and host assembly; `unemupc` owns the journal commands. |
| Front end, `neutral` | `media`: FAT12 format/layout and reader, in-memory formatter/builders, names/times, host-file survey and [`fat12/boot.asm`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/media/src/fat12/boot.asm) with its encoded boot sector and source-equivalence test. It depends on no core crate; PC geometry here describes a volume format, not a controller. |
| Front end, `pc` | `pc-image`: disk closure/decompression and zero-base OCI adapters ([`disk.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/disk.rs), [`zero.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/zero.rs)), media creation/recipe orchestration and store publication. They use core-side `journal-files::{create, layer}` rather than copy its codecs or publication rules. |
| Front end, existing layers | `cli-core` keeps FAT12 file export; `cli-pc` keeps PC media commands. `pc-media`, `pc-library`, `pc-ui`, `media-ui` and `window` keep creation models, host-file selection and views. `pc-session-tests` keeps its front-end integration tests; `platforms` and `upcli` compose the providers/commands. |

Before the suite move, `cargo tree -i media --offline` and manifests showed direct
normal dependents `cli-core`, `cli-pc`, `media-ui`, `pc-image`, `pc-library`,
`pc-media`, `pc-ui`, `window`; additional dev edges included
`pc-session-tests` and **`pc-harness`**. All but the latter belong to the
front end; its transitive users include `platforms` and `upcli`.
[`media/src/lib.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/media/src/lib.rs) now exports only `fat12`, with no journal facade.

**Remove the remaining media boundary exception by moving its tests.**
The only `media::` user in `pc-harness` before the move was
[`tests/user_media_volumes.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/user_media_volumes.rs): it builds volumes and reads them back; it
does not run a guest. It uses `plan_volumes`, names/times and geometry as
well as `reader::Volume`, so extracting just the reader would not remove
the dependency. Move the suite whole to [`crates/pc-image/tests/`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/), keeping
every test, corpus, full SHA-256 check, skip/required-fixture rule and
byte-equality assertion. Adjust corpus paths and affected-test data rules
if needed. Remove `media` from `crates/pc-harness/Cargo.toml`'s dev
dependencies and delete `pc-harness → media (dev)` from
`scripts/crate-boundaries.allow`; a core-side Cargo closure including dev
dependencies must then contain no `media` or front-end crate. Keep the
unrelated application-composition exception separate (§7.3).

This keeps sector overlays and persistence below the object seam,
while volume preparation remains a front-end tool. Reject moving all FAT12
code into the core just to serve one builder test; reject a new shared
repository or reader crate until an actual core consumer needs it. Core
goldens remain with `pc-harness`; this suite tests volume preparation.

Layering remains possible: verify every base's full SHA-256, including
large hard disks; keep bases read-only and written sectors separate. Commit
publishes a new base, never rewrites the old inode; sector reads use one
overlay interface replaceable by a chain of hash-linked layers (§12.5 and
deferred.md, “Layered disk images”). No layer feature is added by this move.

**Accepted 2026-11-21:** FAT12 and the whole volume-builder suite belong
with the front end; `journal-files` keeps raw/layer formats and publication
helpers despite front-end callers. The suite now lives in
[`crates/pc-image/tests/user_media_volumes.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/user_media_volumes.rs); its corpus paths remain valid
at the same directory depth. The affected-test data rules select only
front-end recipe readers. The boundary check passes with no core-side
exception; only the application-composition exception remains (§9, §12.8).
MEASURED: the relocated 304-line suite is byte-identical to its previous
source. A manifest audit of all 17 core-side crates' normal, build and dev
workspace dependencies finds only core-side targets, with no `media` or
front-end crate. The two user-ROM tests retain their ignored status and
required-file rule.

**Q5. The PC text rasteriser and its fonts move below the contract, into
the PC adapter.** The contract then carries pixels only, and the CC BY-SA
font assets, their notices and the `deny.toml` exception move from
`window` to `unemu-pc-core`.

- (a) Move them, as designed.
- (b) Keep them in the desktop and let the contract carry a text grid.

Recommendation: **(a)**. A text grid with MDA and VGA attributes is PC
vocabulary in the contract, and every other front end would need the fonts
again.

## 12. Embeddable cores and the core's own command line

Status: **Accepted** on 2026-11-18 (design step `upc-dfs.26`), with the
recommended answer of every question of §12.13. The
principle is decided and recorded in
[decision-41](<../../decisions/decision-41 - Emulator cores embed like libretro cores and each has its own command line.md>):
a core embeds like a libretro core, with as few host dependencies as
possible; each core has its own small command line; the front end's binary
is `unemu` and a core's binary carries the core's name. This section is the
design that follows. The questions of §12.13 are open. Nothing here is
implemented.

Markers in this section: **[REPO]** = read on `main` at `26142ea`, with
the path; **MEASURED** = produced for this section by `cargo tree`, by
`cargo metadata` read with a throwaway script, or by `grep`, as each place
says; **COUNTED** = counted by a throwaway script that reads each source
file up to its first `#[cfg(test)]` and skips comment lines, so the figures
are lower bounds; **[X86JIT]** = read in the x86jit sources at the pinned
revision `cf32ec2e`, not run; **ASSUMED** = not verified.

### 12.1. The rule

"The core" is the contract crate, the crates of the `emulation` layer
(§7.3) and the CPU core they pin. It is a library that touches the host
only through what its embedder hands it. It needs Rust's `std` with an
allocator, and nothing else of the host.

| Subject | A core may not | It gets instead | Checked by |
|---|---|---|---|
| Picture, sound, input devices | depend on a windowing, graphics, host audio or input crate | `frame`, `drain_audio`, `input` (§3.6, §3.7) | the external-crate list (§12.3) |
| Network, async run times | depend on one | nothing: a core has no use for either | the external-crate list |
| Time | read the host clock: nothing of `std::time` on any path | virtual ticks and steps (§3.5); the start of an emulated clock is part of the config | the host-interface check (§12.3) |
| Randomness | use a random source, or let a result depend on the order of a randomly seeded map | seeds in the config, when a machine needs one | the host-interface check for random sources; goldens for the order |
| Threads | start one | the embedder's thread: a machine lives on the thread that created it (§3.5). The one exception is a JIT backend the embedder supplies for `cpu.engine = "jit"` (§12.7): that backend starts its compile thread; the interpreter default starts none | the host-interface check |
| Process-wide state | keep mutable statics or thread-locals that two machines would share | fields of the machine | the host-interface check |
| Files | open a path, scan a directory, read an environment variable | content by digest (§3.4) and storage objects (§12.5) | the host-interface check |
| Standard streams, process exit | print or exit | `drain_messages` and `Error` (§3.10) | the host-interface check |
| Host-specific code | carry `unsafe`, foreign functions or code for one operating system | nothing; the CPU core is the one stated exception (§12.7) | `#![forbid(unsafe_code)]` in every crate of the two layers; the host-interface check for `cfg` on an operating system |

Tests are outside the rule: a core's tests may use temporary files and the
host's environment. Their dependencies are inside it: a test dependency of
a core crate is listed like any other (§12.3).

The crates that go with the PC core to its repository but are not the
library an embedder links (`machine-file`, `pc-harness`, the command line
of §12.8) are host programs and tools. They open files and print. Their
rule is the other one: they depend on nothing of the front end (§12.8).

### 12.2. What the core touches of the host today

COUNTED over the production code of each crate: mentions of `std::fs` and
file opens; of `std::env`; of `std::time`, `Instant`, `SystemTime`; of
`std::thread`; of `println!`, `eprintln!`, `dbg!` and the standard stream
handles; of mutable or lazily set statics and `thread_local!`. `unsafe` is
MEASURED by `grep` over each crate's `src` and `tests`.

| Crate | Layer | Files | Environment | Host clock | Threads | Standard streams | Process-wide state | `unsafe` |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `unemu-core-api` | contract | 3 signatures, no call | 0 | 0 | 0 | 0 | 0 | 0, forbidden |
| `device`, `bus`, `cpu`, `video`, `audio`, `devices`, `chipset`, `machine`, `testkit` | emulation | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `unemu-pc-core` | emulation | 0 | 0 | 0 | 0 | 0 | 0 | 0, forbidden |
| `assembly` | emulation | 25 | 6 | 5 | 0 | 0 | 3 | 0 |
| `media` (reached through the allowed crossing `assembly` → `media`) | neutral | 46 | 0 | 0 | 0 | 0 | 2 (test counters) | 7 (one test's counting allocator) |
| `machine-file` | pc, goes with the core | 24 | 0 | 0 | 0 | 1 | 0 | 7 (one test's counting allocator) |
| `pc-harness` | pc, goes with the core | 0 | 0 | 0 | 0 | 1 | 0 | 0 |

Ten of the eleven emulation crates touch nothing of the host. What
`assembly` and the contract hold [REPO]:

- **The contract** names `std::fs::File` in `Content::File` and
  `std::path::Path` in `CreateRequest::session` and `WriteTarget::Kept`
  [`crates/core-api/src/core.rs`, `media.rs`]. No core implements any of
  the three yet: `PcCore::create` and `PcMachine::add_medium` answer
  `Unsupported` (§8 step 2).
- **Images by path.** `assemble.rs` reads a ROM or a disk image named by
  `ImageSource::File(path)` and opens disk bases itself (seven places).
  The other way already exists beside it:
  `assemble_with_verified_bases` takes bases the provider verified and
  opened.
- **Journals by path.** `persistence.rs` mounts a journal at the path of
  `WriteMode::Journal { path, .. }` through `media::JournalMedia`, and
  validates every output path before it opens one: canonical names,
  aliases of the base, lock sidecars.
- **Firmware discovery.** `firmware_discovery` and its two submodules scan
  ROM folders, read `UNEMUPC_USER_ROMS_DIR`, `HOME`, `XDG_CACHE_HOME` and
  `LOCALAPPDATA`, keep a hash index in the user's cache directory with
  `SystemTime` to detect files changed within the clock's resolution, and
  hold a process-wide `OnceLock<Mutex<HashCache>>` and a thread-local
  session cache. None of it runs while a machine runs; all of it is in the
  emulation layer.
- **Hash maps.** `assembly` and `devices` use `std::collections::HashMap`
  (26 mentions in their sources, MEASURED), whose default hasher takes its
  keys from the host's random source. No golden depends on the order
  today. A core that must run where no random source exists uses an
  ordered map; nothing asks for that yet.

So the host reaches the core in three places: `assembly` opening images
and journals, firmware discovery, and `media`. The steps of §12.10 move
each behind the contract's objects or out of the core.

### 12.3. External crates: the list per layer

MEASURED with `cargo metadata` on the locked graph, normal and build
dependencies, every target, test dependencies left out. The emulation
layer reaches 26 external crates at run time and 8 at build time. The
contract reaches none.

| Crate | Arrives through | Why it is acceptable |
|---|---|---|
| `thiserror` | declared by seven emulation crates | Error enums (conventions.md). Generated code only; no host access |
| `x86jit-core` | declared by `cpu` | The CPU core, pinned to a revision. Its host needs are in §12.7 |
| `iced-x86`, `lazy_static` | `x86jit-core` | The instruction decoder and its immutable tables. Pure Rust |
| `sha2` | declared by `assembly`; `media` | SHA-256 of journal frames and headers and of firmware bytes. Pure Rust with CPU intrinsics |
| `digest`, `block-buffer`, `crypto-common`, `generic-array`, `typenum`, `cfg-if` | `sha2` | Pure Rust; no host access |
| `cpufeatures` | `sha2` | Asks the processor once which SHA instructions it has. No file, no clock, no thread |
| `libc` | `cpufeatures` on AArch64 Linux, Android and Apple targets and on LoongArch64 Linux; `media` on Unix | Declarations of the host's C library. Through `cpufeatures` it is one capability query on targets that have a C library by definition. Through `media` it is file flags and error numbers, and leaves with the journal split (§12.4) |
| `serde`, `serde_core`, `toml`, `toml_datetime`, `toml_parser`, `toml_writer`, `serde_spanned`, `winnow`, `indexmap`, `hashbrown`, `equivalent`, `foldhash`, `memchr` | `media`; since S2 also `pc-config` | The disk-layer descriptor and the FAT12 builder's formats. Host-side code today inside a crate the core links. Since S2 of §14.3 the PC config codec, `pc-config`, declares `serde` and `toml`, so all thirteen stay after the journal split, for the codec (corrected 2026-11-20, S2) |
| Build time: `thiserror-impl`, `serde_derive`, `proc-macro2`, `quote`, `syn` (two versions), `unicode-ident`, `version_check` | derive macros and one build script | They run on the build host and are not linked into the core. `serde_derive` and the second `syn` (version 3, its dependency) stay for `pc-config` after the journal split (corrected 2026-11-20, S2) |

After the journal split the emulation layer's run-time list is 13 crates:
`thiserror`, `x86jit-core`, `iced-x86`, `lazy_static`, `sha2` and its
seven, and `libc` on the four target families above (MEASURED: the same
graph walked without `media`). (Corrected 2026-11-20, S2: since S2 of §14.3
`pc-config` reaches the thirteen crates of the `serde` and `toml` row too, so
after the journal split the list is those 13 and these 13, 26 crates, with
`libc` only through `cpufeatures`; at build time `serde_derive` and `syn` 3
stay.) No emulation crate declares an external
test dependency today (MEASURED).

**The check.** `scripts/crate-boundaries.sh` gains a second input and a
second list:

1. It reads `cargo metadata --format-version 1 --offline --locked` beside
   the `--no-deps` document it reads today: the resolved graph, from the
   lock file, for every target. MEASURED: 0.27 s.
2. `scripts/crate-boundaries.external` lists the external crates a layer
   may reach, one per line: `LAYER: CRATE (run|build|test): REASON`. The
   list has entries for `contract` (none) and `emulation` only; the other
   layers are not limited by it.
3. For every workspace package of a listed layer the script walks its
   normal and build dependencies through the resolved graph, workspace
   crates included, and fails on an external crate that the layer's list
   does not name, printing the path once:
   `assembly → media → toml → winnow`. A crate reached only through a
   procedural macro or a build script is of kind `build`. A crate declared
   as a test dependency of a package of the layer is of kind `test`.
4. A listed crate that the layer no longer reaches fails, as a stale line
   of the allow list does today: the list is exact, and it shrinks when a
   step removes a crate.
5. The contract's rule stays as it is: any dependency at all fails.
6. `--self-test` gets fixtures: a new external crate declared by an
   emulation crate; one that arrives through a listed crate; a stale line;
   an external test dependency; a crate that is `build` in the list and
   reached at run time.

The list names crates, not versions: the lock file and `cargo deny`
already own versions and licences. A new crate is a line someone reviews
against §12.1, with its reason; a host-touching crate has no acceptable
reason.

**The host-interface check.** A crate list cannot see `std`. A second
script, working name `scripts/core-host-guard.sh`, reads the layer of each
package from the same metadata and scans the production sources of the
`contract` and `emulation` crates (a file up to its first `#[cfg(test)]`;
no `tests/` directory) for: `std::fs`, `std::env`, `std::net`,
`std::process`, `std::thread`, `std::time::Instant`, `SystemTime`,
`std::os`, `std::path` in the contract, `println!`, `eprintln!`, `dbg!`,
`io::stdout`, `io::stderr`, `static mut`, `thread_local!`, `OnceLock`,
`LazyLock`, `extern "C"`, `cfg` on `unix`, `windows` or `target_os`,
and in the `emulation` crates `libc::` and `extern crate libc` (the
external-crate list names `libc` for `cpufeatures` and `media`, so a crate of
the layer could declare it; the scan stops it being used).
`scripts/core-host-guard.allow` lists today's uses as
`CRATE/FILE: PATTERN: REASON`; a line whose use is gone fails, so the list
only shrinks. It starts with the lines of §12.2: the contract's three
signatures, and `assembly`'s `assemble.rs`, `persistence.rs`,
`firmware_discovery` and the temporary directory of its test support. It
runs in the `guard` lane beside the publication guard, with a self-test
over planted files, as that guard has.

Considered instead: Clippy's `disallowed_types`, `disallowed_methods` and
`disallowed_macros`, which resolve paths exactly. They are configured by
one `clippy.toml` per crate directory, so eleven copies of one list, and
they also fire on the crates' unit tests. The scan is coarser and in one
place; if it proves too coarse, the Clippy lists replace it.

With the same step every emulation crate gets `#![forbid(unsafe_code)]`,
which `unemu-core-api` and `unemu-pc-core` already carry. No emulation
crate contains `unsafe` today (MEASURED), so the attribute changes nothing
and stops the first one.

### 12.4. The libc audit

`media` is the only crate the core links that uses `libc` directly
(MEASURED: `grep` for `libc::`), in five places of production code [REPO]:

| Where | What | For |
|---|---|---|
| `offline.rs`, `open_input` | `O_NONBLOCK` on open | An offline tool must not block on a FIFO named as a base or a journal; it then refuses what is not a regular file |
| `offline.rs`, `hard_link_unsupported` | `EPERM`, `ENOTSUP`, `EXDEV` | A clearer message when the output file system has no hard links |
| `sink.rs`, `inspection_lock` | `EACCES`, `EPERM`, `EROFS` | Inspecting a journal on read-only storage without creating a lock |
| `temporary.rs` | `O_NOFOLLOW`, `O_NONBLOCK` | Opening a leftover temporary file for cleanup without following a link |
| [`fat12/layout.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/media/src/fat12/layout.rs) | `O_NOFOLLOW`, `O_NONBLOCK` | Reading host files into a new FAT12 disk without following links |

None of the five is emulation. Each belongs to code that handles host
files: locks, temporary files, the offline tools, the FAT12 builder. The
answer is therefore not to replace `libc` (most of the error numbers have
a `std::io::ErrorKind` now; the open flags have no `std` spelling), but
to take the host-file code out of what the core links:

- The journal's **format and logic** (`codec`, `scan`, `history`,
  `overlay`, and the sink's append, recovery and retention) need bytes and
  barriers, not files. They become a core crate over the records of
  §12.5, with `sha2` and `thiserror` as its only external crates.
- The **host-file half** (the file-backed records with their locks,
  temporary names, canonical paths and alias checks; the offline tools;
  `fat12`, `create`, `layer`) stays host-side and keeps `libc`.

`cpufeatures` is the other way `libc` arrives. Options: (a) accept it,
listed with its reason; (b) patch or fork `sha2` to drop run-time
detection, since its manifest declares `cpufeatures` for x86 and AArch64
whatever the features (READ in `sha2` 0.10.9); (c) stop hashing in the
core. (c) is not available: version-1 journals checksum every header and
frame with SHA-256 [REPO: `crates/media/src/codec.rs`]. Recommendation:
**(a)**. The call is one capability query, it exists only on targets with
a C library, and a target without one (a WebAssembly host, say) does not
get the dependency at all, because Cargo applies it by target.

`x86jit-core` declares its own foreign functions instead of depending on
`libc`; §12.7 lists them.

### 12.5. Storage handed in as objects: contract 0.2

Contract 0.1 leaves "storage handed in as objects" out (§3.12). It comes
in now, as minor version 0.2. While the major is 0 a minor may break
(§3.2), and nothing implements the three path forms of 0.1 (§12.2), so
0.2 **replaces** them instead of standing beside them. After 0.2 the
contract names no type of `std::fs` or `std::path`. The same release
carries what a second platform adds (§13.2), and it is made in the
contract's own repository (§13.1).

```rust
/// The contract this source defines.
pub const CONTRACT: ContractVersion = ContractVersion { major: 0, minor: 2 };

/// Verified read-only content a core reads at any offset: a disk base, a
/// cartridge, a large ROM set.
///
/// The object stands for exactly the bytes whose SHA-256 and length the
/// front end verified in full before it handed the object over. What is
/// behind it is the front end's: a file, a buffer in memory, a
/// decompressing reader, a chain of hash-linked disk layers. A core cannot
/// tell, never writes through it and never hashes it again.
pub trait ContentReader {
    /// The length in bytes. Constant for the object's life.
    fn size(&self) -> u64;

    /// Fill `buf` with the bytes at `offset`. Reading past `size()` fails
    /// with `ErrorKind::Usage`. Fails with `ErrorKind::Content` when the
    /// bytes are no longer the verified ones, such as when the file behind
    /// the object changed, and with `ErrorKind::Storage` when the host
    /// read failed. The guest sees neither: the core reports the failure
    /// at the next run boundary.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), Error>;
}

/// Verified read-only content.
#[non_exhaustive]
pub enum Content {
    /// Small content, in memory.
    Bytes(Vec<u8>),
    /// Large content, read at any offset. Replaces `File` of 0.1.
    Reader(Box<dyn ContentReader>),
}

/// Where a machine keeps what must outlive a run: written sectors,
/// non-volatile memory, battery-backed cartridge memory.
///
/// The front end owns the storage and decides what a record is: a file in
/// the machine's session directory, a row of a database, a key of a
/// browser's storage, a buffer in memory. The core owns the format of a
/// record's bytes. A core opens no file, directory or path.
///
/// A machine and its storage live on one thread.
pub trait Storage {
    /// Open the record `name` for this machine's exclusive use, until the
    /// returned object is dropped. A record that does not exist yet opens
    /// empty.
    ///
    /// A name is 1 to 64 bytes of `a-z`, `0-9`, `-` and `.`, starting with
    /// a letter or a digit. The core chooses the names of what its config
    /// describes, and the same config gives the same names in every
    /// release of the core, because a name is how a later run finds what
    /// an earlier one kept. Fails with `ErrorKind::Usage` when the record
    /// is already open, here or in another program, and with
    /// `ErrorKind::Storage` when the host failed.
    fn open(&self, name: &str) -> Result<Box<dyn Record>, Error>;
}

/// One durable sequence of bytes, held exclusively.
///
/// Dropping the object releases the record. It promises nothing about
/// bytes appended since the last `sync`.
pub trait Record {
    /// The length in bytes now.
    fn size(&self) -> Result<u64, Error>;

    /// Fill `buf` with the bytes at `offset`. Reading past the end fails
    /// with `ErrorKind::Usage`.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error>;

    /// Add `bytes` at the end. They are durable only after `sync`.
    fn append(&mut self, bytes: &[u8]) -> Result<(), Error>;

    /// Cut the record to its first `size` bytes. Durable only after `sync`.
    fn truncate(&mut self, size: u64) -> Result<(), Error>;

    /// Make everything appended and cut so far durable. When this returns
    /// `Ok`, the host has confirmed the bytes as far as it confirms
    /// anything. A host that cannot confirm fails; it never pretends.
    fn sync(&mut self) -> Result<(), Error>;

    /// Start new contents for this record beside the old ones. The record
    /// keeps its old bytes, readable and appendable, until the replacement
    /// is committed. One replacement exists at a time.
    fn replace(&mut self) -> Result<Box<dyn Replacement>, Error>;
}

/// New contents of a record, while they are written.
pub trait Replacement {
    /// The length of the new contents now.
    fn size(&self) -> Result<u64, Error>;

    /// Fill `buf` with the new contents at `offset`, to check them before
    /// they are committed.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error>;

    /// Add `bytes` at the end of the new contents.
    fn append(&mut self, bytes: &[u8]) -> Result<(), Error>;

    /// Make the new contents durable and put them in place of the old ones
    /// in one step: after a crash the record holds the old bytes or the
    /// new ones, complete, never a mixture. On `Ok` the record this came
    /// from reads and appends to the new contents.
    ///
    /// A failure before anything was put in place is `ErrorKind::Storage`
    /// and leaves the record as it was. When the host cannot say which
    /// contents the record holds now, the failure is
    /// `ErrorKind::StorageUncertain`, and the record must be opened again
    /// before it is written. Dropping a replacement without `commit`
    /// discards it.
    fn commit(self: Box<Self>) -> Result<(), Error>;
}

/// Everything a core needs to build one machine.
#[non_exhaustive]
pub struct CreateRequest<'a> {
    pub platform: &'a str,
    pub config: ConfigDocument<'a>,
    /// Verified content by digest: firmware, disk bases, media.
    pub content: &'a dyn ContentSource,
    /// Where the machine keeps what must outlive the run. The machine
    /// holds it for its whole life and opens a record when it needs one,
    /// such as when a medium is inserted. Replaces `session` of 0.1.
    pub storage: Rc<dyn Storage>,
    pub options: &'a [(&'a str, &'a str)],
}

/// Where a guest's writes to a new medium go.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteTarget<'a> {
    ReadOnly,
    Discarded,
    /// Keep written sectors in this record of the machine's storage, named
    /// by the front end under the rules of `Storage::open`. The core owns
    /// the record's format; the base is never changed. Replaces the file
    /// path of 0.1.
    Kept { record: &'a str },
}

#[non_exhaustive]
pub enum ErrorKind {
    // … the kinds of 0.1, and:
    /// A host write failed in a way that leaves it unknown whether it took
    /// effect. The object it was made on must be opened again before it is
    /// written.
    StorageUncertain,
}
```

`ContentSource`, `NewMedium` and `Machine` keep their signatures;
`NewMedium::content` carries the new `Content`. The contract crate still
has no dependency, forbids unsafe code, and gains a scripted storage in
memory behind its `test-support` feature, for its own tests and for every
core's.

Why records and not a file system. libretro hands a core a virtual file
system: open, rename, remove, stat and directory listing by path. That
would let today's journal code move unchanged, and it would put paths,
rename semantics, links and locks into the contract for every core and
every embedder. The journal needs four things of its host and no more
[REPO: `crates/media/src/sink.rs`, the private `JournalIo`, `Filesystem`
and `ToolIo` traits]:

| The journal does today | With a record |
|---|---|
| Takes a stable `<journal>.lock` beside the file and holds it for the session | Exclusive use is what `Storage::open` gives; a file-backed storage takes the same lock, `<journal>.lock` (§14.6 Q8; corrected 2026-11-19, stage B plan) |
| Creates a journal by writing a header to a temporary file, synchronizing it and linking it into place, so that a journal with half a header never exists | A record of size 0 is new: `replace`, append the header, `commit` |
| Appends the frames of one guest command, then `File::sync_all` | `append`, then `sync`, at the same points |
| Truncates an invalid tail at mount | `truncate`, then `sync` |
| Retention: writes the retained generations to a temporary file, verifies every recovery point in it, renames it over the journal and synchronizes the directory | `replace`, append, `read_at` to verify, `commit` |
| A failed retention is either "failed before publication" or "completion uncertain; reopen required" | `ErrorKind::Storage` or `ErrorKind::StorageUncertain` from `commit` |
| Compares the base file's size and times before and after each read (`check_unchanged`) | The `ContentReader` over a file does the same and answers `ErrorKind::Content` |
| Validates output paths, canonical names and aliases of the base | The file-backed storage does, where the paths are |

The version-1 journal's bytes, its place on disk and its recovery rules do
not change. A session written before the change opens after it.

**How layered disk images fit** (the requirement of
[deferred.md](../deferred.md), Layered disk images, and of
[machine-images.md](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md) §4):

- **A base is read-only and identified by its SHA-256.** It reaches the
  core as `Content` with the digest and length the front end verified in
  full. The contract offers no way to write a base, and `ContentSource`'s
  rule stands: the whole content is hashed, never a prefix or a sample,
  however large the disk.
- **Written sectors stay separate.** They go to a `Record`. The record
  binds the base by its digest and length in its header, as the version-1
  journal does.
- **Nothing rewrites a base.** Commit, export and sealing a layer are
  front-end operations: they read the record with the core's journal
  crate, write a new base or a new layer as new content, and leave the old
  base as it was. `Replacement` replaces a record, which holds only
  written sectors.
- **One overlay interface.** Inside the core, sector contents are reached
  through `SectorSource` and `SparseOverlay` only [REPO:
  `crates/media/src/overlay.rs`]: the overlay of written sectors over a
  base that is a `ContentReader`. A chain of hash-linked layers stands
  behind the `ContentReader` (the front end builds it, as
  `media::layer::Chain` does today for sealed layers), so the core reads
  one flat base whose identity is the chain head's `result_sha256`. A
  later journal that binds a chain head changes one header check and
  neither interface.
- A zero base is a `ContentReader` that answers zeros and has no bytes. A
  disposable session is `WriteTarget::Discarded`, or a storage in memory.

**How the PC core moves its journals behind it.** Four moves, each a step
of §12.10:

1. **The journal over records, inside `media`.** `JournalMedia` opens over
   a `Record` and a base that is a `SectorSource`; the code that touches
   files today (`HostFilesystem`, `HostIo`, the temporary-file cleanup,
   `platform_sync`) becomes the file-backed `Storage`, `Record` and
   `ContentReader`. The constructors that take a path stay as thin
   wrappers, so `assembly`, the offline tools and every test call what
   they call today.
2. **A mount seam in `assembly`.** `assembly` asks one object for the
   bytes of an `ImageSource::File`, the base of a disk and the record of a
   journal, instead of opening them. The file-backed implementation is
   still in `assembly` in this move, so nothing else changes.
   `MachineConfig` keeps its paths: `ImageSource::File` and
   `WriteMode::Journal` are named about 200 and 105 times in seven crates
   (MEASURED: `grep`), and a path in the PC's host-side description of a
   machine is not a file the core opens.
3. **The crates split.** The journal's format and logic become the crate
   `journal`, layer `emulation`, over the contract's `Record` and
   `ContentReader`. The file-backed objects go to `unemu-core-files`:
   the contract's objects over host files, which the front end and every
   core's command line use, and which an embedder without a file system
   leaves out. That crate exists by then, in the contract's repository,
   because the launch path of the second platform needs it first (§13.1,
   step 6 of §13.6); this move puts the journal's file code onto it and
   extends it where the journal needs more. The offline tools become
   `journal-files` (working name), layer `pc`, on the core's side: they
   need both. `media` keeps `fat12`, `create` and `layer`. The file-backed
   mount of move 2, the validation of output paths and the tests of
   `assembly` that need real files move to `machine-file`. Both lines of
   the allow list (`assembly` → `media`) are deleted.
4. **Firmware discovery leaves `assembly`** for the host side
   (`machine-file`, or a crate of its own if the module names collide).
   The catalogue and the identity checks over bytes stay.

After these moves `PcCore::create` (§4 stage B) has what it needs below
it: content from a `ContentSource`, journals in a `Storage`.

One consequence for the command line: the journal's format is the PC
core's own, so its offline tools (`media generations`, `verify`,
`rollback`, `commit`) are the PC's commands, not neutral ones. Today they
are in `cli-core`, a `neutral` crate, which may not depend on an
`emulation` crate (§7.3). They move to `cli-pc` and to `unemupc` with
their words, output and exit codes; with one platform `upcli media verify`
reads as before. §11 Q4 left the home of the journal half "decided finally
at extraction"; the embeddability rule decides it now (§12.13 Q4).

### 12.6. Threads, time and process-wide state

- **Threads.** A core starts none (a JIT backend its embedder supplies
  starts its own compile thread, §12.7). `Core` is `Send + Sync` and stateless;
  a `Machine` need not be `Send` and lives on the thread that created it
  (§3.3, §3.5). An embedder may run several machines, each on a thread of
  its own. The library worker, the host audio stream and the registry
  transfers are the front end's threads and stay there.
- **Time.** Decision-5 and decision-14 stand: virtual time is retired
  instructions plus the clocks that pass while the CPU is halted, and
  devices tick off it. The host clock is read in one place, the front
  end's pacing (§5). A machine that shows a date takes its start from its
  config, never from the host ([machine-images.md](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md) §2:
  "RTC start stays in the resolved machine config, never the host
  clock"); a front end that wants today's date writes it into the config
  it hands over.
- **Process-wide state.** Two machines in one process share nothing
  mutable. Today's only such state in the core is the firmware hash cache
  and its thread-local session cache in `assembly` (§12.2), which leave
  with firmware discovery, and the statics of `x86jit-core` (§12.7).
  Immutable tables, such as the decoder's, are fine.
- **Errors.** A core reports through `Error` and `drain_messages`. It
  does not print, and it does not end the process; a panic is a bug.

### 12.7. What the PC core needs from x86jit

[X86JIT], for the way `cpu` uses the core [REPO:
`crates/cpu/src/x86jit_backend.rs`]: `Vm::new(VmConfig::flat(bytes))`,
`Vm::sparse` for the 386 and 486 boards, `CpuMode::Real16` or
`CpuModel::I80286`, one `Vcpu` on one thread, and by default no backend
but the interpreter. With `cpu.engine = "jit"` the 386 and 486 boards run a
backend their embedder supplies (`cpu::JitBackendFactory`,
`Vm::with_backend_sparse`), which compiles flat 32-bit Machine code; 16-bit
code and the 80286 stay on the interpreter. The core itself links no
executable-memory code. `unemupc` and `unemupc-view` supply
x86jit-cranelift's `JitBackend` for `--cpu-engine jit` or `engine = "jit"`;
the compile thread belongs to that backend (background tier-up). Without a
supplied backend the engine is an assembly error. The engine is a local run
choice: a stored config never carries it, so contract launches (pc-core,
`assemble_mounted`) always run the interpreter. See
[machine files § CPU engine](machine-files.md#cpu-engine).

| x86jit-core needs | When | On the PC's path |
|---|---|---|
| The allocator, for guest memory (no `mmap`) | `Vm::new` | yes |
| `getpagesize`, cached | first instruction fetch, on Unix; a constant elsewhere | yes |
| A process-wide barrier probe: `membarrier` on Linux and FreeBSD, a Mach port query on macOS | `Vm::new` | yes, once |
| A `membarrier` call and a clock read for its statistics | when a block is lifted over newly tagged code pages, on Linux and FreeBSD | yes on Linux; the clock value feeds a counter, not the guest |
| Reading and writing the floating-point control register, inline assembly | every run entry, on x86-64 and AArch64 | yes |
| A few statics and thread-locals: the page size, a pending-fault record, time statistics | every run | yes; set once or per run, not shared guest state |
| The host clock for time statistics | only after `Vm::set_time_accounting(true)` | no |
| Host-fault recovery: assembly routines, memory-map queries | only after `Memory::set_host_fault_recovery(true)` | no |
| Environment variables and a trace file | only through `Trace::from_environment()`, under the default feature `environment` | no |
| `pthread_atfork`, the process id, a global code map | a JIT backend or a child view | only with `cpu.engine = "jit"`, through the embedder's backend |
| Signal handlers | never installed by the library | no |
| Threads | none outside tests; a JIT backend starts its compile thread | only with `cpu.engine = "jit"`: the embedder's backend starts one; the interpreter default starts none |
| Executable memory (`mmap` with `PROT_EXEC`, `MAP_JIT` and the write-protect switch on macOS, instruction-cache flushes), `libc`, `memmap2`, `region`, Cranelift, a compile thread | the JIT backend crate, `x86jit-cranelift` | no: the core never depends on it; `unemupc` and `unemupc-view` link it (feature `jit` only) and supply its backend for `cpu.engine = "jit"` |

What follows:

1. **Here: `default-features = false` on `x86jit-core`.** The feature
   `environment` gates the environment variables, the trace file and a
   Linux memory-map read; `cpu` calls none of them (MEASURED: `grep` for
   `from_environment`, `set_time_accounting`, `set_host_fault_recovery`,
   `child_view` in `crates/` finds nothing). Step 3 of §12.10.
2. **Asked of x86jit, in its own tracker and under its own rules** (not
   work of this repository): a way to build a machine for one thread that
   makes no process-wide barrier registration, no barrier call and no
   clock read. The barrier orders code publication between threads; a
   machine that runs on one thread has nobody to order against. The mode
   is fixed by a probe today and has no public setter (READ in
   `process_barrier.rs`). And: the table above, kept in x86jit's own
   documentation, so that an embedder knows what each path costs.
3. **The JIT stays optional for good.** It is a run choice that is off by
   default (`cpu.engine`), with the backend, and so the compile thread,
   supplied by the embedder (x86jit-cranelift lets the embedder replace how
   its worker is started), and virtual time unchanged, because it counts
   retired instructions and not blocks. An embedder that cannot map
   executable memory supplies no backend and runs the interpreter with the
   same guest results.

### 12.8. The core's own command line: `unemupc`

A small program that runs and checks PC machines without the front end:
no image store, no registry, no recipes, no window. It is how the core is
tried, debugged and tested on its own, and what stays usable in the core's
repository once the front end has left it.

**The crate.** Package `unemupc` in `crates/unemupc`, layer `pc`, with a
library (parse, usage, the commands) and the binary `unemupc`. It is in
the `pc` layer, not `application`, so that `cli-pc` may call its library
(§12.9). It depends on `pc-harness` (the runners the goldens pin),
`machine-file`, `unemu-pc-core` (the core's description, notices and key
names), the emulation crates the commands name (`assembly`, `machine`,
`device`, `video`), `journal-files` once it exists, and `anyhow`. It does
not depend on `cli-core`: its parser is the one function that moves from
`cli-pc`, and its `main` reads the arguments, runs the command and exits
with its code.

**The core's side, checked.** The crates §9 sends to the PC core's
repository say so in their manifests:

```toml
[package.metadata.unemu]
layer = "pc"
side = "core"     # every `emulation` crate is on the core's side without saying so
```

`machine-file`, `pc-harness`, `journal-files` and `unemupc` carry it. The
boundary check gains one rule: **a crate on the core's side depends only
on the contract, on crates on the core's side and on `unemu-core-files`**
(the contract's objects over files, which travel with the contract: a
second crate of the contract's repository, §13.1). Its
allow list starts with what is true today (MEASURED from the manifests):
`machine-file` → `image-store` (firmware packages from the store; removed
by item 3 of §9, step 7 of §12.10), `machine-file` → `media` (journal
inspection; removed by the journal split) and, as a test dependency,
`pc-harness` → `media` (historically described as a FAT12 reader for
guest-written disks; the actual consumer was the user-file volume-builder
suite, which runs no guest). Accepted on 2026-11-21 (§11 Q4): FAT12 stays
with the front end, and that whole suite moves to `pc-image`. This removes
the final core-side exception, including dev dependencies; the unrelated
`window` → `upcli` application-composition exception remains. Until
step 7 `unemupc` therefore still compiles
`image-store` through `machine-file`, with its Zstandard C library
(MEASURED: the closure of `machine-file` holds `zstd-sys`, `cc` and
`serde_json`); it calls nothing of it. (Corrected 2026-11-20, step 7:
step 7 landed before step 5, so this rule's allow list starts without the
`machine-file` → `image-store` line, and `machine-file`'s closure no longer
holds `image-store`, `zstd-sys`, `cc` or `serde_json`.)

**The commands.** All are today's, moved with their words. The machine
commands come with the crate (step 5 of §12.10), the journal tools with
the journal split (step 11):

```text
unemupc --boot
unemupc --bios PATH [--allow-unlisted-firmware]
unemupc machine check FILE [--allow-unlisted-firmware] [--rom-dir DIR]...
unemupc run --machine FILE [--rom-dir DIR]... [--max-steps N] [--idle-limit N]
            [--print CARD] [--allow-unlisted-firmware]
unemupc media generations --base IMG --journal J
unemupc media verify --base IMG --journal J [--repair-tail --backup PATH]
unemupc media rollback --base IMG --journal J --to N --backup PATH
unemupc media commit (alias compact) --base IMG --journal J --to N --output OUT
unemupc --version | --about
```

| Command | What it runs | Output |
|---|---|---|
| `--boot` | The built-in Real16 timer demo (`pc_harness::run_boot`) | The serial bytes on standard output |
| `--bios PATH` | A BIOS ROM on the IBM 5170 board profile with a blank disk attached; writes are volatile (`pc_harness::run_bios_file`) | COM1 on standard output; warnings and `stopped: REASON` on standard error |
| `machine check FILE` | Loads a machine file, resolves ROMs in the `--rom-dir` folders, validates, inspects journals; runs nothing | Warnings and notes on standard error |
| `run --machine FILE` | A machine file, headless, to a natural stop, to `--max-steps` (the core's steps: instruction equivalents of virtual time) or to `--idle-limit` halted clocks | With `--print CARD`: the card's text grid or the serial bytes on standard output; `stopped: REASON` on standard error |
| `media …` | The offline journal tools (§12.5) | As today |
| `--version`, `--about` | The core's id, release and the contract it implements, from `PcCore`'s description; the licence, the source pointer and the notices of what is compiled into the core | Text on standard output |

Exit codes are today's: 0 for a run that stopped as a run may (halt,
reset requested, budget exhausted), 1 for a failed stop or a refused
check, 2 for a usage error.

**How the output relates to `upcli`'s.** For the four machine commands and
the four journal commands, standard output, standard error and the exit
code of `unemupc WORDS` are byte for byte those of `upcli WORDS` today.
Three differences, all stated:

- `run --machine` has no `--store` and no `--no-store`: `unemupc` has no
  store. `unemupc run --machine FILE` is `upcli run --machine FILE
  --no-store`: firmware comes from the paths in the machine file and from
  `--rom-dir`. A machine file that takes firmware from a package of the
  store is the front end's to run (§12.9).
- The usage text is `unemupc`'s own and names `unemupc`.
- `--version` and `--about` name the core, not `upcli`, and list the
  core's notices only (no registry or store crates).

The code moves; it is not rewritten: `crates/cli-pc/src/run.rs` (296
lines with its tests: `check_machine`, `run_machine`, `run_bios`,
`bios_stop`, `display_dump`), the machine words of `cli_pc::parse` with
their parser tests, and their usage lines [REPO]. The runners stay in
`pc-harness`, so a golden still pins what the binary does.

**Room for more.** The command line is where the core's own diagnostics
go, because they need no store and no window. None of them is designed
here, and each is a new feature that waits for the maintainer under the
feature freeze:

- **Scripted input and limits in virtual time.** `run` gains
  `--keys FILE` and `--max-time DURATION`. The runners already drive a
  guest with scripted keys after a prompt appears
  (`assembly::RunPolicy` with `StopCondition::PromptThen` and
  `KeyboardStimulus`, used by the DOS goldens); a key script is those
  values as text. Its vocabulary (wait for a text on a card, press named
  keys, type characters, run for a virtual time) should be the one the
  Machinefile's input steps use later ([recipes.md](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipes.md) §11), so
  it is fixed together with them. New limits count virtual time, not
  instruction equivalents (decision-22 §5).
- **Raw images.** `run --bios ROM [--floppy IMAGE] [--disk IMAGE]`: the
  board profile of `--bios` with read-only bases and volatile writes,
  for a boot sector or a test floppy without a machine file
  (`pc_harness::run_bios_boot` exists as a library call).
- **The exception trace (`upc-x83.17`) and the debugger and tracing
  (`upc-x83.18`).** Their beads are not quoted here (their text is not in
  the docs; ASSUMED from their titles). They attach to `run` as options
  (`--trace-exceptions`, `--trace WHAT`) and as one more command
  (`unemupc debug --machine FILE`), over the observation and trace policy
  the core already has ([observation.md](observation.md)).

### 12.9. The front end's side: `unemu`

The desktop binary is `unemu` since step 4. Step 13 joins its command line
under the same executable; `upcli` remains the composition library only.
The table records how the former `upcli` executable's commands were split:

| Today in `upcli` | After |
|---|---|
| `--boot`, `--bios PATH`, `machine check FILE` | `unemupc` only (§12.13 Q2). They need nothing of the store |
| `run --machine FILE [--store DIR \| --no-store] …` | Stays a PC command of the front end, because it adds the store: it takes firmware packages for the machine file from the store, then calls the run of `unemupc`'s library. One implementation, so the two cannot drift |
| `media generations`, `verify`, `rollback`, `commit` | PC commands in `cli-pc` that call `journal-files`; the same words and output; also in `unemupc` |
| `image run NAME:TAG`, `image machine run ID` | PC provider commands until `PcCore::create` reads stored configs (§4 stage B). Then they move to `cli-core` and drive the machine through the contract, for any platform: `core.create(&request)` with a `ContentSource` over the store's receipts and a file-backed `Storage` over the workspace, `run(RunLimit::steps(max))`, `finish` (§5). Corrected 2026-11-19 (stage B plan, §14): for the PC the `Storage` is not a plain `FileStorage` over the workspace but the provider's own storage plan, a table from record name to journal file, because the workspace's journal names (`media/<64 hex>-<n>.upfj`, files under `media/created/`, journals beside a host image) cannot be record names of at most 64 bytes with no directory; the journal keeps its lock `FILE.lock`, which `unemu-core-files` learns (§14.6 Q8). Nor is it a pure move: through `Machine::run` the result line ends in the contract's stop, a guest reset no longer ends the run and there is no idle limit, stated in `commands.md` (§14.6 Q5) |
| Every other `image`, `media create` and `recipe` command | Unchanged |
| `--version`, `--about` | The front end's: its own release, and each compiled-in core's id, release, contract version and notices from `CoreDescription` |

The command line is part of the `unemu` binary (§12.13 Q1(a)). The
libraries stay split as §7.1 has them; `window` is the application composition
point and its executable tests run both entry points. A CLI change now also
rebuilds and tests that application; toolkit-independent library tests stay
in `cli-core`, `cli-pc` and `upcli`.

What the rename covers, and what it does not:

- **Renamed to `unemu`:** the binary of the `window` package (the package
  may take the name `unemu` with move 6.5 of §8.1.9, which said
  `unemupc`), the window title, the application menu ("About", "Hide",
  "Quit"), the Linux window class and Wayland application id, the usage
  and `--version` text, the lines the desktop prints with its own name,
  the release workflow and what it packs, `scripts/third-party-licenses.sh`,
  the binary ids in the runner's self-test, the window family of
  `scripts/crate-boundaries.sh` (where `unemupc` now means the core's
  command line), README.md, `commands.md`, `machines/README.md` and
  `architecture.md`. There is no macOS application bundle in the
  repository today (MEASURED: `grep` finds no `Info.plist` and no bundle
  step); when one is added it is named `unemu`.
- **Not renamed:** the repository and the release archive
  (`unemupc-<version>-<target>.tar.gz`) while both programs ship from it;
  "Needs unemupc X or newer" and `image_store::compat::PROGRAM`, which
  name the core's release (§9, decision-30); the environment variables
  `UNEMUPC_*`; the core's host directories (the firmware index stays
  `unemupc`). The front end's directories follow `unemu` in step 14
  (§12.13 Q5); step 4 deliberately left them alone.

### 12.10. Steps

Each step is one bead under the rules of §7.5: it lands alone on `main`
with the full gate, changes no behaviour unless the row says so, and moves
tests with the code they test. Sizes are ASSUMED.

| # | Step | Behaviour | What proves it |
|---|---|---|---|
| 1 | **Done.** **The external-crate list.** `scripts/crate-boundaries.external` with today's 26 run-time and 8 build-time crates of the emulation layer and their reasons (§12.3); the script reads the resolved graph; fixtures. As landed: the list has 27 run-time and 7 build-time crates and no test-only one. The 27 are the 26 of §12.3 and `unemu-core-api`, which became a crate from outside the workspace with §13.6 step 3 and is reached by `unemu-pc-core`; the 7 are the 8 of §12.3 counted by name (`syn` comes in two versions and has one line). The contract layer has no line and no workspace package. The walk reads the resolved metadata the one-copy rule already reads, now with `--all-features` (MEASURED: the same result as without today), so a non-default optional dependency is seen too. That graph is cargo metadata's resolve, not one package's build: `indexmap`, `hashbrown`, `equivalent`, `foldhash` and `memchr` are in it only through weak optional features (toml's `indexmap?/std`, winnow's `memchr?/std`) and are not compiled for the core, and their lines say so; a crate's kind is the lowest of its paths (run, then build, then test), a procedural macro and what it uses are `build`, and only a package of the layer contributes test dependencies. Besides the five planted cases of §12.3 the self-test has a crate listed as `run` and reached only at build time, a duplicate line, a line for a layer the list does not limit, a malformed line, a contract package reaching an external crate, and a passing graph (`scripts/crate-boundaries-fixtures/external/`). MEASURED: each fixture fails under a mutation of the rule it covers (no stale check, proc macros as run, dev edges of any crate followed, one kind per edge, the test queue drained first, kinds not compared, no transitive walk); a `log` dependency planted in `device` fails with `forbidden: device (emulation) → log (external), run dependency, through device → log`. A planted `libc` dependency in `device` passes by design: `libc` is on the list (through `cpufeatures` and `media`) and the list names crates, not paths; step 2's scan stops its use. The check grows by 0.06 s and the self-test by 0.3 s | None; no code moves | The check passes on `main`; the self-test's five planted cases fail as expected; a planted dependency on a crate the list does not name (`log`) in `device` fails with its path; the check grows by 0.06 s (MEASURED) |
| 2 | **Done.** **The host-interface check and `forbid(unsafe_code)`.** `scripts/core-host-guard.sh` with its allow list of today's uses (§12.2) and its self-test, in the `guard` lane; the scan also refuses `libc::` and `extern crate libc` in the emulation crates, so a `libc` dependency the external-crate list lets through cannot be used; the attribute in the nine emulation crates that lack it. As landed: `scripts/core-host-guard.sh` reads the same package layer metadata as the boundary check and scans all twelve workspace emulation crates (and any workspace contract crate; there is none today). `scripts/core-host-guard.allow` has 41 exact file-and-source-line entries covering 42 matched lines, with reasons; a missing, stale, malformed or duplicate entry fails. Besides host modules and file opens, the spellings include wall clocks, streams, process-wide caches, OS cfgs, foreign functions, random sources and the existing `HashMap` uses; `libc::`, `extern crate libc` and `unsafe` are refused too. Separate `tests/`, `tests.rs` and `*_tests.rs` sources are excluded, and each other file stops at its first `#[cfg(test)]`, as §12.3 specifies: this deliberately coarse scan does not resolve aliases or inspect production code later in that file. Test support compiled behind a feature still needs its listed exceptions. The current exceptions include `mount.rs` (the file-backed mount introduced by step 10), and two lines of `cpu`'s immutable, once-built reset image, absent from the old §12.2 inventory. There are ten missing attributes, rather than nine: `assembly`, `audio`, `bus`, `chipset`, `cpu`, `device`, `devices`, `machine`, `testkit`, `video`; `unemu-pc-core` and the later `pc-config` already forbid unsafe. All ten now carry the attribute; none needs unsafe. The bead and CI guard lanes run the self-test and the tree scan. MEASURED: the tree passes; the self-test rejects Instant in `devices`, both libc spellings in `device`, unsafe, and the other host-interface families, a removed allow entry, a removed use and a use moved to another file, while accepting the listed use and excluding tests and host-layer sources. A planted unsafe block in `device` compiles without the attribute and is rejected by the compiler with it | None | The guard passes with exactly the listed lines; a planted `std::time::Instant` in `devices`, a planted `libc::` use in `device` and a planted `unsafe` block each fail; deleting a line whose use still exists fails |
| 3 | **Done.** **`x86jit-core` without default features.** As landed: the workspace dependency sets `default-features = false` with no features enabled. At the unchanged revision `6cb817b86c1fbf463f7aab8c0da1ce0eec3db0b7`, the only features are the default `environment` (legacy diagnostics and host-permission file queries) and `test-hooks` (barrier injection); the PC uses neither. No CPU source or lock-file change is needed. MEASURED: the whole workspace builds with `cargo build --offline --all-targets`; `cargo tree -e features` for `cpu`, `upcli` and `window` loses `x86jit-core/default` and `environment`, with no x86jit feature left enabled. All 44 selected ignored non-user-ROM goldens pass. The BIOS boot, floppy boot, FreeDOS boot, interactive DOS and DOS write goldens retain their exact retired counts: 19 015 022, 18 990 963, 8 800 473, 11 772 926 and 11 766 596 respectively | None: only API the PC never calls goes away | Build; the goldens tier, because the CPU dependency's configuration changed; `cargo tree -e features` shows no `environment` |
| 4 | **Done.** **The desktop binary becomes `unemu`** (§12.9). As landed: the package stays `window`; its binary is `unemu`. The desktop title, About/Hide/Quit menus, native application name (Wayland app id; X11 defaults to the binary name), usage, version/about first line, own-name messages and accessibility guidance use `unemu`. The release build, binary checks, package contents and download notes, licence generator, affected-test runner self-test, boundary-check window family and the four user guides follow it. The PC core line remains `core unemupc VERSION (unemupc), contract 0.2`; repository and archive names, logo assets/notices, compatibility text, environment variables, host directories and file formats stay unchanged (§12.13 Q5). The changed name pins are `informational_options_carry_the_embedded_font_notice`, `version_and_about_are_pinned`, `help_is_pinned`, `application_menu_lists_the_standard_items_in_order` and `title_about_and_menu_name_the_desktop_unemu`; `desktop_binary_and_host_access_hint_name_unemu` additionally pins the Cargo binary name and the extracted accessibility hint; other desktop dialog, title and stop-question assertions follow the same rename. The same own-name text lives in extracted UI/library modules too: their accessibility and registry/media messages and worker log prefixes follow the rename. MEASURED validation and the maintainer-only release check are recorded below | **Stated:** the program's name everywhere §12.9 lists. No directory, variable or file format changes | The tests that pin the name (`--version`, usage, the menu titles) change with it and are listed in the bead; the runner's self-test; the release workflow run by hand on a test tag; `git grep` for the old binary name leaves only the lines §12.9 keeps |
| 5 | **Done.** **The `unemupc` crate and binary.** [`run.rs`](../../../crates/unemupc/src/run.rs), the machine words and their usage lines move from `cli-pc` to `crates/unemupc`; `cli-pc` calls the library for the four commands, so `upcli` does not change; the binary `unemupc` with the four machine commands, `--version` and `--about`; the core-side rule in the boundary check with its three allowed crossings (§12.8). Needs step 4: two packages cannot build a binary of one name | `upcli`: none. New: the binary `unemupc` | `upcli`'s `command_line_pin` (usage and the parse of 215 lines) passes unchanged; for the four commands, standard output, standard error and exit code of `unemupc` equal `upcli`'s over the invocations of §7.5 step 4 that use them (`run` with `--no-store`); the moved tests keep their names; a planted `unemupc` → `image-registry` dependency fails the check |
| 6 | **Done.** **The raw commands leave the front end** (§12.13 Q2). `--boot`, `--bios` and `machine check` go from `cli-pc`; their tests (`machine_check`, the `--bios` cases of `firmware_catalogue`, the `--boot` cases of `timing_adoption` and `media_tools`, and the raw-command cases of `machine_commands`) move to the `unemupc` package and run that binary; a front-end golden that mixes them with image commands (`at_games`) calls the library; `run --machine` stays and calls the library. As landed: see the step 6 note below | **Stated:** `upcli` no longer has three commands; its usage text and pin change | Every moved test passes against `unemupc` with its assertions unchanged; `command_line_pin` changes only by the three removed forms |
| 7 | **Done.** **`machine-file` off `image-store`** (item 3 of §9). The firmware-package lookup becomes a small trait in `machine-file` that `pc-image` implements over the store. As landed: the trait is `machine_file::FirmwarePackages`, one method `resolve(document, warnings, images)` that points each pinned ROM or chip a package holds at its bytes (a host path, or a never-created key with the decoded bytes in `images`) and returns the ids of the packages used, in first-use order; its error is a boxed error (`machine_file::PackageError`) reported against the machine file as before. `machine_file::prepare_for_assembly_with_firmware_packages(path, roots, warnings, Option<&dyn FirmwarePackages>)` replaces the store-typed preparation, and `PreparedMachineFile::firmware_sources` gives the ids as text; the store spells a package's id as its manifest digest, `sha256:<hex>`. What touched the store moved unchanged from `machine-file/src/firmware_package.rs` to [`pc-image/src/firmware_package.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/firmware_package.rs): the PC package rules (`PLATFORM`, `BOARDS`, `Fit`, `PcFirmwareRules`, `reconstructed`, `lane`, `role`), the lookup over the summary cache and `rom_path`, and the two preparations that take a store or a store root, now `pc_image::firmware::prepare_for_assembly_with_firmware_store` and `prepare_for_assembly_with_store_root` with their old signatures, over `pc_image::firmware::StorePackages(&Store)`; `pc_image::firmware::firmware_sources(&prepared)` parses the ids back into digests for import. Callers changed only their path: `cli-pc`'s `run`, `pc-library`'s machine-file list, `pc-image`'s import; the tests changed only the path of the moved functions and, where they compare provenance with a digest, read it through `firmware_sources` ([`pc-image/tests/firmware.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/firmware.rs), one test of `window`'s shell). Two parts of the proof did not hold as written, because step 5 has not landed (corrected 2026-11-20, step 7): there is no allow-list line to delete, since the core-side rule that would refuse `machine-file` → `image-store` comes with step 5 and the layer rules allow a `pc` crate to use a `neutral` one, so that rule's list now starts without it; and there is no `unemupc` package yet (the binary of that name is the desktop's, package `window`, which depends on `image-store` itself until step 4), so the closure measured is `machine-file`'s, which is what that package will compile of these crates. MEASURED with `cargo tree -e normal`: `machine-file`'s closure goes from 62 packages to 54, losing `image-store`, `zstd`, `zstd-safe`, `zstd-sys`, `serde_json`, `itoa`, `memchr` and `zmij`; with build dependencies from 70 to 57, losing also `cc`, `find-msvc-tools`, `jobserver`, `pkg-config` and `shlex`; `pc-harness`'s stays at 41 (it reaches `machine-file` only in its tests). `upcli`'s `machine_commands` (11 tests), `cli-pc`'s `firmware_host_guard` (which runs `run --machine` with a supplied store, with `--store` and with `--no-store`) and the 21 tests of [`pc-image/tests/firmware.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/firmware.rs) pass with their assertions unchanged; `upcli run --machine` built before and after the step gives the same output and exit code with a package found in the store, with `--no-store`, with a store root that does not exist, with one that is a file (the warning) and with a corrupted package blob (the digest refusal). `scripts/crate-boundaries.sh` passes | None | The front end's `run --machine` with `--store` reads the same packages (its tests in `machine_commands` unchanged); the crossing `machine-file` → `image-store` leaves the allow list; the closure of `unemupc` loses `image-store` and `zstd-sys` (MEASURED by the bead) |
| 8 | **Contract 0.2.** The types of §12.5 in `unemu-core-api`; `Content::File`, `CreateRequest::session` and the path of `WriteTarget::Kept` go; `CONTRACT` is 0.2 and `PcCore` states 0.2; a storage in memory behind `test-support`. Since §13 this step is made in the contract's repository together with the additions of §13.2 (steps 4 and 5 of §13.6), and this repository moves its pin in step 7 there | None: `create` and `add_medium` still answer `Unsupported` | The contract's tests: a record's append, sync, truncate; a replacement that is committed, dropped, and failed in both ways; `drives` refuses a 0.1 core. The contract's lines leave the host guard's list, and the guard then refuses `std::fs` and `std::path` in the contract |
| 9 | **Done.** **The journal over records, inside `media`** (move 1 of §12.5). As landed: `JournalMedia` keeps its journal in one record and opens no file: it scans by `size` and `read_at`, cuts an invalid tail with `truncate`, confirms with `sync`, creates a journal by committing a replacement that holds only the header, and rewrites retained history through a replacement it reads back before `commit`. The record is the contract's `Record` plus one seam of the journal's file record (`media::store::JournalRecord`, crate-private): a retention replacement whose host calls go through the offline tools' `ToolIo`, so their failures stay injectable; any other record ignores it. After a retention commit fails with `StorageUncertain`, the journal reads the record back (reads still work then) and takes the new contents as in place when the record has their length and their header, whose anchor the old contents cannot have; it then reports the new numbering, as it did before for a file whose rename took place and whose directory sync failed. New API: `JournalMedia::open_record(Box<dyn Record>, &VerifiedBase, length, SinkStatus)` and `media::JournalFile`, a journal file as a contract `Record` (`JournalFile::open(path, base identity, base metadata)`; `Usage` when the lock is held, `Storage` otherwise, the message naming the step; an existing file with no bytes is refused, as the path constructors refuse it, because the publication protocol never leaves one; its creation commit publishes only the header of the base it was opened for, and refuses another with `Usage`), which is what §14.4 item 1 asks for and what the storage plan of §14.2.4 opens per record; it takes the base's identity because a creation leftover is removed only when it holds a prefix of that journal's header, and the plan has the identity from its content table. The journal's file code (`HostFilesystem`, the `FILE.lock` sidecar, header publication through `FILE.create-N.tmp` and a hard link that never replaces a file, retention through `FILE.retain-N.tmp` with the journal's exact mode and a rename, the leftover cleanup, `platform_sync`) moved unchanged into `media::journal_file` as that record; the path constructors (`open`, `open_verified`, `open_with_base_metadata`, …) are thin wrappers over it, and no caller in `assembly`, `pc-image`, `cli-core` or the offline tools changed. How journal files map to records: one record is one `.upfj` file, named by the storage's table and not by its path (§12.9, corrected 2026-11-19); its lock `FILE.lock` (§14.6 Q8) and its temporaries are the record's own and are never records. The file record keeps the host calls in their order: its `size` seeks to the end, which also places the next append there, and a read seeks to its offset, so the model traces of `sink_tests.rs` (lock, open, header creation, path check, cleanup, directory sync, seeks, reads, writes, barriers, cut) and every failure's operation name ("lock", "publish header", "directory sync", "append seek", "command/end sync", …) are the same; `sink_tests.rs` and `offline_tests.rs` did not change. One difference exists only through a record, which no caller uses yet, and S10 keeps it: the journal names a failure of the header commit "publish header" whatever the record, so a file record's creation step ("header write", "header sync", "unlink temporary header", "verify journal path", "remove creation temporaries", "directory sync") is not the operation but the start of the message, as `header sync: …`; the path constructors still name the step itself. A failed append to a retention replacement is `Storage` and leaves the record writable, because the replacement's bytes are its own temporary file; when the partial write cannot be cut back, the replacement refuses everything but being dropped. `media` gains the contract as a dependency (layer `neutral` → `contract`, already on the external list); `unemu-core-api` is unchanged. MEASURED: `media`'s 139 unit tests pass unchanged; `media/tests/journal_storage.rs` adds 7: one scripted session (creation, duplicate sectors, checkpoints with retention to 3 and to 2, an interrupted session sealed at the next mount, a torn tail cut at mount) through the path constructors, through `JournalFile` from a name-to-file storage and through the contract's `MemoryStorage` leaves equal bytes after each of its four sessions, pinned to the SHA-256 the parent commit wrote for the same session (so a journal written before the step is the journal written after it, and each stage is mounted by the next); a power cut in memory keeps the synchronized command only; a failed header commit leaves no journal; a session over `JournalFile` and `JournalTool` (inspect and mutate) exclude each other through `disk.upfj.lock`, and both file tests fail when `JournalFile` locks another name; an uncertain retention commit over `MemoryStorage`, once before and once after its contents are in place, reports the numbering of the contents the record holds (fails when the read-back is skipped); a 0-byte journal file is refused by `JournalFile::open` and by the path constructors (fails without the check); a journal file opened for one base refuses to publish another base's header (fails without the check). `cargo test --workspace --release` passes (4 311 tests) and so do the 63 ignored goldens, `ide_journal_golden`, `dos_write_golden` and `media_files_golden` among them. | None | `media`'s own tests pass unchanged, the ones that record the order of writes, barriers and renames among them; the journal goldens (`ide_journal_golden`, `dos_write_golden`, `media_files_golden`) pass; a new test runs one session over the file-backed storage and over the storage in memory and compares the records byte for byte; a journal written before the step mounts after it |
| 10 | **Done.** **The mount seam in `assembly`** (move 2). As landed: `assembly::Mount` (`crates/assembly/src/mount.rs`) is the one object the assembler asks for what a `MachineConfig` names by path, and the path is its key: `image(path)` gives `ImageBytes` (a ROM, a ROM part, a disk image mounted read-only or volatile, the CMOS image of an NVRAM journal); `disk_base(path, check_length)` gives the `DiskBase` of a journalled disk, calling `check_length` before it reads a byte, so a file of the wrong size is still refused before it is hashed; `journal(path)` gives a `JournalStore`, asked when the journal opens; `check_outputs(media)` checks the journal outputs before anything opens. Failures are `MountError::Config` (passed on unchanged) or `MountError::Host(unemu_core_api::Error)`, reported as the path's read or journal failure. The seam names only the contract's `ContentReader` and `Record` and types `assembly` owns, so `unemu-pc-core` can implement it without naming `media` (§14.4 item 2): `DiskBase::from_reader(Box<dyn ContentReader>, sha256, size)` reads the base on demand and never hashes it again, `JournalStore::record(Box<dyn Record>)` keeps the journal in a record through `JournalMedia::open_record`. `FileMount` is the file-backed implementation, still in `assembly`: each path is the file it names, a journal file opens through the journal's path constructors with its `FILE.lock` and the base file's identity, as before, and `FileMount::with_verified_bases` takes the table of `assemble_with_verified_bases`. `assemble_mounted(config, policy, options, Rc<dyn Mount>)` is new; every other `assemble*` function is the same assembly over a `FileMount`, as are a floppy appended by path, which still opens its base itself, and the byte loads of an already resolved config, and `MachineConfig::validate` keeps its host checks, which are `FileMount::check_outputs`. The assembler validates through the mount's `check_outputs`, and two rules of that step that are not about the host now run for every mount: a positive NVRAM retention before it, and distinct journal keys among fixed drives, catalogue entries and NVRAM journals after it (after, so that a file mount, which refuses equal outputs as aliases, reports exactly what it did before; a mount of records would otherwise let two media share one record). The mount's digest of a base is trusted, never recomputed. **Lifetime** (§14.4 item 3): the machine keeps its mount (`AssembledMachine::mount`, an opaque `MachineMount`, host files by default), and each catalogue medium's handle keeps it too and asks it for the journal at every insert. **A medium appended by path** (`append_floppy`, `append_floppy_verified`, signatures unchanged) is a host file whatever the mount: its journal opens through a `FileMount`. On a machine over files the whole config is validated as before; on another mount the mount checks the candidate's outputs and the media appended by path are checked against each other with the file rules (aliases, reserved names), because the mount's keys are not host paths. A failed `Mount::journal` is the journal's mount failure, step "open journal record". A ROM is now read like a disk image (open, metadata, read), not with `std::fs::read`: the same bytes and messages. Callers: none of the production code changed. `AssembledMachine` gained the public field `mount`, so the three struct literals in tests (`assembly`'s `observation_audio_tests.rs`, `pc-harness`'s `inline_host_can_select_capture_output` and its scheduler support) add `mount: Default::default()`. `PersistenceHandle::mount`, `prepare_removable` and `verify_drive_base` stay as test-only wrappers over the mounted forms, so the persistence and image-source tests, and the test that a wrong-sized base is not hashed, did not change. MEASURED: `crates/assembly/tests/mount_seam.rs` adds 6 tests over a mount in memory (the contract's `MemoryStorage` and `MemoryContent`, keys under an absolute folder of the temporary directory that is never created): a machine with a µPD765 floppy journal, an ATA fixed-disk journal and a CMOS journal, its program and CMOS image read through `image`, runs two sessions (a guest floppy write and read-back, a host ATA write, a CMOS byte) beside the same machine over files, with equal guest results, and after each session the three records hold exactly the bytes of the three journal files, durable and released; a catalogue's second medium opens its record only at insert, the ejected one's record is sealed and released, the mount lives exactly as long as the machine (a weak reference dies with it), also for a machine with no journal at all; a machine over records takes a floppy appended by path, its journal is a host file with its own lock and the guest's write lands there, and an appended journal that aliases its own base, or another appended journal by another spelling, is refused by the host check (fails when it is skipped), and the same spelling by the key check; two catalogue media with one journal key, and a CMOS journal with the fixed disk's key, are refused before any record opens; a record held elsewhere fails the assembly with "open journal record" and leaves no record open. `cargo test --workspace --release` passes (4 328 tests, 63 ignored), and the bead gate's goldens tier runs 4 368 tests with the ignored goldens (the 20 user-ROM tests skipped), `ide_journal_golden`, `dos_write_golden` and `media_files_golden` among them, all passing | None | `assembly`'s persistence and image-source tests pass unchanged; the slow goldens; the new seam has a test with a mount that serves from memory |
| 11 | **Done (2026-11-21).** As landed: see the final extraction note and the earlier compatibility notes below. **The crates split** (move 3), as three beads: `journal`; the journal's file code onto `unemu-core-files`, which exists since step 6 of §13.6; `journal-files` with the journal commands moving from `cli-core` to `cli-pc` and `unemupc` | None in `upcli`'s output: the journal commands change crate, and with one platform their words, output and exit codes are the same. New: the four journal commands of `unemupc` | Test names unchanged apart from the crate (§8.1.5 rule 3); `media_tools` and `media_fifo` pass in their new package; both `assembly` → `media` lines leave the allow list; `libc` keeps one way in, through `cpufeatures`, and the reasons of the `serde` and `toml` crates no longer name `media` (corrected 2026-11-20, S2: `pc-config` keeps the thirteen crates, `serde_derive` and `syn` 3, so the list does not lose them); `persistence.rs` and `assemble.rs` leave the host guard's list |
| 12 | **Done.** **Firmware discovery leaves `assembly`** (move 4). As landed: see the note below | None | The discovery tests move with the module and pass; the user-ROM helper of the tests keeps working from its new place; `assembly` has no line left in the host guard's list, and the list is empty |
| 13 | **Done.** **The command line joins `unemu`** (Q1(a)): command words dispatch through the retained `upcli` composition library; the `upcli` executable is removed. Releases pack `unemu` and `unemupc`; `setup-unemu` installs the combined frontend. As landed: see the step 13 note below | **Stated:** the program name in usage and in the release; no arguments or desktop options still select the window | `command_line_pin` changes only in the program name; binary dispatch and existing moved tests run `unemu`; the action workflow installs and runs it |

| 14 | **Done.** **Front-end host directories follow `unemu`**, with a one-time read of the old place (§12.13 Q5). As landed: only the small, self-contained preferences document is copied once to a missing `unemu/preferences.toml`, from `unemupc` or the original project directory. Machines (including journals), diagnostics/trace logs, saves and states each keep using their existing `<data>/unemupc/NAME` tree, including for new writes, until the user moves it; only an absent old tree selects `<data>/unemu/NAME`. The old tree takes precedence even if a new tree exists, so empty or abandoned destinations cannot hide user data. No tree is traversed, copied, merged, rewritten or deleted during location selection. This interprets “a one-time read of the old place” as discovery and continued use, rather than duplicating path-bearing files: absolute journals must have one writable identity, symlinked machine files must retain target-relative paths, private permissions must remain private, and multi-GB journals must not be copied at startup. Preferences alone use a lock, temporary file, sync and rename; an interrupted copy can be retried. The image store, caches, workspaces, explicit roots, source-tree paths, `UNEMUPC_*` and PC firmware index stay unchanged. `commands.md` inventories the locations | **Stated:** new front-end defaults use `unemu`; existing trees stay in use in place until the user moves them, with no automatic tree migration | Temporary HOME/XDG probes cover every directory, repeat and concurrent starts, old-tree precedence and absence, external/dangling/cyclic symlinks, private permissions and abandoned staging. Machine-loader tests cover target-relative symlinks and sector writes to one absolute journal; preferences test an error after a partial write and retry. Store/cache/workspace and core firmware-cache pins remain unchanged |

Not steps of this plan, and where each is:

- **`PcMachine::add_medium` and catalogues through the contract**: the
  media dialogs' move behind `add_medium` (§8 step 2, with `upc-dfs.25.7`
  and step 5 of §8). It uses the `NewMedium` of 0.2 and needs step 9.
- **`PcCore::create`** (§4 stage B) and, after it, `image run` through the
  contract (§12.9). It needs steps 9 and 10 and the config codec below the
  contract, and not step 12: `create` gets firmware as content, and eight
  of stage B's fourteen steps need no step here at all (§14.5; corrected
  2026-11-19, stage B plan: it said "steps 8 to 12"). One more thing to plan with it: the part of `machine-file` the
  codec needs (the schema and its lowering, TOML only) becomes part of
  what an embedder links, so it must pass the list of §12.3; `serde` and
  `toml` are pure Rust and pass, the YAML reader and the path handling
  stay on the host side.
- **The requests to x86jit** (§12.7), in x86jit's tracker.
- **New options of `unemupc`** (§12.8, Room for more): features, after the
  freeze.

Step 4 measured validation (2026-11-20): the five updated primary name pins
fail against the original desktop implementation; the added executable/access-hint
pin fails with the old extracted hint. All six pass after the rename. The
runner self-test passes. The affected gate passes formatting, guards, clippy,
doctests, 28 selected ignored tests, the platform-id scan and rustdoc; its fast
suite has 1781 passes and 79 failures, all denied loopback binds in the sandbox.
Cargo-deny is reserved for the supervisor outside the sandbox. Actionlint passes;
the exact packing shell runs for both target/OS branches with stand-in binaries
and the real licence generator, retaining the archive names and packing `unemu`
and `upcli`, with no old desktop executable. No GUI is opened.

Step 4 release validation: the workflow is not dispatched by the worker. The
supervisor asks the maintainer to publish a test release/tag matching the
workspace version, then run `gh workflow run release.yml --repo
unemu-org/unemupc -f tag=v<test-version>` and inspect both archives for `unemu`
and `upcli`. This manual check remains pending; local packaging and YAML checks
do not establish that the hosted release workflow passes.

**Step 5, as landed (2026-11-20).** `crates/unemupc` is a `pc`,
`side = "core"` package with `src/lib.rs` (machine command data, parser,
usage, dispatch and the PC core's own notice), `src/run.rs` (the moved
runner and its tests), [`src/main.rs`](../../../crates/unemupc/src/main.rs) (arguments, streams and exit code)
and `tests/command_line.rs` (the standalone binary contracts).
`cli-pc` wraps the machine command and delegates its Debug formatting, so
`upcli`'s usage and current 252-line parse pin stay unchanged (the row
above records its earlier 215-line size). The machine usage
lines live in `unemupc`; an exported literal macro lets `cli-pc` append its
image-import lines in the existing usage slot. The library's
`parse_with_store` retains the frontend's store selection as data;
standalone `parse` refuses both store flags. `execute_with_loader` takes
the host's preparation function: `cli-pc` keeps store lookup and warning
ordering, while the standalone runner loads with no firmware packages.
The run itself, diagnostic output and exit classification have one home.
The raw commands remain in `upcli`; their integration tests stay there
until step 6. Seven unit tests move with their names unchanged; the
store-option test remains in `cli-pc`. Its existing `media_tools` parse
assertion follows the command wrapper; the `command_line_pin` source and
fixtures do not change.

All emulation crates are implicitly core-side; `machine-file`,
`pc-harness` and `unemupc` declare `side = "core"`. The boundary walk
checks that side as well as layers and frontend families, for all declared
dependency kinds and transitive normal/build paths. Of §12.8's original
three crossings, only `machine-file` → `media` (normal) and `pc-harness`
→ `media` (dev) are needed: step 7 already removed the store crossing.
Existing `assembly` → `media` exceptions remain. Fixtures cover direct
normal/build/dev crossings, transitive leakage, a neutral helper, the
permitted contract/file/core paths, test-only edges and stale exceptions.
The affected-test runner recognizes the new machine-runner home.
No release workflow change is required by §12.9 or this row: step 4
renamed the desktop in releases, while this step adds the standalone
workspace binary. Release packaging still contains `unemu` and `upcli`.

MEASURED: the unchanged `upcli` `command_line_pin` passes all four tests,
including the usage and parse of 252 lines, also 252 on the base commit
(the planned row says 215). The original §7.5 step-4
invocation corpus has 23 machine-command cases: all 16 non-usage cases
have byte-identical stdout, stderr and exit codes; the seven usage errors
have identical stdout, code 2 and refusal reason, with the explicitly
specified different usage block (§12.8). `run` comparisons disable the
frontend store. Historical absolute repository paths are rebased to this
checkout; the corpus's empty working directory is retained. All seven
moved tests pass before and after. Each of the three new binary tests
fails on its corresponding mutation (store flags accepted, notices
omitted, arguments dropped), then passes after restoration. A real
`unemupc` → `image-registry` manifest dependency fails the new rule with
its path; the same planted metadata passes the old rule. The manifest
and lock file are restored before final checks.
The full goldens-tier gate records 4,333 passed tests and 141 sandbox
failures (140 loopback binds and one host-clipboard test); its doctests and
894 release-profile device tests pass, as do formatting, guards, clippy
and rustdoc. Cargo-deny is left to the supervisor, as required by the
sandbox brief. A final offline all-target build, workspace clippy,
formatting check, the standalone crate's ten tests and the real boundary
check pass. Plain `cargo test --workspace` stops at the expected
`cli-core` loopback failure after 676 passes.

**Step 6, as landed (2026-11-20).** `parse_with_store` accepts only
`run --machine`; raw `--boot`, `--bios` and `machine check` remain in the
standalone parser. The frontend drops their usage lines and keeps the
shared file-run usage and store-backed loader. Removed words receive the
normal usage text followed by `unknown or extra arguments`, exit 2, with
no redirect: §12.9 assigns commands but prescribes no migration message.
`upcli`'s pin changes only for the three removed forms (five usage
variants, four saved documentation examples and their nine parse records);
the first remaining usage line acquires the `usage:` prefix. Every other
parse record, including refusals, and the recipe/about pins stay unchanged.

Twelve tests move with their names and assertions: all six `machine_check`
tests from `cli-pc`, the three binary `--bios` tests of
[`upcli/tests/firmware_catalogue.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/upcli/tests/firmware_catalogue.rs), the two binary tests of
[`upcli/tests/timing_adoption.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/upcli/tests/timing_adoption.rs), and the `--boot` part of
[`cli-pc/tests/media_tools.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/tests/media_tools.rs), to the same test filenames under
`unemupc/tests`. The journal-lock tests call `unemupc::run` in-process,
preserving a test binary without process creation so children cannot inherit
a sibling test's released lock. The other tests invoke `CARGO_BIN_EXE_unemupc`;
the boot parse assertion names the standalone command instead of the frontend
wrapper.
The journal usage cases and usage's `media verify` assertion remain in
`cli-pc` as `usage_errors_are_two`, because journals do not belong to the
standalone binary yet; the direct harness catalogue and timing tests stay
in `upcli`. Four more raw-command tests, absent from the row's inventory,
move (sixteen in total) from `upcli/tests/machine_commands.rs` to
`unemupc/tests/machine_commands.rs`: valid/invalid/usage exit codes,
inherited firmware, the shipped machine and explicit ROM discovery. Their
assertions stay unchanged; the invocations omit frontend-only `--no-store`,
and seven frontend run/journal tests stay in `upcli`.
The standalone usage also points to the DOS goldens, preserving
the moved usage assertion. The mixed [`cli-pc/tests/at_games.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/tests/at_games.rs) keeps media
creation in the frontend and calls `unemupc::run` for machine checks,
without changing assertions. The mixed guide test
`preparing_disks_guide_override_loads_the_real_ide_profile` in
[`cli-pc/tests/media_create.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/tests/media_create.rs) likewise calls the core library only for its
machine check, retaining media creation and every assertion.
The moved journal-inspection fixtures import
`Header`, `JournalMedia`, `SinkStatus` and `HEADER_LEN` through the existing
`assembly::test_support` facade. Its four-name re-export keeps the
standalone package from adding a forbidden direct dependency on `media`;
no manifest, lock-file or boundary exception changes.

`commands.md`, README.md, `machines/README.md` and `status.md` describe the
new command homes. Unlike step 5, removing commands now requires shipping
the standalone binary to retain those commands for release users:
`release.yml` builds, checks and packs `unemupc` alongside `unemu` and
`upcli`, and updates the download and quarantine instructions. The archive
name stays unchanged. The release workflow is not dispatched here.

MEASURED: all sixteen moved tests pass against `unemupc`, with their
assertions unchanged (runner paths and the boot command wrapper adapted);
its full crate has 26 passing tests. The four `command_line_pin` tests pass:
252 parse records become 243, with only the nine raw-command records
removed; all remaining records are byte-identical. The new frontend
refusal regression passes and fails against the unchanged implementation
(`--boot` exits 0 instead of 2). Both mixed test files keep their assertions.

The final goldens-tier gate runs 2,404 tests: 2,335 pass, 69 fail on
loopback binds denied by the sandbox, and 20 are skipped. The platform-id
scan passes both tests; doctests, formatting, publication/host guards,
clippy (including the production frontend) and rustdoc pass. There is no
remaining command-move failure. The wider preliminary workspace run also
records the expected unsupported host-clipboard failure; that crate is
not selected by the final plan. Cargo-deny is not selected because no
manifest, lock file or dependency-policy file changes. The offline
all-target build, workspace clippy, formatting and real boundary checks
pass with the existing four exceptions; plain workspace tests stop at the
expected `cli-core` loopback denial after 676 passes.

**Step 11, record lifecycle and file-adapter compatibility (2026-11-21,
partial).** The record lifecycle moved into `journal::JournalMedia`:
mount/recovery, command frames, barriers, close and verified retention all
work over a contract `Record`. `journal-files::JournalMedia` keeps path
constructors and consumer fault injection, forwarding through a shared
record. A replacement callback carries no file or path types into the core.
The obsolete journal-specific record trait and duplicate record cursor are
removed from `journal-files`. All original file-model and real-file tests
still exercise the production journal path; none was replaced with a
reference implementation to make its assertions pass.

Offline inspection and mutation now hold contract 0.3's `PathLock`,
transferring the exclusive token into recovery instead of reacquiring the
lock. The session's existing file implementation and offline tools still
exclude each other through `FILE.lock`. Contract 0.3 also applies its
reserved-sidecar path checks; the CLI comparison corpus uses ordinary
`.upfj` names.

**Accepted changes and shared file engine (2026-11-21).** The supervisor
accepted two behavior changes. `JournalFile` and path-mounted sessions now
use `unemu-core-files::FileRecord`: header publication, exclusive stable
sidecar locks, protected-input cleanup, append/cut/barriers and replacement
all use that engine. The old record/replacement implementation is removed.
A shared owned `ToolIo` adapter keeps retention fault injection, exact modes,
verification-before-publication and old/new uncertainty reporting. The adapter
keeps established journal diagnostic text, removing the generic engine's
operation prefix while retaining its error kind. Journal creation also keeps
its original 0666 mode filtered by umask and its Unix-only durability barriers.
The test opening seam injects the original creation/open boundaries, then hands the
inode and lock to the production record engine; it contains no second record
or append/replacement implementation. Real-file tests exercise the engine's
creation, cleanup and retention paths directly. Contract 0.3.1
(`unemu-core-api` and `unemu-core-files` at `afba6e6b`, release v0.3.1;
NES core at `0dbf0d56` with the same contract) fixes first mount when
umask removes write permission: `FileRecord` keeps its writable creation
descriptor after publication. The restrictive-umask regression runs by
default, preserving the published file's 0444 mode while appending and
finishing through that descriptor.

1. A failed append now rolls back its partial bytes. The seven-byte probe
   leaves 128 bytes instead of the previous 135 when rollback succeeds;
   `Storage` means bytes unchanged, `StorageUncertain` means undo failed.
   The journal latches either error and stops output. Successful earlier
   frame appends are not undone, so a command with no acknowledged end may
   still leave full frames for recovery. `media verify` may report fewer
   discarded bytes, or none. Recovery still discards real process-death
   tails and tails left by unsuccessful rollback. The existing all-cuts failed
   append test pins the new length and records the old length in its comment;
   a separate test kills a writer after seven bytes reach the file.
2. Mounts and offline tools refuse the engine's reserved sidecar names with
   the same Usage message, `reserved sidecar path`, before lock/data changes:
   `.lock`, `_lock`, `.create-`, `.retain-`, case variants and legacy `_N.tmp`
   suffixes. Both the supplied spelling and canonical target are checked.
   Assembly and machine-file validation use the same check, so no legacy
   path constructor bypasses it. Tests cover both generated name families,
   `<sha256>.upfj` and `<set>-<id>.upfj`, which remain valid. Existing export
   temporary restrictions remain. These two changes are stated in commands.md
   and status.md; no other journal byte or CLI result is exempted from comparison.

**Step 11, final extraction (2026-11-21).** `journal` is now
`emulation` (core); `journal-files` is `pc`, `side = "core"` (host).
`machine-file` owns `FileMount` (`mount.rs`), host `assemble*` entry points
(`assemble.rs`), output-path validation (`mount_paths.rs`) and floppy
append-by-path (`floppy.rs`, `FileMachineExt`). Callers select those host
entry points explicitly. Assembly's inline entry points use `InlineMount`;
`assemble_mounted` remains the content/record seam. Config validation in
assembly is logical; `machine_file::validate` adds the original host checks
through `MachineConfig::validate_mounted`, preserving their order.

`ImageBytes` and `DiskBase` retain only opaque origin tokens (`Rc<dyn Any>`),
never filesystem metadata. The file mount creates and interprets the tokens.
`JournalStore` defers opening until the verified base, geometry length and
status are available, so the file adapter still protects the input inode
while creating or replacing a journal. `JournalSession` exposes sector reads,
command writes, boundary persistence and finalization; file sessions keep
the file engine's retention/publication and failure-injection policy, while
record sessions keep the journal's policy. A removable medium retains its
mount for every insert. Append-by-path keeps the original cross-input/output
checks, including appended media on a machine over records.

**Placement adjustment to §12.5 move 3.** `media` keeps neutral FAT12
formats/readers/builders. Its former `create` and `layer` modules move
unchanged to `journal-files`: both use the PC file engine, shared file-base
receipts and journal overlay helpers. Keeping them in neutral `media` would
leave `cli-core`'s FAT12 export transitively dependent on a PC/core host crate.
Moving these two modules closes that path without a new boundary exception
or a second implementation of publication or sector-source behavior. All
callers now use `journal_files::{create, layer}`; the journal facade in
`media` is gone. Real-file assembly tests move to `machine-file`, retaining
their test names and assertions.

**Final row proofs.** Both `assembly -> media` exceptions and
`machine-file -> media` are removed; `pc-harness -> media` (dev) remains for
the FAT12 reader. Emulation no longer reaches `unemu-core-files`; its only
path to `libc` is `sha2 -> cpufeatures` on targets requiring the capability
query. The serde/TOML reasons name only the `pc-config` codec, which still
needs them. The host guard falls from 20 entries to 6: the four existing
assembly test-directory helpers and the two CPU immutable reset-cache lines.
No `assemble.rs`, `mount.rs` or `observation.rs` exception remains.
`HashMap` is correctly flagged because its default hasher requests host
randomness, even when the table is only used for lookup. Those tables now
use `BTreeMap`; the scanner has not been weakened.

MEASURED final extraction checks: offline all-target build, workspace clippy
with every target and feature and warnings denied, formatting, rustdoc with
warnings denied, crate boundaries, the external list and the host guard pass.
All 4,409 original annotated test names remain; two cases are added. Retained
file/record sessions produce identical journal bytes, durable records and
unchanged bases across four sessions; disabling retention-policy forwarding
fails this case (four generations instead of one). File-only retention fault
injection leaves record sessions with the storage's own behavior, as before;
its regression first fails on the extracted adapter's rejection, then passes.
All 192 saved CLI stdout/stderr/exit-code/output-file-hash records match both
the part-one and file-adapter baselines, with no exclusions.

The full affected plan selects the workspace/goldens tier because Cargo.lock
changes. Its run has 4,375 passes, 142 failures and 20 user-ROM skips: 140
failures are denied loopback binds, one is an unavailable clipboard, and one
is a file-backed test still importing the inline assembler through a wildcard.
That import is corrected and the CLI creation binary's nine cases pass afterward.
The gate also finds a redundant test borrow and a stale FAT12 publication doc
link; both are fixed and the full clippy and rustdoc commands pass afterward.
The final focused run passes eight mount-seam and 28 persistence cases. Doctests
and all 894 release-profile device tests pass. The full gate is not rerun after
these focused corrections; user-ROM checks and cargo-deny remain the supervisor's
lane outside this sandbox. No code move remains in this row.

MEASURED validation of the earlier file-adapter landing (before the final extraction): the journal crates
pass 120 tests, including all archived original names and the four new
rollback/reserved-name/mode/crash-tail tests. The new assembly case passes;
the assembly, journal CLI and machine-file consumer suites pass 28, 32 and
42 tests. Four mutations fail before their fixes are restored and touched.
Three original CLI/session warning tests caught generic engine prefixes;
the adapter restores their existing text and the tests pass unchanged.
The four pre-record byte/digest pins still pass. All 192 saved CLI records
(stdout, stderr, exit code and file hashes) are identical, with no exclusions.

The final offline all-target build, workspace clippy, formatting, publication
and host guards, boundary self-tests, actual boundaries/external list and
rustdoc pass. The policy files have no diff: four boundary exceptions and
20 host entries remain. Plain workspace tests reach the expected CLI
loopback denial after 616 passes, one failure and two ignored tests.
The gate plans 27 paths (including both sides of the store rename), five
directly changed crates and 20 affected crates, at the goldens tier. Nextest
runs 2,647 tests: 2,577 pass, 70 fail on permission-denied loopback setup,
and 20 user-ROM tests are skipped. One passing capability test is marked
leaky by nextest; no independent clipboard failure is observed. The separate
platform-id scan passes both tests, and doctests pass. Cargo-deny is not run
in this sandbox; the temporary gate copy skips only that lane and fixes its
working directory. No gate script in the repository is changed.

The gate takes 431 seconds: plan 1, formatting 2, guards 13, clippy 2,
production CLI clippy 4, affected tests 342 and rustdoc 67. The runner's own
steps take 330 seconds for tests, 10 for doctests and 2 for platform-id scans.
The five slowest test binaries by summed case duration are `pc-ui`
(134.87 s), `pc-image::template` (85.69 s), `assembly` (85.37 s),
`pc-library` (81.54 s) and `window::bin/unemu` (77.13 s).

The following validation records the staged state before these accepted changes.

MEASURED validation: the two journal crates have 116 passing tests; all
original journal test names remain. The new record-only replacement-callback
test fails when pruning bypasses the callback (zero calls instead of one),
then passes after restoration. The four pre-record byte/digest pins and the
path/record/memory comparisons still pass. Re-running the saved part-one
corpus gives 192 identical stdout/stderr/exit-code/file-hash records.
The final offline all-target build, workspace clippy, formatting, publication
and host guards, crate boundaries with the existing allow list, external list,
and rustdoc pass. Plain workspace tests stop at the expected CLI registry
loopback denial after 615 passes (one failure, two ignored). The affected gate
selects 20 crates (both changed journal crates and their dependents) at the
goldens tier: 2,572 of 2,642 selected tests pass, 70 fail on denied
loopback binds, and 20 user-ROM tests are skipped; the separate platform-id
scan passes both tests and the doctest command passes. No independent
clipboard failure was reached in this run. The runner spends 460 seconds
in tests, 39 in doctests and 6 in the platform-id scan; the whole gate takes
616 seconds and exits 1 only for the test step. Formatting, guards, clippy,
the production CLI clippy and rustdoc pass; cargo-deny is skipped for the
supervisor outside the sandbox. The five slowest test binaries by summed
case duration are `pc-ui` (249.56 s), `pc-image::template` (131.06 s),
`pc-library` (105.66 s), `assembly` (104.01 s) and
`pc-image::publication_death` (102.44 s).

The following note records part one's state; the lifecycle and offline-lock
statements above supersede its corresponding pending items.

**Step 11, partial extraction (2026-11-20).** `journal` owns the unchanged
wire codec, scanner, closed-prefix reconstruction and host-independent
`SectorSource`, `VerifiedBase` and `SparseOverlay`. `journal-files` owns
`JournalFile`, file-backed verified bases, the durable sink, offline tools,
locks and temporary cleanup. `media` keeps FAT12, image creation and disk
layers, and temporarily re-exports the journal APIs at their old paths.
The two new packages temporarily declare layer `neutral`, with `side =
"core"`: the compatibility facade must remain usable by `cli-core`'s
neutral FAT12 image export. These are intermediate tags, not completion of
the planned `emulation`/`pc` split. The record-driven sink still lives with
its injectable file retention seam in `journal-files`; its final move into
`journal` remains pending.

The journal command implementation moved from `cli-core::journal` to
`unemupc::journal`, re-exported by `cli-pc::journal`. Both command lines use
it; `cli-core` offers a journal usage slot and only the PC fills it.
`media_tools` and `media_fifo` move to `cli-pc`, keeping their names and
assertions; the former joins its existing usage-error test. The parse pin
changes only by wrapping successful journal parses in `Platform(...)`.
Words, usage ordering, report text and exit codes remain the same in
`upcli`. `unemupc` now offers generations, verify, rollback and commit
(including compact), without store/registry dependencies.

The contract repository was read, not changed. Its pinned file API is
`FileContent::{open, from_file}` and `FileStorage::{open, dir}`, with records
opened by contract names under that directory. `journal-files` uses the
existing `FileContent::from_file(File, size)` and `ContentReader::read_at`
for verified base reads, retaining full-file SHA-256 and the stronger
existing metadata checks. `FileStorage` is not a replacement for an
arbitrary existing journal path and its publication protocol.

**Contract-repository follow-up.** A generic, arbitrary-path file record
adapter is needed before moving the remaining journal file code there.
The adapter must expose the equivalents of the current
`JournalFile::open(path, &Header, Option<&Metadata>)` and
`FileRecord::open(Filesystem, path, lock, header_bytes, base_metadata)`:
contract `Record`/`Replacement` objects, an expected initial byte prefix
(the contract file crate must not depend on this journal format), and a
protected base-file identity. It must preserve `FILE.lock` exclusion,
shared read-only inspection (including an absent lock in a read-only
folder), no-clobber header publication through `FILE.create-N.tmp` and a
hard link, mode-preserving `FILE.retain-N.tmp` replacement, alias-safe
leftover cleanup, and file/directory synchronization with exact failure
stages and `StorageUncertain`. The host test seam must cover the operations
of `Filesystem`, `JournalIo`, `CreationTempIo` and `ToolIo` (open/lock,
metadata/identity, temporary creation and writes, barriers, link/rename,
cleanup and directory sync), including retention replacement injection.
These types/functions currently live in `journal-files/src/journal_file.rs`,
`temporary.rs`, `offline.rs` and `store.rs`; they need a generic public
counterpart in `unemu-core-files`, not a dependency on `journal-files`.

**Remaining row proofs at the earlier partial landing (superseded above).** Neither `assembly` → `media` allow entry is
removed: assembly still reaches the compatibility facade. The host allow
list stays exact at 20 entries: `persistence.rs` has none on this base;
`mount.rs` retains seven file opens/metadata entries; `assemble.rs` retains
six map entries, including `NvramBases` with `std::fs::Metadata`. Moving
the file mount and metadata ownership into `machine-file` remains pending;
the other five assemble entries are device lookup maps, not journal file
operations. `libc` still reaches emulation through media/journal-files as
well as cpufeatures; serde/toml still reach it through media as well as
pc-config. The external list records the existing `unemu-core-files`
dependency newly reached through the facade and updates libc's reason.
It cannot yet claim the row's final dependency proof. No allow entry is
added or widened.

MEASURED: the original 139 media unit-test names are exactly the union of
`journal` (2), `journal-files` (79) and `media` (58). The integration suites
keep their names and assertions: `journal` (23) moves to `journal`,
`journal_storage` (7), `sparse_large` (1) and `temporary_fifo` (2) to
`journal-files`; `boot_sector_source` (1) stays in `media`. The 31 moved
`media_tools` tests join the existing PC usage test (32 pass), and
`media_fifo` (1) passes in `cli-pc`. One journal unit test moves from
`cli-core` (17 → 16) to `unemupc` (7 → 8). Three new standalone tests cover
usage, all four commands and compact without base mutation, and exit codes
0/1/2/3; all three fail when routing and journal usage are removed, and pass
after restoration. The pre/post `upcli` binaries have identical stdout,
stderr, exit codes and written-file hashes over 192 invocations (clean,
torn-tail, wrong-base and missing-base cases). The affected-test policy now
selects goldens for either new crate; a journal-only plan selected fast
before the change, and goldens after it. The runner self-test also caught
missing standalone readers of shared harness support and machine fixtures;
those mappings are corrected. The all-target build, clippy and real crate
boundary/host checks pass. User-ROM and cargo-deny checks remain the
supervisor's responsibility outside the sandbox.

MEASURED gate: 4,364 of 4,505 selected tests pass; the 141 failures are exactly
140 sandbox loopback permission denials and one unsupported host clipboard.
Twenty user-ROM tests are skipped; all 894 release device tests pass.
Formatting, guards, both clippy checks, doctests and rustdoc pass; cargo-deny
is explicitly skipped in a scratch copy of the gate, reserved for the supervisor.
Final focused checks after preserving the PC parser's fall-through for unrelated
platform words pass the existing parser regression plus 41 CLI/pin tests;
all-target build, clippy and formatting pass again. The 192-invocation binary
comparison remains identical after that correction. Existing test names remain
unchanged.

**Step 12, as landed (2026-11-20).** Host discovery lives in
`machine_file::firmware_discovery`: `crates/machine-file/src/firmware_discovery.rs`
and its `frontend.rs` and `persistent.rs` submodules. The existing private
machine-file resolver is now `firmware_resolve.rs`, so the public discovery
module keeps its name. The catalogue and identity checks over bytes remain
in `assembly`; discovery uses its public catalogue instead of a private
catalogue constant in one synthetic test. All 60 discovery tests keep their
names and move with the module. The shared user-ROM helper and its macro
move to `machine-file` (`test_support/user_firmware.rs`, feature
`test-support`); a self dev-dependency enables that feature for its own
integration tests. Host callers in `machine-file`, `unemupc`, `cli-pc`,
`pc-image`, `pc-library`, `pc-ui`, `upcli`, `window` and the harness tests
change their import paths. `pc-ui` and `upcli` add the direct dependency;
no emulation crate depends on the new home (layer `pc`, side `core`).
The package-lookup seam and stage B's launch path are unchanged.

The row's empty-list proof cannot hold on this base: step 11 has not
landed, and the host guard also lists maps, feature-gated temporary-directory
support and the CPU reset cache, which step 11 does not remove. This step
removes exactly 21 discovery entries, reducing the host guard list from 41
to 20. The remaining entries are `assembly/src/assemble.rs` (6: lookup
maps, including the NVRAM metadata table), `assembly/src/observation.rs`
(1: its map), `assembly/src/mount.rs` (7: file-backed image metadata and
opens), `assembly/src/lib.rs` (4: temporary-directory test support) and
`cpu/src/x86jit_backend.rs` (2: the immutable once-built reset image).
They retain their exact source lines and reasons in
`scripts/core-host-guard.allow`; no exception is added or broadened.
What makes the list empty: step 11 moves `mount.rs`'s file-backed
metadata and opens out (7); the lookup maps of `assemble.rs` and
`observation.rs` (7) become ordered maps or move with the NVRAM metadata;
the temporary-directory test support of `lib.rs` (4) moves to a
test-only crate; and the CPU's reset cache (2) is built without a
process-wide once cell. Each is its own small change after step 11.
MEASURED: the 60 moved tests pass before and after, with an empty diff of
their full names. The new public discovery API test fails against the old
private resolver module and passes after the move. Both
`user_rom_landmark_at_identity` and `user_rom_ibm_1985_identity`, run by
name with `UNEMUPC_USER_ROMS_DIR` unset, print SKIPPED and return cleanly.
The boundary check passes (37 crates); the host guard passes with 20
entries for 21 matched lines and its 66-case self-test passes. The full
workspace goldens-tier gate records 4,334 passes, 140 denied loopback
binds and one unavailable host clipboard; all 894 release-profile device
tests and the doctest pass. Formatting, both clippy checks and rustdoc
pass; cargo-deny is reserved for the supervisor and was suppressed at
invocation, not run. The all-target build passes. Plain workspace
`cargo test` stops at the expected loopback failure after 616 passes.

**Step 13, as landed (2026-11-21).** The `window` package still builds
`unemu`. Its first argument selects the route: a word runs
`upcli::run_command`, including unknown words so the command parser owns
its refusals and exit codes; no argument or a leading dash keeps desktop
parsing. No GUI or audio setup precedes command dispatch. `--help` combines
desktop and command usage; `--version` and `--about` remain the same desktop
notice, with every compiled-in core and the font/component notices.
[`crates/upcli`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/upcli) stays as an application-layer composition library, with its
usage/About/parse pins and library tests. Its binary and `main.rs` go.
The three tests that spawn the executable (`machine_commands`,
`image_registry_commands`, `raw_commands`) move to `window`; raw desktop
options retain the desktop refusal (exit 1), while `machine check` is an
unknown command (exit 2). The command-line pin changes only `upcli` to
`unemu`; the argument/parse corpus is unchanged. An explicit normal `window` → `upcli` allow-list entry permits application
composition. Other UI crates still cannot depend on CLI crates, and CLI
crates cannot depend on window crates. Fixtures cover both sides.

The release workflow builds, checks `--version` and archives only `unemu`
and `unemupc`. The renamed `setup-unemu` action retains checksum and optional
source-commit validation and installs `unemu`; its workflow installs the
ALSA build/runtime package on Ubuntu 22.04, builds and packages the current
binary, substitutes only curl's download transport and installs it through
the action with its checksum check. It runs `--version` and `media create`. The licence generator starts from
`window` and `unemupc`; production-binary lint follows `unemu`.
Current command examples in the guides and design reference text follow
the program name; dated implementation and timing records keep `upcli`.
Only name-bearing usage/messages in the other worker's command/runner crates
change; mounting and assembly call sites are untouched.

ALSA choice: Q1(a), with the runtime package rather than a headless Cargo
feature. ASSUMED from the Linux link closure: without `libasound.so.2`,
the loader refuses even command words before Rust starts. This sandbox
cannot run a Linux container. The release's Linux job therefore runs the
`unemu image ls` in bare `ubuntu:22.04`, checks the missing-library refusal,
installs `libasound2`, then checks the ordinary missing-store exit, runs
`--version` and formats a floppy without a display.
That hosted measurement and the release/action workflows remain pending;
local YAML, packaging and install probes do not establish hosted success.

MEASURED validation (2026-11-21): the original cached desktop executable
refuses command words and media creation and omits command usage from help;
the restored implementation passes all 17 focused tests (five dispatch,
four command-line pins, seven moved machine commands, one raw-command
refusal). The 246 argument/parse records and saved command examples are
byte-identical; usage, recipe usage and the command About pin differ only
in the program name. The full goldens-tier bead gate takes 961 s: formatting,
guards, workspace and production-binary clippy and rustdoc pass; tests record
4,381 passes and 141 sandbox failures (140 denied loopback binds, one host
clipboard), with doctests and all 894 release-profile device tests passing.
Cargo-deny is explicitly delegated to the supervisor. The final offline
all-target build, workspace clippy and formatting pass; ordinary workspace
tests stop at the expected CLI loopback refusal after 616 passes. The local
action probe installs the actual binary and creates formatted media, refuses
a missing release, a mismatched source commit and a bad checksum; the former
installer refuses that archive because it contains no `upcli`. Boundary
fixtures and the real graph pass, as do YAML/action syntax checks. No GUI
or container is launched locally.

### 12.11. Where the steps fit

- **Steps 1 to 3** touch scripts, lists, one attribute per crate and one
  line of the root manifest. They can land now. Step 1 and every
  crate-creating move of §8.1 edit `scripts/crate-boundaries.sh` and its
  fixtures, so they go one after another, not side by side.
- **Step 4** is small and textual, and every later step and document
  names the binaries, so it goes early. It edits strings in [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs),
  [`app_menu.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/app_menu.rs) and `main.rs`, which the moves of §8.1 step 6 also touch;
  it lands between two of them. It takes over the optional rename of move
  6.5.
- **Steps 5 to 7** are in `cli-pc`, `upcli`, `machine-file` and the new
  crate. They share nothing with the window split. They share the usage
  text and its pin with the recipe track's command beads
  ([recipes.md](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/recipes.md) §19), so a bead that changes the pin and a
  recipe bead that adds a command go one after the other.
- **Steps 8 to 12** change `assembly` and `media`, which every PC crate
  and the desktop build on, and each of them runs the slow goldens. They
  are the first half of §4 stage B; of them stage B's steps need only 9
  and 10 (and 11's `journal` crate in one case), and its first eight
  steps run beside steps 1 to 7 (§14.5; corrected 2026-11-19, stage B
  plan). They go after the worker and the
  session have settled where they call `assembly` (§8.1 move 3.11 and
  `upc-dfs.25.7`), so that a moved call site is not edited twice. Step 8
  alone touches only the contract and can land at any time; it is steps
  4 and 5 of §13.6 now.
- **Step 13** follows Q1 (a) and changes the application dispatch, release
  packaging and setup action together.
- **Step 14** follows step 4 and is independent of the command-line join
  and firmware-discovery move. It changes only front-end host-location
  preparation and its tests; the core's cache keeps its name. It touches
  `image-store`'s locations, the window's startup and directory pins, and
  the directory descriptions in the user guides.
- **The recipe track** is on the front end's side and needs none of these
  steps. Two later meetings: the Machinefile's input steps and
  `unemupc`'s key script should be one vocabulary (§12.8), and a recipe
  that runs a machine will want a storage in memory or in a temporary
  directory, which 0.2 gives it.

Two workers can hold steps 1 to 3 and steps 4 to 7 at the same time as
the window split, if the beads that edit one file are ordered.

### 12.12. Not required now

| Not required | Why not now | When it is worth it |
|---|---|---|
| `no_std` | It would replace `thiserror`, the maps, `Rc` and the I/O traits in every core crate and in x86jit, for no target that exists. `std` without its host interfaces is what the checks of §12.3 enforce | A target without `std`: bare metal, a console SDK. The checks make the remaining distance small and known |
| A build for a host without an operating system, such as `wasm32-unknown-unknown`, in CI | It needs a second target in the devShell and proves little more than the two checks while `media` is still linked | After step 11, as the cheap proof that nothing needs a C library: one `cargo build -p unemu-pc-core --target …` in the gate |
| A C ABI | No embedder is written in another language | When one is. It is a wrapper crate (a `cdylib`) over the contract; the contract is object-safe and generic-free so that the wrapper is mechanical |
| A libretro wrapper | libretro assumes one core instance per process, a frame-stepped console and one save file. The PC needs a keyboard and a mouse, media catalogues and journals, and it has no frame stops yet (§8 step 2) | After contract 1.0, first for the NES core, whose machine is what libretro models. For the PC only on request (§12.13 Q3) |
| Cores loaded at run time | Out of scope since §1: cores are compiled in at pinned versions | With the C ABI, if ever |

### 12.13. Questions answered by the maintainer

Answered on 2026-11-18, each with the recommendation: Q1 (a) one binary
`unemu`; Q2 the front end keeps only `run --machine`, and `--boot`,
`--bios` and `machine check` live in `unemupc` only; Q3 a libretro wrapper
after contract 1.0, the NES core first; Q4 the journal moves to the PC
core now; Q5 the front end's host directories move to `unemu` with a
one-time read of the old place. The options are kept for the record.

**Q1. Is `unemu` one binary or two?**

- (a) **One.** `unemu` with no arguments, or with the desktop's options
  (`--machine FILE`, `--display CARD`), opens the window; `unemu image
  ls`, `unemu media create …`, `unemu recipe check …` are the command
  line. The two do not collide once `--boot` and `--bios` have left: a
  command starts with a word, a window option with `--`. `upcli` goes.
- (b) **Two.** `unemu` is the window; the command line is its own binary,
  `unemu-cli`, built without a toolkit.
- (c) **One name, two builds.** As (a), with a Cargo feature that leaves
  the window out, for a headless package.

Recommendation: **(a)**, with (c) as the known way out. It is one name to
learn, install and document, and it is what the maintainer's sentence says:
the front end's binary is `unemu`. The cost is on Linux hosts without a
desktop: the binary links the system's audio library (ALSA) and will not
start without it, even for `unemu image ls` (READ: the lock file holds
`alsa-sys` for the host-audio crate; step 13 adds the bare-container release check; its hosted result is
pending). A CI job installs one package, or takes the headless build of
(c). The five-minute criterion of §1 is not affected: it is met by the
crates, which stay split.

**Q2. Do the raw run commands stay in the front end?**

- (a) **Only in `unemupc`.** `--boot`, `--bios`, `machine check` and
  `run --machine` all leave. A machine file is imported before the front
  end runs it.
- (b) **In both.** The front end keeps all four and calls `unemupc`'s
  library.
- (c) **The front end keeps `run --machine` only**, because that command
  adds something the core's command line cannot: firmware packages from
  the store. The other three live in `unemupc`.

Recommendation: **(c)**. The three commands need nothing of the front end
and are the core's own business; keeping them twice is two usage texts to
keep equal. `run --machine` with the store is a real front-end feature and
costs one call into the library that `unemupc` uses.

**Q3. When a libretro wrapper, if ever?**

- (a) Never; the contract is the interface.
- (b) After contract 1.0, for the NES core first; for the PC only when
  someone asks.
- (c) Now, for the PC, as the proof that the core embeds.

Recommendation: **(b)**. The proof that the core embeds is the two checks
and, later, the build for a host without an operating system; they cost
minutes. A wrapper for the PC would spend its effort on what libretro
lacks (catalogues, journals, a keyboard-first machine).

**Q4. The journal format's home is the PC core, now.** §11 Q4 answered
(a) "for now, decided finally at extraction". The embeddability rule needs
the answer earlier: the journal's format and logic become a core crate
(`journal`), the code that touches files leaves the core, and the four
offline journal commands become PC commands (in `cli-pc` and `unemupc`)
instead of neutral ones.

- (a) As designed in §12.5.
- (b) Keep `media` whole and neutral until extraction, with the
  `assembly` → `media` crossing and `libc`, `serde` and `toml` listed as
  exceptions of the core until then. Steps 9 to 11 wait.

Recommendation: **(a)**. (b) leaves the core opening files, which is the
one thing the principle forbids that the core still does.

**Q5. Do the host directories follow the rename?** The store is already
in `unemu` under the platform's data directory. The preferences directory,
the machine-file directory and the firmware index are in `unemupc`
[REPO: [`crates/image-store/src/location/preferences.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/src/location/preferences.rs),
[`crates/window/src/main.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/main.rs), `crates/assembly/src/firmware_discovery/persistent.rs`].

- (a) Leave them until the front end has its own repository; step 4
  renames the program only.
- (b) Move the front end's two (preferences, machine files) to `unemu`
  with step 4, reading the old place once, as the rename from the former
  project name did. The firmware index stays `unemupc`: it is the PC
  core's cache.

Recommendation: **(b)**, as a bead of its own right after step 4. The
maintainer is the only user, the move is already written once, and after
it every directory says whose it is.

## 13. The second platform

Status: **Accepted** on 2026-11-18 (design step `upc-dfs.27`), with the
recommended answer of every question of §13.9. The maintainer's decisions
of that day are binding and restated in §13.0. Nothing here is
implemented yet. This section
changes four places of §12 and adds one sentence to §3.12; §13.8 lists
them.

Markers in this section: **[REPO]** = read on `main` at `beb6558`, with
the path; **MEASURED** = produced for this section by `git log`, `grep` or
a manifest read, as each place says; **READ** = read in a file outside
this repository, named there; **ASSUMED** = not verified.

### 13.0. What is decided, and what this section adds

Decided by the maintainer on 2026-11-18:

1. unemunes, a NES core in its own repository, starts now, as a second
   and completely different platform, as soon as the front end can drive
   a core through the contract.
2. The contract gets its own repository, `unemu-core-api`, in the
   `unemu-org` organisation on GitHub, created from this workspace after
   this design. Until then it lives in `crates/core-api`.
3. The 6502 processor is a crate inside unemunes, written so that it can
   leave later as its own library, for a Commodore 64 core for example.
4. The front end's binary is `unemu`; a core's binary carries the core's
   name (decision-41).

What the unemunes side said the same day: its processor, picture and sound
crates need no contract; it depends on `unemu-core-api` only in its
adapter step and needs the contract's own repository before then, never a
revision of this repository and never a second copy. The gaps it expects:
an absolute pointer for the light gun, `Core::validate`, and sound at a
rational rate.

The feature freeze of 2026-11-15 stands for everything else. The generic
launch path of §13.3 is new behaviour that decision 1 asks for; nothing
beyond what a second platform needs to start is added here.

What the code is today, in the four places this section changes [REPO]:

- **One way to start a machine.** The shell holds
  `presenter: Option<Presenter>`, and `Presenter` is a
  `Session<PcMachine>` with the PC's floppy catalogue beside it
  [[`crates/window/src/main.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/main.rs), [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs)]. Every start is the PC
  library's (`poll_images`: `SessionTransfer::restore`,
  `assemble_with_verified_bases`, `Presenter::from_assembled`) or a PC
  machine file's. [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) names `self.presenter` 97 times (MEASURED:
  `grep`).
- **Sound at 48 kHz only.** `Session::new` refuses the host device for a
  machine that describes another rate, and the converter in
  `host_audio.rs` takes its source rate from the constant
  `FRAMES_PER_SECOND` [[`crates/machine-view/src/session.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/machine-view/src/session.rs),
  `host_audio.rs`].
- **A 4:3 picture.** The shell sizes the screen with the literals `4.0 /
  3.0` and scales by `unemu_pc_core::RASTER_SCALE`; `Frame::aspect`
  reaches `Picture` and is not read [[`crates/window/src/shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs),
  `display`].
- **No list of cores.** `platforms` names the PC's image providers and
  the licence notice; the one place that lists a `Core` is a test in
  [`crates/window/src/main.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/main.rs). `DesktopPlatform` and `CliPlatform` are
  store-side seams: validators, the push guard, format names
  [[`crates/library-model/src/platform.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/library-model/src/platform.rs), [`crates/cli-core/src/lib.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-core/src/lib.rs)].

The session itself is ready: `Session<M: Machine + ?Sized = dyn Machine>`
calls the contract alone and picks its keyboard, its pointer and its
removable slots from the description.

### 13.1. The contract's repository

**Name and place.** Repository `unemu-core-api` in the `unemu-org`
organisation, public, beside `unemupc` and `x86jit`. The crate keeps its
name, `unemu-core-api` (§11 Q2).

**Layout.** The package is at the repository's root, which is where a
history-preserving export of `crates/core-api` puts it. The root manifest
also declares a workspace, so that a second member can join without
moving the first.

```text
unemu-core-api/
  Cargo.toml          the package `unemu-core-api` and `[workspace]`
  src/                the contract, as `crates/core-api/src` is today
  files/              later: the package `unemu-core-files` (below)
  docs/
    versioning.md     the rules of "Versions" below
    embedding.md      how a core and a front end depend on the crate
    conformance.md    what a core must hold: the checklist of §13.5
  CHANGELOG.md
  README.md
  LICENSE…            as §13.9 Q1 is answered
  .github/workflows/ci.yml
```

- **One crate at the start.** `unemu-core-api`, with no dependency of any
  kind, tests included, as today [REPO: `crates/core-api/Cargo.toml`].
- **A second crate with contract 0.2: `unemu-core-files`**, the
  contract's objects over host files (`Storage`, `Record`, `Replacement`
  and `ContentReader` of §12.5). §12.8 already says it travels with the
  contract: a core's own command line needs it and may not depend on the
  front end. It is the one crate of the repository that touches the host,
  and no core library links it. §13.9 Q4 asks whether it lives here.
- **Tests.** The contract's unit tests move with it (the version rule,
  the rates, a scripted core and machine driven only as `&dyn Core` and
  `Box<dyn Machine>`). The storage in memory of §12.5 stays behind the
  crate's `test-support` feature.
- **Docs.** After the move the normative text is the crate's
  documentation comments and `CHANGELOG.md`. §3 of this document stays
  as the record of the design and gets one line at its top that names the
  repository and the release it describes. The three files under `docs/`
  are short and say only what the comments cannot: the rules for
  versions, for depending on the crate, and for a core.
- **CHANGELOG.** One section per release, newest first. Every entry is
  marked **breaking** or **added**, and every release names the commit a
  consumer pins.
- **Tracking.** Work on the contract is tracked in this repository's
  beads, under the epic that holds this design, until the front end has
  a repository of its own. The unemunes side asks for a change by naming
  it to the supervisor; it does not edit the contract.

**Licence.** `crates/core-api` is AGPL-3.0-or-later today: its manifest
takes `license.workspace = true` and every file carries the SPDX line
[REPO: `Cargo.toml`, `crates/core-api/src/*.rs`]. It has no third-party
code: no dependency, and the key names are those of the USB HID usage
tables.

The contract is the one crate that every core and every front end links.
Under the AGPL, as it is usually read, a program that links the contract
is a combined work under the AGPL as a whole. That is no burden for
unemupc and the front end, which are AGPL already. It is one for the
principle of decision-41: a core that "anyone can integrate easily" could
then be integrated only by AGPL programs, whatever licence its author
chose for the core, because nothing drives it without the contract. The
interface libretro cores are written against is under a permissive
licence for this reason, and x86jit, the other library of this family
that others are meant to embed, is `MIT OR Apache-2.0` (READ: its
manifest at the pinned revision). `deny.toml` already allows both
[REPO].

A permissive licence on the contract does not change the licence of any
core or front end. unemupc's crates stay AGPL. The contract holds
signatures, data types and a few dozen lines of arithmetic; what is worth
protecting is on either side of it.

This is the maintainer's decision, and it comes before the repository is
public, because the first public commit states the licence: §13.9 Q1.
ASSUMED: the maintainer holds the whole copyright of `crates/core-api`
and can relicense it alone.

**Versions.**

- The package version is the contract's: `0.1.0` for `CONTRACT` 0.1.
  Today the crate carries the workspace's `0.0.2` [REPO].
- **0.1** is the contract as on `main`. It moves unchanged.
- **0.2** is one release with two parts: the storage objects of §12.5,
  which are breaking, and the additions of §13.2. They go together
  because of the rule below: during major 0 every minor is a meeting of
  all cores and the front end, and because the NES adapter is then
  written once, against the storage and content objects it will keep.
- **Before 1.0**, as §3.2 has it: every minor may break, and a front end
  drives only cores that state its own minor
  (`ContractVersion::drives`). A minor is any change to what a core
  implements or to what a front end may call or read: a method, a field,
  a variant, a rule in a documentation comment that a core must hold. A
  patch changes neither: documentation, tests, a `test-support` helper, a
  named constant for a value that already existed, a release of
  `unemu-core-files`.
- **From 1.0**: a minor only adds, a major breaks, and the crate is
  published to crates.io with SemVer requirements (§11 Q1). 1.0 comes
  when the PC core and a second core both run on the contract (§10).
- **Release tags** are `vMAJOR.MINOR.PATCH` on `main`, signed. `main`
  between two tags may carry half of the next minor; nobody pins it.

**How a consumer depends on it.** As this repository pins x86jit:

```toml
# The contract between front ends and cores, at the commit of release v0.2.0.
unemu-core-api = { version = "0.2", git = "https://github.com/unemu-org/unemu-core-api", rev = "<the 40 hex digits of the tag's commit>" }
```

- The pin is an **exact revision**, never a branch and never a tag name
  (a tag can be moved), and it is the commit of a release tag.
- A bare `path` dependency is banned, as for x86jit: it breaks CI and
  every other checkout. To work against a local checkout, a gitignored
  `.cargo/config.toml` patches the URL, as `dev-setup.md` § CPU core
  describes for x86jit:

  ```toml
  [patch."https://github.com/unemu-org/unemu-core-api"]
  unemu-core-api = { path = "<your checkout>" }
  ```

  The patch applies to the whole graph, so a pinned core that names the
  same URL builds against the local checkout too.
- unemunes writes the same line in its own manifest. It never names a
  revision of unemupc and never copies the crate.

**One copy, and how it is guaranteed.** Cargo builds one copy of a git
dependency for one URL and one revision. Two revisions are two crates
with the same name, and a core built against one does not implement the
trait of the other (§3.2). Four things keep the build at one copy, the
first by construction and the others as readable failures:

1. **The compiler.** A front end that pins the contract at revision B and
   a core that pins it at revision A does not build: the line of
   `platforms` that lists the core (§13.4) fails, because the core's
   `impl Core` is for another crate.
2. **The boundary check.** `scripts/crate-boundaries.sh` gains one rule
   over the resolved graph (`cargo metadata --offline --locked`, the
   second input §12.3 plans; this rule brings it if step 1 of §12.10 has
   not landed): the graph holds exactly one package named
   `unemu-core-api`; otherwise it prints both sources and which crates
   depend on each. This turns the compiler's long error into one line.
3. **The start-up check and its test.** `CONTRACT.drives(implements)` for
   every compiled-in core, and the test that asserts it (§3.2 items 2
   and 3), catch a core that was built against the right copy and states
   another minor.
4. **The protocol for moving the pin**, below: the contract and every
   pinned core move in one commit of the front end.

**Moving to a new release of the contract**, while cores are pinned by
revision:

1. The release is tagged in the contract's repository.
2. Each core's repository re-pins, edits its `implements` literal and its
   code, and names the resulting revision to the supervisor. Nothing
   changes for the front end yet: it still pins the old contract and the
   old core revisions.
3. One commit of the front end moves the contract's pin, every external
   core's pin and `PcCore`'s `implements` literal together. The goldens
   pinned to a core's revision run, as they do when the x86jit pin moves.

A core that has not re-pinned blocks step 3, not the other cores' work.

**CI of the contract's repository**, on every push and pull request:

| Job | What it checks |
|---|---|
| Format and lint | `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings` |
| Tests | `cargo test`, and again with `--all-features` |
| Documentation | `cargo doc --no-deps` with warnings denied: every public item has its comment |
| No dependency | `cargo metadata` shows no dependency of `unemu-core-api`, of any kind. This is rule 4 of §7.4, which leaves this repository with the crate |
| No host | `cargo build -p unemu-core-api --target wasm32-unknown-unknown`: the contract builds where there is no operating system. After 0.2 a `grep` also refuses `std::fs` and `std::path` in `src/` (the host guard's contract lines of §12.3) |
| Oldest toolchain | the build with the `rust-version` the manifest states. It starts at this repository's (1.99) and is lowered when a consumer needs it, never raised above the lowest consumer's |
| Publication | the mechanical publication rules of this repository's `scripts/publication-guard.sh`, copied with their self-test |

`unemu-core-files`, once it exists, is tested on Linux, macOS and
Windows, because it is the crate with code for each.

**The extraction.** The supervisor runs it. Creating this repository is
the maintainer's decision of 2026-11-18 and the one exception to the rule
that publishing a repository is the maintainer's step.

1. The maintainer answers §13.9 Q1 (the licence). The repository does not
   become public before that.
2. In a scratch clone of this repository at `main`:
   `git subtree split --prefix=crates/core-api`, onto a branch of the
   clone. The branch holds the history of that directory and nothing
   else, with the package at its root. Today that history is one commit,
   `705b1af` (MEASURED: `git log -- crates/core-api`). Nothing is
   written to this repository.
3. The branch becomes `main` of the new repository. Commits on top of
   it, each small:
   - the manifest with its own fields in place of the workspace's
     (`version = "0.1.0"`, edition, `rust-version`, licence, the new
     repository URL, the description, `publish = false` until 1.0) and
     the `[workspace]` table;
   - the licence files and SPDX lines as Q1 is answered;
   - `README.md`, `CHANGELOG.md` with the 0.1.0 entry, `docs/`;
   - the CI workflow and the publication guard.
4. CI passes; the commit is tagged `v0.1.0`.
5. One bead in this repository (step 3 of §13.6): the root manifest's
   `unemu-core-api` line becomes the git pin above; the member and the
   directory `crates/core-api` go; `deny.toml` allows the new git source
   beside x86jit's; the boundary check changes as below; `dev-setup.md`
   gains the patch paragraph; `architecture.md` and §3 of this document
   name the repository.
6. The supervisor names the URL and the revision to the unemunes side.

**What this workspace keeps.** No source of the contract. It keeps the PC
adapter (`unemu-pc-core`), the test that every compiled-in core states a
contract the front end drives, the one-copy rule, and this document as
the design's record. Every dependent crate already names the contract
through the workspace (`unemu-core-api.workspace = true` in `pc-core`,
`machine-view`, `library-model`, `pc-media` and `window`; MEASURED:
`grep`), so the pin is one line.

The boundary check after the move: the layer `contract` has no workspace
package any more, so rule 4 of §7.4 goes to the contract's CI; the
external-crate list of §12.3, when it exists, names `unemu-core-api` for
the `emulation` layer, with the reason "the contract"; the one-copy rule
is new; the self-test gets a fixture with two copies.

### 13.2. Contract 0.2: what a second platform adds

Contract 0.2 is the storage objects of §12.5 and the additions below.
Each addition is marked on its own merits: **added** means it would be a
minor after 1.0 (a method with a default body, a field with a "nothing"
value, a variant of a `#[non_exhaustive]` enum); **breaking** means a
core or a front end written for 0.1 must be edited. The release as a
whole is breaking, because of §12.5.

| Addition | Kind | Who must act |
|---|---|---|
| Content kinds: `ContentKind`, `PlatformDescription::content` | added | a core that opens content states them |
| `Core::identify` | added | such a core implements it |
| `Core::validate` | added | a core whose `create` reads configs implements it |
| An absolute pointer: `InputEvent::PointerPosition`, `InputKind::AbsolutePointer` | added | a front end ignores a port it does not serve |
| Sound at any rate: the rule on `AudioDescription::rate` | no signature changes; **breaking** for a front end that assumed one rate | `machine-view` (step 1 of §13.6) |
| The picture: `Frame::visible`; the rule on a finished frame | added | a front end that ignores `visible` shows the whole picture |
| Pads: the rule on port order | no signature changes | none |
| Battery-backed memory | nothing beyond §12.5 | a core keeps it in a record |

#### Content a person opens

A stored image names its platform, and the front end knows which core
runs it. A file on disk names nothing. The core says which kinds of file
it opens, as data, and decides about one file by reading it.

```rust
/// A kind of content a person opens directly to start a machine: a
/// cartridge image, a tape image, a program file.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentKind {
    /// Stable id of the kind within its platform, such as `ines`.
    pub id: &'static str,
    /// The kind's name for people, in the plural, as the filter of a file
    /// dialog says it: `NES cartridge images`.
    pub name: &'static str,
    /// File name extensions of the kind, in lower case and without the
    /// dot. A hint: a front end uses them to filter a file dialog and to
    /// decide which core to ask first. They prove nothing; `identify`
    /// decides.
    pub extensions: &'static [&'static str],
}

impl ContentKind {
    pub const fn new(
        id: &'static str,
        name: &'static str,
        extensions: &'static [&'static str],
    ) -> Self;
}

#[non_exhaustive]
pub struct PlatformDescription {
    // … the fields of 0.1, and:
    /// The kinds of content a person can open to start a machine of this
    /// platform. Empty for a platform whose machines start only from a
    /// config, such as a computer that is assembled from parts.
    pub content: &'static [ContentKind],
}

impl PlatformDescription {
    pub const fn with_content(mut self, content: &'static [ContentKind]) -> Self;
}

pub trait Core: Send + Sync {
    // … `description` and `create`, and:

    /// Say whether this core runs `content`, and as what.
    ///
    /// `Ok(None)`: the bytes are not content this core opens; the front
    /// end asks the next core. `Ok(Some(..))`: the platform, the kind and
    /// the config of a machine that runs the content with the platform's
    /// defaults. Fails with `ErrorKind::Unsupported` when the core
    /// recognises the content and cannot run it, such as a cartridge
    /// board it does not model; the message names what is missing, and
    /// the front end shows it instead of "unknown file". Fails with
    /// `Content` or `Storage` as `ContentReader::read_at` did.
    ///
    /// The core reads what it needs through `content.bytes`: a header,
    /// or all of it. It keeps nothing: the call builds no machine and
    /// changes no state. The same bytes give the same answer in every
    /// run of one release of the core.
    fn identify(&self, content: &OfferedContent<'_>) -> Result<Option<Identified>, Error> {
        let _ = content;
        Ok(None)
    }

    /// Check a config document without building a machine, and name the
    /// content it needs.
    ///
    /// Fails with `ErrorKind::Config`, with diagnostics, as `create`
    /// would for the same document, and with `Unsupported` for a platform
    /// or a config type the core does not read. On success `create` asks
    /// its `ContentSource` for exactly the listed content. A front end
    /// verifies that content before it calls `create`; a store-side
    /// validator can be written over this call alone.
    fn validate(&self, platform: &str, config: ConfigDocument<'_>) -> Result<Validated, Error> {
        let _ = (platform, config);
        Err(Error::new(ErrorKind::Unsupported, "this core does not validate configs"))
    }
}

/// A file a person asked to open, as the front end hands it to a core.
#[non_exhaustive]
pub struct OfferedContent<'a> {
    /// The file's name as the person sees it, without any directory. A
    /// hint for the kind and for a title; never a path, and nothing the
    /// core can open.
    pub name: &'a str,
    /// SHA-256 of the whole content, computed by the front end over all
    /// of its bytes.
    pub sha256: [u8; 32],
    pub size: u64,
    /// The bytes with that digest and size.
    pub bytes: &'a dyn ContentReader,
}

/// What a core makes of offered content.
#[non_exhaustive]
pub struct Identified {
    /// The platform that runs it: an id of this core's description.
    pub platform: &'static str,
    /// The kind it is: the id of one of that platform's content kinds.
    pub kind: &'static str,
    /// The config of a machine that runs the content with the platform's
    /// defaults, in one of the platform's `config_types`. It names the
    /// content by digest and size, never by name. `validate` accepts it
    /// and lists the content; `create` reads it.
    pub config: ConfigDocumentBuf,
    /// The content's own title, when it carries one. A front end shows
    /// the file's name otherwise.
    pub title: Option<String>,
    /// What a person should know before the machine starts, such as a
    /// header that contradicts the content.
    pub notes: Vec<Message>,
}

/// A config document a core wrote.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigDocumentBuf {
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
}

impl ConfigDocumentBuf {
    pub fn new(media_type: &'static str, bytes: Vec<u8>) -> Self;
    pub fn as_document(&self) -> ConfigDocument<'_>;
}

/// What a valid config needs.
#[non_exhaustive]
pub struct Validated {
    /// Every piece of content `create` will ask its `ContentSource` for.
    pub content: Vec<ContentRef>,
}

/// One piece of content a config names.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRef {
    pub sha256: [u8; 32],
    pub size: u64,
    /// What it is, for messages: `cartridge`, `firmware`.
    pub label: String,
}
```

`create` keeps its one input form, the config document (§3.4). Opening a
file is two calls: `identify` writes the config, `create` reads it. So a
machine started from a file and one started later from a stored image
with the same config are the same machine, and the replay rule of §3.11
covers both.

A platform that opens content lists at least one config type, because
`identify` answers in one. A platform with neither is the PC of stage A:
no front end offers it (§4).

#### An absolute pointer

A light gun, a light pen or a touch screen points at a place on the
picture. The relative pointer of 0.1 cannot say where.

```rust
#[non_exhaustive]
pub enum InputEvent {
    // … the variants of 0.1, and:
    /// Where an absolute pointing device points now.
    ///
    /// `position` is a place on the whole picture of the port's display
    /// (never only its `visible` part): `x` from the left edge and `y`
    /// from the top edge, each a fraction of 65 536, so that pixel column
    /// `x * width / 65 536` and pixel row `y * height / 65 536` are meant
    /// whatever size the picture has when the event takes effect. `None`
    /// while the device points off the picture. The position holds until
    /// the next event for the port. The port's buttons are
    /// `PointerButton` events.
    PointerPosition { port: PortId, position: Option<(u16, u16)> },
}

#[non_exhaustive]
pub enum InputKind {
    // … the kinds of 0.1, and:
    /// A device that points at a place on one display: a light gun, a
    /// light pen, a touch screen. The front end sends where the host
    /// pointer is over that display's picture and does not capture it.
    AbsolutePointer { display: DisplayId, buttons: u8 },
}
```

`InputEvent::ReleaseAll` releases the port's buttons and leaves its
position; a front end that loses focus sends `PointerPosition` with
`None`. What the device senses there (the beam's time, the brightness
under the gun) is the core's business.

#### Sound at any rate

`AudioDescription::rate` is already an exact fraction, and
`drain_audio` already hands over frames at that rate. Nothing in the
signatures changes. What 0.1 left unsaid, and `machine-view` filled in
with "48 kHz or no sound", is said in 0.2:

```rust
#[non_exhaustive]
pub struct AudioDescription {
    /// Stereo frames per second, exact, from 8 000 to 192 000: the rate
    /// natural to the machine. A core never resamples to suit a host
    /// device; the front end converts to whatever it plays.
    ///
    /// A machine whose sound generator runs faster than 192 000 Hz (a
    /// chip clocked at the processor's rate) filters and divides by a
    /// whole number inside the core, because only the core knows its
    /// signal, and states the divided rate. It is still an exact
    /// fraction: 19 687 500/11 Hz divided by 40 is 984 375/22 Hz.
    ///
    /// A machine with one sound channel puts it in both samples of a
    /// frame.
    pub rate: Rate,
}
```

And on `Machine::drain_audio`: the number of frames a machine has handed
over since it was created differs from `now() * rate / clock` by less
than one frame, at every return of `run`. A front end may rely on that to
keep sound and picture together. MEASURED for the PC adapter, which mixes
at 48 000 Hz off virtual time: it hands over the floor of
`now() * rate / clock` (the test of step 1 of §13.6), so the rule stands as
written for step 5.

#### The picture

`Frame::aspect` already carries the shape of the whole picture, so a
256 × 240 picture with 8:7 pixels states 128:105 and nothing is added for
it. The front end must honour it (§13.3). Two things are added:

```rust
/// A rectangle of a picture, in its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameArea {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[non_exhaustive]
pub struct Frame<'a> {
    // … the fields of 0.1, and:
    /// The part of the picture that a display of the machine's time
    /// showed for certain; the rest is overscan, which may hold garbage.
    /// The whole picture when the core has nothing to say, which is what
    /// `Frame::new` sets. A front end may show only this part. `aspect`
    /// is always the shape of the whole picture, and the pixels keep
    /// their shape when a front end crops.
    pub visible: FrameArea,
}

impl<'a> Frame<'a> {
    pub fn with_visible(mut self, visible: FrameArea) -> Self;
}
```

And on `Machine::frame`: a machine that draws its picture line by line
returns the last picture it **finished**, never one it is still drawing.
"The picture at the last completed boundary" of 0.1 was written for a
text grid that is whole at any instant. With this rule a front end that
runs by deadline, as the session does, shows whole frames without using
`RunLimit::stop_on_frame`.

Cropping is the front end's choice (a view option), not a setting of the
machine: the core always hands over the whole picture, so a saved state,
a replay and a screenshot do not depend on it.

#### Pads

Nothing is added. Three things are stated:

- **What is plugged in is the machine's.** Which device sits in a
  controller port (a pad, a light gun, nothing) is part of the machine's
  config, and the machine's description shows the result as its input
  ports, constant for the machine's life (§3.5).
- **Port order is player order.** The ports of kind `Pad` are the
  players, first to last, in ascending `PortId`.
- **Which host key or button is which control is the front end's.** No
  core carries a key map, a dead zone or a turbo setting. The mapping is
  data of the front end (§13.3).

`PadControl` has what a NES pad needs (`Up`, `Down`, `Left`, `Right`,
`A`, `B`, `Start`, `Select`).

#### Battery-backed memory

Nothing beyond §12.5. A cartridge's battery-backed memory is one record
of the machine's `Storage`, under a name the core fixes. The rules a core
holds:

- The record is opened only when the content has such memory.
- It is written with `replace` and `commit`, so that it holds the old
  memory or the new one, whole.
- It is written at `finish`, and also when `run` returns with the memory
  changed and ten seconds of **virtual** time gone since the last write,
  so that a power cut loses little. The host clock is not involved, so a
  replay writes at the same points.
- A machine dropped without `finish` keeps what the last write made
  durable (§3.9).

#### Deliberately left out of 0.2

| Left out | Why | What brings it in |
|---|---|---|
| Choosing what is plugged into a port from the front end | It is a setting of the machine, and settings as data are out since §3.12. Until then a machine opened from a file has the devices `identify` chose, and the core's own command line can choose others | "Settings as data": a second platform with a settings dialog |
| Re-plugging a device while the machine runs | No machine needs it yet | A core that models it |
| Analogue axes, paddles, rumble, a microphone | No first-run need of the NES | The first core that has one: new variants, each a minor |
| Host game controllers | A front-end matter, and a new external crate in `machine-view`, which the maintainer decides | After the keyboard path works; the binding table of §13.3 already has room for a second kind of source |
| A media type per content kind, for the store | Store vocabulary; nothing stores NES content yet | §13.4, "When NES images enter the store" |
| Content of several files, archives | The front end hands over one file's bytes. Unpacking is a front-end matter | Asked for |
| Content that needs more than itself: a disk system's firmware | `validate` can list it, and a front end with a store can supply it; a plain "open file" cannot | §13.4, the same |
| A fast path for rewind and run-ahead; cheats; netplay | Features | Asked for |
| The number of sound channels, sample formats other than `i16` | Two channels of `i16` carry every machine in sight | A machine they do not carry |

### 13.3. The front end's generic launch path

"Open a file and run it": a content file, a core that says it runs it, a
machine made by `Core::create`, the session of `machine-view` over
`dyn Machine`, the window. No line of it names a platform.

**Three new crates and one grown one.** The path is built beside the PC's
and touches the PC's shell only where the two meet.

| Crate | Layer | Toolkit | Holds |
|---|---|---|---|
| `unemu-test-core` ([`crates/test-core`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/test-core)) | neutral | none | A scripted core that implements all of contract 0.3 and emulates nothing; the front end's tests run against it ("Testing before a NES exists", below) |
| `core-launch` ([`crates/core-launch`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/core-launch)) | neutral | none | From a file and a list of cores to a machine: which cores to ask, hashing, `identify`, the content source, the machine's storage, `create`. Not a window crate: a later `image run` through the contract (§12.9) starts here too |
| `player-ui` ([`crates/player-ui`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/player-ui)) | neutral, a window crate | egui | The view of one running machine driven through the contract alone: the screen, the status line, the machine's menu built from its description, the Input dialog |
| `machine-view` (exists) | neutral | winit | Grows: sound at any rate, pads from the keyboard, the absolute pointer, reset by kind, saved state, the visible part of a picture |

`core-launch` depends on `unemu-core-api`, `unemu-core-files` and `sha2`;
`player-ui` on `machine-view`, `core-launch`, `ui-kit`,
`unemu-core-api`, `egui` and `winit` (the session's key, button and window
types). None of them may name a PC crate, in its tests either
(§7.3), which is what `unemu-test-core` is for.

**The path, step by step.**

```rust
// core-launch

/// The cores a program was built with, checked against the contract it
/// drives. A core that states another contract is kept with the reason,
/// for the About text, and is never asked anything.
pub struct Cores { /* … */ }

impl Cores {
    pub fn new(cores: &'static [&'static dyn Core]) -> Self;
    /// The cores this program can drive, in the order of the list.
    pub fn usable(&self) -> impl Iterator<Item = &'static dyn Core> + '_;
    /// The cores it cannot, each with "core NAME implements contract A.B;
    /// this program needs C.D".
    pub fn refused(&self) -> &[(&'static CoreDescription, String)];
    pub fn by_id(&self, id: &str) -> Option<&'static dyn Core>;
    /// One file-dialog filter per platform that opens content, then one
    /// for everything any core opens.
    pub fn filters(&self) -> Vec<Filter>;
}

/// A file-dialog filter: a name for people and the extensions it shows.
pub struct Filter {
    pub name: String,
    pub extensions: Vec<&'static str>,
}

/// Why a file did not become a machine to start: it could not be read,
/// no core opens it, a core recognised it and cannot run it, the named
/// core is not compiled in, or it needs content beyond itself. Each
/// carries the sentence the application shows.
pub enum LaunchError { /* … */ }

/// Where the front end keeps what machines started from files leave
/// behind. The application chooses the directories; this crate names no
/// host location.
pub struct HostDirs {
    /// Machines' records: `saves/CORE/aa/DIGEST/`.
    pub saves: PathBuf,
    /// Saved states: `states/CORE/aa/DIGEST/`.
    pub states: PathBuf,
}

/// Everything decided about one file, ready to start. `Send`: it is made
/// on a worker thread.
pub struct Prepared { /* the core, the platform and kind, the title, the
    notes, the config, the verified content, the storage directory */ }

pub enum Outcome {
    Ready(Prepared),
    /// More than one core runs the file, and nothing chose between them.
    Choose(Vec<Prepared>),
}

/// Open `path`, hash all of it, and ask the cores. `wanted` is a core id
/// the person named.
pub fn prepare(
    path: &Path,
    cores: &Cores,
    wanted: Option<&str>,
    dirs: &HostDirs,
) -> Result<Outcome, LaunchError>;

/// Build the machine. On the thread that will drive it: a machine need
/// not be `Send` (§3.5).
pub fn start(prepared: &Prepared) -> Result<Box<dyn Machine>, Error>;
```

1. **Open….** File › Open… shows the host's file dialog with the filters
   of `Cores::filters`: "NES cartridge images (*.nes)", …, "All supported
   files", "All files". The same path starts from the command line:
   `unemu --open FILE [--core ID]`, beside today's `--machine FILE`
   (§13.9 Q3).
2. **Hash, on a worker thread.** `prepare` opens the file once, computes
   the SHA-256 of all of it, and wraps it as a `ContentReader` of
   `unemu-core-files`, which refuses with `ErrorKind::Content` if the
   file changes afterwards (§12.5). The whole file is hashed whatever
   its size; a cartridge takes no time, a disc image takes what it takes.
3. **Choose the core.** With `wanted`, only that core is asked. Without,
   the cores whose content kinds list the file's extension are asked
   first, in the order of the list, then the others: an extension is a
   hint, and `identify` decides. Outcomes:
   - no core answers: "No core in this program opens FILE", with the
     kinds the cores do open; if a core answered `Unsupported`, its
     message is shown instead ("recognised, cannot run: …");
   - one core answers: `Outcome::Ready`;
   - several answer: `Outcome::Choose`. The application refuses with a
     message that names the cores and `--core`. A dialog that asks, and a
     remembered answer per kind, come when two compiled-in cores open one
     kind; until then no one can meet the case.
4. **Check.** `prepare` calls `validate` on the config that `identify`
   wrote and requires that it lists the file's digest and nothing else.
   Content that needs more than itself is refused with the labels of
   what is missing (§13.2, left out).
5. **Create, on the UI thread.** `start` builds the `CreateRequest`: the
   platform, the config, a `ContentSource` that answers the one digest
   with the verified reader, and a `Storage` over the directory
   `saves/CORE/aa/DIGEST/`, where `CORE` is the core's id, `DIGEST` the
   content's SHA-256 in hex and `aa` its first two digits. A save
   therefore follows the content, not the file's name or place. A second
   machine on the same content cannot share a record with the first:
   `Storage::open` answers `ErrorKind::Usage` for a record that is held
   (§12.5).
6. **The session.** `Session::new(machine, display, warnings)` over the
   boxed `dyn Machine`, on the machine's first display.
7. **The window.** The application holds the result as a
   `player_ui::Player` and shows it where the PC's screen is shown.

**Later: a library entry.** A stored image of a platform whose core lists
`config_types` starts through the same `start`, with a `Prepared` made
from the store instead of a file: the config from the image, the digests
from `validate`, a `ContentSource` over the store's receipts, a `Storage`
over the machine's workspace. That is the neutral start of §5, and it is
§13.4's last part, not this one. It fits a platform whose content is its
payload. The PC's stored config is not what its `create` reads: its
provider writes a launch document with flat digests, sizes and records,
and hands over a `Prepared` made from parts with its own content and
storage plans (§14.2.2, §14.2.4, §14.2.5) (corrected 2026-11-19, stage B
plan).

**Beside the PC's path.** The PC stays on its provider path until §4
stage B: its library, machine files, media dialogs and `Presenter`
(`Session<PcMachine>`) are untouched.

- The shell holds at most one running machine, as today: either the PC's
  `presenter` or a `player`. Starting one while the other runs asks to
  stop it first, with today's stop prompt. Open… while a machine runs
  first prepares the file (steps 2 to 4) and asks to stop only for a file
  that can start, so a refused file leaves the running machine alone.
- The shell's parts that meet a running machine ask whichever is there:
  the commands Pause, Stop, Reset, Power cycle, Mute, Speed, Fast
  forward, Release and Fullscreen; `tick`; the central panel; the title
  and status line; the routing of raw key and pointer events. While a
  player runs, the PC's Machine and Media menus are replaced by the
  player's, and the PC's commands (Insert…, Layers, Save as image…) are
  disabled.
- The library window does not list what was opened from a file. A list
  of recent files is not part of this design.
- When the PC reaches stage B it becomes one more core on this path, and
  `Presenter` keeps only what the contract does not carry. The contract's
  `Machine` has no downcast, so the desktop starts a stored PC image with
  `core_launch::start_with` and `PcCore::create_machine` and keeps a typed
  `PcMachine` in its `Presenter`; a stored PC image does not run in
  `player_ui::Player`, which would drop Insert…, Layers, Save as new
  image…, Settings and the drive chips (§14.2.6, §14.6 Q3; corrected
  2026-11-19, stage B plan).

**What a running machine offers**, each control enabled from the
machine's description and never by trying a call (§3.3):

| Control | From | Does |
|---|---|---|
| Pause, Resume | always | The front end stops calling `advance` and re-anchors its pacing on resume (§3.9), as the shell does today |
| Reset | `features.resets` lists `Soft` | `reset(Soft)`. A `Usage` answer (not possible in this status) is a status-line message |
| Power cycle | always | `reset(Hard)` when listed; otherwise `finish`, then `start` again from the same `Prepared` |
| Stop | always | `finish`, then back to the library |
| Mute, Speed, Fast forward, Fullscreen | always | As today (`Session`) |
| Save state, Load state | `features.saved_state` | Below |
| The Media menu | `slots` with `removable` | One Media menu with a section per removable slot, headed by the slot's label and what is in it: Eject, Next in catalogue and the catalogue's entries, from `slot_states()`; `change_medium` at the next boundary. A machine with no removable slot has no Media menu. Adding a medium from a file is not part of the first steps |
| Input… | a port of kind `Pad` | The Input dialog, below |
| Show overscan | the picture's `visible` is smaller than the picture | A view option, off by default: the screen shows the visible part |
| Pointer capture | a `RelativePointer` port | As today |
| Aiming | an `AbsolutePointer` port | Below |

**The picture.** The player sizes the screen from `Picture`'s aspect and
visible part: the shown rectangle has the shape
`aspect × (visible.width / width) ÷ (visible.height / height)`, as large
as the panel allows; with integer scaling on, the height is a whole
multiple of the visible rows. The literals 4:3 and `RASTER_SCALE` stay in
the PC's path.

**Sound.** `HostAudio` learns the machine's rate from the description and
converts from it to the host device's rate. The converter keeps its
method, linear interpolation with an exact position carried across
blocks, and takes the source rate `n/d` where it has the constant today:
per input frame it emits while `phase <= r·d` and adds `n`, then
subtracts `r·d`, with `r` the device's rate. For `n/d` = 48 000/1 that is
today's arithmetic, so the PC's sound is unchanged bit for bit. The
warning "audio disabled" remains only for a rate outside the contract's
bounds. A converter with a proper low-pass filter is a later quality
bead, judged by the sound-level tests; it changes nothing in the
contract.

**Pads from the keyboard.** A table in `machine-view`, front-end data:

```rust
/// Which host keys work which pad control, per player. Front-end data:
/// no core sees it.
pub struct PadBindings { /* per player: control → keys */ }

impl PadBindings {
    /// Player 1: the arrow keys; `X` is A and `Z` is B; `S` is X and `A`
    /// is Y; `Q` and `W` are L and R; Enter is Start and the right Shift
    /// is Select. No other player has keys.
    pub fn defaults() -> Self;
    /// The player and control a key works, if any.
    pub fn control(&self, key: Key) -> Option<(usize, PadControl)>;
    pub fn bind(&mut self, player: usize, control: PadControl, key: Key);
    pub fn unbind(&mut self, player: usize, control: PadControl);
    /// Every binding, for a dialog and for the preferences file.
    pub fn entries(&self) -> impl Iterator<Item = (usize, PadControl, Key)> + '_;
}
```

- Keys are the contract's `Key` (HID usages). `machine-view` gives each a
  name (`keys::name`, `keys::by_name`: the names of the contract's
  constants), and the application writes the bindings to the preferences
  file by those names, so the file does not depend on the toolkit and
  `machine-view` gains no dependency.
- The session gives a key to a pad when the machine has **no keyboard
  port** and the key is bound: it sends
  `InputEvent::Pad { port, control, pressed }` to the player's port and
  nothing else. Host repeats are dropped. A machine with a keyboard port
  gets its keys as today; a machine with both is decided when one
  exists.
- Host chords (mute, media keys, the release chord) are taken first, as
  today.
- The bindings are one table for all cores, under the key `pads` of the
  preferences file, which the application owns as it owns the other keys
  [REPO: [`crates/window/src/preferences.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/preferences.rs)].
- **The Input dialog**, minimal: one tab per pad port of the running
  machine, by the port's label; a row per control the port lists, with
  its keys; click a row and press a key to bind it (the key replaces the
  row's keys and leaves any other control that had it; while a row waits,
  Esc leaves it as it was and Delete clears it, so neither can be bound);
  "Defaults"; one hint line saying so. Nothing else.

**The absolute pointer.** For a machine with an `AbsolutePointer` port
the session does not capture the pointer. While the host pointer is over
the screen rectangle it sends `PointerPosition` with the place on the
**whole** picture (mapped back through the visible part when overscan is
hidden), and `None` when the pointer leaves the rectangle or the window
loses focus. The primary button is button 0. The cursor over the screen
is a cross-hair.

**Saved state.** One quick slot per content and core. Save state writes
`states/CORE/aa/DIGEST/quick.state`: a first line `unemu-state 1`, the
machine's `state_identity()` on the second line, then the core's bytes,
through a temporary file and a rename. Load state is enabled when the
file exists; it compares the identity line with the running machine's and
refuses another ("this state was saved by another release of the core, or
for another setup"), then calls `load_state`. Named slots, thumbnails and
states in the store (machine-images §2) are not part of this design.

**GUI first as mock-ups.** File › Open…, the player's menu and status
line, the Input dialog and the refusal messages are new parts of the
desktop. Their look is approved by the maintainer as mock-ups before any
of it is implemented (step 11 of §13.6). The crates without a toolkit do
not wait for that.

**Testing before a NES exists.** `unemu-test-core` is a core for tests.
It is in this workspace, depends on the contract alone, forbids unsafe
code, touches nothing of the host, and is never in the list of a release
build (§13.4).

- **Its content** is a text file whose first line is
  `unemu-test-core 1`, with the extension `utc`; the lines after it
  script the machine: the picture's size, aspect and visible part; the
  clock; the sound's rate; the input ports (pads, a keyboard, a relative
  or an absolute pointer); slots with a catalogue; the features (resets,
  saved state, frame stops); a battery record; and events at a virtual
  time (`halt`, `power-off`, `fail`, a message).
- **Its machine** is exact and trivially predictable. The picture is a
  pattern of the frame count, with a field whose colour is the set of
  held pad controls and a cross at the pointer's place, so a test reads
  pixels to see that an input arrived. The sound is a square wave whose
  frame count is exactly `now × rate / clock`, so a test counts frames
  and measures the tone after conversion. The battery record counts the
  machine's starts. A saved state is the counters.
- **Its `identify`** answers `None` for a file without the first line and
  `Unsupported` for a line `unsupported REASON`, so both refusals of step
  3 above have a test.

What is tested with it, and where:

| Crate | Tests |
|---|---|
| `machine-view` | Sound at 984 375/22 Hz and 44 100 Hz reaches the ring at the device's rate with the tone's frequency kept and no frame lost or invented over ten virtual seconds; a 48 000 Hz source gives the bytes it gives today. A bound key becomes a pad event and reaches no keyboard; an unbound one reaches nobody. The pointer's place maps to the whole picture with overscan hidden and shown, and to `None` outside. Reset by kind; a state saved, changed and loaded |
| `core-launch` | The order cores are asked in; each outcome of step 3; a file that changes after hashing is refused; a second machine on content whose record the first holds is refused; the record written in one session is read in the next |
| `player-ui` | Headless, as the shell's tests draw without a window: each control of the table above is present exactly when the description says so; the screen's rectangle for 128:105 with and without overscan; the Input dialog binds a key and the next press of it moves the test machine's field |
| `window` | With the test core in the shell's list: `--open` and File › Open… start a player; starting the PC while a player runs asks to stop it; Pause, Stop and Power cycle; the PC's own tests unchanged |

The scripted machine of `machine-view`'s session tests
[REPO: [`crates/machine-view/src/session/tests.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/machine-view/src/session/tests.rs)] stays for the tests
that script a machine's answers call by call; the test core is for tests
that need a whole core.

**The command line.** Nothing new runs a machine from the front end's
command line yet (§13.9 Q3):

- The desktop takes `--open FILE [--core ID]`, as it takes `--machine
  FILE`.
- A headless run of a core is the core's own command line (decision-41,
  item 8): `unemunes` for the NES, as `unemupc` for the PC.
- `--about` of both programs lists every compiled-in core with its id,
  release and contract (§12.9), from `Cores`.
- A neutral `run` of the front end comes with `image run` through the
  contract (§12.9), when a second platform has a stored image to run:
  one command for a file and for an image, designed once.

### 13.4. Registering cores

**The list.** `platforms` is the one place that names what is compiled in
(§7.1). It gains the cores:

```rust
/// Every core compiled into the programs, in the order they are asked
/// about a file and listed for people.
pub fn cores() -> &'static [&'static dyn Core] {
    static CORES: [&dyn Core; 1] = [&unemu_pc_core::PcCore];
    &CORES
}
```

- A core from another repository is one more line here and one pinned
  git dependency in the manifest of `platforms`, with the revision, as
  the contract is pinned: `&unemunes_core::NesCore` (the crate's name is
  unemunes's to choose).
- The test that every compiled-in core states a contract this program
  drives moves from [`crates/window/src/main.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/main.rs) to `platforms` and runs
  over `cores()`. It also asserts that core ids, platform ids and, per
  platform, content-kind ids are unique.
- Both binaries make one `core_launch::Cores` from the list and pass it
  down. No neutral crate calls `platforms` (§5).
- `unemu-test-core` is not in the list. The tests of `core-launch`,
  `player-ui` and `window` build their own list with it. For trying the
  generic path by hand before a NES core exists, `platforms` has a Cargo
  feature `test-core`, off by default and never set by the release
  workflow, that adds it.

**What the description feeds.**

| In the UI | From |
|---|---|
| The filters of File › Open… | each platform's `name` and its content kinds' `name` and `extensions` |
| Which core is asked about a file first | `extensions` |
| The window title of a running machine: TITLE, then the platform | `Identified::title` or the file's name; `PlatformDescription::name` |
| The About text: one line per core, then its notices | `CoreDescription::id`, `name`, `version`, `implements`, `notices`. The PC's two notices already come this way [REPO: `crates/pc-core/src/core.rs`] |
| A core shown as unusable, with the reason | `Cores::refused` |
| The machine's menu, the status line, the Input dialog | `MachineDescription`, as the table of §13.3 |

**Cores and store-side providers are two lists.** A core is the run-time
half of a platform: the contract. A provider is the store-side half:
`DesktopPlatform` and `CliPlatform`, which answer for artifacts
(validators, the push guard, format names, the platform's own commands).
The PC has both. A NES core has the first only, at first.

| | PC | NES at first |
|---|---|---|
| In `platforms::cores()` | yes; `create` unsupported until stage B | yes |
| Opens a file through §13.3 | no: it lists no content kind | yes |
| A `DesktopPlatform`, a `CliPlatform`, validators | yes (`pc-library`, `cli-pc`, `pc-image`) | none |
| In the library, in Remotes, in `image` commands | yes | no. A registry image that names the platform `nes` is listed as another platform's, as any foreign one is today, because `PlatformFacts::runs` reads the providers |

Nothing has to change in `DesktopPlatform` or `CliPlatform` for a core
without a provider: the neutral code that takes a list of them works with
the PC's alone. `PlatformFacts::family` ("this release runs …s") keeps
naming the first provider's platform.

**When NES images enter the store**, later and as a design of its own,
this is what is needed, and none of it is in the steps of §13.6:

1. **An artifact shape.** The machine image of a content platform: the
   config that `identify` writes as the image's config, the content as
   one payload blob, a media type for that payload, and the platform id
   in the manifest (machine-images §1). The platform id joins the alias
   table of `image_store::platform` (decision-32).
2. **A provider written once for every such core**, in a neutral crate:
   a `ConfigValidator` over `Core::validate` (its digest list is what
   `validate` returns) and a `DesktopPlatform` and `CliPlatform` over the
   core's description. This is what §3.12 left `Core::validate` out of
   0.1 for and what §6 predicted: a simple platform ships no provider
   crate. It fits a platform whose content is its payload, not the PC:
   the PC's store closure (layer descriptors, sector packs, source
   archives) and its core's content list (flat bases) are different
   lists, and neutral code must not take one for the other (§14.2.3;
   corrected 2026-11-19, stage B plan).
3. **An import** from a file to an image, with the mark that the content
   is the user's own, so that the push guard holds it back: cartridge
   images are almost never free to redistribute. Whether a core may
   name content that is (a list of open titles by digest) is decided
   there.
4. **The neutral start** of §5 and the `LibraryPlatform` sketch of
   §8.1.4, with `Storage` over the machine's workspace; the library's
   listing and details for a platform that has no settings. A platform's
   store side may need a step between the image and `create`, as the PC's
   does (§14.2.4); the generic one is designed here, later (corrected
   2026-11-19, stage B plan).
5. **Firmware for content that needs it** (a disk system): the
   `FirmwareRules` seam, or a generic one over `validate`'s labels.

### 13.5. The unemunes side

Not a design of the NES emulator (§1). Two things the two sides must
agree on.

**The rule for the 6502 crate.** The processor is a crate of unemunes's
workspace that knows nothing of a NES: no picture or sound chip, no
cartridge board, no memory map, and no dependency on another crate of
unemunes or on the contract. It reaches memory through a bus trait it
defines, one call per bus cycle, so that a machine can advance its other
chips cycle by cycle; the interrupt, reset and ready lines are inputs it
samples where the processor does. What differs between the chips of the
family (no decimal mode in the NES's processor, an I/O port in the
Commodore 64's, the CMOS instructions) is a parameter of the model, not
conditional compilation and not a fork. Its state is plain data a
machine can save. It has its own tests, against a published per-
instruction test set for the processor and not only through NES
programs, and its own documentation, which cites the processor's data
sheet as this repository's device crates do. It forbids unsafe code and
holds the rules of §12.1. Kept so, leaving for its own repository is a
move of one directory.

**The checklist for the adapter step.** The crate that implements the
contract over the NES machine:

1. **The dependency.** `unemu-core-api` by the exact revision of a
   release tag, the line of §13.1, never a path, a branch or a copy; a
   gitignored patch for local work. `implements` is the literal
   `ContractVersion::new(0, 3)`.
2. **The description.** Core id `unemunes`. One platform; its id is the
   one artifacts will name later, so it is chosen once (proposed: `nes`,
   with the region in the config). Content kinds with their extensions.
   A config type of its own. `features`: both resets, `frame_stops`,
   `saved_state` once it holds. Its notices.
3. **`identify` and `validate`** as §13.2 has them: `None` for foreign
   bytes, `Unsupported` with a reason for a board it does not model, a
   config that names the cartridge by digest and size.
4. **`create`.** Content only from the `ContentSource`, by the digest in
   the config. What outlives a run only in records of the `Storage`,
   under names fixed for good. No file, no path, no environment
   variable.
5. **The machine's description.** An exact clock (`Rate`), a step unit,
   one display, the sound's rate within the bounds of §13.2, one `Pad`
   port per controller in player order with exactly the controls it
   has, the cartridge as a slot that is not removable, with its digest.
6. **`run`.** Deadlines and step limits as §3.5; `Stop::Frame` when
   asked. `frame` returns the last finished picture, with `aspect` for
   the whole picture and `visible` for its safe part, and a serial that
   moves only when the pixels did.
7. **Input** takes effect at run boundaries. `ReleaseAll` releases every
   control.
8. **Sound.** The exact frame count of §13.2; division by a whole number
   inside the core when the generator is faster than the bound.
9. **Determinism.** No host clock, no random source (the memory a
   powered-on machine shows is fixed, or seeded from the config), no
   thread, no process-wide state; the replay rule of §3.11 as a test:
   the same request and calls give the same pictures and sound.
10. **Battery memory** by the rules of §13.2; `finish` makes it durable.
11. **Messages, not prints**; errors by kind.
12. **Embeddability** (§12.1): the adapter and what it links forbid
    unsafe code and add no host-touching crate.
13. **A test that the description is honest**: no call the description
    promises answers `Unsupported` (§3.3), driven as `&dyn Core` and
    `Box<dyn Machine>` with the contract's storage in memory.
14. **Its own command line**, `unemunes` (decision-41), a separate crate
    that may open files and uses `unemu-core-files`.
15. **The hand-over**: the revision to pin, the contract revision it was
    built against, and the licences of everything it links, for this
    repository's `deny.toml` and third-party notices.

### 13.6. Steps

Each step is one bead under the rules of §7.5: it lands alone with the
full gate, changes no behaviour unless its row says so, and comes with
tests that fail before it. A step "in the contract's repository" lands
there, under that repository's CI, and changes nothing here until a later
step moves the pin. Sizes are ASSUMED.

| # | Step | Needs | Behaviour | What proves it |
|---|---|---|---|---|
| 1 | **Done.** **Sound at any rate.** `HostAudio` takes the machine's rate; the converter takes `n/d`; `Session::new` refuses only a rate outside 8 000 to 192 000 Hz (§13.3). A test in `unemu-pc-core` that the PC holds the frame-count rule of §13.2. As landed: `HostAudio::for_source(rate)` (the session calls it with the described rate; `HostAudio::new` stays the 48 kHz source), `host_audio::plays` and `SOURCE_RATES` hold the bounds, and the refusal reads "the machine's sound runs at R; the window plays 8000 to 192000 Hz". The converter keeps its position in 128-bit integers, so any denominator stays exact. One addition: the device's rate is chosen as before but preferring the source's own rate when it is whole (48 000 for the PC, as before), so a 44 100 Hz machine on a 44 100 Hz device plays unconverted. MEASURED: the ring's frames for a 48 kHz source, a tone and full-scale noise at six device rates (16 000 to 192 000 Hz), equal FNV-1a digests recorded from the converter before the change; 984 375/22 Hz to 48 000 and 44 100 Hz, 44 100, 32 000 and 192 000 Hz give exactly `(N − 1)·r·d/n + 1` frames after every block, every frame within the interpolation's bound of the ideal tone, and the tone's period count within one, also through the session over ten virtual seconds; the PC hands over exactly the floor of `now() * rate / clock`, so it holds the frame-count rule, at deadlines a tick before, on and a tick after the edges of eleven frames out to frame 9 600 | nothing: contract 0.1 already carries the rate | None for the PC. A machine at another rate now sounds | The host-audio and sound-level tests pass unchanged; a new test compares the ring's bytes for a 48 000 Hz source with a recording made before the change; scripted machines at 984 375/22 Hz and 44 100 Hz give the device's rate, the tone's frequency and the exact frame count. If the PC's count is off by a frame or more, the rule of §13.2 is weakened before step 5 writes it |
| 2 | **Done.** **Pads from the keyboard.** `machine_view::pads` (`PadBindings`, the defaults), `keys::name` and `by_name`, and the session's routing (§13.3). As landed: players are counted from 0 in the API (player 0 is the first pad port); `PadBindings` also has `empty` and `Default` (the defaults); the session's table is set with `Session::set_pad_bindings`, which first releases every control a held key works, and which the application will call with its preferences (not part of this step); a reset of a halted machine forgets held keys. A bound key reaches a pad only if the player's port lists the control; a control bound to two keys is pressed with the first and released with the last; focus loss forgets held keys. A machine with both a keyboard and pads gets its keys on the keyboard, as today, until such a machine decides otherwise. `keys::name` covers the contract's 144 named usages, every one a key reaches a machine as among them | nothing: 0.1 has `InputEvent::Pad` | None for the PC: it has a keyboard port | A scripted machine with two pad ports and no keyboard gets pad events for bound keys and nothing for others; one with a keyboard gets today's key events for the same keys, and the existing key tests pass unchanged; every `Key` constant has a unique name that reads back |
| 3 | **Done.** **The contract's repository, at 0.1.** The extraction of §13.1; this repository pins `v0.1.0` and drops `crates/core-api`. As landed: the workspace dependency is the git pin of §13.1 at commit `4e2dd83` (`unemu-core-api` 0.1.0, `MIT OR Apache-2.0`), its `src/` equal to `crates/core-api/src` but for the SPDX lines (MEASURED); `deny.toml` allows the new git source; `crate-boundaries.sh` adds the one-copy rule over `cargo metadata --offline --locked` with fixtures of no copy, one copy and two copies, and keeps the contract layer's no-dependency rule, now with no workspace package to check; the third-party licence list names the crate with its licence texts. MEASURED: the workspace's test list is the same without the contract's seven unit tests | §13.9 Q1 answered | None | The contract's tests pass in its own CI under their names. Here: the full gate; the workspace's test list is the same without the contract's own tests; `cargo tree -i unemu-core-api` shows one source; a fixture with two copies fails the new rule of the boundary check; `cargo deny` accepts the new git source |
| 4 | **Storage objects**, in the contract's repository: the types of §12.5 with the storage in memory behind `test-support`. This is step 8 of §12.10, moved | 3 | None here | The proofs of §12.10 step 8; the contract builds for a host without an operating system |
| 5 | **The second platform's additions**, in the contract's repository: §13.2, with `CONTRACT` 0.2 | 4 | None here | Tests: `identify` answers `None` and `validate` answers `Unsupported` by default; a scripted core that implements both; `Frame::new` marks the whole picture visible; the pointer's fraction maps to the first and the last pixel for three picture sizes; `drives` refuses 0.1 |
| 6 | **`unemu-core-files`**, in the contract's repository (§13.9 Q4), then the tag `v0.2.0` | 5 | None here | A record's append, sync and truncate over files; a replacement committed, dropped, and failed both ways with a planted file-system seam; a record held by one storage is refused to a second, in the same process and in another; a reader refuses a file that changed; one scripted sequence over the storage in memory and over files leaves equal bytes; Linux, macOS and Windows in CI |
| 7 | **Done.** **This repository on 0.2.** The pin moves; `PcCore` states 0.2; `Picture` carries the visible part. As landed: the workspace pins `unemu-core-api` 0.2.0 at `56c4640` (tag `v0.2.0`), the one source in `Cargo.lock`; `deny.toml` names no commit and the third-party list is generated from the build, so neither changed. `PcCore` states `ContractVersion::new(0, 2)`; its platform lists no content kind (a PC starts only from a config, as §13.2 says of the PC of stage A) and keeps the default `identify` (`None`) and `validate` (`Unsupported`); `create` and `add_medium` answer `Unsupported` as before. The PC's frame comes from `Frame::new`, so its visible part is the whole picture; `machine_view::Picture` copies `Frame::visible` and nothing shows it yet (step 10). One addition in `machine-view`: a `StorageUncertain` failure of `drain_audio` ends the session as a `Storage` failure does instead of becoming an audio warning. The step-1 sound test now cites the 0.2 wording (the next frame's index is `floor(now() × rate / clock)`). MEASURED: the contract's source at the pinned commit and this workspace name none of `Content::File`, the session path of `CreateRequest` or the path of `WriteTarget::Kept`; `cargo tree -i unemu-core-api` shows one source, at `56c4640`; the PC core's description test and the window's test over every compiled-in core fail with the PC stating 0.1 | 6 | None: `PcCore::create` and `add_medium` still answer `Unsupported` | The full gate; the test over every compiled-in core; the contract's three path signatures are gone from what this workspace builds |
| 8 | **Done.** **Core registration.** `platforms::cores()`, the moved and widened test, `core_launch::Cores` with the About lines of both programs (§13.4). `core-launch` starts with `Cores` alone. As landed: `platforms::cores()` lists `unemu_pc_core::PcCore`, and with the `test-core` feature of `platforms` (off by default; no release build sets it) also `unemu_test_core::TestCore`. [`crates/core-launch`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/core-launch), layer `neutral` and in neither the window nor the CLI family, depends on the contract alone (`unemu-test-core` for its tests); `Cores::new(&'static [&'static dyn Core])` checks each core with `CONTRACT.drives(implements)` and gives `usable()` in the list's order, `refused()` with "core NAME implements contract A.B; this program needs C.D" (NAME is the core's `name`), `by_id` over the usable cores only, and `about_lines()`: one line per core in the list's order, `core ID RELEASE (NAME), contract A.B`, a refused core's line ending with `, unusable: ` and the reason. `Cores::filters` and `Filter` are left for step 12, with the rest of the launch path. `upcli` and `unemupc` each make one `Cores` from `platforms::cores()` in `main` and pass it down (`upcli::command::run_program` takes it); their `--version` and `--about` print the core lines after the program's own lines (after the first two for `upcli`, after the first for `unemupc`); the core's notices stay where they were, and the desktop's About dialog and `--help` are unchanged. The test over every compiled-in core moved from [`crates/window/src/main.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/main.rs) to `platforms`, runs over `cores()` through `Cores` (no core refused), and also checks that core ids, platform ids (across cores) and each platform's content-kind ids are unique, with planted lists that repeat each. New pins: [`crates/upcli/tests/pinned/about.txt`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/upcli/tests/pinned/about.txt) and `version_and_about_are_pinned` in the desktop, with the release, the licence notice and the third-party notices replaced by names. `scripts/crate-boundaries.sh` gains two fixtures: the CLI's neutral crate and a window crate both using `core-launch` pass, `core-launch` reaching `platforms` fails. MEASURED: the `--version` and `--about` output of both binaries built before and after differs by exactly the one core line, `--help` and the usage text not at all; each new test fails on a mutation of the code it covers (no core refused, the reason dropped from the line, `by_id` finding a refused core, `usable` unfiltered, no About lines, the PC listed twice, content-kind ids compared across platforms, platform ids not compared, the PC stating 0.1), and the pass fixture fails with `core-launch` put in the window family. Not yet: the desktop's About dialog lists no cores and the notices still come from the window's own constants, not from each core's description; the dialog follows with the player (step 13 or 14) | 7 | **Stated:** `--about` of both programs lists the compiled-in cores; its pins change | The pins change only by the core lines; a core that states another contract is listed as unusable with the sentence of §3.2, in a test with a scripted core; ids are unique |
| 9 | **Done.** **The test core.** `unemu-test-core` (§13.3). As landed: [`crates/test-core`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/test-core), layer `neutral`, its only dependency the contract (`test-support` for its tests); `TestCore` has the core id `unemu-test-core`, one platform `unemu-test` with the config type `application/vnd.unemu.test-core.config`, every feature, and the content kind `script` (`utc`). The script's grammar is in the crate's documentation: after the first line `unemu-test-core 1`, one keyword per line (`title`, `notice`, `unsupported`, `picture`, `aspect`, `visible`, `clock`, `step`, `frame`, `sound RATE TONE`, `keyboard`, `pointer`, `absolute`, `pad`, `slot`, `medium`, `insert`, `resets`, `saved-state`, `frame-stops`, `media-changes`, `battery`, `echo-input`, `at TICKS halt\|power-off\|guest-reset\|fail\|fail-content\|note\|warning`); a bad line is refused with `script line N: …`, as `Unsupported` from `identify` (a file it recognises and cannot run; `Content` from `identify` would mean the bytes changed after hashing) and as `Content` from `create`. `identify` writes a config naming the script by digest and size, `validate` lists that one piece of content. The machine runs in whole steps and jumps to the next limit, frame edge or event; the picture is drawn at each frame edge (overscan magenta, the background the frame count since the last start, the field the held pad controls, a cross per pointer; `unemu_test_core::picture` gives a test the colours and the field's place); the sound is a square wave whose next frame is exactly `floor(now × rate / clock)` (`unemu_test_core::sound`). Battery memory is the record `battery`, counting creations and hard resets, written by `replace` and `commit` at `finish` and at the end of a run ten virtual seconds after the last write; a medium added as `Kept` has its record opened when it is inserted. Additions beyond the step: `echo-input` (each event reaching the machine is a note, for routing tests), `guest-reset` (a reset the guest does inside a run, reported as a message) and `fail-content` (a run that fails with `Content`, as when content changed under the machine). A step is at most 2⁵⁰/1000 ticks and a frame at most 2⁵⁰; a deadline or step budget past the end of virtual time is `Usage` and leaves the machine as it was. `scripts/crate-boundaries.sh` gains the rule that the test core declares no dependency but the contract, with a failing and a passing fixture. MEASURED: 39 tests, each group failing on a mutation of the code it covers (a process-wide serial, a frame stop refused, any header version recognised, the `unsupported` line ignored, a config naming another size, the record not read, a nine-second write, the picture not restored, the frame index rounded up, undrained frames kept across sound off and on and across `load_state`, a failed battery write or kept-record open not failing the run, a refusal without its line, an uncapped step, `Content` from `identify` for a bad script, `fail-content` failing as `Guest`, limits past the end of time not refused, a serial moved by a load that changed no pixel or by a pad the field does not show, a halted machine that cannot finish), and the boundary self-test fails without the new rule | 7 | None: it is in no binary | Its own tests: the replay rule; no promised call answers `Unsupported`; the three answers of `identify`; a record written by one machine is read by the next, over the storage in memory; a state saved, changed and loaded; the exact frame count of the sound. The boundary check passes with its layer `neutral` and no dependency but the contract |
| 10 | **Done.** **`machine-view` on 0.2.** The absolute pointer, the visible part in the screen's geometry (a function the views call), reset by kind, saving and loading a state. As landed: `machine_view::screen::layout(&Picture, panel, show_overscan, integer_scaling)` gives a `ScreenLayout`, the shown `source` area of the picture and the `dest` rectangle in the panel's logical points (`screen::Rect`, no toolkit type), or `None` for an empty picture, panel or aspect; a visible part outside the picture counts as none, and with integer scaling a panel lower than the shown rows shows them smaller. `ScreenLayout::pointer_position(x, y)` is its inverse: the centre of the pixel under the point as a place on the whole picture, so `pointer_pixel` gives that pixel back for pictures up to 32 768 pixels a side, and `None` off the rectangle. The session aims (`Session::aims`, `Session::pointer_at`, `pointer::Aiming`) at an absolute pointer on its own display and never captures for it: a place is sent only when it changes; the host's left, right and middle buttons are 0, 1 and 2 up to the port's count, through `mouse_button` wherever the pointer is, since a light gun also fires off the picture; focus loss sends `ReleaseAll`, then `PointerPosition` with `None`. `Session::resets` and `Session::reset(kind, elapsed, diagnostics)`: queued media changes first, the machine's messages to `diagnostics`, its refusal returned unchanged; a halted machine runs again as its status says, paced from `elapsed` so its wait is not caught up; held input stays held as the machine keeps it, and a hard reset tells the absolute pointer the host pointer's place again. `Session::saves_state`, `state_identity`, `save_state` and `load_state(state, elapsed)`: a refusal (`State` for another identity or a damaged state) is returned and leaves the session as it was; loaded, pacing is anchored again at `elapsed`, the sound the session had queued is dropped with the converter's history (`HostAudio::discard_pending`), the screen takes the loaded picture at once, the machine is told `ReleaseAll` so the next press of a held key is fresh (the relative pointer's buttons count as released), the absolute pointer is told the host pointer's place again, and the session runs or waits halted as the loaded status says (any other status stops it), releasing a captured pointer when it does not run. Reset, save and load answer `Usage` once the session has stopped. The PC's screen keeps its 4:3 and `RASTER_SCALE` path and no PC test or pin changed. `unemu-test-core` and the contract's `test-support` are dev-dependencies of `machine-view`. MEASURED: 15 new tests, 7 of the geometry, 7 of the session over the test core and one over the scripted machine that a reset puts queued media in first (sound at 984 375/22 and 44 100 Hz over ten virtual seconds, frame for frame against the ideal tone converted in one go; a bound and an unbound key; the pointer with overscan hidden and shown and off the screen; resets by kind, counted in the battery record; a state saved, changed and loaded; a refused state and the status a load restores; the gun after a load or a hard reset points where the host pointer rests); each fails on a mutation of the code it covers (integer scaling, the overscan flag, the crop in the shape, the source offset and the open edge of the inverse, a zero aspect, aim events not forwarded or not deduplicated, no `None` at focus loss, the reset's kind, a halted machine not restarted or not paced from the reset, media put in after the reset, a stopped session reset, the place not told again after a load or a hard reset, queued sound kept, pacing not anchored, the picture not taken, no release, the status not taken, a refusal swallowed, a frame lost per drain, a drifting converter, a bound key reaching no pad, keys sent to a keyboard the machine lacks); the existing 48 000 Hz byte test stands | 9 | None for the PC | The `machine-view` row of §13.3's test table, against the test core |
| 11 | **Done.** **The look, as mock-ups**: File › Open…, the player's menu and status line, the Input dialog, the refusals. Not a code bead: the maintainer approves it. As landed: [`mockups/player.html`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/mockups/player.html) revision 1, approved by the maintainer on 2026-11-18 with the page's recommendations: one Media menu with a section per removable slot (the controls table above), no toolbar buttons for Save state and Load state (Machine menu and Ctrl+Alt+S, Ctrl+Alt+L), Open… takes Ctrl+O and Open machine file… goes without a key, Open… while a machine runs checks the file before the stop prompt, and the Input dialog's Esc, Delete and replace rules. Also taken from the page: Open… sits in File above Open machine file… and its dialog opens on "All supported files"; disabled, it says "No core in this release opens files" by its hover text; the title is content — platform — unemupc; controls a description does not offer are absent, not greyed (Load state alone is greyed until a state exists); Reset takes Ctrl+Alt+End; the first two slots take Ctrl+Alt+E/F and the same with Shift; the toolbar keeps the PC's buttons greyed and its Ctrl+Alt+Del button reads Reset | nothing; best early | None | The maintainer's approval, recorded on the bead |
| 12 | **Done.** **`core-launch`**: `prepare`, `start`, `HostDirs` (§13.3). As landed: `core-launch` depends on `unemu-core-api`, `unemu-core-files` (a workspace git dependency on the contract's repository at the same revision `56c4640`, so `Cargo.lock` keeps one copy of the contract) and `sha2`; the boundary check needed no change, as it layers workspace crates only. `Cores::filters()` gives one `Filter { name, extensions }` per platform that opens content, in the list's order, named after its one kind (`NES cartridge images (*.nes)`) or `PLATFORM: KIND, KIND (*.a;*.b)` for several, the same line once when two cores give it, then `All supported files (…)` when any core opens something, then `All files (*.*)` with the extension `*`; a kind that fails `ContentKind::is_valid` is left out of the filters, of the extension match and of the refusals. `prepare(path, cores, wanted, dirs)` refuses a named core before it reads the file (one compiled in that states another contract with the sentence of §3.2, any other as unknown), opens the file once, wraps it as `FileContent` and hashes all of it through that reader in 1 MiB reads, so a change while it hashes is refused too; asks the named core alone, or the cores whose kinds list the extension (in lower case) first, then the others, each in the list's order; takes `Content` from `identify` as the file changed and `Storage` as a failed read, both ending the open; checks that the platform and the content kind answered are the core's, that the core's id can name one directory (1 to 64 bytes of ASCII letters, digits, `-`, `_` and `.`, not starting with a dot) and that `validate` lists the file's digest at its size and nothing else (other pieces are `NeedsMore` with their labels; a list without the file is a core failure). A core that fails, in `identify` or in the check, is left out, so one broken core does not keep another from opening the file; with no core left the refusal is the first failed check, else the first core's `Unsupported` message, else the first failure in `identify`, else "no core opens it". `Outcome::Ready(Prepared)` or `Outcome::Choose(Vec<Prepared>)`, as the sketch has it, each choice checked; `Outcome::into_ready` is the application's refusal of `Choose`. `Prepared` (`Send`, asserted at compile time) gives the core, platform and its name, kind, file name, title (the content's or the file's name), notes, config, digest, size, `saves_dir()` and `states_dir()` (`ROOT/CORE/aa/DIGEST`); preparing writes nothing. `start` opens `FileStorage` over `saves_dir()` (created when missing), behind a thin `Storage` that notes when it refused a valid record name with `Usage` (its lock is held, here or in another program), and calls `create` with a `ContentSource` that serves the one digest and size as a reader sharing the verified file and answers `Content` for anything else; a `Usage` failure of `create` after such a refusal comes back as `Usage` with the mock-up's sentence, every other error unchanged, so a core's own `Usage` (a name the storage does not take, a read past a record's end) keeps its message. `LaunchError` (`Read`, `Changed`, `NoCore`, `NotOpenedBy`, `Unsupported`, `UnknownCore`, `Unusable`, `Core`, `NeedsMore`, `Several`) shows the mock-up's sentences: "No core in this program opens FILE. This program opens: KIND (*.ext); ….", "The core NAME recognised FILE and cannot run it: REASON", "More than one core in this program runs FILE: NAME (ID), …. Name one on the command line: --open FILE --core ID.", "FILE needs more than itself to run. Missing: LABEL, …. Open… hands a machine the one file and nothing else.", and for the held record "FILE is already running in another machine, here or in another program, and its saved data is in use. Stop that machine first."; the cases the mock-up does not draw read "The core NAME does not open FILE. It opens: ….", "This program has no core ID. Its cores: ….", "core NAME implements contract A.B; this program needs C.D" for a named core of another contract, "FILE could not be read: REASON.", "FILE changed while it was being opened. Open it again." and "The core NAME failed on FILE: REASON". Display is written by hand, as the crate's only dependencies are the three above. MEASURED: 26 new tests over the test core and scripted cores (the order cores are asked in, with and without a name, a refused core never asked and named with its reason; no core, with what the cores open; a core's `Unsupported` message, and a core that runs the file winning over it; Ready with its title, notes and directories; Choose and its refusal; needs more; the config naming the file at another size or not at all; a file that cannot be read; a file larger than two reads hashed whole; a file changed after hashing refused with `Content` at `start`, also when only its modification time changed; `Content` and `Storage` from `identify` ending the open; a failing core left out, in `identify` and in the check, and which failure is told when none is left; a kind the platform does not list; a core id that cannot name one directory; a core's own `Usage` from `create` (a bad record name, a read past the end) kept; the second machine on the same content refused with `Usage` while the first holds the record; the record under `saves/unemu-test-core/aa/DIGEST/`, and read by the next machine, also from a copy under another name; the one-file content source; the filters), each failing on a mutation of the code it covers (37 mutants: extension order, case, the name ignored, `Unsupported` dropped or winning, `Choose` collapsed, the kinds of the refusal, a partial hash, the missing and the absent file and the size not checked, the `aa` level, the storage in the states directory or per file name, the held sentence, a change swallowed, any digest served, filters undeduplicated, All supported always or never, the file check, the title fallback, the named core's kinds, the held flag set for any `Usage` or not consulted, a refused named core called unknown, a failing core ending the open in `identify` or in the check, failed checks or `identify` failures forgotten, `Content` or `Storage` from `identify` taken for a core failure, the kind and the id not checked, a leading dot or any length allowed). A change while the file is hashed is refused by the same reader, but no test reaches it deterministically without a seam between two reads | 6, 8, 9 | None: nothing calls it yet | The `core-launch` row of the test table |
| 13 | **Done.** **`player-ui`**: the player (§13.3). As landed: [`crates/player-ui`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/player-ui), layer `neutral` and a window crate in `scripts/crate-boundaries.sh` (a new fixture, the player depending on a CLI crate, fails the check; MEASURED: without the family entry the self-test reports it passing); it depends on `unemu-core-api`, `machine-view`, `core-launch`, `ui-kit`, `egui` and `winit` (the session's key, button and window types, already in the build through `machine-view`), and tests with `unemu-test-core`, `machine-view/test-support` and `ui-kit/test-support`. The API the application calls in step 14: `Player::start(Prepared, PadBindings, warnings)` (the machine from `core_launch::start`, a `Session` on its first display, the bindings set), `open_audio`, `set_window`, `set_look(classic, macos_shortcuts)`, `elapsed`, `title(program)` (CONTENT — PLATFORM — PROGRAM), `offers()` (`Offers`: reset, power cycle by a hard reset, the removable slots, input, show overscan, pointer capture, aiming), `tick(ctx, blocked, elapsed, warnings)`, `command(Command, elapsed, warnings) -> Option<Request>`, `menus(ui, ViewChrome, commands)` (inside the application's menu bar), `screen(ui)` (the central panel), `status(ui)`, `dialogs(ctx)`, the raw events `key_event`, `guest_key` (past the session's chords, for the keys taken from macOS's system shortcuts), `mouse_button`, `mouse_motion`, `pointer_inside`, `focus_changed`, `release_pointer`, `recapture_pointer` (after a cancelled stop prompt; releases always reach the machine, also while a dialog shows), and `screen_rect`, `pad_bindings`, `has_dialog`, `paused` (the user's pause only), `running` (the machine advances when nothing holds it), `selected_speed`, `message`, `prepared`. `Command` is the player's own vocabulary (Pause, Reset, PowerCycle, Stop, Input, Eject, Next and Insert of a slot, ScaleToWindow, IntegerScale, ShowOverscan, Fullscreen, Mute, Speed, FastForward(held), Release); `Request` is what the application does after one: `Stopped { failure }` (drop the player and show the library; a failure to finish is the application's to show, a guest failure was already shown in the player's box), `Fullscreen`, `PadBindings` (store the new keys). `MessageBox::refusal(title, text)` is the mock-up's box (the refused command as title, the refusal mark, one OK), which the application can show for Open's refusals too. The look, as the mock-up: the Machine menu (a note naming the content and running, paused or stopped; Ⅱ Pause or Resume Ctrl+Alt+P; Reset Ctrl+Alt+End when a soft reset is listed; Power cycle; ■ Stop Ctrl+Alt+Q; Input… for a pad port); one Media menu with a section per removable slot, headed by LABEL — MEDIUM (or empty): ⏏ Eject, Next in catalogue (Ctrl+Alt+E and F for the first slot, with Shift for the second), then the catalogue's entries with ● on the one in the slot, an empty slot's Eject and an empty catalogue's Next greyed with their reasons, no Media menu without a removable slot; View: Toolbar and Status bar (the application's, lent through `ViewChrome`), Scale to window and Integer scale marked ●, Show overscan marked ✓ only while the picture is cropped, Full screen, Refresh remote greyed, Mute audio, Speed with ● and the greyed Hold for Unlimited, Skin, Theme; macOS writes the chords with ⌘ as the shell does; the status line: the message (a captured pointer says how to release it), a light with Running (lit), Paused, Halted or Stopped, the word the Machine menu's note uses too, a chip per removable slot (LABEL: MEDIUM, lit for 200 ms after the slot was busy), the platform, the content's title, sound, speed; the screen: black panel, `screen::layout` with overscan hidden by default and integer scaling, nearest sampling, PAUSED over a paused picture, a cross-hair over the picture and the pointer's place for an absolute pointer, a hidden cursor while a relative pointer is captured; the Input dialog: a tab per pad port by its label, a row per control the port lists with its keys by readable names (`key_label`: Arrow Up, Shift Right, 1), click a row and press a key that the application routes raw (so the right Shift can be bound): the key replaces the row's keys and leaves the control that had it, Esc leaves the row, Delete clears it, Defaults restores every player, the hint line, OK, Cancel; nothing changes until OK. Behaviour: Power cycle is `reset(Hard)` when listed and the machine runs or waits halted, otherwise the machine finishes, is dropped (its records let go) and starts again from the same `Prepared`, keeping the speed (also across a failed start), the user's mute (put on once the new machine's sound plays, read with `HostAudio::user_muted`, new in `machine-view`) and the window; both paths resume; a refused reset is a status-line message; only the user pauses: a halted, switched-off or failed machine is not paused but says so in the status line, Pause is greyed and does nothing, and PAUSED shows only over the user's pause; the slots' states are read every tick, also while the machine does not advance; the application's Eject, Next and Insert follow the Media menu's checks; the dialogs do not close on a click outside; a guest failure shows a Machine box, a host failure a Machine error box, a failed restart a Power cycle box; a command the description does not offer does nothing. Not here: Save state and Load state (step 15; until then the Machine menu has neither), and the toolbar, the title strip, the hotkeys, File › Open… and the routing of raw events, which are the application's (step 14). The ui-kit icon inventory moves the player mock-up's Ⅱ ■ ⏏ ● ✓ 🔊 🔇 to the text the player writes, ▸ to egui's submenu arrow and ✕ to the refusal mark it paints. MEASURED: 26 tests over the test core, headless; each fails on a mutation of the code it covers (35 mutants of the first round: Reset, Input or Show overscan always offered, fixed slots in the Media menu, the Reset item unguarded, Reset as a hard reset, Power cycle never or softly resetting, the user's pause ignored, the media action replaced, no light hold, integer scale or overscan ignored, no aim, no cross-hair, a key added instead of replacing, Esc or Delete bound, the raw Esc also cancelling, OK not reaching the session, Defaults emptying, the failure box or Stop without the error, a halt not pausing, the title and the status cells out of order, digits by their constant's name, the macOS Reset chord, no Media menu drawn, the Pause mark, speed and mute not reaching the session, Show overscan on a whole picture, Input opened without a pad, an empty slot's Eject enabled; 19 of the review's fixes: no slot refresh while the machine does not advance, a stop pausing the player, Pause unguarded, PAUSED over a stopped picture, Halted read as Stopped, the halted and switched-off messages swapped, key or button releases or guest_key presses held back wrongly under a dialog, the mute or the speed not carried across a restart, the reset path not resuming, a failed restart without its message, the hotkeys past the menu's checks, recapture while paused or ignored, a reset of a halted machine that never advances again (found by these tests and fixed), a click outside closing the box) | 10, 11, 12 | None: nothing shows it yet | The `player-ui` row of the test table; the boundary check counts it as a window crate |
| 14 | **Done.** **Open… in the application.** `--open FILE [--core ID]`, File › Open…, the shell holds a player beside the PC's presenter, the `test-core` feature of `platforms`, the two host directories. As landed: the usage line reads `usage: unemupc [--machine FILE] [--open FILE [--core ID]] [--display CARD] [--rom-dir DIR]... [--allow-unlisted-firmware]`, the only change to `--help`, now pinned whole (`help_is_pinned`); `--core` without `--open` ("--core names the core for --open FILE") and `--open` with `--machine` ("--machine and --open each start a machine; give one") are usage errors. `pc_ui::commands::Command::Open` is the shell's one new command: File › Open… above Open machine file…, Ctrl+O (⌘O on macOS) moves to it and Open machine file… goes without a key; the keyboard-shortcuts help says so. Disabled, it says "No core in this release opens files" by its hover text, from the shell's `disabled_reason` as every greyed item; it is enabled when a usable core lists a valid content kind. The host's file dialog (`ui_kit::file_dialog`, rfd) gets `Cores::filters` with "All supported files" moved first, because the host dialog opens on its first filter, then each platform's, then "All files". `core_launch::prepare` runs on a worker thread that wakes the window; a later Open… replaces a file still being prepared. A refusal, `Outcome::Choose` refused with `Outcome::into_ready`'s "several cores" sentence, a host without a data directory or a machine that fails to start shows `player_ui::MessageBox::refusal("Open", …)` and leaves a running machine alone; a file that can start asks the stop prompt (`StopPrompt::OpenFile`, the prepared file held until the answer) when a machine runs, as `Library` and Open machine file… ask while a player runs, the prompt naming the content's title. `HostDirs` are `unemupc/saves` and `unemupc/states` in the platform data directory, beside `unemupc/machines`; `core_launch::start` creates a machine's directory under `saves` on its first start, and `states` waits for step 15. The shell holds `Option<player_ui::Player>` beside `Option<Presenter>`, one at a time: `state`, `elapsed`, `title` (content — platform — unemupc), the central panel (`Player::screen`, a click gives the machine the keyboard), the status bar (`Player::status`), `tick` (held by the shell's dialogs, a menu, a hidden or minimised window), the stop prompt, the raw keys (to `Player::key_event`, or `guest_key` on macOS as for the PC; a fresh press goes raw to the player's Input dialog while it shows), the pointer's motion, buttons and place, focus, capture and the cursor go to whichever runs. While a player runs its Machine, Media and View menus stand where the PC's do (`Player::menus` with the shell's toolbar, status bar and skin lent through `ViewChrome`), the hotkeys and toolbar's Pause, Stop, Reset, Power cycle, Mute, Speed, Hold for Unlimited, Release and the media keys of the first two removable slots reach the player (`for_player`), Full screen stays the shell's, the PC's commands (Start, Insert…, Create from files…, the catalogue, Layers, Save as new image…, Settings…) are greyed, and the toolbar keeps every button in place with A:, B:, Eject, Settings and Layers greyed and its Ctrl+Alt+Del button reading Reset; the player's `Request::Stopped` and Stop end in the shell's `stop`, whose failure is shown in the Machine error dialog as for the PC. Pad keys come from the preferences' `pads` (an array of `{ player, control, key }`, controls `up` … `select`, keys by `machine_view::keys::name`, unknown names skipped, absent for the defaults) and the Input dialog's OK writes them back. Tests play no sound (`open_audio` is not called under `cfg(test)`). `window` depends on `player-ui` and tests with `unemu-test-core`. The PC's shell tests are unchanged except where the decisions move them: the rows that expected Ctrl+O to give Open machine file… now expect Open (in `shell_model` and the macOS chord test, where Open with the release's cores does nothing), the `Args` literal of one test names the two new fields, and two matches over `StopPrompt` gained the unreachable new variant. A file prepared while the stop prompt shows or an exit is under way is dropped, never slipped under the answer; Stop and Exit forget a file still being prepared and the status line's "Opening FILE…" gives its message back; starting a player abandons a machine file's Start still loading, a library Start that arrives while a player runs is closed and kept (`finish_with_failure(true, false)`) with a status-line message, and `started` finishes a PC instead of showing it beside a player. A `[[pads]]` entry with a field left out is skipped. MEASURED: 15 new tests (`dash_open_and_file_open_start_a_player`, `open_is_greyed_and_says_why_while_no_compiled_in_core_opens_files`, `a_player_and_the_pc_ask_to_stop_each_other_with_the_stop_prompt`, `pause_reset_power_cycle_and_stop_reach_the_players_machine`, `a_refused_file_shows_its_box_and_leaves_a_running_machine_alone`, `pad_keys_come_from_the_preferences_and_the_input_dialog_stores_them`, `pad_keys_are_stored_by_name_and_the_defaults_need_no_entry`, `help_is_pinned`, `open_takes_a_file_and_names_a_core_only_with_it`, `files_opened_keep_their_saves_and_states_in_the_data_directory`, `a_file_prepared_late_never_replaces_a_question_or_an_exit`, `a_file_started_while_a_pc_start_is_under_way_stays_the_only_machine`, `host_keys_reach_the_players_pad_and_its_input_dialog_raw`, `a_click_on_the_screen_works_an_aimed_players_trigger`, `a_shell_dialog_holds_the_players_machine`), the shell's over the test core; each fails on a mutation of the code it covers (33 mutants: `--open` ignored at start, the dialog's answer dropped, the filters not opening on All supported, Open machine file… keeping Ctrl+O, Ctrl+O still Open machine file…, no hover reason, Open never enabled, the stop prompt blind to a player, Open… starting without the prompt, a refused file stopping the running machine first, Choose taken as Ready, Pause, Reset or Power cycle not reaching the player, Stop dropping the machine unfinished, the toolbar keeping Ctrl+Alt+Del, the PC's menus kept, pad keys not read from or not written to the preferences, `pads` not a key the shell writes, the old usage line, `--core` alone accepted, states under saves, a late prepare replacing the stop prompt, Stop keeping the file being prepared, no Opening message, a loading Start kept, a PC started beside a player, the Input dialog missing raw keys, keys or buttons not reaching the player, a shell dialog not holding it, a `[[pads]]` entry without a field refusing the file) | 13 | **Stated:** one new option and one new menu item. With the release's list, which has no core that opens files yet, the item is disabled and says so | The `window` row of the test table; the usage text's pin changes by one line; the PC's shell tests pass unchanged |
| 15 | **Done.** **Saved state in the player**: the quick slot (§13.3). As landed: `player_ui::Command::SaveState` and `LoadState`, and `Offers::saved_state` from the description's `features.saved_state`. The Machine menu has, after Stop and a separator, Save state (Ctrl+Alt+S) and Load state (Ctrl+Alt+L), written ⌘S and ⌘L on macOS; both absent for a machine that does not list saved state, both greyed with "The machine has stopped" when the machine neither runs nor waits halted, and Load state greyed with "No state saved for this content yet" until the slot's file exists (looked for when the player starts, and set by a save); no toolbar buttons. Save state writes `Prepared::states_dir()/quick.state` (`states/CORE/aa/DIGEST/quick.state`): the line `unemu-state 1`, the machine's `state_identity()` on the second line, then the bytes of the session's `save_state`, streamed into `quick.state.partial` in the same directory, synced and renamed over the slot; a failure removes the partial file and leaves the slot as it was, and shows a box titled Save state, "The state was not saved: REASON.", with "The state saved before is kept." when there is one; an identity that is not one line, or none, is refused the same way; the status line says "State saved" or "State not saved". Load state reads the two lines through a buffered reader, at most 4 KiB together, and hands the same reader on to the core; it refuses a file whose first line is not `unemu-state 1` or whose identity line has no end within that limit ("the file is damaged, or it is not a saved state this program reads"), compares the identity line with the running machine's before the core sees a byte ("this state was saved by another release of the core, or for another setup"), then calls the session's `load_state`, whose own refusal is shown in its words; each refusal is a box titled Load state, "The state was not loaded: REASON." and "The machine runs on as it was.", with "State not loaded" in the status line, and leaves the machine and the file as they were; loaded, the status line says "State loaded"; a machine the state left running is woken for its next paint; one it left halted is not paused. A failure of the session's `load_state` other than the core's `State` refusal may come after the core took the state, so it ends the run as a host failure (the Machine error box) instead of claiming the machine runs on as it was. The box uses the refusal mark: ui-kit has no warning mark yet, and the mock-up's "!" waits for one. The shell: `pc_ui::commands::Command::LoadState` beside `SaveState`, both disabled for the PC (`Enablement`); Ctrl+Alt+S and Ctrl+Alt+L (⌘S, ⌘L) are hotkeys only while a player whose machine saves state runs and reach it through `for_player`, so for the PC and any other machine the chords stay guest input; the keyboard-shortcuts help lists them; the PC's Machine menu keeps its greyed "Save state (not yet available)". MEASURED: 6 new tests (`save_state_run_on_and_load_state_bring_the_saved_picture_back`, `a_state_of_another_identity_or_a_damaged_file_is_refused_and_the_machine_runs_on`, `a_state_saved_while_running_runs_again_after_a_halt_and_one_saved_halted_halts_unpaused`, `a_save_that_fails_keeps_the_state_saved_before`, `a_machine_without_saved_state_has_neither_item_and_saves_nothing` in `player-ui` over the test core, and `save_and_load_state_chords_reach_a_player_that_saves_state_and_the_pc_s_guest` in `window`), two hotkey tables extended; each fails on a mutation of the code it covers (22 mutants: the identity line not compared, a cut-short identity line accepted, the header read unbounded, no wake after a load, the pause kept by a load that halts, the slot written in place, no identity line written, a damaged file handed to the core, Load state never greyed, the items always offered, the slot not looked for at start, a stopped machine saved, a save not enabling Load, no "kept" sentence, no "runs on" line, Save doing nothing, the chord consumed for the PC, Save or Load not routed to the player, no Ctrl+Alt+S hotkey, the shell refusing both) | 14 | **Stated:** Save state and Load state work for a machine that lists `saved_state`; the PC's stay disabled | With the test core: save, run on, load, and the picture is the saved one; a state with another identity is refused with its message |
| 16 | **Done.** **The NES core, compiled in.** One line in `platforms::cores()`, the pinned revision, the git source in `deny.toml`, its notices and licences in the third-party list. As landed: the workspace pins `unemu-nes-core = { version = "0.0.1", git = "https://github.com/unemu-org/unemunes", rev = "7c5abb5270edae8d6df1ceadb5dddd5091d08fef" }`, built by unemunes against the contract's v0.2.0 commit `56c46401`, so the lock gains `unemu-nes-core`, `nes` and `x65` (0.0.1, MIT OR Apache-2.0, no other dependency) and `scripts/crate-boundaries.sh` still finds one copy of `unemu-core-api`; `deny.toml` allows `https://github.com/unemu-org/unemunes` beside x86jit and the contract and nothing else. `platforms::cores()` lists `PcCore`, then `NesCore` (then the test core with its feature); the core's line in both programs' `--version` and `--about` reads `core unemunes 0.0.1 (unemunes), contract 0.2`, after the PC's, and their pins change by that line only. With the release's cores File › Open… is enabled and the host's dialog offers "All supported files (*.nes)", "NES cartridge images (*.nes)", "All files (*.*)"; the shell's test that Open… is greyed with "No core in this release opens files" now makes its list of the PC alone, and the macOS chord test expects ⌘O to open the dialog. `scripts/third-party-licenses.sh` looked for a git dependency's licence files one level above the crate, which misses a workspace of several crates (`crates/NAME`); it now walks up to the checkout's root (marked `.cargo-ok`, at most three levels), so `nes`, `unemu-nes-core` and `x65` list their repository's own `LICENSE-MIT` and `LICENSE-APACHE` with its copyright line instead of the standard texts; the output is generated and gitignored, and the existing MIT block only moves. The golden, [`crates/platforms/tests/nes_cartridge.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/platforms/tests/nes_cartridge.rs), copies the checkerboard cartridge of unemunes' frame golden (MIT OR Apache-2.0, said in its header), writes it to a temporary directory, opens it through `core_launch::prepare` over the release's cores (one answer: core `unemunes`, platform `nes`, kind `ines`) and `start` with temporary `HostDirs`, runs to 80 000 retired instructions with frame stops and pins 8 frames, the backdrop in frames 0 and 1, a partial frame 2 and frames 3 to 7 at FNV-1a `0x64CB5BF7C2F11325`, the digest unemunes pins; `NES_REV` names the revision and is checked against `Cargo.lock`, as `CPU_REV` is. MEASURED: with `NesCore` taken out of the list, the golden (`NoCore`), the list test, both `--about` pins, the Open… test and the macOS chord test fail; a changed `NES_REV` or digest fails its test | 14, and milestone N5 below | **Stated:** the program opens NES cartridge images | A test builds a small cartridge image from bytes of our own (a header and a program that loops), opens it through `core-launch`, runs a fixed number of frames and compares the frame count and the picture's digest, both pinned for the pinned revision of unemunes, as goldens are pinned to x86jit's; the one-copy rule; the test over every compiled-in core |

Steps 1 and 2 can start today. Steps 4 to 6 are the only ones that change
the contract; they are finished before any core but the PC's and the
test core states a version, which is when a change is cheapest.

**Milestones of unemunes**, and what each needs from here:

| # | Milestone | Needs from this repository |
|---|---|---|
| N1 | The processor crate under the rule of §13.5, with its own tests | nothing |
| N2 | The picture and sound chips, the cartridge boards and the machine, with unemunes's own goldens; no contract | nothing |
| N3 | The adapter, by the checklist of §13.5 | step 3 to read the contract where it will live; the tag of step 6 to pin |
| N4 | The command line `unemunes` | the tag of step 6, for `unemu-core-files` |
| N5 | The first revision for the front end, with the hand-over of item 15 | nothing more; step 16 here follows it |
| N6 | Later, each on its own: the light gun, saved state, a disk system, the PAL machine | the light gun needs nothing new; a disk system's firmware needs §13.4's store work |

Dependencies between the two:

- unemunes waits for this repository twice: for the contract's
  repository (step 3) before N3 begins, and for `v0.2.0` (step 6) before
  N3 can pin. N1 and N2 wait for nothing.
- This repository waits for unemunes once: step 16 needs N5. Steps 1 to
  15 are tested with the test core.
- A gap the adapter finds in 0.2 goes to the supervisor as a request, and
  becomes 0.3 by the protocol of §13.1. It is cheap until step 16 has
  landed, because until then only `PcCore` and the test core re-state
  their version.

### 13.7. Where the steps fit

**With §12.10.**

- Step 8 of §12.10 (contract 0.2) is steps 4 and 5 here: made in the
  contract's repository, and wider by §13.2.
- Step 11 of §12.10 planned to create `unemu-core-files` out of
  `media`'s file code. The crate now exists first, small and new (step 6
  here), because the launch path needs a file-backed storage long before
  the journal moves. Steps 9 and 11 of §12.10 then move the journal's
  file half **onto** it, and extend it where the journal needs more (the
  lock beside a journal, opens that do not follow links). The lock keeps
  the journal's name `FILE.lock`, which `unemu-core-files` learns for the
  journal's records (§14.6 Q8; corrected 2026-11-19, stage B plan). Until then the
  two do not meet: the PC's journals keep their own file code.
- Step 1 of §12.10 and step 3 here both edit
  `scripts/crate-boundaries.sh` and its fixtures; they go one after the
  other, in either order.
- Step 4 of §12.10 (the desktop binary becomes `unemu`) and step 14 here
  both edit strings in [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs), [`app_menu.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/app_menu.rs) and `main.rs`; one after
  the other. The option is `--open` under either name.
- Steps 9 to 12 of §12.10 and §4 stage B are not needed by anything
  here. When stage B lands, the PC lists a config type and becomes one
  more core on the path of §13.3.

**With the rest of the window split (§8.1, moves 3.11 to 6.5).**

- Steps 1, 2 and 10 edit `machine-view` only. No remaining move touches
  that crate, so they run beside the split.
- Steps 8, 9, 12 and 13 add crates. Each adds its line to the boundary
  check (and `player-ui` to its window family), as moves 4.1, 5.2 and
  6.2 do; a bead that adds a crate and a move that adds one go one after
  the other.
- Step 14 is the one that meets the split: it edits [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs),
  `main.rs`, [`app_menu.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/app_menu.rs) and the shell's command vocabulary. Move 6.3
  takes that vocabulary to `pc_ui::commands`; the one new shell command
  (open a file) is a variant of it wherever it lives then. The player's
  own commands are `player-ui`'s and never enter it. Step 14 goes
  between two moves of step 6, not beside one (§13.9 Q2).
- `upc-dfs.25.7` (a crate for `Presenter` and its tests) and this design
  do not meet: the player is not a `Presenter`.

**People.** Two workers can hold steps 1 and 2, and steps 3 to 7, at the
same time as the window split. The contract's repository is worked from
this repository's supervisor, like any other step.

### 13.8. Edits to earlier sections

This design changes the text above in these places, and nowhere else:

1. **The status paragraph at the top** names §13.
2. **§3.12**, the bullet on `Core::validate`: one sentence that §13.2
   brings it in with contract 0.2.
3. **§12.5**, first paragraph: 0.2 also carries §13.2 and is made in the
   contract's repository. Move 3: `unemu-core-files` exists before the
   split; the journal's file half moves onto it.
4. **§12.8**: `unemu-core-files` is named as a crate of the contract's
   repository.
5. **§12.10**, the rows of steps 8 and 11, to match 3.
6. **§12.11**: step 8 is steps 4 and 5 of §13.6.

### 13.9. Questions answered by the maintainer

Answered on 2026-11-18, each with the recommendation: Q1 the contract is
licensed `MIT OR Apache-2.0` and its repository is public under
unemu-org; Q2 the generic launch path goes now, beside the window split;
Q3 no front-end command to run content yet (the desktop's `--open`, the
cores' own command lines); Q4 `unemu-core-files` is a second crate of the
contract's repository. The options are kept for the record.

**Q1. The contract's licence**, to be answered before its repository is
public.

- (a) **`MIT OR Apache-2.0`**, as x86jit. Any core and any front end may
  link the contract, whatever their own licence. The first commit after
  the export changes the SPDX lines and adds the two licence texts; the
  export's earlier commit stays as it was published in this repository.
- (b) **AGPL-3.0-or-later**, as today. Nothing to relicense. Every core
  that implements the contract and every program that drives one is then
  bound by the AGPL as a whole, so a third party can embed a core only in
  an AGPL program.
- (c) **A weak copyleft** (MPL-2.0, or the LGPL): changes to the contract
  stay open, linking is free. One more licence for `deny.toml` and for
  everyone who reads the notices.

Recommendation: **(a)**. Decision-41 wants cores that anyone can
integrate, and the contract is what integrating means. It holds
interfaces, not emulation; the family's other embeddable library already
carries this licence; this repository's licence policy already allows
it. It decides nothing about unemupc's own licence or unemunes's.
`unemu-core-files` and `unemu-test-core` follow the same reasoning: the
first takes the contract's licence, the second stays with the front end.

Also to say with the answer: whether the supervisor creates the
repository public, or private for the maintainer to open. It must be
anonymously fetchable before step 3 lands here, because CI fetches it.

**Q2. Does the generic launch path wait for the window split?**

- (a) **It goes now.** Steps 1 to 13 are in `machine-view`, in new crates
  and in the contract's repository, none of which a remaining move of the
  split touches. Step 14 alone edits the shell, and is placed between two
  moves.
- (b) **It waits** until move 6.5 has left only the application in
  `window`. No bead is ordered against a move; unemunes's adapter then
  meets a front end that cannot show it for as long as the split takes.

Recommendation: **(a)**. The split never moves [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs): it stays in
the application at the end (§8.1.9, move 6.5), so waiting removes no
conflict from step 14, only postpones it. Everything before step 14 is
out of the split's way by construction.

**Q3. Does the front end's command line run content?**

- (a) **Not yet.** The desktop takes `--open FILE [--core ID]`. A
  headless run of a core is the core's own command line. A neutral `run`
  comes with `image run` through the contract (§12.9), designed once for
  a file and for a stored image.
- (b) **Now**: `run CONTENT [--core ID] [--max-steps N]` in `cli-core`,
  over `core-launch`. It would print the stop and the messages; what else
  a headless run of any core should print (a picture's digest, the
  sound's) has no answer yet.
- (c) **Now, for tests only**, hidden from the usage text.

Recommendation: **(a)**. The command would be a second headless runner
beside each core's own, with nothing to print that a person wants, and
its words would be fixed before the store-side run they must share them
with exists. The window's tests and `core-launch`'s cover the path.

**Q4. Where does `unemu-core-files` live?**

- (a) **In the contract's repository**, as its second crate. Cores'
  command lines and the front end take it from one pinned source, and one
  revision covers the contract and its file objects.
- (b) **In the front end** (this workspace, layer `neutral`). The
  contract's repository stays one crate with no host code. Each core's
  command line then writes its own file-backed storage or keeps its
  records in memory and copies them to files itself.
- (c) **In a repository of its own.** A third pin to move together.

Recommendation: **(a)**. §12.8 already lets a core-side crate depend on
it, which only works if it does not live in the front end. Its cost is
that the contract's repository has one crate with operating-system code,
tested on three systems; the contract crate itself still has no
dependency and no host code, which its CI checks.

**What this design assumes of the open questions of §12.13**, until they
are answered:

| §12.13 | Assumed here | If answered otherwise |
|---|---|---|
| Q1, `unemu` one binary or two | (a) one: the desktop's options start with `--`, so `--open` fits | With two binaries `--open` is the window binary's option; nothing else changes |
| Q2, the raw run commands | nothing depends on it | |
| Q3, a libretro wrapper | (b), after 1.0. `identify`, frame stops and one record per machine are what such a wrapper needs for a NES core | |
| Q4, the journal's home now | (a). Under (b), `unemu-core-files` is still made in step 6, since the launch path needs it, and the journal joins it at extraction | Steps 9 to 11 of §12.10 wait; nothing here does |
| Q5, the host directories | The two new directories, `saves` and `states`, are created beside the store, in the front end's `unemu` data directory, under either answer | |

Decision-41's names (`unemu`, and a core's binary named as the core) are
taken as decided.

## 14. Stage B in steps

Status: **landed**, with the retained live helpers listed in S14 and §4.
Accepted by the maintainer on 2026-11-19, with the
recommended answer to each question of §14.6. This section records the design
of §4's stage B: the PC config codec below the contract, `PcCore::create`
as the one start path for stored images, and both front ends starting
stored PC images through `core_launch`. The plan was read on `main` at
`f586f5f2` (markers as at the top; **[CONTRACT]** = read in
`unemu-core-api` 0.2.0 at its pinned commit `56c4640`). No code was built
or run for that design review. The beads `upc-dfs.28` to `upc-dfs.40` are §12.10's steps
1 to 7 and 9 to 14; the steps here are named S1 to S14 so that they are
never taken for those.

Three findings shape the whole plan; the corrections they need in the
sections above are made in place, each marked "(corrected 2026-11-19,
stage B plan)":

- **The stored PC config is not a document `create` can read as it is.**
  The contract opens content by SHA-256 **and size**, by the digest of the
  flat bytes [CONTRACT: `ContentSource::open(sha256, size)`]. The stored
  config names a firmware blob without its size, and a layered or blank
  disk by the digest of its head descriptor, not of the disk's bytes
  [REPO: `crates/pc-image/src/lib.rs:161-200`, `disk.rs:119-139`]. What a
  start adds locally (journal retention, pending inserts, created disks,
  kept catalogues) is not in it either. The PC provider therefore writes
  a **launch document** (§14.2.2).
- **[`disk.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/disk.rs) and [`zero.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/zero.rs) stay in `pc-image`.** They walk the manifest's
  layer chain, build files in the store's cache and describe the zero
  base: store work, which §12.5 already places behind the `ContentReader`
  the front end hands over.
- **A plain `FileStorage` over the workspace cannot hold the PC's
  journals.** Several journal file names are longer than a record name may
  be, some sit in a sub-folder or beside a host file outside the
  workspace, and the journal's lock file has another name than
  `FileStorage`'s (§14.2.4, §14.6 Q8).

### 14.1. Today's start path for a stored PC image

Every start of a stored image is the PC provider's. It ends in
`assembly::assemble_with_verified_bases` and, on the desktop,
`PcMachine::from_assembled`.

| # | Step | Where [REPO] | After stage B |
|---|---|---|---|
| 1 | **Session.** Refuse a pending discard or publication; read or write `session.toml` (the pinned image, the config digest, journal retention) | [`crates/pc-image/src/workspace.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/workspace.rs) `prepare_selected` | Unchanged, in `pc-image` |
| 2 | **Resolve and verify.** `Store::resolve_descriptor` hashes the closure and calls the provider's `validate` and `validate_resolved` | [`crates/pc-image/src/provider.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/provider.rs) `load_pinned` | Unchanged; the two callbacks call the moved codec |
| 3 | **Decode and lower: the codec.** `prepare` checks the envelope and every binding, reads firmware and CMOS bytes, lowers the hardware tables with `machine_file::lower`, builds the catalogues | `provider.rs` `prepare`; the model in `lib.rs` | Below the contract, in the new crate `pc-config` |
| 4 | **Bases.** One `media::VerifiedBase` per medium: a raw blob, a compressed floppy in memory, a layered head flattened into the cache, a zero-rooted chain | [`disk.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/disk.rs) `bases`, [`zero.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/zero.rs) | Unchanged, in `pc-image`: the store's side of a `ContentSource` |
| 5 | **Paths into the machine config**: base paths, journal paths `media/<id>.upfj`, `media/cmos.upfj`, the retention | `provider.rs` `resolve_pinned` | Gone. The provider writes the launch document; `PcCore::create` lowers it and mounts through `assembly`'s mount seam (§12.10 step 10) |
| 6 | **A machine's extras**: a CMOS reset, pending inserts | [`crates/pc-image/src/machine.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/machine.rs) `prepare`, `insert_medium` | Edit the launch document's catalogues |
| 7 | **The desktop's extras**: created disks, a kept catalogue | [`crates/pc-media/src/created_media.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-media/src/created_media.rs) `apply`, [`kept_catalogue.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-media/src/kept_catalogue.rs) `restore` | Edit the launch document's catalogues |
| 8 | **Across the thread**: bases as file handles or bytes | `workspace.rs` `SessionTransfer` | A `core_launch::Prepared` made from parts |
| 9, 10 | **Assemble and wrap** | `crates/assembly/src/assemble.rs` `assemble_with_verified_bases`; `crates/pc-core/src/machine.rs` `PcMachine::from_assembled` | `core_launch::start` calls `PcCore::create`; the desktop keeps a typed `PcMachine` (§14.2.6) |

The callers are `upcli image run` and `image machine run`
([`crates/cli-pc/src/image.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/src/image.rs), [`image/machine.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/src/image/machine.rs); their run loop is
`run_session(RunPolicy::natural(max_steps, 1_000_000))` and
`pc_harness::finish_persistence`), and the desktop's Start of an image, of
a machine and of a Gaming-mode copy ([`crates/pc-library/src/library_worker.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/library_worker.rs)
on the worker, [`crates/window/src/shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs) on the UI thread). After the
start the desktop keeps a `Presenter` (`Session<PcMachine>`) and reaches
past the contract through `PcMachine::assembled` in about 90 places
(COUNTED by `grep` in `main.rs` and [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs)). A start from a machine
file is not this path and stays as it is (§3.4: a provider matter). Open…
of a content file is the generic path of §13.3 and is not touched.

### 14.2. The target

#### 14.2.1. The codec's home: the crate `pc-config`

A new crate `crates/pc-config` (package `pc-config`), layer **`emulation`**,
on the core's side (§14.6 Q2). An `emulation` crate may depend only on
`contract` and `emulation` crates, so `pc-config` depends on `assembly`,
`unemu-core-api`, `serde`, `toml`, `sha2` and `thiserror`, and on nothing
of the store, of `media` or of `machine-file`. `unemu-pc-core` cannot call
`machine-file` (layer `pc`), so the machine schema and its lowering move
too: this is the crate move behind §12.10's "the part of `machine-file` the
codec needs … becomes part of what an embedder links".

| Module of `pc-config` | From | Holds |
|---|---|---|
| `schema::v1`, `lower`, `parse_toml`, `ParseError`, `LowerError`, `Origins` (TOML half), `ImageReader`, `pc_names` | `machine-file` | The machine schema and its lowering to `assembly::MachineConfig`. `machine-file` re-exports every name where it was |
| `image` | `pc-image` `lib.rs`, `provider.rs` | The stored config's model (`Config`, `Firmware`, `Medium`, `Slot`, `Nvram`, `Policy`, `Geometry`), its decode and encode, the binding checks |
| `image::encode`, `image::defaults` | `pc-image` `encode.rs`, `defaults.rs` | A machine file's hardware tables with the lowering's defaults frozen in |
| `launch` | new | The launch document, its check without content and its lowering over a `ContentSource` |

What stays: **`machine-file`** keeps what touches the host (the YAML
reader, `extends` and presets, journal path checks, firmware discovery and
the firmware-package lookup, `load*`, `LoadedMachine`, `FileImageReader`).
**`pc-image`** keeps [`disk.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/disk.rs) and [`zero.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/zero.rs) whole, `load_pinned`, the
payload-type table, the session, machines, import, publication,
templates, recipes and the compatibility rows.

Four places of the lowering touch the host today and go behind
`ImageReader` before it can move [REPO: `crates/machine-file/src/lower.rs`]:
two `std::fs::metadata` calls that size a ROM file; `paths::is_retention_input`,
which canonicalises a path; `paths::validate_journals`, which resolves the
journal and input paths on the host; and `ImageReader::verify_journal_base`,
which returns `media::VerifiedBase` although the lowering reads only its
length and digest (corrected 2026-11-19, S1: the plan said three and missed
`validate_journals`).

#### 14.2.2. The launch document

The one config type the PC platform lists:
`application/vnd.unemu.pc.launch.v1+toml`. A portable prepared launch may be
stored as a machine config. It contains document-owned media and deterministic
`<medium>.upfj` record names, with no journal bytes or session storage handles.
`pc-config::launch::encode_portable` and `decode_portable` check this subset;
session-kept record names are refused there. Session-augmented launches remain
transient and use the ordinary launch codec. Front-end stores dispatch on the
config media type and retain the launch's required runtime content; they must
not feed a prepared launch to the old stored-image Config decoder. The launch
uses the stored config's schema plus two tables the PC provider adds
(§14.6 Q1). The format as landed with S5 (corrected 2026-11-20, S5; its
encoding is pinned by `pc-config`'s test
`a_document_round_trips_byte_for_byte_and_its_encoding_is_pinned`):

```toml
schema = 1
platform = "ibm-pc"          # the current spelling; aliases are the store's

[machine]                    # the hardware tables, after the machine's settings overlay

# [[firmware]], [[media]], [[slots]], [nvram]: top-level, as in the stored
# config; the catalogues as this start has them, pending inserts and created
# disks included

[[content]]                  # one row per `blob` and `head` named above
digest = "sha256:…"          # that blob or head
sha256 = "…"                 # SHA-256 of the flat bytes the core opens: 64 lowercase hex digits
size = 2088960               # their length

[[record]]                   # one row per journalled medium, and one for a journalled CMOS
medium = "c"                 # a medium id, or "cmos"
name = "c.upfj"              # the record of the machine's storage
keep_generations = 10
```

- **`content`**: for a raw blob `sha256` is the blob's own digest; for a
  layered head the chain's `result_sha256`; for a zero base the digest of
  that many zero bytes. The provider knows all three today
  (`disk::identity`, `ZeroBase::identity`).
- **`record`**: the name and the retention, which today come from
  `session.toml` and the import record.
- A config's own media use the record `<id>.upfj` and the CMOS
  `cmos.upfj`, today's file names, so a session written before stage B is
  found after it. A medium id that starts with `-` or with the front end's
  prefix `kept-` is not a record name [CONTRACT: `is_record_name`,
  `KEPT_RECORD_PREFIX`]; the provider refuses to start such an image with a
  sentence saying so (ASSUMED: no image has one; S6 measures it).
- Media the provider adds for one start get records under `kept-…`, which
  its storage maps to their files (§14.2.4).
- **Rules of the tables** (as landed, S5). No two `content` rows name one
  digest, every digest the config binds (firmware blob, medium head, CMOS
  head) has a row and every row is bound. A firmware row is at most 16 MiB
  and a CMOS row exactly 64 bytes, before anything is opened, since the
  core reads both whole; a medium's row needs no limit of its own, because
  its length must equal the medium's geometry and assembly holds that to
  what the drive takes (ATA CHS 16383/16/63, at most 8,455,200,768 bytes,
  or a medium of the floppy drive's type). There is one `record` row per
  journalled medium and per journalled CMOS and no other; its name is a
  record name: `<medium>.upfj` or a kept record name. No two rows share a
  name, and each keeps at least one generation. The document is at most 2 MiB, twice the
  store's limit of a config blob. There is no separate numeric row-count
  limit: `content` has exactly the distinct digests of the firmware,
  media and CMOS bindings, and `record` exactly the journalled media plus
  a journalled CMOS. Every row and binding fits within the document limit.

`pc-config`'s API over it (as landed, S5):

```rust
pub mod launch {
    pub const MEDIA_TYPE: &str = "application/vnd.unemu.pc.launch.v1+toml";
    pub const PLATFORM: &str = "ibm-pc";
    pub const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
    pub const MAX_FIRMWARE_BYTES: u64 = 16 * 1024 * 1024;
    pub const CMOS_BYTES: u64 = 64;
    pub const CMOS_MEDIUM: &str = "cmos";
    pub const RECORD_SUFFIX: &str = ".upfj";

    pub struct Launch { pub config: image::Config, pub content: Vec<ContentRow>, pub records: Vec<RecordRow> }
    pub struct ContentRow { pub digest: image::Digest, pub sha256: [u8; 32], pub size: u64 }
    pub struct RecordRow { pub medium: String, pub name: String, pub keep_generations: u32 }

    pub fn decode(bytes: &[u8]) -> Result<Launch, image::Error>;
    pub fn encode(launch: &Launch) -> Result<Vec<u8>, image::Error>;

    /// Everything that needs no content: the tables, the envelope, every
    /// binding, the slots, each size against its geometry, the lowering in
    /// its metadata mode, `MachineConfig::validate_logical`, the option-ROM
    /// catalogue. Answers every `content` row, one per distinct SHA-256 and
    /// length: what `lower` reads and the bases it names. This is
    /// `Core::validate`.
    pub fn check(launch: &Launch, options: AssemblyOptions) -> Result<Vec<ContentRef>, image::Error>;

    /// The machine. Reads firmware and CMOS through `content`; names each
    /// disk base and each journal by a key and says what each key is.
    pub fn lower(launch: &Launch, content: &dyn ContentSource, options: AssemblyOptions)
        -> Result<Lowered, image::Error>;

    pub struct Lowered {
        pub machine: assembly::MachineConfig,
        /// Key of an `ImageSource::File` or an NVRAM base: its digest and size.
        pub bases: BTreeMap<PathBuf, ([u8; 32], u64)>,
        /// Key of a `WriteMode::Journal` or the NVRAM journal: its record.
        pub records: BTreeMap<PathBuf, String>,
    }
}
```

`MachineConfig` keeps its paths, as §12.5 move 2 decides; the keys are
names the codec makes up and nothing opens them: a medium's base is its
logical asset name (`assets/<id>.img`), its journal `<id>.upfj`, the CMOS
image of a journalled CMOS its logical asset name and its journal
`cmos.upfj`. A journalled CMOS lowers to `NvramSource::Journal` over its
image, with the retention of its record row; each journalled medium keeps
the retention of its row (corrected 2026-11-20, S5).

`check` and `lower` run the same checks in the same order (the doubled
content rows; the config's shape and bindings; the content table against
the bindings; the record table; the lowering), so they refuse a document
with the same words, except where only the bytes of content decide: the
firmware and CMOS pins, the system BIOS's and option ROMs' catalogue
membership, an option ROM's header and checksum, the services of a ROM
known only by its bytes. Those are `lower`'s alone,
so `Core::validate` can pass a document whose `create` then refuses its
content. Both refuse with `pc_config::image::Error`, in `pc-image`'s words
(`invalid PC image: …`), so that the store's refusals keep their words and
`needs_firmware`; `PcCore` maps that error to the contract's (S9)
(corrected 2026-11-20, S5: the plan had `lower` answer
`unemu_core_api::Error`).

**The store validates with the same function.** `PcProvider::validate`
keeps its store half (for each digest the config names: the manifest's
descriptor, its kind, its size limit, the firmware's source link, the
layer chain; so it fills the `content` rows and the closure it returns),
then calls `launch::lower` with a `ContentSource` over `BlobSource` that
serves the small content (firmware, CMOS). Disk content is never opened in
validation, as today. Two things the codec takes as arguments instead of
reading them: the retired platform spelling `ibm-pc-at` is checked by
`pc-image` before the call (the launch document always says `ibm-pc`), and
the permission for unlisted firmware is an `AssemblyOptions` argument
(the tests' thread-local stays in `pc-image`; no thread-local enters the
core).

#### 14.2.3. `PcCore::create` and `create_machine`

```rust
impl PcCore {
    /// Build a PC from a launch document. The typed form of `Core::create`,
    /// for PC code that needs more than the contract.
    pub fn create_machine(&self, request: &CreateRequest<'_>) -> Result<PcMachine, Error>;
}
impl Core for PcCore {
    fn create(&self, request: &CreateRequest<'_>) -> Result<Box<dyn Machine>, Error> {
        self.create_machine(request).map(|machine| Box::new(machine) as Box<dyn Machine>)
    }
    fn validate(&self, platform: &str, config: ConfigDocument<'_>) -> Result<Validated, Error>;
}
```

1. Refuse another platform or config type (`Unsupported`) and an unknown
   option (`Usage`).
2. `launch::decode`, then `launch::lower` with `request.content`.
3. Assemble through the mount seam of §12.10 step 10, with a mount that
   answers a base key with `request.content.open(sha256, size)` and a
   record key with `request.storage.open(name)`. The machine keeps the
   storage for its life, because a catalogue's journal opens when its
   medium is inserted.
4. One text scanout route per display card of the config, in a fixed
   order (MDA, then VGA); the front end picks the display.
5. `PcMachine` owns the lowered config and offers it as
   `PcMachine::config()`, for the `Presenter`'s catalogue model.

The description gains `config_types = [pc_config::launch::MEDIA_TYPE]`
and two options, `allow-unlisted-firmware` and `speaker-trace`; it lists
no content kind, so `identify` stays `None`. `create` asks its
`ContentSource`, each by digest and size, for the firmware ROMs (read
whole, at most 16 MiB), the CMOS image (64 bytes) and each floppy's and
fixed disk's flat base (a reader), which is exactly what `validate` lists.
It never asks for a layer descriptor, a sector pack or a source archive:
those are in the store's closure (`ConfigValidator::validate`) and not in
`Core::validate`'s list. For the PC the two lists differ, and neutral code
must not take one for the other. Storage records are `<id>.upfj` per
journalled medium, `cmos.upfj` and the `kept-…` names of the launch
document; a record's bytes are the version-1 journal, unchanged.
`PcMachine::from_assembled` stays for machine files and tests;
`add_medium` stays `Unsupported` (the media dialogs keep
`PcMachine::assembled`, §12.10 "not steps of this plan"). `create` hashes
each ROM again in the lowering (the pin check, the catalogue's
identification); the contract's "does not hash again" is meant for disks,
and that is said in the code.

#### 14.2.4. What `pc-image` keeps, and its module `launch`

- **The session and the verified bases**, as today.
- **A `Launch` instead of paths.** `resolve_pinned` builds a
  `pc_config::launch::Launch` from the effective config and the bases;
  `insert_medium`, `created_media::apply` and `kept_catalogue::restore`
  edit its tables. A catalogue is then data: a list of medium ids per
  slot, as the stored config already has it.
- **A content plan**, `Send`: the firmware and CMOS receipts and the
  verified bases as file handles or bytes (`SessionTransfer`'s content
  today). On the machine's thread it opens as a `ContentSource` that
  answers each `(sha256, size)` of the `content` table and refuses any
  other.
- **A storage plan**, `Send`: a table from record name to journal file,
  over the journal's file-backed record of §12.10 step 9, with the
  journal's own lock `FILE.lock` (§14.6 Q8). On the machine's thread it
  opens as the machine's `Storage`. It is the provider's own and not
  `unemu_core_files::FileStorage` over the workspace: the workspace holds
  `media/<64 hex>.upfj`, `media/<64 hex>-<n>.upfj`, `media/<64 hex>-<id>.upfj`,
  files under `media/created/` and journals beside a host image outside
  the workspace, none of which a record name (at most 64 bytes, no
  directory) can be.
- **`pc_image::launch::prepare…`** returns a `core_launch::Prepared` made
  from those parts, and the pinned image's digest.

#### 14.2.5. `core-launch`: a `Prepared` from parts

`core-launch` stays free of the store (it is under `player-ui`), so the
store's side hands it parts:

```rust
/// Content for one start, made on a worker and opened on the machine's thread.
pub trait ContentPlan: Send { fn open(&self) -> Result<Box<dyn ContentSource>, Error>; }
/// The machine's storage, likewise.
pub trait StoragePlan: Send { fn open(&self) -> Result<Rc<dyn Storage>, Error>; }

pub struct Parts { /* core, platform, title, notes, config, content plan,
                      storage plan, options */ }

impl Prepared {
    pub fn from_parts(parts: Parts) -> Prepared;
    /// Run-only options the application adds before the start.
    pub fn with_options(self, options: Vec<(String, String)>) -> Prepared;
}

/// `start`, with the caller making the machine from the request, so that a
/// platform's own code can keep its machine's type.
pub fn start_with<M>(
    prepared: &Prepared,
    create: impl FnOnce(&CreateRequest<'_>) -> Result<M, Error>,
) -> Result<M, Error>;

impl Cores {
    /// The usable core that provides `platform`, in its current spelling.
    pub fn for_platform(&self, platform: &str) -> Option<&'static dyn Core>;
}
```

`start(prepared)` becomes `start_with(prepared, |request| core.create(request))`.

#### 14.2.6. The desktop

The library's Start goes the way Open… goes: a `core_launch::Prepared`
from the worker, `core_launch` on the UI thread. The worker sends it where
it sends a `SessionTransfer` today (three places in `library_worker.rs`);
the PC's own events beside it (`StartCatalogue`, the inserted list, the
notes) stay. The shell adds the run-only options from its arguments, then
calls `core_launch::start_with(&prepared, |request| PcCore.create_machine(request))`
and `Presenter::from_machine(machine, display, warnings)`, choosing the
display from `machine.config().devices` as today.

The contract's `Machine` has no downcast [CONTRACT], and `Presenter`
uses `PcMachine` in about 90 places; `create_machine` with `start_with` is
how the desktop gets a typed `PcMachine` from the same request
(§14.6 Q3). `Presenter` stays a `Session<PcMachine>` with the PC's
catalogue model and keeps what the contract does not carry. A stored PC
image does **not** run in `player_ui::Player`: that would drop Insert…,
Layers, Save as new image…, Settings and the drive chips.
`Request::Assembled`, Stop, the kept catalogue and the workspace's
ownership on the worker do not change.

#### 14.2.7. The command line

`image run NAME:TAG` and `image machine run ID` first run through the
contract in `cli-pc` (S11), then become neutral commands of `cli-core`
(S12, last and droppable, §14.6 Q4):

```text
open the store; own the workspace (or the machine's)
platform  = the image's platform, from its manifest
provider  = the CliPlatform of that platform
core      = cores.for_platform(image_store::platform::canonical(platform))
stored    = provider.prepare_run(owner, request, core)   // the new hook
print "pinned-image:DIGEST"
machine   = core_launch::start(&stored.prepared)
for line in (stored.created)()  { print line }           // the PC: confirm, "inserted:A:\tdisk-1"
stop      = machine.run(RunLimit::steps(max_steps))
finished  = machine.finish()        // warnings and drained messages to standard error
print "workspace:NAME\tSTOP"  or  "machine:ID\tSTOP"
```

The manifest's platform goes through the store's alias table before
`for_platform`, which compares ids exactly: an old store names the retired
`ibm-pc-at` (corrected 2026-11-20, S8 review).

`cli-core` gains `core-launch` and `unemu-core-api` (both allowed);
`cli_core::run` takes the `Cores` the binary already makes. `CliPlatform`
gains one hook, `prepare_run`, and a way to own a machine's workspace; a
platform without stored images does not implement it, and the command
answers "this platform's images do not run from the command line yet".
The usage lines keep their words and their place.

**This is not a pure move** (§14.6 Q5). `Machine::run` is not
`run_session`: the last field of the result line is the contract's stop
(`Steps`, `Halted`, `PoweredOff`) instead of `RunOutcome`'s `Debug` text;
a guest reset is performed and reported by the core (the old assembly loop
already continued after it); a halted guest waits to the step
limit instead of 1 000 000 idle clocks. `commands.md` states the change
in the bead that makes it. `run --machine FILE` is not touched.

### 14.3. The steps

Fourteen beads, in order. Each lands alone on `main` with the bead gate
and moves tests with the code they test; "name proof", "layer proof" and
"runner" are rules 3, 4 and 5 of §8.1.5. No bead adds a line to
`scripts/crate-boundaries.allow`. Sizes are ASSUMED.

| Step | Depends on |
|---|---|
| S1 | nothing |
| S2 | S1 |
| S3 | nothing (it edits a file S1 edits: one after the other) |
| S4 | S2, S3 |
| S5 | S4 |
| S6 | S5 |
| S7 | S6 |
| S8 | nothing |
| S9 | S5; `upc-dfs.35`, `upc-dfs.36`; the `journal` crate of `upc-dfs.37` unless §14.4 item 2 holds |
| S10 | S7, S8; `upc-dfs.35` |
| S11 | S9, S10 |
| S12 | S11 |
| S13 | S9, S10 (after S7) |
| S14 | S11, S13, and S12 if it is done |

Beads that edit the same files go one after the other, in either order:

| Stage B | Existing | Shared |
|---|---|---|
| S1, S2 | `upc-dfs.34` (step 7), `upc-dfs.37` (step 11), `upc-dfs.38` (step 12) | `crates/machine-file` |
| S2 | `upc-dfs.28` (step 1) | the external-crate list: `serde` and `toml` reach the emulation layer through `media` today, leave it with step 11 and are needed again by `pc-config`, with the reason "the PC config codec" |
| S1, S2 | `upc-dfs.29` (step 2) | the host guard: `pc-config` adds no line to its list |
| S2 | `upc-dfs.28`, `upc-dfs.32` (steps 1, 5), any crate-creating bead | `scripts/crate-boundaries.sh` and its fixtures |
| S11, S12 | `upc-dfs.32`, `upc-dfs.33`, `upc-dfs.39` (steps 5, 6, 13), the recipe track's command beads | `cli-pc`, the usage text and `command_line_pin` |
| S13 | `upc-dfs.31` (step 4) | strings in [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs), `main.rs` |

**S1. The lowering asks its reader for everything of the host.** **Done.**

- **Scope.** Inside `machine-file`; no crate changes. `ImageReader` gains
  two methods with defaults that keep today's answers for a synthetic
  reader: the length of an image without reading it, and whether a path is
  a reserved retention name. `verify_journal_base` returns the base's
  length and digest; `LoadedMachine`'s reader and `FileImageReader` keep the
  `media::VerifiedBase` themselves. `lower.rs` then names neither `std::fs`
  nor `media`.
- **Files.** `crates/machine-file/src/lower.rs`, `lib.rs`, `paths.rs`,
  `integrity.rs`; [`crates/pc-image/src/provider.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/provider.rs) (its `Reader`).
- **Depends on.** Nothing.
- **MUST PROVE.** `machine-file`'s tests pass unchanged, the ones that pin
  diagnostics word for word among them. The goldens that load machine
  files pass. `grep` finds no `std::fs`, `std::env` or `media::` in
  `lower.rs` outside its tests. A ROM's size in the metadata mode is the
  same answer as before, so a Settings dialog shows the same diagnostic.
- **As landed.** `ImageReader` gained three methods, not two:
  `image_length` (default: cannot tell, so the size stays pending),
  `is_reserved_retention_name` (default: the `.retain-` name rule alone)
  and `validate_journal_paths` (default: nothing to check, which is safe
  only for a reader that cannot produce a journal; the trait says so). The third is a
  fourth host touch the plan did not list: the lowering called
  `paths::validate_journals`, which resolves journal and input paths on the
  host; a host reader now answers it. `verify_journal_base` returns a
  `JournalBase` (length and SHA-256, `From<&media::VerifiedBase>`). A test
  per host reader (`FileImageReader`, the reader of `load_for_assembly`,
  `pc-image`'s import reader) refuses a journal that aliases an input.
  `FileImageReader` and the reader of
  `describe` moved to `lib.rs`; `describe` is `describe_with` over the
  host's metadata reader, and the context that words diagnostics after
  lowering uses a reader that reads nothing. The `.retain-` rule is copied
  into `lower.rs`, and a test in `paths.rs` holds it equal to the
  journal's. `pc-image`'s import reader (`import.rs`, not in the file list)
  answers like `FileImageReader`; the provider's `Reader` needed no change,
  since it names logical assets, lowers no journal and never runs the
  metadata mode. For a synthetic reader the retention check no longer
  resolves its paths on the host, which only mattered for a host file
  reached through a synthetic path. MEASURED: `lower.rs` names no
  `std::fs`, `std::env` or `media::` outside its tests. A test lowers a
  machine with a journalled floppy and CMOS from a reader in memory, its
  images in a folder no host has; `MachineConfig::validate` (assembly)
  still resolves a journal's folder on the host, which is §12.10 steps 10
  and 11's.

**S2. `pc-config`: the machine schema and its lowering move.** **Done.**

- **Scope.** Create `crates/pc-config`, layer `emulation`, with
  `#![forbid(unsafe_code)]`. Move `schema/`, `lower.rs` (without
  `FileImageReader`), `integrity.rs`, `pc_names.rs`, `parse_toml`,
  `ParseError`, `LowerError`, and the TOML half of `origin.rs` and
  `source.rs`. `machine-file` depends on it and re-exports every moved
  name at its old path, so no caller changes. `Origins::from_yaml` becomes
  a function of `machine-file`, and its one caller changes (stated). If the
  TOML and YAML halves of `origin.rs` do not split cleanly, `Origins` moves
  whole and only the YAML decoder stays behind a function argument.
- **Files.** `crates/pc-config/**` (new), `crates/machine-file/src/**`,
  root `Cargo.toml`, `scripts/crate-boundaries.sh` fixtures,
  `scripts/affected-tests.sh`, `backlog/docs/architecture.md`.
- **Depends on.** S1.
- **MUST PROVE.** Name proof: the test list is equal, with
  `machine-file`'s moved tests under `pc-config`; the tests that write
  temporary files and use `FileImageReader` stay in `machine-file`. Layer
  proof, and a planted `pc-config → machine-file` and `pc-config → media`
  dependency each fail the check. `cargo tree -p pc-config` holds no
  `serde-saphyr`, no `image-store`, no C library; `pc-config` declares no
  `media`; its tree reaches it only through `assembly`, the allowed crossing
  of §7.4, until §12.10 step 11 (corrected 2026-11-20, S2). The slow
  goldens pass unchanged. The clean build time before and after (§10).
- **As landed.** `pc-config` holds `schema/`, `lower.rs`, `integrity.rs`,
  `pc_names.rs`, `parse_toml`, `ParseError`, `check_size` and
  `source_snippet`, `Origins` with its TOML decoder, and the provenance walk
  with its TOML adapter (`source`: `Shape`, `Node`, `children`, `LineIndex`,
  `record_toml`), all public because `machine-file` names them. `machine-file`
  depends on it and re-exports `lower`, `lower_with_options`, `parse_toml`,
  `pc_names`, `schema`, `ImageReader`, `JournalBase`, `LowerError`,
  `MachineFile`, `Origin`, `Origins` and `ParseError` at their old paths; it
  keeps the YAML halves of `origin.rs` and `source.rs`. Four things could not
  stay as they were, because a crate cannot add inherent methods or a foreign
  trait impl to another crate's type: `Origins::from_yaml` is the trait
  `machine_file::YamlOrigins` (its caller `yaml.rs` and six test files of
  `machine-file` import it; their bodies are unchanged); `MachineFile::describe`
  is the trait `machine_file::Describe` over `pc-config`'s now public
  `MachineFile::describe_with` (`pc-library`'s `new_machine.rs`,
  `image_settings.rs` and `settings_model.rs` and the window's [`shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs)
  import it); `From<&media::VerifiedBase> for JournalBase` is the function
  `machine_file::journal_base` (its callers in `machine-file` and
  `pc-image`'s `import.rs` changed); and `Origins`' merge helpers
  (`set_image_file`, `layer_files`, `use_document`, `has_field`,
  `remove_subtree`, `copy_subtree`, `remap_entries`), `keep_pins`,
  `CardKind::descriptor`, `lower_tracked` and `AssemblyDiagnostics` with its
  four methods became public, since `machine-file`'s merge and
  `LoadedMachine` name them. Behind `pc-config`'s `test-support` feature:
  `has_retention_marker` and `AssemblyDiagnostics::without_sources`, for
  tests that stay in `machine-file` because they need the journal crate.
  Tests: the lowering's tests that build without the host moved with it;
  those that use `FileImageReader` only as a type parameter of the private
  diagnostics context now use the lowering's own reader that reads nothing
  (no test read a file). Four tests stay in `machine-file` under their old
  paths, in a test-only `lower.rs`: the two of `metadata_tests` (temporary
  files, `FileImageReader`, the host's `describe`), the Lo-tech EMS parse in
  TOML and YAML (the YAML reader) and the internal-error test of an
  assembly drive index (it builds a `media::OpenError`; its context is now
  `AssemblyDiagnostics::without_sources`). MEASURED: the name proof holds
  (4377 tests before and after, equal with the package prefix removed;
  `machine-file` 337 before, 312 after and 25 in `pc-config`). The layer
  proof holds with the allow list unchanged; a planted `pc-config →
  machine-file` and `pc-config → media` each fail the check (live and as two
  new self-test fixtures). The external list keeps its lines; the reasons of
  `serde`, `toml` and their crates name the PC config codec.
  `scripts/affected-tests.sh` counts `pc-config` among the crates that
  select the golden tier; no data rule needed it. `pc-config` declares no
  `media`; `cargo tree -p pc-config` reaches it only through `assembly`, the
  allowed crossing of §7.4, until §12.10 step 11. It reaches `libc` through
  `media` until then and through `sha2`'s `cpufeatures` for good: declarations
  only, and listed. No C library is built, and the tree holds no
  `serde-saphyr`, `image-store` or `machine-file`.
  `cargo test --workspace --release -- --include-ignored` passes whole (4380
  tests, the slow goldens among them), with no test changed but the moves
  above. A clean `cargo build -p machine-file` (dev profile, an empty target
  folder, four jobs on a shared machine) took 167 s before and 149 s after,
  within the noise of that machine: the split adds no measurable build time.

**S3. `provider::prepare` splits into a store half and a codec half.** **Done.**

- **Scope.** Inside `pc-image`; no crate changes. The store half reads
  descriptors and blobs (kind, size limit, source link, the chain walk, the
  zero-base rule, the closure) and produces the `content` rows. The codec
  half takes the config, those rows and a reader of small content, and does
  everything else, the lowering included. The platform-alias check and
  `firmware_options()` stay in the store half and reach the codec half as
  arguments.
- **Files.** [`crates/pc-image/src/provider.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/provider.rs), `lib.rs`.
- **Depends on.** Nothing (S1 touches the same `Reader`: one after the
  other).
- **MUST PROVE.** `pc-image`'s tests pass unchanged, with every refusal's
  text; the first failure is the same for each fixture of
  [`crates/pc-image/tests`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests). The codec half names no `image_store` item but
  `Digest`. The order of the closure `validate` returns is unchanged: a
  test compares it for the fixtures before and after.
- **As landed.** `prepare` ([`crates/pc-image/src/provider.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/provider.rs)) is the
  store half `store_half` followed by the codec half `codec::lower`, a
  module of the same file. The store half walks the config's firmware
  bindings (only as many as the hardware has ROMs), its slots paired with
  the hardware's drives, and its CMOS binding; it finds each descriptor of
  the right kind, checks the PC size limit, the ROM's source link and source
  archive, the 64-byte CMOS, walks a fixed disk's chain, applies the
  zero-base rule and collects the closure. It answers a `ContentRow` per
  bound digest (the SHA-256 of the flat bytes, from `disk::identity`, and
  their length: the decoded size, or the geometry's capacity for a layered or
  zero head), the closure, whether the platform is the PC's or its alias,
  and a `StoreContent` that reads a bound ROM or CMOS image through
  `BlobSource`. The codec half takes the config, the rows, a
  `codec::SmallContent` (the bytes of small content by digest), that
  platform answer and the `AssemblyOptions` from `firmware_options()`, and
  does the rest: the envelope, every binding against the hardware, the
  slots, each medium's size against its geometry, the lowering, the drive
  lists and the BIOS check (`validate_bios_with`, `lib.rs`; `validate_bios`
  keeps its signature over it, so `import.rs` is unchanged). It names no
  `image_store` item but `Digest`, no descriptor, blob source, media type
  constant, `disk` or `zero` (MEASURED: `grep` over the module). The
  store half stops its walk where the codec half refuses the config's shape:
  at a firmware binding that differs from its ROM, a ROM count that differs
  from the bindings, a duplicate or empty medium id, a slot that fails the
  codec half's own `check_media_fields` and `check_slot` (position,
  selection, the IDE and single-image shapes), an unknown medium, and a
  writable medium bound again. So a config with one defect fails with the
  same sentence as before; a config with a shape defect and a store defect
  before it may now name the store's. MEASURED: every outcome of `prepare` in the test runs of
  `pc-image`, `pc-library` and `cli-pc` with the ignored tests (2700 calls,
  234 distinct inputs; each keyed by the config and descriptors, with the
  refusal's words, or a hash of the lowered machine and the closure) was
  logged by a throw-away probe before and after: the two sets are equal, and
  the only new entry is the new test's (measured before the review's fix,
  which only makes the store half return earlier on configs the codec half
  refuses). Three unit tests, each failing before that fix, hold one
  slot-pairing defect to its old sentence: slots in another order than the
  drives, a journalled floppy in the IDE slot as well, an IDE catalogue of
  two media. Tests: two unit tests run the codec
  half over a config, rows and small content in memory (no descriptor, blob
  source or store; the floppy's bytes are not given and never read) and
  compare its machine and closure with the store's path, and refuse from a
  row's size, a missing row and the platform argument; an integration test
  in [`zero_base.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/zero_base.rs) pins the closure order for a blank and a layered fixed
  disk image against the order the config states, and it passed on the code
  before the split. `cargo test --workspace --release -- --include-ignored`
  passes whole (4396 tests, the slow goldens among them).
  Left for S4, which moves the codec half to `pc-config`:
  - it still names `machine_file::LoadError` (wrap `ParseError` directly),
    `crate::Error`'s `Load` and `Lower` variants with `needs_firmware`, and
    `super::hash_array`;
  - a `ContentRow`'s key stays a digest, `pc-config`'s own with the same text
    form; `ContentRow::sha256` becomes `[u8; 32]`;
  - `SmallContent::read(digest)` becomes `ContentSource::open(row.sha256,
    row.size)` through the firmware and CMOS rows, which S3 fills and does
    not read yet.

**S4. `pc-config`: the image config and its codec move.** **Done.**

- **Scope.** Move to `pc_config::image` the model of `lib.rs` with public
  fields, `medium_id`, `firmware_images`, `drive_tables`, `unit`,
  `validate_bios`, the codec half of S3, and `encode.rs` and `defaults.rs`.
  The codec half reads small content through `unemu_core_api::ContentSource`;
  `pc-image` implements that over `BlobSource`. `pc-config` gets its own
  digest type with the same text form (`sha256:` and 64 hex digits), and
  `pc-image` converts at its edge.
- **Files.** `crates/pc-config/src/image/**` (new);
  [`crates/pc-image/src/lib.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/lib.rs), `provider.rs`, `encode.rs`, `defaults.rs`,
  and the modules that name the model (`revision.rs`, [`create.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/create.rs),
  `import.rs`, `template.rs`, `machine.rs`, `workspace.rs`,
  [`inspection.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/inspection.rs), [`desktop.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/desktop.rs), `compat.rs`, [`disk.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/disk.rs), [`zero.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/zero.rs)).
- **Depends on.** S2, S3.
- **MUST PROVE.** **Byte identity of the config**: for every fixture and
  template the tests build, the encoded config's bytes and so the image's
  digest are the same before and after; a new test pins one image digest
  if none is pinned. Name proof. Layer proof; `pc-config` still has no
  store dependency. `pc-image`'s tests pass unchanged.
- **As landed.** `crates/pc-config/src/image/` holds [`mod.rs`](../../../crates/pc-config/src/image/mod.rs) (the model
  with public fields: `Config` with `Config::decode` and `Config::encode`,
  `Firmware`, `Medium`, `Slot`, `Nvram`, `NvramMode`, `Policy` with
  `from_write` and `write`, `Geometry` with `of`, `of_medium` and `fits`;
  `firmware_images`, `drive_tables`, `unit`, `validate_bios`, and the
  private deserializer `medium_id`;
  the error `image::Error` with `Parse`, `Lower` and `Invalid`, whose words are
  `pc-image`'s), [`codec.rs`](../../../crates/pc-config/src/image/codec.rs) (S3's codec half: `lower`, `Lowered`,
  `ContentRow`, `check_media_fields`, `check_slot`), `encode.rs` and
  `defaults.rs` (moved whole but for their import paths and two doc
  comments) and `tests.rs`. The
  digest is `pc_config::image::Digest`, 32 bytes, read and written as
  `sha256:` and 64 lowercase hex digits with the store's refusal sentence, so
  its order is the store's and a config's bytes are the same whichever crate
  decodes it. `ContentRow` is `{ sha256: [u8; 32], size }`, keyed by that
  digest; the codec opens a bound ROM or CMOS image with
  `ContentSource::open(row.sha256, row.size)` and takes `Content::Bytes` or
  reads a `Content::Reader` whole; content of another length than its row,
  as bytes or as a reader, is refused before it is read (a test with a lying
  source). For S5: the codec puts no PC size limit on `row.size`; on the
  store's path `bound` holds it, but on the core's path the row sizes come
  from the launch document, so S5 must bound them. `pc-config` gains `unemu-core-api`, and
  `pc-image` gains `pc-config` and `unemu-core-api` (no new external crate;
  three lines of the lock file). Two things differ from the
  plan's words: `firmware_images` and `drive_tables` are generic over the
  caller's error type (`E: From<image::Error>`), and `pc-image` keeps
  private wrappers of them, of `unit`, of `decode` and of `validate_bios` (the
  last over its test-only firmware permission), so its modules call them as
  before; and the digest conversion at `pc-image`'s edge (`StoreDigest::store`,
  `ConfigDigest::to_config`, `lib.rs`) goes through the text form and so returns
  a `Result`, since `image_store::Digest` has no public constructor from bytes
  (it never fails: both sides check the same form). `pc-image` keeps the
  store half and every store operation; `prepare` calls
  `pc_config::image::lower` with a `StoreContent`, a `ContentSource` over
  `BlobSource` keyed by the rows' SHA-256 that refuses an unbound digest, a
  payload without a PC limit and a size that differs from the row. Its
  `Error` is unchanged; `From<image::Error>` maps `Parse` to
  `Load(LoadError::Parse)`, `Lower` to `Lower` and `Invalid` to `Invalid`, so
  `needs_firmware` and every refusal's text are as before. `disk::bases` is
  keyed by the config's digest. `pc_image::Policy` is a re-export.
  `template.rs` and `compat.rs` needed no change. Tests: the S3 test of the
  codec's refusals from its rows and platform argument moved to
  `pc_config::image::tests` over content in memory (no store, no
  descriptor); a new test there holds the digest's text form, its refusal
  sentence and its round trip. MEASURED: byte identity by a throw-away probe
  over `cargo test --workspace --release -- --include-ignored` before and
  after, logging the SHA-256 of every config `pc-image` encodes (import,
  create, revision, publication), of every decoded config encoded again (at
  each `prepare` and each resolved image), and every `prepare` outcome (the
  lowered machine, the closure and the config, or the refusal's words) keyed
  by its config and descriptors: 677 distinct lines before, the same 677
  after. The pinned template digests (`unemu/open-at`, `unemu/open-at-games`
  in [`crates/pc-image/tests/template.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/tests/template.rs)) hold, so no new pin was needed. The
  name proof holds: the test lists are equal but for the moved test (from
  `pc_image` `provider::tests` to `pc_config` `image::tests`) and the new
  ones (the digest test, and the length test of the review's fix;
  `pc-config` 25 to 28 unit tests, `pc-image` 62 to 61). The layer proof
  holds with the allow list and the external list unchanged; a planted
  `pc-config → image-store` fails the check; `cargo tree -p pc-config`
  declares `assembly`, `serde`, `sha2`, `thiserror`, `toml` and
  `unemu-core-api` only, with no `image-store`, `machine-file` or
  `serde-saphyr` in its tree. `cargo test --workspace --release --
  --include-ignored` passes whole after the bead, the slow goldens and every
  doc test among them.

**S5. `pc-config`: the launch document.** **Done.**

- **Scope.** `pc_config::launch` as §14.2.2 has it: `Launch`, `decode`,
  `encode`, `check`, `lower`, `Lowered`. `pc-image`'s validation calls
  `launch::lower`, so one function lowers for the store and for the core.
  Nothing starts a machine from it yet.
- **Files.** `crates/pc-config/src/launch.rs` (new),
  [`crates/pc-image/src/provider.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/provider.rs); this section (the document's final
  format).
- **Depends on.** S4.
- **MUST PROVE.** A document round-trips byte for byte. `check` and
  `lower` refuse the same documents with the same words. `check` lists
  exactly what `lower` opens, in a test with a counting `ContentSource`. A
  document with a missing, a doubled or an unused `content` or `record`
  row is refused. For each fixture image, `lower` gives the
  `MachineConfig` that `prepare` gave before the bead (`==`).
- **As landed.** `crates/pc-config/src/launch.rs` holds the model
  (`Launch`, `ContentRow`, `RecordRow`, `Lowered`), the wire form (a private
  `Document`: the stored config's fields in their order, then `[[content]]`
  and `[[record]]`; `sha256` as 64 lowercase hex digits), `decode`, `encode`,
  `check` and `lower`, and the bounds of §14.2.2 (document 2 MiB, firmware
  row 16 MiB, CMOS row 64 bytes; `pc-image`'s payload table now takes its
  firmware and CMOS limits from these constants). The codec of S4 is split in
  two halves, `codec::bind` (envelope, bindings, slots, sizes against
  geometry; it now also answers each bound digest's role) and `codec::finish`
  (the lowering, the drive lists, validation); `check` and `lower` run the
  launch's table checks between them, so a shape defect is refused with the
  codec's sentence before a table defect is looked at, and `pc-image`'s
  refusals keep their order. `image::lower` is `bind` then `finish`, as
  before. Five things differ from the plan's words:
  - `lower` answers `pc_config::image::Error`, not `unemu_core_api::Error`:
    the store maps its variants (`needs_firmware`, every refusal's words),
    and `PcCore` maps it to the contract's in S9.
  - A medium's row has no size limit of its own (the plan named none; S4's
    note asked for bounds): its length must equal the medium's
    geometry, which assembly holds to what the drive takes (at most
    8,455,200,768 bytes for ATA CHS), before a disk is opened.
    A firmware row above 16 MiB and a CMOS row other than 64 bytes are
    refused before anything is opened, in the store's words.
  - `check`'s lowering is the metadata mode with each ROM sized from its
    row, plus two changes to that mode for a check only (`Mode::Check` in
    `lower.rs`; the Settings dialog's description is unchanged): the system
    BIOS is required and the lowered machine is validated, both with the
    full lowering's words. What only a ROM's bytes decide stays `lower`'s:
    the firmware and CMOS pins, catalogue membership, an option ROM's header
    and checksum, and the firmware services of a ROM that validation cannot
    identify before its bytes are read (it defers them). So `Core::validate`
    can pass a document whose `create` then refuses its content.
  - The store's validation builds a launch with each journal in the record
    `<id>.upfj` / `cmos.upfj` and the default retention, so a stored config
    with a medium id that starts with `-` is now refused at validation, with
    the record sentence (MEASURED: import and create make ids `medium-N`; no
    probed input had one). S6's check at start stays to be done for the ids
    of a start.
  - **The validated machine's CMOS.** `lower` turns a journalled CMOS into
    `NvramSource::Journal` over the CMOS binding's logical asset name, with
    `cmos.upfj` and the record's retention, where the codec of S4 copied its
    64 bytes (`NvramSource::Image`). `prepare` now answers that machine, so
    the `==` of MUST PROVE holds for every fixture without a journalled CMOS,
    and for one with it up to exactly that field. Every user of `prepare`'s
    machine was read: `resolve_pinned` replaces the field for a journalled
    CMOS, the overlay reads the CMOS bytes only in image mode,
    `defaults::materialize` and `cmos_layout_changes` ignore it, and the
    settings flow compares two machines that both have it.
  Review corrections: validation now uses `MachineConfig::validate_logical`,
  which keeps hardware, journal-retention and distinct-key checks without
  filesystem normalization. The image codec's reader uses it during lowering
  as well as after journal keys are installed; file readers keep their host
  checks, and the start path still receives the same journalled CMOS. A
  subprocess regression validates and lowers a journalled CMOS with a dangling
  `cmos.upfj` symlink in its own working directory, without changing the test
  runner's cwd. Metadata checks use mapped ROM lengths as `Blank` sources
  on a clone (including container offsets) through the same validation sequence
  as full lowering. Firmware checks use the original unread sources, so
  byte-dependent checks remain deferred; descriptions are unchanged. An invalid
  RTC start preceding overlapping option ROMs now fails with the full lowering's
  first sentence, as does an overlap alone. The subprocess test uses a unique
  directory and a drop guard to remove it even if setup or spawning fails.
  Launch `decode` checks its envelope version, then reuses
  `image::Config::decode` for the stored fields. That shared decoder parses the
  hardware of known envelopes with the existing machine-schema decoder. Future
  stored envelopes stay opaque until platform validation diagnoses their version,
  preserving the readiness diagnostic. Unknown hardware keys and unsupported hardware
  versions are therefore refused without adding a separate hardware grammar.
  The launch envelope, content rows and record rows already rejected unknown
  fields; regression coverage now pins those refusals too.
  Tests (`pc_config::launch::tests`, all new, so none could run before the
  bead): the round trip and the pinned encoding of a document; `decode`'s
  refusals; `check` lists exactly what `lower` opens (a counting source,
  with a journalled and with an image CMOS: `lower` opens the ROM and the
  CMOS, names the floppy bases and the journalled CMOS's image, and opens no
  floppy); every key of the machine is in `bases` or `records` and each
  journal keeps its row's retention (a kept record name among them), the
  machine otherwise equal to `image::lower`'s; seventeen table and binding
  defects (a missing, doubled and unused row of each table, bad record names,
  a shared record, retention 0, sizes, the retired platform spelling, an
  unknown medium) refused by `check` and `lower` with the same words before
  anything is opened; four hardware defects (memory, CPU, no BIOS) refused by
  both with the same words, the last of which failed before `Mode::Check`
  required the BIOS. `pc-image`'s
  `the_codec_half_lowers_from_memory_and_reads_only_small_content` now
  expects the journalled CMOS in `prepare`'s machine (it fails on the code
  before the bead, which copies the bytes). MEASURED by a throw-away probe in
  `prepare` over the tests of `pc-image`, `pc-library` and `cli-pc` with the
  ignored ones (2715 calls): each outcome (the lowered machine, the closure
  and the config, or the refusal's words), keyed by the config and the
  descriptors, logged before and after the bead, with the old machine's
  journalled CMOS rewritten to the new form: 238 distinct lines before, the
  same 238 after. The same probe ran `check` beside every `lower`: 2693
  agree (2667 accepted, 26 refused with the same words) and 18 differ, all
  `check` passing what `lower` refuses on content: 16 the services of a
  ROM identified only by its bytes, 2 a firmware blob missing from the store.
  Completion review adds coverage of the exact 2 MiB document boundary and
  an oversized encoding, deduplication of distinct heads naming the same flat
  content while preserving both base keys, and the largest ATA CHS row
  (8,455,200,768 bytes), the next cylinder and a `u64::MAX` row, without
  opening disk content. The wire pin and byte-exact round trip cover the
  canonical encoding that `encode` writes; `decode` also accepts other TOML
  formatting, whose whitespace is normalized by `encode`.
  MEASURED: eight source mutations each fail one targeted test: removing the
  row-size check, the metadata check's BIOS requirement, flat-identity
  deduplication, final geometry validation, the pinned retention field name,
  per-medium retention, the journalled CMOS base key, or the encoding size
  limit. Each source was restored with a plain copy and touched before the
  final gate.

**S6. A start is a launch: `pc-image`.** **Done.**

- **Scope.** `resolve_pinned` builds a `Launch`: the effective config
  after the settings overlay, the `content` rows from the bases, the
  `record` rows with the retention. `PreparedSession` carries it. Its
  `machine` is derived from it by `launch::lower` and one function that
  puts today's paths where the keys are, never edited beside it.
  `insert_medium` edits the launch. Assembly and the callers do not change.
- **Files.** [`crates/pc-image/src/provider.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/provider.rs), `workspace.rs`,
  `machine.rs`.
- **Depends on.** S5.
- **MUST PROVE.** For every start the tests of `pc-image`, `pc-library`
  and `cli-pc` make (an image, a machine with settings, a pending insert of
  one medium and of a whole set, a compressed floppy, a layered and a
  zero-based disk, a journalled CMOS), the `MachineConfig` and the keys of
  the bases equal those before the bead. The slow goldens and the
  front-end goldens pass unchanged. A medium id that cannot be a record
  name is refused with its sentence; MEASURED: no fixture and no template
  has one. A test fails when an editor changes the launch without the
  derived config.

- **As landed.** `resolve_pinned` builds a `workspace::PreparedLaunch`:
  the effective config after the settings overlay (under `ibm-pc`), content
  rows with each verified base's flat SHA-256 and length, record rows with
  the session/import retention, small firmware and CMOS content, and maps
  from logical base keys and record names to today's host sources and
  journal files, plus the decoded bytes of in-memory floppies.
  `provider::lower_session` calls `pc_config::launch::lower`
  and is the one function that translates its reported base and record keys
  to those paths (or inline bytes for a compressed, non-journalled floppy,
  chosen from the edited policy). It changes no hardware, catalogue, pin or
  retention field. An
  unjournalled CMOS recomputed by Settings gets a launch-only content binding
  for its new bytes; no store object is written. The source image, stored
  config and base files stay unchanged.
  `PreparedSession` carries this launch alongside its derived `machine`;
  `edit_launch` lowers an edited copy before installing either value, and a
  refused edit leaves both unchanged. `SessionTransfer` carries the launch
  and its small content too, while verified bases still travel as handles,
  bytes or zero-chain parts. Decided here: the launch field is optional for
  callers that construct synthetic sessions directly; every stored-image
  preparation supplies it. Three such test literals needed `launch: None`
  (one in [`pc-library/src/library_worker.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/library_worker.rs), two in
  [`pc-ui/src/library_view.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view.rs)); their assertions and production callers are
  unchanged. `machine::insert_medium` now edits the document's media,
  catalogue, content and record tables through the same transactional editor,
  then extends the verified bases. Its generated `insert-N` ids and
  `kept-insert-N.upfj` record names are bounded and valid; the storage map
  preserves `<set digest>-<medium>.upfj`. The chosen medium still leads,
  duplicate entries are replaced by flat digest and label, and a single
  image converted to a catalogue keeps its old source, policy, journal and
  `A: image`/`B: image` name. Generated ids also avoid existing logical asset
  names. A read-only image shared with another slot gets
  a separate binding before it gains a catalogue name.
  A start refuses ids that cannot form a core record name, including the
  reserved `kept-` prefix, even for a read-only medium; the sentence names
  the id. Validation's earlier journal-name refusal is unchanged. Created
  disks and kept catalogues still edit the public machine after preparation:
  S7 owns their move, so transfer preserves that machine rather than
  deriving it again. Assembly, desktop starts, CLI runners and session-test
  callers are unchanged. No crate or dependency changed. The now-unused
  `Overlay::machine` field and its initializer were removed in `revision.rs`.
  Stored-image validation retains its original medium-name restriction:
  slashes, backslashes, empty names and NUL are refused with the original
  sentence. Only launch labels accept slashes and backslashes, preserving
  pending media-set labels without changing their text. A pending-insert
  test failed when the stored-image restriction was applied to its launch.
  It also fails when generated ids ignore an existing logical asset of the
  same name. A compressed-floppy policy-edit test failed before path
  restoration chose file keys or inline bytes from the edited policy.
  Tests (`provider::tests`, six new; `pc-image` unit tests 61 to 67): a launch
  edit rederives catalogue selection, labels and floppy/CMOS retention, and
  survives a thread transfer; a rejected edit preserves both values; read-only
  media ids `-a` and `kept-a` are refused before the session is written; one
  pending disk and a whole set, read-only and journalled, produce exactly the
  former assembly config and journal paths, including labels with slashes and
  an existing logical asset that could collide with a generated name; a
  compressed floppy switches from a journal key to inline bytes and back when
  its launch policy changes; a stored-image run pins the unchanged CPU revision,
  67,234 retired instructions and VClock 162,500 at the same 100,000-step cap,
  measured on the old path first. MEASURED: five source mutations each fail a
  targeted test (leaving `machine` stale, publishing a rejected edit, dropping
  the document at transfer, omitting the start's id check, ignoring a logical
  asset collision). The label and compressed-policy regressions also failed
  before their fixes. Every mutant was restored with a plain copy and touched.
  Review regressions (three additional tests) reproduce a compressed
  catalogue's source key being reused by another drive's read-only insert,
  unused backing maps surviving document pruning, and stored labels being
  accepted with slashes. Rebinding now clears inline bytes, and pruning
  retains only the document's source, content and record bindings. All three
  tests fail against the reviewed code and pass after the corrections.
  A temporary probe of each `resolve_pinned` and `machine::prepare` result over
  the fixture tests of `pc-image`, `pc-library` and `cli-pc`, their dependent
  front ends and the slow goldens hashes the whole `MachineConfig` plus the
  verified-base keys and identities, normalizing only temporary store/session
  roots: 610 starts and 92 distinct signatures before; 623 starts and 97 after,
  the baseline signature set is a subset of the new set (the new tests
  account for the additions). This establishes set inclusion, not equality
  of the signatures of each paired start.
  No fixture or template start has an invalid core record id. The comparison
  run passes 1,662 of 1,731 tests; its 69 failures are exactly the baseline's
  sandbox loopback-bind refusals, each OS error 1. The slow BIOS, FreeDOS,
  journal, front-end and session goldens pass with their assertions unchanged.

**S7. Created disks and kept catalogues edit the launch.** **Done.**

- **Scope.** `created_media::apply` and `kept_catalogue::restore` edit
  the launch's tables; the `MachineConfig` is derived afterwards. Their
  journals get `kept-…` record names in the launch, with the file each
  stands for, a journal beside a host file outside the workspace included.
- **Files.** [`crates/pc-media/src/created_media.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-media/src/created_media.rs), [`kept_catalogue.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-media/src/kept_catalogue.rs);
  [`crates/pc-library/src/library_worker.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/library_worker.rs) (`copied_disks`, the machine
  start); [`crates/pc-image/src/workspace.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/workspace.rs) (the table of record files).
- **Depends on.** S6.
- **MUST PROVE.** `pc-media`'s and `pc-library`'s tests pass unchanged.
  The machine config of each start flow that has created disks or a kept
  catalogue equals the one before the bead. Every `kept-…` name is a valid
  kept record name of at most 64 bytes, for a 64-digit digest too.

- **As landed.** `created_media::apply_one` and
  `kept_catalogue::restore_drive` collect the catalogue's entries and verified
  bases without editing the session's machine. They call
  `PreparedSession::replace_floppy_catalogue` (`workspace.rs`), which edits a
  copy of the launch's media, slot, content and record tables, prunes unused
  bindings, and calls the existing `provider::lower_session` before installing
  the document, machine and added bases together. Existing bindings keep their
  heads, decoded floppy bytes, paths and retention; a shared read-only binding
  gets its own name without changing another slot. New bindings use
  `catalogue-N` ids and `kept-catalogue-N.upfj` records, independent of the
  digest or host filename. Even the largest `u64` suffix fits within 64 bytes.
  `PreparedLaunch::record_files` exposes the record-to-file table; it preserves
  created disks' `<64 hex>.upfj` files, media sets' `<set hex>-<id>.upfj`
  files, and journals beside host files outside the workspace. No file is
  opened or written by this edit; bases remain immutable and verified in full,
  with sector writes through the existing journal overlay.
  The worker's copy and machine-start flows keep their order and confirmation
  rules. Its existing synthetic-firmware backend now scopes these launch edits
  too; production retains the same firmware checks.
  One small correction outside the file list was necessary: `pc-config`'s
  launch lowering accepted only the first catalogue disk as the initial
  selection. Kept catalogues restore any selected disk without reordering the
  list. `image::codec::bind` now permits any member for a launch, and `finish`
  derives its index; `image::lower` retains the first-entry restriction.
  Review found that stored-image validation also lowers through a launch:
  a partial store walk could supply all content rows for two slots sharing
  read-only media and thereby accept a non-first selection. `store_half` now
  propagates the strict slot check's error before launch lowering. A provider
  regression covers that shared-catalogue case without bound NVRAM and keeps
  the original "unsupported initial selection" refusal. The codec regression
  covers a desktop launch's second entry and an unknown selection.
  Three worker regressions cover created split disks in machine and Gaming-mode
  starts, re-lowering and thread transfer; a kept catalogue's order, selection,
  exact journal bytes and immutable base on two restarts; and a journal outside
  the workspace, retention and atomic refusal of an invalid edit. Each checks
  that the document alone reproduces the complete prepared machine, and that
  record names are valid and bounded even for 64-digit disk identities.
  Review coverage also checks a valid catalogue edit refused by lowering for
  zero journal retention: the document, record paths, machine and base
  identities stay unchanged. Created disks lose to a later pending insert at
  the same start, and two independent creation operations keep distinct bases,
  catalogue order, selection and journal paths when restored together.
  MEASURED: restoring the original `pc-media` editors makes the created-disk
  and kept-catalogue regressions fail because their launches lack the added
  entries, while the existing worker lifecycle tests still pass. With the new
  editors all 33 worker lifecycle tests pass. A temporary probe compared the
  complete machine and verified-base keys and identities at 24 paired starts
  across eight existing kept-catalogue lifecycle tests, including copies and
  Gaming mode: every signature is equal, normalizing only temporary fixture
  roots and the generated copy workspace name. The probe was removed.
  Two further mutations fail their targeted regressions: disallowing non-first
  launch selections, and omitting an added disk's record-to-file mapping.
  Mutated sources were restored with plain copies and touched before the gate.
  No crate, dependency, front-end assembly call, run policy or stored format
  changed; the object-based plans
  of S10 remain unimplemented.

**S8. `core-launch`: a `Prepared` from parts.** **Done.**

- **Scope.** §14.2.5: `ContentPlan`, `StoragePlan`, `Parts`,
  `Prepared::from_parts`, `with_options`, `start_with`,
  `Cores::for_platform`. `Prepared`'s file-only accessors (`file_name`,
  `sha256`, `saves_dir`) answer for a file and say so for parts. A plan
  opens more than once (the player's power cycle starts again from the
  same `Prepared`).
- **Files.** [`crates/core-launch/src/launch.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/core-launch/src/launch.rs), `lib.rs`, `tests.rs`;
  [`crates/player-ui/src/lib.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/player-ui/src/lib.rs) only if an accessor's type changes.
- **Depends on.** Nothing. It can land at any time.
- **MUST PROVE.** With the test core: a `Prepared` from parts starts a
  machine whose record is read by the next start; `start_with` hands the
  caller's own type back; an option reaches `create` and an unknown one is
  the core's refusal; `Prepared` is still `Send` (the compile-time
  assertion). The existing tests pass unchanged. `core-launch` still
  depends on the contract, `unemu-core-files` and `sha2` only.
- **As landed.** [`crates/core-launch/src/launch.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/core-launch/src/launch.rs) has `ContentPlan`
  and `StoragePlan` (each `Send`, `open(&self)`), `Parts` with public
  fields (`core`, `platform: &'static str`, `title`, `notes`, `config`,
  `content`, `storage`, `options`), `Prepared::from_parts`,
  `Prepared::with_options` (appends, after the options the start has) and
  `start_with`; `start` is `start_with` over `Core::create`. `lib.rs` has
  `Cores::for_platform`: the first usable core whose description lists the
  id, compared as it is (no alias). `Prepared` keeps what both starts share
  and a private source, the opened file or the two plans; each start opens
  the plans anew, so a power cycle starts again from the same `Prepared`.
  A start from parts passes a plan's failure and `create`'s error back as
  they are: the sentence for a record another machine holds stays the file
  start's, and a platform's storage plan words its own (§14.3 S10). A
  platform the core does not describe is named by its id, and `create`
  refuses it. Decided here: the file-only accessors answer `Option`, `None`
  for parts, and they are six, not the three the scope names, since
  `kind`, `size` and `states_dir` have no answer for parts either; the
  accessor types changed, so their callers changed. Production: in
  `player-ui`, `lib.rs` (the "Opened …" message falls back to the title;
  `describe` offers Save and Load state only with a states directory) and
  [`quick_state.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/player-ui/src/quick_state.rs) (`save_state` and `load_state` take the directory from
  the accessor). Tests, each line only reading the accessor through
  `Some`/`unwrap`: [`window/src/shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs), one line in each of
  `pause_reset_power_cycle_and_stop_reach_the_players_machine` and
  `save_and_load_state_chords_reach_a_player_that_saves_state_and_the_pc_s_guest`;
  [`platforms/tests/nes_cartridge.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/platforms/tests/nes_cartridge.rs), one line; [`player-ui/src/tests.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/player-ui/src/tests.rs),
  five lines (the helper `quick_state`, two in
  `save_state_run_on_and_load_state_bring_the_saved_picture_back`, one each
  in `a_save_that_fails_keeps_the_state_saved_before` and
  `a_machine_without_saved_state_has_neither_item_and_saves_nothing`);
  [`core-launch/src/launch/tests.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/core-launch/src/launch/tests.rs), the helper `battery_file` and the
  assertions of `one_core_that_runs_the_file_makes_it_ready` and
  `the_whole_file_is_hashed_however_large`. So "the existing tests pass
  unchanged" holds only up to those `Option` reads: no assertion changed
  what it checks, and no other test changed. `with_options` appends, so
  `create` can see a key twice; core-launch neither merges nor refuses it.
  For S9: `PcCore::create` must define the rule for a repeated option key
  (recommended: refuse it with `Usage`). For S12: `for_platform` compares
  the id exactly, and an old store's manifest may name the retired
  `ibm-pc-at` ([`cli-pc/tests/platform_alias.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/tests/platform_alias.rs) runs such an image), so the
  neutral command passes the manifest's platform through
  `image_store::platform::canonical` first; §14.2.7's pseudo-code says so
  (corrected 2026-11-20, S8 review). The platform-id scan ([`crates/image-store/tests/platform_ids.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/tests/platform_ids.rs))
  lists the two new exact lookups, `for_platform`'s and `from_parts`'s name
  of the platform, beside the launch path's one before them: both sides
  are a core's current spelling, and `core-launch` may not depend on
  `image-store` for `platform::same`. No crate gains a dependency. Tests
  (`core-launch` 33 to 41,
  `player-ui` 34 to 35): with the test core, a `Prepared` from parts starts
  three times, each start opening both plans once and reading the record
  the one before wrote, and a second `Prepared` over the same records
  continues them; `start_with` hands back a caller's type, for parts and
  for a file; options from the parts and from `with_options` reach `create`
  in that order (a recording core), and the test core's refusal of an
  option it does not describe comes back as it is, for parts and for a
  file; a plan that cannot open fails the start with its own error; a
  record held by another machine of a start from parts comes back as the
  plan's storage worded it, not as the file start's sentence (it fails with
  the parts branch rewriting a `Usage` error); a
  `Prepared` from parts made on another thread starts; the accessors of a
  start from parts; `for_platform` skips a refused core and takes the first
  usable one; in `player-ui`, a machine started from parts offers no saved
  state and is named by its title. MEASURED: the new tests do not compile
  on the code before the bead; with the options left out of the request,
  or `for_platform` asking refused cores, the option test and the
  `for_platform` test fail; with `describe` offering saved states without
  a states directory, the `player-ui` test fails. `Prepared` and `Parts`
  are `Send` by the compile-time assertion. `core-launch`'s dependencies
  are unchanged (`unemu-core-api`, `unemu-core-files`, `sha2`).

**S9. `PcCore::create`, `create_machine` and `validate`.** **Done.**

- **Scope.** §14.2.3. The mount over `ContentSource` and `Storage`; the
  two run-only options; one display per display card;
  `PcMachine::config()`. The platform lists the launch type. No front end
  calls it yet, except the test over every compiled-in core. If it is more
  than a day, the mount and its tests are one bead and `create` with the
  description another.
- **Files.** `crates/pc-core/src/core.rs`, `machine.rs`, a new
  `mount.rs`, `machine/tests.rs`, `Cargo.toml`; [`crates/pc-session-tests`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-session-tests)
  (the comparison); [`crates/platforms`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/platforms) (its test).
- **Depends on.** S5; `upc-dfs.35` and `upc-dfs.36`; the `journal` crate
  of `upc-dfs.37` unless the seam of `upc-dfs.36` is in the contract's
  types (§14.4).
- **MUST PROVE.** One session through `create` over the contract's
  storage and content in memory, and the same session through
  `assemble_with_verified_bases`, give the same frames, sound, drive lights
  and messages, and the record's bytes equal the journal file's. A second
  `create` over the first one's records continues the session. A `create`
  that fails (a missing content, a held record, a bad document) leaves no
  record a later `create` takes for a session. `validate` lists what
  `create` opens. The replay rule of §3.11. A machine made by `create`
  still takes a medium the media dialog appends by path
  (`append_floppy_verified`). The host guard's list gains no line for
  `unemu-pc-core` or `pc-config`. The journal goldens pass.

- **As landed.** `unemu-pc-core` depends on `pc-config`. `PcCore::validate`
  checks the platform and document type, decodes the launch and calls
  `launch::check` without opening content or storage. `create` delegates to
  the typed `create_machine`: check the type and options, decode and check,
  acquire exactly the validated flat content identities, lower through a
  cache of those readers, acquire the initially inserted media's and CMOS's
  records, and call `assembly::assemble_mounted`. The new `mount.rs` maps
  logical base keys to owned content readers and journal keys to the launch's
  record names. It keeps storage for later catalogue inserts; uninserted
  media's records stay free. All content is acquired before any record, and
  all initial records before journal creation, so missing content and held
  records cannot leave a partly initialized session. Empty records left by
  an unsuccessful acquisition remain empty and a later create can use them.
  The assembler keeps its path-based append seam: a verified floppy appended
  by the media dialog uses the separate file mount.
  `PcMachine` owns a clone of the lowered config and exposes `config()`.
  Displays are routed in the fixed order MDA, VGA, independent of card order.
  The platform lists the launch media type and two run-only options,
  `allow-unlisted-firmware` and `speaker-trace`; both accept exactly `true`
  or `false`, default to false, and neither changes the hardware config.
  Decided here: repeated keys, including equal values, are refused with
  `Usage`, as are unknown keys and invalid values. Other platform spellings
  (including the retired alias) and document types are `Unsupported`;
  malformed documents and hardware are `Config`; content size lies and ROM
  pin mismatches are `Content`. Original content and record errors keep their
  contract kind across the codec and journal's internal textual I/O errors,
  including `StorageUncertain`. Factory-created machines retain errors per
  persistence owner and select the owner named by the failed operation. A
  swallowed retention warning cannot mask a later fatal error on another
  record, and a later I/O failure on the same record supersedes the warning.
  If several records fail during finish, the first failed handle's error is
  preserved. The existing wrappers keep their behavior.
  Disks use the source's full-hash receipt: journalled bases compare the
  receipt, and non-journalled media omit redundant byte pins on the private
  assembly clone. The public config retains its pins. ROM pins and CMOS
  identity are still checked when their bounded bytes are read. No host guard
  allowance, CPU pin, machine file, fixture or existing start call changes.
  Tests: thirteen new tests in `pc-core` cover contract-only creation and the
  promised controls, validate's exact content list, replay, session restart,
  lazy records and storage lifetime, safe retry after failed creation, type
  and option refusals, document and row bounds, content size and pin failures,
  and preservation of content, storage and uncertain-write errors, including
  a floppy retention warning followed by a fatal CMOS write, a checkpoint
  warning followed by a fatal write on the same record, and two records
  failing during the same finish. The old description test now expects the
  launch type and retains its unsupported type and identify checks. Two tests
  in `pc-session-tests` cover the typed factory. The comparison with
  `assemble_with_verified_bases` spans five consecutive sessions
  (both pictures, sound, lit drive activity, messages, virtual counters and
  byte-identical disk and CMOS journals, with a CMOS value incremented by
  each boot). Its synthetic guest reads two sectors before writing distinct
  full-sector patterns, then reads them back; both paths' guest buffers and
  command results are checked. Subsequent boots read the previous writes,
  including after retention prunes to three generations; scans check the
  retained generation count, anchor and sector contents. The other test
  appends and inserts a verified host floppy. A new test in
  `platforms` selects the compiled-in PC and starts a launch from
  `core_launch::Parts`, boxed through `start` and typed through `start_with`;
  it also exercises the reset promised by the description. These are tests
  only: image run, desktop Start and `pc-image::launch` remain later steps.
  MEASURED: restoring the previous factory refusal (`Unsupported`) makes all
  thirteen new tests fail (ten core, two session comparisons, one parts start).
  Separate mutations that admit repeated keys, omit VGA routing, or discard
  the original contract error latch fail their targeted tests; the last
  catches content errors, uncertain writes and held catalogue records. Each
  source was restored with a plain copy and touched before final verification.
  The platform-id scan ([`image-store/tests/platform_ids.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/tests/platform_ids.rs), outside the file
  list) gains two narrow entries: the core's exact check of its current id,
  beside the other cores' checks, and the new test that refuses the retired
  alias. MEASURED: the scan failed on precisely these two lines before the
  entries. This changes no alias handling and no host-guard allowance.

**S10. `pc-image::launch`: the store's side as objects.** **Done.**

- **Scope.** §14.2.4: the content plan (today's `SessionTransfer` content
  plus the firmware and CMOS receipts), the storage plan (record name to
  journal file, over the file-backed record of `upc-dfs.35`, with the
  journal's lock `FILE.lock`), and `prepare` functions for an image's
  workspace, a machine and a copy that return a `core_launch::Prepared`.
  The zero-rooted and in-memory floppy bases have no file; their readers
  are made on the machine's thread from parts that are `Send`. The old
  `PreparedSession` stays beside it until S14.
- **Files.** [`crates/pc-image/src/launch.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/launch.rs) (new), `workspace.rs`,
  `machine.rs`, `Cargo.toml` (`core-launch`, `unemu-core-api`).
- **Depends on.** S7, S8, `upc-dfs.35`.
- **MUST PROVE.** In `pc-session-tests`: a stored image started through
  `core_launch::start` and through the old path gives the same session, and
  the journal files in the workspace are byte-equal. A session written by
  the old path opens through the new one and the other way round. A
  running machine and `upcli media rollback` on its journal exclude each
  other. A second start of a running workspace is refused with today's
  sentence. A base that changes after verification fails the run with
  `Content`, not a wrong read. The content source refuses a digest or a
  size the launch does not list.


- **As landed.** `pc_image::launch` hands `core_launch::Prepared::from_parts`
  the edited launch document, a `ContentPlan` and a `StoragePlan`, and returns
  the pinned image digest alongside it (`launch::Prepared`). Its entry points
  are `prepare_workspace`, `prepare_pinned`, `prepare_machine`, `prepare_copy`
  and `resume_copy`; `from_session` freezes a session after catalogue editors
  have run. Machine preparation returns pending inserts for the caller to
  confirm after successful creation. Workspace ownership remains the caller's
  through the run and finish. No command or desktop start changed.
  The content plan reuses `SessionTransfer`'s verified file descriptors and
  metadata, decoded floppy bytes and sparse zero-chain parts, plus the
  launch's firmware and CMOS snapshots. Every row is checked against its
  receipt (full SHA-256 and length); byte snapshots are hashed, file and zero
  receipts retain the full verification already done during preparation.
  No large disk hash is skipped. Equal flat identities share a source only
  after every row was checked. Every start opens the plan anew on the machine
  thread; it refuses unlisted digest/size pairs and builds readers there.
  File readers retain change checks; changes after preparation or during a
  run are `Content`, out-of-bounds reads are `Usage`, other host I/O failures
  are `Storage`. Zero-rooted disks stay sparse and unmaterialized.
  The storage plan maps only the launch's record names to their existing
  journal paths and base identities, with file metadata for alias protection.
  It opens no journal until `Storage::open`, so uninserted catalogue records
  stay free. `JournalFile` owns the existing `FILE.lock` until the returned
  record is released. A held record is `Usage` with the unchanged journal
  sentence, `lock: journal is locked by another session or tool`; the old
  assembler still adds its mount context and confirmed-sequence suffix.
  Header-creation failures through records keep the wording difference
  documented in §12.10 step 9. Base files, journal format, retention and
  existing start functions are unchanged.
  Tests: two unit tests cover `Send` and each byte/file receipt's digest and
  length (firmware, CMOS and floppy), including failure before a core requests
  content. Seven integration tests in `pc-session-tests` cover the stored path.
  The five-session comparison reuses the factory comparison's assertions for
  both frames, sound, drive activity, messages, virtual counters, complete
  sector patterns and command results. Each workspace alternates the old and
  contract paths, so old journals mount through the contract and vice versa,
  including retention to three generations; disk and CMOS journals remain
  byte-equal after every session. Other tests cover boxed and typed starts,
  repeated starts of one prepared value, mutual exclusion with the rollback
  tool (the same `JournalTool` used by `upcli media rollback`), lock release,
  exact content requests and arbitrary byte offsets, changed verified files,
  compressed floppies and zero disks across a thread transfer, every prepare
  helper, machine settings and copy isolation, and external kept-catalogue
  journal paths without modifying their bases.
  MEASURED: all nine launch integration tests (seven new, two reused) and
  both plan unit tests pass. Removing the new public module makes the new
  integration tests fail to compile. Four behavior mutations fail their
  targeted tests: omit the receipt comparison, accept an unlisted size,
  redirect a journal to another path, or classify a changed base as `Storage`.
  Every mutated source was restored with a plain copy and touched before the
  final checks. The dependency additions are `core-launch` for `pc-image`
  and the comparison crate, and `toml` for the latter's fixture; no new
  external crate, host-guard allowance, CPU pin, machine file or fixture.
  MEASURED final gate: 4,305 workspace/golden tests pass; 140 fail at
  sandbox loopback binds (OS error 1) and one at the unsupported host clipboard.
  All 894 release device tests and the doctest pass. Formatting, publication
  and host guards, both clippy runs and rustdoc pass. Cargo-deny is left to
  the supervisor, as the sandbox brief requires.

**S11. `image run` and `image machine run` through the contract.** **Done.**

- **Scope.** In `cli-pc` still. Both commands start with
  `pc_image::launch` and `core_launch::start_with`, run with
  `RunLimit::steps(max_steps)`, call `finish`, and print the machine's
  messages and the finish warnings to standard error.
  Failed runs keep the old PC-side `pc_harness::finish_persistence` path,
  because contract §3.9 forbids `finish` after `Status::Failed` and offers
  no operation to persist that state.
- **Files.** [`crates/cli-pc/src/image.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/src/image.rs), [`image/machine.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/src/image/machine.rs); their
  tests; `backlog/docs/commands.md`.
- **Depends on.** S9, S10.
- **MUST PROVE.** **Stated behaviour change**, listed in the bead and in
  `commands.md` (§14.6 Q5): the last field of the result line is the
  contract's stop; guest resets are performed and reported by the core
  (the old assembly loop already continued after them); no idle limit.
  Everything else is byte-identical over the invocations of §7.5 step 4
  that use the two commands: `pinned-image:`, `inserted:`, the workspace's
  files, the exit codes. `image_commands`, `image_machine_commands` and
  `platform_alias` pass with only the stop word changed where a test names
  it. A run of the FreeDOS fixture to the same step count leaves journals
  byte-equal to the old path's.

- **As landed.** Both commands remain in `cli-pc`. A workspace uses
  `pc_image::launch::prepare_workspace`; a machine, including `image run`
  on its workspace id, uses `prepare_machine`. Both print the pinned image,
  call `core_launch::start_with` with `PcCore::create_machine`, run the
  typed PC through `Machine` with
  `RunLimit::steps(max_steps)`, call `finish`, and drain messages and finish
  warnings to standard error before the result line. Machine confirmation
  and `inserted:` lines still follow successful creation; the owner stays
  alive through finish. A run failure still saves CMOS and closes journals
  through `PcMachine::assembled`, preserving the old commands' behaviour
  without calling contract `finish` on a failed machine. A persistence error
  takes precedence over a run error, as before. Warning-output failures are
  ignored, as in the old helper. No command moves to `cli-core`, no
  existing start API is removed, and no desktop caller changes.
  `commands.md` states the contract's stop word and removal of the idle
  cutoff, and explains that the core now performs and reports guest resets;
  continuation after a reset was already the old loop's behaviour.
  The PC core is selected directly in this PC command; there is no
  `Cores::for_platform` lookup here. Provider validation still accepts the
  retired store spelling, while the launch document uses the canonical id.
  S12 must canonicalize a manifest id before its exact core lookup.
  Files outside the original code list include `pc-core/src/mount.rs`
  (restore journal-open diagnostics), `pc-core/src/machine.rs` (retain the
  typed guest failure for the CLI source chain), [`cli-pc/Cargo.toml`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/Cargo.toml) and
  `Cargo.lock`: three existing workspace dependencies (`core-launch`,
  `unemu-core-api`, `unemu-pc-core`); the architecture map lists them.
  Tests add contract-stop assertions to the existing workspace and machine
  command tests, preserving all earlier assertions. Both assertions fail
  against the old command implementations. Three unit tests cover the exact
  step cap without the idle cutoff, reset continuation and message draining,
  and preservation of a guest error after persistence. Additional review
  regressions compare changed CMOS and sealed journal bytes after a CPU
  refusal against the old path, pin the old pending-insert directory
  diagnostic, and check that warning-output failures are ignored. Mutating
  the step limit or discarding messages makes the corresponding test fail.
  Mutations are restored with plain copies and
  the sources touched before final checks.
  A new ignored FreeDOS comparison runs two successive 10,000,000-step
  sessions through the old assembler, both CLI commands and a typed launch.
  Workspace files, machine journal bytes and virtual counters match:
  8,800,473 retired instructions, 10,000,000 steps and VClock 16,250,000
  on each boot. These counts are measured from the old path and compared
  directly with the typed factory. Existing golden and session assertions
  are unchanged.
  MEASURED: a temporary probe records all 131 calls made by `image_commands`,
  `image_machine_commands` and `platform_alias` before and after. After
  normalizing temporary roots, host last-run text and the stop word, 130
  outcomes are identical. The pending-insert directory diagnostic originally
  lost drive-mount context and the last confirmed sequence. Review restored
  that context and its repeated source sentence at the PC mount, with an
  assertion pinned to the old command's measured output; this is not an
  accepted exception to byte identity.
  A separate set of 11 before/after binary invocations also compares every
  written file and journal byte without normalizing their contents; the
  accepted stop word is the only successful-output difference. The retired-platform run test passes unchanged; its registry
  pull companion needs a loopback bind unavailable in the sandbox.
  MEASURED final gate: 4,309 workspace/golden tests pass; 140 fail at
  sandbox loopback binds (OS error 1), and one at the unsupported host
  clipboard. All 894 release device tests and the doctest pass. Formatting,
  publication and host guards, both clippy checks and rustdoc pass.
  Cargo-deny is left to the supervisor as required by the sandbox brief.

**S12. The two commands move to `cli-core`.** **Done.**

- **Scope.** §14.2.7. `CliPlatform::prepare_run` and the
  machine-ownership hook; the neutral command and its usage lines;
  `cli_core::run` takes the `Cores`; the PC implements the hook with S10's
  functions. Last and droppable (§14.6 Q4): the repository split does not
  need it, since `cli-pc` goes to the front-end repository too.
- **Files.** [`crates/cli-core/src/lib.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-core/src/lib.rs), `image.rs`, `Cargo.toml`;
  [`crates/cli-pc/src/lib.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/src/lib.rs), `image.rs`, [`image/machine.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/cli-pc/src/image/machine.rs);
  `crates/upcli/src/**`; the toy platform of `cli-core`'s tests.
- **Depends on.** S11.
- **MUST PROVE.** `command_line_pin` passes unchanged: the usage text and
  the parse of every pinned line, the two refusals of `image run` among
  them. The output of S11's invocations is byte-identical. In `cli-core`'s
  tests the toy platform with the test core runs `image run` end to end
  with no PC crate in the build. Layer proof; a planted
  `cli-core → pc-image` dependency still fails.

- **As landed.** `cli-core::image` owns both parsed commands, their run
  loop, pinned/result lines, message draining and finish warnings.
  `cli_core::run` receives the binary's `Cores`; the command canonicalizes
  the selected manifest's platform before `Cores::for_platform`.
  `CliPlatform::own_run(store, request, errors)` returns an optional
  `OwnedRun` (the workspace owner, selected descriptor and whether it is a
  machine). The default owns a new tagged workspace of that platform;
  the PC implementation retains its pinned-session selection, machine
  migration and diagnostics, machine/tag checks and `machine::own`.
  Ownership lasts through creation, confirmation, run and finish. This
  broader ownership hook keeps the PC session schema out of neutral code
  and still resumes a pin whose tag has disappeared.
  `CliPlatform::prepare_run(owner, request, core)` returns `StoredRun`:
  a `core_launch::Prepared`, pinned digest, post-creation confirmation
  callback and starter. Its default refuses with "this platform's images
  do not run from the command line yet". The PC uses S10's
  `prepare_workspace` and `prepare_machine`; confirmation and `inserted:`
  lines still follow successful creation.
  The default starter is `core_launch::start`. The PC supplies a typed
  `start_with` adapter exposing `Machine` and a failed-run operation:
  preserve the typed guest error and call `pc_harness::finish_persistence`.
  Neutral code calls contract `finish` only after a successful run, drains
  messages in either case and preserves persistence-error precedence and
  best-effort warning output. This small CLI-only adapter is necessary to
  retain S11's failed-run behavior without adding a downcast or operation
  to the core contract. The old stored-session start APIs remain for S14.
  The usage lines keep their words and positions; two slots split the PC
  machine/media block around the neutral machine-run line. The `upcli`
  parser's Debug implementation preserves the historical platform nesting
  of these two commands, because the unchanged parse pin records it;
  their actual parsed values and execution belong to `cli-core`.
  Mechanical changes outside the code list: the `cli-pc::command` runner
  and the two neutral journal test runners pass `Cores`; `Cargo.lock`
  records existing dependencies. A test-only `pc-image/test-support`
  dependency in `cli-pc` permits the synthetic failed-run hook regression.
  No external package, boundary allowance, core, fixture or machine file
  changes.
  MEASURED: the unchanged `command_line_pin` passes all four tests (usage,
  About, shell words and all 252 pinned parses, including both `image run`
  refusals). Repeating S11's three existing test binaries records 131 calls
  before and after: all 131 outcomes are equal after normalizing temporary
  roots and host last-run text, with no stop-word normalization. Repeating
  its 11 binary invocations also gives equal stdout, stderr, exit codes and
  complete written-file SHA-256 inventories, including pending insertion and
  the 10,000,000-step FreeDOS run; file contents are not normalized.
  The toy platform's end-to-end test uses the test core alone: a stored
  config and content reach `create`, confirmation holds ownership, exact
  step limits and power-off reach the result line, messages reach stderr,
  and finish durably saves the battery record across two starts. Its build
  closure has no PC crate. The new PC hook regression compares a CPU
  refusal's complete diagnostic chain and changed CMOS journal bytes with
  the old assembly path, including a sealed generation. Mutations that
  leave `image run` to platforms, omit canonicalization, replace the typed
  starter, or omit failed-run persistence each fail their targeted test.
  Every source is restored with a plain copy and touched before the gate.
  The layer check passes with its existing allowances and rejects a live
  planted `cli-core → pc-image` dependency; all 42 boundary self-tests pass.
  MEASURED final gate: 4,319 workspace/golden tests pass; 140 fail at sandbox
  loopback binds (OS error 1), and one at the unsupported host clipboard.
  All 894 release device tests and the doctest pass. Formatting, publication
  and host guards, both clippy checks and rustdoc pass. Cargo-deny remains
  supervisor-owned, as the sandbox brief requires.

**S13. The desktop's Start through `core_launch`.** **Done.**

- **Scope.** §14.2.6: the worker's three start flows send a
  `core_launch::Prepared`; the shell starts with `start_with` and
  `PcCore::create_machine`; `Presenter::from_machine`; the catalogue model
  from `PcMachine::config()`; the two options from the application's
  arguments. The provider checks firmware before it hands the launch over,
  as today, so the Missing firmware hint still comes from the worker.
- **Files.** [`crates/pc-library/src/library_worker.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-library/src/library_worker.rs);
  [`crates/pc-ui/src/library_view.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-ui/src/library_view.rs), `library_worker.rs`;
  [`crates/window/src/shell.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/shell.rs), `main.rs`; [`crates/pc-session-tests`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-session-tests).
- **Depends on.** S9, S10; after S7.
- **MUST PROVE.** The shell's and the worker's tests pass with changes
  only where a test names `SessionTransfer` (listed in the bead). The
  session comparison of `pc-session-tests` for a library Start. By hand,
  once: Start of an image, of a machine with a pending insert, of a
  Gaming-mode copy and its power cycle; Insert… while it runs; Stop; Save
  as new image…. The frame digest of a session with two display cards is
  unchanged (MEASURED).

- **As landed.** The worker's image, machine and Gaming-copy starts send a
  `DesktopLaunch` containing `pc_image::launch::Prepared`. All three freeze
  the session only after their created-disk and kept-catalogue restoration;
  the machine's pending inserts still take precedence and are confirmed only
  after `Request::Assembled`. Firmware checks, workspace ownership, recovery,
  StartCatalogue, notes and stop/publication handling stay on the worker.
  The shell appends `allow-unlisted-firmware` and `speaker-trace` from its
  arguments and calls `core_launch::start_with` with `PcCore::create_machine`.
  `Presenter::from_machine` derives its catalogue and media config from
  `PcMachine::config()` and selects the requested card's display id from the
  description: the factory routes MDA before VGA, while the desktop still
  selects the first display card in hardware order. Machine-file starts keep
  `from_assembled`, which delegates to the same presenter constructor.
  A necessary desktop-only addition accompanies the prepared launch: the host
  image/journal bindings and original labels of each drive. These
  replace logical keys only in the presenter's host media model and display
  names, never in the core's config or mounted content. Kept-catalogue capture
  distinguishes copied host disks from the image's own disks by those paths;
  losing them made an unchanged Stop/Start regression discard a restored host
  disk at the next restart. Media tools also need the real journal paths.
  A test-support-only snapshot of the worker's original bindings preserves
  every existing lifecycle assertion without starting a second machine.
  No crate, dependency, CPU pin, stored format, machine file or fixture changes;
  the old start API remains for S14. No CLI code changes.
  Four new headless tests cover a real worker's library Start against the
  verified-file path over two sessions (both displays' exact pixels, sound,
  drive state, messages, virtual and retired counts, finish results and
  byte-equal disk/CMOS journals); reopening while a finished machine stays
  alive; dropping an unfinished machine and reopening; a factory presenter
  selecting VGA when both cards exist in VGA/MDA hardware order, with a
  different letter on each card after 100,000 steps: the old single-display
  route and the factory have the same VGA frame SHA-256
  `1e388f1e88e0acec0e0af4344b3211ee6a14cccad8f6864ed2db6559bfa544db`,
  exact pixels, stop and retired count; created and restored kept disks
  through the shell, Stop/Start, Save as new image, a Gaming copy and its power
  cycle, immutable base bytes and the existing journal file. The shell test runs egui and
  AccessKit without a native window. Existing pending-insert, Insert while
  running, Save as new image and firmware-readiness tests remain unchanged.
  The requested manual GUI exercise is replaced by these headless tests,
  as required by the worker brief; no windows or screenshots were used.
  MEASURED: four behavior mutations each fail at runtime: freezing before
  catalogue restoration, omitting host bindings, forcing display id 0, and
  dropping the speaker-trace option. Sources were restored with plain copies
  and touched before final verification. The unfinished-drop test adds
  coverage of the existing storage lifetime contract.

**S14. The old path goes.** **Done, with live helpers retained.**

- **Scope.** Remove `SessionTransfer`, `PreparedSession::machine` and the
  path rewriting of S6, `pc_image::resolve`, `workspace::start` and
  `start_new` if nothing calls them. Update the comments that say "until
  `PcCore::create` can read a stored config" (`pc-core`), §4 ("Stage B
  landed", with the deviations), the launch row of §8.1.4, §9,
  `architecture.md`, `status.md`.
- **Files.** [`crates/pc-image/src/workspace.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/workspace.rs), `provider.rs`;
  `crates/pc-core/src/core.rs`, `machine.rs`; the docs above.
- **Depends on.** S11, S13, and S12 if it is done.
- **MUST PROVE.** `grep`: outside `machine-file`, `pc-harness` and tests,
  no front-end crate calls `assemble_with_verified_bases`. The full gate.
  The test list loses only the tests of the removed functions, each named
  in the bead with the test that covers its case on the new path.

- **As landed.** `pc_image::resolve`, `workspace::start` and `start_new` are
  removed: test-only callers do not justify a production API. Their tests keep
  their names and assertions. Hardware, content and publication inspection now
  reads the retained prepared host-path view; workspace lifecycle checks use
  `workspace::prepare_start`; boots use `pc_image::launch`, `core_launch` and
  `PcCore`. No test is deleted, so the per-crate test-list diff is empty.
  Independent A/B reference paths remain in the CLI failed-run diagnostic test,
  the CLI FreeDOS comparison, the window's two-display presenter comparison,
  the imported BIOS versus machine-file checkpoint, and the session equivalence
  tests. The CLI template POST, machine FreeDOS persistence, provider retired-count,
  desktop Create/POST, compressed mount, allocation and verification-count tests
  all boot through the factory.
- **Explicit deviation.** `SessionTransfer`, `PreparedSession::machine` and
  `provider::lower_session` remain because they still serve production.
  `launch::from_session` consumes the transfer's verified descriptors, memory
  snapshots and sparse zero-chain parts. The host-path view and its re-lowering
  serve pending inserts, pending CMOS resets, created disks, catalogue editors
  and `DesktopLaunch::freeze`'s host media bindings. Removing the view would lose
  bindings; removing re-lowering would leave transactional edits stale. The core
  mounts logical keys through content/storage objects, not through this view.
  The transfer can go once launch plans capture verified content directly.
  The view and rewriting can go once the PC tools and host bindings consume
  launch identities directly instead of a path-based `MachineConfig`.
  S11/S12's typed failed-run persistence, S13's host bindings and `upcli`'s pinned
  Debug output remain. This completes §9 item 2 without claiming that the PC
  library worker or every compatibility API is neutral.

### 14.4. What stage B needs from steps 9 and 10 of §12.10

Neither `upc-dfs.35` nor `upc-dfs.36` is designed beyond its row. For S9
and S10 to be possible:

1. **Step 9** offers a file-backed `Record` over a given path with the
   journal's lock (`FILE.lock`, §14.6 Q8), usable from `pc-image`, not
   only path constructors.
2. **Step 10's seam speaks the contract's types** (`ContentReader`,
   `Record`) or types `assembly` owns, so that `unemu-pc-core` implements
   it without naming `media`. Otherwise S9 waits for the `journal` crate of
   step 11, because an `emulation` crate may not depend on `media` and the
   allow list only shrinks.
3. **The mount lives as long as the machine**, since a catalogue's journal
   opens at insert, and a machine made by `create` can still take a medium
   the provider appends by path until `add_medium` replaces that.

### 14.5. Where the steps fit

- **S1 to S8 need no step of §12.10** and start now, beside steps 1 to 7
  (§14.6 Q7). Two workers can hold S1 to S7 and S8 at once.
- **S9 and S10 need steps 9 and 10** (`upc-dfs.35`, `upc-dfs.36`), and
  S9 the `journal` crate of step 11 unless §14.4 item 2 holds. None needs
  step 12 (firmware discovery): `create` gets firmware as content.
- **S11 to S14 follow S9 and S10.** After S10, S11 to S12 and S13 are two
  tracks.
- In §12.10's order, the second half of stage B is a row after step 10;
  it does not wait for step 12.
- §9's item 2 is met by S14. Stage B needs no contract change; it is the
  first real use of `Storage`, `Record` and `validate` by the PC, so a gap
  it finds becomes contract 0.3 by §13.1's protocol, with unemunes
  re-pinning.
- Stage B originally kept launch documents transient. Portable prepared
  launches may now be stored (content-bindings § What may now be stored);
  session-augmented launches remain transient. Native zero media have core
  compatibility rows alongside the existing stored-image vocabulary.

### 14.6. Decided by the maintainer on 2026-11-19

Each question was answered with its recommendation. The options are kept
for the record.

**Q1. What document does `PcCore::create` read?** (a) **A launch document
the PC provider writes**: the stored schema plus the `content` and
`record` tables; the stored format does not change. Rejected: (b) a new
stored schema with sizes and flat digests (a format change during the
freeze, with compatibility rows); (c) a wider contract (content without a
size, or descriptors) as 0.3. (b) stays possible later without changing
`create`'s shape.

**Q2. Where does the codec live?** (a) **One new `emulation` crate,
`pc-config`**, with the machine schema, its lowering, the image config and
the launch document. Rejected: (b) inside `unemu-pc-core`, which would
make `pc-image` and `machine-file` link the rasteriser, the fonts and the
keyboard to validate a config; (c) two crates.

**Q3. How does the desktop keep its PC machine after `create`?** (a)
**`PcCore::create_machine` returns a `PcMachine`, and
`core_launch::start_with` lets the application make it from the same
request**; the desktop keeps a typed `PcMachine` in its `Presenter`.
Rejected: (b) a downcast in the contract (`Machine::as_any`), as 0.3; (c)
stored PC images in the generic player, dropping the PC's menus and
dialogs until `add_medium` and settings-as-data exist.

**Q4. Does `image run` move to `cli-core` now?** (a) **Yes, as §12.9
says, with one start hook on `CliPlatform` — last (S12) and droppable.**
Rejected: (b) run through the contract (S11) and move when a second
platform has stored images. If S12 is dropped, S14 depends on S11 and S13
only.

**Q5. `image run`'s result line.** (a) **The contract's stop**, with the
change stated in `commands.md`. Rejected: (b) today's words and stops by
two more run-only PC options (an idle limit, stop at a guest reset), which
would put PC run policy into the core's description for one command.

**Q6. Decision-30 after the split.** (a) **Amend decision-30 before the
repositories separate**; stage B changes nothing here. Rejected: (b) move
the PC rows to `pc-config` and take the PC's level from
`CoreDescription::version` within stage B, which settles half a question.
What needs the amendment: the level a build compares with is
`image-store`'s own package version ([`crates/image-store/src/compat.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/image-store/src/compat.rs)),
which after the split is the front end's; and the table mixes rows the
core reads (the PC config's keys and vocabulary, [`crates/pc-image/src/compat.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/compat.rs))
with rows the front end reads (envelope media types, compressed layers,
media-set and firmware-package keys). The decision record carries a note.

**Q7. When does stage B start?** (a) **S1 to S8 now, beside steps 1 to 7
of §12.10; S9 on after `upc-dfs.35` and `upc-dfs.36`**, with the orderings
of §14.3. Rejected: (b) all of it after step 12.

**Q8. The journal lock's name** (for `upc-dfs.35` and `upc-dfs.37`). (a)
**The journal keeps `FILE.lock` everywhere, and `unemu-core-files` learns
it for the journal's records** (a patch release there): no migration, and
old and new programs exclude each other. Rejected: (b) journals move to
`FileStorage`'s `FILE_lock` with a one-time rename.

### 14.7. Edits to earlier sections

This section corrected the text above in these places, each marked
"(corrected 2026-11-19, stage B plan)":

1. **The status paragraph at the top** names §14.
2. **§3.4**: for the PC, `create` reads the launch document, not the
   stored config as it is.
3. **§4**: the codec's file list ([`disk.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/disk.rs) and [`zero.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/pc-image/src/zero.rs) stay in
   `pc-image`), and the stage B item points to §14 and names
   `create_machine`, since the contract's `Machine` has no downcast.
4. **§5**: the PC's store-side step before `create`; `image run` moves
   with stage B.
5. **§7.3** and **§9**: `pc-config` in the `emulation` row and in the PC
   core repository; §9 item 2 made precise; decision-30's amendment before
   the repositories separate.
6. **§12.5**: the lock a file-backed storage takes for a journal.
7. **§12.9**: the PC's storage is the provider's own, not `FileStorage`
   over the workspace; `image run` is not a pure move.
8. **§12.10** ("needs steps 8 to 12") and **§12.11**: stage B needs steps
   9 and 10, not 12.
9. **§13.3**: the library entry and the typed `PcMachine`.
10. **§13.4**: a `ConfigValidator` over `Core::validate` does not fit the
    PC; a platform may need a store-side step.
11. **§13.7**: the journal's lock name.
