// SPDX-License-Identifier: AGPL-3.0-or-later
//! BIOS Enhanced Disk Drive Services 3.0, §5.8 INT 13h drive parameters;
//! ATA-2 §§6.2, 7, 8.3–8.4 PIO protocols; IBM PS/2 Hardware Interface
//! Technical Reference, Auxiliary Device Interface and Pointing Device;
//! APM BIOS Interface Specification 1.2, §§4.6.1, 4.6.4, 4.6.6, 4.6.8.
//! SeaBIOS rel-1.17.0, disk.c disk_1308 reserves one landing cylinder.
//! See backlog/docs/design/windows95-on-486.md §5.

use super::{add_floppy, budget, shipped};
use assembly::{CpuChip, DeviceId};
use cpu::Cpu;
use journal::SectorSource;

fn memory(machine: &mut assembly::AssembledMachine, address: u64, len: usize) -> Vec<u8> {
    let mut bytes = vec![0; len];
    machine.machine.cpu().read_mem(address, &mut bytes).unwrap();
    bytes
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn dword(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn disk(config: &mut assembly::MachineConfig, cylinders: u32) {
    config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Fdc)
        .unwrap()
        .ports = assembly::FdcCfg::port_windows(true);
    let ide = assembly::IdeCfg {
        model: devices::ata::AtaControllerModel::Ata,
        base: 0x1f0,
        control: 0x3f6,
    };
    config.devices.push(
        assembly::DeviceConfig::new(
            DeviceId::Ide,
            assembly::DeviceKind::Ide(ide),
            ide.port_windows(),
        )
        .with_irq(14),
    );
    config.drives.push(assembly::DriveConfig {
        media: None,
        kind: assembly::DriveKind::HardDisk {
            geometry: assembly::HardDiskGeometry {
                cylinders,
                heads: 16,
                sectors_per_track: 63,
                bytes_per_sector: 512,
            },
        },
        image: assembly::ImageSource::Blank {
            size: u64::from(cylinders) * 16 * 63 * 512,
        },
        expected_sha256: None,
        controller: DeviceId::Ide,
        unit: 0,
        write: assembly::WriteMode::Volatile,
    });
}

fn sector(lba: u32) -> Vec<u8> {
    (0..256u16)
        .flat_map(|index| (index ^ lba as u16 ^ (lba >> 16) as u16).to_le_bytes())
        .collect()
}

struct ProbeMedia(journal::SparseOverlay);

impl device::SectorMedia for ProbeMedia {
    fn sector_count(&self) -> u64 {
        self.0.byte_len() / 512
    }

    fn read_sector(&self, lba: u64) -> [u8; 512] {
        self.0.read_sector(lba).unwrap()
    }

    fn write_batch(&mut self, sectors: &[device::SectorWrite]) {
        for sector in sectors {
            assert!(sector.lba < self.sector_count());
            self.0.replace(sector.lba, Box::new(sector.bytes));
        }
    }

    fn write_protected(&self) -> bool {
        false
    }
}

fn seed(machine: &mut assembly::AssembledMachine, cylinders: u32, lbas: &[u32]) {
    let capacity = cylinders * 16 * 63;
    let base = journal::VerifiedBase::zero(u64::from(capacity) * 512).unwrap();
    let mut overlay = journal::SparseOverlay::new(std::rc::Rc::new(base));
    // Seeding through ATA would let the same address-decoding bug alias both
    // the expected sector and the guest read. Keep media addressing independent.
    for &lba in lbas {
        assert!(lba < capacity);
        overlay.replace(u64::from(lba), Box::new(sector(lba).try_into().unwrap()));
    }
    let drive = devices::ata::AtaDrive::with_media(
        devices::ata::AtaGeometry::new(cylinders as u16, 16, 63).unwrap(),
        capacity,
        Box::new(ProbeMedia(overlay)),
    )
    .unwrap();
    let index = machine.device_index[&DeviceId::Ide];
    let ata = machine.machine.bus_mut().device_mut(index).unwrap();
    *ata = Box::new(devices::ata::Ata::with_drives([Some(drive), None]).unwrap());
}

fn halted(machine: &mut assembly::AssembledMachine, chip: CpuChip) {
    let outcome = machine.machine.run_bounded(Some(budget(chip, 15_000_000)));
    if let Err(machine::MachineError::Cpu(error)) = &outcome {
        eprintln!("{:?}", machine.machine.cpu().stop_report(error));
    }
    assert_eq!(outcome.unwrap(), machine::RunOutcome::Halted);
    assert!(!machine.machine.cpu().interrupts_enabled());
}

pub fn seabios_int13_disk_services(chip: CpuChip) {
    for cylinders in [1015, 2048] {
        let Some(mut config) = shipped(chip) else {
            return;
        };
        let last = cylinders * 16 * 63 - 1;
        let mut boot = include_bytes!("../fixtures/seabios-int13.bin").to_vec();
        boot[0x1e0..0x1e4].copy_from_slice(&u32::to_le_bytes(last));
        add_floppy(&mut config, &boot);
        disk(&mut config, cylinders);
        let mut machine = machine_file::assemble(&config).unwrap();
        // AH=08h reserves a landing cylinder; translated CHS is 1024/32/63.
        let (bios_cylinders, heads) = if cylinders == 1015 {
            (1015, 16)
        } else {
            (1024, 32)
        };
        let chs_last = (bios_cylinders - 1) * heads * 63 - 1;
        seed(&mut machine, cylinders, &[last, chs_last]);
        halted(&mut machine, chip);
        let result = memory(&mut machine, 0x500, 30);
        eprintln!(
            "{} {cylinders}: INT13 {result:02x?}; retired={}",
            chip.name(),
            machine.machine.cpu().retired()
        );
        assert_eq!(word(&result, 28), 0xcafe);
        for offset in [6, 14, 18, 22, 26] {
            assert_eq!(word(&result, offset) & 1, 0, "CF at {offset}");
        }
        assert_eq!(result[1], 0);
        let max_cylinder = bios_cylinders - 2;
        let cx = ((max_cylinder & 255) << 8) | ((max_cylinder >> 2) & 0xc0) | 63;
        assert_eq!(u32::from(word(&result, 2)), cx);
        assert_eq!(u32::from(word(&result, 4)), ((heads - 1) << 8) | 1);
        assert_eq!(result[9], 0x30);
        assert_eq!(word(&result, 10), 0xaa55);
        assert_eq!(word(&result, 12), 7);
        assert_eq!(result[17], 0);
        assert_eq!(result[21], 0);
        assert_eq!(result[25], 0);
        let edd = memory(&mut machine, 0x600, 66);
        eprintln!("EDD: {edd:02x?}");
        assert_eq!(word(&edd, 0), 0x1e);
        assert_eq!(word(&edd, 2), 2);
        assert_eq!(word(&edd, 30), 0xbedd);
        assert_eq!(edd[32], 36, "EDD device path length");
        assert_eq!(&edd[33..36], &[0; 3], "EDD reserved header");
        assert_eq!(&edd[36..40], b"ISA ");
        assert_eq!(&edd[40..48], b"ATA     ");
        assert_eq!(word(&edd, 48), 0x1f0);
        assert_eq!(&edd[50..56], &[0; 6], "EDD reserved ISA interface path");
        assert_eq!(edd[56], 0, "EDD ATA device path selects master");
        assert_eq!(&edd[57..65], &[0; 8], "EDD reserved ATA path and trailer");
        assert_eq!(
            edd[30..66]
                .iter()
                .fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
            0
        );
        assert_eq!(dword(&edd, 4), cylinders);
        assert_eq!(dword(&edd, 8), 16);
        assert_eq!(dword(&edd, 12), 63);
        assert_eq!(
            u64::from_le_bytes(edd[16..24].try_into().unwrap()),
            u64::from(last + 1)
        );
        assert_eq!(word(&edd, 24), 512);
        assert_eq!(memory(&mut machine, 0x1000, 512), sector(last));
        assert_eq!(memory(&mut machine, 0x1200, 512), sector(chs_last));
        let expected_retired = match (chip == CpuChip::I80386, cylinders) {
            (true, 1015) => 8_241_639,
            (true, _) => 8_241_493,
            (false, 1015) => 10_816_044,
            (false, _) => 10_816_191,
        };
        assert_eq!(machine.machine.cpu().retired(), expected_retired);
    }
}

pub fn seabios_ata_protected_driver(chip: CpuChip) {
    let Some(mut config) = shipped(chip) else {
        return;
    };
    add_floppy(
        &mut config,
        include_bytes!("../fixtures/seabios-ata-driver.bin"),
    );
    disk(&mut config, 1015);
    let mut machine = machine_file::assemble(&config).unwrap();
    seed(&mut machine, 1015, &[4, 5, 6, 7]);
    machine.enable_delivery_log().unwrap();
    halted(&mut machine, chip);
    let result = memory(&mut machine, 0x500, 19);
    eprintln!(
        "{} ATA: {result:02x?}; retired={}",
        chip.name(),
        machine.machine.cpu().retired()
    );
    assert_eq!(result[18], 0x58, "first WRITE MULTIPLE block status");
    assert_eq!(word(&result, 0), 0xcafe);
    assert_eq!(dword(&result, 4), 12);
    let deliveries = machine.machine.recent_deliveries().unwrap();
    let irq14 = deliveries
        .entries
        .iter()
        .filter_map(|entry| match entry {
            machine::DeliveryEntry::Deliveries { count, last, .. }
                if last.kind == cpu::DeliveryKind::Irq && last.vector == 0x2e =>
            {
                assert_eq!(last.irq, Some(14));
                assert_eq!(last.cs, 8);
                Some(count)
            }
            _ => None,
        })
        .sum::<u64>();
    assert_eq!(
        irq14, 12,
        "protected IDT deliveries, including each multiple block"
    );
    assert_eq!(&result[8..12], &[0, 0, 0, 0xff]);
    assert_eq!(&result[16..18], &[0xff, 4]);
    assert_eq!(dword(&result, 12) & 1, 1, "driver ran in protected mode");
    assert_eq!(
        memory(&mut machine, 0x600, 12),
        [0x58, 0x50, 0x58, 0x58, 0x58, 0x50, 0x58, 0x58, 0x50, 0x50, 0x50, 0x51]
    );
    let identify = memory(&mut machine, 0x1000, 512);
    for (index, expected) in [
        (0, 0x40),
        (1, 1015),
        (3, 16),
        (6, 63),
        (47, 0x8010),
        (49, 0x0e00),
        (53, 3),
        (54, 1015),
        (55, 16),
        (56, 63),
    ] {
        assert_eq!(
            word(&identify, index * 2),
            expected,
            "IDENTIFY word {index}"
        );
    }
    assert_eq!(
        dword(&identify, 114),
        1015 * 16 * 63,
        "IDENTIFY current CHS capacity (words 57–58)"
    );
    assert_eq!(dword(&identify, 120), 1015 * 16 * 63);
    let model: Vec<_> = identify[54..94]
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|pair| [pair[1], pair[0]])
        .collect();
    assert_eq!(
        String::from_utf8(model).unwrap().trim_end(),
        "UNEMU ATA-2 PIO DISK"
    );
    assert_eq!(
        memory(&mut machine, 0x1800, 2048),
        (4..8).flat_map(sector).collect::<Vec<_>>()
    );
    let written: Vec<_> = (0..1024u16)
        .flat_map(|index| (index + 0x1234).to_le_bytes())
        .collect();
    assert_eq!(memory(&mut machine, 0x4000, 2048), written);
    assert_eq!(
        machine.machine.cpu().retired(),
        if chip == CpuChip::I80386 {
            8_259_907
        } else {
            10_850_496
        }
    );
    let bus = machine.machine.bus_mut();
    for port in [0x20, 0xa0] {
        bus.port_write(port, 1, 0x0b).unwrap();
        assert_eq!(bus.port_read(port, 1).unwrap(), 0, "PIC ISR after EOI");
    }
}

