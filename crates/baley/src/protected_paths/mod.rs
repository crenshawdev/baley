//! Which paths the guard protects, and the decisions that judge a tool call's
//! paths against them (design 0010, GRD-R11). The filesystem is supplied
//! through [`Lookup`], so every decision here runs over values and a table
//! in tests.

mod contain;
mod resolve;

#[cfg(test)]
mod tests;

pub use contain::{contains, is_inside};
pub use resolve::{
    Disk, Entry, Lookup, Part, ResolveFailure, canonical_cwd, resolve_existing_prefix,
    resolve_target, resolve_under,
};
