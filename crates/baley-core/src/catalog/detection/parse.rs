//! Each provider's list page read into its ids. The shapes follow each
//! provider's API reference as read on 2026-09-30: OpenAI answers
//! `{"object": "list", "data": [{"id", "created", ...}]}` in one page;
//! DeepSeek answers the same shape without `created`; Gemini answers
//! `{"models": [{"name": "models/<id>", ...}], "nextPageToken"}` and leaves
//! out an empty list, as the proto3 JSON mapping does.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::catalog::Provider;

/// The ids one provider listed, each once and in id order, with the
/// creation time it reported in whole seconds, if any. No kind of model is
/// filtered out: embedding, speech, image and moderation ids count too.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderListing {
    models: BTreeMap<String, Option<u64>>,
}
impl ProviderListing {
    /// Each id with its creation time, in id order.
    pub fn models(&self) -> impl Iterator<Item = (&str, Option<u64>)> {
        self.models
            .iter()
            .map(|(id, created)| (id.as_str(), *created))
    }

    /// Whether the provider listed `id`.
    pub fn contains(&self, id: &str) -> bool {
        self.models.contains_key(id)
    }

    // A repeated id counts once, with the time it first came with.
    fn insert(&mut self, id: &str, created: Option<u64>) {
        self.models.entry(id.to_owned()).or_insert(created);
    }

    pub(super) fn merge(&mut self, page: ProviderListing) {
        for (id, created) in page.models {
            self.insert(&id, created);
        }
    }
}

/// One 2xx page body read into its ids, or `None` when it is malformed. One
/// bad item rejects the whole page: skipping it would make detection remove
/// an id the provider still serves.
pub fn parse_page(provider: Provider, body: &[u8]) -> Option<ProviderListing> {
    let page: Value = serde_json::from_slice(body).ok()?;
    let page = page.as_object()?;
    let mut listing = ProviderListing::default();
    match provider {
        Provider::OpenAi | Provider::DeepSeek => {
            for item in page.get("data")?.as_array()? {
                let id = item.get("id")?.as_str().filter(|id| !id.is_empty())?;
                // The time only breaks best-fit ties, so an odd one is
                // dropped, not the id.
                let created = item.get("created").and_then(Value::as_u64);
                listing.insert(id, created);
            }
        }
        Provider::Gemini => {
            // A broken continuation must never end a listing as complete.
            if page
                .get("nextPageToken")
                .is_some_and(|token| !token.is_string())
            {
                return None;
            }
            let Some(models) = page.get("models") else {
                return Some(listing);
            };
            for item in models.as_array()? {
                let name = item.get("name")?.as_str()?;
                let id = name.strip_prefix("models/").filter(|id| !id.is_empty())?;
                listing.insert(id, None);
            }
        }
    }
    Some(listing)
}
