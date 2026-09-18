#[path = "support/phase31.rs"]
mod support;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::{Client, ProcessFixture, observed_plan_review, publish_review_plan};

#[test]
fn phase33_review_return_answers_digest_and_count() {
    let fixture = ProcessFixture::new();
    let mut client = Client::open(fixture.project());
    publish_review_plan(&mut client);
    let mut request = observed_plan_review(&mut client, "typed");
    let findings = json!([
        {"file":"src/lease.rs","line":1,"severity":"P1","claim":"First claim","failure_scenario":"First failure"},
        {"file":"src/lease.rs","line":2,"severity":"P2","claim":"Second claim","failure_scenario":"Second failure"},
        {"file":"src/lease.rs","line":3,"severity":"P3","claim":"Third claim","failure_scenario":"Third failure"}
    ]);
    request["findings"] = findings.clone();
    // The digest preimage is the retained object envelope, in Finding struct
    // field order, with no whitespace; it equals Original.content.
    let canonical = concat!("{\"findings\":[",
        "{\"file\":\"src/lease.rs\",\"line\":1,\"severity\":\"P1\",\"claim\":\"First claim\",\"failure_scenario\":\"First failure\"},",
        "{\"file\":\"src/lease.rs\",\"line\":2,\"severity\":\"P2\",\"claim\":\"Second claim\",\"failure_scenario\":\"Second failure\"},",
        "{\"file\":\"src/lease.rs\",\"line\":3,\"severity\":\"P3\",\"claim\":\"Third claim\",\"failure_scenario\":\"Third failure\"}]}");
    let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    let answer = client.call("cadence_apply", request.clone());
    let expected = json!({"status":"ok","operation":"review-return","result":{
        "attempt":request["identity"]["attempt"],"terminal":"accepted","replayed":false,
        "findings":{"digest":digest,"count":3},"durable_terminal_count":1,"next_selection":null}});
    assert_eq!(answer, expected);
    let mut replay = expected.clone();
    replay["result"]["replayed"] = json!(true);
    assert_eq!(client.call("cadence_apply", request.clone()), replay);
    let attempt = client.call("cadence_query", json!({"operation":"review-attempt","attempt":request["identity"]["attempt"]}));
    let original = client.call("cadence_query", json!({"operation":"review-original","original":attempt["result"]["original"]}));
    assert_eq!(original["result"]["original"]["content"], digest);
    assert!(original["result"].get("raw_bytes").is_none(), "{original}");
    let mut malformed = observed_plan_review(&mut client, "malformed");
    malformed["findings"] = findings;
    malformed["findings"][1].as_object_mut().unwrap().remove("severity");
    let refused = client.call("cadence_apply", malformed);
    assert_eq!(refused["status"], "refused", "{refused}");
    assert_eq!(refused["code"], "invalid-return", "{refused}");
    assert_eq!(refused["slot"], "findings[1].severity", "{refused}");
    request.as_object_mut().unwrap().remove("findings");
    request["raw"] = Value::String(canonical.into());
    let refused = client.call("cadence_apply", request);
    assert_eq!(refused["code"], "typed-content", "{refused}");
    assert_eq!(refused["slot"], "raw", "{refused}");
}
