use cadence::review::{io, model, persistence, returns};

use serde_json::{Value, json};
struct FixedClock;
impl io::Clock for FixedClock {
    fn now(&mut self) -> u64 {
        100
    }
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/phase9/h4-returns.json")).unwrap()
}
/// The review records one snapshot holds.
fn records(review: Value) -> Value {
    persistence::records(&json!({"review":review})).unwrap()
}
fn decide(review: Value, submitted: &returns::ReturnSubmission) -> Result<returns::ReturnDecision, returns::ReturnError> {
    returns::decide_return(records(review), submitted, &mut FixedClock)
}
/// The receipt for a new closure, read from the records it would commit.
fn closed(decision: returns::ReturnDecision) -> returns::ReturnReceipt {
    match decision {
        returns::ReturnDecision::Close { records, admission, attempt } => {
            returns::receipt(&records, &admission, &attempt, false).unwrap()
        }
        returns::ReturnDecision::Replay(_) => panic!("a pending attempt was replayed"),
    }
}
fn submission(input: &Value, raw: Option<&[u8]>) -> returns::ReturnSubmission {
    returns::ReturnSubmission {
        identity: serde_json::from_value(input["identity"].clone()).unwrap(),
        launch: "launch1".into(),
        host_return: Some("return1".into()),
        raw: raw.map(Vec::from),
        host_failure: None,
        citations: vec![],
    }
}
fn accepted(input: &Value) -> Value {
    let mut records = input["pending"].clone();
    records["attempts"]["a1"] = input["accepted_attempt"].clone();
    records["closures"] = json!({"a1":input["closure"]});
    records["originals"] = json!({"o1":input["original"]});
    records["originals"]["o1"]["raw"] = json!(input["F"].as_str().unwrap().as_bytes());
    records["original_sequence"] = json!(1);
    records
}
#[test]
fn accept_replay_ac46() {
    let input = fixture();
    let submitted = submission(&input, Some(input["F"].as_str().unwrap().as_bytes()));
    let Ok(returns::ReturnDecision::Replay(result)) = decide(accepted(&input), &submitted) else {
        panic!("a closed attempt was not replayed")
    };
    assert_eq!(
        json!({"attempt":result.attempt,"findings":result.findings,"terminal":result.terminal,"replayed":result.replayed}),
        json!({"attempt":"a1","findings":{"digest":input["original"]["content"],"count":1},"terminal":"accepted","replayed":true})
    );
}
#[test]
fn accept_conflict_ac47() {
    let input = fixture();
    let submitted = submission(&input, Some(input["Changed"].as_str().unwrap().as_bytes()));
    assert_eq!(
        serde_json::to_value(decide(accepted(&input), &submitted).err().unwrap()).unwrap(),
        json!({"code":"conflicting-return","attempt":"a1","original":"o1"})
    );
}
#[test]
fn accept_missing_return_closes_failed() {
    let input = fixture();
    let result = closed(decide(input["pending"].clone(), &submission(&input, None)).unwrap());
    assert_eq!(
        json!({"terminal":result.terminal,"findings":result.findings,"terminal_count":result.durable_terminal_count}),
        json!({"terminal":"failed","findings":null,"terminal_count":1})
    );
}
#[test]
fn accept_malformed_return_closes_failed() {
    let input = fixture();
    let submitted = submission(&input, Some(b"{\"findings\":"));
    assert_eq!(
        closed(decide(input["pending"].clone(), &submitted).unwrap()).terminal,
        model::AttemptState::Failed
    );
}
