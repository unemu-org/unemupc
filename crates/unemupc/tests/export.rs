// SPDX-License-Identifier: AGPL-3.0-or-later
//! Offline exports round-trip through the host machine-file loader.

use pc_config::{image, launch};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use unemu_core_api::{
    test_support::{MemoryContent, MemoryExportSink},
    *,
};
use unemu_pc_core::PcCore;

#[path = "../../pc-harness/tests/support/fixtures.rs"]
mod fixtures;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// This adapter deliberately starts from the shipped file and the existing
// loader's effective config, so the expectation is independent of PcCore's
// offline drive-table projection.
fn launch_from_file(
    name: &str,
) -> (
    launch::Launch,
    BTreeMap<SlotId, Vec<u8>>,
    assembly::MachineConfig,
) {
    let path = root().join("machines").join(name);
    let (document, _) = machine_file::resolve(&path).unwrap();
    let machine = machine_file::load(&path).unwrap();
    let mut hardware = image::encode::machine(&document);
    image::defaults::materialize(&mut hardware, &machine).unwrap();
    let mut firmware = Vec::new();
    let mut content = Vec::new();
    let mut rom_index = 0;
    image::firmware_images(&mut hardware, |key, rom| -> image::Result<()> {
        let assembly::ImageSource::Inline(bytes) = &machine.images[rom_index].source else {
            panic!("fixture ROM must be inline")
        };
        rom_index += 1;
        let digest = image::Digest::of(bytes);
        let source = format!("assets/rom-{rom_index}.bin");
        rom.insert("image".into(), source.clone().into());
        rom.insert("sha256".into(), digest.hex().into());
        firmware.push(image::Firmware {
            key: key.into(),
            source,
            blob: digest,
            source_archive: None,
        });
        content.push(launch::ContentRow {
            digest,
            sha256: digest.sha256(),
            size: bytes.len() as u64,
        });
        Ok(())
    })
    .unwrap();
    let mut media = Vec::new();
    let mut slots = Vec::new();
    let mut readers = BTreeMap::new();
    let mut index = 0;
    image::drive_tables(&mut hardware, |controller, kind, _| -> image::Result<()> {
        let drive = &machine.drives[index];
        let assembly::ImageSource::Inline(bytes) = &drive.image else {
            panic!("fixture disk must be inline")
        };
        let id = format!("disk-{index}");
        index += 1;
        let digest = image::Digest::of(bytes);
        let slot = SlotId(drive.unit + if kind == "ide" { 2 } else { 0 });
        readers.insert(slot, bytes.to_vec());
        media.push(image::Medium {
            id: id.clone(),
            source: format!("assets/{id}.img"),
            head: digest,
            name: None,
            policy: image::Policy::from_write(&drive.write),
            geometry: image::Geometry::of_medium(drive.kind, bytes.len() as u64)?,
        });
        slots.push(image::Slot {
            controller: controller.into(),
            unit: drive.unit,
            catalogue: vec![id.clone()],
            catalogue_mode: false,
            inserted: Some(id),
        });
        content.push(launch::ContentRow {
            digest,
            sha256: digest.sha256(),
            size: bytes.len() as u64,
        });
        Ok(())
    })
    .unwrap();
    (
        launch::Launch {
            config: image::Config {
                schema: 1,
                platform: "ibm-pc".into(),
                machine: hardware,
                firmware,
                media,
                slots,
                nvram: None,
            },
            content,
            records: vec![],
        },
        readers,
        machine,
    )
}

