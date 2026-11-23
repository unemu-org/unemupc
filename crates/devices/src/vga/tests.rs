// SPDX-License-Identifier: AGPL-3.0-or-later
//! Isolation tests for IBM VGA register and memory access protocols.
use super::*;
use testkit::MockBus;

fn bus() -> MockBus {
    MockBus::new(Box::new(Vga::new()))
}
fn out(bus: &mut MockBus, port: u16, value: u8) {
    bus.port_write(port, 1, u32::from(value)).unwrap();
}
fn input(bus: &mut MockBus, port: u16) -> u8 {
    bus.port_read(port, 1).unwrap() as u8
}
fn reg(bus: &mut MockBus, port: u16, index: u8, value: u8) {
    bus.port_write(port, 2, u32::from(index) | (u32::from(value) << 8))
        .unwrap();
}
fn read_reg(bus: &mut MockBus, port: u16, index: u8) -> u8 {
    out(bus, port, index);
    input(bus, port + 1)
}
fn card(bus: &mut MockBus) -> &mut Vga {
    bus.device_mut()
        .as_any_mut()
        .unwrap()
        .downcast_mut()
        .unwrap()
}
fn planar(bus: &mut MockBus) {
    reg(bus, 0x3C4, 4, 4);
    reg(bus, 0x3C4, 2, 15);
    reg(bus, 0x3CE, 6, 4);
    reg(bus, 0x3CE, 8, 255);
}
fn seed(bus: &mut MockBus, offset: u64, bytes: [u8; 4]) {
    for (p, byte) in bytes.into_iter().enumerate() {
        reg(bus, 0x3C4, 2, 1 << p);
        bus.mmio_write(MEMORY_BASE + offset, 1, u64::from(byte))
            .unwrap();
    }
    reg(bus, 0x3C4, 2, 15);
}
fn bytes(bus: &mut MockBus, offset: usize) -> [u8; 4] {
    std::array::from_fn(|p| card(bus).planes()[p][offset])
}

#[test]
fn indexed_register_files_read_back_masks_and_reserved_indices() {
    let mut b = bus();
    for (port, masks) in [
        (0x3C4, [0x03, 0x3F, 0x0F, 0x3F, 0x0E].as_slice()),
        (
            0x3CE,
            [0x0F, 0x0F, 0x0F, 0x1F, 0x03, 0x7B, 0x0F, 0x0F, 0xFF].as_slice(),
        ),
    ] {
        for (index, mask) in masks.iter().enumerate() {
            reg(&mut b, port, index as u8, 0xFF);
            assert_eq!(read_reg(&mut b, port, index as u8), *mask);
        }
    }
    for i in 0..25 {
        reg(&mut b, 0x3B4, i, i + 1);
        assert_eq!(read_reg(&mut b, 0x3B4, i), i + 1);
    }
    for i in 0..21 {
        input(&mut b, 0x3BA);
        out(&mut b, 0x3C0, i);
        out(&mut b, 0x3C0, 0xFF);
        assert_eq!(
            input(&mut b, 0x3C1),
            match i {
                0..=15 => 0x3F,
                16 => 0xEF,
                17 => 0xFF,
                _ => 0x0F,
            }
        );
    }
    for (port, index) in [(0x3C4, 7), (0x3CE, 15), (0x3B4, 31)] {
        let before = (
            *card(&mut b).sequencer(),
            *card(&mut b).crtc(),
            *card(&mut b).graphics(),
        );
        reg(&mut b, port, index, 0xDD);
        assert_eq!(input(&mut b, port), index);
        assert_eq!(input(&mut b, port + 1), 0);
        assert_eq!(
            (
                *card(&mut b).sequencer(),
                *card(&mut b).crtc(),
                *card(&mut b).graphics()
            ),
            before
        );
    }
    input(&mut b, 0x3BA);
    out(&mut b, 0x3C0, 31);
    out(&mut b, 0x3C0, 0xFF);
    assert_eq!(input(&mut b, 0x3C1), 0);
    out(&mut b, 0x3C4, 0xFA);
    assert_eq!(input(&mut b, 0x3C4), 2);
    out(&mut b, 0x3CE, 0xF4);
    assert_eq!(input(&mut b, 0x3CE), 4);
    out(&mut b, 0x3B4, 0xEC);
    assert_eq!(input(&mut b, 0x3B4), 12);
}

#[test]
fn crtc_protection_preserves_all_low_registers_except_overflow_bit_four() {
    let mut b = bus();
    for i in 0..8 {
        reg(&mut b, 0x3B4, i, 0xA5);
    }
    reg(&mut b, 0x3B4, 0x11, 0x80);
    for i in 0..8 {
        reg(&mut b, 0x3B4, i, 0xFF);
    }
    for i in 0..8 {
        assert_eq!(read_reg(&mut b, 0x3B4, i), if i == 7 { 0xB5 } else { 0xA5 });
    }
    reg(&mut b, 0x3B4, 7, 0);
    assert_eq!(read_reg(&mut b, 0x3B4, 7), 0xA5);
    reg(&mut b, 0x3B4, 8, 0x55);
    assert_eq!(read_reg(&mut b, 0x3B4, 8), 0x55);
    reg(&mut b, 0x3B4, 0x11, 0);
    reg(&mut b, 0x3B4, 0, 0x77);
    assert_eq!(read_reg(&mut b, 0x3B4, 0), 0x77);
}

#[test]
fn mono_colour_decode_misc_feature_and_status_are_direction_sensitive() {
    let mut b = bus();
    for (misc, active, inactive, status, other_status) in [
        (0, 0x3B4, 0x3D4, 0x3BA, 0x3DA),
        (0xE3, 0x3D4, 0x3B4, 0x3DA, 0x3BA),
    ] {
        out(&mut b, 0x3C2, misc);
        assert_eq!(input(&mut b, 0x3CC), misc);
        assert_eq!(input(&mut b, 0x3C2), 0x10);
        reg(&mut b, active, 0x0F, 0x42);
        reg(&mut b, inactive, 0x0F, 0x99);
        assert_eq!(input(&mut b, inactive), 0xFF);
        assert_eq!(input(&mut b, inactive + 1), 0xFF);
        assert_eq!(read_reg(&mut b, active, 0x0F), 0x42);
        out(&mut b, status, 0xFE);
        assert_eq!(input(&mut b, 0x3CA), 0xFE);
        out(&mut b, other_status, 1);
        assert_eq!(input(&mut b, 0x3CA), 0xFE);
        assert_eq!(input(&mut b, other_status), 0xFF);
    }
}

