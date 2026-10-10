//! The programs Claude Code's sandbox needs, found on `PATH` by platform.
//!
//! Gathering takes the `PATH` value as supplied and the target operating
//! system's name, looks at each folder's entry for each program and starts
//! nothing. Judging is pure: it switches on the operating system's name with
//! `linux` and `macos` as the only named platforms.
//!
//! Host facts, read on 2026-10-08 from code.claude.com's sandboxing page and
//! Claude Code 2.1.294: on Linux and WSL2 the sandbox needs `bubblewrap`
//! (the program `bwrap`) and `socat`, installed with `apt-get install
//! bubblewrap socat` or `dnf install bubblewrap socat`. macOS uses its
//! built-in Seatbelt and needs nothing. Native Windows and WSL1 cannot
//! sandbox commands.
//!
//! Not detected: WSL1, a container that blocks bubblewrap, Ubuntu's
//! AppArmor rule on user namespaces, ripgrep (bundled with Claude Code's
//! native binary) and the optional seccomp filter. A found program shows
//! that a file is there, not that the sandbox starts.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::host_artifacts::coverage::Mechanism;

/// A program the Linux sandbox needs and the package that provides it.
struct Required {
    name: &'static str,
    package: &'static str,
}

const LINUX: [Required; 2] = [
    Required {
        name: "bwrap",
        package: "bubblewrap",
    },
    Required {
        name: "socat",
        package: "socat",
    },
];

/// One `PATH` entry's file for a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The `PATH` entry, as written.
    pub folder: PathBuf,
    /// Whether the file there is a regular file, with links followed.
    pub regular_file: bool,
    /// The file's mode bits; 0 when nothing is there.
    pub mode: u32,
}

/// What a program search found: one candidate per `PATH` entry, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    /// The program's name.
    pub program: String,
    /// The entries, in `PATH` order.
    pub candidates: Vec<Candidate>,
}

/// What was gathered about the platform and its sandbox programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// The target operating system, `std::env::consts::OS`.
    pub os: String,
    /// A search for each program the Linux sandbox needs.
    pub searches: Vec<Search>,
}

/// Looks for each sandbox program in each entry of `path`. A program is
/// searched whatever the platform, since the judgement decides which ones
/// the platform needs. Nothing is opened or run.
pub fn gather(path: Option<&OsStr>, os: &str) -> Observed {
    let folders: Vec<PathBuf> = path
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default();
    let searches = LINUX
        .iter()
        .map(|required| Search {
            program: required.name.to_owned(),
            candidates: folders
                .iter()
                .map(|folder| {
                    let meta = std::fs::metadata(folder.join(required.name));
                    Candidate {
                        folder: folder.clone(),
                        regular_file: meta.as_ref().is_ok_and(|meta| meta.is_file()),
                        mode: meta.map_or(0, |meta| meta.permissions().mode()),
                    }
                })
                .collect(),
        })
        .collect();
    Observed {
        os: os.to_owned(),
        searches,
    }
}

/// A program the platform needs and where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The program's name.
    pub name: &'static str,
    /// The package that provides it.
    pub package: &'static str,
    /// The file found, or none when no `PATH` entry holds it.
    pub found: Option<PathBuf>,
}

/// The sentence for a required program that no `PATH` entry holds: the
/// program, its package, the two install commands and that the lookup used
/// this command's own `PATH`. The doctor and `baley install` both use it, so
/// neither words the fix differently.
pub fn missing_sentence(program: &Program) -> String {
    format!(
        "{} is missing from PATH, so the sandbox cannot run; install {} with `apt-get install bubblewrap socat` or `dnf install bubblewrap socat` (this lookup used this command's PATH, which may differ from the one Claude Code runs with)",
        program.name, program.package
    )
}

/// What the platform is to Claude Code's sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Linux: bubblewrap and socat.
    Linux,
    /// macOS: the built-in Seatbelt.
    MacOs,
    /// Any other platform: the sandbox does not run.
    Other,
}

/// The judged prerequisites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// The operating system's name as observed.
    pub os: String,
    /// What the platform is to the sandbox.
    pub kind: Kind,
    /// The programs the platform requires; empty off Linux.
    pub required: Vec<Program>,
}

