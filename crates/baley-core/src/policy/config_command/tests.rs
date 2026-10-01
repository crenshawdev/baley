//! The settings commands' pure half on supplied values. Expected values are
//! written out from design 0003 section 5 and the phase's decisions, never
//! from running this code.

use super::*;
use crate::policy::schema::Default as Builtin;
use crate::policy::{Entry, FileLayer, Host, Kind, Rung, Schema, Scope, Value};

fn entry(name: &str, kind: Kind, default: Builtin, scope: Scope) -> Entry {
    Entry {
        name: name.into(),
        kind,
        default,
        scope,
        owner: "test",
    }
}

/// One setting of each kind and each scope, so `wrong-layer` and the model
/// judge can be produced; Build 2's standard schema holds only both-scoped
/// settings.
fn schema() -> Schema {
    Schema::new(vec![
        entry(
            "roles.planner.model",
            Kind::ModelName,
            Builtin::Absent,
            Scope::Both,
        ),
        entry(
            "roles.planner.effort",
            Kind::Rung,
            Builtin::Rung(Rung::High),
            Scope::Both,
        ),
        entry(
            "example.flag",
            Kind::Bool,
            Builtin::Bool(false),
            Scope::Both,
        ),
        entry(
            "example.project_only",
            Kind::Bool,
            Builtin::Bool(false),
            Scope::Project,
        ),
        entry(
            "example.global_only",
            Kind::Bool,
            Builtin::Bool(false),
            Scope::Global,
        ),
    ])
}

fn judge(
    layer: FileLayer,
    in_project: bool,
    host: Option<Host>,
    pairs: &[(&str, &str)],
) -> Result<Vec<TypedPair>, SetRefusal> {
    judge_pairs(&schema(), layer, in_project, host, pairs)
}

fn refused(layer: FileLayer, in_project: bool, pairs: &[(&str, &str)]) -> SetRefusal {
    judge(layer, in_project, None, pairs).expect_err("the set is refused")
}

fn pair(name: &str, host: Option<Host>, value: Value) -> TypedPair {
    TypedPair {
        name: name.into(),
        host,
        value,
    }
}

#[test]
fn a_project_file_set_outside_a_project_is_not_refused_as_anything_else() {
    // The second pair is unknown too: not-a-project is the first check.
    let refusal = refused(
        FileLayer::Project,
        false,
        &[("example.flag", "true"), ("nonsense", "1")],
    );
    assert_eq!(refusal, SetRefusal::NotAProject);
    assert_eq!(refusal.code(), "not-a-project");
    let text = refusal.to_string();
    assert!(text.starts_with("not-a-project: "), "{text}");
    assert!(text.contains("baley.toml"), "{text}");
    assert!(text.contains("baley init"), "{text}");
}

#[test]
fn a_global_set_outside_a_project_is_not_refused_as_not_a_project() {
    let typed = judge(FileLayer::Global, false, None, &[("example.flag", "true")]);
    assert!(typed.is_ok(), "{typed:?}");
}

#[test]
fn an_unknown_name_or_a_host_prefixed_name_is_not_accepted_as_a_setting() {
    for name in ["example.nonsense", "host.claude-code.example.flag"] {
        let refusal = refused(FileLayer::Global, false, &[(name, "true")]);
        assert_eq!(refusal, SetRefusal::UnknownSetting { name: name.into() });
        assert_eq!(refusal.code(), "unknown-setting");
        let text = refusal.to_string();
        assert!(text.starts_with("unknown-setting: "), "{text}");
        assert!(text.contains(name), "{text}");
    }
}

#[test]
fn a_setting_outside_its_scope_is_not_accepted_in_the_other_file() {
    let global = refused(FileLayer::Global, true, &[("example.project_only", "true")]);
    assert_eq!(
        global,
        SetRefusal::WrongLayer {
            name: "example.project_only".into(),
            scope: Scope::Project,
            layer: FileLayer::Global,
        }
    );
    assert_eq!(global.code(), "wrong-layer");
    let text = global.to_string();
    assert!(text.starts_with("wrong-layer: "), "{text}");
    assert!(text.contains("example.project_only"), "{text}");
    assert!(text.contains("project setting"), "{text}");
    assert!(text.contains("global file"), "{text}");

    let project = refused(FileLayer::Project, true, &[("example.global_only", "true")]);
    assert_eq!(
        project,
        SetRefusal::WrongLayer {
            name: "example.global_only".into(),
            scope: Scope::Global,
            layer: FileLayer::Project,
        }
    );
    let text = project.to_string();
    assert!(text.contains("global setting"), "{text}");
    assert!(text.contains("project file"), "{text}");
}

