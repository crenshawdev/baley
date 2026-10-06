//! A guard answer recorded in the per-user `user` project, and the answer
//! found for a redelivered call before git is read (design 0010 section 6,
//! GRD-R9 and GRD-R10). Every guard write is one command in `user` on stream `guard`,
//! at policy version 0, by Baley, with the hook's caller. The session
//! project is a fact in the caller and the payload, never the ledger
//! project, so recording needs no project ledger and admits no checkout.
//!
//! Each function here runs on a store the entry opened. None opens one, reads
//! git or reads a settings file.

use super::context::HookContext;
use super::decide::{Seen, Selected};
use super::unrecordable::not_recorded;
use crate::discovery::Discovery;
use baley_core::catalog::USER_PROJECT;
use baley_core::guard::{
    Answer, AnsweredFacts, AuditPrecondition, BranchObservation, GUARD_ANSWERED,
    GUARD_ANSWERED_VERSION, GUARD_COMMAND, GUARD_POLICY_RECORDED, GUARD_POLICY_RECORDED_VERSION,
    GUARD_POLICY_VIEW, GUARD_STREAM, GUARD_VIEW, GitVerb, GuardSettings, Redelivery, SettingsFact,
    SettingsInput, answered_payload, commit_push_answer, denial_parts, denials_changed, guard_key,
    guard_policy_key, policy_recorded_payload, redelivery, remembered_settings,
};
use baley_core::policy::{Fault, Unavailable};
use baley_store::{
    Actor, Admin, Caller, Command, CommandKind, Decision, HookCaller, Ledger, NewEvent, Observed,
    OutcomeKind, ProjectId, Refusal, RequestId, StoreError, StreamName, Transaction, Views,
    request_digest,
};
use serde_json::json;
use std::path::Path;

/// What a record keeps in place of git's stderr.
const REDACTED: &str = "[redacted]";

/// Where a commit's remembered denials are kept: the session project's
/// canonical root and the canonical root of the checkout the commit reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PolicyKey<'a> {
    /// The session project's repository root.
    pub project_root: &'a str,
    /// The target checkout's root, or `None` when the target is in none.
    pub checkout_root: Option<&'a str>,
}

/// How the answer to record was judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Judged<'a> {
    /// No settings were read, including a commit with an unestablished
    /// target. It is recorded as judged, with no remembered policy access.
    Absent(Answer),
    /// A commit judged under a complete policy. Its denials are remembered
    /// under `key` when they changed.
    Complete {
        /// The answer as judged.
        answer: Answer,
        /// The policy's guard settings.
        settings: &'a GuardSettings,
        /// Where its denials are remembered.
        key: PolicyKey<'a>,
    },
    /// A commit under torn settings. It is judged inside the transaction
    /// with the denials remembered under `key`.
    Torn {
        /// The settings file that gave no policy.
        torn: &'a Unavailable,
        /// Git's stderr ending the torn file's cause, which the record
        /// leaves out.
        excerpt: Option<&'a str>,
        /// The git verb.
        verb: GitVerb,
        /// The branch observation the commit is judged on.
        branch: &'a BranchObservation,
        /// Where the remembered denials are read from.
        key: PolicyKey<'a>,
    },
}

impl Judged<'_> {
    /// The answer when nothing remembered is read: the call judged on its
    /// own, as a clash is.
    fn alone(&self) -> Answer {
        match self {
            Judged::Absent(answer) | Judged::Complete { answer, .. } => answer.clone(),
            Judged::Torn {
                torn, verb, branch, ..
            } => commit_push_answer(
                *verb,
                true,
                &SettingsInput::Torn((*torn).clone()),
                None,
                branch,
            ),
        }
    }
}

/// How `selected`'s answer is recorded, or the line saying why it cannot be.
/// With no settings read it is recorded as judged. A commit's settings key
/// its remembered denials by the session project's root and the root of the
/// checkout the commit reaches. A failed target walk or a root that is not
/// UTF-8 cannot key a record.
pub(super) fn judged<'a>(
    answer: Answer,
    selected: &'a Selected,
    seen: &'a Seen,
    context: &'a HookContext,
) -> Result<Judged<'a>, String> {
    let Some(settings) = &selected.settings else {
        return Ok(Judged::Absent(answer));
    };
    let text = |root: &'a Path| {
        root.to_str()
            .ok_or_else(|| not_recorded(&format!("{} is not UTF-8", root.display())))
    };
    let project = context
        .project
        .as_ref()
        .ok_or_else(|| not_recorded("no session project is bound"))?;
    let checkout = if selected.target.is_some() {
        match &seen.target_checkout {
            Some(Ok(Discovery::Managed { root, .. } | Discovery::Unmanaged { root })) => {
                Some(root.as_path())
            }
            Some(Ok(Discovery::Outside)) => None,
            Some(Err(cause)) => return Err(not_recorded(cause)),
            None => return Err(not_recorded("the commit target checkout was not walked")),
        }
    } else {
        context.checkout.as_deref()
    };
    let key = PolicyKey {
        project_root: text(&project.root)?,
        checkout_root: checkout.map(text).transpose()?,
    };
    match (settings, &seen.branch, selected.verb) {
        (SettingsInput::Complete(settings), _, _) => Ok(Judged::Complete {
            answer,
            settings,
            key,
        }),
        (SettingsInput::Torn(torn), Some(branch), Some(verb)) => Ok(Judged::Torn {
            torn,
            excerpt: seen.excerpt.as_deref(),
            verb,
            branch,
            key,
        }),
        (SettingsInput::Torn(_), _, _) => Err(not_recorded(
            "torn settings came with no branch or git verb",
        )),
    }
}

