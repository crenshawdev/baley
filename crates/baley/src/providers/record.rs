//! Recording the provider acknowledgement unless a current one covers it
//! (design 0003 section 6, CFG-R28).
//!
//! The reads and the append are gathering over the store. Whether a stored
//! acknowledgement covers the providers now confirmed, and how a store
//! failure is worded, are judged on plain values.

use std::collections::BTreeSet;

use baley_core::catalog::{Provider, USER_PROJECT};
use baley_store::{
    Actor, Admin, Command, CommandKind, Decision, Ledger, NewEvent, Observed, OutcomeKind,
    PageRequest, ProjectId, Refusal, RequestId, StoreError, StreamName, request_digest,
};
use serde_json::{Value, json};

use super::event::{
    PROVIDERS_ACKNOWLEDGED, PROVIDERS_ACKNOWLEDGED_VERSION, PROVIDERS_STREAM, acknowledged_event,
};
use super::warning::{WARNING_VERSION, warning_sha256};
use crate::envelope::LEDGER_BUSY;
use crate::ledger::display::Render;
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::models;

/// The command kind that records an acknowledgement.
pub(crate) const ACKNOWLEDGE_COMMAND: &str = "providers.acknowledge";

/// Events asked for per page when the stream is read.
const PAGE: u32 = 100;

/// Every acknowledgement already recorded, and the version of the stream
/// that holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stored {
    /// The stream's version when it was read, 0 when it holds nothing.
    pub(crate) version: u64,
    /// Each stored `providers.acknowledged` payload, oldest first.
    pub(crate) payloads: Vec<Value>,
}

fn user() -> ProjectId {
    ProjectId(USER_PROJECT.into())
}

/// Reads the acknowledgements stored in `user`. The project list is read
/// first, so a ledger without `user` gives none and creates nothing.
pub(crate) fn read(store: &(impl Admin + Ledger)) -> Result<Stored, StoreError> {
    let mut stored = Stored {
        version: 0,
        payloads: Vec::new(),
    };
    if !store.projects()?.iter().any(|(known, _)| *known == user()) {
        return Ok(stored);
    }
    let stream = StreamName(PROVIDERS_STREAM.into());
    let mut after = None;
    loop {
        let page = store.stream(&user(), &stream, 1, PageRequest { limit: PAGE, after })?;
        for event in page.items {
            stored.version = stored.version.max(event.stream_version);
            if event.type_name == PROVIDERS_ACKNOWLEDGED
                && event.type_version == PROVIDERS_ACKNOWLEDGED_VERSION
            {
                stored.payloads.push(event.payload);
            }
        }
        match page.next {
            Some(cursor) => after = Some(cursor),
            None => return Ok(stored),
        }
    }
}

/// The providers one stored payload covers at the compiled warning: none
/// unless it carries the current version and digest and lists its providers
/// as text. Its owner and time are history and decide nothing.
fn covered_by<'a>(payload: &'a Value, digest: &str) -> Vec<&'a str> {
    let (Some(version), Some(sha), Some(listed)) = (
        payload["warning_version"].as_u64(),
        payload["warning_sha256"].as_str(),
        payload["providers"].as_array(),
    ) else {
        return Vec::new();
    };
    if version != u64::from(WARNING_VERSION) || sha != digest {
        return Vec::new();
    }
    listed
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}

/// Judges whether a stored acknowledgement covers every one of `providers`
/// at the compiled warning. The version and the digest must both match, so a
/// reworded warning asks again whether or not its version was raised. Several
/// payloads together may cover the providers. When they do not, the one event
/// to record, given back, lists every confirmed provider; none means a stored
/// acknowledgement already covers them.
pub(crate) fn judge(
    stored: &[Value],
    providers: &[Provider],
    owner: &str,
    at: &str,
) -> Option<NewEvent> {
    let digest = warning_sha256();
    let covered: BTreeSet<&str> = stored
        .iter()
        .flat_map(|payload| covered_by(payload, &digest))
        .collect();
    if providers
        .iter()
        .all(|provider| covered.contains(provider.name()))
    {
        None
    } else {
        Some(acknowledged_event(providers, owner, at))
    }
}

