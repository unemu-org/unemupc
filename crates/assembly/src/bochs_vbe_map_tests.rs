// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bochs VBE DISPI interface (`vbe.h`): CPU-held video memory, the enable
//! clear, the banked window's target and linear-framebuffer frames on a 486
//! board.

use super::*;
use crate::{
    BoardModel, BochsVbeCfg, ConfigError, CpuChip, DeviceConfig, DeviceId, DeviceKind, FpuChip,
    MachineConfig, MmioWindow, RamRegion,
};
use cpu::{Cpu, DmaAccess};
use device::Device;
use devices::bochs_vbe::{DATA_PORT, ENABLED, INDEX_PORT, NO_CLEAR_MEM};
use devices::Vga;
use video::Frame;

use crate::observation::linear_frame;

pub(crate) fn board(memory_mib: u32) -> MachineConfig {
    let mut config = crate::bios(vec![0xF4; 0x20000], None);
    config.images[0].base = 0xE0000;
    config.board = BoardModel::Generic486Isa;
    config.clock = config.board.default_clock();
    config.cpu.chip = CpuChip::I80486Dx;
    config.cpu.fpu = FpuChip::Integrated;
    let uart = config
        .devices
        .iter()
        .find(|d| d.id == DeviceId::Uart)
        .unwrap()
        .clone();
    config.devices = config.board.motherboard_devices();
    config.devices.push(uart);
    config.devices.push(DeviceConfig::new(
        DeviceId::Vga,
        DeviceKind::BochsVbe(BochsVbeCfg { memory_mib }),
        BochsVbeCfg::port_windows(),
    ));
    config.drives.clear();
    config.memory.address_space_bytes = 0x1000000;
    config.memory.mmio = vec![MmioWindow::new(0xA0000, 0x20000, DeviceId::Vga)];
    config.memory.ram = vec![
        RamRegion::new(0, 0xA0000),
        RamRegion::new(0xC0000, 0x40000),
        RamRegion::new(0x100000, 15 << 20),
    ];
    config
}

struct Rig {
    cpu: X86JitCpu,
    vga: Vga,
    vbe: BochsVbe,
    clear: BochsVbeClear,
}

impl Rig {
    fn new(memory_mib: u32) -> Self {
        let cpu = crate::test_support::build_cpu(&board(memory_mib)).unwrap();
        let vga = Vga::with_bochs_vbe(memory_mib).unwrap();
        let vbe = vga.bochs_vbe().unwrap().clone();
        let mut rig = Self {
            cpu,
            vga,
            clear: BochsVbeClear::new(vbe.clone()),
            vbe,
        };
        rig.sync(MemoryMapSync::Apply);
        rig
    }

    fn sync(&mut self, event: MemoryMapSync) {
        self.clear.sync(&mut self.cpu, event).unwrap();
    }

    /// A guest DISPI write, followed by the machine's memory-map apply.
    fn dispi(&mut self, index: u16, value: u16) {
        self.vga
            .port_write(INDEX_PORT, 2, u32::from(index))
            .unwrap();
        self.vga.port_write(DATA_PORT, 2, u32::from(value)).unwrap();
        self.sync(MemoryMapSync::Apply);
    }

    fn mode(&mut self, xres: u16, yres: u16, bpp: u16, enable: u16) {
        self.dispi(4, 0);
        self.dispi(3, bpp);
        self.dispi(1, xres);
        self.dispi(2, yres);
        self.dispi(4, ENABLED | enable);
    }

    fn lfb(&self, offset: u64, len: usize) -> Vec<u8> {
        let mut bytes = vec![0; len];
        self.cpu.read_mem(LFB_BASE + offset, &mut bytes).unwrap();
        bytes
    }

    fn fill(&mut self, offset: u64, bytes: &[u8]) {
        self.cpu.write_mem(LFB_BASE + offset, bytes).unwrap();
    }
}

#[test]
fn video_memory_is_cpu_ram_at_the_linear_framebuffer_address() {
    for memory_mib in [4, 8, 16] {
        let config = board(memory_mib);
        assert_eq!(config.check_bochs_vbe_resources(), Ok(()));
        let mut regions = Vec::new();
        let mut cpu =
            crate::test_support::build_cpu_with_mapping_trace(&config, |base, end, kind| {
                regions.push((base, end, kind));
            })
            .unwrap();
        let end = LFB_BASE + (u64::from(memory_mib) << 20);
        assert!(regions.contains(&(LFB_BASE, end, "lfb")), "{regions:?}");
        assert_eq!(
            cpu.guest_dma_write(0xE000_1234, 0x5A),
            DmaAccess::Complete(())
        );
        assert_eq!(cpu.guest_dma_read(0xE000_1234), DmaAccess::Complete(0x5A));
        assert_eq!(
            cpu.guest_dma_read(end as u32 - 1),
            DmaAccess::Complete(0),
            "the last byte is mapped"
        );
    }
}

