//! The tier a listed id gets: an exact row, else the longest matching prefix
//! row, both of the id's own provider, else best fit (design 0003 section 6,
//! CFG-R20).
//! Production passes the compiled `EXACT_HINTS` and `PREFIX_HINTS`; the rows
//! are arguments so a test can supply its own.

use crate::catalog::{HintRow, Placement, PrefixRow, Provider, Tier};

/// The tier, high-effort flag and placement one id gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tag {
    /// The tier.
    pub tier: Tier,
    /// Whether the model accepts high effort.
    pub high_effort: bool,
    /// How it got the tier: `hint`, `prefix` or `best-fit`.
    pub placed: Placement,
}

/// The tag the rows give `id` of `provider`, or `None` when no row of that
/// provider names it or starts it.
pub fn tag(provider: Provider, id: &str, exact: &[HintRow], prefixes: &[PrefixRow]) -> Option<Tag> {
    if let Some(row) = exact
        .iter()
        .find(|row| row.provider == provider && row.id == id)
    {
        return Some(Tag {
            tier: row.tier,
            high_effort: row.high_effort,
            placed: Placement::Hint,
        });
    }
    prefixes
        .iter()
        .filter(|row| row.provider == provider && id.starts_with(row.prefix))
        .max_by_key(|row| row.prefix.len())
        .map(|row| Tag {
            tier: row.tier,
            high_effort: row.high_effort,
            placed: Placement::Prefix,
        })
}

/// An id best fit may take a tier from. Which ids qualify is the diff's
/// call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate<'a> {
    /// The candidate's id.
    pub name: &'a str,
    /// Its tier.
    pub tier: Tier,
    /// Its high-effort flag.
    pub high_effort: bool,
    /// The creation time its provider reported, if any.
    pub created: Option<u64>,
}

/// The placement of an id no row tags. The candidate sharing the longest
/// run of leading `-`-separated segments with `id`, at least one, gives its
/// tier and flag. Ties go to a candidate with a creation time, then the
/// newest, then the name that sorts last. With no such candidate the id is
/// `balanced` without high effort. Either way it is placed `best-fit`.
pub fn best_fit(id: &str, candidates: &[Candidate<'_>]) -> Tag {
    // Whole segments, so `gpt-6` shares only `gpt` with `gpt-60-mini`.
    let run = |name: &str| {
        id.split('-')
            .zip(name.split('-'))
            .take_while(|(ours, theirs)| ours == theirs)
            .count()
    };
    let winner = candidates
        .iter()
        .map(|candidate| (run(candidate.name), candidate))
        .filter(|(run, _)| *run > 0)
        // `None` sorts below any time, so a dated candidate wins a tie.
        .max_by_key(|(run, candidate)| (*run, candidate.created, candidate.name));
    match winner {
        Some((_, candidate)) => Tag {
            tier: candidate.tier,
            high_effort: candidate.high_effort,
            placed: Placement::BestFit,
        },
        None => Tag {
            tier: Tier::Balanced,
            high_effort: false,
            placed: Placement::BestFit,
        },
    }
}
