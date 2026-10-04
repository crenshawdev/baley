//! Prepares a session's project read or write from the session's own
//! `CLAUDE_PROJECT_DIR`, in the order the command line already uses.
//!
//! The plan is pure: from what has been observed so far it gives the next
//! step to perform, the `failed` answer, or the prepared result. The entry
//! performs each step and asks again, so a refusal always comes before the
//! step after it. Every refusal is a `failed` answer with `recorded: false`
//! and the preparation itself creates nothing: no project, no
//! `project.initialized` event, no catalog seed, no detection run. The owner
//! sets those up with `baley init` and `baley config`.

use std::path::{Path, PathBuf};

use baley_core::policy::{CONFIG_UNAVAILABLE, SettingsFile, Unavailable};
use baley_store::{Admin, ProjectId, ServerCaller, StoreError};
use serde_json::Value;

use crate::discovery::{self, Ancestor, Discovery, PROJECT_FILE};
use crate::envelope::{Envelope, LEDGER_BUSY};
use crate::{init, settings};

/// The code for a project directory that cannot be walked, such as one
/// removed after the server started.
pub const PROJECT_CONTEXT_INVALID: &str = "project-context-invalid";

/// The code for a project directory that no repository with a `baley.toml`
/// holds.
pub const NOT_A_PROJECT: &str = "not-a-project";

/// The code for a project whose id the ledger does not list.
pub const PROJECT_NOT_IN_LEDGER: &str = "project-not-in-ledger";

/// The code for a ledger that cannot take the call and is not just busy.
pub const LEDGER_UNAVAILABLE: &str = "ledger-unavailable";

const PLACE_PROJECT_DIR: &str = "CLAUDE_PROJECT_DIR";
const PLACE_SETTINGS: &str = "settings";
const PLACE_PROJECT: &str = "project";
const PLACE_LEDGER: &str = "ledger";

/// A project read or write ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The project the working tree's `baley.toml` names, known to the ledger.
    pub project: ProjectId,
    /// The canonical repository root discovery found. The caller keeps the
    /// project directory text exactly as it was given.
    pub root: PathBuf,
}

/// Why a store step could not answer.
#[derive(Debug)]
enum LedgerFault {
    /// The ledger was not opened when the server started.
    NoStore,
    /// The store returned this error.
    Store(StoreError),
}

/// What preparation has observed so far.
#[derive(Debug, Default)]
struct Seen {
    /// The project directory's ancestors, or the walk's fault text.
    ancestors: Option<Result<Vec<Ancestor>, String>>,
    /// The working-tree `baley.toml` in the discovered folder.
    working: Option<Result<Option<SettingsFile>, Unavailable>>,
    /// The ledger's project ids.
    projects: Option<Result<Vec<ProjectId>, LedgerFault>>,
}

/// One step the entry performs for the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Walk the ancestors of this directory.
    Discover(String),
    /// Read the working-tree project file at this path.
    ReadProjectFile(PathBuf),
    /// List the ledger's projects.
    ListProjects,
}

/// What the plan wants next.
#[derive(Debug)]
enum Next {
    Do(Step),
    Failed(Envelope<Value>),
    Ready(Prepared),
}

fn failed(code: &str, reason: impl Into<String>, place: &str) -> Next {
    Next::Failed(Envelope::failed(code, reason.into(), place))
}

/// The answer for a store step that could not answer. Only a busy ledger is
/// retryable.
fn ledger_failed(fault: &LedgerFault) -> Next {
    match fault {
        LedgerFault::NoStore => failed(
            LEDGER_UNAVAILABLE,
            "the ledger could not be opened when this server started, so no project call can be taken",
            PLACE_LEDGER,
        ),
        LedgerFault::Store(StoreError::Busy) => failed(
            LEDGER_BUSY,
            "the ledger is busy, so the call was not taken. Try it again",
            PLACE_LEDGER,
        ),
        LedgerFault::Store(error) => failed(LEDGER_UNAVAILABLE, error.to_string(), PLACE_LEDGER),
    }
}

