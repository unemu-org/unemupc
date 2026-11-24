// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repository-file hardware parity with deterministic synthetic image inputs.
//! See `backlog/docs/design/machine-images.md` §6.

use assembly::{AssemblyOptions, ImageSource, MachineConfig};
use machine_file::{lower_with_options, resolve, schema::v1, ImageReader};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};

struct Images;
impl ImageReader for Images {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let name = path.file_name().unwrap().to_str().unwrap();
        let size = if name.ends_with(".img") {
            if name.contains("1200") {
                1_228_800
            } else if name.contains("fat16") || name == "games.img" {
                615 * 4 * 17 * 512
            } else {
                1_474_560
            }
        } else if name == "bios-1.17.0-isa.bin" {
            131072
        } else if name.contains("ide_xtp") {
            8192
        } else {
            32768
        };
        let mut bytes = vec![0; size];
        if name.contains("ide_xtp") || name.contains("vga-1987") || name.contains("vgabios") {
            let offset = if name.contains("vga-1987") { 8192 } else { 0 };
            bytes[offset..offset + 3].copy_from_slice(&[0x55, 0xAA, ((size - offset) / 512) as u8]);
            bytes[size - 1] =
                0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)));
        }
        Ok(bytes)
    }
}

fn synthetic(document: &mut v1::MachineFile) {
    let firmware_loaded_video = document.board.as_ref().is_some_and(|board| {
        matches!(
            board.model,
            Some(v1::BoardModel::Generic386Isa | v1::BoardModel::Generic486Isa)
        )
    });
    let native_ata = firmware_loaded_video
        && document.card.as_ref().is_some_and(|cards| {
            cards
                .iter()
                .any(|card| card.kind == Some(v1::CardKind::Ide))
        });
    if let Some(firmware) = &mut document.firmware {
        if native_ata {
            // Synthetic bytes cannot identify SeaBIOS's native ATA provider.
            if let Some(bios) = &mut firmware.bios {
                bios.provides = Some(vec![
                    v1::FirmwareService::Floppy,
                    v1::FirmwareService::FixedDisk,
                ]);
            }
        }
        // Only an identified system BIOS establishes the C0000h video-ROM
        // scan. Move video ROMs to an extension address that no repository
        // machine uses, so their hardware is still fingerprinted.
        for rom in firmware.option_roms.iter_mut().flatten() {
            if rom.base == Some(0xC0000) && !firmware_loaded_video {
                rom.base = Some(0xE0000);
            }
        }
        for rom in firmware
            .bios
            .iter_mut()
            .chain(firmware.option_roms.iter_mut().flatten())
        {
            rom.sha256 = None;
            if rom.provides.is_none() {
                rom.provides = Some(vec![v1::FirmwareService::Floppy]);
            }
            for part in rom.parts.iter_mut().flatten() {
                part.sha256 = None;
            }
        }
    }
    for card in document.card.iter_mut().flatten() {
        for drive in card.drives.iter_mut().flatten() {
            drive.sha256 = None;
            drive.write = if drive.media.is_some() || drive.empty.is_some() {
                None
            } else {
                Some(v1::WriteMode::Volatile)
            };
            drive.journal = None;
            drive.keep_generations = None;
            for medium in drive.media.iter_mut().flatten() {
                medium.sha256 = None;
                medium.write = Some(v1::WriteMode::Volatile);
                medium.journal = None;
                medium.keep_generations = None;
            }
        }
    }
}

fn source_digest(source: &mut ImageSource, digest: &mut Sha256) {
    match source {
        ImageSource::Inline(bytes) | ImageSource::RomContainer { bytes, .. } => {
            digest.update(Sha256::digest(bytes));
        }
        _ => {}
    }
    *source = ImageSource::Blank {
        size: source.known_len().unwrap_or(0),
    };
}

fn fingerprint(mut config: MachineConfig) -> String {
    let mut digest = Sha256::new();
    for image in &mut config.images {
        source_digest(&mut image.source, &mut digest);
    }
    for drive in &mut config.drives {
        source_digest(&mut drive.image, &mut digest);
        for medium in drive.media.iter_mut().flat_map(|list| &mut list.entries) {
            source_digest(&mut medium.image, &mut digest);
        }
    }
    digest.update(format!("{config:?}"));
    format!("{:x}", digest.finalize())
}

fn machine_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            machine_files(&path, files);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("toml" | "yaml" | "yml")
        ) {
            files.push(path);
        }
    }
}

