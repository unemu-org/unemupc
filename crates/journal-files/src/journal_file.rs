// SPDX-License-Identifier: AGPL-3.0-or-later
//! The journal's own file code: a version-1 journal file on the host as one
//! record of the contract (`unemu_core_api::Record`), core-contract.md §12.5
//! (move 1) and §14.4; persistent-media.md § Persist at command completion,
//! § Open and tail recovery and § Locking and retention.
//!
//! The journal's logic ([`crate::JournalMedia`]) reads, appends, cuts,
//! synchronizes and replaces a record and never names a path. This module is
//! the journal adapter over the shared file engine:
//!
//! - **Exclusive use** is the stable sidecar lock `FILE.lock` beside the
//!   journal (`flock`-style, never waits, never removed), the same lock the
//!   offline tools take, so a running machine and `media verify`, `rollback`
//!   or `commit` still exclude each other. Offline tools hold the contract's
//!   `PathLock`, which uses that same canonical `FILE.lock`.
//! - **Creation** writes the header to `FILE.create-N.tmp`, synchronizes it and
//!   links it into place without replacing an existing file, so a journal with
//!   half a header never exists. After it, and at every open of an existing
//!   journal, the canonical path is checked again, leftovers of earlier
//!   creations and retentions are removed under the lock and the directory is
//!   synchronized.
//! - **Appends and cuts** go to the open journal file; `sync` is
//!   `File::sync_all` (`F_FULLFSYNC` on Apple targets).
//! - **A replacement** (retention) is written to `FILE.retain-N.tmp` with the
//!   journal's exact permission bits, synchronized, renamed over the journal
//!   and made durable with a directory sync.
//!
//! The shared file engine rolls a failed append back to its previous length.
//! If rollback fails it reports StorageUncertain; either failure is latched by
//! the journal, which stops writing. Recovery still discards crash tails.

use crate::offline::ToolIo;
use crate::{Header, JournalMedia};
use std::cell::RefCell;
use std::fs::OpenOptions;
use std::fs::{File, Metadata, Permissions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use unemu_core_api as contract;
use unemu_core_api::{ErrorKind, Record as _};
use unemu_core_files::{FileIo, Filesystem as HostOps};
#[cfg(test)]
#[path = "file_tests.rs"]
pub(crate) mod tests;
#[cfg(test)]
pub(crate) use tests::{barrier, Filesystem, HostFilesystem, JournalIo};

pub(crate) use unemu_core_files::FileRecord;

/// A version-1 journal file on the host, held exclusively through its
/// `FILE.lock`, as a record of the contract.
///
/// This is what a front end's storage hands to the PC core for a disk or CMOS
/// journal (core-contract.md §12.9, corrected 2026-11-19: a table from record
/// name to journal file). [`crate::JournalMedia::open_record`] mounts it.
/// The path constructors of `JournalMedia` open the same file code directly.
///
/// A journal file that does not exist opens empty; the first replacement's
/// commit creates it through a temporary header file, never in place, and an
/// append before that fails with `ErrorKind::Usage`. That commit publishes
/// only the header the file was opened for. A file that exists with no bytes
/// is refused at open, as the path constructors refuse it: the publication
/// protocol never leaves one.
pub struct JournalFile(FileRecord);

impl JournalFile {
    /// Open the journal at `path` for a session: resolve it without following
    /// a dangling link or accepting a reserved temporary name, take
    /// `path.lock`, open the file and, when it exists, check its canonical
    /// path again, remove leftovers of interrupted creations and retentions,
    /// and synchronize the directory.
    ///
    /// `base` is the identity of the disk the journal belongs to (its length
    /// and SHA-256; the anchor is ignored): the cleanup removes a creation
    /// leftover only when its bytes are a prefix of that journal's header.
    /// `base_metadata`, when the base is a host file, keeps that file from
    /// being removed even when it is linked under a reserved name.
    ///
    /// Fails with `ErrorKind::Usage` when another session or an offline tool
    /// holds the lock, and with `ErrorKind::Storage` when the host failed.
    /// The message names the step, as the journal's own failures do.
    pub fn open(
        path: &Path,
        base: &Header,
        base_metadata: Option<&std::fs::Metadata>,
    ) -> Result<Self, contract::Error> {
        let fail = |step: &str, error: io::Error| {
            let kind = if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::InvalidInput
            ) {
                ErrorKind::Usage
            } else {
                ErrorKind::Storage
            };
            contract::Error::new(kind, format!("{step}: {error}"))
        };
        check_journal_path(path)?;
        let path = resolve_journal_path(path).map_err(|e| fail("resolve journal path", e))?;
        let header = Header {
            anchor_generation: 0,
            ..base.clone()
        }
        .encode()
        .map_err(|error| contract::Error::new(ErrorKind::Usage, error.to_string()))?;
        let record = FileRecord::open_with(
            Rc::new(JournalFilesystem),
            &path,
            None,
            &header,
            base_metadata,
        )
        .map_err(|error| fail(journal_stage(error.stage), error.source))?;
        // The publication protocol never leaves a journal without its
        // header; the path constructors refuse such a file, and so does this.
        if record.exists() && record.size()? == 0 {
            return Err(contract::Error::new(
                ErrorKind::Storage,
                "journal file has no header: it was not made by the journal",
            ));
        }
        Ok(Self(record))
    }
}

