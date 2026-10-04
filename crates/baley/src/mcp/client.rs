//! Which client is calling and whether Baley supports it.
//!
//! The identity is self-reported. Selecting on it picks which host adapter
//! applies, and nothing more: it is not authorization, and a process that
//! wants to claim `claude-code` can. The decoders and the selection are pure
//! functions over plain rmcp values.

use baley_core::policy::Host;
use rmcp::model::{InitializeRequestParams, RequestMetaObject};
use serde_json::{Value, json};

/// The most bytes of a reported name or version an unknown-host answer echoes.
pub const MAX_ECHOED_BYTES: usize = 128;

/// A client's self-reported name and version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    /// The reported name, for example `claude-code`.
    pub name: String,
    /// The reported version.
    pub version: String,
}

/// The 2025 era: the identity `initialize` carried in its parameters.
pub fn decode_2025(params: &InitializeRequestParams) -> ClientIdentity {
    ClientIdentity {
        name: params.client_info.name.clone(),
        version: params.client_info.version.clone(),
    }
}

/// The 2026 era: the identity one request carried in its own `_meta`. A missing
/// or malformed key is no identity.
pub fn decode_2026(meta: Option<&RequestMetaObject>) -> Option<ClientIdentity> {
    let info = meta?.client_info()?;
    Some(ClientIdentity {
        name: info.name,
        version: info.version,
    })
}

/// A reported value as an unknown-host answer echoes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reported {
    /// The value, within the echo limit.
    Text(String),
    /// The value was too long to echo.
    OverLimit {
        /// How long the reported value was, in bytes.
        bytes: usize,
    },
}

impl Reported {
    fn bound(text: &str) -> Self {
        if text.len() > MAX_ECHOED_BYTES {
            Self::OverLimit { bytes: text.len() }
        } else {
            Self::Text(text.to_owned())
        }
    }

    /// The JSON the answer carries: the value, or a note that it was over the limit.
    pub fn value(&self) -> Value {
        match self {
            Self::Text(text) => json!(text),
            Self::OverLimit { bytes } => {
                json!(format!(
                    "over the {MAX_ECHOED_BYTES}-byte limit ({bytes} bytes)"
                ))
            }
        }
    }
}

/// The outcome of selecting a host for one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// A client Baley supports.
    Supported {
        /// The host whose adapter applies.
        host: Host,
        /// The client's reported version, not yet judged for the caller.
        client_version: String,
    },
    /// No identity, or a name Baley does not support.
    UnknownHost {
        /// The reported name, bounded, or `None` when no identity came.
        name: Option<Reported>,
        /// The reported version, bounded, or `None` when no identity came.
        version: Option<Reported>,
    },
}

/// Selects the host for one call.
///
/// A session that began with `initialize` uses the identity stored from it on
/// every call. A session that did not uses only this request's own identity,
/// never an earlier one, so the two eras never combine.
pub fn select(
    began_with_initialize: bool,
    stored: Option<&ClientIdentity>,
    current: Option<&ClientIdentity>,
) -> Selection {
    let identity = if began_with_initialize {
        stored
    } else {
        current
    };
    let Some(identity) = identity else {
        return Selection::UnknownHost {
            name: None,
            version: None,
        };
    };
    match Host::parse(&identity.name) {
        Some(host) => Selection::Supported {
            host,
            client_version: identity.version.clone(),
        },
        None => Selection::UnknownHost {
            name: Some(Reported::bound(&identity.name)),
            version: Some(Reported::bound(&identity.version)),
        },
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::{ClientCapabilities, Implementation};

    use super::*;

    fn identity(name: &str, version: &str) -> ClientIdentity {
        ClientIdentity {
            name: name.into(),
            version: version.into(),
        }
    }

    fn meta(value: Value) -> RequestMetaObject {
        serde_json::from_value(value).unwrap()
    }

    fn claude_meta() -> RequestMetaObject {
        meta(json!({"io.modelcontextprotocol/clientInfo":
            {"name": "claude-code", "version": "2.1.287"}}))
    }

    #[test]
    fn the_2025_decoder_reads_the_initialize_client_info() {
        let params = InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new("claude-code", "2.1.287"),
        );
        assert_eq!(decode_2025(&params), identity("claude-code", "2.1.287"));
    }

    #[test]
    fn the_2026_decoder_reads_the_request_meta_and_treats_a_bad_key_as_none() {
        assert_eq!(
            decode_2026(Some(&claude_meta())),
            Some(identity("claude-code", "2.1.287"))
        );
        assert_eq!(decode_2026(None), None);
        assert_eq!(decode_2026(Some(&meta(json!({})))), None);
        let malformed = meta(json!({"io.modelcontextprotocol/clientInfo": "claude-code"}));
        assert_eq!(decode_2026(Some(&malformed)), None);
    }

    #[test]
    fn an_initialize_session_honours_its_identity_on_a_call_with_no_meta() {
        let stored = identity("claude-code", "2.1.287");
        assert_eq!(
            select(true, Some(&stored), None),
            Selection::Supported {
                host: Host::ClaudeCode,
                client_version: "2.1.287".into()
            }
        );
    }

    #[test]
    fn a_session_without_initialize_never_reuses_an_earlier_identity() {
        let earlier = identity("claude-code", "2.1.287");
        assert_eq!(
            select(false, Some(&earlier), None),
            Selection::UnknownHost {
                name: None,
                version: None
            }
        );
    }

    #[test]
    fn a_session_without_initialize_uses_the_current_requests_identity() {
        let current = decode_2026(Some(&claude_meta()));
        assert_eq!(
            select(false, None, current.as_ref()),
            Selection::Supported {
                host: Host::ClaudeCode,
                client_version: "2.1.287".into()
            }
        );
    }

    #[test]
    fn an_initialize_session_ignores_a_different_identity_in_a_later_meta() {
        let stored = identity("claude-code", "2.1.287");
        let later = identity("codex", "1.0");
        assert!(matches!(
            select(true, Some(&stored), Some(&later)),
            Selection::Supported { .. }
        ));
    }

    #[test]
    fn codex_a_miscased_name_and_an_empty_name_are_not_supported() {
        for name in ["codex", "Claude-Code", ""] {
            let current = identity(name, "1");
            assert!(
                matches!(
                    select(false, None, Some(&current)),
                    Selection::UnknownHost { .. }
                ),
                "{name:?} must not be supported"
            );
        }
    }

    #[test]
    fn a_reported_name_over_128_bytes_is_not_echoed_whole() {
        let current = identity(&"n".repeat(300), "1.0");
        let Selection::UnknownHost { name, version } = select(false, None, Some(&current)) else {
            panic!("an unknown name is not supported");
        };
        assert_eq!(name, Some(Reported::OverLimit { bytes: 300 }));
        assert_eq!(version, Some(Reported::Text("1.0".into())));
        let echoed = name.unwrap().value().to_string();
        assert!(!echoed.contains(&"n".repeat(129)));
    }

    #[test]
    fn a_reported_name_of_exactly_128_bytes_is_echoed_whole() {
        let name = "n".repeat(MAX_ECHOED_BYTES);
        let current = identity(&name, "1.0");
        let Selection::UnknownHost { name: echoed, .. } = select(false, None, Some(&current))
        else {
            panic!("an unknown name is not supported");
        };
        assert_eq!(echoed, Some(Reported::Text(name)));
    }
}
