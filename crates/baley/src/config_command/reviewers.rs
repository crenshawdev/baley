//! The outside providers the owner has stored: the global file's top-level
//! `review.reviewers` (design 0003 section 5, Provider selection). Pure over
//! the parsed layer; the interview shows the result before it asks.

use std::collections::BTreeSet;

use baley_core::catalog::Provider;
use baley_core::policy::config_command::TypedPair;
use baley_core::policy::{ParsedLayer, Value};

/// The setting the stored selection is read from.
const SETTING: &str = "review.reviewers";

/// The host's own reviewer, which the list keeps as it finds it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the list decision reads it")
)]
const HOST: &str = "host";

/// The outside providers in force and the layer they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Selection {
    /// The outside providers, each once, in catalog order.
    pub(super) providers: Vec<Provider>,
    /// `global` when the global file writes the setting, else `default`.
    pub(super) layer: &'static str,
}

impl Selection {
    /// No outside provider, from the built-in default.
    pub(super) fn none() -> Selection {
        Selection {
            providers: Vec::new(),
            layer: "default",
        }
    }

    /// The providers as the owner reads them: names joined with commas, or `none`.
    pub(super) fn text(&self) -> String {
        if self.providers.is_empty() {
            return "none".to_owned();
        }
        let names: Vec<&str> = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .collect();
        names.join(", ")
    }
}

/// The names the global file's top-level `review.reviewers` lists, as written,
/// or `None` when the file writes no such value. A host section's value
/// belongs to that host and does not count.
pub(super) fn written(global: Option<&ParsedLayer>) -> Option<Vec<String>> {
    let written = global?
        .values
        .iter()
        .find(|written| written.host.is_none() && written.name == SETTING)?;
    match &written.value {
        Value::ReviewerList(names) => Some(names.clone()),
        _ => None,
    }
}

/// The selection the global file stores. `host` is the host's own reviewer, not
/// an outside provider. With no value written the selection is none from
/// `default`.
pub(super) fn stored(global: Option<&ParsedLayer>) -> Selection {
    let Some(names) = written(global) else {
        return Selection::none();
    };
    Selection {
        providers: Provider::ALL
            .into_iter()
            .filter(|provider| names.iter().any(|name| name == provider.name()))
            .collect(),
        layer: "global",
    }
}

/// The list to write so the global file names the chosen outside providers, or
/// `None` when it needs no write. `stored` is the global file's top-level
/// `review.reviewers` as written, `None` for no value, which means the default
/// `["host"]`. `chosen` is the owner's explicit choice, `None` for a blank
/// answer, which writes nothing.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the interview's write plan calls it")
)]
pub(super) fn decide(stored: Option<&[String]>, chosen: Option<&[Provider]>) -> Option<TypedPair> {
    let chosen = chosen?;
    let default = [HOST.to_owned()];
    let held = stored.unwrap_or(&default);
    let mut list = Vec::new();
    if held.iter().any(|name| name == HOST) {
        list.push(HOST.to_owned());
    }
    list.extend(
        Provider::ALL
            .into_iter()
            .filter(|provider| chosen.contains(provider))
            .map(|provider| provider.name().to_owned()),
    );
    let same = held.iter().collect::<BTreeSet<_>>() == list.iter().collect::<BTreeSet<_>>();
    (!same).then(|| TypedPair {
        name: SETTING.to_owned(),
        host: None,
        value: Value::ReviewerList(list),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use baley_core::policy::{FileLayer, Schema, parse_layer};

    use super::*;
    use crate::settings;

    fn selection_of(text: Option<&str>) -> Selection {
        let layer = text.map(|text| {
            let file = settings::file(Path::new("/c/config.toml"), text.as_bytes().to_vec());
            parse_layer(&file, FileLayer::Global, Schema::standard()).unwrap()
        });
        stored(layer.as_ref())
    }

    #[test]
    fn a_stored_selection_read_from_a_host_section_or_with_the_host_reviewer_is_caught() {
        assert_eq!(
            selection_of(Some("[review]\nreviewers = [\"host\", \"deepseek\"]\n")),
            Selection {
                providers: vec![Provider::DeepSeek],
                layer: "global"
            }
        );
        assert_eq!(
            selection_of(Some(
                "[host.claude-code.review]\nreviewers = [\"openai\"]\n"
            )),
            Selection::none()
        );
        assert_eq!(
            selection_of(Some("[review]\nreviewers = [\"host\"]\n")),
            Selection {
                providers: Vec::new(),
                layer: "global"
            }
        );
        assert_eq!(selection_of(None), Selection::none());
    }
    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The list `decide` writes, as names.
    fn written_by(stored: Option<&[&str]>, chosen: &[Provider]) -> Option<Vec<String>> {
        let stored = stored.map(names);
        let pair = decide(stored.as_deref(), Some(chosen))?;
        assert_eq!(pair.name, "review.reviewers");
        assert_eq!(pair.host, None);
        let Value::ReviewerList(list) = pair.value else {
            panic!("not a reviewer list: {:?}", pair.value);
        };
        Some(list)
    }

    #[test]
    fn a_reviewer_list_that_drops_the_host_reviewer_or_keeps_a_deselected_provider_is_caught() {
        use Provider::{DeepSeek, OpenAi};
        assert_eq!(
            written_by(None, &[OpenAi]),
            Some(names(&["host", "openai"]))
        );
        assert_eq!(
            written_by(Some(&["openai"]), &[DeepSeek]),
            Some(names(&["deepseek"]))
        );
        assert_eq!(
            written_by(Some(&["host", "deepseek"]), &[]),
            Some(names(&["host"]))
        );
        assert_eq!(
            written_by(Some(&["deepseek", "host"]), &[OpenAi, DeepSeek]),
            Some(names(&["host", "openai", "deepseek"]))
        );
        assert_eq!(written_by(Some(&[]), &[OpenAi]), Some(names(&["openai"])));
    }
    #[test]
    fn a_reviewer_list_rewritten_when_it_already_names_the_selection_is_caught() {
        use Provider::OpenAi;
        let both = names(&["openai", "host"]);
        assert_eq!(decide(Some(&both), Some(&[OpenAi])), None);
        assert_eq!(decide(None, Some(&[])), None);
        assert_eq!(decide(Some(&names(&["host"])), Some(&[])), None);
        assert_eq!(decide(Some(&names(&["host", "openai"])), None), None);
    }
}
