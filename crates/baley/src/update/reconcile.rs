//! Reconciliation of an interrupted update from local observations
//! (design 0012 sections 6 and 12). No interrupted check fetches again.

use baley_store::{
    Actor, Claim, ClaimId, ClaimState, Command, CommandKind, Ledger, NewEvent, Observed, ProjectId,
    ReconcileAuthority, Reconciliation, Refusal, RequestId, Resolution, StaleInput, StoreError,
    claim_state, request_digest,
};
use serde_json::{Value, json};

use crate::ledger::display::{self, Render};

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

/// The writes requested by reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileDecision {
    /// The failed check's event, absent when automatic reconciliation must wait.
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
    decision(
        layout,
        held,
        stable,
        children,
        at,
        ReconcileAuthority::Automatic,
    )
}

fn decision(
    layout: &Layout,
    held: &Claim,
    stable: &StablePath,
    children: &[Child],
    at: &str,
    authority: ReconcileAuthority,
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
    let (event, resolution) =
        if authority == ReconcileAuthority::Owner || result.active_version.is_some() {
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
    record_reconciliation(
        ledger,
        &held,
        &plan,
        request_id,
        at,
        ReconcileAuthority::Automatic,
    )
}

fn record_reconciliation(
    ledger: &dyn Ledger,
    held: &Claim,
    plan: &ReconcileDecision,
    request_id: RequestId,
    at: &str,
    authority: ReconcileAuthority,
) -> Result<ReconcileStep, StoreError> {
    let command = Command {
        project: ProjectId(USER.into()),
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
            "observation": plan.reconciliation.finding["observation"],
            "observed_at": at,
        }))
        .map_err(invalid)?,
        scope: Vec::new(),
        policy_version: 0,
        recorded_at: at.into(),
        actor: match authority {
            ReconcileAuthority::Automatic => Actor::Baley,
            ReconcileAuthority::Owner => Actor::Owner,
        },
        caller: None,
    };
    ledger.reconcile(&command, &held.id, authority, &mut |tx, claim| {
        if claim.seq != held.seq {
            return Err(StoreError::Stale(StaleInput::Claim(claim.id.clone())));
        }
        if let Some(event) = &plan.event {
            tx.append(event.clone())?;
        }
        Ok(plan.reconciliation.clone())
    })?;
    Ok(match plan.reconciliation.resolution {
        Resolution::Resolved(_) => ReconcileStep::Resolved,
        Resolution::AwaitingOwner => ReconcileStep::AwaitingOwner,
    })
}

/// The claim the owner closed and the versions observed at resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedUpdate {
    /// The interrupted check's identity.
    pub claim: ClaimId,
    /// The interrupted outcome and the active and staged versions recorded.
    pub result: CheckResult,
}

/// Resolves this installation's owner hold from supplied local observations.
/// Active claims reach the store's liveness guard. An unheld interrupted
/// claim stays available for the next check's automatic reconciliation.
pub fn resolve_from_observation(
    ledger: &dyn Ledger,
    layout: &Layout,
    stable: &StablePath,
    children: &[Child],
    request_id: RequestId,
    at: &str,
) -> Result<Option<ResolvedUpdate>, StoreError> {
    let held = ledger
        .open_claims(&ProjectId(USER.into()))?
        .into_iter()
        .find(|claim| {
            claim.id.kind.0 == UPDATE_CHECK
                && claim.scope == [format!("update/{}", layout.installation())]
        });
    let Some(held) = held else {
        return Ok(None);
    };
    if held.awaiting_owner.is_none()
        && claim_state(&held, at).map_err(invalid)? != ClaimState::Active
    {
        return Ok(None);
    }
    let plan = decision(
        layout,
        &held,
        stable,
        children,
        at,
        ReconcileAuthority::Owner,
    )?;
    record_reconciliation(
        ledger,
        &held,
        &plan,
        request_id,
        at,
        ReconcileAuthority::Owner,
    )?;
    Ok(Some(ResolvedUpdate {
        claim: held.id,
        result: observed_result(layout, stable, children),
    }))
}

