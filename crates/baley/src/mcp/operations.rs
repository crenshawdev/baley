//! The operation baseline: today's spellings per tool, in today's order, and
//! the build that replaces each one Baley no longer serves.
//!
//! The lists are literal data on purpose. They are not derived from the
//! binary's service types, which a later build parks and deletes, and names
//! are only ever added to them. An operation that is not served answers
//! `operation-unavailable` naming its replacing build, so a caller that knows
//! the old spelling learns where it went.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::envelope::Refusal;
use crate::instruction;
use crate::mcp::parts::{Cut, PART_BOUND, cut};

/// One of the two tools that take an `operation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// `baley_query`.
    Query,
    /// `baley_apply`.
    Apply,
}

impl Tool {
    /// The tool's name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::Query => "baley_query",
            Self::Apply => "baley_apply",
        }
    }

    /// The tool's recognized spellings in baseline order.
    pub fn operations(self) -> &'static [Operation] {
        match self {
            Self::Query => QUERY_OPERATIONS,
            Self::Apply => APPLY_OPERATIONS,
        }
    }
}

/// Whether Baley serves a recognized spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Served by this server.
    Available {
        /// Whether it needs a project, as `capture`, the one served write, does.
        needs_project: bool,
    },
    /// Not served here, and the build that replaces it.
    Retired {
        /// The replacing build's number.
        build: u32,
    },
}

/// One recognized spelling and whether it is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    /// The spelling a caller sends as `operation`.
    pub name: &'static str,
    /// Whether it is served.
    pub status: Status,
}

/// A spelling served with no project.
const fn available(name: &'static str) -> Operation {
    Operation {
        name,
        status: Status::Available {
            needs_project: false,
        },
    }
}

/// A spelling not served here, replaced by `build`.
const fn retired(name: &'static str, build: u32) -> Operation {
    Operation {
        name,
        status: Status::Retired { build },
    }
}

/// `baley_query` spellings, in the order the tool has always listed them.
pub const QUERY_OPERATIONS: &[Operation] = &[
    available("help"),
    retired("recall", 8),
    retired("debug-list", 8),
    retired("debug-status", 8),
    retired("debug-continue", 8),
    retired("undo-read", 6),
    retired("milestone-read", 6),
    retired("land-read", 6),
    retired("progress", 7),
    retired("suggest", 7),
    retired("why", 8),
    retired("document", 3),
    retired("document-search", 8),
    retired("verify-next", 5),
    retired("verification-read", 5),
    retired("verification-audit", 5),
    retired("execution-history", 5),
    retired("evidence-read", 5),
    retired("plan-read", 4),
    retired("context-intake", 4),
    retired("route", 4),
    retired("config-entry", 2),
    retired("config-facts", 2),
    retired("config-interview", 2),
    retired("detect-surfaces", 4),
    retired("execute-next", 5),
    retired("risk-status", 4),
    available("schema"),
    retired("review-next", 4),
    retired("review-admission", 4),
    retired("review-material", 4),
    retired("review-original", 4),
    retired("review-attempt", 4),
    retired("review-roster", 4),
    retired("review-inventory", 4),
    retired("review-deferred", 4),
    retired("review-consumer", 4),
    retired("review-select", 4),
    available("instruction"),
];

