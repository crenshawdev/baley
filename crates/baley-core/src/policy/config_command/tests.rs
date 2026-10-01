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

/// One setting of each kind and each scope, so the judges are tested apart
/// from the standard schema.
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
            "example.remote",
            Kind::RemoteName,
            Builtin::Absent,
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
fn an_unknown_model_refusal_that_garbles_its_sentence_is_caught() {
    let named = models(&[model("opus")], Some(Host::Codex)).expect_err("codex lacks opus");
    assert_eq!(
        named.to_string(),
        "unknown-model: roles.planner.model is \"opus\", which the codex catalog does not \
         accept; codex accepts: none"
    );
    let unnamed = models(&[model("mystery")], None).expect_err("no host holds it");
    assert_eq!(
        unnamed.to_string(),
        "unknown-model: roles.planner.model is \"mystery\", which no host's catalog accepts; \
         claude-code accepts: fable, haiku, opus, sonnet; codex accepts: none"
    );
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

fn changed_effort() -> TypedPair {
    pair("roles.planner.effort", None, Value::Rung(Rung::High))
}

fn changed_flag() -> TypedPair {
    pair("example.flag", None, Value::Bool(true))
}

fn set_lines(
    layer: FileLayer,
    path: &str,
    changed: &[TypedPair],
    place: Place,
    version: u64,
) -> Vec<String> {
    let outcome = choose_outcome(!changed.is_empty(), place);
    render_set(
        layer,
        std::path::Path::new(path),
        changed,
        &outcome,
        version,
    )
}

fn line_with<'l>(lines: &'l [String], needle: &str) -> &'l String {
    lines
        .iter()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line holds {needle:?} in {lines:?}"))
}

#[test]
fn a_project_file_set_output_missing_its_file_settings_commit_note_or_version_is_caught() {
    let lines = set_lines(
        FileLayer::Project,
        "/r/baley.toml",
        &[
            changed_effort(),
            pair(
                "roles.planner.model",
                Some(Host::Codex),
                Value::ModelName("gpt-x".into()),
            ),
        ],
        Place::LedgeredProject,
        7,
    );
    assert_eq!(
        lines,
        vec![
            "wrote /r/baley.toml",
            "set roles.planner.effort = \"high\"",
            "set [host.codex] roles.planner.model = \"gpt-x\"",
            "The change applies at the next commit.",
            "policy version 7",
        ]
    );
}

#[test]
fn a_global_set_output_that_says_commit_or_misses_its_settings_file_or_version_is_caught() {
    let lines = set_lines(
        FileLayer::Global,
        "/c/config.toml",
        &[changed_effort(), changed_flag()],
        Place::LedgeredProject,
        7,
    );
    assert_eq!(
        lines,
        vec![
            "set roles.planner.effort = \"high\"",
            "set example.flag = true",
            "wrote /c/config.toml",
            "policy version 7",
        ]
    );
    assert!(
        lines.iter().all(|line| !line.contains("commit")),
        "{lines:?}"
    );
}

#[test]
fn a_no_op_output_that_claims_a_change_or_mentions_commit_is_caught() {
    for (layer, path) in [
        (FileLayer::Project, "/r/baley.toml"),
        (FileLayer::Global, "/c/config.toml"),
    ] {
        let lines = set_lines(layer, path, &[], Place::LedgeredProject, 4);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("nothing changed"), "{lines:?}");
        assert!(lines[0].contains(path), "{lines:?}");
        assert_eq!(lines[1], "policy version 4");
        assert!(
            lines.iter().all(|line| !line.contains("commit")),
            "{lines:?}"
        );
        assert!(
            lines.iter().all(|line| !line.contains("wrote")),
            "{lines:?}"
        );
    }
}

#[test]
fn a_version_zero_output_outside_a_project_that_drops_its_meaning_is_caught() {
    let lines = set_lines(
        FileLayer::Global,
        "/c/config.toml",
        &[changed_flag()],
        Place::OutsideProject,
        0,
    );
    let version = line_with(&lines, "policy version 0");
    assert!(
        version.contains("no recorded policy applies outside a project"),
        "{version}"
    );
    assert!(!version.contains("baley init"), "{version}");
}

