//! The catalog as route resolution and `baley models list` read it, from the
//! view's documents (design 0003 sections 5 and 6, CFG-R14, CFG-R19).

use std::collections::BTreeSet;

use serde_json::Value;

use super::tables::host_aliases;
use super::view::{Entry, Placement, Source, read_entries, read_state};
use super::{Catalog, CatalogRefusal, Tier};
use crate::policy::AcceptedNames;

/// The entries a catalog document accepts: every one the owner did not
/// remove. A document that does not read is taken as absent, as
/// [`read_state`] takes one.
pub(super) fn held(document: Option<&Value>) -> Vec<Entry> {
    read_entries(document)
        .unwrap_or_default()
        .into_values()
        .filter(|entry| !entry.owner_removed)
        .collect()
}

/// The names one catalog accepts and the catalog version, from that
/// catalog's document and the state document read with
/// [`catalog_key`](super::catalog_key) and [`state_key`](super::state_key).
/// A host's names are its compiled aliases plus its owner entries; a
/// provider's are its seeded, detected and owner entries.
pub fn accepted_names(
    catalog: &str,
    document: Option<&Value>,
    state: Option<&Value>,
) -> Result<AcceptedNames, CatalogRefusal> {
    let catalog = Catalog::parse(catalog)?;
    let mut names: BTreeSet<String> = held(document).into_iter().map(|entry| entry.id).collect();
    if let Catalog::Host(host) = catalog {
        names.extend(host_aliases(host).iter().map(|alias| alias.to_string()));
    }
    let version = read_state(state).catalog_version;
    Ok(AcceptedNames { names, version })
}

/// One accepted name as `baley models list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingRow {
    /// The catalog that accepts it.
    pub catalog: Catalog,
    /// The name.
    pub name: String,
    /// Where it came from.
    pub source: Source,
    /// Its tier; absent for an alias and for an owner entry without one.
    pub tier: Option<Tier>,
    /// How it got its tier; absent for an alias.
    pub placed: Option<Placement>,
}

/// The catalogs asked for, one row per accepted name, and the catalog
/// version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The catalog version the rows were read at.
    pub version: u64,
    /// By catalog in [`Catalog::ALL`] order, then by name, the alias row
    /// first. A name that is both an alias and an owner entry has both rows.
    pub rows: Vec<ListingRow>,
}

/// The listing of each catalog given, with its document, and the state
/// document.
pub fn listing(documents: &[(Catalog, Option<&Value>)], state: Option<&Value>) -> Listing {
    let mut rows = Vec::new();
    for &(catalog, document) in documents {
        if let Catalog::Host(host) = catalog {
            rows.extend(host_aliases(host).iter().map(|alias| ListingRow {
                catalog,
                name: alias.to_string(),
                source: Source::Alias,
                tier: None,
                placed: None,
            }));
        }
        rows.extend(held(document).into_iter().map(|entry| ListingRow {
            catalog,
            name: entry.id,
            source: entry.source,
            tier: entry.tier,
            placed: Some(entry.placed),
        }));
    }
    rows.sort_by(|a, b| (a.catalog, &a.name, a.source).cmp(&(b.catalog, &b.name, b.source)));
    Listing {
        version: read_state(state).catalog_version,
        rows,
    }
}
