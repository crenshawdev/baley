//! Each provider's list body read into its ids. The shapes follow each
//! provider's API reference as read on 2026-09-30: OpenAI answers
//! `{"object": "list", "data": [{"id", "created", ...}]}` in one response;
//! DeepSeek answers the same shape without `created`.

use std::collections::BTreeMap;

use serde_json::Value;

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
}

/// One 2xx list body read into its ids, or `None` when it is malformed. One
/// bad item rejects the whole body: skipping it would make detection remove
/// an id the provider still serves.
pub fn parse_body(body: &[u8]) -> Option<ProviderListing> {
    let list: Value = serde_json::from_slice(body).ok()?;
    let list = list.as_object()?;
    let mut listing = ProviderListing::default();
    for item in list.get("data")?.as_array()? {
        let id = item.get("id")?.as_str().filter(|id| !id.is_empty())?;
        // The time only breaks best-fit ties, so an odd one is dropped, not
        // the id.
        let created = item.get("created").and_then(Value::as_u64);
        listing.insert(id, created);
    }
    Some(listing)
}
