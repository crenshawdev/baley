//! The schema understood by this CLI.
use std::num::NonZeroU32;

use baley_core::catalog::{ModelCatalogProjector, register_model_events};
use baley_core::{Registry, register_anchor_events, register_project_events};
use baley_store_sqlite::Options;

/// Registers exactly the anchor types, `project.initialized` and the four
/// `models.*` types understood by the CLI, and declares view set version 3,
/// `claim_scope model_catalog request`.
pub(crate) fn options() -> Options {
    let mut registry = Registry::new();
    register_anchor_events(&mut registry).expect("unique anchor types");
    register_project_events(&mut registry).expect("unique project types");
    register_model_events(&mut registry).expect("unique model types");
    Options {
        schema: Box::new(registry),
        projectors: vec![Box::new(ModelCatalogProjector::new())],
        view_set_version: NonZeroU32::new(3).expect("nonzero"),
        ..Options::default()
    }
}

/// Creates the ledger home when missing, then opens it with the caller's schema.
pub(crate) fn store(
    home: &std::path::Path,
    at: &str,
    options: Options,
) -> Result<baley_store_sqlite::SqliteStore, baley_store::StoreError> {
    crate::folders::create_private(home).map_err(|error| {
        baley_store::StoreError::Unavailable(format!("cannot create {}: {error}", home.display()))
    })?;
    baley_store_sqlite::SqliteStore::open(home, at, options)
}