/// `baley_apply` spellings, in the order the tool has always listed them.
pub const APPLY_OPERATIONS: &[Operation] = &[
    retired("verification-run", 5),
    retired("verification-submit", 5),
    retired("truth-waive", 5),
    retired("verification-human-result", 5),
    retired("verification-complete", 5),
    retired("execution-task-retire", 5),
    retired("execution-task-progress", 5),
    retired("execution-task-checkpoint", 5),
    retired("execution-task-answer", 5),
    retired("execution-task-close", 5),
    retired("execution-classify-run", 5),
    retired("execution-owner-attest", 5),
    retired("execution-task-start", 5),
    retired("execution-run", 5),
    retired("execution-worker-exit", 5),
    retired("execution-round-record", 5),
    retired("execution-suite", 5),
    retired("execution-suite-repair-answer", 5),
    retired("execution-suite-repair", 5),
    retired("execution-suite-relaunch", 5),
    retired("execution-plan-complete", 5),
    retired("execution-admit", 5),
    retired("execution-extend", 5),
    retired("execution-authorize", 5),
    retired("plan-submit", 4),
    retired("context-submit", 4),
    retired("review-admit", 4),
    retired("review-observation", 4),
    retired("review-return", 4),
    retired("review-material-append", 4),
    retired("review-enqueue", 4),
    retired("config-apply", 2),
    retired("config-interview-apply", 2),
    retired("risk-check", 4),
    retired("risk-fire", 4),
    retired("risk-consequence", 4),
    with_project("capture"),
    retired("milestone-release", 6),
    retired("milestone-release-confirm", 6),
    retired("milestone-close", 6),
    retired("milestone-prune", 6),
    retired("land-start", 6),
    retired("land-publish", 6),
    retired("land-authorize", 6),
    retired("land-open", 6),
    retired("land-merge", 6),
    retired("land-tag-push", 6),
    retired("land-resume", 6),
    retired("land-confirm-merge", 6),
    retired("land-checkout", 6),
    retired("land-pull", 6),
    retired("land-tag", 6),
    retired("land-reap", 6),
    retired("undo-phase", 6),
    retired("debug-open", 8),
    retired("debug-hypothesis", 8),
    retired("debug-observation", 8),
    retired("debug-attempt", 8),
    retired("debug-consult", 8),
    retired("debug-resolve", 8),
    retired("spike-open", 8),
    retired("spike-observation", 8),
    retired("spike-verdict", 8),
    retired("spike-close", 8),
    retired("task-open", 8),
    retired("task-close", 8),
];

/// A spelling served only from a project, which its call prepares.
const fn with_project(name: &'static str) -> Operation {
    Operation {
        name,
        status: Status::Available {
            needs_project: true,
        },
    }
}

/// What a request's `operation` comes to for one tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// A spelling this server serves.
    Available {
        /// Whether the operation needs a project.
        needs_project: bool,
    },
    /// A recognized spelling that now lives in a later build.
    Unavailable {
        /// The replacing build's number.
        build: u32,
    },
    /// No such spelling for this tool, or no string `operation` at all.
    Unknown,
}

/// The `operation` string of a request's arguments, if it has one.
pub fn spelling(arguments: Option<&Map<String, Value>>) -> Option<&str> {
    arguments?.get("operation")?.as_str()
}

/// Looks a spelling up under one tool.
pub fn lookup(tool: Tool, spelling: Option<&str>) -> Lookup {
    let Some(spelling) = spelling else {
        return Lookup::Unknown;
    };
    match tool.operations().iter().find(|op| op.name == spelling) {
        Some(Operation {
            status: Status::Available { needs_project },
            ..
        }) => Lookup::Available {
            needs_project: *needs_project,
        },
        Some(Operation {
            status: Status::Retired { build },
            ..
        }) => Lookup::Unavailable { build: *build },
        None => Lookup::Unknown,
    }
}

/// The refusal for a spelling the tool does not recognize. It lists the
/// recognized spellings so the caller corrects the spelling.
pub fn unknown_operation(tool: Tool, spelling: Option<&str>) -> Value {
    let known = tool
        .operations()
        .iter()
        .map(|op| op.name)
        .collect::<Vec<_>>()
        .join(", ");
    let reason = match spelling {
        Some(name) => format!(
            "no {} operation is named `{name}`; the operations are {known}",
            tool.name()
        ),
        None => format!(
            "{} needs an `operation` string; the operations are {known}",
            tool.name()
        ),
    };
    Refusal::new("unknown-operation", reason)
        .slot("operation")
        .value()
}

/// The refusal for a recognized spelling that a later build replaces.
pub fn operation_unavailable(tool: Tool, spelling: &str, build: u32) -> Value {
    Refusal::new(
        "operation-unavailable",
        format!(
            "{} operation `{spelling}` is not served by this server, Build {build} replaces it",
            tool.name()
        ),
    )
    .slot("operation")
    .details(serde_json::json!({ "build": build }))
    .value()
}

