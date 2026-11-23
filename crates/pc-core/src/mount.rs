// SPDX-License-Identifier: AGPL-3.0-or-later
//! Launch keys backed by verified content and exclusive storage records.
//! No key is opened as a host path. The assembler keeps this mount for later
//! catalogue inserts; media appended by path use its separate file mount.
//! See `backlog/docs/design/core-contract.md` §14.2.3 and §14.4.

use assembly::{
    ConfigError, DeviceKind, DiskBase, ImageBytes, JournalStore, MachineConfig, Mount, MountError,
    NvramSource, WriteMode,
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
};
use unemu_core_api::{
    Content, ContentReader, ContentRef, ContentSource, Error, ErrorKind, Record, Replacement,
    Storage,
};

type Identity = ([u8; 32], u64);
type Failure = Rc<RefCell<Option<Error>>>;

#[derive(Clone)]
pub(crate) struct Failures {
    content: Failure,
    records: BTreeMap<PathBuf, Failure>,
    owners: BTreeMap<(bool, usize), Failure>,
}
impl Failures {
    fn new(config: &MachineConfig, content: Failure) -> Self {
        let mut records = BTreeMap::new();
        let mut owners = BTreeMap::new();
        for (index, device) in config.devices.iter().enumerate() {
            if let DeviceKind::Rtc(rtc) = &device.kind {
                if let NvramSource::Journal { journal, .. } = &rtc.nvram {
                    let failure = Failure::default();
                    records.insert(journal.clone(), failure.clone());
                    owners.insert((true, index), failure);
                }
            }
        }
        for (index, drive) in config.drives.iter().enumerate() {
            let failure = Failure::default();
            owners.insert((false, index), failure.clone());
            let writes = std::iter::once(&drive.write).chain(
                drive
                    .media
                    .iter()
                    .flat_map(|list| list.entries.iter().map(|m| &m.write)),
            );
            for write in writes {
                if let WriteMode::Journal { path, .. } = write {
                    records.insert(path.clone(), failure.clone());
                }
            }
        }
        Self {
            content,
            records,
            owners,
        }
    }

    pub(crate) fn error_for(&self, failure: &(dyn std::error::Error + 'static)) -> Option<Error> {
        let owner = if let Some(error) = failure.downcast_ref::<assembly::PersistenceError>() {
            Some((error.nvram, error.drive_index))
        } else if let Some(error) = failure.downcast_ref::<assembly::FloppyError>() {
            return match error {
                assembly::FloppyError::Persistence(error) => self.error_for(error),
                assembly::FloppyError::Assembly(error) => self.error_for(error),
                _ => None,
            };
        } else if let Some(error) = failure.downcast_ref::<assembly::AssemblyError>() {
            match error {
                assembly::AssemblyError::Journal { drive_index, .. } => Some((false, *drive_index)),
                _ => None,
            }
        } else {
            None
        };
        owner
            .and_then(|owner| self.owners.get(&owner))
            .and_then(|failure| failure.borrow_mut().take())
            .or_else(|| self.content.borrow_mut().take())
    }

    fn take(&self) -> Option<Error> {
        self.content.borrow_mut().take().or_else(|| {
            self.owners
                .values()
                .find_map(|failure| failure.borrow_mut().take())
        })
    }
}

struct Bytes(Vec<u8>);
impl ContentReader for Bytes {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        let start = usize::try_from(offset).ok();
        let bytes = start
            .and_then(|start| {
                start
                    .checked_add(buf.len())
                    .and_then(|end| self.0.get(start..end))
            })
            .ok_or_else(|| Error::new(ErrorKind::Content, "read past content"))?;
        buf.copy_from_slice(bytes);
        Ok(())
    }
}

#[derive(Clone)]
struct Reader {
    inner: Rc<dyn ContentReader>,
    failure: Failure,
}
impl ContentReader for Reader {
    fn size(&self) -> u64 {
        self.inner.size()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        remember(self.inner.read_at(offset, buf), &self.failure)
    }
}

pub(super) struct ContentCache {
    readers: BTreeMap<Identity, Reader>,
    failure: Failure,
}
impl ContentCache {
    pub fn open(source: &dyn ContentSource, required: &[ContentRef]) -> Result<Self, Error> {
        let failure = Failure::default();
        let mut readers = BTreeMap::new();
        for row in required {
            let inner: Rc<dyn ContentReader> = match source.open(&row.sha256, row.size)? {
                Content::Bytes(bytes) => Rc::new(Bytes(bytes)),
                Content::Reader(reader) => Rc::from(reader),
                _ => return Err(Error::new(ErrorKind::Content, "unsupported PC content")),
            };
            if inner.size() != row.size {
                return Err(Error::new(
                    ErrorKind::Content,
                    "content size differs from its launch row",
                ));
            }
            readers.insert(
                (row.sha256, row.size),
                Reader {
                    inner,
                    failure: failure.clone(),
                },
            );
        }
        Ok(Self { readers, failure })
    }
    fn reader(&self, identity: &Identity) -> Result<Reader, Error> {
        self.readers
            .get(identity)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Content, "unbound PC content"))
    }
    pub fn error(&self) -> Option<Error> {
        self.failure.borrow_mut().take()
    }
}
impl ContentSource for ContentCache {
    fn open(&self, sha256: &[u8; 32], size: u64) -> Result<Content, Error> {
        Ok(Content::Reader(Box::new(self.reader(&(*sha256, size))?)))
    }
}

