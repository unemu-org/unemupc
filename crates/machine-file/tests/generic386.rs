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
fn generic386_seabios_equipment_matches_fpu() {
    generic_isa::seabios_equipment_matches_fpu(assembly::CpuChip::I80386);
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

#[test]
fn generic386_seabios_int10_modes_have_golden_frames() {
    generic_isa::seabios_int10_modes_have_golden_frames(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_rejects_relocated_video_rom_with_position() {
    generic_isa::seabios_rejects_relocated_video_rom_with_position(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_freedos_reaches_prompt() {
    generic_isa::seabios_freedos_reaches_prompt(assembly::CpuChip::I80386, false);
}

#[test]
fn generic386_seabios_freedos_lists_readonly_ide_marker() {
    generic_isa::seabios_freedos_reaches_prompt(assembly::CpuChip::I80386, true);
}

#[test]
fn generic386_seabios_storage_configuration() {
    generic_isa::seabios_storage_configuration(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_sb16_opl3_golden() {
    generic_isa::seabios_sb16_opl3(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_freedos_jemmex_v86() {
    generic_isa::seabios_freedos_jemmex_v86(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_freedos_dos32a_raw() {
    generic_isa::seabios_freedos_dos32a(assembly::CpuChip::I80386, false);
}

#[test]
fn generic386_seabios_freedos_dos32a_vcpi() {
    generic_isa::seabios_freedos_dos32a(assembly::CpuChip::I80386, true);
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_DOOM_DIR"]
fn user_media_doom_386() {
    generic_isa::user_media_doom(assembly::CpuChip::I80386, false);
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_DOOM_DIR"]
fn user_media_doom_sound_386() {
    generic_isa::user_media_doom(assembly::CpuChip::I80386, true);
}

#[test]
fn generic386_80387_seabios_reaches_diskless_post_end() {
    generic_isa::seabios_80387_post();
}

#[test]
#[ignore = "slow optional 80387 FreeDOS qualification"]
fn generic386_80387_seabios_freedos_reaches_prompt() {
    generic_isa::seabios_80387_freedos();
}

#[test]
fn generic386_seabios_int13_disk_services() {
    generic_isa::seabios_int13_disk_services(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_ata_protected_driver() {
    generic_isa::seabios_ata_protected_driver(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_apm_protected_power_off() {
    generic_isa::seabios_apm_protected_power_off(assembly::CpuChip::I80386);
}

#[test]
fn generic386_seabios_mouse_aux_and_bios_handler() {
    generic_isa::seabios_mouse_aux_and_bios_handler(assembly::CpuChip::I80386);
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_DOOM_DIR"]
fn user_media_doom_demo_386() {
    generic_isa::user_media_doom_demo(assembly::CpuChip::I80386);
}