#[test]
fn attribute_flip_flop_pas_and_active_status_reset() {
    let mut b = bus();
    out(&mut b, 0x3C0, 3);
    assert_eq!(input(&mut b, 0x3C0), 3);
    input(&mut b, 0x3C1);
    input(&mut b, 0x3DA);
    out(&mut b, 0x3C0, 0x2B);
    assert_eq!(input(&mut b, 0x3C1), 0x2B);
    out(&mut b, 0x3C0, 0x23);
    out(&mut b, 0x3C0, 0x11);
    assert_eq!(input(&mut b, 0x3C1), 0x11);
    assert!(card(&mut b).palette_address_source());
    out(&mut b, 0x3C0, 0x34);
    out(&mut b, 0x3C0, 7);
    assert_eq!(input(&mut b, 0x3C1), 7);
    out(&mut b, 0x3C0, 5);
    input(&mut b, 0x3BA);
    out(&mut b, 0x3C0, 6);
    out(&mut b, 0x3C0, 0x22);
    assert_eq!(input(&mut b, 0x3C1), 0x22);
    assert_eq!(card(&mut b).attribute()[5], 0);
}

#[test]
fn status_phase_is_deterministic_without_ticks_and_tick_is_passive() {
    let mut b = bus();
    assert_eq!(
        (0..4).map(|_| input(&mut b, 0x3BA)).collect::<Vec<_>>(),
        [0; 4]
    );
    b.tick(412 * 38);
    assert_eq!(input(&mut b, 0x3BA), 9);
    assert_eq!(b.device().next_event(b.clock()), NextEvent::Dormant);
    assert!(!b.device().irq_pending());
    assert!(b.device().text_scanout().is_none());
}

#[test]
fn dac_rgb_sequences_wrap_indices_and_mask_only_display_lookup() {
    let mut b = bus();
    out(&mut b, 0x3C8, 255);
    assert_eq!(input(&mut b, 0x3C7), 0);
    for value in [0xFF, 0x82, 3, 4, 5, 6] {
        out(&mut b, 0x3C9, value);
    }
    assert_eq!(input(&mut b, 0x3C8), 1);
    assert_eq!(card(&mut b).palette()[255], [63, 2, 3]);
    assert_eq!(card(&mut b).palette()[0], [4, 5, 6]);
    out(&mut b, 0x3C7, 255);
    assert_eq!(input(&mut b, 0x3C7), 3);
    assert_eq!(
        (0..6).map(|_| input(&mut b, 0x3C9)).collect::<Vec<_>>(),
        [63, 2, 3, 4, 5, 6]
    );
    out(&mut b, 0x3C6, 0);
    assert_eq!(input(&mut b, 0x3C6), 0);
    assert_eq!(card(&mut b).palette_entry(255), [4, 5, 6]);
    out(&mut b, 0x3C7, 255);
    assert_eq!(input(&mut b, 0x3C9), 63);
    out(&mut b, 0x3C6, 255);
    assert_eq!(card(&mut b).palette_entry(255), [63, 2, 3]);
    out(&mut b, 0x3C8, 15);
    for value in [11, 22, 33] {
        out(&mut b, 0x3C9, value);
    }
    out(&mut b, 0x3C6, 0x0F);
    assert_eq!(card(&mut b).palette_entry(255), [11, 22, 33]);
    assert_eq!(card(&mut b).palette()[255], [63, 2, 3]);
    out(&mut b, 0x3C8, 7);
    out(&mut b, 0x3C9, 8);
    out(&mut b, 0x3C8, 8);
    out(&mut b, 0x3C9, 9);
    assert_eq!(card(&mut b).palette()[7], [8, 0, 0]);
    assert_eq!(card(&mut b).palette()[8], [9, 0, 0]);
    out(&mut b, 0x3C7, 255);
    input(&mut b, 0x3C9);
    out(&mut b, 0x3C7, 0);
    assert_eq!(input(&mut b, 0x3C9), 4);
}

#[test]
fn write_mode_zero_set_reset_rotate_logic_and_bit_mask() {
    for (function, expected) in [
        (0, [0xBE, 0x41, 0xD4, 0x17]),
        (1, [0x96, 0x41, 0xD0, 0x07]),
        (2, [0xBE, 0x69, 0xF4, 0x1F]),
        (3, [0xAA, 0x69, 0xE4, 0x1B]),
    ] {
        let mut b = bus();
        planar(&mut b);
        seed(&mut b, 0, [0x96, 0x69, 0xF0, 0x0F]);
        b.mmio_read(MEMORY_BASE, 1).unwrap();
        reg(&mut b, 0x3CE, 0, 5);
        reg(&mut b, 0x3CE, 1, 3);
        reg(&mut b, 0x3CE, 3, (function << 3) | 3);
        reg(&mut b, 0x3CE, 8, 0x3C);
        b.mmio_write(MEMORY_BASE + 1, 1, 0xB2).unwrap();
        assert_eq!(bytes(&mut b, 1), expected, "function {function}");
        assert_eq!(bytes(&mut b, 0), [0x96, 0x69, 0xF0, 0x0F]);
    }
}

#[test]
fn write_mode_one_copies_read_latches_ignoring_host_data_and_pipeline() {
    let mut b = bus();
    planar(&mut b);
    seed(&mut b, 4, [0x12, 0x34, 0x56, 0x78]);
    reg(&mut b, 0x3CE, 4, 3);
    assert_eq!(b.mmio_read(MEMORY_BASE + 4, 1).unwrap(), 0x78);
    reg(&mut b, 0x3CE, 5, 1);
    reg(&mut b, 0x3CE, 8, 0);
    reg(&mut b, 0x3CE, 3, 0x1F);
    reg(&mut b, 0x3C4, 2, 5);
    b.mmio_write(MEMORY_BASE + 5, 1, 0xFF).unwrap();
    assert_eq!(bytes(&mut b, 5), [0x12, 0, 0x56, 0]);
    reg(&mut b, 0x3C4, 2, 15);
    b.mmio_write(MEMORY_BASE + 6, 1, 0).unwrap();
    assert_eq!(bytes(&mut b, 6), [0x12, 0x34, 0x56, 0x78]);
}

