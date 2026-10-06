use super::{Caller, Deadline, Error, Limit, deadline, finish, guard_launch, launch, run};
use crate::guard_budget::Budget;
use crate::process::{Output, Recorded};
use std::{io, time::Duration};

#[test]
fn git_subprocesses_run_under_a_deadline() {
    let rows: &[(Caller, &[&str], &str, u64)] = &[
        (Caller::AnchorForge, &["remote"], "git remote", 60),
        (
            Caller::ExecutionOutput,
            &["show", "HEAD"],
            "git show HEAD",
            60,
        ),
        (
            Caller::ExecutionStatus,
            &["diff", "--quiet"],
            "git diff --quiet",
            60,
        ),
        (
            Caller::PauseRead,
            &["status", "--porcelain"],
            "git status --porcelain",
            60,
        ),
        (Caller::PauseIndex, &["write-tree"], "git write-tree", 60),
        (
            Caller::WhyRead,
            &["log", "--", "a.rs"],
            "git log -- a.rs",
            60,
        ),
        (
            Caller::WhyInput,
            &["cat-file", "--batch-check"],
            "git cat-file --batch-check",
            60,
        ),
        (
            Caller::ExecutionRunner,
            &["rev-parse", "HEAD"],
            "git rev-parse HEAD",
            60,
        ),
        (
            Caller::PauseMergeBase,
            &["merge-base", "base", "head"],
            "git merge-base base head",
            60,
        ),
        (
            Caller::RailRead,
            &["diff", "--cached"],
            "git diff --cached",
            60,
        ),
        (
            Caller::RailCommitInput,
            &["hash-object", "-w", "--stdin"],
            "git hash-object -w --stdin",
            60,
        ),
        (
            Caller::RailConfig,
            &["config", "--get", "user.name"],
            "git config --get user.name",
            60,
        ),
        (
            Caller::RecallHistory,
            &["cat-file", "-s", "HEAD:a.rs"],
            "git cat-file -s HEAD:a.rs",
            60,
        ),
        (
            Caller::ReadDocumentHead,
            &["rev-parse", "HEAD"],
            "git rev-parse HEAD",
            60,
        ),
        (
            Caller::LandingGit,
            &["fetch", "origin"],
            "git fetch origin",
            60,
        ),
        (
            Caller::ProjectHead,
            &["ls-tree", "HEAD", "--", "baley.toml"],
            "git ls-tree HEAD -- baley.toml",
            60,
        ),
        (
            Caller::CheckoutFacts,
            &["rev-list", "--max-parents=0", "HEAD"],
            "git rev-list --max-parents=0 HEAD",
            60,
        ),
    ];
    for &(caller, args, command, seconds) in rows {
        let error = finish(
            &launch(caller).args(args),
            Err(io::ErrorKind::TimedOut.into()),
        )
        .unwrap_err();
        let Error::Limit(limit) = error else {
            panic!("{caller:?}: expected named limit, got {error}");
        };
        assert_eq!(limit.command, command);
        assert_eq!(limit.bound, Duration::from_secs(seconds));
        assert_eq!(
            limit.to_string(),
            format!("{command} exceeded git deadline of {seconds} seconds")
        );
    }
    let status = launch(Caller::PauseRead).args(["status"]);
    for output in [
        Output::exited(0, "kept", "notice"),
        Output::exited(128, "", "ordinary failure"),
        Output::signaled(9),
    ] {
        assert_eq!(finish(&status, Ok(output.clone())).unwrap(), output);
    }
    assert!(matches!(
        finish(&status, Err(io::ErrorKind::PermissionDenied.into())),
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied
    ));
}

#[test]
fn registered_callers_select_their_deadlines() {
    for caller in [
        Caller::ExecutionOutput,
        Caller::ExecutionStatus,
        Caller::PauseRead,
        Caller::PauseIndex,
        Caller::WhyRead,
        Caller::WhyInput,
        Caller::ExecutionRunner,
        Caller::PauseMergeBase,
        Caller::RailRead,
        Caller::RailCommitInput,
        Caller::RailConfig,
        Caller::RecallHistory,
        Caller::ReadDocumentHead,
        Caller::LandingGit,
        Caller::AnchorForge,
        Caller::ProjectHead,
        Caller::CheckoutFacts,
    ] {
        assert_eq!(
            deadline(caller),
            Deadline::Exact(Duration::from_secs(60)),
            "{caller:?}"
        );
        let launch = launch(caller);
        assert_eq!(launch.timeout, Some(Duration::from_secs(60)), "{caller:?}");
        assert!(launch.own_group);
        assert!(!launch.inherit);
        assert!(!launch.die_with_parent);
        assert_eq!(launch.limit, usize::MAX);
    }
    for caller in [Caller::GuardBranch, Caller::GuardProjectHead] {
        assert_eq!(
            deadline(caller),
            Deadline::Guard(Duration::from_secs(5)),
            "{caller:?}"
        );
        // 5.6 s in, the work time left is 2.4 s, under git's 5 s.
        let grant = Budget::with_clock(|| Duration::from_millis(5_600))
            .git()
            .expect("time is left");
        let launch = guard_launch(caller, &grant);
        assert_eq!(launch.git_caller(), Some(caller));
        assert_eq!(
            launch.timeout,
            Some(Duration::from_millis(2_400)),
            "{caller:?}"
        );
        assert!(launch.own_group);
        assert!(!launch.inherit);
        assert!(!launch.die_with_parent);
    }
}

#[test]
fn a_guard_timeout_states_the_time_it_ran_under_not_gits_cap() {
    // 5.6 s in, the work time left is 2.4 s, under git's 5 s.
    let grant = Budget::with_clock(|| Duration::from_millis(5_600))
        .git()
        .expect("time is left");
    let launch = guard_launch(Caller::GuardBranch, &grant).args([
        "symbolic-ref",
        "--quiet",
        "--short",
        "HEAD",
    ]);
    let mut fake = Recorded::new().unavailable(io::ErrorKind::TimedOut.into());

    let error = run(&launch, &mut fake).unwrap_err();

    assert_eq!(
        error.to_string(),
        "git symbolic-ref --quiet --short HEAD exceeded git deadline of 2.4 seconds"
    );
}

#[test]
fn a_limit_states_its_bound_to_the_millisecond_not_in_whole_seconds() {
    for (millis, seconds) in [
        (60_000, "60"),
        (2_400, "2.4"),
        (1_250, "1.25"),
        (750, "0.75"),
        (4_999, "4.999"),
    ] {
        let limit = Limit {
            command: "git status".into(),
            bound: Duration::from_millis(millis),
        };
        assert_eq!(
            limit.to_string(),
            format!("git status exceeded git deadline of {seconds} seconds")
        );
    }
}