#[test]
fn the_card_needs_a_32_bit_bus_its_memory_sizes_and_free_address_space() {
    let mut at = board(16);
    at.board = BoardModel::Ibm5170;
    at.cpu.chip = CpuChip::I80286;
    assert_eq!(
        at.check_bochs_vbe_resources(),
        Err(ConfigError::BochsVbeAddressSpace {
            device: DeviceId::Vga,
            board: "ibm-5170",
            bits: 24
        })
    );
    let mut bare = board(16);
    bare.board = BoardModel::Bare;
    bare.cpu.chip = CpuChip::I80286;
    assert!(matches!(
        bare.check_bochs_vbe_resources(),
        Err(ConfigError::BochsVbeAddressSpace { bits: 24, .. })
    ));
    for memory_mib in [0, 2, 12, 32] {
        assert_eq!(
            board(memory_mib).check_bochs_vbe_resources(),
            Err(ConfigError::InvalidBochsVbeResources {
                device: DeviceId::Vga
            })
        );
    }
    let mut ports = board(16);
    ports.devices.last_mut().unwrap().ports = crate::VgaCfg::port_windows();
    assert!(ports.check_bochs_vbe_resources().is_err());
    let mut overlap = board(4);
    overlap
        .memory
        .ram
        .push(RamRegion::new(LFB_BASE + 0x30_0000, 0x10_0000));
    assert_eq!(
        overlap.check_bochs_vbe_resources(),
        Err(ConfigError::BochsVbeOverlap {
            device: DeviceId::Vga,
            addr: LFB_BASE + 0x30_0000
        })
    );
    let mut mmio = board(16);
    mmio.memory.mmio.push(MmioWindow::new(
        LFB_BASE - 0x1000,
        0x2000,
        DeviceId::LotechEms,
    ));
    assert_eq!(
        mmio.check_bochs_vbe_resources(),
        Err(ConfigError::BochsVbeOverlap {
            device: DeviceId::Vga,
            addr: LFB_BASE
        })
    );
    let mut rom = board(16);
    rom.images.push(crate::LoadImage {
        base: LFB_BASE + 0xF0_0000,
        source: crate::ImageSource::Inline(vec![0; 0x1000]),
        kind: crate::ImageKind::Rom,
        provides: Vec::new(),
    });
    assert_eq!(
        rom.check_bochs_vbe_resources(),
        Err(ConfigError::BochsVbeOverlap {
            device: DeviceId::Vga,
            addr: LFB_BASE + 0xF0_0000
        })
    );
}

#[test]
fn enabling_clears_the_whole_video_memory() {
    let mut rig = Rig::new(4);
    let memory = 4 << 20;
    rig.fill(0, &vec![0xEE; memory]);
    rig.mode(640, 480, 16, 0);
    assert!(rig.lfb(0, memory).iter().all(|&b| b == 0));

    rig.fill(memory as u64 - 16, &[0x11; 16]);
    rig.mode(320, 200, 32, NO_CLEAR_MEM);
    assert_eq!(rig.lfb(memory as u64 - 16, 16), vec![0x11; 16]);
    // Only the port write that enabled requests the clear; a flush and a
    // later run start leave memory alone.
    rig.sync(MemoryMapSync::Flush);
    rig.sync(MemoryMapSync::Apply);
    assert_eq!(rig.lfb(memory as u64 - 16, 16), vec![0x11; 16]);
}

#[test]
fn the_banked_window_targets_the_selected_bank_of_video_memory() {
    let mut rig = Rig::new(16);
    let (window, target) = window(rig.vbe.clone());
    assert_eq!(window, 0xA0000..=0xAFFFF);
    assert_eq!(target(0xA0010), None, "disabled: the VGA serves it");
    rig.mode(800, 600, 8, NO_CLEAR_MEM);
    rig.dispi(5, 7);
    assert_eq!(target(0xA0010), Some(LFB_BASE + 7 * 0x1_0000 + 0x10));
    rig.dispi(5, 0);
    assert_eq!(target(0xAFFFF), Some(LFB_BASE + 0xFFFF));
}

