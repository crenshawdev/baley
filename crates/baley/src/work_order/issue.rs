//! The recording of one issued order (design 0001 Streams and Payloads,
//! design 0012): its inputs as `record` payloads and one `work_order.issued`
//! on its subject's stream, in one domain transaction under the command its
//! caller prepared, or inside a transaction its caller runs, so an order is
//! recorded with the answers or set that issued it.
//!
//! The caller judges everything first and supplies what its judgement read:
//! the facts that must still be absent, the stream versions it saw and the
//! view documents it read. The decision declares all three, so an order
//! recorded, a start approved, a stream moved or a read document changed
//! since makes the store refuse the command as stale with nothing recorded.

use baley_core::policy::Role;
use baley_core::work_order::{
    IssuedOrder, OrderInput, PENDING, START_SUBJECT, WORK_ORDER_ISSUED, WORK_ORDER_ISSUED_VERSION,
    WORK_ORDER_LIVE_INDEX, WORK_ORDER_VIEW, issued_payload, order_stream,
};
use baley_store::{
    Absence, Command, Decision, InstructionEvidence, KeyValue, Ledger, NewEvent, Observed,
    ObservedDocument, OutcomeKind, Payloads, Recorded, Refusal as StoreRefusal, RetentionClass,
    StoreError, StreamName, Transaction,
};
use serde_json::{Map, Value, json};

use crate::envelope::{LEDGER_BUSY, Refusal};
use crate::ledger::answer::{AnswerUnread, answer_value};
use crate::mcp::capture::{PLACE_LEDGER, REQUEST_ID_REUSE, failed, store_failed};
use crate::mcp::prepare::LEDGER_UNAVAILABLE;
use crate::scope::submit::{NOT_RECORDED_BUSY, start_approval};
use crate::work_order::route::IssuedRoute;

/// One input an order is issued with.
#[derive(Debug, Clone, Copy)]
pub enum Input<'a> {
    /// A body stored as a `record` payload under `name`.
    Bytes {
        /// The input's name, such as `brief`.
        name: &'a str,
        /// Its bytes.
        bytes: &'a [u8],
    },
    /// An input already held, attached as it is, as a retry carries its
    /// parent's.
    Held(&'a OrderInput),
}

/// Everything one order is recorded with.
#[derive(Debug, Clone, Copy)]
pub struct Issue<'a> {
    /// The order's id, a lowercase UUID.
    pub id: &'a str,
    /// The id of its chain's first order, its own id at attempt 1.
    pub request: &'a str,
    /// The failed order it retries, if any.
    pub parent: Option<&'a str>,
    /// What it is about, `start` for the start planner's order.
    pub subject: &'a str,
    /// The route it runs under, whose role and attempt it records.
    pub route: &'a IssuedRoute,
    /// The instructions it carries, in order.
    pub instructions: &'a [InstructionEvidence],
    /// Its inputs, in order.
    pub inputs: &'a [Input<'a>],
    /// The facts the caller's judgement read as absent.
    pub absences: &'a [Absence],
    /// The stream versions the caller's judgement read.
    pub streams: &'a [(StreamName, u64)],
    /// The view documents the caller's judgement read, at the sequence that
    /// last produced each, so a change to any of them since makes the command
    /// stale.
    pub documents: &'a [ObservedDocument],
    /// Members the caller adds to the receipt, such as project start's
    /// `settings` and `existing_code` or a retry's `parent`.
    pub further: &'a Map<String, Value>,
}

/// The two facts a start order relies on being absent: a pending start
/// planner order (D-02) and the start approval (D-19).
pub fn start_absences() -> Vec<Absence> {
    let text = |value: &str| KeyValue::Text(value.into());
    vec![
        Absence::Documents {
            view: WORK_ORDER_VIEW.into(),
            index: WORK_ORDER_LIVE_INDEX.into(),
            equals: vec![
                text(Role::Planner.name()),
                text(START_SUBJECT),
                text(PENDING),
            ],
        },
        Absence::Event(start_approval()),
    ]
}

