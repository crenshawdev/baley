//! What the guard answers one classified call, and which answers are
//! recorded (design 0010, GRD-R2 to GRD-R13). The judge says what the call
//! still needs and the entry gathers only that: for a bound command or
//! PowerShell call, the answer recorded under its call id, before any git
//! read; then for a commit its branch, then its policy. Path calls are
//! judged against the hook's cwd through a supplied [`Lookup`], whether or
//! not a project is bound.

use super::context::{Bound, HookContext};
use crate::hook_input::{CommandTool, Envelope, HookInput, PathTarget, PathTool};
use crate::protected_paths::{Lease, Lookup, read_answer, resolve_target, write_answer};
use baley_core::guard::{
    Answer, BranchObservation, CommitTarget, GitCommand, GitVerb, SettingsInput, ToolInput,
    commit_push_answer, git_command, input_digest, powershell_answer, reason,
};
use std::path::PathBuf;

/// What the entry has gathered for the call so far.
#[derive(Debug, Default)]
pub(super) struct Seen {
    /// The branch at the commit's target directory.
    pub branch: Option<BranchObservation>,
    /// The session project's settings.
    pub settings: Option<SettingsInput>,
    /// Git's stderr ending a torn HEAD copy's cause, which a record leaves
    /// out.
    pub excerpt: Option<String>,
    /// Whether the answer recorded under the call's id was looked for.
    pub looked_up: bool,
}

/// What the entry gathers next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step {
    /// Look for the answer recorded under the call's id with this input
    /// digest, before any git or policy read.
    Lookup(String),
    /// Read the branch at this commit's target directory.
    Branch(PathBuf),
    /// Read this session project's policy.
    Policy(Bound),
}

/// The judge's word on the call: gather one more thing, or answer.
#[derive(Debug)]
pub(super) enum Next {
    Do(Step),
    Decided(Box<Decided>),
}

/// The answer, and the facts it is recorded with when it must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Decided {
    /// The answer before the audit precondition is applied.
    pub answer: Answer,
    /// `None` when nothing is recorded.
    pub record: Option<Selected>,
}

/// What a recorded answer carries besides the answer itself. The command is
/// never kept, only its place in the input digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Selected {
    /// The tool's name as the host gives it.
    pub tool: &'static str,
    /// The input digest over the fields the guard read.
    pub input_digest: String,
    /// A path tool's resolved target, when it resolves to UTF-8 text.
    pub target: Option<String>,
    /// The git verb a command runs.
    pub verb: Option<GitVerb>,
    /// The branch, when one was named.
    pub branch: Option<String>,
    /// The settings a commit was judged under.
    pub settings: Option<SettingsInput>,
}

/// The next step for the call, or its answer.
pub(super) fn next(input: &HookInput, context: &HookContext, seen: &Seen, fs: &dyn Lookup) -> Next {
    let bound = context.project.is_some();
    match input {
        HookInput::NoAnswer | HookInput::Watch(_) => unrecorded(Answer::Pass),
        // Unreadable input has no envelope to record it by.
        HookInput::Deny(reason) => unrecorded(Answer::Deny(reason.clone())),
        HookInput::PowerShell(_) => {
            let facts = Facts::new("PowerShell", ToolInput::default());
            if bound && !seen.looked_up {
                return Next::Do(Step::Lookup(facts.digest()));
            }
            decide(powershell_answer(bound), facts)
        }
        HookInput::Path { envelope, target } => path(envelope, target, context, fs),
        HookInput::Command {
            envelope,
            tool,
            text,
        } => {
            let name = match tool {
                CommandTool::Bash => "Bash",
                CommandTool::Monitor => "Monitor",
            };
            let input = ToolInput {
                command: Some(text.as_str()),
                ..ToolInput::default()
            };
            let (Some(project), Some(command)) = (&context.project, git_command(text)) else {
                return unrecorded(Answer::Pass);
            };
            let mut facts = Facts::new(name, input);
            if !seen.looked_up {
                return Next::Do(Step::Lookup(facts.digest()));
            }
            let verb = command.verb();
            facts.verb = Some(verb);
            let at = match command {
                GitCommand::Push => return decide(Answer::Ask(reason::push_ask()), facts),
                GitCommand::Commit(CommitTarget::Unestablished) => {
                    return decide(Answer::Ask(reason::commit_target_ask()), facts);
                }
                GitCommand::Commit(CommitTarget::Cwd) => PathBuf::from(&envelope.cwd),
                GitCommand::Commit(CommitTarget::Directory(operands)) => operands
                    .iter()
                    .fold(PathBuf::from(&envelope.cwd), |at, operand| at.join(operand)),
            };
            let Some(branch) = &seen.branch else {
                return Next::Do(Step::Branch(at));
            };
            let Some(settings) = &seen.settings else {
                return Next::Do(Step::Policy(project.clone()));
            };
            facts.branch = match branch {
                BranchObservation::Read(name) | BranchObservation::Fallback { name, .. } => {
                    Some(name.clone())
                }
                BranchObservation::Unreadable { .. } => None,
            };
            facts.settings = Some(settings.clone());
            // The remembered policy is read only inside the audit transaction.
            decide(
                commit_push_answer(verb, bound, settings, None, branch),
                facts,
            )
        }
    }
}