#[test]
fn a_both_scoped_setting_is_not_refused_in_either_file() {
    for layer in [FileLayer::Global, FileLayer::Project] {
        let typed = judge(layer, true, None, &[("example.flag", "false")]);
        assert!(typed.is_ok(), "{layer:?}: {typed:?}");
    }
}

#[test]
fn a_value_off_its_kinds_grammar_is_not_accepted_and_is_named() {
    let cases = [
        ("example.flag", "True"),
        ("example.flag", "1"),
        ("example.flag", "yes"),
        ("example.flag", "\"true\""),
        ("roles.planner.effort", "High"),
        ("roles.planner.effort", "\"high\""),
        ("roles.planner.model", ""),
    ];
    for (name, value) in cases {
        let refusal = refused(FileLayer::Global, false, &[(name, value)]);
        assert_eq!(
            refusal.code(),
            "invalid-value",
            "{name}={value:?} gave {refusal:?}"
        );
        let text = refusal.to_string();
        assert!(text.starts_with("invalid-value: "), "{text}");
        assert!(text.contains(name), "{text}");
        assert!(
            text.contains(&format!("\"{}\"", value.escape_debug())),
            "{text}"
        );
    }
}

#[test]
fn an_invalid_value_refusal_does_not_say_what_is_expected_for_the_wrong_kind() {
    let flag = refused(FileLayer::Global, false, &[("example.flag", "1")]).to_string();
    assert!(flag.contains("true or false"), "{flag}");
    let rung = refused(
        FileLayer::Global,
        false,
        &[("roles.planner.effort", "High")],
    )
    .to_string();
    assert!(rung.contains("low, medium, high, xhigh, max"), "{rung}");
    let model = refused(FileLayer::Global, false, &[("roles.planner.model", "")]).to_string();
    assert!(model.contains("non-empty"), "{model}");
}

#[test]
fn a_value_with_a_line_break_is_not_printed_raw_in_a_refusal() {
    let text = refused(
        FileLayer::Global,
        false,
        &[("roles.planner.effort", "high\nmax")],
    )
    .to_string();
    assert!(!text.contains('\n'), "{text:?}");
    assert!(text.contains("high\\nmax"), "{text}");
}

#[test]
fn an_invalid_value_before_an_unknown_name_is_not_the_refusal_named() {
    let refusal = refused(
        FileLayer::Global,
        false,
        &[("example.flag", "yes"), ("nonsense", "true")],
    );
    assert_eq!(
        refusal,
        SetRefusal::UnknownSetting {
            name: "nonsense".into()
        }
    );
}

#[test]
fn a_wrong_layer_before_an_invalid_value_is_not_the_refusal_named() {
    let refusal = refused(
        FileLayer::Global,
        true,
        &[("example.flag", "yes"), ("example.project_only", "true")],
    );
    assert_eq!(refusal.code(), "wrong-layer");
}

#[test]
fn valid_pairs_are_not_returned_untyped_unordered_or_without_the_host() {
    let typed = judge(
        FileLayer::Project,
        true,
        Some(Host::Codex),
        &[
            ("roles.planner.effort", "xhigh"),
            ("example.flag", "true"),
            ("roles.planner.model", "opus"),
        ],
    )
    .expect("valid pairs");
    assert_eq!(
        typed,
        vec![
            pair(
                "roles.planner.effort",
                Some(Host::Codex),
                Value::Rung(Rung::Xhigh)
            ),
            pair("example.flag", Some(Host::Codex), Value::Bool(true)),
            pair(
                "roles.planner.model",
                Some(Host::Codex),
                Value::ModelName("opus".into())
            ),
        ]
    );
}

#[test]
fn a_setting_given_twice_is_not_collapsed_by_the_judge() {
    let typed = judge(
        FileLayer::Global,
        false,
        None,
        &[("example.flag", "true"), ("example.flag", "false")],
    )
    .expect("valid pairs");
    assert_eq!(
        typed,
        vec![
            pair("example.flag", None, Value::Bool(true)),
            pair("example.flag", None, Value::Bool(false)),
        ]
    );
}

#[test]
fn the_judge_reading_the_standard_schema_instead_of_its_parameter_is_caught() {
    // `example.flag` is in the test schema only, `roles.executor.effort` in
    // the standard schema only.
    let ours = judge(FileLayer::Global, false, None, &[("example.flag", "true")]);
    assert!(ours.is_ok(), "{ours:?}");
    let theirs = refused(
        FileLayer::Global,
        false,
        &[("roles.executor.effort", "high")],
    );
    assert_eq!(
        theirs,
        SetRefusal::UnknownSetting {
            name: "roles.executor.effort".into()
        }
    );
}
