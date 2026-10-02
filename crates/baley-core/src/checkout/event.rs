//! The `checkout.seen` event: its type, its registration, the checkout as
//! gathered and the payload built from it (design 0001 Project identity and
//! policy).

use serde_json::{Value, json};

use crate::registry::{Registry, RegistryError};

/// A checkout seen under a project: `{path, root_commit, remote_url}`, with
/// `null` for a root commit or remote URL the checkout lacks. Built by
/// [`seen_payload`].
pub const CHECKOUT_SEEN: &str = "checkout.seen";
/// The current `checkout.seen` payload version.
pub const CHECKOUT_SEEN_VERSION: u32 = 1;

/// Registers `checkout.seen` at version 1, with no upcasters. The event goes
/// on a project's `project` stream.
pub fn register_checkout_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(CHECKOUT_SEEN, CHECKOUT_SEEN_VERSION, [])
}

/// One checkout as gathered, with its path as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The checkout's root.
    pub path: String,
    /// The repository's root commit, `None` for an unborn HEAD.
    pub root_commit: Option<String>,
    /// The remote URL with its user information already stripped, the form
    /// the binary records and compares. The core never sees an unstripped
    /// URL and never parses one.
    pub remote_url: Option<String>,
}

/// The `checkout.seen` payload for `checkout`. The event carries the project
/// and the time, so the payload holds neither. An absent field is `null`,
/// never a missing member.
pub fn seen_payload(checkout: &Checkout) -> Value {
    json!({
        "path": checkout.path,
        "root_commit": checkout.root_commit,
        "remote_url": checkout.remote_url,
    })
}
