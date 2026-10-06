//! The commit and push scanner (design 0010, GRD-R3). It reads a shell
//! command as plain text and names the git verb and commit target, or nothing.

/// A git verb the guard judges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitVerb {
    /// `git commit`.
    Commit,
    /// `git push`.
    Push,
}

/// The checkout a commit names, before resolving paths at the hook's cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitTarget {
    /// No repository redirect: use the hook's working directory.
    Cwd,
    /// Git's `-C` operands, each relative to the directory before it.
    Directory(Vec<String>),
    /// The command does not establish one checkout for every commit.
    Unestablished,
}

/// A judged git command, with the checkout when it is a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCommand {
    /// A commit aimed at this checkout.
    Commit(CommitTarget),
    /// Every push asks, without needing a checkout.
    Push,
}

impl GitCommand {
    /// The verb used by the answer and record.
    pub fn verb(&self) -> GitVerb {
        match self {
            Self::Commit(_) => GitVerb::Commit,
            Self::Push => GitVerb::Push,
        }
    }
}

/// The git verb a POSIX shell command runs, if it runs one the guard judges.
///
/// The scan is linear and bounded. A command with shell structure it cannot
/// read soundly (substitution, a subshell, a redirect, a comment, a NUL, an
/// unclosed quote or a trailing backslash) is declined whole, and a declined
/// command gives nothing. When a command runs both verbs, push wins.
pub fn git_verb(command: &str) -> Option<GitVerb> {
    git_command(command).map(|command| command.verb())
}

