//! The binary renders the owner's report from derived rows; no role authors it.
use serde_json::Value;

fn evidence(row: &Value) -> String {
    let items: Vec<_> = row["items"].as_array().map(|items| items.iter()
        .map(|i| format!("{} {}", i["id"].as_str().unwrap_or_default(), i["verdict"].as_str().unwrap_or_default())).collect())
        .unwrap_or_default();
    if items.is_empty() { "-".into() } else { items.join("; ") }
}

fn statuses(rows: &Value) -> String {
    rows.as_array().map(|rows| rows.iter()
        .map(|r| format!("; {} {}", r["id"].as_str().unwrap_or_default(), r["status"].as_str().unwrap_or_default())).collect())
        .unwrap_or_default()
}

pub fn text(report: &Value) -> String {
    let mut out = format!("# Verification report: phase {}\n", report["phase"]);
    let current = &report["current"];
    if current["applicable"] == true {
        out.push_str(&format!("Current: attempt {}, patch {}, HEAD {}, map {}\n",
            current["attempt"].as_str().unwrap_or_default(), current["patch"].as_str().unwrap_or_default(),
            current["verified_at"]["source"]["head"].as_str().unwrap_or_default(),
            current["verified_at"]["map_digest"].as_str().unwrap_or_default()));
    } else {
        out.push_str(&format!("Current: none - {}\n", current["reason"].as_str().unwrap_or_default()));
    }
    out.push_str("| truth | status | evidence |\n");
    for row in report["truths"].as_array().into_iter().flatten() {
        out.push_str(&format!("| {} | {} | {} |\n", row["id"].as_str().unwrap_or_default(),
            row["status"].as_str().unwrap_or_default(), evidence(row)));
    }
    out.push_str("History:\n");
    for entry in report["history"].as_array().into_iter().flatten() {
        out.push_str(&format!("- attempt {}: {} - {}{}\n", entry["attempt"].as_str().unwrap_or_default(),
            entry["applicability"].as_str().unwrap_or_default(), entry["reason"].as_str().unwrap_or_default(),
            statuses(&entry["truths"])));
    }
    let present = |document: &str| if report["legacy"][document] == true { "present" } else { "absent" };
    out.push_str(&format!("Legacy: SUMMARY {}, UAT {} - historical classification only, never native evidence.\n",
        present("summary_document"), present("uat_document")));
    out
}
