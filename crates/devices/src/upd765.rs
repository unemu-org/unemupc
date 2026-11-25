// SPDX-License-Identifier: AGPL-3.0-or-later
//! NEC µPD765 / Intel 8272A floppy disk controller (§7.8).
//!
//! Models the FDC as wired on a PC/AT: the digital output register (`0x3F2`),
//! the main status register (`0x3F4`), the data register (`0x3F5`) that carries
//! the command / parameter / result byte stream, and the digital input /
//! configuration-control register (`0x3F7`). It drives the classic three-phase
//! protocol — **command** (opcode + parameters), **execution** (the data
//! transfer), **result** (status bytes) — and raises **IRQ6** on command
//! completion.
//!
//! ## Data transfer runs through the 8237 DMA
//!
//! A real FDC does not touch memory; it hands each byte to the DMA controller
//! over the DREQ/DACK handshake, and the 8237 writes it to the address the guest
//! programmed. This model keeps that separation: on a READ, the controller pulls
//! the sector bytes through its mounted media and **queues** them as a
//! [`device::DmaRequest`] on channel 2. The machine drains the request, streams
//! the bytes through the 8237's channel (which writes guest RAM through the
//! SMC-safe `note_write` path, §4.3), and reports the byte count and TC through
//! [`Device::dma_complete`] — at which point the controller finishes its result
//! phase and raises IRQ6. The controller never holds a memory reference and the
//! 8237 never sees the disk.
//!
//! READ/WRITE DATA honour MT: after EOT on head 0, a multi-track command
//! continues at sector 1 on head 1 of the same cylinder. Each sector is checked
//! against the physical geometry before mapping it to a media LBA.
//! See NEC µPD765 datasheet, Functional Description of Commands, READ DATA /
//! WRITE DATA and Table 2; Intel 8272A datasheet, Command Descriptions, READ DATA
//! / WRITE DATA and Table 8.
//!
//! ## Diskette changes
//!
//! IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
//! Digital Input Register (3F7h) and Interface Lines: DIR bit 7 samples the
//! DOR-selected drive's change line. IBM PC AT High Capacity Diskette Drive,
//! Output Signals, -Diskette Change, requires a disk and a step pulse to clear
//! the signal. Configured empty-start drives assert it; absent positions and
//! mounted drives start clear for compatibility. Eject and insert set it.
//! A moving SEEK or RECALIBRATE step
//! clears the DOR-selected drive's line only with media present.
//! RECALIBRATE uses a collapsed step sequence even at PCN zero; separate
//! physical head position is not modelled. Bits 6–0 remain zero, including the
//! unmodelled fixed-disk signals. Controller
//! reset preserves the door state and change line. See persistent-media.md § Media changes.
//!
//! ## Geometry and per-drive media
//!
//! Geometry is per-drive data, not a hardcoded constant. Each of the four
//! drive-select positions the DOR can address may carry its own
//! [`Drive`]: a [`FloppyGeometry`] (cylinders × heads × sectors, 512-byte
//! sectors) plus owned [`SectorMedia`]. The four standard PC floppy formats are
//! provided as [`FloppyGeometry`] constants — 1.44 MB (80×2×18), 1.2 MB
//! (80×2×15), 720 KB (80×2×9), and 360 KB (40×2×9) — so drive 0 and drive 1 can
//! differ. The installed mechanism is a separate [`DriveType`]: a 1.44 MB drive
//! also takes 720 KB media and a 1.2 MB drive 360 KB media (see Drive and
//! medium types below). A CHS address maps to an image offset with the
//! inserted medium's own geometry by
//! `lba = (cyl * heads + head) * sectors_per_track + (sector - 1)`,
//! `offset = lba * 512`. An empty bay supplies no index pulses: READ ID and
//! READ/WRITE DATA wait in execution until reset or insertion. An address outside mounted
//! media's geometry terminates abnormally with a no-data status.
//!
//! Media serves sector reads and accepts one ordered batch of complete sector
//! replacements at WRITE DATA completion, before result/IRQ6. Partial sectors
//! are zero-filled, and sectors preceding an abnormal result are included. Empty
//! completions and rejected writes submit no batch. Write protection belongs to
//! the medium. The default [`InMemoryMedia`] changes only its volatile bytes.
//! The contract exposes no host I/O, clock or host-error result to the controller;
//! future adapters must latch host failures outside the guest's view. Reset keeps
//! the same media; snapshots exclude it. See
//! `backlog/docs/design/persistent-media.md`, §Integration.
//!
//! ## Reset polls and seek completion
//!
//! Reset release queues four SENSE INTERRUPT STATUS results (C0h–C3h, PCN=0).
//! ASSUMED for silicon: a SEEK or RECALIBRATE completion discards any unread
//! reset polls so the next sense reports that completion, then 80h when idle.
//! NEC µPD765 datasheet, Functional Description of Commands, SENSE INTERRUPT
//! STATUS, and Intel 8272A datasheet, Command Descriptions, SENSE INTERRUPT
//! STATUS, do not define this interaction. Bochs clears its reset sense count
//! when raising a new interrupt. MAME instead keeps pending ST0 per drive and
//! senses the lowest pending drive. The reset-poll policy here is Bochs-style,
//! limited to seek and recalibrate completion. Four reset polls remain available
//! when no such completion intervenes.
//!
//! Known gap: overlapped seek completions are not modelled; one latch holds the
//! latest completion, even across drives. Intel 8272A datasheet, Command
//! Descriptions, SEEK, describes overlapped seeks on up to four drives.
//!
//! ## Coverage matrix (datasheet vs modelled)
//!
//! | Command                        | Opcode | Modelled | Notes                          |
//! |--------------------------------|--------|----------|--------------------------------|
//! | SPECIFY                        | `0x03` | yes      | timings accepted, not enforced |
//! | SENSE DRIVE STATUS             | `0x04` | yes      | ST3; no interrupt or seek-state change |
//! | RECALIBRATE                    | `0x07` | partial  | PCN=0; installed, even empty: SE; absent: SE+EC, abnormal IRQ6 |
//! | SENSE INTERRUPT STATUS         | `0x08` | yes      | ST0 + PCN, or invalid if idle; ASSUMED: seek/recalibrate completion discards remaining reset polls |
//! | SEEK                           | `0x0F` | yes      | set present cylinder, IRQ6     |
//! | READ ID                        | `0x4A` | partial  | matching rate: synthetic R=1/N=2; empty: waits for index |
//! | READ DATA                      | `0x06` | yes      | DMA transfer to memory, IRQ6   |
//! | WRITE DATA                     | `0x05` | yes      | DMA transfer from memory, IRQ6 |
//! | CCR data rate                  | `3F7h` write | yes | 500/300/250 kbps; rate mismatch faults with ST1 MA |
//! | Rate-dependent timing          | —      | no       | transfer/ID-search delays are collapsed |
//!
//! IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
//! Diskette Control Register (3F7h), p. 15: bits 1-0 select 00=500, 01=300,
//! 10=250 kbps; 11 is unused. ASSUMED: 11 is retained but matches no medium.
//! That section gives no reset value. Intel 82077AA datasheet, §8.2
//! Initialization, specifies 250 kbps on hardware reset; emulators such as
//! Bochs also use that alternative. ASSUMED: this AT adapter's power-on and
//! Device::reset select 500 kbps to preserve the default high-density boot
//! configuration. DOR reset preserves this adapter latch, separate from the
//! FDC command reset (IBM reference, Digital Output Register, p. 12).
//!
//! ## Drive and medium types
//!
//! IBM Personal Computer AT Technical Reference, High Capacity Diskette Drive:
//! the 1.2 MB drive spins at 360 rpm, reads 1.2 MB media at 500 kbps and
//! 320/360 KB media at 300 kbps; the BIOS steps it twice per track for the
//! 40-track media. A 360 KB drive reads its media at 250 kbps. ASSUMED for the 3.5-inch drives, which that reference predates:
//! a 1.44 MB drive reads 1.44 MB media at 500 kbps and 720 KB media at
//! 250 kbps over the same 80 cylinders, and a 720 KB drive takes only 720 KB
//! media at 250 kbps. The inserted medium in its drive sets the rate it
//! answers at; any other CCR rate faults with ST1 MA as below. Custom
//! geometries form a drive type of their own that takes only that geometry;
//! ASSUMED: they record at 500 kbps above nine sectors per track and at
//! 250 kbps otherwise.
//!
//! A 1.2 MB drive has 80 physical cylinders, and a 360 KB medium has 40
//! tracks, written at the even physical cylinders. The PCN is the head's
//! physical cylinder: track `PCN / 2` lies under the head at an even PCN, and
//! its ID fields carry that track number as C. At an odd PCN the head sits
//! between tracks, so READ ID and READ/WRITE DATA find no ID field and end with
//! ST1 MA. A command C that differs from the track's ID cylinder ends with
//! ST1 ND and ST2 WC (NEC µPD765 datasheet, Status Register 2, Wrong Cylinder).
//! ASSUMED, as before: with one physical cylinder per track, READ/WRITE DATA
//! address the command's C directly, without comparing it with the PCN.
//!
//! NEC µPD765 datasheet, READ ID / READ DATA / WRITE DATA and Status Register 1,
//! MA; Intel 8272A datasheet, Command Descriptions and Table 12, Status Registers:
//! a missing ID address mark sets ST1 MA and ends abnormally. ASSUMED: a track
//! without readable ID fields returns MA only (01h). Some references report
//! MA|ND (05h) for READ ID / READ DATA; the distinction can affect a BIOS's
//! INT 13h status (02h versus 04h). Rate mismatch uses the MA-only result on
//! mounted media, at the final command byte like other pre-transfer faults,
//! without DMA. WRITE DATA checks write protection before the ID search: NW
//! wins over MA/ND. Otherwise MA precedes CHS-based ND, because the wrong rate
//! prevents decoding any ID field. The two-index search delay is collapsed;
//! empty bays still wait for index. READ ID faults use PCN/HD/R=1/N=2, the
//! synthetic ID convention; DATA faults echo C/R/N and physical HD as below.
//! Seek, recalibrate and sense commands do not depend on the selected rate.
//!
//! IBM Personal Computer AT Technical Reference, Fixed Disk and Diskette
//! Drive Adapter, Digital Input Register (3F7h) and Interface Lines: connector
//! pin 34 is Diskette Change, not READY. The board model holds the FDC's RDY
//! active for all four selects, consistent with the four RESET ready-change
//! polls (NEC µPD765A datasheet, RESET / SENSE INTERRUPT STATUS).
//! ASSUMED: RDY is held high independently of the selected slot; the available
//! adapter logic diagram, sheet 4 of 9, does not resolve the tie unambiguously.
//! IBM PC AT Technical Reference, High Capacity Diskette Drive, Output Signals,
//! -Track 00: head position supplies TRACK 0 independently of media. ASSUMED:
//! head position is taken from PCN, so an installed drive supplies TRACK 0 at
//! PCN zero, including without a disk. Absent positions supply neither TRACK 0
//! nor index; only mounted media supplies index pulses. NEC µPD765A datasheet,
//! SEEK / RECALIBRATE and Status Register 0: SEEK reaches NCN normally;
//! RECALIBRATE clears PCN and ends with SE when TRACK 0 is reached. An absent
//! drive ends with SE+EC and IC=01 after 77 unsuccessful steps. The step delay
//! is collapsed, as for mounted seeks. READ ID / ST1 MA requires index pulses
//! to end an unsuccessful ID search; without them there is no result or IRQ.
//! This also applies to READ/WRITE DATA's ID search. The controller remains
//! Dormant deterministically; a BIOS's timer IRQs or polling instruction budget
//! can expire its software timeout, then DOR reset aborts the search and makes
//! the four reset SIS results available. No controller timeout is invented.
//! Intel 82077AA datasheet, §8.1 Command and Result Phase Handshaking, likewise
//! requires a software timeout when an empty drive supplies no index pulses.
//!
//! SENSE DRIVE STATUS takes a drive/head parameter and returns one ST3 byte
//! without raising or acknowledging INT. NEC µPD765 datasheet, Functional
//! Description of Commands, SENSE DRIVE STATUS and Status Register 3; Intel
//! 8272A datasheet, Command Descriptions, SENSE DRIVE STATUS and Status Register
//! 3: ST3 reports FT/WP/RY/T0/TS/HD/US1/US0 from bit 7 down to bit 0.
//! HD and US echo the parameter; RY follows the active RDY policy above.
//! ASSUMED: TS comes from the mounted geometry's two heads, and T0 from the
//! installed drive's PCN=0, independently of media. An installed empty drive
//! clears TS/WP; an absent position also clears T0. FT stays clear because
//! drive faults are not modelled; WP follows the medium's write protection.
//! ASSUMED: T0/WP/TS follow the command's unit regardless of DOR drive-select
//! and motor bits. The AT adapter's selected, motor-on drive supplies these
//! lines (IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
//! Interface Lines); the model does not gate them.
//!
//! IBM PC AT Technical Reference, Digital Output Register (3F2h), bit 3 and
//! adapter logic diagram sheet 5 of 9: IRQ6 is pending interrupt AND bit 3.
//! ASSUMED: a DMA request is retained while the same gate is closed. Real
//! hardware would end a data transfer with an overrun (ST1 OR) when bytes are
//! not serviced in time; this model has no data-rate timing, so it waits.
//!
//! Deliberately **not** modelled:
//! - **Rotating READ ID**: the datasheet's first ID encountered depends on
//!   rotational phase, absent from the flat-sector image. The model always
//!   chooses R=1/N=2 on a present track, rather than inventing angular position.
//! - **Fault-result CHRN without a readable ID**: NEC µPD765A datasheet,
//!   Command Symbol Description (H/HD) and Instruction Set require H=HD in
//!   valid commands; no mismatched-head result is specified. Assumption:
//!   pre-transfer failures echo command C/R/N and physical HD as result H;
//!   an empty-bay READ ID has no result CHRN. Logical-ID error reporting
//!   needs an ID-field/rotational model and is not claimed.
//! - **WRITE DELETED DATA** (`0x09`) and **FORMAT TRACK**: the modelled write
//!   path is a plain WRITE DATA. A protected disk is still honoured — WRITE DATA
//!   to one faults with ST1 not-writable.
//! - **Non-DMA (PIO) transfer mode**: transfers always go through the 8237. The
//!   DOR's DMA-enable bit gates IRQ6 and DMA requests, but the controller has
//!   no data-port streaming path.
//! - **Cycle-accurate timing**: SPECIFY's step-rate / head-load / head-unload
//!   times are accepted and discarded; seeks complete immediately, and mounted
//!   reads finish on DMA completion. No rotational-latency model exists; empty
//!   bays never synthesize index pulses.
//! - **The skip (SK) modifier and MFM/FM selection**: SK and MFM do not change
//!   the transfer; READ and WRITE treat the raw image as normal MFM sectors.
//!
//! DMA completion carries TC separately from the byte count. TC ends normally;
//! without TC a short transfer reports overrun, a missing sector reports no data,
//! and exhausting EOT reports end of cylinder (NEC µPD765 datasheet, READ DATA,
//! WRITE DATA and Status Register 1).
//!
//! Snapshots contain registers, command/result phases, pending DMA context
//! and request data, and the absolute trace time base. Mounted media and
//! installed-drive wiring belong to the receiving instance and must match the
//! saved controller. This model has no timed execution deadlines (NEC µPD765
//! datasheet, Execution Phase).
//! Snapshot v4 appends the CCR rate encoding to v3's layout. V3 restores with
//! the reset rate (500 kbps), because it did not retain CCR writes. V3 stores
//! the interrupt latch before the DOR gate and permits an empty-bay ID search
//! without DMA. Version 5 adds catalogue indices and change lines; v3/v4 restore
//! with medium zero and clear lines. Versions before v3 are rejected. Restore
//! rejects selection changes in any slot containing journaled media: only the
//! owning machine can seal/reopen those sessions. A future machine snapshot API
//! must coordinate that persistence boundary before restoring devices. Restore
//! validates the waiting opcode, full command, empty slot, and absence of
//! results and DMA before accepting that execution phase.

use device::{
    AccessDir, Device, DeviceError, DmaOp, DmaOutcome, DmaRequest, InMemoryMedia, NextEvent,
    SectorMedia, SectorWrite, TraceEvent, VClock,
};

/// Stable device name reported by [`Device::name`].
const NAME: &str = "upd765-fdc";

/// First port decoded by the controller.
const PORT_BASE: u16 = 0x3F0;
/// Last port decoded by the controller.
const PORT_END: u16 = 0x3F7;
/// Digital output register (motor / drive-select / reset / DMA-enable).
const DOR: u16 = 0x3F2;
/// Main status register (read-only).
const MSR: u16 = 0x3F4;
/// Command / parameter / result data register.
const DATA: u16 = 0x3F5;
/// Digital input register (read) / configuration control register (write).
const DIR_CCR: u16 = 0x3F7;

/// DOR bit 2: `0` holds the controller in reset, `1` is normal operation.
const DOR_NOT_RESET: u8 = 1 << 2;
/// DOR bit 3: enable the DMA / interrupt path.
const DOR_DMA_ENABLE: u8 = 1 << 3;
/// DOR bit 4: drive 0's motor; bits 5–7 switch drives 1–3.
const DOR_MOTOR_0: u8 = 1 << 4;

/// MSR bit 7 (RQM): the data register is ready for a host transfer.
const MSR_RQM: u8 = 1 << 7;
/// MSR bit 6 (DIO): transfer direction — set means FDC→host (result phase).
const MSR_DIO: u8 = 1 << 6;
/// MSR bit 4 (CB): a command is in progress.
const MSR_BUSY: u8 = 1 << 4;

/// The DMA channel the PC floppy controller is wired to.
const FLOPPY_DMA_CHANNEL: u8 = 2;

/// Bytes per sector. Every standard PC floppy format uses 512-byte sectors.
const SECTOR_SIZE: usize = device::media::SECTOR_SIZE;

/// Total size of a 1.44 MB image in bytes, kept as a public constant for the
/// callers that seed a default high-density disk.
pub const FLOPPY_IMAGE_BYTES: usize = FloppyGeometry::HD1440.image_bytes();

/// The physical layout of a floppy disk: cylinders, heads, and sectors per
/// track, with a fixed 512-byte sector. This is the data the CHS→offset mapping
/// and the total image size are derived from, so a drive's format is a value the
/// machine configures rather than a constant baked into the controller.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FloppyGeometry {
    /// Number of cylinders (tracks per side).
    pub cylinders: u32,
    /// Number of heads (recording surfaces): 1 single-sided, 2 double-sided.
    pub heads: u32,
    /// Sectors per track.
    pub sectors_per_track: u32,
}

impl FloppyGeometry {
    /// 1.44 MB high-density 3.5″ floppy: 80 cylinders × 2 heads × 18 sectors.
    pub const HD1440: Self = Self {
        cylinders: 80,
        heads: 2,
        sectors_per_track: 18,
    };
    /// 1.2 MB high-density 5.25″ floppy: 80 cylinders × 2 heads × 15 sectors.
    pub const HD1200: Self = Self {
        cylinders: 80,
        heads: 2,
        sectors_per_track: 15,
    };
    /// 720 KB double-density 3.5″ floppy: 80 cylinders × 2 heads × 9 sectors.
    pub const DD720: Self = Self {
        cylinders: 80,
        heads: 2,
        sectors_per_track: 9,
    };
    /// 360 KB double-density 5.25″ floppy: 40 cylinders × 2 heads × 9 sectors.
    pub const DD360: Self = Self {
        cylinders: 40,
        heads: 2,
        sectors_per_track: 9,
    };

    /// Recording rate in the AT adapter's CCR encoding in the medium's own
    /// drive type: the standard HD formats use 500 kbps and DD formats
    /// 250 kbps. [`DriveType`] adjusts it for a 360 KB medium in a 1.2 MB drive.
    /// ASSUMED: custom layouts use the same density split at nine sectors.
    fn recording_rate(&self) -> u8 {
        if self.sectors_per_track > 9 {
            0
        } else {
            2
        }
    }

    /// Total sectors described by the geometry, saturating if it exceeds `u64`.
    #[must_use]
    pub const fn sector_count(&self) -> u64 {
        (self.cylinders as u64)
            .saturating_mul(self.heads as u64)
            .saturating_mul(self.sectors_per_track as u64)
    }

    /// Total size in bytes of a raw image with this geometry.
    #[must_use]
    pub const fn image_bytes(&self) -> usize {
        (self.cylinders * self.heads * self.sectors_per_track) as usize * SECTOR_SIZE
    }

    /// Byte offset of a CHS sector within an image of this geometry. Sector
    /// numbers are 1-based on the wire, so sector 1 is the first on the track.
    fn chs_to_offset(&self, cylinder: u8, head: u8, sector: u8) -> usize {
        let lba = (u32::from(cylinder) * self.heads + u32::from(head)) * self.sectors_per_track
            + u32::from(sector.saturating_sub(1));
        lba as usize * SECTOR_SIZE
    }

    /// Whether a CHS address lies within this geometry (a valid, readable
    /// sector). Sectors are 1-based, so sector 0 is out of range.
    fn contains(&self, cylinder: u8, head: u8, sector: u8) -> bool {
        u32::from(cylinder) < self.cylinders
            && u32::from(head) < self.heads
            && sector >= 1
            && u32::from(sector) <= self.sectors_per_track
    }
}

/// CCR encoding of 300 kbps (IBM PC AT Technical Reference, Diskette Control
/// Register (3F7h)).
const RATE_300_KBPS: u8 = 1;

/// The drive mechanism installed at a drive-select position, separate from
/// the medium inserted in it. A type is named by its own, densest format,
/// whose cylinder count is the drive's physical cylinder count; see the
/// module's Drive and medium types for the media each type takes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DriveType {
    native: FloppyGeometry,
}

impl DriveType {
    /// 3.5-inch 1.44 MB drive; also takes 720 KB media.
    pub const HD1440: Self = Self::new(FloppyGeometry::HD1440);
    /// 5.25-inch 1.2 MB drive; also takes 360 KB media, stepped twice per track.
    pub const HD1200: Self = Self::new(FloppyGeometry::HD1200);
    /// 3.5-inch 720 KB drive.
    pub const DD720: Self = Self::new(FloppyGeometry::DD720);
    /// 5.25-inch 360 KB drive.
    pub const DD360: Self = Self::new(FloppyGeometry::DD360);

    /// The drive type whose own format is `native`.
    #[must_use]
    pub const fn new(native: FloppyGeometry) -> Self {
        Self { native }
    }

    /// The drive's own format.
    #[must_use]
    pub const fn native(&self) -> FloppyGeometry {
        self.native
    }

    /// The media this drive takes, its own format first.
    pub fn media(&self) -> impl Iterator<Item = FloppyGeometry> {
        let lower = if *self == Self::HD1440 {
            Some(FloppyGeometry::DD720)
        } else if *self == Self::HD1200 {
            Some(FloppyGeometry::DD360)
        } else {
            None
        };
        std::iter::once(self.native).chain(lower)
    }

    /// Whether a medium of `geometry` fits this drive.
    #[must_use]
    pub fn accepts(&self, geometry: FloppyGeometry) -> bool {
        self.media().any(|medium| medium == geometry)
    }

    /// The CCR rate an accepted `medium` answers at in this drive: a 360 KB
    /// medium turns at 360 rpm in a 1.2 MB drive, so it reads at 300 kbps.
    fn data_rate(&self, medium: FloppyGeometry) -> u8 {
        if self.double_steps(medium) {
            RATE_300_KBPS
        } else {
            medium.recording_rate()
        }
    }

    /// Whether a medium of `medium` has its tracks at every second physical
    /// cylinder of this drive (40-track media in an 80-cylinder 1.2 MB drive).
    fn double_steps(&self, medium: FloppyGeometry) -> bool {
        *self == Self::HD1200 && medium == FloppyGeometry::DD360
    }
}

/// A floppy medium for one of the controller's four drive positions: its
/// [`FloppyGeometry`] and owned sector media. The drive mechanism it sits in
/// is the position's [`DriveType`].
pub struct Drive {
    /// Physical format of the disk.
    pub geometry: FloppyGeometry,
    media: Box<dyn SectorMedia>,
    journaled: bool,
}

impl Drive {
    /// Mount a writable in-memory `image` with the given `geometry`.
    #[must_use]
    pub fn new(geometry: FloppyGeometry, image: Vec<u8>) -> Self {
        Self::with_media(
            geometry,
            Box::new(InMemoryMedia::new(image, geometry.sector_count(), false)),
        )
    }

    /// Mount a write-protected in-memory image; WRITE DATA returns not-writable.
    #[must_use]
    pub fn new_read_only(geometry: FloppyGeometry, image: Vec<u8>) -> Self {
        Self::with_media(
            geometry,
            Box::new(InMemoryMedia::new(image, geometry.sector_count(), true)),
        )
    }

    /// Mount sector media without exposing its host-side state to the controller.
    #[must_use]
    pub fn with_media(geometry: FloppyGeometry, media: Box<dyn SectorMedia>) -> Self {
        Self {
            geometry,
            media,
            journaled: false,
        }
    }

    /// Mark a host-owned journal so device snapshots cannot change its session
    /// without the owning machine's persistence boundary.
    #[must_use]
    pub fn with_journaled_media(geometry: FloppyGeometry, media: Box<dyn SectorMedia>) -> Self {
        Self {
            geometry,
            media,
            journaled: true,
        }
    }

    /// The medium's controller-visible write-protection property.
    #[must_use]
    pub fn write_protected(&self) -> bool {
        self.media.write_protected()
    }

    /// Whether the host keeps the guest's writes in a journal.
    #[must_use]
    pub fn journaled(&self) -> bool {
        self.journaled
    }
}

/// A host media event must address a configured catalogue at a completed DMA boundary.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MediaChangeError {
    /// Drive-select position is outside the adapter.
    #[error("unknown floppy drive")]
    UnknownDrive,
    /// The requested catalogue entry does not exist.
    #[error("unknown floppy medium")]
    UnknownMedium,
    /// DMA must complete before opening the drive door.
    #[error("floppy DMA is still pending")]
    DmaPending,
    /// The drive type does not take a medium of this geometry.
    #[error("the drive type does not take this medium geometry")]
    GeometryMismatch,
}

