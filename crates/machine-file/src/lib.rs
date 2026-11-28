// SPDX-License-Identifier: AGPL-3.0-or-later
//! Parsed and validated machine descriptions for the IBM PC/AT emulator.
//!
//! TOML and YAML are decoded into schema-versioned layers described in
//! `backlog/docs/design/machine-files.md` § Schema and § Versioning, extends
//! and errors. Omitted fields remain omitted so the merger can distinguish
//! inheritance from explicit overrides. Recognised reserved names and
//! cycle-exact timing requests receive positioned diagnostics.
//!
//! Complete documents lower to `assembly::MachineConfig` with board/card
//! defaults and images checked with SHA-256 during lowering. Single volatile/readonly
//! images remain inline. Frontend loads retain verified disk descriptors through assembly.
//! Assembly owns hardware validation. Inheritance resolves before lowering.
//! This crate does not model hardware registers.
//!
//! The schema, its TOML decoder and the lowering are `pc-config`'s, the part
//! of a machine description that touches no host
//! (`backlog/docs/design/core-contract.md` §14.2.1); this crate re-exports
//! each of their names at its old path and adds what touches the host: the
//! YAML reader, `extends` and presets, journal path checks, firmware
//! discovery, the seam of the firmware-package lookup ([`FirmwarePackages`],
//! which the PC image provider implements over the image store;
//! core-contract.md §9 item 3), the host's image reader and the loaders.

use std::path::{Path, PathBuf};

pub mod firmware_discovery;
mod firmware_resolve;
mod floppy;
#[cfg(test)]
mod lower;
pub use floppy::FileMachineExt;
#[path = "assemble.rs"]
mod file_assembly;
mod mount;
mod mount_paths;
pub use file_assembly::*;
pub use mount::FileMount;
mod origin;
mod paths;
mod resolve;
mod source;
mod yaml;

pub use origin::YamlOrigins;
pub use pc_config::bindings;
pub use pc_config::{
    lower, lower_with_options, parse_toml, pc_names, schema, ImageReader, JournalBase, LowerError,
    MachineFile, Origin, Origins, ParseError,
};
pub use resolve::resolve;

use pc_config::{check_size, source_snippet, MAX_MACHINE_FILE_BYTES};

/// Parse one hardened YAML layer into the same schema used by TOML.
pub fn parse_yaml(path: impl AsRef<Path>, input: &str) -> Result<MachineFile, ParseError> {
    yaml::parse(path.as_ref(), input).map(|(document, _)| document)
}

/// Failure while loading a machine document or lowering its images.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Lower(#[from] LowerError),
}

/// Read a TOML or YAML machine file and return an assembly-validated configuration.
/// The given name selects the format and is retained in diagnostics. Image
/// paths use the target file's directory when the machine path is a symlink.
pub fn load(path: impl AsRef<Path>) -> Result<assembly::MachineConfig, LoadError> {
    load_with_options(path, assembly::AssemblyOptions::default())
}

/// Load a machine file with run-only firmware permissions and source diagnostics.
pub fn load_with_options(
    path: impl AsRef<Path>,
    options: assembly::AssemblyOptions,
) -> Result<assembly::MachineConfig, LoadError> {
    load_with_reader_and_options(path, &mut FileImageReader, options)
}

/// Load a machine file with a caller-provided image reader.
///
/// SHA-256 pins are checked against the reader's bytes during lowering, and
/// hardware validation remains mandatory. Volatile/readonly images retain those
/// bytes inline. Journal disks require `ImageReader::verify_journal_base`, which
/// synthetic readers refuse by default. This configuration-only API retains the
/// digest and path; `load_for_assembly` also retains verified descriptors so a
/// frontend launch does not hash or copy its disk bases again.
pub fn load_with_reader<R: ImageReader>(
    path: impl AsRef<Path>,
    reader: &mut R,
) -> Result<assembly::MachineConfig, LoadError> {
    load_with_reader_and_options(path, reader, assembly::AssemblyOptions::default())
}

/// Load with explicit image I/O and run-only firmware permissions.
pub fn load_with_reader_and_options<R: ImageReader>(
    path: impl AsRef<Path>,
    reader: &mut R,
    options: assembly::AssemblyOptions,
) -> Result<assembly::MachineConfig, LoadError> {
    let (document, origins) = resolve(path)?;
    Ok(lower_with_options(&document, &origins, reader, options)?)
}

