//! The plan checker's pure judgements (design 0005 PLN-R16 and section 6):
//! whether a relayed findings list has the shape a check result records.
//!
//! Nothing here reads a store: the binary supplies every value it judges.

use serde_json::{Value, json};

/// The refusal code for a findings list holding an entry that is not a
/// finding: not an object holding exactly a `severity` of `blocker` or
/// `warning` and a `location`, `claim` and `fix` that are not blank. It
/// names the first such entry's position (design 0008 REV-R6, D-17).
pub const FINDING_SHAPE: &str = "finding-shape";

const SEVERITY: &str = "severity";
const BLOCKER: &str = "blocker";
const WARNING: &str = "warning";
const TEXTS: [&str; 3] = ["location", "claim", "fix"];

/// Why a checker's return or a re-check is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckRefusal {
    /// The entry at zero-based `position` is not a finding.
    FindingShape { position: usize },
}

impl CheckRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::FindingShape { .. } => FINDING_SHAPE,
        }
    }

    /// What the refusal names, as its answer's details.
    pub fn details(&self) -> Value {
        match self {
            Self::FindingShape { position } => json!({"position": position}),
        }
    }
}

/// An accepted findings list, each finding exactly as the checker returned
/// it, so the record holds the checker's own words (D-06).
#[derive(Debug, Clone, PartialEq)]
pub struct Findings(Vec<Value>);

impl Findings {
    /// The findings in the order returned.
    pub fn findings(&self) -> &[Value] {
        &self.0
    }

    /// Whether any finding is a blocker, which buys the one revision.
    pub fn has_blocker(&self) -> bool {
        self.0.iter().any(is_blocker)
    }

    /// The blockers in the order returned.
    pub fn blockers(&self) -> Vec<&Value> {
        self.0
            .iter()
            .filter(|finding| is_blocker(finding))
            .collect()
    }
}

fn is_blocker(finding: &Value) -> bool {
    finding[SEVERITY] == BLOCKER
}

/// Judges a relayed findings list: `finding-shape` naming the first entry
/// that is not an object holding exactly a `severity` of `blocker` or
/// `warning`, case not folded, and a `location`, `claim` and `fix` that are
/// text and not blank. An empty list is accepted, since no finding is a
/// pass (PLN-R16).
pub fn judge_findings(findings: Vec<Value>) -> Result<Findings, CheckRefusal> {
    if let Some(position) = findings.iter().position(|finding| !is_finding(finding)) {
        return Err(CheckRefusal::FindingShape { position });
    }
    Ok(Findings(findings))
}

fn is_finding(value: &Value) -> bool {
    let Some(members) = value.as_object() else {
        return false;
    };
    let severity = members.get(SEVERITY).and_then(Value::as_str);
    let text = |name: &str| {
        members
            .get(name)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
    };
    members.len() == 1 + TEXTS.len()
        && matches!(severity, Some(BLOCKER | WARNING))
        && TEXTS.iter().all(|name| text(name))
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005 PLN-R16, design
    //! 0008 REV-R6, D-16, D-17 and the plan's test data, never from running
    //! this code.

    use super::*;
    use crate::plan::fixtures::{with, without};

    fn fb() -> Value {
        json!({"severity": "blocker", "location": "tasks[0]",
            "claim": "No task makes truth t2 of story 1 true",
            "fix": "Add a task that records the round"})
    }

    fn fw() -> Value {
        json!({"severity": "warning", "location": "tasks[0].verify[0]",
            "claim": "The verify runs every ledger test", "fix": "Name the round test"})
    }

    /// FN: FB without its severity.
    fn fn_() -> Value {
        without(fb(), "", "severity")
    }

    fn refused(findings: Vec<Value>) -> (&'static str, Value) {
        let refusal = judge_findings(findings).expect_err("refused");
        (refusal.code(), refusal.details())
    }

    #[test]
    fn a_finding_without_a_severity_or_with_another_accepted_is_caught() {
        assert_eq!(
            refused(vec![fw(), fn_()]),
            ("finding-shape", json!({"position": 1}))
        );
        for findings in [
            vec![with(fb(), "", "severity", json!("BLOCKER"))],
            vec![with(fw(), "", "note", json!("Read this first"))],
            vec![with(fw(), "", "claim", json!("  "))],
            vec![json!("a finding")],
        ] {
            assert_eq!(refused(findings), ("finding-shape", json!({"position": 0})));
        }

        let accepted = judge_findings(vec![fb(), fw()]).expect("accepted");
        assert_eq!(accepted.findings(), [fb(), fw()]);
        assert!(accepted.has_blocker());
        assert_eq!(accepted.blockers(), [&fb()]);

        let empty = judge_findings(Vec::new()).expect("accepted");
        assert!(empty.findings().is_empty());
        assert!(!empty.has_blocker());
    }
}
