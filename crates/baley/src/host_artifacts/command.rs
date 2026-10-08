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

fn escape_controls(text: &str) -> String {
    // Control characters must not add lines or drive the terminal.
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

/// The bytes for standard output: what was selected, even when a path was
/// left out, since partial settings are printed beside their report.
/// Nothing on a refusal.
fn stdout_bytes(outcome: &Result<Selection, Refusal>) -> &[u8] {
    match outcome {
        Ok(selection) => &selection.bytes,
        Err(_) => &[],
    }
}

/// The text for standard error: one `baley: ` line per refusal or
/// unrendered path, its control characters escaped.
fn stderr_text(outcome: &Result<Selection, Refusal>) -> String {
    let messages: Vec<String> = match outcome {
        Ok(selection) => selection
            .unrendered
            .iter()
            .map(ToString::to_string)
            .collect(),
        Err(refusal) => vec![refusal.to_string()],
    };
    messages
        .iter()
        .map(|message| format!("baley: {}\n", escape_controls(message)))
        .collect()
}

/// Failure when the request was refused, a path was left out or standard
/// output did not take the bytes, so partial settings are never taken as
/// complete.
fn exit_code(refused: bool, unrendered: bool, written: bool) -> ExitCode {
    if refused || unrendered || !written {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Writes what a request selects to `out` and its report to `err`. The
/// bytes, the report and the exit code are decided by the functions above.
pub fn print(request: &Request, out: &mut dyn Write, err: &mut dyn Write) -> ExitCode {
    let outcome = select(request);
    let written = out
        .write_all(stdout_bytes(&outcome))
        .and_then(|()| out.flush())
        .is_ok();
    let _ = err.write_all(stderr_text(&outcome).as_bytes());
    let unrendered = outcome
        .as_ref()
        .is_ok_and(|selection| !selection.unrendered.is_empty());
    exit_code(outcome.is_err(), unrendered, written)
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

    const SETTINGS_PATH: &str = "/srv/[x]/baley.toml";

    #[test]
    fn stdout_bytes_other_than_the_selected_content_is_caught() {
        let bytes = b"{\n  \"sandbox\": {}\n}\n".to_vec();
        let clean: Result<Selection, Refusal> = Ok(Selection {
            bytes: bytes.clone(),
            unrendered: Vec::new(),
        });
        assert_eq!(stdout_bytes(&clean), bytes.as_slice());
        let partial: Result<Selection, Refusal> = Ok(Selection {
            bytes: bytes.clone(),
            unrendered: vec![Unrendered::UnsupportedCharacter {
                path: SETTINGS_PATH.to_owned(),
                character: '[',
            }],
        });
        assert_eq!(stdout_bytes(&partial), bytes.as_slice());
        let refused: Result<Selection, Refusal> = Err(Refusal::UnknownStub("bal-plan".to_owned()));
        assert!(stdout_bytes(&refused).is_empty());
    }

    #[test]
    fn a_stub_selected_from_other_than_its_manifest_bytes_is_caught() {
        let mut judged = Vec::new();
        for entry in manifest(&front_doors()).unwrap() {
            let selected = select(&Request::Stub {
                identity: entry.identity.clone(),
            })
            .unwrap();
            let digest: String = Sha256::digest(&selected.bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(selected.bytes, entry.bytes, "{}", entry.identity);
            assert_eq!(digest, entry.digest, "{}", entry.identity);
            assert!(selected.unrendered.is_empty(), "{}", entry.identity);
            judged.push(entry.identity);
        }
        judged.sort();
        assert_eq!(judged, ["bal-capture", "bal-help"]);
    }

    #[test]
    fn partial_settings_selected_without_their_report_is_caught() {
        // A `[` in a rule path is pattern syntax with no escape, so the
        // proposal leaves the file out and says so.
        let home = "/home/o/.local/share/crenshawdev/baley";
        let config = "/home/o/.config/crenshawdev/baley";
        let selected = select(&Request::Settings {
            executable: "/home/o/.local/bin/baley".into(),
            home: home.into(),
            config: config.into(),
            protect: vec![SETTINGS_PATH.into()],
        })
        .unwrap();
        let settings: Value = serde_json::from_slice(&selected.bytes).unwrap();
        for list in ["denyRead", "denyWrite"] {
            let entries = settings["sandbox"]["filesystem"][list].as_array().unwrap();
            assert!(entries.contains(&json!(home)), "{list}");
            assert!(entries.contains(&json!(config)), "{list}");
        }
        let text = String::from_utf8(selected.bytes).unwrap();
        assert!(!text.contains(SETTINGS_PATH), "{text}");
        assert_eq!(
            selected.unrendered,
            [Unrendered::UnsupportedCharacter {
                path: SETTINGS_PATH.to_owned(),
                character: '[',
            }]
        );
    }

    #[test]
    fn a_refusal_an_unrendered_path_or_a_failed_write_exiting_as_success_is_caught() {
        assert_eq!(exit_code(true, false, true), ExitCode::FAILURE);
        assert_eq!(exit_code(false, true, true), ExitCode::FAILURE);
        assert_eq!(exit_code(false, false, true), ExitCode::SUCCESS);
        assert_eq!(exit_code(false, false, false), ExitCode::FAILURE);
    }

    #[test]
    fn a_refusal_holding_a_newline_written_across_two_stderr_lines_is_caught() {
        let refused: Result<Selection, Refusal> = Err(Refusal::UnknownStub(
            "missing\n\u{1b}[2Jsecond line".to_owned(),
        ));
        assert_eq!(
            stderr_text(&refused),
            "baley: `missing\\n\\u{1b}[2Jsecond line` is not a served front door\n"
        );
    }

    #[test]
    fn an_unrendered_path_holding_a_newline_or_its_report_dropped_from_stderr_is_caught() {
        let selected: Result<Selection, Refusal> = Ok(Selection {
            bytes: b"{}\n".to_vec(),
            unrendered: vec![
                Unrendered::MissingPrerequisite {
                    path: "srv\n\u{1b}[2J/baley.toml".into(),
                    fault: PathFault::Relative,
                },
                Unrendered::UnsupportedCharacter {
                    path: SETTINGS_PATH.to_owned(),
                    character: '[',
                },
            ],
        });
        assert_eq!(
            stderr_text(&selected),
            "baley: the path srv\\n\\u{1b}[2J/baley.toml is not absolute\n\
             baley: the path /srv/[x]/baley.toml holds `[`, which a deny rule would read as a pattern\n"
        );
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
}
