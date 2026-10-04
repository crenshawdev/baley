//! The typed answer vocabulary: `ok`, `refused`, `unknown`, `not-applicable`
//! and `failed`.
//!
//! Every operation answers with one of these five and nothing else. The point
//! is that a caller can branch on the tag alone without reading prose, so the
//! spellings are load-bearing: `not-applicable` carries a hyphen because that
//! is how the design document spells it and how every later skill will match
//! on it.
//!
//! `refused` is a SUCCESSFUL tool call carrying a refusal, never an MCP error
//! result (D-07). A refusal that arrives with `isError` set is indistinguishable
//! from a crashed tool, and the informed-retry loop would have nothing left to
//! branch on: it would see a failure where the binary was in fact telling it
//! precisely why the thing it asked for is not allowed.
//!
//! `failed` is the server saying it could not take the call at all, and that
//! the call recorded nothing itself. A refusal reads as a domain answer the
//! ledger may hold, so a server fault such as an unsupported client or a full
//! queue does not borrow that tag. A `failed` answer is a successful tool call
//! too. Only a full queue and a busy ledger are retryable: the same call may
//! succeed unchanged once the queue or the writer frees.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One operation's answer.
///
/// Serialized internally tagged under `status`, so an `ok` payload's fields sit
/// beside the tag at the top level rather than nested under a key:
///
/// ```json
/// {"status": "ok", "version": "3.7.12", "os": "linux"}
/// {"status": "refused", "code": "no-phase-dir", "reason": "the phase has no CONTEXT.md"}
/// ```
///
/// That is the shape the model already reads from the JavaScript seams, whose
/// envelopes are `{ok, ...fields}`, and matching it means the rewrite does not
/// also retrain what an answer looks like.
///
/// The consequence of an internal tag is that `T` MUST serialize as a JSON
/// object - a struct or a map - because there is nowhere to put a bare scalar
/// beside the tag. An operation with one value to report gives it a named
/// field; there is no arm here for a payload without one.
///
/// The four non-`ok` arms carry a machine `code` and a prose `reason`.
/// Goldens compare the code to the JavaScript refusal token (D-04), so two
/// different refusals cannot agree just because they share an arm tag.
///
/// `failed` also names the `place` at fault and always says `recorded: false`
/// and whether the caller may `retryable` try again. Build it with
/// [`Envelope::failed`] so no call site writes those fields by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Envelope<T> {
    /// The operation ran and this is what it produced.
    Ok(T),
    /// The operation could have run and Baley declined to run it. The caller
    /// asked a well-formed question and the answer is no.
    Refused {
        /// Machine token in the operation's kebab-case vocabulary.
        code: String,
        /// Why, in words a person reads.
        reason: String,
    },
    /// Baley cannot say. The question is a fair one for this operation and
    /// the evidence to answer it is missing, unreadable or ambiguous.
    Unknown {
        /// Machine token in the operation's kebab-case vocabulary.
        code: String,
        /// What could not be established, in words a person reads.
        reason: String,
    },
    /// The question does not apply here, so there is nothing to be right or
    /// wrong about - a phase gate asked of a repository with no phases.
    #[serde(rename = "not-applicable")]
    NotApplicable {
        /// Machine token in the operation's kebab-case vocabulary.
        code: String,
        /// Why the question does not apply, in words a person reads.
        reason: String,
    },
    /// The server could not take the call and recorded nothing. Not a domain
    /// answer: see the module comment on why this is not `refused`.
    Failed {
        /// Machine token in the operation's kebab-case vocabulary.
        code: String,
        /// What went wrong, in words a person reads.
        reason: String,
        /// The input at fault, such as `client-info` or `CLAUDE_PROJECT_DIR`.
        place: String,
        /// Always `false`: the call recorded nothing itself. When it was
        /// preparing a write, a checkout admission that ran first stays
        /// recorded if a later step fails, as on the command line, because
        /// checkout admission and the policy step are separate transactions.
        recorded: bool,
        /// True only when the same call may succeed later unchanged: the
        /// codes in [`SERVER_OVERLOADED`] and [`LEDGER_BUSY`].
        retryable: bool,
        /// Structured detail a caller can act on, in the failure's own shape.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<serde_json::Value>,
    },
}

/// A failed code a caller may retry unchanged: the session queue was full, and
/// the same call may be taken once it drains.
pub const SERVER_OVERLOADED: &str = "server-overloaded";

