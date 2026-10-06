//! `baley artifact`: prints the content this module renders, to standard
//! output only (D-21), so a disposable Claude Code run (Build 3 T12) loads
//! the binary's own bytes rather than a hand-written copy.
//!
//! Nothing here writes a file, creates a folder or reads the environment.
//! The home and config folders are arguments, not resolved, so a run can
//! name stand-ins. Writing the artifacts into place is `baley install`'s
//! (T15).

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use super::executable::{Executable, MissingPrerequisite};
use super::security::{Unrendered, propose};
use super::stubs::{DuplicateIdentity, front_doors, manifest};
use super::{hook, registration};
use crate::folders::Folders;

/// Arguments for printing one artifact.
#[derive(Args, Debug, Clone)]
pub struct ArtifactArgs {
    /// What to print.
    #[command(subcommand)]
    pub request: Request,
}

/// The artifact a run prints.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Print the stub manifest: each stub's host, identity and SHA-256.
    Manifest,
    /// Print one stub's bytes exactly as the manifest holds them.
    Stub {
        /// The front door's identity, such as bal-help.
        identity: String,
    },
    /// Print the MCP registration for `baley serve`.
    Registration {
        /// The absolute path of the baley executable the host runs.
        #[arg(long, value_name = "PATH")]
        executable: PathBuf,
        /// Keep every Baley tool loaded instead of behind tool search.
        #[arg(long)]
        always_load: bool,
    },
    /// Print the pre-tool hook for `baley guard`.
    Hook {
        /// The absolute path of the baley executable the hook runs.
        #[arg(long, value_name = "PATH")]
        executable: PathBuf,
    },
    /// Print the sandbox and deny-rule settings.
    Settings {
        /// The absolute path of the baley executable, kept from writes.
        #[arg(long, value_name = "PATH")]
        executable: PathBuf,
        /// Baley's home folder, kept from reads and writes.
        #[arg(long, value_name = "DIR")]
        home: PathBuf,
        /// Baley's config folder, kept from reads and writes.
        #[arg(long, value_name = "DIR")]
        config: PathBuf,
        /// A file kept from writes only, such as a baley.toml; repeatable.
        #[arg(long, value_name = "FILE")]
        protect: Vec<PathBuf>,
    },
}

/// Why a request prints nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The compiled front-door list names one identity twice.
    Manifest(DuplicateIdentity),
    /// No served front door has this identity.
    UnknownStub(String),
    /// The supplied executable cannot be rendered.
    Executable(MissingPrerequisite),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Manifest(duplicate) => duplicate.fmt(f),
            Refusal::UnknownStub(identity) => {
                write!(f, "`{identity}` is not a served front door")
            }
            Refusal::Executable(missing) => missing.fmt(f),
        }
    }
}

/// What a request selects: the bytes for standard output, and every supplied
/// path the content could not hold. Only `settings` reports paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The bytes to print.
    pub bytes: Vec<u8>,
    /// The paths left out of the content, each with why.
    pub unrendered: Vec<Unrendered>,
}

/// Selects the bytes for a request from the library's renderers. A stub is
/// its manifest bytes as they are; every other artifact is its JSON value,
/// pretty-printed once with a trailing newline.
pub fn select(request: &Request) -> Result<Selection, Refusal> {
    let (value, unrendered) = match request {
        Request::Manifest => {
            let entries = manifest(&front_doors()).map_err(Refusal::Manifest)?;
            let listed = entries
                .iter()
                .map(|entry| {
                    json!({
                        "host": entry.host.name(),
                        "identity": entry.identity,
                        "digest": entry.digest,
                    })
                })
                .collect();
            (Value::Array(listed), Vec::new())
        }
        Request::Stub { identity } => {
            let entries = manifest(&front_doors()).map_err(Refusal::Manifest)?;
            let entry = entries
                .into_iter()
                .find(|entry| entry.identity == *identity)
                .ok_or_else(|| Refusal::UnknownStub(identity.clone()))?;
            return Ok(Selection {
                bytes: entry.bytes,
                unrendered: Vec::new(),
            });
        }
        Request::Registration {
            executable,
            always_load,
        } => (
            registration::render(&judged(executable)?, *always_load),
            Vec::new(),
        ),
        Request::Hook { executable } => (hook::render(&judged(executable)?), Vec::new()),
        Request::Settings {
            executable,
            home,
            config,
            protect,
        } => {
            let folders = Folders {
                home: home.clone(),
                config: config.clone(),
            };
            // The proposal adds the executable to the write-only files itself
            // (D-25), as the placement projection does.
            let proposal = propose(&folders, &judged(executable)?, protect);
            (proposal.settings, proposal.unrendered)
        }
    };
    let mut bytes = serde_json::to_vec_pretty(&value).expect("a JSON value serializes");
    bytes.push(b'\n');
    Ok(Selection { bytes, unrendered })
}

