//! Generations: a project's views brought to this binary's on first use,
//! rebuilt in yielding batches and flipped in one transaction, old rows
//! cleaned up, and view verification against a scratch generation (design
//! 0001, Views and projectors, Figure 7; EVD-R10, R19).
//!
//! Every view row carries a generation, and `project_gen.live_gen` names the
//! one readers and commands use. Commands write only the live generation. A
//! rebuild replays the project's events into a new generation, marked in
//! `project_gen.building_gen` with the last event it applied, each batch its
//! own short turn on the writer queue followed by a pause as long as that
//! turn. The last turn applies the tail through the head it reads and moves
//! `live_gen` in the same transaction, which switches every view at once.
//! A verification replays into a scratch generation the same way and never
//! flips. A rebuild or verification holds the maintenance lock throughout;
//! commands do not take it. `view_gen` stamps each generation with the
//! projector versions and view set version that built it, and a project's
//! live stamps decide whether this binary may use its views.

use std::cmp::Ordering;
use std::sync::MutexGuard;
use std::time::Duration;

use baley_store::{
    Actor, DocKey, Event, GitFacts, Hash, ProjectId, RebuildReport, Refusal, RequestId, StoreError,
    ViewsReport,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params};

use crate::payload::sql_int;
use crate::queue::{BATCH_EVENTS, BATCH_ROWS, BATCH_TIME, Turn};
use crate::store::{SqliteStore, lock, sql};
use crate::transact::stored_head;
use crate::view::{Fence, Staging, ViewTable, catalog_tables, fold, live_views, write_staged};

/// The refusal for a project this binary must not write or, for its views,
/// read.
pub(crate) fn read_only(project: &ProjectId, reason: String) -> StoreError {
    StoreError::Refused(Refusal::ProjectReadOnly {
        project: project.clone(),
        reason,
    })
}

impl SqliteStore {
    /// Runs `f` on the project's live generation in the read snapshot that
    /// found it built by this binary's views, so a flip between the check
    /// and the read cannot put an older binary's empty table in front of
    /// the caller. A newer generation refuses; an older or unstamped one is
    /// brought forward and checked again in a fresh snapshot.
    pub(crate) fn read_current<T>(
        &self,
        project: &ProjectId,
        mut f: impl FnMut(&Connection, i64) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        loop {
            let read = self.snapshot(|conn| {
                let live = live_views(conn, project)?;
                match self.views().judge_set(&live) {
                    Fence::Current(generation) => f(conn, generation).map(Some),
                    Fence::Newer(reason) => Err(read_only(project, reason)),
                    Fence::Unstamped | Fence::Behind => Ok(None),
                }
            })?;
            match read {
                Some(value) => return Ok(value),
                None => self.bring_current(project)?,
            }
        }
    }

    /// Brings the project's live views to this binary's before their first
    /// use, waiting for the maintenance lock as long as it takes. A caller
    /// that waited while another process did the work finds them current
    /// and does nothing.
    pub(crate) fn bring_current(&self, project: &ProjectId) -> Result<(), StoreError> {
        let _hold = self.hold_maintenance()?;
        self.bring_current_held(project)
    }

    /// `bring_current` for a caller that holds the maintenance lock. A
    /// rebuild that flipped but could not clean up still leaves the views
    /// current; the next rebuild removes what is left.
    fn bring_current_held(&self, project: &ProjectId) -> Result<(), StoreError> {
        let live = self.snapshot(|conn| live_views(conn, project))?;
        match self.views().judge_set(&live) {
            Fence::Current(_) => Ok(()),
            Fence::Newer(reason) => Err(read_only(project, reason)),
            Fence::Unstamped => self.stamp_empty(project),
            Fence::Behind => {
                let finished = self.start(project, None, Purpose::Rebuild)?.finish()?;
                // The flip committed, so the views are current.
                let _left_for_the_next_rebuild = finished.cleanup;
                Ok(())
            }
        }
    }

    /// Stamps generation 0 of a project that has no generation and no
    /// events: nothing to replay, so no rebuild. Rechecked under the queue;
    /// a project changed meanwhile is left for the caller's next check.
    fn stamp_empty(&self, project: &ProjectId) -> Result<(), StoreError> {
        self.write(|tx| {
            let live = live_views(tx, project)?;
            if live.generation.is_none() && live.head == 0 {
                tx.execute(
                    "INSERT INTO project_gen (project_id, live_gen) VALUES (?1, 0)",
                    [&project.0],
                )
                .map_err(sql)?;
                self.stamp(tx, project, 0)?;
            }
            Ok(())
        })
    }

