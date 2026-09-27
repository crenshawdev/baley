//! EVD-R26: scope blocking and reconciliation from supplied times and findings.
use super::fixture::*;
use crate::*;
use serde_json::json;
fn owner() -> ClaimOwner {
    ClaimOwner {
        process: "p1".into(),
        host_session: "h1".into(),
        started_at: T0.into(),
    }
}
fn claim_command() -> Command {
    let mut cmd = command("fixture.effect", "effect");
    cmd.scope = vec!["anchor".into()];
    cmd
}
fn claim_id() -> ClaimId {
    ClaimId {
        kind: CommandKind("fixture.effect".into()),
        request_id: RequestId("effect".into()),
    }
}
fn take(store: &impl Ledger) {
    assert_eq!(
        store.claim(&claim_command(), &mut |_| Ok(ClaimDecision::Claim {
            intent: json!({"action":"push"}),
            owner: owner(),
            git: None,
            observed: Observed::default()
        })),
        Ok(Claimed::New { seq: 1 })
    );
}
fn scoped(scope: &str) -> Command {
    let mut cmd = command("fixture.check", "check");
    cmd.scope = vec![scope.into()];
    cmd.recorded_at = ACTIVE.into();
    cmd
}
fn reconcile_command(id: &str) -> Command {
    let mut cmd = command("fixture.reconcile", id);
    cmd.recorded_at = EXPIRED.into();
    cmd
}
fn finding(resolution: Resolution) -> Reconciliation {
    Reconciliation {
        finding: json!({"remote":"absent"}),
        resolution,
        observed: Observed::default(),
    }
}
fn hold(store: &impl Ledger) {
    take(store);
    store
        .reconcile(
            &reconcile_command("hold"),
            &claim_id(),
            ReconcileAuthority::Automatic,
            &mut |_, _| Ok(finding(Resolution::AwaitingOwner)),
        )
        .unwrap();
}
/// Runs outside the active token. Catches scope gates that block unrelated commands.
pub fn a_command_outside_an_active_claims_scope_proceeds<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    take(&store);
    assert!(matches!(
        store.transact(&scoped("other"), &mut |_| Ok(done(json!("ok"), false))),
        Ok(Recorded::New { .. })
    ));
    assert_eq!(store.open_claims(&project()).unwrap().len(), 1);
}
/// Enters an active token's scope. Catches decisions run before the scope gate.
pub fn a_command_inside_an_active_claims_scope_is_blocked<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    take(&store);
    let before = history(&store);
    assert_eq!(
        store.transact(&scoped("anchor"), &mut |_| panic!("blocked decision ran")),
        Err(StoreError::Blocked(Block {
            claim: claim_id(),
            state: ClaimState::Active
        }))
    );
    assert_eq!(history(&store), before);
}
/// Retries while the effect is in progress. Catches a second claim or renewed lease on retry.
pub fn a_retry_during_the_effect_is_in_progress<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    take(&store);
    let before = history(&store);
    let claims = store.open_claims(&project()).unwrap();
    let document = store
        .get(&project(), "request", &request("fixture.effect", "effect"))
        .unwrap();
    let mut cmd = claim_command();
    cmd.recorded_at = ACTIVE.into();
    assert_eq!(
        store.claim(&cmd, &mut |_| panic!("duplicate effect")),
        Ok(Claimed::InProgress(claims[0].clone()))
    );
    assert_eq!(history(&store), before);
    assert_eq!(store.open_claims(&project()).unwrap(), claims);
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.effect", "effect"))
            .unwrap(),
        document
    );
}
/// Completes an effect with a refusal. Catches a clean failure leaving a held scope.
pub fn a_cleanly_failed_effect_completes_the_claim<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    take(&store);
    let mut cmd = claim_command();
    cmd.recorded_at = ACTIVE.into();
    let result = store
        .complete(&cmd, &owner(), &mut |_| {
            Ok(Decision {
                kind: OutcomeKind::Refused,
                ..done(json!({"failed":true}), false)
            })
        })
        .unwrap();
    assert!(matches!(
        result,
        Recorded::New {
            outcome: Outcome {
                kind: OutcomeKind::Refused,
                ..
            },
            ..
        }
    ));
    let body = store
        .get(&project(), "request", &request("fixture.effect", "effect"))
        .unwrap()
        .unwrap()
        .body;
    assert_eq!(body["state"], "completed");
    assert_eq!(body["outcome"], "refused");
    assert!(store.open_claims(&project()).unwrap().is_empty());
    assert_eq!(
        store
            .get(
                &project(),
                CLAIM_SCOPE_VIEW,
                &DocKey(vec![KeyValue::Text("anchor".into())])
            )
            .unwrap(),
        None
    );
    assert!(matches!(
        store.transact(&scoped("anchor"), &mut |_| Ok(done(json!("ok"), false))),
        Ok(Recorded::New { .. })
    ));
}
/// Resolves an expired claim from a supplied finding. Catches reconciliation without recorded evidence or completion.
pub fn an_interrupted_claim_reconciles_from_a_supplied_finding<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    take(&store);
    store
        .reconcile(
            &reconcile_command("resolve"),
            &claim_id(),
            ReconcileAuthority::Automatic,
            &mut |_, _| {
                Ok(finding(Resolution::Resolved(Box::new(done(
                    json!("ok"),
                    false,
                )))))
            },
        )
        .unwrap();
    assert!(store.open_claims(&project()).unwrap().is_empty());
    let events = history(&store);
    assert_eq!(events.len(), 4);
    assert_eq!(events[1].type_name, COMMAND_RECONCILED);
    assert_eq!(events[1].stream, "command/fixture.effect");
    assert_eq!(events[1].payload["finding"], json!({"remote":"absent"}));
    assert_eq!(events[2].type_name, COMMAND_COMPLETED);
    assert_eq!(events[3].type_name, COMMAND_COMPLETED);
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.effect", "effect"))
            .unwrap()
            .unwrap()
            .body["state"],
        json!("completed")
    );
}
/// Attempts automatic resolution of an owner hold. Catches unauthorized release of a held claim.
pub fn automatic_reconciliation_leaves_an_awaiting_owner_claim_held<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    hold(&store);
    let before = history(&store);
    let claims = store.open_claims(&project()).unwrap();
    assert_eq!(
        store.reconcile(
            &reconcile_command("automatic"),
            &claim_id(),
            ReconcileAuthority::Automatic,
            &mut |_, _| panic!("held decision ran")
        ),
        Err(StoreError::Refused(Refusal::AwaitingOwner(claim_id())))
    );
    assert_eq!(history(&store), before);
    assert_eq!(store.open_claims(&project()).unwrap(), claims);
}
/// Resolves an owner hold with owner authority. Catches an owner claim with no completion path.
pub fn owner_reconciliation_resolves_an_awaiting_owner_claim<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    hold(&store);
    let claims = store.open_claims(&project()).unwrap();
    assert!(claims[0].awaiting_owner.is_some());
    store
        .reconcile(
            &reconcile_command("owner"),
            &claim_id(),
            ReconcileAuthority::Owner,
            &mut |_, _| {
                Ok(finding(Resolution::Resolved(Box::new(done(
                    json!("ok"),
                    false,
                )))))
            },
        )
        .unwrap();
    assert!(store.open_claims(&project()).unwrap().is_empty());
    assert_eq!(
        store
            .get(
                &project(),
                CLAIM_SCOPE_VIEW,
                &DocKey(vec![KeyValue::Text("anchor".into())])
            )
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .get(&project(), "request", &request("fixture.effect", "effect"))
            .unwrap()
            .unwrap()
            .body["state"],
        json!("completed")
    );
}
