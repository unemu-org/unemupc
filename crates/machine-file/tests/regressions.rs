// SPDX-License-Identifier: AGPL-3.0-or-later
//! Format-neutral and boundary contracts from `backlog/docs/design/machine-files.md`.

use std::{error::Error, fmt::Debug};

use machine_file::{
    parse_toml,
    schema::v1::{
        Board, BoardModel, Card, CardKind, Channel, Cpu, CpuModel, Drive, DriveType, Firmware,
        FirmwareService, Geometry, MachineFile, Memory, NumberOrNone, Nvram, WriteMode,
    },
    ParseError,
};
use serde::{
    de::{value, DeserializeOwned, IntoDeserializer, Visitor},
    Deserializer,
};

// A null value must reach the field decoder without a YAML dependency. The
// integer form lets the same map provide the top-level schema discriminator.
#[derive(Clone, Copy)]
enum Cell {
    Null,
    Integer(i64),
}

impl<'de> IntoDeserializer<'de, value::Error> for Cell {
    type Deserializer = Self;
    fn into_deserializer(self) -> Self {
        self
    }
}

impl<'de> Deserializer<'de> for Cell {
    type Error = value::Error;
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self {
            Self::Null => visitor.visit_unit(),
            Self::Integer(integer) => visitor.visit_i64(integer),
        }
    }
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum
        identifier ignored_any
    }
}

fn null_fields<T: DeserializeOwned + Debug>(fields: &[&str], schema: bool, message: &str) {
    let base = if schema {
        vec![("schema", Cell::Integer(1))]
    } else {
        vec![]
    };
    T::deserialize(value::MapDeserializer::<_, value::Error>::new(
        base.clone().into_iter(),
    ))
    .unwrap();
    for field in fields {
        let mut entries = base.clone();
        entries.push((field, Cell::Null));
        let error = T::deserialize(value::MapDeserializer::<_, value::Error>::new(
            entries.into_iter(),
        ))
        .expect_err(field);
        assert!(error.to_string().contains(message), "{field}: {error}");
    }
}

#[test]
fn rejection_fields_reject_null_presence_in_every_table() {
    let timing = [
        "mhz",
        "frequency",
        "clock_hz",
        "cycles",
        "cycle_exact",
        "wait_states",
        "bus_mhz",
        "memory_wait_states",
        "timing",
    ];
    null_fields::<MachineFile>(&timing, true, "cycle-exact timing is not modelled");
    null_fields::<Board>(&timing, false, "cycle-exact timing is not modelled");
    null_fields::<Cpu>(&timing, false, "cycle-exact timing is not modelled");
    null_fields::<Card>(&timing, false, "cycle-exact timing is not modelled");
    null_fields::<MachineFile>(
        &["run", "presentation", "streams"],
        true,
        "belongs outside the machine file",
    );
    null_fields::<Board>(
        &[
            "a20",
            "kbc",
            "keyboard",
            "keyboard_layout",
            "dip_switches",
            "chipset",
            "speaker",
        ],
        false,
        "reserved, not implemented",
    );
    null_fields::<Cpu>(&["turbo"], false, "reserved, not implemented");
    null_fields::<Memory>(
        &["shadow", "umb", "upper"],
        false,
        "reserved, not implemented",
    );
    null_fields::<Firmware>(&["basic"], false, "reserved, not implemented");
    null_fields::<Card>(
        &["monitor", "rom", "mmio"],
        false,
        "reserved, not implemented",
    );
    null_fields::<Drive>(&["media"], false, "sequence");
}

#[test]
fn reserved_cpu_names_explain_the_missing_model() {
    for (name, reason) in [
        (
            "8086",
            "needs an 8086 instruction model, which the CPU core does not have yet",
        ),
        (
            "8088",
            "needs an 8088 instruction model, which the CPU core does not have yet",
        ),
        (
            "v20",
            "needs a V20 instruction model, which the CPU core does not have yet",
        ),
        (
            "80386sx",
            "needs an 80386SX CPU model for firmware machines",
        ),
        ("80486", "select 80486dx or 80486dx-cpuid explicitly"),
        ("pentium", "needs a Pentium CPU model for firmware machines"),
        (
            "pentium-ii",
            "needs a Pentium II CPU model for firmware machines",
        ),
        (
            "pentium-iii",
            "needs a Pentium III CPU model for firmware machines",
        ),
    ] {
        let error = parse_toml(
            "cpu.toml",
            &format!("schema = 1\n[cpu]\nmodel = \"{name}\"\n"),
        )
        .unwrap_err();
        assert_eq!(error.line_column(), Some((3, 9)));
        assert!(error.message().contains(reason), "{error}");
        assert!(error.message().contains("reserved, not implemented"));
    }
}

