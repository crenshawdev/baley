//! The schema understood by this CLI. Only the per-session server's open
//! runs the startup `quick_check`; every command-line open leaves it off.
use std::num::NonZeroU32;

use baley_core::catalog::{ModelCatalogProjector, register_model_events};
use baley_core::checkout::{CheckoutProjector, register_checkout_events};
use baley_core::policy::recorded::{PolicyProjector, register_policy_events};
use baley_core::{Registry, register_anchor_events, register_project_events};
use baley_store_sqlite::Options;

/// Registers exactly the anchor types, `project.initialized`, the four
/// `models.*` types, `policy.effective` and `checkout.seen` understood by the
/// CLI, and declares view set version 5,
/// `checkout claim_scope model_catalog policy request`.
/// The startup `quick_check` is off, so every command-line caller opens as
/// before.
pub(crate) fn options() -> Options {
    let mut registry = Registry::new();
    register_anchor_events(&mut registry).expect("unique anchor types");
    register_project_events(&mut registry).expect("unique project types");
    register_model_events(&mut registry).expect("unique model types");
    register_policy_events(&mut registry).expect("unique policy types");
    register_checkout_events(&mut registry).expect("unique checkout types");
    Options {
        schema: Box::new(registry),
        projectors: vec![
            Box::new(ModelCatalogProjector::new()),
            Box::new(PolicyProjector::new()),
            Box::new(CheckoutProjector::new()),
        ],
        view_set_version: NonZeroU32::new(5).expect("nonzero"),
        ..Options::default()
    }
}

/// The registry of `options()` with the startup `quick_check` on, for the
/// per-session server's open. It is built from `options()` so the two never
/// disagree on events, views or the view set version.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the per-session server opens the ledger with it once it is wired in"
    )
)]
pub(crate) fn server_options() -> Options {
    Options {
        startup_check: true,
        ..options()
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

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-10-01T10:00:00Z";
    const T1: &str = "2026-10-01T10:00:01Z";

    /// The options as they stood at view set 3, before the `policy` view.
    fn view_set_3() -> Options {
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        Options {
            schema: Box::new(registry),
            projectors: vec![Box::new(ModelCatalogProjector::new())],
            view_set_version: NonZeroU32::new(3).unwrap(),
            ..Options::default()
        }
    }

    /// The options as they stood at view set 4, before the `checkout` view.
    fn view_set_4() -> Options {
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        Options {
            schema: Box::new(registry),
            projectors: vec![
                Box::new(ModelCatalogProjector::new()),
                Box::new(PolicyProjector::new()),
            ],
            view_set_version: NonZeroU32::new(4).unwrap(),
            ..Options::default()
        }
    }

    // Catches a server open that declares other views or another view set
    // version than the command line, which would fence one or the other.
    #[test]
    fn the_server_variant_declares_a_different_view_set_from_options() {
        let names = |options: &Options| -> Vec<String> {
            options
                .projectors
                .iter()
                .map(|projector| projector.spec().name.clone())
                .collect()
        };
        let (cli, server) = (options(), server_options());
        assert_eq!(server.view_set_version, cli.view_set_version);
        assert_eq!(names(&server), names(&cli));
    }

    // Catches the command line running the startup check.
    #[test]
    fn options_selects_the_startup_check() {
        assert!(!options().startup_check);
    }

    // Catches a server open that does not run the startup check.
    #[test]
    fn the_server_variant_does_not_select_the_startup_check() {
        assert!(server_options().startup_check);
    }

    #[test]
    fn a_view_added_without_raising_the_set_version_is_refused_at_open() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        drop(store(&home, T0, view_set_4()).unwrap());

        let reopened = store(&home, T1, options());

        assert!(reopened.is_ok(), "{:?}", reopened.err());
    }

    #[test]
    fn a_ledger_written_at_view_set_3_is_not_refused_by_the_policy_view() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        drop(store(&home, T0, view_set_3()).unwrap());

        let reopened = store(&home, T1, options());

        assert!(reopened.is_ok(), "{:?}", reopened.err());
    }
}
