//! One provider's observation judged: its listing, or exactly one failure
//! category (design 0003 section 6, CFG-R21).

use std::fmt;

use super::parse::{ProviderListing, parse_body};

/// Why one provider's detection failed. A category holds at most a status
/// number, never text, so no body, error or key text can reach a payload
/// through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// DNS, connect, TLS, a timeout, or a connection dropped mid-body.
    Offline,
    /// A body cut short at the 4 MiB bound.
    Incomplete,
    /// A 2xx body outside the provider's list shape.
    Malformed,
    /// 401 or 403.
    Unauthorized,
    /// 429.
    RateLimited,
    /// Any other status that is not 2xx, an unfollowed 3xx included.
    Http(u16),
    /// `keys.env` refused as exposed. The binary records it; `classify`
    /// never gives it.
    KeysFileExposed,
    /// `keys.env` refused as invalid. The binary records it; `classify`
    /// never gives it.
    KeysFileInvalid,
    /// `keys.env` refused as unreadable. The binary records it; `classify`
    /// never gives it.
    KeysFileUnreadable,
}
impl Category {
    /// The name recorded as `category`.
    pub fn name(self) -> String {
        match self {
            Category::Offline => "offline".into(),
            Category::Incomplete => "incomplete".into(),
            Category::Malformed => "malformed".into(),
            Category::Unauthorized => "unauthorized".into(),
            Category::RateLimited => "rate-limited".into(),
            Category::Http(status) => format!("http-{status}"),
            Category::KeysFileExposed => "keys-file-exposed".into(),
            Category::KeysFileInvalid => "keys-file-invalid".into(),
            Category::KeysFileUnreadable => "keys-file-unreadable".into(),
        }
    }
}

/// One response the lister received.
#[derive(Clone, PartialEq, Eq)]
pub struct ObservedResponse {
    /// The HTTP status.
    pub status: u16,
    /// The body bytes kept, up to the per-response bound.
    pub body: Vec<u8>,
    /// Whether the body was cut short at that bound.
    pub cut_short: bool,
}

// An error body can echo part of the key, so only its length is shown.
impl fmt::Debug for ObservedResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservedResponse")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .field("cut_short", &self.cut_short)
            .finish()
    }
}

/// What one provider's lister call saw. It holds no error text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observation {
    /// The response received, if the call got that far.
    pub response: Option<ObservedResponse>,
    /// A transport failure ended the call.
    pub transport_failed: bool,
}

/// The listing the observation shows, or the one category it fails with.
/// The first rule that matches wins: a transport failure, the response's
/// status, a body cut short, a malformed body.
pub fn classify(observation: &Observation) -> Result<ProviderListing, Category> {
    if observation.transport_failed {
        return Err(Category::Offline);
    }
    // The lister never returns this, and an empty listing here would remove
    // every id.
    let Some(response) = &observation.response else {
        return Err(Category::Offline);
    };
    if !(200..300).contains(&response.status) {
        return Err(match response.status {
            401 | 403 => Category::Unauthorized,
            429 => Category::RateLimited,
            status => Category::Http(status),
        });
    }
    if response.cut_short {
        return Err(Category::Incomplete);
    }
    parse_body(&response.body).ok_or(Category::Malformed)
}
