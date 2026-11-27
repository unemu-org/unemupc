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

fn checked_directory(bytes: &[u8], cluster: u16, parent: u16) -> Vec<[u8; 32]> {
    let entries: Vec<_> = bytes
        .as_chunks::<32>()
        .0
        .iter()
        .take_while(|entry| entry[0] != 0)
        .filter(|entry| entry[0] != 0xe5)
        .copied()
        .collect();
    for entry in &entries {
        assert_ne!(entry[11], 0x0f, "unexpected LFN entry");
        assert_eq!(&entry[20..22], &[0, 0], "FAT16 high cluster word");
    }
    if cluster != 0 {
        assert!(entries.len() >= 2, "missing dot entries");
        for (entry, name, link) in [
            (&entries[0], b".          ", cluster),
            (&entries[1], b"..         ", parent),
        ] {
            assert_eq!(&entry[..11], name, "dot entry name");
            assert_eq!(entry[11], 0x10, "dot entry attributes");
            assert_eq!(
                u16::from_le_bytes(entry[26..28].try_into().unwrap()),
                link,
                "dot link cluster"
            );
            assert_eq!(&entry[28..32], &[0; 4], "dot entry size");
        }
    }
    entries
}

#[test]
fn fat_directory_verifier_rejects_lfns_and_incorrect_dot_links() {
    let mut entries = [[0u8; 32]; 4];
    for (index, name, cluster) in [(0, b".          ", 2u16), (1, b"..         ", 0u16)] {
        entries[index][..11].copy_from_slice(name);
        entries[index][11] = 0x10;
        entries[index][26..28].copy_from_slice(&cluster.to_le_bytes());
    }
    let bytes = |entries: &[[u8; 32]; 4]| entries.concat();
    assert_eq!(checked_directory(&bytes(&entries), 2, 0).len(), 2);
    for (entry, offset, value) in [
        (0, 0, b'X'),
        (1, 0, b'X'),
        (0, 26, 3),
        (1, 26, 2),
        (0, 11, 0),
        (1, 11, 0),
    ] {
        let mut corrupt = entries;
        corrupt[entry][offset] = value;
        assert!(std::panic::catch_unwind(|| checked_directory(&bytes(&corrupt), 2, 0)).is_err());
    }
    entries[1][26..28].copy_from_slice(&7u16.to_le_bytes());
    assert_eq!(checked_directory(&bytes(&entries), 2, 7).len(), 2);
    assert!(std::panic::catch_unwind(|| checked_directory(&bytes(&entries), 2, 0)).is_err());
    entries[2][0] = 0x41;
    entries[2][11] = 0x0f;
    // Also reject an orphan LFN that no pathname lookup ever visits.
    assert!(std::panic::catch_unwind(|| checked_directory(&bytes(&entries), 2, 7)).is_err());
    assert!(std::panic::catch_unwind(|| checked_directory(&bytes(&entries), 0, 0)).is_err());
}

fn change_host_mtimes(path: &std::path::Path, seconds: u64) {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).unwrap() {
            change_host_mtimes(&entry.unwrap().path(), seconds);
        }
    }
    std::fs::File::open(path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
        )
        .unwrap();
}

fn image_digest(path: &std::path::Path) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path).unwrap(), &mut hash).unwrap();
    hash.finalize().to_vec()
}

struct Fat16 {
    disk: std::fs::File,
    partition: u64,
    fat: u64,
    root: u64,
    data: u64,
    cluster_bytes: usize,
}

impl Fat16 {
    fn read(&mut self, offset: u64, length: usize) -> Vec<u8> {
        use std::io::{Read, Seek, SeekFrom};
        let mut bytes = vec![0; length];
        self.disk.seek(SeekFrom::Start(offset)).unwrap();
        self.disk.read_exact(&mut bytes).unwrap();
        bytes
    }

