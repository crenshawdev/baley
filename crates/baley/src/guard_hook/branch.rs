//! The current branch at the hook's cwd (design 0010, GRD-R6): git on the
//! guard's budget, and the bounded `.git/HEAD` read for when git fails.

use crate::git_process::{self, Caller};
use crate::guard_budget::Budget;
use crate::process::{Output, Process};
use baley_core::guard::BranchObservation;
use std::path::Path;

/// Bytes kept of each of the branch lookup's streams. A branch name is short,
/// and a runaway git must not fill memory within its grant.
const BRANCH_LIMIT: usize = 4096;

/// What the guard's `git symbolic-ref` launch gave.
#[derive(Debug)]
pub(super) enum GitRead {
    /// The guard's git time was spent, so nothing launched.
    Spent,
    /// The launch's result.
    Ran(Result<Output, git_process::Error>),
}

/// Asks git for the current branch on the time the budget has left, and
/// charges the time it took. With no time left nothing launches.
pub(super) fn git_branch(cwd: &Path, process: &mut dyn Process, budget: &mut Budget) -> GitRead {
    let Some(grant) = budget.git() else {
        return GitRead::Spent;
    };
    let ran = git_process::run(
        &git_process::guard_launch(Caller::GuardBranch, &grant)
            .cwd(cwd)
            .unset("GIT_DIR")
            .unset("GIT_WORK_TREE")
            .unset("GIT_COMMON_DIR")
            .unset("GIT_NAMESPACE")
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .limit(BRANCH_LIMIT),
        process,
    );
    budget.charge_git(grant);
    GitRead::Ran(ran)
}

/// Judges the branch from git's answer and the `.git/HEAD` read. Only git's
/// complete, non-empty name is `Read`. A name from `.git/HEAD` after git
/// failed is a `Fallback`, which feeds hard fail alone, so a file anyone can
/// write never decides `refuse` or `ask`.
pub(super) fn branch(git: GitRead, head: Option<String>) -> BranchObservation {
    let failure = match git {
        GitRead::Spent => "the guard's git time is spent".to_owned(),
        GitRead::Ran(Err(git_process::Error::Limit(limit))) => limit.to_string(),
        GitRead::Ran(Ok(output)) if output.success() => {
            // A name cut at the byte cap is not the branch.
            if output.stdout_complete
                && let Ok(name) = String::from_utf8(output.stdout)
            {
                let name = name.trim_end_matches('\n');
                if !name.is_empty() {
                    return BranchObservation::Read(name.into());
                }
            }
            "current branch is empty or unreadable".to_owned()
        }
        GitRead::Ran(Ok(output)) if output.code() == Some(1) => {
            "current branch is unresolvable or HEAD is detached".to_owned()
        }
        GitRead::Ran(_) => "Git cannot read the cwd repository".to_owned(),
    };
    match head {
        Some(name) => BranchObservation::Fallback { name, failure },
        None => BranchObservation::Unreadable {
            description: failure,
        },
    }
}

fn regular_text(path: &Path) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    // Non-blocking, so a pipe or device in place of the file cannot hold the
    // open past the hook's time; the regular-file check below refuses it.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.mode() & 0o444 == 0 || metadata.len() > 4096 {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref().take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The branch `.git/HEAD` names, read from the cwd's checkout without
