//! The session project's policy for a commit (design 0010, GRD-R5 and
//! GRD-R7): the global file, the working-tree `baley.toml` and HEAD's copy,
//! each bounded, merged with Claude Code's host sections. Nothing here admits
//! a checkout or records a policy.

use super::context::Bound;
use crate::committed::{self, Committed};
use crate::discovery::PROJECT_FILE;
use crate::folders::FolderRefusal;
use crate::guard_budget::Budget;
use crate::policy_step::{self, Reads};
use crate::process::Process;
use crate::settings;
use baley_core::guard::{GuardSettings, SettingsInput};
use baley_core::policy::{Fault, Host, SettingsFile, Unavailable};
use std::path::Path;

/// What the policy reads found, before any rule is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    /// The global file's bounded read, or why the config folder is unknown.
    pub global: Result<Result<Option<SettingsFile>, Unavailable>, FolderRefusal>,
    /// The working-tree `baley.toml`'s bounded read.
    pub working: Result<Option<SettingsFile>, Unavailable>,
    /// HEAD's copy, read only once the working-tree file was.
    pub head: Option<Result<Committed, Unavailable>>,
}

/// Reads the global file from `config` and the project's files, HEAD's copy
/// on the guard's budget. It owns no policy, so it has no unit test.
pub(super) fn gather(
    config: Result<&Path, &FolderRefusal>,
    project: &Bound,
    process: &mut dyn Process,
    budget: &mut Budget,
) -> Seen {
    let global = config
        .map(|config| settings::read_for_guard(&config.join(settings::GLOBAL_FILE)))
        .map_err(Clone::clone);
    let working = settings::read_for_guard(&project.folder.join(PROJECT_FILE));
    let head = match &working {
        Ok(Some(file)) => Some(committed::read_for_guard(
            &project.root,
            file,
            process,
            budget,
        )),
        Ok(None) | Err(_) => None,
    };
    Seen {
        global,
        working,
        head,
    }
}

/// The settings a commit is judged under. A refused working-tree read is
/// torn, and so is the global file when Baley's folders are unknown, since a
/// policy without the owner's global file is not the owner's policy. The
/// merge applies Claude Code's `[host.claude-code]` sections.
pub(super) fn settings(seen: Seen) -> SettingsInput {
    let global = seen.global.unwrap_or_else(|refusal| {
        Err(Unavailable {
            path: settings::GLOBAL_FILE.into(),
            fault: Fault::Unreadable {
                cause: refusal.to_string(),
            },
        })
    });
    let head = match seen.working {
        Err(refused) => Some(Err(refused)),
        Ok(_) => seen.head,
    };
    match policy_step::build(&Reads { global, head }, Some(Host::ClaudeCode)) {
        Ok(policy) => SettingsInput::Complete(GuardSettings::from_policy(&policy)),
        Err(torn) => SettingsInput::Torn(torn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baley_core::policy::OnProtected;

    const PROJECT: &str = "/p/baley.toml";

    fn file(text: &str) -> SettingsFile {
        settings::file(Path::new(PROJECT), text.as_bytes().to_vec())
    }

    fn at_head(text: &str) -> Seen {
        Seen {
            global: Ok(Ok(None)),
            working: Ok(Some(file(text))),
            head: Some(Ok(Committed {
                layer: Some(file(text)),
                pending: None,
            })),
        }
    }

    fn refusing() -> SettingsInput {
        SettingsInput::Complete(GuardSettings {
            protected_branches: vec!["main".into(), "master".into()],
            on_protected: OnProtected::Refuse,
            hard_fail: false,
        })
    }

    fn unreadable(path: &str, cause: &str) -> Unavailable {
        Unavailable {
            path: path.into(),
            fault: Fault::Unreadable {
                cause: cause.into(),
            },
        }
    }

    #[test]
    fn a_refused_head_copy_read_as_complete_defaults_is_caught() {
        let refused = unreadable(PROJECT, "HEAD's copy: git exited with code 128");
        let seen = Seen {
            head: Some(Err(refused.clone())),
            ..at_head("[git]\non_protected = \"refuse\"\n")
        };
        assert_eq!(settings(seen), SettingsInput::Torn(refused));
    }

    #[test]
    fn a_head_copy_that_refuses_read_as_another_answer_is_caught() {
        let seen = at_head("[git]\non_protected = \"refuse\"\n");
        assert_eq!(settings(seen), refusing());
    }

    #[test]
    fn a_claude_code_host_section_left_out_of_the_merge_is_caught() {
        let seen = at_head(
            "[git]\non_protected = \"ask\"\n\n[host.claude-code.git]\non_protected = \"refuse\"\n",
        );
        assert_eq!(settings(seen), refusing());
    }

    #[test]
    fn a_refused_working_tree_file_or_unknown_folders_read_as_complete_is_caught() {
        let refused = unreadable(PROJECT, "Permission denied (os error 13)");
        let seen = Seen {
            working: Err(refused.clone()),
            head: None,
            ..at_head("")
        };
        assert_eq!(settings(seen), SettingsInput::Torn(refused));

        let seen = Seen {
            global: Err(FolderRefusal::UserHomeUnset),
            ..at_head("[git]\non_protected = \"refuse\"\n")
        };
        let SettingsInput::Torn(torn) = settings(seen) else {
            panic!("torn");
        };
        assert_eq!(torn.path, Path::new("config.toml"));
        assert!(
            torn.to_string()
                .contains("user-home-invalid: HOME is not set"),
            "{torn}"
        );
    }
}
