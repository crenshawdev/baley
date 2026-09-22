//! Core shard of the executable phase-6 acceptance inventory.
//!
//! Each test-binary shard checks the harness registry and then invokes every
//! mapped evidence function. A source-level test name is never counted as a
//! pass. Real-host invocation, a real model executor, actual host permission
//! denial and model-produced work remain PLAN-2 UAT obligations, not Cargo
//! claims.
use super::{
    patch::parse_executor_patch,
    plan::{PlanGraph, parse_plan},
};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn capture_retains_result_lines_past_prefix() {
    let mut output = vec![b'x'; 65_537];
    output.extend_from_slice(b"\ntest oversized::late ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n");
    let capture = super::runner::capture(output.as_slice());
    assert_eq!(capture.bytes.len(), 65_536);
    assert!(!capture.complete);
    assert_eq!(capture.result_lines, vec!["test oversized::late ... FAILED",
        "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out"]);
}

// D-168 and the run classifier both read cargo test's own lines. nextest, which
// the suite runs under since D-176, writes its summary to stderr with a leading
// indent and repeats cargo's `test result:` line indented under a failure, so
// every nextest run came back Unknown and cost the owner a classification.
#[test]
fn classify_reads_nextest_summaries_and_keeps_their_result_lines() {
    use super::{receipts::{Observation, Summary}, runner::{capture, classify, valid_result_line}};
    let empty = capture(&b""[..]);
    let green = capture(&b"    Starting 1 test across 1 binary (1 test skipped)\n        PASS [   0.062s] (1/1) cadence::phase32_typed_authoring phase32_plan_body_is_refused\n     Summary [   0.062s] 1 test run: 1 passed, 1 skipped\n"[..]);
    assert_eq!(classify(&empty, &green), Observation::ResultsObserved { summary: Summary::Cargo { failed: false } });
    let red = capture(&b"        FAIL [   0.065s] (1/1) cadence::phase32_typed_authoring phase32_plan_body_is_refused\n    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.06s\n     Summary [   0.066s] 1 test run: 0 passed, 1 failed, 1 skipped\nerror: test run failed\n"[..]);
    assert_eq!(classify(&empty, &red), Observation::ResultsObserved { summary: Summary::Cargo { failed: true } });
    let summary_only_red = capture(&b"     Summary [   0.066s] 2 tests run: 1 passed, 1 failed\n"[..]);
    assert_eq!(classify(&empty, &summary_only_red), Observation::ResultsObserved { summary: Summary::Cargo { failed: true } });
    for line in ["     Summary [   0.062s] 1 test run: 1 passed, 1 skipped",
        "        PASS [   0.062s] (1/1) cadence::phase32_typed_authoring phase32_plan_body_is_refused",
        "        FAIL [   0.065s] (1/1) cadence::phase32_typed_authoring phase32_plan_body_is_refused",
        "    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.06s"] {
        assert!(valid_result_line(line), "{line}");
    }
    assert!(!valid_result_line("    Starting 1 test across 1 binary (1 test skipped)"));
    assert!(!valid_result_line("error: test run failed"));
}

// A retained run classified Unknown by an older binary is still the record of
// what that binary saw. A later classifier that recognizes those bytes may not
// refuse the record; a retained result that claims more than the bytes say still is.
#[test]
fn retained_unknown_observation_stays_valid_when_the_classifier_learns_its_lines() {
    use super::{receipts::{Observation, Summary}, runner::{capture, observation_consistent}};
    let empty = capture(&b""[..]);
    let green = capture(&b"     Summary [   0.062s] 1 test run: 1 passed, 1 skipped\n"[..]);
    assert!(observation_consistent(&Observation::Unknown, &empty, &green));
    let observed = Observation::ResultsObserved { summary: Summary::Cargo { failed: false } };
    assert!(observation_consistent(&observed, &empty, &green));
    let claimed = Observation::ResultsObserved { summary: Summary::Cargo { failed: true } };
    assert!(!observation_consistent(&claimed, &empty, &green));
    let custom = capture(&b"custom output\n"[..]);
    assert!(!observation_consistent(&observed, &empty, &custom));
}