pub(super) struct ContractMount {
    content: ContentCache,
    bases: BTreeMap<PathBuf, Identity>,
    records: BTreeMap<PathBuf, String>,
    initial: RefCell<BTreeMap<PathBuf, Box<dyn Record>>>,
    storage: Rc<dyn Storage>,
    failure: Failures,
}
impl ContractMount {
    pub fn new(
        lowered: &pc_config::launch::Lowered,
        content: ContentCache,
        storage: Rc<dyn Storage>,
    ) -> Result<Self, Error> {
        let mut paths = Vec::new();
        for device in &lowered.machine.devices {
            if let DeviceKind::Rtc(rtc) = &device.kind {
                if let NvramSource::Journal { journal, .. } = &rtc.nvram {
                    paths.push((journal, "NVRAM".to_owned()));
                }
            }
        }
        for (index, drive) in lowered.machine.drives.iter().enumerate() {
            let write = match &drive.media {
                Some(media) if media.empty => continue,
                Some(media) => &media.entries[media.inserted].write,
                None => &drive.write,
            };
            if let WriteMode::Journal { path, .. } = write {
                paths.push((path, format!("drive {index}")));
            }
        }
        // Acquire the initial records before any journal header is written.
        // A catalogue's other records remain free until those media are inserted.
        let failure = Failures::new(&lowered.machine, content.failure.clone());
        let mut initial = BTreeMap::new();
        for (path, owner) in paths {
            let name = lowered
                .records
                .get(path)
                .ok_or_else(|| Error::new(ErrorKind::Config, "unbound PC record"))?;
            initial.insert(
                path.clone(),
                tracked(
                    storage.open(name).map_err(|mut error| {
                        // A record open has confirmed no sequence. Keep the
                        // assembler's mount context and source chain in the
                        // contract's flat diagnostic for stored-image commands.
                        if error.kind == ErrorKind::Storage {
                            let source = format!("{error} (last confirmed sequence 0)");
                            error.message = format!("{owner} journal mount: {source}: {source}");
                        }
                        error
                    })?,
                    &failure.records[path],
                ),
            );
        }
        Ok(Self {
            content,
            bases: lowered.bases.clone(),
            records: lowered.records.clone(),
            initial: RefCell::new(initial),
            storage,
            failure,
        })
    }
    pub fn error(&self) -> Option<Error> {
        self.failure.take()
    }
    pub fn failure(&self) -> Failures {
        self.failure.clone()
    }
    fn reader(&self, path: &Path) -> Result<Reader, MountError> {
        let identity = self
            .bases
            .get(path)
            .ok_or_else(|| Error::new(ErrorKind::Config, "unbound PC base"))?;
        Ok(self.content.reader(identity)?)
    }
}
impl Mount for ContractMount {
    fn image(&self, path: &Path) -> Result<ImageBytes, MountError> {
        let reader = self.reader(path)?;
        let length = usize::try_from(reader.size())
            .map_err(|_| Error::new(ErrorKind::Content, "PC image exceeds memory"))?;
        let mut bytes = vec![0; length];
        reader.read_at(0, &mut bytes)?;
        Ok(ImageBytes::new(bytes))
    }
    fn disk_base(
        &self,
        path: &Path,
        check_length: &dyn Fn(u64) -> Result<(), ConfigError>,
    ) -> Result<DiskBase, MountError> {
        let reader = self.reader(path)?;
        check_length(reader.size())?;
        let (sha256, size) = self
            .bases
            .get(path)
            .ok_or_else(|| Error::new(ErrorKind::Config, "unbound PC base"))?;
        // The receipt covers the whole disk already. ROM pins are checked by
        // the codec, but large disk bases must stay lazy and are not hashed again.
        DiskBase::from_reader(Box::new(reader), *sha256, *size)
    }
    fn journal(&self, path: &Path) -> Result<JournalStore, MountError> {
        if let Some(record) = self.initial.borrow_mut().remove(path) {
            return Ok(JournalStore::record(record));
        }
        let name = self
            .records
            .get(path)
            .ok_or_else(|| Error::new(ErrorKind::Config, "unbound PC record"))?;
        let failure = &self.failure.records[path];
        // A remount starts a new record operation; a previous session's
        // maintenance warning must not replace an error opening this one.
        failure.borrow_mut().take();
        self.storage
            .open(name)
            .map(|record| JournalStore::record(tracked(record, failure)))
            .map_err(|error| {
                *failure.borrow_mut() = Some(error.clone());
                error.into()
            })
    }
    fn check_outputs(&self, _media: &MachineConfig) -> Result<(), ConfigError> {
        Ok(())
    }
}

