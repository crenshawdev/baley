//! The ledger's reads, verification and anchor rows on this adapter, and
//! the core's anchor steps run against it. Each test opens a fresh store in
//! a temporary directory it owns; remote observations and times are
//! supplied values, and no test runs git, a forge, a ticker or a clock. The
//! anchor command's retry runs over an in-memory remote that holds no tags
//! and a ticker that never ticks.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use baley_core::{
    AcknowledgeRestore, AcknowledgeRestoreError, AnchorRequest, AnchorSeams, ClaimStep,
    FetchObservation, Forge, HeldAnchor, PrePushCheck, PushObservation, ReconcileStep, Registry,
    TagQuery, TickGuard, Ticker, TraceRecord, TraceSink, Upcaster, acknowledge_restore,
    anchor_annotation, anchor_command, claim_step, pre_push_check, reconcile_from_observation,
    record_step, register_anchor_events, verify_observed,
};
use baley_store::{
    ANCHOR_FAILED, ANCHOR_PUSHED, Actor, Admin, Anchor, AnchorCheck, AnchorVerdict,
    COMMAND_CLAIMED, COMMAND_COMPLETED, COMMAND_RECONCILED, ClaimDecision, ClaimOwner, ClaimState,
    Command, CommandKind, Decision, DocKey, Event, EventSchema, GitFacts, Hash, Head,
    HistoryFilter, KeyValue, Ledger, NewEvent, Observed, OutcomeKind, PageRequest, PayloadFault,
    PayloadReference, ProjectId, REQUEST_VIEW, Recorded, Refusal, RequestId, RetentionClass,
    StoreError, StoredAnchorComparison, StreamName, Transaction, UnanchoredAge, Views, anchor_tag,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::queue::scripted::Scripted;
use crate::store::{Options, SqliteStore, TraceEntry, sql};

const PROJECT: &str = "7f0c2a4e-8d1b-4c3a-9e5f-2b6d8a1c4e70";
const RECORDED: &str = "fixture.recorded";
const T0: &str = "2026-09-25T18:00:00Z";
const T1: &str = "2026-09-25T18:00:01Z";
const T2: &str = "2026-09-25T18:00:02Z";
/// Past the first claim's 60-second lease.
const EXPIRED: &str = "2026-09-25T18:01:01Z";
const CHECKED: &str = "2026-09-25T18:01:02Z";
const RETRY: &str = "2026-09-25T18:01:03Z";

fn project() -> ProjectId {
    ProjectId(PROJECT.into())
}

/// The core's registry with the anchor events and one fixture type.
fn registry() -> Registry {
    let mut registry = Registry::new();
    register_anchor_events(&mut registry).expect("anchor events");
    registry.register(RECORDED, 1, []).expect("fixture type");
    registry
}

struct Fixture {
    home: TempDir,
    store: Arc<SqliteStore>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_schema(Box::new(registry()))
    }

    fn with_schema(schema: Box<dyn EventSchema>) -> Self {
        let home = crate::checks::private_folder();
        let store = open(home.path(), schema);
        store
            .write(|tx| {
                tx.execute(
                    "INSERT INTO project (project_id, name, created_at) VALUES (?1, 'fixture', ?2)",
                    params![PROJECT, T0],
                )
                .map_err(sql)?;
                Ok(())
            })
            .expect("project");
        Self {
            home,
            store: Arc::new(store),
        }
    }

    fn ledger(&self) -> &dyn Ledger {
        self.store.as_ref()
    }

    fn raw(&self) -> Connection {
        Connection::open(self.home.path().join("baley.db")).expect("raw")
    }

    /// One command appending `count` fixture events at `at`.
    fn record_at(&self, request: &str, count: usize, at: &str) -> Head {
        self.write(request, at, |tx| {
            for n in 0..count {
                tx.append(fixture_event(json!({"n": n}), None))?;
            }
            Ok(())
        })
    }

    fn record(&self, request: &str, count: usize) -> Head {
        self.record_at(request, count, T0)
    }

    /// One fixture command whose decision runs `body` and succeeds.
    fn write(
        &self,
        request: &str,
        at: &str,
        mut body: impl FnMut(&mut dyn Transaction) -> Result<(), StoreError>,
    ) -> Head {
        match self
            .store
            .transact(&fixture_command(request, at), &mut |tx| {
                body(tx)?;
                Ok(done())
            })
            .expect("write")
        {
            Recorded::New { head, .. } => head,
            Recorded::Replayed { .. } => panic!("a fresh request replayed"),
        }
    }

    fn events_of(&self, type_name: &str) -> Vec<Event> {
        self.ledger()
            .history(
                &project(),
                1..=u64::MAX,
                &HistoryFilter {
                    types: vec![type_name.into()],
                    git_commit: None,
                },
                PageRequest {
                    limit: 100,
                    after: None,
                },
            )
            .expect("history")
            .items
    }

    fn head(&self) -> Head {
        self.ledger()
            .head(&project())
            .expect("head")
            .expect("a head")
    }

    fn request_state(&self, kind: &str, request: &str) -> (String, String) {
        let body = self
            .store
            .get(
                &project(),
                REQUEST_VIEW,
                &DocKey(vec![
                    KeyValue::Text(kind.into()),
                    KeyValue::Text(request.into()),
                ]),
            )
            .expect("request")
            .expect("present")
            .body;
        let text = |field: &str| body[field].as_str().unwrap_or_default().to_owned();
        (text("state"), text("outcome"))
    }

    fn anchor_rows(&self) -> Vec<(i64, Vec<u8>, String, String)> {
        let conn = self.raw();
        let mut statement = conn
            .prepare("SELECT seq, head_hash, tag, pushed_at FROM anchor ORDER BY seq")
            .expect("prepare");
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .expect("rows")
            .collect::<rusqlite::Result<_>>()
            .expect("rows")
    }

    fn insert_anchor_row(&self, seq: u64, hash: Hash, tag: &str) {
        self.raw()
            .execute(
                "INSERT INTO anchor (project_id, seq, head_hash, tag, pushed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![PROJECT, seq as i64, &hash.0[..], tag, T0],
            )
            .expect("row");
    }

    fn attach(&self, request: &str, bytes: &[u8], class: RetentionClass) -> PayloadReference {
        let mut reference = None;
        self.write(request, T0, |tx| {
            let body = tx.put_payload(bytes, class)?;
            let seq = tx.append(NewEvent {
                attachments: vec![body.clone()],
                ..fixture_event(json!({"body": body.to_value()}), None)
            })?;
            reference = Some(PayloadReference {
                project: project(),
                seq,
                hash: body.hash,
            });
            Ok(())
        });
        reference.expect("attached")
    }

    /// Replaces a stored body with other bytes, its events untouched.
    fn overwrite_body(&self, hash: &Hash, compressed: &[u8]) {
        self.raw()
            .execute(
                "UPDATE payload SET body = ?1 WHERE hash = ?2",
                params![compressed, &hash.0[..]],
            )
            .expect("overwrite");
    }
}

fn open(home: &Path, schema: Box<dyn EventSchema>) -> SqliteStore {
    SqliteStore::open(
        home,
        T0,
        Options {
            schema,
            timing: Scripted::still(),
            ..Options::default()
        },
    )
    .expect("open")
}

fn fixture_event(payload: Value, git: Option<GitFacts>) -> NewEvent {
    NewEvent {
        stream: StreamName("fixture".into()),
        type_name: RECORDED.into(),
        type_version: 1,
        git,
        payload,
        attachments: Vec::new(),
    }
}

fn fixture_command(request: &str, at: &str) -> Command {
    Command {
        project: project(),
        kind: CommandKind("fixture.write".into()),
        request_id: RequestId(request.into()),
        digest: Hash([3; 32]),
        scope: Vec::new(),
        policy_version: 1,
        recorded_at: at.into(),
        actor: Actor::Owner,
        caller: None,
    }
}