pub fn seabios_apm_protected_power_off(chip: CpuChip) {
    let Some(mut config) = shipped(chip) else {
        return;
    };
    add_floppy(&mut config, include_bytes!("../fixtures/seabios-apm.bin"));
    let mut machine = machine_file::assemble(&config).unwrap();
    // CPU Idle waits for a timer edge; allow two default PIT periods.
    machine.machine.set_idle_limit(Some(2 * 65_536));
    halted(&mut machine, chip);
    let result = memory(&mut machine, 0x500, 44);
    eprintln!(
        "{} APM: {result:02x?}; retired={}",
        chip.name(),
        machine.machine.cpu().retired()
    );
    assert_eq!(word(&result, 0), 0x0102);
    assert_eq!(word(&result, 2), 0x504d);
    assert_eq!(word(&result, 4), 3);
    for offset in [6, 24, 28, 34] {
        assert_eq!(word(&result, offset) & 1, 0, "APM CF at {offset}");
    }
    assert_eq!(word(&result, 8), 0xf000);
    assert!((1..0x10000).contains(&dword(&result, 10)));
    assert_eq!(word(&result, 14), 0xf000);
    assert_eq!(word(&result, 16), 0xf000);
    assert_eq!(dword(&result, 18), 0xfff0fff0);
    assert_eq!(word(&result, 22), 0xfff0);
    assert_eq!(word(&result, 26), 0x0102);
    assert_eq!(word(&result, 32), 0x5305);
    assert_eq!(word(&result, 38), 0xcafe, "idle returned before power-off");
    assert_eq!(word(&result, 40), 0, "power-off must not return");
    assert_eq!(
        word(&result, 42),
        1,
        "idle wakes on IRQ0 through the protected IDT"
    );
    let state = machine.machine.cpu().diagnostic_state();
    assert_eq!(state.segments[1].selector, 24);
    assert_eq!(state.segments[1].base, 0xf0000);
    assert_eq!(
        state.before_code.value(),
        Some(0xf4),
        "stopped at the BIOS HLT"
    );
    assert!(machine.machine.halted());
    assert_eq!(
        machine.machine.cpu().retired(),
        if chip == CpuChip::I80386 {
            8_239_019
        } else {
            10_813_700
        }
    );
}

