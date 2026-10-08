//! The server's project and working directory, as the last recorded server
//! call shows them.
//!
//! The ledger keeps, on every event a server call appended, the
//! `CLAUDE_PROJECT_DIR` and the working directory the server started with.
//! Gathering pages through each project's history with the storage port's
//! existing read, keeps the latest server call and asks whether its project
//! folder is a directory now. Judging runs the server's own rule,
//! [`project_context`], over that text, so the doctor and the server cannot
//! disagree. The doctor process's own environment is never read: a plain
//! terminal has no `CLAUDE_PROJECT_DIR`, which would be a false finding.
//!
//! A server without the variable answers `project-context-missing` and
//! records nothing, so a missing variable never comes from the ledger. The
//! observation allows it anyway, because that is the server's rule, and
//! the tests prove the wording over a supplied observation.

use std::ffi::OsStr;

use baley_store::{
    Caller, EVENT_PAGE_BOUND, Event, HistoryFilter, Ledger, PageRequest, ProjectId, StoreError,
    UtcInstant,
};

use crate::mcp::context::{ProjectContext, project_context};

/// One server call read from a project's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// The event's sequence in its project.
    pub seq: u64,
    /// The `CLAUDE_PROJECT_DIR` the server recorded.
    pub project_directory: String,
    /// The working directory the server recorded.
    pub working_directory: String,
    /// When the event was recorded.
    pub recorded_at: String,
}

/// The last recorded server call and what was observed about its project
/// folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCall {
    /// The recorded `CLAUDE_PROJECT_DIR`, or none when the server had none.
    pub project: Option<String>,
    /// Whether that path is an existing directory now, a link followed.
    pub project_is_directory: bool,
    /// The recorded working directory.
    pub working_directory: String,
    /// When the call was recorded.
    pub recorded_at: String,
}

/// What the ledger showed about the server's context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// No event in any project carries a server caller.
    NoCall,
    /// The last recorded server call.
    Call(ServerCall),
    /// A history read failed; the store's text.
    ReadError(String),
}

/// The server call kept after one more page: the page's last event with a
/// server caller, or `kept` when the page holds none. Events are compared by
/// sequence, so the order of the page does not matter.
pub fn last_server_call(kept: Option<Recorded>, page: &[Event]) -> Option<Recorded> {
    page.iter()
        .filter_map(|event| match &event.caller {
            Some(Caller::Server(caller)) => Some(Recorded {
                seq: event.seq,
                project_directory: caller.project_directory().to_owned(),
                working_directory: caller.working_directory().to_owned(),
                recorded_at: event.recorded_at.clone(),
            }),
            _ => None,
        })
        .chain(kept)
        .max_by_key(|call| call.seq)
}

/// The call recorded latest across projects. `calls` holds one call per
/// project in project id order; instants are compared whole, fraction of a
/// second included, never as text, and a tie keeps the earlier project. A
/// time that does not parse never wins over one that does.
pub fn latest(calls: Vec<Recorded>) -> Option<Recorded> {
    let mut best: Option<(Option<UtcInstant>, Recorded)> = None;
    for call in calls {
        let at = UtcInstant::parse(&call.recorded_at).ok();
        if !matches!(&best, Some((held, _)) if at <= *held) {
            best = Some((at, call));
        }
    }
    best.map(|(_, call)| call)
}

/// Reads every project's history and observes the latest server call. It
/// reads only; it owns no rule and has no unit test of its own.
pub fn gather<S: Ledger>(store: &S, projects: &[ProjectId]) -> Observed {
    let mut calls = Vec::new();
    for project in projects {
        match last_in(store, project) {
            Ok(Some(call)) => calls.push(call),
            Ok(None) => {}
            Err(error) => return Observed::ReadError(format!("project {}: {error}", project.0)),
        }
    }
    match latest(calls) {
        None => Observed::NoCall,
        Some(call) => Observed::Call(ServerCall {
            project_is_directory: std::fs::metadata(&call.project_directory)
                .is_ok_and(|meta| meta.is_dir()),
            project: Some(call.project_directory),
            working_directory: call.working_directory,
            recorded_at: call.recorded_at,
        }),
    }
}

