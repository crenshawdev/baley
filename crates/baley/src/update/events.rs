//! The two update events and their payloads (design 0012 section 6).
//!
//! `update.checked` and `update.failed` go to the per-user `user` project on
//! stream `install`, at version 1. The `install` view keeps only
//! `install.recorded`, so these events are read by their stream and nothing
//! keeps a document for them. The builders here are pure; recording them is
//! the caller's.

use baley_core::{Registry, RegistryError};
use baley_store::{NewEvent, RequestId, StreamName};
use serde_json::{Value, json};

use super::version::Version;

/// A check finished and found the installation current or staged a version.
pub const UPDATE_CHECKED: &str = "update.checked";
/// A check ended at a step that failed.
pub const UPDATE_FAILED: &str = "update.failed";
/// The current payload version of both update events.
pub const UPDATE_EVENT_VERSION: u32 = 1;
/// The stream both update events go on in `user`.
pub const INSTALL_STREAM: &str = "install";

/// Registers `update.checked` and `update.failed` at version 1, with no
/// upcasters.
pub fn register_update_events(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register(UPDATE_CHECKED, UPDATE_EVENT_VERSION, [])?;
    registry.register(UPDATE_FAILED, UPDATE_EVENT_VERSION, [])
}

/// The claim a check ran under: its request id and the sequence of its
/// `command.claimed` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimRef<'a> {
    /// The claim's request id.
    pub request_id: &'a RequestId,
    /// The sequence of the claim's event.
    pub seq: u64,
}

/// What both events say about a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts<'a> {
    /// The installation, the stable path as text.
    pub installation: &'a str,
    /// The UTC day the check was claimed for, `YYYY-MM-DD`.
    pub day: &'a str,
    /// The claim the check ran under.
    pub claim: ClaimRef<'a>,
    /// When the check finished or failed, as a UTC instant.
    pub at: &'a str,
    /// The version the stable path runs, when it names one.
    pub active_version: Option<Version>,
    /// The version the check staged, when it staged one.
    pub staged_version: Option<Version>,
}

/// How a finished check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The offered version was not higher.
    Current,
    /// A higher version was staged and activated.
    Staged,
}

impl CheckOutcome {
    /// The text the event records.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckOutcome::Current => "current",
            CheckOutcome::Staged => "staged",
        }
    }
}

/// Why a check failed: a closed set, so a reader of the ledger can match
/// the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCode {
    /// The stable path is not managed by Baley, so there is nothing to update.
    NotInstalled,
    /// The source could not be reached.
    NetworkUnavailable,
    /// The manifest was unreadable or malformed.
    ManifestInvalid,
    /// The download's SHA-256 differs from the manifest's.
    ChecksumMismatch,
    /// A different file already holds the version's place in the versions folder.
    StagingConflict,
    /// The stable path changed while the check ran.
    ActivationConflict,
    /// A folder or file the check needs could not be written.
    NotWritable,
    /// The check was interrupted and reconciled afterwards.
    Interrupted,
}

impl FailureCode {
    /// Every code, in the order the design lists them.
    pub const ALL: [FailureCode; 8] = [
        FailureCode::NotInstalled,
        FailureCode::NetworkUnavailable,
        FailureCode::ManifestInvalid,
        FailureCode::ChecksumMismatch,
        FailureCode::StagingConflict,
        FailureCode::ActivationConflict,
        FailureCode::NotWritable,
        FailureCode::Interrupted,
    ];

    /// The code as the event and the owner's output spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            FailureCode::NotInstalled => "update-not-installed",
            FailureCode::NetworkUnavailable => "update-network-unavailable",
            FailureCode::ManifestInvalid => "update-manifest-invalid",
            FailureCode::ChecksumMismatch => "update-checksum-mismatch",
            FailureCode::StagingConflict => "update-staging-conflict",
            FailureCode::ActivationConflict => "update-activation-conflict",
            FailureCode::NotWritable => "not-writable",
            FailureCode::Interrupted => "update-interrupted",
        }
    }
}

fn claim_json(claim: &ClaimRef<'_>) -> Value {
    json!({"request_id": claim.request_id.0, "seq": claim.seq})
}

fn version_json(version: Option<Version>) -> Value {
    version.map_or(Value::Null, |version| version.to_string().into())
}