/// A path call, judged against the hook's cwd. Without Baley's folders
/// nothing can be cleared, so the call is refused.
fn path(envelope: &Envelope, target: &PathTarget, context: &HookContext, fs: &dyn Lookup) -> Next {
    let spelled = target.path.as_deref();
    let pattern = target.pattern.as_deref();
    let input = match target.tool {
        PathTool::Read | PathTool::Write | PathTool::Edit => ToolInput {
            file_path: spelled,
            ..ToolInput::default()
        },
        PathTool::NotebookEdit => ToolInput {
            notebook_path: spelled,
            ..ToolInput::default()
        },
        PathTool::Grep => ToolInput {
            path: spelled,
            glob: pattern,
            ..ToolInput::default()
        },
        PathTool::Glob => ToolInput {
            path: spelled,
            pattern,
            ..ToolInput::default()
        },
    };
    let mut facts = Facts::new(target.tool.name(), input);
    let cwd = envelope.cwd.as_str();
    let answer = match (&context.protected, target.tool) {
        (Err(refusal), tool) => Answer::Deny(format!(
            "Baley cannot find its home and config folders ({refusal}), so this {} call is refused",
            tool.name()
        )),
        (Ok(protected), PathTool::Write | PathTool::Edit | PathTool::NotebookEdit) => write_answer(
            cwd,
            spelled.unwrap_or_default(),
            protected,
            &Lease::NoActiveDispatch,
            fs,
        ),
        (Ok(protected), PathTool::Read | PathTool::Grep | PathTool::Glob) => {
            read_answer(cwd, spelled, pattern, protected, fs)
        }
    };
    if answer != Answer::Pass {
        // A Grep or Glob without a path searches the cwd.
        facts.target = resolve_target(cwd, spelled.unwrap_or(cwd), fs)
            .ok()
            .and_then(|resolved| resolved.to_str().map(str::to_owned));
    }
    decide(answer, facts)
}

/// The record's facts while they are gathered.
struct Facts<'a> {
    tool: &'static str,
    input: ToolInput<'a>,
    target: Option<String>,
    verb: Option<GitVerb>,
    branch: Option<String>,
    settings: Option<SettingsInput>,
}

impl<'a> Facts<'a> {
    fn new(tool: &'static str, input: ToolInput<'a>) -> Self {
        Facts {
            tool,
            input,
            target: None,
            verb: None,
            branch: None,
            settings: None,
        }
    }

    /// The input digest over the fields the guard read.
    fn digest(&self) -> String {
        input_digest(self.tool, &self.input)
    }
}

/// An answer with no envelope to record it by, or nothing to record.
fn unrecorded(answer: Answer) -> Next {
    Next::Decided(Box::new(Decided {
        answer,
        record: None,
    }))
}

/// Selects what is recorded: every ask, deny and pass on failure of a call
/// with an envelope, and never a plain pass.
fn decide(answer: Answer, facts: Facts<'_>) -> Next {
    let record = (answer != Answer::Pass).then(|| Selected {
        tool: facts.tool,
        input_digest: facts.digest(),
        target: facts.target,
        verb: facts.verb,
        branch: facts.branch,
        settings: facts.settings,
    });
    Next::Decided(Box::new(Decided { answer, record }))
}

#[cfg(test)]
mod tests {
    //! The filesystem seam is `protected_paths`' table `Lookup`: no test here
    //! touches a disk.

    use super::*;
    use crate::folders::FolderRefusal;
    use crate::protected_paths::ProtectedPaths;
    use crate::protected_paths::tests::{Tree, tree};
    use baley_core::guard::GuardSettings;
    use baley_core::policy::{Fault, OnProtected, Unavailable};

    const CONFIG: &str = "/u/.config/crenshawdev/baley";
    const HOME: &str = "/u/.local/share/crenshawdev/baley";

