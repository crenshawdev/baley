//! The `policy` view: one document per checkout and host within a project,
//! holding the latest `policy.effective` payload (design 0003 section 6).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::Value;

use super::event::POLICY_EFFECTIVE;
use crate::policy::schema::Host;

/// The view's name.
pub const POLICY_VIEW: &str = "policy";

/// The first key field: the checkout's root, as the payload holds it.
const CHECKOUT_FIELD: &str = "checkout";

/// The second key field. It cannot be `host`: the payload's `host` is
/// `null` for the command line, and a key field must be text in the body.
const HOST_KEY_FIELD: &str = "host_key";

/// The view's declaration: the checkout and host as text keys, no indexes,
/// and page bound 1, since every read is a `get` by key.
pub fn policy_spec() -> ViewSpec {
    ViewSpec {
        name: POLICY_VIEW.into(),
        version: 1,
        key: vec![
            FieldSpec {
                name: CHECKOUT_FIELD.into(),
                kind: FieldKind::Text,
            },
            FieldSpec {
                name: HOST_KEY_FIELD.into(),
                kind: FieldKind::Text,
            },
        ],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// The key of one checkout's document for `host`. The command line's host
/// text is `""`, which no host's name can be, so the two never collide.
pub fn policy_key(checkout: &str, host: Option<Host>) -> DocKey {
    let host = host.map_or("", Host::name);
    DocKey(vec![
        KeyValue::Text(checkout.into()),
        KeyValue::Text(host.into()),
    ])
}

/// Keeps the `policy` view current from `policy.effective`: the latest
/// event for a key replaces its document whole.
pub struct PolicyProjector {
    spec: ViewSpec,
}

impl PolicyProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: policy_spec(),
        }
    }
}

impl Default for PolicyProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for PolicyProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[POLICY_EFFECTIVE]
    }

    // The latest event wins, so no stored document is needed.
    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        Vec::new()
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
        let checkout = match payload.get(CHECKOUT_FIELD) {
            Some(Value::String(checkout)) if !checkout.is_empty() => checkout,
            _ => return Err(refuse("checkout is missing, not text or empty")),
        };
        // A `""` host would take the command line's key.
        let host = match payload.get("host") {
            Some(Value::Null) => "",
            Some(Value::String(host)) if !host.is_empty() => host,
            _ => return Err(refuse("host is neither null nor non-empty text")),
        };
        let key = DocKey(vec![
            KeyValue::Text(checkout.clone()),
            KeyValue::Text(host.into()),
        ]);
        let mut body = payload.clone();
        body.insert(HOST_KEY_FIELD.into(), host.into());
        body.insert("version".into(), event.seq.into());
        Ok(vec![Change::Put {
            key,
            body: Value::Object(body),
        }])
    }
}
