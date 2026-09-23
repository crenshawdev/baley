use super::*;

fn files() -> Vec<(PathBuf, String)> {
    vec![
        ("/x/docs/guide.md".into(), "# Guide\n## Default settings\ntext\n".into()),
        ("/x/src/limits.rs".into(), concat!(
            "struct Limits;\n",
            "impl Default for Limits {\n",
            "    fn default() -> Self { Limits }\n",
            "}\n",
            "fn defaults_off() {}\n",
            "fn other() {}\n",
        ).into()),
        ("/x/web/app.js".into(), "class Config {\n  defaultValue() { return 1; }\n}\nfunction other() {}\n".into()),
    ]
}

#[test]
fn symbol_search_matches_qualified_names() {
    let files = files();
    for (name, case_insensitive, expected) in [
        ("default", false, vec![
            ("src/limits.rs", "Limits::default", "function", [3, 3]),
            ("src/limits.rs", "defaults_off", "function", [5, 5]),
            ("web/app.js", "Config.defaultValue", "method", [2, 2]),
        ]),
        ("default", true, vec![
            ("docs/guide.md", "# Guide > ## Default settings", "heading", [2, 3]),
            ("src/limits.rs", "impl Default for Limits", "impl", [2, 4]),
            ("src/limits.rs", "Limits::default", "function", [3, 3]),
            ("src/limits.rs", "defaults_off", "function", [5, 5]),
            ("web/app.js", "Config.defaultValue", "method", [2, 2]),
        ]),
        ("Limits::def", false, vec![
            ("src/limits.rs", "Limits::default", "function", [3, 3]),
        ]),
    ] {
        let result = scan(&files, name, case_insensitive, (0, 0), 50,
            Duration::from_secs(5), &mut || Duration::ZERO);
        let rows: Vec<_> = result.rows.iter().map(|(file, _, unit)| (
            files[*file].0.strip_prefix("/x").unwrap().to_str().unwrap(),
            unit.name.as_str(), unit.kind, unit.range(),
        )).collect();
        assert_eq!(rows, expected, "name={name}, case_insensitive={case_insensitive}");
        assert!(result.unreached.is_empty());
    }
}

#[test]
fn files_past_a_spent_budget_are_named_not_searched() {
    let result = scan(&files(), "default", false, (0, 0), 50,
        Duration::ZERO, &mut || Duration::ZERO);
    assert!(result.rows.is_empty());
    assert_eq!(result.unreached, vec![0, 1, 2]);
}

#[test]
fn a_scan_resumes_at_its_cursor_ordinal() {
    let files = vec![files().remove(1)];
    let result = scan(&files, "default", false, (0, 1), 50,
        Duration::from_secs(5), &mut || Duration::ZERO);
    let rows: Vec<_> = result.rows.iter().map(|(file, ordinal, unit)|
        (*file, *ordinal, unit.name.as_str())).collect();
    assert_eq!(rows, vec![(0, 1, "defaults_off")]);
    assert!(result.unreached.is_empty());
}