/// The one-line prompt the session gives the worker (HST-R14).
pub fn prompt(id: &str) -> String {
    format!("Read Baley work order {id} and follow it.")
}

/// The sentence telling the session how to start the worker: the stub
/// `agent` with the Agent tool and the prompt, with the route's model as the
/// Agent tool's `model` parameter, or with none so the stub's `model:
/// inherit` applies (D-23). An order handed over after its issue is launched
/// with the same sentence.
pub fn launch(agent: &str, model: Option<&str>) -> String {
    match model {
        Some(model) => format!(
            "Start the subagent {agent} with the Agent tool, giving it the prompt above, and pass {model} as the Agent tool's model parameter."
        ),
        None => format!(
            "Start the subagent {agent} with the Agent tool, giving it the prompt above, and pass no model parameter, so the subagent's own model: inherit applies."
        ),
    }
}

/// Records `issue` under `command` in one domain transaction. A request the
/// store already holds is replayed from its record before any decision
/// runs, so a replay passes no issue; a decision asked to run without one
/// records nothing. The answer is the receipt, which never holds an input.
/// The store's result comes back unchanged.
pub fn record<L: Ledger + ?Sized>(
    store: &L,
    command: &Command,
    issue: Option<&Issue<'_>>,
) -> Result<Recorded, StoreError> {
    store.transact(command, &mut |tx| {
        let Some(issue) = issue else {
            return Err(StoreError::Unavailable(
                "a work order reached its transaction without what it is issued with".into(),
            ));
        };
        let mut receipt = Map::new();
        receipt.insert("status".into(), json!("ok"));
        receipt.extend(issue_within(tx, issue, &command.recorded_at)?);
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: Value::Object(receipt),
            sensitive: false,
            observed: Observed {
                absences: issue.absences.to_vec(),
                documents: issue.documents.to_vec(),
                ..Observed::default()
            },
            git: None,
        })
    })
}

/// Issues `issue` inside `tx`, a transaction its caller runs, recorded at
/// `recorded_at`: its inputs stored as `record` payloads and one
/// `work_order.issued` on its subject's stream, after the stream versions it
/// names are confirmed. It builds no decision, so the caller declares the
/// issue's absences and observed documents in its own beside its own, and
/// the order is recorded with the caller's records or not at all, as a
/// closing answer issues the drafting order. The answer is the receipt's
/// members other than `status`.
pub fn issue_within(
    tx: &mut dyn Transaction,
    issue: &Issue<'_>,
    recorded_at: &str,
) -> Result<Map<String, Value>, StoreError> {
    let Some(stream) = order_stream(issue.subject) else {
        return Err(StoreError::Unavailable(format!(
            "no stream records orders about {}",
            issue.subject
        )));
    };
    for (read, version) in issue.streams {
        tx.expect(read, *version)?;
    }
    let mut inputs = Vec::with_capacity(issue.inputs.len());
    for input in issue.inputs {
        inputs.push(match input {
            Input::Bytes { name, bytes } => OrderInput {
                name: (*name).into(),
                body: tx.put_payload(bytes, RetentionClass::Record)?,
            },
            Input::Held(held) => (*held).clone(),
        });
    }
    let route = &issue.route.route;
    let payload = issued_payload(&IssuedOrder {
        id: issue.id,
        request: issue.request,
        parent: issue.parent,
        role: route.role,
        subject: issue.subject,
        attempt: route.attempt,
        route: &issue.route.record,
        agent: &issue.route.agent,
        instructions: issue.instructions,
        inputs: &inputs,
    });
    tx.append(NewEvent {
        stream: StreamName(stream.into()),
        type_name: WORK_ORDER_ISSUED.into(),
        type_version: WORK_ORDER_ISSUED_VERSION,
        git: None,
        payload,
        attachments: inputs.iter().map(|input| input.body.clone()).collect(),
    })?;
    let mut receipt = Map::new();
    receipt.insert("work_order".into(), json!(issue.id));
    receipt.insert("request".into(), json!(issue.request));
    receipt.insert("attempt".into(), json!(route.attempt));
    receipt.insert("agent".into(), json!(issue.route.agent));
    receipt.insert("model".into(), json!(route.model));
    receipt.insert("prompt".into(), json!(prompt(issue.id)));
    receipt.insert(
        "launch".into(),
        json!(launch(&issue.route.agent, route.model.as_deref())),
    );
    receipt.insert("recorded_at".into(), json!(recorded_at));
    receipt.extend(issue.further.clone());
    Ok(receipt)
}

