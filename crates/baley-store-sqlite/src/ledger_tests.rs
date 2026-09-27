//! The ledger's reads, verification and anchor rows on this adapter, and
//! the core's anchor steps run against it. Each test opens a fresh store in
//! a temporary directory it owns; remote observations and times are
//! supplied values, and no test runs git, a forge, a ticker or a clock.

use std::path::Path;

use baley_core::{
    AnchorRequest, AnchorStatus, ClaimStep, FetchObservation, HeldAnchor, PrePushCheck,
    PushObservation, ReconcileStep, Registry, TraceRecord, TraceSink, Upcaster, anchor_annotation,
    claim_step, pre_push_check, reconcile_from_observation, record_step, register_anchor_events,
    verify_observed,
};
use baley_store::{
    ANCHOR_FAILED, ANCHOR_PUSHED, Actor, Anchor, AnchorVerdict, COMMAND_CLAIMED, COMMAND_COMPLETED,
    COMMAND_RECONCILED, ClaimOwner, ClaimState, Command, CommandKind, Decision, DocKey, Event,
    EventSchema, GitFacts, Hash, Head, HistoryFilter, KeyValue, Ledger, NewEvent, Observed,
    OutcomeKind, PageRequest, PayloadFault, PayloadReference, ProjectId, REQUEST_VIEW, Recorded,
    Refusal, RequestId, RetentionClass, StoreError, StoredAnchorComparison, StreamName,
    Transaction, Views, anchor_tag,
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
    store: SqliteStore,
}

impl Fixture {
    fn new() -> Self {
        Self::with_schema(Box::new(registry()))
    }

    fn with_schema(schema: Box<dyn EventSchema>) -> Self {
        let home = tempfile::tempdir().expect("home");
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
        Self { home, store }
    }

