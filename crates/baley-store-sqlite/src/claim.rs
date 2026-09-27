//! Claims, lease renewals and reconciliation through the shared command path.

use baley_store::{
    Actor, Answer, COMMAND_CLAIMED, COMMAND_CLAIMED_VERSION, COMMAND_RECONCILED,
    COMMAND_RECONCILED_VERSION, Claim, ClaimDecision, ClaimDoc, ClaimId, ClaimOwner, Claimed,
    ClaimedPayload, Command, Decide, DecideClaim, DecideReconcile, DocKey, IndexQuery, KeyValue,
    NewEvent, Outcome, OutcomeKind, PageRequest, ProjectId, REQUEST_VIEW, ReconcileAuthority,
    ReconciledPayload, ReconciledResolution, Recorded, Refusal, RequestState, Resolution,
    StaleInput, StoreError, Transaction, UtcInstant, claim_state, command_stream, request_state,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::payload::sql_int;
use crate::rebuild::read_only;
use crate::store::{SqliteStore, sql};
use crate::transact::{CommandResult, Entry};
use crate::view::{Fence, Staging, find_documents, find_with_staged, get_document, live_views};

/// Builds a claim from a request document and its matching lease row.
pub(crate) fn claim_from_doc(
    conn: &Connection,
    project: &ProjectId,
    doc: &ClaimDoc,
    held_seq: Option<u64>,
) -> Result<Claim, StoreError> {
    let row = conn.query_row(
        "SELECT claim_seq, renewed_at FROM claim_lease WHERE project_id = ?1 AND kind = ?2 AND request_id = ?3",
        params![project.0, doc.id.kind.0, doc.id.request_id.0],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
    ).optional().map_err(sql)?;
    let lease_renewed_at = match row {
        Some((seq, renewed_at)) if u64::try_from(seq).ok() == Some(doc.seq) => Some(renewed_at),
        _ => None,
    };
    Ok(Claim {
        id: doc.id.clone(),
        seq: doc.seq,
        claimed_at: doc.claimed_at.clone(),
        intent: doc.intent.clone(),
        scope: doc.scope.clone(),
        owner: doc.owner.clone(),
        lease_renewed_at,
        awaiting_owner: held_seq,
    })
}

/// Pages through the two open states in the live generation.
pub(crate) fn open_claims_in(
    conn: &Connection,
    store: &SqliteStore,
    project: &ProjectId,
    generation: i64,
    staged: Option<&Staging>,
) -> Result<Vec<Claim>, StoreError> {
    let table = store.views().table(REQUEST_VIEW)?;
    let mut claims = Vec::new();
    for state in ["claimed", "awaiting_owner"] {
        let mut after = None;
        loop {
            let query = IndexQuery {
                index: "by_state".into(),
                equals: vec![KeyValue::Text(state.into())],
                page: PageRequest { limit: 100, after },
            };
            let page = match staged.and_then(|all| all.get(REQUEST_VIEW)) {
                Some(changes) => {
                    find_with_staged(conn, table, project, generation, &query, changes)?
                }
                None => find_documents(conn, table, project, generation, &query)?,
            };
            for document in page.items {
                let parsed = request_state(&document.body).ok_or_else(|| {
                    StoreError::Unavailable("a request document that holds no state".into())
                })?;
                let (doc, held) = match parsed {
                    RequestState::Claimed(doc) => (doc, None),
                    RequestState::AwaitingOwner(doc, seq) => (doc, Some(seq)),
                    RequestState::Completed(_, _) => {
                        return Err(StoreError::Unavailable(
                            "an open request index names a completion".into(),
                        ));
                    }
                };
                claims.push(claim_from_doc(conn, project, &doc, held)?);
            }
            match page.next {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
    }
    claims.sort_by_key(|claim| claim.seq);
    Ok(claims)
}

impl SqliteStore {
    /// Takes a claim or records a refusal before any external effect.
    pub fn claim(
        &self,
        command: &Command,
        decide: &mut DecideClaim<'_>,
    ) -> Result<Claimed, StoreError> {
        match self.command_path_for(command, Entry::Claim,
            |tx, state, _| match state {
                RequestState::Completed(_, outcome) => Ok(Claimed::Replayed(outcome)),
                RequestState::Claimed(doc) => Ok(Claimed::InProgress(claim_from_doc(tx, &command.project, &doc, None)?)),
                RequestState::AwaitingOwner(doc, seq) => Ok(Claimed::InProgress(claim_from_doc(tx, &command.project, &doc, Some(seq))?)),
            },
            |work, _| match decide(work)? {
                ClaimDecision::Refuse(decision) => {
                    let outcome = work.outcome(&decision)?;
                    work.finish(&outcome, decision.git, Some(&decision.observed))?;
                    Ok(Claimed::Refused { outcome, head: work.current_head().expect("completion appended") })
                }
                ClaimDecision::Claim { intent, owner, git, observed } => {
                    UtcInstant::parse(&owner.started_at).map_err(|_| StoreError::Refused(Refusal::InvalidEvent("claim owner started_at is not a UTC instant".into())))?;
                    work.recheck_observed(&observed)?;
                    let seq = work.push(NewEvent { stream: command_stream(&command.kind), type_name: COMMAND_CLAIMED.into(),
                        type_version: COMMAND_CLAIMED_VERSION, git,
                        payload: ClaimedPayload { kind: command.kind.clone(), request_id: command.request_id.clone(), digest: command.digest,
                            intent, scope: command.scope.clone(), owner }.to_value(), attachments: Vec::new() })?;
                    work.tx.execute("INSERT INTO claim_lease (project_id, kind, request_id, claim_seq, renewed_at) VALUES (?1, ?2, ?3, ?4, ?5)
                        ON CONFLICT (project_id, kind, request_id) DO UPDATE SET claim_seq = excluded.claim_seq, renewed_at = excluded.renewed_at",
                        params![command.project.0, command.kind.0, command.request_id.0, sql_int(seq)?, command.recorded_at]).map_err(sql)?;
                    Ok(Claimed::New { seq })
                }
            })? {
            CommandResult::Replayed(value) | CommandResult::New(value, _) => Ok(value),
        }
    }

    /// Renews a claimed request's lease without recording an event.
    pub fn renew_lease(
        &self,
        project: &ProjectId,
        claim: &ClaimId,
        owner: &ClaimOwner,
        at: &str,
    ) -> Result<(), StoreError> {
        loop {
            let result = self.write(|tx| {
                let live = live_views(tx, project)?;
                let generation = match self.views().judge_request(&live)? {
                    Fence::Current(generation) => generation,
                    Fence::Newer(reason) => return Err(read_only(project, reason)),
                    Fence::Behind | Fence::Unstamped => return Ok(None),
                };
                let table = self.views().table(REQUEST_VIEW)?;
                let key = DocKey(vec![KeyValue::Text(claim.kind.0.clone()), KeyValue::Text(claim.request_id.0.clone())]);
                let document = get_document(tx, table, project, generation, &key)?.ok_or_else(|| StoreError::Refused(Refusal::UnknownClaim(claim.clone())))?;
                let doc = match request_state(&document.body).ok_or_else(|| StoreError::Unavailable("a request document that holds no state".into()))? {
                    RequestState::Claimed(doc) => doc,
                    RequestState::AwaitingOwner(_, _) | RequestState::Completed(_, _) => return Err(StoreError::Stale(StaleInput::Claim(claim.clone()))),
                };
                if doc.id != *claim { return Err(StoreError::Unavailable("a request document names another claim".into())) }
                if doc.owner != *owner { return Err(StoreError::Refused(Refusal::NotClaimOwner(claim.clone()))) }
                let found = claim_from_doc(tx, project, &doc, None)?;
                let floor = found.lease_renewed_at.as_deref().unwrap_or(&doc.claimed_at);
                let parsed = UtcInstant::parse(at);
                if parsed.is_err() || parsed.expect("checked") < UtcInstant::parse(floor).map_err(|_| StoreError::Unavailable("an invalid lease floor".into()))? {
                    return Err(StoreError::Refused(Refusal::LeaseTime { claim: claim.clone(), at: at.into(), floor: floor.into() }));
                }
                tx.execute("INSERT INTO claim_lease (project_id, kind, request_id, claim_seq, renewed_at) VALUES (?1, ?2, ?3, ?4, ?5)
                    ON CONFLICT (project_id, kind, request_id) DO UPDATE SET claim_seq = excluded.claim_seq, renewed_at = excluded.renewed_at",
                    params![project.0, claim.kind.0, claim.request_id.0, sql_int(doc.seq)?, at]).map_err(sql)?;
                Ok(Some(()))
            })?;
            if result.is_some() {
                return Ok(());
            }
            self.bring_current(project)?;
        }
    }

    /// Records the acting owner's result, even when its lease expired.
    pub fn complete(
        &self,
        command: &Command,
        owner: &ClaimOwner,
        decide: &mut Decide<'_>,
    ) -> Result<Recorded, StoreError> {
        match self.command_path_for(command, Entry::Complete(owner),
            |_tx, state, _| match state { RequestState::Completed(_, outcome) => Ok(outcome), _ => Err(StoreError::Unavailable("a claim that cannot replay".into())) },
            |work, claim| {
                let claim = claim.expect("complete looked up its claim");
                let decision = decide(work)?;
                let outcome = work.outcome(&decision)?;
                work.recheck_observed(&decision.observed)?;
                work.complete_for(&claim.id.kind, &claim.id.request_id, &command.digest, &claim.scope, &outcome, decision.git)?;
                work.tx.execute("DELETE FROM claim_lease WHERE project_id = ?1 AND kind = ?2 AND request_id = ?3", params![command.project.0, claim.id.kind.0, claim.id.request_id.0]).map_err(sql)?;
                Ok(outcome)
            })? {
            CommandResult::Replayed(outcome) => Ok(Recorded::Replayed { outcome }),
            CommandResult::New(outcome, head) => Ok(Recorded::New { outcome, head }),
        }
    }

    /// Records a finding for an interrupted claim and closes or holds it.
    pub fn reconcile(
        &self,
        command: &Command,
        claim: &ClaimId,
        authority: ReconcileAuthority,
        decide: &mut DecideReconcile<'_>,
    ) -> Result<Recorded, StoreError> {
        if authority == ReconcileAuthority::Owner && command.actor != Actor::Owner {
            return Err(StoreError::Refused(Refusal::NotOwner));
        }
        match self.command_path_for(command, Entry::Reconcile(claim),
            |_tx, state, _| match state { RequestState::Completed(_, outcome) => Ok(outcome), _ => Err(StoreError::Unavailable("a reconciliation that cannot replay".into())) },
            |work, _| {
                let key = DocKey(vec![KeyValue::Text(claim.kind.0.clone()), KeyValue::Text(claim.request_id.0.clone())]);
                let document = work.get(REQUEST_VIEW, &key)?.ok_or_else(|| StoreError::Refused(Refusal::UnknownClaim(claim.clone())))?;
                let (doc, held) = match request_state(&document.body).ok_or_else(|| StoreError::Unavailable("a request document that holds no state".into()))? {
                    RequestState::Claimed(doc) => (doc, None),
                    RequestState::AwaitingOwner(doc, seq) => (doc, Some(seq)),
                    RequestState::Completed(_, _) => return Err(StoreError::Stale(StaleInput::Claim(claim.clone()))),
                };
                if doc.id != *claim { return Err(StoreError::Unavailable("a request document names another claim".into())) }
                let open = claim_from_doc(work.tx, &command.project, &doc, held)?;
                match claim_state(&open, &command.recorded_at).map_err(|_| StoreError::Unavailable("an invalid lease time".into()))? {
                    baley_store::ClaimState::Active => return Err(StoreError::Stale(StaleInput::ClaimActive(claim.clone()))),
                    baley_store::ClaimState::AwaitingOwner if authority == ReconcileAuthority::Automatic => return Err(StoreError::Refused(Refusal::AwaitingOwner(claim.clone()))),
                    _ => {},
                }
                let result = decide(work, &open)?;
                work.recheck_observed(&result.observed)?;
                let resolution = match &result.resolution { Resolution::Resolved(_) => ReconciledResolution::Resolved, Resolution::AwaitingOwner => ReconciledResolution::AwaitingOwner };
                let seq = work.push(NewEvent { stream: command_stream(&claim.kind), type_name: COMMAND_RECONCILED.into(), type_version: COMMAND_RECONCILED_VERSION,
                    git: None, payload: ReconciledPayload { claim: claim.clone(), claim_seq: open.seq, finding: result.finding, resolution }.to_value(), attachments: Vec::new() })?;
                if let Resolution::Resolved(decision) = result.resolution {
                    let outcome = work.outcome(&decision)?;
                    work.recheck_observed(&decision.observed)?;
                    work.complete_for(&claim.kind, &claim.request_id, &doc.digest, &doc.scope, &outcome, decision.git)?;
                    work.tx.execute("DELETE FROM claim_lease WHERE project_id = ?1 AND kind = ?2 AND request_id = ?3", params![command.project.0, claim.kind.0, claim.request_id.0]).map_err(sql)?;
                }
                let answer = json!({"claim": {"kind": claim.kind.0, "request_id": claim.request_id.0}, "reconciled": seq,
                    "resolution": match resolution { ReconciledResolution::Resolved => "resolved", ReconciledResolution::AwaitingOwner => "awaiting_owner" }});
                let outcome = Outcome { kind: OutcomeKind::Done, answer: Answer::Inline(answer) };
                work.finish(&outcome, None, None)?;
                Ok(outcome)
            })? {
            CommandResult::Replayed(outcome) => Ok(Recorded::Replayed { outcome }),
            CommandResult::New(outcome, head) => Ok(Recorded::New { outcome, head }),
        }
    }

    /// Reads every open claim in sequence order with matching lease rows.
    pub fn open_claims(&self, project: &ProjectId) -> Result<Vec<Claim>, StoreError> {
        self.read_current(project, |conn, generation| {
            open_claims_in(conn, self, project, generation, None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::scripted::Scripted;
    use crate::store::Options;
    use baley_store::{
        Actor, Block, CLAIM_SCOPE_VIEW, ClaimState, CommandKind, Decision, Hash, Observed,
        Reconciliation, RequestId, Views,
    };
    use rusqlite::params;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    const AT: &str = "2026-09-25T18:00:00Z";
    const LATER: &str = "2026-09-25T18:01:01Z";
    const PROJECT: &str = "7f0c2a4e-8d1b-4c3a-9e5f-2b6d8a1c4e70";

    struct Fixture {
        home: TempDir,
        store: SqliteStore,
    }
    impl Fixture {
        fn new() -> Self {
            let home = tempfile::tempdir().expect("temp dir");
            let store = SqliteStore::open(
                home.path(),
                AT,
                Options {
                    timing: Scripted::still(),
                    ..Options::default()
                },
            )
            .expect("open");
            store.write(|tx| {
                tx.execute("INSERT INTO project (project_id, name, created_at) VALUES (?1, 'fixture', ?2)", params![PROJECT, AT]).map_err(sql)?;
                Ok(())
            }).expect("project");
            Self { home, store }
        }
        fn raw(&self) -> Connection {
            Connection::open(self.home.path().join("baley.db")).expect("raw")
        }
        fn count(&self, table: &str) -> i64 {
            self.raw()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("count")
        }
        fn request(&self, kind: &str, id: &str) -> Value {
            self.store
                .get(
                    &ProjectId(PROJECT.into()),
                    REQUEST_VIEW,
                    &DocKey(vec![KeyValue::Text(kind.into()), KeyValue::Text(id.into())]),
                )
                .expect("request")
                .expect("present")
                .body
        }
        fn take(&self, command: &Command) -> Claimed {
            self.store
                .claim(command, &mut |_| {
                    Ok(ClaimDecision::Claim {
                        intent: json!({"action": "push"}),
                        owner: owner(),
                        git: None,
                        observed: Observed::default(),
                    })
                })
                .expect("claim")
        }
    }

    fn command(kind: &str, id: &str, at: &str, scope: &[&str]) -> Command {
        Command {
            project: ProjectId(PROJECT.into()),
            kind: CommandKind(kind.into()),
            request_id: RequestId(id.into()),
            digest: Hash([1; 32]),
            scope: scope.iter().map(|s| (*s).into()).collect(),
            policy_version: 1,
            recorded_at: at.into(),
            actor: Actor::Owner,
        }
    }
    fn owner() -> ClaimOwner {
        ClaimOwner {
            process: "p1".into(),
            host_session: "h1".into(),
            started_at: AT.into(),
        }
    }
    fn done() -> Decision {
        Decision {
            kind: OutcomeKind::Done,
            answer: json!({"ok": true}),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        }
    }
    fn refused() -> Decision {
        Decision {
            kind: OutcomeKind::Refused,
            answer: json!({"failed": true}),
            ..done()
        }
    }
    fn reconcile_decision(resolution: Resolution) -> Reconciliation {
        Reconciliation {
            finding: json!({"remote": "absent"}),
            resolution,
            observed: Observed::default(),
        }
    }
    fn claim_id() -> ClaimId {
        ClaimId {
            kind: CommandKind("anchor.push".into()),
            request_id: RequestId("r1".into()),
        }
    }
    fn claim_command() -> Command {
        command("anchor.push", "r1", AT, &["anchor"])
    }
    fn reconciler(at: &str, id: &str) -> Command {
        command("claim.reconcile", id, at, &[])
    }
    fn resolve(f: &Fixture) -> Recorded {
        f.take(&claim_command());
        f.store
            .reconcile(
                &reconciler(LATER, "rec1"),
                &claim_id(),
                ReconcileAuthority::Automatic,
                &mut |_, _| Ok(reconcile_decision(Resolution::Resolved(Box::new(done())))),
            )
            .expect("reconcile")
    }
    fn hold(f: &Fixture) -> Recorded {
        f.take(&claim_command());
        f.store
            .reconcile(
                &reconciler(LATER, "rec1"),
                &claim_id(),
                ReconcileAuthority::Automatic,
                &mut |_, _| Ok(reconcile_decision(Resolution::AwaitingOwner)),
            )
            .expect("hold")
    }

    // Catches a retry changing lease or request row counts.
    #[test]
    fn a_claim_retry_adds_no_lease_or_request_rows() {
        let f = Fixture::new();
        f.take(&claim_command());
        let before = (f.count("claim_lease"), f.count("v_request_2"));
        f.take(&claim_command());
        assert_eq!((f.count("claim_lease"), f.count("v_request_2")), before);
    }

    // Catches a completed claim whose lease row remains.
    #[test]
    fn completion_removes_the_lease() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.store
            .complete(&claim_command(), &owner(), &mut |_| Ok(refused()))
            .expect("complete");
        assert_eq!(f.count("claim_lease"), 0);
    }

    // Catches a completed request re-claimed.
    #[test]
    fn retry_after_clean_failure_replays_refusal() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.store
            .complete(&claim_command(), &owner(), &mut |_| Ok(refused()))
            .expect("complete");
        assert!(matches!(
            f.take(&claim_command()),
            Claimed::Replayed(Outcome {
                kind: OutcomeKind::Refused,
                ..
            })
        ));
    }

    // Catches expiry treated as release of the request id.
    #[test]
    fn retry_of_expired_claim_is_still_in_progress() {
        let f = Fixture::new();
        f.take(&claim_command());
        let mut later = claim_command();
        later.recorded_at = LATER.into();
        assert!(matches!(f.take(&later), Claimed::InProgress(_)));
    }

    // Catches a claim retry that ignores its digest.
    #[test]
    fn claim_retry_with_another_digest_is_refused() {
        let f = Fixture::new();
        f.take(&claim_command());
        let mut changed = claim_command();
        changed.digest = Hash([2; 32]);
        assert_eq!(
            f.store
                .claim(&changed, &mut |_| Ok(ClaimDecision::Refuse(done()))),
            Err(StoreError::Refused(Refusal::RequestDigestMismatch {
                request_id: changed.request_id
            }))
        );
    }

    // Catches a refused claim left open.
    #[test]
    fn refusal_at_claim_records_a_completed_request() {
        let f = Fixture::new();
        let result = f
            .store
            .claim(&claim_command(), &mut |_| {
                Ok(ClaimDecision::Refuse(refused()))
            })
            .expect("refuse");
        assert!(matches!(
            result,
            Claimed::Refused {
                outcome: Outcome {
                    kind: OutcomeKind::Refused,
                    ..
                },
                ..
            }
        ));
        assert_eq!(f.request("anchor.push", "r1")["state"], "completed");
        assert_eq!(
            f.store
                .open_claims(&ProjectId(PROJECT.into()))
                .expect("open claims"),
            []
        );
    }

    // Catches an intent bound to the post-claim head.
    #[test]
    fn claim_decision_reads_the_pre_claim_head() {
        let f = Fixture::new();
        let seed = command("seed", "s1", AT, &[]);
        let Recorded::New { head: before, .. } =
            f.store.transact(&seed, &mut |_| Ok(done())).expect("seed")
        else {
            panic!("new")
        };
        let result = f
            .store
            .claim(&claim_command(), &mut |tx| {
                let head = tx.head()?.expect("head");
                Ok(ClaimDecision::Claim {
                    intent: json!({"seq": head.seq, "head": head.hash.to_hex()}),
                    owner: owner(),
                    git: None,
                    observed: Observed::default(),
                })
            })
            .expect("claim");
        assert!(matches!(result, Claimed::New { .. }));
        assert_eq!(
            f.request("anchor.push", "r1")["claim"]["intent"],
            json!({"seq": before.seq, "head": before.hash.to_hex()})
        );
    }

    // Catches a store that does not enforce claim scope for another claim.
    #[test]
    fn second_claim_on_the_same_token_is_blocked() {
        let f = Fixture::new();
        f.take(&claim_command());
        let before = f.count("event");
        let second = command("anchor.push", "r2", AT, &["anchor"]);
        assert_eq!(
            f.store
                .claim(&second, &mut |_| Ok(ClaimDecision::Refuse(done()))),
            Err(StoreError::Blocked(Block {
                claim: claim_id(),
                state: ClaimState::Active
            }))
        );
        assert_eq!(f.count("event"), before);
    }

    // Catches a scope gate that blocks an unscoped command.
    #[test]
    fn unscoped_database_command_commits_beside_a_claim() {
        let f = Fixture::new();
        f.take(&claim_command());
        assert!(matches!(
            f.store
                .transact(&command("other", "r2", AT, &[]), &mut |_| Ok(done())),
            Ok(Recorded::New { .. })
        ));
    }

    // Catches a database-only path that reruns a claimed request.
    #[test]
    fn database_command_using_a_claimed_request_is_blocked() {
        let f = Fixture::new();
        f.take(&claim_command());
        assert_eq!(
            f.store.transact(&claim_command(), &mut |_| Ok(done())),
            Err(StoreError::Blocked(Block {
                claim: claim_id(),
                state: ClaimState::Active
            }))
        );
    }

    // Catches a transaction returning the old open-claims stub.
    #[test]
    fn transaction_reads_open_claims_with_lease() {
        let f = Fixture::new();
        f.take(&claim_command());
        let mut found = None;
        f.store
            .transact(&command("inspect", "r2", AT, &[]), &mut |tx| {
                found = Some(tx.open_claims()?);
                Ok(done())
            })
            .expect("inspect");
        let claims = found.expect("claims");
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].lease_renewed_at.as_deref(), Some(AT));
    }

    // Catches a scope document left after completion.
    #[test]
    fn completion_releases_the_scope_token() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.store
            .complete(&claim_command(), &owner(), &mut |_| Ok(done()))
            .expect("complete");
        let second = command("anchor.push", "r2", AT, &["anchor"]);
        assert!(matches!(f.take(&second), Claimed::New { .. }));
    }

    // Catches the store set version not raised for the new view.
    #[test]
    fn default_view_set_names_both_store_views() {
        let f = Fixture::new();
        let names: String = f
            .raw()
            .query_row(
                "SELECT sorted_view_names FROM view_set_catalog WHERE version = 2",
                [],
                |row| row.get(0),
            )
            .expect("catalog");
        assert_eq!(names, "claim_scope request");
    }

    // Catches a renewal that does not persist.
    #[test]
    fn renewal_updates_the_matching_row() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.store
            .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &owner(), LATER)
            .expect("renew");
        assert_eq!(
            f.store
                .open_claims(&ProjectId(PROJECT.into()))
                .expect("claims")[0]
                .lease_renewed_at
                .as_deref(),
            Some(LATER)
        );
    }

    // Catches a renewal that records an event.
    #[test]
    fn renewal_appends_no_event() {
        let f = Fixture::new();
        f.take(&claim_command());
        let before = f.count("event");
        f.store
            .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &owner(), LATER)
            .expect("renew");
        assert_eq!(f.count("event"), before);
    }

    // Catches a lease any process can renew.
    #[test]
    fn another_owner_cannot_renew() {
        let f = Fixture::new();
        f.take(&claim_command());
        let mut another = owner();
        another.process = "p2".into();
        assert_eq!(
            f.store
                .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &another, LATER),
            Err(StoreError::Refused(Refusal::NotClaimOwner(claim_id())))
        );
    }

    // Catches a lease moved backwards from its row time.
    #[test]
    fn renewal_uses_the_matching_row_as_its_floor() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.store
            .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &owner(), LATER)
            .expect("renew");
        let earlier = "2026-09-25T18:00:30Z";
        assert_eq!(
            f.store
                .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &owner(), earlier),
            Err(StoreError::Refused(Refusal::LeaseTime {
                claim: claim_id(),
                at: earlier.into(),
                floor: LATER.into()
            }))
        );
    }

    // Catches a lost row that removes the renewal lower bound.
    #[test]
    fn renewal_without_a_row_uses_claim_time_as_floor() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.raw()
            .execute("DELETE FROM claim_lease", [])
            .expect("delete");
        let earlier = "2026-09-25T17:59:59Z";
        assert_eq!(
            f.store
                .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &owner(), earlier),
            Err(StoreError::Refused(Refusal::LeaseTime {
                claim: claim_id(),
                at: earlier.into(),
                floor: AT.into()
            }))
        );
    }

    // Catches a lost row that cannot be recovered by its owner.
    #[test]
    fn renewal_replaces_a_lost_row_with_the_claim_sequence() {
        let f = Fixture::new();
        let Claimed::New { seq } = f.take(&claim_command()) else {
            panic!("claim")
        };
        f.raw()
            .execute("DELETE FROM claim_lease", [])
            .expect("delete");
        f.store
            .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &owner(), AT)
            .expect("renew");
        let row: i64 = f
            .raw()
            .query_row("SELECT claim_seq FROM claim_lease", [], |row| row.get(0))
            .expect("row");
        assert_eq!(u64::try_from(row), Ok(seq));
        assert_eq!(
            claim_state(
                &f.store
                    .open_claims(&ProjectId(PROJECT.into()))
                    .expect("claims")[0],
                AT
            ),
            Ok(ClaimState::Active)
        );
    }

    // Catches a heartbeat that reopens a completed claim.
    #[test]
    fn completed_claim_cannot_be_renewed() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.store
            .complete(&claim_command(), &owner(), &mut |_| Ok(done()))
            .expect("complete");
        assert_eq!(
            f.store
                .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &owner(), LATER),
            Err(StoreError::Stale(StaleInput::Claim(claim_id())))
        );
    }

    // Catches a heartbeat that turns an owner hold active.
    #[test]
    fn awaiting_owner_claim_cannot_be_renewed() {
        let f = Fixture::new();
        hold(&f);
        assert_eq!(
            f.store
                .renew_lease(&ProjectId(PROJECT.into()), &claim_id(), &owner(), LATER),
            Err(StoreError::Stale(StaleInput::Claim(claim_id())))
        );
    }

    // Catches a record step that requires a live lease.
    #[test]
    fn owner_can_complete_after_lease_expiry() {
        let f = Fixture::new();
        f.take(&claim_command());
        let mut later = claim_command();
        later.recorded_at = LATER.into();
        assert!(matches!(
            f.store.complete(&later, &owner(), &mut |_| Ok(done())),
            Ok(Recorded::New { .. })
        ));
    }

    // Catches a result recorded by a process that did not act.
    #[test]
    fn another_owner_cannot_complete() {
        let f = Fixture::new();
        f.take(&claim_command());
        let before = f.count("event");
        let mut another = owner();
        another.process = "p2".into();
        assert_eq!(
            f.store
                .complete(&claim_command(), &another, &mut |_| Ok(done())),
            Err(StoreError::Refused(Refusal::NotClaimOwner(claim_id())))
        );
        assert_eq!(f.count("event"), before);
    }

    // Catches a late owner recording another outcome.
    #[test]
    fn owner_completion_after_reconciliation_replays() {
        let f = Fixture::new();
        resolve(&f);
        assert!(matches!(
            f.store
                .complete(&claim_command(), &owner(), &mut |_| Ok(refused())),
            Ok(Recorded::Replayed {
                outcome: Outcome {
                    kind: OutcomeKind::Done,
                    ..
                }
            })
        ));
    }

    // Catches an owner closing a held claim through complete.
    #[test]
    fn owner_cannot_complete_an_awaiting_owner_claim() {
        let f = Fixture::new();
        hold(&f);
        assert_eq!(
            f.store
                .complete(&claim_command(), &owner(), &mut |_| Ok(done())),
            Err(StoreError::Refused(Refusal::AwaitingOwner(claim_id())))
        );
    }

    // Catches reconciliation leaving the claim open.
    #[test]
    fn claim_retry_after_reconciliation_replays() {
        let f = Fixture::new();
        resolve(&f);
        assert!(matches!(
            f.take(&claim_command()),
            Claimed::Replayed(Outcome {
                kind: OutcomeKind::Done,
                ..
            })
        ));
    }

    // Catches a reconciled claim whose lease row remains.
    #[test]
    fn resolved_reconciliation_removes_the_lease() {
        let f = Fixture::new();
        resolve(&f);
        assert_eq!(f.count("claim_lease"), 0);
    }

    // Catches a reconciler without its own completion receipt.
    #[test]
    fn reconciliation_returns_its_own_receipt() {
        let f = Fixture::new();
        let Recorded::New { outcome, .. } = resolve(&f) else {
            panic!("new")
        };
        let Answer::Inline(answer) = outcome.answer else {
            panic!("inline")
        };
        assert_eq!(
            answer,
            json!({"claim": {"kind": "anchor.push", "request_id": "r1"}, "reconciled": 2, "resolution": "resolved"})
        );
    }

    // Catches a held claim indistinguishable from an interrupted one.
    #[test]
    fn awaiting_owner_is_a_durable_request_state() {
        let f = Fixture::new();
        hold(&f);
        let body = f.request("anchor.push", "r1");
        assert_eq!(body["state"], "awaiting_owner");
        assert_eq!(
            body["held"],
            json!({"seq": 2, "finding": {"remote": "absent"}})
        );
        assert_eq!(
            f.store
                .open_claims(&ProjectId(PROJECT.into()))
                .expect("claims")[0]
                .awaiting_owner,
            Some(2)
        );
    }

    // Catches a held claim that stops blocking its scope.
    #[test]
    fn awaiting_owner_claim_blocks_its_scope() {
        let f = Fixture::new();
        hold(&f);
        assert_eq!(
            f.store
                .transact(&command("other", "r2", LATER, &["anchor"]), &mut |_| Ok(
                    done()
                )),
            Err(StoreError::Blocked(Block {
                claim: claim_id(),
                state: ClaimState::AwaitingOwner
            }))
        );
    }

    // Catches reconciliation of a live lease.
    #[test]
    fn active_claim_cannot_be_reconciled() {
        let f = Fixture::new();
        f.take(&claim_command());
        let before = f.count("event");
        assert_eq!(
            f.store.reconcile(
                &reconciler(AT, "rec1"),
                &claim_id(),
                ReconcileAuthority::Automatic,
                &mut |_, _| Ok(reconcile_decision(Resolution::Resolved(Box::new(done()))))
            ),
            Err(StoreError::Stale(StaleInput::ClaimActive(claim_id())))
        );
        assert_eq!(f.count("event"), before);
    }

    // Catches a second reconciliation of the same claim.
    #[test]
    fn completed_claim_cannot_be_reconciled_again() {
        let f = Fixture::new();
        resolve(&f);
        let before = f.count("event");
        assert_eq!(
            f.store.reconcile(
                &reconciler(LATER, "rec2"),
                &claim_id(),
                ReconcileAuthority::Automatic,
                &mut |_, _| Ok(reconcile_decision(Resolution::Resolved(Box::new(done()))))
            ),
            Err(StoreError::Stale(StaleInput::Claim(claim_id())))
        );
        assert_eq!(f.count("event"), before);
    }

    // Catches a new store version fencing replay of its own claim events.
    #[test]
    fn reopened_store_rebuilds_claim_and_reconciliation_events() {
        let f = Fixture::new();
        resolve(&f);
        let second = command("anchor.push", "r2", LATER, &["anchor"]);
        f.take(&second);
        let reopened = SqliteStore::open(
            f.home.path(),
            AT,
            Options {
                timing: Scripted::still(),
                ..Options::default()
            },
        )
        .expect("reopen");
        assert!(reopened.rebuild(&ProjectId(PROJECT.into())).is_ok());
    }

    // Catches a rebuild that loses an open claim from the request view.
    #[test]
    fn open_claim_survives_rebuild() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.store
            .rebuild(&ProjectId(PROJECT.into()))
            .expect("rebuild");
        assert!(matches!(f.take(&claim_command()), Claimed::InProgress(_)));
    }

    // Catches a claim completion projected to the reconciling request.
    #[test]
    fn rebuilt_request_documents_keep_both_request_identities() {
        let f = Fixture::new();
        resolve(&f);
        f.store
            .rebuild(&ProjectId(PROJECT.into()))
            .expect("rebuild");
        assert_eq!(
            f.request("anchor.push", "r1"),
            json!({"kind": "anchor.push", "request_id": "r1", "digest": "01".repeat(32),
            "scope": ["anchor"], "state": "completed", "outcome": "done", "answer": {"inline": {"ok": true}}})
        );
        assert_eq!(
            f.request("claim.reconcile", "rec1"),
            json!({"kind": "claim.reconcile", "request_id": "rec1", "digest": "01".repeat(32),
            "scope": [], "state": "completed", "outcome": "done", "answer": {"inline": {"claim": {"kind": "anchor.push", "request_id": "r1"}, "reconciled": 2, "resolution": "resolved"}}})
        );
    }

    // Catches replay restoring a token whose claim was completed.
    #[test]
    fn rebuilt_scope_view_keeps_resolved_token_released() {
        let f = Fixture::new();
        resolve(&f);
        f.store
            .rebuild(&ProjectId(PROJECT.into()))
            .expect("rebuild");
        assert_eq!(
            f.store.get(
                &ProjectId(PROJECT.into()),
                CLAIM_SCOPE_VIEW,
                &DocKey(vec![KeyValue::Text("anchor".into())])
            ),
            Ok(None)
        );
    }

    // Catches a damaged holder accepted despite its body's wrong identity.
    #[test]
    fn scope_check_refuses_a_holder_with_wrong_body_identity() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.raw().execute("UPDATE v_request_2 SET doc_json = json_set(doc_json, '$.request_id', 'other') WHERE k_request_id = 'r1'", []).expect("damage");
        let before = f.count("event");
        assert_eq!(
            f.store.transact(
                &command("other", "r2", AT, &["anchor"]),
                &mut |_| Ok(done())
            ),
            Err(StoreError::Unavailable(
                "a claim_scope document names a claim that does not hold it".into()
            ))
        );
        assert_eq!(f.count("event"), before);
    }

    // Catches a record step narrowing the gate it claimed under.
    #[test]
    fn complete_refuses_a_different_scope() {
        let f = Fixture::new();
        f.take(&claim_command());
        let before = f.count("event");
        let command = command("anchor.push", "r1", AT, &[]);
        assert_eq!(
            f.store.complete(&command, &owner(), &mut |_| Ok(done())),
            Err(StoreError::Refused(Refusal::ScopeMismatch {
                claim: claim_id(),
                supplied: vec![],
                claimed: vec!["anchor".into()]
            }))
        );
        assert_eq!(f.count("event"), before);
    }

    // Catches an owner resolution attributed to another actor.
    #[test]
    fn owner_authority_requires_owner_actor() {
        let f = Fixture::new();
        hold(&f);
        let before = f.count("event");
        let mut command = reconciler(LATER, "rec2");
        command.actor = Actor::Baley;
        assert_eq!(
            f.store.reconcile(
                &command,
                &claim_id(),
                ReconcileAuthority::Owner,
                &mut |_, _| Ok(reconcile_decision(Resolution::Resolved(Box::new(done()))))
            ),
            Err(StoreError::Refused(Refusal::NotOwner))
        );
        assert_eq!(f.count("event"), before);
    }

    // Catches a version-1 command completion accepted by this binary.
    #[test]
    fn version_one_completion_fences_new_commands() {
        use baley_store::{Event, EventDraft};
        let f = Fixture::new();
        let event = Event::seal(ProjectId(PROJECT.into()), 1, None, EventDraft { stream: "command/old".into(), stream_version: 1,
            type_name: "command.completed".into(), type_version: 1, actor: Actor::Owner, recorded_at: AT.into(), request_id: RequestId("old".into()),
            git: None, policy_version: 1, payload: json!({"kind": "old", "request_id": "old", "digest": "01".repeat(32), "outcome": "done", "answer": {"inline": null}}) }).expect("seal");
        let payload = String::from_utf8(baley_store::canonical_json(&event.payload).expect("json"))
            .expect("utf8");
        f.raw().execute("INSERT INTO event (project_id, seq, stream, stream_version, type, type_version, actor, recorded_at, request_id, policy_version, payload_json, hash)
            VALUES (?1, 1, ?2, 1, ?3, 1, 'owner', ?4, ?5, 1, ?6, ?7)",
            params![PROJECT, event.stream, event.type_name, AT, event.request_id.0, payload, &event.hash.0[..]]).expect("insert");
        f.raw()
            .execute(
                "UPDATE project SET head_seq = 1, head_hash = ?1 WHERE project_id = ?2",
                params![&event.hash.0[..], PROJECT],
            )
            .expect("head");
        let result = f
            .store
            .transact(&command("other", "r2", AT, &[]), &mut |_| Ok(done()));
        assert!(
            matches!(result, Err(StoreError::Refused(Refusal::ProjectReadOnly { reason, .. })) if reason.contains("command.completed version 1"))
        );
    }

    // Catches a replay checking the reconciling receipt's answer reference.
    #[test]
    fn replay_of_reconciled_claim_returns_a_purged_answer_tombstone() {
        let f = Fixture::new();
        f.take(&claim_command());
        let answer = "x".repeat(5000);
        f.store
            .reconcile(
                &reconciler(LATER, "rec1"),
                &claim_id(),
                ReconcileAuthority::Automatic,
                &mut |_, _| {
                    Ok(reconcile_decision(Resolution::Resolved(Box::new(
                        Decision {
                            answer: json!(answer),
                            ..done()
                        },
                    ))))
                },
            )
            .expect("reconcile");
        let hash = Hash::from_hex(
            f.request("anchor.push", "r1")["answer"]["stored"]["payload"]
                .as_str()
                .expect("hash"),
        )
        .expect("hex");
        f.store
            .purge(
                &command("retention.purge", "p1", LATER, &[]),
                &[hash],
                "owner request",
            )
            .expect("purge");
        assert!(matches!(
            f.take(&claim_command()),
            Claimed::Replayed(Outcome {
                answer: Answer::Tombstone { .. },
                ..
            })
        ));
    }

    // Catches a missing row read as active for sixty seconds.
    #[test]
    fn lost_lease_row_reads_as_interrupted_immediately() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.raw()
            .execute("DELETE FROM claim_lease", [])
            .expect("delete");
        let claim = &f
            .store
            .open_claims(&ProjectId(PROJECT.into()))
            .expect("claims")[0];
        assert_eq!(claim.lease_renewed_at, None);
        assert_eq!(
            claim_state(claim, "2026-09-25T18:00:01Z"),
            Ok(ClaimState::Interrupted)
        );
    }

    // Catches a stale row of another incarnation keeping a claim active.
    #[test]
    fn mismatched_lease_sequence_reads_as_missing() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.raw()
            .execute(
                "UPDATE claim_lease SET claim_seq = 999, renewed_at = ?1",
                [AT],
            )
            .expect("damage");
        assert_eq!(
            f.store
                .open_claims(&ProjectId(PROJECT.into()))
                .expect("claims")[0]
                .lease_renewed_at,
            None
        );
    }

    // Catches a malformed existing request treated as absent and reclaimed.
    #[test]
    fn malformed_request_document_fails_claim_without_writing() {
        let f = Fixture::new();
        f.take(&claim_command());
        f.raw()
            .execute(
                "UPDATE v_request_2 SET doc_json = '{}' WHERE k_request_id = 'r1'",
                [],
            )
            .expect("damage");
        let before = f.count("event");
        assert_eq!(
            f.store
                .claim(&claim_command(), &mut |_| Ok(ClaimDecision::Refuse(done()))),
            Err(StoreError::Unavailable(
                "a request document that holds no state".into()
            ))
        );
        assert_eq!(f.count("event"), before);
    }

    // Catches an old epoch-1 file with no completion events being written to.
    #[test]
    fn old_schema_digest_is_refused_without_changing_the_file() {
        let home = tempfile::tempdir().expect("temp dir");
        let path = home.path().join("baley.db");
        let conn = Connection::open(&path).expect("old file");
        conn.execute_batch("CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value ANY NOT NULL);
            CREATE TABLE claim_lease (project_id TEXT, kind TEXT, request_id TEXT, claim_seq INTEGER, owner TEXT, renewed_at TEXT);
            INSERT INTO schema_meta VALUES ('epoch', 1), ('schema_digest', 'old');").expect("old schema");
        drop(conn);
        let before = std::fs::read(&path).expect("read");
        let result = SqliteStore::open(home.path(), AT, Options::default());
        assert!(
            matches!(result, Err(StoreError::Refused(Refusal::SchemaChanged { path: named })) if named == path)
        );
        assert_eq!(std::fs::read(&path).expect("read"), before);
    }

    // Catches the owner actor check running after the replay path.
    #[test]
    fn non_owner_cannot_replay_an_owner_resolution() {
        let f = Fixture::new();
        hold(&f);
        let command = reconciler(LATER, "rec2");
        f.store
            .reconcile(
                &command,
                &claim_id(),
                ReconcileAuthority::Owner,
                &mut |_, _| Ok(reconcile_decision(Resolution::Resolved(Box::new(done())))),
            )
            .expect("owner resolution");
        let before = f.count("event");
        let mut other = command;
        other.actor = Actor::Baley;
        assert_eq!(
            f.store.reconcile(
                &other,
                &claim_id(),
                ReconcileAuthority::Owner,
                &mut |_, _| Ok(reconcile_decision(Resolution::Resolved(Box::new(done()))))
            ),
            Err(StoreError::Refused(Refusal::NotOwner))
        );
        assert_eq!(f.count("event"), before);
    }
}
