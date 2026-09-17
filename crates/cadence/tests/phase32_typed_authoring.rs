#[path = "support/phase31.rs"]
#[allow(dead_code)]
mod phase31;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs};
use phase31::{Client, ProcessFixture, approve, tree};

const PHASE: u32 = 31;

fn native_context(client: &mut Client) {
    let answer = client.call(
        "cadence_apply",
        approve(json!({"operation":"context-submit","submission":{
            "phase":PHASE,"title":"Typed plan authoring",
            "scope":"The binary owns plan rendering.",
            "durable_decisions":[],"decisions":[],"assumptions":[],
            "truths":[
                {"id":"T1","trigger":"the planner submits typed plan content",
                    "observer":"the planner","verb":"gets",
                    "outcome":"the rendered digest and no document",
                    "kind":"property","observable":true,"fixed_oracle":true},
                {"id":"T5","trigger":"the planner submits a Markdown body",
                    "observer":"the planner","verb":"gets",
                    "outcome":"a refusal naming the body slot",
                    "kind":"property","observable":true,"fixed_oracle":true}
            ]
        }})),
    );
    assert_eq!(answer["persisted"], true, "{answer}");
}

fn check(id: &str, truth: &str) -> Value {
    json!({
        "kind":"check","id":id,"reason":"The authoring boundary must retain its contract.",
        "spec":{"command":"python3 -B tests/tiny.py",
            "expected":{"kind":"property","value":"the authoring contract is observed"},
            "test":{"file":"tests/tiny.py","function":"Tiny.test_ok"},
            "setup":"A real fixture project.","call":"Call the real stdio server.",
            "boundary":"stdio JSON-RPC and the fixture filesystem","fakes":[]},
        "associations":[{"truth_id":truth,"truth_version":1,
            "reason":"This check observes the truth at the public authoring boundary."}]
    })
}

fn typed_submission(allocation: &Value) -> Value {
    let target = allocation["targets"][0].clone();
    json!({
        "phase":PHASE,"occurrence":allocation["occurrence"],
        "request_id":"phase32-typed-plan",
        "inventory_basis":allocation["inventory"]["basis"],
        "plans":[{"target":target,"content":{
            "phase":PHASE,"plan":target["plan"],
            "goal":"Render the complete plan from typed content.",
            "context":"The stdio authoring boundary is the observed surface.",
            "notes":"The installed bytes must match the reported revision.",
            "requirements":["T1","T5"],
            "files":["src/lease.rs","tests/tiny.py"],
            "tasks":[{"id":"typed-plan","title":"Exercise typed plan authoring",
                "files":["src/lease.rs","tests/tiny.py"],
                "action":"Submit typed pieces through the real binary.",
                "verify":["python3 -B tests/tiny.py"]}],
            "suite":"python3 -B tests/tiny.py",
            "evidence_map":{"mode":"attached","items":[
                check("fixture/T1", "T1"), check("fixture/T5", "T5")
            ]}
        }}]
    })
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value.as_object().unwrap().keys().map(String::as_str).collect()
}

