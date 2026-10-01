//! The settings commands' pure half on supplied values. Expected values are
//! written out from design 0003 section 5 and the phase's decisions, never
//! from running this code.

use std::collections::BTreeMap;

use super::*;
use crate::policy::schema::Default as Builtin;
use crate::policy::{AcceptedNames, Entry, FileLayer, Host, Kind, Rung, Schema, Scope, Value};

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

fn names(list: &[&str]) -> AcceptedNames {
    AcceptedNames {
        names: list.iter().map(|name| name.to_string()).collect(),
        version: 1,
    }
}

/// Claude Code accepts four names and Codex none.
fn accepted() -> BTreeMap<Host, AcceptedNames> {
    BTreeMap::from([
        (
            Host::ClaudeCode,
            names(&["opus", "sonnet", "haiku", "fable"]),
        ),
        (Host::Codex, names(&[])),
    ])
}

fn model(value: &str) -> TypedPair {
    pair("roles.planner.model", None, Value::ModelName(value.into()))
}

fn models(pairs: &[TypedPair], host: Option<Host>) -> Result<(), SetRefusal> {
    judge_models(pairs, host, &accepted())
}

#[test]
fn a_model_the_named_host_holds_is_not_refused() {
    assert_eq!(models(&[model("opus")], Some(Host::ClaudeCode)), Ok(()));
}

#[test]
fn a_model_the_named_host_lacks_is_not_accepted_because_another_host_holds_it() {
    // AC3: opus is claude-code's, so codex refuses it.
    let refusal = models(&[model("opus")], Some(Host::Codex)).expect_err("codex lacks opus");
    assert_eq!(refusal.code(), "unknown-model");
    assert_eq!(
        refusal,
        SetRefusal::UnknownModel {
            setting: "roles.planner.model".into(),
            model: "opus".into(),
            host: Some(Host::Codex),
            checked: vec![(Host::Codex, vec![])],
        }
    );
    let text = refusal.to_string();
    assert!(text.starts_with("unknown-model: "), "{text}");
    assert!(text.contains("roles.planner.model"), "{text}");
    assert!(text.contains("opus"), "{text}");
    assert!(text.contains("codex accepts: none"), "{text}");
    assert!(!text.contains("claude-code"), "{text}");
}

#[test]
fn a_model_one_host_holds_is_not_refused_when_no_host_is_named() {
    assert_eq!(models(&[model("opus")], None), Ok(()));
    // Codex's names hold it and Claude Code's do not: still enough.
    let only_codex = BTreeMap::from([
        (Host::ClaudeCode, names(&[])),
        (Host::Codex, names(&["gpt-x"])),
    ]);
    assert_eq!(judge_models(&[model("gpt-x")], None, &only_codex), Ok(()));
}

#[test]
fn a_model_no_host_holds_is_not_accepted_and_both_hosts_are_named() {
    let refusal = models(&[model("mystery")], None).expect_err("no host holds it");
    let text = refusal.to_string();
    assert!(text.starts_with("unknown-model: "), "{text}");
    assert!(text.contains("mystery"), "{text}");
    assert!(
        text.contains("claude-code accepts: fable, haiku, opus, sonnet"),
        "{text}"
    );
    assert!(text.contains("codex accepts: none"), "{text}");
}

#[test]
fn a_host_missing_from_the_supplied_names_is_not_taken_to_accept_anything() {
    let only_claude = BTreeMap::from([(Host::ClaudeCode, names(&["opus"]))]);
    let refusal = judge_models(&[model("opus")], Some(Host::Codex), &only_claude)
        .expect_err("codex supplied no names");
    assert!(refusal.to_string().contains("codex accepts: none"));
}

#[test]
fn a_model_name_matched_by_prefix_or_case_is_not_accepted() {
    assert!(models(&[model("Opus")], Some(Host::ClaudeCode)).is_err());
    assert!(models(&[model("opu")], Some(Host::ClaudeCode)).is_err());
}

#[test]
fn a_set_with_no_model_pair_is_not_said_to_need_the_catalog() {
    let pairs = [
        pair("roles.planner.effort", None, Value::Rung(Rung::High)),
        pair("example.flag", None, Value::Bool(true)),
    ];
    assert!(!needs_catalog(&pairs));
    assert!(needs_catalog(&[
        pairs[0].clone(),
        model("opus"),
        pairs[1].clone()
    ]));
    // With no model pair, the judge accepts without any names supplied.
    assert_eq!(judge_models(&pairs, None, &BTreeMap::new()), Ok(()));
}

#[test]
fn a_bad_model_overwritten_by_a_good_one_in_the_same_set_is_not_forgiven() {
    let pairs = [model("mystery"), model("opus")];
    let refusal = models(&pairs, None).expect_err("the first value is unknown");
    assert!(refusal.to_string().contains("mystery"));
    // Collapsing first would hide it, which is why the order is fixed.
    assert_eq!(models(&collapse_repeats(pairs.to_vec()), None), Ok(()));
}

#[test]
fn the_first_failing_model_pair_is_not_skipped_for_a_later_one() {
    let refusal = models(
        &[model("opus"), model("first-bad"), model("second-bad")],
        None,
    )
    .expect_err("two unknown models");
    assert!(refusal.to_string().contains("first-bad"));
}

#[test]
fn a_repeated_setting_is_not_kept_at_its_last_position_or_with_its_first_value() {
    let flag = |value| pair("example.flag", None, Value::Bool(value));
    let effort = pair("roles.planner.effort", None, Value::Rung(Rung::Low));
    let collapsed = collapse_repeats(vec![
        flag(true),
        effort.clone(),
        flag(false),
        model("opus"),
        model("sonnet"),
    ]);
    assert_eq!(collapsed, vec![flag(false), effort, model("sonnet")]);
}