#[test]
fn later_answered_launch_supersedes_only_matching_unanswered_runs() {
    use super::{allocation::Check, history::{self, Event, Record, Request, Task}, receipts::*, runner::capture};
    let task = Task { phase: 33, occurrence: "active-cycle:phase:33".into(), admission_digest: "admitted".into(),
        plan: 3, task: "P33-3-T3".into() };
    let launch = |id: &str| Launch { run_id: id.into(), check: None, stage: Stage::Verify,
        material: Material { command: "cargo nextest run -p cadence --test mcp".into(), commit: "f74d252d".into(),
            tree: "material-tree".into(), test_file: String::new(), test_digest: String::new() }, launched_at: 1 };
    let result = |id: &str| Event::Result(RunResult { run_id: id.into(), disposition: Disposition::Exited { code: 0 },
        stdout: capture(&b""[..]), stderr: capture(&b""[..]), observed_at: 2,
        observation: Observation::Unknown, material_unchanged: true });
    let record = |version: u64, event: Event| {
        let request = Request { request_id: format!("event-{version}"), task: task.clone(), attempt: "attempt".into(),
            expected_version: version - 1, event };
        Record { schema: "native-task-event-1".into(), root_binding: "fixture".into(), version,
            request_digest: history::request_digest(&request).unwrap(), request }
    };
    let check = Check { id: "check/mcp".into(), item_revision: "revision-1".into() };
    for case in ["different command", "different stage", "different check", "different check revision", "different attempt",
        "different task", "unanswered retry", "older result", "matching null check", "matching check"] {
        let mut first = launch("dead-launch");
        let mut second = launch("resumed-launch");
        match case {
            "different command" => second.material.command = "cargo nextest run -p cadence --test phase33_verification".into(),
            "different stage" => second.stage = Stage::Green,
            "different check" => second.check = Some(check.clone()),
            "different check revision" | "matching check" => {
                first.check = Some(check.clone());
                second.check = Some(check.clone());
                if case == "different check revision" { second.check.as_mut().unwrap().item_revision = "revision-2".into(); }
            }
            _ => {}
        }
        let mut records = vec![record(1, Event::Launch(first)), record(2, Event::Launch(second))];
        assert_eq!(history::project(&records[..1], &task).unknown_runs, ["dead-launch"], "{case}");
        assert_eq!(history::project(&records, &task).unknown_runs, ["dead-launch", "resumed-launch"], "{case}");
        if case != "unanswered retry" {
            records.push(record(3, result(if case == "older result" { "dead-launch" } else { "resumed-launch" })));
        }
        for record in &mut records[1..] {
            if case == "different attempt" { record.request.attempt = "another-attempt".into(); }
            if case == "different task" { record.request.task.task = "another-task".into(); }
            record.request_digest = history::request_digest(&record.request).unwrap();
        }
        let retained = serde_json::to_vec(&records).unwrap();
        let expected = match case {
            "matching null check" | "matching check" => vec![],
            "unanswered retry" => vec!["dead-launch", "resumed-launch"],
            "older result" => vec!["resumed-launch"],
            _ => vec!["dead-launch"],
        };
        assert_eq!(history::project(&records, &task).unknown_runs, expected, "{case}");
        assert_eq!(serde_json::to_vec(&records).unwrap(), retained, "projection must preserve retained records");
    }
}

// D-168: a suite receipt always carries every failing test's name. nextest
// names a failure as `FAIL [ time ] (n/m) crate::binary test`, indented, and
// repeats the name in its final list; the repair question for suite-p32-1
// carried no names from nineteen failures.
#[test]
fn failing_tests_reads_nextest_fail_lines_once_each() {
    use super::{history::failing_tests, receipts::{Disposition, Observation, RunResult}, runner::capture};
    let stderr = capture(&b"        FAIL [   0.299s] ( 351/1065) cadence::phase12_execution phase12_acknowledged_progress_survives_restart\n    test phase12_acknowledged_progress_survives_restart ... FAILED\n        FAIL [   0.268s] ( 362/1065) cadence::phase13_close phase13_rules_gate_retirement_rehearsal\n     Summary [ 120.000s] 1065 tests run: 1046 passed, 19 failed, 2 skipped\n        FAIL [   0.299s] ( 351/1065) cadence::phase12_execution phase12_acknowledged_progress_survives_restart\n        FAIL [   0.268s] ( 362/1065) cadence::phase13_close phase13_rules_gate_retirement_rehearsal\nerror: test run failed\n"[..]);
    let result = RunResult { run_id: "suite".into(), disposition: Disposition::Exited { code: 100 }, stdout: capture(&b""[..]),
        stderr, observed_at: 1, observation: Observation::Unknown, material_unchanged: true };
    assert_eq!(failing_tests(&result), vec![
        "phase12_execution phase12_acknowledged_progress_survives_restart".to_owned(),
        "phase13_close phase13_rules_gate_retirement_rehearsal".to_owned()]);
}

