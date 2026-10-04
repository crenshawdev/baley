//! The guard's rules as pure decisions (design 0010). Nothing in this module
//! reads a file, the environment, git, the store or a clock: the hook
//! supplies every observation it judges.

pub mod reason;
mod scan;

#[cfg(test)]
mod tests;

pub use scan::{GitVerb, git_verb};
