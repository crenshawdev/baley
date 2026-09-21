#[allow(dead_code)]
#[path = "support/serve.rs"]
mod serve;
#[path = "support/support_records.rs"]
mod support_records;

use serve::Client;
use serde_json::{Value, json};
use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

#[test]
fn help_lists_the_installed_skills_from_the_compiled_table() {
    let temp = support_records::fixture();
    let project = temp.path();
    assert!(!project.join("cadence-core/references/COMMANDS.md").exists());
    let before = serve::tree(project);
    let mut client = Client::open(project);
    let all = client.call("cadence_query", json!({"operation":"help"}));
    assert_eq!(all["status"], "ok", "{all}");
    let expected: [(&str, &[&str]); 4] = [
        ("Build spine", &["new-project", "adopt", "context", "plan", "execute", "verify", "progress", "task"]),
        ("Review & quality gates", &["review", "plan-review", "decision-review", "minimalism-review", "debug", "coverage", "docs-verify", "audit"]),
        ("Lifecycle & git", &["land", "milestone", "phase", "undo"]),
        ("Support", &["capture", "config", "help", "pause", "spike", "suggest", "why"]),
    ];
    let clusters = all["clusters"].as_array().expect("help lists clusters");
    assert_eq!(clusters.len(), 4);
    let mut rows = BTreeMap::new();
    for (cluster, (name, commands)) in clusters.iter().zip(expected) {
        assert_eq!(cluster["name"], name);
        let actual = cluster["commands"].as_array().unwrap();
        assert_eq!(actual.iter().map(|row| row["name"].as_str().unwrap()).collect::<BTreeSet<_>>(),
            commands.iter().map(|name| format!("cad-{name}")).collect::<BTreeSet<_>>()
                .iter().map(String::as_str).collect::<BTreeSet<_>>());
        assert_eq!(actual.len(), commands.len());
        for row in actual {
            assert_eq!(row["cluster"], name);
            assert!(rows.insert(row["name"].as_str().unwrap().to_owned(), row.clone()).is_none());
        }
    }
    assert_eq!(rows.len(), 27);
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut shipped = BTreeSet::new();
    let mut internal = BTreeSet::new();
    let mut files = 0;
    for entry in fs::read_dir(source.join("skills")).unwrap() {
        let path = entry.unwrap().path().join("SKILL.md");
        if !path.is_file() { continue; }
        files += 1;
        let text = fs::read_to_string(&path).unwrap();
        let front = support_records::skill_frontmatter(&text);
        let name = front["name"].as_str().unwrap();
        if front["user-invocable"] == false {
            assert!(!rows.contains_key(name));
            internal.insert(name.to_owned());
        } else {
            assert!(shipped.insert(name.to_owned()));
            assert_eq!(rows[name]["description"], front["description"], "{name}");
        }
    }
    assert_eq!(files, 35);
    assert_eq!(internal, ["cad-assumptions-analyzer-contract", "cad-plan-checker-contract",
        "cad-planner-contract", "cad-review-delivery", "cad-verifier-contract",
        "cad-reviewer-contract", "cad-executor-contract", "cad-read-contract"]
        .into_iter().map(str::to_owned).collect());
    assert_eq!(shipped.len(), 27);
    assert_eq!(shipped, rows.keys().cloned().collect());
    for absent in ["cad-report", "cad-health"] { assert!(!rows.contains_key(absent)); }
    for (name, description) in [
        ("cad-debug", "Resume a recorded debug session, review its staged fix, and offer a configured consult at dead ends."),
        ("cad-spike", "Record risk-ordered spike criteria before experimenting, then retain observations and a bounded verdict."),
        ("cad-help", "List Cadence commands shipped under skills/ by cluster, or show one command and its compiled description."),
    ] { assert_eq!(rows[name]["description"], description); }

    let mut rendered_users = BTreeSet::new();
    for target in cadence::execution::render::RENDERED_PROJECT_FILES {
        let rendered = support_records::render_skill(project, target.command);
        let installed = fs::read(source.join(target.path)).unwrap();
        assert_eq!(installed, rendered.as_bytes(), "{}", target.path);
        let front = support_records::skill_frontmatter(&rendered);
        if front["user-invocable"] != false {
            let name = front["name"].as_str().unwrap();
            assert_eq!(rows[name]["description"], front["description"], "{name}");
            rendered_users.insert(name.to_owned());
        }
    }
    assert_eq!(rendered_users.len(), 20);
    assert_eq!(shipped.difference(&rendered_users).map(String::as_str).collect::<BTreeSet<_>>(),
        ["cad-adopt", "cad-config", "cad-docs-verify", "cad-new-project", "cad-pause", "cad-phase", "cad-task"].into_iter().collect());

    let mut single = Value::Null;
    for name in ["debug", "cad-debug", "/cad-debug"] {
        let answer = client.call("cadence_query", json!({"operation":"help", "name":name}));
        assert_eq!(answer["status"], "ok", "{answer}");
        assert_eq!(answer["rows"], json!([rows["cad-debug"]]));
        assert_eq!(answer["closest"], json!([]));
        if !single.is_null() { assert_eq!(answer, single); }
        single = answer;
    }
    let missing = client.call("cadence_query", json!({"operation":"help", "name":"healt"}));
    assert_eq!(missing["status"], "ok", "{missing}");
    assert_eq!(missing["rows"], json!([]));
    assert_eq!(missing["closest"], json!(["cad-help", "cad-adopt", "cad-audit"]));
    assert_eq!(client.call("cadence_query", json!({"operation":"help", "name":"healt"})), missing);
    client.finish();
    assert_eq!(serve::tree(project), before, "help writes nothing");
    assert!(!source.join("cadence-core/references/COMMANDS.md").exists());
}
