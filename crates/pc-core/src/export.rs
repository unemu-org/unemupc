// SPDX-License-Identifier: AGPL-3.0-or-later
//! Offline PC media and schema-1 exports; machine-files § Schema and
//! core-contract §4. Current sectors arrive through one read-only reader,
//! leaving base images, layers and journals under the front end's ownership.
//! IBM Personal Computer AT Technical Reference, Fixed Disk and Diskette
//! Drive Adapter sections: slot wiring and drive types match the running PC.

use super::{check_type, config_error};
use pc_config::{image, launch};
use sha2::{Digest, Sha256};
use toml::{Table, Value};
use unemu_core_api::{
    ConfigDocument, Error, ErrorKind, ExportMedium, ExportReport, ExportRequest, ExportSink,
    ExportedOutput, MediaDescription, SlotDescription, SlotId, SlotKind,
};

struct Drive {
    description: MediaDescription,
    slot_index: usize,
    chs: Option<[u32; 3]>,
}

fn checked(
    platform: &str,
    document: ConfigDocument<'_>,
) -> Result<(launch::Launch, Vec<Drive>), Error> {
    check_type(platform, document)?;
    let launch = launch::decode(document.bytes).map_err(config_error)?;
    launch::check(&launch, assembly::AssemblyOptions::default()).map_err(config_error)?;
    let mut hardware = launch.config.machine.clone();
    let mut drives = Vec::new();
    image::drive_tables(&mut hardware, |_, kind, drive| -> image::Result<()> {
        let slot_index = drives.len();
        let binding = &launch.config.slots[slot_index];
        let fixed = kind == "ide";
        let id = SlotId(binding.unit + if fixed { 2 } else { 0 });
        let selected = binding
            .inserted
            .as_ref()
            .and_then(|id| launch.config.media.iter().find(|m| &m.id == id));
        let (detail, size, chs) = if fixed {
            let g = drive
                .get("geometry")
                .and_then(Value::as_table)
                .ok_or_else(|| image::Error::Invalid("missing disk geometry".into()))?;
            let mut chs = [0; 3];
            for (out, key) in chs.iter_mut().zip(["cylinders", "heads", "sectors"]) {
                *out = g
                    .get(key)
                    .and_then(Value::as_integer)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| image::Error::Invalid("invalid disk geometry".into()))?;
            }
            let size = u64::from(chs[0]) * u64::from(chs[1]) * u64::from(chs[2]) * 512;
            (
                format!(
                    "CHS {}/{}/{}, {} MiB",
                    chs[0],
                    chs[1],
                    chs[2],
                    size / (1024 * 1024)
                ),
                Some(size),
                Some(chs),
            )
        } else {
            let drive_type = drive.get("type").and_then(Value::as_str).unwrap_or("1.44M");
            let (detail, geometry) = match drive_type {
                "1.44M" => ("3.5-inch 1.44M", assembly::FloppyGeometry::HD_1_44),
                "720K" => ("3.5-inch 720K", assembly::FloppyGeometry::DD_720K),
                "1.2M" => ("5.25-inch 1.2M", assembly::FloppyGeometry::HD_1_2),
                "360K" => ("5.25-inch 360K", assembly::FloppyGeometry::DD_360K),
                _ => return Err(image::Error::Invalid("invalid floppy type".into())),
            };
            let size = selected.map(|m| geometry_size(&m.geometry)).or_else(|| {
                let media = geometry.accepted_media();
                (media.len() == 1)
                    .then(|| {
                        image::Geometry::of(assembly::DriveKind::Floppy { geometry: media[0] })
                    })
                    .map(|g| geometry_size(&g))
            });
            (detail.to_owned(), size, None)
        };
        drives.push(Drive {
            description: MediaDescription::new(SlotDescription::new(
                id,
                if fixed {
                    SlotKind::FixedDisk
                } else {
                    SlotKind::Floppy
                },
                format!("{}:", char::from(b'A' + id.0)),
                !fixed,
            ))
            .with_detail(detail)
            .with_size(size),
            slot_index,
            chs,
        });
        Ok(())
    })
    .map_err(config_error)?;
    drives.sort_by_key(|d| d.description.slot.id);
    Ok((launch, drives))
}

fn geometry_size(g: &image::Geometry) -> u64 {
    u64::from(g.cylinders) * u64::from(g.heads) * u64::from(g.sectors) * u64::from(g.sector_bytes)
}

pub(super) fn describe(
    platform: &str,
    config: ConfigDocument<'_>,
) -> Result<Vec<MediaDescription>, Error> {
    Ok(checked(platform, config)?
        .1
        .into_iter()
        .map(|d| d.description)
        .collect())
}

fn eligible(launch: &launch::Launch, drives: &[Drive], format: &str) -> Result<Vec<SlotId>, Error> {
    if !matches!(format, "raw" | "vhd-fixed" | "machine") {
        return Err(Error::new(ErrorKind::Usage, "unknown PC export format"));
    }
    Ok(drives
        .iter()
        .filter(|d| match format {
            "vhd-fixed" => d.chs.is_some(),
            "machine" => launch.config.slots[d.slot_index].inserted.is_some(),
            _ => true,
        })
        .map(|d| d.description.slot.id)
        .collect())
}