/// Scans the verb and commit target with the grammar of [`git_verb`].
/// Earlier directory or environment changes make a commit's target unknown.
pub fn git_command(command: &str) -> Option<GitCommand> {
    let mut quote = None;
    let mut word = String::new();
    let mut started = false;
    let mut words = Vec::new();
    let mut push = false;
    let mut commit = None;
    let mut changed = false;
    let mut chars = command.chars();
    let finish_word = |word: &mut String, started: &mut bool, words: &mut Vec<String>| {
        if *started {
            words.push(std::mem::take(word));
            *started = false;
        }
    };
    let segment = |words: &mut Vec<String>,
                   push: &mut bool,
                   commit: &mut Option<CommitTarget>,
                   changed: &mut bool| {
        if words
            .first()
            .is_some_and(|head| matches!(head.as_str(), "cd" | "pushd" | "popd" | "export"))
        {
            *changed = true;
        }
        if words
            .first()
            .is_some_and(|head| head == "git" || head.ends_with("/git"))
        {
            let mut i = 1;
            let mut directories = Vec::new();
            let mut unknown = *changed;
            while i < words.len() {
                let word = &words[i];
                if let Some(attached) = word.strip_prefix("-C") {
                    let operand = if word == "-C" {
                        i += 1;
                        words.get(i).map(String::as_str).unwrap_or_default()
                    } else {
                        attached
                    };
                    unknown |= operand.is_empty()
                        || operand.starts_with('~')
                        || operand.contains(['*', '?', '[']);
                    directories.push(operand.to_owned());
                    i += 1;
                } else if word == "--git-dir" || word.starts_with("--git-dir=") {
                    unknown = true;
                    i += if word == "--git-dir" { 2 } else { 1 };
                } else if matches!(
                    word.as_str(),
                    "-c" | "--work-tree" | "--namespace" | "--exec-path" | "--config-env"
                ) {
                    i += 2;
                } else if word.starts_with('-') {
                    i += 1;
                } else {
                    *push |= word == "push";
                    if word == "commit" {
                        let target = if unknown {
                            CommitTarget::Unestablished
                        } else if directories.is_empty() {
                            CommitTarget::Cwd
                        } else {
                            CommitTarget::Directory(directories)
                        };
                        *commit = Some(match commit.as_ref() {
                            Some(previous) if previous != &target => CommitTarget::Unestablished,
                            _ => target,
                        });
                    }
                    break;
                }
            }
        }
        words.clear();
    };
    while let Some(ch) = chars.next() {
        if ch == '\0' {
            return None;
        }
        if quote == Some('\'') {
            if ch == '\'' {
                quote = None;
            } else {
                word.push(ch);
            }
            continue;
        }
        if ch == '\\' {
            let escaped = chars.next()?;
            if escaped != '\n' {
                word.push(escaped);
                started = true;
            }
            continue;
        }
        if matches!(ch, '$' | '`') {
            return None;
        }
        if quote == Some('"') {
            if ch == '"' {
                quote = None;
            } else {
                word.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                started = true;
            }
            ';' | '|' | '&' | '\n' => {
                finish_word(&mut word, &mut started, &mut words);
                segment(&mut words, &mut push, &mut commit, &mut changed);
            }
            '(' | ')' | '{' | '}' | '<' | '>' => return None,
            '#' if !started => return None,
            ch if ch.is_ascii_whitespace() => finish_word(&mut word, &mut started, &mut words),
            ch => {
                word.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    finish_word(&mut word, &mut started, &mut words);
    segment(&mut words, &mut push, &mut commit, &mut changed);
    if push {
        Some(GitCommand::Push)
    } else {
        commit.map(GitCommand::Commit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(command: &str, target: CommitTarget) {
        assert_eq!(
            git_command(command),
            Some(GitCommand::Commit(target)),
            "{command}"
        );
    }

    #[test]
    fn a_commit_directory_discarded_as_cwd_is_caught() {
        for command in ["git -C /r commit -m x", "git -C/r commit"] {
            commit(command, CommitTarget::Directory(vec!["/r".into()]));
        }
    }

    #[test]
    fn chained_commit_directories_dropped_or_reordered_are_caught() {
        commit(
            "git -C a -C ../b commit",
            CommitTarget::Directory(vec!["a".into(), "../b".into()]),
        );
    }

    #[test]
    fn a_commit_without_a_redirect_losing_cwd_is_caught() {
        commit("git commit", CommitTarget::Cwd);
    }

    #[test]
    fn an_explicit_git_dir_judged_at_cwd_is_caught() {
        for command in [
            "git --git-dir=/r/.git commit",
            "git --git-dir /r/.git commit",
            "git -C /r --git-dir .git commit",
        ] {
            commit(command, CommitTarget::Unestablished);
        }
    }

    #[test]
    fn a_commit_after_a_directory_or_environment_change_judged_at_cwd_is_caught() {
        for command in [
            "cd /r && git commit",
            "cd /r; git commit",
            "pushd /r && git commit",
            "popd; git commit",
            "export GIT_DIR=/r/.git; git commit",
            "cd /s && git -C /r commit",
        ] {
            commit(command, CommitTarget::Unestablished);
        }
    }

    #[test]
    fn an_empty_or_expanding_directory_taken_as_literal_is_caught() {
        for command in [
            "git -C ~/r commit",
            "git -C 'r*' commit",
            "git -C 'r?' commit",
            "git -C 'r[ab]' commit",
            "git -C '' commit",
        ] {
            commit(command, CommitTarget::Unestablished);
        }
    }

    #[test]
    fn a_work_tree_or_config_option_redirecting_the_checkout_is_caught() {
        for command in [
            "git --work-tree=/w commit",
            "git --work-tree /w commit",
            "git -c a=b commit",
        ] {
            commit(command, CommitTarget::Cwd);
        }
    }

    #[test]
    fn directory_changes_in_arguments_or_after_the_commit_poisoning_its_target_are_caught() {
        for command in [
            "echo cd /r; git commit",
            "echo pushd /r; git commit",
            "echo popd; git commit",
            "echo export GIT_DIR=/r/.git; git commit",
            "git commit; cd /r",
        ] {
            commit(command, CommitTarget::Cwd);
        }
    }

    #[test]
    fn commits_to_different_targets_judged_at_one_checkout_are_caught() {
        for command in [
            "git -C /r commit && git -C /s commit",
            "git commit; git -C /r commit",
        ] {
            commit(command, CommitTarget::Unestablished);
        }
    }

    #[test]
    fn repeated_commits_to_the_same_target_losing_the_directory_are_caught() {
        commit(
            "git -C /r commit; git -C /r commit",
            CommitTarget::Directory(vec!["/r".into()]),
        );
    }

    #[test]
    fn an_unknown_commit_target_overriding_a_push_is_caught() {
        assert_eq!(
            git_command("cd /r; git commit && git push"),
            Some(GitCommand::Push)
        );
    }
}
