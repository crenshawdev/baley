//! The guard's decisions on supplied values. Expectations come from design
//! 0010 and the guard's rules, never from running this code.

use super::*;
use crate::policy::{
    EffectivePolicy, Host, OnProtected, Schema, SettingsFile, Unavailable, effective_policy,
};

#[test]
fn a_command_holding_unsupported_posix_syntax_is_judged_instead_of_declined_is_caught() {
    for command in [
        "git push $(date)",
        "git push $BRANCH",
        "git push `date`",
        "(git push)",
        "git push)",
        "{ git push; }",
        "git push }",
        "git push < in",
        "git push > out",
        "# git push",
        "git push\0",
        "git push 'origin",
        "git push \"origin",
        "git push \\",
    ] {
        assert_eq!(git_verb(command), None, "{command:?}");
    }
}

#[test]
fn a_commit_winning_over_a_push_in_one_command_is_caught() {
    assert_eq!(git_verb("git commit -m x && git push"), Some(GitVerb::Push));
}

#[test]
fn a_later_segment_missed_after_a_separator_is_caught() {
    assert_eq!(git_verb("echo a; git push"), Some(GitVerb::Push));
    assert_eq!(git_verb("echo a\ngit push"), Some(GitVerb::Push));
    assert_eq!(git_verb("true || git commit -m x"), Some(GitVerb::Commit));
}

#[test]
fn a_git_path_not_read_as_git_is_caught() {
    assert_eq!(git_verb("/usr/bin/git push"), Some(GitVerb::Push));
}

#[test]
fn a_global_flag_operand_read_as_the_verb_is_caught() {
    assert_eq!(git_verb("git -C push commit -m x"), Some(GitVerb::Commit));
    assert_eq!(git_verb("git -c a=b --no-pager push"), Some(GitVerb::Push));
}

#[test]
fn a_quoted_separator_splitting_the_command_is_caught() {
    assert_eq!(
        git_verb("git commit -m \"a; git push\""),
        Some(GitVerb::Commit)
    );
    assert_eq!(
        git_verb("git commit -m 'a && git push'"),
        Some(GitVerb::Commit)
    );
}

#[test]
fn a_verb_named_only_as_an_argument_of_another_command_is_caught() {
    assert_eq!(git_verb("echo git push"), None);
    assert_eq!(git_verb("echo push"), None);
    assert_eq!(git_verb("gitk commit"), None);
}

// The scanner judges a git verb, never the files a command names.
#[test]
fn a_command_naming_an_owned_file_is_judged_by_its_git_verb_alone() {
    for command in [
        "cat .planning/state.json",
        "rm .planning/decisions.jsonl",
        "cp draft.md .planning/phases/6/SUMMARY.md",
    ] {
        assert_eq!(git_verb(command), None, "{command}");
    }
    assert_eq!(
        git_verb("git commit .planning/state.json"),
        Some(GitVerb::Commit)
    );
}

// Reason text.

/// Every commit reason, each built from `branch`.
fn commit_reasons(branch: &str) -> Vec<String> {
    vec![
        reason::protected_ask(branch),
        reason::refuse_deny(branch),
        reason::hard_fail_deny(branch, "git could not read the branch"),
        reason::remembered_refuse_deny("baley.toml is torn", branch),
        reason::remembered_hard_fail_deny(
            "baley.toml is torn",
            "git could not read the branch",
            branch,
        ),
        reason::torn_ask("baley.toml is torn", Some(branch)),
    ]
}

#[test]
fn a_branch_written_in_debug_form_in_a_reason_is_caught() {
    let branch = "say\"hi";
    for text in commit_reasons(branch) {
        assert!(text.contains(branch), "{text}");
        assert!(!text.contains("Some("), "{text}");
        assert!(!text.contains("None"), "{text}");
        assert!(!text.contains("\\\""), "{text}");
    }
}

#[test]
fn a_commit_answer_naming_a_protected_branch_without_the_guidance_is_caught() {
    for text in [
        reason::protected_ask("main"),
        reason::refuse_deny("main"),
        reason::hard_fail_deny("main", "git could not read the branch"),
        reason::remembered_refuse_deny("baley.toml is torn", "main"),
        reason::remembered_hard_fail_deny(
            "baley.toml is torn",
            "git could not read the branch",
            "main",
        ),
    ] {
        assert!(text.contains("Create a task branch first"), "{text}");
        assert!(text.starts_with("Baley guard"), "{text}");
    }
}

