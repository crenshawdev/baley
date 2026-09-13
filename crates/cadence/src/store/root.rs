//! Durable record identity, distinct from the live directory observation.
use super::{
    Error, Observed, Result,
    model::Snapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Root {
    pub bound: String,
    pub path: PathBuf,
    pub relocations: Vec<Relocation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relocation {
    pub from: String,
    pub to: String,
    pub generation: u64,
}

fn retained(value: &Value, chains: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if key == "root_binding" {
                    if let Some(chain) = value.as_str() {
                        chains.insert(chain.to_owned());
                    }
                } else {
                    retained(value, chains);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                retained(value, chains);
            }
        }
        _ => {}
    }
}

fn legacy_binding(data: &Value, live: &str) -> Result<String> {
    let mut chains = BTreeSet::new();
    retained(data, &mut chains);
    if chains.len() > 1 {
        return Err(Error::Conflict(format!(
            "retained root bindings disagree: {chains:?}"
        )));
    }
    Ok(chains.into_iter().next().unwrap_or_else(|| live.to_owned()))
}

pub fn binding(observed: &Observed) -> Result<String> {
    let Some(bytes) = &observed.bytes else {
        return Ok(observed.directory_identity.clone());
    };
    let snapshot: Snapshot = serde_json::from_slice(bytes)?;
    if let Some(root) = snapshot.root {
        return Ok(root.bound);
    }
    legacy_binding(&snapshot.data, &observed.directory_identity)
}

/// Check before parsing the generation so a changed root with damaged bytes
/// gets the same diagnostic as a changed root at a different path.
pub fn at_open(
    observed: &Observed,
    path: &Path,
    items: &[u8],
    decisions: &[u8],
) -> Result<Option<Root>> {
    let Some(bytes) = &observed.bytes else {
        return Ok(None);
    };
    let snapshot: Snapshot = serde_json::from_slice(bytes)?;
    let recorded = snapshot.root.as_ref();
    let mut root = if let Some(recorded) = recorded {
        recorded.clone()
    } else {
        let bound = legacy_binding(&snapshot.data, &observed.directory_identity)?;
        Root {
            bound,
            path: path.to_owned(),
            relocations: vec![],
        }
    };
    let live = &observed.directory_identity;
    let last = root.relocations.last().map_or(&root.bound, |change| &change.to);
    let refused = |why: &str| Error::Conflict(format!(
        "changed root binding: {why}; bound {}, last {}, live {}; recorded path {}, now {}",
        root.bound, last, live, root.path.display(), path.display()));
    if root.path != path {
        return Err(refused("the store directory moved"));
    }
    if last != live {
        snapshot
            .validate(items, decisions)
            .map_err(|_| refused("the bytes differ from the snapshot"))?;
        root.relocations.push(Relocation {
            from: last.clone(),
            to: live.clone(),
            generation: snapshot
                .generation
                .checked_add(1)
                .ok_or_else(|| Error::Invalid("generation overflow".into()))?,
        });
        return Ok(Some(root));
    }
    Ok(recorded.is_none().then_some(root))
}
