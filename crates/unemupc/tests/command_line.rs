// SPDX-License-Identifier: AGPL-3.0-or-later
//! Standalone program contracts; core-contract.md §12.8. Synthetic firmware
//! keeps the command dispatch test independent of host firmware and stores.

use std::process::Command;

fn command(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_unemupc"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn standalone_usage_and_notices_name_only_the_pc_core() {
    let usage = command(&[]);
    assert_eq!(usage.status.code(), Some(0));
    assert!(usage.stderr.is_empty());
    let usage = String::from_utf8(usage.stdout).unwrap();
    assert!(usage.starts_with("usage: unemupc --boot"));
    for words in [
        "--bios PATH",
        "machine check FILE",
        "run --machine FILE",
        "--version",
        "--about",
    ] {
        assert!(usage.contains(words), "{words}: {usage}");
    }
    assert!(
        !usage.contains("upcli") && !usage.contains("--store") && !usage.contains("--no-store")
    );
    use unemu_core_api::Core;
    let description = unemu_pc_core::PcCore.description();
    let version = command(&["--version"]);
    let about = command(&["--about"]);
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(about.status.code(), Some(0));
    assert!(version.stderr.is_empty() && about.stderr.is_empty());
    assert_eq!(version.stdout, about.stdout);
    let text = String::from_utf8(version.stdout).unwrap();
    assert!(text.starts_with(&format!("{} {}\n", description.id, description.version)));
    assert!(text.contains("contract 0.6\n"));
    assert!(text.contains("AGPL-3.0-or-later") && text.contains(env!("CARGO_PKG_REPOSITORY")));
    let repository = env!("CARGO_PKG_REPOSITORY");
    let source = match option_env!("UNEMUPC_SOURCE_REV").filter(|rev| !rev.is_empty()) {
        Some(rev) => format!("{repository}/tree/{rev}"),
        None => format!("{repository}, version {}", env!("CARGO_PKG_VERSION")),
    };
    let commit = match option_env!("UNEMUPC_SOURCE_COMMIT").filter(|commit| !commit.is_empty()) {
        Some(commit) => format!("commit: {commit}\n"),
        None => String::new(),
    };
    assert!(text.contains(&format!(
        "\nsource: {source}\n{commit}Third-party licence texts: THIRD-PARTY-LICENSES.md\n"
    )));
    for notice in description.notices {
        assert!(text.contains(notice.text));
    }
    assert!(!text.contains("unemunes") && !text.contains("registry") && !text.contains("store"));
}

#[test]
fn standalone_refuses_store_flags_and_frontend_commands() {
    for args in [
        vec!["run", "--machine", "missing.toml", "--store", "store"],
        vec!["run", "--machine", "missing.toml", "--no-store"],
        vec!["image", "ls"],
        vec!["media", "verify"],
        vec!["--about", "extra"],
    ] {
        let output = command(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty());
        assert!(output.stderr.starts_with(b"usage: unemupc "));
    }
}

#[test]
fn standalone_dispatches_all_four_machine_commands() {
    let demo = command(&["--boot"]);
    assert_eq!(demo.status.code(), Some(0));
    assert_eq!(demo.stdout, b".....");
    assert!(demo.stderr.is_empty());
    let files = machine_file::test_support::TempDir::new("standalone-commands");
    let mut rom = vec![0; 32768];
    rom[0x7ff0..0x7ff8].copy_from_slice(&[0xfa, 0xba, 0xf8, 0x03, 0xb0, b'Z', 0xee, 0xf4]);
    let bios = files.path().join("bios.bin");
    std::fs::write(&bios, rom).unwrap();
    let machine = files.path().join("machine.toml");
    std::fs::write(&machine, "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='bios.bin'\n[[card]]\nkind='uart-16550'\nid='console'\nirq='none'\n").unwrap();
    let bios = command(&[
        "--bios",
        bios.to_str().unwrap(),
        "--allow-unlisted-firmware",
    ]);
    assert_eq!(bios.status.code(), Some(0), "{bios:?}");
    assert_eq!(bios.stdout, b"Z");
    assert!(bios.stderr.ends_with(b"stopped: halt\n"));
    let check = command(&[
        "machine",
        "check",
        machine.to_str().unwrap(),
        "--allow-unlisted-firmware",
    ]);
    assert_eq!(check.status.code(), Some(0), "{check:?}");
    assert!(check.stdout.is_empty());
    let run = command(&[
        "run",
        "--machine",
        machine.to_str().unwrap(),
        "--allow-unlisted-firmware",
        "--print",
        "console",
    ]);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(run.stdout, b"Z");
    assert!(run.stderr.ends_with(b"stopped: halt\n"));
}
