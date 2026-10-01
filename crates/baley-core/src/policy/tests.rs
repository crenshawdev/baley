//! The policy's decisions on supplied files. Expected values come from
//! design 0003 (CFG-R6, CFG-R12, CFG-R16, section 6) and from `toml`'s own
//! error rendering, never from running this code.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::panic::catch_unwind;

use super::schema::Default as Builtin;
use super::*;

const GLOBAL: &str = "/c/config.toml";
const PROJECT: &str = "/r/baley.toml";

fn file(path: &str, text: &str) -> SettingsFile {
    SettingsFile {
        path: path.into(),
        bytes: text.as_bytes().to_vec(),
        digest: format!("digest of {path}"),
    }
}

fn file_ref(path: &str) -> FileRef {
    FileRef {
        path: path.into(),
        digest: format!("digest of {path}"),
    }
}

fn source(layer: Layer, path: &str, line: u32, column: u32) -> Source {
    Source {
        layer,
        file: Some(file_ref(path)),
        line,
        column,
    }
}

fn default_source() -> Source {
    Source {
        layer: Layer::Default,
        file: None,
        line: 0,
        column: 0,
    }
}

/// The standard schema over optional global and project texts.
fn standard(
    host: Option<Host>,
    global: Option<&str>,
    project: Option<&str>,
) -> Result<EffectivePolicy, Unavailable> {
    let global = global.map(|text| file(GLOBAL, text));
    let project = project.map(|text| file(PROJECT, text));
    effective_policy(Schema::standard(), host, global.as_ref(), project.as_ref())
}

/// The fault the global file alone gives under the standard schema.
fn global_fault(text: &str) -> Unavailable {
    standard(None, Some(text), None).expect_err(text)
}

fn entry(name: &str, default: Builtin, scope: Scope) -> Entry {
    Entry {
        name: name.into(),
        kind: Kind::Bool,
        default,
        scope,
        owner: "test",
    }
}

/// One setting of each scope, for the cases Build 2's schema has none of.
fn test_schema() -> Schema {
    Schema::new(vec![
        entry("example.both", Builtin::Bool(true), Scope::Both),
        entry("example.project_only", Builtin::Bool(false), Scope::Project),
        entry("example.global_only", Builtin::Bool(false), Scope::Global),
    ])
}

fn value_of<'p>(policy: &'p EffectivePolicy, name: &str) -> &'p Effective {
    &policy.settings[name]
}

fn rung_map() -> RungMap {
    RungMap::new(["l", "m", "h", "x", "M"].map(String::from))
}

fn catalog(names: &[&str], version: u64) -> AcceptedNames {
    AcceptedNames {
        names: names.iter().map(|name| name.to_string()).collect(),
        version,
    }
}