/// One project's last server call, over its whole chain.
fn last_in<S: Ledger>(store: &S, project: &ProjectId) -> Result<Option<Recorded>, StoreError> {
    let filter = HistoryFilter::default();
    let mut kept = None;
    let mut after = None;
    loop {
        let page = PageRequest {
            limit: EVENT_PAGE_BOUND,
            after,
        };
        let page = store.history(project, 1..=u64::MAX, &filter, page)?;
        kept = last_server_call(kept, &page.items);
        match page.next {
            Some(cursor) => after = Some(cursor),
            None => return Ok(kept),
        }
    }
}

/// The server's context, judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judged {
    /// No server call is recorded.
    NoCall,
    /// The read failed; the store's text.
    ReadError(String),
    /// The last server call.
    Call {
        /// The server's own judgement of the project variable.
        project: ProjectContext,
        /// The recorded project text, when the server had a variable.
        recorded: Option<String>,
        /// The recorded working directory. It may differ from the project.
        working_directory: String,
        /// When the call was recorded.
        recorded_at: String,
    },
}

/// Judges the observation. The project variable goes through the server's
/// own rule, unchanged.
pub fn judge(observed: &Observed) -> Judged {
    match observed {
        Observed::NoCall => Judged::NoCall,
        Observed::ReadError(text) => Judged::ReadError(text.clone()),
        Observed::Call(call) => Judged::Call {
            project: project_context(
                call.project.as_deref().map(OsStr::new),
                call.project_is_directory,
            ),
            recorded: call.project.clone(),
            working_directory: call.working_directory.clone(),
            recorded_at: call.recorded_at.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use baley_store::{Actor, Hash, HookCaller, RequestId, ServerCaller};
    use serde_json::json;

    use super::*;

    fn server(project: &str, working: &str) -> Caller {
        Caller::Server(
            ServerCaller::new(
                project,
                working,
                "claude-code",
                "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f",
                &json!(1),
            )
            .unwrap(),
        )
    }

    fn hook() -> Caller {
        Caller::Hook(HookCaller::new("claude-code", "/hook/dir", "toolu_1").unwrap())
    }

    fn event(seq: u64, caller: Option<Caller>) -> Event {
        Event {
            project_id: ProjectId("p".into()),
            seq,
            stream: "s".into(),
            stream_version: seq,
            type_name: "t".into(),
            type_version: 1,
            actor: Actor::Baley,
            caller,
            recorded_at: "2026-10-08T10:00:00Z".into(),
            request_id: RequestId(format!("r{seq}")),
            git: None,
            policy_version: 0,
            payload: json!({}),
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    fn recorded(at: &str, project: &str) -> Recorded {
        Recorded {
            seq: 1,
            project_directory: project.into(),
            working_directory: project.into(),
            recorded_at: at.into(),
        }
    }

    #[test]
    fn a_hook_caller_or_an_earlier_server_call_chosen_as_the_last_server_call_is_caught() {
        let first = [
            event(1, Some(server("/p1", "/p1"))),
            event(2, Some(hook())),
            event(3, Some(server("/p3", "/p3/sub"))),
            event(4, None),
        ];
        let kept = last_server_call(None, &first).expect("a server call");
        assert_eq!(
            (
                kept.seq,
                kept.project_directory.as_str(),
                kept.working_directory.as_str()
            ),
            (3, "/p3", "/p3/sub")
        );

        let second = [event(5, Some(hook())), event(6, None)];
        let kept = last_server_call(Some(kept), &second).expect("the earlier page's call");
        assert_eq!(
            (
                kept.seq,
                kept.project_directory.as_str(),
                kept.working_directory.as_str()
            ),
            (3, "/p3", "/p3/sub")
        );
    }

    #[test]
    fn the_last_server_call_ordered_by_text_or_by_project_instead_of_time_is_caught() {
        let chosen = |a: &str, b: &str| {
            latest(vec![recorded(a, "/a"), recorded(b, "/b")])
                .expect("a call")
                .project_directory
        };
        assert_eq!(
            chosen("2026-10-08T10:00:05.5Z", "2026-10-08T10:00:05Z"),
            "/a"
        );
        assert_eq!(
            chosen("2026-10-08T10:00:05Z", "2026-10-08T10:00:05.5Z"),
            "/b"
        );
    }
}