/// The guest bytes of a five-pixel-wide virtual screen of three lines.
fn virtual_screen(bytes_per_pixel: usize, pixel: impl Fn(usize) -> Vec<u8>) -> Vec<u8> {
    let bytes: Vec<u8> = (0..15).flat_map(pixel).collect();
    assert_eq!(bytes.len(), 15 * bytes_per_pixel);
    bytes
}

#[test]
fn linear_frames_pan_a_virtual_screen_in_every_direct_format() {
    let expand5 = |v: u32| (v << 3) | (v >> 2);
    let expand6 = |v: u32| (v << 2) | (v >> 4);
    type Case = (
        u16,
        video::PixelFormat,
        usize,
        Box<dyn Fn(usize) -> (Vec<u8>, u32)>,
    );
    let cases: [Case; 4] = [
        (
            15,
            video::PixelFormat::Rgb555,
            2,
            Box::new(move |n| {
                let (r, g, b) = (n as u32, 31 - n as u32, (n as u32 * 2) & 31);
                let word = (r << 10 | g << 5 | b) as u16;
                (
                    word.to_le_bytes().to_vec(),
                    expand5(r) << 16 | expand5(g) << 8 | expand5(b),
                )
            }),
        ),
        (
            16,
            video::PixelFormat::Rgb565,
            2,
            Box::new(move |n| {
                let (r, g, b) = (n as u32, 63 - n as u32 * 3, 31 - n as u32);
                let word = (r << 11 | g << 5 | b) as u16;
                (
                    word.to_le_bytes().to_vec(),
                    expand5(r) << 16 | expand6(g) << 8 | expand5(b),
                )
            }),
        ),
        (
            24,
            video::PixelFormat::Rgb888,
            3,
            Box::new(|n| {
                let (r, g, b) = (n as u8 * 10, n as u8 + 1, 255 - n as u8);
                (
                    vec![b, g, r],
                    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
                )
            }),
        ),
        (
            32,
            video::PixelFormat::Xrgb8888,
            4,
            Box::new(|n| {
                let (r, g, b) = (n as u8 * 10, n as u8 + 1, 255 - n as u8);
                (
                    vec![b, g, r, 0xFF],
                    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
                )
            }),
        ),
    ];
    for (bpp, format, bytes_per_pixel, pixel) in cases {
        let mut rig = Rig::new(4);
        rig.mode(3, 2, bpp, NO_CLEAR_MEM);
        rig.dispi(6, 5);
        rig.dispi(8, 2);
        rig.dispi(9, 1);
        let screen = virtual_screen(bytes_per_pixel, |n| pixel(n).0);
        rig.fill(0, &screen);
        let scanout = rig.vbe.linear_scanout().unwrap();
        let Some(Frame::Direct(frame)) = linear_frame(&scanout, &rig.cpu, &[[0; 3]; 256]) else {
            panic!("{bpp} bpp must give a direct-colour frame");
        };
        assert_eq!(
            (frame.width, frame.height, frame.repeat, frame.format),
            (3, 2, (1, 1), format)
        );
        assert_eq!(frame.stride as usize, 5 * bytes_per_pixel);
        let start = (5 + 2) * bytes_per_pixel;
        assert_eq!(
            frame.bytes,
            screen[start..start + (5 + 3) * bytes_per_pixel]
        );
        let expected: Vec<u32> = [7, 8, 9, 12, 13, 14].map(|n| pixel(n).1).to_vec();
        assert_eq!(frame.to_xrgb8888().unwrap(), expected, "{bpp} bpp");
    }
}

#[test]
fn eight_bit_linear_frames_index_the_dac_palette() {
    let mut rig = Rig::new(4);
    rig.mode(3, 2, 8, NO_CLEAR_MEM);
    rig.dispi(6, 5);
    rig.dispi(8, 1);
    rig.dispi(9, 1);
    rig.fill(0, &(0..15).collect::<Vec<u8>>());
    let mut palette = [[0; 3]; 256];
    palette[6] = [1, 2, 3];
    let scanout = rig.vbe.linear_scanout().unwrap();
    let Some(Frame::Pixels(frame)) = linear_frame(&scanout, &rig.cpu, &palette) else {
        panic!("8 bpp must give an indexed frame");
    };
    assert_eq!((frame.width, frame.height, frame.repeat), (3, 2, (1, 1)));
    assert_eq!(frame.indices, [6, 7, 8, 11, 12, 13]);
    assert_eq!(frame.palette[6], [1, 2, 3]);
}
