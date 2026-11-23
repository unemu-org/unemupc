// SPDX-License-Identifier: AGPL-3.0-or-later
//! Machine-file command contracts without guest windows or lock-holding siblings.
//! See machine-files § Frontends and persistent-media § Session lifecycle.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

// The 80286 LOADALL (0F 05) in real mode with the table at physical 800h
// still zero: no valid SS or CS, a table the CPU core refuses to load. Two
// bytes fit at the reset vector and keep the failure path independent of
// protected mode.
const REFUSED_LOADALL: &[u8] = &[0x0F, 0x05];

struct Files(PathBuf);
impl Files {
    fn new(program: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "machine-commands-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let files = Self(root);
        let mut rom = vec![0; 32768];
        rom[0x7FF0..0x7FF0 + program.len()].copy_from_slice(program);
        files.write("bios.bin", rom);
        files.write("machine.toml", "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='bios.bin'\n[[card]]\nkind='uart-16550'\nid='console'\nirq='none'\n[[card]]\nkind='mda'\nid='screen'\n");
        files
    }
    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }
    fn machine(&self) -> PathBuf {
        self.0.join("machine.toml")
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn command(args: &[&str], path: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_unemupc"))
        .args(args)
        .arg(path)
        .args(extra)
        .output()
        .unwrap()
}
fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

#[path = "../../pc-harness/tests/support/fixtures.rs"]
mod fixtures;

#[test]
fn machine_check_has_valid_invalid_and_usage_exit_codes() {
    // Executing this ROM would fail; check must assemble without executing it.
    let files = Files::new(REFUSED_LOADALL);
    let accepted = command(
        &["machine", "check"],
        &files.machine(),
        &["--allow-unlisted-firmware"],
    );
    assert_eq!(accepted.status.code(), Some(0), "{accepted:?}");
    assert!(accepted.stdout.is_empty());
    let invalid = files.write("invalid.toml", "schema=1\n[board]\nmodel='unknown'\n");
    let rejected = command(&["machine", "check"], &invalid, &[]);
    assert_eq!(rejected.status.code(), Some(1), "{rejected:?}");
    assert!(stderr(&rejected).contains(&format!("{}:", invalid.display())));
    assert!(stderr(&rejected).contains("line 3"), "{rejected:?}");
    for args in [
        vec!["machine"],
        vec!["machine", "check"],
        vec!["machine", "check", "a.toml", "extra"],
        vec!["run", "--machine"],
        vec!["run", "--machine", "a.toml", "--max-steps", "-1"],
        vec![
            "run",
            "--machine",
            "a.toml",
            "--idle-limit",
            "18446744073709551616",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_unemupc"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
    }
}

#[test]
fn inherited_unlisted_firmware_is_positioned_and_escape_reaches_both_commands() {
    let files = Files::new(&[0xFA, 0xBA, 0xF8, 0x03, 0xB0, b'Z', 0xEE, 0xF4]);
    let child = files.write("child.toml", "schema=1\nextends='machine.toml'\n");
    for args in [&["machine", "check"][..], &["run", "--machine"][..]] {
        let rejected = command(args, &child, &[]);
        assert_eq!(rejected.status.code(), Some(1), "{rejected:?}");
        assert!(
            stderr(&rejected).contains(&format!("{}:5:", files.machine().display())),
            "{rejected:?}"
        );
        assert!(stderr(&rejected).contains("firmware.bios.image: bios image (sha256"));
        let accepted = command(args, &child, &["--allow-unlisted-firmware"]);
        assert_eq!(accepted.status.code(), Some(0), "{accepted:?}");
        assert_eq!(
            stderr(&accepted)
                .matches("warning: running unlisted firmware")
                .count(),
            1
        );
    }
    let run = command(
        &["run", "--machine"],
        &child,
        &["--allow-unlisted-firmware", "--print", "console"],
    );
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(run.stdout, b"Z");
    assert!(stderr(&run).ends_with("stopped: halt\n"));
    let bad_card = command(
        &["run", "--machine"],
        &child,
        &["--allow-unlisted-firmware", "--print", "absent"],
    );
    assert_eq!(bad_card.status.code(), Some(1));
    assert!(stderr(&bad_card).contains("output card \"absent\" is not configured"));
}

#[test]
fn machine_check_accepts_the_shipped_machine() {
    let (Some(_rom), Some(_)) = (
        fixtures::fixture("kiselev-bios/bios-xi8088.bin"),
        fixtures::fixture("freedos/freedos.img"),
    ) else {
        return;
    };
    let machine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../machines/at-freedos.toml");
    let valid = command(&["machine", "check"], &machine, &[]);
    assert_eq!(valid.status.code(), Some(0), "{valid:?}");
}

#[test]
fn rom_discovery_requires_explicit_flags_and_reports_skipped_roots() {
    let files = Files::new(&[0xF4]);
    let dumps = files.0.join("dumps");
    std::fs::create_dir(&dumps).unwrap();
    std::fs::write(dumps.join("renamed.bin"), vec![0xf4; 32768]).unwrap();
    files.write("bios.bin", vec![0; 32768]);
    files.write("machine.toml", "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='bios.bin'\nsha256='d7da9668c2e0720cf257c00882bf495ca672e7a4d972b4b124b6b853e7ac9668'\n[[card]]\nkind='mda'\n");
    for args in [vec!["machine", "check"], vec!["run", "--machine"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_unemupc"))
            .args(&args)
            .arg(files.machine())
            .arg("--allow-unlisted-firmware")
            .env("UNEMUPC_USER_ROMS_DIR", &dumps)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).contains("sha256 mismatch"),
            "{}",
            stderr(&output)
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_unemupc"))
        .args(["machine", "check"])
        .arg(files.machine())
        .arg("--allow-unlisted-firmware")
        .arg("--rom-dir")
        .arg(files.0.join("unmounted"))
        .arg("--rom-dir")
        .arg(dumps)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("warning: <--rom-dir>: skipped"));
}

#[test]
fn shared_loader_preserves_machine_config_cards_firmware_policy_and_name() {
    let files = Files::new(&[0xFA, 0xF4]);
    let path = files.machine();
    let mut document = std::fs::read_to_string(&path).unwrap();
    document = document.replacen("schema=1", "schema=1\nname='named machine'", 1);
    std::fs::write(&path, document).unwrap();
    let options = assembly::AssemblyOptions {
        allow_unlisted_firmware: true,
    };
    let prepared = machine_file::prepare_for_assembly_with_firmware_packages(
        &path,
        &[],
        &mut Vec::new(),
        None,
    )
    .unwrap();
    let original = prepared.load(options).unwrap();
    let (name, shared) =
        unemupc::run::load_named_machine(&path, options, &[], &mut Vec::new()).unwrap();
    assert_eq!(name.as_deref(), Some("named machine"));
    assert_eq!(shared.config, original.config);
    assert_eq!(shared.cards, original.cards);
    let refused = unemupc::run::load_machine(
        &path,
        assembly::AssemblyOptions::default(),
        &[],
        &mut Vec::new(),
    )
    .unwrap();
    assert!(refused
        .assemble(&assembly::ObservationPolicy::default())
        .is_err());
    let mut original_out = Vec::new();
    let mut original_errors = Vec::new();
    let original_code = unemupc::run::run_machine(
        original,
        (100, 100, None, false),
        None,
        &mut original_out,
        &mut original_errors,
    )
    .unwrap();
    let mut shared_out = Vec::new();
    let mut shared_errors = Vec::new();
    let shared_code = unemupc::run::run_machine(
        shared,
        (100, 100, None, false),
        None,
        &mut shared_out,
        &mut shared_errors,
    )
    .unwrap();
    assert_eq!(
        (shared_code, shared_out, shared_errors),
        (original_code, original_out, original_errors)
    );
}
