//! Whether the policy step records: the payload it would record against the
//! `policy` document stored at its key (design 0003 section 6, CFG-R9).

use serde_json::Value;

/// The members a record is compared on. The project, checkout and host are
/// the key, and `host_key` and `version` are the document's own stamp.
const COMPARED: [&str; 4] = ["values", "sources", "diagnostics", "catalog_version"];

/// What the policy step does with a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyJudgement {
    /// The stored record already holds this policy.
    Unchanged {
        /// The version in force: the stored record's sequence.
        version: u64,
    },
    /// Record one `policy.effective`.
    Record,
}

/// Judges `payload` against the body stored at its key, if any.
///
/// The whole-file refs and the pending note are not in the payload, so an
/// uncommitted edit records nothing, and neither does a comment in a file
/// that sets nothing unless it moves an ignored name, since a diagnostic
/// carries its line and column. Any byte change to a file that sets a value moves that value's
/// source digest and records. A stored body missing a compared member or a
/// positive `version` was not written by `PolicyProjector`, and recording
/// again repairs it.
pub fn judge_policy(payload: &Value, stored: Option<&Value>) -> PolicyJudgement {
    let Some(stored) = stored else {
        return PolicyJudgement::Record;
    };
    let version = match stored.get("version").and_then(Value::as_u64) {
        Some(version) if version > 0 => version,
        _ => return PolicyJudgement::Record,
    };
    let same = COMPARED.iter().all(|member| {
        stored
            .get(member)
            .is_some_and(|held| payload.get(member) == Some(held))
    });
    if same {
        PolicyJudgement::Unchanged { version }
    } else {
        PolicyJudgement::Record
    }
}