#[test]
fn shipped_floppy_and_fixed_disk_configs_have_exact_offline_descriptions() {
    let (Some(_), Some(_), Some(_), Some(_), Some(_)) = (
        fixtures::fixture("kiselev-bios/bios-xi8088.bin"),
        fixtures::fixture("freedos/freedos.img"),
        fixtures::fixture("unemu-at-bios/bios-unemu-at.bin"),
        fixtures::fixture("xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin"),
        fixtures::fixture("hdd/fat16.img"),
    ) else {
        return;
    };
    for name in ["at-freedos.toml", "at-ide.toml"] {
        let (launch, readers, machine) = launch_from_file(name);
        let text = launch::encode(&launch).unwrap();
        let doc = ConfigDocument::new(launch::MEDIA_TYPE, &text);
        let descriptions = PcCore.describe_media("ibm-pc", doc).unwrap();
        assert_eq!(descriptions.len(), machine.drives.len());
        for description in descriptions {
            assert_eq!(
                description.size,
                Some(readers[&description.slot.id].len() as u64)
            );
            if description.slot.id.0 < 2 {
                assert_eq!(description.detail, "3.5-inch 1.44M");
            } else {
                assert_eq!(description.detail, "CHS 615/4/17, 20 MiB");
            }
        }
    }
}

#[test]
fn exported_machine_loads_with_relative_images_and_hash_only_firmware() {
    let (Some(_), Some(_), Some(_), Some(_)) = (
        fixtures::fixture("freedos/freedos.img"),
        fixtures::fixture("unemu-at-bios/bios-unemu-at.bin"),
        fixtures::fixture("xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin"),
        fixtures::fixture("hdd/fat16.img"),
    ) else {
        return;
    };
    let (launch, bytes, original) = launch_from_file("at-ide.toml");
    let text = launch::encode(&launch).unwrap();
    let doc = ConfigDocument::new(launch::MEDIA_TYPE, &text);
    let readers: BTreeMap<_, _> = bytes
        .iter()
        .map(|(&id, bytes)| (id, MemoryContent::new(bytes.clone())))
        .collect();
    let media: Vec<_> = readers
        .iter()
        .map(|(&id, reader)| ExportMedium::new(id, reader.size(), reader))
        .collect();
    let mut sink = MemoryExportSink::new();
    let report = PcCore
        .export(
            "ibm-pc",
            doc,
            ExportRequest::new("machine", &media),
            &mut sink,
        )
        .unwrap();
    let mut reordered = media.clone();
    reordered.reverse();
    let mut second = MemoryExportSink::new();
    assert_eq!(
        PcCore
            .export(
                "ibm-pc",
                doc,
                ExportRequest::new("machine", &reordered),
                &mut second
            )
            .unwrap(),
        report
    );
    assert_eq!(sink.outputs(), second.outputs());
    let directory = machine_file::test_support::TempDir::new("export-machine");
    for (name, bytes) in sink.outputs() {
        std::fs::write(directory.path().join(name), bytes).unwrap();
    }
    let file = directory.path().join("machine.toml");
    let firmware = machine_file::test_support::TempDir::new("export-firmware");
    for image in &original.images {
        let assembly::ImageSource::Inline(bytes) = &image.source else {
            panic!("fixture ROM must be inline")
        };
        std::fs::write(firmware.path().join(image::Digest::of(bytes).hex()), bytes).unwrap();
    }
    let loaded = machine_file::load_for_assembly_with_rom_dirs(
        &file,
        assembly::AssemblyOptions::default(),
        &[firmware.path().to_owned()],
    )
    .unwrap();
    let config = &loaded.config;
    assert_eq!(config.cpu, original.cpu);
    assert_eq!(config.clock, original.clock);
    assert_eq!(config.memory, original.memory);
    assert_eq!(config.devices, original.devices);
    assert_eq!(config.images, original.images);
    assert_eq!(config.drives.len(), original.drives.len());
    for drive in &config.drives {
        assert!(matches!(drive.write, assembly::WriteMode::Volatile));
        let slot = SlotId(
            drive.unit
                + if drive.controller == assembly::DeviceId::Ide {
                    2
                } else {
                    0
                },
        );
        assert_eq!(
            drive.kind,
            original
                .drives
                .iter()
                .find(|d| d.unit == drive.unit && d.controller == drive.controller)
                .unwrap()
                .kind
        );
        assert_eq!(
            drive.image,
            assembly::ImageSource::Inline(bytes[&slot].clone())
        );
    }
    assert!(report.notes.iter().any(|n| n.contains("readonly")));
    assert_eq!(report.outputs.len(), 3);
}
