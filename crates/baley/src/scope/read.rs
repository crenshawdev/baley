//! The draft preview read (design 0004 section 5): a draft by its digest,
//! as the latest submission of that digest recorded it, so the owner sees
//! exactly what an approval would record. Phase numbers and story ids are
//! assigned at approval, so a preview has none. `document`'s draft kind and
//! `baley scope preview` both read through here (D-20).

use std::io::Read;

use baley_core::scope::{DRAFT_VIEW, draft_key};
use baley_store::{Document, Hash, PayloadBody, Payloads, ProjectId, StoreError, Views};
use serde_json::Value;

/// A draft as its latest submission recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The change kind, such as `start`.
    pub kind: String,
    /// The submission's digest.
    pub digest: Hash,
    /// The sequence of the latest submission of this digest.
    pub seq: u64,
    /// When the latest submission of this digest was recorded.
    pub recorded_at: String,
    /// The submission as recorded.
    pub submission: Value,
}

/// Why a preview could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    /// No submission has this digest, answered `unknown-draft`.
    UnknownDraft,
    /// The draft is recorded, but its document or body cannot be read: a
    /// body no longer held, or one that is not JSON. Callers answer it
    /// `failed` `ledger-unavailable`, never as a missing draft.
    Unreadable(String),
    /// The store's own error.
    Store(StoreError),
}

/// Reads the draft with `digest` in `project`: its `draft` document, then
/// its body by hash, parsed as JSON. Nothing is appended.
pub fn preview<S: Views + Payloads + ?Sized>(
    store: &S,
    project: &ProjectId,
    digest: Hash,
) -> Result<Preview, PreviewError> {
    match store.get(project, DRAFT_VIEW, &draft_key(digest)) {
        Ok(Some(document)) => preview_of(store, &document),
        Ok(None) => Err(PreviewError::UnknownDraft),
        Err(error) => Err(PreviewError::Store(error)),
    }
}

/// The preview a `draft` document stands for, its body opened through
/// `payloads`.
pub(crate) fn preview_of<P: Payloads + ?Sized>(
    payloads: &P,
    document: &Document,
) -> Result<Preview, PreviewError> {
    let unreadable = || PreviewError::Unreadable("the draft's view document cannot be read".into());
    let body = &document.body;
    let text = |name: &str| body.get(name).and_then(Value::as_str);
    let kind = text("kind").ok_or_else(unreadable)?;
    let digest = text("digest")
        .and_then(Hash::from_hex)
        .ok_or_else(unreadable)?;
    let seq = body
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(unreadable)?;
    let recorded_at = text("recorded_at").ok_or_else(unreadable)?;
    let hash = text("hash")
        .and_then(Hash::from_hex)
        .ok_or_else(unreadable)?;
    let mut reader = match payloads.open(&hash).map_err(PreviewError::Store)? {
        PayloadBody::Present(reader) => reader,
        PayloadBody::Gone(_) => {
            return Err(PreviewError::Unreadable(
                "the draft is recorded, but its body is no longer held".into(),
            ));
        }
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| PreviewError::Store(StoreError::Unavailable(error.to_string())))?;
    let submission = serde_json::from_slice(&bytes).map_err(|_| {
        PreviewError::Unreadable("the draft is recorded, but its body is not JSON".into())
    })?;
    Ok(Preview {
        kind: kind.into(),
        digest,
        seq,
        recorded_at: recorded_at.into(),
        submission,
    })
}

#[cfg(test)]
mod tests {
    use baley_core::scope::{
        BACKLOG_VIEW, PHASE_STATUS_INDEX, PHASE_VIEW, ROADMAP_VIEW, backlog_key, roadmap_key,
    };
    use baley_store::{IndexQuery, KeyValue, PageRequest};

    use super::*;
    use crate::scope::fixtures::*;

    fn digest_of(recorded: &baley_store::Recorded) -> Hash {
        Hash::from_hex(receipt(recorded)["digest"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn a_draft_lost_on_reopening_or_projected_as_accepted_scope_is_caught() {
        let (dir, store) = store();
        let digest = digest_of(&submit(&store, &s1(), R1, 1, T1).unwrap());

        assert!(
            store
                .get(&project(), ROADMAP_VIEW, &roadmap_key())
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get(&project(), BACKLOG_VIEW, &backlog_key())
                .unwrap()
                .is_none()
        );
        let declared = IndexQuery {
            index: PHASE_STATUS_INDEX.into(),
            equals: vec![KeyValue::Text("declared".into())],
            page: PageRequest {
                limit: 10,
                after: None,
            },
        };
        assert!(
            store
                .find(&project(), PHASE_VIEW, &declared)
                .unwrap()
                .items
                .is_empty()
        );

        drop(store);
        let reopened = crate::ledger::open::store(
            &dir.path().join("home"),
            T2,
            crate::ledger::open::options(),
        )
        .unwrap();
        let read = preview(&reopened, &project(), digest).unwrap();
        assert_eq!(read.kind, "start");
        assert_eq!(read.digest, digest);
        assert_eq!(read.recorded_at, T1);
        assert_eq!(read.submission, s1());
    }

    #[test]
    fn an_unknown_digest_read_as_an_empty_draft_is_caught() {
        let (_dir, store) = store();
        assert_eq!(
            preview(&store, &project(), Hash([0; 32])),
            Err(PreviewError::UnknownDraft)
        );
    }

    #[test]
    fn a_resubmitted_draft_read_with_its_first_time_is_caught() {
        let (_dir, store) = store();
        let digest = digest_of(&submit(&store, &m(), R1, 1, T1).unwrap());
        submit(&store, &m(), R2, 1, T2).unwrap();
        assert_eq!(preview(&store, &project(), digest).unwrap().recorded_at, T2);
    }
}