pub(super) fn slots(
    platform: &str,
    config: ConfigDocument<'_>,
    format: &str,
) -> Result<Vec<SlotId>, Error> {
    let (launch, drives) = checked(platform, config)?;
    eligible(&launch, &drives, format)
}

pub(super) fn export(
    platform: &str,
    config: ConfigDocument<'_>,
    request: ExportRequest<'_>,
    sink: &mut dyn ExportSink,
) -> Result<ExportReport, Error> {
    let (launch, drives) = checked(platform, config)?;
    let slots = eligible(&launch, &drives, request.format)?;
    let machine = request.format == "machine";
    let mut seen = std::collections::BTreeSet::new();
    if request.media.len() != if machine { slots.len() } else { 1 }
        || request
            .media
            .iter()
            .any(|m| !slots.contains(&m.slot) || !seen.insert(m.slot))
    {
        return Err(Error::new(
            ErrorKind::Usage,
            "export needs each required slot exactly once",
        ));
    }
    // Validate every reader before reserving even the first output.
    for medium in request.media {
        let drive = drives
            .iter()
            .find(|d| d.description.slot.id == medium.slot)
            .ok_or_else(|| Error::new(ErrorKind::Usage, "unknown export slot"))?;
        if medium.size != medium.bytes.size()
            || drive
                .description
                .size
                .is_some_and(|size| size != medium.size)
        {
            return Err(Error::new(
                ErrorKind::Content,
                "export medium length mismatch",
            ));
        }
    }
    let mut report = ExportReport::new();
    for drive in &drives {
        let Some(medium) = request
            .media
            .iter()
            .find(|m| m.slot == drive.description.slot.id)
        else {
            continue;
        };
        let name = format!(
            "slot-{}.{}",
            medium.slot.0,
            if request.format == "vhd-fixed" {
                "vhd"
            } else {
                "img"
            }
        );
        let output = sink.create(&name)?;
        if let Some(chs) = drive.chs.filter(|_| request.format == "vhd-fixed") {
            let mut hash = Sha256::new();
            stream(*medium, |bytes| {
                hash.update(bytes);
                output.write(bytes)
            })?;
            output.write(&vhd_footer(medium.size, chs, hash.finalize().into()))?;
        } else {
            stream(*medium, |bytes| output.write(bytes))?;
        }
        report.outputs.push(output.finish()?);
    }
    if machine {
        let text = machine_document(&launch, &drives, &report.outputs, &mut report.notes)?;
        let output = sink.create("machine.toml")?;
        output.write(text.as_bytes())?;
        report.outputs.push(output.finish()?);
    } else {
        let mut fields = Vec::new();
        field_names(
            "machine",
            &Value::Table(launch.config.machine.clone()),
            &mut fields,
        );
        report.notes.push(format!("Disk images omit machine settings: {}. They also omit firmware bindings, media identities, labels and write policies, slot catalogues and insertion, NVRAM and journal record/retention settings.", fields.join(", ")));
        if request.format == "raw" {
            report
                .notes
                .push("Raw images omit the configured drive type and CHS geometry.".into());
        }
    }
    Ok(report)
}

fn stream(
    medium: ExportMedium<'_>,
    mut write: impl FnMut(&[u8]) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut buffer = [0; 65536];
    let mut offset = 0;
    while offset < medium.size {
        let count = (medium.size - offset).min(buffer.len() as u64) as usize;
        medium
            .bytes
            .read_at(offset, &mut buffer[..count])
            .map_err(|error| {
                if error.kind == ErrorKind::Usage {
                    Error::new(
                        ErrorKind::Content,
                        "export reader rejected an in-range read",
                    )
                } else {
                    error
                }
            })?;
        write(&buffer[..count])?;
        offset += count as u64;
    }
    Ok(())
}

/// Microsoft Virtual Hard Disk Image Format, “Hard Disk Footer Format” and
/// “Disk Geometry”: network-byte-order fields and one's-complement checksum
/// describe a fixed disk. The footer preserves configured CHS rather than
/// estimating geometry. A fixed epoch, creator and SHA-256-derived UUID keep
/// identical current media byte-for-byte reproducible; UUID version 8 marks
/// this application-defined derivation (RFC 9562, §5.8).
fn vhd_footer(size: u64, chs: [u32; 3], digest: [u8; 32]) -> [u8; 512] {
    let mut footer = [0; 512];
    footer[..8].copy_from_slice(b"conectix");
    footer[8..12].copy_from_slice(&2u32.to_be_bytes());
    footer[12..16].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    footer[16..24].fill(0xff);
    footer[28..32].copy_from_slice(b"upc ");
    footer[32..36].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    footer[36..40].copy_from_slice(b"Wi2k");
    footer[40..48].copy_from_slice(&size.to_be_bytes());
    footer[48..56].copy_from_slice(&size.to_be_bytes());
    footer[56..58].copy_from_slice(&(chs[0] as u16).to_be_bytes());
    footer[58] = chs[1] as u8;
    footer[59] = chs[2] as u8;
    footer[60..64].copy_from_slice(&2u32.to_be_bytes());
    footer[68..84].copy_from_slice(&digest[..16]);
    footer[74] = (footer[74] & 0x0f) | 0x80;
    footer[76] = (footer[76] & 0x3f) | 0x80;
    let checksum = !footer.iter().map(|&b| u32::from(b)).sum::<u32>();
    footer[64..68].copy_from_slice(&checksum.to_be_bytes());
    footer
}

