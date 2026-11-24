// SPDX-License-Identifier: AGPL-3.0-or-later
//! Preparation through the contract uses only declared facts and verified content.

use pc_config::{image::Digest, launch};
use std::{cell::Cell, collections::BTreeMap, rc::Rc};
use unemu_core_api::{
    test_support::{check_prepared, MemoryStorage},
    *,
};
use unemu_pc_core::PcCore;

#[path = "../../pc-harness/tests/support/fixtures.rs"]
mod fixtures;

const OPEN: &str = include_str!("../../pc-config/tests/fixtures/open-at.pc.toml");
const DOS: &str = include_str!("../../pc-config/tests/fixtures/open-at-dos.pc.toml");
fn doc(text: &str) -> ConfigDocument<'_> {
    ConfigDocument::new(launch::MEDIA_TYPE, text.as_bytes())
}
#[derive(Default)]
struct Source {
    blobs: BTreeMap<[u8; 32], Vec<u8>>,
    opens: Cell<usize>,
}
impl ContentSource for Source {
    fn open(&self, digest: &[u8; 32], size: u64) -> Result<Content, Error> {
        self.opens.set(self.opens.get() + 1);
        self.blobs
            .get(digest)
            .filter(|b| b.len() as u64 == size)
            .cloned()
            .map(Content::Bytes)
            .ok_or_else(|| Error::new(ErrorKind::Content, "missing receipt"))
    }
}
fn facts(dos: bool) -> Option<(Vec<BindingUse>, Source)> {
    let mut uses = Vec::new();
    let mut source = Source::default();
    for (id, item, role, path) in [
        (
            "bios",
            "system-bios",
            "system-bios",
            "unemu-at-bios/bios-unemu-at.bin",
        ),
        ("vga", "video-bios", "video-bios", "vgabios/vgabios-std.bin"),
        (
            "ide",
            "option-rom",
            "option-rom",
            "xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin",
        ),
    ] {
        let bytes = fixtures::fixture(path)?;
        let digest = Digest::of(&bytes).sha256();
        uses.push(BindingUse::new(
            id,
            "opaque:package",
            vec![
                BindingItem::new(item, BindingKind::FirmwareRom, bytes.len() as u64, digest)
                    .with_roles(vec![role.into()]),
            ],
        ));
        source.blobs.insert(digest, bytes);
    }
    if dos {
        let bytes = fixtures::fixture("freedos/freedos-1200.img")?;
        let digest = Digest::of(&bytes).sha256();
        uses.push(BindingUse::new(
            "dos",
            "opaque:package",
            vec![BindingItem::new(
                "disk-1",
                BindingKind::Medium,
                bytes.len() as u64,
                digest,
            )],
        ));
        source.blobs.insert(digest, bytes);
    }
    Some((uses, source))
}

