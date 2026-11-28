// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bochs VBE DISPI interface (`vbe.h`) and the "VBE Display API" document:
//! every register through the VGA's bus faces on the mock bus.

use super::*;
use crate::Vga;
use testkit::MockBus;

fn card(memory_mib: u32) -> (MockBus, BochsVbe) {
    let vga = Vga::with_bochs_vbe(memory_mib).unwrap();
    let vbe = vga.bochs_vbe().unwrap().clone();
    (MockBus::new(Box::new(vga)), vbe)
}

fn write(bus: &mut MockBus, index: u16, value: u16) {
    bus.port_write(INDEX_PORT, 2, u32::from(index)).unwrap();
    bus.port_write(DATA_PORT, 2, u32::from(value)).unwrap();
}

fn read(bus: &mut MockBus, index: u16) -> u16 {
    bus.port_write(INDEX_PORT, 2, u32::from(index)).unwrap();
    bus.port_read(DATA_PORT, 2).unwrap() as u16
}

fn mode(bus: &mut MockBus, xres: u16, yres: u16, bpp: u16, enable: u16) {
    write(bus, 4, 0);
    write(bus, 3, bpp);
    write(bus, 1, xres);
    write(bus, 2, yres);
    write(bus, 4, ENABLED | enable);
}

fn vga(bus: &mut MockBus) -> &mut Vga {
    bus.device_mut()
        .as_any_mut()
        .unwrap()
        .downcast_mut()
        .unwrap()
}

#[test]
fn id_register_selects_interface_versions_and_ignores_other_values() {
    let (mut bus, _) = card(16);
    assert_eq!(read(&mut bus, 0), 0xB0C5);
    for id in 0xB0C0..=0xB0C5 {
        write(&mut bus, 0, id);
        assert_eq!(read(&mut bus, 0), id);
    }
    write(&mut bus, 0, 0xB0C6);
    write(&mut bus, 0, 0x1234);
    assert_eq!(read(&mut bus, 0), 0xB0C5);
}

#[test]
fn ports_accept_byte_word_and_index_data_doubleword_accesses() {
    let (mut bus, vbe) = card(16);
    bus.port_write(INDEX_PORT, 1, 0x0A).unwrap();
    assert_eq!(bus.port_read(INDEX_PORT, 2).unwrap(), 0x0A);
    assert_eq!(bus.port_read(DATA_PORT, 2).unwrap(), 256);
    assert_eq!(bus.port_read(DATA_ALIAS_PORT, 2).unwrap(), 256);
    assert_eq!(bus.port_read(DATA_PORT, 1).unwrap(), 0x00, "low byte");
    assert_eq!(
        bus.port_read(DATA_ALIAS_PORT, 1).unwrap(),
        0x01,
        "high byte"
    );
    bus.port_write(INDEX_PORT, 4, 0x0320_0001).unwrap();
    assert_eq!(vbe.registers().xres, 800);
    assert_eq!(bus.port_read(INDEX_PORT, 4).unwrap(), 0x0320_0001);
    bus.port_write(DATA_ALIAS_PORT, 2, 640).unwrap();
    assert_eq!(vbe.registers().xres, 640);

    // A lone byte at the data port writes zero-extended at the next access;
    // a lone byte at the alias writes the high byte.
    bus.port_write(INDEX_PORT, 2, 2).unwrap();
    bus.port_write(DATA_PORT, 1, 0x1C8).unwrap();
    assert_eq!(bus.port_read(DATA_PORT, 2).unwrap(), 0xC8);
    bus.port_write(DATA_ALIAS_PORT, 1, 0x01).unwrap();
    assert_eq!(vbe.registers().yres, 0x100);
    assert!(bus.port_write(DATA_ALIAS_PORT, 4, 0).is_err());
}