    fn open(path: &std::path::Path, geometry: (u32, u16, u16)) -> Self {
        let mut fat = Self {
            disk: std::fs::File::open(path).unwrap(),
            partition: 0,
            fat: 0,
            root: 0,
            data: 0,
            cluster_bytes: 0,
        };
        let mbr = fat.read(0, 512);
        assert_eq!(&mbr[510..], &[0x55, 0xaa]);
        assert_eq!((mbr[446], mbr[450]), (0x80, 6));
        assert!(mbr[462..510].iter().all(|byte| *byte == 0));
        let start = u32::from_le_bytes(mbr[454..458].try_into().unwrap());
        assert_eq!(start, u32::from(geometry.2));
        let end_cylinder = geometry.0 - 1;
        assert_eq!(
            &mbr[451..454],
            &[
                geometry.1 as u8 - 1,
                geometry.2 as u8 | ((end_cylinder >> 8) as u8) << 6,
                end_cylinder as u8
            ]
        );
        let sectors = u32::from_le_bytes(mbr[458..462].try_into().unwrap());
        assert_eq!(
            sectors + start,
            geometry.0 * u32::from(geometry.1) * u32::from(geometry.2)
        );
        fat.partition = u64::from(start) * 512;
        let bpb = fat.read(fat.partition, 512);
        let word = |at| u16::from_le_bytes(bpb[at..at + 2].try_into().unwrap());
        assert_eq!(&bpb[54..62], b"FAT16   ");
        assert_eq!(
            (word(11), word(24), word(26)),
            (512, geometry.2, geometry.1)
        );
        assert_eq!(u32::from_le_bytes(bpb[28..32].try_into().unwrap()), start);
        assert_eq!(u32::from_le_bytes(bpb[32..36].try_into().unwrap()), sectors);
        let reserved = u64::from(word(14));
        let fat_sectors = u64::from(word(22));
        let root_sectors = (u64::from(word(17)) * 32).div_ceil(512);
        fat.fat = fat.partition + reserved * 512;
        fat.root = fat.fat + u64::from(bpb[16]) * fat_sectors * 512;
        fat.data = fat.root + root_sectors * 512;
        fat.cluster_bytes = bpb[13] as usize * 512;
        let clusters =
            (u64::from(sectors) - reserved - u64::from(bpb[16]) * fat_sectors - root_sectors)
                / u64::from(bpb[13]);
        assert!((4085..65525).contains(&clusters));
        let first_fat = fat.read(fat.fat, fat_sectors as usize * 512);
        assert_eq!(
            first_fat,
            fat.read(fat.fat + fat_sectors * 512, fat_sectors as usize * 512)
        );
        fat
    }

    fn chain(&mut self, mut cluster: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        while cluster < 0xfff8 {
            assert!(cluster >= 2 && seen.insert(cluster), "invalid FAT chain");
            bytes.extend(self.read(
                self.data + u64::from(cluster - 2) * self.cluster_bytes as u64,
                self.cluster_bytes,
            ));
            cluster = u16::from_le_bytes(
                self.read(self.fat + u64::from(cluster) * 2, 2)
                    .try_into()
                    .unwrap(),
            );
        }
        bytes
    }

    fn check_directories(
        &mut self,
        cluster: u16,
        parent: u16,
        seen: &mut std::collections::BTreeSet<u16>,
    ) {
        assert!(seen.insert(cluster), "directory cycle or shared cluster");
        let bytes = if cluster == 0 {
            self.read(self.root, 32 * 512)
        } else {
            self.chain(cluster)
        };
        for entry in checked_directory(&bytes, cluster, parent) {
            if entry[11] & 0x10 != 0 && entry[0] != b'.' {
                self.check_directories(
                    u16::from_le_bytes(entry[26..28].try_into().unwrap()),
                    cluster,
                    seen,
                );
            }
        }
    }

    fn entry(&mut self, path: &[[u8; 11]]) -> [u8; 32] {
        let mut directory = self.read(self.root, 32 * 512);
        for (index, name) in path.iter().enumerate() {
            let entry = *directory
                .as_chunks::<32>()
                .0
                .iter()
                .find(|entry| &entry[..11] == name)
                .expect("directory entry");
            if index == path.len() - 1 {
                return entry;
            }
            assert_ne!(entry[11] & 0x10, 0);
            directory = self.chain(u16::from_le_bytes(entry[26..28].try_into().unwrap()));
        }
        unreachable!()
    }

