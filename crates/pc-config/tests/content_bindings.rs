// SPDX-License-Identifier: AGPL-3.0-or-later
//! Content-independent planning and package-local fact validation.

use pc_config::{
    bindings::{self, BindingPlan},
    ImageReader, MachineFile, Origins,
};
use std::{cell::Cell, io, path::Path};
use unemu_core_api::{
    BindingItem, BindingKind as K, BindingUse, Content, ContentSource, Error, ErrorKind,
};

const BIOS: &str =
    "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nuse='bios'\nrom='reset'\nbase=0xF8000\n";
fn parsed(text: &str) -> (MachineFile, Origins) {
    let doc = pc_config::parse_toml("config.toml", text).unwrap();
    let mut origins = Origins::from_toml("config.toml", text).unwrap();
    pc_config::source::record_toml(Path::new("config.toml"), text, &mut origins).unwrap();
    (doc, origins)
}
fn plan(text: &str) -> BindingPlan {
    let (d, o) = parsed(text);
    bindings::plan(&d, &o).unwrap()
}
fn bios() -> BindingUse {
    BindingUse::new(
        "bios",
        "opaque:manifest",
        vec![BindingItem::new("reset", K::FirmwareRom, 32768, [1; 32])
            .with_roles(vec!["system-bios".into()])],
    )
}
#[derive(Default)]
struct Counting {
    opens: Cell<usize>,
    paths: Cell<usize>,
}
impl ContentSource for Counting {
    fn open(&self, _: &[u8; 32], _: u64) -> Result<Content, Error> {
        self.opens.set(self.opens.get() + 1);
        Err(Error::new(ErrorKind::Content, "test content unavailable"))
    }
}
impl ImageReader for Counting {
    fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
        self.paths.set(self.paths.get() + 1);
        Err(io::Error::other("unexpected path open"))
    }
}
fn rejected(text: &str, uses: &[BindingUse], expected: &str) -> Error {
    let (doc, origins) = parsed(text);
    let source = Counting::default();
    let mut paths = Counting::default();
    let error = bindings::lower_with_bindings(
        &doc,
        &origins,
        uses,
        &source,
        &mut paths,
        assembly::AssemblyOptions::default(),
    )
    .unwrap_err();
    assert_eq!(source.opens.get(), 0, "{error:?}");
    assert_eq!(paths.paths.get(), 0, "{error:?}");
    assert_eq!(error.kind, ErrorKind::Config, "{error:?}");
    assert!(
        error.to_string().contains(expected),
        "expected {expected:?}: {error:?}"
    );
    error
}

