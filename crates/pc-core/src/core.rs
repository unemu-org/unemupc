// SPDX-License-Identifier: AGPL-3.0-or-later
//! The PC factory over launch documents, content receipts and storage records.
//! See `backlog/docs/design/core-contract.md` §14.2.3. Machine-file callers
//! may still wrap their assembled PC with [`crate::PcMachine::from_assembled`].

use crate::machine::PC_FEATURES;
use unemu_core_api::{
    BindingFacts, BindingSlot, ConfigDocument, ConfigDocumentBuf, ContractVersion, Core,
    CoreDescription, CreateRequest, Error, ErrorKind, ExportFormat, ExportReport, ExportRequest,
    ExportScope, ExportSink, Machine, MediaDescription, Notice, OptionDescription,
    PlatformDescription, PreparedConfig, SlotId, Validated,
};

/// The platform id stored PC images name.
const PLATFORM: &str = pc_config::launch::PLATFORM;

#[path = "mount.rs"]
pub(crate) mod mount;

#[path = "export.rs"]
mod export;

static OPTIONS: [OptionDescription; 2] = [
    OptionDescription::new(
        "allow-unlisted-firmware",
        "Allow firmware outside the catalogue (true or false)",
    ),
    OptionDescription::new(
        "speaker-trace",
        "Record speaker diagnostics (true or false)",
    ),
];

static PLATFORMS: [PlatformDescription; 1] =
    [PlatformDescription::new(PLATFORM, "IBM PC compatible")
        .with_content_bindings(true)
        .with_features(PC_FEATURES)
        .with_config_types(&[pc_config::launch::MEDIA_TYPE])
        .with_options(&OPTIONS)
        .with_export_formats(&[
            ExportFormat::new("raw", "Raw disk image", "img", ExportScope::Slot),
            ExportFormat::new("vhd-fixed", "Fixed VHD", "vhd", ExportScope::Slot),
            ExportFormat::new("machine", "PC machine file", "toml", ExportScope::Machine),
        ])];

static NOTICES: [Notice; 2] = [
    Notice::new("Nuked-OPL3", devices::OPL3_NOTICE),
    Notice::new("Ultimate Oldschool PC Font Pack", crate::FONT_NOTICE),
];

static DESCRIPTION: CoreDescription = CoreDescription::new(
    ContractVersion::new(0, 6),
    "unemupc",
    "unemupc",
    env!("CARGO_PKG_VERSION"),
)
.with_platforms(&PLATFORMS)
.with_notices(&NOTICES);

/// The PC core's entry in a front end's list of cores.
#[derive(Clone, Copy, Debug, Default)]
pub struct PcCore;

impl Core for PcCore {
    fn description(&self) -> &CoreDescription {
        &DESCRIPTION
    }

    fn binding_slots(
        &self,
        platform: &str,
        config: ConfigDocument<'_>,
    ) -> Result<Vec<BindingSlot>, Error> {
        check_type(platform, config)?;
        let text = std::str::from_utf8(config.bytes)
            .map_err(|e| Error::new(ErrorKind::Config, e.to_string()))?;
        let launch = pc_config::launch::decode(config.bytes).map_err(config_error)?;
        let plan = pc_config::bindings::plan_envelope(text)?;
        if plan.slots().is_empty() {
            self.validate(platform, config)?;
        }
        let mut slots = plan.slots().to_vec();
        if let Some(cards) = launch
            .config
            .machine
            .get("card")
            .and_then(toml::Value::as_array)
        {
            for (i, card) in cards.iter().enumerate() {
                let Some(kind) = card.get("kind").and_then(toml::Value::as_str) else {
                    continue;
                };
                let controller = card.get("id").and_then(toml::Value::as_str).unwrap_or(kind);
                for (j, drive) in card
                    .get("drives")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let drive = drive
                        .as_table()
                        .ok_or_else(|| Error::new(ErrorKind::Config, "drive table required"))?;
                    let unit = pc_config::image::unit(kind, drive).map_err(config_error)?;
                    let Some(binding) = launch
                        .config
                        .slots
                        .iter()
                        .find(|slot| slot.controller == controller && slot.unit == unit)
                    else {
                        continue;
                    };
                    for id in &binding.catalogue {
                        if let Some(zero) = launch
                            .config
                            .media
                            .iter()
                            .find(|m| m.id == *id)
                            .and_then(|m| m.zero.as_ref())
                        {
                            slots.push(BindingSlot::zero_disk(
                                format!("card[{i}].drives[{j}]"),
                                id,
                                zero.size,
                            ));
                        }
                    }
                }
            }
        }
        slots.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(slots)
    }

    fn compose(
        &self,
        platform: &str,
        config: ConfigDocument<'_>,
        facts: BindingFacts<'_>,
    ) -> Result<PreparedConfig, Error> {
        check_type(platform, config)?;
        let launch = pc_config::launch::prepare(config.bytes, facts.uses)?;
        let required = pc_config::launch::check(&launch, assembly::AssemblyOptions::default())
            .map_err(config_error)?;
        let bytes = pc_config::launch::encode_portable(&launch).map_err(config_error)?;
        let content = mount::ContentCache::open(facts.content, &required)?;
        pc_config::launch::lower(&launch, &content, assembly::AssemblyOptions::default()).map_err(
            |error| {
                content.error().unwrap_or_else(|| {
                    let mut mapped = config_error(error);
                    if mapped.kind == ErrorKind::Config {
                        mapped.kind = ErrorKind::Unsupported;
                    }
                    mapped
                })
            },
        )?;
        Ok(PreparedConfig::new(
            ConfigDocumentBuf::new(pc_config::launch::MEDIA_TYPE, bytes),
            Validated::new(required),
        ))
    }

    fn create(&self, request: &CreateRequest<'_>) -> Result<Box<dyn Machine>, Error> {
        self.create_machine(request)
            .map(|machine| Box::new(machine) as Box<dyn Machine>)
    }

    fn describe_media(
        &self,
        platform: &str,
        config: ConfigDocument<'_>,
    ) -> Result<Vec<MediaDescription>, Error> {
        export::describe(platform, config)
    }

    fn export_slots(
        &self,
        platform: &str,
        config: ConfigDocument<'_>,
        format_id: &str,
    ) -> Result<Vec<SlotId>, Error> {
        export::slots(platform, config, format_id)
    }

    fn export(
        &self,
        platform: &str,
        config: ConfigDocument<'_>,
        request: ExportRequest<'_>,
        sink: &mut dyn ExportSink,
    ) -> Result<ExportReport, Error> {
        export::export(platform, config, request, sink)
    }

    fn validate(&self, platform: &str, config: ConfigDocument<'_>) -> Result<Validated, Error> {
        check_type(platform, config)?;
        let launch = pc_config::launch::decode(config.bytes).map_err(config_error)?;
        pc_config::launch::check(&launch, assembly::AssemblyOptions::default())
            .map(Validated::new)
            .map_err(config_error)
    }
}