/// What the decision settled before the store finished its transaction.
enum Settled {
    /// Another process answered the call first. The decision failed on
    /// purpose so nothing is appended, and this answer stands whatever the
    /// store then returns.
    AnsweredFirst(Answer, AuditPrecondition),
    /// A new answer appended inside the transaction. It is recorded only
    /// once the transaction commits.
    Staged(Answer),
}

/// Records `judged`'s answer for the call `caller` made, creating `user`
/// first when it is missing. It gives the answer to render and whether it is
/// recorded.
///
/// The call's `guard` document is read inside the transaction before
/// anything is appended. A replay gives the recorded answer and a clash the
/// call judged on its own, unrecordable. Either way the decision returns an
/// error so nothing is appended, and one call id never has two records.
/// Under torn settings the remembered denials are read and the commit judged
/// again; under a complete policy a changed denial is appended as
/// `guard.policy_recorded` beside the answer, never on its own. A store
/// error is returned as it is, one after the answer was appended included,
/// so a new answer counts as recorded only once its transaction commits.
pub(super) fn record(
    store: &(impl Admin + Ledger),
    caller: &HookCaller,
    selected: &Selected,
    judged: &Judged<'_>,
    request_id: RequestId,
    at: &str,
) -> Result<(Answer, AuditPrecondition), StoreError> {
    crate::models::create_user(store, at)?;
    let host = caller.host();
    let session = caller.host_session();
    let call = caller.call().text();
    let actor = Actor::Baley;
    let digest = request_digest(&json!({
        "kind": GUARD_COMMAND,
        "project": USER_PROJECT,
        "actor": actor.as_str(),
        "policy_version": 0,
        "scope": [],
        "host": host,
        "session": session,
        "call": call,
        "input_digest": selected.input_digest,
    }))
    .map_err(invalid)?;
    let command = Command {
        project: ProjectId(USER_PROJECT.into()),
        kind: CommandKind(GUARD_COMMAND.into()),
        request_id,
        digest,
        scope: vec![],
        policy_version: 0,
        recorded_at: at.into(),
        actor,
        caller: Some(Caller::Hook(caller.clone())),
    };
    let mut settled = None;
    let result = store.transact(&command, &mut |tx| {
        settled = None;
        let stored = tx.get(GUARD_VIEW, &guard_key(host, session, call))?;
        match redelivery(
            stored.as_ref().map(|document| &document.body),
            &selected.input_digest,
            caller.project_directory(),
            caller.working_directory(),
        ) {
            Redelivery::NoRecord => {}
            Redelivery::Replay(answer) => {
                settled = Some(Settled::AnsweredFirst(answer, AuditPrecondition::Recorded));
                return Err(answered_first(call));
            }
            Redelivery::Clash => {
                settled = Some(Settled::AnsweredFirst(
                    judged.alone(),
                    AuditPrecondition::Unrecordable,
                ));
                return Err(answered_first(call));
            }
        }
        let (answer, kept, torn) = decide(tx, judged)?;
        let settings = match (judged, &torn) {
            (Judged::Complete { settings, .. }, _) => SettingsFact::Complete(settings),
            (_, Some(torn)) => SettingsFact::Torn(torn),
            _ => SettingsFact::Absent,
        };
        let facts = AnsweredFacts {
            host,
            session,
            call,
            project_directory: caller.project_directory(),
            cwd: caller.working_directory(),
            tool: selected.tool,
            input_digest: &selected.input_digest,
            target: selected.target.as_deref(),
            verb: selected.verb,
            branch: selected.branch.as_deref(),
            settings,
        };
        let payload = answered_payload(&facts, &kept)
            .ok_or_else(|| invalid("a plain pass is never recorded"))?;
        tx.append(NewEvent {
            stream: StreamName(GUARD_STREAM.into()),
            type_name: GUARD_ANSWERED.into(),
            type_version: GUARD_ANSWERED_VERSION,
            git: None,
            payload,
            attachments: vec![],
        })?;
        settled = Some(Settled::Staged(answer));
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({ "recorded": true }),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    });
    match (result, settled) {
        (_, Some(Settled::AnsweredFirst(answer, precondition))) => Ok((answer, precondition)),
        (Ok(_), Some(Settled::Staged(answer))) => Ok((answer, AuditPrecondition::Recorded)),
        // The projectors, `command.completed` or the commit can fail after
        // the decision returned, and then nothing was recorded.
        (Err(error), _) => Err(error),
        // A fresh request id is never answered before.
        (Ok(_), None) => Ok((judged.alone(), AuditPrecondition::Unrecordable)),
    }
}