#[test]
fn complete_examples_discover_exact_occurrences_without_a_reader() {
    for (text, dos) in [
        (include_str!("fixtures/open-at.pc.toml"), false),
        (include_str!("fixtures/open-at-dos.pc.toml"), true),
    ] {
        let plan = bindings::plan_envelope(text).unwrap();
        let mut expected = vec![(
            "card[1].drives[0]",
            K::ZeroDisk,
            None,
            Some("system"),
            Some(528482304),
            None,
        )];
        if dos {
            expected.push((
                "card[2].drives[0]",
                K::Medium,
                Some("dos"),
                Some("disk-1"),
                None,
                None,
            ));
        }
        expected.extend([
            (
                "firmware.bios",
                K::FirmwareRom,
                Some("bios"),
                Some("system-bios"),
                None,
                if dos { None } else { Some("system-bios") },
            ),
            (
                "firmware.option_roms[0]",
                K::FirmwareRom,
                Some("vga"),
                Some("video-bios"),
                None,
                Some("video-bios"),
            ),
            (
                "firmware.option_roms[1]",
                K::FirmwareRom,
                Some("ide"),
                Some("option-rom"),
                None,
                Some("option-rom"),
            ),
        ]);
        assert_eq!(
            plan.slots()
                .iter()
                .map(|s| (
                    s.id.as_str(),
                    s.kind,
                    s.use_id.as_deref(),
                    s.item_id.as_deref(),
                    s.size,
                    s.role.as_deref()
                ))
                .collect::<Vec<_>>(),
            expected
        );
        assert!(plan.slots().iter().all(|s| s.sha256.is_none()));
    }
}
#[test]
fn slots_use_lexicographic_occurrences_and_repeated_items_keep_distinct_keys() {
    let text = format!(
        "{BIOS}{}",
        (0..12)
            .map(|i| format!(
                "[[firmware.option_roms]]\nuse='ext'\nrom='same'\nbase={}\n",
                0xC8000 + i * 2048
            ))
            .collect::<String>()
    );
    let plan = plan(&text);
    let ids = plan
        .slots()
        .iter()
        .map(|s| s.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        ids.iter()
            .position(|id| *id == "firmware.option_roms[10]")
            .unwrap()
            < ids
                .iter()
                .position(|id| *id == "firmware.option_roms[2]")
                .unwrap()
    );
    let ext = BindingUse::new(
        "ext",
        "manifest",
        vec![BindingItem::new("same", K::FirmwareRom, 2048, [2; 32])
            .with_roles(vec!["option-rom".into()])],
    );
    let resolved = plan.resolve(&[bios(), ext]).unwrap();
    assert_eq!(resolved.items[3].slot_id, "firmware.option_roms[2]");
    assert_ne!(resolved.items[1].key, resolved.items[2].key);
}
#[test]
fn every_selector_excludes_other_sources_and_incomplete_pairs() {
    for (suffix, expected) in [
        ("image='bios.bin'\n", "mutually exclusive"),
        ("parts=[]\n", "mutually exclusive"),
        ("role='wildcard'\n", "unknown requested role"),
        ("role='video-bios'\n", "role conflicts"),
        ("size=0\n", "positive"),
        ("size=65536\n", "BIOS base versus size"),
        ("offset=1\n", "offset is only"),
    ] {
        rejected(&format!("{BIOS}{suffix}"), &[bios()], expected);
    }
    for selector in [
        "use='bios'",
        "rom='reset'",
        "use=''\nrom='reset'",
        "use='bios'\nrom='' ",
    ] {
        rejected(
            &format!("schema=1\n[firmware.bios]\n{selector}\n"),
            &[],
            "nonempty use and rom",
        );
    }
    for selector in [
        "{}",
        "{use='dos'}",
        "{medium='disk'}",
        "{use='',medium='disk'}",
    ] {
        rejected(&format!("{BIOS}[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\ninserted={selector}\n"),&[bios()],"binding requires");
    }
    let drive="[[card]]\nkind='ide'\n[[card.drives]]\nchannel='master'\ngeometry={cylinders=1,heads=1,sectors=1}\n";
    for sources in [
        "image='disk'\ndisk='zero'",
        "image='disk'\ninserted={use='os',medium='disk'}",
        "disk='zero'\ninserted={use='os',medium='disk'}",
        "disk='zero'\nmedia=[]",
        "inserted={use='os',medium='disk'}\nmedia=[]",
    ] {
        rejected(
            &format!("{BIOS}{drive}{sources}\n"),
            &[bios()],
            "mutually exclusive",
        );
    }
    rejected(&format!("{BIOS}[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\nmedia=[{{image='disk',use='dos',medium='disk'}}]\n"),&[bios()],"mutually exclusive");
}
#[test]
fn every_fact_rejection_precedes_content_opens() {
    rejected(BIOS, &[], "unbound use");
    let mut b = bios();
    b.items.clear();
    rejected(BIOS, &[b], "selected item is missing");
    rejected(
        BIOS,
        &[bios(), BindingUse::new("extra", "manifest", vec![])],
        "unused supplied use",
    );
    rejected(BIOS, &[bios(), bios()], "duplicate use");
    let mut b = bios();
    b.id = "".into();
    rejected(BIOS, &[b], "empty name");
    let mut b = bios();
    b.manifest_digest = "".into();
    rejected(BIOS, &[b], "manifest digest");
    let mut b = bios();
    b.items[0].id = "".into();
    rejected(BIOS, &[b], "empty item");
    let mut b = bios();
    b.items[0].roles.push("".into());
    rejected(BIOS, &[b], "empty item or role");
    let mut b = bios();
    b.items.push(b.items[0].clone());
    rejected(BIOS, &[b], "ambiguous duplicate item");
    let mut b = bios();
    let mut extra = b.items[0].clone();
    extra.id = "unselected".into();
    b.items.extend([extra.clone(), extra]);
    rejected(BIOS, &[b], "ambiguous duplicate item");
    let mut b = bios();
    b.items[0].kind = K::Medium;
    rejected(BIOS, &[b], "wrong item kind");
    let mut b = bios();
    b.items[0].kind = K::ZeroDisk;
    rejected(BIOS, &[b], "invalid item kind");
    let mut b = bios();
    b.items[0].size = 0;
    rejected(BIOS, &[b], "invalid item kind or size");
    let mut b = bios();
    b.items[0].roles.clear();
    rejected(BIOS, &[b], "required role");
    let mut b = bios();
    b.items[0].size = 65536;
    rejected(BIOS, &[b], "BIOS base versus size");
    rejected(
        &format!("{BIOS}size=16384\n"),
        &[bios()],
        "BIOS base versus size",
    );
    rejected(
        &format!("{}size=16384\n", BIOS.replace("base=0xF8000\n", "")),
        &[bios()],
        "size conflicts",
    );
    rejected(
        &format!("{BIOS}sha256='{}'\n", "00".repeat(32)),
        &[bios()],
        "sha256 conflicts",
    );
    let mut b = bios();
    b.items
        .push(BindingItem::new("other", K::Medium, 512, [1; 32]));
    rejected(BIOS, &[b], "conflicting flat sizes");
}
#[test]
fn position_roles_are_required_even_without_an_assertion() {
    for (base, role) in [(0xC0000, "video-bios"), (0xC8000, "option-rom")] {
        let text = format!("{BIOS}[[firmware.option_roms]]\nuse='ext'\nrom='addon'\nbase={base}\n");
        let extra = BindingUse::new(
            "ext",
            "manifest",
            vec![BindingItem::new("addon", K::FirmwareRom, 2048, [2; 32])],
        );
        let error = rejected(&text, &[bios(), extra], role);
        assert_eq!(error.diagnostics[0].path, "firmware.option_roms[0].rom");
    }
}
#[test]
fn package_and_item_roles_never_leak_across_a_shared_digest() {
    let text = format!("{BIOS}[[firmware.option_roms]]\nuse='other'\nrom='reset'\nbase=0xC8000\n");
    let mut wrong = bios();
    wrong.items[0].roles = vec!["option-rom".into()];
    let other = BindingUse::new(
        "other",
        "different-manifest",
        vec![BindingItem::new("reset", K::FirmwareRom, 32768, [1; 32])
            .with_roles(vec!["system-bios".into(), "option-rom".into()])],
    );
    rejected(&text, &[wrong.clone(), other.clone()], "system-bios");
    wrong.items.push(
        BindingItem::new("unselected", K::FirmwareRom, 32768, [1; 32])
            .with_roles(vec!["system-bios".into()]),
    );
    rejected(&text, &[wrong.clone(), other.clone()], "system-bios");
    wrong.items[0].roles.push("system-bios".into());
    plan(&text).resolve(&[wrong, other]).unwrap();
}
#[test]
fn extra_items_and_fact_permutations_preserve_resolved_rows() {
    let empty = plan(&BIOS.replacen("schema=1\n", "schema=1\ndisks=[]\n", 1))
        .resolve(&[bios()])
        .unwrap();
    assert!(empty.document.disks.is_none());
    assert!(empty.zero_slots.is_empty());

    let mut b = bios();
    b.items
        .push(BindingItem::new("extra", K::Medium, 512, [3; 32]));
    let first = plan(BIOS).resolve(&[b.clone()]).unwrap();
    b.items.reverse();
    b.items.iter_mut().for_each(|item| item.roles.reverse());
    let second = plan(BIOS).resolve(&[b]).unwrap();
    assert_eq!(first.items, second.items);
    assert_eq!(first.document, second.document);
    assert_eq!(
        first
            .origins
            .image_path("firmware.bios.image", "assets/firmware/bios.bin"),
        Path::new("assets/firmware/bios.bin")
    );
    assert_eq!(first.origins.get("firmware.bios.image").line, Some(6));
}
#[test]
fn zero_declarations_require_unique_safe_ids_one_attachment_and_exact_geometry() {
    let attachment="[[card]]\nkind='ide'\n[[card.drives]]\nchannel='master'\ndisk='system'\ngeometry={cylinders=1,heads=1,sectors=1}\n";
    let with = |decl: &str, drives: &str| {
        format!(
            "schema=1\ndisks=[{decl}]\n{}{}",
            BIOS.strip_prefix("schema=1\n").unwrap(),
            drives
        )
    };
    rejected(
        &with("{id='system',size=512},{id='system',size=512}", attachment),
        &[bios()],
        "duplicate zero",
    );
    rejected(
        &with("{id='',size=512}", attachment),
        &[bios()],
        "invalid zero",
    );
    rejected(
        &with("{id='../bad',size=512}", attachment),
        &[bios()],
        "invalid zero",
    );
    rejected(
        &with("{id='system',size=0}", attachment),
        &[bios()],
        "invalid zero",
    );
    rejected(
        &with("{id='system',size=1024}", attachment),
        &[bios()],
        "zero declaration",
    );
    rejected(
        &with("{id='system',size=512}", ""),
        &[bios()],
        "unused zero",
    );
    rejected(&with("", attachment), &[bios()], "undeclared zero");
    rejected(&with("{id='system',size=512}",&format!("{attachment}[[card.drives]]\nchannel='slave'\ndisk='system'\ngeometry={{cylinders=1,heads=1,sectors=1}}\n")),&[bios()],"more than once");
    rejected(
        &with(
            "{id='system',size=512}",
            &attachment.replace(
                "geometry={cylinders=1,heads=1,sectors=1}",
                "geometry={cylinders=4294967295,heads=4294967295,sectors=4294967295}",
            ),
        ),
        &[bios()],
        "overflows",
    );
    let resolved = plan(&with("{id='system',size=512}", attachment))
        .resolve(&[bios()])
        .unwrap();
    assert_eq!(resolved.zero_slots.len(), 1);
    assert_eq!(resolved.items.len(), 1);
    assert!(
        resolved.document.card.unwrap()[0].drives.as_ref().unwrap()[0]
            .disk
            .is_some()
    );
}
#[test]
fn medium_geometry_and_writable_sharing_are_checked_from_facts() {
    let text=format!("{BIOS}[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\ninserted={{use='dos',medium='disk'}}\n");
    let medium = BindingUse::new(
        "dos",
        "manifest",
        vec![BindingItem::new("disk", K::Medium, 512, [2; 32])],
    );
    rejected(&text, &[bios(), medium], "drive 0 image");
    let text=format!("{BIOS}[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\nmedia=[{{use='dos',medium='disk'}},{{use='dos',medium='disk'}}]\n");
    let medium = BindingUse::new(
        "dos",
        "manifest",
        vec![BindingItem::new("disk", K::Medium, 1228800, [2; 32])],
    );
    rejected(&text, &[bios(), medium.clone()], "writable medium");
    let readonly = text.replace("medium='disk'", "medium='disk',write='readonly'");
    let resolved = plan(&readonly).resolve(&[bios(), medium]).unwrap();
    assert_eq!(resolved.items.len(), 3);
}
#[test]
fn envelope_diagnostics_retain_original_lines_and_prefixed_paths() {
    let text = include_str!("fixtures/open-at-dos.pc.toml");
    let error = bindings::plan_envelope(text)
        .unwrap()
        .resolve(&[])
        .unwrap_err();
    assert_eq!(error.diagnostics[0].path, "machine.firmware.bios.rom");
    let line = text
        .lines()
        .position(|l| l == "rom = \"system-bios\"")
        .unwrap()
        + 1;
    assert!(
        error.to_string().contains(&format!("config.toml:{line}")),
        "{error}"
    );
    let fact = plan(BIOS)
        .resolve(&[BindingUse::new("unused", "manifest", vec![])])
        .unwrap_err();
    assert!(fact.diagnostics[0].path.is_empty());
}
#[test]
fn ordinary_lowering_refuses_unresolved_templates_before_opening_a_path() {
    let (document, origins) = parsed(BIOS);
    let mut reader = Counting::default();
    let error = pc_config::lower(&document, &origins, &mut reader).unwrap_err();
    assert!(error.message.contains("composition is required"));
    assert_eq!(reader.paths.get(), 0);
}

