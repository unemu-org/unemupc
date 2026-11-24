// SPDX-License-Identifier: AGPL-3.0-or-later
//! SeaBIOS ISA board qualification; see machine-files.md §Generic 386 ISA board.

#[path = "support/generic_isa.rs"]
mod generic_isa;

#[test]
fn generic386_shipped_file_has_listed_pinned_firmware_and_assembles() {
    generic_isa::shipped_file_has_listed_pinned_firmware_and_assembles(assembly::CpuChip::I80386);
}

#[test]
fn generic386_toml_yaml_defaults_and_memory_limits_match() {
    generic_isa::toml_yaml_defaults_and_memory_limits_match(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_reaches_diskless_post_end() {
    generic_isa::seabios_reaches_diskless_post_end(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_keyboard_reset_reaches_resume() {
    generic_isa::seabios_keyboard_reset_reaches_resume(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_int15_memory_matches_e820() {
    generic_isa::seabios_int15_memory_matches_e820(assembly::CpuChip::I80386);
}