impl PcCore {
    /// Build a typed PC from the same launch request as [`Core::create`].
    /// See `backlog/docs/design/core-contract.md` §14.2.3.
    pub fn create_machine(&self, request: &CreateRequest<'_>) -> Result<crate::PcMachine, Error> {
        check_type(request.platform, request.config)?;
        let mut options = assembly::AssemblyOptions::default();
        let mut trace = false;
        let mut seen = std::collections::BTreeSet::new();
        for &(key, value) in request.options {
            if !seen.insert(key) {
                return Err(Error::new(
                    ErrorKind::Usage,
                    format!("repeated PC option: {key}"),
                ));
            }
            let target = match key {
                "allow-unlisted-firmware" => &mut options.allow_unlisted_firmware,
                "speaker-trace" => &mut trace,
                _ => {
                    return Err(Error::new(
                        ErrorKind::Usage,
                        format!("unknown PC option: {key}"),
                    ))
                }
            };
            *target = value.parse().map_err(|_| {
                Error::new(
                    ErrorKind::Usage,
                    format!("PC option {key} needs true or false"),
                )
            })?;
        }
        let launch = pc_config::launch::decode(request.config.bytes).map_err(config_error)?;
        let required = pc_config::launch::check(&launch, options).map_err(config_error)?;
        let content = mount::ContentCache::open(request.content, &required)?;
        let lowered = pc_config::launch::lower(&launch, &content, options)
            .map_err(|error| content.error().unwrap_or_else(|| config_error(error)))?;
        let policy = assembly::ObservationPolicy {
            routes: [assembly::DeviceId::Mda, assembly::DeviceId::Vga]
                .into_iter()
                .filter(|id| {
                    lowered
                        .machine
                        .devices
                        .iter()
                        .any(|device| device.id == *id)
                })
                .map(|device| assembly::ObservationRequest {
                    device,
                    output: assembly::OutputKind::TextScanout,
                    byte_limit: 0,
                })
                .collect(),
            speaker_trace: trace,
            ..Default::default()
        };
        let mount = std::rc::Rc::new(mount::ContractMount::new(
            &lowered,
            content,
            request.storage.clone(),
        )?);
        let mut mounted = lowered.machine.clone();
        // The source verified each disk in full. The file assembler's byte
        // pins are redundant for non-journalled media too; keep the original
        // pins in the machine's public config and trust the receipt here.
        for drive in &mut mounted.drives {
            if !matches!(drive.write, assembly::WriteMode::Journal { .. }) {
                drive.expected_sha256 = None;
            }
            if let Some(list) = &mut drive.media {
                for medium in &mut list.entries {
                    if !matches!(medium.write, assembly::WriteMode::Journal { .. }) {
                        medium.expected_sha256 = None;
                    }
                }
            }
        }
        let assembled = assembly::assemble_mounted(&mounted, &policy, options, mount.clone())
            .map_err(|error| {
                mount.error().unwrap_or_else(|| {
                    Error::new(
                        match error {
                            assembly::AssemblyError::Journal { .. }
                            | assembly::AssemblyError::NvramJournal { .. } => ErrorKind::Storage,
                            _ => ErrorKind::Config,
                        },
                        error.to_string(),
                    )
                })
            })?;
        let mut machine = crate::PcMachine::from_assembled(&lowered.machine, assembled)?;
        machine.contract_failure = Some(mount.failure());
        Ok(machine)
    }
}

fn check_type(platform: &str, config: ConfigDocument<'_>) -> Result<(), Error> {
    if platform != PLATFORM || config.media_type != pc_config::launch::MEDIA_TYPE {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "the PC core needs an ibm-pc launch document",
        ));
    }
    Ok(())
}

fn config_error(error: pc_config::image::Error) -> Error {
    let kind = match &error {
        pc_config::image::Error::Lower(error) if error.pin_origin.is_some() => ErrorKind::Content,
        _ => ErrorKind::Config,
    };
    Error::new(kind, error.to_string())
}
