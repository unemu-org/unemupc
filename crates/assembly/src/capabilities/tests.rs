// SPDX-License-Identifier: AGPL-3.0-or-later
//! Isolated configuration tests, without CPUs, host images or UI state.
//! See `backlog/docs/design/machine-images.md` §6.

use super::*;

fn draft() -> Draft {
    let mut config = crate::bios(vec![0xF4; 0x8000], Some(vec![0; 1_474_560]));
    config.devices.push(CardKind::Sb16.device());
    let mut ems = CardKind::LotechEms.device();
    if let DeviceKind::LotechEms(cfg) = &mut ems.kind {
        cfg.frame = 0xD000;
    }
    config
        .memory
        .mmio
        .push(CardKind::LotechEms.mmio(&ems).unwrap());
    config.devices.push(ems);
    config.validate().unwrap();
    let mut draft = Draft::new(config);
    draft.revision = 17;
    draft
}

fn conformance(draft: &Draft, path: &str) {
    let options = choices(draft, path);
    assert!(!options.is_empty(), "{path}");
    let baseline = validate_draft(draft).diagnostics;
    for option in options {
        let mut edited = draft.clone();
        edited.set(path, option.value.clone());
        let result = validate_draft(&edited);
        assert_eq!(option.revision, 17);
        assert_eq!(
            option.diagnostics, result.diagnostics,
            "{path}: {:?}",
            option.value
        );
        if option.enabled {
            for error in result
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error)
            {
                assert!(baseline.contains(error), "new error for {path}: {error:?}");
                assert!(!error.field_paths.iter().any(|field| field == path));
            }
            if !result.has_errors() {
                result.config.validate().unwrap();
            }
            assert!(option.reason.is_none());
        } else {
            let reason = option.reason.unwrap();
            assert!(!reason.is_empty());
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|d| d.severity == Severity::Error && d.reason == reason)
                .unwrap();
            assert!(
                result.config.validate().is_err()
                    || matches!(
                        diagnostic.code,
                        "field-domain" | "unknown-field" | "bios-drive-type"
                    ),
                "blocked without a validator error or a known draft-only rule: {diagnostic:?}"
            );
        }
    }
}

#[test]
fn sb16_choices_conform_to_validation() {
    let draft = draft();
    for field in ["base", "irq", "dma8", "dma16"] {
        conformance(&draft, &format!("card[sb16].{field}"));
    }
    let bases = choices(&draft, "card[sb16].base");
    assert!(bases
        .iter()
        .any(|c| c.value == Value::Integer(0x240) && c.enabled));
    let conflict = bases
        .iter()
        .find(|c| c.value == Value::Integer(0x260))
        .unwrap();
    assert!(!conflict.enabled);
    let diagnostic = conflict
        .diagnostics
        .iter()
        .find(|d| d.code == "port-conflict")
        .unwrap();
    assert_eq!(diagnostic.resource, Some(Resource::Port(0x260)));
    assert!(diagnostic
        .conflict_partners
        .contains(&"card[lotech-ems]".into()));
}

#[test]
fn ems_choices_conform_to_validation_including_system_rom() {
    let draft = draft();
    for field in ["port", "frame", "size_kib"] {
        conformance(&draft, &format!("card[lotech-ems].{field}"));
    }
    let e000 = choices(&draft, "card[lotech-ems].frame")
        .into_iter()
        .find(|c| c.value == Value::Integer(0xE000))
        .unwrap();
    assert!(!e000.enabled);
    assert!(e000
        .diagnostics
        .iter()
        .any(|d| d.code == "ems-system-rom" && d.field_paths == ["card[lotech-ems].frame"]));
}

#[test]
fn memory_choices_conform_and_scalar_edits_use_descriptor_domains() {
    let draft = draft();
    conformance(&draft, "memory.conventional_kib");
    conformance(&draft, "memory.extended_kib");
    for (path, value) in [
        ("memory.conventional_kib", 641),
        ("memory.conventional_kib", 639),
        ("memory.extended_kib", 65),
        ("memory.extended_kib", 15296),
    ] {
        let mut edited = draft.clone();
        edited.set(path, Value::Integer(value));
        assert!(validate_draft(&edited)
            .diagnostics
            .iter()
            .any(|d| d.code == "field-domain" && d.field_paths == [path]));
    }
}

