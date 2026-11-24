// SPDX-License-Identifier: AGPL-3.0-or-later
//! Configuration validation (§3.1).
//!
//! [`crate::MachineConfig::validate`] runs a fixed sequence of checks over the
//! pure-data config and returns the first [`ConfigError`] it finds. Each check is
//! a small pure function over the config; none assembles a machine, so the whole
//! module is unit-testable in isolation.

use crate::board::BoardModel;
use crate::device::{DeviceConfig, DeviceId, DeviceKind, PortWindow};
use crate::error::ConfigError;
use crate::image::ImageKind;
use crate::machine::MachineConfig;

type ConfigCheck = fn(&MachineConfig) -> Result<(), ConfigError>;

/// Whether two half-open ranges `[a_base, a_end)` and `[b_base, b_end)` share any
/// byte.
fn ranges_overlap(a_base: u64, a_end: u64, b_base: u64, b_end: u64) -> bool {
    a_base < b_end && b_base < a_end
}

fn format_board_ports(ports: &[PortWindow]) -> String {
    let windows: Vec<_> = ports
        .iter()
        .map(|window| {
            let ports = if window.start == window.end {
                format!("{:#x}", window.start)
            } else {
                format!("{:#x}-{:#x}", window.start, window.end)
            };
            let ports = if window.width == bus::PortWidth::Word {
                format!("{ports} (16-bit)")
            } else {
                ports
            };
            if window.allow_overlap {
                format!("{ports} (overlap allowed)")
            } else {
                ports
            }
        })
        .collect();
    format!("[{}]", windows.join(", "))
}

fn format_board_kind(kind: &DeviceKind) -> String {
    match kind {
        DeviceKind::Uart(cfg) => format!(
            "Uart(UartCfg {{ base: {:#x}, mouse: {} }})",
            cfg.base, cfg.mouse
        ),
        DeviceKind::Post(cfg) => format!("Post(PostPortCfg {{ port: {:#x} }})", cfg.port),
        DeviceKind::Fdc(cfg) => format!("Fdc(FdcCfg {{ drive_slots: {:#x} }})", cfg.drive_slots),
        DeviceKind::Rtc(cfg) => {
            let timing = cfg.timing.map_or_else(
                || "None".into(),
                |(second, uip)| format!("Some(({second:#x}, {uip:#x}))"),
            );
            format!(
                "Rtc(CmosCfg {{ start: {:?}, nvram: {:?}, timing: {timing} }})",
                cfg.start, cfg.nvram
            )
        }
        kind => format!("{kind:?}"),
    }
}

fn format_board_irq(irq: Option<u8>) -> String {
    irq.map_or_else(|| "None".into(), |line| format!("Some({line:#x})"))
}

fn format_board_dma(dma: Option<crate::DmaSpec>) -> String {
    dma.map_or_else(
        || "None".into(),
        |dma| {
            format!(
                "Some(DmaSpec {{ controller: {:?}, channel: {:#x} }})",
                dma.controller, dma.channel
            )
        },
    )
}

struct FirmwareRequirement {
    matches: fn(&DeviceKind) -> bool,
    service: crate::FirmwareService,
    needs_bound_drive: bool,
}

// A controller's firmware contract belongs to its kind, not to an optional
// annotation. Controllers that serve firmware without media use false here.
const FIRMWARE_REQUIREMENTS: &[FirmwareRequirement] = &[
    FirmwareRequirement {
        matches: |kind| matches!(kind, DeviceKind::Ide(_)),
        service: crate::FirmwareService::FixedDisk,
        needs_bound_drive: false,
    },
    FirmwareRequirement {
        matches: |kind| matches!(kind, DeviceKind::Fdc(_)),
        service: crate::FirmwareService::Floppy,
        needs_bound_drive: true,
    },
];

struct WordPortSupport {
    supports: fn(&DeviceKind, u16) -> bool,
}

const WORD_PORT_SUPPORT: &[WordPortSupport] = &[
    // IOCS16# follows the configured task-file base, including secondary channels.
    WordPortSupport {
        supports: |kind, port| matches!(kind, DeviceKind::Ide(cfg) if port == cfg.base),
    },
    WordPortSupport {
        supports: |kind, port| matches!(kind, DeviceKind::FwCfg(_) if port == 0x510),
    },
];

