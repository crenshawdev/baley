//! `command.completed` and the `request` view (design 0001, Commands;
//! EVD-R6).
//!
//! The store records `command.completed` itself, at the end of every command
//! that reaches an outcome, so the event's shape and the view built from it
//! belong to the port, not to the core. Every adapter registers this
//! projector beside the core's and reads the view before any decision runs.

use serde_json::{Value, json};

use crate::command::{Answer, Command, CommandKind, Outcome, OutcomeKind, StreamName};
use crate::event::{Event, Hash};
use crate::payload::PayloadRef;
use crate::view::{
    Change, DocKey, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};

pub const COMMAND_COMPLETED: &str = "command.completed";
pub const COMMAND_COMPLETED_VERSION: u32 = 1;
pub const REQUEST_VIEW: &str = "request";

/// An answer whose canonical JSON is longer than this is stored as a
/// `record` payload and `command.completed` carries its reference.
pub const INLINE_ANSWER_LIMIT: usize = 4096;

/// The stream a command kind's `command.*` events go to.
pub fn command_stream(kind: &CommandKind) -> StreamName {
    StreamName(format!("command/{}", kind.0))
}

/// Whether the store, not a decision, records events of this type.
pub fn store_owned(type_name: &str) -> bool {
    type_name.starts_with("command.") || type_name.starts_with("payload.")
}

/// The `request` document key of a command.
pub fn request_key(command: &Command) -> DocKey {
    DocKey(vec![
        KeyValue::Text(command.kind.0.clone()),
        KeyValue::Text(command.request_id.0.clone()),
    ])
}

/// The payload of `command.completed`: the command kind and request id the
/// view is keyed by, the digest a retry is compared with, and the outcome.
/// A stored answer is its reference object, so the store records the
/// reference like any other attachment.
pub fn completed_payload(command: &Command, outcome: &Outcome) -> Value {
    let answer = match &outcome.answer {
        Answer::Inline(value) => json!({ "inline": value }),
        Answer::Stored(reference) | Answer::Tombstone { reference, .. } => {
            json!({ "stored": reference.to_value() })
        }
    };
    json!({
        "kind": command.kind.0,
        "request_id": command.request_id.0,
        "digest": command.digest.to_hex(),
        "outcome": match outcome.kind {
            OutcomeKind::Done => "done",
            OutcomeKind::Refused => "refused",
        },
        "answer": answer,
    })
}

/// The digest and outcome a `request` document records, or `None` if the
/// body is not one.
pub fn recorded_outcome(body: &Value) -> Option<(Hash, Outcome)> {
    let digest = Hash::from_hex(body.get("digest")?.as_str()?)?;
    let kind = match body.get("outcome")?.as_str()? {
        "done" => OutcomeKind::Done,
        "refused" => OutcomeKind::Refused,
        _ => return None,
    };
    let answer = body.get("answer")?.as_object()?;
    if answer.len() != 1 {
        return None;
    }
    let answer = match (answer.get("inline"), answer.get("stored")) {
        (Some(value), None) => Answer::Inline(value.clone()),
        (None, Some(reference)) => Answer::Stored(PayloadRef::from_value(reference)?),
        _ => return None,
    };
    Some((digest, Outcome { kind, answer }))
}

/// The `request` view: one document per (command kind, request id), the
/// outcome a retry receives. No index: it is only ever read by key.
pub fn request_spec() -> ViewSpec {
    ViewSpec {
        name: REQUEST_VIEW.into(),
        version: 1,
        key: vec![
            FieldSpec {
                name: "kind".into(),
                kind: FieldKind::Text,
            },
            FieldSpec {
                name: "request_id".into(),
                kind: FieldKind::Text,
            },
        ],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// Keeps the `request` view: the document is the `command.completed`
/// payload.
pub struct RequestProjector {
    spec: ViewSpec,
}

impl RequestProjector {
    pub fn new() -> Self {
        Self {
            spec: request_spec(),
        }
    }
}

impl Default for RequestProjector {
    fn default() -> Self {
        Self::new()
    }
}

fn key_of(event: &Event) -> Option<DocKey> {
    Some(DocKey(vec![
        KeyValue::Text(event.payload.get("kind")?.as_str()?.to_owned()),
        KeyValue::Text(event.payload.get("request_id")?.as_str()?.to_owned()),
    ]))
}

impl Projector for RequestProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[COMMAND_COMPLETED]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        key_of(event).into_iter().collect()
    }

    fn apply(
        &self,
        event: &Event,
        _documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let key = key_of(event).ok_or_else(|| {
            ProjectorError("command.completed without its kind and request id".into())
        })?;
        Ok(vec![Change::Put {
            key,
            body: event.payload.clone(),
        }])
    }
}
