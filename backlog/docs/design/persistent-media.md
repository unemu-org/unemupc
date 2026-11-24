# Recoverable persistent media

Status: implemented and validated; offline tools, immutable export and retention available.
The maintainer approved the defaults on 2026-11-10.
The version-1 codec and recovery scanner are implemented in `crates/media`;
the host-independent device media contract and its in-memory implementation
are implemented in `crates/device`. The durable host sink and single-writer
lifecycle are implemented in `crates/media`. Assembly now wires all three write
modes, validates journal/sidecar path aliases, replays before execution, and exposes
drive and NVRAM status/checkpoint/finish handles. CLI and window run boundaries check
host failures and explicitly finish successful sessions. Offline generation
listing, verification, explicit tail repair and backed-up rollback are implemented
in the CLI and `crates/media`, including immutable-image export (`commit`/`compact`).
The sink applies the configured positive retention count after durable checkpoints
and session closes, replacing old history with a verified anchor and retained frames.

## Context

Guest writes need to survive a restart without destroying the last usable disk.
Keep the original raw image immutable and append sector replacements to a
separate journal. Close a generation at the end of each session or at an explicit
checkpoint. A user can discard a bad session by returning to an earlier generation.
A generation means a recovery point, not a guarantee that the guest filesystem
is healthy: the user decides which version works.

Evidence from the current implementation:

- **READ (before the media contract landed; historical line numbers):**
  `crates/devices/src/upd765.rs:882–909`: WRITE DATA committed DMA bytes
  into the former `Drive.image` field. A partial final sector was zero-filled to
  512 bytes; a short isolation-test image grew. `:918–935` computed status and
  raised IRQ6 after mutation. An abnormal result could therefore accompany
  committed sectors.
- **READ:** `crates/assembly/src/image.rs:13–22` defines inline, file and blank
  sources. `assemble.rs:147–153,284–318` reads file sources once, resolves them
  to inline bytes, and validates the bytes actually mounted.
- **READ:** `crates/assembly/src/assemble.rs`, `mount_fdc_drives`, routes drives
  by named `controller: DeviceId` and controller-local `unit`. In
  `crates/assembly/src/drive.rs`, `DriveConfig.write` is `WriteMode::Volatile`
  (default), `WriteMode::ReadOnly`, or explicit `WriteMode::Journal`. Readonly
  uses `Drive::new_read_only` and preserves the existing not-writable status;
  journal mounts replay into a sector-media adapter. Volatile and readonly
  create no host output.
