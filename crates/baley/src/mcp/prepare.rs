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

use baley_core::policy::recorded::{RecordedPolicy, recorded_policy};
use baley_core::policy::{CONFIG_UNAVAILABLE, EffectivePolicy, Host, SettingsFile, Unavailable};
use baley_store::{
    Actor, Admin, Caller, Command, CommandKind, Hash, ProjectId, RequestId, ServerCaller,
    StoreError,
};
use serde_json::Value;

use crate::discovery::{self, Ancestor, Discovery, PROJECT_FILE};
use crate::envelope::{Envelope, LEDGER_BUSY};
use crate::policy_step::{self, Reads};
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

/// The two policies a write is judged under.
#[derive(Debug)]
pub struct WritePolicies {
    /// The policy built with no host. Its `git.remote` names the remote the
    /// checkout's facts come from, the one the command line uses, so the
    /// server and the command line record one remote for one checkout.
    pub facts: EffectivePolicy,
    /// The policy built for the call's host, recorded at the root. The policy
    /// step records it as `policy.effective` under (checkout, host), so the
    /// host's section values apply.
    pub recorded: RecordedPolicy,
}

/// Judges the gathered settings of a write for the call's `host` and the
/// discovered `root`. The recorded checkout is the root's text, the path
/// checkout admission keys its hostless version by. A fault, which includes a
/// root that is not UTF-8, is the `config-unavailable` text, in `build`'s
/// order.
pub fn judge_settings(reads: &Reads, host: Host, root: &Path) -> Result<WritePolicies, String> {
    let facts = policy_step::build(reads, None).map_err(|e| e.to_string())?;
    let with_host = policy_step::build(reads, Some(host)).map_err(|e| e.to_string())?;
    let recorded = recorded_policy(root, &with_host).map_err(|e| e.to_string())?;
    Ok(WritePolicies { facts, recorded })
}

/// The identity a served write supplies, which preparation carries into the
/// domain command unchanged.
///
/// The digest covers the operation and its meaningful input and leaves out
/// the policy version, which travels only in the command's `policy_version`.
/// So a retry after a policy edit is still a replay of the same request. The
/// command line's digests keep the version, which is safe only because its
/// request ids are always fresh. The request id is an operation argument,
/// never derived from the JSON-RPC id, which repeats after a restart and
/// across subagents sharing one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    /// The command kind, such as `capture.record`.
    pub kind: CommandKind,
    /// The request id the operation was given.
    pub request_id: RequestId,
    /// The operation's digest, with no policy version in it.
    pub digest: Hash,
}

