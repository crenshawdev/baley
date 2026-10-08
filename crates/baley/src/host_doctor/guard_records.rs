//! Whether the guard's per-user records were behind this binary when the
//! doctor started.
//!
//! The guard keeps its decisions in the per-user project `user`. When that
//! project's views are behind this binary's, the guard answers needs-rebuild
//! and records nothing until `baley rebuild user` runs. The store's doctor
//! reads each project's raw view stamps before it verifies the views, and
//! verifying brings an old project current, so the stamps in the [`Health`]
//! it returned show the state the doctor started with.
//!
//! The judgement mirrors the store adapter's rule for a project's live views
//! as far as the raw stamps show it. A stored view this binary no longer
//! declares is not in the stamps, so a project that holds only such a view
//! is not seen here.

use baley_core::catalog::USER_PROJECT;
use baley_store::Health;

/// What the `user` project's views are to this binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judged {
    /// The ledger holds no `user` project yet.
    NoUserProject,
    /// The views are this binary's, or the project is empty and unstamped.
    Current,
    /// The views were built by an older binary: the guard needs a rebuild.
    Behind,
    /// A newer binary built the views. A rebuild would take them backward, so
    /// there is no rebuild finding; the doctor's other lines report it.
    Newer,
    /// The stamps or the chain could not be read, which the doctor's other
    /// lines report.
    NotJudged,
}

/// Judges the `user` project's raw views from the store's health, and no
/// other project's.
pub fn judge(health: &Health) -> Judged {
    let Some(user) = health
        .projects
        .iter()
        .find(|project| project.project.0 == USER_PROJECT)
    else {
        return Judged::NoUserProject;
    };
    let Ok(raw) = &user.raw_views else {
        return Judged::NotJudged;
    };
    let (live_set, binary_set) = raw.view_set;
    if live_set.is_some_and(|set| set > binary_set)
        || raw.views.iter().any(|view| {
            view.live_version
                .is_some_and(|live| live > view.binary_version)
        })
    {
        return Judged::Newer;
    }
    let stamped = live_set.is_some() || raw.views.iter().any(|view| view.live_version.is_some());
    let behind = if stamped {
        live_set.is_some_and(|set| set != binary_set)
            || raw
                .views
                .iter()
                .any(|view| view.live_version != Some(view.binary_version))
    } else {
        // Nothing is stamped. An empty project needs no stamps from a rebuild,
        // the guard stamps it itself, and one with events needs a rebuild.
        match &user.verify {
            Ok(report) => report.chain.head.is_some(),
            Err(_) => return Judged::NotJudged,
        }
    };
    if behind {
        Judged::Behind
    } else {
        Judged::Current
    }
}

#[cfg(test)]
mod tests {
    use baley_store::StoreError;

    use super::super::fixtures::{health_with, project_health, raw_views};
    use super::*;

    fn behind_views() -> baley_store::RawViewHealth {
        raw_views(Some(6), 7, &[(Some(1), 1), (Some(3), 3)])
    }

    fn current_views() -> baley_store::RawViewHealth {
        raw_views(Some(7), 7, &[(Some(1), 1), (Some(3), 3)])
    }

    #[test]
    fn a_project_other_than_user_judged_for_the_guard_is_caught() {
        let other_behind = health_with(vec![
            project_health("P", Ok(behind_views()), Some(5)),
            project_health(USER_PROJECT, Ok(current_views()), Some(5)),
        ]);
        assert_eq!(judge(&other_behind), Judged::Current);

        let user_behind = health_with(vec![
            project_health("P", Ok(current_views()), Some(5)),
            project_health(USER_PROJECT, Ok(behind_views()), Some(5)),
        ]);
        assert_eq!(judge(&user_behind), Judged::Behind);

        let no_user = health_with(vec![project_health("P", Ok(behind_views()), Some(5))]);
        assert_eq!(judge(&no_user), Judged::NoUserProject);
    }

    #[test]
    fn an_empty_user_project_called_behind_or_a_used_one_without_stamps_called_current_is_caught() {
        let unstamped = || raw_views(None, 7, &[(None, 1), (None, 3)]);

        let empty = health_with(vec![project_health(USER_PROJECT, Ok(unstamped()), None)]);
        assert_eq!(judge(&empty), Judged::Current);

        let used = health_with(vec![project_health(USER_PROJECT, Ok(unstamped()), Some(3))]);
        assert_eq!(judge(&used), Judged::Behind);

        let mut failed = project_health(USER_PROJECT, Ok(unstamped()), Some(3));
        failed.verify = Err(StoreError::Busy);
        assert_eq!(judge(&health_with(vec![failed])), Judged::NotJudged);
    }

    #[test]
    fn newer_user_views_called_needs_rebuild_is_caught() {
        let newer_view = raw_views(Some(7), 7, &[(Some(2), 1), (Some(3), 3)]);
        let newer_set = raw_views(Some(8), 7, &[(Some(1), 1), (Some(3), 3)]);
        for raw in [newer_view, newer_set] {
            let health = health_with(vec![project_health(USER_PROJECT, Ok(raw), Some(5))]);
            assert_eq!(judge(&health), Judged::Newer);
        }
    }
}
