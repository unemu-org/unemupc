// SPDX-License-Identifier: AGPL-3.0-or-later
//! Host journal ownership for NEC µPD765 media (WRITE DATA, N=2), ATA-2 §7
//! WRITE SECTORS / WRITE MULTIPLE (512-byte sectors), and
//! Motorola MC146818 general RAM (0Eh–3Fh, excluding the transient AT shutdown
//! byte 0Fh). See `backlog/docs/design/persistent-media.md`, §Integration and
//! §Host errors and determinism. Only frontend boundaries observe host errors.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use device::{SectorMedia, SectorWrite};
use journal::{
    SectorSource, SinkFailure, SinkStatus, SparseOverlay, VerificationReport, VerifiedBase,
};

use crate::mount::{DiskBase, Mount, MountError};
use crate::{ConfigError, MachineConfig, WriteMode};

/// What a removable medium needs to open its journal again at each insert:
/// the machine's mount, the journal's key in it and the medium's base.
struct Remount {
    mount: Rc<dyn Mount>,
    path: PathBuf,
    base: DiskBase,
}

enum Session {
    Active(Box<dyn journal::JournalSession>),
    Finished(SparseOverlay, SinkStatus),
    Empty,
}

/// One medium's persistence failure, identifying a drive or CMOS RAM.
#[derive(Debug, thiserror::Error)]
#[error("{} persistence: {source}", if *.nvram { "NVRAM".into() } else { format!("drive {}", .drive_index) })]
pub struct PersistenceError {
    /// Drive configuration index, or RTC device configuration index for NVRAM.
    pub drive_index: usize,
    /// This failure belongs to CMOS rather than a drive.
    pub nvram: bool,
    #[source]
    pub source: SinkFailure,
}

/// Frontend ownership of a journal session. Call `finish` only after successful
/// execution; dropping this handle never writes a clean generation marker.
#[must_use = "journal sessions must be checked and explicitly finished"]
pub struct PersistenceHandle {
    /// Drive configuration index, or RTC device configuration index for NVRAM.
    pub drive_index: usize,
    /// Validated closed-generation retention setting (including the anchor).
    pub keep_generations: u32,
    pub(crate) ata_index: Option<usize>,
    session: Rc<RefCell<Session>>,
    status: SinkStatus,
    maintenance: Option<journal::MaintenanceFailure>,
    report: VerificationReport,
    nvram: Option<devices::cmos_rtc::NvramStore>,
    remount: Option<Remount>,
    ejected: bool,
    pub floppy: Option<(crate::DeviceId, u8, usize)>,
}

impl PersistenceHandle {
    /// Open the journal that `mount` keeps under `path` over `base`, and
    /// replay it. The mount supplies the session and its publication policy.
    pub fn mount_journal(
        drive_index: usize,
        mount: &dyn Mount,
        path: &Path,
        base: &DiskBase,
        geometry_length: u64,
        keep_generations: u32,
    ) -> Result<(Self, Box<dyn SectorMedia>), crate::AssemblyError> {
        let status = SinkStatus::default();
        let store = match mount.journal(path) {
            Ok(store) => store,
            Err(MountError::Config(error)) => return Err(error.into()),
            Err(MountError::Host(error)) => {
                return Err(crate::AssemblyError::Journal {
                    drive_index,
                    source: journal::OpenError::Host(SinkFailure {
                        operation: "open journal record",
                        message: error.to_string(),
                        last_confirmed_sequence: 0,
                    }),
                })
            }
        };
        let mut journal = store
            .open(base, geometry_length, status.clone())
            .map_err(|source| crate::AssemblyError::Journal {
                drive_index,
                source,
            })?;
        journal.set_retention(keep_generations);
        let report = journal.mount_report().clone();
        let session = Rc::new(RefCell::new(Session::Active(journal)));
        let adapter = Box::new(JournalAdapter(session.clone()));
        Ok((
            Self {
                drive_index,
                keep_generations,
                ata_index: None,
                session,
                status,
                maintenance: None,
                report,
                nvram: None,
                remount: None,
                ejected: false,
                floppy: None,
            },
            adapter,
        ))
    }

