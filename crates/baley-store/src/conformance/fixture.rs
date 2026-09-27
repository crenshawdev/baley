//! Supplied fixtures and documents derived by hand from their events.

use crate::*;
use serde_json::{Value, json};
use std::num::NonZeroU32;

/// The supplied time at which commands and claims begin.
pub(super) const T0: &str = "2026-09-25T18:00:00Z";
const PROJECT: &str = "7f0c2a4e-8d1b-4c3a-9e5f-2b6d8a1c4e70";
const ITEM: &str = "fixture.item";
const QUIET: &str = "fixture.quiet";
/// The item of the last command, which a second store's failing
/// projector cannot apply.
pub(super) const TAIL: i64 = 99;

/// Reads item events and the anchor events used by these checks.
/// Version 1 of an item called its state `status`; version 2 calls it
/// `state`, and projection upcasts a version-1 payload to that.
struct Schema;

impl EventSchema for Schema {
    fn reads(&self, type_name: &str, version: u32) -> bool {
        (type_name == ITEM && (1..=2).contains(&version))
            || (matches!(type_name, ANCHOR_PUSHED | ANCHOR_RESTORE_ACKNOWLEDGED) && version == 1)
    }

    fn projection_payload(&self, event: &Event) -> Result<(u32, Value), String> {
        match (event.type_name.as_str(), event.type_version) {
            (ITEM, 1) => {
                let mut payload = event.payload.clone();
                let object = payload.as_object_mut().ok_or("not an object")?;
                let state = object.remove("status").ok_or("no status")?;
                object.insert("state".into(), state);
                object.insert("rank".into(), json!(0));
                object.insert("owner".into(), json!(""));
                Ok((2, payload))
            }
            (ITEM, 2) | (ANCHOR_PUSHED | ANCHOR_RESTORE_ACKNOWLEDGED, 1) => {
                Ok((event.type_version, event.payload.clone()))
            }
            _ => Err("unknown".into()),
        }
    }
}

fn id_key() -> Vec<FieldSpec> {
    vec![FieldSpec {
        name: "id".into(),
        kind: FieldKind::Integer,
    }]
}

/// `item`: each item's latest state, indexed by state. Applies only the
/// current version 2 of `fixture.item`, so an event handed over without
/// its upcast fails.
struct Item(ViewSpec);

fn item(version: u32) -> Box<dyn Projector> {
    Box::new(Item(ViewSpec {
        name: "item".into(),
        version,
        key: id_key(),
        indexes: vec![
            IndexSpec {
                name: "by_state_rank".into(),
                fields: vec![
                    IndexField {
                        name: "state".into(),
                        kind: FieldKind::Text,
                        order: Order::Ascending,
                    },
                    IndexField {
                        name: "rank".into(),
                        kind: FieldKind::Integer,
                        order: Order::Descending,
                    },
                ],
            },
            IndexSpec {
                name: "by_owner".into(),
                fields: vec![IndexField {
                    name: "owner".into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                }],
            },
        ],
        page_bound: 3,
    }))
}

impl Projector for Item {
    fn spec(&self) -> &ViewSpec {
        &self.0
    }

    fn handles(&self) -> &[&str] {
        &[ITEM]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        vec![DocKey(vec![KeyValue::Integer(
            event.payload["id"].as_i64().unwrap_or(0),
        )])]
    }

    fn apply(
        &self,
        event: &Event,
        _documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        if event.type_version != 2 {
            return Err(ProjectorError(format!(
                "{ITEM} version {} is not current",
                event.type_version
            )));
        }
        let mut body = json!({"id": event.payload["id"], "state": event.payload["state"],
            "rank": event.payload["rank"], "owner": event.payload["owner"]});
        if self.0.version == 3 {
            body["new_field"] = json!(true);
        }
        Ok(vec![Change::Put {
            key: self.keys(event).remove(0),
            body,
        }])
    }
}

/// `tally`: how many events each item has had, counted from the stored
/// document, so a replay that reads the wrong generation counts wrong.
/// With `fail_on`, it refuses that item, as a same-spec projector of a
/// second store over the same home.
struct Tally {
    spec: ViewSpec,
    fail_on: Option<i64>,
}

fn tally(fail_on: Option<i64>) -> Box<dyn Projector> {
    Box::new(Tally {
        spec: ViewSpec {
            name: "tally".into(),
            version: 1,
            key: id_key(),
            indexes: Vec::new(),
            page_bound: 10,
        },
        fail_on,
    })
}

impl Projector for Tally {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[ITEM]
    }

    fn keys(&self, event: &Event) -> Vec<DocKey> {
        vec![DocKey(vec![KeyValue::Integer(
            event.payload["id"].as_i64().unwrap_or(0),
        )])]
    }

    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let id = event.payload["id"].as_i64().unwrap_or(0);
        if self.fail_on == Some(id) {
            return Err(ProjectorError(format!("item {id} is refused")));
        }
        let seen = documents
            .first()
            .and_then(|(_, body)| body["seen"].as_i64())
            .unwrap_or(0);
        Ok(vec![Change::Put {
            key: self.keys(event).remove(0),
            body: json!({"id": id, "seen": seen + 1}),
        }])
    }
}