fn compose_reviewed_generic_firmware(board: &str, cpu: &str) {
    let core: &dyn Core = &PcCore;
    let text = format!(
        "schema=1\nplatform='ibm-pc'\n[machine]\nschema=1\n\
         [machine.board]\nmodel='{board}'\n\
         [machine.cpu]\nmodel='{cpu}'\n\
         [machine.firmware.bios]\nuse='bios'\nrom='system-bios'\nrole='system-bios'\nbase=0xE0000\nsize=131072\n\
         [[machine.firmware.option_roms]]\nuse='vga'\nrom='video-bios'\nrole='video-bios'\nbase=0xC0000\nsize=32768\n\
         [[machine.card]]\nkind='vga'\n\
         [[machine.card]]\nkind='uart-16550'\nport=0x3F8\nirq=4\n"
    );
    let mut uses = Vec::new();
    let mut source = Source::default();
    for (id, role, entry_id, path) in [
        (
            "bios",
            "system-bios",
            "seabios-1.17.0-unemu1-isa",
            "seabios/bios-1.17.0-isa.bin",
        ),
        (
            "vga",
            "video-bios",
            "seavgabios-1.17.0-unemu1-stdvga",
            "seabios/build/vgabios-1.17.0-stdvga.bin",
        ),
    ] {
        let entry = assembly::BoardModel::Generic386Isa
            .firmware_catalogue()
            .iter()
            .find(|entry| entry.id == entry_id)
            .unwrap();
        let digest = format!("sha256:{}", entry.sha256)
            .parse::<Digest>()
            .unwrap()
            .sha256();
        let Some(bytes) = fixtures::fixture(path) else {
            return;
        };
        assert_eq!(bytes.len() as u64, entry.size);
        assert_eq!(Digest::of(&bytes).sha256(), digest);
        uses.push(BindingUse::new(
            id,
            "opaque:package",
            vec![
                BindingItem::new(role, BindingKind::FirmwareRom, entry.size, digest)
                    .with_roles(vec![role.into()]),
            ],
        ));
        source.blobs.insert(digest, bytes);
    }
    let slots = core.binding_slots("ibm-pc", doc(&text)).unwrap();
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].role.as_deref(), Some("system-bios"));
    assert_eq!(slots[1].role.as_deref(), Some("video-bios"));
    let prepared = core
        .compose("ibm-pc", doc(&text), BindingFacts::new(&uses, &source))
        .unwrap();
    assert_eq!(source.opens.get(), 2);
    assert_eq!(prepared.validated.content.len(), 2);
    check_prepared(core, "ibm-pc", &prepared).unwrap();
    assert!(core
        .binding_slots("ibm-pc", prepared.config.as_document())
        .unwrap()
        .is_empty());
    let launch = launch::decode(&prepared.config.bytes).unwrap();
    assert_eq!(
        launch.config.machine["board"]["model"].as_str(),
        Some(board)
    );
    assert_eq!(launch.config.machine["cpu"]["model"].as_str(), Some(cpu));
}

#[test]
fn generic_486_dx_composes_reviewed_seabios_and_seavgabios() {
    compose_reviewed_generic_firmware("generic-486-isa", "80486dx");
}

#[test]
fn generic_486_dx_cpuid_composes_reviewed_seabios_and_seavgabios() {
    compose_reviewed_generic_firmware("generic-486-isa", "80486dx-cpuid");
}

#[test]
fn generic_386_dx_composes_reviewed_seabios_and_seavgabios() {
    compose_reviewed_generic_firmware("generic-386-isa", "80386dx");
}

#[test]
fn supported_operations_check_arguments_and_advertise_bindings() {
    let core: &dyn Core = &PcCore;
    assert!(core.description().platforms[0].content_bindings);
    let source = Source::default();
    assert_eq!(
        core.binding_slots("ibm-pc", doc("bad TOML"))
            .unwrap_err()
            .kind,
        ErrorKind::Config
    );
    assert_eq!(
        core.compose("ibm-pc", doc("bad TOML"), BindingFacts::new(&[], &source))
            .unwrap_err()
            .kind,
        ErrorKind::Config
    );
    assert_eq!(
        core.binding_slots("absent", doc("bad TOML"))
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    assert_eq!(
        core.compose(
            "ibm-pc",
            ConfigDocument::new("absent", b"bad"),
            BindingFacts::new(&[], &source)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Unsupported
    );
    assert_eq!(source.opens.get(), 0);
}

#[test]
fn both_examples_prepare_deterministically_validate_and_create_with_only_their_set() {
    let core: &dyn Core = &PcCore;
    for (text, dos) in [(OPEN, false), (DOS, true)] {
        let Some((mut uses, source)) = facts(dos) else {
            return;
        };
        for package in &mut uses {
            package.items[0].roles.push("extra-role".into());
            package.items.push(BindingItem::new(
                "unused",
                BindingKind::Medium,
                512,
                [9; 32],
            ));
        }
        let prepared = core
            .compose("ibm-pc", doc(text), BindingFacts::new(&uses, &source))
            .unwrap();
        assert_eq!(source.opens.get(), prepared.validated.content.len());
        check_prepared(core, "ibm-pc", &prepared).unwrap();
        let slots = core
            .binding_slots("ibm-pc", prepared.config.as_document())
            .unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].item_id.as_deref(), Some("system"));
        assert_eq!(
            core.validate("ibm-pc", prepared.config.as_document())
                .unwrap(),
            prepared.validated
        );
        assert_eq!(prepared.validated.content.len(), if dos { 4 } else { 3 });
        let parsed = launch::decode(&prepared.config.bytes).unwrap();
        assert_eq!(
            parsed
                .config
                .media
                .iter()
                .find(|m| m.id == "system")
                .unwrap()
                .zero
                .as_ref()
                .unwrap()
                .size,
            528482304
        );
        assert!(!String::from_utf8_lossy(&prepared.config.bytes)
            .lines()
            .any(|line| line.starts_with("use =")));
        uses.reverse();
        for package in &mut uses {
            package.items.reverse();
            for item in &mut package.items {
                item.roles.reverse();
            }
        }
        assert_eq!(
            core.compose("ibm-pc", doc(text), BindingFacts::new(&uses, &source))
                .unwrap(),
            prepared
        );
        let allowed = Source {
            blobs: source
                .blobs
                .into_iter()
                .filter(|(digest, bytes)| {
                    prepared
                        .validated
                        .content
                        .iter()
                        .any(|r| r.sha256 == *digest && r.size == bytes.len() as u64)
                })
                .collect(),
            opens: Cell::new(0),
        };
        let request = CreateRequest::new(
            "ibm-pc",
            prepared.config.as_document(),
            &allowed,
            Rc::new(MemoryStorage::new()),
        );
        let machine = core.create(&request).unwrap();
        assert!(!machine.description().displays.is_empty());
        drop(machine);
    }
}