// Constructed unit authority, not a claim of approval through the public API.
// The acceptance check separately supplies that boundary with real stdio calls.
fn native_unit_contract(command: &str) -> (serde_json::Value, std::collections::BTreeMap<String, String>, super::admission::Contract) {
    use crate::{plan::{model::*, evidence::Map}, store::model::digest};
    let truths = ["truth/A", "truth/B"].map(|id| json!({"id":id,"trigger":"the sender sends the parcel",
        "observer":"the recipient","verb":"gets","outcome":"a receipt","kind":"property",
        "observable":true,"fixed_oracle":true}));
    let submission = json!({"phase":12,"title":"Delivery","scope":"Approved delivery.","decisions":[],
        "durable_decisions":[],"assumptions":[],"truths":truths});
    let approval = json!({"approved":true,"owner":"Fixture Owner","at":"2026-09-10T14:00:00Z","submission":submission});
    let context = crate::context::persistence::approved(serde_json::from_value(submission).unwrap(), serde_json::from_value(approval).unwrap()).unwrap();
    let edges = ["truth/A", "truth/B"].map(|id| json!({"truth_id":id,"truth_version":1,"reason":"Delivery provides the receipt."}));
    let map: Map = serde_json::from_value(json!({"mode":"attached","items":[{
        "kind":"check","id":"check/shared","spec":{"command":command,"expected":{"kind":"literal","value":"receipt"},
        "test":{"file":"tests/not_yet_written.rs","function":"delivery"},"setup":"","call":"","boundary":"","fakes":[]},
        "reason":"Removing delivery loses the receipt.","associations":edges},
        {"kind":"artifact","id":"artifact/delivery","spec":{"locators":["src/delivery.rs"],"substance":"Delivery exists."},
        "reason":"Delivery needs an implementation.","associations":edges}]})).unwrap();
    let body = format!("# Delivery\n## Evidence map\n\n```json\n{}\n```\n\n", serde_json::to_string_pretty(&map).unwrap());
    // Blank check commands must reach admission with valid task metadata.
    let verify = if command.is_empty() { "printf verified" } else { command };
    let entries = (1..=2).map(|n| serde_json::from_value(json!({"target":{"phase":12,"plan":n},"content":{
        "phase":12,"plan":n,"requirements":["truth/A","truth/B"],"files":["src/delivery.rs"],"directories":[],
        "execution":{"schema":1,"suite":"printf suite","tasks":[{"id":"deliver","verify":[verify]},
        {"id":"document","verify":["printf documented"]}]},"body":body,"evidence_map":map}})).unwrap()).collect();
    let submission = Submission { phase: 12.try_into().unwrap(), occurrence:"active-cycle:phase:12".into(),
        request_id:"unit-publication".into(), inventory_basis:"unit-inventory".into(), plans:entries };
    let approval = Approval { approved:true, owner:Some("Fixture Owner".into()), at:Some("2026-09-10T14:00:00Z".into()), submission:Some(submission.clone()), submission_digest:None };
    let mut documents = std::collections::BTreeMap::new();
    let mut publications = std::collections::BTreeMap::new();
    let mut revisions = Vec::new();
    let mut bindings = Vec::new();
    let payload = crate::plan::persistence::payload_digest(&submission, &approval).unwrap();
    for entry in &submission.plans {
        let bytes = crate::plan::render::document(&entry.content).unwrap();
        let content_revision = digest(&bytes);
        let map_revision = crate::plan::map_history::event_id(&submission, &entry.target).unwrap();
        let Map::Attached { items } = &map else { unreachable!() };
        let item_revisions = items.iter().map(|i| (i.id().into(), digest(&serde_json::to_vec(&crate::plan::map_history::definition(i).unwrap()).unwrap()))).collect();
        revisions.push(crate::plan::map_history::Revision { revision:map_revision.clone(), occurrence:submission.occurrence.clone(),
            request_id:submission.request_id.clone(), payload_digest:payload.clone(), identity:entry.target.clone(),
            content_revision:content_revision.clone(),items:items.clone(),item_revisions });
        publications.insert(entry.target.plan.get(), Publication { identity:entry.target.clone(), occurrence:submission.occurrence.clone(),
            revision:content_revision.clone(),content:entry.content.clone(),approval:approval.clone(),readiness:Readiness::ProvisionalAuthoring,
            history:vec![content_revision.clone()],map_revision:Some(map_revision.clone()) });
        documents.insert(format!("phases/12/PLAN-{}.md",entry.target.plan), String::from_utf8(bytes).unwrap());
        bindings.push(super::admission::Binding {plan:entry.target.plan.get(),publication_request:submission.request_id.clone(),content_revision,map_revision});
    }
    let receipt = Receipt {payload_digest:payload,results:publications.values().cloned().collect()};
    let occurrence = Occurrence {id:submission.occurrence.clone(),phase:12,cycle:"active".into(),high_water:2,consumed:vec![1,2],
        provenance:Default::default(),publications,receipts:std::collections::BTreeMap::from([(submission.request_id,receipt)])};
    let allocation = (1..=2).flat_map(|plan| ["deliver","document"].map(|task| super::allocation::Assignment {
        plan,task:task.into(),checks:if plan == 1 && task == "deliver" {vec![super::allocation::Check {
            id:"check/shared".into(),item_revision:revisions[0].item_revisions["check/shared"].clone()}]} else {vec![]}
    })).collect();
    let data = json!({"context":{"schema":"context-1","phases":{"12":context}},
        "plan_publications":{"schema":"plan-1","phases":{"12":occurrence}},
        "acceptance_maps":{"schema":"acceptance-map-1","phases":{"12":{"occurrence":submission.occurrence,"revisions":revisions}}}});
    (data, documents, super::admission::Contract {phase:12,occurrence:submission.occurrence,plans:bindings,allocation})
}

