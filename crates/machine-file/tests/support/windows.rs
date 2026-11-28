// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared private Windows staging on an Intel486 ISA board;
//! user-media.md §Windows 95 and §Windows 3.1.
//! ATA-2, Command Register; IBM PS/2 Hardware Interface Technical Reference,
//! Common Interfaces, Keyboard and Auxiliary Device Controller;
//! IBM VGA/XGA Technical Reference, Video BIOS mode 12h;
//! Intel 8259A, ICW/OCW programming; Intel 8254, Modes 0–5;
//! Intel 8237A, register set; NEC uPD765A, command phase.

use assembly::{DeviceId, DeviceKind, DriveKind, ImageSource, WriteMode};
use cpu::Cpu;
use device::{AccessDir, TraceEvent};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Command,
    time::Instant,
};

#[path = "private_diagnostics.rs"]
mod private_diagnostics;

/// Every progress and findings line of the runner goes through [`emit`].
macro_rules! say {
    ($($arg:tt)*) => {
        emit(format!($($arg)*))
    };
}

/// The product key is written only into the generated batch file on the
/// temporary disk and typed as digits on setup's certificate page. A line that
/// would carry it, with or without its dashes and in any case, or carry its
/// digit groups joined as they are typed, is withheld and fails the run
/// without being printed.
fn emit(line: String) {
    let key = std::env::var("UNEMUPC_USER_WIN95_KEY").ok();
    assert!(
        !carries_secret(&line, key.as_deref()),
        "a diagnostic line would carry the product key; withheld"
    );
    eprintln!("{line}");
}

fn carries_secret(line: &str, secret: Option<&str>) -> bool {
    let fold = |text: &str| -> String {
        text.chars()
            .filter(|c| *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect()
    };
    let Some(secret) = secret else {
        return false;
    };
    let folded = fold(line);
    let key = fold(secret);
    // The digit groups alone, as certificate_keys types them. A single group
    // is not checked: five digits turn up in ordinary retired counts.
    let digits: String = secret
        .split('-')
        .filter(|group| !group.is_empty() && group.bytes().all(|byte| byte.is_ascii_digit()))
        .collect();
    (!key.is_empty() && (line.to_ascii_uppercase().contains(&key) || folded.contains(&key)))
        || (digits.len() >= 10 && folded.contains(&digits))
}

#[derive(Default, Debug)]
struct Ports {
    accesses: BTreeMap<&'static str, u64>,
    writes: BTreeMap<(&'static str, u16, u32), u64>,
    aux_data_next: bool,
    pic_special_mask: u64,
    pic_poll: u64,
    /// Command ports that have seen an OCW3 with ESMM set.
    pic_special_mask_ports: BTreeSet<u16>,
    /// PIC command-port writes made after that controller's first ESMM write,
    /// by port and command byte; tells whether ICW1 or OCW2 followed it.
    pic_after_special_mask: BTreeMap<(u16, u32), u64>,
    pit_modes: BTreeMap<(u8, u8), u64>,
    vga_indices: BTreeMap<u16, u8>,
    vga_registers: BTreeMap<u16, BTreeSet<u8>>,
    attribute_data: bool,
    fdc_parameters: u8,
    last_access: Option<(u16, AccessDir, u8)>,
    /// Board reset signals written so far, by kind: the 8042's pulse-reset
    /// command FEh (IBM PS/2 Hardware Interface Technical Reference, Keyboard
    /// and Auxiliary Device Controller, command FEh), the fast reset bit of
    /// system control port A (92h bit 0), and the reset control register's
    /// reset bit (CF9h bit 2). A processor shutdown asks for a reset without
    /// any of them.
    reset_signals: BTreeMap<&'static str, u64>,
    recent: Vec<String>,
}

impl Ports {
    fn observe(&mut self, mut events: Vec<TraceEvent>) {
        events.retain(|event| matches!(event, TraceEvent::Port { .. }));
        events.sort_by_key(|event| match event {
            TraceEvent::Port { at, .. } => *at,
            _ => unreachable!(),
        });
        for event in &events {
            let TraceEvent::Port {
                port,
                dir,
                value,
                size,
                ..
            } = event
            else {
                unreachable!()
            };
            self.last_access = Some((*port, *dir, *size));
            let device = match port {
                0x1f0..=0x1f7 | 0x170..=0x177 | 0x376 | 0x3f6 => "ATA",
                0x60 | 0x64 => "8042",
                0x20..=0x21 | 0xa0..=0xa1 => "PIC",
                0x40..=0x43 => "PIT",
                0x3b0..=0x3df => "VGA",
                0x00..=0x0f | 0x80..=0x8f | 0xc0..=0xde => "DMA",
                0x3f0..=0x3f5 | 0x3f7 => "FDC",
                _ => "other",
            };
            *self.accesses.entry(device).or_default() += 1;
            if matches!(port, 0x3ba | 0x3da) && *dir == AccessDir::Read {
                self.attribute_data = false;
            }
            if *dir != AccessDir::Write {
                continue;
            }
            let reset_signal = match port {
                0x64 if *value == 0xfe => Some("8042 FEh"),
                0x92 if value & 1 != 0 => Some("port 92h"),
                0xcf9 if value & 4 != 0 => Some("CF9h"),
                _ => None,
            };
            if let Some(kind) = reset_signal {
                *self.reset_signals.entry(kind).or_default() += 1;
            }
            if matches!(port, 0x20 | 0xa0) && self.pic_special_mask_ports.contains(port) {
                *self
                    .pic_after_special_mask
                    .entry((*port, *value))
                    .or_default() += 1;
            }
            if matches!(port, 0x20 | 0xa0) && value & 0x18 == 8 {
                self.pic_special_mask += u64::from(value & 0x40 != 0);
                self.pic_poll += u64::from(value & 4 != 0);
                if value & 0x40 != 0 {
                    self.pic_special_mask_ports.insert(*port);
                }
            }
            if *port == 0x43 && value >> 6 != 3 && value & 0x30 != 0 {
                let mode = ((value >> 1) & 7) as u8;
                *self
                    .pit_modes
                    .entry(((value >> 6) as u8, if mode >= 6 { mode - 4 } else { mode }))
                    .or_default() += 1;
            }
            match *port {
                0x3c4 | 0x3ce | 0x3b4 | 0x3d4 => {
                    self.vga_indices.insert(*port, *value as u8);
                    self.vga_registers
                        .entry(*port)
                        .or_default()
                        .insert(*value as u8);
                }
                0x3c5 | 0x3cf | 0x3b5 | 0x3d5 => {
                    if let Some(index) = self.vga_indices.get(&(port - 1)) {
                        self.vga_registers
                            .entry(port - 1)
                            .or_default()
                            .insert(*index);
                    }
                }
                0x3c0 => {
                    if !self.attribute_data {
                        self.vga_registers
                            .entry(*port)
                            .or_default()
                            .insert((*value & 0x1f) as u8);
                    }
                    self.attribute_data = !self.attribute_data;
                }
                _ => {}
            }
            let command = match port {
                0x1f7 | 0x177 => Some("ATA command"),
                0x64 => {
                    self.aux_data_next = *value == 0xd4;
                    Some("8042 command")
                }
                0x60 if self.aux_data_next => {
                    self.aux_data_next = false;
                    Some("8042 aux data")
                }
                0x20 | 0xa0 if value & 0x10 != 0 => Some("PIC ICW1"),
                0x20 | 0xa0 if value & 0x18 == 8 => Some("PIC OCW3"),
                0x20 | 0xa0 => Some("PIC OCW2"),
                0x21 | 0xa1 => Some("PIC data (ICW2-4/OCW1)"),
                0x43 => Some("PIT control"),
                _ if device == "DMA" => Some(device),
                0x3f2 => {
                    // A digital-output write with the reset bit clear aborts
                    // any command in progress.
                    if value & 4 == 0 {
                        self.fdc_parameters = 0;
                    }
                    Some("FDC digital output")
                }
                0x3f4 => Some("FDC data rate"),
                0x3f5 if self.fdc_parameters > 0 => {
                    self.fdc_parameters -= 1;
                    None
                }
                0x3f5 => {
                    self.fdc_parameters = fdc_parameter_bytes(*value as u8);
                    Some("FDC command")
                }
                _ => None,
            };
            if let Some(command) = command {
                *self.writes.entry((command, *port, *value)).or_default() += 1;
            }
        }
        // ATA data words can contain batch-file secrets, and the 8042 data
        // port carries typed keys, the product key among them, as scancodes.
        // Retain access metadata without retaining payloads in any printable
        // diagnostic structure.
        self.recent.extend(events.iter().skip(events.len().saturating_sub(64)).map(|event| {
            match event {
                TraceEvent::Port { at, port: port @ (0x1f0 | 0x170 | 0x60), size, dir, .. } =>
                    format!("Port {{ at: {at:?}, port: {port:04x}, size: {size}, dir: {dir:?}, value: [payload omitted] }}"),
                _ => format!("{event:x?}"),
            }
        }));
        let excess = self.recent.len().saturating_sub(64);
        self.recent.drain(..excess);
    }
}

/// Parameter bytes that follow a command byte written to the data register.
/// NEC uPD765A datasheet, command summary; the configure, perpendicular, lock,
/// verify and version commands follow the Intel 82077AA datasheet. Unknown
/// commands take none, as the controller answers them with an invalid status.
fn fdc_parameter_bytes(command: u8) -> u8 {
    match command & 0x1f {
        0x02 | 0x05 | 0x06 | 0x09 | 0x0c | 0x11 | 0x16 | 0x19 | 0x1d => 8,
        0x0d => 5,
        0x13 => 3,
        0x03 | 0x0f => 2,
        0x04 | 0x07 | 0x0a | 0x12 => 1,
        _ => 0,
    }
}

fn command_keys(command: &str) -> Vec<u8> {
    let mut keys = Vec::new();
    let rows = [
        ("1234567890", 0x02),
        ("qwertyuiop", 0x10),
        ("asdfghjkl", 0x1e),
        ("zxcvbnm", 0x2c),
    ];
    for c in command.chars() {
        let (code, shift) = match c {
            ':' => (0x27, true),
            '\\' => (0x2b, false),
            ' ' => (0x39, false),
            '/' => (0x35, false),
            '.' => (0x34, false),
            '\n' => (0x1c, false),
            _ => (
                rows.iter()
                    .find_map(|(row, start)| row.find(c).map(|offset| start + offset as u8))
                    .expect("supported command key"),
                false,
            ),
        };
        if shift {
            keys.push(0x2a);
        }
        keys.extend([code, code | 0x80]);
        if shift {
            keys.push(0xaa);
        }
    }
    keys
}

/// One private Windows stage. Each runs in a fresh machine; the stages after
/// the first mount the disk generation the previous stage closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Windows 95 setup from the DOS floppy, up to setup's first screen.
    Win95,
    /// The same setup run on, through hardware detection and file copy, up to
    /// the restart it requests at the end of its DOS-hosted phase.
    Win95FileCopy,
    /// The first boot from C: after file copy: setup's post-restart phase
    /// to the desktop, then the pointer, the Start menu and Shut Down.
    Win95FirstBoot,
    /// A second boot from the generation the first boot closed: the desktop
    /// again, then the same pointer, Start menu and Shut Down script.
    Win95SecondBoot,
    /// SYS C: from the DOS floppy, plus our startup files and XMS driver.
    SystemTransfer,
    /// Windows 3.1 SETUP /H from C:, back to DOS with WIN386.EXE installed.
    Install,
    /// WIN /S from C: (16-bit protected mode without paging), then a DOS
    /// prompt opened from Program Manager's File/Run and closed with EXIT.
    Standard,
    /// WIN /3 from C: (paging and V86 under WIN386), then the same DOS prompt
    /// round trip, with the prompt seen in paged V86 mode.
    Enhanced,
    /// WIN /3 under a DOS 7 kernel, which refuses it before paging.
    EnhancedRefusedByDos7,
}

impl Stage {
    fn command(self) -> &'static str {
        match self {
            Self::Win95 | Self::Win95FileCopy => "c:\\win95\\setup /is /iq c:\\msbatch.inf\n",
            // Windows 95 starts from its own startup files; nothing is typed.
            Self::Win95FirstBoot | Self::Win95SecondBoot => "",
            Self::SystemTransfer => "c:\\sys31.bat\n",
            Self::Install => "c:\\inst31.bat\n",
            Self::Standard => "c:\\windows\\win /s c:\\windows\\progman.exe\n",
            Self::Enhanced | Self::EnhancedRefusedByDos7 => {
                "c:\\windows\\win /3 c:\\windows\\progman.exe\n"
            }
        }
    }

    /// Stages after setup boot C: with no floppy mounted, because setup
    /// rewrites the startup files of the drive the system booted from.
    fn boots_hard_disk(self) -> bool {
        matches!(
            self,
            Self::Install
                | Self::Standard
                | Self::Enhanced
                | Self::EnhancedRefusedByDos7
                | Self::Win95FirstBoot
                | Self::Win95SecondBoot
        )
    }

    fn windows(self) -> bool {
        matches!(self, Self::Standard | Self::Enhanced)
    }

    fn win95(self) -> bool {
        matches!(self, Self::Win95 | Self::Win95FileCopy) || self.win95_boot()
    }

    /// Stages that boot an installed Windows 95 rather than type a command.
    fn win95_boot(self) -> bool {
        matches!(self, Self::Win95FirstBoot | Self::Win95SecondBoot)
    }
}