#[test]
fn video_choices_conform_and_exclude_overlapping_adapters() {
    let draft = draft();
    conformance(&draft, "card[vga-text].kind");
    let options = choices(&draft, "card[vga-text].kind");
    assert!(options
        .iter()
        .any(|c| c.value == Value::Name("vga-text".into()) && c.enabled));
    assert!(options
        .iter()
        .any(|c| c.value == Value::Name("vga".into()) && !c.enabled));
}

#[test]
fn drive_choices_conform_to_media_geometry() {
    let draft = draft();
    conformance(&draft, "card[fdc-765].drives[unit=0].type");
    let options = choices(&draft, "card[fdc-765].drives[unit=0].type");
    assert_eq!(options.iter().filter(|c| c.enabled).count(), 1);
}

#[test]
fn known_bios_drive_domain_runs_without_firmware_bytes() {
    let mut draft = draft();
    draft.config.drives[0].media = Some(FloppyMediaList {
        entries: Vec::new(),
        empty: true,
        inserted: 0,
    });
    draft.set(
        "firmware.bios",
        Value::Name("ibm-5170-bios-1985-06-10".into()),
    );
    conformance(&draft, "card[fdc-765].drives[unit=0].type");
    let options = choices(&draft, "card[fdc-765].drives[unit=0].type");
    let hd144 = options
        .iter()
        .find(|c| c.value == Value::Name("1.44M".into()))
        .unwrap();
    assert!(!hd144.enabled);
    assert!(hd144
        .diagnostics
        .iter()
        .any(|d| d.code == "bios-drive-type"));
    assert!(options
        .iter()
        .any(|c| c.value == Value::Name("1.2M".into()) && c.enabled));
}

#[test]
fn replacing_own_irq_and_decode_does_not_conflict_with_previous_resources() {
    let mut draft = draft();
    for value in [5, 7] {
        draft.set("card[sb16].irq", Value::Integer(value));
        let result = validate_draft(&draft);
        assert!(!result.has_errors(), "{:?}", result.diagnostics);
        assert_eq!(
            result
                .config
                .devices
                .iter()
                .find(|d| d.id == DeviceId::Sb16)
                .unwrap()
                .irq,
            Some(value as u8)
        );
    }
    draft.set("card[lotech-ems].port", Value::Integer(0x264));
    assert!(!validate_draft(&draft).has_errors());
}

#[test]
fn atomic_edits_can_swap_ports_and_irqs_in_either_order() {
    let mut draft = draft();
    for edits in [
        vec![("card[sb16].base", 0x240), ("card[lotech-ems].port", 0x264)],
        vec![("card[lotech-ems].port", 0x264), ("card[sb16].base", 0x240)],
    ] {
        draft.edits.clear();
        for (path, value) in edits {
            draft.set(path, Value::Integer(value));
        }
        let result = validate_draft(&draft);
        assert!(!result.has_errors(), "{:?}", result.diagnostics);
        result.config.validate().unwrap();
    }
    draft.edits.clear();
    draft
        .config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Uart)
        .unwrap()
        .irq = Some(7);
    draft.set("card[sb16].irq", Value::Integer(7));
    assert!(validate_draft(&draft).has_errors());
    draft.set("card[uart-16550].irq", Value::Integer(5));
    assert!(!validate_draft(&draft).has_errors());
}

#[test]
fn ide_channel_resources_are_evaluated_atomically() {
    let mut draft = draft();
    draft.config.devices.push(CardKind::Ide.device());
    draft.config.images[0]
        .provides
        .push(FirmwareService::FixedDisk);
    draft.set("card[ide].port", Value::Integer(0x170));
    assert_eq!(
        describe(&draft, "card[ide].irq").unwrap().default,
        Some(Value::Integer(15))
    );
    assert!(!validate_draft(&draft).has_errors());
    draft.set("card[ide].irq", Value::Integer(15));
    let result = validate_draft(&draft);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    result.config.validate().unwrap();
    assert_eq!(
        result
            .config
            .devices
            .iter()
            .find(|d| d.id == DeviceId::Fdc)
            .unwrap()
            .ports,
        FdcCfg::port_windows(false)
    );
}

