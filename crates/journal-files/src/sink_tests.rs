// SPDX-License-Identifier: AGPL-3.0-or-later
//! Filesystem model and real-file checks; persistent-media.md § Tests. Software
//! barriers measure ordering and valid-prefix recovery, not physical power cuts.

use super::*;
use crate::temporary::CreationTempIo;
use crate::{scan, FRAME_LEN, HEADER_LEN};
use std::cell::Cell;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Lock,
    Open,
    VerifyPath,
    CleanupTemps,
    CreateTemp,
    Publish,
    RemoveTemp,
    DirectorySync,
    Read(usize),
    Seek,
    Write(usize),
    Flush,
    Sync,
    Truncate(u64),
}

#[derive(Default)]
struct Model {
    bytes: Cursor<Vec<u8>>,
    durable: Vec<u8>,
    events: Vec<Event>,
    fail_on: Option<usize>,
    fail_errno: Option<i32>,
    write_budget: Option<usize>,
    short_limit: Option<usize>,
    interrupted: bool,
    zero_write: bool,
    published: bool,
    temporary_exists: bool,
    leftovers: usize,
}

impl Model {
    fn step(&mut self, event: Event) -> io::Result<()> {
        self.events.push(event);
        if self.fail_on == Some(self.events.len() - 1) {
            return Err(self
                .fail_errno
                .map(io::Error::from_raw_os_error)
                .unwrap_or_else(|| io::Error::other("injected operation failure")));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ModelIo(Rc<RefCell<Model>>);
impl Read for ModelIo {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut model = self.0.borrow_mut();
        model.step(Event::Read(bytes.len()))?;
        model.bytes.read(bytes)
    }
}
impl Seek for ModelIo {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let mut model = self.0.borrow_mut();
        model.step(Event::Seek)?;
        model.bytes.seek(position)
    }
}
impl Write for ModelIo {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut model = self.0.borrow_mut();
        model.step(Event::Write(bytes.len()))?;
        if std::mem::take(&mut model.interrupted) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "interrupted"));
        }
        if model.zero_write {
            return Ok(0);
        }
        let mut length = bytes.len().min(model.short_limit.unwrap_or(usize::MAX));
        if let Some(budget) = model.write_budget.as_mut() {
            if *budget == 0 {
                return Err(io::Error::other("injected short-write failure"));
            }
            length = length.min(*budget);
            *budget -= length;
        }
        model.bytes.write(&bytes[..length])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.borrow_mut().step(Event::Flush)
    }
}
impl JournalIo for ModelIo {
    fn metadata(&self) -> io::Result<std::fs::Metadata> {
        let dir = TempDir::new();
        let file = File::create(dir.journal())?;
        file.set_len(self.0.borrow().bytes.get_ref().len() as u64)?;
        file.metadata()
    }

    fn sync(&mut self) -> io::Result<()> {
        let mut model = self.0.borrow_mut();
        model.step(Event::Sync)?;
        model.durable = model.bytes.get_ref().clone();
        Ok(())
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        let mut model = self.0.borrow_mut();
        model.step(Event::Truncate(length))?;
        model.bytes.get_mut().truncate(length as usize);
        Ok(())
    }
}

struct Guard(Rc<Cell<bool>>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
struct ModelFs {
    io: ModelIo,
    locked: Rc<Cell<bool>>,
}
impl Filesystem for ModelFs {
    fn lock(&mut self, _: &Path) -> io::Result<Box<dyn Any>> {
        self.io.0.borrow_mut().step(Event::Lock)?;
        if self.locked.replace(true) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "locked"));
        }
        Ok(Box::new(Guard(self.locked.clone())))
    }
    fn open(&mut self, _: &Path) -> io::Result<Option<Box<dyn JournalIo>>> {
        let mut model = self.io.0.borrow_mut();
        model.step(Event::Open)?;
        Ok(model
            .published
            .then(|| Box::new(self.io.clone()) as Box<dyn JournalIo>))
    }
    fn verify_path(&mut self, _: &Path) -> io::Result<()> {
        assert!(self.locked.get());
        self.io.0.borrow_mut().step(Event::VerifyPath)
    }
    fn cleanup_creation_temps(
        &mut self,
        _: &Path,
        _: &[u8],
        _: Option<&std::fs::Metadata>,
    ) -> io::Result<()> {
        assert!(self.locked.get());
        let mut model = self.io.0.borrow_mut();
        model.step(Event::CleanupTemps)?;
        model.leftovers = 0;
        Ok(())
    }
    fn create_temp(&mut self, _: &Path) -> io::Result<(Box<dyn JournalIo>, PathBuf)> {
        let mut model = self.io.0.borrow_mut();
        model.step(Event::CreateTemp)?;
        model.temporary_exists = true;
        model.bytes = Cursor::new(Vec::new());
        model.durable.clear();
        Ok((Box::new(self.io.clone()), PathBuf::from("header.tmp")))
    }
    fn publish(&mut self, _: &Path, _: &Path) -> io::Result<()> {
        let mut model = self.io.0.borrow_mut();
        model.step(Event::Publish)?;
        assert!(model.temporary_exists);
        assert!(!model.published);
        assert_eq!(model.durable.len(), HEADER_LEN);
        model.published = true;
        Ok(())
    }
    fn remove_temp(&mut self, _: &Path) -> io::Result<()> {
        let mut model = self.io.0.borrow_mut();
        model.step(Event::RemoveTemp)?;
        model.temporary_exists = false;
        Ok(())
    }
    fn sync_directory(&mut self, _: &Path) -> io::Result<()> {
        self.io.0.borrow_mut().step(Event::DirectorySync)
    }
}

fn model_fs(bytes: Option<Vec<u8>>) -> ModelFs {
    let published = bytes.is_some();
    ModelFs {
        io: ModelIo(Rc::new(RefCell::new(Model {
            bytes: Cursor::new(bytes.unwrap_or_default()),
            published,
            ..Model::default()
        }))),
        locked: Rc::default(),
    }
}
fn mount(fs: &mut ModelFs, status: SinkStatus) -> Result<JournalMedia, OpenError> {
    JournalMedia::open_with(
        fs,
        Path::new("disk.upfj"),
        &[0; 2 * SECTOR_LEN],
        (2 * SECTOR_LEN) as u64,
        status,
    )
}
fn batch(value: u8) -> [(u64, [u8; SECTOR_LEN]); 2] {
    [
        (1, [value; SECTOR_LEN]),
        (0, [value.wrapping_add(1); SECTOR_LEN]),
    ]
}
fn assert_image(media: &JournalMedia, value: u8) {
    assert_eq!(media.read_sector(1), [value; SECTOR_LEN]);
    assert_eq!(media.read_sector(0), [value.wrapping_add(1); SECTOR_LEN]);
    assert!(!media.write_protected());
}

#[test]
fn creation_and_each_command_have_ordered_barriers() {
    let mut fs = model_fs(None);
    let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
    assert_eq!(
        &fs.io.0.borrow().events[..11],
        &[
            Event::Lock,
            Event::Open,
            Event::CreateTemp,
            Event::Write(HEADER_LEN),
            Event::Flush,
            Event::Sync,
            Event::Publish,
            Event::RemoveTemp,
            Event::VerifyPath,
            Event::CleanupTemps,
            Event::DirectorySync,
        ]
    );
    fs.io.0.borrow_mut().events.clear();
    media.write_batch(&batch(7));
    assert_eq!(
        fs.io.0.borrow().events,
        [
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Flush,
            Event::Sync,
        ]
    );
    assert_eq!(fs.io.0.borrow().durable, *fs.io.0.borrow().bytes.get_ref());
    let durable = fs.io.0.borrow().durable.clone();
    for (index, expected_lba) in [1, 0].into_iter().enumerate() {
        let start = HEADER_LEN + index * FRAME_LEN;
        assert!(
            matches!(Frame::decode(&durable[start..start + FRAME_LEN]).unwrap().record,
            Record::Sector { lba, .. } if lba == expected_lba)
        );
    }
    let report = scan(
        &fs.io.0.borrow().durable,
        &[0; 2 * SECTOR_LEN],
        (2 * SECTOR_LEN) as u64,
    );
    assert_eq!(report.image.as_deref(), Some(media.image().as_slice()));
    assert_eq!(report.report.last_durable_sequence, 3);
    fs.io.0.borrow_mut().events.clear();
    media.checkpoint().unwrap();
    assert_eq!(
        fs.io.0.borrow().events,
        [
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Flush,
            Event::Sync
        ]
    );
    fs.io.0.borrow_mut().events.clear();
    media.finish().unwrap();
    assert_eq!(
        fs.io.0.borrow().events,
        [
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Flush,
            Event::Sync
        ]
    );
    assert!(!fs.locked.get());
}

#[test]
fn mounted_sector_access_never_reads_or_seeks_host_storage() {
    let mut fs = model_fs(None);
    let mut medium = mount(&mut fs, SinkStatus::default()).unwrap();
    fs.io.0.borrow_mut().events.clear();
    assert_eq!(medium.read_sector(0), [0; SECTOR_LEN]);
    assert_eq!(medium.read_sector(1), [0; SECTOR_LEN]);
    assert!(fs.io.0.borrow().events.is_empty());
    medium.write_batch(&batch(7));
    assert_image(&medium, 7);
    assert_eq!(
        fs.io.0.borrow().events,
        [
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Flush,
            Event::Sync,
        ]
    );
}

#[test]
fn short_and_interrupted_writes_never_duplicate_bytes() {
    let mut expected_fs = model_fs(None);
    let mut expected = mount(&mut expected_fs, SinkStatus::default()).unwrap();
    expected.write_batch(&batch(9));
    expected.finish().unwrap();
    for limit in [1, 7, 127, 639] {
        let mut fs = model_fs(None);
        fs.io.0.borrow_mut().short_limit = Some(limit);
        fs.io.0.borrow_mut().interrupted = true;
        let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
        fs.io.0.borrow_mut().interrupted = true;
        media.write_batch(&batch(9));
        media.finish().unwrap();
        assert_eq!(fs.io.0.borrow().durable, expected_fs.io.0.borrow().durable);
    }
}

#[test]
fn every_partial_command_failure_preserves_memory_and_latches_first_error() {
    for budget in 0..3 * FRAME_LEN {
        let mut fs = model_fs(None);
        let status = SinkStatus::default();
        let mut media = mount(&mut fs, status.clone()).unwrap();
        media.write_batch(&batch(3));
        media.checkpoint().unwrap();
        fs.io.0.borrow_mut().write_budget = Some(budget);
        media.write_batch(&batch(7));
        assert_image(&media, 7);
        let failure = status.failure().expect("partial write must be latched");
        assert_eq!(failure.operation, "write");
        assert_eq!(failure.last_confirmed_sequence, 3);
        let calls = fs.io.0.borrow().events.len();
        media.write_batch(&batch(11));
        assert_image(&media, 11);
        assert_eq!(media.checkpoint(), Err(failure.clone()));
        assert_eq!(media.finish(), Err(failure.clone()));
        assert_eq!(status.failure(), Some(failure));
        assert_eq!(fs.io.0.borrow().events.len(), calls);
        assert!(!fs.locked.get());
        let bytes = fs.io.0.borrow().bytes.get_ref().clone();
        // Previously the failed append left HEADER_LEN + 4 * FRAME_LEN + budget
        // bytes. Rollback now removes its partial frame, keeping earlier appends.
        assert_eq!(
            bytes.len(),
            HEADER_LEN + (4 + budget / FRAME_LEN) * FRAME_LEN
        );
        let scan = scan(&bytes, &[0; 2 * SECTOR_LEN], (2 * SECTOR_LEN) as u64);
        assert_eq!(
            scan.image.unwrap(),
            [vec![4; SECTOR_LEN], vec![3; SECTOR_LEN]].concat()
        );
        assert_eq!(scan.report.safe_offset, HEADER_LEN + 4 * FRAME_LEN);
    }
}

