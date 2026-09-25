//! The write path of a database-only command (design 0001, Commands and
//! Figure 6; EVD-R5, R6, R7).
//!
//! A decision's events and document changes are held in memory until it
//! returns. Reads inside the decision see them; the stored rows stay as they
//! stood when the transaction began. So the re-check of what the caller's
//! slow work observed compares against the store before this command, never
//! against the command's own writes. Then, in the same transaction, the
//! events are written with their chain and references, each changed
//! document once, `command.completed` last, and the project's head moves.

use std::collections::BTreeMap;

use baley_store::{
    Absence, Answer, COMMAND_COMPLETED, COMMAND_COMPLETED_VERSION, Change, Claim, Command, Decide,
    Decision, DocKey, Document, Event, EventDraft, EventMatch, GitFact, GitFacts, Hash, Head,
    INLINE_ANSWER_LIMIT, IndexQuery, NewEvent, Observed, Outcome, PAYLOAD_PURGED, PAYLOAD_REDUCED,
    Page, PageRequest, PayloadRef, PayloadStatus, ProjectId, PurgedEvent, REQUEST_VIEW, Recorded,
    ReducedEvent, Refusal, RetentionClass, StaleInput, StoreError, StreamName, Transaction,
    UtcInstant, canonical_json, command_stream, completed_payload, recorded_outcome, request_key,
    store_owned,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{OptionalExtension, params, params_from_iter};
use serde_json::Value;

use crate::payload::{put_payload, put_reference, sql_int, stored};
use crate::store::{SqliteStore, sql};
use crate::view::{Staged, find_documents, find_with_staged, get_document, write_live};

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
    /// store-owned commands. The replay callback sees the recorded event sequence.
    pub(crate) fn command_path<T>(
        &self,
        command: &Command,
        replay: impl FnOnce(&rusqlite::Transaction<'_>, Outcome, u64) -> Result<T, StoreError>,
        new: impl FnOnce(
            &mut Work<'_, '_>,
        )
            -> Result<(T, Outcome, Option<GitFacts>, Option<Observed>), StoreError>,
    ) -> Result<CommandResult<T>, StoreError> {
        UtcInstant::parse(&command.recorded_at).map_err(|_| {
            StoreError::Refused(Refusal::InvalidEvent(
                "recorded_at is not a UTC instant".into(),
            ))
        })?;
        let recorded = self.write(|tx| {
            let project = &command.project;
            let head = stored_head(tx, project)?;
            let requests = self.views().table(REQUEST_VIEW)?;
            if let Some(document) = get_document(tx, requests, project, &request_key(command))? {
                let (digest, mut outcome) = recorded_outcome(&document.body).ok_or_else(|| {
                    StoreError::Unavailable("a request document that holds no outcome".into())
                })?;
                if digest != command.digest {
                    return Err(StoreError::Refused(Refusal::RequestDigestMismatch {
                        request_id: command.request_id.clone(),
                    }));
                }
                if let Answer::Stored(reference) = &outcome.answer {
                    let status =
                        reference_status(tx, project, document.produced_seq, &reference.hash)?;
                    if !matches!(status, PayloadStatus::Present { .. }) {
                        outcome.answer = Answer::Tombstone {
                            reference: reference.clone(),
                            status,
                        };
                    }
                }
                return replay(tx, outcome, document.produced_seq).map(CommandResult::Replayed);
            }
            // A replay above only reads; a project this binary cannot read
            // still answers it. A new write is fenced here.
            self.check_readable(tx, project, head.as_ref())?;
            let mut work = Work::new(self, tx, command, head);
            let (result, outcome, git, observed) = new(&mut work)?;
            // An operation that failed fails the command, even when the
            // decision went on past the error.
            if let Some(error) = work.failed.take() {
                return Err(error);
            }
            work.check_payloads_attached()?;
            if let Some(observed) = observed {
                recheck(self, tx, project, &observed)?;
            }
            work.complete(&outcome, git)?;
            let head = work.write()?;
            Ok(CommandResult::New(result, head))
        })?;
        if let CommandResult::New(_, head) = &recorded {
            // Committed: every event through the new head was checked
            // readable or was written here by this binary.
            self.readable()
                .insert(command.project.clone(), head.clone());
        }
        Ok(recorded)
    }

    /// Refuses a project holding an event this binary cannot read. Checks
    /// only the events recorded since the last check, unless the chain no
    /// longer holds the checked head, as after a restore.
    fn check_readable(
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
fn stored_head(
    tx: &rusqlite::Transaction<'_>,
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
fn recheck(
    store: &SqliteStore,
    tx: &rusqlite::Transaction<'_>,
    project: &ProjectId,
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
        let now = get_document(tx, table, project, &document.key)?.map(|found| found.produced_seq);
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
                !find_documents(tx, table, project, &query)?.items.is_empty()
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
    events: Vec<(Event, Vec<PayloadRef>)>,
    /// Each stream's version after the events appended so far.
    streams: BTreeMap<StreamName, u64>,
    /// Per view, the documents the appended events changed.
    staged: BTreeMap<String, BTreeMap<DocKey, Staged>>,
    /// Every payload the decision stored, each of which an event must
    /// attach.
    put: Vec<Hash>,
    /// The first error any operation returned, which fails the command.
    failed: Option<StoreError>,
}

impl<'s, 't> Work<'s, 't> {
    fn new(
        store: &'s SqliteStore,
        tx: &'s rusqlite::Transaction<'t>,
        command: &'s Command,
        head: Option<Head>,
    ) -> Self {
        Self {
            store,
            tx,
            command,
            head,
            events: Vec::new(),
            streams: BTreeMap::new(),
            staged: BTreeMap::new(),
            put: Vec::new(),
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
    fn check_payloads_attached(&self) -> Result<(), StoreError> {
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

    /// Folds one event into the staged documents of every projector that
    /// handles its type.
    fn apply_projectors(&mut self, event: &Event) -> Result<(), StoreError> {
        let store = self.store;
        for projector in store.projectors() {
            if !projector.handles().contains(&event.type_name.as_str()) {
                continue;
            }
            let view = projector.spec().name.clone();
            let table = store.views().table(&view)?;
            let mut documents = Vec::new();
            for key in projector.keys(event) {
                let body = match self.staged.get(&view).and_then(|staged| staged.get(&key)) {
                    Some(staged) => staged.body.clone(),
                    None => get_document(self.tx, table, self.project_id(), &key)?
                        .map(|document| document.body),
                };
                if let Some(body) = body {
                    documents.push((key, body));
                }
            }
            let changes =
                projector
                    .apply(event, &documents)
                    .map_err(|error| StoreError::Projector {
                        view: view.clone(),
                        seq: event.seq,
                        message: error.0,
                    })?;
            let staged = self.staged.entry(view).or_default();
            for change in changes {
                let (key, body) = match change {
                    Change::Put { key, body } => (key, Some(body)),
                    Change::Delete { key } => (key, None),
                };
                staged.insert(
                    key,
                    Staged {
                        body,
                        produced_seq: event.seq,
                    },
                );
            }
        }
        Ok(())
    }

    /// The outcome the caller receives: the answer inline when it is small
    /// and not sensitive, otherwise stored as a `record` payload.
    fn outcome(&mut self, decision: &Decision) -> Result<Outcome, StoreError> {
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

    /// Appends `command.completed` for the outcome, with the git facts it
    /// depended on.
    fn complete(&mut self, outcome: &Outcome, git: Option<GitFacts>) -> Result<(), StoreError> {
        let attachments = match &outcome.answer {
            Answer::Stored(reference) | Answer::Tombstone { reference, .. } => {
                vec![reference.clone()]
            }
            Answer::Inline(_) => Vec::new(),
        };
        self.push(NewEvent {
            stream: command_stream(&self.command.kind),
            type_name: COMMAND_COMPLETED.into(),
            type_version: COMMAND_COMPLETED_VERSION,
            git,
            payload: completed_payload(self.command, outcome),
            attachments,
        })?;
        Ok(())
    }

    /// Writes the events, their references, each staged document once and
    /// the new head.
    fn write(self) -> Result<Head, StoreError> {
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
        for (view, documents) in &self.staged {
            let table = self.store.views().table(view)?;
            for (key, staged) in documents {
                let change = match &staged.body {
                    Some(body) => Change::Put {
                        key: key.clone(),
                        body: body.clone(),
                    },
                    None => Change::Delete { key: key.clone() },
                };
                write_live(self.tx, table, project, &change, staged.produced_seq)?;
            }
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
    tx.execute(
        "INSERT INTO event (project_id, seq, stream, stream_version, type, type_version, actor,
           recorded_at, request_id, git_commit, git_tree, git_checkout, policy_version,
           payload_json, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            event.project_id.0,
            sql_int(event.seq)?,
            event.stream,
            sql_int(event.stream_version)?,
            event.type_name,
            event.type_version,
            event.actor.as_str(),
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
        get_document(self.tx, table, self.project_id(), key)
    }

    fn find_staged(&self, view: &str, query: &IndexQuery) -> Result<Page<Document>, StoreError> {
        let table = self.store.views().table(view)?;
        let empty = BTreeMap::new();
        let staged = self.staged.get(view).unwrap_or(&empty);
        find_with_staged(self.tx, table, self.project_id(), query, staged)
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

    /// Claims arrive with Build 1's tenth task; until then no claim can
    /// exist, and saying so is an error rather than an empty list a
    /// decision would trust.
    fn open_claims(&mut self) -> Result<Vec<Claim>, StoreError> {
        let result = Err(StoreError::Unavailable("claims are not built yet".into()));
        self.noted(result)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::Read;
    use std::path::Path;
    use std::time::Duration;

    use baley_store::{
        Actor, CommandKind, EventSchema, FieldKind, FieldSpec, GitObservation, IndexField,
        IndexSpec, KeyValue, ObservedDocument, Order, OutcomeKind, PayloadBody, Payloads,
        Projector, ProjectorError, RequestId, ViewSpec, Views, verify_chain,
    };
    use rusqlite::{Connection, ErrorCode};
    use serde_json::json;

    use super::*;
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

    /// A store with the item projector, holding the empty project.
    fn open(home: &Path) -> SqliteStore {
        let options = Options {
            projectors: vec![Box::new(ItemProjector::new())],
            schema: Box::new(Items),
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
            policy_version: 1,
            recorded_at: AT.into(),
            actor: Actor::Owner,
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

    /// A command that appends nothing and depends on `observed`.
    fn observing(store: &SqliteStore, observed: Observed) -> Result<Recorded, StoreError> {
        store.transact(&command("item.check", "check", 1), &mut |_| {
            Ok(Decision {
                observed: observed.clone(),
                ..done(json!("ok"))
            })
        })
    }

    fn item_key(id: i64) -> DocKey {
        DocKey(vec![KeyValue::Integer(id)])
    }

    // A projector fails on the decision's event, the decision ignores the
    // error and answers, and still nothing is left: no event, payload,
    // reference, document or request answer. Catches a partial commit and
    // a projector failure a decision can swallow.
    #[test]
    fn a_failing_projector_leaves_nothing_behind() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let result = store.transact(&command("item.add", "r1", 1), &mut |tx| {
            let output = tx.put_payload(b"output", RetentionClass::Output)?;
            let mut event = item(1, "broken");
            event.payload["output"] = output.to_value();
            event.attachments.push(output);
            let _ignored = tx.append(event);
            Ok(done(json!("ok")))
        });
        assert!(matches!(
            result,
            Err(StoreError::Projector { view, seq: 1, .. }) if view == "item"
        ));
        for table in ["event", "payload", "payload_ref", "v_item_1", "v_request_1"] {
            assert_eq!(count(home.path(), table), 0, "{table}");
        }
    }

    // The same request again returns the first outcome without running
    // the decision or recording anything. Catches a retry that records
    // twice.
    #[test]
    fn a_replayed_request_returns_its_outcome_and_records_nothing() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let runs = Cell::new(0);
        let mut decide = |tx: &mut dyn Transaction| {
            runs.set(runs.get() + 1);
            tx.append(item(1, "open"))?;
            Ok(done(json!({ "added": 1 })))
        };
        let first = store
            .transact(&command("item.add", "r1", 1), &mut decide)
            .expect("first");
        let Recorded::New { outcome, .. } = first else {
            panic!("the first delivery records");
        };
        let events = count(home.path(), "event");
        let second = store.transact(&command("item.add", "r1", 1), &mut decide);
        assert_eq!(second, Ok(Recorded::Replayed { outcome }));
        assert_eq!(runs.get(), 1);
        assert_eq!(count(home.path(), "event"), events);
    }

    // The same request id and kind with another digest is refused and
    // records nothing. Catches a retry matched on the id alone.
    #[test]
    fn the_same_request_with_another_digest_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        record(&store, "r1", &[(1, "open")]);
        let events = count(home.path(), "event");
        let result = store.transact(&command("item.add", "r1", 2), &mut |tx| {
            tx.append(item(2, "open"))?;
            Ok(done(json!("ok")))
        });
        assert_eq!(
            result,
            Err(StoreError::Refused(Refusal::RequestDigestMismatch {
                request_id: RequestId("r1".into())
            }))
        );
        assert_eq!(count(home.path(), "event"), events);
    }

    // Two command kinds with one request id are two requests: the second
    // runs and gets its own answer. Catches request answers keyed without
    // the command kind.
    #[test]
    fn two_command_kinds_do_not_share_a_request_id() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        record(&store, "shared", &[]);
        let other = store
            .transact(&command("item.remove", "shared", 1), &mut |_| {
                Ok(done(json!("removed")))
            })
            .expect("other kind");
        assert!(matches!(
            other,
            Recorded::New { outcome, .. } if outcome.answer == Answer::Inline(json!("removed"))
        ));
    }

    // A decision expecting a stream at the version it read is refused once
    // the stream has moved. Catches `expect` that never compares.
    #[test]
    fn expect_refuses_a_stream_that_moved() {
        let home = tempfile::tempdir().expect("temp dir");
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

    // A document the caller saw at sequence 1 is at sequence 3 now, so the
    // command is stale. Catches a stale check that compares the wrong
    // sequence or none.
    #[test]
    fn a_document_seen_at_an_older_sequence_is_stale() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        record(&store, "r1", &[(1, "open")]);
        record(&store, "r2", &[(1, "done")]);
        let observed = Observed {
            documents: vec![ObservedDocument {
                view: "item".into(),
                key: item_key(1),
                produced_seq: Some(1),
            }],
            ..Observed::default()
        };
        assert_eq!(
            observing(&store, observed),
            Err(StoreError::Stale(StaleInput::Document {
                view: "item".into(),
                key: item_key(1),
                seen: Some(1),
                now: Some(3),
            }))
        );
    }

    // An event the caller saw absent now exists, so the command is stale.
    // Catches an event absence that is never re-checked.
    #[test]
    fn an_event_absence_that_no_longer_holds_is_stale() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        record(&store, "r1", &[(1, "open")]);
        let absence = Absence::Event(EventMatch {
            type_name: RECORDED.into(),
            stream: None,
            fields: BTreeMap::from([("id".into(), json!(1))]),
        });
        let observed = Observed {
            absences: vec![absence.clone()],
            ..Observed::default()
        };
        assert_eq!(
            observing(&store, observed),
            Err(StoreError::Stale(StaleInput::Absence(absence)))
        );
    }

    // No open item was seen, and now one exists, so the command is stale.
    // Catches a document absence that is never re-checked.
    #[test]
    fn a_document_absence_that_no_longer_holds_is_stale() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        record(&store, "r1", &[(1, "open")]);
        let absence = Absence::Documents {
            view: "item".into(),
            index: "by_state".into(),
            equals: vec![KeyValue::Text("open".into())],
        };
        let observed = Observed {
            absences: vec![absence.clone()],
            ..Observed::default()
        };
        assert_eq!(
            observing(&store, observed),
            Err(StoreError::Stale(StaleInput::Absence(absence)))
        );
    }

    // The HEAD the caller saw differs from the HEAD read again inside the
    // transaction, so the command is stale. Catches git facts that are
    // recorded but never compared.
    #[test]
    fn a_moved_head_is_stale() {
        let home = tempfile::tempdir().expect("temp dir");
        let store = open(home.path());
        let observed = Observed {
            git: vec![GitObservation {
                checkout: "/work".into(),
                head_seen: "aaa".into(),
                head_now: "bbb".into(),
                index_seen: "idx".into(),
                index_now: "idx".into(),
            }],
            ..Observed::default()
        };
        assert_eq!(
            observing(&store, observed),
            Err(StoreError::Stale(StaleInput::Git {
                checkout: "/work".into(),
                fact: GitFact::Head,
                seen: "aaa".into(),
                now: "bbb".into(),
            }))
        );
    }

    // What the decision itself appends does not make it stale: it saw no
    // item 1 and records item 1. Catches a re-check against the command's
    // own writes, which would refuse every create-if-absent command.
    #[test]
    fn a_decision_is_not_stale_against_its_own_events() {
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
                reason: "item.future version 1 is not readable by this binary".into(),
            }))
        );
    }

    // The decision stores a payload and records an event that does not
    // attach it; the command is refused and the body is not kept. Catches
    // a body left outside the chain, where no purge reaches it.
    #[test]
    fn a_payload_no_event_attaches_is_refused() {
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
        let home = tempfile::tempdir().expect("temp dir");
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
