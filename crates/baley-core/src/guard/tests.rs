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
