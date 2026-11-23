// SPDX-License-Identifier: AGPL-3.0-or-later
//! Standalone window arguments; see backlog/docs/commands.md § Viewing a machine.

use machine_view::pacing::Speed;
use std::{ffi::OsString, path::PathBuf};

pub const USAGE: &str = "usage: unemupc-view --machine FILE [--rom-dir DIR]... [--allow-unlisted-firmware] [--speed 1|2|4|unlimited]\n\
controls: ctrl+alt+r reset, ctrl+alt+p pause/resume, ctrl+alt+q quit\n\
pointer: click to capture; ctrl+alt+g releases (command+g on macOS)\n";

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub machine: PathBuf,
    pub rom_dirs: Vec<PathBuf>,
    pub allow_unlisted_firmware: bool,
    pub speed: Speed,
}

impl Args {
    pub fn parse(words: &[OsString]) -> Result<Self, &'static str> {
        let mut machine = None;
        let mut rom_dirs = Vec::new();
        let mut allow_unlisted_firmware = false;
        let mut speed = Speed::Normal;
        let mut speed_seen = false;
        let mut words = words.iter();
        while let Some(word) = words.next() {
            match word.to_str() {
                Some("--machine") if machine.is_none() => {
                    machine = Some(PathBuf::from(words.next().ok_or("--machine needs a file")?));
                }
                Some("--rom-dir") => {
                    rom_dirs.push(PathBuf::from(
                        words.next().ok_or("--rom-dir needs a directory")?,
                    ));
                }
                Some("--allow-unlisted-firmware") if !allow_unlisted_firmware => {
                    allow_unlisted_firmware = true;
                }
                Some("--speed") if !speed_seen => {
                    speed_seen = true;
                    speed = match words.next().and_then(|word| word.to_str()) {
                        Some("1") => Speed::Normal,
                        Some("2") => Speed::Double,
                        Some("4") => Speed::Quadruple,
                        Some("unlimited") => Speed::Unlimited,
                        _ => return Err("speed must be 1, 2, 4 or unlimited"),
                    };
                }
                _ => return Err("unknown or repeated argument"),
            }
        }
        Ok(Self {
            machine: machine.ok_or("--machine is required")?,
            rom_dirs,
            allow_unlisted_firmware,
            speed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Args, &'static str> {
        Args::parse(&words.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn defaults_and_repeatable_firmware_roots() {
        let args = parse(&["--machine", "pc.toml"]).unwrap();
        assert_eq!(args.speed, Speed::Normal);
        assert!(!args.allow_unlisted_firmware);
        let args = parse(&[
            "--rom-dir",
            "one",
            "--machine",
            "pc.yaml",
            "--rom-dir",
            "two",
            "--allow-unlisted-firmware",
            "--speed",
            "unlimited",
        ])
        .unwrap();
        assert_eq!(args.machine, PathBuf::from("pc.yaml"));
        assert_eq!(args.rom_dirs, [PathBuf::from("one"), PathBuf::from("two")]);
        assert!(args.allow_unlisted_firmware);
        assert_eq!(args.speed, Speed::Unlimited);
    }

    #[test]
    fn speed_matches_the_shared_session_choices() {
        for (word, speed) in [
            ("1", Speed::Normal),
            ("2", Speed::Double),
            ("4", Speed::Quadruple),
        ] {
            assert_eq!(
                parse(&["--machine", "pc.toml", "--speed", word])
                    .unwrap()
                    .speed,
                speed
            );
        }
        for word in ["0", "3", "-1", "NaN", "1.5", ""] {
            assert!(parse(&["--machine", "pc.toml", "--speed", word]).is_err());
        }
    }

    #[test]
    fn refuses_missing_unknown_and_repeated_arguments() {
        for words in [
            vec![],
            vec!["--machine"],
            vec!["--rom-dir"],
            vec!["--machine", "pc.toml", "--speed"],
            vec!["--machine", "pc.toml", "extra"],
            vec!["--machine", "one", "--machine", "two"],
            vec!["--machine", "pc.toml", "--speed", "1", "--speed", "2"],
            vec![
                "--machine",
                "pc.toml",
                "--allow-unlisted-firmware",
                "--allow-unlisted-firmware",
            ],
        ] {
            assert!(parse(&words).is_err(), "{words:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn paths_need_not_be_unicode() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'p', 0xff]);
        let args = Args::parse(&[
            "--machine".into(),
            path.clone(),
            "--rom-dir".into(),
            path.clone(),
        ])
        .unwrap();
        assert_eq!(args.machine.as_os_str(), path);
        assert_eq!(args.rom_dirs[0].as_os_str(), path);
    }
}