fn done() -> Decision {
    Decision {
        kind: OutcomeKind::Done,
        answer: json!("ok"),
        sensitive: false,
        observed: Observed::default(),
        git: None,
    }
}

fn page(limit: u32) -> PageRequest {
    PageRequest { limit, after: None }
}

fn owner() -> ClaimOwner {
    ClaimOwner {
        process: "p1".into(),
        host_session: "h1".into(),
        started_at: T0.into(),
    }
}

fn anchor_request(request: &str, remote: Option<&str>) -> AnchorRequest {
    AnchorRequest {
        project: project(),
        request_id: RequestId(request.into()),
        reconcile_request_id: RequestId(format!("{request}-reconcile")),
        actor: Actor::Owner,
        owner: owner(),
        remote: remote.map(str::to_owned),
        policy_version: 1,
    }
}

/// The core's trace seam over this store's trace table.
struct Trace<'a>(&'a SqliteStore);

impl TraceSink for Trace<'_> {
    fn record(&self, record: TraceRecord) -> Result<(), StoreError> {
        self.0.record_trace(&TraceEntry {
            at: record.at,
            project: record.project,
            payload: record.payload,
            kind: record.kind,
            data: record.data,
        })
    }
}

fn act(f: &Fixture, request: &AnchorRequest, at: &str) -> baley_core::AnchorAct {
    match claim_step(f.ledger(), request, at).expect("claim") {
        ClaimStep::Act(act) => act,
        other => panic!("expected an act, got {other:?}"),
    }
}

/// The present tag a matching remote holds for `act`.
fn matching_tag(act: &baley_core::AnchorAct) -> FetchObservation {
    FetchObservation::Present {
        tag: act.target.tag.clone(),
        annotation: act.annotation.clone(),
    }
}

/// A remote anchor observation for `anchor` of this project.
fn remote_anchor(anchor: &Anchor) -> FetchObservation {
    FetchObservation::Present {
        tag: anchor_tag(&project(), anchor.seq),
        annotation: anchor_annotation(anchor),
    }
}

fn head_anchor(head: &Head) -> Anchor {
    Anchor {
        seq: head.seq,
        hash: head.hash,
    }
}

/// An anchor claim left interrupted, and a second request blocked by it.
struct Interrupted {
    first: AnchorRequest,
    first_act: baley_core::AnchorAct,
    second: AnchorRequest,
    held: HeldAnchor,
}

fn interrupted(f: &Fixture) -> Interrupted {
    f.record("w1", 3);
    let first = anchor_request("a1", Some("origin"));
    let first_act = act(f, &first, T0);
    let second = anchor_request("b1", Some("origin"));
    let ClaimStep::Blocked(block) = claim_step(f.ledger(), &second, EXPIRED).expect("claim") else {
        panic!("expected the second request to be blocked");
    };
    assert_eq!(block.state, ClaimState::Interrupted);
    let holder = f
        .ledger()
        .open_claims(&project())
        .expect("open claims")
        .into_iter()
        .find(|claim| claim.id == block.claim)
        .expect("the holder");
    let held = HeldAnchor::from_claim(&project(), &holder).expect("an anchor claim");
    Interrupted {
        first,
        first_act,
        second,
        held,
    }
}

/// One trace row: time, project, payload hash, kind and data.
type TraceRow = (String, Option<String>, Option<Vec<u8>>, String, String);

fn reconcile(f: &Fixture, setup: &Interrupted, observation: &FetchObservation) -> ReconcileStep {
    reconcile_from_observation(
        f.ledger(),
        &Trace(&f.store),
        &setup.second,
        &setup.held,
        observation,
        CHECKED,
    )
    .expect("reconcile")
}

// --- stream, history and head ---

// A stream reads from its starting version in version order, one bounded
// page at a time, and the cursor continues where the page ended. Catches
// an unbounded read and a page that repeats or skips an event.
#[test]
fn a_stream_pages_in_version_order_from_its_start() {
    let f = Fixture::new();
    f.record("w1", 4);
    let stream = StreamName("fixture".into());
    let first = f
        .ledger()
        .stream(&project(), &stream, 2, page(2))
        .expect("first page");
    let versions = |events: &[Event]| -> Vec<u64> {
        events.iter().map(|event| event.stream_version).collect()
    };
    assert_eq!(versions(&first.items), [2, 3]);
    let second = f
        .ledger()
        .stream(
            &project(),
            &stream,
            2,
            PageRequest {
                limit: 2,
                after: first.next,
            },
        )
        .expect("second page");
    assert_eq!(versions(&second.items), [4]);
    assert_eq!(second.next, None);
}

// A page never holds more than 100 events, whatever the limit asks.
// Catches a caller-sized read of the whole history.
#[test]
fn a_page_holds_at_most_one_hundred_events() {
    let f = Fixture::new();
    f.record("w1", 120);
    let found = f
        .ledger()
        .history(
            &project(),
            1..=u64::MAX,
            &HistoryFilter::default(),
            page(500),
        )
        .expect("history");
    assert_eq!(found.items.len(), 100);
    assert!(found.next.is_some());
}

// A cursor works only for the query that issued it: another stream start,
// or a history query, refuses it. Catches cursor reuse across queries.
#[test]
fn a_cursor_is_refused_by_another_query() {
    let f = Fixture::new();
    f.record("w1", 3);
    let stream = StreamName("fixture".into());
    let cursor = f
        .ledger()
        .stream(&project(), &stream, 1, page(1))
        .expect("page")
        .next;
    assert!(cursor.is_some());
    let again = PageRequest {
        limit: 1,
        after: cursor,
    };
    assert_eq!(
        f.ledger().stream(&project(), &stream, 2, again.clone()),
        Err(StoreError::Refused(Refusal::InvalidCursor))
    );
    assert_eq!(
        f.ledger()
            .history(&project(), 1..=10, &HistoryFilter::default(), again),
        Err(StoreError::Refused(Refusal::InvalidCursor))
    );
}

// History is in sequence order within its range, keeps any of the named
// types, and of those only the ones that recorded the named commit.
// Catches types combined as AND, the commit ignored, or the range ignored.
#[test]
fn history_filters_by_range_types_and_commit() {
    let f = Fixture::new();
    let git = |commit: &str| {
        Some(GitFacts {
            commit: commit.into(),
            tree: "t".into(),
            checkout: "/c".into(),
        })
    };
    f.write("w1", T0, |tx| {
        tx.append(fixture_event(json!({"n": 1}), git("c1")))?;
        tx.append(fixture_event(json!({"n": 2}), git("c2")))?;
        tx.append(fixture_event(json!({"n": 3}), git("c1")))?;
        Ok(())
    });
    let seqs = |filter: HistoryFilter, range| -> Vec<u64> {
        f.ledger()
            .history(&project(), range, &filter, page(100))
            .expect("history")
            .items
            .iter()
            .map(|event| event.seq)
            .collect()
    };
    let both = vec![RECORDED.to_owned(), COMMAND_COMPLETED.to_owned()];
    assert_eq!(
        seqs(
            HistoryFilter {
                types: both.clone(),
                git_commit: None
            },
            1..=u64::MAX
        ),
        [1, 2, 3, 4]
    );
    assert_eq!(
        seqs(
            HistoryFilter {
                types: both,
                git_commit: Some("c1".into())
            },
            1..=u64::MAX
        ),
        [1, 3]
    );
    assert_eq!(seqs(HistoryFilter::default(), 2..=3), [2, 3]);
}

// History hands back the stored event, not the upcast one the projectors
// see. Catches a read that rewrites the envelope's version or payload.
#[test]
fn history_returns_the_stored_version() {
    fn v1_to_v2(mut payload: Value) -> Result<Value, baley_core::UpcastError> {
        payload["upcast"] = json!(true);
        Ok(payload)
    }
    let mut schema = Registry::new();
    schema
        .register(RECORDED, 2, [(1, v1_to_v2 as Upcaster)])
        .expect("register");
    let f = Fixture::with_schema(Box::new(schema));
    f.record("w1", 1);
    let stored = &f.events_of(RECORDED)[0];
    assert_eq!(stored.type_version, 1);
    assert_eq!(stored.payload, json!({"n": 0}));
}