    fn disk() -> Tree {
        tree()
            .dir("/u")
            .dir("/u/.config")
            .dir("/u/.config/crenshawdev")
            .dir(CONFIG)
            .file("/u/.config/crenshawdev/baley/config.toml")
            .dir("/p")
            .file("/p/baley.toml")
            .dir("/p/src")
            .dir("/q")
            .file("/q/baley.toml")
    }

    fn context(project: Option<&str>) -> HookContext {
        let mut files: Vec<_> = project
            .map(|p| format!("{p}/baley.toml").into())
            .into_iter()
            .collect();
        files.push("/q/baley.toml".into());
        HookContext {
            project_directory: Ok(project.map(str::to_owned)),
            project: project.map(|p| Bound {
                folder: p.into(),
                root: p.into(),
            }),
            checkout: Some("/q".into()),
            protected: Ok(ProtectedPaths {
                home: HOME.into(),
                config: CONFIG.into(),
                files,
            }),
        }
    }

    fn envelope(cwd: &str) -> Envelope {
        Envelope {
            cwd: cwd.into(),
            session_id: Some("s".into()),
            tool_use_id: Some("t1".into()),
        }
    }

    fn bash(text: &str) -> HookInput {
        HookInput::Command {
            envelope: envelope("/q"),
            tool: CommandTool::Bash,
            text: text.into(),
        }
    }

    fn write(cwd: &str, file_path: &str) -> HookInput {
        HookInput::Path {
            envelope: envelope(cwd),
            target: PathTarget {
                tool: PathTool::Write,
                path: Some(file_path.into()),
                pattern: None,
            },
        }
    }

    fn decided(next: Next) -> Decided {
        match next {
            Next::Decided(decided) => *decided,
            Next::Do(step) => panic!("asked to gather {step:?}"),
        }
    }

    fn protecting_main() -> SettingsInput {
        SettingsInput::Complete(GuardSettings {
            protected_branches: vec!["main".into()],
            on_protected: OnProtected::Refuse,
            hard_fail: false,
        })
    }

    fn torn() -> SettingsInput {
        SettingsInput::Torn(Unavailable {
            path: "/p/baley.toml".into(),
            fault: Fault::NotRegular,
        })
    }

    /// The redelivery lookup found no record.
    fn looked_up() -> Seen {
        Seen {
            looked_up: true,
            ..Seen::default()
        }
    }

    fn after_gathering(settings: SettingsInput) -> Seen {
        Seen {
            branch: Some(BranchObservation::Read("main".into())),
            settings: Some(settings),
            ..looked_up()
        }
    }

    #[test]
    fn a_redirected_commit_reading_the_branch_at_cwd_is_caught() {
        assert_eq!(
            match next(
                &bash("git -C /r commit"),
                &context(Some("/p")),
                &looked_up(),
                &disk()
            ) {
                Next::Do(step) => step,
                other => panic!("a branch read: {other:?}"),
            },
            Step::Branch("/r".into())
        );
    }

    #[test]
    fn chained_directories_resolved_against_cwd_each_time_are_caught() {
        for (command, expected) in [
            ("git -C a -C ../b commit", "/q/a/../b"),
            ("git -C a -C /r commit", "/r"),
        ] {
            assert!(
                matches!(
                    next(&bash(command), &context(Some("/p")), &looked_up(), &disk()),
                    Next::Do(Step::Branch(at)) if at == std::path::Path::new(expected)
                ),
                "{command}"
            );
        }
    }

    #[test]
    fn a_redirected_commit_taking_policy_from_its_target_is_caught() {
        let seen = Seen {
            branch: Some(BranchObservation::Read("main".into())),
            ..looked_up()
        };
        assert!(matches!(
            next(&bash("git -C /r commit"), &context(Some("/p")), &seen, &disk()),
            Next::Do(Step::Policy(project)) if project == Bound { folder: "/p".into(), root: "/p".into() }
        ));
    }

    #[test]
    fn a_redirected_commit_on_main_passing_or_losing_its_denial_record_is_caught() {
        let decided = decided(next(
            &bash("git -C /r commit"),
            &context(Some("/p")),
            &after_gathering(protecting_main()),
            &disk(),
        ));
        assert_eq!(decided.answer, Answer::Deny(reason::refuse_deny("main")));
        let record = decided.record.expect("the denial is recorded");
        assert_eq!(record.verb, Some(GitVerb::Commit));
        assert_eq!(record.branch.as_deref(), Some("main"));
        assert_eq!(record.settings, Some(protecting_main()));
    }

