//! A refusal has one shape, and one constructor builds it.
//!
//! The binary-rendered skills tell every reader that a refusal carries
//! `status: refused`, `code`, `reason`, and the location fields. The read
//! layer, the verification verdicts, the native-execution admission and the
//! server's own argument parsing each wrote their refusals by hand, and four
//! field sets grew out of it. A caller had to know which subsystem answered
//! before it knew which field to read.

#[path = "support/production_source.rs"]
mod production_source;
use serde_json::{Value, json};
use std::path::Path;

fn code_of(answer: &Value) -> &str {
    answer["code"].as_str().unwrap_or_else(|| panic!("no code in {answer}"))
}

#[test]
fn no_refusal_is_written_by_hand() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let sites = production_source::production_sites(&root, &|line| line.replace(' ', "").contains("\"status\":\"refused\""));
    assert!(
        sites.is_empty(),
        "refusals built without cadence::envelope::Refusal:\n{}",
        sites.join("\n")
    );
}

#[test]
fn no_refusal_names_a_decision_id() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let sites = production_source::production_sites(&root, &|line| line.contains(".rule(\"D-") || line.contains(".rule(\"H"));
    assert!(
        sites.is_empty(),
        "refusals naming a decision id or validator version instead of a rule:\n{}",
        sites.join("\n")
    );
}

#[test]
fn a_verdict_refusal_carries_a_code() {
    let answer = cadence::verification::verdicts::refusal(
        "verification-basis", "basis", "", "current verification authority unavailable", json!(null), json!(null),
    );
    assert_eq!(answer["status"], "refused", "{answer}");
    assert_eq!(answer["rule"], "verification-basis", "{answer}");
    assert!(!code_of(&answer).is_empty(), "{answer}");
}