    fn file(&mut self, path: &[[u8; 11]]) -> Vec<u8> {
        let entry = self.entry(path);
        assert_eq!(entry[11] & 0x10, 0);
        let size = u32::from_le_bytes(entry[28..32].try_into().unwrap()) as usize;
        if size == 0 {
            return Vec::new();
        }
        let mut bytes = self.chain(u16::from_le_bytes(entry[26..28].try_into().unwrap()));
        bytes.truncate(size);
        bytes
    }
}

fn builder() -> Command {
    let mut command = Command::new("bash");
    command.arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/user-media-disk.sh"));
    command.env_remove("UNEMUPC_USER_WIN95_KEY");
    command.env_remove("UNEMUPC_USER_DOOM_DIR");
    command
}

#[test]
fn general_builder_roundtrips_nested_tree_and_geometry_through_fat_parser() {
    let source = TempDir::new("synthetic-fat-tree");
    std::fs::create_dir_all(source.path().join("subdir/EMPTYDIR")).unwrap();
    let payload: Vec<u8> = (0..70000).map(|index| (index % 251) as u8).collect();
    std::fs::write(source.path().join("subdir/PAYLOAD.BIN"), &payload).unwrap();
    std::fs::write(source.path().join("EMPTY.DAT"), []).unwrap();
    for name in ["A!#$%&().TXT", "_@^`{}~-.BIN"] {
        std::fs::write(source.path().join(name), b"open punctuation").unwrap();
    }
    for index in 0..140 {
        std::fs::write(
            source.path().join(format!("subdir/F{index:03}.TXT")),
            b"open data",
        )
        .unwrap();
    }
    let extra = source.path().join("EXTRA.INF");
    std::fs::write(&extra, b"generated open configuration").unwrap();
    // Exercise both the default half-gigabyte CHS and a chosen smaller size.
    for (options, geometry) in [
        (vec![], (1015, 16, 63)),
        (
            vec![
                "--size-mib",
                "40",
                "--cylinders",
                "700",
                "--heads",
                "4",
                "--sectors",
                "32",
            ],
            (640, 4, 32),
        ),
    ] {
        let build_tree = || {
            image(
                builder()
                    .arg("--win95")
                    .arg("--source")
                    .arg(source.path())
                    .args(["--target", "\\WIN95"])
                    .args(&options)
                    .arg("--extra")
                    .arg(&extra)
                    .arg("\\EXTRA\\CONFIG.INF")
                    .output()
                    .unwrap(),
            )
        };
        let path = build_tree();
        change_host_mtimes(source.path(), 1_577_934_240 + u64::from(geometry.0));
        let second = build_tree();
        assert_eq!(
            image_digest(&path),
            image_digest(&second),
            "nested/generated image depends on host mtimes"
        );
        let mut fat = Fat16::open(&path, geometry);
        let mut directories = std::collections::BTreeSet::new();
        fat.check_directories(0, 0, &mut directories);
        assert_eq!(directories.len(), 5);
        assert_eq!(
            fat.file(&[*b"WIN95      ", *b"SUBDIR     ", *b"PAYLOAD BIN"]),
            payload
        );
        assert_eq!(
            fat.file(&[*b"WIN95      ", *b"SUBDIR     ", *b"F139    TXT"]),
            b"open data"
        );
        assert!(fat.file(&[*b"WIN95      ", *b"EMPTY   DAT"]).is_empty());
        assert_eq!(
            fat.file(&[*b"EXTRA      ", *b"CONFIG  INF"]),
            b"generated open configuration"
        );
        assert!(String::from_utf8(fat.file(&[*b"MSBATCH INF"]))
            .unwrap()
            .contains("Express=1\r\n"));
        drop(fat);
        for image in [path, second] {
            std::fs::remove_dir_all(image.parent().unwrap()).unwrap();
        }
    }
}

