//! Remote names are explicit until project discovery arrives. The per-user
//! project `user` is checked locally unless the owner names a remote for it.
use super::CliRefusal;
use baley_core::catalog::USER_PROJECT;
use baley_store::ProjectId;
use std::collections::BTreeMap;

/// Matches a configured remote as one complete line.
pub(super) fn configured(stdout: &str, name: &str) -> bool {
    stdout.lines().any(|line| line == name)
}
/// Requires exactly one remote choice for every ledger project but `user`,
/// which is local only when not named.
pub(super) fn anchor_plan(
    projects: &[(ProjectId, String)],
    remotes: &[(String, String)],
    local_only: &[String],
) -> Result<BTreeMap<ProjectId, Option<String>>, CliRefusal> {
    let mut plan = BTreeMap::new();
    for (id, remote) in remotes
        .iter()
        .map(|(p, r)| (p, Some(r.clone())))
        .chain(local_only.iter().map(|p| (p, None)))
    {
        let project = ProjectId(id.clone());
        if !projects.iter().any(|(p, _)| p == &project) {
            return Err(CliRefusal(format!("project {id} is not in the ledger")));
        }
        if plan.insert(project, remote).is_some() {
            return Err(CliRefusal(format!("project {id} is named more than once")));
        }
    }
    // Placed after the named projects, so an explicit choice for it stands.
    let user = ProjectId(USER_PROJECT.into());
    if projects.iter().any(|(p, _)| *p == user) {
        plan.entry(user).or_insert(None);
    }
    let missing: Vec<_> = projects
        .iter()
        .filter(|(p, _)| !plan.contains_key(p))
        .map(|(p, n)| {
            format!(
                "doctor needs --remote {}=REMOTE or --local-only {} for project {} ({n})",
                p.0, p.0, p.0
            )
        })
        .collect();
    if !missing.is_empty() {
        return Err(CliRefusal(missing.join("\nbaley: ")));
    }
    Ok(plan)
}
