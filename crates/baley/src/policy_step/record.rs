//! The recorder: one `policy.effective` as Baley's own command, only when the
//! policy differs from the one stored for its checkout and host (design 0003
//! section 6, CFG-R9).

use baley_core::policy::recorded::{
    POLICY_EFFECTIVE, POLICY_EFFECTIVE_VERSION, POLICY_VIEW, PolicyJudgement, RecordedPolicy,
    effective_payload, judge_policy, policy_key,
};
use baley_store::{
    Actor, Command, CommandKind, Decision, DocKey, Ledger, NewEvent, Observed, ObservedDocument,
    OutcomeKind, ProjectId, Recorded, Refusal, RequestId, StoreError, StreamName, Views,
    request_digest,
};
use serde_json::{Value, json};

use crate::init::PROJECT_STREAM;

/// The command kind the policy step records under.
pub const RECORD_COMMAND: &str = "policy.record";

/// Tries in all before a record that keeps moving is returned as stale.
const ATTEMPTS: usize = 3;

/// Records `recorded` for `project` at `catalog_version` when it differs
/// from the stored record of its checkout and host, and returns the version
/// in force: the appended event's sequence, or the stored record's.
///
/// A match opens no command, so not even `command.completed` is recorded.
/// The command's policy version is the version its event replaces, fixed
/// before the transaction, so the observed document makes the store refuse
/// it as stale when another record of the key lands first. The recorder then
/// observes again and decides afresh, at most three times in all. It never
/// shares a transaction with a caller's events.
pub fn record(
    store: &(impl Ledger + Views),
    project: &ProjectId,
    recorded: &RecordedPolicy,
    catalog_version: u64,
    request_id: RequestId,
    at: &str,
) -> Result<u64, StoreError> {
    let payload = effective_payload(recorded, &project.0, catalog_version);
    let key = policy_key(&recorded.checkout, recorded.host);
    let mut attempt = 1;
    loop {
        // A stale refusal records nothing, so the request id is still free.
        match record_once(store, project, &key, &payload, &request_id, at) {
            Err(StoreError::Stale(_)) if attempt < ATTEMPTS => attempt += 1,
            result => return result,
        }
    }
}

fn record_once(
    store: &(impl Ledger + Views),
    project: &ProjectId,
    key: &DocKey,
    payload: &Value,
    request_id: &RequestId,
    at: &str,
) -> Result<u64, StoreError> {
    let stored = store.get(project, POLICY_VIEW, key)?;
    let body = stored.as_ref().map(|document| &document.body);
    if let PolicyJudgement::Unchanged { version } = judge_policy(payload, body) {
        return Ok(version);
    }
    let replaces = body.map_or(0, stored_version);
    let command = command(project, payload, replaces, request_id.clone(), at)?;
    let observed = Observed {
        documents: vec![ObservedDocument {
            view: POLICY_VIEW.into(),
            key: key.clone(),
            produced_seq: stored.as_ref().map(|document| document.produced_seq),
        }],
        ..Observed::default()
    };
    let mut in_force = None;
    let result = store.transact(&command, &mut |tx| {
        let now = tx.get(POLICY_VIEW, key)?;
        let judged = judge_policy(payload, now.as_ref().map(|document| &document.body));
        let appended = judged == PolicyJudgement::Record;
        in_force = Some(match judged {
            PolicyJudgement::Unchanged { version } => version,
            PolicyJudgement::Record => tx.append(NewEvent {
                stream: StreamName(PROJECT_STREAM.into()),
                type_name: POLICY_EFFECTIVE.into(),
                type_version: POLICY_EFFECTIVE_VERSION,
                git: None,
                payload: payload.clone(),
                attachments: vec![],
            })?,
        });
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({ "recorded": appended }),
            sensitive: false,
            observed: observed.clone(),
            git: None,
        })
    })?;
    match (result, in_force) {
        (Recorded::New { .. }, Some(version)) => Ok(version),
        // The same request already ran, so its record is the stored one.
        _ => Ok(store
            .get(project, POLICY_VIEW, key)?
            .map_or(0, |document| stored_version(&document.body))),
    }
}