#[test]
fn native_admission_validates_authority_and_allocation() {
    use super::admission::{decode,validate};
    let (data, documents, contract) = native_unit_contract("custom-delivery-check");
    let valid = validate(&data,&documents,&contract).unwrap();
    assert_eq!(valid.plans.iter().map(|p| p.plan).collect::<Vec<_>>(), vec![1,2]);
    assert_eq!(valid.plans[0].tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["deliver","document"]);
    assert_eq!(valid.maps.len(),2);
    assert_eq!(contract.allocation[1].checks,vec![]);
    let assert_refusal = |error:crate::store::Error, rule:&str, slot:&str, id:&str| {
        let crate::store::Error::Invalid(message) = error else {panic!("expected located invalid: {error}")};
        let diagnostic:crate::plan::model::Diagnostic = serde_json::from_str(message.strip_prefix("plan-refusal:").unwrap()).unwrap();
        assert_eq!(diagnostic.rule,rule);
        assert_eq!(diagnostic.slot,slot);
        if !id.is_empty() {assert_eq!(diagnostic.id.as_deref(),Some(id));}
    };
    for field in ["phase","occurrence","plans","allocation"] {
        let mut raw=serde_json::to_value(&contract).unwrap(); raw.as_object_mut().unwrap().remove(field);
        assert_refusal(decode(raw).unwrap_err(),"admission-shape",&format!("contract.{field}"),"");
    }
    for (pointer,value,rule,slot) in [
        ("/context",serde_json::Value::Null,"native-approved-truths","context"),
        ("/context/phases/12/approval/approved",json!(false),"native-approved-truths","context.approval"),
        ("/context/phases/12/truths/0/version",json!(2),"native-approved-truths","context.approval"),
        ("/context/phases/12/submission/truths/0/outcome",json!("another outcome"),"native-approved-truths","context.approval"),
        ("/plan_publications/phases/12/receipts/unit-publication/payload_digest",json!("stale"),"publication-authority","current.plans[1].receipt"),
        ("/acceptance_maps/phases/12/revisions/0/item_revisions/check~1shared",json!("stale"),"map-authority","current.plans[1].item_revision"),
        ("/acceptance_maps/phases/12/revisions/0/content_revision",json!("stale"),"map-authority","current.plans[1].map_revision"),
    ] {
        let mut changed=data.clone();
        if pointer=="/context" {changed.as_object_mut().unwrap().remove("context");}
        else {*changed.pointer_mut(pointer).unwrap()=value;}
        assert_refusal(validate(&changed,&documents,&contract).unwrap_err(),rule,slot,"");
    }
    let mut drift=documents.clone(); drift.get_mut("phases/12/PLAN-1.md").unwrap().push('\n');
    assert_refusal(validate(&data,&drift,&contract).unwrap_err(),"installed-plan","phases/12/PLAN-1.md","1");
    let (blank_data,blank_docs,blank_contract)=native_unit_contract("");
    assert_refusal(validate(&blank_data,&blank_docs,&blank_contract).unwrap_err(),"check-command","current.plans[1].evidence_map.items[0].spec.command","check/shared");
    for (n,rule,slot,id) in [
        (0,"allocation-task","contract.allocation","deliver"),
        (1,"allocation-task","contract.allocation","document"),
        (2,"allocation-task","contract.allocation[0]","unknown"),
        (3,"allocation-item","contract.allocation[0].checks[0].id","unknown"),
        (4,"allocation-kind","contract.allocation[0].checks[0].id","artifact/delivery"),
        (5,"allocation-revision","contract.allocation[0].checks[0].item_revision","check/shared"),
        (6,"allocation-check","contract.allocation","check/shared"),
        (7,"allocation-owner","contract.allocation[2].checks[0]","check/shared"),
    ] {
        let mut changed=contract.clone();
        match n {
            0=>changed.allocation.clear(),1=>{changed.allocation.remove(1);},2=>changed.allocation[0].task="unknown".into(),
            3=>changed.allocation[0].checks[0].id="unknown".into(),4=>changed.allocation[0].checks[0].id="artifact/delivery".into(),
            5=>changed.allocation[0].checks[0].item_revision="stale".into(),6=>changed.allocation[0].checks.clear(),
            7=>changed.allocation[2].checks=changed.allocation[0].checks.clone(),_=>unreachable!(),
        }
        assert_refusal(validate(&data,&documents,&changed).unwrap_err(),rule,slot,id);
    }
}