impl Judged {
    /// The required programs no `PATH` entry holds.
    pub fn missing(&self) -> Vec<&Program> {
        self.required
            .iter()
            .filter(|program| program.found.is_none())
            .collect()
    }

    /// The mechanisms this machine cannot carry: the sandbox, when the
    /// platform cannot run it or a required program is missing.
    pub fn unsupported(&self) -> Vec<Mechanism> {
        if self.kind == Kind::Other || !self.missing().is_empty() {
            vec![Mechanism::Sandbox]
        } else {
            Vec::new()
        }
    }
}

/// Judges what was gathered. A program counts at the first entry that is an
/// absolute folder holding a regular file with any execute bit; a relative
/// entry is skipped because what it names depends on the working directory.
pub fn judge(observed: &Observed) -> Judged {
    let (kind, required): (Kind, &[Required]) = match observed.os.as_str() {
        "linux" => (Kind::Linux, &LINUX),
        "macos" => (Kind::MacOs, &[]),
        _ => (Kind::Other, &[]),
    };
    let required = required
        .iter()
        .map(|required| Program {
            name: required.name,
            package: required.package,
            found: observed
                .searches
                .iter()
                .find(|search| search.program == required.name)
                .and_then(|search| first_usable(&search.candidates))
                .map(|folder| folder.join(required.name)),
        })
        .collect();
    Judged {
        os: observed.os.clone(),
        kind,
        required,
    }
}

/// The folder of the first candidate that is usable.
fn first_usable(candidates: &[Candidate]) -> Option<&Path> {
    candidates
        .iter()
        .find(|candidate| {
            candidate.folder.is_absolute() && candidate.regular_file && candidate.mode & 0o111 != 0
        })
        .map(|candidate| candidate.folder.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(folder: &str, regular_file: bool, mode: u32) -> Candidate {
        Candidate {
            folder: folder.into(),
            regular_file,
            mode,
        }
    }

    fn observed(os: &str, bwrap: Vec<Candidate>, socat: Vec<Candidate>) -> Observed {
        Observed {
            os: os.into(),
            searches: vec![
                Search {
                    program: "bwrap".into(),
                    candidates: bwrap,
                },
                Search {
                    program: "socat".into(),
                    candidates: socat,
                },
            ],
        }
    }

    #[test]
    fn bwrap_or_socat_required_on_macos_is_caught() {
        for observed in [
            observed("macos", vec![], vec![]),
            observed("macos", vec![candidate("/usr/bin", true, 0o755)], vec![]),
        ] {
            let judged = judge(&observed);
            assert!(judged.missing().is_empty(), "{judged:?}");
            assert!(judged.unsupported().is_empty(), "{judged:?}");
        }
    }

    #[test]
    fn an_unsupported_platform_reported_as_sandbox_ready_is_caught() {
        let both = vec![candidate("/usr/bin", true, 0o755)];
        for observed in [
            observed("freebsd", vec![], vec![]),
            observed("windows", both.clone(), both),
        ] {
            let judged = judge(&observed);
            assert_eq!(
                judged.unsupported(),
                [Mechanism::Sandbox],
                "{}",
                observed.os
            );
            assert!(judged.required.is_empty(), "{}", observed.os);
        }
    }

    #[test]
    fn a_directory_or_non_executable_file_on_path_counted_as_the_program_is_caught() {
        let relative = candidate("bin", true, 0o755);
        let directory = candidate("/a", false, 0o755);
        let not_executable = candidate("/b", true, 0o644);
        let usable = candidate("/c", true, 0o755);
        let found = |bwrap: Vec<Candidate>| {
            let judged = judge(&observed("linux", bwrap, vec![]));
            judged
                .required
                .into_iter()
                .find(|program| program.name == "bwrap")
                .unwrap()
                .found
        };
        let before = [relative, directory, not_executable];
        assert_eq!(
            found([&before[..], &[usable]].concat()),
            Some(PathBuf::from("/c/bwrap"))
        );
        assert_eq!(found(before.to_vec()), None);
        assert_eq!(
            found(vec![candidate("/e", true, 0o610)]),
            Some(PathBuf::from("/e/bwrap"))
        );
        assert_eq!(
            found(vec![candidate("/f", true, 0o601)]),
            Some(PathBuf::from("/f/bwrap"))
        );
    }
}