#[test]
fn repository_machine_file_parity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut files = Vec::new();
    machine_files(&root.join("machines"), &mut files);
    files.sort();
    let mut observed = Vec::new();
    for file in files {
        let outcome = match resolve(&file) {
            Err(error) => format!(
                "resolve:{}",
                error.to_string().replace(root.to_str().unwrap(), "<repo>")
            ),
            Ok((mut document, origins)) => {
                synthetic(&mut document);
                match lower_with_options(
                    &document,
                    &origins,
                    &mut Images,
                    AssemblyOptions {
                        allow_unlisted_firmware: true,
                    },
                ) {
                    Ok(config) => {
                        machine_file::validate(&config).unwrap();
                        if file.ends_with("machines/at-games-open.toml") {
                            // The fingerprint must cover the open game hardware.
                            let has = |kind: fn(&assembly::DeviceKind) -> bool| {
                                config.devices.iter().any(|device| kind(&device.kind))
                            };
                            assert!(has(|k| matches!(k, assembly::DeviceKind::VgaRegisters(_))));
                            assert!(has(|k| matches!(k, assembly::DeviceKind::LotechEms(_))));
                            assert!(has(|k| matches!(k, assembly::DeviceKind::Ide(_))));
                            assert!(has(|k| matches!(k, assembly::DeviceKind::Sb16(_))));
                            // A: 1.2M, the empty 1.44M B: and the fixed disk.
                            assert_eq!(config.drives.len(), 3);
                            assert_eq!(config.images[1].base, 0xE0000);
                        }
                        fingerprint(config)
                    }
                    Err(error) => format!(
                        "error:{}:{}",
                        error.field,
                        error.message.replace(root.to_str().unwrap(), "<repo>")
                    ),
                }
            }
        };
        observed.push((
            file.strip_prefix(&root).unwrap().display().to_string(),
            outcome,
        ));
    }
    let expected: Vec<(String, String)> = vec![
        ("machines/at-freedos-from-preset.toml".into(), "3b07a7a4089cf617f743f2f7f5c80e3c85a3f7bdc0e5b6f3b6eb835946afac0c".into()),
        ("machines/at-freedos.toml".into(), "3b07a7a4089cf617f743f2f7f5c80e3c85a3f7bdc0e5b6f3b6eb835946afac0c".into()),
        ("machines/at-freedos.yaml".into(), "3b07a7a4089cf617f743f2f7f5c80e3c85a3f7bdc0e5b6f3b6eb835946afac0c".into()),
        // Open VGA BIOS and VGA card, an empty 1.44M B:; the video ROM is relocated by synthetic().
        // Its floppy media lists name their power-on entry, the first.
        ("machines/at-games-open.toml".into(), "f80bf12dce409214e0f4834b6586ffdc42993bea15b4ffbfe116f6a70aef8cf8".into()),
        // The video ROM is relocated by synthetic(); the media lists as above.
        ("machines/at-games.toml".into(), "803c59aaa00a7b9c78a66a52ea37193d7bde6e5bb63a4077322e5f3e52e02c74".into()),
        // The open AT BIOS's mono display switch.
        ("machines/at-ide.toml".into(), "e42872c37de7d3674033a09efaf304ff1e47f95392af3baa9f2ed2553ab8a60d".into()),
        ("machines/generic-386-freedos.toml".into(), "9f1c1f819169673c59131c097b33720b71d3f3bb151fee25d42e489ee9b2338c".into()),
        ("machines/generic-386-ide.toml".into(), "49d6dcf2620f0fceb502015e04f19257874d6dffeb207d0ac7de577c6a7263db".into()),
        ("machines/generic-386-seabios.toml".into(), "fb5592babbaa5a31093d1501e6a070dc6af8ee4634cf713f4446a8a4425a6265".into()),
        ("machines/generic-486-freedos.toml".into(), "e71c732d666f2ecb48e4a7655554f9d3227a4db10efc3950aa23c99d7fda988d".into()),
        ("machines/generic-486-ide.toml".into(), "565fc6eb2eca9b325cdd75bf59eadd826308be6efdb3f41dd602dd274339683b".into()),
        ("machines/generic-486-seabios.toml".into(), "18df36806e3236a87a678d52be3e5e5bf8809ba4475f2280d6ad8491899eaf9d".into()),
        ("machines/presets/ibm-5170.toml".into(), "resolve:<repo>/machines/presets/ibm-5170.toml:16: firmware.bios.image: required fields are missing: firmware.bios.image (<repo>/machines/presets/ibm-5170.toml:16); inheritance chain: <repo>/machines/presets/ibm-5170.toml".into()),
    ];
    assert_eq!(observed, expected);
}