#[path = "../../pc-harness/tests/support/fixtures.rs"]
mod fixtures;

#[test]
fn mixed_path_and_bound_sources_lower_through_the_existing_reader() {
    use sha2::{Digest, Sha256};
    let Some(bios_bytes) = fixtures::fixture("kiselev-bios/bios-xi8088.bin") else {
        return;
    };
    let disk = vec![0xF6; 368640];
    let disk_digest: [u8; 32] = Sha256::digest(&disk).into();
    struct Paths {
        bios: Vec<u8>,
        seen: Vec<String>,
    }
    impl ImageReader for Paths {
        fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
            self.seen.push(path.to_string_lossy().into_owned());
            if path == Path::new("layers/bios.bin") {
                Ok(self.bios.clone())
            } else {
                Err(io::Error::other("unexpected key"))
            }
        }
        fn image_length(&self, path: &Path) -> io::Result<u64> {
            if path == Path::new("layers/bios.bin") {
                Ok(self.bios.len() as u64)
            } else {
                Err(io::Error::other("unknown path"))
            }
        }
    }
    struct Source {
        bytes: Vec<u8>,
        digest: [u8; 32],
        opens: Cell<usize>,
    }
    impl ContentSource for Source {
        fn open(&self, digest: &[u8; 32], size: u64) -> Result<Content, Error> {
            assert_eq!(digest, &self.digest);
            assert_eq!(size, self.bytes.len() as u64);
            self.opens.set(self.opens.get() + 1);
            Ok(Content::Bytes(self.bytes.clone()))
        }
    }
    let text="schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='bios.bin'\n[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='1.2M'\ninserted={use='dos',medium='disk'}\nwrite='readonly'\n";
    let document = pc_config::parse_toml("layers/config.toml", text).unwrap();
    let origins = Origins::from_toml("layers/config.toml", text).unwrap();
    let uses = [BindingUse::new(
        "dos",
        "manifest",
        vec![BindingItem::new(
            "disk",
            K::Medium,
            disk.len() as u64,
            disk_digest,
        )],
    )];
    let source = Source {
        bytes: disk.clone(),
        digest: disk_digest,
        opens: Cell::new(0),
    };
    let mut reader = Paths {
        bios: bios_bytes,
        seen: Vec::new(),
    };
    let machine = bindings::lower_with_bindings(
        &document,
        &origins,
        &uses,
        &source,
        &mut reader,
        assembly::AssemblyOptions::default(),
    )
    .unwrap();
    assert_eq!(reader.seen, ["layers/bios.bin"]);
    assert_eq!(source.opens.get(), 1);
    assert!(
        matches!(&machine.drives[0].image,assembly::ImageSource::Inline(bytes) if bytes==&disk)
    );
    assert_eq!(machine.drives[0].write, assembly::WriteMode::ReadOnly);
}