    pub fn prepare_mounted(
        drive_index: usize,
        mount: Rc<dyn Mount>,
        path: &Path,
        base: DiskBase,
        keep_generations: u32,
    ) -> (Self, Box<dyn SectorMedia>) {
        let session = Rc::new(RefCell::new(Session::Empty));
        let status = SinkStatus::default();
        let adapter = Box::new(JournalAdapter(session.clone()));
        let handle = Self {
            drive_index,
            keep_generations,
            ata_index: None,
            session,
            status,
            maintenance: None,
            report: journal::scan(&[], &[], 0).report,
            nvram: None,
            remount: Some(Remount {
                mount,
                path: path.into(),
                base,
            }),
            ejected: true,
            floppy: None,
        };
        (handle, adapter)
    }

    pub fn insert(&mut self) -> Result<(), crate::AssemblyError> {
        let Some(remount) = &self.remount else {
            return Ok(());
        };
        self.check().map_err(|e| {
            crate::AssemblyError::Config(ConfigError::Journal {
                drive_index: self.drive_index,
                message: e.to_string(),
            })
        })?;
        let (mounted, _) = Self::mount_journal(
            self.drive_index,
            remount.mount.as_ref(),
            &remount.path,
            &remount.base,
            remount.base.len(),
            self.keep_generations,
        )?;
        let state = std::mem::replace(&mut *mounted.session.borrow_mut(), Session::Empty);
        *self.session.borrow_mut() = state;
        self.maintenance = self.maintenance_failure();
        self.status = mounted.status;
        self.report = mounted.report;
        self.ejected = false;
        Ok(())
    }

    pub fn eject(
        &mut self,
        machine: &mut machine::Machine<cpu::X86JitCpu>,
    ) -> Result<(), PersistenceError> {
        self.finish(machine)?;
        self.ejected = true;
        Ok(())
    }

    /// Reopen a removable session consumed by a failed closing attempt.
    /// The caller must first classify the host error as recoverable; uncertain
    /// storage errors require terminating the running machine instead.
    pub fn restore_failed_eject(&mut self) -> Result<(), crate::AssemblyError> {
        if self.remount.is_some()
            && !self.ejected
            && matches!(*self.session.borrow(), Session::Finished(..))
        {
            self.status = SinkStatus::default();
            self.insert()?;
        }
        Ok(())
    }

    pub(crate) fn cancel_insert(&mut self) {
        *self.session.borrow_mut() = Session::Empty;
        self.ejected = true;
    }

    pub(crate) fn mount_nvram(
        device_index: usize,
        mount: &dyn Mount,
        path: &Path,
        base: &[u8; 64],
        keep_generations: u32,
        origin: Option<&Rc<dyn std::any::Any>>,
        rtc: &mut devices::CmosRtc,
    ) -> Result<Self, crate::AssemblyError> {
        let (mut handle, _) = VerifiedBase::from_bytes(base)
            .map_err(|source| crate::AssemblyError::Journal {
                drive_index: device_index,
                source: source.into(),
            })
            .and_then(|base| {
                Self::mount_journal(
                    device_index,
                    mount,
                    path,
                    &DiskBase::from(base).with_origin(origin.cloned()),
                    64,
                    keep_generations,
                )
            })
            .map_err(|error| match error {
                crate::AssemblyError::Journal { source, .. } => {
                    crate::AssemblyError::NvramJournal { source }
                }
                other => other,
            })?;
        let mut bytes = [0; 64];
        if let Session::Active(journal) = &*handle.session.borrow() {
            bytes[14..].copy_from_slice(&journal.read_sector(0)[14..64]);
        }
        // Clock, alarm and control registers belong to the RTC model on each run.
        *rtc = std::mem::take(rtc).with_general_nvram(std::array::from_fn(|i| bytes[14 + i]));
        handle.nvram = Some(rtc.nvram_store());
        Ok(handle)
    }

    /// Human-readable owner used by frontend diagnostics.
    pub fn owner(&self) -> String {
        if self.nvram.is_some() {
            "NVRAM".into()
        } else {
            format!("drive {}", self.drive_index)
        }
    }

    /// The IBM PC AT Technical Reference, CMOS shutdown status byte, assigns
    /// 0Fh to the live reset protocol; it must not become persistent setup data.
    fn persist_nvram(&self) -> Result<(), PersistenceError> {
        let Some(store) = &self.nvram else {
            return Ok(());
        };
        let bytes = store.snapshot();
        if let Session::Active(journal) = &mut *self.session.borrow_mut() {
            if journal.read_sector(0)[14] != bytes[0]
                || journal.read_sector(0)[16..64] != bytes[2..]
            {
                let mut block = journal.read_sector(0);
                block[14] = bytes[0];
                block[16..64].copy_from_slice(&bytes[2..]);
                journal.write_batch(&[(0, block)]);
            }
        }
        self.check()
    }