/// Reports owner resolution, keeping a live check in the busy exit class.
pub(crate) fn resolve_receipt(
    installation: &str,
    result: Result<Option<ResolvedUpdate>, StoreError>,
) -> Render {
    match result {
        Ok(None) => Render::line("no update check is held for this installation", 0),
        Ok(Some(resolved)) => Render {
            lines: vec![
                format!(
                    "resolved update check {} for {installation}",
                    resolved.claim.request_id.0
                ),
                format!(
                    "observed active version: {}; staged version: {}",
                    resolved
                        .result
                        .active_version
                        .map_or_else(|| "none".into(), |version| version.to_string()),
                    resolved
                        .result
                        .staged_version
                        .map_or_else(|| "none".into(), |version| version.to_string()),
                ),
                "the next baley update can run".into(),
            ],
            code: 0,
            error: false,
        },
        Err(StoreError::Stale(StaleInput::ClaimActive(holder))) => Render::refusal(format!(
            "update-check-busy: update check {} for {installation} is active",
            holder.request_id.0
        )),
        Err(error) => display::store_error(&error, Some(USER)),
    }
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
    fn a_held_update_claim_left_blocking_after_the_owner_resolves_it_is_caught() {
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
        assert_eq!(
            reconcile_from_observation(
                &ledger,
                &layout(),
                &holder,
                &StablePath::NotALink,
                &children(&["0.1.0"]),
                RequestId("00000000-0000-4000-8000-0000000000cc".into()),
                OBSERVED_AT,
            )
            .unwrap(),
            ReconcileStep::AwaitingOwner
        );
        let blocked = manual("00000000-0000-4000-8000-0000000000dd", OBSERVED_AT);
        assert_eq!(
            claim_check(&ledger, &blocked),
            Err(StoreError::Blocked(Block {
                claim: holder.clone(),
                state: ClaimState::AwaitingOwner,
            }))
        );
        let resolved = resolve_from_observation(
            &ledger,
            &layout(),
            &link("0.1.0", Followed::RegularFile),
            &children(&["0.1.0"]),
            RequestId("00000000-0000-4000-8000-0000000000ee".into()),
            "2026-10-08T09:02:00Z",
        )
        .unwrap()
        .expect("the owner must resolve the held claim");
        assert_eq!(resolved.claim, holder);
        let user = ProjectId("user".into());
        assert!(ledger.open_claims(&user).unwrap().is_empty());
        let install = ledger
            .stream(
                &user,
                &StreamName("install".into()),
                0,
                PageRequest {
                    limit: 100,
                    after: None,
                },
            )
            .unwrap()
            .items;
        assert_eq!(install.len(), 1);
        assert_eq!(install[0].type_name, "update.failed");
        assert_eq!(install[0].type_version, 1);
        assert_eq!(install[0].actor, Actor::Owner);
        assert_eq!(install[0].policy_version, 0);
        assert_eq!(install[0].caller, None);
        assert_eq!(
            install[0].payload,
            json!({
                "installation": INSTALLATION,
                "day": "2026-10-08",
                "claim": {"request_id": CLAIM_ID, "seq": seq},
                "observed_at": "2026-10-08T09:02:00Z",
                "code": "update-interrupted",
                "active_version": "0.1.0",
                "staged_version": null,
            })
        );
        let retry = manual(
            "00000000-0000-4000-8000-0000000000ff",
            "2026-10-08T09:03:00Z",
        );
        assert!(matches!(
            claim_check(&ledger, &retry),
            Ok(Claimed::New { .. })
        ));
    }

    #[test]
    fn an_owner_resolve_with_nothing_held_recording_something_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = store(&dir.path().join("home"), "2026-10-08T08:00:00Z", options()).unwrap();
        models::create_user(&ledger, "2026-10-08T08:00:00Z").unwrap();
        let user = ProjectId("user".into());
        let before = ledger.head(&user).unwrap();
        let resolve = |request: &str, at: &str| {
            resolve_from_observation(
                &ledger,
                &layout(),
                &link("0.1.0", Followed::RegularFile),
                &children(&["0.1.0"]),
                RequestId(request.into()),
                at,
            )
        };
        let nothing = resolve("00000000-0000-4000-8000-0000000000aa", CLAIMED_AT);
        assert_eq!(nothing, Ok(None));
        assert_eq!(ledger.head(&user).unwrap(), before);
        let receipt = resolve_receipt(INSTALLATION, nothing);
        assert_eq!(receipt.code, 0);
        assert!(!receipt.error);
        assert_eq!(
            receipt.lines,
            ["no update check is held for this installation"]
        );

        let check = Check {
            installation: INSTALLATION.into(),
            trigger: Trigger::Manual,
            request_id: RequestId(CLAIM_ID.into()),
            at: CLAIMED_AT.into(),
            owner: claim_owner(4242, CLAIMED_AT),
        };
        assert!(matches!(
            claim_check(&ledger, &check),
            Ok(Claimed::New { .. })
        ));
        let holder = ClaimId {
            kind: CommandKind("update.check".into()),
            request_id: check.request_id.clone(),
        };
        ledger
            .renew_lease(&user, &holder, &check.owner, "2026-10-08T09:00:30Z")
            .unwrap();
        let before = ledger.head(&user).unwrap();
        let open = ledger.open_claims(&user).unwrap();
        let busy = resolve(
            "00000000-0000-4000-8000-0000000000cc",
            "2026-10-08T09:00:40Z",
        );
        assert_eq!(
            busy,
            Err(StoreError::Stale(StaleInput::ClaimActive(holder)))
        );
        assert_eq!(ledger.head(&user).unwrap(), before);
        assert_eq!(ledger.open_claims(&user).unwrap(), open);
        let receipt = resolve_receipt(INSTALLATION, busy);
        assert_eq!(receipt.code, 2);
        assert!(receipt.error);
        assert!(receipt.lines[0].contains("update-check-busy"));
        assert!(receipt.lines[0].contains("active"));
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