#[test]
fn all_document_and_fact_rejections_happen_before_content_opens() {
    let core: &dyn Core = &PcCore;
    let Some((uses, source)) = facts(true) else {
        return;
    };
    for text in [
        DOS.replace("size = 528482304", "size = 512"),
        DOS.replace("use = \"bios\"", "use = \"bios\"\nimage = \"host.bin\""),
        DOS.replace("disk = \"system\"", "disk = \"missing\""),
        DOS.replace("role = \"video-bios\"", "role = \"unknown\""),
        DOS.replace("base = 0xC8000", "base = 0xC8001"),
        DOS.replace("irq = 14", "irq = 99"),
        DOS.replace(
            "write = \"volatile\"",
            "write = \"journal\"\njournal = \"host.upfj\"",
        ),
    ] {
        source.opens.set(0);
        let error = core
            .compose("ibm-pc", doc(&text), BindingFacts::new(&uses, &source))
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Config, "{error}");
        assert_eq!(source.opens.get(), 0, "{error}");
    }
    let mut variants = vec![Vec::new(), uses[..3].to_vec()];
    let mut duplicate = uses.clone();
    duplicate.push(uses[0].clone());
    variants.push(duplicate);
    let mut missing_role = uses.clone();
    missing_role[0].items[0].roles.clear();
    variants.push(missing_role);
    let mut wrong_size = uses.clone();
    wrong_size[3].items[0].size = 512;
    variants.push(wrong_size);
    for uses in variants {
        source.opens.set(0);
        let error = core
            .compose("ibm-pc", doc(DOS), BindingFacts::new(&uses, &source))
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Config, "{error}");
        assert_eq!(source.opens.get(), 0, "{error}");
    }
}

#[test]
fn unreviewed_bound_firmware_is_unsupported_before_content_opens() {
    let Some((mut uses, source)) = facts(false) else {
        return;
    };
    uses[0].items[0].sha256 = [7; 32];
    let core: &dyn Core = &PcCore;
    let error = core
        .compose("ibm-pc", doc(OPEN), BindingFacts::new(&uses, &source))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported, "{error}");
    assert!(
        error.to_string().contains("not in the reviewed catalogue"),
        "{error}"
    );
    assert_eq!(source.opens.get(), 0);
}

