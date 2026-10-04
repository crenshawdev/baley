//! Checkout admission's store step: one checkout recorded as Baley's own
//! command, or refused as a fork, in one transaction (design 0001 Project
//! identity and policy, EVD-R17).

use std::fmt;

use baley_core::checkout::{
    CHECKOUT_SEEN, CHECKOUT_SEEN_VERSION, CHECKOUT_VIEW, Checkout, CheckoutAction,
    CheckoutOperation, ProjectIdConflict, checkout_spec, plan_checkout_admission,
};
use baley_core::policy::recorded::{POLICY_VIEW, policy_key};
use baley_store::{
    Actor, Caller, Command, CommandKind, Decision, Document, IndexQuery, Ledger, NewEvent,
    Observed, ObservedDocument, OutcomeKind, Page, PageRequest, ProjectId, Refusal, RequestId,
    StoreError, StreamName, Views, request_digest,
};
use serde_json::{Value, json};

use crate::init::PROJECT_STREAM;

/// The command kind checkout admission records under.
pub const ADMIT_COMMAND: &str = "checkout.admit";

/// Tries in all before a checkout admission that keeps moving is returned as stale.
const ATTEMPTS: usize = 3;

/// Why checkout admission did not succeed, kept apart so a caller renders a
/// fork as a refusal and a store error through `display::store_error`.
#[derive(Debug)]
pub enum AdmitError {
    /// The checkout is a fork. Nothing was recorded in the project.
    Fork(ProjectIdConflict),
    /// The store failed.
    Store(StoreError),
}

impl fmt::Display for AdmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fork(conflict) => conflict.fmt(f),
            Self::Store(error) => error.fmt(f),
        }
    }
}

impl From<StoreError> for AdmitError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Records `checkout` as seen under `project`, or refuses it as a fork.
///
/// It prints nothing. It runs before the policy step and never in its
/// transaction. Its callers are the ledger commands, `purge`, `config set`
/// and `baley init`, and the session server's write preparation, which passes
/// the call's caller and the server's time. The guard never runs it.
///
/// A checkout whose row already holds its root commit and remote URL opens
/// no command, so not even `command.completed` is recorded. Any other
/// verdict, a fork included, is judged again inside the transaction, so two
/// checkouts admitted at once are judged in queue order. A fork records
/// nothing in the project: the verdict leaves the decision as an error and
/// comes back as [`AdmitError::Fork`].
///
/// The command's policy version is the one stored for the checkout and no
/// host, or 0, and the stored `policy` document is observed, so a record of
/// it landing in between makes the store refuse the command as stale. A
/// stale refusal is read and decided afresh with the same request id, at
/// most three tries in all.
///
/// `caller` is recorded on the command and so on every event it appends,
/// `command.completed` included. It stays out of the request digest, so the
/// same checkout and request id are the same request with or without one. The
/// command line passes none. The actor is Baley's own either way, and the
/// time is the one given.
pub fn admit(
    store: &(impl Ledger + Views),
    project: &ProjectId,
    checkout: &Checkout,
    request_id: RequestId,
    at: &str,
    caller: Option<Caller>,
) -> Result<(), AdmitError> {
    let mut attempt = 1;
    loop {
        // A stale refusal records nothing, so the request id is still free.
        match admit_once(store, project, checkout, &request_id, at, caller.clone()) {
            Err(AdmitError::Store(StoreError::Stale(_))) if attempt < ATTEMPTS => attempt += 1,
            result => return result,
        }
    }
}