/// A busy ledger's reason when a recorded answer was not read.
const NOT_READ_BUSY: &str = "the ledger is busy, so the recorded answer was not read. Try it again";

/// The answer for an order's transaction result, as `scope::submit::answer`
/// gives one. A new or replayed outcome answers the recorded answer, read
/// through `payloads` when it was stored. A ledger that moved after the call
/// was judged is `ledger-busy`, since the same call is judged afresh, and a
/// request id already used for another call is `request-id-reuse`.
pub fn answer(result: Result<Recorded, StoreError>, payloads: &dyn Payloads) -> Value {
    match result {
        Ok(Recorded::New { outcome, .. } | Recorded::Replayed { outcome }) => {
            match answer_value(&outcome.answer, payloads) {
                Ok(value) => value,
                Err(AnswerUnread::Read(error)) => store_failed(&error, NOT_READ_BUSY),
                Err(AnswerUnread::Gone(_)) => failed(
                    LEDGER_UNAVAILABLE,
                    "the call is recorded, but its stored answer is no longer held",
                    PLACE_LEDGER,
                ),
                Err(AnswerUnread::Malformed) => failed(
                    LEDGER_UNAVAILABLE,
                    "the call is recorded, but its stored answer cannot be read",
                    PLACE_LEDGER,
                ),
            }
        }
        Err(StoreError::Refused(StoreRefusal::RequestDigestMismatch { .. })) => Refusal::new(
            REQUEST_ID_REUSE,
            "this request_id was already used for a different call, so nothing was recorded. A new call needs a new request_id",
        )
        .slot("request_id")
        .value(),
        Err(StoreError::Stale(_)) => failed(
            LEDGER_BUSY,
            "the project's work orders, its start or its stored inputs moved while this call was prepared, so nothing was recorded. The same call can be sent again",
            PLACE_LEDGER,
        ),
        Err(error) => store_failed(&error, NOT_RECORDED_BUSY),
    }
}

#[cfg(test)]
mod tests {
    //! Expected values come from design 0012, the phase's decisions and the
    //! plan's test data, never from running this code. Each route comes from
    //! `route::compose` over supplied settings, which this unit records.

    use std::collections::BTreeSet;
    use std::io::Read;
    use std::num::NonZeroU32;
    use std::path::Path;

    use baley_core::policy::{AcceptedNames, Host, Role, Schema, effective_policy};
    use baley_core::work_order::{WORK_ORDER_FAILED, failed_payload, work_order_key};
    use baley_store::{Admin, Hash, PayloadBody, PayloadRef, ProjectId, RetentionClass, Views};

    use super::*;
    use crate::scope::fixtures::*;
    use crate::work_order::route::compose;

    const I0: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const I1: &str = "6e2d3c4b-5f60-4b7c-9d8e-0f1a2b3c4d5e";
    const B1: &str = "Build a ledger for agent work";
    const B1_SHA256: &str = "04fe1dc2bdd0a95df1e6d96c47379024f73df0d9584025e1431b6b5c3162ac9b";

