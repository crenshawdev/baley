//! The scope records of design 0004 over the storage port: a start
//! submission recorded as a draft, the start-once read every scope write
//! is checked against first, the draft preview read by digest, the reads of
//! accepted scope, and the owner's approval of a draft by digest, which only
//! the command line gives.

/// Records one start submission as a draft in its own domain transaction.
pub mod submit;

/// Reads a draft back by its digest, and accepted scope from its views.
pub mod read;

/// Approves a draft by its digest under the owner: the judgement first, then the records.
pub mod approve;

/// The owner's `baley scope` commands.
pub mod command;

#[cfg(test)]
pub(crate) mod fixtures {
    //! The submissions, times and store the scope tests share. The
    //! submissions are written out as design 0004's start shape, never
    //! produced by the code under test.

    use baley_core::scope::{PROJECT_STREAM, SCOPE_APPROVED, approved_payload, judge_submission};
    use baley_store::{
        Actor, Admin, Answer, Command, CommandKind, Decision, Event, Hash, HistoryFilter, Ledger,
        NewEvent, Observed, OutcomeKind, PageRequest, ProjectId, Recorded, RequestId, ServerCaller,
        StoreError, StreamName,
    };
    use baley_store_sqlite::SqliteStore;
    use serde_json::{Value, json};

    use crate::mcp::prepare::{WriteRequest, prepared_command};
    use crate::scope::submit::SUBMIT_COMMAND;

    pub const T0: &str = "2026-10-13T09:00:00Z";
    pub const T1: &str = "2026-10-13T09:00:01Z";
    pub const T2: &str = "2026-10-13T09:00:02Z";
    pub const T3: &str = "2026-10-13T09:00:03Z";
    pub const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    /// A second project, for reads that must not see another project's purge.
    pub const ID_B: &str = "7a2d3b5f-9c0e-4f1a-8b2c-3d4e5f6a7b8c";
    pub const SESSION: &str = "0b7e4a52-3c1d-4f6a-8e9b-1a2b3c4d5e6f";
    pub const R1: &str = "9d0c1b7e-2f4a-4b6c-8d1e-3a5b7c9d0e2f";
    pub const R2: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    pub const R3: &str = "3c4d5e6f-7a8b-4c9d-8e0f-1a2b3c4d5e6f";
    pub const R5: &str = "7a8b9c0d-1e2f-4a3b-9c4d-5e6f7a8b9c0d";

    /// The digest of M's canonical JSON, from `sha256sum` over those bytes.
    pub const M_DIGEST: &str = "1b11196b8f9f4ffd5c08a4d97643f80867194c5e1a6d8cc51d2929f8db38d68e";
    /// M's canonical JSON, 202 bytes.
    pub const M_CANONICAL: &str = r#"{"description":{"constraints":[],"core_value":"v","what":"w"},"roadmap":[{"depends_on":[],"detail":"d","goal":"g","name":"n","stories":["s"]}],"stories":[{"label":"s","sentence":"t","status":"active"}]}"#;

    /// The minimal start.
    pub fn m() -> Value {
        json!({"description": {"what": "w", "core_value": "v", "constraints": []},
            "stories": [{"label": "s", "sentence": "t", "status": "active"}],
            "roadmap": [{"name": "n", "goal": "g", "detail": "d", "depends_on": [],
                "stories": ["s"]}]})
    }

    /// Three stories, one deferred, and two phases, the second depending on
    /// the first.
    pub fn s1() -> Value {
        json!({"description": {"what": "A ledger for agent work",
                "core_value": "Nothing moves on an unproven claim",
                "constraints": ["One binary"]},
            "stories": [
                {"label": "record", "sentence": "Every step is recorded", "status": "active"},
                {"label": "approve", "sentence": "The owner approves scope", "status": "active"},
                {"label": "later", "sentence": "Teammates see the backlog", "status": "deferred"}],
            "roadmap": [
                {"name": "Records", "goal": "Steps are recorded",
                    "detail": "The ledger and its views", "depends_on": [],
                    "stories": ["record"]},
                {"name": "Approval", "goal": "The owner approves",
                    "detail": "Approval by digest", "depends_on": [1],
                    "stories": ["approve"]}]})
    }

    /// S1 with the second phase's goal changed.
    pub fn s2() -> Value {
        let mut s2 = s1();
        s2["roadmap"][1]["goal"] = json!("The owner approves by digest");
        s2
    }

    pub fn project() -> ProjectId {
        ProjectId(ID.into())
    }

    pub fn project_b() -> ProjectId {
        ProjectId(ID_B.into())
    }

