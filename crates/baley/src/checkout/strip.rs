//! The user-information strip applied to a remote URL before it is recorded
//! or compared (design 0001 Project identity and policy, EVD-R17).

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

/// The byte offset just past `://`, when `url` starts with a scheme.
fn scheme_end(url: &str) -> Option<usize> {
    let mut chars = url.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    for (at, c) in chars {
        match c {
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') => {}
            ':' => return url[at..].strip_prefix("://").map(|_| at + 3),
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
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
}