/// The private disk after a stage, and whether stage 1 found a DOS 7 kernel
/// (its batch file prints a marker in the DOS 7 branch).
pub struct Staged {
    pub image: PathBuf,
    pub dos7: bool,
}

fn decoded_text(frame: &video::Frame) -> String {
    match frame {
        video::Frame::Text(grid) => grid.rows_as_strings().join("\n"),
        _ => String::new(),
    }
}

/// A digest of the captured graphics frame, to tell a settled screen from one
/// still being drawn. `None` in text mode and for direct-colour frames.
fn frame_digest(frame: &video::Frame) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    match frame {
        video::Frame::Pixels(pixels) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            pixels.indices.hash(&mut hasher);
            pixels.palette.hash(&mut hasher);
            Some(hasher.finish())
        }
        video::Frame::Text(_) | video::Frame::Direct(_) => None,
    }
}

fn first_screen(
    assembled: &assembly::AssembledMachine,
    frame: &video::Frame,
    text: &str,
    stage: Stage,
) -> bool {
    let mut mode = [0];
    assembled.machine.cpu().read_mem(0x449, &mut mode).unwrap();
    if let video::Frame::Pixels(pixels) = frame {
        let mut counts = [0usize; 256];
        for index in &pixels.indices {
            counts[*index as usize] += 1;
        }
        let distinct = counts.iter().filter(|count| **count != 0).count();
        let minority = pixels.indices.len() - counts.into_iter().max().unwrap_or(0);
        return (stage != Stage::Win95 || mode[0] == 0x12)
            && (pixels.width, pixels.height) == (640, 480)
            && distinct >= 3
            && minority >= 4096;
    }
    // These short identifiers qualify a full-screen setup dialog, rather than
    // the shell's echoed command or the real-mode initialization banner.
    text.contains("Windows 95 Setup")
        && (text.contains("Continue") || text.contains("ENTER"))
        && text.lines().filter(|line| !line.trim().is_empty()).count() >= 8
}

fn installed_choices(ini: &str) -> BTreeMap<&'static str, bool> {
    let mut section = String::new();
    let mut entries = BTreeMap::new();
    for line in ini
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with(';'))
    {
        if line.starts_with('[') {
            section = line.to_ascii_lowercase();
        } else if let Some((key, value)) = line.split_once('=') {
            entries.insert(
                (section.clone(), key.trim().to_ascii_lowercase()),
                value.trim().to_ascii_lowercase(),
            );
        }
    }
    let value = |section: &str, key: &str| {
        entries
            .get(&(section.into(), key.into()))
            .map(String::as_str)
    };
    [
        (
            "standard VGA",
            value("[boot]", "display.drv") == Some("vga.drv"),
        ),
        (
            "PS/2 mouse description",
            value("[boot.description]", "mouse.drv").is_some_and(|s| s.contains("ps/2")),
        ),
        (
            "Program Manager shell",
            value("[boot]", "shell") == Some("progman.exe"),
        ),
        (
            "network disabled",
            value("[boot]", "network.drv") == Some(""),
        ),
        (
            "386Enh settings",
            ini.to_ascii_lowercase().contains("[386enh]"),
        ),
    ]
    .into()
}

/// A private copy of the merged sector view of the IDE master, read through
/// the drive's existing overlay. mtools reads this new snapshot, never the
/// immutable mounted base. The directory is removed when it is dropped.
fn disk_snapshot(
    assembled: &mut assembly::AssembledMachine,
) -> (machine_file::test_support::TempDir, PathBuf) {
    let bytes = assembled
        .machine
        .bus_mut()
        .device_mut(assembled.device_index[&DeviceId::Ide])
        .unwrap()
        .as_any_mut()
        .unwrap()
        .downcast_mut::<devices::Ata>()
        .unwrap()
        .drive(0)
        .unwrap()
        .image();
    let directory = machine_file::test_support::TempDir::new("private-windows-inspection");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private inspection directory permissions failed");
    }
    let snapshot = directory.path().join("inspection.img");
    std::fs::write(&snapshot, bytes).expect("private inspection snapshot failed");
    (directory, snapshot)
}

/// Read one file of the first partition (it starts at sector 63) from a
/// snapshot. `None` when it is absent.
fn snapshot_file(snapshot: &std::path::Path, path: &str) -> Option<Vec<u8>> {
    let output = Command::new("mtype")
        .arg("-i")
        .arg(format!("{}@@32256", snapshot.display()))
        .arg(format!("::/{path}"))
        .env("MTOOLSRC", "/dev/null")
        // A volume Windows has mounted carries its "dirty" bit in the second
        // FAT entry, which mtools otherwise refuses as a bad FAT.
        .env("MTOOLS_SKIP_CHECK", "1")
        .output()
        .expect("start private file inspection");
    output.status.success().then_some(output.stdout)
}

fn report_installed_choices(assembled: &mut assembly::AssembledMachine) {
    let (_directory, snapshot) = disk_snapshot(assembled);
    let ini = snapshot_file(&snapshot, "WINDOWS/SYSTEM.INI");
    say!("FINDINGS SYSTEM.INI readable={}", ini.is_some());
    if let Some(ini) = ini {
        say!(
            "FINDINGS installed choices (derived, no source text): {:?}",
            installed_choices(&String::from_utf8_lossy(&ini))
        );
    }
}

/// Files the DOS-hosted phase of Windows 95 setup leaves for the first boot.
const WIN95_INSTALLED_FILES: [&str; 8] = [
    "IO.SYS",
    "MSDOS.SYS",
    "COMMAND.COM",
    "WINDOWS/WIN.COM",
    "WINDOWS/SYSTEM.DAT",
    "WINDOWS/USER.DAT",
    "WINDOWS/SYSTEM/VMM32.VXD",
    "WINDOWS/SYSTEM.INI",
];

/// Booleans for the files and boot records setup must have written, plus the
/// hardware identifiers its detection log names. Returns whether every file
/// in [`WIN95_INSTALLED_FILES`] is present and both boot records are setup's.
fn report_win95_files(assembled: &mut assembly::AssembledMachine) -> bool {
    let (_directory, snapshot) = disk_snapshot(assembled);
    let present: BTreeMap<&str, bool> = WIN95_INSTALLED_FILES
        .iter()
        .map(|path| (*path, snapshot_file(&snapshot, path).is_some()))
        .collect();
    say!("FINDINGS installed files present: {present:?}");
    // The master boot record and the partition's boot record (sector 63)
    // both lie in the first 32 KiB.
    let mut head = [0u8; 32 * 1024];
    {
        use std::io::Read;
        std::fs::File::open(&snapshot)
            .and_then(|mut file| file.read_exact(&mut head))
            .expect("private inspection snapshot unreadable");
    }
    // The builder leaves the master boot record without code and writes its
    // own OEM name into the volume boot record.
    let mbr_code = head[..446].iter().any(|byte| *byte != 0);
    let vbr_replaced = &head[63 * 512 + 3..63 * 512 + 11] != b"UNEMUPC ";
    say!("FINDINGS boot records: MBR code present={mbr_code}; VBR replaced={vbr_replaced}");
    for log in ["SETUPLOG.TXT", "DETLOG.TXT", "BOOTLOG.TXT"] {
        say!(
            "FINDINGS {log} present={}",
            snapshot_file(&snapshot, log).is_some()
        );
    }
    if let Some(log) = snapshot_file(&snapshot, "DETLOG.TXT") {
        say!(
            "FINDINGS detected device identifiers: {:?}",
            detected_ids(&String::from_utf8_lossy(&log))
        );
    }
    present.values().all(|present| *present) && mbr_code && vbr_replaced
}

/// Plug and Play identifiers (three letters and four hexadecimal digits, as
/// the Plug and Play ISA Specification, Appendix A, writes them) from the
/// detection log's "Detected:" lines. Device descriptions are left out.
fn detected_ids(log: &str) -> BTreeSet<String> {
    log.lines()
        .filter_map(|line| line.trim().strip_prefix("Detected:"))
        .filter_map(|rest| {
            let id = rest.trim().strip_prefix('*').unwrap_or(rest.trim());
            let id: String = id.chars().take(7).collect();
            let bytes = id.as_bytes();
            (bytes.len() == 7
                && bytes[..3].iter().all(u8::is_ascii_uppercase)
                && bytes[3..].iter().all(u8::is_ascii_hexdigit))
            .then(|| format!("*{id}"))
        })
        .collect()
}

/// The delivery log reduced to totals and, for the latest entries, their
/// kind, vector, saved address, error code and privilege/VM transition. The
/// bytes at the saved address and the interrupted registers are guest data
/// and are left out.
fn delivery_summary(log: Option<machine::RecentDeliveries>) -> String {
    let Some(log) = log else {
        return "log off".into();
    };
    let mut text = format!("total={}; dropped={}", log.total, log.dropped);
    for entry in log.entries.iter().skip(log.entries.len().saturating_sub(8)) {
        text += &match entry {
            machine::DeliveryEntry::Deliveries { count, last, .. } => format!(
                "; x{count} {} vector={:02x} at {:04x}:{:08x} linear={:08x} error={:x?} CPL {}->{} VM {}->{}",
                last.kind.word(),
                last.vector,
                last.cs,
                last.ip,
                last.linear,
                last.error_code,
                last.cpl_before,
                last.cpl_after,
                last.virtual_8086_before,
                last.virtual_8086_after
            ),
            machine::DeliveryEntry::Dropped(count) => format!("; dropped {count}"),
        };
    }
    text
}

/// Keys for the dialogs Windows 95 OSR2 setup still shows with our batch
/// file, in the order it shows them. Each goes out once the setup frame has
/// held (all but a caret's pixels) with no disk command at all for
/// [`WIN95_DIALOG_HOLD`] instructions; setup's long phases behind a still
/// frame keep the disk busy.
const WIN95_ANSWERS: [(&str, Answer); 3] = [
    // No batch key accepts the OEM licence agreement. Alt+Y picks its Yes
    // button whichever control has the focus.
    (
        "licence agreement: Alt+Y",
        Answer::Keys(&[0x38, 0x15, 0x95, 0xb8]),
    ),
    // The OEM certificate page does not take the batch file's ProductID: setup
    // presses its Next button with the boxes empty and refuses the number.
    // ENTER picks the refusal's default Re-enter button.
    (
        "certificate refusal: ENTER for Re-enter",
        Answer::Keys(&[0x1c, 0x9c]),
    ),
    // One box per digit group; a full box moves the focus to the next one by
    // itself, so a TAB would skip a box. ENTER picks the default Next button.
    (
        "certificate page: digit groups, then ENTER for Next",
        Answer::Certificate,
    ),
];

