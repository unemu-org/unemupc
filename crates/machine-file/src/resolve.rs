// SPDX-License-Identifier: AGPL-3.0-or-later
//! Resolve machine-file inheritance without reading images or validating hardware.
//! See `backlog/docs/design/machine-files.md` § extends for the merge contract.

use crate::{schema::v1::*, LoadError, LowerError, MachineFile, Origins, ParseError};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const MAX_DEPTH: usize = 8;
const PRESETS: &[(&str, &str)] = &[(
    "ibm-5170",
    include_str!("../../../machines/presets/ibm-5170.toml"),
)];

fn error(origins: &Origins, field: &str, message: impl Into<String>) -> LowerError {
    LowerError {
        pin_origin: None,
        origin: origins.get(field).clone(),
        field: field.into(),
        message: message.into(),
        validation: None,
    }
}

fn parent_parse_error(cause: ParseError, caller: Option<&Origins>) -> LoadError {
    match caller {
        Some(origins) => error(
            origins,
            "extends",
            format!("parent {}: {}", cause.path().display(), cause.message()),
        )
        .into(),
        None => cause.into(),
    }
}

fn parse(path: &Path, input: &str, yaml: bool) -> Result<(MachineFile, Origins), ParseError> {
    let (document, mut origins) = if yaml {
        crate::yaml::parse(path, input)?
    } else {
        (
            crate::parse_toml(path, input)?,
            Origins::from_toml(path, input)?,
        )
    };
    crate::source::record(path, input, yaml, &mut origins)?;
    Ok((document, origins))
}

#[derive(Debug)]
struct Resolved {
    document: MachineFile,
    origins: Origins,
    chain: Vec<PathBuf>,
}

/// Resolve and merge layers, retaining each value's defining source and image base.
///
/// Completeness is checked before returning; no image I/O or hardware validation
/// occurs. Callers may inspect the result or pass it to [`crate::lower`].
pub fn resolve(path: impl AsRef<Path>) -> Result<(MachineFile, Origins), LoadError> {
    let path = path.as_ref();
    let resolved = Resolver {
        presets: PRESETS,
        stack: Vec::new(),
    }
    .visit(path, path, None, None)?;
    complete(&resolved)?;
    Ok((resolved.document, resolved.origins))
}

struct Resolver<'a> {
    presets: &'a [(&'a str, &'a str)],
    stack: Vec<(PathBuf, PathBuf)>,
}
impl Resolver<'_> {
    fn visit(
        &mut self,
        path: &Path,
        spelling: &Path,
        caller: Option<&Origins>,
        preset: Option<&str>,
    ) -> Result<Resolved, LoadError> {
        let yaml = if preset.is_some() {
            false
        } else {
            match path.extension().and_then(|s| s.to_str()) {
                Some("toml") => false,
                Some("yaml" | "yml") => true,
                _ => {
                    return Err(parent_parse_error(
                        ParseError::new(
                            path,
                            None,
                            "expected a machine file with extension .toml, .yaml or .yml",
                            None,
                        ),
                        caller,
                    ))
                }
            }
        };
        let identity = if preset.is_some() {
            path.to_path_buf()
        } else {
            std::fs::canonicalize(path).map_err(|e| match caller {
                Some(origins) => LoadError::Lower(error(
                    origins,
                    "extends",
                    if e.kind() == std::io::ErrorKind::NotFound {
                        format!("parent file not found: {}", path.display())
                    } else {
                        format!("cannot resolve parent {}: {e}", path.display())
                    },
                )),
                None => LoadError::Parse(ParseError::new(
                    path,
                    None,
                    format!("cannot resolve machine file: {e}"),
                    None,
                )),
            })?
        };
        let fallback = Origins::new(path);
        let source = caller.unwrap_or(&fallback);
        if self.stack.iter().any(|(id, _)| *id == identity) {
            let chain = self
                .stack
                .iter()
                .map(|(_, spelling)| spelling.as_path())
                .chain(std::iter::once(spelling))
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" → ");
            return Err(error(source, "extends", format!("inheritance cycle: {chain}")).into());
        }
        if self.stack.len() >= MAX_DEPTH {
            return Err(error(
                source,
                "extends",
                "inheritance depth exceeds limit of 8 layers",
            )
            .into());
        }
        let input = if let Some(name) = preset {
            self.presets
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, text)| (*text).to_owned())
                .ok_or_else(|| {
                    error(
                        source,
                        "extends",
                        format!(
                            "unknown preset `{name}`; known presets: {}",
                            self.presets
                                .iter()
                                .map(|(name, _)| *name)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    )
                })?
        } else {
            use std::io::Read;
            let read_error = |e| match caller {
                Some(origins) => LoadError::Lower(error(
                    origins,
                    "extends",
                    format!("cannot read parent {}: {e}", path.display()),
                )),
                None => LoadError::Parse(ParseError::new(
                    path,
                    None,
                    format!("cannot read machine file: {e}"),
                    None,
                )),
            };
            let mut input = Vec::new();
            std::fs::File::open(path)
                .and_then(|file| {
                    file.take((crate::MAX_MACHINE_FILE_BYTES + 1) as u64)
                        .read_to_end(&mut input)
                })
                .map_err(read_error)?;
            crate::check_size(path, input.len())
                .map_err(|cause| parent_parse_error(cause, caller))?;
            String::from_utf8(input).map_err(|_| {
                read_error(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "machine file is not valid UTF-8",
                ))
            })?
        };
        let (document, mut origins) = parse(path, &input, yaml)?;
        if preset.is_some() {
            check_preset(&document, &origins)?;
        }
        origins.set_image_file(identity.clone());
        validate_layer(&document, &origins)?;
        self.stack.push((identity.clone(), spelling.to_path_buf()));
        let result = if let Some(parent) = &document.extends {
            if !parent.starts_with("preset:") && Path::new(parent).is_absolute() {
                return Err(error(
                    &origins,
                    "extends",
                    "parent must be a relative path or preset:<name>",
                )
                .into());
            }
            let parent_spelling = Path::new(parent);
            let parent_preset = parent.strip_prefix("preset:");
            let parent = if parent_preset.is_some() {
                PathBuf::from(parent)
            } else {
                identity.parent().unwrap_or(Path::new(".")).join(parent)
            };
            let inherited = self.visit(&parent, parent_spelling, Some(&origins), parent_preset)?;
            merge(inherited, document, origins)?
        } else {
            let mut document = document;
            clear_remove(&mut document);
            Resolved {
                document,
                origins,
                chain: Vec::new(),
            }
        };
        self.stack.pop();
        let mut result = result;
        result.chain.push(spelling.to_path_buf());
        Ok(result)
    }
}

pub(crate) fn card_id(card: &Card) -> Option<&str> {
    card.id
        .as_deref()
        .or_else(|| card.kind.map(|kind| kind.descriptor().name()))
}
type DriveKey = (Option<u8>, Option<Channel>);

fn drive_key(drive: &Drive) -> DriveKey {
    (drive.unit, drive.channel)
}
pub(crate) fn drive_label(drive: &Drive) -> String {
    match drive_key(drive) {
        (Some(unit), _) => format!("unit={unit}"),
        (_, Some(Channel::Master)) => "channel=master".into(),
        (_, Some(Channel::Slave)) => "channel=slave".into(),
        _ => "missing key".into(),
    }
}

