//! The owner's identity: the name every provider acknowledgement and every
//! scope approval records (design 0003 section 6, design 0012 HST-R10). It is
//! `owner.name` when the global settings set it, and git's `user.name` when
//! they do not.

use crate::committed::{Ran, failed, finished};
use crate::git_process::{self, Caller};
use crate::ledger::display::Render;
use crate::process::{Launch, Process};

/// The code of a refusal when no owner name can be read.
pub(crate) const OWNER_IDENTITY_MISSING: &str = "owner-identity-missing";

/// The setting that names the owner.
const OWNER_NAME: &str = "owner.name";

/// The launch that reads the name, in the process's own working directory so
/// a repository's own `user.name` applies as git applies it.
fn name_launch() -> Launch {
    git_process::launch(Caller::OwnerIdentity).args(["config", "--get", "user.name"])
}

/// The owner's identity: `name`, the `owner.name` value, when it is set, and
/// then nothing is launched; otherwise git's `user.name` read through
/// `process`. With neither, the refusal text, which the interview, the session
/// server and the command line each answer in their own way.
pub(crate) fn owner(name: Option<&str>, process: &mut dyn Process) -> Result<String, String> {
    if let Some(name) = name {
        return Ok(name.to_owned());
    }
    let launch = name_launch();
    let ran = git_process::run(&launch, process);
    judge(&launch, ran).map_err(|reason| {
        format!(
            "{OWNER_IDENTITY_MISSING}: {reason}; set it with baley config set --global {OWNER_NAME}=\"Your Name\" or git config --global user.name \"Your Name\""
        )
    })
}

/// Reads git's `user.name` through `process`. A name that cannot be had
/// refuses `owner-identity-missing`, because an acknowledgement without an
/// owner is not recorded.
pub(crate) fn read(process: &mut dyn Process) -> Result<String, Render> {
    owner(None, process)
        .map_err(|text| Render::refusal(format!("{text} and run the interview again")))
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

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use baley_core::policy::Value;

    use super::*;
    use crate::git_process::{Deadline, deadline};
    use crate::policy_step::{self, Reads};
    use crate::process::{Output, Recorded};
    use crate::settings;
    use std::time::Duration;

    /// The fix every missing-identity refusal ends with (Task 2's wording).
    const FIX: &str = "; set it with baley config set --global owner.name=\"Your Name\" or git config --global user.name \"Your Name\"";

    /// The owner read from global settings text as Baley builds the policy,
    /// with no project file: a build refusal is its text, and then the reader
    /// never runs.
    fn identify(global: &str, process: &mut dyn Process) -> Result<String, String> {
        let reads = Reads {
            global: Ok(Some(settings::file(
                Path::new("/c/config.toml"),
                global.as_bytes().to_vec(),
            ))),
            head: None,
        };
        let policy = policy_step::build(&reads, None).map_err(|refusal| refusal.to_string())?;
        let name = match &policy.settings["owner.name"].value {
            Some(Value::PersonName(name)) => Some(name.as_str()),
            _ => None,
        };
        owner(name, process)
    }

    #[test]
    fn an_owner_taken_from_git_while_owner_name_is_set_or_an_invalid_owner_name_accepted_is_caught()
    {
        let mut fake = Recorded::new().out("Git Owner\n");
        assert_eq!(
            identify("[owner]\nname = \"Ada Owner\"\n", &mut fake),
            Ok("Ada Owner".to_owned())
        );
        assert!(fake.launches().is_empty());

        let mut fake = Recorded::new().out("Git Owner\n");
        assert_eq!(identify("", &mut fake), Ok("Git Owner".to_owned()));
        assert_eq!(fake.arguments(), [["config", "--get", "user.name"]]);

        for text in [
            "[owner]\nname = \"  \"\n",
            "[owner]\nname = \"Ada\\nOwner\"\n",
            "[owner]\nname = \"Ada\\u0007\"\n",
        ] {
            let mut fake = Recorded::new().out("Git Owner\n");
            let refusal = identify(text, &mut fake).expect_err(text);
            assert!(
                refusal.starts_with("config-unavailable: "),
                "{text:?}: {refusal}"
            );
            assert!(refusal.contains("owner.name"), "{text:?}: {refusal}");
            assert!(fake.launches().is_empty(), "{text:?}");
        }
    }

    #[test]
    fn an_owner_name_read_with_other_git_arguments_or_kept_with_its_line_ending_is_caught() {
        let mut fake = Recorded::new().out("Ada Owner\n");
        assert_eq!(owner(None, &mut fake).unwrap(), "Ada Owner");
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
            let refusal = owner(None, &mut fake).unwrap_err();
            assert!(!refusal.contains('\n'), "case {index}: {refusal}");
            assert!(
                refusal.starts_with("owner-identity-missing:"),
                "case {index}: {refusal}"
            );
            assert!(refusal.ends_with(FIX), "case {index}: {refusal}");
            if index == 4 {
                assert!(refusal.contains("exited with code 128"), "{refusal}");
            }
        }
    }
}