/// following a link. Current symbolic identity only; a previous branch
/// observation is never reused.
pub(super) fn symbolic_head(cwd: &Path) -> Option<String> {
    let cwd = std::fs::canonicalize(cwd).ok()?;
    for directory in cwd.ancestors() {
        let dotgit = directory.join(".git");
        let metadata = match std::fs::symlink_metadata(&dotgit) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let gitdir = if metadata.is_dir() {
            dotgit
        } else if metadata.is_file() {
            let text = regular_text(&dotgit)?;
            let target = text.strip_prefix("gitdir: ")?.trim_end_matches('\n');
            std::fs::canonicalize(directory.join(target)).ok()?
        } else {
            return None;
        };
        let text = regular_text(&gitdir.join("HEAD"))?;
        let name = text
            .strip_prefix("ref: refs/heads/")?
            .trim_end_matches('\n');
        if name.is_empty()
            || name.starts_with(['/', '-'])
            || name.ends_with(['/', '.'])
            || name.contains("..")
            || name.contains("//")
            || name.contains("@{")
            || name
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
            || name
                .split('/')
                .any(|part| part.starts_with('.') || part.ends_with(".lock"))
        {
            return None;
        }
        return Some(name.into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Recorded;
    use std::{cell::Cell, rc::Rc, time::Duration};

    fn unreadable(description: &str) -> BranchObservation {
        BranchObservation::Unreadable {
            description: description.into(),
        }
    }

    #[test]
    fn an_unbounded_or_truncated_branch_read_is_not_taken_as_the_branch() {
        let mut fake = Recorded::new().answer(Output {
            stdout_complete: false,
            ..Output::exited(0, "main", "")
        });
        let mut budget = Budget::with_clock(|| Duration::ZERO);

        let git = git_branch(Path::new("/r"), &mut fake, &mut budget);

        assert_eq!(fake.launch().limit, 4096);
        assert_eq!(
            branch(git, None),
            unreadable("current branch is empty or unreadable")
        );
    }

    #[test]
    fn spent_git_time_launches_no_branch_lookup_and_reads_as_git_unavailable() {
        // A launch from 1 s to 6 s spends git's whole 5 s with 2 s of work
        // left, so only the spent git time can refuse the next one.
        let now = Rc::new(Cell::new(Duration::from_secs(1)));
        let clock = Rc::clone(&now);
        let mut budget = Budget::with_clock(move || clock.get());
        let first = budget.git().expect("a grant at the start");
        now.set(Duration::from_secs(6));
        budget.charge_git(first);
        // No scripted answer: a launch would panic.
        let mut fake = Recorded::new();

        let git = git_branch(Path::new("/r"), &mut fake, &mut budget);

        assert!(fake.launches().is_empty());
        assert_eq!(
            branch(git, None),
            unreadable("the guard's git time is spent")
        );
    }

    #[test]
    fn a_branch_timeout_names_the_command_and_bound() {
        let git = GitRead::Ran(Err(git_process::Error::Limit(git_process::Limit {
            command: "git symbolic-ref --quiet --short HEAD".into(),
            bound: Duration::from_millis(750),
        })));
        assert_eq!(
            branch(git, None),
            unreadable(
                "git symbolic-ref --quiet --short HEAD exceeded git deadline of 0.75 seconds"
            )
        );
    }

    #[test]
    fn a_head_file_name_taking_over_from_a_git_read_is_caught() {
        let git = GitRead::Ran(Ok(Output::exited(0, "feat/x\n", "")));
        assert_eq!(
            branch(git, Some("main".into())),
            BranchObservation::Read("feat/x".into())
        );
    }

    #[test]
    fn a_head_file_name_after_git_failed_read_as_the_git_branch_is_caught() {
        let failed = GitRead::Ran(Ok(Output::exited(128, "", "fatal: not a git repository")));
        assert_eq!(
            branch(failed, Some("main".into())),
            BranchObservation::Fallback {
                name: "main".into(),
                failure: "Git cannot read the cwd repository".into(),
            }
        );
        assert_eq!(
            branch(GitRead::Spent, Some("main".into())),
            BranchObservation::Fallback {
                name: "main".into(),
                failure: "the guard's git time is spent".into(),
            }
        );
    }

    #[test]
    fn a_detached_head_with_no_head_file_name_taken_as_a_branch_is_caught() {
        let detached = GitRead::Ran(Ok(Output::exited(1, "", "")));
        assert_eq!(
            branch(detached, None),
            unreadable("current branch is unresolvable or HEAD is detached")
        );
    }

    #[test]
    fn a_head_file_that_is_a_pipe_holding_the_fallback_open_is_caught() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let pipe = dir.path().join("HEAD");
        let name = std::ffi::CString::new(pipe.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a valid NUL-terminated path inside the test's own directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o644) }, 0);
        // A pipe with no writer blocks a plain open forever, so the read runs
        // on its own thread and a blocked open fails here instead of hanging.
        let (sent, read) = std::sync::mpsc::channel();
        std::thread::spawn(move || sent.send(regular_text(&pipe)));
        let text = read
            .recv_timeout(Duration::from_secs(10))
            .expect("the open blocked on the pipe");
        assert_eq!(text, None);
    }
}