fn attempt(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

fn route(policy: &EffectivePolicy, role: Role, host: Host, n: u32) -> Result<Route, RouteRefusal> {
    resolve_route(&RouteRequest {
        policy,
        policy_version: 1,
        role,
        host,
        attempt: attempt(n),
        catalog: &catalog(&["opus", "sonnet", "haiku"], 1),
        rungs: &rung_map(),
    })
}

#[test]
fn standard_schema_holds_build_2s_thirteen_entries_with_their_defaults() {
    use Builtin::{Absent, Bool};
    let effort = Builtin::Rung;
    let expected = [
        ("roles.planner.model", Kind::ModelName, Absent),
        ("roles.analyzer.model", Kind::ModelName, Absent),
        ("roles.checker.model", Kind::ModelName, Absent),
        ("roles.executor.model", Kind::ModelName, Absent),
        ("roles.verifier.model", Kind::ModelName, Absent),
        ("roles.reviewer.model", Kind::ModelName, Absent),
        ("roles.planner.effort", Kind::Rung, effort(Rung::High)),
        ("roles.analyzer.effort", Kind::Rung, effort(Rung::High)),
        ("roles.checker.effort", Kind::Rung, effort(Rung::Low)),
        ("roles.executor.effort", Kind::Rung, effort(Rung::High)),
        ("roles.verifier.effort", Kind::Rung, effort(Rung::High)),
        ("roles.reviewer.effort", Kind::Rung, effort(Rung::Medium)),
        ("escalate_on_failure", Kind::Bool, Bool(false)),
    ];
    let schema = Schema::standard();
    assert_eq!(schema.entries().len(), 13);
    for (name, kind, default) in expected {
        let entry = schema
            .get(name)
            .unwrap_or_else(|| panic!("{name} is missing"));
        assert_eq!(
            (entry.kind, entry.default, entry.scope, entry.owner),
            (kind, default, Scope::Both, "0003"),
            "{name}"
        );
    }
}

#[test]
fn schema_refuses_a_repeated_name_and_a_default_off_its_kind() {
    let repeated = vec![
        entry("example.twice", Builtin::Bool(false), Scope::Both),
        entry("example.twice", Builtin::Bool(true), Scope::Both),
    ];
    assert!(catch_unwind(move || Schema::new(repeated)).is_err());
    let off_kind = vec![Entry {
        name: "example.effort".into(),
        kind: Kind::Rung,
        default: Builtin::Bool(true),
        scope: Scope::Both,
        owner: "test",
    }];
    assert!(catch_unwind(move || Schema::new(off_kind)).is_err());
}

#[test]
fn rungs_are_ordered_and_max_is_the_top() {
    use Rung::{High, Low, Max, Medium, Xhigh};
    assert_eq!(Rung::ALL, [Low, Medium, High, Xhigh, Max]);
    assert!(Low < Medium && Medium < High && High < Xhigh && Xhigh < Max);
    assert_eq!(Rung::ALL.map(Rung::up), [Medium, High, Xhigh, Max, Max]);
    for (name, rung) in [
        ("low", Low),
        ("medium", Medium),
        ("high", High),
        ("xhigh", Xhigh),
        ("max", Max),
    ] {
        assert_eq!(Rung::parse(name), Some(rung));
        assert_eq!(rung.name(), name);
    }
    assert_eq!(Rung::parse("High"), None);
    assert_eq!(Rung::parse(""), None);
}

#[test]
fn line_and_column_follow_tomls_rule() {
    let text = "a = 1\nbé = 2";
    let positions = [0, 4, 6, 9, 12].map(|offset| line_and_column(text, offset));
    assert_eq!(positions, [(1, 1), (1, 5), (2, 1), (2, 3), (2, 6)]);
    // One past the end of a file ending in a newline stays on the last line.
    assert_eq!(line_and_column("x = \"\"\"abc\n", 11), (1, 12));
    // The slice through the second é ends inside it, so bytes are counted.
    assert_eq!(line_and_column("\"é\" = é", 7), (1, 8));
    assert_eq!(line_and_column("", 0), (1, 1));
}

#[test]
fn each_layer_overrides_the_one_below() {
    // Each value is written at the leaf key `both`, column 9.
    let schema = test_schema();
    let sectioned = "example.both = false\n[host.codex]\nexample.both = true\n";
    let top_only = "example.both = false\n";
    let (global_full, global_top) = (file(GLOBAL, sectioned), file(GLOBAL, top_only));
    let (project_full, project_top) = (file(PROJECT, sectioned), file(PROJECT, top_only));
    let project_empty = file(PROJECT, "");
    let cases = [
        (
            Some(&global_full),
            Some(&project_full),
            true,
            source(Layer::ProjectHost, PROJECT, 3, 9),
        ),
        (
            Some(&global_full),
            Some(&project_top),
            false,
            source(Layer::Project, PROJECT, 1, 9),
        ),
        (
            Some(&global_full),
            Some(&project_empty),
            true,
            source(Layer::GlobalHost, GLOBAL, 3, 9),
        ),
        (
            Some(&global_top),
            Some(&project_empty),
            false,
            source(Layer::Global, GLOBAL, 1, 9),
        ),
        (None, None, true, default_source()),
    ];
    for (global, project, value, expected) in cases {
        let policy = effective_policy(&schema, Some(Host::Codex), global, project).unwrap();
        let effective = value_of(&policy, "example.both");
        assert_eq!(effective.value, Some(Value::Bool(value)), "{expected:?}");
        assert_eq!(effective.source, expected);
    }
}

#[test]
fn only_the_connected_hosts_section_applies() {
    let text = "[host.claude-code]\nescalate_on_failure = true\n\
                [host.codex.roles.planner]\neffort = \"max\"\n";
    let claude = standard(Some(Host::ClaudeCode), Some(text), None).unwrap();
    assert_eq!(
        claude.escalate_on_failure(),
        (true, &source(Layer::GlobalHost, GLOBAL, 2, 1))
    );
    assert_eq!(
        claude.effort(Role::Planner),
        (Rung::High, &default_source())
    );

    let codex = standard(Some(Host::Codex), Some(text), None).unwrap();
    assert_eq!(codex.escalate_on_failure(), (false, &default_source()));
    assert_eq!(
        codex.effort(Role::Planner),
        (Rung::Max, &source(Layer::GlobalHost, GLOBAL, 4, 1))
    );

    let none = standard(None, Some(text), None).unwrap();
    assert_eq!(none.host, None);
    assert_eq!(none.escalate_on_failure(), (false, &default_source()));
    assert_eq!(none.effort(Role::Planner), (Rung::High, &default_source()));
    assert!(none.diagnostics.is_empty());
}

#[test]
fn a_project_setting_in_the_global_file_is_ignored_with_a_diagnostic() {
    // A diagnostic sits at the leaf key: column 9 is `project_only` after `example.`.
    let schema = test_schema();
    let global = file(GLOBAL, "example.project_only = true\n");
    let project = file(PROJECT, "example.global_only = true\n");
    let policy = effective_policy(&schema, None, Some(&global), Some(&project)).unwrap();
    for name in ["example.project_only", "example.global_only"] {
        let effective = value_of(&policy, name);
        assert_eq!(effective.value, Some(Value::Bool(false)), "{name}");
        assert_eq!(effective.source, default_source(), "{name}");
    }
    assert_eq!(
        policy.diagnostics,
        [
            Diagnostic {
                layer: FileLayer::Global,
                path: GLOBAL.into(),
                name: "example.project_only".into(),
                line: 1,
                column: 9,
                kind: DiagnosticKind::WrongScope {
                    scope: Scope::Project
                },
            },
            Diagnostic {
                layer: FileLayer::Project,
                path: PROJECT.into(),
                name: "example.global_only".into(),
                line: 1,
                column: 9,
                kind: DiagnosticKind::WrongScope {
                    scope: Scope::Global
                },
            },
        ]
    );
    assert_eq!(
        policy
            .diagnostics
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "/c/config.toml:1:9: example.project_only is a project setting and was ignored in the global file",
            "/r/baley.toml:1:9: example.global_only is a global setting and was ignored in the project file",
        ]
    );
}

