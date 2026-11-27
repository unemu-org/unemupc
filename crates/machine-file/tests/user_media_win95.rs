// SPDX-License-Identifier: AGPL-3.0-or-later
//! Local Windows 95 setup and first boots; user-media.md §Windows 95.

#[path = "support/windows.rs"]
mod windows;
use windows::{resume, run, ResumeCopy, Stage, Staged};

/// Setup's disk after its file copy: a copy of a retained stage-2 directory
/// when UNEMUPC_USER_WIN95_STAGE2_DIR names one, else a fresh stage 2. The
/// copy's guard deletes it if a later stage fails.
fn installed_disk() -> Option<(Staged, Option<ResumeCopy>)> {
    match resume("UNEMUPC_USER_WIN95_STAGE2_DIR") {
        Some((disk, copy)) => Some((disk, Some(copy))),
        None => run(Stage::Win95FileCopy, None).map(|disk| (disk, None)),
    }
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN95_DIR and UNEMUPC_USER_DOS_FLOPPY"]
fn user_media_win95_stage1() {
    run(Stage::Win95, None);
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN95_DIR, UNEMUPC_USER_DOS_FLOPPY and UNEMUPC_USER_WIN95_KEY"]
fn user_media_win95_stage2() {
    run(Stage::Win95FileCopy, None);
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN95_DIR, UNEMUPC_USER_DOS_FLOPPY and UNEMUPC_USER_WIN95_KEY"]
fn user_media_win95_stage3() {
    if let Some((disk, _copy)) = installed_disk() {
        run(Stage::Win95FirstBoot, Some(disk));
    }
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN95_DIR, UNEMUPC_USER_DOS_FLOPPY and UNEMUPC_USER_WIN95_KEY"]
fn user_media_win95_stage4() {
    let (first, _copy) = match resume("UNEMUPC_USER_WIN95_STAGE3_DIR") {
        Some((disk, copy)) => (Some(disk), Some(copy)),
        None => match installed_disk() {
            Some((disk, copy)) => (run(Stage::Win95FirstBoot, Some(disk)), copy),
            None => (None, None),
        },
    };
    if let Some(disk) = first {
        run(Stage::Win95SecondBoot, Some(disk));
    }
}
