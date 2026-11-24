// SPDX-License-Identifier: AGPL-3.0-or-later
//! YAML mirrors of the schema parsing contracts, using the shared serde model.

use machine_file::{parse_yaml, schema::v1::*, ParseError};
use std::{error::Error, path::Path};

const FILE: &str = "machines/test.yaml";
const SECOND_CARD: &str = "schema: 1\n\ncard:\n  - kind: mda\n\n  - kind: uart-16550\n";

fn positioned(input: &str, position: (usize, usize), message: &str) -> ParseError {
    let error = parse_yaml(FILE, input).unwrap_err();
    assert_eq!(error.path(), Path::new(FILE));
    assert_eq!(error.line_column(), Some(position), "{error}");
    assert!(error.message().contains(message), "{error}");
    assert!(error.to_string().starts_with(&format!("{FILE}: ")));
    assert!(error.source().is_none());
    assert!(error.snippet().unwrap().contains('^'));
    error
}

#[test]
fn unknown_field_points_to_second_card_key() {
    positioned(
        &format!("{SECOND_CARD}    irqq: 4\n"),
        (7, 5),
        "unknown field",
    );
}
#[test]
fn wrong_type_points_to_second_card_value() {
    positioned(&format!("{SECOND_CARD}    port: bad\n"), (7, 11), "u16");
}
#[test]
fn unknown_kind_points_to_second_card_value() {
    positioned(
        "schema: 1\ncard:\n  - kind: mda\n  - kind: mystery\n",
        (4, 5),
        "unknown variant",
    );
}
#[test]
fn reserved_kind_has_positioned_implementation_message() {
    positioned(
        "schema: 1\ncard:\n  - kind: mda\n  - kind: voodoo\n",
        (4, 11),
        "card kind `voodoo` is reserved, not implemented",
    );
}
#[test]
fn cycle_exact_value_explains_instruction_timing() {
    let error = positioned(
        "schema: 1\ncpu:\n  mhz: 8\n",
        (3, 8),
        "cycle-exact timing is not modelled",
    );
    assert!(error
        .message()
        .contains("`[cpu] rate` is a ratio of completed steps per PIT input clock"));
}
#[test]
fn newer_schema_is_rejected_before_interpreting_fields() {
    positioned(
        "future: true\nschema: 2\n",
        (2, 9),
        "file uses schema 2; this build reads schema 1",
    );
}
#[test]
fn missing_schema_is_a_positioned_error() {
    positioned("name: missing-version\n", (1, 1), "missing field");
}
#[test]
fn run_table_explains_where_run_policy_belongs() {
    let error = positioned(
        "schema: 1\nrun:\n  max_steps: 100\n",
        (3, 3),
        "belongs outside the machine file",
    );
    assert!(error.message().contains("run flags for run policy"));
}

#[test]
fn freedos_example_preserves_hardware_order_and_explicit_values() {
    let yaml = parse_yaml(FILE, include_str!("../../../machines/at-freedos.yaml")).unwrap();
    let toml = machine_file::parse_toml(
        "machines/at-freedos.toml",
        include_str!("../../../machines/at-freedos.toml"),
    )
    .unwrap();
    assert_eq!(yaml, toml);
}