/// The request shapes of the operations this server serves. They live here,
/// not in the binary's service types, so the library owns their schema.
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "operation", deny_unknown_fields)]
enum RequestShape {
    #[serde(rename = "help")]
    Help {
        /// The command to show, with or without a leading slash or `bal-` prefix.
        name: Option<String>,
        /// One-based part for answers over 24,576 bytes; defaults to 1.
        /// Concatenate the returned bodies in order.
        part: Option<usize>,
    },
    #[serde(rename = "schema")]
    Schema {
        /// The tool whose operation is requested: apply or query.
        tool: String,
        #[serde(rename = "for")]
        operation: String,
        /// One-based part for schemas over 24,576 bytes; defaults to 1.
        /// Concatenate the returned bodies in order, then parse the JSON.
        part: Option<usize>,
    },
    #[serde(rename = "instruction")]
    Instruction {
        /// The identity of the instruction to read, such as `bal-help`.
        identity: String,
        /// One-based part for texts over 24,576 bytes; defaults to 1.
        /// Concatenate the returned bodies in order.
        part: Option<usize>,
    },
    // `capture` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "capture")]
    Capture(#[allow(dead_code)] crate::mcp::capture::CaptureShape),
}

fn invalid_arguments(reason: impl Into<String>) -> Value {
    Refusal::new("invalid-arguments", reason)
        .slot("arguments")
        .value()
}

/// Answers `help` for the optional `name`, in parts when large. An unknown
/// field is refused.
pub fn help_answer(arguments: &Value) -> Value {
    match serde_json::from_value::<RequestShape>(arguments.clone()) {
        Ok(RequestShape::Help { name, part }) => {
            help_part(crate::help::table::answer(name.as_deref()), part)
        }
        Ok(_) => invalid_arguments("these are not help arguments"),
        Err(error) => invalid_arguments(error.to_string()),
    }
}

/// The help answer whole when it fits, or one numbered part of its JSON text.
fn help_part(answer: Value, part: Option<usize>) -> Value {
    let serialized = serde_json::to_string(&answer).expect("help answer JSON");
    match cut(&serialized, part) {
        Cut::Whole(_) => answer,
        Cut::Part { body, part, next } => serde_json::json!({"status":"ok",
        "bound":PART_BOUND,"part":part,"body":body,"next":next}),
        Cut::Absent => Refusal::new("help-part-not-found", "the requested help part is absent")
            .slot("part")
            .value(),
    }
}

/// Answers `instruction`: the compiled text of one served identity, in parts
/// when large. It needs no project and reads only the registry, and the version
/// and hash it serves are the registry's pinned values.
pub fn instruction_answer(arguments: &Value) -> Value {
    let (identity, part) = match serde_json::from_value::<RequestShape>(arguments.clone()) {
        Ok(RequestShape::Instruction { identity, part }) => (identity, part),
        Ok(_) => return invalid_arguments("these are not instruction arguments"),
        Err(error) => return invalid_arguments(error.to_string()),
    };
    match instruction::lookup(&identity) {
        instruction::Lookup::Served { entry, text } => {
            instruction_part(entry.identity, text.version, text.hash, text.body, part)
        }
        instruction::Lookup::Unavailable { identity, build } => {
            instruction::unavailable(identity, build)
        }
        instruction::Lookup::Unknown => instruction::unknown(),
    }
}

/// One instruction whole when it fits, or one numbered part of its text. Every
/// part carries the evidence of the whole text, since the hash covers all parts.
fn instruction_part(
    identity: &str,
    version: &str,
    hash: &str,
    body: &str,
    part: Option<usize>,
) -> Value {
    match cut(body, part) {
        Cut::Whole(body) => serde_json::json!({"status":"ok","identity":identity,
        "version":version,"hash":hash,"text":body}),
        Cut::Part { body, part, next } => serde_json::json!({"status":"ok",
        "identity":identity,"version":version,"hash":hash,
        "bound":PART_BOUND,"part":part,"body":body,"next":next}),
        Cut::Absent => Refusal::new(
            "instruction-part-not-found",
            "the requested instruction part is absent",
        )
        .slot("part")
        .value(),
    }
}

/// Answers `schema`: the request shape of one operation, in parts when large.
pub fn schema_answer(arguments: &Value) -> Value {
    let (tool, operation, part) = match serde_json::from_value::<RequestShape>(arguments.clone()) {
        Ok(RequestShape::Schema {
            tool,
            operation,
            part,
        }) => (tool, operation, part),
        Ok(_) => return invalid_arguments("these are not schema arguments"),
        Err(error) => return invalid_arguments(error.to_string()),
    };
    let which = match tool.as_str() {
        "apply" => Tool::Apply,
        "query" => Tool::Query,
        _ => {
            return Refusal::new("unknown-tool", "schema tool must be apply or query")
                .slot("tool")
                .value();
        }
    };
    match lookup(which, Some(&operation)) {
        Lookup::Unavailable { build } => operation_unavailable(which, &operation, build),
        Lookup::Unknown => Refusal::new(
            "unknown-operation",
            format!("no baley_{tool} operation is named `{operation}`"),
        )
        .slot("for")
        .value(),
        Lookup::Available { .. } => match served_schema(&operation) {
            Some(schema) => schema_part(&tool, &operation, &schema, part),
            None => unknown_operation(which, Some(&operation)),
        },
    }
}

/// The schema of one served operation, from the library's request shapes.
fn served_schema(operation: &str) -> Option<Value> {
    let root = serde_json::to_value(schemars::schema_for!(RequestShape)).ok()?;
    root["oneOf"]
        .as_array()?
        .iter()
        .find(|variant| variant["properties"]["operation"]["const"] == operation)
        .cloned()
}

fn schema_part(tool: &str, operation: &str, schema: &Value, part: Option<usize>) -> Value {
    let serialized = serde_json::to_string(schema).expect("operation schema JSON");
    match cut(&serialized, part) {
        Cut::Whole(_) => {
            serde_json::json!({"status":"ok","tool":tool,"operation":operation,"schema":schema})
        }
        Cut::Part { body, part, next } => {
            serde_json::json!({"status":"ok","tool":tool,"operation":operation,
            "bound":PART_BOUND,"part":part,"body":body,"next":next})
        }
        Cut::Absent => Refusal::new(
            "schema-part-not-found",
            "the requested schema part is absent",
        )
        .slot("part")
        .value(),
    }
}

/// Today's spellings written out again, independent of the data above, for the
/// tests that hold the baseline and the advertised enums to them.
#[cfg(test)]
pub(crate) mod expected {
    /// `baley_query` spellings in order.
    pub const QUERY: &[&str] = &[
        "help",
        "recall",
        "debug-list",
        "debug-status",
        "debug-continue",
        "undo-read",
        "milestone-read",
        "land-read",
        "progress",
        "suggest",
        "why",
        "document",
        "document-search",
        "verify-next",
        "verification-read",
        "verification-audit",
        "execution-history",
        "evidence-read",
        "plan-read",
        "context-intake",
        "route",
        "config-entry",
        "config-facts",
        "config-interview",
        "detect-surfaces",
        "execute-next",
        "risk-status",
        "schema",
        "review-next",
        "review-admission",
        "review-material",
        "review-original",
        "review-attempt",
        "review-roster",
        "review-inventory",
        "review-deferred",
        "review-consumer",
        "review-select",
        "instruction",
    ];