#[test]
fn resolved_config_failures_are_rejected_before_any_open() {
    for extra in ["[memory]\nconventional_kib=641\n", "[cpu]\nmodel='80386dx'\n", "[[card]]\nkind='ide'\n[[card.drives]]\nchannel='master'\ninserted={use='disk',medium='base'}\ngeometry={cylinders=1,heads=17,sectors=1}\n"] {
        let (doc,origins)=parsed(&format!("{BIOS}{extra}"));let source=Counting::default();let mut paths=Counting::default();
        assert!(bindings::lower_with_bindings(&doc,&origins,&[bios()],&source,&mut paths,assembly::AssemblyOptions::default()).is_err());
        assert_eq!(source.opens.get(),0);assert_eq!(paths.paths.get(),0);
    }
}

#[test]
fn reviewed_video_kind_requires_its_package_role_outside_the_video_window() {
    let catalogue = assembly::BoardModel::Ibm5170.firmware_catalogue();
    let entry = catalogue
        .iter()
        .find(|e| e.kind == assembly::FirmwareKind::VideoBios && e.rom_offset == 0)
        .unwrap();
    let digest = format!("sha256:{}", entry.sha256)
        .parse::<pc_config::image::Digest>()
        .unwrap()
        .sha256();
    let text =
        format!("{BIOS}[[firmware.option_roms]]\nuse='video'\nrom='adapter'\nbase=0xD0000\n");
    let item = BindingItem::new("adapter", K::FirmwareRom, entry.size, digest)
        .with_roles(vec!["option-rom".into()]);
    rejected(
        &text,
        &[
            bios(),
            BindingUse::new("video", "manifest", vec![item.clone()]),
        ],
        "video-bios",
    );
    let mut item = item;
    item.roles.push("video-bios".into());
    plan(&text)
        .resolve(&[
            bios(),
            BindingUse::new("video", "manifest", vec![item.clone()]),
        ])
        .unwrap();
    let explicit = format!("{text}role='video-bios'\n");
    plan(&explicit)
        .resolve(&[bios(), BindingUse::new("video", "manifest", vec![item])])
        .unwrap();
}

