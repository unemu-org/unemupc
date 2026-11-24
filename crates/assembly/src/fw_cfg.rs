// SPDX-License-Identifier: AGPL-3.0-or-later
//! Assembly of emulator-only firmware configuration files.
//!
//! QEMU Firmware Configuration (fw_cfg) Device specification, File Directory;
//! SeaBIOS romfile_loadint reads etc/sercon-port as a little-endian integer.
//! See `backlog/docs/design/machine-config.md` §Emulator-only firmware configuration.

use crate::{
    ConfigError, DeviceId, DeviceKind, FwCfgCfg, FwCfgFile, ImageKind, ImageSource, LoadImage,
    MachineConfig,
};

// SeaBIOS optionroms.c vgarom_setup clears adapter shadow RAM before it
// deploys vgaroms/ files. Keep the ordinary ROM source and validation path,
// but let firmware perform the copy and initialization itself. Its allocator
// starts at C0000h regardless of the filename (optionroms.c run_file_roms;
// malloc.c rom_reserve / rom_confirm), so only that base can be represented.
pub(crate) fn video_rom_file(machine: &MachineConfig, image: &LoadImage) -> bool {
    machine.board.is_generic_isa() && image.kind == ImageKind::Rom && image.base == 0xC0000
}

pub(crate) fn files(
    machine: &MachineConfig,
    device: DeviceId,
    cfg: &FwCfgCfg,
) -> Result<Vec<FwCfgFile>, ConfigError> {
    let mut files = cfg.files.clone();
    if cfg.sercon_from_com1 {
        let port = machine
            .devices
            .iter()
            .find_map(|entry| {
                if let (DeviceId::Uart, DeviceKind::Uart(uart)) = (entry.id, &entry.kind) {
                    Some(uart.base)
                } else {
                    None
                }
            })
            .ok_or(ConfigError::FwCfgCom1Missing { device })?;
        files.push(FwCfgFile {
            name: "etc/sercon-port".into(),
            bytes: port.to_le_bytes().into(),
        });
    }
    for image in machine
        .images
        .iter()
        .filter(|image| video_rom_file(machine, image))
    {
        let bytes = match &image.source {
            ImageSource::Inline(bytes) => bytes.as_slice(),
            ImageSource::RomContainer { bytes, offset } => match bytes.get(*offset..) {
                Some(bytes) => bytes,
                None => continue,
            },
            // Validation also runs before the normal firmware loader resolves files.
            _ => continue,
        };
        files.push(FwCfgFile {
            name: "vgaroms/vga-c0000.bin".into(),
            bytes: bytes.to_vec(),
        });
    }
    devices::FwCfg::validate_files(&files)
        .map_err(|source| ConfigError::InvalidFwCfgFiles { device, source })?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{assemble, BoardModel, DeviceConfig, PortWidth, PortWindow};
    use cpu::Cpu;

    fn entry(cfg: FwCfgCfg) -> DeviceConfig {
        DeviceConfig::new(
            DeviceId::FwCfg,
            DeviceKind::FwCfg(cfg),
            FwCfgCfg::port_windows(),
        )
    }

    #[test]
    fn fw_cfg_confinement_is_by_kind_and_has_exact_ibm5170_message() {
        for id in [DeviceId::FwCfg, DeviceId::Uart] {
            let mut config = crate::at_boot(vec![0xF4]);
            config.board = BoardModel::Ibm5170;
            config.devices.retain(|device| device.id != id);
            let mut fw_cfg = entry(FwCfgCfg::default());
            fw_cfg.id = id;
            config.devices.push(fw_cfg);
            let error = config.validate().unwrap_err();
            assert_eq!(error, ConfigError::FwCfgBoard);
            assert_eq!(
                error.to_string(),
                "fw-cfg port is emulator-only; it is allowed only on board generic-386-isa"
            );
        }
    }

    #[test]
    fn fw_cfg_resources_require_word_selector_byte_data_and_no_irq_or_dma() {
        for change in 0..6 {
            let mut config = crate::at_boot(vec![0xF4]);
            let mut device = entry(FwCfgCfg::default());
            match change {
                0 => device.ports[0].width = PortWidth::Byte,
                1 => device.ports[1].width = PortWidth::Word,
                2 => device.ports = vec![PortWindow::new(0x510, 0x511)],
                3 => device.irq = Some(4),
                4 => {
                    device.dma = Some(crate::DmaSpec {
                        controller: DeviceId::Dma,
                        channel: 1,
                    })
                }
                _ => device.ports[0].start = 0x512,
            }
            config.devices.push(device);
            assert_eq!(
                config.validate(),
                Err(ConfigError::InvalidFwCfgResources {
                    device: DeviceId::FwCfg
                })
            );
        }
    }

    #[test]
    fn fw_cfg_sercon_is_explicit_little_endian_and_follows_com1_configuration() {
        let mut config = crate::at_boot(vec![0xF4]);
        let uart = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Uart)
            .unwrap();
        uart.kind = DeviceKind::Uart(crate::UartCfg {
            base: 0x2F8,
            mouse: false,
        });
        uart.ports = vec![PortWindow::new(0x2F8, 0x2FF)];
        config.devices.push(entry(FwCfgCfg {
            sercon_from_com1: true,
            ..Default::default()
        }));
        let mut assembled = assemble(&config).unwrap();
        let bus = assembled.machine.bus_mut();
        bus.port_write(0x510, 2, 0x19).unwrap();
        let directory: Vec<_> = (0..68)
            .map(|_| bus.port_read(0x511, 1).unwrap() as u8)
            .collect();
        assert_eq!(&directory[..12], &[0, 0, 0, 1, 0, 0, 0, 2, 0, 0x20, 0, 0]);
        assert_eq!(&directory[12..27], b"etc/sercon-port");
        assert!(directory[27..].iter().all(|byte| *byte == 0));
        bus.port_write(0x510, 2, 0x20).unwrap();
        assert_eq!(bus.port_read(0x511, 1).unwrap(), 0xF8);
        assert_eq!(bus.port_read(0x511, 1).unwrap(), 2);
        assert!(files(&config, DeviceId::FwCfg, &FwCfgCfg::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn fw_cfg_invalid_files_and_missing_or_wrong_com1_are_rejected() {
        let mut config = crate::at_boot(vec![0xF4]);
        let cfg = FwCfgCfg {
            files: vec![FwCfgFile {
                name: "etc/sercon-port".into(),
                bytes: vec![],
            }],
            sercon_from_com1: true,
        };
        config.devices.push(entry(cfg));
        assert_eq!(
            config.validate(),
            Err(ConfigError::InvalidFwCfgFiles {
                device: DeviceId::FwCfg,
                source: devices::FwCfgError::InvalidName
            })
        );
        config.devices.last_mut().unwrap().kind = DeviceKind::FwCfg(FwCfgCfg {
            sercon_from_com1: true,
            ..Default::default()
        });
        config.devices.retain(|d| d.id != DeviceId::Uart);
        assert_eq!(
            config.validate(),
            Err(ConfigError::FwCfgCom1Missing {
                device: DeviceId::FwCfg
            })
        );
        config.devices.push(DeviceConfig::new(
            DeviceId::Uart,
            DeviceKind::Post(crate::PostPortCfg { port: 0x80 }),
            vec![],
        ));
        assert_eq!(
            config.validate(),
            Err(ConfigError::FwCfgCom1Missing {
                device: DeviceId::FwCfg
            })
        );
    }

    #[test]
    fn fw_cfg_bus_preserves_full_word_selectors_and_ignores_byte_writes() {
        let mut config = crate::at_boot(vec![0xF4]);
        config.devices.push(entry(FwCfgCfg::default()));
        let mut assembled = assemble(&config).unwrap();
        let bus = assembled.machine.bus_mut();
        for (key, expected) in [
            (0x8000, 0),
            (0, u32::from(b'Q')),
            (0x8001, 0),
            (1, 1),
            (0x4000, u32::from(b'Q')),
        ] {
            bus.port_write(0x510, 2, key).unwrap();
            assert_eq!(bus.port_read(0x511, 1).unwrap(), expected);
        }
        bus.port_write(0x510, 2, 1).unwrap();
        bus.port_write(0x510, 1, 0).unwrap();
        bus.port_write(0x511, 1, 0).unwrap();
        assert_eq!(bus.port_read(0x511, 1).unwrap(), 1);
        assert_eq!(bus.port_read(0x511, 1).unwrap(), 0);
    }

    #[test]
    fn fw_cfg_bare_guest_rep_insb_copies_file_to_ram() {
        // Explicit ES and direction flag keep the transfer independent of preset seeds.
        let program = vec![
            0x31, 0xC0, 0x8E, 0xC0, 0xFC, 0xBA, 0x10, 0x05, 0xB8, 0x20, 0x00, 0xEF, 0x42, 0xBF,
            0x00, 0x02, 0xB9, 0x08, 0x00, 0xF3, 0x6C, 0xFA, 0xF4,
        ];
        let mut config = crate::at_boot(program);
        config.devices.push(entry(FwCfgCfg {
            files: vec![FwCfgFile {
                name: "opt/transfer".into(),
                bytes: vec![0x41, 0x00, 0x7F, 0x80, 0xFE, 0x5A],
            }],
            ..Default::default()
        }));
        config.images.push(crate::LoadImage {
            base: 0x1FF,
            source: crate::ImageSource::Inline(vec![0xCC; 10]),
            kind: crate::ImageKind::RamLoad,
            provides: vec![],
        });
        let mut assembled = assemble(&config).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(64)).unwrap(),
            machine::RunOutcome::Halted
        );
        let cpu = assembled.machine.cpu();
        let mut actual = [0; 10];
        cpu.read_mem(0x1FF, &mut actual).unwrap();
        assert_eq!(
            actual,
            [0xCC, 0x41, 0x00, 0x7F, 0x80, 0xFE, 0x5A, 0, 0, 0xCC]
        );
        assert_eq!(cpu.reg(cpu::CpuReg::Rcx), 0);
        assert_eq!(cpu.reg(cpu::CpuReg::Rdi), 0x208);
    }

    #[test]
    fn fw_cfg_bare_guest_reads_signature_via_in_instructions() {
        // Read ID before reselecting signature, so ignored OUTW cannot pass.
        // Copy four IN bytes to the diagnostic console using a second DX port.
        let mut program = vec![
            0xBA, 0x10, 0x05, 0xB8, 1, 0, 0xEF, 0x42, 0xEC, 0x4A, 0x31, 0xC0, 0xEF,
        ];
        for _ in 0..4 {
            program.extend([0xBA, 0x11, 0x05, 0xEC, 0xBA, 0x02, 0x04, 0xEE]);
        }
        program.push(0xF4);
        let mut config = crate::at_boot(program);
        config.devices.push(entry(FwCfgCfg::default()));
        config.devices.push(DeviceConfig::new(
            DeviceId::DebugConsole,
            DeviceKind::DebugConsole(crate::DebugConsoleCfg::default()),
            vec![PortWindow::single(0x402)],
        ));
        let policy = crate::ObservationPolicy {
            routes: vec![crate::ObservationRequest {
                device: DeviceId::DebugConsole,
                output: crate::OutputKind::Bytes(device::ByteStreamKind::DebugConsole),
                byte_limit: 4,
            }],
            ..Default::default()
        };
        let mut assembled = crate::assemble_with_observation(&config, &policy).unwrap();
        assert_eq!(
            assembled.machine.run_bounded(Some(64)).unwrap(),
            machine::RunOutcome::Halted
        );
        assert_eq!(
            assembled
                .observation
                .drain_bytes(DeviceId::DebugConsole, device::ByteStreamKind::DebugConsole)
                .unwrap(),
            b"QEMU"
        );
    }
}

