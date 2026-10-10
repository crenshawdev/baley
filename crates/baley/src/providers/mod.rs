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
/// The compiled provider warning, its version and its digest.
pub mod warning;