#[test]
fn write_mode_two_expands_cpu_low_nibble_before_logic_and_mask() {
    let mut b = bus();
    planar(&mut b);
    seed(&mut b, 0, [0x96, 0x69, 0xF0, 0x0F]);
    b.mmio_read(MEMORY_BASE, 1).unwrap();
    reg(&mut b, 0x3CE, 0, 15);
    reg(&mut b, 0x3CE, 1, 15);
    reg(&mut b, 0x3CE, 5, 2);
    reg(&mut b, 0x3CE, 3, 0x1B);
    reg(&mut b, 0x3CE, 8, 0x5A);
    b.mmio_write(MEMORY_BASE + 1, 1, 0xAA).unwrap();
    assert_eq!(bytes(&mut b, 1), [0x96, 0x33, 0xF0, 0x55]);
}

#[test]
fn write_mode_three_uses_rotated_mask_set_reset_and_function_select() {
    let mut b = bus();
    planar(&mut b);
    seed(&mut b, 0, [0x96, 0x69, 0xF0, 0x0F]);
    b.mmio_read(MEMORY_BASE, 1).unwrap();
    reg(&mut b, 0x3CE, 0, 5);
    reg(&mut b, 0x3CE, 1, 0);
    reg(&mut b, 0x3CE, 5, 3);
    reg(&mut b, 0x3CE, 3, 0x1B);
    reg(&mut b, 0x3CE, 8, 0x3C);
    b.mmio_write(MEMORY_BASE + 1, 1, 0xB2).unwrap();
    assert_eq!(bytes(&mut b, 1), [0x82, 0x69, 0xE4, 0x0F]);
}

#[test]
fn read_modes_map_select_compare_and_dont_care_load_all_latches() {
    let mut b = bus();
    planar(&mut b);
    seed(&mut b, 0, [0xAA, 0xCC, 0xF0, 0x0F]);
    for (p, expected) in [0xAA, 0xCC, 0xF0, 0x0F].into_iter().enumerate() {
        reg(&mut b, 0x3CE, 4, p as u8);
        assert_eq!(b.mmio_read(MEMORY_BASE, 1).unwrap(), expected);
    }
    reg(&mut b, 0x3CE, 5, 8);
    reg(&mut b, 0x3CE, 2, 5);
    reg(&mut b, 0x3CE, 7, 15);
    assert_eq!(b.mmio_read(MEMORY_BASE, 1).unwrap(), 0x20);
    reg(&mut b, 0x3CE, 7, 3);
    assert_eq!(b.mmio_read(MEMORY_BASE, 1).unwrap(), 0x22);
    reg(&mut b, 0x3CE, 7, 0);
    assert_eq!(b.mmio_read(MEMORY_BASE, 1).unwrap(), 0xFF);
    reg(&mut b, 0x3CE, 5, 1);
    b.mmio_write(MEMORY_BASE + 1, 1, 0).unwrap();
    assert_eq!(bytes(&mut b, 1), [0xAA, 0xCC, 0xF0, 0x0F]);
}

#[test]
fn map_mask_and_writes_do_not_reload_latches() {
    let mut b = bus();
    planar(&mut b);
    reg(&mut b, 0x3C4, 2, 0);
    b.mmio_write(MEMORY_BASE, 1, 255).unwrap();
    assert_eq!(bytes(&mut b, 0), [0; 4]);
    reg(&mut b, 0x3C4, 2, 10);
    b.mmio_write(MEMORY_BASE, 1, 0xA5).unwrap();
    assert_eq!(bytes(&mut b, 0), [0, 0xA5, 0, 0xA5]);
    reg(&mut b, 0x3CE, 8, 0x0F);
    b.mmio_write(MEMORY_BASE, 1, 0x3C).unwrap();
    assert_eq!(bytes(&mut b, 0), [0, 0x0C, 0, 0x0C]);
}

#[test]
fn chain_four_round_trip_plane_offset_precedence_and_map_mask() {
    let mut b = bus();
    planar(&mut b);
    reg(&mut b, 0x3B4, 0x14, 0x40);
    reg(&mut b, 0x3C4, 4, 8);
    reg(&mut b, 0x3CE, 5, 0x10);
    b.mmio_write(MEMORY_BASE + 4, 4, 0x44332211).unwrap();
    assert_eq!(bytes(&mut b, 4), [0x11, 0x22, 0x33, 0x44]);
    assert_eq!(b.mmio_read(MEMORY_BASE + 4, 4).unwrap(), 0x44332211);
    reg(&mut b, 0x3C4, 2, 2);
    b.mmio_write(MEMORY_BASE + 4, 4, 0x88776655).unwrap();
    assert_eq!(bytes(&mut b, 4), [0x11, 0x66, 0x33, 0x44]);
    b.mmio_read(MEMORY_BASE + 6, 1).unwrap();
    reg(&mut b, 0x3C4, 4, 4);
    reg(&mut b, 0x3C4, 2, 15);
    reg(&mut b, 0x3CE, 5, 1);
    b.mmio_write(MEMORY_BASE + 9, 1, 0).unwrap();
    assert_eq!(bytes(&mut b, 9), [0x11, 0x66, 0x33, 0x44]);
}

#[test]
fn odd_even_round_trip_pairs_chain_bit_and_independent_read_write_controls() {
    let mut b = bus();
    planar(&mut b);
    out(&mut b, 0x3C2, 0x66);
    reg(&mut b, 0x3C4, 4, 0);
    reg(&mut b, 0x3CE, 6, 6);
    reg(&mut b, 0x3CE, 5, 0x10);
    reg(&mut b, 0x3C4, 2, 3);
    b.mmio_write(MEMORY_BASE + 4, 2, 0x1F41).unwrap();
    assert_eq!(bytes(&mut b, 4), [0x41, 0x1F, 0, 0]);
    assert_eq!(b.mmio_read(MEMORY_BASE + 4, 2).unwrap(), 0x1F41);
    reg(&mut b, 0x3C4, 2, 12);
    b.mmio_write(MEMORY_BASE + 4, 2, 0x7253).unwrap();
    reg(&mut b, 0x3CE, 4, 2);
    assert_eq!(b.mmio_read(MEMORY_BASE + 4, 2).unwrap(), 0x7253);
    assert_eq!(bytes(&mut b, 4), [0x41, 0x1F, 0x53, 0x72]);
    reg(&mut b, 0x3CE, 6, 2);
    b.mmio_write(MEMORY_BASE + 0x10000, 2, 0xA796).unwrap();
    assert_eq!(bytes(&mut b, 1), [0, 0, 0x96, 0xA7]);
    assert_eq!(b.mmio_read(MEMORY_BASE + 0x10000, 2).unwrap(), 0xA796);
    reg(&mut b, 0x3C4, 4, 4);
    b.mmio_write(MEMORY_BASE + 8, 1, 0x88).unwrap();
    assert_eq!(bytes(&mut b, 8), [0, 0, 0x88, 0x88]);
    assert_eq!(b.mmio_read(MEMORY_BASE + 8, 2).unwrap(), 0x8888);
    reg(&mut b, 0x3C4, 4, 0);
    reg(&mut b, 0x3CE, 5, 0);
    b.mmio_write(MEMORY_BASE + 10, 2, 0xDDBC).unwrap();
    assert_eq!(bytes(&mut b, 10), [0, 0, 0xBC, 0xDD]);
    assert_eq!(b.mmio_read(MEMORY_BASE + 10, 2).unwrap(), 0xBC);
    reg(&mut b, 0x3CE, 5, 0x10);
    reg(&mut b, 0x3C4, 2, 15);
    reg(&mut b, 0x3CE, 6, 4);
    b.mmio_write(MEMORY_BASE + 6, 2, 0xBBAA).unwrap();
    assert_eq!(bytes(&mut b, 6), [0xAA, 0, 0xAA, 0]);
    assert_eq!(bytes(&mut b, 7), [0, 0xBB, 0, 0xBB]);
    assert_eq!(b.mmio_read(MEMORY_BASE + 6, 2).unwrap(), 0xBBAA);
    reg(&mut b, 0x3C4, 4, 4);
    reg(&mut b, 0x3CE, 5, 0);
    assert_eq!(bytes(&mut b, 3), [0; 4]);
    b.mmio_write(MEMORY_BASE + 3, 1, 0x99).unwrap();
    assert_eq!(bytes(&mut b, 3), [0x99; 4]);
}