#[test]
fn an_unknown_name_gives_a_diagnostic_and_changes_nothing() {
    let text = "roles.planner.effort = \"low\"\n\
                roles.planner.foo = 1\n\
                roles.nobody.effort = \"low\"\n\
                [git]\n\
                protected_branches = [\"main\"]\n\
                [project]\n\
                id = \"x\"\n";
    let policy = standard(None, Some(text), None).unwrap();
    assert_eq!(
        policy.effort(Role::Planner),
        (Rung::Low, &source(Layer::Global, GLOBAL, 1, 15))
    );
    for (role, rung) in [
        (Role::Analyzer, Rung::High),
        (Role::Checker, Rung::Low),
        (Role::Executor, Rung::High),
        (Role::Verifier, Rung::High),
        (Role::Reviewer, Rung::Medium),
    ] {
        assert_eq!(policy.effort(role), (rung, &default_source()), "{role:?}");
    }
    let found: Vec<_> = policy
        .diagnostics
        .iter()
        .map(|d| (d.name.as_str(), d.line, d.kind))
        .collect();
    let unknown = DiagnosticKind::UnknownName;
    assert_eq!(
        found,
        [
            ("roles.planner.foo", 2, unknown),
            ("roles.nobody.effort", 3, unknown),
            ("git.protected_branches", 5, unknown),
            ("project.id", 7, unknown),
        ]
    );
    assert_eq!(
        policy.diagnostics[2].to_string(),
        "/c/config.toml:5:1: git.protected_branches is not a setting Baley reads and was ignored"
    );
}

#[test]
fn a_quoted_key_holding_dots_is_an_unknown_name_not_a_nested_setting() {
    let quoted = Diagnostic {
        layer: FileLayer::Global,
        path: GLOBAL.into(),
        name: "\"roles.planner.effort\"".into(),
        line: 1,
        column: 1,
        kind: DiagnosticKind::UnknownName,
    };
    let alone = standard(None, Some("\"roles.planner.effort\" = \"low\"\n"), None).unwrap();
    assert_eq!(alone.effort(Role::Planner), (Rung::High, &default_source()));
    assert_eq!(alone.diagnostics, std::slice::from_ref(&quoted));

    let both = "\"roles.planner.effort\" = \"low\"\nroles.planner.effort = \"max\"\n";
    let both = standard(None, Some(both), None).unwrap();
    let (rung, from) = both.effort(Role::Planner);
    assert_eq!((rung, from.layer, from.line), (Rung::Max, Layer::Global, 2));
    assert_eq!(both.diagnostics, [quoted]);
}

#[test]
fn diagnostics_come_out_in_file_order_when_dotted_keys_interleave() {
    let text = "roles.planner.foo = 1\nunrecognized = 2\nroles.planner.bar = 3\n";
    let policy = standard(None, Some(text), None).unwrap();
    let found: Vec<_> = policy
        .diagnostics
        .iter()
        .map(|d| (d.name.as_str(), d.line, d.kind))
        .collect();
    let unknown = DiagnosticKind::UnknownName;
    assert_eq!(
        found,
        [
            ("roles.planner.foo", 1, unknown),
            ("unrecognized", 2, unknown),
            ("roles.planner.bar", 3, unknown),
        ]
    );
}

#[test]
fn the_project_table_of_the_project_file_is_not_a_setting() {
    let text = "escalate_on_failure = true\n\
                [project]\n\
                id = \"6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f\"\n\
                name = \"x\"\n";
    let policy = standard(None, None, Some(text)).unwrap();
    assert!(policy.diagnostics.is_empty());
    assert_eq!(
        policy.escalate_on_failure(),
        (true, &source(Layer::Project, PROJECT, 1, 1))
    );
}

