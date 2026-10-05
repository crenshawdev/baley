//! The judgements a capture's arguments and its named phase pass before
//! anything is recorded (design 0014 section 5, SUP-R1).

use super::event::CaptureKind;

/// The refusal code for a kind that is neither `note` nor `story`.
pub const UNKNOWN_KIND: &str = "unknown-kind";
/// The refusal code for text that is empty or only whitespace.
pub const BLANK_TEXT: &str = "blank-text";
/// The refusal code for a named phase the project does not have.
pub const NO_SUCH_PHASE: &str = "no-such-phase";

/// A kind that is not a capture kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownKind {
    /// `todo` or `seed`, kinds the inherited engine took and captures no
    /// longer do, so a refusal can name the one sent.
    Obsolete(&'static str),
    /// Anything else, which a refusal does not echo.
    Other,
}

/// The capture kind `kind` names. Only the exact lowercase spellings count.
pub fn judge_kind(kind: &str) -> Result<CaptureKind, UnknownKind> {
    match kind {
        "note" => Ok(CaptureKind::Note),
        "story" => Ok(CaptureKind::Story),
        "todo" => Err(UnknownKind::Obsolete("todo")),
        "seed" => Err(UnknownKind::Obsolete("seed")),
        _ => Err(UnknownKind::Other),
    }
}

/// Whether `text` is blank: empty or only whitespace. Any other text is
/// accepted as it is, control characters included, with no length cap
/// below the server's frame.
pub fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// Whether a capture naming `phase` may be recorded, given whether that
/// phase was observed to exist. No phase is always accepted.
///
/// Until Build 4 there is no `phase` view, so the binary always observes a
/// named phase absent and every named phase is refused. Build 4 supplies the
/// observation from its `phase` view here.
pub fn judge_phase(phase: Option<u32>, observed: bool) -> Result<(), &'static str> {
    match phase {
        Some(_) if !observed => Err(NO_SUCH_PHASE),
        _ => Ok(()),
    }
}