#[cfg(test)]
mod video_tests {
    use super::*;

    #[test]
    fn generic_isa_video_rom_uses_owned_fw_cfg_bytes_without_a_second_source() {
        let mut config = crate::at_boot(vec![0xF4]);
        config.board = crate::BoardModel::Generic386Isa;
        let mut bytes = vec![0; 512];
        bytes[..3].copy_from_slice(&[0x55, 0xAA, 1]);
        config.images.push(LoadImage {
            base: 0xC0000,
            source: ImageSource::Inline(bytes.clone()),
            kind: ImageKind::Rom,
            provides: vec![],
        });
        let cfg = FwCfgCfg::default();
        let files = files(&config, DeviceId::FwCfg, &cfg).unwrap();
        assert_eq!(
            files,
            vec![FwCfgFile {
                name: "vgaroms/vga-c0000.bin".into(),
                bytes,
            }]
        );
        config.images.last_mut().unwrap().source = ImageSource::Inline(vec![0xFF; 512]);
        assert_eq!(&files[0].bytes[..3], &[0x55, 0xAA, 1]);
        config.board = crate::BoardModel::Ibm5170;
        assert!(super::files(&config, DeviceId::FwCfg, &cfg)
            .unwrap()
            .is_empty());
        config.board = crate::BoardModel::Bare;
        assert!(super::files(&config, DeviceId::FwCfg, &cfg)
            .unwrap()
            .is_empty());
    }
}
