use super::*;
use crate::process::Recorded;

/// Capture refuses before it asks git anything, so none of these checks needs
/// a repository: the project path only has to be absolute.
const PROJECT: &str = "/project";

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
fn capture_refuses_a_missing_blank_or_multiline_note_before_asking_git() {
    for note in [
        None,
        Some(""),
        Some(" \t "),
        Some("first\nsecond"),
        Some("first\rsecond"),
    ] {
        let process = &mut Recorded::new();
        let mut request = input(Path::new(PROJECT));
        request.sentence = note.map(str::to_owned);
        assert!(capture(request, None, process).is_err(), "{note:?}");
        assert!(process.launches().is_empty(), "{note:?}");
    }
}

#[test]
fn capture_refuses_a_missing_phase_before_asking_git() {
    let process = &mut Recorded::new();
    let mut request = input(Path::new(PROJECT));
    request.phase = None;
    assert!(
        capture(request, None, process)
            .unwrap_err()
            .to_string()
            .contains("missing pause phase")
    );
    assert!(process.launches().is_empty());
}

#[test]
fn capture_refuses_an_unsafe_authorized_path_before_asking_git() {
    for path in ["../outside", "/absolute", ".git/index", ""] {
        let process = &mut Recorded::new();
        let mut request = input(Path::new(PROJECT));
        request.authorized.insert(path.into());
        assert!(capture(request, None, process).is_err(), "{path}");
        assert!(process.launches().is_empty(), "{path}");
    }
}