fn admit_once(
    store: &(impl Ledger + Views),
    project: &ProjectId,
    checkout: &Checkout,
    request_id: &RequestId,
    at: &str,
    caller: Option<Caller>,
) -> Result<(), AdmitError> {
    let key = policy_key(&checkout.path, None);
    let rows = all_rows(&mut |query| store.find(project, CHECKOUT_VIEW, query))?;
    let stored = store.get(project, POLICY_VIEW, &key)?;
    let plan = plan_checkout_admission(
        checkout,
        &rows,
        stored.as_ref().map(|document| &document.body),
    );
    let records = match &plan.action {
        CheckoutAction::Refuse(_) => true,
        CheckoutAction::Proceed(operations) => operations
            .iter()
            .any(|operation| matches!(operation, CheckoutOperation::RecordSeen(_))),
    };
    if !records {
        return Ok(());
    }
    // The plan's version rides a refusal too: the judgement inside the
    // transaction can differ from this one, and the command is already built.
    let command = command(
        project,
        checkout,
        plan.policy_version,
        request_id,
        at,
        caller,
    )?;
    let observed = Observed {
        documents: vec![ObservedDocument {
            view: POLICY_VIEW.into(),
            key: key.clone(),
            produced_seq: stored.as_ref().map(|document| document.produced_seq),
        }],
        ..Observed::default()
    };
    let mut refusal = None;
    let result = store.transact(&command, &mut |tx| {
        let rows = all_rows(&mut |query| tx.find(CHECKOUT_VIEW, query))?;
        let stored = tx.get(POLICY_VIEW, &key)?;
        let plan = plan_checkout_admission(
            checkout,
            &rows,
            stored.as_ref().map(|document| &document.body),
        );
        match plan.action {
            CheckoutAction::Refuse(conflict) => {
                let text = conflict.to_string();
                refusal = Some(conflict);
                // Never `Stale`, and never a recorded refusal: the caller
                // tells a fork apart by the captured verdict alone.
                return Err(StoreError::Refused(Refusal::InvalidEvent(text)));
            }
            CheckoutAction::Proceed(operations) => {
                for operation in operations {
                    if let CheckoutOperation::RecordSeen(payload) = operation {
                        tx.append(NewEvent {
                            stream: StreamName(PROJECT_STREAM.into()),
                            type_name: CHECKOUT_SEEN.into(),
                            type_version: CHECKOUT_SEEN_VERSION,
                            git: None,
                            payload,
                            attachments: vec![],
                        })?;
                    }
                }
            }
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({ "admitted": true }),
            sensitive: false,
            observed: observed.clone(),
            git: None,
        })
    });
    if let Some(conflict) = refusal {
        return Err(AdmitError::Fork(conflict));
    }
    result?;
    Ok(())
}

/// Every body of the project's `checkout` view, read page by page through
/// its one index with no equal values, as far as `next` goes (the view's
/// page bound caps one page).
fn all_rows(
    find: &mut dyn FnMut(&IndexQuery) -> Result<Page<Document>, StoreError>,
) -> Result<Vec<Value>, StoreError> {
    let spec = checkout_spec();
    let index = spec.indexes[0].name.clone();
    let mut rows = Vec::new();
    let mut after = None;
    loop {
        let page = find(&IndexQuery {
            index: index.clone(),
            equals: vec![],
            page: PageRequest {
                limit: spec.page_bound,
                after,
            },
        })?;
        rows.extend(page.items.into_iter().map(|document| document.body));
        match page.next {
            Some(cursor) => after = Some(cursor),
            None => return Ok(rows),
        }
    }
}