#[test]
fn image_mode_cmos_binding_survives_composition_and_default_freezing() {
    let Some((uses, mut source)) = facts(false) else {
        return;
    };
    let bytes = vec![0x5a; 64];
    let head = Digest::of(&bytes);
    source.blobs.insert(head.sha256(), bytes);
    let mut launch = launch::decode(OPEN.as_bytes()).unwrap();
    launch.config.machine["board"]["rtc"]["nvram"] =
        toml::Table::from_iter([("image".into(), "assets/cmos/base".into())]).into();
    launch.config.nvram = Some(pc_config::image::Nvram {
        mode: pc_config::image::NvramMode::Image,
        source: "assets/cmos/base".into(),
        head,
    });
    launch.content.push(launch::ContentRow {
        digest: head,
        sha256: head.sha256(),
        size: 64,
    });
    let input = launch::encode(&launch).unwrap();
    let core: &dyn Core = &PcCore;
    let prepared = core
        .compose(
            "ibm-pc",
            ConfigDocument::new(launch::MEDIA_TYPE, &input),
            BindingFacts::new(&uses, &source),
        )
        .unwrap();
    check_prepared(core, "ibm-pc", &prepared).unwrap();
    let decoded = launch::decode_portable(&prepared.config.bytes).unwrap();
    let nvram = decoded.config.nvram.as_ref().unwrap();
    assert!(matches!(nvram.mode, pc_config::image::NvramMode::Image));
    assert_eq!(nvram.source, "assets/cmos/base");
    assert_eq!(nvram.head, head);
    assert_eq!(
        decoded.config.machine["board"]["rtc"]["nvram"],
        launch.config.machine["board"]["rtc"]["nvram"]
    );
    assert!(prepared
        .validated
        .content
        .iter()
        .any(|row| row.sha256 == head.sha256() && row.size == 64));
    assert!(decoded.records.is_empty());
    assert_eq!(
        core.compose(
            "ibm-pc",
            prepared.config.as_document(),
            BindingFacts::new(&[], &source),
        )
        .unwrap(),
        prepared
    );
}

#[test]
fn mixed_discovery_merges_prepared_zero_disks_with_new_rom_bindings() {
    let Some((uses, source)) = facts(false) else {
        return;
    };
    let core: &dyn Core = &PcCore;
    let prepared = core
        .compose("ibm-pc", doc(OPEN), BindingFacts::new(&uses, &source))
        .unwrap();
    let mut launch = launch::decode(&prepared.config.bytes).unwrap();
    let bios = launch.config.firmware.remove(0);
    launch.content.retain(|row| row.digest != bios.blob);
    launch.config.machine["firmware"]["bios"] =
        launch::decode(OPEN.as_bytes()).unwrap().config.machine["firmware"]["bios"].clone();
    let input = launch::encode(&launch).unwrap();
    source.opens.set(0);
    let slots = core
        .binding_slots("ibm-pc", ConfigDocument::new(launch::MEDIA_TYPE, &input))
        .unwrap();
    assert_eq!(slots.len(), 2, "{slots:?}");
    assert_eq!(
        slots[0],
        BindingSlot::zero_disk("card[1].drives[0]", "system", 528482304)
    );
    assert_eq!(slots[1].id, "firmware.bios");
    assert_eq!(slots[1].kind, BindingKind::FirmwareRom);
    assert_eq!(slots[1].use_id.as_deref(), Some("bios"));
    assert_eq!(source.opens.get(), 0);
}

#[test]
#[ignore = "slow: open AT POST through a composed native-zero launch"]
fn composed_open_at_shows_its_post_banner_on_the_existing_observation_path() {
    let Some((uses, source)) = facts(false) else {
        return;
    };
    let core: &dyn Core = &PcCore;
    let prepared = core
        .compose("ibm-pc", doc(OPEN), BindingFacts::new(&uses, &source))
        .unwrap();
    let request = CreateRequest::new(
        "ibm-pc",
        prepared.config.as_document(),
        &source,
        Rc::new(MemoryStorage::new()),
    );
    let mut machine = PcCore.create_machine(&request).unwrap();
    machine.run(RunLimit::steps(6_000_000)).unwrap();
    let capture = machine
        .assembled()
        .capture_text(assembly::DeviceId::Vga)
        .unwrap();
    let video::Frame::Text(grid) = capture.frame else {
        panic!("expected VGA text")
    };
    let text = grid.rows_as_strings().join("\n");
    assert!(
        text.contains("8088 BIOS") || text.contains("AT BIOS"),
        "{text}"
    );
}