#[test]
fn failed_flush_sync_and_zero_write_preserve_guest_data_and_poison_output() {
    for (fail_on, zero_write) in [(Some(6), false), (Some(7), false), (None, true)] {
        let mut fs = model_fs(None);
        let status = SinkStatus::default();
        let mut media = mount(&mut fs, status.clone()).unwrap();
        {
            let mut model = fs.io.0.borrow_mut();
            model.events.clear();
            model.fail_on = fail_on;
            model.zero_write = zero_write;
        }
        media.write_batch(&batch(5));
        assert_image(&media, 5);
        let failure = status.failure().unwrap();
        assert_eq!(failure.last_confirmed_sequence, 0);
        let operations = fs.io.0.borrow().events.len();
        media.write_batch(&batch(8));
        assert_image(&media, 8);
        assert_eq!(media.finish(), Err(failure));
        assert_eq!(fs.io.0.borrow().events.len(), operations);
        assert_eq!(fs.io.0.borrow().durable.len(), HEADER_LEN);
    }
}

#[cfg(unix)]
#[test]
fn enospc_and_eio_latch_without_write_protection_or_lost_memory_writes() {
    for errno in [libc::ENOSPC, libc::EIO] {
        for operation in [1, 7] {
            let mut fs = model_fs(None);
            let status = SinkStatus::default();
            let mut medium = mount(&mut fs, status.clone()).unwrap();
            {
                let mut model = fs.io.0.borrow_mut();
                model.events.clear();
                model.fail_on = Some(operation);
                model.fail_errno = Some(errno);
            }
            medium.write_batch(&batch(5));
            assert_image(&medium, 5);
            let failure = status.failure().unwrap();
            assert!(failure
                .message
                .contains(&io::Error::from_raw_os_error(errno).to_string()));
            assert_eq!(failure.last_confirmed_sequence, 0);
            let events = fs.io.0.borrow().events.clone();
            medium.write_batch(&batch(7));
            assert_image(&medium, 7);
            assert_eq!(status.failure(), Some(failure.clone()));
            assert_eq!(medium.finish(), Err(failure));
            assert_eq!(fs.io.0.borrow().events, events);
            assert_eq!(fs.io.0.borrow().durable.len(), HEADER_LEN);
        }
    }
}

#[test]
fn lost_or_reordered_unsynchronized_frames_never_half_replay_a_command() {
    let mut fs = model_fs(None);
    let status = SinkStatus::default();
    let mut medium = mount(&mut fs, status.clone()).unwrap();
    medium.write_batch(&batch(3));
    medium.checkpoint().unwrap();
    let confirmed = fs.io.0.borrow().durable.clone();
    let previous = medium.image().to_vec();
    {
        let mut model = fs.io.0.borrow_mut();
        model.events.clear();
        model.fail_on = Some(7);
    }
    medium.write_batch(&batch(9));
    assert_eq!(status.failure().unwrap().operation, "command/end sync");
    assert_eq!(status.failure().unwrap().last_confirmed_sequence, 3);
    assert_eq!(fs.io.0.borrow().durable, confirmed);
    let updated = medium.image().to_vec();
    let unconfirmed = fs.io.0.borrow().bytes.get_ref()[confirmed.len()..].to_vec();
    assert_eq!(unconfirmed.len(), 3 * FRAME_LEN);
    // A failed barrier may leave any subset/order of the new frames, but cannot
    // revoke the earlier successful barrier under the storage contract.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        for mask in 0..8 {
            let mut suffix = Vec::new();
            for frame in order {
                if mask & (1 << frame) != 0 {
                    suffix.extend_from_slice(
                        &unconfirmed[frame * FRAME_LEN..(frame + 1) * FRAME_LEN],
                    );
                }
            }
            for cut in 0..=suffix.len() {
                let mut crashed = confirmed.clone();
                crashed.extend_from_slice(&suffix[..cut]);
                let recovered = scan(&crashed, &[0; 2 * SECTOR_LEN], (2 * SECTOR_LEN) as u64);
                assert!(!matches!(recovered.report.status, Status::Refused(_)));
                let expected = if order == [0, 1, 2] && mask == 7 && cut == suffix.len() {
                    &updated
                } else {
                    &previous
                };
                assert_eq!(
                    recovered.image.as_ref(),
                    Some(expected),
                    "order {order:?}, mask {mask}, cut {cut}"
                );
            }
        }
    }
}

#[test]
fn every_creation_operation_failure_aborts_mount_and_releases_lock() {
    let mut fs = model_fs(None);
    drop(mount(&mut fs, SinkStatus::default()).unwrap());
    let events = fs.io.0.borrow().events.clone();
    let publish = events.iter().position(|e| *e == Event::Publish).unwrap();
    for fail_on in 0..events.len() {
        let mut fs = model_fs(None);
        let status = SinkStatus::default();
        fs.io.0.borrow_mut().fail_on = Some(fail_on);
        assert!(
            matches!(mount(&mut fs, status.clone()), Err(OpenError::Host(_))),
            "operation {fail_on}"
        );
        assert!(status.failure().is_some());
        assert!(!fs.locked.get());
        let model = fs.io.0.borrow();
        assert_eq!(&model.events[..=fail_on], &events[..=fail_on]);
        assert!(model.events[fail_on + 1..]
            .iter()
            .all(|e| *e == Event::RemoveTemp));
        assert!(!model.temporary_exists);
        assert_eq!(model.published, fail_on > publish);
        if model.published {
            assert_eq!(model.durable.len(), HEADER_LEN);
        }
    }
}

fn interrupted_journal() -> Vec<u8> {
    let mut fs = model_fs(None);
    let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
    media.write_batch(&batch(6));
    drop(media);
    let mut bytes = fs.io.0.borrow().durable.clone();
    bytes.extend_from_slice(&[0xff; 73]);
    bytes
}

#[test]
fn repair_truncates_and_syncs_before_sealing_and_refuses_every_io_failure() {
    let bytes = interrupted_journal();
    let mut fs = model_fs(Some(bytes.clone()));
    let media = mount(&mut fs, SinkStatus::default()).unwrap();
    assert_image(&media, 6);
    assert_eq!(media.mount_report().discarded_bytes, 73);
    let events = fs.io.0.borrow().events.clone();
    let truncate = events
        .iter()
        .position(|e| matches!(e, Event::Truncate(_)))
        .unwrap();
    assert_eq!(
        &events[truncate..],
        &[
            Event::Truncate((HEADER_LEN + 3 * FRAME_LEN) as u64),
            Event::Flush,
            Event::Sync,
            Event::Seek,
            Event::Write(FRAME_LEN),
            Event::Flush,
            Event::Sync,
        ]
    );
    let scan = scan(
        &fs.io.0.borrow().durable,
        &[0; 2 * SECTOR_LEN],
        (2 * SECTOR_LEN) as u64,
    );
    assert_eq!(scan.report.generations[0].reason, ClosingReason::Recovered);
    assert_eq!(scan.report.status, Status::Valid);
    assert_eq!(scan.report.discarded_bytes, 0);
    drop(media);
    for fail_on in 0..events.len() {
        let mut fs = model_fs(Some(bytes.clone()));
        let status = SinkStatus::default();
        fs.io.0.borrow_mut().fail_on = Some(fail_on);
        assert!(
            matches!(mount(&mut fs, status.clone()), Err(OpenError::Host(_))),
            "operation {fail_on}"
        );
        assert_eq!(
            status.failure().unwrap().last_confirmed_sequence,
            if fail_on > truncate + 2 { 3 } else { 0 }
        );
        assert!(!fs.locked.get());
        let actual = fs.io.0.borrow().events.clone();
        assert_eq!(&actual[..=fail_on], &events[..=fail_on]);
        assert!(actual[fail_on + 1..]
            .iter()
            .all(|event| matches!(event, Event::Truncate(_))));
    }
}

#[test]
fn failed_checkpoint_or_finish_never_seals_again() {
    for finish in [false, true] {
        for operation in 0..3 {
            let mut fs = model_fs(None);
            let status = SinkStatus::default();
            let mut media = mount(&mut fs, status.clone()).unwrap();
            media.write_batch(&batch(1));
            {
                let mut model = fs.io.0.borrow_mut();
                model.events.clear();
                model.fail_on = Some(operation);
            }
            if finish {
                assert!(media.finish().is_err());
            } else {
                let error = media.checkpoint().unwrap_err();
                let events = fs.io.0.borrow().events.len();
                assert_eq!(media.finish(), Err(error));
                assert_eq!(fs.io.0.borrow().events.len(), events);
            }
            assert_eq!(status.failure().unwrap().last_confirmed_sequence, 3);
        }
    }
}

