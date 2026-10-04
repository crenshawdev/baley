//! Admits one `tools/call` in the order faults are answered.
//!
//! The gate before the queue answers first: an unknown tool, an unsupported
//! client and an unknown or retired operation never reach the queue, so an
//! unsupported client cannot take a slot. Then the queue answers overload. The
//! project and the caller are checked after this, on the worker, so overload
//! always comes before a project fault.

use rmcp::ErrorData;
use rmcp::model::CallToolResult;

use super::client::Selection;
use super::gate::{Admitted, Before, before_queue, encode};
use super::queue::{Placed, Queue};
use crate::envelope::{Envelope, SERVER_OVERLOADED};

/// What admission decided.
#[derive(Debug, PartialEq)]
pub enum Admission<T> {
    /// Answer now, as a successful tool result. Nothing was queued.
    Answer(CallToolResult),
    /// Fail the request as a protocol error. Nothing was queued.
    ProtocolError(ErrorData),
    /// The call took a place in the queue.
    Accepted(Placed<T>),
}

/// Decides one call. `raw_bytes` is the size of the call's whole frame as the
/// transport read it. `job` builds what the worker will run from what the gate
/// admitted, and is called only for a call that reaches the queue.
pub fn admit<T>(
    tool: &str,
    selection: &Selection,
    spelling: Option<&str>,
    raw_bytes: usize,
    queue: &mut Queue<T>,
    job: impl FnOnce(Admitted) -> T,
) -> Admission<T> {
    let admitted = match before_queue(tool, selection, spelling) {
        Before::Proceed(admitted) => admitted,
        Before::Answer(result) => return Admission::Answer(result),
        Before::ProtocolError(error) => return Admission::ProtocolError(error),
    };
    match queue.admit(job(admitted), raw_bytes) {
        Ok(placed) => Admission::Accepted(placed),
        Err(_) => Admission::Answer(overloaded()),
    }
}

/// The answer for a call the queue refused. A closed queue says the same: the
/// session's next server can take the call.
fn overloaded() -> CallToolResult {
    let answer = Envelope::<serde_json::Value>::failed(
        SERVER_OVERLOADED,
        "Baley is holding as many calls as it takes, so the call was not started. Try it again",
        "queue",
    );
    encode(serde_json::to_value(answer).expect("failed answer"))
}

#[cfg(test)]
mod tests {
    use super::super::client::Reported;
    use super::super::gate::Called;
    use super::super::operations::Tool;
    use super::super::queue::BYTE_CAP;
    use super::*;
    use baley_core::policy::Host;
    use serde_json::{Value, json};

    fn supported() -> Selection {
        Selection::Supported {
            host: Host::ClaudeCode,
            client_version: "2.1.287".into(),
        }
    }

    fn unsupported() -> Selection {
        Selection::UnknownHost {
            name: Some(Reported::Text("codex".into())),
            version: Some(Reported::Text("1.0".into())),
        }
    }

    fn held() -> Admitted {
        Admitted {
            called: Called::Version,
            host: Host::ClaudeCode,
            client_version: "2.1.287".into(),
            needs_project: false,
        }
    }

    fn full() -> Queue<Admitted> {
        let mut queue = Queue::new();
        for _ in 0..5 {
            queue.admit(held(), 10).unwrap();
        }
        queue
    }

    fn run(
        tool: &str,
        selection: &Selection,
        spelling: Option<&str>,
        queue: &mut Queue<Admitted>,
    ) -> Admission<Admitted> {
        admit(tool, selection, spelling, 100, queue, |admitted| admitted)
    }

