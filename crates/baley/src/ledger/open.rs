//! The schema understood by this CLI. Only the per-session server's open
//! runs the startup `quick_check`; every command-line open leaves it off.
//! Only the guard's open bounds its storage waits.
use std::num::NonZeroU32;
use std::time::Duration;

use baley_core::capture::{CaptureProjector, register_capture_events};
use baley_core::catalog::{ModelCatalogProjector, register_model_events};
use baley_core::checkout::{CheckoutProjector, register_checkout_events};
use baley_core::guard::{GuardPolicyProjector, GuardProjector, register_guard_events};
use baley_core::plan::{
    EvidenceMapProjector, PhasePlanProjector, PlanProjector, register_plan_events,
};
use baley_core::policy::recorded::{PolicyProjector, register_policy_events};
use baley_core::questions::{QuestionsProjector, register_question_events};
use baley_core::scope::{
    BacklogProjector, DraftProjector, PhaseDetailProjector, PhaseProjector, RoadmapProjector,
    register_scope_events,
};
use baley_core::truth::{TruthsProjector, register_truth_events};
use baley_core::work_order::{WorkOrderProjector, register_work_order_events};
use baley_core::{Registry, register_anchor_events, register_project_events};
use baley_store_sqlite::Options;

use crate::install::event::register_install_events;
use crate::install::view::InstallProjector;
use crate::providers::event::register_provider_events;
use crate::update::events::register_update_events;

