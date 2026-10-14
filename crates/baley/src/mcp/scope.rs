//! `scope-submit` on `baley_apply`: a start submission judged whole and
//! recorded as a draft the owner can approve by its digest (design 0004
//! section 5). A submission refused on its content, or after the project's
//! start was approved, records nothing.

use std::path::Path;

use baley_core::scope::{Submission, SubmissionRefusal, judge_submission};
use baley_store::{CommandKind, ProjectId, RequestId, request_digest};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed};
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::{LEDGER_UNAVAILABLE, WriteRequest, prepare_checked};
use crate::scope::submit::{self, SUBMIT_COMMAND};

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `scope-submit` takes beside its `operation`. Unknown fields
/// are refused.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitShape {
    /// A fresh UUID for this submission in lowercase hyphenated form. A retry
    /// of the same submission sends the same id.
    pub request_id: String,
    /// The whole start submission: `description` with `what`, `core_value`
    /// and `constraints`; `stories`, each with a `label`, a `sentence` and a
    /// `status` of `active`, `deferred` or `excluded`; and `roadmap`, the
    /// phases in order, each with a `name`, a `goal`, a `detail`,
    /// `depends_on` as positions in `roadmap` counted from 1, and the
    /// `stories` it delivers by label.
    pub submission: Value,
}

/// The tagged form a call's arguments arrive in, so `operation` is read and
/// left out of the shape.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum SubmitCall {
    #[serde(rename = "scope-submit")]
    Submit(SubmitShape),
}

/// A submission whose arguments passed every check made before preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedSubmission {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The accepted submission.
    pub submission: Submission,
}

/// Judges `scope-submit`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, then the submission whole. A
/// fault is the `refused` answer, which records nothing. A fault in the
/// submission is never `invalid-arguments` (D-08).
pub fn judge_arguments(arguments: &Value) -> Result<JudgedSubmission, Value> {
    let shape = match serde_json::from_value::<SubmitCall>(arguments.clone()) {
        Ok(SubmitCall::Submit(shape)) => shape,
        Err(error) => {
            return Err(Refusal::new("invalid-arguments", error.to_string())
                .slot("arguments")
                .value());
        }
    };
    // One spelling per id, so a retry cannot miss its replay by changing
    // case or wrapping.
    let canonical = uuid::Uuid::parse_str(&shape.request_id)
        .map(|parsed| parsed.hyphenated().to_string())
        .ok();
    if canonical.as_deref() != Some(shape.request_id.as_str()) {
        return Err(Refusal::new(
            "invalid-arguments",
            "request_id must be a UUID in lowercase hyphenated form, such as 9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f. A retry of the same submission sends the same id",
        )
        .slot("request_id")
        .value());
    }
    let submission = judge_submission(&shape.submission).map_err(|refusal| refused(&refusal))?;
    // The content digest stands for the submission, so the same content
    // under the same id replays and changed content is a reused id.
    let digest = request_digest(&json!({
        "command": SUBMIT_COMMAND,
        "submission": submission.digest().to_hex(),
    }))
    .expect("text is canonical");
    Ok(JudgedSubmission {
        request: WriteRequest {
            kind: CommandKind(SUBMIT_COMMAND.into()),
            request_id: RequestId(shape.request_id),
            digest,
        },
        submission,
    })
}

/// The `refused` answer for a submission judged unacceptable, with the
/// fault's place in its details. Positions are counted from 1.
fn refused(refusal: &SubmissionRefusal) -> Value {
    let (reason, details) = match refusal {
        SubmissionRefusal::Malformed { section, field } => (
            "a field of the submission is missing, blank, of the wrong type, unknown or out of range, so nothing was recorded. The details name it",
            json!({"section": section, "field": field}),
        ),
        SubmissionRefusal::StoryDoubleAssigned { story, phases } => (
            "a story is listed by two phases, and a story belongs to one phase at most, so nothing was recorded",
            json!({"story": story, "phases": phases}),
        ),
        SubmissionRefusal::DependencyCycle { phases } => (
            "phases depend on each other in a cycle, so no order can deliver them and nothing was recorded",
            json!({"phases": phases}),
        ),
    };
    Refusal::new(refusal.code(), reason)
        .slot("submission")
        .details(details)
        .value()
}

/// Serves one `scope-submit` call for the session's `preparation`: the
/// arguments are judged first, then the write is prepared with the
/// start-once read as its own check, so a start already approved ends the
/// call before checkout admission and the policy step. Then the submission
/// runs in its own transaction. An answer that ends preparation is returned
/// as it is.
pub fn serve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let ledger = preparation.ledger.as_deref();
    // With no ledger, preparation fails at the project check, before any
    // settings are read from the config folder or the check is asked.
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    let check = |project: &ProjectId| match ledger {
        Some(ledger) => submit::start_once(&ledger.store, project),
        None => Err(failed(
            LEDGER_UNAVAILABLE,
            "the write was checked without a ledger",
            PLACE_LEDGER,
        )),
    };
    let prepared = match prepare_checked(
        ledger.map(|ledger| &ledger.store),
        &preparation.caller,
        &judged.request,
        config,
        preparation.host,
        &preparation.at,
        &mut crate::process::System,
        &check,
    ) {
        Ok(prepared) => prepared,
        Err(answer) => return answer,
    };
    let (Some(ledger), Some(command)) = (ledger, prepared.command) else {
        return failed(
            LEDGER_UNAVAILABLE,
            "the write was prepared without a ledger or a command",
            PLACE_LEDGER,
        );
    };
    submit::answer(
        submit::record(&ledger.store, &command, &judged.submission),
        &ledger.store,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::fixtures::{R1, m, s1};

    fn arguments(request_id: &str, submission: Value) -> Value {
        json!({"operation": "scope-submit", "request_id": request_id,
            "submission": submission})
    }

    fn refused(arguments: &Value) -> Value {
        let refusal = judge_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        refusal
    }

    #[test]
    fn a_request_id_in_another_spelling_or_an_unknown_argument_accepted_is_caught() {
        let refusal = refused(&arguments("9D0C1B7E-2F4A-4B6C-8D1E-3A5B7C9D0E2F", m()));
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "request_id");

        let mut extra = arguments(R1, m());
        extra["owner"] = json!("Ada Owner");
        let refusal = refused(&extra);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");
    }

    #[test]
    fn a_malformed_submission_answered_as_invalid_arguments_is_caught() {
        let refusal = refused(&arguments(R1, json!({"description": 1})));
        assert_eq!(refusal["code"], "malformed-submission");
        assert_eq!(refusal["slot"], "submission");
        assert_eq!(
            refusal["details"],
            json!({"section": "description", "field": "description"})
        );
    }

    #[test]
    fn a_write_request_digest_that_ignores_the_submission_is_caught() {
        let judged = |submission: Value| judge_arguments(&arguments(R1, submission)).unwrap();
        let minimal = judged(m());
        assert_ne!(minimal.request.digest, judged(s1()).request.digest);
        let reversed = json!({
            "roadmap": [{"stories": ["s"], "depends_on": [], "detail": "d", "goal": "g",
                "name": "n"}],
            "stories": [{"status": "active", "sentence": "t", "label": "s"}],
            "description": {"constraints": [], "core_value": "v", "what": "w"}});
        assert_eq!(judged(reversed).request.digest, minimal.request.digest);
        assert_eq!(minimal.request.kind, CommandKind("scope.submit".into()));
        assert_eq!(minimal.request.request_id, RequestId(R1.into()));
    }
}