/// A failed code a caller may retry unchanged: the ledger's writer was busy,
/// and the same call may succeed once it frees.
pub const LEDGER_BUSY: &str = "ledger-busy";

/// Every failed code a caller may retry unchanged.
const RETRYABLE_CODES: [&str; 2] = [SERVER_OVERLOADED, LEDGER_BUSY];

impl<T> Envelope<T> {
    /// A `failed` answer with no structured detail. `recorded` is always false
    /// and `retryable` is true only for [`SERVER_OVERLOADED`] and
    /// [`LEDGER_BUSY`], so a caller has one shape to branch on.
    pub fn failed(
        code: impl Into<String>,
        reason: impl Into<String>,
        place: impl Into<String>,
    ) -> Self {
        Self::build_failed(code.into(), reason.into(), place.into(), None)
    }

    /// A `failed` answer that also carries structured detail.
    pub fn failed_with_details(
        code: impl Into<String>,
        reason: impl Into<String>,
        place: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self::build_failed(code.into(), reason.into(), place.into(), Some(details))
    }

    fn build_failed(
        code: String,
        reason: String,
        place: String,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self::Failed {
            retryable: RETRYABLE_CODES.contains(&code.as_str()),
            code,
            reason,
            place,
            recorded: false,
            details,
        }
    }
}

/// A refusal built where a typed [`Envelope`] cannot be: the read layer's
/// `Value` answers, the verification verdicts, native-execution admission and
/// the server's own argument parsing. Nothing else writes `status: refused`;
/// the scan in `tests/refusal_shape.rs` keeps it that way.
///
/// `code` and `reason` are always present, the same pair the [`Envelope`]
/// arms carry, so a caller branches on `code` without knowing which subsystem
/// answered. The rest say where the fault is and appear only when set, so a
/// refusal with nothing more to say carries nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Refusal {
    status: Refused,
    /// Machine token in the operation's kebab-case vocabulary.
    code: String,
    /// Why, in words a person reads.
    reason: String,
    /// The rule that refused, when the operation names its rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rule: Option<String>,
    /// The argument or document slot at fault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phase: Option<u32>,
    /// Zero-based within the named section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entry: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// Structured detail a caller can act on, in the operation's own shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

/// The one value `status` takes on a [`Refusal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum Refused {
    Refused,
}

impl Refusal {
    pub fn new(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            status: Refused::Refused,
            code: code.into(),
            reason: reason.into(),
            rule: None,
            slot: None,
            phase: None,
            entry: None,
            id: None,
            details: None,
        }
    }

    pub fn rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    pub fn slot(mut self, slot: impl Into<String>) -> Self {
        self.slot = Some(slot.into());
        self
    }

    pub fn phase(mut self, phase: u32) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn entry(mut self, entry: usize) -> Self {
        self.entry = Some(entry);
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// The refusal as the JSON a caller receives.
    pub fn value(self) -> serde_json::Value {
        serde_json::to_value(self).expect("a refusal serializes")
    }
}

impl From<Refusal> for serde_json::Value {
    fn from(refusal: Refusal) -> Self {
        refusal.value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A stand-in for a real operation's payload. Struct-shaped on purpose:
    /// see the `Envelope` doc comment on why a payload has named fields.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    struct Payload {
        phase: u32,
    }

    #[test]
    fn ok_puts_the_payload_fields_beside_the_tag() {
        let value = serde_json::to_value(Envelope::Ok(Payload { phase: 1 })).unwrap();
        assert_eq!(value, json!({"status": "ok", "phase": 1}));
    }

