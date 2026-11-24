// SPDX-License-Identifier: AGPL-3.0-or-later
//! The lowering's tests that stay with the host.
//!
//! The lowering is `pc-config`'s (`backlog/docs/design/core-contract.md`
//! §14.2.1). These tests need what only this crate has: the YAML reader, the
//! host's `FileImageReader` and the metadata reader of `describe`, temporary
//! files, or the journal's error types. They keep the paths they had when the
//! lowering lived here.

use std::collections::BTreeMap;

use assembly::DeviceId;
// The journal's retention-name rule, which `paths`' test holds equal to the
// journal's own.
pub(crate) use pc_config::has_retention_marker;
use pc_config::AssemblyDiagnostics;

use crate::{lower_with_options as lower, Describe, Origins};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lotech_ems_card_fields_parse_in_toml_and_yaml() {
        let toml = crate::parse_toml(
            "ems.toml",
            "schema=1\n[[card]]\nkind='lotech-ems'\nport=0x264\nframe=0xD000\nsize_kib=1536\n",
        )
        .unwrap();
        let yaml = crate::parse_yaml("ems.yaml", "schema: 1\ncard:\n  - kind: lotech-ems\n    port: 0x264\n    frame: 0xD000\n    size_kib: 1536\n").unwrap();
        assert_eq!(toml.card, yaml.card);
        for field in ["port: -1", "frame: 65536", "size_kib: -1"] {
            assert!(crate::parse_yaml(
                "bad.yaml",
                &format!("schema: 1\ncard:\n  - kind: lotech-ems\n    {field}\n")
            )
            .is_err());
        }
    }

    #[test]
    fn invalid_assembly_drive_index_is_an_internal_error() {
        let diagnostics = AssemblyDiagnostics::without_sources(Origins::new("machine.toml"));
        let error = diagnostics.map(assembly::AssemblyError::Journal {
            drive_index: 9,
            source: journal_files::OpenError::Format(journal_files::Error::HeaderMagic),
        });
        assert_eq!(error.field, "internal error");
        assert!(
            error
                .message
                .contains("drive index 9, but only 0 drive origins exist"),
            "{error}"
        );
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use crate::FileImageReader;
    use assembly::capabilities::{self as caps, Value};

    #[test]
    fn metadata_uses_normal_lowering_defaults_and_file_length_without_reading_bytes() {
        let temp = crate::test_support::TempDir::new("metadata-lowering");
        std::fs::write(temp.path().join("bios.bin"), vec![0xF4; 32768]).unwrap();
        let path = temp.path().join("machine.toml");
        let text = "schema=1\n[board]\nmodel='ibm-5170'\n[cpu]\nfpu='80287'\n[board.rtc]\nstart='2001-02-03T04:05:06'\n[firmware.bios]\nimage='bios.bin'\n[[card]]\nkind='mda'\n";
        let file = crate::parse_toml(&path, text).unwrap();
        let origins = Origins::from_toml(&path, text).unwrap();
        let metadata = file.describe(&origins).unwrap();
        let config = lower(
            &file,
            &origins,
            &mut FileImageReader,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap();
        assert_eq!(metadata.config.images[0].base, 0xF8000);
        assert_eq!(metadata.config.images[0].base, config.images[0].base);
        assert_eq!(metadata.config.cpu, config.cpu);
        assert_eq!(metadata.config.devices, config.devices);
        assert_eq!(metadata.config.memory, config.memory);
        let mut expected = caps::Draft::new(config);
        expected.card_ids.push((DeviceId::Mda, "mda".into()));
        expected.image_paths.push((0, "firmware.bios".into()));
        expected.firmware_files.push((0, "bios.bin".into()));
        let fields = |draft: &caps::Draft| {
            caps::fields(draft)
                .into_iter()
                .map(|field| (field.path, field.value))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(fields(&metadata), fields(&expected));
        assert!(!caps::validate_draft(&metadata).ready());
    }

    #[test]
    fn indexed_options_work_without_a_bios_and_keep_catalogue_and_file_names_distinct() {
        let text = format!("schema=1\n[board]\nmodel='ibm-5170'\n[[firmware.option_roms]]\nimage='{}'\nbase=0xC8000\n[[firmware.option_roms]]\nimage='second.bin'\nbase=0xD0000\n[[card]]\nkind='mda'\n", assembly::KISELEV_XI8088.id);
        let file = crate::parse_toml("machine.toml", &text).unwrap();
        let mut draft = file.describe(&Origins::new("machine.toml")).unwrap();
        assert_eq!(draft.config.system_bios_image, None);
        assert_eq!(
            draft.image_paths,
            [
                (0, "firmware.option_roms[0]".into()),
                (1, "firmware.option_roms[1]".into())
            ]
        );
        assert!(draft.firmware_ids.is_empty());
        assert!(caps::fields(&draft)
            .iter()
            .any(|field| field.value == Value::FileName(assembly::KISELEV_XI8088.id.into())));
        draft.set("firmware.option_roms[1]", Value::Disconnected);
        draft.set(
            "firmware.bios",
            Value::Name(assembly::KISELEV_XI8088.id.into()),
        );
        let validation = caps::validate_draft(&draft);
        assert_eq!(validation.config.system_bios_image, Some(1));
        assert_eq!(validation.config.images.len(), 2);
        assert_eq!(validation.config.images[0].base, 0xC8000);
    }
}

#[cfg(test)]
mod clock_input_tests {
    #[test]
    fn nominal_clock_and_turbo_parse_in_both_syntaxes() {
        crate::parse_toml(
            "clock.toml",
            "schema=1\n[cpu]\nclock_mhz=4.77\nturbo={clock_mhz=33}\n",
        )
        .unwrap();
        crate::parse_yaml(
            "clock.yaml",
            "schema: 1\ncpu:\n  clock_mhz: 4.77\n  turbo: {clock_mhz: 33}\n",
        )
        .unwrap();
    }
}
