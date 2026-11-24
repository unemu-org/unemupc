// SPDX-License-Identifier: AGPL-3.0-or-later
//! SeaBIOS ISA board qualification; see machine-files.md §Generic 486 ISA board.

#[path = "support/generic_isa.rs"]
mod generic_isa;

#[test]
fn generic486_dx_shipped_file_has_listed_pinned_firmware_and_assembles() {
    generic_isa::shipped_file_has_listed_pinned_firmware_and_assembles(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_dx_toml_yaml_defaults_and_memory_limits_match() {
    generic_isa::toml_yaml_defaults_and_memory_limits_match(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_dx_seabios_reaches_diskless_post_end() {
    generic_isa::seabios_reaches_diskless_post_end(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_dx_seabios_keyboard_reset_reaches_resume() {
    generic_isa::seabios_keyboard_reset_reaches_resume(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_dx_seabios_int15_memory_matches_e820() {
    generic_isa::seabios_int15_memory_matches_e820(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_cpuid_shipped_file_has_listed_pinned_firmware_and_assembles() {
    generic_isa::shipped_file_has_listed_pinned_firmware_and_assembles(
        assembly::CpuChip::I80486DxCpuid,
    );
}

#[test]
fn generic486_cpuid_toml_yaml_defaults_and_memory_limits_match() {
    generic_isa::toml_yaml_defaults_and_memory_limits_match(assembly::CpuChip::I80486DxCpuid);
}

#[test]
fn generic486_cpuid_seabios_reaches_diskless_post_end() {
    generic_isa::seabios_reaches_diskless_post_end(assembly::CpuChip::I80486DxCpuid);
}

#[test]
fn generic486_cpuid_seabios_keyboard_reset_reaches_resume() {
    generic_isa::seabios_keyboard_reset_reaches_resume(assembly::CpuChip::I80486DxCpuid);
}

#[test]
fn generic486_cpuid_seabios_int15_memory_matches_e820() {
    generic_isa::seabios_int15_memory_matches_e820(assembly::CpuChip::I80486DxCpuid);
}

#[test]
fn generic486_cpu_choices_and_fpu_names_have_positions() {
    for model in ["80486dx", "80486dx-cpuid"] {
        let input = format!("schema = 1\n[cpu]\nmodel = \"{model}\"\nfpu = \"integrated\"\n");
        assert!(machine_file::parse_toml("machine.toml", &input).is_ok());
    }
    for model in ["80486dx-unknown", "80486"] {
        let input = format!("schema = 1\n[cpu]\nmodel = \"{model}\"\n");
        let error = machine_file::parse_toml("machine.toml", &input).unwrap_err();
        assert_eq!(error.line_column(), Some((3, 9)));
        assert!(error.to_string().contains(model));
        let yaml = format!("schema: 1\ncpu:\n  model: {model}\n");
        let error = machine_file::parse_yaml("machine.yaml", &yaml).unwrap_err();
        assert_eq!(
            error.line_column(),
            Some((3, if model == "80486" { 10 } else { 3 }))
        );
    }
}

#[test]
fn generic486_machine_file_requires_explicit_cpu_and_integrated_fpu() {
    struct Image;
    impl machine_file::ImageReader for Image {
        fn read(&mut self, _: &std::path::Path) -> std::io::Result<Vec<u8>> {
            Ok(vec![0xFF; 131072])
        }
    }
    let base = "schema = 1\n[board]\nmodel = \"generic-486-isa\"\n[cpu]\nmodel = \"80486dx\"\nfpu = \"integrated\"\n[firmware.bios]\nimage = \"bios.bin\"\n[[card]]\nkind = \"uart-16550\"\nirq = 4\n";
    for (input, valid, marker) in [
        (base.to_owned(), true, ""),
        (base.replace("model = \"80486dx\"\n", ""), false, "explicit"),
        (base.replace("80486dx", "80386dx"), false, "CPU model"),
        (base.replace("integrated", "none"), false, "integrated"),
        (base.replace("integrated", "80287"), false, "integrated"),
    ] {
        let result = machine_file::lower_with_options(
            &machine_file::parse_toml("machine.toml", &input).unwrap(),
            &machine_file::Origins::from_toml("machine.toml", &input).unwrap(),
            &mut Image,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        );
        assert_eq!(result.is_ok(), valid, "{result:?}");
        if !valid {
            assert!(result.unwrap_err().to_string().contains(marker));
        }
    }
}

#[test]
fn generic486_dx_seabios_int10_modes_have_golden_frames() {
    generic_isa::seabios_int10_modes_have_golden_frames(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_cpuid_seabios_int10_modes_have_golden_frames() {
    generic_isa::seabios_int10_modes_have_golden_frames(assembly::CpuChip::I80486DxCpuid);
}

#[test]
fn generic486_dx_seabios_rejects_relocated_video_rom_with_position() {
    generic_isa::seabios_rejects_relocated_video_rom_with_position(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_dx_seabios_freedos_reaches_prompt() {
    generic_isa::seabios_freedos_reaches_prompt(assembly::CpuChip::I80486Dx, false);
}

#[test]
fn generic486_cpuid_seabios_freedos_reaches_prompt() {
    generic_isa::seabios_freedos_reaches_prompt(assembly::CpuChip::I80486DxCpuid, false);
}

#[test]
fn generic486_dx_seabios_freedos_lists_readonly_ide_marker() {
    generic_isa::seabios_freedos_reaches_prompt(assembly::CpuChip::I80486Dx, true);
}

#[test]
fn generic486_cpuid_seabios_freedos_lists_readonly_ide_marker() {
    generic_isa::seabios_freedos_reaches_prompt(assembly::CpuChip::I80486DxCpuid, true);
}

#[test]
fn generic486_dx_seabios_storage_configuration() {
    generic_isa::seabios_storage_configuration(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_cpuid_seabios_storage_configuration() {
    generic_isa::seabios_storage_configuration(assembly::CpuChip::I80486DxCpuid);
}

#[test]
fn generic486_dx_seabios_freedos_jemmex_v86() {
    generic_isa::seabios_freedos_jemmex_v86(assembly::CpuChip::I80486Dx);
}

#[test]
fn generic486_cpuid_seabios_freedos_jemmex_v86() {
    generic_isa::seabios_freedos_jemmex_v86(assembly::CpuChip::I80486DxCpuid);
}

#[test]
fn generic486_dx_seabios_freedos_dos32a_raw() {
    generic_isa::seabios_freedos_dos32a(assembly::CpuChip::I80486Dx, false);
}

#[test]
fn generic486_dx_seabios_freedos_dos32a_vcpi() {
    generic_isa::seabios_freedos_dos32a(assembly::CpuChip::I80486Dx, true);
}

#[test]
fn generic486_cpuid_seabios_freedos_dos32a_raw() {
    generic_isa::seabios_freedos_dos32a(assembly::CpuChip::I80486DxCpuid, false);
}

#[test]
fn generic486_cpuid_seabios_freedos_dos32a_vcpi() {
    generic_isa::seabios_freedos_dos32a(assembly::CpuChip::I80486DxCpuid, true);
}
