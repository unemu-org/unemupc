// SPDX-License-Identifier: AGPL-3.0-or-later
//! Lower schema 1 into the assembly-owned PC/AT configuration.
//!
//! See `backlog/docs/design/machine-files.md` § Schema and § Validation and
//! presets. Motherboard wiring comes from assembly; card defaults describe the
//! IBM Personal Computer AT Technical Reference, Section 1, I/O Address Map.
//! Hardware consistency is checked only by `MachineConfig::validate`.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use assembly::{
    BoardModel, ConfigError, CpuChip, CpuConfig, DeviceConfig, DeviceId, DeviceKind, DmaSpec,
    DriveConfig, DriveKind, ImageKind, ImageSource, LoadImage, MachineConfig, MemoryConfig,
    MmioWindow, NvramSource, PortWindow, PostPortCfg, ResetSpec, RtcStart, SegmentSeed, UartCfg,
};
use sha2::{Digest, Sha256};

use crate::{schema::v1, MachineFile, Origin, Origins};

const MIB: u64 = 0x100000;

pub(crate) fn board_model(model: v1::BoardModel) -> BoardModel {
    match model {
        v1::BoardModel::Ibm5170 => BoardModel::Ibm5170,
        v1::BoardModel::Generic386Isa => BoardModel::Generic386Isa,
        v1::BoardModel::Generic486Isa => BoardModel::Generic486Isa,
    }
}

/// Image I/O seam: everything the lowering asks of the host goes through it.
///
/// Lowering verifies each referenced image through this seam, including
/// synthetic bytes supplied by tests. Volatile/readonly bytes are retained inline;
/// catalogues and journal bases retain paths and the verified digest for assembly's mount check.
/// Journal reads require a reader that can verify a base, so the journal remains bound to a
/// reproducible immutable base. Paths resolve against the defining layer's directory.
///
/// The lowering itself opens no file and asks the host nothing: a reader
/// over the host file system is `machine-file`'s `FileImageReader`; a reader that serves
/// every answer from memory lowers the same document without one.
pub trait ImageReader {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>>;

    /// Validate logical keys in the namespace used by this reader. A file
    /// reader adds its host path checks here.
    fn validate_machine(&self, config: &MachineConfig) -> Result<(), ConfigError> {
        config.validate()
    }

    /// Verify a disk without returning a whole-image buffer, and answer its
    /// length and SHA-256. Synthetic readers refuse persistent media; a
    /// frontend's reader keeps whatever verified handle it opened itself.
    fn verify_journal_base(&mut self, _path: &Path) -> io::Result<JournalBase> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "journal mode requires a file-backed image reader",
        ))
    }

    /// Read a small CMOS journal base. Disk callers use `verify_journal_base`.
    /// Synthetic readers refuse journals because their bytes need not exist on disk.
    fn read_journal_base(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "journal mode requires a file-backed image reader",
        ))
    }

    /// The length of an image without reading it. Metadata checks and
    /// descriptions ask to place unread ROMs; a reader that cannot tell
    /// leaves that size pending.
    fn image_length(&self, _path: &Path) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this image reader cannot size an image without reading it",
        ))
    }

    /// Whether an image path is a name the journal reserves for its
    /// retention temporaries, which no machine may mount. The default looks
    /// at the name alone; a host reader also resolves links.
    fn is_reserved_retention_name(&self, path: &Path) -> bool {
        has_retention_marker(path)
    }

    /// Check the lowered machine's journal paths against its inputs: the
    /// image paths read and every inheritance layer. The paths are host
    /// paths only for a host reader, which checks that no journal is a
    /// directory, aliases an input or claims an input as its temporary
    /// file. Any other reader has nothing on the host to check.
    ///
    /// The default accepts every journal, which is safe only for a reader
    /// that cannot produce one. A reader that overrides `verify_journal_base`
    /// or `read_journal_base` over host files must override this too, or a
    /// journal could overwrite one of the machine's own inputs.
    fn validate_journal_paths(
        &self,
        _config: &MachineConfig,
        _origins: &Origins,
        _images: &[PathBuf],
    ) -> Result<(), ConfigError> {
        Ok(())
    }
}

/// The length and SHA-256 of a verified journal base: all the lowering
/// needs of it. The reader that verified the base keeps the base itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalBase {
    pub length: u64,
    pub sha256: [u8; 32],
}

/// Whether a file name carries the journal's reserved retention marker,
/// `.retain-`. The journal owns this rule; the copy keeps the lowering free
/// of the journal, and a test of `machine-file`, which has the journal,
/// holds the two equal (through the `test-support` feature).
pub fn has_retention_marker(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name.as_encoded_bytes()
            .windows(8)
            .any(|part| part == b".retain-")
    })
}

/// A completeness, image-integrity, unsupported-feature or assembly diagnostic.
#[derive(Debug, thiserror::Error)]
#[error("{origin}: {field}: {message}")]
pub struct LowerError {
    pub origin: Origin,
    pub field: String,
    pub message: String,
    /// The defining source of the checked pin, when its digest mismatched.
    // Boxing keeps LowerError below Clippy's large-error limit.
    pub pin_origin: Option<Box<Origin>>,
    /// The original assembly error, when hardware validation failed.
    #[source]
    pub validation: Option<Box<ConfigError>>,
}

#[derive(Clone)]
struct DriveOrigin {
    image_name: Option<String>,
    field: String,
    label: String,
}

struct Context<'a, R> {
    origins: &'a Origins,
    reader: &'a mut R,
    labels: Vec<Option<String>>,
    devices: Vec<(DeviceId, String)>,
    irq_fields: Vec<(DeviceId, u8, String)>,
    drives: Vec<DriveOrigin>,
    images: Vec<String>,
    image_paths: Vec<PathBuf>,
    options: assembly::AssemblyOptions,
    bios_parts: bool,
    metadata: bool,
    rom_sizes: Option<Vec<Option<u64>>>,
}

