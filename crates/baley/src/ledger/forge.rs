//! Git observations for immutable tags without local references.
use super::{CliRefusal, remotes::configured};
use crate::{
    git_process::{self, Caller},
    process::{Output, Process},
};
use baley_core::{FetchObservation, Forge, PushObservation, TagQuery, tag_sequence};
use baley_store::ProjectId;
use std::path::PathBuf;

/// Translates bounded git observations into forge observations.
pub(super) struct GitForge<P: Process> {
    /// The process seam used by git gathering.
    pub(super) process: P,
    cwd: PathBuf,
    seconds: Box<dyn Fn() -> i64>,
    /// The last failed git call's diagnostic.
    pub(super) last_error: Option<String>,
}
impl<P: Process> GitForge<P> {
    /// Builds the edge adapter from its supplied dependencies.
    pub(super) fn new(process: P, cwd: PathBuf, seconds: impl Fn() -> i64 + 'static) -> Self {
        Self {
            process,
            cwd,
            seconds: Box::new(seconds),
            last_error: None,
        }
    }
    fn call(&mut self, args: &[String], input: &[u8]) -> Option<Output> {
        let launch = git_process::launch(Caller::AnchorForge)
            .args(args)
            .cwd(&self.cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .limit(16 * 1024 * 1024)
            .stdin(input);
        match git_process::run(&launch, &mut self.process) {
            Ok(output) => {
                if !output.success() || !output.complete() {
                    self.last_error =
                        Some(String::from_utf8_lossy(&output.stderr).trim().to_owned());
                }
                if output.complete() {
                    Some(output)
                } else {
                    if self.last_error.as_deref() == Some("") {
                        self.last_error = Some("git output exceeded 16 MiB".into());
                    }
                    None
                }
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                None
            }
        }
    }
    fn remote(&mut self, name: &str) -> Option<bool> {
        let output = self.call(&["remote".into()], &[])?;
        output
            .success()
            .then(|| configured(&String::from_utf8_lossy(&output.stdout), name))
    }
    /// Refuses an absent remote before a command records or fetches.
    pub(super) fn require_remote(&mut self, name: &str) -> Result<(), CliRefusal> {
        match self.remote(name) {
            Some(true) => Ok(()),
            Some(false) => Err(CliRefusal(format!("remote {name} is not configured in the git repository at {}", self.cwd.display()))),
            None => Err(CliRefusal("the current directory is not in a git repository; remotes are read from the repository at the current directory".into())),
        }
    }
    /// Resolves the one repository anchors are written to and read from.
    fn anchor_url(&mut self, remote: &str) -> Option<String> {
        let output = self.call(
            &strings(&["remote", "get-url", "--push", "--all", remote]),
            &[],
        )?;
        match push_url(output) {
            Ok(url) => Some(url),
            Err(reason) => {
                self.last_error = Some(format!("remote {remote} {reason}"));
                None
            }
        }
    }
    fn object(&mut self, args: &[String], input: &[u8]) -> Option<String> {
        let output = self.call(args, input)?;
        if !output.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?;
        let id = text.strip_suffix('\n').unwrap_or(&text);
        if object_id(id) {
            Some(id.into())
        } else {
            self.last_error = Some("git returned a malformed object id".into());
            None
        }
    }
}
fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).into()).collect()
}
fn object_id(id: &str) -> bool {
    matches!(id.len(), 40 | 64)
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Builds the immutable tag object on its supplied tree.
pub(super) fn tag_object_text(tree: &str, tag: &str, annotation: &str, seconds: i64) -> String {
    format!(
        "object {tree}\ntype tree\ntag {tag}\ntagger Baley <baley@localhost> {seconds} +0000\n\n{annotation}"
    )
}
/// Requires one destination rather than silently choosing among push URLs.
pub(super) fn push_url(output: Output) -> Result<String, String> {
    if !output.complete() {
        return Err("push URL lookup output exceeded 16 MiB".into());
    }
    if !output.success() {
        return Err(format!(
            "push URL lookup failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "push URL lookup returned invalid UTF-8".to_owned())?;
    let lines: Vec<_> = text.lines().collect();
    match lines.as_slice() {
        [url] if !url.is_empty() => Ok((*url).into()),
        [] | [""] => Err("has no push URL; anchoring needs exactly one".into()),
        _ => Err(format!(
            "has {} push URLs; anchoring needs exactly one",
            lines.len()
        )),
    }
}
/// Pushes an object id to a URL without applying named-remote ref mappings.
pub(super) fn push_args(url: &str, sha: &str, tag: &str) -> Vec<String> {
    strings(&[
        "push",
        "--porcelain",
        "--no-verify",
        url,
        &format!("{sha}:refs/tags/{tag}"),
    ])
}
/// Requests both exact and peeled refs for exact queries.
pub(super) fn ls_remote_args(url: &str, project: &ProjectId, query: &TagQuery) -> Vec<String> {
    let mut args = strings(&["ls-remote", "--tags", "--exit-code", url]);
    match query {
        TagQuery::Exact(tag) => {
            args.push(format!("refs/tags/{tag}"));
            args.push(format!("refs/tags/{tag}^{{}}"));
        }
        TagQuery::LatestAnchor => args.push(format!("refs/tags/baley-anchor/{}/*", project.0)),
    }
    args
}
/// Fetches objects without creating local refs.
pub(super) fn fetch_args(url: &str, tag: &str) -> Vec<String> {
    strings(&[
        "fetch",
        "--no-tags",
        "--no-write-fetch-head",
        "--refmap=",
        url,
        &format!("refs/tags/{tag}"),
    ])
}
/// Interprets the requested ref's porcelain push status.
pub(super) fn push_observation(output: &Output, tag: &str) -> PushObservation {
    if !output.complete() {
        return PushObservation::Unreachable;
    }
    let reference = format!("refs/tags/{tag}");
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let columns: Vec<_> = line.split('\t').collect();
        if columns.len() != 3
            || columns[1].rsplit_once(':').map(|(_, r)| r) != Some(reference.as_str())
        {
            continue;
        }
        match columns[0] {
            // A success line from a git that then failed is not a confirmed push.
            "*" | "=" if output.success() => return PushObservation::Pushed,
            "*" | "=" => return PushObservation::Unreachable,
            "!" => {
                return PushObservation::Refused {
                    reason: columns[2].into(),
                };
            }
            _ => {}
        }
    }
    PushObservation::Unreachable
}
#[derive(Debug, PartialEq, Eq)]
/// The selected remote ref and whether it is annotated.
pub(super) enum Listing {
    Absent,
    Unreachable,
    Found {
        tag: String,
        sha: String,
        annotated: bool,
    },
}
/// Selects an exact ref or the highest numeric anchor.
pub(super) fn listing(project: &ProjectId, query: &TagQuery, output: &Output) -> Listing {
    if !output.complete() {
        return Listing::Unreachable;
    }
    if output.code() == Some(2) {
        return Listing::Absent;
    }
    if !output.success() {
        return Listing::Unreachable;
    }
    let Ok(text) = std::str::from_utf8(&output.stdout) else {
        return Listing::Unreachable;
    };
    let mut refs = Vec::new();
    for line in text.lines() {
        let Some((sha, name)) = line.split_once('\t') else {
            return Listing::Unreachable;
        };
        if !object_id(sha) {
            return Listing::Unreachable;
        }
        refs.push((sha, name));
    }
    let found = refs
        .iter()
        .filter_map(|(sha, reference)| {
            let tag = reference.strip_prefix("refs/tags/")?;
            if tag.ends_with("^{}") {
                return None;
            }
            let seq = match query {
                TagQuery::Exact(exact) if tag == exact => 0,
                TagQuery::Exact(_) => return None,
                TagQuery::LatestAnchor => tag_sequence(project, tag)?,
            };
            Some((seq, *sha, tag))
        })
        .max_by_key(|(seq, _, _)| *seq);
    match found {
        None => Listing::Absent,
        Some((_, sha, tag)) => Listing::Found {
            tag: tag.into(),
            sha: sha.into(),
            annotated: refs
                .iter()
                .any(|(_, name)| *name == format!("refs/tags/{tag}^{{}}")),
        },
    }
}
/// Keeps the annotation and any following signature.
pub(super) fn tag_message(stdout: &str) -> String {
    stdout
        .split_once("\n\n")
        .map_or("", |(_, text)| text)
        .into()
}
impl<P: Process> Forge for GitForge<P> {
    fn push_tag(
        &mut self,
        _project: &ProjectId,
        remote: &str,
        tag: &str,
        annotation: &str,
    ) -> PushObservation {
        match self.remote(remote) {
            Some(true) => {}
            Some(false) => return PushObservation::NoRemote,
            None => return PushObservation::Unreachable,
        }
        let Some(url) = self.anchor_url(remote) else {
            return PushObservation::Unreachable;
        };
        let Some(tree) = self.object(
            &strings(&["hash-object", "-t", "tree", "-w", "--stdin"]),
            &[],
        ) else {
            return PushObservation::Unreachable;
        };
        let text = tag_object_text(&tree, tag, annotation, (self.seconds)());
        let Some(sha) = self.object(&strings(&["mktag"]), text.as_bytes()) else {
            return PushObservation::Unreachable;
        };
        self.call(&push_args(&url, &sha, tag), &[])
            .map_or(PushObservation::Unreachable, |o| push_observation(&o, tag))
    }
    fn fetch_tag(
        &mut self,
        project: &ProjectId,
        remote: &str,
        query: &TagQuery,
    ) -> FetchObservation {
        match self.remote(remote) {
            Some(true) => {}
            Some(false) => return FetchObservation::NoRemote,
            None => return FetchObservation::Unreachable,
        }
        // Read the repository the push wrote to, not a separate fetch URL.
        let Some(url) = self.anchor_url(remote) else {
            return FetchObservation::Unreachable;
        };
        let Some(output) = self.call(&ls_remote_args(&url, project, query), &[]) else {
            return FetchObservation::Unreachable;
        };
        match listing(project, query, &output) {
            Listing::Absent => FetchObservation::Absent,
            Listing::Unreachable => FetchObservation::Unreachable,
            Listing::Found {
                tag,
                annotated: false,
                ..
            } => FetchObservation::Present {
                tag,
                annotation: String::new(),
            },
            Listing::Found {
                tag,
                sha,
                annotated: true,
            } => {
                if !self
                    .call(&fetch_args(&url, &tag), &[])
                    .is_some_and(|o| o.success())
                {
                    return FetchObservation::Unreachable;
                }
                match self.call(&strings(&["cat-file", "tag", &sha]), &[]) {
                    Some(o) if o.success() => FetchObservation::Present {
                        tag,
                        annotation: tag_message(&String::from_utf8_lossy(&o.stdout)),
                    },
                    _ => FetchObservation::Unreachable,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Recorded;
    fn p() -> ProjectId {
        ProjectId("p".into())
    }
    fn output(code: i32, text: &str) -> Output {
        Output::exited(code, text, "")
    }
    fn exact() -> TagQuery {
        TagQuery::Exact("baley-anchor/p/5".into())
    }
    fn forge(process: Recorded) -> GitForge<Recorded> {
        GitForge::new(process, PathBuf::from("/fixture"), || 123)
    }
    fn listed(peeled: bool) -> String {
        format!(
            "{}\trefs/tags/baley-anchor/p/5\n{}",
            "a".repeat(40),
            if peeled {
                format!("{}\trefs/tags/baley-anchor/p/5^{{}}\n", "b".repeat(40))
            } else {
                String::new()
            }
        )
    }
    #[test]
    fn a_new_tag_is_pushed() {
        assert_eq!(
            push_observation(&output(0, "*\tsha:refs/tags/tag\t[new tag]\n"), "tag"),
            PushObservation::Pushed
        );
    }
    #[test]
    fn an_up_to_date_tag_is_pushed() {
        assert_eq!(
            push_observation(&output(0, "=\tsha:refs/tags/tag\t[up to date]\n"), "tag"),
            PushObservation::Pushed
        );
    }
    #[test]
    fn a_rejection_keeps_its_reason() {
        assert_eq!(
            push_observation(
                &output(1, "!\tsha:refs/tags/tag\t[rejected] (already exists)\n"),
                "tag"
            ),
            PushObservation::Refused {
                reason: "[rejected] (already exists)".into()
            }
        );
    }
    #[test]
    fn a_failed_git_exit_cannot_confirm_a_push() {
        assert_eq!(
            push_observation(&output(1, "*\tsha:refs/tags/tag\t[new tag]\n"), "tag"),
            PushObservation::Unreachable
        );
        assert_eq!(
            push_observation(&output(128, "=\tsha:refs/tags/tag\t[up to date]\n"), "tag"),
            PushObservation::Unreachable
        );
    }
    #[test]
    fn a_push_without_porcelain_is_unreachable() {
        assert_eq!(
            push_observation(&output(128, ""), "tag"),
            PushObservation::Unreachable
        );
    }
    #[test]
    fn a_timed_out_push_is_unreachable() {
        let mut f = forge(
            Recorded::new()
                .out("origin\n")
                .out("/fixture/remote.git\n")
                .out(format!("{}\n", "a".repeat(40)))
                .out(format!("{}\n", "b".repeat(40)))
                .unavailable(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout")),
        );
        assert_eq!(
            f.push_tag(&p(), "origin", "tag", "note\n"),
            PushObservation::Unreachable
        );
        assert_eq!(f.process.launches().len(), 5);
    }
    #[test]
    fn an_unconfigured_remote_launches_nothing_more() {
        let mut f = forge(Recorded::new().out("other\n"));
        assert_eq!(
            f.push_tag(&p(), "origin", "tag", "note\n"),
            PushObservation::NoRemote
        );
        assert_eq!(f.process.launches().len(), 1);
    }
    #[test]
    fn push_uses_the_resolved_url_instead_of_the_remote_name() {
        let mut f = forge(
            Recorded::new()
                .out("origin\n")
                .out("/fixture/remote.git\n")
                .out(format!("{}\n", "a".repeat(40)))
                .out(format!("{}\n", "b".repeat(40)))
                .out("*\tsha:refs/tags/TAG\t[new tag]\n"),
        );
        assert_eq!(
            f.push_tag(&p(), "origin", "TAG", "note\n"),
            PushObservation::Pushed
        );
        assert_eq!(
            f.process.launches()[1].args,
            strings(&["remote", "get-url", "--push", "--all", "origin"])
                .iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            f.process.launches()[4].args,
            strings(&[
                "push",
                "--porcelain",
                "--no-verify",
                "/fixture/remote.git",
                &format!("{}:refs/tags/TAG", "b".repeat(40))
            ])
            .iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
        );
    }
    #[test]
    fn one_push_url_is_preserved() {
        assert_eq!(
            push_url(output(0, "ssh://forge/repo.git\n")),
            Ok("ssh://forge/repo.git".into())
        );
    }
    #[test]
    fn empty_push_url_output_is_not_a_destination() {
        assert!(push_url(output(0, "")).is_err());
    }
    #[test]
    fn several_push_urls_cannot_silently_choose_the_first() {
        assert_eq!(
            push_url(output(0, "one\ntwo\n")),
            Err("has 2 push URLs; anchoring needs exactly one".into())
        );
    }
    #[test]
    fn failed_push_url_lookup_cannot_use_its_stdout() {
        assert_eq!(
            push_url(Output::exited(128, "one\n", "lookup failed")),
            Err("push URL lookup failed: lookup failed".into())
        );
    }
    #[test]
    fn several_push_urls_stop_before_writing_objects() {
        let mut f = forge(Recorded::new().out("origin\n").out("one\ntwo\n"));
        assert_eq!(
            f.push_tag(&p(), "origin", "TAG", "note\n"),
            PushObservation::Unreachable
        );
        assert_eq!(
            f.last_error.as_deref(),
            Some("remote origin has 2 push URLs; anchoring needs exactly one")
        );
        assert_eq!(f.process.launches().len(), 2);
    }
    #[test]
    fn tag_object_has_the_empty_tree_and_fixed_tagger() {
        assert_eq!(
            tag_object_text("TREE", "TAG", "annotation\n", 123),
            "object TREE\ntype tree\ntag TAG\ntagger Baley <baley@localhost> 123 +0000\n\nannotation\n"
        );
    }
    #[test]
    fn reads_cannot_use_a_fetch_url_the_push_never_wrote_to() {
        let mut f = forge(
            Recorded::new()
                .out("origin\n")
                .out("/fixture/push.git\n")
                .out(listed(true))
                .out("")
                .out("object x\ntype tree\ntag baley-anchor/p/5\n\nnote\n"),
        );
        assert_eq!(
            f.fetch_tag(&p(), "origin", &exact()),
            FetchObservation::Present {
                tag: "baley-anchor/p/5".into(),
                annotation: "note\n".into()
            }
        );
        let launches = f.process.launches();
        assert_eq!(
            launches[1].args,
            strings(&["remote", "get-url", "--push", "--all", "origin"])
                .iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(launches[2].args[3], "/fixture/push.git");
        assert_eq!(launches[3].args[4], "/fixture/push.git");
    }
    #[test]
    fn several_push_urls_stop_a_read_before_listing() {
        let mut f = forge(Recorded::new().out("origin\n").out("one\ntwo\n"));
        assert_eq!(
            f.fetch_tag(&p(), "origin", &exact()),
            FetchObservation::Unreachable
        );
        assert_eq!(f.process.launches().len(), 2);
    }
    #[test]
    fn an_absent_listing_launches_no_fetch() {
        let mut f = forge(
            Recorded::new()
                .out("o\n")
                .out("/fixture/remote.git\n")
                .answer(output(2, "")),
        );
        assert_eq!(f.fetch_tag(&p(), "o", &exact()), FetchObservation::Absent);
        assert_eq!(f.process.launches().len(), 3);
    }
    #[test]
    fn a_failed_listing_is_not_absence() {
        assert_eq!(
            listing(&p(), &exact(), &output(128, "")),
            Listing::Unreachable
        );
    }
    #[test]
    fn latest_is_numeric_not_lexicographic() {
        let lines = [9, 10, 5]
            .map(|n| format!("{}\trefs/tags/baley-anchor/p/{n}\n", "a".repeat(40)))
            .concat();
        assert!(
            matches!(listing(&p(),&TagQuery::LatestAnchor,&output(0,&lines)),Listing::Found { tag,.. } if tag == "baley-anchor/p/10")
        );
    }
    #[test]
    fn latest_ignores_peeled_and_foreign_names() {
        let lines = format!(
            "{}\trefs/tags/baley-anchor/p/99^{{}}\n{}\trefs/tags/baley-anchor/other/100\n{}\trefs/tags/baley-anchor/p/05\n{}",
            "a".repeat(40),
            "a".repeat(40),
            "a".repeat(40),
            listed(false)
        );
        assert!(
            matches!(listing(&p(),&TagQuery::LatestAnchor,&output(0,&lines)),Listing::Found { tag,.. } if tag == "baley-anchor/p/5")
        );
    }
    #[test]
    fn lightweight_tags_are_present_without_fetch() {
        let mut f = forge(
            Recorded::new()
                .out("o\n")
                .out("/fixture/remote.git\n")
                .out(listed(false)),
        );
        assert_eq!(
            f.fetch_tag(&p(), "o", &exact()),
            FetchObservation::Present {
                tag: "baley-anchor/p/5".into(),
                annotation: "".into()
            }
        );
        assert_eq!(f.process.launches().len(), 3);
    }
    #[test]
    fn a_failed_fetch_is_not_absence() {
        let mut f = forge(
            Recorded::new()
                .out("o\n")
                .out("/fixture/remote.git\n")
                .out(listed(true))
                .fail(128, "offline"),
        );
        assert_eq!(
            f.fetch_tag(&p(), "o", &exact()),
            FetchObservation::Unreachable
        );
    }
    #[test]
    fn tag_message_keeps_a_signature_after_the_annotation() {
        assert_eq!(
            tag_message("object TREE\ntype tree\n\nannotation\nSIGNATURE\n"),
            "annotation\nSIGNATURE\n"
        );
    }
    #[test]
    fn exact_does_not_accept_a_longer_matching_ref() {
        let lines = format!("{}\trefs/tags/prefix/baley-anchor/p/5\n", "a".repeat(40));
        assert_eq!(listing(&p(), &exact(), &output(0, &lines)), Listing::Absent);
    }
    #[test]
    fn a_truncated_listing_is_unreachable() {
        let mut o = output(0, &listed(true));
        o.stdout_complete = false;
        assert_eq!(listing(&p(), &exact(), &o), Listing::Unreachable);
    }
    #[test]
    fn launches_never_prompt_and_use_the_anchor_deadline() {
        let mut f = forge(
            Recorded::new()
                .out("o\n")
                .out("/fixture/remote.git\n")
                .out(listed(true))
                .out("")
                .out("header\n\nnote\n"),
        );
        f.fetch_tag(&p(), "o", &exact());
        assert_eq!(f.process.launches().len(), 5);
        for l in f.process.launches() {
            assert_eq!(l.git_caller(), Some(Caller::AnchorForge));
            assert_eq!(l.timeout, Some(std::time::Duration::from_secs(60)));
            assert!(l.own_group);
            assert_eq!(l.limit, 16 * 1024 * 1024);
            assert!(
                l.env
                    .contains(&("GIT_TERMINAL_PROMPT".into(), Some("0".into())))
            );
        }
    }
    #[test]
    fn exact_listing_requests_the_peeled_pattern() {
        assert_eq!(
            ls_remote_args("o", &p(), &exact()),
            [
                "ls-remote",
                "--tags",
                "--exit-code",
                "o",
                "refs/tags/baley-anchor/p/5",
                "refs/tags/baley-anchor/p/5^{}"
            ]
        );
    }
    #[test]
    fn exact_peeled_line_marks_an_annotated_tag() {
        assert!(matches!(
            listing(&p(), &exact(), &output(0, &listed(true))),
            Listing::Found {
                annotated: true,
                ..
            }
        ));
    }
    #[test]
    fn fetch_disables_configured_refspecs() {
        let mut f = forge(
            Recorded::new()
                .out("o\n")
                .out("/fixture/remote.git\n")
                .out(listed(true))
                .out("")
                .out("header\n\nnote\n"),
        );
        f.fetch_tag(&p(), "o", &exact());
        assert_eq!(
            f.process.launches()[3].args,
            strings(&[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--refmap=",
                "/fixture/remote.git",
                "refs/tags/baley-anchor/p/5"
            ])
            .iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
        );
    }
    #[test]
    fn malformed_tree_ids_never_reach_mktag() {
        let mut f = forge(
            Recorded::new()
                .out("o\n")
                .out("/fixture/remote.git\n")
                .out("not-an-object\n"),
        );
        assert_eq!(
            f.push_tag(&p(), "o", "tag", "note\n"),
            PushObservation::Unreachable
        );
        assert_eq!(f.process.launches().len(), 3);
    }
    #[test]
    fn sha256_git_object_ids_are_readable() {
        let lines = format!("{}\trefs/tags/baley-anchor/p/5\n", "a".repeat(64));
        assert!(
            matches!(listing(&p(),&exact(),&output(0,&lines)),Listing::Found { sha,.. } if sha == "a".repeat(64))
        );
    }
    #[test]
    fn missing_repository_refusal_names_the_lookup_location() {
        let mut f = forge(Recorded::new().fail(128, "not a repository"));
        assert_eq!(
            f.require_remote("o").unwrap_err().to_string(),
            "baley: the current directory is not in a git repository; remotes are read from the repository at the current directory"
        );
    }
    #[test]
    fn missing_remote_refusal_names_the_remote_and_directory() {
        let mut f = forge(Recorded::new().out("other\n"));
        assert_eq!(
            f.require_remote("o").unwrap_err().to_string(),
            "baley: remote o is not configured in the git repository at /fixture"
        );
    }
}
