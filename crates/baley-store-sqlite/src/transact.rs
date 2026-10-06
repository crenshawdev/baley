//! The shared command write path (design 0001, Commands and Figure 7;
//! EVD-R5, R6, R7).
//!
//! Before the request lookup the project's live `request` view must be at
//! this binary's version, and before a new request's decision every live
//! view must be (see `rebuild.rs`); the command writes the live generation
//! only.
//!
//! A decision's events and document changes are held in memory until it
//! returns. Reads inside the decision see them; the stored rows stay as they
//! stood when the transaction began. So the re-check of what the caller's
//! slow work observed compares against the store before this command, never
//! against the command's own writes. Then, in the same transaction, the
//! events are written with their chain and references, each changed
//! document once, any anchor row the decision recorded, and the project's
//! head moves. A claim appends no completion until its record step.

use std::collections::BTreeMap;

use baley_store::{
    ANCHOR_PUSHED, ANCHOR_PUSHED_VERSION, ANCHOR_STREAM, Absence, Anchor, AnchorPushedPayload,
    Answer, Block, CLAIM_SCOPE_VIEW, COMMAND_COMPLETED, COMMAND_COMPLETED_VERSION, Claim, ClaimId,
    ClaimOwner, Command, Decide, Decision, DocKey, Document, Event, EventDraft, EventMatch,
    GitFact, GitFacts, Hash, Head, INLINE_ANSWER_LIMIT, IndexQuery, KeyValue, NewEvent, Observed,
    Outcome, PAYLOAD_PURGED, PAYLOAD_REDUCED, Page, PageRequest, PayloadRef, PayloadStatus,
    ProjectId, PurgedEvent, REQUEST_VIEW, Recorded, ReducedEvent, Refusal, RequestState,
    RetentionClass, StaleInput, StoreError, StoredAnchor, StreamName, Transaction, UtcInstant,
    anchor_tag, blocking, canonical_json, claim_state, command_stream, completed_payload_for,
    request_key, request_state, store_owned,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::Value;

use crate::claim::{claim_from_doc, open_claims_in};
use crate::payload::{put_payload, put_reference, sql_int, stored};
use crate::rebuild::{NotCurrent, caller_text, read_only};
use crate::store::{SqliteStore, sql};
use crate::view::{
    Fence, Staging, find_documents, find_with_staged, fold, get_document, live_views, write_staged,
};

impl SqliteStore {
    /// Runs one database-only command: the writer queue, `BEGIN IMMEDIATE`
    /// and the epoch (the write path), then the project, the request and
    /// the project's readability, then `decide`, the re-check of what it
    /// observed, the answer, `command.completed`, the writes and the head,
    /// and commit. Anything that fails records nothing.
    pub fn transact(
        &self,
        command: &Command,
        decide: &mut Decide<'_>,
    ) -> Result<Recorded, StoreError> {
        match self.command_path(
            command,
            |_tx, outcome, _| Ok(outcome),
            |work| {
                let decision = decide(work)?;
                let outcome = work.outcome(&decision)?;
                Ok((
                    outcome.clone(),
                    outcome,
                    decision.git,
                    Some(decision.observed),
                ))
            },
        )? {
            CommandResult::Replayed(outcome) => Ok(Recorded::Replayed { outcome }),
            CommandResult::New(outcome, head) => Ok(Recorded::New { outcome, head }),
        }
    }

    /// Runs the shared request, fence, event and head path for domain and
    /// store-owned commands. The replay callback sees the recorded event
    /// sequence. When the live `request` view, or for a new request any
    /// live view, is behind this binary's, the turn ends without writing,
    /// the views are brought forward outside the queue, and the command
    /// starts again; a newer one refuses. A guard store answers views behind
    /// as `NeedsRebuild` instead, and stamps an empty project's first.
    pub(crate) fn command_path<T>(
        &self,
        command: &Command,
        replay: impl FnOnce(&rusqlite::Transaction<'_>, Outcome, u64) -> Result<T, StoreError>,
        new: impl FnOnce(
            &mut Work<'_, '_>,
        )
            -> Result<(T, Outcome, Option<GitFacts>, Option<Observed>), StoreError>,
    ) -> Result<CommandResult<T>, StoreError> {
        self.command_path_for(
            command,
            Entry::Ordinary,
            move |tx, state, seq| match state {
                RequestState::Completed(_, outcome) => replay(tx, outcome, seq),
                _ => Err(StoreError::Unavailable(
                    "a request that cannot replay".into(),
                )),
            },
            move |work, _| {
                let (result, outcome, git, observed) = new(work)?;
                work.finish(&outcome, git, observed.as_ref())?;
                Ok(result)
            },
        )
    }

    /// The common path for all command entries. The callback appends the
    /// store-owned event or events appropriate to its entry.
    pub(crate) fn command_path_for<T>(
        &self,
        command: &Command,
        entry: Entry<'_>,
        replay: impl FnOnce(&rusqlite::Transaction<'_>, RequestState, u64) -> Result<T, StoreError>,
        new: impl FnOnce(&mut Work<'_, '_>, Option<Claim>) -> Result<T, StoreError>,
    ) -> Result<CommandResult<T>, StoreError> {
        UtcInstant::parse(&command.recorded_at).map_err(|_| {
            StoreError::Refused(Refusal::InvalidEvent(
                "recorded_at is not a UTC instant".into(),
            ))
        })?;
        let mut replay = Some(replay);
        let mut new = Some(new);
        loop {
            match self.command_turn(command, entry, &mut replay, &mut new)? {
                Ok(recorded) => return Ok(recorded),
                Err(found) => self.catch_up(&command.project, found)?,
            }
        }
    }

    /// One try at a command under the writer queue, or why the project's
    /// views must be readied first; nothing is written then, and the
    /// callbacks are still unused.
    fn command_turn<T>(
        &self,
        command: &Command,
        entry: Entry<'_>,
        replay: &mut Option<
            impl FnOnce(&rusqlite::Transaction<'_>, RequestState, u64) -> Result<T, StoreError>,
        >,
        new: &mut Option<impl FnOnce(&mut Work<'_, '_>, Option<Claim>) -> Result<T, StoreError>>,
    ) -> Result<Result<CommandResult<T>, NotCurrent>, StoreError> {
        let recorded = self.write(|tx| {
            let project = &command.project;
            let head = stored_head(tx, project)?;
            let live = live_views(tx, project)?;
            let generation = match self.views().judge_request(&live)? {
                Fence::Current(generation) => generation,
                Fence::Newer(reason) => return Err(read_only(project, reason)),
                Fence::Unstamped => return Ok(Err(NotCurrent::Unstamped)),
                Fence::Behind => return Ok(Err(NotCurrent::Behind)),
            };
            let requests = self.views().table(REQUEST_VIEW)?;
            if let Some(document) =
                get_document(tx, requests, project, generation, &request_key(command))?
            {
                let state = request_state(&document.body).ok_or_else(|| {
                    StoreError::Unavailable("a request document that holds no state".into())
                })?;
                let digest = match &state {
                    RequestState::Completed(digest, _) => *digest,
                    RequestState::Claimed(doc) | RequestState::AwaitingOwner(doc, _) => doc.digest,
                };
                if digest != command.digest {
                    return Err(StoreError::Refused(Refusal::RequestDigestMismatch {
                        request_id: command.request_id.clone(),
                    }));
                }
                match state {
                    RequestState::Completed(digest, mut outcome) => {
                        if let Answer::Stored(reference) = &outcome.answer {
                            let status = reference_status(
                                tx,
                                project,
                                document.produced_seq,
                                &reference.hash,
                            )?;
                            if !matches!(status, PayloadStatus::Present { .. }) {
                                outcome.answer = Answer::Tombstone {
                                    reference: reference.clone(),
                                    status,
                                };
                            }
                        }
                        let replay = replay.take().ok_or_else(spent)?;
                        return replay(
                            tx,
                            RequestState::Completed(digest, outcome),
                            document.produced_seq,
                        )
                        .map(|value| Ok(CommandResult::Replayed(value)));
                    }
                    RequestState::Claimed(doc) | RequestState::AwaitingOwner(doc, _)
                        if matches!(entry, Entry::Complete(_)) =>
                    {
                        if document.body.get("state").and_then(Value::as_str)
                            == Some("awaiting_owner")
                        {
                            return Err(StoreError::Refused(Refusal::AwaitingOwner(doc.id)));
                        }
                        let Entry::Complete(owner) = entry else {
                            unreachable!()
                        };
                        if &doc.owner != owner {
                            return Err(StoreError::Refused(Refusal::NotClaimOwner(doc.id)));
                        }
                        if doc.scope != command.scope {
                            return Err(StoreError::Refused(Refusal::ScopeMismatch {
                                claim: doc.id,
                                supplied: command.scope.clone(),
                                claimed: doc.scope,
                            }));
                        }
                    }
                    state @ (RequestState::Claimed(_) | RequestState::AwaitingOwner(_, _))
                        if matches!(entry, Entry::Claim) =>
                    {
                        let replay = replay.take().ok_or_else(spent)?;
                        return replay(tx, state, document.produced_seq)
                            .map(|value| Ok(CommandResult::Replayed(value)));
                    }
                    RequestState::Claimed(doc) | RequestState::AwaitingOwner(doc, _) => {
                        let claim = claim_from_doc(
                            tx,
                            project,
                            &doc,
                            document
                                .body
                                .get("held")
                                .and_then(|held| held.get("seq"))
                                .and_then(Value::as_u64),
                        )?;
                        return Err(StoreError::Blocked(Block {
                            claim: doc.id,
                            state: claim_state(&claim, &command.recorded_at).map_err(|_| {
                                StoreError::Unavailable("an invalid lease time".into())
                            })?,
                        }));
                    }
                }
            } else if matches!(entry, Entry::Complete(_)) {
                return Err(StoreError::Refused(Refusal::UnknownClaim(ClaimId {
                    kind: command.kind.clone(),
                    request_id: command.request_id.clone(),
                })));
            }
            // A replay above only reads; a project this binary cannot read
            // still answers it, whatever its other views. A new request
            // uses them all, and appends only events this binary reads.
            match self.views().judge_set(&live) {
                Fence::Current(_) => {}
                Fence::Newer(reason) => return Err(read_only(project, reason)),
                Fence::Unstamped => return Ok(Err(NotCurrent::Unstamped)),
                Fence::Behind => return Ok(Err(NotCurrent::Behind)),
            }
            self.check_readable(tx, project, head.as_ref())?;
            let own_claim = if matches!(entry, Entry::Complete(_)) {
                let document =
                    get_document(tx, requests, project, generation, &request_key(command))?
                        .ok_or_else(|| {
                            StoreError::Unavailable("a claim request vanished".into())
                        })?;
                let RequestState::Claimed(doc) =
                    request_state(&document.body).ok_or_else(|| {
                        StoreError::Unavailable("a request document that holds no state".into())
                    })?
                else {
                    return Err(StoreError::Unavailable("a claim request changed".into()));
                };
                Some(claim_from_doc(tx, project, &doc, None)?)
            } else {
                None
            };
            self.check_scope(tx, command, generation, entry.excluded(own_claim.as_ref()))?;
            let new = new.take().ok_or_else(spent)?;
            let mut work = Work::new(self, tx, command, head, generation);
            let result = new(&mut work, own_claim)?;
            // An operation that failed fails the command, even when the
            // decision went on past the error.
            if let Some(error) = work.failed.take() {
                return Err(error);
            }
            work.check_payloads_attached()?;
            let head = work.write()?;
            Ok(Ok(CommandResult::New(result, head)))
        })?;
        if let Ok(CommandResult::New(_, head)) = &recorded {
            // Committed: every event through the new head was checked
            // readable or was written here by this binary.
            self.readable()
                .insert(command.project.clone(), head.clone());
        }
        Ok(recorded)
    }

    fn check_scope(
        &self,
        tx: &rusqlite::Transaction<'_>,
        command: &Command,
        generation: i64,
        excluded: Option<&ClaimId>,
    ) -> Result<(), StoreError> {
        if command.scope.is_empty() {
            return Ok(());
        }
        let scopes = self.views().table(CLAIM_SCOPE_VIEW)?;
        let requests = self.views().table(REQUEST_VIEW)?;
        let mut holders = Vec::new();
        for token in &command.scope {
            let key = DocKey(vec![KeyValue::Text(token.clone())]);
            let Some(token_doc) = get_document(tx, scopes, &command.project, generation, &key)?
            else {
                continue;
            };
            let invalid = || {
                StoreError::Unavailable(
                    "a claim_scope document names a claim that does not hold it".into(),
                )
            };
            let named = token_doc.body.get("claim").ok_or_else(invalid)?;
            let kind = named
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let request_id = named
                .get("request_id")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let seq = named
                .get("seq")
                .and_then(Value::as_u64)
                .ok_or_else(invalid)?;
            if token_doc.body.get("scope").and_then(Value::as_str) != Some(token) {
                return Err(invalid());
            }
            let request_key = DocKey(vec![
                KeyValue::Text(kind.into()),
                KeyValue::Text(request_id.into()),
            ]);
            let request_doc =
                get_document(tx, requests, &command.project, generation, &request_key)?
                    .ok_or_else(invalid)?;
            let doc = match request_state(&request_doc.body).ok_or_else(invalid)? {
                RequestState::Claimed(doc) | RequestState::AwaitingOwner(doc, _) => doc,
                RequestState::Completed(_, _) => return Err(invalid()),
            };
            if doc.id.kind.0 != kind
                || doc.id.request_id.0 != request_id
                || doc.seq != seq
                || !doc.scope.contains(token)
            {
                return Err(invalid());
            }
            if excluded != Some(&doc.id) {
                let held = request_doc
                    .body
                    .get("held")
                    .and_then(|held| held.get("seq"))
                    .and_then(Value::as_u64);
                holders.push(claim_from_doc(tx, &command.project, &doc, held)?);
            }
        }
        if let Some(block) = blocking(&command.scope, &holders, &command.recorded_at)
            .map_err(|_| StoreError::Unavailable("an invalid lease time".into()))?
        {
            return Err(StoreError::Blocked(block));
        }
        Ok(())
    }

    /// Refuses a project holding an event this binary cannot read. Checks
    /// only the events recorded since the last check, unless the chain no
    /// longer holds the checked head, as after a restore.
    pub(crate) fn check_readable(
        &self,
        tx: &rusqlite::Transaction<'_>,
        project: &ProjectId,
        head: Option<&Head>,
    ) -> Result<(), StoreError> {
        let Some(head) = head else {
            return Ok(());
        };
        let mark = self.readable().get(project).cloned();
        let from = match mark {
            Some(mark) if stored_hash(tx, project, mark.seq)? == Some(mark.hash) => mark.seq,
            _ => 0,
        };
        if from == head.seq {
            return Ok(());
        }
        let mut statement = tx
            .prepare(
                "SELECT DISTINCT type, type_version FROM event WHERE project_id = ?1 AND seq > ?2",
            )
            .map_err(sql)?;
        let types = statement
            .query_map(params![project.0, sql_int(from)?], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })
            .map_err(sql)?;
        for found in types {
            let (type_name, version) = found.map_err(sql)?;
            if !self.reads(&type_name, version) {
                return Err(StoreError::Refused(Refusal::ProjectReadOnly {
                    project: project.clone(),
                    reason: format!("{type_name} version {version} is not readable by this binary"),
                }));
            }
        }
        self.readable().insert(project.clone(), head.clone());
        Ok(())
    }
}

/// Whether a command was answered from its request or wrote a new head.
pub(crate) enum CommandResult<T> {
    Replayed(T),
    New(T, Head),
}

/// Which request lookup rules and scope exclusion apply.
#[derive(Clone, Copy)]
pub(crate) enum Entry<'a> {
    Ordinary,
    Claim,
    Complete(&'a ClaimOwner),
    Reconcile(&'a ClaimId),
}

impl Entry<'_> {
    fn excluded<'a>(&'a self, own: Option<&'a Claim>) -> Option<&'a ClaimId> {
        match self {
            Self::Complete(_) => own.map(|claim| &claim.id),
            Self::Reconcile(id) => Some(id),
            _ => None,
        }
    }
}

/// A callback asked for twice: a turn that used one never asks for a retry.
fn spent() -> StoreError {
    StoreError::Unavailable("a command callback ran twice".into())
}

/// The state of one answer's own reference, including its releasing event.
pub(crate) fn reference_status(
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
    seq: u64,
    hash: &Hash,
) -> Result<PayloadStatus, StoreError> {
    let released = tx
        .query_row(
            "SELECT released_seq FROM payload_ref WHERE project_id = ?1 AND seq = ?2 AND hash = ?3",
            params![project.0, sql_int(seq)?, &hash.0[..]],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(|| {
            StoreError::Unavailable(format!("payload {hash}: answer reference is missing"))
        })?;
    let Some(released) = released else {
        return Ok(stored(tx, hash)?.status);
    };
    let event: (String, String) = tx
        .query_row(
            "SELECT type, payload_json FROM event WHERE project_id = ?1 AND seq = ?2",
            params![project.0, released],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(sql)?;
    let value: Value = serde_json::from_str(&event.1)
        .map_err(|error| StoreError::Unavailable(format!("a release event: {error}")))?;
    match event.0.as_str() {
        PAYLOAD_PURGED => Ok(PayloadStatus::Purged {
            reason: PurgedEvent::from_value(&value)
                .ok_or_else(|| StoreError::Unavailable("a malformed purge event".into()))?
                .reason,
        }),
        PAYLOAD_REDUCED => {
            let reduced = ReducedEvent::from_value(&value)
                .ok_or_else(|| StoreError::Unavailable("a malformed reduction event".into()))?;
            Ok(PayloadStatus::Reduced {
                excerpt: reduced.excerpt,
                kept: reduced.kept.to_vec(),
            })
        }
        _ => Err(StoreError::Unavailable(
            "a reference released by another event".into(),
        )),
    }
}

/// The hash of the project's event at `seq`, `None` when there is none.
fn stored_hash(
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
    seq: u64,
) -> Result<Option<Hash>, StoreError> {
    let bytes = tx
        .query_row(
            "SELECT hash FROM event WHERE project_id = ?1 AND seq = ?2",
            params![project.0, sql_int(seq)?],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(sql)?;
    bytes
        .map(|bytes| {
            bytes.try_into().map(Hash).map_err(|_| {
                StoreError::Unavailable("a stored event hash that is not 32 bytes".into())
            })
        })
        .transpose()
}

/// The project's stored head, `None` for an empty chain.
pub(crate) fn stored_head(
    tx: &Connection,
    project: &ProjectId,
) -> Result<Option<Head>, StoreError> {
    let row = tx
        .query_row(
            "SELECT head_seq, head_hash FROM project WHERE project_id = ?1",
            [&project.0],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(|| StoreError::Refused(Refusal::UnknownProject(project.clone())))?;
    match (unsigned(row.0)?, row.1) {
        (0, None) => Ok(None),
        (seq, Some(bytes)) if seq > 0 => {
            let hash: [u8; 32] = bytes.try_into().map_err(|_| {
                StoreError::Unavailable("a stored head hash that is not 32 bytes".into())
            })?;
            Ok(Some(Head {
                seq,
                hash: Hash(hash),
            }))
        }
        (seq, _) => Err(StoreError::Unavailable(format!(
            "the project's head at {seq} does not agree with its head hash"
        ))),
    }
}

/// A stored count or sequence, which is never negative.
fn unsigned(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Unavailable(format!("a stored count of {value}")))
}

/// Refuses the command as stale if anything the caller's slow work
/// observed moved. Runs before the command's writes, so it sees the store
/// as it stood when the command began.
pub(crate) fn recheck(
    store: &SqliteStore,
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
    generation: i64,
    observed: &Observed,
) -> Result<(), StoreError> {
    for git in &observed.git {
        for (fact, seen, now) in [
            (GitFact::Head, &git.head_seen, &git.head_now),
            (GitFact::Index, &git.index_seen, &git.index_now),
        ] {
            if seen != now {
                return Err(StoreError::Stale(StaleInput::Git {
                    checkout: git.checkout.clone(),
                    fact,
                    seen: seen.clone(),
                    now: now.clone(),
                }));
            }
        }
    }
    for document in &observed.documents {
        let table = store.views().table(&document.view)?;
        let now = get_document(tx, table, project, generation, &document.key)?
            .map(|found| found.produced_seq);
        if now != document.produced_seq {
            return Err(StoreError::Stale(StaleInput::Document {
                view: document.view.clone(),
                key: document.key.clone(),
                seen: document.produced_seq,
                now,
            }));
        }
    }
    for absence in &observed.absences {
        let present = match absence {
            Absence::Event(matching) => stored_event_exists(tx, project, matching)?,
            Absence::Documents {
                view,
                index,
                equals,
            } => {
                let table = store.views().table(view)?;
                let query = IndexQuery {
                    index: index.clone(),
                    equals: equals.clone(),
                    page: PageRequest {
                        limit: 1,
                        after: None,
                    },
                };
                !find_documents(tx, table, project, generation, &query)?
                    .items
                    .is_empty()
            }
        };
        if present {
            return Err(StoreError::Stale(StaleInput::Absence(absence.clone())));
        }
    }
    Ok(())
}

/// Whether a stored event of the project matches. SQL narrows by type,
/// stream and the fields it can compare exactly (strings and integers under
/// names a JSON path can quote); every candidate is then compared in full.
fn stored_event_exists(
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
    matching: &EventMatch,
) -> Result<bool, StoreError> {
    let mut sql_text =
        String::from("SELECT payload_json FROM event WHERE project_id = ? AND type = ?");
    let mut bound = vec![
        SqlValue::Text(project.0.clone()),
        SqlValue::Text(matching.type_name.clone()),
    ];
    if let Some(stream) = &matching.stream {
        sql_text.push_str(" AND stream = ?");
        bound.push(SqlValue::Text(stream.0.clone()));
    }
    for (name, value) in &matching.fields {
        // SQLite ends a path at a NUL.
        if name.contains(['"', '\\', '\0']) {
            continue;
        }
        let value = match value {
            Value::String(text) => SqlValue::Text(text.clone()),
            Value::Number(number) => match number.as_i64() {
                Some(number) => SqlValue::Integer(number),
                None => continue,
            },
            _ => continue,
        };
        sql_text.push_str(" AND json_extract(payload_json, ?) = ?");
        bound.push(SqlValue::Text(format!("$.\"{name}\"")));
        bound.push(value);
    }
    let mut statement = tx.prepare(&sql_text).map_err(sql)?;
    let mut rows = statement.query(params_from_iter(bound)).map_err(sql)?;
    while let Some(row) = rows.next().map_err(sql)? {
        let text: String = row.get(0).map_err(sql)?;
        let payload: Value = serde_json::from_str(&text)
            .map_err(|error| StoreError::Unavailable(format!("a stored payload: {error}")))?;
        if fields_match(&payload, matching) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn fields_match(payload: &Value, matching: &EventMatch) -> bool {
    matching
        .fields
        .iter()
        .all(|(name, value)| payload.get(name) == Some(value))
}

/// Every reference object anywhere in `value`. Refuses an object with a
/// reference's three fields that is not exactly one, which would otherwise
/// pass as plain JSON and leave its body unreferenced.
fn references_in(value: &Value, found: &mut Vec<PayloadRef>) -> Result<(), StoreError> {
    if let Some(reference) = PayloadRef::from_value(value) {
        found.push(reference);
        return Ok(());
    }
    match value {
        Value::Array(items) => {
            for item in items {
                references_in(item, found)?;
            }
        }
        Value::Object(fields) => {
            if ["payload", "bytes", "class"]
                .iter()
                .all(|name| fields.contains_key(*name))
            {
                return Err(StoreError::Refused(Refusal::InvalidEvent(
                    "an object with a payload reference's fields is not a reference".into(),
                )));
            }
            for field in fields.values() {
                references_in(field, found)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// One command's decision at work inside the write transaction.
pub(crate) struct Work<'s, 't> {
    store: &'s SqliteStore,
    pub(crate) tx: &'s rusqlite::Transaction<'t>,
    command: &'s Command,
    /// The head after the events appended so far.
    head: Option<Head>,
    /// The live generation, the only one a command reads or writes.
    generation: i64,
    events: Vec<(Event, Vec<PayloadRef>)>,
    /// Each stream's version after the events appended so far.
    streams: BTreeMap<StreamName, u64>,
    /// Per view, the documents the appended events changed.
    staged: Staging,
    /// Every payload the decision stored, each of which an event must
    /// attach.
    put: Vec<Hash>,
    /// Anchor rows the decision recorded, written with the events.
    anchors: Vec<StoredAnchor>,
    /// The first error any operation returned, which fails the command.
    failed: Option<StoreError>,
}

impl<'s, 't> Work<'s, 't> {
    pub(crate) fn new(
        store: &'s SqliteStore,
        tx: &'s rusqlite::Transaction<'t>,
        command: &'s Command,
        head: Option<Head>,
        generation: i64,
    ) -> Self {
        Self {
            store,
            tx,
            command,
            head,
            generation,
            events: Vec::new(),
            streams: BTreeMap::new(),
            staged: BTreeMap::new(),
            put: Vec::new(),
            anchors: Vec::new(),
            failed: None,
        }
    }

    /// Passes an operation's result through, keeping its error to fail the
    /// command should the decision go on past it.
    fn noted<T>(&mut self, result: Result<T, StoreError>) -> Result<T, StoreError> {
        if let Err(error) = &result {
            self.failed.get_or_insert(error.clone());
        }
        result
    }

    /// Refuses a payload the decision stored that no event attaches.
    pub(crate) fn check_payloads_attached(&self) -> Result<(), StoreError> {
        for hash in &self.put {
            let attached = self.events.iter().any(|(_, attachments)| {
                attachments
                    .iter()
                    .any(|attachment| attachment.hash == *hash)
            });
            if !attached {
                return Err(StoreError::Refused(Refusal::UnattachedPayload(*hash)));
            }
        }
        Ok(())
    }

    fn project_id(&self) -> &ProjectId {
        &self.command.project
    }

    /// The stream's version after the events appended so far.
    fn stream_version(&self, stream: &StreamName) -> Result<u64, StoreError> {
        if let Some(version) = self.streams.get(stream) {
            return Ok(*version);
        }
        self.tx
            .query_row(
                "SELECT max(stream_version) FROM event WHERE project_id = ?1 AND stream = ?2",
                params![self.project_id().0, stream.0],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(sql)?
            .map_or(Ok(0), unsigned)
    }

    /// Refuses an attachment the payload does not carry, a payload
    /// reference not listed as an attachment, and a reference to a body
    /// that is not stored whole under that length.
    fn check_attachments(&self, event: &NewEvent) -> Result<(), StoreError> {
        let invalid = |reason: &str| StoreError::Refused(Refusal::InvalidEvent(reason.into()));
        let mut carried = Vec::new();
        references_in(&event.payload, &mut carried)?;
        for (at, attachment) in event.attachments.iter().enumerate() {
            if event.attachments[..at]
                .iter()
                .any(|earlier| earlier.hash == attachment.hash)
            {
                return Err(invalid("an attachment is listed twice"));
            }
            if !carried.contains(attachment) {
                return Err(invalid(
                    "an attachment is not carried in the event's payload",
                ));
            }
            let stored = self
                .tx
                .query_row(
                    "SELECT state, bytes FROM payload WHERE hash = ?1",
                    [&attachment.hash.0[..]],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()
                .map_err(sql)?;
            match stored {
                None => {
                    return Err(StoreError::Refused(Refusal::UnknownPayload(
                        attachment.hash,
                    )));
                }
                Some((state, _)) if state != "present" => {
                    return Err(StoreError::Refused(Refusal::PayloadTombstoned(
                        attachment.hash,
                    )));
                }
                Some((_, bytes)) if unsigned(bytes)? != attachment.bytes => {
                    return Err(invalid("an attachment's length is not its body's"));
                }
                Some(_) => {}
            }
        }
        if carried
            .iter()
            .any(|reference| !event.attachments.contains(reference))
        {
            return Err(invalid(
                "the payload carries a reference that is not listed as an attachment",
            ));
        }
        Ok(())
    }

    /// Places an event after the head, runs the projectors over it, and
    /// holds both until the command writes.
    pub(crate) fn push(&mut self, event: NewEvent) -> Result<u64, StoreError> {
        if !self.store.reads(&event.type_name, event.type_version) {
            return Err(StoreError::Refused(Refusal::UnreadableType {
                type_name: event.type_name,
                version: event.type_version,
            }));
        }
        self.check_attachments(&event)?;
        let stream_version = self.stream_version(&event.stream)? + 1;
        let seq = self.head.as_ref().map_or(1, |head| head.seq + 1);
        let prev = self.head.as_ref().map(|head| head.hash);
        let draft = EventDraft {
            stream: event.stream.0.clone(),
            stream_version,
            type_name: event.type_name,
            type_version: event.type_version,
            actor: self.command.actor.clone(),
            caller: self.command.caller.clone(),
            recorded_at: self.command.recorded_at.clone(),
            request_id: self.command.request_id.clone(),
            git: event.git,
            policy_version: self.command.policy_version,
            payload: event.payload,
        };
        let sealed = Event::seal(self.project_id().clone(), seq, prev, draft)
            .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
        self.apply_projectors(&sealed)?;
        self.streams.insert(event.stream, stream_version);
        self.head = Some(Head {
            seq,
            hash: sealed.hash,
        });
        self.events.push((sealed, event.attachments));
        Ok(seq)
    }

    /// Folds the event's projection copy, upcast as replay upcasts it, into
    /// the staged documents of every projector that handles its type. A
    /// decision may append an older version its upcaster then refuses.
    fn apply_projectors(&mut self, event: &Event) -> Result<(), StoreError> {
        let copy = self
            .store
            .projection_copy(event)
            .map_err(|reason| StoreError::Refused(Refusal::InvalidEvent(reason)))?;
        fold(
            self.store,
            self.tx,
            &self.command.project,
            self.generation,
            &copy,
            &mut self.staged,
        )
    }

    /// The outcome the caller receives: the answer inline when it is small
    /// and not sensitive, otherwise stored as a `record` payload.
    pub(crate) fn outcome(&mut self, decision: &Decision) -> Result<Outcome, StoreError> {
        let bytes = canonical_json(&decision.answer)
            .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
        let answer = if !decision.sensitive && bytes.len() <= INLINE_ANSWER_LIMIT {
            Answer::Inline(decision.answer.clone())
        } else {
            Answer::Stored(put_payload(self.tx, &bytes, RetentionClass::Record)?)
        };
        Ok(Outcome {
            kind: decision.kind,
            answer,
        })
    }

    pub(crate) fn recheck_observed(&self, observed: &Observed) -> Result<(), StoreError> {
        recheck(
            self.store,
            self.tx,
            self.project_id(),
            self.generation,
            observed,
        )
    }

    pub(crate) fn current_head(&self) -> Option<Head> {
        self.head.clone()
    }

    /// Appends `command.completed` for the outcome, with the git facts it
    /// depended on.
    pub(crate) fn complete_for(
        &mut self,
        kind: &baley_store::CommandKind,
        request_id: &baley_store::RequestId,
        digest: &Hash,
        scope: &[String],
        outcome: &Outcome,
        git: Option<GitFacts>,
    ) -> Result<(), StoreError> {
        let attachments = match &outcome.answer {
            Answer::Stored(reference) | Answer::Tombstone { reference, .. } => {
                vec![reference.clone()]
            }
            Answer::Inline(_) => Vec::new(),
        };
        self.push(NewEvent {
            stream: command_stream(kind),
            type_name: COMMAND_COMPLETED.into(),
            type_version: COMMAND_COMPLETED_VERSION,
            git,
            payload: completed_payload_for(kind, request_id, digest, scope, outcome),
            attachments,
        })?;
        Ok(())
    }

    pub(crate) fn finish(
        &mut self,
        outcome: &Outcome,
        git: Option<GitFacts>,
        observed: Option<&Observed>,
    ) -> Result<(), StoreError> {
        if let Some(observed) = observed {
            recheck(
                self.store,
                self.tx,
                self.project_id(),
                self.generation,
                observed,
            )?;
        }
        let kind = self.command.kind.clone();
        let request_id = self.command.request_id.clone();
        let digest = self.command.digest;
        self.complete_for(&kind, &request_id, &digest, &[], outcome, git)
    }

    /// Writes the events, their references, each staged document once and
    /// the new head.
    pub(crate) fn write(self) -> Result<Head, StoreError> {
        let head = self
            .head
            .clone()
            .ok_or_else(|| StoreError::Unavailable("a command that recorded no event".into()))?;
        let project = self.project_id();
        for (event, attachments) in &self.events {
            insert_event(self.tx, event)?;
            for attachment in attachments {
                put_reference(self.tx, project, event.seq, attachment)?;
            }
        }
        write_staged(self.tx, self.store, project, self.generation, &self.staged)?;
        for row in &self.anchors {
            self.tx
                .execute(
                    "INSERT INTO anchor (project_id, seq, head_hash, tag, pushed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        project.0,
                        sql_int(row.anchor.seq)?,
                        &row.anchor.hash.0[..],
                        row.tag,
                        row.pushed_at
                    ],
                )
                .map_err(sql)?;
        }
        self.tx
            .execute(
                "UPDATE project SET head_seq = ?1, head_hash = ?2 WHERE project_id = ?3",
                params![sql_int(head.seq)?, &head.hash.0[..], project.0],
            )
            .map_err(sql)?;
        Ok(head)
    }
}

/// One event row, its payload as canonical JSON text.
fn insert_event(tx: &rusqlite::Transaction<'_>, event: &Event) -> Result<(), StoreError> {
    let payload = canonical_json(&event.payload)
        .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
    let payload = String::from_utf8(payload)
        .map_err(|_| StoreError::Unavailable("canonical JSON that is not UTF-8".into()))?;
    let caller = caller_text(event.caller.as_ref())?;
    tx.execute(
        "INSERT INTO event (project_id, seq, stream, stream_version, type, type_version, actor,
           caller, recorded_at, request_id, git_commit, git_tree, git_checkout, policy_version,
           payload_json, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            event.project_id.0,
            sql_int(event.seq)?,
            event.stream,
            sql_int(event.stream_version)?,
            event.type_name,
            event.type_version,
            event.actor.as_str(),
            caller,
            event.recorded_at,
            event.request_id.0,
            event.git.as_ref().map(|git| &git.commit),
            event.git.as_ref().map(|git| &git.tree),
            event.git.as_ref().map(|git| &git.checkout),
            sql_int(event.policy_version)?,
            payload,
            event.prev_hash.as_ref().map(|hash| &hash.0[..]),
            &event.hash.0[..],
        ],
    )
    .map_err(sql)?;
    Ok(())
}

/// What a decision's operations do, before their errors are noted.
impl Work<'_, '_> {
    fn get_staged(&self, view: &str, key: &DocKey) -> Result<Option<Document>, StoreError> {
        let table = self.store.views().table(view)?;
        if let Some(staged) = self.staged.get(view).and_then(|staged| staged.get(key)) {
            return Ok(staged.document(table, key));
        }
        get_document(self.tx, table, self.project_id(), self.generation, key)
    }

    fn find_staged(&self, view: &str, query: &IndexQuery) -> Result<Page<Document>, StoreError> {
        let table = self.store.views().table(view)?;
        let empty = BTreeMap::new();
        let staged = self.staged.get(view).unwrap_or(&empty);
        find_with_staged(
            self.tx,
            table,
            self.project_id(),
            self.generation,
            query,
            staged,
        )
    }

    fn exists(&self, matching: &EventMatch) -> Result<bool, StoreError> {
        let appended = self.events.iter().any(|(event, _)| {
            event.type_name == matching.type_name
                && matching
                    .stream
                    .as_ref()
                    .is_none_or(|stream| stream.0 == event.stream)
                && fields_match(&event.payload, matching)
        });
        Ok(appended || stored_event_exists(self.tx, self.project_id(), matching)?)
    }

    fn expect_version(&self, stream: &StreamName, version: u64) -> Result<(), StoreError> {
        let actual = self.stream_version(stream)?;
        if actual != version {
            return Err(StoreError::Stale(StaleInput::StreamVersion {
                stream: stream.clone(),
                expected: version,
                actual,
            }));
        }
        Ok(())
    }

    fn append_domain(&mut self, event: NewEvent) -> Result<u64, StoreError> {
        if store_owned(&event.type_name) {
            return Err(StoreError::Refused(Refusal::InvalidEvent(format!(
                "{} is recorded by the store, not by a decision",
                event.type_name
            ))));
        }
        self.push(event)
    }

    pub(crate) fn put(
        &mut self,
        bytes: &[u8],
        class: RetentionClass,
    ) -> Result<PayloadRef, StoreError> {
        let reference = put_payload(self.tx, bytes, class)?;
        self.put.push(reference.hash);
        Ok(reference)
    }

    /// The sequence the next store-owned event will take.
    pub(crate) fn next_seq(&self) -> u64 {
        self.head.as_ref().map_or(1, |head| head.seq + 1)
    }

    /// Stages an anchor row after checking it against the command's own
    /// `anchor.pushed` event and any row already stored for its sequence.
    fn stage_anchor(
        &mut self,
        anchor: &Anchor,
        tag: &str,
        remote: &str,
        observed_at: &str,
    ) -> Result<(), StoreError> {
        let invalid = |reason: String| StoreError::Refused(Refusal::InvalidEvent(reason));
        if tag != anchor_tag(self.project_id(), anchor.seq) {
            return Err(invalid(format!(
                "{tag} is not the anchor tag of sequence {} in this project",
                anchor.seq
            )));
        }
        let expected = AnchorPushedPayload {
            tag: tag.into(),
            seq: anchor.seq,
            head: anchor.hash,
            remote: remote.into(),
            observed_at: observed_at.into(),
        };
        let evidenced = self.events.iter().any(|(event, _)| {
            event.type_name == ANCHOR_PUSHED
                && event.type_version == ANCHOR_PUSHED_VERSION
                && event.stream == ANCHOR_STREAM
                && AnchorPushedPayload::from_value(&event.payload).as_ref() == Some(&expected)
        });
        if !evidenced {
            return Err(invalid(
                "an anchor row needs a matching anchor.pushed event in the same command".into(),
            ));
        }
        let row = StoredAnchor {
            anchor: anchor.clone(),
            tag: tag.into(),
            pushed_at: observed_at.into(),
        };
        let stored = self
            .tx
            .query_row(
                "SELECT head_hash, tag, pushed_at FROM anchor WHERE project_id = ?1 AND seq = ?2",
                params![self.project_id().0, sql_int(anchor.seq)?],
                |found| {
                    Ok((
                        found.get::<_, Vec<u8>>(0)?,
                        found.get::<_, String>(1)?,
                        found.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?
            .map(|(hash, tag, pushed_at)| {
                <[u8; 32]>::try_from(hash)
                    .map(|hash| StoredAnchor {
                        anchor: Anchor {
                            seq: anchor.seq,
                            hash: Hash(hash),
                        },
                        tag,
                        pushed_at,
                    })
                    .map_err(|_| {
                        StoreError::Unavailable("a stored anchor hash that is not 32 bytes".into())
                    })
            })
            .transpose()?;
        let earlier = stored.or_else(|| {
            self.anchors
                .iter()
                .find(|staged| staged.anchor.seq == anchor.seq)
                .cloned()
        });
        match earlier {
            // The same row again changes nothing.
            Some(earlier) if earlier == row => Ok(()),
            Some(_) => Err(invalid(format!(
                "an anchor row for sequence {} already records another hash, tag or time",
                anchor.seq
            ))),
            None => {
                self.anchors.push(row);
                Ok(())
            }
        }
    }
}

/// Every operation's error is noted, so a decision that goes on past one
/// still fails the command.
impl Transaction for Work<'_, '_> {
    fn get(&mut self, view: &str, key: &DocKey) -> Result<Option<Document>, StoreError> {
        let result = self.get_staged(view, key);
        self.noted(result)
    }

    fn find(&mut self, view: &str, query: &IndexQuery) -> Result<Page<Document>, StoreError> {
        let result = self.find_staged(view, query);
        self.noted(result)
    }

    fn event_exists(&mut self, matching: &EventMatch) -> Result<bool, StoreError> {
        let result = self.exists(matching);
        self.noted(result)
    }

    fn expect(&mut self, stream: &StreamName, version: u64) -> Result<(), StoreError> {
        let result = self.expect_version(stream, version);
        self.noted(result)
    }

    fn append(&mut self, event: NewEvent) -> Result<u64, StoreError> {
        let result = self.append_domain(event);
        self.noted(result)
    }

    fn put_payload(
        &mut self,
        bytes: &[u8],
        class: RetentionClass,
    ) -> Result<PayloadRef, StoreError> {
        let result = self.put(bytes, class);
        self.noted(result)
    }

    fn open_claims(&mut self) -> Result<Vec<Claim>, StoreError> {
        let result = open_claims_in(
            self.tx,
            self.store,
            self.project_id(),
            self.generation,
            Some(&self.staged),
        );
        self.noted(result)
    }

    fn head(&mut self) -> Result<Option<Head>, StoreError> {
        Ok(self.head.clone())
    }

    fn record_anchor(
        &mut self,
        anchor: &Anchor,
        tag: &str,
        remote: &str,
        observed_at: &str,
    ) -> Result<(), StoreError> {
        let result = self.stage_anchor(anchor, tag, remote, observed_at);
        self.noted(result)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::Read;
    use std::path::Path;
    use std::time::Duration;

    use std::num::NonZeroU32;

    use baley_store::{
        Actor, Caller, Change, CommandKind, EventSchema, FieldKind, FieldSpec, HistoryFilter,
        IndexField, IndexSpec, KeyValue, Ledger, Order, OutcomeKind, PayloadBody, Payloads,
        Projector, ProjectorError, RequestId, ServerCaller, ViewSpec, Views, verify_chain,
    };
    use rusqlite::{Connection, ErrorCode};
    use serde_json::json;

    use super::*;
    use crate::queue::scripted::Scripted;
    use crate::store::Options;

    const AT: &str = "2026-09-25T18:00:00Z";
    const PROJECT: &str = "7f0c2a4e-8d1b-4c3a-9e5f-2b6d8a1c4e70";
    const RECORDED: &str = "item.recorded";

    /// The core's side for these tests: one event type, `item.recorded`.
    struct Items;

    impl EventSchema for Items {
        fn reads(&self, type_name: &str, version: u32) -> bool {
            type_name == RECORDED && version == 1
        }
    }

    /// Keeps `item` documents, keyed by id and indexed by state. An item
    /// whose state is `broken` cannot be applied.
    struct ItemProjector(ViewSpec);

    impl ItemProjector {
        fn new() -> Self {
            Self(ViewSpec {
                name: "item".into(),
                version: 1,
                key: vec![FieldSpec {
                    name: "id".into(),
                    kind: FieldKind::Integer,
                }],
                indexes: vec![IndexSpec {
                    name: "by_state".into(),
                    fields: vec![IndexField {
                        name: "state".into(),
                        kind: FieldKind::Text,
                        order: Order::Ascending,
                    }],
                }],
                page_bound: 10,
            })
        }
    }

    impl Projector for ItemProjector {
        fn spec(&self) -> &ViewSpec {
            &self.0
        }

        fn handles(&self) -> &[&str] {
            &[RECORDED]
        }

        fn keys(&self, event: &Event) -> Vec<DocKey> {
            event.payload["id"]
                .as_i64()
                .map(|id| DocKey(vec![KeyValue::Integer(id)]))
                .into_iter()
                .collect()
        }

        fn apply(
            &self,
            event: &Event,
            _documents: &[(DocKey, Value)],
        ) -> Result<Vec<Change>, ProjectorError> {
            if event.payload["state"] == "broken" {
                return Err(ProjectorError("a broken item".into()));
            }
            Ok(vec![Change::Put {
                key: self.keys(event).remove(0),
                body: json!({ "id": event.payload["id"], "state": event.payload["state"] }),
            }])
        }
    }

    fn project() -> ProjectId {
        ProjectId(PROJECT.into())
    }

    /// A store with the item projector, holding the empty project. Set
    /// version 1 is `request` alone, so this set declares 2.
    fn open(home: &Path) -> SqliteStore {
        let options = Options {
            projectors: vec![Box::new(ItemProjector::new())],
            schema: Box::new(Items),
            view_set_version: NonZeroU32::new(3).expect("positive"),
            timing: Scripted::still(),
            ..Options::default()
        };
        let store = SqliteStore::open(home, AT, options).expect("open");
        store
            .write(|tx| {
                tx.execute(
                    "INSERT INTO project (project_id, name, created_at) VALUES (?1, 'one', ?2)",
                    params![PROJECT, AT],
                )
                .map_err(sql)?;
                Ok(())
            })
            .expect("project");
        store
    }

    /// A connection of the test's own, beside the store's.
    fn raw(home: &Path) -> Connection {
        Connection::open(home.join("baley.db")).expect("raw connection")
    }

    fn count(home: &Path, table: &str) -> i64 {
        raw(home)
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count")
    }

    fn command(kind: &str, request: &str, digest: u8) -> Command {
        Command {
            project: project(),
            kind: CommandKind(kind.into()),
            request_id: RequestId(request.into()),
            digest: Hash([digest; 32]),
            scope: Vec::new(),
            policy_version: 1,
            recorded_at: AT.into(),
            actor: Actor::Owner,
            caller: None,
        }
    }

    fn item(id: i64, state: &str) -> NewEvent {
        NewEvent {
            stream: StreamName(format!("item/{id}")),
            type_name: RECORDED.into(),
            type_version: 1,
            git: None,
            payload: json!({ "id": id, "state": state }),
            attachments: Vec::new(),
        }
    }

    fn done(answer: Value) -> Decision {
        Decision {
            kind: OutcomeKind::Done,
            answer,
            sensitive: false,
            observed: Observed::default(),
            git: None,
        }
    }

    /// Records the items under a fresh request and answers `"ok"`.
    fn record(store: &SqliteStore, request: &str, items: &[(i64, &str)]) -> Recorded {
        store
            .transact(&command("item.add", request, 1), &mut |tx| {
                for (id, state) in items {
                    tx.append(item(*id, state))?;
                }
                Ok(done(json!("ok")))
            })
            .expect("record")
    }

    fn item_key(id: i64) -> DocKey {
        DocKey(vec![KeyValue::Integer(id)])
    }

    fn server_caller() -> Caller {
        Caller::Server(
            ServerCaller::new(
                "/work/project",
                "/work/project/crates",
                "claude-code",
                PROJECT,
                &json!(7),
            )
            .expect("a valid server caller"),
        )
    }

    // The caller column holds the caller's canonical bytes on every event of
    // its command and NULL for a command without one, and history returns an
    // equal caller. Catches a caller dropped at `push` or at the insert, and
    // the text 'null' written for absence.
    #[test]
    fn the_caller_column_holds_the_canonical_bytes_or_null() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let caller = server_caller();
        let expected = String::from_utf8(canonical_json(&caller.to_value()).expect("canonical"))
            .expect("utf-8");
        let with = Command {
            caller: Some(caller.clone()),
            ..command("item.add", "r1", 1)
        };
        store
            .transact(&with, &mut |tx| {
                tx.append(item(1, "open"))?;
                tx.append(item(2, "open"))?;
                Ok(done(json!("ok")))
            })
            .expect("with a caller");
        record(&store, "r2", &[(3, "open")]);
        let stored: Vec<Option<String>> = raw(home.path())
            .prepare("SELECT caller FROM event ORDER BY seq")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        // The command's two items and its own completion event, then a caller-free
        // command's item and completion.
        let stamped = Some(expected);
        assert_eq!(
            stored,
            [stamped.clone(), stamped.clone(), stamped, None, None]
        );
        let page = store
            .history(
                &project(),
                1..=u64::MAX,
                &HistoryFilter::default(),
                PageRequest {
                    limit: 10,
                    after: None,
                },
            )
            .expect("history");
        let callers: Vec<Option<Caller>> = page.items.into_iter().map(|e| e.caller).collect();
        let stamped = Some(caller);
        assert_eq!(
            callers,
            [stamped.clone(), stamped.clone(), stamped, None, None]
        );
    }

    // Catches a replay fenced by a raw newer set stamp beside a current request view.
    #[test]
    fn a_replay_is_answered_beside_a_newer_view_set() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let Recorded::New { outcome, .. } = record(&store, "r1", &[(1, "open")]) else {
            panic!("new request")
        };
        raw(home.path())
            .execute(
                "UPDATE view_gen SET view_set_version = 4 WHERE project_id = ?1",
                [PROJECT],
            )
            .expect("newer set");
        assert_eq!(
            store.transact(&command("item.add", "r1", 1), &mut |_| panic!("replay ran")),
            Ok(Recorded::Replayed { outcome })
        );
    }

    // A decision expecting a stream at the version it read is refused once
    // the stream has moved. Catches `expect` that never compares.
    #[test]
    fn expect_refuses_a_stream_that_moved() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        record(&store, "r1", &[(1, "open")]);
        let stream = StreamName("item/1".into());
        let result = store.transact(&command("item.add", "r2", 1), &mut |tx| {
            tx.expect(&stream, 0)?;
            tx.append(item(1, "done"))?;
            Ok(done(json!("ok")))
        });
        assert_eq!(
            result,
            Err(StoreError::Stale(StaleInput::StreamVersion {
                stream: stream.clone(),
                expected: 0,
                actual: 1,
            }))
        );
    }

    // What the decision itself appends does not make it stale: it saw no
    // item 1 and records item 1. Catches a re-check against the command's
    // own writes, which would refuse every create-if-absent command.
    #[test]
    fn a_decision_is_not_stale_against_its_own_events() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let absence = Absence::Event(EventMatch {
            type_name: RECORDED.into(),
            stream: None,
            fields: BTreeMap::from([("id".into(), json!(1))]),
        });
        let result = store.transact(&command("item.add", "r1", 1), &mut |tx| {
            tx.append(item(1, "open"))?;
            Ok(Decision {
                observed: Observed {
                    absences: vec![absence.clone()],
                    ..Observed::default()
                },
                ..done(json!("ok"))
            })
        });
        assert!(matches!(result, Ok(Recorded::New { .. })), "{result:?}");
    }

    // While the decision runs, another connection cannot begin a write:
    // the transaction was opened before it. Catches a decision that runs
    // before `BEGIN IMMEDIATE`.
    #[test]
    fn the_write_transaction_is_open_while_the_decision_runs() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let other = raw(home.path());
        other.busy_timeout(Duration::ZERO).expect("no wait");
        let code = Cell::new(None);
        store
            .transact(&command("item.add", "r1", 1), &mut |_| {
                code.set(
                    other
                        .execute_batch("BEGIN IMMEDIATE")
                        .err()
                        .and_then(|error| error.sqlite_error_code()),
                );
                Ok(done(json!("ok")))
            })
            .expect("transact");
        assert_eq!(code.get(), Some(ErrorCode::DatabaseBusy));
    }

    /// The stored answer's bytes and the request document's answer.
    fn stored_answer(store: &SqliteStore, request: &str) -> (Vec<u8>, Value) {
        let document = store
            .get(
                &project(),
                REQUEST_VIEW,
                &DocKey(vec![
                    KeyValue::Text("item.add".into()),
                    KeyValue::Text(request.into()),
                ]),
            )
            .expect("get")
            .expect("request document");
        let reference =
            PayloadRef::from_value(&document.body["answer"]["stored"]).expect("a stored answer");
        let PayloadBody::Present(mut body) = store.open(&reference.hash).expect("open") else {
            panic!("the answer body is present");
        };
        let mut bytes = Vec::new();
        body.read_to_end(&mut bytes).expect("read");
        (bytes, document.body["answer"].clone())
    }

    // An answer past 4 KiB is stored as a `record` payload; the outcome
    // and the request document carry its reference, and the body is the
    // answer's canonical JSON. Catches a large answer written inline.
    #[test]
    fn an_answer_past_4_kib_is_stored_as_a_payload() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let answer = json!("x".repeat(4096));
        let recorded = store
            .transact(&command("item.add", "big", 1), &mut |_| {
                Ok(done(answer.clone()))
            })
            .expect("transact");
        let Recorded::New { outcome, .. } = recorded else {
            panic!("recorded");
        };
        let Answer::Stored(reference) = outcome.answer else {
            panic!("stored, not inline");
        };
        assert_eq!(reference.class, RetentionClass::Record);
        let (bytes, held) = stored_answer(&store, "big");
        assert_eq!(bytes, format!("\"{}\"", "x".repeat(4096)).into_bytes());
        assert_eq!(held, json!({ "stored": reference.to_value() }));
    }

    // A small answer marked sensitive is stored as a payload too. Catches
    // the sensitive flag ignored below the size limit.
    #[test]
    fn a_sensitive_answer_is_stored_as_a_payload() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        store
            .transact(&command("item.add", "secret", 1), &mut |_| {
                Ok(Decision {
                    sensitive: true,
                    ..done(json!("token"))
                })
            })
            .expect("transact");
        let (bytes, _) = stored_answer(&store, "secret");
        assert_eq!(bytes, b"\"token\"");
    }

    // Two events change one document in one command, and the document is
    // written once, with the second event's state and sequence. Catches a
    // projector written per event.
    #[test]
    fn each_changed_document_is_written_once() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        raw(home.path())
            .execute_batch(
                "CREATE TABLE writes (n INTEGER);
                 CREATE TRIGGER counted AFTER INSERT ON v_item_1
                 BEGIN INSERT INTO writes VALUES (1); END;",
            )
            .expect("trigger");
        record(&store, "r1", &[(1, "open"), (1, "done")]);
        assert_eq!(count(home.path(), "writes"), 1);
        let document = store
            .get(&project(), "item", &item_key(1))
            .expect("get")
            .expect("item 1");
        assert_eq!(
            (document.body["state"].clone(), document.produced_seq),
            (json!("done"), 2)
        );
    }

    // Inside the decision, `get` returns the document its own event just
    // changed, before anything is written. Catches reads that miss the
    // command's own writes.
    #[test]
    fn get_inside_a_decision_sees_its_own_events() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let seen = std::cell::RefCell::new(None);
        record(&store, "r1", &[(1, "open")]);
        store
            .transact(&command("item.add", "r2", 1), &mut |tx| {
                tx.append(item(1, "done"))?;
                *seen.borrow_mut() = tx.get("item", &item_key(1))?;
                Ok(done(json!("ok")))
            })
            .expect("transact");
        let document = seen.into_inner().expect("item 1");
        assert_eq!(
            (document.body["state"].clone(), document.produced_seq),
            (json!("done"), 3)
        );
    }

    // Inside the decision, `find` lays its own changes over the stored
    // documents: item 1 it added appears in order, item 3 it closed drops
    // out, stored item 2 stays. Catches a find that reads only stored rows.
    #[test]
    fn find_inside_a_decision_sees_its_own_events() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        record(&store, "r1", &[(2, "open"), (3, "open")]);
        let seen = std::cell::RefCell::new(Vec::new());
        store
            .transact(&command("item.add", "r2", 1), &mut |tx| {
                tx.append(item(1, "open"))?;
                tx.append(item(3, "done"))?;
                let page = tx.find(
                    "item",
                    &IndexQuery {
                        index: "by_state".into(),
                        equals: vec![KeyValue::Text("open".into())],
                        page: PageRequest {
                            limit: 10,
                            after: None,
                        },
                    },
                )?;
                *seen.borrow_mut() = page.items.into_iter().map(|found| found.key).collect();
                Ok(done(json!("ok")))
            })
            .expect("transact");
        assert_eq!(seen.into_inner(), vec![item_key(1), item_key(2)]);
    }

    /// Every stored event of the project, in sequence order.
    fn stored_events(home: &Path) -> Vec<Event> {
        let conn = raw(home);
        let mut statement = conn
            .prepare(
                "SELECT seq, stream, stream_version, type, type_version, actor, recorded_at,
                        request_id, policy_version, payload_json, prev_hash, hash
                   FROM event WHERE project_id = ?1 ORDER BY seq",
            )
            .expect("prepare");
        let rows = statement
            .query_map([PROJECT], |row| {
                let hash = |bytes: Vec<u8>| Hash(bytes.try_into().expect("32 bytes"));
                Ok(Event {
                    project_id: project(),
                    seq: row.get::<_, i64>(0)? as u64,
                    stream: row.get(1)?,
                    stream_version: row.get::<_, i64>(2)? as u64,
                    type_name: row.get(3)?,
                    type_version: row.get(4)?,
                    actor: Actor::parse(&row.get::<_, String>(5)?).expect("actor"),
                    caller: None,
                    recorded_at: row.get(6)?,
                    request_id: RequestId(row.get(7)?),
                    git: None,
                    policy_version: row.get::<_, i64>(8)? as u64,
                    payload: serde_json::from_str(&row.get::<_, String>(9)?).expect("json"),
                    prev_hash: row.get::<_, Option<Vec<u8>>>(10)?.map(hash),
                    hash: hash(row.get(11)?),
                })
            })
            .expect("events");
        rows.collect::<rusqlite::Result<_>>().expect("events")
    }

    // Two commands' events, read back as stored, form one intact chain
    // whose head is the one the second command reported, and the project
    // row holds it. Catches a chain restarted per command, a head that
    // does not move, and stored fields that differ from the hashed ones.
    #[test]
    fn a_commands_events_extend_the_chain_and_move_the_head() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        record(&store, "r1", &[(1, "open")]);
        let Recorded::New { head, .. } = record(&store, "r2", &[(2, "open")]) else {
            panic!("recorded");
        };
        let events = stored_events(home.path());
        let report = verify_chain(&events, None);
        assert!(report.is_intact(), "{report:?}");
        assert_eq!(report.head, Some(head.clone()));
        let stored: (i64, Vec<u8>) = raw(home.path())
            .query_row(
                "SELECT head_seq, head_hash FROM project WHERE project_id = ?1",
                [PROJECT],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("head");
        assert_eq!(stored, (head.seq as i64, head.hash.0.to_vec()));
    }

    // The decision ignores a failed `expect` and records anyway; the
    // command still fails with that error and nothing is recorded.
    // Catches an operation's error a decision can swallow into a commit.
    #[test]
    fn an_error_the_decision_goes_past_still_fails_the_command() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        record(&store, "r1", &[(1, "open")]);
        let events = count(home.path(), "event");
        let stream = StreamName("item/1".into());
        let result = store.transact(&command("item.add", "r2", 1), &mut |tx| {
            let _ignored = tx.expect(&stream, 0);
            tx.append(item(2, "open"))?;
            Ok(done(json!("ok")))
        });
        assert_eq!(
            result,
            Err(StoreError::Stale(StaleInput::StreamVersion {
                stream: stream.clone(),
                expected: 0,
                actual: 1,
            }))
        );
        assert_eq!(count(home.path(), "event"), events);
    }

    /// Plants, as the project's head, an event of a type this binary
    /// cannot read, the way a newer binary would have recorded it.
    fn plant_unreadable(home: &Path, seq: i64) {
        let conn = raw(home);
        let hash = vec![9u8; 32];
        conn.execute(
            "INSERT INTO event (project_id, seq, stream, stream_version, type, type_version,
               actor, recorded_at, request_id, policy_version, payload_json, hash)
             VALUES (?1, ?2, 'future', 1, 'item.future', 1, 'owner', ?3, 'planted', 1, '{}', ?4)",
            params![PROJECT, seq, AT, hash],
        )
        .expect("planted event");
        conn.execute(
            "UPDATE project SET head_seq = ?1, head_hash = ?2 WHERE project_id = ?3",
            params![seq, hash, PROJECT],
        )
        .expect("planted head");
    }

    // A project now holds an event this binary cannot read, and a retry of
    // a request it completed before still gets its outcome. Catches the
    // read-only fence applied to a replay, which only reads.
    #[test]
    fn a_replay_is_answered_on_a_project_this_binary_cannot_write() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let Recorded::New { outcome, .. } = record(&store, "r1", &[(1, "open")]) else {
            panic!("recorded");
        };
        plant_unreadable(home.path(), 3);
        let replay = store.transact(&command("item.add", "r1", 1), &mut |_| {
            Ok(done(json!("ran again")))
        });
        assert_eq!(replay, Ok(Recorded::Replayed { outcome }));
    }

    // The chain is rewritten under the checked head, as by a restore, to
    // one of the same length whose last event this binary cannot read; the
    // next write is refused. Catches a mark trusted by its sequence alone.
    #[test]
    fn a_chain_rewritten_under_the_checked_head_is_checked_again() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        record(&store, "r1", &[(1, "open")]);
        let conn = raw(home.path());
        let hash = vec![9u8; 32];
        conn.execute(
            "UPDATE event SET type = 'item.future', hash = ?1 WHERE project_id = ?2 AND seq = 2",
            params![hash, PROJECT],
        )
        .expect("rewritten event");
        conn.execute(
            "UPDATE project SET head_hash = ?1 WHERE project_id = ?2",
            params![hash, PROJECT],
        )
        .expect("rewritten head");
        let result = store.transact(&command("item.add", "r2", 1), &mut |tx| {
            tx.append(item(2, "open"))?;
            Ok(done(json!("ok")))
        });
        assert_eq!(
            result,
            Err(StoreError::Refused(Refusal::ProjectReadOnly {
                project: project(),
                reason: "item.future version 2 is not readable by this binary".into(),
            }))
        );
    }

    // The decision stores a payload and records an event that does not
    // attach it; the command is refused and the body is not kept. Catches
    // a body left outside the chain, where no purge reaches it.
    #[test]
    fn a_payload_no_event_attaches_is_refused() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let loose = Cell::new(None);
        let result = store.transact(&command("item.add", "r1", 1), &mut |tx| {
            loose.set(Some(tx.put_payload(b"loose", RetentionClass::Output)?.hash));
            tx.append(item(1, "open"))?;
            Ok(done(json!("ok")))
        });
        let hash = loose.get().expect("the payload was stored");
        assert_eq!(
            result,
            Err(StoreError::Refused(Refusal::UnattachedPayload(hash)))
        );
        assert_eq!(count(home.path(), "payload"), 0);
    }

    // An event carries a reference's three fields plus a fourth and lists
    // no attachment; the event is refused. Catches a near-reference passed
    // as plain JSON, so its body gets no reference row.
    #[test]
    fn an_object_with_a_references_fields_that_is_not_one_is_refused() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let mut reference = PayloadRef {
            hash: Hash([7; 32]),
            bytes: 5,
            class: RetentionClass::Output,
        }
        .to_value();
        reference["note"] = json!("x");
        let mut event = item(1, "open");
        event.payload["output"] = reference;
        let result = store.transact(&command("item.add", "r1", 1), &mut |tx| {
            tx.append(event.clone())?;
            Ok(done(json!("ok")))
        });
        assert_eq!(
            result,
            Err(StoreError::Refused(Refusal::InvalidEvent(
                "an object with a payload reference's fields is not a reference".into()
            )))
        );
    }

    // A stored event has a field whose name holds a NUL, and a match on
    // that field finds it. Catches a NUL name passed into a JSON path,
    // which SQLite ends at the NUL and rejects.
    #[test]
    fn event_exists_matches_a_field_name_holding_a_nul() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        let mut event = item(1, "open");
        event.payload["a\u{0}b"] = json!("x");
        store
            .transact(&command("item.add", "r1", 1), &mut |tx| {
                tx.append(event.clone())?;
                Ok(done(json!("ok")))
            })
            .expect("record");
        let matching = EventMatch {
            type_name: RECORDED.into(),
            stream: None,
            fields: BTreeMap::from([("a\u{0}b".into(), json!("x"))]),
        };
        let found = std::cell::RefCell::new(None);
        store
            .transact(&command("item.add", "r2", 1), &mut |tx| {
                *found.borrow_mut() = Some(tx.event_exists(&matching));
                Ok(done(json!("ok")))
            })
            .expect("transact");
        assert_eq!(found.into_inner(), Some(Ok(true)));
    }

    // A decision whose outcome depended on git names those facts, and
    // `command.completed` records them. Catches a completion that drops
    // the git facts a refusal on git state rests on.
    #[test]
    fn command_completed_records_the_decisions_git_facts() {
        let home = crate::checks::private_folder();
        let store = open(home.path());
        store
            .transact(&command("item.add", "r1", 1), &mut |_| {
                Ok(Decision {
                    git: Some(GitFacts {
                        commit: "c1".into(),
                        tree: "t1".into(),
                        checkout: "/work".into(),
                    }),
                    ..done(json!("ok"))
                })
            })
            .expect("transact");
        let git: (String, String, String) = raw(home.path())
            .query_row(
                "SELECT git_commit, git_tree, git_checkout FROM event WHERE type = ?1",
                [COMMAND_COMPLETED],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("command.completed");
        assert_eq!(git, ("c1".into(), "t1".into(), "/work".into()));
    }
}
