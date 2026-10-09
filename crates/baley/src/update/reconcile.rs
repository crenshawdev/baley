//! Reconciliation of an interrupted update from local observations
//! (design 0012 section 6). No interrupted check fetches again.

use baley_store::{
    Actor, Claim, ClaimId, Command, CommandKind, Ledger, NewEvent, Observed, ProjectId,
    ReconcileAuthority, Reconciliation, Refusal, RequestId, Resolution, StaleInput, StoreError,
    request_digest,
};
use serde_json::{Value, json};

use super::claim::UPDATE_CHECK;
use super::events::{ClaimRef, Facts, FailureCode};
use super::installation::{
    Active, Child, Followed, Layout, StablePath, judge_active, versions_present,
};
use super::record::{CheckResult, record_decision};

const UPDATE_RECONCILE: &str = "update.reconcile";
const USER: &str = "user";

/// Whether the caller can retry its own claim after reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileStep {
    /// The interrupted claim was completed. Retry the caller's claim once.
    Resolved,
    /// Activation is ambiguous. The claim now waits for the owner.
    AwaitingOwner,
}

/// The writes requested by automatic reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileDecision {
    /// The failed check's event, absent while activation is ambiguous.
    pub event: Option<NewEvent>,
    /// The finding and whether it closes the interrupted claim.
    pub reconciliation: Reconciliation,
}

fn invalid(error: impl std::fmt::Display) -> StoreError {
    StoreError::Refused(Refusal::InvalidEvent(error.to_string()))
}

// Version judgement does not depend on who may close the claim.
fn observed_result(layout: &Layout, stable: &StablePath, children: &[Child]) -> CheckResult {
    let active_version = match judge_active(layout, stable) {
        Active::Version(version) => Some(version),
        Active::NotManaged(_) => None,
    };
    let staged_version = versions_present(children)
        .into_iter()
        .filter(|version| Some(*version) > active_version)
        .max();
    CheckResult {
        outcome: Err(FailureCode::Interrupted),
        active_version,
        staged_version,
    }
}

fn observation_value(stable: &StablePath, children: &[Child]) -> Value {
    // Keep the bytes so different non-UTF-8 names never share a digest.
    let stable = match stable {
        StablePath::Nothing => json!({"kind": "nothing"}),
        StablePath::NotALink => json!({"kind": "not-a-link"}),
        StablePath::Link { target, followed } => json!({
            "kind": "link",
            "target_bytes": target.as_os_str().as_encoded_bytes(),
            "followed": match followed {
                Followed::Missing => "missing",
                Followed::RegularFile => "regular-file",
                Followed::Other => "other",
            },
        }),
    };
    let children: Vec<_> = children
        .iter()
        .map(|child| {
            json!({
                "name_bytes": child.name.as_encoded_bytes(),
                "is_folder": child.is_folder,
                "holds_binary": child.holds_binary,
            })
        })
        .collect();
    json!({"stable_path": stable, "children": children})
}

/// Judges supplied local state. Only a managed stable link closes the claim
/// automatically; every other state records a finding and waits for the owner.
pub fn reconcile_decision(
    layout: &Layout,
    held: &Claim,
    stable: &StablePath,
    children: &[Child],
    at: &str,
) -> Result<ReconcileDecision, StoreError> {
    let installation = held.intent["installation"]
        .as_str()
        .ok_or_else(|| invalid("an update claim has no installation"))?;
    let day = held.intent["day"]
        .as_str()
        .ok_or_else(|| invalid("an update claim has no day"))?;
    if held.id.kind.0 != UPDATE_CHECK
        || installation != layout.installation()
        || held.scope != [format!("update/{installation}")]
    {
        return Err(invalid("the update claim does not hold this installation"));
    }
    let result = observed_result(layout, stable, children);
    let finding = json!({
        "installation": installation,
        "day": day,
        "observation": observation_value(stable, children),
        "observed_at": at,
        "active_version": result.active_version.map(|version| version.to_string()),
        "staged_version": result.staged_version.map(|version| version.to_string()),
    });
    let (event, resolution) = if result.active_version.is_some() {
        let record = record_decision(
            &Facts {
                installation,
                day,
                claim: ClaimRef {
                    request_id: &held.id.request_id,
                    seq: held.seq,
                },
                at,
                active_version: result.active_version,
                staged_version: result.staged_version,
            },
            result.outcome,
        );
        (
            Some(record.event),
            Resolution::Resolved(Box::new(record.decision)),
        )
    } else {
        (None, Resolution::AwaitingOwner)
    };
    Ok(ReconcileDecision {
        event,
        reconciliation: Reconciliation {
            finding,
            resolution,
            observed: Observed::default(),
        },
    })
}