#[test]
fn native_admission_refuses_check_command_outside_task_verify() {
    use super::admission::validate;
    let command = "custom-delivery-check";
    let (data, documents, mut contract) = native_unit_contract(command);
    validate(&data, &documents, &contract).unwrap();
    // The document task names only `printf documented`; delivery's command
    // exists on another task, but cannot launch under this closing owner.
    contract.allocation[1].checks = std::mem::take(&mut contract.allocation[0].checks);
    let error = validate(&data, &documents, &contract).unwrap_err();
    let crate::store::Error::Invalid(message) = error else { panic!("expected located invalid: {error}") };
    let diagnostic: crate::plan::model::Diagnostic = serde_json::from_str(message.strip_prefix("plan-refusal:").unwrap()).unwrap();
    assert_eq!(diagnostic.rule, "check-command-verify");
    assert_eq!(diagnostic.slot, "contract.allocation");
    assert_eq!(diagnostic.phase, Some(12));
    assert_eq!(diagnostic.id.as_deref(), Some("document"));
    assert!(diagnostic.reason.contains("check/shared"), "{}", diagnostic.reason);
    assert!(diagnostic.reason.contains(command), "{}", diagnostic.reason);
}

#[test]
fn native_records_outlive_the_directory_identity_they_were_stamped_with() {
    let process = &mut cadence::process::System;
    // A reboot or restore gives .planning new device and inode numbers at the
    // same path. The records a store retains were stamped under the old
    // identity; the identity is provenance, never a key the next process must
    // reproduce, so admissions, task events and plan events still bind.
    use super::{admission, history::{self, Event, Request, Task}};
    let (data, documents, contract) = native_unit_contract("custom-delivery-check");
    let before_reboot = "46:302462;46:302461;46:1;36:256;";
    let after_reboot = "46:302568;46:302566;46:1;36:256;";
    let admit = admission::Request { request_id: "admit".into(), expected_set_version: 0, contract };
    let (data, basis) = admission::contribute(&data, &documents, before_reboot, &admit).unwrap();
    assert_eq!(basis.root_binding, before_reboot);
    assert_eq!(admission::replay(&data, &admit).unwrap(), Some(basis.clone()));
    let task = Task { phase: 12, occurrence: "active-cycle:phase:12".into(), admission_digest: basis.request_digest.clone(),
        plan: 1, task: "deliver".into() };
    let check = basis.request.contract.allocation[0].checks[0].clone();
    let start = Request { request_id: "event-0".into(), task: task.clone(), attempt: "attempt-1".into(), expected_version: 0,
        event: Event::Attempt { predecessor: None, checks: vec![check], base_commit: "unit-base".into() } };
    let (data, event) = history::contribute(&data, after_reboot, &start, process).unwrap();
    assert_eq!(event.root_binding, after_reboot);
    assert_eq!(history::replay(&data, &start).unwrap(), Some(event));
}