    #[test]
    fn an_unknown_commit_target_reading_a_branch_or_losing_its_ask_record_is_caught() {
        let decided = decided(next(
            &bash("cd /r && git commit"),
            &context(Some("/p")),
            &looked_up(),
            &disk(),
        ));
        assert_eq!(decided.answer, Answer::Ask(
            "Baley guard: cannot tell which checkout this commit lands in. Approve only if you intend to commit there.".into()
        ));
        let record = decided.record.expect("the ask is recorded");
        assert_eq!(record.verb, Some(GitVerb::Commit));
        assert_eq!(record.branch, None);
        assert_eq!(record.settings, None);
        assert_eq!(record.target, None);
    }

    #[test]
    fn an_unbound_redirected_commit_gathered_or_recorded_is_caught() {
        let decided = decided(next(
            &bash("git -C /r commit"),
            &context(None),
            &Seen::default(),
            &disk(),
        ));
        assert_eq!(decided.answer, Answer::Pass);
        assert_eq!(decided.record, None);
    }

    #[test]
    fn a_bound_commit_that_skips_the_lookup_the_branch_or_the_policy_read_is_caught() {
        let call = bash("git commit -m x");
        let mut seen = Seen::default();
        let command = ToolInput {
            command: Some("git commit -m x"),
            ..ToolInput::default()
        };
        assert!(matches!(
            next(&call, &context(Some("/p")), &seen, &disk()),
            Next::Do(Step::Lookup(digest)) if digest == input_digest("Bash", &command)
        ));
        seen.looked_up = true;
        assert!(matches!(
            next(&call, &context(Some("/p")), &seen, &disk()),
            Next::Do(Step::Branch(at)) if at == std::path::Path::new("/q")
        ));
        seen.branch = Some(BranchObservation::Read("main".into()));
        let session = Bound {
            folder: "/p".into(),
            root: "/p".into(),
        };
        assert!(matches!(
            next(&call, &context(Some("/p")), &seen, &disk()),
            Next::Do(Step::Policy(project)) if project == session
        ));
    }

    #[test]
    fn a_plain_pass_on_an_unprotected_commit_selected_for_recording_is_caught() {
        let seen = Seen {
            branch: Some(BranchObservation::Read("feat/x".into())),
            ..after_gathering(protecting_main())
        };
        let decided = decided(next(
            &bash("git commit -m x"),
            &context(Some("/p")),
            &seen,
            &disk(),
        ));
        assert_eq!(decided.answer, Answer::Pass);
        assert_eq!(decided.record, None);
    }

    #[test]
    fn torn_settings_with_nothing_remembered_denying_instead_of_asking_is_caught() {
        let decided = decided(next(
            &bash("git commit -m x"),
            &context(Some("/p")),
            &after_gathering(torn()),
            &disk(),
        ));
        let Answer::Ask(reason) = &decided.answer else {
            panic!("asked: {:?}", decided.answer);
        };
        assert!(reason.contains("/p/baley.toml"), "{reason}");
        let record = decided.record.expect("an ask is recorded");
        assert_eq!(record.settings, Some(torn()));
        assert_eq!(record.branch.as_deref(), Some("main"));
    }

    #[test]
    fn an_unbound_commit_or_push_gathered_answered_or_selected_is_caught() {
        for text in ["git commit -m x", "git push"] {
            let decided = decided(next(&bash(text), &context(None), &Seen::default(), &disk()));
            assert_eq!(decided.answer, Answer::Pass, "{text}");
            assert_eq!(decided.record, None, "{text}");
        }
    }

    #[test]
    fn an_unbound_write_to_the_config_folder_or_checkout_file_let_through_is_caught() {
        for target in ["/u/.config/crenshawdev/baley/config.toml", "/q/baley.toml"] {
            let decided = decided(next(
                &write("/q", target),
                &context(None),
                &Seen::default(),
                &disk(),
            ));
            assert!(matches!(decided.answer, Answer::Deny(_)), "{target}");
            let record = decided.record.expect("a deny is recorded");
            assert_eq!(record.tool, "Write");
            assert_eq!(record.target.as_deref(), Some(target));
        }
    }

    #[test]
    fn a_relative_write_target_resolved_from_anywhere_but_the_hook_cwd_is_caught() {
        let decided = decided(next(
            &write("/q", "baley.toml"),
            &context(None),
            &Seen::default(),
            &disk(),
        ));
        let Answer::Deny(reason) = &decided.answer else {
            panic!("denied: {:?}", decided.answer);
        };
        assert!(reason.contains("/q/baley.toml"), "{reason}");
        assert_eq!(
            decided.record.and_then(|record| record.target).as_deref(),
            Some("/q/baley.toml")
        );
    }

