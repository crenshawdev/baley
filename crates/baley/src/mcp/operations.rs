//! The operation baseline: today's spellings per tool, in today's order, and
//! the build that replaces each one Baley no longer serves.
//!
//! The lists are literal data on purpose. They are not derived from the
//! binary's service types, which a later build parks and deletes, and names
//! are only ever added to them. An operation that is not served answers
//! `operation-unavailable` naming its replacing build, so a caller that knows
//! the old spelling learns where it went.

use serde_json::{Map, Value};

use crate::envelope::Refusal;

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
        /// Whether the operation needs a project. None does yet.
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
    retired("capture", 3),
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const QUERY: &[&str] = &[
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
    ];

    const APPLY: &[&str] = &[
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

    fn names(tool: Tool) -> Vec<&'static str> {
        tool.operations().iter().map(|op| op.name).collect()
    }

    #[test]
    fn the_baseline_lists_are_todays_spellings_in_todays_order() {
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
    fn help_and_schema_are_available_and_need_no_project() {
        for name in ["help", "schema"] {
            assert_eq!(
                lookup(Tool::Query, Some(name)),
                Lookup::Available {
                    needs_project: false
                }
            );
        }
    }

    #[test]
    fn every_other_recognized_spelling_is_unavailable() {
        for tool in [Tool::Query, Tool::Apply] {
            for name in names(tool)
                .into_iter()
                .filter(|n| !["help", "schema"].contains(n))
            {
                assert!(
                    matches!(lookup(tool, Some(name)), Lookup::Unavailable { .. }),
                    "{name} must be unavailable"
                );
            }
        }
    }

    #[test]
    fn each_family_names_its_replacing_build() {
        let build = |tool, name| match lookup(tool, Some(name)) {
            Lookup::Unavailable { build } => build,
            other => panic!("{name} is {other:?}"),
        };
        assert_eq!(build(Tool::Query, "document"), 3);
        assert_eq!(build(Tool::Apply, "capture"), 3);
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
}
