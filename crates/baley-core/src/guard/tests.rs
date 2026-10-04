//! The guard's decisions on supplied values. Expectations come from design
//! 0010 and the guard's rules, never from running this code.

use super::*;

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