/// `quiet`: a view no fixture event feeds, so it stays empty.
struct Quiet(ViewSpec);

fn quiet() -> Box<dyn Projector> {
    Box::new(Quiet(ViewSpec {
        name: "quiet".into(),
        version: 1,
        key: id_key(),
        indexes: Vec::new(),
        page_bound: 10,
    }))
}

impl Projector for Quiet {
    fn spec(&self) -> &ViewSpec {
        &self.0
    }

    fn handles(&self) -> &[&str] {
        &[QUIET]
    }

    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        Vec::new()
    }

    fn apply(
        &self,
        _event: &Event,
        _documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        Ok(Vec::new())
    }
}

/// The primary fixture project.
pub(super) fn project() -> ProjectId {
    ProjectId(PROJECT.into())
}

fn set(version: u32) -> NonZeroU32 {
    NonZeroU32::new(version).expect("positive")
}

/// The baseline views, event schema and view set.
pub(super) fn binary() -> Binary {
    Binary {
        projectors: vec![item(2), tally(None)],
        schema: Box::new(Schema),
        view_set_version: set(3),
    }
}
/// The item projector at its next version.
pub(super) fn newer_views() -> Binary {
    Binary {
        projectors: vec![item(3), tally(None)],
        ..binary()
    }
}
/// A new set containing one additional empty view.
pub(super) fn newer_set() -> Binary {
    Binary {
        projectors: vec![item(2), tally(None), quiet()],
        view_set_version: set(4),
        ..binary()
    }
}
/// An additional view without the required set version change.
pub(super) fn changed_set() -> Binary {
    Binary {
        view_set_version: set(3),
        ..newer_set()
    }
}
/// A forward set version with tally removed.
pub(super) fn removed_view() -> Binary {
    Binary {
        projectors: vec![item(2)],
        view_set_version: set(5),
        ..binary()
    }
}
/// The baseline binary whose tally refuses the tail item.
pub(super) fn failing() -> Binary {
    Binary {
        projectors: vec![item(2), tally(Some(TAIL))],
        ..binary()
    }
}
struct Blind;
impl EventSchema for Blind {
    fn reads(&self, _: &str, _: u32) -> bool {
        false
    }
}
/// The fixture views with no readable domain event types.
pub(super) fn blind() -> Binary {
    Binary {
        schema: Box::new(Blind),
        ..binary()
    }
}
/// An empty store with both fixture projects registered.
pub(super) fn created<F: StoreFactory>(factory: &F) -> F::Store {
    let store = factory.create(binary()).expect("open");
    store
        .create_project(&project(), "fixture", T0)
        .expect("project");
    store
        .create_project(&other_project(), "other", T0)
        .expect("other project");
    store
}
/// The second project for isolation checks.
pub(super) fn other_project() -> ProjectId {
    ProjectId("bffab3d0-67e8-41ab-94a8-26e7a064a809".into())
}
/// A supplied owner command with a stable digest and time.
pub(super) fn command(kind: &str, request: &str) -> Command {
    Command {
        project: project(),
        kind: CommandKind(kind.into()),
        request_id: RequestId(request.into()),
        digest: Hash([1; 32]),
        scope: Vec::new(),
        policy_version: 1,
        recorded_at: T0.into(),
        actor: Actor::Owner,
    }
}

/// A completed decision with no observed inputs.
pub(super) fn done(answer: Value, sensitive: bool) -> Decision {
    Decision {
        kind: OutcomeKind::Done,
        answer,
        sensitive,
        observed: Observed::default(),
        git: None,
    }
}

/// An item event with no attachments or git observation.
pub(super) fn event(type_version: u32, payload: Value) -> NewEvent {
    NewEvent {
        stream: StreamName("fixture".into()),
        type_name: ITEM.into(),
        type_version,
        git: None,
        payload,
        attachments: Vec::new(),
    }
}

/// Records the items, each as a version-2 `fixture.item`, under a
/// fresh `fixture.add` request that answers `"ok"`.
pub(super) fn record(
    store: &impl Ledger,
    request: &str,
    items: &[(i64, &str)],
) -> Result<Recorded, StoreError> {
    store.transact(&command("fixture.add", request), &mut |tx| {
        for (id, state) in items {
            tx.append(event(
                2,
                json!({"id": id, "state": state, "rank": 0, "owner": ""}),
            ))?;
        }
        Ok(done(json!("ok"), false))
    })
}

/// The fixture chain: 1 item 1 open, 2 r1 completed, 3 item 2 open,
/// 4 item 1 done, 5 r2 completed.
pub(super) fn fixture(store: &impl Ledger) {
    record(store, "r1", &[(1, "open")]).expect("r1");
    record(store, "r2", &[(2, "open"), (1, "done")]).expect("r2");
}

