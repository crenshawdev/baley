//! The settings commands' pure half on supplied values. Expected values are
//! written out from design 0003 section 5 and the phase's decisions, never
//! from running this code.

use std::collections::BTreeMap;

use super::*;
use crate::policy::schema::Default as Builtin;
use crate::policy::{
    AcceptedNames, Entry, Fault, FileLayer, Host, Kind, Rung, Schema, Scope, SettingsFile,
    Unavailable, Value, parse_layer,
};

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

fn settings_file(path: &str, text: &str) -> SettingsFile {
    SettingsFile {
        path: path.into(),
        bytes: text.as_bytes().to_vec(),
        digest: format!("digest of {path}"),
    }
}

/// The table a hand-written TOML text holds.
fn table(text: &str) -> toml::Table {
    text.parse().expect("the expected text is TOML")
}

fn rendered(
    layer: FileLayer,
    base: Option<&SettingsFile>,
    pairs: &[TypedPair],
) -> Result<toml::Table, Unavailable> {
    let bytes = render_file(&schema(), layer, base, pairs)?;
    Ok(String::from_utf8(bytes)
        .expect("the file is UTF-8")
        .parse()
        .expect("the file is TOML"))
}

const PROJECT_ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";

#[test]
fn a_rendered_file_that_drops_or_rewrites_another_key_of_the_base_is_caught() {
    let base = settings_file(
        "/r/baley.toml",
        &format!(
            "# a comment the render may lose\n\
             unknown_name = 3\n\
             example.global_only = true\n\
             example.flag = false\n\
             \n\
             [project]\n\
             id = \"{PROJECT_ID}\"\n\
             name = \"demo\"\n\
             \n\
             [host.claude-code.roles.planner]\n\
             effort = \"max\"\n"
        ),
    );
    let pairs = [
        pair("roles.planner.effort", None, Value::Rung(Rung::High)),
        pair("roles.planner.model", None, Value::ModelName("opus".into())),
    ];
    let out = rendered(FileLayer::Project, Some(&base), &pairs).expect("a valid base");
    let expected = table(&format!(
        "unknown_name = 3\n\
         [example]\n\
         global_only = true\n\
         flag = false\n\
         [project]\n\
         id = \"{PROJECT_ID}\"\n\
         name = \"demo\"\n\
         [host.claude-code.roles.planner]\n\
         effort = \"max\"\n\
         [roles.planner]\n\
         effort = \"high\"\n\
         model = \"opus\"\n"
    ));
    assert_eq!(out, expected);
}

#[test]
fn a_host_value_written_at_the_top_level_or_over_the_host_table_is_caught() {
    let base = settings_file(
        "/c/config.toml",
        "roles.planner.effort = \"low\"\n\
         [host.codex]\n\
         example.flag = true\n\
         [host.claude-code.roles.planner]\n\
         model = \"kept\"\n",
    );
    let pairs = [pair(
        "roles.planner.effort",
        Some(Host::Codex),
        Value::Rung(Rung::Xhigh),
    )];
    let out = rendered(FileLayer::Global, Some(&base), &pairs).expect("a valid base");
    let expected = table(
        "[roles.planner]\n\
         effort = \"low\"\n\
         [host.codex]\n\
         example.flag = true\n\
         [host.codex.roles.planner]\n\
         effort = \"xhigh\"\n\
         [host.claude-code.roles.planner]\n\
         model = \"kept\"\n",
    );
    assert_eq!(out, expected);
}

#[test]
fn a_new_file_that_holds_a_value_a_read_would_refuse_or_misplace_is_caught() {
    let pairs = [
        pair(
            "roles.planner.effort",
            Some(Host::Codex),
            Value::Rung(Rung::Max),
        ),
        pair("example.flag", Some(Host::Codex), Value::Bool(true)),
        pair(
            "roles.planner.model",
            Some(Host::Codex),
            Value::ModelName("gpt-x".into()),
        ),
    ];
    let out = rendered(FileLayer::Global, None, &pairs).expect("no base");
    assert_eq!(
        out,
        table(
            "[host.codex.roles.planner]\n\
             effort = \"max\"\n\
             model = \"gpt-x\"\n\
             [host.codex.example]\n\
             flag = true\n"
        )
    );
    // The bytes read back through the parser with each value's type and host.
    let bytes = render_file(&schema(), FileLayer::Global, None, &pairs).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let parsed = parse_layer(
        &settings_file("/c/config.toml", &text),
        FileLayer::Global,
        &schema(),
    )
    .expect("the file reads back");
    let mut read: Vec<_> = parsed
        .values
        .iter()
        .map(|w| (w.name.as_str(), w.host, w.value.clone()))
        .collect();
    read.sort_by(|a, b| a.0.cmp(b.0));
    assert_eq!(
        read,
        vec![
            ("example.flag", Some(Host::Codex), Value::Bool(true)),
            (
                "roles.planner.effort",
                Some(Host::Codex),
                Value::Rung(Rung::Max)
            ),
            (
                "roles.planner.model",
                Some(Host::Codex),
                Value::ModelName("gpt-x".into())
            ),
        ]
    );
}

