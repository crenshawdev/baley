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
        /// Whether it needs a project, as `capture` and `document` do.
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
    with_project("document"),
    retired("document-search", 8),
    retired("verify-next", 5),
    retired("verification-read", 5),
    retired("verification-audit", 5),
    retired("execution-history", 5),
    retired("evidence-read", 5),
    retired("plan-read", 4),
    with_project("context-intake"),
    with_project("route"),
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
    with_project("context-submit"),
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
    with_project("scope-submit"),
    with_project("project-describe"),
    with_project("phase-declare"),
    with_project("phase-edit"),
    with_project("phase-reorder"),
    with_project("phase-withdraw"),
    with_project("story-declare"),
    with_project("story-edit"),
    with_project("story-reassign"),
    with_project("story-drop"),
    with_project("backlog-reorder"),
    with_project("project-start"),
    with_project("work-order-fail"),
    with_project("work-order-retry"),
    with_project("questions-answer"),
    with_project("story-refine"),
    with_project("story-questions-submit"),
    with_project("phase-plan"),
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
    // `document` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "document")]
    Document(#[allow(dead_code)] crate::mcp::document::DocumentShape),
    // `scope-submit` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "scope-submit")]
    ScopeSubmit(#[allow(dead_code)] crate::mcp::scope::SubmitShape),
    // Each change parses its own arguments, so these variants are here for
    // their schemas.
    #[serde(rename = "project-describe")]
    ProjectDescribe(#[allow(dead_code)] crate::mcp::scope::ProjectDescribeShape),
    #[serde(rename = "phase-declare")]
    PhaseDeclare(#[allow(dead_code)] crate::mcp::scope::PhaseDeclareShape),
    #[serde(rename = "phase-edit")]
    PhaseEdit(#[allow(dead_code)] crate::mcp::scope::PhaseEditShape),
    #[serde(rename = "phase-reorder")]
    PhaseReorder(#[allow(dead_code)] crate::mcp::scope::PhaseReorderShape),
    #[serde(rename = "phase-withdraw")]
    PhaseWithdraw(#[allow(dead_code)] crate::mcp::scope::PhaseWithdrawShape),
    #[serde(rename = "story-declare")]
    StoryDeclare(#[allow(dead_code)] crate::mcp::scope::StoryDeclareShape),
    #[serde(rename = "story-edit")]
    StoryEdit(#[allow(dead_code)] crate::mcp::scope::StoryEditShape),
    #[serde(rename = "story-reassign")]
    StoryReassign(#[allow(dead_code)] crate::mcp::scope::StoryReassignShape),
    #[serde(rename = "story-drop")]
    StoryDrop(#[allow(dead_code)] crate::mcp::scope::StoryDropShape),
    #[serde(rename = "backlog-reorder")]
    BacklogReorder(#[allow(dead_code)] crate::mcp::scope::BacklogReorderShape),
    // `project-start` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "project-start")]
    ProjectStart(#[allow(dead_code)] crate::mcp::work_order::StartShape),
    // `work-order-fail` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "work-order-fail")]
    WorkOrderFail(#[allow(dead_code)] crate::mcp::work_order::FailShape),
    // `work-order-retry` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "work-order-retry")]
    WorkOrderRetry(#[allow(dead_code)] crate::mcp::work_order::RetryShape),
    // `questions-answer` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "questions-answer")]
    QuestionsAnswer(#[allow(dead_code)] crate::mcp::questions::AnswerShape),
    // `story-refine` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "story-refine")]
    StoryRefine(#[allow(dead_code)] crate::mcp::refine::RefineShape),
    // `context-submit` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "context-submit")]
    ContextSubmit(#[allow(dead_code)] crate::mcp::refine::ContextShape),
    // `story-questions-submit` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "story-questions-submit")]
    StoryQuestionsSubmit(#[allow(dead_code)] crate::mcp::refine::QuestionsShape),
    // `phase-plan` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "phase-plan")]
    PhasePlan(#[allow(dead_code)] crate::mcp::planning::PhasePlanShape),
    // `context-intake` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "context-intake")]
    ContextIntake(#[allow(dead_code)] crate::mcp::refine::IntakeShape),
    // `route` parses its own arguments, so this variant is here for its schema.
    #[serde(rename = "route")]
    Route(#[allow(dead_code)] crate::mcp::work_order::RouteShape),
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

/// One served operation's schema from the request shapes; the tool input schemas use it too.
pub(crate) fn served_schema(operation: &str) -> Option<Value> {
    // The selected variant must not refer to definitions left on the root.
    let settings = schemars::generate::SchemaSettings::default()
        .with(|settings| settings.inline_subschemas = true);
    let root = serde_json::to_value(
        settings
            .into_generator()
            .into_root_schema_for::<RequestShape>(),
    )
    .ok()?;
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
        "scope-submit",
        "project-describe",
        "phase-declare",
        "phase-edit",
        "phase-reorder",
        "phase-withdraw",
        "story-declare",
        "story-edit",
        "story-reassign",
        "story-drop",
        "backlog-reorder",
        "project-start",
        "work-order-fail",
        "work-order-retry",
        "questions-answer",
        "story-refine",
        "story-questions-submit",
        "phase-plan",
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
            for name in names(tool).into_iter().filter(|n| {
                ![
                    "help",
                    "schema",
                    "instruction",
                    "capture",
                    "document",
                    "scope-submit",
                    "project-describe",
                    "phase-declare",
                    "phase-edit",
                    "phase-reorder",
                    "phase-withdraw",
                    "story-declare",
                    "story-edit",
                    "story-reassign",
                    "story-drop",
                    "backlog-reorder",
                    "project-start",
                    "work-order-fail",
                    "work-order-retry",
                    "questions-answer",
                    "story-refine",
                    "story-questions-submit",
                    "phase-plan",
                    "context-submit",
                    "context-intake",
                    "route",
                ]
                .contains(n)
            }) {
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
        for name in ["document", "context-intake", "route"] {
            assert_eq!(
                lookup(Tool::Query, Some(name)),
                Lookup::Available {
                    needs_project: true
                },
                "{name}"
            );
        }
        for name in [
            "scope-submit",
            "project-describe",
            "phase-declare",
            "phase-edit",
            "phase-reorder",
            "phase-withdraw",
            "story-declare",
            "story-edit",
            "story-reassign",
            "story-drop",
            "backlog-reorder",
            "project-start",
            "work-order-fail",
            "work-order-retry",
            "questions-answer",
            "story-refine",
            "story-questions-submit",
            "phase-plan",
            "context-submit",
        ] {
            assert_eq!(
                lookup(Tool::Apply, Some(name)),
                Lookup::Available {
                    needs_project: true
                },
                "{name}"
            );
        }
    }

    #[test]
    fn each_family_names_its_replacing_build() {
        let build = |tool, name| match lookup(tool, Some(name)) {
            Lookup::Unavailable { build } => build,
            other => panic!("{name} is {other:?}"),
        };
        assert_eq!(build(Tool::Query, "config-entry"), 2);
        assert_eq!(build(Tool::Apply, "config-apply"), 2);
        assert_eq!(build(Tool::Query, "plan-read"), 4);
        assert_eq!(build(Tool::Apply, "review-admit"), 4);
        assert_eq!(build(Tool::Apply, "risk-fire"), 4);
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
        let refusal = operation_unavailable(Tool::Query, "document-search", 8);
        assert_eq!(refusal["status"], "refused");
        assert_eq!(refusal["code"], "operation-unavailable");
        assert_eq!(refusal["slot"], "operation");
        assert_eq!(refusal["details"], json!({"build": 8}));
        assert!(refusal["reason"].as_str().unwrap().contains("Build 8"));
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

    #[test]
    fn a_document_schema_left_out_of_the_request_shapes_is_caught() {
        let answer =
            schema_answer(&json!({"operation": "schema", "tool": "query", "for": "document"}));
        assert_eq!(answer["status"], "ok", "{answer}");
        assert_eq!(answer["tool"], "query");
        assert_eq!(
            answer["schema"]["properties"]["operation"]["const"],
            "document"
        );
        for property in ["identity", "part"] {
            assert!(
                answer["schema"]["properties"].get(property).is_some(),
                "{property} missing from {answer}"
            );
        }
    }

    #[test]
    fn a_scope_submit_schema_left_out_of_the_request_shapes_is_caught() {
        let answer =
            schema_answer(&json!({"operation": "schema", "tool": "apply", "for": "scope-submit"}));
        assert_eq!(answer["status"], "ok", "{answer}");
        let mut properties: Vec<&str> = answer["schema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .filter(|name| *name != "operation")
            .collect();
        properties.sort_unstable();
        assert_eq!(properties, ["request_id", "submission", "work_order"]);
    }

    #[test]
    fn a_change_operation_left_out_of_the_request_shapes_is_caught() {
        // Design 0012: each change takes a request_id and a submission.
        for operation in [
            "project-describe",
            "phase-declare",
            "phase-edit",
            "phase-reorder",
            "phase-withdraw",
            "story-declare",
            "story-edit",
            "story-reassign",
            "story-drop",
            "backlog-reorder",
        ] {
            let answer =
                schema_answer(&json!({"operation": "schema", "tool": "apply", "for": operation}));
            assert_eq!(answer["status"], "ok", "{operation}: {answer}");
            let mut properties: Vec<&str> = answer["schema"]["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .filter(|name| *name != "operation")
                .collect();
            properties.sort_unstable();
            assert_eq!(properties, ["request_id", "submission"], "{operation}");
        }
    }

    #[test]
    fn no_served_operation_schema_refers_to_a_definition_it_drops() {
        fn check(schema: &Value) {
            match schema {
                Value::Object(fields) => {
                    assert!(
                        !fields.contains_key("$ref"),
                        "unresolved reference: {schema}"
                    );
                    for value in fields.values() {
                        check(value);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        check(value);
                    }
                }
                _ => {}
            }
        }

        for tool in [Tool::Query, Tool::Apply] {
            for operation in tool.operations() {
                if matches!(operation.status, Status::Available { .. }) {
                    let schema = served_schema(operation.name).expect(operation.name);
                    check(&schema);
                }
            }
        }
    }

    #[test]
    fn the_document_schema_does_not_omit_or_widen_the_capture_identity() {
        // Design 0012: a capture by its string id, a draft by its string
        // digest, the roadmap with no other field, a story by its integer id,
        // a work order by its string id, a question set by its string set id
        // and a refinement by its integer story id, each refusing any other
        // field.
        let schema = served_schema("document").unwrap();
        let identities = schema["properties"]["identity"]["oneOf"]
            .as_array()
            .unwrap();
        assert_eq!(identities.len(), 7, "{identities:?}");
        for (identity, kind, field) in [
            (&identities[0], "capture", Some(("id", "string"))),
            (&identities[1], "draft", Some(("digest", "string"))),
            (&identities[2], "roadmap", None),
            (&identities[3], "story", Some(("id", "integer"))),
            (&identities[4], "work_order", Some(("id", "string"))),
            (&identities[5], "questions", Some(("set", "string"))),
            (&identities[6], "refinement", Some(("story", "integer"))),
        ] {
            assert_eq!(identity["type"], "object", "{identity}");
            assert_eq!(identity["properties"]["kind"]["const"], kind, "{identity}");
            assert_eq!(identity["additionalProperties"], false, "{identity}");
            let mut required: Vec<&str> = identity["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap())
                .collect();
            required.sort_unstable();
            let properties = identity["properties"].as_object().unwrap();
            match field {
                Some((name, kind)) => {
                    assert_eq!(identity["properties"][name]["type"], kind, "{identity}");
                    let mut expected = vec!["kind", name];
                    expected.sort_unstable();
                    assert_eq!(required, expected, "{identity}");
                    assert_eq!(properties.len(), 2, "{identity}");
                }
                None => {
                    assert_eq!(required, ["kind"], "{identity}");
                    assert_eq!(properties.len(), 1, "{identity}");
                }
            }
        }
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
            (
                "bal-capture",
                crate::instruction::capture::VERSION,
                crate::instruction::capture::HASH,
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

    #[test]
    fn a_project_start_spelling_left_unserved_or_out_of_the_baseline_order_is_caught() {
        assert_eq!(
            lookup(Tool::Apply, Some("project-start")),
            Lookup::Available {
                needs_project: true
            }
        );
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        let at = names.iter().position(|name| *name == "work-order-fail");
        assert_eq!(
            at.and_then(|at| at.checked_sub(1))
                .map(|before| names[before]),
            Some("project-start")
        );
        let served =
            schema_answer(&json!({"operation": "schema", "tool": "apply", "for": "project-start"}));
        assert_eq!(served["status"], "ok", "{served}");
        let required = served["schema"]["required"].as_array().unwrap();
        for name in ["request_id", "brief"] {
            assert!(required.contains(&json!(name)), "{name} in {served}");
        }
        assert!(!required.contains(&json!("instruction")), "{served}");
    }

    #[test]
    fn the_work_order_operations_out_of_the_baseline_order_or_unserved_are_caught() {
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        let served = ["project-start", "work-order-fail", "work-order-retry"];
        // The three follow `backlog-reorder` and directly precede
        // `questions-answer`, appended after them.
        let at = names
            .iter()
            .position(|name| *name == "project-start")
            .expect("listed");
        assert_eq!(names[at - 1], "backlog-reorder");
        assert_eq!(names[at..at + 3], served);
        assert_eq!(names[at + 3], "questions-answer");
        for name in served {
            assert_eq!(
                lookup(Tool::Apply, Some(name)),
                Lookup::Available {
                    needs_project: true
                },
                "{name}"
            );
        }
        for (name, required) in [
            (
                "work-order-fail",
                &["request_id", "work_order", "reason"][..],
            ),
            ("work-order-retry", &["request_id", "work_order"][..]),
        ] {
            let answer =
                schema_answer(&json!({"operation": "schema", "tool": "apply", "for": name}));
            assert_eq!(answer["status"], "ok", "{answer}");
            let listed = answer["schema"]["required"].as_array().unwrap();
            for field in required {
                assert!(listed.contains(&json!(field)), "{field} in {answer}");
            }
            assert!(!listed.contains(&json!("instruction")), "{answer}");
        }
    }

    #[test]
    fn a_questions_answer_spelling_left_unserved_or_out_of_the_baseline_order_is_caught() {
        let names: Vec<&str> = APPLY_OPERATIONS.iter().map(|op| op.name).collect();
        let at = names
            .iter()
            .position(|name| *name == "questions-answer")
            .expect("listed");
        assert_eq!(names[at - 1], "work-order-retry");
        assert_eq!(names[at + 1], "story-refine");
        assert_eq!(
            lookup(Tool::Apply, Some("questions-answer")),
            Lookup::Available {
                needs_project: true
            }
        );
        let answer = schema_answer(
            &json!({"operation": "schema", "tool": "apply", "for": "questions-answer"}),
        );
        assert_eq!(answer["status"], "ok", "{answer}");
        let listed = answer["schema"]["required"].as_array().unwrap();
        for field in ["request_id", "set", "items"] {
            assert!(listed.contains(&json!(field)), "{field} in {answer}");
        }
        assert!(
            answer["schema"]["properties"].get("instruction").is_none(),
            "{answer}"
        );
    }

    #[test]
    fn a_route_query_left_retired_or_moved_in_the_baseline_is_caught() {
        assert_eq!(
            lookup(Tool::Query, Some("route")),
            Lookup::Available {
                needs_project: true
            }
        );
        let names: Vec<&str> = QUERY_OPERATIONS.iter().map(|op| op.name).collect();
        let at = names.iter().position(|name| *name == "route").unwrap();
        assert_eq!(names[at - 1], "context-intake");
        assert_eq!(names[at + 1], "config-entry");
        assert_eq!(
            expected::QUERY.iter().position(|name| *name == "route"),
            Some(at)
        );
    }
}