#[test]
fn memory_maps_decode_boundaries_mirror_and_preserve_latches_on_open_bus() {
    for (map, base, len) in [
        (0, 0xA0000, 0x20000),
        (1, 0xA0000, 0x10000),
        (2, 0xB0000, 0x8000),
        (3, 0xB8000, 0x8000),
    ] {
        let mut b = bus();
        planar(&mut b);
        reg(&mut b, 0x3CE, 6, map << 2);
        b.mmio_write(base, 1, 0x5A).unwrap();
        b.mmio_write(base + len - 1, 1, 0xC3).unwrap();
        assert_eq!(b.mmio_read(base, 1).unwrap(), 0x5A);
        assert_eq!(b.mmio_read(base + len - 1, 2).unwrap(), 0xFFC3);
        for addr in [0xA0000, 0xAFFFF, 0xB0000, 0xB7FFF, 0xB8000, 0xBFFFF] {
            if !(base..base + len).contains(&addr) {
                b.mmio_write(addr, 1, 0x77).unwrap();
                assert_eq!(b.mmio_read(addr, 1).unwrap(), 0xFF);
            }
        }
        assert_eq!(b.mmio_read(base, 1).unwrap(), 0x5A);
        if map == 0 {
            assert_eq!(b.mmio_read(base + 0x10000, 1).unwrap(), 0x5A);
        }
        if map != 0 {
            let floating = if map == 1 { 0xB0000 } else { 0xA0000 };
            assert_eq!(b.mmio_read(floating, 1).unwrap(), 0xFF);
        }
        reg(&mut b, 0x3CE, 5, 1);
        b.mmio_write(base + 1, 1, 0).unwrap();
        assert_eq!(bytes(&mut b, 1), [0x5A; 4]);
    }
}

#[test]
fn snapshot_round_trip_continues_partial_dac_ac_latches_and_status() {
    let mut b = bus();
    planar(&mut b);
    seed(&mut b, 0x1234, [1, 2, 3, 4]);
    b.mmio_read(MEMORY_BASE + 0x1234, 1).unwrap();
    b.tick(91);
    for i in 0..25 {
        reg(&mut b, 0x3B4, i, i + 3);
    }
    out(&mut b, 0x3BA, 3);
    out(&mut b, 0x3C6, 0x7F);
    out(&mut b, 0x3C8, 254);
    out(&mut b, 0x3C9, 12);
    input(&mut b, 0x3BA);
    out(&mut b, 0x3C0, 0x34);
    let saved = b.device().save();
    let mut restored = bus();
    restored.device_mut().restore(&saved).unwrap();
    assert_eq!(restored.device().save(), saved);
    for device in [&mut b, &mut restored] {
        out(device, 0x3C9, 21);
        out(device, 0x3C9, 9);
        assert_eq!(input(device, 0x3C8), 255);
        out(device, 0x3C0, 9);
        out(device, 0x3C7, 254);
        assert_eq!(
            (0..3).map(|_| input(device, 0x3C9)).collect::<Vec<_>>(),
            [12, 21, 9]
        );
        assert_eq!(input(device, 0x3BA), 0);
        reg(device, 0x3CE, 5, 1);
        device.mmio_write(MEMORY_BASE + 0x4321, 1, 0xFF).unwrap();
        assert_eq!(bytes(device, 0x4321), [1, 2, 3, 4]);
        assert_eq!(card(device).palette()[254], [12, 21, 9]);
        assert_eq!(card(device).attribute()[20], 9);
    }
    assert_eq!(restored.device().save(), b.device().save());
    restored.device_mut().reset();
    assert_eq!(restored.device().save(), Vga::new().save());
}

#[test]
fn invalid_snapshots_and_accesses_are_atomic_and_untraced() {
    let mut b = bus();
    planar(&mut b);
    b.device_mut()
        .set_trace_policy(device::TracePolicy::Ring { capacity: 10 });
    let saved = b.device().save();
    for (offset, value) in [
        (0, 1),
        (3, 8),
        (4, 32),
        (5, 16),
        (6, 64),
        (7, 2),
        (10, 3),
        (11, 2),
        (16, 0x80),
        (71, 0x10),
        (73, 0x30),
        (80, 64),
    ] {
        let mut bad = saved.clone();
        bad[offset] = value;
        assert!(b.device_mut().restore(&bad).is_err(), "offset {offset}");
        assert_eq!(b.device().save(), saved);
    }
    assert!(b.device_mut().restore(&saved[..saved.len() - 1]).is_err());
    for size in [0, 9, 255] {
        assert!(b.mmio_read(MEMORY_BASE, size).is_err());
        assert!(b.mmio_write(MEMORY_BASE, size, 0).is_err());
    }
    for addr in [MEMORY_BASE - 1, MEMORY_BASE + MEMORY_LEN as u64, u64::MAX] {
        assert!(b.mmio_read(addr, 1).is_err());
        assert!(b.mmio_write(addr, 1, 0).is_err());
    }
    for (port, size) in [(0x3C4, 0), (0x3C4, 3), (0x3CA, 2), (0xFFFF, 2)] {
        assert!(b.port_read(port, size).is_err());
        assert!(b.port_write(port, size, 0).is_err());
    }
    assert_eq!(b.device().save(), saved);
    assert!(b.device_mut().take_trace().is_empty());
}

