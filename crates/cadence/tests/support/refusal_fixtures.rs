use crate::serve;
use serde_json::{Value, json};
use std::path::Path;

pub type Tree = std::collections::BTreeMap<std::path::PathBuf, Option<Vec<u8>>>;

/// Call only after the server exits. Compare every prior journal row and all
/// domain data, permitting one exact verification replay receipt when named.
pub fn assert_delta(project: &Path, before: &Tree, answer: &Value) {
    assert_delta_with_receipt(project, before, answer, None);
}

pub fn assert_delta_with_receipt(project: &Path, before: &Tree, answer: &Value, patch: Option<&Value>) {
    assert_eq!(answer["status"], "refused");
    let after = serve::reopened(project);
    let bytes = |name: &str| before.get(Path::new(name)).and_then(|v| v.as_deref()).unwrap_or_default();
    let prior: Option<cadence::store::model::Snapshot> = if bytes(".planning/state.json").is_empty() { None } else {
        Some(cadence::store::model::Snapshot::parse(bytes(".planning/state.json"),
            bytes(".planning/items.jsonl"), bytes(".planning/decisions.jsonl")).unwrap())
    };
    let data = prior.as_ref().map(|p| p.data.clone()).unwrap_or(json!({}));
    let decisions: Vec<cadence::store::model::DecisionRecord> =
        cadence::store::model::parse_lines(bytes(".planning/decisions.jsonl")).unwrap();
    assert_eq!(after.decisions.len(), decisions.len() + 1 + usize::from(patch.is_some()));
    assert!(std::fs::read(project.join(".planning/decisions.jsonl")).unwrap().starts_with(bytes(".planning/decisions.jsonl")));
    assert_eq!(&after.decisions[..decisions.len()], &decisions);
    let cadence::store::model::Decision::BoundaryV1(saved) = &after.decisions.last().unwrap().decision else {
        panic!("refusal did not append a boundary decision");
    };
    assert!(saved.boundary.native_refusal);
    assert!(!saved.terminal);
    assert_eq!(saved.store_generation, after.snapshot.generation);
    assert!(after.snapshot.operations.contains_key(&format!(
        "execution-observation:{}", after.decisions.last().unwrap().id)));
    assert_answer(&saved.boundary, answer);
    let cadence::execution::boundary::Receipt::Compact {
        envelope: cadence::envelope::Envelope::Refused { reason, .. }
    } = &saved.boundary.receipt else { unreachable!() };
    if let Some(expected) = answer["reason"].as_str() {
        assert_eq!(*reason, cadence::execution::boundary::native_refusal_reason(expected));
    }
    let mut actual = after.snapshot.data.clone();
    if let Some(patch) = patch {
        let old = data["verification"]["claims"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let claims = actual["verification"]["claims"].as_array_mut().unwrap();
        assert_eq!(claims.len(), old.len() + 1);
        assert_eq!(&claims[..old.len()], old);
        let claim = claims.pop().unwrap();
        assert_eq!(claim["patch"], *patch);
        assert_eq!(claim["answer"], *answer);
        let receipt = &after.decisions[decisions.len()];
        assert_eq!(receipt.id, format!("verification-claim:{}", cadence::store::model::digest(patch["request_id"].as_str().unwrap().as_bytes())));
        let cadence::store::model::Decision::Gate { evidence: cadence::store::model::Evidence::Text(encoded), .. } = &receipt.decision else {
            panic!("missing verification replay receipt");
        };
        assert_eq!(serde_json::from_str::<Value>(encoded).unwrap(), claim);
        if data["verification"].get("claims").is_none() {
            actual["verification"].as_object_mut().unwrap().remove("claims");
        }
    }
    assert_eq!(actual, data, "every domain record and prior replay receipt is preserved");
    if let Some(prior) = prior {
        assert_eq!(after.snapshot.operations.len(), prior.operations.len() + 1 + usize::from(patch.is_some()));
        assert_eq!(after.snapshot.generation, prior.generation + 1 + u64::from(patch.is_some()));
        for (id, fingerprint) in prior.operations {
            assert_eq!(after.snapshot.operations.get(&id), Some(&fingerprint));
        }
    }
    let mut old_files = before.clone();
    let mut new_files = serve::tree(project);
    for path in [".planning/state.json", ".planning/decisions.jsonl"] {
        old_files.remove(Path::new(path));
        new_files.remove(Path::new(path));
    }
    // First observation can initialize an empty items journal, never an item.
    if !old_files.contains_key(Path::new(".planning/items.jsonl")) {
        assert_eq!(new_files.remove(Path::new(".planning/items.jsonl")), Some(Some(vec![])));
    }
    assert_eq!(new_files, old_files, "projections and non-store files stay byte-identical");
}

pub struct Active {
    pub completed: serve::Completed,
    pub contract: Value,
    pub tasks: Vec<Value>,
    pub plan: Value,
    pub replacement: Value,
}

impl Active {
    pub fn new() -> Self {
        let completed = serve::Completed::new();
        let project = completed.project();
        // The fixture owns roadmap order; phase 13 retains its real native
        // completion while phase 28 is now the current authored work.
        std::fs::write(project.join(".planning/ROADMAP.md"),
            "## Phases\n- [ ] **Phase 28: Refusals**\n- [ ] **Phase 13: Plan publication**\n").unwrap();
        let mut client = serve::Client::open(project);
        let context = client.call("cadence_apply", serve::approve(json!({
            "operation":"context-submit","submission":{"phase":28,"title":"Refusals",
            "scope":"Observe refused requests.","durable_decisions":[],"decisions":[],"assumptions":[],
            "truths":[{"id":"T1","trigger":"a request is refused","observer":"the owner","verb":"gets",
                "outcome":"a retained decision","kind":"property","observable":true,"fixed_oracle":true}]}
        })));
        assert_eq!(context["status"], "ok", "{context}");
        let preview = client.call("cadence_query", json!({"operation":"plan-read","phase":28,"count":2}));
        let mut input = serve::request(&preview, 28, "refusal-two-plans", &["First plan", "Second plan"]);
        for entry in input["submission"]["plans"].as_array_mut().unwrap() {
            entry["content"]["evidence_map"] = serve::attached(vec![serve::check("check/refusal", &["T1"])]);
            entry["content"]["tasks"][0]["verify"] = json!(["custom-delivery-check"]);
        }
        let published = client.call("cadence_apply", serve::approve(input.clone()));
        assert_eq!(published["status"], "ok", "{published}");
        let read = client.call("cadence_query", json!({"operation":"plan-read","phase":28}));
        let publications = read["native"]["publications"].as_object().unwrap();
        let mut plans = vec![];
        let mut allocation = vec![];
        let map = client.call("cadence_query", json!({"operation":"evidence-read","phase":28}));
        let check = json!({"id":map["items"][0]["id"],"item_revision":map["items"][0]["item_revision"]});
        for publication in publications.values() {
            let number = &publication["identity"]["plan"];
            plans.push(json!({"plan":number,"publication_request":publication["publication_request"],
                "content_revision":publication["revision"],"map_revision":publication["map_revision"]}));
            for task in publication["tasks"].as_array().unwrap() {
                let checks = if allocation.is_empty() { vec![check.clone()] } else { vec![] };
                allocation.push(json!({"plan":number,"task":task["id"],"checks":checks}));
            }
        }
        let contract = json!({"phase":28,"occurrence":read["occurrence"],"plans":plans,"allocation":allocation});
        let admitted = client.call("cadence_apply", serve::admit_request(contract.clone(), "refusal-admit", 0));
        assert_eq!(admitted["status"], "ok", "{admitted}");
        let authorized = client.call("cadence_apply", json!({"operation":"execution-authorize","phase":28,
            "request_id":"refusal-authorize","owner":"Fixture Owner","at":"2026-09-21T12:00:00Z","response":"Proceed"}));
        assert_eq!(authorized["status"], "ok", "{authorized}");
        let dispatch = client.call("cadence_query", json!({"operation":"execute-next","phase":28}));
        assert_eq!(dispatch["status"], "ok", "{dispatch}");
        let history = client.call("cadence_query", json!({"operation":"execution-history","phase":28}));
        let tasks = history["tasks"].as_array().unwrap().clone();
        let started = client.call("cadence_apply", json!({"operation":"execution-task-start","request":{
            "request_id":"refusal-start-first","task":tasks[0]["task"],"attempt":"refusal-first",
            "expected_version":0,"predecessor":null,"checks":[check]}}));
        assert_eq!(started["status"], "ok", "{started}");
        let history = client.call("cadence_query", json!({"operation":"execution-history","phase":28}));
        let plan = history["plans"][0].clone();
        let preview = client.call("cadence_query", json!({"operation":"plan-read","phase":28,"count":1}));
        let mut replacement = input;
        replacement["submission"]["request_id"] = json!("refusal-replace");
        replacement["submission"]["inventory_basis"] = preview["inventory"]["basis"].clone();
        replacement["submission"]["plans"].as_array_mut().unwrap().truncate(1);
        let entry = &mut replacement["submission"]["plans"][0];
        entry["replacement"] = json!({"approved":true,"owner":"Fixture Owner","at":"2026-09-21T12:00:00Z",
            "target":entry["target"],"old_revision":publications["1"]["revision"],
            "old_document":std::fs::read_to_string(project.join(".planning/phases/28/PLAN-1.md")).unwrap(),
            "content":entry["content"]});
        client.finish();
        Self { completed, contract, tasks, plan, replacement: serve::approve(replacement) }
    }

    pub fn project(&self) -> &Path { self.completed.project() }

    pub fn requests(&self) -> Vec<Value> {
        vec![
            json!({"operation":"execution-task-start","request":{"request_id":"refusal-not-next",
                "task":self.tasks[2]["task"],"attempt":"refusal-non-next","expected_version":0,"predecessor":null,"checks":[]}}),
            json!({"operation":"execution-run","request":{"request_id":"refusal-unstarted",
                "task":self.tasks[1]["task"],"attempt":"refusal-second","expected_version":0,
                "command":"printf documented","check":null,"stage":"verify"}}),
            json!({"operation":"execution-plan-complete","request":{"request_id":"refusal-open-plan",
                "plan":self.plan["plan"],"expected_version":self.plan["state"]["version"]}}),
            serve::admit_request(self.contract.clone(), "refusal-stale-admission", 0),
            self.replacement.clone(),
        ]
    }
}

pub fn assert_answer(boundary: &cadence::execution::boundary::BoundaryV1, answer: &Value) {
    let cadence::execution::boundary::Receipt::Compact { envelope: cadence::envelope::Envelope::Refused { code, .. } } = &boundary.receipt else {
        panic!("expected a compact refused receipt: {boundary:?}");
    };
    assert_eq!(code, answer["code"].as_str().unwrap());
    let located = boundary.located.as_ref().unwrap();
    assert_eq!(located.rule.as_deref(), answer["rule"].as_str());
    assert_eq!(located.slot.as_deref(), answer["slot"].as_str());
    assert_eq!(located.id.as_deref(), answer["id"].as_str().filter(|s| !s.is_empty()));
}