#[test]
fn a_version_zero_output_for_a_project_not_in_the_ledger_that_does_not_name_init_is_caught() {
    let lines = set_lines(
        FileLayer::Project,
        "/r/baley.toml",
        &[changed_flag()],
        Place::ProjectNotInLedger,
        0,
    );
    let version = line_with(&lines, "policy version 0");
    assert!(
        version.contains("no recorded policy applies to this project on this machine"),
        "{version}"
    );
    assert!(version.contains("baley init records it"), "{version}");
}

#[test]
fn a_ledgered_checkout_with_no_stored_record_is_not_printed_as_a_bare_zero() {
    let lines = set_lines(
        FileLayer::Global,
        "/c/config.toml",
        &[],
        Place::LedgeredProject,
        0,
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("policy version 0: no recorded policy applies")
    );
}

#[test]
fn a_nonzero_version_is_not_followed_by_a_meaning() {
    for place in [
        Place::OutsideProject,
        Place::ProjectNotInLedger,
        Place::LedgeredProject,
    ] {
        let lines = set_lines(
            FileLayer::Global,
            "/c/config.toml",
            &[changed_flag()],
            place,
            3,
        );
        assert_eq!(lines.last().map(String::as_str), Some("policy version 3"));
    }
}

#[test]
fn a_model_name_with_a_line_break_is_not_printed_across_lines() {
    let lines = set_lines(
        FileLayer::Global,
        "/c/config.toml",
        &[pair(
            "roles.planner.model",
            None,
            Value::ModelName("a\nb \"c\"".into()),
        )],
        Place::LedgeredProject,
        1,
    );
    assert_eq!(lines[0], "set roles.planner.model = \"a\\nb \\\"c\\\"\"");
    assert!(lines.iter().all(|line| !line.contains('\n')), "{lines:?}");
}

fn read(path: &str, text: &str) -> Result<Option<SettingsFile>, Unavailable> {
    Ok(Some(settings_file(path, text)))
}

fn unreadable(path: &str, cause: &str) -> Result<Option<SettingsFile>, Unavailable> {
    Err(Unavailable {
        path: path.into(),
        fault: Fault::Unreadable {
            cause: cause.into(),
        },
    })
}

const NONE: Result<Option<SettingsFile>, Unavailable> = Ok(None);

fn show(
    names: &[&str],
    in_project: bool,
    global: Result<Option<SettingsFile>, Unavailable>,
    working: Result<Option<SettingsFile>, Unavailable>,
    head: Result<Option<SettingsFile>, Unavailable>,
) -> Result<ShowLayers, ShowRefusal> {
    judge_show(&schema(), names, in_project, global, working, head)
}

#[test]
fn a_project_scoped_name_asked_outside_a_project_is_not_shown() {
    let refusal =
        show(&["example.project_only"], false, NONE, NONE, NONE).expect_err("outside a project");
    assert_eq!(
        refusal,
        ShowRefusal::NotAProject {
            name: "example.project_only".into()
        }
    );
    assert_eq!(refusal.code(), "not-a-project");
    let text = refusal.to_string();
    assert!(text.starts_with("not-a-project: "), "{text}");
    assert!(text.contains("example.project_only"), "{text}");
}

#[test]
fn a_project_scoped_name_inside_a_project_or_no_names_outside_one_are_not_refused() {
    assert!(show(&["example.project_only"], true, NONE, NONE, NONE).is_ok());
    assert!(show(&[], false, NONE, NONE, NONE).is_ok());
    // A both-scoped name needs no project.
    assert!(show(&["example.flag"], false, NONE, NONE, NONE).is_ok());
}

#[test]
fn a_name_not_in_the_schema_is_not_shown_and_is_named_before_a_project_scoped_one() {
    let refusal = show(
        &["example.project_only", "nonsense"],
        false,
        NONE,
        NONE,
        NONE,
    )
    .expect_err("an unknown name");
    assert_eq!(
        refusal,
        ShowRefusal::UnknownSetting {
            name: "nonsense".into()
        }
    );
    assert_eq!(refusal.code(), "unknown-setting");
    let text = refusal.to_string();
    assert!(text.starts_with("unknown-setting: "), "{text}");
    assert!(text.contains("nonsense"), "{text}");
}