fn remember<T>(result: Result<T, Error>, failure: &Failure) -> Result<T, Error> {
    result.inspect_err(|error| {
        *failure.borrow_mut() = Some(error.clone());
    })
}
fn tracked(record: Box<dyn Record>, failure: &Failure) -> Box<dyn Record> {
    Box::new(TrackedRecord {
        record,
        failure: failure.clone(),
    })
}

// The journal's internal I/O errors carry text. Keep the contract's original
// kind per owner so a swallowed maintenance warning on one medium cannot
// replace a fatal error on another. A later I/O failure on the same record
// supersedes a maintenance warning from an earlier checkpoint.
struct TrackedRecord {
    record: Box<dyn Record>,
    failure: Failure,
}
impl Record for TrackedRecord {
    fn size(&self) -> Result<u64, Error> {
        remember(self.record.size(), &self.failure)
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        remember(self.record.read_at(offset, buf), &self.failure)
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        remember(self.record.append(bytes), &self.failure)
    }
    fn truncate(&mut self, size: u64) -> Result<(), Error> {
        remember(self.record.truncate(size), &self.failure)
    }
    fn sync(&mut self) -> Result<(), Error> {
        remember(self.record.sync(), &self.failure)
    }
    fn replace(&mut self) -> Result<Box<dyn Replacement>, Error> {
        let replacement = remember(self.record.replace(), &self.failure)?;
        Ok(Box::new(TrackedReplacement {
            replacement,
            failure: self.failure.clone(),
        }))
    }
}
struct TrackedReplacement {
    replacement: Box<dyn Replacement>,
    failure: Failure,
}
impl Replacement for TrackedReplacement {
    fn size(&self) -> Result<u64, Error> {
        remember(self.replacement.size(), &self.failure)
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        remember(self.replacement.read_at(offset, buf), &self.failure)
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        remember(self.replacement.append(bytes), &self.failure)
    }
    fn commit(self: Box<Self>) -> Result<(), Error> {
        remember(self.replacement.commit(), &self.failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unemu_core_api::test_support::{MemoryStorage, Operation};

    #[test]
    fn tracked_reads_preserve_partial_buffers_and_the_original_error() {
        let storage = MemoryStorage::new();
        storage.insert("disk", b"abcdefgh");
        let failure = Failure::default();
        let mut record = tracked(storage.open("disk").unwrap(), &failure);
        let error = Error::new(ErrorKind::Storage, "partial read");
        storage.fail_next("disk", Operation::Read, error.clone());
        let mut bytes = [0; 8];
        assert_eq!(record.read_at(0, &mut bytes), Err(error.clone()));
        assert_eq!(&bytes[..4], b"abcd");
        assert_eq!(&bytes[4..], &[0; 4]);
        assert_eq!(failure.borrow_mut().take(), Some(error));
    }

    #[test]
    fn tracked_replacement_uncertainty_preserves_parent_bytes_and_poisons_writes() {
        let storage = MemoryStorage::new();
        storage.insert("disk", b"old bytes");
        let failure = Failure::default();
        let mut record = tracked(storage.open("disk").unwrap(), &failure);
        let mut replacement = record.replace().unwrap();
        let error = Error::new(ErrorKind::StorageUncertain, "partial replacement");
        storage.fail_next("disk", Operation::ReplacementAppend, error.clone());
        assert_eq!(replacement.append(b"new bytes"), Err(error.clone()));
        assert_eq!(failure.borrow_mut().take(), Some(error));
        assert_eq!(replacement.size().unwrap(), 4);
        assert_eq!(replacement.append(&[]).unwrap_err().kind, ErrorKind::Usage);
        drop(replacement);
        assert_eq!(record.append(b"tail").unwrap_err().kind, ErrorKind::Usage);
        assert_eq!(storage.contents("disk").unwrap(), b"old bytes");
        drop(record);
        let mut reopened = tracked(storage.open("disk").unwrap(), &failure);
        reopened.append(b"tail").unwrap();
        reopened.sync().unwrap();
        assert_eq!(storage.contents("disk").unwrap(), b"old bytestail");
    }
}
