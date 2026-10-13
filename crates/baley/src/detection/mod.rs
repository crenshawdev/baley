//! Detection: records a provider model list the owner or a session fetched
//! and piped into `baley models import` (design 0003 section 6, CFG-R20,
//! CFG-R21). Baley fetches none itself and reads no key: it prints the
//! fetch command, reads what comes back and records the outcome. The
//! judging lives in `baley_core::catalog::detection`.

mod fetch;
mod record;
mod trailer;
mod trigger;

pub use fetch::fetch_command;
pub use record::{Recording, detection_request, record};
pub use trailer::parse_fetched;
pub use trigger::{IMPORT_COMMAND, key_name, list_address};

/// The most body bytes one model list may hold: the fetch command's
/// `--max-filesize` and the bound `import` checks.
pub(crate) const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// How many seconds one fetch may take, body included.
pub(crate) const DEADLINE_SECONDS: u64 = 20;