/// A validated configuration retaining card identities and assembly diagnostics.
/// See machine-files § Frontends: catalogue checks need the resolved ROM bytes.
pub struct LoadedMachine {
    pub config: assembly::MachineConfig,
    /// Card ids in file order, paired with their assembly roles.
    pub cards: Vec<(String, assembly::DeviceId)>,
    /// The embedder's JIT backend for `cpu.engine = "jit"`; without one that
    /// engine fails to assemble. A host program sets it; files never do.
    pub jit_backend: Option<assembly::JitBackendFactory>,
    diagnostics: pc_config::AssemblyDiagnostics,
    options: assembly::AssemblyOptions,
    bases: std::collections::BTreeMap<PathBuf, journal_files::VerifiedBase>,
}

impl LoadedMachine {
    /// Check hardware and inspect journals without creating, recovering or closing them.
    /// Existing stable locks use the offline inspector's shared-lock path. With
    /// no lock entry, the same scanner runs without a lock and reports that fact.
    /// Guest drives assemble as volatile; no guest instruction is executed.
    pub fn check(&self) -> Result<Vec<String>, LowerError> {
        let mut config = self.config.clone();
        let mut notes = Vec::new();
        for (index, drive) in config.drives.iter_mut().enumerate() {
            if let Some(list) = &mut drive.media {
                for medium in &mut list.entries {
                    if let assembly::WriteMode::Journal { path, .. } = &medium.write {
                        let assembly::ImageSource::File(base) = &medium.image else {
                            return Err(self
                                .diagnostics
                                .drive_error(index, "internal error: journal base is not a file"));
                        };
                        let message = inspect_journal(base, path, drive.kind.image_bytes())
                            .map_err(|message| self.diagnostics.drive_error(index, message))?;
                        if let Some(message) = message {
                            notes.push(self.diagnostics.drive_error(index, message).to_string());
                        }
                        medium.write = assembly::WriteMode::Volatile;
                    }
                }
            }
            if let assembly::WriteMode::Journal { path, .. } = &drive.write {
                let assembly::ImageSource::File(base) = &drive.image else {
                    return Err(self
                        .diagnostics
                        .drive_error(index, "internal error: journal base is not a file"));
                };
                let message = inspect_journal(base, path, drive.kind.image_bytes())
                    .map_err(|message| self.diagnostics.drive_error(index, message))?;
                if let Some(message) = message {
                    notes.push(self.diagnostics.drive_error(index, message).to_string());
                }
                drive.write = assembly::WriteMode::Volatile;
            }
        }
        for device in &mut config.devices {
            let assembly::DeviceKind::Rtc(rtc) = &mut device.kind else {
                continue;
            };
            let assembly::NvramSource::Journal {
                image,
                expected_sha256,
                journal,
                ..
            } = &rtc.nvram
            else {
                continue;
            };
            use sha2::{Digest, Sha256};
            let error = |message| {
                self.diagnostics
                    .nvram_error("board.rtc.nvram.image", message)
            };
            let bytes = std::fs::read(image)
                .map_err(|e| error(format!("image {}: {e}", image.display())))?;
            let actual: [u8; 32] = Sha256::digest(&bytes).into();
            if &actual != expected_sha256 {
                return Err(error(format!(
                    "image sha256 mismatch: expected {:x}, actual {:x}",
                    sha2::digest::Output::<Sha256>::from(*expected_sha256),
                    sha2::digest::Output::<Sha256>::from(actual)
                )));
            }
            let message = inspect_journal(image, journal, 64).map_err(|message| {
                self.diagnostics
                    .nvram_error("board.rtc.nvram.journal", message)
            })?;
            if let Some(message) = message {
                notes.push(
                    self.diagnostics
                        .nvram_error("board.rtc.nvram.journal", message)
                        .to_string(),
                );
            }
            // Hardware validation executes no guest code and must never open a writer.
            rtc.nvram = assembly::NvramSource::Blank;
        }
        let assembled = assembly::assemble_mounted_with_jit(
            &config,
            &assembly::ObservationPolicy::default(),
            self.options,
            std::rc::Rc::new(FileMount::new()),
            self.jit_backend,
        )
        .map_err(|error| self.diagnostics.map(error))?;
        notes.extend(assembled.unlisted_firmware.iter().map(ToString::to_string));
        Ok(notes)
    }

