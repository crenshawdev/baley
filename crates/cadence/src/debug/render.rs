use super::model::Record;

/// A readable projection; continuation always reads the typed record.
pub fn render(record: &Record) -> String {
    let mut out = format!("# Debug: {}\n\nSymptom: {}\nStatus: {}\nAttempts: {}\nVersion: {}\n", record.slug,
        record.symptom, serde_json::to_value(&record.status).unwrap().as_str().unwrap(), record.attempt_count, record.version);
    out.push_str("\n## Hypotheses\n");
    for h in &record.hypotheses {
        out.push_str(&format!("\n{} [{}]: {}\nRank reason: {}\n", h.id,
            serde_json::to_value(&h.state).unwrap().as_str().unwrap(), h.description, h.rank_reason));
    }
    out.push_str("\n## Observations\n");
    for observation in &record.observations {
        out.push_str(&format!("\nTest: {}\nResult: {}\nRules in: {}\nRules out: {}\n", observation.test,
            observation.result, observation.rules_in.join(", "), observation.rules_out.join(", ")));
    }
    out.push_str("\n## Failed attempts\n");
    for attempt in &record.attempts { out.push_str(&format!("\nAttempt: {}\nResult: {}\n", attempt.description, attempt.result)); }
    if let Some(resolution) = &record.resolution {
        out.push_str(&format!("\n## Resolution\n\n{}\nTest: {}\nResult: {}\n", resolution.description, resolution.reproduction.test, resolution.reproduction.result));
    }
    out
}
