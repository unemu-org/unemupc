// SPDX-License-Identifier: AGPL-3.0-or-later
//! Synthetic system ROM parts, inheritance and integrity; machine-files § User-supplied split system BIOS.

use assembly::{AssemblyOptions, ImageSource};
use machine_file::{
    lower_with_options, parse_toml, parse_yaml, ImageReader, MachineFile, Origins, YamlOrigins,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

#[path = "../../pc-harness/tests/support/fixtures.rs"]
mod fixtures;

#[path = "../../pc-harness/tests/support/at_games.rs"]
mod game_profiles;

#[derive(Default)]
struct Images {
    files: BTreeMap<PathBuf, Vec<u8>>,
    reads: Vec<PathBuf>,
}
impl ImageReader for Images {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        self.reads.push(path.into());
        self.files.remove(path).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "missing synthetic chip or duplicate read",
            )
        })
    }
}
fn pin(text: &str) -> machine_file::schema::v1::Sha256 {
    parse_toml(
        "pins.toml",
        &format!("schema=1\n[firmware.bios]\nsha256='{text}'\n"),
    )
    .unwrap()
    .firmware
    .unwrap()
    .bios
    .unwrap()
    .sha256
    .unwrap()
}
fn inputs() -> Images {
    Images {
        files: [
            ("machines/roms/even", vec![0x12; 4096]),
            ("machines/roms/odd", vec![0x34; 4096]),
        ]
        .into_iter()
        .map(|(p, b)| (p.into(), b))
        .collect(),
        reads: vec![],
    }
}
const TOML: &str = "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nparts=[\n {image='roms/even',lane='even'},\n {image='roms/odd',lane='odd'},\n]\n";
const YAML: &str = "schema: 1\nboard: {model: ibm-5170}\nfirmware:\n  bios:\n    parts:\n      - {image: roms/even, lane: even}\n      - {image: roms/odd, lane: odd}\n";
fn parse(input: &str, yaml: bool) -> (MachineFile, Origins) {
    if yaml {
        (
            parse_yaml("machines/test.yaml", input).unwrap(),
            Origins::from_yaml("machines/test.yaml", input).unwrap(),
        )
    } else {
        (
            parse_toml("machines/test.toml", input).unwrap(),
            Origins::from_toml("machines/test.toml", input).unwrap(),
        )
    }
}
fn lower(
    document: &MachineFile,
    origins: &Origins,
    images: &mut Images,
) -> Result<assembly::MachineConfig, machine_file::LowerError> {
    lower_with_options(
        document,
        origins,
        images,
        AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
}
#[test]
fn host_firmware_files_and_parts_resolve_and_assemble() {
    use assembly::{ImagePart, Lane};

    let directory = Layers::new();
    let bytes = [0xF4, 0x90].repeat(16384);
    let image = directory.0.join("bios.bin");
    let even = directory.0.join("even.bin");
    let odd = directory.0.join("odd.bin");
    std::fs::write(&image, &bytes).unwrap();
    std::fs::write(&even, vec![0xF4; 16384]).unwrap();
    std::fs::write(&odd, vec![0x90; 16384]).unwrap();
    let options = AssemblyOptions {
        allow_unlisted_firmware: true,
    };
    for source in [
        ImageSource::File(image),
        ImageSource::Interleaved(vec![
            ImagePart {
                path: even,
                lane: Lane::Even,
                offset: 0,
            },
            ImagePart {
                path: odd,
                lane: Lane::Odd,
                offset: 0,
            },
        ]),
    ] {
        let mut config = assembly::bios(Vec::new(), None);
        config.images[0].provides.clear();
        config.images[0].source = source;
        let resolved = machine_file::test_support::resolve_file_sources(&config, options).unwrap();
        assert_eq!(
            resolved.images[0].source,
            ImageSource::Inline(bytes.clone())
        );
        let _assembled = machine_file::assemble_with_options(&config, options).unwrap();
    }
}

#[test]
fn parts_toml_yaml_parse_and_lower_identical_bytes_with_relative_paths() {
    for (input, yaml) in [(TOML, false), (YAML, true)] {
        let (document, origins) = parse(input, yaml);
        let rom = document.firmware.as_ref().unwrap().bios.as_ref().unwrap();
        assert!(rom.image.is_none());
        let parts = rom.parts.as_ref().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].offset, 0);
        assert_eq!(parts[1].offset, 0);
        assert_eq!(parts[0].lane, machine_file::schema::v1::Lane::Even);
        assert_eq!(parts[1].lane, machine_file::schema::v1::Lane::Odd);
        let mut images = inputs();
        let config = lower(&document, &origins, &mut images).unwrap();
        assert_eq!(
            config.images[0].source,
            ImageSource::Inline([0x12, 0x34].repeat(4096))
        );
        assert_eq!(config.images[0].base, 0xFE000);
        assert_eq!(
            images.reads,
            vec![
                PathBuf::from("machines/roms/even"),
                PathBuf::from("machines/roms/odd")
            ]
        );
        assert_eq!(origins.get("firmware.bios.parts[1].image").line, Some(7));
    }
}
#[test]
fn parts_pins_check_each_chip_and_assembled_bytes_with_origins() {
    for (input, yaml) in [(TOML, false), (YAML, true)] {
        let (mut document, mut origins) = parse(input, yaml);
        let rom = document.firmware.as_mut().unwrap().bios.as_mut().unwrap();
        rom.parts.as_mut().unwrap()[1].sha256 = Some(pin(&"00".repeat(32)));
        origins.insert(
            "firmware.bios.parts[1].sha256",
            machine_file::Origin {
                file: "pins.toml".into(),
                line: Some(19),
            },
        );
        let error = lower(&document, &origins, &mut inputs()).unwrap_err();
        assert_eq!(error.field, "firmware.bios.parts[1].image");
        assert_eq!(error.origin.line, Some(7));
        let file = if yaml {
            "machines/test.yaml"
        } else {
            "machines/test.toml"
        };
        assert_eq!(error.origin.file, Path::new(file));
        let actual = Sha256::digest(vec![0x34; 4096]);
        let expected = format!(
            concat!(
                "sha256 mismatch for machines/roms/odd: expected {}, ",
                "actual {actual:x}; pin set at pins.toml:19"
            ),
            "00".repeat(32),
            actual = actual
        );
        assert_eq!(error.message, expected);
        assert_eq!(
            error.pin_origin.as_deref(),
            Some(origins.get("firmware.bios.parts[1].sha256"))
        );
        let rom = document.firmware.as_mut().unwrap().bios.as_mut().unwrap();
        rom.parts.as_mut().unwrap()[1].sha256 =
            Some(pin(&format!("{:x}", Sha256::digest(vec![0x34; 4096]))));
        rom.sha256 = Some(pin(&"00".repeat(32)));
        origins.insert(
            "firmware.bios.sha256",
            machine_file::Origin {
                file: "assembled-pin.toml".into(),
                line: Some(23),
            },
        );
        let error = lower(&document, &origins, &mut inputs()).unwrap_err();
        let actual = Sha256::digest([0x12, 0x34].repeat(4096));
        let expected = format!(
            concat!(
                "sha256 mismatch for assembled BIOS: expected {}, ",
                "actual {actual:x}; pin set at assembled-pin.toml:23"
            ),
            "00".repeat(32),
            actual = actual
        );
        assert_eq!(error.message, expected);
        assert_eq!(
            error.pin_origin.as_deref(),
            Some(origins.get("firmware.bios.sha256"))
        );
        document
            .firmware
            .as_mut()
            .unwrap()
            .bios
            .as_mut()
            .unwrap()
            .sha256 = Some(pin(&format!(
            "{:x}",
            Sha256::digest([0x12, 0x34].repeat(4096))
        )));
        assert!(lower(&document, &origins, &mut inputs()).is_ok());
    }
}
#[test]
fn parts_mutual_exclusion_option_roms_and_invalid_layout_are_exact() {
    let (mut document, origins) = parse(TOML, false);
    document
        .firmware
        .as_mut()
        .unwrap()
        .bios
        .as_mut()
        .unwrap()
        .image = Some("other".into());
    let mut images = inputs();
    assert_eq!(
        lower(&document, &origins, &mut images).unwrap_err().message,
        "image and parts are mutually exclusive"
    );
    assert!(images.reads.is_empty());
    document
        .firmware
        .as_mut()
        .unwrap()
        .bios
        .as_mut()
        .unwrap()
        .image = None;
    let mut option = document
        .firmware
        .as_ref()
        .unwrap()
        .bios
        .as_ref()
        .unwrap()
        .clone();
    option.base = Some(0xC8000);
    document.firmware.as_mut().unwrap().option_roms = Some(vec![option]);
    assert_eq!(
        lower(&document, &origins, &mut inputs())
            .unwrap_err()
            .message,
        "option ROMs cannot use parts; only firmware.bios supports parts"
    );
    document.firmware.as_mut().unwrap().option_roms = None;
    document
        .firmware
        .as_mut()
        .unwrap()
        .bios
        .as_mut()
        .unwrap()
        .parts
        .as_mut()
        .unwrap()[1]
        .offset = 2;
    assert_eq!(
        lower(&document, &origins, &mut inputs())
            .unwrap_err()
            .message,
        "firmware parts: offset 0 requires exactly one even and one odd part"
    );
}
struct Layers(PathBuf);
impl Layers {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "machine-rom-layers-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("child")).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }
    fn resolve(
        &self,
        parent: &str,
        child: &str,
    ) -> Result<(MachineFile, Origins), machine_file::LoadError> {
        std::fs::write(self.0.join("parent.toml"), parent).unwrap();
        std::fs::write(
            self.0.join("child/layer.toml"),
            format!("schema=1\nextends='../parent.toml'\n{child}"),
        )
        .unwrap();
        machine_file::resolve(self.0.join("child/layer.toml"))
    }
}
impl Drop for Layers {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn parts_inheritance_replaces_sources_clears_pins_and_preserves_each_path_origin() {
    let layers = Layers::new();
    let parent = format!("{TOML}sha256='{}'\n", "00".repeat(32));
    let (document, origins) = layers
        .resolve(&parent, "[firmware.bios]\nprovides=['floppy']\n")
        .unwrap();
    let mut images = Images::default();
    images
        .files
        .insert(layers.0.join("roms/even"), vec![0x12; 4096]);
    images
        .files
        .insert(layers.0.join("roms/odd"), vec![0x34; 4096]);
    let error = lower(&document, &origins, &mut images).unwrap_err();
    assert!(error.message.contains("pin set at"), "{error}");
    assert!(error.message.contains("parent.toml:9"));
    assert_eq!(
        images.reads,
        vec![layers.0.join("roms/even"), layers.0.join("roms/odd")]
    );
    let image_parent = concat!(
        "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='parent.bin'\n",
        "sha256='0000000000000000000000000000000000000000000000000000000000000000'\n"
    );
    let parts_child = concat!(
        "[firmware.bios]\nparts=[",
        "{image='even',lane='even'},{image='odd',lane='odd'}]\n"
    );
    for (parent, child, parts) in [
        (
            parent.as_str(),
            "[firmware.bios]\nimage='child.bin'\n",
            false,
        ),
        (image_parent, parts_child, true),
        (parent.as_str(), parts_child, true),
    ] {
        let (document, origins) = layers.resolve(parent, child).unwrap();
        let rom = document.firmware.as_ref().unwrap().bios.as_ref().unwrap();
        assert_eq!(rom.parts.is_some(), parts);
        assert_eq!(rom.image.is_some(), !parts);
        assert!(rom.sha256.is_none());
        let mut images = Images::default();
        if parts {
            images
                .files
                .insert(layers.0.join("child/even"), vec![0x12; 4096]);
            images
                .files
                .insert(layers.0.join("child/odd"), vec![0x34; 4096]);
        } else {
            images
                .files
                .insert(layers.0.join("child/child.bin"), [0x12, 0x34].repeat(4096));
        }
        assert!(lower(&document, &origins, &mut images).is_ok());
        assert_eq!(
            origins
                .get(if parts {
                    "firmware.bios.parts[0].image"
                } else {
                    "firmware.bios.image"
                })
                .file,
            layers.0.join("child/layer.toml")
        );
    }
    let pin = format!("{:x}", Sha256::digest([0x12, 0x34].repeat(4096)));
    let child = format!(
        concat!(
            "[firmware.bios]\nparts=[",
            "{{image='even',lane='even'}},{{image='odd',lane='odd'}}]\n",
            "sha256='{pin}'\n"
        ),
        pin = pin
    );
    let (document, origins) = layers.resolve(&parent, &child).unwrap();
    assert_eq!(
        document
            .firmware
            .unwrap()
            .bios
            .unwrap()
            .sha256
            .unwrap()
            .as_str(),
        pin
    );
    assert_eq!(
        origins.get("firmware.bios.sha256").file,
        layers.0.join("child/layer.toml")
    );
}
#[test]
fn parts_resolver_rejects_conflicting_sources_and_option_parts_in_layer() {
    let layers = Layers::new();
    assert!(layers
        .resolve(TOML, "[firmware.bios]\nimage='x'\nparts=[]\n")
        .unwrap_err()
        .to_string()
        .ends_with("image and parts are mutually exclusive"));
    assert!(layers
        .resolve(TOML, "[firmware]\noption_roms=[{parts=[],base=819200}]\n")
        .unwrap_err()
        .to_string()
        .ends_with("option ROMs cannot use parts; only firmware.bios supports parts"));
}

#[test]
fn parts_explicit_offsets_and_pins_parse_in_both_formats() {
    let even = format!("{:x}", Sha256::digest(vec![0x12; 2048]));
    let odd = format!("{:x}", Sha256::digest(vec![0x34; 2048]));
    let toml = format!("schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nparts=[{{image='roms/even',lane='even',sha256='{even}'}},{{image='roms/odd',lane='odd',sha256='{odd}'}},{{image='roms/high-even',lane='even',offset=4096,sha256='{odd}'}},{{image='roms/high-odd',lane='odd',offset=4096,sha256='{even}'}}]\n");
    let yaml = format!("schema: 1\nboard: {{model: ibm-5170}}\nfirmware:\n  bios:\n    parts:\n      - {{image: roms/even, lane: even, sha256: {even}}}\n      - {{image: roms/odd, lane: odd, sha256: {odd}}}\n      - {{image: roms/high-even, lane: even, offset: 4096, sha256: {odd}}}\n      - {{image: roms/high-odd, lane: odd, offset: 4096, sha256: {even}}}\n");
    for (input, yaml) in [(toml, false), (yaml, true)] {
        let (document, origins) = parse(&input, yaml);
        let mut images = Images::default();
        for (name, byte) in [
            ("even", 0x12),
            ("odd", 0x34),
            ("high-even", 0x34),
            ("high-odd", 0x12),
        ] {
            images
                .files
                .insert(Path::new("machines/roms").join(name), vec![byte; 2048]);
        }
        let config = lower(&document, &origins, &mut images).unwrap();
        assert_eq!(
            config.images[0].source,
            ImageSource::Inline([[0x12, 0x34].repeat(2048), [0x34, 0x12].repeat(2048)].concat())
        );
        assert_eq!(
            origins.get("firmware.bios.parts[3].offset").line,
            Some(if yaml { 9 } else { 5 })
        );
    }
}

#[test]
fn parts_yaml_inheritance_keeps_parent_part_paths_and_replaces_with_child_image() {
    let layers = Layers::new();
    std::fs::write(layers.0.join("parent.yaml"), YAML).unwrap();
    let child = layers.0.join("child/layer.yaml");
    std::fs::write(
        &child,
        "schema: 1\nextends: ../parent.yaml\nfirmware:\n  bios:\n    provides: [floppy]\n",
    )
    .unwrap();
    let (document, origins) = machine_file::resolve(&child).unwrap();
    let mut images = Images::default();
    images
        .files
        .insert(layers.0.join("roms/even"), vec![0x12; 4096]);
    images
        .files
        .insert(layers.0.join("roms/odd"), vec![0x34; 4096]);
    assert!(lower(&document, &origins, &mut images).is_ok());
    assert_eq!(
        origins.get("firmware.bios.parts[1].image").file,
        layers.0.join("child/../parent.yaml")
    );
    assert_eq!(origins.get("firmware.bios.parts[1].image").line, Some(7));
    std::fs::write(
        &child,
        "schema: 1\nextends: ../parent.yaml\nfirmware:\n  bios:\n    image: child.bin\n",
    )
    .unwrap();
    let (document, origins) = machine_file::resolve(&child).unwrap();
    assert!(document
        .firmware
        .as_ref()
        .unwrap()
        .bios
        .as_ref()
        .unwrap()
        .parts
        .is_none());
    let mut images = Images::default();
    images
        .files
        .insert(layers.0.join("child/child.bin"), vec![0; 8192]);
    assert!(lower(&document, &origins, &mut images).is_ok());
}

#[test]
fn parts_source_replacement_drops_inherited_services_and_keeps_restatements_toml_yaml() {
    let layers = Layers::new();
    for yaml in [false, true] {
        let extension = if yaml { "yaml" } else { "toml" };
        let parent = layers.0.join(format!("services.{extension}"));
        let base = if yaml {
            "schema: 1\nboard: {model: ibm-5170}\nfirmware:\n  bios: {image: parent.bin, provides: [floppy]}\ncard:\n  - kind: fdc-765\n    drives: [{unit: 0, type: 1.44M, image: disk.img}]\n"
        } else {
            "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='parent.bin'\nprovides=['floppy']\n[[card]]\nkind='fdc-765'\ndrives=[{unit=0,type='1.44M',image='disk.img'}]\n"
        };
        std::fs::write(&parent, base).unwrap();
        for parts in [false, true] {
            for restate in [false, true] {
                let child = layers.0.join(format!("child/services.{extension}"));
                let source = match (yaml, parts) {
                    (true, true) => {
                        "    parts: [{image: even, lane: even}, {image: odd, lane: odd}]\n"
                    }
                    (true, false) => "    image: child.bin\n",
                    (false, true) => {
                        "parts=[{image='even',lane='even'},{image='odd',lane='odd'}]\n"
                    }
                    (false, false) => "image='child.bin'\n",
                };
                let declaration = match (yaml, restate) {
                    (true, true) => "    provides: [floppy]\n",
                    (false, true) => "provides=['floppy']\n",
                    (_, false) => "",
                };
                let header = if yaml {
                    format!("schema: 1\nextends: ../services.yaml\nfirmware:\n  bios:\n{source}{declaration}")
                } else {
                    format!("schema=1\nextends='../services.toml'\n[firmware.bios]\n{source}{declaration}")
                };
                std::fs::write(&child, header).unwrap();
                let (doc, origins) = machine_file::resolve(&child).unwrap();
                assert_eq!(
                    doc.firmware
                        .as_ref()
                        .unwrap()
                        .bios
                        .as_ref()
                        .unwrap()
                        .provides
                        .is_some(),
                    restate
                );
                let mut images = Images::default();
                for (name, bytes) in [
                    ("child/even", vec![0x12; 4096]),
                    ("child/odd", vec![0x34; 4096]),
                    ("child/child.bin", [0x12, 0x34].repeat(4096)),
                    ("disk.img", vec![0; 1_474_560]),
                ] {
                    images.files.insert(layers.0.join(name), bytes);
                }
                let result = lower(&doc, &origins, &mut images);
                if restate {
                    assert!(result.is_ok(), "{}", result.unwrap_err());
                } else {
                    assert_eq!(
                        result.unwrap_err().message,
                        "device Fdc needs firmware service floppy; no ROM provides it"
                    );
                }
            }
        }
    }
}

#[test]
fn parts_bios_placement_names_parts_field() {
    let (mut document, origins) = parse(TOML, false);
    document
        .firmware
        .as_mut()
        .unwrap()
        .bios
        .as_mut()
        .unwrap()
        .base = Some(0xF8000);
    let error = lower(&document, &origins, &mut inputs()).unwrap_err();
    assert_eq!(error.field, "firmware.bios.parts");
    assert_eq!(error.origin, origins.get("firmware.bios.parts").clone());
}

#[test]
fn repository_machine_cpu_regions_cover_the_full_firmware_board_space() {
    for relative in [
        "seabios/bios-1.17.0-isa.bin",
        "kiselev-bios/bios-xi8088.bin",
        "kiselev-bios/bios-xi8088-atreset.bin",
        "unemu-at-bios/bios-unemu-at.bin",
        "freedos/freedos.img",
        "xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin",
        "hdd/fat16.img",
    ] {
        if fixtures::fixture(relative).is_none() {
            return;
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../machines");
    let mut names = Vec::new();
    for directory in [&root, &root.join("presets")] {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("toml" | "yaml" | "yml")
            ) {
                names.push(path.strip_prefix(&root).unwrap().to_path_buf());
            }
        }
    }
    names.sort();
    assert_eq!(
        names,
        [
            "at-freedos-from-preset.toml",
            "at-freedos.toml",
            "at-freedos.yaml",
            "at-games-open.toml",
            "at-games.toml",
            "at-ide.toml",
            "generic-386-freedos.toml",
            "generic-386-ide.toml",
            "generic-386-seabios.toml",
            "generic-486-freedos.toml",
            "generic-486-ide.toml",
            "generic-486-seabios.toml",
            "presets/ibm-5170.toml",
        ]
        .map(PathBuf::from)
    );
    // This snapshot guards the repository machines' lower-memory maps while
    // board-owned adapter gaps, empty bus and ROM aliases evolve. Keep the
    // production mapping order as well as ownership of every range.
    let ordinary = vec![
        (0, 0xB0000, "ram"),
        (0xB0000, 0xC0000, "mmio"),
        (0xC0000, 0xF8000, "rom"),
        (0xF8000, 0x100000, "rom"),
        (0x100000, 0xFE0000, "empty-bus"),
        (0xFE0000, 0x1000000, "rom"),
    ];
    let ide = vec![
        (0, 0xB0000, "ram"),
        (0xB0000, 0xC0000, "mmio"),
        (0xC0000, 0xC8000, "rom"),
        (0xCA000, 0xF8000, "rom"),
        (0xF8000, 0x100000, "rom"),
        (0xC8000, 0xCA000, "rom"),
        (0x100000, 0xFE0000, "empty-bus"),
        (0xFE0000, 0x1000000, "rom"),
    ];
    let generic386 = vec![
        (0, 0xA0000, "ram"),
        (0xC0000, 0x100000, "ram"),
        (0x100000, 0x1000000, "ram"),
        (0xA0000, 0xC0000, "mmio"),
        (0xFFFE0000, 0x100000000, "rom"),
    ];
    let options = AssemblyOptions::default();
    let mut checked = 0;
    for name in names {
        if matches!(
            name.to_str(),
            Some(game_profiles::IBM | game_profiles::OPEN)
        ) {
            // Local game media and proprietary firmware are never CI inputs.
            // An unidentified BIOS cannot establish the C0000h video scan.
            let profile = game_profiles::Profile::new(name.to_str().unwrap(), true);
            let ibm = name == Path::new(game_profiles::IBM);
            profile.relocate_synthetic_video();
            let options = AssemblyOptions {
                allow_unlisted_firmware: true,
            };
            let config = machine_file::load_with_options(&profile.file, options).unwrap();
            let expected = if ibm {
                vec![
                    (0, 0xA0000, "ram"),
                    (0x100000, 0x200000, "ram"),
                    (0xA0000, 0xC0000, "mmio"),
                    (0xD0000, 0xE0000, "mmio"),
                    (0xC0000, 0xC8000, "rom"),
                    (0xCE000, 0xD0000, "rom"),
                    (0xE0000, 0xF0000, "rom"),
                    (0xF0000, 0x100000, "rom"),
                    (0xC8000, 0xCE000, "rom"),
                    (0x200000, 0xFE0000, "empty-bus"),
                    (0xFE0000, 0x1000000, "rom"),
                ]
            } else {
                // The VGA window starts at A0000h; the synthetic video ROM
                // stand-in sits at E0000h, clear of XTIDE and the EMS frame.
                vec![
                    (0, 0xA0000, "ram"),
                    (0x100000, 0x200000, "ram"),
                    (0xA0000, 0xC0000, "mmio"),
                    (0xD0000, 0xE0000, "mmio"),
                    (0xC0000, 0xC8000, "rom"),
                    (0xCA000, 0xD0000, "rom"),
                    (0xE8000, 0xF8000, "rom"),
                    (0xF8000, 0x100000, "rom"),
                    (0xE0000, 0xE8000, "rom"),
                    (0xC8000, 0xCA000, "rom"),
                    (0x200000, 0xFE0000, "empty-bus"),
                    (0xFE0000, 0x1000000, "rom"),
                ]
            };
            assert_eq!(
                machine_file::test_support::cpu_mapping_regions(&config, options).unwrap(),
                expected,
                "{}",
                name.display()
            );
            assert!(!profile.journal().exists());
            checked += 1;
        } else if name.starts_with("presets") {
            for image in ["bios-xi8088.bin", "bios-xi8088-atreset.bin"] {
                let layers = Layers::new();
                let path = layers.0.join("preset.toml");
                let image = root
                    .join("../vendor/kiselev-bios")
                    .join(image)
                    .canonicalize()
                    .unwrap();
                std::fs::write(
                    &path,
                    format!(
                        "schema=1\nextends='preset:ibm-5170'\n[firmware.bios]\nimage='{}'\n",
                        image.display(),
                    ),
                )
                .unwrap();
                let loaded = machine_file::load_for_assembly(path, options).unwrap();
                assert_eq!(
                    machine_file::test_support::cpu_mapping_regions(&loaded.config, options)
                        .unwrap(),
                    ordinary,
                    "preset mapping"
                );
                checked += 1;
            }
        } else {
            let loaded = machine_file::load_for_assembly(root.join(&name), options).unwrap();
            let expected = if name == Path::new("at-ide.toml") {
                &ide
            } else if loaded.config.board.is_generic_isa() {
                &generic386
            } else {
                &ordinary
            };
            assert_eq!(
                &machine_file::test_support::cpu_mapping_regions(&loaded.config, options).unwrap(),
                expected,
                "{}",
                name.display()
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 14,
        "every machine file and both preset BIOS variants"
    );
}

#[test]
#[ignore = "requires separately supplied proprietary firmware; identity and diagnostics only"]
fn user_rom_landmark_at_identity() {
    let catalogue = assembly::BoardModel::Ibm5170.firmware_catalogue();
    let found = machine_file::require_user_firmware!("landmark-rom-post-at-1.2.0", catalogue);
    let entry = catalogue
        .iter()
        .find(|entry| entry.id == "landmark-rom-post-at-1.2.0")
        .expect("catalogued diagnostic");
    let mut config = assembly::bios(Vec::new(), Some(Vec::new()));
    config.drives.clear();
    config.images[0].base = 0xF0000;
    config.images[0].provides.clear();
    config.images[0].source = found.source.clone();
    let resolved = found.expect_redacted(
        machine_file::test_support::resolve_file_sources(&config, AssemblyOptions::default()),
        "resolving diagnostic parts",
    );
    let ImageSource::Inline(bytes) = &resolved.images[0].source else {
        panic!("expected resolved diagnostic bytes")
    };
    assert_eq!(bytes.len() as u64, entry.size);
    assert_eq!(format!("{:x}", Sha256::digest(bytes)), entry.sha256);
    assert!(
        bytes[..32768] == bytes[32768..],
        "mirrored diagnostic halves"
    );
    for (lane, part) in entry.parts[0].iter().enumerate() {
        let chip: Vec<_> = bytes.iter().skip(lane).step_by(2).copied().collect();
        assert_eq!(chip.len() as u64, part.size, "{} size", part.label);
        assert_eq!(
            format!("{:x}", Sha256::digest(&chip)),
            part.sha256,
            "{} hash",
            part.label
        );
    }
    found.expect_redacted(
        assembly::check_board_system_bios(
            &resolved,
            resolved.board,
            0xF0000,
            bytes,
            AssemblyOptions::default(),
        ),
        "checking diagnostic identity",
    );
    machine_file::validate(&resolved).unwrap();
    assert!(resolved.images[0].provides.is_empty());
}

#[test]
#[ignore = "requires separately supplied proprietary firmware; identity and diagnostics only"]
fn user_rom_ibm_1985_identity() {
    use assembly::{ImagePart, Lane};
    let catalogue = assembly::BoardModel::Ibm5170.firmware_catalogue();
    let found = machine_file::require_user_firmware!("ibm-5170-bios-1985-06-10", catalogue);
    let entry = found.expect_redacted(
        catalogue
            .iter()
            .find(|e| e.id == "ibm-5170-bios-1985-06-10")
            .ok_or("missing catalogue entry"),
        "selecting firmware",
    );
    let split = Layers::new();
    let chips: Vec<_> = match &found.source {
        ImageSource::Interleaved(parts) => parts
            .iter()
            .map(|part| found.expect_redacted(std::fs::read(&part.path), "reading firmware part"))
            .collect(),
        ImageSource::File(path) => {
            let bytes = found.expect_redacted(std::fs::read(path), "reading assembled firmware");
            (0..2)
                .map(|lane| bytes.iter().skip(lane).step_by(2).copied().collect())
                .collect()
        }
        _ => panic!("discovery must return file-backed firmware"),
    };
    // Stable temporary names keep arbitrary dump filenames out of machine-file diagnostics.
    let paths = [split.0.join("even"), split.0.join("odd")];
    for (path, chip) in paths.iter().zip(&chips) {
        found.expect_redacted(
            std::fs::write(path, chip),
            "writing temporary firmware part",
        );
    }
    for (chip, known) in chips.iter().zip(entry.parts[0]) {
        assert_eq!(format!("{:x}", Sha256::digest(chip)), known.sha256);
    }
    let config_for = |paths: &[PathBuf; 2]| {
        let mut config = assembly::bios(vec![], Some(vec![0; 1_474_560]));
        config.images[0].base = 0xF0000;
        config.images[0].provides.clear();
        config.images[0].source = ImageSource::Interleaved(
            paths
                .iter()
                .zip([Lane::Even, Lane::Odd])
                .map(|(path, lane)| ImagePart {
                    path: path.clone(),
                    lane,
                    offset: 0,
                })
                .collect(),
        );
        config
    };
    let document_for = |paths: &[PathBuf; 2]| {
        let input = format!(
            "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nparts=[\
             {{image='even',lane='even'}},{{image='odd',lane='odd'}}]\nsha256='{}'\n",
            entry.sha256
        );
        let mut document = found.expect_redacted(
            parse_toml("machines/test.toml", &input),
            "parsing firmware document",
        );
        let origins = found.expect_redacted(
            Origins::from_toml("machines/test.toml", &input),
            "locating firmware fields",
        );
        let parts = found.expect_redacted(
            document
                .firmware
                .as_mut()
                .and_then(|firmware| firmware.bios.as_mut())
                .and_then(|bios| bios.parts.as_mut())
                .ok_or("missing firmware parts"),
            "selecting document parts",
        );
        for (part, path) in parts.iter_mut().zip(paths) {
            part.image = found
                .expect_redacted(
                    path.to_str().ok_or("non-UTF-8 temporary path"),
                    "setting document path",
                )
                .into();
        }
        (document, origins)
    };
    let default = AssemblyOptions::default();
    let escaped = AssemblyOptions {
        allow_unlisted_firmware: true,
    };
    let mut config = config_for(&paths);
    config.images[0].source = found.source.clone();
    assert_eq!(config.board, assembly::BoardModel::Ibm5170);
    found.expect_redacted(
        machine_file::validate(&config),
        "validating firmware config",
    );
    let resolved = found.expect_redacted(
        machine_file::test_support::resolve_file_sources(&config, default),
        "resolving firmware",
    );
    found.expect_redacted(
        machine_file::validate(&resolved),
        "validating resolved firmware",
    );
    let ImageSource::Inline(bytes) = &resolved.images[0].source else {
        panic!("unresolved BIOS")
    };
    assert!(entry.identifies(bytes));
    found.expect_redacted(
        assembly::check_board_system_bios(&resolved, resolved.board, 0xF0000, bytes, default),
        "checking firmware identity",
    );
    let (doc, origins) = document_for(&paths);
    let lowered = found.expect_redacted(
        machine_file::lower(&doc, &origins, &mut machine_file::FileImageReader),
        "lowering firmware document",
    );
    assert_eq!(lowered.images[0].base, 0xF0000);
    assert!(
        lowered.images[0].source == resolved.images[0].source,
        "machine-file bytes differ from resolved BIOS"
    );
    assert_eq!(lowered.images[0].provides, entry.provides);
    let swapped = [paths[1].clone(), paths[0].clone()];
    let message = concat!(
        "firmware parts are swapped for ibm-5170-bios-1985-06-10: ",
        "even lane offset 0 holds U47 6480091, which belongs in the odd lane at offset 0; ",
        "odd lane offset 0 holds U27 6480090, which belongs in the even lane at offset 0"
    );
    assert_eq!(
        found.redact(
            &found
                .expect_err_redacted(
                    machine_file::test_support::resolve_file_sources(
                        &config_for(&swapped),
                        default
                    ),
                    "checking swapped firmware",
                )
                .to_string()
        ),
        message
    );
    found.expect_redacted(
        machine_file::test_support::resolve_file_sources(&config_for(&swapped), escaped),
        "resolving swapped firmware with permission",
    );
    let (doc, origins) = document_for(&swapped);
    let error = found.expect_err_redacted(
        machine_file::lower(&doc, &origins, &mut machine_file::FileImageReader),
        "checking swapped firmware document",
    );
    assert_eq!(error.field, "firmware.bios.parts");
    assert_eq!(found.redact(&error.message), message);
    let error = found.expect_err_redacted(
        machine_file::lower_with_options(
            &doc,
            &origins,
            &mut machine_file::FileImageReader,
            escaped,
        ),
        "checking assembled pin after swapped firmware",
    );
    assert!(error
        .message
        .starts_with("sha256 mismatch for assembled BIOS:"));
    let temp = Layers::new();
    let mut corrupted = chips[1].clone();
    corrupted[0] ^= 1;
    let wrong = temp.0.join("corrupt-odd");
    found.expect_redacted(
        std::fs::write(&wrong, &corrupted),
        "writing temporary corrupt firmware",
    );
    let wrong = [paths[0].clone(), wrong];
    let message = format!(
        "firmware parts do not match ibm-5170-bios-1985-06-10: odd lane offset 0: \
         expected U47 6480091 sha256 {}, actual {:x} (bad dump or wrong chip)",
        entry.parts[0][1].sha256,
        Sha256::digest(&corrupted)
    );
    assert_eq!(
        found.redact(
            &found
                .expect_err_redacted(
                    machine_file::test_support::resolve_file_sources(&config_for(&wrong), default),
                    "checking corrupt firmware",
                )
                .to_string()
        ),
        message
    );
    let (doc, origins) = document_for(&wrong);
    assert_eq!(
        found.redact(
            &found
                .expect_err_redacted(
                    machine_file::lower(&doc, &origins, &mut machine_file::FileImageReader),
                    "checking corrupt firmware document",
                )
                .message
        ),
        message
    );
}

#[test]
fn repository_machines_and_preset_keep_effective_firmware_services() {
    for relative in [
        "kiselev-bios/bios-xi8088.bin",
        "unemu-at-bios/bios-unemu-at.bin",
        "freedos/freedos.img",
        "xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin",
        "hdd/fat16.img",
    ] {
        if fixtures::fixture(relative).is_none() {
            return;
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../machines");
    let mut floppy_configs = Vec::new();
    for file in [
        "at-freedos.toml",
        "at-freedos.yaml",
        "at-freedos-from-preset.toml",
        "at-ide.toml",
    ] {
        let config = machine_file::load(root.join(file)).unwrap();
        assert_eq!(
            config.images[0].provides,
            vec![assembly::FirmwareService::Floppy],
            "{file}"
        );
        if file == "at-ide.toml" {
            assert_eq!(
                config.images[1].provides,
                vec![assembly::FirmwareService::FixedDisk]
            );
        } else if !file.starts_with("presets/") {
            floppy_configs.push(config);
        }
    }
    assert_eq!(floppy_configs[0], floppy_configs[1]);
    assert_eq!(floppy_configs[0], floppy_configs[2]);
    let preset = std::fs::read_to_string(root.join("presets/ibm-5170.toml")).unwrap();
    let bios = parse_toml("preset.toml", &preset)
        .unwrap()
        .firmware
        .unwrap()
        .bios
        .unwrap();
    assert!(bios.image.is_none());
    assert!(bios.provides.is_none());
    let layers = Layers::new();
    let child = layers.0.join("preset-child.toml");
    std::fs::write(
        &child,
        format!(
            "schema=1\nextends='preset:ibm-5170'\n[firmware.bios]\nimage='{}'\n",
            root.join("../vendor/kiselev-bios/bios-xi8088.bin")
                .display()
        ),
    )
    .unwrap();
    let config = machine_file::load(child).unwrap();
    assert_eq!(
        config.images[0].provides,
        vec![assembly::FirmwareService::Floppy]
    );
}

#[test]
fn inherited_parts_repin_reports_the_child_pin_in_both_syntaxes() {
    let layers = Layers::new();
    let pin = "0".repeat(64);
    for (ext, parent, child, pin_line) in [
        (
            "toml",
            TOML,
            format!("schema=1\nextends='../parent.toml'\n[firmware.bios]\nsha256='{pin}'\n"),
            4,
        ),
        (
            "yaml",
            YAML,
            format!(
                "schema: 1\nextends: ../parent.yaml\nfirmware:\n  bios:\n    sha256: '{pin}'\n"
            ),
            5,
        ),
    ] {
        let parent_file = layers.0.join(format!("parent.{ext}"));
        let child_file = layers.0.join(format!("child/layer.{ext}"));
        std::fs::write(&parent_file, parent).unwrap();
        std::fs::write(&child_file, child).unwrap();
        let (document, origins) = machine_file::resolve(&child_file).unwrap();
        let mut images = Images {
            files: [
                (layers.0.join("roms/even"), vec![0x12; 4096]),
                (layers.0.join("roms/odd"), vec![0x34; 4096]),
            ]
            .into(),
            reads: vec![],
        };
        let error = lower(&document, &origins, &mut images).unwrap_err();
        let pin_origin = machine_file::Origin {
            file: child_file,
            line: Some(pin_line),
        };
        assert_eq!(error.field, "firmware.bios.parts");
        assert_eq!(
            error.origin.file,
            layers.0.join(format!("child/../parent.{ext}"))
        );
        assert_eq!(error.pin_origin.as_deref(), Some(&pin_origin));
        assert_eq!(error.message, format!(
            "sha256 mismatch for assembled BIOS: expected {pin}, actual {:x}; pin set at {pin_origin}",
            Sha256::digest([0x12, 0x34].repeat(4096)),
        ));
        assert_eq!(
            images.reads,
            vec![layers.0.join("roms/even"), layers.0.join("roms/odd")]
        );
    }
}