// A project a binary may not write to is still readable through history.
// Catches reads gated by the view or type fences commands use.
#[test]
fn history_reads_a_project_this_binary_cannot_write() {
    let f = Fixture::new();
    f.record("w1", 1);
    let older = open(f.home.path(), Box::new(Registry::new()));
    assert!(matches!(
        older.transact(&fixture_command("w2", T0), &mut |_| Ok(done())),
        Err(StoreError::Refused(Refusal::ProjectReadOnly { .. }))
    ));
    let read = older
        .history(
            &project(),
            1..=u64::MAX,
            &HistoryFilter::default(),
            page(10),
        )
        .expect("history");
    assert_eq!(read.items.len(), 2);
}

// An existing project with no events has no head; an absent project is
// unknown. Catches empty treated as missing, or missing as empty.
#[test]
fn head_is_none_for_an_empty_project_and_refused_for_an_absent_one() {
    let f = Fixture::new();
    assert_eq!(f.ledger().head(&project()), Ok(None));
    assert_eq!(
        f.ledger().head(&ProjectId("absent".into())),
        Err(StoreError::Refused(Refusal::UnknownProject(ProjectId(
            "absent".into()
        ))))
    );
}

// --- verification ---

// Verification only reads: with the database file gone it fails and
// leaves no file behind. Catches a verify connection opened read-write,
// which creates an empty baley.db.
#[test]
fn verify_without_a_database_file_fails_and_creates_none() {
    let f = Fixture::new();
    let path = f.home.path().join("baley.db");
    std::fs::remove_file(&path).expect("remove");
    assert!(f.ledger().verify(&project(), None).is_err());
    assert!(!path.exists());
}

// Catches malformed compressed bytes escaping body fault reporting.
#[test]
fn an_undecodable_body_is_corrupt() {
    let f = Fixture::new();
    let body = f.attach("w1", b"the test output", RetentionClass::Output);
    f.overwrite_body(&body.hash, b"not zstd at all");
    let report = f.ledger().verify(&project(), None).expect("verify");
    assert!(report.chain.is_intact());
    assert_eq!(report.payloads, [PayloadFault::Corrupt(body.hash)]);
}

// --- anchor rows ---

// A decision that records an anchor row without the matching
// anchor.pushed event in its command is refused, and nothing is written.
// Catches a row with no evidence behind it.
#[test]
fn an_anchor_row_needs_its_event_in_the_same_command() {
    let f = Fixture::new();
    let head = f.record("w1", 2);
    let anchor = head_anchor(&head);
    let result = f.store.transact(&fixture_command("w2", T1), &mut |tx| {
        tx.append(fixture_event(json!({"n": 9}), None))?;
        tx.record_anchor(&anchor, &anchor_tag(&project(), anchor.seq), "origin", T1)?;
        Ok(done())
    });
    assert!(matches!(
        result,
        Err(StoreError::Refused(Refusal::InvalidEvent(_)))
    ));
    assert_eq!(f.anchor_rows(), []);
    assert_eq!(f.head(), head);
}

// --- the anchor command's steps ---

// The claim step answers with the tag only once the claim event is in the
// chain, and the tag and annotation name the pre-claim head, not the claim
// event. A supplied successful push then records as done. Catches acting
// before the claim and anchoring the claim event.
#[test]
fn the_claim_step_acts_only_after_its_claim_is_recorded() {
    let f = Fixture::new();
    let before = f.record("w1", 3);
    let request = anchor_request("a1", Some("origin"));
    let claimed = act(&f, &request, T0);
    let claims = f.events_of(COMMAND_CLAIMED);
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].seq, claimed.claim_seq);
    assert_eq!(claimed.claim_seq, before.seq + 1);
    assert_eq!(claimed.target.tag, anchor_tag(&project(), before.seq));
    assert_eq!(claimed.annotation, anchor_annotation(&head_anchor(&before)));
    let recorded = record_step(
        f.ledger(),
        &request,
        &claimed,
        &PushObservation::Pushed,
        T1,
        T2,
    )
    .expect("record");
    assert!(matches!(recorded, Recorded::New { outcome, .. } if outcome.kind == OutcomeKind::Done));
}

// A second run of a claimed request is in progress and a completed one
// replays; neither acts. Catches a duplicate push.
#[test]
fn in_progress_and_replayed_requests_do_not_act() {
    let f = Fixture::new();
    f.record("w1", 1);
    let request = anchor_request("a1", Some("origin"));
    let claimed = act(&f, &request, T0);
    assert!(matches!(
        claim_step(f.ledger(), &request, T1),
        Ok(ClaimStep::InProgress(_))
    ));
    record_step(
        f.ledger(),
        &request,
        &claimed,
        &PushObservation::Pushed,
        T1,
        T1,
    )
    .expect("record");
    assert!(matches!(
        claim_step(f.ledger(), &request, T2),
        Ok(ClaimStep::Replayed(outcome)) if outcome.kind == OutcomeKind::Done
    ));
}

// A request meeting another request's live claim is blocked and names that
// holder. Catches a second anchor claimed beside the first.
#[test]
fn a_blocked_request_names_its_holder() {
    let f = Fixture::new();
    f.record("w1", 1);
    act(&f, &anchor_request("a1", Some("origin")), T0);
    let Ok(ClaimStep::Blocked(block)) =
        claim_step(f.ledger(), &anchor_request("b1", Some("origin")), T1)
    else {
        panic!("expected a block");
    };
    assert_eq!(block.claim.request_id, RequestId("a1".into()));
    assert_eq!(block.state, ClaimState::Active);
}

// Building the same request again after its claim gives the same digest,
// so the claim is found in progress. Catches a head-dependent digest.
#[test]
fn the_same_request_keeps_its_digest_after_the_head_moves() {
    let f = Fixture::new();
    f.record("w1", 1);
    let request = anchor_request("a1", Some("origin"));
    act(&f, &request, T0);
    assert!(matches!(
        claim_step(f.ledger(), &request, T1),
        Ok(ClaimStep::InProgress(_))
    ));
}

// No configured remote refuses in the claim decision: the refusal is
// recorded and no claim event or lease exists. Catches a lease taken for
// an impossible push.
#[test]
fn no_configured_remote_refuses_without_a_claim() {
    let f = Fixture::new();
    f.record("w1", 1);
    let result = claim_step(f.ledger(), &anchor_request("a1", None), T0).expect("claim");
    assert!(
        matches!(result, ClaimStep::Refused { outcome, .. } if outcome.kind == OutcomeKind::Refused)
    );
    assert_eq!(f.events_of(COMMAND_CLAIMED), []);
    assert_eq!(f.ledger().open_claims(&project()), Ok(Vec::new()));
}

/// Claims, then records `observation`, and returns the fixture.
fn record_observation(observation: PushObservation) -> Fixture {
    let f = Fixture::new();
    f.record("w1", 1);
    let request = anchor_request("a1", Some("origin"));
    let claimed = act(&f, &request, T0);
    let recorded =
        record_step(f.ledger(), &request, &claimed, &observation, T1, T2).expect("record");
    assert!(
        matches!(recorded, Recorded::New { outcome, .. } if outcome.kind == OutcomeKind::Refused)
    );
    f
}

// A push the remote refused completes the claim as refused with
// anchor.failed and no row. Catches a clean failure left open.
#[test]
fn a_refused_push_completes_the_claim_with_anchor_failed() {
    let f = record_observation(PushObservation::Refused {
        reason: "ruleset".into(),
    });
    let failed = f.events_of(ANCHOR_FAILED);
    assert_eq!(failed.len(), 1);
    assert_eq!(
        failed[0].payload["reason"],
        json!("the remote refused the push: ruleset")
    );
    assert_eq!(f.request_state("anchor.push", "a1").0, "completed");
    assert_eq!(f.anchor_rows(), []);
}

