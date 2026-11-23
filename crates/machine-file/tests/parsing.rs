// SPDX-License-Identifier: AGPL-3.0-or-later
//! Syntax and diagnostic contracts from `backlog/docs/design/machine-files.md`.

use std::{error::Error, path::Path};

use machine_file::{
    parse_toml,
    schema::v1::{
        BoardModel, CardKind, Channel, CpuModel, DriveType, FirmwareService, Geometry,
        NumberOrNone, Nvram, WriteMode,
    },
    ParseError,
};

const FILE: &str = "machines/test.toml";

fn positioned(input: &str, expected_position: (usize, usize), message: &str) -> ParseError {
    let error = parse_toml(FILE, input).expect_err("invalid input must be rejected");
    assert_eq!(error.path(), Path::new(FILE));
    assert_eq!(error.line_column(), Some(expected_position), "{error}");
    let rendered = error.to_string();
    assert!(rendered.starts_with(&format!("{FILE}: ")), "{rendered}");
    assert!(rendered.contains(message), "{rendered}");
    assert!(error.source().is_none());
    assert!(error.span().is_some());
    error
}

const SECOND_CARD: &str = "schema = 1\n\n[[card]]\nkind = \"mda\"\n\n[[card]]\n";

#[test]
fn unknown_field_points_to_second_card_key() {
    positioned(
        &format!("{SECOND_CARD}kind = \"uart-16550\"\nirqq = 4\n"),
        (8, 1),
        "unknown field `irqq`",
    );
}

#[test]
fn wrong_type_points_to_second_card_value() {
    let input = format!("{SECOND_CARD}kind = \"uart-16550\"\nport = \"bad\"\n");
    let error = positioned(&input, (8, 8), "invalid type: string");
    assert!(error.to_string().contains("expected u16"));
    assert_eq!(&input[error.span().unwrap()], "\"bad\"");
}

#[test]
fn unknown_kind_points_to_second_card_value() {
    positioned(
        &format!("{SECOND_CARD}kind = \"mystery\"\n"),
        (7, 8),
        "unknown variant `mystery`",
    );
}

#[test]
fn reserved_kind_has_positioned_implementation_message() {
    positioned(
        &format!("{SECOND_CARD}kind = \"voodoo\"\n"),
        (7, 8),
        "card kind `voodoo` is reserved, not implemented",
    );
}

#[test]
fn cycle_exact_value_explains_instruction_timing() {
    let error = positioned(
        "schema = 1\n\n[cpu]\nmhz = 8\n",
        (4, 7),
        "cycle-exact timing is not modelled",
    );
    assert!(error
        .to_string()
        .contains("`[cpu] rate` is a ratio of completed steps per PIT input clock"));
}

#[test]
fn newer_schema_is_rejected_before_interpreting_fields() {
    positioned(
        "future = true\nschema = 2\n",
        (2, 10),
        "file uses schema 2; this build reads schema 1",
    );
}

#[test]
fn missing_schema_is_a_positioned_error() {
    positioned(
        "name = \"missing-version\"\n",
        (1, 1),
        "missing field `schema`",
    );
}

#[test]
fn run_table_explains_where_run_policy_belongs() {
    let error = positioned(
        "schema = 1\n\n[run]\nmax_steps = 100\n",
        (3, 1),
        "belongs outside the machine file",
    );
    assert!(error.to_string().contains("run flags for run policy"));
}

const FREEDOS: &str = r#"schema = 1
name = "at-freedos"
description = "PC/AT-class machine booting FreeDOS from drive A: with the Xi8088 open BIOS"

[board]
model = "ibm-5170"
post_port = 0x80

[board.rtc]
start = "2000-01-01T00:00:00"
nvram = "derived"

[cpu]
model = "80286"
rate = "1/1"

[memory]
conventional_kib = 640
extended_kib = 0

[firmware.bios]
image = "../vendor/kiselev-bios/bios-xi8088.bin"
sha256 = "1ad458acb1f22dee6b3d9b6bd95e83218a7ddc5389fa2b2c67a4001e689c81a8"
provides = ["floppy"]

[[card]]
kind = "mda"          # the Xi8088 BIOS console; first display card = window default

[[card]]
kind = "vga-text"

[[card]]
kind = "uart-16550"
port = 0x3F8
irq = "none"          # the reference machine leaves COM1 unwired

[[card]]
kind = "fdc-765"

[[card.drives]]
unit = 0
type = "1.44M"
image = "../vendor/freedos/freedos.img"
sha256 = "b7b088333f7f30584d768941fecea8288b63f2d61e13ec6c9472439eda66e51f"
write = "volatile"
"#;