impl<R: ImageReader> Context<'_, R> {
    fn label(&self, field: &str) -> String {
        let field = self
            .drives
            .iter()
            .find_map(|drive| {
                field
                    .strip_prefix(&drive.field)
                    .filter(|tail| tail.is_empty() || tail.starts_with('.'))
                    .map(|tail| format!("{}{tail}", drive.label))
            })
            .unwrap_or_else(|| field.into());
        for (index, label) in self.labels.iter().enumerate() {
            let prefix = format!("card[{index}]");
            if let Some(tail) = field.strip_prefix(&prefix) {
                return match label {
                    Some(label) => format!("card[{label}]{tail}"),
                    None => format!("card #{index}{tail}"),
                };
            }
        }
        field
    }

    fn error(&self, field: &str, message: impl Into<String>) -> LowerError {
        LowerError {
            origin: self.origins.get(field).clone(),
            field: self.label(field),
            message: message.into(),
            pin_origin: None,
            validation: None,
        }
    }

    fn pin_error(&self, field: &str, pin_field: &str, message: impl Into<String>) -> LowerError {
        // An inherited pin can come from a different layer than its image path.
        let pin_origin = self.origins.get(pin_field).clone();
        let mut error = self.error(field, message);
        error
            .message
            .push_str(&format!("; pin set at {pin_origin}"));
        error.pin_origin = Some(Box::new(pin_origin));
        error
    }

    fn required<T>(&self, field: &str, value: Option<T>) -> Result<T, LowerError> {
        value.ok_or_else(|| self.error(field, "required field is missing"))
    }

    fn journal_image(
        &mut self,
        field: &str,
        image: Option<&str>,
        pin: Option<&v1::Sha256>,
        pin_field: &str,
    ) -> Result<(u64, [u8; 32]), LowerError> {
        let image = self.required(field, image)?;
        let origin = self.origins.get(field);
        let display_path = origin.file.parent().unwrap_or(Path::new(".")).join(image);
        let path = self.origins.image_path(field, image);
        let description = if path == display_path {
            display_path.display().to_string()
        } else {
            format!(
                "{} (resolved to {})",
                display_path.display(),
                path.display()
            )
        };
        if self.reader.is_reserved_retention_name(&path) {
            return Err(self.error(
                field,
                format!("image {description} uses a reserved retention-temporary name"),
            ));
        }
        let base = self.reader.verify_journal_base(&path).map_err(|error| {
            self.error(field, format!("cannot read image {description}: {error}"))
        })?;
        let digest = base.sha256;
        let actual = format!("{:x}", sha2::digest::Output::<Sha256>::from(digest));
        if let Some(pin) = pin {
            if actual != pin.as_str() {
                return Err(self.pin_error(
                    field,
                    pin_field,
                    format!(
                        "sha256 mismatch for {description}: expected {}, actual {actual}",
                        pin.as_str()
                    ),
                ));
            }
        }
        self.image_paths.push(path);
        Ok((base.length, digest))
    }

    fn image(
        &mut self,
        field: &str,
        image: Option<&str>,
        pin: Option<&v1::Sha256>,
        pin_field: &str,
        journal: bool,
    ) -> Result<(Vec<u8>, [u8; 32]), LowerError> {
        let image = self.required(field, image)?;
        let origin = self.origins.get(field);
        let display_path = origin.file.parent().unwrap_or(Path::new(".")).join(image);
        let path = self.origins.image_path(field, image);
        let description = if path == display_path {
            display_path.display().to_string()
        } else {
            format!(
                "{} (resolved to {})",
                display_path.display(),
                path.display()
            )
        };
        if self.reader.is_reserved_retention_name(&path) {
            return Err(self.error(
                field,
                format!("image {description} uses a reserved retention-temporary name"),
            ));
        }
        let bytes = if journal {
            self.reader.read_journal_base(&path)
        } else {
            self.reader.read(&path)
        }
        .map_err(|error| self.error(field, format!("cannot read image {description}: {error}")))?;
        let digest = Sha256::digest(&bytes);
        let actual = format!("{digest:x}");
        if let Some(pin) = pin {
            if actual != pin.as_str() {
                return Err(self.pin_error(
                    field,
                    pin_field,
                    format!(
                        "sha256 mismatch for {description}: expected {}, actual {actual}",
                        pin.as_str()
                    ),
                ));
            }
        }
        self.image_paths.push(path);
        Ok((bytes, digest.into()))
    }

    fn write(
        &self,
        field: &str,
        mode: Option<v1::WriteMode>,
        journal: bool,
        retention: bool,
    ) -> Result<assembly::WriteMode, LowerError> {
        match mode.unwrap_or(v1::WriteMode::Volatile) {
            v1::WriteMode::Journal => Err(self.error(field,
                "write = \"journal\" is not implemented yet; journal attachment and persistent-media integration are later work")),
            _ if journal || retention => Err(self.error(field,
                "journal and keep_generations are only accepted with write = \"journal\"")),
            v1::WriteMode::ReadOnly => Ok(assembly::WriteMode::ReadOnly),
            v1::WriteMode::Volatile => Ok(assembly::WriteMode::Volatile),
        }
    }

    fn rom(
        &mut self,
        field: &str,
        rom: &v1::Rom,
        bios: bool,
        catalogue: &[assembly::FirmwareEntry],
    ) -> Result<LoadImage, LowerError> {
        if bios {
            self.bios_parts = rom.parts.is_some();
        }
        if bios && rom.offset.is_some() {
            return Err(self.error(
                &format!("{field}.offset"),
                "offset is only supported by option ROM images",
            ));
        }
        if rom.image.is_some() && rom.parts.is_some() {
            return Err(self.error(field, "image and parts are mutually exclusive"));
        }
        let bytes = if self.metadata {
            None
        } else {
            Some(if let Some(parts) = &rom.parts {
                if !bios {
                    return Err(self.error(
                        &format!("{field}.parts"),
                        "option ROMs cannot use parts; only firmware.bios supports parts",
                    ));
                }
                let mut loaded = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    let (bytes, _) = self.image(
                        &format!("{field}.parts[{i}].image"),
                        Some(&part.image),
                        part.sha256.as_ref(),
                        &format!("{field}.parts[{i}].sha256"),
                        false,
                    )?;
                    loaded.push((
                        match part.lane {
                            v1::Lane::Even => assembly::Lane::Even,
                            v1::Lane::Odd => assembly::Lane::Odd,
                        },
                        part.offset,
                        bytes,
                    ));
                }
                let bytes = assembly::interleave_parts(&loaded)
                    .map_err(|e| self.error(&format!("{field}.parts"), e.to_string()))?;
                assembly::check_firmware_parts(&loaded, &bytes, catalogue, self.options)
                    .map_err(|e| self.error(&format!("{field}.parts"), e.to_string()))?;
                if let Some(pin) = &rom.sha256 {
                    let actual = format!("{:x}", Sha256::digest(&bytes));
                    if actual != pin.as_str() {
                        return Err(self.pin_error(
                            &format!("{field}.parts"),
                            &format!("{field}.sha256"),
                            format!(
                                "sha256 mismatch for assembled BIOS: expected {}, actual {actual}",
                                pin.as_str()
                            ),
                        ));
                    }
                }
                bytes
            } else {
                self.image(
                    &format!("{field}.image"),
                    rom.image.as_deref(),
                    rom.sha256.as_ref(),
                    &format!("{field}.sha256"),
                    false,
                )?
                .0
            })
        };
        let identified = if let Some(bytes) = &bytes {
            catalogue.iter().find(|entry| entry.identifies(bytes))
        } else {
            rom.sha256
                .as_ref()
                .and_then(|pin| catalogue.iter().find(|entry| entry.sha256 == pin.as_str()))
        };
        let size = if let Some(bytes) = &bytes {
            Some(bytes.len() as u64)
        } else if let Some(entry) = identified.filter(|_| self.rom_sizes.is_none()) {
            Some(entry.size)
        } else if let Some(parts) = &rom.parts {
            parts
                .iter()
                .enumerate()
                .map(|(index, part)| {
                    self.reader
                        .image_length(
                            &self
                                .origins
                                .image_path(&format!("{field}.parts[{index}].image"), &part.image),
                        )
                        .ok()
                        .map(|length| part.offset.saturating_add(length.saturating_mul(2)))
                })
                .collect::<Option<Vec<_>>>()
                .and_then(|sizes| sizes.into_iter().max())
        } else {
            rom.image.as_ref().and_then(|image| {
                self.reader
                    .image_length(&self.origins.image_path(&format!("{field}.image"), image))
                    .ok()
            })
        };

        if let Some(entry) =
            identified.filter(|entry| bios && entry.kind == assembly::FirmwareKind::VideoBios)
        {
            return Err(self.error(
                field,
                ConfigError::VideoBiosAsSystemBios { firmware: entry.id }.to_string(),
            ));
        }
        let base = match rom.base {
            Some(base) => u64::from(base),
            None if bios => MIB
                .checked_sub(size.unwrap_or(0))
                .ok_or_else(|| self.error(field, "BIOS exceeds the 1 MiB reset address space"))?,
            None => return Err(self.error(&format!("{field}.base"), "required field is missing")),
        };
        if bios && size.is_some_and(|size| base.checked_add(size) != Some(MIB)) {
            return Err(self.validation_error(ConfigError::BiosPlacement {
                base,
                size: size.unwrap_or(0),
            }));
        }
        self.images.push(field.into());
        let mut provides: Vec<_> = rom
            .provides
            .iter()
            .flatten()
            .map(|service| match service {
                v1::FirmwareService::Floppy => assembly::FirmwareService::Floppy,
                v1::FirmwareService::FixedDisk => assembly::FirmwareService::FixedDisk,
            })
            .collect();
        if provides.is_empty() {
            if let Some(entry) = identified
                .filter(|entry| (entry.kind == assembly::FirmwareKind::SystemBios) == bios)
            {
                provides = entry.provides.to_vec();
            }
        }
        let adapter =
            identified.filter(|entry| !bios && entry.kind != assembly::FirmwareKind::SystemBios);
        let offset = rom
            .offset
            .unwrap_or_else(|| adapter.map_or(0, |entry| entry.rom_offset));
        if (rom.offset.is_some() || offset != 0) && size.is_some_and(|size| offset as u64 >= size) {
            return Err(self.error(
                &format!("{field}.offset"),
                "offset must leave a nonempty ROM",
            ));
        }
        if let Some(entry) = adapter {
            if offset != entry.rom_offset {
                return Err(self.error(
                    &format!("{field}.offset"),
                    "offset differs from the catalogue ROM layout",
                ));
            }
        }
        if let Some(sizes) = &mut self.rom_sizes {
            sizes.push(size.map(|size| size.saturating_sub(offset as u64)));
        }
        Ok(LoadImage {
            base,
            source: if let Some(bytes) = bytes {
                if offset == 0 {
                    ImageSource::Inline(bytes)
                } else {
                    ImageSource::RomContainer { bytes, offset }
                }
            } else if let Some(parts) = &rom.parts {
                ImageSource::Interleaved(
                    parts
                        .iter()
                        .enumerate()
                        .map(|(index, part)| assembly::ImagePart {
                            path: self
                                .origins
                                .image_path(&format!("{field}.parts[{index}].image"), &part.image),
                            lane: match part.lane {
                                v1::Lane::Even => assembly::Lane::Even,
                                v1::Lane::Odd => assembly::Lane::Odd,
                            },
                            offset: part.offset,
                        })
                        .collect(),
                )
            } else {
                ImageSource::File(rom.image.as_ref().map_or_else(
                    || PathBuf::from("pending-firmware"),
                    |image| self.origins.image_path(&format!("{field}.image"), image),
                ))
            },
            kind: ImageKind::Rom,
            provides,
        })
    }

    fn rtc(&mut self, rtc: &v1::Rtc) -> Result<assembly::CmosCfg, LowerError> {
        let start = match rtc.start.as_deref() {
            Some(text) => rtc_start(text).ok_or_else(|| {
                self.error("board.rtc.start", "expected quoted YYYY-MM-DDTHH:MM:SS")
            })?,
            None => RtcStart::default(),
        };
        let nvram = match rtc.nvram.as_ref().unwrap_or(&v1::Nvram::Derived) {
            v1::Nvram::Derived => NvramSource::Derived,
            v1::Nvram::Blank => NvramSource::Blank,
            v1::Nvram::Image(image) => {
                if image.write == Some(v1::WriteMode::ReadOnly) {
                    return Err(self.error("board.rtc.nvram.write", "readonly NVRAM is unsupported: the MC146818 has no write-protect input; use write = \"volatile\""));
                }
                let journal = image.write == Some(v1::WriteMode::Journal);
                let journal_path = if journal {
                    let field = "board.rtc.nvram.journal";
                    let path = self.required(field, image.journal.as_deref())?;
                    if path.is_empty() {
                        return Err(self.error(field, "an explicit journal path is required"));
                    }
                    Some(self.origins.image_path(field, path))
                } else {
                    self.write(
                        "board.rtc.nvram.write",
                        image.write,
                        image.journal.is_some(),
                        image.keep_generations.is_some(),
                    )?;
                    None
                };
                if self.metadata {
                    return Ok(assembly::CmosCfg {
                        start,
                        nvram: NvramSource::Blank,
                        ..Default::default()
                    });
                }
                let (bytes, expected_sha256) = self.image(
                    "board.rtc.nvram.image",
                    image.image.as_deref(),
                    image.sha256.as_ref(),
                    "board.rtc.nvram.sha256",
                    journal,
                )?;
                let actual = bytes.len();
                let bytes: [u8; 64] = bytes.try_into().map_err(|_| {
                    self.error(
                        "board.rtc.nvram.image",
                        format!("NVRAM image must contain 64 bytes, got {actual}"),
                    )
                })?;
                match journal_path {
                    Some(journal) => NvramSource::Journal {
                        image: self.origins.image_path(
                            "board.rtc.nvram.image",
                            image.image.as_deref().unwrap_or_default(),
                        ),
                        expected_sha256,
                        journal,
                        keep_generations: image
                            .keep_generations
                            .map_or(assembly::WriteMode::DEFAULT_KEEP_GENERATIONS, |n| n.get()),
                    },
                    None => NvramSource::Image(bytes),
                }
            }
        };
        Ok(assembly::CmosCfg {
            ram: assembly::CmosRam::Bytes64,
            start,
            nvram,
            timing: None,
        })
    }

    fn card(
        &mut self,
        index: usize,
        card: &v1::Card,
        mmio: &mut Vec<MmioWindow>,
        drives: &mut Vec<DriveConfig>,
    ) -> Result<DeviceConfig, LowerError> {
        let field = format!("card[{index}]");
        if card.remove.is_some() {
            return Err(self.error(
                &format!("{field}.remove"),
                "remove is only accepted in an extends layer",
            ));
        }
        let kind = self.required(&format!("{field}.kind"), card.kind)?;
        if kind != v1::CardKind::Uart16550 && card.mouse.is_some() {
            return Err(self.error(
                &format!("{field}.mouse"),
                "mouse is only supported by uart-16550 cards",
            ));
        }
        if kind != v1::CardKind::Ide && card.controller_model.is_some() {
            return Err(self.error(
                &format!("{field}.controller_model"),
                "controller_model is only supported by ide cards",
            ));
        }
        if !matches!(
            kind,
            v1::CardKind::Uart16550 | v1::CardKind::Ide | v1::CardKind::LotechEms
        ) && card.port.is_some()
        {
            return Err(self.error(
                &format!("{field}.port"),
                "port override is not supported by this fixed-port card kind",
            ));
        }
        if kind != v1::CardKind::Sb16
            && (card.base.is_some() || card.dma8.is_some() || card.dma16.is_some())
        {
            return Err(self.error(
                &field,
                "base, dma8 and dma16 are only supported by sb16 cards",
            ));
        }
        if kind == v1::CardKind::Sb16 && card.dma.is_some() {
            return Err(self.error(&format!("{field}.dma"), "sb16 uses dma8 and dma16"));
        }
        if kind != v1::CardKind::LotechEms && (card.frame.is_some() || card.size_kib.is_some()) {
            return Err(self.error(
                &field,
                "frame and size_kib are only supported by lotech-ems cards",
            ));
        }
        let mut device = match kind {
            v1::CardKind::LotechEms => {
                let defaults = assembly::LotechEmsCfg::default();
                let cfg = assembly::LotechEmsCfg {
                    port: card.port.unwrap_or(defaults.port),
                    frame: self.required(&format!("{field}.frame"), card.frame)?,
                    size_kib: card.size_kib.unwrap_or(defaults.size_kib),
                };
                if let Some(name) = cfg.invalid_field() {
                    return Err(self.error(
                        &format!("{field}.{name}"),
                        "invalid Lo-tech EMS jumper or SRAM population",
                    ));
                }
                mmio.push(MmioWindow::new(
                    u64::from(cfg.frame) << 4,
                    0x10000,
                    DeviceId::LotechEms,
                ));
                DeviceConfig::new(
                    DeviceId::LotechEms,
                    DeviceKind::LotechEms(cfg),
                    cfg.port_windows(),
                )
            }
            v1::CardKind::Mda | v1::CardKind::Vga | v1::CardKind::VgaText => {
                let descriptor = kind.descriptor();
                let device = descriptor.device();
                if let Some(window) = descriptor.mmio(&device) {
                    mmio.push(window);
                }
                device
            }
            v1::CardKind::Uart16550 => {
                let base = card
                    .port
                    .unwrap_or(assembly::descriptors::UART_PORT.default as u16);
                let default_irq = assembly::descriptors::uart_default_irq(base);
                let irq = match card.irq {
                    Some(irq) => line(irq),
                    None => Some(self.required(&format!("{field}.irq"), default_irq)?),
                };
                let mut device = DeviceConfig::new(
                    DeviceId::Uart,
                    DeviceKind::Uart(UartCfg {
                        base,
                        mouse: card.mouse == Some(v1::SerialMouseModel::Microsoft),
                    }),
                    vec![PortWindow::new(base, base.saturating_add(7))],
                );
                device.irq = irq;
                device
            }
            v1::CardKind::Sb16 => {
                let cfg = assembly::Sb16Cfg {
                    dma16: card
                        .dma16
                        .unwrap_or(assembly::descriptors::SB16_DEFAULT.dma16),
                    base: card
                        .base
                        .unwrap_or(assembly::descriptors::SB16_DEFAULT.base),
                };
                DeviceConfig::new(DeviceId::Sb16, DeviceKind::Sb16(cfg), cfg.port_windows())
                    .with_irq(assembly::descriptors::SB16_DEFAULT_IRQ)
                    .with_dma(DmaSpec {
                        controller: DeviceId::Dma,
                        channel: card
                            .dma8
                            .unwrap_or(assembly::descriptors::SB16_DEFAULT_DMA8),
                    })
            }
            v1::CardKind::Fdc765 => assembly::descriptors::CardKind::Fdc765.device(),
            v1::CardKind::Ide => {
                let base = card.port.unwrap_or(assembly::descriptors::IDE_PORTS[0]);
                let cfg = assembly::IdeCfg {
                    model: match card.controller_model.unwrap_or(v1::ControllerModel::Ata) {
                        v1::ControllerModel::Ata => assembly::AtaControllerModel::Ata,
                        v1::ControllerModel::AtAdapter => assembly::AtaControllerModel::AtAdapter,
                    },
                    base,
                    control: base.saturating_add(0x206),
                };
                DeviceConfig::new(DeviceId::Ide, DeviceKind::Ide(cfg), cfg.port_windows())
                    .with_irq(cfg.irq())
            }
        };
        if matches!(
            kind,
            v1::CardKind::Mda | v1::CardKind::VgaText | v1::CardKind::Vga | v1::CardKind::LotechEms
        ) && card.irq.is_some()
        {
            return Err(self.error(&format!("{field}.irq"), "this card has no interrupt source"));
        }
        if kind != v1::CardKind::Fdc765 && card.dma.is_some() {
            return Err(self.error(&format!("{field}.dma"), "this card cannot master DMA"));
        }
        if let Some(irq) = card.irq {
            device.irq = line(irq);
        }
        if let Some(dma) = card.dma {
            device.dma = line(dma).map(|channel| DmaSpec {
                controller: DeviceId::Dma,
                channel,
            });
        }
        if !matches!(kind, v1::CardKind::Fdc765 | v1::CardKind::Ide) && card.drives.is_some() {
            return Err(self.error(
                &format!("{field}.drives"),
                "drives are only accepted on a disk controller",
            ));
        }
        let mut ordered: Vec<_> = card.drives.iter().flatten().enumerate().collect();
        ordered.sort_by_key(|(_, drive)| {
            if kind == v1::CardKind::Ide {
                drive
                    .channel
                    .map(|channel| u8::from(channel == v1::Channel::Slave))
            } else {
                drive.unit
            }
        });
        for (drive_index, drive) in ordered {
            let drive_field = format!("{field}.drives[{drive_index}]");
            let (unit, drive_kind, label) = if kind == v1::CardKind::Ide {
                if drive.remove.is_some() || drive.unit.is_some() || drive.drive_type.is_some() {
                    return Err(self.error(&drive_field, "IDE drives accept channel and geometry; unit/type are floppy-only, remove is extends-only"));
                }
                let channel = self.required(&format!("{drive_field}.channel"), drive.channel)?;
                let unit = u8::from(channel == v1::Channel::Slave);
                let v1::Geometry::Chs(geometry) =
                    self.required(&format!("{drive_field}.geometry"), drive.geometry.as_ref())?;
                (
                    unit,
                    DriveKind::HardDisk {
                        geometry: assembly::HardDiskGeometry {
                            cylinders: self.required(
                                &format!("{drive_field}.geometry.cylinders"),
                                geometry.cylinders,
                            )?,
                            heads: self.required(
                                &format!("{drive_field}.geometry.heads"),
                                geometry.heads,
                            )?,
                            sectors_per_track: self.required(
                                &format!("{drive_field}.geometry.sectors"),
                                geometry.sectors,
                            )?,
                            bytes_per_sector: 512,
                        },
                    },
                    format!(
                        "{field}.drives[channel={}]",
                        if unit == 0 { "master" } else { "slave" }
                    ),
                )
            } else {
                let unit = self.required(&format!("{drive_field}.unit"), drive.unit)?;
                if drive.remove.is_some() || drive.channel.is_some() || drive.geometry.is_some() {
                    return Err(self.error(&drive_field, "floppy drives accept unit and type; channel/geometry are IDE-only, remove is extends-only"));
                }
                let geometry =
                    match self.required(&format!("{drive_field}.type"), drive.drive_type)? {
                        v1::DriveType::Hd144 => assembly::descriptors::FloppyType::HD_1_44.geometry,
                        v1::DriveType::Hd12 => assembly::descriptors::FloppyType::HD_1_2.geometry,
                        v1::DriveType::Dd720 => assembly::descriptors::FloppyType::DD_720K.geometry,
                        v1::DriveType::Dd360 => assembly::descriptors::FloppyType::DD_360K.geometry,
                    };
                (
                    unit,
                    DriveKind::Floppy { geometry },
                    format!("{field}.drives[unit={unit}]"),
                )
            };
            self.drives.push(DriveOrigin {
                image_name: drive
                    .image
                    .as_ref()
                    .and_then(|name| Path::new(name).file_name())
                    .map(|name| name.to_string_lossy().into_owned()),
                field: drive_field.clone(),
                label,
            });
            if drive.media.is_some() || drive.empty.is_some() {
                if kind == v1::CardKind::Ide
                    || drive.image.is_some()
                    || drive.sha256.is_some()
                    || drive.write.is_some()
                    || drive.journal.is_some()
                    || drive.keep_generations.is_some()
                {
                    return Err(self.error(&drive_field, "media/empty require a floppy and cannot be combined with image/write/journal"));
                }
                let mut entries = Vec::new();
                for (index, medium) in drive.media.iter().flatten().enumerate() {
                    let field = format!("{drive_field}.media[{index}]");
                    let write = if medium.write == Some(v1::WriteMode::Journal) {
                        let path =
                            self.required(&format!("{field}.journal"), medium.journal.as_deref())?;
                        if path.is_empty() {
                            return Err(self.error(&field, "an explicit journal path is required"));
                        }
                        assembly::WriteMode::Journal {
                            path: self.origins.image_path(&format!("{field}.journal"), path),
                            keep_generations: medium
                                .keep_generations
                                .map_or(assembly::WriteMode::DEFAULT_KEEP_GENERATIONS, |v| v.get()),
                        }
                    } else {
                        self.write(
                            &format!("{field}.write"),
                            medium.write,
                            medium.journal.is_some(),
                            medium.keep_generations.is_some(),
                        )?
                    };
                    if self.metadata {
                        entries.push(assembly::FloppyMedium {
                            name: Path::new(&medium.image)
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            image: ImageSource::File(
                                self.origins
                                    .image_path(&format!("{field}.image"), &medium.image),
                            ),
                            expected_sha256: medium.sha256.as_ref().map(metadata_pin),
                            write,
                        });
                        continue;
                    }
                    let (length, digest) = if matches!(write, assembly::WriteMode::Journal { .. }) {
                        self.journal_image(
                            &format!("{field}.image"),
                            Some(&medium.image),
                            medium.sha256.as_ref(),
                            &format!("{field}.sha256"),
                        )?
                    } else {
                        let (bytes, digest) = self.image(
                            &format!("{field}.image"),
                            Some(&medium.image),
                            medium.sha256.as_ref(),
                            &format!("{field}.sha256"),
                            false,
                        )?;
                        (bytes.len() as u64, digest)
                    };
                    if let DriveKind::Floppy { geometry } = drive_kind {
                        if geometry.medium_for_len(length).is_none() {
                            return Err(self.error(
                                &field,
                                format!(
                                    "medium image is {length} bytes but {}",
                                    assembly::descriptors::floppy_drive_media(geometry)
                                ),
                            ));
                        }
                    }
                    let path = self
                        .origins
                        .image_path(&format!("{field}.image"), &medium.image);
                    entries.push(assembly::FloppyMedium {
                        name: std::path::Path::new(&medium.image)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        image: ImageSource::File(path),
                        expected_sha256: Some(digest),
                        write,
                    });
                }
                if entries.is_empty() && drive.empty != Some(true) {
                    return Err(self.error(
                        &drive_field,
                        "a media list must be nonempty unless empty=true",
                    ));
                }
                drives.push(DriveConfig {
                    media: Some(assembly::FloppyMediaList {
                        entries,
                        empty: drive.empty.unwrap_or(false),
                        inserted: 0,
                    }),
                    controller: device.id,
                    unit,
                    kind: drive_kind,
                    image: ImageSource::Blank {
                        size: drive_kind.image_bytes(),
                    },
                    expected_sha256: None,
                    write: assembly::WriteMode::Volatile,
                });
                continue;
            }
            let write = if drive.write == Some(v1::WriteMode::Journal) {
                let field = format!("{drive_field}.journal");
                let journal = self.required(&field, drive.journal.as_deref())?;
                if journal.is_empty() {
                    return Err(self.error(&field, "an explicit journal path is required"));
                }
                let path = self.origins.image_path(&field, journal);
                match drive.keep_generations {
                    Some(count) => assembly::WriteMode::Journal {
                        path,
                        keep_generations: count.get(),
                    },
                    None => assembly::WriteMode::journal(path),
                }
            } else {
                self.write(
                    &format!("{drive_field}.write"),
                    drive.write,
                    drive.journal.is_some(),
                    drive.keep_generations.is_some(),
                )?
            };
            if self.metadata {
                drives.push(DriveConfig {
                    media: None,
                    controller: device.id,
                    unit,
                    kind: drive_kind,
                    image: drive.image.as_ref().map_or(
                        ImageSource::Blank {
                            size: drive_kind.image_bytes(),
                        },
                        |image| {
                            ImageSource::File(
                                self.origins
                                    .image_path(&format!("{drive_field}.image"), image),
                            )
                        },
                    ),
                    expected_sha256: drive.sha256.as_ref().map(metadata_pin),
                    write,
                });
                continue;
            }
            let journal = matches!(write, assembly::WriteMode::Journal { .. });
            let (length, digest, bytes) = if journal {
                let (length, digest) = self.journal_image(
                    &format!("{drive_field}.image"),
                    drive.image.as_deref(),
                    drive.sha256.as_ref(),
                    &format!("{drive_field}.sha256"),
                )?;
                (length, digest, Vec::new())
            } else {
                let (bytes, digest) = self.image(
                    &format!("{drive_field}.image"),
                    drive.image.as_deref(),
                    drive.sha256.as_ref(),
                    &format!("{drive_field}.sha256"),
                    false,
                )?;
                (bytes.len() as u64, digest, bytes)
            };
            let image = if journal {
                // Assembly needs the base path to reject aliases and mount the journal.
                if let Err(error) = drive_kind.check_image_len(drives.len(), length) {
                    return Err(self.validation_error(error));
                }
                let field = format!("{drive_field}.image");
                ImageSource::File(
                    self.origins
                        .image_path(&field, self.required(&field, drive.image.as_deref())?),
                )
            } else {
                ImageSource::Inline(bytes)
            };
            drives.push(DriveConfig {
                media: None,
                controller: device.id,
                unit,
                kind: drive_kind,
                image,
                expected_sha256: journal.then_some(digest),
                write,
            });
        }
        self.devices.push((device.id, field));
        Ok(device)
    }

    fn bios_source_field(&self) -> &'static str {
        if self.bios_parts {
            "firmware.bios.parts"
        } else {
            "firmware.bios.image"
        }
    }

    fn validation_error(&self, error: ConfigError) -> LowerError {
        use ConfigError::*;
        let device_fields = |ids: &[DeviceId]| -> Vec<String> {
            let mut fields: Vec<_> = self
                .devices
                .iter()
                .filter(|(id, _)| ids.contains(id))
                .map(|(_, field)| field.clone())
                .collect();
            fields.sort_by_key(|field| field.starts_with("board"));
            fields
        };
        let mut message = error.to_string();
        let fields = match &error {
            DriveImageSha256 { drive_index, .. } => {
                message = replace_index_prefix(
                    &message,
                    &format!("drive {drive_index} "),
                    &format!("{} ", self.drive_label(*drive_index, false)),
                );
                self.drives
                    .get(*drive_index)
                    .map(|drive| format!("{}.image", drive.field))
                    .into_iter()
                    .collect()
            }
            FirmwareParts { .. } | FirmwarePartsSwapped { .. } | FirmwarePartsMismatch { .. } => {
                vec!["firmware.bios.parts".into()]
            }
            MissingSystemBios { .. }
            | FirmwareNotListed { .. }
            | ConflictingCmosCapabilities { .. }
            | FirmwareBoardMismatch { .. }
            | FirmwareNeedsCpu { .. }
            | FirmwareRequiresDevice { .. }
            | BiosPlacement { .. }
            | VideoBiosAsSystemBios { .. } => vec![self.bios_source_field().into()],
            FirmwareDoesNotScanOptionRoms { image_index, .. }
            | FirmwareDoesNotScanVideoRoms { image_index, .. }
            | OptionRomInVideoWindow { image_index, .. }
            | OptionRomNotListed { image_index, .. }
            | OptionRomCardMismatch { image_index, .. }
            | OptionRomBadHeader { image_index, .. }
            | OptionRomBadChecksum { image_index, .. } => {
                self.images.get(*image_index).cloned().into_iter().collect()
            }
            SeaBiosVideoRomBase { image_index, .. }
            | OptionRomMisaligned { image_index, .. }
            | OptionRomOutsideScanWindow { image_index, .. } => self
                .images
                .get(*image_index)
                .map(|field| format!("{field}.base"))
                .into_iter()
                .collect(),
            LotechEmsFrameInSystemRom { device } => device_fields(&[*device])
                .into_iter()
                .map(|field| format!("{field}.frame"))
                .collect(),
            AdapterSpaceOverlapsRam { .. }
            | HighRomAliasOverlap { .. }
            | BoardPhysicalAddressOutOfRange { .. }
            | HighRomAliasSourceMmio { .. } => vec!["memory".into()],
            BoardPhysicalWidth { .. } => vec!["cpu.model".into()],
            ExtendedRamLimit { .. } => vec!["memory.extended_kib".into()],
            FirmwareProvidesMismatch { image_index, .. } => {
                vec![format!("{}.provides", self.image_label(*image_index))]
            }
            DisplaySwitchUnavailable { .. } => vec!["board.display_switch".into()],
            ConflictingVgaCards => device_fields(&[DeviceId::Vga]),
            MultipleIdeChannels => device_fields(&[DeviceId::Ide]),
            DebugConsoleBoard | FwCfgBoard | Ibm5170RtcRam | SysCtrlPortABoard => {
                vec!["board".into()]
            }
            FirmwareReset { .. } => vec!["cpu".into()],
            SystemBiosOverlapsRam { .. } => vec!["memory".into(), self.bios_source_field().into()],
            DuplicateDeviceId(id) | MmioDeviceMissing(id) => device_fields(&[*id]),
            InvalidDmaChannel { device, .. }
            | InvalidSb16Resources { device }
            | InvalidLotechEmsResources { device }
            | InvalidIdeResources { device }
            | InvalidDebugConsoleResources { device }
            | InvalidSysCtrlPortAResources { device }
            | InvalidFwCfgResources { device }
            | InvalidFwCfgFiles { device, .. }
            | FwCfgCom1Missing { device }
            | InvalidIdePortWindows { device }
            | NoProvider { device, .. }
            | InvalidRtcStart { device, .. }
            | SelfAccessCoupling { device }
            | AccessDependentMissing { device, .. }
            | UartPortWindowOutOfSpace { device, .. }
            | UnsupportedWordPort { device, .. }
            | UnalignedWordPortWindow { device, .. }
            | UnsupportedObservationOutput { device, .. }
            | DuplicateObservationRoute { device, .. }
            | EmptyMmioWindow { device, .. }
            | InvalidIrqLine { device, .. }
            | MmioOverlapsRam { device, .. }
            | BoardDeviceMissing { device, .. }
            | BoardDeviceOrder { device, .. }
            | BoardDeviceWiring { device, .. }
            | BoardCardIrq { device, .. }
            | BoardCardDma { device, .. }
            | BoardCardDmaController { device, .. } => device_fields(&[*device]),
            DmaConflict { a, b, .. }
            | DuplicatePicFace { a, b, .. }
            | PortOverlap { a, b, .. }
            | PortOverlapDecodeOrder { a, b, .. }
            | MmioOverlap { a, b, .. } => device_fields(&[*a, *b]),
            IrqConflict { a, b, line } => {
                let mut fields = Vec::new();
                for id in [a, b] {
                    if let Some((_, _, field)) = self
                        .irq_fields
                        .iter()
                        .find(|(device, irq, _)| device == id && irq == line)
                    {
                        fields.push(field.clone());
                    } else {
                        fields.extend(device_fields(&[*id]));
                    }
                }
                fields.sort_by_key(|field| field.starts_with("board"));
                fields
            }
            DmaControllerMissing { requester, .. } => device_fields(&[*requester]),
            MultipleMouseTargets => {
                let mut fields = vec!["board.mouse".into()];
                fields.extend(
                    self.devices
                        .iter()
                        .filter(|(id, _)| *id == DeviceId::Uart)
                        .map(|(_, field)| format!("{field}.mouse")),
                );
                fields
            }
            UnsupportedMouse { .. } => vec!["board.mouse".into()],
            Nvram { .. } | UnsupportedExtendedNvram => vec!["board.rtc.nvram".into()],
            Journal { drive_index, .. }
            | DriveInput { drive_index, .. }
            | UnsupportedCmosDiskGeometry { drive_index, .. }
            | InvalidAtaGeometry { drive_index }
            | AtaMissingMaster { drive_index, .. }
            | UnsupportedFloppySectorSize { drive_index, .. }
            | DriveControllerMissing { drive_index, .. }
            | DriveControllerKind { drive_index, .. }
            | DriveUnitUnavailable { drive_index, .. }
            | ImageGeometryMismatch { drive_index, .. } => {
                message = replace_index_prefix(
                    &message,
                    &format!("drive {drive_index} "),
                    &format!("{} ", self.drive_label(*drive_index, false)),
                );
                self.drives
                    .get(*drive_index)
                    .map(|drive| drive.field.clone())
                    .into_iter()
                    .collect()
            }
            DuplicateDriveUnit { a, b, .. } => {
                message = replace_index_prefix(
                    &message,
                    &format!("drives {a} and {b} "),
                    &format!(
                        "{} and {} ",
                        self.drive_label(*a, true),
                        self.drive_label(*b, true)
                    ),
                );
                [*a, *b]
                    .iter()
                    .filter_map(|i| self.drives.get(*i).map(|drive| drive.field.clone()))
                    .collect()
            }
            ImageInput { image_index, .. } => {
                self.images.get(*image_index).cloned().into_iter().collect()
            }
            ServicesOnRamLoad { image } => {
                message = replace_index_prefix(
                    &message,
                    &format!("RAM load image {image} "),
                    &format!("{} ", self.image_label(*image)),
                );
                self.images.get(*image).cloned().into_iter().collect()
            }
            ImageOverlapsMmio {
                image_index,
                device,
                ..
            } => {
                message = replace_index_prefix(
                    &message,
                    &format!("image {image_index} "),
                    &format!("{} ", self.image_label(*image_index)),
                );
                let mut fields: Vec<_> =
                    self.images.get(*image_index).cloned().into_iter().collect();
                fields.extend(device_fields(&[*device]));
                fields
            }
            ImageOutsideRam { image_index, .. } => {
                message = replace_index_prefix(
                    &message,
                    &format!("ram-load image {image_index} "),
                    &format!("{} ", self.image_label(*image_index)),
                );
                self.images.get(*image_index).cloned().into_iter().collect()
            }
            ImageOverlap { a, b, .. } => {
                message = replace_index_prefix(
                    &message,
                    &format!("images {a} and {b} "),
                    &format!("{} and {} ", self.image_label(*a), self.image_label(*b)),
                );
                [*a, *b]
                    .iter()
                    .filter_map(|i| self.images.get(*i).cloned())
                    .collect()
            }
            BoardConfiguration { .. } => vec!["board.model".into()],
            InvalidCpuRate { .. } | StepBoundedCpuRequired(_) => vec!["cpu".into()],
            RegionOutOfSpace { .. } => self.images.clone(),
            ImageRead { .. } => vec!["firmware".into()],
        };
        let field = fields.first().map_or("memory", String::as_str);
        for related in fields.iter().skip(1) {
            message.push_str(&format!(
                "; {}: {}",
                self.origins.get(related),
                self.label(related)
            ));
        }
        let mut diagnostic = self.error(field, message);
        diagnostic.validation = Some(Box::new(error));
        diagnostic
    }

    fn drive_label(&self, index: usize, with_origin: bool) -> String {
        self.drives.get(index).map_or_else(
            || format!("drive #{index}"),
            |drive| {
                let label = self.label(&drive.field);
                if with_origin {
                    format!("{label} ({})", self.origins.get(&drive.field))
                } else {
                    label
                }
            },
        )
    }

    fn image_label(&self, index: usize) -> String {
        self.images
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("image #{index}"))
    }
}