#[test]
fn an_invalid_global_working_tree_or_head_file_is_not_shown_and_is_named_with_its_line() {
    let bad = "example.flag = 3\n";
    let cases = [
        (
            show(&[], true, read("/c/config.toml", bad), NONE, NONE),
            "/c/config.toml:1:16",
        ),
        (
            show(&[], true, NONE, read("/r/baley.toml", bad), NONE),
            "/r/baley.toml:1:16",
        ),
        (
            show(&[], true, NONE, NONE, read("/r/baley.toml", bad)),
            "/r/baley.toml:1:16",
        ),
    ];
    for (result, place) in cases {
        let refusal = result.expect_err(place);
        assert_eq!(refusal.code(), "config-unavailable");
        let text = refusal.to_string();
        assert!(text.starts_with("config-unavailable: "), "{text}");
        assert!(text.contains(place), "{text}");
        assert!(
            text.contains("example.flag is an integer, not a boolean"),
            "{text}"
        );
    }
}

#[test]
fn a_fault_in_heads_copy_is_not_named_as_the_working_tree_files() {
    // HEAD's copy carries the working-tree path, so only the label tells the
    // owner the fault is not in the file on disk.
    let bad = "example.flag = 3\n";
    let head = show(&[], true, NONE, NONE, read("/r/baley.toml", bad)).expect_err("HEAD's copy");
    assert_eq!(
        head.to_string(),
        "config-unavailable: HEAD's copy of /r/baley.toml:1:16: example.flag is an integer, \
         not a boolean"
    );
    let working =
        show(&[], true, NONE, read("/r/baley.toml", bad), NONE).expect_err("the working tree");
    assert_eq!(
        working.to_string(),
        "config-unavailable: /r/baley.toml:1:16: example.flag is an integer, not a boolean"
    );
}

#[test]
fn an_unreadable_head_copy_is_not_shown_and_is_named_with_its_cause() {
    let refusal = show(
        &[],
        true,
        NONE,
        NONE,
        unreadable("/r/baley.toml", "permission denied"),
    )
    .expect_err("an unreadable HEAD copy");
    assert_eq!(refusal.code(), "config-unavailable");
    let text = refusal.to_string();
    assert!(
        text.contains("cannot read /r/baley.toml: permission denied"),
        "{text}"
    );
}

#[test]
fn an_unreadable_head_is_not_passed_over_for_an_invalid_global_file() {
    let refusal = show(
        &[],
        true,
        read("/c/config.toml", "example.flag = 3\n"),
        NONE,
        unreadable("/r/baley.toml", "permission denied"),
    )
    .expect_err("two faults");
    assert!(refusal.to_string().contains("cannot read /r/baley.toml"));
}

#[test]
fn an_unreadable_working_tree_is_not_passed_over_for_an_unreadable_head() {
    let refusal = show(
        &[],
        true,
        NONE,
        unreadable("/r/baley.toml", "working tree"),
        unreadable("/r/baley.toml", "head"),
    )
    .expect_err("two faults");
    assert!(refusal.to_string().contains("working tree"));
}

#[test]
fn an_invalid_global_file_is_not_passed_over_for_an_invalid_working_tree_file() {
    let refusal = show(
        &[],
        true,
        read("/c/config.toml", "example.flag = 3\n"),
        read("/r/baley.toml", "example.flag = 4\n"),
        NONE,
    )
    .expect_err("two faults");
    assert!(refusal.to_string().contains("/c/config.toml:1:16"));
}

#[test]
fn a_valid_set_of_files_is_not_returned_unparsed_or_under_the_wrong_layer() {
    let layers = show(
        &[],
        true,
        read("/c/config.toml", "example.flag = true\n"),
        read("/r/baley.toml", "roles.planner.effort = \"low\"\n"),
        read("/r/baley.toml", "roles.planner.effort = \"max\"\n"),
    )
    .expect("valid files");
    let global = layers.global.expect("global");
    assert_eq!(global.layer, FileLayer::Global);
    assert_eq!(global.values[0].value, Value::Bool(true));
    let working = layers.working.expect("working tree");
    assert_eq!(working.layer, FileLayer::Project);
    assert_eq!(working.values[0].value, Value::Rung(Rung::Low));
    let head = layers.head.expect("head");
    assert_eq!(head.layer, FileLayer::Project);
    assert_eq!(head.values[0].value, Value::Rung(Rung::Max));
}

