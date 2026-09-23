use super::*;

#[test]
fn search_answers_rows_without_bodies() {
    let files = [
        FileHits {
            path: "/x/notes.txt".into(),
            revision: "notes-revision".into(),
            content: format!("needle here\nneedle{}\n", "x".repeat(294)),
            lines: vec![1, 2],
        },
        FileHits {
            path: "/x/src/a.rs".into(),
            revision: "rust-revision".into(),
            content: "fn alpha() {\n    let needle = 1;\n    needle\n}\n\n// needle outside\n".into(),
            lines: vec![2, 3, 6],
        },
    ];
    let answer = plan_answer(&files, Duration::from_secs(5), &mut || Duration::ZERO);
    let rows = rows(&files, &answer, Path::new("/x"));
    let expected = [
        (json!({"file":"notes.txt","line":1,"text":"needle here","name":"(no enclosing unit)","kind":"window"}), [1, 2]),
        (json!({"file":"notes.txt","line":2,"text":format!("needle{}…", "x".repeat(194)),"name":"(no enclosing unit)","kind":"window"}), [1, 2]),
        (json!({"file":"src/a.rs","line":2,"text":"    let needle = 1;","name":"alpha","kind":"function"}), [1, 4]),
        (json!({"file":"src/a.rs","line":3,"text":"    needle","name":"alpha","kind":"function"}), [1, 4]),
        (json!({"file":"src/a.rs","line":6,"text":"// needle outside","name":"(no enclosing unit)","kind":"window"}), [4, 6]),
    ];
    assert_eq!(rows.len(), 5);
    for (row, (value, range)) in rows.iter().zip(expected) {
        assert_eq!(serde_json::to_value(row).unwrap(), value);
        assert_eq!(row.target.range(), range);
    }
}