/// The next step for a read, or its answer. Discovery starts from the
/// project directory the caller holds and from nothing else.
fn next(caller: &ServerCaller, seen: &Seen) -> Next {
    let Some(ancestors) = &seen.ancestors else {
        return Next::Do(Step::Discover(caller.project_directory().into()));
    };
    let ancestors = match ancestors {
        Ok(ancestors) => ancestors,
        Err(fault) => {
            return failed(
                PROJECT_CONTEXT_INVALID,
                format!(
                    "{PLACE_PROJECT_DIR} {} cannot be read as the project: {fault}",
                    caller.project_directory()
                ),
                PLACE_PROJECT_DIR,
            );
        }
    };
    let (folder, root) = match discovery::discover(ancestors) {
        Discovery::Managed { folder, root } => (folder, root),
        Discovery::Unmanaged { .. } | Discovery::Outside => {
            return failed(
                NOT_A_PROJECT,
                format!(
                    "{PLACE_PROJECT_DIR} {} is not in a project: no {PROJECT_FILE} was found in its repository",
                    caller.project_directory()
                ),
                PLACE_PROJECT_DIR,
            );
        }
    };
    let file = folder.join(PROJECT_FILE);
    let Some(working) = &seen.working else {
        return Next::Do(Step::ReadProjectFile(file));
    };
    let project = match init::observe_file(working.clone()) {
        Ok(Some(identity)) => ProjectId(identity.id),
        Ok(None) => return unavailable_file(&file),
        Err(fault) => return failed(CONFIG_UNAVAILABLE, fault.to_string(), PLACE_SETTINGS),
    };
    let Some(projects) = &seen.projects else {
        return Next::Do(Step::ListProjects);
    };
    match projects {
        Err(fault) => return ledger_failed(fault),
        Ok(known) if !known.contains(&project) => {
            return failed(
                PROJECT_NOT_IN_LEDGER,
                format!(
                    "project {} is not in this machine's ledger. The owner ties this checkout to the ledger by running `baley init` in it",
                    project.0
                ),
                PLACE_PROJECT,
            );
        }
        Ok(_) => {}
    }
    Next::Ready(Prepared { project, root })
}

/// The answer for a project file that was there at the walk and is gone.
fn unavailable_file(file: &Path) -> Next {
    failed(
        CONFIG_UNAVAILABLE,
        format!("{CONFIG_UNAVAILABLE}: {} was not found", file.display()),
        PLACE_SETTINGS,
    )
}

/// Prepares a project read for the session whose caller is given.
///
/// It finds the project fresh on every call from the caller's project
/// directory, reads the working-tree project file for its id and checks that
/// the ledger lists it. It validates no settings, gathers no checkout facts
/// and appends nothing. `store` is `None` when the ledger was not opened at
/// startup. The refusal is the `failed` answer, boxed to keep the result small.
pub fn prepare<S: Admin>(
    store: Option<&S>,
    caller: &ServerCaller,
) -> Result<Prepared, Box<Envelope<Value>>> {
    let mut seen = Seen::default();
    loop {
        match next(caller, &seen) {
            Next::Ready(prepared) => return Ok(prepared),
            Next::Failed(answer) => return Err(Box::new(answer)),
            Next::Do(step) => perform(step, store, &mut seen),
        }
    }
}