/// Registers exactly the anchor types, `project.initialized`, the four
/// `models.*` types, `policy.effective`, `checkout.seen`, `capture.recorded`,
/// `guard.answered`, `guard.policy_recorded`, `install.recorded`,
/// `update.checked`, `update.failed`, `providers.acknowledged` and the eleven
/// scope types `scope.submitted`, `project.described`, `scope.approved`,
/// `story.declared`, `story.corrected`, `story.reassigned`,
/// `story.reprioritized`, `story.dropped`, `phase.declared`,
/// `phase.reordered` and `phase.withdrawn`, `work_order.issued` and
/// `work_order.failed`, the four question types `questions.opened`,
/// `question.answered`, `question.deferred` and `questions.abandoned`,
/// `story.refined`, and the six plan types `plan.approved`,
/// `plan.replaced`, `plan.continuation`, `plan.continuation_followed`,
/// `plan.check_issued` and `plan.checked` understood by the CLI, and
/// declares view set version 15 over twenty views, the seventeen of view
/// set 13 and `evidence_map`, `phase_plan` and `plan`: `backlog capture
/// checkout claim_scope draft evidence_map guard guard_policy install
/// model_catalog phase phase_detail phase_plan plan policy questions request
/// roadmap truths work_order`. View set 13 covered the `work_order` view's
/// version 2 and the `roadmap` view's version 3; view set 14 also covered
/// the `backlog` view's version 4; view set 15 covers the `phase_plan`
/// view's version 2 and the `work_order` view's version 3, which project
/// the check events. The install view holds only install records. Update
/// outcomes and provider acknowledgements are read from their streams and
/// have no projector. A project brought to view set 15 is read-only for
/// older binaries.
/// The startup `quick_check` is off for command-line callers.
pub(crate) fn options() -> Options {
    let mut registry = Registry::new();
    register_anchor_events(&mut registry).expect("unique anchor types");
    register_project_events(&mut registry).expect("unique project types");
    register_model_events(&mut registry).expect("unique model types");
    register_policy_events(&mut registry).expect("unique policy types");
    register_checkout_events(&mut registry).expect("unique checkout types");
    register_capture_events(&mut registry).expect("unique capture types");
    register_guard_events(&mut registry).expect("unique guard types");
    register_install_events(&mut registry).expect("unique install types");
    register_update_events(&mut registry).expect("unique update types");
    register_provider_events(&mut registry).expect("unique provider types");
    register_scope_events(&mut registry).expect("unique scope types");
    register_work_order_events(&mut registry).expect("unique work order types");
    register_question_events(&mut registry).expect("unique question types");
    register_truth_events(&mut registry).expect("unique truth types");
    register_plan_events(&mut registry).expect("unique plan types");
    Options {
        schema: Box::new(registry),
        projectors: vec![
            Box::new(ModelCatalogProjector::new()),
            Box::new(PolicyProjector::new()),
            Box::new(CheckoutProjector::new()),
            Box::new(CaptureProjector::new()),
            Box::new(GuardProjector::new()),
            Box::new(GuardPolicyProjector::new()),
            Box::new(InstallProjector::new()),
            Box::new(DraftProjector::new()),
            Box::new(RoadmapProjector::new()),
            Box::new(PhaseProjector::new()),
            Box::new(PhaseDetailProjector::new()),
            Box::new(BacklogProjector::new()),
            Box::new(WorkOrderProjector::new()),
            Box::new(QuestionsProjector::new()),
            Box::new(TruthsProjector::new()),
            Box::new(PhasePlanProjector::new()),
            Box::new(PlanProjector::new()),
            Box::new(EvidenceMapProjector::new()),
        ],
        view_set_version: NonZeroU32::new(15).expect("nonzero"),
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
/// version, and the startup `quick_check` stays off. The guard hook opens
/// its two short stores with it.
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
pub(crate) mod tests {
    //! Other modules' store tests open guard stores through `Fixed`, `guard`
    //! and `view_set_6`, as these tests do.

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
    pub(crate) fn view_set_6() -> Options {
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

    /// The options at view set 7 before update events were registered.
    /// A project stays writable here until a newer binary brings it forward.
    fn view_set_7() -> Options {
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        register_checkout_events(&mut registry).unwrap();
        register_capture_events(&mut registry).unwrap();
        register_guard_events(&mut registry).unwrap();
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
            view_set_version: NonZeroU32::new(7).unwrap(),
            ..Options::default()
        }
    }

    /// The options as they stood at view set 8, before the scope records.
    fn view_set_8() -> Options {
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        register_checkout_events(&mut registry).unwrap();
        register_capture_events(&mut registry).unwrap();
        register_guard_events(&mut registry).unwrap();
        register_install_events(&mut registry).unwrap();
        register_update_events(&mut registry).unwrap();
        register_provider_events(&mut registry).unwrap();
        Options {
            schema: Box::new(registry),
            projectors: vec![
                Box::new(ModelCatalogProjector::new()),
                Box::new(PolicyProjector::new()),
                Box::new(CheckoutProjector::new()),
                Box::new(CaptureProjector::new()),
                Box::new(GuardProjector::new()),
                Box::new(GuardPolicyProjector::new()),
                Box::new(InstallProjector::new()),
            ],
            view_set_version: NonZeroU32::new(8).unwrap(),
            ..Options::default()
        }
    }

    /// The options as they stood at view set 11, before the question
    /// records.
    fn view_set_11() -> Options {
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        register_checkout_events(&mut registry).unwrap();
        register_capture_events(&mut registry).unwrap();
        register_guard_events(&mut registry).unwrap();
        register_install_events(&mut registry).unwrap();
        register_update_events(&mut registry).unwrap();
        register_provider_events(&mut registry).unwrap();
        register_scope_events(&mut registry).unwrap();
        register_work_order_events(&mut registry).unwrap();
        Options {
            schema: Box::new(registry),
            projectors: vec![
                Box::new(ModelCatalogProjector::new()),
                Box::new(PolicyProjector::new()),
                Box::new(CheckoutProjector::new()),
                Box::new(CaptureProjector::new()),
                Box::new(GuardProjector::new()),
                Box::new(GuardPolicyProjector::new()),
                Box::new(InstallProjector::new()),
                Box::new(DraftProjector::new()),
                Box::new(RoadmapProjector::new()),
                Box::new(PhaseProjector::new()),
                Box::new(PhaseDetailProjector::new()),
                Box::new(BacklogProjector::new()),
                Box::new(WorkOrderProjector::new()),
            ],
            view_set_version: NonZeroU32::new(11).unwrap(),
            ..Options::default()
        }
    }

    /// The options as they stood at view set 12, before `story.refined` and
    /// the `truths` view.
    fn view_set_12() -> Options {
        let mut options = view_set_11();
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        register_checkout_events(&mut registry).unwrap();
        register_capture_events(&mut registry).unwrap();
        register_guard_events(&mut registry).unwrap();
        register_install_events(&mut registry).unwrap();
        register_update_events(&mut registry).unwrap();
        register_provider_events(&mut registry).unwrap();
        register_scope_events(&mut registry).unwrap();
        register_work_order_events(&mut registry).unwrap();
        register_question_events(&mut registry).unwrap();
        options.schema = Box::new(registry);
        options.projectors.push(Box::new(QuestionsProjector::new()));
        options.view_set_version = NonZeroU32::new(12).unwrap();
        options
    }

    /// The options as they stood at view set 13, before the plan events and
    /// the `phase_plan`, `plan` and `evidence_map` views.
    fn view_set_13() -> Options {
        let mut options = view_set_12();
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        register_checkout_events(&mut registry).unwrap();
        register_capture_events(&mut registry).unwrap();
        register_guard_events(&mut registry).unwrap();
        register_install_events(&mut registry).unwrap();
        register_update_events(&mut registry).unwrap();
        register_provider_events(&mut registry).unwrap();
        register_scope_events(&mut registry).unwrap();
        register_work_order_events(&mut registry).unwrap();
        register_question_events(&mut registry).unwrap();
        register_truth_events(&mut registry).unwrap();
        options.schema = Box::new(registry);
        options.projectors.push(Box::new(TruthsProjector::new()));
        options.view_set_version = NonZeroU32::new(13).unwrap();
        options
    }

    /// The options as they stood at view set 14, before the check events
    /// and the `phase_plan` and `work_order` views that project them.
    fn view_set_14() -> Options {
        use baley_core::plan::{
            PLAN_APPROVED, PLAN_CONTINUATION, PLAN_CONTINUATION_FOLLOWED, PLAN_REPLACED,
        };

        let mut options = view_set_13();
        let mut registry = Registry::new();
        register_anchor_events(&mut registry).unwrap();
        register_project_events(&mut registry).unwrap();
        register_model_events(&mut registry).unwrap();
        register_policy_events(&mut registry).unwrap();
        register_checkout_events(&mut registry).unwrap();
        register_capture_events(&mut registry).unwrap();
        register_guard_events(&mut registry).unwrap();
        register_install_events(&mut registry).unwrap();
        register_update_events(&mut registry).unwrap();
        register_provider_events(&mut registry).unwrap();
        register_scope_events(&mut registry).unwrap();
        register_work_order_events(&mut registry).unwrap();
        register_question_events(&mut registry).unwrap();
        register_truth_events(&mut registry).unwrap();
        // The four plan types of view set 14, without the check events.
        for name in [
            PLAN_APPROVED,
            PLAN_REPLACED,
            PLAN_CONTINUATION,
            PLAN_CONTINUATION_FOLLOWED,
        ] {
            registry.register(name, 1, []).unwrap();
        }
        options.schema = Box::new(registry);
        options.projectors.push(Box::new(PhasePlanProjector::new()));
        options.projectors.push(Box::new(PlanProjector::new()));
        options
            .projectors
            .push(Box::new(EvidenceMapProjector::new()));
        options.view_set_version = NonZeroU32::new(14).unwrap();
        options
    }

    /// The domain views of view set 13, sorted: the fourteen of view set 12
    /// and `truths`.
    const VIEW_SET_13: [&str; 15] = [
        "backlog",
        "capture",
        "checkout",
        "draft",
        "guard",
        "guard_policy",
        "install",
        "model_catalog",
        "phase",
        "phase_detail",
        "policy",
        "questions",
        "roadmap",
        "truths",
        "work_order",
    ];

    /// The domain views of view set 14, sorted: the fifteen of view set 13
    /// and `evidence_map`, `phase_plan` and `plan`.
    const VIEW_SET_14: [&str; 18] = [
        "backlog",
        "capture",
        "checkout",
        "draft",
        "evidence_map",
        "guard",
        "guard_policy",
        "install",
        "model_catalog",
        "phase",
        "phase_detail",
        "phase_plan",
        "plan",
        "policy",
        "questions",
        "roadmap",
        "truths",
        "work_order",
    ];

    /// The domain views of view set 15, sorted: the eighteen of view set 14,
    /// with the `phase_plan` view at version 2 and `work_order` at version 3.
    const VIEW_SET_15: [&str; 18] = [
        "backlog",
        "capture",
        "checkout",
        "draft",
        "evidence_map",
        "guard",
        "guard_policy",
        "install",
        "model_catalog",
        "phase",
        "phase_detail",
        "phase_plan",
        "plan",
        "policy",
        "questions",
        "roadmap",
        "truths",
        "work_order",
    ];

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
    pub(crate) struct Fixed;

    impl baley_store_sqlite::Timing for Fixed {
        fn now(&self) -> Duration {
            Duration::from_secs(1)
        }

        fn pause(&self, _duration: Duration) {}
    }

    /// The guard's options with 1.5 s of storage time on the fixed clock.
    pub(crate) fn guard() -> Options {
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

    #[test]
    fn an_install_record_unreadable_by_the_server_or_the_guard_is_caught() {
        for opened in [
            options(),
            server_options(),
            guard_options(Duration::from_millis(1_500)),
        ] {
            assert!(opened.schema.reads("install.recorded", 1));
            assert!(!opened.schema.reads("install.recorded", 2));
        }
    }

    // Update outcomes stay on their stream, outside the install view.
    #[test]
    fn the_update_events_left_unregistered_or_given_a_view_is_caught() {
        use crate::update::events::{UPDATE_CHECKED, UPDATE_EVENT_VERSION, UPDATE_FAILED};

        for opened in [
            options(),
            server_options(),
            guard_options(Duration::from_millis(1_500)),
        ] {
            for name in [UPDATE_CHECKED, UPDATE_FAILED] {
                assert!(opened.schema.reads(name, UPDATE_EVENT_VERSION), "{name}");
                assert!(
                    !opened.schema.reads(name, UPDATE_EVENT_VERSION + 1),
                    "{name}"
                );
            }
            assert_eq!(opened.view_set_version, NonZeroU32::new(15).unwrap());
            let mut names = view_names(&opened);
            names.sort();
            assert_eq!(names, VIEW_SET_15);
        }
    }

    // The acknowledgement stays on its stream, outside every view, so the view
    // set does not rise and an older binary keeps writing `user`.
    #[test]
    fn the_acknowledgement_event_left_unregistered_or_given_a_view_is_caught() {
        use crate::providers::event::{PROVIDERS_ACKNOWLEDGED, PROVIDERS_ACKNOWLEDGED_VERSION};

        for opened in [
            options(),
            server_options(),
            guard_options(Duration::from_millis(1_500)),
        ] {
            assert!(
                opened
                    .schema
                    .reads(PROVIDERS_ACKNOWLEDGED, PROVIDERS_ACKNOWLEDGED_VERSION)
            );
            assert!(
                !opened
                    .schema
                    .reads(PROVIDERS_ACKNOWLEDGED, PROVIDERS_ACKNOWLEDGED_VERSION + 1)
            );
            assert_eq!(opened.view_set_version, NonZeroU32::new(15).unwrap());
            let mut names = view_names(&opened);
            names.sort();
            assert_eq!(names, VIEW_SET_15);
        }
    }

    // Catches a change event the store would refuse at append, a scope view
    // left out, or the views raised without raising the set version.
    #[test]
    fn a_change_type_left_unregistered_or_the_view_set_left_at_9_is_caught() {
        for opened in [
            options(),
            server_options(),
            guard_options(Duration::from_millis(1_500)),
        ] {
            for name in [
                "phase.withdrawn",
                "story.corrected",
                "story.reassigned",
                "story.dropped",
                "story.reprioritized",
                "project.described",
                "scope.approved",
                "story.declared",
                "phase.declared",
                "phase.reordered",
            ] {
                assert!(opened.schema.reads(name, 1), "{name}");
                assert!(!opened.schema.reads(name, 2), "{name}");
            }
            assert!(opened.schema.reads("scope.submitted", 1));
            assert!(opened.schema.reads("scope.submitted", 2));
            assert!(!opened.schema.reads("scope.submitted", 3));
            assert_eq!(opened.view_set_version, NonZeroU32::new(15).unwrap());
            let mut names = view_names(&opened);
            names.sort();
            assert_eq!(names, VIEW_SET_15);
        }
    }

    // Catches a work order the store would refuse at append, the order view
    // left out, or the view added without raising the set version.
    #[test]
    fn a_work_order_type_left_unregistered_or_the_view_set_left_at_10_is_caught() {
        for opened in [
            options(),
            server_options(),
            guard_options(Duration::from_millis(1_500)),
        ] {
            for name in ["work_order.issued", "work_order.failed"] {
                assert!(opened.schema.reads(name, 1), "{name}");
                assert!(!opened.schema.reads(name, 2), "{name}");
            }
            assert_eq!(opened.view_set_version, NonZeroU32::new(15).unwrap());
            let mut names = view_names(&opened);
            names.sort();
            assert_eq!(names, VIEW_SET_15);
        }
    }

    // Catches a question record the store would refuse at append, or the
    // questions view added without raising the set version.
    #[test]
    fn a_question_type_left_unregistered_or_the_view_set_left_at_11_is_caught() {
        for opened in [
            options(),
            server_options(),
            guard_options(Duration::from_millis(1_500)),
        ] {
            for name in [
                "questions.opened",
                "question.answered",
                "question.deferred",
                "questions.abandoned",
            ] {
                assert!(opened.schema.reads(name, 1), "{name}");
                assert!(!opened.schema.reads(name, 2), "{name}");
            }
            assert_eq!(opened.view_set_version, NonZeroU32::new(15).unwrap());
            let mut names = view_names(&opened);
            names.sort();
            assert_eq!(names, VIEW_SET_15);
        }
    }

    // Catches a raise that fences the previous binary's ledger, or the
    // questions view not built forward for it.
    #[test]
    fn a_ledger_written_at_view_set_11_refused_or_left_without_the_questions_view_is_caught() {
        use baley_core::questions::questions_key;
        use baley_store::{Admin, ProjectId, Views};

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        let older = store(&home, T0, view_set_11()).unwrap();
        older.create_project(&project, "sample", T0).unwrap();
        drop(older);

        let reopened = store(&home, T1, options()).unwrap();
        let key = questions_key("5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
        assert_eq!(reopened.get(&project, "questions", &key), Ok(None));
    }

    // Catches a version-1 start draft upcast without the start's target, so
    // the start's newest-draft read misses a draft recorded before targets.
    #[test]
    fn a_start_draft_recorded_before_targets_filed_apart_from_new_ones_is_caught() {
        use baley_core::scope::{
            DRAFT_NEWEST_INDEX, DRAFT_VIEW, PROJECT_STREAM, SCOPE_SUBMITTED, draft_key,
        };
        use baley_store::{
            Actor, Command, CommandKind, Decision, Hash, IndexQuery, KeyValue, NewEvent, Observed,
            OutcomeKind, PageRequest, RequestId, RetentionClass, StreamName, Views,
        };
        use serde_json::json;

        use crate::scope::fixtures::{M_CANONICAL, M_DIGEST, project, store};

        let (_dir, store) = store();
        let command = Command {
            project: project(),
            kind: CommandKind("test.submit".into()),
            request_id: RequestId("4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f7a".into()),
            digest: Hash([0x11; 32]),
            scope: Vec::new(),
            policy_version: 0,
            recorded_at: "2026-10-13T09:00:01Z".into(),
            actor: Actor::Baley,
            caller: None,
        };
        store
            .transact(&command, &mut |tx| {
                let body = tx.put_payload(M_CANONICAL.as_bytes(), RetentionClass::Record)?;
                tx.append(NewEvent {
                    stream: StreamName(PROJECT_STREAM.into()),
                    type_name: SCOPE_SUBMITTED.into(),
                    type_version: 1,
                    git: None,
                    payload: json!({"digest": M_DIGEST, "kind": "start", "body": body.to_value()}),
                    attachments: vec![body],
                })?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();

        let digest = Hash::from_hex(M_DIGEST).unwrap();
        let document = store
            .get(&project(), DRAFT_VIEW, &draft_key(digest))
            .unwrap()
            .expect("the start's draft");
        assert_eq!(document.body["target"], "project");
        let query = IndexQuery {
            index: DRAFT_NEWEST_INDEX.into(),
            equals: vec![
                KeyValue::Text("start".into()),
                KeyValue::Text("project".into()),
            ],
            page: PageRequest {
                limit: 1,
                after: None,
            },
        };
        let page = store.find(&project(), DRAFT_VIEW, &query).unwrap();
        assert_eq!(page.items[0].body["digest"], M_DIGEST);
    }

    // Catches a raise that fences the previous binary's ledger, or scope
    // views not built forward for it.
    #[test]
    fn a_ledger_written_at_view_set_8_refused_or_left_without_the_scope_views_is_caught() {
        use baley_core::scope::{backlog_key, draft_key, phase_detail_key, phase_key, roadmap_key};
        use baley_store::{Admin, Hash, ProjectId, Views};

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        let older = store(&home, T0, view_set_8()).unwrap();
        older.create_project(&project, "sample", T0).unwrap();
        drop(older);

        let reopened = store(&home, T1, options()).unwrap();
        for (view, key) in [
            ("roadmap", roadmap_key()),
            ("draft", draft_key(Hash([0x5c; 32]))),
            ("phase", phase_key(1)),
            ("phase_detail", phase_detail_key(1)),
            ("backlog", backlog_key()),
        ] {
            assert_eq!(reopened.get(&project, view, &key), Ok(None), "{view}");
        }
    }

    /// One capture in `project`, recorded as the capture command does.
    fn record_capture(
        store: &baley_store_sqlite::SqliteStore,
        project: &baley_store::ProjectId,
        request: &str,
        id: &str,
        text: &str,
        at: &str,
    ) -> baley_store::Recorded {
        use baley_core::capture::{
            CAPTURE_RECORDED, CAPTURE_RECORDED_VERSION, CAPTURE_STREAM, CaptureKind, inline_payload,
        };
        use baley_store::{
            Actor, Command, CommandKind, Decision, Hash, NewEvent, Observed, OutcomeKind,
            RequestId, StreamName,
        };

        let command = Command {
            project: project.clone(),
            kind: CommandKind("capture.record".into()),
            request_id: RequestId(request.into()),
            digest: Hash([7; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: at.into(),
            actor: Actor::Baley,
            caller: None,
        };
        store
            .transact(&command, &mut |tx| {
                tx.append(NewEvent {
                    stream: StreamName(CAPTURE_STREAM.into()),
                    type_name: CAPTURE_RECORDED.into(),
                    type_version: CAPTURE_RECORDED_VERSION,
                    git: None,
                    payload: inline_payload(id, CaptureKind::Note, None, text),
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
            .unwrap()
    }

    /// One event in `user`, recorded under a command of the given kind.
    fn record_in_user(
        store: &baley_store_sqlite::SqliteStore,
        kind: &str,
        request: &str,
        event: baley_store::NewEvent,
        at: &str,
    ) {
        use baley_core::catalog::USER_PROJECT;
        use baley_store::{
            Actor, Command, CommandKind, Decision, Hash, Observed, OutcomeKind, ProjectId,
            RequestId,
        };

        let command = Command {
            project: ProjectId(USER_PROJECT.into()),
            kind: CommandKind(kind.into()),
            request_id: RequestId(request.into()),
            digest: Hash([9; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: at.into(),
            actor: Actor::Baley,
            caller: None,
        };
        let mut event = Some(event);
        store
            .transact(&command, &mut |tx| {
                if let Some(event) = event.take() {
                    tx.append(event)?;
                }
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: serde_json::json!({}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
    }

    // A newer view set fences only the project brought forward to it.
    #[test]
    fn an_older_baley_fenced_out_of_the_session_project_by_the_newer_binary_is_caught() {
        use crate::models;
        use crate::update::events::{CheckOutcome, ClaimRef, Facts, checked_event};
        use crate::update::version::Version;
        use baley_core::capture::{CAPTURE_ID_INDEX, CAPTURE_VIEW};
        use baley_core::catalog::USER_PROJECT;
        use baley_core::guard::{
            Answer, AnsweredFacts, GUARD_ANSWERED, GUARD_ANSWERED_VERSION, GUARD_STREAM,
            SettingsFact, answered_payload,
        };
        use baley_store::{
            Admin, IndexQuery, KeyValue, NewEvent, PageRequest, ProjectId, Recorded, Refusal,
            RequestId, StoreError, StreamName, Views,
        };

        const T2: &str = "2026-10-01T10:00:02Z";
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());

        // The older binary: a project with a capture, and a seeded `user`.
        let older = store(&home, T0, view_set_7()).unwrap();
        older.create_project(&project, "sample", T0).unwrap();
        let first = record_capture(
            &older,
            &project,
            "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e21",
            "c1",
            "first",
            T0,
        );
        assert!(matches!(first, Recorded::New { .. }), "{first:?}");
        models::create_user(&older, T0).unwrap();
        models::record_seed(&older, seed_request(), T0).unwrap();
        drop(older);

        // The newer binary records an update event, then a guard record, in `user`.
        let newer = store(
            &home,
            T1,
            Options {
                timing: std::sync::Arc::new(Fixed),
                ..options()
            },
        )
        .unwrap();
        let request = RequestId("00000000-0000-4000-8000-0000000000aa".into());
        let checked = checked_event(
            &Facts {
                installation: "/home/o/.local/bin/baley",
                day: "2026-10-01",
                claim: ClaimRef {
                    request_id: &request,
                    seq: 3,
                },
                at: T1,
                active_version: Some(Version::parse("0.1.0").unwrap()),
                staged_version: None,
            },
            CheckOutcome::Current,
        );
        record_in_user(
            &newer,
            "update.check",
            "00000000-0000-4000-8000-0000000000ab",
            checked,
            T1,
        );
        drop(newer);

        let guarded = store(&home, T1, guard()).unwrap();
        let payload = answered_payload(
            &AnsweredFacts {
                host: "claude-code",
                session: Some("s1"),
                call: "call-1",
                project_directory: None,
                cwd: "/work",
                tool: "Write",
                input_digest: "d",
                target: None,
                verb: None,
                branch: None,
                settings: SettingsFact::Absent,
            },
            &Answer::Deny("protected".into()),
        )
        .unwrap();
        record_in_user(
            &guarded,
            "guard.record",
            "00000000-0000-4000-8000-0000000000ac",
            NewEvent {
                stream: StreamName(GUARD_STREAM.into()),
                type_name: GUARD_ANSWERED.into(),
                type_version: GUARD_ANSWERED_VERSION,
                git: None,
                payload,
                attachments: vec![],
            },
            T1,
        );
        drop(guarded);

        // Only `user` was brought forward, so the session project stays writable.
        let older = store(&home, T2, view_set_7()).unwrap();
        let catalog = models::observe_hint_version(&older);
        assert!(
            matches!(
                &catalog,
                Err(StoreError::Refused(Refusal::ProjectReadOnly { project, .. }))
                    if *project == ProjectId(USER_PROJECT.into())
            ),
            "{catalog:?}"
        );
        let second = record_capture(
            &older,
            &project,
            "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e22",
            "c2",
            "second",
            T2,
        );
        assert!(matches!(second, Recorded::New { .. }), "{second:?}");
        let found = older
            .find(
                &project,
                CAPTURE_VIEW,
                &IndexQuery {
                    index: CAPTURE_ID_INDEX.into(),
                    equals: vec![KeyValue::Text("c2".into())],
                    page: PageRequest {
                        limit: 1,
                        after: None,
                    },
                },
            )
            .unwrap();
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.items[0].body["text"], "second");
    }

    // Catches the truths view declared without raising the view set, or left
    // out of the command line, server or guard open.
    #[test]
    fn a_view_set_left_at_12_or_missing_the_truths_view_is_caught() {
        for opened in [
            options(),
            server_options(),
            guard_options(Duration::from_millis(1_500)),
        ] {
            assert!(opened.schema.reads("story.refined", 1));
            assert!(!opened.schema.reads("story.refined", 2));
            assert_eq!(opened.view_set_version, NonZeroU32::new(15).unwrap());
            let mut names = view_names(&opened);
            names.sort();
            assert_eq!(names, VIEW_SET_15);
        }
    }

    /// Appends one `roadmap` event to `project` under a test command.
    fn append_roadmap(
        store: &baley_store_sqlite::SqliteStore,
        project: &baley_store::ProjectId,
        request: &str,
        type_name: &str,
        payload: serde_json::Value,
        at: &str,
    ) {
        use baley_core::scope::ROADMAP_STREAM;
        use baley_store::{
            Actor, Command, CommandKind, Decision, Hash, NewEvent, Observed, OutcomeKind, Recorded,
            RequestId, StreamName,
        };

        let command = Command {
            project: project.clone(),
            kind: CommandKind("test.append".into()),
            request_id: RequestId(request.into()),
            digest: Hash([0x33; 32]),
            scope: vec![],
            policy_version: 0,
            recorded_at: at.into(),
            actor: Actor::Owner,
            caller: None,
        };
        let mut payload = Some(payload);
        let recorded = store
            .transact(&command, &mut |tx| {
                if let Some(payload) = payload.take() {
                    tx.append(NewEvent {
                        stream: StreamName(ROADMAP_STREAM.into()),
                        type_name: type_name.into(),
                        type_version: 1,
                        git: None,
                        payload,
                        attachments: vec![],
                    })?;
                }
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
    }

    // Catches a raise that fences the previous binary's ledger, or the truths
    // view or the roadmap's truth count not built forward for it.
    #[test]
    fn a_ledger_at_view_set_12_not_reopening_with_the_truths_view_is_caught() {
        use baley_core::scope::{StoryStatus, roadmap_key, story_declared_payload};
        use baley_core::truth::{Truth, refined_payload, truths_key};
        use baley_store::{Admin, Hash, ProjectId, Views};

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        let older = store(&home, T0, view_set_12()).unwrap();
        older.create_project(&project, "sample", T0).unwrap();
        let declared = story_declared_payload(
            1,
            "Every step is recorded",
            StoryStatus::Active,
            None,
            Hash([0x5c; 32]),
        );
        append_roadmap(
            &older,
            &project,
            "4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f71",
            "story.declared",
            declared,
            T0,
        );
        drop(older);

        let reopened = store(&home, T1, options()).unwrap();
        let t1 = Truth {
            id: "t1".into(),
            form: "when".into(),
            trigger: "the owner approves a draft by a stale digest".into(),
            observer: "the owner".into(),
            verb: "is refused".into(),
            outcome: "stale-draft naming the first differing part".into(),
            kind: "literal".into(),
            human_verify: None,
        };
        let t2 = Truth {
            id: "t2".into(),
            form: "for-any".into(),
            trigger: "order the answers of a set are recorded in".into(),
            observer: "a ledger reader".into(),
            verb: "gets".into(),
            outcome: "the same round".into(),
            kind: "property".into(),
            human_verify: None,
        };
        let refined = refined_payload(
            1,
            "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
            Hash([0x6d; 32]),
            "Ada Owner",
            T1,
            &[(t1, 1), (t2, 1)],
        );
        append_roadmap(
            &reopened,
            &project,
            "4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f72",
            "story.refined",
            refined,
            T1,
        );

        let truths = reopened
            .get(&project, "truths", &truths_key(1))
            .unwrap()
            .expect("story 1's truths");
        assert_eq!(truths.body["story"], 1);
        assert_eq!(truths.body["truths"].as_array().map(Vec::len), Some(2));
        let roadmap = reopened
            .get(&project, "roadmap", &roadmap_key())
            .unwrap()
            .expect("the roadmap");
        assert_eq!(roadmap.body["stories"][0]["id"], 1);
        assert_eq!(roadmap.body["stories"][0]["truths"], 2);
    }

    // Catches a plan event the store would refuse at append, a planning view
    // left out so a plan write projects nothing, or the views added without
    // raising the set, which an older binary would misread.
    #[test]
    fn a_planning_view_or_event_left_unregistered_or_the_set_left_at_13_is_caught() {
        use baley_core::plan::{
            PlanApproval, evidence_map_key, phase_plan_key, phase_stream, plan_key,
        };
        use baley_store::{
            Actor, Admin, Command, CommandKind, Decision, Hash, NewEvent, Observed, OutcomeKind,
            ProjectId, RequestId, RetentionClass, StreamName, Views,
        };

        let opened = options();
        assert_eq!(opened.view_set_version, NonZeroU32::new(15).unwrap());
        let mut names = view_names(&opened);
        names.sort();
        assert_eq!(names, VIEW_SET_15);
        for name in [
            "plan.approved",
            "plan.replaced",
            "plan.continuation",
            "plan.continuation_followed",
        ] {
            assert!(opened.schema.reads(name, 1), "{name}");
            assert!(!opened.schema.reads(name, 2), "{name}");
        }

        let older_options = view_set_13();
        let mut older_names = view_names(&older_options);
        older_names.sort();
        assert_eq!(older_names, VIEW_SET_13);
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        let older = store(&home, T0, older_options).unwrap();
        older.create_project(&project, "sample", T0).unwrap();
        drop(older);

        let reopened = store(&home, T1, options()).unwrap();
        let command = Command {
            project: project.clone(),
            kind: CommandKind("test.approve".into()),
            request_id: RequestId("4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f73".into()),
            digest: Hash([0x33; 32]),
            scope: Vec::new(),
            policy_version: 0,
            recorded_at: T1.into(),
            actor: Actor::Owner,
            caller: None,
        };
        reopened
            .transact(&command, &mut |tx| {
                let body = tx.put_payload(b"the plan's body", RetentionClass::Record)?;
                // Serving no story, so the backlog has none to refuse.
                let approval = PlanApproval {
                    phase: 1,
                    plan: 1,
                    digest: Hash([0x5c; 32]),
                    stories: &[],
                    size: 1,
                    suite: None,
                    question_set: "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
                    evidence_map: &[],
                    body: &body,
                    owner: "Ada Owner",
                    at: T1,
                };
                tx.append(NewEvent {
                    stream: StreamName(phase_stream(1)),
                    type_name: "plan.approved".into(),
                    type_version: 1,
                    git: None,
                    payload: approval.payload(),
                    attachments: vec![body],
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
        for (view, key) in [
            ("phase_plan", phase_plan_key(1)),
            ("plan", plan_key(1, 1)),
            ("evidence_map", evidence_map_key(1, 1)),
        ] {
            let held = reopened.get(&project, view, &key).unwrap();
            assert!(held.is_some(), "{view} holds no document for plan 1");
        }
    }

    // Catches a check event the store would refuse at append, or the set
    // left at 14, which an older binary would misread.
    #[test]
    fn a_check_event_left_unregistered_or_the_set_left_at_14_is_caught() {
        use baley_core::plan::{CheckResult, phase_plan_key, phase_stream};
        use baley_core::policy::RecheckScope;
        use baley_core::work_order::work_order_key;
        use baley_store::{
            Actor, Admin, Command, CommandKind, Decision, Hash, NewEvent, Observed, OutcomeKind,
            ProjectId, RequestId, StreamName, Views,
        };
        use serde_json::{Value, json};

        const W1: &str = "2c3d4e5f-6071-4c8d-9eaf-1a2b3c4d5e6f";
        const V: &str = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

        let opened = options();
        assert_eq!(opened.view_set_version, NonZeroU32::new(15).unwrap());
        let mut names = view_names(&opened);
        names.sort();
        assert_eq!(names, VIEW_SET_15);
        for name in ["plan.checked", "plan.check_issued"] {
            assert!(opened.schema.reads(name, 1), "{name}");
            assert!(!opened.schema.reads(name, 2), "{name}");
        }

        let older_options = view_set_14();
        assert!(!older_options.schema.reads("plan.checked", 1));
        let mut older_names = view_names(&older_options);
        older_names.sort();
        assert_eq!(older_names, VIEW_SET_14);
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        let older = store(&home, T0, older_options).unwrap();
        older.create_project(&project, "sample", T0).unwrap();
        drop(older);

        let reopened = store(&home, T1, options()).unwrap();
        // Appends `events` on phase 1's stream in one transaction and gives
        // the sequence of the last.
        let append = |request: &str, events: Vec<(&str, Value)>| {
            let command = Command {
                project: project.clone(),
                kind: CommandKind("test.check".into()),
                request_id: RequestId(request.into()),
                digest: Hash([0x33; 32]),
                scope: Vec::new(),
                policy_version: 0,
                recorded_at: T1.into(),
                actor: Actor::Baley,
                caller: None,
            };
            let mut last = 0;
            reopened
                .transact(&command, &mut |tx| {
                    for (type_name, payload) in &events {
                        last = tx.append(NewEvent {
                            stream: StreamName(phase_stream(1)),
                            type_name: (*type_name).into(),
                            type_version: 1,
                            git: None,
                            payload: payload.clone(),
                            attachments: vec![],
                        })?;
                    }
                    Ok(Decision {
                        kind: OutcomeKind::Done,
                        answer: json!({}),
                        sensitive: false,
                        observed: Observed::default(),
                        git: None,
                    })
                })
                .unwrap();
            last
        };
        let checked = |revision: Option<&str>| {
            CheckResult {
                phase: 1,
                plan_digest: Hash([0x5c; 32]),
                findings: &[],
                work_order: W1,
                scope: RecheckScope::Full,
                previous_check: None,
                truth_versions: &[],
                material: &[],
                scope_policy_version: 0,
                revision,
            }
            .payload()
        };
        let order = json!({"id": W1, "request": W1, "parent": null, "role": "checker",
            "subject": "phase/1", "attempt": 1, "route": {"rung": "low"},
            "agent": "bal-checker-low", "instructions": [], "inputs": []});
        append(
            "4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f74",
            vec![
                ("work_order.issued", order),
                ("plan.checked", checked(None)),
            ],
        );

        let phase = |reopened: &baley_store_sqlite::SqliteStore| {
            reopened
                .get(&project, "phase_plan", &phase_plan_key(1))
                .unwrap()
                .expect("phase 1's plan state")
                .body
        };
        assert_eq!(phase(&reopened)["checks"], json!([]));
        let held = reopened
            .get(&project, "work_order", &work_order_key(W1))
            .unwrap()
            .expect("the checker order");
        assert_eq!(held.body["state"], "answered");

        let seq = append(
            "4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f75",
            vec![("plan.checked", checked(Some(V)))],
        );
        assert_eq!(
            phase(&reopened)["checks"],
            json!([{"revision": V, "first": {"seq": seq, "plan_digest": "5c".repeat(32)},
                "recheck": null}])
        );
    }
}