#[test]
fn observation_getters_are_passive_and_mmio_shares_the_port_trace_ring() {
    let mut b = bus();
    planar(&mut b);
    b.tick(47);
    b.device_mut()
        .set_trace_policy(device::TracePolicy::Ring { capacity: 2 });
    out(&mut b, 0x3C2, 3);
    b.mmio_write(MEMORY_BASE + 2, 2, 0xBBAA).unwrap();
    assert_eq!(b.mmio_read(MEMORY_BASE + 2, 2).unwrap(), 0xBBAA);
    let saved = b.device().save();
    let vga = card(&mut b);
    assert_eq!(vga.planes()[0][2..4], [0xAA, 0xBB]);
    assert_eq!(vga.sequencer(), &[0, 0, 15, 0, 4]);
    assert_eq!(vga.crtc(), &[0; 25]);
    assert_eq!(vga.graphics(), &[0, 0, 0, 0, 0, 0, 4, 0, 255]);
    assert_eq!(vga.attribute(), &[0; 21]);
    assert_eq!(vga.palette(), &[[0; 3]; 256]);
    assert_eq!(vga.palette_entry(123), [0; 3]);
    assert_eq!(vga.misc_output(), 3);
    assert_eq!(vga.pel_mask(), 0);
    assert!(!vga.palette_address_source());
    assert_eq!(b.device().save(), saved);
    assert_eq!(b.device().trace_dropped(), 1);
    assert_eq!(
        b.device_mut().take_trace(),
        vec![
            TraceEvent::Mmio {
                at: VClock(47),
                addr: MEMORY_BASE + 2,
                size: 2,
                dir: AccessDir::Write,
                value: 0xBBAA
            },
            TraceEvent::Mmio {
                at: VClock(47),
                addr: MEMORY_BASE + 2,
                size: 2,
                dir: AccessDir::Read,
                value: 0xBBAA
            },
        ]
    );
}

#[test]
fn chain_four_unchaining_exposes_doubleword_offsets_to_planar_reads() {
    let mut b = bus();
    planar(&mut b);
    reg(&mut b, 0x3B4, 0x14, 0x40);
    reg(&mut b, 0x3C4, 4, 0x0E);
    let locations = [
        (0, 0),
        (4, 4),
        (0x4000, 0x4001),
        (0x4004, 0x4005),
        (0x8000, 0x8002),
        (0xC000, 0xC003),
        (0xFFFC, 0xFFFF),
    ];
    for (n, (cpu, _)) in locations.into_iter().enumerate() {
        let value = 0x40302010 + (n as u64) * 0x01010101;
        b.mmio_write(MEMORY_BASE + cpu, 4, value).unwrap();
        assert_eq!(b.mmio_read(MEMORY_BASE + cpu, 4).unwrap(), value);
    }
    reg(&mut b, 0x3C4, 4, 6);
    reg(&mut b, 0x3B4, 0x14, 0);
    for (n, (_, offset)) in locations.into_iter().enumerate() {
        for (plane, base) in [0x10, 0x20, 0x30, 0x40].into_iter().enumerate() {
            reg(&mut b, 0x3CE, 4, plane as u8);
            assert_eq!(
                b.mmio_read(MEMORY_BASE + offset as u64, 1).unwrap(),
                base + n as u64
            );
        }
    }
    reg(&mut b, 0x3CE, 4, 0);
    assert_eq!(b.mmio_read(MEMORY_BASE + 1, 1).unwrap(), 0);
}

#[test]
fn odd_even_chain_replaces_a0_only_when_gr6_enables_it() {
    let mut b = bus();
    planar(&mut b);
    reg(&mut b, 0x3C4, 4, 0);
    reg(&mut b, 0x3CE, 5, 0x10);
    reg(&mut b, 0x3CE, 6, 0);
    b.mmio_write(MEMORY_BASE + 6, 2, 0xBBAA).unwrap();
    assert_eq!(bytes(&mut b, 6), [0xAA, 0, 0xAA, 0]);
    assert_eq!(bytes(&mut b, 7), [0, 0xBB, 0, 0xBB]);
    assert_eq!(b.mmio_read(MEMORY_BASE + 6, 2).unwrap(), 0xBBAA);
    reg(&mut b, 0x3CE, 6, 2);
    b.mmio_write(MEMORY_BASE + 8, 2, 0xDDCC).unwrap();
    b.mmio_write(MEMORY_BASE + 0x10008, 2, 0xFFEE).unwrap();
    assert_eq!(bytes(&mut b, 8), [0xCC, 0xDD, 0xCC, 0xDD]);
    assert_eq!(bytes(&mut b, 9), [0xEE, 0xFF, 0xEE, 0xFF]);
    assert_eq!(b.mmio_read(MEMORY_BASE + 8, 2).unwrap(), 0xDDCC);
    assert_eq!(b.mmio_read(MEMORY_BASE + 0x10008, 2).unwrap(), 0xFFEE);
}

#[test]
fn odd_even_page_policy_uses_inverted_msr_bit_five_outside_map_zero() {
    for (map, base) in [(1, 0xA0000), (2, 0xB0000), (3, 0xB8000)] {
        let mut b = bus();
        planar(&mut b);
        reg(&mut b, 0x3C4, 4, 0);
        reg(&mut b, 0x3CE, 5, 0x10);
        reg(&mut b, 0x3CE, 6, (map << 2) | 2);
        for misc in [0x67, 0x63, 0x66] {
            out(&mut b, 0x3C2, misc);
            b.mmio_write(base + 4, 2, 0x2211).unwrap();
            assert_eq!(bytes(&mut b, 4), [0x11, 0x22, 0x11, 0x22]);
            assert_eq!(b.mmio_read(base + 4, 2).unwrap(), 0x2211);
            out(&mut b, 0x3C2, misc & !0x20);
            b.mmio_write(base + 4, 2, 0x4433).unwrap();
            assert_eq!(bytes(&mut b, 5), [0x33, 0x44, 0x33, 0x44]);
            assert_eq!(b.mmio_read(base + 4, 2).unwrap(), 0x4433);
            out(&mut b, 0x3C2, misc);
            assert_eq!(b.mmio_read(base + 4, 2).unwrap(), 0x2211);
        }
        reg(&mut b, 0x3CE, 6, 2);
        for misc in [0, 0x20] {
            out(&mut b, 0x3C2, misc);
            assert_eq!(b.mmio_read(MEMORY_BASE + 4, 2).unwrap(), 0x2211);
            assert_eq!(b.mmio_read(MEMORY_BASE + 0x10004, 2).unwrap(), 0x4433);
        }
    }
}

