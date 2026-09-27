use super::*;

fn status_name(status: LifecycleStatus) -> &'static str {
    match status {
        LifecycleStatus::Unplanned => "unplanned",
        LifecycleStatus::Planned => "planned",
        LifecycleStatus::Executed => "executed",
        LifecycleStatus::Complete => "complete",
    }
}

/// Declarations never determine lifecycle. Refuse the first disagreement in
/// parsed ROADMAP order (including textual ties).
pub fn check_consistency(
    declarations: &ParsedRoadmap,
    answer: &Lifecycle,
) -> Result<(), DerivationError> {
    if let Some((issue, entry)) = located_conflicts(declarations, answer).into_iter().next() {
        return Err(DerivationError::StateConflict {
            source: issue.source,
            field: issue.field,
            declared: issue.declared,
            derived: issue.derived,
            entry: Some(Box::new(entry)),
        });
    }
    Ok(())
}

/// Progress retains each disagreement without granting it execution authority.
pub fn roadmap_conflicts(declarations: &ParsedRoadmap, answer: &Lifecycle) -> Vec<RoadmapConflict> {
    located_conflicts(declarations, answer)
        .into_iter()
        .map(|(issue, _)| issue)
        .collect()
}

/// Each disagreement with the roadmap line it sits on, so a refusal over the
/// first one can be joined to that line rather than to a flattened sentence.
fn located_conflicts(
    declarations: &ParsedRoadmap,
    answer: &Lifecycle,
) -> Vec<(RoadmapConflict, ConflictEntry)> {
    let mut issues = Vec::new();
    for (declaration, phase) in declarations.phases.iter().zip(&answer.phases) {
        let complete = phase.status == LifecycleStatus::Complete;
        if declaration.checked != complete {
            issues.push((
                RoadmapConflict {
                    phase: phase.id,
                    status: phase.status,
                    source: format!(
                        "ROADMAP.md:{} entry {}",
                        declaration.source_line, declaration.ordinal
                    ),
                    field: "complete".into(),
                    declared: declaration.checked.to_string(),
                    derived: complete.to_string(),
                },
                ConflictEntry {
                    source: "ROADMAP.md".into(),
                    line: declaration.source_line as u64,
                    entry: declaration.ordinal as u64,
                    phase: phase.id.address(),
                    status: status_name(phase.status).into(),
                },
            ));
        }
    }
    issues
}
