//! `command.completed` and the `request` view (design 0001, Commands;
//! EVD-R6).
//!
//! The store records `command.completed` itself, at the end of every command
//! that reaches an outcome, so the event's shape and the view built from it
//! belong to the port, not to the core. Every adapter registers this
//! projector beside the core's and reads the view before any decision runs.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::canonical::{CanonicalError, canonical_json};
use crate::claim::{
    COMMAND_CLAIMED, COMMAND_RECONCILED, ClaimOwner, ClaimedPayload, ReconciledPayload,
    ReconciledResolution,
};
use crate::command::{Answer, Command, CommandKind, Outcome, OutcomeKind, StreamName};
use crate::event::{Event, Hash, RequestId};
use crate::payload::PayloadRef;
use crate::view::{
    Change, DocKey, FieldKind, FieldSpec, IndexField, IndexSpec, KeyValue, Order, Projector,
    ProjectorError, ViewSpec,
};

pub const COMMAND_COMPLETED: &str = "command.completed";
pub const COMMAND_COMPLETED_VERSION: u32 = 2;
pub const REQUEST_VIEW: &str = "request";

/// An answer whose canonical JSON is longer than this is stored as a
/// `record` payload and `command.completed` carries its reference.
pub const INLINE_ANSWER_LIMIT: usize = 4096;

/// A request digest: the SHA-256 of the canonical form of the command's
/// kind and every field that carries authority, as the caller builds it.
/// A value with no canonical form, such as one holding a float, has none.
pub fn request_digest(value: &Value) -> Result<Hash, CanonicalError> {
    Ok(Hash(Sha256::digest(canonical_json(value)?).into()))
}

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
    completed_payload_for(
        &command.kind,
        &command.request_id,
        &command.digest,
        &[],
        outcome,
    )
}

/// Builds a completion for the named request, including a closed claim's scope.
pub fn completed_payload_for(
    kind: &CommandKind,
    request_id: &RequestId,
    digest: &Hash,
    scope: &[String],
    outcome: &Outcome,
) -> Value {
    let answer = match &outcome.answer {
        Answer::Inline(value) => json!({ "inline": value }),
        Answer::Stored(reference) | Answer::Tombstone { reference, .. } => {
            json!({ "stored": reference.to_value() })
        }
    };
    json!({
        "kind": kind.0,
        "request_id": request_id.0,
        "digest": digest.to_hex(),
        "scope": scope,
        "outcome": match outcome.kind {
            OutcomeKind::Done => "done",
            OutcomeKind::Refused => "refused",
        },
        "answer": answer,
    })
}

/// The claimed fields kept in a request document, without lease liveness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimDoc {
    /// The claimed command's identity.
    pub id: crate::claim::ClaimId,
    /// The claimed command's digest.
    pub digest: Hash,
    /// The claim event's sequence.
    pub seq: u64,
    /// The claim event's recorded time.
    pub claimed_at: String,
    /// The tokens held by the claim.
    pub scope: Vec<String>,
    /// The process that acts.
    pub owner: ClaimOwner,
    /// The intended external effect.
    pub intent: Value,
}

/// The state held by a request document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestState {
    /// An open claim.
    Claimed(ClaimDoc),
    /// An open claim held for the owner at this reconciliation sequence.
    AwaitingOwner(ClaimDoc, u64),
    /// A completed request and its digest.
    Completed(Hash, Outcome),
}

