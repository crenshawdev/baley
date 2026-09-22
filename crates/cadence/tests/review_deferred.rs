use cadence::review::{deferred, io, persistence};

use serde_json::{Value, json};
struct FixedClock;
impl io::Clock for FixedClock {
    fn now(&mut self) -> u64 {
        100
    }
}
fn fixture(name: &str) -> Value {
    let input: Value =
        serde_json::from_str(include_str!("fixtures/phase9/h5-all-homes.json")).unwrap();
    input[name].clone()
}
/// The review records one snapshot holds.
fn records(input: &Value) -> Value {
    persistence::records(&json!({"review":input["records"]})).unwrap()
}
fn answered(decision: deferred::Enqueue) -> Value {
    match decision {
        deferred::Enqueue::Answered(reply) => serde_json::to_value(reply).unwrap(),
        deferred::Enqueue::Write { member, .. } => panic!("wrote a new member: {member:?}"),
    }
}
#[test]
fn deferred_advisory_ac146() {
    let input = fixture("advisory");
    let decision = deferred::decide_enqueue(records(&input), "f1", &mut FixedClock).unwrap();
    assert_eq!(answered(decision), json!({"member":null}));
}
#[test]
fn deferred_off_ac147() {
    let input = fixture("off");
    let decision = deferred::decide_enqueue(records(&input), "f1", &mut FixedClock).unwrap();
    assert_eq!(answered(decision), json!({"member":null}));
}
/// A member already saved is answered from what was saved: no new member, so
/// its first timestamp stands.
#[test]
fn deferred_replay_preserves_initial_member() {
    let input = fixture("replay");
    assert_eq!(input["records"]["deferred"]["f1"]["enqueued_at"], 99);
    let decision = deferred::decide_enqueue(records(&input), "f1", &mut FixedClock).unwrap();
    assert_eq!(
        answered(decision),
        json!({"member":"f1","state":"unruled","continuation":"allowed"})
    );
}
#[test]
fn deferred_adjudication_rendering_does_not_filter() {
    let input = fixture("with-adjudication-rendering");
    let output = deferred::inventory(&records(&input)).unwrap();
    assert_eq!(
        output
            .members
            .iter()
            .map(|member| member.member.as_str())
            .collect::<Vec<_>>(),
        ["f1", "f2", "f3"]
    );
}