// An unreachable remote at push completes the claim the same way. Catches
// an unreachable push left as an interrupted claim.
#[test]
fn an_unreachable_push_completes_the_claim_with_anchor_failed() {
    let f = record_observation(PushObservation::Unreachable);
    assert_eq!(f.events_of(ANCHOR_FAILED).len(), 1);
    assert_eq!(f.ledger().open_claims(&project()), Ok(Vec::new()));
}

// A confirmed push commits anchor.pushed, the completion and the row
// together, the row at the observed time. Catches a row committed apart
// from its evidence.
#[test]
fn a_successful_record_step_writes_the_event_completion_and_row() {
    let f = Fixture::new();
    let before = f.record("w1", 2);
    let request = anchor_request("a1", Some("origin"));
    let claimed = act(&f, &request, T0);
    record_step(
        f.ledger(),
        &request,
        &claimed,
        &PushObservation::Pushed,
        T1,
        T2,
    )
    .expect("record");
    let pushed = f.events_of(ANCHOR_PUSHED);
    assert_eq!(pushed.len(), 1);
    assert_eq!(
        pushed[0].payload,
        json!({"tag": anchor_tag(&project(), before.seq), "seq": before.seq,
            "head": before.hash.to_hex(), "remote": "origin", "observed_at": T1})
    );
    assert_eq!(
        f.request_state("anchor.push", "a1"),
        ("completed".into(), "done".into())
    );
    assert_eq!(
        f.anchor_rows(),
        [(
            before.seq as i64,
            before.hash.0.to_vec(),
            anchor_tag(&project(), before.seq),
            T1.to_owned()
        )]
    );
}

// A row already stored for the sequence with another hash refuses the
// record step: no result event, no completion, and the stored row stands.
// Catches a partial record step.
#[test]
fn a_conflicting_row_leaves_the_record_step_unwritten() {
    let f = Fixture::new();
    let before = f.record("w1", 2);
    let request = anchor_request("a1", Some("origin"));
    let claimed = act(&f, &request, T0);
    let tag = anchor_tag(&project(), before.seq);
    f.insert_anchor_row(before.seq, Hash([9; 32]), &tag);
    let head = f.head();
    let result = record_step(
        f.ledger(),
        &request,
        &claimed,
        &PushObservation::Pushed,
        T1,
        T2,
    );
    assert!(matches!(
        result,
        Err(StoreError::Refused(Refusal::InvalidEvent(_)))
    ));
    assert_eq!(f.events_of(ANCHOR_PUSHED), []);
    assert_eq!(f.head(), head);
    assert_eq!(f.anchor_rows()[0].1, vec![9u8; 32]);
    assert_eq!(f.ledger().open_claims(&project()).expect("claims").len(), 1);
}

// With a remote anchor at 60 and a local chain ending at 50, the check
// refuses; regrown with other events through 61, it still refuses.
// Catches a rolled-back chain re-anchored once it grows past the anchor.
#[test]
fn a_rolled_back_chain_is_refused_before_the_push() {
    let f = Fixture::new();
    f.record("w1", 49);
    assert_eq!(f.head().seq, 50);
    let witness = remote_anchor(&Anchor {
        seq: 60,
        hash: Hash([7; 32]),
    });
    let short = pre_push_check(f.ledger(), &project(), &witness).expect("check");
    assert!(
        matches!(short, PrePushCheck::Refuse { reason, .. } if reason.contains("ends at sequence 50"))
    );
    f.record("w2", 10);
    assert_eq!(f.head().seq, 61);
    let regrown = pre_push_check(f.ledger(), &project(), &witness).expect("check");
    assert!(
        matches!(regrown, PrePushCheck::Refuse { reason, .. } if reason.contains("differs from the remote anchor at sequence 60"))
    );
}

// A remote anchor the local chain agrees with lets the push go ahead.
// Catches a check that refuses every anchored project.
#[test]
fn a_chain_that_agrees_with_the_remote_may_push() {
    let f = Fixture::new();
    let head = f.record("w1", 3);
    f.record("w2", 1);
    let check =
        pre_push_check(f.ledger(), &project(), &remote_anchor(&head_anchor(&head))).expect("check");
    assert_eq!(
        check,
        PrePushCheck::Push {
            payload_faults: Vec::new()
        }
    );
}

// --- reconciliation from the command ---

// A reconciliation fetch that could not reach the remote writes one trace
// row, at the check time, naming the project and the claim's kind, request
// id and sequence, with no payload; the head does not move and the claim
// stays open. Catches the trace call omitted.
#[test]
fn an_unreachable_reconciliation_writes_one_trace_row() {
    let f = Fixture::new();
    let setup = interrupted(&f);
    let head = f.head();
    assert_eq!(
        reconcile(&f, &setup, &FetchObservation::Unreachable),
        ReconcileStep::Unknown
    );
    let conn = f.raw();
    let rows: Vec<TraceRow> = conn
        .prepare("SELECT at, project_id, payload_hash, kind, data FROM trace")
        .expect("prepare")
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .expect("rows")
        .collect::<rusqlite::Result<_>>()
        .expect("rows");
    assert_eq!(
        rows,
        [(
            CHECKED.to_owned(),
            Some(PROJECT.to_owned()),
            None,
            "claim.unreachable".to_owned(),
            format!(
                r#"{{"claim_seq":{},"kind":"anchor.push","remote":"unreachable","request_id":"a1"}}"#,
                setup.first_act.claim_seq
            )
        )]
    );
    assert_eq!(f.head(), head);
    let open = f.ledger().open_claims(&project()).expect("claims");
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id.request_id, setup.first.request_id);
}

// A matching holder tag completes the holder as done under a reconciler
// attributed to Baley. Catches a landed push recorded as anything but
// done, or a reconciliation written as the owner's.
#[test]
fn a_matching_holder_tag_reconciles_as_done() {
    let f = Fixture::new();
    let setup = interrupted(&f);
    let step = reconcile(&f, &setup, &matching_tag(&setup.first_act));
    assert!(matches!(step, ReconcileStep::Retry(Recorded::New { .. })));
    assert_eq!(
        f.request_state("anchor.push", "a1"),
        ("completed".into(), "done".into())
    );
    let reconciled = f.events_of(COMMAND_RECONCILED);
    assert_eq!(reconciled.len(), 1);
    assert_eq!(reconciled[0].actor, Actor::Baley);
}

// A holder tag naming another head completes the holder as refused with
// anchor.failed. Catches existence taken for agreement.
#[test]
fn a_conflicting_holder_tag_reconciles_as_refused() {
    let f = Fixture::new();
    let setup = interrupted(&f);
    let other = FetchObservation::Present {
        tag: setup.held.target.tag.clone(),
        annotation: anchor_annotation(&Anchor {
            seq: setup.held.target.intent.seq,
            hash: Hash([9; 32]),
        }),
    };
    assert!(matches!(
        reconcile(&f, &setup, &other),
        ReconcileStep::Retry(_)
    ));
    assert_eq!(
        f.request_state("anchor.push", "a1"),
        ("completed".into(), "refused".into())
    );
    assert_eq!(f.events_of(ANCHOR_FAILED).len(), 1);
    assert_eq!(f.anchor_rows(), []);
}

// A holder tag the remote confirms absent completes the holder as not
// pushed. Catches absence left unrecorded.
#[test]
fn an_absent_holder_tag_reconciles_as_not_pushed() {
    let f = Fixture::new();
    let setup = interrupted(&f);
    assert!(matches!(
        reconcile(&f, &setup, &FetchObservation::Absent),
        ReconcileStep::Retry(_)
    ));
    assert_eq!(
        f.request_state("anchor.push", "a1"),
        ("completed".into(), "refused".into())
    );
}

