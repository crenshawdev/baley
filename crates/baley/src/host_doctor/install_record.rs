//! What the latest install record says about the install itself: whether it
//! finished, whether the sandbox block was held back and whether the guard
//! hook was placed. The record is a plain JSON value and nothing is read here.

use serde_json::Value;

/// The install record judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// Whether the install finished. Only a boolean `true` `complete` does,
    /// so a missing or mistyped field never reads as a finished install.
    pub finished: bool,
    /// Why the sandbox block was held back, when it was.
    pub held_back: Option<HeldBack>,
    /// Whether the record holds the guard hook install placed.
    pub hook_placed: bool,
}

/// A sandbox block install left out on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldBack {
    /// The reason the record gives.
    pub reason: String,
    /// The settings file the record names, when it names one.
    pub settings_path: Option<String>,
}

/// Judges a record payload.
pub fn judge(record: &Value) -> Judged {
    let sandbox = &record["sandbox"];
    Judged {
        finished: record["complete"] == Value::Bool(true),
        held_back: sandbox["held_back"].as_str().map(|reason| HeldBack {
            reason: reason.to_owned(),
            settings_path: sandbox["settings_path"].as_str().map(str::to_owned),
        }),
        hook_placed: !record["registered"]["hook"].is_null(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn an_install_record_without_a_true_complete_read_as_finished_is_caught() {
        let finished = |complete: Option<Value>| {
            let mut record = json!({"registered": {"hook": null}, "sandbox": null});
            if let Some(complete) = complete {
                record["complete"] = complete;
            }
            judge(&record).finished
        };
        assert!(finished(Some(json!(true))));
        for complete in [
            Some(json!(false)),
            None,
            Some(json!("true")),
            Some(json!(1)),
        ] {
            assert!(!finished(complete.clone()), "{complete:?}");
        }
    }

    #[test]
    fn a_held_back_reason_or_an_unplaced_hook_lost_from_the_judgement_is_caught() {
        let with_sandbox = |sandbox: Value| {
            judge(&json!({"complete": true, "registered": {"hook": null}, "sandbox": sandbox}))
        };
        let held = with_sandbox(json!({
            "settings_path": "/home/o/.claude/settings.json",
            "held_back": "socat missing from PATH",
        }));
        assert_eq!(
            held.held_back.map(|held| held.reason),
            Some("socat missing from PATH".to_owned())
        );
        let written = with_sandbox(json!({
            "settings_path": "/home/o/.claude/settings.json",
            "held_back": null,
        }));
        assert_eq!(written.held_back, None);
        assert_eq!(with_sandbox(Value::Null).held_back, None);

        let with_hook = |hook: Value| {
            judge(&json!({"complete": true, "registered": {"hook": hook}, "sandbox": null}))
                .hook_placed
        };
        assert!(with_hook(json!({
            "path": "/home/o/.claude/settings.json",
            "item": {"matcher": "Bash", "hooks": []},
        })));
        assert!(!with_hook(Value::Null));
    }
}