fn validate_layer(document: &MachineFile, origins: &Origins) -> Result<(), LowerError> {
    pc_config::bindings::validate_sources(document, origins)?;
    if document
        .cpu
        .as_ref()
        .is_some_and(|cpu| cpu.rate.is_some() && cpu.clock_mhz.is_some())
    {
        return Err(error(
            origins,
            "cpu.clock_mhz",
            "cpu.rate and cpu.clock_mhz are mutually exclusive",
        ));
    }
    if let Some(f) = &document.firmware {
        for (field, rom) in f
            .bios
            .iter()
            .map(|r| ("firmware.bios".to_owned(), r))
            .chain(
                f.option_roms
                    .iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, r)| (format!("firmware.option_roms[{i}]"), r)),
            )
        {
            if rom.image.is_some() && rom.parts.is_some() {
                return Err(error(
                    origins,
                    &field,
                    "image and parts are mutually exclusive",
                ));
            }
            if field != "firmware.bios" && rom.parts.is_some() {
                return Err(error(
                    origins,
                    &format!("{field}.parts"),
                    "option ROMs cannot use parts; only firmware.bios supports parts",
                ));
            }
        }
    }
    let mut ids = BTreeMap::new();
    for (i, card) in document.card.iter().flatten().enumerate() {
        let field = format!("card[{i}]");
        if let Some(id) = card_id(card) {
            if let Some(first) = ids.insert(id, field.clone()) {
                return Err(error(
                    origins,
                    &field,
                    format!(
                        "duplicate card id `{id}`; first defined at {}",
                        origins.get(&first)
                    ),
                ));
            }
        }
        if card.remove.is_some() && document.extends.is_none() {
            return Err(error(
                origins,
                &format!("{field}.remove"),
                "remove is only accepted in an extends layer",
            ));
        }
        if card.kind.is_none() && document.extends.is_none() {
            return Err(error(
                origins,
                &format!("{field}.kind"),
                "kind is required for a new card",
            ));
        }
        if card.remove == Some(true)
            && (card.kind.is_some()
                || card.controller_model.is_some()
                || card.frame.is_some()
                || card.size_kib.is_some()
                || card.port.is_some()
                || card.base.is_some()
                || card.dma8.is_some()
                || card.dma16.is_some()
                || card.irq.is_some()
                || card.dma.is_some()
                || card.drives.is_some())
        {
            return Err(error(
                origins,
                &field,
                "a removed card accepts only id and remove",
            ));
        }
        let mut keys: Vec<(DriveKey, String)> = Vec::new();
        for (j, drive) in card.drives.iter().flatten().enumerate() {
            let field = format!("{field}.drives[{j}]");
            if drive.remove.is_some() && document.extends.is_none() {
                return Err(error(
                    origins,
                    &format!("{field}.remove"),
                    "remove is only accepted in an extends layer",
                ));
            }
            if drive.unit.is_some() && drive.channel.is_some() {
                return Err(error(
                    origins,
                    &field,
                    "a drive must have exactly one key: unit or channel",
                ));
            }
            if drive.unit.is_some() || drive.channel.is_some() {
                if let Some((_, first)) = keys.iter().find(|(key, _)| *key == drive_key(drive)) {
                    return Err(error(
                        origins,
                        &field,
                        format!(
                            "duplicate drive key {}; first defined at {}",
                            drive_label(drive),
                            origins.get(first)
                        ),
                    ));
                }
                keys.push((drive_key(drive), field.clone()));
            }
            if drive.remove == Some(true)
                && (drive.drive_type.is_some()
                    || drive.image.is_some()
                    || drive.sha256.is_some()
                    || drive.geometry.is_some()
                    || drive.write.is_some()
                    || drive.journal.is_some()
                    || drive.keep_generations.is_some()
                    || drive.media.is_some()
                    || drive.empty.is_some()
                    || drive.disk.is_some()
                    || drive.inserted.is_some())
            {
                return Err(error(
                    origins,
                    &field,
                    "a removed drive accepts only its key and remove",
                ));
            }
        }
    }
    Ok(())
}

fn clear_remove(document: &mut MachineFile) {
    document.extends = None;
    for card in document.card.iter_mut().flatten() {
        card.remove = None;
        for drive in card.drives.iter_mut().flatten() {
            drive.remove = None;
        }
    }
}

