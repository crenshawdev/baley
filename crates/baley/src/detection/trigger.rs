//! What each provider's step does in one detection run, judged from plain
//! values: the trigger and what `keys.env` held (design 0003 section 6,
//! CFG-R20, CFG-R21).

use baley_core::catalog::Provider;
use baley_core::catalog::detection::Category;
use baley_store::Actor;

use crate::keys::{KEYS_FILE_EXPOSED, KEYS_FILE_INVALID};

/// The command kind `baley models update` records each provider under.
pub const UPDATE_COMMAND: &str = "models.update";
/// The command kind the automatic trigger records each provider under.
pub const DETECT_COMMAND: &str = "models.detect";

/// The name of `provider`'s key in `keys.env`.
pub fn key_name(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAi => "OPENAI_API_KEY",
        Provider::Gemini => "GEMINI_API_KEY",
        Provider::DeepSeek => "DEEPSEEK_API_KEY",
    }
}

/// What started a detection run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// `baley models update` with the providers the owner named, each once.
    /// Empty when none was named.
    Owner(Vec<Provider>),
    /// A run Baley starts itself: `baley init`, and later install and a
    /// model-not-found failure.
    Automatic,
}
impl Trigger {
    /// The command kind each provider's record is made under.
    pub fn kind(&self) -> &'static str {
        match self {
            Trigger::Owner(_) => UPDATE_COMMAND,
            Trigger::Automatic => DETECT_COMMAND,
        }
    }

    /// Who each provider's record is made by.
    pub fn actor(&self) -> Actor {
        match self {
            Trigger::Owner(_) => Actor::Owner,
            Trigger::Automatic => Actor::Baley,
        }
    }

    fn named(&self, provider: Provider) -> bool {
        matches!(self, Trigger::Owner(named) if named.contains(&provider))
    }

    fn covers(&self, provider: Provider) -> bool {
        match self {
            Trigger::Owner(named) if !named.is_empty() => named.contains(&provider),
            _ => true,
        }
    }
}

