//! `scope-submit` and `scope-approve` on `baley_apply`: a start submission
//! judged whole and recorded as a draft, and the owner's approval of a draft
//! by its digest (design 0004 section 5). A submission refused on its
//! content, or after the project's start was approved, records nothing, and
//! neither does an approval refused as stale, unknown or a second start. An
//! approval names the owner from `owner.name`, or git's `user.name` when it
//! is unset, and takes Baley's time; the session supplies neither.

use std::cell::RefCell;
use std::path::Path;

use baley_core::scope::{Submission, SubmissionRefusal, judge_submission};
use baley_store::{CommandKind, Hash, ProjectId, RequestId, request_digest};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::mcp::capture::{PLACE_LEDGER, failed};
use crate::mcp::client::MAX_ECHOED_BYTES;
use crate::mcp::handler::Preparation;
use crate::mcp::prepare::{LEDGER_UNAVAILABLE, Prepared, WriteRequest, prepare_checked};
use crate::process::Process;
use crate::providers::identity::{self, OWNER_IDENTITY_MISSING};
use crate::scope::approve::{self, APPROVE_COMMAND};
use crate::scope::submit::{self, SUBMIT_COMMAND};

/// Where a missing owner identity is answered: the owner's settings.
const PLACE_SETTINGS: &str = "settings";

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
            return Err(
                Refusal::new("invalid-arguments", bounded(error.to_string()))
                    .slot("arguments")
                    .value(),
            );
        }
    };
    let request_id = canonical_request_id(shape.request_id, "submission")?;
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
            request_id,
            digest,
        },
        submission,
    })
}

/// Serde's text for a shape fault, cut to the echo limit with a closing
/// `...`, since it repeats an unknown field's name at whatever length the
/// caller sent.
fn bounded(text: String) -> String {
    const CUT: &str = "...";
    if text.len() <= MAX_ECHOED_BYTES {
        return text;
    }
    let mut end = MAX_ECHOED_BYTES - CUT.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{CUT}", &text[..end])
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

// `schema` serves this same declaration, so its doc comments are wire text.
/// The arguments `scope-approve` takes beside its `operation`. Unknown fields
/// are refused: the owner and the time are Baley's to supply, never the
/// session's.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveShape {
    /// A fresh UUID for this approval in lowercase hyphenated form. A retry
    /// of the same approval sends the same id.
    pub request_id: String,
    /// The digest of the draft to approve, as `scope-submit` returned it: 64
    /// lowercase hex characters.
    pub digest: String,
}

/// The tagged form a `scope-approve` call's arguments arrive in.
#[derive(Deserialize)]
#[serde(tag = "operation")]
enum ApproveCall {
    #[serde(rename = "scope-approve")]
    Approve(ApproveShape),
}

/// An approval whose arguments passed every check made before preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedApproval {
    /// The write preparation carries into the domain command.
    pub request: WriteRequest,
    /// The digest of the draft to approve.
    pub digest: Hash,
}

/// Judges `scope-approve`'s arguments before anything is prepared or
/// recorded: the shape, then the request id, then the digest. A fault is the
/// `refused` answer `invalid-arguments`, which records nothing.
pub fn judge_approve_arguments(arguments: &Value) -> Result<JudgedApproval, Value> {
    let shape = match serde_json::from_value::<ApproveCall>(arguments.clone()) {
        Ok(ApproveCall::Approve(shape)) => shape,
        Err(error) => {
            return Err(
                Refusal::new("invalid-arguments", bounded(error.to_string()))
                    .slot("arguments")
                    .value(),
            );
        }
    };
    let request_id = canonical_request_id(shape.request_id, "approval")?;
    // One spelling per digest, as `scope-submit` returns it, so a digest that
    // could never name a draft is the caller's mistake, not `unknown-draft`.
    let Some(digest) = Hash::from_hex(&shape.digest).filter(|hash| hash.to_hex() == shape.digest)
    else {
        return Err(Refusal::new(
            "invalid-arguments",
            "digest must be 64 lowercase hex characters, the digest scope-submit returned for the draft",
        )
        .slot("digest")
        .value());
    };
    let request = request_digest(&json!({
        "command": APPROVE_COMMAND,
        "digest": shape.digest,
    }))
    .expect("text is canonical");
    Ok(JudgedApproval {
        request: WriteRequest {
            kind: CommandKind(APPROVE_COMMAND.into()),
            request_id,
            digest: request,
        },
        digest,
    })
}