#[test]
fn model_lock_contention_is_latched_before_journal_access() {
    let mut fs = model_fs(None);
    let media = mount(&mut fs, SinkStatus::default()).unwrap();
    let before = fs.io.0.borrow().bytes.get_ref().clone();
    fs.io.0.borrow_mut().events.clear();
    let status = SinkStatus::default();
    assert!(matches!(
        mount(&mut fs, status.clone()),
        Err(OpenError::Host(_))
    ));
    assert_eq!(status.failure().unwrap().operation, "lock");
    assert_eq!(fs.io.0.borrow().events, [Event::Lock]);
    assert_eq!(*fs.io.0.borrow().bytes.get_ref(), before);
    assert!(fs.locked.get());
    drop(media);
    assert!(!fs.locked.get());
}

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::temp_dir();
        loop {
            let path = parent.join(format!(
                "media-sink-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("test directory: {e}"),
            }
        }
    }
    fn journal(&self) -> PathBuf {
        self.0.join("disk.upfj")
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn real_mount(path: &Path) -> Result<JournalMedia, OpenError> {
    JournalMedia::open(
        path,
        &[0; 2 * SECTOR_LEN],
        (2 * SECTOR_LEN) as u64,
        SinkStatus::default(),
    )
}

#[test]
fn real_file_round_trip_checkpoints_finish_and_empty_generations() {
    let dir = TempDir::new();
    let path = dir.journal();
    let mut media = real_mount(&path).unwrap();
    media.write_batch(&batch(9));
    media.checkpoint().unwrap();
    media.checkpoint().unwrap();
    media.write_batch(&batch(17));
    let image = media.image().to_vec();
    media.finish().unwrap();
    let media = real_mount(&path).unwrap();
    assert_eq!(media.image(), image);
    let generations = &media.mount_report().generations;
    assert_eq!(
        generations
            .iter()
            .map(|g| (g.id, g.reason, g.commands, g.last_sequence))
            .collect::<Vec<_>>(),
        [
            (1, ClosingReason::Checkpoint, 1, 4),
            (2, ClosingReason::Checkpoint, 0, 5),
            (3, ClosingReason::SessionEnd, 1, 9),
        ]
    );
    media.finish().unwrap();
    let media = real_mount(&path).unwrap();
    assert_eq!(media.mount_report().generations[3].id, 4);
    assert_eq!(media.mount_report().generations[3].commands, 0);
}

#[test]
fn real_lock_excludes_second_open_and_drop_only_releases_handles() {
    let dir = TempDir::new();
    let path = dir.journal();
    let mut media = real_mount(&path).unwrap();
    media.write_batch(&batch(4));
    let before = std::fs::read(&path).unwrap();
    let status = SinkStatus::default();
    let error = JournalMedia::open(
        &path,
        &[0; 2 * SECTOR_LEN],
        (2 * SECTOR_LEN) as u64,
        status.clone(),
    );
    assert!(matches!(error, Err(OpenError::Host(_))));
    assert_eq!(status.failure().unwrap().operation, "lock");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_image(&media, 4);
    assert!(media.status().failure().is_none());
    drop(media);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(path.with_extension("upfj.lock").exists());
    let media = real_mount(&path).unwrap();
    assert_image(&media, 4);
    assert_eq!(
        media
            .mount_report()
            .unsealed_tip
            .as_ref()
            .unwrap()
            .generation,
        1
    );
    media.finish().unwrap();
    let media = real_mount(&path).unwrap();
    assert_eq!(
        media.mount_report().generations[0].reason,
        ClosingReason::Recovered
    );
    assert_eq!(media.mount_report().generations[1].id, 2);
}

#[test]
fn real_truncated_frame_is_discarded_then_interrupted_tip_is_sealed() {
    let dir = TempDir::new();
    let path = dir.journal();
    let mut media = real_mount(&path).unwrap();
    media.write_batch(&batch(1));
    media.checkpoint().unwrap();
    media.write_batch(&batch(7));
    media.write_batch(&batch(12));
    drop(media);
    let file = OpenOptions::new().write(true).open(&path).unwrap();
    // Cut the second sector of the final command; its first sector must not replay.
    file.set_len((HEADER_LEN + 8 * FRAME_LEN + 73) as u64)
        .unwrap();
    drop(file);
    let mut media = real_mount(&path).unwrap();
    assert_image(&media, 7);
    assert_eq!(media.mount_report().discarded_bytes, FRAME_LEN + 73);
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        (HEADER_LEN + 8 * FRAME_LEN) as u64
    );
    media.write_batch(&batch(19));
    media.finish().unwrap();
    let media = real_mount(&path).unwrap();
    assert_image(&media, 19);
    assert_eq!(
        media
            .mount_report()
            .generations
            .iter()
            .map(|g| (g.id, g.reason))
            .collect::<Vec<_>>(),
        [
            (1, ClosingReason::Checkpoint),
            (2, ClosingReason::Recovered),
            (3, ClosingReason::SessionEnd),
        ]
    );
    assert_eq!(media.mount_report().status, Status::Valid);
}

#[test]
fn real_refusals_preserve_invalid_header_base_anchor_and_interior_damage() {
    let base = [0; 2 * SECTOR_LEN];
    let dir = TempDir::new();
    let path = dir.journal();
    let mut fs = model_fs(None);
    let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
    media.write_batch(&batch(1));
    media.finish().unwrap();
    let closed = fs.io.0.borrow().durable.clone();
    let mut interior = closed.clone();
    interior[HEADER_LEN + 80] ^= 1;
    let anchor = Header::new(&base, 9).unwrap().encode().unwrap().to_vec();
    let mut wrong_base = base;
    wrong_base[0] = 1;
    let wrong = Header::new(&wrong_base, 0)
        .unwrap()
        .encode()
        .unwrap()
        .to_vec();
    for bytes in [vec![], vec![0; 17], wrong, anchor, interior] {
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(real_mount(&path), Err(OpenError::Refused(_))));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn pending_only_tail_reuses_generation_and_invalid_batches_do_not_append() {
    let mut fs = model_fs(None);
    let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
    media.write_batch(&batch(1));
    drop(media);
    let mut bytes = fs.io.0.borrow().durable.clone();
    bytes.truncate(HEADER_LEN + FRAME_LEN);
    let mut fs = model_fs(Some(bytes));
    let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
    assert_eq!(media.image(), [0; 2 * SECTOR_LEN]);
    assert_eq!(fs.io.0.borrow().durable.len(), HEADER_LEN);
    let operations = fs.io.0.borrow().events.len();
    for batch in [
        vec![],
        vec![(2, [1; SECTOR_LEN])],
        vec![(u64::MAX, [1; SECTOR_LEN])],
        vec![(0, [1; SECTOR_LEN]), (2, [2; SECTOR_LEN])],
    ] {
        media.write_batch(&batch);
    }
    assert_eq!(media.image(), [0; 2 * SECTOR_LEN]);
    assert_eq!(media.read_sector(u64::MAX), [0; SECTOR_LEN]);
    assert_eq!(fs.io.0.borrow().events.len(), operations);
    media.finish().unwrap();
    let scan = scan(
        &fs.io.0.borrow().durable,
        &[0; 2 * SECTOR_LEN],
        (2 * SECTOR_LEN) as u64,
    );
    assert_eq!(scan.report.generations[0].id, 1);
    assert_eq!(scan.report.generations[0].commands, 0);
}

#[test]
fn reopen_clean_and_unsealed_histories_sync_directory_and_mount_before_append() {
    for closed in [true, false] {
        let mut fs = model_fs(None);
        let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
        media.write_batch(&batch(6));
        if closed {
            media.finish().unwrap();
        } else {
            drop(media);
        }
        let bytes = fs.io.0.borrow().durable.clone();
        let mut expected = vec![
            Event::Lock,
            Event::Open,
            Event::VerifyPath,
            Event::CleanupTemps,
            Event::DirectorySync,
            Event::Seek,
            Event::Read(HEADER_LEN),
        ];
        for _ in 0..if closed { 4 } else { 3 } {
            expected.extend([Event::Seek, Event::Read(FRAME_LEN)]);
        }
        let mount_sync = expected.len() + 1;
        expected.extend([Event::Flush, Event::Sync]);
        if !closed {
            expected.extend([
                Event::Seek,
                Event::Write(FRAME_LEN),
                Event::Flush,
                Event::Sync,
            ]);
        }
        let mut fs = model_fs(Some(bytes.clone()));
        let media = mount(&mut fs, SinkStatus::default()).unwrap();
        assert_image(&media, 6);
        assert_eq!(fs.io.0.borrow().events, expected);
        assert_eq!(media.confirmed_sequence(), 3);
        let replay = scan(
            &fs.io.0.borrow().durable,
            &[0; 2 * SECTOR_LEN],
            (2 * SECTOR_LEN) as u64,
        );
        assert_eq!(replay.report.status, Status::Valid);
        assert!(replay.report.unsealed_tip.is_none());
        assert_eq!(
            replay.report.generations[0].reason,
            if closed {
                ClosingReason::SessionEnd
            } else {
                ClosingReason::Recovered
            }
        );
        drop(media);
        for fail_on in 0..expected.len() {
            let mut fs = model_fs(Some(bytes.clone()));
            let status = SinkStatus::default();
            fs.io.0.borrow_mut().fail_on = Some(fail_on);
            assert!(
                matches!(mount(&mut fs, status.clone()), Err(OpenError::Host(_))),
                "closed={closed}, operation={fail_on}"
            );
            assert_eq!(
                status.failure().unwrap().last_confirmed_sequence,
                if fail_on > mount_sync { 3 } else { 0 }
            );
            assert!(!fs.locked.get());
            let actual = fs.io.0.borrow().events.clone();
            assert_eq!(&actual[..=fail_on], &expected[..=fail_on]);
            assert!(actual[fail_on + 1..]
                .iter()
                .all(|event| matches!(event, Event::Truncate(_))));
            assert!(fs.io.0.borrow().published);
        }
    }
}

#[test]
fn duplicate_lbas_keep_last_bytes_and_last_occurrence_order() {
    let mut fs = model_fs(None);
    let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
    media.write_batch(&[
        (1, [1; SECTOR_LEN]),
        (0, [2; SECTOR_LEN]),
        (1, [3; SECTOR_LEN]),
        (0, [4; SECTOR_LEN]),
        (1, [5; SECTOR_LEN]),
    ]);
    assert_eq!(media.read_sector(0), [4; SECTOR_LEN]);
    assert_eq!(media.read_sector(1), [5; SECTOR_LEN]);
    assert!(media.status().failure().is_none());
    let durable = fs.io.0.borrow().durable.clone();
    assert_eq!(durable.len(), HEADER_LEN + 3 * FRAME_LEN);
    for (index, (lba, value)) in [(0, 4), (1, 5)].into_iter().enumerate() {
        let start = HEADER_LEN + index * FRAME_LEN;
        assert_eq!(
            Frame::decode(&durable[start..start + FRAME_LEN])
                .unwrap()
                .record,
            Record::Sector {
                lba,
                bytes: Box::new([value; SECTOR_LEN])
            }
        );
    }
    assert_eq!(
        Frame::decode(&durable[HEADER_LEN + 2 * FRAME_LEN..])
            .unwrap()
            .record,
        Record::CommandEnd { sectors: 2 }
    );
    let expected = media.image().to_vec();
    drop(media);
    let mut media = mount(&mut fs, SinkStatus::default()).unwrap();
    assert_eq!(media.image(), expected);
    // The same last-wins rule applies after output has been poisoned.
    fs.io.0.borrow_mut().write_budget = Some(0);
    media.write_batch(&[(0, [7; SECTOR_LEN]), (0, [8; SECTOR_LEN])]);
    assert!(media.status().failure().is_some());
    media.write_batch(&[(1, [9; SECTOR_LEN]), (1, [10; SECTOR_LEN])]);
    assert_eq!(media.read_sector(0), [8; SECTOR_LEN]);
    assert_eq!(media.read_sector(1), [10; SECTOR_LEN]);
}

#[cfg(unix)]
#[test]
fn symlink_in_another_directory_uses_the_same_writer_lock() {
    let dir = TempDir::new();
    let path = dir.journal();
    let alias_dir = TempDir::new();
    let alias = alias_dir.journal();
    let mut media = real_mount(&path).unwrap();
    media.write_batch(&batch(7));
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(matches!(real_mount(&alias), Err(OpenError::Host(_))));
    assert!(!alias.with_extension("upfj.lock").exists());
    drop(media);
    let mut media = real_mount(&alias).unwrap();
    assert_image(&media, 7);
    assert!(matches!(real_mount(&path), Err(OpenError::Host(_))));
    media.write_batch(&batch(9));
    media.finish().unwrap();
    let media = real_mount(&path).unwrap();
    assert_image(&media, 9);
    // The post-open path comparison also needs a canonical parent for new files.
    let parent_alias = alias_dir.0.join("parent");
    std::os::unix::fs::symlink(&dir.0, &parent_alias).unwrap();
    let fresh = parent_alias.join("fresh.upfj");
    let _media = real_mount(&fresh).unwrap();
    assert!(matches!(
        real_mount(&dir.0.join("fresh.upfj")),
        Err(OpenError::Host(_))
    ));
}

struct HeaderFailureIo {
    output: Box<dyn JournalIo>,
    write_budget: Option<usize>,
    fail_sync: bool,
}
impl Read for HeaderFailureIo {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.output.read(bytes)
    }
}
impl Seek for HeaderFailureIo {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.output.seek(position)
    }
}
impl Write for HeaderFailureIo {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut length = bytes.len();
        if let Some(remaining) = self.write_budget {
            if remaining == 0 {
                return Err(io::Error::other("injected header write failure"));
            }
            length = length.min(remaining);
        }
        let written = self.output.write(&bytes[..length])?;
        if let Some(remaining) = self.write_budget.as_mut() {
            *remaining -= written;
        }
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}
impl JournalIo for HeaderFailureIo {
    fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.output.metadata()
    }
    fn sync(&mut self) -> io::Result<()> {
        if self.fail_sync {
            return Err(io::Error::other("injected header sync failure"));
        }
        self.output.sync()
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.output.truncate(length)
    }
}
struct HeaderFailureFs {
    write_budget: Option<usize>,
    fail_sync: bool,
}
impl Filesystem for HeaderFailureFs {
    fn lock(&mut self, path: &Path) -> io::Result<Box<dyn Any>> {
        HostFilesystem.lock(path)
    }
    fn open(&mut self, path: &Path) -> io::Result<Option<Box<dyn JournalIo>>> {
        HostFilesystem.open(path)
    }
    fn verify_path(&mut self, path: &Path) -> io::Result<()> {
        HostFilesystem.verify_path(path)
    }
    fn cleanup_creation_temps(
        &mut self,
        path: &Path,
        header: &[u8],
        base_metadata: Option<&std::fs::Metadata>,
    ) -> io::Result<()> {
        HostFilesystem.cleanup_creation_temps(path, header, base_metadata)
    }
    fn create_temp(&mut self, path: &Path) -> io::Result<(Box<dyn JournalIo>, PathBuf)> {
        let (output, path) = HostFilesystem.create_temp(path)?;
        Ok((
            Box::new(HeaderFailureIo {
                output,
                write_budget: self.write_budget,
                fail_sync: self.fail_sync,
            }),
            path,
        ))
    }
    fn publish(&mut self, temporary: &Path, path: &Path) -> io::Result<()> {
        HostFilesystem.publish(temporary, path)
    }
    fn remove_temp(&mut self, temporary: &Path) -> io::Result<()> {
        HostFilesystem.remove_temp(temporary)
    }
    fn sync_directory(&mut self, path: &Path) -> io::Result<()> {
        HostFilesystem.sync_directory(path)
    }
}