/// Snapshot layout version tag.
const SNAPSHOT_VERSION: u8 = 5;

/// Assumed adapter reset rate: CCR 00 selects 500 kbps (see module coverage).
const RESET_DATA_RATE: u8 = 0;

/// ST0 bit 5: seek end (set after a completed RECALIBRATE / SEEK).
const ST0_SEEK_END: u8 = 1 << 5;
/// ST0 interrupt-code bits 7-6 == `11`: abnormal termination / ready-line change,
/// reported by each of the four polling SENSE INTERRUPT STATUS calls a reset
/// raises.
const ST0_POLL: u8 = 0b1100_0000;
/// ST0 interrupt-code bits 7-6 == `01`: the command terminated abnormally.
const ST0_ABNORMAL: u8 = 0b0100_0000;
/// ST0 bit 4 (EC): TRACK 0 was not reached within 77 recalibration steps.
const ST0_EQUIPMENT_CHECK: u8 = 1 << 4;
/// NEC µPD765 / Intel 8272A datasheets, Status Register 1, Missing Address
/// Mark: no ID address mark detected after two index pulses.
const ST1_MISSING_ADDRESS_MARK: u8 = 1;
/// ST1 bit 2 (ND): no data — the requested sector was not found on the track.
/// NEC µPD765 / Intel 8272A datasheets, Status Register 1, No Data.
const ST1_NO_DATA: u8 = 1 << 2;
/// ST1 bit 1 (NW): not writable — a WRITE addressed a write-protected disk.
const ST1_NOT_WRITABLE: u8 = 1 << 1;
/// NEC µPD765 / Intel 8272A datasheets, Status Register 1, End of Cylinder.
const ST1_END_OF_CYLINDER: u8 = 1 << 7;
/// NEC µPD765 / Intel 8272A datasheets, Status Register 1, Overrun.
const ST1_OVERRUN: u8 = 1 << 4;
/// NEC µPD765 / Intel 8272A datasheets, Status Register 2, Wrong Cylinder:
/// the ID field's C differs from the command's C.
const ST2_WRONG_CYLINDER: u8 = 1 << 4;

/// NEC µPD765 / Intel 8272A datasheets, Status Register 3, Two Side.
const ST3_TWO_SIDE: u8 = 1 << 3;
/// NEC µPD765 / Intel 8272A datasheets, Status Register 3, Track 0.
const ST3_TRACK_ZERO: u8 = 1 << 4;
/// NEC µPD765 / Intel 8272A datasheets, Status Register 3, Ready.
const ST3_READY: u8 = 1 << 5;
/// NEC µPD765 / Intel 8272A datasheets, Status Register 3, Write Protected.
const ST3_WRITE_PROTECTED: u8 = 1 << 6;

/// Which phase of the command protocol the controller is in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Phase {
    /// Idle, ready to accept a command opcode.
    Idle,
    /// Collecting the parameter bytes that follow an opcode.
    Command,
    /// Running a transfer (bytes move over DMA, not the data port).
    Execution,
    /// Handing result status bytes back to the host.
    Result,
}

/// Command framing and result interrupt acknowledgement. NEC µPD765A/µPD765B
/// datasheet, Instruction Set; Intel 8272A datasheet, Registers — CPU Interface.
struct CommandInfo {
    length: usize,
    result_interrupt: bool,
}

/// Context captured when a READ DATA enters execution, used to build the result
/// phase once the DMA transfer completes.
#[derive(Copy, Clone, Debug)]
struct PendingRead {
    drive: u8,
    head: u8,
    cylinder: u8,
    sector: u8,
    sector_size_code: u8,
    requested: usize,
    missing_sector: bool,
}

/// Context captured when a WRITE DATA enters execution, used to commit the DMA
/// bytes to media and build the result phase once the transfer completes.
///
/// Unlike [`PendingRead`] this also carries `param_head` — the head field from
/// the command parameters — because committing the bytes needs the same
/// CHS→offset mapping the read path uses. `head` retains the drive-select head
/// from which the result ID advances.
#[derive(Copy, Clone, Debug)]
struct PendingWrite {
    drive: u8,
    head: u8,
    param_head: u8,
    cylinder: u8,
    sector: u8,
    sector_size_code: u8,
    requested: usize,
    missing_sector: bool,
}

/// The NEC µPD765 / Intel 8272A floppy disk controller (§7.8).
///
/// Decodes `0x3F0..=0x3F7`; the modelled registers are the DOR, MSR, data, and
/// DIR/CCR ports. Drives IRQ6 through [`Device::irq_pending`]; the 8259 PIC
/// owns interrupt vectoring.
pub struct Upd765 {
    /// Digital output register: motor-on bits, drive select, reset, DMA-enable.
    dor: u8,
    /// Adapter CCR bits 1-0; independent of the DOR's FDC reset bit.
    data_rate: u8,
    /// Current protocol phase.
    phase: Phase,
    /// Command opcode plus the parameter bytes collected so far.
    command: Vec<u8>,
    /// Total bytes (opcode + parameters) expected before execution.
    expected: usize,
    /// Result status bytes queued for the host to read.
    result: Vec<u8>,
    /// Index of the next result byte to hand back.
    result_index: usize,
    /// Present cylinder number per drive (advanced by SEEK / RECALIBRATE).
    present_cylinder: [u8; 4],
    /// Status register 0 from the last completed command.
    st0: u8,
    /// Controller interrupt latch, before the DOR bit-3 bus gate.
    irq: bool,
    /// Remaining post-reset SENSE INTERRUPT STATUS polls that report the
    /// abnormal / ready-change status a controller reset raises for each drive.
    reset_poll: u8,
    /// A transfer queued for the machine to run through the 8237 (READ moves
    /// device→memory, WRITE moves memory→device).
    pending_request: Option<DmaRequest>,
    /// The READ context whose result phase is completed once the DMA finishes.
    pending_read: Option<PendingRead>,
    /// The WRITE context whose media commit and result phase follow the DMA.
    pending_write: Option<PendingWrite>,
    /// Media mounted in each of the four drive positions the DOR can select.
    /// `None` supplies no index pulses; TRACK 0 depends on the installed drive.
    drives: [Option<Drive>; 4],
    /// Installed drive mechanisms. Drive wiring is configuration, independent of
    /// removable media and snapshots.
    drive_types: [Option<DriveType>; 4],
    media_lists: [Vec<Option<Drive>>; 4],
    inserted: [Option<usize>; 4],
    disk_changed: [bool; 4],
    /// Trace events observed since the last [`Device::take_trace`].
    trace: device::TraceBuffer,
    /// Virtual time of the most recent [`Device::tick`], stamped onto traces.
    now: VClock,
}

impl Upd765 {
    /// Create a controller in power-on state with no disk attached.
    #[must_use]
    pub fn new() -> Self {
        Self {
            dor: 0,
            data_rate: RESET_DATA_RATE,
            phase: Phase::Idle,
            command: Vec::new(),
            expected: 0,
            result: Vec::new(),
            result_index: 0,
            present_cylinder: [0; 4],
            st0: 0,
            irq: false,
            reset_poll: 0,
            pending_request: None,
            pending_read: None,
            pending_write: None,
            drives: [None, None, None, None],
            drive_types: [None; 4],
            media_lists: std::array::from_fn(|_| Vec::new()),
            inserted: [None; 4],
            disk_changed: [false; 4],
            trace: device::TraceBuffer::default(),
            now: VClock::ZERO,
        }
    }

    /// Create a controller with a single 1.44 MB disk mounted in drive 0, backed
    /// by `image` (typically [`FLOPPY_IMAGE_BYTES`] bytes). Equivalent to `with_drives` carrying
    /// one high-density drive.
    #[must_use]
    pub fn with_image(image: Vec<u8>) -> Self {
        Self::with_drives([
            Some(Drive::new(FloppyGeometry::HD1440, image)),
            None,
            None,
            None,
        ])
    }

    /// Create a controller with media mounted per drive position. Each entry
    /// carries its own geometry, so drive 0 and drive 1 may differ; each
    /// position gets the drive type whose own format is its medium's geometry.
    #[must_use]
    pub fn with_drives(drives: [Option<Drive>; 4]) -> Self {
        let drive_types = std::array::from_fn(|unit| {
            drives[unit]
                .as_ref()
                .map(|drive| DriveType::new(drive.geometry))
        });
        let inserted = std::array::from_fn(|unit| drives[unit].as_ref().map(|_| 0));
        let media_lists = std::array::from_fn(|unit| {
            if drives[unit].is_some() {
                vec![None]
            } else {
                Vec::new()
            }
        });
        Self {
            drives,
            drive_types,
            inserted,
            media_lists,
            ..Self::new()
        }
    }

    /// Install a drive of `drive_type` and configure its catalogue before
    /// power-on, even if empty. Every entry must be a medium the drive takes.
    /// The `empty` flag opens the door and asserts disk change, regardless of
    /// catalogue contents (see the module's AT interface policy).
    pub fn set_media_list(
        &mut self,
        unit: usize,
        drive_type: DriveType,
        media: Vec<Drive>,
        empty: bool,
    ) -> Result<(), MediaChangeError> {
        self.set_media_list_from(unit, drive_type, media, empty, 0)
    }

    /// [`Self::set_media_list`] with entry `first` in the drive at power-on
    /// instead of the first one. The drive holds it from power-on, so the
    /// disk-change line starts clear, as with the first entry: IBM PC AT
    /// Technical Reference, Fixed Disk and Diskette Drive Adapter, Digital
    /// Input Register (3F7h) reports a change only after the door opened.
    pub fn set_media_list_from(
        &mut self,
        unit: usize,
        drive_type: DriveType,
        media: Vec<Drive>,
        empty: bool,
        first: usize,
    ) -> Result<(), MediaChangeError> {
        if unit >= 4 {
            return Err(MediaChangeError::UnknownDrive);
        }
        if media
            .iter()
            .any(|entry| !drive_type.accepts(entry.geometry))
        {
            return Err(MediaChangeError::GeometryMismatch);
        }
        if !empty && !media.is_empty() && first >= media.len() {
            return Err(MediaChangeError::UnknownMedium);
        }
        self.drive_types[unit] = Some(drive_type);
        self.drives[unit] = None;
        self.inserted[unit] = None;
        self.media_lists[unit] = media.into_iter().map(Some).collect();
        if !empty && !self.media_lists[unit].is_empty() {
            self.drives[unit] = self.media_lists[unit][first].take();
            self.inserted[unit] = Some(first);
        }
        self.disk_changed[unit] = empty;
        Ok(())
    }

    /// Host input is accepted only after any DMA completion has been serviced.
    pub fn check_media_boundary(&self) -> Result<(), MediaChangeError> {
        if self.pending_read.is_some() || self.pending_write.is_some() {
            Err(MediaChangeError::DmaPending)
        } else {
            Ok(())
        }
    }

    /// Extend an installed drive's catalogue with a medium its drive type takes,
    /// without changing its door or medium. IBM PC AT Technical Reference,
    /// Fixed Disk and Diskette Drive Adapter, Interface Lines describes the
    /// separate door-change signal. Host catalogue edits do not assert it.
    pub fn append_medium(&mut self, unit: usize, medium: Drive) -> Result<usize, MediaChangeError> {
        self.check_append_medium(unit, medium.geometry)?;
        let index = self.media_lists[unit].len();
        self.media_lists[unit].push(Some(medium));
        Ok(index)
    }

    /// Undo an unselected host catalogue extension at a completed boundary.
    /// IBM PC AT Technical Reference, Diskette Drive Adapter: this does not move
    /// the door or assert the diskette-change line.
    pub fn truncate_catalogue(
        &mut self,
        unit: usize,
        length: usize,
    ) -> Result<(), MediaChangeError> {
        self.check_media_boundary()?;
        let list = self
            .media_lists
            .get(unit)
            .ok_or(MediaChangeError::UnknownDrive)?;
        if length > list.len() || self.inserted[unit].is_some_and(|index| index >= length) {
            return Err(MediaChangeError::UnknownMedium);
        }
        self.media_lists[unit].truncate(length);
        Ok(())
    }

    /// Validate a host catalogue extension before opening its backing store.
    pub fn check_append_medium(
        &self,
        unit: usize,
        geometry: FloppyGeometry,
    ) -> Result<(), MediaChangeError> {
        self.check_media_boundary()?;
        let drive_type = self
            .drive_type(unit)
            .ok_or(MediaChangeError::UnknownDrive)?;
        if !drive_type.accepts(geometry) {
            return Err(MediaChangeError::GeometryMismatch);
        }
        Ok(())
    }

    /// Eject at a completed run boundary. IBM PC AT Technical Reference,
    /// Fixed Disk and Diskette Drive Adapter, Digital Input Register (3F7h)
    /// and Interface Lines: opening the door asserts diskette change.
    pub fn eject(&mut self, unit: usize) -> Result<(), MediaChangeError> {
        if unit >= 4 {
            return Err(MediaChangeError::UnknownDrive);
        }
        if self.pending_read.is_some() || self.pending_write.is_some() {
            return Err(MediaChangeError::DmaPending);
        }
        if let Some(index) = self.inserted[unit].take() {
            self.media_lists[unit][index] = self.drives[unit].take();
        }
        self.disk_changed[unit] = true;
        Ok(())
    }

    /// Insert a catalogue entry at a run boundary; it asserts diskette change.
    pub fn insert(&mut self, unit: usize, index: usize) -> Result<(), MediaChangeError> {
        if unit >= 4 {
            return Err(MediaChangeError::UnknownDrive);
        }
        if index >= self.media_lists[unit].len() {
            return Err(MediaChangeError::UnknownMedium);
        }
        self.eject(unit)?;
        self.drives[unit] = self.media_lists[unit][index].take();
        self.inserted[unit] = Some(index);
        if self.phase == Phase::Execution
            && self
                .command
                .get(1)
                .is_some_and(|value| usize::from(value & 3) == unit)
        {
            // A search parked without index pulses can see the newly inserted disk.
            self.execute();
        }
        Ok(())
    }

    /// The drive mechanism installed at `unit`, or `None` for an absent position.
    #[must_use]
    pub fn drive_type(&self, unit: usize) -> Option<DriveType> {
        self.drive_types.get(unit).copied().flatten()
    }

    /// Catalogue entry currently inserted, or an open door.
    pub fn inserted_medium(&self, unit: usize) -> Option<usize> {
        self.inserted.get(unit).copied().flatten()
    }

    /// Drive `unit`'s catalogue in order, the inserted entry included, for
    /// host presentation; reading it changes no controller state.
    pub fn catalogue(&self, unit: usize) -> impl Iterator<Item = &Drive> {
        let inserted = self.inserted_medium(unit);
        self.media_lists
            .get(unit)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(move |(index, entry)| {
                if Some(index) == inserted {
                    self.drives[unit].as_ref()
                } else {
                    entry.as_ref()
                }
            })
    }

    /// Whether drive `unit`'s front-panel activity light is on, for host
    /// presentation; reading it changes no controller state.
    ///
    /// IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
    /// Digital Output Register (3F2h) and Interface Lines: bits 1–0 select a
    /// drive and bits 7–4 switch on the motors of drives 3–0; the selected,
    /// motor-on drive is the one on the interface lines. ASSUMED: the drive's
    /// light follows that selection, so a BIOS motor-off delay keeps it lit
    /// after an access. A DMA transfer still pending for the drive also lights
    /// it, because seeks complete without modelled time.
    #[must_use]
    pub fn drive_active(&self, unit: usize) -> bool {
        let selected =
            unit < 4 && usize::from(self.dor & 3) == unit && self.dor & (DOR_MOTOR_0 << unit) != 0;
        let transferring = self
            .pending_read
            .is_some_and(|read| usize::from(read.drive) == unit)
            || self
                .pending_write
                .is_some_and(|write| usize::from(write.drive) == unit);
        selected || transferring
    }

    /// IBM PC AT High Capacity Diskette Drive, Output Signals, -Diskette Change:
    /// removing the medium prevents a step pulse from clearing the signal.
    /// IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
    /// Interface Lines and Digital Output Register: DOR selects the physical
    /// recipient, independently of the controller command's internal PCN counter.
    fn step(&mut self) {
        let unit = usize::from(self.dor & 3);
        if self.drives[unit].is_some() {
            self.disk_changed[unit] = false;
        }
    }

    /// The current IRQ6 line level, for tests and the machine's IRQ sampler.
    /// IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
    /// Digital Output Register (3F2h): bit 3 gates the controller INT onto IRQ6.
    #[must_use]
    pub fn irq_line(&self) -> bool {
        self.irq && self.dor & DOR_DMA_ENABLE != 0
    }

    /// The present cylinder recorded for `drive`, for tests.
    #[must_use]
    pub fn present_cylinder(&self, drive: usize) -> u8 {
        self.present_cylinder[drive & 0b11]
    }

    /// The full main status register byte for the current phase.
    fn msr(&self) -> u8 {
        match self.phase {
            Phase::Idle => MSR_RQM,
            Phase::Command => MSR_RQM | MSR_BUSY,
            // The host does not drive the data port during a DMA transfer.
            Phase::Execution => MSR_BUSY,
            Phase::Result => MSR_RQM | MSR_DIO | MSR_BUSY,
        }
    }

    /// NEC µPD765A/µPD765B datasheet, Instruction Set: the low five opcode
    /// bits select the command; MT/MFM/SK do not change its framing.
    /// Intel 8272A datasheet, Registers — CPU Interface and SENSE INTERRUPT
    /// STATUS: result INT acknowledgement differs from seek/reset sensing.
    fn command_info(opcode: u8) -> Option<CommandInfo> {
        let (length, result_interrupt) = match opcode & 0x1F {
            0x03 => (3, false), // SPECIFY
            0x04 => (2, false), // SENSE DRIVE STATUS
            0x07 => (2, false), // RECALIBRATE
            0x08 => (1, false), // SENSE INTERRUPT STATUS
            0x0F => (3, false), // SEEK
            0x0A => (2, true),  // READ ID
            0x06 => (9, true),  // READ DATA
            0x05 => (9, true),  // WRITE DATA
            _ => return None,
        };
        Some(CommandInfo {
            length,
            result_interrupt,
        })
    }

    /// Total command length, or `None` if the opcode is not modelled.
    fn command_length(opcode: u8) -> Option<usize> {
        Self::command_info(opcode).map(|info| info.length)
    }

    /// IBM PC AT Technical Reference, Fixed Disk and Diskette Drive Adapter,
    /// Digital Output Register: bit 3 gates reset-completion IRQ6 too.
    /// NEC µPD765A/µPD765B datasheet, Pin Functions — RESET: RESET places the
    /// FDC in the idle state and resets its output lines. INT and DRQ are
    /// therefore low while RESET is asserted. Releasing it permits the
    /// ready-change polling INT, acknowledged by SENSE INTERRUPT STATUS
    /// (Intel 8272A datasheet, SENSE INTERRUPT STATUS).
    fn write_dor(&mut self, value: u8) {
        let leaving_reset = self.dor & DOR_NOT_RESET == 0 && value & DOR_NOT_RESET != 0;
        self.dor = value;
        if value & DOR_NOT_RESET == 0 {
            self.irq = false;
        }
        if value & DOR_NOT_RESET == 0 || leaving_reset {
            self.phase = Phase::Idle;
            self.command.clear();
            self.result.clear();
            self.result_index = 0;
            self.pending_request = None;
            self.pending_read = None;
            self.pending_write = None;
        }
        if leaving_reset {
            // Four drive polls remain available unless seek/recalibrate completes
            // first; see the module's assumed reset-poll policy.
            self.reset_poll = 4;
            self.raise_irq();
        }
    }

    /// NEC µPD765A/µPD765B datasheet, Pin Functions — RESET: command bytes
    /// cannot leave the idle state while RESET is asserted.
    /// Intel 8272A datasheet, INVALID and SENSE INTERRUPT STATUS: INVALID's
    /// own result does not replace the status of a pending interrupt.
    fn write_data(&mut self, byte: u8) {
        if self.dor & DOR_NOT_RESET == 0 {
            return;
        }
        match self.phase {
            Phase::Idle => {
                let Some(len) = Self::command_length(byte) else {
                    // An unknown opcode leaves an invalid-command status for the
                    // host's next result read, matching the real controller.
                    self.result = vec![0x80];
                    self.result_index = 0;
                    self.phase = Phase::Result;
                    return;
                };
                self.command.clear();
                self.command.push(byte);
                self.expected = len;
                if self.command.len() == self.expected {
                    self.execute();
                } else {
                    self.phase = Phase::Command;
                }
            }
            Phase::Command => {
                self.command.push(byte);
                if self.command.len() == self.expected {
                    self.execute();
                }
            }
            // Bytes written during execution or result are ignored by the real
            // controller; nothing to collect.
            Phase::Execution | Phase::Result => {}
        }
    }

    /// Dispatch a fully-collected command to its handler.
    fn execute(&mut self) {
        let opcode = self.command[0] & 0x1F;
        match opcode {
            0x03 => self.cmd_specify(),
            0x04 => self.cmd_sense_drive_status(),
            0x07 => self.cmd_recalibrate(),
            0x08 => self.cmd_sense_interrupt(),
            0x0F => self.cmd_seek(),
            0x0A => self.cmd_read_id(),
            0x06 => self.cmd_read_data(),
            0x05 => self.cmd_write_data(),
            _ => self.finish_no_result(),
        }
    }

    /// SPECIFY: accept the step-rate / head timings and return to idle. No
    /// interrupt, no result phase.
    fn cmd_specify(&mut self) {
        self.finish_no_result();
    }

    /// NEC µPD765 datasheet, SENSE DRIVE STATUS / Status Register 3; Intel
    /// 8272A datasheet, SENSE DRIVE STATUS / Status Register 3: report the
    /// selected drive's signals without disturbing seek completion or INT.
    /// RDY and empty-bay signal assumptions follow the module's board policy.
    fn cmd_sense_drive_status(&mut self) {
        let drive_head = self.command[1] & 0b111;
        let unit = usize::from(drive_head & 0b11);
        let mut st3 = ST3_READY | drive_head;
        if self.drive_types[unit].is_some() && self.present_cylinder[unit] == 0 {
            st3 |= ST3_TRACK_ZERO;
        }
        if let Some(drive) = &self.drives[unit] {
            if drive.geometry.heads == 2 {
                st3 |= ST3_TWO_SIDE;
            }
            if drive.write_protected() {
                st3 |= ST3_WRITE_PROTECTED;
            }
        }
        self.result = vec![st3];
        self.result_index = 0;
        self.phase = Phase::Result;
    }

    /// RECALIBRATE: seek the selected drive to cylinder 0 and raise IRQ6. The
    /// result is read later via SENSE INTERRUPT STATUS. NEC µPD765A datasheet,
    /// RECALIBRATE: PCN is cleared before checking TRACK 0; failure after 77
    /// steps reports equipment check for an absent drive only, with the step
    /// delay collapsed here.
    fn cmd_recalibrate(&mut self) {
        let drive = self.command[1] & 0b11;
        self.st0 = ST0_SEEK_END | drive;
        self.step();
        self.present_cylinder[usize::from(drive)] = 0;
        if self.drive_types[usize::from(drive)].is_none() {
            self.st0 |= ST0_ABNORMAL | ST0_EQUIPMENT_CHECK;
        }
        self.reset_poll = 0;
        self.raise_irq();
        self.finish_no_result();
    }

    /// SEEK: set the selected drive's present cylinder and raise IRQ6.
    /// NEC µPD765A datasheet, SEEK: the PCN counter advances to NCN without
    /// position feedback. The AT adapter keeps RDY active even for an empty bay.
    fn cmd_seek(&mut self) {
        let drive_head = self.command[1];
        let drive = drive_head & 0b11;
        let head = (drive_head >> 2) & 1;
        let cylinder = self.command[2];
        self.st0 = ST0_SEEK_END | (head << 2) | drive;
        if self.present_cylinder[usize::from(drive)] != cylinder {
            self.step();
        }
        self.present_cylinder[usize::from(drive)] = cylinder;
        self.reset_poll = 0;
        self.raise_irq();
        self.finish_no_result();
    }

    /// SENSE INTERRUPT STATUS: report ST0 and the present cylinder, clearing the
    /// pending interrupt. After reset, up to four calls report drive-poll status
    /// unless seek/recalibrate completes first (see the module's assumed policy).
    /// With nothing pending, return only invalid-command ST0.
    /// See NEC µPD765 datasheet, Functional Description of Commands, SENSE
    /// INTERRUPT STATUS / INVALID; Intel 8272A datasheet, Command Descriptions,
    /// SENSE INTERRUPT STATUS / INVALID.
    fn cmd_sense_interrupt(&mut self) {
        if self.reset_poll == 0 && !self.irq {
            self.st0 = 0x80;
            self.result = vec![self.st0];
            self.result_index = 0;
            self.phase = Phase::Result;
            return;
        }
        let (st0, pcn) = if self.reset_poll > 0 {
            let drive = 4 - self.reset_poll;
            self.reset_poll -= 1;
            (ST0_POLL | drive, 0)
        } else {
            let drive = usize::from(self.st0 & 0b11);
            (self.st0, self.present_cylinder[drive])
        };
        self.irq = false;
        self.result = vec![st0, pcn];
        self.result_index = 0;
        self.phase = Phase::Result;
    }

    /// READ ID: report synthetic C/H/R/N on mounted media. Without index pulses,
    /// stay in execution until reset or insertion (NEC µPD765A, READ ID and ST1 MA).
    /// A double-stepped medium's ID cylinder is the track under the head; between
    /// tracks there is no ID field (see the module's drive and medium types).
    fn cmd_read_id(&mut self) {
        let drive_head = self.command[1];
        let drive = drive_head & 0b11;
        let head = (drive_head >> 2) & 1;
        let unit = usize::from(drive);
        let pcn = self.present_cylinder[unit];
        let Some(media) = self.drives[unit].as_ref() else {
            self.phase = Phase::Execution;
            return;
        };
        let geometry = media.geometry;
        let track = self.track_under_head(unit, geometry);
        let Some(cylinder) = track.filter(|_| self.data_rate == self.medium_rate(unit, geometry))
        else {
            self.command_fault(drive, head, pcn, 1, 2, 0, ST1_MISSING_ADDRESS_MARK);
            return;
        };
        self.st0 = (head << 2) | drive;
        self.raise_irq();
        self.result = vec![self.st0, 0, 0, cylinder, head, 1, 2];
        self.result_index = 0;
        self.phase = Phase::Result;
    }

