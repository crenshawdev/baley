//! The policy step (design 0003, CFG-R8, CFG-R9; build-2-plan decision 18).
//! Before every command that appends to a project's chain from a checkout,
//! it re-reads both settings files and records `policy.effective` when the
//! policy changed. It prints nothing.

mod read;
mod record;

pub use read::{Reads, build, gather};
pub use record::{RECORD_COMMAND, record};