/// The answer to give, the answer the record keeps, and the torn file's
/// words as the record keeps them. Only torn settings read the remembered
/// denials, and only a complete policy writes them.
fn decide(
    tx: &mut dyn Transaction,
    judged: &Judged<'_>,
) -> Result<(Answer, Answer, Option<String>), StoreError> {
    match judged {
        Judged::Absent(answer) => Ok((answer.clone(), answer.clone(), None)),
        Judged::Complete {
            answer,
            settings,
            key,
        } => {
            let host = claude_code();
            let stored = tx.get(
                GUARD_POLICY_VIEW,
                &guard_policy_key(key.project_root, key.checkout_root, host),
            )?;
            let parts = denial_parts(settings);
            if denials_changed(stored.as_ref().map(|document| &document.body), &parts) {
                tx.append(NewEvent {
                    stream: StreamName(GUARD_STREAM.into()),
                    type_name: GUARD_POLICY_RECORDED.into(),
                    type_version: GUARD_POLICY_RECORDED_VERSION,
                    git: None,
                    payload: policy_recorded_payload(
                        key.project_root,
                        key.checkout_root,
                        host,
                        &parts,
                    ),
                    attachments: vec![],
                })?;
            }
            Ok((answer.clone(), answer.clone(), None))
        }
        Judged::Torn {
            torn,
            excerpt,
            verb,
            branch,
            key,
        } => {
            let stored = tx.get(
                GUARD_POLICY_VIEW,
                &guard_policy_key(key.project_root, key.checkout_root, claude_code()),
            )?;
            let remembered = stored
                .as_ref()
                .and_then(|document| remembered_settings(&document.body));
            let judge = |torn: Unavailable| {
                commit_push_answer(
                    *verb,
                    true,
                    &SettingsInput::Torn(torn),
                    remembered.as_ref(),
                    branch,
                )
            };
            let kept_torn = without_excerpt(torn, *excerpt);
            Ok((
                judge((*torn).clone()),
                judge(kept_torn.clone()),
                Some(kept_torn.to_string()),
            ))
        }
    }
}

/// The torn file's refusal as a record keeps it: Baley's words, with git's
/// excerpt replaced. The excerpt ends the cause, so it is cut from the end,
/// and replaced wherever it stands should it not.
fn without_excerpt(torn: &Unavailable, excerpt: Option<&str>) -> Unavailable {
    let (Some(excerpt), Fault::Unreadable { cause }) = (excerpt, &torn.fault) else {
        return torn.clone();
    };
    let cause = match cause.strip_suffix(excerpt) {
        Some(words) => format!("{words}{REDACTED}"),
        None => cause.replace(excerpt, REDACTED),
    };
    Unavailable {
        path: torn.path.clone(),
        fault: Fault::Unreadable { cause },
    }
}

/// Claude Code's host name, the third part of the remembered-policy key.
fn claude_code() -> &'static str {
    baley_core::policy::Host::ClaudeCode.name()
}

/// The answer recorded under the call's id, read before any git or policy
/// read. With no `user` there is no record, and nothing is created. It reads
/// no policy and writes nothing.
pub(super) fn lookup(
    store: &(impl Admin + Views),
    caller: &HookCaller,
    input_digest: &str,
) -> Result<Redelivery, StoreError> {
    let user = ProjectId(USER_PROJECT.into());
    if !store.projects()?.iter().any(|(known, _)| *known == user) {
        return Ok(Redelivery::NoRecord);
    }
    let key = guard_key(caller.host(), caller.host_session(), caller.call().text());
    let stored = store.get(&user, GUARD_VIEW, &key)?;
    Ok(redelivery(
        stored.as_ref().map(|document| &document.body),
        input_digest,
        caller.project_directory(),
        caller.working_directory(),
    ))
}

/// The error a decision returns once another process answered the call id
/// first, so nothing is appended.
fn answered_first(call: &str) -> StoreError {
    invalid(format!("call {call} was answered first by another process"))
}

fn invalid(error: impl std::fmt::Display) -> StoreError {
    StoreError::Refused(Refusal::InvalidEvent(error.to_string()))
}