#[test]
fn header_failures_leave_no_published_journal_and_allow_retry() {
    for write_budget in (0..HEADER_LEN).map(Some).chain([None]) {
        let dir = TempDir::new();
        let path = dir.journal();
        let mut fs = HeaderFailureFs {
            write_budget,
            fail_sync: write_budget.is_none(),
        };
        let status = SinkStatus::default();
        assert!(matches!(
            JournalMedia::open_with(
                &mut fs,
                &path,
                &[0; 2 * SECTOR_LEN],
                (2 * SECTOR_LEN) as u64,
                status.clone()
            ),
            Err(OpenError::Host(_))
        ));
        assert!(status.failure().is_some());
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_dir(&dir.0).unwrap().count(),
            1,
            "only the stable lock remains"
        );
        real_mount(&path).unwrap().finish().unwrap();
    }
}

#[test]
fn header_publication_never_replaces_an_existing_destination() {
    let dir = TempDir::new();
    let path = dir.journal();
    let (mut output, temporary) = HostFilesystem.create_temp(&path).unwrap();
    output
        .write_all(&Header::new(&[0; SECTOR_LEN], 0).unwrap().encode().unwrap())
        .unwrap();
    barrier(output.as_mut()).unwrap();
    std::fs::write(&path, b"existing destination").unwrap();
    assert_eq!(
        HostFilesystem
            .publish(&temporary, &path)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
    HostFilesystem.remove_temp(&temporary).unwrap();
}

#[test]
fn real_file_test_directories_are_unique_temporary_and_removed() {
    let first = TempDir::new();
    let second = TempDir::new();
    assert_ne!(first.0, second.0);
    assert_eq!(first.0.parent().unwrap(), std::env::temp_dir());
    assert_eq!(second.0.parent().unwrap(), std::env::temp_dir());
    let path = first.0.clone();
    drop(first);
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn dangling_journal_symlink_is_refused_before_locking() {
    let target_dir = TempDir::new();
    let target = target_dir.journal();
    let alias_dir = TempDir::new();
    let alias = alias_dir.journal();
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let status = SinkStatus::default();
    let result = JournalMedia::open(
        &alias,
        &[0; 2 * SECTOR_LEN],
        (2 * SECTOR_LEN) as u64,
        status.clone(),
    );
    assert!(matches!(result, Err(OpenError::Host(_))));
    let failure = status.failure().unwrap();
    assert_eq!(failure.operation, "resolve journal path");
    assert!(failure.message.contains("dangling symlink"));
    assert!(!alias.with_extension("upfj.lock").exists());
    assert!(!target.exists());
    assert!(std::fs::symlink_metadata(&alias)
        .unwrap()
        .file_type()
        .is_symlink());
    let mut writer = real_mount(&target).unwrap();
    writer.write_batch(&batch(7));
    assert!(matches!(real_mount(&alias), Err(OpenError::Host(_))));
    drop(writer);
    let media = real_mount(&alias).unwrap();
    assert_image(&media, 7);
}

#[cfg(unix)]
struct AliasRaceFs {
    target: PathBuf,
    alias: PathBuf,
    writer: Option<JournalMedia>,
    before: Vec<u8>,
}
#[cfg(unix)]
impl Filesystem for AliasRaceFs {
    fn lock(&mut self, path: &Path) -> io::Result<Box<dyn Any>> {
        HostFilesystem.lock(path)
    }
    fn open(&mut self, path: &Path) -> io::Result<Option<Box<dyn JournalIo>>> {
        assert_eq!(path, self.alias);
        // Introduce the alias after resolution and complete another mount
        // before open follows it. No scheduler timing is needed for this race.
        std::os::unix::fs::symlink(&self.target, path)?;
        let mut writer = real_mount(&self.target).unwrap();
        writer.write_batch(&batch(7));
        self.before = std::fs::read(&self.target)?;
        self.writer = Some(writer);
        HostFilesystem.open(path)
    }
    fn verify_path(&mut self, path: &Path) -> io::Result<()> {
        HostFilesystem.verify_path(path)
    }
    fn cleanup_creation_temps(
        &mut self,
        path: &Path,
        header: &[u8],
        base_metadata: Option<&std::fs::Metadata>,
    ) -> io::Result<()> {
        HostFilesystem.cleanup_creation_temps(path, header, base_metadata)
    }
    fn create_temp(&mut self, path: &Path) -> io::Result<(Box<dyn JournalIo>, PathBuf)> {
        HostFilesystem.create_temp(path)
    }
    fn publish(&mut self, temporary: &Path, path: &Path) -> io::Result<()> {
        HostFilesystem.publish(temporary, path)
    }
    fn remove_temp(&mut self, temporary: &Path) -> io::Result<()> {
        HostFilesystem.remove_temp(temporary)
    }
    fn sync_directory(&mut self, path: &Path) -> io::Result<()> {
        HostFilesystem.sync_directory(path)
    }
}

#[cfg(unix)]
#[test]
fn post_open_path_check_refuses_the_stale_alias_resolution_race() {
    let target_dir = TempDir::new();
    let alias_dir = TempDir::new();
    let target = target_dir.journal();
    let expected = canonical_journal_path(&alias_dir.journal()).unwrap();
    assert!(!expected.exists());
    let mut fs = AliasRaceFs {
        target: target.clone(),
        alias: expected.clone(),
        writer: None,
        before: Vec::new(),
    };
    let status = SinkStatus::default();
    let result = JournalMedia::open_with(
        &mut fs,
        &expected,
        &[0; 2 * SECTOR_LEN],
        (2 * SECTOR_LEN) as u64,
        status.clone(),
    );
    assert!(matches!(result, Err(OpenError::Host(_))));
    let failure = status.failure().unwrap();
    assert_eq!(failure.operation, "verify journal path");
    assert!(failure
        .message
        .contains("canonical path changed after locking"));
    assert_eq!(std::fs::read(&target).unwrap(), fs.before);
    let writer = fs.writer.as_mut().unwrap();
    assert_image(writer, 7);
    assert!(writer.status().failure().is_none());
    // The rejected mount released its alias lock but cannot bypass the target's.
    drop(HostFilesystem.lock(&expected).unwrap());
    assert!(matches!(real_mount(&expected), Err(OpenError::Host(_))));
    writer.write_batch(&batch(9));
    drop(fs.writer.take());
    let media = real_mount(&expected).unwrap();
    assert_image(&media, 9);
}

#[test]
fn creation_temporary_cleanup_is_locked_and_synced_and_errors_abort() {
    let mut fs = model_fs(None);
    mount(&mut fs, SinkStatus::default())
        .unwrap()
        .finish()
        .unwrap();
    let bytes = fs.io.0.borrow().durable.clone();
    let mut fs = model_fs(Some(bytes.clone()));
    fs.io.0.borrow_mut().leftovers = 2;
    let media = mount(&mut fs, SinkStatus::default()).unwrap();
    assert_eq!(
        &fs.io.0.borrow().events[..5],
        &[
            Event::Lock,
            Event::Open,
            Event::VerifyPath,
            Event::CleanupTemps,
            Event::DirectorySync,
        ]
    );
    assert_eq!(fs.io.0.borrow().leftovers, 0);
    assert_eq!(*fs.io.0.borrow().bytes.get_ref(), bytes);
    drop(media);
    for (fail_on, operation, remaining) in [
        (3, "remove creation temporaries", 2),
        (4, "directory sync", 0),
    ] {
        let mut fs = model_fs(Some(bytes.clone()));
        {
            let mut model = fs.io.0.borrow_mut();
            model.leftovers = 2;
            model.fail_on = Some(fail_on);
        }
        let status = SinkStatus::default();
        assert!(matches!(
            mount(&mut fs, status.clone()),
            Err(OpenError::Host(_))
        ));
        assert_eq!(status.failure().unwrap().operation, operation);
        assert_eq!(fs.io.0.borrow().leftovers, remaining);
        assert_eq!(fs.io.0.borrow().events.len(), fail_on + 1);
        assert_eq!(*fs.io.0.borrow().bytes.get_ref(), bytes);
        assert!(!fs.locked.get());
    }
}

#[test]
fn unpublished_creation_temporaries_are_removed_without_replaying_them() {
    let dir = TempDir::new();
    let path = dir.journal();
    let (mut output, temporary) = HostFilesystem.create_temp(&path).unwrap();
    let header = Header::new(&[0; 2 * SECTOR_LEN], 0)
        .unwrap()
        .encode()
        .unwrap();
    output.write_all(&header[..17]).unwrap();
    barrier(output.as_mut()).unwrap();
    drop(output);
    let second = path.with_file_name("disk.upfj.create-900.tmp");
    std::fs::write(&second, header).unwrap();
    let unrelated = path.with_file_name("other.upfj.create-0.tmp");
    let backup = path.with_file_name("disk.upfj.create-0.tmp.backup");
    std::fs::write(&unrelated, b"other journal").unwrap();
    std::fs::write(&backup, b"backup").unwrap();
    assert!(!path.exists());
    let media = real_mount(&path).unwrap();
    assert_eq!(media.image(), [0; 2 * SECTOR_LEN]);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), HEADER_LEN as u64);
    assert!(!temporary.exists());
    assert!(!second.exists());
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"other journal");
    assert_eq!(std::fs::read(&backup).unwrap(), b"backup");
}

