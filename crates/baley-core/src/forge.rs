//! The forge seam for anchors and the anchor tag codec (design 0001, The
//! hash chain and anchors; ADR 0007).
//!
//! The core asks the forge for two things only: push an anchor tag, and
//! fetch one. Each answer is an observation, never an error: a refused or
//! unreachable remote is a fact the anchor command records. The binary
//! implements the seam over git; nothing under test runs git, and no store
//! crate reaches a forge.

use baley_store::{ANCHOR_TAG_PREFIX, Anchor, Hash, ProjectId};
use serde_json::Value;

pub use baley_store::anchor_tag;

/// Which tag to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagQuery {
    /// Exactly this tag.
    Exact(String),
    /// The project's anchor tag with the highest numeric sequence.
    LatestAnchor,
}

/// What pushing a tag observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushObservation {
    /// The remote holds the tag.
    Pushed,
    /// The remote answered and refused the push.
    Refused {
        /// What the remote said.
        reason: String,
    },
    /// The project has no remote to push to.
    NoRemote,
    /// The remote could not be reached.
    Unreachable,
}

/// What fetching a tag observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchObservation {
    /// The remote holds the tag; `annotation` is its raw message, any
    /// signature included.
    Present {
        /// The tag found.
        tag: String,
        /// Its raw message.
        annotation: String,
    },
    /// The remote answered and holds no such tag.
    Absent,
    /// The project has no remote to fetch from.
    NoRemote,
    /// The remote could not be reached.
    Unreachable,
}

/// The forge as the anchor command sees it.
pub trait Forge {
    /// Pushes `tag` with `annotation` to the configured remote named
    /// `remote`.
    fn push_tag(
        &mut self,
        project: &ProjectId,
        remote: &str,
        tag: &str,
        annotation: &str,
    ) -> PushObservation;

    /// Fetches one tag from the configured remote named `remote`.
    fn fetch_tag(
        &mut self,
        project: &ProjectId,
        remote: &str,
        query: &TagQuery,
    ) -> FetchObservation;
}

/// The annotation of an anchor tag: one line of canonical JSON,
/// `{"head":"<64 lower-case hex>","seq":<decimal>}`, and a line feed.
pub fn anchor_annotation(anchor: &Anchor) -> String {
    format!(
        "{{\"head\":\"{}\",\"seq\":{}}}\n",
        anchor.hash.to_hex(),
        anchor.seq
    )
}

/// The anchor an annotation names. Only the first line is read, so a tag
/// signature after it is ignored; that line must be exactly the canonical
/// form `anchor_annotation` writes.
pub fn parse_annotation(annotation: &str) -> Option<Anchor> {
    let line = annotation.split('\n').next()?;
    let value: Value = serde_json::from_str(line).ok()?;
    let object = value.as_object()?;
    let anchor = Anchor {
        seq: object.get("seq")?.as_u64()?,
        hash: Hash::from_hex(object.get("head")?.as_str()?)?,
    };
    // Byte equality with the canonical line refuses extra keys, spacing,
    // upper-case hex and any other spelling of the same values.
    (anchor_annotation(&anchor).trim_end_matches('\n') == line).then_some(anchor)
}

/// The sequence a tag of `project` names, when it is exactly
/// `baley-anchor/<project_id>/<seq>` with `seq` in plain decimal.
pub fn tag_sequence(project: &ProjectId, tag: &str) -> Option<u64> {
    let digits = tag
        .strip_prefix(ANCHOR_TAG_PREFIX)?
        .strip_prefix(project.0.as_str())?
        .strip_prefix('/')?;
    let seq: u64 = digits.parse().ok()?;
    (seq.to_string() == digits).then_some(seq)
}

/// The anchor a fetched tag of `project` holds: a well-formed tag name and
/// annotation that name the same sequence. `None` for anything else, which
/// is a tag that exists but is not an anchor.
pub fn parse_tag_anchor(project: &ProjectId, tag: &str, annotation: &str) -> Option<Anchor> {
    let anchor = parse_annotation(annotation)?;
    (tag_sequence(project, tag)? == anchor.seq).then_some(anchor)
}

#[cfg(test)]
pub(crate) mod fake {
    //! An in-memory remote for tests of the seam: named, immutable tag
    //! annotations, scripted failures and a record of every call.

    use std::collections::BTreeMap;

    use super::*;

