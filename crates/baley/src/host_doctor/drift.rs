//! Whether a placed hook or settings document still holds Baley's own
//! entries: those `baley install` renders over the derived placements and the
//! whole entries the latest install record holds. Each entry is compared
//! whole, so order and the owner's other entries never count, and nothing is
//! compared by digest, so hosts the interview adds are never drift.

use serde_json::Value;

use crate::host_artifacts::hook;
use crate::install::settings::Ours;

/// One of Baley's entries a document lacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    /// The dotted key of the list that should hold it.
    pub list: &'static str,
    /// The entry, as its string or its compact JSON.
    pub entry: String,
}

/// The `PreToolUse` items `document` lacks: this binary's and the one the
/// record holds.
pub fn hook(document: &Value, ours: &Ours, record: Option<&Value>) -> Vec<Missing> {
    let mut expected = vec![&ours.hook_item];
    if let Some(item) = record.and_then(|record| record.pointer("/registered/hook/item"))
        && !expected.contains(&item)
    {
        expected.push(item);
    }
    let items = document
        .pointer(&format!("/hooks/{}", hook::EVENT))
        .and_then(Value::as_array);
    expected
        .into_iter()
        .filter(|item| items.is_none_or(|items| !items.contains(item)))
        .map(|item| Missing {
            list: "hooks.PreToolUse",
            entry: item.to_string(),
        })
        .collect()
}

/// The `permissions.deny`, `sandbox.filesystem.denyRead` and
/// `sandbox.filesystem.denyWrite` strings `document` lacks: this binary's
/// and the record's. While the record says the sandbox block was held back,
/// this binary's `denyRead` and `denyWrite` entries are not expected, since
/// install left them out on purpose.
pub fn settings(document: &Value, ours: &Ours, record: Option<&Value>) -> Vec<Missing> {
    let held_back = record.is_some_and(|record| record["sandbox"]["held_back"].is_string());
    let lists: [(&'static str, &str, &[String], &str, bool); 3] = [
        (
            "permissions.deny",
            "/permissions/deny",
            &ours.permissions_deny,
            "/sandbox/permissions_deny",
            true,
        ),
        (
            "sandbox.filesystem.denyRead",
            "/sandbox/filesystem/denyRead",
            &ours.deny_read,
            "/sandbox/deny_read",
            !held_back,
        ),
        (
            "sandbox.filesystem.denyWrite",
            "/sandbox/filesystem/denyWrite",
            &ours.deny_write,
            "/sandbox/deny_write",
            !held_back,
        ),
    ];
    let mut missing = Vec::new();
    for (list, pointer, rendered, recorded, expect_rendered) in lists {
        let mut expected: Vec<&str> = Vec::new();
        if expect_rendered {
            expected.extend(rendered.iter().map(String::as_str));
        }
        let recorded = record
            .and_then(|record| record.pointer(recorded))
            .and_then(Value::as_array);
        expected.extend(recorded.into_iter().flatten().filter_map(Value::as_str));
        let held = document.pointer(pointer).and_then(Value::as_array);
        let mut seen: Vec<&str> = Vec::new();
        for entry in expected {
            if seen.contains(&entry) {
                continue;
            }
            seen.push(entry);
            if held.is_none_or(|held| !held.iter().any(|item| item.as_str() == Some(entry))) {
                missing.push(Missing {
                    list,
                    entry: entry.to_owned(),
                });
            }
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::install::fixtures::{complete_settings, folders, hook_item, installed};
    use crate::install::settings::ours;

    fn rendered() -> Ours {
        ours(&installed().placements, &folders())
    }

    fn missing(list: &'static str, entry: &str) -> Missing {
        Missing {
            list,
            entry: entry.to_owned(),
        }
    }

    #[test]
    fn a_settings_entry_matched_by_order_or_prefix_is_caught() {
        let mut reversed = complete_settings();
        for pointer in ["/permissions/deny", "/sandbox/filesystem/denyWrite"] {
            reversed
                .pointer_mut(pointer)
                .and_then(Value::as_array_mut)
                .unwrap()
                .reverse();
        }
        assert_eq!(settings(&reversed, &rendered(), None), []);

        let mut longer = complete_settings();
        for rule in longer["permissions"]["deny"].as_array_mut().unwrap() {
            if rule == "Edit(//home/o/.local/bin/baley)" {
                *rule = json!("Edit(//home/o/.local/bin/baley2)");
            }
        }
        assert_eq!(
            settings(&longer, &rendered(), None),
            [missing(
                "permissions.deny",
                "Edit(//home/o/.local/bin/baley)"
            )]
        );
    }

    #[test]
    fn a_held_back_sandbox_read_as_drift_or_its_deny_rules_excused_is_caught() {
        let mut document = complete_settings();
        document.as_object_mut().unwrap().remove("sandbox");
        document["permissions"]["deny"]
            .as_array_mut()
            .unwrap()
            .retain(|rule| rule != "Read(//home/o/.config/crenshawdev/baley/**)");
        let record = json!({"sandbox": {
            "settings_path": "/home/o/.claude/settings.json",
            "held_back": "socat missing from PATH",
            "permissions_deny": [],
            "deny_read": [],
            "deny_write": [],
        }});
        assert_eq!(
            settings(&document, &rendered(), Some(&record)),
            [missing(
                "permissions.deny",
                "Read(//home/o/.config/crenshawdev/baley/**)"
            )]
        );
    }

    #[test]
    fn an_entry_the_record_holds_left_out_of_the_expectation_is_caught() {
        let document = complete_settings();
        let mut older = hook_item();
        older["hooks"][0]["command"] = json!("'/old/baley' guard");
        let mut permissions_deny = document["permissions"]["deny"].clone();
        permissions_deny
            .as_array_mut()
            .unwrap()
            .push(json!("Read(//old/baley/**)"));
        let mut deny_read = document["sandbox"]["filesystem"]["denyRead"].clone();
        deny_read
            .as_array_mut()
            .unwrap()
            .push(json!("/old/baley/home"));
        let mut deny_write = document["sandbox"]["filesystem"]["denyWrite"].clone();
        deny_write
            .as_array_mut()
            .unwrap()
            .push(json!("/old/baley/versions"));
        let record = json!({
            "registered": {"hook": {"path": "/home/o/.claude/settings.json", "item": older}},
            "sandbox": {
                "settings_path": "/home/o/.claude/settings.json",
                "held_back": null,
                "permissions_deny": permissions_deny,
                "deny_read": deny_read,
                "deny_write": deny_write,
            },
        });
        assert_eq!(
            settings(&document, &rendered(), Some(&record)),
            [
                missing("permissions.deny", "Read(//old/baley/**)"),
                missing("sandbox.filesystem.denyRead", "/old/baley/home"),
                missing("sandbox.filesystem.denyWrite", "/old/baley/versions"),
            ]
        );
        assert_eq!(
            hook(&document, &rendered(), Some(&record)),
            [missing("hooks.PreToolUse", &older.to_string())]
        );
    }
}