#[test]
fn stored_and_launch_hardware_accept_templates_but_resolved_checks_require_composition() {
    use std::collections::BTreeMap;
    let text = include_str!("fixtures/open-at.pc.toml");
    let config = pc_config::image::Config::decode(toml::from_str(text).unwrap()).unwrap();
    let source = Counting::default();
    let error = pc_config::image::lower(
        config,
        &BTreeMap::new(),
        &source,
        true,
        assembly::AssemblyOptions::default(),
    )
    .err()
    .unwrap();
    assert!(
        error.to_string().contains("composition is required"),
        "{error}"
    );
    let launch = pc_config::launch::decode(text.as_bytes()).unwrap();
    let error =
        pc_config::launch::check(&launch, assembly::AssemblyOptions::default()).unwrap_err();
    assert!(
        error.to_string().contains("composition is required"),
        "{error}"
    );
    assert_eq!(source.opens.get(), 0);
}

#[test]
fn catalogue_option_role_cannot_replace_a_video_position_requirement() {
    let entry = assembly::BoardModel::Ibm5170
        .firmware_catalogue()
        .iter()
        .find(|e| e.kind == assembly::FirmwareKind::OptionRom && e.rom_offset == 0)
        .unwrap();
    let digest = format!("sha256:{}", entry.sha256)
        .parse::<pc_config::image::Digest>()
        .unwrap()
        .sha256();
    let text =
        format!("{BIOS}[[firmware.option_roms]]\nuse='addon'\nrom='adapter'\nbase=0xC0000\n");
    rejected(
        &text,
        &[
            bios(),
            BindingUse::new(
                "addon",
                "manifest",
                vec![
                    BindingItem::new("adapter", K::FirmwareRom, entry.size, digest)
                        .with_roles(vec!["option-rom".into()]),
                ],
            ),
        ],
        "video position",
    );
}