#[test]
fn missing_firmware_is_pending_and_does_not_disable_unrelated_choices() {
    let mut draft = draft();
    draft.config.images.clear();
    draft.config.system_bios_image = None;
    let result = validate_draft(&draft);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Pending && d.field_paths == ["firmware.bios"]));
    assert!(!result.ready());
    assert!(choices(&draft, "card[sb16].irq")
        .iter()
        .any(|c| c.value == Value::Integer(7) && c.enabled));
    draft.config.drives[0].media = Some(FloppyMediaList {
        entries: vec![FloppyMedium {
            name: "disk.img".into(),
            image: ImageSource::File("disk.img".into()),
            expected_sha256: Some([0; 32]),
            write: WriteMode::Volatile,
        }],
        empty: false,
        inserted: 0,
    });
    assert!(validate_draft(&draft)
        .diagnostics
        .iter()
        .any(|d| d.code == "media-pending" && d.severity == Severity::Pending));
}

#[test]
fn diagnostics_keep_custom_card_ids_routed_irqs_and_all_independent_checks() {
    let mut draft = draft();
    draft.card_ids.push((DeviceId::Sb16, "sound".into()));
    draft
        .config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Uart)
        .unwrap()
        .irq = Some(9);
    draft.set("card[sound].irq", Value::Integer(2));
    draft.set("card[sound].base", Value::Integer(0x260));
    let result = validate_draft(&draft);
    let irq = result
        .diagnostics
        .iter()
        .find(|d| d.code == "irq-conflict")
        .unwrap();
    assert_eq!(irq.resource, Some(Resource::Irq(9)));
    assert!(irq.field_paths.contains(&"card[sound].irq".into()));
    assert!(result.diagnostics.iter().any(|d| d.code == "port-conflict"));
}

#[test]
fn uart_domain_is_pageable_and_fixed_fdc_wiring_is_checked() {
    let draft = draft();
    assert_eq!(
        choices_page(&draft, "card[uart-16550].port", 0x3F8, 1).len(),
        1
    );
    conformance(&draft, "card[fdc-765].irq");
    conformance(&draft, "card[fdc-765].dma");
    assert!(
        !choices(&draft, "card[fdc-765].irq")
            .iter()
            .find(|c| c.value == Value::Integer(0))
            .unwrap()
            .enabled
    );
    assert!(
        !choices(&draft, "card[fdc-765].dma")
            .iter()
            .find(|c| c.value == Value::Integer(1))
            .unwrap()
            .enabled
    );
    assert!(!choices_page(&draft, "card[uart-16550].port", 0x260, 1)[0].enabled);
    let mut switched = draft.clone();
    switched.set("card[uart-16550].port", Value::Integer(0x2F8));
    assert_eq!(
        describe(&switched, "card[uart-16550].irq").unwrap().default,
        Some(Value::Integer(3))
    );
}

#[test]
fn kind_and_resource_edits_use_the_new_kind_domain_in_any_order() {
    let mut draft = draft();
    draft.config.devices.retain(|d| d.id != DeviceId::Mda);
    draft
        .config
        .memory
        .mmio
        .retain(|w| w.device != DeviceId::Mda);
    draft.config.devices.retain(|d| d.id != DeviceId::Sb16);
    draft.config.display_switch = DisplaySwitch::Auto;
    draft.set("card[vga-text].base", Value::Integer(0x240));
    draft.set("card[vga-text].kind", Value::Name("sb16".into()));
    let result = validate_draft(&draft);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    assert!(result
        .config
        .devices
        .iter()
        .any(|d| matches!(d.kind, DeviceKind::Sb16(Sb16Cfg { base: 0x240, .. }))));
    result.config.validate().unwrap();
}

#[test]
fn pending_video_firmware_reserves_its_known_range_and_reports_metadata_requirements() {
    let mut draft = draft();
    draft.set(
        "firmware.video",
        Value::Name(FirmwareEntry::IBM_VGA_1987.id.into()),
    );
    draft.set("card[lotech-ems].frame", Value::Integer(0xC000));
    let result = validate_draft(&draft);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "rom-mmio-conflict"));
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Pending));
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.field_paths == ["firmware.video"] && d.severity == Severity::Error));
}