#[cfg(unix)]
#[test]
fn creation_temp_hard_link_with_commands_is_preserved_without_changing_history() {
    use std::os::unix::fs::MetadataExt;
    let dir = TempDir::new();
    let path = dir.journal();
    let mut media = real_mount(&path).unwrap();
    media.write_batch(&batch(7));
    media.finish().unwrap();
    let before = std::fs::read(&path).unwrap();
    let temporary = path.with_file_name("disk.upfj.create-27.tmp");
    std::fs::hard_link(&path, &temporary).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().nlink(), 2);
    let mut media = real_mount(&path).unwrap();
    assert_image(&media, 7);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read(&temporary).unwrap(), before);
    assert_eq!(std::fs::metadata(&path).unwrap().nlink(), 2);
    media.write_batch(&batch(9));
    media.finish().unwrap();
    let media = real_mount(&path).unwrap();
    assert_image(&media, 9);
}

#[cfg(unix)]
#[test]
fn cleanup_preserves_creation_temp_symlinks_to_journals() {
    let dir = TempDir::new();
    let path = dir.journal();
    let target = dir.0.join("other.upfj");
    drop(real_mount(&target).unwrap());
    let before = std::fs::read(&target).unwrap();
    let temporary = dir.0.join("disk.upfj.create-7.tmp");
    std::os::unix::fs::symlink("other.upfj", &temporary).unwrap();
    let _media = real_mount(&path).unwrap();
    assert!(std::fs::symlink_metadata(&temporary)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&target).unwrap(), before);
}

#[test]
fn cleanup_preserves_creation_temp_files_with_journal_commands() {
    let dir = TempDir::new();
    let path = dir.journal();
    let other_path = dir.0.join("other.upfj");
    let mut other = real_mount(&other_path).unwrap();
    other.write_batch(&batch(9));
    other.finish().unwrap();
    let temporary = dir.0.join("disk.upfj.create-7.tmp");
    std::fs::rename(&other_path, &temporary).unwrap();
    let before = std::fs::read(&temporary).unwrap();
    let _media = real_mount(&path).unwrap();
    assert_eq!(std::fs::read(&temporary).unwrap(), before);
}

#[test]
fn cleanup_preserves_foreign_creation_temp_entries() {
    let dir = TempDir::new();
    let path = dir.journal();
    let temporary = dir.0.join("disk.upfj.create-7.tmp");
    std::fs::write(&temporary, b"foreign file").unwrap();
    let directory = dir.0.join("disk.upfj.create-8.tmp");
    std::fs::create_dir(&directory).unwrap();
    let _media = real_mount(&path).unwrap();
    assert_eq!(std::fs::read(&temporary).unwrap(), b"foreign file");
    assert!(directory.is_dir());
}

#[test]
fn cleanup_preserves_foreign_entries_while_removing_creation_header_prefixes() {
    let dir = TempDir::new();
    let path = dir.journal();
    let header = Header::new(&[0; 2 * SECTOR_LEN], 0)
        .unwrap()
        .encode()
        .unwrap();
    let foreign = dir.0.join("disk.upfj.create-900.tmp");
    std::fs::write(&foreign, b"foreign file").unwrap();
    for length in 0..=header.len() {
        let temporary = dir.0.join(format!("disk.upfj.create-{length}.tmp"));
        std::fs::write(temporary, &header[..length]).unwrap();
    }
    let _media = real_mount(&path).unwrap();
    for length in 0..=header.len() {
        assert!(!dir
            .0
            .join(format!("disk.upfj.create-{length}.tmp"))
            .exists());
    }
    assert_eq!(std::fs::read(foreign).unwrap(), b"foreign file");
}

#[test]
fn journal_open_refuses_creation_temp_names_before_locking() {
    let dir = TempDir::new();
    for name in [
        "j.upfj.create-7.tmp",
        "j.upfj.create-.tmp",
        "j.upfj.create-text.tmp",
    ] {
        let path = dir.0.join(name);
        let error = match real_mount(&path) {
            Ok(_) => panic!("reserved journal name mounted"),
            Err(error) => error.to_string(),
        };
        assert_eq!(error, "reserved sidecar path");
        assert!(!path.exists());
        assert!(!dir.0.join(format!("{name}.lock")).exists());
    }
}

#[cfg(unix)]
#[test]
fn journal_open_refuses_creation_temp_names_through_symlinks() {
    for reserved_link in [false, true] {
        let dir = TempDir::new();
        let normal = dir.0.join("other.upfj");
        real_mount(&normal).unwrap().finish().unwrap();
        let reserved = dir.0.join("disk.upfj.create-7.tmp");
        let path = if reserved_link {
            std::os::unix::fs::symlink(&normal, &reserved).unwrap();
            &reserved
        } else {
            std::fs::rename(&normal, &reserved).unwrap();
            std::os::unix::fs::symlink(&reserved, &normal).unwrap();
            &normal
        };
        let before = std::fs::read(path).unwrap();
        let error = match real_mount(path) {
            Ok(_) => panic!("reserved journal alias mounted"),
            Err(error) => error.to_string(),
        };
        assert_eq!(error, "reserved sidecar path");
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}

#[test]
fn cleanup_only_removes_decimal_creation_names() {
    let dir = TempDir::new();
    let path = dir.journal();
    let header = Header::new(&[0; 2 * SECTOR_LEN], 0)
        .unwrap()
        .encode()
        .unwrap();
    let preserved = [
        "disk.upfj.create-.tmp",
        "disk.upfj.create-word.tmp",
        "disk.upfj.create-x.upfj.create-0.tmp",
        "disk.upfj.create-1.tmp.bak",
        "disk.upfj.create-+1.tmp",
        "disk.upfj.create-١.tmp",
    ];
    for name in preserved {
        std::fs::write(dir.0.join(name), header).unwrap();
    }
    std::fs::write(dir.0.join("disk.upfj.create-7.tmp"), header).unwrap();
    let _media = real_mount(&path).unwrap();
    for name in preserved {
        assert_eq!(std::fs::read(dir.0.join(name)).unwrap(), header, "{name}");
    }
    assert!(!dir.0.join("disk.upfj.create-7.tmp").exists());
}

#[test]
fn nested_creation_names_are_refused_before_locking() {
    let dir = TempDir::new();
    for name in ["disk.upfj.create-x.upfj", "disk.create-", ".create-hidden"] {
        let path = dir.0.join(name);
        let error = match real_mount(&path) {
            Ok(_) => panic!("nested creation name mounted"),
            Err(error) => error.to_string(),
        };
        assert_eq!(error, "reserved sidecar path");
        assert!(!path.exists());
        assert!(!dir.0.join(format!("{name}.lock")).exists());
    }
}

struct VanishingCreationIo {
    stage: &'static str,
}

impl CreationTempIo for VanishingCreationIo {
    fn metadata(&mut self, path: &Path) -> io::Result<std::fs::Metadata> {
        if self.stage == "metadata" {
            std::fs::remove_file(path)?;
        }
        HostCreationTempIo.metadata(path)
    }
    fn open(&mut self, path: &Path) -> io::Result<File> {
        if self.stage == "open" {
            std::fs::remove_file(path)?;
        }
        let file = HostCreationTempIo.open(path)?;
        if self.stage == "final-metadata" {
            std::fs::remove_file(path)?;
        }
        Ok(file)
    }
    fn remove(&mut self, path: &Path) -> io::Result<()> {
        if self.stage == "remove" {
            std::fs::remove_file(path)?;
        }
        HostCreationTempIo.remove(path)
    }
}

#[test]
fn cleanup_disappearing_entries_are_already_gone() {
    for stage in ["metadata", "open", "final-metadata", "remove"] {
        let dir = TempDir::new();
        let path = dir.0.join("disk.upfj.create-7.tmp");
        let header = Header::new(&[0; 2 * SECTOR_LEN], 0)
            .unwrap()
            .encode()
            .unwrap();
        std::fs::write(&path, header).unwrap();
        cleanup_creation_temp(&mut VanishingCreationIo { stage }, &path, &header).unwrap();
        assert!(!path.exists(), "{stage}");
    }
}

#[cfg(unix)]
struct ReplacedCreationIo {
    foreign: PathBuf,
    restore_before_return: bool,
}

#[cfg(unix)]
impl CreationTempIo for ReplacedCreationIo {
    fn metadata(&mut self, path: &Path) -> io::Result<std::fs::Metadata> {
        HostCreationTempIo.metadata(path)
    }
    fn open(&mut self, path: &Path) -> io::Result<File> {
        let saved = path.with_extension("saved");
        if self.restore_before_return {
            std::fs::rename(path, &saved)?;
            std::fs::hard_link(&self.foreign, path)?;
            let file = HostCreationTempIo.open(path)?;
            std::fs::remove_file(path)?;
            std::fs::rename(saved, path)?;
            Ok(file)
        } else {
            let file = HostCreationTempIo.open(path)?;
            std::fs::rename(path, saved)?;
            std::fs::hard_link(&self.foreign, path)?;
            Ok(file)
        }
    }
    fn remove(&mut self, path: &Path) -> io::Result<()> {
        HostCreationTempIo.remove(path)
    }
}

#[cfg(unix)]
#[test]
fn cleanup_checks_the_opened_file_identity() {
    let dir = TempDir::new();
    let path = dir.0.join("disk.upfj.create-7.tmp");
    let foreign = dir.0.join("foreign");
    let header = Header::new(&[0; 2 * SECTOR_LEN], 0)
        .unwrap()
        .encode()
        .unwrap();
    std::fs::write(&path, header).unwrap();
    std::fs::write(&foreign, header).unwrap();
    cleanup_creation_temp(
        &mut ReplacedCreationIo {
            foreign,
            restore_before_return: true,
        },
        &path,
        &header,
    )
    .unwrap();
    assert!(
        path.exists(),
        "a different open handle must not authorize unlinking"
    );
}

#[cfg(unix)]
#[test]
fn cleanup_checks_identity_again_before_unlinking() {
    use std::os::unix::fs::MetadataExt;
    let dir = TempDir::new();
    let path = dir.0.join("disk.upfj.create-7.tmp");
    let foreign = dir.0.join("foreign");
    let header = Header::new(&[0; 2 * SECTOR_LEN], 0)
        .unwrap()
        .encode()
        .unwrap();
    std::fs::write(&path, header).unwrap();
    std::fs::write(&foreign, header).unwrap();
    cleanup_creation_temp(
        &mut ReplacedCreationIo {
            foreign: foreign.clone(),
            restore_before_return: false,
        },
        &path,
        &header,
    )
    .unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().ino(),
        std::fs::metadata(foreign).unwrap().ino()
    );
}

#[cfg(unix)]
#[test]
fn creation_temp_open_refuses_symlinks() {
    let dir = TempDir::new();
    let target = dir.0.join("foreign");
    std::fs::write(&target, b"header prefix").unwrap();
    let link = dir.0.join("disk.upfj.create-7.tmp");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(HostCreationTempIo.open(&link).is_err());
}

#[derive(Default)]
struct RetentionIo {
    calls: Vec<&'static str>,
    fail: Option<&'static str>,
    publish_then_fail: bool,
}
impl RetentionIo {
    fn step(&mut self, role: &'static str) -> io::Result<()> {
        self.calls.push(role);
        if self.fail == Some(role) {
            return Err(io::Error::other("injected retention failure"));
        }
        Ok(())
    }
}
impl crate::offline::ToolIo for RetentionIo {
    fn write_temporary(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        if self.fail == Some("temporary write") {
            file.write_all(&bytes[..bytes.len() / 2])?;
        }
        self.step("temporary write")?;
        crate::offline::HostIo.write_temporary(file, bytes)
    }
    fn sync(&mut self, file: &File, role: &'static str) -> io::Result<()> {
        assert_eq!(
            file.metadata()?.is_dir(),
            role == "publication directory sync"
        );
        self.step(role)?;
        crate::offline::HostIo.sync(file, role)
    }
    fn publish_temporary(&mut self, temp: &Path, path: &Path, replace: bool) -> io::Result<()> {
        assert!(replace);
        if self.publish_then_fail {
            crate::offline::HostIo.publish_temporary(temp, path, replace)?;
        }
        self.step("replace")?;
        crate::offline::HostIo.publish_temporary(temp, path, replace)
    }
}

