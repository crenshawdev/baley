//! The hash chain and its pure verifier (design 0001, The hash chain and
//! anchors; EVD-R3).
//!
//! - `hash(1) = SHA-256("baley-ledger/1" || project_id || JCS(envelope(1)) || JCS(payload(1)))`
//! - `hash(n) = SHA-256(hash(n-1) || JCS(envelope(n)) || JCS(payload(n)))`
//!
//! `project_id` enters as its UTF-8 text, `hash(n-1)` as its raw 32 bytes.
//! Nothing here reads storage: the adapter feeds rows in and gets a report
//! out, then goes on to the payload bodies on its own (Build 1, decision 3).

use std::ops::RangeInclusive;

use sha2::{Digest, Sha256};

use crate::canonical::CanonicalError;
use crate::event::{Event, Hash, ProjectId};

/// The domain prefix of the first hash of every chain.
pub const DOMAIN: &[u8] = b"baley-ledger/1";

/// The design's formula over already canonical bytes. `prev` is `None`
/// only for the first event of a project.
pub fn chain_hash(
    prev: Option<&Hash>,
    project_id: &ProjectId,
    envelope: &[u8],
    payload: &[u8],
) -> Hash {
    let mut hasher = Sha256::new();
    match prev {
        None => {
            hasher.update(DOMAIN);
            hasher.update(project_id.0.as_bytes());
        }
        Some(prev) => hasher.update(prev.0),
    }
    hasher.update(envelope);
    hasher.update(payload);
    Hash(hasher.finalize().into())
}

/// A head pushed outside the machine: the sequence and the hash at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub seq: u64,
    pub hash: Hash,
}

/// The last event the verifier accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub seq: u64,
    pub hash: Hash,
}

/// What is wrong at the first bad sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakKind {
    /// The row at this position carries another sequence: the expected one
    /// was deleted, or rows were reordered or duplicated. `found` is the
    /// sequence the row carries.
    Sequence { found: u64 },
    /// The event names a project other than the one the chain started in.
    Project { expected: ProjectId },
    /// The stored `prev_hash` is not the recomputed hash of the predecessor.
    PrevHash {
        expected: Option<Hash>,
        found: Option<Hash>,
    },
    /// The stored `hash` is not what the formula gives from the event's own
    /// fields.
    Hash { expected: Hash, found: Hash },
    /// The event's envelope or payload has no canonical form, so no hash.
    Canonical(CanonicalError),
}

/// The first bad position in the ledger and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Break {
    /// The sequence that should have been next: the first position the
    /// verifier could not accept.
    pub seq: u64,
    pub kind: BreakKind,
}

/// How the recomputed chain compares with the anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorVerdict {
    /// No anchor was supplied: the whole chain is checked locally only.
    NoAnchor,
    /// The recomputed hash at the anchored sequence is the anchored hash.
    Matches,
    /// Every row verified and the chain ends before the anchored sequence.
    Truncated { anchored: u64, head: u64 },
    /// The accepted chain reaches the anchored sequence with another hash:
    /// a rewrite or a rollback.
    Rewritten {
        seq: u64,
        anchored: Hash,
        found: Hash,
    },
    /// A break at or before the anchored sequence stopped the walk, so the
    /// anchor was never compared. The break is in `first_break`.
    Unchecked { anchored: u64 },
}

/// The verifier's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainReport {
    /// The last accepted event; `None` for an empty chain or a break at 1.
    pub head: Option<Head>,
    /// The first position the verifier refused, if any. Events after it
    /// are not examined.
    pub first_break: Option<Break>,
    pub anchor: AnchorVerdict,
    /// The accepted sequences no anchor covers: after the anchored
    /// sequence, or the whole chain without an anchor. `None` when empty.
    pub unanchored: Option<RangeInclusive<u64>>,
}

impl ChainReport {
    /// True when every event was accepted and the anchor, if any, matches.
    pub fn is_intact(&self) -> bool {
        self.first_break.is_none()
            && matches!(
                self.anchor,
                AnchorVerdict::NoAnchor | AnchorVerdict::Matches
            )
    }
}