    /// `baley_apply` spellings in order.
    pub const APPLY: &[&str] = &[
        "verification-run",
        "verification-submit",
        "truth-waive",
        "verification-human-result",
        "verification-complete",
        "execution-task-retire",
        "execution-task-progress",
        "execution-task-checkpoint",
        "execution-task-answer",
        "execution-task-close",
        "execution-classify-run",
        "execution-owner-attest",
        "execution-task-start",
        "execution-run",
        "execution-worker-exit",
        "execution-round-record",
        "execution-suite",
        "execution-suite-repair-answer",
        "execution-suite-repair",
        "execution-suite-relaunch",
        "execution-plan-complete",
        "execution-admit",
        "execution-extend",
        "execution-authorize",
        "plan-submit",
        "context-submit",
        "review-admit",
        "review-observation",
        "review-return",
        "review-material-append",
        "review-enqueue",
        "config-apply",
        "config-interview-apply",
        "risk-check",
        "risk-fire",
        "risk-consequence",
        "capture",
        "milestone-release",
        "milestone-release-confirm",
        "milestone-close",
        "milestone-prune",
        "land-start",
        "land-publish",
        "land-authorize",
        "land-open",
        "land-merge",
        "land-tag-push",
        "land-resume",
        "land-confirm-merge",
        "land-checkout",
        "land-pull",
        "land-tag",
        "land-reap",
        "undo-phase",
        "debug-open",
        "debug-hypothesis",
        "debug-observation",
        "debug-attempt",
        "debug-consult",
        "debug-resolve",
        "spike-open",
        "spike-observation",
        "spike-verdict",
        "spike-close",
        "task-open",
        "task-close",
    ];
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::expected::{APPLY, QUERY};
    use super::*;

