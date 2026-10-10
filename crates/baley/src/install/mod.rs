//! `baley install`: Claude Code's wiring at the stable path, from the content
//! `host_artifacts` renders, with ownership evidence in the ledger before
//! any replacement. Gathering is kept apart from judging.

/// Command arguments, host selection and installation gathering.
pub mod command;
/// Installation ownership facts and the `install.recorded` event.
pub mod event;
/// The install outcome and the owner's next steps, rendered from supplied facts.
pub mod receipt;
/// The latest installation ownership record for each host.
pub mod view;