    /// Inject host write failures while retaining real in-memory media updates.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_output(&mut self) {
        let mut session = self.session.borrow_mut();
        let Session::Active(journal) = std::mem::replace(&mut *session, Session::Empty) else {
            panic!("active session required")
        };
        *session = Session::Active(journal.fail_output());
    }

    /// Inject retention publication failures without altering guest commands.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_retention(&mut self, operation: &'static str) {
        let mut session = self.session.borrow_mut();
        let Session::Active(journal) = std::mem::replace(&mut *session, Session::Empty) else {
            panic!("active session required")
        };
        *session = Session::Active(journal.fail_retention(operation));
    }

    /// First latched host error; the guest never observes this channel.
    pub fn status(&self) -> Option<SinkFailure> {
        match &*self.session.borrow() {
            Session::Active(journal) => journal.check_base(),
            Session::Finished(overlay, status) => {
                if let Err(error) = overlay.check_unchanged() {
                    status.latch_host_error("base changed", error.to_string());
                }
            }
            Session::Empty => {}
        }
        self.status.failure()
    }

    /// First maintenance diagnostic, retained across removable remounts and
    /// separate from the guest-command error latch.
    pub fn maintenance_failure(&self) -> Option<journal::MaintenanceFailure> {
        self.maintenance
            .clone()
            .or_else(|| self.status.maintenance_failure())
    }

    /// Recovery evidence from before mount repaired or sealed the tip.
    pub fn mount_report(&self) -> &VerificationReport {
        &self.report
    }

    /// Report the first latched host error without changing guest-visible state.
    pub fn check(&self) -> Result<(), PersistenceError> {
        self.status().map_or(Ok(()), |source| {
            Err(PersistenceError {
                drive_index: self.drive_index,
                nvram: self.nvram.is_some(),
                source,
            })
        })
    }

    pub fn check_running(&self) -> Result<(), PersistenceError> {
        self.check()?;
        if !self.ejected && matches!(*self.session.borrow(), Session::Finished(..)) {
            return Err(PersistenceError {
                drive_index: self.drive_index,
                nvram: self.nvram.is_some(),
                source: SinkFailure {
                    operation: "run",
                    message: "session already finished".into(),
                    last_confirmed_sequence: 0,
                },
            });
        }
        Ok(())
    }

    /// Append changed CMOS RAM before the guest CPU reset, without closing its
    /// session generation. Host-paced run boundaries only check error latches.
    pub fn before_cpu_reset(&self) -> Result<(), PersistenceError> {
        self.check_running()?;
        self.persist_nvram()
    }

    /// Flush this ATA controller's completed sectors before closing a generation.
    /// Supply the owning assembly's machine; ordinary run stops must not flush.
    /// For CMOS, only validate the session: its generation closes at finish.
    pub fn checkpoint(
        &mut self,
        machine: &mut machine::Machine<cpu::X86JitCpu>,
    ) -> Result<(), PersistenceError> {
        flush_ata(machine, std::slice::from_ref(self));
        self.check()?;
        if self.nvram.is_some() && matches!(*self.session.borrow(), Session::Active(_)) {
            return Ok(());
        }
        match &mut *self.session.borrow_mut() {
            Session::Active(journal) => journal.checkpoint().map_err(|source| PersistenceError {
                drive_index: self.drive_index,
                nvram: self.nvram.is_some(),
                source,
            }),
            Session::Finished(..) | Session::Empty => Err(PersistenceError {
                drive_index: self.drive_index,
                nvram: self.nvram.is_some(),
                source: SinkFailure {
                    operation: "checkpoint",
                    message: "session already finished".into(),
                    last_confirmed_sequence: 0,
                },
            }),
        }
    }

    /// Flush this ATA controller's completed sectors and explicitly finish once.
    /// Supply the owning assembly's machine, after successful execution only.
    /// Readable bytes remain available for capture; the sink releases its lock
    /// only after attempting the closing marker. Partial sectors are excluded.
    pub fn finish(
        &mut self,
        machine: &mut machine::Machine<cpu::X86JitCpu>,
    ) -> Result<(), PersistenceError> {
        flush_ata(machine, std::slice::from_ref(self));
        self.check()?;
        self.ejected = false;
        self.persist_nvram()?;
        let mut session = self.session.borrow_mut();
        if !matches!(*session, Session::Active(_)) {
            return Ok(());
        }
        let old = std::mem::replace(&mut *session, Session::Empty);
        if let Session::Active(journal) = old {
            let (overlay, result) = journal.finish_readable();
            *session = Session::Finished(overlay, self.status.clone());
            result.map_err(|source| PersistenceError {
                drive_index: self.drive_index,
                nvram: self.nvram.is_some(),
                source,
            })?;
        }
        Ok(())
    }
}

