//! Owner acknowledgement of a restored chain behind the remote anchor.

use std::fmt;

use baley_store::{
    ANCHOR_ACKNOWLEDGE_RESTORE, ANCHOR_RESTORE_ACKNOWLEDGED, ANCHOR_RESTORE_ACKNOWLEDGED_VERSION,
    ANCHOR_SCOPE, ANCHOR_STREAM, Actor, Anchor, AnchorCheck, AnchorVerdict, Command, CommandKind,
    Decision, Hash, Ledger, NewEvent, Observed, OutcomeKind, ProjectId, Recorded, Refusal,
    RequestId, RestoreAcknowledgedPayload, StaleInput, StoreError, StreamName, VerifyReport,
    anchor_tag, request_digest,
};
use serde_json::json;

use crate::anchor::anchor_status;
use crate::forge::{Forge, TagQuery};

/// An owner's request to accept a restored gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcknowledgeRestore {
    /// Project whose chain was restored.
    pub project: ProjectId,
    /// Stable identity for retries.
    pub request_id: RequestId,
    /// Actor recorded on the event.
    pub actor: Actor,
    /// Effective policy version.
    pub policy_version: u64,
    /// Configured remote name.
    pub remote: String,
}

/// A restore cannot be accepted without a remote anchor, or storage failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcknowledgeRestoreError {
    /// The latest-anchor fetch did not give an anchor identity.
    NothingToAcknowledge(AnchorCheck),
    /// The store refused or failed the command.
    Store(StoreError),
}