#[test]
fn an_unknown_host_section_is_ignored_with_one_diagnostic() {
    let text = "[host.cursor]\nescalate_on_failure = 3\n";
    let policy = standard(Some(Host::ClaudeCode), Some(text), None).unwrap();
    assert_eq!(
        policy.diagnostics,
        [Diagnostic {
            layer: FileLayer::Global,
            path: GLOBAL.into(),
            name: "host.cursor".into(),
            line: 1,
            column: 7,
            kind: DiagnosticKind::UnknownHost,
        }]
    );
    assert_eq!(
        policy.diagnostics[0].to_string(),
        "/c/config.toml:1:7: host.cursor is not a host Baley knows (claude-code, codex) and its section was ignored"
    );
    assert_eq!(policy.escalate_on_failure(), (false, &default_source()));
}

#[test]
fn an_invalid_value_makes_the_whole_policy_unavailable_naming_its_file() {
    let global = "roles.executor.effort = \"max\"\n";
    let project = "# settings for this project\n\
                   escalate_on_failure = true\n\
                   roles.planner.effort = \"hig\"\n";
    let refusal = standard(None, Some(global), Some(project)).unwrap_err();
    assert_eq!(
        refusal,
        Unavailable {
            path: PROJECT.into(),
            fault: Fault::OutsideGrammar {
                name: "roles.planner.effort".into(),
                kind: Kind::Rung,
                written: "hig".into(),
                line: 3,
                column: 24,
            },
        }
    );
    assert_eq!(refusal.code(), "config-unavailable");
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /r/baley.toml:3:24: roles.planner.effort is \"hig\", which is not a rung (low, medium, high, xhigh, max)"
    );
}

#[test]
fn the_first_invalid_value_in_the_file_is_the_one_named() {
    // The walk meets escalate_on_failure first; the file has the effort first.
    let refusal = global_fault("roles.planner.effort = \"hig\"\nescalate_on_failure = \"yes\"\n");
    let Fault::OutsideGrammar { name, line, .. } = refusal.fault else {
        panic!("{refusal}")
    };
    assert_eq!((name.as_str(), line), ("roles.planner.effort", 1));
}

#[test]
fn a_wrong_type_names_the_setting_and_its_position() {
    let cases = [
        (
            "escalate_on_failure = \"yes\"\n",
            "escalate_on_failure",
            Expected::Bool,
            "string",
            (1, 23),
            "escalate_on_failure is a string, not a boolean",
        ),
        (
            "roles.planner.effort = 3\n",
            "roles.planner.effort",
            Expected::Rung,
            "integer",
            (1, 24),
            "roles.planner.effort is an integer, not a rung (low, medium, high, xhigh, max)",
        ),
        (
            "roles.planner.model = true\n",
            "roles.planner.model",
            Expected::ModelName,
            "boolean",
            (1, 23),
            "roles.planner.model is a boolean, not a model name",
        ),
        (
            "roles.planner = \"x\"\n",
            "roles.planner",
            Expected::Table,
            "string",
            (1, 17),
            "roles.planner is a string, not a table",
        ),
        (
            "[roles.planner.effort]\nx = 1\n",
            "roles.planner.effort",
            Expected::Rung,
            "table",
            (1, 1),
            "roles.planner.effort is a table, not a rung (low, medium, high, xhigh, max)",
        ),
        (
            "host = 1\n",
            "host",
            Expected::Table,
            "integer",
            (1, 8),
            "host is an integer, not a table",
        ),
    ];
    for (text, name, expected, found, (line, column), message) in cases {
        let refusal = global_fault(text);
        assert_eq!(
            refusal.fault,
            Fault::WrongType {
                name: name.into(),
                expected,
                found,
                line,
                column,
            },
            "{text}"
        );
        assert_eq!(
            refusal.to_string(),
            format!("config-unavailable: /c/config.toml:{line}:{column}: {message}")
        );
    }
}

#[test]
fn a_section_for_another_host_is_still_validated() {
    let text = "[host.codex]\nescalate_on_failure = \"no\"\n";
    let refusal = standard(Some(Host::ClaudeCode), Some(text), None).unwrap_err();
    assert_eq!(
        refusal.fault,
        Fault::WrongType {
            name: "host.codex.escalate_on_failure".into(),
            expected: Expected::Bool,
            found: "string",
            line: 2,
            column: 23,
        }
    );
}

#[test]
fn a_parse_error_names_its_line_and_column() {
    let text = "escalate_on_failure = true\nroles = [\n";
    let toml_error = toml::de::DeTable::parse(text).unwrap_err();
    assert_eq!(
        toml_error.to_string().lines().next(),
        Some("TOML parse error at line 2, column 10")
    );
    let refusal = global_fault(text);
    assert_eq!(
        refusal.fault,
        Fault::Parse {
            position: Some((2, 10)),
            message: toml_error.message().to_owned(),
        }
    );
    assert_eq!(
        refusal.to_string(),
        format!(
            "config-unavailable: /c/config.toml:2:10: {}",
            toml_error.message()
        )
    );
}