#[test]
fn a_model_name_with_quotes_and_a_line_break_is_not_written_so_it_reads_back_changed() {
    let name = "odd \"name\"\nsecond line \\ end";
    let pairs = [pair(
        "roles.planner.model",
        None,
        Value::ModelName(name.into()),
    )];
    let bytes = render_file(&schema(), FileLayer::Global, None, &pairs).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let parsed = parse_layer(
        &settings_file("/c/config.toml", &text),
        FileLayer::Global,
        &schema(),
    )
    .expect("the file reads back");
    assert_eq!(parsed.values[0].value, Value::ModelName(name.into()));
}

#[test]
fn a_value_at_a_path_the_base_holds_is_not_added_beside_it_or_allowed_to_drop_its_siblings() {
    let base = settings_file(
        "/c/config.toml",
        "[roles.planner]\neffort = \"low\"\nmodel = \"keep\"\n",
    );
    let pairs = [pair("roles.planner.effort", None, Value::Rung(Rung::Max))];
    let out = rendered(FileLayer::Global, Some(&base), &pairs).expect("a valid base");
    assert_eq!(
        out,
        table("[roles.planner]\neffort = \"max\"\nmodel = \"keep\"\n")
    );
}

#[test]
fn a_base_whose_schema_prefix_is_not_a_table_is_not_overwritten() {
    let base = settings_file("/r/baley.toml", "roles = 5\n");
    let pairs = [pair("roles.planner.effort", None, Value::Rung(Rung::High))];
    let refusal = render_file(&schema(), FileLayer::Project, Some(&base), &pairs)
        .expect_err("roles is an integer");
    assert_eq!(refusal.code(), "config-unavailable");
    assert_eq!(refusal.path, std::path::PathBuf::from("/r/baley.toml"));
    assert_eq!(
        refusal.fault,
        Fault::WrongType {
            name: "roles".into(),
            expected: crate::policy::Expected::Table,
            found: "integer",
            line: 1,
            column: 9,
        }
    );
}

#[test]
fn a_base_that_is_not_toml_is_not_replaced_by_a_fresh_file() {
    let base = settings_file("/c/config.toml", "this is not toml\n");
    let pairs = [pair("example.flag", None, Value::Bool(true))];
    let refusal = render_file(&schema(), FileLayer::Global, Some(&base), &pairs)
        .expect_err("the base does not parse");
    assert!(matches!(refusal.fault, Fault::Parse { .. }), "{refusal:?}");
    assert_eq!(refusal.path, std::path::PathBuf::from("/c/config.toml"));
}

#[test]
fn a_base_holding_an_invalid_value_is_not_repaired_by_the_set_that_replaces_it() {
    let base = settings_file("/c/config.toml", "roles.planner.effort = \"HIGH\"\n");
    let pairs = [pair("roles.planner.effort", None, Value::Rung(Rung::High))];
    let refusal = render_file(&schema(), FileLayer::Global, Some(&base), &pairs)
        .expect_err("the base holds an invalid rung");
    assert!(
        matches!(refusal.fault, Fault::OutsideGrammar { .. }),
        "{refusal:?}"
    );
}

fn parsed(layer: FileLayer, text: &str) -> crate::policy::ParsedLayer {
    parse_layer(&settings_file("/c/config.toml", text), layer, &schema()).expect("a valid file")
}

const HELD_FILE: &str = "example.flag = true\n\
                         roles.planner.effort = \"high\"\n\
                         [host.codex.roles.planner]\n\
                         model = \"gpt-x\"\n";