fn schema_fixture() -> serde_json::Value {
    json!({
        "schema": 1, "kind": "executor", "dispatch_id": "dispatch-1",
        "expected_execution_version": 1, "outcome": "blocked",
        "tasks": [
            {"status":"completed","task_id":"T1","commit":"abc",
             "verification":{"disposition":"passed","commands":[
                 {"command":"verify","exit_code":0,"output_digest":"digest"}]},
             "evidence":[{"kind":"commit","sha":"abc"},
                 {"kind":"file-line","path":"src/a.rs","line":1},
                 {"kind":"criterion","id":"AC1"}]},
            {"status":"blocked","task_id":"T2","blocker_id":"B1"},
            {"status":"not-run","task_id":"T3"}
        ],
        "deviations":[{"id":"D1","text":"judgment","evidence":[{"kind":"criterion","id":"AC1"}]}],
        "blockers":[{"id":"B1","text":"judgment","evidence":[{"kind":"file-line","path":"src/a.rs","line":2}]}]
    })
}

fn resolve_schema<'a>(
    root: &'a serde_json::Value,
    node: &'a serde_json::Value,
) -> &'a serde_json::Value {
    match node.get("$ref") {
        Some(reference) => resolve_schema(
            root,
            root.pointer(reference.as_str().unwrap().strip_prefix('#').unwrap())
                .unwrap(),
        ),
        None => node,
    }
}

// Evaluate only the structural keywords generated for this contract. Field
// inventories below also check each independently authored object in full.
fn schema_accepts(
    root: &serde_json::Value,
    node: &serde_json::Value,
    value: &serde_json::Value,
) -> bool {
    let node = resolve_schema(root, node);
    if let Some(variants) = node.get("oneOf") {
        return variants
            .as_array()
            .unwrap()
            .iter()
            .filter(|variant| schema_accepts(root, variant, value))
            .count()
            == 1;
    }
    if node.get("const").is_some_and(|expected| expected != value)
        || node
            .get("enum")
            .is_some_and(|values| !values.as_array().unwrap().contains(value))
    {
        return false;
    }
    match node.get("type").and_then(|value| value.as_str()) {
        Some("object") => value.as_object().is_some_and(|object| {
            let properties = node["properties"].as_object().unwrap();
            node["required"]
                .as_array()
                .unwrap()
                .iter()
                .all(|key| object.contains_key(key.as_str().unwrap()))
                && object.iter().all(|(key, value)| match properties.get(key) {
                    Some(property) => schema_accepts(root, property, value),
                    None => node["additionalProperties"] != false,
                })
        }),
        Some("array") => value.as_array().is_some_and(|values| {
            values
                .iter()
                .all(|value| schema_accepts(root, &node["items"], value))
        }),
        Some("string") => value.is_string(),
        Some("integer") => {
            (value.is_u64() || value.is_i64())
                && node
                    .get("minimum")
                    .is_none_or(|min| value.as_f64().unwrap() >= min.as_f64().unwrap())
                && node
                    .get("maximum")
                    .is_none_or(|max| value.as_f64().unwrap() <= max.as_f64().unwrap())
        }
        None => true,
        unexpected => panic!("unexpected schema type {unexpected:?}"),
    }
}

