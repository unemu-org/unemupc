// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared hardware binding grammar in both host syntaxes.

#[test]
fn toml_accepts_all_binding_source_forms() {
    machine_file::parse_toml(
        "bindings.toml",
        r#"
schema = 1
disks = [{ id = "system", size = 512 }]
[firmware.bios]
use = "bios"
rom = "reset"
role = "system-bios"
size = 32768
[[firmware.option_roms]]
use = "video"
rom = "vga"
base = 0xC0000
[[card]]
kind = "ide"
[[card.drives]]
channel = "master"
disk = "system"
geometry = { cylinders = 1, heads = 1, sectors = 1 }
[[card.drives]]
channel = "slave"
inserted = { use = "os", medium = "hard-disk", size = 512 }
geometry = { cylinders = 1, heads = 1, sectors = 1 }
[[card]]
kind = "fdc-765"
[[card.drives]]
unit = 0
type = "1.2M"
inserted = { use = "dos", medium = "disk-1" }
[[card.drives]]
unit = 1
type = "1.2M"
media = [{ use = "dos", medium = "disk-2", write = "readonly" }]
"#,
    )
    .unwrap();
}

#[test]
fn yaml_accepts_all_binding_source_forms() {
    machine_file::parse_yaml(
        "bindings.yaml",
        r#"
schema: 1
disks: [{id: system, size: 512}]
firmware:
  bios: {use: bios, rom: reset, role: system-bios, size: 32768}
  option_roms: [{use: video, rom: vga, base: 0xC0000}]
card:
  - kind: ide
    drives:
      - channel: master
        disk: system
        geometry: {cylinders: 1, heads: 1, sectors: 1}
      - channel: slave
        inserted: {use: os, medium: hard-disk, size: 512}
        geometry: {cylinders: 1, heads: 1, sectors: 1}
  - kind: fdc-765
    drives:
      - {unit: 0, type: 1.2M, inserted: {use: dos, medium: disk-1}}
      - unit: 1
        type: 1.2M
        media: [{use: dos, medium: disk-2, write: readonly}]
"#,
    )
    .unwrap();
}

use machine_file::{bindings, resolve, test_support::TempDir};
use unemu_core_api::{BindingItem, BindingKind, BindingUse};

#[test]
fn inheritance_retains_selector_positions_and_merges_declarations() {
    for yaml in [false, true] {
        let tree = TempDir::new("binding-inheritance");
        let parent = tree
            .path()
            .join(if yaml { "parent.yaml" } else { "parent.toml" });
        std::fs::write(&parent,if yaml {
            "schema: 1\ndisks: [{id: system, size: 512}]\nboard: {model: ibm-5170}\nfirmware:\n  bios:\n    use: bios\n    rom: reset\ncard:\n  - kind: ide\n    drives:\n      - channel: master\n        disk: system\n        geometry: {cylinders: 1, heads: 1, sectors: 1}\n"
        }else{
            "schema=1\ndisks=[{id='system',size=512}]\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nuse='bios'\nrom='reset'\n[[card]]\nkind='ide'\n[[card.drives]]\nchannel='master'\ndisk='system'\ngeometry={cylinders=1,heads=1,sectors=1}\n"
        }).unwrap();
        let child = tree.path().join("child.toml");
        std::fs::write(&child,format!("schema=1\nextends='{}'\n[firmware.bios]\nbase=0xF8000\n[[card]]\nid='ide'\n[[card.drives]]\nchannel='master'\nwrite='readonly'\n",parent.file_name().unwrap().to_str().unwrap())).unwrap();
        let (document, origins) = resolve(&child).unwrap();
        assert_eq!(document.disks.as_ref().unwrap()[0].id, "system");
        let error = bindings::plan(&document, &origins)
            .unwrap()
            .resolve(&[BindingUse::new(
                "bios",
                "manifest",
                vec![BindingItem::new(
                    "reset",
                    BindingKind::FirmwareRom,
                    32768,
                    [1; 32],
                )],
            )])
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("{}:7", parent.display())),
            "{error}"
        );
        assert_eq!(error.diagnostics[0].path, "firmware.bios.rom");
        assert_eq!(
            origins.get("card[0].drives[0].disk").file,
            parent.canonicalize().unwrap()
        );
        assert_eq!(origins.get("firmware.bios.base").file, child);
    }
}