/// Reads the blocking claim in `user` and records automatic reconciliation
/// under a fresh supplied request id and time. Observations come from the
/// caller. Store refusals, including a still-active claim, pass through.
pub fn reconcile_from_observation(
    ledger: &dyn Ledger,
    layout: &Layout,
    holder: &ClaimId,
    stable: &StablePath,
    children: &[Child],
    request_id: RequestId,
    at: &str,
) -> Result<ReconcileStep, StoreError> {
    let project = ProjectId(USER.into());
    let held = ledger
        .open_claims(&project)?
        .into_iter()
        .find(|claim| claim.id == *holder)
        .ok_or_else(|| StoreError::Stale(StaleInput::Claim(holder.clone())))?;
    let plan = reconcile_decision(layout, &held, stable, children, at)?;
    let command = Command {
        project,
        kind: CommandKind(UPDATE_RECONCILE.into()),
        request_id,
        digest: request_digest(&json!({
            "kind": UPDATE_RECONCILE,
            "project": USER,
            "claim": {
                "kind": held.id.kind.0,
                "request_id": held.id.request_id.0,
                "seq": held.seq,
            },
            "observation": observation_value(stable, children),
            "observed_at": at,
        }))
        .map_err(invalid)?,
        scope: Vec::new(),
        policy_version: 0,
        recorded_at: at.into(),
        actor: Actor::Baley,
        caller: None,
    };
    ledger.reconcile(
        &command,
        &held.id,
        ReconcileAuthority::Automatic,
        &mut |tx, claim| {
            if claim.seq != held.seq {
                return Err(StoreError::Stale(StaleInput::Claim(claim.id.clone())));
            }
            if let Some(event) = &plan.event {
                tx.append(event.clone())?;
            }
            Ok(plan.reconciliation.clone())
        },
    )?;
    Ok(match plan.reconciliation.resolution {
        Resolution::Resolved(_) => ReconcileStep::Resolved,
        Resolution::AwaitingOwner => ReconcileStep::AwaitingOwner,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::Environment;
    use crate::ledger::open::{options, store};
    use crate::models;
    use crate::update::claim::{Check, Trigger, claim_check, claim_owner};
    use baley_store::{Block, ClaimState, Claimed, OutcomeKind, PageRequest, StreamName};

    const INSTALLATION: &str = "/home/o/.local/bin/baley";
    const CLAIM_ID: &str = "00000000-0000-4000-8000-0000000000bb";
    const CLAIMED_AT: &str = "2026-10-08T09:00:00Z";
    const OBSERVED_AT: &str = "2026-10-08T09:01:01Z";

    fn layout() -> Layout {
        Layout::resolve(&Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        })
        .unwrap()
    }

    fn link(version: &str, followed: Followed) -> StablePath {
        StablePath::Link {
            target: format!("/home/o/.local/lib/crenshawdev/baley/versions/{version}/baley").into(),
            followed,
        }
    }

    fn children(versions: &[&str]) -> Vec<Child> {
        versions
            .iter()
            .map(|version| Child {
                name: (*version).into(),
                is_folder: true,
                holds_binary: true,
            })
            .collect()
    }

    #[test]
    fn an_interrupted_check_with_a_broken_stable_path_resolved_automatically_is_caught() {
        let held = Claim {
            id: ClaimId {
                kind: CommandKind("update.check".into()),
                request_id: RequestId(CLAIM_ID.into()),
            },
            seq: 12,
            claimed_at: CLAIMED_AT.into(),
            intent: json!({"installation": INSTALLATION, "day": "2026-10-08"}),
            scope: vec![format!("update/{INSTALLATION}")],
            owner: claim_owner(4242, CLAIMED_AT),
            lease_renewed_at: None,
            awaiting_owner: None,
        };
        let children = children(&["0.1.0", "0.2.0"]);
        for (active, staged) in [("0.1.0", Some("0.2.0")), ("0.2.0", None)] {
            let plan = reconcile_decision(
                &layout(),
                &held,
                &link(active, Followed::RegularFile),
                &children,
                OBSERVED_AT,
            )
            .unwrap();
            let event = plan.event.expect("a managed link completes the check");
            assert_eq!(event.stream.0, "install");
            assert_eq!(event.type_name, "update.failed");
            assert_eq!(event.type_version, 1);
            let expected = json!({
                "installation": INSTALLATION,
                "day": "2026-10-08",
                "claim": {"request_id": CLAIM_ID, "seq": 12},
                "observed_at": OBSERVED_AT,
                "code": "update-interrupted",
                "active_version": active,
                "staged_version": staged,
            });
            assert_eq!(event.payload, expected);
            let Resolution::Resolved(decision) = plan.reconciliation.resolution else {
                panic!("a managed link must resolve the claim");
            };
            assert_eq!(decision.kind, OutcomeKind::Refused);
            assert_eq!(decision.answer, expected);
        }
        for stable in [
            StablePath::NotALink,
            StablePath::Nothing,
            link("0.1.0", Followed::Missing),
        ] {
            let plan =
                reconcile_decision(&layout(), &held, &stable, &children, OBSERVED_AT).unwrap();
            assert_eq!(plan.reconciliation.resolution, Resolution::AwaitingOwner);
            assert_eq!(plan.event, None);
        }
    }

    #[test]
    fn an_interrupted_update_claim_left_blocking_after_reconciliation_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = store(&dir.path().join("home"), "2026-10-08T08:00:00Z", options()).unwrap();
        models::create_user(&ledger, "2026-10-08T08:00:00Z").unwrap();
        let manual = |request: &str, at: &str| Check {
            installation: INSTALLATION.into(),
            trigger: Trigger::Manual,
            request_id: RequestId(request.into()),
            at: at.into(),
            owner: claim_owner(4242, at),
        };
        let first = manual(CLAIM_ID, CLAIMED_AT);
        let Claimed::New { seq } = claim_check(&ledger, &first).unwrap() else {
            panic!("the first check must claim the installation");
        };
        let holder = ClaimId {
            kind: CommandKind("update.check".into()),
            request_id: first.request_id.clone(),
        };
        let second = manual("00000000-0000-4000-8000-0000000000cc", OBSERVED_AT);
        assert_eq!(
            claim_check(&ledger, &second),
            Err(StoreError::Blocked(Block {
                claim: holder.clone(),
                state: ClaimState::Interrupted,
            }))
        );
        let result = reconcile_from_observation(
            &ledger,
            &layout(),
            &holder,
            &link("0.1.0", Followed::RegularFile),
            &children(&["0.1.0"]),
            RequestId("00000000-0000-4000-8000-0000000000dd".into()),
            OBSERVED_AT,
        )
        .unwrap();
        assert_eq!(result, ReconcileStep::Resolved);
        let user = ProjectId("user".into());
        assert!(ledger.open_claims(&user).unwrap().is_empty());
        let stream = |name: &str| {
            ledger
                .stream(
                    &user,
                    &StreamName(name.into()),
                    0,
                    PageRequest {
                        limit: 100,
                        after: None,
                    },
                )
                .unwrap()
                .items
        };
        let commands = stream("command/update.check");
        let reconciled: Vec<_> = commands
            .iter()
            .filter(|event| event.type_name == "command.reconciled")
            .collect();
        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].payload["request_id"], CLAIM_ID);
        assert_eq!(reconciled[0].payload["claim_seq"], seq);
        assert_eq!(reconciled[0].payload["resolution"], "resolved");
        let install = stream("install");
        assert_eq!(install.len(), 1);
        assert_eq!(install[0].type_name, "update.failed");
        assert_eq!(
            install[0].payload,
            json!({
                "installation": INSTALLATION,
                "day": "2026-10-08",
                "claim": {"request_id": CLAIM_ID, "seq": seq},
                "observed_at": OBSERVED_AT,
                "code": "update-interrupted",
                "active_version": "0.1.0",
                "staged_version": null,
            })
        );
        let retry = manual("00000000-0000-4000-8000-0000000000ee", OBSERVED_AT);
        assert!(matches!(
            claim_check(&ledger, &retry),
            Ok(Claimed::New { .. })
        ));
    }
}