    fn names(tool: Tool) -> Vec<&'static str> {
        tool.operations().iter().map(|op| op.name).collect()
    }

    #[test]
    fn the_baseline_lists_keep_todays_order_with_instruction_appended_last() {
        assert_eq!(names(Tool::Query), QUERY);
        assert_eq!(names(Tool::Apply), APPLY);
    }

    #[test]
    fn no_spelling_appears_twice_within_a_tool() {
        for tool in [Tool::Query, Tool::Apply] {
            let mut seen = std::collections::BTreeSet::new();
            for name in names(tool) {
                assert!(seen.insert(name), "{name} is listed twice");
            }
        }
    }

    #[test]
    fn help_schema_and_instruction_are_available_and_need_no_project() {
        for name in ["help", "schema", "instruction"] {
            assert_eq!(
                lookup(Tool::Query, Some(name)),
                Lookup::Available {
                    needs_project: false
                }
            );
        }
    }

    #[test]
    fn every_spelling_but_the_three_reads_is_unavailable() {
        for tool in [Tool::Query, Tool::Apply] {
            for name in names(tool)
                .into_iter()
                .filter(|n| !["help", "schema", "instruction", "capture"].contains(n))
            {
                assert!(
                    matches!(lookup(tool, Some(name)), Lookup::Unavailable { .. }),
                    "{name} must be unavailable"
                );
            }
        }
        assert_eq!(
            lookup(Tool::Apply, Some("capture")),
            Lookup::Available {
                needs_project: true
            }
        );
    }

    #[test]
    fn each_family_names_its_replacing_build() {
        let build = |tool, name| match lookup(tool, Some(name)) {
            Lookup::Unavailable { build } => build,
            other => panic!("{name} is {other:?}"),
        };
        assert_eq!(build(Tool::Query, "document"), 3);
        assert_eq!(build(Tool::Query, "config-entry"), 2);
        assert_eq!(build(Tool::Apply, "config-apply"), 2);
        assert_eq!(build(Tool::Query, "plan-read"), 4);
        assert_eq!(build(Tool::Apply, "review-admit"), 4);
        assert_eq!(build(Tool::Apply, "risk-fire"), 4);
        assert_eq!(build(Tool::Query, "route"), 4);
        assert_eq!(build(Tool::Query, "detect-surfaces"), 4);
        assert_eq!(build(Tool::Apply, "execution-run"), 5);
        assert_eq!(build(Tool::Query, "verify-next"), 5);
        assert_eq!(build(Tool::Apply, "truth-waive"), 5);
        assert_eq!(build(Tool::Query, "evidence-read"), 5);
        assert_eq!(build(Tool::Query, "land-read"), 6);
        assert_eq!(build(Tool::Apply, "undo-phase"), 6);
        assert_eq!(build(Tool::Apply, "milestone-close"), 6);
        assert_eq!(build(Tool::Query, "progress"), 7);
        assert_eq!(build(Tool::Query, "suggest"), 7);
        assert_eq!(build(Tool::Query, "recall"), 8);
        assert_eq!(build(Tool::Query, "document-search"), 8);
        assert_eq!(build(Tool::Query, "why"), 8);
        assert_eq!(build(Tool::Apply, "task-open"), 8);
        assert_eq!(build(Tool::Apply, "spike-close"), 8);
    }

