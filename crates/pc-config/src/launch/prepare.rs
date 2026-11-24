// SPDX-License-Identifier: AGPL-3.0-or-later
//! Config-only preparation of digest-addressed launches; content-bindings § Prepared representation.

use super::*;
use crate::{
    bindings,
    image::{drive_tables, firmware_images, Geometry, Zero},
    schema::v1 as schema,
};
use unemu_core_api::{BindingKind, BindingUse, Error as ApiError, ErrorKind};

fn config(error: impl std::fmt::Display) -> ApiError {
    ApiError::new(ErrorKind::Config, error.to_string())
}

/// Resolve facts and construct every row before any content can be opened.
/// Preserved heads keep their flat receipts, including layered identities.
pub fn prepare(bytes: &[u8], uses: &[BindingUse]) -> std::result::Result<Launch, ApiError> {
    let text = std::str::from_utf8(bytes).map_err(config)?;
    let plan = bindings::plan_envelope(text)?;
    let resolved = plan.resolve(uses)?;
    let mut launch = decode(bytes).map_err(config)?;
    let original = launch.config.clone();
    // Preserve the IBM 5170 catalogue fallback when the document omits a board model.
    let board = resolved
        .document
        .board
        .as_ref()
        .and_then(|b| b.model)
        .unwrap_or(schema::BoardModel::Ibm5170);
    let catalogue = crate::lower::board_model(board).firmware_catalogue();
    for item in &resolved.items {
        if item.kind == BindingKind::FirmwareRom
            && !catalogue.iter().any(|entry| {
                entry.sha256 == Digest::from_sha256(item.sha256).hex() && entry.size == item.size
            })
        {
            return Err(ApiError::new(
                ErrorKind::Unsupported,
                format!(
                    "{}: bound firmware is not in the reviewed catalogue",
                    item.slot_id
                ),
            ));
        }
        let digest = Digest::from_sha256(item.sha256);
        if let Some(row) = launch.content.iter().find(|r| r.digest == digest) {
            if row.sha256 != item.sha256 || row.size != item.size {
                return Err(config(
                    "resolved content conflicts with a preserved receipt",
                ));
            }
        } else {
            launch.content.push(ContentRow {
                digest,
                sha256: item.sha256,
                size: item.size,
            });
        }
    }
    let mut hardware = image::encode::machine(&resolved.document);
    let mut firmware = Vec::new();
    firmware_images(&mut hardware, |key, rom| -> image::Result<()> {
        let source = rom
            .get("image")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| invalid("firmware requires a logical image"))?;
        if let Some(item) = resolved.items.iter().find(|i| i.key == source) {
            firmware.push(Firmware {
                key: key.into(),
                source: source.into(),
                blob: Digest::from_sha256(item.sha256),
                source_archive: None,
            });
        } else {
            firmware.push(
                original
                    .firmware
                    .iter()
                    .find(|f| f.key == key && f.source == source)
                    .cloned()
                    .ok_or_else(|| invalid("missing preserved firmware binding"))?,
            );
        }
        Ok(())
    })
    .map_err(config)?;
    if original.firmware.iter().any(|f| {
        !firmware
            .iter()
            .any(|v| v.key == f.key && v.source == f.source && v.blob == f.blob)
    }) {
        return Err(config("unused preserved firmware binding"));
    }
    let mut media = original.media.clone();
    let mut slots = Vec::new();
    let mut records = launch.records.clone();
    let mut drive_specs = Vec::new();
    for (i, card) in resolved.document.card.iter().flatten().enumerate() {
        let mut drives: Vec<_> = card.drives.iter().flatten().enumerate().collect();
        drives.sort_by_key(|(_, d)| {
            d.unit
                .unwrap_or(u8::from(d.channel == Some(schema::Channel::Slave)))
        });
        for (j, drive) in drives {
            drive_specs.push((i, j, drive));
        }
    }
    let mut specs = drive_specs.into_iter();
    drive_tables(
        &mut hardware,
        |controller, kind, table| -> image::Result<()> {
            let (i, j, drive) = specs
                .next()
                .ok_or_else(|| invalid("missing drive specification"))?;
            let unit = image::unit(kind, table)?;
            if drive.image.is_none()
                && drive.disk.is_none()
                && drive.media.is_none()
                && drive.empty != Some(true)
            {
                if let Some(slot) = original
                    .slots
                    .iter()
                    .find(|s| s.controller == controller && s.unit == unit)
                {
                    if drive.write.is_some()
                        || drive.sha256.is_some()
                        || drive.journal.is_some()
                        || drive.keep_generations.is_some()
                    {
                        return Err(invalid("resolved slot hardware contains media fields"));
                    }
                    slots.push(slot.clone());
                    return Ok(());
                }
            }
            let mut ids = Vec::new();
            let catalogue_mode = drive.media.is_some() || drive.empty == Some(true);
            let sources: Vec<_> = if let Some(list) = &drive.media {
                list.iter()
                    .enumerate()
                    .map(|(k, m)| {
                        (
                            Some(k),
                            Some(m.image.as_str()),
                            m.write,
                            m.keep_generations,
                            m.journal.as_ref(),
                        )
                    })
                    .collect()
            } else if drive.empty == Some(true) {
                Vec::new()
            } else {
                vec![(
                    None,
                    drive.image.as_deref(),
                    drive.write,
                    drive.keep_generations,
                    drive.journal.as_ref(),
                )]
            };
            for (k, source, mode, keep, journal) in sources {
                if journal.is_some() {
                    return Err(invalid("host journal paths are not portable"));
                }
                if let Some(source) =
                    source.filter(|source| !resolved.items.iter().any(|i| i.key == *source))
                {
                    let preserved = original
                        .media
                        .iter()
                        .find(|m| m.source == source)
                        .filter(|m| {
                            original.slots.iter().any(|s| {
                                s.controller == controller
                                    && s.unit == unit
                                    && s.catalogue.contains(&m.id)
                            })
                        })
                        .ok_or_else(|| {
                            invalid(
                                "path media require a preserved logical source and slot binding",
                            )
                        })?;
                    if let Some(assertion) = if let Some(k) = k {
                        drive
                            .media
                            .as_ref()
                            .and_then(|ms| ms.get(k))
                            .and_then(|m| m.sha256.as_ref())
                    } else {
                        drive.sha256.as_ref()
                    } {
                        let row = preserved
                            .head
                            .and_then(|h| launch.content.iter().find(|r| r.digest == h))
                            .ok_or_else(|| invalid("missing preserved medium receipt"))?;
                        if assertion.as_str() != Digest::from_sha256(row.sha256).hex() {
                            return Err(invalid("medium sha256 differs from preserved receipt"));
                        }
                    }
                    if mode.is_some_and(|mode| match mode {
                        schema::WriteMode::ReadOnly => preserved.policy != Policy::Readonly,
                        schema::WriteMode::Volatile => preserved.policy != Policy::Volatile,
                        schema::WriteMode::Journal => preserved.policy != Policy::Journal,
                    }) {
                        return Err(invalid("medium policy differs from preserved binding"));
                    }
                    ids.push(preserved.id.clone());
                    continue;
                }
                let id = drive.disk.clone().unwrap_or_else(|| match k {
                    Some(k) => format!("disk-{i}-{j}-{k}"),
                    None => format!("disk-{i}-{j}"),
                });
                if media.iter().any(|m| m.id == id) {
                    return Err(invalid(
                        "generated medium id collides with a preserved medium",
                    ));
                }
                let policy = match mode.unwrap_or(schema::WriteMode::Volatile) {
                    schema::WriteMode::ReadOnly => Policy::Readonly,
                    schema::WriteMode::Volatile => Policy::Volatile,
                    schema::WriteMode::Journal => Policy::Journal,
                };
                let (source, head, zero, size) = if let Some(disk) = &drive.disk {
                    let size = resolved
                        .zero_slots
                        .iter()
                        .find(|s| s.item_id.as_ref() == Some(disk))
                        .and_then(|s| s.size)
                        .ok_or_else(|| invalid("missing zero declaration"))?;
                    (
                        format!("assets/disks/{disk}"),
                        None,
                        Some(Zero { size }),
                        size,
                    )
                } else {
                    let source = source.ok_or_else(|| invalid("missing logical medium source"))?;
                    let item = resolved
                        .items
                        .iter()
                        .find(|item| item.key == source)
                        .ok_or_else(|| invalid("path media must have preserved slot bindings"))?;
                    (
                        source.into(),
                        Some(Digest::from_sha256(item.sha256)),
                        None,
                        item.size,
                    )
                };
                let geometry = if kind == "ide" {
                    let g = table
                        .get("geometry")
                        .and_then(toml::Value::as_table)
                        .ok_or_else(|| invalid("missing IDE geometry"))?;
                    let n = |key| {
                        g.get(key)
                            .and_then(toml::Value::as_integer)
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or_else(|| invalid("invalid IDE geometry"))
                    };
                    Geometry {
                        cylinders: n("cylinders")?,
                        heads: n("heads")?,
                        sectors: n("sectors")?,
                        sector_bytes: 512,
                    }
                } else {
                    let geometry = match drive.drive_type.unwrap_or(schema::DriveType::Hd144) {
                        schema::DriveType::Hd144 => assembly::FloppyGeometry::HD_1_44,
                        schema::DriveType::Hd12 => assembly::FloppyGeometry::HD_1_2,
                        schema::DriveType::Dd720 => assembly::FloppyGeometry::DD_720K,
                        schema::DriveType::Dd360 => assembly::FloppyGeometry::DD_360K,
                    };
                    Geometry::of_medium(assembly::DriveKind::Floppy { geometry }, size)?
                };
                media.push(Medium {
                    id: id.clone(),
                    source,
                    head,
                    zero,
                    name: catalogue_mode.then(|| format!("Disk {}", k.unwrap_or(0) + 1)),
                    policy,
                    geometry,
                });
                if policy == Policy::Journal {
                    records.push(RecordRow {
                        medium: id.clone(),
                        name: format!("{id}.upfj"),
                        keep_generations: keep
                            .map_or(assembly::WriteMode::DEFAULT_KEEP_GENERATIONS, |k| k.get()),
                    });
                }
                ids.push(id);
            }
            slots.push(Slot {
                controller: controller.into(),
                unit,
                inserted: if drive.empty == Some(true) {
                    None
                } else {
                    ids.first().cloned()
                },
                catalogue: ids,
                catalogue_mode,
            });
            Ok(())
        },
    )
    .map_err(config)?;
    if original.slots.iter().any(|s| {
        !slots
            .iter()
            .any(|v| v.controller == s.controller && v.unit == s.unit && v.catalogue == s.catalogue)
    }) {
        return Err(config("unused preserved slot binding"));
    }
    launch.config.machine = hardware;
    launch.config.firmware = firmware;
    launch.config.media = media;
    launch.config.slots = slots;
    launch.records = records;
    let checked = run(&launch, None, assembly::AssemblyOptions::default()).map_err(config)?;
    let mut seen = BTreeSet::new();
    launch.content = launch
        .config
        .firmware
        .iter()
        .map(|f| f.blob)
        .chain(launch.config.media.iter().filter_map(|m| m.head))
        .chain(launch.config.nvram.iter().map(|n| n.head))
        .filter(|digest| seen.insert(*digest))
        .map(|digest| {
            launch
                .content
                .iter()
                .find(|r| r.digest == digest)
                .cloned()
                .ok_or_else(|| config("missing resolved content row"))
        })
        .collect::<std::result::Result<_, _>>()?;
    image::defaults::materialize(&mut launch.config.machine, &checked.machine).map_err(config)?;
    check_portable(&launch).map_err(config)?;
    Ok(launch)
}