/// What the runner types at a held setup dialog.
#[derive(Clone, Copy)]
enum Answer {
    /// Fixed scancodes, make and break.
    Keys(&'static [u8]),
    /// The digit groups of the product key, then ENTER. The key itself
    /// never reaches a log line.
    Certificate,
}

impl Answer {
    fn scancodes(self) -> Vec<u8> {
        match self {
            Self::Keys(keys) => keys.to_vec(),
            Self::Certificate => {
                certificate_keys(&std::env::var("UNEMUPC_USER_WIN95_KEY").unwrap_or_default())
            }
        }
    }
}

/// Scancodes for an OEM certificate number: its digit groups back to back,
/// then ENTER. The "OEM" group is printed on the page.
fn certificate_keys(key: &str) -> Vec<u8> {
    let digits: String = key
        .split('-')
        .filter(|group| group.bytes().all(|byte| byte.is_ascii_digit()))
        .collect();
    let mut keys = command_keys(&digits);
    keys.extend([0x1c, 0x9c]);
    keys
}

/// How long a setup frame must hold before it is taken as a dialog waiting
/// for a key. Progress pages redraw well within this.
const WIN95_DIALOG_HOLD: u64 = 30_000_000;

/// Pixels a held setup frame may differ by from where the hold began.
const WIN95_CARET_PIXELS: usize = 64;

/// Run slices (of ten thousand virtual-time units each) between two screen
/// captures: about one million guest instructions.
const SCREEN_POLL_SLICES: u64 = 100;

/// The stage's machine: generic-486-ide with the PS/2 mouse, the IDE master
/// on `image` with its journal in `directory`, and, for the stages that boot
/// a floppy, the DOS floppy read-only. The delivery log is on.
fn assemble_stage(
    stage: Stage,
    image: &std::path::Path,
    floppy: &std::ffi::OsStr,
    directory: &std::path::Path,
) -> assembly::AssembledMachine {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut config = machine_file::load_for_assembly(
        root.join("machines/generic-486-ide.toml"),
        Default::default(),
    )
    .unwrap()
    .config;
    if stage.boots_hard_disk() {
        config
            .drives
            .retain(|drive| matches!(drive.kind, DriveKind::HardDisk { .. }));
    }
    for device in &mut config.devices {
        if let DeviceKind::Keyboard(kbc) = &mut device.kind {
            kbc.mouse = true;
        }
    }
    for drive in &mut config.drives {
        let path = if matches!(drive.kind, DriveKind::HardDisk { .. }) {
            drive.kind = DriveKind::HardDisk {
                geometry: assembly::HardDiskGeometry {
                    cylinders: 1015,
                    heads: 16,
                    sectors_per_track: 63,
                    bytes_per_sector: 512,
                },
            };
            drive.write = WriteMode::journal(directory.join("ide.journal"));
            image.to_path_buf()
        } else {
            drive.write = WriteMode::ReadOnly;
            PathBuf::from(floppy)
        };
        // Hash every byte, including sparse extents. Neither digest is printed.
        let mut file =
            std::fs::File::open(&path).unwrap_or_else(|_| panic!("private media open failed"));
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash).unwrap();
        drive.expected_sha256 = Some(hash.finalize().into());
        drive.image = ImageSource::File(path);
    }
    let policy = assembly::ObservationPolicy {
        trace: device::TracePolicy::Ring { capacity: 65_536 },
        routes: vec![assembly::ObservationRequest {
            device: DeviceId::Vga,
            output: assembly::OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    };
    let mut assembled = machine_file::assemble_with_observation(&config, &policy)
        .unwrap_or_else(|_| panic!("private media assembly failed"));
    for handle in &assembled.persistence {
        let generation = handle.mount_report().last_closed_generation;
        say!("{stage:?} mounted journal generation={generation:?}");
        if stage.boots_hard_disk() {
            assert!(
                generation.is_some_and(|id| id >= 1),
                "a prior stage must close its journal"
            );
        }
    }
    assembled.machine.enable_delivery_log().unwrap();
    assembled
}

/// One run slice of ten thousand virtual-time units, with every device trace
/// drained into `ports` and the journal output checked.
fn run_slice(
    assembled: &mut assembly::AssembledMachine,
    ports: &mut Ports,
) -> Result<machine::RunOutcome, machine::MachineError> {
    let cap = assembled.machine.virtual_time() + 10_000;
    let result = assembled.machine.run_bounded(Some(cap));
    let mut events = assembled.machine.bus_mut().take_open_bus_trace();
    for index in assembled.device_index.values() {
        let device = assembled.machine.bus_mut().device_mut(*index).unwrap();
        assert_eq!(
            device.trace_dropped(),
            0,
            "private evidence trace overflowed"
        );
        events.extend(device.take_trace());
    }
    ports.observe(events);
    assembled
        .check_persistence()
        .expect("private journal output failed");
    result
}

/// The findings every stage prints at its end: scanout shape, CPU mode and
/// place, port counters and register writes, 8259 state and deliveries.
fn report_stage_end(
    assembled: &mut assembly::AssembledMachine,
    ports: &Ports,
    reason: &str,
    started: Instant,
) {
    if let video::Frame::Pixels(pixels) = assembled.capture_text(DeviceId::Vga).unwrap().frame {
        let mut palette = BTreeSet::new();
        palette.extend(pixels.indices.iter().copied());
        let mut mode = [0];
        assembled.machine.cpu().read_mem(0x449, &mut mode).unwrap();
        say!(
            "FINDINGS scanout: {}x{}; distinct indices={}; BDA mode={:02x}",
            pixels.width,
            pixels.height,
            palette.len(),
            mode[0]
        );
    }
    let state = assembled.machine.cpu().diagnostic_state();
    let system = state.system.unwrap();
    let mode = if system.virtual_8086 {
        "V86"
    } else if system.protected() {
        "protected"
    } else {
        "real"
    };
    let eip = assembled.machine.cpu().reg(cpu::CpuReg::Rip);
    let linear = state.cs().base.wrapping_add(eip as u32);
    say!("FINDINGS: {reason}; mode={mode}; CS:EIP={:04x}:{:08x}; linear={:08x}; CPL={}; CR0={:08x}; retired={}; wall={:?}", state.cs().selector, eip, linear, system.cpl, system.cr0, state.retired, started.elapsed());
    // Guest bytes (code, registers, screen text, port data) stay out of the
    // log; commands, register indices, addresses and counts are enough to
    // name the device or CPU path that stopped.
    say!(
        "FINDINGS counters: device accesses={:?}; last port access (port, direction, size)={:x?}",
        ports.accesses,
        ports.last_access
    );
    say!("FINDINGS command/register writes (hex)={:x?}", ports.writes);
    say!("FINDINGS PIC special-mask writes={}; poll requests={}; PIT (channel, mode)={:?}; VGA indexed registers={:x?}", ports.pic_special_mask, ports.pic_poll, ports.pit_modes, ports.vga_registers);
    say!(
        "FINDINGS PIC command writes after the first special-mask write (port, value)={:x?}",
        ports.pic_after_special_mask
    );
    // Only the 8259 lines of the diagnostic report: register values, no
    // guest code or memory bytes.
    let report = assembled.diagnostic_report("stage end").to_string();
    for line in report.lines().filter(|line| line.starts_with("pic ")) {
        say!("FINDINGS {line}");
    }
    say!(
        "FINDINGS deliveries: {}",
        delivery_summary(assembled.machine.recent_deliveries())
    );
}

/// Run one stage on a new machine, starting from the previous stage's disk or
/// a freshly built one, and assert that it reached its boundary. Returns
/// `None` without running when the private-media variables are unset.
pub fn run(stage: Stage, previous: Option<Staged>) -> Option<Staged> {
    let source = if stage.win95() {
        "UNEMUPC_USER_WIN95_DIR"
    } else {
        "UNEMUPC_USER_WIN31_DIR"
    };
    let (Some(_), Some(floppy)) = (
        std::env::var_os(source),
        std::env::var_os("UNEMUPC_USER_DOS_FLOPPY"),
    ) else {
        say!("skipping: set {source} and UNEMUPC_USER_DOS_FLOPPY");
        return None;
    };
    // Past its first screen, setup refuses to continue without the product
    // key, which the runner types on the certificate page. The installed
    // disk holds the key too, so the boot stages keep the log guard armed.
    if (stage == Stage::Win95FileCopy || stage.win95_boot())
        && std::env::var_os("UNEMUPC_USER_WIN95_KEY").is_none()
    {
        say!("skipping: set UNEMUPC_USER_WIN95_KEY");
        return None;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dos7 = previous.as_ref().is_some_and(|disk| disk.dos7);
    assert!(
        !stage.win95_boot() || previous.is_some(),
        "a Windows 95 boot stage needs the disk of the stage before"
    );
    let image = previous.map(|disk| disk.image).unwrap_or_else(|| {
        let built = Command::new("bash")
            .arg(root.join("scripts/user-media-disk.sh"))
            .arg(if stage.win95() { "--win95" } else { "--win31" })
            .output()
            .expect("start private disk builder");
        assert!(built.status.success(), "private disk builder failed");
        PathBuf::from(String::from_utf8(built.stdout).unwrap().trim())
    });
    let directory = image.parent().unwrap().to_path_buf();
    let mut assembled = assemble_stage(stage, &image, &floppy, &directory);
    if stage.win95_boot() {
        return boot_win95(stage, assembled, image, &directory);
    }
    let directory = directory.as_path();
    let keyboard = assembled.device_index[&DeviceId::Keyboard];
    let mut ports = Ports::default();
    let started = Instant::now();
    let mut launched = None;
    let mut boot_menu_selected = false;
    let mut date_accepted = false;
    let mut clean_boot = false;
    let mut next_progress = 25_000_000;
    let mut success = false;
    let mut protected_seen = false;
    let mut paging_seen = false;
    let mut v86_seen = false;
    let mut v86_exit_seen = false;
    let mut shell_back = None;
    let mut settled: Option<(u64, u64)> = None;
    let mut last_disk_write = 0;
    let mut disk_writes = 0;
    let mut last_disk_command = 0;
    let mut disk_commands = 0;
    let mut dos_launch = None;
    let mut dos_prompt = false;
    let mut exit_sent = None;
    let mut first_graphics = None;
    let mut setup_confirmed = false;
    let mut execution_error = false;
    let mut reset_after_launch = false;
    let mut held: Option<(u64, Vec<u8>)> = None;
    let mut answered = 0;
    let reason;
    // The retired budget is independent of wall time. A second virtual-time
    // ceiling bounds HLT waits, which can consume no retired instructions.
    let mut slice = 0u64;
    loop {
        slice += 1;
        let result = run_slice(&mut assembled, &mut ports);
        let retired = assembled.machine.cpu().retired();
        let system = assembled.machine.cpu().diagnostic_state().system.unwrap();
        // Mode evidence counts only after the Windows command was typed, so
        // the BIOS, DOS or its memory manager cannot supply it.
        if let Some(at) = launched {
            protected_seen |= system.protected();
            paging_seen |= system.paging();
            v86_seen |= system.virtual_8086;
            if stage == Stage::Enhanced && !v86_exit_seen {
                // A delivery that leaves V86 for the monitor shows the VMM
                // running a V86 task, not just a VM flag sampled once.
                v86_exit_seen = assembled.machine.recent_deliveries().is_some_and(|log| {
                    log.entries.iter().any(|entry| {
                        matches!(entry, machine::DeliveryEntry::Deliveries { last, .. }
                            if last.retired >= at
                                && last.virtual_8086_before
                                && !last.virtual_8086_after)
                    })
                });
            }
        }
        match result {
            Err(error) => {
                reason = format!(
                    "execution stop: {}",
                    private_diagnostics::error_summary(&error)
                );
                execution_error = true;
                break;
            }
            Ok(machine::RunOutcome::Halted) => {
                reason = "CPU halted".into();
                break;
            }
            Ok(machine::RunOutcome::ResetRequested) => {
                // Setup hands its restart to the firmware, whose reset routine
                // ends with a write to the reset control register at CF9h.
                reset_after_launch =
                    launched.is_some() && ports.last_access == Some((0xcf9, AccessDir::Write, 1));
                reason = "guest reset requested".into();
                break;
            }
            _ => {}
        }
        // Capturing a 640x480 frame costs far more host time than one slice of
        // guest work, so the screen-driven decisions and budgets run once per
        // SCREEN_POLL_SLICES slices. Traces are still drained every slice.
        if !slice.is_multiple_of(SCREEN_POLL_SLICES) {
            continue;
        }
        // One capture per poll: rendering a 640x480 frame dominates the
        // harness's host time, so every screen decision below shares it.
        let frame = assembled.capture_text(DeviceId::Vga).unwrap().frame;
        let text = decoded_text(&frame);
        if stage == Stage::Win95FileCopy && launched.is_some() {
            // A blinking caret in a focused text box changes a few pixels;
            // any real redraw changes more than WIN95_CARET_PIXELS.
            held = match (held, &frame) {
                (Some((since, start)), video::Frame::Pixels(pixels))
                    if start.len() == pixels.indices.len()
                        && start
                            .iter()
                            .zip(&pixels.indices)
                            .filter(|(a, b)| a != b)
                            .count()
                            <= WIN95_CARET_PIXELS =>
                {
                    Some((since, start))
                }
                (_, video::Frame::Pixels(pixels)) => Some((retired, pixels.indices.clone())),
                (_, video::Frame::Text(_) | video::Frame::Direct(_)) => None,
            };
            if let Some((since, _)) = &held {
                let since = *since;
                if retired.saturating_sub(since.max(last_disk_command)) >= WIN95_DIALOG_HOLD
                    && answered < WIN95_ANSWERS.len()
                {
                    let (what, answer) = WIN95_ANSWERS[answered];
                    assembly::inject_scancodes(
                        &mut assembled.machine,
                        keyboard,
                        &answer.scancodes(),
                    );
                    answered += 1;
                    held = None;
                    say!(
                        "{stage:?} held setup dialog {answered} ({what}): retired={retired}; held since={since}; wall={:?}",
                        started.elapsed()
                    );
                }
            }
        }
        if !boot_menu_selected && text.contains("Startup Menu") && text.contains("without CD-ROM") {
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x03, 0x83, 0x1c, 0x9c]);
            boot_menu_selected = true;
            say!(
                "{stage:?} floppy no-CD boot option: retired={retired}; wall={:?}",
                started.elapsed()
            );
        }
        if stage == Stage::SystemTransfer && !clean_boot && text.contains("Starting MS-DOS") {
            // An MS-DOS 6 setup disk's startup files load keyboard support and
            // start DOS Setup; one of those programs times an 8042 wait with
            // the port 61h refresh bit, which never toggles here because the
            // firmware leaves PIT counter 1 unprogrammed. F5 at this message is
            // MS-DOS 6's clean boot: both files are skipped and the shell
            // prompt appears directly.
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x3f, 0xbf]);
            clean_boot = true;
            say!(
                "{stage:?} MS-DOS clean boot (F5): retired={retired}; wall={:?}",
                started.elapsed()
            );
        }
        if !date_accepted && text.contains("Enter new date") {
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x1c, 0x9c, 0x1c, 0x9c]);
            date_accepted = true;
        }
        if matches!(stage, Stage::Enhanced | Stage::EnhancedRefusedByDos7)
            && launched.is_some()
            && !paging_seen
            && text.contains("incompatible")
        {
            // MS-DOS 7.x refuses the Windows 3.1 Enhanced-mode launcher from its
            // own kernel and halts in real mode before paging is enabled.
            success = stage == Stage::EnhancedRefusedByDos7;
            reason = "the DOS kernel refused Enhanced mode before paging".into();
            break;
        }
        if stage == Stage::Install
            && launched.is_some()
            && first_screen(&assembled, &frame, &text, stage)
        {
            let first = *first_graphics.get_or_insert(retired);
            if !setup_confirmed && retired.saturating_sub(first) >= 5_000_000 {
                assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x1c, 0x9c]);
                setup_confirmed = true;
                say!(
                    "Install GUI confirmation ENTER: retired={retired}; wall={:?}",
                    started.elapsed()
                );
            }
        }
        // The Windows mode each stage must be in, from the launch on.
        let in_mode = match stage {
            Stage::Standard => protected_seen && !paging_seen && system.protected(),
            Stage::Enhanced => {
                paging_seen && v86_seen && v86_exit_seen && system.protected() && system.paging()
            }
            _ => false,
        };
        if stage.windows() && launched.is_some() {
            if dos_launch.is_none() {
                // Keys typed while Windows still shows its hourglass are lost,
                // so wait for a populated frame that stays identical at every
                // poll for ten million instructions in the stage's mode.
                let digest = (in_mode && first_screen(&assembled, &frame, &text, stage))
                    .then(|| frame_digest(&frame))
                    .flatten();
                settled = match (settled, digest) {
                    (Some((since, previous)), Some(digest)) if previous == digest => {
                        Some((since, digest))
                    }
                    (_, Some(digest)) => Some((retired, digest)),
                    (_, None) => None,
                };
            }
            if dos_launch.is_none()
                && settled.is_some_and(|(since, _)| retired.saturating_sub(since) >= 10_000_000)
            {
                // An explicit shell argument chooses Program Manager. Its File/Run
                // command opens COMMAND.COM without a mouse dependency; only the
                // shell's menu can start it, so its prompt proves the shell.
                let mut keys = vec![0x38, 0x21, 0xa1, 0xb8, 0x13, 0x93];
                keys.extend(command_keys("c:\\command.com\n"));
                assembly::inject_scancodes(&mut assembled.machine, keyboard, &keys);
                dos_launch = Some(retired);
                say!(
                    "{stage:?} settled frame / File/Run COMMAND.COM: retired={retired}; wall={:?}",
                    started.elapsed()
                );
            } else if dos_launch.is_some()
                && !dos_prompt
                && text.lines().any(|line| {
                    let line = line.trim();
                    line.starts_with("C:\\") && line.ends_with('>')
                })
                && if stage == Stage::Enhanced {
                    // The DOS VM runs in V86 mode under WIN386's paging.
                    system.virtual_8086 && system.paging()
                } else {
                    // Standard mode switches back to real mode to run DOS programs.
                    !system.protected()
                }
            {
                dos_prompt = true;
                assembly::inject_scancodes(
                    &mut assembled.machine,
                    keyboard,
                    &command_keys("exit\n"),
                );
                exit_sent = Some(retired);
                say!(
                    "{stage:?} DOS prompt / EXIT: retired={retired}; wall={:?}",
                    started.elapsed()
                );
            }
            // After EXIT, a populated frame must hold at every poll for five
            // million instructions while the CPU stays in the stage's mode.
            if exit_sent.is_some() {
                if in_mode && first_screen(&assembled, &frame, &text, stage) {
                    shell_back.get_or_insert(retired);
                } else {
                    shell_back = None;
                }
            }
        }
        if launched.is_none()
            && text.lines().any(|line| {
                line.trim_end().ends_with(if stage.boots_hard_disk() {
                    "C:\\>"
                } else {
                    "A:\\>"
                })
            })
        {
            assembly::inject_scancodes(
                &mut assembled.machine,
                keyboard,
                &command_keys(stage.command()),
            );
            launched = Some(retired);
            say!(
                "{stage:?} DOS prompt / command: retired={retired}; wall={:?}",
                started.elapsed()
            );
        } else if launched.is_some()
            && match stage {
                Stage::Win95 => first_screen(&assembled, &frame, &text, stage),
                // Only the restart, handled above, ends this stage.
                Stage::Win95FileCopy => false,
                Stage::SystemTransfer => {
                    dos7 = text.lines().any(|line| line.trim() == "UM31DOS7");
                    text.lines().any(|line| line.trim() == "UM31SYSOK")
                }
                Stage::Install => text.lines().any(|line| line.trim() == "UM31INSTALLED"),
                Stage::Standard | Stage::Enhanced => {
                    dos_prompt
                        && shell_back.is_some_and(|at| retired.saturating_sub(at) >= 5_000_000)
                }
                // Only the refusal itself, handled above, completes this stage.
                Stage::EnhancedRefusedByDos7 => false,
                // The boot stages run in boot_win95.
                Stage::Win95FirstBoot | Stage::Win95SecondBoot => false,
            }
        {
            success = true;
            reason = format!("{stage:?} boundary");
            break;
        }
        let commands = |write_only: bool| {
            ports
                .writes
                .iter()
                .filter(|((kind, _, value), _)| {
                    *kind == "ATA command" && (!write_only || matches!(*value, 0x30 | 0x31 | 0xc5))
                })
                .map(|(_, count)| *count)
                .sum::<u64>()
        };
        let writes = commands(true);
        if launched.is_none() || writes != disk_writes {
            disk_writes = writes;
            last_disk_write = retired;
        }
        let accesses = commands(false);
        if launched.is_none() || accesses != disk_commands {
            disk_commands = accesses;
            last_disk_command = retired;
        }
        if stage == Stage::Install
            && launched.is_some()
            && retired.saturating_sub(last_disk_write) >= 150_000_000
        {
            reason =
                "installation made no disk-write progress for 150 million retired instructions"
                    .into();
            break;
        }
        // Windows 95 setup reads for long stretches without writing (its
        // component scan, for one), so only a disk left idle counts as a stall.
        if stage == Stage::Win95FileCopy
            && launched.is_some()
            && retired.saturating_sub(last_disk_command) >= 150_000_000
        {
            reason = "setup issued no disk command for 150 million retired instructions".into();
            break;
        }
        if retired >= next_progress {
            say!(
                "{stage:?} progress: retired={retired}; wall={:?}; command={}; protected={}; paging={}; CPL={}",
                started.elapsed(),
                launched.is_some(), system.protected(), system.paging(), system.cpl
            );
            next_progress += 25_000_000;
        }
        let budget = match (launched.is_some(), stage) {
            (false, _) => 200_000_000,
            (true, Stage::Install) => 1_000_000_000,
            (true, Stage::Win95FileCopy) => 30_000_000_000,
            (true, _) => 300_000_000,
        };
        let virtual_ceiling = match stage {
            Stage::Install => 2_000_000_000,
            Stage::Win95FileCopy => 60_000_000_000,
            _ => 600_000_000,
        };
        if retired.saturating_sub(launched.unwrap_or(0)) >= budget
            || assembled.machine.virtual_time() >= virtual_ceiling
        {
            reason = "instruction/virtual-time budget exhausted".into();
            break;
        }
    }
    say!("FINDINGS stage={stage:?}; DOS 7={dos7}; after launch: protected={protected_seen}, paging={paging_seen}, V86={v86_seen}, V86 exit delivery={v86_exit_seen}; File/Run sent={}; DOS prompt={dos_prompt}; EXIT sent={}; frame held after EXIT={}", dos_launch.is_some(), exit_sent.is_some(), shell_back.is_some());
    report_stage_end(&mut assembled, &ports, &reason, started);
    say!(
        "FINDINGS command={}; setup ENTER={setup_confirmed}; dialogs answered={answered}; last ATA command at retired={last_disk_command}; last ATA write at retired={last_disk_write}",
        launched.is_some()
    );
    if stage.boots_hard_disk() {
        report_installed_choices(&mut assembled);
    }
    if stage == Stage::Win95FileCopy {
        let installed = report_win95_files(&mut assembled);
        success = reset_after_launch && installed;
    }
    close_stage(&mut assembled, execution_error);
    say!("{stage:?} private stage directory: {}", directory.display());
    assert!(
        success,
        "{stage:?} recorded its first stop before the required boundary"
    );
    Some(Staged { image, dos7 })
}

