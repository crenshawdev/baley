//! The schema understood by this CLI. Only the per-session server's open
//! runs the startup `quick_check`; every command-line open leaves it off.
//! Only the guard's open bounds its storage waits.
use std::num::NonZeroU32;
use std::time::Duration;

use baley_core::capture::{CaptureProjector, register_capture_events};
use baley_core::catalog::{ModelCatalogProjector, register_model_events};
use baley_core::checkout::{CheckoutProjector, register_checkout_events};
use baley_core::guard::{GuardPolicyProjector, GuardProjector, register_guard_events};
use baley_core::policy::recorded::{PolicyProjector, register_policy_events};
use baley_core::{Registry, register_anchor_events, register_project_events};
use baley_store_sqlite::Options;

/// Registers exactly the anchor types, `project.initialized`, the four
/// `models.*` types, `policy.effective`, `checkout.seen`, `capture.recorded`,
/// `guard.answered` and `guard.policy_recorded` understood by the CLI, and
/// declares view set version 7,
/// `capture checkout claim_scope guard guard_policy model_catalog policy request`.
/// The startup `quick_check` is off, so every command-line caller opens as
/// before.
pub(crate) fn options() -> Options {
    let mut registry = Registry::new();
    register_anchor_events(&mut registry).expect("unique anchor types");
    register_project_events(&mut registry).expect("unique project types");
    register_model_events(&mut registry).expect("unique model types");
    register_policy_events(&mut registry).expect("unique policy types");
    register_checkout_events(&mut registry).expect("unique checkout types");
    register_capture_events(&mut registry).expect("unique capture types");
    register_guard_events(&mut registry).expect("unique guard types");
    Options {
        schema: Box::new(registry),
        projectors: vec![
            Box::new(ModelCatalogProjector::new()),
            Box::new(PolicyProjector::new()),
            Box::new(CheckoutProjector::new()),
            Box::new(CaptureProjector::new()),
            Box::new(GuardProjector::new()),
            Box::new(GuardPolicyProjector::new()),
        ],
        view_set_version: NonZeroU32::new(7).expect("nonzero"),
        ..Options::default()
    }
}

/// The registry of `options()` with the startup `quick_check` on, for the
/// per-session server's open. It is built from `options()` so the two never
/// disagree on events, views or the view set version.
pub(crate) fn server_options() -> Options {
    Options {
        startup_check: true,
        ..options()
    }
}