fn five_generations(path: &Path) {
    let mut disk = real_mount(path).unwrap();
    for id in 1..=5 {
        // Sector one is changed only in history that retention must fold away.
        disk.write_batch(&[(0, [id; 512])]);
        if id == 1 {
            disk.write_batch(&[(1, [0xA7; 512])]);
        }
        if id < 5 {
            disk.checkpoint().unwrap();
        }
    }
    disk.finish().unwrap();
}

#[test]
fn retention_keeps_two_closed_states_with_anchor_ids_reasons_and_new_sequences() {
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let base = [0; 1024];
    let original = std::fs::read(&path).unwrap();
    let old = scan(&original, &base, 1024);
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    // Drop adds no generation: pruning itself is tested at the existing boundary.
    disk.prune_with(Rc::new(RefCell::new(RetentionIo::default())))
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let after = scan(&bytes, &base, 1024);
    assert_eq!(after.report.status, Status::Valid);
    assert!(!after.report.needs_recovery());
    assert_eq!(after.image, old.image);
    assert_eq!(after.report.header.as_ref().unwrap().anchor_generation, 4);
    assert_eq!(
        after.report.header.as_ref().unwrap().base_digest,
        Header::new(&base, 0).unwrap().base_digest
    );
    assert_eq!(
        after
            .report
            .generations
            .iter()
            .map(|g| (g.id, g.reason))
            .collect::<Vec<_>>(),
        [
            (4, ClosingReason::RetentionAnchor),
            (5, ClosingReason::SessionEnd)
        ]
    );
    assert_eq!(bytes.len(), HEADER_LEN + 7 * FRAME_LEN);
    for (index, bytes) in bytes[HEADER_LEN..]
        .as_chunks::<FRAME_LEN>()
        .0
        .iter()
        .enumerate()
    {
        assert_eq!(Frame::decode(bytes).unwrap().sequence, index as u64 + 1);
    }
    for generation in &after.report.generations {
        let old_generation = old
            .report
            .generations
            .iter()
            .find(|g| g.id == generation.id)
            .unwrap();
        assert_eq!(
            scan(&original[..old_generation.end_offset], &base, 1024).image,
            scan(&bytes[..generation.end_offset], &base, 1024).image
        );
    }
    drop(disk);
    assert_eq!(real_mount(&path).unwrap().image(), old.image.unwrap());
}

#[test]
fn retention_runs_after_checkpoint_and_finish_and_continues_on_replaced_inode() {
    let dir = TempDir::new();
    let path = dir.journal();
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    for id in 1..=5 {
        disk.write_batch(&[(0, [id; 512])]);
        if id < 5 {
            disk.checkpoint().unwrap();
        }
    }
    disk.finish().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let result = scan(&bytes, &[0; 1024], 1024);
    assert_eq!(result.report.status, Status::Valid);
    assert_eq!(
        result
            .report
            .generations
            .iter()
            .map(|g| g.id)
            .collect::<Vec<_>>(),
        [4, 5]
    );
    assert_eq!(
        result.report.generations[1].reason,
        ClosingReason::SessionEnd
    );
    let mounted = real_mount(&path).unwrap();
    assert_eq!(mounted.read_sector(0), [5; 512]);
}

#[test]
fn retention_failures_report_maintenance_separately_and_keep_old_or_new_valid_files() {
    for fail in [
        "temporary write",
        "temporary sync",
        "replace",
        "publication directory sync",
    ] {
        for publish_then_fail in [false, true] {
            if publish_then_fail && fail != "replace" {
                continue;
            }
            let dir = TempDir::new();
            let path = dir.journal();
            five_generations(&path);
            let mut disk = real_mount(&path).unwrap().with_retention(2);
            let status = disk.status();
            let mut expected_old = std::fs::read(&path).unwrap();
            expected_old.extend(
                Frame {
                    sequence: disk.sequence() + 1,
                    generation: 6,
                    record: Record::GenerationEnd {
                        reason: ClosingReason::Checkpoint,
                    },
                }
                .encode()
                .unwrap(),
            );
            let io = Rc::new(RefCell::new(RetentionIo {
                fail: Some(fail),
                publish_then_fail,
                ..RetentionIo::default()
            }));
            disk.close_with(ClosingReason::Checkpoint, io.clone())
                .unwrap();
            let error = status.maintenance_failure().unwrap();
            let visible = std::fs::read(&path).unwrap();
            let result = scan(&visible, &[0; 1024], 1024);
            assert_eq!(result.report.status, Status::Valid);
            assert!(!result.report.needs_recovery());
            assert_eq!(result.image.as_deref(), Some(disk.image().as_slice()));
            let uncertain = matches!(fail, "replace" | "publication directory sync");
            assert_eq!(status.maintenance_failure().unwrap().uncertain, uncertain);
            assert!(error.to_string().contains(if uncertain {
                "uncertain"
            } else {
                "before publication"
            }));
            assert_eq!(status.failure().is_some(), uncertain);
            assert!(!disk.write_protected());
            let new = fail == "publication directory sync" || publish_then_fail;
            assert_eq!(result.report.generations.len(), if new { 2 } else { 6 });
            assert_eq!(result.report.generations.last().unwrap().id, 6);
            if !new {
                assert_eq!(visible, expected_old);
            } else {
                let before = scan(&expected_old, &[0; 1024], 1024);
                assert_eq!(result.image, before.image);
                assert_eq!(result.report.header.as_ref().unwrap().anchor_generation, 5);
            }
            if !uncertain {
                io.borrow_mut().fail = None;
                disk.write_batch(&[(0, [0xB5; 512])]);
                disk.close_with(ClosingReason::SessionEnd, io.clone())
                    .unwrap();
                assert!(status.maintenance_failure().is_none());
                assert_eq!(
                    scan(&std::fs::read(&path).unwrap(), &[0; 1024], 1024)
                        .image
                        .as_deref(),
                    Some(disk.image().as_slice())
                );
            } else {
                let before = std::fs::read(&path).unwrap();
                disk.write_batch(&[(0, [0xB5; 512])]);
                assert_eq!(disk.read_sector(0), [0xB5; 512]);
                assert!(disk.checkpoint().is_err());
                assert_eq!(std::fs::read(&path).unwrap(), before);
            }
            drop(disk);
            assert!(real_mount(&path).is_ok());
        }
    }
}

#[test]
fn retention_barriers_follow_complete_verified_temporary_and_namespace_is_isolated() {
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let creation = path.with_file_name("disk.upfj.create-999.tmp");
    let retention = path.with_file_name("disk.upfj.retain-999.tmp");
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    std::fs::write(
        &creation,
        Header::new(&[0; 1024], 0).unwrap().encode().unwrap(),
    )
    .unwrap();
    let retention_bytes = Header::new(&[0; 1024], 0).unwrap().encode().unwrap();
    std::fs::write(&retention, retention_bytes).unwrap();
    let io = Rc::new(RefCell::new(RetentionIo::default()));
    disk.close_with(ClosingReason::Checkpoint, io.clone())
        .unwrap();
    assert_eq!(
        &io.borrow().calls[io.borrow().calls.len() - 3..],
        ["temporary sync", "replace", "publication directory sync"]
    );
    assert!(io.borrow().calls[..io.borrow().calls.len() - 3]
        .iter()
        .all(|s| *s == "temporary write"));
    assert!(creation.exists());
    assert!(retention.exists());
    drop(disk);
    let disk = real_mount(&path).unwrap();
    assert!(!creation.exists());
    assert!(!retention.exists());
    drop(disk);
    assert!(JournalMedia::open(&retention, &[0; 1024], 1024, SinkStatus::default()).is_err());
}