/// End the stage's disk session. A clean end finishes a journal generation.
/// A CPU failure must not be published as a successfully finished session:
/// it checkpoints only completed sectors for diagnosis or a restart; no CPU
/// state is resumed by a disk generation.
fn close_stage(assembled: &mut assembly::AssembledMachine, execution_error: bool) {
    if execution_error {
        for handle in &mut assembled.persistence {
            handle
                .checkpoint(&mut assembled.machine)
                .expect("private stop checkpoint failed");
        }
    } else {
        assembled
            .finish_persistence()
            .expect("private stage generation failed");
    }
}

/// A rectangle of the 640x480 screen: left, top, right and bottom, exclusive
/// at the right and bottom.
type Region = (usize, usize, usize, usize);

/// Where Windows 95 puts the pointer when it starts: the screen's centre,
/// with the arrow's hot spot at its top-left corner.
const POINTER_START: Region = (316, 236, 352, 276);
/// The bottom-left corner, where the script parks the pointer.
const POINTER_CORNER: Region = (0, 432, 48, 480);
/// Where the Start menu opens: above the Start button, left-aligned.
const START_MENU: Region = (0, 160, 200, 450);

/// Pixels of `region` that differ between two 640x480 frames.
fn changed_in(before: &[u8], after: &[u8], (left, top, right, bottom): Region) -> usize {
    (top..bottom)
        .map(|row| {
            let span = row * 640 + left..row * 640 + right;
            before[span.clone()]
                .iter()
                .zip(&after[span])
                .filter(|(a, b)| a != b)
                .count()
        })
        .sum()
}

/// Whether a frame has the desktop's shape: 640x480 with a taskbar along the
/// bottom, a full-width row of white (the raised edge's highlight) just above
/// a band at least 70% light grey (the bar's face), over an area at least 30%
/// one other colour (the desktop background), whatever windows are open on
/// it. Setup's first-boot screens and dialogs leave the bottom rows to their
/// background picture.
fn desktop_layout(pixels: &video::IndexedPixels) -> bool {
    if (pixels.width, pixels.height) != (640, 480) || pixels.indices.len() != 640 * 480 {
        return false;
    }
    let colour = |index: u8| {
        pixels
            .palette
            .get(index as usize)
            .copied()
            .unwrap_or_default()
    };
    let white = |index: u8| colour(index).iter().all(|level| *level >= 0xf0);
    // The Windows 95 VGA driver's button face is a slightly blue grey.
    let light_grey = |index: u8| {
        let levels = colour(index);
        let (low, high) = (*levels.iter().min().unwrap(), *levels.iter().max().unwrap());
        low >= 0x90 && high <= 0xd8 && high - low <= 0x10
    };
    let row = |y: usize| &pixels.indices[y * 640..(y + 1) * 640];
    let edge =
        (444..460).any(|y| row(y).iter().filter(|index| white(**index)).count() * 10 >= 640 * 9);
    let band = &pixels.indices[460 * 640..476 * 640];
    let face = band.iter().filter(|index| light_grey(**index)).count() * 10 >= band.len() * 7;
    let mut counts = [0usize; 256];
    for index in &pixels.indices[..440 * 640] {
        counts[*index as usize] += 1;
    }
    let (back, back_count) = counts
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count)
        .unwrap();
    edge && face && !light_grey(back as u8) && back_count * 10 >= 440 * 640 * 3
}