#[test]
fn bound_bios_size_is_checked_before_content_open() {
    let text = BIOS.replace("base=0xF8000", "base=0xFFFFF");
    let mut package = bios();
    package.items[0].size = 1;
    for declaration in ["", "size=1\n"] {
        rejected(
            &format!("{text}{declaration}"),
            &[package.clone()],
            "multiple of 8 KiB",
        );
    }
}

#[test]
fn bound_video_alignment_is_checked_before_content_open() {
    let package = BindingUse::new(
        "video",
        "manifest",
        vec![BindingItem::new("adapter", K::FirmwareRom, 2048, [2; 32])
            .with_roles(vec!["video-bios".into()])],
    );
    for declaration in ["", "size=2048\n"] {
        let text = format!("{BIOS}[[firmware.option_roms]]\nuse='video'\nrom='adapter'\nbase=0xC0001\n{declaration}");
        rejected(&text, &[bios(), package.clone()], "2 KiB aligned");
    }
}

#[test]
fn ordinary_lowering_rejects_path_rom_assertions_before_reading() {
    for assertion in [
        "role='wildcard'",
        "role='system-bios'",
        "size=0",
        "size=32768",
    ] {
        let text = format!("schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='bios.bin'\nbase=0xF8000\n{assertion}\n");
        let (document, origins) = parsed(&text);
        let mut reader = Counting::default();
        let error = pc_config::lower(&document, &origins, &mut reader).unwrap_err();
        assert!(
            error
                .message
                .contains("assertions require a package binding"),
            "{assertion}: {error}"
        );
        assert_eq!(reader.paths.get(), 0);
    }
}

#[test]
fn binding_lowering_rejects_path_rom_assertions_before_reading() {
    for assertion in [
        "role='wildcard'",
        "role='system-bios'",
        "size=0",
        "size=32768",
    ] {
        let text = format!("schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='bios.bin'\nbase=0xF8000\n{assertion}\n");
        rejected(&text, &[], "assertions require a package binding");
    }
}