fn on_install_stream(type_name: &str, payload: Value) -> NewEvent {
    NewEvent {
        stream: StreamName(INSTALL_STREAM.into()),
        type_name: type_name.into(),
        type_version: UPDATE_EVENT_VERSION,
        git: None,
        payload,
        attachments: vec![],
    }
}

/// The `update.checked` event: the installation, the day, the claim, when the
/// check finished, how it ended and the active and staged versions, each
/// version `null` when there is none.
pub fn checked_event(facts: &Facts<'_>, outcome: CheckOutcome) -> NewEvent {
    on_install_stream(
        UPDATE_CHECKED,
        json!({
            "installation": facts.installation,
            "day": facts.day,
            "claim": claim_json(&facts.claim),
            "checked_at": facts.at,
            "outcome": outcome.as_str(),
            "active_version": version_json(facts.active_version),
            "staged_version": version_json(facts.staged_version),
        }),
    )
}

/// The `update.failed` event: the same facts as [`checked_event`], with the
/// failure code in place of the outcome.
pub fn failed_event(facts: &Facts<'_>, code: FailureCode) -> NewEvent {
    on_install_stream(
        UPDATE_FAILED,
        json!({
            "installation": facts.installation,
            "day": facts.day,
            "claim": claim_json(&facts.claim),
            "observed_at": facts.at,
            "code": code.as_str(),
            "active_version": version_json(facts.active_version),
            "staged_version": version_json(facts.staged_version),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Option<Version> {
        Some(Version::parse(text).unwrap())
    }

    #[test]
    fn an_update_event_missing_a_design_fact_or_on_another_stream_is_caught() {
        let request = RequestId("00000000-0000-4000-8000-0000000000aa".into());
        let facts = |at, active, staged| Facts {
            installation: "/home/o/.local/bin/baley",
            day: "2026-10-08",
            claim: ClaimRef {
                request_id: &request,
                seq: 12,
            },
            at,
            active_version: active,
            staged_version: staged,
        };
        let claim = json!({"request_id": "00000000-0000-4000-8000-0000000000aa", "seq": 12});

        let staged = checked_event(
            &facts("2026-10-08T09:00:05Z", version("0.2.0"), version("0.2.0")),
            CheckOutcome::Staged,
        );
        let current = checked_event(
            &facts("2026-10-08T09:00:05Z", version("0.1.0"), None),
            CheckOutcome::Current,
        );
        let failed = failed_event(
            &facts("2026-10-08T09:00:06Z", version("0.1.0"), None),
            FailureCode::ChecksumMismatch,
        );

        for (event, type_name) in [
            (&staged, "update.checked"),
            (&current, "update.checked"),
            (&failed, "update.failed"),
        ] {
            assert_eq!(event.stream, StreamName("install".into()));
            assert_eq!(event.type_version, 1);
            assert_eq!(event.type_name, type_name);
            assert!(event.attachments.is_empty());
        }
        assert_eq!(
            staged.payload,
            json!({
                "installation": "/home/o/.local/bin/baley",
                "day": "2026-10-08",
                "claim": claim,
                "checked_at": "2026-10-08T09:00:05Z",
                "outcome": "staged",
                "active_version": "0.2.0",
                "staged_version": "0.2.0",
            })
        );
        assert_eq!(
            current.payload,
            json!({
                "installation": "/home/o/.local/bin/baley",
                "day": "2026-10-08",
                "claim": claim,
                "checked_at": "2026-10-08T09:00:05Z",
                "outcome": "current",
                "active_version": "0.1.0",
                "staged_version": null,
            })
        );
        assert_eq!(
            failed.payload,
            json!({
                "installation": "/home/o/.local/bin/baley",
                "day": "2026-10-08",
                "claim": claim,
                "observed_at": "2026-10-08T09:00:06Z",
                "code": "update-checksum-mismatch",
                "active_version": "0.1.0",
                "staged_version": null,
            })
        );
    }

    #[test]
    fn an_unbounded_or_misspelled_failure_code_is_caught() {
        let texts: Vec<&str> = FailureCode::ALL.iter().map(|code| code.as_str()).collect();
        assert_eq!(
            texts,
            [
                "update-not-installed",
                "update-network-unavailable",
                "update-manifest-invalid",
                "update-checksum-mismatch",
                "update-staging-conflict",
                "update-activation-conflict",
                "not-writable",
                "update-interrupted",
            ]
        );
    }
}
