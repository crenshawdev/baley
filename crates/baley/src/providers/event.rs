//! The `providers.acknowledged` event (design 0003 section 6).
//!
//! It goes on stream `providers` in the per-user project `user`. No view keeps
//! a document for it: the reuse check and Build 4 read the stream directly,
//! as the update events are read, so the view set stays where it is. The
//! builder here is pure; recording the event is the caller's.

use baley_core::catalog::Provider;
use baley_core::{Registry, RegistryError};
use baley_store::{NewEvent, StreamName};
use serde_json::json;

use super::warning::{WARNING_VERSION, warning_sha256};

/// The owner confirmed the warning for these providers:
/// `{providers, warning_version, warning_sha256, owner, acknowledged_at}`.
pub const PROVIDERS_ACKNOWLEDGED: &str = "providers.acknowledged";
/// The current payload version of the event.
pub const PROVIDERS_ACKNOWLEDGED_VERSION: u32 = 1;
/// The stream the event goes on in `user`.
pub const PROVIDERS_STREAM: &str = "providers";

/// Registers `providers.acknowledged` at version 1, with no upcasters.
pub fn register_provider_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(PROVIDERS_ACKNOWLEDGED, PROVIDERS_ACKNOWLEDGED_VERSION, [])
}

/// The `providers.acknowledged` event for the confirmed `providers`.
///
/// The payload lists each provider once, in catalog order, whatever the order
/// or repeats of the argument. It names the compiled warning's version and
/// SHA-256, the owner identity text and the supplied time. It holds no
/// credential or environment value.
pub fn acknowledged_event(providers: &[Provider], owner: &str, at: &str) -> NewEvent {
    let names: Vec<&str> = Provider::ALL
        .into_iter()
        .filter(|provider| providers.contains(provider))
        .map(Provider::name)
        .collect();
    NewEvent {
        stream: StreamName(PROVIDERS_STREAM.into()),
        type_name: PROVIDERS_ACKNOWLEDGED.into(),
        type_version: PROVIDERS_ACKNOWLEDGED_VERSION,
        git: None,
        payload: json!({
            "providers": names,
            "warning_version": WARNING_VERSION,
            "warning_sha256": warning_sha256(),
            "owner": owner,
            "acknowledged_at": at,
        }),
        attachments: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fixtures::{DESIGN_WARNING, sha256_hex};

    #[test]
    fn an_acknowledgement_listing_providers_out_of_catalog_order_or_twice_is_caught() {
        let event = acknowledged_event(
            &[Provider::DeepSeek, Provider::OpenAi, Provider::DeepSeek],
            "Ada Owner",
            "2026-10-10T09:00:00Z",
        );
        assert_eq!(event.stream, StreamName("providers".into()));
        assert_eq!(event.type_name, "providers.acknowledged");
        assert_eq!(event.type_version, 1);
        assert!(event.git.is_none());
        assert!(event.attachments.is_empty());
        assert_eq!(
            event.payload,
            json!({
                "providers": ["openai", "deepseek"],
                "warning_version": 1,
                "warning_sha256": sha256_hex(DESIGN_WARNING),
                "owner": "Ada Owner",
                "acknowledged_at": "2026-10-10T09:00:00Z",
            })
        );
    }
}