#[test]
fn freedos_example_preserves_hardware_order_and_explicit_values() {
    let file = parse_toml("machines/at-freedos.toml", FREEDOS).unwrap();
    assert_eq!(file.name.as_deref(), Some("at-freedos"));
    assert!(file.description.unwrap().contains("PC/AT-class"));
    let board = file.board.unwrap();
    assert_eq!(board.model, Some(BoardModel::Ibm5170));
    assert_eq!(board.post_port, Some(NumberOrNone::Number(0x80)));
    let rtc = board.rtc.unwrap();
    assert_eq!(rtc.start.as_deref(), Some("2000-01-01T00:00:00"));
    assert_eq!(rtc.nvram, Some(Nvram::Derived));
    let cpu = file.cpu.unwrap();
    assert_eq!(cpu.model, Some(CpuModel::I80286));
    let rate = cpu.rate.unwrap();
    assert_eq!((rate.steps.get(), rate.pit_clocks.get()), (1, 1));
    let memory = file.memory.unwrap();
    assert_eq!(
        (memory.conventional_kib, memory.extended_kib),
        (Some(640), Some(0))
    );
    let bios = file.firmware.unwrap().bios.unwrap();
    assert_eq!(
        bios.image.as_deref(),
        Some("../vendor/kiselev-bios/bios-xi8088.bin")
    );
    assert_eq!(
        bios.sha256.unwrap().as_str(),
        "1ad458acb1f22dee6b3d9b6bd95e83218a7ddc5389fa2b2c67a4001e689c81a8"
    );
    assert_eq!(bios.provides, Some(vec![FirmwareService::Floppy]));
    assert_eq!(bios.base, None);
    let cards = file.card.unwrap();
    assert_eq!(
        cards.iter().map(|card| card.kind).collect::<Vec<_>>(),
        vec![
            Some(CardKind::Mda),
            Some(CardKind::VgaText),
            Some(CardKind::Uart16550),
            Some(CardKind::Fdc765),
        ]
    );
    assert_eq!(cards[2].port, Some(0x3F8));
    assert_eq!(cards[2].irq, Some(NumberOrNone::None));
    assert_eq!(cards[3].irq, None);
    let drives = cards[3].drives.as_ref().unwrap();
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0].unit, Some(0));
    assert_eq!(drives[0].drive_type, Some(DriveType::Hd144));
    assert_eq!(
        drives[0].image.as_deref(),
        Some("../vendor/freedos/freedos.img")
    );
    assert_eq!(
        drives[0].sha256.as_ref().unwrap().as_str(),
        "b7b088333f7f30584d768941fecea8288b63f2d61e13ec6c9472439eda66e51f"
    );
    assert_eq!(drives[0].write, Some(WriteMode::Volatile));
}

#[test]
fn two_form_values_preserve_explicit_overrides() {
    let file = parse_toml(
        FILE,
        r#"schema = 1
[board]
post_port = "none"
[board.rtc]
nvram = { image = "cmos.bin", write = "journal", journal = "cmos.upfj", keep_generations = 10 }
[cpu]
rate = 3
[[card]]
kind = "uart-16550"
irq = 4
dma = "none"
[[card]]
kind = "fdc-765"
dma = 2
[[card]]
kind = "ide"
[[card.drives]]
channel = "slave"
geometry = { cylinders = 615, heads = 4, sectors = 17 }
write = "readonly"
[firmware]
option_roms = [{ image = "ide.bin", base = 0xC8000, provides = ["fixed-disk"] }]
"#,
    )
    .unwrap();
    let board = file.board.unwrap();
    assert_eq!(board.post_port, Some(NumberOrNone::None));
    match board.rtc.unwrap().nvram.unwrap() {
        Nvram::Image(image) => {
            assert_eq!(image.image.as_deref(), Some("cmos.bin"));
            assert_eq!(image.write, Some(WriteMode::Journal));
            assert_eq!(image.journal.as_deref(), Some("cmos.upfj"));
            assert_eq!(image.keep_generations.unwrap().get(), 10);
        }
        other => panic!("expected image, got {other:?}"),
    }
    let rate = file.cpu.unwrap().rate.unwrap();
    assert_eq!((rate.steps.get(), rate.pit_clocks.get()), (3, 1));
    let cards = file.card.unwrap();
    assert_eq!(cards[0].irq, Some(NumberOrNone::Number(4)));
    assert_eq!(cards[0].dma, Some(NumberOrNone::None));
    assert_eq!(cards[1].dma, Some(NumberOrNone::Number(2)));
    let drive = &cards[2].drives.as_ref().unwrap()[0];
    assert_eq!(drive.channel, Some(Channel::Slave));
    let Geometry::Chs(geometry) = drive.geometry.as_ref().unwrap();
    assert_eq!(
        (geometry.cylinders, geometry.heads, geometry.sectors),
        (Some(615), Some(4), Some(17))
    );
    assert_eq!(drive.write, Some(WriteMode::ReadOnly));
    let rom = &file.firmware.unwrap().option_roms.unwrap()[0];
    assert_eq!(rom.base, Some(0xC8000));
    assert_eq!(rom.provides, Some(vec![FirmwareService::FixedDisk]));
    let blank = parse_toml(FILE, "schema = 1\n[board.rtc]\nnvram = \"blank\"\n").unwrap();
    assert_eq!(blank.board.unwrap().rtc.unwrap().nvram, Some(Nvram::Blank));
    let ratio = parse_toml(FILE, "schema = 1\n[cpu]\nrate = \"7/3\"\n")
        .unwrap()
        .cpu
        .unwrap()
        .rate
        .unwrap();
    assert_eq!((ratio.steps.get(), ratio.pit_clocks.get()), (7, 3));
}

