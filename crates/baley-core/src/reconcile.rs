//! Pure anchor reconciliation over a claimed intent and a supplied remote observation.

use baley_store::{Anchor, Decision, Hash, Observed, OutcomeKind, Reconciliation, Resolution};
use serde_json::{Value, json};

/// The project head before the anchor claim was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorIntent {
    /// The sequence to anchor.
    pub seq: u64,
    /// The hash to anchor.
    pub head: Hash,
}

impl AnchorIntent {
    /// Encodes the intent in the claim event.
    pub fn to_value(&self) -> Value {
        json!({"seq": self.seq, "head": self.head.to_hex()})
    }
    /// Reads an intent from a claim event.
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            seq: value.get("seq")?.as_u64()?,
            head: Hash::from_hex(value.get("head")?.as_str()?)?,
        })
    }
}

/// What the caller observed at the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteTag {
    /// The tag exists. `None` means its annotation is not an anchor.
    Present(Option<Anchor>),
    /// The remote confirmed the tag is absent.
    Absent,
    /// The remote could not be read.
    Unreachable,
}

/// What the remote observation means for the claimed anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorFinding {
    /// The annotation agrees with the intent.
    Anchored,
    /// A tag exists but its annotation does not agree.
    Conflicting { remote: Option<Anchor> },
    /// The remote confirmed absence.
    NotPushed,
    /// No real state was established.
    Unknown,
}

/// Judges agreement using both the sequence and hash.
pub fn judge_anchor(intent: &AnchorIntent, remote: RemoteTag) -> AnchorFinding {
    match remote {
        RemoteTag::Present(Some(anchor))
            if anchor.seq == intent.seq && anchor.hash == intent.head =>
        {
            AnchorFinding::Anchored
        }
        RemoteTag::Present(remote) => AnchorFinding::Conflicting { remote },
        RemoteTag::Absent => AnchorFinding::NotPushed,
        RemoteTag::Unreachable => AnchorFinding::Unknown,
    }
}

/// Builds a recordable resolution from a finding and supplied check time.
/// An unknown state leaves the claim interrupted and records nothing.
pub fn anchor_reconciliation(
    intent: &AnchorIntent,
    finding: AnchorFinding,
    checked_at: &str,
) -> Option<Reconciliation> {
    let anchor = json!({"seq": intent.seq, "head": intent.head.to_hex()});
    let (finding, kind, answer) = match finding {
        AnchorFinding::Anchored => (
            json!({"remote": "tag", "anchor": anchor, "checked_at": checked_at}),
            OutcomeKind::Done,
            json!({"anchored": anchor}),
        ),
        AnchorFinding::Conflicting { remote } => {
            let remote =
                remote.map(|anchor| json!({"seq": anchor.seq, "head": anchor.hash.to_hex()}));
            (
                json!({"remote": "tag", "anchor": remote, "checked_at": checked_at}),
                OutcomeKind::Refused,
                json!({"conflicting": {"seq": intent.seq, "claimed": intent.head.to_hex(), "remote": remote}}),
            )
        }
        AnchorFinding::NotPushed => (
            json!({"remote": "absent", "checked_at": checked_at}),
            OutcomeKind::Refused,
            json!({"not_pushed": {"seq": intent.seq}}),
        ),
        AnchorFinding::Unknown => return None,
    };
    Some(Reconciliation {
        finding,
        resolution: Resolution::Resolved(Box::new(Decision {
            kind,
            answer,
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })),
        observed: Observed::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const AT: &str = "2026-09-25T18:01:00Z";
    fn intent() -> AnchorIntent {
        AnchorIntent {
            seq: 7,
            head: Hash([0x42; 32]),
        }
    }

    // Catches a matching tag reported as conflicting.
    #[test]
    fn matching_tag_resolves_as_done() {
        let intent = intent();
        let finding = judge_anchor(
            &intent,
            RemoteTag::Present(Some(Anchor {
                seq: 7,
                hash: intent.head,
            })),
        );
        let result = anchor_reconciliation(&intent, finding.clone(), AT).expect("resolution");
        assert_eq!(finding, AnchorFinding::Anchored);
        assert!(
            matches!(result.resolution, Resolution::Resolved(decision) if decision.kind == OutcomeKind::Done && decision.answer == json!({"anchored": {"seq": 7, "head": "42".repeat(32)}}))
        );
    }

    // Catches existence taken for agreement.
    #[test]
    fn differing_tag_resolves_as_refused() {
        let intent = intent();
        let remote = Anchor {
            seq: 7,
            hash: Hash([0x11; 32]),
        };
        let finding = judge_anchor(&intent, RemoteTag::Present(Some(remote.clone())));
        let result = anchor_reconciliation(&intent, finding.clone(), AT).expect("resolution");
        assert_eq!(
            finding,
            AnchorFinding::Conflicting {
                remote: Some(remote)
            }
        );
        assert!(
            matches!(result.resolution, Resolution::Resolved(decision) if decision.kind == OutcomeKind::Refused)
        );
    }

    // Catches confirmed absence recorded as unknown.
    #[test]
    fn absent_tag_resolves_as_not_pushed() {
        let intent = intent();
        let finding = judge_anchor(&intent, RemoteTag::Absent);
        let result = anchor_reconciliation(&intent, finding.clone(), AT).expect("resolution");
        assert_eq!(finding, AnchorFinding::NotPushed);
        assert_eq!(
            result.finding,
            json!({"remote": "absent", "checked_at": AT})
        );
        assert!(
            matches!(result.resolution, Resolution::Resolved(decision) if decision.kind == OutcomeKind::Refused)
        );
    }

    // Catches an unreachable remote taken for absence.
    #[test]
    fn unreachable_tag_stays_unknown() {
        let intent = intent();
        let finding = judge_anchor(&intent, RemoteTag::Unreachable);
        assert_eq!(finding, AnchorFinding::Unknown);
        assert_eq!(anchor_reconciliation(&intent, finding, AT), None);
    }

    // Catches an intent that cannot be read from a replayed claim.
    #[test]
    fn intent_round_trips() {
        let intent = intent();
        assert_eq!(AnchorIntent::from_value(&intent.to_value()), Some(intent));
    }
}
