// SPDX-License-Identifier: AGPL-3.0-or-later
//! Nominal clock schema, origins, inheritance and resolved artifact round trips.
//! See `backlog/docs/design/machine-files.md` §Nominal CPU clocks.
use machine_file::{Describe, Origins, YamlOrigins};

#[test]
fn nominal_clock_values_are_exact_in_toml_and_yaml() {
    for (number, khz) in [
        ("4.77", 4770),
        ("6", 6000),
        ("33.000", 33000),
        ("0.001", 1),
        ("1000", 1_000_000),
    ] {
        let toml = format!("schema=1\n[cpu]\nclock_mhz={number}\n");
        let yaml = format!("schema: 1\ncpu:\n  clock_mhz: {number}\n");
        for doc in [
            machine_file::parse_toml("clock.toml", &toml).unwrap(),
            machine_file::parse_yaml("clock.yaml", &yaml).unwrap(),
        ] {
            assert_eq!(doc.cpu.unwrap().clock_mhz.unwrap().0.get(), khz);
        }
    }
}

#[test]
fn invalid_nominal_clocks_and_turbo_tables_have_positions() {
    for value in [
        "0", "-1", "1001", "0.0001", "4.7701", "'33'", "true", "{}", "[]", "nan", "inf", "-inf",
        "null",
    ] {
        for key in ["clock_mhz", "turbo.clock_mhz"] {
            let t = format!("schema=1\n[cpu]\n{key}={value}\n");
            let y = if key == "clock_mhz" {
                format!("schema: 1\ncpu:\n  clock_mhz: {value}\n")
            } else {
                format!("schema: 1\ncpu:\n  turbo: {{clock_mhz: {value}}}\n")
            };
            for error in [
                machine_file::parse_toml("clock.toml", &t).unwrap_err(),
                machine_file::parse_yaml("clock.yaml", &y).unwrap_err(),
            ] {
                assert_eq!(error.line_column().unwrap().0, 3, "{error}");
            }
        }
    }
    for text in [
        "schema=1\n[cpu]\nturbo={}\n",
        "schema=1\n[cpu]\nturbo={clock_mhz=8, rate=1}\n",
    ] {
        assert!(machine_file::parse_toml("clock.toml", text).is_err());
    }
}

#[test]
fn rate_and_nominal_clock_conflict_names_the_clock_origin() {
    for yaml in [false, true] {
        let text = if yaml {
            "schema: 1\nboard: {model: generic-386-isa}\ncpu:\n  rate: 1\n  clock_mhz: 33\n"
        } else {
            "schema=1\n[board]\nmodel='generic-386-isa'\n[cpu]\nrate=1\nclock_mhz=33\n"
        };
        let (doc, origins) = if yaml {
            (
                machine_file::parse_yaml("clock.yaml", text).unwrap(),
                Origins::from_yaml("clock.yaml", text).unwrap(),
            )
        } else {
            (
                machine_file::parse_toml("clock.toml", text).unwrap(),
                Origins::from_toml("clock.toml", text).unwrap(),
            )
        };
        let error = doc.describe(&origins).unwrap_err().to_string();
        assert!(error.contains("mutually exclusive"), "{error}");
        assert!(error.contains(if yaml { ":5:" } else { ":6:" }), "{error}");
    }
}

#[test]
fn per_model_clock_mapping_and_board_defaults_are_pinned() {
    for (board, model, mhz, expected) in [
        ("ibm-5170", "80286", "4.77", (1749, 1750)),
        ("ibm-5170", "80286", "6", (44, 35)),
        ("generic-386-isa", "80386dx", "16", (352, 105)),
        ("generic-386-isa", "80386dx", "20", (88, 21)),
        ("generic-386-isa", "80386dx", "25", (110, 21)),
        ("generic-386-isa", "80386dx", "33", (242, 35)),
        ("generic-386-isa", "80386dx", "40", (176, 21)),
        ("generic-486-isa", "80486dx", "25", (220, 21)),
        ("generic-486-isa", "80486dx", "33", (484, 35)),
        ("generic-486-isa", "80486dx", "50", (440, 21)),
        ("generic-486-isa", "80486dx", "66", (968, 35)),
        ("generic-486-isa", "80486dx-cpuid", "100", (880, 21)),
    ] {
        let input = format!("schema=1\n[board]\nmodel='{board}'\n[cpu]\nmodel='{model}'\nclock_mhz={mhz}\nturbo={{clock_mhz=8}}\n");
        let doc = machine_file::parse_toml("clock.toml", &input).unwrap();
        let draft = doc
            .describe(&Origins::from_toml("clock.toml", &input).unwrap())
            .unwrap();
        assert_eq!(
            (
                draft.config.clock.retired_per_pit_clock,
                draft.config.clock.retired_per_pit_clock_denominator
            ),
            expected,
            "{model} {mhz}"
        );
        assert!(draft.config.clock.turbo.is_some());
        let encoded = pc_config::image::encode::machine(&doc);
        let restored =
            machine_file::parse_toml("resolved.toml", &toml::to_string(&encoded).unwrap()).unwrap();
        assert_eq!(doc.cpu, restored.cpu);
    }
    for (board, expected) in [
        (assembly::BoardModel::Ibm5170, (8, 13)),
        (assembly::BoardModel::Generic386Isa, (242, 35)),
        (assembly::BoardModel::Generic486Isa, (484, 35)),
    ] {
        let clock = board.default_clock();
        assert_eq!(
            (
                clock.retired_per_pit_clock,
                clock.retired_per_pit_clock_denominator
            ),
            expected
        );
    }
}