/// A remote that holds no tags: every fetch finds none and every push
/// lands.
struct EmptyRemote;

impl Forge for EmptyRemote {
    fn push_tag(&mut self, _: &ProjectId, _: &str, _: &str, _: &str) -> PushObservation {
        PushObservation::Pushed
    }

    fn fetch_tag(&mut self, _: &ProjectId, _: &str, _: &TagQuery) -> FetchObservation {
        FetchObservation::Absent
    }
}

/// A ticker that never ticks.
struct Still;

impl Ticker for Still {
    fn start(&mut self, _: u64, _: Box<dyn FnMut(String) + Send + 'static>) -> Box<dyn TickGuard> {
        Box::new(Still)
    }
}

impl TickGuard for Still {
    fn stop(self: Box<Self>) {}
}

// Once the command has reconciled an interrupted holder, it retries the
// blocked request under its own identity, digest and scope, at the clock's
// time, and the retried claim reads the head as it then stands. Catches the
// command stopping after reconciliation instead of retrying, or retrying
// under another identity or a changed digest.
#[test]
fn the_anchor_command_retries_its_request_after_reconciling() {
    let f = Fixture::new();
    let setup = interrupted(&f);
    anchor_command(
        &setup.second,
        AnchorSeams {
            ledger: f.store.clone(),
            forge: &mut EmptyRemote,
            ticker: &mut Still,
            trace: &Trace(&f.store),
            now: &mut || RETRY.to_owned(),
        },
    )
    .expect("anchor");
    let claim = f
        .events_of(COMMAND_CLAIMED)
        .pop()
        .expect("the retried claim");
    assert_eq!(claim.request_id, setup.second.request_id);
    assert_eq!(claim.actor, Actor::Owner);
    assert_eq!(claim.policy_version, 1);
    assert_eq!(claim.recorded_at, RETRY);
    assert_eq!(
        claim.payload["digest"],
        json!(setup.second.digest().expect("digest").to_hex())
    );
    assert_eq!(claim.payload["scope"], json!(["anchor"]));
    assert_eq!(claim.payload["intent"]["seq"], json!(claim.seq - 1));
}

// A matching tag found by reconciliation writes the anchor row at the
// check time. Catches a push that landed before its record step lost from
// the local record.
#[test]
fn a_reconciled_matching_tag_writes_the_row_at_the_check_time() {
    let f = Fixture::new();
    let setup = interrupted(&f);
    reconcile(&f, &setup, &matching_tag(&setup.first_act));
    let intent = &setup.held.target.intent;
    assert_eq!(
        f.anchor_rows(),
        [(
            intent.seq as i64,
            intent.head.0.to_vec(),
            setup.held.target.tag.clone(),
            CHECKED.to_owned()
        )]
    );
}

// The holder's own record step, arriving after reconciliation closed its
// claim as not pushed, gets that outcome back and writes nothing, though
// its push reported success. Catches a late record rewriting the outcome.
#[test]
fn a_late_record_step_replays_the_reconciled_outcome() {
    let f = Fixture::new();
    let setup = interrupted(&f);
    reconcile(&f, &setup, &FetchObservation::Absent);
    let head = f.head();
    let late = record_step(
        f.ledger(),
        &setup.first,
        &setup.first_act,
        &PushObservation::Pushed,
        RETRY,
        RETRY,
    )
    .expect("late record");
    assert!(matches!(late, Recorded::Replayed { outcome } if outcome.kind == OutcomeKind::Refused));
    assert_eq!(f.head(), head);
    assert_eq!(f.anchor_rows(), []);
}

// --- verification through the core ---

// Catches a local row replacing the remote observation status.
#[test]
fn a_remote_anchor_observation_retains_its_status() {
    let f = Fixture::new();
    f.record("w1", 2);
    let request = anchor_request("a1", Some("origin"));
    let claimed = act(&f, &request, T0);
    record_step(
        f.ledger(),
        &request,
        &claimed,
        &PushObservation::Pushed,
        T1,
        T1,
    )
    .expect("record");
    let later = f.record("w2", 2);
    let verified = verify_observed(
        f.ledger(),
        &project(),
        &remote_anchor(&head_anchor(&later)),
        T2,
    )
    .expect("verify");
    assert_eq!(verified.status, AnchorCheck::Remote(head_anchor(&later)));
}

// Catches the stored tag omitted from anchor comparison.
#[test]
fn a_same_sequence_row_with_another_tag_is_a_conflict() {
    let f = Fixture::new();
    let head = f.record("w1", 3);
    f.insert_anchor_row(head.seq, head.hash, "baley-anchor/x/4");
    let report = f
        .ledger()
        .verify(&project(), Some(&head_anchor(&head)))
        .expect("verify");
    assert_eq!(
        report.stored_anchor_comparison,
        StoredAnchorComparison::Conflict
    );
}

// No remote gives an explicitly local-only report: the whole chain is
// checked, no anchor is compared, and a local row is not used as one.
// Catches the local row silently trusted.
#[test]
fn no_remote_gives_a_local_only_report() {
    let f = Fixture::new();
    let head = f.record("w1", 2);
    f.insert_anchor_row(head.seq, head.hash, &anchor_tag(&project(), head.seq));
    let verified =
        verify_observed(f.ledger(), &project(), &FetchObservation::NoRemote, T1).expect("verify");
    assert_eq!(verified.status, AnchorCheck::LocalOnly);
    assert_eq!(verified.report.chain.anchor, AnchorVerdict::NoAnchor);
    assert_eq!(verified.report.chain.unanchored, Some(1..=head.seq));
    assert_eq!(
        verified.report.stored_anchor_comparison,
        StoredAnchorComparison::NotCompared
    );
}

// A confirmed absence is reported as such even with a local row claiming
// a push. Catches the row put in place of a missing remote tag.
#[test]
fn a_remote_absence_is_reported_despite_a_local_row() {
    let f = Fixture::new();
    let head = f.record("w1", 2);
    f.insert_anchor_row(head.seq, head.hash, &anchor_tag(&project(), head.seq));
    let verified =
        verify_observed(f.ledger(), &project(), &FetchObservation::Absent, T1).expect("verify");
    assert_eq!(verified.status, AnchorCheck::RemoteAbsent);
    assert_eq!(verified.report.chain.anchor, AnchorVerdict::NoAnchor);
}

// An unreachable remote is reported as such, with no anchor compared.
// Catches an unchecked remote treated as verified.
#[test]
fn an_unreachable_remote_leaves_verification_unanchored() {
    let f = Fixture::new();
    f.record("w1", 2);
    let verified = verify_observed(f.ledger(), &project(), &FetchObservation::Unreachable, T1)
        .expect("verify");
    assert_eq!(verified.status, AnchorCheck::RemoteUnreachable);
    assert_eq!(verified.report.chain.anchor, AnchorVerdict::NoAnchor);
}

// --- the unanchored age ---

/// Anchors the current head through the claim and record steps.
fn anchor_now(f: &Fixture, request: &str, at: &str) -> Anchor {
    let request = anchor_request(request, Some("origin"));
    let claimed = act(f, &request, at);
    record_step(
        f.ledger(),
        &request,
        &claimed,
        &PushObservation::Pushed,
        at,
        at,
    )
    .expect("record");
    claimed_anchor(&claimed)
}

fn claimed_anchor(claimed: &baley_core::AnchorAct) -> Anchor {
    Anchor {
        seq: claimed.target.intent.seq,
        hash: claimed.target.intent.head,
    }
}

// Right after an anchor, its claim, result and completion are unanchored
// but start no age. Catches a one-day warning on an idle project.
#[test]
fn a_fresh_anchor_starts_no_unanchored_age() {
    let f = Fixture::new();
    f.record("w1", 3);
    let anchor = anchor_now(&f, "a1", T1);
    let report = f
        .ledger()
        .verify(&project(), Some(&anchor))
        .expect("verify");
    assert_eq!(
        report.chain.unanchored,
        Some(anchor.seq + 1..=anchor.seq + 3)
    );
    assert_eq!(report.chain.age_unanchored_since, None);
}