/// What reading `keys.env` gave, as plain values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeysRead {
    /// The file was refused with this code.
    Refused(&'static str),
    /// The key names of [`key_name`] the file holds.
    Present(Vec<&'static str>),
}

/// The category a refused `keys.env` records: the one named as the
/// refusal's code. `load` gives only the three keys-file codes, and any
/// other code means the keys were not read, which is what unreadable says.
pub fn refusal_category(code: &str) -> Category {
    match code {
        KEYS_FILE_EXPOSED => Category::KeysFileExposed,
        KEYS_FILE_INVALID => Category::KeysFileInvalid,
        _ => Category::KeysFileUnreadable,
    }
}

/// One provider's step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing recorded and nothing printed.
    Skip,
    /// Report that this key is missing, with the provider's detected
    /// entries as unverifiable. Nothing recorded.
    ReportMissing(&'static str),
    /// Record a failure with this category without listing.
    RecordFailure(Category),
    /// List with the provider's key and record what the listing shows.
    List,
}

/// Each provider's step in one run, in [`Provider::ALL`] order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steps(pub Vec<(Provider, Action)>);
impl Steps {
    /// Whether any provider records, which is when the run seeds first.
    pub fn records(&self) -> bool {
        self.0
            .iter()
            .any(|(_, action)| matches!(action, Action::RecordFailure(_) | Action::List))
    }
}

/// Decides each provider's step. A run covers the providers the owner
/// named, or all three. A refused file fails every covered provider with
/// its code. Otherwise a covered provider with its key is listed, and one
/// without is reported only when the owner named it.
pub fn judge(trigger: &Trigger, keys: &KeysRead) -> Steps {
    let action = |provider: Provider| {
        if !trigger.covers(provider) {
            return Action::Skip;
        }
        match keys {
            KeysRead::Refused(code) => Action::RecordFailure(refusal_category(code)),
            KeysRead::Present(names) if names.contains(&key_name(provider)) => Action::List,
            KeysRead::Present(_) if trigger.named(provider) => {
                Action::ReportMissing(key_name(provider))
            }
            KeysRead::Present(_) => Action::Skip,
        }
    };
    Steps(
        Provider::ALL
            .into_iter()
            .map(|provider| (provider, action(provider)))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use baley_core::catalog::Provider::{DeepSeek, Gemini, OpenAi};

    use super::*;
    use crate::keys::{KEYS_FILE_UNREADABLE, KeysRefusal};

    fn present(names: &[&'static str]) -> KeysRead {
        KeysRead::Present(names.to_vec())
    }

    fn steps(actions: [Action; 3]) -> Steps {
        Steps(Provider::ALL.into_iter().zip(actions).collect())
    }

    #[test]
    fn the_automatic_trigger_with_no_keys_lists_reports_and_records_nothing() {
        let steps_taken = judge(&Trigger::Automatic, &present(&[]));
        assert_eq!(
            steps_taken,
            steps([Action::Skip, Action::Skip, Action::Skip])
        );
        assert!(!steps_taken.records());
    }

    #[test]
    fn an_update_naming_none_lists_only_providers_with_a_key_and_skips_the_rest_quietly() {
        let steps_taken = judge(&Trigger::Owner(vec![]), &present(&["OPENAI_API_KEY"]));
        assert_eq!(
            steps_taken,
            steps([Action::List, Action::Skip, Action::Skip])
        );
        assert!(steps_taken.records());
    }

    #[test]
    fn a_named_provider_without_its_key_is_reported_not_recorded_or_skipped() {
        let steps_taken = judge(&Trigger::Owner(vec![OpenAi]), &present(&[]));
        assert_eq!(
            steps_taken,
            steps([
                Action::ReportMissing("OPENAI_API_KEY"),
                Action::Skip,
                Action::Skip,
            ])
        );
        assert!(!steps_taken.records());
    }

    #[test]
    fn a_named_run_reports_the_keyless_lists_the_keyed_and_leaves_the_unnamed() {
        let steps_taken = judge(
            &Trigger::Owner(vec![OpenAi, Gemini]),
            &present(&["GEMINI_API_KEY", "DEEPSEEK_API_KEY"]),
        );
        assert_eq!(
            steps_taken,
            steps([
                Action::ReportMissing("OPENAI_API_KEY"),
                Action::List,
                Action::Skip,
            ])
        );
    }

    #[test]
    fn a_refused_file_fails_all_three_when_none_is_named_and_seeds() {
        let steps_taken = judge(
            &Trigger::Owner(vec![]),
            &KeysRead::Refused(KEYS_FILE_EXPOSED),
        );
        let failed = Action::RecordFailure(Category::KeysFileExposed);
        assert_eq!(steps_taken, steps([failed.clone(), failed.clone(), failed]));
        assert!(steps_taken.records());
    }

    #[test]
    fn a_refused_file_fails_only_the_named_provider() {
        let steps_taken = judge(
            &Trigger::Owner(vec![Gemini]),
            &KeysRead::Refused(KEYS_FILE_EXPOSED),
        );
        assert_eq!(
            steps_taken,
            steps([
                Action::Skip,
                Action::RecordFailure(Category::KeysFileExposed),
                Action::Skip,
            ])
        );
    }

    #[test]
    fn a_refused_file_fails_all_three_on_the_automatic_trigger() {
        let steps_taken = judge(&Trigger::Automatic, &KeysRead::Refused(KEYS_FILE_INVALID));
        let failed = Action::RecordFailure(Category::KeysFileInvalid);
        assert_eq!(steps_taken, steps([failed.clone(), failed.clone(), failed]));
    }

    #[test]
    fn each_refusal_code_records_the_category_of_the_same_name_not_its_text() {
        let path = PathBuf::from("/c/keys.env");
        let refusals = [
            KeysRefusal::Exposed {
                path: path.clone(),
                user: 1000,
                owner: None,
                mode: Some(0o644),
            },
            KeysRefusal::Invalid {
                path: path.clone(),
                faults: vec![],
            },
            KeysRefusal::Unreadable {
                path: path.clone(),
                error: "Permission denied (os error 13)".into(),
            },
            KeysRefusal::NotRegular { path },
        ];
        let expected = [
            (KEYS_FILE_EXPOSED, Category::KeysFileExposed),
            (KEYS_FILE_INVALID, Category::KeysFileInvalid),
            (KEYS_FILE_UNREADABLE, Category::KeysFileUnreadable),
            (KEYS_FILE_UNREADABLE, Category::KeysFileUnreadable),
        ];
        for (refusal, (code, category)) in refusals.iter().zip(expected) {
            assert_eq!(refusal.code(), code);
            assert_eq!(refusal_category(refusal.code()), category);
            assert_eq!(category.name(), code);
        }
    }

    #[test]
    fn the_owner_trigger_records_as_update_by_the_owner_and_the_automatic_one_as_detect_by_baley() {
        assert_eq!(Trigger::Owner(vec![]).kind(), "models.update");
        assert_eq!(Trigger::Owner(vec![OpenAi]).actor(), Actor::Owner);
        assert_eq!(Trigger::Automatic.kind(), "models.detect");
        assert_eq!(Trigger::Automatic.actor(), Actor::Baley);
    }

    #[test]
    fn each_provider_is_listed_under_its_own_key_name() {
        let names = [
            (OpenAi, "OPENAI_API_KEY"),
            (Gemini, "GEMINI_API_KEY"),
            (DeepSeek, "DEEPSEEK_API_KEY"),
        ];
        for (provider, name) in names {
            let steps_taken = judge(&Trigger::Automatic, &present(&[name]));
            for (listed, action) in steps_taken.0 {
                let expected = if listed == provider {
                    Action::List
                } else {
                    Action::Skip
                };
                assert_eq!(action, expected, "{name} for {}", listed.name());
            }
        }
    }
}