#[test]
fn stored_and_launch_checks_reject_path_rom_assertions_before_binding_rows() {
    use std::collections::BTreeMap;
    for assertion in [
        "role='wildcard'",
        "role='system-bios'",
        "size=0",
        "size=32768",
    ] {
        let text = format!("schema=1\nplatform='ibm-pc'\n[machine]\nschema=1\n[machine.board]\nmodel='ibm-5170'\n[machine.firmware.bios]\nimage='assets/bios.bin'\nbase=0xF8000\n{assertion}\n");
        let config = pc_config::image::Config::decode(text.parse().unwrap()).unwrap();
        let source = Counting::default();
        let error = pc_config::image::lower(
            config,
            &BTreeMap::new(),
            &source,
            true,
            assembly::AssemblyOptions::default(),
        )
        .err()
        .unwrap();
        assert!(
            error
                .to_string()
                .contains("assertions require a package binding"),
            "{assertion}: {error}"
        );
        let launch = pc_config::launch::decode(text.as_bytes()).unwrap();
        let error =
            pc_config::launch::check(&launch, assembly::AssemblyOptions::default()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("assertions require a package binding"),
            "{assertion}: {error}"
        );
        assert_eq!(source.opens.get(), 0);
    }
}

#[test]
fn preserved_paths_cannot_alias_bound_keys_with_dot_or_redundant_separators() {
    let package = BindingUse::new(
        "dos",
        "manifest",
        vec![BindingItem::new("disk", K::Medium, 368640, [2; 32])],
    );
    for path in [
        "assets/card-0/./drive-0/image.bin",
        "assets//card-0///drive-0//image.bin",
        "assets//card-0/./drive-0//image.bin",
    ] {
        let text = format!("{BIOS}[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='360K'\ninserted={{use='dos',medium='disk'}}\n[[card.drives]]\nunit=1\ntype='360K'\nimage='{path}'\n");
        rejected(
            &text,
            &[bios(), package.clone()],
            "collides with preserved source",
        );
    }
}

#[test]
fn bound_option_byte_errors_retain_selector_position_and_package_context() {
    use sha2::{Digest, Sha256};
    let Some(bios_bytes) = fixtures::fixture("kiselev-bios/bios-xi8088.bin") else {
        return;
    };
    struct BiosPath(Vec<u8>);
    impl ImageReader for BiosPath {
        fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
            assert_eq!(path, Path::new("bios.bin"));
            Ok(self.0.clone())
        }
        fn image_length(&self, _: &Path) -> io::Result<u64> {
            Ok(self.0.len() as u64)
        }
    }
    struct Source(Vec<u8>, Cell<usize>);
    impl ContentSource for Source {
        fn open(&self, digest: &[u8; 32], size: u64) -> Result<Content, Error> {
            assert_eq!(*digest, <[u8; 32]>::from(Sha256::digest(&self.0)));
            assert_eq!(size, self.0.len() as u64);
            self.1.set(self.1.get() + 1);
            Ok(Content::Bytes(self.0.clone()))
        }
    }
    let text = "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='bios.bin'\n[[firmware.option_roms]]\nuse='addon'\nrom='extension'\nbase=0xC8000\n";
    let (document, mut origins) = parsed(text);
    origins.insert(
        "firmware.option_roms[0].rom",
        pc_config::Origin {
            file: "parent.toml".into(),
            line: Some(42),
        },
    );
    for (header, expected) in [(true, "checksum"), (false, "55AAh")] {
        let mut bytes = vec![0; 2048];
        if header {
            bytes[..3].copy_from_slice(&[0x55, 0xAA, 4]);
        }
        let package = BindingUse::new(
            "addon",
            "manifest",
            vec![BindingItem::new(
                "extension",
                K::FirmwareRom,
                2048,
                Sha256::digest(&bytes).into(),
            )
            .with_roles(vec!["option-rom".into()])],
        );
        let source = Source(bytes, Cell::new(0));
        let error = bindings::lower_with_bindings(
            &document,
            &origins,
            &[package],
            &source,
            &mut BiosPath(bios_bytes.clone()),
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(source.1.get(), 1);
        assert_eq!(error.diagnostics[0].path, "firmware.option_roms[0].rom");
        for context in ["parent.toml:42", "use \"addon\"", "item \"extension\""] {
            assert!(
                error.diagnostics[0].message.contains(context),
                "missing {context}: {error:?}"
            );
        }
    }
}