- **READ:** [Deferred journal tools and retention](../deferred.md#offline-floppy-journal-tools-and-retention)
  records implemented tools and the remaining frontend/configuration work.
  Append/replay persistence uses the assembly session API below.
- **READ:** [Machine configuration](machine-config.md#single-homes-no-triplication)
  assigns media to top-level drives and composition to assembly. It deliberately
  defines Rust data, without a serialized config format.
- **READ:** `crates/cli/tests/fdc_write_golden.rs:42–48` proves same-run write/read
  behavior. `dos_write_golden.rs:132–136` inspects final in-memory FAT12 bytes;
  it does not establish persistence across assemblies.

Scope: the existing 512-byte-sector floppy formats, ATA hard disks and 64-byte
CMOS images, a single writer per journal, offline recovery tools, and deterministic
headless execution. No CPU change,
new fixed-disk controller, whole-machine snapshot, guest-filesystem transaction,
live recovery service, compression, encryption or qcow2 compatibility is needed.
Default drives remain volatile; persistence tests select private temporary
journals. Fixture paths must never become journal outputs implicitly.

**ASSUMED storage contract:** local storage preserves previously synchronized
bytes when an append is interrupted, supports exclusive locking and atomic
same-directory replacement, and honors the platform durability operations
specified below. A crash may tear, lose or reorder unsynchronized tail writes.
Checksums detect damage; they cannot repair a damaged base, malicious edits,
bit rot in closed history, or hardware that violates synchronization guarantees.
Those cases require a separately retained base/journal backup. This is not a
claim of measured power-loss resilience.

## Options compared

Let `B` be image bytes, `W` sector replacements, `C` completed commands, and `G`
closed generations since the last retention rewrite. Costs below are analytical,
not filesystem benchmarks. Copies mean ordinary full copies, without assuming
filesystem reflinks.

| Design | Crash safety, including torn writes | Disk use | Simplicity and testability | Rollback cost |
| --- | --- | --- | --- | --- |
| Append-only sector journal | Check each frame; replay only complete command batches. Preserve synchronized history and discard an incomplete tail. Generation markers identify recovery points. No live base overwrite. | This format: `128 + 640 × (W + C + G)` bytes. Repeated writes accumulate until retention/compaction. | One forward parser and a bounded command buffer; exhaustive byte-cut and injected-I/O tests are practical. No persistent block index. | Scan to find the generation, then truncate; remount replays the retained prefix. |
| Full-image copy-on-write generations | Write a new full image, synchronize it, publish it atomically and synchronize its directory. A torn unpublished copy is ignored. In-place copying over the current version is unsafe. | About `B × G`, plus a working image and temporary copy. Persisting each command needs a full copy per command; copying only at session end loses that session on a crash. | Simplest recovery representation; whole-image comparisons and publication-failure tests are straightforward. A defensible fallback for tiny disks. | Select a prior full image, effectively constant metadata work plus loading `B` bytes. |
| Block-mapped overlay, qcow-like | Data, allocation maps and generation roots must be published in a consistent order. Torn mapping/root changes need redundant roots or their own journal. | Changed blocks plus maps, allocation metadata and retained old blocks; repeated writes within a generation can reuse space. | More invariants: allocation, map updates, reference ownership and reclamation. Much larger fault-injection surface for these floppy sizes. | Select an old root; reclaiming unreachable blocks is separate work. |

Choose the append-only journal: incremental command persistence is the requested
behavior, without an on-disk block lookup tree. Replay only sector replacements
into a sparse in-memory overlay at mount. For example, a one-sector command
adds two 640-byte frames (1,280 bytes); a full 1.44 MB image copy is 1,474,560
bytes. Heavy repeated overwrites can make the journal larger than full copies;
retention and export are explicit parts of the design.

The qcow comparison is an inference from QEMU's documented allocation tables,
refcounts and snapshots, not a proposal to implement a subset of its format.
See [QEMU, Qcow2 Image File Format](https://www.qemu.org/docs/master/interop/qcow2.html).
The publication and storage assumptions follow the issues described in
[SQLite, Atomic Commit, §§2, 3.7, 3.11 and 9](https://www.sqlite.org/atomiccommit.html).

## Chosen format (byte layout)

Version 1 is a header followed by fixed-size frames. All integers are unsigned
little-endian. Offsets in the tables are inclusive. There is no native Rust
struct serialization, alignment padding, timestamp, pathname, host identifier
or random identifier. SHA-256 fields contain the raw 32-byte digest.

### Header: exactly 128 bytes

| Bytes | Size | Field and required value |
| --- | ---: | --- |
| 0–7 | 8 | Magic: ASCII `UPCFJNL` followed by `00` hex |
| 8–9 | 2 | Version: `1` |
| 10–11 | 2 | Header length: `128` |
| 12–15 | 4 | Sector length: `512` |
| 16–23 | 8 | Exact base-image length in bytes; positive and divisible by 512, or 64 for CMOS |
| 24–55 | 32 | SHA-256 of every original base-image byte, in file order |
| 56–63 | 8 | Anchor generation: `0` initially; positive after retention rewrite |
| 64–67 | 4 | Flags: `0` |
| 68–95 | 28 | Reserved: all zero |
| 96–127 | 32 | SHA-256 of header bytes 0–95 |

Before applying any frame, check the header, image length, configured geometry
length and base digest against the once-verified base identity for this assembly. A
different base, even one byte different at the same size, is a hard mount error.
Never create a fresh journal over an existing invalid file or silently mount it
as volatile. An incomplete header is not a recoverable tail.

### Every frame: exactly 640 bytes

Frame `i`, starting at zero, begins at file offset `128 + 640 × i`. A fixed
stride permits inspection beyond a damaged frame without searching inside
sector data for a magic string. The extra padding trades space for a simpler
parser; this is not a claim that a 640-byte host write is atomic.

| Bytes within frame | Size | Field and required value |
| --- | ---: | --- |
| 0–7 | 8 | Magic: ASCII `UPCFREC` followed by `00` hex |
| 8 | 1 | Kind: `1` SECTOR, `2` COMMAND_END, `3` GENERATION_END |
| 9 | 1 | Flags: `0` |
| 10–11 | 2 | Frame header length: `64` |
| 12–15 | 4 | Payload length: `512` for SECTOR, otherwise `0` |
| 16–23 | 8 | Sequence: starts at `1`, increases by exactly one per frame |
| 24–31 | 8 | Generation id; positive, governed by the grammar below |
| 32–39 | 8 | Target: LBA, sector count, or closing reason, according to kind |
| 40–63 | 24 | Reserved: all zero |
| 64–575 | 512 | Complete replacement sector; all zero for marker frames |
| 576–607 | 32 | Reserved: all zero |
| 608–639 | 32 | SHA-256 of frame bytes 0–607 |

SECTOR's target is a zero-based LBA, strictly below `ceil(base_length / 512)`.
Its bytes replace that whole sector. COMMAND_END's target is the exact number
of immediately preceding SECTOR frames in that command batch. GENERATION_END's
target is a closing reason: `1` session end, `2` explicit checkpoint, `3`
recovered interrupted session, `4` retention anchor. All other encodings,
nonzero reserved bytes and integer overflow are errors. No wraparound is allowed.

### Grammar and generation identity

An ordinary command is one or more SECTOR frames followed by COMMAND_END. All
frames have the same generation id. LBAs within a command are unique and occur
in the controller's transfer order; across commands an LBA may repeat, with the
last replacement winning. The parser buffers at most one image's sector count,
checks each LBA before indexing, and applies replacements only after validating
COMMAND_END. A zero-byte DMA completion emits no command batch. A write of
unchanged bytes still emits the touched sectors.

A generation contains zero or more commands followed by GENERATION_END. The
marker is legal only between commands. After it, the next generation id is
exactly one greater. Empty sessions/checkpoints may close empty generations.
No BEGIN frame is needed: the header and preceding generation end determine
the current generation. A complete command batch without GENERATION_END is
an interrupted generation, not an invalid journal.

For a new journal the immutable base is logical generation `0`, the first run
uses `1`, and sequences start at `1`. A checkpoint or session end closes the
current generation. Each independently mounted media session has its own ids;
a call to `run_bounded`, a cap, a CPU/device reset, or a prompt poll does not
close one. CMOS records append only at CPU reset and finish, never at host-paced
run stops (see § CMOS NVRAM journals). Multiple media have independent journals;
there is no atomic multi-medium checkpoint promise.

After rollback, discarded generation ids and sequence numbers may be reused.
They are addresses within the current journal lineage, not permanent global
identifiers. Tools print the base digest and warn that later generations will
be removed. Retention preserves retained generation ids but renumbers frames.

### Retention anchor

If header `anchor_generation = A > 0`, the file must begin with exactly one
synthetic command describing generation A: list, in increasing LBA order, every
sector whose bytes at A differ from the original base; then COMMAND_END and
GENERATION_END with reason `4`. This one synthetic command may have zero
SECTOR frames. No other zero-sector command or reason-4 marker is legal.
Subsequent generations are A+1, A+2, and so on, using the ordinary grammar.
The anchor must be complete before any part of this file can be mounted.

This retains the full state at the oldest retained recovery point without
changing the base or its hash. An incomplete anchor is a hard error, not an
invitation to mount the original base and lose retained changes. Generation 0
is available to rollback only while the header anchor is zero; after pruning,
the original base remains independently usable with a new journal path.

### Open and tail recovery

1. Acquire the journal's lock, validate the header and base binding, and scan
   forward at fixed frame offsets. Check hashes, encodings, sequences, generation
   grammar, counts and bounds. Remember the offsets after every validated
   COMMAND_END and GENERATION_END. Do not expose pending SECTOR bytes.
2. At EOF, a partial frame or complete SECTOR frames without COMMAND_END are
   an uncommitted tail. A checksum/grammar failure can also be a torn tail.
   At the first failure, inspect later complete frames at the known stride:
   if any independently checksum-valid GENERATION_END frame is present, refuse
   ordinary mount as interior corruption. Do not skip the bad frame and replay
   later data. This check is conservative, including markers with inconsistent ids.
3. With no later intact generation marker and a complete anchor, retain only
   the validated prefix through the last command/generation end (or the initial
   header). Discard the entire pending command and every subsequent byte.
   Report the discarded byte count and last closed generation. Automatic
   tail classification assumes the crash fault model: a damaged final marker
   and a torn append cannot always be distinguished. Verification reports this
   uncertainty; it never labels recovered data as known good.
4. A writable mount truncates to that safe offset and durably synchronizes
   before appending. If complete commands remain after the last generation end,
   replay them and append/synchronize a reason-3 end marker. They become a
   visible recovered generation. Start the new session in the next generation.
   If no complete command remains in the interrupted generation, reuse its id.
5. A read-only verification scan reports the same proposed actions without
   truncating or sealing. A writable recovery failure aborts mount before any
   guest execution. Closed history with detected interior corruption requires
   explicit salvage or restoration, never automatic truncation.

A crash after a fully valid but not yet acknowledged COMMAND_END may preserve
that entire command. Recovery promises a valid prefix, not knowledge of which
sync calls returned before the crash. Checksums provide accidental-damage
detection, not authentication. A checksum-valid marker after corrupted data
does not make that generation usable.

## CMOS NVRAM journals

The AT's battery keeps the Motorola MC146818's 64 bytes and clock running
(Motorola MC146818 datasheet, "RAM" and "Time, Calendar, and Alarm Locations").
This emulator's journal persists only general
RAM 0Eh–3Fh **except 0Fh** (49 bytes). The IBM PC AT Technical Reference,
CMOS shutdown status byte, assigns 0Fh to the transient reset protocol; DOS
extenders and INT 15h AH=87h may change it on every protected-mode exit. It
neither triggers nor appears in replacements and takes the base image's value
at mount. Guest reset still preserves its live value within the session.

Time/date, alarms and registers A–D (00h–0Dh) come from `start` and RTC defaults
every session: a guest DATE/TIME change is not kept. The IBM PC AT Technical
Reference, System Board, **CMOS RAM address map**, defines 32h as the BCD date
century byte, outside the 10h–2Dh checksum range. Every derived AT CMOS layout
initially seeds it from `start.year / 100`; this does not update it on a live
99-to-00 year rollover. Image NVRAM keeps the supplied byte, and journal NVRAM
keeps the persisted byte without replacing it from `start`. If a guest changes
the century, 32h may conflict with `start`'s year; set `start` accordingly.

Reuse version 1, its sink and all offline tools: additionally accept an exact
64-byte base image, pinned by SHA-256 over all 64 original bytes. This is the
only accepted short base: images/journals with a 128-byte RTC are rejected with
`128-byte NVRAM images are not supported yet`; the journal format is unchanged.
It has one logical block at LBA 0. Its SECTOR payload remains 512 bytes: the first 64
bytes hold the store and the remaining 448 must be zero. Bytes 00h–0Dh and 0Fh
stay identical to the base in emitted records and exports. The scanner and sink
both reject changes to these immutable bytes or nonzero padding before replay
or durable output. Retention anchors use the same padded block. Other short
base sizes remain invalid. `media generations`, `verify` (including
`--repair-tail`), `rollback` and `commit`/`compact` use the original 64-byte
base without conversion; exports are 64 bytes.

Append a replacement and COMMAND_END only before a CPU reset and at session
finish, when the persisted bytes differ from the last appended state. Run
stops, prompt polls, instruction caps and window frame slices perform no NVRAM
I/O. Multiple reset records remain in one open generation; finish closes it
once with the session-end reason, including an empty session, just like drives.
NVRAM checkpoints only check that the session is active; they do not append,
close a generation or run retention. Retention runs once at session finish.
After an execution error there is no finish: the next mount seals any complete
open commands with the recovered-session reason (3), as for drives.

A crash between reset/finish boundaries can lose changes since the last such
boundary. Record contents and cadence depend only on guest state at reset and
finish, identically for CLI and window; host pacing cannot add records. No wall
clock, worker thread or timer enters this path.

Finish at a CLI instruction cap or normal window close persists whatever general
RAM holds at that point. If the guest stops partway through a multi-byte setup
update, its checksum bytes 2Eh/2Fh can be stale; the next POST reports a CMOS
checksum error, like power loss on real hardware. Persistence does not repair
the guest's checksum or make a setup update atomic.

Assembly reads and checks the pinned base once before opening any journal,
then replays it before exposing RTC ports. Base mismatch, damaged-tail reports,
reserved `.create-`/`.retain-` names, locking, retention and host-error handling
follow drive journals. NVRAM shares the frontend persistence handles so a host
failure is reported before another run or CPU reset.

Machine files accept `nvram = { image = "cmos.bin", sha256 = "…", write =
"journal", journal = "cmos.upfj", keep_generations = 10 }` (TOML), or the
same mapping in YAML. Journal path is required, retention defaults to 10 and
must be positive. Image/pin/journal override units follow drive images.
`write = "readonly"` is rejected with the exact message
`readonly NVRAM is unsupported: the MC146818 has no write-protect input; use write = "volatile"`
because suppressing POST writes to diagnostic/shutdown RAM breaks the AT.

## Integration (layers, flush points, determinism argument)

### Media behind the drive

Introduce a small hardware-neutral sector-media contract in `device`, an
in-memory implementation, and a host journal adapter owned by `assembly`.
The uPD765's drive owns media through that contract. Its operations provide
sector reads and a batch of complete replacement sectors for one completed
WRITE DATA. The in-memory implementation retains today's zero-read and growth
semantics for short isolation-test media. Normal assembly still validates a
full geometry-sized floppy image.

The journal adapter contains a sparse sector overlay plus an output sink.
It first applies the batch to memory exactly as today, then mirrors the final
sector bytes and COMMAND_END. The device sees no file, pathname, flush policy,
generation, lock, or host error. Write protection remains a controller-visible
medium property and rejects writes before calling this contract.

`Upd765::complete_write` supplies complete 512-byte replacements, including
the zero-filled partial sector. It submits all sectors actually committed,
including those preceding an overrun/no-data/end-of-cylinder result. Persisting
only ST1-successful commands or the unpadded DMA buffer would change behavior.
Reset keeps the mounted media and does not create a generation; device snapshots
continue to exclude media. This does not implement machine snapshot/replay.

### Where bases and journals come from

When it assembles a machine, the assembler asks the machine's mount
(`assembly::Mount`, `crates/assembly/src/mount.rs`) for the bytes of an image
named by path, for the verified base of a journalled disk and for the journal
of a disk or of CMOS RAM, instead of opening them. `FileMount` answers each path
with the file it names, through the journal's path constructors and its
`FILE.lock`; every `assemble*` function but `assemble_mounted` uses it, and so
does a floppy appended by path, which still opens its base itself. No two
journal outputs may share a key, whatever the mount. Another mount answers with the contract's objects: a
`ContentReader` for a base, read on demand and never hashed again, and a
`Record` for a journal, whose bytes are the same version-1 journal
(`assemble_mounted`; core-contract.md §12.5 move 2 and §14.4). The machine keeps
its mount for its life, so a catalogue medium's journal opens through it at each
insert. A medium appended by path is a host file whatever the mount.

### IDE hard disks

ATA drives use the same `JournalAdapter`, version-1 codec, immutable file-backed
base, retention count (default 10), sidecar lock and host failure latch as floppies.
Sector size is 512 bytes and journal geometry length is the image byte length;
assembly validates that the configured CHS capacity matches that image. No IDE
format discriminator or media-specific sink hook is needed. `media generations`,
`verify`, `rollback` and `commit` take the same `--base` and `--journal` arguments;
the offline API derives capacity from the base length, without a floppy preset.
`JournalMedia` shares a read-only verified base and keeps only written sectors
in a deterministic `BTreeMap`. See § Memory and hashing for the bounded-memory
mount, maintenance and immutable-source checks.

ATA-2 §7 WRITE SECTORS (30h/31h), WRITE MULTIPLE (C5h), and §8.4 PIO data-out
supply ordered whole-sector batches. Each sector becomes committed after its
256th word. Normal command completion submits the batch; replacement, reset or
an address failure submits the completed prefix of an interrupted command.
Unlike floppy DMA's zero padding, an incomplete ATA sector is never journaled.
The prefix remains readable while the command is still active.

A completed guest command submits its batch immediately at the guest completion
event. Ordinary run caps, CLI prompt polls, window frame slices and
`check_persistence` only poll host error latches; they never flush an active
command. Running the same complete workload with different host cap splits
therefore produces byte-identical journal files.

Assembly flushes an in-flight completed-sector prefix only at session finish,
before CPU reset, after a run error, or explicitly before a device snapshot or
checkpoint. `AssembledMachine::flush_persistence` prepares a device snapshot or
captures a prefix after an embedder's raw `machine.run_bounded` error.
`PersistenceHandle::checkpoint` and `finish` require the owning machine and flush
themselves, so callers cannot accidentally close a generation without the prefix.
`check_persistence` is a latch-only ordinary-boundary check. There is no
whole-machine snapshot API. Explicit flushes leave PIO buffers/task/IRQ state
intact and may split an in-flight command; partial sectors are never included.
Dropping without a successful finish leaves submitted complete records unsealed
and recoverable on the next mount. An abrupt drop during an active command may
lose its still-buffered prefix.

A host sink failure does not turn the ATA medium readonly: live reads and writes
continue in memory with unchanged guest status. Assembly reports the latched
failure at its boundary and refuses a clean session end. Explicit readonly mode
still rejects writes with ERR/ABRT. Volatile mode creates no journal.

IDE acceptance coverage is in `crates/assembly/tests/ide_persistence.rs`
(single/multiple commands, interrupted prefixes, host pacing independence, snapshot preparation, continued
PIO transfers, guest OUTSW state equivalence and execution-error recovery, retention, rollback and immutable
export), `crates/assembly/src/persistence.rs` (sink failure isolation), and
`crates/machine-file/tests/review.rs` (TOML/YAML paths and defaults).
`crates/cli/tests/ide_journal_golden.rs` boots FreeDOS with the FAT16 fixture as
C:, writes `ZZ`, checks its exact on-media bytes, then boots a fresh assembly
and reads it through `TYPE C:ZZ`. It also exercises all four offline CLI tools
and checks that the base image stays byte-identical. The existing ATA MockBus
contract tests continue to define command batching and readonly ERR/ABRT.

The assembly media loader retains the original `ImageSource::File` metadata
and a `VerifiedBase` handle for journal drives, including removable floppies.
It streams the base hash once, checks geometry and the configured digest, then
shares that identity and open read-only descriptor with binding and replay.
It does not resolve a journal drive into `Inline` bytes. All host paths, journal
ownership and finalization belong to assembly/frontend code, never the generic
machine core.

Machine-file lowering retains the verified base digest in
`DriveConfig.expected_sha256` alongside its file source, even when no explicit
`sha256` pin is present. Assembly compares that digest after its streaming hash
and before opening journals or creating locks. A mismatch names the drive and
both hashes; it takes precedence over an existing journal's base-binding error.
The checked descriptor supplies journal binding, replay and guest reads.
Machine-file synthetic readers refuse journal reads by default through
`ImageReader::read_journal_base`; only file-backed implementations may opt in.
This preserves the immutable file base needed to reopen a journal and to use
offline tools. Volatile/readonly readers retain their verified bytes inline.

### Persist at command completion

Choose synchronous mirroring at each WRITE DATA completion, rather than at a
run boundary. A long session interrupted before the next frontend boundary
must not lose every earlier command. One call through the media contract
groups all sectors of a command; this also bounds replay buffering. No new
device deadlines, wall-clock timers, worker thread or machine scheduling hook
is needed.

For every nonempty completed command, in order:

1. Apply all replacement bytes to the in-memory sparse overlay.
2. Encode and `write_all` the SECTOR frames, followed by COMMAND_END, retrying
   interrupted writes and handling short writes without duplicating bytes.
3. Flush any userspace buffer, then perform a durability synchronization of
   the journal before returning from the adapter. One barrier covers the batch
   and marker. Reordered incomplete batches fail validation on recovery.

Version 1 has a single, fixed durability policy: **every completed write
command**, plus each generation end and each recovery truncation. `flush()`
alone is insufficient. On Unix use a file-data-and-metadata sync (`fsync`, via
`File::sync_all`). On Apple targets, std's `File::sync_all` already issues
`fcntl(F_FULLFSYNC)` and retries interrupted calls; rely on that implementation
rather than issuing a second device-cache flush. Any remaining direct platform
syscalls must retry `Interrupted`. An unsupported durability primitive is a host
error, not a silent downgrade. Apple documents the distinction in
[fsync(2), Description](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html).

Before the first guest instruction, creation writes the header into an unused
same-directory temporary file and synchronizes it. Publish with a hard link to
the final path (which refuses an existing destination), unlink the temporary,
then synchronize the directory. A failure before publication leaves no journal
at the final path; an unpublished temporary is never mounted automatically.
A failure after publication can leave a complete header whose directory entry
is synchronized again on reopen. Under the stable lock, mounts remove the
journal's `<journal>.create-*.tmp` leftovers before synchronizing the directory.
This handles both unpublished orphans and the extra hard link left by a crash
between publication and unlink; journal contents remain authoritative.
Each replacement file is written to an unused temporary name in the same
directory, fully verified and synchronized before atomic publication;
synchronize the containing directory after publication.
Use the corresponding tested namespace/durability operations per supported
platform. Unsupported filesystems/platforms are refused for journal mode.

An explicit `checkpoint()` is accepted only at an existing completed machine
service boundary: drain any completed command, append GENERATION_END, flush
and synchronize, then increment the generation id. The caller may continue.
`finish()` does the same when the embedding frontend explicitly ends the media
session, including a policy-limited but otherwise successful session. It is
fallible and mandatory before reporting persistence success; `Drop` only
releases handles. On execution errors do not label the session clean: leave
its completed commands for recovery. A clean label does not certify filesystem
consistency, even when the guest is at a prompt.

### Implemented host sink boundary

`media::JournalMedia` owns the sparse working overlay and host journal. Assembly
wraps its `(LBA, complete sector)` command batches in `device::SectorMedia`;
this keeps host I/O out of device crates without adding a dependency between the
format crate and device contracts. `JournalMedia::open_verified` takes the shared
verified base, geometry length and a frontend-owned `SinkStatus`. The byte-backed
`open` entry point remains available for CMOS and small callers. Each media session
uses its own status handle. Duplicate LBAs are collapsed to their final bytes,
ordered by their last occurrence, before encoding unique SECTOR frames; memory
applies the original batch in order, matching volatile last-wins behavior.
`checkpoint` closes a generation and continues;
`finish` consumes the session. Dropping without finish releases the journal and
stable sidecar lock without writing a marker. The sink prunes only after a
successfully synchronized checkpoint or session-end marker.

All scanner entry points share validation and classification.
`scan_overlay_reader` uses `Read + Seek` and returns `SparseScan` (evidence plus
an optional sparse overlay); offline verification, rollback, repair and retention
use it. The byte-slice `scan` and `scan_reader` compatibility helpers explicitly
materialize a small image for existing codec callers/tests. They are not used by
file-backed journal mounts or maintenance. Offsets must fit the host's `usize`;
larger journals are refused before replay. Read/seek errors abort mount rather
than authorizing tail truncation.

### Memory and hashing

For a file-backed journal drive, mounting performs exactly one SHA-256 pass over
all base bytes with a 64 KiB buffer. The verified length/digest are reused for
header creation, header binding, replay, retention and prefix verification;
maintenance never hashes the original base again. A configured digest still
must match: large images never bypass integrity checks. An offline tool open
performs its own single verification pass. Export separately hashes its outgoing
stream and rereads the temporary in bounded chunks to verify length and SHA-256
before synchronization and atomic no-clobber publication.

Use positioned reads (`FileExt::read_exact_at` on Unix), rather than mapping the
whole image: this bounds address-space/resident application allocations and
reports read/truncation failures through the host error latch instead of risking
a mapping fault. The verified open descriptor pins the mounted file object;
replacing a pathname never redirects reads to an unverified replacement.
Before and after each base read, including export ranges, compare descriptor
metadata against its verification snapshot: length, mtime and, on Unix, ctime,
including their nanosecond fields. Check it around verification/replay, before
writes and closes, and at frontend status/run boundaries too. Overlay hits also
check the parent. Any detected mutation is a hard host error; appends stop and
the frontend stops at its next cooperation boundary. Controller interfaces have
no I/O error result, so a failed read returns a placeholder zero sector only
with the host failure latched; it never authorizes continued successful execution.
Metadata is an integrity check only, never a clock or virtual-time input.
Unix ctime changes from chmod, extended attributes or hard-link operations also
stop a session, even if its content bytes remain identical.

**ASSUMED immutability contract:** callers must not modify an active base.
Metadata checks detect ordinary edits/truncations, including changes to sectors
already shadowed by the overlay. They are not an adversarial filesystem change
monitor: an actor able to change bytes while preserving all observed metadata,
or a filesystem whose timestamp resolution misses an edit, can evade this policy.
The unchanged-file assumption is explicit; a complete rehash per sector is not
performed. On non-Unix targets the metadata comparison uses length and mtime;
the durable journal sink itself remains Unix-only.

`SectorSource` is the single immutable-parent interface (`byte_len`, sector/range
reads, integrity check). Both `VerifiedBase` and `SparseOverlay` implement it.
A sparse top owns an `Rc<dyn SectorSource>` parent plus an ordered map of last
written sectors, so a future hash-linked chain can replace base → writable top
with base → immutable layer → immutable layer → writable top. No layer format,
base tags, deduplication or FAT-specific optimization is implemented here.
Unchanged writes still emit their original journal records and may occupy one
map entry; retention emits only entries differing from the immutable base.

Let D be distinct written sectors, P the largest pending command and G generation
summaries. Media memory is O(512 × (D + P) + G + 64 KiB), independent of
base length and payload history; configured machine RAM/ROM allocations are
additional. Retention holds a constant number of sparse
prefix states while comparing each retained recovery point, then streams its
canonical anchor and old frames. Finish moves the overlay to the readable closed
session; it never clones a disk. Commit streams base plus overlay into a new
file in 64 KiB ranges, verifies the temporary, and leaves both inputs untouched.
Byte-backed callers still own their small base bytes; the explicit `materialize`
capture helper allocates a complete image only when requested.

This bound includes disk loading through `load_for_assembly` and
`LoadedMachine::assemble`, as used by CLI/window launches. Lowering streams disk
length/SHA-256 through `ImageReader::verify_journal_base`, retains the verified
read-only descriptors in `LoadedMachine`, and passes them separately from the
declarative configuration into assembly. Assembly rechecks metadata, geometry
and configured pins without rehashing. The configuration-only `load` API returns
paths/digests and drops descriptors; a later independent assembly verifies them
anew. Volatile/readonly images retain their existing inline loading policy; CMOS
uses a bounded small byte-backed loader. Invalid geometry in direct assembly is
rejected from file metadata before hashing.

A removable medium shares its verified open descriptor across reinserts within
one assembly, checking metadata rather than hashing that same descriptor again.

Tests in `crates/media/tests/sparse_large.rs` create a sparse 512 MiB base and
measure peak additional live heap bytes with a counting global allocator through
mount, replay, retention, finish, verify and rollback (limit 4 MiB), with an
8 MiB streamed export to avoid a dense half-GiB temporary on each run. They
skip with a message if sparse files are unavailable. `overlay_tests.rs` counts
hash passes, rejects same-size edits and truncation, exercises a two-layer parent
chain, and compares 32 deterministic workload seeds against the former dense
replacement/frame-order algorithm, including every closed-generation export.
A separate seeded keep=2 workload restores base bytes and exports every retained
generation after each prune. The frontend launch test in
`crates/machine-file/tests/sparse_launch.rs` counts exactly one disk hash and
bounds heap allocations through lowering and assembly for a sparse 504 MiB disk.

The supported implementation is Unix: `sync_all` for file and directory, with
std supplying `F_FULLFSYNC` on Apple targets for both. A rejected barrier aborts
creation/mount or poisons an active sink, with no fallback. New headers are
synchronized before hard-link publication; existing destinations are preserved.
After locking and opening (or publishing a new header), mount re-resolves the
canonical journal path and refuses a mismatch before cleanup, scan or recovery.
Cleanup considers only `<journal-file-name>.create-<decimal digits>.tmp`, with
at least one ASCII digit. Journal filenames containing `.create-`, `.retain-` or `.export-` anywhere are
refused before locking in both assembly and the sink, including through aliases;
nested journal names cannot own overlapping temporary namespaces.

A candidate must be a regular file whose bytes are empty, a prefix of the exact
base-bound creation header, or that header alone. On Unix it is opened with
`O_NOFOLLOW | O_NONBLOCK`: a symlink swap cannot be followed and a FIFO swap
cannot block. Metadata from the open handle must still describe a regular file
with the same device/inode as the initial `symlink_metadata`. Cleanup reads that
handle, then rechecks the entry's device/inode immediately before unlinking.
There remains a small path-replacement window between the final check and the
unlink, so the journal directory must be trusted. Off Unix cleanup retains the
initial type/content checks without the Unix open flags or identity checks;
durable mounts remain unsupported there.

Creation writes no other bytes to its temporary, so commands, foreign data,
symlinks and directories remain untouched. Unknown entries are ignored: mount
never replays them, and `create_new` skips occupied names. An entry disappearing
during metadata lookup, open or unlink is already gone and does not fail mount.
The directory is synchronized even when no tail needs repair. Other cleanup or
directory-sync errors abort mount.
Existing validated history is synchronized on mount before confirming its last
command sequence. The status records the last confirmed COMMAND_END sequence;
generation markers do not advance that command receipt. A writable recovery
failure aborts before a media session can be returned.

**MEASURED on macOS:** Rust tests exercise real-file creation, reopen/replay,
lock exclusion through normal paths and cross-directory symlinks, torn-frame
repair/sealing, checkpoints, finish and Drop. Real header I/O tests fail after
each of the 128 incomplete write prefixes and on header sync, then verify no
journal was published and a fresh mount can succeed. Another real-file test
checks that hard-link publication refuses an existing destination. Each real
file test uses a unique subdirectory of `std::env::temp_dir()` and removes it.
A dangling-symlink test checks refusal before lock creation. A filesystem hook
stages alias creation and a competing target mount between initial resolution
and open, without threads or timing; post-open verification refuses the stale
alias path without changing the active target's bytes or sink status. Real-file
tests reconstruct both temporary-file crash shapes, check orphan removal and
hard-link count reduction, preserve unrelated names, and replay the journal.

The injected model checks header publication ordering, torn-tail repair/seal
ordering, and exact successful traces for clean-history and command-complete
unsealed reopens. For each of those four fixtures it injects one error at every
operation in the successful trace, checking abort, lock release and the
first-error status; creation also checks publication state and allows
best-effort temporary cleanup. This
covers the model boundary (lock/open, post-open path verification, temporary
creation/publication/unlink and leftover cleanup, read/seek, write/flush/sync,
directory sync and recovery truncation), rather than every syscall, filesystem
failure combination or per-entry cleanup failure. A separate model check
requires leftover cleanup under the lock followed by directory sync, and aborts
on either boundary's injected failure. Both reopen fixtures explicitly require
directory sync and the existing-history barrier,
with recovery sealing only after that barrier for an unsealed tip.

Command-output tests cover all 1,920 incomplete prefixes of a two-sector batch,
short/interrupted/zero writes and flush/sync errors. Duplicate-sector tests
check last-wins memory and replay, unique frame counts and last-occurrence order,
including memory writes after poisoning. Streaming tests compare both scanners
at every truncation of a generated journal and reject oversized read requests
and injected read errors. Targeted mutations check the changed behaviors; this
is not a claim that every assertion or every platform syscall is mutation-tested.
These process/software fault tests establish neither a physical power-cut result
nor, by themselves, status/IRQ equivalence through an assembled journal drive.
Assembly integration is measured separately below.

**ASSUMED:** the filesystem/device honors successful synchronization and retains
previously synchronized bytes under the tail-only crash model stated above.

### Host errors and determinism

The media write operation has no host-I/O error result visible to the controller.
Its adapter latches the first sink failure in an assembly-owned status handle,
preserves all in-memory writes, and disables further appends in that session.
The frontend checks that handle at every existing host cooperation return and
before final success. It reports a host persistence error, stops further host
advancement, and does not seal or prune the poisoned generation. The current
chunk may finish before this check; a strict stop before the next instruction
would require a separate execution-loop change and is not promised here.
Report the last confirmed durable command sequence; the failed command's
durability is uncertain. Never translate ENOSPC/EIO into ST1, write protection,
an IRQ, a retry of the guest command, or replacement read data.

For fixed mounted bytes and fixed guest input, memory mutations, status bytes,
DMA results, IRQ edges and virtual-time advancement are identical across
volatile and journal adapters. Sink latency contributes no virtual time;
hashing, sequencing and generation ids use only deterministic bytes/counters.
Base sector reads during execution use the pinned verified descriptor; the
journal is read only at mount and explicit closed-boundary maintenance. A base
mutation is a host failure under the immutability policy above. Opening/replaying
media is an explicit initial-state input, like a base-image load.
Output-sink failure can stop a run at a frontend boundary; its completed guest
prefix must equal the corresponding volatile prefix. A base I/O or integrity
failure instead invalidates the current execution quantum: the unchanged
`SectorMedia` interface cannot return an error, so a placeholder read is paired
with a latched hard host failure. No successful execution or equivalent-prefix
claim is made after that failed read. Frontends must check persistence at their
cooperation boundaries. Failing and healthy hosts need not finish the same
amount of execution. Interactive-input timing remains a frontend
concern; persistence adds no new timing input.

## Tooling and config

### Config contract

Keep the agreed per-drive `write` modes: `volatile` (default), `readonly`,
`journal`. The `Eq`/`Debug` Rust enum, `WriteMode`, implements `Volatile`, `ReadOnly` and
`Journal { path, keep_generations }`, with the existing not-writable status
preserved. The journal variant carries its own path and retention count rather
than independent, potentially conflicting options.
The following is field notation only; it does not introduce TOML parsing:

```toml
write = "journal"
journal = "media/work.img.upfj"
keep_generations = 10
```

- A journal path is explicit and required; suggest an adjacent `<base>.upfj`
  path in help, but never create one merely because an image is file-backed.
  Relative paths follow the existing explicit-image convention: process cwd;
  every path in a machine description file, including `journal`, resolves
  against the canonical directory of the layer that wrote it before building
  the `MachineConfig` (diagnostics retain the spelled file path).
- `keep_generations` defaults to 10 and must be positive. It counts closed
  positive generations, including recovered and empty ones; the active
  generation does not consume a slot. No age-based pruning or wall clock.
- Journal mode initially requires `ImageSource::File`, a floppy, and 512-byte
  sectors. Reject blank/inline sources and the unserved HardDisk seam for this
  mode. The same sources remain valid for volatile/readonly.
- Reject journal-only fields with other modes and any journal/lock/output path
  aliasing a base, mounted image or another journal. Check existing file identity
  as well as normalized paths, including hard links. Open the base read-only.
- Volatile/readonly must not create a journal, lock, backup or temporary output.
  Presets, tests and goldens select volatile unless explicitly testing readonly.

### Assembly session API

`WriteMode::journal(path)` constructs `Journal { path, keep_generations: 10 }`;
the explicit variant can select a different positive count. Journal-only fields
cannot be represented on `Volatile` or `ReadOnly`. Assembly rejects non-file
sources, non-floppies and non-512-byte geometry for journal mode. Before any
journal or lock is opened, it compares canonical paths and existing file
identity (including hard links) against every drive/load image and other journal
or lock output. It also reserves each journal's same-directory
`<journal-file-name>.create-<decimal digits>.tmp` namespace against mounted
inputs, checking the canonical parent plus unresolved filename as well as the
fully canonical path. Other journal filenames containing `.create-` are refused
independently. These checks reserve names more broadly than cleanup, which also
checks file type and contents. Dangling journal symlinks are refused
before mounting any drive. Journal normalization errors identify the drive and
path; every failed input normalization is ignored for alias comparison so its
own image read reports the real error, without attribution to another journal. On case-insensitive filesystems,
not-yet-existing journal names differing only in case can pass validation; the
second mount fails safely at lock acquisition and names its drive, without case
folding in validation. Observation policy errors (missing roles, unsupported
outputs and duplicate routes) are checked before device construction and journal
mounts, so these failures neither create outputs nor recover an existing tip.
Mounting multiple drives is not transactional: if a later mount fails (including
a case-only lock collision), earlier journals and lock sidecars may already
exist, and earlier recovery may already have repaired or sealed a tip. Failure
drops their handles without a session-end seal; durable command history remains
recoverable, and stable lock sidecars are retained.
Base files are read once, read-only; replay uses those exact
loaded bytes. Volatile and readonly sessions create no host output.

`AssembledMachine::persistence` contains one `PersistenceHandle` per journal
with its drive index, configured retention count, mount recovery report, first
latched host error, `checkpoint(&mut machine)` and `finish(&mut machine)`.
Those two methods flush completed ATA sectors themselves. The assembly-owned sector
adapter shares the session with this handle. `run_session(policy)` checks errors
before and after every existing natural/prompt/stimulus run boundary;
`check_persistence()` supports an embedding frontend's own boundaries without
flushing. `flush_persistence()` explicitly prepares device snapshots or captures
an active prefix after a raw run error. A frontend
handling `ResetRequested` itself must call `AssembledMachine::before_cpu_reset()`
before `reset_cpu()`. Calling `run_bounded()` again instead performs the pending
reset automatically without capturing pre-reset CMOS state. The journal can then
differ from the CLI's, and a later execution error can lose that state. A failed
run must be dropped without `finish_persistence()`. A latched error during a run
skips every drive's finish; the other drives' unsealed sessions are recovered
on their next mount. A successful cap or halt, or an
explicit normal window close, calls `finish_persistence()`; repeated finish calls
are harmless, and the session run API refuses further advancement after finish.
The window keeps a session whose guest halted with interrupts disabled open
for a reset: at the halt it calls `before_cpu_reset()`, which appends changed
CMOS RAM and submits completed ATA sectors without closing a generation, and
it finishes the session at Stop.
Drop never writes a clean marker. Final display/disk capture can still read the
finished media bytes. There is no atomic close across multiple drives.

`machine check` performs no writes. Existing journals are inspected through the
offline shared-lock path, with read-only data opens, no temporary cleanup and no
recovery. With no stable lock entry, the same scanner inspects without a lock
and reports it; no lock is created. A missing journal is valid with a creation
note. Recoverable tails and unsealed tips are valid with a warning that the next
mount will recover them. Interior damage or a wrong base binding is invalid,
with the drive's machine-file origin. A journal in use is valid with a warning
that inspection was skipped. Hardware/catalogue validation then assembles the
journal drives as volatile, without replaying or mounting journal sessions.

There is no new frontend flag or serialized journal configuration in this step.
The existing presets and goldens remain volatile; journal selection is explicit
Rust data. Assembly passes `keep_generations` into the sink. The anchor counts
as one retained closed generation: keep=2 over ids 1–5 retains anchor 4 and id 5.

### Offline commands

The existing binary provides the following `media` commands, preserving `--boot`.
Parsing is dependency-free; the command layer and journal operations are library
APIs tested without spawning the binary:

```text
unemu media generations --base disk.img --journal disk.img.upfj
unemu media verify --base disk.img --journal disk.img.upfj
unemu media verify --base disk.img --journal disk.img.upfj --repair-tail --backup before-repair.upfj
unemu media rollback --base disk.img --journal disk.img.upfj --to 7 --backup before-rollback.upfj
```

`generations` lists ids, closing reasons, end offsets, command/sector counts,
the retained anchor and any unsealed tip. Include the base digest. Do not call
an interrupted generation clean or claim that clean means bootable.

`verify` checks binding and every frame/grammar invariant, replaying without
mutating media by default. It reports closed history, recoverable-tail bytes,
interior damage and the latest reconstructible state. Exit codes: 0 fully
valid/closed, 1 corruption/binding or I/O failure, 2 usage/config error, 3
recoverable tail or unsealed generation. `--repair-tail` explicitly applies
the normal open procedure, after a synchronized full backup to an unused path.
It still refuses interior corruption and incomplete anchors; arbitrary salvage
is a later, separately reviewed tool.

`rollback --to N` requires a retained closed generation (or available generation
0), backs up the entire journal to an unused path, synchronizes the backup and
its directory, then truncates just after N's GENERATION_END (128 bytes for 0)
and synchronizes. It removes later generations and any open tip. No guest runs
concurrently. A successful return means the truncated prefix is durable;
interruption may leave the old or truncated file, with the backup available.
An unavailable, pruned, nonexistent or unsealed target is a configuration error
(exit 2); a missing journal is an I/O failure (exit 1), and is never created by
a tool. Existing backup destinations, including symlink/hard-link aliases of
the base or journal, are refused (exit 2). Backup filenames containing the
reserved `.create-`, `.retain-` or `.export-` markers are also refused so mount cleanup cannot delete a
header-only backup. Backups are never deleted by these tools. On Unix they are
created with the journal's permission bits, then those bits are set exactly
before copying, so the umask cannot expose data or silently narrow permissions.
A failed copy or backup barrier leaves a retained file and reports it as a
partial, invalid backup with its path. Every I/O diagnostic names the role and
path of the affected base, journal, lock or backup. Tools preflight both inputs before
creating lock metadata, and open them without blocking on Unix; non-regular
inputs such as FIFOs are refused. Missing inputs leave no new files. Dangling
journal symlinks and canonical-path changes
after locking are I/O failures (exit 1). Tail diagnostics give the safe truncation
point and discarded bytes, separately from the failure/EOF offset. Rollback
validates its target before printing the removal warning.

Immutable export:

```text
unemu media commit --base disk.img --journal disk.img.upfj --to 7 --output restored.img
```

`commit` (alias `compact`) reconstructs a selected closed generation into a
**new raw image** at an unused output path. Write a same-directory temporary,
check its size and digest against reconstruction, synchronize it, publish
without overwriting an existing destination, then synchronize its directory.
The old base and journal remain untouched; never update them as a two-file
transaction. To continue from this new base, point config at it and an unused
journal path; its new lineage starts at generation 0. No automatic config edit,
in-place commit, or export of an unsealed tip. A user repairs/seals the tip first.
Export requires the shared stable lock (it refuses inspection's lockless fallback).
A recoverable tail or unsealed tip is exit 2; format/binding or I/O failures are
exit 1, and an unavailable generation or occupied output is exit 2.
On supported Unix hosts, publication uses `hard_link(temporary, output)`, which
refuses an existing destination, including symlinks/hard links to inputs. This
also protects against a destination appearing after the unused-path preflight.
The temporary is `<output>.export-<decimal>.tmp`, created exclusively with mode
0600. Read back and compare its length and SHA-256 before `sync_all`, hard-link
publication and directory `sync_all`. Cleanup unlinks only this export's temporary;
an interrupted orphan is never mounted or automatically imported. Errors before the hard-link attempt say "nothing was published"; errors at or
after that attempt report uncertain publication. EPERM, ENOTSUP and EXDEV from the
hard-link operation instead name the output-filesystem hard-link requirement and
say "nothing was published". An error from the directory barrier remains uncertain
even with those codes. Existing-destination errors keep their configuration exit.
A publication or directory-sync failure may leave a complete output; inputs remain authoritative.
Other platforms are refused by the durability barrier before publication.

### Locking and retention

Resolve an existing journal's canonical path before deriving the stable
`<journal>.lock` sidecar, so symlink aliases share the same nonblocking OS
advisory lock. Refuse a dangling journal symlink before creating a lock. For a
missing journal, canonicalize its parent only to establish the expected name
for the post-open comparison; locking itself already follows parent aliases.
After acquiring the lock and opening the journal, re-resolve its canonical path
and refuse any mismatch with the name from which the lock was derived. This
also catches a symlink introduced after initial resolution. Under that lock,
remove only decimal-named creation leftovers passing the type, identity and
header-prefix checks described above, then synchronize the directory before
scan/recovery. Never mount a leftover temporary as a journal.
Mounts and mutating tools take it exclusively; inspections take it shared.
Inspections open an existing lock read-only. If it is absent and creation fails
with EACCES, EPERM or EROFS, they may inspect without a lock and print one line
saying no lock was taken. A writing session must have created a lock in that
directory; an existing entry is never bypassed, even when unreadable. Mutating
tools still require an exclusive lock with write access.
Creation of lock metadata is permitted for inspections, but image/journal
contents remain read-only. The lock file is never replaced/unlinked by normal
tools; locks release on process exit, without stale-PID or time heuristics.
Never delete `<journal>.lock`: it is not a stale-PID file; deletion during a
session defeats the single-writer guarantee for sessions and tools alike.
Two drives/sessions cannot share a writable journal. The same lock protects
rename-based pruning; locking only the journal inode would not. External tools
that ignore this protocol are outside the contract.

After a successful checkpoint/session close exceeds `keep_generations`, rewrite
one journal atomically. Select the earliest of the newest N closed generations,
reconstruct it, and encode it as the anchor described above. Copy/re-encode
later retained generations, preserving ids/reasons while renumbering sequences.
Verify the complete replacement against the old states before synchronization
and publication. The original immutable base remains the hash binding.

Pruning runs only at a closed boundary, never on a poisoned/open generation.
A failed pre-publication rewrite leaves the original journal intact; report
maintenance failure separately from already durable guest commands. After a
publication/sync error, report uncertain maintenance completion and verify on
next open. The visible journal must be either the old valid version or the new
valid version under the storage contract. Temporary files are never mounted
automatically. No automatic deletion of offline backups.

The implementation uses `create_new` for `<journal>.retain-<decimal>.tmp`,
requests the journal's mode bits at creation, then applies those bits exactly with
`fchmod` before writing, to restore permissions filtered by the umask. Ownership
is not preserved: the replacement is owned by its creator; preserving another
owner would require privileges that this tool does not request. It streams later
frames from the old descriptor, re-encodes their sequence numbers, and compares
full images from each retained old/new closed prefix. A separate whole-replacement
scan checks status, generation count, ids and later closing reasons before sync.
The oldest point becomes reason 4;
subsequent reasons, including recovered reason 3, are preserved. Unix `rename`
replaces only the journal path under the unchanged sidecar lock, followed by
directory `sync_all`. The live sink then appends through the replacement descriptor.
The base and offline backups are never rewritten or removed. Creation cleanup
considers only `.create-` decimal names and requires the creation-header prefix.
Under the exclusive mount lock, cleanup also removes this journal's exact
`.retain-<decimal>.tmp` names, accepting any content/length because a crash can
leave any prefix of a full replacement. Both paths require a regular entry,
`O_NOFOLLOW | O_NONBLOCK` open, matching device/inode after open and just before
unlink; a disappearing entry is already gone. Symlinks, directories, nondecimal
names and other journals' leftovers survive. The subsequent directory barrier
covers removal. No leftover is mounted. Retention/export operations unlink only
their own temporary and never sweep another operation's active files.
Validation rejects input and journal filenames containing `.retain-`, including
through resolved aliases. This applies to volatile/read-only inputs too; errors
name the owning drive or the loadable image's role and path. Offline tools also
refuse reserved `.create-` and `.retain-` names, as given or resolved, for every
input/output including the base, journal, lock, backup and export destination
(exit 2), before any destructive operation. Independently, cleanup skips any
opened entry with the base file's device/inode. The identity comes from the same
file read that supplied the mounted bytes; offline repair carries it into sink
recovery. Thus even a reserved-name or hard-link alias cannot make cleanup own
the mounted base.

`SinkStatus::maintenance_failure()` and the corresponding assembly handle method
report maintenance separately from `failure()`/`status()`'s command-error latch.
Checkpoint/finish return success once the generation marker is durable, even when
retention fails. Assembly attempts every drive's finish, then returns the first
real command/close error. CLI and window frontends print one warning per drive
with a maintenance outcome; maintenance alone does not make the run fail.
A pre-publication failure leaves the old journal intact and permits retry at a
later close. A publication attempt or directory-barrier failure is uncertain:
verify on the next open before further persistent work. The old session preserves in-memory guest behavior but appends no more host
records. At a checkpoint this disabled output latches a host persistence failure,
so `check()` and the next frontend boundary stop further advancement; it cannot
durably close another generation without reopening. A session already ending
still reports its maintenance outcome without failing the durable close.
No controller-visible error/protection/IRQ is synthesized; Drop adds no marker.

Retention cost scales with journal size because retained states are reconstructed
by scanning history; this is acceptable for floppy images.
The anchor costs at most one frame per base sector plus two markers. Retained
write history and the current session can still grow without bound if a guest
rewrites sectors indefinitely before closing a generation. A generation count
is a history bound, not a byte quota. Explicit checkpoints or export bound
ordinary use; do not invent a hidden time/size checkpoint policy in version 1.

## Failure modes and recovery

| Failure | Required outcome |
| --- | --- |
| Different/modified base, short header, unknown header version | Refuse before replay and guest execution; preserve files. A path match is not a binding. |
| Process kill/power loss midway through SECTOR or COMMAND_END | Recover only through the last complete validated command; truncate pending/torn suffix under lock before further append. |
| Valid COMMAND_END but no generation end | Replay that command, seal the interrupted generation with reason 3, and retain the prior closed generation for rollback. |
| Torn GENERATION_END | Preserve completed commands; recover/seal the generation. Do not infer filesystem health from the marker. |
| Damaged history with a later intact generation marker | Refuse ordinary mount/repair; use the original base plus a valid backup or explicitly designed salvage. No magic-string resynchronization. |
| Corrupt final suffix without any later intact generation marker | Treat as tail under the documented fault assumption, report discarded bytes and uncertainty. Offline repair preserves a full backup first. |
| Guest logically damages a disk during an otherwise clean session | Checksums can remain valid. List generations, select a working one, back up and roll back; this loses later guest writes deliberately. |
| ENOSPC, short writes, flush/sync failure | Latch a host error; guest memory/status remain unchanged by it, appends stop, no clean end/prune, frontend reports failure. Recovery may preserve more or less than the last attempted batch. |
| Failure during pruning/export | Before publication, old inputs remain authoritative. After publication, verify old/new outcome; never pair a new base with an old journal automatically. |
| Failure during rollback | A retained validated prefix and synchronized pre-rollback backup are the recovery sources. Reopen/verify before continuing. |
| Second writer or mutating tool against live media | Fail lock acquisition before modifying journal data; no concurrent rollback/commit. |
| Torn storage writes that damage previously synchronized bytes, lost files, broken flush/rename guarantees | Outside the tail-only guarantee. Detect where possible; restore separately stored backups. Software fault injection is not a power-cut test. |

Always keep the base when copying/backing up a journal: a journal alone may
depend on untouched base sectors. A checkpoint cannot make a multi-command FAT
update atomic or capture CPU/RAM state. Rollback is a disk operation performed
before assembling a new machine, not a rewind of a live guest.

## Tests

### Spike evidence

**MEASURED:** a disposable Python standard-library probe outside the repository
encoded the specified 128-byte header and 640-byte frames and replayed a
two-generation, two-command journal. Seven tests passed, zero failed:

- Every one of 3,969 truncation boundaries of a 3,968-byte journal: incomplete
  headers refuse; complete headers recover precisely the command-complete prefix.
- Single-byte corruption at all 3,200 positions in the first five frames of
  sealed history: refuse rather than skipping to a later intact generation end.
- Exact layout/size, bad final-frame checksum, different-base rejection,
  rollback followed by append, and anchor reconstruction. The anchor test also
  rejects all 1,920 incomplete anchor-prefix lengths with a complete header.

**MEASURED mutation checks:** removing base-hash comparison makes the base-binding
test fail (one failure). Applying SECTOR bytes before COMMAND_END makes the
exhaustive cut test fail at 1,280 subcases. Both mutations were detected.
These are parser-model checks, not Rust integration tests, lock/fsync tests,
physical power cuts or performance measurements. No repository test was added
or changed by this documentation-only spike.

### Implementation acceptance tests

1. **Format units:** fixed known byte vectors for header and all frame kinds;
   endian/checksum coverage, reserved/unknown values, bad sizes/hash/base,
   sequence gaps/duplicates, LBA overflow/out of bounds, marker counts,
   zero-sector special case, malformed generation transitions and integer
   exhaustion. Generate every truncation of headers, multi-sector commands,
   markers and anchors. Ignore incomplete batches; never half-replay a command.
   Corrupt sealed history and prove refusal rather than loss of later history.
2. **Writer and filesystem fault injection:** an injectable output/filesystem
   boundary records write/flush/sync/publish/directory-sync/lock operations.
   Fail every operation, every partial write position, interrupted write and
   recovery truncation; model lost/reordered unsynchronized writes. Check the
   parser's valid-prefix result, complete-anchor requirement, error latch,
   last-confirmed sequence and refusal to append after failure. This validates
   ordering under a model; platform synchronization still needs host validation.
3. **Device isolation on `testkit::MockBus`:** a recording in-memory media
   adapter observes actual committed sectors for normal, multi-track,
   partial-terminal-count, zero-byte, overrun and invalid-address/protected
   commands. Reads, result bytes, IRQ and zero-filled tails match the current
   model. A failing output sink never changes guest-visible device state;
   rejected commands emit no sector batch. Reset retains media and output state.
4. **Two-run assembly integration:** copy a generated full floppy into a test
   temp directory. Run a small deterministic guest through actual uPD765/8237
   WRITE DATA, explicitly finish/drop the entire assembled machine, then freshly
   assemble using the original file plus journal. A second guest that only issues
   READ DATA asserts the written bytes through DMA. Assert the base hash is unchanged
   and no state is shared between runs. Repeat with a zero-filled partial sector
   and with process-style termination after a synchronized command but before
   GENERATION_END; recovery must list reason 3 and expose the committed bytes.
5. **Rollback and tools:** create A/B/C generations that overwrite the same LBA,
   roll back to A, remount and read A through the controller; append a new B and
   verify no discarded bytes return. Exercise generation 0, unavailable/pruned
   ids, open tips, required backups, lock exclusion and output-path collisions.
   Export every retained generation and compare full bytes with its replay.
6. **Retention/publication:** with keep=2 and at least four generations, compare
   every surviving generation before/after rewrite, including sectors last
   changed only in a pruned generation and an anchor equal to the base. Verify
   stable ids, renumbered sequences, original-base binding and pruned-id errors.
   Inject failure at every temporary write/sync/replace/directory-sync: only
   the old or complete new journal is authoritative; base bytes never change.
7. **Volatile, readonly and deterministic regression:** compare full memory,
   sector reads, status/IRQ traces, CPU state and virtual time for identical
   inputs under volatile and journal adapters, including delayed/failing sinks
   up to the host error boundary. Volatile starts from the unchanged file on
   run two and creates no output; readonly rejects writes with the existing
   ST1 not-writable result (ATA ERR/ABRT for IDE), also without output. The IDE
   regression drives REP OUTSW with small caps and compares full guest memory,
   ATA save(), retired instructions, virtual time and PIT clock across volatile,
   journal and failing-sink runs. Journal bytes must also be identical across
   different ordinary host cap splits. Run existing fast floppy-write
   and boot goldens unchanged. At implementation merge, run the fixture-backed
   slow suite with volatile explicitly selected and compare fixture hashes and
   directory entries before/after; no sidecars may appear beside fixtures.

Record red/green evidence or a targeted mutation for each new behavioral test.
The original design predicted that the two-run check would fail against the
former load-once/no-output behavior; that prediction was **READ**, not an executed
red test. Current executable evidence is listed in the appendix. Suppressing
command mirroring/replay must fail the cross-run test; bypassing the sink latch
must fail fault tests; skipping the anchor snapshot must fail retention tests.
Existing goldens are regression evidence for the unchanged volatile default.

### Session wiring evidence

**MEASURED:** eighteen assembly integration tests cover two fully independent guest
WRITE/READ DMA sessions, 17-byte terminal-count padding, explicit finish and
interrupted-tip recovery, immutable bases, volatile/readonly output absence,
config defaults/refusals, canonical and hard-link aliases (including locks and
load images), reserved temporary names (including unresolved symlink inputs),
dangling journal refusal, missing-input error attribution, observation-policy
preflight without output or recovery, damaged-header refusal, checkpoint/finish
and error boundaries.
A journal/volatile comparison checks all RAM, exposed CPU registers, interrupt
readiness, device snapshots, IRQ levels, traces and virtual time at successive
run boundaries, for full and partial-sector commands.

**MEASURED:** an assembly unit test wraps a real mounted journal in the optional
`media/test-support` failing-output adapter. Actual WRITE DATA updates memory,
then its output fails. A second WRITE DATA in the same bounded run still replaces
memory; read-back DMA and guest status match the volatile run without write
protection. The frontend boundary reports the latched error, refuses further advancement and cannot seal the session. A
separate exhausted-generation test covers a latched checkpoint error. CLI and
headless presenter tests check successful cap/halt finalization, persistence-error
classification, stopping at the first failed window boundary, and preservation
of unsealed execution errors, without opening a window. A matrix compares the
observation pre-check with actual `Observation::bind` for every configured device
kind and all output kinds, including missing and duplicate routes. Real-file
sink tests retain symlinks, commands and foreign matching entries while cleaning
all creation-header prefixes, including empty and full-header files.

**MEASURED:** forty-five recorded mutation variants establish command mirroring,
output-free volatile mode, config/alias validation, guest-state preservation,
checkpoint/error latching and frontend finalization. They also reject both
post-error write regressions, omitted window checks, dangling journal symlinks,
wrong CLI persistence classifiers, unconditional temporary deletion, symlink
following, omitted content checks, disabled genuine-header cleanup, reserved
journal names, canonical-only input checks and observation pre-check drift.
Thirteen variants specifically cover decimal-only temporary names, nested
journal refusal in both layers, disappearance at each I/O boundary, both Unix
open flags, blocking FIFO swaps, opened/final entry identity and input error
attribution for not-a-directory paths and symlink loops. They produce fourteen
expected targeted test failures. Positive integration runs cover independent
sessions, immutable bases, byte replay and host failure boundaries. Deterministic
I/O hooks stage the disappearance and identity races; FIFO tests run in isolated
child processes with a timeout so a blocking regression fails safely.
These are software checks, not a physical power-loss measurement. Retention
acceptance is recorded below; offline command
acceptance is covered by the CLI media-tool integration suite and the backup
barrier fault model.

### Offline command evidence

The initial sixteen CLI integration tests cover rollback/remount/reappend,
generation 0, retained-anchor and unavailable/open targets, read-only scans,
all four exit codes, wrong bases, torn tails, interior damage, incomplete anchors,
required backups and their aliases, shared/exclusive locks (including journal
symlinks), deterministic listing output, usage errors and the existing boot demo.
The initial twenty-nine targeted mutations produced failing tests. Their backup
barrier checks omitted calls in the abstract backup model; they did not establish
that HostBackup invoked either production barrier or that rollback synchronized
the truncated journal. They also did not test the post-lock canonical recheck.

Recording tests now drive the production HostBackup and rollback paths against
real files. They record backup creation, copy, backup-file sync, backup-directory
sync, journal truncate and journal sync in that order. Each operation also
records its target kind; sync/copy/truncate classify the actual descriptor using
metadata. File and directory barrier targets are asserted separately. Repair
records the same backup prefix before transferring the held lock to normal sink
recovery; the sink's existing fault model separately tests recovery truncation/seal
barriers.
Failure injection at copy and both backup barriers checks that no journal
mutation follows and that the retained invalid backup is identified. A path-swap
hook exercises the actual post-lock canonical comparison. Permission and FIFO
checks cover read-only locks/storage, private backups and nonblocking refusal.
The FIFO test has its own one-test binary so process creation cannot transiently
inherit locks held by sibling tests. Permission checks use a file-access probe
without spawning, and skip when the host bypasses permission bits (such as root).
Tests require an unreadable existing lock to fail with its lock path, rather than
falling back to lockless inspection; dangling lock symlinks name the lock too.
**MEASURED:** each of `media_tools` and `media_fifo` passed sixty consecutive
runs with the default parallel harness, with zero failed runs in either series.
Twenty-four targeted mutations fail their tests, including removal of the
existing-lock guard, incorrect lock error paths/roles and swapping file/directory
sync descriptors while preserving their operation labels.
These checks do not establish physical power-loss durability or implement
retention/export acceptance by themselves.

### Export and retention evidence

**MEASURED:** real-file tests export base generation 0 and every retained closed
generation, compare complete output bytes, and compare both input files byte for
byte. They reject occupied paths, input hard-link/symlink aliases, unavailable or
pruned ids, torn/unsealed tips, bad bases and corruption. Shared inspections can
coexist with export, while a live writer excludes it. The recording I/O seam drives
production writes and syncs, asserting temporary write → file sync → no-clobber
publication → directory sync and the descriptor kinds. Corrupted or shortened
export temporaries never reach a barrier or publication; a destination created
at publication is preserved.

**MEASURED:** retention tests compare each retained image, ids, original-base
binding and frame sequences. They cover keep=2 over five generations, remount,
continued writes after pruning, empty anchors, sectors last changed in
pruned history, recovered closing reasons, unavailable/pruned export targets and
`media verify` exit 0. An altered anchor whose sector is overwritten later is
rejected before sync, proving verification checks intermediate states too.
Failures at every replacement write (including partial writes), file sync, rename
and directory sync leave the old valid journal or the complete valid replacement.
A rename-then-error hook exercises an uncertain successful publication. Reports
distinguish pre-publication failure from uncertainty. Write-protection status
stays unchanged; pre-publication maintenance leaves the command-error latch clear,
while uncertainty at a live checkpoint latches disabled appends for the next
frontend boundary. A real assembled controller test checks
RAM/result bytes, device snapshots/IRQ levels and virtual time across all four
failure boundaries. Open and poisoned generations never prune. Namespace tests
keep creation and retention operations from sweeping each other's active files;
the next exclusive mount removes its own regular retention orphans, including a
header-only orphan. Symlink orphans survive.

**MEASURED (initial export/retention implementation):** the 21 new behavioral tests have matching targeted mutation
failures. Across 34 mutation variants and 40 targeted executions, 39 executions
fail their tests. Removing only the explicit export length comparison survives
because SHA-256 still rejects a shortened file; bypassing the complete
size/digest guard fails. Other detected variants remove export writes/barriers,
change no-clobber publication to replacement, permit unsealed export, use an
exclusive export lock, disable pruning or assembly wiring, omit anchor sectors,
reuse old sequences, change later reasons, skip retained-state verification,
latch maintenance as a command error, suppress uncertainty, prune open/poisoned
history, mix cleanup namespaces, or truncate occupied temporary aliases.
All mutation source changes and runs used a disposable copy outside the repository.

**MEASURED (initial export/retention implementation):** gates passed: workspace build, Clippy with warnings denied,
format check, the fast workspace suite (zero failed, slow tests ignored), and the
required fixture-backed ignored run (14 passed, zero failed). Counts use top-level test
harnesses and exclude duplicate watchdog child executions. Before/after checks
find identical SHA-256 values for all 14 vendor files and identical directory
entries, including the four resolved fixture target directories; no sidecars
appear beside fixtures. No new test spawns a process.

### Export and retention review evidence

**MEASURED:** thirteen additional tests cover maintenance-only close success and
warnings for either or both of two drives, uncertain reporting, finishing later
drives after a real error (including a previously latched error), exact 0664 mode
before writes/sync/publication under an emulated restrictive umask, and the orphan
and export-diagnostic rules above. An otherwise valid later closing reason is
changed without changing any retained image: the whole-history check refuses it
before sync. A close reaching exactly `keep_generations` preserves the expected
closed journal byte for byte and records no replacement operation. A generation
sealed during mount consumes a retention slot. Failed rewrites remove their own
temporary while preserving unrelated entries. A command written and synchronized
after pruning survives Drop without close and is recovered on remount; changing
rename to copy fails this test directly.

**MEASURED:** twenty-five valid targeted mutation variants fail their tests,
including all four previously surviving mutations, copy-instead-of-rename,
maintenance-as-error, warning suppression/duplication, lost mode restoration,
unsafe orphan selection/identity checks and incorrect publication diagnostics.
Two initial mutation invocations omitted the optional assembly test-support
feature and are excluded as compilation failures, not counted as detection.
Mutation checks leave the tested source tree unchanged.

**MEASURED:** the two pre-existing spawning tests now have their own
`media/tests/temporary_fifo.rs` binary; the library test binary spawns no process.
Both use the same production temporary-entry implementation. Each binary passes
thirty consecutive default-parallel runs (zero failures in either binary).
Removing nonblocking open still fails the isolated watchdog test rather than
blocking the test runner indefinitely.

**MEASURED:** review gates pass: build, warning-free Clippy and format check;
the fast workspace suite passes with slow tests ignored, then the required
fixture-backed slow set passes once (14 passed, zero failed). All 14 vendor-file hashes and directory
entries, including four resolved fixture directories, are unchanged. These
checks remain software fault/reopen tests, not physical power-cut evidence.

**MEASURED:** reserved-base repair and rollback probes, along with the read-only
and export tools, refuse both reserved name families as given and through
symlinks without changing base/journal bytes or creating backups. Output and lock
aliases receive the same configuration refusal. Direct cleanup tests protect the
base inode in both namespaces while deleting a different inode with identical
bytes; a sink mount and assembly mount independently check identity propagation.
An uncertain checkpoint followed by an actual guest WRITE DATA retains guest
memory/status behavior, reports disabled appends through the persistence latch,
and prevents the next checked run boundary from advancing. Subject tests pin the
second drive and the firmware/RAM-load image roles. Fourteen targeted mutations
cover the path guards, inode protection and propagation, disabled-output latch,
and input error subjects; every mutation fails its targeted test.

**ASSUMED:** successful platform synchronization and atomic namespace operations
obey the storage contract above. These checks measure software ordering, real-file
reopen behavior and injected failures on the test host, not physical power cuts,
other operating systems/filesystems, storage latency or guest filesystem health.

## Implementation beads

Historical implementation breakdown, not a second task board. Items 2–7 are
implemented; item 8 is validated by the evidence below. Current task lifecycle
remains in beads. Dependencies refer to this design breakdown.

1. **Re-scope upc-sk3.1: Recoverable journal-backed floppy persistence.** Make
   the existing write-back bead the umbrella for this contract, with volatile
   default, readonly mapping, immutable bases and the explicit durability/error
   behavior. Acceptance: maintainer reviews the format and defaults; link child
   beads below and update the deferred write-back entry when implementation
   lands. Dependencies: this design review; completion waits for items 2–8.
2. **Implement the floppy journal codec and recovery scanner.** Acceptance:
   byte-exact version-1 encoder/parser, base binding, command/generation grammar,
   anchor validation, exhaustive truncations, interior-corruption refusal and
   read-only verification report. No device/filesystem coupling. Dependency: 1's
   approved contract, not its umbrella closure.
3. **Route floppy writes through a host-independent media contract.** Acceptance:
   replace direct image mutation with command-batched media operations, retain
   existing in-memory behavior and write protection, and pass MockBus and fast
   goldens unchanged, including partial/abnormal completions and reset. No host
   persistence enabled yet. Dependency: 1's approved contract.
4. **Implement the durable journal sink and single-writer lifecycle.** Acceptance:
   append/flush/platform-sync ordering, stable locks, synchronized creation,
   tail repair/sealing, checkpoints/finish and latched output errors; injected
   short-write/sync/lock failures cannot alter device bytes/status. Dependencies:
   2 and 3.
5. **Wire drive write modes and replay into assembly sessions.** Acceptance:
   validated Rust config enum/path/retention count, preserved source metadata,
   replay before boot, explicit finalization/error checks in consuming frontends,
   no silently discarded persistence handle, and two-run guest-write integration.
   Volatile/readonly remain free of host output; no serialization feature.
   Dependencies: 3 and 4.
6. **Add offline media verification, generation listing and rollback.** Acceptance:
   proposed CLI/exit contracts, backups and lock exclusion, correct truncation
   through a selected retained generation, bad-base/interior-damage refusal,
   and rollback/remount/reappend integration. Dependencies: 2 and 4.
7. **Add immutable-image export and atomic journal retention.** Acceptance:
   commit/compact produces a new verified raw image without changing inputs;
   keep=N rewrites an independently valid anchor plus retained history;
   publication-failure tests show old/new valid outcomes and no input aliases.
   Dependencies: 2, 4 and 5; reuse the command group from 6.
8. **Validate persistence failures and unchanged volatile goldens.** Acceptance:
   full tests above, targeted mutation/red-green evidence, platform sync/locking
   validation, fixture hashes/absence of sidecars and full-result deterministic
   comparisons; document actual supported host guarantees and command examples.
   **Done:** evidence appendix and supported host guarantees below record the
   acceptance audit, regression tests and targeted mutations. The required slow
   fixture gate runs without updating golden expectations. Dependencies: 5–7.

Approved defaults are ten retained generations, synchronous per-command
synchronization, mandatory backups for destructive offline tools, and replay/seal
of an interrupted command-complete tip on normal mount. A requirement for
explicit repair before normal mounting would be a contract change. Measured
macOS/APFS support and the remaining host/storage assumptions are listed below.
Real power-cut validation remains deferred. The original one-day estimate
applied to the design spike, not implementation of this breakdown.

## Supported host guarantees

**MEASURED (2026-11-10):** this validation ran on macOS; `statfs` identified
APFS for both the checkout and the temporary test directories. Real-file tests
created, synchronized, closed and reopened journals, recovered interrupted tips,
excluded a second writer and offline tools, and exported images through
no-clobber hard-link publication. Renaming retained journals and syncing their
directories succeeded. Injected errors establish operation order and old-or-new
valid publication outcomes in the software model. None of these measurements
is a power-loss test.

**READ:** `sink.rs::platform_sync` calls `File::sync_all` for files and directories.
The installed Rust 1.99.1 standard-library source (`sys/fs/unix.rs`, `File::fsync`)
uses `fcntl(F_FULLFSYNC)` on Apple targets and retries interrupted calls;
`File::try_lock`/`try_lock_shared` use nonblocking `flock`. Thus the passing real-file
tests exercise those platform implementations; this audit did not trace syscalls
or independently inspect a device cache. Journal creation and immutable exports
use `std::fs::hard_link`; retention replaces the journal by same-directory rename.
A successful export is checked byte for byte, and occupied destinations survive.

On Linux the same code calls `File::sync_all` (`fsync` in Rust's Unix implementation),
nonblocking `flock` and same-directory hard links/rename. Linux storage behavior
is **ASSUMED, not measured in this macOS session**. The `ubuntu-latest` GitHub
Actions job is an x86-64 Linux VM; its workspace tests exercise
`real_file_round_trip_checkpoints_finish_and_empty_generations`,
`real_lock_excludes_second_open_and_drop_only_releases_handles`,
`header_publication_never_replaces_an_existing_destination`,
`commit_records_real_write_verification_and_publication_barriers`,
`export_no_clobber_handles_destination_created_at_publication`,
`retention_runs_after_checkpoint_and_finish_and_continues_on_replaced_inode`,
and `exclusive_session_excludes_all_tools_and_shared_inspections_coexist`.
This identifies CI coverage, not an assertion that a new remote CI run was observed.
Permission tests may skip when the runner bypasses Unix permission bits.

**ASSUMED storage contract:** local Unix storage honors successful file/directory
syncs, preserves previously acknowledged bytes, and implements atomic same-directory
namespace operations and working advisory locks. All writers must cooperate with
the stable `<journal>.lock`; nobody deletes it or replaces mounted inputs. Network
filesystems, other filesystems, non-Unix hosts, disk/controller caches, physical
power cuts and storage latency bounds were not validated. Power-loss durability
is assumed under this storage contract; it is not a measured guarantee. Host
synchronization errors, including injected ENOSPC and EIO, stop persistence at the
next checked frontend boundary, and the failed command's durability is uncertain.
The first failure and last confirmed command sequence remain available; completed
guest work still matches the corresponding volatile prefix.

**MEASURED determinism scope:** the expanded assembly comparison uses full
64 KiB RAM, all exposed general registers/RIP/CS, IF/interrupt readiness and pending
IRQ, completed steps, retired instructions, virtual time/PIT clock, every device
snapshot and IRQ level, and complete per-device/open-bus access traces (zero dropped
events). Guest stores also capture final FLAGS and DS/ES/SS in compared RAM.
The same guest performs multi-sector and partial-sector writes and a write that
commits a sector before an abnormal missing-sector result. Every sector of the
final controller image is compared with the volatile image and explicit expected
bytes, then with scanner replay and a freshly reopened journal. The failing-output
comparison checks the complete guest prefix and final media after subsequent
writes, and refuses another checked run or a seal. Private CPU implementation
state is outside the public CPU seam and is not independently inspected.
Real synchronous journal I/O adds host latency in these comparisons without
adding virtual time; no artificial sleep or timing threshold is used.

The ignored fixture protection test runs the BIOS POST, FreeDOS prompt and DOS
file-write paths with `WriteMode::Volatile` selected explicitly. It checks the
pinned BIOS, FreeDOS and test386 SHA-256 values read from the included vendor
build scripts; the HDD and XTIDE output pins are read from their scripts and
checked whenever those auxiliary fixtures exist initially. A missing, ambiguous
or malformed output pin fails with a fixture-specific script-format diagnostic. It compares
recursive vendor directory entries and each present fixture's resolved parent
tree before/after every run, covering worktree symlinks and sidecars of any name.
Missing boot/CPU inputs fail under `UNEMUPC_REQUIRE_FIXTURES=1`, otherwise the
test skips like other goldens. Auxiliary fixtures are not required by the CI
fixture-fetch gate and are not silently fetched by this test. All present pins
must remain readable and unchanged. Existing golden expectations stay unchanged.
This binary spawns no process and holds no journal lock.

## Acceptance evidence appendix

These are executable evidence locations, not a test-count inventory. File paths
are relative to the repository root; names identify Rust tests in those files.
**READ** identifies the assertion being audited; the local build/fast/slow gates
provide **MEASURED** execution evidence. Filesystem fault tests are models, not
physical disk-failure or power-cut measurements.

| Acceptance requirement | File | Tests / evidence |
| --- | --- | --- |
| Format: exact header and every frame kind, endian/checksum bytes | `crates/media/tests/journal.rs` | `exact_header_vector_and_round_trip`, `exact_sector_vector_and_round_trip`, `exact_command_end_vector_and_round_trip`, `exact_generation_end_vector_and_round_trip` |
| Format: reserved/unknown values, length, hash, base/geometry binding | `crates/media/tests/journal.rs` | `header_binding_sizes_and_reserved_fields_are_enforced`, `header_rejects_a_matching_non_sector_aligned_base`, `header_version_and_length_are_checked_before_self_hash`, `frame_sizes_reserved_fields_and_unknown_encodings_are_enforced`, `every_single_byte_corruption_is_detected` |
| Format: sequence gaps/duplicates, generation transitions, LBA overflow/bounds, counts | `crates/media/tests/journal.rs` | `command_and_generation_grammar_is_enforced`, `final_grammar_failure_is_reported_as_uncertain_tail` |
| Format: zero-sector exception and canonical anchors | `crates/media/tests/journal.rs` | `empty_anchor_is_the_only_legal_zero_sector_command`, `anchor_grammar_rejects_noncanonical_snapshots` |
| Format: integer exhaustion without wrapping | `crates/media/src/scan.rs` | `exhausted_counters_never_wrap`; anchor-generation exhaustion also in `journal.rs::anchor_grammar_rejects_noncanonical_snapshots` |
| Format: every header/command/marker truncation, atomic command replay | `crates/media/tests/journal.rs` | `every_truncation_replays_only_complete_commands`, `replay_preserves_order_padding_and_unchanged_sectors`, `streaming_matches_slice_at_every_cut_and_bounds_read_requests` |
| Format: every anchor prefix requires complete synthetic command and marker | `crates/media/tests/journal.rs` | `anchor_round_trip_and_every_incomplete_prefix` |
| Format: refuse damaged sealed history despite later markers | `crates/media/tests/journal.rs` | `torn_frame_with_later_marker_refuses_at_every_cut`, `later_marker_detection_is_conservative_and_uses_only_fixed_stride`, `refused_report_with_a_committed_tip_never_proposes_recovery` |
| Writer: record write/flush/sync/publication/directory-sync/lock order | `crates/media/src/sink_tests.rs` | `creation_and_each_command_have_ordered_barriers`, `model_lock_contention_is_latched_before_journal_access` |
| Writer: fail every creation operation and every partial header/command write | `crates/media/src/sink_tests.rs` | `every_creation_operation_failure_aborts_mount_and_releases_lock`, `header_failures_leave_no_published_journal_and_allow_retry`, `every_partial_command_failure_preserves_memory_and_latches_first_error` |
| Writer: interrupted/short/zero writes and failed flush/sync | `crates/media/src/sink_tests.rs` | `short_and_interrupted_writes_never_duplicate_bytes`, `failed_flush_sync_and_zero_write_preserve_guest_data_and_poison_output` |
| Writer: recovery truncation/seal failures and repeated checkpoint/finish | `crates/media/src/sink_tests.rs` | `repair_truncates_and_syncs_before_sealing_and_refuses_every_io_failure`, `failed_checkpoint_or_finish_never_seals_again` |
| Writer: lost/reordered unsynchronized frames, every suffix cut, confirmed-prefix preservation | `crates/media/src/sink_tests.rs` | `lost_or_reordered_unsynchronized_frames_never_half_replay_a_command`; enumerates frame subsets/permutations and byte prefixes after failed sync |
| Device isolation: normal/multi-track command batches and result/IRQ/read bytes | `crates/devices/src/upd765.rs` | `media_contract_batches_commands_in_transfer_order`, `write_data_commits_memory_then_reads_back_identical` (MockBus) |
| Device isolation: partial TC padding and abnormal overrun/no-data/end-of-cylinder writes | `crates/devices/src/upd765.rs` | `media_contract_partial_sector_is_padded_in_one_batch`, `media_contract_abnormal_completion_keeps_committed_sectors` |
| Device isolation: zero-byte, invalid/protected rejection emits no batch | `crates/devices/src/upd765.rs` | `media_contract_empty_and_invalid_writes_emit_no_batch`, `media_contract_read_only_rejects_without_a_batch` |
| Device isolation: reset retains media/output state; incomplete writes discarded | `crates/devices/src/upd765.rs`; `crates/cli/tests/media_device.rs` | `media_contract_reset_preserves_media_and_discards_uncompleted_write`; `failing_sink_on_mock_bus_keeps_dma_results_irq_padding_and_reset_state` uses a thin test media bridge to a real failing sink, with volatile result/snapshot/trace/IRQ comparison |
| Mount: the base pin is checked on the mounted bytes before a journal is opened | `crates/machine-file/tests/journal.rs`; `crates/assembly/src/assemble.rs` | `pinned_base_rewritten_after_load_is_refused_before_journal_creation`, `existing_journal_with_changed_base_reports_pin_mismatch_before_binding`; **READ:** `resolve_file_sources` hashes the loaded bytes before mounting drives |
| Two-run integration: actual guest DMA, full/partial sector, finish/drop, immutable base, reason-3 interruption | `crates/assembly/tests/persistence.rs` | `guest_write_survives_fresh_assembly_including_partial_sector_and_interruption` |
| Tools: A/B/C overwrite, rollback to A, guest read, reappend without discarded data | `crates/assembly/tests/persistence.rs`; `crates/cli/tests/media_tools.rs` | `rollback_then_reappend_is_visible_through_fresh_guest_dma_reads`; `rollback_remount_reappend_reuses_next_generation_and_discards_tip` |
| Tools: generation 0, pruned/unavailable/open targets, backups and output aliases | `crates/cli/tests/media_tools.rs` | `rollback_zero_is_exactly_the_header`, `rollback_unavailable_ids_are_config_errors_without_backup`, `rollback_refuses_an_unsealed_target_and_backs_up_a_torn_suffix`, `backup_aliases_and_reserved_names_are_refused_without_changing_inputs`, `commit_refuses_input_aliases_open_tips_and_unavailable_targets` |
| Tools: shared/exclusive lock exclusion, inspect immutability, exit classes/repair | `crates/cli/tests/media_tools.rs` | `exclusive_session_excludes_all_tools_and_shared_inspections_coexist`, `inspection_keeps_read_only_inputs_and_creation_leftovers_untouched`, `verify_exit_contract_covers_clean_tail_tip_damage_base_and_missing`, `repair_backs_up_full_torn_tip_then_seals_as_interrupted` |
| Tools: production backup barriers before mutation, every backup/truncate/sync failure | `crates/media/src/offline_tests.rs` | `rollback_records_production_backup_truncate_and_sync_order`, `repair_records_production_backup_barriers_before_normal_recovery`, `production_backup_failures_leave_declared_invalid_backup_and_no_mutation`, `production_rollback_stops_on_failed_truncate_or_sync_with_journal_context` |
| Export: every retained generation equals replay; base and journal unchanged | `crates/cli/tests/media_tools.rs` | `commit_and_compact_export_each_closed_generation_without_changing_inputs`, `retained_journal_verifies_and_exports_all_surviving_states` |
| Export: verified temporary, real sync descriptors, no-clobber publication/race, every publication failure | `crates/media/src/offline_tests.rs` | `commit_records_real_write_verification_and_publication_barriers`, `export_checks_temporary_size_and_digest_before_any_barrier_or_publication`, `export_no_clobber_handles_destination_created_at_publication`, `export_publication_failures_preserve_inputs_and_only_publish_verified_bytes` |
| Retention: keep=2 over multiple generations, surviving images/ids/reasons, original binding, new sequences | `crates/media/src/sink_tests.rs` | `retention_keeps_two_closed_states_with_anchor_ids_reasons_and_new_sequences`, `retention_runs_after_checkpoint_and_finish_and_continues_on_replaced_inode` |
| Retention: pruned-only sector changes and anchor equal to base, pruned targets | `crates/media/src/sink_tests.rs`; `crates/cli/tests/media_tools.rs` | `retention_keeps_two_closed_states_with_anchor_ids_reasons_and_new_sequences`, `retention_encodes_empty_anchor_and_preserves_later_recovered_reason`; `retained_journal_verifies_and_exports_all_surviving_states` |
| Retention: fail every temporary write/partial write/sync/replace/directory-sync; old or complete new authority | `crates/media/src/sink_tests.rs` | `retention_fails_every_temporary_write_without_changing_durable_closed_journal`, `retention_failures_report_maintenance_separately_and_keep_old_or_new_valid_files`, `retention_barriers_follow_complete_verified_temporary_and_namespace_is_isolated` |
| Retention: verify intermediate states/metadata; never prune open or poisoned state | `crates/media/src/sink_tests.rs` | `retention_verifies_each_state_before_sync_even_when_final_image_matches`, `retention_checks_whole_history_metadata_before_sync`, `retention_never_runs_for_open_or_poisoned_generation` |
| Regression: volatile second run starts from base without outputs; readonly ST1 NW and no outputs | `crates/assembly/tests/persistence.rs` | `volatile_and_readonly_create_no_outputs_and_restart_from_base` |
| Regression: full healthy adapter result and image/reopen equivalence, multi-sector/partial/abnormal writes | `crates/assembly/tests/persistence.rs` | `journal_and_volatile_have_identical_guest_state_at_every_boundary` |
| Regression: fast floppy write/boot goldens unchanged | `crates/cli/tests/fdc_write_golden.rs`; `crates/cli/tests/floppy_boot_golden.rs`; `crates/cli/tests/boot_golden.rs` | `fdc_write_commits_a_sector_and_reads_it_back`, `floppy_boot_reads_sector_and_runs_it`, `timer_interrupt_boot_matches_golden` |
| Regression: volatile fixture slow paths, pinned hashes and no vendor/target-directory sidecars | `crates/cli/tests/media_fixtures.rs` | `volatile_boots_preserve_pinned_fixture_hashes_and_vendor_entries`; existing BIOS/FreeDOS/DOS and scheduler goldens run unchanged in the required ignored gate |
| Host errors: latch first error/confirmed sequence, disable later appends but retain all memory writes | `crates/media/src/sink_tests.rs` | `every_partial_command_failure_preserves_memory_and_latches_first_error`, `failed_flush_sync_and_zero_write_preserve_guest_data_and_poison_output`, `enospc_and_eio_latch_without_write_protection_or_lost_memory_writes` |
| Host errors: no guest error/write protection/IRQ/retry/replacement data; completed chunk matches volatile prefix | `crates/cli/tests/media_device.rs`; `crates/assembly/tests/persistence.rs` | `failing_sink_on_mock_bus_keeps_dma_results_irq_padding_and_reset_state`; `failed_output_preserves_complete_guest_prefix_and_final_media` (workspace enables `assembly/test-support`) |
| Host errors: check every frontend cooperation/final-success boundary; stop further advancement and never seal/prune poison | `crates/assembly/src/persistence.rs`; `crates/cli/src/lib.rs`; [`crates/window/src/main.rs`](https://github.com/unemu-org/unemu/blob/8702727a5f88dae352d095ba456ab0ca5f529c72/crates/window/src/main.rs) | `failing_sink_preserves_guest_prefix_and_stops_at_frontend_boundary`; `journal_host_failures_are_reported_as_persistence_failures`, `journal_frontend_finishes_successful_caps_but_leaves_execution_errors_unsealed`; `journal_host_failure_stops_at_the_first_window_boundary`, `journal_presenter_finishes_halts_and_preserves_unsealed_execution_errors` |
| Host errors: uncertain retention disables appends at next boundary without altering guest state | `crates/assembly/tests/persistence.rs`; `crates/assembly/src/persistence.rs` | `uncertain_checkpoint_latches_disabled_appends_and_stops_the_next_boundary`; `retention_failures_preserve_controller_status_dma_irq_and_virtual_time` |
| Determinism: identical memory/status/DMA/IRQ/clock despite synchronous host latency or output failure | `crates/assembly/tests/persistence.rs` | `journal_and_volatile_have_identical_guest_state_at_every_boundary`, `failed_output_preserves_complete_guest_prefix_and_final_media`; no wall-time thresholds |
| Determinism: no journal/base reads during command access; closed retention uses saved base | `crates/media/src/sink_tests.rs` | `mounted_sector_access_never_reads_or_seeks_host_storage`; `retention_keeps_two_closed_states_with_anchor_ids_reasons_and_new_sequences`; **READ:** `JournalMedia::base` and retention implementation use owned original bytes |
| Determinism: bytes/counters define hashes/sequences/ids; replay is an initial-state input | `crates/assembly/tests/persistence.rs`; `crates/media/tests/journal.rs` | `identical_guest_sessions_produce_byte_identical_journals` compares complete journal bytes across separate directories after identical full/partial writes, checkpoints and session closes; fixed vectors and `replay_preserves_order_padding_and_unchanged_sectors` establish replay |

The strict per-instruction stop, interactive host-input timing, guest filesystem
health and physical power-cut guarantees are intentionally outside the contract;
they are not missing tests for promised behavior. Static inspection of the media
and adapter execution paths finds no wall-clock input; behavioral comparisons
exercise that property for the recorded guest inputs, rather than proving it
for all possible programs or future edits.

### Validation mutations

Each newly added behavior has a matching isolated mutation that fails its targeted
test: publishing sectors before COMMAND_END (crash-prefix model), adding a host
read to command access (mounted-I/O boundary), turning a latched error into write
protection (ENOSPC/EIO), mirroring only the first replacement (full-result/replay),
dropping memory updates after a host error (complete failed prefix and MockBus
reset/multi-track checks), skipping rollback truncation (fresh guest DMA), writing
the volatile base (fixture hash), and creating a volatile sidecar (directory
entries). Mutations run only in a disposable source copy with copied fixture
bytes; no fixture target or source file in the tested worktree is mutated.
The walkthrough in [commands.md](../commands.md#journal-session-walkthrough)
was run against generated media in a temporary directory; its excerpts are
captured output with relative filenames.

Review follow-up evidence: identical independent guest sessions now compare the
complete journal files directly, including hashes, sequence/generation fields and
closing markers. A path-dependent but otherwise valid closing reason fails this
test. Fixture pin tests read the output hashes from `include_str!` vendor scripts;
missing, duplicate and malformed pin definitions fail explicitly. Their targeted
mutations change a script pin selector and bypass hash syntax validation. These
checks use the same disposable-copy rule as the validation above.

## Media changes

Floppy catalogues keep one independent persistence handle per journaled medium.
Only the inserted medium has an active journal. Assembly reads, SHA-256-checks
and geometry-checks every configured base before opening any journal, and
validates all output/lock/temporary paths against all bases and configuration
inputs, including inactive media. Inactive journal contents are validated when
that medium is inserted. The saved immutable base and metadata are reused for
reopening; insertion does not reread a potentially changed base file.

At a completed execution boundary, eject calls the same `finish` API as a
successful run: synchronize and close a session-end generation, run retention,
keep readable overlay bytes, release the lock. This also closes an empty
session after an insert with no writes: the current journal API deliberately
records empty recovery points and has no clean no-op close. Dropping a session
is the error/recovery path, not successful completion. Each eject therefore
consumes one retained generation; with the default ten, repeated swaps can
prune older rollback points even without writes. Export important states or
raise `keep_generations` before such a session. A future no-op close must
preserve recovery and lock-release guarantees before this policy can change.
Reinsert opens the journal anew
and replaces the adapter's backing session with replayed state. Failed seals
or opens stop the frontend before another guest instruction. No disk-change
error is substituted for a host persistence failure. Volatile media retain their
working overlay while parked; readonly policy belongs to each medium.
All guest sectors still pass through `SectorMedia`; the immutable base binding,
full-base SHA-256 and separate sector replacements are unchanged. A future
chain of hash-linked layers can replace that implementation at the same seam.

The IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
Digital Input Register (3F7h) and Interface Lines, defines the selected
change line. Eject and insertion assert it independently for each drive. A
moving SEEK or RECALIBRATE step sequence clears the DOR-selected physical
drive's line with media present, even when the command names another drive; an empty
drive cannot clear it. DIR bit 7 samples the DOR-selected drive; bits 6–0 remain
zero in this model. Power-on asserts the line only for a configured drive with
`empty = true`. Unconfigured positions have no drive asserting the line and
start clear, as do drives starting with a medium for compatibility.
FDC snapshot version 5 includes selected catalogue indices, empty slots and
change lines. Versions 3 and 4 restore medium zero and clear lines. Media bytes,
journal sessions and the catalogue itself remain external snapshot inputs;
restoration requires a compatible catalogue and does not roll back disk writes.
Device-only restore rejects any selection change in a slot containing a
journaled medium, including transitions to/from empty and legacy defaults.
Otherwise it would bypass the persistence handle's seal/reopen boundary.
Same-medium restores remain supported. A future machine snapshot API must
coordinate these journal sessions and device state before changing that slot.

Hardware sources: [IBM PC AT Fixed Disk and Diskette Drive Adapter, Digital
Input Register, pp. 12–13, and Interface Lines, p. 40](https://minuszerodegrees.net/oa/OA%20-%20IBM%20PC%20AT%20Fixed%20Disk%20and%20Diskette%20Drive%20Adapter.pdf);
[IBM PC AT High Capacity Diskette Drive, Output Signals, -Diskette Change,
p. 5](https://minuszerodegrees.net/oa/OA%20-%20IBM%20PC%20AT%20High%20Capacity%20Diskette%20Drive.pdf).
The power-on clear line for inserted media is a deliberate compatibility policy;
physical drive output can start asserted until its first step with a disk present.

## Native zero disks

Prepared launches may give a fixed medium `zero = { size = N }` instead of
an external `head`. These sources have no content row or required blob.
Size is positive, sector aligned and equal to the explicit IDE geometry.
The image codec lowers them to Blank, and assembly builds a verified zero
sector source before any byte-vector fallback can materialize the disk.

`journal::VerifiedBase::zero` computes the actual SHA-256 of all N zero bytes
with a fixed 64 KiB streaming buffer at runtime, never during discovery or
composition. The immutable source retains only its length. Volatile and
readonly media use SparseOverlay; protected writes are refused and volatile
replacements disappear with the mount. Journal mode uses the same verified
base hash and length in its header and replays separate replacements on reopen.
Memory grows with replaced sectors, independently of disk capacity.

Current-reader raw, fixed-VHD and machine exports also work for native zeros.
They emit current sector bytes to new outputs and never rewrite the generated
base or an existing external base. The single sector-source/overlay boundary
remains suitable for future hash-linked layers.