// Assembly owns the wording; only its configuration indices need file labels.
fn replace_index_prefix(message: &str, prefix: &str, label: &str) -> String {
    message
        .strip_prefix(prefix)
        .map_or_else(|| message.into(), |tail| format!("{label}{tail}"))
}

fn line<const MAX: u16>(value: v1::NumberOrNone<MAX>) -> Option<u8> {
    match value {
        v1::NumberOrNone::Number(value) => Some(value as u8),
        v1::NumberOrNone::None => None,
    }
}

fn kind_name(kind: v1::CardKind) -> &'static str {
    kind.descriptor().name()
}

fn rtc_start(text: &str) -> Option<RtcStart> {
    let bytes = text.as_bytes();
    if bytes.len() != 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, byte)| ![4, 7, 10, 13, 16].contains(&i) && !byte.is_ascii_digit())
    {
        return None;
    }
    Some(RtcStart {
        year: text.get(0..4)?.parse().ok()?,
        month: text.get(5..7)?.parse().ok()?,
        day: text.get(8..10)?.parse().ok()?,
        hour: text.get(11..13)?.parse().ok()?,
        minute: text.get(14..16)?.parse().ok()?,
        second: text.get(17..19)?.parse().ok()?,
    })
}

/// Expand a complete typed document, read/hash its images and validate the result.
///
/// Origins define per-field path bases and diagnostic positions. No environment
/// or clock is consulted. Inheritance must be resolved before calling lowering;
/// unresolved `extends` and `remove` are rejected rather than ignored.
pub fn lower<R: ImageReader>(
    document: &MachineFile,
    origins: &Origins,
    reader: &mut R,
    options: assembly::AssemblyOptions,
) -> Result<MachineConfig, LowerError> {
    lower_tracked(document, origins, reader, options).map(|(config, _)| config)
}