    /// READ DATA: queue sectors through EOT, continuing onto head 1 when MT is
    /// set. Only sector IDs within the physical geometry can reach the media;
    /// an invalid starting CHS returns ND (NEC µPD765 datasheet, READ DATA;
    /// Intel 8272A datasheet, READ DATA and Status Register 1, ND).
    /// Result and IRQ6 follow DMA completion.
    fn cmd_read_data(&mut self) {
        let drive_head = self.command[1];
        let drive = drive_head & 0b11;
        let head = (drive_head >> 2) & 1;
        let cylinder = self.command[2];
        let param_head = self.command[3];
        let sector = self.command[4];
        let sector_size_code = self.command[5];

        // With RDY active and no index pulses, the ID search has no endpoint
        // (NEC µPD765A datasheet, READ DATA and Status Register 1, MA).
        let Some(media) = self.drives[usize::from(drive)].as_ref() else {
            self.phase = Phase::Execution;
            return;
        };
        let geometry = media.geometry;
        if self.data_rate != self.medium_rate(usize::from(drive), geometry) {
            self.command_fault(
                drive,
                head,
                cylinder,
                sector,
                sector_size_code,
                0,
                ST1_MISSING_ADDRESS_MARK,
            );
            return;
        }
        if let Some((st1, st2)) = self.id_search_fault(usize::from(drive), cylinder, geometry) {
            self.command_fault(drive, head, cylinder, sector, sector_size_code, 0, st1);
            self.result[2] = st2;
            return;
        }

        // An address outside the drive's geometry has no matching data field.
        if !geometry.contains(cylinder, param_head, sector) {
            self.command_fault(
                drive,
                head,
                cylinder,
                sector,
                sector_size_code,
                0,
                ST1_NO_DATA,
            );
            return;
        }

        let mut data = Vec::new();
        for (h, s) in self
            .data_sectors(param_head, sector)
            .take_while(|&(h, s)| geometry.contains(cylinder, h, s))
        {
            let offset = geometry.chs_to_offset(cylinder, h, s);
            data.extend_from_slice(&media.media.read_sector((offset / SECTOR_SIZE) as u64));
        }

        let requested = data.len();
        let missing_sector = self
            .data_sectors(param_head, sector)
            .nth(requested / SECTOR_SIZE)
            .is_some();
        self.pending_read = Some(PendingRead {
            drive,
            head,
            cylinder,
            sector,
            sector_size_code,
            requested,
            missing_sector,
        });
        self.pending_request = Some(DmaRequest {
            channel: FLOPPY_DMA_CHANNEL,
            op: DmaOp::ToMemory(data),
        });
        self.phase = Phase::Execution;
    }

    /// Sector IDs in transfer order. MT spans at most the two sides of one
    /// cylinder, not the next cylinder (NEC µPD765 datasheet, READ DATA,
    /// Multi-Track; Intel 8272A datasheet, READ DATA, Transfer Capacity).
    fn data_sectors(&self, head: u8, sector: u8) -> impl Iterator<Item = (u8, u8)> {
        let eot = self.command[6];
        let next_head = self.command[0] & 0x80 != 0 && head == 0 && sector <= eot;
        (sector..=eot.max(sector))
            .map(move |r| (head, r))
            .chain((1..=eot).filter(move |_| next_head).map(|r| (1, r)))
    }

    /// Result ID after a transfer terminated by the host: at EOT, R becomes 1;
    /// MT complements H, and C advances unless MT ended on side 0. N is unchanged.
    /// NEC µPD765 datasheet, READ DATA, Table 2; Intel 8272A datasheet, READ DATA,
    /// Table 8, ID Information When Processor Terminates Command.
    fn result_chs(
        &self,
        mut cylinder: u8,
        mut head: u8,
        mut sector: u8,
        sectors_done: usize,
    ) -> (u8, u8, u8) {
        let mt = self.command[0] & 0x80 != 0;
        for _ in 0..sectors_done {
            if sector == self.command[6] {
                sector = 1;
                if !mt || head == 1 {
                    cylinder = cylinder.wrapping_add(1);
                }
                if mt {
                    head ^= 1;
                }
            } else {
                sector = sector.wrapping_add(1);
            }
        }
        (cylinder, head, sector)
    }

    /// The CCR rate the medium of `geometry` answers at in drive `unit`.
    fn medium_rate(&self, unit: usize, geometry: FloppyGeometry) -> u8 {
        self.drive_types[unit].map_or_else(
            || geometry.recording_rate(),
            |drive_type| drive_type.data_rate(geometry),
        )
    }

    /// The medium track under the head of drive `unit`: the PCN, or for a
    /// double-stepped medium half an even PCN; `None` between two tracks.
    fn track_under_head(&self, unit: usize, geometry: FloppyGeometry) -> Option<u8> {
        let pcn = self.present_cylinder[unit];
        if self.drive_types[unit].is_some_and(|drive_type| drive_type.double_steps(geometry)) {
            pcn.is_multiple_of(2).then_some(pcn / 2)
        } else {
            Some(pcn)
        }
    }

    /// The ID search of READ/WRITE DATA on a double-stepped medium: between
    /// tracks no ID field is found (ST1 MA); an ID cylinder other than the
    /// command's C ends with ST1 ND and ST2 WC (NEC µPD765 datasheet, READ
    /// DATA and Status Registers 1 and 2). Returns the fault's ST1 and ST2.
    /// Single-stepped media keep addressing the command's C directly.
    fn id_search_fault(
        &self,
        unit: usize,
        cylinder: u8,
        geometry: FloppyGeometry,
    ) -> Option<(u8, u8)> {
        if !self.drive_types[unit].is_some_and(|drive_type| drive_type.double_steps(geometry)) {
            return None;
        }
        match self.track_under_head(unit, geometry) {
            None => Some((ST1_MISSING_ADDRESS_MARK, 0)),
            Some(track) if track != cylinder => Some((ST1_NO_DATA, ST2_WRONG_CYLINDER)),
            Some(_) => None,
        }
    }

    /// Terminate READ ID or READ/WRITE DATA abnormally: build the seven-byte result
    /// with the given ST0/ST1 fault bits and raise IRQ6.
    #[allow(clippy::too_many_arguments)]
    fn command_fault(
        &mut self,
        drive: u8,
        head: u8,
        cylinder: u8,
        sector: u8,
        sector_size_code: u8,
        st0_extra: u8,
        st1: u8,
    ) {
        self.st0 = ST0_ABNORMAL | st0_extra | (head << 2) | drive;
        self.raise_irq();
        self.result = vec![self.st0, st1, 0, cylinder, head, sector, sector_size_code];
        self.result_index = 0;
        self.phase = Phase::Result;
    }

    /// TC terminates READ/WRITE normally even when EOT exceeds the geometry.
    /// Without TC, a stalled DMA is an overrun; consuming the valid prefix leaves
    /// either a missing sector (ND) or the command's EOT limit (EN).
    /// NEC µPD765 datasheet, Functional Description of Commands, READ DATA /
    /// WRITE DATA and Status Registers 0/1; Intel 8272A datasheet, Command
    /// Descriptions, READ DATA / WRITE DATA and Status Register 1, OR / ND / EN.
    fn transfer_status(
        transferred: usize,
        requested: usize,
        terminal_count: bool,
        missing_sector: bool,
    ) -> u8 {
        if terminal_count {
            0
        } else if transferred < requested {
            ST1_OVERRUN
        } else if missing_sector {
            ST1_NO_DATA
        } else {
            ST1_END_OF_CYLINDER
        }
    }

    /// Complete a READ transfer once the machine has moved `transferred` bytes:
    /// advance the sector past what was read, build the seven-byte result, and
    /// raise IRQ6.
    fn complete_read(&mut self, transferred: usize, terminal_count: bool) {
        let Some(read) = self.pending_read.take() else {
            return;
        };
        let sectors_done = transferred / SECTOR_SIZE;
        let (cylinder, head, next_sector) =
            self.result_chs(read.cylinder, read.head, read.sector, sectors_done);
        let active_head = self
            .data_sectors(read.head, read.sector)
            .take(sectors_done)
            .last()
            .map_or(read.head, |(h, _)| h);
        let st1 = Self::transfer_status(
            transferred,
            read.requested,
            terminal_count,
            read.missing_sector,
        );
        self.st0 = (active_head << 2) | read.drive | if st1 == 0 { 0 } else { ST0_ABNORMAL };
        self.raise_irq();
        self.result = vec![
            self.st0,
            st1,
            0,
            cylinder,
            head,
            next_sector,
            read.sector_size_code,
        ];
        self.result_index = 0;
        self.phase = Phase::Result;
    }

    /// WRITE DATA: request sectors through EOT and, with MT, head 1, to be
    /// pulled out of memory over DMA and submitted to the mounted media. The
    /// result phase and IRQ6 follow once the machine reports the transfer
    /// complete. Mirrors [`Upd765::cmd_read_data`] with the DMA direction
    /// reversed.
    fn cmd_write_data(&mut self) {
        let drive_head = self.command[1];
        let drive = drive_head & 0b11;
        let head = (drive_head >> 2) & 1;
        let cylinder = self.command[2];
        let param_head = self.command[3];
        let sector = self.command[4];
        let sector_size_code = self.command[5];

        // With RDY active and no index pulses, the ID search has no endpoint
        // (NEC µPD765A datasheet, READ DATA and Status Register 1, MA).
        let Some(media) = self.drives[usize::from(drive)].as_ref() else {
            self.phase = Phase::Execution;
            return;
        };
        let geometry = media.geometry;

        // Write protection rejects the command before the ID search, so NW
        // takes precedence over rate/CHS faults (Intel 8272A, WRITE DATA).
        if media.write_protected() {
            self.command_fault(
                drive,
                head,
                cylinder,
                sector,
                sector_size_code,
                0,
                ST1_NOT_WRITABLE,
            );
            return;
        }

        if self.data_rate != self.medium_rate(usize::from(drive), geometry) {
            self.command_fault(
                drive,
                head,
                cylinder,
                sector,
                sector_size_code,
                0,
                ST1_MISSING_ADDRESS_MARK,
            );
            return;
        }
        if let Some((st1, st2)) = self.id_search_fault(usize::from(drive), cylinder, geometry) {
            self.command_fault(drive, head, cylinder, sector, sector_size_code, 0, st1);
            self.result[2] = st2;
            return;
        }

        // An address outside the drive's geometry has no matching data field.
        if !geometry.contains(cylinder, param_head, sector) {
            self.command_fault(
                drive,
                head,
                cylinder,
                sector,
                sector_size_code,
                0,
                ST1_NO_DATA,
            );
            return;
        }

        let sector_count = self
            .data_sectors(param_head, sector)
            .take_while(|&(h, s)| geometry.contains(cylinder, h, s))
            .count();

        let requested = sector_count * SECTOR_SIZE;
        let missing_sector = self
            .data_sectors(param_head, sector)
            .nth(sector_count)
            .is_some();
        self.pending_write = Some(PendingWrite {
            drive,
            head,
            param_head,
            cylinder,
            sector,
            sector_size_code,
            requested,
            missing_sector,
        });
        self.pending_request = Some(DmaRequest {
            channel: FLOPPY_DMA_CHANNEL,
            op: DmaOp::FromMemory {
                len: sector_count * SECTOR_SIZE,
            },
        });
        self.phase = Phase::Execution;
    }

    /// Commit a WRITE transfer once the machine has pulled `bytes` out of memory:
    /// submit replacements to the addressed drive's media, advance the sector,
    /// build the seven-byte result, and raise IRQ6. Mirrors [`Upd765::complete_read`].
    ///
    /// A floppy write is atomic per sector. If terminal count cut the DMA short
    /// mid-sector, the last partially-supplied sector is still committed whole
    /// with its tail zero-filled — so the number of sectors written rounds *up*
    /// (the read path, which reports how many whole sectors reached memory, rounds
    /// down). Media owns storage allocation; the controller supplies complete
    /// replacement sectors without depending on the media's representation.
    fn complete_write(&mut self, bytes: Vec<u8>, terminal_count: bool) {
        let Some(write) = self.pending_write.take() else {
            return;
        };
        let sectors_written = bytes.len().div_ceil(SECTOR_SIZE);
        let sectors = self.data_sectors(write.param_head, write.sector);
        if let Some(media) = self.drives[usize::from(write.drive)].as_mut() {
            let batch: Vec<_> = sectors
                .zip(bytes.chunks(SECTOR_SIZE))
                .map(|((head, sector), chunk)| {
                    let offset = media.geometry.chs_to_offset(write.cylinder, head, sector);
                    let mut bytes = [0; SECTOR_SIZE];
                    bytes[..chunk.len()].copy_from_slice(chunk);
                    SectorWrite {
                        lba: (offset / SECTOR_SIZE) as u64,
                        bytes,
                    }
                })
                .collect();
            if !batch.is_empty() {
                media.media.write_batch(&batch);
            }
        }
        let (cylinder, head, next_sector) =
            self.result_chs(write.cylinder, write.head, write.sector, sectors_written);
        let active_head = self
            .data_sectors(write.head, write.sector)
            .take(sectors_written)
            .last()
            .map_or(write.head, |(h, _)| h);
        let st1 = Self::transfer_status(
            bytes.len(),
            write.requested,
            terminal_count,
            write.missing_sector,
        );
        self.st0 = (active_head << 2) | write.drive | if st1 == 0 { 0 } else { ST0_ABNORMAL };
        self.raise_irq();
        self.result = vec![
            self.st0,
            st1,
            0,
            cylinder,
            head,
            next_sector,
            write.sector_size_code,
        ];
        self.result_index = 0;
        self.phase = Phase::Result;
    }

    /// Latch controller completion independently of the adapter's bus gate.
    fn raise_irq(&mut self) {
        self.irq = true;
    }

    /// Finish a command that has no result phase, returning to idle.
    fn finish_no_result(&mut self) {
        self.phase = Phase::Idle;
        self.command.clear();
    }

    /// Intel 8272A datasheet, Registers — CPU Interface:
    /// the first result byte acknowledges command completion INT, but all
    /// result bytes must be read before another command can be accepted.
    /// Only completion commands acknowledge INT here: reset/seek/recalibrate
    /// require SENSE INTERRUPT STATUS (same datasheet, SENSE INTERRUPT STATUS).
    fn read_result(&mut self) -> u8 {
        if self.result_index >= self.result.len() {
            return 0;
        }
        if self.result_index == 0
            && self
                .command
                .first()
                .and_then(|&opcode| Self::command_info(opcode))
                .is_some_and(|info| info.result_interrupt)
        {
            self.irq = false;
        }
        let byte = self.result[self.result_index];
        self.result_index += 1;
        if self.result_index >= self.result.len() {
            self.phase = Phase::Idle;
            self.command.clear();
            self.result.clear();
            self.result_index = 0;
        }
        byte
    }

    /// Record a serviced port access on the tracing seam.
    fn trace_port(&mut self, port: u16, size: u8, dir: AccessDir, value: u32) {
        self.trace.push(TraceEvent::Port {
            at: self.now,
            port,
            size,
            dir,
            value,
        });
    }
}

impl Default for Upd765 {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for Upd765 {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
        if !(PORT_BASE..=PORT_END).contains(&port) {
            return Err(DeviceError::UnmappedRegister {
                kind: "port",
                offset: u64::from(port),
            });
        }
        let value = match port {
            DOR => u32::from(self.dor),
            MSR => u32::from(self.msr()),
            DATA => u32::from(self.read_result()),
            DIR_CCR => {
                if self.disk_changed[usize::from(self.dor & 3)] {
                    0x80
                } else {
                    0
                }
            }
            // Other ports in the decoded window are not driven by this controller.
            _ => 0xFF,
        };
        self.trace_port(port, size, AccessDir::Read, value);
        Ok(value)
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
        if !(PORT_BASE..=PORT_END).contains(&port) {
            return Err(DeviceError::UnmappedRegister {
                kind: "port",
                offset: u64::from(port),
            });
        }
        let byte = value as u8;
        match port {
            DOR => self.write_dor(byte),
            DATA => self.write_data(byte),
            DIR_CCR => self.data_rate = byte & 3,
            // MSR is read-only; other ports are not driven.
            _ => {}
        }
        self.trace_port(port, size, AccessDir::Write, value);
        Ok(())
    }

    fn mmio_read(&mut self, addr: u64, _size: u8) -> Result<u64, DeviceError> {
        Err(DeviceError::UnmappedRegister {
            kind: "mmio",
            offset: addr,
        })
    }

    fn mmio_write(&mut self, addr: u64, _size: u8, _value: u64) -> Result<(), DeviceError> {
        Err(DeviceError::UnmappedRegister {
            kind: "mmio",
            offset: addr,
        })
    }

    fn tick(&mut self, now: VClock) {
        debug_assert!(now >= self.now, "device clock moved backwards");
        if now < self.now {
            return;
        }
        // Empty-bay searches cannot advance without external index pulses;
        // mounted transfers finish through DMA. This clock only stamps traces.
        self.now = now;
    }

    /// NEC µPD765 datasheet, SPECIFY, SEEK and RECALIBRATE: step/head timings
    /// are accepted but not enforced here. Reset IRQ and seeks finish on port
    /// access; READ/WRITE DATA finishes on DMA completion, with no timed phase.
    fn next_event(&self, _: VClock) -> NextEvent {
        NextEvent::Dormant
    }

    fn irq_pending(&self) -> bool {
        self.irq_line()
    }

    /// NEC µPD765A datasheet, READ DATA / WRITE DATA; IBM Personal Computer AT
    /// Technical Reference, Diskette Adapter, Digital Output Register: bit 3
    /// gates the queued DMA request without consuming it.
    fn dma_request_pending(&self) -> bool {
        self.dor & DOR_DMA_ENABLE != 0 && self.pending_request.is_some()
    }

    fn take_dma_request(&mut self) -> Option<DmaRequest> {
        if self.dor & DOR_DMA_ENABLE != 0 {
            self.pending_request.take()
        } else {
            None
        }
    }

    fn dma_complete(&mut self, outcome: DmaOutcome) {
        match outcome {
            DmaOutcome::ToMemory {
                transferred,
                terminal_count,
            } => self.complete_read(transferred, terminal_count),
            DmaOutcome::FromMemory {
                bytes,
                terminal_count,
            } => self.complete_write(bytes, terminal_count),
        }
    }

    fn reset(&mut self) {
        let trace_policy = self.trace.policy();
        // A controller reset clears the registers but does not eject any disk.
        let drives = std::mem::take(&mut self.drives);
        let drive_types = self.drive_types;
        let media_lists = std::mem::take(&mut self.media_lists);
        let inserted = self.inserted;
        let disk_changed = self.disk_changed;
        *self = Self {
            drives,
            drive_types,
            media_lists,
            inserted,
            disk_changed,
            ..Self::new()
        };
        self.trace.set_policy(trace_policy);
    }

    fn save(&self) -> Vec<u8> {
        // Sector contents and host journal sessions remain external snapshot inputs.
        let phase_tag = match self.phase {
            Phase::Idle => 0,
            Phase::Command => 1,
            Phase::Execution => 2,
            Phase::Result => 3,
        };
        let mut blob = vec![
            SNAPSHOT_VERSION,
            self.dor,
            phase_tag,
            self.st0,
            u8::from(self.irq),
            self.reset_poll,
        ];
        blob.extend_from_slice(&self.present_cylinder);
        blob.push(self.expected as u8);
        blob.push(self.command.len() as u8);
        blob.extend_from_slice(&self.command);
        blob.push(self.result.len() as u8);
        blob.extend_from_slice(&self.result);
        blob.push(self.result_index as u8);
        blob.extend_from_slice(&self.now.0.to_le_bytes());
        let pending = self
            .pending_read
            .map(|r| {
                (
                    1,
                    r.drive,
                    r.head,
                    self.command[3],
                    r.cylinder,
                    r.sector,
                    r.sector_size_code,
                    r.requested,
                    r.missing_sector,
                )
            })
            .or_else(|| {
                self.pending_write.map(|w| {
                    (
                        2,
                        w.drive,
                        w.head,
                        w.param_head,
                        w.cylinder,
                        w.sector,
                        w.sector_size_code,
                        w.requested,
                        w.missing_sector,
                    )
                })
            });
        if let Some((tag, drive, head, param_head, cylinder, sector, size, requested, missing)) =
            pending
        {
            blob.extend([tag, drive, head, param_head, cylinder, sector, size]);
            blob.extend_from_slice(&(requested as u32).to_le_bytes());
            blob.push(u8::from(missing));
        } else {
            blob.push(0);
        }
        blob.push(u8::from(self.pending_request.is_some()));
        if let Some(DmaRequest {
            op: DmaOp::ToMemory(bytes),
            ..
        }) = &self.pending_request
        {
            blob.extend_from_slice(bytes);
        }
        blob.push(self.data_rate);
        for unit in 0..4 {
            blob.extend_from_slice(
                &self.inserted[unit]
                    .map_or(u32::MAX, |v| v as u32)
                    .to_le_bytes(),
            );
            blob.push(u8::from(self.disk_changed[unit]));
        }
        blob
    }

    fn restore(&mut self, snapshot: &[u8]) -> Result<(), DeviceError> {
        let bad = |reason| DeviceError::InvalidSnapshot {
            device: NAME,
            reason,
        };
        let mut it = snapshot.iter().copied();
        let mut next = || it.next().ok_or(bad("snapshot truncated"));
        let version = next()?;
        if !matches!(version, 3 | 4 | SNAPSHOT_VERSION) {
            return Err(bad("unsupported snapshot version"));
        }
        let mut inserted = std::array::from_fn(|unit| {
            if self.media_lists[unit].is_empty() {
                None
            } else {
                Some(0)
            }
        });
        let mut disk_changed = [false; 4];
        if version >= 5 {
            let tail = snapshot
                .get(
                    snapshot
                        .len()
                        .checked_sub(20)
                        .ok_or(bad("snapshot truncated"))?..,
                )
                .ok_or(bad("snapshot truncated"))?;
            for unit in 0..4 {
                let chunk = &tail[unit * 5..unit * 5 + 5];
                let index = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                inserted[unit] = if index == u32::MAX {
                    None
                } else {
                    Some(index as usize)
                };
                disk_changed[unit] = match chunk[4] {
                    0 => false,
                    1 => true,
                    _ => return Err(bad("invalid disk-change line")),
                };
                if inserted[unit].is_some_and(|i| i >= self.media_lists[unit].len()) {
                    return Err(bad("medium absent from catalogue"));
                }
            }
        }
        let dor = next()?;
        let phase = match next()? {
            0 => Phase::Idle,
            1 => Phase::Command,
            2 => Phase::Execution,
            3 => Phase::Result,
            _ => return Err(bad("invalid phase tag")),
        };
        let st0 = next()?;
        let irq = next()?;
        let reset_poll = next()?;
        let mut present_cylinder = [0; 4];
        for pcn in &mut present_cylinder {
            *pcn = next()?;
        }
        let expected = usize::from(next()?);
        let command_len = usize::from(next()?);
        if command_len > 9 || !matches!(expected, 0 | 1 | 2 | 3 | 9) || irq > 1 || reset_poll > 4 {
            return Err(bad("invalid command state"));
        }
        let mut command = Vec::new();
        for _ in 0..command_len {
            command.push(next()?);
        }
        let result_len = usize::from(next()?);
        if result_len > 7 {
            return Err(bad("invalid result length"));
        }
        let mut result = Vec::new();
        for _ in 0..result_len {
            result.push(next()?);
        }
        let result_index = usize::from(next()?);
        let mut clock = [0; 8];
        for byte in &mut clock {
            *byte = next()?;
        }
        let now = VClock(u64::from_le_bytes(clock));
        let pending_tag = next()?;
        let mut pending_read = None;
        let mut pending_write = None;
        let mut requested = 0;
        if pending_tag != 0 {
            if pending_tag > 2
                || phase != Phase::Execution
                || command_len != 9
                || expected != 9
                || command[0] & 0x1F != if pending_tag == 1 { 6 } else { 5 }
            {
                return Err(bad("inconsistent transfer phase"));
            }
            let drive = next()?;
            let head = next()?;
            let param_head = next()?;
            let cylinder = next()?;
            let sector = next()?;
            let sector_size_code = next()?;
            let mut count = [0; 4];
            for byte in &mut count {
                *byte = next()?;
            }
            requested = u32::from_le_bytes(count) as usize;
            let missing_sector = next()?;
            if drive != command[1] & 3
                || head != (command[1] >> 2) & 1
                || param_head != command[3]
                || cylinder != command[2]
                || sector != command[4]
                || sector_size_code != command[5]
                || requested == 0
                || requested > 510 * SECTOR_SIZE
                || !requested.is_multiple_of(SECTOR_SIZE)
                || missing_sector > 1
            {
                return Err(bad("invalid transfer context"));
            }
            let unit = usize::from(drive);
            let media = inserted[unit].and_then(|index| {
                if self.inserted[unit] == Some(index) {
                    self.drives[unit].as_ref()
                } else {
                    self.media_lists[unit][index].as_ref()
                }
            });
            let Some(media) = media else {
                return Err(bad("transfer requires mounted media"));
            };
            if !media.geometry.contains(cylinder, param_head, sector)
                || (pending_tag == 2 && media.write_protected())
            {
                return Err(bad("transfer incompatible with media"));
            }
            let eot = command[6];
            let next_head = command[0] & 0x80 != 0 && param_head == 0 && sector <= eot;
            let mut sectors = (sector..=eot.max(sector))
                .map(|r| (param_head, r))
                .chain((1..=eot).filter(|_| next_head).map(|r| (1, r)));
            for _ in 0..requested / SECTOR_SIZE {
                if !sectors
                    .next()
                    .is_some_and(|(h, r)| media.geometry.contains(cylinder, h, r))
                {
                    return Err(bad("transfer length exceeds media"));
                }
            }
            let following = sectors.next();
            if following.is_some_and(|(h, r)| media.geometry.contains(cylinder, h, r))
                || (missing_sector != 0) != following.is_some()
            {
                return Err(bad("inconsistent transfer boundary"));
            }
            if pending_tag == 1 {
                pending_read = Some(PendingRead {
                    drive,
                    head,
                    cylinder,
                    sector,
                    sector_size_code,
                    requested,
                    missing_sector: missing_sector != 0,
                });
            } else {
                pending_write = Some(PendingWrite {
                    drive,
                    head,
                    param_head,
                    cylinder,
                    sector,
                    sector_size_code,
                    requested,
                    missing_sector: missing_sector != 0,
                });
            }
        }
        let request_tag = next()?;
        if request_tag > 1 || (request_tag != 0 && pending_tag == 0) {
            return Err(bad("invalid DMA request tag"));
        }
        let pending_request = if request_tag != 0 {
            let op = if pending_tag == 1 {
                let mut bytes = Vec::new();
                for _ in 0..requested {
                    bytes.push(next()?);
                }
                DmaOp::ToMemory(bytes)
            } else {
                DmaOp::FromMemory { len: requested }
            };
            Some(DmaRequest {
                channel: FLOPPY_DMA_CHANNEL,
                op,
            })
        } else {
            None
        };
        let data_rate = if version >= 4 {
            let rate = next()?;
            if rate > 3 {
                return Err(bad("invalid data rate"));
            }
            rate
        } else {
            RESET_DATA_RATE
        };
        if version >= 5 {
            for _ in 0..20 {
                next()?;
            }
        }
        if it.next().is_some() {
            return Err(bad("trailing snapshot bytes"));
        }
        let valid_command = command.first().and_then(|&op| Self::command_length(op));
        let command_consistent =
            command.is_empty() || (valid_command == Some(expected) && command_len <= expected);
        let result_consistent = match command.first().map(|op| op & 0x1F) {
            Some(4) => result_len == 1,
            Some(8) => matches!(result_len, 1 | 2),
            Some(5 | 6 | 0x0A) => result_len == 7,
            None => result_len == 1,
            _ => false,
        };
        let valid_phase = command_consistent
            && match phase {
                Phase::Idle => {
                    command.is_empty() && result.is_empty() && result_index == 0 && pending_tag == 0
                }
                Phase::Command => {
                    !command.is_empty()
                        && command_len < expected
                        && result.is_empty()
                        && result_index == 0
                        && pending_tag == 0
                }
                Phase::Execution => {
                    let waiting_for_index = pending_tag == 0
                        && matches!(command.first().map(|op| op & 0x1F), Some(5 | 6 | 0x0A))
                        && command_len == expected
                        && inserted[usize::from(command[1] & 3)].is_none();
                    (pending_tag != 0 || waiting_for_index)
                        && result.is_empty()
                        && result_index == 0
                }
                Phase::Result => {
                    result_consistent
                        && result_index < result_len
                        && pending_tag == 0
                        && (command.is_empty() || command_len == expected)
                }
            };
        if !valid_phase {
            return Err(bad("inconsistent command phase"));
        }
        for (unit, &selected) in inserted.iter().enumerate() {
            let journaled = self.drives[unit]
                .iter()
                .chain(self.media_lists[unit].iter().flatten())
                .any(|drive| drive.journaled);
            if selected != self.inserted[unit] && journaled {
                return Err(bad(
                    "cannot change journaled medium without the machine persistence API",
                ));
            }
        }
        for (unit, &selected) in inserted.iter().enumerate() {
            if let Some(index) = self.inserted[unit].take() {
                self.media_lists[unit][index] = self.drives[unit].take();
            }
            self.drives[unit] = selected.and_then(|index| self.media_lists[unit][index].take());
        }
        self.inserted = inserted;
        self.disk_changed = disk_changed;
        self.dor = dor;
        self.data_rate = data_rate;
        self.phase = phase;
        self.st0 = st0;
        self.irq = irq != 0;
        self.reset_poll = reset_poll;
        self.present_cylinder = present_cylinder;
        self.expected = expected;
        self.command = command;
        self.result = result;
        self.result_index = result_index;
        self.now = now;
        self.pending_request = pending_request;
        self.pending_read = pending_read;
        self.pending_write = pending_write;
        Ok(())
    }