#[test]
fn guidance_claimed_where_no_branch_is_called_protected_is_caught() {
    for text in [
        reason::push_ask(),
        reason::torn_ask("baley.toml is torn", Some("main")),
        reason::torn_ask("baley.toml is torn", None),
        reason::failure_pass("git could not read the branch"),
    ] {
        assert!(!text.contains("Create a task branch first"), "{text}");
        assert!(text.starts_with("Baley guard"), "{text}");
    }
}

#[test]
fn a_push_reason_naming_a_tool_is_caught() {
    let text = reason::push_ask();
    for tool in ["Bash", "Monitor", "PowerShell"] {
        assert!(!text.contains(tool), "{text}");
    }
    assert!(text.contains("git push"), "{text}");
}

#[test]
fn an_unknown_branch_printed_as_none_instead_of_words_is_caught() {
    let text = reason::torn_ask("baley.toml is torn", None);
    assert!(
        text.contains("an unknown branch (git could not read it)"),
        "{text}"
    );
    assert!(!text.contains("None"), "{text}");
}

#[test]
fn a_deny_not_naming_the_setting_that_caused_it_is_caught() {
    assert!(reason::refuse_deny("main").contains("git.on_protected"));
    assert!(reason::hard_fail_deny("main", "x").contains("git.guard_hard_fail"));
    assert!(reason::hard_fail_deny("main", "x").contains(".git/HEAD"));
    assert!(reason::remembered_refuse_deny("torn file", "main").contains("torn file"));
    assert!(reason::remembered_refuse_deny("torn file", "main").contains("git.on_protected"));
    assert!(
        reason::remembered_hard_fail_deny("torn file", "x", "main").contains("git.guard_hard_fail")
    );
    assert!(reason::torn_ask("torn file", None).contains("torn file"));
    assert!(reason::failure_pass("git is unreadable").contains("not policy approval"));
}

// Settings.

fn project_file(text: &str) -> SettingsFile {
    SettingsFile {
        path: "/r/baley.toml".into(),
        bytes: text.as_bytes().to_vec(),
        digest: "digest".into(),
    }
}

fn merged(project: Option<&str>) -> Result<EffectivePolicy, Unavailable> {
    let project = project.map(project_file);
    effective_policy(
        Schema::standard(),
        Some(Host::ClaudeCode),
        None,
        project.as_ref(),
    )
}

fn policy(project: Option<&str>) -> EffectivePolicy {
    merged(project).expect("the file is valid")
}

#[test]
fn a_wrong_default_for_a_guard_setting_is_caught() {
    assert_eq!(
        GuardSettings::from_policy(&policy(None)),
        GuardSettings {
            protected_branches: vec!["main".into(), "master".into()],
            on_protected: OnProtected::Ask,
            hard_fail: false,
        }
    );
}

#[test]
fn a_guard_setting_read_from_the_wrong_name_or_dropped_is_caught() {
    let settings = GuardSettings::from_policy(&policy(Some(
        "[git]\nprotected_branches = [\"release\"]\non_protected = \"refuse\"\nguard_hard_fail = true\n",
    )));
    assert_eq!(
        settings,
        GuardSettings {
            protected_branches: vec!["release".into()],
            on_protected: OnProtected::Refuse,
            hard_fail: true,
        }
    );
}

// The commit and push answer.

fn settings_of(project: Option<&str>) -> SettingsInput {
    SettingsInput::Complete(GuardSettings::from_policy(&policy(project)))
}

fn torn_by(project: &str) -> (SettingsInput, String) {
    let torn = merged(Some(project)).expect_err("the file is torn");
    let text = torn.to_string();
    (SettingsInput::Torn(torn), text)
}

fn read(name: &str) -> BranchObservation {
    BranchObservation::Read(name.into())
}

fn fallback(name: &str) -> BranchObservation {
    BranchObservation::Fallback {
        name: name.into(),
        failure: "git exited with status 128".into(),
    }
}

fn unreadable() -> BranchObservation {
    BranchObservation::Unreadable {
        description: "HEAD is detached".into(),
    }
}

fn commit(
    settings: &SettingsInput,
    remembered: Option<&GuardSettings>,
    branch: &BranchObservation,
) -> Answer {
    commit_push_answer(GitVerb::Commit, true, settings, remembered, branch)
}

