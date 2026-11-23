// SPDX-License-Identifier: AGPL-3.0-or-later
//! The journal over records of the contract (core-contract.md §12.5, move 1;
//! §12.10 step 9): one scripted session written through the path
//! constructors, through a journal file handed over as a record, and through
//! the contract's storage in memory leaves the same bytes; the bytes are the
//! ones the journal wrote before it sat on records; and a running session and
//! the offline tools still exclude each other through `FILE.lock`.

use journal_files::offline::{Access, JournalTool};
use journal_files::{
    JournalFile, JournalMedia, OpenError, SectorSource, SinkStatus, VerifiedBase, FRAME_LEN,
    HEADER_LEN, SECTOR_LEN,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use unemu_core_api::test_support::{MemoryStorage, Operation};
use unemu_core_api::{Error, ErrorKind, Record, Storage};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "media-journal-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn base_bytes() -> Vec<u8> {
    (0..4 * SECTOR_LEN).map(|i| (i * 7 % 251) as u8).collect()
}

fn base() -> VerifiedBase {
    VerifiedBase::from_bytes(&base_bytes()).unwrap()
}

fn sector(value: u8) -> [u8; SECTOR_LEN] {
    [value; SECTOR_LEN]
}

/// The PC's storage as a front end builds it (core-contract.md §12.9,
/// corrected 2026-11-19): a table from record name to journal file, each
/// record a [`JournalFile`] with its `FILE.lock`.
struct JournalFiles {
    files: BTreeMap<String, PathBuf>,
}

impl Storage for JournalFiles {
    fn open(&self, name: &str) -> Result<Box<dyn Record>, Error> {
        let path = self
            .files
            .get(name)
            .ok_or_else(|| Error::new(ErrorKind::Usage, "no such record"))?;
        Ok(Box::new(JournalFile::open(path, base().identity(), None)?))
    }
}

/// Where one session's journal is kept, and how a test reaches its bytes.
enum Keep {
    /// `JournalMedia::open_verified` on a path, as assembly opens journals.
    Path(PathBuf),
    /// A [`JournalFile`] from a name-to-file storage, through `open_record`.
    Files(JournalFiles, PathBuf),
    /// The contract's storage in memory: no host file at all.
    Memory(MemoryStorage),
}

const NAME: &str = "disk.upfj";

