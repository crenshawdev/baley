//! Reference fixtures expressed through the storage port.
use crate::generator::Profile;
use baley_core::{Registry, register_anchor_events};
use baley_store::*;
use baley_store_sqlite::{Options, SqliteStore};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    sync::OnceLock,
};

/// Errors of a manually invoked measurement.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
/// The prototype's view families.
pub const VIEWS: [&str; 15] = [
    "phase",
    "plan",
    "evidence_map",
    "admission",
    "dispatch",
    "run",
    "verification",
    "review",
    "review_queue",
    "risk",
    "milestone",
    "pause",
    "policy",
    "guard_policy",
    "capture",
];
/// Supplied fixture time, independent of measurement clocks.
pub const AT: &str = "2026-09-27T00:00:00Z";
/// Uncompressed content handed to the real adapter.
pub struct Attachment {
    /// Retention class name.
    pub class: &'static str,
    /// Original bytes.
    pub body: Vec<u8>,
}
/// Prepared fixture event.
pub struct NewEvent {
    /// Record stream.
    pub stream: String,
    /// Event type.
    pub etype: String,
    /// Phase index.
    pub phase: i64,
    /// Inline facts.
    pub payload: Value,
    /// Large content.
    pub attachments: Vec<Attachment>,
    /// Whether the event depends on supplied git facts.
    pub git: bool,
}
/// A profile command and its decision inputs.
pub struct Command {
    /// Ledger project.
    pub project: String,
    /// Command family.
    pub kind: String,
    /// Unique fixture request.
    pub request_id: String,
    /// Phase index.
    pub phase: i64,
    /// Whether to use claim and complete.
    pub external: bool,
    /// Authority event and prototype stream prefix.
    pub authority: Option<(String, String)>,
    /// Events appended by its decision.
    pub events: Vec<NewEvent>,
}
/// Defers compression to the adapter being measured.
pub fn prepare(class: &'static str, bytes: &[u8], _searchable: bool) -> Attachment {
    Attachment {
        class,
        body: bytes.into(),
    }
}
/// Fixture keys use the same document identity in every view.
pub fn key(text: &str) -> DocKey {
    DocKey(vec![KeyValue::Text(text.into())])
}
fn view_for(t: &str) -> &'static str {
    match t.split('.').next().unwrap_or("") {
        "context" | "phase" => "phase",
        "plan" => "plan",
        "evidence_map" => "evidence_map",
        "execution" => "admission",
        "dispatch" | "task" | "suite" | "evidence" | "worker" => "dispatch",
        "verification" | "verdict" | "truth" | "human" => "verification",
        "review" => "review",
        "risk" => "risk",
        "milestone" | "release" | "landing" | "payload" => "milestone",
        "pause" => "pause",
        "policy" => "policy",
        "guard" => "guard_policy",
        "item" => "capture",
        _ => "phase",
    }
}
struct FixtureProjector {
    spec: ViewSpec,
    types: &'static [&'static str],
}
impl FixtureProjector {
    fn keys_for(&self, event: &Event) -> Vec<DocKey> {
        let mut keys = Vec::new();
        if view_for(&event.type_name) == self.spec.name {
            keys.push(key(&event.stream));
        }
        if self.spec.name == "phase" {
            keys.push(key(&format!(
                "phase/{}",
                event.payload["phase"].as_i64().unwrap_or(0)
            )));
        }
        if self.spec.name == "run"
            && (event.type_name.starts_with("task.") || event.type_name.starts_with("suite."))
        {
            keys.push(key(&format!("run/{}", event.seq)));
        }
        keys.sort();
        keys.dedup();
        keys
    }
}
impl Projector for FixtureProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }
    fn handles(&self) -> &[&str] {
        self.types
    }
    fn keys(&self, event: &Event) -> Vec<DocKey> {
        self.keys_for(event)
    }
    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> std::result::Result<Vec<Change>, ProjectorError> {
        Ok(self.keys_for(event).into_iter().map(|k| {
            let mut body = documents.iter().find(|(key,_)| key == &k).map_or(json!({}), |(_,body)|body.clone());
            let KeyValue::Text(id) = &k.0[0] else { unreachable!() };
            body["doc_key"] = json!(id);
            body["phase"] = json!(event.payload["phase"].as_i64().unwrap_or(0));
            body["state"] = json!(event.type_name);
            body["stream"] = json!(event.stream);
            body["events"] = json!(body["events"].as_u64().unwrap_or(0) + 1);
            if let Some(facts) = event.payload.get("facts").and_then(Value::as_object) { for (name,value) in facts { body[name] = value.clone(); } }
            let summary: String = event.payload["text"].as_str().unwrap_or(&event.type_name).chars().take(60).collect();
            let entry = json!({"seq":event.seq,"type":event.type_name,"summary":summary,"attachments":event.payload.get("attachments").cloned().unwrap_or(json!([]))});
            let field = if self.spec.name == "phase" { "recent" } else { "history" };
            if !body[field].is_array() { body[field] = json!([]); }
            let history = body[field].as_array_mut().unwrap(); history.push(entry);
            let cap = if self.spec.name == "phase" { 20 } else { 50 };
            if history.len() > cap { history.remove(0); }
            Change::Put { key:k,body }
        }).collect())
    }
}
/// Options shared by every process in a measurement run.
pub fn fixture_options(profile: &Profile) -> Options {
    let mut registry = Registry::new();
    register_anchor_events(&mut registry).expect("anchor registry");
    let types: BTreeSet<_> = profile
        .commands
        .iter()
        .flat_map(|c| c.events.iter().map(|e| e.etype.clone()))
        .chain(["guard.allowed", "task.run", "suite.result", "anchor.pushed"].map(String::from))
        .collect();
    for name in &types {
        if registry.current_version(name).is_none() {
            registry.register(name, 1, []).expect("fixture registry");
        }
    }
    // One profile per process, shared by all opens without repeated allocations.
    static HANDLES: OnceLock<Vec<&'static str>> = OnceLock::new();
    let handles = HANDLES.get_or_init(|| {
        types
            .into_iter()
            .chain([
                COMMAND_COMPLETED.into(),
                COMMAND_CLAIMED.into(),
                COMMAND_RECONCILED.into(),
            ])
            .map(|s| &*Box::leak(s.into_boxed_str()))
            .collect()
    });
    let projectors = VIEWS
        .iter()
        .map(|name| {
            Box::new(FixtureProjector {
                spec: ViewSpec {
                    name: (*name).into(),
                    version: 1,
                    key: vec![FieldSpec {
                        name: "doc_key".into(),
                        kind: FieldKind::Text,
                    }],
                    indexes: vec![IndexSpec {
                        name: "phase_state".into(),
                        fields: vec![
                            IndexField {
                                name: "phase".into(),
                                kind: FieldKind::Integer,
                                order: Order::Ascending,
                            },
                            IndexField {
                                name: "state".into(),
                                kind: FieldKind::Text,
                                order: Order::Ascending,
                            },
                        ],
                    }],
                    page_bound: 100,
                },
                types: handles,
            }) as Box<dyn Projector>
        })
        .collect();
    Options {
        schema: Box::new(registry),
        projectors,
        view_set_version: NonZeroU32::new(3).unwrap(),
        ..Options::default()
    }
}
/// The CLI's schema, without benchmark fixture types or projectors.
pub fn seed_options() -> Options {
    let mut registry = Registry::new();
    register_anchor_events(&mut registry).expect("anchor registry");
    Options {
        schema: Box::new(registry),
        ..Options::default()
    }
}
/// A request envelope from supplied fixture values.
pub fn command(project: &str, kind: &str, request: &str) -> baley_store::Command {
    baley_store::Command {
        project: ProjectId(project.into()),
        kind: CommandKind(kind.into()),
        request_id: RequestId(request.into()),
        digest: request_digest(&json!({"project":project,"kind":kind,"request":request})).unwrap(),
        scope: vec![],
        policy_version: 0,
        recorded_at: AT.into(),
        actor: Actor::Owner,
    }
}
/// Executes a prepared command through the adapter and returns material hashes.
pub fn transact(store: &SqliteStore, fixture: &Command) -> Result<Vec<Hash>> {
    let command = command(&fixture.project, &fixture.kind, &fixture.request_id);
    let owner = ClaimOwner {
        process: "bench".into(),
        host_session: fixture.request_id.clone(),
        started_at: AT.into(),
    };
    if fixture.external {
        store.claim(&command, &mut |_| {
            Ok(ClaimDecision::Claim {
                intent: json!({"fixture":true}),
                owner: owner.clone(),
                observed: Observed::default(),
                git: None,
            })
        })?;
    }
    let mut materials = Vec::new();
    let mut decide = |tx: &mut dyn Transaction| {
        tx.get("phase", &key(&format!("phase/{}", fixture.phase)))?;
        if let Some(event) = fixture.events.first() {
            tx.get(view_for(&event.etype), &key(&event.stream))?;
        }
        if let Some((event, _prefix)) = &fixture.authority {
            tx.event_exists(&EventMatch {
                type_name: event.clone(),
                stream: None,
                fields: BTreeMap::from([("phase".into(), json!(fixture.phase))]),
            })?;
        }
        for event in &fixture.events {
            let mut refs = Vec::new();
            for attachment in &event.attachments {
                let class = RetentionClass::parse(attachment.class).unwrap();
                let reference = tx.put_payload(&attachment.body, class)?;
                if class == RetentionClass::Material {
                    materials.push(reference.hash);
                }
                refs.push(reference);
            }
            let mut payload = event.payload.clone();
            if event.etype != ANCHOR_PUSHED {
                payload["phase"] = json!(event.phase);
                payload["attachments"] =
                    json!(refs.iter().map(PayloadRef::to_value).collect::<Vec<_>>());
            }
            tx.append(baley_store::NewEvent {
                stream: StreamName(event.stream.clone()),
                type_name: event.etype.clone(),
                type_version: 1,
                git: event.git.then(git_facts),
                payload,
                attachments: refs,
            })?;
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({"fixture":fixture.request_id}),
            sensitive: false,
            observed: Observed {
                git: if fixture.events.iter().any(|e| e.git) {
                    vec![GitObservation {
                        checkout: "fixture".into(),
                        head_seen: "a".repeat(40),
                        head_now: "a".repeat(40),
                        index_seen: "b".repeat(40),
                        index_now: "b".repeat(40),
                    }]
                } else {
                    vec![]
                },
                ..Observed::default()
            },
            git: None,
        })
    };
    if fixture.external {
        store.complete(&command, &owner, &mut decide)?;
    } else {
        store.transact(&command, &mut decide)?;
    }
    Ok(materials)
}
fn git_facts() -> GitFacts {
    GitFacts {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
        checkout: "fixture".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_schema_reads_every_profile_and_auxiliary_event() {
        let profile: Profile = serde_json::from_str(include_str!(
            "../../../spikes/evidence-ledger-bench/profile/baseline-store.json"
        ))
        .unwrap();
        let options = fixture_options(&profile);
        for event in profile.commands.iter().flat_map(|c| &c.events) {
            assert!(options.schema.reads(&event.etype, 1), "{}", event.etype);
        }
        for name in ["guard.allowed", "task.run", "suite.result", "anchor.pushed"] {
            assert!(options.schema.reads(name, 1), "{name}");
        }
    }
}