fn phase(machine: &mut assembly::AssembledMachine, chip: CpuChip, expected: u16) {
    let limit = budget(chip, 15_000_000);
    loop {
        let marker = word(&memory(machine, 0x500, 2), 0);
        if marker == expected {
            return;
        }
        assert!(
            machine.machine.virtual_time() < limit,
            "mouse phase {expected}, marker {marker:04x}"
        );
        let before = machine.machine.virtual_time();
        let cap = (before + 1000).min(limit);
        let outcome = machine.machine.run_bounded(Some(cap)).unwrap();
        let marker = word(&memory(machine, 0x500, 2), 0);
        if marker == expected {
            return;
        }
        assert_eq!(
            outcome,
            machine::RunOutcome::RetiredCapReached,
            "mouse stopped before phase {expected}, marker {marker:04x}"
        );
        assert!(
            machine.machine.virtual_time() > before,
            "mouse made no progress before phase {expected}, marker {marker:04x}"
        );
    }
}

pub fn seabios_mouse_aux_and_bios_handler(chip: CpuChip) {
    let Some(mut config) = shipped(chip) else {
        return;
    };
    let input = super::template(chip).replace("[board]\n", "[board]\nmouse = true\n");
    config = machine_file::lower(
        &machine_file::parse_toml("mouse.toml", &input).unwrap(),
        &machine_file::Origins::from_toml("mouse.toml", &input).unwrap(),
        &mut super::FirmwareImages(&config),
    )
    .unwrap();
    add_floppy(&mut config, include_bytes!("../fixtures/seabios-mouse.bin"));
    let mut machine = machine_file::assemble(&config).unwrap();
    let index = machine.mouse_device_index().unwrap();
    phase(&mut machine, chip, 1);
    assert!(assembly::inject_mouse(
        &mut machine.machine,
        index,
        5,
        -7,
        1
    ));
    phase(&mut machine, chip, 2);
    assert!(assembly::inject_mouse(
        &mut machine.machine,
        index,
        -9,
        11,
        2
    ));
    halted(&mut machine, chip);
    let result = memory(&mut machine, 0x500, 56);
    eprintln!(
        "{} mouse: {result:02x?}; retired={}",
        chip.name(),
        machine.machine.cpu().retired()
    );
    assert_eq!(word(&result, 0), 0xcafe);
    for offset in [2, 6, 10, 14] {
        assert_eq!(result[offset + 1], 0, "INT15 AH at {offset}");
    }
    for offset in [4, 8, 12, 16] {
        assert_eq!(word(&result, offset) & 1, 0, "INT15 CF at {offset}");
    }
    assert_eq!(word(&result, 40), 3);
    assert_eq!(word(&result, 42), 1);
    assert_eq!(&result[48..56], &[0x29, 0, 5, 0, 0xf9, 0, 0, 0]);
    assert_eq!(
        memory(&mut machine, 0x610, 8),
        [0xfa, 0xaa, 0, 0xfa, 0xfa, 0xfa, 0xfa, 0]
    );
    for status in memory(&mut machine, 0x600, 8)
        .into_iter()
        .chain(memory(&mut machine, 0x620, 3))
    {
        assert_eq!(status & 0xe1, 0x21, "auxiliary OBF, no timeout/parity");
    }
    assert_eq!(memory(&mut machine, 0x630, 3), [0x1a, 0xf7, 11]);
    assert_eq!(
        machine.machine.cpu().retired(),
        if chip == CpuChip::I80386 {
            8_402_774
        } else {
            11_140_838
        }
    );
    let bus = machine.machine.bus_mut();
    for port in [0x20, 0xa0] {
        bus.port_write(port, 1, 0x0b).unwrap();
        assert_eq!(
            bus.port_read(port, 1).unwrap(),
            0,
            "IRQ12 handler EOIs both PICs"
        );
    }
}
