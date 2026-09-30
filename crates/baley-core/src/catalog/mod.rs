//! The model catalog: the model names Baley accepts per host and per
//! provider (design 0003, CFG-R19 to CFG-R23). A host's names are its
//! compiled aliases plus the owner's entries; a provider's names are what the
//! hint table seeded, what detection found and what the owner added, less what
//! the owner removed. The catalog is a view of the reserved `user` project.
//!
//! Nothing here reads a file, the environment, a clock or the store: the
//! binary supplies every event, document and version.

use std::fmt;

use crate::policy::Host;

pub mod events;
pub mod lookup;
pub mod seed;
pub mod tables;
pub mod view;

pub use events::{
    MODELS_DETECTED, MODELS_DETECTED_VERSION, MODELS_DETECTION_FAILED,
    MODELS_DETECTION_FAILED_VERSION, MODELS_OWNER_CHANGED, MODELS_OWNER_CHANGED_VERSION,
    MODELS_SEEDED, MODELS_SEEDED_VERSION, MODELS_STREAM, OwnerChange, owner_changed_payload,
    register_model_events, seeded_payload,
};
pub use lookup::{Listing, ListingRow, accepted_names, listing};
pub use seed::{seed_due, seed_payload};
pub use tables::{EXACT_HINTS, HINT_VERSION, HintRow, PREFIX_HINTS, PrefixRow, host_aliases};
pub use view::{
    CatalogState, MODEL_CATALOG_VIEW, ModelCatalogProjector, Placement, Source, catalog_key,
    model_catalog_spec, read_state, state_key,
};

#[cfg(test)]
mod tests;

/// The code of a name that is no host or provider catalog (CFG-R22).
pub const UNKNOWN_PROVIDER: &str = "unknown-provider";

/// The reserved per-user project that holds the model catalog.
///
/// It is never a UUID, so `policy::is_project_id` refuses it and no
/// `baley.toml` can name it. It holds the `models` stream, and its records
/// carry policy version 0, since the catalog is per user and runs no policy
/// step.
pub const USER_PROJECT: &str = "user";

/// A model vendor Baley reaches by API key: a catalog Baley seeds and
/// detects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    /// OpenAI.
    OpenAi,
    /// Google's Gemini.
    Gemini,
    /// DeepSeek.
    DeepSeek,
}
impl Provider {
    /// Every provider, in the catalog's order.
    pub const ALL: [Provider; 3] = [Provider::OpenAi, Provider::Gemini, Provider::DeepSeek];

    /// The name commands and events use.
    pub fn name(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Gemini => "gemini",
            Provider::DeepSeek => "deepseek",
        }
    }

    /// The provider with exactly this name, if any; case is not folded.
    pub fn parse(name: &str) -> Option<Provider> {
        Provider::ALL
            .into_iter()
            .find(|provider| provider.name() == name)
    }
}

/// How capable a model is within its provider, from the hint table, best fit
/// or the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// The provider's most capable models.
    Flagship,
    /// Between the two.
    Balanced,
    /// The cheapest models.
    Cheap,
}
impl Tier {
    /// Every tier, most capable first.
    pub const ALL: [Tier; 3] = [Tier::Flagship, Tier::Balanced, Tier::Cheap];

    /// The name `--tier` and events use.
    pub fn name(self) -> &'static str {
        match self {
            Tier::Flagship => "flagship",
            Tier::Balanced => "balanced",
            Tier::Cheap => "cheap",
        }
    }

    /// The tier with exactly this name, if any; case is not folded.
    pub fn parse(name: &str) -> Option<Tier> {
        Tier::ALL.into_iter().find(|tier| tier.name() == name)
    }
}

/// One catalog: a host's or a provider's. The derived order is `ALL`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Catalog {
    /// A host, whose compiled aliases the catalog merges with owner entries.
    Host(Host),
    /// A provider.
    Provider(Provider),
}
impl Catalog {
    /// Every catalog, in the order listings and the docs use.
    pub const ALL: [Catalog; 5] = [
        Catalog::Host(Host::ClaudeCode),
        Catalog::Host(Host::Codex),
        Catalog::Provider(Provider::OpenAi),
        Catalog::Provider(Provider::Gemini),
        Catalog::Provider(Provider::DeepSeek),
    ];

    /// The name commands and events use.
    pub fn name(self) -> &'static str {
        match self {
            Catalog::Host(host) => host.name(),
            Catalog::Provider(provider) => provider.name(),
        }
    }

    /// The catalog with exactly this name. Case is not folded, and any other
    /// name is refused with `unknown-provider`.
    pub fn parse(name: &str) -> Result<Catalog, CatalogRefusal> {
        Catalog::ALL
            .into_iter()
            .find(|catalog| catalog.name() == name)
            .ok_or_else(|| CatalogRefusal::UnknownProvider {
                name: name.to_owned(),
            })
    }
}

/// Why the catalog refused a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogRefusal {
    /// The name is no host or provider catalog. `anthropic` is refused too:
    /// nothing seeds or detects an Anthropic catalog.
    UnknownProvider {
        /// The name given.
        name: String,
    },
}
impl CatalogRefusal {
    /// The stable refusal code.
    pub fn code(&self) -> &'static str {
        match self {
            CatalogRefusal::UnknownProvider { .. } => UNKNOWN_PROVIDER,
        }
    }
}
impl fmt::Display for CatalogRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        match self {
            // Debug quoting shows an empty name and escapes control bytes
            // from the command line.
            CatalogRefusal::UnknownProvider { name } => {
                let accepted: Vec<&str> = Catalog::ALL.iter().map(|c| c.name()).collect();
                write!(
                    f,
                    "{code}: {name:?} is no model catalog; catalogs: {}",
                    accepted.join(", ")
                )
            }
        }
    }
}
