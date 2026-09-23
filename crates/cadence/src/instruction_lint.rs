//! Assertions over real compiled products, with no checkout or process seam.
//!
//! Candidate grammar (before any authority lookup): a config name is a complete
//! backticked dotted identifier, a dotted JSON object key, or a code span directly
//! introduced by `config key`. Segments contain letters, digits, `_` and `-`, with
//! optional bracketed indices. Only explicit `<role>`, `<provider>` and `<trigger>`
//! segments expand over compiled vocabularies. Relative `.field` and embedded
//! `<attempt.field>` placeholders are field syntax, not complete identifiers.
//! `answer field`, `request field` and `dispatch field` must directly introduce
//! each excluded code span. Literal `file`/`path` and caller `code`/`language API`
//! markers likewise classify their next span before lookup; slash paths and
//! language examples inside prose strings are not dotted identifiers or JSON keys.
//! Operations are JSON operation string values/consts, `operation: name` spans,
//! and explicit cadence_query/apply invocations (including slash-separated names).
//! The grammatical connectors `with`, `operation`, `permission` and `an` after a
//! bare tool name introduce prose, not an invocation. Skill candidates use
//! `/cad-*` syntax, including `skills/cad-*/SKILL.md`; bare agent names do not.
//! Hook candidates are code spans directly introduced by `hook event`.

use std::collections::BTreeSet;

use cadence::execution::{instructions, render::RENDERED_PROJECT_FILES};
use regex::Regex;

fn corpus() -> Vec<(String, String)> {
    let mut surfaces: Vec<_> = RENDERED_PROJECT_FILES.iter().map(|file| {
        (file.path.to_owned(), super::instruction_surfaces::render(file.command)
            .unwrap_or_else(|| panic!("{}: no renderer for {:?}", file.path, file.command)))
    }).collect();
    // An explicit canonical alias is also accepted by main.rs.
    surfaces.push(("review-instructions --alias cad-review".into(),
        super::instruction_surfaces::render(&["review-instructions", "--alias", "cad-review"])
            .expect("canonical review alias")));
    surfaces.push(("executor dispatch_text".into(), instructions::dispatch_text()));
    for present in instructions::MANIFESTS.iter().map(|(name, _)| vec![*name])
        .chain(std::iter::once(Vec::new()))
    {
        surfaces.push((format!("executor command_policy {present:?}"),
            instructions::command_policy(&[], &present).to_string()));
    }
    surfaces.extend(cadence::help::table::COMMANDS.iter()
        .map(|command| (format!("help description {}", command.name), command.description.to_owned())));
    surfaces.push(("reviewer brief".into(), cadence::review::provider::payload::brief().to_owned()));
    surfaces
}

fn config_expansions(token: &str) -> Vec<String> {
    let schema = super::config::schema();
    let family = |prefix: &str| -> BTreeSet<&str> {
        schema.keys().filter_map(|key| key.strip_prefix(prefix)?.split('.').next()).collect()
    };
    let roles: BTreeSet<_> = super::config::roles::ROLES.into_iter().collect();
    let mut expanded = vec![token.to_owned()];
    for (marker, vocabulary) in [
        ("<role>", roles),
        ("<provider>", family("review.providers.")),
        ("<trigger>", family("review.triggers.")),
    ] {
        expanded = expanded.into_iter().flat_map(|name| {
            if name.contains(marker) {
                vocabulary.iter().map(|value| name.replace(marker, value)).collect()
            } else {
                vec![name]
            }
        }).collect();
    }
    expanded
}