/// [`lower`](fn@crate::lower), also answering the diagnostics context that words an
/// assembly error of the lowered config at its source: what a loader that
/// assembles later keeps beside the config.
pub fn lower_tracked<R: ImageReader>(
    document: &MachineFile,
    origins: &Origins,
    reader: &mut R,
    options: assembly::AssemblyOptions,
) -> Result<(MachineConfig, AssemblyDiagnostics), LowerError> {
    lower_tracked_mode(document, origins, reader, options, Mode::Full)
}

/// How much of the lowering runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Every image read and checked, then assembly validation.
    Full,
    /// Nothing read and no validation, for a description of a draft that
    /// may still lack its BIOS.
    Describe,
    /// Nothing read, but a complete machine: its BIOS is required and the
    /// machine validated, as in the full lowering.
    Check,
}

/// The lowering in its metadata mode, with the default firmware options: no
/// image is read, a ROM whose size no catalogue entry states is sized by
/// `ImageReader::image_length`. Unlike a description, it requires the
/// system BIOS and validates the machine, with the full lowering's words;
/// what validation can only decide on the bytes of a ROM (the services of a
/// ROM the catalogue does not list, its format) waits for them. A launch
/// document's check without content runs it.
pub(crate) fn lower_metadata<R: ImageReader>(
    document: &MachineFile,
    origins: &Origins,
    reader: &mut R,
) -> Result<MachineConfig, LowerError> {
    lower_tracked_mode(
        document,
        origins,
        reader,
        assembly::AssemblyOptions::default(),
        Mode::Check,
    )
    .map(|(config, _)| config)
}

