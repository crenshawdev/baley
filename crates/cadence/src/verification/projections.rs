//! Binary-owned projections rendered from retained records, never authored.
//!
//! UAT.md for a native phase is the imported original, verbatim, followed by
//! the native human results. Every renderer here is a pure function of the
//! snapshot so commit and recovery derive the same bytes.
use super::human;
use crate::store::Result;
use serde_json::Value;

pub const UAT_HEADING: &str = "## Native human results";

/// None when the phase has neither a retained original nor a native result.
pub fn uat(data: &Value, phase: u32) -> Result<Option<String>> {
    let originals = human::originals(data)?;
    let history = human::records(data)?;
    let original = originals.get(&phase.to_string());
    if original.is_none() && !history.iter().any(|r| r.submission.phase == phase) {
        return Ok(None);
    }
    let mut out = String::new();
    if let Some(original) = original {
        out.push_str(&original.text);
        if !out.ends_with('\n') { out.push('\n'); }
        out.push('\n');
    }
    out.push_str(UAT_HEADING);
    out.push('\n');
    out.push_str("\nRendered by the binary from attributed verification-human-result records.\nReplies are immutable; a later result supersedes an earlier one and the\nfirst-pass outcome is carried, never rewritten. Edit through the operation.\n");
    for (index, item) in human::items(data, phase)?.iter().filter(|i| i["source"] == "native").enumerate() {
        let history = item["history"].as_array().cloned().unwrap_or_default();
        let latest = history.last().cloned().unwrap_or(Value::Null);
        out.push_str(&format!("\n### {}. {}\n", index + 1, item["id"].as_str().unwrap_or_default()));
        if let Some(name) = item["name"].as_str() { out.push_str(&format!("name: {name}\n")); }
        out.push_str(&format!("status: {}\n", item["status"].as_str().unwrap_or_default()));
        out.push_str(&format!("first_pass: {}\n", item["first_pass"].as_str().unwrap_or_default()));
        out.push_str(&format!("reported: {}\n", serde_json::to_string(&latest["reply"])?));
        out.push_str(&format!("owner: {}\n", latest["owner"].as_str().unwrap_or_default()));
        out.push_str(&format!("at: {}\n", latest["at"].as_str().unwrap_or_default()));
        out.push_str(&format!("record: {}\n", latest["id"].as_str().unwrap_or_default()));
        out.push_str(&format!("results: {}\n", history.len()));
    }
    Ok(Some(out))
}
