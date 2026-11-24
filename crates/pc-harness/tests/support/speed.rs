// SPDX-License-Identifier: AGPL-3.0-or-later
//! Project-owned speed probes on complete motherboards. Intel 8254 datasheet,
//! Mode 2; see `backlog/docs/design/machine-files.md` §Nominal CPU clocks.

use assembly::{BoardModel, CpuChip, FpuChip, MachineConfig, RamRegion};

pub fn board(rom: &[u8], board: BoardModel) -> MachineConfig {
    let mut config = assembly::bios(rom.to_vec(), None);
    config.board = board;
    if board.is_generic_isa() {
        let mut image = vec![0; 0x18000];
        image.extend_from_slice(rom);
        config.images[0].base = 0xe0000;
        config.images[0].source = assembly::ImageSource::Inline(image);
    }
    config.cpu.chip = match board {
        BoardModel::Ibm5170 | BoardModel::Bare => CpuChip::I80286,
        BoardModel::Generic386Isa => CpuChip::I80386,
        BoardModel::Generic486Isa => CpuChip::I80486Dx,
    };
    config.cpu.fpu = if board == BoardModel::Generic486Isa {
        FpuChip::Integrated
    } else {
        FpuChip::None
    };
    config.clock = board.default_clock();
    let uart = config
        .devices
        .iter()
        .find(|d| d.id == assembly::DeviceId::Uart)
        .unwrap()
        .clone();
    config.devices = board.motherboard_devices();
    config.devices.push(uart);
    config.drives.clear();
    config.memory.address_space_bytes = 0x1000000;
    config.memory.mmio.clear();
    config.memory.ram = vec![
        RamRegion::new(0, 0xa0000),
        RamRegion::new(0xc0000, 0x40000),
        RamRegion::new(0x100000, 15 << 20),
    ];
    if board == BoardModel::Ibm5170 {
        config.memory.ram.retain(|r| r.base < 0xa0000);
    }
    config
}
