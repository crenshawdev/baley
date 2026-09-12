#[path = "support/phase31.rs"]
mod phase31;

use phase31::{Client, Fixture};
use serde_json::json;

#[test]
fn phase31_search_returns_located_units() {
    let fixture = Fixture::new();
    let mut client = Client::open(fixture.project());
    let answer = client.call("cadence_query", json!({
        "operation":"search", "pattern":"needle",
        "scope":{"kind":"directory", "selector":"src"}
    }));
    assert_eq!(answer["status"], "ok", "{answer}");
    assert_eq!(answer["kind"], "search");
    assert_eq!(answer["bound"], 65_536);
    let units = answer["hits"].as_array().unwrap();
    let rust: Vec<_> = units.iter().filter(|unit| unit["file"] == "src/units.rs").collect();
    assert_eq!(rust.len(), 2, "{answer}");
    assert_eq!(rust[0]["name"], "alpha");
    assert_eq!(rust[0]["range"], json!([1, 4]));
    assert_eq!(rust[0]["match_lines"], json!([2, 3]));
    assert_eq!(rust[0]["body"], "fn alpha() {\n    let needle = 1;\n    let needle_again = 2;\n}\n");
    assert_eq!(rust[1]["name"], "beta");
    assert_eq!(rust[1]["range"], json!([6, 8]));
    assert_eq!(rust[1]["match_lines"], json!([7]));
    assert_eq!(rust[1]["body"], "fn beta() {\n    let needle = 3;\n}\n");
    assert!(rust.iter().all(|unit| unit["location"].as_str().is_some_and(|value| !value.is_empty())));
    assert!(units.iter().all(|unit| unit["file"] != "ignored/sentinel.rs"));
    let nested = units.iter().find(|unit| unit["file"] == "src/nested/mod.rs").unwrap();
    assert_eq!(nested["name"], "outer::inner");
    assert_eq!(nested["range"], json!([2, 2]));

    let grammar_answer = client.call("cadence_query", json!({
        "operation":"search", "pattern":"NEEDLE", "case_insensitive":true,
        "scope":{"kind":"glob", "selector":"**/*.{js,md,json,c}"}
    }));
    assert_eq!(grammar_answer["status"], "ok", "{grammar_answer}");
    let names: Vec<_> = grammar_answer["hits"].as_array().unwrap().iter()
        .map(|unit| (unit["file"].as_str().unwrap(), unit["name"].as_str().unwrap())).collect();
    assert_eq!(names, vec![
        ("docs/units.md", "# Markdown unit"),
        ("src/units.c", "c_unit"),
        ("src/units.js", "javascriptUnit"),
        ("src/units.json", "jsonUnit"),
    ]);
    let refusal = client.call("cadence_query", json!({
        "operation":"search", "pattern":"[", "scope":{"kind":"project"}
    }));
    assert_eq!(refusal["status"], "refused");
    assert_eq!(refusal["slot"], "pattern");
    client.finish();
}