    /// One `view_gen` row for every registered view, empty ones included,
    /// at its projector version and this binary's view set version.
    fn stamp(
        &self,
        tx: &rusqlite::Transaction<'_>,
        project: &ProjectId,
        generation: i64,
    ) -> Result<(), StoreError> {
        let set = self.views().version().get();
        for table in self.views().tables() {
            tx.execute(
                "INSERT INTO view_gen (project_id, gen, view, projector_version, view_set_version)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    project.0,
                    generation,
                    table.spec().name,
                    table.spec().version,
                    set
                ],
            )
            .map_err(sql)?;
        }
        Ok(())
    }

    /// Rebuilds the project's views into a new generation and makes it
    /// live; see `Admin::rebuild`. When removing the old generation fails
    /// after the flip, the error says the new generation is live.
    pub fn rebuild(&self, project: &ProjectId) -> Result<RebuildReport, StoreError> {
        let finished = self.start_rebuild(project)?.finish()?;
        finished.cleanup.map_err(|error| {
            StoreError::Unavailable(format!(
                "project {} now reads generation {}; removing the old generation failed and is left for the next rebuild: {error}",
                project.0, finished.report.generation
            ))
        })?;
        Ok(finished.report)
    }

    /// A rebuild of the project's views that the caller steps through:
    /// takes the maintenance lock and holds it until the session is
    /// finished or dropped. Dropping it unfinished leaves its generation
    /// and marker for the next rebuild to remove, as a crash would.
    pub(crate) fn start_rebuild(&self, project: &ProjectId) -> Result<Rebuild<'_>, StoreError> {
        let hold = self.hold_maintenance()?;
        self.start(project, Some(hold), Purpose::Rebuild)
    }

    /// Replays the project's events into a scratch generation and compares
    /// it with the live one at one head; see `Admin::verify_views`. Views
    /// behind this binary's are rebuilt first; the scratch rows are removed
    /// whatever happens after they were started.
    pub fn verify_views(&self, project: &ProjectId) -> Result<ViewsReport, StoreError> {
        let _hold = self.hold_maintenance()?;
        self.bring_current_held(project)?;
        let mut scratch = self.start(project, None, Purpose::Verify)?;
        let report = scratch.compare();
        let cleanup = self.remove_generation(project, scratch.generation, true);
        let report = report?;
        cleanup?;
        Ok(report)
    }

    /// Sets up a new generation under the maintenance lock the caller holds
    /// or hands over. A rebuild first removes an unfinished generation left
    /// by a crash and every generation that is neither live nor being
    /// built; a verification refuses while one is unfinished.
    fn start<'s>(
        &'s self,
        project: &ProjectId,
        hold: Option<Turn<'s>>,
        purpose: Purpose,
    ) -> Result<Rebuild<'s>, StoreError> {
        let orphan = self.batch(
            |tx, _| {
                let live = live_views(tx, project)?;
                match self.views().judge_set(&live) {
                    Fence::Newer(reason) => return Err(read_only(project, reason)),
                    Fence::Current(_) => {}
                    Fence::Unstamped | Fence::Behind if purpose == Purpose::Rebuild => {}
                    Fence::Unstamped | Fence::Behind => {
                        return Err(StoreError::Unavailable(format!(
                            "project {}'s live views are not this binary's",
                            project.0
                        )));
                    }
                }
                let head = stored_head(tx, project)?;
                self.check_readable(tx, project, head.as_ref())?;
                marker(tx, project).map(|marker| marker.map(|(generation, _)| generation))
            },
            Ok,
        )?;
        match (orphan, purpose) {
            (Some(orphan), Purpose::Rebuild) => self.remove_generation(project, orphan, true)?,
            (Some(orphan), Purpose::Verify) => {
                return Err(StoreError::Unavailable(format!(
                    "project {} holds generation {orphan} of a rebuild or verification that never finished; rebuild the project first",
                    project.0
                )));
            }
            (None, _) => {}
        }
        if purpose == Purpose::Rebuild {
            self.sweep(project)?;
        }
        let generation = self.batch(
            |tx, _| {
                let generation = next_generation(tx, project)?;
                tx.execute(
                    "INSERT INTO project_gen (project_id, live_gen, building_gen, building_applied_seq)
                     VALUES (?1, 0, ?2, 0)
                     ON CONFLICT (project_id) DO UPDATE
                        SET building_gen = ?2, building_applied_seq = 0",
                    params![project.0, generation],
                )
                .map_err(sql)?;
                self.stamp(tx, project, generation)?;
                Ok(generation)
            },
            Ok,
        )?;
        Ok(Rebuild {
            store: self,
            project: project.clone(),
            _hold: hold,
            generation,
            events: 0,
        })
    }

    /// Removes every generation of the project that is neither live nor
    /// being built, one generation at a time in yielding batches, rows
    /// before their `view_gen` stamps. Generations are found by keyed seeks
    /// on `view_gen` and on every catalog table, so a generation that was
    /// never stamped, or whose view this binary no longer registers, is
    /// found too.
    fn sweep(&self, project: &ProjectId) -> Result<(), StoreError> {
        let mut after = -1;
        loop {
            let step = self.batch(
                |tx, acquired| {
                    let (live, building) = generations(tx, project)?;
                    let mut from = after;
                    let found = loop {
                        match next_after(tx, project, from)? {
                            Some(generation)
                                if generation == live || Some(generation) == building =>
                            {
                                from = generation;
                            }
                            other => break other,
                        }
                    };
                    let Some(generation) = found else {
                        return Ok(None);
                    };
                    let done = self.remove_rows(tx, project, generation, acquired, false)?;
                    Ok(Some((generation, done)))
                },
                Ok,
            )?;
            match step {
                None => return Ok(()),
                Some((generation, true)) => after = generation,
                Some((_, false)) => {}
            }
        }
    }

    /// Removes one generation in yielding batches, then its stamps, and
    /// with `marker` the building marker naming it, in the last batch. A
    /// failure leaves the marker for the next rebuild.
    fn remove_generation(
        &self,
        project: &ProjectId,
        generation: i64,
        marker: bool,
    ) -> Result<(), StoreError> {
        while !self.batch(
            |tx, acquired| self.remove_rows(tx, project, generation, acquired, marker),
            Ok,
        )? {}
        Ok(())
    }

    /// One batch of removing a generation: at most `BATCH_ROWS` document
    /// rows across every catalog table, stopping at the next table once the
    /// batch has removed a row and held the queue `BATCH_TIME`. `true` once
    /// no row is left and the stamps, and with `marker` the marker, are
    /// gone too.
    fn remove_rows(
        &self,
        tx: &rusqlite::Transaction<'_>,
        project: &ProjectId,
        generation: i64,
        acquired: Duration,
        marker: bool,
    ) -> Result<bool, StoreError> {
        let mut left = BATCH_ROWS;
        for table in catalog_tables(tx)? {
            if left == 0 || (left < BATCH_ROWS && self.elapsed(acquired) >= BATCH_TIME) {
                return Ok(false);
            }
            left -= delete_rows(tx, &table, project, generation, left)?;
        }
        // A table that filled the batch may hold more.
        if left == 0 {
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM view_gen WHERE project_id = ?1 AND gen = ?2",
            params![project.0, generation],
        )
        .map_err(sql)?;
        if marker {
            tx.execute(
                "UPDATE project_gen SET building_gen = NULL, building_applied_seq = NULL
                  WHERE project_id = ?1 AND building_gen = ?2",
                params![project.0, generation],
            )
            .map_err(sql)?;
        }
        Ok(true)
    }

    /// How long the current batch has held the queue.
    fn elapsed(&self, acquired: Duration) -> Duration {
        self.timing().now().saturating_sub(acquired)
    }

    /// Replays, inside `tx`, the events after the marker into `generation`:
    /// at most `BATCH_EVENTS`, and none past `BATCH_TIME` once one is
    /// applied. Each event is read as its projection copy and folded
    /// through the projectors against the documents of `generation`, never
    /// the live one; each changed document is written once, stamped with
    /// the last event that changed it, and the marker moves to the last
    /// event applied. Stored events are never changed or skipped.
    fn replay(
        &self,
        tx: &rusqlite::Transaction<'_>,
        project: &ProjectId,
        generation: i64,
        acquired: Duration,
    ) -> Result<Batch, StoreError> {
        let applied = match marker(tx, project)? {
            Some((building, applied)) if building == generation => applied,
            _ => {
                return Err(StoreError::Unavailable(format!(
                    "project {} is no longer building generation {generation}",
                    project.0
                )));
            }
        };
        let head = stored_head(tx, project)?.map_or(0, |head| head.seq);
        let mut staged = Staging::new();
        let mut last = applied;
        let mut count = 0u64;
        let mut read = 0usize;
        let mut stopped = false;
        {
            let mut statement = tx.prepare(EVENTS_AFTER).map_err(sql)?;
            let mut rows = statement
                .query(params![project.0, sql_int(applied)?, BATCH_EVENTS as i64])
                .map_err(sql)?;
            while let Some(row) = rows.next().map_err(sql)? {
                if count > 0 && self.elapsed(acquired) >= BATCH_TIME {
                    stopped = true;
                    break;
                }
                read += 1;
                let event = stored_event(project, row).map_err(sql)?;
                if event.seq != last + 1 {
                    return Err(missing(project, last + 1));
                }
                let copy = self.projection_copy(&event).map_err(|reason| {
                    read_only(
                        project,
                        format!("event {} cannot be read: {reason}", event.seq),
                    )
                })?;
                fold(self, tx, project, generation, &copy, &mut staged)?;
                last = event.seq;
                count += 1;
            }
        }
        if !stopped && read < BATCH_EVENTS && last != head {
            return Err(missing(project, last + 1));
        }
        write_staged(tx, self, project, generation, &staged)?;
        if count > 0 {
            tx.execute(
                "UPDATE project_gen SET building_applied_seq = ?1 WHERE project_id = ?2",
                params![sql_int(last)?, project.0],
            )
            .map_err(sql)?;
        }
        Ok(Batch {
            applied: count,
            caught_up: last == head,
        })
    }
}

