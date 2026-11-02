//! The delta of a checked plan draft and its revision, the input the `diff`
//! re-check scope reads (design 0005 PLN-R16, D-12).
//!
//! Entries are matched by identity, never by position, so a reorder is not
//! read as every entry changed, and each change keeps the value it replaced.
//! This is apart from the approval judge's stale path, which compares by
//! position and answers one path.

use serde_json::{Value, json};

use super::approval::PLAN;
use super::submission::{RecordedPlan, Replaces};

const REPLACES: &str = "replaces";

/// Every change from `original` to `revision`, each `{change, member, id,
/// before, after}`: `change` is `added`, `modified` or `deleted`; `id` is a
/// list entry's id, or its path for `files` and `directories`, and absent
/// for a member compared whole by name; `before` and `after` are the whole
/// entry or member, null on the side that lacks it. Members come in the plan
/// judge's order, then `replaces`; inside a list, deletions and
/// modifications in the original's order, then additions in the revision's
/// order. Equal drafts give an empty list.
pub fn plan_delta(original: &RecordedPlan, revision: &RecordedPlan) -> Vec<Value> {
    let mut changes = Vec::new();
    for member in PLAN {
        let (before, after) = (original.plan.get(member), revision.plan.get(member));
        match member {
            "tasks" | "evidence_map" | "questions" => {
                entries(&mut changes, member, before, after, |entry| &entry["id"]);
            }
            "files" | "directories" => entries(&mut changes, member, before, after, |entry| entry),
            _ => whole(&mut changes, member, before, after),
        }
    }
    let (before, after) = (replaces(original.replaces), replaces(revision.replaces));
    whole(&mut changes, REPLACES, before.as_ref(), after.as_ref());
    changes
}

/// The binding as a draft's body writes it.
fn replaces(binding: Option<Replaces>) -> Option<Value> {
    binding.map(|binding| json!({"plan": binding.plan, "digest": binding.digest.to_hex()}))
}

fn change(before: Option<&Value>, after: Option<&Value>) -> Option<&'static str> {
    match (before, after) {
        (None, Some(_)) => Some("added"),
        (Some(_), None) => Some("deleted"),
        (Some(before), Some(after)) if before != after => Some("modified"),
        _ => None,
    }
}

/// A member compared whole by name.
fn whole(changes: &mut Vec<Value>, member: &str, before: Option<&Value>, after: Option<&Value>) {
    if let Some(kind) = change(before, after) {
        changes.push(json!({"change": kind, "member": member, "before": before, "after": after}));
    }
}

/// A list member compared entry by entry, each matched by `key`. A member
/// that is not a list on both sides is compared whole.
fn entries(
    changes: &mut Vec<Value>,
    member: &str,
    before: Option<&Value>,
    after: Option<&Value>,
    key: impl Fn(&Value) -> &Value,
) {
    let (Some(Value::Array(old)), Some(Value::Array(new))) = (before, after) else {
        return whole(changes, member, before, after);
    };
    let find = |list: &'_ [Value], id: &Value| list.iter().find(|entry| key(entry) == id).cloned();
    let mut push = |id: &Value, before: Option<&Value>, after: Option<&Value>| {
        if let Some(kind) = change(before, after) {
            changes.push(
                json!({"change": kind, "member": member, "id": id, "before": before,
                "after": after}),
            );
        }
    };
    for entry in old {
        let id = key(entry);
        push(id, Some(entry), find(new, id).as_ref());
    }
    for entry in new {
        let id = key(entry);
        if find(old, id).is_none() {
            push(id, None, Some(entry));
        }
    }
}

#[cfg(test)]
mod tests {
    //! Expected deltas are written out from design 0005 PLN-R16, D-12 and the
    //! plan's test data, never from running this code.

    use baley_store::Hash;

    use super::*;
    use crate::plan::fixtures::{c2, k1, k3, p1, qa, replacing, with};

    fn recorded(plan: Value) -> RecordedPlan {
        RecordedPlan {
            plan,
            replaces: None,
        }
    }

    /// K2: K1 with id `k2`.
    fn k2() -> Value {
        with(k1(), "", "id", json!("k2"))
    }

    #[test]
    fn a_delta_matching_tasks_by_position_or_dropping_before_values_is_caught() {
        let retitled = with(k2(), "", "title", json!("Read the plan"));
        let original = recorded(with(p1(), "", "tasks", json!([k1(), k2()])));
        let revision = recorded(with(p1(), "", "tasks", json!([retitled.clone(), k3()])));
        assert_eq!(
            plan_delta(&original, &revision),
            [
                json!({"change": "deleted", "member": "tasks", "id": "k1", "before": k1(),
                    "after": null}),
                json!({"change": "modified", "member": "tasks", "id": "k2", "before": k2(),
                    "after": retitled}),
                json!({"change": "added", "member": "tasks", "id": "k3", "before": null,
                    "after": k3()}),
            ]
        );
    }

    #[test]
    fn a_delta_of_lease_items_questions_or_named_members_missing_a_change_is_caught() {
        let original = recorded(with(p1(), "", "questions", json!([qa()])));
        let c2_reasoned = with(c2(), "", "reason", json!("the preview reorders the tasks"));
        let mut plan = with(p1(), "", "files", json!(["src/judge.rs", "src/delta.rs"]));
        plan = with(plan, "", "directories", json!([]));
        plan = with(plan, "", "goal", json!("Judge a plan and its revision."));
        plan = replacing(plan, "c2", c2_reasoned.clone());
        let revision = RecordedPlan {
            plan,
            replaces: Some(Replaces {
                plan: 1,
                digest: Hash([0x5c; 32]),
            }),
        };
        assert_eq!(
            plan_delta(&original, &revision),
            [
                json!({"change": "added", "member": "files", "id": "src/delta.rs",
                    "before": null, "after": "src/delta.rs"}),
                json!({"change": "deleted", "member": "directories", "id": "docs",
                    "before": "docs", "after": null}),
                json!({"change": "modified", "member": "goal",
                    "before": "Judge a plan whole.", "after": "Judge a plan and its revision."}),
                json!({"change": "modified", "member": "evidence_map", "id": "c2",
                    "before": c2(), "after": c2_reasoned}),
                json!({"change": "deleted", "member": "questions", "id": "q1",
                    "before": qa(), "after": null}),
                json!({"change": "added", "member": "replaces", "before": null,
                    "after": {"plan": 1, "digest": "5c".repeat(32)}}),
            ]
        );
    }

    #[test]
    fn a_delta_of_equal_drafts_reporting_a_change_is_caught() {
        let plan = p1();
        let members = plan.as_object().expect("an object");
        let reversed: serde_json::Map<String, Value> = members
            .keys()
            .rev()
            .map(|name| (name.clone(), members[name].clone()))
            .collect();
        assert_eq!(
            plan_delta(&recorded(p1()), &recorded(Value::Object(reversed))),
            Vec::<Value>::new()
        );
    }
}
