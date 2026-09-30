//! Detection's pure decisions (design 0003 section 6, CFG-R20, CFG-R21):
//! reading each provider's list pages into its ids, deciding which page
//! comes next, judging what a lister saw as a listing or a failure category,
//! tagging each listed id, comparing a listing with the catalog, and
//! choosing the event a provider's detection records.
//!
//! Nothing here takes a key or reaches a file, a clock, the network or the
//! store. The binary gathers every status, body and flag, and this module
//! judges them.

mod classify;
mod diff;
mod event;
mod paging;
mod parse;
mod tagging;

pub use classify::{Category, Observation, ObservedResponse, classify};
pub use diff::{Added, Diff, IdChange, Report, ReportRow, diff};
pub use event::{ChosenEvent, Outcome, choose_event};
pub use paging::{PAGE_BOUND, Paging, next_page, paging};
pub use parse::{ProviderListing, parse_page};
pub use tagging::{Candidate, Tag, best_fit, tag};

#[cfg(test)]
mod tests;
