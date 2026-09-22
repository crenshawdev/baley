use super::*;
use std::fs;

fn repo(process: &mut dyn Process) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    assert!(temp.path().starts_with("/tmp"));
    git::run(temp.path(), ["init", "-b", "main"], process).unwrap();
    fs::write(temp.path().join("baseline"), "baseline\n").unwrap();
    git::run(temp.path(), ["add", "--", "baseline"], process).unwrap();
    commit(temp.path());
    temp
}

fn commit(root: &Path) {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Pause Fixture",
            "-c",
            "user.email=pause@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "test fixture",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn input(root: &Path) -> Input {
    Input {
        scope: Scope {
            project: root.to_str().unwrap().into(),
            planning_root: root.join(".planning").to_str().unwrap().into(),
            cycle: "cycle".into(),
            occurrence: "pause-1".into(),
            phase: "1".into(),
            plan: "PLAN.md".into(),
            report: "reports/plan-1.md".into(),
        },
        phase: Some(Phase {
            identity: "1".into(),
            name: "Work".into(),
            total: 7,
            provenance: "recorded work".into(),
        }),
        sentence: Some("  verify the fix on the device 日本語  ".into()),
        authorized: BTreeSet::new(),
    }
}

#[test]
fn capture_refuses_missing_or_multiline_input_and_unsafe_paths_without_mutation() {
    let process = &mut cadence::process::System;
    let temp = repo(process);
    let before = git::observe(temp.path(), process).unwrap();
    for note in [
        None,
        Some(""),
        Some(" \t "),
        Some("first\nsecond"),
        Some("first\rsecond"),
    ] {
        let mut request = input(temp.path());
        request.sentence = note.map(str::to_owned);
        assert!(capture(request, None, process).is_err());
    }
    let mut request = input(temp.path());
    request.phase = None;
    assert!(
        capture(request, None, process)
            .unwrap_err()
            .to_string()
            .contains("missing pause phase")
    );
    for path in ["../outside", "/absolute", ".git/index", ""] {
        let mut request = input(temp.path());
        request.authorized.insert(path.into());
        assert!(capture(request, None, process).is_err());
    }
    assert_eq!(git::observe(temp.path(), process).unwrap(), before);
}
