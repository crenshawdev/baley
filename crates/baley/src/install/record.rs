//! Records changed installation ownership facts in `user` and reads the latest
//! payload for a host from the `install` view. Times and request ids are supplied.

use baley_core::catalog::USER_PROJECT;
use baley_store::{
    Actor, Admin, Command, CommandKind, Decision, Ledger, Observed, OutcomeKind, ProjectId,
    Refusal, RequestId, StaleInput, StoreError, Views, request_digest,
};
use serde_json::{Value, json};

use crate::models;

use super::event::{self, Facts};
use super::view::{INSTALL_VIEW, install_key};

/// The command kind used to record installation ownership facts.
pub const RECORD_COMMAND: &str = "install.record";

/// The latest record of a host: the sequence of the event that wrote it and its
/// payload. A later run names the sequence to show which record it planned from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    /// The sequence of the `install.recorded` event behind the payload.
    pub seq: u64,
    /// The recorded payload.
    pub payload: Value,
}

/// Reads the latest record for `host`, or none when `user` or its record is absent.
pub fn latest(store: &(impl Admin + Views), host: &str) -> Result<Option<Latest>, StoreError> {
    let project = ProjectId(USER_PROJECT.into());
    if !store.projects()?.iter().any(|(known, _)| *known == project) {
        return Ok(None);
    }
    let stored = store.get(&project, INSTALL_VIEW, &install_key(host))?;
    Ok(stored.map(|document| Latest {
        seq: document.produced_seq,
        payload: document.body["payload"].clone(),
    }))
}

/// Reads the latest payload for `host`, or none when `user` or its record is absent.
pub fn read(store: &(impl Admin + Views), host: &str) -> Result<Option<Value>, StoreError> {
    Ok(latest(store, host)?.map(|latest| latest.payload))
}

