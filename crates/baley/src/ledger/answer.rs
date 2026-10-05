//! Reading a recorded answer never changes whether it committed.
use baley_store::{Answer, PayloadBody, PayloadStatus, Payloads, StoreError};
use serde_json::Value;
use std::io::Read;

#[derive(Debug, PartialEq, Eq)]
/// Why a committed answer could not be displayed.
pub(crate) enum AnswerUnread {
    Gone(PayloadStatus),
    Malformed,
    Read(StoreError),
}
/// Loads one recorded answer without changing its outcome.
pub(crate) fn answer_value(
    answer: &Answer,
    payloads: &dyn Payloads,
) -> Result<Value, AnswerUnread> {
    let value = match answer {
        Answer::Inline(value) => value.clone(),
        Answer::Tombstone { status, .. } => return Err(AnswerUnread::Gone(status.clone())),
        Answer::Stored(reference) => {
            match payloads.open(&reference.hash).map_err(AnswerUnread::Read)? {
                PayloadBody::Gone(status) => return Err(AnswerUnread::Gone(status)),
                PayloadBody::Present(mut reader) => {
                    let mut bytes = Vec::new();
                    reader
                        .read_to_end(&mut bytes)
                        .map_err(|e| AnswerUnread::Read(StoreError::Unavailable(e.to_string())))?;
                    serde_json::from_slice(&bytes).map_err(|_| AnswerUnread::Malformed)?
                }
            }
        }
    };
    if value.is_object() {
        Ok(value)
    } else {
        Err(AnswerUnread::Malformed)
    }
}