    /// One call the fake received.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum Call {
        Push { remote: String, tag: String },
        Fetch { remote: String, query: TagQuery },
    }

    /// How the fake remote answers.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Reach {
        Up,
        NoRemote,
        Unreachable,
    }

    pub(crate) struct FakeForge {
        pub(crate) tags: BTreeMap<String, String>,
        pub(crate) reach: Reach,
        /// When set, the next push is refused with this reason.
        pub(crate) refuse_next: Option<String>,
        pub(crate) calls: Vec<Call>,
    }

    impl FakeForge {
        pub(crate) fn new() -> Self {
            Self {
                tags: BTreeMap::new(),
                reach: Reach::Up,
                refuse_next: None,
                calls: Vec::new(),
            }
        }
    }

    impl Forge for FakeForge {
        fn push_tag(
            &mut self,
            _project: &ProjectId,
            remote: &str,
            tag: &str,
            annotation: &str,
        ) -> PushObservation {
            self.calls.push(Call::Push {
                remote: remote.into(),
                tag: tag.into(),
            });
            match self.reach {
                Reach::NoRemote => return PushObservation::NoRemote,
                Reach::Unreachable => return PushObservation::Unreachable,
                Reach::Up => {}
            }
            if let Some(reason) = self.refuse_next.take() {
                return PushObservation::Refused { reason };
            }
            // Tags are immutable, as the ruleset makes them.
            if self.tags.contains_key(tag) {
                return PushObservation::Refused {
                    reason: "tag exists".into(),
                };
            }
            self.tags.insert(tag.into(), annotation.into());
            PushObservation::Pushed
        }

        fn fetch_tag(
            &mut self,
            project: &ProjectId,
            remote: &str,
            query: &TagQuery,
        ) -> FetchObservation {
            self.calls.push(Call::Fetch {
                remote: remote.into(),
                query: query.clone(),
            });
            match self.reach {
                Reach::NoRemote => return FetchObservation::NoRemote,
                Reach::Unreachable => return FetchObservation::Unreachable,
                Reach::Up => {}
            }
            let found = match query {
                TagQuery::Exact(tag) => self.tags.get_key_value(tag),
                TagQuery::LatestAnchor => self
                    .tags
                    .iter()
                    .filter_map(|(tag, annotation)| {
                        tag_sequence(project, tag).map(|seq| (seq, (tag, annotation)))
                    })
                    .max_by_key(|(seq, _)| *seq)
                    .map(|(_, found)| found),
            };
            match found {
                Some((tag, annotation)) => FetchObservation::Present {
                    tag: tag.clone(),
                    annotation: annotation.clone(),
                },
                None => FetchObservation::Absent,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Call, FakeForge, Reach};
    use super::*;

    const PROJECT: &str = "3f2b1a9c-6d4e-4f0a-9b8c-7d6e5f4a3b2c";

    fn project() -> ProjectId {
        ProjectId(PROJECT.into())
    }

    fn anchor(seq: u64) -> Anchor {
        Anchor {
            seq,
            hash: Hash([0x5a; 32]),
        }
    }

    // The annotation's bytes are the canonical line and one line feed,
    // written out by hand. Catches key order, spacing or case that T14's
    // git adapter and a later reader would not agree on.
    #[test]
    fn the_annotation_is_one_canonical_line() {
        assert_eq!(
            anchor_annotation(&anchor(7)),
            format!("{{\"head\":\"{}\",\"seq\":7}}\n", "5a".repeat(32))
        );
    }

    // An annotation reads back, and a signature after the first line is
    // ignored. Catches a parser that reads the whole message and fails on
    // a signed tag.
    #[test]
    fn the_first_line_round_trips_and_the_rest_is_ignored() {
        let signed = format!(
            "{}-----BEGIN PGP SIGNATURE-----\nabc\n-----END PGP SIGNATURE-----\n",
            anchor_annotation(&anchor(7))
        );
        assert_eq!(
            parse_annotation(&anchor_annotation(&anchor(7))),
            Some(anchor(7))
        );
        assert_eq!(parse_annotation(&signed), Some(anchor(7)));
    }

    // Other spellings of the same values are not the canonical line.
    // Catches a lenient parser that lets a hand-made tag pass as an anchor.
    #[test]
    fn a_non_canonical_first_line_is_refused() {
        let hex = "5a".repeat(32);
        for line in [
            format!("{{\"seq\":7,\"head\":\"{hex}\"}}"),
            format!("{{\"head\": \"{hex}\",\"seq\":7}}"),
            format!("{{\"head\":\"{}\",\"seq\":7}}", hex.to_uppercase()),
            format!("{{\"head\":\"{hex}\",\"seq\":\"7\"}}"),
            format!("{{\"head\":\"{hex}\",\"seq\":7,\"x\":1}}"),
        ] {
            assert_eq!(parse_annotation(&line), None, "{line}");
        }
    }

    // The tag for sequence 7 is the design's name; one naming another
    // sequence or another project does not agree. Catches the claim
    // event's sequence in the tag and a tag read across projects.
    #[test]
    fn the_tag_and_annotation_must_name_the_same_sequence() {
        let tag = anchor_tag(&project(), 7);
        assert_eq!(tag, format!("baley-anchor/{PROJECT}/7"));
        let annotation = anchor_annotation(&anchor(7));
        assert_eq!(
            parse_tag_anchor(&project(), &tag, &annotation),
            Some(anchor(7))
        );
        assert_eq!(
            parse_tag_anchor(&project(), &anchor_tag(&project(), 8), &annotation),
            None
        );
        assert_eq!(
            parse_tag_anchor(&ProjectId("other".into()), &tag, &annotation),
            None
        );
        assert_eq!(
            tag_sequence(&project(), &format!("baley-anchor/{PROJECT}/07")),
            None
        );
    }

    // Each push outcome comes back as its own variant. Catches a clean
    // failure observation that the command could not tell apart.
    #[test]
    fn the_fake_push_distinguishes_every_outcome() {
        let tag = anchor_tag(&project(), 7);
        let note = anchor_annotation(&anchor(7));
        let mut forge = FakeForge::new();
        forge.refuse_next = Some("ruleset".into());
        assert_eq!(
            forge.push_tag(&project(), "origin", &tag, &note),
            PushObservation::Refused {
                reason: "ruleset".into()
            }
        );
        assert_eq!(
            forge.push_tag(&project(), "origin", &tag, &note),
            PushObservation::Pushed
        );
        forge.reach = Reach::NoRemote;
        assert_eq!(
            forge.push_tag(&project(), "origin", &tag, &note),
            PushObservation::NoRemote
        );
        forge.reach = Reach::Unreachable;
        assert_eq!(
            forge.push_tag(&project(), "origin", &tag, &note),
            PushObservation::Unreachable
        );
    }

    // An exact fetch returns only the named tag, and records its query.
    // Catches a reconciliation that reads another claim's tag.
    #[test]
    fn an_exact_fetch_returns_only_the_named_tag() {
        let mut forge = FakeForge::new();
        for seq in [3, 9] {
            forge
                .tags
                .insert(anchor_tag(&project(), seq), anchor_annotation(&anchor(seq)));
        }
        let query = TagQuery::Exact(anchor_tag(&project(), 3));
        assert_eq!(
            forge.fetch_tag(&project(), "origin", &query),
            FetchObservation::Present {
                tag: anchor_tag(&project(), 3),
                annotation: anchor_annotation(&anchor(3))
            }
        );
        assert_eq!(
            forge.calls,
            [Call::Fetch {
                remote: "origin".into(),
                query
            }]
        );
    }

    // The latest anchor is the highest number, not the last in text order:
    // 10 after 9. Another project's higher tag is not this project's.
    // Catches lexicographic selection.
    #[test]
    fn the_latest_fetch_selects_the_highest_numeric_sequence() {
        let mut forge = FakeForge::new();
        for seq in [9, 10, 2] {
            forge
                .tags
                .insert(anchor_tag(&project(), seq), anchor_annotation(&anchor(seq)));
        }
        forge.tags.insert(
            anchor_tag(&ProjectId("other".into()), 99),
            anchor_annotation(&anchor(99)),
        );
        let FetchObservation::Present { tag, .. } =
            forge.fetch_tag(&project(), "origin", &TagQuery::LatestAnchor)
        else {
            panic!("expected a present tag");
        };
        assert_eq!(tag, anchor_tag(&project(), 10));
    }

    // A missing tag is absent only when the remote answered; a remote that
    // cannot be read, or none at all, is its own observation. Catches a
    // failed read turned into confirmed absence.
    #[test]
    fn absent_unreachable_and_no_remote_stay_distinct() {
        let mut forge = FakeForge::new();
        let query = TagQuery::Exact(anchor_tag(&project(), 3));
        assert_eq!(
            forge.fetch_tag(&project(), "origin", &query),
            FetchObservation::Absent
        );
        forge.reach = Reach::Unreachable;
        assert_eq!(
            forge.fetch_tag(&project(), "origin", &query),
            FetchObservation::Unreachable
        );
        forge.reach = Reach::NoRemote;
        assert_eq!(
            forge.fetch_tag(&project(), "origin", &query),
            FetchObservation::NoRemote
        );
    }
}
