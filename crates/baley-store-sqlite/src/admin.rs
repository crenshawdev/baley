//! Owner operations exposed through the storage port.

use std::collections::BTreeMap;
use std::path::Path;

use baley_store::{
    Admin, AnchorCheck, Command, ExportReport, Hash, Health, PayloadRef, PayloadReference,
    ProjectId, PurgeReport, RebuildReport, Refusal, ScrubReport, StoreError, ViewsReport,
};
use rusqlite::{OptionalExtension, params};

use crate::store::{SqliteStore, sql};

impl Admin for SqliteStore {
    fn create_project(&self, project: &ProjectId, name: &str, at: &str) -> Result<(), StoreError> {
        self.write(|tx| {
            if tx
                .query_row(
                    "SELECT 1 FROM project WHERE project_id = ?1",
                    [&project.0],
                    |_| Ok(()),
                )
                .optional()
                .map_err(sql)?
                .is_some()
            {
                return Err(StoreError::Refused(Refusal::ProjectExists(project.clone())));
            }
            tx.execute(
                "INSERT INTO project (project_id, name, created_at) VALUES (?1, ?2, ?3)",
                params![project.0, name, at],
            )
            .map_err(sql)?;
            Ok(())
        })
    }

    fn projects(&self) -> Result<Vec<(ProjectId, String)>, StoreError> {
        self.snapshot(|conn| {
            let mut statement = conn
                .prepare("SELECT project_id, name FROM project ORDER BY project_id")
                .map_err(sql)?;
            let rows = statement
                .query_map([], |row| Ok((ProjectId(row.get(0)?), row.get(1)?)))
                .map_err(sql)?;
            rows.map(|row| row.map_err(sql)).collect()
        })
    }

    fn export(
        &self,
        project: &ProjectId,
        target: &Path,
        at: &str,
    ) -> Result<ExportReport, StoreError> {
        self.export_with(project, target, at, || {})
    }

    fn reduce(
        &self,
        command: &Command,
        reference: &PayloadReference,
    ) -> Result<PayloadRef, StoreError> {
        SqliteStore::reduce(self, command, reference)
    }

    fn purge(
        &self,
        command: &Command,
        hashes: &[Hash],
        reason: &str,
    ) -> Result<PurgeReport, StoreError> {
        SqliteStore::purge(self, command, hashes, reason)
    }

    fn scrub(&self) -> Result<ScrubReport, StoreError> {
        SqliteStore::scrub(self)
    }

    fn rebuild(&self, project: &ProjectId) -> Result<RebuildReport, StoreError> {
        SqliteStore::rebuild(self, project)
    }

    fn verify_views(&self, project: &ProjectId) -> Result<ViewsReport, StoreError> {
        SqliteStore::verify_views(self, project)
    }

    fn doctor(
        &self,
        at: &str,
        checks: &BTreeMap<ProjectId, AnchorCheck>,
    ) -> Result<Health, StoreError> {
        self.doctor_in(at, checks)
    }
}