#[test]
fn a_global_only_setting_in_the_project_file_is_a_diagnostic_not_a_refusal_when_shown() {
    // A wrongly scoped name is ignored with a diagnostic, never refused.
    let layers = show(
        &[],
        true,
        NONE,
        read("/r/baley.toml", "example.global_only = true\n"),
        NONE,
    )
    .expect("valid file");
    assert_eq!(layers.working.expect("working tree").diagnostics.len(), 1);
}

fn layer_at(layer: FileLayer, path: &str, text: &str) -> crate::policy::ParsedLayer {
    parse_layer(&settings_file(path, text), layer, &schema()).expect("a valid file")
}

fn global(text: &str) -> Option<crate::policy::ParsedLayer> {
    Some(layer_at(FileLayer::Global, "/c/config.toml", text))
}

fn project(text: &str) -> Option<crate::policy::ParsedLayer> {
    Some(layer_at(FileLayer::Project, "/r/baley.toml", text))
}

/// The report in a project at `/r`, with the global file at `/c/config.toml`.
fn report(
    host: Option<Host>,
    names: &[&str],
    layers: &ShowLayers,
    pending: Option<&str>,
) -> Vec<String> {
    render_show(&ShowRequest {
        schema: &schema(),
        host,
        names,
        layers,
        pending,
        global_path: std::path::Path::new("/c/config.toml"),
        project_path: Some(std::path::Path::new("/r/baley.toml")),
    })
}

const PENDING: &str = "baley.toml has changes that are not in HEAD, which apply once committed";

#[test]
fn a_report_that_takes_the_effective_value_from_the_working_tree_or_hides_heads_is_caught() {
    let layers = ShowLayers {
        global: None,
        working: project("roles.planner.effort = \"medium\"\n"),
        head: project("roles.planner.effort = \"high\"\n"),
    };
    let lines = report(None, &["roles.planner.effort"], &layers, Some(PENDING));
    assert_eq!(
        lines,
        vec![
            "roles.planner.effort: kind rung, default \"high\", scope both",
            "  global: not set",
            "  project: \"medium\" (HEAD has \"high\", applies once committed)",
            "  effective: \"high\" from project (/r/baley.toml)",
            PENDING,
            "global file: /c/config.toml",
            "project file: /r/baley.toml",
        ]
    );
}

#[test]
fn a_report_that_shows_heads_value_when_it_equals_the_working_trees_is_caught() {
    let same = "roles.planner.effort = \"low\"\n";
    let layers = ShowLayers {
        global: None,
        working: project(same),
        head: project(same),
    };
    let lines = report(None, &["roles.planner.effort"], &layers, None);
    assert_eq!(
        lines,
        vec![
            "roles.planner.effort: kind rung, default \"high\", scope both",
            "  global: not set",
            "  project: \"low\"",
            "  effective: \"low\" from project (/r/baley.toml)",
            "global file: /c/config.toml",
            "project file: /r/baley.toml",
        ]
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("applies once committed"))
    );
}

#[test]
fn a_value_only_in_the_working_tree_or_only_in_head_is_not_shown_as_settled() {
    let only_working = ShowLayers {
        global: None,
        working: project("roles.planner.effort = \"low\"\n"),
        head: None,
    };
    let lines = report(None, &["roles.planner.effort"], &only_working, None);
    assert_eq!(
        lines[2],
        "  project: \"low\" (HEAD has no value, applies once committed)"
    );
    assert_eq!(lines[3], "  effective: \"high\" from default");

    let only_head = ShowLayers {
        global: None,
        working: project("example.flag = true\n"),
        head: project("roles.planner.effort = \"max\"\n"),
    };
    let lines = report(None, &["roles.planner.effort"], &only_head, None);
    assert_eq!(
        lines[2],
        "  project: not set (HEAD has \"max\", applies once committed)"
    );
    assert_eq!(
        lines[3],
        "  effective: \"max\" from project (/r/baley.toml)"
    );
}

#[test]
fn a_host_section_value_is_not_reported_from_the_wrong_layer_when_a_host_is_given() {
    let layers = ShowLayers {
        global: global("roles.planner.effort = \"low\"\n"),
        working: None,
        head: project("[host.claude-code.roles.planner]\neffort = \"max\"\n"),
    };
    let lines = report(
        Some(Host::ClaudeCode),
        &["roles.planner.effort"],
        &layers,
        None,
    );
    assert!(
        lines.contains(&"  effective: \"max\" from project-host (/r/baley.toml)".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"  global: \"low\"".to_owned()), "{lines:?}");
}

