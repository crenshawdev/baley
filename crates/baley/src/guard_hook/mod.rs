//! The guard hook (design 0010): one host tool call read from stdin, judged
//! with the guard's rules and answered in the host's pre-tool hook form.
//!
//! Gathering and judging are kept apart. A gatherer reads the input, the
//! environment, the filesystem or git and owns no policy, so it has no unit
//! test. A judge takes plain values and is tested directly.

// The hook's entry is its caller once `baley guard` runs through this module.
#[allow(dead_code)]
mod branch;
#[allow(dead_code)]
mod context;
mod render;

pub use render::{Rendered, failed_write, render};