    /// Assemble without running the guest, preserving defining-layer positions.
    pub fn assemble(
        &self,
        observation: &assembly::ObservationPolicy,
    ) -> Result<assembly::AssembledMachine, LowerError> {
        let mut assembled = crate::assemble_with_verified_bases_and_jit(
            &self.config,
            observation,
            self.options,
            &self.bases,
            self.jit_backend,
        )
        .map_err(|error| self.diagnostics.map(error))?;
        self.diagnostics
            .label_floppies(&self.config, &mut assembled);
        Ok(assembled)
    }
}

/// Read inline images and verify disk sources with bounded buffers: the
/// lowering's reader over the host file system.
pub struct FileImageReader;

impl ImageReader for FileImageReader {
    fn validate_machine(
        &self,
        config: &assembly::MachineConfig,
    ) -> Result<(), assembly::ConfigError> {
        crate::validate(config)
    }
    fn read(&mut self, path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn verify_journal_base(&mut self, path: &Path) -> std::io::Result<JournalBase> {
        journal_files::VerifiedBase::open(path).map(|base| journal_base(&base))
    }

    fn read_journal_base(&mut self, path: &Path) -> std::io::Result<Vec<u8>> {
        use std::io::Read;
        // Only the small CMOS store uses the byte-backed journal path.
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(513)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 512 {
            return Err(std::io::Error::other("NVRAM image exceeds 512 bytes"));
        }
        Ok(bytes)
    }

    fn image_length(&self, path: &Path) -> std::io::Result<u64> {
        std::fs::metadata(path).map(|metadata| metadata.len())
    }

    fn is_reserved_retention_name(&self, path: &Path) -> bool {
        paths::is_retention_input(path)
    }

    fn validate_journal_paths(
        &self,
        config: &assembly::MachineConfig,
        origins: &Origins,
        images: &[PathBuf],
    ) -> Result<(), assembly::ConfigError> {
        paths::validate_journals(config, origins, images)
    }
}

/// What the lowering needs of a verified base; the caller keeps the base.
pub fn journal_base(base: &journal_files::VerifiedBase) -> JournalBase {
    JournalBase {
        length: base.identity().base_length,
        sha256: base.identity().base_digest,
    }
}

/// The reader of [`Describe::describe`]: it sizes an image from the host's
/// file metadata and reads nothing.
struct MetadataReader;

impl ImageReader for MetadataReader {
    fn validate_machine(
        &self,
        config: &assembly::MachineConfig,
    ) -> Result<(), assembly::ConfigError> {
        crate::validate(config)
    }
    fn read(&mut self, _: &Path) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other(
            "metadata lowering does not read images",
        ))
    }

    fn image_length(&self, path: &Path) -> std::io::Result<u64> {
        FileImageReader.image_length(path)
    }
}

/// `MachineFile::describe`, which sizes ROMs from the host's file metadata:
/// `pc-config`'s `MachineFile::describe_with` over the host.
pub trait Describe {
    /// Describe resolved settings through the normal lowering rules without reading or
    /// hashing image bytes or opening journals. Unknown extents stay pending; known
    /// catalogue sizes or file metadata determine ROM placement. This draft cannot
    /// authorize execution. See `backlog/docs/design/machine-images.md` §6.
    fn describe(&self, origins: &Origins) -> Result<assembly::capabilities::Draft, LowerError>;
}

impl Describe for MachineFile {
    fn describe(&self, origins: &Origins) -> Result<assembly::capabilities::Draft, LowerError> {
        self.describe_with(origins, &mut MetadataReader)
    }
}

#[derive(Default)]
struct LaunchImageReader<'a> {
    bases: std::collections::BTreeMap<PathBuf, journal_files::VerifiedBase>,
    /// Verified decoded ROMs from compressed firmware packages.
    memory: Option<&'a std::collections::BTreeMap<PathBuf, Vec<u8>>>,
}

impl ImageReader for LaunchImageReader<'_> {
    fn validate_machine(
        &self,
        config: &assembly::MachineConfig,
    ) -> Result<(), assembly::ConfigError> {
        crate::validate(config)
    }
    fn read(&mut self, path: &Path) -> std::io::Result<Vec<u8>> {
        if let Some(bytes) = self.memory.and_then(|memory| memory.get(path)) {
            return Ok(bytes.clone());
        }
        FileImageReader.read(path)
    }
    fn read_journal_base(&mut self, path: &Path) -> std::io::Result<Vec<u8>> {
        FileImageReader.read_journal_base(path)
    }
    fn verify_journal_base(&mut self, path: &Path) -> std::io::Result<JournalBase> {
        use journal_files::SectorSource;
        if let Some(base) = self.bases.get(path) {
            base.check_unchanged()?;
            return Ok(journal_base(base));
        }
        let base = journal_files::VerifiedBase::open(path)?;
        let answer = journal_base(&base);
        self.bases.insert(path.into(), base);
        Ok(answer)
    }
    fn image_length(&self, path: &Path) -> std::io::Result<u64> {
        FileImageReader.image_length(path)
    }
    fn is_reserved_retention_name(&self, path: &Path) -> bool {
        FileImageReader.is_reserved_retention_name(path)
    }
    fn validate_journal_paths(
        &self,
        config: &assembly::MachineConfig,
        origins: &Origins,
        images: &[PathBuf],
    ) -> Result<(), assembly::ConfigError> {
        FileImageReader.validate_journal_paths(config, origins, images)
    }
}