/// Why a generation is being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// To become live.
    Rebuild,
    /// To be compared with the live one and removed.
    Verify,
}

/// What one replay batch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Batch {
    /// Events applied in the batch.
    pub(crate) applied: u64,
    /// Whether the generation now holds every event up to the head the
    /// batch read.
    pub(crate) caught_up: bool,
}

/// A finished rebuild: what it made live, and whether removing the old
/// generation after the flip succeeded.
pub(crate) struct Finished {
    pub(crate) report: RebuildReport,
    pub(crate) cleanup: Result<(), StoreError>,
}

/// A rebuild or verification in progress, holding the maintenance lock or
/// running under a caller that holds it.
pub(crate) struct Rebuild<'s> {
    store: &'s SqliteStore,
    project: ProjectId,
    _hold: Option<Turn<'s>>,
    generation: i64,
    /// Events replayed into the generation so far.
    events: u64,
}

impl Rebuild<'_> {
    /// Applies one replay batch as its own queue turn, and pauses after it
    /// as long as the turn held the queue.
    pub(crate) fn apply_one_batch(&mut self) -> Result<Batch, StoreError> {
        let (store, project, generation) = (self.store, &self.project, self.generation);
        let batch = store.batch(
            |tx, acquired| store.replay(tx, project, generation, acquired),
            Ok,
        )?;
        self.events += batch.applied;
        Ok(batch)
    }

    /// The final catch-up: each turn reads the head, applies the tail
    /// after the marker that commands added meanwhile and, only if the
    /// whole tail fit one bounded batch, makes the generation live for
    /// every view in that same transaction by moving `live_gen` and
    /// clearing the marker. A tail too long for one batch commits its
    /// progress and tries again after the pause. Then removes every other
    /// generation before returning.
    pub(crate) fn finish(mut self) -> Result<Finished, StoreError> {
        let (store, project, generation) = (self.store, self.project.clone(), self.generation);
        loop {
            let batch = store.batch(
                |tx, acquired| {
                    let batch = store.replay(tx, &project, generation, acquired)?;
                    if batch.caught_up {
                        let flipped = tx
                            .execute(
                                "UPDATE project_gen
                                    SET live_gen = building_gen, building_gen = NULL,
                                        building_applied_seq = NULL
                                  WHERE project_id = ?1 AND building_gen = ?2",
                                params![project.0, generation],
                            )
                            .map_err(sql)?;
                        if flipped != 1 {
                            return Err(StoreError::Unavailable(format!(
                                "project {} is no longer building generation {generation}",
                                project.0
                            )));
                        }
                    }
                    Ok(batch)
                },
                Ok,
            )?;
            self.events += batch.applied;
            if batch.caught_up {
                break;
            }
        }
        let report = RebuildReport {
            generation: u64::try_from(generation).map_err(|_| {
                StoreError::Unavailable(format!("a stored generation of {generation}"))
            })?,
            events: self.events,
        };
        let cleanup = store.sweep(&project);
        Ok(Finished { report, cleanup })
    }

    /// Replays until the scratch generation reaches the head, and in the
    /// turn that gets there pins a read snapshot before the queue is
    /// released, so scratch and live are compared at that one head however
    /// many commands follow. Every registered view's rows are compared, key
    /// by key: missing, extra or unequal in any stored column, the document
    /// text as its bytes, so a live document that is not canonical differs.
    fn compare(&mut self) -> Result<ViewsReport, StoreError> {
        while !self.apply_one_batch()?.caught_up {}
        let (store, project, generation) = (self.store, self.project.clone(), self.generation);
        let pinned = loop {
            let (batch, pinned) = store.batch(
                |tx, acquired| store.replay(tx, &project, generation, acquired),
                |batch| {
                    let pinned = if batch.caught_up {
                        Some(Pinned::begin(store, &project)?)
                    } else {
                        None
                    };
                    Ok((batch, pinned))
                },
            )?;
            self.events += batch.applied;
            if let Some(pinned) = pinned {
                break pinned;
            }
        };
        let mut differing = Vec::new();
        for table in store.views().tables() {
            for key in differing_keys(&pinned.conn, table, &project, generation, pinned.live)? {
                differing.push((table.spec().name.clone(), key));
            }
        }
        Ok(ViewsReport {
            checked_seq: pinned.head,
            differing,
        })
    }
}

/// A read transaction on the store's read connection, begun while the
/// writer queue was held, with the head and live generation it saw. Ended
/// when dropped.
struct Pinned<'s> {
    conn: MutexGuard<'s, Connection>,
    head: u64,
    live: i64,
}

impl<'s> Pinned<'s> {
    fn begin(store: &'s SqliteStore, project: &ProjectId) -> Result<Self, StoreError> {
        let conn = lock(&store.reader);
        conn.execute_batch("BEGIN DEFERRED").map_err(sql)?;
        let mut pinned = Self {
            conn,
            head: 0,
            live: 0,
        };
        // The first read fixes the snapshot.
        let (head, live) = pinned
            .conn
            .query_row(
                "SELECT p.head_seq, g.live_gen FROM project p
                   JOIN project_gen g ON g.project_id = p.project_id
                  WHERE p.project_id = ?1",
                [&project.0],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(sql)?;
        pinned.head = u64::try_from(head)
            .map_err(|_| StoreError::Unavailable(format!("a stored head of {head}")))?;
        pinned.live = live;
        Ok(pinned)
    }
}

impl Drop for Pinned<'_> {
    fn drop(&mut self) {
        // A read transaction has nothing to keep; ending it cannot lose data.
        let _ = self.conn.execute_batch("ROLLBACK");
    }
}

/// The keys whose rows differ between two generations of one view, in key
/// order. Both sides are read in key order and merged, so neither is held
/// in memory.
fn differing_keys(
    conn: &Connection,
    table: &ViewTable,
    project: &ProjectId,
    scratch: i64,
    live: i64,
) -> Result<Vec<DocKey>, StoreError> {
    let sql_text = table.rows_sql();
    let mut rebuilt = conn.prepare(&sql_text).map_err(sql)?;
    let mut stored = conn.prepare(&sql_text).map_err(sql)?;
    let mut rebuilt = rebuilt
        .query_map(params![project.0, scratch], |row| table.stored_row(row))
        .map_err(sql)?;
    let mut stored = stored
        .query_map(params![project.0, live], |row| table.stored_row(row))
        .map_err(sql)?;
    let mut differing = Vec::new();
    let mut left = next(&mut rebuilt)?;
    let mut right = next(&mut stored)?;
    loop {
        match (&left, &right) {
            (None, None) => return Ok(differing),
            (Some((key, _)), None) => {
                differing.push(key.clone());
                left = next(&mut rebuilt)?;
            }
            (None, Some((key, _))) => {
                differing.push(key.clone());
                right = next(&mut stored)?;
            }
            (Some((a, a_values)), Some((b, b_values))) => match a.cmp(b) {
                Ordering::Less => {
                    differing.push(a.clone());
                    left = next(&mut rebuilt)?;
                }
                Ordering::Greater => {
                    differing.push(b.clone());
                    right = next(&mut stored)?;
                }
                Ordering::Equal => {
                    if a_values != b_values {
                        differing.push(a.clone());
                    }
                    left = next(&mut rebuilt)?;
                    right = next(&mut stored)?;
                }
            },
        }
    }
}