    /// The planner's route at attempt 1 under a global file setting
    /// `model`, or none, with the four aliases and two concrete ids
    /// accepted at version 0.
    fn route_for(model: Option<&str>) -> IssuedRoute {
        let text = model.map_or_else(String::new, |model| {
            format!("[roles.planner]\nmodel = \"{model}\"\n")
        });
        let file = crate::settings::file(Path::new("/c/config.toml"), text.into_bytes());
        let policy = effective_policy(
            Schema::standard(),
            Some(Host::ClaudeCode),
            Some(&file),
            None,
        )
        .unwrap();
        let names: BTreeSet<String> = [
            "fable",
            "haiku",
            "opus",
            "sonnet",
            "claude-opus-5-5",
            "claude-haiku-4-5-20251001",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let names = AcceptedNames { names, version: 0 };
        let attempt = NonZeroU32::new(1).unwrap();
        compose(&policy, 1, Host::ClaudeCode, Role::Planner, attempt, &names).unwrap()
    }

    fn instructions() -> Vec<InstructionEvidence> {
        vec![crate::instruction::evidence("bal-start-planner").unwrap()]
    }

    /// Records start order `id` in `project` with brief B1 under `request_id`
    /// at `at`, with the start's two absences and the `further` members.
    fn start_in(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectId,
        id: &str,
        route: &IssuedRoute,
        request_id: &str,
        at: &str,
        further: &Map<String, Value>,
    ) -> Value {
        let inputs = [Input::Bytes {
            name: "brief",
            bytes: B1.as_bytes(),
        }];
        let instructions = instructions();
        let absences = start_absences();
        let issue = Issue {
            id,
            request: id,
            parent: None,
            subject: START_SUBJECT,
            route,
            instructions: &instructions,
            inputs: &inputs,
            absences: &absences,
            streams: &[],
            documents: &[],
            further,
        };
        let command = command_in(project, "project.start", request_id, 1, at);
        answer(record(store, &command, Some(&issue)), store)
    }

    fn start(
        store: &baley_store_sqlite::SqliteStore,
        id: &str,
        request_id: &str,
        at: &str,
    ) -> Value {
        start_in(
            store,
            &project(),
            id,
            &route_for(None),
            request_id,
            at,
            &Map::new(),
        )
    }

    fn head_of(store: &baley_store_sqlite::SqliteStore, project: &ProjectId) -> u64 {
        store.head(project).unwrap().map_or(0, |head| head.seq)
    }

    fn order(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectId,
        id: &str,
    ) -> Option<Value> {
        store
            .get(project, WORK_ORDER_VIEW, &work_order_key(id))
            .unwrap()
            .map(|document| document.body)
    }

    #[test]
    fn an_order_recorded_off_its_stream_or_without_its_brief_body_is_caught() {
        let (_dir, store) = two_projects();
        let route = route_for(None);
        let first = start(&store, I0, R1, T1);

        let events = history(&store);
        let issued = of_type(&events, WORK_ORDER_ISSUED);
        assert_eq!(issued.len(), 1);
        assert_eq!(issued[0].stream, "roadmap");
        assert_eq!(issued[0].type_version, 1);
        let brief = OrderInput {
            name: "brief".into(),
            body: PayloadRef {
                hash: Hash::from_hex(B1_SHA256).unwrap(),
                bytes: 29,
                class: RetentionClass::Record,
            },
        };
        let instructions = instructions();
        let expected = issued_payload(&IssuedOrder {
            id: I0,
            request: I0,
            parent: None,
            role: Role::Planner,
            subject: "start",
            attempt: 1,
            route: &route.record,
            agent: "bal-planner-high",
            instructions: &instructions,
            inputs: std::slice::from_ref(&brief),
        });
        assert_eq!(issued[0].payload, expected);
        let PayloadBody::Present(mut reader) = store.open(&brief.body.hash).unwrap() else {
            panic!("the brief's body is present");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, B1.as_bytes());

        let launch = first["launch"].as_str().unwrap().to_string();
        assert_eq!(
            first,
            json!({"status": "ok", "work_order": I0, "request": I0, "attempt": 1,
                "agent": "bal-planner-high", "model": null,
                "prompt": format!("Read Baley work order {I0} and follow it."),
                "launch": launch, "recorded_at": T1})
        );
        assert!(launch.contains("bal-planner-high"), "{launch}");
        assert!(launch.contains("Agent tool"), "{launch}");
        assert!(launch.contains("no model parameter"), "{launch}");

        let again = start(&store, I0, R1, T2);
        assert_eq!(again, first);
        assert_eq!(of_type(&history(&store), WORK_ORDER_ISSUED).len(), 1);

        let mut further = Map::new();
        further.insert("settings".into(), json!(["S"]));
        further.insert("existing_code".into(), json!(true));
        let other = start_in(&store, &project_b(), I0, &route, R4, T1, &further);
        assert_eq!(other["status"], "ok", "{other}");
        assert_eq!(other["settings"], json!(["S"]));
        assert_eq!(other["existing_code"], json!(true));
    }

    fn assert_busy(answer: &Value) {
        assert_eq!(answer["status"], "failed", "{answer}");
        assert_eq!(answer["code"], "ledger-busy", "{answer}");
        assert_eq!(answer["recorded"], false, "{answer}");
    }

    #[test]
    fn a_start_order_recorded_beside_a_pending_one_or_after_the_approval_is_caught() {
        let (_dir, store) = two_projects();
        assert_eq!(start(&store, I0, R1, T1)["status"], "ok");
        let before = head(&store);
        assert_busy(&start(&store, I1, R2, T2));
        assert_eq!(head(&store), before);

        let digest = receipt(&submit_in(&store, &project_b(), &m(), R1, 1, T1).unwrap())["digest"]
            .as_str()
            .and_then(Hash::from_hex)
            .unwrap();
        approve_draft_in(&store, &project_b(), digest);
        let before_b = head_of(&store, &project_b());
        let refused = start_in(
            &store,
            &project_b(),
            I0,
            &route_for(None),
            R3,
            T4,
            &Map::new(),
        );
        assert_busy(&refused);
        assert_eq!(head_of(&store, &project_b()), before_b);

        append(
            &store,
            R9,
            T5,
            vec![NewEvent {
                stream: StreamName("roadmap".into()),
                type_name: WORK_ORDER_FAILED.into(),
                type_version: 1,
                git: None,
                payload: failed_payload(I0, "the planner stopped"),
                attachments: vec![],
            }],
        );
        let after = start(&store, I1, R5, T6);
        assert_eq!(after["status"], "ok", "{after}");
        assert_eq!(after["work_order"], I1);
    }

    #[test]
    fn an_issue_inside_a_callers_transaction_lost_or_split_is_caught() {
        use baley_core::questions::{QUESTIONS_OPENED, judge_set};
        use baley_store::{Decision, Observed, OutcomeKind};

        use crate::questions::record::open;
        use crate::questions::record::tests::{declare, story};

        let (_dir, store) = store();
        declare(&store, &project(), 1);
        let policy =
            effective_policy(Schema::standard(), Some(Host::ClaudeCode), None, None).unwrap();
        let names = AcceptedNames {
            names: BTreeSet::new(),
            version: 0,
        };
        let route = compose(
            &policy,
            0,
            Host::ClaudeCode,
            Role::Analyzer,
            NonZeroU32::MIN,
            &names,
        )
        .unwrap();
        let instructions = [crate::instruction::evidence("bal-analyzer-drafting").unwrap()];
        let inputs = [Input::Bytes {
            name: "story",
            bytes: b"{}",
        }];
        let empty = judge_set(&json!([])).unwrap();
        // Opens an empty set of story 1 and issues `id` beside it, then
        // ends the decision with `fails` deciding whether it errs.
        let run = |id: &str, request_id: &str, fails: bool| {
            let issue = Issue {
                id,
                request: id,
                parent: None,
                subject: "story/1",
                route: &route,
                instructions: &instructions,
                inputs: &inputs,
                absences: &[],
                streams: &[],
                documents: &[],
                further: &Map::new(),
            };
            let mut members = None;
            let result = store.transact(&command("test.close", request_id, 3, T2), &mut |tx| {
                open(tx, &empty, &story(1), T2)?;
                members = Some(issue_within(tx, &issue, T2)?);
                if fails {
                    return Err(StoreError::Unavailable(
                        "the caller's own step failed".into(),
                    ));
                }
                Ok(Decision {
                    kind: OutcomeKind::Done,
                    answer: json!({"status": "ok"}),
                    sensitive: false,
                    observed: Observed::default(),
                    git: None,
                })
            });
            (result, members)
        };

        let (result, members) = run(I0, R1, false);
        assert!(result.is_ok(), "{result:?}");
        let members = members.expect("the decision ran");
        assert_eq!(members["work_order"], I0);
        assert_eq!(members["agent"], "bal-analyzer-high");
        assert_eq!(members["recorded_at"], T2);
        assert!(members.get("status").is_none(), "{members:?}");
        let events = history(&store);
        let opened = of_type(&events, QUESTIONS_OPENED);
        let issued = of_type(&events, WORK_ORDER_ISSUED);
        assert_eq!((opened.len(), issued.len()), (1, 1));
        assert_eq!(opened[0].request_id.0, R1);
        assert_eq!(issued[0].request_id.0, R1);
        assert_eq!(issued[0].payload["id"], I0);
        assert_eq!(issued[0].stream, "roadmap");

        let before = head(&store);
        let (result, _) = run(I1, R2, true);
        assert!(result.is_err(), "{result:?}");
        assert_eq!(head(&store), before);
        let events = history(&store);
        assert_eq!(of_type(&events, QUESTIONS_OPENED).len(), 1);
        assert_eq!(of_type(&events, WORK_ORDER_ISSUED).len(), 1);
        assert_eq!(order(&store, &project(), I1), None);
    }

    #[test]
    fn a_launch_line_dropping_the_route_model_is_caught() {
        let (_dir, store) = store();
        let route = route_for(Some("claude-opus-5-5"));
        let answer = start_in(&store, &project(), I0, &route, R1, T1, &Map::new());
        assert_eq!(answer["model"], "claude-opus-5-5");
        let launch = answer["launch"].as_str().unwrap();
        assert!(launch.contains("bal-planner-high"), "{launch}");
        assert!(
            launch.contains("pass claude-opus-5-5 as the Agent tool's model parameter"),
            "{launch}"
        );
    }

    #[test]
    fn an_effective_value_recorded_as_known_or_a_listed_model_without_effort_given_a_level_is_caught()
     {
        let (_dir, store) = two_projects();
        let id_c = ProjectId("8b3e4c6a-0d1f-4a2b-9c3d-4e5f6a7b8c9d".into());
        let id_d = ProjectId("9c4f5d7b-1e2a-4b3c-8d4e-5f6a7b8c9d0e".into());
        store.create_project(&id_c, "third", T0).unwrap();
        store.create_project(&id_d, "fourth", T0).unwrap();
        let cases = [
            (project(), None, "no-model", "unknown"),
            (project_b(), Some("opus"), "alias", "unknown"),
            (id_c, Some("claude-opus-5-5"), "unobserved-id", "high"),
            (
                id_d,
                Some("claude-haiku-4-5-20251001"),
                "unobserved-id",
                "not-applicable",
            ),
        ];
        for (project, model, because, supported) in cases {
            let route = route_for(model);
            let answer = start_in(&store, &project, I0, &route, R1, T1, &Map::new());
            assert_eq!(answer["status"], "ok", "{answer}");
            let document = order(&store, &project, I0).expect("the order is held");
            let route = &document["route"];
            assert_eq!(route["effective_model"], "unknown", "{model:?}");
            assert_eq!(route["effective_effort"], "unknown", "{model:?}");
            assert_eq!(
                route["unknown_because"],
                json!([because, "possible-host-cap"]),
                "{model:?}"
            );
            assert_eq!(route["supported_effort"], supported, "{model:?}");
        }
    }
}
