//! Records changed installation ownership facts in `user` and reads the latest
//! payload for a host from the `install` view. Times and request ids are supplied.

use baley_core::catalog::USER_PROJECT;
use baley_store::{
    Actor, Admin, Command, CommandKind, Decision, Ledger, Observed, OutcomeKind, ProjectId,
    Refusal, RequestId, StoreError, Views, request_digest,
};
use serde_json::{Value, json};

use crate::models;

use super::event::{self, Facts};
use super::view::{INSTALL_VIEW, install_key};

/// The command kind used to record installation ownership facts.
pub const RECORD_COMMAND: &str = "install.record";

/// Reads the latest payload for `host`, or none when `user` or its record is absent.
pub fn read(store: &(impl Admin + Views), host: &str) -> Result<Option<Value>, StoreError> {
    let project = ProjectId(USER_PROJECT.into());
    if !store.projects()?.iter().any(|(known, _)| *known == project) {
        return Ok(None);
    }
    let stored = store.get(&project, INSTALL_VIEW, &install_key(host))?;
    Ok(stored.map(|document| document.body["payload"].clone()))
}

/// Records supplied facts as the owner, creating `user` when absent. The request
/// id must be fresh. Returns true only when this call appended `install.recorded`.
/// An unchanged record skips the command. The comparison runs again inside the
/// transaction, so a racing match records only `command.completed`.
pub fn write(
    store: &(impl Admin + Views + Ledger),
    facts: &Facts,
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
        let stored = tx.get(INSTALL_VIEW, &install_key(facts.host.name()))?;
        appended =
            stored.as_ref().map(|document| &document.body["payload"]) != Some(&event.payload);
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

        assert_eq!(write(&store, &facts, request(1), AT), Ok(true));

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
        assert_eq!(write(&store, &facts, request(1), AT), Ok(true));
        let head = store.head(&ProjectId("user".into())).unwrap();

        assert_eq!(
            write(&store, &facts, request(2), "2026-10-10T09:05:00Z"),
            Ok(false)
        );
        assert_eq!(events(&store).len(), 1);
        assert_eq!(store.head(&ProjectId("user".into())).unwrap(), head);

        facts.stubs[1].sha256 = "d".repeat(64);
        expected["stubs"][1]["sha256"] = json!("d".repeat(64));
        assert_eq!(
            write(&store, &facts, request(3), "2026-10-10T09:10:00Z"),
            Ok(true)
        );
        assert_eq!(events(&store).len(), 2);

        facts.hook.as_mut().unwrap().item["hooks"][0]["timeout"] = json!(10);
        expected["registered"]["hook"]["item"]["hooks"][0]["timeout"] = json!(10);
        assert_eq!(
            write(&store, &facts, request(4), "2026-10-10T09:15:00Z"),
            Ok(true)
        );
        assert_eq!(events(&store).len(), 3);

        facts.complete = false;
        expected["complete"] = json!(false);
        assert_eq!(
            write(&store, &facts, request(5), "2026-10-10T09:20:00Z"),
            Ok(true)
        );
        let events = events(&store);
        assert_eq!(events.len(), 4);
        assert_eq!(events[3].payload, expected);
        assert_eq!(read(&store, "claude-code"), Ok(Some(expected)));
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

        assert_eq!(write(&store, &facts, request(1), AT), Ok(true));

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
        assert_eq!(write(&store, &facts, request(1), AT), Ok(true));
        assert_eq!(read(&store, "claude-code"), Ok(Some(expected)));
    }
}