/// The registry of `options()` for the guard hook's open, with
/// `storage_time`, the storage wait the guard's budget has left, as its
/// guard storage time. The store then waits for nothing past it, answering
/// `StoreError::Busy`, and answers views behind this binary's as
/// `StoreError::NeedsRebuild` instead of rebuilding them. It is built from
/// `options()` so the two never disagree on events, views or the view set
/// version, and the startup `quick_check` stays off. Build 3 T10's guard
/// hook is its caller.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Build 3 T10's guard hook is its first caller")
)]
pub(crate) fn guard_options(storage_time: Duration) -> Options {
    Options {
        guard_storage_time: Some(storage_time),
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

    /// The options as they stood at view set 5, before the `capture` view.
    fn view_set_5() -> Options {
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        register_checkout_events(&mut registry).unwrap();
        Options {
            schema: Box::new(registry),
            projectors: vec![
                Box::new(ModelCatalogProjector::new()),
                Box::new(PolicyProjector::new()),
                Box::new(CheckoutProjector::new()),
            ],
            view_set_version: NonZeroU32::new(5).unwrap(),
            ..Options::default()
        }
    }

    /// The options as they stood at view set 6, before the guard views.
    fn view_set_6() -> Options {
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        register_checkout_events(&mut registry).unwrap();
        register_capture_events(&mut registry).unwrap();
        Options {
            schema: Box::new(registry),
            projectors: vec![
                Box::new(ModelCatalogProjector::new()),
                Box::new(PolicyProjector::new()),
                Box::new(CheckoutProjector::new()),
                Box::new(CaptureProjector::new()),
            ],
            view_set_version: NonZeroU32::new(6).unwrap(),
            ..Options::default()
        }
    }

    /// The names of the views the options declare, in order.
    fn view_names(options: &Options) -> Vec<String> {
        options
            .projectors
            .iter()
            .map(|projector| projector.spec().name.clone())
            .collect()
    }

    // Catches a server open that declares other views or another view set
    // version than the command line, which would fence one or the other.
    #[test]
    fn the_server_variant_declares_a_different_view_set_from_options() {
        let (cli, server) = (options(), server_options());
        assert_eq!(server.view_set_version, cli.view_set_version);
        assert_eq!(view_names(&server), view_names(&cli));
    }

    /// A clock that always reads the same, so a guard store's storage time
    /// never runs out here, and a pause that returns at once.
    struct Fixed;

    impl baley_store_sqlite::Timing for Fixed {
        fn now(&self) -> Duration {
            Duration::from_secs(1)
        }

        fn pause(&self, _duration: Duration) {}
    }

    /// The guard's options with 1.5 s of storage time on the fixed clock.
    fn guard() -> Options {
        Options {
            timing: std::sync::Arc::new(Fixed),
            ..guard_options(Duration::from_millis(1_500))
        }
    }

    fn seed_request() -> baley_store::RequestId {
        baley_store::RequestId("00000000-0000-4000-8000-000000000001".into())
    }

    // Catches a guard open that declares other events, views or another
    // view set version than the command line, which would fence one or the
    // other.
    #[test]
    fn the_guard_variant_declares_a_different_view_set_from_options() {
        use baley_core::capture::{CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION};
        use baley_core::guard::{GUARD_ANSWERED, GUARD_ANSWERED_VERSION};

        let (cli, guard) = (options(), guard_options(Duration::from_millis(1_500)));
        assert_eq!(guard.view_set_version, cli.view_set_version);
        assert_eq!(view_names(&guard), view_names(&cli));
        assert!(
            guard
                .schema
                .reads(CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION)
        );
        assert!(guard.schema.reads(GUARD_ANSWERED, GUARD_ANSWERED_VERSION));
    }

    // Catches a guard open that runs the startup check, or loses the storage
    // time it was given.
    #[test]
    fn the_guard_variant_runs_the_startup_check_or_drops_its_storage_time() {
        let guard = guard_options(Duration::from_millis(1_500));
        assert!(!guard.startup_check);
        assert_eq!(guard.guard_storage_time, Some(Duration::from_millis(1_500)));
    }

    // Catches guard options that open a normal store, which rebuilds
    // `user`'s views inline on the hook's path, or an answer that flips them.
    #[test]
    fn a_guard_open_rebuilds_user_views_left_at_view_set_5() {
        use crate::models;
        use baley_core::catalog::{HINT_VERSION, USER_PROJECT};
        use baley_store::{ProjectId, StoreError};

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let old = store(&home, T0, view_set_5()).unwrap();
        models::create_user(&old, T0).unwrap();
        models::record_seed(&old, seed_request(), T0).unwrap();
        drop(old);

        let guarded = store(&home, T1, guard()).unwrap();
        assert_eq!(
            models::observe_hint_version(&guarded),
            Err(StoreError::NeedsRebuild {
                project: ProjectId(USER_PROJECT.into())
            })
        );
        drop(guarded);
        let old = store(&home, T1, view_set_5()).unwrap();
        assert_eq!(models::observe_hint_version(&old), Ok(Some(HINT_VERSION)));
        drop(old);
        // The rebuild that brings it current reads the store's clock.
        let current = store(
            &home,
            T1,
            Options {
                timing: std::sync::Arc::new(Fixed),
                ..options()
            },
        )
        .unwrap();
        assert_eq!(
            models::observe_hint_version(&current),
            Ok(Some(HINT_VERSION))
        );
    }

    // Catches guard options that refuse a new `user`, which has no views to
    // rebuild, as needing a rebuild, so every guarded ask would be denied.
    #[test]
    fn a_new_user_under_guard_options_needs_a_rebuild() {
        use crate::models;
        use baley_core::catalog::HINT_VERSION;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let guarded = store(&home, T0, guard()).unwrap();
        assert_eq!(models::create_user(&guarded, T0), Ok(true));
        assert_eq!(models::record_seed(&guarded, seed_request(), T0), Ok(true));
        assert_eq!(
            models::observe_hint_version(&guarded),
            Ok(Some(HINT_VERSION))
        );
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
    fn a_ledger_written_at_view_set_5_is_refused_by_the_capture_view() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        drop(store(&home, T0, view_set_5()).unwrap());

        let reopened = store(&home, T1, options());

        assert!(reopened.is_ok(), "{:?}", reopened.err());
    }

    // Catches the guard views added without raising the view set version,
    // which the store refuses at reopen, or a set version that fences a
    // ledger the previous binary wrote.
    #[test]
    fn a_guard_view_added_without_raising_the_set_version_or_refused_at_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        drop(store(&home, T0, view_set_6()).unwrap());

        let reopened = store(&home, T1, options());

        assert!(reopened.is_ok(), "{:?}", reopened.err());
    }

    // Catches a capture view left unbuilt for a ledger the previous binary
    // made: the capture commits and is then found through the id index.
    #[test]
    fn a_capture_in_a_ledger_made_at_view_set_5_missing_from_the_capture_view() {
        use baley_core::capture::{
            CAPTURE_ID_INDEX, CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION, CAPTURE_STREAM,
            CAPTURE_VIEW, CaptureKind, inline_payload,
        };
        use baley_store::{
            Actor, Admin, Command, CommandKind, Decision, Hash, IndexQuery, KeyValue, NewEvent,
            Observed, OutcomeKind, PageRequest, ProjectId, Recorded, RequestId, StreamName, Views,
        };

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        let old = store(&home, T0, view_set_5()).unwrap();
        old.create_project(&project, "sample", T0).unwrap();
        drop(old);
        let store = store(&home, T1, options()).unwrap();

        let command = Command {
            project: project.clone(),
            kind: CommandKind("capture.record".into()),
            request_id: RequestId("9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f".into()),
            digest: Hash([7; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: T1.into(),
            actor: Actor::Baley,
            caller: None,
        };
        let recorded = store
            .transact(&command, &mut |tx| {
                tx.append(NewEvent {
                    stream: StreamName(CAPTURE_STREAM.into()),
                    type_name: CAPTURE_RECORDED.into(),
                    type_version: CAPTURE_RECORDED_VERSION,
                    git: None,
                    payload: inline_payload("c1", CaptureKind::Note, None, "keep this"),
                    attachments: vec![],
                })?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: serde_json::json!({}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
        assert!(matches!(recorded, Recorded::New { .. }), "{recorded:?}");

        let found = store
            .find(
                &project,
                CAPTURE_VIEW,
                &IndexQuery {
                    index: CAPTURE_ID_INDEX.into(),
                    equals: vec![KeyValue::Text("c1".into())],
                    page: PageRequest {
                        limit: 1,
                        after: None,
                    },
                },
            )
            .unwrap();
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.items[0].body["text"], "keep this");
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