impl contract::Record for JournalFile {
    fn size(&self) -> Result<u64, contract::Error> {
        self.0.size().map_err(journal_error)
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), contract::Error> {
        self.0.read_at(offset, buf).map_err(journal_error)
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), contract::Error> {
        self.0.append(bytes).map_err(journal_error)
    }
    fn truncate(&mut self, size: u64) -> Result<(), contract::Error> {
        self.0.truncate(size).map_err(journal_error)
    }
    fn sync(&mut self) -> Result<(), contract::Error> {
        self.0.sync().map_err(journal_error)
    }
    fn replace(&mut self) -> Result<Box<dyn contract::Replacement>, contract::Error> {
        journal_replacement(self.0.replace())
    }
}

/// Refuse a reserved temporary name, then resolve the journal's canonical
/// path (a missing journal's resolved parent forms it).
pub(crate) fn resolve_journal_path(path: &Path) -> io::Result<PathBuf> {
    refuse_creation_temp_name(path)?;
    let canonical = canonical_journal_path(path)?;
    refuse_creation_temp_name(&canonical)?;
    Ok(canonical)
}

pub(crate) fn refuse_creation_temp_name(path: &Path) -> io::Result<()> {
    if JournalMedia::is_creation_temp_name(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "journal path {} uses a reserved creation-temporary name or replacement-temporary name",
                path.display()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn canonical_journal_path(path: &Path) -> io::Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "journal path is a dangling symlink",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            // This is the expected canonical name for post-open verification,
            // not an additional locking mechanism for parent-directory aliases.
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let name = path.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "journal path must have a filename",
                )
            })?;
            Ok(parent.canonicalize()?.join(name))
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
pub(crate) fn platform_sync(file: &File) -> io::Result<()> {
    // std uses F_FULLFSYNC on Apple targets with an Interrupted retry. Calling
    // fcntl again would duplicate the device-cache flush for every command.
    file.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn platform_sync(_file: &File) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "journal durability is supported only on Unix",
    ))
}

pub(crate) fn journal_lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

pub(crate) fn inspection_lock(path: &Path) -> io::Result<Option<unemu_core_files::PathLock>> {
    let lock = unemu_core_files::PathLock::inspection(path).map_err(lock_error)?;
    Ok(lock.is_locked().then_some(lock))
}

fn lock_error(error: unemu_core_files::FileError) -> io::Error {
    if error.source.kind() == io::ErrorKind::WouldBlock {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            "journal is locked by another session or tool",
        )
    } else {
        error.source
    }
}

pub(crate) fn exclusive_lock(path: &Path) -> io::Result<unemu_core_files::PathLock> {
    unemu_core_files::PathLock::exclusive(path).map_err(lock_error)
}

#[cfg(test)]
pub(crate) fn lock_journal(path: &Path, shared: bool) -> io::Result<File> {
    let name = journal_lock_path(path);
    let file = if shared {
        match File::open(&name) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&name)
                {
                    Ok(file) => file,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        File::open(&name)?
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    } else {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(name)?
    };
    let result = if shared {
        file.try_lock_shared()
    } else {
        file.try_lock()
    };
    result.map_err(|e| match e {
        std::fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "journal is locked by another session or tool",
        ),
        std::fs::TryLockError::Error(error) => error,
    })?;
    Ok(file)
}

pub(crate) fn journal_stage(stage: unemu_core_files::FileStage) -> &'static str {
    use unemu_core_files::FileStage;
    match stage {
        FileStage::ResolvePath => "resolve journal path",
        FileStage::Lock => "lock",
        FileStage::Open => "open",
        FileStage::Metadata => "metadata",
        FileStage::VerifyPath => "verify journal path",
        FileStage::Cleanup => "remove creation temporaries",
        FileStage::DirectorySync => "directory sync",
        _ => "open journal",
    }
}