#[test]
fn compiled_instructions_name_only_what_exists() {
    let schema = super::config::schema();
    let operations: BTreeSet<_> = super::server::query_operation_names()
        .chain(super::server::apply_operation_names()).collect();
    // Hand-authored skills, traced against skills/ independently of this assertion.
    let mut skills: BTreeSet<_> = [
        "cad-assumptions-analyzer-contract", "cad-plan-checker-contract",
        "cad-planner-contract", "cad-review-delivery", "cad-reviewer-contract",
    ].into_iter().collect();
    skills.extend(cadence::help::table::COMMANDS.iter().map(|command| command.name));
    skills.extend(RENDERED_PROJECT_FILES.iter().map(|file| {
        file.path.strip_prefix("skills/").unwrap().strip_suffix("/SKILL.md").unwrap()
    }));
    // guard::input and guard::bash accept this named event. The hook manifest is
    // a separate artifact, never a member of this instruction corpus.
    let hook_events = ["PreToolUse"];
    let segment = r"(?:[A-Za-z_][A-Za-z0-9_-]*(?:\[[A-Za-z0-9_]+\])?|<(?:role|provider|trigger)>)";
    let dotted = Regex::new(&format!(r"^{segment}(?:\.{segment})+$")).unwrap();
    let identifier = Regex::new(&format!(r"^{segment}(?:\.{segment})*$")).unwrap();
    let spans = Regex::new(r"`([^`\n]+)`").unwrap();
    let keys = Regex::new(r#""([^"\n]+)"\s*:"#).unwrap();
    let json_operations = Regex::new(r#""operation"\s*:\s*"([^"]+)""#).unwrap();
    let operation_consts = Regex::new(r#""operation"\s*:\s*\{[^{}]*"const"\s*:\s*"([^"]+)""#).unwrap();
    let operation_fields = Regex::new(r#"\boperation:\s*"?([a-z][a-z0-9_-]*)"#).unwrap();
    let invocations = Regex::new(r"\bcadence_(?:query|apply)(?:`\s+with\s+`|\s+(?:with\s+operation\s+)?)([a-z][a-z0-9_/-]*)").unwrap();
    let skill_names = Regex::new(r"/(cad-[a-z0-9-]+)").unwrap();
    let mut unresolved = BTreeSet::new();
    for (surface, text) in corpus() {
        let mut candidates = BTreeSet::new();
        for capture in spans.captures_iter(&text) {
            let whole = capture.get(0).unwrap();
            let token = &capture[1];
            let prefix = text[..whole.start()].trim_end();
            if prefix.ends_with("hook event") {
                candidates.insert(("hook event", token.to_owned()));
                continue;
            }
            if ["answer field", "request field", "dispatch field", "file", "path", "code", "language API"]
                .iter().any(|marker| prefix.ends_with(marker))
            {
                continue;
            }
            if dotted.is_match(token) || (prefix.ends_with("config key") && identifier.is_match(token)) {
                candidates.insert(("config key", token.to_owned()));
            }
        }
        for capture in keys.captures_iter(&text) {
            if dotted.is_match(&capture[1]) {
                candidates.insert(("config key", capture[1].to_owned()));
            }
        }
        for pattern in [&json_operations, &operation_consts, &operation_fields] {
            for capture in pattern.captures_iter(&text) {
                candidates.insert(("wire operation", capture[1].to_owned()));
            }
        }
        for capture in invocations.captures_iter(&text) {
            if !["with", "operation", "permission", "an"].contains(&&capture[1]) {
                candidates.extend(capture[1].split('/').map(|name| ("wire operation", name.to_owned())));
            }
        }
        for capture in skill_names.captures_iter(&text) {
            candidates.insert(("skill", capture[1].to_owned()));
        }
        // No candidate is filtered by membership before this point.
        for (kind, token) in candidates {
            let known = match kind {
                "config key" => config_expansions(&token).iter().all(|name| schema.contains_key(name)),
                "wire operation" => operations.contains(token.as_str()),
                "skill" => skills.contains(token.as_str()),
                "hook event" => hook_events.contains(&token.as_str()),
                _ => unreachable!(),
            };
            if !known {
                unresolved.insert(format!("{surface}: {kind} `{token}`"));
            }
        }
    }
    assert!(unresolved.is_empty(), "unresolved compiled instruction names:\n{}",
        unresolved.into_iter().collect::<Vec<_>>().join("\n"));
}

#[test]
fn schema_defaults_match_their_declared_domain() {
    for (key, spec) in super::config::schema() {
        // Retired entries are migration evidence, with no live default required.
        if spec["disposition"] == "dead" {
            continue;
        }
        let default = spec.get("default").unwrap_or_else(|| panic!("{key}: missing default"));
        // Import semantics admit explicit null array defaults as unanswered;
        // missing defaults must not be silently substituted with null.
        assert!(super::config::reload::valid_type(spec, default, true)
            && super::config::write::valid_grammar(spec, default),
            "{key}: default {default} is outside its declared domain {spec}");
    }
}

#[test]
fn rendered_files_obey_named_byte_ceilings() {
    let ceilings = [
        ("skills/cad-help/SKILL.md", 1024),
        ("skills/cad-spike/SKILL.md", 6144),
        ("skills/cad-debug/SKILL.md", 15360),
        ("skills/cad-undo/SKILL.md", 4096),
        ("skills/cad-land/SKILL.md", 8192),
        ("skills/cad-milestone/SKILL.md", 6144),
        ("skills/cad-suggest/SKILL.md", 1536),
        ("skills/cad-why/SKILL.md", 4096),
        ("skills/cad-progress/SKILL.md", 1024),
        ("skills/cad-capture/SKILL.md", 1536),
        ("skills/cad-context/SKILL.md", 24576),
        ("skills/cad-plan/SKILL.md", 57344),
        ("skills/cad-executor-contract/SKILL.md", 28672),
        ("skills/cad-execute/SKILL.md", 20480),
        ("skills/cad-verifier-contract/SKILL.md", 18432),
        ("skills/cad-verify/SKILL.md", 18432),
        ("skills/cad-review/SKILL.md", 12288),
        ("skills/cad-decision-review/SKILL.md", 12288),
        ("skills/cad-minimalism-review/SKILL.md", 12288),
        ("skills/cad-plan-review/SKILL.md", 12288),
        ("skills/cad-audit/SKILL.md", 9216),
        ("skills/cad-coverage/SKILL.md", 9216),
        ("skills/cad-read-contract/SKILL.md", 6144),
        ("skills/cad-task/SKILL.md", 32768),
    ];
    for file in RENDERED_PROJECT_FILES {
        let ceiling = ceilings.iter().find(|(path, _)| *path == file.path)
            .unwrap_or_else(|| panic!("{}: missing named byte ceiling", file.path)).1;
        let rendered = super::instruction_surfaces::render(file.command)
            .unwrap_or_else(|| panic!("{}: missing renderer", file.path));
        assert!(rendered.len() <= ceiling,
            "{}: {} UTF-8 bytes exceed the {ceiling}-byte ceiling", file.path, rendered.len());
    }
}
