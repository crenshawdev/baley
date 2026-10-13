//! The user-information strip applied to a remote URL before it is recorded
//! or compared, and the redaction applied to git's error text before it is
//! stored or shown (design 0001 Project identity and policy, EVD-R17).

/// Quotes git and remotes put around a URL. One ends a URL's authority.
const QUOTES: [char; 3] = ['\'', '"', '`'];

/// Removes the user information from a URL that has a scheme, a bare user
/// included, and returns every other form as given.
///
/// The stripped form is both what `checkout.seen` records and what the fork
/// judgement compares, so a token rotation is not a fork and no secret enters
/// an immutable event. Only user information is removed, so a credential
/// elsewhere, such as in a query string, is kept. No host or path is
/// normalized, so an https clone and an ssh clone of one repository stay
/// different.
///
/// A scheme is a letter followed by letters, digits, `+`, `-` or `.`, then
/// `://`. The authority runs to the first `/`, `?` or `#`, and everything up
/// to and including its last `@` goes. An scp-style `git@host:path`, a local
/// path or a relative path has no scheme and comes back unchanged.
pub fn strip_user_information(url: &str) -> String {
    let Some(scheme_end) = scheme_end(url) else {
        return url.to_string();
    };
    let (head, rest) = url.split_at(scheme_end);
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    format!("{head}{host}{tail}")
}

/// Returns git's error text with every control character dropped and the
/// user information of each URL in it removed, a bare user included.
///
/// Every path that stores or prints git's stderr passes it through here, so a
/// remote's user or token never reaches a recorded reason, a detail or the
/// terminal, and no repository text can move the owner's cursor. Control
/// characters go first: a tab inside `https://user<TAB>:tok@host` would
/// otherwise end the URL early and leave `user:tok@host` once dropped.
///
/// Git and remotes put URLs mid-sentence, so the text is scanned for each
/// `://`, whatever stands before it. No scheme or preceding character is
/// checked, because a colour code, a slash or a letter glued to a scheme
/// would otherwise hide a URL's credentials. The authority after each `://`
/// runs to the first `/`, `?`, `#`, whitespace or quote, and everything up to
/// and including its last `@` goes, so `/srv/odd://u@x/y` becomes
/// `/srv/odd://x/y`. Nothing else changes: a credential in a query string, a
/// `file:///` URL, an scp-style `git@host:path` and text in which no `://` is
/// followed by an authority holding `@` come back as given apart from control
/// characters.
pub fn redact_git_text(text: &str) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    let mut redacted = String::with_capacity(text.len());
    let mut copied = 0;
    let mut from = 0;
    // Each search resumes just past the last `://`, so a `://` whose colon
    // closes an authority is still found. An authority holds no `/`, so no
    // other `://` sits inside one and the scan stays linear on the guard's
    // 1 MiB line.
    while let Some(found) = text[from..].find("://") {
        let authority = from + found + 3;
        from = authority;
        let end = text[authority..]
            .find(|c: char| {
                matches!(c, '/' | '?' | '#') || c.is_whitespace() || QUOTES.contains(&c)
            })
            .map_or(text.len(), |at| authority + at);
        if let Some(at) = text[authority..end].rfind('@') {
            redacted.push_str(&text[copied..authority]);
            copied = authority + at + 1;
        }
    }
    redacted.push_str(&text[copied..]);
    redacted
}

/// A character a scheme may hold after its first letter.
fn scheme_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')
}

