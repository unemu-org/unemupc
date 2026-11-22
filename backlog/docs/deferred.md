# Deferred

Things **deliberately not implemented**. If something seems missing and is listed here, don't add it unprompted — there's a reason. Each entry: what, why deferred, when to revisit.

## Format

```markdown
### <Feature / behavior>

- **Why deferred:** <one paragraph>
- **Revisit when:** <trigger condition>
- **Tracked in:** <task-N, if any>
```

A20 gating is implemented: the board's 8042/port 92h line drives CPU physical
address bit 20 and survives CPU-only reset. See
[the board contract](design/machine-config.md#a20-line-and-system-control-port-a).

## Entries

### Open AT-class (80286) system BIOS

- **Why deferred:** No usable open 286 AT BIOS was identified in the candidate
  survey. Legacy Bochs `rombios` was rejected; a proper port is **ASSUMED** to
  take about 2–4 weeks for CPU/runtime, video, A20/protected-mode services and
  boot qualification. Keep Kiselev with an explicit display switch for open
  goldens and measure AT fidelity through the user-firmware lane; see
  [open AT BIOS candidates](design/firmware-and-machine-classes.md#24-open-at-bios-candidates).
- **Revisit when:** Goldens need AT-faithful firmware behaviour that Kiselev
  cannot provide, or a maintainer asks.
- **Tracked in:** upc-6le.37.
- **Resolved 2026-11-15:** not by a port but by the project's own fork of the
  Kiselev BIOS, unemu-at-bios, now the system BIOS of the open AT machines;
  see [open AT BIOS candidates](design/firmware-and-machine-classes.md#24-open-at-bios-candidates)
  and `vendor/unemu-at-bios/README.md`. Its remaining AT gaps are that
  repository's own work.

### Protected mode, paging (386+), APIC, PCI

- **Why deferred:** Target is IBM PC/AT-class (80286). The 286 has no paging;
  protected mode is not needed for real-mode boot. APIC/PCI are 386+/later-era.
- **Revisit when:** Phase B (protected mode, paging) and phase C (PCI) of
  decision-16, which chose the later machine generations. APIC stays out: the
  target span runs on the 8259 pair.
- **Tracked in:** decision-1, decision-16; epics `protected-mode` and `win9x-base`.

### `RegionKind::Rom` in x86jit

- **Why deferred:** BIOS stub can load into `Ram` by convention initially.
- **Revisit when:** Faithful ROM write-protection matters (real BIOS blob).
- **Tracked in:** decision-9.

### Graphics beyond serial (VGA text, framebuffer)

- **Why deferred:** First E2E target is headless serial (deterministic goldens).
- **Revisit when:** VGA text landed (task-5). A window is phase A of
  decision-16; a pixel framebuffer is phase C.
- **Tracked in:** task-5; epics `skeleton` and `win9x-base`.

### Real-mode segment-register loads (`mov Sreg, r/m`) and far transfers

- **Why deferred:** x86jit's real mode does not yet lift `mov Sreg, r16`/`pop Sreg`
  or far `jmp`/`call`/`retf` (CS reloads other than `INT`/`IRET`). The emulator's
  boot programs work around this: the machine seeds CS/DS/ES/SS before running.
- **Revisit when:** running a real BIOS/DOS/bootloader, which loads segment
  registers itself — needs these lifted in x86jit (a further real-mode extension).
- **Tracked in:** decision-3 (real-mode scope), task-7 (test386.asm / real BIOS).

### Snapshot / save-restore of a running machine

- **Why deferred:** The machine-config layer makes `MachineConfig` identifiable
  (`Eq`/`Debug`) precisely so a future snapshot feature has a stable machine
  identity to hang off, and the `Device` contract carries `save`/`restore` from
  day one (decision-7). But wiring up whole-machine snapshot/replay is a
  standalone feature, not part of building the config layer — designing it now
  would pre-empt work with no current consumer.
- **Revisit when:** A replay/debug or golden-snapshot workflow needs to freeze
  and restore a running machine.
- **Tracked in:** decision-7, [`design/machine-config.md`](design/machine-config.md).

### Retired Long64 machine skeleton

- **Why removed:** Adoption of instruction-boundary timing intentionally deletes
  `run_serial`, `HELLO_PROGRAM`, the Long64 CLI blob mode/default demo, and
  `golden_serial.rs`. The machine runs step-bounded Real16 CPUs only; the CPU
  adapter's own Long64 unit tests remain. This is not a regression to restore
  and does not justify a CPU-mode axis in `MachineConfig`.
- **Revisit when:** An independently justified machine target requires another
  CPU mode and provides an execution/timing oracle. Long64 is outside the
  current PC/AT-to-Windows-98 goal line.
- **Design:** [`design/machine-config.md`](design/machine-config.md), Non-goals.

### Fixed-disk (XT-IDE / ATA) controller device

- **Why deferred:** The config side of fixed disks is done — `DriveKind::HardDisk`
  is first-class `MachineConfig` data with a named `controller: DeviceId`, a
  controller-local `unit`, and `write: WriteMode`. Validation rejects a hard
  disk until a compatible controller is implemented; an FDC cannot serve it.
  CMOS `hard_disk_types` derivation retains the fixed-disk attachment seam.
  What is missing is the *device* that serves it. The
  first end-to-end target is headless serial and floppy boot; a fixed-disk
  controller has no current consumer, and modelling ATA register semantics is a
  standalone device task, not part of the config layer. The seam is built so
  adding one is a new `Device` plus a `build_ide_drives`-style router, with no new
  assembler architecture (see
  [`design/machine-config.md`](design/machine-config.md), the fixed-disk seam).
- **Revisit when:** A boot flow or guest needs to read or write a hard disk (e.g.
  installing DOS to an HDD image rather than booting a floppy).
- **Tracked in:** [`design/machine-config.md`](design/machine-config.md).

### Real (unmodified) BIOS

- **Why deferred:** Start from a minimal BIOS stub; x86jit models a modern CPU, so
  286-detection quirks would break a period BIOS.
- **Revisit when:** A real BIOS blob is a goal.
- **Tracked in:** decision-10.

### Offline floppy journal tools and retention

- **Implemented:** Explicit journal mode persists floppy writes as command-complete
  sector batches beside an immutable file-backed base; new sessions replay the
  journal. Volatile remains the default, and readonly rejects guest writes.
  Offline `media generations`, `verify`, explicit backed-up tail repair and
  rollback and immutable raw-image export (`media commit`/`compact`) are available
  with shared/exclusive stable locking. Retention atomically replaces a verified
  anchor plus retained history at closed boundaries; the positive count includes
  the anchor. Failure isolation, complete volatile/journal result comparisons,
  rollback through guest DMA and volatile fixture hash/namespace protection have
  executable acceptance evidence in the persistence design appendix. TOML/YAML
  machine-file floppy drive tables accept `write = "journal"` with an explicit
  journal path, alongside Rust `MachineConfig` selection.
- **Still deferred:** Frontend journal-selection flags. The CLI and window
  frontends do not load machine files yet; that integration is in progress.
  The runnable walkthrough uses Rust configuration. Physical power-cut validation,
  Linux storage measurement in this session and support beyond the local Unix
  storage contract remain unmeasured; see the design's Supported host guarantees
  section.
- **Revisit when:** Frontend selection or another host platform is needed.
- **Tracked in:** [`design/persistent-media.md`](design/persistent-media.md),
  §Offline commands and §Retention anchor.

### Layered disk images (base image plus content-addressed layers)

Block-level copy-on-write layers would store only changed sectors and identify
their parent by content hash. A base image, such as a user-supplied Windows 98 SE
installation, plus layers of installed programs would form a linear chain.
Layers cannot be freely combined: two layers built separately on one base both
change the FAT and the Windows registry, so combining them means building one
on top of the other. A discardable per-session top layer would make every run
start from the same disk state; committing it would create a new layer.

Optional recipe layers would replay a deterministic input script (keys, mouse
and media changes at virtual times) on the parent to reproduce the layer byte
for byte. Results could be cached by `(parent hash, recipe hash, CPU core revision)`.
Proprietary bases and layers are never committed to the repository. They live
in the user's private image store and private registry as parts of whole-machine
images; see [`design/machine-images.md`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/backlog/docs/design/machine-images.md), §§4 and 5.
A saved machine state would be tied to the emulator version; disk layers remain
plain data.

Still deferred beyond that design: recipe layers, block-level deduplication
across similar bases, FAT-aware optimisation, payload compression, sector
layers for floppies and merging sibling layers.

- **Why deferred:** No workload needs layered images before Windows 98 SE runs
  in phase C. The floppy journal and the coming hard-disk write-back cover
  persistence needs in the meantime.
- **Revisit when:** The hard-disk write-back is designed, and again when phase C
  needs a reproducible Windows 98 SE disk state for golden tests.
- **Requirement for now:** The write-back format must not preclude layers: keep
  the base read-only and identified by hash, and keep written sectors separate
  from the base. The floppy design already preserves this separation; see
  [`design/persistent-media.md`](design/persistent-media.md), §Context and
  §Header: exactly 128 bytes, and
  [`design/machine-files.md`](design/machine-files.md), §Drives.
- **Tracked in:** upc-18x.5.

### Bridging real hardware: an OPL chip, later real ISA cards

The maintainer asked (2026-11-13) whether a real chip or card could replace an
emulated one. Two cases differ sharply.

An OPL synthesizer is almost write-only: guests write registers and read only
the status port (timer flags, AdLib detection). The emulated OPL3 would keep
answering every read, so guest-visible behaviour and the goldens stay
deterministic. Its register writes, stamped with virtual time, would also be
streamed to a host backend. That backend replays them on a real chip in wall
time behind a short buffer, over USB or a parallel-port board. After a snapshot
restore, the backend reloads the chip from the model's registers. This is a
host-side audio output, like today's sound output, not a new guest device.

A general ISA card needs synchronous port and memory reads, its own IRQs and,
for many cards, 8237 DMA handshakes (DRQ/DACK) in real time. That works only in
an explicit live mode that runs in wall time and stalls on reads; such a mode
has no determinism, no goldens and no snapshots. The bridge would be a `Device`
that forwards accesses. The access time on the bus and the clock domains from
decision-22 map virtual time to wall time.

Transport sets what is feasible:

- USB (tens of microseconds up to 1 ms per round trip) suits only write-mostly
  devices such as the OPL.
- Thunderbolt to PCIe (about a microsecond per access) is the only realistic
  path for synchronous reads on a modern laptop. It means a PCIe enclosure, a
  PCI or PCIe to ISA bridge with a powered ISA backplane (+-5 V, +-12 V), and a
  host PCI driver.
- Off-the-shelf bridges forward I/O, memory and IRQ cycles but rarely ISA DMA.
  DMA cards (Sound Blaster digital audio, GUS) need an FPGA bridge that runs
  the DMA handshake and buffering locally.

The maintainer floated designing that FPGA bridge as a project of our own
(2026-11-13): an FPGA board with a PCIe endpoint, used in a Thunderbolt PCIe
enclosure, with 5 V level shifting to a powered ISA backplane. Its gateware
would:

- drive ISA I/O and memory cycles with real bus timing (wait states, IOCHRDY,
  0WS);
- forward IRQ lines as message interrupts with their edge timestamps;
- implement an 8237-compatible DMA engine on the bridge, which answers DRQ
  with DACK at bus speed from a buffer that it fills and drains from host
  memory by PCIe bus mastering;
- expose one register window per mapped card for the host driver.

The emulator's bridge `Device` would then exchange time-stamped transactions
with the gateware, not single bus cycles, so only synchronous reads stall the
guest.

- **Why deferred:** No workload needs it. The OPL bridge is an optional audio
  output, and a live-mode ISA bridge is a separate hardware project that gives
  up determinism by design.
- **Revisit when:** The maintainer wants real OPL sound (OPL output bridge
  first), owns bridge hardware and wants a live mode, or starts the FPGA bridge
  as a separate hardware project.
- **Requirement for now:** None beyond the existing seams. Devices stay behind
  the `Device` trait, bus accesses carry their time, and audio output stays a
  host-side sink.
- **Tracked in:** not tracked; idea only.