/// The next row of one side of a comparison.
fn next(
    rows: &mut dyn Iterator<Item = rusqlite::Result<(DocKey, Vec<SqlValue>)>>,
) -> Result<Option<(DocKey, Vec<SqlValue>)>, StoreError> {
    rows.next().transpose().map_err(sql)
}

/// The project's events after a sequence, in order, bounded in SQL.
const EVENTS_AFTER: &str = "SELECT seq, stream, stream_version, type, type_version, actor,
        recorded_at, request_id, git_commit, git_tree, git_checkout, policy_version,
        payload_json, prev_hash, hash
   FROM event WHERE project_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3";

/// One row of `EVENTS_AFTER` as the stored event.
fn stored_event(project: &ProjectId, row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    let bad = |at: usize, what: String| {
        rusqlite::Error::FromSqlConversionFailure(at, rusqlite::types::Type::Text, what.into())
    };
    let unsigned = |at: usize| -> rusqlite::Result<u64> {
        let value: i64 = row.get(at)?;
        u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(at, value))
    };
    let hash = |at: usize, bytes: Vec<u8>| -> rusqlite::Result<Hash> {
        bytes
            .try_into()
            .map(Hash)
            .map_err(|_| bad(at, "a stored hash that is not 32 bytes".into()))
    };
    let git = match (
        row.get::<_, Option<String>>(8)?,
        row.get::<_, Option<String>>(9)?,
        row.get::<_, Option<String>>(10)?,
    ) {
        (Some(commit), Some(tree), Some(checkout)) => Some(GitFacts {
            commit,
            tree,
            checkout,
        }),
        (None, None, None) => None,
        _ => return Err(bad(8, "git facts stored in part".into())),
    };
    let actor: String = row.get(5)?;
    let payload: String = row.get(12)?;
    Ok(Event {
        project_id: project.clone(),
        seq: unsigned(0)?,
        stream: row.get(1)?,
        stream_version: unsigned(2)?,
        type_name: row.get(3)?,
        type_version: row.get(4)?,
        actor: Actor::parse(&actor).map_err(|error| bad(5, format!("{error:?}")))?,
        recorded_at: row.get(6)?,
        request_id: RequestId(row.get(7)?),
        git,
        policy_version: unsigned(11)?,
        payload: serde_json::from_str(&payload).map_err(|error| bad(12, error.to_string()))?,
        prev_hash: row
            .get::<_, Option<Vec<u8>>>(13)?
            .map(|bytes| hash(13, bytes))
            .transpose()?,
        hash: hash(14, row.get(14)?)?,
    })
}

fn missing(project: &ProjectId, seq: u64) -> StoreError {
    StoreError::Unavailable(format!(
        "event {seq} of project {} is missing from its chain",
        project.0
    ))
}

/// The generation being built and the last event applied to it.
fn marker(conn: &Connection, project: &ProjectId) -> Result<Option<(i64, u64)>, StoreError> {
    let row = conn
        .query_row(
            "SELECT building_gen, building_applied_seq FROM project_gen WHERE project_id = ?1",
            [&project.0],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
        )
        .optional()
        .map_err(sql)?;
    match row {
        Some((Some(generation), Some(applied))) => {
            let applied = u64::try_from(applied).map_err(|_| {
                StoreError::Unavailable(format!("a stored applied sequence of {applied}"))
            })?;
            Ok(Some((generation, applied)))
        }
        _ => Ok(None),
    }
}

