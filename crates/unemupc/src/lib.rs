// SPDX-License-Identifier: AGPL-3.0-or-later
//! The PC core's standalone command line, without a store or frontend;
//! `backlog/docs/design/core-contract.md` §12.8. Machine runs use the same
//! runners as the PC goldens, and the frontend calls this library too.

#![forbid(unsafe_code)]

pub mod journal;
pub mod run;

pub use run::FirmwareStore;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

/// A parsed machine command.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Journal(journal::JournalCommand),
    Boot,
    Bios {
        path: PathBuf,
        options: assembly::AssemblyOptions,
    },
    Check {
        rom_dirs: Vec<PathBuf>,
        path: PathBuf,
        options: assembly::AssemblyOptions,
    },
    Run {
        store: FirmwareStore,
        rom_dirs: Vec<PathBuf>,
        path: PathBuf,
        max_steps: u64,
        idle_limit: u64,
        /// `--diag-at N`: the step at which the diagnostic report goes to
        /// stderr before the run goes on.
        diag_at: Option<u64>,
        /// `--trace-exceptions`: the recording of the exception and
        /// interrupt deliveries, the last entries, goes to stderr.
        trace_exceptions: bool,
        print: Option<String>,
        options: assembly::AssemblyOptions,
    },
}

/// Parse machine words without accepting frontend store options.
pub fn parse(args: &[OsString]) -> Option<Result<Command, &'static str>> {
    parse_words(args, false)
}

/// Parse the frontend's file run, including its firmware-store selection.
/// The library only carries that selection; the frontend prepares the file.
pub fn parse_with_store(args: &[OsString]) -> Option<Result<Command, &'static str>> {
    parse_words(args, true)
}

fn parse_words(args: &[OsString], with_store: bool) -> Option<Result<Command, &'static str>> {
    match machine_words(args, with_store) {
        Ok(None)
            if args.first().is_some_and(|arg| arg == "media")
                && !args.get(1).is_some_and(|arg| arg == "create") =>
        {
            Some(journal::parse(args).map(Command::Journal))
        }
        result => result.transpose(),
    }
}

fn machine_words(args: &[OsString], with_store: bool) -> Result<Option<Command>, &'static str> {
    let mut options = assembly::AssemblyOptions::default();
    let mut frontend = Vec::new();
    let mut rom_dirs = Vec::new();
    let mut arguments = args.iter();
    while let Some(arg) = arguments.next() {
        if arg == "--allow-unlisted-firmware" {
            options.allow_unlisted_firmware = true;
        } else if arg == "--rom-dir" {
            let path = arguments
                .next()
                .filter(|value| !value.is_empty() && !value.as_encoded_bytes().starts_with(b"--"))
                .ok_or("missing ROM directory")?;
            rom_dirs.push(PathBuf::from(path));
        } else {
            frontend.push(arg);
        }
    }
    if !rom_dirs.is_empty()
        && !frontend
            .first()
            .is_some_and(|arg| matches!(arg.to_str(), Some("run" | "machine")))
    {
        return Err("--rom-dir requires run or machine check");
    }
    if !with_store && frontend.len() == 1 && frontend[0] == "--boot" {
        return Ok(Some(Command::Boot));
    }
    if !with_store && frontend.len() == 2 && frontend[0] == "--bios" {
        return Ok(Some(Command::Bios {
            path: frontend[1].into(),
            options,
        }));
    }
    if !with_store && frontend.first().is_some_and(|arg| *arg == "machine") {
        return if frontend.len() == 3
            && frontend[1] == "check"
            && !frontend[2].is_empty()
            && !frontend[2].as_encoded_bytes().starts_with(b"--")
        {
            Ok(Some(Command::Check {
                rom_dirs,
                path: frontend[2].into(),
                options,
            }))
        } else {
            Err("expected machine check FILE")
        };
    }
    if frontend.first().is_some_and(|arg| *arg == "run") {
        let (mut path, mut max_steps, mut idle_limit, mut print) = (None, None, None, None);
        let mut diag_at = None;
        let mut trace_exceptions = false;
        let mut store = None;
        let mut args = frontend[1..].iter();
        while let Some(arg) = args.next() {
            if *arg == "--trace-exceptions" {
                if std::mem::replace(&mut trace_exceptions, true) {
                    return Err("unknown or duplicate run option");
                }
                continue;
            }
            if with_store && *arg == "--no-store" {
                if store.replace(FirmwareStore::Disabled).is_some() {
                    return Err("duplicate or conflicting store option");
                }
                continue;
            }
            let value = args
                .next()
                .filter(|value| !value.is_empty() && !value.as_encoded_bytes().starts_with(b"--"))
                .ok_or("missing option value")?;
            match arg.to_str() {
                Some("--store") if with_store && store.is_none() => {
                    store = Some(FirmwareStore::Root(PathBuf::from(value)))
                }
                Some("--machine") if path.is_none() => path = Some(PathBuf::from(value)),
                Some("--max-steps") if max_steps.is_none() => max_steps = Some(decimal(value)?),
                Some("--idle-limit") if idle_limit.is_none() => idle_limit = Some(decimal(value)?),
                Some("--diag-at") if diag_at.is_none() => diag_at = Some(decimal(value)?),
                Some("--print") if print.is_none() => {
                    print = Some(value.to_str().ok_or("invalid card id")?.to_owned())
                }
                _ => return Err("unknown or duplicate run option"),
            }
        }
        return Ok(Some(Command::Run {
            store: store.unwrap_or(FirmwareStore::Default),
            rom_dirs,
            path: path.ok_or("run requires --machine FILE")?,
            max_steps: max_steps.unwrap_or(pc_harness::BIOS_MAX_RETIRED),
            idle_limit: idle_limit.unwrap_or(pc_harness::BIOS_IDLE_LIMIT),
            diag_at,
            trace_exceptions,
            print,
            options,
        }));
    }
    Ok(None)
}

