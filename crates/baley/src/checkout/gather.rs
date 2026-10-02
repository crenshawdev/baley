//! A checkout's facts read through git: its root commit and, later, its remote
//! URL (design 0001 Project identity and policy, EVD-R17). Git's messages are
//! translated, so only exit codes classify a result.

use std::path::Path;

use crate::committed::{Ran, born, command, failed, finished};
use crate::git_process::{self, Caller};
use crate::process::{Launch, Process};

/// One git launch at the root, read-only: no index lock, no lazy fetch from a
/// promisor remote and no prompt.
fn git(root: &Path, args: &[&str]) -> Launch {
    git_process::launch(Caller::CheckoutFacts)
        .cwd(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
}

/// The root commit of the repository at `root`: `None` when HEAD is unborn,
/// the lexically smallest root when HEAD has several, and a refusal text when
/// git fails or gives anything else.
pub fn root_commit(root: &Path, process: &mut dyn Process) -> Result<Option<String>, String> {
    let head = git(root, &["rev-parse", "--verify", "-q", "HEAD"]);
    if !born(&head, git_process::run(&head, process))? {
        return Ok(None);
    }
    let roots = git(root, &["rev-list", "--max-parents=0", "HEAD"]);
    smallest_root(&roots, git_process::run(&roots, process)).map(Some)
}

/// `rev-list --max-parents=0 HEAD`: every line is one object id. The smallest
/// is kept so the choice does not depend on the order git prints them in.
fn smallest_root(launch: &Launch, ran: Ran) -> Result<String, String> {
    let output = finished(launch, ran)?;
    if !output.success() {
        return Err(failed(launch, &output));
    }
    if !output.stdout_complete {
        return Err(format!("{} gave incomplete output", command(launch)));
    }
    let unreadable = || {
        format!(
            "{} gave output that is not root commit ids",
            command(launch)
        )
    };
    let text = std::str::from_utf8(&output.stdout).map_err(|_| unreadable())?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let mut smallest: Option<&str> = None;
    for line in text.split('\n') {
        if !object_id(line) {
            return Err(unreadable());
        }
        smallest = Some(smallest.map_or(line, |kept| kept.min(line)));
    }
    smallest.map(str::to_string).ok_or_else(unreadable)
}

/// A lower-case hex object id of either hash length.
fn object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{Output, Recorded};

    const ROOT: &str = "/r";
    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn gathered(fake: &mut Recorded) -> Result<Option<String>, String> {
        let result = root_commit(Path::new(ROOT), fake);
        for launch in fake.launches() {
            assert_eq!(
                launch.git_caller(),
                Some(Caller::CheckoutFacts),
                "{launch:?}"
            );
            assert_eq!(launch.cwd.as_deref(), Some(Path::new(ROOT)), "{launch:?}");
        }
        result
    }

    #[test]
    fn an_unborn_head_is_not_given_a_root_and_rev_list_never_runs() {
        let mut fake = Recorded::new().fail(1, "");
        assert_eq!(gathered(&mut fake), Ok(None));
        assert_eq!(fake.arguments(), [["rev-parse", "--verify", "-q", "HEAD"]]);
    }

    #[test]
    fn a_rev_parse_failure_is_not_read_as_an_unborn_head() {
        let mut fake = Recorded::new().fail(128, "fatal: not a git repository\n");
        let refusal = gathered(&mut fake).unwrap_err();
        assert_eq!(
            refusal,
            "git rev-parse --verify -q HEAD exited with code 128: fatal: not a git repository"
        );
        assert_eq!(fake.launches().len(), 1);
    }

    #[test]
    fn one_root_is_kept_as_it_is() {
        let mut fake = Recorded::new().out("").out(format!("{A}\n"));
        assert_eq!(gathered(&mut fake), Ok(Some(A.into())));
        assert_eq!(fake.arguments()[1], ["rev-list", "--max-parents=0", "HEAD"]);
    }

    #[test]
    fn the_first_of_two_roots_is_not_kept_when_the_other_is_smaller() {
        let mut fake = Recorded::new().out("").out(format!("{B}\n{A}\n"));
        assert_eq!(gathered(&mut fake), Ok(Some(A.into())));
    }

    #[test]
    fn a_sha_256_root_is_not_refused() {
        let id = "a".repeat(64);
        let mut fake = Recorded::new().out("").out(format!("{id}\n"));
        assert_eq!(gathered(&mut fake), Ok(Some(id)));
    }

    #[test]
    fn a_short_id_an_upper_case_id_or_no_output_is_not_taken_for_a_root() {
        for bad in [&A[..39], &"A".repeat(40)[..], ""] {
            let mut fake = Recorded::new().out("").out(format!("{bad}\n"));
            let refusal = gathered(&mut fake).unwrap_err();
            assert!(
                refusal.contains("git rev-list --max-parents=0 HEAD"),
                "{refusal}"
            );
        }
        let mut fake = Recorded::new().out("").out("");
        assert!(gathered(&mut fake).is_err());
    }

    #[test]
    fn one_bad_line_among_good_ones_is_not_ignored() {
        let mut fake = Recorded::new().out("").out(format!("{A}\nnot-an-id\n"));
        assert!(gathered(&mut fake).is_err());
    }

    #[test]
    fn a_failed_rev_list_is_not_read_as_a_root() {
        let mut fake = Recorded::new()
            .out("")
            .fail(128, "fatal: ambiguous argument 'HEAD'\n");
        let refusal = gathered(&mut fake).unwrap_err();
        assert_eq!(
            refusal,
            "git rev-list --max-parents=0 HEAD exited with code 128: \
             fatal: ambiguous argument 'HEAD'"
        );
    }

    #[test]
    fn an_incomplete_rev_list_is_not_read_as_a_root() {
        let mut cut = Output::exited(0, format!("{A}\n"), "");
        cut.stdout_complete = false;
        let mut fake = Recorded::new().out("").answer(cut);
        assert!(gathered(&mut fake).unwrap_err().contains("incomplete"));
    }

    #[test]
    fn a_git_that_cannot_start_refuses_naming_the_command() {
        let mut fake = Recorded::new().unavailable(std::io::ErrorKind::NotFound.into());
        let refusal = gathered(&mut fake).unwrap_err();
        assert!(refusal.starts_with("git rev-parse --verify -q HEAD could not run"));
    }
}
