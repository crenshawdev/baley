//! Whether a current provider acknowledgement covers each outside provider
//! (design 0003 section 6, CFG-R28).
//!
//! Currency is the interview's own rule, `providers::record::covered_by`, so
//! the doctor and the interview never disagree on what still holds. Payloads
//! are taken together: a later acknowledgement of one provider never hides an
//! earlier current one of another.

use baley_core::catalog::Provider;
use serde_json::Value;

use crate::providers::record::covered_by;
use crate::providers::warning::warning_sha256;

/// Where one provider's acknowledgement stands against this binary's warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// A stored payload covers it at this binary's warning version and text.
    Current,
    /// Payloads list it only under another warning version or text.
    NotCurrent {
        /// The highest warning version among them.
        version: u64,
    },
    /// No stored payload lists it.
    NoneRecorded,
}

/// Each provider in catalog order with where its acknowledgement stands. A
/// payload without a numeric `warning_version` or a `providers` list counts
/// for nothing.
pub fn judge(payloads: &[Value]) -> Vec<(Provider, Standing)> {
    let digest = warning_sha256();
    Provider::ALL
        .into_iter()
        .map(|provider| {
            let name = provider.name();
            if payloads
                .iter()
                .any(|payload| covered_by(payload, &digest).contains(&name))
            {
                return (provider, Standing::Current);
            }
            let older = payloads
                .iter()
                .filter(|payload| {
                    payload["providers"]
                        .as_array()
                        .is_some_and(|listed| listed.iter().any(|item| item == name))
                })
                .filter_map(|payload| payload["warning_version"].as_u64())
                .max();
            let standing = match older {
                Some(version) => Standing::NotCurrent { version },
                None => Standing::NoneRecorded,
            };
            (provider, standing)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sha2::{Digest, Sha256};

    use super::*;

    fn hex(text: &str) -> String {
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn payload(providers: &[&str], digest: &str) -> Value {
        json!({
            "providers": providers,
            "warning_version": 1,
            "warning_sha256": digest,
            "owner": "o",
            "acknowledged_at": "2026-10-11T09:00:00Z",
        })
    }

    #[test]
    fn an_acknowledgement_of_changed_text_or_another_provider_counted_current_is_caught() {
        // Design 0003 section 5's warning, word for word.
        let current = hex(
            "Baley never reads your API keys. Keys live in your environment, where every program Claude Code starts can see them, agents and subagents included. Nothing scrubs a key a command prints. Your code goes to the chosen provider under its terms.",
        );
        let reworded = hex("Baley reads nothing.");

        assert_eq!(
            judge(&[payload(&["openai"], &reworded)]),
            [
                (Provider::OpenAi, Standing::NotCurrent { version: 1 }),
                (Provider::DeepSeek, Standing::NoneRecorded),
            ]
        );
        assert_eq!(
            judge(&[payload(&["deepseek"], &current)]),
            [
                (Provider::OpenAi, Standing::NoneRecorded),
                (Provider::DeepSeek, Standing::Current),
            ]
        );
        assert_eq!(
            judge(&[
                payload(&["openai"], &current),
                payload(&["deepseek"], &current)
            ]),
            [
                (Provider::OpenAi, Standing::Current),
                (Provider::DeepSeek, Standing::Current),
            ]
        );
        assert_eq!(
            judge(&[]),
            [
                (Provider::OpenAi, Standing::NoneRecorded),
                (Provider::DeepSeek, Standing::NoneRecorded),
            ]
        );
    }
}