    #[test]
    fn a_read_grep_or_glob_target_resolved_from_anywhere_but_the_hook_cwd_is_caught() {
        // From the hook's cwd these reach the config folder; from the
        // project or the config folder they reach nothing that exists.
        let config_file = format!("{CONFIG}/config.toml");
        for (tool, path, reached) in [
            (PathTool::Read, "baley/config.toml", config_file.as_str()),
            (PathTool::Grep, "baley", CONFIG),
            (PathTool::Glob, "baley", CONFIG),
        ] {
            let call = HookInput::Path {
                envelope: envelope("/u/.config/crenshawdev"),
                target: PathTarget {
                    tool,
                    path: Some(path.into()),
                    pattern: None,
                },
            };
            let decided = decided(next(&call, &context(Some("/p")), &Seen::default(), &disk()));
            let Answer::Deny(reason) = &decided.answer else {
                panic!("{} denied: {:?}", tool.name(), decided.answer);
            };
            assert!(
                reason.contains(&format!("reaches {reached}, which")),
                "{reason}"
            );
        }
    }

    #[test]
    fn a_monitor_watch_or_a_declined_command_selected_for_recording_is_caught() {
        let watch = HookInput::Watch(envelope("/q"));
        let declined = bash("git commit -m \"$(date)\"");
        for call in [watch, declined] {
            let decided = decided(next(&call, &context(Some("/p")), &Seen::default(), &disk()));
            assert_eq!(decided.answer, Answer::Pass, "{call:?}");
            assert_eq!(decided.record, None, "{call:?}");
        }
    }

    #[test]
    fn unreadable_input_passed_or_selected_for_recording_is_caught() {
        let call = HookInput::Deny("Write hook input cannot be read safely".into());
        let decided = decided(next(&call, &context(Some("/p")), &Seen::default(), &disk()));
        assert_eq!(
            decided.answer,
            Answer::Deny("Write hook input cannot be read safely".into())
        );
        assert_eq!(decided.record, None);
    }

    #[test]
    fn a_path_call_judged_without_baleys_folders_is_caught() {
        let mut unresolved = context(Some("/p"));
        unresolved.protected = Err(FolderRefusal::UserHomeUnset);
        let decided = decided(next(
            &write("/p", "/p/src/lib.rs"),
            &unresolved,
            &Seen::default(),
            &disk(),
        ));
        let Answer::Deny(reason) = &decided.answer else {
            panic!("denied: {:?}", decided.answer);
        };
        assert!(reason.contains("HOME is not set"), "{reason}");
        assert!(decided.record.is_some());
    }

    #[test]
    fn a_bound_powershell_call_passed_or_left_unrecorded_is_caught() {
        let call = HookInput::PowerShell(envelope("/q"));
        let decided = decided(next(&call, &context(Some("/p")), &looked_up(), &disk()));
        assert_eq!(decided.answer, Answer::Ask(reason::powershell_ask()));
        let record = decided.record.expect("an ask is recorded");
        assert_eq!(record.tool, "PowerShell");
        assert_eq!(
            record.input_digest,
            input_digest("PowerShell", &ToolInput::default())
        );
    }

    #[test]
    fn a_record_digest_over_the_wrong_fields_or_carrying_the_command_is_caught() {
        let grep = HookInput::Path {
            envelope: envelope("/q"),
            target: PathTarget {
                tool: PathTool::Grep,
                path: Some(CONFIG.into()),
                pattern: Some("*.env".into()),
            },
        };
        let record = decided(next(&grep, &context(None), &Seen::default(), &disk()))
            .record
            .expect("a refused read is recorded");
        let grep_fields = ToolInput {
            path: Some(CONFIG),
            glob: Some("*.env"),
            ..ToolInput::default()
        };
        assert_eq!(record.input_digest, input_digest("Grep", &grep_fields));
        assert_eq!(record.target.as_deref(), Some(CONFIG));

        let push = bash("git push origin main");
        let record = decided(next(&push, &context(Some("/p")), &looked_up(), &disk()))
            .record
            .expect("a push ask is recorded");
        let command = ToolInput {
            command: Some("git push origin main"),
            ..ToolInput::default()
        };
        assert_eq!(record.input_digest, input_digest("Bash", &command));
        assert_eq!(record.verb, Some(GitVerb::Push));
        assert_eq!(record.target, None);
        assert!(!format!("{record:?}").contains("origin main"), "{record:?}");
    }
}