#[test]
fn an_isa_split_data_word_writes_once() {
    let (mut bus, vbe) = card(16);
    mode(&mut bus, 640, 480, 8, NO_CLEAR_MEM);
    write(&mut bus, 6, 1024);
    write(&mut bus, 8, 300);
    // OUT 1CFh, AX on ISA: the low byte at 1CFh, then the high byte at 1D0h.
    // Writing the low byte alone would narrow the virtual screen to XRES and
    // clamp the offset to zero.
    bus.port_write(INDEX_PORT, 2, 6).unwrap();
    bus.port_write(DATA_PORT, 1, 0x00).unwrap();
    bus.port_write(DATA_ALIAS_PORT, 1, 0x04).unwrap();
    assert_eq!([read(&mut bus, 6), read(&mut bus, 8)], [1024, 300]);
    assert_eq!(vbe.registers().virt_width, 1024);
    // IN AX, 1CFh on ISA: the low byte, then the high byte.
    bus.port_write(INDEX_PORT, 2, 6).unwrap();
    let low = bus.port_read(DATA_PORT, 1).unwrap();
    let high = bus.port_read(DATA_ALIAS_PORT, 1).unwrap();
    assert_eq!(low | high << 8, 1024);
}

#[test]
fn plain_vga_does_not_decode_the_dispi_ports() {
    let mut bus = MockBus::new(Box::new(Vga::new()));
    assert!(bus.port_write(INDEX_PORT, 2, 0).is_err());
    assert!(bus.port_read(DATA_PORT, 2).is_err());
    assert!(Vga::new().bochs_vbe().is_none());
}

#[test]
fn getcaps_reads_the_maxima_until_cleared() {
    let (mut bus, _) = card(16);
    write(&mut bus, 1, 1024);
    write(&mut bus, 4, GETCAPS);
    assert_eq!(
        [read(&mut bus, 1), read(&mut bus, 2), read(&mut bus, 3)],
        [MAX_XRES, MAX_YRES, MAX_BPP]
    );
    write(&mut bus, 4, 0);
    assert_eq!([read(&mut bus, 1), read(&mut bus, 3)], [1024, 8]);
}

#[test]
fn geometry_writes_clamp_and_refuse_unsupported_values() {
    let (mut bus, _) = card(16);
    write(&mut bus, 1, 4000);
    write(&mut bus, 2, 2000);
    assert_eq!([read(&mut bus, 1), read(&mut bus, 2)], [MAX_XRES, MAX_YRES]);
    for bpp in [15, 16, 24, 32, 8] {
        write(&mut bus, 3, bpp);
        assert_eq!(read(&mut bus, 3), bpp);
    }
    for refused in [4, 1, 7, 12, 33] {
        write(&mut bus, 3, refused);
        assert_eq!(read(&mut bus, 3), 8, "bpp {refused}");
    }
    write(&mut bus, 3, 16);
    write(&mut bus, 3, 0);
    assert_eq!(read(&mut bus, 3), 8, "zero means eight");
    write(&mut bus, 4, 0x1F);
    assert_eq!(
        read(&mut bus, 4),
        ENABLED | GETCAPS,
        "reserved bits read zero"
    );

    mode(&mut bus, 640, 480, 16, NO_CLEAR_MEM);
    write(&mut bus, 1, 800);
    write(&mut bus, 2, 600);
    write(&mut bus, 3, 32);
    assert_eq!(
        [read(&mut bus, 1), read(&mut bus, 2), read(&mut bus, 3)],
        [640, 480, 16],
        "mode registers are read-only while enabled"
    );
    write(&mut bus, 5, 255);
    assert_eq!(read(&mut bus, 5), 255);
    write(&mut bus, 5, 256);
    assert_eq!(read(&mut bus, 5), 255, "a bank past memory is ignored");
}

