// SPDX-License-Identifier: AGPL-3.0-or-later
//! Diagnostic rendering contracts from `backlog/docs/design/machine-files.md` § Errors.

use machine_file::parse_toml;

#[test]
fn crlf_snippets_strip_the_trailing_carriage_return() {
    let error = parse_toml("crlf.toml", "schema = 1\r\n[cpu]\r\nrate = 0\r\n").unwrap_err();
    assert_eq!(error.line_column(), Some((3, 8)));
    assert_eq!(error.snippet(), Some("rate = 0\n       ^"));
    assert!(!error.to_string().contains('\r'));
}

#[test]
fn snippet_padding_preserves_tabs_before_the_error() {
    let error = parse_toml("tabs.toml", "schema = 1\n[cpu]\n\trate =\t0\n").unwrap_err();
    assert_eq!(error.line_column(), Some((3, 9)));
    assert_eq!(error.snippet(), Some("\trate =\t0\n\t      \t^"));
}

#[test]
fn long_snippets_bound_text_and_padding_and_mark_only_cut_sides() {
    for (line, column, left_cut, right_cut, padding) in [
        (
            format!(
                "card = [{{ port = \"bad\", id = \"{}\" }}]",
                "y".repeat(400_000)
            ),
            18,
            false,
            true,
            17,
        ),
        (
            format!(
                "card = [{{ id = \"{}\", port = \"bad\" }}]",
                "x".repeat(400_000)
            ),
            400_027,
            true,
            false,
            43,
        ),
        (
            format!(
                "card = [{{ id = \"{}\", port = \"bad\" }}] # {}",
                "é".repeat(200_000),
                "界".repeat(200_000)
            ),
            200_027,
            true,
            true,
            43,
        ),
    ] {
        let input = format!("schema = 1\n{line}\n");
        let error = parse_toml("long.toml", &input).unwrap_err();
        assert_eq!(error.line_column(), Some((2, column)));
        assert_eq!(&input[error.span().unwrap()], "\"bad\"");
        let snippet = error.snippet().unwrap();
        assert!(snippet.len() < 400, "snippet has {} bytes", snippet.len());
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
        let error = parse_toml(
            "board.toml",
            &format!("schema = 1\n[board]\nmodel = \"{model}\"\n"),
        )
        .unwrap_err();
        assert_eq!(error.line_column(), Some((3, 9)));
        assert_eq!(
            error.message(),
            format!("board model `{model}` is reserved, not implemented (schema 1): {reason}")
        );
        assert_eq!(error.message().matches("is reserved").count(), 1);
    }
}