#[test]
fn attribute_widths_follow_ibm_mode_overscan_and_plane_enable_figures() {
    let mut b = bus();
    for (index, written, expected) in [
        (0x10, 0xFF, 0xEF),
        (0x10, 0x10, 0),
        (0x11, 0xA5, 0xA5),
        (0x11, 0xFF, 0xFF),
        (0x12, 0x3F, 0x0F),
        (0x12, 0x30, 0),
    ] {
        input(&mut b, 0x3BA);
        out(&mut b, 0x3C0, index);
        out(&mut b, 0x3C0, written);
        assert_eq!(input(&mut b, 0x3C1), expected);
    }
}

#[test]
fn dac_address_is_shared_and_either_address_port_restarts_rgb() {
    let mut b = bus();
    out(&mut b, 0x3C8, 5);
    for value in [10, 20, 30] {
        out(&mut b, 0x3C9, value);
    }
    assert_eq!(input(&mut b, 0x3C8), 6);
    out(&mut b, 0x3C7, 5);
    assert_eq!(input(&mut b, 0x3C8), 5);
    assert_eq!(input(&mut b, 0x3C7), 3);
    assert_eq!(input(&mut b, 0x3C9), 10);
    assert_eq!(input(&mut b, 0x3C8), 5);
    assert_eq!(input(&mut b, 0x3C9), 20);
    out(&mut b, 0x3C8, 9);
    assert_eq!(input(&mut b, 0x3C7), 0);
    out(&mut b, 0x3C9, 7);
    out(&mut b, 0x3C7, 5);
    assert_eq!(input(&mut b, 0x3C8), 5);
    assert_eq!(input(&mut b, 0x3C9), 10);
    out(&mut b, 0x3C8, 10);
    out(&mut b, 0x3C9, 8);
    assert_eq!(card(&mut b).palette()[9], [7, 0, 0]);
    assert_eq!(card(&mut b).palette()[10], [8, 0, 0]);
    out(&mut b, 0x3C7, 5);
    assert_eq!(
        (0..3).map(|_| input(&mut b, 0x3C9)).collect::<Vec<_>>(),
        [10, 20, 30]
    );
    assert_eq!(input(&mut b, 0x3C8), 6);
}

#[test]
fn dac_snapshot_continues_each_shared_read_and_write_component_phase() {
    for read in [false, true] {
        for partial in 0..3 {
            let mut b = bus();
            out(&mut b, 0x3C8, 255);
            for value in [10, 20, 30] {
                out(&mut b, 0x3C9, value);
            }
            out(&mut b, if read { 0x3C7 } else { 0x3C8 }, 255);
            for value in [10, 20, 30].into_iter().take(partial) {
                if read {
                    assert_eq!(input(&mut b, 0x3C9), value);
                } else {
                    out(&mut b, 0x3C9, value);
                }
            }
            let saved = b.device().save();
            let mut restored = bus();
            restored.device_mut().restore(&saved).unwrap();
            for device in [&mut b, &mut restored] {
                assert_eq!(input(device, 0x3C8), 255);
                assert_eq!(input(device, 0x3C7), if read { 3 } else { 0 });
                for value in [10, 20, 30].into_iter().skip(partial) {
                    if read {
                        assert_eq!(input(device, 0x3C9), value);
                    } else {
                        out(device, 0x3C9, value);
                    }
                }
                assert_eq!(input(device, 0x3C8), 0);
                assert_eq!(card(device).palette()[255], [10, 20, 30]);
            }
            assert_eq!(b.device().save(), restored.device().save());
        }
    }
}

#[test]
fn ibm_vga_subsystem_enable_gates_cpu_access_but_preserves_storage() {
    let mut b = bus();
    out(&mut b, 0x3C3, 1);
    planar(&mut b);
    reg(&mut b, 0x3C4, 2, 4);
    b.mmio_write(MEMORY_BASE + 32, 1, 0xA5).unwrap();
    out(&mut b, 0x3C3, 0xFE);
    assert_eq!(input(&mut b, 0x3C3), 0);
    assert_eq!(input(&mut b, 0x3C4), 0xFF);
    assert_eq!(b.mmio_read(MEMORY_BASE + 32, 1).unwrap(), 0xFF);
    reg(&mut b, 0x3C4, 2, 1);
    b.mmio_write(MEMORY_BASE + 32, 1, 0).unwrap();
    out(&mut b, 0x3C3, 0xFF);
    assert_eq!(input(&mut b, 0x3C3), 1);
    assert_eq!(read_reg(&mut b, 0x3C4, 2), 4);
    assert_eq!(bytes(&mut b, 32), [0, 0, 0xA5, 0]);
}

#[test]
fn ibm_vga_adapter_setup_and_enable_are_write_only_and_snapshot_state() {
    let mut b = bus();
    planar(&mut b);
    out(&mut b, 0x46E8, 0x10);
    assert_eq!(input(&mut b, 0x46E8), 0xFF);
    assert_eq!(input(&mut b, 0x3C4), 0xFF);
    let saved = b.device().save();
    let mut restored = bus();
    restored.device_mut().restore(&saved).unwrap();
    for device in [&mut b, &mut restored] {
        out(device, 0x46E8, 0x18);
        assert_eq!(input(device, 0x3C4), 0xFF, "setup suppresses normal decode");
        out(device, 0x46E8, 8);
        assert_eq!(read_reg(device, 0x3C4, 2), 15);
        out(device, 0x46E8, 0);
        assert_eq!(device.mmio_read(MEMORY_BASE, 1).unwrap(), 0xFF);
        out(device, 0x46E8, 8);
    }
    assert_eq!(b.device().save(), restored.device().save());
}