/// The live generation, 0 before the project has one, and the one being
/// built, if any.
fn generations(conn: &Connection, project: &ProjectId) -> Result<(i64, Option<i64>), StoreError> {
    Ok(conn
        .query_row(
            "SELECT live_gen, building_gen FROM project_gen WHERE project_id = ?1",
            [&project.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or((0, None)))
}

/// The lowest generation above `after` that the project has stamps or rows
/// in: one keyed seek on `view_gen` and one on each catalog table.
fn next_after(
    conn: &Connection,
    project: &ProjectId,
    after: i64,
) -> Result<Option<i64>, StoreError> {
    let mut found: Option<i64> = conn
        .query_row(
            "SELECT gen FROM view_gen WHERE project_id = ?1 AND gen > ?2 ORDER BY gen LIMIT 1",
            params![project.0, after],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    for table in catalog_tables(conn)? {
        let generation: Option<i64> = conn
            .query_row(
                &format!(
                    "SELECT generation FROM {table}
                      WHERE project_id = ?1 AND generation > ?2 ORDER BY generation LIMIT 1"
                ),
                params![project.0, after],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        found = match (found, generation) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    Ok(found)
}

/// The generation a new build takes: above the live one and every one
/// still stored, so no generation a cursor was issued under is used again.
fn next_generation(conn: &Connection, project: &ProjectId) -> Result<i64, StoreError> {
    let (live, _) = generations(conn, project)?;
    let mut highest = live;
    let stamped: Option<i64> = conn
        .query_row(
            "SELECT gen FROM view_gen WHERE project_id = ?1 ORDER BY gen DESC LIMIT 1",
            [&project.0],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    highest = highest.max(stamped.unwrap_or(0));
    for table in catalog_tables(conn)? {
        let stored: Option<i64> = conn
            .query_row(
                &format!(
                    "SELECT generation FROM {table}
                      WHERE project_id = ?1 ORDER BY generation DESC LIMIT 1"
                ),
                [&project.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        highest = highest.max(stored.unwrap_or(0));
    }
    highest
        .checked_add(1)
        .ok_or_else(|| StoreError::Unavailable("no generation number is left".into()))
}

/// Deletes at most `limit` of the project's rows of `generation` from one
/// view table, by primary key, and says how many went.
fn delete_rows(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    project: &ProjectId,
    generation: i64,
    limit: usize,
) -> Result<usize, StoreError> {
    let mut statement = tx
        .prepare("SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk")
        .map_err(sql)?;
    let key: Vec<String> = statement
        .query_map([table], |row| row.get(0))
        .map_err(sql)?
        .collect::<rusqlite::Result<_>>()
        .map_err(sql)?;
    if key.is_empty() {
        return Err(StoreError::Unavailable(format!(
            "the view table {table} has no primary key"
        )));
    }
    let key = key.join(", ");
    let limit =
        i64::try_from(limit).map_err(|_| StoreError::Unavailable("a batch too large".into()))?;
    tx.execute(
        &format!(
            "DELETE FROM {table} WHERE ({key}) IN
               (SELECT {key} FROM {table} WHERE project_id = ?1 AND generation = ?2 LIMIT ?3)"
        ),
        params![project.0, generation, limit],
    )
    .map_err(sql)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::num::NonZeroU32;
    use std::path::Path;
    use std::sync::Arc;

    use baley_store::{
        Change, Command, CommandKind, Decision, DocKey, EventSchema, FieldKind, FieldSpec,
        IndexField, IndexSpec, KeyValue, NewEvent, Observed, Order, OutcomeKind, PayloadReference,
        Projector, ProjectorError, Recorded, RetentionClass, StreamName, ViewSpec, Views,
    };
    use rusqlite::Connection;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::queue::scripted::Scripted;
    use crate::store::Options;

    const AT: &str = "2026-09-25T18:00:00Z";
    const PROJECT: &str = "7f0c2a4e-8d1b-4c3a-9e5f-2b6d8a1c4e70";
    const ITEM: &str = "fixture.item";
    const QUIET: &str = "fixture.quiet";
    /// The item a second store's failing projector cannot apply.
    const TAIL: i64 = 99;

    /// Reads `fixture.item` at versions 1 and 2 and `fixture.quiet` at 1.
    /// Version 1 of an item called its state `status`; version 2 calls it
    /// `state`, and projection upcasts a version-1 payload to that.
    struct Schema;

    impl EventSchema for Schema {
        fn reads(&self, type_name: &str, version: u32) -> bool {
            (type_name == ITEM && (1..=2).contains(&version))
                || (type_name == QUIET && version == 1)
        }

        fn projection_payload(&self, event: &Event) -> Result<(u32, Value), String> {
            match (event.type_name.as_str(), event.type_version) {
                (ITEM, 1) => {
                    let mut payload = event.payload.clone();
                    let object = payload.as_object_mut().ok_or("not an object")?;
                    let state = object.remove("status").ok_or("no status")?;
                    object.insert("state".into(), state);
                    Ok((2, payload))
                }
                (ITEM, 2) | (QUIET, 1) => Ok((event.type_version, event.payload.clone())),
                _ => Err("unknown".into()),
            }
        }
    }

    fn id_key() -> Vec<FieldSpec> {
        vec![FieldSpec {
            name: "id".into(),
            kind: FieldKind::Integer,
        }]
    }

    /// `item`: each item's latest state, indexed by state. Applies only the
    /// current version 2 of `fixture.item`, so an event handed over without
    /// its upcast fails.
    struct Item(ViewSpec);

    fn item(version: u32) -> Box<dyn Projector> {
        Box::new(Item(ViewSpec {
            name: "item".into(),
            version,
            key: id_key(),
            indexes: vec![IndexSpec {
                name: "by_state".into(),
                fields: vec![IndexField {
                    name: "state".into(),
                    kind: FieldKind::Text,
                    order: Order::Ascending,
                }],
            }],
            page_bound: 10,
        }))
    }

    impl Projector for Item {
        fn spec(&self) -> &ViewSpec {
            &self.0
        }

        fn handles(&self) -> &[&str] {
            &[ITEM]
        }

        fn keys(&self, event: &Event) -> Vec<DocKey> {
            vec![DocKey(vec![KeyValue::Integer(
                event.payload["id"].as_i64().unwrap_or(0),
            )])]
        }

        fn apply(
            &self,
            event: &Event,
            _documents: &[(DocKey, Value)],
        ) -> Result<Vec<Change>, ProjectorError> {
            if event.type_version != 2 {
                return Err(ProjectorError(format!(
                    "{ITEM} version {} is not current",
                    event.type_version
                )));
            }
            Ok(vec![Change::Put {
                key: self.keys(event).remove(0),
                body: json!({"id": event.payload["id"], "state": event.payload["state"]}),
            }])
        }
    }

    /// `tally`: how many events each item has had, counted from the stored
    /// document, so a replay that reads the wrong generation counts wrong.
    /// With `fail_on`, it refuses that item, as a same-spec projector of a
    /// second store over the same home.
    struct Tally {
        spec: ViewSpec,
        fail_on: Option<i64>,
    }

    fn tally(fail_on: Option<i64>) -> Box<dyn Projector> {
        Box::new(Tally {
            spec: ViewSpec {
                name: "tally".into(),
                version: 1,
                key: id_key(),
                indexes: Vec::new(),
                page_bound: 10,
            },
            fail_on,
        })
    }

    impl Projector for Tally {
        fn spec(&self) -> &ViewSpec {
            &self.spec
        }

        fn handles(&self) -> &[&str] {
            &[ITEM]
        }

        fn keys(&self, event: &Event) -> Vec<DocKey> {
            vec![DocKey(vec![KeyValue::Integer(
                event.payload["id"].as_i64().unwrap_or(0),
            )])]
        }

        fn apply(
            &self,
            event: &Event,
            documents: &[(DocKey, Value)],
        ) -> Result<Vec<Change>, ProjectorError> {
            let id = event.payload["id"].as_i64().unwrap_or(0);
            if self.fail_on == Some(id) {
                return Err(ProjectorError(format!("item {id} is refused")));
            }
            let seen = documents
                .first()
                .and_then(|(_, body)| body["seen"].as_i64())
                .unwrap_or(0);
            Ok(vec![Change::Put {
                key: self.keys(event).remove(0),
                body: json!({"id": id, "seen": seen + 1}),
            }])
        }
    }

    /// `quiet`: a view no fixture event feeds, so it stays empty.
    struct Quiet(ViewSpec);

    fn quiet() -> Box<dyn Projector> {
        Box::new(Quiet(ViewSpec {
            name: "quiet".into(),
            version: 1,
            key: id_key(),
            indexes: Vec::new(),
            page_bound: 10,
        }))
    }

    impl Projector for Quiet {
        fn spec(&self) -> &ViewSpec {
            &self.0
        }

        fn handles(&self) -> &[&str] {
            &[QUIET]
        }

        fn keys(&self, _event: &Event) -> Vec<DocKey> {
            Vec::new()
        }

        fn apply(
            &self,
            _event: &Event,
            _documents: &[(DocKey, Value)],
        ) -> Result<Vec<Change>, ProjectorError> {
            Ok(Vec::new())
        }
    }

    fn project() -> ProjectId {
        ProjectId(PROJECT.into())
    }

    fn set(version: u32) -> NonZeroU32 {
        NonZeroU32::new(version).expect("positive")
    }

    fn open_as(
        home: &Path,
        projectors: Vec<Box<dyn Projector>>,
        view_set_version: u32,
        timing: Arc<Scripted>,
    ) -> Result<SqliteStore, StoreError> {
        SqliteStore::open(
            home,
            AT,
            Options {
                projectors,
                schema: Box::new(Schema),
                view_set_version: set(view_set_version),
                timing,
                ..Options::default()
            },
        )
    }

    /// The two fixture views, `item` at version 2 and `tally`, as set 2.
    fn open_with(home: &Path, timing: Arc<Scripted>) -> SqliteStore {
        open_as(home, vec![item(2), tally(None)], 2, timing).expect("open")
    }

    /// A store over a new home, holding the empty project.
    fn created(home: &Path, timing: Arc<Scripted>) -> SqliteStore {
        let store = open_with(home, timing);
        store
            .write(|tx| {
                tx.execute(
                    "INSERT INTO project (project_id, name, created_at) VALUES (?1, 'fixture', ?2)",
                    params![PROJECT, AT],
                )
                .map_err(sql)?;
                Ok(())
            })
            .expect("project");
        store
    }

    fn command(kind: &str, request: &str) -> Command {
        Command {
            project: project(),
            kind: CommandKind(kind.into()),
            request_id: RequestId(request.into()),
            digest: Hash([1; 32]),
            policy_version: 1,
            recorded_at: AT.into(),
            actor: Actor::Owner,
        }
    }

    fn done(answer: Value, sensitive: bool) -> Decision {
        Decision {
            kind: OutcomeKind::Done,
            answer,
            sensitive,
            observed: Observed::default(),
            git: None,
        }
    }

    fn event(type_version: u32, payload: Value) -> NewEvent {
        NewEvent {
            stream: StreamName("fixture".into()),
            type_name: ITEM.into(),
            type_version,
            git: None,
            payload,
            attachments: Vec::new(),
        }
    }

    /// Records the items, each as a version-2 `fixture.item`, under a
    /// fresh `fixture.add` request that answers `"ok"`.
    fn record(
        store: &SqliteStore,
        request: &str,
        items: &[(i64, &str)],
    ) -> Result<Recorded, StoreError> {
        store.transact(&command("fixture.add", request), &mut |tx| {
            for (id, state) in items {
                tx.append(event(2, json!({"id": id, "state": state})))?;
            }
            Ok(done(json!("ok"), false))
        })
    }

    /// The fixture chain: 1 item 1 open, 2 r1 completed, 3 item 2 open,
    /// 4 item 1 done, 5 r2 completed.
    fn fixture(store: &SqliteStore) {
        record(store, "r1", &[(1, "open")]).expect("r1");
        record(store, "r2", &[(2, "open"), (1, "done")]).expect("r2");
    }

    fn id(id: i64) -> DocKey {
        DocKey(vec![KeyValue::Integer(id)])
    }

    fn request(kind: &str, request: &str) -> DocKey {
        DocKey(vec![
            KeyValue::Text(kind.into()),
            KeyValue::Text(request.into()),
        ])
    }

    /// A document's body and the event that produced it, read through the
    /// port.
    fn doc(store: &SqliteStore, view: &str, key: &DocKey) -> Option<(Value, u64)> {
        store
            .get(&project(), view, key)
            .expect("get")
            .map(|document| (document.body, document.produced_seq))
    }

    fn completed(request: &str) -> Value {
        json!({"kind": "fixture.add", "request_id": request, "digest": "01".repeat(32),
               "outcome": "done", "answer": {"inline": "ok"}})
    }

    /// The fixture chain's documents, written by hand from its events:
    /// (view, key, body, produced_seq).
    fn expected() -> Vec<(&'static str, DocKey, Value, u64)> {
        vec![
            ("item", id(1), json!({"id": 1, "state": "done"}), 4),
            ("item", id(2), json!({"id": 2, "state": "open"}), 3),
            ("request", request("fixture.add", "r1"), completed("r1"), 2),
            ("request", request("fixture.add", "r2"), completed("r2"), 5),
            ("tally", id(1), json!({"id": 1, "seen": 2}), 4),
            ("tally", id(2), json!({"id": 2, "seen": 1}), 3),
        ]
    }

    /// The fixture chain's documents as the store reads them.
    fn read_back(store: &SqliteStore) -> Vec<(&'static str, DocKey, Value, u64)> {
        expected()
            .into_iter()
            .map(|(view, key, _, _)| {
                let (body, seq) = doc(store, view, &key).expect("present");
                (view, key, body, seq)
            })
            .collect()
    }

    fn raw(home: &Path) -> Connection {
        Connection::open(home.join("baley.db")).expect("raw connection")
    }

    fn scalar(home: &Path, query: &str) -> Option<i64> {
        raw(home)
            .query_row(query, [PROJECT], |row| row.get(0))
            .optional()
            .expect("query")
            .flatten()
    }

    fn live_gen(home: &Path) -> Option<i64> {
        scalar(
            home,
            "SELECT live_gen FROM project_gen WHERE project_id = ?1",
        )
    }

    fn building_gen(home: &Path) -> Option<i64> {
        scalar(
            home,
            "SELECT building_gen FROM project_gen WHERE project_id = ?1",
        )
    }

    fn events(home: &Path) -> i64 {
        scalar(home, "SELECT count(*) FROM event WHERE project_id = ?1").unwrap_or(0)
    }

    /// Each view's stamp in a generation: (view, projector version, set).
    fn stamps(home: &Path, generation: i64) -> Vec<(String, i64, i64)> {
        let conn = raw(home);
        let mut statement = conn
            .prepare(
                "SELECT view, projector_version, view_set_version FROM view_gen
                  WHERE project_id = ?1 AND gen = ?2 ORDER BY view",
            )
            .expect("prepare");
        statement
            .query_map(params![PROJECT, generation], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("stamps")
            .collect::<rusqlite::Result<_>>()
            .expect("stamps")
    }

    /// Every generation the project has stamps or rows in, in any catalog
    /// table.
    fn stored_generations(home: &Path) -> BTreeSet<i64> {
        let conn = raw(home);
        let mut queries =
            vec!["SELECT DISTINCT gen FROM view_gen WHERE project_id = ?1".to_owned()];
        for table in catalog_tables(&conn).expect("catalog") {
            queries.push(format!(
                "SELECT DISTINCT generation FROM {table} WHERE project_id = ?1"
            ));
        }
        let mut found = BTreeSet::new();
        for query in queries {
            let mut statement = conn.prepare(&query).expect("prepare");
            for generation in statement
                .query_map([PROJECT], |row| row.get::<_, i64>(0))
                .expect("generations")
            {
                found.insert(generation.expect("generation"));
            }
        }
        found
    }

    fn rows(home: &Path, table: &str) -> i64 {
        scalar(
            home,
            &format!("SELECT count(*) FROM {table} WHERE project_id = ?1"),
        )
        .unwrap_or(0)
    }

    fn is_read_only(result: &Result<impl std::fmt::Debug, StoreError>) -> bool {
        matches!(
            result,
            Err(StoreError::Refused(Refusal::ProjectReadOnly { project: refused, .. })) if *refused == project()
        )
    }

    // Commands project the fixture chain into both fixture views and the
    // `request` view, each document as written by hand from the events.
    // Catches a projection that ignores the stored document it builds on,
    // which would count item 1's second event as its first.
    #[test]
    fn live_projection_equals_the_hand_written_documents() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = created(home.path(), Scripted::still());
        fixture(&store);
        assert_eq!(read_back(&store), expected());
    }

    // With a live `tally` document corrupted in place, a rebuild still
    // makes the hand-written documents. Catches a rebuild that reads or
    // copies the live generation instead of replaying the events.
    #[test]
    fn a_rebuild_from_the_events_ignores_a_corrupted_live_document() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = created(home.path(), Scripted::still());
        fixture(&store);
        raw(home.path())
            .execute(
                r#"UPDATE v_tally_1 SET doc_json = '{"id":1,"seen":99}' WHERE k_id = 1"#,
                [],
            )
            .expect("corrupt");
        store.rebuild(&project()).expect("rebuild");
        assert_eq!(read_back(&store), expected());
    }

    // A command committed between two replay batches, driven by the test,
    // reaches the new live generation, and the report counts it. Catches a
    // flip without the final catch-up.
    #[test]
    fn a_command_between_replay_batches_reaches_the_new_generation() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = created(home.path(), Scripted::still());
        fixture(&store);
        let mut session = store.start_rebuild(&project()).expect("start");
        session.apply_one_batch().expect("batch");
        record(&store, "r3", &[(3, "open")]).expect("between batches");
        let report = session.finish().expect("finish").report;
        assert_eq!(
            doc(&store, "item", &id(3)),
            Some((json!({"id": 3, "state": "open"}), 6))
        );
        assert_eq!(report.events, 7);
    }

    // A second store over the same home, whose `tally` projector refuses
    // the tail event after `item` has taken it, fails the final turn:
    // `live_gen` and both views still read the old generation. Catches a
    // flip that switches views one at a time or before the tail is applied.
    #[test]
    fn the_flip_switches_both_views_in_one_transaction() {
        let home = tempfile::tempdir().expect("temp dir");
        let writer = created(home.path(), Scripted::still());
        fixture(&writer);
        let before = live_gen(home.path());
        let failing = open_as(
            home.path(),
            vec![item(2), tally(Some(TAIL))],
            2,
            Scripted::still(),
        )
        .expect("second store");
        let mut session = failing.start_rebuild(&project()).expect("start");
        session.apply_one_batch().expect("batch");
        record(&writer, "tail", &[(TAIL, "open")]).expect("tail");
        assert!(matches!(
            session.finish(),
            Err(StoreError::Projector { view, seq: 6, .. }) if view == "tally"
        ));
        assert_eq!(live_gen(home.path()), before);
        assert_eq!(
            (
                doc(&writer, "item", &id(TAIL)),
                doc(&writer, "tally", &id(TAIL))
            ),
            (
                Some((json!({"id": TAIL, "state": "open"}), 6)),
                Some((json!({"id": TAIL, "seen": 1}), 6))
            )
        );
    }

    // A session dropped after one committed batch, as a crash would leave
    // it, changes nothing live; the next rebuild removes its rows, stamps
    // and marker, and only the new live generation is left. Catches an
    // abandoned build that touches the live generation, and an orphan left
    // behind.
    #[test]
    fn an_abandoned_rebuild_leaves_live_untouched_and_the_next_removes_it() {
        let home = tempfile::tempdir().expect("temp dir");
        // Every batch reaches the time bound after its first event.
        let store = created(home.path(), Scripted::stepping(BATCH_TIME));
        fixture(&store);
        let mut session = store.start_rebuild(&project()).expect("start");
        assert_eq!(session.apply_one_batch().expect("batch").applied, 1);
        drop(session);
        assert_eq!(
            (live_gen(home.path()), building_gen(home.path())),
            (Some(0), Some(1))
        );
        assert_eq!(read_back(&store), expected());
        let report = store.rebuild(&project()).expect("rebuild");
        assert_eq!(building_gen(home.path()), None);
        assert_eq!(
            stored_generations(home.path()),
            BTreeSet::from([report.generation as i64])
        );
    }

    // The pause after a batch is the time from acquiring the queue to
    // releasing it, from supplied instants. Catches a pause that ignores
    // how long the batch held the queue.
    #[test]
    fn the_pause_after_a_batch_is_as_long_as_it_held_the_queue() {
        let home = tempfile::tempdir().expect("temp dir");
        let timing = Scripted::still();
        let store = created(home.path(), Arc::clone(&timing));
        record(&store, "r1", &[]).expect("one event");
        let mut session = store.start_rebuild(&project()).expect("start");
        timing.script(&[Duration::from_millis(40), Duration::from_millis(95)]);
        session.apply_one_batch().expect("batch");
        assert_eq!(timing.pauses().last(), Some(&Duration::from_millis(55)));
    }

    // After an output reference is reduced and a stored answer purged, a
    // rebuild makes the answer's request document as written by hand: its
    // reference, never its gone body. Catches a replay without the store's
    // own `request` projector, or one that needs purged bytes.
    #[test]
    fn rebuild_restores_request_after_reduction_and_purge() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = created(home.path(), Scripted::still());
        let output = vec![7u8; 200_000];
        let mut attached = None;
        store
            .transact(&command("fixture.attach", "a1"), &mut |tx| {
                let reference = tx.put_payload(&output, RetentionClass::Output)?;
                let mut new = event(
                    2,
                    json!({"id": 5, "state": "open", "output": reference.to_value()}),
                );
                new.attachments.push(reference.clone());
                let seq = tx.append(new)?;
                attached = Some(PayloadReference {
                    project: project(),
                    seq,
                    hash: reference.hash,
                });
                Ok(done(json!("ok"), false))
            })
            .expect("attach");
        store
            .reduce(
                &command("retention.reduce", "d1"),
                &attached.expect("attached"),
            )
            .expect("reduce");
        store
            .transact(&command("fixture.secret", "s1"), &mut |_| {
                Ok(done(json!("token"), true))
            })
            .expect("secret");
        let answer = Hash(Sha256::digest(b"\"token\"").into());
        store
            .purge(
                &command("retention.purge", "p1"),
                &[answer],
                "owner request",
            )
            .expect("purge");
        store.rebuild(&project()).expect("rebuild");
        let expected = json!({"kind": "fixture.secret", "request_id": "s1",
            "digest": "01".repeat(32), "outcome": "done",
            "answer": {"stored": {"payload": answer.to_hex(), "bytes": 7, "class": "record"}}});
        assert_eq!(
            doc(&store, "request", &request("fixture.secret", "s1")),
            Some((expected, 5))
        );
    }

    // A live generation missing the stamp of the empty `quiet` view is
    // rebuilt on its first read, and the new one stamps every registered
    // view, `quiet` included. Catches an absent or empty view taken as
    // current.
    #[test]
    fn an_older_or_missing_view_version_rebuilds_before_use() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open_as(
            home.path(),
            vec![item(2), tally(None), quiet()],
            2,
            Scripted::still(),
        )
        .expect("open");
        store
            .write(|tx| {
                tx.execute(
                    "INSERT INTO project (project_id, name, created_at) VALUES (?1, 'fixture', ?2)",
                    params![PROJECT, AT],
                )
                .map_err(sql)?;
                Ok(())
            })
            .expect("project");
        fixture(&store);
        raw(home.path())
            .execute(
                "DELETE FROM view_gen WHERE project_id = ?1 AND view = 'quiet'",
                [PROJECT],
            )
            .expect("unstamp");
        doc(&store, "item", &id(1));
        let live = live_gen(home.path()).expect("live");
        assert_eq!(
            stamps(home.path(), live),
            [
                ("item".into(), 2, 2),
                ("quiet".into(), 1, 2),
                ("request".into(), 1, 2),
                ("tally".into(), 1, 2),
            ]
        );
    }

    // A store opened with `item` at version 2 stays open while a newer
    // binary with `item` at version 3 rebuilds; the older store's next new
    // command is refused as read-only and appends nothing. This is also
    // the plan's view stamped with a newer projector version. Catches a
    // version check made only at open.
    #[test]
    fn an_old_binary_is_fenced_at_its_next_write_after_flip() {
        let home = tempfile::tempdir().expect("temp dir");
        let old = created(home.path(), Scripted::still());
        fixture(&old);
        let newer = open_as(
            home.path(),
            vec![item(3), tally(None)],
            2,
            Scripted::still(),
        )
        .expect("newer store");
        newer.rebuild(&project()).expect("rebuild");
        let before = events(home.path());
        assert!(is_read_only(&record(&old, "r3", &[(3, "open")])));
        assert_eq!(events(home.path()), before);
    }

    // A version-1 event, recorded before `item`'s shape changed, projects
    // live and replays as the same current document, while the stored row
    // keeps version 1, its payload and its hash. Catches live and replay
    // upcasting differently, a copy left at the stored version, and an
    // upcast written back into the evidence.
    #[test]
    fn replay_upcasts_an_old_event_without_rewriting_it() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = created(home.path(), Scripted::still());
        store
            .transact(&command("fixture.add", "old"), &mut |tx| {
                tx.append(event(1, json!({"id": 7, "status": "open"})))?;
                Ok(done(json!("ok"), false))
            })
            .expect("record version 1");
        let stored = || -> (i64, String, Vec<u8>) {
            raw(home.path())
                .query_row(
                    "SELECT type_version, payload_json, hash FROM event WHERE seq = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("event")
        };
        let before = stored();
        let current = Some((json!({"id": 7, "state": "open"}), 1));
        assert_eq!(doc(&store, "item", &id(7)), current);
        store.rebuild(&project()).expect("rebuild");
        assert_eq!(doc(&store, "item", &id(7)), current);
        assert_eq!(stored(), before);
        assert_eq!(before.0, 1);
    }

    // Two rebuilds in a row, each cleaning up the generation before, make
    // two different live generations, the second higher. Catches a number
    // reused once its rows are gone, which would let a cursor issued under
    // the first read the second.
    #[test]
    fn generation_numbers_are_not_reused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = created(home.path(), Scripted::still());
        fixture(&store);
        let first = store.rebuild(&project()).expect("first");
        let second = store.rebuild(&project()).expect("second");
        assert!(second.generation > first.generation, "{first:?} {second:?}");
    }

    // Documents a binary with `item` at version 1 wrote into `v_item_1`
    // are removed with their stamps by a rebuild of a binary whose `item`
    // is version 2, which no longer reads that table. Catches cleanup
    // limited to the current versions' tables.
    #[test]
    fn cleanup_removes_old_and_orphan_rows() {
        let home = tempfile::tempdir().expect("temp dir");
        let old = open_as(
            home.path(),
            vec![item(1), tally(None)],
            2,
            Scripted::still(),
        )
        .expect("version 1");
        old.write(|tx| {
            tx.execute(
                "INSERT INTO project (project_id, name, created_at) VALUES (?1, 'fixture', ?2)",
                params![PROJECT, AT],
            )
            .map_err(sql)?;
            Ok(())
        })
        .expect("project");
        fixture(&old);
        drop(old);
        let store = open_with(home.path(), Scripted::still());
        assert_eq!(rows(home.path(), "v_item_1"), 2);
        let report = store.rebuild(&project()).expect("rebuild");
        assert_eq!(rows(home.path(), "v_item_1"), 0);
        assert_eq!(
            stored_generations(home.path()),
            BTreeSet::from([report.generation as i64])
        );
    }

    // A live `tally` document whose text is corrupted in place, its key
    // kept, is reported once, at the head the comparison read. Catches a
    // comparison against the corrupted live rows themselves.
    #[test]
    fn verify_views_reports_a_corrupt_live_document() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = created(home.path(), Scripted::still());
        fixture(&store);
        raw(home.path())
            .execute(
                r#"UPDATE v_tally_1 SET doc_json = '{"id":2,"seen":5}' WHERE k_id = 2"#,
                [],
            )
            .expect("corrupt");
        assert_eq!(
            store.verify_views(&project()),
            Ok(ViewsReport {
                checked_seq: 5,
                differing: vec![("tally".into(), id(2))],
            })
        );
    }

    // A second store over the same home, whose `tally` projector refuses
    // item 2, fails verification with the projector's error and leaves no
    // scratch row, stamp or marker. Catches an ordinary error that leaves
    // a scratch generation looking like an unfinished rebuild.
    #[test]
    fn verify_views_cleans_scratch_after_projector_error() {
        let home = tempfile::tempdir().expect("temp dir");
        let writer = created(home.path(), Scripted::still());
        fixture(&writer);
        let failing = open_as(
            home.path(),
            vec![item(2), tally(Some(2))],
            2,
            Scripted::still(),
        )
        .expect("second store");
        assert!(matches!(
            failing.verify_views(&project()),
            Err(StoreError::Projector { view, seq: 3, .. }) if view == "tally"
        ));
        assert_eq!(building_gen(home.path()), None);
        assert_eq!(stored_generations(home.path()), BTreeSet::from([0]));
    }

    // A store with `item` at version 2 stays open while a newer binary
    // flips the project to `item` version 3; its next read is refused as
    // read-only. Catches a read answered as absent from the older table,
    // which is empty at the new generation.
    #[test]
    fn a_newer_live_view_read_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let old = created(home.path(), Scripted::still());
        fixture(&old);
        let newer = open_as(
            home.path(),
            vec![item(3), tally(None)],
            2,
            Scripted::still(),
        )
        .expect("newer store");
        newer.rebuild(&project()).expect("rebuild");
        assert!(is_read_only(&old.get(&project(), "item", &id(1))));
    }

    // A newer binary adds the `quiet` view under set version 3 and
    // rebuilds; the older store, still open at set 2, is refused its next
    // new request and appends nothing. Catches an older binary writing on
    // without a view a newer one added.
    #[test]
    fn an_added_view_set_fences_an_older_binary() {
        let home = tempfile::tempdir().expect("temp dir");
        let old = created(home.path(), Scripted::still());
        fixture(&old);
        let newer = open_as(
            home.path(),
            vec![item(2), tally(None), quiet()],
            3,
            Scripted::still(),
        )
        .expect("newer store");
        newer.rebuild(&project()).expect("rebuild");
        let before = events(home.path());
        assert!(is_read_only(&record(&old, "r3", &[(3, "open")])));
        assert_eq!(events(home.path()), before);
    }

    // A binary that dropped `tally` under set version 3 rebuilds on its
    // first read: the new generation is stamped set 3 for `item` and
    // `request` only, and `tally`'s rows and stamps are gone. Catches a
    // removed view that fences the project for good, or whose rows stay.
    #[test]
    fn a_removed_view_rebuilds_and_cleans_rows() {
        let home = tempfile::tempdir().expect("temp dir");
        let old = created(home.path(), Scripted::still());
        fixture(&old);
        drop(old);
        let store = open_as(home.path(), vec![item(2)], 3, Scripted::still()).expect("set 3");
        assert_eq!(
            doc(&store, "item", &id(2)),
            Some((json!({"id": 2, "state": "open"}), 3))
        );
        let live = live_gen(home.path()).expect("live");
        assert_eq!(
            stamps(home.path(), live),
            [("item".into(), 2, 3), ("request".into(), 1, 3)]
        );
        assert_eq!(rows(home.path(), "v_tally_1"), 0);
        assert_eq!(
            scalar(
                home.path(),
                "SELECT count(*) FROM view_gen WHERE project_id = ?1 AND view = 'tally'"
            ),
            Some(0)
        );
    }

    // Opening with the `quiet` view added but the set version still 2,
    // whose names are recorded, is refused at open, naming `quiet`.
    // Catches a set version trusted without its names, which would let two
    // different sets pass as one.
    #[test]
    fn a_changed_view_set_needs_a_new_version() {
        let home = tempfile::tempdir().expect("temp dir");
        drop(open_with(home.path(), Scripted::still()));
        let changed = open_as(
            home.path(),
            vec![item(2), tally(None), quiet()],
            2,
            Scripted::still(),
        );
        assert!(matches!(
            changed,
            Err(StoreError::Refused(Refusal::MalformedKey { view, .. })) if view == "quiet"
        ));
    }
}