impl fmt::Display for AcknowledgeRestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToAcknowledge(check) => write!(f, "nothing to acknowledge: {check:?}"),
            Self::Store(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AcknowledgeRestoreError {}

/// Judges whether this verification permits an acknowledgement event.
pub fn acknowledgement(
    report: &VerifyReport,
    anchor: &Anchor,
    remote: &str,
    checked_at: &str,
    project: &ProjectId,
) -> Result<RestoreAcknowledgedPayload, String> {
    if let Some(broken) = &report.chain.first_break {
        return Err(format!(
            "the local chain breaks at sequence {}; restore an earlier copy instead",
            broken.seq
        ));
    }
    match report.chain.anchor {
        AnchorVerdict::Truncated { .. } | AnchorVerdict::Rewritten { .. } => {}
        AnchorVerdict::Matches => return Err("matches the remote anchor".into()),
        AnchorVerdict::Acknowledged { .. } => return Err("already acknowledged".into()),
        AnchorVerdict::NoAnchor => return Err("no remote anchor was checked".into()),
        AnchorVerdict::Unchecked { .. } => return Err("the remote anchor was not checked".into()),
    }
    Ok(RestoreAcknowledgedPayload {
        remote: remote.into(),
        tag: anchor_tag(project, anchor.seq),
        seq: anchor.seq,
        head: anchor.hash,
        restored_seq: report.chain.head.as_ref().map_or(0, |head| head.seq),
        restored_head: report.chain.head.as_ref().map(|head| head.hash),
        checked_at: checked_at.into(),
    })
}

/// Fetches the remote anchor, verifies the copy, then records the owner's
/// acknowledgement through the normal scoped command path.
pub fn acknowledge_restore(
    request: &AcknowledgeRestore,
    ledger: &dyn Ledger,
    forge: &mut dyn Forge,
    now: &mut dyn FnMut() -> String,
) -> Result<Recorded, AcknowledgeRestoreError> {
    let observation = forge.fetch_tag(&request.project, &request.remote, &TagQuery::LatestAnchor);
    let checked_at = now();
    let check = anchor_status(&request.project, &observation);
    let AnchorCheck::Remote(anchor) = check else {
        return Err(AcknowledgeRestoreError::NothingToAcknowledge(check));
    };
    let report = ledger
        .verify(&request.project, Some(&anchor))
        .map_err(AcknowledgeRestoreError::Store)?;
    let judged = acknowledgement(
        &report,
        &anchor,
        &request.remote,
        &checked_at,
        &request.project,
    );
    let digest: Hash = request_digest(&json!({
        "kind": ANCHOR_ACKNOWLEDGE_RESTORE, "project": request.project.0,
        "actor": request.actor.as_str(), "policy_version": request.policy_version,
        "remote": request.remote, "scope": [ANCHOR_SCOPE],
        "anchor": {"seq": anchor.seq, "head": anchor.hash.to_hex()}
    }))
    .map_err(|error| {
        AcknowledgeRestoreError::Store(StoreError::Refused(Refusal::InvalidEvent(
            error.to_string(),
        )))
    })?;
    let command = Command {
        project: request.project.clone(),
        kind: CommandKind(ANCHOR_ACKNOWLEDGE_RESTORE.into()),
        request_id: request.request_id.clone(),
        digest,
        scope: vec![ANCHOR_SCOPE.into()],
        policy_version: request.policy_version,
        recorded_at: now(),
        actor: request.actor.clone(),
        caller: None,
    };
    ledger
        .transact(&command, &mut |tx| {
            if command.actor != Actor::Owner {
                return Err(StoreError::Refused(Refusal::NotOwner));
            }
            let head = tx.head()?;
            if head != report.chain.head {
                return Err(StoreError::Stale(StaleInput::Head {
                    seen: report.chain.head.clone(),
                    now: head,
                }));
            }
            let (kind, answer) = match &judged {
                Ok(payload) => {
                    tx.append(NewEvent {
                        stream: StreamName(ANCHOR_STREAM.into()),
                        type_name: ANCHOR_RESTORE_ACKNOWLEDGED.into(),
                        type_version: ANCHOR_RESTORE_ACKNOWLEDGED_VERSION,
                        git: None,
                        payload: payload.to_value(),
                        attachments: Vec::new(),
                    })?;
                    (
                        OutcomeKind::Done,
                        json!({"acknowledged": {"seq": anchor.seq, "head": anchor.hash.to_hex()}}),
                    )
                }
                Err(reason) => (OutcomeKind::Refused, json!({"reason": reason})),
            };
            Ok(Decision {
                kind,
                answer,
                sensitive: false,
                observed: Observed::default(),
                git: None,
            })
        })
        .map_err(AcknowledgeRestoreError::Store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use baley_store::{ChainReport, Head, StoredAnchorComparison};

    fn anchor() -> Anchor {
        Anchor {
            seq: 5,
            hash: Hash([5; 32]),
        }
    }
    fn report(verdict: AnchorVerdict) -> VerifyReport {
        VerifyReport {
            chain: ChainReport {
                head: Some(Head {
                    seq: 2,
                    hash: Hash([2; 32]),
                }),
                first_break: None,
                anchor: verdict,
                unanchored: None,
                acknowledged_restores: Vec::new(),
                age_unanchored_since: None,
            },
            payloads: Vec::new(),
            bodies_checked: 0,
            tombstones_checked: 0,
            stored_anchor: None,
            stored_anchor_comparison: StoredAnchorComparison::NotCompared,
        }
    }
    fn judge(report: &VerifyReport) -> Result<RestoreAcknowledgedPayload, String> {
        acknowledgement(
            report,
            &anchor(),
            "origin",
            "2026-09-25T18:00:00Z",
            &ProjectId("p".into()),
        )
    }

    // Catches a restored chain behind the remote refused by the pure judge.
    #[test]
    fn truncated_and_rewritten_chains_can_be_acknowledged() {
        let truncated = report(AnchorVerdict::Truncated {
            anchored: 5,
            head: 2,
        });
        let rewritten = report(AnchorVerdict::Rewritten {
            seq: 5,
            anchored: Hash([5; 32]),
            found: Hash([2; 32]),
        });
        assert_eq!(judge(&truncated).expect("truncated").restored_seq, 2);
        assert_eq!(
            judge(&rewritten).expect("rewritten").restored_head,
            Some(Hash([2; 32]))
        );
    }

    // Catches acknowledging a chain already matched or accepted.
    #[test]
    fn matching_and_acknowledged_chains_are_refused() {
        assert_eq!(
            judge(&report(AnchorVerdict::Matches)),
            Err("matches the remote anchor".into())
        );
        assert_eq!(
            judge(&report(AnchorVerdict::Acknowledged {
                anchored: 5,
                restored: None,
                acknowledged_seq: 3
            })),
            Err("already acknowledged".into())
        );
    }

    // Catches an acknowledgement that conceals a local chain break.
    #[test]
    fn a_local_break_cannot_be_acknowledged() {
        let mut broken = report(AnchorVerdict::Truncated {
            anchored: 5,
            head: 2,
        });
        broken.chain.first_break = Some(baley_store::Break {
            seq: 3,
            kind: baley_store::BreakKind::Sequence { found: 4 },
        });
        assert_eq!(
            judge(&broken),
            Err("the local chain breaks at sequence 3; restore an earlier copy instead".into())
        );
    }
}