fn stored_version(body: &Value) -> u64 {
    body.get("version").and_then(Value::as_u64).unwrap_or(0)
}

/// The step's command. The payload is in the digest, so two different
/// policies never share an identity.
fn command(
    project: &ProjectId,
    payload: &Value,
    policy_version: u64,
    request_id: RequestId,
    at: &str,
) -> Result<Command, StoreError> {
    let actor = Actor::Baley;
    let digest = request_digest(&json!({
        "kind": RECORD_COMMAND,
        "project": project.0,
        "actor": actor.as_str(),
        "policy_version": policy_version,
        "payload": payload,
        "scope": [],
    }))
    .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
    Ok(Command {
        project: project.clone(),
        kind: CommandKind(RECORD_COMMAND.into()),
        request_id,
        digest,
        scope: vec![],
        policy_version,
        recorded_at: at.into(),
        actor,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ops::RangeInclusive;
    use std::path::Path;

    use baley_core::policy::recorded::recorded_policy;
    use baley_core::policy::{Host, Schema, SettingsFile, effective_policy};
    use baley_store::{
        Admin, Anchor, Claim, ClaimId, ClaimOwner, Claimed, Decide, DecideClaim, DecideReconcile,
        Document, Event, Head, HistoryFilter, IndexQuery, Page, PageRequest, ReconcileAuthority,
        VerifyReport,
    };
    use baley_store_sqlite::SqliteStore;

    use super::*;

    const T0: &str = "2026-10-01T10:00:00Z";
    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const CHECKOUT: &str = "/w/r";
    const CATALOG: u64 = 5;

    fn at(n: u8) -> String {
        format!("2026-10-01T10:00:{n:02}Z")
    }

    fn request(n: u8) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-0000000000{n:02}"))
    }

    fn project() -> ProjectId {
        ProjectId(ID.into())
    }

    /// A real store in a fresh temporary directory, holding the project.
    fn store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
        store.create_project(&project(), "sample", T0).unwrap();
        (dir, store)
    }

    fn global(text: &str) -> SettingsFile {
        crate::settings::file(Path::new("/c/config.toml"), text.as_bytes().to_vec())
    }

    fn policy(checkout: &str, host: Option<Host>, text: &str) -> RecordedPolicy {
        let file = global(text);
        let policy = effective_policy(Schema::standard(), host, Some(&file), None).unwrap();
        recorded_policy(Path::new(checkout), &policy).unwrap()
    }

    fn escalates(on: bool) -> RecordedPolicy {
        policy(CHECKOUT, None, &format!("escalate_on_failure = {on}\n"))
    }

    fn events(store: &SqliteStore, stream: &str) -> Vec<Event> {
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        store
            .stream(&project(), &StreamName(stream.into()), 1, page)
            .unwrap()
            .items
    }

    fn effective(store: &SqliteStore) -> Vec<Event> {
        events(store, PROJECT_STREAM)
            .into_iter()
            .filter(|event| event.type_name == POLICY_EFFECTIVE)
            .collect()
    }

    fn document(store: &SqliteStore, recorded: &RecordedPolicy) -> Document {
        let key = policy_key(&recorded.checkout, recorded.host);
        store.get(&project(), POLICY_VIEW, &key).unwrap().unwrap()
    }

    fn head(store: &SqliteStore) -> u64 {
        store.head(&project()).unwrap().unwrap().seq
    }

    #[test]
    fn the_first_record_is_not_off_version_0_the_baley_actor_or_the_project_stream() {
        let (_dir, store) = store();
        let recorded = escalates(true);

        let version = record(&store, &project(), &recorded, CATALOG, request(1), &at(1)).unwrap();

        let appended = effective(&store);
        assert_eq!(appended.len(), 1);
        let event = &appended[0];
        assert_eq!(event.stream, "project");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.actor, Actor::Baley);
        assert_eq!(event.policy_version, 0);
        assert_eq!(event.payload, effective_payload(&recorded, ID, CATALOG));
        assert_eq!(event.seq, version);
        let document = document(&store, &recorded);
        assert_eq!(document.produced_seq, version);
        assert_eq!(document.body["version"], json!(version));
        assert_eq!(events(&store, "command/policy.record").len(), 1);
    }

    #[test]
    fn an_unchanged_policy_does_not_open_a_command() {
        let (_dir, store) = store();
        let recorded = escalates(true);
        let first = record(&store, &project(), &recorded, CATALOG, request(1), &at(1)).unwrap();
        let before = head(&store);

        let again = record(&store, &project(), &recorded, CATALOG, request(2), &at(2)).unwrap();

        assert_eq!(head(&store), before);
        assert_eq!(again, first);
    }

    #[test]
    fn a_changed_policy_is_not_recorded_at_version_0_and_leaves_only_the_latest() {
        let (_dir, store) = store();
        let first = record(
            &store,
            &project(),
            &escalates(true),
            CATALOG,
            request(1),
            &at(1),
        )
        .unwrap();
        let changed = escalates(false);

        let second = record(&store, &project(), &changed, CATALOG, request(2), &at(2)).unwrap();

        let appended = effective(&store);
        assert_eq!(appended.len(), 2);
        assert_eq!(appended[1].seq, second);
        assert_eq!(appended[1].policy_version, first);
        let document = document(&store, &changed);
        assert_eq!(document.body["values"]["escalate_on_failure"], json!(false));
        assert_eq!(document.body["version"], json!(second));
    }

    #[test]
    fn two_checkouts_and_two_hosts_do_not_share_one_document() {
        let (_dir, store) = store();
        let text = "escalate_on_failure = true\n[host.claude-code]\nescalate_on_failure = false\n";
        let command_line = policy(CHECKOUT, None, text);
        let claude = policy(CHECKOUT, Some(Host::ClaudeCode), text);
        let other = policy("/w/s", None, "escalate_on_failure = false\n");
        let all = [&command_line, &claude, &other];
        let mut versions = Vec::new();
        for (n, recorded) in (1..).zip(all) {
            let version = record(&store, &project(), recorded, CATALOG, request(n), &at(n));
            versions.push(version.unwrap());
        }
        let before = head(&store);

        // Each is judged against its own document, so none records again.
        for ((n, recorded), first) in (4..).zip(all).zip(&versions) {
            let version = record(&store, &project(), recorded, CATALOG, request(n), &at(n));
            assert_eq!(version.unwrap(), *first);
        }
        assert_eq!(head(&store), before);
        assert_eq!(effective(&store).len(), 3);
        for (recorded, held) in [(&command_line, true), (&claude, false), (&other, false)] {
            let body = document(&store, recorded).body;
            assert_eq!(body["values"]["escalate_on_failure"], json!(held));
            assert_eq!(body["checkout"], json!(recorded.checkout));
        }
        assert_eq!(document(&store, &claude).body["host"], json!("claude-code"));
        assert_eq!(document(&store, &command_line).body["host"], json!(null));
    }

    /// The real store, with another record of `racer` committed on the first
    /// `transact` only, after the recorder observed and before its command
    /// runs: the window in which another process can commit.
    struct RecordsFirst<'a> {
        store: &'a SqliteStore,
        racer: &'a RecordedPolicy,
        raced: Cell<Option<u64>>,
    }

    impl Ledger for RecordsFirst<'_> {
        fn transact(
            &self,
            command: &Command,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            if self.raced.get().is_none() {
                let version = record(
                    self.store,
                    &project(),
                    self.racer,
                    CATALOG,
                    request(9),
                    &at(9),
                );
                self.raced.set(Some(version?));
            }
            Ledger::transact(self.store, command, decide)
        }

        fn claim(
            &self,
            command: &Command,
            decide: &mut DecideClaim<'_>,
        ) -> Result<Claimed, StoreError> {
            Ledger::claim(self.store, command, decide)
        }

        fn renew_lease(
            &self,
            project: &ProjectId,
            claim: &ClaimId,
            owner: &ClaimOwner,
            at: &str,
        ) -> Result<(), StoreError> {
            Ledger::renew_lease(self.store, project, claim, owner, at)
        }

        fn complete(
            &self,
            command: &Command,
            owner: &ClaimOwner,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::complete(self.store, command, owner, decide)
        }

        fn reconcile(
            &self,
            command: &Command,
            claim: &ClaimId,
            authority: ReconcileAuthority,
            decide: &mut DecideReconcile<'_>,
        ) -> Result<Recorded, StoreError> {
            Ledger::reconcile(self.store, command, claim, authority, decide)
        }

        fn open_claims(&self, project: &ProjectId) -> Result<Vec<Claim>, StoreError> {
            Ledger::open_claims(self.store, project)
        }

        fn stream(
            &self,
            project: &ProjectId,
            stream: &StreamName,
            from_version: u64,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::stream(self.store, project, stream, from_version, page)
        }

        fn history(
            &self,
            project: &ProjectId,
            range: RangeInclusive<u64>,
            filter: &HistoryFilter,
            page: PageRequest,
        ) -> Result<Page<Event>, StoreError> {
            Ledger::history(self.store, project, range, filter, page)
        }

        fn head(&self, project: &ProjectId) -> Result<Option<Head>, StoreError> {
            Ledger::head(self.store, project)
        }

        fn verify(
            &self,
            project: &ProjectId,
            anchor: Option<&Anchor>,
        ) -> Result<VerifyReport, StoreError> {
            Ledger::verify(self.store, project, anchor)
        }
    }

    impl Views for RecordsFirst<'_> {
        fn get(
            &self,
            project: &ProjectId,
            view: &str,
            key: &DocKey,
        ) -> Result<Option<Document>, StoreError> {
            Views::get(self.store, project, view, key)
        }

        fn get_many(
            &self,
            project: &ProjectId,
            view: &str,
            keys: &[DocKey],
        ) -> Result<Vec<Option<Document>>, StoreError> {
            Views::get_many(self.store, project, view, keys)
        }

        fn find(
            &self,
            project: &ProjectId,
            view: &str,
            query: &IndexQuery,
        ) -> Result<Page<Document>, StoreError> {
            Views::find(self.store, project, view, query)
        }
    }

    #[test]
    fn an_identical_record_committed_first_is_not_appended_twice() {
        let (_dir, store) = store();
        let recorded = escalates(true);
        let racing = RecordsFirst {
            store: &store,
            racer: &recorded,
            raced: Cell::new(None),
        };

        let version = record(&racing, &project(), &recorded, CATALOG, request(1), &at(1)).unwrap();

        let appended = effective(&store);
        assert_eq!(appended.len(), 1);
        assert_eq!(Some(appended[0].seq), racing.raced.get());
        assert_eq!(version, appended[0].seq);
        let completed = events(&store, "command/policy.record");
        assert_eq!(completed.len(), 1, "only the racer's command completed");
    }

    #[test]
    fn a_different_record_committed_first_is_not_replaced_under_the_older_version() {
        let (_dir, store) = store();
        let first = record(
            &store,
            &project(),
            &escalates(true),
            CATALOG,
            request(1),
            &at(1),
        )
        .unwrap();
        let racer = policy(CHECKOUT, None, "roles.reviewer.effort = \"high\"\n");
        let racing = RecordsFirst {
            store: &store,
            racer: &racer,
            raced: Cell::new(None),
        };
        let mine = escalates(false);

        let version = record(&racing, &project(), &mine, CATALOG, request(2), &at(2)).unwrap();

        let raced = racing.raced.get().unwrap();
        assert!(raced > first);
        let appended = effective(&store);
        assert_eq!(appended.len(), 3);
        let last = &appended[2];
        assert_eq!(last.seq, version);
        assert_eq!(
            last.policy_version, raced,
            "not {first}, the version it observed"
        );
        let body = document(&store, &mine).body;
        assert_eq!(body["values"]["escalate_on_failure"], json!(false));
        assert_eq!(body["values"]["roles.reviewer.effort"], json!("medium"));
    }
}