fn judged(executable: &Path) -> Result<Executable, Refusal> {
    Executable::new(executable).map_err(Refusal::Executable)
}

/// Writes what a request selects to `out` and each refusal or unrendered
/// path to `err`, one line each. It fails when anything was refused or left
/// out, so partial settings are never taken as complete.
pub fn print(request: &Request, out: &mut dyn Write, err: &mut dyn Write) -> ExitCode {
    let selection = match select(request) {
        Ok(selection) => selection,
        Err(refusal) => {
            let _ = writeln!(err, "baley: {refusal}");
            return ExitCode::FAILURE;
        }
    };
    if out
        .write_all(&selection.bytes)
        .and_then(|()| out.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    for unrendered in &selection.unrendered {
        let _ = writeln!(err, "baley: {unrendered}");
    }
    if selection.unrendered.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Runs `baley artifact`, printing to standard output and standard error.
pub fn run(args: ArtifactArgs) -> ExitCode {
    print(
        &args.request,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    )
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::host_artifacts::executable::PathFault;

    #[test]
    fn the_print_only_command_printing_bytes_other_than_the_selected_content_is_caught() {
        let request = Request::Stub {
            identity: "bal-help".to_owned(),
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = print(&request, &mut out, &mut err);
        let entries = manifest(&front_doors()).unwrap();
        let help = entries
            .iter()
            .find(|entry| entry.identity == "bal-help")
            .unwrap();
        let digest: String = Sha256::digest(&out)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(digest, help.digest);
        assert_eq!(out, help.bytes);
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn an_unknown_stub_printed_as_another_or_refused_without_its_name_is_caught() {
        let request = Request::Stub {
            identity: "bal-plan".to_owned(),
        };
        let refused = select(&request).unwrap_err();
        assert_eq!(refused, Refusal::UnknownStub("bal-plan".to_owned()));
        assert!(refused.to_string().contains("bal-plan"));
    }

    #[test]
    fn a_hook_rendered_for_a_relative_executable_is_caught() {
        let request = Request::Hook {
            executable: "target/debug/baley".into(),
        };
        assert_eq!(
            select(&request),
            Err(Refusal::Executable(MissingPrerequisite {
                fault: PathFault::Relative,
            }))
        );
    }

    #[test]
    fn a_manifest_printed_with_the_stub_bytes_or_without_host_identity_and_digest_is_caught() {
        let printed = select(&Request::Manifest).unwrap();
        let listed: Value = serde_json::from_slice(&printed.bytes).unwrap();
        let entries = manifest(&front_doors()).unwrap();
        let expected: Vec<Value> = entries
            .iter()
            .map(|entry| {
                json!({
                    "host": "claude-code",
                    "identity": entry.identity,
                    "digest": entry.digest,
                })
            })
            .collect();
        assert_eq!(listed, Value::Array(expected));
        assert!(printed.bytes.ends_with(b"]\n"));
    }

    #[test]
    fn partial_settings_exiting_as_success_or_dropping_the_report_is_caught() {
        // A `[` in a rule path is pattern syntax with no escape, so the
        // proposal leaves the file out and says so.
        let protect = PathBuf::from("/srv/[x]/baley.toml");
        let request = Request::Settings {
            executable: "/home/o/.local/bin/baley".into(),
            home: "/home/o/.local/share/crenshawdev/baley".into(),
            config: "/home/o/.config/crenshawdev/baley".into(),
            protect: vec![protect],
        };
        let selected = select(&request).unwrap();
        assert_eq!(
            selected.unrendered,
            [Unrendered::UnsupportedCharacter {
                path: "/srv/[x]/baley.toml".to_owned(),
                character: '[',
            }]
        );
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = print(&request, &mut out, &mut err);
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(out, selected.bytes);
        let err = String::from_utf8(err).unwrap();
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(err.contains("/srv/[x]/baley.toml"), "{err}");
    }
}
