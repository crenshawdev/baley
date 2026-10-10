//! The `install` view keeps the latest `install.recorded` payload and its
//! sequence for each host. Update events stay on their stream without a view.

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Value, json};

use super::event::INSTALL_RECORDED;

/// The view's name.
pub const INSTALL_VIEW: &str = "install";

/// Declares a text host key with no index.
pub fn install_spec() -> ViewSpec {
    ViewSpec {
        name: INSTALL_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: "host".into(),
            kind: FieldKind::Text,
        }],
        indexes: vec![],
        page_bound: 100,
    }
}

/// The key of a host's latest installation record.
pub fn install_key(host: &str) -> DocKey {
    DocKey(vec![KeyValue::Text(host.into())])
}

/// Replaces each host's document with `{host, seq, payload}` from its latest
/// `install.recorded`, preserving the whole payload as ownership evidence.
pub struct InstallProjector {
    spec: ViewSpec,
}

impl InstallProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: install_spec(),
        }
    }
}

impl Default for InstallProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for InstallProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[INSTALL_RECORDED]
    }

    // Each event replaces the whole document, so no previous value is needed.
    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        vec![]
    }

    fn apply(
        &self,
        event: &Event,
        _documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: &str| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let Value::Object(payload) = &event.payload else {
            return Err(refuse("the payload is not an object"));
        };
        let Some(Value::String(host)) = payload.get("host") else {
            return Err(refuse("host is missing or not text"));
        };
        Ok(vec![Change::Put {
            key: install_key(host),
            body: json!({"host": host, "seq": event.seq, "payload": event.payload}),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baley_store::{Actor, Hash, ProjectId, RequestId};

    fn event(seq: u64, payload: Value) -> Event {
        Event {
            project_id: ProjectId("user".into()),
            seq,
            stream: "install".into(),
            stream_version: 1,
            type_name: "install.recorded".into(),
            type_version: 1,
            actor: Actor::Owner,
            caller: None,
            recorded_at: "2026-10-10T09:00:00Z".into(),
            request_id: RequestId("00000000-0000-4000-8000-000000000001".into()),
            git: None,
            policy_version: 0,
            payload,
            prev_hash: Some(Hash([0; 32])),
            hash: Hash([1; 32]),
        }
    }

    #[test]
    fn an_install_view_keeping_an_older_record_is_caught() {
        let projector = InstallProjector::new();
        assert_eq!(projector.handles(), ["install.recorded"]);
        assert_eq!(projector.spec().name, "install");
        assert_eq!(
            projector.spec().key,
            [FieldSpec {
                name: "host".into(),
                kind: FieldKind::Text,
            }]
        );
        assert!(projector.spec().indexes.is_empty());
        let first = event(
            3,
            json!({"host": "claude-code", "binary_version": "0.1.0", "stubs": ["old"]}),
        );
        let changes = projector.apply(&first, &[]).unwrap();
        let [Change::Put { key, body }] = changes.as_slice() else {
            panic!("one host document must be written");
        };
        assert_eq!(key, &DocKey(vec![KeyValue::Text("claude-code".into())]));
        assert_eq!(
            body,
            &json!({
                "host": "claude-code", "seq": 3,
                "payload": {"host": "claude-code", "binary_version": "0.1.0", "stubs": ["old"]},
            })
        );

        let second = event(
            7,
            json!({"host": "claude-code", "binary_version": "0.2.0", "stubs": []}),
        );
        assert_eq!(
            projector
                .apply(&second, &[(key.clone(), body.clone())])
                .unwrap(),
            [Change::Put {
                key: DocKey(vec![KeyValue::Text("claude-code".into())]),
                body: json!({
                    "host": "claude-code", "seq": 7,
                    "payload": {"host": "claude-code", "binary_version": "0.2.0", "stubs": []},
                }),
            }]
        );
    }

    #[test]
    fn an_install_record_without_a_host_projected_is_caught() {
        let projector = InstallProjector::new();
        for (payload, reason) in [
            (json!({}), "host is missing or not text"),
            (json!({"host": 3}), "host is missing or not text"),
            (json!([]), "the payload is not an object"),
        ] {
            let refused = projector.apply(&event(4, payload), &[]).unwrap_err();
            assert_eq!(refused.0, format!("install.recorded at seq 4: {reason}"));
        }
    }
}