    /// A real store in a fresh temporary directory, holding the project
    /// created at T0.
    pub fn store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::ledger::open::store(
            &dir.path().join("home"),
            T0,
            crate::ledger::open::options(),
        )
        .unwrap();
        store.create_project(&project(), "sample", T0).unwrap();
        (dir, store)
    }

    pub fn caller() -> ServerCaller {
        ServerCaller::new("/real/r", "/real/r/p", "claude-code", SESSION, &json!(7)).unwrap()
    }

    /// A real store holding both the project and the second project.
    pub fn two_projects() -> (tempfile::TempDir, SqliteStore) {
        let (dir, store) = store();
        store.create_project(&project_b(), "other", T0).unwrap();
        (dir, store)
    }

    /// The command preparation makes for a write of `kind` under
    /// `request_id`, with a digest of `digest` bytes, at `at`.
    pub fn command(kind: &str, request_id: &str, digest: u8, at: &str) -> Command {
        command_in(&project(), kind, request_id, digest, at)
    }

    /// `command` for a write in `project`.
    pub fn command_in(
        project: &ProjectId,
        kind: &str,
        request_id: &str,
        digest: u8,
        at: &str,
    ) -> Command {
        let request = WriteRequest {
            kind: CommandKind(kind.into()),
            request_id: RequestId(request_id.into()),
            digest: Hash([digest; 32]),
        };
        prepared_command(project, &request, 0, at, &caller())
    }

    /// Records `value`, judged whole, as a submission under `request_id`
    /// with a write digest of `digest` bytes, at `at`.
    pub fn submit(
        store: &SqliteStore,
        value: &Value,
        request_id: &str,
        digest: u8,
        at: &str,
    ) -> Result<Recorded, StoreError> {
        submit_in(store, &project(), value, request_id, digest, at)
    }

    /// `submit` in `project`.
    pub fn submit_in(
        store: &SqliteStore,
        project: &ProjectId,
        value: &Value,
        request_id: &str,
        digest: u8,
        at: &str,
    ) -> Result<Recorded, StoreError> {
        let submission = judge_submission(value).unwrap();
        crate::scope::submit::record(
            store,
            &command_in(project, SUBMIT_COMMAND, request_id, digest, at),
            &submission,
        )
    }

    /// Releases `project`'s references to `hash` with `reason`, under a
    /// purge command of the owner's at T3.
    pub fn purge_in(store: &SqliteStore, project: &ProjectId, hash: Hash, reason: &str) {
        let purge = Command {
            project: project.clone(),
            kind: CommandKind("payload.purge".into()),
            request_id: RequestId("5e6f7a8b-9c0d-4e1f-8a2b-3c4d5e6f7a8b".into()),
            digest: Hash([9; 32]),
            scope: Vec::new(),
            policy_version: 0,
            recorded_at: T3.into(),
            actor: Actor::Owner,
            caller: None,
        };
        store.purge(&purge, &[hash], reason).unwrap();
    }

    /// The inline answer a command recorded or replayed.
    pub fn receipt(recorded: &Recorded) -> Value {
        let (Recorded::New { outcome, .. } | Recorded::Replayed { outcome }) = recorded;
        match &outcome.answer {
            Answer::Inline(value) => value.clone(),
            other => panic!("expected an inline answer, got {other:?}"),
        }
    }

    pub fn history(store: &SqliteStore) -> Vec<Event> {
        let mut events = Vec::new();
        let mut after = None;
        loop {
            let page = store
                .history(
                    &project(),
                    1..=u64::MAX,
                    &HistoryFilter::default(),
                    PageRequest { limit: 100, after },
                )
                .unwrap();
            events.extend(page.items);
            match page.next {
                Some(next) => after = Some(next),
                None => return events,
            }
        }
    }

    pub fn of_type<'a>(events: &'a [Event], type_name: &str) -> Vec<&'a Event> {
        events
            .iter()
            .filter(|event| event.type_name == type_name)
            .collect()
    }

    pub fn head(store: &SqliteStore) -> u64 {
        store.head(&project()).unwrap().map_or(0, |head| head.seq)
    }

    /// Approves the draft with `digest` as the owner `Ada Owner` at T3,
    /// through the approval's two steps.
    pub fn approve_draft(store: &SqliteStore, digest: Hash) {
        approve_draft_in(store, &project(), digest);
    }

    /// `approve_draft` in `project`.
    pub fn approve_draft_in(store: &SqliteStore, project: &ProjectId, digest: Hash) {
        let judged = crate::scope::approve::judge(store, project, digest).unwrap();
        let command = command_in(project, crate::scope::approve::APPROVE_COMMAND, R5, 5, T3);
        crate::scope::approve::record(store, &command, Some(&judged), "Ada Owner").unwrap();
    }

    /// Appends a `scope.approved` of kind `start` under a test command of
    /// its own, as an approval would leave it.
    pub fn approve_a_start(store: &SqliteStore, at: &str) {
        let command = Command {
            project: project(),
            kind: CommandKind("test.approve".into()),
            request_id: RequestId("2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e".into()),
            digest: Hash([0xee; 32]),
            scope: Vec::new(),
            policy_version: 0,
            recorded_at: at.into(),
            actor: Actor::Owner,
            caller: None,
        };
        store
            .transact(&command, &mut |tx| {
                tx.append(NewEvent {
                    stream: StreamName(PROJECT_STREAM.into()),
                    type_name: SCOPE_APPROVED.into(),
                    type_version: 1,
                    git: None,
                    payload: approved_payload(Hash([0x5c; 32]), "start", "Ada Owner", at),
                    attachments: Vec::new(),
                })?;
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            })
            .unwrap();
    }
}