#[test]
fn a_parse_error_without_a_span_is_named_without_a_position() {
    let refusal = Unavailable {
        path: GLOBAL.into(),
        fault: Fault::Parse {
            position: None,
            message: "unexpected end".into(),
        },
    };
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /c/config.toml: unexpected end"
    );
}

#[test]
fn a_parse_error_at_the_end_of_file_or_at_a_non_ascii_character_keeps_tomls_position() {
    for (text, position) in [("x = \"\"\"abc\n", (1, 12)), ("\"é\" = é", (1, 8))] {
        let Fault::Parse {
            position: found, ..
        } = global_fault(text).fault
        else {
            panic!("{text} must not parse")
        };
        assert_eq!(found, Some(position), "{text}");
    }
}

#[test]
fn bytes_that_are_not_utf8_are_unavailable_at_their_position() {
    let bytes = SettingsFile {
        path: GLOBAL.into(),
        bytes: b"a = 1\nb = \xff".to_vec(),
        digest: "d".into(),
    };
    let refusal = effective_policy(Schema::standard(), None, Some(&bytes), None).unwrap_err();
    assert_eq!(refusal.fault, Fault::NotUtf8 { line: 2, column: 5 });
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /c/config.toml:2:5: the file is not UTF-8 from this point"
    );
}

#[test]
fn an_empty_model_is_outside_its_grammar_and_an_absent_one_is_the_default() {
    let refusal = global_fault("roles.planner.model = \"\"\n");
    assert_eq!(
        refusal.fault,
        Fault::OutsideGrammar {
            name: "roles.planner.model".into(),
            kind: Kind::ModelName,
            written: String::new(),
            line: 1,
            column: 23,
        }
    );
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /c/config.toml:1:23: roles.planner.model is empty; write a model name or remove the line"
    );
    let policy = standard(None, Some("roles.planner.effort = \"low\"\n"), None).unwrap();
    assert_eq!(policy.model(Role::Planner), (None, &default_source()));
}

#[test]
fn an_empty_file_and_no_file_both_give_the_defaults_but_only_the_file_has_a_digest() {
    let digest = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let empty = SettingsFile {
        path: GLOBAL.into(),
        bytes: Vec::new(),
        digest: digest.into(),
    };
    let with = effective_policy(Schema::standard(), None, Some(&empty), None).unwrap();
    let without = effective_policy(Schema::standard(), None, None, None).unwrap();
    for policy in [&with, &without] {
        assert_eq!(policy.settings.len(), 13);
        assert!(
            policy
                .settings
                .values()
                .all(|effective| effective.source == default_source())
        );
        assert!(policy.diagnostics.is_empty());
    }
    assert_eq!(with.settings, without.settings);
    assert_eq!(
        with.global,
        Some(FileRef {
            path: GLOBAL.into(),
            digest: digest.into(),
        })
    );
    assert_eq!(without.global, None);
}

#[test]
fn defaults_follow_cfg_r12_when_no_file_is_present() {
    let policy = merge(Schema::standard(), None, None, None);
    for (role, rung) in [
        (Role::Planner, Rung::High),
        (Role::Analyzer, Rung::High),
        (Role::Checker, Rung::Low),
        (Role::Executor, Rung::High),
        (Role::Verifier, Rung::High),
        (Role::Reviewer, Rung::Medium),
    ] {
        assert_eq!(policy.effort(role), (rung, &default_source()), "{role:?}");
        assert_eq!(policy.model(role), (None, &default_source()), "{role:?}");
    }
    assert_eq!(policy.escalate_on_failure(), (false, &default_source()));
    assert_eq!((&policy.global, &policy.project), (&None, &None));
}

#[test]
fn an_absent_model_passes_no_model() {
    let policy = standard(None, None, None).unwrap();
    let route = resolve_route(&RouteRequest {
        policy: &policy,
        policy_version: 0,
        role: Role::Planner,
        host: Host::ClaudeCode,
        attempt: attempt(1),
        catalog: &catalog(&[], 1),
        rungs: &rung_map(),
    })
    .unwrap();
    assert_eq!(route.model, None);
    assert_eq!(
        route.model_source,
        SettingSource {
            setting: "roles.planner.model".into(),
            layer: Layer::Default,
            file: None,
        }
    );
    assert!(
        route
            .reasons
            .contains(&"roles.planner.model is not set; the host session's model is used".into())
    );
}

#[test]
fn a_model_the_hosts_catalog_lacks_gives_unknown_model() {
    let policy = standard(None, None, Some("roles.reviewer.model = \"gpt-9\"\n")).unwrap();
    let refusal = resolve_route(&RouteRequest {
        policy: &policy,
        policy_version: 4,
        role: Role::Reviewer,
        host: Host::ClaudeCode,
        attempt: attempt(1),
        catalog: &catalog(&["sonnet", "opus"], 7),
        rungs: &rung_map(),
    })
    .unwrap_err();
    assert_eq!(
        refusal,
        RouteRefusal::UnknownModel {
            setting: "roles.reviewer.model".into(),
            model: "gpt-9".into(),
            layer: Layer::Project,
            file: Some(PROJECT.into()),
            host: Host::ClaudeCode,
            accepted: vec!["opus".into(), "sonnet".into()],
        }
    );
    assert_eq!(refusal.code(), "unknown-model");
    assert_eq!(
        refusal.to_string(),
        "unknown-model: roles.reviewer.model is gpt-9 from the project file /r/baley.toml, which the claude-code catalog does not accept; accepted: opus, sonnet"
    );
}