#[test]
fn staged_swap_keeps_unrelated_choices_enabled_and_conflicting_fields_blocked() {
    let mut draft = draft();
    draft
        .config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Uart)
        .unwrap()
        .irq = Some(7);
    draft.set("card[sb16].irq", Value::Integer(7));
    assert!(validate_draft(&draft).has_errors());
    assert!(choices(&draft, "memory.extended_kib")
        .iter()
        .all(|c| c.enabled));
    assert!(
        choices(&draft, "card[sb16].base")
            .iter()
            .find(|c| c.value == Value::Integer(0x240))
            .unwrap()
            .enabled
    );
    assert!(
        !choices(&draft, "card[sb16].irq")
            .iter()
            .find(|c| c.value == Value::Integer(7))
            .unwrap()
            .enabled
    );
    assert!(
        !choices(&draft, "card[uart-16550].irq")
            .iter()
            .find(|c| c.value == Value::Integer(7))
            .unwrap()
            .enabled
    );
    assert!(
        choices(&draft, "card[uart-16550].irq")
            .iter()
            .find(|c| c.value == Value::Integer(5))
            .unwrap()
            .enabled
    );
    conformance(&draft, "memory.extended_kib");
    conformance(&draft, "card[sb16].base");
    let blocked = choices(&draft, "card[sb16].base")
        .into_iter()
        .find(|c| c.value == Value::Integer(0x260))
        .unwrap();
    assert!(!blocked.enabled);
    assert!(blocked
        .diagnostics
        .iter()
        .any(|d| d.code == "port-conflict" && blocked.reason.as_ref() == Some(&d.reason)));
    let mut named = draft.clone();
    named.edits.clear();
    named.card_ids.push((DeviceId::Sb16, "sound.card".into()));
    named.set("card[sound.card].irq", Value::Integer(7));
    assert!(
        choices(&named, "card[sound.card].base")
            .iter()
            .find(|c| c.value == Value::Integer(0x240))
            .unwrap()
            .enabled
    );
    draft.set("card[uart-16550].irq", Value::Integer(5));
    assert!(!validate_draft(&draft).has_errors());
}

#[test]
fn ide_port_choice_carries_derived_irq_and_preserves_explicit_edits() {
    let mut draft = draft();
    draft.config.devices.push(CardKind::Ide.device());
    draft.config.images[0]
        .provides
        .push(FirmwareService::FixedDisk);
    assert!(
        choices(&draft, "card[ide].port")
            .iter()
            .find(|c| c.value == Value::Integer(0x170))
            .unwrap()
            .enabled
    );
    draft.set("card[ide].port", Value::Integer(0x170));
    let result = validate_draft(&draft);
    result.config.validate().unwrap();
    assert_eq!(
        result
            .config
            .devices
            .iter()
            .find(|d| d.id == DeviceId::Ide)
            .unwrap()
            .irq,
        Some(15)
    );
    let mut back = Draft::new(result.config);
    back.set("card[ide].port", Value::Integer(0x1F0));
    let primary = validate_draft(&back);
    primary.config.validate().unwrap();
    assert_eq!(
        primary
            .config
            .devices
            .iter()
            .find(|d| d.id == DeviceId::Ide)
            .unwrap()
            .irq,
        Some(14)
    );
    for port_first in [false, true] {
        let mut explicit = draft.clone();
        explicit.edits.clear();
        let edits = if port_first {
            [("card[ide].port", 0x170), ("card[ide].irq", 14)]
        } else {
            [("card[ide].irq", 14), ("card[ide].port", 0x170)]
        };
        for (path, value) in edits {
            explicit.set(path, Value::Integer(value));
        }
        assert_eq!(
            validate_draft(&explicit)
                .config
                .devices
                .iter()
                .find(|d| d.id == DeviceId::Ide)
                .unwrap()
                .irq,
            Some(14)
        );
        assert!(validate_draft(&explicit).has_errors());
    }
}

