//! The `guard` view and the redelivery judge (design 0010, GRD-R9 and
//! GRD-R10): one confirmed answer per host, native session and call id, and
//! what a call arriving under a recorded call id gets.

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Value, json};

use super::answer::Answer;
use super::event::{ASK, DENY, GUARD_ANSWERED, PASS_ON_FAILURE};

/// The view's name.
pub const GUARD_VIEW: &str = "guard";

/// The first key field: the host's name.
const HOST_FIELD: &str = "host";
/// The second key field: the native session, `""` when the call had none.
/// It cannot be `session`, which the payload holds as `null` when absent.
const SESSION_KEY_FIELD: &str = "session_key";
/// The third key field: the host's id for the call.
const CALL_FIELD: &str = "call";

/// The view's declaration: the host, native session and call id as text
/// keys, no indexes, and page bound 1, since every read is a `get` by key.
pub fn guard_spec() -> ViewSpec {
    let text = |name: &str| FieldSpec {
        name: name.into(),
        kind: FieldKind::Text,
    };
    ViewSpec {
        name: GUARD_VIEW.into(),
        version: 1,
        key: vec![text(HOST_FIELD), text(SESSION_KEY_FIELD), text(CALL_FIELD)],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// The key of one call's answer. A call with no native session keys under
/// `""`, which no session can be, since a hook caller refuses empty text.
pub fn guard_key(host: &str, session: Option<&str>, call: &str) -> DocKey {
    DocKey(vec![
        KeyValue::Text(host.into()),
        KeyValue::Text(session.unwrap_or_default().into()),
        KeyValue::Text(call.into()),
    ])
}

/// Keeps the `guard` view current from `guard.answered`: the latest event
/// for a key replaces its document whole.
///
/// A document holds the key, the input digest, the project directory or
/// `null`, the cwd, the outcome, the reason and the event's sequence. A call
/// id has one record because the recording transaction reads the view
/// before it appends, not because this projector merges.
pub struct GuardProjector {
    spec: ViewSpec,
}

impl GuardProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self { spec: guard_spec() }
    }
}

impl Default for GuardProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for GuardProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[GUARD_ANSWERED]
    }

    // The latest event wins, so no stored document is needed.
    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        Vec::new()
    }

    fn apply(
        &self,
        event: &Event,
        _documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: &str| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let Value::Object(payload) = &event.payload else {
            return Err(refuse("the payload is not an object"));
        };
        let text = |member: &str| match payload.get(member) {
            Some(Value::String(value)) if !value.is_empty() => Some(value.as_str()),
            _ => None,
        };
        let (Some(host), Some(call), Some(digest), Some(cwd)) = (
            text(HOST_FIELD),
            text(CALL_FIELD),
            text("input_digest"),
            text("cwd"),
        ) else {
            return Err(refuse(
                "host, call, input_digest or cwd is missing, not text or empty",
            ));
        };
        // A `""` session would take the key of a call with none.
        let session = match payload.get("session") {
            Some(Value::Null) => None,
            Some(Value::String(session)) if !session.is_empty() => Some(session.as_str()),
            _ => return Err(refuse("session is neither null nor non-empty text")),
        };
        let directory = match payload.get("project_directory") {
            Some(Value::Null) => Value::Null,
            Some(Value::String(directory)) if !directory.is_empty() => json!(directory),
            _ => {
                return Err(refuse(
                    "project_directory is neither null nor non-empty text",
                ));
            }
        };
        let outcome = match payload.get("outcome").and_then(Value::as_str) {
            Some(outcome @ (ASK | DENY | PASS_ON_FAILURE)) => outcome,
            _ => return Err(refuse("outcome is not ask, deny or pass-on-failure")),
        };
        let Some(Value::String(reason)) = payload.get("reason") else {
            return Err(refuse("reason is missing or not text"));
        };
        Ok(vec![Change::Put {
            key: guard_key(host, session, call),
            body: json!({
                HOST_FIELD: host,
                SESSION_KEY_FIELD: session.unwrap_or_default(),
                CALL_FIELD: call,
                "input_digest": digest,
                "project_directory": directory,
                "cwd": cwd,
                "outcome": outcome,
                "reason": reason,
                "seq": event.seq,
            }),
        }])
    }
}

/// What a call gets from the answer recorded under its call id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redelivery {
    /// Nothing is recorded under the call id. The call is judged fresh.
    NoRecord,
    /// The same call was answered before. This is that answer.
    Replay(Answer),
    /// The call id was answered for another input, project directory or
    /// cwd. The call is judged on its own and is unrecordable.
    Clash,
}

/// Judges a call against `stored`, the `guard` document under its key, if
/// any, from the call's input digest, project directory and cwd.
///
/// A replay is the confirmed answer even after the policy changed
/// (GRD-R10). A clash is judged on its own and never recorded, so one call
/// id never has two records. A document whose outcome or reason cannot be
/// read is a clash too: it cannot be replayed, and recording again would
/// give the call id a second record.
pub fn redelivery(
    stored: Option<&Value>,
    input_digest: &str,
    project_directory: Option<&str>,
    cwd: &str,
) -> Redelivery {
    let Some(document) = stored else {
        return Redelivery::NoRecord;
    };
    let text = |member: &str| document.get(member).and_then(Value::as_str);
    let directory = match document.get("project_directory") {
        Some(Value::Null) => Some(None),
        Some(Value::String(directory)) => Some(Some(directory.as_str())),
        _ => None,
    };
    if text("input_digest") != Some(input_digest)
        || directory != Some(project_directory)
        || text("cwd") != Some(cwd)
    {
        return Redelivery::Clash;
    }
    let Some(reason) = text("reason") else {
        return Redelivery::Clash;
    };
    match text("outcome") {
        Some(ASK) => Redelivery::Replay(Answer::Ask(reason.into())),
        Some(DENY) => Redelivery::Replay(Answer::Deny(reason.into())),
        Some(PASS_ON_FAILURE) => Redelivery::Replay(Answer::PassOnFailure(reason.into())),
        _ => Redelivery::Clash,
    }
}