#[test]
fn inherited_source_changes_clear_old_selectors_and_assertions() {
    let tree = TempDir::new("binding-source-overrides");
    let parent = tree.path().join("parent.toml");
    std::fs::write(&parent,format!("schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nuse='bios'\nrom='reset'\nrole='system-bios'\nsize=32768\nsha256='{}'\n[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\ninserted={{use='dos',medium='disk'}}\n","11".repeat(32))).unwrap();
    let child = tree.path().join("child.yaml");
    std::fs::write(&child,"schema: 1\nextends: parent.toml\nfirmware:\n  bios: {image: replacement.bin}\ncard:\n  - id: fdc-765\n    drives: [{unit: 0, image: replacement.img}]\n").unwrap();
    let (document, _) = resolve(&child).unwrap();
    let rom = document.firmware.as_ref().unwrap().bios.as_ref().unwrap();
    assert!(
        rom.use_id.is_none()
            && rom.rom.is_none()
            && rom.role.is_none()
            && rom.size.is_none()
            && rom.sha256.is_none()
    );
    assert!(
        document.card.as_ref().unwrap()[0].drives.as_ref().unwrap()[0]
            .inserted
            .is_none()
    );
    std::fs::write(&parent,"schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='old.bin'\n[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\nimage='old.img'\n").unwrap();
    std::fs::write(&child,"schema: 1\nextends: parent.toml\nfirmware:\n  bios: {use: bios, rom: reset}\ncard:\n  - id: fdc-765\n    drives: [{unit: 0, inserted: {use: dos, medium: disk}}]\n").unwrap();
    let (document, origins) = resolve(&child).unwrap();
    assert!(document
        .firmware
        .as_ref()
        .unwrap()
        .bios
        .as_ref()
        .unwrap()
        .image
        .is_none());
    assert!(
        document.card.as_ref().unwrap()[0].drives.as_ref().unwrap()[0]
            .image
            .is_none()
    );
    assert_eq!(
        bindings::plan(&document, &origins).unwrap().slots().len(),
        2
    );
}

#[test]
fn child_zero_declaration_list_replaces_parent_list_and_retains_positions() {
    let tree = TempDir::new("binding-declarations");
    std::fs::write(tree.path().join("parent.toml"),"schema=1\ndisks=[{id='unused-parent',size=512}]\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nuse='bios'\nrom='reset'\n").unwrap();
    let child = tree.path().join("child.toml");
    std::fs::write(&child,"schema=1\nextends='parent.toml'\ndisks=[{id='system',size=512}]\n[[card]]\nkind='ide'\n[[card.drives]]\nchannel='master'\ndisk='system'\ngeometry={cylinders=1,heads=1,sectors=1}\n").unwrap();
    let (document, origins) = resolve(&child).unwrap();
    assert_eq!(document.disks.as_ref().unwrap().len(), 1);
    assert_eq!(origins.get("disks[0].id").file, child);
    bindings::plan(&document, &origins).unwrap();
}

#[test]
fn conflicting_fields_report_both_original_sites_in_each_syntax() {
    for (suffix, text, line) in [
        (
            "toml",
            "schema=1\n[firmware.bios]\nimage='bios.bin'\nuse='bios'\nrom='reset'\n",
            3,
        ),
        (
            "yaml",
            "schema: 1\nfirmware:\n  bios:\n    image: bios.bin\n    use: bios\n    rom: reset\n",
            4,
        ),
    ] {
        let tree = TempDir::new("binding-conflicts");
        let file = tree.path().join(format!("conflict.{suffix}"));
        std::fs::write(&file, text).unwrap();
        let error = resolve(&file).unwrap_err().to_string();
        assert!(error.contains("mutually exclusive"), "{error}");
        assert!(
            error.contains(&format!("{}:{line}", file.display())),
            "{error}"
        );
        assert!(
            error.contains(&format!("{}:{}", file.display(), line + 1)),
            "{error}"
        );
    }
}

#[test]
fn inherited_pin_conflicts_name_the_selector_and_assertion_layers() {
    let tree = TempDir::new("binding-pin-origins");
    let parent = tree.path().join("parent.toml");
    std::fs::write(
        &parent,
        "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nuse='bios'\nrom='reset'\n",
    )
    .unwrap();
    let child = tree.path().join("child.yaml");
    std::fs::write(
        &child,
        format!(
            "schema: 1\nextends: parent.toml\nfirmware:\n  bios:\n    sha256: '{}'\n",
            "22".repeat(32)
        ),
    )
    .unwrap();
    let (document, origins) = resolve(&child).unwrap();
    let error = bindings::plan(&document, &origins)
        .unwrap()
        .resolve(&[BindingUse::new(
            "bios",
            "manifest",
            vec![
                BindingItem::new("reset", BindingKind::FirmwareRom, 32768, [1; 32])
                    .with_roles(vec!["system-bios".into()]),
            ],
        )])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&format!("{}:6", parent.display())),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains(&format!("{}:5", child.display())),
        "{error}"
    );
}
