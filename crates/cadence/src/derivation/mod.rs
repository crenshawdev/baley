//! Captured artifact evidence and synchronous lifecycle derivation.
mod capture;
mod consistency;
mod memo;
mod model;
mod parse;
mod query;
pub use capture::{ArtifactFiles, ArtifactIo, capture_inputs};
pub use consistency::{check_consistency, roadmap_conflicts};
pub use memo::{DOMAIN, ENCODING_VERSION, SEMANTICS_VERSION, encode_inputs, input_key, input_key_with};
pub use model::*;
pub use parse::{parse_roadmap, parse_uat};
pub use query::{PreparedLifecycle, RecheckedLifecycle, prepare_query, prepare_progress, query, recheck_query};

fn validate_observation_failures(capture: &CapturedInputs) -> Result<(), DerivationError> {
    fn check<T>(observation: &Observation<T>) -> Result<(), DerivationError> {
        if let Observation::Failed(error) = observation {
            return Err(DerivationError::InputFailure(error.clone()));
        }
        Ok(())
    }
    check(&capture.root_probe)?;
    check(&capture.roadmap)?;
    for phase in &capture.phases {
        check(&phase.plans)?;
        check(&phase.summary)?;
        check(&phase.uat)?;
    }
    Ok(())
}

/// Availability is checked before interpreting any evidence as lifecycle state.
pub(crate) fn validate_inputs(capture: &CapturedInputs) -> Result<&ParsedRoadmap, DerivationError> {
    validate_observation_failures(capture)?;
    if matches!(capture.root_probe, Observation::Absent) {
        return Err(DerivationError::MissingPlanningRoot {
            path: capture.root.clone(),
        });
    }
    if matches!(capture.roadmap, Observation::Absent) {
        return Err(DerivationError::MissingRoadmap {
            path: capture.root.join("ROADMAP.md"),
        });
    }
    capture
        .declarations
        .as_ref()
        .ok_or_else(|| DerivationError::InvalidRoadmap {
            detail: "captured roadmap has no parsed declarations".into(),
        })?
        .as_ref()
        .map_err(Clone::clone)
}

fn store_failure(error: crate::store::Error) -> DerivationError {
    DerivationError::Store { kind: "invalid".into(), detail: error.to_string() }
}

/// The native acceptance authority in one snapshot (D-131). A phase is native
/// when its truths are approved; it is executed when every current
/// publication is admitted, no dispatch is active and every admitted plan
/// and task has its completion; it is complete only through an applicable
/// completion record. Nothing here reads SUMMARY.md or UAT.md.
pub fn acceptance_overlay(data: &serde_json::Value) -> Result<AcceptanceOverlay, DerivationError> {
    use crate::{execution::{admission, history}, plan::persistence, verification::completion};
    let mut overlay = AcceptanceOverlay::default();
    let mut keys: std::collections::BTreeSet<String> = data.get("context").and_then(|c| c.get("phases"))
        .and_then(|p| p.as_object()).map(|p| p.keys().cloned().collect()).unwrap_or_default();
    keys.extend(crate::undo::model::records(data).map_err(store_failure)?.values()
        .filter(|r| r.state == "committed").map(|r| r.manifest.phase.to_string()));
    for key in &keys {
        let Ok(phase) = key.parse::<u32>() else { continue };
        if phase == 0 || phase.to_string() != *key { continue }
        let native = (|| -> crate::store::Result<AcceptancePhase> {
            if crate::undo::model::undone(data, phase)? {
                return Ok(AcceptancePhase { published: true, executed: false, completion: None,
                    label: None, met: 0, waived: 0, disagreement: None });
            }
            let occurrence = persistence::saved(data, phase)?;
            let published = occurrence.as_ref().is_some_and(|o| !o.publications.is_empty());
            let admissions = admission::records(data, phase)?;
            let mut executed = published && !admissions.is_empty()
                && !data["execution"]["occurrences"][key]["active"].is_object();
            if executed {
                let latest = admissions.last().expect("nonempty");
                let current: Vec<_> = occurrence.as_ref().map(|o| o.publications.iter()
                    .map(|(n, p)| (*n, p.revision.clone(), p.map_revision.clone())).collect()).unwrap_or_default();
                let admitted: Vec<_> = latest.request.contract.plans.iter()
                    .map(|b| (b.plan, b.content_revision.clone(), Some(b.map_revision.clone()))).collect();
                executed = current == admitted;
            }
            if executed {
                let events = history::records(data, phase)?;
                let plan_events = history::plan_records(data, phase)?;
                let admitted = history::admitted_plans(data, phase)?;
                let outcomes = history::plan_outcomes(data, phase)?;
                executed = !admitted.is_empty() && history::phase_complete(data, phase)?;
                for (identity, _) in &admitted {
                    if !executed { break }
                    if !outcomes.iter().any(|outcome| outcome.plan == identity.plan
                        && outcome.disposition == crate::execution::model::PlanDisposition::Complete) {
                        continue;
                    }
                    executed = history::plan_project(&plan_events, identity).completed
                        && history::plan_task_views(data, &events, phase, identity.plan)?.iter()
                            .all(|t| t.state.completed && t.state.unknown_runs.is_empty());
                }
            }
            let (completion, label, met, waived, disagreement) = match completion::applicable(data, phase)? {
                Some((record, true, _)) => {
                    let count = |status: &str| record.truths.iter().filter(|t| t["status"] == status).count();
                    (Some(record.id.clone()), Some(record.label.clone()), count("met"), count("waived"), None)
                }
                Some((_, false, reason)) => (None, None, 0, 0, Some(reason)),
                None => (None, None, 0, 0, None),
            };
            Ok(AcceptancePhase { published, executed: executed || completion.is_some(), completion, label, met, waived, disagreement })
        })().map_err(store_failure)?;
        overlay.phases.insert(key.clone(), native);
    }
    Ok(overlay)
}