fn remembered(on_protected: OnProtected, hard_fail: bool, branches: &[&str]) -> GuardSettings {
    GuardSettings {
        protected_branches: branches.iter().map(|name| (*name).to_owned()).collect(),
        on_protected,
        hard_fail,
    }
}

#[test]
fn the_wrong_branch_protected_by_default_is_caught() {
    let settings = settings_of(None);
    for name in ["main", "master"] {
        assert_eq!(
            commit(&settings, None, &read(name)),
            Answer::Ask(reason::protected_ask(name))
        );
    }
    assert_eq!(commit(&settings, None, &read("develop")), Answer::Pass);
}

#[test]
fn an_empty_protected_list_still_asking_on_main_is_caught() {
    let settings = settings_of(Some("[git]\nprotected_branches = []\n"));
    assert_eq!(commit(&settings, None, &read("main")), Answer::Pass);
}

#[test]
fn refuse_not_denying_a_commit_on_a_protected_branch_is_caught() {
    let settings = settings_of(Some("[git]\non_protected = \"refuse\"\n"));
    assert_eq!(
        commit(&settings, None, &read("main")),
        Answer::Deny(reason::refuse_deny("main"))
    );
    assert_eq!(commit(&settings, None, &read("develop")), Answer::Pass);
}

#[test]
fn allow_still_asking_on_a_protected_branch_is_caught() {
    let settings = settings_of(Some("[git]\non_protected = \"allow\"\n"));
    assert_eq!(commit(&settings, None, &read("main")), Answer::Pass);
}

#[test]
fn a_torn_file_passing_or_going_unnamed_in_the_ask_is_caught() {
    for text in [
        "[git]\nprotected_branches = \"main\"\n",
        "[git]\non_protected = \"sometimes\"\n",
    ] {
        let (settings, named) = torn_by(text);
        assert!(named.contains("/r/baley.toml"), "{named}");
        for branch in [
            read("main"),
            read("develop"),
            fallback("main"),
            unreadable(),
        ] {
            let Answer::Ask(text) = commit(&settings, None, &branch) else {
                panic!("a commit under torn settings must ask: {branch:?}");
            };
            assert!(text.contains(&named), "{text}");
        }
    }
}

#[test]
fn a_remembered_refuse_not_denying_under_torn_settings_is_caught() {
    let (settings, named) = torn_by("[git]\nprotected_branches = \"main\"\n");
    let last = remembered(OnProtected::Refuse, false, &["main", "master"]);
    assert_eq!(
        commit(&settings, Some(&last), &read("main")),
        Answer::Deny(reason::remembered_refuse_deny(&named, "main"))
    );
}

#[test]
fn a_remembered_refuse_judged_against_the_wrong_list_is_caught() {
    let (settings, _) = torn_by("[git]\nprotected_branches = \"main\"\n");
    let last = remembered(OnProtected::Refuse, false, &["release"]);
    assert!(matches!(
        commit(&settings, Some(&last), &read("main")),
        Answer::Ask(_)
    ));
    assert!(matches!(
        commit(&settings, Some(&last), &read("release")),
        Answer::Deny(_)
    ));
}

#[test]
fn a_remembered_allow_or_ask_relaxing_torn_settings_is_caught() {
    let (settings, named) = torn_by("[git]\nprotected_branches = \"main\"\n");
    for on_protected in [OnProtected::Allow, OnProtected::Ask] {
        let last = remembered(on_protected, false, &["main"]);
        assert_eq!(
            commit(&settings, Some(&last), &read("main")),
            Answer::Ask(reason::torn_ask(&named, Some("main")))
        );
    }
}

#[test]
fn a_remembered_refuse_acting_on_a_fallback_only_name_is_caught() {
    let (settings, named) = torn_by("[git]\nprotected_branches = \"main\"\n");
    let last = remembered(OnProtected::Refuse, false, &["main"]);
    assert_eq!(
        commit(&settings, Some(&last), &fallback("main")),
        Answer::Ask(reason::torn_ask(&named, Some("main")))
    );
}

#[test]
fn a_remembered_hard_fail_not_denying_a_provably_protected_branch_is_caught() {
    let (settings, named) = torn_by("[git]\nprotected_branches = \"main\"\n");
    let last = remembered(OnProtected::Ask, true, &["main"]);
    assert_eq!(
        commit(&settings, Some(&last), &fallback("main")),
        Answer::Deny(reason::remembered_hard_fail_deny(
            &named,
            "git could not read the branch (git exited with status 128)",
            "main"
        ))
    );
    assert!(matches!(
        commit(&settings, Some(&last), &fallback("develop")),
        Answer::Ask(_)
    ));
}

