use super::{Caller, Deadline, Error, deadline, finish, guard_launch, launch};
use crate::guard_budget::Budget;
use crate::process::Output;
use std::{ffi::OsString, io, time::Duration};

#[test]
fn git_subprocesses_run_under_a_deadline() {
    let rows: &[(Caller, &[&str], &str, u64)] = &[
        (Caller::AnchorForge, &["remote"], "git remote", 60),
        (
            Caller::GuardBranch,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            "git symbolic-ref --quiet --short HEAD",
            5,
        ),
        (
            Caller::GuardProjectHead,
            &["rev-parse", "--verify", "-q", "HEAD"],
            "git rev-parse --verify -q HEAD",
            5,
        ),
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
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let error = finish(caller, &args, Err(io::ErrorKind::TimedOut.into())).unwrap_err();
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
    let args = [OsString::from("status")];
    for output in [
        Output::exited(0, "kept", "notice"),
        Output::exited(128, "", "ordinary failure"),
        Output::signaled(9),
    ] {
        assert_eq!(
            finish(Caller::PauseRead, &args, Ok(output.clone())).unwrap(),
            output
        );
    }
    assert!(matches!(
        finish(Caller::PauseRead, &args, Err(io::ErrorKind::PermissionDenied.into())),
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