struct JournalAdapter(Rc<RefCell<Session>>);

impl SectorMedia for JournalAdapter {
    fn sector_count(&self) -> u64 {
        match &*self.0.borrow() {
            Session::Active(journal) => journal.byte_len() / 512,
            Session::Finished(image, _) => image.byte_len() / 512,
            Session::Empty => 0,
        }
    }

    fn read_sector(&self, lba: u64) -> [u8; 512] {
        match &*self.0.borrow() {
            Session::Active(journal) => journal.read_sector(lba),
            Session::Finished(image, status) => image.read_sector(lba).unwrap_or_else(|error| {
                status.latch_host_error("base read", error.to_string());
                [0; 512]
            }),
            Session::Empty => [0; 512],
        }
    }

    fn write_batch(&mut self, sectors: &[SectorWrite]) {
        match &mut *self.0.borrow_mut() {
            Session::Active(journal) => {
                journal.write_batch(&sectors.iter().map(|s| (s.lba, s.bytes)).collect::<Vec<_>>())
            }
            Session::Finished(..) | Session::Empty => {}
        }
    }

    fn write_protected(&self) -> bool {
        false
    }
}

/// ATA-2 §8.4 commits sectors after their final word. An explicit persistence
/// boundary submits that prefix; ordinary host pacing must not split commands.
pub(crate) fn flush_ata(
    machine: &mut machine::Machine<cpu::X86JitCpu>,
    handles: &[PersistenceHandle],
) {
    for index in handles.iter().filter_map(|handle| handle.ata_index) {
        let ata = machine
            .bus_mut()
            .device_mut(index)
            .and_then(|device| device.as_any_mut())
            .and_then(|device| device.downcast_mut::<devices::Ata>())
            .expect("journal ATA controller must exist and expose Ata");
        ata.flush_pending();
    }
}

fn path_error(config: &MachineConfig, index: usize, message: String) -> ConfigError {
    if index >= config.drives.len() {
        ConfigError::Nvram { message }
    } else {
        ConfigError::Journal {
            drive_index: index,
            message,
        }
    }
}

/// NVRAM journals keep at least one closed generation, whatever holds them.
pub(crate) fn check_nvram_retention(config: &MachineConfig) -> Result<(), ConfigError> {
    let zero = config.devices.iter().any(|device| {
        matches!(
            &device.kind,
            crate::DeviceKind::Rtc(crate::CmosCfg {
                nvram: crate::NvramSource::Journal {
                    keep_generations: 0,
                    ..
                },
                ..
            })
        )
    });
    if zero {
        return Err(ConfigError::Nvram {
            message: "keep_generations must be positive".into(),
        });
    }
    Ok(())
}

/// No two journal outputs (fixed drives, catalogue entries, NVRAM journals)
/// name the same key, whatever the mount: two media would share one journal.
/// `config` has one drive per catalogue entry.
pub(crate) fn check_distinct_journals(config: &MachineConfig) -> Result<(), ConfigError> {
    let drives = config.drives.iter().map(|drive| match &drive.write {
        WriteMode::Journal { path, .. } => Some(path),
        _ => None,
    });
    let nvram = config.devices.iter().map(|device| match &device.kind {
        crate::DeviceKind::Rtc(crate::CmosCfg {
            nvram: crate::NvramSource::Journal { journal, .. },
            ..
        }) => Some(journal),
        _ => None,
    });
    let mut seen = std::collections::BTreeSet::new();
    for (index, path) in drives.chain(nvram).enumerate() {
        if let Some(path) = path {
            if !seen.insert(path) {
                return Err(path_error(
                    config,
                    index,
                    format!(
                        "journal {} is also another medium's journal",
                        path.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}
