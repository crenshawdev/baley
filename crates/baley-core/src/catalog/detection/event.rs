//! The event one provider's detection records: `models.detected` for a
//! listing, `models.detection_failed` for a category (design 0003 section 6,
//! CFG-R20, CFG-R21). The binary calls this inside its recording
//! transaction, with the document and catalog version read there.

use serde_json::Value;

use super::classify::Category;
use super::diff::{Report, diff};
use super::parse::ProviderListing;
use crate::catalog::events::{
    MODELS_DETECTED, MODELS_DETECTED_VERSION, MODELS_DETECTION_FAILED,
    MODELS_DETECTION_FAILED_VERSION, detected_payload, detection_failed_payload,
};
use crate::catalog::{EXACT_HINTS, HINT_VERSION, PREFIX_HINTS, Provider};

/// What one provider's detection records, and what the owner is told.
#[derive(Debug, Clone, PartialEq)]
pub struct ChosenEvent {
    /// The event type.
    pub type_name: &'static str,
    /// Its payload version.
    pub type_version: u32,
    /// The payload.
    pub payload: Value,
    /// The report of a listing, or the category of a failure.
    pub outcome: Outcome,
}

/// How one provider's detection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The provider listed its ids.
    Detected(Report),
    /// It failed with this category, and the catalog stays as it was.
    Failed(Category),
}

/// The event for `classified`, diffed against `provider`'s catalog
/// document with the compiled hint rows. `catalog_version` is the version
/// read beside the document. A failure carries no `removed`, so a cut or
/// failed listing removes nothing.
pub fn choose_event(
    provider: Provider,
    classified: Result<ProviderListing, Category>,
    document: Option<&Value>,
    catalog_version: u64,
) -> ChosenEvent {
    match classified {
        Ok(listing) => {
            let diff = diff(provider, &listing, EXACT_HINTS, PREFIX_HINTS, document);
            ChosenEvent {
                type_name: MODELS_DETECTED,
                type_version: MODELS_DETECTED_VERSION,
                // Seeding runs first, so the compiled version is the
                // recorded one.
                payload: detected_payload(
                    provider,
                    &diff.added,
                    &diff.removed,
                    catalog_version,
                    HINT_VERSION,
                ),
                outcome: Outcome::Detected(diff.report),
            }
        }
        Err(category) => ChosenEvent {
            type_name: MODELS_DETECTION_FAILED,
            type_version: MODELS_DETECTION_FAILED_VERSION,
            payload: detection_failed_payload(provider, category, catalog_version),
            outcome: Outcome::Failed(category),
        },
    }
}