#[test]
fn a_report_with_no_host_that_leaves_out_a_host_section_or_applies_one_is_caught() {
    let layers = ShowLayers {
        global: global(
            "[host.codex.roles.planner]\neffort = \"low\"\n\
             [host.claude-code.roles.planner]\neffort = \"max\"\n",
        ),
        working: None,
        head: None,
    };
    let lines = report(None, &["roles.planner.effort"], &layers, None);
    assert_eq!(
        &lines[..5],
        [
            "roles.planner.effort: kind rung, default \"high\", scope both",
            "  global [host.claude-code]: \"max\"",
            "  global [host.codex]: \"low\"",
            "  project: not set",
            "  effective: \"high\" from default",
        ]
    );
    // With a host given, the other host's section is not listed.
    let lines = report(Some(Host::Codex), &["roles.planner.effort"], &layers, None);
    assert!(
        lines.iter().all(|line| !line.contains("claude-code")),
        "{lines:?}"
    );
    assert!(lines.contains(&"  global [host.codex]: \"low\"".to_owned()));
    assert!(lines.contains(&"  effective: \"low\" from global-host (/c/config.toml)".to_owned()));
}

#[test]
fn a_settings_line_missing_its_kind_default_or_scope_is_caught() {
    let layers = ShowLayers {
        global: None,
        working: None,
        head: None,
    };
    let lines = report(None, &[], &layers, None);
    let headers: Vec<&str> = lines
        .iter()
        .filter(|line| line.contains(": kind "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        headers,
        [
            "roles.planner.model: kind model name, default absent, scope both",
            "roles.planner.effort: kind rung, default \"high\", scope both",
            "example.flag: kind boolean, default false, scope both",
            "example.remote: kind remote name, default absent, scope both",
            "example.project_only: kind boolean, default false, scope project",
            "example.global_only: kind boolean, default false, scope global",
        ]
    );
    assert!(lines.contains(&"  effective: absent from default".to_owned()));
}

#[test]
fn a_global_value_is_not_reported_without_its_layer_and_file() {
    let layers = ShowLayers {
        global: global("example.flag = true\n"),
        working: None,
        head: None,
    };
    let lines = report(None, &["example.flag"], &layers, None);
    assert_eq!(lines[1], "  global: true");
    assert_eq!(lines[3], "  effective: true from global (/c/config.toml)");
}

#[test]
fn an_ignored_name_in_the_global_or_head_file_is_not_left_out_or_reworded() {
    let layers = ShowLayers {
        global: global("mystery = 1\n"),
        working: project("only_in_the_working_tree = 2\n"),
        head: project("example.global_only = true\n"),
    };
    let lines = report(None, &["example.flag"], &layers, None);
    // Each is the diagnostic's own text, written out from its Display.
    assert!(
        lines.contains(
            &"/c/config.toml:1:1: mystery is not a setting Baley reads and was ignored".to_owned()
        ),
        "{lines:?}"
    );
    assert!(
        lines.contains(
            &"/r/baley.toml:1:9: example.global_only is a global setting and was ignored in the project file"
                .to_owned()
        ),
        "{lines:?}"
    );
    // The policy is merged from HEAD's copy, so the working tree's own
    // diagnostics are not part of it.
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("only_in_the_working_tree"))
    );
}

#[test]
fn a_report_outside_a_project_that_prints_a_project_path_is_caught() {
    let layers = ShowLayers {
        global: global("example.flag = true\n"),
        working: None,
        head: None,
    };
    let lines = render_show(&ShowRequest {
        schema: &schema(),
        host: None,
        names: &["example.flag"],
        layers: &layers,
        pending: None,
        global_path: std::path::Path::new("/c/config.toml"),
        project_path: None,
    });
    assert_eq!(
        lines.last().map(String::as_str),
        Some("global file: /c/config.toml")
    );
    assert!(
        lines.iter().all(|line| !line.contains("project file")),
        "{lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("baley.toml")),
        "{lines:?}"
    );
}