fn lower_tracked_mode<R: ImageReader>(
    document: &MachineFile,
    origins: &Origins,
    reader: &mut R,
    options: assembly::AssemblyOptions,
    mode: Mode,
) -> Result<(MachineConfig, AssemblyDiagnostics), LowerError> {
    crate::bindings::validate_sources(document, origins)?;
    if crate::bindings::has_bindings(document) {
        return Err(LowerError {
            origin: origins.get("").clone(),
            field: String::new(),
            message: "unresolved content bindings; composition is required".into(),
            pin_origin: None,
            validation: None,
        });
    }
    let metadata = mode != Mode::Full;
    let mut context = Context {
        origins,
        reader,
        labels: document
            .card
            .iter()
            .flatten()
            .map(|card| {
                card.id
                    .clone()
                    .or_else(|| card.kind.map(kind_name).map(String::from))
            })
            .collect(),
        devices: Vec::new(),
        irq_fields: Vec::new(),
        drives: Vec::new(),
        images: Vec::new(),
        image_paths: Vec::new(),
        options,
        bios_parts: false,
        metadata,
        rom_sizes: (mode == Mode::Check).then(Vec::new),
    };
    if document.extends.is_some() {
        return Err(context.error(
            "extends",
            "unresolved extends; call resolve before lowering",
        ));
    }
    let mut card_ids = BTreeMap::new();
    for (index, card) in document.card.iter().flatten().enumerate() {
        let Some(id) = card.id.as_deref().or_else(|| card.kind.map(kind_name)) else {
            continue;
        };
        if let Some(first) = card_ids.insert(id, index) {
            let first_field = format!("card[{first}]");
            return Err(context.error(
                &format!("card[{index}]"),
                format!(
                    "duplicate card id `{id}`; first defined at {}: {}",
                    origins.get(&first_field),
                    context.label(&first_field)
                ),
            ));
        }
    }
    let board = context.required(
        "board.model",
        document.board.as_ref().and_then(|board| board.model),
    )?;
    let board = board_model(board);
    let mut devices = board.motherboard_devices();
    if let Some(settings) = &document.board {
        if let Some(mouse) = settings.mouse {
            for device in &mut devices {
                if let DeviceKind::Keyboard(cfg) = &mut device.kind {
                    cfg.mouse = mouse;
                }
            }
        }
        if let Some(port) = settings.post_port {
            let port = match port {
                v1::NumberOrNone::Number(port) => Some(port),
                v1::NumberOrNone::None => None,
            };
            if let Some(post) = devices.iter_mut().find(|d| d.id == DeviceId::Post) {
                if let Some(port) = port {
                    post.kind = DeviceKind::Post(PostPortCfg { port });
                    post.ports = vec![PortWindow::single(port)];
                }
            }
            if port.is_none() {
                devices.retain(|d| d.id != DeviceId::Post);
            }
            if port != Some(0x80) {
                if let Some(dma) = devices.iter_mut().find(|d| d.id == DeviceId::Dma) {
                    dma.ports
                        .iter_mut()
                        .filter(|window| window.start == 0x81)
                        .for_each(|window| window.start = 0x80);
                }
            }
        }
        if let Some(rtc) = &settings.rtc {
            let mut cfg = context.rtc(rtc)?;
            if board.is_generic_isa() {
                cfg.ram = assembly::CmosRam::Bytes128;
            }
            if let Some(device) = devices.iter_mut().find(|d| d.id == DeviceId::Rtc) {
                device.kind = DeviceKind::Rtc(cfg);
            }
        }
    }
    context.irq_fields.extend(devices.iter().flat_map(|device| {
        device
            .interrupt_outputs()
            .filter(|(output, _)| *output != 0)
            .map(|(_, irq)| (device.id, irq, "board.mouse".into()))
    }));
    context.devices.extend(devices.iter().map(|d| {
        (
            d.id,
            if d.id == DeviceId::Rtc {
                "board.rtc"
            } else {
                "board"
            }
            .into(),
        )
    }));
    let mut images = Vec::new();
    let mut system_bios_image = None;
    if let Some(bios) = document
        .firmware
        .as_ref()
        .and_then(|firmware| firmware.bios.as_ref())
    {
        images.push(context.rom("firmware.bios", bios, true, board.firmware_catalogue())?);
        system_bios_image = Some(0);
    } else if mode != Mode::Describe {
        return Err(context.error("firmware.bios", "required field is missing"));
    }
    for (index, rom) in document
        .firmware
        .iter()
        .flat_map(|firmware| firmware.option_roms.iter().flatten())
        .enumerate()
    {
        images.push(context.rom(
            &format!("firmware.option_roms[{index}]"),
            rom,
            false,
            board.firmware_catalogue(),
        )?);
    }
    let mut mmio = Vec::new();
    let mut drives = Vec::new();
    for (index, card) in document.card.iter().flatten().enumerate() {
        devices.push(context.card(index, card, &mut mmio, &mut drives)?);
    }
    let primary_ata = devices
        .iter()
        .any(|device| matches!(device.kind, DeviceKind::Ide(cfg) if cfg.base == 0x1F0));
    for device in &mut devices {
        if matches!(device.kind, DeviceKind::Fdc(_)) {
            device.ports = assembly::FdcCfg::port_windows(primary_ata);
        }
    }
    mmio.sort_by_key(|window| window.base);
    let conventional = document
        .memory
        .as_ref()
        .and_then(|memory| memory.conventional_kib)
        .unwrap_or(assembly::descriptors::CONVENTIONAL_KIB.default);
    let extended = document
        .memory
        .as_ref()
        .and_then(|memory| memory.extended_kib)
        .unwrap_or(assembly::descriptors::EXTENDED_KIB.default);
    if !assembly::descriptors::CONVENTIONAL_KIB.contains(conventional) {
        return Err(context.error(
            "memory.conventional_kib",
            "must be a multiple of 16 KiB, at most 640 KiB",
        ));
    }
    if !assembly::descriptors::extended_kib(board).contains(extended) {
        return Err(context.error(
            "memory.extended_kib",
            format!(
                "must be a multiple of 64 KiB, at most {} KiB on {}",
                assembly::descriptors::extended_kib(board).max,
                board.id()
            ),
        ));
    }
    let mut clock = board.default_clock();
    if let Some(rate) = document.cpu.as_ref().and_then(|cpu| cpu.rate) {
        clock.retired_per_pit_clock = rate.steps.get();
        clock.retired_per_pit_clock_denominator = rate.pit_clocks.get();
    }
    if board == BoardModel::Generic486Isa
        && document.cpu.as_ref().and_then(|cpu| cpu.model).is_none()
    {
        return Err(context.error(
            "cpu.model",
            "generic-486-isa requires an explicit 80486dx or 80486dx-cpuid model",
        ));
    }
    let chip = match document.cpu.as_ref().and_then(|cpu| cpu.model).unwrap_or(
        if board == BoardModel::Generic386Isa {
            v1::CpuModel::I80386
        } else {
            v1::CpuModel::I80286
        },
    ) {
        v1::CpuModel::I80286 => CpuChip::I80286,
        v1::CpuModel::I80386 => CpuChip::I80386,
        v1::CpuModel::I80486Dx => CpuChip::I80486Dx,
        v1::CpuModel::I80486DxCpuid => CpuChip::I80486DxCpuid,
    };
    let display_switch = match document
        .board
        .as_ref()
        .and_then(|board| board.display_switch)
    {
        None | Some(v1::DisplaySwitch::Auto) => assembly::DisplaySwitch::Auto,
        Some(v1::DisplaySwitch::Mono) => assembly::DisplaySwitch::Mono,
        Some(v1::DisplaySwitch::Colour) => assembly::DisplaySwitch::Colour,
    };
    let config = MachineConfig {
        display_switch,
        system_bios_image,
        board,
        cpu: CpuConfig {
            chip,
            fpu: match document.cpu.as_ref().and_then(|cpu| cpu.fpu) {
                None if board == BoardModel::Generic486Isa => assembly::FpuChip::Integrated,
                None | Some(v1::FpuModel::None) => assembly::FpuChip::None,
                Some(v1::FpuModel::I80287) => assembly::FpuChip::I80287,
                Some(v1::FpuModel::Integrated) => assembly::FpuChip::Integrated,
            },
            reset: ResetSpec::reset_vector(),
            segments: SegmentSeed::flat(0xF000),
        },
        clock,
        memory: MemoryConfig {
            address_space_bytes: MIB + u64::from(extended) * 1024,
            ram: assembly::descriptors::ram_regions_for_board(
                board,
                u64::from(conventional) * 1024,
                u64::from(extended) * 1024,
                &mmio,
                &images,
            ),
            mmio,
        },
        devices,
        drives,
        images,
    };
    // A check validates the machine it lowered, with the full lowering's
    // words, though with ROMs it has not read.
    if mode != Mode::Describe {
        if let Some(sizes) = &context.rom_sizes {
            // Keep placement in the shared validation order, while firmware
            // checks still defer anything that needs the unread bytes.
            let mut placement = config.clone();
            for (image, size) in placement.images.iter_mut().zip(sizes) {
                if let Some(size) = size {
                    image.source = ImageSource::Blank { size: *size };
                }
            }
            placement
                .validate_logical_with_firmware(&config)
                .map_err(|error| context.validation_error(error))?;
        } else {
            context
                .reader
                .validate_machine(&config)
                .map_err(|error| context.validation_error(error))?;
        }
        context
            .reader
            .validate_journal_paths(&config, origins, &context.image_paths)
            .map_err(|error| context.validation_error(error))?;
        assembly::check_option_rom_catalogue(&config, options)
            .map_err(|error| context.validation_error(error))?;
    }
    let diagnostics = AssemblyDiagnostics {
        bios_parts: context.bios_parts,
        origins: origins.clone(),
        labels: context.labels,
        devices: context.devices,
        irq_fields: context.irq_fields,
        drives: context.drives,
        images: context.images,
        image_paths: context.image_paths,
    };
    Ok((config, diagnostics))
}

fn metadata_pin(pin: &v1::Sha256) -> [u8; 32] {
    let mut digest = [0; 32];
    for (byte, pair) in digest
        .iter_mut()
        .zip(pin.as_str().as_bytes().as_chunks::<2>().0)
    {
        *byte = (hex_digit(pair[0]) << 4) | hex_digit(pair[1]);
    }
    digest
}
fn hex_digit(byte: u8) -> u8 {
    if byte.is_ascii_digit() {
        byte - b'0'
    } else {
        byte.to_ascii_lowercase() - b'a' + 10
    }
}

impl MachineFile {
    /// Describe resolved settings through the normal lowering rules, with
    /// the reader that sizes a ROM no catalogue entry states. The reader is
    /// asked for lengths only; nothing is read, hashed or opened. Unknown
    /// extents stay pending. This draft cannot authorize execution. See
    /// `backlog/docs/design/machine-images.md` §6; `machine-file`'s
    /// `describe` asks the host file system.
    pub fn describe_with<R: ImageReader>(
        &self,
        origins: &Origins,
        reader: &mut R,
    ) -> Result<assembly::capabilities::Draft, LowerError> {
        let (config, diagnostics) = lower_tracked_mode(
            self,
            origins,
            reader,
            assembly::AssemblyOptions::default(),
            Mode::Describe,
        )?;
        let mut draft = assembly::capabilities::Draft::new(config);
        draft.card_ids = diagnostics
            .devices
            .iter()
            .filter_map(|(id, path)| {
                path.strip_prefix("card[")
                    .and_then(|path| path.strip_suffix(']'))
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| diagnostics.labels.get(index).and_then(Clone::clone))
                    .map(|name| (*id, name))
            })
            .collect();
        draft.image_paths = diagnostics.images.iter().cloned().enumerate().collect();
        for (index, path) in &draft.image_paths {
            let rom = if path == "firmware.bios" {
                self.firmware.as_ref().and_then(|f| f.bios.as_ref())
            } else {
                path.strip_prefix("firmware.option_roms[")
                    .and_then(|s| s.strip_suffix(']'))
                    .and_then(|s| s.parse::<usize>().ok())
                    .and_then(|i| {
                        self.firmware
                            .as_ref()
                            .and_then(|f| f.option_roms.as_ref())
                            .and_then(|roms| roms.get(i))
                    })
            };
            if let Some(rom) = rom {
                if let Some(entry) = rom.sha256.as_ref().and_then(|pin| {
                    draft
                        .config
                        .board
                        .firmware_catalogue()
                        .iter()
                        .find(|entry| entry.sha256 == pin.as_str())
                }) {
                    draft.firmware_ids.push((*index, entry.id.into()));
                } else {
                    draft.firmware_files.push((
                        *index,
                        rom.image
                            .as_ref()
                            .and_then(|image| Path::new(image).file_name())
                            .map_or_else(
                                || "Chip dumps".into(),
                                |name| name.to_string_lossy().into_owned(),
                            ),
                    ));
                }
            }
        }
        Ok(draft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(origins: &'a Origins, reader: &'a mut NoImages) -> Context<'a, NoImages> {
        Context {
            origins,
            reader,
            labels: vec![Some("serial".into()), Some("diskette".into())],
            devices: vec![
                (DeviceId::MasterPic, "board".into()),
                (DeviceId::Uart, "card[0]".into()),
            ],
            irq_fields: Vec::new(),
            drives: (0..3)
                .map(|index| DriveOrigin {
                    image_name: None,
                    field: format!("card[1].drives[{index}]"),
                    label: format!("card[1].drives[unit={}]", u8::from(index != 0)),
                })
                .collect(),
            images: Vec::new(),
            image_paths: Vec::new(),
            options: assembly::AssemblyOptions::default(),
            bios_parts: false,
            metadata: false,
            rom_sizes: None,
        }
    }

    #[test]
    fn lotech_ems_jumpers_population_and_frame_decode() {
        for port in [0x260, 0x264, 0x268, 0x26C] {
            for frame in [0xC000, 0xD000] {
                for size in [512, 1024, 1536, 2048] {
                    let config = sb16_config(&format!("[[card]]\nkind='lotech-ems'\nport={port}\nframe={frame}\nsize_kib={size}\n")).unwrap();
                    let cfg = assembly::LotechEmsCfg {
                        port,
                        frame,
                        size_kib: size,
                    };
                    let card = config
                        .devices
                        .iter()
                        .find(|d| d.id == DeviceId::LotechEms)
                        .unwrap();
                    assert_eq!(card.kind, DeviceKind::LotechEms(cfg));
                    assert_eq!(card.ports, cfg.port_windows());
                    assert!(config.memory.mmio.contains(&MmioWindow::new(
                        u64::from(frame) << 4,
                        0x10000,
                        DeviceId::LotechEms
                    )));
                }
            }
        }
        let config = sb16_config("[[card]]\nkind='lotech-ems'\nframe=0xD000\n").unwrap();
        assert!(config.devices.iter().any(|d| d.kind
            == DeviceKind::LotechEms(assembly::LotechEmsCfg {
                frame: 0xD000,
                ..Default::default()
            })));
    }

