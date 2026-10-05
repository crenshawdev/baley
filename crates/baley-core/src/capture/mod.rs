//! Captures' pure half (design 0014 sections 5 and 6, SUP-R1): the
//! `capture.recorded` event, the capture id, the byte threshold between
//! inline text and a payload, the judgements of a capture's kind, text and
//! named phase, and the `capture` view.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges, and stores any body before the payload names it.

mod event;
mod judge;
mod view;

#[cfg(test)]
mod tests;

pub use event::{
    CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION, CAPTURE_STREAM, CaptureKind, INLINE_TEXT_LIMIT,
    TextForm, capture_id, inline_payload, register_capture_events, stored_payload, text_form,
};
pub use judge::{
    BLANK_TEXT, NO_SUCH_PHASE, UNKNOWN_KIND, UnknownKind, is_blank, judge_kind, judge_phase,
};
pub use view::{CAPTURE_ID_INDEX, CAPTURE_VIEW, CaptureProjector, capture_key, capture_spec};
