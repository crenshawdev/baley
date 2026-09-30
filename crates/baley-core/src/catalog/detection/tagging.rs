//! The tier a listed id gets: an exact row, else the longest matching prefix
//! row, both of the id's own provider (design 0003 section 6, CFG-R20).
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