#[test]
fn ibm_vga_colour_monitor_sense_follows_loaded_dac_zero() {
    let mut b = bus();
    out(&mut b, 0x3C3, 1);
    for (rgb, sense) in [
        ([0, 0, 0], 0x10),
        ([0x12; 3], 0x10),
        ([0x14; 3], 0x10),
        ([0x1F; 3], 0x10),
        ([0x20, 0, 0], 0),
        ([0x2D, 0x14, 0x14], 0),
        ([0x14, 0x2D, 0x14], 0),
        ([0x14, 0x14, 0x2D], 0),
        ([0x3F, 0, 0], 0),
        ([0, 0x3F, 0], 0),
        ([0, 0, 0x3F], 0),
    ] {
        out(&mut b, 0x3C8, 0);
        for component in rgb {
            out(&mut b, 0x3C9, component);
        }
        assert_eq!(input(&mut b, 0x3C2) & 0x10, sense);
    }
}

#[test]
fn ibm_vga_retrace_depends_on_virtual_clock_and_not_read_count() {
    let mut b = bus();
    let first = input(&mut b, 0x3BA);
    for _ in 0..8 {
        assert_eq!(input(&mut b, 0x3BA), first);
    }
    let mut states = std::collections::BTreeSet::new();
    for _ in (0..40_000).step_by(31) {
        b.tick(31);
        let state = input(&mut b, 0x3BA) & 9;
        assert_ne!(state, 8, "retrace implies display disable");
        states.insert(state);
    }
    assert_eq!(states, [0, 1, 9].into());
    let mut raster = bus();
    raster.tick(30);
    assert_eq!(input(&mut raster, 0x3BA), 1);
    raster.tick(38 * 412 - 30);
    assert_eq!(input(&mut raster, 0x3BA), 9);
    raster.tick(38 * 2);
    assert_eq!(input(&mut raster, 0x3BA), 1);
    raster.tick(38 * (449 - 414));
    assert_eq!(input(&mut raster, 0x3BA), first);
}

#[test]
fn ibm_vga_font_upload_and_dac_protocol_survive_enable_cycle() {
    let mut b = bus();
    out(&mut b, 0x46E8, 0x08);
    out(&mut b, 0x3C3, 1);
    // IBM VGA/XGA Technical Reference, RAM Loadable Character Generator:
    // sequential addressing, map 2 only, A0000h aperture, write mode zero.
    reg(&mut b, 0x3C4, 0, 1);
    reg(&mut b, 0x3C4, 2, 4);
    reg(&mut b, 0x3C4, 4, 6);
    reg(&mut b, 0x3C4, 0, 3);
    reg(&mut b, 0x3CE, 4, 2);
    reg(&mut b, 0x3CE, 5, 0);
    reg(&mut b, 0x3CE, 6, 4);
    reg(&mut b, 0x3CE, 8, 255);
    for (row, byte) in [0x18, 0x3C, 0x66, 0x7E].into_iter().enumerate() {
        b.mmio_write(MEMORY_BASE + 65 * 32 + row as u64, 1, byte)
            .unwrap();
    }
    assert_eq!(
        &card(&mut b).planes()[2][65 * 32..65 * 32 + 4],
        &[0x18, 0x3C, 0x66, 0x7E]
    );
    assert_eq!(card(&mut b).planes()[0][65 * 32], 0);
    out(&mut b, 0x3C8, 255);
    for byte in [1, 2, 3, 4, 5, 6] {
        out(&mut b, 0x3C9, byte);
    }
    out(&mut b, 0x3C7, 255);
    assert_eq!(input(&mut b, 0x3C7), 3);
    out(&mut b, 0x3C3, 0);
    out(&mut b, 0x3C3, 1);
    assert_eq!(
        (0..6).map(|_| input(&mut b, 0x3C9)).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6]
    );
    assert_eq!(input(&mut b, 0x3C8), 1);
    out(&mut b, 0x3C2, 1);
    reg(&mut b, 0x3D4, 0x0F, 0x42);
    reg(&mut b, 0x3B4, 0x0F, 0x99);
    assert_eq!(read_reg(&mut b, 0x3D4, 0x0F), 0x42);
    assert_eq!(input(&mut b, 0x3BA), 0xFF);
}

#[test]
fn ibm_vga_setup_register_is_accessible_only_in_setup_mode() {
    let mut b = bus();
    out(&mut b, 0x102, 0);
    assert_eq!(input(&mut b, 0x3C3), 1);
    out(&mut b, 0x46E8, 0x10);
    out(&mut b, 0x102, 0);
    out(&mut b, 0x46E8, 8);
    assert_eq!(input(&mut b, 0x3C4), 0xFF);
    out(&mut b, 0x102, 1);
    assert_eq!(input(&mut b, 0x3C4), 0xFF);
    out(&mut b, 0x46E8, 0x10);
    assert_eq!(input(&mut b, 0x102), 0xFF);
    out(&mut b, 0x102, 0xFF);
    out(&mut b, 0x46E8, 8);
    assert_eq!(input(&mut b, 0x3C4), 0);
    out(&mut b, 0x3C3, 0);
    let saved = b.device().save();
    let mut restored = bus();
    restored.device_mut().restore(&saved).unwrap();
    assert_eq!(input(&mut restored, 0x3C3), 0);
    assert_eq!(input(&mut restored, 0x3C4), 0xFF);
    out(&mut restored, 0x3C3, 1);
    assert_eq!(input(&mut restored, 0x3C4), 0);
}

#[test]
fn ibm_vga_programmed_retrace_uses_crtc_totals_clock_and_overflow() {
    let mut b = bus();
    out(&mut b, 0x3C3, 1);
    reg(&mut b, 0x3C4, 1, 1);
    reg(&mut b, 0x3B4, 0, 5);
    reg(&mut b, 0x3B4, 1, 4);
    reg(&mut b, 0x3B4, 6, 8);
    reg(&mut b, 0x3B4, 0x12, 4);
    reg(&mut b, 0x3B4, 0x10, 7);
    reg(&mut b, 0x3B4, 0x11, 9);
    let tick_for_dot = |dot: u64, clock: device::clock::ClockDomain| clock.edge_at(dot).0;
    let mut at = |dot, expected| {
        let target = tick_for_dot(dot, VGA_25);
        b.tick(target - b.clock().0);
        assert_eq!(input(&mut b, 0x3BA) & 9, expected, "dot {dot}");
    };
    at(0, 0);
    at(40, 1);
    at(80, 0);
    at(560, 9);
    at(720, 1);
    at(800, 0);
    // Overflow extends retrace to line 263, with a 266-line frame.
    reg(&mut b, 0x3B4, 7, 5);
    let target = tick_for_dot(263 * 80, VGA_25);
    b.tick(target - b.clock().0);
    assert_eq!(input(&mut b, 0x3BA) & 9, 9);
    out(&mut b, 0x3C2, 4);
    let frame_dots = 266 * 80;
    let target = tick_for_dot(2 * frame_dots + 263 * 80, VGA_28);
    b.tick(target - b.clock().0);
    assert_eq!(input(&mut b, 0x3BA) & 9, 9);
    for (sequencer, dots_per_char, frame) in [(0, 9, 3), (8, 18, 4)] {
        reg(&mut b, 0x3C4, 1, sequencer);
        let target = tick_for_dot((frame * 266 + 263) * 10 * dots_per_char, VGA_28);
        b.tick(target - b.clock().0);
        assert_eq!(input(&mut b, 0x3BA) & 9, 9);
    }
}