#[test]
fn attempt_two_with_escalation_runs_one_rung_up_and_later_attempts_hold() {
    let text = "escalate_on_failure = true\nroles.executor.effort = \"high\"\n";
    let policy = standard(None, Some(text), None).unwrap();
    let routes: Vec<Route> = (1..=4)
        .map(|n| route(&policy, Role::Executor, Host::ClaudeCode, n).unwrap())
        .collect();
    let rungs: Vec<_> = routes
        .iter()
        .map(|r| (r.starting_rung, r.rung, r.escalated))
        .collect();
    assert_eq!(
        rungs,
        [
            (Rung::High, Rung::High, false),
            (Rung::High, Rung::Xhigh, true),
            (Rung::High, Rung::Xhigh, true),
            (Rung::High, Rung::Xhigh, true),
        ]
    );
    assert_eq!(routes[0].reasons[2], "attempt 1 runs at the starting rung");
    assert_eq!(
        routes[1].reasons[2],
        "attempt 2 with escalate_on_failure true from the global file /c/config.toml: one rung up from high to xhigh"
    );
    assert_eq!(
        routes[2].reasons[2],
        "attempt 3 with escalate_on_failure true from the global file /c/config.toml: held at xhigh, one rung above high"
    );
}

#[test]
fn max_stays_max() {
    let text = "escalate_on_failure = true\nroles.planner.effort = \"max\"\n";
    let policy = standard(None, Some(text), None).unwrap();
    let route = route(&policy, Role::Planner, Host::ClaudeCode, 2).unwrap();
    assert_eq!((route.rung, route.escalated), (Rung::Max, false));
    assert_eq!(
        route.reasons[2],
        "attempt 2 with escalate_on_failure true from the global file /c/config.toml: max is the top rung, held there"
    );
}

#[test]
fn escalation_off_keeps_the_rung() {
    let policy = standard(None, Some("roles.verifier.effort = \"medium\"\n"), None).unwrap();
    for n in [1, 3] {
        let route = route(&policy, Role::Verifier, Host::ClaudeCode, n).unwrap();
        assert_eq!((route.rung, route.escalated), (Rung::Medium, false), "{n}");
    }
    let third = route(&policy, Role::Verifier, Host::ClaudeCode, 3).unwrap();
    assert_eq!(
        third.reasons[2],
        "attempt 3 with escalate_on_failure false from the built-in default: medium kept"
    );
}

#[test]
fn the_route_names_the_setting_and_layer_of_its_model_and_effort() {
    let global = "[roles.checker]\nmodel = \"haiku\"\n";
    let project = "[host.codex.roles.checker]\neffort = \"xhigh\"\n";
    let policy = standard(Some(Host::Codex), Some(global), Some(project)).unwrap();
    let route = resolve_route(&RouteRequest {
        policy: &policy,
        policy_version: 11,
        role: Role::Checker,
        host: Host::Codex,
        attempt: attempt(1),
        catalog: &AcceptedNames {
            names: BTreeSet::from(["haiku".to_owned()]),
            version: 3,
        },
        rungs: &rung_map(),
    })
    .unwrap();
    assert_eq!(
        route,
        Route {
            role: Role::Checker,
            model: Some("haiku".into()),
            starting_rung: Rung::Xhigh,
            rung: Rung::Xhigh,
            attempt: 1,
            escalated: false,
            host_effort: "x".into(),
            effort_source: SettingSource {
                setting: "roles.checker.effort".into(),
                layer: Layer::ProjectHost,
                file: Some(PROJECT.into()),
            },
            model_source: SettingSource {
                setting: "roles.checker.model".into(),
                layer: Layer::Global,
                file: Some(GLOBAL.into()),
            },
            policy_version: 11,
            catalog_version: 3,
            reasons: vec![
                "roles.checker.effort is xhigh from the project file's codex section /r/baley.toml"
                    .into(),
                "roles.checker.model is haiku from the global file /c/config.toml".into(),
                "attempt 1 runs at the starting rung".into(),
                "codex runs rung xhigh as x".into(),
            ],
        }
    );
}

// Project ids below are written by hand from D-10 and RFC 9562: version
// nibble at byte 14, variant nibble at byte 19.