#[test]
fn inherited_clock_selectors_replace_each_other_and_keep_turbo() {
    assert_inherited_clock_selectors(false);
}

#[test]
fn yaml_inherited_clock_selectors_replace_each_other_and_keep_turbo() {
    assert_inherited_clock_selectors(true);
}

fn assert_inherited_clock_selectors(yaml: bool) {
    let temp = machine_file::test_support::TempDir::new("clock-inheritance");
    let extension = if yaml { "yaml" } else { "toml" };
    let parent = temp.path().join(format!("parent.{extension}"));
    let child = temp.path().join(format!("child.{extension}"));
    for nominal in [false, true] {
        let (parent_text, child_text) = if yaml {
            let (base, next) = if nominal {
                ("rate: '8/13'", "clock_mhz: 6")
            } else {
                ("clock_mhz: 6", "rate: '3/2'")
            };
            (
                    format!("schema: 1\nboard: {{model: ibm-5170}}\ncpu:\n  {base}\n  turbo: {{clock_mhz: 8}}\nfirmware:\n  bios: {{image: synthetic.bin}}\n"),
                    format!("schema: 1\nextends: parent.yaml\ncpu:\n  {next}\n"),
                )
        } else {
            let (base, next) = if nominal {
                ("rate='8/13'", "clock_mhz=6")
            } else {
                ("clock_mhz=6", "rate='3/2'")
            };
            (
                    format!("schema=1\n[board]\nmodel='ibm-5170'\n[cpu]\n{base}\nturbo={{clock_mhz=8}}\n[firmware.bios]\nimage='synthetic.bin'\n"),
                    format!("schema=1\nextends='parent.toml'\n[cpu]\n{next}\n"),
                )
        };
        std::fs::write(&parent, parent_text).unwrap();
        std::fs::write(&child, child_text).unwrap();
        let (document, origins) = machine_file::resolve(&child).unwrap();
        let cpu = document.cpu.as_ref().unwrap();
        assert_eq!(cpu.clock_mhz.is_some(), nominal);
        assert_eq!(cpu.rate.is_some(), !nominal);
        assert_eq!(cpu.turbo.as_ref().unwrap().clock_mhz.0.get(), 8000);
        let key = if nominal { "cpu.clock_mhz" } else { "cpu.rate" };
        assert_eq!(origins.get(key).file, child);
        assert_eq!(origins.get(key).line, Some(4));
        assert_eq!(
            origins.get("cpu.turbo.clock_mhz").file,
            std::fs::canonicalize(&parent).unwrap()
        );
        assert_eq!(
            origins.get("cpu.turbo.clock_mhz").line,
            Some(if yaml { 5 } else { 6 })
        );
        let clock = document.describe(&origins).unwrap().config.clock;
        let expected = if nominal { (44, 35) } else { (3, 2) };
        assert_eq!(
            (
                clock.retired_per_pit_clock,
                clock.retired_per_pit_clock_denominator
            ),
            expected
        );
        assert_eq!(clock.turbo.unwrap(), (176, 105));
    }
}

#[test]
fn inherited_clock_conflicts_report_the_defining_layer() {
    assert_inherited_clock_conflicts(false);
}

#[test]
fn yaml_inherited_clock_conflicts_report_the_defining_layer() {
    assert_inherited_clock_conflicts(true);
}

fn assert_inherited_clock_conflicts(yaml: bool) {
    let temp = machine_file::test_support::TempDir::new("clock-conflict-origins");
    let extension = if yaml { "yaml" } else { "toml" };
    let parent = temp.path().join(format!("parent.{extension}"));
    let child = temp.path().join(format!("child.{extension}"));
    for conflict_in_parent in [false, true] {
        let conflict = if yaml {
            "  clock_mhz: 6\n"
        } else {
            "clock_mhz=6\n"
        };
        let (parent_text, child_text) = if yaml {
            (
                    format!("schema: 1\nboard: {{model: ibm-5170}}\ncpu:\n  rate: '8/13'\n{}firmware:\n  bios: {{image: synthetic.bin}}\n", if conflict_in_parent { conflict } else { "" }),
                    format!("schema: 1\nextends: parent.yaml\ncpu:\n  rate: '3/2'\n{}", if conflict_in_parent { "" } else { conflict }),
                )
        } else {
            (
                    format!("schema=1\n[board]\nmodel='ibm-5170'\n[cpu]\nrate='8/13'\n{}[firmware.bios]\nimage='synthetic.bin'\n", if conflict_in_parent { conflict } else { "" }),
                    format!("schema=1\nextends='parent.toml'\n[cpu]\nrate='3/2'\n{}", if conflict_in_parent { "" } else { conflict }),
                )
        };
        std::fs::write(&parent, parent_text).unwrap();
        std::fs::write(&child, child_text).unwrap();
        let machine_file::LoadError::Lower(error) = machine_file::resolve(&child).unwrap_err()
        else {
            panic!("expected a positioned selector conflict");
        };
        assert_eq!(error.field, "cpu.clock_mhz");
        assert_eq!(
            error.origin.file,
            if conflict_in_parent {
                std::fs::canonicalize(&parent).unwrap()
            } else {
                child.clone()
            }
        );
        assert_eq!(
            error.origin.line,
            Some(if conflict_in_parent && !yaml { 6 } else { 5 })
        );
        assert!(error.message.contains("mutually exclusive"));
    }
}