fn creation_stage_name(stage: &'static str) -> &'static str {
    match stage {
        "resolve path" => "resolve journal path",
        "create temporary" => "create temporary header",
        "initial read" => "header read",
        "initial sync" => "header sync",
        "publish initial" => "publish header",
        "unlink initial" => "unlink creation temporary",
        "verify path" => "verify journal path",
        "cleanup" => "remove creation temporaries",
        other => other,
    }
}

/// Commit errors carry their file stage in the contract message rather than
/// a FileError. Recover it before journal_error removes that prefix.
pub(crate) fn creation_stage(error: &contract::Error, fallback: &'static str) -> &'static str {
    for stage in [
        "create temporary",
        "initial read",
        "initial sync",
        "publish initial",
        "unlink initial",
        "verify path",
        "cleanup",
        "directory sync",
    ] {
        if error
            .message
            .strip_prefix(stage)
            .is_some_and(|message| message.starts_with(": "))
        {
            return creation_stage_name(stage);
        }
    }
    fallback
}

/// Retention injection owns its state because a temporary inode outlives
/// the call which creates it.
pub(crate) struct ToolFilesystem {
    io: Rc<RefCell<dyn ToolIo>>,
    path: PathBuf,
    pending: RefCell<Option<(PathBuf, File)>>,
}
impl ToolFilesystem {
    pub(crate) fn new(io: Rc<RefCell<dyn ToolIo>>, path: PathBuf) -> Self {
        Self {
            io,
            path,
            pending: RefCell::new(None),
        }
    }
}
impl Drop for ToolFilesystem {
    fn drop(&mut self) {
        if let Some((path, _)) = self.pending.get_mut().take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl HostOps for ToolFilesystem {
    fn create_new(
        &self,
        path: &Path,
        permissions: Option<&Permissions>,
    ) -> io::Result<Box<dyn FileIo>> {
        if self.pending.borrow().is_none() {
            let made = self
                .io
                .borrow_mut()
                .create_temporary(&self.path, "retain", permissions)?;
            *self.pending.borrow_mut() = Some((made.1, made.0));
        }
        if self
            .pending
            .borrow()
            .as_ref()
            .is_some_and(|(chosen, _)| chosen != path)
        {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let (_, file) = self
            .pending
            .borrow_mut()
            .take()
            .ok_or_else(|| io::Error::other("no temporary"))?;
        Ok(Box::new(ToolFile {
            file,
            io: self.io.clone(),
        }))
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.io.borrow_mut().publish_temporary(from, to, true)
    }
    fn sync_directory(&self, path: &Path) -> io::Result<()> {
        self.io.borrow_mut().sync(
            &File::open(crate::offline::parent(path))?,
            "publication directory sync",
        )
    }
}
struct ToolFile {
    file: File,
    io: Rc<RefCell<dyn ToolIo>>,
}
impl Read for ToolFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.read(bytes)
    }
}
impl Seek for ToolFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.file.seek(from)
    }
}
impl Write for ToolFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.io
            .borrow_mut()
            .write_temporary(&mut self.file, bytes)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl FileIo for ToolFile {
    fn metadata(&self) -> io::Result<Metadata> {
        self.file.metadata()
    }
    fn set_len(&mut self, size: u64) -> io::Result<()> {
        self.file.set_len(size)
    }
    fn sync_all(&mut self) -> io::Result<()> {
        self.io.borrow_mut().sync(&self.file, "temporary sync")
    }
    fn set_permissions(&mut self, permissions: Permissions) -> io::Result<()> {
        self.io
            .borrow_mut()
            .set_permissions(&self.file, permissions)
    }
}

/// The journal already names the failing operation in its diagnostic. Keep
/// that established text instead of repeating the generic engine's stage.
pub(crate) fn journal_error(mut error: contract::Error) -> contract::Error {
    if error.message == "initial bytes differ" {
        error.message = "creation header differs from opened base".into();
        return error;
    }
    for prefix in [
        "cannot open the record: ",
        "cannot read the record: ",
        "cannot append to the record: ",
        "cannot append to the record nor undo it: ",
        "cannot cut the record: ",
        "cannot make the record durable: ",
        "cannot make the record's name durable: ",
        "cannot read the replacement: ",
        "cannot append to the replacement: ",
        "cannot append to the replacement nor undo it: ",
        "replacement metadata: ",
        "replacement permissions: ",
        "create temporary: ",
        "initial read: ",
        "initial sync: ",
        "replacement sync: ",
        "publish initial: ",
        "replacement rename: ",
        "unlink initial: ",
        "verify path: ",
        "cleanup: ",
        "directory sync: ",
    ] {
        if let Some(message) = error.message.strip_prefix(prefix) {
            error.message = message.to_owned();
            break;
        }
    }
    error
}

pub(crate) fn journal_replacement<'a>(
    result: Result<Box<dyn contract::Replacement + 'a>, contract::Error>,
) -> Result<Box<dyn contract::Replacement + 'a>, contract::Error> {
    result
        .map(|inner| Box::new(JournalReplacement(inner)) as Box<dyn contract::Replacement>)
        .map_err(journal_error)
}
struct JournalReplacement<'a>(Box<dyn contract::Replacement + 'a>);
impl contract::Replacement for JournalReplacement<'_> {
    fn size(&self) -> Result<u64, contract::Error> {
        self.0.size().map_err(journal_error)
    }
    fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> Result<(), contract::Error> {
        self.0.read_at(offset, bytes).map_err(journal_error)
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), contract::Error> {
        self.0.append(bytes).map_err(journal_error)
    }
    fn commit(self: Box<Self>) -> Result<(), contract::Error> {
        self.0.commit().map_err(journal_error)
    }
}

