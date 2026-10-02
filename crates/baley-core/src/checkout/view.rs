//! The `checkout` view: one document per checkout path within a project,
//! holding the latest `checkout.seen` payload (design 0001 Project identity
//! and policy).

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, IndexField, IndexSpec, KeyValue, Order, Projector,
    ProjectorError, ViewSpec,
};
use serde_json::Value;

use super::event::CHECKOUT_SEEN;

/// The view's name.
pub const CHECKOUT_VIEW: &str = "checkout";

/// The key field: the checkout's root, as the payload holds it.
const PATH_FIELD: &str = "path";

/// The view's declaration: the path as a text key and one index over it.
///
/// The index exists so checkout admission can list every row of the project
/// with `Transaction::find` and empty `equals`. It is over `path`, not
/// `remote_url`, because an index field must be non-null text in every body
/// and a checkout with no remote has none.
pub fn checkout_spec() -> ViewSpec {
    ViewSpec {
        name: CHECKOUT_VIEW.into(),
        version: 1,
        key: vec![FieldSpec {
            name: PATH_FIELD.into(),
            kind: FieldKind::Text,
        }],
        indexes: vec![IndexSpec {
            name: "by_path".into(),
            fields: vec![IndexField {
                name: PATH_FIELD.into(),
                kind: FieldKind::Text,
                order: Order::Ascending,
            }],
        }],
        page_bound: 100,
    }
}

/// The key of one checkout's document.
pub fn checkout_key(path: &str) -> DocKey {
    DocKey(vec![KeyValue::Text(path.into())])
}

/// Keeps the `checkout` view current from `checkout.seen`: the latest event
/// for a path replaces its document whole.
///
/// It never deletes. A row stays until a later `checkout.seen` for the same
/// path replaces it, since rows do not expire.
pub struct CheckoutProjector {
    spec: ViewSpec,
}

impl CheckoutProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: checkout_spec(),
        }
    }
}

impl Default for CheckoutProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for CheckoutProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[CHECKOUT_SEEN]
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
        let path = match payload.get(PATH_FIELD) {
            Some(Value::String(path)) if !path.is_empty() => path,
            _ => return Err(refuse("path is missing, not text or empty")),
        };
        for member in ["root_commit", "remote_url"] {
            match payload.get(member) {
                Some(Value::Null) => {}
                Some(Value::String(text)) if !text.is_empty() => {}
                _ => {
                    return Err(refuse(&format!(
                        "{member} is neither null nor non-empty text"
                    )));
                }
            }
        }
        Ok(vec![Change::Put {
            key: checkout_key(path),
            body: event.payload.clone(),
        }])
    }
}
