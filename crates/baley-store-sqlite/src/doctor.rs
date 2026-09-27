//! Store health from a supplied time and caller-supplied remote checks.

use std::collections::BTreeMap;
use std::fs;

use baley_store::{
    AnchorCheck, Building, ClaimCounts, ClaimState, Health, Ledger, ProjectHealth, ProjectId,
    Refusal, StoreError, UnanchoredAge, UtcInstant, ViewHealth, claim_state, unanchored_warning,
};
use rusqlite::OptionalExtension;

use crate::store::{SqliteStore, sql};
use crate::view::live_views;

fn bad(value: &str) -> StoreError {
    StoreError::Unavailable(format!("malformed {value}"))
}
fn io(error: std::io::Error) -> StoreError {
    StoreError::Unavailable(error.to_string())
}

impl SqliteStore {
    /// Reports store-wide facts and each project's independent findings.
    pub(crate) fn doctor_in(
        &self,
        at: &str,
        checks: &BTreeMap<ProjectId, AnchorCheck>,
    ) -> Result<Health, StoreError> {
        UtcInstant::parse(at).map_err(|_| bad("doctor time"))?;
        let projects = <Self as baley_store::Admin>::projects(self)?;
        for (project, _) in &projects {
            if !checks.contains_key(project) {
                return Err(StoreError::Refused(Refusal::MissingAnchorCheck(
                    project.clone(),
                )));
            }
        }
        for project in checks.keys() {
            if !projects.iter().any(|(known, _)| known == project) {
                return Err(StoreError::Refused(Refusal::UnknownProject(
                    project.clone(),
                )));
            }
        }
        let (epoch, scrub_pending, integrity) = self.snapshot(|conn| {
            let epoch: u32 = conn
                .query_row(
                    "SELECT value FROM schema_meta WHERE key = 'epoch'",
                    [],
                    |row| row.get(0),
                )
                .map_err(sql)?;
            let scrub_pending = conn
                .query_row(
                    "SELECT value FROM schema_meta WHERE key = 'scrub_pending'",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sql)?;
            let mut statement = conn.prepare("PRAGMA integrity_check").map_err(sql)?;
            let integrity = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(sql)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql)?;
            Ok((epoch, scrub_pending, integrity))
        })?;
        let database_bytes = fs::metadata(self.home.join("baley.db")).map_err(io)?.len();
        let log_bytes = match fs::metadata(self.home.join("baley.db-wal")) {
            Ok(file) => file.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(io(error)),
        };
        let mut health = Vec::with_capacity(projects.len());
        for (project, _) in projects {
            let check = checks.get(&project).expect("checked above").clone();
            let (views, view_set, building) = self.snapshot(|conn| {
                let live = live_views(conn, &project)?;
                let views = self.views().tables().map(|table| {
                    let version = live.stamps.get(&table.spec().name).map(|stamp| u32::try_from(stamp.projector).map_err(|_| bad("view version"))).transpose()?;
                    Ok(ViewHealth { view: table.spec().name.clone(), live_version: version, binary_version: table.spec().version })
                }).collect::<Result<Vec<_>, StoreError>>()?;
                let set = live.stamps.values().next().map(|stamp| u32::try_from(stamp.set).map_err(|_| bad("view set version"))).transpose()?;
                let marker: Option<(i64, i64)> = conn.query_row(
                    "SELECT building_gen, building_applied_seq FROM project_gen WHERE project_id = ?1", [&project.0],
                    |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?))
                ).optional().map_err(sql)?.and_then(|(generation, applied)| generation.zip(applied));
                let building = marker.map(|(generation, applied_seq)| {
                    let generation = u64::try_from(generation).map_err(|_| bad("building generation"))?;
                    let applied_seq = u64::try_from(applied_seq).map_err(|_| bad("building sequence"))?;
                    Ok(Building { generation, applied_seq, lag: live.head.saturating_sub(applied_seq) })
                }).transpose()?;
                Ok((views, (set, self.views().version().get()), building))
            })?;
            let witness = match &check {
                AnchorCheck::Remote(anchor) => Some(anchor),
                _ => None,
            };
            let verify = self.verify(&project, witness);
            let remote_absent_local_row = if matches!(check, AnchorCheck::RemoteAbsent) {
                verify
                    .as_ref()
                    .ok()
                    .and_then(|report| report.stored_anchor.clone())
            } else {
                None
            };
            let unanchored = match (&check, &verify) {
                (AnchorCheck::RemoteUnreachable | AnchorCheck::RemoteMalformed(_), _)
                | (_, Err(_)) => UnanchoredAge::Unchecked,
                (_, Ok(report)) => match &report.chain.age_unanchored_since {
                    None => UnanchoredAge::None,
                    Some(since) => match unanchored_warning(since, at) {
                        Ok(warning) => UnanchoredAge::Since {
                            since: since.clone(),
                            warning,
                        },
                        Err(_) => UnanchoredAge::Unchecked,
                    },
                },
            };
            let views_check = self.verify_views(&project);
            let claims = self.open_claims(&project).and_then(|claims| {
                let mut counts = ClaimCounts {
                    active: 0,
                    interrupted: 0,
                    awaiting_owner: 0,
                };
                for claim in claims {
                    match claim_state(&claim, at).map_err(|_| bad("claim time"))? {
                        ClaimState::Active => counts.active += 1,
                        ClaimState::Interrupted => counts.interrupted += 1,
                        ClaimState::AwaitingOwner => counts.awaiting_owner += 1,
                    }
                }
                Ok(counts)
            });
            health.push(ProjectHealth {
                project,
                check,
                verify,
                remote_absent_local_row,
                unanchored,
                views,
                view_set,
                building,
                views_check,
                claims,
            });
        }
        Ok(Health {
            epoch,
            scrub_pending,
            integrity,
            database_bytes,
            log_bytes,
            projects: health,
        })
    }
}
