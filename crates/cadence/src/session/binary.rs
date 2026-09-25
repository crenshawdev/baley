//! The session layer: first-touch store initialization, the config layers and
//! guarded snapshot writes.

// This executable owns these tests; shared source includes register none.
include!("mod.rs");

#[cfg(test)]
mod tests;

#[cfg(test)]
mod routing_admission_tests;