#[test]
fn ems_rom_conflicts_use_file_paths_and_plain_rom_names() {
    let mut video = draft();
    video.config.images[0].source = ImageSource::File("bios.bin".into());
    video.config.images.push(LoadImage {
        base: 0xC0000,
        source: ImageSource::Inline(vec![0; 0x6000]),
        kind: ImageKind::Rom,
        provides: Vec::new(),
    });
    video.set("card[lotech-ems].frame", Value::Integer(0xC000));
    let result = validate_draft(&video);
    let conflict = result
        .diagnostics
        .iter()
        .find(|d| d.code == "rom-mmio-conflict")
        .unwrap();
    assert_eq!(
        conflict.field_paths,
        ["firmware.option_roms[0]", "card[lotech-ems].frame"]
    );
    assert!(conflict.reason.contains("VGA BIOS"));
    assert!(!conflict.reason.contains("LotechEms"));
    let mut system = draft();
    system.config.images[0].base = 0xE0000;
    system.config.images[0].source = ImageSource::Inline(vec![0; 0x20000]);
    system.set("card[lotech-ems].frame", Value::Integer(0xE000));
    let result = validate_draft(&system);
    let conflict = result
        .diagnostics
        .iter()
        .find(|d| d.code == "rom-mmio-conflict")
        .unwrap();
    assert!(conflict.field_paths.contains(&"firmware.bios".into()));
    assert!(!conflict
        .field_paths
        .iter()
        .any(|p| p.starts_with("firmware.option_roms")));
    assert!(conflict.reason.contains("system BIOS"));
}

#[test]
fn pending_option_rom_path_excludes_the_system_bios_index() {
    let mut draft = draft();
    draft.config.images.push(LoadImage {
        base: 0xC8000,
        source: ImageSource::File("option.bin".into()),
        kind: ImageKind::Rom,
        provides: Vec::new(),
    });
    let result = validate_draft(&draft);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "firmware-pending" && d.field_paths == ["firmware.option_roms[0]"]));
    draft.config.images.swap(0, 1);
    draft.config.system_bios_image = Some(1);
    assert!(validate_draft(&draft)
        .diagnostics
        .iter()
        .any(|d| d.code == "firmware-pending" && d.field_paths == ["firmware.option_roms[0]"]));
}

#[test]
fn dma_and_mmio_conflicts_use_plain_device_names() {
    let mut dma = draft();
    dma.set("card[sb16].dma8", Value::Integer(2));
    let error = diagnostic::from_error(
        &dma,
        &ConfigError::DmaConflict {
            a: DeviceId::Fdc,
            b: DeviceId::Sb16,
            controller: DeviceId::Dma,
            channel: 1,
        },
    );
    assert!(error.reason.contains("floppy controller"));
    assert!(error.reason.contains("Sound Blaster 16"));
    assert!(!error.reason.contains("Dma"));
    let error = diagnostic::from_error(
        &dma,
        &ConfigError::MmioOverlap {
            a: DeviceId::Vga,
            b: DeviceId::Mda,
            addr: 0xB0000,
        },
    );
    assert!(error.reason.contains("VGA card"));
    assert!(error.reason.contains("monochrome adapter"));
}

#[test]
fn port_descriptors_expose_irq_dependencies_and_follow_defaults_only() {
    let mut draft = draft();
    draft.config.devices.push(CardKind::Ide.device());
    for card in ["ide", "uart-16550"] {
        assert_eq!(
            describe(&draft, &format!("card[{card}].port"))
                .unwrap()
                .dependent_fields,
            [format!("card[{card}].irq")]
        );
        assert!(describe(&draft, &format!("card[{card}].irq"))
            .unwrap()
            .dependent_fields
            .is_empty());
    }
    draft
        .config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Uart)
        .unwrap()
        .irq = Some(4);
    draft.set("card[uart-16550].port", Value::Integer(0x2F8));
    assert_eq!(
        validate_draft(&draft)
            .config
            .devices
            .iter()
            .find(|d| d.id == DeviceId::Uart)
            .unwrap()
            .irq,
        Some(3)
    );
    draft
        .config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Uart)
        .unwrap()
        .irq = Some(5);
    assert_eq!(
        validate_draft(&draft)
            .config
            .devices
            .iter()
            .find(|d| d.id == DeviceId::Uart)
            .unwrap()
            .irq,
        Some(5)
    );
    draft
        .config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Uart)
        .unwrap()
        .irq = None;
    assert_eq!(
        validate_draft(&draft)
            .config
            .devices
            .iter()
            .find(|d| d.id == DeviceId::Uart)
            .unwrap()
            .irq,
        None
    );
}

