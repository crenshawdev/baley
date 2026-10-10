//! The owner identity an acknowledgement names: git's `user.name` (design 0003
//! section 6, design 0012 HST-R10). `owner.name` is not read here, since that
//! setting belongs to the approval build.

use crate::committed::{Ran, failed, finished};
use crate::git_process::{self, Caller};
use crate::ledger::display::Render;
use crate::process::{Launch, Process};

/// The code of a refusal when no owner name can be read from git.
pub(crate) const OWNER_IDENTITY_MISSING: &str = "owner-identity-missing";

/// The launch that reads the name, in the process's own working directory so
/// a repository's own `user.name` applies as git applies it.
fn name_launch() -> Launch {
    git_process::launch(Caller::OwnerIdentity).args(["config", "--get", "user.name"])
}

/// Reads git's `user.name` through `process`. A name that cannot be had
/// refuses `owner-identity-missing`, because an acknowledgement without an
/// owner is not recorded.
pub(crate) fn read(process: &mut dyn Process) -> Result<String, Render> {
    let launch = name_launch();
    let ran = git_process::run(&launch, process);
    judge(&launch, ran).map_err(refusal)
}

/// Interprets git's answer: the name, or why there is none.
///
/// Exit 1 is git's answer for an unset key. Every other failure also gives no
/// name and says what git did, since an acknowledgement needs one either way.
fn judge(launch: &Launch, ran: Ran) -> Result<String, String> {
    if let Err(git_process::Error::Io(error)) = &ran
        && error.kind() == std::io::ErrorKind::NotFound
    {
        return Err("git was not found, so user.name could not be read".into());
    }
    let output = finished(launch, ran)?;
    match output.code() {
        Some(0) => {}
        Some(1) => return Err("git user.name is not set".into()),
        _ => return Err(failed(launch, &output)),
    }
    if !output.stdout_complete {
        return Err("git gave incomplete output for user.name".into());
    }
    let unreadable = || "git gave a user.name that is not one line of text".to_owned();
    let text = std::str::from_utf8(&output.stdout).map_err(|_| unreadable())?;
    let line = text.strip_suffix('\n').unwrap_or(text);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.trim().is_empty() {
        return Err("git user.name is blank".into());
    }
    // The name reaches the owner's terminal and the ledger, so a control
    // character from a config file is refused rather than passed on.
    if line.chars().any(char::is_control) {
        return Err(unreadable());
    }
    Ok(line.to_owned())
}

/// The owner's refusal for a missing name, with how to fix it.
fn refusal(reason: String) -> Render {
    Render::refusal(format!(
        "{OWNER_IDENTITY_MISSING}: {reason}; set it with git config --global user.name \"Your Name\" and run the interview again"
    ))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::git_process::{Deadline, deadline};
    use crate::process::{Output, Recorded};
    use std::time::Duration;

    #[test]
    fn an_owner_name_read_with_other_git_arguments_or_kept_with_its_line_ending_is_caught() {
        let mut fake = Recorded::new().out("Ada Owner\n");
        assert_eq!(read(&mut fake).unwrap(), "Ada Owner");
        assert_eq!(fake.arguments(), [["config", "--get", "user.name"]]);
        let launch = fake.launch();
        assert_eq!(launch.git_caller(), Some(Caller::OwnerIdentity));
        assert_eq!(launch.timeout, Some(Duration::from_secs(60)));
        assert_eq!(
            deadline(Caller::OwnerIdentity),
            Deadline::Exact(Duration::from_secs(60))
        );
    }

    #[test]
    fn an_unset_name_or_a_missing_git_taken_as_an_owner_is_caught() {
        let mut cut_short = Output::exited(0, "Ada Owner\n", "");
        cut_short.stdout_complete = false;
        let cases = [
            Recorded::new().fail(1, ""),
            Recorded::new().unavailable(io::ErrorKind::NotFound.into()),
            Recorded::new().out("   \n"),
            Recorded::new().out(b"Ada \xff\n".to_vec()),
            Recorded::new().fail(128, "fatal: bad config line 1\n"),
            Recorded::new().answer(cut_short),
            Recorded::new().out("Ada\x1b[31m Owner\n"),
            Recorded::new().out("Ada\nOwner\n"),
        ];
        for (index, mut fake) in cases.into_iter().enumerate() {
            let refusal = read(&mut fake).unwrap_err();
            assert_eq!(refusal.lines.len(), 1, "case {index}");
            assert!(
                refusal.lines[0].starts_with("owner-identity-missing:"),
                "case {index}: {}",
                refusal.lines[0]
            );
            assert_eq!(refusal.code, 2, "case {index}");
            assert!(refusal.error, "case {index}");
            if index == 4 {
                assert!(
                    refusal.lines[0].contains("exited with code 128"),
                    "{}",
                    refusal.lines[0]
                );
            }
        }
    }
}