fn assert_builder_error(result: std::process::Output, category: &str) {
    if result.status.success() {
        let path = image(result);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        panic!("invalid input was accepted");
    }
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8(result.stderr).unwrap(),
        format!("user-media disk build failed: {category}\n")
    );
}

#[test]
fn general_builder_refuses_collisions_invalid_names_and_geometry_without_paths() {
    let source = TempDir::new("synthetic-invalid-tree");
    let extra = source.path().join("ONE.TXT");
    std::fs::write(&extra, b"open data").unwrap();
    // A control must succeed without DOOM input; an unrelated tool/input error
    // must never count as evidence that a new argument was validated.
    let control = image(
        builder()
            .arg("--source")
            .arg(source.path())
            .output()
            .unwrap(),
    );
    std::fs::remove_dir_all(control.parent().unwrap()).unwrap();
    for (args, category) in [
        (
            vec!["--extra", extra.to_str().unwrap(), "one.txt"],
            "case-insensitive filename collision",
        ),
        (
            vec!["--target", "../escape"],
            "expected a DOS 8.3 target path",
        ),
        (vec!["--cylinders", "1025"], "invalid CHS geometry"),
        (vec!["--heads", "0"], "invalid CHS geometry"),
        (vec!["--size-mib", "1"], "chosen size cannot hold FAT16"),
    ] {
        assert_builder_error(
            builder()
                .arg("--source")
                .arg(source.path())
                .args(args)
                .output()
                .unwrap(),
            category,
        );
    }
    std::fs::write(source.path().join("TOO-LONG-NAME.TXT"), b"open data").unwrap();
    assert_builder_error(
        builder()
            .arg("--source")
            .arg(source.path())
            .output()
            .unwrap(),
        "expected regular files/directories with DOS 8.3 names",
    );
}

#[test]
fn general_builder_rejects_mtools_renaming_in_every_path_input() {
    let source = TempDir::new("synthetic-short-names");
    let extra = source.path().join("OPEN.TXT");
    std::fs::write(&extra, b"open data").unwrap();
    for name in [
        "O'NEIL.TXT",
        "CON",
        "prn.txt",
        "AUX",
        "NUL.DAT",
        "COM1",
        "COM9.LOG",
        "LPT1",
        "lpt9.txt",
    ] {
        for directory in [false, true] {
            let invalid = source.path().join(name);
            if directory {
                std::fs::create_dir(&invalid).unwrap();
            } else {
                std::fs::write(&invalid, b"open data").unwrap();
            }
            let result = builder()
                .arg("--source")
                .arg(source.path())
                .output()
                .unwrap();
            if directory {
                std::fs::remove_dir(invalid).unwrap();
            } else {
                std::fs::remove_file(invalid).unwrap();
            }
            assert_builder_error(
                result,
                "expected regular files/directories with DOS 8.3 names",
            );
        }
        assert_builder_error(
            builder()
                .arg("--source")
                .arg(source.path())
                .args(["--target", name])
                .output()
                .unwrap(),
            "expected a DOS 8.3 target path",
        );
        assert_builder_error(
            builder()
                .arg("--source")
                .arg(source.path())
                .arg("--extra")
                .arg(&extra)
                .arg(name)
                .output()
                .unwrap(),
            "expected a DOS 8.3 target path",
        );
    }
}

