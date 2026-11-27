// SPDX-License-Identifier: AGPL-3.0-or-later
//! Private Windows 3.1 stage checkpoints; user-media.md §Windows 3.1.

#[path = "support/windows.rs"]
mod windows;
use windows::{run, Stage, Staged};

fn system_disk() -> Option<Staged> {
    run(Stage::SystemTransfer, None)
}

fn installed_disk(system: Staged) -> Option<Staged> {
    run(Stage::Install, Some(system))
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN31_DIR and UNEMUPC_USER_DOS_FLOPPY"]
fn user_media_win31_stage1() {
    system_disk();
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN31_DIR and UNEMUPC_USER_DOS_FLOPPY"]
fn user_media_win31_stage2() {
    system_disk().and_then(installed_disk);
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN31_DIR and UNEMUPC_USER_DOS_FLOPPY"]
fn user_media_win31_standard_mode() {
    if let Some(disk) = system_disk().and_then(installed_disk) {
        run(Stage::Standard, Some(disk));
    }
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN31_DIR and UNEMUPC_USER_DOS_FLOPPY"]
fn user_media_win31_stage3() {
    let Some(system) = system_disk() else { return };
    if system.dos7 {
        eprintln!("skipping: a DOS 7 kernel refuses Enhanced mode; user_media_win31_enhanced_refused_by_dos7 covers that floppy");
        return;
    }
    if let Some(disk) = installed_disk(system) {
        run(Stage::Enhanced, Some(disk));
    }
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN31_DIR and UNEMUPC_USER_DOS_FLOPPY"]
fn user_media_win31_enhanced_refused_by_dos7() {
    let Some(system) = system_disk() else { return };
    if !system.dos7 {
        eprintln!("skipping: the DOS floppy is not DOS 7; user_media_win31_stage3 covers it");
        return;
    }
    if let Some(disk) = installed_disk(system) {
        run(Stage::EnhancedRefusedByDos7, Some(disk));
    }
}
