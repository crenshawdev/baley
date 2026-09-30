//! Detection's pure decisions (design 0003 section 6, CFG-R20, CFG-R21):
//! reading each provider's list pages into its ids, deciding which page
//! comes next, and judging what a lister saw as a listing or a failure
//! category.
//!
//! Nothing here takes a key or reaches a file, a clock, the network or the
//! store. The binary gathers every status, body and flag, and this module
//! judges them.

mod classify;
mod paging;
mod parse;

pub use classify::{Category, Observation, ObservedResponse, classify};
pub use paging::{PAGE_BOUND, Paging, next_page, paging};
pub use parse::{ProviderListing, parse_page};

#[cfg(test)]
mod tests;