impl Keep {
    fn mount(&self) -> JournalMedia {
        let base = base();
        let length = base.byte_len();
        match self {
            Keep::Path(path) => {
                JournalMedia::open_verified(path, &base, length, SinkStatus::default()).unwrap()
            }
            Keep::Files(files, _) => JournalMedia::open_record(
                files.open(NAME).unwrap(),
                &base,
                length,
                SinkStatus::default(),
            )
            .unwrap(),
            Keep::Memory(storage) => JournalMedia::open_record(
                storage.open(NAME).unwrap(),
                &base,
                length,
                SinkStatus::default(),
            )
            .unwrap(),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        match self {
            Keep::Path(path) | Keep::Files(_, path) => std::fs::read(path).unwrap(),
            Keep::Memory(storage) => storage.contents(NAME).unwrap(),
        }
    }

    /// Leave a torn tail, as a crash in the middle of a frame does.
    fn tear(&self, garbage: &[u8]) {
        let mut bytes = self.bytes();
        bytes.extend_from_slice(garbage);
        match self {
            Keep::Path(path) | Keep::Files(_, path) => std::fs::write(path, bytes).unwrap(),
            Keep::Memory(storage) => storage.insert(NAME, &bytes),
        }
    }
}

/// Creation, duplicate sectors, checkpoints with retention, an interrupted
/// session sealed at the next mount, retention again, and a torn tail cut at
/// mount. The bytes after each of the four sessions.
fn session(keep: &Keep) -> Vec<Vec<u8>> {
    let mut stages = Vec::new();
    let mut disk = keep.mount().with_retention(3);
    disk.write_batch(&[(0, sector(1)), (2, sector(2)), (0, sector(3))]);
    disk.checkpoint().unwrap();
    for id in 4..9 {
        disk.write_batch(&[(u64::from(id % 4), sector(id))]);
        disk.write_batch(&[(3, sector(id + 40))]);
        disk.checkpoint().unwrap();
    }
    disk.write_batch(&[(1, sector(77))]);
    disk.finish().unwrap();
    stages.push(keep.bytes());
    let mut disk = keep.mount();
    disk.write_batch(&[(2, sector(90))]);
    disk.write_batch(&[(1, sector(91)), (1, sector(92))]);
    drop(disk);
    stages.push(keep.bytes());
    let mut disk = keep.mount().with_retention(2);
    assert!(disk.mount_report().unsealed_tip.is_some());
    disk.write_batch(&[(0, sector(100))]);
    disk.checkpoint().unwrap();
    disk.finish().unwrap();
    stages.push(keep.bytes());
    keep.tear(&[0x5a; 700]);
    let mut disk = keep.mount();
    assert_eq!(disk.mount_report().discarded_bytes, 700);
    disk.write_batch(&[(3, sector(120))]);
    let image = disk.image();
    disk.finish().unwrap();
    stages.push(keep.bytes());
    assert_eq!(keep.mount().image(), image);
    stages
}

/// SHA-256 of the four stages, as the journal wrote them before it sat on
/// records (measured on the parent of this change with the same session
/// through `JournalMedia::open_verified`). Equal bytes are also why a journal
/// written before the change mounts after it: the session mounts each stage.
const BEFORE: [(usize, &str); 4] = [
    (
        9088,
        "41945b412e99244b9e62dc380ebf23b50be3e6a4536fc34918391070f5c2a227",
    ),
    (
        11648,
        "86fc69716cb34e47190f9ae2d29593449fa2c8529a19c56f4e92487b205ae00d",
    ),
    (
        4608,
        "9a98c0559fac5cdce1ecb85503e20559c38625ddefa14ea8a3a8e4abb6d18285",
    ),
    (
        6528,
        "bcca162bc5472be454662089e60b41ee66e53093987436789a6f76a1b823a723",
    ),
];

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn one_session_over_a_journal_file_and_over_memory_writes_the_same_records() {
    let dir = TempDir::new();
    let path = dir.0.join("path.upfj");
    let by_path = session(&Keep::Path(path));

    let file = dir.0.join("record.upfj");
    let files = JournalFiles {
        files: BTreeMap::from([(NAME.to_owned(), file.clone())]),
    };
    let by_file = session(&Keep::Files(files, file.clone()));

    let storage = MemoryStorage::new();
    let in_memory = session(&Keep::Memory(storage.clone()));

    assert_eq!(by_file, by_path, "a journal file handed over as a record");
    assert_eq!(in_memory, by_path, "the storage in memory");
    assert_eq!(storage.names(), [NAME]);
    assert_eq!(storage.durable(NAME), storage.contents(NAME));
    for (stage, (bytes, (length, digest))) in by_path.iter().zip(BEFORE).enumerate() {
        assert_eq!(
            (bytes.len(), hex(bytes).as_str()),
            (length, digest),
            "stage {stage}"
        );
    }
    // The record's own files beside it: the stable lock, no temporaries.
    let mut names: Vec<_> = std::fs::read_dir(&dir.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "path.upfj",
            "path.upfj.lock",
            "record.upfj",
            "record.upfj.lock"
        ]
    );
}

#[test]
fn a_command_survives_a_power_cut_in_memory_and_an_unsynchronized_one_does_not() {
    let storage = MemoryStorage::new();
    let keep = Keep::Memory(storage.clone());
    let mut disk = keep.mount();
    disk.write_batch(&[(1, sector(7))]);
    let status = disk.status();
    storage.fail_next(
        NAME,
        Operation::Sync,
        Error::new(ErrorKind::StorageUncertain, "injected sync failure"),
    );
    disk.write_batch(&[(1, sector(9))]);
    // The guest keeps its write; the host failure is latched, never retried.
    assert_eq!(disk.read_sector(1), sector(9));
    let failure = status.failure().unwrap();
    assert_eq!(
        (
            failure.operation,
            failure.message.as_str(),
            failure.last_confirmed_sequence
        ),
        ("command/end sync", "injected sync failure", 2)
    );
    drop(disk);
    storage.power_cut();
    assert_eq!(
        storage.durable(NAME).unwrap().len(),
        HEADER_LEN + 2 * FRAME_LEN
    );
    let disk = keep.mount();
    assert_eq!(disk.read_sector(1), sector(7));
    assert_eq!(disk.mount_report().last_durable_sequence, 2);
}

