//! Recall accepts this projection, never an unfiltered revision log.
use super::model::ItemRecord;
use super::writer::View;
use std::collections::BTreeMap;

pub struct RecallItems<'a> {
    records: Vec<&'a ItemRecord>,
}
impl<'a> RecallItems<'a> {
    pub fn iter(&self) -> impl Iterator<Item = &'a ItemRecord> + '_ {
        self.records.iter().copied()
    }
}
impl View {
    pub fn recall_items(&self) -> RecallItems<'_> {
        let latest: BTreeMap<&str, u64> = self
            .items
            .iter()
            .map(|item| (item.id.as_str(), item.revision))
            .collect();
        RecallItems {
            records: self
                .items
                .iter()
                .filter(|item| latest[item.id.as_str()] == item.revision)
                .collect(),
        }
    }

    /// Explicit evidence/dedup lookup. This is not a recall input.
    pub fn lookup_item(&self, id: &str) -> Option<&ItemRecord> {
        self.items.iter().rev().find(|item| item.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{Disposition, Evidence, Origin, Snapshot, VERSION};

    fn item(id: &str, text: &str) -> ItemRecord {
        ItemRecord {
            version: VERSION,
            id: id.into(),
            revision: 1,
            origin: Origin { source: "capture".into(), original: Evidence::Missing },
            text: text.into(),
            kind: "todo".into(),
            phase: None,
            disposition: Disposition::Captured,
            completed: false,
        }
    }

    fn view(items: Vec<ItemRecord>) -> View {
        View { items, decisions: vec![], snapshot: Snapshot::new(1, b"", b"", serde_json::Value::Null).unwrap() }
    }

    fn recalled(view: &View) -> Vec<(&str, u64)> {
        view.recall_items().iter().map(|item| (item.id.as_str(), item.revision)).collect()
    }

    #[test]
    fn recall_offers_each_identity_once_at_its_latest_revision() {
        let first = item("first", "same words");
        let mut completed = first.clone();
        completed.revision = 2;
        completed.completed = true;
        let log = view(vec![first, item("second", "same words"), completed]);
        assert_eq!(recalled(&log), [("second", 1), ("first", 2)]);
    }

    #[test]
    fn lookup_returns_the_latest_revision() {
        let captured = item("quasar", "unique quasar");
        let mut completed = captured.clone();
        completed.revision = 2;
        completed.completed = true;
        let log = view(vec![captured, completed.clone()]);
        assert_eq!(log.lookup_item("quasar"), Some(&completed));
        assert_eq!(log.lookup_item("absent"), None);
    }
}
