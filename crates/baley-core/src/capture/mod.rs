//! Captures' pure half (design 0014 sections 5 and 6, SUP-R1): the
//! `capture.recorded` event, the capture id and the byte threshold between
//! inline text and a payload.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges, and stores any body before the payload names it.

mod event;

#[cfg(test)]
mod tests;

pub use event::{
    CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION, CAPTURE_STREAM, CaptureKind, INLINE_TEXT_LIMIT,
    TextForm, capture_id, inline_payload, register_capture_events, stored_payload, text_form,
};