#[test]
fn a_running_session_and_the_offline_tools_exclude_each_other_through_file_lock() {
    let dir = TempDir::new();
    let base_path = dir.0.join("base.img");
    std::fs::write(&base_path, base_bytes()).unwrap();
    let base = VerifiedBase::open(&base_path).unwrap();
    let journal = dir.0.join("disk.upfj");
    let held = |error: Error| {
        assert_eq!(error.kind, ErrorKind::Usage, "{error}");
        assert!(error.message.starts_with("lock: "), "{error}");
    };

    // A session over a journal file record holds `disk.upfj.lock`.
    let record = JournalFile::open(&journal, base.identity(), base.metadata()).unwrap();
    let mut session = JournalMedia::open_record(
        Box::new(record),
        &base,
        base.byte_len(),
        SinkStatus::default(),
    )
    .unwrap();
    session.write_batch(&[(0, sector(5))]);
    assert!(dir.0.join("disk.upfj.lock").is_file());
    assert!(!dir.0.join("disk.upfj_lock").exists());
    for access in [Access::Inspect, Access::Mutate] {
        let Err(error) = JournalTool::open(&base_path, &journal, access) else {
            panic!("{access:?} opened a journal a session holds");
        };
        assert!(error.to_string().contains("locked"), "{error}");
    }
    held(
        JournalFile::open(&journal, base.identity(), None)
            .err()
            .unwrap(),
    );
    let status = SinkStatus::default();
    assert!(matches!(
        JournalMedia::open_verified(&journal, &base, base.byte_len(), status.clone()),
        Err(OpenError::Host(_))
    ));
    assert_eq!(status.failure().unwrap().operation, "lock");
    session.finish().unwrap();

    // An offline tool that mutates holds the same lock against both mounts.
    let mut tool = JournalTool::open(&base_path, &journal, Access::Mutate).unwrap();
    assert!(tool.has_lock());
    held(
        JournalFile::open(&journal, base.identity(), None)
            .err()
            .unwrap(),
    );
    assert!(
        JournalMedia::open_verified(&journal, &base, base.byte_len(), SinkStatus::default())
            .is_err()
    );
    assert_eq!(tool.verify().unwrap().report.generations.len(), 1);
    drop(tool);

    // An inspecting tool shares the lock with other inspections only.
    let inspect = JournalTool::open(&base_path, &journal, Access::Inspect).unwrap();
    assert!(inspect.has_lock());
    held(
        JournalFile::open(&journal, base.identity(), None)
            .err()
            .unwrap(),
    );
    drop(inspect);
    let session = JournalMedia::open_record(
        Box::new(JournalFile::open(&journal, base.identity(), None).unwrap()),
        &base,
        base.byte_len(),
        SinkStatus::default(),
    )
    .unwrap();
    assert_eq!(session.read_sector(0), sector(5));
}

#[test]
fn a_new_journal_record_is_published_whole_or_not_at_all() {
    // The header goes through a replacement: a failed commit leaves no
    // journal, and the next mount creates it.
    let storage = MemoryStorage::new();
    storage.fail_next(
        NAME,
        Operation::Commit,
        Error::new(ErrorKind::Storage, "injected commit failure"),
    );
    let status = SinkStatus::default();
    let base = base();
    let result = JournalMedia::open_record(
        storage.open(NAME).unwrap(),
        &base,
        base.byte_len(),
        status.clone(),
    );
    assert!(matches!(result, Err(OpenError::Host(_))));
    let failure = status.failure().unwrap();
    assert_eq!(
        (failure.operation, failure.message.as_str()),
        ("publish header", "injected commit failure")
    );
    assert_eq!(storage.contents(NAME).unwrap_or_default(), Vec::<u8>::new());
    assert!(!storage.is_open(NAME));
    let keep = Keep::Memory(storage.clone());
    keep.mount().finish().unwrap();
    assert_eq!(storage.durable(NAME).unwrap().len(), HEADER_LEN + FRAME_LEN);
}

