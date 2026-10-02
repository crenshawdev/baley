//! A checkout's facts read through git: its root commit and its remote URL (design 0001 Project identity and policy, EVD-R17). Git's messages are
//! translated, so only exit codes classify a result.

use std::path::Path;

use baley_core::policy::EffectivePolicy;

use super::strip_user_information;
use crate::committed::{Ran, born, command, failed, finished};
use crate::git_process::{self, Caller};
use crate::ledger::anchor_plan::remote_of;
use crate::ledger::forge::not_configured;
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

/// A checkout's root commit and the stripped URL of the remote it records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The root commit, `None` when HEAD is unborn.
    pub root_commit: Option<String>,
    /// The remote's URL without user information, `None` when the checkout has
    /// no `origin` and `git.remote` is unset.
    pub remote_url: Option<String>,
}

/// Gathers the facts of the checkout at `root` under `policy`, the root commit
/// first. It needs no store. The remote is the policy's `git.remote`, else
/// `origin`. The refusal is text, since a caller words it as its own.
pub fn gather(
    root: &Path,
    policy: &EffectivePolicy,
    process: &mut dyn Process,
) -> Result<Facts, String> {
    let root_commit = root_commit(root, process)?;
    let named = remote_of(policy);
    let name = named.as_deref().unwrap_or("origin");
    let get_url = git(root, &["remote", "get-url", name]);
    let remote_url = remote_url(
        &get_url,
        git_process::run(&get_url, process),
        named.is_some().then_some(name),
        root,
    )?;
    Ok(Facts {
        root_commit,
        remote_url,
    })
}

/// `remote get-url <name>`: exit 0 with one line is the URL, stripped. Exit 2
/// is no such remote, which refuses for a name `git.remote` gave (`named`) and
/// is none for the `origin` fallback. Anything else refuses.
fn remote_url(
    launch: &Launch,
    ran: Ran,
    named: Option<&str>,
    root: &Path,
) -> Result<Option<String>, String> {
    let output = finished(launch, ran)?;
    if output.code() == Some(2) {
        return match named {
            Some(name) => Err(not_configured(name, root)),
            None => Ok(None),
        };
    }
    if !output.success() {
        return Err(failed(launch, &output));
    }
    if !output.stdout_complete {
        return Err(format!("{} gave incomplete output", command(launch)));
    }
    let unreadable = || format!("{} gave output that is not one URL", command(launch));
    let text = std::str::from_utf8(&output.stdout).map_err(|_| unreadable())?;
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.is_empty() || line.contains('\n') {
        return Err(unreadable());
    }
    Ok(Some(strip_user_information(line)))
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

    fn policy(project_file: &str) -> EffectivePolicy {
        let file = crate::settings::file(Path::new("/r/baley.toml"), project_file.into());
        let reads = crate::policy_step::Reads {
            global: Ok(None),
            head: Some(Ok(crate::committed::Committed {
                layer: Some(file),
                pending: None,
            })),
        };
        crate::policy_step::build(&reads).unwrap()
    }

    fn upstream() -> EffectivePolicy {
        policy("[git]\nremote = \"upstream\"\n")
    }

    fn unset() -> EffectivePolicy {
        policy("")
    }

    fn facts(policy: &EffectivePolicy, fake: &mut Recorded) -> Result<Facts, String> {
        let result = gather(Path::new(ROOT), policy, fake);
        for launch in fake.launches() {
            assert_eq!(launch.git_caller(), Some(Caller::CheckoutFacts));
            assert_eq!(launch.cwd.as_deref(), Some(Path::new(ROOT)));
        }
        result
    }

    /// A born HEAD with one root, ready for the remote's answer.
    fn born_with() -> Recorded {
        Recorded::new().out("").out(format!("{A}\n"))
    }

    #[test]
    fn a_named_remote_is_not_replaced_by_origin_and_its_token_is_not_kept() {
        let mut fake = born_with().out("https://ghp_x@host/o/r.git\n");
        let gathered = facts(&upstream(), &mut fake).unwrap();
        assert_eq!(gathered.remote_url.as_deref(), Some("https://host/o/r.git"));
        assert_eq!(gathered.root_commit.as_deref(), Some(A));
        assert_eq!(fake.arguments()[2], ["remote", "get-url", "upstream"]);
    }

    #[test]
    fn an_unset_git_remote_does_not_skip_the_url_of_origin() {
        let mut fake = born_with().out("ssh://git@host/o/r.git\n");
        let gathered = facts(&unset(), &mut fake).unwrap();
        assert_eq!(gathered.remote_url.as_deref(), Some("ssh://host/o/r.git"));
        assert_eq!(fake.arguments()[2], ["remote", "get-url", "origin"]);
    }

    #[test]
    fn an_unborn_head_does_not_stop_the_remote_from_being_gathered() {
        let mut fake = Recorded::new().fail(1, "").out("git@host:o/r.git\n");
        let gathered = facts(&unset(), &mut fake).unwrap();
        assert_eq!(gathered.root_commit, None);
        assert_eq!(gathered.remote_url.as_deref(), Some("git@host:o/r.git"));
        assert_eq!(fake.launches().len(), 2);
    }

    #[test]
    fn a_missing_origin_without_git_remote_is_none_not_a_refusal() {
        let mut fake = born_with().fail(2, "error: No such remote 'origin'\n");
        let gathered = facts(&unset(), &mut fake).unwrap();
        assert_eq!(gathered.remote_url, None);
    }

    #[test]
    fn a_missing_named_remote_is_not_none_and_not_origin() {
        let mut fake = born_with().fail(2, "error: No such remote 'upstream'\n");
        let refusal = facts(&upstream(), &mut fake).unwrap_err();
        assert_eq!(
            refusal,
            "remote upstream is not configured in the git repository at /r"
        );
        assert_eq!(fake.launches().len(), 3, "origin is never asked for");
    }

    #[test]
    fn a_failed_get_url_on_the_origin_fallback_is_not_read_as_none() {
        let mut fake = born_with().fail(128, "fatal: not a git repository\n");
        let refusal = facts(&unset(), &mut fake).unwrap_err();
        assert!(
            refusal.starts_with("git remote get-url origin exited with code 128"),
            "{refusal}"
        );
    }

    #[test]
    fn two_urls_or_none_with_a_clean_exit_are_not_taken_for_a_url() {
        for bad in ["https://a/x\nhttps://b/y\n", "", "\n"] {
            let mut fake = born_with().out(bad);
            let refusal = facts(&unset(), &mut fake).unwrap_err();
            assert!(refusal.contains("not one URL"), "{bad:?}: {refusal}");
        }
    }

    #[test]
    fn an_incomplete_url_is_not_read_as_one() {
        let mut cut = Output::exited(0, "https://host/o/r", "");
        cut.stdout_complete = false;
        let mut fake = born_with().answer(cut);
        assert!(
            facts(&unset(), &mut fake)
                .unwrap_err()
                .contains("incomplete")
        );
    }
}
