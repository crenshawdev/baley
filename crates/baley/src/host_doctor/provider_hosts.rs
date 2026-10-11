//! The review providers whose API host the placed settings document allows
//! in `sandbox.network.allowedDomains`, where the interview adds each chosen
//! provider's host. Only catalog hosts count, matched exactly; any other
//! host is the owner's. An allowed host is the owner's choice, never a gap,
//! and says nothing about a provider acknowledgement.

use std::path::PathBuf;

use baley_core::catalog::Provider;
use serde_json::Value;

/// What the provider-host check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judged {
    /// The settings document was read.
    Allowed {
        /// The settings file.
        settings: PathBuf,
        /// The providers whose API host it allows, in catalog order.
        providers: Vec<Provider>,
    },
    /// The settings placement is unknown.
    Unknown,
    /// The settings document could not be used.
    Unusable,
}

/// The providers whose API host `document` allows, in catalog order.
pub fn allowed(document: &Value) -> Vec<Provider> {
    let Some(hosts) = document
        .pointer("/sandbox/network/allowedDomains")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    Provider::ALL
        .into_iter()
        .filter(|provider| {
            hosts
                .iter()
                .any(|host| host.as_str() == Some(provider.api_host()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn named(document: &Value) -> Vec<(&'static str, &'static str)> {
        allowed(document)
            .into_iter()
            .map(|provider| (provider.name(), provider.api_host()))
            .collect()
    }

    #[test]
    fn a_provider_host_matched_loosely_or_listed_out_of_catalog_order_is_caught() {
        let domains = |hosts: Value| json!({"sandbox": {"network": {"allowedDomains": hosts}}});
        assert_eq!(
            named(&domains(json!([
                "api.deepseek.com",
                "API.OPENAI.COM",
                "api.openai.com.example",
                7,
                "api.openai.com",
            ]))),
            [
                ("openai", "api.openai.com"),
                ("deepseek", "api.deepseek.com")
            ]
        );
        assert_eq!(
            named(&domains(json!([
                "API.OPENAI.COM",
                "api.openai.com.example"
            ]))),
            []
        );
        assert_eq!(named(&json!({"sandbox": {"network": {}}})), []);
        assert_eq!(
            named(&json!({"sandbox": {"network": "api.openai.com"}})),
            []
        );
    }
}
