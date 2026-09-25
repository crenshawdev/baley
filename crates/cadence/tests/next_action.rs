use cadence::{derivation::*, next_action::observations::*};
use std::{fs, os::unix::fs::symlink, path::Path};

fn lifecycle(root: &Path, closed: bool) -> Lifecycle {
    fs::write(
        root.join("ROADMAP.md"),
        if closed {
            "## Phases\n"
        } else {
            "## Phases\n- [ ] **Phase 1: One**\n"
        },
    )
    .unwrap();
    derive(&capture_inputs(root, &mut ArtifactFiles).unwrap()).unwrap()
}

fn member(root: &Path, home: &str, name: &str, value: serde_json::Value) {
    fs::create_dir_all(root.join(home)).unwrap();
    fs::write(
        root.join(home).join(name),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}
fn valid(phase: &str) -> serde_json::Value {
    serde_json::json!({"phase":phase,"trigger":"diff-plan","discriminator":"1","round":1,"findings":[{}]})
}

#[test]
fn the_queue_reads_deferred_members_from_both_homes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for home in ["phases/1", "deferred/1"] {
        member(root, home, "DEFERRED-diff-plan-1.json", valid("1"));
    }
    let life = lifecycle(root, true);
    assert_eq!(capture(root, &life).unwrap().queue.members.len(), 2);
}

#[test]
fn a_regular_adjudication_sibling_suppresses_its_member_and_a_symlinked_one_does_not() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let name = "DEFERRED-diff-plan-1.json";
    for home in ["phases/1", "deferred/1"] {
        member(root, home, name, valid("1"));
    }
    let life = lifecycle(root, true);
    fs::write(
        root.join("phases/1/ADJUDICATION-diff-plan-1.json"),
        "malformed",
    )
    .unwrap();
    symlink(
        "DEFERRED-diff-plan-1.json",
        root.join("deferred/1/ADJUDICATION-diff-plan-1.json"),
    )
    .unwrap();
    let q = capture(root, &life).unwrap().queue;
    assert_eq!(q.members.len(), 1);
    assert!(q.members[0].path.starts_with("deferred"));
}

fn queue_member(findings: usize) -> QueueMember {
    QueueMember {
        path: "deferred/1/DEFERRED-diff-plan-1.json".into(),
        phase: "1".into(),
        trigger: "diff-plan".into(),
        discriminator: "1".into(),
        round: 1,
        findings,
    }
}

#[test]
fn a_queue_needs_triage_when_a_member_has_findings_or_something_is_unreadable() {
    let with = |members, unreadable| Queue { members, unreadable };
    assert!(with(vec![queue_member(1)], vec![]).needs_triage());
    assert!(with(vec![], vec!["deferred/link".into()]).needs_triage());
    assert!(!with(vec![queue_member(0)], vec![]).needs_triage());
}

#[test]
fn a_member_whose_phase_round_findings_or_name_does_not_match_is_unreadable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let name = "DEFERRED-diff-plan-1.json";
    for (i, mut value) in [valid("wrong"), valid("2"), valid("3"), valid("4")]
        .into_iter()
        .enumerate()
    {
        match i {
            1 => value["round"] = 0.into(),
            2 => value["findings"] = false.into(),
            3 => value["trigger"] = "other".into(),
            _ => {}
        }
        member(root, &format!("phases/{}", i + 1), name, value);
    }
    let life = lifecycle(root, true);
    let q = capture(root, &life).unwrap().queue;
    assert!(q.members.is_empty());
    assert_eq!(q.unreadable.len(), 4);
}

#[test]
fn a_symlinked_phase_directory_in_a_home_is_unreadable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    member(root, "phases/1", "DEFERRED-diff-plan-1.json", valid("1"));
    fs::create_dir_all(root.join("deferred")).unwrap();
    symlink("../phases/1", root.join("deferred/1")).unwrap();
    let life = lifecycle(root, true);
    let q = capture(root, &life).unwrap().queue;
    assert_eq!(q.unreadable, [Path::new("deferred/1")]);
}

#[test]
fn a_symlinked_member_file_is_unreadable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let name = "DEFERRED-diff-plan-1.json";
    member(root, "phases/1", name, valid("1"));
    fs::create_dir_all(root.join("deferred/1")).unwrap();
    symlink("../../phases/1/DEFERRED-diff-plan-1.json", root.join("deferred/1").join(name)).unwrap();
    let life = lifecycle(root, true);
    let q = capture(root, &life).unwrap().queue;
    assert_eq!(q.unreadable, [Path::new("deferred/1").join(name)]);
}

#[test]
fn a_plain_file_directly_in_a_home_is_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("deferred")).unwrap();
    fs::write(root.join("deferred/file"), "unrelated").unwrap();
    let life = lifecycle(root, true);
    let q = capture(root, &life).unwrap().queue;
    assert_eq!((q.members.len(), q.unreadable.len()), (0, 0));
}

#[test]
fn absent_homes_are_empty_unreadable_homes_are_explicit() {
    use std::io::ErrorKind;
    assert_eq!(unlisted_home("phases", ErrorKind::NotFound), None);
    for error in [ErrorKind::NotADirectory, ErrorKind::PermissionDenied, ErrorKind::Other] {
        assert_eq!(unlisted_home("deferred", error), Some("deferred".into()), "{error:?}");
    }
    let unreadable = Queue { members: vec![], unreadable: vec!["phases".into(), "deferred".into()] };
    assert!(unreadable.needs_triage());
    assert!(!Queue::default().needs_triage());
}

#[test]
fn closed_residue_uses_legal_names_and_live_directories_are_not_residue() {
    let names = || {
        ["1", "1.10", "1.1", "2", "02", "0", "1.0", "1.2.3", "archive"]
            .map(String::from)
    };
    assert_eq!(residue(Cycle::Closed, names()), ["1", "1.1", "1.10", "2"]);
    assert!(residue(Cycle::Live, names()).is_empty());
}