fn decimal(value: &std::ffi::OsStr) -> Result<u64, &'static str> {
    value
        .to_str()
        .filter(|text| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or("expected an unsigned decimal integer")?
        .parse()
        .map_err(|_| "integer is out of range")
}

/// Execute a machine command with the host-only preparation supplied by the
/// caller. This keeps frontend firmware-package lookup out of the core crate.
pub fn execute_with_loader(
    command: Command,
    output: &mut impl Write,
    errors: &mut impl Write,
    load: impl FnOnce(
        &std::path::Path,
        assembly::AssemblyOptions,
        &[PathBuf],
        &mut dyn Write,
    ) -> anyhow::Result<machine_file::LoadedMachine>,
) -> anyhow::Result<u8> {
    let line = |error: &dyn std::fmt::Display| anyhow::Error::msg(error.to_string());
    match command {
        Command::Journal(command) => Ok(journal::run(command, output, errors)),
        Command::Boot => {
            let serial = pc_harness::run_boot(pc_harness::BOOT_PROGRAM).map_err(|e| line(&e))?;
            output.write_all(&serial).map_err(|e| line(&e))?;
            Ok(0)
        }
        Command::Bios { path, options } => run::run_bios(&path, options, output, errors)
            .map_err(|error| anyhow::Error::msg(format!("Error: {error:?}"))),
        Command::Check {
            path,
            options,
            rom_dirs,
        } => run::check_machine(&path, options, &rom_dirs, errors).map_err(|e| line(&e)),
        Command::Run {
            store: _,
            path,
            rom_dirs,
            max_steps,
            idle_limit,
            diag_at,
            trace_exceptions,
            print,
            options,
        } => {
            let loaded = load(&path, options, &rom_dirs, errors).map_err(|e| line(&e))?;
            run::run_machine(
                loaded,
                (max_steps, idle_limit, diag_at, trace_exceptions),
                print.as_deref(),
                output,
                errors,
            )
            .map_err(|e| line(&e))
        }
    }
}
/// Run the standalone command line, returning its process exit code.
pub fn run(args: &[OsString], output: &mut impl Write, errors: &mut impl Write) -> u8 {
    let result = if args.is_empty() {
        output
            .write_all(usage().as_bytes())
            .map(|()| 0)
            .map_err(anyhow::Error::from)
    } else if matches!(args, [arg] if arg == "--version" || arg == "--about") {
        output
            .write_all(notice().as_bytes())
            .map(|()| 0)
            .map_err(anyhow::Error::from)
    } else {
        let command = match parse(args).unwrap_or(Err("unknown or extra arguments")) {
            Ok(command) => command,
            Err(error) => {
                let _ = writeln!(errors, "{}{error}", usage());
                return 2;
            }
        };
        execute_with_loader(command, output, errors, |path, options, dirs, errors| {
            run::load_machine(path, options, dirs, errors)
        })
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            let _ = writeln!(errors, "{error:#}");
            1
        }
    }
}

