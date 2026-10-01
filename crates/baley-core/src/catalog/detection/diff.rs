//! One provider's listing compared with its catalog document as read: the
//! event's `added` and `removed`, and the owner's report (design 0003
//! section 6, CFG-R20).

use std::collections::BTreeMap;

use serde_json::Value;

use super::parse::ProviderListing;
use super::tagging::{Candidate, Tag, best_fit, tag};
use crate::catalog::view::read_entries;
use crate::catalog::{HintRow, Placement, PrefixRow, Provider, Source, Tier};

/// One id `models.detected` names in `added`, with the tag this run gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// The id.
    pub id: String,
    /// Its tier, flag and placement.
    pub tag: Tag,
}

/// What a detection does to one id, as the owner's report shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdChange {
    /// Listed, and the catalog did not accept it before.
    New,
    /// Listed, and the catalog already accepted it.
    Unchanged,
    /// Accepted from seed or detection, and no longer listed.
    Removed,
}

/// One id in the owner's report, with the tier and placement it holds once
/// the event is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    /// The id.
    pub id: String,
    /// What the detection does to it.
    pub change: IdChange,
    /// Its tier; absent only for an owner entry that never had one.
    pub tier: Option<Tier>,
    /// How it got its tier.
    pub placed: Placement,
}

/// The owner's report of one provider's detection, in id order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Every listed id and every removed one.
    pub rows: Vec<ReportRow>,
}
impl Report {
    /// How many ids had `change`, counted against the document before the
    /// event.
    pub fn count(&self, change: IdChange) -> usize {
        self.rows.iter().filter(|row| row.change == change).count()
    }
}

/// The lists `models.detected` records and the report that goes with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    /// Every listed id the owner has not removed, in id order.
    pub added: Vec<Added>,
    /// The accepted seed and detected ids the listing lacks, in id order.
    pub removed: Vec<String>,
    /// The owner's report.
    pub report: Report,
}

/// Compares `listing` with `provider`'s catalog document. A document that
/// does not read is taken as absent; the projector refuses to apply an
/// event over one, so nothing is recorded from it.
///
/// Every listed id is in `added`, accepted ones too, so a seeded id becomes
/// detected, an owner entry's high-effort flag fills in, and a best-fit
/// placement is redone with the current rows on every run. Only an id the
/// owner removed is left out.
pub fn diff(
    provider: Provider,
    listing: &ProviderListing,
    exact: &[HintRow],
    prefixes: &[PrefixRow],
    document: Option<&Value>,
) -> Diff {
    let entries = read_entries(document).unwrap_or_default();
    let listed: BTreeMap<&str, Option<u64>> = listing
        .models()
        .filter(|(id, _)| !entries.get(*id).is_some_and(|entry| entry.owner_removed))
        .collect();
    let tags: BTreeMap<&str, Option<Tag>> = listed
        .keys()
        .map(|id| (*id, tag(provider, id, exact, prefixes)))
        .collect();

    // Tagged ids of this listing, then the document's accepted entries
    // placed by a row or the owner. Best fit never feeds best fit.
    let mut candidates: BTreeMap<&str, Candidate<'_>> = BTreeMap::new();
    for (id, tagged) in &tags {
        if let Some(tagged) = tagged {
            candidates.insert(
                id,
                Candidate {
                    name: id,
                    tier: tagged.tier,
                    high_effort: tagged.high_effort,
                    created: listed[id],
                },
            );
        }
    }
    for entry in entries.values().filter(|entry| !entry.owner_removed) {
        let Some(tier) = entry.tier else {
            continue;
        };
        if entry.placed == Placement::BestFit {
            continue;
        }
        // The catalog keeps the owner's tier over the listing's tag.
        if entry.source != Source::Owner && candidates.contains_key(entry.id.as_str()) {
            continue;
        }
        candidates.insert(
            &entry.id,
            Candidate {
                name: &entry.id,
                tier,
                high_effort: entry.high_effort,
                created: listed.get(entry.id.as_str()).copied().flatten(),
            },
        );
    }

    let mut added = Vec::new();
    let mut rows = Vec::new();
    for (id, tagged) in &tags {
        let tag = tagged.unwrap_or_else(|| {
            // An id never places itself: that would keep a placement the
            // current rows no longer give.
            let others: Vec<Candidate<'_>> = candidates
                .values()
                .filter(|candidate| candidate.name != *id)
                .copied()
                .collect();
            best_fit(id, &others)
        });
        let held = entries.get(*id).filter(|entry| !entry.owner_removed);
        let (tier, placed) = match held {
            Some(entry) if entry.source == Source::Owner => (entry.tier, entry.placed),
            _ => (Some(tag.tier), tag.placed),
        };
        rows.push(ReportRow {
            id: (*id).to_owned(),
            change: if held.is_some() {
                IdChange::Unchanged
            } else {
                IdChange::New
            },
            tier,
            placed,
        });
        added.push(Added {
            id: (*id).to_owned(),
            tag,
        });
    }

    let mut removed = Vec::new();
    for entry in entries.values() {
        let lapsed = matches!(entry.source, Source::Seed | Source::Detected)
            && !entry.owner_removed
            && !listing.contains(&entry.id);
        if lapsed {
            removed.push(entry.id.clone());
            rows.push(ReportRow {
                id: entry.id.clone(),
                change: IdChange::Removed,
                tier: entry.tier,
                placed: entry.placed,
            });
        }
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Diff {
        added,
        removed,
        report: Report { rows },
    }
}
