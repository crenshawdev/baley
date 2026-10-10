//! Planned stub writes through digest-checked replacement. The plan owns
//! the permission to write and interprets the returned observations.

use std::fs;

use crate::replace::{self, Failure};

use super::plan::Plan;

/// Applies writes in order with default folder modes, stopping at the first
/// failure. A failed sync is returned too, since the rename already happened.
pub(crate) fn run(plan: &Plan) -> Vec<Result<(), Failure>> {
    let mut results = Vec::new();
    for write in &plan.writes {
        let result = fs::create_dir_all(
            write
                .path
                .parent()
                .expect("an absolute stub path has a parent"),
        )
        .map_err(|cause| Failure::Unchanged {
            path: write.path.clone(),
            cause,
        })
        .and_then(|()| replace::replace(&write.path, &write.bytes, write.read_digest.as_deref()));
        let stopped = result.is_err();
        results.push(result);
        if stopped {
            break;
        }
    }
    results
}