#[test]
fn a_project_id_with_any_rfc_variant_nibble_is_not_refused() {
    for id in [
        "6f1c2a4e-8b1d-4c3a-8e2f-0a5b7c9d1e3f",
        "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f",
        "6f1c2a4e-8b1d-4c3a-ae2f-0a5b7c9d1e3f",
        "6f1c2a4e-8b1d-4c3a-be2f-0a5b7c9d1e3f",
    ] {
        assert!(is_project_id(id), "{id}");
    }
}

#[test]
fn an_upper_case_project_id_is_not_folded_to_lower_case() {
    assert!(!is_project_id("6F1C2A4E-8B1D-4C3A-9E2F-0A5B7C9D1E3F"));
}

#[test]
fn a_project_id_of_another_uuid_version_is_refused() {
    for id in [
        "6f1c2a4e-8b1d-1c3a-9e2f-0a5b7c9d1e3f",
        "6f1c2a4e-8b1d-7c3a-9e2f-0a5b7c9d1e3f",
    ] {
        assert!(!is_project_id(id), "{id}");
    }
}

#[test]
fn a_project_id_outside_the_rfc_variant_is_refused() {
    for id in [
        "6f1c2a4e-8b1d-4c3a-ce2f-0a5b7c9d1e3f",
        "6f1c2a4e-8b1d-4c3a-7e2f-0a5b7c9d1e3f",
    ] {
        assert!(!is_project_id(id), "{id}");
    }
}

#[test]
fn a_project_id_one_byte_short_or_long_is_refused() {
    for id in [
        "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3",
        "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f0",
    ] {
        assert!(!is_project_id(id), "{id}");
    }
}

#[test]
fn a_project_id_with_a_hyphen_out_of_place_is_refused() {
    assert!(!is_project_id("6f1c2a4e8-b1d-4c3a-9e2f-0a5b7c9d1e3f"));
}

#[test]
fn a_project_id_holding_a_letter_past_f_is_refused() {
    assert!(!is_project_id("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3g"));
}

const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";

fn project_fault(text: &str) -> Unavailable {
    read_project(&file(PROJECT, text)).expect_err(text)
}

#[test]
fn a_valid_project_table_gives_back_its_id_and_name() {
    let text = format!("escalate_on_failure = true\n[project]\nid = \"{ID}\"\nname = \"baley\"\n");
    assert_eq!(
        read_project(&file(PROJECT, &text)),
        Ok(ProjectIdentity {
            id: ID.into(),
            name: "baley".into(),
        })
    );
}

#[test]
fn a_project_id_that_is_not_a_lower_case_uuid_v4_is_refused_at_its_value() {
    for id in [
        "6F1C2A4E-8B1D-4C3A-9E2F-0A5B7C9D1E3F",
        "6f1c2a4e-8b1d-1c3a-9e2f-0a5b7c9d1e3f",
        "6f1c2a4e-8b1d-4c3a-ce2f-0a5b7c9d1e3f",
    ] {
        let refusal = project_fault(&format!("[project]\nid = \"{id}\"\nname = \"x\"\n"));
        assert_eq!(
            refusal,
            Unavailable {
                path: PROJECT.into(),
                fault: Fault::Project {
                    name: "project.id",
                    problem: ProjectProblem::NotAnId { written: id.into() },
                    position: Some((2, 6)),
                },
            },
            "{id}"
        );
        assert_eq!(refusal.code(), "config-unavailable");
    }
    assert_eq!(
        project_fault("[project]\nid = \"6F1C2A4E-8B1D-4C3A-9E2F-0A5B7C9D1E3F\"\nname = \"x\"\n")
            .to_string(),
        "config-unavailable: /r/baley.toml:2:6: project.id is \"6F1C2A4E-8B1D-4C3A-9E2F-0A5B7C9D1E3F\", which is not a lower-case UUID version 4"
    );
}

#[test]
fn a_project_table_without_an_id_is_refused() {
    let refusal = project_fault("[project]\nname = \"x\"\n");
    assert_eq!(
        refusal.fault,
        Fault::Project {
            name: "project.id",
            problem: ProjectProblem::Missing,
            position: None,
        }
    );
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /r/baley.toml: project.id is missing; the [project] table needs an id and a string name"
    );
}

#[test]
fn a_project_table_without_a_name_is_refused() {
    let refusal = project_fault(&format!("[project]\nid = \"{ID}\"\n"));
    assert_eq!(
        refusal,
        Unavailable {
            path: PROJECT.into(),
            fault: Fault::Project {
                name: "project.name",
                problem: ProjectProblem::Missing,
                position: None,
            },
        }
    );
}

#[test]
fn a_project_name_that_is_not_a_string_is_refused() {
    let refusal = project_fault(&format!("[project]\nid = \"{ID}\"\nname = 7\n"));
    assert_eq!(
        refusal.fault,
        Fault::Project {
            name: "project.name",
            problem: ProjectProblem::WrongType { found: "integer" },
            position: Some((3, 8)),
        }
    );
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /r/baley.toml:3:8: project.name is an integer, not a string"
    );
}

