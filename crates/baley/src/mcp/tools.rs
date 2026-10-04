//! The tools and what the always-available `baley_version` answers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::envelope::Envelope;

/// What `baley_version` reports on success.
///
/// A struct rather than a bare string because an `ok` envelope's payload sits
/// beside the `status` tag at the top level, so it has to have named fields
/// (see `envelope::Envelope`). Three of them, and each answers a different
/// question a user actually asks when a session behaves unexpectedly: which
/// release, and which of the four release archives.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VersionReport {
    /// The crate version of the running binary, matching the release tag it
    /// was cut from.
    pub version: String,
    /// The operating system this binary was built for: `linux` or `macos`.
    pub os: String,
    /// The CPU architecture this binary was built for: `x86_64` or `aarch64`.
    pub arch: String,
}

/// Answers `baley_version`. Only an empty arguments object is accepted.
///
/// Read from this binary's own compile-time constants, never from a manifest
/// on disk: the question is which binary is serving, and a file beside it can
/// be from a different install.
pub fn version_answer(arguments: Option<&Value>) -> Value {
    let envelope = match arguments {
        Some(Value::Object(map)) if map.is_empty() => Envelope::Ok(VersionReport {
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }),
        _ => Envelope::Refused {
            code: "invalid-arguments".into(),
            reason: "baley_version requires an empty arguments object".into(),
        },
    };
    serde_json::to_value(envelope).expect("version envelope")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn version_with_empty_arguments_is_ok_with_version_os_and_arch() {
        let answer = version_answer(Some(&json!({})));
        assert_eq!(answer["status"], "ok");
        assert_eq!(answer["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(answer["os"], std::env::consts::OS);
        assert_eq!(answer["arch"], std::env::consts::ARCH);
    }

    #[test]
    fn version_with_arguments_missing_or_not_an_object_is_refused() {
        for arguments in [
            Some(json!({"x": 1})),
            None,
            Some(json!([])),
            Some(json!(null)),
        ] {
            let answer = version_answer(arguments.as_ref());
            assert_eq!(answer["status"], "refused", "{arguments:?}");
            assert_eq!(answer["code"], "invalid-arguments");
        }
    }
}