/// The core's identity and the notices of the components compiled into it.
pub fn notice() -> String {
    use unemu_core_api::Core;
    let description = unemu_pc_core::PcCore.description();
    let mut text = format!(
        "{} {}\ncore {} {} ({}), contract {}\nAGPL-3.0-or-later\nsource: {}\n{}Third-party licence texts: THIRD-PARTY-LICENSES.md\n",
        description.id, description.version, description.id, description.version,
        description.name, description.implements,
        source_pointer(option_env!("UNEMUPC_SOURCE_REV")),
        commit_line(option_env!("UNEMUPC_SOURCE_COMMIT")),
    );
    for notice in description.notices {
        text.push('\n');
        text.push_str(notice.text);
        text.push('\n');
    }
    text
}

fn source_pointer(revision: Option<&str>) -> String {
    let repository = env!("CARGO_PKG_REPOSITORY");
    match revision.filter(|revision| !revision.is_empty()) {
        Some(revision) => format!("{repository}/tree/{revision}"),
        None => format!("{repository}, version {}", env!("CARGO_PKG_VERSION")),
    }
}

fn commit_line(commit: Option<&str>) -> String {
    match commit.filter(|commit| !commit.is_empty()) {
        Some(commit) => format!("commit: {commit}\n"),
        None => String::new(),
    }
}

/// The frontend's remaining machine usage lines, before image import.
#[macro_export]
macro_rules! machine_usage {
    ($($tail:literal),* $(,)?) => {
        concat!(
            "       unemu run --machine FILE [--store DIR | --no-store] [--rom-dir DIR]... [--max-steps N] [--idle-limit N] [--diag-at N] [--trace-exceptions] [--print CARD] [--allow-unlisted-firmware]\n",
            "         --diag-at N writes the machine's diagnostic report to stderr when the run reaches step N (the unit of --max-steps), then runs on\n",
            "         --trace-exceptions writes the exceptions and interrupts the guest takes to stderr when the run ends: the last 10000 entries, each with the registers of the interrupted code (the bytes at CS:IP as at the end of each run slice), consecutive identical deliveries folded into one with a count and the last one's registers, and a summary per vector\n", $($tail),*)
    };
}

pub const FRONTEND_MACHINE_USAGE: &str = machine_usage!();

/// Journal usage shared with the frontend; keep command words and ordering equal.
pub const JOURNAL_USAGE: &str = concat!(
    "       unemu media generations --base IMG --journal J\n",
    "       unemu media verify --base IMG --journal J [--repair-tail --backup PATH]\n",
    "       unemu media rollback --base IMG --journal J --to N --backup PATH\n",
    "       unemu media commit (alias compact) --base IMG --journal J --to N --output OUT\n",
);