#[test]
fn format_neutral_errors_support_optional_context() {
    let error = ParseError::new(
        "other-format.yaml",
        Some((4, 7)),
        "invalid board",
        Some("board: unknown\n      ^".to_owned()),
    );
    assert_eq!(
        error.to_string(),
        "other-format.yaml: line 4, column 7: invalid board\nboard: unknown\n      ^"
    );
    assert_eq!(error.message(), "invalid board");
    assert_eq!(error.snippet(), Some("board: unknown\n      ^"));
    assert_eq!(error.span(), None);
    assert!(error.source().is_none());
    let plain = ParseError::new("plain.toml", None, "missing schema", None);
    assert_eq!(plain.to_string(), "plain.toml: missing schema");
    assert_eq!(plain.line_column(), None);
    assert_eq!(plain.snippet(), None);
}

#[test]
fn toml_error_chain_renders_its_message_once() {
    let error = parse_toml("single.toml", "schema = 2\n").unwrap_err();
    let mut chain: Option<&dyn Error> = Some(&error);
    let mut messages = Vec::new();
    while let Some(current) = chain {
        messages.push(current.to_string());
        chain = current.source();
    }
    assert_eq!(messages.len(), 1);
    assert_eq!(messages.join(": ").matches("file uses schema 2").count(), 1);
    assert_eq!(error.snippet(), Some("schema = 2\n         ^"));
}

#[test]
fn range_bounds_are_inclusive_and_reject_one_past() {
    let file = parse_toml(
        "limits.toml",
        "schema = 1\n[board]\npost_port = 65535\n[[card]]\nirq = 15\ndma = 3\n",
    )
    .unwrap();
    assert_eq!(
        file.board.unwrap().post_port,
        Some(NumberOrNone::Number(65535))
    );
    let card = &file.card.unwrap()[0];
    assert_eq!(card.irq, Some(NumberOrNone::Number(15)));
    assert_eq!(card.dma, Some(NumberOrNone::Number(3)));
    for (table, key, value, position) in [
        ("[board]", "post_port", 65536, (3, 13)),
        ("[[card]]", "irq", 16, (3, 7)),
        ("[[card]]", "dma", 4, (3, 7)),
    ] {
        let error = parse_toml(
            "limits.toml",
            &format!("schema = 1\n{table}\n{key} = {value}\n"),
        )
        .unwrap_err();
        assert_eq!(error.line_column(), Some(position));
        assert!(error.message().contains("invalid value"), "{error}");
    }
}

#[test]
fn rate_parts_require_ascii_digits_without_signs() {
    for rate in ["+1/1", "1/+1", "１/1", "1/١"] {
        let error = parse_toml(
            "rate.toml",
            &format!("schema = 1\n[cpu]\nrate = \"{rate}\"\n"),
        )
        .unwrap_err();
        assert_eq!(error.line_column(), Some((3, 8)));
        assert!(error.message().contains("positive integer"));
    }
}

#[test]
fn unicode_columns_count_characters_and_keep_byte_spans() {
    let input = "schema = 1\ncard = [{ id = \"é\", port = \"bad\" }]\n";
    let error = parse_toml("unicode.toml", input).unwrap_err();
    assert_eq!(error.line_column(), Some((2, 28)));
    assert_eq!(&input[error.span().unwrap()], "\"bad\"");
    assert!(error.to_string().contains("line 2, column 28"));
    assert_eq!(
        error.snippet(),
        Some("card = [{ id = \"é\", port = \"bad\" }]\n                           ^")
    );
}

#[test]
fn drive_journal_path_and_retention_are_preserved() {
    let file = parse_toml("journal.toml", "schema = 1\n[[card]]\nkind = \"fdc-765\"\n[[card.drives]]\nunit = 1\nwrite = \"journal\"\njournal = \"disks/base.img.upfj\"\nkeep_generations = 7\n").unwrap();
    let cards = file.card.unwrap();
    let drive = &cards[0].drives.as_ref().unwrap()[0];
    assert_eq!(drive.journal.as_deref(), Some("disks/base.img.upfj"));
    assert_eq!(drive.write, Some(WriteMode::Journal));
    assert_eq!(drive.keep_generations.unwrap().get(), 7);
}

fn spellings<T: DeserializeOwned + Debug + PartialEq>(cases: &[(&str, T)]) {
    for (name, expected) in cases {
        let actual = T::deserialize(value::StrDeserializer::<value::Error>::new(name)).unwrap();
        assert_eq!(&actual, expected, "{name}");
    }
}