    #[test]
    fn lotech_ems_frame_is_required_and_invalid_fields_are_named() {
        let error = sb16_config("[[card]]\nkind='lotech-ems'\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("card[lotech-ems].frame"), "{error}");
        for (field, value) in [("port", "0x261"), ("frame", "0xB000"), ("size_kib", "513")] {
            let frame = if field == "frame" {
                ""
            } else {
                "frame=0xD000\n"
            };
            let error = sb16_config(&format!(
                "[[card]]\nkind='lotech-ems'\n{frame}{field}={value}\n"
            ))
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(&format!("card[lotech-ems].{field}")),
                "{error}"
            );
        }
        let error = sb16_config("[[card]]\nkind='lotech-ems'\nframe=0xE000\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("frame 0xD000"), "{error}");
        assert!(error.contains("card[lotech-ems].frame"), "{error}");
    }

    #[test]
    fn lotech_ems_rejects_bad_resources_and_alias_conflicts() {
        for fields in [
            "port=0x261",
            "frame=0xB000",
            "size_kib=0",
            "size_kib=513",
            "irq=5",
            "dma=1",
            "base=0x260",
            "dma8=1",
            "dma16=5",
        ] {
            let frame = if fields.starts_with("frame=") {
                ""
            } else {
                "frame=0xD000\n"
            };
            assert!(
                sb16_config(&format!("[[card]]\nkind='lotech-ems'\n{frame}{fields}\n")).is_err(),
                "{fields}"
            );
        }
        for fields in ["frame=0xE000", "size_kib=2048"] {
            assert!(sb16_config(&format!("[[card]]\nkind='uart-16550'\n{fields}\n")).is_err());
        }
        for port in [0x260, 0x660, 0xFE60] {
            assert!(sb16_config(&format!(
                "[[card]]\nkind='lotech-ems'\nframe=0xD000\n[[card]]\nkind='uart-16550'\nport={port}\nirq=4\n"
            ))
            .is_err());
        }
    }

    fn sb16_config(cards: &str) -> Result<MachineConfig, LowerError> {
        struct Rom;
        impl ImageReader for Rom {
            fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
                Ok(vec![0xF4; 32768])
            }
        }
        let input = format!(
            "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='synthetic.bin'\n{cards}"
        );
        let document = crate::parse_toml("machine.toml", &input).unwrap();
        let origins = Origins::from_toml("machine.toml", &input).unwrap();
        lower(
            &document,
            &origins,
            &mut Rom,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
    }

    #[test]
    fn sb16_optional_defaults_and_all_valid_resources() {
        assert!(!sb16_config("")
            .unwrap()
            .devices
            .iter()
            .any(|d| d.id == DeviceId::Sb16));
        let config = sb16_config("[[card]]\nkind='sb16'\n").unwrap();
        let dsp = config
            .devices
            .iter()
            .find(|d| d.id == DeviceId::Sb16)
            .unwrap();
        assert_eq!(
            dsp.kind,
            DeviceKind::Sb16(assembly::Sb16Cfg {
                base: 0x220,
                dma16: 5
            })
        );
        assert_eq!(dsp.irq, Some(5));
        assert_eq!(dsp.dma.unwrap().channel, 1);
        assert_eq!(
            dsp.ports.iter().map(|p| p.start).collect::<Vec<_>>(),
            [
                0x220, 0x221, 0x222, 0x223, 0x224, 0x225, 0x226, 0x228, 0x229, 0x22A, 0x22C, 0x22E,
                0x22F, 0x388, 0x389, 0x38A, 0x38B
            ]
        );
        for base in [0x220, 0x240, 0x260, 0x280] {
            for irq in [2, 5, 7, 10] {
                for dma in [0, 1, 3] {
                    let config = sb16_config(&format!(
                        "[[card]]\nkind='sb16'\nbase={base}\nirq={irq}\ndma8={dma}\n"
                    ))
                    .unwrap();
                    let dsp = config
                        .devices
                        .iter()
                        .find(|d| d.id == DeviceId::Sb16)
                        .unwrap();
                    assert_eq!(
                        dsp.interrupt_outputs().next(),
                        Some((0, if irq == 2 { 9 } else { irq }))
                    );
                }
            }
        }
    }

    #[test]
    fn sb16_invalid_resources_and_port_irq_dma_conflicts_have_card_origins() {
        for field in [
            "base=0x230",
            "irq=3",
            "irq='none'",
            "dma8=2",
            "dma8=4",
            "dma16=4",
            "dma16=8",
            "port=0x220",
            "dma=1",
        ] {
            let error =
                sb16_config(&format!("[[card]]\nid='sound'\nkind='sb16'\n{field}\n")).unwrap_err();
            assert!(
                error.to_string().contains("card[sound]"),
                "{field}: {error}"
            );
        }
        for card in [
            "kind='uart-16550'\nport=0x226\nirq=4",
            "kind='uart-16550'\nirq=5",
            "kind='fdc-765'\ndma=1",
        ] {
            let error = sb16_config(&format!(
                "[[card]]\nid='sound'\nkind='sb16'\n[[card]]\nid='other'\n{card}\n"
            ))
            .unwrap_err();
            assert!(error.to_string().contains("card[sound]"), "{error}");
            assert!(error.to_string().contains("card[other]"), "{error}");
        }
    }

    #[test]
    fn sb16_word_channel_defaults_overrides_and_required_controller() {
        for channel in 5..=7 {
            let mut config =
                sb16_config(&format!("[[card]]\nkind='sb16'\ndma16={channel}\n")).unwrap();
            assert!(config
                .devices
                .iter()
                .any(|d| matches!(d.kind, DeviceKind::Sb16(cfg) if cfg.dma16 == channel)));
            config.board = assembly::BoardModel::Bare;
            config.devices.retain(|d| d.id != DeviceId::DmaWord);
            assert_eq!(
                config.validate(),
                Err(assembly::ConfigError::DmaControllerMissing {
                    requester: DeviceId::Sb16,
                    controller: DeviceId::DmaWord
                })
            );
        }
        assert!(sb16_config("[[card]]\nkind='uart-16550'\ndma16=5\n").is_err());
    }

    #[test]
    fn omitted_cpu_rate_uses_board_calibration_and_explicit_rate_wins() {
        struct Rom;
        impl ImageReader for Rom {
            fn read(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
                Ok(vec![0xF4; 0x8000])
            }
        }
        for (cpu, expected) in [("", (8, 13)), ("[cpu]\nrate='3/2'\n", (3, 2))] {
            let input = format!("schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='synthetic.bin'\n{cpu}");
            let document = crate::parse_toml("machine.toml", &input).unwrap();
            let origins = Origins::from_toml("machine.toml", &input).unwrap();
            let config = lower(
                &document,
                &origins,
                &mut Rom,
                assembly::AssemblyOptions::default(),
            )
            .unwrap();
            assert_eq!(
                (
                    config.clock.retired_per_pit_clock,
                    config.clock.retired_per_pit_clock_denominator
                ),
                expected
            );
        }
    }

    #[test]
    fn register_vga_lowering_owns_full_aperture_and_exact_ports() {
        let origins = Origins::new("machine.toml");
        let mut reader = NoImages;
        let mut context = context(&origins, &mut reader);
        let document =
            crate::parse_toml("machine.toml", "schema=1\n[[card]]\nkind='vga'\n").unwrap();
        let mut mmio = Vec::new();
        let device = context
            .card(0, &document.card.unwrap()[0], &mut mmio, &mut Vec::new())
            .unwrap();
        assert_eq!(device.kind, DeviceKind::VgaRegisters(assembly::VgaCfg {}));
        assert_eq!(device.ports, assembly::VgaCfg::port_windows());
        assert_eq!(mmio, vec![MmioWindow::new(0xA0000, 0x20000, DeviceId::Vga)]);
        assert_eq!((device.irq, device.dma), (None, None));
    }

    #[test]
    fn pin_diagnostic_retains_its_origin_independently_of_message_wording() {
        let mut origins = Origins::new("machine.toml");
        let pin_origin = Origin {
            file: "pins.toml".into(),
            line: Some(17),
        };
        origins.insert("card[1].drives[2].sha256", pin_origin.clone());
        let mut reader = NoImages;
        let context = context(&origins, &mut reader);
        let error = context.pin_error(
            "card[1].drives[2].image",
            "card[1].drives[2].sha256",
            "image digest differs",
        );
        assert_eq!(
            error.origin,
            Origin {
                file: "machine.toml".into(),
                line: None
            }
        );
        assert_eq!(error.field, "card[diskette].drives[unit=1].image");
        assert_eq!(error.pin_origin.as_deref(), Some(&pin_origin));
        assert_eq!(
            error.message,
            "image digest differs; pin set at pins.toml:17"
        );
        let ordinary = context.error("card[1].drives[2].image", "sha256 mismatch for anything");
        assert_eq!(ordinary.pin_origin, None);
        assert_eq!(ordinary.message, "sha256 mismatch for anything");
    }

    #[test]
    fn firmware_errors_retain_the_bios_image_origin_and_assembly_wording() {
        let mut origins = Origins::new("machine.toml");
        origins.insert(
            "firmware.bios",
            Origin {
                file: "machine.toml".into(),
                line: Some(4),
            },
        );
        let image_origin = Origin {
            file: "firmware.toml".into(),
            line: Some(9),
        };
        origins.insert("firmware.bios.image", image_origin.clone());
        origins.insert("firmware.bios.parts", image_origin.clone());
        for bios_parts in [false, true] {
            let source = if bios_parts {
                "firmware.bios.parts"
            } else {
                "firmware.bios.image"
            };
            let mut reader = NoImages;
            let context = context(&origins, &mut reader);
            for error in [
                ConfigError::MissingSystemBios { board: "ibm-5170" },
                ConfigError::FirmwareNotListed {
                    sha256: "0".repeat(64),
                    board: "ibm-5170",
                    known: "kiselev-xi8088-1.0.2 (open, vendor/kiselev-bios/fetch.sh)".into(),
                },
                ConfigError::FirmwareBoardMismatch {
                    firmware: "glabios-0.4.3-5160",
                    boards: "ibm-5160".into(),
                    board: "ibm-5170",
                },
                ConfigError::FirmwareNeedsCpu {
                    firmware: "seabios-1.17.0-isapc",
                    minimum: assembly::FirmwareCpu::I80386,
                    cpu: assembly::FirmwareCpu::I80286,
                },
                ConfigError::FirmwareRequiresDevice {
                    firmware: "bochs-rombios-legacy",
                    part: assembly::FirmwarePart::VideoRom,
                },
                ConfigError::BiosPlacement {
                    base: 0xF8000,
                    size: 24 * 1024,
                },
            ] {
                let diagnostics = AssemblyDiagnostics {
                    bios_parts,
                    origins: origins.clone(),
                    labels: context.labels.clone(),
                    devices: context.devices.clone(),
                    irq_fields: context.irq_fields.clone(),
                    drives: context.drives.clone(),
                    images: context.images.clone(),
                    image_paths: context.image_paths.clone(),
                };
                let diagnostic = diagnostics.map(assembly::AssemblyError::Config(error.clone()));
                assert_eq!(diagnostic.origin, image_origin);
                assert_eq!(diagnostic.field, source);
                assert_eq!(diagnostic.message, error.to_string());
                assert_eq!(diagnostic.validation.as_deref(), Some(&error));
                assert_eq!(
                    diagnostic.to_string(),
                    format!("firmware.toml:9: {source}: {error}")
                );
            }
        }
    }

    #[test]
    fn firmware_layout_errors_name_cpu_or_memory_and_the_related_bios() {
        let input = "schema=1\n[cpu]\nmodel='80286'\n[memory]\nconventional_kib=640\n[firmware.bios]\nimage='bios.bin'\n";
        let origins = Origins::from_toml("machine.toml", input).unwrap();
        let mut reader = NoImages;
        let context = context(&origins, &mut reader);
        let reset = ConfigError::FirmwareReset {
            board: "ibm-5170",
            reset: ResetSpec::Entry { cs: 0, ip: 0x500 },
        };
        let diagnostic = context.validation_error(reset.clone());
        assert_eq!(diagnostic.field, "cpu");
        assert_eq!(&diagnostic.origin, origins.get("cpu"));
        assert_eq!(diagnostic.message, reset.to_string());
        let ram = ConfigError::SystemBiosOverlapsRam {
            board: "ibm-5170",
            addr: 0xF8000,
        };
        let diagnostic = context.validation_error(ram.clone());
        assert_eq!(diagnostic.field, "memory");
        assert_eq!(&diagnostic.origin, origins.get("memory"));
        assert_eq!(
            diagnostic.message,
            format!(
                "{ram}; {}: firmware.bios.image",
                origins.get("firmware.bios.image")
            )
        );
    }

    #[test]
    fn word_port_errors_retain_their_device_origins() {
        let mut origins = Origins::new("machine.toml");
        origins.insert(
            "card[0]",
            Origin {
                file: "serial.toml".into(),
                line: Some(12),
            },
        );
        let mut reader = NoImages;
        let context = context(&origins, &mut reader);
        for error in [
            ConfigError::InvalidIdeResources {
                device: DeviceId::Uart,
            },
            ConfigError::InvalidIdePortWindows {
                device: DeviceId::Uart,
            },
            ConfigError::UnsupportedWordPort {
                device: DeviceId::Uart,
                port: 0x3F8,
            },
            ConfigError::UnalignedWordPortWindow {
                device: DeviceId::Uart,
                start: 0x3F9,
            },
            ConfigError::PortOverlapDecodeOrder {
                a: DeviceId::MasterPic,
                b: DeviceId::Uart,
                a_window: PortWindow::single(0x20),
                b_window: PortWindow::new(0x20, 0x27),
                port: 0x20,
            },
        ] {
            let diagnostic = context.validation_error(error.clone());
            assert_eq!(diagnostic.field, "card[serial]");
            assert_eq!(&diagnostic.origin, origins.get("card[0]"));
            assert_eq!(diagnostic.validation.as_deref(), Some(&error));
            if matches!(error, ConfigError::PortOverlapDecodeOrder { .. }) {
                assert!(diagnostic.message.contains("; machine.toml: board"));
            }
        }
    }

    #[test]
    fn drive_error_prefixes_match_assembly_display() {
        let mut origins = Origins::new("machine.toml");
        origins.insert(
            "card[1].drives[1]",
            Origin {
                file: "machine.toml".into(),
                line: Some(10),
            },
        );
        origins.insert(
            "card[1].drives[2]",
            Origin {
                file: "machine.toml".into(),
                line: Some(18),
            },
        );
        let mut reader = NoImages;
        let context = context(&origins, &mut reader);
        for (error, prefix, label) in [
            (ConfigError::InvalidAtaGeometry { drive_index: 2 }, "drive 2 ", "card[diskette].drives[unit=1] "),
            (ConfigError::AtaMissingMaster { drive_index: 2, controller: DeviceId::Ide }, "drive 2 ", "card[diskette].drives[unit=1] "),
            (ConfigError::Journal { drive_index: 2, message: "invalid journal path".into() }, "drive 2 ", "card[diskette].drives[unit=1] "),
            (ConfigError::UnsupportedFloppySectorSize { drive_index: 2, bytes_per_sector: 256 }, "drive 2 ", "card[diskette].drives[unit=1] "),
            (ConfigError::DriveControllerMissing { drive_index: 2, controller: DeviceId::Fdc }, "drive 2 ", "card[diskette].drives[unit=1] "),
            (ConfigError::DriveControllerKind { drive_index: 2, controller: DeviceId::Uart }, "drive 2 ", "card[diskette].drives[unit=1] "),
            (ConfigError::DriveUnitUnavailable { drive_index: 2, controller: DeviceId::Fdc, unit: 1, available: 1 }, "drive 2 ", "card[diskette].drives[unit=1] "),
            (ConfigError::ImageGeometryMismatch { drive_index: 2, expected: 512, actual: 7, drive_type: None }, "drive 2 ", "card[diskette].drives[unit=1] "),
            (ConfigError::DuplicateDriveUnit { a: 1, b: 2, controller: DeviceId::Fdc, unit: 1 }, "drives 1 and 2 ", "card[diskette].drives[unit=1] (machine.toml:10) and card[diskette].drives[unit=1] (machine.toml:18) "),
        ] {
            let original = error.to_string();
            assert!(original.starts_with(prefix), "assembly prefix changed: {original}");
            let suffix = original.strip_prefix(prefix).unwrap();
            let diagnostic = context.validation_error(error);
            assert!(diagnostic.message.starts_with(&format!("{label}{suffix}")), "{diagnostic}");
        }
    }

    #[test]
    fn mount_digest_error_names_the_drive_image_origin() {
        let mut origins = Origins::new("machine.toml");
        origins.insert(
            "card[1].drives[2].image",
            Origin {
                file: "media.toml".into(),
                line: Some(23),
            },
        );
        let mut reader = NoImages;
        let context = context(&origins, &mut reader);
        let expected = "ab".repeat(32);
        let actual = "cd".repeat(32);
        let error = ConfigError::DriveImageSha256 {
            drive_index: 2,
            expected: expected.clone(),
            actual: actual.clone(),
        };
        let diagnostic = context.validation_error(error.clone());
        assert_eq!(diagnostic.origin.file, Path::new("media.toml"));
        assert_eq!(diagnostic.origin.line, Some(23));
        assert_eq!(diagnostic.field, "card[diskette].drives[unit=1].image");
        assert_eq!(diagnostic.message, format!("card[diskette].drives[unit=1] image sha256 mismatch: expected {expected}, actual {actual}"));
        assert_eq!(diagnostic.validation.as_deref(), Some(&error));
        assert_eq!(diagnostic.to_string(), format!("media.toml:23: card[diskette].drives[unit=1].image: card[diskette].drives[unit=1] image sha256 mismatch: expected {expected}, actual {actual}"));
    }

    #[test]
    fn input_errors_retain_the_owning_source_and_assembly_message() {
        let mut origins = Origins::new("machine.toml");
        for (field, file, line) in [
            ("card[1].drives[2]", "drives.toml", 18),
            ("firmware.option_roms[0]", "firmware.toml", 9),
        ] {
            origins.insert(
                field,
                Origin {
                    file: file.into(),
                    line: Some(line),
                },
            );
        }
        let mut reader = NoImages;
        let mut context = context(&origins, &mut reader);
        context.images = vec!["firmware.bios".into(), "firmware.option_roms[0]".into()];
        let drive = ConfigError::DriveInput {
            drive_index: 2,
            path: "disk.retain-1.tmp".into(),
            message: "uses a reserved retention-temporary name".into(),
        };
        let diagnostic = context.validation_error(drive.clone());
        assert_eq!(diagnostic.origin, *origins.get("card[1].drives[2]"));
        assert_eq!(diagnostic.field, "card[diskette].drives[unit=1]");
        assert_eq!(
            diagnostic.message,
            drive
                .to_string()
                .replacen("drive 2 ", "card[diskette].drives[unit=1] ", 1)
        );
        assert_eq!(diagnostic.validation.as_deref(), Some(&drive));
        let image = ConfigError::ImageInput {
            image_index: 1,
            role: "firmware",
            path: "rom.retain-1.tmp".into(),
            message: "uses a reserved retention-temporary name".into(),
        };
        let diagnostic = context.validation_error(image.clone());
        assert_eq!(diagnostic.origin, *origins.get("firmware.option_roms[0]"));
        assert_eq!(diagnostic.field, "firmware.option_roms[0]");
        assert_eq!(diagnostic.message, image.to_string());
        assert_eq!(diagnostic.validation.as_deref(), Some(&image));
    }

    #[test]
    fn image_error_prefixes_match_assembly_display() {
        let origins = Origins::new("machine.toml");
        let mut reader = NoImages;
        let mut context = context(&origins, &mut reader);
        context.images = vec![
            "firmware.bios".into(),
            "firmware.option_roms[0]".into(),
            "firmware.option_roms[1]".into(),
        ];
        for (error, prefix, label) in [
            (
                ConfigError::ServicesOnRamLoad { image: 2 },
                "RAM load image 2 ",
                "firmware.option_roms[1] ",
            ),
            (
                ConfigError::ImageOverlapsMmio {
                    image_index: 2,
                    device: DeviceId::Uart,
                    addr: 0xC8800,
                },
                "image 2 ",
                "firmware.option_roms[1] ",
            ),
            (
                ConfigError::ImageOutsideRam {
                    image_index: 2,
                    addr: 0xC8800,
                },
                "ram-load image 2 ",
                "firmware.option_roms[1] ",
            ),
            (
                ConfigError::ImageOverlap {
                    a: 1,
                    b: 2,
                    addr: 0xC8800,
                },
                "images 1 and 2 ",
                "firmware.option_roms[0] and firmware.option_roms[1] ",
            ),
        ] {
            let original = error.to_string();
            assert!(
                original.starts_with(prefix),
                "assembly prefix changed: {original}"
            );
            let suffix = original.strip_prefix(prefix).unwrap();
            let diagnostic = context.validation_error(error);
            assert!(
                diagnostic.message.starts_with(&format!("{label}{suffix}")),
                "{diagnostic}"
            );
        }
    }
    #[test]
    fn firmware_service_mismatch_keeps_the_matched_image_origin() {
        let mut origins = Origins::new("machine.toml");
        origins.insert(
            "firmware.option_roms[0]",
            Origin {
                file: "roms.toml".into(),
                line: Some(12),
            },
        );
        let mut reader = NoImages;
        let mut context = context(&origins, &mut reader);
        context.images = vec!["firmware.bios".into(), "firmware.option_roms[0]".into()];
        let error = ConfigError::FirmwareProvidesMismatch {
            image_index: 1,
            firmware: "test-rom",
            declared: "[\"floppy\"]".into(),
            expected: "[\"fixed-disk\"]".into(),
        };
        let diagnostic = context.validation_error(error.clone());
        assert_eq!(diagnostic.field, "firmware.option_roms[0].provides");
        assert_eq!(diagnostic.origin.file, Path::new("roms.toml"));
        assert_eq!(diagnostic.origin.line, Some(12));
        assert_eq!(diagnostic.message, error.to_string());
    }
    fn catalogue_container_rom(
        bios: bool,
        offset: &str,
        kind: assembly::FirmwareKind,
    ) -> Result<LoadImage, LowerError> {
        struct Container;
        impl ImageReader for Container {
            fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
                Ok(vec![0; 32768])
            }
        }
        let entry = assembly::FirmwareEntry {
            sha256: Box::leak(format!("{:x}", Sha256::digest(vec![0; 32768])).into_boxed_str()),
            kind,
            rom_offset: 8192,
            ..assembly::FirmwareEntry::IBM_VGA_1987
        };
        let table = if bios {
            "[firmware.bios]"
        } else {
            "[[firmware.option_roms]]"
        };
        let input = format!("schema=1\n{table}\nimage='synthetic.bin'\n{offset}");
        let document = crate::parse_toml("machine.toml", &input).unwrap();
        let origins = Origins::from_toml("machine.toml", &input).unwrap();
        let mut reader = Container;
        let mut context = Context {
            origins: &origins,
            reader: &mut reader,
            labels: vec![],
            devices: vec![],
            irq_fields: vec![],
            drives: vec![],
            images: vec![],
            image_paths: vec![],
            options: assembly::AssemblyOptions::default(),
            bios_parts: false,
            metadata: false,
            rom_sizes: None,
        };
        let firmware = document.firmware.unwrap();
        let rom = if bios {
            firmware.bios.unwrap()
        } else {
            firmware.option_roms.unwrap().remove(0)
        };
        context.rom(
            if bios {
                "firmware.bios"
            } else {
                "firmware.option_roms[0]"
            },
            &rom,
            bios,
            &[entry],
        )
    }

