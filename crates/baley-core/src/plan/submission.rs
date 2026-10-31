//! A plan draft's body, digest and target, and a recorded body read back
//! (design 0005 sections 5 and 6, PLN-R14, PLN-R15, D-03, D-05).
//!
//! A plan draft is `scope.submitted` of kind `plan`. Its body is the plan
//! exactly as submitted with `kind` added, and for a replacement the plan
//! number and digest it replaces; nothing a call carries enters it, so the
//! digest names the plan alone. Every draft of one phase is filed under
//! `phase/<n>`, so the newest of any kind makes the others stale.

use baley_store::{Hash, canonical_json, request_digest};
use serde_json::{Map, Value, json};

use super::content::counter;
use super::event::phase_stream;
use super::judge::Plan;

/// The kind a plan draft's body and `scope.submitted` spell.
pub const PLAN_KIND: &str = "plan";

const KIND_MEMBER: &str = "kind";
const REPLACES_MEMBER: &str = "replaces";

/// The approved plan a replacement draft replaces (PLN-R14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replaces {
    /// The replaced plan's number.
    pub plan: u32,
    /// The digest the replaced plan was approved at.
    pub digest: Hash,
}

/// A plan draft built from an accepted plan. Only an accepted [`Plan`]
/// gives one, so nothing unjudged is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDraft {
    body: Vec<u8>,
    digest: Hash,
    target: String,
}

impl PlanDraft {
    /// The draft of `plan` submitted as a new plan.
    pub fn new(plan: &Plan) -> Self {
        Self::build(plan, None)
    }

    /// The draft of `plan` submitted to replace `replaces`. Its body names
    /// the replaced plan, so its digest is never the plain draft's.
    pub fn replacing(plan: &Plan, replaces: Replaces) -> Self {
        Self::build(plan, Some(replaces))
    }

    fn build(plan: &Plan, replaces: Option<Replaces>) -> Self {
        let mut body = plan.submitted.clone();
        body.insert(KIND_MEMBER.into(), Value::String(PLAN_KIND.into()));
        if let Some(Replaces { plan, digest }) = replaces {
            body.insert(
                REPLACES_MEMBER.into(),
                json!({"plan": plan, "digest": digest.to_hex()}),
            );
        }
        let value = Value::Object(body);
        // Every leaf of an accepted plan is text or a counter within u32,
        // so it always has a canonical form.
        let digest = request_digest(&value).expect("an accepted plan is canonical");
        let body = canonical_json(&value).expect("an accepted plan is canonical");
        Self {
            body,
            digest,
            target: phase_stream(plan.phase()),
        }
    }

    /// The canonical JSON bytes the digest is taken over, stored as the
    /// draft's body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The SHA-256 of the body and nothing else.
    pub fn digest(&self) -> Hash {
        self.digest
    }

    /// What the draft is filed under among plan drafts: `phase/<n>`, for a
    /// new plan and a replacement alike, so a phase has one draft line.
    pub fn target(&self) -> &str {
        &self.target
    }
}

/// A recorded plan draft body read back.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedPlan {
    /// The plan's members as submitted, without `kind` or `replaces`, for
    /// the plan judge to read again.
    pub plan: Value,
    /// The plan it replaces, for a replacement draft.
    pub replaces: Option<Replaces>,
}

/// Reads a recorded plan draft body, or `None` when it is not an object,
/// its `kind` is not `plan`, or its `replaces` is not an object holding a
/// whole `plan` from 1 and a 64-character lowercase hex `digest`. The plan
/// itself is not judged here: callers judge it again against the phase as
/// it stands.
pub fn read_plan_draft(body: &Value) -> Option<RecordedPlan> {
    let Value::Object(members) = body else {
        return None;
    };
    let mut plan: Map<String, Value> = members.clone();
    if plan.remove(KIND_MEMBER)? != PLAN_KIND {
        return None;
    }
    let replaces = match plan.remove(REPLACES_MEMBER) {
        None => None,
        Some(binding) => Some(Replaces {
            plan: counter(binding.get("plan"))?,
            digest: lowercase_digest(binding.get("digest"))?,
        }),
    };
    Some(RecordedPlan {
        plan: Value::Object(plan),
        replaces,
    })
}