#[test]
fn virtual_width_and_height_follow_memory_and_offsets_stay_inside() {
    let (mut bus, vbe) = card(4);
    mode(&mut bus, 640, 480, 32, NO_CLEAR_MEM);
    assert_eq!(read(&mut bus, 6), 640);
    assert_eq!(read(&mut bus, 7), ((4u32 << 20) / 2560) as u16);
    write(&mut bus, 6, 1024);
    assert_eq!([read(&mut bus, 6), read(&mut bus, 7)], [1024, 1024]);
    write(&mut bus, 6, 100);
    assert_eq!(read(&mut bus, 6), 640, "never narrower than XRES");
    write(&mut bus, 6, 4000);
    assert_eq!(
        [read(&mut bus, 6), read(&mut bus, 7)],
        [2184, 480],
        "the widest virtual screen whose XRES lines fit 4 MiB"
    );
    write(&mut bus, 7, 9);
    assert_eq!(read(&mut bus, 7), 480, "VIRT_HEIGHT is read-only");
    write(&mut bus, 6, 1024);
    write(&mut bus, 8, 500);
    write(&mut bus, 9, 2000);
    assert_eq!(
        [read(&mut bus, 8), read(&mut bus, 9)],
        [1024 - 640, 1024 - 480]
    );
    let scanout = vbe.linear_scanout().unwrap();
    assert_eq!(
        scanout,
        LinearScanout {
            offset: (544 * 1024 + 384) * 4,
            width: 640,
            height: 480,
            bits_per_pixel: 32,
            stride: 4096,
        }
    );
    assert_eq!(scanout.offset + scanout.byte_len(), 4 << 20);
    // Re-enabling restores the default virtual screen and clears the offsets.
    write(&mut bus, 4, 0);
    write(&mut bus, 4, ENABLED | NO_CLEAR_MEM);
    assert_eq!(
        [read(&mut bus, 6), read(&mut bus, 8), read(&mut bus, 9)],
        [640, 0, 0]
    );
}

#[test]
fn video_memory_register_reports_the_size_and_ignores_writes() {
    for (mib, units) in [(4, 64), (8, 128), (16, 256)] {
        let (mut bus, vbe) = card(mib);
        assert_eq!(read(&mut bus, 0x0A), units);
        write(&mut bus, 0x0A, 1);
        assert_eq!(read(&mut bus, 0x0A), units);
        assert_eq!(vbe.memory_bytes(), u64::from(mib) << 20);
    }
    for refused in [0, 1, 2, 12, 32] {
        assert!(Vga::with_bochs_vbe(refused).is_err());
    }
    let (mut bus, _) = card(16);
    assert_eq!(read(&mut bus, 0x0B), 0);
    assert_eq!(read(&mut bus, 0xFFFF), 0);
}

#[test]
fn enabling_clears_video_memory_unless_no_clear_memory_is_set() {
    let (mut bus, vbe) = card(16);
    assert_eq!(vbe.clears(), 0);
    mode(&mut bus, 640, 480, 16, 0);
    assert_eq!(vbe.clears(), 1);
    // Enabled again: no transition, no clear.
    write(&mut bus, 4, ENABLED);
    assert_eq!(vbe.clears(), 1);
    mode(&mut bus, 1024, 768, 32, NO_CLEAR_MEM);
    assert_eq!(vbe.clears(), 1);
    mode(&mut bus, 1024, 768, 32, LFB_ENABLED);
    assert_eq!(vbe.clears(), 2);
}

#[test]
fn bank_window_names_video_memory_only_while_enabled() {
    let (mut bus, vbe) = card(16);
    assert_eq!(vbe.window_bank(), None);
    assert_eq!(vbe.window_target(0xA0000), None);
    // Map mask and bit mask open, so the planar path stores the byte.
    bus.port_write(0x3C4, 2, 0x0F02).unwrap();
    bus.port_write(0x3CE, 2, 0xFF08).unwrap();
    bus.mmio_write(0xA0000, 1, 0x77).unwrap();
    assert_eq!(vga(&mut bus).planes()[0][0], 0x77);

    mode(&mut bus, 800, 600, 8, 0);
    write(&mut bus, 5, 3);
    assert_eq!(vbe.window_bank(), Some(3));
    assert_eq!(vbe.window_target(0xA0000), Some(LFB_BASE + 0x3_0000));
    assert_eq!(vbe.window_target(0xAFFFF), Some(LFB_BASE + 0x3_FFFF));
    assert_eq!(vbe.window_target(0xB0000), None);
    assert_eq!(vbe.window_target(0x9FFFF), None);
    write(&mut bus, 5, 255);
    assert_eq!(vbe.window_target(0xA1234), Some(LFB_BASE + 0xFF_1234));
    // The machine serves the window; what still reaches the card floats.
    assert_eq!(bus.mmio_read(0xA0000, 2).unwrap(), 0xFFFF);
    assert_eq!(bus.mmio_read(0xB8000, 2).unwrap(), 0xFFFF);
    bus.mmio_write(0xA0000, 1, 1).unwrap();
    assert_eq!(vga(&mut bus).planes()[0][0], 0x77, "planes untouched");

    write(&mut bus, 4, 0);
    assert_eq!(vbe.window_target(0xA0000), None);
    assert_eq!(bus.mmio_read(0xA0000, 1).unwrap(), 0x77);
}