/// Virtual time the desktop must hold, apart from a caret's pixels and with
/// no ATA command at all, before the script starts: setup's first-boot items
/// keep the disk busy between their screens.
const DESKTOP_HOLD: u64 = 40_000_000;

/// Virtual time the frame must hold after a scripted input before the next.
const INPUT_SETTLE: u64 = 10_000_000;

/// Virtual time a script step may take, from its start to its settled frame,
/// and the power-off may take after the last key.
const STEP_DEADLINE: u64 = 1_000_000_000;

/// A frame that holds this long with the disk idle, without the desktop's
/// shape, is a dialog the runner has no answer for; the stage stops there.
const UNANSWERED_HOLD: u64 = 400_000_000;

/// How long a first-boot dialog must hold, with the disk idle, before the
/// runner answers it.
const FIRST_BOOT_DIALOG_HOLD: u64 = 40_000_000;

/// Keys for the dialogs setup's first boot shows with our batch file, in the
/// order it shows them.
const FIRST_BOOT_ANSWERS: [(&str, &[u8]); 1] = [
    // The last first-boot item opens the Add Printer Wizard; the board has
    // no printer port. ESC is the wizard's Cancel button.
    ("Add Printer Wizard: ESC for Cancel", &[0x01, 0x81]),
];

/// One scripted input on the desktop.
#[derive(Clone, Copy)]
enum Input {
    /// Relative counts and buttons (left is bit 0; positive Y is up), each
    /// packet sent once the mouse has delivered the one before.
    Mouse(&'static [(i32, i32, u8)]),
    /// Scancodes, make and break.
    Keys(&'static [u8]),
}

/// What the runner does on the settled desktop, in order. Each step waits
/// for the frame to hold for [`INPUT_SETTLE`] after its last input.
const DESKTOP_SCRIPT: [(&str, Input); 5] = [
    // Large moves down and to the left pin the pointer to the bottom-left
    // corner whatever the driver's acceleration does with them.
    (
        "pointer to the bottom-left corner",
        Input::Mouse(&[(-60, -60, 0); 12]),
    ),
    // Small moves, below the acceleration threshold, back the pointer off
    // the corner onto the Start button.
    (
        "pointer onto the Start button",
        Input::Mouse(&[(4, 4, 0), (4, 4, 0)]),
    ),
    ("left click on Start", Input::Mouse(&[(0, 0, 1), (0, 0, 0)])),
    // U is the access key of the Start menu's Shut Down item.
    ("Shut Down: U", Input::Keys(&[0x16, 0x96])),
    // The dialog's default choice shuts the computer down; ENTER is Yes.
    ("shut down the computer: ENTER", Input::Keys(&[0x1c, 0x9c])),
];

/// ATA commands seen so far.
fn ata_commands(ports: &Ports) -> u64 {
    ports
        .writes
        .iter()
        .filter(|((kind, ..), _)| *kind == "ATA command")
        .map(|(_, count)| *count)
        .sum()
}

/// Boot the installed Windows 95 from C: to the desktop, move the pointer,
/// open the Start menu with a click, and shut the computer down from it.
/// The stage passes when, after the last scripted key, the machine ends in
/// the firmware's APM power-off: a halt with interrupts off in real mode at a
/// firmware address, entered through INT 15h. A reset request never passes:
/// it also stands for a processor shutdown.
fn boot_win95(
    stage: Stage,
    mut assembled: assembly::AssembledMachine,
    image: PathBuf,
    directory: &std::path::Path,
) -> Option<Staged> {
    // Windows idles through the APM CPU Idle call, whose firmware routine
    // halts with interrupts enabled until the next one. The machine keeps
    // waiting for it; only a halt with interrupts off (the power-off) ends
    // the run. The virtual-time ceiling still bounds the wait.
    assembled.machine.set_idle_limit(Some(u64::MAX));
    let keyboard = assembled.device_index[&DeviceId::Keyboard];
    let mouse = assembled
        .mouse_device_index()
        .expect("the PS/2 mouse is configured");
    let mut ports = Ports::default();
    let mut frames = FrameDump::new(directory);
    let started = Instant::now();
    let mut milestones: Vec<(&str, u64, u64)> = Vec::new();
    let milestone = |what: &'static str, retired: u64, at: u64, list: &mut Vec<_>| {
        say!(
            "{stage:?} {what}: retired={retired}; virtual={at}; wall={:?}",
            started.elapsed()
        );
        list.push((what, retired, at));
    };
    let mut seen = BTreeSet::new();
    let mut v86_exit_seen = false;
    let mut ring3_seen = false;
    let mut restarts: Vec<&'static str> = Vec::new();
    let mut reset_signals_seen: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut held: Option<(u64, Vec<u8>)> = None;
    let mut last_disk_command = 0;
    let mut disk_commands = 0;
    let mut desktop = false;
    let mut step = 0usize;
    let mut sent = 0usize;
    let mut step_start: Vec<u8> = Vec::new();
    let mut evidence: Vec<(&str, usize, [usize; 3])> = Vec::new();
    let mut ending = None;
    let mut step_began = 0u64;
    let mut answered = 0;
    let mut execution_error = false;
    let mut next_progress = 250_000_000;
    let mut slice = 0u64;
    let reason: String;
    loop {
        slice += 1;
        let result = run_slice(&mut assembled, &mut ports);
        let retired = assembled.machine.cpu().retired();
        let now = assembled.machine.virtual_time();
        match result {
            Err(error) => {
                reason = format!(
                    "execution stop: {}",
                    private_diagnostics::error_summary(&error)
                );
                execution_error = true;
                break;
            }
            Ok(machine::RunOutcome::Halted) => {
                reason = "CPU halted with interrupts off".into();
                ending = Some("halt");
                break;
            }
            Ok(machine::RunOutcome::ResetRequested) => {
                // A reset request is also how a processor shutdown (a triple
                // fault) ends. Before the desktop, a restart the board was
                // asked for through one of its reset signals boots C: again
                // on the same machine; any other request ends the stage, as
                // does every reset once the script has begun.
                let signal = ports
                    .reset_signals
                    .iter()
                    .find(|(kind, count)| reset_signals_seen.get(*kind) < Some(*count))
                    .map(|(kind, _)| *kind);
                reset_signals_seen = ports.reset_signals.clone();
                let board = signal.is_some();
                if desktop || !board {
                    reason = format!(
                        "guest reset requested (board reset signal={board}; desktop={desktop}; script step={step})"
                    );
                    ending = Some("reset");
                    break;
                }
                restarts.extend(signal);
                milestone("board-signalled restart", retired, now, &mut milestones);
                assembled
                    .before_cpu_reset()
                    .expect("private journal output failed");
                assembled.machine.reset_cpu().unwrap();
                held = None;
                continue;
            }
            _ => {}
        }
        if !slice.is_multiple_of(SCREEN_POLL_SLICES) {
            continue;
        }
        let system = assembled.machine.cpu().diagnostic_state().system.unwrap();
        for (what, now_seen) in [
            ("protected mode", system.protected()),
            ("paging", system.paging()),
        ] {
            if now_seen && seen.insert(what) {
                milestone(what, retired, now, &mut milestones);
            }
        }
        if let Some(log) = assembled.machine.recent_deliveries() {
            for entry in &log.entries {
                if let machine::DeliveryEntry::Deliveries { last, .. } = entry {
                    // A delivery that leaves V86 for ring 0 is the VMM running a
                    // V86 task; one that interrupts ring-3 protected code is a
                    // Windows application running in the system VM.
                    if !v86_exit_seen && last.virtual_8086_before && !last.virtual_8086_after {
                        v86_exit_seen = true;
                        milestone("V86 exit delivery", retired, now, &mut milestones);
                    }
                    if !ring3_seen
                        && system.paging()
                        && last.cpl_before == 3
                        && !last.virtual_8086_before
                    {
                        ring3_seen = true;
                        milestone("ring-3 protected delivery", retired, now, &mut milestones);
                    }
                }
            }
        }
        let accesses = ata_commands(&ports);
        if accesses != disk_commands {
            disk_commands = accesses;
            last_disk_command = now;
        }
        let frame = assembled.capture_text(DeviceId::Vga).unwrap().frame;
        frames.write(&frame, retired, false);
        held = match (held, &frame) {
            (Some((since, start)), video::Frame::Pixels(pixels))
                if start.len() == pixels.indices.len()
                    && start
                        .iter()
                        .zip(&pixels.indices)
                        .filter(|(a, b)| a != b)
                        .count()
                        <= WIN95_CARET_PIXELS =>
            {
                Some((since, start))
            }
            (_, video::Frame::Pixels(pixels)) => Some((now, pixels.indices.clone())),
            (_, video::Frame::Text(_) | video::Frame::Direct(_)) => None,
        };
        let video::Frame::Pixels(pixels) = &frame else {
            continue;
        };
        if (pixels.width, pixels.height) == (640, 480) && seen.insert("640x480 graphics") {
            milestone("640x480 graphics", retired, now, &mut milestones);
        }
        let held_for = held
            .as_ref()
            .map_or(0, |(since, _)| now.saturating_sub(*since));
        let idle_for = held.as_ref().map_or(0, |(since, _)| {
            now.saturating_sub((*since).max(last_disk_command))
        });
        if !desktop {
            let layout = desktop_layout(pixels);
            // A desktop counts only once setup's first-boot dialogs are all
            // answered; a message box over a desktop would still pass.
            let questions_done =
                stage != Stage::Win95FirstBoot || answered == FIRST_BOOT_ANSWERS.len();
            if idle_for >= DESKTOP_HOLD
                && layout
                && questions_done
                && system.paging()
                && v86_exit_seen
                && ring3_seen
            {
                milestone("settled desktop", retired, now, &mut milestones);
                frames.write(&frame, retired, true);
                desktop = true;
                step_began = now;
                step_start = pixels.indices.clone();
            } else if stage == Stage::Win95FirstBoot
                && answered < FIRST_BOOT_ANSWERS.len()
                && !layout
                && idle_for >= FIRST_BOOT_DIALOG_HOLD
            {
                let (what, keys) = FIRST_BOOT_ANSWERS[answered];
                frames.write(&frame, retired, true);
                assembly::inject_scancodes(&mut assembled.machine, keyboard, keys);
                answered += 1;
                held = None;
                say!(
                    "{stage:?} held dialog {answered} ({what}): retired={retired}; virtual={now}; wall={:?}",
                    started.elapsed()
                );
            } else if idle_for >= UNANSWERED_HOLD {
                frames.write(&frame, retired, true);
                reason = format!(
                    "a frame held with the disk idle without the desktop's shape (layout={layout}; paging={}; V86 exit={v86_exit_seen}; ring 3={ring3_seen})",
                    system.paging()
                );
                break;
            }
        } else if step < DESKTOP_SCRIPT.len() {
            let (what, input) = DESKTOP_SCRIPT[step];
            match input {
                Input::Mouse(packets) if sent < packets.len() => {
                    if assembled.mouse_input_ready() {
                        let (dx, dy, buttons) = packets[sent];
                        assert!(assembly::inject_mouse(
                            &mut assembled.machine,
                            mouse,
                            dx,
                            dy,
                            buttons
                        ));
                        sent += 1;
                        held = None;
                    }
                }
                Input::Keys(keys) if sent == 0 => {
                    assembly::inject_scancodes(&mut assembled.machine, keyboard, keys);
                    sent = 1;
                    held = None;
                    // The last key shuts Windows down: nothing settles after it.
                    if step + 1 == DESKTOP_SCRIPT.len() {
                        say!(
                            "{stage:?} desktop step {} ({what}): retired={retired}; virtual={now}; wall={:?}",
                            step + 1,
                            started.elapsed()
                        );
                        step += 1;
                        step_began = now;
                        milestone("shutdown requested", retired, now, &mut milestones);
                    }
                }
                _ if held_for >= INPUT_SETTLE => {
                    let after = &pixels.indices;
                    let total = step_start.iter().zip(after).filter(|(a, b)| a != b).count();
                    let regions = [POINTER_START, POINTER_CORNER, START_MENU]
                        .map(|region| changed_in(&step_start, after, region));
                    say!(
                        "{stage:?} desktop step {} ({what}): retired={retired}; virtual={now}; changed pixels={total}; in pointer start / corner / Start menu regions={regions:?}; wall={:?}",
                        step + 1,
                        started.elapsed()
                    );
                    frames.write(&frame, retired, true);
                    evidence.push((what, total, regions));
                    step_start = after.clone();
                    step += 1;
                    step_began = now;
                    sent = 0;
                }
                _ => {}
            }
        }
        // Each script step, and the power-off after the last key, has its own
        // deadline, so a stall stops the stage long before the budget.
        if desktop && now.saturating_sub(step_began) >= STEP_DEADLINE {
            reason = match DESKTOP_SCRIPT.get(step) {
                Some((what, _)) => format!(
                    "desktop step {} ({what}) did not settle within {STEP_DEADLINE} virtual-time units",
                    step + 1
                ),
                None => format!(
                    "no power-off within {STEP_DEADLINE} virtual-time units of the last key"
                ),
            };
            break;
        }
        if retired >= next_progress {
            say!(
                "{stage:?} progress: retired={retired}; virtual={now}; wall={:?}; protected={}; paging={}; CPL={}; restarts={restarts:?}; desktop={}; step={step}",
                started.elapsed(),
                system.protected(),
                system.paging(),
                system.cpl,
                desktop
            );
            next_progress += 250_000_000;
        }
        if retired >= 25_000_000_000 || now >= 50_000_000_000 {
            reason = "instruction/virtual-time budget exhausted".into();
            break;
        }
    }
    let state = assembled.machine.cpu().diagnostic_state();
    let system = state.system.unwrap();
    let eip = assembled.machine.cpu().reg(cpu::CpuReg::Rip);
    let linear = state.cs().base.wrapping_add(eip as u32);
    // SeaBIOS's APM power-off ends in its own HLT with interrupts disabled,
    // in the firmware's segment. Windows reaches it from real mode through
    // INT 15h (APM BIOS Interface Specification 1.2, Set Power State), the
    // last interrupt before the halt.
    let firmware = (0xf0000..=0xfffff).contains(&linear);
    let last_delivery = assembled.machine.recent_deliveries().and_then(|log| {
        log.entries.iter().rev().find_map(|entry| match entry {
            machine::DeliveryEntry::Deliveries { last, .. } => {
                Some((last.kind.word(), last.vector))
            }
            machine::DeliveryEntry::Dropped(_) => None,
        })
    });
    let ended_after_script = step == DESKTOP_SCRIPT.len() && ending.is_some();
    let powered_off = ending == Some("halt")
        && firmware
        && !system.protected()
        && last_delivery == Some(("software", 0x15));
    say!(
        "FINDINGS stage={stage:?}; board-signalled restarts={restarts:?}; dialogs answered={answered}; desktop={desktop}; script steps done={step}; end={ending:?}; ended after the script={ended_after_script}; halt in the firmware segment={firmware}; protected at the end={}; last delivery before the end={last_delivery:x?}; APM power-off={powered_off}",
        system.protected()
    );
    say!("FINDINGS milestones (what, retired, virtual): {milestones:?}");
    report_stage_end(&mut assembled, &ports, &reason, started);
    let mouse_bytes = ports
        .writes
        .iter()
        .filter(|((kind, ..), _)| *kind == "8042 aux data")
        .map(|((_, _, value), count)| (*value, *count))
        .collect::<Vec<_>>();
    say!("FINDINGS mouse commands (byte, count): {mouse_bytes:x?}");
    let pointer_moved = evidence
        .first()
        .is_some_and(|(_, _, regions)| regions[0] > 0 && regions[1] > 0);
    let menu_opened = evidence
        .get(2)
        .is_some_and(|(_, _, regions)| regions[2] >= 10_000);
    // The Shut Down dialog opens over the middle of the screen, outside the
    // menu it replaces.
    let dialog_opened = evidence
        .get(3)
        .is_some_and(|(_, total, regions)| total.saturating_sub(regions[2]) >= 10_000);
    say!("FINDINGS desktop proof: pointer moved={pointer_moved}; Start menu opened={menu_opened}; Shut Down dialog opened={dialog_opened}");
    let installed = report_win95_boot(&mut assembled);
    close_stage(&mut assembled, execution_error);
    say!("{stage:?} private stage directory: {}", directory.display());
    assert!(
        !execution_error
            && desktop
            && pointer_moved
            && menu_opened
            && dialog_opened
            && ended_after_script
            && powered_off
            && installed,
        "{stage:?} recorded its first stop before the required boundary"
    );
    Some(Staged { image, dos7: true })
}

/// Booleans and identifiers from the disk after a Windows 95 boot: whether
/// the first boot completed, which IDE path the I/O supervisor took, and the
/// devices Windows enumerated. Returns whether the registry's device tree
/// could be read from the disk.
fn report_win95_boot(assembled: &mut assembly::AssembledMachine) -> bool {
    let (_directory, snapshot) = disk_snapshot(assembled);
    // The partition's boot sector and the start of its first FAT.
    let mut head = vec![0u8; 64 * 1024];
    let flags = {
        use std::io::Read;
        std::fs::File::open(&snapshot)
            .and_then(|mut file| file.read_exact(&mut head))
            .ok()
            .and_then(|()| fat16_volume_flags(&head[63 * 512..]))
    };
    say!("FINDINGS FAT16 volume flags (clean shutdown, no disk error): {flags:?}");
    let present: BTreeMap<&str, bool> = [
        "WINDOWS/SYSTEM.1ST",
        "WINDOWS/SYSTEM.DA0",
        "BOOTLOG.TXT",
        "WINDOWS/IOS.LOG",
        "WINDOWS/SYSTEM/IOSUBSYS/ESDI_506.PDR",
    ]
    .iter()
    .map(|path| (*path, snapshot_file(&snapshot, path).is_some()))
    .collect();
    say!("FINDINGS boot files present: {present:?}");
    let listing = Command::new("mdir")
        .arg("-b")
        .arg("-i")
        .arg(format!("{}@@32256", snapshot.display()))
        .arg("::/WINDOWS/SYSTEM/IOSUBSYS")
        .env("MTOOLSRC", "/dev/null")
        // A volume Windows has mounted carries its "dirty" bit in the second
        // FAT entry, which mtools otherwise refuses as a bad FAT.
        .env("MTOOLS_SKIP_CHECK", "1")
        .output()
        .expect("start private directory listing");
    // Only the 8.3 names of the I/O supervisor's drivers.
    let drivers: BTreeSet<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter_map(|line| line.rsplit('/').next())
        .map(|name| {
            name.chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~'))
                .take(12)
                .collect::<String>()
                .to_ascii_uppercase()
        })
        .filter(|name| !name.is_empty())
        .collect();
    say!("FINDINGS IOSUBSYS drivers: {drivers:?}");
    if !listing.status.success() {
        // mtools names the check that refused the volume; no file data.
        let error = String::from_utf8_lossy(&listing.stderr);
        say!(
            "FINDINGS directory listing refused: {}",
            error
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>()
        );
    }
    if let Some(log) = snapshot_file(&snapshot, "WINDOWS/IOS.LOG") {
        say!(
            "FINDINGS IOS.LOG units through real-mode drivers: {:?}",
            ios_log_real_mode_units(&String::from_utf8_lossy(&log))
        );
    }
    if let Some(log) = snapshot_file(&snapshot, "BOOTLOG.TXT") {
        say!(
            "FINDINGS boot log (derived): {:?}",
            boot_log_summary(&String::from_utf8_lossy(&log))
        );
    }
    let devices =
        snapshot_file(&snapshot, "WINDOWS/SYSTEM.DAT").and_then(|file| registry_enum(&file));
    match &devices {
        Some(devices) => say!("FINDINGS registry Enum devices: {devices:?}"),
        None => say!("FINDINGS registry Enum unreadable"),
    }
    devices.is_some()
}

/// What the boot log says about the protected-mode IDE driver, plus the
/// number of failure lines and the component each names. Each line of the
/// log names a component and an outcome; no other text is kept.
fn boot_log_summary(log: &str) -> BTreeMap<&'static str, String> {
    let lines: Vec<String> = log.lines().map(str::to_ascii_lowercase).collect();
    let esdi = |outcome: &str| {
        lines
            .iter()
            .any(|line| line.contains("esdi_506") && line.contains(outcome))
    };
    let failed: Vec<&String> = lines.iter().filter(|line| line.contains("fail")).collect();
    let names: BTreeSet<String> = failed
        .iter()
        .filter_map(|line| {
            // "Outcome = name ..." or "Outcome name": the first word after
            // the equals sign, else the last word, without its directory.
            let word = match line.split_once('=') {
                Some((_, rest)) => rest.split_whitespace().next(),
                None => line.split_whitespace().last(),
            }?;
            let name: String = word
                .rsplit('\\')
                .next()?
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_'))
                .take(16)
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect();
    [
        ("lines", lines.len().to_string()),
        ("ESDI_506 initialising", esdi("initing").to_string()),
        ("ESDI_506 init success", esdi("success").to_string()),
        ("ESDI_506 failure", esdi("fail").to_string()),
        ("failure lines", failed.len().to_string()),
        ("failed components", format!("{names:?}")),
    ]
    .into()
}

/// The DOS block-device units the I/O supervisor's log says go through
/// real-mode drivers (compatibility mode), as two hexadecimal digits each.
fn ios_log_real_mode_units(log: &str) -> BTreeSet<String> {
    log.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("Unit number ")?;
            let (unit, tail) = rest.split_once(' ')?;
            (tail.contains("real mode")
                && unit.len() <= 2
                && unit.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| unit.to_ascii_uppercase())
        })
        .collect()
}