#[test]
fn enable_register_observation_is_passive_and_keeps_write_only_latches() {
    let mut b = bus();
    out(&mut b, 0x46E8, 0x18);
    out(&mut b, 0x102, 0);
    out(&mut b, 0x3C3, 0);
    let saved = b.device().save();
    assert_eq!(card(&mut b).subsystem_enable(), 0);
    assert_eq!(card(&mut b).adapter_control(), 0x18);
    assert_eq!(card(&mut b).setup_enable(), 0);
    assert_eq!(b.device().save(), saved);
    assert_eq!(input(&mut b, 0x46E8), 0xFF);
    assert_eq!(input(&mut b, 0x102), 0xFF);
    out(&mut b, 0x3C3, 1);
    out(&mut b, 0x102, 1);
    out(&mut b, 0x46E8, 8);
    let saved = b.device().save();
    assert_eq!(card(&mut b).subsystem_enable(), 1);
    assert_eq!(card(&mut b).adapter_control(), 8);
    assert_eq!(card(&mut b).setup_enable(), 1);
    assert_eq!(b.device().save(), saved);
}

#[test]
fn diagnostic_state_preserves_every_ac_and_dac_phase_without_guest_accesses() {
    for read_mode in [false, true] {
        for component in 0..3 {
            for expects_data in [false, true] {
                let mut b = bus();
                planar(&mut b);
                seed(&mut b, 0, [0x12, 0x34, 0x56, 0x78]);
                b.mmio_read(MEMORY_BASE, 1).unwrap();
                out(&mut b, 0x3C8, 7);
                for byte in [11, 22, 33] {
                    out(&mut b, 0x3C9, byte);
                }
                out(&mut b, if read_mode { 0x3C7 } else { 0x3C8 }, 7);
                for _ in 0..component {
                    input(&mut b, 0x3C9);
                }
                out(&mut b, 0x3C0, 0x33);
                if !expects_data {
                    out(&mut b, 0x3C0, 5);
                }
                b.device_mut()
                    .set_trace_policy(device::TracePolicy::Ring { capacity: 32 });
                let before = b.device().save();
                let first = card(&mut b).diagnostic_state();
                assert_eq!(first, card(&mut b).diagnostic_state());
                assert_eq!(before, b.device().save());
                assert!(b.device_mut().take_trace().is_empty());
                assert_eq!(first.attribute_address, 0x33);
                assert_eq!(first.attribute_expects_data, expects_data);
                assert_eq!(first.dac_read_mode, read_mode);
                assert_eq!((first.dac_address, first.dac_component), (7, component));
                assert_eq!(first.dac[7], [11, 22, 33]);
                assert_eq!(input(&mut b, 0x3C0), 0x33);
                assert_eq!(input(&mut b, 0x3C9), [11, 22, 33][usize::from(component)]);
                out(&mut b, 0x3C0, 9);
                assert_eq!(
                    card(&mut b).attribute()[0x13],
                    if expects_data { 9 } else { 5 }
                );
            }
        }
    }
}

#[test]
fn diagnostic_shown_colours_follow_scanout_plane_enable_p54s_select_and_pel_mask() {
    let mut b = bus();
    out(&mut b, 0x3C0, 5);
    out(&mut b, 0x3C0, 0x2E);
    out(&mut b, 0x3C0, 0x12);
    out(&mut b, 0x3C0, 7);
    out(&mut b, 0x3C0, 0x14);
    out(&mut b, 0x3C0, 0x0D);
    out(&mut b, 0x3C6, 0xDF);
    for (mode, expected) in [(0x01, 0xCE), (0x81, 0xDE), (0x80, 0xDE)] {
        out(&mut b, 0x3C0, 0x10);
        out(&mut b, 0x3C0, mode);
        out(&mut b, 0x3C8, expected);
        for value in [13, 27, 42] {
            out(&mut b, 0x3C9, value);
        }
        let state = card(&mut b).diagnostic_state();
        assert_eq!(state.shown[13], (expected, [13, 27, 42]));
        let expanded = [52, 109, 170];
        if mode & 1 == 0 {
            assert_eq!(
                b.device().planar_text_scanout().unwrap().palette[13],
                expanded
            );
        } else {
            let scanout = b.device().graphics_scanout().unwrap();
            assert_eq!(scanout.colours[13] & state.pel_mask, expected);
            assert_eq!(scanout.palette[usize::from(scanout.colours[13])], expanded);
        }
    }
}

#[test]
fn attribute_palette_writes_accept_both_pas_values_without_changing_protocol() {
    for pas in [0, 0x20] {
        let mut b = bus();
        out(&mut b, 0x3C6, 0xFF);
        out(&mut b, 0x3C0, 0x12);
        out(&mut b, 0x3C0, 15);
        out(&mut b, 0x3C8, 0x2E);
        for value in [13, 27, 42] {
            out(&mut b, 0x3C9, value);
        }
        for index in 0..16 {
            input(&mut b, 0x3BA);
            out(&mut b, 0x3C0, index | pas);
            assert!(card(&mut b).diagnostic_state().attribute_expects_data);
            assert_eq!(input(&mut b, 0x3C0), index | pas);
            input(&mut b, 0x3C1);
            assert!(card(&mut b).diagnostic_state().attribute_expects_data);
            out(&mut b, 0x3C0, 0xEE);
            let state = card(&mut b).diagnostic_state();
            assert!(!state.attribute_expects_data);
            assert_eq!(state.attribute_address, index | pas);
            assert_eq!(card(&mut b).attribute()[usize::from(index)], 0x2E);
            assert_eq!(input(&mut b, 0x3C1), 0x2E);
            assert_eq!(state.shown[usize::from(index)], (0x2E, [13, 27, 42]));
            let scanout = b.device().planar_text_scanout().unwrap();
            assert_eq!(scanout.palette[usize::from(index)], [52, 109, 170]);
            assert_eq!(scanout.enabled, pas != 0);
        }
    }
}
