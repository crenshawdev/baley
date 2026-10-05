//! The one place a long read answer is cut into numbered parts. `schema`, and
//! every other read that can outgrow a tool result, cuts through `cut` so the
//! bound and the numbering are the same everywhere.

/// The most body bytes in one part, cut at a UTF-8 character boundary. The
/// bound counts the body only, not the encoded tool result around it.
pub const PART_BOUND: usize = 24_576;

/// What `cut` decided for one request.
#[derive(Debug, PartialEq, Eq)]
pub enum Cut<'a> {
    /// The text fits in one part and part 1 (or no part) was asked for.
    Whole(&'a str),
    /// One numbered part, with the next part's number unless this is the last.
    Part {
        /// The part's text, at most `PART_BOUND` bytes.
        body: &'a str,
        /// The one-based number of this part.
        part: usize,
        /// The following part's number, or none on the last part.
        next: Option<usize>,
    },
    /// The requested part does not exist: part 0, or one past the last.
    Absent,
}

/// Cuts `text` for the one-based `part` (part 1 when none is given). Joining
/// every part in order gives back `text` exactly.
pub fn cut(text: &str, part: Option<usize>) -> Cut<'_> {
    let part = part.unwrap_or(1);
    if text.len() <= PART_BOUND {
        return if part == 1 {
            Cut::Whole(text)
        } else {
            Cut::Absent
        };
    }
    let mut remaining = text;
    let mut parts = Vec::new();
    while !remaining.is_empty() {
        let mut end = remaining.len().min(PART_BOUND);
        while !remaining.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(&remaining[..end]);
        remaining = &remaining[end..];
    }
    // `checked_sub` keeps part 0 from wrapping around to a huge index.
    let Some(body) = part.checked_sub(1).and_then(|index| parts.get(index)) else {
        return Cut::Absent;
    };
    Cut::Part {
        body,
        part,
        next: (part < parts.len()).then_some(part + 1),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn parts_round_trip_utf8_and_escaped_json() {
        let schema = json!({"description":"é🦀\"\\".repeat(PART_BOUND)});
        let text = serde_json::to_string(&schema).unwrap();
        let mut combined = String::new();
        let mut part = None;
        let mut number = 1;
        loop {
            let Cut::Part {
                body,
                part: got,
                next,
            } = cut(&text, part)
            else {
                panic!("part {number} must be a part");
            };
            assert_eq!(got, number);
            assert!(!body.is_empty() && body.len() <= PART_BOUND);
            combined.push_str(body);
            let Some(next) = next else { break };
            number += 1;
            assert_eq!(next, number);
            part = Some(number);
        }
        assert!(number > 1);
        assert_eq!(combined, text);
        assert_eq!(serde_json::from_str::<Value>(&combined).unwrap(), schema);
    }

    #[test]
    fn text_at_the_bound_is_served_whole_and_one_byte_over_is_paged() {
        let at_bound = "x".repeat(PART_BOUND);
        assert_eq!(cut(&at_bound, None), Cut::Whole(&at_bound));
        assert_eq!(cut(&at_bound, Some(1)), Cut::Whole(&at_bound));
        let oversized = "x".repeat(PART_BOUND + 1);
        let Cut::Part { body, part, next } = cut(&oversized, None) else {
            panic!("one byte over the bound must be paged");
        };
        assert_eq!((body.len(), part, next), (PART_BOUND, 1, Some(2)));
        let Cut::Part { body, part, next } = cut(&oversized, Some(2)) else {
            panic!("the second part must exist");
        };
        assert_eq!((body.len(), part, next), (1, 2, None));
    }

    #[test]
    fn part_0_past_the_end_or_usize_max_is_absent() {
        let whole = "x".repeat(PART_BOUND);
        let paged = "x".repeat(PART_BOUND + 1);
        for part in [0, 2, usize::MAX] {
            assert_eq!(cut(&whole, Some(part)), Cut::Absent, "whole, part {part}");
        }
        for part in [0, 3, usize::MAX] {
            assert_eq!(cut(&paged, Some(part)), Cut::Absent, "paged, part {part}");
        }
    }
}