    fn answer(admission: Admission<Admitted>) -> Value {
        match admission {
            Admission::Answer(result) => {
                assert_ne!(result.is_error, Some(true));
                result.structured_content.expect("a structured answer")
            }
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    #[test]
    fn an_unsupported_client_changes_the_queue_state() {
        let mut queue = Queue::new();
        for _ in 0..5 {
            let _ = run("baley_query", &unsupported(), Some("help"), &mut queue);
        }
        assert!(queue.is_drained());
    }

    #[test]
    fn an_unsupported_client_with_a_full_queue_answers_overload_instead_of_unknown_host() {
        let mut queue = full();
        let value = answer(run("baley_query", &unsupported(), Some("help"), &mut queue));
        assert_eq!(value["code"], "unknown-host");
    }

    #[test]
    fn a_retired_operation_with_a_full_queue_answers_overload() {
        let mut queue = full();
        let value = answer(run(
            "baley_apply",
            &supported(),
            Some("execution-run"),
            &mut queue,
        ));
        assert_eq!(value["code"], "operation-unavailable");
    }

    #[test]
    fn an_unrecognized_operation_with_a_full_queue_answers_overload() {
        let mut queue = full();
        let value = answer(run("baley_query", &supported(), Some("nope"), &mut queue));
        assert_eq!(value["code"], "unknown-operation");
    }

    #[test]
    fn an_unknown_tool_with_a_full_queue_answers_anything_but_the_protocol_error() {
        let mut queue = full();
        let admission = run("baley_other", &supported(), Some("help"), &mut queue);
        assert!(
            matches!(admission, Admission::ProtocolError(_)),
            "{admission:?}"
        );
    }

    #[test]
    fn a_supported_call_to_an_available_operation_with_a_full_queue_is_accepted() {
        let mut queue = full();
        let value = answer(run("baley_query", &supported(), Some("help"), &mut queue));
        assert_eq!(value["status"], "failed");
        assert_eq!(value["code"], "server-overloaded");
    }

    #[test]
    fn the_overload_answer_lacks_retryable_true_or_recorded_false() {
        let mut queue = full();
        let value = answer(run("baley_query", &supported(), Some("help"), &mut queue));
        assert_eq!(value["retryable"], json!(true));
        assert_eq!(value["recorded"], json!(false));
        assert_eq!(value["place"], "queue");
    }

    #[test]
    fn an_accepted_call_is_started_when_idle_and_queued_behind_a_running_one() {
        let mut queue = Queue::new();
        let first = run("baley_query", &supported(), Some("help"), &mut queue);
        let Admission::Accepted(Placed::Started(job)) = first else {
            panic!("an idle queue starts the call: {first:?}");
        };
        assert_eq!(job.called, Called::Operation(Tool::Query, "help".into()));
        assert_eq!(job.host, Host::ClaudeCode);
        let second = run("baley_version", &supported(), None, &mut queue);
        assert!(
            matches!(second, Admission::Accepted(Placed::Queued)),
            "{second:?}"
        );
    }

    #[test]
    fn a_call_that_would_pass_the_byte_cap_with_a_slot_free_is_accepted() {
        let mut queue = Queue::new();
        queue.admit(held(), BYTE_CAP - 50).unwrap();
        let value = answer(admit(
            "baley_version",
            &supported(),
            None,
            51,
            &mut queue,
            |admitted| admitted,
        ));
        assert_eq!(value["code"], "server-overloaded");
        let fits = admit("baley_version", &supported(), None, 50, &mut queue, |a| a);
        assert!(
            matches!(fits, Admission::Accepted(Placed::Queued)),
            "{fits:?}"
        );
    }

    #[test]
    fn a_closed_queue_takes_a_call_instead_of_answering_overload() {
        let mut queue = Queue::new();
        queue.close();
        let value = answer(run("baley_query", &supported(), Some("help"), &mut queue));
        assert_eq!(value["code"], "server-overloaded");
        assert_eq!(value["retryable"], json!(true));
    }

    #[test]
    fn a_call_the_gate_answers_still_builds_the_worker_job() {
        let mut built = 0;
        let mut queue = Queue::new();
        let _ = admit(
            "baley_query",
            &unsupported(),
            Some("help"),
            1,
            &mut queue,
            |admitted| {
                built += 1;
                admitted
            },
        );
        assert_eq!(built, 0);
    }
}
