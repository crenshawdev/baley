//! The policy step (design 0003, CFG-R8, CFG-R9; build-2-plan decision 18).
//! Before every command that appends to a project's chain from a checkout,
//! it re-reads both settings files and records `policy.effective` when the
//! policy changed. It prints nothing.

mod read;
mod record;

use baley_core::catalog::{MODEL_CATALOG_VIEW, USER_PROJECT, read_state, state_key};
use baley_core::policy::recorded::RecordedPolicy;
use baley_store::{Admin, Ledger, ProjectId, RequestId, StoreError, Views};

pub use read::{Reads, build, gather};
pub use record::{RECORD_COMMAND, record};

/// Runs the policy step for `project` with the checkout and policy in
/// `recorded`, and returns the version in force for the caller's own
/// command. It runs as its own command, before the caller's, and prints
/// nothing.
///
/// `baley init`, `purge` and `config set` call it now. `anchor` and
/// `acknowledge-restore` (T12), and the server and guard per request
/// (Build 3), follow.
pub fn step(
    store: &(impl Admin + Views + Ledger),
    project: &ProjectId,
    recorded: &RecordedPolicy,
    request_id: RequestId,
    at: &str,
) -> Result<u64, StoreError> {
    let catalog_version = observe_catalog_version(store)?;
    record(store, project, recorded, catalog_version, request_id, at)
}

/// The catalog version, read outside any transaction: 0 when the `user`
/// project is absent. The project list is read first, so an absent project
/// is not read as an error, and the step never creates or seeds `user`.
fn observe_catalog_version(store: &(impl Admin + Views)) -> Result<u64, StoreError> {
    let user = ProjectId(USER_PROJECT.into());
    if !store.projects()?.iter().any(|(known, _)| *known == user) {
        return Ok(0);
    }
    let state = store.get(&user, MODEL_CATALOG_VIEW, &state_key())?;
    Ok(read_state(state.as_ref().map(|document| &document.body)).catalog_version)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use baley_core::catalog::{MODELS_SEEDED, MODELS_STREAM};
    use baley_core::policy::recorded::{POLICY_EFFECTIVE, recorded_policy};
    use baley_core::policy::{Schema, effective_policy};
    use baley_store::{Event, PageRequest, StreamName};
    use baley_store_sqlite::SqliteStore;
    use serde_json::json;

    use super::*;

    const T0: &str = "2026-10-01T10:00:00Z";
    const T1: &str = "2026-10-01T10:00:01Z";
    const T2: &str = "2026-10-01T10:00:02Z";
    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";

    fn request(n: u8) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-0000000000{n:02}"))
    }

    fn store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
        store
            .create_project(&ProjectId(ID.into()), "sample", T0)
            .unwrap();
        (dir, store)
    }

    fn recorded() -> RecordedPolicy {
        let policy = effective_policy(Schema::standard(), None, None, None).unwrap();
        recorded_policy(Path::new("/w/r"), &policy).unwrap()
    }

    fn events(store: &SqliteStore, project: &str, stream: &str) -> Vec<Event> {
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        let project = ProjectId(project.into());
        store
            .stream(&project, &StreamName(stream.into()), 1, page)
            .unwrap()
            .items
    }

    fn recorded_catalog_version(store: &SqliteStore) -> serde_json::Value {
        let recorded: Vec<_> = events(store, ID, "project")
            .into_iter()
            .filter(|event| event.type_name == POLICY_EFFECTIVE)
            .collect();
        assert_eq!(recorded.len(), 1);
        recorded[0].payload["catalog_version"].clone()
    }

    #[test]
    fn the_step_does_not_create_or_seed_user_and_records_catalog_version_0_without_it() {
        let (_dir, store) = store();

        step(&store, &ProjectId(ID.into()), &recorded(), request(1), T1).unwrap();

        assert_eq!(recorded_catalog_version(&store), json!(0));
        let projects = store.projects().unwrap();
        assert!(
            projects.iter().all(|(id, _)| id.0 != USER_PROJECT),
            "{projects:?}"
        );
    }

    #[test]
    fn the_step_does_not_record_catalog_version_0_once_the_catalog_is_seeded() {
        let (_dir, store) = store();
        assert!(crate::models::seed(&store, request(1), T1).unwrap());
        let seeded: Vec<_> = events(&store, USER_PROJECT, MODELS_STREAM)
            .into_iter()
            .filter(|event| event.type_name == MODELS_SEEDED)
            .collect();
        assert_eq!(seeded.len(), 1);

        step(&store, &ProjectId(ID.into()), &recorded(), request(2), T2).unwrap();

        assert_eq!(recorded_catalog_version(&store), json!(seeded[0].seq));
    }
}