/// Appends `event` to the `providers` stream of `user`, bound to the stream
/// version `seen`, so an acknowledgement recorded since the read is refused as
/// stale instead of duplicated. Creates `user` when it is absent.
pub(crate) fn append(
    store: &(impl Admin + Ledger),
    event: NewEvent,
    seen: u64,
    request_id: RequestId,
    at: &str,
) -> Result<(), StoreError> {
    let actor = Actor::Owner;
    let digest = request_digest(&json!({
        "kind": ACKNOWLEDGE_COMMAND,
        "project": USER_PROJECT,
        "actor": actor.as_str(),
        "policy_version": 0,
        "payload": event.payload,
    }))
    .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
    models::create_user(store, at)?;
    let command = Command {
        project: user(),
        kind: CommandKind(ACKNOWLEDGE_COMMAND.into()),
        request_id,
        digest,
        scope: vec![],
        policy_version: 0,
        recorded_at: at.into(),
        actor,
        caller: None,
    };
    store.transact(&command, &mut |tx| {
        tx.expect(&event.stream, seen)?;
        tx.append(event.clone())?;
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({"recorded": true}),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    })?;
    Ok(())
}

/// The owner's line for a failed read or append: `ledger-busy` for a busy
/// store and `ledger-unavailable` for every other failure, then the store's
/// own text. It is a write failure, exit 3.
pub(crate) fn failure(error: &StoreError) -> Render {
    let code = match error {
        StoreError::Busy => LEDGER_BUSY,
        _ => LEDGER_UNAVAILABLE,
    };
    Render {
        lines: vec![format!("{code}: {error}")],
        code: 3,
        error: true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use baley_store::{Actor, StaleInput};
    use baley_store_sqlite::{Options, SqliteStore};
    use serde_json::json;

    use super::*;
    use crate::ledger::open::{self, tests::Fixed};
    use crate::providers::fixtures::{DESIGN_WARNING, sha256_hex};

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

    fn stored_payload(providers: &[&str], version: u64, digest: &str) -> Value {
        json!({
            "providers": providers,
            "warning_version": version,
            "warning_sha256": digest,
            "owner": "Ada Owner",
            "acknowledged_at": "2026-10-01T09:00:00Z",
        })
    }

    fn only_event(need: Option<NewEvent>) -> NewEvent {
        need.expect("an acknowledgement was reused")
    }

    #[test]
    fn an_acknowledgement_of_an_older_warning_reused_is_caught() {
        let current = sha256_hex(DESIGN_WARNING);
        let expected = json!({
            "providers": ["openai"],
            "warning_version": 1,
            "warning_sha256": current,
            "owner": "Ada Owner",
            "acknowledged_at": AT,
        });
        for stored in [
            stored_payload(&["openai"], 0, &sha256_hex("other text")),
            stored_payload(&["openai"], 0, &current),
        ] {
            let need = judge(&[stored], &[Provider::OpenAi], "Ada Owner", AT);
            assert_eq!(only_event(need).payload, expected);
        }
    }

    #[test]
    fn an_acknowledgement_missing_its_warning_owner_or_time_or_off_the_providers_stream_is_caught()
    {
        let dir = tempfile::tempdir().unwrap();
        let ledger = store(&dir);
        let seen = read(&ledger).unwrap();
        let event = only_event(judge(&seen.payloads, &[Provider::OpenAi], "Ada Owner", AT));
        append(&ledger, event, seen.version, request(1), AT).unwrap();

        let user = ProjectId("user".into());
        assert!(
            ledger
                .projects()
                .unwrap()
                .iter()
                .any(|(known, _)| *known == user)
        );
        let page = |stream: &str| {
            ledger
                .stream(
                    &user,
                    &StreamName(stream.into()),
                    1,
                    PageRequest {
                        limit: 100,
                        after: None,
                    },
                )
                .unwrap()
        };
        let providers = page("providers");
        assert_eq!(providers.items.len(), 1);
        let recorded = &providers.items[0];
        assert_eq!(recorded.type_name, "providers.acknowledged");
        assert_eq!(recorded.type_version, 1);
        assert_eq!(recorded.actor, Actor::Owner);
        assert_eq!(recorded.policy_version, 0);
        assert_eq!(recorded.caller, None);
        assert_eq!(recorded.recorded_at, AT);
        assert_eq!(
            recorded.payload,
            json!({
                "providers": ["openai"],
                "warning_version": 1,
                "warning_sha256": sha256_hex(DESIGN_WARNING),
                "owner": "Ada Owner",
                "acknowledged_at": AT,
            })
        );
        assert!(page("install").items.is_empty());
    }

    #[test]
    fn a_current_acknowledgement_recorded_again_is_caught() {
        let current = sha256_hex(DESIGN_WARNING);
        let openai = stored_payload(&["openai"], 1, &current);
        let deepseek = stored_payload(&["deepseek"], 1, &current);

        let alone = judge(
            std::slice::from_ref(&openai),
            &[Provider::OpenAi],
            "Ada Owner",
            AT,
        );
        assert_eq!(alone, None);

        let widened = judge(
            std::slice::from_ref(&openai),
            &[Provider::OpenAi, Provider::DeepSeek],
            "Ada Owner",
            AT,
        );
        assert_eq!(
            only_event(widened).payload["providers"],
            json!(["openai", "deepseek"])
        );

        let together = judge(
            &[openai, deepseek],
            &[Provider::OpenAi, Provider::DeepSeek],
            "Ada Owner",
            AT,
        );
        assert_eq!(together, None);
    }

    #[test]
    fn an_acknowledgement_of_changed_warning_text_reused_is_caught() {
        let stored = stored_payload(&["openai"], 1, &sha256_hex("other text"));
        let need = judge(&[stored], &[Provider::OpenAi], "Ada Owner", AT);
        assert_eq!(
            only_event(need).payload,
            json!({
                "providers": ["openai"],
                "warning_version": 1,
                "warning_sha256": sha256_hex(DESIGN_WARNING),
                "owner": "Ada Owner",
                "acknowledged_at": AT,
            })
        );
    }

    #[test]
    fn a_store_failure_reported_under_the_wrong_ledger_code_is_caught() {
        let stale = StoreError::Stale(StaleInput::StreamVersion {
            stream: StreamName("providers".into()),
            expected: 0,
            actual: 1,
        });
        for (error, code, text) in [
            (StoreError::Busy, "ledger-busy: ", None),
            (
                StoreError::Unavailable("disk I/O error".into()),
                "ledger-unavailable: ",
                Some("disk I/O error"),
            ),
            (stale, "ledger-unavailable: ", None),
        ] {
            let render = failure(&error);
            assert_eq!(render.lines.len(), 1, "{error}");
            assert!(render.lines[0].starts_with(code), "{}", render.lines[0]);
            if let Some(text) = text {
                assert!(render.lines[0].ends_with(text), "{}", render.lines[0]);
            }
            assert_eq!(render.code, 3, "{error}");
            assert!(render.error, "{error}");
        }
    }

    #[test]
    fn an_acknowledgement_read_creating_user_or_failing_on_a_fresh_ledger_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = store(&dir);
        let seen = read(&ledger).unwrap();
        assert_eq!(seen.version, 0);
        assert!(seen.payloads.is_empty());
        assert!(
            !ledger
                .projects()
                .unwrap()
                .iter()
                .any(|(known, _)| known.0 == "user")
        );
    }

    #[test]
    fn an_acknowledgement_appended_over_one_recorded_since_the_read_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = store(&dir);
        let event = |provider| acknowledged_event(&[provider], "Ada Owner", AT);
        let held = |ledger: &SqliteStore| read(ledger).unwrap().payloads.len();

        assert_eq!(read(&ledger).unwrap().version, 0);
        append(&ledger, event(Provider::OpenAi), 0, request(1), AT).unwrap();
        let stale = append(&ledger, event(Provider::DeepSeek), 0, request(2), AT).unwrap_err();
        assert!(
            matches!(
                stale,
                StoreError::Stale(StaleInput::StreamVersion {
                    expected: 0,
                    actual: 1,
                    ..
                })
            ),
            "{stale:?}"
        );
        assert_eq!(held(&ledger), 1);

        assert_eq!(read(&ledger).unwrap().version, 1);
        append(&ledger, event(Provider::DeepSeek), 1, request(3), AT).unwrap();
        assert_eq!(held(&ledger), 2);
        let again = append(&ledger, event(Provider::OpenAi), 1, request(4), AT).unwrap_err();
        assert!(matches!(again, StoreError::Stale(_)), "{again:?}");
        assert_eq!(held(&ledger), 2);
    }
}
