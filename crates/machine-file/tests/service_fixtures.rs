// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reproducible project-owned BIOS probes; see windows95-on-486.md §5.

#[test]
fn seabios_service_fixtures_rebuild_identically() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let temp = machine_file::test_support::TempDir::new("service-fixtures");
    for name in [
        "seabios-int13",
        "seabios-ata-driver",
        "seabios-mouse",
        "seabios-apm",
    ] {
        let binary = temp.path().join(format!("{name}.bin"));
        let output = std::process::Command::new("nasm")
            .args(["-f", "bin"])
            .arg(root.join(format!("{name}.asm")))
            .arg("-o")
            .arg(&binary)
            .output();
        let output = match output {
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::env::var_os("UNEMUPC_REQUIRE_FIXTURES").is_none() =>
            {
                eprintln!("skipping: nasm is missing");
                return;
            }
            other => other.unwrap(),
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read(binary).unwrap(),
            std::fs::read(root.join(format!("{name}.bin"))).unwrap(),
            "{name}"
        );
    }
}
