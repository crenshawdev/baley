//! The outside providers the owner has stored: the global file's top-level
//! `review.reviewers` (design 0003 section 5, Provider selection). Pure over
//! the parsed layer; the interview shows the result before it asks.

use baley_core::catalog::Provider;
use baley_core::policy::{ParsedLayer, Value};

/// The setting the stored selection is read from.
const SETTING: &str = "review.reviewers";

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

/// The selection the global file stores. Only the file's top level counts: a
/// host section's value belongs to that host, and `host` is the host's own
/// reviewer, not an outside provider. With no value written the selection is
/// none from `default`.
pub(super) fn stored(global: Option<&ParsedLayer>) -> Selection {
    let written = global.and_then(|layer| {
        layer
            .values
            .iter()
            .find(|written| written.host.is_none() && written.name == SETTING)
    });
    let Some(Value::ReviewerList(names)) = written.map(|written| &written.value) else {
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
}