/// Load for a frontend that needs card identities and positioned assembly errors.
/// Loading alone validates; `LoadedMachine::assemble` also checks ROM identity.
pub fn load_for_assembly(
    path: impl AsRef<Path>,
    options: assembly::AssemblyOptions,
) -> Result<LoadedMachine, LoadError> {
    load_for_assembly_with_rom_dirs(path, options, &[])
}

/// Load a launch configuration, resolving missing or mismatched pinned ROMs in
/// explicit host search directories. Correct declared paths win. The source
/// file is unchanged, and verified disk descriptors are retained as usual.
pub fn load_for_assembly_with_rom_dirs(
    path: impl AsRef<Path>,
    options: assembly::AssemblyOptions,
    roots: &[PathBuf],
) -> Result<LoadedMachine, LoadError> {
    use crate::firmware_discovery::{FirmwareRoot, RootOrigin};
    let roots = roots
        .iter()
        .map(|path| FirmwareRoot::new(path, RootOrigin::CommandLine))
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    let result = load_for_assembly_with_firmware_roots(path, options, &roots, &mut warnings);
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
    result
}

/// Load with labelled host ROM roots, reporting skipped roots and entries even
/// if subsequent machine validation fails. Package lookup is disabled; folder
/// discovery requires explicit roots.
pub fn load_for_assembly_with_firmware_roots(
    path: impl AsRef<Path>,
    options: assembly::AssemblyOptions,
    roots: &[crate::firmware_discovery::FirmwareRoot],
    warnings: &mut Vec<String>,
) -> Result<LoadedMachine, LoadError> {
    prepare_for_assembly_with_firmware_roots(path, roots, warnings)?.load(options)
}

/// The failure of a firmware-package lookup, reported against the machine
/// file being prepared.
pub type PackageError = Box<dyn std::error::Error + Send + Sync>;

/// Firmware packages a machine file's pinned ROMs may come from, asked before
/// the host's ROM folders are searched. The PC image provider implements it
/// over the image store's firmware packages; this crate knows no store.
pub trait FirmwarePackages {
    /// Point each ROM (or each chip of a ROM) of `document` whose pins a
    /// package holds at that package's bytes, and return the ids of the
    /// packages used, in first-use order. A ROM whose bytes cannot be named
    /// by a host path gets a path that is never created and its bytes in
    /// `images` under that path, so no decoded copy reaches the disk. An
    /// unavailable package source is a warning and a miss; an error fails the
    /// preparation.
    fn resolve(
        &self,
        document: &mut MachineFile,
        warnings: &mut Vec<String>,
        images: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
    ) -> Result<Vec<String>, PackageError>;
}

/// A resolved host document that can cross the worker boundary without moving
/// verified disk handles or guest execution state between threads. Loading it
/// revalidates pins in place and performs no directory discovery.
pub struct PreparedMachineFile {
    firmware_sources: Vec<String>,
    /// Decoded ROMs of compressed firmware packages, keyed by the never-created
    /// path the document names; see [`PreparedMachineFile::firmware_image`].
    firmware_images: std::collections::BTreeMap<PathBuf, Vec<u8>>,
    document: MachineFile,
    origins: Origins,
}

/// Locate firmware on a host worker before launch; no guest state is created.
pub fn prepare_for_assembly_with_firmware_roots(
    path: impl AsRef<Path>,
    roots: &[crate::firmware_discovery::FirmwareRoot],
    warnings: &mut Vec<String>,
) -> Result<PreparedMachineFile, LoadError> {
    prepare_for_assembly_with_firmware_packages(path, roots, warnings, None)
}

