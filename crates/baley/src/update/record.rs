//! Completion of an `update.check` claim (design 0012 section 6).
//!
//! The result event and the claim's outcome commit together. The caller
//! stops the heartbeat before this step; completion never renews the lease.

use baley_store::{Decision, Ledger, NewEvent, Observed, OutcomeKind, Recorded, StoreError};

use super::claim::Check;
use super::events::{CheckOutcome, ClaimRef, Facts, FailureCode, checked_event, failed_event};
use super::version::Version;

/// The finished check and the versions it left in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckResult {
    /// A checked outcome, or the code of the failed step.
    pub outcome: Result<CheckOutcome, FailureCode>,
    /// The version the stable path runs, when it names one.
    pub active_version: Option<Version>,
    /// The version the check staged, when it staged one.
    pub staged_version: Option<Version>,
}

/// The result event and the outcome that closes its claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDecision {
    /// One `update.checked` or `update.failed` event on `install`.
    pub event: NewEvent,
    /// The claim's outcome, with the event payload as its answer.
    pub decision: Decision,
}

/// A checked outcome completes as done; a failed step completes as refused.
pub fn record_decision(
    facts: &Facts<'_>,
    outcome: Result<CheckOutcome, FailureCode>,
) -> RecordDecision {
    let (event, kind) = match outcome {
        Ok(outcome) => (checked_event(facts, outcome), OutcomeKind::Done),
        Err(code) => (failed_event(facts, code), OutcomeKind::Refused),
    };
    let decision = Decision {
        kind,
        answer: event.payload.clone(),
        sensitive: false,
        observed: Observed::default(),
        git: None,
    };
    RecordDecision { event, decision }
}