/// Walks `events` in the order given, recomputing every hash from the
/// event's own fields, and compares the result with `anchor`. The stored
/// `prev_hash` and `hash` are checked against the recomputation, never
/// trusted: a chain whose stored links agree with each other but not with
/// its contents is broken at the first event whose recomputed hash differs.
pub fn verify_chain<'a>(
    events: impl IntoIterator<Item = &'a Event>,
    anchor: Option<&Anchor>,
) -> ChainReport {
    let mut head: Option<Head> = None;
    let mut project: Option<ProjectId> = None;
    let mut first_break = None;
    // Set when the walk accepts the anchored sequence.
    let mut reached: Option<AnchorVerdict> = None;

    for event in events {
        let expected_seq = head.as_ref().map_or(1, |head| head.seq + 1);
        let expected_prev = head.as_ref().map(|head| head.hash);
        let kind = if event.seq != expected_seq {
            Some(BreakKind::Sequence { found: event.seq })
        } else if project
            .as_ref()
            .is_some_and(|project| *project != event.project_id)
        {
            Some(BreakKind::Project {
                expected: project.clone().expect("checked"),
            })
        } else if event.prev_hash != expected_prev {
            Some(BreakKind::PrevHash {
                expected: expected_prev,
                found: event.prev_hash,
            })
        } else {
            match event.compute_hash() {
                Err(error) => Some(BreakKind::Canonical(error)),
                Ok(expected) if expected != event.hash => Some(BreakKind::Hash {
                    expected,
                    found: event.hash,
                }),
                Ok(hash) => {
                    project.get_or_insert_with(|| event.project_id.clone());
                    head = Some(Head {
                        seq: event.seq,
                        hash,
                    });
                    if let Some(anchor) = anchor
                        && anchor.seq == event.seq
                    {
                        reached = Some(if anchor.hash == hash {
                            AnchorVerdict::Matches
                        } else {
                            AnchorVerdict::Rewritten {
                                seq: anchor.seq,
                                anchored: anchor.hash,
                                found: hash,
                            }
                        });
                    }
                    None
                }
            }
        };
        if let Some(kind) = kind {
            first_break = Some(Break {
                seq: expected_seq,
                kind,
            });
            break;
        }
    }

    let head_seq = head.as_ref().map_or(0, |head| head.seq);
    let anchor_verdict = match (anchor, reached) {
        (None, _) => AnchorVerdict::NoAnchor,
        (Some(_), Some(verdict)) => verdict,
        (Some(anchor), None) if first_break.is_some() => AnchorVerdict::Unchecked {
            anchored: anchor.seq,
        },
        (Some(anchor), None) => AnchorVerdict::Truncated {
            anchored: anchor.seq,
            head: head_seq,
        },
    };
    let unanchored_from = match anchor {
        None => 1,
        Some(anchor) => anchor.seq + 1,
    };
    let unanchored = (head_seq >= unanchored_from).then_some(unanchored_from..=head_seq);

    ChainReport {
        head,
        first_break,
        anchor: anchor_verdict,
        unanchored,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::event::{Actor, EventDraft, GitFacts, RequestId};

    const PROJECT: &str = "3f2b1a9c-6d4e-4f0a-9b8c-7d6e5f4a3b2c";

    fn draft(seq: u64) -> EventDraft {
        EventDraft {
            stream: "phase/1".into(),
            stream_version: seq,
            type_name: "phase.declared".into(),
            type_version: 1,
            actor: Actor::Owner,
            recorded_at: "2026-09-25T18:00:00Z".into(),
            request_id: RequestId(format!("00000000-0000-4000-8000-00000000000{seq}")),
            git: None,
            policy_version: 1,
            payload: json!({"phase": 1, "title": "Foundation"}),
        }
    }

    fn chain(length: u64) -> Vec<Event> {
        let mut events: Vec<Event> = Vec::new();
        for seq in 1..=length {
            let prev = events.last().map(|event| event.hash);
            events
                .push(Event::seal(ProjectId(PROJECT.into()), seq, prev, draft(seq)).expect("seal"));
        }
        events
    }

    fn hex(text: &str) -> Hash {
        Hash::from_hex(text).expect("hex")
    }

    // The genesis hash, computed outside this crate from the design's
    // formula: printf of "baley-ledger/1", the project id, the canonical
    // envelope and the canonical payload piped through sha256sum. The
    // envelope bytes are written here by hand from the design's table.
    // Catches a formula that drops the domain prefix, the project id or a
    // field, or hashes non-canonical bytes.
    #[test]
    fn genesis_hash_matches_the_formula_by_hand() {
        let draft = EventDraft {
            git: Some(GitFacts {
                commit: "0a1b2c3d".into(),
                tree: "4e5f6a7b".into(),
                checkout: "/code/baley".into(),
            }),
            ..draft(1)
        };
        let event = Event::seal(ProjectId(PROJECT.into()), 1, None, draft).expect("seal");
        assert_eq!(
            String::from_utf8(event.canonical_envelope().expect("envelope")).expect("utf-8"),
            concat!(
                r#"{"actor":"owner","git":{"checkout":"/code/baley","commit":"0a1b2c3d","tree":"4e5f6a7b"},"#,
                r#""policy_version":1,"prev_hash":null,"project_id":"3f2b1a9c-6d4e-4f0a-9b8c-7d6e5f4a3b2c","#,
                r#""recorded_at":"2026-09-25T18:00:00Z","request_id":"00000000-0000-4000-8000-000000000001","#,
                r#""seq":1,"stream":"phase/1","stream_version":1,"type":"phase.declared","type_version":1}"#
            )
        );
        assert_eq!(event.hash, hex(GENESIS_HASH));
    }

    // The second hash chains the raw bytes of the first, with no domain
    // prefix and no project id. Computed outside this crate the same way,
    // with the first hash's bytes from xxd. Catches a formula that repeats
    // the prefix, chains the hex text, or ignores the predecessor.
    #[test]
    fn second_hash_chains_the_first_by_hand() {
        let events = chain(2);
        assert_eq!(events[0].hash, hex(GENESIS_HASH_NO_GIT));
        assert_eq!(
            String::from_utf8(events[1].canonical_envelope().expect("envelope")).expect("utf-8"),
            concat!(
                r#"{"actor":"owner","policy_version":1,"prev_hash":""#,
                "2e8b0f17b8239b2c583c92e06aea027478083cba903cebd46dc70ff7e7afa5d6",
                r#"","project_id":"3f2b1a9c-6d4e-4f0a-9b8c-7d6e5f4a3b2c","recorded_at":"2026-09-25T18:00:00Z","#,
                r#""request_id":"00000000-0000-4000-8000-000000000002","seq":2,"stream":"phase/1","#,
                r#""stream_version":2,"type":"phase.declared","type_version":1}"#
            )
        );
        assert_eq!(events[1].hash, hex(SECOND_HASH));
    }

    const GENESIS_HASH: &str = "3ea5d953b87d75ba33d12833b83e5655322060a5a3f14e1193cec86852f2360a";
    const GENESIS_HASH_NO_GIT: &str =
        "2e8b0f17b8239b2c583c92e06aea027478083cba903cebd46dc70ff7e7afa5d6";
    const SECOND_HASH: &str = "9bdd1d9b39bc7861fa37c48f8328b7f10b2f89d70a55b3072aeef6fd867722da";
    // The third event of `chain`, and the same event after its payload's
    // title is edited to "Foundations", both computed outside this crate
    // the same way as the second.
    const THIRD_HASH: &str = "3da1e818903e1177bc193c9a6b1028e47aac2093888bcafac2bb279ca28626bc";
    const EDITED_THIRD_HASH: &str =
        "09109e32f9c1cb95a2939737362c275c45d70c725db7cd6a9449045a3f4a6bc5";

    // An untouched chain verifies with no break, its head at the end, and
    // the whole range unanchored. Catches a verifier that reports a break
    // on a good chain or forgets the unanchored range.
    #[test]
    fn an_intact_chain_without_an_anchor_is_wholly_unanchored() {
        let events = chain(3);
        let report = verify_chain(&events, None);
        assert_eq!(report.first_break, None);
        assert_eq!(
            report.head,
            Some(Head {
                seq: 3,
                hash: hex(THIRD_HASH)
            })
        );
        assert_eq!(report.anchor, AnchorVerdict::NoAnchor);
        assert_eq!(report.unanchored, Some(1..=3));
        assert!(report.is_intact());
    }

    // Editing the payload at n, leaving every stored hash as it was, is
    // reported at n: the recomputed hash differs there. Catches a verifier
    // that only follows prev_hash links.
    #[test]
    fn an_edit_at_n_is_reported_at_n() {
        let mut events = chain(4);
        events[2].payload = json!({"phase": 1, "title": "Foundations"});
        let report = verify_chain(&events, None);
        let Some(Break {
            seq,
            kind: BreakKind::Hash { expected, found },
        }) = report.first_break
        else {
            panic!("expected a hash break, got {:?}", report.first_break);
        };
        assert_eq!(seq, 3);
        assert_eq!(found, hex(THIRD_HASH));
        assert_eq!(expected, hex(EDITED_THIRD_HASH));
        assert_eq!(
            report.head,
            Some(Head {
                seq: 2,
                hash: hex(SECOND_HASH)
            })
        );
        assert_eq!(report.unanchored, Some(1..=2));
    }

    // A chain whose stored prev_hash and hash values agree with each other
    // but not with the events' contents is refused at the first such event.
    // Every stored link is consistent; only recomputation tells. Catches a
    // verifier that trusts the stored fields.
    #[test]
    fn consistent_stored_links_over_wrong_contents_are_refused() {
        let mut events = chain(3);
        let forged = Hash([0x42; 32]);
        events[1].hash = forged;
        events[2].prev_hash = Some(forged);
        events[2].hash = events[2].compute_hash().expect("hash");
        let report = verify_chain(&events, None);
        assert_eq!(
            report.first_break,
            Some(Break {
                seq: 2,
                kind: BreakKind::Hash {
                    expected: hex(SECOND_HASH),
                    found: forged
                }
            })
        );
    }

    // A deleted event is reported at its own sequence, the first bad
    // position, with the sequence of the row found there instead. Catches
    // a verifier that renumbers as it goes or blames the row after the gap.
    #[test]
    fn a_missing_sequence_is_a_break_at_the_gap() {
        let mut events = chain(3);
        events.remove(1);
        let report = verify_chain(&events, None);
        assert_eq!(
            report.first_break,
            Some(Break {
                seq: 2,
                kind: BreakKind::Sequence { found: 3 }
            })
        );
        assert_eq!(report.head.map(|head| head.seq), Some(1));
    }

    // An anchor past the accepted head means the chain was truncated.
    // Catches a verifier that treats a short chain as unanchored only.
    #[test]
    fn an_anchor_past_the_head_reports_truncation() {
        let events = chain(2);
        let anchor = Anchor {
            seq: 5,
            hash: Hash([0x11; 32]),
        };
        let report = verify_chain(&events, Some(&anchor));
        assert_eq!(report.first_break, None);
        assert_eq!(
            report.anchor,
            AnchorVerdict::Truncated {
                anchored: 5,
                head: 2
            }
        );
        assert_eq!(report.unanchored, None);
        assert!(!report.is_intact());
    }

    // The anchored sequence is present but its recomputed hash is not the
    // anchored one: a rewrite or rollback, with the unanchored range after
    // it still reported. Catches a verifier that compares the stored hash
    // instead of the recomputed one, or that stops at the anchor.
    #[test]
    fn a_different_hash_at_the_anchored_sequence_reports_rewrite() {
        let events = chain(4);
        let anchored = Hash([0x99; 32]);
        let anchor = Anchor {
            seq: 2,
            hash: anchored,
        };
        let report = verify_chain(&events, Some(&anchor));
        assert_eq!(report.first_break, None);
        assert_eq!(
            report.anchor,
            AnchorVerdict::Rewritten {
                seq: 2,
                anchored,
                found: hex(SECOND_HASH)
            }
        );
        assert_eq!(report.unanchored, Some(3..=4));
    }

    // An anchor that matches the stored hash of an event whose contents
    // were edited is not a match: the recomputed hash is what counts, and
    // the break at the anchored sequence leaves the anchor unchecked.
    // Catches a verifier that compares the anchor with the stored hash.
    #[test]
    fn an_anchor_matching_only_the_stored_hash_is_not_a_match() {
        let mut events = chain(3);
        let anchor = Anchor {
            seq: 2,
            hash: hex(SECOND_HASH),
        };
        events[1].payload = json!({"phase": 1, "title": "Foundations"});
        let report = verify_chain(&events, Some(&anchor));
        assert_eq!(report.first_break.as_ref().map(|at| at.seq), Some(2));
        assert_eq!(report.anchor, AnchorVerdict::Unchecked { anchored: 2 });
    }

    // A break before the anchored sequence stops the walk short of the
    // anchor, and the rows after the break are still there, so the verdict
    // is unchecked, not truncation. Catches a report that calls a broken
    // chain truncated.
    #[test]
    fn a_break_before_the_anchor_leaves_it_unchecked() {
        let mut events = chain(4);
        events[1].payload = json!({"phase": 1, "title": "Foundations"});
        let anchor = Anchor {
            seq: 3,
            hash: hex(THIRD_HASH),
        };
        let report = verify_chain(&events, Some(&anchor));
        assert_eq!(report.first_break.as_ref().map(|at| at.seq), Some(2));
        assert_eq!(report.anchor, AnchorVerdict::Unchecked { anchored: 3 });
        assert!(!report.is_intact());
    }

    // A matching anchor leaves only the sequences after it unanchored, and
    // an anchor at the head leaves none. Catches an off-by-one at the
    // anchored sequence.
    #[test]
    fn a_matching_anchor_bounds_the_unanchored_range() {
        let events = chain(4);
        let at_two = Anchor {
            seq: 2,
            hash: events[1].hash,
        };
        let report = verify_chain(&events, Some(&at_two));
        assert_eq!(report.anchor, AnchorVerdict::Matches);
        assert_eq!(report.unanchored, Some(3..=4));
        assert!(report.is_intact());

        let at_head = Anchor {
            seq: 4,
            hash: events[3].hash,
        };
        let report = verify_chain(&events, Some(&at_head));
        assert_eq!(report.anchor, AnchorVerdict::Matches);
        assert_eq!(report.unanchored, None);
    }

    // An event from another project in the walk is refused where it
    // appears. Catches a verifier keyed on sequence alone.
    #[test]
    fn an_event_of_another_project_is_a_break() {
        let mut events = chain(2);
        events[1].project_id = ProjectId("other".into());
        let report = verify_chain(&events, None);
        assert_eq!(
            report.first_break,
            Some(Break {
                seq: 2,
                kind: BreakKind::Project {
                    expected: ProjectId(PROJECT.into())
                }
            })
        );
    }

    // An empty chain has no head, no break and nothing unanchored; with an
    // anchor it is truncated to nothing. Catches a range of 1..=0.
    #[test]
    fn an_empty_chain_has_nothing_to_report() {
        let report = verify_chain(&[], None);
        assert_eq!(
            report,
            ChainReport {
                head: None,
                first_break: None,
                anchor: AnchorVerdict::NoAnchor,
                unanchored: None
            }
        );
        let anchor = Anchor {
            seq: 1,
            hash: Hash([0; 32]),
        };
        let report = verify_chain(&[], Some(&anchor));
        assert_eq!(
            report.anchor,
            AnchorVerdict::Truncated {
                anchored: 1,
                head: 0
            }
        );
        assert_eq!(report.unanchored, None);
    }
}