struct Merger<'a> {
    out: &'a mut Origins,
    source: &'a Origins,
    to: String,
    from: String,
}
impl Merger<'_> {
    fn scalar<T>(&mut self, parent: &mut Option<T>, child: Option<T>, name: &str) {
        if child.is_some() {
            *parent = child;
            self.out
                .copy_subtree(self.source, &join(&self.from, name), &join(&self.to, name));
        }
    }
    fn image(
        &mut self,
        image: &mut Option<String>,
        pin: &mut Option<Sha256>,
        child_image: Option<String>,
        child_pin: Option<Sha256>,
    ) {
        if child_image.is_some() {
            *pin = None;
            self.out.remove_subtree(&join(&self.to, "sha256"));
        }
        self.scalar(image, child_image, "image");
        self.scalar(pin, child_pin, "sha256");
    }

    fn table<T: Merge>(&mut self, parent: &mut Option<T>, child: Option<T>, name: &str) {
        if let Some(child) = child {
            let to = join(&self.to, name);
            let from = join(&self.from, name);
            if let Some(parent) = parent {
                self.out.insert(&to, self.source.get(&from).clone());
                parent.merge(
                    child,
                    &mut Merger {
                        out: self.out,
                        source: self.source,
                        to,
                        from,
                    },
                );
            } else {
                *parent = Some(child);
                self.out.copy_subtree(self.source, &from, &to);
            }
        }
    }
}
fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.into()
    } else {
        format!("{prefix}.{name}")
    }
}
trait Merge {
    fn merge(&mut self, child: Self, m: &mut Merger<'_>);
}
macro_rules! table {
    ($type:ty, [$($scalar:ident),*], [$($nested:ident),*]) => {
        impl Merge for $type {
            fn merge(&mut self, child: Self, m: &mut Merger<'_>) {
                $(m.scalar(&mut self.$scalar, child.$scalar, stringify!($scalar));)*
                $(m.table(&mut self.$nested, child.$nested, stringify!($nested));)*
            }
        }
    };
}
table!(Board, [model, post_port, mouse, display_switch], [rtc]);
table!(Rtc, [start], [nvram]);
impl Merge for Cpu {
    fn merge(&mut self, child: Self, m: &mut Merger<'_>) {
        // A child selecting a different clock form replaces the inherited
        // selection. Two forms in the same layer remain a positioned error.
        if child.clock_mhz.is_some() && child.rate.is_none() {
            self.rate = None;
            m.out.remove_subtree(&join(&m.to, "rate"));
        }
        if child.rate.is_some() && child.clock_mhz.is_none() {
            self.clock_mhz = None;
            m.out.remove_subtree(&join(&m.to, "clock_mhz"));
        }
        m.scalar(&mut self.model, child.model, "model");
        m.scalar(&mut self.rate, child.rate, "rate");
        m.scalar(&mut self.clock_mhz, child.clock_mhz, "clock_mhz");
        m.scalar(&mut self.turbo, child.turbo, "turbo");
        m.scalar(&mut self.fpu, child.fpu, "fpu");
        m.scalar(&mut self.engine, child.engine, "engine");
    }
}
table!(Memory, [conventional_kib, extended_kib], []);
table!(Firmware, [option_roms], [bios]);
impl Merge for Rom {
    fn merge(&mut self, child: Self, m: &mut Merger<'_>) {
        if child.image.is_some()
            || child.parts.is_some()
            || child.use_id.is_some()
            || child.rom.is_some()
        {
            self.provides = None;
            self.offset = None;
            m.out.remove_subtree(&join(&m.to, "offset"));
            m.out.remove_subtree(&join(&m.to, "provides"));
        }
        if child.image.is_some()
            || child.parts.is_some()
            || child.use_id.is_some()
            || child.rom.is_some()
        {
            self.use_id = None;
            self.rom = None;
            self.role = None;
            self.size = None;
            self.sha256 = None;
            for name in ["use", "rom", "role", "size", "sha256"] {
                m.out.remove_subtree(&join(&m.to, name));
            }
        }
        if child.use_id.is_some() || child.rom.is_some() {
            self.image = None;
            self.parts = None;
            for name in ["image", "parts"] {
                m.out.remove_subtree(&join(&m.to, name));
            }
        }
        if child.parts.is_some() {
            self.image = None;
            self.sha256 = None;
            m.out.remove_subtree(&join(&m.to, "image"));
            m.out.remove_subtree(&join(&m.to, "sha256"));
        } else if child.image.is_some() {
            self.parts = None;
            m.out.remove_subtree(&join(&m.to, "parts"));
        }
        m.scalar(&mut self.use_id, child.use_id, "use");
        m.scalar(&mut self.rom, child.rom, "rom");
        m.scalar(&mut self.role, child.role, "role");
        m.scalar(&mut self.size, child.size, "size");
        m.scalar(&mut self.parts, child.parts, "parts");
        m.image(&mut self.image, &mut self.sha256, child.image, child.sha256);
        m.scalar(&mut self.offset, child.offset, "offset");
        m.scalar(&mut self.base, child.base, "base");
        m.scalar(&mut self.provides, child.provides, "provides");
    }
}

// Keep journal override units in sync with the Drive merge below.
impl Merge for NvramImage {
    fn merge(&mut self, child: Self, m: &mut Merger<'_>) {
        if child.image.is_some() || child.write.is_some_and(|mode| mode != WriteMode::Journal) {
            // A journal belongs to its base and is irrelevant when writes stop using it.
            self.journal = None;
            self.keep_generations = None;
            for name in ["journal", "keep_generations"] {
                m.out.remove_subtree(&join(&m.to, name));
            }
        }
        if child.image.is_some() && self.write == Some(WriteMode::Journal) {
            self.write = None;
            m.out.remove_subtree(&join(&m.to, "write"));
        }
        m.image(&mut self.image, &mut self.sha256, child.image, child.sha256);
        m.scalar(&mut self.write, child.write, "write");
        m.scalar(&mut self.journal, child.journal, "journal");
        m.scalar(
            &mut self.keep_generations,
            child.keep_generations,
            "keep_generations",
        );
    }
}
table!(Chs, [cylinders, heads, sectors], []);
impl Merge for Nvram {
    fn merge(&mut self, child: Self, m: &mut Merger<'_>) {
        if let (Self::Image(parent), Self::Image(child)) = (&mut *self, &child) {
            parent.merge(child.clone(), m);
        } else {
            *self = child;
            m.out.copy_subtree(m.source, &m.from, &m.to);
        }
    }
}
impl Merge for Geometry {
    fn merge(&mut self, child: Self, m: &mut Merger<'_>) {
        let (Self::Chs(parent), Self::Chs(child)) = (self, child);
        parent.merge(child, m);
    }
}
impl Merge for Drive {
    fn merge(&mut self, child: Self, m: &mut Merger<'_>) {
        let new_source = child.image.is_some()
            || child.media.is_some()
            || child.disk.is_some()
            || child.inserted.is_some();
        if new_source {
            self.disk = None;
            self.inserted = None;
            for name in ["disk", "inserted"] {
                m.out.remove_subtree(&join(&m.to, name));
            }
        }
        if child.disk.is_some() || child.inserted.is_some() {
            self.image = None;
            self.media = None;
            self.empty = None;
            self.sha256 = None;
            for name in ["image", "media", "empty", "sha256"] {
                m.out.remove_subtree(&join(&m.to, name));
            }
        }
        m.scalar(&mut self.disk, child.disk, "disk");
        m.scalar(&mut self.inserted, child.inserted, "inserted");
        if child.media.is_some() {
            self.empty = None;
            self.image = None;
            self.sha256 = None;
            self.write = None;
            self.journal = None;
            self.keep_generations = None;
            for name in [
                "empty",
                "image",
                "sha256",
                "write",
                "journal",
                "keep_generations",
            ] {
                m.out.remove_subtree(&join(&m.to, name));
            }
        } else if child.image.is_some() {
            self.media = None;
            self.empty = None;
            for name in ["media", "empty"] {
                m.out.remove_subtree(&join(&m.to, name));
            }
        }
        m.scalar(&mut self.media, child.media, "media");
        m.scalar(&mut self.empty, child.empty, "empty");
        m.scalar(&mut self.unit, child.unit, "unit");
        m.scalar(&mut self.channel, child.channel, "channel");
        m.scalar(&mut self.drive_type, child.drive_type, "type");
        if new_source || child.write.is_some_and(|mode| mode != WriteMode::Journal) {
            // A journal belongs to its base and is irrelevant when writes stop using it.
            self.journal = None;
            self.keep_generations = None;
            for name in ["journal", "keep_generations"] {
                m.out.remove_subtree(&join(&m.to, name));
            }
        }
        if new_source && self.write == Some(WriteMode::Journal) {
            self.write = None;
            m.out.remove_subtree(&join(&m.to, "write"));
        }
        m.image(&mut self.image, &mut self.sha256, child.image, child.sha256);
        m.scalar(&mut self.write, child.write, "write");
        m.scalar(&mut self.journal, child.journal, "journal");
        m.scalar(
            &mut self.keep_generations,
            child.keep_generations,
            "keep_generations",
        );
        m.table(&mut self.geometry, child.geometry, "geometry");
    }
}

fn compact<T>(entries: Vec<Option<T>>, origins: &mut Origins, prefix: &str) -> Vec<T> {
    let mut indices = Vec::with_capacity(entries.len());
    let mut values = Vec::new();
    for entry in entries {
        indices.push(entry.as_ref().map(|_| values.len()));
        if let Some(value) = entry {
            values.push(value);
        }
    }
    origins.remap_entries(prefix, &indices);
    values
}

fn merge(
    mut parent: Resolved,
    mut child: MachineFile,
    origins: Origins,
) -> Result<Resolved, LowerError> {
    parent.origins.use_document(&origins);
    let mut m = Merger {
        out: &mut parent.origins,
        source: &origins,
        to: String::new(),
        from: String::new(),
    };
    m.table(&mut parent.document.board, child.board.take(), "board");
    m.table(&mut parent.document.cpu, child.cpu.take(), "cpu");
    m.table(&mut parent.document.memory, child.memory.take(), "memory");
    m.table(
        &mut parent.document.firmware,
        child.firmware.take(),
        "firmware",
    );
    m.scalar(&mut parent.document.disks, child.disks.take(), "disks");
    parent.document.name = child.name.take();
    parent.document.description = child.description.take();
    parent.document.schema = child.schema;
    for name in ["name", "description", "schema"] {
        parent.origins.copy_subtree(&origins, name, name);
    }
    let mut cards = parent
        .document
        .card
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    let mut ids = cards
        .iter()
        .enumerate()
        .filter_map(|(i, card)| card.as_ref().and_then(card_id).map(|id| (id.to_owned(), i)))
        .collect::<BTreeMap<_, _>>();
    for (i, mut card) in child
        .card
        .take()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let from = format!("card[{i}]");
        let Some(id) = card_id(&card).map(str::to_owned) else {
            return Err(error(
                &origins,
                &format!("{from}.kind"),
                "kind is required for a new card",
            ));
        };
        let existing = ids.get(&id).copied();
        if card.remove == Some(true) {
            let Some(index) = existing else {
                return Err(error(
                    &origins,
                    &format!("{from}.remove"),
                    format!("cannot remove card `{id}`: parent does not define it"),
                ));
            };
            cards[index] = None;
            ids.remove(&id);
            continue;
        }
        card.remove = None;
        if let Some(index) = existing {
            let to = format!("card[{index}]");
            let Some(inherited) = cards[index].as_mut() else {
                continue;
            };
            parent.origins.insert(&to, origins.get(&from).clone());
            if card.kind.is_some() && card.kind != inherited.kind {
                return Err(error(
                    &origins,
                    &format!("{from}.kind"),
                    format!("card `{id}` cannot change inherited kind"),
                ));
            }
            let mut m = Merger {
                out: &mut parent.origins,
                source: &origins,
                to: to.clone(),
                from: from.clone(),
            };
            m.scalar(&mut inherited.id, card.id, "id");
            m.scalar(&mut inherited.kind, card.kind, "kind");
            m.scalar(
                &mut inherited.controller_model,
                card.controller_model,
                "controller_model",
            );
            m.scalar(&mut inherited.port, card.port, "port");
            m.scalar(&mut inherited.frame, card.frame, "frame");
            m.scalar(&mut inherited.size_kib, card.size_kib, "size_kib");
            m.scalar(&mut inherited.mouse, card.mouse, "mouse");
            m.scalar(&mut inherited.base, card.base, "base");
            m.scalar(&mut inherited.dma8, card.dma8, "dma8");
            m.scalar(&mut inherited.dma16, card.dma16, "dma16");
            m.scalar(&mut inherited.irq, card.irq, "irq");
            m.scalar(&mut inherited.dma, card.dma, "dma");
            merge_drives(inherited, card.drives, &mut m)?;
        } else {
            if card.kind.is_none() {
                return Err(error(
                    &origins,
                    &format!("{from}.kind"),
                    "kind is required for a new card",
                ));
            }
            reject_new_drive_removal(&card, &origins, &from)?;
            for drive in card.drives.iter_mut().flatten() {
                drive.remove = None;
            }
            parent
                .origins
                .copy_subtree(&origins, &from, &format!("card[{}]", cards.len()));
            ids.insert(id, cards.len());
            cards.push(Some(card));
        }
    }
    parent.document.card = Some(compact(cards, &mut parent.origins, "card"));
    parent.document.extends = None;
    Ok(parent)
}

