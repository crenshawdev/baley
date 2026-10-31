//! The facts a plan is judged on, read from the views (design 0005 sections
//! 5 and 6, PLN-R9, PLN-R13, D-02, D-13, D-16): the phase's committed
//! stories in commitment order, each with its current truths; the evidence
//! items of the phase's plans in force; the phase's size; the capacity the
//! caller gives; and the active phases. `plan submit`'s check and the
//! approval's first step both gather them here, so an approval judges the
//! plan again on the same reading the submit judged it on.
//!
//! Nothing here judges or records anything. Every document a fact came from
//! is kept at the sequence that produced it, so a recording can confirm
//! none moved.

use std::num::NonZeroU32;

use baley_core::plan::{
    ApprovedItem, CommittedStory, CurrentTruth, EVIDENCE_MAP_VIEW, IN_FORCE, PHASE_PLAN_VIEW,
    PlanFacts, approved_items, evidence_map_key, phase_plan_key,
};
use baley_core::policy::{EffectivePolicy, Value as Setting};
use baley_store::{Absence, DocKey, ObservedDocument, ProjectId, StoreError, Views};
use serde_json::Value;

use crate::mcp::capture::{PLACE_LEDGER, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::planning::phase_plan::read_readiness;
use crate::refine::submit::current_truths;
use crate::scope::submit::NOT_RECORDED_BUSY;

/// The setting the capacity is read from.
const CAPACITY_SETTING: &str = "planning.phase_capacity";

/// A phase's facts with the documents they came from, read outside any
/// transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseFacts {
    /// What the plan judge takes.
    pub facts: PlanFacts,
    /// The active phases in number order, which may include this one.
    pub active: Vec<u32>,
    /// The phase's `phase_plan` document, or none before anything was
    /// projected for it.
    pub phase_plan: Option<Value>,
    /// The `roadmap` document, each committed story's `truths` document,
    /// the `phase_plan` document and each plan in force's `evidence_map`
    /// document, each at the sequence that produced it.
    documents: Vec<ObservedDocument>,
    /// What a recording relies on being absent.
    absences: Vec<Absence>,
}

impl PhaseFacts {
    /// The documents the facts were read from.
    pub fn documents(&self) -> &[ObservedDocument] {
        &self.documents
    }

    /// What a recording relies on being absent: with no phase active,
    /// `phase_plan`'s status index still holding none, since an empty read
    /// leaves no document to observe (D-02).
    pub fn absences(&self) -> &[Absence] {
        &self.absences
    }
}

/// The capacity `policy` sets, or none.
pub fn capacity(policy: &EffectivePolicy) -> Option<NonZeroU32> {
    match policy
        .settings
        .get(CAPACITY_SETTING)
        .and_then(|effective| effective.value.as_ref())
    {
        Some(Setting::Capacity(capacity)) => Some(*capacity),
        _ => None,
    }
}

/// Reads phase `phase` of `project`'s facts with `capacity`: the committed
/// stories and the active phases through `read_readiness`, each committed
/// story's current truths from the `truths` view, and the size and the
/// approved items of the plans in force, in plan-number order, from
/// `phase_plan` and each plan's `evidence_map` document. A phase the
/// roadmap does not hold, or a withdrawn one, commits no story. A fault is
/// the `failed` answer.
pub fn read<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    capacity: Option<NonZeroU32>,
) -> Result<PhaseFacts, Value> {
    let readiness = read_readiness(store, project, phase, &[])?;
    let mut documents = readiness.documents().to_vec();
    let mut stories = Vec::with_capacity(readiness.committed().len());
    for story in readiness.committed() {
        let current = current_truths(store, project, *story)?;
        documents.push(current.document);
        let truths = current
            .truths
            .into_iter()
            .map(|(truth, version)| CurrentTruth {
                id: truth.id,
                version,
                trigger: truth.trigger,
                observer: truth.observer,
                outcome: truth.outcome,
                human_verify: truth.human_verify,
            })
            .collect();
        stories.push(CommittedStory { id: *story, truths });
    }
    let key = phase_plan_key(phase);
    let document = store.get(project, PHASE_PLAN_VIEW, &key).map_err(busy)?;
    documents.push(observed(
        PHASE_PLAN_VIEW,
        key,
        document.as_ref().map(|document| document.produced_seq),
    ));
    let phase_plan = document.map(|document| document.body);
    let (size, approved) = match &phase_plan {
        None => (0, Vec::new()),
        Some(body) => {
            let size = body
                .get("size")
                .and_then(Value::as_u64)
                .and_then(|size| u32::try_from(size).ok())
                .ok_or_else(|| unreadable(PHASE_PLAN_VIEW))?;
            (size, in_force(store, project, phase, body, &mut documents)?)
        }
    };
    Ok(PhaseFacts {
        facts: PlanFacts {
            stories,
            approved,
            size,
            capacity,
        },
        active: readiness.readiness().active.to_vec(),
        phase_plan,
        absences: readiness.absences(),
        documents,
    })
}

/// The approved items of the plans in force that `phase_plan` lists, in
/// plan-number order, each read through the judge's reading of its plan's
/// `evidence_map` document, which is added to `documents`.
fn in_force<S: Views + ?Sized>(
    store: &S,
    project: &ProjectId,
    phase: u32,
    phase_plan: &Value,
    documents: &mut Vec<ObservedDocument>,
) -> Result<Vec<ApprovedItem>, Value> {
    let plans = phase_plan
        .get("plans")
        .and_then(Value::as_array)
        .ok_or_else(|| unreadable(PHASE_PLAN_VIEW))?;
    let mut items = Vec::new();
    // The view keeps a phase's plans in number order.
    for held in plans {
        if held.get("state").and_then(Value::as_str) != Some(IN_FORCE) {
            continue;
        }
        let plan = held
            .get("plan")
            .and_then(Value::as_u64)
            .and_then(|plan| u32::try_from(plan).ok())
            .ok_or_else(|| unreadable(PHASE_PLAN_VIEW))?;
        let key = evidence_map_key(phase, plan);
        let map = store
            .get(project, EVIDENCE_MAP_VIEW, &key)
            .map_err(busy)?
            .ok_or_else(|| unreadable(EVIDENCE_MAP_VIEW))?;
        documents.push(observed(EVIDENCE_MAP_VIEW, key, Some(map.produced_seq)));
        items.extend(approved_items(&map.body).ok_or_else(|| unreadable(EVIDENCE_MAP_VIEW))?);
    }
    Ok(items)
}

fn observed(view: &str, key: DocKey, produced_seq: Option<u64>) -> ObservedDocument {
    ObservedDocument {
        view: view.into(),
        key,
        produced_seq,
    }
}

fn busy(error: StoreError) -> Value {
    store_failed(&error, NOT_RECORDED_BUSY)
}

fn unreadable(view: &str) -> Value {
    failed(
        LEDGER_UNAVAILABLE,
        format!("the {view} view document cannot be read, so nothing was recorded"),
        PLACE_LEDGER,
    )
}