/// Observe the overlay from the store files under the planning root without
/// opening the store; absent files are an empty overlay.
pub fn observe_acceptance(root: &std::path::Path) -> Result<AcceptanceOverlay, DerivationError> {
    match crate::context::persistence::read_snapshot(root).map_err(store_failure)? {
        Some(snapshot) => acceptance_overlay(&snapshot.data),
        None => Ok(AcceptanceOverlay::default()),
    }
}

/// Pure lifecycle truth table over the exact bytes and probes in one capture,
/// with no native acceptance authority: the legacy SUMMARY/UAT table.
pub fn derive(capture: &CapturedInputs) -> Result<Lifecycle, DerivationError> {
    derive_with(capture, &AcceptanceOverlay::default())
}

/// The truth table with native acceptance overlaid: a phase with approved
/// truths takes Planned, Executed and Complete from the overlay, and its
/// counts are its met and waived truths; every other phase is legacy.
pub fn derive_with(capture: &CapturedInputs, overlay: &AcceptanceOverlay) -> Result<Lifecycle, DerivationError> {
    let parsed = validate_inputs(capture)?;
    let mut phases = Vec::with_capacity(parsed.phases.len());
    for declaration in &parsed.phases {
        let observation = capture
            .phases
            .iter()
            .find(|p| p.relative_path == declaration.relative_path)
            .ok_or_else(|| {
                DerivationError::InputFailure(InputFailure {
                    path: capture.root.join(&declaration.relative_path),
                    category: InputFailureCategory::InvalidPath,
                    diagnostic: Some("missing addressed phase observation".into()),
                })
            })?;
        let plans = match &observation.plans {
            Observation::Present(names) => names.clone(),
            _ => Vec::new(),
        };
        let uat = match &observation.uat {
            Observation::Present(bytes) if !bytes.is_empty() => {
                Some(parse_uat(&String::from_utf8_lossy(bytes)))
            }
            _ => None,
        };
        let mut status = if plans.is_empty() {
            LifecycleStatus::Unplanned
        } else {
            LifecycleStatus::Planned
        };
        if let Some(native) = overlay.phases.get(&declaration.id.address()) {
            let (status, uat) = if native.completion.is_some() {
                (LifecycleStatus::Complete, Some(UatCounts { pass: native.met, skipped: native.waived, ..UatCounts::default() }))
            } else if native.executed {
                (LifecycleStatus::Executed, None)
            } else if native.published || !plans.is_empty() {
                (LifecycleStatus::Planned, None)
            } else {
                (LifecycleStatus::Unplanned, None)
            };
            phases.push(PhaseRecord { id: declaration.id, name: declaration.name.clone(), plans, status, uat, accepted: true });
            continue;
        }
        if matches!(observation.summary, Observation::Present(())) {
            let complete = uat.as_ref().is_some_and(|uat| {
                !uat.items.is_empty()
                    && uat.items.iter().all(|item| {
                        item.status.as_deref() == Some("pass")
                            || (item.status.as_deref() == Some("skipped")
                                && item
                                    .reason
                                    .as_ref()
                                    .is_some_and(|reason| !reason.is_empty()))
                    })
            });
            status = if complete {
                LifecycleStatus::Complete
            } else {
                LifecycleStatus::Executed
            };
        }
        phases.push(PhaseRecord {
            id: declaration.id,
            name: declaration.name.clone(),
            plans,
            status,
            uat: uat.map(|u| u.counts),
            accepted: false,
        });
    }
    Ok(Lifecycle {
        cycle: parsed.cycle,
        current: phases
            .iter()
            .find(|p| p.status != LifecycleStatus::Complete)
            .map(|p| p.id),
        total: phases.len(),
        phases,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod overlay_tests {
    use super::*;

    fn native_capture() -> CapturedInputs {
        let text = "## Phases\n- [x] **Phase 13: Native**\n- [ ] **Phase 14: Legacy**\n";
        let parsed = parse_roadmap(text).unwrap();
        let phases = parsed.phases.iter().map(|p| PhaseObservation {
            relative_path: p.relative_path.clone(),
            plans: Observation::Present(vec!["PLAN-1.md".into()]),
            summary: Observation::Present(()),
            uat: Observation::Present(b"### 1. Check\nstatus: fail\n".to_vec()),
        }).collect();
        CapturedInputs { root: "/planning".into(), root_probe: Observation::Present(()),
            roadmap: Observation::Present(text.as_bytes().into()), declarations: Some(Ok(parsed)), phases }
    }

    fn native(completion: Option<&str>, executed: bool) -> AcceptancePhase {
        AcceptancePhase { published: true, executed, completion: completion.map(str::to_owned),
            label: completion.map(|_| "complete-with-waivers".to_owned()), met: 1, waived: 1, disagreement: None }
    }

    #[test]
    fn a_native_phase_takes_its_status_from_the_overlay_not_summary_or_uat() {
        let capture = native_capture();
        // Legacy reading: SUMMARY present, UAT failing, so Executed for both.
        let legacy = derive(&capture).unwrap();
        assert_eq!(legacy.phases.iter().map(|p| p.status).collect::<Vec<_>>(), [LifecycleStatus::Executed, LifecycleStatus::Executed]);
        let mut overlay = AcceptanceOverlay::default();
        overlay.phases.insert("13".into(), native(Some("c1"), true));
        let answer = derive_with(&capture, &overlay).unwrap();
        assert_eq!(answer.phases[0].status, LifecycleStatus::Complete);
        assert_eq!(answer.phases[0].uat, Some(UatCounts { pass: 1, skipped: 1, ..UatCounts::default() }));
        assert_eq!(answer.phases[1].status, LifecycleStatus::Executed, "the legacy phase keeps its table");
        assert_eq!(answer.current.map(|p| p.address()), Some("14".to_owned()));
        overlay.phases.insert("13".into(), native(None, true));
        assert_eq!(derive_with(&capture, &overlay).unwrap().phases[0].status, LifecycleStatus::Executed);
        overlay.phases.insert("13".into(), native(None, false));
        assert_eq!(derive_with(&capture, &overlay).unwrap().phases[0].status, LifecycleStatus::Planned);
        let mut unpublished = native(None, false);
        unpublished.published = false;
        let mut capture = capture;
        capture.phases[0].plans = Observation::Present(vec![]);
        overlay.phases.insert("13".into(), unpublished);
        assert_eq!(derive_with(&capture, &overlay).unwrap().phases[0].status, LifecycleStatus::Unplanned);
    }

    #[test]
    fn the_memo_key_changes_with_the_overlay_and_keeps_the_legacy_key_without_one() {
        let capture = native_capture();
        let legacy = input_key(&capture).unwrap();
        assert_eq!(input_key_with(&capture, &AcceptanceOverlay::default()).unwrap(), legacy);
        let mut overlay = AcceptanceOverlay::default();
        overlay.phases.insert("13".into(), native(None, false));
        let planned = input_key_with(&capture, &overlay).unwrap();
        assert_ne!(planned, legacy);
        overlay.phases.insert("13".into(), native(Some("c1"), true));
        let complete = input_key_with(&capture, &overlay).unwrap();
        assert_ne!(complete, planned);
        assert_eq!(input_key_with(&capture, &overlay).unwrap(), complete);
    }
}