/// An integer item or tally key.
pub(super) fn id(id: i64) -> DocKey {
    DocKey(vec![KeyValue::Integer(id)])
}

/// A request key scoped by command kind.
pub(super) fn request(kind: &str, request: &str) -> DocKey {
    DocKey(vec![
        KeyValue::Text(kind.into()),
        KeyValue::Text(request.into()),
    ])
}

/// A document's body and the event that produced it, read through the
/// port.
pub(super) fn doc(store: &impl Views, view: &str, key: &DocKey) -> Option<(Value, u64)> {
    store
        .get(&project(), view, key)
        .expect("get")
        .map(|document| (document.body, document.produced_seq))
}

fn completed(request: &str) -> Value {
    json!({"kind": "fixture.add", "request_id": request, "digest": "01".repeat(32),
           "state": "completed", "scope": [], "outcome": "done", "answer": {"inline": "ok"}})
}

/// The fixture chain's documents, written by hand from its events:
/// (view, key, body, produced_seq).
pub(super) fn expected() -> Vec<(&'static str, DocKey, Value, u64)> {
    vec![
        (
            "item",
            id(1),
            json!({"id": 1, "state": "done", "rank": 0, "owner": ""}),
            4,
        ),
        (
            "item",
            id(2),
            json!({"id": 2, "state": "open", "rank": 0, "owner": ""}),
            3,
        ),
        ("request", request("fixture.add", "r1"), completed("r1"), 2),
        ("request", request("fixture.add", "r2"), completed("r2"), 5),
        ("tally", id(1), json!({"id": 1, "seen": 2}), 4),
        ("tally", id(2), json!({"id": 2, "seen": 1}), 3),
    ]
}

/// The fixture chain's documents as the store reads them.
pub(super) fn read_back(store: &impl Views) -> Vec<(&'static str, DocKey, Value, u64)> {
    expected()
        .into_iter()
        .map(|(view, key, _, _)| {
            let (body, seq) = doc(store, view, &key).expect("present");
            (view, key, body, seq)
        })
        .collect()
}

/// A supplied time while the claim lease remains active.
pub(super) const ACTIVE: &str = "2026-09-25T18:00:10Z";
/// A supplied time after the claim lease expires.
pub(super) const EXPIRED: &str = "2026-09-25T18:01:01Z";
/// All events of the primary project through the port.
pub(super) fn history(store: &impl Ledger) -> Vec<Event> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = store
            .history(
                &project(),
                1..=u64::MAX,
                &HistoryFilter::default(),
                PageRequest { limit: 100, after },
            )
            .expect("history");
        out.extend(page.items);
        after = page.next;
        if after.is_none() {
            return out;
        }
    }
}
/// A supplied witness copied from the current head.
pub(super) fn anchor(store: &impl Ledger) -> Anchor {
    let head = store.head(&project()).expect("head").expect("nonempty");
    Anchor {
        seq: head.seq,
        hash: head.hash,
    }
}
/// A page of the mixed-direction item index.
pub(super) fn query(limit: u32) -> IndexQuery {
    IndexQuery {
        index: "by_state_rank".into(),
        equals: vec![],
        page: PageRequest { limit, after: None },
    }
}
/// Items whose index ties require a primary-key tiebreaker.
pub(super) fn stocked<F: StoreFactory>(factory: &F) -> F::Store {
    let store = created(factory);
    store
        .transact(&command("fixture.add", "stock"), &mut |tx| {
            for (id, state, rank, owner) in [
                (6, "open", 2, "b"),
                (3, "done", 1, "a"),
                (1, "open", 3, "b"),
                (5, "open", 2, "a"),
                (2, "done", 2, "a"),
                (4, "open", 3, "a"),
            ] {
                tx.append(event(
                    2,
                    json!({"id":id,"state":state,"rank":rank,"owner":owner}),
                ))?;
            }
            Ok(done(json!("ok"), false))
        })
        .expect("stock");
    store
}
/// An output body referenced by an item event.
pub(super) fn attach(store: &impl Ledger, command: &Command, bytes: &[u8]) -> PayloadRef {
    attach_class(store, command, bytes, RetentionClass::Output)
}

/// A body referenced under the supplied retention class.
pub(super) fn attach_class(
    store: &impl Ledger,
    command: &Command,
    bytes: &[u8],
    class: RetentionClass,
) -> PayloadRef {
    let mut reference = None;
    store
        .transact(command, &mut |tx| {
            let body = tx.put_payload(bytes, class)?;
            let mut e = event(
                2,
                json!({"id":1,"state":"open","rank":0,"owner":"","output":body.to_value()}),
            );
            e.attachments.push(body.clone());
            tx.append(e)?;
            reference = Some(body);
            Ok(done(json!("ok"), false))
        })
        .expect("attach");
    reference.expect("body")
}
/// Distinct first, middle and last chunks for reduction checks.
pub(super) fn body_bytes() -> Vec<u8> {
    [vec![b'a'; 65536], vec![b'b'; 65536], vec![b'c'; 65536]].concat()
}