/// Records one result with the claim's command and owner at the supplied time.
/// `check` is the original request and `claim_seq` came from `Claimed::New`.
/// The caller must stop lease renewal before calling this step.
pub fn complete(
    ledger: &dyn Ledger,
    check: &Check,
    claim_seq: u64,
    result: &CheckResult,
    at: &str,
) -> Result<Recorded, StoreError> {
    let mut command = check.command()?;
    command.recorded_at = at.into();
    let plan = record_decision(
        &Facts {
            installation: &check.installation,
            // The command validated this time. Keep its day across midnight.
            day: &check.at[..10],
            claim: ClaimRef {
                request_id: &check.request_id,
                seq: claim_seq,
            },
            at,
            active_version: result.active_version,
            staged_version: result.staged_version,
        },
        result.outcome,
    );
    ledger.complete(&command, &check.owner, &mut |tx| {
        tx.append(plan.event.clone())?;
        Ok(plan.decision.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::open::{options, store};
    use crate::models;
    use crate::update::claim::{Trigger, claim_check, claim_owner};
    use baley_store::{Actor, Claimed, PageRequest, ProjectId, RequestId, StreamName};
    use serde_json::json;

    fn manual(request: &str, at: &str) -> Check {
        Check {
            installation: "/home/o/.local/bin/baley".into(),
            trigger: Trigger::Manual,
            request_id: RequestId(request.into()),
            at: at.into(),
            owner: claim_owner(4242, at),
        }
    }

    #[test]
    fn an_update_outcome_missing_from_install_or_recorded_twice_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = store(&dir.path().join("home"), "2026-10-08T08:00:00Z", options()).unwrap();
        models::create_user(&ledger, "2026-10-08T08:00:00Z").unwrap();
        let user = ProjectId("user".into());
        let version = Some(Version::parse("0.2.0").unwrap());
        let checked = manual(
            "00000000-0000-4000-8000-0000000000aa",
            "2026-10-08T09:00:00Z",
        );
        let Claimed::New { seq } = claim_check(&ledger, &checked).unwrap() else {
            panic!("the first manual check must take a new claim");
        };

        let recorded = complete(
            &ledger,
            &checked,
            seq,
            &CheckResult {
                outcome: Ok(CheckOutcome::Staged),
                active_version: version,
                staged_version: version,
            },
            "2026-10-08T09:00:05Z",
        )
        .unwrap();
        assert!(matches!(recorded, Recorded::New { .. }), "{recorded:?}");
        let first = ledger
            .stream(
                &user,
                &StreamName("install".into()),
                0,
                PageRequest {
                    limit: 100,
                    after: None,
                },
            )
            .unwrap();
        assert_eq!(first.items.len(), 1);
        let event = &first.items[0];
        assert_eq!(event.project_id, user);
        assert_eq!(event.stream, "install");
        assert_eq!(event.type_name, "update.checked");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.recorded_at, "2026-10-08T09:00:05Z");
        assert_eq!(event.actor, Actor::Owner);
        assert_eq!(event.caller, None);
        assert_eq!(event.request_id.0, "00000000-0000-4000-8000-0000000000aa");
        assert_eq!(
            event.payload,
            json!({
                "installation": "/home/o/.local/bin/baley",
                "day": "2026-10-08",
                "claim": {"request_id": "00000000-0000-4000-8000-0000000000aa", "seq": seq},
                "checked_at": "2026-10-08T09:00:05Z",
                "outcome": "staged",
                "active_version": "0.2.0",
                "staged_version": "0.2.0",
            })
        );
        assert!(ledger.open_claims(&user).unwrap().is_empty());

        let failed = manual(
            "00000000-0000-4000-8000-0000000000bb",
            "2026-10-08T09:10:00Z",
        );
        let Claimed::New { seq } = claim_check(&ledger, &failed).unwrap() else {
            panic!("completion must release the installation for a second manual check");
        };
        let recorded = complete(
            &ledger,
            &failed,
            seq,
            &CheckResult {
                outcome: Err(FailureCode::ChecksumMismatch),
                active_version: version,
                staged_version: None,
            },
            "2026-10-08T09:10:05Z",
        )
        .unwrap();
        assert!(matches!(recorded, Recorded::New { .. }), "{recorded:?}");
        let both = ledger
            .stream(
                &user,
                &StreamName("install".into()),
                0,
                PageRequest {
                    limit: 100,
                    after: None,
                },
            )
            .unwrap();
        assert_eq!(both.items.len(), 2);
        assert_eq!(both.items[0], first.items[0]);
        let event = &both.items[1];
        assert_eq!(event.project_id, user);
        assert_eq!(event.stream, "install");
        assert_eq!(event.type_name, "update.failed");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.recorded_at, "2026-10-08T09:10:05Z");
        assert_eq!(event.actor, Actor::Owner);
        assert_eq!(event.caller, None);
        assert_eq!(event.request_id.0, "00000000-0000-4000-8000-0000000000bb");
        assert_eq!(
            event.payload,
            json!({
                "installation": "/home/o/.local/bin/baley",
                "day": "2026-10-08",
                "claim": {"request_id": "00000000-0000-4000-8000-0000000000bb", "seq": seq},
                "observed_at": "2026-10-08T09:10:05Z",
                "code": "update-checksum-mismatch",
                "active_version": "0.2.0",
                "staged_version": null,
            })
        );
        assert!(ledger.open_claims(&user).unwrap().is_empty());
    }

    #[test]
    fn a_failed_check_completed_as_done_is_caught() {
        let request = RequestId("00000000-0000-4000-8000-0000000000aa".into());
        let version = Some(Version::parse("0.2.0").unwrap());
        for (outcome, staged_version, expected_kind) in [
            (Ok(CheckOutcome::Staged), version, OutcomeKind::Done),
            (
                Err(FailureCode::ChecksumMismatch),
                None,
                OutcomeKind::Refused,
            ),
        ] {
            let plan = record_decision(
                &Facts {
                    installation: "/home/o/.local/bin/baley",
                    day: "2026-10-08",
                    claim: ClaimRef {
                        request_id: &request,
                        seq: 12,
                    },
                    at: "2026-10-08T09:00:05Z",
                    active_version: version,
                    staged_version,
                },
                outcome,
            );
            assert_eq!(plan.decision.kind, expected_kind);
            assert_eq!(plan.decision.answer, plan.event.payload);
        }
    }
}