#[test]
fn asked_names_that_do_not_limit_or_order_the_settings_shown_are_caught() {
    let layers = ShowLayers {
        global: None,
        working: None,
        head: None,
    };
    let lines = report(
        None,
        &["example.flag", "roles.planner.effort"],
        &layers,
        None,
    );
    let headers: Vec<&str> = lines
        .iter()
        .filter(|line| line.contains(": kind "))
        .map(|line| line.split(':').next().unwrap())
        .collect();
    assert_eq!(headers, ["example.flag", "roles.planner.effort"]);
}

fn remote(name: &str) -> TypedPair {
    pair("example.remote", None, Value::RemoteName(name.to_owned()))
}

#[test]
fn a_remote_name_converted_as_a_model_name_is_caught() {
    let pairs = judge(
        FileLayer::Global,
        false,
        None,
        &[("example.remote", "origin")],
    )
    .expect("a non-empty remote name is accepted");
    assert_eq!(pairs, vec![remote("origin")]);
}

#[test]
fn an_empty_remote_name_that_is_accepted_or_not_named_is_caught() {
    let refusal = refused(FileLayer::Global, false, &[("example.remote", "")]);
    assert_eq!(refusal.code(), "invalid-value");
    let text = refusal.to_string();
    assert!(text.contains("a git remote name"), "{text}");
    assert!(text.contains("non-empty"), "{text}");
}

#[test]
fn a_set_of_only_a_remote_name_that_is_said_to_need_the_catalog_is_caught() {
    assert!(!needs_catalog(&[remote("origin")]));
    // No names supplied: a remote value is not checked against any catalog.
    assert_eq!(
        judge_models(&[remote("origin")], None, &BTreeMap::new()),
        Ok(())
    );
}

#[test]
fn a_remote_name_not_written_as_a_string_that_reads_back_as_one_is_caught() {
    let table = rendered(FileLayer::Global, None, &[remote("origin")]).unwrap();
    assert_eq!(
        table["example"]["remote"],
        toml::Value::String("origin".into())
    );
    let bytes = render_file(&schema(), FileLayer::Global, None, &[remote("a \"b\"")]).unwrap();
    let parsed = parse_layer(
        &settings_file("/c/config.toml", &String::from_utf8(bytes).unwrap()),
        FileLayer::Global,
        &schema(),
    )
    .expect("the file reads back");
    assert_eq!(parsed.values[0].value, Value::RemoteName("a \"b\"".into()));
}

#[test]
fn a_report_that_names_a_remote_names_kind_as_a_model_name_is_caught() {
    let layers = ShowLayers {
        global: None,
        working: None,
        head: None,
    };
    let lines = report(None, &["example.remote"], &layers, None);
    assert_eq!(
        lines[0],
        "example.remote: kind remote name, default absent, scope both"
    );
}

#[test]
fn the_standard_schema_accepting_git_remote_in_the_global_file_is_caught() {
    let refusal = judge_pairs(
        Schema::standard(),
        FileLayer::Global,
        true,
        None,
        &[("git.remote", "origin")],
    )
    .expect_err("git.remote is a project setting");
    assert_eq!(refusal.code(), "wrong-layer");
    let text = refusal.to_string();
    assert!(text.contains("git.remote"), "{text}");
    assert!(text.contains("project"), "{text}");
    assert!(text.contains("--project"), "{text}");
}

#[test]
fn the_standard_schema_not_taking_git_remote_in_the_project_file_without_the_catalog_is_caught() {
    let pairs = judge_pairs(
        Schema::standard(),
        FileLayer::Project,
        true,
        None,
        &[("git.remote", "origin")],
    )
    .expect("the project file may set git.remote");
    assert_eq!(
        pairs,
        vec![pair("git.remote", None, Value::RemoteName("origin".into()))]
    );
    assert!(!needs_catalog(&pairs));
}

#[test]
fn the_standard_schema_accepting_an_empty_git_remote_is_caught() {
    let refusal = judge_pairs(
        Schema::standard(),
        FileLayer::Project,
        true,
        None,
        &[("git.remote", "")],
    )
    .expect_err("an empty remote name is refused");
    assert_eq!(refusal.code(), "invalid-value");
}

#[test]
fn the_standard_schema_showing_git_remote_outside_a_project_is_caught() {
    let refusal = judge_show(Schema::standard(), &["git.remote"], false, NONE, NONE, NONE)
        .expect_err("outside a project");
    assert_eq!(
        refusal,
        ShowRefusal::NotAProject {
            name: "git.remote".into()
        }
    );
}