    #[test]
    fn an_unrecognized_or_missing_or_other_tools_spelling_is_unknown() {
        assert_eq!(lookup(Tool::Query, Some("no-such-thing")), Lookup::Unknown);
        assert_eq!(lookup(Tool::Query, Some("Help")), Lookup::Unknown);
        assert_eq!(lookup(Tool::Query, None), Lookup::Unknown);
        assert_eq!(lookup(Tool::Query, Some("execution-run")), Lookup::Unknown);
        assert_eq!(lookup(Tool::Apply, Some("help")), Lookup::Unknown);
    }

    #[test]
    fn only_a_string_operation_counts_as_a_spelling() {
        let object = |value: Value| value.as_object().unwrap().clone();
        assert_eq!(
            spelling(Some(&object(json!({"operation": "help"})))),
            Some("help")
        );
        assert_eq!(spelling(Some(&object(json!({"operation": 7})))), None);
        assert_eq!(spelling(Some(&object(json!({})))), None);
        assert_eq!(spelling(None), None);
    }

    #[test]
    fn unknown_operation_is_a_refusal_in_slot_operation_naming_the_recognized_spellings() {
        let refusal = unknown_operation(Tool::Apply, Some("nope"));
        assert_eq!(refusal["status"], "refused");
        assert_eq!(refusal["code"], "unknown-operation");
        assert_eq!(refusal["slot"], "operation");
        let reason = refusal["reason"].as_str().unwrap();
        assert!(reason.contains("`nope`") && reason.contains("execution-run"));
        assert!(!reason.contains("help,"), "apply has no help");
    }

    #[test]
    fn operation_unavailable_names_the_build_in_words_and_as_an_integer() {
        let refusal = operation_unavailable(Tool::Query, "document", 3);
        assert_eq!(refusal["status"], "refused");
        assert_eq!(refusal["code"], "operation-unavailable");
        assert_eq!(refusal["slot"], "operation");
        assert_eq!(refusal["details"], json!({"build": 3}));
        assert!(refusal["reason"].as_str().unwrap().contains("Build 3"));
    }
    #[test]
    fn help_answers_what_the_help_table_answers_for_the_same_name() {
        assert_eq!(
            help_answer(&json!({"operation": "help"})),
            crate::help::table::answer(None)
        );
        assert_eq!(
            help_answer(&json!({"operation": "help", "name": "baley_query"})),
            crate::help::table::answer(Some("baley_query"))
        );
    }

    #[test]
    fn help_part_1_is_the_whole_answer_and_part_2_of_a_fitting_answer_is_refused() {
        let first = help_answer(&json!({"operation": "help", "part": 1}));
        // The table's own answer, so a fitting answer wrapped as a part cannot equal it.
        assert_eq!(first, crate::help::table::answer(None));
        let second = help_answer(&json!({"operation": "help", "part": 2}));
        assert_eq!(second["code"], "help-part-not-found", "{second}");
        assert_eq!(second["slot"], "part");
        assert_eq!(second["status"], "refused");
    }

    #[test]
    fn help_with_an_unknown_field_is_refused_in_slot_arguments() {
        let answer = help_answer(&json!({"operation": "help", "bogus": 1}));
        assert_eq!(answer["status"], "refused");
        assert_eq!(answer["code"], "invalid-arguments");
        assert_eq!(answer["slot"], "arguments");
    }

    #[test]
    fn schema_for_the_three_served_operations_is_an_ok_answer() {
        for operation in ["help", "schema", "instruction"] {
            let answer =
                schema_answer(&json!({"operation": "schema", "tool": "query", "for": operation}));
            assert_eq!(answer["status"], "ok", "{operation}: {answer}");
            assert_eq!(answer["tool"], "query");
            assert_eq!(answer["operation"], operation);
            assert_eq!(
                answer["schema"]["properties"]["operation"]["const"],
                operation
            );
        }
    }