/// Performs one step and records what it observed. It owns no policy.
fn perform<S: Admin>(step: Step, store: Option<&S>, seen: &mut Seen) {
    match step {
        Step::Discover(directory) => {
            seen.ancestors =
                Some(discovery::ancestors(Path::new(&directory)).map_err(|e| e.to_string()));
        }
        Step::ReadProjectFile(path) => seen.working = Some(settings::read(&path)),
        Step::ListProjects => {
            seen.projects = Some(match store {
                None => Err(LedgerFault::NoStore),
                Some(store) => store
                    .projects()
                    .map(|listed| listed.into_iter().map(|(id, _)| id).collect())
                    .map_err(LedgerFault::Store),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";
    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";

    fn caller(project: &str, working: &str) -> ServerCaller {
        ServerCaller::new(project, working, "claude-code", SESSION, &json!(7)).unwrap()
    }

    fn at(path: &str, has_project_file: bool, has_git: bool) -> Ancestor {
        Ancestor {
            path: path.into(),
            has_project_file,
            has_git,
        }
    }

    /// A project at `/real/r`, reached through the nested folder `/real/r/p`.
    fn managed() -> Vec<Ancestor> {
        vec![
            at("/real/r/p", false, false),
            at("/real/r", true, true),
            at("/", false, false),
        ]
    }

    fn project_file(id: &str) -> Result<Option<SettingsFile>, Unavailable> {
        let text = format!("[project]\nid = \"{id}\"\nname = \"r\"\n");
        Ok(Some(settings::file(
            Path::new("/real/r/baley.toml"),
            text.into_bytes(),
        )))
    }

    fn walked() -> Seen {
        Seen {
            ancestors: Some(Ok(managed())),
            ..Seen::default()
        }
    }

    fn with_file(mut seen: Seen) -> Seen {
        seen.working = Some(project_file(ID));
        seen
    }

    fn known(mut seen: Seen, ids: &[&str]) -> Seen {
        seen.projects = Some(Ok(ids.iter().map(|id| ProjectId((*id).into())).collect()));
        seen
    }

    fn answer(next: Next) -> Value {
        match next {
            Next::Failed(answer) => serde_json::to_value(answer).unwrap(),
            other => panic!("expected a failed answer, got {other:?}"),
        }
    }

    fn assert_failed(next: Next, code: &str, place: &str, retryable: bool) -> Value {
        let value = answer(next);
        assert_eq!(value["status"], "failed", "{value}");
        assert_eq!(value["code"], code, "{value}");
        assert_eq!(value["place"], place, "{value}");
        assert_eq!(value["recorded"], false, "{value}");
        assert_eq!(value["retryable"], retryable, "{value}");
        value
    }

    #[test]
    fn discovery_that_starts_from_the_working_directory_instead_of_the_project_directory() {
        let next = next(&caller("/p", "/w"), &Seen::default());
        assert!(
            matches!(next, Next::Do(Step::Discover(ref d)) if d == "/p"),
            "{next:?}"
        );
    }

    #[test]
    fn a_read_asks_for_the_file_then_the_ledger_and_nothing_else() {
        let caller = caller("/real/r/p", "/w");
        let next_step = |seen: &Seen| match next(&caller, seen) {
            Next::Do(step) => step,
            other => panic!("expected a step, got {other:?}"),
        };
        assert_eq!(
            next_step(&walked()),
            Step::ReadProjectFile("/real/r/baley.toml".into())
        );
        assert_eq!(next_step(&with_file(walked())), Step::ListProjects);
        assert!(matches!(
            next(&caller, &known(with_file(walked()), &[ID])),
            Next::Ready(_)
        ));
    }

    #[test]
    fn a_prepared_read_whose_root_is_the_callers_text_instead_of_the_canonical_root() {
        let caller = caller("/link/to/r/p", "/w");
        let next = next(&caller, &known(with_file(walked()), &[ID]));
        match next {
            Next::Ready(prepared) => {
                assert_eq!(prepared.root, Path::new("/real/r"));
                assert_eq!(prepared.project, ProjectId(ID.into()));
            }
            other => panic!("expected a prepared read, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_project_that_is_prepared_or_answered_as_anything_but_not_in_ledger() {
        let seen = known(
            with_file(walked()),
            &["00000000-0000-4000-8000-000000000000"],
        );
        let value = assert_failed(
            next(&caller("/real/r/p", "/w"), &seen),
            "project-not-in-ledger",
            "project",
            false,
        );
        let reason = value["reason"].as_str().unwrap();
        assert!(reason.contains("baley init"), "{reason}");
        assert!(reason.contains(ID), "{reason}");
    }

    #[test]
    fn a_walk_fault_that_is_not_project_context_invalid_at_the_project_directory() {
        let seen = Seen {
            ancestors: Some(Err("No such file or directory (os error 2)".into())),
            ..Seen::default()
        };
        assert_failed(
            next(&caller("/gone", "/w"), &seen),
            "project-context-invalid",
            "CLAUDE_PROJECT_DIR",
            false,
        );
    }

    #[test]
    fn an_unmanaged_or_outside_directory_that_is_not_answered_as_not_a_project() {
        for ancestors in [
            vec![at("/r/p", false, false), at("/r", false, true)],
            vec![at("/r/p", false, false), at("/", false, false)],
        ] {
            let seen = Seen {
                ancestors: Some(Ok(ancestors)),
                ..Seen::default()
            };
            assert_failed(
                next(&caller("/r/p", "/w"), &seen),
                "not-a-project",
                "CLAUDE_PROJECT_DIR",
                false,
            );
        }
    }

    #[test]
    fn a_project_file_fault_that_is_not_config_unavailable_at_settings() {
        let unreadable = Err(Unavailable {
            path: "/real/r/baley.toml".into(),
            fault: baley_core::policy::Fault::Unreadable {
                cause: "Permission denied (os error 13)".into(),
            },
        });
        let no_id = Ok(Some(settings::file(
            Path::new("/real/r/baley.toml"),
            b"[project]\nname = \"r\"\n".to_vec(),
        )));
        for read in [unreadable, no_id, Ok(None)] {
            let mut seen = walked();
            seen.working = Some(read);
            let value = assert_failed(
                next(&caller("/real/r/p", "/w"), &seen),
                "config-unavailable",
                "settings",
                false,
            );
            assert!(
                value["reason"]
                    .as_str()
                    .unwrap()
                    .contains("/real/r/baley.toml"),
                "{value}"
            );
        }
    }

    #[test]
    fn a_busy_store_that_is_not_retryable_or_another_fault_that_is() {
        let ask = |fault: LedgerFault| {
            let mut seen = with_file(walked());
            seen.projects = Some(Err(fault));
            next(&caller("/real/r/p", "/w"), &seen)
        };
        assert_failed(
            ask(LedgerFault::Store(StoreError::Busy)),
            "ledger-busy",
            "ledger",
            true,
        );
        let unavailable = StoreError::Unavailable("the ledger is fenced".into());
        let value = assert_failed(
            ask(LedgerFault::Store(unavailable)),
            "ledger-unavailable",
            "ledger",
            false,
        );
        assert!(
            value["reason"].as_str().unwrap().contains("fenced"),
            "{value}"
        );
        assert_failed(
            ask(LedgerFault::NoStore),
            "ledger-unavailable",
            "ledger",
            false,
        );
    }
}