/// Standalone usage names the core and offers no frontend store options.
pub fn usage() -> String {
    let text = format!(
        concat!(
            "       unemu --boot (Real16 timer demo)\n",
            "       unemu --version | --about\n",
            "       unemu --bios PATH [--allow-unlisted-firmware]\n",
            "       unemu machine check FILE [--allow-unlisted-firmware] [--rom-dir DIR]...\n",
            "{}{}",
            "DOS boot and interactive sessions: see crates/pc-harness/tests/*dos*golden.rs\n",
        ),
        FRONTEND_MACHINE_USAGE, JOURNAL_USAGE
    );
    text.replacen("       ", "usage: ", 1)
        .replace("unemu", "unemupc")
        .replace(" [--store DIR | --no-store]", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_metadata_revision_set() {
        assert_eq!(
            source_pointer(Some("v9.9.9")),
            format!("{}/tree/v9.9.9", env!("CARGO_PKG_REPOSITORY"))
        );
    }

    #[test]
    fn source_metadata_revision_empty() {
        assert_eq!(
            source_pointer(Some("")),
            format!(
                "{}, version {}",
                env!("CARGO_PKG_REPOSITORY"),
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn source_metadata_revision_absent() {
        assert_eq!(
            source_pointer(None),
            format!(
                "{}, version {}",
                env!("CARGO_PKG_REPOSITORY"),
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn source_metadata_commit_set() {
        assert_eq!(
            commit_line(Some("0123456789abcdef0123456789abcdef01234567")),
            "commit: 0123456789abcdef0123456789abcdef01234567\n"
        );
    }

    #[test]
    fn source_metadata_commit_empty() {
        assert_eq!(commit_line(Some("")), "");
    }

    #[test]
    fn source_metadata_commit_absent() {
        assert_eq!(commit_line(None), "");
    }

    /// The PC command a command line parses to.
    fn pc(args: &[OsString]) -> Command {
        parse(args).expect("a PC command").unwrap()
    }

    #[test]
    fn file_run_parser_preserves_policy_defaults_and_explicit_limits() {
        let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            pc(&args(&["run", "--machine", "at.toml"])),
            Command::Run {
                store: FirmwareStore::Default,
                rom_dirs: Vec::new(),
                path: "at.toml".into(),
                max_steps: 20_000_000,
                idle_limit: 1_000_000,
                diag_at: None,
                trace_exceptions: false,
                print: None,
                options: assembly::AssemblyOptions::default(),
            }
        );
        assert_eq!(
            pc(&args(&[
                "run",
                "--machine",
                "at.toml",
                "--max-steps",
                "42",
                "--idle-limit",
                "0",
                "--diag-at",
                "7",
                "--trace-exceptions",
                "--print",
                "screen",
                "--allow-unlisted-firmware"
            ])),
            Command::Run {
                store: FirmwareStore::Default,
                rom_dirs: Vec::new(),
                path: "at.toml".into(),
                max_steps: 42,
                idle_limit: 0,
                diag_at: Some(7),
                trace_exceptions: true,
                print: Some("screen".into()),
                options: assembly::AssemblyOptions {
                    allow_unlisted_firmware: true
                },
            }
        );
    }

    #[test]
    fn bios_parser_preserves_firmware_flags_and_boot_mode() {
        for args in [
            vec!["--bios", "firmware.bin"],
            vec!["--bios", "firmware.bin", "--allow-unlisted-firmware"],
            vec!["--allow-unlisted-firmware", "--bios", "firmware.bin"],
            vec![
                "--bios",
                "--allow-unlisted-firmware",
                "firmware.bin",
                "--allow-unlisted-firmware",
            ],
        ] {
            let allow = args.contains(&"--allow-unlisted-firmware");
            let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            assert_eq!(
                pc(&args),
                Command::Bios {
                    path: PathBuf::from("firmware.bin"),
                    options: assembly::AssemblyOptions {
                        allow_unlisted_firmware: allow
                    },
                }
            );
        }
        assert_eq!(
            pc(&["--allow-unlisted-firmware".into(), "--boot".into()]),
            Command::Boot
        );
        for args in [
            vec!["--bios"],
            vec!["--bios", "firmware.bin", "extra"],
            vec!["--allow-unlisted-firmware"],
        ] {
            let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            // `--allow-unlisted-firmware` alone is no PC command; the others are refused ones.
            assert!(parse(&args).is_none_or(|parsed| parsed.is_err()));
            assert_eq!(
                crate::run(&args, &mut std::io::sink(), &mut std::io::sink()),
                2
            );
        }
    }
    #[test]
    fn rom_directory_option_is_repeatable_for_launch_and_check() {
        for prefix in [
            vec!["run", "--machine", "machine.toml"],
            vec!["machine", "check", "machine.toml"],
        ] {
            let mut args: Vec<OsString> = prefix.into_iter().map(Into::into).collect();
            args.extend(
                ["--rom-dir", "first", "--rom-dir", "second"]
                    .into_iter()
                    .map(OsString::from),
            );
            let roots = match pc(&args) {
                Command::Run { rom_dirs, .. } | Command::Check { rom_dirs, .. } => rom_dirs,
                _ => panic!("machine command expected"),
            };
            assert_eq!(roots, [PathBuf::from("first"), PathBuf::from("second")]);
        }
    }
}
