//! The anchor command's identity, its result event and the rules both sides
//! of the port read (design 0001, The hash chain and anchors; EVD-R3).
//!
//! The core runs the command and talks to the forge; the adapter stores the
//! anchor row and verifies against a fetched anchor. Both need the command
//! kind, the event shapes, the tag name and which events belong to the
//! command itself, so those live here.

use serde_json::{Value, json};

use crate::chain::Anchor;
use crate::claim::COMMAND_CLAIMED;
use crate::event::{Hash, ProjectId};
use crate::request::COMMAND_COMPLETED;

/// The command kind of an anchor push.
pub const ANCHOR_PUSH: &str = "anchor.push";
/// The command kind that reconciles an interrupted anchor push.
pub const ANCHOR_RECONCILE: &str = "anchor.reconcile";
/// The command kind that accepts a restored chain behind a remote anchor.
pub const ANCHOR_ACKNOWLEDGE_RESTORE: &str = "anchor.acknowledge_restore";
/// The scope token an anchor push holds.
pub const ANCHOR_SCOPE: &str = "anchor";
/// The stream the anchor result events go to.
pub const ANCHOR_STREAM: &str = "project";
/// The event that records a confirmed anchor tag.
pub const ANCHOR_PUSHED: &str = "anchor.pushed";
/// The current `anchor.pushed` payload version.
pub const ANCHOR_PUSHED_VERSION: u32 = 1;
/// The event that records an anchor that was not pushed.
pub const ANCHOR_FAILED: &str = "anchor.failed";
/// The current `anchor.failed` payload version.
pub const ANCHOR_FAILED_VERSION: u32 = 1;
/// The event that records an owner accepted restore gap.
pub const ANCHOR_RESTORE_ACKNOWLEDGED: &str = "anchor.restore_acknowledged";
/// The current restore acknowledgement payload version.
pub const ANCHOR_RESTORE_ACKNOWLEDGED_VERSION: u32 = 1;
/// Every anchor tag's name starts with this.
pub const ANCHOR_TAG_PREFIX: &str = "baley-anchor/";

/// The tag that anchors `seq` of `project`: `baley-anchor/<project_id>/<seq>`.
pub fn anchor_tag(project: &ProjectId, seq: u64) -> String {
    format!("{ANCHOR_TAG_PREFIX}{}/{seq}", project.0)
}

/// The `anchor.pushed` payload: the tag Baley confirmed on the remote, the
/// sequence and head hash it names, the configured remote's name, and when
/// Baley confirmed it, which is not necessarily when the forge created it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorPushedPayload {
    /// The tag on the remote.
    pub tag: String,
    /// The anchored sequence.
    pub seq: u64,
    /// The head hash at that sequence.
    pub head: Hash,
    /// The configured remote's name, never a URL.
    pub remote: String,
    /// The supplied UTC time of the confirmation.
    pub observed_at: String,
}

impl AnchorPushedPayload {
    /// Builds the event payload.
    pub fn to_value(&self) -> Value {
        json!({"tag": self.tag, "seq": self.seq, "head": self.head.to_hex(),
            "remote": self.remote, "observed_at": self.observed_at})
    }

    /// Reads the event payload: exactly its five fields, the head in lower
    /// case.
    pub fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.len() != 5 {
            return None;
        }
        let head = object.get("head")?.as_str()?;
        let hash = Hash::from_hex(head)?;
        if hash.to_hex() != head {
            return None;
        }
        Some(Self {
            tag: object.get("tag")?.as_str()?.into(),
            seq: object.get("seq")?.as_u64()?,
            head: hash,
            remote: object.get("remote")?.as_str()?.into(),
            observed_at: object.get("observed_at")?.as_str()?.into(),
        })
    }

    /// The anchor the tag names.
    pub fn anchor(&self) -> Anchor {
        Anchor {
            seq: self.seq,
            hash: self.head,
        }
    }
}

/// The remote anchor an owner accepted and the local head before the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreAcknowledgedPayload {
    /// Remote whose anchor was checked.
    pub remote: String,
    /// Project anchor tag name.
    pub tag: String,
    /// Sequence of the remote anchor.
    pub seq: u64,
    /// Hash of the remote anchor.
    pub head: Hash,
    /// Local sequence before this event.
    pub restored_seq: u64,
    /// Local hash before this event, absent for an empty chain.
    pub restored_head: Option<Hash>,
    /// Time of the remote check.
    pub checked_at: String,
}

impl RestoreAcknowledgedPayload {
    /// Encodes the exact event payload.
    pub fn to_value(&self) -> Value {
        json!({"remote": self.remote, "tag": self.tag, "seq": self.seq,
            "head": self.head.to_hex(), "restored_seq": self.restored_seq,
            "restored_head": self.restored_head.map(|hash| hash.to_hex()),
            "checked_at": self.checked_at})
    }