#[test]
fn retention_never_runs_for_open_or_poisoned_generation() {
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    disk.write_batch(&[(0, [0xB5; 512])]);
    let before = std::fs::read(&path).unwrap();
    let io = Rc::new(RefCell::new(RetentionIo::default()));
    assert!(disk.prune_with(io.clone()).is_err());
    assert!(io.borrow().calls.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    disk.status()
        .latch_host_error("injected", "failed command".into());
    assert!(disk
        .close_with(ClosingReason::Checkpoint, io.clone())
        .is_err());
    disk.prune_with(io.clone()).unwrap();
    assert!(io.borrow().calls.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn retention_encodes_empty_anchor_and_preserves_later_recovered_reason() {
    let dir = TempDir::new();
    let path = dir.journal();
    let mut disk = real_mount(&path).unwrap();
    disk.write_batch(&[(0, [1; 512])]);
    disk.checkpoint().unwrap();
    disk.write_batch(&[(0, [0; 512])]);
    disk.checkpoint().unwrap();
    disk.checkpoint().unwrap();
    disk.write_batch(&[(1, [0xA7; 512])]);
    drop(disk);
    let disk = real_mount(&path).unwrap().with_retention(3);
    disk.finish().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let scan = scan(&bytes, &[0; 1024], 1024);
    assert_eq!(scan.report.status, Status::Valid);
    assert_eq!(scan.report.header.unwrap().anchor_generation, 3);
    assert_eq!(
        Frame::decode(&bytes[HEADER_LEN..HEADER_LEN + FRAME_LEN])
            .unwrap()
            .record,
        Record::CommandEnd { sectors: 0 }
    );
    assert_eq!(
        scan.report
            .generations
            .iter()
            .map(|g| (g.id, g.reason))
            .collect::<Vec<_>>(),
        [
            (3, ClosingReason::RetentionAnchor),
            (4, ClosingReason::Recovered),
            (5, ClosingReason::SessionEnd)
        ]
    );
    assert_eq!(real_mount(&path).unwrap().read_sector(1), [0xA7; 512]);
}

#[test]
fn retention_verifies_each_state_before_sync_even_when_final_image_matches() {
    struct AlterAnchor;
    impl crate::offline::ToolIo for AlterAnchor {
        fn write_temporary(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
            let mut bytes = bytes.to_vec();
            if bytes.len() == FRAME_LEN {
                let mut frame = Frame::decode(&bytes).unwrap();
                // This sector is overwritten in the next retained generation.
                if frame.generation == 4 {
                    if let Record::Sector {
                        lba: 0,
                        bytes: sector,
                    } = &mut frame.record
                    {
                        sector.fill(0xEE);
                        bytes = frame.encode().unwrap().to_vec();
                    }
                }
            }
            crate::offline::HostIo.write_temporary(file, &bytes)
        }
        fn sync(&mut self, _: &File, _: &'static str) -> io::Result<()> {
            panic!("unverified retention synchronized")
        }
        fn publish_temporary(&mut self, _: &Path, _: &Path, _: bool) -> io::Result<()> {
            panic!("unverified retention published")
        }
    }
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let original = std::fs::read(&path).unwrap();
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    assert!(disk
        .prune_with(Rc::new(RefCell::new(AlterAnchor)))
        .unwrap_err()
        .to_string()
        .contains("state verification"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(disk.status().failure().is_none());
}

#[test]
fn retention_fails_every_temporary_write_without_changing_durable_closed_journal() {
    struct FailWrite {
        position: usize,
        count: usize,
    }
    impl crate::offline::ToolIo for FailWrite {
        fn write_temporary(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
            let fail = self.position == self.count;
            self.count += 1;
            if fail {
                file.write_all(&bytes[..bytes.len() / 2])?;
                return Err(io::Error::other("injected partial rewrite"));
            }
            crate::offline::HostIo.write_temporary(file, bytes)
        }
        fn sync(&mut self, _: &File, _: &'static str) -> io::Result<()> {
            panic!("failed rewrite synchronized")
        }
    }
    // Header, two anchor sectors, two anchor markers, and three later frames.
    for position in 0..8 {
        let dir = TempDir::new();
        let path = dir.journal();
        five_generations(&path);
        let original = std::fs::read(&path).unwrap();
        let mut disk = real_mount(&path).unwrap().with_retention(2);
        assert!(disk
            .prune_with(Rc::new(RefCell::new(FailWrite { position, count: 0 })))
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!disk.status().maintenance_failure().unwrap().uncertain);
        assert!(disk.status().failure().is_none());
    }
}

#[test]
fn retention_skips_occupied_temporary_aliases_and_preserves_offline_backups() {
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let base = dir.0.join("base.img");
    std::fs::write(&base, [0; 1024]).unwrap();
    let original = std::fs::read(&path).unwrap();
    let journal_alias = dir.0.join("disk.upfj.retain-0.tmp");
    let base_alias = dir.0.join("disk.upfj.retain-1.tmp");
    let backup = dir.0.join("offline-backup.upfj");
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    std::fs::hard_link(&path, &journal_alias).unwrap();
    std::fs::hard_link(&base, &base_alias).unwrap();
    std::fs::write(&backup, &original).unwrap();
    disk.prune_with(Rc::new(RefCell::new(RetentionIo::default())))
        .unwrap();
    assert_eq!(std::fs::read(journal_alias).unwrap(), original);
    assert_eq!(std::fs::read(base_alias).unwrap(), [0; 1024]);
    assert_eq!(std::fs::read(base).unwrap(), [0; 1024]);
    assert_eq!(std::fs::read(backup).unwrap(), original);
    assert_ne!(std::fs::read(path).unwrap(), original);
}

#[test]
fn retention_does_not_rewrite_at_exact_limit() {
    let dir = TempDir::new();
    let path = dir.journal();
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    disk.write_batch(&[(0, [1; 512])]);
    disk.checkpoint().unwrap();
    disk.write_batch(&[(0, [2; 512])]);
    let mut expected = std::fs::read(&path).unwrap();
    expected.extend(
        Frame {
            sequence: disk.sequence() + 1,
            generation: 2,
            record: Record::GenerationEnd {
                reason: ClosingReason::Checkpoint,
            },
        }
        .encode()
        .unwrap(),
    );
    let io = Rc::new(RefCell::new(RetentionIo::default()));
    disk.close_with(ClosingReason::Checkpoint, io.clone())
        .unwrap();
    assert!(io.borrow().calls.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), expected);
}

#[test]
fn retention_counts_the_generation_sealed_on_mount() {
    let dir = TempDir::new();
    let path = dir.journal();
    let mut disk = real_mount(&path).unwrap();
    disk.checkpoint().unwrap();
    disk.write_batch(&[(0, [2; 512])]);
    drop(disk);
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    assert!(disk.mount_report().unsealed_tip.is_some());
    disk.checkpoint().unwrap();
    let scan = scan(&std::fs::read(&path).unwrap(), &[0; 1024], 1024);
    assert_eq!(scan.report.header.unwrap().anchor_generation, 2);
    assert_eq!(scan.report.generations.len(), 2);
}

#[test]
fn retention_checks_whole_history_metadata_before_sync() {
    struct ChangeReason;
    impl crate::offline::ToolIo for ChangeReason {
        fn write_temporary(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
            if bytes.len() == FRAME_LEN {
                let mut frame = Frame::decode(bytes).unwrap();
                if frame.generation == 5 && matches!(frame.record, Record::GenerationEnd { .. }) {
                    frame.record = Record::GenerationEnd {
                        reason: ClosingReason::Checkpoint,
                    };
                    return file.write_all(&frame.encode().unwrap());
                }
            }
            file.write_all(bytes)
        }
        fn sync(&mut self, _: &File, _: &'static str) -> io::Result<()> {
            panic!("changed history reached sync")
        }
    }
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let original = std::fs::read(&path).unwrap();
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    assert!(disk
        .prune_with(Rc::new(RefCell::new(ChangeReason)))
        .unwrap_err()
        .to_string()
        .contains("history verification failed"));
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn failed_retention_removes_its_temporary_and_preserves_other_entries() {
    for fail in [
        "temporary write",
        "temporary sync",
        "replace",
        "publication directory sync",
    ] {
        let dir = TempDir::new();
        let path = dir.journal();
        five_generations(&path);
        let mut disk = real_mount(&path).unwrap().with_retention(2);
        let other = dir.0.join("other.upfj.retain-0.tmp");
        std::fs::write(&other, b"other journal").unwrap();
        assert!(disk
            .prune_with(Rc::new(RefCell::new(RetentionIo {
                fail: Some(fail),
                ..Default::default()
            })))
            .is_err());
        assert!(std::fs::read_dir(&dir.0)
            .unwrap()
            .all(|entry| !JournalMedia::is_retention_temp(&path, &entry.unwrap().path())));
        assert_eq!(std::fs::read(other).unwrap(), b"other journal");
    }
}

#[test]
fn acknowledged_command_after_retention_survives_drop_without_close() {
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    disk.prune_with(Rc::new(RefCell::new(RetentionIo::default())))
        .unwrap();
    disk.write_batch(&[(0, [0xCE; 512])]);
    assert!(disk.status().failure().is_none());
    let confirmed = disk.confirmed_sequence();
    drop(disk);
    let disk = real_mount(&path).unwrap();
    assert_eq!(disk.read_sector(0), [0xCE; 512]);
    assert_eq!(
        disk.mount_report()
            .unsealed_tip
            .as_ref()
            .unwrap()
            .generation,
        6
    );
    assert_eq!(disk.mount_report().last_durable_sequence, confirmed);
}

#[cfg(unix)]
#[test]
fn retention_preserves_exact_mode_before_writing_and_publication() {
    use std::os::unix::fs::PermissionsExt;
    struct ModeIo {
        created: bool,
        written: bool,
    }
    impl crate::offline::ToolIo for ModeIo {
        fn create_temporary(
            &mut self,
            path: &Path,
            marker: &str,
            permissions: Option<&std::fs::Permissions>,
        ) -> io::Result<(File, PathBuf)> {
            assert_eq!(permissions.unwrap().mode() & 0o7777, 0o664);
            let result = crate::offline::HostIo.create_temporary(path, marker, permissions)?;
            // Emulate a restrictive umask without changing process-global state.
            result
                .0
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
            self.created = true;
            Ok(result)
        }
        fn write_temporary(&mut self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
            assert_eq!(file.metadata()?.permissions().mode() & 0o7777, 0o664);
            self.written = true;
            file.write_all(bytes)
        }
        fn sync(&mut self, file: &File, role: &'static str) -> io::Result<()> {
            if role == "temporary sync" {
                assert_eq!(file.metadata()?.permissions().mode() & 0o7777, 0o664);
            }
            platform_sync(file)
        }
    }
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    let io = Rc::new(RefCell::new(ModeIo {
        created: false,
        written: false,
    }));
    disk.prune_with(io.clone()).unwrap();
    assert!(io.borrow().created && io.borrow().written);
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777,
        0o664
    );
}

#[cfg(unix)]
#[test]
fn mount_removes_only_own_decimal_regular_retention_leftovers() {
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let old = std::fs::read(&path).unwrap();
    for (name, bytes) in [
        ("disk.upfj.retain-0.tmp", old.as_slice()),
        ("disk.upfj.retain-1.tmp", b"partial replacement"),
        ("disk.upfj.retain-2.tmp", b""),
    ] {
        std::fs::write(dir.0.join(name), bytes).unwrap();
    }
    let link = dir.0.join("disk.upfj.retain-3.tmp");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    for name in [
        "disk.upfj.retain-.tmp",
        "disk.upfj.retain-word.tmp",
        "disk.upfj.retain-4.tmp.bak",
        "other.upfj.retain-0.tmp",
    ] {
        std::fs::write(dir.0.join(name), b"foreign").unwrap();
    }
    let directory = dir.0.join("disk.upfj.retain-5.tmp");
    std::fs::create_dir(&directory).unwrap();
    let mounted = real_mount(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), old);
    assert_eq!(mounted.read_sector(0), [5; 512]);
    for id in 0..3 {
        assert!(!dir.0.join(format!("disk.upfj.retain-{id}.tmp")).exists());
    }
    assert!(std::fs::symlink_metadata(link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(directory.is_dir());
    for name in [
        "disk.upfj.retain-.tmp",
        "disk.upfj.retain-word.tmp",
        "disk.upfj.retain-4.tmp.bak",
        "other.upfj.retain-0.tmp",
    ] {
        assert_eq!(std::fs::read(dir.0.join(name)).unwrap(), b"foreign");
    }
}

#[cfg(unix)]
#[test]
fn retention_cleanup_checks_both_identities_and_disappearing_entries() {
    for restore in [false, true] {
        let dir = TempDir::new();
        let candidate = dir.0.join("disk.upfj.retain-0.tmp");
        let foreign = dir.0.join("foreign");
        std::fs::write(&candidate, b"replacement").unwrap();
        std::fs::write(&foreign, b"foreign").unwrap();
        cleanup_retention_temp(
            &mut ReplacedCreationIo {
                foreign: foreign.clone(),
                restore_before_return: restore,
            },
            &candidate,
        )
        .unwrap();
        assert!(candidate.exists());
        assert_eq!(std::fs::read(foreign).unwrap(), b"foreign");
    }
    for stage in ["metadata", "open", "final-metadata", "remove"] {
        let dir = TempDir::new();
        let candidate = dir.0.join("disk.upfj.retain-0.tmp");
        std::fs::write(&candidate, b"replacement").unwrap();
        cleanup_retention_temp(&mut VanishingCreationIo { stage }, &candidate).unwrap();
        assert!(!candidate.exists());
    }
}

#[cfg(unix)]
#[test]
fn cleanup_skips_base_identity_in_both_temporary_namespaces() {
    for marker in ["create", "retain"] {
        let dir = TempDir::new();
        let path = dir.journal();
        let base = dir.0.join("base.img");
        let bytes = if marker == "create" {
            Header::new(&[0; 1024], 0)
                .unwrap()
                .encode()
                .unwrap()
                .to_vec()
        } else {
            vec![0; 1024]
        };
        std::fs::write(&base, &bytes).unwrap();
        let candidate = dir.0.join(format!("disk.upfj.{marker}-0.tmp"));
        let other = dir.0.join(format!("disk.upfj.{marker}-1.tmp"));
        std::fs::hard_link(&base, &candidate).unwrap();
        std::fs::write(&other, &bytes).unwrap();
        let metadata = File::open(&base).unwrap().metadata().unwrap();
        HostFilesystem
            .cleanup_creation_temps(
                &path,
                &Header::new(&[0; 1024], 0).unwrap().encode().unwrap(),
                Some(&metadata),
            )
            .unwrap();
        assert_eq!(std::fs::read(&candidate).unwrap(), bytes, "{marker}");
        assert_eq!(std::fs::read(&base).unwrap(), bytes);
        assert!(
            !other.exists(),
            "protection is identity-based, not content-based"
        );
    }
}

#[cfg(unix)]
#[test]
fn sink_mount_protects_the_actual_base_file_even_under_a_reserved_name() {
    let dir = TempDir::new();
    let path = dir.journal();
    let base = dir.0.join("disk.upfj.retain-0.tmp");
    let orphan = dir.0.join("disk.upfj.retain-1.tmp");
    std::fs::write(&base, [0; 1024]).unwrap();
    std::fs::write(&orphan, b"interrupted rewrite").unwrap();
    let file = File::open(&base).unwrap();
    let metadata = file.metadata().unwrap();
    let disk = JournalMedia::open_with_base_metadata(
        &path,
        &[0; 1024],
        1024,
        SinkStatus::default(),
        &metadata,
    )
    .unwrap();
    assert_eq!(disk.image(), &[0; 1024]);
    assert_eq!(std::fs::read(&base).unwrap(), [0; 1024]);
    assert!(!orphan.exists());
    drop(disk);
    assert!(JournalMedia::open_with_base_metadata(
        &path,
        &[0; 1024],
        1024,
        SinkStatus::default(),
        &metadata
    )
    .is_ok());
    assert!(base.exists());
}

#[test]
fn nvram_write_batch_rejects_immutable_bytes_before_any_output() {
    for index in [0, 13, 15, 64, 511] {
        let base = [0x22; 64];
        let mut fs = model_fs(None);
        let status = SinkStatus::default();
        let mut journal =
            JournalMedia::open_with(&mut fs, Path::new("cmos.upfj"), &base, 64, status.clone())
                .unwrap();
        let before = fs.io.0.borrow().durable.clone();
        fs.io.0.borrow_mut().events.clear();
        let mut block = journal.read_sector(0);
        block[16] = 9;
        block[index] ^= 1;
        journal.write_batch(&[(0, block)]);
        assert_eq!(
            status.failure().unwrap().message,
            "NVRAM immutable bytes or padding"
        );
        assert_eq!(journal.image(), base);
        assert_eq!(fs.io.0.borrow().durable, before);
        assert!(fs.io.0.borrow().events.is_empty(), "invalid byte {index}");
    }
}

#[test]
fn failed_append_rolls_back_or_reports_uncertain_and_recovery_discards_the_tail() {
    for failed_rollback in [false, true] {
        let mut fs = model_fs(None);
        let header = Header::new(&[0; 2 * SECTOR_LEN], 0)
            .unwrap()
            .encode()
            .unwrap();
        let (mut record, _) =
            crate::journal_file::tests::open(&mut fs, Path::new("disk.upfj"), &header, None)
                .unwrap();
        {
            let mut model = fs.io.0.borrow_mut();
            model.events.clear();
            model.write_budget = Some(7);
            // Seek, partial write, failed write, rollback truncate.
            model.fail_on = failed_rollback.then_some(3);
        }
        let error = record.append(&[0xA5; 20]).unwrap_err();
        assert_eq!(
            error.kind,
            if failed_rollback {
                contract::ErrorKind::StorageUncertain
            } else {
                contract::ErrorKind::Storage
            }
        );
        // Previous behavior left 135 bytes for both failures; successful rollback now leaves 128.
        assert_eq!(
            fs.io.0.borrow().bytes.get_ref().len(),
            if failed_rollback { 135 } else { 128 }
        );
        drop(record);
        fs.io.0.borrow_mut().write_budget = None;
        fs.io.0.borrow_mut().fail_on = None;
        let media = mount(&mut fs, SinkStatus::default()).unwrap();
        assert_eq!(
            media.mount_report().discarded_bytes,
            if failed_rollback { 7 } else { 0 }
        );
        assert_eq!(fs.io.0.borrow().bytes.get_ref().len(), HEADER_LEN);
    }
}

#[test]
fn reserved_sidecars_have_one_usage_diagnostic_and_generated_journals_mount() {
    use crate::offline::{Access, JournalTool, ToolError};
    let dir = TempDir::new();
    let bytes = vec![0; 2 * SECTOR_LEN];
    let base_path = dir.0.join("base.img");
    std::fs::write(&base_path, &bytes).unwrap();
    let header = Header::new(&bytes, 0).unwrap();
    for name in [
        "disk.lock",
        "disk.LOCK",
        "disk_lock",
        "disk_LOCK",
        "disk.create-0.tmp",
        "disk.CREATE-7.TMP",
        "disk.retain-1.tmp",
        "disk.RETAIN-9.TMP",
        "disk_7.tmp",
    ] {
        let path = dir.0.join(name);
        std::fs::write(&path, header.encode().unwrap()).unwrap();
        let before = std::fs::read(&path).unwrap();
        let OpenError::Usage(error) = real_mount(&path).err().unwrap() else {
            panic!("expected Usage")
        };
        assert_eq!(error.kind, contract::ErrorKind::Usage);
        assert_eq!(error.message, "reserved sidecar path");
        let error = crate::JournalFile::open(&path, &header, None)
            .err()
            .unwrap();
        assert_eq!(error.kind, contract::ErrorKind::Usage);
        assert_eq!(error.message, "reserved sidecar path");
        for access in [Access::Inspect, Access::Mutate] {
            let ToolError::Config(message) =
                JournalTool::open(&base_path, &path, access).err().unwrap()
            else {
                panic!("expected usage refusal")
            };
            assert_eq!(message, error.message);
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.0.join(format!("{name}.lock")).exists());
    }
    #[cfg(unix)]
    for reserved_link in [false, true] {
        let normal = dir.0.join("alias.upfj");
        let reserved = dir.0.join("alias.LOCK");
        let (link, target) = if reserved_link {
            (&reserved, &normal)
        } else {
            (&normal, &reserved)
        };
        std::fs::write(target, header.encode().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, link).unwrap();
        let OpenError::Usage(error) = real_mount(link).err().unwrap() else {
            panic!("expected Usage")
        };
        assert_eq!(error.message, "reserved sidecar path");
        let ToolError::Config(message) = JournalTool::open(&base_path, link, Access::Inspect)
            .err()
            .unwrap()
        else {
            panic!("expected usage refusal")
        };
        assert_eq!(message, error.message);
        std::fs::remove_file(link).unwrap();
        std::fs::remove_file(target).unwrap();
    }
    let hash = "a".repeat(64);
    for name in [format!("{hash}.upfj"), format!("{hash}-disk-1.upfj")] {
        let path = dir.0.join(name);
        real_mount(&path).unwrap().finish().unwrap();
        let mut tool = JournalTool::open(&base_path, &path, Access::Inspect).unwrap();
        assert_eq!(tool.verify().unwrap().report.status, Status::Valid);
    }
}

#[cfg(unix)]
#[test]
fn journal_creation_keeps_the_original_umask_filtered_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    let reference = dir.0.join("reference");
    File::create(&reference).unwrap();
    let expected = std::fs::metadata(&reference).unwrap().permissions().mode();
    let path = dir.journal();
    real_mount(&path).unwrap().finish().unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode(),
        expected
    );
    std::fs::remove_file(&path).unwrap();
    let record =
        crate::JournalFile::open(&path, &Header::new(&[0; 2 * SECTOR_LEN], 0).unwrap(), None)
            .unwrap();
    JournalMedia::open_record(
        Box::new(record),
        &VerifiedBase::from_bytes(&[0; 2 * SECTOR_LEN]).unwrap(),
        (2 * SECTOR_LEN) as u64,
        SinkStatus::default(),
    )
    .unwrap()
    .finish()
    .unwrap();
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode(),
        expected
    );
}

#[cfg(unix)]
#[test]
fn retention_failed_temporary_rollback_reports_uncertainty_and_allows_reopen_retry() {
    struct ReadOnlyTemporary;
    impl unemu_core_files::Filesystem for ReadOnlyTemporary {
        fn create_new(
            &self,
            path: &Path,
            permissions: Option<&std::fs::Permissions>,
        ) -> io::Result<Box<dyn unemu_core_files::FileIo>> {
            drop(unemu_core_files::HostFilesystem::new().create_new(path, permissions)?);
            // A read-only descriptor rejects both the write and its rollback.
            Ok(Box::new(File::open(path)?))
        }
    }
    let dir = TempDir::new();
    let path = dir.journal();
    five_generations(&path);
    let mut disk = real_mount(&path).unwrap().with_retention(2);
    let status = disk.status();
    let file = disk.file.clone().unwrap();
    disk.inner
        .close_with(ClosingReason::Checkpoint, &mut |_| {
            journal_replacement(file.borrow_mut().replace_with(&ReadOnlyTemporary))
        })
        .unwrap();
    let failure = status.maintenance_failure().unwrap();
    assert!(failure.uncertain, "{failure}");
    assert_eq!(
        failure.message,
        io::Error::from_raw_os_error(libc::EBADF).to_string()
    );
    assert_eq!(
        status.failure().unwrap().operation,
        "journal appends disabled"
    );
    let closed = std::fs::read(&path).unwrap();
    assert_eq!(scan(&closed, &[0; 1024], 1024).report.status, Status::Valid);
    disk.write_batch(&[(0, [0xB5; 512])]);
    assert_eq!(disk.read_sector(0), [0xB5; 512]);
    assert_eq!(std::fs::read(&path).unwrap(), closed);
    drop(file);
    drop(disk);
    let mut retry = real_mount(&path).unwrap().with_retention(2);
    retry.write_batch(&[(0, [0xB5; 512])]);
    retry.finish().unwrap();
    let result = scan(&std::fs::read(&path).unwrap(), &[0; 1024], 1024);
    assert_eq!(result.report.generations.len(), 2);
    assert_eq!(&result.image.unwrap()[..512], &[0xB5; 512]);
}

#[test]
fn shared_creation_header_sync_failure_keeps_the_original_stage_text() {
    use unemu_core_files::{FileIo, Filesystem};
    struct FailSync(Box<dyn FileIo>);
    impl Read for FailSync {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.0.read(bytes)
        }
    }
    impl Write for FailSync {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl Seek for FailSync {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.0.seek(position)
        }
    }
    impl FileIo for FailSync {
        fn metadata(&self) -> io::Result<std::fs::Metadata> {
            self.0.metadata()
        }
        fn set_len(&mut self, length: u64) -> io::Result<()> {
            self.0.set_len(length)
        }
        fn sync_all(&mut self) -> io::Result<()> {
            Err(io::Error::other("injected header sync failure"))
        }
        fn set_permissions(&mut self, permissions: std::fs::Permissions) -> io::Result<()> {
            self.0.set_permissions(permissions)
        }
    }
    struct HeaderSyncFs;
    impl Filesystem for HeaderSyncFs {
        fn create_new(
            &self,
            path: &Path,
            permissions: Option<&std::fs::Permissions>,
        ) -> io::Result<Box<dyn FileIo>> {
            Ok(Box::new(FailSync(
                crate::journal_file::JournalFilesystem.create_new(path, permissions)?,
            )))
        }
    }
    let dir = TempDir::new();
    let path = dir.journal();
    let status = SinkStatus::default();
    let error = JournalMedia::open_path_with(
        &path,
        &VerifiedBase::from_bytes(&[0; 1024]).unwrap(),
        1024,
        status.clone(),
        None,
        None,
        Rc::new(HeaderSyncFs),
    )
    .err()
    .unwrap();
    assert_eq!(status.failure().unwrap().operation, "header sync");
    assert_eq!(
        error.to_string(),
        "header sync: injected header sync failure (last confirmed sequence 0)"
    );
    assert!(!path.exists());
    real_mount(&path).unwrap().finish().unwrap();
}