#[test]
fn partial_layers_keep_absence_and_explicit_empty_lists() {
    let empty = parse_toml(FILE, "schema = 1\n").unwrap();
    assert!(empty.board.is_none() && empty.firmware.is_none() && empty.card.is_none());
    let layer = parse_toml(
        FILE,
        r#"schema = 1
extends = "preset:ibm-5170"
card = []
[firmware]
option_roms = []
[firmware.bios]
provides = []
"#,
    )
    .unwrap();
    assert_eq!(layer.extends.as_deref(), Some("preset:ibm-5170"));
    assert_eq!(layer.card, Some(vec![]));
    let firmware = layer.firmware.unwrap();
    assert_eq!(firmware.option_roms, Some(vec![]));
    assert_eq!(firmware.bios.unwrap().provides, Some(vec![]));
    let remove = parse_toml(FILE, "schema = 1\nextends = \"parent.toml\"\n[[card]]\nid = \"fdc-765\"\nremove = true\n[[card.drives]]\nunit = 0\nremove = true\n").unwrap();
    let card = &remove.card.unwrap()[0];
    assert_eq!(card.kind, None);
    assert_eq!(card.remove, Some(true));
    assert_eq!(card.drives.as_ref().unwrap()[0].remove, Some(true));
}

#[test]
fn all_cycle_exact_fields_are_explained_in_each_timing_table() {
    for (table, line) in [("", 2), ("[board]\n", 3), ("[cpu]\n", 3), ("[[card]]\n", 3)] {
        for (key, column) in [
            ("mhz", 7),
            ("frequency", 13),
            ("clock_hz", 12),
            ("cycles", 10),
            ("cycle_exact", 15),
            ("wait_states", 15),
            ("bus_mhz", 11),
            ("memory_wait_states", 22),
            ("timing", 10),
        ] {
            positioned(
                &format!("schema = 1\n{table}{key} = {{ requested = true }}\n"),
                (line, column),
                "cycle-exact timing is not modelled",
            );
        }
    }
}

#[test]
fn all_reserved_model_and_kind_names_have_specific_errors() {
    for (table, key, position, values) in [
        (
            "[board]",
            "model",
            (3, 9),
            &[
                "ibm-5160",
                "ibm-5150",
                "bare",
                "compaq-deskpro",
                "tandy-1000",
                "i440fx-piix3",
                "i440bx-piix4",
            ][..],
        ),
        (
            "[cpu]",
            "model",
            (3, 9),
            &[
                "8086",
                "8088",
                "v20",
                "80386sx",
                "80486",
                "pentium",
                "pentium-ii",
                "pentium-iii",
            ][..],
        ),
        (
            "[[card]]",
            "kind",
            (3, 8),
            &[
                "hercules",
                "cga",
                "ega",
                "svga-et4000",
                "svga-s3-trio64",
                "voodoo",
                "voodoo2",
                "banshee",
                "adlib",
                "sb1",
                "sb2",
                "sbpro",
                "gus",
                "mpu-401",
                "covox",
                "xt-mfm",
                "xt-rll",
                "xt-ide",
                "xt-cf-lite",
                "lpt",
                "game-port",
                "bus-mouse",
                "ems",
                "ne2000",
            ][..],
        ),
    ] {
        for value in values {
            let error = positioned(
                &format!("schema = 1\n{table}\n{key} = \"{value}\"\n"),
                position,
                "reserved, not implemented",
            );
            assert!(error.to_string().contains(value));
        }
    }
}

