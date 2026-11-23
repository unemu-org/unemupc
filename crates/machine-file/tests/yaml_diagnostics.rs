// SPDX-License-Identifier: AGPL-3.0-or-later
//! YAML diagnostic rendering mirrors, see the machine-files design § Errors.

use machine_file::parse_yaml;

#[test]
fn crlf_snippets_strip_the_trailing_carriage_return() {
    let error = parse_yaml("crlf.yaml", "schema: 1\r\ncpu:\r\n  rate: 0\r\n").unwrap_err();
    assert_eq!(error.line_column(), Some((3, 9)));
    assert_eq!(error.snippet(), Some("  rate: 0\n        ^"));
    assert!(!error.to_string().contains('\r'));
}

#[test]
fn snippet_padding_preserves_tabs_before_the_error() {
    let error = parse_yaml("tabs.yaml", "schema: 1\ncard: [{port:\tbad}]\n").unwrap_err();
    assert_eq!(error.line_column(), Some((2, 15)));
    assert_eq!(
        error.snippet(),
        Some("card: [{port:\tbad}]\n             \t^")
    );
}

#[test]
fn long_snippets_bound_text_and_padding_and_mark_only_cut_sides() {
    for (line, column, left_cut, right_cut, padding) in [
        (
            format!("card: [{{port: \"bad\", id: \"{}\"}}]", "y".repeat(400_000)),
            15,
            false,
            true,
            14,
        ),
        (
            format!("card: [{{id: \"{}\", port: \"bad\"}}]", "x".repeat(400_000)),
            400_023,
            true,
            false,
            43,
        ),
        (
            format!(
                "card: [{{id: \"{}\", port: \"bad\"}}] # {}",
                "é".repeat(200_000),
                "界".repeat(200_000)
            ),
            200_023,
            true,
            true,
            43,
        ),
    ] {
        let error = parse_yaml("long.yaml", &format!("schema: 1\n{line}\n")).unwrap_err();
        assert_eq!(error.line_column(), Some((2, column)), "{error}");
        let snippet = error.snippet().unwrap();
        assert!(snippet.len() < 400);
        assert!(error.to_string().len() < 1024);
        let (text, caret) = snippet.split_once('\n').unwrap();
        assert_eq!(text.starts_with("..."), left_cut);
        assert_eq!(text.ends_with("..."), right_cut);
        assert_eq!(text.chars().nth(padding), Some('"'));
        assert!(text.contains("\"bad\""));
        assert_eq!(caret, format!("{}^", " ".repeat(padding)));
    }
}

#[test]
fn every_reserved_board_model_explains_the_missing_board_support() {
    for (model, reason) in [
        (
            "ibm-5160",
            "needs the XT 8255 PPI keyboard interface and an 8088 CPU model",
        ),
        (
            "ibm-5150",
            "needs the XT 8255 PPI keyboard interface and an 8088 CPU model",
        ),
        (
            "bare",
            "file-based bare-board composition is not implemented; use Rust test rigs",
        ),
        ("compaq-deskpro", "needs a Compaq Deskpro motherboard model"),
        ("tandy-1000", "needs a Tandy 1000 motherboard model"),
        ("i440fx-piix3", "needs i440FX and PIIX3 chipset models"),
        ("i440bx-piix4", "needs i440BX and PIIX4 chipset models"),
    ] {
        let error = parse_yaml(
            "board.yaml",
            &format!("schema: 1\nboard:\n  model: {model}\n"),
        )
        .unwrap_err();
        assert_eq!(error.line_column(), Some((3, 10)));
        assert_eq!(
            error.message(),
            format!("board model `{model}` is reserved, not implemented (schema 1): {reason}")
        );
        assert_eq!(error.message().matches("is reserved").count(), 1);
    }
}

#[test]
fn yaml_error_chain_renders_its_message_once() {
    use std::error::Error;
    let error = parse_yaml("single.yaml", "schema: 2\n").unwrap_err();
    assert!(error.source().is_none());
    assert_eq!(error.to_string().matches("file uses schema 2").count(), 1);
    assert_eq!(error.snippet(), Some("schema: 2\n        ^"));
}

fn exact(input: &str, position: (usize, usize), message: &str) {
    let error = parse_yaml("author.yaml", input).unwrap_err();
    assert_eq!(error.message(), message, "{error}");
    assert_eq!(error.line_column(), Some(position), "{error}");
    assert!(error.to_string().starts_with(&format!(
        "author.yaml: line {}, column {}: ",
        position.0, position.1
    )));
}

#[test]
fn duplicate_keys_explain_the_document_problem() {
    exact(
        "schema: 1\nname: first\nname: second\n",
        (3, 1),
        "duplicate mapping key `name`",
    );
}
#[test]
fn nulls_explain_how_to_omit_an_optional_field() {
    exact(
        "schema: 1\nname: ~\n",
        (2, 7),
        "null is not allowed; omit the field instead",
    );
}
#[test]
fn recursive_aliases_explain_the_unsupported_document_shape() {
    exact(
        "schema: 1\ncard: &cards [*cards]\n",
        (2, 15),
        "recursive aliases are not allowed",
    );
}
#[test]
fn multiple_documents_explain_the_file_contract() {
    exact(
        "schema: 1\n---\nschema: 1\n",
        (2, 1),
        "exactly one YAML document is required",
    );
}
#[test]
fn an_integer_field_requires_an_integer_instead_of_a_sequence() {
    exact(
        "schema: 1\ncard: [{port: [1]}]\n",
        (2, 15),
        "invalid type: sequence, expected u16",
    );
}