    #[test]
    fn catalogue_container_offset_is_automatic_and_explicit_zero_is_refused() {
        let image =
            catalogue_container_rom(false, "base=0xC0000", assembly::FirmwareKind::VideoBios)
                .unwrap();
        assert_eq!(image.source.known_len(), Some(24576));
        assert!(matches!(
            image.source,
            ImageSource::RomContainer { offset: 8192, .. }
        ));
        let error = catalogue_container_rom(
            false,
            "base=0xC0000\noffset=0",
            assembly::FirmwareKind::VideoBios,
        )
        .expect_err("catalogue offset override must be refused");
        assert_eq!(error.field, "firmware.option_roms[0].offset");
        assert_eq!(
            error.message,
            "offset differs from the catalogue ROM layout"
        );
    }

    #[test]
    fn system_bios_never_uses_an_adapter_container_layout() {
        let error =
            catalogue_container_rom(true, "", assembly::FirmwareKind::VideoBios).unwrap_err();
        assert_eq!(error.field, "firmware.bios");
        assert_eq!(
            error.message,
            "video BIOS ibm-vga-bios-1987-32k cannot be used as the system BIOS"
        );
        let image = catalogue_container_rom(true, "", assembly::FirmwareKind::SystemBios).unwrap();
        assert!(matches!(image.source, ImageSource::Inline(_)));
        assert_eq!(image.source.known_len(), Some(32768));
        let image =
            catalogue_container_rom(false, "base=0xC8000", assembly::FirmwareKind::SystemBios)
                .unwrap();
        assert!(matches!(image.source, ImageSource::Inline(_)));
    }