    /// Decodes only the exact event payload with lower case hashes.
    pub fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.len() != 7 {
            return None;
        }
        let head_text = object.get("head")?.as_str()?;
        let head = Hash::from_hex(head_text)?;
        if head.to_hex() != head_text {
            return None;
        }
        let restored_head = match object.get("restored_head")? {
            Value::Null => None,
            Value::String(text) => {
                let hash = Hash::from_hex(text)?;
                if hash.to_hex() != *text {
                    return None;
                }
                Some(hash)
            }
            _ => return None,
        };
        Some(Self {
            remote: object.get("remote")?.as_str()?.into(),
            tag: object.get("tag")?.as_str()?.into(),
            seq: object.get("seq")?.as_u64()?,
            head,
            restored_seq: object.get("restored_seq")?.as_u64()?,
            restored_head,
            checked_at: object.get("checked_at")?.as_str()?.into(),
        })
    }

    /// The remote anchor accepted by the owner.
    pub fn anchor(&self) -> Anchor {
        Anchor {
            seq: self.seq,
            hash: self.head,
        }
    }
}

/// Whether an event is one the anchor command itself records: its claim,
/// its result, a reconciliation of it, or a completion of it or of its
/// reconciler. These say nothing about the project's own work, so they do
/// not start the unanchored age that `doctor` warns on.
pub fn anchor_command_event(type_name: &str, payload: &Value) -> bool {
    let kind = || payload.get("kind").and_then(Value::as_str);
    match type_name {
        ANCHOR_PUSHED | ANCHOR_FAILED | ANCHOR_RESTORE_ACKNOWLEDGED => true,
        COMMAND_CLAIMED | crate::claim::COMMAND_RECONCILED => kind() == Some(ANCHOR_PUSH),
        COMMAND_COMPLETED => matches!(
            kind(),
            Some(ANCHOR_PUSH | ANCHOR_RECONCILE | ANCHOR_ACKNOWLEDGE_RESTORE)
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pushed() -> AnchorPushedPayload {
        AnchorPushedPayload {
            tag: "baley-anchor/p/7".into(),
            seq: 7,
            head: Hash([0xab; 32]),
            remote: "origin".into(),
            observed_at: "2026-09-25T18:00:00Z".into(),
        }
    }

    // The payload is exactly the five named fields, and reads back. Catches
    // a renamed or extra field the adapter's row check would not match.
    #[test]
    fn the_pushed_payload_has_exactly_its_five_fields() {
        let value = pushed().to_value();
        assert_eq!(
            value,
            json!({"tag": "baley-anchor/p/7", "seq": 7, "head": "ab".repeat(32),
                "remote": "origin", "observed_at": "2026-09-25T18:00:00Z"})
        );
        assert_eq!(AnchorPushedPayload::from_value(&value), Some(pushed()));
    }

    // An upper-case head or a sixth field is not the recorded shape. Catches
    // a reader looser than the writer.
    #[test]
    fn the_pushed_payload_refuses_other_shapes() {
        let mut upper = pushed().to_value();
        upper["head"] = json!("AB".repeat(32));
        assert_eq!(AnchorPushedPayload::from_value(&upper), None);
        let mut extra = pushed().to_value();
        extra["reason"] = json!("x");
        assert_eq!(AnchorPushedPayload::from_value(&extra), None);
    }

    // The tag names the project and the sequence. Catches another prefix or
    // separator than the design's.
    #[test]
    fn the_tag_is_the_designs_name() {
        assert_eq!(
            anchor_tag(&ProjectId("p-1".into()), 42),
            "baley-anchor/p-1/42"
        );
    }

    // The command's own claim, result, reconciliation and completions are
    // its events; another command's are not. Catches an age exclusion that
    // hides the project's own work, or one that misses the reconciler.
    #[test]
    fn only_the_anchor_commands_events_are_its_own() {
        let push = json!({"kind": ANCHOR_PUSH});
        let reconcile = json!({"kind": ANCHOR_RECONCILE});
        let other = json!({"kind": "plan.approve"});
        assert!(anchor_command_event(ANCHOR_PUSHED, &json!({})));
        assert!(anchor_command_event(ANCHOR_FAILED, &json!({})));
        assert!(anchor_command_event(COMMAND_CLAIMED, &push));
        assert!(anchor_command_event(
            crate::claim::COMMAND_RECONCILED,
            &push
        ));
        assert!(anchor_command_event(COMMAND_COMPLETED, &push));
        assert!(anchor_command_event(COMMAND_COMPLETED, &reconcile));
        assert!(!anchor_command_event(COMMAND_CLAIMED, &reconcile));
        assert!(!anchor_command_event(COMMAND_COMPLETED, &other));
        assert!(!anchor_command_event("phase.declared", &push));
    }
}
