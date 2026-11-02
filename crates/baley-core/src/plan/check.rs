//! The plan checker's pure judgements (design 0005 PLN-R16 and section 6):
//! whether a relayed findings list has the shape a check result records,
//! which round of which check lineage a submit is, and whether the truths a
//! re-check would read are round 1's.
//!
//! Nothing here reads a store: the binary supplies every value it judges.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::event::TruthVersion;

/// The refusal code for a findings list holding an entry that is not a
/// finding: not an object holding exactly a `severity` of `blocker` or
/// `warning` and a `location`, `claim` and `fix` that are not blank. It
/// names the first such entry's position (design 0008 REV-R6, D-17).
pub const FINDING_SHAPE: &str = "finding-shape";
/// The refusal code for a submit under a revision order whose round 2 was
/// already issued, since a lineage buys one revision and one re-check. It
/// names the revision order (design 0008 REV-R10, D-02, D-17).
pub const THIRD_ROUND: &str = "third-round";
/// The refusal code for a re-check whose truths are not round 1's: a truth
/// added, removed or re-versioned since needs a new submission and a full
/// first check. It names each difference (design 0005 section 5, ADR 0037,
/// D-11).
pub const RECHECK_TRUTHS_CHANGED: &str = "recheck-truths-changed";
/// The refusal code for a plan submit or replace while the phase's checker
/// order is pending, since a phase has one live checker order. It names
/// that order, which the binary finds (D-03).
pub const CHECKER_ORDER_PENDING: &str = "checker-order-pending";

const SEVERITY: &str = "severity";
const BLOCKER: &str = "blocker";
const WARNING: &str = "warning";
const TEXTS: [&str; 3] = ["location", "claim", "fix"];

/// Why a checker's return or a re-check is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckRefusal {
    /// The entry at zero-based `position` is not a finding.
    FindingShape { position: usize },
    /// The lineage of revision order `work_order` already had its round 2.
    ThirdRound { work_order: String },
    /// The truths differ from round 1's in these, in round 1's order, then
    /// the added ones in the current order.
    RecheckTruthsChanged { truths: Vec<TruthChange> },
}

impl CheckRefusal {
    /// The refusal's public code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::FindingShape { .. } => FINDING_SHAPE,
            Self::ThirdRound { .. } => THIRD_ROUND,
            Self::RecheckTruthsChanged { .. } => RECHECK_TRUTHS_CHANGED,
        }
    }

    /// What the refusal names, as its answer's details.
    pub fn details(&self) -> Value {
        match self {
            Self::FindingShape { position } => json!({"position": position}),
            Self::ThirdRound { work_order } => json!({"work_order": work_order}),
            Self::RecheckTruthsChanged { truths } => {
                let truths: Vec<Value> = truths
                    .iter()
                    .map(|t| {
                        json!({"story": t.story, "truth": t.truth, "bound": t.bound,
                            "current": t.current})
                    })
                    .collect();
                json!({"truths": truths})
            }
        }
    }
}

/// One truth a re-check would read differently from round 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruthChange {
    /// The story holding the truth.
    pub story: u32,
    /// The truth's id within its story.
    pub truth: String,
    /// Its version at round 1, or `None` when round 1 did not read it.
    pub bound: Option<u32>,
    /// Its version now, or `None` when it is gone.
    pub current: Option<u32>,
}

/// A check lineage of the phase that bought its one revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lineage<'a> {
    /// The planner revision order its round-1 blocker issued.
    pub revision: &'a str,
    /// Whether its round 2 was issued.
    pub rechecked: bool,
}

/// Which check a submit gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckRound<'a> {
    /// Round 1 of a fresh lineage, reading the whole plan.
    Fresh,
    /// Round 2 of the lineage whose revision order is `revision`.
    Recheck { revision: &'a str },
}

/// Judges which check a submit gets, given every lineage of the phase that
/// bought a revision and `chain`, the first order of the chain the submit's
/// order belongs to (D-02). A chain headed by a lineage's revision order is
/// that lineage's round 2, or `third-round` once its round 2 was issued; any
/// other chain starts a fresh lineage. The chain, not the order, is
/// compared, so a retry of the revision order buys no second revision, and
/// every lineage is given, so an old revision order buys no fresh budget.
pub fn judge_check_round<'a>(
    lineages: &[Lineage<'a>],
    chain: &str,
) -> Result<CheckRound<'a>, CheckRefusal> {
    match lineages.iter().find(|lineage| lineage.revision == chain) {
        None => Ok(CheckRound::Fresh),
        Some(lineage) if lineage.rechecked => Err(CheckRefusal::ThirdRound {
            work_order: lineage.revision.to_owned(),
        }),
        Some(lineage) => Ok(CheckRound::Recheck {
            revision: lineage.revision,
        }),
    }
}

