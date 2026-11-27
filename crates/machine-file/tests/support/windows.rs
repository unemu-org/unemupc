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
        // ATA data words can contain batch-file secrets. Retain access metadata
        // without retaining payloads in any printable diagnostic structure.
        self.recent.extend(events.iter().skip(events.len().saturating_sub(64)).map(|event| {
            match event {
                TraceEvent::Port { at, port: port @ (0x1f0 | 0x170), size, dir, .. } =>
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
            Self::Win95 => "c:\\win95\\setup /is /iq c:\\msbatch.inf\n",
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
            Self::Install | Self::Standard | Self::Enhanced | Self::EnhancedRefusedByDos7
        )
    }

    fn windows(self) -> bool {
        matches!(self, Self::Standard | Self::Enhanced)
    }
}

/// The private disk after a stage, and whether stage 1 found a DOS 7 kernel
/// (its batch file prints a marker in the DOS 7 branch).
pub struct Staged {
    pub image: PathBuf,
    pub dos7: bool,
}

fn decoded_text(assembled: &assembly::AssembledMachine) -> String {
    match assembled.capture_text(DeviceId::Vga).unwrap().frame {
        video::Frame::Text(grid) => grid.rows_as_strings().join("\n"),
        _ => String::new(),
    }
}

/// A digest of the captured graphics frame, to tell a settled screen from one
/// still being drawn. `None` in text mode.
fn frame_digest(assembled: &assembly::AssembledMachine) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    match assembled.capture_text(DeviceId::Vga).unwrap().frame {
        video::Frame::Pixels(pixels) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            pixels.indices.hash(&mut hasher);
            pixels.palette.hash(&mut hasher);
            Some(hasher.finish())
        }
        video::Frame::Text(_) => None,
    }
}