#[test]
fn every_implemented_enum_spelling_maps_to_its_variant() {
    spellings(&[("ibm-5170", BoardModel::Ibm5170)]);
    spellings(&[("80286", CpuModel::I80286)]);
    spellings(&[
        ("mda", CardKind::Mda),
        ("vga-text", CardKind::VgaText),
        ("uart-16550", CardKind::Uart16550),
        ("fdc-765", CardKind::Fdc765),
        ("ide", CardKind::Ide),
        ("sb16", CardKind::Sb16),
    ]);
    spellings(&[
        ("1.44M", DriveType::Hd144),
        ("1.2M", DriveType::Hd12),
        ("720K", DriveType::Dd720),
        ("360K", DriveType::Dd360),
    ]);
    spellings(&[("master", Channel::Master), ("slave", Channel::Slave)]);
    spellings(&[
        ("volatile", WriteMode::Volatile),
        ("readonly", WriteMode::ReadOnly),
        ("journal", WriteMode::Journal),
    ]);
    spellings(&[
        ("floppy", FirmwareService::Floppy),
        ("fixed-disk", FirmwareService::FixedDisk),
    ]);
    spellings(&[("derived", Nvram::Derived), ("blank", Nvram::Blank)]);
}

const IDE_EXAMPLE: &str = r#"schema = 1
name = "at-ide"
extends = "at-freedos.toml"

[firmware]
# Hard-disk INT 13h comes from an option ROM. Which XTIDE Universal BIOS
# build, and its configuration, is decided by the firmware spike.
option_roms = [
  { image = "../vendor/xtide/ide_at.bin", base = 0xC8000, provides = ["fixed-disk"] },
]

[[card]]
id = "fdc-765"                 # inherited card: only its drive changes
[[card.drives]]
unit = 0
write = "readonly"             # keep the boot floppy pristine

[[card]]
kind = "ide"                   # 0x1F0-0x1F7 + 0x3F6, IRQ 14 (type defaults)

[[card.drives]]
channel = "master"
image = "disks/dos-hd.img"
geometry = { cylinders = 615, heads = 4, sectors = 17 }
write = "journal"
journal = "disks/dos-hd.img.upfj"
"#;

#[test]
fn full_ide_example_preserves_inheritance_and_drive_declarations() {
    let file = parse_toml("at-ide.toml", IDE_EXAMPLE).unwrap();
    assert_eq!(file.name.as_deref(), Some("at-ide"));
    assert_eq!(file.extends.as_deref(), Some("at-freedos.toml"));
    assert_eq!(file.board, None);
    let roms = file.firmware.unwrap().option_roms.unwrap();
    assert_eq!(roms.len(), 1);
    assert_eq!(roms[0].image.as_deref(), Some("../vendor/xtide/ide_at.bin"));
    assert_eq!(roms[0].base, Some(0xC8000));
    assert_eq!(roms[0].provides, Some(vec![FirmwareService::FixedDisk]));
    let cards = file.card.unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].id.as_deref(), Some("fdc-765"));
    assert_eq!(cards[0].kind, None);
    let floppy = &cards[0].drives.as_ref().unwrap()[0];
    assert_eq!(floppy.unit, Some(0));
    assert_eq!(floppy.write, Some(WriteMode::ReadOnly));
    assert_eq!(floppy.image, None);
    assert_eq!(cards[1].kind, Some(CardKind::Ide));
    let disk = &cards[1].drives.as_ref().unwrap()[0];
    assert_eq!(disk.channel, Some(Channel::Master));
    assert_eq!(disk.image.as_deref(), Some("disks/dos-hd.img"));
    assert_eq!(disk.write, Some(WriteMode::Journal));
    assert_eq!(disk.journal.as_deref(), Some("disks/dos-hd.img.upfj"));
    let Geometry::Chs(chs) = disk.geometry.as_ref().unwrap();
    assert_eq!(
        (chs.cylinders, chs.heads, chs.sectors),
        (Some(615), Some(4), Some(17))
    );
}

#[test]
fn sha256_spelling_is_normalised_before_comparison() {
    let mixed = "aB12".repeat(16);
    let upper = parse_toml(
        "hash.toml",
        &format!("schema = 1\n[firmware.bios]\nsha256 = \"{mixed}\"\n"),
    )
    .unwrap()
    .firmware
    .unwrap()
    .bios
    .unwrap()
    .sha256
    .unwrap();
    let lower = parse_toml(
        "hash.toml",
        &format!(
            "schema = 1\n[firmware.bios]\nsha256 = \"{}\"\n",
            mixed.to_ascii_lowercase()
        ),
    )
    .unwrap()
    .firmware
    .unwrap()
    .bios
    .unwrap()
    .sha256
    .unwrap();
    assert_eq!(upper.as_str(), "ab12".repeat(16));
    assert_eq!(upper, lower);
}

#[test]
fn sb16_toml_and_yaml_share_resource_fields() {
    let toml = parse_toml(
        "machine.toml",
        "schema=1\n[[card]]\nkind='sb16'\nbase=0x240\nirq=7\ndma8=3\ndma16=7\n",
    )
    .unwrap();
    let yaml = machine_file::parse_yaml(
        "machine.yaml",
        "schema: 1\ncard:\n  - kind: sb16\n    base: 0x240\n    irq: 7\n    dma8: 3\n    dma16: 7\n",
    )
    .unwrap();
    assert_eq!(toml.card.as_ref().unwrap()[0].dma16, Some(7));
    assert_eq!(toml, yaml);
}