impl MachineConfig {
    /// Validate this configuration, returning the first inconsistency found.
    ///
    /// Validation runs in this fixed order: journal modes; RTC RAM capacity and
    /// NVRAM format; firmware-part sources; VGA exclusion; unique device IDs;
    /// System Control Port A board support and resources; debug console; fw_cfg;
    /// board constraints; mouse support; firmware layout and services; RTC start;
    /// access coupling; UART bases; IDE resources; port widths and overlaps; MMIO devices, sizes,
    /// overlaps and RAM exclusion; address-space bounds; image placement; option
    /// ROMs; IRQ conflicts and PIC faces; DMA controllers; drive binding and image
    /// geometry; clock ratio; persistent path validation. This makes the first
    /// error deterministic. File lengths are checked when [`crate::assemble`] reads
    /// the sources and validates their bytes. For a BIOS file not yet read,
    /// service validation uses the declaration or defers missing services when it
    /// is empty. Catalogue checks requiring image bytes run at assembly time.
    ///
    /// This checks logical keys. `machine_file::validate` additionally checks
    /// canonical paths and file identities against inputs and other outputs.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_logical()
    }

    /// Validate hardware and logical media keys without consulting the host.
    ///
    /// Stored configs and launch documents name mount keys, not filesystem
    /// paths. Retention and distinct journal keys still apply; the mount
    /// checks its outputs against inputs when the machine starts. See
    /// `backlog/docs/design/core-contract.md` §14.2.2.
    pub fn validate_logical(&self) -> Result<(), ConfigError> {
        self.validate_logical_with_firmware(self)
    }

    /// Validate a logical config with sized placeholders for unread ROMs.
    ///
    /// `firmware` must be the same config with the original ROM sources.
    /// Hardware checks use this config's known lengths; firmware checks use
    /// the originals so a placeholder never stands in for firmware bytes.
    /// Both run in the normal validation order. See
    /// `backlog/docs/design/core-contract.md` §14.2.2.
    pub fn validate_logical_with_firmware(&self, firmware: &Self) -> Result<(), ConfigError> {
        self.validate_mounted_with_firmware(firmware, |_| Ok(()))
    }

    /// [`MachineConfig::validate`] with the last step, the check of the
    /// journal outputs against the host, left to `check_outputs`, which a
    /// mount answers ([`crate::Mount::check_outputs`]). It takes the config
    /// with one drive per catalogue entry. Whatever the mount, a positive
    /// retention for NVRAM journals is checked before it and distinct
    /// journal keys after it.
    pub fn validate_mounted(
        &self,
        check_outputs: impl FnOnce(&Self) -> Result<(), ConfigError>,
    ) -> Result<(), ConfigError> {
        self.validate_mounted_with_firmware(self, check_outputs)
    }

    fn validate_mounted_with_firmware(
        &self,
        firmware: &Self,
        check_outputs: impl FnOnce(&Self) -> Result<(), ConfigError>,
    ) -> Result<(), ConfigError> {
        let media = self.expanded_media()?;
        let original_indices: Vec<_> = self
            .drives
            .iter()
            .enumerate()
            .flat_map(|(index, drive)| {
                std::iter::repeat_n(
                    index,
                    drive.media.as_ref().map_or(1, |list| list.entries.len()),
                )
            })
            .collect();
        let original_drive = |mut error| {
            match &mut error {
                ConfigError::Journal { drive_index, .. }
                | ConfigError::DriveInput { drive_index, .. } => {
                    *drive_index = original_indices[*drive_index];
                }
                _ => {}
            }
            error
        };
        media.check_journal_modes().map_err(original_drive)?;
        media.check_part_sources()?;
        self.validate_resolved_with_firmware(firmware)?;
        crate::persistence::check_nvram_retention(&media)?;
        check_outputs(&media).map_err(original_drive)?;
        // After the mount's check, so that a file mount, which refuses equal
        // outputs as aliases, reports exactly what it did before.
        crate::persistence::check_distinct_journals(&media).map_err(original_drive)
    }

    pub(crate) fn expanded_media(&self) -> Result<Self, ConfigError> {
        let mut expanded = self.clone();
        expanded.drives.clear();
        for (drive_index, drive) in self.drives.iter().enumerate() {
            if let Some(list) = &drive.media {
                if !matches!(drive.kind, crate::DriveKind::Floppy { .. })
                    || (list.entries.is_empty() && !list.empty)
                {
                    return Err(ConfigError::Journal {
                        drive_index,
                        message: "media requires a floppy catalogue or an empty drive".into(),
                    });
                }
                if !list.empty && list.inserted >= list.entries.len() {
                    return Err(ConfigError::Journal {
                        drive_index,
                        message: "the disk in the drive at power-on is not in its catalogue".into(),
                    });
                }
                for medium in &list.entries {
                    if let Some(actual) = medium.image.known_len() {
                        drive.kind.check_image_len(drive_index, actual)?;
                    }
                    expanded.drives.push(drive.with_medium(medium));
                }
            } else {
                expanded.drives.push(drive.clone());
            }
        }
        Ok(expanded)
    }

    // Journal source checks run before file resolution; repeat the remaining
    // checks on the exact loaded bytes without requiring those bytes to be files.
    pub(crate) fn validate_resolved(&self) -> Result<(), ConfigError> {
        self.validate_resolved_with_firmware(self)
    }

    fn validate_resolved_with_firmware(&self, firmware: &Self) -> Result<(), ConfigError> {
        self.expanded_media()?;
        for (check, config) in self.resolved_checks(firmware) {
            check(config)?;
        }
        Ok(())
    }

    fn check_vga_exclusion(&self) -> Result<(), ConfigError> {
        if self
            .devices
            .iter()
            .any(|d| matches!(d.kind, DeviceKind::Vga(_)))
            && self
                .devices
                .iter()
                .any(|d| matches!(d.kind, DeviceKind::VgaRegisters(_)))
        {
            return Err(ConfigError::ConflictingVgaCards);
        }
        Ok(())
    }

    fn check_primary_display(&self) -> Result<(), ConfigError> {
        self.primary_display().map(|_| ())
    }
    fn check_clock(&self) -> Result<(), ConfigError> {
        self.clock.cpu_rate().map(|_| ())
    }
    fn check_option_roms(&self) -> Result<(), ConfigError> {
        crate::firmware::validate_option_roms(self)
    }

    fn resolved_checks<'a>(
        &'a self,
        firmware: &'a Self,
    ) -> impl Iterator<Item = (ConfigCheck, &'a Self)> {
        let hardware = |check: ConfigCheck| (check, self);
        let firmware = |check: ConfigCheck| (check, firmware);
        [
            hardware(Self::check_rtc_ram),
            hardware(Self::check_part_sources),
            hardware(Self::check_vga_exclusion),
            hardware(Self::check_unique_device_ids),
            hardware(Self::check_sys_ctrl_port_a),
            hardware(Self::check_debug_console),
            hardware(Self::check_fw_cfg),
            hardware(Self::check_board),
            hardware(Self::check_mouse),
            hardware(Self::check_primary_display),
            firmware(Self::check_firmware_layout),
            firmware(Self::check_firmware_services),
            hardware(Self::check_rtc_starts),
            hardware(Self::check_access_couplings),
            hardware(Self::check_uart_bases),
            hardware(Self::check_ide_resources),
            hardware(Self::check_sb16_resources),
            hardware(Self::check_lotech_ems_resources),
            hardware(Self::check_port_widths),
            hardware(Self::check_port_overlaps),
            hardware(Self::check_mmio_devices_present),
            hardware(Self::check_mmio_sizes),
            hardware(Self::check_mmio_overlaps),
            hardware(Self::check_mmio_vs_ram),
            hardware(Self::check_regions_in_space),
            hardware(Self::validate_image_placement),
            hardware(Self::check_board_memory),
            firmware(Self::check_option_roms),
            hardware(Self::check_irq_conflicts),
            hardware(Self::check_pic_faces),
            hardware(Self::check_dma_controllers_present),
            hardware(Self::check_drive_controllers),
            hardware(Self::check_image_geometry),
            hardware(Self::check_clock),
        ]
        .into_iter()
    }

    /// Collect independent checks and every resource conflict without changing launch order.
    pub(crate) fn draft_errors(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();
        match self.expanded_media() {
            Ok(media) => {
                if let Err(error) = media.check_journal_modes() {
                    errors.push(error);
                }
            }
            Err(error) => errors.push(error),
        }
        for (check, config) in self.resolved_checks(self) {
            if let Err(error) = check(config) {
                let original = error.clone();
                match error {
                    ConfigError::IrqConflict { .. } | ConfigError::InvalidIrqLine { .. } => {
                        errors.extend(self.collect_irq_conflicts())
                    }
                    ConfigError::PortOverlap { .. }
                    | ConfigError::PortOverlapDecodeOrder { .. } => {
                        errors.extend(self.collect_port_overlaps())
                    }
                    ConfigError::DmaConflict { .. }
                    | ConfigError::InvalidDmaChannel { .. }
                    | ConfigError::DmaControllerMissing { .. } => {
                        errors.extend(self.collect_dma_controllers_present())
                    }
                    _ => {}
                }
                if !errors.contains(&original) {
                    errors.push(original);
                }
            }
        }
        errors
    }

    /// Placement checks can use a selected firmware's known size before its bytes arrive.
    pub(crate) fn draft_image_errors(&self) -> Vec<ConfigError> {
        [
            self.check_firmware_layout(),
            self.check_regions_in_space(),
            self.validate_image_placement(),
            self.check_board_memory(),
        ]
        .into_iter()
        .filter_map(Result::err)
        .collect()
    }

    pub(crate) fn check_part_sources(&self) -> Result<(), ConfigError> {
        for drive in &self.drives {
            if matches!(drive.image, crate::ImageSource::RomContainer { .. })
                || drive.media.as_ref().is_some_and(|list| {
                    list.entries.iter().any(|medium| {
                        matches!(medium.image, crate::ImageSource::RomContainer { .. })
                    })
                })
            {
                return Err(ConfigError::FirmwareParts {
                    message: "drives cannot use ROM containers".into(),
                });
            }
            if drive.media.is_none() && matches!(drive.image, crate::ImageSource::Interleaved(_)) {
                return Err(ConfigError::FirmwareParts {
                    message: "drives cannot use parts".into(),
                });
            }
        }
        for image in &self.images {
            if let crate::ImageSource::RomContainer { bytes, offset } = &image.source {
                if image.kind != ImageKind::Rom || *offset >= bytes.len() {
                    return Err(ConfigError::FirmwareParts {
                        message: "ROM containers require a nonempty mapped suffix and ROM kind"
                            .into(),
                    });
                }
            }
            if matches!(image.source, crate::ImageSource::Interleaved(_)) {
                if image.kind == ImageKind::RamLoad {
                    return Err(ConfigError::FirmwareParts {
                        message: "RAM load images cannot use parts".into(),
                    });
                }
                let bios = match crate::firmware::system_bios(self) {
                    Ok(bios) => Some(bios),
                    Err(_) if crate::firmware::unresolved_roms(self) => None,
                    Err(error) => return Err(error),
                };
                if bios.is_some_and(|bios| !std::ptr::eq(image, bios)) {
                    return Err(ConfigError::FirmwareParts {
                        message: "option ROMs cannot use parts; only firmware.bios supports parts"
                            .into(),
                    });
                }
            }
        }
        Ok(())
    }

    fn check_firmware_layout(&self) -> Result<(), ConfigError> {
        if self.board.firmware_catalogue().is_empty() {
            return Ok(());
        }
        let bios = match crate::firmware::system_bios(self) {
            Ok(bios) => Some(bios),
            Err(_) if crate::firmware::unresolved_roms(self) => None,
            Err(error) => return Err(error),
        };
        if self.cpu.reset != crate::ResetSpec::reset_vector() {
            return Err(ConfigError::FirmwareReset {
                board: self.board.id(),
                reset: self.cpu.reset,
            });
        }
        let size = bios.and_then(|bios| bios.source.known_len());
        if let (Some(bios), Some(size)) = (bios, size) {
            crate::firmware::check_bios_placement(bios.base, size)?;
        }
        // A file's full range is checked after its single read. Its reset byte
        // is already reserved before I/O, so RAM cannot replace the BIOS entry.
        let protected_base = if size.is_some() {
            bios.map_or(0xFFFF0, |bios| bios.base)
        } else {
            0xFFFF0
        };
        let protected_end = size.map_or(0xFFFF1, |size| protected_base.saturating_add(size));
        for ram in &self.memory.ram {
            if self.board.is_generic_isa() {
                continue;
            }
            if ram.size > 0 && ranges_overlap(protected_base, protected_end, ram.base, ram.end()) {
                return Err(ConfigError::SystemBiosOverlapsRam {
                    board: self.board.id(),
                    addr: protected_base.max(ram.base),
                });
            }
        }
        let firmware = if bios.is_some() {
            Some(crate::firmware::scan_firmware(self)?)
        } else {
            self.board
                .firmware_catalogue()
                .iter()
                .find(|entry| entry.kind == crate::FirmwareKind::SystemBios)
        };
        if let Some((_, end)) = firmware.and_then(|entry| entry.scan_window) {
            for ram in &self.memory.ram {
                if ram.size > 0 && ranges_overlap(0xC0000, end, ram.base, ram.end()) {
                    return Err(ConfigError::AdapterSpaceOverlapsRam {
                        board: self.board.id(),
                        addr: 0xC0000.max(ram.base),
                    });
                }
            }
        }
        Ok(())
    }

    fn check_firmware_services(&self) -> Result<(), ConfigError> {
        for (index, image) in self.images.iter().enumerate() {
            if image.kind != ImageKind::Rom && !image.provides.is_empty() {
                return Err(ConfigError::ServicesOnRamLoad { image: index });
            }
        }
        self.check_firmware_requirements(FIRMWARE_REQUIREMENTS, self.board.firmware_catalogue())
    }

    fn check_firmware_requirements(
        &self,
        requirements: &[FirmwareRequirement],
        catalogue: &[crate::FirmwareEntry],
    ) -> Result<(), ConfigError> {
        self.check_firmware_requirements_with_metadata(requirements, catalogue, &[])
    }

    pub(crate) fn draft_firmware_error(
        &self,
        known: &[(usize, &crate::FirmwareEntry)],
    ) -> Option<ConfigError> {
        self.check_firmware_requirements_with_metadata(
            FIRMWARE_REQUIREMENTS,
            self.board.firmware_catalogue(),
            known,
        )
        .err()
    }

    fn check_firmware_requirements_with_metadata(
        &self,
        requirements: &[FirmwareRequirement],
        catalogue: &[crate::FirmwareEntry],
        known: &[(usize, &crate::FirmwareEntry)],
    ) -> Result<(), ConfigError> {
        if self.board == BoardModel::Bare {
            return Ok(());
        }
        let services = self
            .images
            .iter()
            .enumerate()
            .filter(|(_, image)| image.kind == ImageKind::Rom)
            .map(|(index, image)| {
                if let Some((_, entry)) = known.iter().find(|(image, _)| *image == index) {
                    if !image.provides.is_empty()
                        && (image
                            .provides
                            .iter()
                            .any(|service| !entry.provides.contains(service))
                            || entry
                                .provides
                                .iter()
                                .any(|service| !image.provides.contains(service)))
                    {
                        let names = |services: &[crate::FirmwareService]| {
                            let mut names: Vec<_> =
                                services.iter().map(ToString::to_string).collect();
                            names.sort();
                            names.dedup();
                            format!("{names:?}")
                        };
                        return Err(ConfigError::FirmwareProvidesMismatch {
                            image_index: index,
                            firmware: entry.id,
                            declared: names(&image.provides),
                            expected: names(entry.provides),
                        });
                    }
                    Ok(Some(entry.provides))
                } else {
                    crate::firmware::image_services_from(self, index, catalogue)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Scan-window fallback is only a memory-map policy: an unknown BIOS
        // must never inherit a diagnostic ROM's permission to drive hardware.
        let diagnostic = known
            .iter()
            .find(|(index, _)| Some(*index) == self.system_bios_image)
            .map(|(_, entry)| *entry)
            .or_else(|| {
                crate::firmware::system_bios(self)
                    .ok()
                    .and_then(|bios| crate::firmware::matched_entry(bios, catalogue))
            });
        for device in &self.devices {
            for rule in requirements {
                if (rule.matches)(&device.kind)
                    && (!rule.needs_bound_drive
                        || self
                            .drives
                            .iter()
                            .any(|drive| drive.controller == device.id))
                    && !services
                        .iter()
                        .any(|services| services.is_none_or(|set| set.contains(&rule.service)))
                    && !diagnostic.is_some_and(|entry| {
                        entry.kind == crate::FirmwareKind::SystemBios
                            && entry.capabilities.contains(
                                &crate::FirmwareCapability::DirectHardwareTest(rule.service),
                            )
                    })
                {
                    return Err(ConfigError::NoProvider {
                        device: device.id,
                        service: rule.service,
                    });
                }
            }
        }
        Ok(())
    }

    fn check_rtc_ram(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            if let DeviceKind::Rtc(cfg) = &device.kind {
                if self.board.is_generic_isa() && cfg.ram != crate::CmosRam::Bytes128 {
                    return Err(ConfigError::BoardConfiguration {
                        board: self.board.id(),
                        reason: "requires a 128-byte CMOS RTC",
                    });
                }
                if cfg.ram == crate::CmosRam::Bytes128 {
                    if self.board == BoardModel::Ibm5170 {
                        return Err(ConfigError::Ibm5170RtcRam);
                    }
                    if matches!(
                        cfg.nvram,
                        crate::NvramSource::Image(_) | crate::NvramSource::Journal { .. }
                    ) {
                        return Err(ConfigError::UnsupportedExtendedNvram);
                    }
                }
            }
        }
        Ok(())
    }

    fn check_rtc_starts(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            if let DeviceKind::Rtc(cfg) = &device.kind {
                if let Some(field) = cfg.start.invalid_field() {
                    return Err(ConfigError::InvalidRtcStart {
                        device: device.id,
                        start: cfg.start,
                        field,
                    });
                }
            }
        }
        Ok(())
    }

    fn check_board(&self) -> Result<(), ConfigError> {
        if self.board == BoardModel::Bare {
            return Ok(());
        }
        let cpu_matches = if self.board == BoardModel::Generic486Isa {
            matches!(
                self.cpu.chip,
                crate::CpuChip::I80486Dx | crate::CpuChip::I80486DxCpuid
            )
        } else {
            self.cpu.chip == self.board.cpu_chip()
        };
        if !cpu_matches {
            return Err(ConfigError::BoardConfiguration {
                board: self.board.id(),
                reason: "CPU model does not match the board",
            });
        }
        if self.board == BoardModel::Generic486Isa && self.cpu.fpu != crate::FpuChip::Integrated {
            return Err(ConfigError::BoardConfiguration {
                board: self.board.id(),
                reason: "requires cpu.fpu = integrated",
            });
        }
        if self.board != BoardModel::Generic486Isa && self.cpu.fpu == crate::FpuChip::Integrated {
            return Err(ConfigError::BoardConfiguration {
                board: self.board.id(),
                reason: "integrated FPU requires an Intel486 DX board",
            });
        }
        if self.board == BoardModel::Generic386Isa && self.cpu.fpu != crate::FpuChip::None {
            return Err(ConfigError::BoardConfiguration {
                board: self.board.id(),
                reason: "requires cpu.fpu = none",
            });
        }
        let mut expected = self.board.motherboard_devices();
        let post = self.devices.iter().find(|d| d.id == DeviceId::Post);
        let mut post_port = None;
        if let Some(actual) = post {
            for standard in expected.iter_mut().filter(|d| d.id == DeviceId::Post) {
                if let DeviceKind::Post(cfg) = actual.kind {
                    post_port = Some(cfg.port);
                    standard.kind = actual.kind.clone();
                    standard.ports = vec![PortWindow::single(cfg.port)];
                }
                // POST determines which DMA page ports are free. Diagnose its
                // own configuration before using it to derive another device.
                self.check_board_wiring(actual, standard)?;
            }
        }
        expected.retain(|d| d.id != DeviceId::Post || post.is_some());
        for (index, standard) in expected.iter_mut().enumerate() {
            if standard.id == DeviceId::Dma && post_port != Some(0x80) {
                standard.ports[1] = PortWindow::new(0x80, 0x88);
            }
            let actual = self.devices.iter().find(|d| d.id == standard.id).ok_or(
                ConfigError::BoardDeviceMissing {
                    board: self.board,
                    device: standard.id,
                },
            )?;
            if self.devices[index].id != standard.id {
                return Err(ConfigError::BoardDeviceOrder {
                    board: self.board,
                    device: standard.id,
                    index,
                });
            }
            self.check_board_wiring(actual, standard)?;
        }
        for card in &self.devices[expected.len()..] {
            for (_, line) in card.interrupt_outputs() {
                if !matches!(line, 3..=7 | 9..=12 | 14..=15) {
                    return Err(ConfigError::BoardCardIrq {
                        board: self.board,
                        device: card.id,
                        line,
                    });
                }
            }
            if let Some(dma) = card.dma {
                if dma.controller != DeviceId::Dma {
                    return Err(ConfigError::BoardCardDmaController {
                        board: self.board,
                        device: card.id,
                        controller: dma.controller,
                    });
                }
                if dma.channel > 3 {
                    return Err(ConfigError::BoardCardDma {
                        board: self.board,
                        device: card.id,
                        channel: dma.channel,
                    });
                }
            }
        }
        Ok(())
    }

    fn check_mouse(&self) -> Result<(), ConfigError> {
        let targets = self
            .devices
            .iter()
            .filter(|device| {
                matches!(&device.kind,
            DeviceKind::Keyboard(cfg) if cfg.mouse)
                    || matches!(&device.kind, DeviceKind::Uart(cfg) if cfg.mouse)
            })
            .count();
        if targets > 1 {
            return Err(ConfigError::MultipleMouseTargets);
        }
        for device in &self.devices {
            if !matches!(&device.kind, DeviceKind::Keyboard(cfg) if cfg.mouse) {
                continue;
            }
            let reason = if device.id != DeviceId::Keyboard {
                "no 8042 at Keyboard".into()
            } else if !self.devices.iter().any(|d| {
                d.id == DeviceId::SlavePic
                    && matches!(&d.kind, DeviceKind::Pic(cfg) if !cfg.cascade)
            }) {
                "no slave PIC".into()
            } else if !matches!(
                self.board,
                BoardModel::Ibm5170 | BoardModel::Generic386Isa | BoardModel::Generic486Isa
            ) {
                format!("unsupported board {}", self.board.id())
            } else {
                continue;
            };
            return Err(ConfigError::UnsupportedMouse { reason });
        }
        Ok(())
    }

    fn check_board_wiring(
        &self,
        actual: &DeviceConfig,
        standard: &DeviceConfig,
    ) -> Result<(), ConfigError> {
        let mut expected = standard.clone();
        // Peripheral selection and RTC initialization leave motherboard wiring fixed.
        if matches!(
            (&actual.kind, &expected.kind),
            (DeviceKind::Rtc(_), DeviceKind::Rtc(_))
                | (DeviceKind::Keyboard(_), DeviceKind::Keyboard(_))
                | (DeviceKind::FwCfg(_), DeviceKind::FwCfg(_))
        ) {
            expected.kind = actual.kind.clone();
        }
        if actual == &expected {
            return Ok(());
        }
        let differences = [
            (
                "kind",
                format_board_kind(&expected.kind),
                format_board_kind(&actual.kind),
            ),
            (
                "ports",
                format_board_ports(&expected.ports),
                format_board_ports(&actual.ports),
            ),
            (
                "irq",
                format_board_irq(expected.irq),
                format_board_irq(actual.irq),
            ),
            (
                "dma",
                format_board_dma(expected.dma),
                format_board_dma(actual.dma),
            ),
            (
                "access_dependents",
                format!("{:?}", expected.access_dependents),
                format!("{:?}", actual.access_dependents),
            ),
            (
                "allow_port_overlap",
                format!("{:?}", expected.allow_port_overlap),
                format!("{:?}", actual.allow_port_overlap),
            ),
        ];
        let (field, expected, actual_value) = differences
            .into_iter()
            .find(|(_, expected, actual)| expected != actual)
            .unwrap_or_else(|| {
                (
                    "configuration",
                    format!("{expected:#x?}"),
                    format!("{actual:#x?}"),
                )
            });
        Err(ConfigError::BoardDeviceWiring {
            board: self.board,
            device: actual.id,
            field,
            expected,
            actual: actual_value,
        })
    }

    /// No [`DeviceId`] appears twice.
    fn check_unique_device_ids(&self) -> Result<(), ConfigError> {
        for (i, dev) in self.devices.iter().enumerate() {
            if self.devices[..i].iter().any(|d| d.id == dev.id) {
                if dev.id == DeviceId::Ide {
                    return Err(ConfigError::MultipleIdeChannels);
                }
                return Err(ConfigError::DuplicateDeviceId(dev.id));
            }
        }
        Ok(())
    }

    fn check_access_couplings(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            for &dependent in &device.access_dependents {
                if dependent == device.id {
                    return Err(ConfigError::SelfAccessCoupling { device: device.id });
                }
                if !self.devices.iter().any(|d| d.id == dependent) {
                    return Err(ConfigError::AccessDependentMissing {
                        device: device.id,
                        dependent,
                    });
                }
            }
        }
        Ok(())
    }

    fn check_fw_cfg(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            if let DeviceKind::FwCfg(cfg) = &device.kind {
                if !matches!(
                    self.board,
                    BoardModel::Bare | BoardModel::Generic386Isa | BoardModel::Generic486Isa
                ) {
                    return Err(ConfigError::FwCfgBoard);
                }
                if device.ports != crate::FwCfgCfg::port_windows()
                    || device.irq.is_some()
                    || device.dma.is_some()
                {
                    return Err(ConfigError::InvalidFwCfgResources { device: device.id });
                }
                crate::fw_cfg::files(self, device.id, cfg)?;
            }
        }
        Ok(())
    }

    fn check_sys_ctrl_port_a(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            if matches!(device.kind, DeviceKind::SysCtrlPortA) {
                if !matches!(
                    self.board,
                    BoardModel::Bare | BoardModel::Generic386Isa | BoardModel::Generic486Isa
                ) {
                    return Err(ConfigError::SysCtrlPortABoard);
                }
                if device.ports != [PortWindow::single(0x92)]
                    || device.irq.is_some()
                    || device.dma.is_some()
                {
                    return Err(ConfigError::InvalidSysCtrlPortAResources { device: device.id });
                }
            }
        }
        Ok(())
    }

    fn check_debug_console(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            if let DeviceKind::DebugConsole(cfg) = device.kind {
                if !matches!(
                    self.board,
                    BoardModel::Bare | BoardModel::Generic386Isa | BoardModel::Generic486Isa
                ) {
                    return Err(ConfigError::DebugConsoleBoard);
                }
                if device.ports != [PortWindow::single(cfg.port)]
                    || device.irq.is_some()
                    || device.dma.is_some()
                {
                    return Err(ConfigError::InvalidDebugConsoleResources { device: device.id });
                }
            }
        }
        Ok(())
    }

    /// The constructor's programmer-error guard must never reject validated data.
    fn check_uart_bases(&self) -> Result<(), ConfigError> {
        for dev in &self.devices {
            if let DeviceKind::Uart(uart) = &dev.kind {
                if !crate::descriptors::UART_PORT.contains(u32::from(uart.base)) {
                    return Err(ConfigError::UartPortWindowOutOfSpace {
                        device: dev.id,
                        base: uart.base,
                    });
                }
            }
        }
        Ok(())
    }

    fn check_lotech_ems_resources(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            if let DeviceKind::LotechEms(cfg) = device.kind {
                let windows: Vec<_> = self
                    .memory
                    .mmio
                    .iter()
                    .filter(|w| w.device == device.id)
                    .collect();
                if !cfg.is_valid()
                    || device.ports != cfg.port_windows()
                    || device.irq.is_some()
                    || device.dma.is_some()
                    || windows.len() != 1
                    || windows[0].base != u64::from(cfg.frame) << 4
                    || windows[0].size != 0x10000
                {
                    return Err(ConfigError::InvalidLotechEmsResources { device: device.id });
                }
                for other in self.devices.iter().filter(|other| other.id != device.id) {
                    for window in &device.ports {
                        if let Some(conflict) =
                            other.ports.iter().find(|port| window.overlaps(port))
                        {
                            return Err(ConfigError::PortOverlap {
                                a: device.id,
                                b: other.id,
                                port: window.start.max(conflict.start),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn check_sb16_resources(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            if let DeviceKind::Sb16(cfg) = device.kind {
                if !crate::descriptors::SB16_DMA16.contains(&cfg.dma16)
                    || !crate::descriptors::SB16_BASES.contains(&cfg.base)
                    || !device
                        .irq
                        .is_some_and(|irq| crate::descriptors::SB16_IRQS.contains(&irq))
                    || !device.dma.is_some_and(|dma| {
                        dma.controller == DeviceId::Dma
                            && crate::descriptors::SB16_DMA8.contains(&dma.channel)
                    })
                    || device.ports != cfg.port_windows()
                {
                    return Err(ConfigError::InvalidSb16Resources { device: device.id });
                }
            }
        }
        Ok(())
    }

    fn check_ide_resources(&self) -> Result<(), ConfigError> {
        for device in &self.devices {
            if let DeviceKind::Ide(cfg) = device.kind {
                if !crate::descriptors::IDE_PORTS.contains(&cfg.base)
                    || cfg.control != cfg.base + 0x206
                    || device.irq != Some(cfg.irq())
                    || device.dma.is_some()
                {
                    return Err(ConfigError::InvalidIdeResources { device: device.id });
                }
                if device.ports != cfg.port_windows() {
                    return Err(ConfigError::InvalidIdePortWindows { device: device.id });
                }
            }
        }
        Ok(())
    }

    fn check_port_widths(&self) -> Result<(), ConfigError> {
        self.check_port_width_support(WORD_PORT_SUPPORT)
    }

    fn check_port_width_support(&self, support: &[WordPortSupport]) -> Result<(), ConfigError> {
        for device in &self.devices {
            for window in &device.ports {
                if window.width == bus::PortWidth::Word && !window.start.is_multiple_of(2) {
                    return Err(ConfigError::UnalignedWordPortWindow {
                        device: device.id,
                        start: window.start,
                    });
                }
                if window.width == bus::PortWidth::Word {
                    for port in (window.start..=window.end).step_by(2) {
                        if !support
                            .iter()
                            .any(|rule| (rule.supports)(&device.kind, port))
                        {
                            return Err(ConfigError::UnsupportedWordPort {
                                device: device.id,
                                port,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// A flagged overlap must be strict containment with the same start, so
    /// `(start, end)` order gives every shared port to the narrower window.
    fn check_port_overlaps(&self) -> Result<(), ConfigError> {
        self.collect_port_overlaps()
            .into_iter()
            .next()
            .map_or(Ok(()), Err)
    }

    fn collect_port_overlaps(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();
        for (i, a) in self.devices.iter().enumerate() {
            for b in &self.devices[i + 1..] {
                for wa in &a.ports {
                    for wb in &b.ports {
                        if wa.overlaps(wb) {
                            let allowed = a.allow_port_overlap
                                || b.allow_port_overlap
                                || wa.allow_overlap
                                || wb.allow_overlap;
                            if !allowed {
                                errors.push(ConfigError::PortOverlap {
                                    a: a.id,
                                    b: b.id,
                                    port: wa.start.max(wb.start),
                                });
                            } else if wa.start != wb.start || wa.end == wb.end {
                                errors.push(ConfigError::PortOverlapDecodeOrder {
                                    a: a.id,
                                    b: b.id,
                                    a_window: *wa,
                                    b_window: *wb,
                                    port: wa.start.max(wb.start),
                                });
                            }
                        }
                    }
                }
            }
        }
        errors
    }

    /// Every MMIO window traps to a device that is present.
    fn check_mmio_devices_present(&self) -> Result<(), ConfigError> {
        for w in &self.memory.mmio {
            if !self.devices.iter().any(|d| d.id == w.device) {
                return Err(ConfigError::MmioDeviceMissing(w.device));
            }
        }
        Ok(())
    }

    fn check_mmio_sizes(&self) -> Result<(), ConfigError> {
        for window in &self.memory.mmio {
            if window.size == 0 {
                return Err(ConfigError::EmptyMmioWindow {
                    device: window.device,
                    base: window.base,
                });
            }
        }
        Ok(())
    }

    /// No two MMIO windows cover an overlapping address range.
    fn check_mmio_overlaps(&self) -> Result<(), ConfigError> {
        let mmio = &self.memory.mmio;
        for (i, a) in mmio.iter().enumerate() {
            for b in &mmio[i + 1..] {
                if ranges_overlap(a.base, a.end(), b.base, b.end()) {
                    return Err(ConfigError::MmioOverlap {
                        a: a.device,
                        b: b.device,
                        addr: a.base.max(b.base),
                    });
                }
            }
        }
        Ok(())
    }

    /// No trapped MMIO window overlaps a RAM region (the CPU core forbids it).
    fn check_mmio_vs_ram(&self) -> Result<(), ConfigError> {
        for w in &self.memory.mmio {
            for r in &self.memory.ram {
                if ranges_overlap(w.base, w.end(), r.base, r.end()) {
                    return Err(ConfigError::MmioOverlapsRam {
                        device: w.device,
                        addr: w.base.max(r.base),
                    });
                }
            }
        }
        Ok(())
    }

    /// Reserve the board's fixed high decode and enforce the same expansion-RAM
    /// range as machine files. IBM Personal Computer AT Technical Reference,
    /// Section 1 "System Board", System Memory Map (READ).
    fn check_board_memory(&self) -> Result<(), ConfigError> {
        if self.board.is_generic_isa() {
            return self.check_386_memory();
        }
        if self.board != BoardModel::Ibm5170 {
            return Ok(());
        }
        let bits = self.cpu.chip.physical_address_bits();
        if bits != 24 {
            return Err(ConfigError::BoardPhysicalWidth {
                board: self.board.id(),
                bits,
            });
        }
        let alias = crate::memory::AT_HIGH_ALIAS_BASE;
        for (base, end) in self
            .memory
            .ram
            .iter()
            .map(|r| (r.base, r.end()))
            .chain(self.memory.mmio.iter().map(|w| (w.base, w.end())))
            .chain(self.images.iter().map(|i| {
                (
                    i.base,
                    i.base.saturating_add(i.source.known_len().unwrap_or(0)),
                )
            }))
        {
            if base < end && end > crate::memory::AT_PHYSICAL_END {
                return Err(ConfigError::BoardPhysicalAddressOutOfRange {
                    board: self.board.id(),
                    addr: base.max(crate::memory::AT_PHYSICAL_END),
                });
            }
        }
        let limit = 0x100000 + u64::from(crate::IBM5170_MAX_EXTENDED_KIB) * 1024;
        for ram in &self.memory.ram {
            if ram.size > 0 && ram.base < alias && ram.end() > limit {
                return Err(ConfigError::ExtendedRamLimit {
                    board: self.board.id(),
                    max_kib: crate::IBM5170_MAX_EXTENDED_KIB,
                    addr: ram.base.max(limit),
                });
            }
        }
        for (base, end) in self
            .memory
            .ram
            .iter()
            .map(|r| (r.base, r.end()))
            .chain(self.memory.mmio.iter().map(|w| (w.base, w.end())))
            .chain(self.images.iter().map(|i| {
                (
                    i.base,
                    i.base.saturating_add(i.source.known_len().unwrap_or(0)),
                )
            }))
        {
            if base < end && ranges_overlap(base, end, alias, crate::memory::AT_PHYSICAL_END) {
                return Err(ConfigError::HighRomAliasOverlap {
                    board: self.board.id(),
                    addr: base.max(alias),
                });
            }
        }
        for window in &self.memory.mmio {
            if ranges_overlap(window.base, window.end(), 0xE0000, 0x100000) {
                if self.devices.iter().any(|device| {
                    device.id == window.device && matches!(device.kind, DeviceKind::LotechEms(_))
                }) {
                    return Err(ConfigError::LotechEmsFrameInSystemRom {
                        device: window.device,
                    });
                }
                return Err(ConfigError::HighRomAliasSourceMmio {
                    board: self.board.id(),
                    addr: window.base.max(0xE0000),
                });
            }
        }
        Ok(())
    }

    /// Intel 80386 Programmer's Reference Manual, §2.1 and §10.1: a 32-bit
    /// bus and a high reset address. SeaBIOS rel-1.17.0, shadow.c make_bios_writable
    /// needs low RAM independently of the pristine reboot source.
    fn check_386_memory(&self) -> Result<(), ConfigError> {
        let invalid = |reason| ConfigError::BoardConfiguration {
            board: self.board.id(),
            reason,
        };
        let covers = |base, end| {
            let mut cursor = base;
            while cursor < end {
                let next = self
                    .memory
                    .ram
                    .iter()
                    .filter(|r| r.base <= cursor && cursor < r.end())
                    .map(|r| r.end())
                    .max()
                    .unwrap_or(cursor);
                if next == cursor {
                    return false;
                }
                cursor = next;
            }
            true
        };
        if !covers(0, 0xA0000) || !covers(0xC0000, 0x100000) {
            return Err(invalid(
                "requires 640 KiB conventional RAM and writable shadow RAM at 0xc0000..0x100000",
            ));
        }
        let limit = 0x100000 + u64::from(crate::GENERIC386_MAX_EXTENDED_KIB) * 1024;
        for ram in &self.memory.ram {
            if ram.end() > limit {
                return Err(ConfigError::ExtendedRamLimit {
                    board: self.board.id(),
                    max_kib: crate::GENERIC386_MAX_EXTENDED_KIB,
                    addr: ram.base.max(limit),
                });
            }
            if ram.end() > 0x100000 && (ram.base != 0x100000 || !ram.size.is_multiple_of(65536)) {
                return Err(invalid("extended RAM must start at 1 MiB in 64 KiB steps"));
            }
        }
        for window in &self.memory.mmio {
            if window.end() > crate::memory::I386_PHYSICAL_END
                || ranges_overlap(
                    window.base,
                    window.end(),
                    crate::memory::I386_HIGH_BIOS_BASE,
                    crate::memory::I386_PHYSICAL_END,
                )
            {
                return Err(ConfigError::HighRomAliasOverlap {
                    board: self.board.id(),
                    addr: window.base,
                });
            }
        }
        for image in &self.images {
            let end = image
                .base
                .saturating_add(image.source.known_len().unwrap_or(0));
            if end > crate::memory::I386_PHYSICAL_END
                || ranges_overlap(
                    image.base,
                    end,
                    crate::memory::I386_HIGH_BIOS_BASE,
                    crate::memory::I386_PHYSICAL_END,
                )
            {
                return Err(ConfigError::HighRomAliasOverlap {
                    board: self.board.id(),
                    addr: image.base,
                });
            }
        }
        let bios = crate::firmware::system_bios(self)?;
        if bios.base != 0xE0000 || bios.source.known_len().is_some_and(|size| size != 0x20000) {
            return Err(invalid("requires a 128 KiB BIOS at 0xe0000"));
        }
        Ok(())
    }

    /// Every RAM region, MMIO window, and loaded image fits inside the address
    /// space.
    fn check_regions_in_space(&self) -> Result<(), ConfigError> {
        let space = self.memory.address_space_bytes;
        let check = |base: u64, size: u64| -> Result<(), ConfigError> {
            let end = base.saturating_add(size);
            if base > space || end > space {
                Err(ConfigError::RegionOutOfSpace { base, end, space })
            } else {
                Ok(())
            }
        };
        for r in &self.memory.ram {
            check(r.base, r.size)?;
        }
        for w in &self.memory.mmio {
            check(w.base, w.size)?;
        }
        for img in &self.images {
            check(img.base, img.source.known_len().unwrap_or(0))?;
        }
        Ok(())
    }

    /// No two devices drive the same PIC input line.
    fn check_irq_conflicts(&self) -> Result<(), ConfigError> {
        self.collect_irq_conflicts()
            .into_iter()
            .next()
            .map_or(Ok(()), Err)
    }

    fn collect_irq_conflicts(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();
        let routes: Vec<_> = self
            .devices
            .iter()
            .flat_map(|device| {
                device
                    .interrupt_outputs()
                    .map(move |(_, irq)| (device.id, irq))
            })
            .collect();
        for (i, &(a, line)) in routes.iter().enumerate() {
            if line >= 16 {
                errors.push(ConfigError::InvalidIrqLine { device: a, line });
            }
            for &(b, other) in &routes[i + 1..] {
                if other == line {
                    errors.push(ConfigError::IrqConflict { a, b, line });
                }
            }
        }
        errors
    }

    fn check_pic_faces(&self) -> Result<(), ConfigError> {
        for (i, a) in self.devices.iter().enumerate() {
            let DeviceKind::Pic(pic) = &a.kind else {
                continue;
            };
            for b in &self.devices[i + 1..] {
                if matches!(&b.kind, DeviceKind::Pic(other) if pic.cascade == other.cascade) {
                    return Err(ConfigError::DuplicatePicFace {
                        a: a.id,
                        b: b.id,
                        cascade: pic.cascade,
                    });
                }
            }
        }
        Ok(())
    }

    /// Check image overlaps and RAM coverage using lengths known without I/O.
    /// Metadata callers use [`Self::validate_logical_with_firmware`] to keep
    /// placement in the shared validation sequence. Assembly repeats this
    /// check after reading file sources.
    pub fn validate_image_placement(&self) -> Result<(), ConfigError> {
        for (i, image) in self.images.iter().enumerate() {
            let Some(size) = image.source.known_len() else {
                continue;
            };
            if size == 0 {
                continue;
            }
            let end = image.base.saturating_add(size);
            for window in &self.memory.mmio {
                if ranges_overlap(image.base, end, window.base, window.end()) {
                    return Err(ConfigError::ImageOverlapsMmio {
                        image_index: i,
                        device: window.device,
                        addr: image.base.max(window.base),
                    });
                }
            }
            for (j, other) in self.images.iter().enumerate().skip(i + 1) {
                let Some(other_size) = other.source.known_len() else {
                    continue;
                };
                if other_size > 0
                    && ranges_overlap(
                        image.base,
                        end,
                        other.base,
                        other.base.saturating_add(other_size),
                    )
                {
                    return Err(ConfigError::ImageOverlap {
                        a: i,
                        b: j,
                        addr: image.base.max(other.base),
                    });
                }
            }
            if image.kind == ImageKind::RamLoad {
                let mut covered = image.base;
                while covered < end {
                    let next = self
                        .memory
                        .ram
                        .iter()
                        .filter(|region| region.base <= covered && covered < region.end())
                        .map(|region| region.end())
                        .max()
                        .unwrap_or(covered);
                    if next == covered {
                        return Err(ConfigError::ImageOutsideRam {
                            image_index: i,
                            addr: covered,
                        });
                    }
                    covered = next;
                }
            }
        }
        Ok(())
    }

    /// Every device's DMA wiring names a controller that is present.
    fn check_dma_controllers_present(&self) -> Result<(), ConfigError> {
        self.collect_dma_controllers_present()
            .into_iter()
            .next()
            .map_or(Ok(()), Err)
    }

    fn collect_dma_controllers_present(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();
        for (index, dev) in self.devices.iter().enumerate() {
            for dma in dev.dma_inputs() {
                if dma.controller == DeviceId::DmaWord && !matches!(dma.channel, 5..=7) {
                    errors.push(ConfigError::InvalidDmaChannel {
                        device: dev.id,
                        controller: dma.controller,
                        channel: dma.channel,
                    });
                }
                for other in &self.devices[index + 1..] {
                    if other.dma_inputs().any(|input| input == dma) {
                        errors.push(ConfigError::DmaConflict {
                            a: dev.id,
                            b: other.id,
                            controller: dma.controller,
                            channel: dma.channel,
                        });
                    }
                }
                if !self.devices.iter().any(|d| {
                    d.id == dma.controller
                        && (dma.controller != DeviceId::DmaWord
                            || matches!(d.kind, DeviceKind::DmaWord(_)))
                }) {
                    errors.push(ConfigError::DmaControllerMissing {
                        requester: dev.id,
                        controller: dma.controller,
                    });
                }
            }
        }
        errors
    }

    /// Resolve each binding independently; counting all controller slots can
    /// silently attach media to a different device or overwrite another medium.
    fn check_journal_modes(&self) -> Result<(), ConfigError> {
        for (drive_index, drive) in self.drives.iter().enumerate() {
            if let crate::WriteMode::Journal {
                path,
                keep_generations,
            } = &drive.write
            {
                let message = if path.as_os_str().is_empty() {
                    Some("an explicit journal path is required")
                } else if *keep_generations == 0 {
                    Some("keep_generations must be positive")
                } else if !matches!(drive.image, crate::ImageSource::File(_))
                    && !(matches!(drive.kind, crate::DriveKind::HardDisk { .. })
                        && matches!(drive.image, crate::ImageSource::Blank { .. }))
                {
                    Some("journal mode requires a file-backed image")
                } else if match drive.kind {
                    crate::DriveKind::Floppy { geometry } => geometry.bytes_per_sector != 512,
                    crate::DriveKind::HardDisk { geometry } => geometry.bytes_per_sector != 512,
                } {
                    Some("journal mode requires 512-byte sectors")
                } else {
                    None
                };
                if let Some(message) = message {
                    return Err(ConfigError::Journal {
                        drive_index,
                        message: message.into(),
                    });
                }
            }
        }
        Ok(())
    }

    fn check_drive_controllers(&self) -> Result<(), ConfigError> {
        for (drive_index, drive) in self.drives.iter().enumerate() {
            let controller = self
                .devices
                .iter()
                .find(|d| d.id == drive.controller)
                .ok_or(ConfigError::DriveControllerMissing {
                    drive_index,
                    controller: drive.controller,
                })?;
            let available = match (drive.kind, &controller.kind) {
                (crate::DriveKind::Floppy { .. }, DeviceKind::Fdc(fdc)) => fdc.drive_slots.min(4),
                (crate::DriveKind::HardDisk { .. }, DeviceKind::Ide(_)) => 2,
                _ => {
                    return Err(ConfigError::DriveControllerKind {
                        drive_index,
                        controller: drive.controller,
                    })
                }
            };
            if drive.unit >= available {
                return Err(ConfigError::DriveUnitUnavailable {
                    drive_index,
                    controller: drive.controller,
                    unit: drive.unit,
                    available,
                });
            }
            if matches!(controller.kind, DeviceKind::Ide(_))
                && drive.unit == 1
                && !self
                    .drives
                    .iter()
                    .any(|other| other.controller == drive.controller && other.unit == 0)
            {
                return Err(ConfigError::AtaMissingMaster {
                    drive_index,
                    controller: drive.controller,
                });
            }
            if let Some(a) = self.drives[..drive_index]
                .iter()
                .position(|other| other.controller == drive.controller && other.unit == drive.unit)
            {
                return Err(ConfigError::DuplicateDriveUnit {
                    a,
                    b: drive_index,
                    controller: drive.controller,
                    unit: drive.unit,
                });
            }
        }
        Ok(())
    }

    /// Every drive's backing image (when its size is known without I/O) fits
    /// it: a fixed disk's geometry, or a medium the floppy drive type takes.
    fn check_image_geometry(&self) -> Result<(), ConfigError> {
        for (drive_index, drive) in self.drives.iter().enumerate() {
            if let crate::drive::DriveKind::Floppy { geometry } = drive.kind {
                if geometry.bytes_per_sector != 512 {
                    return Err(ConfigError::UnsupportedFloppySectorSize {
                        drive_index,
                        bytes_per_sector: geometry.bytes_per_sector,
                    });
                }
            }
            if let crate::DriveKind::HardDisk { geometry } = drive.kind {
                if !(1..=16383).contains(&geometry.cylinders)
                    || !(1..=16).contains(&geometry.heads)
                    || !(1..=63).contains(&geometry.sectors_per_track)
                    || geometry.bytes_per_sector != 512
                {
                    return Err(ConfigError::InvalidAtaGeometry { drive_index });
                }
            }
            if drive.media.is_some() {
                continue;
            }
            if let Some(actual) = drive.image.known_len() {
                drive.kind.check_image_len(drive_index, actual)?;
            }
        }
        Ok(())
    }
}

/// Locate a device by role in a config being mutated by a test.
#[cfg(test)]
fn find_device(
    config: &mut MachineConfig,
    id: crate::device::DeviceId,
) -> &mut crate::device::DeviceConfig {
    config
        .devices
        .iter_mut()
        .find(|d| d.id == id)
        .expect("device present")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::BoardModel;
    use crate::clock::ClockConfig;
    use crate::cpu::{CpuChip, CpuConfig, ResetSpec, SegmentSeed};
    use crate::device::{
        CmosCfg, DeviceConfig, DeviceId, DmaCfg, DmaSpec, FdcCfg, I8042Cfg, PicCfg, PitCfg,
        PortWindow, PostPortCfg, SysControlCfg, UartCfg, VgaTextCfg,
    };
    use crate::drive::{DriveConfig, DriveKind, FloppyGeometry, WriteMode};
    use crate::image::{ImageKind, ImageSource, LoadImage};
    use crate::memory::{MemoryConfig, MmioWindow, RamRegion};

    fn ems_config() -> MachineConfig {
        let mut config = valid_config();
        config.memory.ram = config
            .memory
            .ram
            .iter()
            .flat_map(|r| {
                [
                    (r.base, r.end().min(0xD0000)),
                    (r.base.max(0xE0000), r.end()),
                ]
            })
            .filter(|&(start, end)| start < end)
            .map(|(start, end)| RamRegion::new(start, end - start))
            .collect();
        let cfg = crate::LotechEmsCfg {
            frame: 0xD000,
            ..Default::default()
        };
        config.devices.push(DeviceConfig::new(
            DeviceId::LotechEms,
            DeviceKind::LotechEms(cfg),
            cfg.port_windows(),
        ));
        config
            .memory
            .mmio
            .push(MmioWindow::new(0xD0000, 0x10000, DeviceId::LotechEms));
        config
    }

    #[test]
    fn lotech_ems_e000_is_system_rom_on_at_and_allowed_on_bare() {
        let mut config = crate::bios(vec![0xF4; 0x8000], Some(vec![0; 1_474_560]));
        let cfg = crate::LotechEmsCfg::default();
        config.devices.push(DeviceConfig::new(
            DeviceId::LotechEms,
            DeviceKind::LotechEms(cfg),
            cfg.port_windows(),
        ));
        config
            .memory
            .mmio
            .push(MmioWindow::new(0xE0000, 0x10000, DeviceId::LotechEms));
        let error = config.validate().unwrap_err();
        assert!(matches!(
            error,
            ConfigError::LotechEmsFrameInSystemRom {
                device: DeviceId::LotechEms
            }
        ));
        assert!(error.to_string().contains("frame 0xD000"));
        config.board = BoardModel::Bare;
        config.validate().unwrap();
    }

    #[test]
    fn lotech_ems_rejects_ram_rom_and_mmio_frame_conflicts() {
        let config = ems_config();
        config.validate().unwrap();
        let mut ram = config.clone();
        ram.memory.ram.push(RamRegion::new(0xD0000, 0x1000));
        assert!(matches!(
            ram.validate(),
            Err(ConfigError::MmioOverlapsRam { .. })
        ));
        let mut rom = config.clone();
        rom.images.push(LoadImage {
            base: 0xD8000,
            source: ImageSource::Inline(vec![0; 2048]),
            kind: ImageKind::Rom,
            provides: Vec::new(),
        });
        assert!(matches!(
            rom.validate(),
            Err(ConfigError::ImageOverlapsMmio { .. })
        ));
        let mut mmio = config;
        mmio.memory
            .mmio
            .push(MmioWindow::new(0xD8000, 0x1000, DeviceId::Vga));
        assert!(matches!(
            mmio.validate(),
            Err(ConfigError::MmioOverlap { .. })
        ));
    }

    #[test]
    fn lotech_ems_alias_conflicts_cannot_use_priority_overlap() {
        let mut config = ems_config();
        let uart = find_device(&mut config, DeviceId::Uart);
        uart.kind = DeviceKind::Uart(UartCfg {
            base: 0x660,
            mouse: false,
        });
        uart.ports = vec![PortWindow::overlapping(0x660, 0x667)];
        uart.allow_port_overlap = true;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::PortOverlap { port: 0x660, .. })
        ));
    }

    #[test]
    fn lotech_ems_requires_exact_frame_alias_ports_and_no_irq_dma() {
        for mutation in 0..8 {
            let mut config = ems_config();
            let dev = config.devices.last_mut().unwrap();
            match mutation {
                0 => {
                    dev.ports.pop();
                }
                1 => dev.irq = Some(5),
                2 => {
                    dev.dma = Some(DmaSpec {
                        controller: DeviceId::Dma,
                        channel: 1,
                    })
                }
                3 => {
                    if let DeviceKind::LotechEms(ref mut cfg) = dev.kind {
                        cfg.port = 0xFFFF;
                        dev.ports = cfg.port_windows();
                    }
                }
                4 => {
                    if let DeviceKind::LotechEms(ref mut cfg) = dev.kind {
                        cfg.frame = 0xFFFF;
                    }
                }
                5 => {
                    if let DeviceKind::LotechEms(ref mut cfg) = dev.kind {
                        cfg.size_kib = 0;
                    }
                }
                6 => config.memory.mmio.last_mut().unwrap().size = 0x4000,
                _ => {
                    config.memory.mmio.pop();
                }
            }
            assert!(
                matches!(
                    config.validate(),
                    Err(ConfigError::InvalidLotechEmsResources { .. })
                ),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn drive_rom_containers_are_rejected_in_single_and_catalogued_media() {
        let mut config = valid_config();
        let container = ImageSource::RomContainer {
            bytes: vec![0; 2],
            offset: 1,
        };
        let expected = Err(ConfigError::FirmwareParts {
            message: "drives cannot use ROM containers".into(),
        });
        config.drives[0].image = container.clone();
        assert_eq!(config.check_part_sources(), expected);
        config.drives[0].media = Some(crate::FloppyMediaList {
            entries: vec![],
            empty: true,
            inserted: 0,
        });
        assert_eq!(config.check_part_sources(), expected);
        config.drives[0].image = ImageSource::Blank { size: 1_474_560 };
        let medium = crate::FloppyMedium {
            image: config.drives[0].image.clone(),
            expected_sha256: None,
            write: WriteMode::ReadOnly,
            name: "synthetic".into(),
        };
        for empty in [false, true] {
            for invalid in 0..2 {
                let mut entries = vec![medium.clone(), medium.clone()];
                entries[invalid].image = container.clone();
                config.drives[0].media = Some(crate::FloppyMediaList {
                    entries,
                    empty,
                    inserted: 0,
                });
                assert_eq!(
                    config.check_part_sources(),
                    expected,
                    "entry {invalid}, empty={empty}"
                );
            }
        }
    }

    #[test]
    fn media_lists_replace_the_legacy_parts_source() {
        let mut config = valid_config();
        config.drives[0].image = ImageSource::Interleaved(vec![]);
        assert_eq!(
            config.check_part_sources(),
            Err(ConfigError::FirmwareParts {
                message: "drives cannot use parts".into(),
            })
        );
        for entries in [
            vec![],
            vec![crate::FloppyMedium {
                image: ImageSource::Blank { size: 1_474_560 },
                expected_sha256: None,
                write: WriteMode::ReadOnly,
                name: "synthetic".into(),
            }],
        ] {
            config.drives[0].media = Some(crate::FloppyMediaList {
                entries,
                empty: true,
                inserted: 0,
            });
            config.check_part_sources().unwrap();
        }
    }

    #[test]
    fn port_a_is_confined_by_kind_and_rejects_invalid_resources() {
        for id in [DeviceId::SysCtrlPortA, DeviceId::Uart] {
            let mut config = board_config();
            config.devices.retain(|device| device.id != id);
            config.devices.push(DeviceConfig::new(
                id,
                DeviceKind::SysCtrlPortA,
                vec![PortWindow::single(0x92)],
            ));
            let error = config.validate().unwrap_err();
            assert_eq!(error, ConfigError::SysCtrlPortABoard);
            assert_eq!(error.to_string(), "sys-ctrl-port-a at 0x92 is absent on ibm-5170; it is allowed only on bare rigs and future generic-386-isa");
        }
        let mut config = crate::at_boot(vec![0xF4]);
        config.devices.push(DeviceConfig::new(
            DeviceId::SysCtrlPortA,
            DeviceKind::SysCtrlPortA,
            vec![PortWindow::single(0x92)],
        ));
        assert_eq!(config.validate(), Ok(()));
        let valid = config.devices.last().unwrap().clone();
        let mut invalid = Vec::new();
        let mut wrong = valid.clone();
        wrong.ports = vec![PortWindow::single(0x93)];
        invalid.push(wrong);
        let mut word = valid.clone();
        word.ports[0].width = bus::PortWidth::Word;
        invalid.push(word);
        let mut irq = valid.clone();
        irq.irq = Some(1);
        invalid.push(irq);
        let mut dma = valid;
        dma.dma = Some(DmaSpec {
            controller: DeviceId::Dma,
            channel: 2,
        });
        invalid.push(dma);
        for device in invalid {
            *config.devices.last_mut().unwrap() = device;
            assert_eq!(
                config.validate(),
                Err(ConfigError::InvalidSysCtrlPortAResources {
                    device: DeviceId::SysCtrlPortA
                })
            );
        }
    }

    #[test]
    fn vga_models_are_mutually_exclusive_before_identity_or_decode_checks() {
        for id in [DeviceId::Vga, DeviceId::Mda] {
            let mut config = crate::at_vga_text(vec![0xF4]);
            config.devices.push(DeviceConfig::new(
                id,
                DeviceKind::VgaRegisters(crate::VgaCfg {}),
                crate::VgaCfg::port_windows(),
            ));
            assert_eq!(
                config.validate().unwrap_err().to_string(),
                "vga and vga-text cards are mutually exclusive (shared ports and MMIO)"
            );
        }
    }

    #[test]
    fn debug_console_is_confined_by_device_kind_to_bare_rigs() {
        for id in [DeviceId::DebugConsole, DeviceId::Uart] {
            let mut config = board_config();
            config.devices.retain(|device| device.id != id);
            config.devices.push(DeviceConfig::new(
                id,
                DeviceKind::DebugConsole(crate::DebugConsoleCfg::default()),
                vec![PortWindow::single(0x402)],
            ));
            let error = config.validate().unwrap_err();
            assert_eq!(error, ConfigError::DebugConsoleBoard);
            assert_eq!(
                error.to_string(),
                "debug-console port is emulator-only; it is allowed only on board generic-386-isa"
            );
        }
        let mut config = crate::at_boot(vec![0xF4]);
        config.devices.push(DeviceConfig::new(
            DeviceId::DebugConsole,
            DeviceKind::DebugConsole(crate::DebugConsoleCfg { port: 0xE9 }),
            vec![PortWindow::single(0xE9)],
        ));
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn debug_console_requires_a_matching_byte_port_without_irq_or_dma() {
        let device = DeviceConfig::new(
            DeviceId::DebugConsole,
            DeviceKind::DebugConsole(crate::DebugConsoleCfg::default()),
            vec![PortWindow::single(0x402)],
        );
        let mut mismatched = device.clone();
        mismatched.ports = vec![PortWindow::single(0x403)];
        let mut wide = device.clone();
        wide.ports[0].width = bus::PortWidth::Word;
        let irq = device.clone().with_irq(5);
        let dma = device.with_dma(DmaSpec {
            controller: DeviceId::Dma,
            channel: 1,
        });
        for device in [mismatched, wide, irq, dma] {
            let mut config = crate::at_boot(vec![0xF4]);
            config.devices.push(device);
            assert_eq!(
                config.validate(),
                Err(ConfigError::InvalidDebugConsoleResources {
                    device: DeviceId::DebugConsole
                })
            );
        }
    }

    #[test]
    fn diagnostic_system_rom_tests_only_catalogued_controllers_without_services() {
        use crate::{FirmwareCapability, FirmwareEntry, FirmwareService};
        use sha2::{Digest, Sha256};

        let mut config = board_config();
        config.images[0].base = 0xF0000;
        config.images[0].source = ImageSource::Inline(vec![0; 65536]);
        config.images[0].provides.clear();
        let ide = crate::IdeCfg {
            model: devices::ata::AtaControllerModel::AtAdapter,
            base: 0x1F0,
            control: 0x3F6,
        };
        config.devices.push(
            DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(ide), ide.port_windows()).with_irq(14),
        );
        let diagnostic = FirmwareEntry {
            sha256: Box::leak(format!("{:x}", Sha256::digest(vec![0; 65536])).into_boxed_str()),
            ..crate::firmware::LANDMARK_ROM_POST_AT_1_2_0
        };
        let check = |entries: &[FirmwareEntry]| {
            config.check_firmware_requirements(FIRMWARE_REQUIREMENTS, entries)
        };
        assert_eq!(check(&[diagnostic]), Ok(()));
        assert_eq!(
            check(&[FirmwareEntry {
                capabilities: &[],
                ..diagnostic
            }]),
            Err(ConfigError::NoProvider {
                device: DeviceId::Fdc,
                service: FirmwareService::Floppy,
            })
        );
        assert_eq!(
            check(&[FirmwareEntry {
                capabilities: &[FirmwareCapability::DirectHardwareTest(
                    FirmwareService::Floppy
                )],
                ..diagnostic
            }]),
            Err(ConfigError::NoProvider {
                device: DeviceId::Ide,
                service: FirmwareService::FixedDisk,
            })
        );
        assert_eq!(
            check(&[crate::firmware::LANDMARK_ROM_POST_AT_1_2_0]),
            Err(ConfigError::NoProvider {
                device: DeviceId::Fdc,
                service: FirmwareService::Floppy,
            })
        );
        assert_eq!(
            check(&[FirmwareEntry {
                kind: crate::FirmwareKind::OptionRom,
                ..diagnostic
            }]),
            Err(ConfigError::NoProvider {
                device: DeviceId::Fdc,
                service: FirmwareService::Floppy,
            })
        );
        assert!(config.images[0].provides.is_empty());
        let adapter_catalogue = [FirmwareEntry {
            kind: crate::FirmwareKind::OptionRom,
            ..diagnostic
        }];
        for system_bios_image in [None, Some(0)] {
            let mut non_top = config.clone();
            non_top.images[0].base = 0xC8000;
            non_top.system_bios_image = system_bios_image;
            let selected = crate::firmware::system_bios(&non_top).unwrap();
            assert!(std::ptr::eq(selected, &non_top.images[0]));
            assert_eq!(
                crate::firmware::matched_entry(selected, &adapter_catalogue),
                Some(&adapter_catalogue[0])
            );
            assert_eq!(
                non_top.check_firmware_requirements(FIRMWARE_REQUIREMENTS, &adapter_catalogue),
                Err(ConfigError::NoProvider {
                    device: DeviceId::Fdc,
                    service: FirmwareService::Floppy,
                }),
                "non-top ROM selected with system_bios_image={system_bios_image:?}",
            );
        }
    }

    #[test]
    fn initialization_fdc_with_bound_drive_requires_floppy_provider() {
        let mut config = board_config();
        config.images[0].provides.clear();
        let error = ConfigError::NoProvider {
            device: DeviceId::Fdc,
            service: crate::FirmwareService::Floppy,
        };
        assert_eq!(config.validate(), Err(error.clone()));
        assert_eq!(
            error.to_string(),
            "device Fdc needs firmware service floppy; no ROM provides it"
        );
        config.images[0]
            .provides
            .push(crate::FirmwareService::FixedDisk);
        assert_eq!(config.validate(), Err(error));
        config.images.push(LoadImage {
            base: 0xC8000,
            source: ImageSource::Inline({
                let mut bytes = vec![0; 8192];
                bytes[..3].copy_from_slice(&[0x55, 0xAA, 16]);
                bytes[8191] = 0xF1;
                bytes
            }),
            kind: ImageKind::Rom,
            provides: vec![crate::FirmwareService::Floppy],
        });
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn initialization_bios_preset_declares_floppy_and_validates() {
        let config = board_config();
        assert_eq!(
            config.images[0].provides,
            vec![crate::FirmwareService::Floppy]
        );
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn initialization_fdc_without_bound_drive_needs_no_provider() {
        let mut config = board_config();
        config.images[0].provides.clear();
        config.drives.clear();
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn initialization_fdc_ignores_drives_bound_to_other_controllers() {
        let mut config = board_config();
        config.images[0].provides.clear();
        config.drives[0].controller = DeviceId::Uart;
        assert_eq!(config.check_firmware_services(), Ok(()));
    }

    #[test]
    fn initialization_bare_rig_skips_firmware_requirements() {
        let mut config = board_config();
        config.images[0].provides.clear();
        config.board = BoardModel::Bare;
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn initialization_requirement_table_supports_a_service_without_media() {
        let mut config = board_config();
        let rules = [FirmwareRequirement {
            matches: |kind| matches!(kind, DeviceKind::Uart(_)),
            service: crate::FirmwareService::FixedDisk,
            needs_bound_drive: false,
        }];
        assert_eq!(
            config.check_firmware_requirements(&rules, config.board.firmware_catalogue()),
            Err(ConfigError::NoProvider {
                device: DeviceId::Uart,
                service: crate::FirmwareService::FixedDisk,
            })
        );
        config.images[0]
            .provides
            .push(crate::FirmwareService::FixedDisk);
        assert_eq!(
            config.check_firmware_requirements(&rules, config.board.firmware_catalogue()),
            Ok(())
        );
    }

    #[test]
    fn initialization_ram_load_cannot_provide_firmware_services() {
        let mut config = crate::at_boot(vec![0xF4]);
        config.images[0]
            .provides
            .push(crate::FirmwareService::FixedDisk);
        let error = ConfigError::ServicesOnRamLoad { image: 0 };
        assert_eq!(config.validate(), Err(error.clone()));
        assert_eq!(
            error.to_string(),
            "RAM load image 0 cannot provide firmware services"
        );
    }

    #[test]
    fn initialization_invalid_rtc_calendar_is_rejected_before_assembly() {
        let valid = crate::RtcStart {
            year: 2024,
            month: 2,
            day: 29,
            hour: 23,
            minute: 59,
            second: 59,
        };
        for (start, field, formatted, range) in [
            (
                crate::RtcStart {
                    year: 1979,
                    ..valid
                },
                "year",
                "1979-02-29T23:59:59",
                "1980-2099",
            ),
            (
                crate::RtcStart {
                    year: 2100,
                    ..valid
                },
                "year",
                "2100-02-29T23:59:59",
                "1980-2099",
            ),
            (
                crate::RtcStart {
                    year: 2023,
                    ..valid
                },
                "day",
                "2023-02-29T23:59:59",
                "1-28 for the selected month and year",
            ),
            (
                crate::RtcStart { month: 0, ..valid },
                "month",
                "2024-00-29T23:59:59",
                "1-12",
            ),
            (
                crate::RtcStart { month: 13, ..valid },
                "month",
                "2024-13-29T23:59:59",
                "1-12",
            ),
            (
                crate::RtcStart { day: 0, ..valid },
                "day",
                "2024-02-00T23:59:59",
                "1-29 for the selected month and year",
            ),
            (
                crate::RtcStart { day: 30, ..valid },
                "day",
                "2024-02-30T23:59:59",
                "1-29 for the selected month and year",
            ),
            (
                crate::RtcStart {
                    month: 4,
                    day: 31,
                    ..valid
                },
                "day",
                "2024-04-31T23:59:59",
                "1-30 for the selected month and year",
            ),
            (
                crate::RtcStart {
                    month: 1,
                    day: 32,
                    ..valid
                },
                "day",
                "2024-01-32T23:59:59",
                "1-31 for the selected month and year",
            ),
            (
                crate::RtcStart { hour: 24, ..valid },
                "hour",
                "2024-02-29T24:59:59",
                "0-23",
            ),
            (
                crate::RtcStart {
                    minute: 60,
                    ..valid
                },
                "minute",
                "2024-02-29T23:60:59",
                "0-59",
            ),
            (
                crate::RtcStart {
                    second: 60,
                    ..valid
                },
                "second",
                "2024-02-29T23:59:60",
                "0-59",
            ),
        ] {
            let mut config = crate::at_rtc(vec![0xF4], 100, 10);
            let rtc = config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Rtc)
                .unwrap();
            let DeviceKind::Rtc(ref mut cfg) = rtc.kind else {
                panic!("RTC kind")
            };
            cfg.start = start;
            let error = ConfigError::InvalidRtcStart {
                device: DeviceId::Rtc,
                start,
                field,
            };
            assert_eq!(config.validate(), Err(error.clone()));
            assert!(
                matches!(crate::assemble(&config), Err(crate::AssemblyError::Config(actual))
                if actual == error)
            );
            assert_eq!(
                error.to_string(),
                format!("device Rtc has invalid RTC start {formatted}: invalid {field}; expected {range}")
            );
        }
        assert_eq!(valid.invalid_field(), None);
        assert_eq!(valid.weekday(), 5);
        assert_eq!(
            crate::RtcStart {
                year: 2099,
                month: 12,
                day: 31,
                ..valid
            }
            .weekday(),
            5
        );
    }

    fn board_config() -> MachineConfig {
        crate::bios(
            vec![0; 0x8000],
            Some(vec![0; FloppyGeometry::HD_1_44.image_bytes() as usize]),
        )
    }

    fn board_error(config: &MachineConfig, expected: ConfigError, message: &str) {
        let error = config.validate().expect_err("invalid board or binding");
        assert_eq!(error, expected);
        assert_eq!(error.to_string(), message);
    }

    #[test]
    fn board_missing_role_has_exact_message() {
        let mut config = board_config();
        config.devices.retain(|d| d.id != DeviceId::Keyboard);
        board_error(
            &config,
            ConfigError::BoardDeviceMissing {
                board: BoardModel::Ibm5170,
                device: DeviceId::Keyboard,
            },
            "board Ibm5170 requires device Keyboard",
        );
        config.board = BoardModel::Bare;
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn board_requires_dma_for_decoded_fdc() {
        let mut config = board_config();
        config.devices.retain(|d| d.id != DeviceId::Dma);
        board_error(
            &config,
            ConfigError::BoardDeviceMissing {
                board: BoardModel::Ibm5170,
                device: DeviceId::Dma,
            },
            "board Ibm5170 requires device Dma",
        );
    }

    #[test]
    fn board_order_has_exact_message() {
        let mut config = board_config();
        config.devices.swap(0, 1);
        board_error(
            &config,
            ConfigError::BoardDeviceOrder {
                board: BoardModel::Ibm5170,
                device: DeviceId::MasterPic,
                index: 0,
            },
            "board Ibm5170 requires device MasterPic at index 0",
        );
        config.board = BoardModel::Bare;
        assert_eq!(config.validate(), Ok(()));
        let mut config = board_config();
        config.devices.swap(9, 10);
        board_error(
            &config,
            ConfigError::BoardDeviceOrder {
                board: BoardModel::Ibm5170,
                device: DeviceId::Post,
                index: 9,
            },
            "board Ibm5170 requires device Post at index 9",
        );
    }

    fn wiring_error(
        device: DeviceId,
        field: &'static str,
        expected: &str,
        actual: &str,
    ) -> ConfigError {
        ConfigError::BoardDeviceWiring {
            board: BoardModel::Ibm5170,
            device,
            field,
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    #[test]
    fn board_wiring_has_exact_message_for_kind_ports_irq_and_dma() {
        for (change, field, expected, actual) in [
            (0, "kind", "Pit(PitCfg)", "SysControl(SysControlCfg)"),
            (1, "ports", "[0x40-0x43]", "[0x44-0x47]"),
            (2, "irq", "Some(0x0)", "Some(0x3)"),
            (
                3,
                "dma",
                "None",
                "Some(DmaSpec { controller: Dma, channel: 0x1 })",
            ),
        ] {
            let mut config = board_config();
            let pit = config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Pit)
                .unwrap();
            match change {
                0 => pit.kind = DeviceKind::SysControl(SysControlCfg {}),
                1 => pit.ports = vec![PortWindow::new(0x44, 0x47)],
                2 => pit.irq = Some(3),
                _ => {
                    pit.dma = Some(DmaSpec {
                        controller: DeviceId::Dma,
                        channel: 1,
                    })
                }
            }
            board_error(
                &config,
                wiring_error(DeviceId::Pit, field, expected, actual),
                &format!(
                    "board Ibm5170 device Pit field {field}: expected {expected}, got {actual}"
                ),
            );
            config.board = BoardModel::Bare;
            assert_eq!(config.validate(), Ok(()));
        }
        let mut config = board_config();
        config.devices[0].kind = DeviceKind::Pic(PicCfg { cascade: false });
        board_error(&config, wiring_error(DeviceId::MasterPic, "kind",
            "Pic(PicCfg { cascade: true })", "Pic(PicCfg { cascade: false })"),
            "board Ibm5170 device MasterPic field kind: expected Pic(PicCfg { cascade: true }), got Pic(PicCfg { cascade: false })");
    }

    #[test]
    fn board_wiring_numeric_kind_parameters_are_hexadecimal() {
        for (kind, actual) in [
            (
                DeviceKind::Uart(UartCfg { base: 0x3F8, mouse: false }),
                "Uart(UartCfg { base: 0x3f8, mouse: false })",
            ),
            (
                DeviceKind::Post(PostPortCfg { port: 0x190 }),
                "Post(PostPortCfg { port: 0x190 })",
            ),
            (
                DeviceKind::Fdc(FdcCfg { drive_slots: 2 }),
                "Fdc(FdcCfg { drive_slots: 0x2 })",
            ),
            (
                DeviceKind::Rtc(CmosCfg {
                    ram: crate::CmosRam::Bytes64,
                    nvram: crate::NvramSource::Blank,
                    start: crate::RtcStart::default(),
                    timing: Some((0x1234, 0x56)),
                }),
                "Rtc(CmosCfg { start: RtcStart { year: 2000, month: 1, day: 1, hour: 0, minute: 0, second: 0 }, nvram: Blank, timing: Some((0x1234, 0x56)) })",
            ),
        ] {
            let mut config = board_config();
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Pit)
                .unwrap()
                .kind = kind;
            board_error(
                &config,
                wiring_error(DeviceId::Pit, "kind", "Pit(PitCfg)", actual),
                &format!("board Ibm5170 device Pit field kind: expected Pit(PitCfg), got {actual}"),
            );
        }
    }

    #[test]
    fn board_wiring_dma_port_windows_are_hexadecimal() {
        let mut config = board_config();
        config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Dma)
            .unwrap()
            .ports[1]
            .start = 0x82;
        board_error(&config, wiring_error(DeviceId::Dma, "ports", "[0x0-0xf, 0x81-0x88]", "[0x0-0xf, 0x82-0x88]"),
            "board Ibm5170 device Dma field ports: expected [0x0-0xf, 0x81-0x88], got [0x0-0xf, 0x82-0x88]");
    }

    #[test]
    fn board_wiring_port_diagnostics_preserve_overlap_allowances() {
        let mut config = board_config();
        config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Pit)
            .unwrap()
            .ports[0]
            .allow_overlap = true;
        board_error(&config, wiring_error(DeviceId::Pit, "ports", "[0x40-0x43]", "[0x40-0x43 (overlap allowed)]"),
            "board Ibm5170 device Pit field ports: expected [0x40-0x43], got [0x40-0x43 (overlap allowed)]");
    }

    #[test]
    fn board_requires_dma_even_with_inert_fdc() {
        let mut config = board_config();
        let fdc = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Fdc)
            .unwrap();
        fdc.ports.clear();
        fdc.irq = None;
        fdc.dma = None;
        config.devices.retain(|d| d.id != DeviceId::Dma);
        board_error(
            &config,
            ConfigError::BoardDeviceMissing {
                board: BoardModel::Ibm5170,
                device: DeviceId::Dma,
            },
            "board Ibm5170 requires device Dma",
        );
    }

    #[test]
    fn board_overlap_allowance_cannot_hide_a_card_on_pic_ports() {
        let mut config = board_config();
        config.devices[0].allow_port_overlap = true;
        let uart = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Uart)
            .unwrap();
        uart.kind = DeviceKind::Uart(UartCfg {
            base: 0x20,
            mouse: false,
        });
        uart.ports = vec![PortWindow::new(0x20, 0x27)];
        board_error(
            &config,
            wiring_error(DeviceId::MasterPic, "allow_port_overlap", "false", "true"),
            "board Ibm5170 device MasterPic field allow_port_overlap: expected false, got true",
        );
    }

    #[test]
    fn board_access_dependents_must_match_the_board_data() {
        let mut config = board_config();
        config.devices[0].access_dependents = vec![DeviceId::Pit];
        board_error(
            &config,
            wiring_error(DeviceId::MasterPic, "access_dependents", "[]", "[Pit]"),
            "board Ibm5170 device MasterPic field access_dependents: expected [], got [Pit]",
        );
    }

    #[test]
    fn board_post_is_validated_before_deriving_dma_pages() {
        let mut config = board_config();
        let post = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Post)
            .unwrap();
        post.kind = DeviceKind::Pit(PitCfg {});
        board_error(&config, wiring_error(DeviceId::Post, "kind", "Post(PostPortCfg { port: 0x80 })", "Pit(PitCfg)"),
            "board Ibm5170 device Post field kind: expected Post(PostPortCfg { port: 0x80 }), got Pit(PitCfg)");
    }

    #[test]
    fn board_post_decode_error_names_expected_and_actual_ports() {
        let mut config = board_config();
        let post = config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Post)
            .unwrap();
        post.ports = vec![PortWindow::single(0x190)];
        board_error(
            &config,
            wiring_error(DeviceId::Post, "ports", "[0x80]", "[0x190]"),
            "board Ibm5170 device Post field ports: expected [0x80], got [0x190]",
        );
    }

    #[test]
    fn board_card_dma_must_name_the_board_8237() {
        let mut config = board_config();
        config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Fdc)
            .unwrap()
            .dma
            .as_mut()
            .unwrap()
            .controller = DeviceId::Uart;
        board_error(
            &config,
            ConfigError::BoardCardDmaController {
                board: BoardModel::Ibm5170,
                device: DeviceId::Fdc,
                controller: DeviceId::Uart,
            },
            "board Ibm5170 requires dma controller Dma for card Fdc, got Uart",
        );
    }

    #[test]
    fn board_card_irq_has_exact_message() {
        for line in [0, 1, 2, 8, 13, 16] {
            let mut config = board_config();
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Uart)
                .unwrap()
                .irq = Some(line);
            board_error(
                &config,
                ConfigError::BoardCardIrq {
                    board: BoardModel::Ibm5170,
                    device: DeviceId::Uart,
                    line,
                },
                &format!("board Ibm5170 does not offer irq line {line} to card Uart"),
            );
        }
        for line in [3, 4, 5, 7, 9, 10, 11, 12, 14, 15] {
            let mut config = board_config();
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Uart)
                .unwrap()
                .irq = Some(line);
            assert_eq!(config.validate(), Ok(()));
        }
        let mut config = valid_config();
        config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Uart)
            .unwrap()
            .irq = Some(13);
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn board_card_dma_has_exact_message() {
        for channel in [4, 7, 255] {
            let mut config = board_config();
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Fdc)
                .unwrap()
                .dma
                .as_mut()
                .unwrap()
                .channel = channel;
            board_error(
                &config,
                ConfigError::BoardCardDma {
                    board: BoardModel::Ibm5170,
                    device: DeviceId::Fdc,
                    channel,
                },
                &format!("board Ibm5170 does not offer dma channel {channel} to card Fdc"),
            );
            config.board = BoardModel::Bare;
            assert_eq!(config.validate(), Ok(()));
        }
        for channel in 0..=3 {
            let mut config = board_config();
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Fdc)
                .unwrap()
                .dma
                .as_mut()
                .unwrap()
                .channel = channel;
            assert_eq!(config.validate(), Ok(()));
        }
    }

    #[test]
    fn board_post_and_rtc_options_preserve_standard_wiring() {
        for post_port in [None, Some(0x190)] {
            let mut config = board_config();
            if let Some(port) = post_port {
                let post = config
                    .devices
                    .iter_mut()
                    .find(|d| d.id == DeviceId::Post)
                    .unwrap();
                post.kind = DeviceKind::Post(PostPortCfg { port });
                post.ports = vec![PortWindow::single(port)];
            } else {
                config.devices.retain(|d| d.id != DeviceId::Post);
            }
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Dma)
                .unwrap()
                .ports[1] = PortWindow::new(0x80, 0x88);
            config
                .devices
                .iter_mut()
                .find(|d| d.id == DeviceId::Rtc)
                .unwrap()
                .kind = DeviceKind::Rtc(CmosCfg {
                ram: crate::CmosRam::Bytes64,
                nvram: crate::NvramSource::Blank,
                start: crate::RtcStart::default(),
                timing: Some((20, 2)),
            });
            assert_eq!(config.validate(), Ok(()));
        }
        assert_eq!(BoardModel::Ibm5170.class(), Some(crate::MachineClass::At));
        assert_eq!(BoardModel::Bare.class(), None);
        assert!(BoardModel::Bare.motherboard_devices().is_empty());
        for floppy in [
            None,
            Some(vec![0; FloppyGeometry::HD_1_44.image_bytes() as usize]),
        ] {
            assert_eq!(crate::bios(vec![0; 0x8000], floppy).validate(), Ok(()));
        }
    }

    #[test]
    fn drive_binding_missing_controller_has_exact_message() {
        let mut config = valid_config();
        config.drives[0].controller = DeviceId::Mda;
        board_error(
            &config,
            ConfigError::DriveControllerMissing {
                drive_index: 0,
                controller: DeviceId::Mda,
            },
            "drive 0 binds to absent controller Mda",
        );
    }

    #[test]
    fn drive_binding_wrong_controller_kind_has_exact_message() {
        let mut config = valid_config();
        config.drives[0].controller = DeviceId::Uart;
        board_error(
            &config,
            ConfigError::DriveControllerKind {
                drive_index: 0,
                controller: DeviceId::Uart,
            },
            "drive 0 kind is incompatible with controller Uart",
        );
        config.drives[0].controller = DeviceId::Fdc;
        config.drives[0].kind = crate::DriveKind::HardDisk {
            geometry: crate::HardDiskGeometry {
                cylinders: 80,
                heads: 2,
                sectors_per_track: 18,
                bytes_per_sector: 512,
            },
        };
        board_error(
            &config,
            ConfigError::DriveControllerKind {
                drive_index: 0,
                controller: DeviceId::Fdc,
            },
            "drive 0 kind is incompatible with controller Fdc",
        );
    }

    #[test]
    fn drive_binding_unit_has_exact_message_and_hardware_bound() {
        let mut config = valid_config();
        config.drives[0].unit = 2;
        board_error(
            &config,
            ConfigError::DriveUnitUnavailable {
                drive_index: 0,
                controller: DeviceId::Fdc,
                unit: 2,
                available: 2,
            },
            "drive 0 binds to controller Fdc unit 2 but only 2 unit(s) exist",
        );
        config
            .devices
            .iter_mut()
            .find(|d| d.id == DeviceId::Fdc)
            .unwrap()
            .kind = DeviceKind::Fdc(FdcCfg { drive_slots: 8 });
        config.drives[0].unit = 4;
        board_error(
            &config,
            ConfigError::DriveUnitUnavailable {
                drive_index: 0,
                controller: DeviceId::Fdc,
                unit: 4,
                available: 4,
            },
            "drive 0 binds to controller Fdc unit 4 but only 4 unit(s) exist",
        );
    }

    #[test]
    fn drive_binding_duplicate_unit_has_exact_message() {
        let mut config = valid_config();
        config.drives.push(config.drives[0].clone());
        board_error(
            &config,
            ConfigError::DuplicateDriveUnit {
                a: 0,
                b: 1,
                controller: DeviceId::Fdc,
                unit: 0,
            },
            "drives 0 and 1 bind to controller Fdc unit 0",
        );
        config.drives[1].unit = 1;
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn access_couplings_require_present_distinct_devices() {
        for (dependent, expected, message) in [
            (
                DeviceId::Uart,
                ConfigError::SelfAccessCoupling {
                    device: DeviceId::Uart,
                },
                "device Uart cannot declare itself as an access dependent",
            ),
            (
                DeviceId::Mda,
                ConfigError::AccessDependentMissing {
                    device: DeviceId::Uart,
                    dependent: DeviceId::Mda,
                },
                "device Uart declares absent access dependent Mda",
            ),
        ] {
            let mut config = valid_config();
            find_device(&mut config, DeviceId::Uart)
                .access_dependents
                .push(dependent);
            assert_eq!(config.validate(), Err(expected));
            assert_eq!(config.validate().unwrap_err().to_string(), message);
        }
    }

    #[test]
    fn isa_word_window_validation_rejects_odd_start() {
        let mut config = valid_config();
        find_device(&mut config, DeviceId::Post).ports =
            vec![PortWindow::single(0x191).with_width(bus::PortWidth::Word)];
        assert_eq!(
            config.validate(),
            Err(ConfigError::UnalignedWordPortWindow {
                device: DeviceId::Post,
                start: 0x191,
            })
        );
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "16-bit port window for Post starts at odd port 0x191"
        );
        find_device(&mut config, DeviceId::Post).ports[0].width = bus::PortWidth::Byte;
        assert_eq!(config.validate(), Ok(()));
    }

    const SYNTHETIC_WORD_PORT_SUPPORT: &[WordPortSupport] = &[WordPortSupport {
        supports: |kind, port| {
            matches!(kind, DeviceKind::Post(_)) && [0x190, 0x192].contains(&port)
        },
    }];

    #[test]
    fn isa_synthetic_word_window_accepts_single_even_port_and_odd_end() {
        let mut config = valid_config();
        for window in [PortWindow::single(0x190), PortWindow::new(0x190, 0x193)] {
            find_device(&mut config, DeviceId::Post).ports =
                vec![window.with_width(bus::PortWidth::Word)];
            assert_eq!(
                config.check_port_width_support(SYNTHETIC_WORD_PORT_SUPPORT),
                Ok(())
            );
        }
    }

    #[test]
    fn isa_byte_only_kinds_reject_word_windows() {
        let mut baseline = valid_config();
        baseline.devices.push(DeviceConfig::new(
            DeviceId::Mda,
            DeviceKind::Mda(crate::MdaTextCfg {}),
            Vec::new(),
        ));
        for device in &baseline.devices {
            let mut config = baseline.clone();
            find_device(&mut config, device.id).ports =
                vec![PortWindow::single(0x190).with_width(bus::PortWidth::Word)];
            assert_eq!(
                config.validate(),
                Err(ConfigError::UnsupportedWordPort {
                    device: device.id,
                    port: 0x190,
                })
            );
        }
    }

    #[test]
    fn isa_synthetic_word_support_is_scoped_to_kind_and_register() {
        for (id, port) in [(DeviceId::Post, 0x194), (DeviceId::Pit, 0x190)] {
            let mut config = valid_config();
            find_device(&mut config, id).ports =
                vec![PortWindow::single(port).with_width(bus::PortWidth::Word)];
            assert_eq!(
                config.check_port_width_support(SYNTHETIC_WORD_PORT_SUPPORT),
                Err(ConfigError::UnsupportedWordPort { device: id, port })
            );
        }
        let mut config = valid_config();
        find_device(&mut config, DeviceId::Post).ports =
            vec![PortWindow::new(0x190, 0x195).with_width(bus::PortWidth::Word)];
        assert_eq!(
            config.check_port_width_support(SYNTHETIC_WORD_PORT_SUPPORT),
            Err(ConfigError::UnsupportedWordPort {
                device: DeviceId::Post,
                port: 0x194
            })
        );
    }

    #[test]
    fn isa_board_validation_rejects_word_width_on_byte_hardware() {
        let mut config = board_config();
        find_device(&mut config, DeviceId::Pit).ports[0].width = bus::PortWidth::Word;
        board_error(
            &config,
            wiring_error(
                DeviceId::Pit,
                "ports",
                "[0x40-0x43]",
                "[0x40-0x43 (16-bit)]",
            ),
            "board Ibm5170 device Pit field ports: expected [0x40-0x43], got [0x40-0x43 (16-bit)]",
        );
    }

    #[test]
    fn access_couplings_allow_duplicates_and_directed_cycles() {
        let mut config = valid_config();
        find_device(&mut config, DeviceId::Uart).access_dependents =
            vec![DeviceId::Pit, DeviceId::Pit];
        find_device(&mut config, DeviceId::Pit).access_dependents = vec![DeviceId::Uart];
        assert_eq!(config.validate(), Ok(()));
    }

    const SPACE: u64 = 0x10_0000;
    const TEXT_BASE: u64 = 0xB_8000;
    const TEXT_LEN: u64 = 0x8000;
    const ROM_BASE: u64 = 0xF_8000;
    const ROM_LEN: u64 = 0x8000;

    fn dev(id: DeviceId, kind: DeviceKind, ports: Vec<PortWindow>) -> DeviceConfig {
        DeviceConfig::new(id, kind, ports)
    }

    /// A complete, internally consistent AT-class machine used as the baseline;
    /// each malformed-config test mutates one aspect of a clone.
    fn valid_config() -> MachineConfig {
        let devices = vec![
            dev(
                DeviceId::Uart,
                DeviceKind::Uart(UartCfg {
                    base: 0x3F8,
                    mouse: false,
                }),
                vec![PortWindow::new(0x3F8, 0x3FF)],
            ),
            dev(
                DeviceId::Pit,
                DeviceKind::Pit(PitCfg {}),
                vec![PortWindow::new(0x40, 0x43)],
            )
            .with_irq(0),
            dev(
                DeviceId::MasterPic,
                DeviceKind::Pic(PicCfg { cascade: true }),
                vec![PortWindow::new(0x20, 0x21)],
            ),
            dev(
                DeviceId::SlavePic,
                DeviceKind::Pic(PicCfg { cascade: false }),
                vec![PortWindow::new(0xA0, 0xA1)],
            ),
            dev(
                DeviceId::Keyboard,
                DeviceKind::Keyboard(I8042Cfg::default()),
                vec![PortWindow::single(0x60), PortWindow::single(0x64)],
            )
            .with_irq(1),
            dev(
                DeviceId::Rtc,
                DeviceKind::Rtc(CmosCfg {
                    ram: crate::CmosRam::Bytes64,
                    nvram: crate::NvramSource::Derived,
                    start: crate::RtcStart::default(),
                    timing: None,
                }),
                vec![PortWindow::new(0x70, 0x71)],
            )
            .with_irq(8),
            dev(
                DeviceId::Vga,
                DeviceKind::Vga(VgaTextCfg {}),
                vec![PortWindow::new(0x3D4, 0x3D5)],
            ),
            dev(
                DeviceId::SysControl,
                DeviceKind::SysControl(SysControlCfg {}),
                vec![PortWindow::single(0x61)],
            ),
            dev(
                DeviceId::Post,
                DeviceKind::Post(PostPortCfg { port: 0x190 }),
                vec![PortWindow::single(0x190)],
            ),
            dev(
                DeviceId::Fdc,
                DeviceKind::Fdc(FdcCfg { drive_slots: 2 }),
                vec![PortWindow::new(0x3F0, 0x3F7)],
            )
            .with_irq(6)
            .with_dma(DmaSpec {
                controller: DeviceId::Dma,
                channel: 2,
            }),
            dev(
                DeviceId::Dma,
                DeviceKind::Dma(DmaCfg {}),
                vec![PortWindow::new(0x00, 0x0F), PortWindow::new(0x81, 0x8F)],
            ),
        ];

        let memory = MemoryConfig {
            address_space_bytes: SPACE,
            ram: vec![
                RamRegion::new(0, TEXT_BASE),
                RamRegion::new(TEXT_BASE + TEXT_LEN, ROM_BASE - (TEXT_BASE + TEXT_LEN)),
            ],
            mmio: vec![MmioWindow::new(TEXT_BASE, TEXT_LEN, DeviceId::Vga)],
        };

        let images = vec![LoadImage {
            base: ROM_BASE,
            source: ImageSource::Inline(vec![0u8; ROM_LEN as usize]),
            kind: ImageKind::Rom,
            provides: Vec::new(),
        }];

        let drives = vec![DriveConfig {
            media: None,
            kind: DriveKind::Floppy {
                geometry: FloppyGeometry::HD_1_44,
            },
            image: ImageSource::Blank {
                size: FloppyGeometry::HD_1_44.image_bytes(),
            },
            controller: DeviceId::Fdc,
            unit: 0,
            write: WriteMode::Volatile,
            expected_sha256: None,
        }];

        MachineConfig {
            system_bios_image: None,
            board: BoardModel::Bare,
            display_switch: crate::DisplaySwitch::Auto,
            cpu: CpuConfig {
                fpu: crate::FpuChip::None,
                chip: CpuChip::I80286,
                reset: ResetSpec::reset_vector(),
                segments: SegmentSeed::flat(0xF000),
            },
            clock: ClockConfig::default(),
            memory,
            devices,
            drives,
            images,
        }
    }

    #[test]
    fn mda_decode_conflicts_and_missing_card_are_rejected() {
        let mut config = crate::bios(vec![0; 0x8000], None);
        // Overlap diagnostics name owners in configuration order. Keep this
        // invalid fixture VGA-first independently of the preset's device order.
        let [vga, mda] = [DeviceId::Vga, DeviceId::Mda]
            .map(|id| config.devices.iter().position(|d| d.id == id).unwrap());
        if vga > mda {
            config.devices.swap(vga, mda);
        }
        let [vga, mda] = [DeviceId::Vga, DeviceId::Mda].map(|id| {
            config
                .memory
                .mmio
                .iter()
                .position(|w| w.device == id)
                .unwrap()
        });
        if vga > mda {
            config.memory.mmio.swap(vga, mda);
        }
        let mut ports = config.clone();
        find_device(&mut ports, DeviceId::Vga)
            .ports
            .push(PortWindow::single(0x3B4));
        assert_eq!(
            ports.validate(),
            Err(ConfigError::PortOverlap {
                a: DeviceId::Vga,
                b: DeviceId::Mda,
                port: 0x3B4,
            })
        );
        let mut mmio = config.clone();
        mmio.memory
            .mmio
            .iter_mut()
            .find(|w| w.device == DeviceId::Vga)
            .unwrap()
            .base = 0xB7000;
        assert_eq!(
            mmio.validate(),
            Err(ConfigError::MmioOverlap {
                a: DeviceId::Vga,
                b: DeviceId::Mda,
                addr: 0xB7000,
            })
        );
        let mut ram = config.clone();
        ram.memory.ram.push(RamRegion::new(0xB0000, 0x8000));
        assert_eq!(
            ram.validate(),
            Err(ConfigError::MmioOverlapsRam {
                device: DeviceId::Mda,
                addr: 0xB0000,
            })
        );
        let mut missing = config;
        missing.devices.retain(|d| d.id != DeviceId::Mda);
        assert_eq!(
            missing.validate(),
            Err(ConfigError::MmioDeviceMissing(DeviceId::Mda))
        );
    }

    #[test]
    fn uart_window_must_fit_port_space() {
        let mut config = valid_config();
        for base in 0xFFF9..=0xFFFF {
            find_device(&mut config, DeviceId::Uart).kind =
                DeviceKind::Uart(UartCfg { base, mouse: false });
            assert_eq!(
                config.validate(),
                Err(ConfigError::UartPortWindowOutOfSpace {
                    device: DeviceId::Uart,
                    base
                })
            );
        }
        find_device(&mut config, DeviceId::Uart).kind = DeviceKind::Uart(UartCfg {
            base: 0xFFF8,
            mouse: false,
        });
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn valid_config_passes() {
        assert_eq!(valid_config().validate(), Ok(()));
    }

    #[test]
    fn zero_cpu_rate_is_rejected() {
        for (numerator, denominator) in [(0, 1), (1, 0), (0, 0)] {
            let mut config = valid_config();
            config.clock.retired_per_pit_clock = numerator;
            config.clock.retired_per_pit_clock_denominator = denominator;
            assert_eq!(
                config.validate(),
                Err(ConfigError::InvalidCpuRate {
                    numerator,
                    denominator,
                })
            );
            assert_eq!(
                config.validate().expect_err("invalid CPU rate").to_string(),
                format!("cpu rate must have a positive numerator and denominator (got {numerator}/{denominator})")
            );
        }
    }

    #[test]
    fn positive_cpu_rates_are_valid() {
        for (numerator, denominator) in [(1, 1), (2, 1), (1, 3), (3, 2), (u64::MAX, u64::MAX)] {
            let mut config = valid_config();
            config.clock.retired_per_pit_clock = numerator;
            config.clock.retired_per_pit_clock_denominator = denominator;
            assert_eq!(config.validate(), Ok(()));
        }
    }

    #[test]
    fn the_four_standard_floppy_geometries_have_the_expected_sizes() {
        assert_eq!(FloppyGeometry::HD_1_44.image_bytes(), 1_474_560);
        assert_eq!(FloppyGeometry::HD_1_2.image_bytes(), 1_228_800);
        assert_eq!(FloppyGeometry::DD_720K.image_bytes(), 737_280);
        assert_eq!(FloppyGeometry::DD_360K.image_bytes(), 368_640);
    }

    #[test]
    fn duplicate_device_id_is_rejected() {
        let mut config = valid_config();
        config.devices.push(dev(
            DeviceId::Uart,
            DeviceKind::Uart(UartCfg {
                base: 0x2F8,
                mouse: false,
            }),
            vec![PortWindow::new(0x2F8, 0x2FF)],
        ));
        assert_eq!(
            config.validate(),
            Err(ConfigError::DuplicateDeviceId(DeviceId::Uart))
        );
    }

    #[test]
    fn port_overlap_is_rejected() {
        let mut config = valid_config();
        // Give the UART a stray window over the PIT's 0x40 base.
        find_device(&mut config, DeviceId::Uart)
            .ports
            .push(PortWindow::single(0x40));
        assert_eq!(
            config.validate(),
            Err(ConfigError::PortOverlap {
                a: DeviceId::Uart,
                b: DeviceId::Pit,
                port: 0x40,
            })
        );
    }

    #[test]
    fn an_explicitly_allowed_port_overlap_passes() {
        let mut config = valid_config();
        find_device(&mut config, DeviceId::Uart)
            .ports
            .push(PortWindow::overlapping(0x40, 0x40));
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn a_device_level_overlap_allowance_passes() {
        let mut config = valid_config();
        {
            let uart = find_device(&mut config, DeviceId::Uart);
            uart.ports.push(PortWindow::single(0x40));
            uart.allow_port_overlap = true;
        }
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn flagged_primary_control_inside_unsplit_fdc_is_rejected() {
        for flag_on_fdc in [false, true] {
            for device_flag in [false, true] {
                let mut config = valid_config();
                let post = find_device(&mut config, DeviceId::Post);
                post.kind = DeviceKind::Post(PostPortCfg { port: 0x3F6 });
                post.ports = vec![PortWindow::single(0x3F6)];
                let flagged = find_device(
                    &mut config,
                    if flag_on_fdc {
                        DeviceId::Fdc
                    } else {
                        DeviceId::Post
                    },
                );
                if device_flag {
                    flagged.allow_port_overlap = true;
                } else {
                    flagged.ports[0].allow_overlap = true;
                }
                let error = config.validate().unwrap_err();
                assert_eq!(
                    error,
                    ConfigError::PortOverlapDecodeOrder {
                        a: DeviceId::Post,
                        b: DeviceId::Fdc,
                        a_window: find_device(&mut config, DeviceId::Post).ports[0],
                        b_window: find_device(&mut config, DeviceId::Fdc).ports[0],
                        port: 0x3F6,
                    }
                );
                assert_eq!(error.to_string(), "port overlap at 0x03f6 between Post [0x03f6..=0x03f6] and Fdc [0x03f0..=0x03f7]: overlapping windows must share a start port and differ in width; split the wider window or move one");
            }
        }
    }

    #[test]
    fn flagged_partial_port_overlap_is_rejected() {
        for reverse_devices in [false, true] {
            let mut config = valid_config();
            let post = find_device(&mut config, DeviceId::Post);
            post.kind = DeviceKind::Post(PostPortCfg { port: 0x3EE });
            post.ports = vec![PortWindow::overlapping(0x3EE, 0x3F1)];
            if reverse_devices {
                config.devices.reverse();
            }
            let (a, b, a_window, b_window) = if reverse_devices {
                (
                    DeviceId::Fdc,
                    DeviceId::Post,
                    PortWindow::new(0x3F0, 0x3F7),
                    PortWindow::overlapping(0x3EE, 0x3F1),
                )
            } else {
                (
                    DeviceId::Post,
                    DeviceId::Fdc,
                    PortWindow::overlapping(0x3EE, 0x3F1),
                    PortWindow::new(0x3F0, 0x3F7),
                )
            };
            assert_eq!(
                config.validate(),
                Err(ConfigError::PortOverlapDecodeOrder {
                    a,
                    b,
                    a_window,
                    b_window,
                    port: 0x3F0,
                })
            );
        }
    }

    #[test]
    fn flagged_identical_port_windows_are_rejected() {
        let mut config = valid_config();
        find_device(&mut config, DeviceId::Post).ports =
            vec![PortWindow::overlapping(0x3F0, 0x3F7)];
        let error = config.validate().unwrap_err();
        assert_eq!(
            error,
            ConfigError::PortOverlapDecodeOrder {
                a: DeviceId::Post,
                b: DeviceId::Fdc,
                a_window: PortWindow::overlapping(0x3F0, 0x3F7),
                b_window: PortWindow::new(0x3F0, 0x3F7),
                port: 0x3F0,
            }
        );
        assert_eq!(error.to_string(), "port overlap at 0x03f0 between Post [0x03f0..=0x03f7] and Fdc [0x03f0..=0x03f7]: overlapping windows must share a start port and differ in width; split the wider window or move one");
    }

    #[test]
    fn flagged_equal_width_partial_overlap_message_offers_relocation() {
        let mut config = valid_config();
        let post = find_device(&mut config, DeviceId::Post);
        post.kind = DeviceKind::Post(PostPortCfg { port: 0x3EE });
        post.ports = vec![PortWindow::overlapping(0x3EE, 0x3F5)];
        let error = config.validate().unwrap_err();
        assert!(matches!(error, ConfigError::PortOverlapDecodeOrder { .. }));
        assert_eq!(error.to_string(), "port overlap at 0x03f0 between Post [0x03ee..=0x03f5] and Fdc [0x03f0..=0x03f7]: overlapping windows must share a start port and differ in width; split the wider window or move one");
    }

    #[test]
    fn mmio_window_to_absent_device_is_rejected() {
        let mut config = valid_config();
        config.devices.retain(|d| d.id != DeviceId::Vga);
        assert_eq!(
            config.validate(),
            Err(ConfigError::MmioDeviceMissing(DeviceId::Vga))
        );
    }

    #[test]
    fn overlapping_mmio_windows_are_rejected() {
        let mut config = valid_config();
        config
            .memory
            .mmio
            .push(MmioWindow::new(TEXT_BASE, TEXT_LEN, DeviceId::Vga));
        assert_eq!(
            config.validate(),
            Err(ConfigError::MmioOverlap {
                a: DeviceId::Vga,
                b: DeviceId::Vga,
                addr: TEXT_BASE,
            })
        );
    }

    #[test]
    fn mmio_over_ram_is_rejected() {
        let mut config = valid_config();
        // Extend the low RAM region over the trapped text buffer.
        config.memory.ram[0] = RamRegion::new(0, TEXT_BASE + TEXT_LEN);
        assert_eq!(
            config.validate(),
            Err(ConfigError::MmioOverlapsRam {
                device: DeviceId::Vga,
                addr: TEXT_BASE,
            })
        );
    }

    #[test]
    fn a_region_outside_the_address_space_is_rejected() {
        let mut config = valid_config();
        config.memory.ram.push(RamRegion::new(0xF_FF00, 0x1000));
        assert_eq!(
            config.validate(),
            Err(ConfigError::RegionOutOfSpace {
                base: 0xF_FF00,
                end: 0x10_0F00,
                space: SPACE,
            })
        );
    }

    #[test]
    fn an_image_running_past_the_address_space_is_rejected() {
        let mut config = valid_config();
        // A ROM one byte too long overruns the top of the 1 MiB space.
        config.images[0].source = ImageSource::Inline(vec![0u8; ROM_LEN as usize + 1]);
        assert_eq!(
            config.validate(),
            Err(ConfigError::RegionOutOfSpace {
                base: ROM_BASE,
                end: SPACE + 1,
                space: SPACE,
            })
        );
    }

    #[test]
    fn an_irq_line_conflict_is_rejected() {
        let mut config = valid_config();
        // Wire the VGA onto IRQ0, already the PIT's line.
        find_device(&mut config, DeviceId::Vga).irq = Some(0);
        let err = config.validate().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::IrqConflict {
                line: 0,
                a: DeviceId::Pit,
                b: DeviceId::Vga,
            }
        ));
    }

    #[test]
    fn sb16_opl_ports_are_claimed_and_conflicts_are_rejected() {
        for port in [
            0x220, 0x221, 0x222, 0x223, 0x228, 0x229, 0x388, 0x389, 0x38a, 0x38b,
        ] {
            let mut config = valid_config();
            let cfg = crate::Sb16Cfg {
                base: 0x220,
                dma16: 5,
            };
            assert!(cfg
                .port_windows()
                .iter()
                .any(|w| w.start == port && w.end == port));
            config.devices.push(
                crate::DeviceConfig::new(DeviceId::Sb16, DeviceKind::Sb16(cfg), cfg.port_windows())
                    .with_irq(5)
                    .with_dma(crate::DmaSpec {
                        controller: DeviceId::Dma,
                        channel: 1,
                    }),
            );
            config.devices.push(dev(
                DeviceId::DmaWord,
                DeviceKind::DmaWord(DmaCfg {}),
                Vec::new(),
            ));
            assert_eq!(config.validate(), Ok(()));
            find_device(&mut config, DeviceId::Post).ports = vec![PortWindow::single(port)];
            assert!(
                matches!(config.validate(), Err(ConfigError::PortOverlap { .. })),
                "port {port:x}"
            );
        }
    }

    #[test]
    fn sb16_word_dma_conflicts_are_detected_in_either_device_order() {
        for reversed in [false, true] {
            let mut config = valid_config();
            let cfg = crate::Sb16Cfg {
                base: 0x220,
                dma16: 5,
            };
            config.devices.push(
                crate::DeviceConfig::new(DeviceId::Sb16, DeviceKind::Sb16(cfg), cfg.port_windows())
                    .with_irq(5)
                    .with_dma(crate::DmaSpec {
                        controller: DeviceId::Dma,
                        channel: 1,
                    }),
            );
            config.devices.push(dev(
                DeviceId::DmaWord,
                DeviceKind::DmaWord(DmaCfg {}),
                Vec::new(),
            ));
            // The spare POST identity stands for a mock DMA requester in this
            // wiring-only fixture; the floppy retains its real byte-DMA format.
            find_device(&mut config, DeviceId::Post).dma = Some(crate::DmaSpec {
                controller: DeviceId::DmaWord,
                channel: 6,
            });
            assert_eq!(config.validate(), Ok(()));
            find_device(&mut config, DeviceId::Post)
                .dma
                .as_mut()
                .unwrap()
                .channel = 5;
            if reversed {
                config.devices.reverse();
            }
            assert!(matches!(
                config.validate(),
                Err(ConfigError::DmaConflict {
                    controller: DeviceId::DmaWord,
                    channel: 5,
                    ..
                })
            ));
        }
    }

    #[test]
    fn word_dma_specs_reject_local_and_cascade_channel_numbers() {
        for channel in [0, 1, 2, 3, 4, 8] {
            let mut config = valid_config();
            config.devices.push(dev(
                DeviceId::DmaWord,
                DeviceKind::DmaWord(DmaCfg {}),
                Vec::new(),
            ));
            assert_eq!(config.validate(), Ok(()));
            find_device(&mut config, DeviceId::Uart).dma = Some(crate::DmaSpec {
                controller: DeviceId::DmaWord,
                channel,
            });
            assert_eq!(
                config.validate(),
                Err(ConfigError::InvalidDmaChannel {
                    device: DeviceId::Uart,
                    controller: DeviceId::DmaWord,
                    channel,
                }),
                "word DMA system channel {channel}"
            );
        }
    }

    #[test]
    fn dma_wiring_to_an_absent_controller_is_rejected() {
        let mut config = valid_config();
        config.devices.retain(|d| d.id != DeviceId::Dma);
        assert_eq!(
            config.validate(),
            Err(ConfigError::DmaControllerMissing {
                requester: DeviceId::Fdc,
                controller: DeviceId::Dma,
            })
        );
    }

    #[test]
    fn a_drive_on_a_nonexistent_unit_is_rejected() {
        let mut config = valid_config();
        config.drives[0].unit = 5; // the FDC offers only slots 0 and 1
        assert_eq!(
            config.validate(),
            Err(ConfigError::DriveUnitUnavailable {
                drive_index: 0,
                controller: DeviceId::Fdc,
                unit: 5,
                available: 2,
            })
        );
    }

    #[test]
    fn a_drive_with_no_controller_present_is_rejected() {
        let mut config = valid_config();
        // Drop the FDC and its DMA wiring source; the floppy now has no
        // controller at all (zero available slots).
        config
            .devices
            .retain(|d| d.id != DeviceId::Fdc && d.id != DeviceId::Dma);
        assert_eq!(
            config.validate(),
            Err(ConfigError::DriveControllerMissing {
                drive_index: 0,
                controller: DeviceId::Fdc,
            })
        );
    }

    #[test]
    fn a_floppy_image_of_the_wrong_size_is_rejected() {
        let mut config = valid_config();
        config.drives[0].image = ImageSource::Blank { size: 1000 };
        assert_eq!(
            config.validate(),
            Err(ConfigError::ImageGeometryMismatch {
                drive_index: 0,
                expected: 1_474_560,
                actual: 1000,
                drive_type: Some("1.44M"),
            })
        );
    }

    #[test]
    fn file_geometry_validation_is_deferred_until_assembly() {
        let mut config = valid_config();
        // A file's size is unknown without I/O, so geometry cannot be checked
        // here — validation must not fail on it.
        config.drives[0].image = ImageSource::File("disk.img".into());
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn segment_seed_converts_to_the_cpu_core_type() {
        let seed = SegmentSeed::flat(0x1234);
        let real: cpu::x86jit_backend::RealSegments = seed.into();
        assert_eq!(real.ds, 0x1234);
        assert_eq!(real.ss, 0x1234);
        assert_eq!(real.sp, seed.sp);
    }

    #[test]
    fn zero_size_mmio_is_rejected() {
        let mut config = valid_config();
        config.memory.mmio[0] = MmioWindow::new(0, 0, DeviceId::Vga);
        assert_eq!(
            config.validate(),
            Err(ConfigError::EmptyMmioWindow {
                device: DeviceId::Vga,
                base: 0
            })
        );
    }

    #[test]
    fn irq_outside_pic_inputs_is_rejected() {
        for line in [16, u8::MAX] {
            let mut config = valid_config();
            find_device(&mut config, DeviceId::Pit).irq = Some(line);
            assert_eq!(
                config.validate(),
                Err(ConfigError::InvalidIrqLine {
                    device: DeviceId::Pit,
                    line
                })
            );
        }
    }

    #[test]
    fn duplicate_pic_faces_are_rejected() {
        for cascade in [true, false] {
            let mut config = valid_config();
            for device in &mut config.devices {
                if let DeviceKind::Pic(pic) = &mut device.kind {
                    pic.cascade = cascade;
                }
            }
            assert_eq!(
                config.validate(),
                Err(ConfigError::DuplicatePicFace {
                    a: DeviceId::MasterPic,
                    b: DeviceId::SlavePic,
                    cascade
                })
            );
        }
    }

    #[test]
    fn unsupported_floppy_sector_size_is_rejected() {
        let mut config = valid_config();
        let geometry = FloppyGeometry {
            bytes_per_sector: 256,
            ..FloppyGeometry::HD_1_44
        };
        config.drives[0].kind = DriveKind::Floppy { geometry };
        config.drives[0].image = ImageSource::Blank {
            size: geometry.image_bytes(),
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::UnsupportedFloppySectorSize {
                drive_index: 0,
                bytes_per_sector: 256
            })
        );
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "drive 0 uses 256-byte sectors; the floppy model supports 512-byte sectors only"
        );
    }

    #[test]
    fn image_over_mmio_is_rejected() {
        for kind in [ImageKind::Rom, ImageKind::RamLoad] {
            let mut config = valid_config();
            config.images[0] = LoadImage {
                base: TEXT_BASE,
                source: ImageSource::Blank { size: 1 },
                kind,
                provides: Vec::new(),
            };
            assert_eq!(
                config.validate(),
                Err(ConfigError::ImageOverlapsMmio {
                    image_index: 0,
                    device: DeviceId::Vga,
                    addr: TEXT_BASE
                })
            );
        }
    }

    #[test]
    fn overlapping_images_are_rejected() {
        let mut config = valid_config();
        config.images.push(LoadImage {
            base: ROM_BASE + 1,
            source: ImageSource::Blank { size: 1 },
            kind: ImageKind::Rom,
            provides: Vec::new(),
        });
        assert_eq!(
            config.validate(),
            Err(ConfigError::ImageOverlap {
                a: 0,
                b: 1,
                addr: ROM_BASE + 1
            })
        );
    }

    #[test]
    fn ram_load_outside_ram_is_rejected() {
        let mut config = valid_config();
        config.images[0].kind = ImageKind::RamLoad;
        assert_eq!(
            config.validate(),
            Err(ConfigError::ImageOutsideRam {
                image_index: 0,
                addr: ROM_BASE
            })
        );
    }

    #[test]
    fn ram_load_crossing_a_ram_hole_is_rejected() {
        let mut config = valid_config();
        config.memory.ram = vec![RamRegion::new(0, 16), RamRegion::new(32, 16)];
        config.images[0] = LoadImage {
            base: 8,
            source: ImageSource::Blank { size: 32 },
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::ImageOutsideRam {
                image_index: 0,
                addr: 16
            })
        );
    }

    #[test]
    fn ram_load_can_span_adjacent_regions_in_any_order() {
        let mut config = valid_config();
        config.memory.ram = vec![RamRegion::new(16, 16), RamRegion::new(0, 16)];
        config.images[0] = LoadImage {
            base: 8,
            source: ImageSource::Blank { size: 24 },
            kind: ImageKind::RamLoad,
            provides: Vec::new(),
        };
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn adjacent_images_and_empty_images_do_not_overlap() {
        let mut config = valid_config();
        config.images = vec![
            LoadImage {
                base: 0,
                source: ImageSource::Blank { size: 16 },
                kind: ImageKind::RamLoad,
                provides: Vec::new(),
            },
            LoadImage {
                base: 16,
                source: ImageSource::Blank { size: 16 },
                kind: ImageKind::RamLoad,
                provides: Vec::new(),
            },
            LoadImage {
                base: 8,
                source: ImageSource::Blank { size: 0 },
                kind: ImageKind::RamLoad,
                provides: Vec::new(),
            },
        ];
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn highest_pic_input_is_valid() {
        let mut config = valid_config();
        find_device(&mut config, DeviceId::Pit).irq = Some(15);
        assert_eq!(config.validate(), Ok(()));
    }
}
