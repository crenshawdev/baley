//! Detection: refreshes the OpenAI and DeepSeek catalogs from each
//! provider's model-list endpoint (design 0003 section 6, CFG-R20,
//! CFG-R21). The judging lives in `baley_core::catalog::detection`; this
//! module gathers each listing with its key and records the outcome.

mod lister;
mod record;
mod trigger;

use baley_core::catalog::{ListingRow, Provider};
use baley_store::{Admin, Ledger, StoreError, Views};

pub use lister::{HttpLister, ModelLister};
pub use record::{Recording, detection_request, record, unverifiable};
pub use trigger::{
    Action, DETECT_COMMAND, KeysRead, Steps, Trigger, UPDATE_COMMAND, judge, key_name,
    refusal_category,
};

use crate::keys::{Keys, KeysRefusal};
use crate::ledger::commands::new_request_id;
use crate::models;

/// How one provider's step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderOutcome {
    /// Not covered, or keyless and not named: nothing recorded or shown.
    Skipped,
    /// Named by the owner without its key: the key name and the detected
    /// entries it cannot verify. Nothing recorded.
    Missing {
        /// The key name `keys.env` lacks.
        key: &'static str,
        /// The provider's accepted detected entries.
        entries: Vec<ListingRow>,
    },
    /// A detection or a failure was recorded.
    Recorded(Recording),
}

/// What one detection run did, for its caller to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    /// The run seeded the catalog before recording.
    pub seeded: bool,
    /// Each provider's outcome, in [`Provider::ALL`] order.
    pub providers: Vec<(Provider, ProviderOutcome)>,
}

/// Runs one detection: judges each provider's step from `keys` (what
/// `keys::load` gave) and `trigger`, seeds when anything will be recorded,
/// lists the keyed providers concurrently on the caller's runtime, then
/// records each provider in its own command. Every trigger calls this; a
/// caller outside tokio builds a current-thread runtime and blocks on it.
///
/// A store error stops the run. Providers already recorded stay recorded.
pub async fn detect(
    store: &(impl Admin + Views + Ledger),
    lister: &impl ModelLister,
    keys: Result<&Keys, &KeysRefusal>,
    trigger: &Trigger,
    at: &str,
) -> Result<Detection, StoreError> {
    let read = match keys {
        Ok(keys) => KeysRead::Present(
            Provider::ALL
                .map(key_name)
                .into_iter()
                .filter(|name| keys.get(name).is_ok())
                .collect(),
        ),
        Err(refusal) => KeysRead::Refused(refusal.code()),
    };
    let steps = judge(trigger, &read);
    // With nothing to record, `user` is neither created nor seeded.
    let seeded = steps.records() && models::seed(store, new_request_id(), at)?;

    // `Key` is borrowed and not `'static`, so the providers are joined on
    // this task rather than handed to others.
    let list = |(provider, action): &(Provider, Action)| {
        let key = match (action, keys) {
            (Action::List, Ok(keys)) => keys.get(key_name(*provider)).ok(),
            _ => None,
        };
        let provider = *provider;
        async move {
            match key {
                Some(key) => Some(lister.list(provider, key).await),
                None => None,
            }
        }
    };
    let [openai, deepseek] = &steps.0;
    let (openai, deepseek) = tokio::join!(list(openai), list(deepseek));

    let mut providers = Vec::with_capacity(2);
    for ((provider, action), observation) in steps.0.into_iter().zip([openai, deepseek]) {
        let outcome = match (action, observation) {
            (_, Some(observation)) => ProviderOutcome::Recorded(record(
                store,
                provider,
                trigger,
                Ok(&observation),
                new_request_id(),
                at,
            )?),
            (Action::RecordFailure(category), None) => ProviderOutcome::Recorded(record(
                store,
                provider,
                trigger,
                Err(category),
                new_request_id(),
                at,
            )?),
            (Action::ReportMissing(key), None) => ProviderOutcome::Missing {
                key,
                entries: unverifiable(store, provider)?,
            },
            _ => ProviderOutcome::Skipped,
        };
        providers.push((provider, outcome));
    }
    Ok(Detection { seeded, providers })
}

/// Runs [`detect`] with the HTTPS lister on a current-thread runtime of its
/// own, for a command outside tokio. A runtime that will not start is
/// reported as the store being unavailable.
pub(crate) fn detect_blocking(
    store: &(impl Admin + Views + Ledger),
    keys: Result<&Keys, &KeysRefusal>,
    trigger: &Trigger,
    at: &str,
) -> Result<Detection, StoreError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|error| StoreError::Unavailable(format!("detection cannot start: {error}")))?;
    runtime.block_on(detect(store, &HttpLister::new(), keys, trigger, at))
}