#[test]
fn a_set_whose_every_pair_the_file_holds_is_not_a_change() {
    let file = parsed(FileLayer::Global, HELD_FILE);
    let pairs = [
        pair("example.flag", None, Value::Bool(true)),
        pair("roles.planner.effort", None, Value::Rung(Rung::High)),
        pair(
            "roles.planner.model",
            Some(Host::Codex),
            Value::ModelName("gpt-x".into()),
        ),
    ];
    assert_eq!(changed_pairs(Some(&file), &pairs), vec![]);
}

#[test]
fn a_value_held_at_the_other_host_level_is_not_taken_as_held() {
    let file = parsed(FileLayer::Global, HELD_FILE);
    // Held at the top level, set under a host section.
    let under_host = pair("example.flag", Some(Host::Codex), Value::Bool(true));
    assert_eq!(
        changed_pairs(Some(&file), std::slice::from_ref(&under_host)),
        vec![under_host]
    );
    // Held in a host section, set at the top level.
    let top = pair(
        "roles.planner.model",
        None,
        Value::ModelName("gpt-x".into()),
    );
    assert_eq!(
        changed_pairs(Some(&file), std::slice::from_ref(&top)),
        vec![top]
    );
    // Held in codex's section, set in claude-code's.
    let other = pair(
        "roles.planner.model",
        Some(Host::ClaudeCode),
        Value::ModelName("gpt-x".into()),
    );
    assert_eq!(
        changed_pairs(Some(&file), std::slice::from_ref(&other)),
        vec![other]
    );
}

#[test]
fn a_different_value_at_a_held_path_is_not_taken_as_held() {
    let file = parsed(FileLayer::Global, HELD_FILE);
    let pairs = [pair("roles.planner.effort", None, Value::Rung(Rung::Low))];
    assert_eq!(changed_pairs(Some(&file), &pairs), pairs.to_vec());
}

#[test]
fn a_set_against_no_file_is_not_a_no_op() {
    let pairs = [
        pair("example.flag", None, Value::Bool(false)),
        pair("roles.planner.effort", None, Value::Rung(Rung::High)),
    ];
    assert_eq!(changed_pairs(None, &pairs), pairs.to_vec());
}

#[test]
fn a_mixed_set_does_not_report_a_held_pair_or_lose_the_order_of_the_changed_ones() {
    let file = parsed(FileLayer::Global, HELD_FILE);
    let changed_a = pair("roles.planner.effort", None, Value::Rung(Rung::Max));
    let held = pair("example.flag", None, Value::Bool(true));
    let changed_b = pair(
        "roles.planner.model",
        Some(Host::Codex),
        Value::ModelName("other".into()),
    );
    let pairs = [changed_b.clone(), held, changed_a.clone()];
    assert_eq!(
        changed_pairs(Some(&file), &pairs),
        vec![changed_b, changed_a]
    );
}

#[test]
fn a_change_in_a_ledgered_project_is_not_written_without_the_step_or_the_steps_version() {
    assert_eq!(
        choose_outcome(true, Place::LedgeredProject),
        SetOutcome {
            write: true,
            run_step: true,
            version: VersionSource::Step,
        }
    );
}

#[test]
fn a_change_in_a_project_not_in_the_ledger_is_not_given_a_step() {
    assert_eq!(
        choose_outcome(true, Place::ProjectNotInLedger),
        SetOutcome {
            write: true,
            run_step: false,
            version: VersionSource::NotInLedger,
        }
    );
}

#[test]
fn a_change_outside_a_project_is_not_given_a_step() {
    assert_eq!(
        choose_outcome(true, Place::OutsideProject),
        SetOutcome {
            write: true,
            run_step: false,
            version: VersionSource::OutsideProject,
        }
    );
}

#[test]
fn a_no_op_in_a_ledgered_project_is_not_written_stepped_or_printed_as_version_zero() {
    assert_eq!(
        choose_outcome(false, Place::LedgeredProject),
        SetOutcome {
            write: false,
            run_step: false,
            version: VersionSource::Stored,
        }
    );
}

#[test]
fn a_no_op_in_a_project_not_in_the_ledger_is_not_written_or_stepped() {
    assert_eq!(
        choose_outcome(false, Place::ProjectNotInLedger),
        SetOutcome {
            write: false,
            run_step: false,
            version: VersionSource::NotInLedger,
        }
    );
}

#[test]
fn a_no_op_outside_a_project_is_not_written_or_stepped() {
    assert_eq!(
        choose_outcome(false, Place::OutsideProject),
        SetOutcome {
            write: false,
            run_step: false,
            version: VersionSource::OutsideProject,
        }
    );
}