fn inspect_schema_objects(
    root: &serde_json::Value,
    node: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
    paths: &mut Vec<String>,
) {
    let mut node = resolve_schema(root, node);
    if let Some(variants) = node.get("oneOf") {
        node = variants
            .as_array()
            .unwrap()
            .iter()
            .find(|node| schema_accepts(root, node, value))
            .unwrap();
    }
    if let Some(object) = value.as_object() {
        assert_eq!(node["additionalProperties"], false, "{path}");
        let actual: BTreeSet<_> = object.keys().map(String::as_str).collect();
        let properties: BTreeSet<_> = node["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let required: BTreeSet<_> = node["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap())
            .collect();
        assert_eq!(actual, properties, "{path}");
        assert_eq!(actual, required, "{path}");
        paths.push(path.to_owned());
        for (key, value) in object {
            inspect_schema_objects(
                root,
                &node["properties"][key],
                value,
                &format!("{path}/{key}"),
                paths,
            );
        }
    } else if let Some(array) = value.as_array() {
        for (index, value) in array.iter().enumerate() {
            inspect_schema_objects(
                root,
                &node["items"],
                value,
                &format!("{path}/{index}"),
                paths,
            );
        }
    }
}

#[test]
fn patch_schema_all_variants_and_nested_field_inventories_match_deserialization() {
    let schema = super::model::patch_schema();
    let fixture = schema_fixture();
    assert!(schema_accepts(&schema, &schema, &fixture));
    parse_executor_patch(fixture.clone()).unwrap();
    let mut paths = Vec::new();
    inspect_schema_objects(&schema, &schema, &fixture, "", &mut paths);
    assert_eq!(paths.len(), 13);
    for path in paths {
        let object = fixture.pointer(&path).unwrap().as_object().unwrap();
        for key in object.keys() {
            let mut missing = fixture.clone();
            missing
                .pointer_mut(&path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(
                !schema_accepts(&schema, &schema, &missing),
                "missing {path}/{key}"
            );
            assert!(
                parse_executor_patch(missing).is_err(),
                "missing {path}/{key}"
            );
            let mut wrong = fixture.clone();
            wrong.pointer_mut(&path).unwrap()[key] = json!(false);
            assert!(
                !schema_accepts(&schema, &schema, &wrong),
                "type {path}/{key}"
            );
            assert!(parse_executor_patch(wrong).is_err(), "type {path}/{key}");
        }
        let mut extra = fixture.clone();
        extra.pointer_mut(&path).unwrap()["unexpected"] = json!(true);
        assert!(!schema_accepts(&schema, &schema, &extra), "extra {path}");
        assert!(parse_executor_patch(extra).is_err(), "extra {path}");
    }
}

#[test]
fn patch_schema_rejects_incorrect_union_tags_and_integer_types() {
    let schema = super::model::patch_schema();
    for path in [
        "/kind",
        "/outcome",
        "/tasks/0/status",
        "/tasks/1/status",
        "/tasks/2/status",
        "/tasks/0/verification/disposition",
        "/tasks/0/evidence/0/kind",
        "/tasks/0/evidence/1/kind",
        "/tasks/0/evidence/2/kind",
        "/deviations/0/evidence/0/kind",
        "/blockers/0/evidence/0/kind",
        "/schema",
        "/expected_execution_version",
        "/tasks/0/verification/commands/0/exit_code",
        "/tasks/0/evidence/1/line",
    ] {
        let mut fixture = schema_fixture();
        *fixture.pointer_mut(path).unwrap() = json!("invalid-tag-or-integer");
        assert!(!schema_accepts(&schema, &schema, &fixture), "{path}");
        assert!(parse_executor_patch(fixture).is_err(), "{path}");
    }
}

#[test]
fn patch_schema_keeps_structural_and_semantic_admission_separate() {
    let schema = super::model::patch_schema();
    let mut fixture = schema_fixture();
    fixture["schema"] = json!(2);
    fixture["tasks"][0]["verification"]["disposition"] = json!("failed");
    fixture["tasks"][0]["evidence"] = json!([]);
    assert!(schema_accepts(&schema, &schema, &fixture));
    let patch = parse_executor_patch(fixture).unwrap();
    assert_eq!(
        super::patch::apply_executor_patch(&json!({}), &patch)
            .unwrap_err()
            .code,
        "unsupported-patch-schema"
    );
}

#[test]
fn executor_never_requests_the_gates_the_orchestrator_owns() {
    // D-171: the suite and plan completion are the orchestrator's requests. The
    // executor closes its last task and reports. D-170: a committed path outside
    // the lease is retained as a deviation, never refused.
    let executor = crate::execution::instructions::dispatch_text();
    let frontdoor = crate::execution::instructions::frontdoor_markdown();
    assert!(executor.contains("The executor never requests `execution-suite` or `execution-plan-complete`"),
        "the compiled executor text must hand the gates to the orchestrator");
    assert!(!executor.contains("Every path in an evidence or completion commit"),
        "D-170: the lease paragraph still promises a refusal");
    assert!(executor.contains("retained as a deviation on the plan record"),
        "D-170: the lease paragraph must name the retained deviation");
    assert!(frontdoor.contains("request `execution-suite`"), "the front door must request the suite itself");
    assert!(frontdoor.contains("then request `execution-plan-complete`"), "the front door must complete the plan itself");
}

#[test]
fn suite_repair_is_plan_level_single_use_and_retains_deviations() {
    let question = json!({"kind":"suite-repair-question","id":"suite-repair:run-1",
        "failed_run":"run-1","failing_tests":["repair::alpha"],
        "proposed_paths":["src/delivery.rs","docs/outside.md"]});
    let event = serde_json::from_value::<super::history::PlanEvent>(question.clone())
        .expect("the plan history must accept a suite repair question");
    assert_eq!(serde_json::to_value(event).unwrap(), question);
    let answer = json!({"kind":"suite-repair-answer","question_id":"suite-repair:run-1",
        "owner":"Fixture Owner","at":"2026-09-15T18:00:00Z","disposition":"approve"});
    assert!(serde_json::from_value::<super::history::PlanEvent>(answer).is_ok(),
        "the owner answer must be a plan event, never a task checkpoint");
    let repair = json!({"kind":"suite-repair","question_id":"suite-repair:run-1",
        "commits":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        "changed_paths":{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":["docs/outside.md"]}});
    assert!(serde_json::from_value::<super::history::PlanEvent>(repair).is_ok(),
        "the Git-observed repair must be retained at plan level");
}

#[test]
fn ac2_strict_plan_and_overlap_selection_are_executable_evidence() {
    let source = |plan, files: &str, body: &str| {
        format!(
            "---\nphase: 6\nplan: {plan}\nrequirements: [AC2]\nfiles: [{files}]\nexecution:\n  schema: 1\n  suite: cargo test\n  tasks:\n    - id: T{plan}\n      verify: [cargo test task{plan}]\n---\n{body}"
        )
    };
    let first = parse_plan(source(1, "src/a.rs", "opaque 日本語\n").as_bytes(), 6, 1).unwrap();
    let second = parse_plan(source(2, "src/a.rs", "second\n").as_bytes(), 6, 2).unwrap();
    let graph = PlanGraph::build(&[first.clone(), second]).unwrap();
    assert_eq!(first.body, "opaque 日本語\n");
    assert_eq!(graph.ready(&BTreeSet::new()), [1]);
    assert_eq!(graph.ready(&BTreeSet::from([1])), [2]);
    assert_eq!(graph.next_ready(&BTreeSet::new()), Some(1));
    assert_eq!(graph.next_ready(&BTreeSet::from([1])), Some(2));
}

#[test]
fn ac5_unknown_patch_keys_are_executable_evidence() {
    let value = json!({
        "schema": 1,
        "kind": "executor",
        "dispatch_id": "dispatch",
        "expected_execution_version": 1,
        "outcome": "complete",
        "tasks": [],
        "deviations": [],
        "blockers": [],
        "unknown": true,
    });
    assert_eq!(
        parse_executor_patch(value).unwrap_err().code,
        "invalid-patch"
    );
}