fn first_screen(assembled: &assembly::AssembledMachine, text: &str, stage: Stage) -> bool {
    let mut mode = [0];
    assembled.machine.cpu().read_mem(0x449, &mut mode).unwrap();
    if let video::Frame::Pixels(pixels) = assembled.capture_text(DeviceId::Vga).unwrap().frame {
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

fn report_installed_choices(assembled: &mut assembly::AssembledMachine) {
    // Inspect the merged sector view through the drive's existing overlay.
    // mtools reads a new private snapshot, never the immutable mounted base.
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
    let directory = machine_file::test_support::TempDir::new("private-win31-inspection");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private inspection directory permissions failed");
    }
    let snapshot = directory.path().join("inspection.img");
    std::fs::write(&snapshot, bytes).expect("private inspection snapshot failed");
    let output = Command::new("mtype")
        .arg("-i")
        .arg(format!("{}@@32256", snapshot.display()))
        .arg("::/WINDOWS/SYSTEM.INI")
        .env("MTOOLSRC", "/dev/null")
        .output()
        .expect("start private settings inspection");
    eprintln!("FINDINGS SYSTEM.INI readable={}", output.status.success());
    if output.status.success() {
        eprintln!(
            "FINDINGS installed choices (derived, no source text): {:?}",
            installed_choices(&String::from_utf8_lossy(&output.stdout))
        );
    }
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

/// Run slices (of ten thousand virtual-time units each) between two screen
/// captures: about one million guest instructions.
const SCREEN_POLL_SLICES: u64 = 100;

/// Run one stage on a new machine, starting from the previous stage's disk or
/// a freshly built one, and assert that it reached its boundary. Returns
/// `None` without running when the private-media variables are unset.
pub fn run(stage: Stage, previous: Option<Staged>) -> Option<Staged> {
    let source = if stage == Stage::Win95 {
        "UNEMUPC_USER_WIN95_DIR"
    } else {
        "UNEMUPC_USER_WIN31_DIR"
    };
    let (Some(_), Some(floppy)) = (
        std::env::var_os(source),
        std::env::var_os("UNEMUPC_USER_DOS_FLOPPY"),
    ) else {
        eprintln!("skipping: set {source} and UNEMUPC_USER_DOS_FLOPPY");
        return None;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dos7 = previous.as_ref().is_some_and(|disk| disk.dos7);
    let image = previous.map(|disk| disk.image).unwrap_or_else(|| {
        let built = Command::new("bash")
            .arg(root.join("scripts/user-media-disk.sh"))
            .arg(if stage == Stage::Win95 {
                "--win95"
            } else {
                "--win31"
            })
            .output()
            .expect("start private disk builder");
        assert!(built.status.success(), "private disk builder failed");
        PathBuf::from(String::from_utf8(built.stdout).unwrap().trim())
    });
    let directory = image.parent().unwrap();
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
            image.clone()
        } else {
            drive.write = WriteMode::ReadOnly;
            PathBuf::from(&floppy)
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
        eprintln!("{stage:?} mounted journal generation={generation:?}");
        if stage.boots_hard_disk() {
            assert!(
                generation.is_some_and(|id| id >= 1),
                "a prior stage must close its journal"
            );
        }
    }
    assembled.machine.enable_delivery_log().unwrap();
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
    let mut dos_launch = None;
    let mut dos_prompt = false;
    let mut exit_sent = None;
    let mut first_graphics = None;
    let mut setup_confirmed = false;
    let mut execution_error = false;
    let reason;
    // The retired budget is independent of wall time. A second virtual-time
    // ceiling bounds HLT waits, which can consume no retired instructions.
    let mut slice = 0u64;
    loop {
        slice += 1;
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
        let text = decoded_text(&assembled);
        if !boot_menu_selected && text.contains("Startup Menu") && text.contains("without CD-ROM") {
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x03, 0x83, 0x1c, 0x9c]);
            boot_menu_selected = true;
            eprintln!(
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
            eprintln!(
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
        if stage == Stage::Install && launched.is_some() && first_screen(&assembled, &text, stage) {
            let first = *first_graphics.get_or_insert(retired);
            if !setup_confirmed && retired.saturating_sub(first) >= 5_000_000 {
                assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x1c, 0x9c]);
                setup_confirmed = true;
                eprintln!(
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
                let digest = (in_mode && first_screen(&assembled, &text, stage))
                    .then(|| frame_digest(&assembled))
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
                eprintln!(
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
                eprintln!(
                    "{stage:?} DOS prompt / EXIT: retired={retired}; wall={:?}",
                    started.elapsed()
                );
            }
            // After EXIT, a populated frame must hold at every poll for five
            // million instructions while the CPU stays in the stage's mode.
            if exit_sent.is_some() {
                if in_mode && first_screen(&assembled, &text, stage) {
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
            eprintln!(
                "{stage:?} DOS prompt / command: retired={retired}; wall={:?}",
                started.elapsed()
            );
        } else if launched.is_some()
            && match stage {
                Stage::Win95 => first_screen(&assembled, &text, stage),
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
            }
        {
            success = true;
            reason = format!("{stage:?} boundary");
            break;
        }
        let writes = ports
            .writes
            .iter()
            .filter(|((kind, _, value), _)| {
                *kind == "ATA command" && matches!(*value, 0x30 | 0x31 | 0xc5)
            })
            .map(|(_, count)| *count)
            .sum::<u64>();
        if launched.is_none() || writes != disk_writes {
            disk_writes = writes;
            last_disk_write = retired;
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
        if retired >= next_progress {
            eprintln!(
                "{stage:?} progress: retired={retired}; wall={:?}; command={}; protected={}; paging={}; CPL={}",
                started.elapsed(),
                launched.is_some(), system.protected(), system.paging(), system.cpl
            );
            next_progress += 25_000_000;
        }
        let budget = if launched.is_some() {
            if stage == Stage::Install {
                1_000_000_000
            } else {
                300_000_000
            }
        } else {
            200_000_000
        };
        if retired.saturating_sub(launched.unwrap_or(0)) >= budget
            || assembled.machine.virtual_time()
                >= if stage == Stage::Install {
                    2_000_000_000
                } else {
                    600_000_000
                }
        {
            reason = "instruction/virtual-time budget exhausted".into();
            break;
        }
    }
    if let video::Frame::Pixels(pixels) = assembled.capture_text(DeviceId::Vga).unwrap().frame {
        let mut palette = BTreeSet::new();
        palette.extend(pixels.indices.iter().copied());
        let mut mode = [0];
        assembled.machine.cpu().read_mem(0x449, &mut mode).unwrap();
        eprintln!("FINDINGS scanout: {}x{}; distinct indices={}; BDA mode={:02x}; setup ENTER={setup_confirmed}",
            pixels.width, pixels.height, palette.len(), mode[0]);
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
    eprintln!("FINDINGS stage={stage:?}; DOS 7={dos7}; after launch: protected={protected_seen}, paging={paging_seen}, V86={v86_seen}, V86 exit delivery={v86_exit_seen}; File/Run sent={}; DOS prompt={dos_prompt}; EXIT sent={}; frame held after EXIT={}", dos_launch.is_some(), exit_sent.is_some(), shell_back.is_some());
    eprintln!("FINDINGS: {reason}; command={}; mode={mode}; CS:EIP={:04x}:{:08x}; linear={:08x}; CPL={}; CR0={:08x}; retired={}; wall={:?}", launched.is_some(), state.cs().selector, eip, linear, system.cpl, system.cr0, state.retired, started.elapsed());
    // Guest bytes (code, registers, screen text, port data) stay out of the
    // log; commands, register indices, addresses and counts are enough to
    // name the device or CPU path that stopped.
    eprintln!(
        "FINDINGS counters: device accesses={:?}; last port access (port, direction, size)={:x?}",
        ports.accesses, ports.last_access
    );
    eprintln!("FINDINGS command/register writes (hex)={:x?}", ports.writes);
    eprintln!("FINDINGS PIC special-mask writes={}; poll requests={}; PIT (channel, mode)={:?}; VGA indexed registers={:x?}", ports.pic_special_mask, ports.pic_poll, ports.pit_modes, ports.vga_registers);
    eprintln!(
        "FINDINGS PIC command writes after the first special-mask write (port, value)={:x?}",
        ports.pic_after_special_mask
    );
    // Only the 8259 lines of the diagnostic report: register values, no
    // guest code or memory bytes.
    let report = assembled.diagnostic_report("stage end").to_string();
    for line in report.lines().filter(|line| line.starts_with("pic ")) {
        eprintln!("FINDINGS {line}");
    }
    eprintln!(
        "FINDINGS deliveries: {}",
        delivery_summary(assembled.machine.recent_deliveries())
    );
    if stage.boots_hard_disk() {
        report_installed_choices(&mut assembled);
    }
    if execution_error {
        // A CPU failure must not be published as a successfully finished session.
        // Checkpoint only completed sectors for diagnosis/restart; no CPU state
        // is resumed by a disk generation.
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
    eprintln!("{stage:?} private stage directory: {}", directory.display());
    assert!(
        success,
        "{stage:?} recorded its first stop before the required boundary"
    );
    Some(Staged { image, dos7 })
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
fn installed_choices_use_sections_and_never_echo_values() {
    let ini = "[boot]\nshell = PROGMAN.EXE\ndisplay.drv=VGA.DRV\nnetwork.drv=\n[boot.description]\nmouse.drv=synthetic PS/2\n[386Enh]\nsecret=PRIVATE-VALUE\n";
    let choices = installed_choices(ini);
    assert!(choices.values().all(|value| *value));
    assert!(!format!("{choices:?}").contains("PRIVATE-VALUE"));
    let other_section = installed_choices("[other]\nshell=progman.exe\ndisplay.drv=vga.drv\n");
    assert!(other_section.values().all(|value| !*value));
}