#[test]
fn syntax_errors_render_the_position_once() {
    for (input, position, message) in [
        ("schema: 1\ncard: [\n", (2, 7), "unclosed bracket '['"),
        (
            "schema: 1\nboard:\n\tmodel: ibm-5170\n",
            (3, 2),
            "tabs disallowed within this context (block indentation)",
        ),
        (
            "schema: 1\nname: a: b\n",
            (2, 8),
            "mapping values are not allowed in this context",
        ),
        (
            "schema: 1\nname: *unknown\n",
            (2, 7),
            "while parsing node, found unknown anchor",
        ),
    ] {
        exact(input, position, message);
        let error = parse_yaml("syntax.yaml", input).unwrap_err();
        assert_eq!(error.to_string().matches("line ").count(), 1, "{error}");
        assert_eq!(error.to_string().matches("column ").count(), 1, "{error}");
    }
}

#[test]
fn wrong_shapes_match_the_toml_parser_messages_exactly() {
    for (toml, yaml, position, expected) in [
        (
            "schema=1\nname=[1]\n",
            "schema: 1\nname: [1]\n",
            (2, 7),
            "invalid type: sequence, expected a string",
        ),
        (
            "schema=1\n[[card]]\nkind=['mda']\n",
            "schema: 1\ncard: [{kind: [mda]}]\n",
            (2, 15),
            "invalid type: sequence, expected a card kind name",
        ),
        (
            "schema=1\n[firmware.bios]\nprovides='floppy'\n",
            "schema: 1\nfirmware: {bios: {provides: floppy}}\n",
            (2, 29),
            "invalid type: string \"floppy\", expected a sequence",
        ),
        (
            "schema=1\ncard='mda'\n",
            "schema: 1\ncard: mda\n",
            (2, 7),
            "invalid type: string \"mda\", expected a sequence",
        ),
        (
            "schema=1\n[[card]]\ndrives={unit=0}\n",
            "schema: 1\ncard: [{drives: {unit: 0}}]\n",
            (2, 17),
            "invalid type: map, expected a sequence",
        ),
        (
            "schema=1\nboard='x'\n",
            "schema: 1\nboard: x\n",
            (2, 8),
            "invalid type: string \"x\", expected struct Board",
        ),
        (
            "schema=1\nfirmware=[1]\n",
            "schema: 1\nfirmware: [1]\n",
            (2, 12),
            "invalid type: integer `1`, expected struct Rom",
        ),
        ("", "", (1, 1), "missing field `schema`"),
    ] {
        assert_eq!(
            machine_file::parse_toml("shape.toml", toml)
                .unwrap_err()
                .message(),
            expected
        );
        exact(yaml, position, expected);
    }
}

#[test]
fn integer_lexical_errors_have_value_positions_and_author_messages() {
    for (literal, message) in [
        ("01016", "unexpected leading zero, expected nothing"),
        ("99999999999999999999999", "integer is out of range"),
        ("0X3F8", "radix must be lowercase, expected `0x`"),
        ("0O77", "radix must be lowercase, expected `0o`"),
        ("0B11", "radix must be lowercase, expected `0b`"),
    ] {
        exact(
            &format!("schema: 1\ncard: [{{port: {literal}}}]\n"),
            (2, 15),
            message,
        );
    }
    for literal in ["01016", "99999999999999999999999"] {
        let error = parse_yaml(
            "irq.yaml",
            &format!("schema: 1\ncard: [{{irq: {literal}}}]\n"),
        )
        .unwrap_err();
        assert_eq!(error.line_column(), Some((2, 14)));
        assert_eq!(
            error.message(),
            if literal == "01016" {
                "unexpected leading zero, expected nothing"
            } else {
                "integer is out of range"
            }
        );
    }
}

#[test]
fn missing_explicit_values_point_at_the_key_and_maps_at_the_node_start() {
    exact(
        "schema: 1\ncard:\n  - ? irq\n",
        (3, 7),
        "null is not allowed; omit the field instead",
    );
    exact(
        "schema: 1\ncard:\n  - port:\n      a: 1\n",
        (4, 7),
        "invalid type: map, expected u16",
    );
    exact(
        "schema: 1\ncard:\n  - irq:\n      a: 1\n",
        (4, 7),
        "invalid type: map, expected an integer in 0..=15 or \"none\"",
    );
}

#[test]
fn alias_expansion_and_replay_limits_name_the_written_anchor_and_limit() {
    let expansion = format!(
        "schema: 1\ncard:\n  - &card {{kind: mda}}\n{}",
        "  - *card\n".repeat(17)
    );
    let error = parse_yaml("aliases.yaml", &expansion).unwrap_err();
    assert_eq!(
        error.message(),
        "anchor `card`: alias expansion limit of 16 exceeded"
    );
    assert!(error.line_column().is_some());

    let replay = format!(
        "schema: 1\nfuture:\n  - &data [{}]\n  - *data\n  - *data\n",
        vec!["0"; 1100].join(", ")
    );
    let error = parse_yaml("aliases.yaml", &replay).unwrap_err();
    assert_eq!(
        error.message(),
        "alias `*data`: replay event limit of 2000 exceeded"
    );
    assert!(error.line_column().is_some());
}