#[test]
fn explicit_image_paths_are_preserved_for_conflicts_and_pending_inputs() {
    let mut draft = draft();
    draft.config.images.push(LoadImage {
        base: 0xC0000,
        source: ImageSource::File("video.bin".into()),
        kind: ImageKind::Rom,
        provides: Vec::new(),
    });
    draft.image_paths.push((1, "firmware.video".into()));
    assert!(validate_draft(&draft)
        .diagnostics
        .iter()
        .any(|d| d.code == "firmware-pending" && d.field_paths == ["firmware.video"]));
    let error = diagnostic::from_error(
        &draft,
        &ConfigError::ImageOverlapsMmio {
            image_index: 1,
            device: DeviceId::LotechEms,
            addr: 0xC0000,
        },
    );
    assert_eq!(
        error.field_paths,
        ["firmware.video", "card[lotech-ems].frame"]
    );
    assert!(error.conflict_partners.contains(&"firmware.video".into()));
}

#[test]
fn ems_descriptors_share_the_device_domains() {
    assert_eq!(EMS_PORTS, devices::LotechEms::PORTS);
    assert_eq!(EMS_FRAMES, devices::LotechEms::FRAMES);
    assert_eq!(EMS_SIZES_KIB, devices::LotechEms::SIZES_KIB);
    for port in EMS_PORTS {
        assert!(devices::LotechEms::valid_port(*port));
    }
    for frame in EMS_FRAMES {
        assert!(devices::LotechEms::valid_frame(*frame));
    }
    for size in EMS_SIZES_KIB {
        assert!(devices::LotechEms::valid_size_kib(*size));
    }
    assert!(!devices::LotechEms::valid_port(0x261));
    assert!(!devices::LotechEms::valid_frame(0xC800));
    assert!(!devices::LotechEms::valid_size_kib(513));
}

#[test]
fn vga_model_and_video_memory_are_card_fields_that_follow_the_kind() {
    let mut base = Draft::for_board(BoardModel::Generic486Isa);
    base.config.devices.retain(|d| d.id != DeviceId::Vga);
    base.config
        .memory
        .mmio
        .retain(|w| w.device != DeviceId::Vga);
    let vga = CardKind::Vga.device();
    base.config
        .memory
        .mmio
        .push(CardKind::Vga.mmio(&vga).unwrap());
    base.config.devices.push(vga);
    let model = describe(&base, "card[vga].model").unwrap();
    assert_eq!(
        model.domain,
        Domain::Values(vec![
            Value::Name("ibm".into()),
            Value::Name("bochs-vbe".into())
        ])
    );
    assert_eq!(model.default, Some(Value::Name("ibm".into())));
    let memory = describe(&base, "card[vga].memory_mib").unwrap();
    assert_eq!(
        memory.domain,
        Domain::Values([4, 8, 16].map(Value::Integer).to_vec())
    );
    assert_eq!(
        (memory.default, memory.unit),
        (Some(Value::Integer(16)), Some("MiB"))
    );

    let mut draft = base.clone();
    draft.set("card[vga].model", Value::Name("bochs-vbe".into()));
    draft.set("card[vga].memory_mib", Value::Integer(8));
    // Naming the same kind again keeps the model.
    draft.set("card[vga].kind", Value::Name("vga".into()));
    let result = validate_draft(&draft);
    let card = result
        .config
        .devices
        .iter()
        .find(|d| d.id == DeviceId::Vga)
        .unwrap();
    assert_eq!(
        card.kind,
        DeviceKind::BochsVbe(BochsVbeCfg { memory_mib: 8 })
    );
    assert_eq!(card.ports, BochsVbeCfg::port_windows());
    let sheet = fields(&draft);
    let value = |path: &str| {
        sheet
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.value.clone())
    };
    assert_eq!(
        value("card[vga].model"),
        Some(Value::Name("bochs-vbe".into()))
    );
    assert_eq!(value("card[vga].memory_mib"), Some(Value::Integer(8)));

    // Another kind drops both.
    let mut other = draft.clone();
    other.set("card[vga].kind", Value::Name("vga-text".into()));
    let sheet = fields(&other);
    assert!(!sheet
        .iter()
        .any(|f| f.path.starts_with("card[") && f.path.ends_with(".memory_mib")));
    assert!(!sheet
        .iter()
        .any(|f| f.path.starts_with("card[") && f.path.ends_with(".model")));

    // The 24-bit AT board names the model as the cause.
    let mut at = draft.clone();
    at.config.board = BoardModel::Ibm5170;
    at.config.cpu.chip = CpuChip::I80286;
    let result = validate_draft(&at);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == "vbe-address-space" && d.field_paths == ["card[vga].model"]),
        "{:?}",
        result.diagnostics
    );
}
