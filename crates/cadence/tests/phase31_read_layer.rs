#[path = "support/phase31.rs"]
mod phase31;

use phase31::{Client, Fixture};
use serde_json::json;
use std::fs;

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

#[test]
fn phase31_unissued_location_is_refused() {
    let fixture = Fixture::new();
    let mut client = Client::open(fixture.project());
    let search = client.call("cadence_query", json!({"operation":"search","pattern":"needle",
        "scope":{"kind":"directory","selector":"src"}}));
    let location = search["hits"].as_array().unwrap().iter()
        .find(|hit| hit["name"] == "beta").unwrap()["location"].as_str().unwrap().to_owned();
    let valid = client.call("cadence_query", json!({"operation":"read","location":location}));
    assert_eq!(valid["status"], "ok", "{valid}");
    assert_eq!(valid["kind"], "slice");
    assert_eq!(valid["body"], "fn beta() {\n    let needle = 3;\n}\n");
    for token in ["unissued-opaque-token", "/etc/passwd", "../outside", "loc-altered"] {
        let answer = client.call("cadence_query", json!({"operation":"read","location":token}));
        assert_eq!(answer["status"], "refused", "{answer}");
        assert_eq!(answer["code"], "location-not-issued", "{answer}");
        assert_eq!(answer["rule"], "D-147", "{answer}");
        assert_eq!(answer["slot"], "location", "{answer}");
        assert!(answer.get("body").is_none(), "{answer}");
    }
    let malformed = client.call("cadence_query", json!({"operation":"read","location":location,
        "path":"src/units.rs","start":1,"end":8}));
    assert_eq!(malformed["status"], "refused", "{malformed}");
    assert_eq!(malformed["code"], "read-contract", "{malformed}");
    let other_fixture = Fixture::new();
    let mut other = Client::open(other_fixture.project());
    let foreign = other.call("cadence_query", json!({"operation":"read","location":location}));
    assert_eq!(foreign["code"], "location-not-issued", "{foreign}");
    other.finish();
    let first = client.call("cadence_query", json!({"operation":"search","pattern":"beta",
        "scope":{"kind":"directory","selector":"src"}}))["hits"][0]["location"].as_str().unwrap().to_owned();
    for _ in 0..65 {
        let answer = client.call("cadence_query", json!({"operation":"search","pattern":"alpha",
            "scope":{"kind":"directory","selector":"src"}}));
        assert_eq!(answer["status"], "ok", "{answer}");
    }
    let expired = client.call("cadence_query", json!({"operation":"read","location":first}));
    assert_eq!(expired["code"], "location-not-issued", "{expired}");
    client.finish();
}

#[test]
fn phase31_read_returns_exact_slice_and_continuation() {
    let fixture = Fixture::new();
    let long_line = format!("    // first-marker {} last-marker\\n", "é".repeat(40_000));
    let oversized = format!("fn oversized() {{\\n{long_line}}}\\n");
    fs::write(fixture.path("src/oversized.rs"), &oversized).unwrap();
    let mut client = Client::open(fixture.project());

    let search = client.call("cadence_query", json!({"operation":"search","pattern":"first-marker",
        "scope":{"kind":"directory","selector":"src"}}));
    let location = search["hits"].as_array().unwrap().iter()
        .find(|hit| hit["name"] == "oversized").unwrap()["location"].as_str().unwrap().to_owned();

    let mut page = client.call("cadence_query", json!({"operation":"read","location":location}));
    assert_eq!(page["status"], "ok", "{page}");
    assert_eq!(page["kind"], "slice");
    assert_eq!(page["truncated"], true, "{page}");
    assert!(page["continuation"].as_str().is_some_and(|value| !value.is_empty()), "{page}");
    assert!(page["continue_from_byte"].as_u64().is_some_and(|byte| byte > 0), "{page}");

    let mut served = String::new();
    loop {
        served.push_str(page["body"].as_str().unwrap());
        let Some(next) = page["continuation"].as_str() else { break };
        page = client.call("cadence_query", json!({"operation":"read","location":next}));
        assert_eq!(page["status"], "ok", "{page}");
    }
    assert_eq!(served, oversized);
    assert_eq!(page["truncated"], false);
    assert!(page["continuation"].is_null());

    let beta = client.call("cadence_query", json!({"operation":"search","pattern":"fn beta",
        "scope":{"kind":"directory","selector":"src"}}))["hits"][0]["location"].as_str().unwrap().to_owned();
    let short = client.call("cadence_query", json!({"operation":"read","location":beta}));
    assert_eq!(short["body"], "fn beta() {\n    let needle = 3;\n}\n");
    client.finish();
}

#[test]
fn phase31_large_file_returns_unit_outline() {
    let fixture = Fixture::new();
    let padding = "// outline padding\n".repeat(2_000);
    let large = format!(
        "fn first_unit() {{\n    let outline_needle = 1;\n}}\n{padding}fn second_unit() {{\n    let outline_needle = 2;\n}}\n"
    );
    assert!(large.len() > 24 * 1024);
    fs::write(fixture.path("src/outline.rs"), large).unwrap();
    let mut client = Client::open(fixture.project());

    let search = client.call("cadence_query", json!({"operation":"search","pattern":"outline_needle",
        "scope":{"kind":"directory","selector":"src"}}));
    let file = search["hits"].as_array().unwrap().iter()
        .find(|hit| hit["name"] == "first_unit").unwrap()["file_reference"].as_str().unwrap().to_owned();
    let outline = client.call("cadence_query", json!({"operation":"read","file":file}));
    assert_eq!(outline["status"], "ok", "{outline}");
    assert_eq!(outline["kind"], "outline");
    assert!(outline.get("body").is_none(), "{outline}");
    let rows = outline["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{outline}");
    assert_eq!(rows[0]["name"], "first_unit");
    assert_eq!(rows[0]["range"], json!([1, 3]));
    assert!(rows.iter().all(|row| row["location"].as_str().is_some_and(|value| !value.is_empty())));

    let selected = client.call("cadence_query", json!({"operation":"read","file":file,"unit":"second_unit"}));
    assert_eq!(selected["kind"], "slice");
    assert_eq!(selected["body"], "fn second_unit() {\n    let outline_needle = 2;\n}\n");
    let missing = client.call("cadence_query", json!({"operation":"read","file":file,"unit":"missing_unit"}));
    assert_eq!(missing["kind"], "outline");
    assert_eq!(missing["reason"], "missing-unit");
    assert_eq!(missing["rows"].as_array().unwrap().len(), 2, "{missing}");
    client.finish();
}
