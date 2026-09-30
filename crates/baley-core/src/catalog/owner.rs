//! The refusals of `baley models add` and `baley models remove` beyond
//! `unknown-provider` (design 0003 section 5).

use serde_json::Value;

use super::lookup::held;
use super::tables::host_aliases;
use super::{Catalog, CatalogRefusal};
use crate::policy::Host;

/// The host whose compiled alias `name` is, if the catalog is a host's.
fn compiled_alias(catalog: Catalog, name: &str) -> Option<Host> {
    match catalog {
        Catalog::Host(host) if host_aliases(host).contains(&name) => Some(host),
        _ => None,
    }
}

/// Refuses removing a host's compiled alias: the binary owns its aliases and
/// the host resolves them whatever the catalog says. It needs no document,
/// so it runs before any store opens.
pub fn judge_alias_removal(catalog: Catalog, name: &str) -> Result<(), CatalogRefusal> {
    match compiled_alias(catalog, name) {
        Some(host) => Err(CatalogRefusal::AliasNotRemovable {
            host,
            name: name.to_owned(),
        }),
        None => Ok(()),
    }
}

/// Refuses adding a host's compiled alias as an owner entry: the host
/// already accepts it, so the entry would change no accepted name and could
/// never be removed. It needs no document, so it runs before any store
/// opens.
pub fn judge_alias_addition(catalog: Catalog, name: &str) -> Result<(), CatalogRefusal> {
    match compiled_alias(catalog, name) {
        Some(host) => Err(CatalogRefusal::AliasNotAddable {
            host,
            name: name.to_owned(),
        }),
        None => Ok(()),
    }
}

/// Refuses removing a name the catalog's document does not accept; an id
/// the owner already removed is not held. It runs inside the removal's
/// transaction, on the document read there.
pub fn judge_held_removal(
    catalog: Catalog,
    name: &str,
    document: Option<&Value>,
) -> Result<(), CatalogRefusal> {
    if held(document).iter().any(|entry| entry.id == name) {
        Ok(())
    } else {
        Err(CatalogRefusal::UnknownModel {
            catalog,
            name: name.to_owned(),
        })
    }
}