/// The request id when it is a UUID in its one lowercase hyphenated
/// spelling, so a retry cannot miss its replay by changing case or wrapping.
/// Otherwise the `invalid-arguments` refusal, slot `request_id`, saying a
/// retry of the same `call` sends the same id.
fn canonical_request_id(request_id: String, call: &str) -> Result<RequestId, Value> {
    let canonical = uuid::Uuid::parse_str(&request_id)
        .map(|parsed| parsed.hyphenated().to_string())
        .ok();
    if canonical.as_deref() != Some(request_id.as_str()) {
        return Err(Refusal::new(
            "invalid-arguments",
            format!(
                "request_id must be a UUID in lowercase hyphenated form, such as 9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f. A retry of the same {call} sends the same id"
            ),
        )
        .slot("request_id")
        .value());
    }
    Ok(RequestId(request_id))
}

/// The owner an approval is recorded under, from what preparation found.
/// With the policy of a fresh write it is the owner's identity, `owner.name`
/// first and git's `user.name` through `process` when it is unset. With
/// none, a replay, it is `None`, and nothing is read or launched (D-18). A
/// missing identity is `failed` at `settings`, not retryable, since the
/// owner's settings are at fault and not the call's arguments.
pub fn approval_owner(
    prepared: &Prepared,
    process: &mut dyn Process,
) -> Result<Option<String>, Value> {
    let Some(policy) = &prepared.policy else {
        return Ok(None);
    };
    identity::owner(identity::owner_name(policy), process)
        .map(Some)
        .map_err(|text| failed(OWNER_IDENTITY_MISSING, text, PLACE_SETTINGS))
}

