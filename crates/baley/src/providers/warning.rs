//! The provider risk warning, compiled in (design 0003 section 5).

use crate::store::model::digest;

/// The warning the owner types `yes` to for each outside provider, worded as
/// design 0003 section 5 words it.
///
/// Any change of wording raises [`WARNING_VERSION`]. A changed text also
/// moves [`warning_sha256`], so a stored acknowledgement no longer matches
/// and every owner is asked again (CFG-R28).
pub const WARNING_TEXT: &str = "Baley never reads your API keys. Keys live in your environment, where every program Claude Code starts can see them, agents and subagents included. Nothing scrubs a key a command prints. Your code goes to the chosen provider under its terms.";

/// The version of [`WARNING_TEXT`]; it rises with any change of wording.
pub const WARNING_VERSION: u32 = 1;

/// The lower-case hex SHA-256 of [`WARNING_TEXT`]'s UTF-8 bytes.
pub fn warning_sha256() -> String {
    digest(WARNING_TEXT.as_bytes())
}