/// Reject the shared engine's sidecar namespace before acquiring any lock.
/// Both path mounts and offline tools use this same Usage diagnostic.
pub fn check_journal_path(path: &Path) -> Result<(), contract::Error> {
    fn reserved(path: &Path) -> bool {
        path.file_name().is_some_and(|name| {
            let name = name.as_encoded_bytes().to_ascii_lowercase();
            [b".create-".as_slice(), b".retain-"]
                .iter()
                .any(|marker| name.windows(marker.len()).any(|part| part == *marker))
                || name.ends_with(b".lock")
                || name.ends_with(b"_lock")
                || name.strip_suffix(b".tmp").is_some_and(|stem| {
                    stem.contains(&b'_')
                        && stem
                            .rsplit(|b| *b == b'_')
                            .next()
                            .is_some_and(|n| !n.is_empty() && n.iter().all(u8::is_ascii_digit))
                })
        })
    }
    if reserved(path)
        || canonical_journal_path(path)
            .ok()
            .is_some_and(|p| reserved(&p))
    {
        return Err(contract::Error::new(
            ErrorKind::Usage,
            "reserved sidecar path",
        ));
    }
    Ok(())
}

/// Preserve the journal's original creation mode (0666 filtered by umask).
/// Generic records default to 0600; existing journal permission semantics must
/// also apply to the temporary inode before its header is published.
pub(crate) struct JournalFilesystem;
impl HostOps for JournalFilesystem {
    fn lock(&self, path: &Path, shared: bool) -> io::Result<Box<dyn std::any::Any>> {
        unemu_core_files::HostFilesystem::new()
            .lock(path, shared)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::WouldBlock {
                    io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "journal is locked by another session or tool",
                    )
                } else {
                    error
                }
            })
    }

    fn open(&self, path: &Path, write: bool) -> io::Result<Box<dyn FileIo>> {
        unemu_core_files::HostFilesystem::new()
            .open(path, write)
            .map(journal_io)
    }

    fn sync_directory(&self, path: &Path) -> io::Result<()> {
        platform_sync(&File::open(crate::offline::parent(path))?)
    }

    fn create_new(
        &self,
        path: &Path,
        permissions: Option<&Permissions>,
    ) -> io::Result<Box<dyn FileIo>> {
        let file: Box<dyn FileIo> = match permissions {
            Some(permissions) => {
                unemu_core_files::HostFilesystem::new().create_new(path, Some(permissions))?
            }
            None => Box::new(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(path)?,
            ),
        };
        Ok(journal_io(file))
    }
}

// The generic engine supports more hosts; the journal keeps its established
// refusal where its durability barriers are unsupported.
fn journal_io(file: Box<dyn FileIo>) -> Box<dyn FileIo> {
    #[cfg(unix)]
    {
        file
    }
    #[cfg(not(unix))]
    {
        Box::new(unsupported_barrier::File(file))
    }
}

#[cfg(not(unix))]
mod unsupported_barrier {
    //! Preserve the journal's Unix-only durability policy over a generic inode.
    use super::*;
    pub(super) struct File(pub(super) Box<dyn FileIo>);
    impl Read for File {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.0.read(bytes)
        }
    }
    impl Write for File {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl Seek for File {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.0.seek(position)
        }
    }
    impl FileIo for File {
        fn metadata(&self) -> io::Result<Metadata> {
            self.0.metadata()
        }
        fn set_len(&mut self, len: u64) -> io::Result<()> {
            self.0.set_len(len)
        }
        fn set_permissions(&mut self, permissions: Permissions) -> io::Result<()> {
            self.0.set_permissions(permissions)
        }
        fn sync_all(&mut self) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "journal durability is supported only on Unix",
            ))
        }
    }
}