/// A digest written as Baley writes one: 64 lowercase hex digits.
fn lowercase_digest(value: Option<&Value>) -> Option<Hash> {
    let text = value?.as_str()?;
    if !text
        .bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    Hash::from_hex(text)
}

#[cfg(test)]
mod tests {
    //! Expected answers are written out from design 0005, the phase's
    //! decisions and the plan's test data, never from running this code.

    use serde_json::json;

    use super::*;
    use crate::plan::fixtures::{accepted, c1, c2, f1, k1, o1, p1};

    /// X: `5c` repeated 32 times.
    fn x() -> Hash {
        Hash([0x5c; 32])
    }

    fn replaces(plan: u32) -> Replaces {
        Replaces { plan, digest: x() }
    }

    fn reversed(value: &Value) -> Value {
        let members = value.as_object().expect("an object");
        let mut names: Vec<&String> = members.keys().collect();
        names.reverse();
        Value::Object(
            names
                .into_iter()
                .map(|name| (name.clone(), members[name].clone()))
                .collect(),
        )
    }

    #[test]
    fn a_plan_digest_over_more_than_the_plan_or_not_its_body_is_caught() {
        let by_hand = json!({"phase": 8, "stories": [1], "files": ["src/judge.rs"],
            "directories": ["docs"], "goal": "Judge a plan whole.", "context": "Pure core.",
            "notes": "", "tasks": [k1()], "evidence_map": [c1(), c2(), o1()], "questions": [],
            "kind": "plan"});
        let draft = PlanDraft::new(&accepted(&p1(), &f1()));
        assert_eq!(draft.body(), canonical_json(&by_hand).unwrap());
        assert_eq!(draft.digest(), request_digest(&by_hand).unwrap());
        assert_eq!(draft.target(), "phase/8");

        let reordered = PlanDraft::new(&accepted(&reversed(&p1()), &f1()));
        assert_eq!(reordered.body(), draft.body());
        assert_eq!(reordered.digest(), draft.digest());
    }

    #[test]
    fn a_replacement_sharing_a_digest_or_another_line_with_a_new_plan_is_caught() {
        let plan = accepted(&p1(), &f1());
        let plain = PlanDraft::new(&plan);
        let of_one = PlanDraft::replacing(&plan, replaces(1));
        let of_two = PlanDraft::replacing(&plan, replaces(2));
        assert_ne!(plain.digest(), of_one.digest());
        assert_ne!(plain.digest(), of_two.digest());
        assert_ne!(of_one.digest(), of_two.digest());
        for draft in [&plain, &of_one, &of_two] {
            assert_eq!(draft.target(), "phase/8");
        }
        let binding = |draft: &PlanDraft| {
            serde_json::from_slice::<Value>(draft.body()).unwrap()["replaces"].clone()
        };
        let x = "5c".repeat(32);
        assert_eq!(binding(&of_one), json!({"plan": 1, "digest": x}));
        assert_eq!(binding(&of_two), json!({"plan": 2, "digest": x}));
    }

    #[test]
    fn a_recorded_plan_body_read_back_wrongly_is_caught() {
        let draft = PlanDraft::replacing(&accepted(&p1(), &f1()), replaces(1));
        let body: Value = serde_json::from_slice(draft.body()).unwrap();
        assert_eq!(
            read_plan_draft(&body),
            Some(RecordedPlan {
                plan: p1(),
                replaces: Some(Replaces {
                    plan: 1,
                    digest: x()
                }),
            })
        );

        let altered = |member: &str, to: Value| {
            let mut altered = body.clone();
            altered[member] = to;
            altered
        };
        let x = "5c".repeat(32);
        for refused in [
            altered("kind", json!("phase-edit")),
            altered("replaces", json!({"plan": 1, "digest": "XYZ"})),
            altered("replaces", json!({"plan": 0, "digest": x})),
        ] {
            assert_eq!(read_plan_draft(&refused), None, "{refused}");
        }
    }
}