#[test]
fn msbatch_key_is_only_in_generated_disk_never_in_builder_logs() {
    let source = TempDir::new("synthetic-batch");
    std::fs::write(
        source.path().join("SETUP.EXE"),
        b"open synthetic executable",
    )
    .unwrap();
    let key = "SYNTHETIC-PRIVATE-KEY";
    for supplied in [None, Some(key), Some("INVALID\nSECRET")] {
        let mut command = builder();
        command
            .arg("--win95")
            .env("UNEMUPC_USER_WIN95_DIR", source.path());
        if let Some(key) = supplied {
            command.env("UNEMUPC_USER_WIN95_KEY", key);
        }
        let result = command.output().unwrap();
        for log in [&result.stdout, &result.stderr] {
            let log = String::from_utf8_lossy(log);
            assert!(!log.contains(key) && !log.contains("INVALID") && !log.contains("SECRET"));
        }
        if supplied.is_some_and(|value| value.contains('\n')) {
            assert!(!result.status.success());
            continue;
        }
        let path = image(result);
        let mut fat = Fat16::open(&path, (1015, 16, 63));
        let batch = String::from_utf8(fat.file(&[*b"MSBATCH INF"])).unwrap();
        assert!(batch.contains("Express=1\r\n") && batch.contains("Mouse="));
        assert_eq!(batch.contains(key), supplied.is_some());
        assert_eq!(batch.contains("ProductKey="), supplied.is_some());
        drop(fat);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    assert!(!source.path().join("MSBATCH.INF").exists());
}

#[test]
fn win31_builder_generates_boot_chain_and_settings_without_touching_source() {
    let source = TempDir::new("synthetic-win31");
    std::fs::write(source.path().join("setup.exe"), b"synthetic setup").unwrap();
    let build_disk = || {
        image(
            builder()
                .arg("--win31")
                .env("UNEMUPC_USER_WIN31_DIR", source.path())
                .output()
                .unwrap(),
        )
    };
    let first = build_disk();
    let second = build_disk();
    assert_eq!(image_digest(&first), image_digest(&second));
    let mut fat = Fat16::open(&first, (1015, 16, 63));
    assert_eq!(
        fat.file(&[*b"WIN31SRC   ", *b"SETUP   EXE"]),
        b"synthetic setup"
    );
    let settings = String::from_utf8(fat.file(&[*b"SETUP   SHH"])).unwrap();
    for request in [
        "machine=ibm_compatible",
        "display=vga",
        "mouse=ps2mouse",
        "keyboard=t4s0enha",
        "network=nonet",
        "[windir]\r\nC:\\WINDOWS",
        "endopt=exit",
        "configfiles=modify",
    ] {
        assert!(settings.contains(request), "missing request: {request}");
    }
    assert!(!settings.contains("[printers]"));
    assert!(fat
        .file(&[*b"CONFIG  SYS"])
        .windows(9)
        .any(|s| s == b"HIMEM.SYS"));
    let transfer = String::from_utf8(fat.file(&[*b"SYS31   BAT"])).unwrap();
    for required in [
        "SYS C:",
        "IF ERRORLEVEL 1 GOTO END",
        "IF NOT EXIST C:\\IO.SYS",
        "IF NOT EXIST C:\\COMMAND.COM",
        "C:\\WIN31SRC\\EXPAND C:\\WIN31SRC\\HIMEM.SY_ C:\\HIMEM.SYS",
        "IF NOT EXIST C:\\HIMEM.SYS",
        "BootGUI=0",
        "Logo=0",
        "BootDelay=0",
    ] {
        assert!(
            transfer.contains(required),
            "missing transfer guard or setting: {required}"
        );
    }
    // MS-DOS 6 keeps part of its kernel in MSDOS.SYS: every line that touches
    // it must sit between the version gate and the label the gate jumps to.
    let lines: Vec<&str> = transfer.split("\r\n").collect();
    let at = |line: &str| lines.iter().position(|l| *l == line).unwrap();
    let gate = at("IF NOT ERRORLEVEL 7 GOTO DONE");
    assert_eq!(lines[gate - 1], "C:\\DOSVER.COM");
    let done = at(":DONE");
    let touching: Vec<usize> = (0..lines.len())
        .filter(|index| lines[*index].contains("MSDOS.SYS"))
        .collect();
    assert_eq!(touching.len(), 5);
    assert!(touching.iter().all(|index| (gate..done).contains(index)));
    assert!((gate..done).contains(&at("ECHO UM31DOS7")));
    assert!(at("ECHO UM31SYSOK") > done);
    // MOV AH,30h; INT 21h (DOS version into AL); MOV AH,4Ch; INT 21h (exit
    // with AL as the ERRORLEVEL).
    assert_eq!(
        fat.file(&[*b"DOSVER  COM"]),
        [0xb4, 0x30, 0xcd, 0x21, 0xb4, 0x4c, 0xcd, 0x21]
    );
    let batch = String::from_utf8(fat.file(&[*b"INST31  BAT"])).unwrap();
    assert!(batch.contains("SETUP /H:C:\\SETUP.SHH"));
    assert!(batch.contains("IF NOT EXIST C:\\WINDOWS\\SYSTEM\\WIN386.EXE"));
    let mbr = fat.read(0, 512);
    assert_ne!(&mbr[..62], &[0; 62], "SYS needs an MBR chain loader");
    assert_eq!(&mbr[510..], &[0x55, 0xaa]);
    assert_eq!(mbr[446], 0x80);
    assert_eq!(
        std::fs::read(source.path().join("setup.exe")).unwrap(),
        b"synthetic setup"
    );
    assert_eq!(std::fs::read_dir(source.path()).unwrap().count(), 1);
    for path in [first, second] {
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

#[test]
fn win31_mbr_chainloads_a_synthetic_partition_and_preserves_boot_drive() {
    use std::io::{Seek, SeekFrom, Write};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // The machine file names SeaBIOS and the default floppy and disk images;
    // none of them is in git, so the test skips where they are not fetched.
    if [
        "seabios/bios-1.17.0-isa.bin",
        "seabios/build/vgabios-1.17.0-stdvga.bin",
        "freedos/freedos.img",
        "hdd/fat16.img",
    ]
    .iter()
    .any(|name| !root.join("vendor").join(name).exists())
    {
        assert!(
            std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none(),
            "required machine fixture is missing"
        );
        eprintln!("skipping: a machine fixture is missing");
        return;
    }
    let source = TempDir::new("synthetic-win31-boot");
    let path = image(
        builder()
            .arg("--win31")
            .args(["--size-mib", "40"])
            .env("UNEMUPC_USER_WIN31_DIR", source.path())
            .output()
            .unwrap(),
    );
    // Complete the generated base before assembly; all subsequent guest writes
    // go through the overlay. This VBR is ours, independent of a DOS SYS binary.
    let mut disk = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    disk.seek(SeekFrom::Start(63 * 512)).unwrap();
    disk.write_all(&[
        0x31, 0xc0, 0x8e, 0xd8, 0xc6, 0x06, 0x00, 0x05, 0x31, 0x88, 0x16, 0x01, 0x05, 0xfa, 0xf4,
    ])
    .unwrap();
    drop(disk);
    let mut config = machine_file::load_for_assembly(
        root.join("machines/generic-486-ide.toml"),
        Default::default(),
    )
    .unwrap()
    .config;
    config
        .drives
        .retain(|drive| matches!(drive.kind, assembly::DriveKind::HardDisk { .. }));
    let drive = &mut config.drives[0];
    drive.kind = assembly::DriveKind::HardDisk {
        geometry: assembly::HardDiskGeometry {
            cylinders: (std::fs::metadata(&path).unwrap().len() / (16 * 63 * 512)) as u32,
            heads: 16,
            sectors_per_track: 63,
            bytes_per_sector: 512,
        },
    };
    drive.expected_sha256 = Some(image_digest(&path).try_into().unwrap());
    drive.image = assembly::ImageSource::File(path.clone());
    let mut assembled =
        machine_file::assemble_with_observation(&config, &Default::default()).unwrap();
    assert_eq!(
        assembled.machine.run_bounded(Some(20_000_000)).unwrap(),
        machine::RunOutcome::Halted
    );
    let mut result = [0; 2];
    assembled
        .machine
        .cpu()
        .read_mem(0x500, &mut result)
        .unwrap();
    assert_eq!(result, [0x31, 0x80]);
    assembled.finish_persistence().unwrap();
    drop(assembled);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
