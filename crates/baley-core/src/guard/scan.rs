//! The commit and push scanner (design 0010, GRD-R3). It reads a shell
//! command as plain text and names the git verb it runs, or nothing.

/// A git verb the guard judges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitVerb {
    /// `git commit`.
    Commit,
    /// `git push`.
    Push,
}

/// The git verb a POSIX shell command runs, if it runs one the guard judges.
///
/// The scan is linear and bounded. A command with shell structure it cannot
/// read soundly (substitution, a subshell, a redirect, a comment, a NUL, an
/// unclosed quote or a trailing backslash) is declined whole, and a declined
/// command gives nothing. When a command runs both verbs, push wins.
pub fn git_verb(command: &str) -> Option<GitVerb> {
    let mut quote = None;
    let mut word = String::new();
    let mut started = false;
    let mut words = Vec::new();
    let mut push = false;
    let mut commit = false;
    let mut chars = command.chars();
    let finish_word = |word: &mut String, started: &mut bool, words: &mut Vec<String>| {
        if *started {
            words.push(std::mem::take(word));
            *started = false;
        }
    };
    let segment = |words: &mut Vec<String>, push: &mut bool, commit: &mut bool| {
        if words
            .first()
            .is_some_and(|head| head == "git" || head.ends_with("/git"))
        {
            let mut i = 1;
            while i < words.len() {
                let word = &words[i];
                if matches!(
                    word.as_str(),
                    "-C" | "-c"
                        | "--git-dir"
                        | "--work-tree"
                        | "--namespace"
                        | "--exec-path"
                        | "--config-env"
                ) {
                    i += 2;
                } else if word.starts_with('-') {
                    i += 1;
                } else {
                    *push |= word == "push";
                    *commit |= word == "commit";
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
                segment(&mut words, &mut push, &mut commit);
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
    segment(&mut words, &mut push, &mut commit);
    if push {
        Some(GitVerb::Push)
    } else if commit {
        Some(GitVerb::Commit)
    } else {
        None
    }
}