    #[test]
    fn a_capture_schema_left_out_of_the_request_shapes_or_served_on_query_is_caught() {
        let answer =
            schema_answer(&json!({"operation": "schema", "tool": "apply", "for": "capture"}));
        assert_eq!(answer["status"], "ok", "{answer}");
        assert_eq!(answer["tool"], "apply");
        assert_eq!(
            answer["schema"]["properties"]["operation"]["const"],
            "capture"
        );
        for property in ["request_id", "kind", "text", "phase", "instruction"] {
            assert!(
                answer["schema"]["properties"].get(property).is_some(),
                "{property} missing from {answer}"
            );
        }
        let query =
            schema_answer(&json!({"operation": "schema", "tool": "query", "for": "capture"}));
        assert_eq!(query["code"], "unknown-operation", "{query}");
    }

    fn sha256_hex(text: &str) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn instruction_call(identity: &str) -> Value {
        instruction_answer(&json!({"operation": "instruction", "identity": identity}))
    }

    #[test]
    fn an_instruction_answers_its_registry_evidence_and_a_text_that_hashes_to_it() {
        for (identity, version, hash) in [
            (
                "bal-help",
                crate::help::front_door::VERSION,
                crate::help::front_door::HASH,
            ),
            (
                "bal-read-contract",
                crate::instruction::read_contract::VERSION,
                crate::instruction::read_contract::HASH,
            ),
        ] {
            let answer = instruction_call(identity);
            assert_eq!(answer["status"], "ok", "{identity}: {answer}");
            assert_eq!(answer["identity"], identity);
            assert_eq!(answer["version"], version);
            assert_eq!(answer["hash"], hash);
            let text = answer["text"].as_str().expect("a whole text");
            assert_eq!(
                sha256_hex(text),
                hash,
                "{identity} text must match its hash"
            );
        }
    }

    #[test]
    fn an_unavailable_instruction_is_refused_naming_its_build() {
        let answer = instruction_call("bal-plan");
        assert_eq!(answer["code"], "instruction-unavailable");
        assert_eq!(answer["slot"], "identity");
        assert_eq!(answer["details"], json!({"build": 4}));
    }

    #[test]
    fn an_unknown_instruction_is_refused_in_slot_identity() {
        let answer = instruction_call("no-such-instruction");
        assert_eq!(answer["code"], "unknown-instruction");
        assert_eq!(answer["slot"], "identity");
    }

    #[test]
    fn part_2_of_a_one_part_instruction_is_refused_in_slot_part() {
        let answer = instruction_answer(
            &json!({"operation": "instruction", "identity": "bal-help", "part": 2}),
        );
        assert_eq!(answer["code"], "instruction-part-not-found");
        assert_eq!(answer["slot"], "part");
    }

    #[test]
    fn part_1_of_a_one_part_instruction_is_the_whole_answer() {
        let whole = instruction_call("bal-help");
        let first = instruction_answer(
            &json!({"operation": "instruction", "identity": "bal-help", "part": 1}),
        );
        assert_eq!(first, whole);
    }

    /// Fetches part 1 with no part named, then follows `next` to the end,
    /// checking the fields every paged answer carries. Returns each part.
    fn every_part(fetch: impl Fn(Option<usize>) -> Value) -> Vec<Value> {
        let mut parts = vec![fetch(None)];
        loop {
            let answer = parts.last().unwrap();
            let number = parts.len();
            assert_eq!(answer["status"], "ok", "{answer}");
            assert_eq!(answer["bound"], PART_BOUND);
            assert_eq!(answer["part"], number);
            let body = answer["body"].as_str().expect("a part body");
            assert!(!body.is_empty() && body.len() <= PART_BOUND);
            if answer["next"].is_null() {
                break;
            }
            assert_eq!(answer["next"], number + 1);
            parts.push(fetch(Some(number + 1)));
        }
        assert!(parts.len() > 1, "the answer must be paged");
        parts
    }

