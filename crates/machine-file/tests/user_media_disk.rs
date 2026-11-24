// SPDX-License-Identifier: AGPL-3.0-or-later
//! Synthetic media-builder checks; Microsoft FAT Specification, BPB and FAT
//! type determination. See user-media.md §Temporary disk.

use machine_file::test_support::TempDir;
use std::{path::PathBuf, process::Command};

fn build(source: &std::path::Path) -> std::process::Output {
    Command::new("bash")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/user-media-disk.sh"))
        .env("UNEMUPC_USER_DOOM_DIR", source)
        .output()
        .unwrap()
}

fn image(output: std::process::Output) -> PathBuf {
    assert!(
        output.status.success(),
        "builder failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

#[test]
fn synthetic_media_is_deterministic_partitioned_fat16() {
    let source = TempDir::new("synthetic-user-media");
    std::fs::write(source.path().join("doom.exe"), b"synthetic executable").unwrap();
    std::fs::write(source.path().join("DOOM.WAD"), b"synthetic data").unwrap();
    let first = image(build(source.path()));
    // Host mtimes must not enter the FAT directory entries.
    assert!(Command::new("touch")
        .args(["-t", "202001020304"])
        .arg(source.path().join("doom.exe"))
        .status()
        .unwrap()
        .success());
    let second = image(build(source.path()));
    let bytes = std::fs::read(&first).unwrap();
    assert!(
        bytes == std::fs::read(&second).unwrap(),
        "fixed timestamps and sorted names must reproduce identical media"
    );
    assert_eq!(bytes.len(), 34 * 1024 * 1024);
    assert_eq!(&bytes[510..512], &[0x55, 0xaa]);
    assert_eq!(bytes[450], 6);
    assert_eq!(u32::from_le_bytes(bytes[454..458].try_into().unwrap()), 32);
    let bpb = &bytes[16384..16896];
    assert_eq!(&bpb[54..62], b"FAT16   ");
    assert_eq!(u16::from_le_bytes(bpb[11..13].try_into().unwrap()), 512);
    let reserved = u16::from_le_bytes(bpb[14..16].try_into().unwrap()) as usize;
    let fat_sectors = u16::from_le_bytes(bpb[22..24].try_into().unwrap()) as usize;
    let root = 16384 + (reserved + bpb[16] as usize * fat_sectors) * 512;
    let entry = bytes[root..root + 512 * 32]
        .as_chunks::<32>()
        .0
        .iter()
        .find(|entry| &entry[..11] == b"DOOM    EXE")
        .unwrap();
    assert_eq!(u16::from_le_bytes(entry[22..24].try_into().unwrap()), 0);
    assert_eq!(u16::from_le_bytes(entry[24..26].try_into().unwrap()), 33);
    let copied = Command::new("mtype")
        .arg("-i")
        .arg(format!("{}@@16384", first.display()))
        .arg("::DOOM.EXE")
        .output()
        .unwrap();
    assert!(copied.status.success());
    assert_eq!(copied.stdout, b"synthetic executable");
    for path in [first, second] {
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

#[test]
fn synthetic_media_rejects_missing_input_without_disclosing_paths() {
    let source = TempDir::new("missing-user-media");
    let result = build(source.path());
    assert!(!result.status.success());
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(error.contains("expected DOOM.EXE and DOOM.WAD"));
    assert!(!error.contains(source.path().to_str().unwrap()));
}

#[test]
fn synthetic_media_grows_to_fit_input() {
    let source = TempDir::new("growing-user-media");
    std::fs::write(source.path().join("DOOM.EXE"), b"synthetic").unwrap();
    std::fs::File::create(source.path().join("doom.wad"))
        .unwrap()
        .set_len(40 * 1024 * 1024)
        .unwrap();
    let disk = image(build(source.path()));
    assert!(std::fs::metadata(&disk).unwrap().len() > 34 * 1024 * 1024);
    std::fs::remove_dir_all(disk.parent().unwrap()).unwrap();
}