    #[test]
    fn refused_carries_a_code_and_prose_reason() {
        let envelope: Envelope<Payload> = Envelope::Refused {
            code: "no-phase-dir".to_string(),
            reason: "the phase has no CONTEXT.md".to_string(),
        };
        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            json!({"status": "refused", "code": "no-phase-dir", "reason": "the phase has no CONTEXT.md"})
        );
    }

    #[test]
    fn a_refusal_with_only_a_code_and_reason_matches_the_envelope_arm() {
        let typed: Envelope<Payload> = Envelope::Refused {
            code: "no-phase-dir".to_string(),
            reason: "the phase has no CONTEXT.md".to_string(),
        };
        assert_eq!(
            Refusal::new("no-phase-dir", "the phase has no CONTEXT.md").value(),
            serde_json::to_value(typed).unwrap()
        );
    }

    #[test]
    fn a_refusal_carries_its_location_fields_only_when_set() {
        let full = Refusal::new("read-contract", "no such unit")
            .rule("issued-location")
            .slot("unit")
            .phase(31)
            .entry(2)
            .id("T1")
            .details(json!({"units": ["a"]}))
            .value();
        assert_eq!(
            full,
            json!({"status": "refused", "code": "read-contract", "reason": "no such unit", "rule": "issued-location",
                "slot": "unit", "phase": 31, "entry": 2, "id": "T1", "details": {"units": ["a"]}})
        );
        let bare = Refusal::new("no-phase-dir", "the phase has no CONTEXT.md").value();
        assert_eq!(
            bare.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["status", "code", "reason"]
        );
    }

    #[test]
    fn unknown_carries_a_code_and_prose_reason() {
        let envelope: Envelope<Payload> = Envelope::Unknown {
            code: "unreadable-tree".to_string(),
            reason: "the working tree could not be read".to_string(),
        };
        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            json!({"status": "unknown", "code": "unreadable-tree", "reason": "the working tree could not be read"})
        );
    }

    /// The hyphen is the whole test. `not_applicable` and `notApplicable` are
    /// both plausible renderings of the Rust variant and neither is the one
    /// the design document spells or a later skill will match on.
    #[test]
    fn not_applicable_keeps_its_hyphen() {
        let envelope: Envelope<Payload> = Envelope::NotApplicable {
            code: "no-phases".to_string(),
            reason: "this repository has no phases".to_string(),
        };
        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["status"], json!("not-applicable"));
        assert_eq!(
            value,
            json!({"status": "not-applicable", "code": "no-phases", "reason": "this repository has no phases"})
        );
    }

    #[test]
    fn failed_serializes_under_its_own_status_with_place_and_recorded_false() {
        let envelope: Envelope<Payload> = Envelope::failed(
            "project-context-missing",
            "CLAUDE_PROJECT_DIR is not set",
            "CLAUDE_PROJECT_DIR",
        );
        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            json!({"status": "failed", "code": "project-context-missing",
                "reason": "CLAUDE_PROJECT_DIR is not set", "place": "CLAUDE_PROJECT_DIR",
                "recorded": false, "retryable": false})
        );
    }

    #[test]
    fn only_server_overloaded_and_ledger_busy_are_retryable() {
        let retryable = |code: &str| {
            let value = serde_json::to_value(Envelope::<Payload>::failed(code, "r", "p")).unwrap();
            value["retryable"].clone()
        };
        assert_eq!(retryable("server-overloaded"), json!(true));
        assert_eq!(retryable("ledger-busy"), json!(true));
        for code in [
            "unknown-host",
            "caller-invalid",
            "project-context-invalid",
            "not-a-project",
            "config-unavailable",
            "project-not-in-ledger",
            "checkout-facts-unavailable",
            "project-id-conflict",
            "ledger-unavailable",
        ] {
            assert_eq!(
                retryable(code),
                json!(false),
                "{code} must not be retryable"
            );
        }
    }

    #[test]
    fn failed_carries_details_only_when_given() {
        let with: Envelope<Payload> = Envelope::failed_with_details(
            "unknown-host",
            "the client is not supported",
            "client-info",
            json!({"supported": ["claude-code"]}),
        );
        assert_eq!(
            serde_json::to_value(with).unwrap()["details"],
            json!({"supported": ["claude-code"]})
        );
        let bare = serde_json::to_value(Envelope::<Payload>::failed("c", "r", "p")).unwrap();
        assert!(bare.get("details").is_none());
    }

    #[test]
    fn every_arm_round_trips_through_serde_json() {
        let arms: Vec<Envelope<Payload>> = vec![
            Envelope::Ok(Payload { phase: 4 }),
            Envelope::Refused {
                code: "no-phase-dir".to_string(),
                reason: "no".to_string(),
            },
            Envelope::Unknown {
                code: "unreadable-tree".to_string(),
                reason: "cannot say".to_string(),
            },
            Envelope::NotApplicable {
                code: "no-phases".to_string(),
                reason: "does not apply".to_string(),
            },
            Envelope::failed_with_details(
                "unknown-host",
                "no",
                "client-info",
                json!({"supported": ["claude-code"]}),
            ),
            Envelope::failed("server-overloaded", "busy", "queue"),
        ];
        for arm in arms {
            let text = serde_json::to_string(&arm).unwrap();
            let back: Envelope<Payload> = serde_json::from_str(&text).unwrap();
            assert_eq!(back, arm, "round trip changed the envelope: {text}");
        }
    }
}