/// Prepare with a firmware-package lookup, or disable package lookup with
/// None. Pins a package holds are resolved first, then the remaining ones by
/// folder discovery.
pub fn prepare_for_assembly_with_firmware_packages(
    path: impl AsRef<Path>,
    roots: &[crate::firmware_discovery::FirmwareRoot],
    warnings: &mut Vec<String>,
    packages: Option<&dyn FirmwarePackages>,
) -> Result<PreparedMachineFile, LoadError> {
    let (mut document, origins) = resolve(&path)?;
    let roots = roots
        .iter()
        .filter_map(|root| match std::path::absolute(&root.path) {
            Ok(path) => Some(crate::firmware_discovery::FirmwareRoot::new(
                path,
                root.origin,
            )),
            Err(error) => {
                warnings.push(format!(
                    "{}: skipped ({})",
                    root.origin.label(),
                    error.kind()
                ));
                None
            }
        })
        .collect::<Vec<_>>();
    let mut firmware_images = std::collections::BTreeMap::new();
    let firmware_sources = match packages {
        Some(packages) => packages
            .resolve(&mut document, warnings, &mut firmware_images)
            .map_err(|e| ParseError::new(path.as_ref(), None, e.to_string(), None))?,
        None => Vec::new(),
    };
    firmware_resolve::resolve(&mut document, &origins, &roots, warnings, &firmware_images)?;
    Ok(PreparedMachineFile {
        document,
        origins,
        firmware_sources,
        firmware_images,
    })
}

impl PreparedMachineFile {
    /// The ids of the contributing immutable packages, as the lookup names
    /// them, in first-use order; provenance only.
    pub fn firmware_sources(&self) -> &[String] {
        &self.firmware_sources
    }

    /// The verified decoded bytes of a ROM that a compressed firmware package
    /// supplied. The document names such a ROM by a path that does not exist,
    /// so readers of the resolved document must look here first.
    pub fn firmware_image(&self, path: &Path) -> Option<&[u8]> {
        self.firmware_images.get(path).map(Vec::as_slice)
    }

    /// Borrow the resolved, discovered document and its unchanged source origins.
    pub fn resolved(&self) -> (&MachineFile, &Origins) {
        (&self.document, &self.origins)
    }

    /// Load the already-resolved document, verifying images without scanning roots.
    pub fn load(&self, options: assembly::AssemblyOptions) -> Result<LoadedMachine, LoadError> {
        let document = &self.document;
        let origins = &self.origins;
        let mut reader = LaunchImageReader {
            memory: Some(&self.firmware_images),
            ..LaunchImageReader::default()
        };
        let (config, diagnostics) =
            pc_config::lower_tracked(document, origins, &mut reader, options)?;
        let cards = document
            .card
            .iter()
            .flatten()
            .filter_map(|card| {
                use schema::v1::CardKind::*;
                let device = match card.kind? {
                    Mda => assembly::DeviceId::Mda,
                    VgaText | Vga => assembly::DeviceId::Vga,
                    Uart16550 => assembly::DeviceId::Uart,
                    Fdc765 => assembly::DeviceId::Fdc,
                    Ide => assembly::DeviceId::Ide,
                    Sb16 => assembly::DeviceId::Sb16,
                    LotechEms => assembly::DeviceId::LotechEms,
                };
                Some((resolve::card_id(card)?.to_owned(), device))
            })
            .collect();
        Ok(LoadedMachine {
            config,
            cards,
            jit_backend: None,
            diagnostics,
            options,
            bases: reader.bases,
        })
    }
}

