// SPDX-License-Identifier: AGPL-3.0-or-later
//! Resolve host ROM paths before lowering, without editing the machine file.
//! See `backlog/docs/design/machine-files.md` § Frontends and § Validation.

use crate::firmware_discovery::{
    locate_firmware_with_roots, with_firmware_pins, FirmwarePin, FirmwareRoot, PinnedPart,
};
use crate::{schema::v1, LoadError, LowerError, MachineFile, Origins};
use std::path::Path;

/// `memory` holds ROMs already resolved from compressed firmware packages; their
/// key paths do not exist and are never searched for.
pub(crate) fn resolve(
    document: &mut MachineFile,
    origins: &Origins,
    roots: &[FirmwareRoot],
    warnings: &mut Vec<String>,
    memory: &std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
) -> Result<(), LoadError> {
    if pc_config::bindings::has_bindings(document) {
        return Err(LowerError {
            origin: origins.get("").clone(),
            field: String::new(),
            message: "unresolved content bindings; composition is required".into(),
            pin_origin: None,
            validation: None,
        }
        .into());
    }
    if roots.is_empty() {
        return Ok(());
    }
    let pins = document
        .firmware
        .iter()
        .flat_map(|firmware| {
            firmware
                .bios
                .iter()
                .chain(firmware.option_roms.iter().flatten())
        })
        .flat_map(|rom| {
            rom.sha256.iter().chain(
                rom.parts
                    .iter()
                    .flatten()
                    .filter_map(|part| part.sha256.as_ref()),
            )
        })
        .map(|pin| {
            FirmwarePin::from_catalogue(
                pin.as_str(),
                assembly::BoardModel::Ibm5170.firmware_catalogue(),
                None,
            )
        })
        .collect::<Vec<_>>();
    with_firmware_pins(&pins, || {
        if let Some(firmware) = &mut document.firmware {
            if let Some(bios) = &mut firmware.bios {
                rom(bios, "firmware.bios", origins, roots, warnings, memory)?;
            }
            for (index, option) in firmware.option_roms.iter_mut().flatten().enumerate() {
                rom(
                    option,
                    &format!("firmware.option_roms[{index}]"),
                    origins,
                    roots,
                    warnings,
                    memory,
                )?;
            }
        }
        Ok(())
    })
}

fn rom(
    rom: &mut v1::Rom,
    field: &str,
    origins: &Origins,
    roots: &[FirmwareRoot],
    warnings: &mut Vec<String>,
    memory: &std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
) -> Result<(), LoadError> {
    if (rom.image.is_some() && rom.parts.is_some()) || rom.parts.as_ref().is_some_and(Vec::is_empty)
    {
        return Ok(());
    }
    let in_memory = |image: &str| memory.contains_key(std::path::Path::new(image));
    if rom.image.as_deref().is_some_and(in_memory)
        || rom
            .parts
            .as_ref()
            .is_some_and(|parts| parts.iter().all(|p| in_memory(&p.image)))
    {
        return Ok(());
    }
    let pin = |pin: &v1::Sha256| {
        FirmwarePin::from_catalogue(
            pin.as_str(),
            assembly::BoardModel::Ibm5170.firmware_catalogue(),
            None,
        )
    };
    let image = rom
        .image
        .as_ref()
        .map(|image| origins.image_path(&format!("{field}.image"), image));
    let parts = rom
        .parts
        .iter()
        .flatten()
        .enumerate()
        .map(|(index, part)| PinnedPart {
            part: assembly::ImagePart {
                path: origins.image_path(&format!("{field}.parts[{index}].image"), &part.image),
                lane: match part.lane {
                    v1::Lane::Even => assembly::Lane::Even,
                    v1::Lane::Odd => assembly::Lane::Odd,
                },
                offset: part.offset,
            },
            pin: part.sha256.as_ref().map(pin),
        })
        .collect::<Vec<_>>();
    if rom.sha256.is_none() && parts.iter().all(|part| part.pin.is_none()) {
        return Ok(());
    }
    let source = locate_firmware_with_roots(
        roots,
        image.as_deref(),
        rom.sha256.as_ref().map(pin).as_ref(),
        &parts,
        warnings,
    )
    .map_err(|error| LowerError {
        origin: origins.get(field).clone(),
        field: field.into(),
        message: error.to_string(),
        pin_origin: None,
        validation: None,
    })?;
    match source {
        Some(assembly::ImageSource::File(path)) => {
            rom.image = Some(path_text(&path)?);
            rom.parts = None;
        }
        Some(assembly::ImageSource::Interleaved(found)) => {
            if let Some(parts) = &mut rom.parts {
                for (part, found) in parts.iter_mut().zip(found) {
                    part.image = path_text(&found.path)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<String, LoadError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        crate::ParseError::new(path, None, "discovered ROM path is not valid Unicode", None).into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware_discovery::{with_discovery_session, DiscoveryControl, RootOrigin};
    use crate::test_support::TempDir;
    use sha2::{Digest, Sha256};

    #[test]
    fn bios_and_option_roms_share_one_listing_and_progress_total() {
        let temp = TempDir::new("machine-firmware-session");
        let root = temp.path().join("roms");
        std::fs::create_dir(&root).unwrap();
        let bios = vec![1; 32768];
        let option = vec![2; 16384];
        std::fs::write(root.join("bios"), &bios).unwrap();
        std::fs::write(root.join("option"), &option).unwrap();
        let machine = temp.path().join("machine.toml");
        std::fs::write(&machine, format!("schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='absent-bios'\nsha256='{:x}'\n[[firmware.option_roms]]\nimage='absent-option'\nsha256='{:x}'\nbase=0xC0000\n", Sha256::digest(&bios), Sha256::digest(&option))).unwrap();
        let control = DiscoveryControl::default();
        with_discovery_session(None, false, &control, || {
            let prepared = crate::prepare_for_assembly_with_firmware_roots(
                &machine,
                &[FirmwareRoot::new(&root, RootOrigin::CommandLine)],
                &mut Vec::new(),
            )
            .unwrap();
            let firmware = prepared.document.firmware.unwrap();
            assert!(firmware.bios.unwrap().image.unwrap().ends_with("bios"));
            assert!(firmware.option_roms.unwrap()[0]
                .image
                .as_ref()
                .unwrap()
                .ends_with("option"));
        });
        let progress = control.snapshot();
        assert_eq!(progress.listed, 2);
        assert_eq!(progress.candidates, 2);
        assert_eq!(progress.checked, 2);
        assert_eq!(progress.files_read, 2);
        assert_eq!(progress.matches, 2);
    }
}