#[test]
fn reserved_fields_values_and_outside_options_are_recognised() {
    for (table, key, value, position) in [
        ("[board]", "a20", "\"kbc\"", (3, 7)),
        ("[board]", "kbc", "\"at\"", (3, 7)),
        ("[board]", "keyboard", "\"xt-83\"", (3, 12)),
        ("[board]", "keyboard_layout", "\"us\"", (3, 19)),
        ("[board]", "dip_switches", "[]", (3, 16)),
        ("[board]", "chipset", "{}", (3, 11)),
        ("[board]", "speaker", "true", (3, 11)),
        ("[cpu]", "turbo", "{}", (3, 9)),
        ("[memory]", "shadow", "true", (3, 10)),
        ("[memory]", "umb", "true", (3, 7)),
        ("[memory]", "upper", "\"ram\"", (3, 9)),
        ("[firmware]", "basic", "{}", (3, 9)),
        ("[[card]]", "monitor", "\"green\"", (3, 11)),
        ("[[card]]", "rom", "{}", (3, 7)),
        ("[[card]]", "mmio", "0xB0000", (3, 8)),
        ("[[card]]\n[[card.drives]]", "type", "\"2.88M\"", (4, 8)),
        (
            "[[card]]\n[[card.drives]]",
            "type",
            "\"atapi-cdrom\"",
            (4, 8),
        ),
        ("[[card]]\n[[card.drives]]", "image", "\"blank\"", (4, 9)),
        ("[[card]]\n[[card.drives]]", "geometry", "\"auto\"", (4, 12)),
    ] {
        positioned(
            &format!("schema = 1\n{table}\n{key} = {value}\n"),
            position,
            "reserved, not implemented",
        );
    }
    for service in ["video", "serial", "network-boot"] {
        positioned(
            &format!("schema = 1\n[firmware.bios]\nprovides = [\"{service}\"]\n"),
            (3, 13),
            "reserved, not implemented",
        );
    }
    for option in ["presentation", "streams"] {
        positioned(
            &format!("schema = 1\n[{option}]\n"),
            (2, 1),
            "belongs outside the machine file",
        );
    }
    positioned(
        "schema = 1\n[board.rtc]\nstart = \"host\"\n",
        (3, 9),
        "--rtc-start host",
    );
}

#[test]
fn nested_tables_reject_unknown_fields_at_their_keys() {
    for (table, position) in [
        ("[board]", (3, 1)),
        ("[board.rtc]", (3, 1)),
        ("[cpu]", (3, 1)),
        ("[memory]", (3, 1)),
        ("[firmware]", (3, 1)),
        ("[firmware.bios]", (3, 1)),
        ("[[firmware.option_roms]]", (3, 1)),
        ("[[card]]", (3, 1)),
        ("[[card]]\n[[card.drives]]", (4, 1)),
        ("[board.rtc.nvram]", (3, 1)),
        ("[[card]]\n[[card.drives]]\n[card.drives.geometry]", (5, 1)),
    ] {
        positioned(
            &format!("schema = 1\n{table}\nunknown = true\n"),
            position,
            "unknown field `unknown`",
        );
    }
}

#[test]
fn malformed_two_form_values_retain_the_value_position() {
    for (table, key, position, values) in [
        (
            "[board]",
            "post_port",
            (3, 13),
            &["-1", "65536", "\"off\"", "true"][..],
        ),
        (
            "[[card]]",
            "irq",
            (3, 7),
            &["-1", "16", "\"four\"", "4.0"][..],
        ),
        ("[[card]]", "dma", (3, 7), &["-1", "4", "\"two\""][..]),
        (
            "[cpu]",
            "rate",
            (3, 8),
            &[
                "0",
                "-1",
                "\"0/1\"",
                "\"1/0\"",
                "\"1/2/3\"",
                "\"2\"",
                "1.5",
                "\"18446744073709551616/1\"",
            ][..],
        ),
        (
            "[board.rtc]",
            "nvram",
            (3, 9),
            &["\"seeded\"", "false", "64"][..],
        ),
    ] {
        for value in values {
            positioned(
                &format!("schema = 1\n{table}\n{key} = {value}\n"),
                position,
                "expected",
            );
        }
    }
}

#[test]
fn hashes_dates_and_journal_counts_keep_syntax_strict() {
    for value in ["short".to_owned(), "g".repeat(64)] {
        positioned(
            &format!("schema = 1\n[firmware.bios]\nsha256 = \"{value}\"\n"),
            (3, 10),
            "64 hexadecimal characters",
        );
    }
    positioned(
        "schema = 1\n[board.rtc]\nstart = 2000-01-01T00:00:00\n",
        (3, 9),
        "expected a string",
    );
    positioned(
        "schema = 1\n[[card]]\n[[card.drives]]\nkeep_generations = 0\n",
        (4, 20),
        "nonzero u32",
    );
    positioned("schema = \"1\"\n", (1, 10), "expected i64");
    positioned("schema = 0\n", (1, 10), "file uses schema 0");
}