    fn set_trace_policy(&mut self, policy: device::TracePolicy) {
        self.trace.set_policy(policy);
    }

    fn trace_dropped(&self) -> u64 {
        self.trace.dropped()
    }

    fn take_trace(&mut self) -> Vec<TraceEvent> {
        self.trace.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testkit::MockBus;

    #[test]
    fn the_catalogue_lists_every_entry_with_its_write_policy_wherever_it_is() {
        let mut fdc = Upd765::new();
        let entry = |protected, journaled| {
            let media = Box::new(InMemoryMedia::new(vec![0; 512], 2880, protected));
            if journaled {
                Drive::with_journaled_media(FloppyGeometry::HD1440, media)
            } else {
                Drive::with_media(FloppyGeometry::HD1440, media)
            }
        };
        fdc.set_media_list(
            0,
            DriveType::HD1440,
            vec![entry(false, false), entry(true, false), entry(false, true)],
            false,
        )
        .unwrap();
        let policy = |fdc: &Upd765| {
            fdc.catalogue(0)
                .map(|drive| (drive.write_protected(), drive.journaled()))
                .collect::<Vec<_>>()
        };
        let expected = [(false, false), (true, false), (false, true)];
        assert_eq!(policy(&fdc), expected);
        fdc.insert(0, 2).unwrap();
        assert_eq!(fdc.inserted_medium(0), Some(2));
        assert_eq!(policy(&fdc), expected);
        fdc.eject(0).unwrap();
        assert_eq!(policy(&fdc), expected);
        assert_eq!(fdc.catalogue(1).count(), 0);
        assert_eq!(fdc.catalogue(9).count(), 0);
    }

    fn swap_fdc(bus: &mut MockBus) -> &mut Upd765 {
        bus.device_mut()
            .as_any_mut()
            .unwrap()
            .downcast_mut()
            .unwrap()
    }

    fn swap_bus() -> MockBus {
        let mut fdc = ready_fdc(vec![0x11; SECTOR_SIZE]);
        fdc.set_media_list(
            0,
            DriveType::HD1440,
            vec![
                Drive::new(FloppyGeometry::HD1440, vec![0x11; SECTOR_SIZE]),
                Drive::new_read_only(FloppyGeometry::HD1440, vec![0x22; SECTOR_SIZE]),
            ],
            false,
        )
        .unwrap();
        fdc.set_media_list(
            1,
            DriveType::HD1440,
            vec![Drive::new(FloppyGeometry::HD1440, vec![0x33; SECTOR_SIZE])],
            false,
        )
        .unwrap();
        MockBus::new(Box::new(fdc))
    }

    #[test]
    fn drive_light_follows_the_selected_motor_and_a_pending_transfer() {
        let disk = || Some(Drive::new(FloppyGeometry::HD1440, vec![0x5A; SECTOR_SIZE]));
        let mut bus = MockBus::new(Box::new(ready_drives([disk(), disk(), None, None])));
        let lights = |bus: &mut MockBus| -> [bool; 5] {
            let fdc = swap_fdc(bus);
            std::array::from_fn(|unit| fdc.drive_active(unit))
        };
        // The ready controller selects drive 0 with its motor on (DOR 1Ch).
        assert_eq!(lights(&mut bus), [true, false, false, false, false]);
        bus.port_write(DOR, 1, 0x0C).unwrap();
        assert_eq!(lights(&mut bus), [false; 5]);
        bus.port_write(DOR, 1, 0x2D).unwrap();
        assert_eq!(lights(&mut bus), [false, true, false, false, false]);
        // A running motor without the drive's selection does not light it.
        bus.port_write(DOR, 1, 0x1D).unwrap();
        assert_eq!(lights(&mut bus), [false; 5]);
        bus.port_write(DOR, 1, 0x0D).unwrap();
        bus_command(&mut bus, &[0x46, 1, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
        assert_eq!(lights(&mut bus), [false, true, false, false, false]);
        finish_test_dma(&mut bus, SECTOR_SIZE, true);
        assert_eq!(lights(&mut bus), [false; 5]);
        assert_eq!(bus_results(&mut bus, 7)[0] & 3, 1);
        // WRITE DATA waits for its bytes from memory: the pending write lights it.
        bus_command(&mut bus, &[0x45, 1, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
        assert_eq!(lights(&mut bus), [false, true, false, false, false]);
        finish_test_dma(&mut bus, SECTOR_SIZE, true);
        assert_eq!(lights(&mut bus), [false; 5]);
        assert_eq!(bus_results(&mut bus, 7)[0] & 3, 1);
    }

    #[test]
    fn append_medium_preserves_the_door_and_rejects_wrong_geometry_or_absent_drives() {
        let mut bus = swap_bus();
        let fdc = swap_fdc(&mut bus);
        let changed = fdc.disk_changed;
        let index = fdc
            .append_medium(
                0,
                Drive::new_read_only(FloppyGeometry::HD1440, vec![0x44; SECTOR_SIZE]),
            )
            .unwrap();
        assert_eq!(index, 2);
        assert_eq!(fdc.inserted_medium(0), Some(0));
        assert_eq!(fdc.disk_changed, changed);
        assert_eq!(
            fdc.append_medium(2, Drive::new(FloppyGeometry::HD1440, vec![])),
            Err(MediaChangeError::UnknownDrive)
        );
        assert_eq!(
            fdc.append_medium(
                0,
                Drive::new(
                    FloppyGeometry {
                        cylinders: 80,
                        heads: 2,
                        sectors_per_track: 15
                    },
                    vec![]
                )
            ),
            Err(MediaChangeError::GeometryMismatch)
        );
        assert_eq!(fdc.media_lists[0].len(), 3);
        fdc.insert(0, index).unwrap();
        assert_eq!(fdc.inserted_medium(0), Some(index));
        assert!(fdc.disk_changed[0]);
        assert!(fdc.drives[0].as_ref().unwrap().write_protected());
        assert_eq!(media_read(&mut bus, 0, 0, 1), vec![0x44; SECTOR_SIZE]);
    }

    #[test]
    fn review_truncate_unselected_append_preserves_controller_state() {
        let mut bus = swap_bus();
        let fdc = swap_fdc(&mut bus);
        let before = fdc.save();
        fdc.append_medium(
            0,
            Drive::new_read_only(FloppyGeometry::HD1440, vec![0x44; SECTOR_SIZE]),
        )
        .unwrap();
        fdc.truncate_catalogue(0, 2).unwrap();
        assert_eq!(fdc.media_lists[0].len(), 2);
        assert_eq!(fdc.save(), before);
        assert_eq!(
            fdc.truncate_catalogue(0, 0),
            Err(MediaChangeError::UnknownMedium)
        );
        assert_eq!(fdc.save(), before);
    }

    #[test]
    fn append_medium_waits_for_a_completed_dma_boundary() {
        let mut bus = swap_bus();
        bus_command(&mut bus, &[6, 0, 0, 0, 1, 2, 1, 0x1b, 0xff]);
        let fdc = swap_fdc(&mut bus);
        assert_eq!(
            fdc.append_medium(
                0,
                Drive::new_read_only(FloppyGeometry::HD1440, vec![0x44; SECTOR_SIZE])
            ),
            Err(MediaChangeError::DmaPending)
        );
        assert_eq!(fdc.media_lists[0].len(), 2);
        assert_eq!(fdc.inserted_medium(0), Some(0));
    }

    #[test]
    fn floppy_media_power_on_distinguishes_absent_and_configured_empty_drives() {
        let controllers = [
            Upd765::new(),
            Upd765::with_drives([None, None, None, None]),
            Upd765::with_image(vec![0; SECTOR_SIZE]),
        ];
        let mut readings = Vec::new();
        for controller in controllers {
            let mut bus = MockBus::new(Box::new(controller));
            for stage in 0..3 {
                match stage {
                    1 => swap_fdc(&mut bus)
                        .set_media_list(
                            1,
                            DriveType::HD1440,
                            vec![Drive::new(FloppyGeometry::HD1440, vec![])],
                            true,
                        )
                        .unwrap(),
                    2 => swap_fdc(&mut bus)
                        .set_media_list(3, DriveType::HD1440, vec![], true)
                        .unwrap(),
                    _ => {}
                }
                for unit in 0..4 {
                    bus.port_write(DOR, 1, 0x0c | unit).unwrap();
                    readings.push(bus.port_read(DIR_CCR, 1).unwrap());
                }
            }
        }
        assert_eq!(
            readings,
            [0, 0, 0, 0, 0, 0x80, 0, 0, 0, 0x80, 0, 0x80].repeat(3),
            "only configured empty-start drives assert change at power-on"
        );
    }

    #[test]
    fn floppy_media_empty_power_on_asserts_change_until_inserted_and_stepped() {
        let mut bus = swap_bus();
        swap_fdc(&mut bus)
            .set_media_list(
                0,
                DriveType::HD1440,
                vec![Drive::new(FloppyGeometry::HD1440, vec![])],
                true,
            )
            .unwrap();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
        bus_command(&mut bus, &[7, 0]);
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
        swap_fdc(&mut bus).insert(0, 0).unwrap();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
        bus_command(&mut bus, &[7, 0]);
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0);
    }

    #[test]
    fn floppy_media_step_clears_dor_selected_drive_even_if_command_select_differs() {
        let mut bus = swap_bus();
        swap_fdc(&mut bus).insert(0, 0).unwrap();
        swap_fdc(&mut bus).insert(1, 0).unwrap();
        bus.port_write(DOR, 1, 0x2d).unwrap();
        bus_command(&mut bus, &[0x0f, 0, 1]);
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0);
        bus.port_write(DOR, 1, 0x1c).unwrap();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
        bus_command(&mut bus, &[7, 1]);
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0);
    }

    #[test]
    fn floppy_media_snapshot_cannot_change_a_journaled_slots_selection() {
        for journal_at in [0, 1] {
            let mut bus = swap_bus();
            let entries = (0..2)
                .map(|index| {
                    let media = Box::new(InMemoryMedia::new(vec![index as u8; 512], 2880, false));
                    if index == journal_at {
                        Drive::with_journaled_media(FloppyGeometry::HD1440, media)
                    } else {
                        Drive::with_media(FloppyGeometry::HD1440, media)
                    }
                })
                .collect();
            swap_fdc(&mut bus)
                .set_media_list(0, DriveType::HD1440, entries, false)
                .unwrap();
            let saved = bus.device_mut().save();
            bus.device_mut().restore(&saved).unwrap();
            swap_fdc(&mut bus).insert(0, 1).unwrap();
            let before = bus.device_mut().save();
            let error = bus
                .device_mut()
                .restore(&saved)
                .expect_err("journal selection requires the machine persistence API");
            assert!(error.to_string().contains("journaled medium"));
            assert_eq!(
                bus.device_mut().save(),
                before,
                "rejected restore is atomic"
            );
            for version in [3, 4] {
                let mut legacy = saved[..saved.len() - 20].to_vec();
                legacy[0] = version;
                if version == 3 {
                    legacy.pop();
                }
                assert!(
                    bus.device_mut().restore(&legacy).is_err(),
                    "legacy medium-zero defaults also require persistence"
                );
                assert_eq!(bus.device_mut().save(), before);
            }
            swap_fdc(&mut bus).eject(0).unwrap();
            assert!(bus.device_mut().restore(&saved).is_err());
        }
    }

    #[test]
    fn floppy_media_change_line_tracks_door_step_and_dor_per_drive() {
        let mut bus = swap_bus();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0);
        swap_fdc(&mut bus).eject(0).unwrap();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
        bus_command(&mut bus, &[0x0f, 0, 1]);
        bus_command(&mut bus, &[7, 0]);
        assert_eq!(
            bus.port_read(DIR_CCR, 1).unwrap(),
            0x80,
            "no disk cannot clear change"
        );
        bus.port_write(DOR, 1, 0x1d).unwrap();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0);
        swap_fdc(&mut bus).insert(0, 1).unwrap();
        bus.port_write(DOR, 1, 0x1c).unwrap();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
        bus_command(&mut bus, &[0x0f, 0, 0]);
        assert_eq!(
            bus.port_read(DIR_CCR, 1).unwrap(),
            0x80,
            "no movement means no step pulse"
        );
        bus_command(&mut bus, &[0x0f, 0, 1]);
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0);
        swap_fdc(&mut bus).eject(1).unwrap();
        swap_fdc(&mut bus).insert(1, 0).unwrap();
        bus.port_write(DOR, 1, 0x2d).unwrap();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
        bus_command(&mut bus, &[7, 0]);
        assert_eq!(
            bus.port_read(DIR_CCR, 1).unwrap(),
            0,
            "DOR selects the step recipient even when the command names another drive"
        );
    }

    #[test]
    fn floppy_media_change_preserves_overlays_and_empty_bay_behavior() {
        let mut bus = swap_bus();
        swap_fdc(&mut bus).drives[0]
            .as_mut()
            .unwrap()
            .media
            .write_batch(&[SectorWrite {
                lba: 0,
                bytes: [0x77; SECTOR_SIZE],
            }]);
        swap_fdc(&mut bus).eject(0).unwrap();
        bus_command(&mut bus, &[0x4a, 0]);
        assert!(bus.device_mut().take_dma_request().is_none());
        assert_eq!(bus.port_read(MSR, 1).unwrap(), 0x10);
        bus.device_mut().reset();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
        swap_fdc(&mut bus).insert(0, 1).unwrap();
        assert_eq!(
            swap_fdc(&mut bus).drives[0]
                .as_ref()
                .unwrap()
                .media
                .read_sector(0),
            [0x22; SECTOR_SIZE]
        );
        assert!(swap_fdc(&mut bus).drives[0]
            .as_ref()
            .unwrap()
            .write_protected());
        swap_fdc(&mut bus).insert(0, 0).unwrap();
        assert_eq!(
            swap_fdc(&mut bus).drives[0]
                .as_ref()
                .unwrap()
                .media
                .read_sector(0),
            [0x77; SECTOR_SIZE]
        );
        assert_eq!(swap_fdc(&mut bus).inserted_medium(0), Some(0));
    }

    #[test]
    fn floppy_media_insertion_supplies_index_to_a_parked_empty_bay_search() {
        let mut bus = swap_bus();
        swap_fdc(&mut bus).eject(0).unwrap();
        bus_command(&mut bus, &[0x4a, 0]);
        assert_eq!(bus.port_read(MSR, 1).unwrap(), 0x10);
        swap_fdc(&mut bus).insert(0, 1).unwrap();
        assert_eq!(bus_results(&mut bus, 7), [0, 0, 0, 0, 0, 1, 2]);
        let saved = bus.device().save();
        bus.device_mut().restore(&saved).unwrap();
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
    }

    #[test]
    fn floppy_media_change_snapshot_restores_selection_door_and_legacy_defaults() {
        let mut bus = swap_bus();
        swap_fdc(&mut bus).insert(0, 1).unwrap();
        swap_fdc(&mut bus).eject(1).unwrap();
        let saved = bus.device().save();
        swap_fdc(&mut bus).insert(0, 0).unwrap();
        swap_fdc(&mut bus).insert(1, 0).unwrap();
        bus.device_mut().restore(&saved).unwrap();
        assert_eq!(swap_fdc(&mut bus).inserted_medium(0), Some(1));
        assert_eq!(swap_fdc(&mut bus).inserted_medium(1), None);
        assert_eq!(bus.device().save(), saved);
        for version in [3, 4] {
            let mut old = saved[..saved.len() - 20 - usize::from(version == 3)].to_vec();
            old[0] = version;
            bus.device_mut().restore(&old).unwrap();
            assert_eq!(swap_fdc(&mut bus).inserted_medium(0), Some(0));
            assert_eq!(swap_fdc(&mut bus).inserted_medium(1), Some(0));
            assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0);
        }
        let before = bus.device().save();
        let mut invalid = saved;
        let end = invalid.len();
        invalid[end - 20] = 99;
        assert!(bus.device_mut().restore(&invalid).is_err());
        assert_eq!(bus.device().save(), before);
    }

    #[test]
    fn data_rate_selection_masks_upper_bits_and_preserves_dir_reads() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        for value in 0..=255 {
            bus.port_write(DIR_CCR, 1, value).unwrap();
            assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0);
            let saved = bus.device().save();
            assert_eq!(saved[0], 5);
            assert_eq!(saved.get(saved.len() - 21), Some(&((value as u8) & 3)));
        }
    }

    #[test]
    fn data_rate_device_reset_defaults_to_500_but_dor_reset_preserves_selection() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        assert_eq!(bus.device().save()[0], 5);
        assert_eq!(
            bus.device().save().get(bus.device().save().len() - 21),
            Some(&0)
        );
        for rate in 0..4 {
            bus.port_write(DIR_CCR, 1, rate).unwrap();
            bus.port_write(DOR, 1, 0).unwrap();
            assert_eq!(
                bus.device().save().get(bus.device().save().len() - 21),
                Some(&(rate as u8))
            );
            bus.port_write(DOR, 1, 0x1C).unwrap();
            assert_eq!(
                bus.device().save().get(bus.device().save().len() - 21),
                Some(&(rate as u8))
            );
            bus.device_mut().reset();
            assert_eq!(
                bus.device().save().get(bus.device().save().len() - 21),
                Some(&0)
            );
        }
    }

    fn check_media_data_rates(geometry: FloppyGeometry, matching: u32, opcode: u8) {
        // Exercise both heads and every unit so fault status cannot lose HD/US.
        let mut failures = Vec::new();
        for rate in 0..4 {
            for unit_head in 0..8 {
                let drives = std::array::from_fn(|_| {
                    Some(Drive::new(geometry, vec![0xA5; geometry.image_bytes()]))
                });
                let mut bus = MockBus::new(Box::new(ready_drives(drives)));
                bus.port_write(DIR_CCR, 1, rate).unwrap();
                let head = unit_head >> 2;
                bus_command(&mut bus, &[0x07, unit_head]);
                bus_command(&mut bus, &[0x04, unit_head]);
                assert_eq!(bus_results(&mut bus, 1), [0x38 | unit_head]);
                assert!(bus.device().irq_pending());
                bus_command(&mut bus, &[8]);
                assert_eq!(bus_results(&mut bus, 2), [0x20 | (unit_head & 3), 0]);
                bus_command(&mut bus, &[0x0F, unit_head, 7]);
                bus_command(&mut bus, &[8]);
                assert_eq!(bus_results(&mut bus, 2), [0x20 | unit_head, 7]);
                let bytes = if opcode == 0x4A {
                    vec![opcode, unit_head]
                } else {
                    vec![opcode, unit_head, 7, head, 3, 2, 3, 0x1B, 0xFF]
                };
                // Completion must not precede the last command parameter.
                bus_command(&mut bus, &bytes[..bytes.len() - 1]);
                assert!(!bus.device().irq_pending());
                bus_command(&mut bus, &bytes[bytes.len() - 1..]);
                let request = bus.device_mut().take_dma_request();
                if let Some(request) = &request {
                    assert_eq!(request.op, DmaOp::ToMemory(vec![0xA5; SECTOR_SIZE]));
                    assert!(!bus.device().irq_pending());
                    bus.device_mut().dma_complete(DmaOutcome::ToMemory {
                        transferred: SECTOR_SIZE,
                        terminal_count: true,
                    });
                }
                assert!(bus.device().irq_pending());
                assert_eq!(
                    bus.port_read(MSR, 1).unwrap(),
                    u32::from(MSR_RQM | MSR_DIO | MSR_BUSY)
                );
                let result = bus_results(&mut bus, 7);
                assert!(!bus.device().irq_pending());
                let expected = if rate != matching {
                    vec![
                        0x40 | unit_head,
                        1,
                        0,
                        7,
                        head,
                        if opcode == 0x4A { 1 } else { 3 },
                        2,
                    ]
                } else if opcode == 0x4A {
                    vec![unit_head, 0, 0, 7, head, 1, 2]
                } else {
                    vec![unit_head, 0, 0, 8, head, 1, 2]
                };
                if result != expected || request.is_some() != (rate == matching && opcode != 0x4A) {
                    failures.push((rate, unit_head, result, expected));
                }
            }
        }
        assert!(failures.is_empty(), "rate/HD/US mismatches: {failures:?}");
    }

    #[test]
    fn read_id_checks_recording_rate_for_every_standard_geometry() {
        for (geometry, rate) in [
            (FloppyGeometry::HD1440, 0),
            (FloppyGeometry::HD1200, 0),
            (FloppyGeometry::DD720, 2),
            (FloppyGeometry::DD360, 2),
        ] {
            check_media_data_rates(geometry, rate, 0x4A);
        }
    }

    #[test]
    fn read_data_checks_recording_rate_for_every_standard_geometry() {
        for (geometry, rate) in [
            (FloppyGeometry::HD1440, 0),
            (FloppyGeometry::HD1200, 0),
            (FloppyGeometry::DD720, 2),
            (FloppyGeometry::DD360, 2),
        ] {
            check_media_data_rates(geometry, rate, 0x46);
        }
    }

    #[test]
    fn write_data_rate_mismatch_leaves_media_unchanged() {
        for (geometry, matching) in [
            (FloppyGeometry::HD1440, 0),
            (FloppyGeometry::HD1200, 0),
            (FloppyGeometry::DD720, 2),
            (FloppyGeometry::DD360, 2),
        ] {
            for rate in (0..4).filter(|&rate| rate != matching) {
                let image = patterned(geometry, 0x93);
                let mut bus = MockBus::new(Box::new(ready_drives([
                    Some(Drive::new(geometry, image.clone())),
                    None,
                    None,
                    None,
                ])));
                bus.port_write(DIR_CCR, 1, rate).unwrap();
                bus_command(&mut bus, &[0x45, 4, 7, 1, 3, 2, 3, 0x1B, 0xFF]);
                assert!(bus.device_mut().take_dma_request().is_none());
                assert!(bus.device().irq_pending());
                assert_eq!(bus_results(&mut bus, 7), [0x44, 1, 0, 7, 1, 3, 2]);
                // A late completion must not commit data from a rejected command.
                bus.device_mut().dma_complete(DmaOutcome::FromMemory {
                    bytes: vec![0x5A; SECTOR_SIZE],
                    terminal_count: true,
                });
                bus.port_write(DIR_CCR, 1, matching).unwrap();
                for c in 0..geometry.cylinders as u8 {
                    for h in 0..geometry.heads as u8 {
                        bus_command(
                            &mut bus,
                            &[
                                0x46,
                                h << 2,
                                c,
                                h,
                                1,
                                2,
                                geometry.sectors_per_track as u8,
                                0x1B,
                                0xFF,
                            ],
                        );
                        let DmaOp::ToMemory(data) = bus.device_mut().take_dma_request().unwrap().op
                        else {
                            panic!("READ DMA direction");
                        };
                        let offset = geometry.chs_to_offset(c, h, 1);
                        assert_eq!(data, image[offset..offset + data.len()]);
                        bus.device_mut().dma_complete(DmaOutcome::ToMemory {
                            transferred: data.len(),
                            terminal_count: true,
                        });
                        bus_results(&mut bus, 7);
                    }
                }
            }
        }
    }

    #[test]
    fn write_protect_precedes_rate_mismatch_on_dd_media() {
        for geometry in [FloppyGeometry::DD360, FloppyGeometry::DD720] {
            for rate in 0..4 {
                let original = vec![0xA5; SECTOR_SIZE];
                let mut bus = MockBus::new(Box::new(ready_drives([
                    Some(Drive::new_read_only(geometry, original.clone())),
                    None,
                    None,
                    None,
                ])));
                bus.port_write(DIR_CCR, 1, rate).unwrap();
                bus_command(&mut bus, &[0x45, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
                assert!(bus.device_mut().take_dma_request().is_none());
                assert!(bus.device().irq_pending());
                assert_eq!(bus_results(&mut bus, 7), [0x40, 0x02, 0, 0, 0, 1, 2]);
                assert!(!bus.device().irq_pending());
                bus.device_mut().dma_complete(DmaOutcome::FromMemory {
                    bytes: vec![0x5A; SECTOR_SIZE],
                    terminal_count: true,
                });
                bus.port_write(DIR_CCR, 1, 2).unwrap();
                assert_eq!(media_read(&mut bus, 0, 0, 1), original);
            }
        }
    }

    #[test]
    fn rate_mismatch_precedes_invalid_chs_on_writable_media() {
        for (geometry, matching) in [
            (FloppyGeometry::HD1440, 0),
            (FloppyGeometry::HD1200, 0),
            (FloppyGeometry::DD720, 2),
            (FloppyGeometry::DD360, 2),
        ] {
            for rate in (0..4).filter(|&rate| rate != matching) {
                for opcode in [0x46, 0x45] {
                    for (c, h, r) in [
                        (geometry.cylinders as u8, 1, 1),
                        (0, 2, 1),
                        (0, 1, 0),
                        (0, 1, geometry.sectors_per_track as u8 + 1),
                    ] {
                        let mut bus = MockBus::new(Box::new(ready_drives([
                            Some(Drive::new(geometry, Vec::new())),
                            None,
                            None,
                            None,
                        ])));
                        // Unreadable ID fields prevent the CHS search; MA wins ND.
                        for (selected, st1) in [(rate, 0x01), (matching, 0x04)] {
                            bus.port_write(DIR_CCR, 1, selected).unwrap();
                            bus_command(&mut bus, &[opcode, 4, c, h, r, 2, r, 0x1B, 0xFF]);
                            assert!(bus.device_mut().take_dma_request().is_none());
                            assert!(bus.device().irq_pending());
                            assert_eq!(bus_results(&mut bus, 7), [0x44, st1, 0, c, 1, r, 2]);
                            assert!(!bus.device().irq_pending());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn read_id_media_probe_rejects_300_then_accepts_500_on_1200k() {
        let mut bus = MockBus::new(Box::new(ready_drives([
            Some(Drive::new(FloppyGeometry::HD1200, Vec::new())),
            None,
            None,
            None,
        ])));
        for (rate, expected) in [(1, [0x40, 1, 0, 0, 0, 1, 2]), (0, [0, 0, 0, 0, 0, 1, 2])] {
            bus.port_write(DIR_CCR, 1, rate).unwrap();
            bus_command(&mut bus, &[0x4A, 0]);
            assert!(bus.device().irq_pending());
            assert_eq!(bus_results(&mut bus, 7), expected);
        }
    }

    #[test]
    fn sense_drive_status_has_two_command_bytes_and_one_result() {
        for modifiers in (0..=7).map(|bits| bits << 5) {
            let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
            bus_command(&mut bus, &[modifiers | 0x04]);
            assert_eq!(
                bus.port_read(MSR, 1).unwrap(),
                u32::from(MSR_RQM | MSR_BUSY),
                "the drive/head parameter must precede the result"
            );
            bus_command(&mut bus, &[0xF8]);
            assert_eq!(
                bus.port_read(MSR, 1).unwrap(),
                u32::from(MSR_RQM | MSR_DIO | MSR_BUSY)
            );
            assert_eq!(bus_results(&mut bus, 1), vec![0x38]);
            assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
        }
    }

    #[test]
    fn sense_drive_status_echoes_unit_and_head_from_parameter() {
        let drives = std::array::from_fn(|_| Some(Drive::new(FloppyGeometry::HD1440, Vec::new())));
        let mut bus = MockBus::new(Box::new(ready_drives(drives)));
        for drive_head in 0..8 {
            bus_command(&mut bus, &[0x04, 0xF8 | drive_head]);
            assert_eq!(bus_results(&mut bus, 1), vec![0x38 | drive_head]);
        }
    }

    #[test]
    fn sense_drive_status_two_side_follows_selected_geometry() {
        let mut bus = MockBus::new(Box::new(ready_drives(std::array::from_fn(|unit| {
            let geometry = FloppyGeometry {
                heads: if unit % 2 == 0 { 1 } else { 2 },
                ..FloppyGeometry::HD1440
            };
            Some(Drive::new(geometry, Vec::new()))
        }))));
        for unit in 0..4 {
            bus_command(&mut bus, &[0x04, unit]);
            let two_side = if unit % 2 == 0 { 0 } else { 0x08 };
            assert_eq!(bus_results(&mut bus, 1), vec![0x30 | two_side | unit]);
        }
    }

    #[test]
    fn sense_drive_status_track_zero_follows_recalibrate_and_seek() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus_command(&mut bus, &[0x0F, 0, 10]);
        bus_command(&mut bus, &[0x07, 0]);
        bus_command(&mut bus, &[0x04, 0]);
        assert_eq!(bus_results(&mut bus, 1), vec![0x38]);
        bus_command(&mut bus, &[0x0F, 0, 10]);
        for _ in 0..2 {
            bus_command(&mut bus, &[0x04, 0]);
            assert_eq!(bus_results(&mut bus, 1), vec![0x28]);
        }
        bus_command(&mut bus, &[0x08]);
        assert_eq!(bus_results(&mut bus, 2), vec![ST0_SEEK_END, 10]);
    }

    #[test]
    fn sense_drive_status_write_protect_follows_selected_media() {
        let mut bus = MockBus::new(Box::new(ready_drives([
            Some(Drive::new_read_only(FloppyGeometry::HD1440, Vec::new())),
            Some(Drive::new(FloppyGeometry::HD1440, Vec::new())),
            None,
            None,
        ])));
        for (unit, status) in [(0, 0x78), (1, 0x39)] {
            bus_command(&mut bus, &[0x04, unit]);
            assert_eq!(bus_results(&mut bus, 1), vec![status]);
        }
    }

    /// Pins the assumed command-unit drive lines without DOR select/motor gating.
    #[test]
    fn sense_drive_status_ready_is_active_with_motors_and_gate_off() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus.port_write(DOR, 1, 4).unwrap();
        for unit in 0..4 {
            bus_command(&mut bus, &[0x04, unit]);
            let status = if unit == 0 { 0x38 } else { 0x20 | unit };
            assert_eq!(bus_results(&mut bus, 1), vec![status]);
        }
    }

    #[test]
    fn sense_drive_status_empty_bay_has_no_track_zero_two_side_or_write_protect() {
        let mut bus = MockBus::new(Box::new(ready_drives([None, None, None, None])));
        for unit in 0..4 {
            for command in [vec![0x07, unit], vec![0x0F, unit, 10]] {
                bus_command(&mut bus, &command);
                bus_command(&mut bus, &[0x04, 4 | unit]);
                assert_eq!(bus_results(&mut bus, 1), vec![0x24 | unit]);
            }
        }
    }

    #[test]
    fn sense_drive_status_fault_is_clear_for_mounted_and_empty_bays() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        for unit in 0..4 {
            bus_command(&mut bus, &[0x04, unit]);
            assert_eq!(bus_results(&mut bus, 1)[0] & 0x80, 0);
        }
    }

    #[test]
    fn sense_drive_status_does_not_raise_an_interrupt() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        for byte in [0x04, 0] {
            bus_command(&mut bus, &[byte]);
            assert!(!bus.device().irq_pending());
        }
        assert_eq!(bus_results(&mut bus, 1), vec![0x38]);
        assert!(!bus.device().irq_pending());
        bus_command(&mut bus, &[0x08]);
        assert_eq!(bus_results(&mut bus, 1), vec![0x80]);
    }

    #[test]
    fn sense_drive_status_preserves_pending_seek_and_other_drive_cylinders() {
        let drives = std::array::from_fn(|_| Some(Drive::new(FloppyGeometry::HD1440, Vec::new())));
        let mut bus = MockBus::new(Box::new(ready_drives(drives)));
        bus_command(&mut bus, &[0x0F, 6, 10]);
        for (drive_head, status) in [(0, 0x38), (6, 0x2E)] {
            for byte in [0x04, drive_head] {
                bus_command(&mut bus, &[byte]);
                assert!(bus.device().irq_pending());
            }
            assert_eq!(bus_results(&mut bus, 1), vec![status]);
            assert!(bus.device().irq_pending(), "ST3 does not acknowledge INT");
        }
        bus_command(&mut bus, &[0x08]);
        assert!(!bus.device().irq_pending());
        assert_eq!(bus_results(&mut bus, 2), vec![ST0_SEEK_END | 6, 10]);
    }

    #[test]
    fn sense_drive_status_preserves_all_reset_polls() {
        let mut bus = MockBus::new(Box::new(Upd765::new()));
        bus.port_write(DOR, 1, 12).unwrap();
        for unit in 0..4 {
            let pending = bus.device().irq_pending();
            bus_command(&mut bus, &[0x04, unit]);
            assert_eq!(bus_results(&mut bus, 1), vec![0x20 | unit]);
            assert_eq!(bus.device().irq_pending(), pending);
            bus_command(&mut bus, &[0x08]);
            assert_eq!(bus_results(&mut bus, 2), vec![ST0_POLL | unit, 0]);
        }
    }

    #[test]
    fn recalibrate_completion_supersedes_remaining_reset_polls() {
        let mut bus = MockBus::new(Box::new(Upd765::with_image(Vec::new())));
        bus.port_write(DOR, 1, 0x1C).unwrap();
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 2), [0xC0, 0]);
        bus_command(&mut bus, &[3, 0xDF, 2]);
        assert!(!bus.device().irq_pending());
        bus_command(&mut bus, &[7, 0]);
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[8]);
        assert!(!bus.device().irq_pending());
        assert_eq!(bus_results(&mut bus, 2), [0x20, 0]);
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 1), [0x80]);
        assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
    }

    #[test]
    fn seek_completion_supersedes_remaining_reset_polls() {
        let mut bus = MockBus::new(Box::new(Upd765::new()));
        bus.port_write(DOR, 1, 12).unwrap();
        for unit in 0..2 {
            bus_command(&mut bus, &[8]);
            assert_eq!(bus_results(&mut bus, 2), [0xC0 | unit, 0]);
        }
        bus_command(&mut bus, &[0x0F, 1, 5]);
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[8]);
        assert!(!bus.device().irq_pending());
        assert_eq!(bus_results(&mut bus, 2), [0x21, 5]);
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 1), [0x80]);
        assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
    }

    #[test]
    fn reset_polls_without_seek_completion_still_report_all_four_drives() {
        let mut bus = MockBus::new(Box::new(Upd765::new()));
        bus.port_write(DOR, 1, 12).unwrap();
        for unit in 0..4 {
            bus_command(&mut bus, &[8]);
            assert_eq!(bus_results(&mut bus, 2), [0xC0 | unit, 0]);
        }
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 1), [0x80]);
        assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
    }

    #[test]
    fn pending_seek_completion_is_replaced_by_the_latest_drive_seek() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus_command(&mut bus, &[0x0F, 0, 9]);
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[0x0F, 1, 5]);
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 2), [0x21, 5]);
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 1), [0x80]);
        bus_command(&mut bus, &[0x0F, 0, 9]);
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 2), [0x20, 9]);
    }

    #[test]
    fn sense_drive_status_probe_sequence_accepts_the_next_command() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus_command(&mut bus, &[0x07, 0]);
        bus_command(&mut bus, &[0x0F, 4, 10]);
        bus_command(&mut bus, &[0x04, 4]);
        assert_eq!(bus_results(&mut bus, 1), vec![0x2C]);
        assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
        bus_command(&mut bus, &[0x03, 0xDF, 2]);
        assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
        bus_command(&mut bus, &[0x08]);
        assert_eq!(bus_results(&mut bus, 2), vec![ST0_SEEK_END | 4, 10]);
        assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
    }

    #[test]
    fn dor_reset_assertion_clears_int_before_release() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus_command(&mut bus, &[0x0F, 0, 7]);
        assert!(bus.device().irq_pending());
        bus.port_write(DOR, 1, 8).unwrap();
        assert!(
            !bus.device().irq_pending(),
            "RESET clears INT with the gate open"
        );
        bus.port_write(DOR, 1, 12).unwrap();
        assert!(
            bus.device().irq_pending(),
            "reset release raises the polling INT"
        );
        bus_command(&mut bus, &[8]);
        assert!(
            !bus.device().irq_pending(),
            "SIS acknowledges the polling INT"
        );
        assert_eq!(bus_results(&mut bus, 2), vec![ST0_POLL, 0]);
    }

    #[test]
    fn dor_reset_assertion_clears_drq_before_release() {
        for opcode in [5, 6] {
            let mut bus = MockBus::new(Box::new(ready_fdc(vec![0x5A; SECTOR_SIZE])));
            bus_command(&mut bus, &[opcode, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
            assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_BUSY));
            bus.port_write(DOR, 1, 8).unwrap();
            assert!(
                bus.device_mut().take_dma_request().is_none(),
                "RESET clears DRQ"
            );
        }
    }

    fn check_first_result_read_clears_int(opcode: u8, gate_open: bool) {
        let mut bus = MockBus::new(Box::new(ready_fdc(vec![0x5A; SECTOR_SIZE])));
        let expected: Vec<u8> = if opcode == 0x4A {
            bus_command(&mut bus, &[opcode, 0]);
            vec![0, 0, 0, 0, 0, 1, 2]
        } else {
            bus_command(&mut bus, &[opcode, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
            finish_test_dma(&mut bus, SECTOR_SIZE, true);
            vec![0, 0, 0, 1, 0, 1, 2]
        };
        assert!(bus.device().irq_pending());
        if !gate_open {
            bus.port_write(DOR, 1, 4).unwrap();
            assert!(!bus.device().irq_pending());
        }
        assert_eq!(bus.port_read(DATA, 1).unwrap(), u32::from(expected[0]));
        bus.port_write(DOR, 1, 12).unwrap();
        assert!(
            !bus.device().irq_pending(),
            "first result read clears INT behind the DOR gate"
        );
        assert_eq!(
            bus.port_read(MSR, 1).unwrap(),
            u32::from(MSR_RQM | MSR_DIO | MSR_BUSY)
        );
        bus_command(&mut bus, &[0x0F, 0, 9]);
        for &byte in &expected[1..] {
            assert_eq!(bus.port_read(DATA, 1).unwrap(), u32::from(byte));
            assert!(!bus.device().irq_pending());
        }
        assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 1), vec![0x80]);
    }

    #[test]
    fn read_id_first_result_read_clears_int() {
        for gate_open in [true, false] {
            check_first_result_read_clears_int(0x4A, gate_open);
        }
    }

    #[test]
    fn read_data_first_result_read_clears_int() {
        for gate_open in [true, false] {
            check_first_result_read_clears_int(6, gate_open);
        }
    }

    #[test]
    fn write_data_first_result_read_clears_int() {
        for gate_open in [true, false] {
            check_first_result_read_clears_int(5, gate_open);
        }
    }

    #[test]
    fn reset_int_requires_sense_across_new_command() {
        let mut bus = MockBus::new(Box::new(Upd765::new()));
        bus.port_write(DOR, 1, 12).unwrap();
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[3, 0xDF, 2]);
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[0xFF]);
        assert!(bus.device().irq_pending());
        assert_eq!(bus_results(&mut bus, 1), vec![0x80]);
        assert!(
            bus.device().irq_pending(),
            "invalid result does not acknowledge reset INT"
        );
        for drive in 0..4 {
            bus_command(&mut bus, &[8]);
            assert!(!bus.device().irq_pending());
            assert_eq!(bus_results(&mut bus, 2), vec![ST0_POLL | drive, 0]);
        }
    }

    #[test]
    fn seek_and_recalibrate_int_requires_sense_across_new_command() {
        for drive in [0, 2] {
            for (command, pcn) in [(vec![0x0F, drive, 7], 7), (vec![7, drive], 0)] {
                let drives =
                    std::array::from_fn(|_| Some(Drive::new(FloppyGeometry::HD1440, Vec::new())));
                let mut bus = MockBus::new(Box::new(ready_drives(drives)));
                bus_command(&mut bus, &command);
                assert!(bus.device().irq_pending());
                for byte in [3, 0xDF, 2] {
                    bus_command(&mut bus, &[byte]);
                    assert!(
                        bus.device().irq_pending(),
                        "command writes do not acknowledge seek INT"
                    );
                }
                bus_command(&mut bus, &[0xFF]);
                assert!(bus.device().irq_pending());
                assert_eq!(bus_results(&mut bus, 1), vec![0x80]);
                assert!(
                    bus.device().irq_pending(),
                    "invalid result does not acknowledge seek INT"
                );
                bus_command(&mut bus, &[8]);
                assert!(
                    !bus.device().irq_pending(),
                    "only SIS acknowledges seek INT"
                );
                assert_eq!(bus_results(&mut bus, 2), vec![ST0_SEEK_END | drive, pcn]);
            }
        }
    }

    #[test]
    fn held_reset_ignores_data_writes() {
        let mut bus = MockBus::new(Box::new(ready_fdc(vec![0x5A; SECTOR_SIZE])));
        bus.port_write(DOR, 1, 8).unwrap();
        for command in [
            vec![0x0F, 0, 7],
            vec![7, 0],
            vec![0x4A, 0],
            vec![6, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF],
            vec![5, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF],
            vec![0xFF],
            vec![8],
        ] {
            for byte in command {
                bus_command(&mut bus, &[byte]);
                assert!(!bus.device().irq_pending(), "held RESET keeps INT low");
                assert!(bus.device_mut().take_dma_request().is_none());
                assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
                assert_eq!(bus.port_read(DATA, 1).unwrap(), 0);
            }
        }
        bus.port_write(DOR, 1, 12).unwrap();
        for drive in 0..4 {
            bus_command(&mut bus, &[8]);
            assert_eq!(bus_results(&mut bus, 2), vec![ST0_POLL | drive, 0]);
        }
        bus_command(&mut bus, &[0x4A, 0]);
        assert_eq!(bus_results(&mut bus, 7), vec![0, 0, 0, 0, 0, 1, 2]);
    }

    #[test]
    fn asserted_reset_reads_idle_from_each_active_phase() {
        for command in [
            vec![0x0F],
            vec![6, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF],
            vec![5, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF],
            vec![0x4A, 0],
        ] {
            let mut bus = MockBus::new(Box::new(ready_fdc(vec![0x5A; SECTOR_SIZE])));
            bus_command(&mut bus, &command);
            assert_ne!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
            bus.port_write(DOR, 1, 8).unwrap();
            assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_RQM));
            for _ in 0..8 {
                assert_eq!(bus.port_read(DATA, 1).unwrap(), 0);
            }
            assert!(!bus.device().irq_pending());
            assert!(bus.device_mut().take_dma_request().is_none());
            bus.port_write(DOR, 1, 12).unwrap();
            assert!(bus.device().irq_pending());
            bus_command(&mut bus, &[8]);
            assert_eq!(bus_results(&mut bus, 2), vec![ST0_POLL, 0]);
        }
    }

    #[test]
    fn dor_clearing_gate_removes_existing_irq() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus_command(&mut bus, &[0x0F, 0, 7]);
        assert!(bus.device().irq_pending());
        bus.port_write(DOR, 1, 4).unwrap();
        assert!(!bus.device().irq_pending());
        bus.port_write(DOR, 1, 12).unwrap();
        assert!(bus.device().irq_pending());
    }

    #[test]
    fn dor_gate_retains_dma_request() {
        let mut bus = MockBus::new(Box::new(ready_fdc(vec![0x5A; SECTOR_SIZE])));
        bus.port_write(DOR, 1, 4).unwrap();
        bus_command(&mut bus, &[6, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
        assert!(bus.device_mut().take_dma_request().is_none());
        bus.port_write(DOR, 1, 12).unwrap();
        let request = bus.device_mut().take_dma_request().unwrap();
        assert!(matches!(request.op, DmaOp::ToMemory(bytes) if bytes == vec![0x5A; SECTOR_SIZE]));
    }

    #[test]
    fn installed_empty_drive_recalibrates_without_equipment_check() {
        for unit in 0..4 {
            let mut fdc = ready_drives([None, None, None, None]);
            fdc.set_media_list(usize::from(unit), DriveType::HD1440, vec![], true)
                .unwrap();
            let mut bus = MockBus::new(Box::new(fdc));
            assert_empty_drive_recalibrates(&mut bus, unit);
        }
    }

    #[test]
    fn installed_empty_drive_recalibrates_after_ejection() {
        let mut bus = MockBus::new(Box::new(ready_drives([
            Some(Drive::new(FloppyGeometry::HD1200, vec![])),
            None,
            None,
            None,
        ])));
        swap_fdc(&mut bus).eject(0).unwrap();
        assert_empty_drive_recalibrates(&mut bus, 0);
    }

    fn assert_empty_drive_recalibrates(bus: &mut MockBus, unit: u8) {
        bus.port_write(DOR, 1, u32::from(0x0c | unit)).unwrap();
        bus_command(bus, &[0x0f, unit, 40]);
        bus_command(bus, &[8]);
        assert_eq!(bus_results(bus, 2), [ST0_SEEK_END | unit, 40]);
        bus_command(bus, &[7, unit]);
        assert!(bus.device().irq_pending());
        bus_command(bus, &[8]);
        assert_eq!(bus_results(bus, 2), [ST0_SEEK_END | unit, 0]);
        bus_command(bus, &[4, unit]);
        assert_eq!(bus_results(bus, 1), [ST3_READY | ST3_TRACK_ZERO | unit]);
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
    }

    /// Any catalogue entry can be in the drive at power-on: the drive reads
    /// it and the disk-change line is clear, exactly as with the first
    /// entry; an entry past the catalogue is refused.
    #[test]
    fn any_catalogue_entry_can_be_in_the_drive_at_power_on() {
        let entries = || {
            vec![
                Drive::new(FloppyGeometry::HD1440, vec![0x11; SECTOR_SIZE]),
                Drive::new(FloppyGeometry::HD1440, vec![0x22; SECTOR_SIZE]),
            ]
        };
        let mut views = Vec::new();
        for first in [0, 1] {
            let mut bus = MockBus::new(Box::new(ready_drives([None, None, None, None])));
            swap_fdc(&mut bus)
                .set_media_list_from(0, DriveType::HD1440, entries(), false, first)
                .unwrap();
            assert_eq!(swap_fdc(&mut bus).inserted_medium(0), Some(first));
            views.push(bus.port_read(DIR_CCR, 1).unwrap());
            bus_command(&mut bus, &[0x46, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
            let DmaOp::ToMemory(data) = bus.device_mut().take_dma_request().unwrap().op else {
                panic!("READ DMA direction");
            };
            assert_eq!(data[0], [0x11, 0x22][first], "entry {first}");
        }
        assert_eq!(views, [0, 0], "the change line starts clear");
        let mut fdc = ready_drives([None, None, None, None]);
        assert_eq!(
            fdc.set_media_list_from(0, DriveType::HD1440, entries(), false, 2),
            Err(MediaChangeError::UnknownMedium)
        );
        fdc.set_media_list_from(0, DriveType::HD1440, entries(), true, 2)
            .unwrap();
        assert_eq!(fdc.inserted_medium(0), None);
    }

    #[test]
    fn installed_empty_drive_track_zero_follows_seek_and_recalibrate() {
        let mut bus = MockBus::new(Box::new(ready_drives([None, None, None, None])));
        swap_fdc(&mut bus)
            .set_media_list(0, DriveType::HD1440, vec![], true)
            .unwrap();
        bus_command(&mut bus, &[4, 0]);
        assert_eq!(bus_results(&mut bus, 1), [ST3_READY | ST3_TRACK_ZERO]);
        bus_command(&mut bus, &[0x0f, 0, 40]);
        bus_command(&mut bus, &[4, 0]);
        assert_eq!(bus_results(&mut bus, 1), [ST3_READY]);
        bus_command(&mut bus, &[7, 0]);
        bus_command(&mut bus, &[4, 0]);
        assert_eq!(bus_results(&mut bus, 1), [ST3_READY | ST3_TRACK_ZERO]);
        bus.port_write(DOR, 1, 0x0d).unwrap();
        bus_command(&mut bus, &[7, 1]);
        bus_command(&mut bus, &[8]);
        assert_eq!(
            bus_results(&mut bus, 2),
            [ST0_ABNORMAL | ST0_SEEK_END | ST0_EQUIPMENT_CHECK | 1, 0]
        );
    }

    #[test]
    fn installed_empty_drive_survives_reset_and_restore_into_another_controller() {
        let mut source = MockBus::new(Box::new(ready_fdc(vec![])));
        swap_fdc(&mut source).eject(0).unwrap();
        let saved = source.device().save();
        let mut receiver = ready_fdc(vec![]);
        receiver.eject(0).unwrap();
        let mut bus = MockBus::new(Box::new(receiver));
        bus.device_mut().reset();
        bus.device_mut().restore(&saved).unwrap();
        assert_empty_drive_recalibrates(&mut bus, 0);
        swap_fdc(&mut bus)
            .set_media_list(1, DriveType::HD1440, vec![], true)
            .unwrap();
        bus.device_mut().reset();
        bus.port_write(DOR, 1, 0x0d).unwrap();
        bus_command(&mut bus, &[4, 1]);
        assert_eq!(bus_results(&mut bus, 1), [ST3_READY | ST3_TRACK_ZERO | 1]);
    }

    #[test]
    fn empty_bay_recalibrate_clears_pcn_and_reports_equipment_check() {
        let mut fdc = ready_fdc(Vec::new());
        fdc.present_cylinder[1] = 90;
        let mut bus = MockBus::new(Box::new(fdc));
        bus_command(&mut bus, &[7, 1]);
        bus_command(&mut bus, &[8]);
        assert_eq!(
            bus_results(&mut bus, 2),
            vec![ST0_ABNORMAL | ST0_SEEK_END | 0x10 | 1, 0]
        );
    }

    #[test]
    fn wait_snapshot_rejects_inconsistent_state_atomically() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus_command(&mut bus, &[0x4A, 1]);
        let saved = bus.device().save();
        for (index, value) in [(0, 2), (12, 0x03), (13, 0), (11, 1)] {
            let mut invalid = saved.clone();
            invalid[index] = value;
            assert!(bus.device_mut().restore(&invalid).is_err(), "index={index}");
            assert_eq!(bus.device().save(), saved);
        }
        let mut mounted = MockBus::new(Box::new(ready_drives([
            None,
            Some(Drive::new(FloppyGeometry::HD1440, Vec::new())),
            None,
            None,
        ])));
        let before = mounted.device().save();
        assert!(mounted.device_mut().restore(&saved).is_err());
        assert_eq!(mounted.device().save(), before);
    }

    #[test]
    fn dor_gate_exposes_and_hides_pending_irq() {
        let mut bus = MockBus::new(Box::new(Upd765::new()));
        bus.port_write(DOR, 1, 4).unwrap();
        assert!(!bus.device().irq_pending());
        bus.port_write(DOR, 1, 12).unwrap();
        assert!(bus.device().irq_pending());
        bus.port_write(DOR, 1, 4).unwrap();
        assert!(!bus.device().irq_pending());
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 2), vec![ST0_POLL, 0]);
        for _ in 0..3 {
            bus_command(&mut bus, &[8]);
            bus_results(&mut bus, 2);
        }
        bus_command(&mut bus, &[0x0F, 1, 7]);
        assert!(!bus.device().irq_pending());
        let saved = bus.device().save();
        bus.device_mut().restore(&saved).unwrap();
        bus.port_write(DOR, 1, 12).unwrap();
        assert!(bus.device().irq_pending());
        bus.port_write(DOR, 1, 4).unwrap();
        assert!(!bus.device().irq_pending());
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 2), vec![ST0_SEEK_END | 1, 7]);
        bus.port_write(DOR, 1, 12).unwrap();
        assert!(!bus.device().irq_pending());
    }

    #[test]
    fn empty_bay_seek_and_recalibrate_follow_at_signals() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus_command(&mut bus, &[0x0F, 5, 90]);
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 2), vec![ST0_SEEK_END | 5, 90]);
        bus_command(&mut bus, &[7, 1]);
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[8]);
        assert_eq!(
            bus_results(&mut bus, 2),
            vec![ST0_ABNORMAL | ST0_SEEK_END | 0x10 | 1, 0]
        );
    }

    #[test]
    fn empty_bay_data_commands_wait_without_dma_or_irq() {
        for opcode in [5, 6] {
            let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
            bus_command(&mut bus, &[opcode, 1, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
            bus.tick(1_000_000);
            assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_BUSY));
            assert!(!bus.device().irq_pending());
            assert!(bus.device_mut().take_dma_request().is_none());
            let saved = bus.device().save();
            bus.device_mut().restore(&saved).unwrap();
            assert_eq!(bus.device().save(), saved);
        }
    }

    #[test]
    fn empty_bay_read_id_waits_until_reset() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        bus_command(&mut bus, &[0x4A, 1]);
        for elapsed in [1, 1000, 1_000_000] {
            bus.tick(elapsed);
            assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_BUSY));
            assert!(!bus.device().irq_pending());
            assert_eq!(bus.device().next_event(VClock(elapsed)), NextEvent::Dormant);
            assert!(bus.device_mut().take_dma_request().is_none());
        }
        let saved = bus.device().save();
        bus.device_mut().restore(&saved).unwrap();
        assert_eq!(bus.device().save(), saved);
        bus_command(&mut bus, &[8]);
        assert_eq!(bus.port_read(MSR, 1).unwrap(), u32::from(MSR_BUSY));
        bus.port_write(DOR, 1, 0).unwrap();
        bus.port_write(DOR, 1, 12).unwrap();
        assert!(bus.device().irq_pending());
        bus_command(&mut bus, &[8]);
        assert_eq!(bus_results(&mut bus, 2), vec![ST0_POLL, 0]);
    }

    #[test]
    fn fidelity_reset_irq_obeys_dor_gate() {
        let mut bus = MockBus::new(Box::new(Upd765::new()));
        bus.port_write(DOR, 1, 4).unwrap();
        assert!(!bus.device().irq_pending());
        for drive in 0..4 {
            bus_command(&mut bus, &[0x08]);
            assert_eq!(bus_results(&mut bus, 2), vec![ST0_POLL | drive, 0]);
        }
    }

    use crate::dma8237::Dma8237;
    use device::DmaMemory;
    use std::collections::HashMap;

    /// NEC µPD765 datasheet, SPECIFY, SEEK, RECALIBRATE and READ/WRITE DATA:
    /// this functional model completes seeks/reset on access and transfers on
    /// DMA completion; SPECIFY timings do not create an autonomous phase.
    #[test]
    fn dormant_fdc_matches_per_clock_reset_seek_dma_and_traces() {
        let mut reference = MockBus::new(Box::new(Upd765::with_image(vec![0x5A; 512])));
        let mut scheduled = MockBus::new(Box::new(Upd765::with_image(vec![0x5A; 512])));
        for (index, bytes) in [
            vec![], // Release reset, then drain the four sense results.
            vec![0x03, 0xDF, 0x02],
            vec![0x0F, 0, 7],
            vec![0x07, 0],
            vec![0x46, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF],
            vec![0x45, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF],
        ]
        .into_iter()
        .enumerate()
        {
            let target = 1 + index as u64 * 13107;
            while reference.clock().0 < target {
                reference.tick(1);
            }
            assert_eq!(
                scheduled.device().next_event(VClock(target)),
                device::NextEvent::Dormant
            );
            scheduled.device_mut().sync(VClock(target));
            for bus in [&mut reference, &mut scheduled] {
                if bytes.is_empty() {
                    bus.port_write(DOR, 1, 0x1C).unwrap();
                    assert!(bus.device().irq_pending());
                    for _ in 0..4 {
                        bus_command(bus, &[0x08]);
                        bus_results(bus, 2);
                    }
                } else {
                    bus_command(bus, &bytes);
                    if matches!(index, 2 | 3) {
                        assert!(bus.device().irq_pending());
                        bus_command(bus, &[0x08]);
                        bus_results(bus, 2);
                    } else if index >= 4 {
                        assert!(!bus.device().irq_pending());
                        finish_test_dma(bus, 512, true);
                        assert!(bus.device().irq_pending());
                        bus_results(bus, 7);
                    }
                }
                assert_eq!(
                    bus.device().next_event(VClock(target)),
                    device::NextEvent::Dormant
                );
            }
            assert_eq!(
                reference.port_read(MSR, 1).unwrap(),
                scheduled.port_read(MSR, 1).unwrap()
            );
            assert_eq!(reference.device().save(), scheduled.device().save());
            assert_eq!(
                reference.device().irq_pending(),
                scheduled.device().irq_pending()
            );
            assert_eq!(
                reference.device_mut().take_trace(),
                scheduled.device_mut().take_trace()
            );
        }
    }

    /// A sparse mock guest memory implementing the SMC-safe DMA seam.
    #[derive(Default)]
    struct MockMem {
        cells: HashMap<u32, u8>,
    }

    impl DmaMemory for MockMem {
        fn dma_read(&self, addr: u32) -> u8 {
            self.cells.get(&addr).copied().unwrap_or(0)
        }
        fn dma_write(&mut self, addr: u32, value: u8) {
            self.cells.insert(addr, value);
        }
    }

    /// Issue a command opcode plus its parameters through the data port.
    fn command(fdc: &mut Upd765, bytes: &[u8]) {
        for &b in bytes {
            fdc.port_write(DATA, 1, u32::from(b)).unwrap();
        }
    }

    /// Read `n` result bytes back through the data port.
    fn results(fdc: &mut Upd765, n: usize) -> Vec<u8> {
        (0..n)
            .map(|_| fdc.port_read(DATA, 1).unwrap() as u8)
            .collect()
    }

    /// A controller with the given per-drive media, out of reset with the
    /// DMA/interrupt path enabled and the reset polls drained. Select the
    /// recording rate of drive 0; mixed-density tests must switch for other units.
    fn ready_drives(drives: [Option<Drive>; 4]) -> Upd765 {
        let rate = drives[0]
            .as_ref()
            .map_or(0, |drive| drive.geometry.recording_rate());
        let mut fdc = Upd765::with_drives(drives);
        fdc.port_write(DIR_CCR, 1, u32::from(rate)).unwrap();
        // Leave reset (bit 2), enable DMA (bit 3), motor 0 on (bit 4), drive 0.
        fdc.port_write(DOR, 1, 0x1C).unwrap();
        // The reset raises IRQ6 and queues the four polling senses; drain them so
        // tests start from a clean interrupt state.
        for _ in 0..4 {
            command(&mut fdc, &[0x08]);
            let _ = results(&mut fdc, 2);
        }
        fdc.irq = false;
        fdc
    }

    /// A ready controller with a single 1.44 MB disk in drive 0 (single-drive setup).
    fn ready_fdc(image: Vec<u8>) -> Upd765 {
        ready_drives([
            Some(Drive::new(FloppyGeometry::HD1440, image)),
            None,
            None,
            None,
        ])
    }

    fn bus_command(bus: &mut MockBus, bytes: &[u8]) {
        for &byte in bytes {
            bus.port_write(DATA, 1, u32::from(byte)).unwrap();
        }
    }

    fn bus_results(bus: &mut MockBus, count: usize) -> Vec<u8> {
        (0..count)
            .map(|_| bus.port_read(DATA, 1).unwrap() as u8)
            .collect()
    }

    #[test]
    fn dma_readiness_is_passive_and_preserves_the_dor_gate_and_completion() {
        for opcode in [0x06, 0x05] {
            let mut bus = MockBus::new(Box::new(ready_fdc(vec![0x5A; SECTOR_SIZE])));
            assert!(!bus.device().dma_request_pending());
            bus_command(&mut bus, &[opcode, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
            let saved = bus.device().save();
            for _ in 0..1000 {
                assert!(bus.device().dma_request_pending());
            }
            assert_eq!(bus.device().save(), saved);
            bus.port_write(DOR, 1, 0x14).unwrap();
            assert!(!bus.device().dma_request_pending());
            let gated = bus.device().save();
            assert!(bus.device_mut().take_dma_request().is_none());
            bus.port_write(DOR, 1, 0x1C).unwrap();
            assert!(bus.device().dma_request_pending());
            let request = bus.device_mut().take_dma_request().unwrap();
            assert!(!bus.device().dma_request_pending());
            let consumed = bus.device().save();
            bus.device_mut().restore(&saved).unwrap();
            finish_test_dma(&mut bus, SECTOR_SIZE, true);
            assert!(!bus.device().dma_request_pending());
            assert!(bus.device().irq_pending());
            bus.device_mut().reset();
            let reset = bus.device().save();
            for (snapshot, pending, enable) in [
                (&saved, true, false),
                (&gated, false, true),
                (&consumed, false, false),
            ] {
                // Restore over an outstanding request so consumed snapshots must clear it.
                bus.device_mut().restore(&saved).unwrap();
                bus.device_mut().restore(snapshot).unwrap();
                assert_eq!(bus.device().dma_request_pending(), pending);
                assert_eq!(
                    bus.device_mut().take_dma_request(),
                    pending.then(|| request.clone())
                );
                if enable {
                    bus.port_write(DOR, 1, 0x1C).unwrap();
                    assert!(bus.device().dma_request_pending());
                    assert_eq!(bus.device_mut().take_dma_request(), Some(request.clone()));
                }
            }
            bus.device_mut().restore(&saved).unwrap();
            assert!(bus.device().dma_request_pending());
            bus.device_mut().reset();
            assert_eq!(bus.device().save(), reset);
            assert!(!bus.device().dma_request_pending());
            assert!(bus.device_mut().take_dma_request().is_none());
            bus.port_write(DOR, 1, 0x1C).unwrap();
            assert!(!bus.device().dma_request_pending());
            assert!(bus.device_mut().take_dma_request().is_none());
        }
    }

    fn finish_test_dma(bus: &mut MockBus, transferred: usize, terminal_count: bool) {
        let request = bus.device_mut().take_dma_request().unwrap();
        let outcome = match request.op {
            DmaOp::ToMemory(_) => DmaOutcome::ToMemory {
                transferred,
                terminal_count,
            },
            DmaOp::FromMemory { .. } => DmaOutcome::FromMemory {
                bytes: vec![0x5A; transferred],
                terminal_count,
            },
        };
        bus.device_mut().dma_complete(outcome);
        assert!(bus.device().irq_pending());
    }

    #[test]
    fn dma_count_distinguishes_eot_and_missing_sector_from_tc() {
        for opcode in [0x06, 0x05, 0x86, 0x85] {
            for (sector, eot, fault) in [(1, 1, ST1_END_OF_CYLINDER), (18, 36, ST1_NO_DATA)] {
                for count in [511, 1023] {
                    let mut fdc = MockBus::new(Box::new(ready_fdc(Vec::new())));
                    let mut dma = Dma8237::new();
                    let mut mem = MockMem::default();
                    program_dma_from_memory(&mut dma, 0, count);
                    if opcode & 0x1F == 6 {
                        dma.port_write(0x0B, 1, 0x46).unwrap();
                    }
                    let mut dma = MockBus::new(Box::new(dma));
                    bus_command(&mut fdc, &[opcode, 4, 7, 1, sector, 2, eot, 0x1B, 0xFF]);
                    let request = fdc.device_mut().take_dma_request().unwrap();
                    let outcome = match request.op {
                        DmaOp::ToMemory(data) => {
                            let transfer = dma.device_mut().dma_service_write(
                                request.channel,
                                &mut mem,
                                &data,
                            );
                            assert_eq!(transfer.transferred, SECTOR_SIZE);
                            DmaOutcome::ToMemory {
                                transferred: transfer.transferred,
                                terminal_count: transfer.terminal_count,
                            }
                        }
                        DmaOp::FromMemory { len } => {
                            let mut bytes = vec![0; len];
                            let transfer = dma.device_mut().dma_service_read(
                                request.channel,
                                &mut mem,
                                &mut bytes,
                            );
                            assert_eq!(transfer.transferred, SECTOR_SIZE);
                            bytes.truncate(transfer.transferred);
                            DmaOutcome::FromMemory {
                                bytes,
                                terminal_count: transfer.terminal_count,
                            }
                        }
                    };
                    fdc.device_mut().dma_complete(outcome);
                    assert!(fdc.device().irq_pending());
                    let result = bus_results(&mut fdc, 7);
                    assert_eq!(
                        result[..3],
                        if count == 511 {
                            [4, 0, 0]
                        } else {
                            [ST0_ABNORMAL | 4, fault, 0]
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn dma_eot_without_tc_reports_end_of_cylinder() {
        for opcode in [0x06, 0x05, 0x86, 0x85] {
            for tc in [false, true] {
                let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
                bus_command(&mut bus, &[opcode, 4, 7, 1, 1, 2, 1, 0x1B, 0xFF]);
                finish_test_dma(&mut bus, SECTOR_SIZE, tc);
                let result = bus_results(&mut bus, 7);
                assert_eq!(
                    &result[..3],
                    &[
                        4 | if tc { 0 } else { ST0_ABNORMAL },
                        if tc { 0 } else { 0x80 },
                        0
                    ]
                );
                if tc {
                    assert_eq!(
                        &result[3..],
                        &[8, if opcode & 0x80 != 0 { 0 } else { 1 }, 1, 2]
                    );
                }
            }
        }
    }

    #[test]
    fn dma_short_without_tc_reports_overrun() {
        for opcode in [0x06, 0x05, 0x86, 0x85] {
            for transferred in [0, 127, SECTOR_SIZE, SECTOR_SIZE + 127] {
                let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
                bus_command(&mut bus, &[opcode, 4, 7, 1, 1, 2, 2, 0x1B, 0xFF]);
                finish_test_dma(&mut bus, transferred, false);
                assert_eq!(&bus_results(&mut bus, 7)[..3], &[ST0_ABNORMAL | 4, 0x10, 0]);
            }
        }
    }

    #[test]
    fn dma_missing_sector_after_prefix_without_tc_reports_no_data() {
        for opcode in [0x06, 0x05, 0x86, 0x85] {
            let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
            bus_command(&mut bus, &[opcode, 0, 7, 0, 18, 2, 36, 0x1B, 0xFF]);
            finish_test_dma(&mut bus, SECTOR_SIZE, false);
            assert_eq!(
                bus_results(&mut bus, 7),
                [ST0_ABNORMAL, ST1_NO_DATA, 0, 7, 0, 19, 2]
            );
        }
    }

    #[test]
    fn dma_tc_before_missing_sector_preserves_normal_result() {
        for opcode in [0x06, 0x05, 0x86, 0x85] {
            for transferred in [127, SECTOR_SIZE] {
                let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
                bus_command(&mut bus, &[opcode, 0, 7, 0, 1, 2, 36, 0x1B, 0xFF]);
                finish_test_dma(&mut bus, transferred, true);
                let result = bus_results(&mut bus, 7);
                assert_eq!(&result[..3], &[0, 0, 0]);
                let sectors = if opcode & 0x1F == 5 {
                    transferred.div_ceil(SECTOR_SIZE)
                } else {
                    transferred / SECTOR_SIZE
                };
                assert_eq!(&result[3..], &[7, 0, 1 + sectors as u8, 2]);
            }
        }
    }

    #[test]
    fn sense_without_pending_interrupt_returns_one_invalid_byte() {
        let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
        // Consume a seek completion so the controller has a stale ST0 and PCN.
        bus_command(&mut bus, &[0x0F, 0x00, 25]);
        bus_command(&mut bus, &[0x08]);
        assert_eq!(bus_results(&mut bus, 2), [ST0_SEEK_END, 25]);
        for _ in 0..2 {
            bus_command(&mut bus, &[0x08]);
            assert_eq!(
                bus.port_read(MSR, 1).unwrap() as u8,
                MSR_RQM | MSR_DIO | MSR_BUSY
            );
            assert_eq!(bus_results(&mut bus, 1), [0x80]);
            assert_eq!(bus.port_read(MSR, 1).unwrap() as u8, MSR_RQM);
            assert!(!bus.device().irq_pending());
        }
    }

    #[test]
    fn data_result_at_eot_advances_chrn_for_both_heads_and_mt_modes() {
        for opcode in [0x06, 0x05] {
            for (mt, head, expected_c, expected_h) in
                [(0, 0, 8, 0), (0, 1, 8, 1), (0x80, 0, 7, 1), (0x80, 1, 8, 0)]
            {
                let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
                bus_command(
                    &mut bus,
                    &[mt | opcode, head << 2, 7, head, 2, 2, 3, 0x1B, 0xFF],
                );
                let request = bus.device_mut().take_dma_request().unwrap();
                assert_eq!(request.channel, FLOPPY_DMA_CHANNEL);
                let outcome = match request.op {
                    DmaOp::ToMemory(_) => DmaOutcome::ToMemory {
                        transferred: 2 * SECTOR_SIZE,
                        terminal_count: true,
                    },
                    DmaOp::FromMemory { .. } => DmaOutcome::FromMemory {
                        bytes: vec![0x5A; 2 * SECTOR_SIZE],
                        terminal_count: true,
                    },
                };
                bus.device_mut().dma_complete(outcome);
                let result = bus_results(&mut bus, 7);
                assert_eq!(
                    &result[1..],
                    &[0, 0, expected_c, expected_h, 1, 2],
                    "opcode={opcode:#x}, MT={mt:#x}, head={head}"
                );
                assert_eq!(result[0] & 0xC0, 0);
                assert_eq!(bus.port_read(MSR, 1).unwrap() as u8, MSR_RQM);
            }
        }
    }

    #[test]
    fn data_result_before_eot_keeps_existing_bytes() {
        for opcode in [0x06, 0x05, 0x86, 0x85, 0xE6, 0xC5] {
            for (sector, eot) in [(2, 4), (18, 36)] {
                let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
                bus_command(&mut bus, &[opcode, 0x04, 7, 1, sector, 2, eot, 0x1B, 0xFF]);
                let request = bus.device_mut().take_dma_request().unwrap();
                let outcome = match request.op {
                    DmaOp::ToMemory(_) => DmaOutcome::ToMemory {
                        transferred: SECTOR_SIZE,
                        terminal_count: true,
                    },
                    DmaOp::FromMemory { .. } => DmaOutcome::FromMemory {
                        bytes: vec![0x5A; SECTOR_SIZE],
                        terminal_count: true,
                    },
                };
                bus.device_mut().dma_complete(outcome);
                assert_eq!(bus_results(&mut bus, 7), [4, 0, 0, 7, 1, sector + 1, 2]);
            }
        }
    }

    #[test]
    fn multitrack_read_continues_at_sector_one_on_head_one() {
        let geometry = FloppyGeometry::DD360;
        let image = patterned(geometry, 0xA3);
        let mut bus = MockBus::new(Box::new(ready_drives([
            Some(Drive::new(geometry, image.clone())),
            None,
            None,
            None,
        ])));
        // EOT is smaller than the physical track: continuation must skip the
        // remaining sectors on head 0 rather than use a contiguous image slice.
        bus_command(&mut bus, &[0x86, 0, 7, 0, 2, 2, 3, 0x1B, 0xFF]);
        let request = bus.device_mut().take_dma_request().unwrap();
        let DmaOp::ToMemory(data) = request.op else {
            panic!("READ DMA direction")
        };
        let mut expected = Vec::new();
        for (head, sector) in [(0, 2), (0, 3), (1, 1), (1, 2), (1, 3)] {
            let offset = geometry.chs_to_offset(7, head, sector);
            expected.extend_from_slice(&image[offset..offset + SECTOR_SIZE]);
        }
        assert_eq!(data.len(), expected.len());
        assert_eq!(data, expected);
        bus.device_mut().dma_complete(DmaOutcome::ToMemory {
            transferred: data.len(),
            terminal_count: true,
        });
        let result = bus_results(&mut bus, 7);
        assert_eq!(&result[1..], &[0, 0, 8, 0, 1, 2]);
    }

    #[test]
    fn multitrack_write_continues_on_head_one_without_touching_track_tail() {
        let geometry = FloppyGeometry::DD360;
        let original = patterned(geometry, 0xC7);
        let mut bus = MockBus::new(Box::new(ready_drives([
            Some(Drive::new(geometry, original.clone())),
            None,
            None,
            None,
        ])));
        bus_command(&mut bus, &[0x85, 0, 7, 0, 2, 2, 3, 0x1B, 0xFF]);
        let request = bus.device_mut().take_dma_request().unwrap();
        assert_eq!(
            request.op,
            DmaOp::FromMemory {
                len: 5 * SECTOR_SIZE
            }
        );
        let mut expected = original;
        let mut bytes = Vec::new();
        for (index, (head, sector)) in [(0, 2), (0, 3), (1, 1), (1, 2), (1, 3)]
            .into_iter()
            .enumerate()
        {
            let data = vec![0x30 + index as u8; SECTOR_SIZE];
            bytes.extend_from_slice(&data);
            let offset = geometry.chs_to_offset(7, head, sector);
            expected[offset..offset + SECTOR_SIZE].copy_from_slice(&data);
        }
        bus.device_mut().dma_complete(DmaOutcome::FromMemory {
            bytes,
            terminal_count: true,
        });
        assert_eq!(&bus_results(&mut bus, 7)[1..], &[0, 0, 8, 0, 1, 2]);
        // Read every sector of both tracks through the public DMA seam.
        for head in 0..2 {
            bus_command(&mut bus, &[0x06, head << 2, 7, head, 1, 2, 9, 0x1B, 0xFF]);
            let DmaOp::ToMemory(data) = bus.device_mut().take_dma_request().unwrap().op else {
                panic!("READ DMA direction")
            };
            let offset = geometry.chs_to_offset(7, head, 1);
            assert_eq!(data, expected[offset..offset + 9 * SECTOR_SIZE]);
            bus.device_mut().dma_complete(DmaOutcome::ToMemory {
                transferred: data.len(),
                terminal_count: true,
            });
            let _ = bus_results(&mut bus, 7);
        }
    }

    #[test]
    fn data_stops_at_physical_track_without_aliasing_next_track() {
        for opcode in [0x06, 0x05, 0x86, 0x85] {
            let geometry = FloppyGeometry::DD360;
            let image = patterned(geometry, 0x5C);
            let mut bus = MockBus::new(Box::new(ready_drives([
                Some(Drive::new(geometry, image.clone())),
                None,
                None,
                None,
            ])));
            bus_command(&mut bus, &[opcode, 0, 7, 0, 9, 2, 10, 0x1B, 0xFF]);
            let request = bus.device_mut().take_dma_request().unwrap();
            let outcome = match request.op {
                DmaOp::ToMemory(data) => {
                    let offset = geometry.chs_to_offset(7, 0, 9);
                    assert_eq!(data.len(), SECTOR_SIZE, "no sector 10 from another track");
                    assert_eq!(data, image[offset..offset + SECTOR_SIZE]);
                    DmaOutcome::ToMemory {
                        transferred: data.len(),
                        terminal_count: true,
                    }
                }
                DmaOp::FromMemory { len } => {
                    assert_eq!(len, SECTOR_SIZE, "no write into another track");
                    DmaOutcome::FromMemory {
                        bytes: vec![0xAA; len],
                        terminal_count: true,
                    }
                }
            };
            bus.device_mut().dma_complete(outcome);
            assert!(bus.device().irq_pending());
            // TC on the last valid sector prevents a search for the absent next ID.
            assert_eq!(bus_results(&mut bus, 7), [0, 0, 0, 7, 0, 10, 2]);
            // The next track remains byte-identical even after WRITE DATA.
            bus_command(&mut bus, &[0x06, 4, 7, 1, 1, 2, 1, 0x1B, 0xFF]);
            let DmaOp::ToMemory(data) = bus.device_mut().take_dma_request().unwrap().op else {
                panic!("READ DMA direction")
            };
            let offset = geometry.chs_to_offset(7, 1, 1);
            assert_eq!(data, image[offset..offset + SECTOR_SIZE]);
            bus.device_mut().dma_complete(DmaOutcome::ToMemory {
                transferred: data.len(),
                terminal_count: true,
            });
            let _ = bus_results(&mut bus, 7);
        }
    }

    #[test]
    fn data_starting_past_physical_track_faults_without_dma() {
        for opcode in [0x06, 0x05] {
            let mut bus = MockBus::new(Box::new(ready_fdc(Vec::new())));
            bus_command(&mut bus, &[opcode, 4, 7, 1, 19, 2, 19, 0x1B, 0xFF]);
            assert!(bus.device_mut().take_dma_request().is_none());
            assert_eq!(
                bus_results(&mut bus, 7),
                [ST0_ABNORMAL | 4, ST1_NO_DATA, 0, 7, 1, 19, 2]
            );
        }
    }

    #[test]
    fn power_on_state_is_idle_and_ready() {
        let fdc = Upd765::new();
        assert_eq!(
            fdc.msr(),
            MSR_RQM,
            "idle controller reports RQM ready, DIO host"
        );
        assert!(!fdc.irq_pending());
    }

    #[test]
    fn chs_maps_to_image_offset() {
        let g = FloppyGeometry::HD1440;
        assert_eq!(g.chs_to_offset(0, 0, 1), 0);
        assert_eq!(g.chs_to_offset(0, 0, 2), SECTOR_SIZE);
        assert_eq!(g.chs_to_offset(0, 1, 1), 18 * SECTOR_SIZE);
        assert_eq!(g.chs_to_offset(1, 0, 1), 36 * SECTOR_SIZE);
    }

    #[test]
    fn chs_offset_and_size_track_the_geometry() {
        // 360 KB has 9 sectors per track over 40 cylinders, so head 1 and the
        // next cylinder sit at different offsets than a 1.44 MB disk would.
        let g = FloppyGeometry::DD360;
        assert_eq!(g.image_bytes(), 40 * 2 * 9 * SECTOR_SIZE);
        assert_eq!(g.chs_to_offset(0, 0, 1), 0);
        assert_eq!(g.chs_to_offset(0, 1, 1), 9 * SECTOR_SIZE);
        assert_eq!(g.chs_to_offset(1, 0, 1), 18 * SECTOR_SIZE);

        // 1.2 MB and 720 KB image sizes.
        assert_eq!(
            FloppyGeometry::HD1200.image_bytes(),
            80 * 2 * 15 * SECTOR_SIZE
        );
        assert_eq!(
            FloppyGeometry::DD720.image_bytes(),
            80 * 2 * 9 * SECTOR_SIZE
        );
        assert_eq!(FloppyGeometry::HD1440.image_bytes(), FLOPPY_IMAGE_BYTES);

        // A cylinder past a 40-cylinder disk, or a sector past the track, is out.
        assert!(!FloppyGeometry::DD360.contains(40, 0, 1));
        assert!(!FloppyGeometry::DD360.contains(0, 0, 10));
        assert!(!FloppyGeometry::DD360.contains(0, 2, 1));
        assert!(FloppyGeometry::DD360.contains(39, 1, 9));
    }

    #[test]
    fn dor_reset_edge_raises_irq_and_queues_polls() {
        let mut fdc = Upd765::new();
        fdc.port_write(DOR, 1, 0x1C).unwrap();
        assert!(fdc.irq_pending(), "leaving reset raises IRQ6");
        assert_eq!(fdc.reset_poll, 4);
        // Four SENSE INTERRUPT STATUS calls report the polling status.
        for i in 0..4 {
            command(&mut fdc, &[0x08]);
            let res = results(&mut fdc, 2);
            assert_eq!(res[0], ST0_POLL | i, "poll {i} reports abnormal status");
            assert_eq!(res[1], 0, "present cylinder 0 after reset");
        }
        assert!(!fdc.irq_pending(), "the interrupt clears once sensed");
    }

    #[test]
    fn specify_has_no_result_and_no_irq() {
        let mut fdc = ready_fdc(Vec::new());
        command(&mut fdc, &[0x03, 0xDF, 0x02]);
        assert_eq!(fdc.msr(), MSR_RQM, "back to idle after SPECIFY");
        assert!(!fdc.irq_pending(), "SPECIFY raises no interrupt");
    }

    #[test]
    fn recalibrate_seeks_to_zero_and_interrupts() {
        let mut fdc = ready_fdc(Vec::new());
        // Move the drive off track 0 first so recalibrate is observable.
        command(&mut fdc, &[0x0F, 0x00, 40]); // SEEK drive 0 -> cyl 40
        command(&mut fdc, &[0x08]);
        let _ = results(&mut fdc, 2);
        fdc.irq = false;
        command(&mut fdc, &[0x07, 0x00]); // RECALIBRATE drive 0
        assert!(fdc.irq_pending(), "RECALIBRATE raises IRQ6");
        command(&mut fdc, &[0x08]); // SENSE INTERRUPT STATUS
        let res = results(&mut fdc, 2);
        assert_eq!(res[0], ST0_SEEK_END, "seek-end for drive 0");
        assert_eq!(res[1], 0, "recalibrated to cylinder 0");
        assert_eq!(fdc.present_cylinder(0), 0);
    }

    #[test]
    fn seek_sets_present_cylinder_and_interrupts() {
        let mut fdc = ready_fdc(Vec::new());
        command(&mut fdc, &[0x0F, 0x00, 25]); // SEEK drive 0 -> cyl 25
        assert!(fdc.irq_pending(), "SEEK raises IRQ6");
        assert_eq!(fdc.present_cylinder(0), 25);
        command(&mut fdc, &[0x08]);
        let res = results(&mut fdc, 2);
        assert_eq!(res[0], ST0_SEEK_END, "seek-end, drive 0, head 0");
        assert_eq!(res[1], 25, "present cylinder 25");
        assert!(!fdc.irq_pending());
    }

    #[test]
    fn read_id_returns_current_position() {
        let mut fdc = ready_fdc(Vec::new());
        command(&mut fdc, &[0x0F, 0x00, 7]); // SEEK to cyl 7
        command(&mut fdc, &[0x08]);
        let _ = results(&mut fdc, 2);
        fdc.irq = false;
        command(&mut fdc, &[0x4A, 0x00]); // READ ID, drive 0 head 0
        assert!(fdc.irq_pending());
        let res = results(&mut fdc, 7);
        assert_eq!(res[3], 7, "C = present cylinder");
        assert_eq!(res[5], 1, "R = 1");
        assert_eq!(res[6], 2, "N = 2 (512 bytes)");
        assert!(
            !fdc.irq_pending(),
            "reading the result acknowledges the interrupt"
        );
    }

    #[test]
    fn read_data_queues_the_sector_and_raises_irq() {
        // Seed a distinctive first sector.
        let mut image = vec![0u8; SECTOR_SIZE];
        for (i, b) in image.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let mut fdc = ready_fdc(image.clone());

        // READ DATA: drive/head 0, C=0, H=0, R=1, N=2, EOT=1, GPL, DTL.
        command(
            &mut fdc,
            &[0x06, 0x00, 0x00, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF],
        );
        assert_eq!(
            fdc.phase,
            Phase::Execution,
            "READ enters execution awaiting DMA"
        );
        assert!(
            !fdc.irq_pending(),
            "no interrupt until the transfer completes"
        );

        let request = fdc.take_dma_request().expect("a DMA request is queued");
        assert_eq!(request.channel, FLOPPY_DMA_CHANNEL);
        assert_eq!(
            request.op,
            DmaOp::ToMemory(image.clone()),
            "the queued bytes are sector 0, moved device -> memory"
        );
        assert!(
            fdc.take_dma_request().is_none(),
            "the request is taken once"
        );

        fdc.dma_complete(DmaOutcome::ToMemory {
            transferred: SECTOR_SIZE,
            terminal_count: true,
        });
        assert!(fdc.irq_pending(), "IRQ6 rises on transfer completion");
        let res = results(&mut fdc, 7);
        assert_eq!(res[0] & 0xC0, 0, "ST0 normal termination");
        assert_eq!(res[3], 1, "result cylinder advances at EOT");
        assert_eq!(res[5], 1, "result sector resets at EOT");
        assert_eq!(res[6], 2, "N echoed");
    }

    #[test]
    fn read_data_transfers_through_the_8237_into_memory() {
        let mut image = vec![0u8; SECTOR_SIZE];
        for (i, b) in image.iter_mut().enumerate() {
            *b = (0xA0 ^ i) as u8;
        }
        let mut fdc = ready_fdc(image.clone());
        let mut dma = Dma8237::new();
        let mut mem = MockMem::default();

        // Program 8237 channel 2 for a write transfer (device -> memory) to
        // physical 0x07C00, count 511 (512 bytes).
        dma.port_write(0x0A, 1, 0x06).unwrap(); // mask channel 2
        dma.port_write(0x0C, 1, 0).unwrap(); // clear flip-flop
        dma.port_write(0x0B, 1, 0x46).unwrap(); // mode: ch2, write, single
        dma.port_write(0x04, 1, 0x00).unwrap(); // addr low
        dma.port_write(0x04, 1, 0x7C).unwrap(); // addr high
        dma.port_write(0x81, 1, 0x00).unwrap(); // page (channel 2)
        dma.port_write(0x05, 1, 0xFF).unwrap(); // count low
        dma.port_write(0x05, 1, 0x01).unwrap(); // count high
        dma.port_write(0x0A, 1, 0x02).unwrap(); // unmask channel 2

        command(
            &mut fdc,
            &[0x06, 0x00, 0x00, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF],
        );
        let request = fdc.take_dma_request().expect("queued request");
        let DmaOp::ToMemory(data) = request.op else {
            panic!("a floppy read queues a device -> memory transfer");
        };
        let n = dma.dma_service_write(request.channel, &mut mem, &data);
        assert_eq!(n.transferred, SECTOR_SIZE, "the whole sector transfers");
        fdc.dma_complete(DmaOutcome::ToMemory {
            transferred: n.transferred,
            terminal_count: n.terminal_count,
        });

        for (i, &expected) in image.iter().enumerate() {
            assert_eq!(
                mem.dma_read(0x07C00 + i as u32),
                expected,
                "byte {i} in memory"
            );
        }
        assert!(fdc.irq_pending());
    }

    /// Program an 8237 channel 2 for a memory→device (read) transfer at physical
    /// `address`, moving `count + 1` bytes — the direction a floppy WRITE drains.
    fn program_dma_from_memory(dma: &mut Dma8237, address: u32, count: u16) {
        let addr = address as u16;
        dma.port_write(0x0A, 1, 0x06).unwrap(); // mask channel 2
        dma.port_write(0x0C, 1, 0).unwrap(); // clear flip-flop
        dma.port_write(0x0B, 1, 0x4A).unwrap(); // mode: ch2, read, single
        dma.port_write(0x04, 1, u32::from(addr & 0xFF)).unwrap(); // addr low
        dma.port_write(0x04, 1, u32::from(addr >> 8)).unwrap(); // addr high
        dma.port_write(0x81, 1, address >> 16).unwrap(); // page (channel 2)
        dma.port_write(0x05, 1, u32::from(count & 0xFF)).unwrap(); // count low
        dma.port_write(0x05, 1, u32::from(count >> 8)).unwrap(); // count high
        dma.port_write(0x0A, 1, 0x02).unwrap(); // unmask channel 2
    }

    #[test]
    fn write_data_commits_memory_then_reads_back_identical() {
        // Drive 0 starts with an empty image, so the write must also grow the
        // backing buffer to hold the sector.
        let mut fdc = ready_fdc(Vec::new());
        let mut dma = Dma8237::new();
        let mut mem = MockMem::default();

        // A distinctive sector sitting in guest memory at physical 0x07C00.
        let pattern: Vec<u8> = (0..SECTOR_SIZE).map(|i| (0x5A ^ i) as u8).collect();
        for (i, &b) in pattern.iter().enumerate() {
            mem.dma_write(0x07C00 + i as u32, b);
        }
        program_dma_from_memory(&mut dma, 0x07C00, (SECTOR_SIZE - 1) as u16);

        // WRITE DATA: drive/head 0, C=0, H=0, R=1, N=2, EOT=1.
        command(
            &mut fdc,
            &[0x05, 0x00, 0x00, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF],
        );
        assert_eq!(
            fdc.msr(),
            MSR_BUSY,
            "the write execution phase reports BUSY only (host drives no data port)"
        );
        assert!(
            !fdc.irq_pending(),
            "no interrupt until the transfer completes"
        );

        let request = fdc
            .take_dma_request()
            .expect("a write queues a DMA request");
        let DmaOp::FromMemory { len } = request.op else {
            panic!("a floppy write queues a memory -> device transfer");
        };
        assert_eq!(len, SECTOR_SIZE, "one sector requested from memory");
        let mut buf = vec![0u8; len];
        let n = dma.dma_service_read(request.channel, &mut mem, &mut buf);
        assert_eq!(n.transferred, SECTOR_SIZE, "the whole sector transfers");
        buf.truncate(n.transferred);
        fdc.dma_complete(DmaOutcome::FromMemory {
            bytes: buf,
            terminal_count: n.terminal_count,
        });

        assert!(fdc.irq_pending(), "IRQ6 rises on write completion");
        let res = results(&mut fdc, 7);
        assert_eq!(res[0] & 0xC0, 0, "ST0 normal termination");
        assert_eq!(&res[3..], &[1, 0, 1, 2], "result ID advances at EOT");

        // Read the same CHS back through the controller: the bytes committed to
        // media are byte-identical to what was written from memory.
        let read_back = read_sector(&mut fdc, 0x00, 0, 0, 1);
        assert_eq!(
            read_back, pattern,
            "the sector reads back exactly as written"
        );
    }

    type RecordedBatches = std::rc::Rc<std::cell::RefCell<Vec<Vec<SectorWrite>>>>;

    struct RecordingMedia {
        memory: InMemoryMedia,
        batches: RecordedBatches,
    }

    impl SectorMedia for RecordingMedia {
        fn sector_count(&self) -> u64 {
            self.memory.sector_count()
        }
        fn read_sector(&self, lba: u64) -> [u8; SECTOR_SIZE] {
            self.memory.read_sector(lba)
        }

        fn write_batch(&mut self, sectors: &[SectorWrite]) {
            self.batches.borrow_mut().push(sectors.to_vec());
            self.memory.write_batch(sectors);
        }

        fn write_protected(&self) -> bool {
            self.memory.write_protected()
        }
    }

    fn recording_media_bus(protected: bool) -> (MockBus, RecordedBatches) {
        let batches = RecordedBatches::default();
        let drive = Drive::with_media(
            FloppyGeometry::DD360,
            Box::new(RecordingMedia {
                memory: InMemoryMedia::new(
                    vec![0xAA; FloppyGeometry::DD360.image_bytes()],
                    FloppyGeometry::DD360.sector_count(),
                    protected,
                ),
                batches: batches.clone(),
            }),
        );
        let bus = MockBus::new(Box::new(ready_drives([Some(drive), None, None, None])));
        (bus, batches)
    }

    fn complete_media_write(bus: &mut MockBus, bytes: Vec<u8>, terminal_count: bool) -> Vec<u8> {
        let request = bus
            .device_mut()
            .take_dma_request()
            .expect("write DMA queued");
        assert_eq!(request.channel, FLOPPY_DMA_CHANNEL);
        let DmaOp::FromMemory { len } = request.op else {
            panic!("WRITE DMA direction");
        };
        assert!(bytes.len() <= len);
        bus.device_mut().dma_complete(DmaOutcome::FromMemory {
            bytes,
            terminal_count,
        });
        assert!(bus.device().irq_pending());
        let result = bus_results(bus, 7);
        assert!(!bus.device().irq_pending());
        result
    }

    fn media_read(bus: &mut MockBus, c: u8, h: u8, r: u8) -> Vec<u8> {
        bus_command(bus, &[0x06, h << 2, c, h, r, 2, r, 0x1B, 0xFF]);
        let DmaOp::ToMemory(bytes) = bus.device_mut().take_dma_request().unwrap().op else {
            panic!("READ DMA direction");
        };
        bus.device_mut().dma_complete(DmaOutcome::ToMemory {
            transferred: bytes.len(),
            terminal_count: true,
        });
        assert_eq!(bus_results(bus, 7)[1], 0);
        bytes
    }

    #[test]
    fn media_contract_drive_uses_extended_geometry_capacity() {
        let geometry = FloppyGeometry {
            cylinders: 80,
            heads: 2,
            sectors_per_track: 36,
        };
        let mut bus = MockBus::new(Box::new(ready_drives([
            Some(Drive::new(geometry, Vec::new())),
            None,
            None,
            None,
        ])));
        bus_command(&mut bus, &[0x05, 4, 79, 1, 36, 2, 36, 0x1B, 0xFF]);
        let result = complete_media_write(&mut bus, vec![0xBB; SECTOR_SIZE], true);
        assert_eq!(result[1], 0);
        assert_eq!(media_read(&mut bus, 79, 1, 36), [0xBB; SECTOR_SIZE]);
    }

    #[test]
    fn media_contract_batches_commands_in_transfer_order() {
        let (mut bus, batches) = recording_media_bus(false);
        bus_command(&mut bus, &[0x85, 0, 2, 0, 2, 2, 3, 0x1B, 0xFF]);
        assert!(
            batches.borrow().is_empty(),
            "no batch before DMA completion"
        );
        assert!(!bus.device().irq_pending());
        assert_eq!(bus.port_read(MSR, 1).unwrap() as u8, MSR_BUSY);
        let expected: Vec<_> = [37, 38, 45, 46, 47]
            .into_iter()
            .enumerate()
            .map(|(index, lba)| SectorWrite {
                lba,
                bytes: [0x30 + index as u8; SECTOR_SIZE],
            })
            .collect();
        let bytes = expected.iter().flat_map(|sector| sector.bytes).collect();
        assert_eq!(
            complete_media_write(&mut bus, bytes, true),
            [4, 0, 0, 3, 0, 1, 2]
        );
        assert_eq!(*batches.borrow(), vec![expected.clone()]);
        for (index, (head, sector)) in [(0, 2), (0, 3), (1, 1), (1, 2), (1, 3)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(media_read(&mut bus, 2, head, sector), expected[index].bytes);
        }
        bus_command(&mut bus, &[0x05, 4, 1, 1, 4, 2, 4, 0x1B, 0xFF]);
        assert_eq!(batches.borrow().len(), 1);
        complete_media_write(&mut bus, vec![0xEF; SECTOR_SIZE], true);
        assert_eq!(
            *batches.borrow(),
            vec![
                expected,
                vec![SectorWrite {
                    lba: 30,
                    bytes: [0xEF; SECTOR_SIZE]
                }]
            ]
        );
    }

    #[test]
    fn media_contract_partial_sector_is_padded_in_one_batch() {
        let (mut bus, batches) = recording_media_bus(false);
        assert_eq!(media_read(&mut bus, 0, 0, 2), [0xAA; SECTOR_SIZE]);
        bus_command(&mut bus, &[0x05, 0, 0, 0, 1, 2, 3, 0x1B, 0xFF]);
        let bytes: Vec<_> = (0..SECTOR_SIZE + 100).map(|i| (i ^ 0xC3) as u8).collect();
        let first: [u8; SECTOR_SIZE] = bytes[..SECTOR_SIZE].try_into().unwrap();
        let mut partial = [0; SECTOR_SIZE];
        partial[..100].copy_from_slice(&bytes[SECTOR_SIZE..]);
        assert_eq!(
            complete_media_write(&mut bus, bytes, true),
            [0, 0, 0, 0, 0, 3, 2]
        );
        assert_eq!(
            *batches.borrow(),
            vec![vec![
                SectorWrite {
                    lba: 0,
                    bytes: first
                },
                SectorWrite {
                    lba: 1,
                    bytes: partial
                },
            ]]
        );
        let read_back = media_read(&mut bus, 0, 0, 2);
        assert_eq!(read_back, partial);
        assert!(read_back[100..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn media_contract_abnormal_completion_keeps_committed_sectors() {
        for (sector, eot, supplied, status, lbas) in [
            (2, 3, SECTOR_SIZE + 5, ST1_OVERRUN, vec![1, 2]),
            (9, 10, SECTOR_SIZE, ST1_NO_DATA, vec![8]),
            (2, 3, 2 * SECTOR_SIZE, ST1_END_OF_CYLINDER, vec![1, 2]),
        ] {
            let (mut bus, batches) = recording_media_bus(false);
            assert_eq!(media_read(&mut bus, 0, 0, sector), [0xAA; SECTOR_SIZE]);
            bus_command(&mut bus, &[0x05, 0, 0, 0, sector, 2, eot, 0x1B, 0xFF]);
            let result = complete_media_write(&mut bus, vec![0x5A; supplied], false);
            assert_eq!(result[0] & 0xC0, ST0_ABNORMAL);
            assert_eq!(result[1], status);
            let expected: Vec<_> = lbas
                .into_iter()
                .enumerate()
                .map(|(index, lba)| {
                    let mut bytes = [0; SECTOR_SIZE];
                    bytes[..(supplied - index * SECTOR_SIZE).min(SECTOR_SIZE)].fill(0x5A);
                    SectorWrite { lba, bytes }
                })
                .collect();
            assert_eq!(*batches.borrow(), vec![expected.clone()]);
            for replacement in expected {
                let read_back = media_read(&mut bus, 0, 0, replacement.lba as u8 + 1);
                assert_eq!(read_back, replacement.bytes);
                if status == ST1_OVERRUN && replacement.lba == 2 {
                    assert!(read_back[5..].iter().all(|&byte| byte == 0));
                }
            }
        }
    }

    #[test]
    fn media_contract_read_only_rejects_without_a_batch() {
        let (mut bus, batches) = recording_media_bus(true);
        bus_command(&mut bus, &[0x05, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
        assert!(bus.device_mut().take_dma_request().is_none());
        assert!(bus.device().irq_pending());
        assert_eq!(
            bus_results(&mut bus, 7),
            [ST0_ABNORMAL, ST1_NOT_WRITABLE, 0, 0, 0, 1, 2]
        );
        assert!(batches.borrow().is_empty());
        assert_eq!(media_read(&mut bus, 0, 0, 1), [0xAA; SECTOR_SIZE]);
    }

    #[test]
    fn media_contract_empty_and_invalid_writes_emit_no_batch() {
        let (mut bus, batches) = recording_media_bus(false);
        for tc in [false, true] {
            bus_command(&mut bus, &[0x05, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
            let result = complete_media_write(&mut bus, Vec::new(), tc);
            assert_eq!(result[1], if tc { 0 } else { ST1_OVERRUN });
            assert!(batches.borrow().is_empty());
        }
        bus_command(&mut bus, &[0x05, 0, 40, 0, 1, 2, 1, 0x1B, 0xFF]);
        assert!(bus.device_mut().take_dma_request().is_none());
        assert_eq!(bus_results(&mut bus, 7)[1], ST1_NO_DATA);
        assert!(batches.borrow().is_empty());
        assert_eq!(media_read(&mut bus, 0, 0, 1), [0xAA; SECTOR_SIZE]);
    }

    #[test]
    fn media_contract_reset_preserves_media_and_discards_uncompleted_write() {
        let (mut bus, batches) = recording_media_bus(false);
        bus_command(&mut bus, &[0x05, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
        complete_media_write(&mut bus, vec![0x55; SECTOR_SIZE], true);
        let snapshot = bus.device().save();
        bus_command(&mut bus, &[0x05, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
        complete_media_write(&mut bus, vec![0x66; SECTOR_SIZE], true);
        bus.device_mut().restore(&snapshot).unwrap();
        assert_eq!(media_read(&mut bus, 0, 0, 1), [0x66; SECTOR_SIZE]);
        bus_command(&mut bus, &[0x05, 0, 0, 0, 1, 2, 1, 0x1B, 0xFF]);
        bus.device_mut().reset();
        assert!(bus.device_mut().take_dma_request().is_none());
        assert!(!bus.device().irq_pending());
        bus.device_mut().dma_complete(DmaOutcome::FromMemory {
            bytes: vec![0x77; SECTOR_SIZE],
            terminal_count: true,
        });
        assert_eq!(batches.borrow().len(), 2);
        bus.port_write(DOR, 1, 0x1C).unwrap();
        for _ in 0..4 {
            bus_command(&mut bus, &[0x08]);
            bus_results(&mut bus, 2);
        }
        bus.port_write(DIR_CCR, 1, 2).unwrap();
        assert_eq!(media_read(&mut bus, 0, 0, 1), [0x66; SECTOR_SIZE]);
        assert_eq!(batches.borrow().len(), 2);
    }

    #[test]
    fn write_out_of_geometry_faults_no_data() {
        // 360 KB disk has 40 cylinders; cylinder 40 is one past the last.
        let mut fdc = ready_drives([
            Some(Drive::new(FloppyGeometry::DD360, vec![0u8; SECTOR_SIZE])),
            None,
            None,
            None,
        ]);
        command(
            &mut fdc,
            &[0x05, 0x00, 40, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF],
        );
        assert!(
            fdc.take_dma_request().is_none(),
            "an out-of-geometry write queues no transfer"
        );
        let res = results(&mut fdc, 7);
        assert_eq!(res[0] & 0x40, 0x40, "ST0 abnormal termination");
        assert_eq!(
            res[1] & 0x04,
            0x04,
            "ST1 ND: no data field past the geometry"
        );
    }

    #[test]
    fn write_to_protected_disk_faults_not_writable() {
        let original = vec![0xAAu8; SECTOR_SIZE];
        let mut fdc = ready_drives([
            Some(Drive::new_read_only(
                FloppyGeometry::HD1440,
                original.clone(),
            )),
            None,
            None,
            None,
        ]);
        command(
            &mut fdc,
            &[0x05, 0x00, 0x00, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF],
        );
        assert!(
            fdc.take_dma_request().is_none(),
            "a protected disk queues no transfer"
        );
        let res = results(&mut fdc, 7);
        assert_eq!(res[0] & 0x40, 0x40, "ST0 abnormal termination");
        assert_eq!(res[1] & 0x02, 0x02, "ST1 NW: not writable");
        assert_eq!(
            fdc.drives[0]
                .as_ref()
                .map(|d| d.media.read_sector(0).to_vec()),
            Some(original),
            "the image is not mutated"
        );
    }

    #[test]
    fn writeterminal_count_zero_fills_partial_sector() {
        // Non-zero old bytes make retaining the partial sector's tail observable.
        let mut fdc = ready_fdc(vec![0xAA; SECTOR_SIZE]);
        let mut dma = Dma8237::new();
        let mut mem = MockMem::default();

        const SUPPLIED: usize = 100;
        let pattern: Vec<u8> = (0..SUPPLIED).map(|i| (0xC3 ^ i) as u8).collect();
        for (i, &b) in pattern.iter().enumerate() {
            mem.dma_write(0x07C00 + i as u32, b);
        }
        // Program only 100 bytes (count 99): terminal count cuts the sector short.
        program_dma_from_memory(&mut dma, 0x07C00, (SUPPLIED - 1) as u16);

        command(
            &mut fdc,
            &[0x05, 0x00, 0x00, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF],
        );
        let request = fdc
            .take_dma_request()
            .expect("a write queues a DMA request");
        let DmaOp::FromMemory { len } = request.op else {
            panic!("a floppy write queues a memory -> device transfer");
        };
        let mut buf = vec![0u8; len];
        let n = dma.dma_service_read(request.channel, &mut mem, &mut buf);
        assert_eq!(
            n.transferred, SUPPLIED,
            "terminal count clamps the transfer"
        );
        buf.truncate(n.transferred);
        fdc.dma_complete(DmaOutcome::FromMemory {
            bytes: buf,
            terminal_count: n.terminal_count,
        });

        let res = results(&mut fdc, 7);
        assert_eq!(&res[3..], &[1, 0, 1, 2], "the completed sector reaches EOT");
        let image = fdc.drives[0].as_ref().unwrap().media.read_sector(0);
        assert_eq!(&image[..SUPPLIED], &pattern[..], "supplied bytes committed");
        assert!(
            image[SUPPLIED..SECTOR_SIZE].iter().all(|&b| b == 0),
            "the tail of the partial sector is zero-filled"
        );
    }

    #[test]
    fn unknown_command_reports_invalid() {
        let mut fdc = ready_fdc(Vec::new());
        command(&mut fdc, &[0x1D]); // not a modelled opcode
        assert!(!fdc.irq_pending(), "INVALID does not generate INT");
        let res = results(&mut fdc, 1);
        assert_eq!(res[0], 0x80, "invalid-command status");
        assert!(!fdc.irq_pending());
    }

    #[test]
    fn reset_preserves_the_disk_but_clears_registers() {
        let image = vec![0x5Au8; SECTOR_SIZE];
        let mut fdc = ready_fdc(image.clone());
        command(&mut fdc, &[0x0F, 0x00, 30]); // SEEK to cyl 30
        fdc.reset();
        assert_eq!(fdc.present_cylinder(0), 0, "registers cleared");
        assert_eq!(fdc.dor, 0);
        assert!(!fdc.irq_pending());
        assert_eq!(
            fdc.drives[0]
                .as_ref()
                .map(|d| d.media.read_sector(0).to_vec()),
            Some(image),
            "the disk stays attached across reset"
        );
    }

    #[test]
    fn save_restore_round_trip() {
        let mut fdc = ready_fdc(vec![0u8; SECTOR_SIZE]);
        command(&mut fdc, &[0x0F, 0x00, 42]); // SEEK, leaves an interrupt pending
        let blob = fdc.save();

        let mut restored = Upd765::with_image(vec![0; SECTOR_SIZE]);
        restored.restore(&blob).unwrap();
        assert_eq!(restored.present_cylinder(0), 42);
        assert_eq!(restored.dor, fdc.dor);
        assert_eq!(restored.irq_pending(), fdc.irq_pending());

        // A pending SENSE resolves identically after restore.
        command(&mut restored, &[0x08]);
        let res = results(&mut restored, 2);
        assert_eq!(res[1], 42);
    }

    #[test]
    fn restore_rejects_bad_snapshot() {
        let mut fdc = Upd765::new();
        assert!(matches!(
            fdc.restore(&[0xFF]),
            Err(DeviceError::InvalidSnapshot { .. })
        ));
        assert!(matches!(
            fdc.restore(&[]),
            Err(DeviceError::InvalidSnapshot { .. })
        ));
    }

    #[test]
    fn mock_bus_isolation_harness() {
        let mut bus = MockBus::new(Box::new(Upd765::new()));
        bus.tick(1);
        // MSR reads ready at power-on through the bus decode.
        assert_eq!(bus.port_read(MSR, 1).unwrap() as u8, MSR_RQM);
        // A DOR write out of reset raises the IRQ line, visible through the seam.
        bus.port_write(DOR, 1, 0x1C).unwrap();
        assert!(bus.device().irq_pending());
    }

    #[test]
    fn port_outside_window_is_unmapped() {
        let mut fdc = Upd765::new();
        assert!(matches!(
            fdc.port_read(0x3E0, 1),
            Err(DeviceError::UnmappedRegister { kind: "port", .. })
        ));
    }

    /// A distinctive full-geometry image whose every byte is a function of its
    /// offset and a per-disk seed, so a read landing on the wrong offset or the
    /// wrong drive is caught byte-for-byte.
    fn patterned(geometry: FloppyGeometry, seed: u8) -> Vec<u8> {
        let mut img = vec![0u8; geometry.image_bytes()];
        for (i, b) in img.iter_mut().enumerate() {
            *b = seed ^ (i % 251) as u8;
        }
        img
    }

    /// Issue a single-sector READ DATA, run the DMA to completion, drain the
    /// result phase, and return the sector bytes the controller pulled.
    fn read_sector(fdc: &mut Upd765, drive_head: u8, c: u8, h: u8, r: u8) -> Vec<u8> {
        command(fdc, &[0x06, drive_head, c, h, r, 0x02, r, 0x1B, 0xFF]);
        let DmaOp::ToMemory(data) = fdc
            .take_dma_request()
            .expect("a valid read queues a DMA request")
            .op
        else {
            panic!("a floppy read queues a device -> memory transfer");
        };
        fdc.dma_complete(DmaOutcome::ToMemory {
            transferred: data.len(),
            terminal_count: true,
        });
        let _ = results(fdc, 7);
        data
    }

    #[test]
    fn read_uses_the_drive_geometry_for_the_offset() {
        // Read cylinder 1 on both a 360 KB and a 1.44 MB disk: the same CHS lands
        // at different image offsets because the geometries differ.
        for geometry in [
            FloppyGeometry::DD360,
            FloppyGeometry::DD720,
            FloppyGeometry::HD1200,
            FloppyGeometry::HD1440,
        ] {
            let image = patterned(geometry, 0x33);
            let mut fdc =
                ready_drives([Some(Drive::new(geometry, image.clone())), None, None, None]);
            let data = read_sector(&mut fdc, 0x00, 1, 0, 1);
            let offset = geometry.chs_to_offset(1, 0, 1);
            assert_eq!(
                data,
                image[offset..offset + SECTOR_SIZE],
                "read must use {geometry:?} to compute the offset"
            );
        }
    }

    #[test]
    fn drives_zero_and_one_carry_independent_media() {
        let img0 = patterned(FloppyGeometry::DD360, 0x11);
        let img1 = patterned(FloppyGeometry::HD1440, 0xEE);
        let mut fdc = ready_drives([
            Some(Drive::new(FloppyGeometry::DD360, img0.clone())),
            Some(Drive::new(FloppyGeometry::HD1440, img1.clone())),
            None,
            None,
        ]);

        // Drive 0 (drive-select bits 00) reads its own 360 KB media at the 360 KB
        // offset for C=1; drive 1 (bits 01) reads its 1.44 MB media at the 1.44 MB
        // offset for the same CHS. Neither sees the other's disk.
        let d0 = read_sector(&mut fdc, 0x00, 1, 0, 1);
        let o0 = FloppyGeometry::DD360.chs_to_offset(1, 0, 1);
        assert_eq!(d0, img0[o0..o0 + SECTOR_SIZE], "drive 0 reads 360 KB media");

        fdc.port_write(DIR_CCR, 1, 0).unwrap();
        let d1 = read_sector(&mut fdc, 0x01, 1, 0, 1);
        let o1 = FloppyGeometry::HD1440.chs_to_offset(1, 0, 1);
        assert_eq!(
            d1,
            img1[o1..o1 + SECTOR_SIZE],
            "drive 1 reads 1.44 MB media"
        );
        assert_ne!(d0, d1, "the two drives return different bytes");
    }

    #[test]
    fn read_outside_geometry_terminates_abnormally() {
        // Cylinder 60 does not exist on a 40-cylinder 360 KB disk.
        let image = patterned(FloppyGeometry::DD360, 0x22);
        let mut fdc = ready_drives([
            Some(Drive::new(FloppyGeometry::DD360, image)),
            None,
            None,
            None,
        ]);
        command(
            &mut fdc,
            &[0x06, 0x00, 60, 0x00, 0x01, 0x02, 0x01, 0x1B, 0xFF],
        );
        assert!(
            fdc.take_dma_request().is_none(),
            "an out-of-geometry read moves no data"
        );
        let res = results(&mut fdc, 7);
        assert_eq!(res[0] & 0xC0, ST0_ABNORMAL, "ST0 abnormal termination");
        assert_eq!(res[1] & ST1_NO_DATA, ST1_NO_DATA, "ST1 no-data");
    }

    /// Build a real FAT12 360 KB image with `mformat` (from the devShell's
    /// mtools), attach it as a 360 KB drive, and read sector 0 through the full
    /// command path — proving a non-1.44 MB geometry reads a genuine on-disk
    /// filesystem, boot signature and all.
    #[test]
    fn reads_a_real_fat12_360k_image() {
        use std::process::Command;

        let path = std::env::temp_dir().join(format!(
            "upd765_360k_{}_{}.img",
            std::process::id(),
            line!()
        ));
        let out = match Command::new("mformat")
            .args(["-C", "-f", "360", "-v", "TEST360", "-i"])
            .arg(&path)
            .arg("::")
            .output()
        {
            Ok(out) => out,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let message = "missing mformat — run this test in nix develop (mtools)";
                assert!(
                    std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").as_deref()
                        != Some(std::ffi::OsStr::new("1")),
                    "{message}"
                );
                eprintln!("skipping: {message}");
                return;
            }
            Err(error) => panic!("cannot run mformat: {error}"),
        };
        assert!(
            out.status.success(),
            "mformat failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let image = std::fs::read(&path).expect("read the freshly formatted image");
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            image.len(),
            FloppyGeometry::DD360.image_bytes(),
            "a 360 KB image is exactly 40*2*9*512 bytes"
        );

        let mut fdc = ready_drives([
            Some(Drive::new(FloppyGeometry::DD360, image.clone())),
            None,
            None,
            None,
        ]);

        // Sector 1 (LBA 0) is the boot sector; mformat writes the 0x55 0xAA
        // signature at the end of it.
        let boot = read_sector(&mut fdc, 0x00, 0, 0, 1);
        assert_eq!(boot.len(), SECTOR_SIZE);
        assert_eq!(
            &boot[..],
            &image[..SECTOR_SIZE],
            "sector 0 matches the image"
        );
        assert_eq!(
            (boot[510], boot[511]),
            (0x55, 0xAA),
            "the FAT12 boot sector carries the boot signature"
        );
    }

    /// A ready controller with one drive of `drive_type` holding `media` in
    /// unit 0, the first entry inserted, at CCR `rate`.
    fn typed_bus(drive_type: DriveType, media: Vec<Drive>, rate: u8) -> MockBus {
        let mut fdc = ready_drives([None, None, None, None]);
        fdc.set_media_list(0, drive_type, media, false).unwrap();
        fdc.port_write(DIR_CCR, 1, u32::from(rate)).unwrap();
        MockBus::new(Box::new(fdc))
    }

    fn bus_seek(bus: &mut MockBus, unit_head: u8, cylinder: u8) {
        bus_command(bus, &[0x0F, unit_head, cylinder]);
        bus_command(bus, &[0x08]);
        assert_eq!(bus_results(bus, 2), [0x20 | unit_head, cylinder]);
    }

    /// READ DATA of one sector: the DMA bytes, or the seven fault result bytes.
    fn try_read(bus: &mut MockBus, unit_head: u8, c: u8, h: u8, r: u8) -> Result<Vec<u8>, Vec<u8>> {
        bus_command(bus, &[0x46, unit_head, c, h, r, 2, r, 0x1B, 0xFF]);
        let Some(request) = bus.device_mut().take_dma_request() else {
            return Err(bus_results(bus, 7));
        };
        let DmaOp::ToMemory(bytes) = request.op else {
            panic!("READ DMA direction");
        };
        bus.device_mut().dma_complete(DmaOutcome::ToMemory {
            transferred: bytes.len(),
            terminal_count: true,
        });
        assert_eq!(bus_results(bus, 7)[..3], [unit_head, 0, 0]);
        Ok(bytes)
    }

    #[test]
    fn drive_types_take_their_own_and_the_lower_density_media_only() {
        let all = [
            FloppyGeometry::HD1440,
            FloppyGeometry::HD1200,
            FloppyGeometry::DD720,
            FloppyGeometry::DD360,
        ];
        for (drive_type, accepted) in [
            (
                DriveType::HD1440,
                &[FloppyGeometry::HD1440, FloppyGeometry::DD720][..],
            ),
            (
                DriveType::HD1200,
                &[FloppyGeometry::HD1200, FloppyGeometry::DD360][..],
            ),
            (DriveType::DD720, &[FloppyGeometry::DD720][..]),
            (DriveType::DD360, &[FloppyGeometry::DD360][..]),
        ] {
            assert_eq!(drive_type.media().collect::<Vec<_>>(), accepted);
            for geometry in all {
                let mut fdc = Upd765::new();
                let listed = fdc.set_media_list(
                    0,
                    drive_type,
                    vec![Drive::new(geometry, Vec::new())],
                    false,
                );
                let mut fdc = Upd765::new();
                fdc.set_media_list(1, drive_type, Vec::new(), true).unwrap();
                let appended = fdc.append_medium(1, Drive::new(geometry, Vec::new()));
                if accepted.contains(&geometry) {
                    assert_eq!(listed, Ok(()), "{drive_type:?} {geometry:?}");
                    assert_eq!(appended, Ok(0), "{drive_type:?} {geometry:?}");
                    assert_eq!(fdc.drive_type(1), Some(drive_type));
                } else {
                    assert_eq!(listed, Err(MediaChangeError::GeometryMismatch));
                    assert_eq!(appended, Err(MediaChangeError::GeometryMismatch));
                }
            }
        }
        // A 1.44M drive's catalogue mixes 1.44M and 720K media.
        let mut fdc = Upd765::new();
        fdc.set_media_list(
            0,
            DriveType::HD1440,
            vec![
                Drive::new(FloppyGeometry::DD720, Vec::new()),
                Drive::new(FloppyGeometry::HD1440, Vec::new()),
            ],
            false,
        )
        .unwrap();
        assert_eq!(fdc.drive_type(0), Some(DriveType::HD1440));
        assert_eq!(fdc.drive_type(1), None);
    }

    #[test]
    fn dd720_medium_in_a_hd1440_drive_reads_at_250_and_faults_at_500() {
        let image = patterned(FloppyGeometry::DD720, 0x3C);
        for (rate, readable) in [(0, false), (1, false), (2, true), (3, false)] {
            let mut bus = typed_bus(
                DriveType::HD1440,
                vec![Drive::new(FloppyGeometry::DD720, image.clone())],
                rate,
            );
            bus_seek(&mut bus, 4, 79);
            bus_command(&mut bus, &[0x4A, 4]);
            let id = bus_results(&mut bus, 7);
            let read = try_read(&mut bus, 4, 79, 1, 9);
            if readable {
                assert_eq!(id, [4, 0, 0, 79, 1, 1, 2]);
                let offset = FloppyGeometry::DD720.chs_to_offset(79, 1, 9);
                assert_eq!(read.unwrap(), image[offset..offset + SECTOR_SIZE]);
            } else {
                assert_eq!(id, [0x44, 1, 0, 79, 1, 1, 2], "CCR {rate}");
                assert_eq!(read.unwrap_err(), [0x44, 1, 0, 79, 1, 9, 2], "CCR {rate}");
            }
        }
        // The same drive answers its own 1.44M medium at 500 kbps only.
        let mut bus = typed_bus(
            DriveType::HD1440,
            vec![
                Drive::new(FloppyGeometry::DD720, image.clone()),
                Drive::new(FloppyGeometry::HD1440, patterned(FloppyGeometry::HD1440, 7)),
            ],
            0,
        );
        assert!(try_read(&mut bus, 0, 0, 0, 1).is_err());
        swap_fdc(&mut bus).insert(0, 1).unwrap();
        assert!(try_read(&mut bus, 0, 0, 0, 18).is_ok());
        bus.port_write(DIR_CCR, 1, 2).unwrap();
        assert!(try_read(&mut bus, 0, 0, 0, 1).is_err());
    }

    #[test]
    fn dd360_medium_in_a_hd1200_drive_reads_at_300_with_double_stepping() {
        let image = patterned(FloppyGeometry::DD360, 0xC3);
        for rate in [0, 2, 3] {
            let mut bus = typed_bus(
                DriveType::HD1200,
                vec![Drive::new(FloppyGeometry::DD360, image.clone())],
                rate,
            );
            bus_seek(&mut bus, 0, 6);
            bus_command(&mut bus, &[0x4A, 0]);
            assert_eq!(bus_results(&mut bus, 7), [0x40, 1, 0, 6, 0, 1, 2]);
            assert_eq!(
                try_read(&mut bus, 0, 3, 0, 1).unwrap_err(),
                [0x40, 1, 0, 3, 0, 1, 2]
            );
        }
        let mut bus = typed_bus(
            DriveType::HD1200,
            vec![Drive::new(FloppyGeometry::DD360, image.clone())],
            1,
        );
        for track in [0, 1, 20, 39] {
            // The BIOS seeks to twice the track; the ID field carries the track.
            bus_seek(&mut bus, 4, track * 2);
            bus_command(&mut bus, &[0x4A, 4]);
            assert_eq!(bus_results(&mut bus, 7), [4, 0, 0, track, 1, 1, 2]);
            let offset = FloppyGeometry::DD360.chs_to_offset(track, 1, 9);
            assert_eq!(
                try_read(&mut bus, 4, track, 1, 9).unwrap(),
                image[offset..offset + SECTOR_SIZE]
            );
            // An ID cylinder other than C: no data, wrong cylinder.
            assert_eq!(
                try_read(&mut bus, 4, track + 1, 1, 9).unwrap_err(),
                [0x44, 0x04, 0x10, track + 1, 1, 9, 2]
            );
        }
        // Between tracks no ID field passes under the head.
        for pcn in [1, 3, 41, 79] {
            bus_seek(&mut bus, 0, pcn);
            bus_command(&mut bus, &[0x4A, 0]);
            assert_eq!(bus_results(&mut bus, 7), [0x40, 1, 0, pcn, 0, 1, 2]);
            for c in [pcn / 2, pcn] {
                assert_eq!(
                    try_read(&mut bus, 0, c, 0, 1).unwrap_err(),
                    [0x40, 1, 0, c, 0, 1, 2]
                );
            }
        }
        // Single stepping lands on the wrong track or between tracks.
        bus_seek(&mut bus, 0, 4);
        assert_eq!(
            try_read(&mut bus, 0, 4, 0, 1).unwrap_err(),
            [0x40, 0x04, 0x10, 4, 0, 1, 2]
        );
        assert!(bus.device_mut().take_dma_request().is_none());
    }

    #[test]
    fn dd360_write_in_a_hd1200_drive_needs_the_double_stepped_track() {
        let image = patterned(FloppyGeometry::DD360, 0x11);
        let mut bus = typed_bus(
            DriveType::HD1200,
            vec![Drive::new(FloppyGeometry::DD360, image.clone())],
            1,
        );
        bus_seek(&mut bus, 0, 5);
        bus_command(&mut bus, &[0x45, 0, 2, 0, 1, 2, 1, 0x1B, 0xFF]);
        assert!(bus.device_mut().take_dma_request().is_none());
        assert_eq!(bus_results(&mut bus, 7), [0x40, 1, 0, 2, 0, 1, 2]);
        bus_seek(&mut bus, 0, 2);
        bus_command(&mut bus, &[0x45, 0, 2, 0, 1, 2, 1, 0x1B, 0xFF]);
        assert!(bus.device_mut().take_dma_request().is_none());
        assert_eq!(bus_results(&mut bus, 7), [0x40, 0x04, 0x10, 2, 0, 1, 2]);
        bus_seek(&mut bus, 0, 4);
        bus_command(&mut bus, &[0x45, 0, 2, 0, 1, 2, 1, 0x1B, 0xFF]);
        let written = complete_media_write(&mut bus, vec![0xEE; SECTOR_SIZE], true);
        assert_eq!(written, [0, 0, 0, 3, 0, 1, 2]);
        assert_eq!(media_read(&mut bus, 2, 0, 1), [0xEE; SECTOR_SIZE]);
        let offset = FloppyGeometry::DD360.chs_to_offset(2, 0, 2);
        assert_eq!(
            media_read(&mut bus, 2, 0, 2),
            image[offset..offset + SECTOR_SIZE]
        );
    }

    #[test]
    fn snapshot_restores_a_double_stepped_medium_in_a_receiving_controller() {
        let media = || {
            vec![
                Drive::new(
                    FloppyGeometry::HD1200,
                    patterned(FloppyGeometry::HD1200, 0x12),
                ),
                Drive::new(
                    FloppyGeometry::DD360,
                    patterned(FloppyGeometry::DD360, 0x36),
                ),
            ]
        };
        let mut bus = typed_bus(DriveType::HD1200, media(), 1);
        swap_fdc(&mut bus).insert(0, 1).unwrap();
        bus_seek(&mut bus, 0, 22);
        let saved = bus.device().save();
        assert_eq!(saved[0], SNAPSHOT_VERSION);
        // The receiving controller has the same wiring and catalogue but its
        // own selection, head position and rate.
        let mut restored = typed_bus(DriveType::HD1200, media(), 0);
        restored.device_mut().restore(&saved).unwrap();
        assert_eq!(swap_fdc(&mut restored).inserted_medium(0), Some(1));
        assert_eq!(swap_fdc(&mut restored).present_cylinder(0), 22);
        assert_eq!(
            swap_fdc(&mut restored).drive_type(0),
            Some(DriveType::HD1200)
        );
        bus_command(&mut restored, &[0x4A, 0]);
        assert_eq!(bus_results(&mut restored, 7), [0, 0, 0, 11, 0, 1, 2]);
        let image = patterned(FloppyGeometry::DD360, 0x36);
        let offset = FloppyGeometry::DD360.chs_to_offset(11, 0, 5);
        assert_eq!(
            try_read(&mut restored, 0, 11, 0, 5).unwrap(),
            image[offset..offset + SECTOR_SIZE]
        );
    }

    #[test]
    fn sense_drive_status_follows_the_medium_in_a_mixed_density_drive() {
        let mut bus = typed_bus(
            DriveType::HD1200,
            vec![Drive::new_read_only(FloppyGeometry::DD360, Vec::new())],
            1,
        );
        bus_command(&mut bus, &[0x04, 0]);
        assert_eq!(
            bus_results(&mut bus, 1),
            [ST3_READY | ST3_WRITE_PROTECTED | ST3_TRACK_ZERO | ST3_TWO_SIDE]
        );
        // TRACK 0 follows the physical cylinder, not the medium track.
        bus_seek(&mut bus, 0, 1);
        bus_command(&mut bus, &[0x04, 0]);
        assert_eq!(
            bus_results(&mut bus, 1),
            [ST3_READY | ST3_WRITE_PROTECTED | ST3_TWO_SIDE]
        );
        swap_fdc(&mut bus).eject(0).unwrap();
        bus_command(&mut bus, &[0x04, 0]);
        assert_eq!(bus_results(&mut bus, 1), [ST3_READY]);
        assert_eq!(bus.port_read(DIR_CCR, 1).unwrap(), 0x80);
    }
}