fn is_digest(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

#[test]
fn phase32_draft_read_by_identity_and_approved_by_digest_installs_same_bytes() {
    let fixture = ProcessFixture::new();
    let project = fixture.project();
    let mut client = Client::open(project);
    native_context(&mut client);
    let allocation = client.call(
        "cadence_query",
        json!({"operation":"plan-read","phase":PHASE,"count":1}),
    );
    let draft = client.call(
        "cadence_apply",
        json!({"operation":"plan-submit","submission":typed_submission(&allocation)}),
    );
    assert_eq!(draft["status"], "ok", "{draft}");
    let identity = json!({
        "kind":"plan-draft","phase":PHASE,"plan":draft["documents"][0]["identity"]["plan"],
        "digest":draft["submission_digest"]
    });
    let index = client.call(
        "cadence_query",
        json!({"operation":"document","identity":identity}),
    );
    assert_eq!(index["status"], "ok", "{index}");
    let parts = index["parts"].as_array().unwrap();
    assert_eq!(parts.iter().map(|part| part["part"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["frontmatter", "goal", "truths", "context", "evidence-map",
            "tasks-heading", "task:typed-plan", "notes"]);
    let mut rendered = String::new();
    for part in parts {
        let slice = client.call(
            "cadence_query",
            json!({"operation":"document","identity":identity,"part":part["part"]}),
        );
        assert_eq!(slice["status"], "ok", "{slice}");
        assert_eq!(slice["revision"], draft["documents"][0]["revision"]);
        rendered.push_str(slice["body"].as_str().unwrap());
    }
    let published = client.call(
        "cadence_apply",
        json!({"operation":"plan-submit","phase":PHASE,"approval":{
            "approved":true,"owner":"Fixture Owner","at":"2026-09-17T12:00:00Z",
            "submission_digest":draft["submission_digest"]
        }}),
    );
    assert_eq!(published["persisted"], true, "{published}");
    let installed = fs::read(project.join(".planning/phases/31/PLAN-1.md")).unwrap();
    assert_eq!(installed, rendered.as_bytes());
    assert_eq!(format!("{:x}", Sha256::digest(&installed)), draft["documents"][0]["revision"]);
    client.finish();
}

#[test]
fn phase32_typed_context_answers_digest_and_no_document() {
    let fixture = ProcessFixture::new();
    let project = fixture.project();
    let mut client = Client::open(project);
    let submission = json!({
        "phase": PHASE,
        "title": "Typed context authoring",
        "scope": "The binary owns context rendering.",
        "durable_decisions": [{"id":"D-179","text":"Approval binds to the draft digest."}],
        "decisions": [{"id":"D-182","text":"The draft answer omits document bytes."}],
        "assumptions": ["The installed document is UTF-8 Markdown."],
        "truths": [{
            "id":"T2","trigger":"the context author submits typed context content",
            "observer":"the context author","verb":"gets",
            "outcome":"the rendered digest and no document",
            "kind":"literal","observable":true,"fixed_oracle":true
        }]
    });

    let draft = client.call(
        "cadence_apply",
        json!({"operation":"context-submit","submission":submission.clone()}),
    );
    assert_eq!(
        keys(&draft),
        BTreeSet::from([
            "status", "operation", "phase", "persisted", "validation", "submission_digest",
            "revision"
        ]),
        "{draft}",
    );
    assert_eq!(draft["status"], "ok", "{draft}");
    assert_eq!(draft["operation"], "context-submit", "{draft}");
    assert_eq!(draft["phase"], PHASE, "{draft}");
    assert_eq!(draft["persisted"], false, "{draft}");
    assert_eq!(draft["validation"], "draft", "{draft}");
    assert!(is_digest(&draft["submission_digest"]), "{draft}");
    assert!(is_digest(&draft["revision"]), "{draft}");

    let published = client.call(
        "cadence_apply",
        json!({"operation":"context-submit","submission":submission,"approval":{
            "approved":true,"owner":"Fixture Owner","at":"2026-09-17T12:00:00Z",
            "submission_digest":draft["submission_digest"]
        }}),
    );
    assert_eq!(published["persisted"], true, "{published}");
    let installed = fs::read(project.join(".planning/phases/31/CONTEXT.md")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&installed)),
        draft["revision"],
    );
    client.finish();
}

#[test]
fn phase32_typed_plan_answers_digest_and_no_document() {
    let fixture = ProcessFixture::new();
    let project = fixture.project();
    let mut client = Client::open(project);
    native_context(&mut client);
    let allocation = client.call(
        "cadence_query",
        json!({"operation":"plan-read","phase":PHASE,"count":1}),
    );
    let submission = typed_submission(&allocation);

    let draft = client.call(
        "cadence_apply",
        json!({"operation":"plan-submit","submission":submission.clone()}),
    );
    assert_eq!(
        keys(&draft),
        BTreeSet::from([
            "status", "operation", "persisted", "validation", "submission_digest", "documents"
        ]),
        "{draft}",
    );
    assert_eq!(draft["status"], "ok", "{draft}");
    assert_eq!(draft["operation"], "plan-submit", "{draft}");
    assert_eq!(draft["persisted"], false, "{draft}");
    assert_eq!(draft["validation"], "draft", "{draft}");
    assert!(is_digest(&draft["submission_digest"]), "{draft}");
    assert_eq!(draft["documents"].as_array().unwrap().len(), 1, "{draft}");
    assert_eq!(draft["documents"][0]["identity"], allocation["targets"][0]);
    assert!(is_digest(&draft["documents"][0]["revision"]), "{draft}");

    let published = client.call(
        "cadence_apply",
        json!({"operation":"plan-submit","submission":submission,"approval":{
            "approved":true,"owner":"Fixture Owner","at":"2026-09-17T12:00:00Z",
            "submission_digest":draft["submission_digest"]
        }}),
    );
    assert_eq!(published["persisted"], true, "{published}");
    let installed = fs::read(project.join(".planning/phases/31/PLAN-1.md")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&installed)),
        draft["documents"][0]["revision"],
    );
    client.finish();
}

#[test]
fn phase32_plan_body_is_refused() {
    let fixture = ProcessFixture::new();
    let project = fixture.project();
    let mut client = Client::open(project);
    native_context(&mut client);
    let allocation = client.call(
        "cadence_query",
        json!({"operation":"plan-read","phase":PHASE,"count":1}),
    );
    let mut submission = typed_submission(&allocation);
    submission["plans"][0]["content"]["body"] = json!("# Caller-owned Markdown\n");
    let before = tree(project);

    let answer = client.call(
        "cadence_apply",
        json!({"operation":"plan-submit","submission":submission}),
    );
    assert_eq!(answer["status"], "refused", "{answer}");
    assert_eq!(answer["code"], "typed-content", "{answer}");
    assert_eq!(answer["slot"], "submission.plans[0].content.body", "{answer}");
    assert_eq!(answer["phase"], PHASE, "{answer}");
    assert_eq!(tree(project), before);
    client.finish();
}