#[test]
fn a_torn_commit_with_no_branch_and_no_memory_passing_on_failure_is_caught() {
    let (settings, named) = torn_by("[git]\nprotected_branches = \"main\"\n");
    assert_eq!(
        commit(&settings, None, &unreadable()),
        Answer::Ask(reason::torn_ask(&named, None))
    );
}

#[test]
fn an_unreadable_branch_under_complete_settings_not_passing_on_failure_is_caught() {
    let words = "git could not read the branch (HEAD is detached)";
    for text in [None, Some("[git]\non_protected = \"refuse\"\n")] {
        assert_eq!(
            commit(&settings_of(text), None, &unreadable()),
            Answer::PassOnFailure(reason::failure_pass(words))
        );
    }
}

#[test]
fn hard_fail_not_denying_a_provably_protected_branch_is_caught() {
    let settings = settings_of(Some("[git]\nguard_hard_fail = true\n"));
    assert_eq!(
        commit(&settings, None, &fallback("main")),
        Answer::Deny(reason::hard_fail_deny(
            "main",
            "git could not read the branch (git exited with status 128)"
        ))
    );
}

#[test]
fn hard_fail_denying_a_branch_that_is_not_protected_is_caught() {
    let settings = settings_of(Some("[git]\nguard_hard_fail = true\n"));
    assert!(matches!(
        commit(&settings, None, &fallback("develop")),
        Answer::PassOnFailure(_)
    ));
    assert!(matches!(
        commit(&settings, None, &unreadable()),
        Answer::PassOnFailure(_)
    ));
}

#[test]
fn refuse_acting_on_a_fallback_only_name_is_caught() {
    let settings = settings_of(Some("[git]\non_protected = \"refuse\"\n"));
    assert!(matches!(
        commit(&settings, None, &fallback("main")),
        Answer::PassOnFailure(_)
    ));
}

#[test]
fn a_fallback_name_on_a_protected_branch_denying_without_hard_fail_is_caught() {
    let settings = settings_of(None);
    assert!(matches!(
        commit(&settings, None, &fallback("main")),
        Answer::PassOnFailure(_)
    ));
}

#[test]
fn a_push_not_asking_on_any_branch_or_settings_is_caught() {
    let (torn, _) = torn_by("[git]\nprotected_branches = \"main\"\n");
    let allow = settings_of(Some("[git]\non_protected = \"allow\"\n"));
    let none = settings_of(Some("[git]\nprotected_branches = []\n"));
    for settings in [&settings_of(None), &allow, &none, &torn] {
        for branch in [
            read("develop"),
            read("main"),
            fallback("main"),
            unreadable(),
        ] {
            assert_eq!(
                commit_push_answer(GitVerb::Push, true, settings, None, &branch),
                Answer::Ask(reason::push_ask()),
                "{branch:?}"
            );
        }
    }
}

#[test]
fn a_call_outside_a_project_not_passing_is_caught() {
    let (torn, _) = torn_by("[git]\nprotected_branches = \"main\"\n");
    let refuse = settings_of(Some("[git]\non_protected = \"refuse\"\n"));
    for verb in [GitVerb::Commit, GitVerb::Push] {
        for settings in [&refuse, &torn] {
            for branch in [read("main"), fallback("main"), unreadable()] {
                assert_eq!(
                    commit_push_answer(verb, false, settings, None, &branch),
                    Answer::Pass
                );
            }
        }
    }
}

// PowerShell.

#[test]
fn a_powershell_call_in_a_project_passing_without_its_ask_is_caught() {
    let Answer::Ask(text) = powershell_answer(true) else {
        panic!("a PowerShell call in a project must ask");
    };
    assert!(text.contains("PowerShell"), "{text}");
    assert!(text.contains("POSIX shell grammar"), "{text}");
}

#[test]
fn a_powershell_call_outside_a_project_asking_is_caught() {
    assert_eq!(powershell_answer(false), Answer::Pass);
}

#[test]
fn task_branch_guidance_on_the_powershell_ask_is_caught() {
    assert!(!reason::powershell_ask().contains("Create a task branch first"));
    assert!(reason::powershell_ask().starts_with("Baley guard"));
}
