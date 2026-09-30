//! Which page a listing asks for next. The lister follows these decisions
//! and judges nothing itself.

use serde_json::Value;

use crate::catalog::Provider;

/// The most pages one provider's listing may take. A 20th page that still
/// continues ends the listing cut short.
pub const PAGE_BOUND: usize = 20;

/// The continuation a 2xx page body gives: Gemini's non-empty
/// `nextPageToken`, and nothing for OpenAI or DeepSeek, whose list endpoints
/// do not page. A token that is not text gives nothing, and the parser
/// rejects that page, so the listing ends malformed, never complete.
pub fn next_page(provider: Provider, body: &[u8]) -> Option<String> {
    match provider {
        Provider::OpenAi | Provider::DeepSeek => None,
        Provider::Gemini => {
            let page: Value = serde_json::from_slice(body).ok()?;
            let token = page.get("nextPageToken")?.as_str()?;
            (!token.is_empty()).then(|| token.to_owned())
        }
    }
}

/// What the lister does after a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Paging {
    /// No continuation: the listing is complete.
    Complete,
    /// Ask for the page this continuation names.
    Follow(String),
    /// The page bound was reached with a continuation left: the listing is
    /// cut short, and `classify` makes it `incomplete`.
    CutShort,
}

/// The decision after `pages_received` pages, the latest of which gave
/// `continuation`.
pub fn paging(pages_received: usize, continuation: Option<String>) -> Paging {
    match continuation {
        None => Paging::Complete,
        Some(_) if pages_received >= PAGE_BOUND => Paging::CutShort,
        Some(token) => Paging::Follow(token),
    }
}