#[test]
fn two_form_values_preserve_explicit_overrides() {
    let file = parse_yaml(
        FILE,
        r#"schema: 1
board:
  post_port: none
  rtc:
    nvram: {image: cmos.bin, write: journal, journal: cmos.upfj, keep_generations: 10}
cpu: {rate: 3}
card:
  - {kind: uart-16550, irq: 4, dma: none}
  - {kind: fdc-765, dma: 2}
  - kind: ide
    drives:
      - channel: slave
        geometry: {cylinders: 615, heads: 4, sectors: 17}
        write: readonly
firmware:
  option_roms: [{image: ide.bin, base: 0xC8000, provides: [fixed-disk]}]
"#,
    )
    .unwrap();
    let board = file.board.unwrap();
    assert_eq!(board.post_port, Some(NumberOrNone::None));
    let Nvram::Image(image) = board.rtc.unwrap().nvram.unwrap() else {
        panic!("expected image")
    };
    assert_eq!(image.image.as_deref(), Some("cmos.bin"));
    assert_eq!(image.write, Some(WriteMode::Journal));
    assert_eq!(image.journal.as_deref(), Some("cmos.upfj"));
    assert_eq!(image.keep_generations.unwrap().get(), 10);
    let rate = file.cpu.unwrap().rate.unwrap();
    assert_eq!((rate.steps.get(), rate.pit_clocks.get()), (3, 1));
    let cards = file.card.unwrap();
    assert_eq!(cards[0].irq, Some(NumberOrNone::Number(4)));
    assert_eq!(cards[0].dma, Some(NumberOrNone::None));
    assert_eq!(cards[1].dma, Some(NumberOrNone::Number(2)));
    let drive = &cards[2].drives.as_ref().unwrap()[0];
    assert_eq!(drive.channel, Some(Channel::Slave));
    let Geometry::Chs(chs) = drive.geometry.as_ref().unwrap();
    assert_eq!(
        (chs.cylinders, chs.heads, chs.sectors),
        (Some(615), Some(4), Some(17))
    );
    assert_eq!(drive.write, Some(WriteMode::ReadOnly));
    let rom = &file.firmware.unwrap().option_roms.unwrap()[0];
    assert_eq!(rom.base, Some(0xC8000));
    assert_eq!(rom.provides, Some(vec![FirmwareService::FixedDisk]));
    let blank = parse_yaml(FILE, "schema: 1\nboard: {rtc: {nvram: blank}}\n").unwrap();
    assert_eq!(blank.board.unwrap().rtc.unwrap().nvram, Some(Nvram::Blank));
    let ratio = parse_yaml(FILE, "schema: 1\ncpu: {rate: 7/3}\n")
        .unwrap()
        .cpu
        .unwrap()
        .rate
        .unwrap();
    assert_eq!((ratio.steps.get(), ratio.pit_clocks.get()), (7, 3));
}

#[test]
fn partial_layers_keep_absence_and_explicit_empty_lists() {
    let empty = parse_yaml(FILE, "schema: 1\n").unwrap();
    assert!(empty.board.is_none() && empty.firmware.is_none() && empty.card.is_none());
    let layer = parse_yaml(FILE, "schema: 1\nextends: preset:ibm-5170\ncard: []\nfirmware: {option_roms: [], bios: {provides: []}}\n").unwrap();
    assert_eq!(layer.extends.as_deref(), Some("preset:ibm-5170"));
    assert_eq!(layer.card, Some(vec![]));
    let firmware = layer.firmware.unwrap();
    assert_eq!(firmware.option_roms, Some(vec![]));
    assert_eq!(firmware.bios.unwrap().provides, Some(vec![]));
    let remove = parse_yaml(FILE, "schema: 1\nextends: parent.toml\ncard: [{id: fdc-765, remove: true, drives: [{unit: 0, remove: true}]}]\n").unwrap();
    let card = &remove.card.unwrap()[0];
    assert_eq!(card.kind, None);
    assert_eq!(card.remove, Some(true));
    assert_eq!(card.drives.as_ref().unwrap()[0].remove, Some(true));
}

// Flow mappings let the same scalar contract run at every nesting depth.
fn layer(table: &str, key: &str, value: &str) -> String {
    let mut body = format!("{{{key}: {value}}}");
    for part in table.split('.').rev().filter(|part| !part.is_empty()) {
        body = if ["card", "drives", "option_roms"].contains(&part) {
            format!("{{{part}: [{body}]}}")
        } else {
            format!("{{{part}: {body}}}")
        };
    }
    format!("schema: 1\n{}\n", &body[1..body.len() - 1])
}

fn rejected(table: &str, key: &str, value: &str, message: &str) {
    let input = layer(table, key, value);
    let error = parse_yaml(FILE, &input).unwrap_err();
    assert_eq!(error.path(), Path::new(FILE));
    assert_eq!(error.line_column().unwrap().0, 2, "{error}");
    assert!(error.message().contains(message), "{error}");
    assert!(error.snippet().unwrap().contains('^'));
}