/// The numbering the journal reports after a retention whose commit failed
/// uncertain: that of whichever contents the record holds, read back from it.
fn confirmed_in(bytes: &[u8]) -> u64 {
    journal_files::scan(bytes, &base_bytes(), base().byte_len())
        .report
        .generations
        .iter()
        .rev()
        .find(|g| g.commands != 0)
        .map_or(0, |g| g.last_sequence - 1)
}

#[test]
fn an_uncertain_retention_commit_reports_the_numbers_of_the_contents_the_record_holds() {
    let mut reported = Vec::new();
    for takes_effect in [false, true] {
        let storage = MemoryStorage::new();
        let keep = Keep::Memory(storage.clone());
        let mut disk = keep.mount().with_retention(2);
        let status = disk.status();
        for id in 1..=2 {
            disk.write_batch(&[(0, sector(id)), (u64::from(id), sector(id + 10))]);
            disk.checkpoint().unwrap();
        }
        disk.write_batch(&[(3, sector(3))]);
        let old = storage.contents(NAME).unwrap();
        storage.uncertain_commit(NAME, takes_effect);
        // The third closed generation starts a retention to two.
        disk.checkpoint().unwrap();
        let failure = status.maintenance_failure().unwrap();
        assert!(failure.uncertain);
        let now = storage.contents(NAME).unwrap();
        assert_eq!(now.len() < old.len() + FRAME_LEN, takes_effect);
        assert_eq!(failure.last_confirmed_sequence, confirmed_in(&now));
        // Appends stay disabled and later failures keep that numbering.
        disk.write_batch(&[(0, sector(99))]);
        let later = status.failure().unwrap();
        assert_eq!(later.last_confirmed_sequence, confirmed_in(&now));
        assert_eq!(storage.contents(NAME).unwrap(), now);
        reported.push(failure.last_confirmed_sequence);
    }
    assert_ne!(reported[0], reported[1], "retention renumbers the history");
}

#[test]
fn a_journal_file_without_a_header_is_refused_by_both_ways_in() {
    let dir = TempDir::new();
    let journal = dir.0.join("disk.upfj");
    std::fs::write(&journal, b"").unwrap();
    let base = base();
    let error = JournalFile::open(&journal, base.identity(), None)
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Storage, "{error}");
    assert!(matches!(
        JournalMedia::open_verified(&journal, &base, base.byte_len(), SinkStatus::default()),
        Err(OpenError::Refused(_))
    ));
    assert_eq!(std::fs::read(&journal).unwrap(), b"");
}

#[test]
fn a_journal_file_publishes_only_the_header_of_the_base_it_was_opened_for() {
    let dir = TempDir::new();
    let journal = dir.0.join("disk.upfj");
    let mut other = base_bytes();
    other[0] ^= 1;
    let other = VerifiedBase::from_bytes(&other).unwrap();
    let record = JournalFile::open(&journal, base().identity(), None).unwrap();
    let status = SinkStatus::default();
    let result =
        JournalMedia::open_record(Box::new(record), &other, other.byte_len(), status.clone());
    assert!(matches!(result, Err(OpenError::Host(_))));
    let failure = status.failure().unwrap();
    assert_eq!(failure.operation, "publish header");
    assert!(failure.message.contains("header"), "{failure}");
    assert!(!journal.exists());
}

#[test]
fn ordinary_reopen_recovers_an_uncertain_append_tail_without_rollback() {
    let storage = MemoryStorage::new();
    let keep = Keep::Memory(storage.clone());
    let mut disk = keep.mount();
    disk.write_batch(&[(1, sector(7))]);
    let confirmed = storage.contents(NAME).unwrap();
    storage.fail_next(
        NAME,
        Operation::Append,
        Error::new(ErrorKind::StorageUncertain, "partial append"),
    );
    disk.write_batch(&[(1, sector(9))]);
    assert!(disk.status().failure().is_some());
    let torn = storage.contents(NAME).unwrap();
    assert_eq!(torn.len(), confirmed.len() + FRAME_LEN / 2);
    disk.write_batch(&[(2, sector(10))]);
    assert_eq!(
        storage.contents(NAME).unwrap(),
        torn,
        "poisoned output is never retried"
    );
    drop(disk);
    assert_eq!(
        storage.contents(NAME).unwrap(),
        torn,
        "close is not rollback"
    );
    let mut recovered = keep.mount();
    assert_eq!(recovered.mount_report().discarded_bytes, FRAME_LEN / 2);
    assert_eq!(recovered.read_sector(1), sector(7));
    assert_eq!(storage.contents(NAME), storage.durable(NAME));
    recovered.write_batch(&[(2, sector(11))]);
    recovered.finish().unwrap();
    assert_eq!(keep.mount().read_sector(2), sector(11));
}

