use crate::serve;
use serde_json::{Value, json};
use std::path::Path;

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
            entry["content"]["evidence_map"] = serve::attached(vec![serve::artifact("artifact/refusal", &["T1"])]);
        }
        let published = client.call("cadence_apply", serve::approve(input.clone()));
        assert_eq!(published["status"], "ok", "{published}");
        let read = client.call("cadence_query", json!({"operation":"plan-read","phase":28}));
        let publications = read["native"]["publications"].as_object().unwrap();
        let mut plans = vec![];
        let mut allocation = vec![];
        for publication in publications.values() {
            let number = &publication["identity"]["plan"];
            plans.push(json!({"plan":number,"publication_request":publication["publication_request"],
                "content_revision":publication["revision"],"map_revision":publication["map_revision"]}));
            for task in publication["tasks"].as_array().unwrap() {
                allocation.push(json!({"plan":number,"task":task["id"],"checks":[]}));
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
            "expected_version":0,"predecessor":null,"checks":[]}}));
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
                "task":self.tasks[1]["task"],"attempt":"refusal-second","expected_version":0,"predecessor":null,"checks":[]}}),
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
    let cadence::execution::boundary::Receipt::Compact { envelope: cadence::execution::boundary::Envelope::Refused { code, .. } } = &boundary.receipt else {
        panic!("expected a compact refused receipt: {boundary:?}");
    };
    assert_eq!(code, answer["code"].as_str().unwrap());
    let located = boundary.located.as_ref().unwrap();
    assert_eq!(located.rule.as_deref(), answer["rule"].as_str());
    assert_eq!(located.slot.as_deref(), answer["slot"].as_str());
    assert_eq!(located.id.as_deref(), answer["id"].as_str().filter(|s| !s.is_empty()));
}