    fn ledger(&self) -> &dyn Ledger {
        &self.store
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

// An anchor whose hash the chain does not have at its sequence is a
// rewrite. Catches a verify that ignores the anchor.
#[test]
fn verify_reports_a_rewrite_against_a_foreign_head() {
    let f = Fixture::new();
    f.record("w1", 3);
    let foreign = Anchor {
        seq: 2,
        hash: Hash([9; 32]),
    };
    let report = f
        .ledger()
        .verify(&project(), Some(&foreign))
        .expect("verify");
    assert!(matches!(
        report.chain.anchor,
        AnchorVerdict::Rewritten { seq: 2, .. }
    ));
}

// An anchor past the head is a truncation. Catches a short chain accepted
// as merely unanchored.
#[test]
fn verify_reports_truncation_past_the_head() {
    let f = Fixture::new();
    f.record("w1", 3);
    let past = Anchor {
        seq: 10,
        hash: Hash([9; 32]),
    };
    let report = f.ledger().verify(&project(), Some(&past)).expect("verify");
    assert_eq!(
        report.chain.anchor,
        AnchorVerdict::Truncated {
            anchored: 10,
            head: 4
        }
    );
}

// A body whose stored bytes are damaged, its event untouched, is corrupt
// while the chain verifies. Catches a verify that never opens a body.
#[test]
fn a_body_corrupted_in_place_is_corrupt_and_the_chain_holds() {
    let f = Fixture::new();
    let body = f.attach("w1", b"the test output", RetentionClass::Output);
    f.overwrite_body(&body.hash, b"not zstd at all");
    let report = f.ledger().verify(&project(), None).expect("verify");
    assert!(report.chain.is_intact());
    assert_eq!(report.payloads, [PayloadFault::Corrupt(body.hash)]);
}

// A purged body is a valid tombstone, not a fault. Catches a tombstone
// reported as corruption.
#[test]
fn a_purged_body_is_a_tombstone() {
    let f = Fixture::new();
    let body = f.attach("w1", b"a secret in the output", RetentionClass::Output);
    f.store
        .purge(
            &Command {
                kind: CommandKind("retention.purge".into()),
                ..fixture_command("p1", T1)
            },
            &[body.hash],
            "owner request",
        )
        .expect("purge");
    let report = f.ledger().verify(&project(), None).expect("verify");
    assert!(report.chain.is_intact());
    assert_eq!(report.payloads, []);
    assert_eq!(report.tombstones_checked, 1);
}

/// A reducible output body: over 128 KiB, its edges distinct.
fn long_output() -> Vec<u8> {
    let mut body = vec![7u8; 200 * 1024];
    body[..65_536].fill(1);
    body[200 * 1024 - 65_536..].fill(2);
    body
}

fn reduce(f: &Fixture, reference: &PayloadReference) -> Hash {
    f.store
        .reduce(
            &Command {
                kind: CommandKind("retention.reduce".into()),
                ..fixture_command("r1", T1)
            },
            reference,
        )
        .expect("reduce")
        .hash
}

// A reduced original is a tombstone and its retained excerpt is opened and
// hashed. Catches retained bytes skipped with their original.
#[test]
fn a_reduced_body_checks_its_retained_excerpt() {
    let f = Fixture::new();
    let reference = f.attach("w1", &long_output(), RetentionClass::Output);
    reduce(&f, &reference);
    let report = f.ledger().verify(&project(), None).expect("verify");
    assert_eq!(report.payloads, []);
    assert_eq!(report.tombstones_checked, 1);
    assert_eq!(report.bodies_checked, 1);
}

// An excerpt whose bytes decompress to other content is corrupt, while its
// reduced original stays a tombstone. Catches corruption taken for a
// tombstone.
#[test]
fn a_corrupt_excerpt_faults_beside_a_valid_tombstone() {
    let f = Fixture::new();
    let reference = f.attach("w1", &long_output(), RetentionClass::Output);
    let excerpt = reduce(&f, &reference);
    let other = zstd::bulk::compress(&vec![5u8; 131_072], 3).expect("compress");
    f.overwrite_body(&excerpt, &other);
    let report = f.ledger().verify(&project(), None).expect("verify");
    assert_eq!(report.payloads, [PayloadFault::Corrupt(excerpt)]);
    assert_eq!(report.tombstones_checked, 1);
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

/// Retries the blocked request once and checks it claims its own new tag
/// under its original identity.
fn retry_once(f: &Fixture, setup: &Interrupted) {
    let head = f.head();
    let retried = act(f, &setup.second, RETRY);
    assert_eq!(retried.target.intent.seq, head.seq);
    assert_ne!(retried.target.tag, setup.held.target.tag);
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
}

// A matching holder tag completes the holder as done under a reconciler
// attributed to Baley, then the blocked request retries once under its own
// identity. Catches a missing retry or a changed digest.
#[test]
fn a_matching_holder_tag_reconciles_as_done_and_retries_once() {
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
    retry_once(&f, &setup);
}

// A holder tag naming another head completes the holder as refused with
// anchor.failed, then retries once. Catches existence taken for
// agreement.
#[test]
fn a_conflicting_holder_tag_reconciles_as_refused_and_retries_once() {
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
    retry_once(&f, &setup);
}

// A holder tag the remote confirms absent completes the holder as not
// pushed, then retries once. Catches absence left unrecorded.
#[test]
fn an_absent_holder_tag_reconciles_as_not_pushed_and_retries_once() {
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
    retry_once(&f, &setup);
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

// A remote anchor newer than the local row is the witness, and the row is
// reported as behind. Catches the local row taken as authoritative.
#[test]
fn a_remote_anchor_newer_than_the_row_reports_the_row_behind() {
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
    assert_eq!(verified.status, AnchorStatus::Remote(head_anchor(&later)));
    assert_eq!(verified.report.chain.anchor, AnchorVerdict::Matches);
    assert_eq!(
        verified.report.stored_anchor_comparison,
        StoredAnchorComparison::LocalBehind
    );
}

// A row at the remote's sequence with another hash, or another tag, is a
// conflict. Catches a row trusted because its sequence matches.
#[test]
fn a_same_sequence_row_that_differs_is_a_conflict() {
    let head_tag = |f: &Fixture| anchor_tag(&project(), f.head().seq);
    for (hash, tag) in [
        (Some(Hash([9; 32])), None),
        (None, Some("baley-anchor/x/4")),
    ] {
        let f = Fixture::new();
        let head = f.record("w1", 3);
        f.insert_anchor_row(
            head.seq,
            hash.unwrap_or(head.hash),
            tag.map_or_else(|| head_tag(&f), str::to_owned).as_str(),
        );
        let verified = verify_observed(
            f.ledger(),
            &project(),
            &remote_anchor(&head_anchor(&head)),
            T1,
        )
        .expect("verify");
        assert_eq!(
            verified.report.stored_anchor_comparison,
            StoredAnchorComparison::Conflict
        );
    }
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
    assert_eq!(verified.status, AnchorStatus::LocalOnly);
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
    assert_eq!(verified.status, AnchorStatus::RemoteAbsent);
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
    assert_eq!(verified.status, AnchorStatus::RemoteUnreachable);
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
    assert_eq!(report.age_unanchored_since, None);
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
    assert_eq!(report.age_unanchored_since, None);
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
    assert_eq!(report.age_unanchored_since.as_deref(), Some(T1));
}