#[test]
fn all_cycle_exact_fields_are_explained_in_each_timing_table() {
    for table in ["", "board", "cpu", "card"] {
        for key in [
            "mhz",
            "frequency",
            "clock_hz",
            "cycles",
            "cycle_exact",
            "wait_states",
            "bus_mhz",
            "memory_wait_states",
            "timing",
        ] {
            rejected(
                table,
                key,
                "{requested: true}",
                "cycle-exact timing is not modelled",
            );
        }
    }
}

#[test]
fn all_reserved_model_and_kind_names_have_specific_errors() {
    for (table, key, values) in [
        (
            "board",
            "model",
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
            "cpu",
            "model",
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
            "card",
            "kind",
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
            rejected(
                table,
                key,
                &format!("\"{value}\""),
                "reserved, not implemented",
            );
            let error = parse_yaml(FILE, &layer(table, key, &format!("\"{value}\""))).unwrap_err();
            assert!(error.to_string().contains(value));
        }
    }
}

#[test]
fn reserved_fields_values_and_outside_options_are_recognised() {
    for (table, key, value) in [
        ("board", "a20", "\"kbc\""),
        ("board", "kbc", "\"at\""),
        ("board", "keyboard", "\"xt-83\""),
        ("board", "keyboard_layout", "\"us\""),
        ("board", "dip_switches", "[]"),
        ("board", "chipset", "{}"),
        ("board", "speaker", "true"),
        ("memory", "shadow", "true"),
        ("memory", "umb", "true"),
        ("memory", "upper", "\"ram\""),
        ("firmware", "basic", "{}"),
        ("card", "monitor", "\"green\""),
        ("card", "rom", "{}"),
        ("card", "mmio", "0xB0000"),
        ("card.drives", "type", "\"2.88M\""),
        ("card.drives", "type", "\"atapi-cdrom\""),
        ("card.drives", "image", "\"blank\""),
        ("card.drives", "geometry", "\"auto\""),
    ] {
        rejected(table, key, value, "reserved, not implemented");
    }
    for service in ["video", "serial", "network-boot"] {
        rejected(
            "firmware.bios",
            "provides",
            &format!("[{service}]"),
            "reserved, not implemented",
        );
    }
    for option in ["presentation", "streams"] {
        rejected("", option, "{}", "belongs outside the machine file");
    }
    rejected("board.rtc", "start", "host", "--rtc-start host");
}

#[test]
fn nested_tables_reject_unknown_fields_at_their_keys() {
    for table in [
        "board",
        "board.rtc",
        "cpu",
        "memory",
        "firmware",
        "firmware.bios",
        "firmware.option_roms",
        "card",
        "card.drives",
        "board.rtc.nvram",
        "card.drives.geometry",
    ] {
        rejected(table, "unknown", "true", "unknown field");
    }
}

#[test]
fn malformed_two_form_values_retain_the_value_position() {
    for (table, key, values) in [
        (
            "board",
            "post_port",
            &["-1", "65536", "\"off\"", "true"][..],
        ),
        ("card", "irq", &["-1", "16", "\"four\"", "4.0"][..]),
        ("card", "dma", &["-1", "4", "\"two\""][..]),
        (
            "cpu",
            "rate",
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
        ("board.rtc", "nvram", &["\"seeded\"", "false", "64"][..]),
    ] {
        for value in values {
            rejected(table, key, value, "expected");
        }
    }
}

#[test]
fn hashes_dates_and_journal_counts_keep_syntax_strict() {
    for value in ["short".to_owned(), "g".repeat(64)] {
        rejected(
            "firmware.bios",
            "sha256",
            &format!("\"{value}\""),
            "64 hexadecimal characters",
        );
    }
    rejected("board.rtc", "start", "2000-01-01T00:00:00", "quoted string");
    rejected("card.drives", "keep_generations", "0", "nonzero u32");
    positioned("schema: \"1\"\n", (1, 9), "i64");
    positioned("schema: 0\n", (1, 9), "file uses schema 0");
}
