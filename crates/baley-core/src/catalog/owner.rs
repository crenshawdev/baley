//! The two refusals of `baley models remove` (design 0003 section 5). An
//! addition has no refusal beyond `unknown-provider`.

use serde_json::Value;

use super::lookup::held;
use super::tables::host_aliases;
use super::{Catalog, CatalogRefusal};

/// Refuses removing a host's compiled alias: the binary owns its aliases and
/// the host resolves them whatever the catalog says. It needs no document,
/// so it runs before any store opens.
pub fn judge_alias_removal(catalog: Catalog, name: &str) -> Result<(), CatalogRefusal> {
    match catalog {
        Catalog::Host(host) if host_aliases(host).contains(&name) => {
            Err(CatalogRefusal::AliasNotRemovable {
                host,
                name: name.to_owned(),
            })
        }
        _ => Ok(()),
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