/// The byte offset just past `://`, when `url` starts with a scheme.
fn scheme_end(url: &str) -> Option<usize> {
    let mut chars = url.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    for (at, c) in chars {
        match c {
            c if scheme_char(c) => {}
            ':' => return url[at..].strip_prefix("://").map(|_| at + 3),
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::redact_git_text as redact;
    use super::strip_user_information as strip;

    #[test]
    fn a_user_and_password_are_not_kept() {
        assert_eq!(strip("https://user:pass@host/p"), "https://host/p");
    }

    #[test]
    fn a_token_used_as_the_user_name_is_not_kept() {
        assert_eq!(
            strip("https://ghp_x@github.com/o/r.git"),
            "https://github.com/o/r.git"
        );
    }

    #[test]
    fn a_bare_ssh_user_is_not_kept() {
        assert_eq!(strip("ssh://git@host/p"), "ssh://host/p");
    }

    #[test]
    fn the_scp_style_form_is_not_stripped() {
        assert_eq!(strip("git@github.com:o/r.git"), "git@github.com:o/r.git");
    }

    #[test]
    fn a_scheme_with_a_plus_is_not_missed() {
        assert_eq!(strip("git+ssh://git@host/p"), "git+ssh://host/p");
    }

    #[test]
    fn a_bracketed_host_and_port_are_not_lost() {
        assert_eq!(strip("ssh://user@[::1]:22/p"), "ssh://[::1]:22/p");
    }

    #[test]
    fn a_password_holding_an_at_sign_is_cut_at_the_last_one() {
        assert_eq!(strip("https://user:p@ss@host/p"), "https://host/p");
    }

    #[test]
    fn an_at_sign_after_the_authority_is_not_user_information() {
        assert_eq!(strip("https://host/a@b"), "https://host/a@b");
    }

    #[test]
    fn an_at_sign_in_a_query_is_not_taken_for_user_information() {
        assert_eq!(strip("https://host?x=a@b"), "https://host?x=a@b");
    }

    #[test]
    fn a_file_url_and_plain_paths_are_not_changed() {
        for url in ["file:///srv/r.git", "/srv/r.git", "../r"] {
            assert_eq!(strip(url), url);
        }
    }

    #[test]
    fn an_at_sign_in_an_scp_style_path_is_not_stripped() {
        assert_eq!(strip("git@host:path/with@sign"), "git@host:path/with@sign");
    }

    #[test]
    fn credential_free_git_text_or_an_scp_address_changed_by_redaction_is_caught() {
        assert_eq!(
            redact(
                "fatal: unable to access 'https://host/o/r.git/': Could not resolve host: host\u{7}\u{9b}\r"
            ),
            "fatal: unable to access 'https://host/o/r.git/': Could not resolve host: host"
        );
        for kept in ["git@host:o/r.git", "  remote:  kept  as is  "] {
            assert_eq!(redact(kept), kept);
        }
    }

    #[test]
    fn a_url_after_a_colour_code_a_slash_or_a_letter_keeping_its_user_is_caught() {
        for (given, shown) in [
            (
                "fatal: \u{1b}[31mhttps://alice:secret@host/r",
                "fatal: [31mhttps://host/r",
            ),
            ("x/https://alice:secret@host/r", "x/https://host/r"),
            ("xhttps://alice:secret@host/r", "xhttps://host/r"),
            ("/srv/odd://alice:secret@x/y", "/srv/odd://x/y"),
        ] {
            let shown_now = redact(given);
            assert!(
                !shown_now.contains("alice") && !shown_now.contains("secret"),
                "{given}"
            );
            assert_eq!(shown_now, shown, "{given}");
        }
    }

    #[test]
    fn a_url_after_the_start_whitespace_a_quote_or_a_bracket_keeping_its_user_is_caught() {
        for (given, shown) in [
            ("https://alice:tok@host/p", "https://host/p"),
            (
                "fatal: could not read from remote https://alice:tok@host/o/r.git",
                "fatal: could not read from remote https://host/o/r.git",
            ),
            (
                "fatal: unable to access 'https://alice:ghp_tok@host/o/r.git/': Could not resolve host: host",
                "fatal: unable to access 'https://host/o/r.git/': Could not resolve host: host",
            ),
            (
                "[remote rejected] (https://alice:secret@host/o/r.git denied)",
                "[remote rejected] (https://host/o/r.git denied)",
            ),
            (
                "fatal: \"https://alice:secret@host/r\"",
                "fatal: \"https://host/r\"",
            ),
            ("[https://alice:secret@host/r]", "[https://host/r]"),
            ("{https://alice:secret@host/r}", "{https://host/r}"),
            ("<https://alice:secret@host/r>", "<https://host/r>"),
            ("`https://alice:secret@host/r`", "`https://host/r`"),
        ] {
            assert_eq!(redact(given), shown, "{given}");
        }
    }

    #[test]
    fn a_url_after_punctuation_or_a_scheme_run_led_by_a_symbol_keeping_its_user_is_caught() {
        for (given, shown) in [
            (
                "remote: url=https://alice:secret@example.org/r",
                "remote: url=https://example.org/r",
            ),
            // The tab goes first, which leaves `:` before the scheme.
            (
                "remote:\thttps://alice:secret@example.org/r",
                "remote:https://example.org/r",
            ),
            ("(-https://alice:secret@host/r)", "(-https://host/r)"),
            ("(+https://alice:secret@host/r)", "(+https://host/r)"),
            ("(.https://alice:secret@host/r)", "(.https://host/r)"),
        ] {
            assert_eq!(redact(given), shown, "{given}");
        }
    }

    #[test]
    fn a_bare_user_or_a_token_as_the_user_kept_in_git_text_is_caught() {
        assert_eq!(
            redact("fatal: could not read from remote ssh://git@host/o/r.git"),
            "fatal: could not read from remote ssh://host/o/r.git"
        );
        assert_eq!(
            redact("fatal: repository 'https://ghp_x@github.com/o/r.git/' not found"),
            "fatal: repository 'https://github.com/o/r.git/' not found"
        );
    }

    #[test]
    fn a_url_run_past_whitespace_or_a_closing_quote_to_a_later_at_sign_is_caught() {
        for kept in [
            "fatal: could not read from https://host, ask admin@corp.example",
            "fatal: 'https://host'@x",
        ] {
            assert_eq!(redact(kept), kept);
        }
        assert_eq!(
            redact("fatal: 'https://alice:tok@host' does not appear to be a git repository"),
            "fatal: 'https://host' does not appear to be a git repository"
        );
        assert_eq!(
            redact("remote: see https://alice:tok@host now"),
            "remote: see https://host now"
        );
    }

    #[test]
    fn an_at_sign_in_a_query_or_a_file_url_path_taken_for_a_user_is_caught() {
        for kept in [
            "fatal: repository 'https://host?token=a@b' not found",
            "fatal: 'file:///srv/a@b/r.git' does not appear to be a git repository",
        ] {
            assert_eq!(redact(kept), kept);
        }
    }

    #[test]
    fn a_tab_in_the_user_information_rejoining_the_token_after_redaction_is_caught() {
        assert_eq!(
            redact("fatal: unable to access 'https://alice\t:tok@host/o/r.git/'"),
            "fatal: unable to access 'https://host/o/r.git/'"
        );
    }

    #[test]
    fn a_second_url_in_one_line_keeping_its_user_is_caught() {
        assert_eq!(
            redact(
                "warning: redirecting to https://alice@new.example/o/r.git/ from https://alice:tok@old.example/o/r.git/"
            ),
            "warning: redirecting to https://new.example/o/r.git/ from https://old.example/o/r.git/"
        );
        assert_eq!(
            redact("https://alice:secret@a https://bob:secret@b/r"),
            "https://a https://b/r"
        );
    }
}