#[test]
fn receipt_size_missing_receipt_and_read_io_keep_contract_error_kinds() {
    struct Refusal<'a> {
        source: &'a Source,
        kind: u8,
    }
    struct Unreadable(u64);
    impl ContentReader for Unreadable {
        fn size(&self) -> u64 {
            self.0
        }
        fn read_at(&self, _: u64, _: &mut [u8]) -> Result<(), Error> {
            Err(Error::new(ErrorKind::Storage, "injected read I/O"))
        }
    }
    impl ContentSource for Refusal<'_> {
        fn open(&self, digest: &[u8; 32], size: u64) -> Result<Content, Error> {
            match self.kind {
                0 => Err(Error::new(ErrorKind::Content, "missing receipt")),
                1 => Ok(Content::Bytes(vec![0])),
                2 => Ok(Content::Reader(Box::new(Unreadable(size)))),
                3 => Ok(Content::Reader(Box::new(Unreadable(size - 1)))),
                _ => self.source.open(digest, size),
            }
        }
    }
    let Some((uses, source)) = facts(false) else {
        return;
    };
    let core: &dyn Core = &PcCore;
    for (kind, expected) in [
        (0, ErrorKind::Content),
        (1, ErrorKind::Content),
        (2, ErrorKind::Storage),
        (3, ErrorKind::Content),
    ] {
        let refusal = Refusal {
            source: &source,
            kind,
        };
        let error = core
            .compose("ibm-pc", doc(OPEN), BindingFacts::new(&uses, &refusal))
            .unwrap_err();
        assert_eq!(error.kind, expected, "{error}");
    }
}

#[test]
fn correctly_receipted_bad_option_rom_is_unsupported() {
    let Some((uses, mut source)) = facts(false) else {
        return;
    };
    let core: &dyn Core = &PcCore;
    let prepared = core
        .compose("ibm-pc", doc(OPEN), BindingFacts::new(&uses, &source))
        .unwrap();
    let mut launch = launch::decode(&prepared.config.bytes).unwrap();
    let firmware = &mut launch.config.firmware[2];
    let old = firmware.blob;
    let bytes = vec![0; 8192];
    let digest = Digest::of(&bytes);
    firmware.blob = digest;
    launch.config.machine["firmware"]["option_roms"]
        .as_array_mut()
        .unwrap()[1]["sha256"] = digest.hex().into();
    let row = launch.content.iter_mut().find(|r| r.digest == old).unwrap();
    row.digest = digest;
    row.sha256 = digest.sha256();
    row.size = bytes.len() as u64;
    source.blobs.insert(digest.sha256(), bytes);
    let encoded = launch::encode(&launch).unwrap();
    let document = ConfigDocument::new(launch::MEDIA_TYPE, &encoded);
    core.validate("ibm-pc", document).unwrap();
    let error = core
        .compose("ibm-pc", document, BindingFacts::new(&[], &source))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported, "{error}");
}