/// Serves one `scope-approve` call for the session's `preparation`: the
/// arguments are judged first, then the write is prepared with the
/// approval's first step as its own check, so a `stale-draft`,
/// `unknown-draft` or `project-already-started` refusal ends the call before
/// checkout admission and the policy step and records nothing. A request the
/// ledger already holds skips the check and is answered from its record.
/// Then the owner is decided from the prepared result, and the second step
/// records the approval in its own transaction. An answer that ends
/// preparation is returned as it is.
pub fn serve_approve(arguments: &Value, preparation: &Preparation) -> Value {
    let judged = match judge_approve_arguments(arguments) {
        Ok(judged) => judged,
        Err(refusal) => return refusal,
    };
    let ledger = preparation.ledger.as_deref();
    let config = ledger.map_or(Path::new(""), |ledger| ledger.config.as_path());
    // The first step's reads, kept for the second step. None on a replay,
    // which skips the check.
    let first = RefCell::new(None);
    let check = |project: &ProjectId| match ledger {
        Some(ledger) => {
            let read = approve::judge(&ledger.store, project, judged.digest)?;
            *first.borrow_mut() = Some(read);
            Ok(())
        }
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
    let owner = match approval_owner(&prepared, &mut crate::process::System) {
        Ok(owner) => owner,
        Err(answer) => return answer,
    };
    let (Some(ledger), Some(command)) = (ledger, prepared.command) else {
        return failed(
            LEDGER_UNAVAILABLE,
            "the write was prepared without a ledger or a command",
            PLACE_LEDGER,
        );
    };
    // A replay is answered from its record before any decision runs, so its
    // empty owner and missing reads are never used.
    let first = first.into_inner();
    submit::answer(
        approve::record(
            &ledger.store,
            &command,
            first.as_ref(),
            owner.as_deref().unwrap_or_default(),
        ),
        &ledger.store,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_step::{self, Reads};
    use crate::process::Recorded;
    use crate::scope::fixtures::{M_DIGEST, R1, m, project, s1};
    use crate::settings;

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
    fn an_unknown_argument_name_echoed_at_any_length_is_caught() {
        // Three bytes a character, so the cut cannot land on a whole one by luck.
        let mut extra = arguments(R1, m());
        extra["名".repeat(10_000)] = json!(true);
        let refusal = refused(&extra);
        assert_eq!(refusal["code"], "invalid-arguments");
        assert_eq!(refusal["slot"], "arguments");
        let reason = refusal["reason"].as_str().expect("a reason");
        assert!(reason.len() <= MAX_ECHOED_BYTES, "{} bytes", reason.len());
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

    fn approve(digest: &str) -> Value {
        json!({"operation": "scope-approve", "request_id": R1, "digest": digest})
    }

    fn refused_approval(arguments: &Value) -> Value {
        let refusal = judge_approve_arguments(arguments).expect_err("refused");
        assert_eq!(refusal["status"], "refused", "{refusal}");
        assert_eq!(refusal["code"], "invalid-arguments", "{refusal}");
        refusal
    }

    #[test]
    fn an_approval_carrying_an_owner_or_time_accepted_is_caught() {
        for (field, value) in [
            ("owner", "Mallory"),
            ("at", "2026-01-01T00:00:00Z"),
            ("time", "now"),
        ] {
            let mut arguments = approve(M_DIGEST);
            arguments[field] = json!(value);
            assert_eq!(refused_approval(&arguments)["slot"], "arguments", "{field}");
        }
    }

    #[test]
    fn a_digest_not_64_lowercase_hex_accepted_is_caught() {
        let upper = format!("A{}", &M_DIGEST[1..]);
        let with_g = format!("g{}", &M_DIGEST[1..]);
        for digest in [&M_DIGEST[..63], &format!("{M_DIGEST}0"), &upper, &with_g] {
            assert_eq!(
                refused_approval(&approve(digest))["slot"],
                "digest",
                "{digest}"
            );
        }
    }

    /// A prepared write over the global settings text, or a replay when
    /// there is none.
    fn prepared(global: Option<&str>) -> Prepared {
        let policy = global.map(|text| {
            let reads = Reads {
                global: Ok(Some(settings::file(
                    std::path::Path::new("/c/config.toml"),
                    text.as_bytes().to_vec(),
                ))),
                head: None,
            };
            policy_step::build(&reads, None).unwrap()
        });
        Prepared {
            project: project(),
            root: "/real/r".into(),
            command: None,
            policy,
        }
    }

    #[test]
    fn a_replay_reading_settings_or_launching_git_is_caught() {
        let mut process = Recorded::new();
        assert_eq!(approval_owner(&prepared(None), &mut process), Ok(None));
        assert!(process.launches().is_empty());

        let mut process = Recorded::new().out("Git Owner\n");
        let set = prepared(Some("[owner]\nname = \"Ada Owner\"\n"));
        assert_eq!(
            approval_owner(&set, &mut process),
            Ok(Some("Ada Owner".into()))
        );
        assert!(process.launches().is_empty());

        let mut process = Recorded::new().out("Git Owner\n");
        assert_eq!(
            approval_owner(&prepared(Some("")), &mut process),
            Ok(Some("Git Owner".into()))
        );
        assert_eq!(process.launches().len(), 1);
    }

    #[test]
    fn a_missing_owner_answered_as_an_argument_refusal_is_caught() {
        let mut process = Recorded::new().fail(1, "");
        let answer = approval_owner(&prepared(Some("")), &mut process).expect_err("failed");
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "owner-identity-missing", "{answer}");
        assert_eq!(answer["place"], "settings", "{answer}");
        assert_eq!(answer["retryable"], false, "{answer}");
        assert!(
            answer["reason"].as_str().unwrap().contains("owner.name"),
            "{answer}"
        );
    }
}