/// The two volume flags Windows 95 keeps in the second entry of a FAT16
/// volume's first FAT, from the volume's boot sector onward: bit 15 set means
/// the volume was shut down cleanly (clear while mounted), bit 14 set means no
/// disk error was met. mtools refuses a volume whose flags are clear unless
/// told to skip its checks. `None` when the boot sector's layout does not fit.
fn fat16_volume_flags(volume: &[u8]) -> Option<(bool, bool)> {
    let bytes_per_sector = u16::from_le_bytes(volume.get(11..13)?.try_into().ok()?) as usize;
    let reserved = u16::from_le_bytes(volume.get(14..16)?.try_into().ok()?) as usize;
    let at = reserved.checked_mul(bytes_per_sector)? + 2;
    let entry = u16::from_le_bytes(volume.get(at..at + 2)?.try_into().ok()?);
    Some((entry & 0x8000 != 0, entry & 0x4000 != 0))
}

/// The `Enum` branch of a Windows 95 registry file (SYSTEM.DAT holds
/// HKEY_LOCAL_MACHINE) as "enumerator\device" key names: the devices Device
/// Manager lists, by hardware identifier. Values are never read. The format
/// is the one Windows 95 shares with its predecessor's REG.DAT: a "CREG"
/// header, an "RGKN" block of fixed-size key nodes linked by parent, child
/// and sibling offsets, and "RGDB" blocks holding each node's name. `None`
/// when the file does not parse.
fn registry_enum(file: &[u8]) -> Option<BTreeSet<String>> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(file.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(file.get(at..at + 4)?.try_into().ok()?));
    if file.get(..4)? != b"CREG" || file.get(0x20..0x24)? != b"RGKN" {
        return None;
    }
    // Key names by (block, id), from every RGDB block's key records.
    let mut names = BTreeMap::new();
    let mut block_at = u32_at(8)? as usize;
    for _ in 0..u16_at(0x10)? {
        if file.get(block_at..block_at + 4)? != b"RGDB" {
            return None;
        }
        let block_size = u32_at(block_at + 4)? as usize;
        let mut record = block_at + 0x20;
        while record + 0x14 <= block_at + block_size {
            let next = u32_at(record)? as usize;
            let id = u16_at(record + 4)?;
            let block = u16_at(record + 6)?;
            let length = u16_at(record + 0x0c)? as usize;
            if id != 0xffff {
                let name = file.get(record + 0x14..record + 0x14 + length)?;
                names.insert((block, id), String::from_utf8_lossy(name).into_owned());
            }
            if next == 0 {
                break;
            }
            record += next;
        }
        block_at += block_size;
    }
    let node = |offset: u32| -> Option<(u32, u32, String)> {
        let at = 0x20 + offset as usize;
        let child = u32_at(at + 0x10)?;
        let sibling = u32_at(at + 0x14)?;
        let name = names
            .get(&(u16_at(at + 0x1a)?, u16_at(at + 0x18)?))?
            .clone();
        Some((child, sibling, name))
    };
    let children = |parent: u32| -> Option<Vec<(u32, String)>> {
        let mut list = Vec::new();
        let (mut next, ..) = node(parent)?;
        while next != u32::MAX && list.len() < 4096 {
            let (_, sibling, name) = node(next)?;
            list.push((next, name));
            next = sibling;
        }
        Some(list)
    };
    let root = u32_at(0x28)?;
    let (enum_key, _) = children(root)?
        .into_iter()
        .find(|(_, name)| name.eq_ignore_ascii_case("enum"))?;
    let mut devices = BTreeSet::new();
    for (enumerator, enumerator_name) in children(enum_key)? {
        for (_, device) in children(enumerator)? {
            devices.insert(format!("{enumerator_name}\\{device}"));
        }
    }
    Some(devices)
}

