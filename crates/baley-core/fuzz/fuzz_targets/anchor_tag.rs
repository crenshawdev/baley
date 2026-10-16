//! An anchor tag read back from the forge: its name and its message.
#![no_main]

use baley_core::forge::{parse_annotation, parse_tag_anchor, tag_sequence};
use baley_store::ProjectId;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // Three fields, one per line: project id, tag name, then the tag message.
    let mut fields = text.splitn(3, '\n');
    let project = ProjectId(fields.next().unwrap_or_default().to_owned());
    let tag = fields.next().unwrap_or_default();
    let annotation = fields.next().unwrap_or_default();
    let _ = parse_annotation(annotation);
    let _ = tag_sequence(&project, tag);
    let _ = parse_tag_anchor(&project, tag, annotation);
});