#[test]
fn portable_storage_round_trip_preserves_heads_and_rejects_session_kept_records_before_open() {
    let Some((uses, source)) = facts(true) else {
        return;
    };
    let core: &dyn Core = &PcCore;
    let text = DOS.replace("write = \"volatile\"", "write = \"journal\"");
    let prepared = core
        .compose("ibm-pc", doc(&text), BindingFacts::new(&uses, &source))
        .unwrap();
    let mut launch = launch::decode_portable(&prepared.config.bytes).unwrap();
    assert_eq!(launch.records[0].name, "system.upfj");
    assert_eq!(
        launch::encode_portable(&launch).unwrap(),
        prepared.config.bytes
    );
    let disk = launch
        .config
        .media
        .iter_mut()
        .find(|m| m.head.is_some())
        .unwrap();
    let old = disk.head.unwrap();
    let layered = Digest::of(b"preserved layered head");
    disk.head = Some(layered);
    launch
        .content
        .iter_mut()
        .find(|r| r.digest == old)
        .unwrap()
        .digest = layered;
    let bytes = launch::encode_portable(&launch).unwrap();
    let recomposed = core
        .compose(
            "ibm-pc",
            ConfigDocument::new(launch::MEDIA_TYPE, &bytes),
            BindingFacts::new(&[], &source),
        )
        .unwrap();
    let decoded = launch::decode_portable(&recomposed.config.bytes).unwrap();
    assert!(decoded.config.media.iter().any(|m| m.head == Some(layered)));
    assert_eq!(prepared.validated, recomposed.validated);
    launch.records[0].name = "kept-test.upfj".into();
    let transient = launch::encode(&launch).unwrap();
    assert!(launch::decode_portable(&transient).is_err());
    launch::check(
        &launch::decode(&transient).unwrap(),
        assembly::AssemblyOptions::default(),
    )
    .unwrap();
    source.opens.set(0);
    assert_eq!(
        core.compose(
            "ibm-pc",
            ConfigDocument::new(launch::MEDIA_TYPE, &transient),
            BindingFacts::new(&[], &source)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Config
    );
    assert_eq!(source.opens.get(), 0);
    let storage = Rc::new(MemoryStorage::new());
    let request = CreateRequest::new(
        "ibm-pc",
        prepared.config.as_document(),
        &source,
        storage.clone(),
    );
    core.create(&request).unwrap().finish().unwrap();
    assert!(storage.contents("system.upfj").unwrap().len() >= 128);
}

#[test]
fn native_zero_media_exports_through_current_readers_without_base_content() {
    let Some((uses, source)) = facts(false) else {
        return;
    };
    let core: &dyn Core = &PcCore;
    let text = OPEN.replace("size = 528482304", "size = 512").replace(
        "cylinders = 1024, heads = 16, sectors = 63",
        "cylinders = 1, heads = 1, sectors = 1",
    );
    let prepared = core
        .compose("ibm-pc", doc(&text), BindingFacts::new(&uses, &source))
        .unwrap();
    let bytes = Content::Bytes(vec![0x5a; 512]);
    let media = [ExportMedium::new(SlotId(2), 512, &bytes)];
    for format in ["raw", "vhd-fixed", "machine"] {
        let mut sink = test_support::MemoryExportSink::new();
        let report = core
            .export(
                "ibm-pc",
                prepared.config.as_document(),
                ExportRequest::new(format, &media),
                &mut sink,
            )
            .unwrap();
        assert!(!report.outputs.is_empty());
        assert_eq!(
            &sink.outputs()[if format == "vhd-fixed" {
                "slot-2.vhd"
            } else {
                "slot-2.img"
            }][..512],
            &[0x5a; 512]
        );
    }
}

#[test]
fn native_zero_codec_rejects_conflicting_sources_sizes_and_geometry_without_opening() {
    let Some((uses, source)) = facts(false) else {
        return;
    };
    let core: &dyn Core = &PcCore;
    let prepared = core
        .compose("ibm-pc", doc(OPEN), BindingFacts::new(&uses, &source))
        .unwrap();
    let original = std::str::from_utf8(&prepared.config.bytes).unwrap();
    let table: toml::Table = original.parse().unwrap();
    for mutation in 0..5 {
        let mut table = table.clone();
        let medium = table["media"].as_array_mut().unwrap()[0]
            .as_table_mut()
            .unwrap();
        match mutation {
            0 => {
                medium.insert("head".into(), Digest::of(b"extra head").to_string().into());
            }
            1 => {
                medium.remove("zero");
            }
            2 => {
                medium["zero"]["size"] = 0.into();
            }
            3 => {
                medium["zero"]["size"] = 512.into();
            }
            _ => {
                medium["geometry"]["heads"] = i64::MAX.into();
            }
        }
        let text = toml::to_string(&table).unwrap();
        source.opens.set(0);
        assert_eq!(
            core.validate("ibm-pc", doc(&text)).unwrap_err().kind,
            ErrorKind::Config
        );
        assert_eq!(
            core.compose("ibm-pc", doc(&text), BindingFacts::new(&[], &source))
                .unwrap_err()
                .kind,
            ErrorKind::Config
        );
        assert_eq!(source.opens.get(), 0);
    }
}

#[test]
fn portable_records_cannot_claim_the_session_kept_namespace_even_with_a_matching_medium_id() {
    let Some((uses, source)) = facts(false) else {
        return;
    };
    let text = OPEN
        .replace("\"system\"", "\"kept-system\"")
        .replace("write = \"volatile\"", "write = \"journal\"");
    let core: &dyn Core = &PcCore;
    let error = core
        .compose("ibm-pc", doc(&text), BindingFacts::new(&uses, &source))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Config);
    assert_eq!(source.opens.get(), 0);
}