#[test]
fn a_project_file_without_a_project_table_is_not_an_empty_project() {
    let refusal = project_fault("escalate_on_failure = true\n");
    assert_eq!(
        refusal,
        Unavailable {
            path: PROJECT.into(),
            fault: Fault::Project {
                name: "project",
                problem: ProjectProblem::Missing,
                position: None,
            },
        }
    );
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /r/baley.toml: there is no [project] table; the project file needs one with an id and a string name"
    );
}

/// `text` as a `toml` table with its `project` table taken out.
fn without_project(text: &str) -> (toml::Table, Option<toml::Value>) {
    let mut table: toml::Table = text.parse().expect(text);
    let project = table.remove("project");
    (table, project)
}

/// The settings a project file writes, positions aside.
fn settings_of(text: &str) -> Vec<(String, Option<Host>, Value)> {
    let parsed = parse_layer(&file(PROJECT, text), FileLayer::Project, Schema::standard());
    let parsed = parsed.expect(text);
    parsed
        .values
        .into_iter()
        .map(|written| (written.name, written.host, written.value))
        .collect()
}

#[test]
fn rendering_a_new_id_keeps_every_other_table_and_value() {
    let before = "escalate_on_failure = true\n\
                  [roles.planner]\n\
                  effort = \"low\"\n\
                  [host.codex.roles.checker]\n\
                  effort = \"xhigh\"\n\
                  [review]\n\
                  depth = 3\n\
                  [project]\n\
                  id = \"0b8e2f4a-1c3d-4e5f-a6b7-c8d9e0f1a2b3\"\n\
                  name = \"old\"\n\
                  owner = \"kept\"\n";
    let bytes = render_project(ID, "baley", Some(&file(PROJECT, before))).unwrap();
    let after = String::from_utf8(bytes).unwrap();
    assert_eq!(
        read_project(&file(PROJECT, &after)),
        Ok(ProjectIdentity {
            id: ID.into(),
            name: "baley".into(),
        }),
        "{after}"
    );
    let (rest_before, _) = without_project(before);
    let (rest_after, project_after) = without_project(&after);
    assert_eq!(rest_after, rest_before, "{after}");
    let mut project = toml::Table::new();
    project.insert("id".into(), ID.into());
    project.insert("name".into(), "baley".into());
    project.insert("owner".into(), "kept".into());
    assert_eq!(project_after, Some(toml::Value::Table(project)), "{after}");
    let settings = settings_of(before);
    assert_eq!(settings.len(), 3);
    assert_eq!(settings_of(&after), settings, "{after}");
}

#[test]
fn rendering_without_a_file_writes_only_the_project_table() {
    let after = String::from_utf8(render_project(ID, "baley", None).unwrap()).unwrap();
    let mut project = toml::Table::new();
    project.insert("id".into(), ID.into());
    project.insert("name".into(), "baley".into());
    let mut expected = toml::Table::new();
    expected.insert("project".into(), toml::Value::Table(project));
    assert_eq!(after.parse::<toml::Table>(), Ok(expected), "{after}");
}

#[test]
fn rendering_over_a_file_that_is_not_toml_refuses_naming_it() {
    let refusal = render_project(ID, "baley", Some(&file(PROJECT, "roles = [\n"))).unwrap_err();
    assert_eq!(refusal.path, std::path::PathBuf::from(PROJECT));
    assert!(
        matches!(
            refusal.fault,
            Fault::Parse {
                position: Some((1, 10)),
                ..
            }
        ),
        "{refusal}"
    );
    assert!(
        refusal
            .to_string()
            .starts_with("config-unavailable: /r/baley.toml:1:10: "),
        "{refusal}"
    );
}

/// A schema of one remote-name setting, so the kind is tested apart from the
/// standard schema.
fn remote_schema() -> Schema {
    Schema::new(vec![Entry {
        name: "git.remote".into(),
        kind: Kind::RemoteName,
        default: Builtin::Absent,
        scope: Scope::Both,
        owner: "test",
    }])
}

fn remote_policy(text: &str) -> Result<EffectivePolicy, Unavailable> {
    effective_policy(&remote_schema(), None, Some(&file(GLOBAL, text)), None)
}

#[test]
fn a_remote_name_read_as_a_model_name_is_caught() {
    let policy = remote_policy("git.remote = \"origin\"\n").unwrap();
    assert_eq!(
        value_of(&policy, "git.remote").value,
        Some(Value::RemoteName("origin".into()))
    );
}

#[test]
fn an_empty_remote_name_that_is_accepted_or_not_named_is_caught() {
    let refusal = remote_policy("git.remote = \"\"\n").unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /c/config.toml:1:14: git.remote is empty; write a remote name or remove the line"
    );
}

#[test]
fn a_remote_name_given_as_an_integer_that_is_not_a_type_fault_is_caught() {
    let refusal = remote_policy("git.remote = 1\n").unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "config-unavailable: /c/config.toml:1:14: git.remote is an integer, not a remote name"
    );
}