fn inspect_journal(
    base: &Path,
    path: &Path,
    geometry_length: u64,
) -> Result<Option<String>, String> {
    use journal_files::offline::{Access, JournalTool, ToolError};
    journal_files::check_journal_path(path).map_err(|error| error.message)?;
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some("note: journal will be created".into()))
        }
        Err(error) => return Err(format!("journal {}: {error}", path.display())),
        Ok(_) => {}
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("journal {}: {error}", path.display()))?;
    let mut lock_path = canonical.as_os_str().to_os_string();
    lock_path.push(".lock");
    let no_lock = match std::fs::symlink_metadata(Path::new(&lock_path)) {
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(format!("journal lock: {error}")),
    };
    let report = if no_lock {
        // The offline tool may create a missing stable lock. A check cannot do
        // that, so inspect this snapshot with its scanner and disclose no lock.
        let base_bytes =
            std::fs::read(base).map_err(|error| format!("base {}: {error}", base.display()))?;
        let mut file = std::fs::File::open(&canonical)
            .map_err(|error| format!("journal {}: {error}", path.display()))?;
        journal_files::scan_reader(&mut file, &base_bytes, geometry_length)
            .map_err(|error| format!("journal {}: {error}", path.display()))?
            .report
    } else {
        let mut tool = match JournalTool::open(base, path, Access::Inspect) {
            Ok(tool) => tool,
            Err(ToolError::PathIo { source, .. })
                if source.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Ok(Some(
                    "warning: journal is in use and was not inspected".into(),
                ));
            }
            Err(error) => return Err(error.to_string()),
        };
        tool.verify().map_err(|error| error.to_string())?.report
    };
    if let journal_files::Status::Refused(refusal) = &report.status {
        return Err(format!(
            "journal {}: verification refused at {}: {}",
            path.display(),
            refusal.failure_offset,
            refusal.reason
        ));
    }
    let mut notes = Vec::new();
    if no_lock {
        notes.push("note: journal lock is absent; inspected without a lock");
    }
    if report.needs_recovery() {
        notes.push("warning: journal recovery will run on the next mount");
    }
    Ok((!notes.is_empty()).then(|| notes.join("; ")))
}

#[cfg(test)]
mod vga_tests {
    use super::*;

    #[test]
    fn register_vga_kind_parses_in_toml_and_yaml() {
        for document in [
            parse_toml("machine.toml", "schema=1\n[[card]]\nkind='vga'\n").unwrap(),
            parse_yaml("machine.yaml", "schema: 1\ncard:\n  - kind: vga\n").unwrap(),
        ] {
            let card = &document.card.as_ref().unwrap()[0];
            assert_eq!(card.kind, Some(schema::v1::CardKind::Vga));
        }
    }
}

/// Shared host fixtures, user-ROM discovery and skip policy for tests.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    mod temp_dir;
    pub use temp_dir::TempDir;

    /// Resolve host images through the production resolver without executing firmware.
    pub fn resolve_file_sources(
        config: &assembly::MachineConfig,
        options: assembly::AssemblyOptions,
    ) -> Result<assembly::MachineConfig, assembly::ConfigError> {
        assembly::test_support::resolve_sources_preverified(
            config,
            options,
            config.board.firmware_catalogue(),
            &crate::FileMount::new(),
        )
        .map(|(resolved, _)| resolved.into_owned())
    }

    /// Record the physical mappings of a config backed by host files.
    pub fn cpu_mapping_regions(
        config: &assembly::MachineConfig,
        options: assembly::AssemblyOptions,
    ) -> Result<Vec<(u64, u64, &'static str)>, assembly::AssemblyError> {
        assembly::test_support::cpu_mapping_regions_mounted(
            config,
            options,
            &crate::FileMount::new(),
        )
    }
    /// Build a CPU map of a config backed by host files for inspection and DMA tests.
    pub fn mapped_cpu(
        config: &assembly::MachineConfig,
        options: assembly::AssemblyOptions,
    ) -> Result<cpu::X86JitCpu, assembly::AssemblyError> {
        assembly::test_support::mapped_cpu_mounted(config, options, &crate::FileMount::new())
    }

    mod user_firmware;
    pub use user_firmware::{
        find_user_firmware, required_user_firmware, FoundUserFirmware, UserFirmwareError,
        UserFirmwareLayout,
    };
}

#[cfg(test)]
#[path = "assembly_tests.rs"]
mod assemble;
#[cfg(test)]
#[path = "persistence_tests.rs"]
mod persistence;

#[cfg(test)]
#[path = "run_tests.rs"]
mod run;
#[cfg(test)]
#[path = "validation_tests.rs"]
mod validate;

#[cfg(test)]
#[path = "firmware_tests.rs"]
mod firmware;

/// Resolve host inheritance and lower mixed path/package sources through one reader.
/// No package discovery is performed; facts are supplied explicitly by the caller.
pub fn load_with_bindings<R: ImageReader>(
    path: impl AsRef<Path>,
    uses: &[unemu_core_api::BindingUse],
    content: &dyn unemu_core_api::ContentSource,
    reader: &mut R,
    options: assembly::AssemblyOptions,
) -> Result<assembly::MachineConfig, unemu_core_api::Error> {
    let (document, origins) = resolve(path).map_err(|e| {
        unemu_core_api::Error::new(unemu_core_api::ErrorKind::Config, e.to_string())
    })?;
    bindings::lower_with_bindings(&document, &origins, uses, content, reader, options)
}
