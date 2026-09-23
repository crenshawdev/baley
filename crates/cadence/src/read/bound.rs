use super::slice::ANSWER_BOUND;
use crate::envelope::Refusal;
use serde_json::Value;
use std::num::NonZeroU32;

pub(super) const DEFAULT_LIMIT: usize = 50;
pub(super) const MAX_LIMIT: usize = 200;
pub(super) const LINE_TEXT_CHARS: usize = 200;
pub(super) const LINE_CUT_MARKER: &str = "…";

pub(super) struct Page {
    pub served: Vec<usize>,
    pub passed: Vec<usize>,
    pub next: Option<usize>,
}

/// Charge the empty answer with every optional field and widest token present.
pub(super) fn room(envelope: &Value) -> usize {
    ANSWER_BOUND.saturating_sub(serde_json::to_vec(envelope).unwrap().len())
}

pub(super) fn longest_token(prefix: &str) -> String {
    format!("{prefix}-0000000000000000-{}", u64::MAX)
}

pub(super) fn fit(_room: usize, rows: &[Value]) -> usize {
    rows.len()
}

/// A row too large for an empty page is skipped so continuation makes progress.
pub(super) fn page(rows: &[Value], room: usize, limit: usize) -> Page {
    let mut page = Page { served: Vec::new(), passed: Vec::new(), next: None };
    let mut remaining = room;
    for (index, row) in rows.iter().enumerate() {
        if page.served.len() == limit {
            page.next = Some(index);
            break;
        }
        if fit(room, std::slice::from_ref(row)) == 0 {
            page.passed.push(index);
        } else if fit(remaining, std::slice::from_ref(row)) == 0 {
            page.next = Some(index);
            break;
        } else {
            page.served.push(index);
            remaining = remaining.saturating_sub(serde_json::to_vec(row).unwrap().len() + 1);
        }
    }
    page
}

pub(super) fn limit(requested: Option<NonZeroU32>) -> Result<usize, Value> {
    let requested = requested.map_or(DEFAULT_LIMIT, |value| value.get() as usize);
    if requested > MAX_LIMIT {
        return Err(Refusal::new("invalid-limit", format!("limit must be at most {MAX_LIMIT}"))
            .slot("limit").value());
    }
    Ok(requested)
}

pub(super) fn line_text(line: &str) -> String {
    match line.char_indices().nth(LINE_TEXT_CHARS) {
        Some((end, _)) => format!("{}{LINE_CUT_MARKER}", &line[..end]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests;
