//! Outside review providers: the compiled risk warning, the owner's recorded
//! acknowledgement of it and the owner identity it names (design 0003
//! section 5, CFG-R28).
//!
//! Baley never reads, stores or sends a provider key (CFG-R24). Nothing here
//! touches one, and the acknowledgement records no credential or environment
//! value.

/// The `providers.acknowledged` event and its payload.
pub mod event;
/// The owner identity from git.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the interview reads the owner identity and is wired in next"
    )
)]
pub(crate) mod identity;
/// Recording the acknowledgement unless a current one covers it.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the interview records the acknowledgement and is wired in next"
    )
)]
pub(crate) mod record;
/// The compiled provider warning, its version and its digest.
pub mod warning;

/// Shared inputs for the provider tests: design 0003 section 5's warning
/// paragraph written out, so no expected digest comes from the constant under
/// test, and a SHA-256 helper that does not use the code under test.
#[cfg(test)]
pub(crate) mod fixtures {
    use sha2::{Digest, Sha256};

    /// Design 0003 section 5's warning paragraph.
    pub(crate) const DESIGN_WARNING: &str = "Baley never reads your API keys. Keys live in your environment, where every program Claude Code starts can see them, agents and subagents included. Nothing scrubs a key a command prints. Your code goes to the chosen provider under its terms.";

    /// The lower-case hex SHA-256 of `text`.
    pub(crate) fn sha256_hex(text: &str) -> String {
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