#[test]
fn eight_bit_dac_keeps_whole_components() {
    let (mut bus, _) = card(16);
    bus.port_write(0x3C8, 1, 5).unwrap();
    for value in [0xC8, 0x41, 0x3F] {
        bus.port_write(0x3C9, 1, value).unwrap();
    }
    bus.port_write(0x3C7, 1, 5).unwrap();
    assert_eq!(bus.port_read(0x3C9, 1).unwrap(), 0x08);
    write(&mut bus, 4, DAC_8BIT);
    bus.port_write(0x3C8, 1, 5).unwrap();
    for value in [0xC8, 0x41, 0x3F] {
        bus.port_write(0x3C9, 1, value).unwrap();
    }
    bus.port_write(0x3C7, 1, 5).unwrap();
    let back: Vec<u32> = (0..3).map(|_| bus.port_read(0x3C9, 1).unwrap()).collect();
    assert_eq!(back, [0xC8, 0x41, 0x3F]);
    bus.port_write(0x3C6, 1, 0xFF).unwrap();
    mode(&mut bus, 640, 480, 8, DAC_8BIT | NO_CLEAR_MEM);
    let scanout = bus.device().graphics_scanout().unwrap();
    assert_eq!(scanout.palette[5], [0xC8, 0x41, 0x3F]);
    assert!(!scanout.enabled, "DISPI blanks the VGA serializers");
    assert!(bus.device().planar_text_scanout().is_none());
    write(&mut bus, 4, ENABLED | NO_CLEAR_MEM);
    let scanout = bus.device().graphics_scanout().unwrap();
    assert_eq!(scanout.palette[5], [0x20, 0x04, 0xFF], "six-bit display");
}

#[test]
fn retrace_keeps_toggling_in_dispi_modes() {
    let (mut bus, _) = card(16);
    mode(&mut bus, 1024, 768, 32, NO_CLEAR_MEM);
    bus.port_write(0x3C2, 1, 1).unwrap();
    let mut seen = [false; 2];
    for _ in 0..2000 {
        bus.tick(10);
        let status = bus.port_read(0x3DA, 1).unwrap();
        seen[usize::from(status & 8 != 0)] = true;
    }
    assert_eq!(seen, [true, true]);
}

#[test]
fn snapshot_restores_registers_and_reset_keeps_the_memory_size() {
    let (mut bus, vbe) = card(8);
    mode(&mut bus, 800, 600, 24, DAC_8BIT | NO_CLEAR_MEM);
    write(&mut bus, 6, 1000);
    write(&mut bus, 9, 7);
    bus.port_write(0x3C8, 1, 0).unwrap();
    bus.port_write(0x3C9, 1, 0xFE).unwrap();
    let saved = bus.device().save();
    let registers = vbe.registers();

    bus.device_mut().reset();
    assert_eq!(vbe.registers().enable, 0);
    assert_eq!(read(&mut bus, 0x0A), 128);
    bus.device_mut().restore(&saved).unwrap();
    assert_eq!(vbe.registers(), registers);
    assert_eq!(bus.device().save(), saved);

    let mut corrupt = saved.clone();
    let at = corrupt.len() - 2 * 7;
    corrupt[at] = 4; // BPP 4
    assert!(bus.device_mut().restore(&corrupt).is_err());
    assert!(bus
        .device_mut()
        .restore(&saved[..saved.len() - SNAPSHOT_LEN])
        .is_err());
    let (mut other, _) = card(16);
    assert!(other.device_mut().restore(&saved).is_err(), "memory size");
    assert!(MockBus::new(Box::new(Vga::new()))
        .device_mut()
        .restore(&saved)
        .is_err());
    assert_eq!(vbe.registers(), registers);
}