/// Parses the current request document shape.
pub fn request_state(body: &Value) -> Option<RequestState> {
    match body.get("state")?.as_str()? {
        "completed" => {
            body.get("scope")?
                .as_array()?
                .iter()
                .map(|token| token.as_str())
                .collect::<Option<Vec<_>>>()?;
            let (digest, outcome) = recorded_outcome(body)?;
            Some(RequestState::Completed(digest, outcome))
        }
        "claimed" | "awaiting_owner" => {
            let claimed = body.get("claim")?;
            let owner = claimed.get("owner")?;
            let doc = ClaimDoc {
                id: crate::claim::ClaimId {
                    kind: CommandKind(body.get("kind")?.as_str()?.into()),
                    request_id: RequestId(body.get("request_id")?.as_str()?.into()),
                },
                digest: Hash::from_hex(body.get("digest")?.as_str()?)?,
                seq: claimed.get("seq")?.as_u64()?,
                claimed_at: claimed.get("claimed_at")?.as_str()?.into(),
                scope: claimed
                    .get("scope")?
                    .as_array()?
                    .iter()
                    .map(|token| token.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?,
                owner: ClaimOwner {
                    process: owner.get("process")?.as_str()?.into(),
                    host_session: owner.get("host_session")?.as_str()?.into(),
                    started_at: owner.get("started_at")?.as_str()?.into(),
                },
                intent: claimed.get("intent")?.clone(),
            };
            if body.get("state")?.as_str()? == "claimed" {
                Some(RequestState::Claimed(doc))
            } else {
                let held = body.get("held")?;
                held.get("finding")?;
                Some(RequestState::AwaitingOwner(doc, held.get("seq")?.as_u64()?))
            }
        }
        _ => None,
    }
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
/// outcome or open claim a retry receives, indexed by state for open claims.
pub fn request_spec() -> ViewSpec {
    ViewSpec {
        name: REQUEST_VIEW.into(),
        version: 2,
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
        indexes: vec![IndexSpec {
            name: "by_state".into(),
            fields: vec![IndexField {
                name: "state".into(),
                kind: FieldKind::Text,
                order: Order::Ascending,
            }],
        }],
        page_bound: 100,
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
        &[COMMAND_CLAIMED, COMMAND_COMPLETED, COMMAND_RECONCILED]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        key_of(event).into_iter().collect()
    }

    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let key = key_of(event).ok_or_else(|| {
            ProjectorError("a command event without its kind and request id".into())
        })?;
        let body = match event.type_name.as_str() {
            COMMAND_CLAIMED => {
                let payload = ClaimedPayload::from_value(&event.payload)
                    .ok_or_else(|| ProjectorError("a malformed command.claimed payload".into()))?;
                json!({"kind": payload.kind.0, "request_id": payload.request_id.0, "digest": payload.digest.to_hex(),
                    "state": "claimed", "claim": {"seq": event.seq, "claimed_at": event.recorded_at,
                    "scope": payload.scope, "owner": {"process": payload.owner.process, "host_session": payload.owner.host_session, "started_at": payload.owner.started_at}, "intent": payload.intent}})
            }
            COMMAND_RECONCILED => {
                let payload = ReconciledPayload::from_value(&event.payload).ok_or_else(|| {
                    ProjectorError("a malformed command.reconciled payload".into())
                })?;
                if payload.resolution == ReconciledResolution::Resolved {
                    return Ok(Vec::new());
                }
                let mut body = documents
                    .iter()
                    .find(|(found, _)| found == &key)
                    .map(|(_, body)| body.clone())
                    .ok_or_else(|| {
                        ProjectorError("command.reconciled without an open claim".into())
                    })?;
                match request_state(&body) {
                    Some(RequestState::Claimed(doc) | RequestState::AwaitingOwner(doc, _))
                        if doc.seq == payload.claim_seq => {}
                    _ => {
                        return Err(ProjectorError(
                            "command.reconciled without its claim".into(),
                        ));
                    }
                }
                body["state"] = json!("awaiting_owner");
                body["held"] = json!({"seq": event.seq, "finding": payload.finding});
                body
            }
            COMMAND_COMPLETED => {
                let mut body = event.payload.clone();
                body["state"] = json!("completed");
                body
            }
            _ => return Ok(Vec::new()),
        };
        Ok(vec![Change::Put { key, body }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The digest is the SHA-256 of the canonical bytes, keys in UTF-16
    // order: U+10000 before U+E000, where a byte sort puts them the other
    // way. The vector is `printf '{"\xf0\x90\x80\x80":2,"\xee\x80\x80":1}' |
    // sha256sum`. Catches a digest over serializer output, which another
    // implementation of the same request could not reproduce.
    #[test]
    fn the_digest_hashes_the_canonical_bytes() {
        let digest = request_digest(&json!({"\u{e000}": 1, "\u{10000}": 2})).expect("digest");
        assert_eq!(
            digest.to_hex(),
            "9d4cdc71dda603c42f9b21d88d0c2ffc31a76cd1bd461d7359406cf169845f1e"
        );
    }
}
