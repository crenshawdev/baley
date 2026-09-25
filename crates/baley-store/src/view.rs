//! Views, their declarations and projectors (design 0001, Views and
//! projectors; EVD-R9, R10).

use serde_json::Value;

use crate::event::Event;

/// One value of a key or index field.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyValue {
    Text(String),
    Integer(i64),
}

/// A document's key within its view and project: the values of the view's
/// key fields, in declared order. The project is implicit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocKey(pub Vec<KeyValue>);

/// The type of a key or index field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Integer,
}

/// A key field: a top-level field of the document body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: String,
    pub kind: FieldKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Ascending,
    Descending,
}

/// One field of an index, with its order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexField {
    pub name: String,
    pub kind: FieldKind,
    pub order: Order,
}

/// A declared index. Pages follow its fields in their orders, then the
/// document key ascending, so the order is total and a cursor never skips
/// or repeats a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSpec {
    pub name: String,
    pub fields: Vec<IndexField>,
}

/// A view as the core declares it. The adapter creates the view's table and
/// indexes from this; it has no hand-written table per view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewSpec {
    pub name: String,
    /// The projector version. A stored document with an older version is
    /// rebuilt forward; a newer one makes the project read-only.
    pub version: u32,
    pub key: Vec<FieldSpec>,
    pub indexes: Vec<IndexSpec>,
    /// No page holds more documents than this.
    pub page_bound: u32,
}

/// A view document and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub key: DocKey,
    /// The sequence of the last event that changed it.
    pub produced_seq: u64,
    pub projector_version: u32,
    pub body: Value,
}

/// A position in a result, issued by the store. It is bound to the query,
/// project, generation and view version that issued it, and refused under
/// any other. It is not an access control: a caller can read or build one,
/// and the store checks only that it fits the query it is used with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor(pub String);

/// Which page to read: at most `limit` documents, capped at the view's
/// bound, after `after` or from the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub limit: u32,
    pub after: Option<Cursor>,
}

/// One page and the cursor to the next, if there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<Cursor>,
}

/// A read by a declared index: equal values for a leading prefix of the
/// index's fields, the rest ordered as declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexQuery {
    pub index: String,
    pub equals: Vec<KeyValue>,
    pub page: PageRequest,
}

/// One document change a projector returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Put { key: DocKey, body: Value },
    Delete { key: DocKey },
}

/// Why a projector could not apply an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectorError(pub String);

/// Domain code that keeps one view current. Pure: an event and the current
/// documents in, changes out. Implemented in `baley-core`, handed to the
/// adapter when the store opens, so the adapter holds no business rules.
pub trait Projector: Send + Sync {
    fn spec(&self) -> &ViewSpec;

    /// The event types this projector applies. The adapter calls it for
    /// no others.
    fn handles(&self) -> &[&str];

    /// The keys of the documents `apply` needs for this event.
    fn keys(&self, event: &Event) -> Vec<DocKey>;

    /// The changes this event makes, given the documents at `keys` as they
    /// stand after the transaction's earlier events. A missing key is
    /// absent from `documents`.
    fn apply(
        &self,
        event: &Event,
        documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError>;
}