/// Judges the truths a re-check would read against round 1's, as sets keyed
/// by story and truth id (D-11): `recheck-truths-changed` naming every truth
/// added, removed or at another version, and nothing for the same truths in
/// any order. An unchanged truth keeps its version, so a truth approval that
/// left the text alone passes.
pub fn judge_recheck_truths(
    bound: &[TruthVersion],
    current: &[TruthVersion],
) -> Result<(), CheckRefusal> {
    let key = |t: &TruthVersion| (t.story, t.truth.clone());
    let now: HashMap<(u32, String), u32> = current.iter().map(|t| (key(t), t.version)).collect();
    let then: HashSet<(u32, String)> = bound.iter().map(key).collect();
    let mut truths: Vec<TruthChange> = bound
        .iter()
        .filter_map(|t| {
            let version = now.get(&key(t)).copied();
            (version != Some(t.version)).then(|| TruthChange {
                story: t.story,
                truth: t.truth.clone(),
                bound: Some(t.version),
                current: version,
            })
        })
        .collect();
    truths.extend(
        current
            .iter()
            .filter(|t| !then.contains(&key(t)))
            .map(|t| TruthChange {
                story: t.story,
                truth: t.truth.clone(),
                bound: None,
                current: Some(t.version),
            }),
    );
    if truths.is_empty() {
        Ok(())
    } else {
        Err(CheckRefusal::RecheckTruthsChanged { truths })
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

    const V: &str = "2d3e4f50-6172-4c83-9d94-aea5f6071823";
    const V2: &str = "3e4f5061-7283-4d94-aea5-b6f708192a34";
    const O3: &str = "4f506172-8394-4ea5-bfb6-c708192a3b45";

    fn lineage(revision: &str, rechecked: bool) -> Lineage<'_> {
        Lineage {
            revision,
            rechecked,
        }
    }

    fn round<'a>(
        lineages: &[Lineage<'a>],
        chain: &str,
    ) -> Result<CheckRound<'a>, (&'static str, Value)> {
        judge_check_round(lineages, chain).map_err(|refusal| (refusal.code(), refusal.details()))
    }

    #[test]
    fn a_third_round_bought_or_a_fresh_lineage_taken_for_round_two_is_caught() {
        let third = |order: &str| Err(("third-round", json!({"work_order": order})));
        assert_eq!(
            round(&[lineage(V, false)], V),
            Ok(CheckRound::Recheck { revision: V })
        );
        assert_eq!(round(&[lineage(V, true)], V), third(V));
        assert_eq!(round(&[lineage(V, false)], O3), Ok(CheckRound::Fresh));
        assert_eq!(round(&[], V), Ok(CheckRound::Fresh));
        let both = [lineage(V, true), lineage(V2, false)];
        assert_eq!(round(&both, V), third(V));
        assert_eq!(round(&both, V2), Ok(CheckRound::Recheck { revision: V2 }));
    }

    fn tv(story: u32, truth: &str, version: u32) -> TruthVersion {
        TruthVersion {
            story,
            truth: truth.into(),
            version,
        }
    }

    #[test]
    fn a_changed_truth_let_through_or_a_reordered_one_refused_is_caught() {
        let bound = [tv(1, "t1", 1), tv(1, "t2", 1)];
        let changed = |current: &[TruthVersion]| {
            let refusal = judge_recheck_truths(&bound, current).expect_err("refused");
            (refusal.code(), refusal.details())
        };
        let refused = |truths: Value| ("recheck-truths-changed", json!({"truths": truths}));

        assert_eq!(
            judge_recheck_truths(&bound, &[tv(1, "t2", 1), tv(1, "t1", 1)]),
            Ok(())
        );
        assert_eq!(
            changed(&[tv(1, "t1", 1), tv(1, "t2", 2)]),
            refused(json!([{"story": 1, "truth": "t2", "bound": 1, "current": 2}]))
        );
        assert_eq!(
            changed(&[tv(1, "t1", 1)]),
            refused(json!([{"story": 1, "truth": "t2", "bound": 1, "current": null}]))
        );
        assert_eq!(
            changed(&[tv(1, "t1", 1), tv(1, "t2", 1), tv(4, "t2", 1)]),
            refused(json!([{"story": 4, "truth": "t2", "bound": null, "current": 1}]))
        );
    }
}
