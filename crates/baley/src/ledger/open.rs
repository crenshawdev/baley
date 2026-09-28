//! The schema understood by this CLI.
use baley_core::{Registry, register_anchor_events};
use baley_store_sqlite::Options;

/// Registers exactly the anchor types understood by the CLI.
pub(super) fn options() -> Options {
    let mut registry = Registry::new();
    register_anchor_events(&mut registry).expect("unique anchor types");
    Options {
        schema: Box::new(registry),
        ..Options::default()
    }
}