/// The domain command a prepared write runs: the supplied kind, request id
/// and digest, an empty scope, the policy version the step returned, the
/// server time, Baley as the actor and the server caller. It never computes
/// or extends the digest.
pub fn prepared_command(
    project: &ProjectId,
    request: &WriteRequest,
    policy_version: u64,
    at: &str,
    caller: &ServerCaller,
) -> Command {
    Command {
        project: project.clone(),
        kind: request.kind.clone(),
        request_id: request.request_id.clone(),
        digest: request.digest,
        scope: Vec::new(),
        policy_version,
        recorded_at: at.into(),
        actor: Actor::Baley,
        caller: Some(Caller::Server(caller.clone())),
    }
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

    const HEAD_PATH: &str = "/real/r/baley.toml";

    fn reads(head_text: &str) -> Reads {
        Reads {
            global: Ok(None),
            head: Some(Ok(crate::committed::Committed {
                layer: Some(settings::file(
                    Path::new(HEAD_PATH),
                    head_text.as_bytes().to_vec(),
                )),
                pending: None,
            })),
        }
    }

    /// The file sets a remote and a value at the top level, and the host
    /// section sets another of each.
    const SECTIONED: &str = "escalate_on_failure = false\n[git]\nremote = \"upstream\"\n[host.claude-code]\nescalate_on_failure = true\ngit.remote = \"other\"\n";

    fn judged() -> WritePolicies {
        judge_settings(&reads(SECTIONED), Host::ClaudeCode, Path::new("/real/r")).unwrap()
    }

    fn remote(policy: &EffectivePolicy) -> Option<&baley_core::policy::Value> {
        policy.settings["git.remote"].value.as_ref()
    }

    #[test]
    fn a_facts_policy_built_with_the_host_names_the_hosts_remote_instead_of_the_files() {
        let judged = judged();
        assert_eq!(
            remote(&judged.facts),
            Some(&baley_core::policy::Value::RemoteName("upstream".into()))
        );
        assert_eq!(judged.facts.host, None);
    }

    #[test]
    fn a_recorded_policy_that_is_not_keyed_to_the_host_or_lacks_its_section_values() {
        use baley_core::policy::recorded::effective_payload;
        let payload = effective_payload(&judged().recorded, "p", 0);
        assert_eq!(payload["host"], "claude-code");
        assert_eq!(payload["values"]["escalate_on_failure"], true);
        assert_eq!(payload["values"]["git.remote"], "other");
    }

    #[test]
    fn a_recorded_checkout_that_differs_from_the_discovered_root() {
        assert_eq!(judged().recorded.checkout, "/real/r");
    }

    #[test]
    fn an_invalid_heads_copy_that_is_not_config_unavailable() {
        let refusal = judge_settings(
            &reads("escalate_on_failure = \"yes\"\n"),
            Host::ClaudeCode,
            Path::new("/real/r"),
        )
        .unwrap_err();
        assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
        assert!(
            refusal.contains("HEAD's copy of /real/r/baley.toml"),
            "{refusal}"
        );
    }

    #[test]
    fn a_root_that_is_not_utf8_that_is_not_config_unavailable() {
        use std::os::unix::ffi::OsStrExt;
        let root = Path::new(std::ffi::OsStr::from_bytes(b"/real/\xff"));
        let refusal = judge_settings(&reads(SECTIONED), Host::ClaudeCode, root).unwrap_err();
        assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
        assert!(refusal.contains("is not UTF-8"), "{refusal}");
    }

    const AT: &str = "2026-10-04T09:30:00Z";
    const REQUEST: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";

    fn write_request(digest: u8) -> WriteRequest {
        WriteRequest {
            kind: CommandKind("capture.record".into()),
            request_id: RequestId(REQUEST.into()),
            digest: Hash::from_hex(&format!("{digest:02x}").repeat(32)).unwrap(),
        }
    }

    fn command(request: &WriteRequest, version: u64) -> Command {
        prepared_command(
            &ProjectId(ID.into()),
            request,
            version,
            AT,
            &caller("/real/r/p", "/w"),
        )
    }

    #[test]
    fn a_prepared_command_attributed_to_the_owner_instead_of_baley() {
        assert_eq!(command(&write_request(1), 3).actor, Actor::Baley);
    }

    #[test]
    fn a_prepared_command_that_carries_another_version_or_digest_than_supplied() {
        let request = write_request(1);
        let first = command(&request, 3);
        let second = command(&request, 9);
        assert_eq!(first.policy_version, 3);
        assert_eq!(second.policy_version, 9);
        assert_eq!(first.digest, request.digest);
        assert_eq!(
            second.digest, first.digest,
            "the version leaked into the digest"
        );
        assert_ne!(command(&write_request(2), 3).digest, first.digest);
    }

    #[test]
    fn a_prepared_command_recorded_at_another_time_than_the_supplied_one() {
        assert_eq!(command(&write_request(1), 3).recorded_at, AT);
    }

    #[test]
    fn a_prepared_command_without_the_server_caller() {
        let prepared = command(&write_request(1), 3);
        assert_eq!(
            prepared.caller,
            Some(Caller::Server(caller("/real/r/p", "/w")))
        );
    }

    #[test]
    fn a_prepared_command_whose_request_id_is_the_calls_identity_instead_of_the_supplied_id() {
        let prepared = command(&write_request(1), 3);
        assert_eq!(prepared.request_id, RequestId(REQUEST.into()));
        assert_eq!(prepared.kind, CommandKind("capture.record".into()));
        assert!(prepared.scope.is_empty());
    }
}