#[test]
fn a_power_cut_after_an_uncertain_append_recovers_the_confirmed_command() {
    let storage = MemoryStorage::new();
    let keep = Keep::Memory(storage.clone());
    let mut disk = keep.mount();
    disk.write_batch(&[(1, sector(7))]);
    storage.fail_next(
        NAME,
        Operation::Append,
        Error::new(ErrorKind::StorageUncertain, "partial append"),
    );
    disk.write_batch(&[(1, sector(9))]);
    drop(disk);
    storage.power_cut();
    let recovered = keep.mount();
    assert_eq!(recovered.read_sector(1), sector(7));
    assert_eq!(recovered.mount_report().discarded_bytes, 0);
}

#[test]
fn uncertain_truncation_does_not_resurrect_synchronized_journal_bytes() {
    for power_cut in [false, true] {
        let storage = MemoryStorage::new();
        let keep = Keep::Memory(storage.clone());
        let mut disk = keep.mount();
        disk.write_batch(&[(1, sector(7))]);
        disk.finish().unwrap();
        let mut record = storage.open(NAME).unwrap();
        storage.fail_next(
            NAME,
            Operation::Truncate,
            Error::new(ErrorKind::StorageUncertain, "destructive cut"),
        );
        assert_eq!(
            record.truncate(HEADER_LEN as u64).unwrap_err().kind,
            ErrorKind::StorageUncertain
        );
        assert_eq!(record.size().unwrap(), HEADER_LEN as u64);
        assert_eq!(record.sync().unwrap_err().kind, ErrorKind::Usage);
        drop(record);
        if power_cut {
            storage.power_cut();
        }
        assert_eq!(storage.contents(NAME).unwrap().len(), HEADER_LEN);
        let recovered = keep.mount();
        assert_eq!(recovered.read_sector(1), base().read_sector(1).unwrap());
        assert_eq!(recovered.mount_report().last_durable_sequence, 0);
    }
}

#[test]
fn a_partial_failed_record_read_never_becomes_a_valid_journal_scan() {
    let storage = MemoryStorage::new();
    let keep = Keep::Memory(storage.clone());
    keep.mount().finish().unwrap();
    let before = storage.contents(NAME).unwrap();
    storage.fail_next(
        NAME,
        Operation::Read,
        Error::new(ErrorKind::Storage, "partial read"),
    );
    let status = SinkStatus::default();
    let base = base();
    assert!(matches!(
        JournalMedia::open_record(
            storage.open(NAME).unwrap(),
            &base,
            base.byte_len(),
            status.clone()
        ),
        Err(OpenError::Host(_))
    ));
    assert_eq!(status.failure().unwrap().operation, "scan");
    assert_eq!(storage.contents(NAME).unwrap(), before);
    keep.mount().finish().unwrap();
}

#[test]
fn named_journals_accept_sidecar_shaped_record_names_through_the_shared_mapping() {
    let dir = TempDir::new();
    let storage = unemu_core_files::FileStorage::open(&dir.0).unwrap();
    for name in [
        "record.lock",
        "record.create-0.tmp",
        "record.retain-9.tmp",
        "con.lock",
    ] {
        let base = base();
        let mut disk = JournalMedia::open_record(
            storage.open(name).unwrap(),
            &base,
            base.byte_len(),
            SinkStatus::default(),
        )
        .unwrap();
        disk.write_batch(&[(0, sector(5))]);
        disk.finish().unwrap();
        let mapped = unemu_core_files::file_name(name).unwrap();
        assert_eq!(
            unemu_core_files::record_name(&mapped).as_deref(),
            Some(name)
        );
        assert!(dir.0.join(mapped).is_file());
        let disk = JournalMedia::open_record(
            storage.open(name).unwrap(),
            &base,
            base.byte_len(),
            SinkStatus::default(),
        )
        .unwrap();
        assert_eq!(disk.read_sector(0), sector(5));
    }
}