#[test]
fn registry_enum_lists_enumerator_and_device_names_only() {
    // A synthetic CREG file: root -> {Enum -> {Root -> {*PNP0C01}, BIOS ->
    // {*PNP0000}}, Software}; the device key holds a value that must not be read.
    let names = [
        "", "Enum", "Root", "*PNP0C01", "BIOS", "*PNP0000", "Software",
    ];
    // (child, sibling) per node, by node index; node offsets are 0x20 + 0x1c * i.
    let links: [(Option<usize>, Option<usize>); 7] = [
        (Some(1), None),
        (Some(2), Some(6)),
        (Some(3), Some(4)),
        (None, None),
        (Some(5), None),
        (None, None),
        (None, None),
    ];
    let offset = |node: Option<usize>| node.map_or(u32::MAX, |i| 0x20 + 0x1c * i as u32);
    let mut rgkn = vec![0u8; 0x20];
    rgkn[..4].copy_from_slice(b"RGKN");
    rgkn[8..12].copy_from_slice(&0x20u32.to_le_bytes());
    for (i, (child, sibling)) in links.iter().enumerate() {
        let mut entry = [0u8; 0x1c];
        entry[0x10..0x14].copy_from_slice(&offset(*child).to_le_bytes());
        entry[0x14..0x18].copy_from_slice(&offset(*sibling).to_le_bytes());
        entry[0x18..0x1a].copy_from_slice(&(i as u16).to_le_bytes());
        rgkn.extend(entry);
    }
    let rgkn_size = rgkn.len() as u32;
    rgkn[4..8].copy_from_slice(&rgkn_size.to_le_bytes());
    let mut rgdb = vec![0u8; 0x20];
    rgdb[..4].copy_from_slice(b"RGDB");
    for (i, name) in names.iter().enumerate() {
        let value: &[u8] = if i == 3 { b"PRIVATE-VALUE" } else { b"" };
        let size = 0x14 + name.len() + value.len();
        let mut record = vec![0u8; 0x14];
        let next = if i + 1 == names.len() { 0 } else { size as u32 };
        record[..4].copy_from_slice(&next.to_le_bytes());
        record[4..6].copy_from_slice(&(i as u16).to_le_bytes());
        record[0x0c..0x0e].copy_from_slice(&(name.len() as u16).to_le_bytes());
        record.extend(name.as_bytes());
        record.extend(value);
        rgdb.extend(record);
    }
    let rgdb_size = rgdb.len() as u32;
    rgdb[4..8].copy_from_slice(&rgdb_size.to_le_bytes());
    let mut file = vec![0u8; 0x20];
    file[..4].copy_from_slice(b"CREG");
    file[8..12].copy_from_slice(&(0x20 + rgkn_size).to_le_bytes());
    file[0x10..0x12].copy_from_slice(&1u16.to_le_bytes());
    file.extend(rgkn);
    file.extend(rgdb);
    let devices = registry_enum(&file).expect("synthetic registry parses");
    assert_eq!(
        devices,
        ["BIOS\\*PNP0000", "Root\\*PNP0C01"]
            .map(String::from)
            .into()
    );
    assert!(!format!("{devices:?}").contains("PRIVATE"));
    assert!(registry_enum(b"not a registry").is_none());
}

/// Local debugging aid for one stage: with UNEMUPC_PRIVATE_FRAMES=1, the
/// stage's 640x480 frames go as BMP files into `frames/` inside its private
/// stage directory, every 200 million retired instructions and at each
/// runner event. They show commercial screens and stay with the directory.
struct FrameDump {
    directory: Option<PathBuf>,
    next: u64,
}

impl FrameDump {
    fn new(stage_directory: &std::path::Path) -> Self {
        let wanted = std::env::var_os("UNEMUPC_PRIVATE_FRAMES").is_some_and(|value| value == "1");
        let directory = wanted
            .then(|| stage_directory.join("frames"))
            .and_then(|directory| {
                let made = std::fs::create_dir_all(&directory).and_then(|()| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(
                            &directory,
                            std::fs::Permissions::from_mode(0o700),
                        )?;
                    }
                    Ok(())
                });
                match made {
                    Ok(()) => Some(directory),
                    Err(error) => {
                        say!("frame dump off: {:?}", error.kind());
                        None
                    }
                }
            });
        Self { directory, next: 0 }
    }

    fn write(&mut self, frame: &video::Frame, retired: u64, event: bool) {
        let Some(directory) = &self.directory else {
            return;
        };
        if !event {
            if retired < self.next {
                return;
            }
            self.next = retired + 200_000_000;
        }
        let video::Frame::Pixels(pixels) = frame else {
            return;
        };
        let (w, h) = (pixels.width as usize, pixels.height as usize);
        let row = (w * 3).div_ceil(4) * 4;
        let mut bmp = Vec::with_capacity(54 + row * h);
        bmp.extend(b"BM");
        bmp.extend(((54 + row * h) as u32).to_le_bytes());
        bmp.extend([0u8; 4]);
        bmp.extend(54u32.to_le_bytes());
        bmp.extend(40u32.to_le_bytes());
        bmp.extend((w as i32).to_le_bytes());
        bmp.extend((h as i32).to_le_bytes());
        bmp.extend(1u16.to_le_bytes());
        bmp.extend(24u16.to_le_bytes());
        bmp.extend([0u8; 24]);
        for y in (0..h).rev() {
            let start = bmp.len();
            for x in 0..w {
                let index = pixels.indices.get(y * w + x).copied().unwrap_or(0);
                let [r, g, b] = pixels
                    .palette
                    .get(index as usize)
                    .copied()
                    .unwrap_or_default();
                bmp.extend([b, g, r]);
            }
            bmp.resize(start + row, 0);
        }
        let name = format!(
            "frame-{retired:012}{}.bmp",
            if event { "-event" } else { "" }
        );
        if let Err(error) = std::fs::write(directory.join(name), bmp) {
            say!("frame dump failed at retired={retired}: {:?}", error.kind());
        }
    }
}

/// A private copy made by [`resume`]. It holds the product key on its disk:
/// when the test that made it fails, dropping it deletes the copy; when the
/// stages run on it pass, it is their retained result.
#[allow(dead_code, reason = "only the Windows 95 stages resume")]
pub struct ResumeCopy(PathBuf);

impl Drop for ResumeCopy {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let removed = std::fs::remove_dir_all(&self.0).is_ok();
            eprintln!("deleted the resume copy after a failed stage: {removed}");
        }
    }
}

/// A copy of the retained stage directory `variable` names, in a new private
/// directory beside the builder's, so a later stage can start from that
/// stage's last generation without changing it. Keep the returned guard for
/// the whole test. `None` when the variable or one of the Windows 95 inputs
/// is unset.
#[allow(dead_code, reason = "only the Windows 95 stages resume")]
pub fn resume(variable: &str) -> Option<(Staged, ResumeCopy)> {
    let source = PathBuf::from(std::env::var_os(variable)?);
    for input in [
        "UNEMUPC_USER_WIN95_DIR",
        "UNEMUPC_USER_DOS_FLOPPY",
        "UNEMUPC_USER_WIN95_KEY",
    ] {
        std::env::var_os(input)?;
    }
    let made = Command::new("mktemp")
        .args(["-d", "/tmp/unemupc-user-media-XXXXXXXX"])
        .output()
        .expect("start mktemp");
    assert!(made.status.success(), "private stage copy directory failed");
    let target = PathBuf::from(String::from_utf8(made.stdout).unwrap().trim());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
            .expect("private stage copy permissions failed");
    }
    let copy = ResumeCopy(target.clone());
    for name in ["data.img", "ide.journal"] {
        std::fs::copy(source.join(name), target.join(name))
            .unwrap_or_else(|_| panic!("private stage copy failed"));
    }
    say!("resuming from a copy of {variable}: {}", target.display());
    Some((
        Staged {
            image: target.join("data.img"),
            dos7: true,
        },
        copy,
    ))
}

#[test]
fn port_summary_keeps_aux_routing_across_poll_boundaries() {
    let event = |port, value| TraceEvent::Port {
        at: device::VClock(0),
        port,
        size: 1,
        dir: AccessDir::Write,
        value,
    };
    let mut ports = Ports::default();
    ports.observe(vec![
        event(0x64, 0xd4),
        event(0x1f7, 0xec),
        event(0x20, 0x11),
    ]);
    ports.observe(vec![
        event(0x60, 0xf4),
        event(0x20, 0x0b),
        event(0x3f5, 0x08),
    ]);
    assert_eq!(ports.writes[&("8042 aux data", 0x60, 0xf4)], 1);
    assert_eq!(ports.writes[&("ATA command", 0x1f7, 0xec)], 1);
    assert_eq!(ports.writes[&("PIC ICW1", 0x20, 0x11)], 1);
    assert_eq!(ports.writes[&("PIC OCW3", 0x20, 0x0b)], 1);
}

#[test]
fn port_summary_counts_board_reset_signals_only() {
    let event = |port, value| TraceEvent::Port {
        at: device::VClock(0),
        port,
        size: 1,
        dir: AccessDir::Write,
        value,
    };
    let mut ports = Ports::default();
    ports.observe(vec![
        event(0x64, 0xfe),
        event(0x64, 0xd4),
        event(0x92, 0x02),
        event(0x92, 0x03),
        event(0xcf9, 0x02),
        event(0xcf9, 0x06),
        event(0x60, 0xfe),
    ]);
    assert_eq!(
        ports.reset_signals,
        [("8042 FEh", 1), ("CF9h", 1), ("port 92h", 1)].into()
    );
}

#[test]
fn private_port_diagnostics_cannot_reconstruct_disk_payloads() {
    let diagnostics = |payload: &[u8]| {
        let mut ports = Ports::default();
        for port in [0x1f0, 0x170] {
            for dir in [AccessDir::Read, AccessDir::Write] {
                for size in [1, 2, 4] {
                    // Stop in the middle of a transfer, with key bytes still in
                    // the recent history. Compare the whole printable summary.
                    ports.observe(
                        payload
                            .chunks(size)
                            .enumerate()
                            .map(|(index, bytes)| {
                                let mut word = [0; 4];
                                word[..bytes.len()].copy_from_slice(bytes);
                                TraceEvent::Port {
                                    at: device::VClock(index as u64 + 100),
                                    port,
                                    size: size as u8,
                                    dir,
                                    value: u32::from_le_bytes(word),
                                }
                            })
                            .collect(),
                    );
                }
            }
        }
        format!("{ports:x?}")
    };
    let private = diagnostics(b"SECRET-KEY");
    assert_eq!(
        private,
        diagnostics(b"OTHER-DATA"),
        "payload must not affect any diagnostic field"
    );
    for metadata in [
        "01f0",
        "0170",
        "Read",
        "Write",
        "size: 1",
        "size: 2",
        "size: 4",
        "VClock(100)",
    ] {
        assert!(
            private.contains(metadata),
            "missing port metadata: {metadata}"
        );
    }
}

#[test]
fn recent_port_history_hides_keyboard_data_values() {
    let history = |scancode: u32| {
        let mut ports = Ports::default();
        ports.observe(
            [AccessDir::Read, AccessDir::Write]
                .into_iter()
                .map(|dir| TraceEvent::Port {
                    at: device::VClock(7),
                    port: 0x60,
                    size: 1,
                    dir,
                    value: scancode,
                })
                .collect(),
        );
        ports.recent.join("\n")
    };
    let typed = history(0x02);
    assert_eq!(typed, history(0x0b), "scancode must not affect the history");
    for metadata in ["0060", "Read", "Write", "VClock(7)", "[payload omitted]"] {
        assert!(
            typed.contains(metadata),
            "missing port metadata: {metadata}"
        );
    }
}