fn reject_new_drive_removal(card: &Card, origins: &Origins, field: &str) -> Result<(), LowerError> {
    for (i, drive) in card.drives.iter().flatten().enumerate() {
        if drive.remove == Some(true) {
            return Err(error(
                origins,
                &format!("{field}.drives[{i}].remove"),
                format!(
                    "cannot remove drive {}: parent does not define it",
                    drive_label(drive)
                ),
            ));
        }
    }
    Ok(())
}

fn merge_drives(
    card: &mut Card,
    child: Option<Vec<Drive>>,
    m: &mut Merger<'_>,
) -> Result<(), LowerError> {
    let Some(child) = child else {
        return Ok(());
    };
    let mut drives = card
        .drives
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    let mut keys = drives
        .iter()
        .enumerate()
        .filter_map(|(i, d)| d.as_ref().map(|d| (drive_label(d), i)))
        .collect::<BTreeMap<_, _>>();
    for (i, mut drive) in child.into_iter().enumerate() {
        let from = format!("{}.drives[{i}]", m.from);
        let key = drive_label(&drive);
        let existing = keys
            .get(&key)
            .copied()
            .filter(|_| drive_key(&drive) != (None, None));
        if drive.remove == Some(true) {
            let Some(index) = existing else {
                return Err(error(
                    m.source,
                    &format!("{from}.remove"),
                    format!("cannot remove drive {key}: parent does not define it"),
                ));
            };
            drives[index] = None;
            keys.remove(&key);
        } else {
            drive.remove = None;
            if let Some(index) = existing {
                m.out.insert(
                    format!("{}.drives[{index}]", m.to),
                    m.source.get(&from).clone(),
                );
                if let Some(parent) = drives[index].as_mut() {
                    parent.merge(
                        drive,
                        &mut Merger {
                            out: m.out,
                            source: m.source,
                            to: format!("{}.drives[{index}]", m.to),
                            from,
                        },
                    );
                }
            } else {
                m.out.copy_subtree(
                    m.source,
                    &from,
                    &format!("{}.drives[{}]", m.to, drives.len()),
                );
                keys.insert(key, drives.len());
                drives.push(Some(drive));
            }
        }
    }
    card.drives = Some(compact(drives, m.out, &format!("{}.drives", m.to)));
    Ok(())
}

fn check_preset(document: &MachineFile, origins: &Origins) -> Result<(), LowerError> {
    let mut paths = Vec::new();
    if document
        .extends
        .as_ref()
        .is_some_and(|p| !p.starts_with("preset:"))
    {
        paths.push("extends".into());
    }
    if let Some(firmware) = &document.firmware {
        if firmware
            .bios
            .as_ref()
            .is_some_and(|rom| rom.image.is_some())
        {
            paths.push("firmware.bios.image".into());
        }
        if let Some(rom) = &firmware.bios {
            for (i, _) in rom.parts.iter().flatten().enumerate() {
                paths.push(format!("firmware.bios.parts[{i}].image"));
            }
        }
        for (i, rom) in firmware.option_roms.iter().flatten().enumerate() {
            if rom.image.is_some() {
                paths.push(format!("firmware.option_roms[{i}].image"));
            }
        }
    }
    if let Some(Nvram::Image(image)) = document
        .board
        .as_ref()
        .and_then(|b| b.rtc.as_ref())
        .and_then(|r| r.nvram.as_ref())
    {
        if image.image.is_some() {
            paths.push("board.rtc.nvram.image".into());
        }
        if image.journal.is_some() {
            paths.push("board.rtc.nvram.journal".into());
        }
    }
    for (i, card) in document.card.iter().flatten().enumerate() {
        for (j, drive) in card.drives.iter().flatten().enumerate() {
            if let Some(media) = &drive.media {
                for (k, medium) in media.iter().enumerate() {
                    if !medium.image.is_empty() {
                        paths.push(format!("card[{i}].drives[{j}].media[{k}].image"));
                    }
                    if medium.journal.is_some() {
                        paths.push(format!("card[{i}].drives[{j}].media[{k}].journal"));
                    }
                }
            }
            if drive.image.is_some() {
                paths.push(format!("card[{i}].drives[{j}].image"));
            }
            if drive.journal.is_some() {
                paths.push(format!("card[{i}].drives[{j}].journal"));
            }
        }
    }
    if let Some(field) = paths.first() {
        return Err(error(
            origins,
            field,
            "embedded presets must not contain paths",
        ));
    }
    Ok(())
}

