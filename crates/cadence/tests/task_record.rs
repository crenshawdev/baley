#[allow(dead_code)]
#[path = "support/serve.rs"]
mod serve;
#[path = "support/support_records.rs"]
mod support_records;
#[path = "support/task_fixtures.rs"]
mod task_fixtures;

use serde_json::json;
use std::path::Path;
use task_fixtures::{TaskFixture, commit_file, listing, task_close, task_open};

/// T1: a task done with no planning root reports done with the risk
/// disposition stated and the record called unrecorded, creates nothing,
/// and a blocking risk still blocks.
#[test]
fn task_treeless_done_reports_unrecorded() {
    let fixture = TaskFixture::treeless();
    let project = fixture.project();
    let planning = project.join(".planning");
    let mut client = fixture.client();

    // A protected starting branch gets the branch policy's own disposition.
    let before = listing(&project);
    let protected = task_open(&mut client, "task-open-protected", "harmless-note", "inline", "add a harmless note");
    assert_eq!(listing(&project), before);
    assert_eq!(protected["status"], "refused", "{protected}");
    assert_eq!(protected["code"], "protected-branch");
    assert_eq!(protected["rule"], "branch-policy");
    assert_eq!(protected["details"]["branch"], "main");
    assert_eq!(protected["details"]["permission"], "ask");
    assert_eq!(protected["details"]["policy"], json!({"protected":["main"],"on_protected":"ask"}));
    assert_eq!(protected["details"]["gate"]["options"].as_array().unwrap().iter()
        .map(|option| option["id"].as_str().unwrap()).collect::<Vec<_>>(), ["create", "proceed", "abort"]);
    assert!(!planning.exists());

    // The caller switches to an authorized branch before opening.
    serve::git(&project, &["switch", "-c", "task/harmless-note"]);
    let start = serve::git_value(&project, &["rev-parse", "HEAD"]);
    let before = listing(&project);
    let opened = task_open(&mut client, "task-open-harmless", "harmless-note", "inline", "add a harmless note");
    assert_eq!(listing(&project), before);
    assert_eq!(opened["status"], "ok", "{opened}");
    assert_eq!(opened["outcome"], "open");
    assert_eq!(opened["ephemeral"], true);
    assert_eq!(opened["task"]["slug"], "harmless-note");
    assert_eq!(opened["task"]["mode"], "inline");
    assert_eq!(opened["task"]["branch"], "task/harmless-note");
    assert_eq!(opened["task"]["start"], start);
    assert_eq!(opened["root"], json!({"kind":"absent","path":planning.to_str().unwrap()}));
    assert_eq!(opened["recording"]["kind"], "unrecorded");
    assert_eq!(opened["policy"], json!({"protected":["main"],"on_protected":"ask","permission":"pass"}));
    let token = opened["task"]["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("task-"), "{token}");
    assert!(!planning.exists());

    // The caller's own commit is outside the before/after comparisons.
    let commit = commit_file(&project, "docs/note.txt", b"a harmless note\n", "docs: add a harmless note");

    // A close naming a file that does not exist is classified as that file.
    let missing = fixture.absent("what-shipped.txt");
    let before = listing(&project);
    let named = task_close(&mut client, "task-close-missing", "harmless-note", &token,
        json!({"kind":"file","path":missing.to_str().unwrap()}), Some(&["auth"]));
    assert_eq!(listing(&project), before);
    assert_eq!(named["status"], "refused", "{named}");
    assert_eq!(named["code"], "missing-file");
    assert_eq!(named["rule"], "task-boundary");
    assert_eq!(named["slot"], "request.report.path");
    assert_eq!(named["details"]["path"], missing.to_str().unwrap());
    assert_eq!(named["details"]["root"]["kind"], "absent");
    let reason = named["reason"].as_str().unwrap();
    assert!(reason.contains(missing.to_str().unwrap()), "{reason}");
    assert!(!reason.contains("planning root") && !reason.contains("unrecorded"), "{reason}");
    assert!(!planning.exists());

    // The harmless close: done, risk clear, record unrecorded, nothing created.
    let before = listing(&project);
    let closed = task_close(&mut client, "task-close-harmless", "harmless-note", &token,
        json!({"kind":"text","text":"added docs/note.txt"}), Some(&["auth"]));
    assert_eq!(listing(&project), before);
    assert_eq!(closed["status"], "ok", "{closed}");
    assert_eq!(closed["outcome"], "done");
    assert_eq!(closed["ephemeral"], true);
    let record = &closed["record"];
    assert_eq!(record["schema"], "task-1");
    assert_eq!(record["slug"], "harmless-note");
    assert_eq!(record["mode"], "inline");
    assert_eq!(record["description"], "add a harmless note");
    assert_eq!(record["token"], token);
    assert_eq!(record["root"]["kind"], "absent");
    assert_eq!(record["branch"], "task/harmless-note");
    assert_eq!(record["start"], start);
    assert_eq!(record["head"], commit);
    assert_eq!(record["commits"], json!([{"id":commit,"subject":"docs: add a harmless note","files":["docs/note.txt"]}]));
    assert_eq!(record["files"], json!(["docs/note.txt"]));
    assert_eq!(record["risk"]["kind"], "clear");
    assert_eq!(record["risk"]["surfaces"], json!(["auth"]));
    assert_eq!(record["risk"]["gate"], "blocking");
    assert_eq!(record["risk"]["scan"], json!({"checked":true,"categories":["auth"],"matches":[],"inconclusive":false,"empty":false}));
    assert_eq!(record["report"], "added docs/note.txt");
    assert_eq!(record["recording"]["kind"], "unrecorded");
    assert_eq!(record["recording"]["reason"], format!("no planning root at {}: git is the record", planning.display()));
    assert!(!planning.exists());
    assert!(!project.join("tasks/harmless-note").exists());
    assert!(!planning.join("tasks/harmless-note").exists());
    assert_eq!(fixture.task_material(), Vec::<String>::new());

    // The same request replays its answer; a changed body under it is refused;
    // the closed token is no longer an open task.
    assert_eq!(task_close(&mut client, "task-close-harmless", "harmless-note", &token,
        json!({"kind":"text","text":"added docs/note.txt"}), Some(&["auth"])), closed);
    let reused = task_close(&mut client, "task-close-harmless", "harmless-note", &token,
        json!({"kind":"text","text":"different"}), Some(&["auth"]));
    assert_eq!(reused["code"], "request-reused", "{reused}");
    let again = task_close(&mut client, "task-close-again", "harmless-note", &token,
        json!({"kind":"text","text":"added docs/note.txt"}), Some(&["auth"]));
    assert_eq!(again["code"], "unknown-task", "{again}");
    client.finish();
    assert_eq!(listing(&project), before);
    assert!(!planning.exists());

    // A second independent repository whose change matches the auth surface.
    let fixture = TaskFixture::treeless();
    let project = fixture.project();
    let planning = project.join(".planning");
    serve::git(&project, &["switch", "-c", "task/token-check"]);
    let start = serve::git_value(&project, &["rev-parse", "HEAD"]);
    let mut client = fixture.client();
    let opened = task_open(&mut client, "task-open-auth", "token-check", "inline", "verify the session token");
    assert_eq!(opened["status"], "ok", "{opened}");
    let token = opened["task"]["token"].as_str().unwrap().to_owned();
    let commit = commit_file(&project, "src/risk.txt", b"jwt.verify(token)\n", "feat: verify the session token");
    let before = listing(&project);
    let blocked = task_close(&mut client, "task-close-auth", "token-check", &token,
        json!({"kind":"text","text":"verified the token"}), Some(&["auth"]));
    assert_eq!(listing(&project), before);
    assert_eq!(blocked["status"], "refused", "{blocked}");
    assert_eq!(blocked["code"], "risk-blocked");
    assert_eq!(blocked["rule"], "risk-gate");
    let reason = blocked["reason"].as_str().unwrap();
    assert!(reason.contains("risk surface auth matched") && reason.contains("a JWT sign/verify call")
        && reason.contains("the blocking gate refuses done"), "{reason}");
    let record = &blocked["details"]["record"];
    assert_eq!(record["start"], start);
    assert_eq!(record["head"], commit);
    assert_eq!(record["commits"], json!([{"id":commit,"subject":"feat: verify the session token","files":["src/risk.txt"]}]));
    assert_eq!(record["risk"]["kind"], "blocked");
    assert_eq!(record["risk"]["gate"], "blocking");
    assert_eq!(record["risk"]["matched"], json!(["auth"]));
    assert_eq!(record["risk"]["scan"]["matches"], json!([{"category":"auth","signal":"changed line: a JWT sign/verify call"}]));
    assert_eq!(record["recording"]["kind"], "unrecorded");
    assert_eq!(record["root"]["kind"], "absent");
    // The matched material was held per run under the child's TMPDIR and is gone.
    let expected_diff = "diff --git a/src/risk.txt b/src/risk.txt\nnew file mode 100644\nindex 0000000..bb23d37\n\
        --- /dev/null\n+++ b/src/risk.txt\n@@ -0,0 +1 @@\n+jwt.verify(token)\n";
    let transient = &blocked["details"]["transient"];
    let location = Path::new(transient["location"].as_str().unwrap());
    assert!(location.starts_with(fixture.scratch()), "{}", location.display());
    assert_eq!(location.file_name().unwrap().to_str().unwrap(), format!("cadence-task-{token}"));
    assert_eq!(transient["file"], location.join("risk-task-token-check.diff").to_str().unwrap());
    assert_eq!(transient["bytes"], expected_diff.len());
    assert_eq!(transient["digest"], cadence::store::model::digest(expected_diff.as_bytes()));
    assert!(!location.exists(), "transient material survived the answer at {}", location.display());
    assert_eq!(fixture.task_material(), Vec::<String>::new());
    assert!(!planning.exists());
    assert!(!project.join("tasks/token-check").exists());
    client.finish();
    assert_eq!(listing(&project), before);
    assert!(!planning.exists());
}
