// SPDX-License-Identifier: AGPL-3.0-or-later
//! Local Windows 95 setup; user-media.md §Windows 95.

#[path = "support/windows.rs"]
mod windows;

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN95_DIR and UNEMUPC_USER_DOS_FLOPPY"]
fn user_media_win95_stage1() {
    windows::run(windows::Stage::Win95, None);
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN95_DIR, UNEMUPC_USER_DOS_FLOPPY and UNEMUPC_USER_WIN95_KEY"]
fn user_media_win95_stage2() {
    windows::run(windows::Stage::Win95FileCopy, None);
}