/// Records supplied facts as the owner, creating `user` when absent. The request
/// id must be fresh. Returns true only when this call appended `install.recorded`.
/// An unchanged record skips the command. The comparison runs again inside the
/// transaction, so a racing match records only `command.completed`.
/// `seen` is the sequence of the record the facts were planned from, or none when
/// the run saw no record. The facts keep ownership hashes from that record, so a
/// newer record in the store means they may be stale, and nothing is recorded.
pub fn write(
    store: &(impl Admin + Views + Ledger),
    facts: &Facts,
    seen: Option<u64>,
    request_id: RequestId,
    at: &str,
) -> Result<bool, StoreError> {
    let event = event::recorded_event(facts);
    // Avoid even a command completion when the installation already matches.
    if read(store, facts.host.name())?.as_ref() == Some(&event.payload) {
        return Ok(false);
    }
    let actor = Actor::Owner;
    let digest = request_digest(&json!({
        "kind": RECORD_COMMAND,
        "project": USER_PROJECT,
        "actor": actor.as_str(),
        "policy_version": 0,
        "payload": event.payload,
        "scope": [],
    }))
    .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
    models::create_user(store, at)?;
    let command = Command {
        project: ProjectId(USER_PROJECT.into()),
        kind: CommandKind(RECORD_COMMAND.into()),
        request_id,
        digest,
        scope: vec![],
        policy_version: 0,
        recorded_at: at.into(),
        actor,
        caller: None,
    };
    let mut appended = false;
    store.transact(&command, &mut |tx| {
        let key = install_key(facts.host.name());
        let stored = tx.get(INSTALL_VIEW, &key)?;
        appended =
            stored.as_ref().map(|document| &document.body["payload"]) != Some(&event.payload);
        // The install lock keeps two runs from overlapping, and this stays as a
        // second guard. The facts carry hashes from the record this run read, so
        // if another run recorded since, appending them would roll its hashes back.
        let now = stored.as_ref().map(|document| document.produced_seq);
        if appended && now != seen {
            return Err(StoreError::Stale(StaleInput::Document {
                view: INSTALL_VIEW.into(),
                key,
                seen,
                now,
            }));
        }
        if appended {
            tx.append(event.clone())?;
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({"recorded": appended}),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    })?;
    Ok(appended)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_artifacts::stubs;
    use crate::install::event::{Facts, Hook, Registration, Sandbox, Stub, Updates};
    use crate::ledger::open::{self, tests::Fixed};
    use baley_core::policy::Host;
    use baley_store::{Actor, Event, Ledger, PageRequest, ProjectId, RequestId, StreamName};
    use baley_store_sqlite::{Options, SqliteStore};
    use serde_json::{Value, json};
    use std::sync::Arc;

    const AT: &str = "2026-10-10T09:00:00Z";

    fn store(dir: &tempfile::TempDir) -> SqliteStore {
        open::store(
            &dir.path().join("home"),
            AT,
            Options {
                timing: Arc::new(Fixed),
                ..open::options()
            },
        )
        .unwrap()
    }

    fn request(n: u8) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-{n:012x}"))
    }

    fn facts() -> Facts {
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        Facts {
            host: Host::ClaudeCode,
            binary_version: env!("CARGO_PKG_VERSION").into(),
            binary_path: "/home/o/.local/bin/baley".into(),
            complete: true,
            registration: Some(Registration {
                path: "/home/o/.claude.json".into(),
                entry: json!({"command": "/home/o/.local/bin/baley", "args": ["serve"]}),
            }),
            hook: Some(Hook {
                path: "/home/o/.claude/settings.json".into(),
                item: json!({
                    "matcher": "Write|Edit",
                    "hooks": [{"type": "command", "command": "/home/o/.local/bin/baley guard", "timeout": 5}],
                }),
            }),
            stubs: manifest
                .into_iter()
                .map(|entry| Stub {
                    path: format!("/home/o/.claude/skills/{}/SKILL.md", entry.identity),
                    identity: entry.identity,
                    sha256: entry.digest,
                })
                .collect(),
            sandbox: Some(Sandbox {
                settings_path: "/home/o/.claude/settings.json".into(),
                sha256: "c".repeat(64),
                home: "/home/o/.local/share/crenshawdev/baley".into(),
                config: "/home/o/.config/crenshawdev/baley".into(),
                write_only_files: vec!["/home/o/.local/bin/baley".into()],
                write_only_folders: vec!["/home/o/.local/lib/crenshawdev/baley/versions".into()],
                permissions_deny: vec!["Edit(//home/o/.local/bin/baley)".into()],
                deny_read: vec!["/home/o/.local/share/crenshawdev/baley".into()],
                deny_write: vec!["/home/o/.local/bin/baley".into()],
                held_back: None,
            }),
            updates: Updates {
                auto: Some(false),
                staged_version: Some("0.2.0".into()),
            },
        }
    }

    fn expected(capture_hash: &str, help_hash: &str) -> Value {
        json!({
            "host": "claude-code",
            "binary_version": env!("CARGO_PKG_VERSION"),
            "binary_path": "/home/o/.local/bin/baley",
            "complete": true,
            "registered": {
                "registration": {
                    "path": "/home/o/.claude.json",
                    "entry": {"command": "/home/o/.local/bin/baley", "args": ["serve"]},
                },
                "hook": {
                    "path": "/home/o/.claude/settings.json",
                    "item": {
                        "matcher": "Write|Edit",
                        "hooks": [{"type": "command", "command": "/home/o/.local/bin/baley guard", "timeout": 5}],
                    },
                },
            },
            "stubs": [
                {"identity": "bal-capture", "path": "/home/o/.claude/skills/bal-capture/SKILL.md", "sha256": capture_hash},
                {"identity": "bal-help", "path": "/home/o/.claude/skills/bal-help/SKILL.md", "sha256": help_hash},
            ],
            "sandbox": {
                "settings_path": "/home/o/.claude/settings.json",
                "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "home": "/home/o/.local/share/crenshawdev/baley",
                "config": "/home/o/.config/crenshawdev/baley",
                "write_only_files": ["/home/o/.local/bin/baley"],
                "write_only_folders": ["/home/o/.local/lib/crenshawdev/baley/versions"],
                "permissions_deny": ["Edit(//home/o/.local/bin/baley)"],
                "deny_read": ["/home/o/.local/share/crenshawdev/baley"],
                "deny_write": ["/home/o/.local/bin/baley"],
                "held_back": null,
            },
            "defaults": {},
            "updates": {"auto": false, "staged_version": "0.2.0"},
        })
    }

    /// Plans from the stored record the way a sequential run does, then records.
    fn write_from_latest(
        store: &SqliteStore,
        facts: &Facts,
        request_id: RequestId,
        at: &str,
    ) -> Result<bool, StoreError> {
        let seen = latest(store, facts.host.name())?.map(|latest| latest.seq);
        write(store, facts, seen, request_id, at)
    }

    fn events(store: &impl Ledger) -> Vec<Event> {
        store
            .stream(
                &ProjectId("user".into()),
                &StreamName("install".into()),
                0,
                PageRequest {
                    limit: 100,
                    after: None,
                },
            )
            .unwrap()
            .items
    }

    #[test]
    fn an_install_record_missing_an_artifact_or_off_the_install_stream_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let facts = facts();
        let expected = expected(&facts.stubs[0].sha256, &facts.stubs[1].sha256);

        assert_eq!(write_from_latest(&store, &facts, request(1), AT), Ok(true));

        let events = events(&store);
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.project_id, ProjectId("user".into()));
        assert_eq!(event.stream, "install");
        assert_eq!(event.type_name, "install.recorded");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.actor, Actor::Owner);
        assert_eq!(event.caller, None);
        assert_eq!(event.policy_version, 0);
        assert_eq!(event.recorded_at, AT);
        assert_eq!(event.request_id, request(1));
        assert_eq!(event.payload, expected);
        assert_eq!(read(&store, "claude-code"), Ok(Some(expected)));
    }

    #[test]
    fn a_repeated_install_recorded_twice_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut facts = facts();
        let mut expected = expected(&facts.stubs[0].sha256, &facts.stubs[1].sha256);
        assert_eq!(write_from_latest(&store, &facts, request(1), AT), Ok(true));
        let head = store.head(&ProjectId("user".into())).unwrap();

        assert_eq!(
            write_from_latest(&store, &facts, request(2), "2026-10-10T09:05:00Z"),
            Ok(false)
        );
        assert_eq!(events(&store).len(), 1);
        assert_eq!(store.head(&ProjectId("user".into())).unwrap(), head);

        facts.stubs[1].sha256 = "d".repeat(64);
        expected["stubs"][1]["sha256"] = json!("d".repeat(64));
        assert_eq!(
            write_from_latest(&store, &facts, request(3), "2026-10-10T09:10:00Z"),
            Ok(true)
        );
        assert_eq!(events(&store).len(), 2);

        facts.hook.as_mut().unwrap().item["hooks"][0]["timeout"] = json!(10);
        expected["registered"]["hook"]["item"]["hooks"][0]["timeout"] = json!(10);
        assert_eq!(
            write_from_latest(&store, &facts, request(4), "2026-10-10T09:15:00Z"),
            Ok(true)
        );
        assert_eq!(events(&store).len(), 3);

        facts.complete = false;
        expected["complete"] = json!(false);
        assert_eq!(
            write_from_latest(&store, &facts, request(5), "2026-10-10T09:20:00Z"),
            Ok(true)
        );
        let events = events(&store);
        assert_eq!(events.len(), 4);
        assert_eq!(events[3].payload, expected);
        assert_eq!(read(&store, "claude-code"), Ok(Some(expected)));
    }

    #[test]
    fn a_stale_install_run_overwriting_a_newer_record_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut facts = facts();
        assert_eq!(write_from_latest(&store, &facts, request(1), AT), Ok(true));
        let planned_from = latest(&store, "claude-code").unwrap().unwrap();

        // A newer run records different stub hashes after both read the record above.
        let mut newer = facts.clone();
        newer.stubs[0].sha256 = "e".repeat(64);
        assert_eq!(
            write(
                &store,
                &newer,
                Some(planned_from.seq),
                request(2),
                "2026-10-10T09:05:00Z"
            ),
            Ok(true)
        );
        let newest = latest(&store, "claude-code").unwrap().unwrap();
        let head = store.head(&ProjectId("user".into())).unwrap();

        facts.stubs[1].sha256 = "d".repeat(64);
        let stale = write(
            &store,
            &facts,
            Some(planned_from.seq),
            request(3),
            "2026-10-10T09:10:00Z",
        );

        assert_eq!(
            stale,
            Err(StoreError::Stale(StaleInput::Document {
                view: "install".into(),
                key: install_key("claude-code"),
                seen: Some(planned_from.seq),
                now: Some(newest.seq),
            }))
        );
        assert_eq!(latest(&store, "claude-code"), Ok(Some(newest)));
        assert_eq!(events(&store).len(), 2);
        assert_eq!(store.head(&ProjectId("user".into())).unwrap(), head);
    }

    #[test]
    fn a_partial_install_recorded_as_complete_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut facts = facts();
        let mut expected = expected(&facts.stubs[0].sha256, &facts.stubs[1].sha256);
        facts.complete = false;
        facts.registration = None;
        facts.hook = None;
        facts.sandbox = None;
        expected["complete"] = json!(false);
        expected["registered"] = json!({"registration": null, "hook": null});
        expected["sandbox"] = Value::Null;

        assert_eq!(write_from_latest(&store, &facts, request(1), AT), Ok(true));

        let events = events(&store);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload, expected);
        assert_eq!(read(&store, "claude-code"), Ok(Some(expected)));
    }

    #[test]
    fn a_first_install_refused_for_a_missing_user_project_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let facts = facts();
        let expected = expected(&facts.stubs[0].sha256, &facts.stubs[1].sha256);

        assert_eq!(read(&store, "claude-code"), Ok(None));
        assert_eq!(write_from_latest(&store, &facts, request(1), AT), Ok(true));
        assert_eq!(read(&store, "claude-code"), Ok(Some(expected)));
    }
}