    fn joined(parts: &[Value]) -> String {
        parts
            .iter()
            .map(|part| part["body"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_schema_part_that_drops_its_wire_fields_or_loses_bytes_is_caught() {
        let schema = json!({"description":"é🦀\"\\".repeat(PART_BOUND)});
        let parts = every_part(|part| schema_part("apply", "synthetic", &schema, part));
        for part in &parts {
            assert_eq!(part["tool"], "apply");
            assert_eq!(part["operation"], "synthetic");
            assert!(
                part.get("schema").is_none(),
                "a part carries no parsed schema"
            );
        }
        let combined = joined(&parts);
        assert_eq!(combined, serde_json::to_string(&schema).unwrap());
        assert_eq!(serde_json::from_str::<Value>(&combined).unwrap(), schema);
    }

    #[test]
    fn a_schema_part_of_0_past_the_end_or_usize_max_served_instead_of_refused_is_caught() {
        // Quoted, these serialize to exactly the bound and two bytes over it.
        let fitting = json!("x".repeat(PART_BOUND - 2));
        let paged = json!("x".repeat(PART_BOUND));
        for (schema, past_end) in [(&fitting, 2), (&paged, 3)] {
            for part in [0, past_end, usize::MAX] {
                let refused = schema_part("query", "synthetic", schema, Some(part));
                assert_eq!(refused["status"], "refused", "part {part}");
                assert_eq!(refused["slot"], "part");
                assert_eq!(refused["code"], "schema-part-not-found");
            }
        }
    }

    #[test]
    fn a_help_part_that_drops_its_wire_fields_or_serves_a_missing_part_is_caught() {
        let answer = json!({"status":"ok","about":"é🦀".repeat(PART_BOUND)});
        let parts = every_part(|part| help_part(answer.clone(), part));
        for part in &parts {
            assert!(
                part.get("about").is_none(),
                "a part carries no answer field"
            );
        }
        let combined = joined(&parts);
        assert_eq!(serde_json::from_str::<Value>(&combined).unwrap(), answer);
        let past = help_part(answer, Some(parts.len() + 1));
        assert_eq!(past["code"], "help-part-not-found");
        assert_eq!(past["slot"], "part");
    }

    #[test]
    fn an_instruction_part_that_drops_its_evidence_or_serves_a_missing_part_is_caught() {
        let body = "é🦀".repeat(PART_BOUND);
        let parts = every_part(|part| instruction_part("bal-synthetic", "7", "abc", &body, part));
        for part in &parts {
            assert_eq!(part["identity"], "bal-synthetic");
            assert_eq!(part["version"], "7");
            assert_eq!(part["hash"], "abc");
            assert!(part.get("text").is_none(), "a part carries no whole text");
        }
        assert_eq!(joined(&parts), body);
        let past = instruction_part("bal-synthetic", "7", "abc", &body, Some(parts.len() + 1));
        assert_eq!(past["code"], "instruction-part-not-found");
        assert_eq!(past["slot"], "part");
    }

    #[test]
    fn an_instruction_with_a_version_field_is_refused_in_slot_arguments() {
        let answer = instruction_answer(
            &json!({"operation": "instruction", "identity": "bal-help", "version": "1"}),
        );
        assert_eq!(answer["code"], "invalid-arguments");
        assert_eq!(answer["slot"], "arguments");
    }

    #[test]
    fn schema_for_a_retired_operation_names_its_build() {
        let answer =
            schema_answer(&json!({"operation": "schema", "tool": "apply", "for": "execution-run"}));
        assert_eq!(answer["code"], "operation-unavailable");
        assert_eq!(answer["details"], json!({"build": 5}));
    }

    #[test]
    fn schema_for_an_unrecognized_operation_is_unknown_operation_in_slot_for() {
        let answer = schema_answer(&json!({"operation": "schema", "tool": "query", "for": "nope"}));
        assert_eq!(answer["code"], "unknown-operation");
        assert_eq!(answer["slot"], "for");
    }

    #[test]
    fn schema_for_another_tool_is_unknown_tool_in_slot_tool() {
        let answer = schema_answer(&json!({"operation": "schema", "tool": "other", "for": "help"}));
        assert_eq!(answer["code"], "unknown-tool");
        assert_eq!(answer["slot"], "tool");
    }
}