// After an interrupted anchor is reconciled and the retried push fails,
// the reconciliation and both completions start no age either. Catches a
// warning raised by the anchor command's own bookkeeping.
#[test]
fn reconciliation_and_a_failed_retry_start_no_unanchored_age() {
    let f = Fixture::new();
    let setup = interrupted(&f);
    // The project's own work ends before the first claim.
    let witness = claimed_anchor(&setup.first_act);
    reconcile(&f, &setup, &FetchObservation::Absent);
    let retried = act(&f, &setup.second, RETRY);
    record_step(
        f.ledger(),
        &setup.second,
        &retried,
        &PushObservation::Unreachable,
        RETRY,
        RETRY,
    )
    .expect("record");
    let report = f
        .ledger()
        .verify(&project(), Some(&witness))
        .expect("verify");
    assert_eq!(report.chain.anchor, AnchorVerdict::Matches);
    assert!(report.chain.unanchored.is_some());
    assert_eq!(report.chain.age_unanchored_since, None);
}

// Project work after an anchor starts the age at its own recorded time,
// though anchor command events come before and after it. Catches an age
// hidden by the exclusion, or taken from an anchor event.
#[test]
fn project_work_starts_the_age_at_its_own_time() {
    let f = Fixture::new();
    f.record("w1", 1);
    let first = anchor_now(&f, "a1", T0);
    f.record_at("w2", 1, T1);
    anchor_now(&f, "a2", T2);
    let report = f.ledger().verify(&project(), Some(&first)).expect("verify");
    assert_eq!(report.chain.age_unanchored_since.as_deref(), Some(T1));
}

// Catches another project's raw rows left in an export.
#[test]
fn an_export_holds_no_row_of_another_project() {
    let f = Fixture::new();
    let other = ProjectId("other".into());
    f.store
        .create_project(&other, "Other", T0)
        .expect("other project");
    f.record("one", 1);
    let mut other_command = fixture_command("other-work", T0);
    other_command.project = other.clone();
    let mut other_hash = None;
    f.store
        .transact(&other_command, &mut |tx| {
            let body = tx.put_payload(b"other project's body", RetentionClass::Material)?;
            other_hash = Some(body.hash);
            tx.append(NewEvent {
                attachments: vec![body.clone()],
                ..fixture_event(json!({"body": body.to_value()}), None)
            })?;
            Ok(done())
        })
        .expect("other work");
    let other_hash = other_hash.expect("other payload");
    let target = f.home.path().join("export");
    f.store.export(&project(), &target, T1).expect("export");
    let exported_db = Connection::open(target.join("baley.db")).expect("exported database");
    for table in ["event", "payload_ref"] {
        let rows: i64 = exported_db
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE project_id = ?1"),
                [&other.0],
                |row| row.get(0),
            )
            .expect("other rows");
        assert_eq!(rows, 0, "{table}");
    }
    let bodies: i64 = exported_db
        .query_row(
            "SELECT count(*) FROM payload WHERE hash = ?1",
            [&other_hash.0[..]],
            |row| row.get(0),
        )
        .expect("other payload");
    assert_eq!(bodies, 0);
}

// Catches an export mixing two source snapshots after a command commits.
#[test]
fn export_keeps_the_snapshot_fixed_after_its_first_read() {
    let f = Fixture::new();
    let before = f.record("before", 1);
    let target = f.home.path().join("export");
    let exported = f
        .store
        .export_with(&project(), &target, T1, || {
            f.record("after", 1);
        })
        .expect("export");
    assert_eq!(exported.head, Some(before));
    let copy = SqliteStore::open(
        &target,
        T1,
        Options {
            schema: Box::new(registry()),
            ..Options::default()
        },
    )
    .expect("open copy");
    assert!(
        copy.verify(&project(), None)
            .expect("verify")
            .chain
            .first_break
            .is_none()
    );
}

// Catches an existing directory overwritten or recorded as an export.
#[test]
fn existing_export_target_is_refused_without_a_record() {
    let f = Fixture::new();
    let target = f.home.path().join("export");
    std::fs::create_dir(&target).expect("existing");
    assert_eq!(
        f.store.export(&project(), &target, T1),
        Err(StoreError::Refused(Refusal::TargetExists(target.clone())))
    );
    let records: i64 = f
        .raw()
        .query_row("SELECT count(*) FROM export_record", [], |row| row.get(0))
        .expect("records");
    assert_eq!(records, 0);
    assert!(target.is_dir());
}

// Catches a corrupt local chain reported as a successful export.
#[test]
fn export_refuses_a_copy_with_an_edited_event() {
    let f = Fixture::new();
    f.record("one", 1);
    f.raw()
        .execute(
            "UPDATE event SET payload_json = '{\"edited\":true}' WHERE project_id = ?1 AND seq = 1",
            [&project().0],
        )
        .expect("edit");
    let target = f.home.path().join("export");
    assert!(matches!(
        f.store.export(&project(), &target, T1),
        Err(StoreError::Refused(Refusal::ExportUnverified { .. }))
    ));
    assert!(!target.exists());
    assert_eq!(
        f.raw()
            .query_row("SELECT count(*) FROM export_record", [], |row| row
                .get::<_, i64>(0))
            .expect("records"),
        0
    );
}

// Catches creating a target before the epoch-fenced intent can be written.
#[test]
fn export_record_failure_creates_no_target() {
    let f = Fixture::new();
    f.record("work", 1);
    f.raw()
        .execute(
            "UPDATE schema_meta SET value = ?1 WHERE key = 'epoch'",
            [crate::EPOCH + 1],
        )
        .expect("newer epoch");
    let target = f.home.path().join("export");
    assert_eq!(
        f.store.export(&project(), &target, T1),
        Err(StoreError::ReadOnly {
            needed_epoch: crate::EPOCH + 1
        })
    );
    assert!(!target.exists());
}

// Catches a completed export record left pending after verification.
#[test]
fn completed_export_record_holds_the_verified_head() {
    let f = Fixture::new();
    let head = f.record("one", 1);
    let target = f.home.path().join("export");
    f.store.export(&project(), &target, T1).expect("export");
    let recorded: Option<i64> = f
        .raw()
        .query_row("SELECT head_seq FROM export_record", [], |row| row.get(0))
        .expect("record");
    assert_eq!(recorded, Some(head.seq as i64));
}