#[cfg(test)]
mod tests {
    //! Real SQLite in a fresh temporary directory, opened under the guard's
    //! options on a fixed clock, with supplied times.

    use super::*;
    use crate::ledger::open::{self, tests::guard};
    use baley_core::guard::{
        GUARD_POLICY_RECORDED, ToolInput, input_digest, reason, remembered_settings,
    };
    use baley_core::policy::OnProtected;
    use baley_store::{
        Anchor, AnchorCheck, CallSource, Claim, ClaimId, ClaimOwner, Claimed, Decide, DecideClaim,
        DecideReconcile, Event, ExportReport, Hash, Head, Health, HistoryFilter, Page, PageRequest,
        PayloadRef, PayloadReference, PurgeReport, RebuildReport, ReconcileAuthority, Recorded,
        ScrubReport, VerifyReport, ViewsReport,
    };
    use baley_store_sqlite::SqliteStore;
    use std::collections::BTreeMap;
    use std::ops::RangeInclusive;

    const T0: &str = "2026-10-06T10:00:00Z";
    const T1: &str = "2026-10-06T10:00:01Z";
    /// Not canonical, so a record that rewrote it would show.
    const PROJECT_DIR: &str = "/p/./";
    const CWD: &str = "/q/src";
    const KEY: PolicyKey<'static> = PolicyKey {
        project_root: "/p",
        checkout_root: Some("/q"),
    };

    fn open(dir: &tempfile::TempDir) -> SqliteStore {
        open::store(&dir.path().join("home"), T0, guard()).unwrap()
    }

    fn caller(call: &str) -> HookCaller {
        HookCaller::new("claude-code", CWD, call)
            .unwrap()
            .with_project_directory(PROJECT_DIR)
            .unwrap()
            .with_host_session("s")
            .unwrap()
    }

    fn request(n: u64) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-{n:012}"))
    }

    fn bash(command: &str, verb: GitVerb) -> Selected {
        Selected {
            tool: "Bash",
            input_digest: input_digest(
                "Bash",
                &ToolInput {
                    command: Some(command),
                    ..ToolInput::default()
                },
            ),
            target: None,
            verb: Some(verb),
            branch: Some("main".into()),
            settings: None,
        }
    }

    fn commit() -> Selected {
        bash("git commit -m x", GitVerb::Commit)
    }

    fn refusing() -> GuardSettings {
        GuardSettings {
            protected_branches: vec!["main".into()],
            on_protected: OnProtected::Refuse,
            hard_fail: false,
        }
    }

    fn refused_on_main<'a>(settings: &'a GuardSettings, key: PolicyKey<'a>) -> Judged<'a> {
        Judged::Complete {
            answer: Answer::Deny(reason::refuse_deny("main")),
            settings,
            key,
        }
    }

    fn events(store: &SqliteStore, type_name: &str) -> Vec<Event> {
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        store
            .stream(
                &ProjectId(USER_PROJECT.into()),
                &StreamName(GUARD_STREAM.into()),
                1,
                page,
            )
            .unwrap()
            .items
            .into_iter()
            .filter(|event| event.type_name == type_name)
            .collect()
    }

    fn remembered(store: &SqliteStore, key: PolicyKey<'_>) -> Option<GuardSettings> {
        let stored = store
            .get(
                &ProjectId(USER_PROJECT.into()),
                GUARD_POLICY_VIEW,
                &guard_policy_key(key.project_root, key.checkout_root, "claude-code"),
            )
            .unwrap();
        stored.and_then(|document| remembered_settings(&document.body))
    }

    fn context() -> HookContext {
        HookContext {
            project_directory: Ok(Some(PROJECT_DIR.into())),
            project: Some(super::super::context::Bound {
                folder: "/p/policy".into(),
                root: "/p".into(),
            }),
            checkout: Some("/q".into()),
            protected: Err(crate::folders::FolderRefusal::UserHomeUnset),
        }
    }

    #[test]
    fn a_redirected_policy_key_borrowing_the_cwd_or_project_folder_is_caught() {
        let context = context();
        for (walk, expected) in [
            (
                Discovery::Managed {
                    folder: "/r/policy".into(),
                    root: "/r".into(),
                },
                Some("/r"),
            ),
            (Discovery::Unmanaged { root: "/r".into() }, Some("/r")),
            (Discovery::Outside, None),
        ] {
            let seen = Seen {
                branch: Some(BranchObservation::Read("main".into())),
                target_checkout: Some(Ok(walk)),
                ..Seen::default()
            };
            for settings in [
                SettingsInput::Complete(refusing()),
                SettingsInput::Torn(Unavailable {
                    path: "/p/baley.toml".into(),
                    fault: Fault::NotRegular,
                }),
            ] {
                let selected = Selected {
                    target: Some("/r/src".into()),
                    settings: Some(settings),
                    ..commit()
                };
                let judged =
                    judged(Answer::Ask("check".into()), &selected, &seen, &context).unwrap();
                let key = match judged {
                    Judged::Complete { key, .. } | Judged::Torn { key, .. } => key,
                    other => panic!("a commit's policy key: {other:?}"),
                };
                assert_eq!(
                    key,
                    PolicyKey {
                        project_root: "/p",
                        checkout_root: expected
                    }
                );
            }
        }
    }

    #[test]
    fn an_unreadable_target_walk_falling_back_to_a_remembered_key_is_caught() {
        let selected = Selected {
            target: Some("/r/src".into()),
            settings: Some(SettingsInput::Complete(refusing())),
            ..commit()
        };
        for target_checkout in [None, Some(Err("target cannot be walked".into()))] {
            let seen = Seen {
                target_checkout,
                ..Seen::default()
            };
            assert!(judged(Answer::Deny("main".into()), &selected, &seen, &context()).is_err());
        }
    }

    #[test]
    fn a_storage_round_trip_losing_the_supplied_target_or_checkout_key_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let selected = Selected {
            target: Some("/r/src".into()),
            ..bash("git -C /r/src commit", GitVerb::Commit)
        };
        let key = PolicyKey {
            project_root: "/p",
            checkout_root: Some("/r"),
        };
        let refuse = refusing();
        let given = record(
            &store,
            &caller("t1"),
            &selected,
            &refused_on_main(&refuse, key),
            request(1),
            T1,
        );

        assert_eq!(
            given,
            Ok((
                Answer::Deny(reason::refuse_deny("main")),
                AuditPrecondition::Recorded
            ))
        );
        let answered = events(&store, GUARD_ANSWERED);
        assert_eq!(answered[0].payload["target"], "/r/src");
        assert_eq!(answered[0].payload["cwd"], CWD);
        assert_eq!(answered[0].payload["project_directory"], PROJECT_DIR);
        let policies = events(&store, GUARD_POLICY_RECORDED);
        assert_eq!(policies[0].payload["checkout_root"], "/r");
        assert_eq!(remembered(&store, key), Some(refuse));
        assert_eq!(remembered(&store, KEY), None);
    }

    #[test]
    fn a_guard_record_needing_a_project_ledger_or_carrying_a_policy_version_or_command_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let ask = Answer::Ask(reason::push_ask());
        let push = bash("git push origin SENTINEL", GitVerb::Push);

        let given = record(
            &store,
            &caller("t1"),
            &push,
            &Judged::Absent(ask.clone()),
            request(1),
            T1,
        );

        assert_eq!(given, Ok((ask, AuditPrecondition::Recorded)));
        // The record made `user`, and no ledger for the session project.
        let projects: Vec<_> = store
            .projects()
            .unwrap()
            .into_iter()
            .map(|(id, _)| id.0)
            .collect();
        assert_eq!(projects, [USER_PROJECT]);
        let answered = events(&store, GUARD_ANSWERED);
        assert_eq!(answered.len(), 1);
        let event = &answered[0];
        assert_eq!(event.policy_version, 0);
        assert_eq!(event.actor, Actor::Baley);
        let Some(Caller::Hook(hook)) = &event.caller else {
            panic!("a hook caller: {:?}", event.caller);
        };
        assert_eq!(hook.call().text(), "t1");
        assert_eq!(hook.call().source(), CallSource::ToolUseId);
        assert_eq!(hook.project_directory(), Some(PROJECT_DIR));
        assert_eq!(hook.working_directory(), CWD);
        assert_eq!(hook.host_session(), Some("s"));
        assert!(
            !event.payload.to_string().contains("SENTINEL"),
            "{}",
            event.payload
        );
    }

    #[test]
    fn a_redelivered_call_recorded_twice_or_given_a_new_answer_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let first = Answer::Deny("first".into());
        record(
            &store,
            &caller("t1"),
            &commit(),
            &Judged::Absent(first.clone()),
            request(1),
            T1,
        )
        .unwrap();

        let again = record(
            &store,
            &caller("t1"),
            &commit(),
            &Judged::Absent(Answer::Ask("second".into())),
            request(2),
            T1,
        );

        assert_eq!(again, Ok((first, AuditPrecondition::Recorded)));
        assert_eq!(events(&store, GUARD_ANSWERED).len(), 1);
    }

    #[test]
    fn a_reused_call_id_with_other_input_recorded_or_left_recordable_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let ask = Answer::Ask(reason::push_ask());
        let push = |command| bash(command, GitVerb::Push);
        record(
            &store,
            &caller("t1"),
            &push("git push a"),
            &Judged::Absent(ask.clone()),
            request(1),
            T1,
        )
        .unwrap();

        let reused = record(
            &store,
            &caller("t1"),
            &push("git push b"),
            &Judged::Absent(ask.clone()),
            request(2),
            T1,
        );

        assert_eq!(reused, Ok((ask, AuditPrecondition::Unrecordable)));
        assert_eq!(events(&store, GUARD_ANSWERED).len(), 1);
    }

    /// The real store, whose transaction fails once the decision returned:
    /// the window in which the projectors, `command.completed` or the commit
    /// can fail. SQLite rolls the decision's appends back.
    struct FailsAfterDecide<'a> {
        store: &'a SqliteStore,
    }

    fn commit_failed() -> StoreError {
        StoreError::Unavailable("the commit failed".into())
    }

    impl Ledger for FailsAfterDecide<'_> {
        fn transact(
            &self,
            command: &Command,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::transact(self.store, command, &mut |tx| {
                decide(tx)?;
                Err(commit_failed())
            })
        }

        fn claim(
            &self,
            command: &Command,
            decide: &mut DecideClaim<'_>,
        ) -> Result<Claimed, StoreError> {
            Ledger::claim(self.store, command, decide)
        }

        fn renew_lease(
            &self,
            project: &ProjectId,
            claim: &ClaimId,
            owner: &ClaimOwner,
            at: &str,
        ) -> Result<(), StoreError> {
            Ledger::renew_lease(self.store, project, claim, owner, at)
        }

        fn complete(
            &self,
            command: &Command,
            owner: &ClaimOwner,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::complete(self.store, command, owner, decide)
        }

        fn reconcile(
            &self,
            command: &Command,
            claim: &ClaimId,
            authority: ReconcileAuthority,
            decide: &mut DecideReconcile<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::reconcile(self.store, command, claim, authority, decide)
        }

        fn open_claims(&self, project: &ProjectId) -> Result<Vec<Claim>, StoreError> {
            Ledger::open_claims(self.store, project)
        }

        fn stream(
            &self,
            project: &ProjectId,
            stream: &StreamName,
            from_version: u64,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::stream(self.store, project, stream, from_version, page)
        }

        fn history(
            &self,
            project: &ProjectId,
            range: RangeInclusive<u64>,
            filter: &HistoryFilter,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::history(self.store, project, range, filter, page)
        }

        fn head(&self, project: &ProjectId) -> Result<Option<Head>, StoreError> {
            Ledger::head(self.store, project)
        }

        fn verify(
            &self,
            project: &ProjectId,
            anchor: Option<&Anchor>,
        ) -> Result<VerifyReport, StoreError> {
            Ledger::verify(self.store, project, anchor)
        }
    }

    impl Admin for FailsAfterDecide<'_> {
        fn create_project(
            &self,
            project: &ProjectId,
            name: &str,
            at: &str,
        ) -> Result<(), StoreError> {
            Admin::create_project(self.store, project, name, at)
        }

        fn projects(&self) -> Result<Vec<(ProjectId, String)>, StoreError> {
            Admin::projects(self.store)
        }

        fn export(
            &self,
            project: &ProjectId,
            target: &Path,
            at: &str,
        ) -> Result<ExportReport, StoreError> {
            Admin::export(self.store, project, target, at)
        }

        fn reduce(
            &self,
            command: &Command,
            reference: &PayloadReference,
        ) -> Result<PayloadRef, StoreError> {
            Admin::reduce(self.store, command, reference)
        }

        fn purge(
            &self,
            command: &Command,
            hashes: &[Hash],
            reason: &str,
        ) -> Result<PurgeReport, StoreError> {
            Admin::purge(self.store, command, hashes, reason)
        }

        fn scrub(&self) -> Result<ScrubReport, StoreError> {
            Admin::scrub(self.store)
        }

        fn rebuild(&self, project: &ProjectId) -> Result<RebuildReport, StoreError> {
            Admin::rebuild(self.store, project)
        }

        fn verify_views(&self, project: &ProjectId) -> Result<ViewsReport, StoreError> {
            Admin::verify_views(self.store, project)
        }

        fn doctor(
            &self,
            at: &str,
            checks: &BTreeMap<ProjectId, AnchorCheck>,
        ) -> Result<Health, StoreError> {
            Admin::doctor(self.store, at, checks)
        }
    }

    #[test]
    fn an_ask_reported_recorded_when_its_transaction_fails_after_the_decision_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let failing = FailsAfterDecide { store: &store };
        let ask = Answer::Ask(reason::push_ask());

        let given = record(
            &failing,
            &caller("t1"),
            &bash("git push", GitVerb::Push),
            &Judged::Absent(ask),
            request(1),
            T1,
        );

        // The store error goes to the hook's store-failure path, which
        // makes the ask unrecordable.
        assert_eq!(given, Err(commit_failed()));
        assert_eq!(events(&store, GUARD_ANSWERED).len(), 0);
    }

    #[test]
    fn a_remembered_refuse_ignored_when_torn_or_applied_to_another_checkout_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let refuse = refusing();
        record(
            &store,
            &caller("t1"),
            &commit(),
            &refused_on_main(&refuse, KEY),
            request(1),
            T1,
        )
        .unwrap();
        let torn = Unavailable {
            path: "/p/baley.toml".into(),
            fault: Fault::NotRegular,
        };
        let main = BranchObservation::Read("main".into());
        let torn_at = |key| Judged::Torn {
            torn: &torn,
            excerpt: None,
            verb: GitVerb::Commit,
            branch: &main,
            key,
        };

        let same = record(
            &store,
            &caller("t2"),
            &commit(),
            &torn_at(KEY),
            request(2),
            T1,
        );
        let other = PolicyKey {
            checkout_root: Some("/r"),
            ..KEY
        };
        let elsewhere = record(
            &store,
            &caller("t3"),
            &commit(),
            &torn_at(other),
            request(3),
            T1,
        );

        let words = torn.to_string();
        assert_eq!(
            same,
            Ok((
                Answer::Deny(reason::remembered_refuse_deny(&words, "main")),
                AuditPrecondition::Recorded
            ))
        );
        assert_eq!(
            elsewhere,
            Ok((
                Answer::Ask(reason::torn_ask(&words, Some("main"))),
                AuditPrecondition::Recorded
            ))
        );
    }

    #[test]
    fn a_policy_recorded_on_every_call_or_never_cleared_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let refuse = refusing();
        for (call, n) in [("t1", 1), ("t2", 2)] {
            record(
                &store,
                &caller(call),
                &commit(),
                &refused_on_main(&refuse, KEY),
                request(n),
                T1,
            )
            .unwrap();
        }
        assert_eq!(events(&store, GUARD_POLICY_RECORDED).len(), 1);

        let asking = GuardSettings {
            on_protected: OnProtected::Ask,
            ..refusing()
        };
        let judged = Judged::Complete {
            answer: Answer::Ask(reason::protected_ask("main")),
            settings: &asking,
            key: KEY,
        };
        record(&store, &caller("t3"), &commit(), &judged, request(3), T1).unwrap();

        assert_eq!(events(&store, GUARD_POLICY_RECORDED).len(), 2);
        let cleared = remembered(&store, KEY).expect("a document");
        assert_ne!(cleared.on_protected, OnProtected::Refuse);
        assert!(!cleared.hard_fail);
    }

    #[test]
    fn absent_settings_treated_as_a_complete_policy_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let refuse = refusing();
        record(
            &store,
            &caller("t1"),
            &commit(),
            &refused_on_main(&refuse, KEY),
            request(1),
            T1,
        )
        .unwrap();
        // An outside-project path denial: no project directory, no settings.
        let write = Selected {
            tool: "Write",
            target: Some("/u/.config/crenshawdev/baley/config.toml".into()),
            verb: None,
            branch: None,
            ..commit()
        };
        let unbound = HookCaller::new("claude-code", CWD, "t2").unwrap();
        let denied = Answer::Deny("Baley guard: the config folder is protected".into());
        let push = bash("git push", GitVerb::Push);
        let asked = Answer::Ask(reason::push_ask());

        record(
            &store,
            &unbound,
            &write,
            &Judged::Absent(denied),
            request(2),
            T1,
        )
        .unwrap();
        record(
            &store,
            &caller("t3"),
            &push,
            &Judged::Absent(asked),
            request(3),
            T1,
        )
        .unwrap();

        assert_eq!(events(&store, GUARD_ANSWERED).len(), 3);
        assert_eq!(events(&store, GUARD_POLICY_RECORDED).len(), 1);
        assert_eq!(remembered(&store, KEY), Some(refusing()));
    }

    #[test]
    fn git_stderr_reaching_the_guard_record_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let words = "HEAD's copy: git rev-parse --verify -q HEAD exited with code 128";
        let torn = Unavailable {
            path: "/p/baley.toml".into(),
            fault: Fault::Unreadable {
                cause: format!("{words}: fatal: SENTINEL"),
            },
        };
        let main = BranchObservation::Read("main".into());
        let judged = Judged::Torn {
            torn: &torn,
            excerpt: Some("fatal: SENTINEL"),
            verb: GitVerb::Commit,
            branch: &main,
            key: KEY,
        };

        let (answer, _) =
            record(&store, &caller("t1"), &commit(), &judged, request(1), T1).unwrap();

        let Answer::Ask(live) = answer else {
            panic!("an ask: {answer:?}");
        };
        assert!(
            live.contains(&format!("{words}: fatal: SENTINEL")),
            "{live}"
        );
        let answered = events(&store, GUARD_ANSWERED);
        let payload = &answered[0].payload;
        assert!(!payload.to_string().contains("SENTINEL"), "{payload}");
        let kept = format!("{words}: [redacted]");
        for recorded in [&payload["reason"], &payload["settings"]["torn"]] {
            let text = recorded.as_str().expect("text");
            assert!(text.contains(&kept), "{text}");
        }
    }

    #[test]
    fn a_replay_that_changes_after_the_policy_changed_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let refuse = refusing();
        let judged = refused_on_main(&refuse, KEY);
        record(&store, &caller("t1"), &commit(), &judged, request(1), T1).unwrap();
        // Judged fresh under `allow`, the same commit would pass.
        let allowing = GuardSettings {
            on_protected: OnProtected::Allow,
            ..refusing()
        };
        let main = BranchObservation::Read("main".into());
        let fresh = commit_push_answer(
            GitVerb::Commit,
            true,
            &SettingsInput::Complete(allowing),
            None,
            &main,
        );
        assert_eq!(fresh, Answer::Pass);

        let found = lookup(&store, &caller("t1"), &commit().input_digest);

        assert_eq!(
            found,
            Ok(Redelivery::Replay(Answer::Deny(reason::refuse_deny(
                "main"
            ))))
        );
    }

    #[test]
    fn different_input_accepted_under_one_call_id_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let ask = Answer::Ask(reason::push_ask());
        let push = bash("git push a", GitVerb::Push);
        record(
            &store,
            &caller("t1"),
            &push,
            &Judged::Absent(ask),
            request(1),
            T1,
        )
        .unwrap();

        let other = bash("git push b", GitVerb::Push);
        let found = lookup(&store, &caller("t1"), &other.input_digest);

        assert_eq!(found, Ok(Redelivery::Clash));
    }

    #[test]
    fn a_lookup_that_creates_user_or_finds_a_record_on_a_fresh_home_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);

        let found = lookup(&store, &caller("t1"), &commit().input_digest);

        assert_eq!(found, Ok(Redelivery::NoRecord));
        assert_eq!(store.projects(), Ok(vec![]));
    }

    #[test]
    fn a_user_left_at_view_set_6_read_as_recorded_is_caught() {
        use crate::guard_hook::unrecordable::store_failure;
        use crate::ledger::open::tests::view_set_6;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let old = open::store(&home, T0, view_set_6()).unwrap();
        crate::models::create_user(&old, T0).unwrap();
        // A `user` with no events has no views to bring forward.
        crate::models::record_seed(&old, request(9), T0).unwrap();
        drop(old);
        let guarded = open::store(&home, T1, guard()).unwrap();

        let found = lookup(&guarded, &caller("t1"), &commit().input_digest);

        let needs = StoreError::NeedsRebuild {
            project: ProjectId(USER_PROJECT.into()),
        };
        assert_eq!(found, Err(needs.clone()));
        assert_eq!(store_failure(&needs).0, AuditPrecondition::Unrecordable);
    }

    #[test]
    fn a_policy_key_from_the_wrong_roots_or_from_a_root_that_is_not_utf8_is_caught() {
        use crate::guard_hook::context::Bound;
        use crate::protected_paths::ProtectedPaths;
        use std::os::unix::ffi::OsStrExt;

        let torn = Unavailable {
            path: "/p/baley.toml".into(),
            fault: Fault::NotRegular,
        };
        let selected = Selected {
            settings: Some(SettingsInput::Torn(torn.clone())),
            ..commit()
        };
        let main = BranchObservation::Read("main".into());
        let seen = Seen {
            branch: Some(main.clone()),
            excerpt: Some("fatal: x".into()),
            ..Seen::default()
        };
        let mut context = HookContext {
            project_directory: Ok(Some("/p".into())),
            project: Some(Bound {
                folder: "/p/sub".into(),
                root: "/p".into(),
            }),
            checkout: Some("/q".into()),
            protected: Ok(ProtectedPaths {
                home: "/h".into(),
                config: "/c".into(),
                files: vec![],
            }),
        };
        let ask = Answer::Ask("approve?".into());

        assert_eq!(
            judged(ask.clone(), &selected, &seen, &context),
            Ok(Judged::Torn {
                torn: &torn,
                excerpt: Some("fatal: x"),
                verb: GitVerb::Commit,
                branch: &main,
                key: KEY,
            })
        );
        assert_eq!(
            judged(ask.clone(), &commit(), &seen, &context),
            Ok(Judged::Absent(ask.clone()))
        );
        context.checkout = Some(std::ffi::OsStr::from_bytes(b"/q\xff").into());
        assert!(judged(ask, &selected, &seen, &context).is_err());
    }
}