fn complete(resolved: &Resolved) -> Result<(), LowerError> {
    let d = &resolved.document;
    let mut missing = Vec::new();
    let mut require = |field: String, present: bool| {
        if !present {
            missing.push(field);
        }
    };
    require(
        "board.model".into(),
        d.board.as_ref().is_some_and(|b| b.model.is_some()),
    );
    require(
        "firmware.bios.image".into(),
        d.firmware
            .as_ref()
            .and_then(|f| f.bios.as_ref())
            .is_some_and(|r| r.image.is_some() || r.parts.is_some() || r.use_id.is_some()),
    );
    if let Some(f) = &d.firmware {
        for (i, rom) in f.option_roms.iter().flatten().enumerate() {
            require(
                format!("firmware.option_roms[{i}].image"),
                rom.image.is_some() || rom.use_id.is_some(),
            );
            require(
                format!("firmware.option_roms[{i}].base"),
                rom.base.is_some(),
            );
        }
    }
    if let Some(Nvram::Image(image)) = d
        .board
        .as_ref()
        .and_then(|b| b.rtc.as_ref())
        .and_then(|r| r.nvram.as_ref())
    {
        require("board.rtc.nvram.image".into(), image.image.is_some());
    }
    for (i, card) in d.card.iter().flatten().enumerate() {
        let field = format!("card[{i}]");
        require(format!("{field}.kind"), card.kind.is_some());
        if card.kind == Some(CardKind::Uart16550)
            && card.irq.is_none()
            && card
                .port
                .is_some_and(|p| ![0x3F8, 0x3E8, 0x2F8, 0x2E8].contains(&p))
        {
            require(format!("{field}.irq"), false);
        }
        for (j, drive) in card.drives.iter().flatten().enumerate() {
            let field = format!("{field}.drives[{j}]");
            require(
                format!("{field}.image"),
                drive.image.is_some()
                    || drive.media.is_some()
                    || drive.empty == Some(true)
                    || drive.disk.is_some()
                    || drive.inserted.is_some(),
            );
            match card.kind {
                Some(CardKind::Fdc765) => {
                    require(format!("{field}.unit"), drive.unit.is_some());
                    require(format!("{field}.type"), drive.drive_type.is_some());
                }
                Some(CardKind::Ide) => {
                    require(format!("{field}.channel"), drive.channel.is_some());
                    if let Some(Geometry::Chs(chs)) = &drive.geometry {
                        require(
                            format!("{field}.geometry.cylinders"),
                            chs.cylinders.is_some(),
                        );
                        require(format!("{field}.geometry.heads"), chs.heads.is_some());
                        require(format!("{field}.geometry.sectors"), chs.sectors.is_some());
                    } else {
                        require(format!("{field}.geometry"), false);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(first) = missing.first() {
        let fields = missing
            .iter()
            .map(|field| format!("{field} ({})", resolved.origins.get(field)))
            .collect::<Vec<_>>()
            .join(", ");
        let chain = resolved
            .chain
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" → ");
        return Err(error(
            &resolved.origins,
            first,
            format!("required fields are missing: {fields}; inheritance chain: {chain}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(parent: &str, child: &str) -> Result<Resolved, LowerError> {
        let (mut document, origins) = parse(
            Path::new("parent.toml"),
            &format!("schema=1\n{parent}"),
            false,
        )
        .unwrap();
        validate_layer(&document, &origins)?;
        clear_remove(&mut document);
        let (child, child_origins) = parse(
            Path::new("child.toml"),
            &format!("schema=1\nextends='parent.toml'\n{child}"),
            false,
        )
        .unwrap();
        validate_layer(&child, &child_origins)?;
        merge(
            Resolved {
                document,
                origins,
                chain: vec!["parent.toml".into()],
            },
            child,
            child_origins,
        )
    }
    fn cards(d: &MachineFile) -> &[Card] {
        d.card.as_deref().unwrap()
    }

    #[test]
    fn lotech_ems_resource_fields_inherit_override_and_reject_remove() {
        let parent = "[[card]]\nkind='lotech-ems'\nport=0x260\nframe=0xE000\nsize_kib=2048\n";
        let inherited = merged(parent, "[[card]]\nid='lotech-ems'\nport=0x264\n").unwrap();
        let card = &cards(&inherited.document)[0];
        assert_eq!(
            (card.port, card.frame, card.size_kib),
            (Some(0x264), Some(0xE000), Some(2048))
        );
        let overridden = merged(
            parent,
            "[[card]]\nid='lotech-ems'\nframe=0xD000\nsize_kib=512\n",
        )
        .unwrap();
        let card = &cards(&overridden.document)[0];
        assert_eq!(
            (card.port, card.frame, card.size_kib),
            (Some(0x260), Some(0xD000), Some(512))
        );
        assert_eq!(
            overridden.origins.get("card[0].frame").file,
            Path::new("child.toml")
        );
        for field in ["frame=0xD000", "size_kib=512"] {
            assert!(merged(
                parent,
                &format!("[[card]]\nid='lotech-ems'\nremove=true\n{field}\n")
            )
            .is_err());
        }
    }

    #[test]
    fn sb16_resource_fields_merge_and_removed_cards_reject_overrides() {
        let result = merged(
            "[[card]]\nkind='sb16'\nbase=0x220\nirq=5\ndma8=1\ndma16=5\n",
            "[[card]]\nid='sb16'\nbase=0x240\nirq=7\ndma8=3\ndma16=7\n",
        )
        .unwrap();
        let card = &result.document.card.as_ref().unwrap()[0];
        assert_eq!(card.base, Some(0x240));
        assert_eq!(card.dma8, Some(3));
        assert_eq!(card.dma16, Some(7));
        assert_eq!(card.irq, Some(crate::schema::v1::NumberOrNone::Number(7)));
        for field in ["base=0x240", "dma8=1", "dma16=5"] {
            assert!(merged(
                "[[card]]\nkind='sb16'\n",
                &format!("[[card]]\nid='sb16'\nremove=true\n{field}\n")
            )
            .is_err());
        }
    }

    #[test]
    fn at_adapter_model_inherits_overrides_and_cannot_accompany_removal() {
        let parent = "[[card]]\nkind='ide'\ncontroller_model='at-adapter'\n";
        let inherited = merged(parent, "[[card]]\nid='ide'\nirq=14\n").unwrap();
        assert_eq!(
            cards(&inherited.document)[0].controller_model,
            Some(ControllerModel::AtAdapter)
        );
        assert_eq!(
            inherited.origins.get("card[0].controller_model").file,
            Path::new("parent.toml")
        );
        let overridden = merged(parent, "[[card]]\nid='ide'\ncontroller_model='ata'\n").unwrap();
        assert_eq!(
            cards(&overridden.document)[0].controller_model,
            Some(ControllerModel::Ata)
        );
        assert_eq!(
            overridden.origins.get("card[0].controller_model").file,
            Path::new("child.toml")
        );
        let error = merged(
            parent,
            "[[card]]\nid='ide'\nremove=true\ncontroller_model='at-adapter'\n",
        )
        .unwrap_err();
        assert!(error.message.contains("only id and remove"));
    }

    #[test]
    fn keyless_drives_never_merge_with_one_another() {
        let resolved = merged(
            "[[card]]\nkind='fdc-765'\n[[card.drives]]\nimage='parent.img'\n",
            "[[card]]\nid='fdc-765'\n[[card.drives]]\nimage='child.img'\n",
        )
        .unwrap();
        let drives = cards(&resolved.document)[0].drives.as_ref().unwrap();
        assert_eq!(drives.len(), 2);
        assert_eq!(drives[0].image.as_deref(), Some("parent.img"));
        assert_eq!(drives[1].image.as_deref(), Some("child.img"));
    }

    #[test]
    fn referenced_presets_are_checked_during_loading() {
        let path =
            std::env::temp_dir().join(format!("machine-unsafe-preset-{}.toml", std::process::id()));
        std::fs::write(&path, "schema=1\nextends='preset:unsafe'\n").unwrap();
        let presets = [(
            "unsafe",
            "schema=1\n[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='forbidden.bin'\n",
        )];
        let result = Resolver {
            presets: &presets,
            stack: Vec::new(),
        }
        .visit(&path, &path, None, None);
        std::fs::remove_file(path).unwrap();
        let LoadError::Lower(error) = result.unwrap_err() else {
            panic!()
        };
        assert_eq!(error.message, "embedded presets must not contain paths");
        assert_eq!(error.field, "firmware.bios.image");
        assert_eq!(error.origin.file, Path::new("preset:unsafe"));
        assert_eq!(error.origin.line, Some(5));
    }

    #[test]
    fn embedded_presets_reject_journal_paths_without_images() {
        let input = "schema=1\n[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\njournal='forbidden.upfj'\n";
        let (document, origins) = parse(Path::new("preset:unsafe"), input, false).unwrap();
        let error = check_preset(&document, &origins).unwrap_err();
        assert_eq!(error.message, "embedded presets must not contain paths");
        assert_eq!(error.field, "card[0].drives[0].journal");
        assert_eq!(error.origin.file, Path::new("preset:unsafe"));
        assert_eq!(error.origin.line, Some(6));
    }

    #[test]
    fn empty_keyed_arrays_keep_entries_and_false_remove_merges() {
        let parent = "[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\nimage='a.img'\n";
        let r = merged(parent, "card=[]\n").unwrap();
        assert_eq!(cards(&r.document).len(), 1);
        let r = merged(parent, "[[card]]\nid='fdc-765'\nremove=false\ndrives=[]\n").unwrap();
        assert_eq!(cards(&r.document)[0].drives.as_ref().unwrap().len(), 1);
        assert_eq!(cards(&r.document)[0].remove, None);
        let r = merged(
            parent,
            "[[card]]\nid='fdc-765'\n[[card.drives]]\nunit=0\nremove=false\nwrite='readonly'\n",
        )
        .unwrap();
        let drive = &cards(&r.document)[0].drives.as_ref().unwrap()[0];
        assert_eq!(drive.remove, None);
        assert_eq!(drive.write, Some(WriteMode::ReadOnly));
    }

    #[test]
    fn image_overrides_clear_inherited_hashes_unless_explicitly_replaced() {
        let pin = "0".repeat(64);
        let r = merged(
            &format!("[firmware.bios]\nimage='old.bin'\nsha256='{pin}'\n"),
            "[firmware.bios]\nimage='new.bin'\n",
        )
        .unwrap();
        let bios = r.document.firmware.unwrap().bios.unwrap();
        assert_eq!(bios.image.as_deref(), Some("new.bin"));
        assert!(bios.sha256.is_none());
        assert_eq!(
            r.origins.get("firmware.bios.image").file,
            Path::new("child.toml")
        );
        assert_ne!(
            r.origins.get("firmware.bios.sha256").file,
            Path::new("parent.toml")
        );
    }

    #[test]
    fn array_values_and_empty_lists_keep_their_own_source_lines() {
        for (yaml, input, path) in [
            (
                false,
                "schema=1\n[firmware.bios]\nprovides=[]\n",
                "layer.toml",
            ),
            (
                true,
                "schema: 1\nfirmware:\n  bios:\n    image: bios.bin\n    provides: []\n",
                "layer.yaml",
            ),
        ] {
            let (_, origins) = parse(Path::new(path), input, yaml).unwrap();
            assert_eq!(
                origins.get("firmware.bios.provides").line,
                Some(if yaml { 5 } else { 3 })
            );
        }
    }

    #[test]
    fn scalar_child_replaces_parent_and_omission_inherits() {
        let r = merged("[cpu]\nmodel='80286'\nrate='2/3'\n", "[cpu]\nrate=4\n").unwrap();
        let cpu = r.document.cpu.unwrap();
        assert_eq!(cpu.model, Some(CpuModel::I80286));
        assert_eq!(
            cpu.rate,
            Some(CpuRate {
                steps: std::num::NonZeroU64::new(4).unwrap(),
                pit_clocks: std::num::NonZeroU64::new(1).unwrap()
            })
        );
    }
    #[test]
    fn board_display_switch_inherits_and_auto_resets_the_parent_override() {
        for (child, expected) in [
            ("", DisplaySwitch::Mono),
            ("display_switch='colour'", DisplaySwitch::Colour),
            ("display_switch='auto'", DisplaySwitch::Auto),
        ] {
            let resolved = merged(
                "[board]\nmodel='ibm-5170'\ndisplay_switch='mono'\n",
                &format!("[board]\n{child}\n"),
            )
            .unwrap();
            assert_eq!(
                resolved.document.board.unwrap().display_switch,
                Some(expected)
            );
        }
    }

    #[test]
    fn serial_mouse_inherits_and_can_be_explicitly_disabled() {
        for (child, expected) in [
            ("", SerialMouseModel::Microsoft),
            ("mouse='none'\n", SerialMouseModel::None),
        ] {
            let resolved = merged(
                "[[card]]\nkind='uart-16550'\nmouse='microsoft'\n",
                &format!("[[card]]\nid='uart-16550'\n{child}"),
            )
            .unwrap();
            assert_eq!(resolved.document.card.unwrap()[0].mouse, Some(expected));
        }
    }

    #[test]
    fn board_mouse_inherits_and_can_be_explicitly_disabled() {
        for (parent, child, expected) in [
            ("mouse=true\n", "", Some(true)),
            ("mouse=true\n", "mouse=false\n", Some(false)),
            ("mouse=false\n", "mouse=true\n", Some(true)),
        ] {
            let resolved = merged(
                &format!("[board]\nmodel='ibm-5170'\n{parent}"),
                &format!("[board]\n{child}"),
            )
            .unwrap();
            assert_eq!(resolved.document.board.unwrap().mouse, expected);
        }
    }

    #[test]
    fn child_cpu_fpu_override_is_merged_with_its_origin() {
        for (parent, child, expected) in [
            ("none", "80287", FpuModel::I80287),
            ("none", "80387", FpuModel::I80387),
            ("80387", "none", FpuModel::None),
            ("80287", "none", FpuModel::None),
        ] {
            let resolved = merged(
                &format!("[cpu]\nmodel='80286'\nrate='8/13'\nfpu='{parent}'\n"),
                &format!("[cpu]\nfpu='{child}'\n"),
            )
            .unwrap();
            let cpu = resolved.document.cpu.unwrap();
            assert_eq!(cpu.fpu, Some(expected));
            assert_eq!(cpu.model, Some(CpuModel::I80286));
            assert_eq!(cpu.rate.unwrap().steps.get(), 8);
            assert_eq!(
                resolved.origins.get("cpu.fpu").file,
                Path::new("child.toml")
            );
        }
        let inherited = merged("[cpu]\nfpu='80287'\n", "[cpu]\nrate=2\n").unwrap();
        assert_eq!(inherited.document.cpu.unwrap().fpu, Some(FpuModel::I80287));
        assert_eq!(
            inherited.origins.get("cpu.fpu").file,
            Path::new("parent.toml")
        );
    }

    #[test]
    fn tables_merge_recursively_field_by_field() {
        let r = merged("[board]\nmodel='ibm-5170'\npost_port=128\n[board.rtc]\nstart='2001-02-03T00:00:00'\nnvram='derived'\n[cpu]\nmodel='80286'\nrate=2\n[memory]\nconventional_kib=512\nextended_kib=64\n[firmware.bios]\nimage='rom.bin'\nbase=0xF8000\nprovides=['floppy']\n", "[board]\npost_port='none'\n[board.rtc]\nnvram='blank'\n[cpu]\nrate=3\n[memory]\nextended_kib=128\n[firmware.bios]\nbase=0xF0000\n").unwrap();
        let d = r.document;
        let b = d.board.unwrap();
        assert_eq!(b.model, Some(BoardModel::Ibm5170));
        assert_eq!(b.post_port, Some(NumberOrNone::None));
        let rtc = b.rtc.unwrap();
        assert_eq!(rtc.start.as_deref(), Some("2001-02-03T00:00:00"));
        assert_eq!(rtc.nvram, Some(Nvram::Blank));
        assert_eq!(d.cpu.unwrap().model, Some(CpuModel::I80286));
        let ram = d.memory.unwrap();
        assert_eq!(
            (ram.conventional_kib, ram.extended_kib),
            (Some(512), Some(128))
        );
        let rom = d.firmware.unwrap().bios.unwrap();
        assert_eq!(rom.image.as_deref(), Some("rom.bin"));
        assert_eq!(rom.base, Some(0xF0000));
        assert_eq!(rom.provides, Some(vec![FirmwareService::Floppy]));
    }
    #[test]
    fn two_form_nvram_replaces_whole_when_form_changes() {
        let r = merged(
            "[board.rtc]\nnvram={image='old.bin',write='readonly'}\n",
            "[board.rtc]\nnvram='derived'\n",
        )
        .unwrap();
        assert_eq!(
            r.document.board.unwrap().rtc.unwrap().nvram,
            Some(Nvram::Derived)
        );
        let r = merged(
            "[board.rtc]\nnvram='blank'\n",
            "[board.rtc]\nnvram={image='new.bin'}\n",
        )
        .unwrap();
        assert!(matches!(
            r.document.board.unwrap().rtc.unwrap().nvram,
            Some(Nvram::Image(_))
        ));
    }
    #[test]
    fn nvram_and_geometry_tables_merge_fields() {
        let r = merged("[board.rtc]\nnvram={image='old.bin',write='volatile'}\n[[card]]\nkind='ide'\n[[card.drives]]\nchannel='master'\ngeometry={cylinders=615,heads=4,sectors=17}\n", "[board.rtc]\nnvram={write='readonly'}\n[[card]]\nid='ide'\n[[card.drives]]\nchannel='master'\ngeometry={heads=8}\n").unwrap();
        let Some(Nvram::Image(nvram)) = r
            .document
            .board
            .as_ref()
            .unwrap()
            .rtc
            .as_ref()
            .unwrap()
            .nvram
            .as_ref()
        else {
            panic!()
        };
        assert_eq!(nvram.image.as_deref(), Some("old.bin"));
        assert_eq!(nvram.write, Some(WriteMode::ReadOnly));
        let Some(Geometry::Chs(chs)) = cards(&r.document)[0].drives.as_ref().unwrap()[0]
            .geometry
            .as_ref()
        else {
            panic!()
        };
        assert_eq!(
            (chs.cylinders, chs.heads, chs.sectors),
            (Some(615), Some(8), Some(17))
        );
    }
    #[test]
    fn provides_and_option_rom_arrays_replace_whole_including_empty() {
        let parent = "[firmware]\noption_roms=[{image='a.bin',base=0xC8000},{image='b.bin',base=0xD0000}]\n[firmware.bios]\nimage='bios.bin'\nprovides=['floppy','fixed-disk']\n";
        let r = merged(parent, "[firmware]\noption_roms=[{image='c.bin',base=0xD8000}]\n[firmware.bios]\nprovides=['fixed-disk']\n").unwrap();
        let f = r.document.firmware.unwrap();
        assert_eq!(
            f.option_roms
                .unwrap()
                .iter()
                .map(|rom| rom.image.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["c.bin"]
        );
        assert_eq!(
            f.bios.unwrap().provides.unwrap(),
            [FirmwareService::FixedDisk]
        );
        let r = merged(
            parent,
            "[firmware]\noption_roms=[]\n[firmware.bios]\nprovides=[]\n",
        )
        .unwrap();
        let f = r.document.firmware.unwrap();
        assert!(f.option_roms.unwrap().is_empty());
        assert!(f.bios.unwrap().provides.unwrap().is_empty());
    }
    #[test]
    fn matching_cards_merge_by_effective_id_and_preserve_parent_order() {
        let r = merged(
            "[[card]]\nkind='mda'\n[[card]]\nid='serial'\nkind='uart-16550'\nport=0x2F8\nirq=3\n",
            "[[card]]\nid='serial'\nirq='none'\n[[card]]\nid='mda'\nkind='mda'\n",
        )
        .unwrap();
        let c = cards(&r.document);
        assert_eq!(
            c.iter().map(card_id).collect::<Vec<_>>(),
            [Some("mda"), Some("serial")]
        );
        assert_eq!(c[1].kind, Some(CardKind::Uart16550));
        assert_eq!(c[1].port, Some(0x2F8));
        assert_eq!(c[1].irq, Some(NumberOrNone::None));
    }
    #[test]
    fn inherited_card_kind_cannot_change() {
        let e = merged(
            "[[card]]\nkind='mda'\n",
            "[[card]]\nid='mda'\nkind='vga-text'\n",
        )
        .err()
        .unwrap();
        assert_eq!(e.message, "card `mda` cannot change inherited kind");
        assert_eq!(e.field, "card[0].kind");
        assert_eq!(e.origin.file, Path::new("child.toml"));
        assert_eq!(e.origin.line, Some(5));
    }
    #[test]
    fn new_cards_append_in_child_order() {
        let r = merged(
            "[[card]]\nkind='mda'\n",
            "[[card]]\nkind='uart-16550'\n[[card]]\nkind='vga-text'\n",
        )
        .unwrap();
        assert_eq!(
            cards(&r.document).iter().map(card_id).collect::<Vec<_>>(),
            [Some("mda"), Some("uart-16550"), Some("vga-text")]
        );
    }
    #[test]
    fn remove_deletes_inherited_card_and_reindexes_origins() {
        let r = merged(
            "[[card]]\nkind='mda'\n[[card]]\nkind='uart-16550'\nport=0x2F8\n",
            "[[card]]\nid='mda'\nremove=true\n",
        )
        .unwrap();
        assert_eq!(cards(&r.document).len(), 1);
        assert_eq!(card_id(&cards(&r.document)[0]), Some("uart-16550"));
        assert_eq!(r.origins.get("card[0].port").line, Some(6));
        assert_eq!(r.origins.get("card[0].port").file, Path::new("parent.toml"));
    }
    #[test]
    fn drive_unit_and_channel_keys_merge_append_and_delete() {
        for (kind, key, first, second) in [
            ("fdc-765", "unit", "0", "1"),
            ("ide", "channel", "'master'", "'slave'"),
        ] {
            let p = format!("[[card]]\nkind='{kind}'\n[[card.drives]]\n{key}={first}\nimage='first.img'\nwrite='volatile'\n");
            let c = format!("[[card]]\nid='{kind}'\n[[card.drives]]\n{key}={first}\nwrite='readonly'\n[[card.drives]]\n{key}={second}\nimage='second.img'\n");
            let r = merged(&p, &c).unwrap();
            let drives = cards(&r.document)[0].drives.as_ref().unwrap();
            assert_eq!(drives.len(), 2);
            assert_eq!(drives[0].image.as_deref(), Some("first.img"));
            assert_eq!(drives[0].write, Some(WriteMode::ReadOnly));
            assert_eq!(drives[1].image.as_deref(), Some("second.img"));
            let r = merged(
                &format!("{p}[[card.drives]]\n{key}={second}\nimage='second.img'\n"),
                &format!("[[card]]\nid='{kind}'\n[[card.drives]]\n{key}={first}\nremove=true\n"),
            )
            .unwrap();
            assert_eq!(cards(&r.document)[0].drives.as_ref().unwrap().len(), 1);
            assert_eq!(
                cards(&r.document)[0].drives.as_ref().unwrap()[0]
                    .image
                    .as_deref(),
                Some("second.img")
            );
            assert_eq!(r.origins.get("card[0].drives[0].image").line, Some(10));
        }
    }
    #[test]
    fn metadata_is_not_inherited_and_each_layer_requires_schema() {
        let r = merged("name='parent'\ndescription='parent text'\n", "").unwrap();
        assert!(r.document.name.is_none());
        assert!(r.document.description.is_none());
        let r = merged(
            "name='parent'\n",
            "name='child'\ndescription='child text'\n",
        )
        .unwrap();
        assert_eq!(r.document.name.as_deref(), Some("child"));
        assert_eq!(r.document.description.as_deref(), Some("child text"));
        for input in [
            "",
            "extends='parent.toml'\n",
            "schema=2\nextends='parent.toml'\n",
        ] {
            assert!(parse(Path::new("child.toml"), input, false).is_err());
        }
    }
    #[test]
    fn remove_requires_extends_even_when_false() {
        for input in [
            "[[card]]\nid='mda'\nremove=true\n",
            "[[card]]\nkind='mda'\nremove=true\n",
            "[[card]]\nkind='mda'\nremove=false\n",
            "[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\nremove=true\n",
            "[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\nremove=false\n",
        ] {
            let (d, o) =
                parse(Path::new("root.toml"), &format!("schema=1\n{input}"), false).unwrap();
            assert_eq!(
                validate_layer(&d, &o).unwrap_err().message,
                "remove is only accepted in an extends layer"
            );
        }
    }
    #[test]
    fn removal_of_unknown_card_or_drive_fails() {
        let e = merged("", "[[card]]\nid='absent'\nremove=true\n")
            .err()
            .unwrap();
        assert_eq!(
            e.message,
            "cannot remove card `absent`: parent does not define it"
        );
        for p in ["", "[[card]]\nkind='fdc-765'\n"] {
            let e = merged(
                p,
                "[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\nremove=true\n",
            )
            .err()
            .unwrap();
            assert_eq!(
                e.message,
                "cannot remove drive unit=0: parent does not define it"
            );
        }
    }
    #[test]
    fn new_card_requires_kind() {
        for child in ["[[card]]\nid='new'\n", "[[card]]\n"] {
            assert_eq!(
                merged("", child).err().unwrap().message,
                "kind is required for a new card"
            );
        }
    }
    #[test]
    fn duplicate_keys_are_not_silently_merged_inside_one_layer() {
        for child in [
            "[[card]]\nkind='mda'\n[[card]]\nid='mda'\n",
            "[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\n[[card.drives]]\nunit=0\n",
        ] {
            let e = merged("", child).err().unwrap();
            assert!(e.message.starts_with("duplicate "));
            assert!(e.message.contains("first defined at child.toml:"));
        }
    }
    #[test]
    fn removed_entries_reject_overrides_and_ambiguous_drive_keys() {
        for (child, message) in [
            (
                "[[card]]\nid='mda'\nremove=true\nirq=4\n",
                "a removed card accepts only id and remove",
            ),
            (
                "[[card]]\nid='fdc-765'\n[[card.drives]]\nunit=0\nremove=true\nimage='x'\n",
                "a removed drive accepts only its key and remove",
            ),
            (
                "[[card]]\nid='fdc-765'\n[[card.drives]]\nunit=0\nchannel='master'\n",
                "a drive must have exactly one key: unit or channel",
            ),
        ] {
            assert_eq!(merged("", child).err().unwrap().message, message);
        }
    }
    #[test]
    fn field_origins_follow_values_across_layers_and_array_replacement() {
        let r = merged("[board]\nmodel='ibm-5170'\n[firmware.bios]\nimage='parent.bin'\nprovides=['floppy']\n[[card]]\nkind='fdc-765'\n[[card.drives]]\nunit=0\nimage='parent.img'\ntype='1.44M'\n", "[firmware.bios]\nbase=0xF8000\n[[card]]\nid='fdc-765'\n[[card.drives]]\nunit=0\nwrite='readonly'\n").unwrap();
        for field in [
            "board.model",
            "firmware.bios.image",
            "card[0].drives[0].image",
            "card[0].drives[0].type",
        ] {
            assert_eq!(
                r.origins.get(field).file,
                Path::new("parent.toml"),
                "{field}"
            );
        }
        for (field, line) in [("firmware.bios.base", 4), ("card[0].drives[0].write", 9)] {
            assert_eq!(r.origins.get(field).file, Path::new("child.toml"));
            assert_eq!(r.origins.get(field).line, Some(line));
        }
        let r = merged(
            "[firmware]\noption_roms=[{image='old',base=1},{image='old2',base=2}]\n",
            "[firmware]\noption_roms=[{image='new',base=3}]\n",
        )
        .unwrap();
        assert_eq!(
            r.origins.get("firmware.option_roms[0].image").file,
            Path::new("child.toml")
        );
        assert!(!r
            .origins
            .get("firmware.option_roms[1].image")
            .file
            .ends_with("parent.toml"));
    }
    #[test]
    fn embedded_presets_reject_every_path_bearing_field() {
        for input in [
            "extends='parent.toml'\n",
            "[firmware.bios]\nimage='bios.bin'\n",
            "[firmware]\noption_roms=[{image='option.bin'}]\n",
            "[board.rtc]\nnvram={image='nvram.bin'}\n",
            "[board.rtc]\nnvram={journal='nvram.journal'}\n",
            "[[card]]\nkind='fdc-765'\n[[card.drives]]\nimage='disk.img'\n",
            "[[card]]\nkind='fdc-765'\n[[card.drives]]\njournal='disk.journal'\n",
        ] {
            let (d, o) = parse(
                Path::new("preset:test"),
                &format!("schema=1\n{input}"),
                false,
            )
            .unwrap();
            let e = check_preset(&d, &o).unwrap_err();
            assert_eq!(e.message, "embedded presets must not contain paths");
            assert_eq!(e.origin.file, Path::new("preset:test"));
            assert!(e.origin.line.is_some());
        }
    }
    #[test]
    fn embedded_preset_has_no_image_specific_bios_fields() {
        let (_, text) = PRESETS
            .iter()
            .find(|(name, _)| *name == "ibm-5170")
            .unwrap();
        let (document, _) = parse(Path::new("preset:ibm-5170"), text, false).unwrap();
        let bios = document.firmware.unwrap().bios.unwrap();
        assert!(bios.image.is_none());
        assert!(bios.parts.is_none());
        assert!(bios.sha256.is_none());
        assert!(bios.provides.is_none());
    }

    #[test]
    fn preset_registry_matches_files_and_contains_no_paths() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../machines/presets");
        let mut files = std::fs::read_dir(&directory)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        files.sort();
        let mut registered = PRESETS
            .iter()
            .map(|(name, text)| {
                assert_eq!(
                    *text,
                    std::fs::read_to_string(directory.join(format!("{name}.toml"))).unwrap()
                );
                let (d, o) = parse(Path::new(&format!("preset:{name}")), text, false).unwrap();
                check_preset(&d, &o).unwrap();
                format!("{name}.toml")
            })
            .collect::<Vec<_>>();
        registered.sort();
        assert_eq!(files, registered);
    }
}
