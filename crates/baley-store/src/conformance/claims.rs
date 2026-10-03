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
fn owned_by(request: &str, caller: Option<&Caller>) -> Command {
    let mut cmd = command("fixture.effect", request);
    cmd.scope = vec!["anchor".into()];
    cmd.caller = caller.cloned();
    cmd
}
fn take_as(store: &impl Ledger, cmd: &Command) {
    assert!(matches!(
        store.claim(cmd, &mut |_| Ok(ClaimDecision::Claim {
            intent: json!({"action":"push"}),
            owner: owner(),
            git: None,
            observed: Observed::default()
        })),
        Ok(Claimed::New { .. })
    ));
}
fn finish_as(store: &impl Ledger, cmd: &Command) {
    let mut cmd = cmd.clone();
    cmd.recorded_at = ACTIVE.into();
    store
        .complete(&cmd, &owner(), &mut |tx| {
            tx.append(event(
                2,
                json!({"id": 1, "state": "done", "rank": 0, "owner": ""}),
            ))?;
            Ok(done(json!("ok"), false))
        })
        .unwrap();
}
/// Claims by one caller and completes by another under one owner, completes a second claim with no caller, then takes a third claim with no caller. Catches a completion stamped with the claim's caller, a claim's provenance rewritten, a caller-free completion falling back to the claimer's, and a caller-free claim inheriting the previous claimer's caller.
pub fn a_claim_and_its_completion_keep_their_own_callers<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let (server, hook) = (server_caller(), hook_caller());
    take_as(&store, &owned_by("effect", Some(&server)));
    finish_as(&store, &owned_by("effect", Some(&hook)));
    take_as(&store, &owned_by("effect2", Some(&server)));
    finish_as(&store, &owned_by("effect2", None));
    take_as(&store, &owned_by("effect3", None));
    finish_as(&store, &owned_by("effect3", None));
    let events = history(&store);
    let seen: Vec<(&str, Option<Caller>)> = events
        .iter()
        .map(|e| (e.type_name.as_str(), e.caller.clone()))
        .collect();
    assert_eq!(
        seen,
        [
            (COMMAND_CLAIMED, Some(server.clone())),
            ("fixture.item", Some(hook.clone())),
            (COMMAND_COMPLETED, Some(hook)),
            (COMMAND_CLAIMED, Some(server)),
            ("fixture.item", None),
            (COMMAND_COMPLETED, None),
            (COMMAND_CLAIMED, None),
            ("fixture.item", None),
            (COMMAND_COMPLETED, None),
        ]
    );
}
fn id_of(request: &str) -> ClaimId {
    ClaimId {
        kind: CommandKind("fixture.effect".into()),
        request_id: RequestId(request.into()),
    }
}
fn resolve_with(
    store: &impl Ledger,
    cmd: &Command,
    claim: &ClaimId,
) -> Result<Recorded, StoreError> {
    store.reconcile(cmd, claim, ReconcileAuthority::Automatic, &mut |_, _| {
        Ok(finding(Resolution::Resolved(Box::new(done(
            json!("ok"),
            false,
        )))))
    })
}
/// Reconciles under a command that carries a caller, then retries a finished reconciliation with one. Catches a caller ignored or stamped on reconciliation, and a refusal placed after the request lookup, where the retry would replay.
pub fn a_reconciliation_that_carries_a_caller_is_refused<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let server = server_caller();
    take_as(&store, &owned_by("effect", None));
    let (before, claims) = (history(&store), store.open_claims(&project()).unwrap());
    let error = resolve_with(
        &store,
        &by(reconcile_command("with-caller"), &server),
        &id_of("effect"),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        StoreError::Refused(Refusal::InvalidEvent(_))
    ));
    assert_eq!(history(&store), before);
    assert_eq!(store.open_claims(&project()).unwrap(), claims);

    let mut second = owned_by("effect2", None);
    second.scope = vec!["other".into()];
    take_as(&store, &second);
    resolve_with(&store, &reconcile_command("retry"), &id_of("effect2")).unwrap();
    let before = history(&store);
    let claims = store.open_claims(&project()).unwrap();
    let error = resolve_with(
        &store,
        &by(reconcile_command("retry"), &server),
        &id_of("effect2"),
    );
    assert!(matches!(
        error,
        Err(StoreError::Refused(Refusal::InvalidEvent(_)))
    ));
    assert_eq!(history(&store), before);
    assert_eq!(store.open_claims(&project()).unwrap(), claims);
}
/// Reconciles a server caller's expired claim with a caller-free command. Catches the claim's caller copied onto the reconciliation.
pub fn a_reconciliation_records_no_caller_and_copies_none<F: StoreFactory>(factory: &F) {
    let store = created(factory);
    let server = server_caller();
    take_as(&store, &owned_by("effect", Some(&server)));
    resolve_with(&store, &reconcile_command("auto"), &id_of("effect")).unwrap();
    let seen: Vec<(String, Option<Caller>)> = history(&store)
        .into_iter()
        .map(|e| (e.type_name, e.caller))
        .collect();
    assert_eq!(
        seen,
        [
            (COMMAND_CLAIMED.to_string(), Some(server)),
            (COMMAND_RECONCILED.to_string(), None),
            (COMMAND_COMPLETED.to_string(), None),
            (COMMAND_COMPLETED.to_string(), None),
        ]
    );
}