    #[test]
    fn parts_diagnosis_precedes_pin_and_call_site_preserves_escape_and_catalogue_services() {
        struct Chips;
        impl ImageReader for Chips {
            fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
                Ok(vec![if path.ends_with("even") { 0x12 } else { 0x34 }; 4096])
            }
        }
        let bytes = [0x12, 0x34].repeat(4096);
        let chip = |label, number, lane, byte| assembly::KnownPart {
            label,
            part_number: number,
            lane,
            offset: 0,
            size: 4096,
            sha256: Box::leak(format!("{:x}", Sha256::digest(vec![byte; 4096])).into_boxed_str()),
        };
        let entry = assembly::FirmwareEntry {
            id: "synthetic-pinned-bios",
            size: 8192,
            sha256: Box::leak(format!("{:x}", Sha256::digest(&bytes)).into_boxed_str()),
            parts: Box::leak(
                vec![Box::leak(
                    vec![
                        chip("U27", "even-chip", assembly::Lane::Even, 0x12),
                        chip("U47", "odd-chip", assembly::Lane::Odd, 0x34),
                    ]
                    .into_boxed_slice(),
                ) as &'static [assembly::KnownPart]]
                .into_boxed_slice(),
            ),
            provides: &[
                assembly::FirmwareService::Floppy,
                assembly::FirmwareService::FixedDisk,
            ],
            ..assembly::KISELEV_XI8088
        };
        let input = format!(
            concat!(
                "schema=1\n[firmware.bios]\nparts=[",
                "{{image='even',lane='odd'}},{{image='odd',lane='even'}}]\n",
                "sha256='{}'\n"
            ),
            entry.sha256
        );
        let doc = crate::parse_toml("machines/test.toml", &input).unwrap();
        let origins = Origins::from_toml("machines/test.toml", &input).unwrap();
        let mut chips = Chips;
        let mut context = Context {
            origins: &origins,
            reader: &mut chips,
            labels: vec![],
            devices: vec![],
            irq_fields: Vec::new(),
            drives: vec![],
            images: vec![],
            image_paths: vec![],
            options: assembly::AssemblyOptions::default(),
            bios_parts: false,
            metadata: false,
            rom_sizes: None,
        };
        let mut rom = doc.firmware.unwrap().bios.unwrap();
        let error = context
            .rom("firmware.bios", &rom, true, &[entry])
            .unwrap_err();
        const SWAPPED_MESSAGE: &str = concat!(
            "firmware parts are swapped for synthetic-pinned-bios: ",
            "odd lane offset 0 holds U27 even-chip, ",
            "which belongs in the even lane at offset 0; ",
            "even lane offset 0 holds U47 odd-chip, ",
            "which belongs in the odd lane at offset 0"
        );
        assert_eq!(error.message, SWAPPED_MESSAGE);
        context.options.allow_unlisted_firmware = true;
        assert!(context
            .rom("firmware.bios", &rom, true, &[entry])
            .unwrap_err()
            .message
            .starts_with("sha256 mismatch for assembled BIOS:"));
        context.options.allow_unlisted_firmware = false;
        let parts = rom.parts.as_mut().unwrap();
        parts[0].lane = v1::Lane::Even;
        parts[1].lane = v1::Lane::Odd;
        let image = context.rom("firmware.bios", &rom, true, &[entry]).unwrap();
        assert_eq!(image.provides, entry.provides);
        rom.sha256 = Some(
            crate::parse_toml(
                "pin.toml",
                &format!("schema=1\n[firmware.bios]\nsha256='{}'\n", "00".repeat(32)),
            )
            .unwrap()
            .firmware
            .unwrap()
            .bios
            .unwrap()
            .sha256
            .unwrap(),
        );
        assert!(context
            .rom("firmware.bios", &rom, true, &[entry])
            .unwrap_err()
            .message
            .starts_with("sha256 mismatch for assembled BIOS:"));
        rom.sha256 = None;
        rom.provides = Some(vec![v1::FirmwareService::Floppy]);
        let image = context.rom("firmware.bios", &rom, true, &[entry]).unwrap();
        let mut config = assembly::bios(bytes.clone(), Some(vec![0; 1_474_560]));
        config.images[0] = image;
        let error = assembly::check_system_bios(
            &config,
            BoardModel::Ibm5170,
            &[entry],
            0xFE000,
            &bytes,
            assembly::AssemblyOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ConfigError::FirmwareProvidesMismatch { .. }
        ));
        rom.provides = Some(vec![
            v1::FirmwareService::Floppy,
            v1::FirmwareService::FixedDisk,
        ]);
        config.images[0] = context.rom("firmware.bios", &rom, true, &[entry]).unwrap();
        assembly::check_system_bios(
            &config,
            BoardModel::Ibm5170,
            &[entry],
            0xFE000,
            &bytes,
            assembly::AssemblyOptions::default(),
        )
        .unwrap();
    }
}

/// The reader of a context that only words a diagnostic after lowering.
struct NoImages;

impl ImageReader for NoImages {
    fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::other("a diagnostic reads no image"))
    }
}

/// The lowering's record of where each part of a config came from, kept to
/// word an assembly error of that config at its source after lowering.
#[derive(Clone)]
pub struct AssemblyDiagnostics {
    bios_parts: bool,
    origins: Origins,
    labels: Vec<Option<String>>,
    devices: Vec<(DeviceId, String)>,
    irq_fields: Vec<(DeviceId, u8, String)>,
    drives: Vec<DriveOrigin>,
    images: Vec<String>,
    image_paths: Vec<PathBuf>,
}

impl AssemblyDiagnostics {
    /// A context that knows no source: every error is worded at the document
    /// of `origins`.
    #[cfg(feature = "test-support")]
    pub fn without_sources(origins: Origins) -> Self {
        Self {
            bios_parts: false,
            origins,
            labels: Vec::new(),
            devices: Vec::new(),
            irq_fields: Vec::new(),
            drives: Vec::new(),
            images: Vec::new(),
            image_paths: Vec::new(),
        }
    }

    /// Give each assembled floppy drive the name its image had in the document.
    pub fn label_floppies(
        &self,
        config: &MachineConfig,
        assembled: &mut assembly::AssembledMachine,
    ) {
        for (origin, drive) in self.drives.iter().zip(&config.drives) {
            if let Some(name) = &origin.image_name {
                assembled.set_floppy_name(drive.controller, drive.unit, 0, name.clone());
            }
        }
    }

    /// An error at the source of drive `index` of the config.
    pub fn drive_error(&self, index: usize, message: impl Into<String>) -> LowerError {
        let mut reader = NoImages;
        let context = self.context(&mut reader);
        match self.drives.get(index) {
            Some(drive) => context.error(&drive.field, message),
            None => {
                let mut error = context.error("", format!("assembly reported drive index {index}, but only {} drive origins exist: {}", self.drives.len(), message.into()));
                error.field = "internal error".into();
                error
            }
        }
    }

    fn context<'a>(&'a self, reader: &'a mut NoImages) -> Context<'a, NoImages> {
        Context {
            origins: &self.origins,
            reader,
            labels: self.labels.clone(),
            devices: self.devices.clone(),
            irq_fields: self.irq_fields.clone(),
            drives: self.drives.clone(),
            images: self.images.clone(),
            image_paths: self.image_paths.clone(),
            options: assembly::AssemblyOptions::default(),
            bios_parts: self.bios_parts,
            metadata: false,
            rom_sizes: None,
        }
    }

    /// An error at the source of an NVRAM `field`.
    pub fn nvram_error(&self, field: &str, message: impl Into<String>) -> LowerError {
        let mut reader = NoImages;
        self.context(&mut reader).error(field, message)
    }

    /// An assembly error, worded at the source of what it names.
    pub fn map(&self, error: assembly::AssemblyError) -> LowerError {
        let mut reader = NoImages;
        let context = self.context(&mut reader);
        match error {
            assembly::AssemblyError::Config(error) => context.validation_error(error),
            assembly::AssemblyError::NvramJournal { source } => context.error(
                "board.rtc.nvram.journal",
                format!("journal mount: {source}"),
            ),
            assembly::AssemblyError::Journal {
                drive_index,
                source,
            } => self.drive_error(drive_index, format!("journal mount: {source}")),
            assembly::AssemblyError::AccessCoupling(error) => {
                context.error("board", format!("bus access coupling: {error}"))
            }
            assembly::AssemblyError::Cpu(error) => {
                context.error("memory", format!("cpu memory map: {error}"))
            }
        }
    }
}

#[cfg(test)]
mod reader_tests {
    //! The lowering asks its reader for everything of the host: a reader that
    //! answers from memory lowers a machine whose files exist nowhere.
    use std::cell::RefCell;

    use super::*;

    /// A folder no host has, so any host access would fail or find nothing.
    const ROOT: &str = "/nonexistent-machine-file-memory-reader";

    #[derive(Default)]
    struct Memory {
        images: BTreeMap<PathBuf, Vec<u8>>,
        reserved: Vec<PathBuf>,
        asked: RefCell<Vec<String>>,
    }

    impl Memory {
        fn with(mut self, name: &str, bytes: Vec<u8>) -> Self {
            self.images.insert(Path::new(ROOT).join(name), bytes);
            self
        }
        fn get(&self, question: &str, path: &Path) -> io::Result<&Vec<u8>> {
            self.asked
                .borrow_mut()
                .push(format!("{question} {}", path.display()));
            self.images
                .get(path)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not in memory"))
        }
    }

    impl ImageReader for Memory {
        fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
            self.get("read", path).cloned()
        }
        fn verify_journal_base(&mut self, path: &Path) -> io::Result<JournalBase> {
            let bytes = self.get("verify", path)?;
            Ok(JournalBase {
                length: bytes.len() as u64,
                sha256: Sha256::digest(bytes).into(),
            })
        }
        fn read_journal_base(&mut self, path: &Path) -> io::Result<Vec<u8>> {
            self.get("journal base", path).cloned()
        }
        fn image_length(&self, path: &Path) -> io::Result<u64> {
            self.get("length", path).map(|bytes| bytes.len() as u64)
        }
        fn is_reserved_retention_name(&self, path: &Path) -> bool {
            self.asked
                .borrow_mut()
                .push(format!("reserved {}", path.display()));
            self.reserved.iter().any(|reserved| reserved == path)
        }
        fn validate_journal_paths(
            &self,
            config: &MachineConfig,
            _: &Origins,
            images: &[PathBuf],
        ) -> Result<(), ConfigError> {
            self.asked.borrow_mut().push(format!(
                "journals of {} drives over {} images",
                config.drives.len(),
                images.len()
            ));
            Ok(())
        }
    }

    /// Journals live where the host can put them: `MachineConfig::validate`
    /// (assembly) still resolves a journal's folder on the host. Nothing is
    /// created there; every image and base comes from memory.
    fn journals() -> PathBuf {
        std::env::temp_dir()
    }

    fn machine() -> (MachineFile, Origins) {
        let path = Path::new(ROOT).join("machine.toml");
        let journals = journals();
        let text = format!(
            "schema=1\n[board]\nmodel='ibm-5170'\n[board.rtc]\nnvram={{image='cmos.bin',write='journal',journal='{0}/memory-reader-cmos.upfj'}}\n[firmware.bios]\nimage='bios.bin'\nprovides=['floppy']\n[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\ntype='360K'\nimage='base.img'\nwrite='journal'\njournal='{0}/memory-reader-disk.upfj'\n",
            journals.display()
        );
        (
            crate::parse_toml(&path, &text).unwrap(),
            Origins::from_toml(&path, &text).unwrap(),
        )
    }

    fn memory() -> Memory {
        Memory::default()
            .with("bios.bin", vec![0xF4; 0x8000])
            .with("cmos.bin", vec![0x5A; 64])
            .with("base.img", vec![0xE5; 368_640])
    }

    #[test]
    fn a_reader_that_answers_everything_lowers_without_the_host() {
        assert!(!Path::new(ROOT).exists());
        let (file, origins) = machine();
        let mut reader = memory();
        let config = lower(
            &file,
            &origins,
            &mut reader,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap();
        let root = Path::new(ROOT);
        let drive = &config.drives[0];
        assert_eq!(drive.image, ImageSource::File(root.join("base.img")));
        assert_eq!(
            drive.expected_sha256,
            Some(Sha256::digest(vec![0xE5; 368_640]).into())
        );
        let journal = journals().join("memory-reader-disk.upfj");
        assert!(matches!(&drive.write,
            assembly::WriteMode::Journal { path, .. } if *path == journal));
        assert!(!journal.exists());
        assert!(config.devices.iter().any(|device| matches!(&device.kind,
            DeviceKind::Rtc(assembly::CmosCfg { nvram: NvramSource::Journal { image, expected_sha256, .. }, .. })
                if *image == root.join("cmos.bin")
                    && *expected_sha256 == <[u8; 32]>::from(Sha256::digest([0x5A; 64])))));
        assert_eq!(config.images[0].base, 0xF8000);
        let asked = reader.asked.take();
        for question in [
            format!("read {ROOT}/bios.bin"),
            format!("journal base {ROOT}/cmos.bin"),
            format!("verify {ROOT}/base.img"),
            format!("reserved {ROOT}/base.img"),
            "journals of 1 drives over 3 images".into(),
        ] {
            assert!(asked.contains(&question), "{question} not in {asked:?}");
        }
    }

    #[test]
    fn the_reader_decides_which_names_are_reserved() {
        let (file, origins) = machine();
        let mut reader = memory();
        reader.reserved.push(Path::new(ROOT).join("base.img"));
        let error = lower(&file, &origins, &mut reader, Default::default()).unwrap_err();
        assert_eq!(error.field, "card[fdc-765].drives[unit=0].image");
        assert!(
            error
                .message
                .ends_with("uses a reserved retention-temporary name"),
            "{error}"
        );
    }

    #[test]
    fn metadata_mode_sizes_a_rom_through_the_reader() {
        let (file, origins) = machine();
        let mut reader = memory().with("bios.bin", vec![0xF4; 0x10000]);
        let draft = file.describe_with(&origins, &mut reader).unwrap();
        assert_eq!(draft.config.images[0].base, 0xF0000);
        assert_eq!(
            reader.asked.take(),
            [format!("length {ROOT}/bios.bin")],
            "metadata mode asks for lengths only"
        );
        let draft = file
            .describe_with(&origins, &mut Memory::default())
            .unwrap();
        assert_eq!(
            draft.config.images[0].base, MIB,
            "an unknown size stays pending"
        );
    }

    #[test]
    fn a_synthetic_reader_reserves_retention_names_by_name_alone() {
        struct Synthetic;
        impl ImageReader for Synthetic {
            fn read(&mut self, _: &Path) -> io::Result<Vec<u8>> {
                Err(io::Error::other("unused"))
            }
        }
        assert!(Synthetic.is_reserved_retention_name(Path::new("a/disk.retain-1.tmp")));
        assert!(!Synthetic.is_reserved_retention_name(Path::new("a.retain-1/disk.img")));
        assert!(Synthetic.image_length(Path::new("Cargo.toml")).is_err());
    }
}