// Catches released body bytes left inside an export.
#[test]
fn an_export_removes_the_bytes_of_a_released_body() {
    let f = Fixture::new();
    let other = ProjectId("other".into());
    f.store.create_project(&other, "Other", T0).expect("other");
    let reference = f.attach("one", b"shared secret", RetentionClass::Material);
    let mut command = fixture_command("other-one", T0);
    command.project = other;
    f.store
        .transact(&command, &mut |tx| {
            let body = tx.put_payload(b"shared secret", RetentionClass::Material)?;
            tx.append(NewEvent {
                attachments: vec![body.clone()],
                ..fixture_event(json!({"body": body.to_value()}), None)
            })?;
            Ok(done())
        })
        .expect("other reference");
    let mut purge = fixture_command("purge-one", T1);
    purge.kind = CommandKind("retention.purge".into());
    f.store
        .purge(&purge, &[reference.hash], "owner request")
        .expect("purge");
    let target = f.home.path().join("export");
    f.store.export(&project(), &target, T2).expect("export");
    let row: (String, Option<Vec<u8>>) = Connection::open(target.join("baley.db"))
        .expect("copy")
        .query_row(
            "SELECT state, body FROM payload WHERE hash = ?1",
            [&reference.hash.0[..]],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("row");
    assert_eq!(row, ("purged".into(), None));
}

fn local_checks() -> BTreeMap<ProjectId, AnchorCheck> {
    BTreeMap::from([(project(), AnchorCheck::LocalOnly)])
}

// Catches a pending scrub marker or epoch read from the wrong metadata key.
#[test]
fn doctor_reports_epoch_and_pending_scrub() {
    let f = Fixture::new();
    f.raw()
        .execute(
            "INSERT INTO schema_meta (key, value) VALUES ('scrub_pending', ?1)",
            [T1],
        )
        .expect("marker");
    let health = f.store.doctor(T2, &local_checks()).expect("doctor");
    assert_eq!(
        (health.epoch, health.scrub_pending.as_deref()),
        (crate::EPOCH, Some(T1))
    );
}

// Catches an integrity check omitted or misread on a sound fixture.
#[test]
fn doctor_reports_sqlite_integrity_rows() {
    let f = Fixture::new();
    assert_eq!(
        f.store
            .doctor(T2, &local_checks())
            .expect("doctor")
            .integrity,
        ["ok"]
    );
}

// Catches measuring a different file or inventing a missing log size.
#[test]
fn doctor_reports_main_and_log_file_sizes() {
    let f = Fixture::new();
    let main = std::fs::metadata(f.home.path().join("baley.db"))
        .expect("main")
        .len();
    let wal = std::fs::metadata(f.home.path().join("baley.db-wal"))
        .map(|file| file.len())
        .unwrap_or(0);
    let health = f.store.doctor(T2, &local_checks()).expect("doctor");
    assert_eq!((health.database_bytes, health.log_bytes), (main, wal));
}

// Catches reading view stamps after doctor rebuilt a marked generation.
#[test]
fn doctor_reports_raw_view_versions_and_building_lag() {
    let f = Fixture::new();
    let head = f.record("work", 1);
    f.raw().execute("UPDATE view_gen SET projector_version = projector_version - 1, view_set_version = 1 WHERE project_id = ?1", [&project().0]).expect("older stamps");
    f.raw().execute("UPDATE project_gen SET building_gen = 5, building_applied_seq = 1 WHERE project_id = ?1", [&project().0]).expect("marker");
    let health = f.store.doctor(T2, &local_checks()).expect("doctor");
    let entry = &health.projects[0];
    let raw_views = entry.raw_views.as_ref().expect("raw views");
    assert_eq!(raw_views.view_set, (Some(1), 2));
    assert_eq!(
        raw_views
            .views
            .iter()
            .map(|view| (view.view.as_str(), view.live_version, view.binary_version))
            .collect::<Vec<_>>(),
        vec![("claim_scope", Some(0), 1), ("request", Some(1), 2)]
    );
    assert_eq!(
        raw_views.building,
        Some(baley_store::Building {
            generation: 5,
            applied_seq: 1,
            lag: head.seq - 1
        })
    );
}

// Catches one project's malformed building marker aborting the whole doctor run.
#[test]
fn doctor_keeps_a_bad_building_marker_inside_its_entry() {
    let f = Fixture::new();
    let other = ProjectId("other".into());
    f.store.create_project(&other, "Other", T0).expect("other");
    f.raw()
        .execute(
            "INSERT INTO project_gen (project_id, live_gen, building_gen, building_applied_seq) VALUES (?1, 0, 5, -1)",
            [&other.0],
        )
        .expect("bad marker");
    let checks = BTreeMap::from([
        (project(), AnchorCheck::LocalOnly),
        (other.clone(), AnchorCheck::LocalOnly),
    ]);
    let health = f.store.doctor(T2, &checks).expect("doctor");
    assert_eq!(health.projects.len(), 2);
    assert_eq!(health.projects[0].project, project());
    assert!(health.projects[0].verify.is_ok());
    assert!(health.projects[0].raw_views.is_ok());
    assert_eq!(health.projects[1].project, other);
    assert_eq!(
        health.projects[1].raw_views,
        Err(StoreError::Unavailable(
            "malformed building sequence".into()
        ))
    );
}

// Catches losing the first accepted work event's age in doctor.
#[test]
fn doctor_reports_unanchored_work_from_the_chain() {
    let f = Fixture::new();
    f.record_at("work", 1, T1);
    let health = f.store.doctor(T2, &local_checks()).expect("doctor");
    assert_eq!(
        health.projects[0].unanchored,
        UnanchoredAge::Since {
            since: T1.into(),
            warning: false
        }
    );
}

// Catches one project's malformed event time aborting the whole doctor run.
#[test]
fn doctor_keeps_an_unjudgeable_unanchored_age_inside_its_project() {
    let f = Fixture::new();
    let other = ProjectId("other".into());
    f.store.create_project(&other, "Other", T0).expect("other");
    f.record("work", 1);
    let events = f
        .ledger()
        .history(
            &project(),
            1..=u64::MAX,
            &HistoryFilter {
                types: vec![],
                git_commit: None,
            },
            PageRequest {
                limit: 100,
                after: None,
            },
        )
        .expect("history")
        .items;
    let mut previous = None;
    for (index, mut event) in events.into_iter().enumerate() {
        if index == 0 {
            event.recorded_at = "invalid time".into();
        }
        event.prev_hash = previous;
        event.hash = event.compute_hash().expect("reseal");
        f.raw()
            .execute(
                "UPDATE event SET recorded_at = ?1, prev_hash = ?2, hash = ?3 WHERE project_id = ?4 AND seq = ?5",
                params![event.recorded_at, event.prev_hash.map(|hash| hash.0.to_vec()), &event.hash.0[..], project().0, event.seq as i64],
            )
            .expect("rewrite event");
        previous = Some(event.hash);
    }
    f.raw()
        .execute(
            "UPDATE project SET head_hash = ?1 WHERE project_id = ?2",
            params![&previous.expect("head").0[..], project().0],
        )
        .expect("rewrite head");
    let checks = BTreeMap::from([
        (project(), AnchorCheck::LocalOnly),
        (other.clone(), AnchorCheck::LocalOnly),
    ]);
    let health = f.store.doctor(T2, &checks).expect("doctor");
    assert_eq!(health.projects.len(), 2);
    assert_eq!(
        health.projects[0]
            .verify
            .as_ref()
            .expect("verified chain")
            .chain
            .age_unanchored_since
            .as_deref(),
        Some("invalid time")
    );
    assert_eq!(health.projects[0].unanchored, UnanchoredAge::Unchecked);
    assert_eq!(health.projects[1].project, other);
    assert_eq!(health.projects[1].unanchored, UnanchoredAge::None);
}

// Catches an invalid supplied time hidden when a project has no work to age.
#[test]
fn doctor_rejects_an_invalid_supplied_time() {
    let f = Fixture::new();
    assert_eq!(
        f.store.doctor("invalid time", &local_checks()),
        Err(StoreError::Unavailable("malformed doctor time".into()))
    );
}

// Catches claim counts judged without the supplied doctor time.
#[test]
fn doctor_counts_active_and_interrupted_claims_at_the_supplied_time() {
    let f = Fixture::new();
    for (id, at, scope) in [("old", T0, "old-scope"), ("new", EXPIRED, "new-scope")] {
        let mut command = fixture_command(id, at);
        command.kind = CommandKind("fixture.effect".into());
        command.scope = vec![scope.into()];
        f.store
            .claim(&command, &mut |_| {
                Ok(ClaimDecision::Claim {
                    intent: json!({"effect": id}),
                    owner: ClaimOwner {
                        started_at: at.into(),
                        ..owner()
                    },
                    git: None,
                    observed: Observed::default(),
                })
            })
            .expect("claim");
    }
    let counts = f
        .store
        .doctor(EXPIRED, &local_checks())
        .expect("doctor")
        .projects
        .remove(0)
        .claims
        .expect("claims");
    assert_eq!(
        (counts.active, counts.interrupted, counts.awaiting_owner),
        (1, 1, 0)
    );
}

// Catches a project silently verified as local-only without a supplied check.
#[test]
fn doctor_requires_one_check_per_project() {
    let f = Fixture::new();
    f.store
        .create_project(&ProjectId("other".into()), "Other", T0)
        .expect("other");
    assert_eq!(
        f.store.doctor(T2, &local_checks()),
        Err(StoreError::Refused(Refusal::MissingAnchorCheck(ProjectId(
            "other".into()
        ))))
    );
}

// Catches a remote witness ignored while verifying a project.
#[test]
fn doctor_uses_the_remote_anchor_for_verification() {
    let f = Fixture::new();
    f.record("work", 1);
    let remote = Anchor {
        seq: 99,
        hash: Hash([9; 32]),
    };
    let checks = BTreeMap::from([(project(), AnchorCheck::Remote(remote))]);
    let health = f.store.doctor(T2, &checks).expect("doctor");
    assert!(matches!(
        health.projects[0]
            .verify
            .as_ref()
            .expect("verify")
            .chain
            .anchor,
        AnchorVerdict::Truncated { anchored: 99, .. }
    ));
}

// Catches a locally confirmed tag lost from the remote without a finding.
#[test]
fn doctor_reports_remote_absence_beside_a_local_anchor_row() {
    let f = Fixture::new();
    let head = f.record("work", 1);
    f.insert_anchor_row(head.seq, head.hash, &anchor_tag(&project(), head.seq));
    let checks = BTreeMap::from([(project(), AnchorCheck::RemoteAbsent)]);
    let health = f.store.doctor(T2, &checks).expect("doctor");
    assert_eq!(
        health.projects[0]
            .remote_absent_local_row
            .as_ref()
            .map(|row| row.anchor.clone()),
        Some(head_anchor(&head))
    );
}

// Catches an unreachable remote reported as a known empty age.
#[test]
fn doctor_marks_unreachable_remote_age_unchecked() {
    let f = Fixture::new();
    f.record("work", 1);
    let checks = BTreeMap::from([(project(), AnchorCheck::RemoteUnreachable)]);
    assert_eq!(
        f.store.doctor(T2, &checks).expect("doctor").projects[0].unanchored,
        UnanchoredAge::Unchecked
    );
}

// Catches one project's failed view check aborting all of doctor.
#[test]
fn doctor_keeps_a_project_view_failure_inside_its_entry() {
    let f = Fixture::new();
    let other = ProjectId("other".into());
    f.store.create_project(&other, "Other", T0).expect("other");
    f.record("work", 1);
    f.raw().execute("UPDATE project_gen SET building_gen = 5, building_applied_seq = 1 WHERE project_id = ?1", [&project().0]).expect("marker");
    let checks = BTreeMap::from([
        (project(), AnchorCheck::LocalOnly),
        (other, AnchorCheck::LocalOnly),
    ]);
    let health = f.store.doctor(T2, &checks).expect("doctor");
    assert_eq!(health.projects.len(), 2);
    assert!(matches!(
        health
            .projects
            .iter()
            .find(|entry| entry.project == project())
            .expect("project")
            .views_check,
        Err(StoreError::UnfinishedGeneration { .. })
    ));
}

// Catches project creation reading a clock instead of storing its argument.
#[test]
fn create_project_stores_the_supplied_time() {
    let f = Fixture::new();
    let added = ProjectId("added".into());
    f.store.create_project(&added, "Added", T2).expect("create");
    let at: String = f
        .raw()
        .query_row(
            "SELECT created_at FROM project WHERE project_id = ?1",
            [&added.0],
            |row| row.get(0),
        )
        .expect("time");
    assert_eq!(at, T2);
}

// Catches a created project omitted from the admin list.
#[test]
fn admin_projects_lists_created_projects_in_id_order() {
    let f = Fixture::new();
    f.store
        .create_project(&ProjectId("a".into()), "Alpha", T2)
        .expect("create");
    assert_eq!(
        f.store.projects().expect("projects"),
        vec![
            (project(), "fixture".into()),
            (ProjectId("a".into()), "Alpha".into())
        ]
    );
}

struct FixedRemote(Anchor);

impl Forge for FixedRemote {
    fn push_tag(&mut self, _: &ProjectId, _: &str, _: &str, _: &str) -> PushObservation {
        panic!("acknowledgement never pushes")
    }

    fn fetch_tag(&mut self, _: &ProjectId, _: &str, _: &TagQuery) -> FetchObservation {
        remote_anchor(&self.0)
    }
}

fn restore_request(actor: Actor) -> AcknowledgeRestore {
    AcknowledgeRestore {
        project: project(),
        request_id: RequestId("restore-1".into()),
        actor,
        policy_version: 1,
        remote: "origin".into(),
    }
}

fn remote_future() -> Anchor {
    Anchor {
        seq: 8,
        hash: Hash([8; 32]),
    }
}

// Catches an acknowledgement recorded but ignored by verification.
#[test]
fn acknowledged_restore_verifies_as_accepted() {
    let f = Fixture::new();
    f.record("work", 1);
    let remote = remote_future();
    acknowledge_restore(
        &restore_request(Actor::Owner),
        f.ledger(),
        &mut FixedRemote(remote.clone()),
        &mut || T1.into(),
    )
    .expect("acknowledge");
    let report = f
        .ledger()
        .verify(&project(), Some(&remote))
        .expect("verify");
    assert!(matches!(
        report.chain.anchor,
        AnchorVerdict::Acknowledged { anchored: 8, .. }
    ));
    assert_eq!(report.chain.acknowledged_restores.len(), 1);
}

// Catches pre-push still refusing an owner-accepted restore gap.
#[test]
fn acknowledged_restore_permits_the_next_anchor_check() {
    let f = Fixture::new();
    f.record("work", 1);
    let remote = remote_future();
    acknowledge_restore(
        &restore_request(Actor::Owner),
        f.ledger(),
        &mut FixedRemote(remote.clone()),
        &mut || T1.into(),
    )
    .expect("acknowledge");
    assert!(matches!(
        pre_push_check(f.ledger(), &project(), &remote_anchor(&remote)).expect("check"),
        PrePushCheck::Push { .. }
    ));
}

// Catches a non-owner acknowledgement that appends an event.
#[test]
fn non_owner_cannot_record_a_restore_acknowledgement() {
    let f = Fixture::new();
    let before = f.record("work", 1);
    let result = acknowledge_restore(
        &restore_request(Actor::Baley),
        f.ledger(),
        &mut FixedRemote(remote_future()),
        &mut || T1.into(),
    );
    assert_eq!(
        result,
        Err(AcknowledgeRestoreError::Store(StoreError::Refused(
            Refusal::NotOwner
        )))
    );
    assert_eq!(f.head(), before);
}

// Catches a retry digest bound to the head moved by the first completion.
#[test]
fn restore_acknowledgement_retry_replays_without_a_second_event() {
    let f = Fixture::new();
    f.record("work", 1);
    let remote = remote_future();
    let request = restore_request(Actor::Owner);
    acknowledge_restore(
        &request,
        f.ledger(),
        &mut FixedRemote(remote.clone()),
        &mut || T1.into(),
    )
    .expect("first");
    let before = f.head();
    assert!(matches!(
        acknowledge_restore(&request, f.ledger(), &mut FixedRemote(remote), &mut || T2
            .into()),
        Ok(Recorded::Replayed { .. })
    ));
    assert_eq!(f.head(), before);
}

// Catches acknowledgement of a tail added after verification.
#[test]
fn restore_acknowledgement_refuses_a_stale_head() {
    let f = Fixture::new();
    f.record("work", 1);
    let mut calls = 0;
    let result = acknowledge_restore(
        &restore_request(Actor::Owner),
        f.ledger(),
        &mut FixedRemote(remote_future()),
        &mut || {
            calls += 1;
            if calls == 2 {
                f.record("late", 1);
            }
            T1.into()
        },
    );
    assert!(matches!(
        result,
        Err(AcknowledgeRestoreError::Store(StoreError::Stale(
            baley_store::StaleInput::Head { .. }
        )))
    ));
}