#[test]
fn delivery_summary_keeps_addresses_and_omits_guest_bytes_and_registers() {
    let delivery = |bytes: u8, eax| cpu::Delivery {
        retired: 7,
        kind: cpu::DeliveryKind::Fault,
        vector: 13,
        irq: None,
        cs: 0x28,
        ip: 0x1234,
        linear: 0x8000_1234,
        error_code: Some(0x18),
        cpl_before: 3,
        cpl_after: 0,
        virtual_8086_before: true,
        virtual_8086_after: false,
        bytes: [cpu::diagnostic::Peek::Value(bytes); cpu::delivery::DELIVERY_BYTES],
        regs: Some(cpu::DeliveryRegisters {
            eax,
            ebx: eax,
            ecx: eax,
            edx: eax,
            esi: eax,
            edi: eax,
            ebp: eax,
            esp: eax,
            ds: eax as u16,
            es: eax as u16,
            ss: eax as u16,
            fs: Some(eax as u16),
            gs: Some(eax as u16),
            flags: eax,
        }),
    };
    let summary = |bytes, eax| {
        delivery_summary(Some(machine::RecentDeliveries {
            entries: vec![machine::DeliveryEntry::Deliveries {
                first_retired: 7,
                count: 2,
                last: delivery(bytes, eax),
            }],
            kept: 2,
            total: 2,
            dropped: 0,
        }))
    };
    let text = summary(0xcc, 0x5a5a_5a5a);
    assert_eq!(text, summary(0x90, 0));
    for metadata in [
        "total=2",
        "vector=0d",
        "0028:00001234",
        "80001234",
        "Some(18)",
        "CPL 3->0",
        "VM true->false",
    ] {
        assert!(
            text.contains(metadata),
            "missing delivery metadata: {metadata}"
        );
    }
}

#[test]
fn fdc_summary_keeps_command_bytes_and_drops_parameters() {
    let event = |port, value| TraceEvent::Port {
        at: device::VClock(0),
        port,
        size: 1,
        dir: AccessDir::Write,
        value,
    };
    let mut ports = Ports::default();
    // READ DATA with its eight parameters, split across two polls, then
    // SENSE INTERRUPT; a reset in the middle of SPECIFY aborts it.
    let mut first = vec![event(0x3f2, 0x1c), event(0x3f4, 0x00), event(0x3f5, 0xe6)];
    first.extend((0..4).map(|n| event(0x3f5, 0x40 + n)));
    ports.observe(first);
    ports.observe((4..8).map(|n| event(0x3f5, 0x40 + n)).collect());
    ports.observe(vec![
        event(0x3f5, 0x08),
        event(0x3f5, 0x03),
        event(0x3f2, 0x08),
        event(0x3f5, 0x07),
    ]);
    let fdc: Vec<_> = ports
        .writes
        .iter()
        .filter(|((kind, ..), _)| kind.starts_with("FDC"))
        .map(|((kind, port, value), count)| (*kind, *port, *value, *count))
        .collect();
    assert_eq!(
        fdc,
        [
            ("FDC command", 0x3f5, 0x03, 1),
            ("FDC command", 0x3f5, 0x07, 1),
            ("FDC command", 0x3f5, 0x08, 1),
            ("FDC command", 0x3f5, 0xe6, 1),
            ("FDC data rate", 0x3f4, 0x00, 1),
            ("FDC digital output", 0x3f2, 0x08, 1),
            ("FDC digital output", 0x3f2, 0x1c, 1),
        ]
    );
}

#[test]
fn port_summary_decodes_pic_requests_pit_modes_and_vga_indices() {
    let event = |port, value| TraceEvent::Port {
        at: device::VClock(0),
        port,
        size: 1,
        dir: AccessDir::Write,
        value,
    };
    let mut ports = Ports::default();
    ports.observe(vec![
        event(0x20, 0x68),
        event(0xa0, 0x0c),
        event(0x20, 0x20),
        event(0x43, 0x36),
        event(0x43, 0x74),
        event(0x43, 0xb0),
        event(0x3c4, 2),
    ]);
    ports.observe(vec![event(0x3c5, 0xf), event(0x3ce, 8), event(0x3cf, 0xff)]);
    assert_eq!(ports.pic_special_mask, 1);
    assert_eq!(ports.pic_poll, 1);
    assert_eq!(ports.writes[&("PIC OCW2", 0x20, 0x20)], 1);
    // Only the master saw ESMM; its later EOI is counted, the earlier OCW3
    // and the slave's write are not.
    assert_eq!(ports.pic_after_special_mask, [((0x20, 0x20), 1)].into());
    assert_eq!(
        ports.pit_modes,
        [((0, 3), 1), ((1, 2), 1), ((2, 0), 1)].into()
    );
    assert_eq!(ports.vga_registers[&0x3c4], [2].into());
    assert_eq!(ports.vga_registers[&0x3ce], [8].into());
}

#[test]
fn commands_release_shift_and_keys_before_the_next_character() {
    for stage in [
        Stage::Win95,
        Stage::Win95FileCopy,
        Stage::Win95FirstBoot,
        Stage::Win95SecondBoot,
        Stage::SystemTransfer,
        Stage::Install,
        Stage::Standard,
        Stage::Enhanced,
        Stage::EnhancedRefusedByDos7,
    ] {
        let mut down = [false; 128];
        for key in command_keys(stage.command()) {
            let pressed = &mut down[(key & 0x7f) as usize];
            assert_eq!(*pressed, key & 0x80 != 0, "unbalanced command scancode");
            *pressed = key & 0x80 == 0;
        }
        assert!(down.iter().all(|pressed| !pressed));
    }
    assert_eq!(
        command_keys("c:\\3\n"),
        [0x2e, 0xae, 0x2a, 0x27, 0xa7, 0xaa, 0x2b, 0xab, 0x04, 0x84, 0x1c, 0x9c]
    );
}

#[test]
fn certificate_keys_type_digit_groups_back_to_back_and_skip_oem() {
    let mut expected = command_keys("123456");
    expected.extend([0x1c, 0x9c]);
    assert_eq!(certificate_keys("12-OEM-345-6"), expected);
    assert_eq!(certificate_keys("12-oem-345-6"), expected);
    assert!(!certificate_keys("12-OEM-345-6").contains(&0x0f), "no TAB");
}

#[test]
fn diagnostic_lines_carrying_the_product_key_are_withheld() {
    let key = Some("SYNTH-0123-ABCDE");
    for line in [
        "FINDINGS SYNTH-0123-ABCDE",
        "progress synth-0123-abcde",
        "joined SYNTH0123ABCDE here",
        "split SYNTH-01-23-ABCDE",
    ] {
        assert!(carries_secret(line, key), "missed a key form");
    }
    for line in ["FINDINGS retired=12345; SYNTH-0123", "ABCDE-0123-SYNTH"] {
        assert!(!carries_secret(line, key), "withheld an ordinary line");
    }
    // The joined digit groups, as typed on the certificate page.
    let oem = Some("12345-OEM-0012345-67890");
    for line in ["typed 12345001234567890", "typed 12345-0012345-67890"] {
        assert!(carries_secret(line, oem), "missed the typed digits");
    }
    assert!(
        !carries_secret("retired=12345678901; at 0012345", oem),
        "a single digit group must not withhold a line"
    );
    assert!(!carries_secret("anything", None));
    assert!(!carries_secret("anything", Some("")));
    assert!(!carries_secret("anything", Some("--")));
}

#[test]
fn installed_choices_use_sections_and_never_echo_values() {
    let ini = "[boot]\nshell = PROGMAN.EXE\ndisplay.drv=VGA.DRV\nnetwork.drv=\n[boot.description]\nmouse.drv=synthetic PS/2\n[386Enh]\nsecret=PRIVATE-VALUE\n";
    let choices = installed_choices(ini);
    assert!(choices.values().all(|value| *value));
    assert!(!format!("{choices:?}").contains("PRIVATE-VALUE"));
    let other_section = installed_choices("[other]\nshell=progman.exe\ndisplay.drv=vga.drv\n");
    assert!(other_section.values().all(|value| !*value));
}

#[test]
fn desktop_layout_needs_a_taskbar_over_a_background() {
    // Index 0 black, 3 teal, 7 light grey, 15 white, close to the Windows 95
    // VGA driver's 16-colour palette.
    let mut palette = vec![[0u8; 3]; 256];
    palette[3] = [0x55, 0xaa, 0xaa];
    palette[7] = [0xc3, 0xc7, 0xcb];
    palette[15] = [0xfc, 0xfc, 0xfc];
    let frame = |background: u8, edge: u8, band: u8| {
        let mut indices = vec![background; 640 * 480];
        // A dialog over the middle of the desktop.
        for row in 100..300 {
            indices[row * 640 + 100..row * 640 + 540].fill(7);
        }
        indices[453 * 640..454 * 640].fill(edge);
        indices[454 * 640..].fill(band);
        video::IndexedPixels {
            width: 640,
            height: 480,
            repeat: (1, 1),
            indices,
            palette: palette.clone().into(),
        }
    };
    assert!(desktop_layout(&frame(3, 15, 7)), "taskbar over the desktop");
    assert!(!desktop_layout(&frame(3, 7, 7)), "no highlight edge");
    assert!(
        !desktop_layout(&frame(3, 15, 0)),
        "a dark band is no taskbar"
    );
    assert!(
        !desktop_layout(&frame(3, 3, 3)),
        "background down to the bottom"
    );
    assert!(
        !desktop_layout(&frame(7, 15, 7)),
        "a grey screen is no desktop"
    );
    let mut small = frame(3, 15, 7);
    small.width = 320;
    assert!(!desktop_layout(&small), "only the 640x480 desktop counts");
}

#[test]
fn region_changes_count_only_inside_the_region() {
    let before = vec![0u8; 640 * 480];
    let mut after = before.clone();
    after[10 * 640 + 10] = 1;
    after[470 * 640 + 5] = 1;
    after[479 * 640 + 639] = 1;
    assert_eq!(changed_in(&before, &after, (0, 0, 640, 480)), 3);
    assert_eq!(changed_in(&before, &after, POINTER_CORNER), 1);
    assert_eq!(changed_in(&before, &after, (0, 0, 10, 10)), 0);
    assert_eq!(changed_in(&before, &after, (10, 10, 11, 11)), 1);
}

#[test]
fn boot_log_summary_keeps_outcomes_and_component_names_only() {
    let log = "[0011A1B2] Loading Vxd = PRIVATE\r\n\
               [0011A1B3] Initing esdi_506.pdr\r\n\
               [0011A1B4] Init Success esdi_506.pdr\r\n\
               [0011A1B5] LoadFailed  = vshare\r\n\
               [0011A1B6] Init Failure hsflop.pdr\r\n\
               LoadFail = C:\\WINDOWS\\fonts\\SOME.TTF Failure code is 0016\r\n";
    let summary = boot_log_summary(log);
    assert_eq!(summary["ESDI_506 initialising"], "true");
    assert_eq!(summary["ESDI_506 init success"], "true");
    assert_eq!(summary["ESDI_506 failure"], "false");
    assert_eq!(summary["failure lines"], "3");
    assert_eq!(
        summary["failed components"],
        "{\"hsflop.pdr\", \"some.ttf\", \"vshare\"}"
    );
    assert!(!format!("{summary:?}")
        .to_ascii_uppercase()
        .contains("PRIVATE"));
}

#[test]
fn fat16_volume_flags_read_the_second_fat_entry() {
    let volume = |entry: u16| {
        let mut bytes = vec![0u8; 1024];
        bytes[11..13].copy_from_slice(&512u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&1u16.to_le_bytes());
        bytes[512..514].copy_from_slice(&0xfff8u16.to_le_bytes());
        bytes[514..516].copy_from_slice(&entry.to_le_bytes());
        bytes
    };
    assert_eq!(fat16_volume_flags(&volume(0xffff)), Some((true, true)));
    assert_eq!(fat16_volume_flags(&volume(0x7fff)), Some((false, true)));
    assert_eq!(fat16_volume_flags(&volume(0xbfff)), Some((true, false)));
    assert_eq!(fat16_volume_flags(&volume(0xffff)[..515]), None);
}

#[test]
fn ios_log_units_keep_only_real_mode_unit_numbers() {
    let log = "Unit number 00 going through real mode drivers.\r\n\
               Unit number 01 going through real mode drivers.\r\n\
               Driver Name:  private.sys\r\n\
               Unit number 02 is fine\r\n";
    assert_eq!(
        ios_log_real_mode_units(log),
        ["00", "01"].map(String::from).into()
    );
}

#[test]
fn desktop_script_balances_keys_and_buttons() {
    for (_, input) in DESKTOP_SCRIPT {
        match input {
            Input::Keys(keys) => {
                let mut down = [false; 128];
                for key in keys {
                    let pressed = &mut down[(key & 0x7f) as usize];
                    assert_eq!(*pressed, key & 0x80 != 0, "unbalanced scancode");
                    *pressed = key & 0x80 == 0;
                }
                assert!(down.iter().all(|pressed| !pressed));
            }
            Input::Mouse(packets) => {
                assert_eq!(
                    packets.last().map(|packet| packet.2),
                    Some(0),
                    "buttons released"
                );
                assert!(packets
                    .iter()
                    .all(|(dx, dy, _)| dx.abs() <= 255 && dy.abs() <= 255));
            }
        }
    }
}