/// Checkout admission's command. The checkout is in the digest, so two different
/// checkouts never share an identity.
fn command(
    project: &ProjectId,
    checkout: &Checkout,
    policy_version: u64,
    request_id: &RequestId,
    at: &str,
    caller: Option<Caller>,
) -> Result<Command, StoreError> {
    let actor = Actor::Baley;
    let digest = request_digest(&json!({
        "kind": ADMIT_COMMAND,
        "project": project.0,
        "actor": actor.as_str(),
        "policy_version": policy_version,
        "payload": baley_core::checkout::seen_payload(checkout),
        "scope": [],
    }))
    .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
    Ok(Command {
        project: project.clone(),
        kind: CommandKind(ADMIT_COMMAND.into()),
        request_id: request_id.clone(),
        digest,
        scope: vec![],
        policy_version,
        recorded_at: at.into(),
        actor,
        caller,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ops::RangeInclusive;
    use std::path::Path;

    use baley_core::checkout::{checkout_key, seen_payload};
    use baley_core::policy::recorded::recorded_policy;
    use baley_core::policy::{Host, Schema, effective_policy};
    use baley_store::{
        Admin, Anchor, COMMAND_CLAIMED, COMMAND_COMPLETED, COMMAND_RECONCILED, Claim, ClaimId,
        ClaimOwner, Claimed, Decide, DecideClaim, DecideReconcile, Event, Head, HistoryFilter,
        ReconcileAuthority, Recorded, ServerCaller, VerifyReport,
    };
    use baley_store_sqlite::SqliteStore;

    use super::*;
    use crate::policy_step;

    const T0: &str = "2026-10-01T10:00:00Z";
    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const ROOT: &str = "1111111111111111111111111111111111111111";
    const OTHER_ROOT: &str = "2222222222222222222222222222222222222222";
    const URL_X: &str = "https://example.com/o/x.git";
    const URL_Y: &str = "https://example.com/o/y.git";

    fn at(n: u8) -> String {
        format!("2026-10-01T10:00:{n:02}Z")
    }

    fn request(n: u32) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-{n:012}"))
    }

    fn project() -> ProjectId {
        ProjectId(ID.into())
    }

    fn checkout(path: &str, root: Option<&str>, url: Option<&str>) -> Checkout {
        Checkout {
            path: path.into(),
            root_commit: root.map(Into::into),
            remote_url: url.map(Into::into),
        }
    }

    /// A real store in a fresh temporary directory, holding the project.
    fn store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
        store.create_project(&project(), "sample", T0).unwrap();
        (dir, store)
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

    fn seen(store: &SqliteStore) -> Vec<Event> {
        events(store, PROJECT_STREAM)
            .into_iter()
            .filter(|event| event.type_name == CHECKOUT_SEEN)
            .collect()
    }

    fn head(store: &SqliteStore) -> u64 {
        store.head(&project()).unwrap().unwrap().seq
    }

    fn row(store: &SqliteStore, path: &str) -> Option<Document> {
        store
            .get(&project(), CHECKOUT_VIEW, &checkout_key(path))
            .unwrap()
    }

    fn rows(store: &SqliteStore) -> Vec<Value> {
        all_rows(&mut |query| store.find(&project(), CHECKOUT_VIEW, query)).unwrap()
    }

    fn admitted(store: &(impl Ledger + Views), n: u8, checkout: &Checkout) {
        admit(store, &project(), checkout, request(n.into()), &at(n), None).unwrap();
    }

    fn refused(store: &(impl Ledger + Views), n: u8, checkout: &Checkout) -> ProjectIdConflict {
        match admit(store, &project(), checkout, request(n.into()), &at(n), None) {
            Err(AdmitError::Fork(conflict)) => conflict,
            other => panic!("expected a fork refusal, got {other:?}"),
        }
    }

    /// Records the policy of `path` for `host` through the policy step's
    /// recorder and returns the version in force.
    fn record_policy(store: &(impl Ledger + Views), path: &str, host: Option<Host>, n: u8) -> u64 {
        let file = crate::settings::file(
            Path::new("/c/config.toml"),
            b"escalate_on_failure = true\n".to_vec(),
        );
        let policy = effective_policy(Schema::standard(), host, Some(&file), None).unwrap();
        let recorded = recorded_policy(Path::new(path), &policy).unwrap();
        policy_step::record(
            store,
            &project(),
            &recorded,
            5,
            request(n.into()),
            &at(n),
            None,
        )
        .unwrap()
    }

    fn policy_version(store: &SqliteStore, path: &str) -> u64 {
        let document = store
            .get(&project(), POLICY_VIEW, &policy_key(path, None))
            .unwrap()
            .unwrap();
        document.body["version"].as_u64().unwrap()
    }

    #[test]
    fn the_first_checkout_is_not_off_version_1_the_baley_actor_the_project_stream_or_unviewed() {
        let (_dir, store) = store();
        let first = checkout("/w/a", Some(ROOT), Some(URL_X));

        admitted(&store, 1, &first);

        let appended = seen(&store);
        assert_eq!(appended.len(), 1);
        let event = &appended[0];
        assert_eq!(event.stream, "project");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.actor, Actor::Baley);
        assert_eq!(event.policy_version, 0);
        assert_eq!(event.payload, seen_payload(&first));
        assert_eq!(events(&store, "command/checkout.admit").len(), 1);
        assert_eq!(row(&store, "/w/a").unwrap().body, seen_payload(&first));
    }

    #[test]
    fn an_unchanged_checkout_does_not_open_a_command() {
        let (_dir, store) = store();
        let first = checkout("/w/a", Some(ROOT), Some(URL_X));
        admitted(&store, 1, &first);
        let before = head(&store);

        admitted(&store, 2, &first);

        assert_eq!(head(&store), before);
    }

    #[test]
    fn a_changed_root_commit_is_not_kept_as_a_second_row_or_left_unrecorded() {
        let (_dir, store) = store();
        admitted(&store, 1, &checkout("/w/a", Some(ROOT), Some(URL_X)));

        admitted(&store, 2, &checkout("/w/a", Some(OTHER_ROOT), Some(URL_X)));

        assert_eq!(seen(&store).len(), 2);
        let held = rows(&store);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0]["root_commit"], json!(OTHER_ROOT));
    }

    #[test]
    fn a_second_clone_of_the_same_url_is_not_refused() {
        let (_dir, store) = store();
        admitted(&store, 1, &checkout("/w/a", Some(ROOT), Some(URL_X)));

        admitted(&store, 2, &checkout("/w/b", Some(ROOT), Some(URL_X)));

        assert_eq!(rows(&store).len(), 2);
    }

    #[test]
    fn a_fork_is_not_recorded_or_given_a_command_and_names_both_checkouts() {
        let (_dir, store) = store();
        admitted(&store, 1, &checkout("/w/a", Some(ROOT), Some(URL_X)));
        let before = head(&store);
        let commands = events(&store, "command/checkout.admit").len();

        let conflict = refused(&store, 2, &checkout("/w/b", Some(ROOT), Some(URL_Y)));

        let text = conflict.to_string();
        assert!(text.starts_with("project-id-conflict:"), "{text}");
        for part in ["/w/a", "/w/b", "baley init --new-id"] {
            assert!(text.contains(part), "{part} missing from {text}");
        }
        assert_eq!(head(&store), before);
        assert!(row(&store, "/w/b").is_none());
        assert!(
            seen(&store)
                .iter()
                .all(|event| event.payload["path"] != "/w/b")
        );
        assert_eq!(events(&store, "command/checkout.admit").len(), commands);
    }

    #[test]
    fn the_command_is_not_always_at_version_0_and_ignores_a_policy_for_another_host() {
        let (_dir, store) = store();
        let version = record_policy(&store, "/w/a", None, 1);
        record_policy(&store, "/w/c", Some(Host::ClaudeCode), 2);

        admitted(&store, 3, &checkout("/w/a", Some(ROOT), None));
        admitted(&store, 4, &checkout("/w/c", Some(ROOT), None));

        let appended = seen(&store);
        assert_eq!(appended.len(), 2);
        assert_eq!(appended[0].payload["path"], json!("/w/a"));
        assert_eq!(appended[0].policy_version, version);
        assert_ne!(version, 0);
        assert_eq!(appended[1].payload["path"], json!("/w/c"));
        assert_eq!(appended[1].policy_version, 0);
    }

    const SESSION: &str = "3f2b8c1e-4d5a-4e6f-8a7b-9c0d1e2f3a4b";

    /// A server caller for JSON-RPC call 7.
    fn server_caller() -> Caller {
        Caller::Server(
            ServerCaller::new("/p", "/p/sub", "claude-code", SESSION, &json!(7)).unwrap(),
        )
    }

    /// Every `command.claimed` and `command.reconciled` event in the project.
    fn claim_events(store: &SqliteStore) -> Vec<Event> {
        let filter = HistoryFilter {
            types: vec![COMMAND_CLAIMED.into(), COMMAND_RECONCILED.into()],
            git_commit: None,
        };
        let page = PageRequest {
            limit: 100,
            after: None,
        };
        let range = 1..=head(store);
        store
            .history(&project(), range, &filter, page)
            .unwrap()
            .items
    }

    fn completed(store: &SqliteStore) -> Vec<Event> {
        events(store, "command/checkout.admit")
            .into_iter()
            .filter(|event| event.type_name == COMMAND_COMPLETED)
            .collect()
    }

    #[test]
    fn a_server_callers_checkout_is_not_recorded_without_its_caller_actor_or_time() {
        let (_dir, store) = store();
        let caller = server_caller();

        admit(
            &store,
            &project(),
            &checkout("/w/a", Some(ROOT), Some(URL_X)),
            request(1),
            &at(1),
            Some(caller.clone()),
        )
        .unwrap();

        let appended = seen(&store);
        let done = completed(&store);
        assert_eq!((appended.len(), done.len()), (1, 1));
        for event in appended.iter().chain(&done) {
            assert_eq!(event.caller.as_ref(), Some(&caller), "{}", event.type_name);
            assert_eq!(event.actor, Actor::Baley, "{}", event.type_name);
            assert_eq!(event.recorded_at, at(1), "{}", event.type_name);
        }
    }

    #[test]
    fn a_command_line_checkout_is_not_recorded_with_a_caller_after_a_server_one() {
        let (_dir, store) = store();
        admit(
            &store,
            &project(),
            &checkout("/w/a", Some(ROOT), Some(URL_X)),
            request(1),
            &at(1),
            Some(server_caller()),
        )
        .unwrap();

        admitted(&store, 2, &checkout("/w/b", Some(ROOT), Some(URL_X)));

        let second: Vec<_> = seen(&store)
            .into_iter()
            .chain(completed(&store))
            .filter(|event| event.request_id == request(2))
            .collect();
        assert_eq!(second.len(), 2);
        assert!(second.iter().all(|event| event.caller.is_none()));
    }

    #[test]
    fn a_server_callers_checkout_is_not_left_with_a_claim_or_a_reconciliation() {
        let (_dir, store) = store();

        admit(
            &store,
            &project(),
            &checkout("/w/a", Some(ROOT), Some(URL_X)),
            request(1),
            &at(1),
            Some(server_caller()),
        )
        .unwrap();

        assert!(store.open_claims(&project()).unwrap().is_empty());
        assert!(claim_events(&store).is_empty());
    }

    #[test]
    fn the_checkout_command_digest_is_not_changed_by_a_caller() {
        let held = checkout("/w/a", Some(ROOT), Some(URL_X));
        let without = command(&project(), &held, 3, &request(1), &at(1), None).unwrap();

        let with = command(
            &project(),
            &held,
            3,
            &request(1),
            &at(1),
            Some(server_caller()),
        );

        let with = with.unwrap();
        assert_eq!(with.caller, Some(server_caller()));
        assert_eq!(with.digest, without.digest);
    }

    /// The real store, with `before` run once ahead of the first `transact`,
    /// after the caller read and before its command runs: the window in which
    /// another process can commit.
    struct Racing<'a> {
        store: &'a SqliteStore,
        before: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    }

    impl<'a> Racing<'a> {
        fn new(store: &'a SqliteStore, before: impl FnOnce() + 'a) -> Self {
            Self {
                store,
                before: RefCell::new(Some(Box::new(before))),
            }
        }
    }

    impl Ledger for Racing<'_> {
        fn transact(
            &self,
            command: &Command,
            decide: &mut Decide<'_>,
        ) -> Result<Recorded, StoreError> {
            let before = self.before.borrow_mut().take();
            if let Some(before) = before {
                before();
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

    impl Views for Racing<'_> {
        fn get(
            &self,
            project: &ProjectId,
            view: &str,
            key: &baley_store::DocKey,
        ) -> Result<Option<Document>, StoreError> {
            Views::get(self.store, project, view, key)
        }

        fn get_many(
            &self,
            project: &ProjectId,
            view: &str,
            keys: &[baley_store::DocKey],
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
    fn a_checkout_judged_only_before_the_transaction_is_not_queue_ordered() {
        let (_dir, store) = store();
        let racing = Racing::new(&store, || {
            admitted(&store, 9, &checkout("/w/b", Some(ROOT), Some(URL_Y)));
        });

        let conflict = refused(&racing, 1, &checkout("/w/a", Some(ROOT), Some(URL_X)));

        assert_eq!(conflict.other_path, "/w/b");
        let appended = seen(&store);
        assert_eq!(appended.len(), 1);
        assert_eq!(appended[0].payload["path"], json!("/w/b"));
        assert_eq!(head(&store), appended[0].seq + 1);
    }

    #[test]
    fn rows_past_the_first_page_are_not_ignored_by_the_fork_judgement() {
        let (_dir, store) = store();
        let bound = checkout_spec().page_bound;
        // Rows with no URL never conflict, so all of them are recorded first
        // and sort ahead of the one row holding a URL.
        for n in 0..=bound {
            let path = format!("/a/{n:04}");
            let bare = checkout(&path, Some(ROOT), None);
            admit(&store, &project(), &bare, request(100 + n), &at(1), None).unwrap();
        }
        admitted(&store, 1, &checkout("/z", Some(ROOT), Some(URL_X)));
        assert!(rows(&store).len() > bound as usize);

        let conflict = refused(&store, 2, &checkout("/w/q", Some(ROOT), Some(URL_Y)));

        assert_eq!(conflict.other_path, "/z");
    }

    #[test]
    fn a_policy_recorded_mid_command_is_not_left_at_the_version_read_before() {
        let (_dir, store) = store();
        let racing = Racing::new(&store, || {
            record_policy(&store, "/w/a", None, 9);
        });

        admit(
            &racing,
            &project(),
            &checkout("/w/a", Some(ROOT), None),
            request(1),
            &at(1),
            None,
        )
        .unwrap();

        let appended = seen(&store);
        assert_eq!(appended.len(), 1);
        let version = policy_version(&store, "/w/a");
        assert_ne!(version, 0);
        assert_eq!(appended[0].policy_version, version);
    }

    #[test]
    fn a_refusal_turned_into_a_record_inside_the_transaction_is_not_built_at_version_0() {
        let (_dir, store) = store();
        admitted(&store, 1, &checkout("/w/b", Some(ROOT), Some(URL_Y)));
        let version = record_policy(&store, "/w/a", None, 2);
        // The read before the transaction sees /w/b holding URL Y. Then /w/b
        // loses its URL, so the judgement inside the transaction records.
        let racing = Racing::new(&store, || {
            admitted(&store, 3, &checkout("/w/b", Some(ROOT), None));
        });

        admit(
            &racing,
            &project(),
            &checkout("/w/a", Some(ROOT), Some(URL_X)),
            request(4),
            &at(4),
            None,
        )
        .unwrap();

        let mine: Vec<_> = seen(&store)
            .into_iter()
            .filter(|event| event.payload["path"] == "/w/a")
            .collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].policy_version, version);
        assert_ne!(version, 0);
    }
}
