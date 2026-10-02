//! Checkout admission and the facts it judges (design 0001 Project identity
//! and policy, EVD-R17, ADR 0004). The module gathers a checkout's root
//! commit and remote URL through git and runs checkout admission against the
//! project's `checkout` view.

mod admit;

pub use admit::{ADMIT_COMMAND, AdmitError, admit};