fn field_names(prefix: &str, value: &Value, names: &mut Vec<String>) {
    match value {
        Value::Table(table) => {
            for (key, value) in table {
                field_names(&format!("{prefix}.{key}"), value, names);
            }
        }
        Value::Array(values) => {
            for (i, value) in values.iter().enumerate() {
                field_names(&format!("{prefix}[{i}]"), value, names);
            }
        }
        _ => names.push(prefix.into()),
    }
}

fn machine_document(
    launch: &launch::Launch,
    drives: &[Drive],
    outputs: &[ExportedOutput],
    notes: &mut Vec<String>,
) -> Result<String, Error> {
    let mut hardware = launch.config.machine.clone();
    image::firmware_images(&mut hardware, |key, rom| -> image::Result<()> {
        let binding = launch
            .config
            .firmware
            .iter()
            .find(|f| f.key == key)
            .ok_or_else(|| image::Error::Invalid("missing firmware binding".into()))?;
        let row = launch
            .content
            .iter()
            .find(|r| r.digest == binding.blob)
            .ok_or_else(|| image::Error::Invalid("missing firmware content row".into()))?;
        let hash = image::Digest::from_sha256(row.sha256).hex();
        // The loader requires an image name before pin-based firmware
        // discovery. A digest name carries no host location or ROM bytes.
        rom.insert("image".into(), hash.clone().into());
        rom.insert("sha256".into(), hash.into());
        if binding.source_archive.is_some() {
            notes.push(format!(
                "Firmware {key}: source_archive provenance is not represented in schema 1."
            ));
        }
        Ok(())
    })
    .map_err(config_error)?;
    let mut index = 0;
    image::drive_tables(&mut hardware, |_, _, drive| -> image::Result<()> {
        let slot = &launch.config.slots[index];
        let descriptor = drives
            .iter()
            .find(|d| d.slot_index == index)
            .ok_or_else(|| image::Error::Invalid("missing drive description".into()))?;
        index += 1;
        for key in [
            "image", "sha256", "write", "journal", "keep_generations", "media", "empty",
        ] {
            drive.remove(key);
        }
        let name = format!("slot-{}.img", descriptor.description.slot.id.0);
        if let Some(output) = outputs.iter().find(|o| o.name == name) {
            let mut entry = Table::new();
            entry.insert("image".into(), output.name.clone().into());
            entry.insert(
                "sha256".into(),
                image::Digest::from_sha256(output.sha256).hex().into(),
            );
            entry.insert("write".into(), "volatile".into());
            if slot.catalogue_mode {
                let selected = launch
                    .config
                    .media
                    .iter()
                    .find(|m| Some(&m.id) == slot.inserted.as_ref())
                    .ok_or_else(|| image::Error::Invalid("missing inserted medium".into()))?;
                if let Some(name) = &selected.name {
                    notes.push(format!(
                        "Medium {}: custom catalogue label {name:?} is not represented in schema 1; \
                         the loader derives a label from the output name.",
                        selected.id,
                    ));
                }
                drive.insert("media".into(), vec![Value::Table(entry)].into());
            } else {
                drive.extend(entry);
            }
        } else {
            drive.insert("media".into(), Vec::<Value>::new().into());
            drive.insert("empty".into(), true.into());
        }
        for id in &slot.catalogue {
            if Some(id) != slot.inserted.as_ref() {
                notes.push(format!(
                    "Slot {}: catalogue medium {id} is omitted; \
                     only the inserted medium has a current-byte reader.",
                    descriptor.description.slot.id.0,
                ));
            }
        }
        Ok(())
    })
    .map_err(config_error)?;
    for medium in &launch.config.media {
        notes.push(format!("Medium {}: source/head identity {} is replaced by the exported current-byte SHA-256; write policy {} becomes volatile for exported media.", medium.id, medium.head, medium.policy.as_str()));
    }
    for record in &launch.records {
        notes.push(format!("Journal {}: record {} and keep_generations={} are omitted; exported media start with volatile writes.", record.medium, record.name, record.keep_generations));
    }
    if let Some(nvram) = &launch.config.nvram {
        if let Some(rtc) = hardware
            .get_mut("board")
            .and_then(Value::as_table_mut)
            .and_then(|b| b.get_mut("rtc"))
            .and_then(Value::as_table_mut)
        {
            rtc.insert("nvram".into(), "derived".into());
        }
        notes.push(format!("NVRAM: {:?} image {} and its current contents are not exported; board.rtc.nvram becomes derived.", nvram.mode, nvram.head));
    }
    toml::to_string(&hardware).map_err(|error| Error::new(ErrorKind::Config, error.to_string()))
}
