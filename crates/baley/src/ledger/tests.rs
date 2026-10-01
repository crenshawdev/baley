use super::*;
use baley_core::*;
use baley_store::*;
use clap::Parser;
use serde_json::json;

#[derive(Parser)]
struct Arguments {
    #[command(subcommand)]
    command: LedgerCommand,
}
fn parse(args: &[&str]) -> Result<LedgerCommand, clap::Error> {
    Arguments::try_parse_from(std::iter::once("baley").chain(args.iter().copied()))
        .map(|a| a.command)
}
macro_rules! rejects {
    ($name:ident, $args:expr) => {
        #[test]
        fn $name() {
            assert!(parse($args).is_err());
        }
    };
}
fn exit_code(args: &[&str]) -> i32 {
    parse(args).unwrap_err().exit_code()
}
#[test]
fn verify_with_no_flag_is_the_anchored_form_with_or_without_a_project() {
    assert!(matches!(
        parse(&["verify"]).unwrap(),
        LedgerCommand::Verify {
            project: None,
            local_only: false,
            views: false
        }
    ));
    assert!(
        matches!(parse(&["verify", "P"]).unwrap(), LedgerCommand::Verify { project: Some(p), local_only: false, views: false } if p == "P")
    );
}
#[test]
fn verify_local_only_and_views_parse_with_a_project() {
    assert!(
        matches!(parse(&["verify", "--local-only", "P"]).unwrap(), LedgerCommand::Verify { project: Some(p), local_only: true, views: false } if p == "P")
    );
    assert!(
        matches!(parse(&["verify", "P", "--views"]).unwrap(), LedgerCommand::Verify { project: Some(p), local_only: false, views: true } if p == "P")
    );
}
#[test]
fn verify_flags_without_a_project_exit_two() {
    assert_eq!(exit_code(&["verify", "--local-only"]), 2);
    assert_eq!(exit_code(&["verify", "--views"]), 2);
}
#[test]
fn verify_flags_cannot_be_combined() {
    assert_eq!(exit_code(&["verify", "P", "--local-only", "--views"]), 2);
}
#[test]
fn every_remote_flag_is_gone_from_the_commands_that_had_one() {
    assert_eq!(exit_code(&["verify", "P", "--remote", "o"]), 2);
    assert_eq!(exit_code(&["verify", "--remote", "o"]), 2);
}
#[test]
fn the_commands_that_keep_their_project_exit_two_without_it() {
    let hash = "ab".repeat(32);
    assert_eq!(exit_code(&["export", "--to", "/x"]), 2);
    assert_eq!(exit_code(&["purge", &hash, "--reason", "r"]), 2);
    assert_eq!(exit_code(&["rebuild"]), 2);
}
#[test]
fn the_arguments_pick_the_form_verify_runs_in() {
    use command_plan::{Form, verify_form};
    assert_eq!(
        verify_form(None, false, false),
        Ok(Form::Anchored { named: None })
    );
    assert_eq!(
        verify_form(Some("P"), false, false),
        Ok(Form::Anchored { named: Some("P") })
    );
    assert_eq!(
        verify_form(Some("P"), true, false),
        Ok(Form::LocalOnly("P"))
    );
    assert_eq!(verify_form(Some("P"), false, true), Ok(Form::Views("P")));
    assert!(verify_form(None, true, false).is_err());
    assert!(verify_form(Some("P"), true, true).is_err());
}
rejects!(export_requires_a_destination, &["export", "P"]);
rejects!(purge_requires_a_reason, &["purge", "P", &"ab".repeat(32)]);
rejects!(purge_requires_hashes, &["purge", "P", "--reason", "r"]);
#[test]
fn doctor_splits_project_from_remote() {
    assert!(
        matches!(parse(&["doctor","--remote","P=origin","--local-only","Q"]).unwrap(),LedgerCommand::Doctor { remote,local_only } if remote == [("P".into(),"origin".into())] && local_only == ["Q"])
    );
}
#[test]
fn doctor_rejects_an_unsplit_remote() {
    assert!(
        parse(&["doctor", "--remote", "P"])
            .unwrap_err()
            .to_string()
            .contains("expected PROJECT=REMOTE")
    );
}
#[test]
fn purge_parses_all_hash_bytes() {
    assert!(
        matches!(parse(&["purge","P",&"ab".repeat(32),"--reason","r"]).unwrap(),LedgerCommand::Purge { hashes,.. } if hashes == [Hash([0xab;32])])
    );
}
#[test]
fn purge_rejects_a_short_hash() {
    assert!(
        parse(&["purge", "P", "abc", "--reason", "r"])
            .unwrap_err()
            .to_string()
            .contains("a payload hash is 64 hex digits")
    );
}
#[test]
fn purge_rejects_an_empty_reason() {
    assert!(
        parse(&["purge", "P", &"ab".repeat(32), "--reason", ""])
            .unwrap_err()
            .to_string()
            .contains("a purge needs a non-empty --reason")
    );
}
#[test]
fn acknowledge_restore_parses_with_and_without_a_project() {
    assert!(matches!(
        parse(&["acknowledge-restore"]).unwrap(),
        LedgerCommand::AcknowledgeRestore { project: None }
    ));
    assert!(
        matches!(parse(&["acknowledge-restore", "P"]).unwrap(), LedgerCommand::AcknowledgeRestore { project: Some(p) } if p == "P")
    );
}
#[test]
fn acknowledge_restore_no_longer_takes_a_remote_flag() {
    assert_eq!(
        parse(&["acknowledge-restore", "P", "--remote", "o"])
            .unwrap_err()
            .exit_code(),
        2
    );
}

#[test]
fn a_configured_remote_matches_a_whole_line() {
    assert!(remotes::configured("dead\norigin\n", "origin"));
}
#[test]
fn a_remote_prefix_is_not_configured() {
    assert!(!remotes::configured("origin\n", "orig"));
}
fn projects() -> Vec<(ProjectId, String)> {
    vec![
        (ProjectId("P".into()), "One".into()),
        (ProjectId("Q".into()), "Two".into()),
    ]
}
#[test]
fn doctor_keeps_every_named_project() {
    let plan =
        remotes::anchor_plan(&projects(), &[("P".into(), "origin".into())], &["Q".into()]).unwrap();
    assert_eq!(
        plan,
        std::collections::BTreeMap::from([
            (ProjectId("P".into()), Some("origin".into())),
            (ProjectId("Q".into()), None)
        ])
    );
}
#[test]
fn doctor_refuses_an_unnamed_project() {
    assert_eq!(
        remotes::anchor_plan(&projects(), &[], &["P".into()])
            .unwrap_err()
            .to_string(),
        "baley: doctor needs --remote Q=REMOTE or --local-only Q for project Q (Two)"
    );
}
#[test]
fn doctor_refuses_a_duplicate_project() {
    assert_eq!(
        remotes::anchor_plan(&projects(), &[("P".into(), "o".into())], &["P".into()])
            .unwrap_err()
            .to_string(),
        "baley: project P is named more than once"
    );
}
#[test]
fn doctor_refuses_an_unknown_project() {
    assert_eq!(
        remotes::anchor_plan(&projects(), &[], &["Z".into()])
            .unwrap_err()
            .to_string(),
        "baley: project Z is not in the ledger"
    );
}
fn with_user(ids: &[&str]) -> Vec<(ProjectId, String)> {
    ids.iter()
        .map(|id| (ProjectId((*id).into()), format!("{id} name")))
        .collect()
}
#[test]
fn doctor_does_not_refuse_until_the_owner_names_user() {
    let plan = remotes::anchor_plan(
        &with_user(&["P", "user"]),
        &[("P".into(), "origin".into())],
        &[],
    )
    .unwrap();
    assert_eq!(
        plan,
        std::collections::BTreeMap::from([
            (ProjectId("P".into()), Some("origin".into())),
            (ProjectId("user".into()), None)
        ])
    );
}
#[test]
fn doctor_takes_an_explicit_local_only_user_without_a_duplicate() {
    let plan = remotes::anchor_plan(
        &with_user(&["P", "user"]),
        &[("P".into(), "origin".into())],
        &["user".into()],
    )
    .unwrap();
    assert_eq!(
        plan,
        std::collections::BTreeMap::from([
            (ProjectId("P".into()), Some("origin".into())),
            (ProjectId("user".into()), None)
        ])
    );
}
#[test]
fn doctor_still_refuses_an_unnamed_project_beside_user_and_not_user() {
    assert_eq!(
        remotes::anchor_plan(
            &with_user(&["P", "Q", "user"]),
            &[("P".into(), "origin".into())],
            &[]
        )
        .unwrap_err()
        .to_string(),
        "baley: doctor needs --remote Q=REMOTE or --local-only Q for project Q (Q name)"
    );
}

fn clean() -> VerifyReport {
    VerifyReport {
        chain: ChainReport {
            head: Some(Head {
                seq: 50,
                hash: Hash([5; 32]),
            }),
            first_break: None,
            anchor: AnchorVerdict::Matches,
            unanchored: None,
            acknowledged_restores: vec![],
            age_unanchored_since: None,
        },
        payloads: vec![],
        bodies_checked: 4,
        tombstones_checked: 0,
        stored_anchor: None,
        stored_anchor_comparison: StoredAnchorComparison::NotCompared,
    }
}
fn text(r: &display::Render) -> String {
    r.lines.join("\n")
}
#[test]
fn refused_requests_exit_two() {
    assert_eq!(
        display::exit_class(&StoreError::Refused(Refusal::UnknownProject(ProjectId(
            "P".into()
        )))),
        2
    );
}
#[test]
fn an_unverified_export_exits_one() {
    assert_eq!(
        display::exit_class(&StoreError::Refused(Refusal::ExportUnverified {
            project: ProjectId("P".into()),
            report: Box::new(clean())
        })),
        1
    );
}
#[test]
fn busy_exits_three() {
    assert_eq!(display::exit_class(&StoreError::Busy), 3);
}
#[test]
fn anchor_errors_do_not_promise_a_clean_noop() {
    let r = display::anchor_error(&StoreError::Busy, "REQ", Some("origin"));
    assert_eq!(r.code, 3);
    assert!(text(&r).contains("anchor request REQ failed"));
    assert!(text(&r).contains("tag may have reached origin"));
    assert!(text(&r).contains("running baley anchor again from this checkout reconciles"));
    assert!(!text(&r).contains("--remote"));
    assert!(!text(&r).contains("nothing was recorded"));
}
#[test]
fn a_matching_chain_is_successful() {
    let r = display::verify_report(&clean());
    assert_eq!(r.code, 0);
    assert!(text(&r).contains("matches the remote anchor"));
}
#[test]
fn truncation_is_a_finding() {
    let mut report = clean();
    report.chain.anchor = AnchorVerdict::Truncated {
        anchored: 60,
        head: 50,
    };
    let r = display::verify_report(&report);
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("truncated: remote anchor at 60, chain ends at 50"));
}
#[test]
fn an_unreachable_remote_is_not_a_clean_check() {
    let r = display::verification(
        &Verification {
            status: AnchorCheck::RemoteUnreachable,
            report: clean(),
            checked_at: "T".into(),
        },
        Some("o"),
    );
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("remote o: unreachable, checked locally only"));
}
#[test]
fn a_corrupt_body_is_named() {
    let mut report = clean();
    report
        .payloads
        .push(PayloadFault::Corrupt(Hash([0xab; 32])));
    let r = display::verify_report(&report);
    assert_eq!(r.code, 1);
    assert!(text(&r).contains(&format!("corrupt payload {}", "ab".repeat(32))));
}
#[test]
fn an_accepted_restore_is_visible_and_successful() {
    let mut report = clean();
    report.chain.anchor = AnchorVerdict::Acknowledged {
        anchored: 60,
        restored: None,
        acknowledged_seq: 1,
    };
    report
        .chain
        .acknowledged_restores
        .push(AcknowledgedRestore {
            seq: 1,
            anchor: Anchor {
                seq: 60,
                hash: Hash([6; 32]),
            },
            restored: None,
        });
    let r = display::verify_report(&report);
    assert_eq!(r.code, 0);
    assert_eq!(text(&r).matches("acknowledged").count(), 1);
    assert!(text(&r).contains("acknowledged at 1"));
}
#[test]
fn an_earlier_acknowledgement_stays_listed_after_a_newer_anchor() {
    let mut report = clean();
    report
        .chain
        .acknowledged_restores
        .push(AcknowledgedRestore {
            seq: 1,
            anchor: Anchor {
                seq: 60,
                hash: Hash([6; 32]),
            },
            restored: None,
        });
    assert!(text(&display::verify_report(&report)).contains("acknowledged restore at 1"));
}
#[test]
fn anchor_row_comparison_is_not_printed_as_a_type_name() {
    let mut report = clean();
    report.stored_anchor_comparison = StoredAnchorComparison::MissingLocal;
    let r = text(&display::verify_report(&report));
    assert!(r.contains("local anchor row: missing, although the remote holds an anchor"));
    assert!(!r.contains("MissingLocal"));
}
#[test]
fn view_versions_are_not_printed_as_options() {
    let mut h = health();
    if let Ok(raw) = &mut h.projects[0].raw_views {
        raw.views.push(ViewHealth {
            view: "request".into(),
            live_version: None,
            binary_version: 2,
        });
    }
    let r = text(&display::doctor(&h, &projects()));
    assert!(r.contains("view set 2, binary 2"));
    assert!(r.contains("view request: live none, binary 2"));
    assert!(!r.contains("Some("));
}
#[test]
fn claim_states_and_breaks_are_not_printed_as_type_names() {
    assert_eq!(
        display::claim_state_text(ClaimState::AwaitingOwner),
        "awaiting the owner"
    );
    assert_eq!(
        display::break_text(&BreakKind::Sequence { found: 7 }),
        "the row there carries sequence 7"
    );
    assert_eq!(
        display::status_text(&PayloadStatus::Purged {
            reason: "leaked".into()
        }),
        "purged: leaked"
    );
}
#[test]
fn view_differences_are_named() {
    let r = display::views(&ViewsReport {
        checked_seq: 9,
        differing: vec![("request".into(), DocKey(vec![KeyValue::Text("R".into())]))],
    });
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("request \"R\""));
}
fn health() -> Health {
    Health {
        epoch: 1,
        scrub_pending: None,
        integrity: vec!["ok".into()],
        database_bytes: 8192,
        log_bytes: 0,
        projects: vec![ProjectHealth {
            project: ProjectId("P".into()),
            check: AnchorCheck::LocalOnly,
            verify: Ok(clean()),
            remote_absent_local_row: None,
            unanchored: UnanchoredAge::None,
            raw_views: Ok(RawViewHealth {
                views: vec![],
                view_set: (Some(2), 2),
                building: None,
            }),
            views_check: Ok(ViewsReport {
                checked_seq: 50,
                differing: vec![],
            }),
            claims: Ok(ClaimCounts {
                active: 0,
                interrupted: 0,
                awaiting_owner: 0,
            }),
        }],
    }
}
#[test]
fn doctor_prints_integrity_findings() {
    let mut h = health();
    h.integrity = vec!["damaged page".into()];
    let r = display::doctor(&h, &projects());
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("integrity: damaged page"));
}
#[test]
fn doctor_prints_old_unanchored_work() {
    let mut h = health();
    h.projects[0].unanchored = UnanchoredAge::Since {
        since: "T".into(),
        warning: true,
    };
    let r = display::doctor(&h, &projects());
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("since T, more than a day"));
}
#[test]
fn one_verify_error_does_not_hide_another_project() {
    let mut h = health();
    let mut q = h.projects[0].clone();
    q.project = ProjectId("Q".into());
    h.projects.push(q);
    h.projects[0].verify = Err(StoreError::Busy);
    let r = display::doctor(&h, &projects());
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("verify failed: store busy"));
    assert!(text(&r).contains("project Q (Two)"));
}
#[test]
fn doctor_reports_a_vanished_remote_anchor() {
    let mut h = health();
    h.projects[0].remote_absent_local_row = Some(StoredAnchor {
        anchor: Anchor {
            seq: 7,
            hash: Hash([7; 32]),
        },
        tag: "tag".into(),
        pushed_at: "T".into(),
    });
    let r = display::doctor(&h, &projects());
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("remote absent, local anchor 7"));
}
#[test]
fn a_clean_doctor_report_succeeds() {
    assert_eq!(display::doctor(&health(), &projects()).code, 0);
}
#[test]
fn the_log_threshold_itself_is_not_a_warning() {
    let mut h = health();
    h.log_bytes = 8_192_000;
    let r = display::doctor(&h, &projects());
    assert_eq!(r.code, 0);
    assert!(!text(&r).contains("warning:"));
}
#[test]
fn one_byte_above_the_log_threshold_warns() {
    let mut h = health();
    h.log_bytes = 8_192_001;
    let r = display::doctor(&h, &projects());
    assert_eq!(r.code, 1);
    assert!(text(&r).contains(
        "warning: the write-ahead log is 8192001 bytes, more than 8,192,000 (1,000 pages)"
    ));
}
fn purge_report() -> PurgeReport {
    PurgeReport {
        purged: vec![],
        shared: vec![],
        recorded: vec![],
        unreachable: vec![],
        scrubbed: false,
    }
}
#[test]
fn an_incomplete_scrub_names_its_recovery() {
    let r = display::purge(&purge_report());
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("run baley scrub"));
}
#[test]
fn every_purge_keeps_the_rotation_reminder() {
    let mut p = purge_report();
    for scrubbed in [false, true] {
        p.scrubbed = scrubbed;
        assert!(text(&display::purge(&p)).contains("A secret that already reached a review provider, an export or any other system must still be rotated."));
    }
}
#[test]
fn nothing_to_acknowledge_is_a_refusal() {
    let r = display::acknowledgement_error(
        &AcknowledgeRestoreError::NothingToAcknowledge(AnchorCheck::RemoteAbsent),
        "P",
    );
    assert_eq!(r.code, 2);
    assert_eq!(text(&r), "nothing to acknowledge: no anchor yet");
}
#[test]
fn a_recorded_acknowledgement_refusal_keeps_its_reason() {
    let r = display::recorded(
        OutcomeKind::Refused,
        &Ok(json!({"reason":"matches"})),
        &ProjectId("P".into()),
        true,
    );
    assert_eq!(r.code, 1);
    assert_eq!(text(&r), "not acknowledged: matches");
}
#[test]
fn acknowledgement_prints_the_remote_identity() {
    let r = display::recorded(
        OutcomeKind::Done,
        &Ok(json!({"acknowledged":{"seq":60,"head":"ab".repeat(32)}})),
        &ProjectId("P".into()),
        true,
    );
    assert_eq!(r.code, 0);
    assert_eq!(
        text(&r),
        format!(
            "acknowledged: the restored chain is accepted behind remote anchor baley-anchor/P/60 (60 {}); anchoring may resume",
            "ab".repeat(32)
        )
    );
}
#[test]
fn trace_bridge_keeps_every_field() {
    let e = trace::trace_entry(TraceRecord {
        at: "T".into(),
        project: Some(ProjectId("P".into())),
        payload: Some(Hash([7; 32])),
        kind: "kind".into(),
        data: "data".into(),
    });
    assert_eq!(
        e,
        baley_store_sqlite::TraceEntry {
            at: "T".into(),
            project: Some(ProjectId("P".into())),
            payload: Some(Hash([7; 32])),
            kind: "kind".into(),
            data: "data".into()
        }
    );
}

struct Bodies {
    body: Result<Option<Vec<u8>>, StoreError>,
    opened: std::cell::Cell<bool>,
}
impl Payloads for Bodies {
    fn open(&self, _: &Hash) -> Result<PayloadBody<'_>, StoreError> {
        self.opened.set(true);
        match &self.body {
            Ok(Some(bytes)) => Ok(PayloadBody::Present(Box::new(bytes.as_slice()))),
            Ok(None) => Ok(PayloadBody::Gone(PayloadStatus::Purged {
                reason: "secret".into(),
            })),
            Err(e) => Err(e.clone()),
        }
    }
    fn status(&self, _: &Hash) -> Result<PayloadStatus, StoreError> {
        panic!("not used")
    }
}
fn bodies(body: Result<Option<Vec<u8>>, StoreError>) -> Bodies {
    Bodies {
        body,
        opened: std::cell::Cell::new(false),
    }
}
fn reference() -> PayloadRef {
    PayloadRef {
        hash: Hash([1; 32]),
        bytes: 16,
        class: RetentionClass::Record,
    }
}
#[test]
fn inline_answers_do_not_read_payloads() {
    let b = bodies(Err(StoreError::Busy));
    assert_eq!(
        answer::answer_value(&Answer::Inline(json!({"reason":"x"})), &b),
        Ok(json!({"reason":"x"}))
    );
    assert!(!b.opened.get());
}
#[test]
fn stored_answers_are_parsed() {
    assert_eq!(
        answer::answer_value(
            &Answer::Stored(reference()),
            &bodies(Ok(Some(b"{\"reason\":\"x\"}".to_vec())))
        ),
        Ok(json!({"reason":"x"}))
    );
}
#[test]
fn purged_stored_answers_remain_tombstones() {
    assert_eq!(
        answer::answer_value(&Answer::Stored(reference()), &bodies(Ok(None))),
        Err(answer::AnswerUnread::Gone(PayloadStatus::Purged {
            reason: "secret".into()
        }))
    );
}
#[test]
fn explicit_tombstones_do_not_read_payloads() {
    let b = bodies(Err(StoreError::Busy));
    let status = PayloadStatus::Purged {
        reason: "secret".into(),
    };
    assert_eq!(
        answer::answer_value(
            &Answer::Tombstone {
                reference: reference(),
                status: status.clone()
            },
            &b
        ),
        Err(answer::AnswerUnread::Gone(status))
    );
    assert!(!b.opened.get());
}
#[test]
fn malformed_answer_bytes_cannot_be_a_reason() {
    assert_eq!(
        answer::answer_value(
            &Answer::Stored(reference()),
            &bodies(Ok(Some(b"bad".to_vec())))
        ),
        Err(answer::AnswerUnread::Malformed)
    );
}
#[test]
fn answer_read_failures_keep_the_store_error() {
    assert_eq!(
        answer::answer_value(&Answer::Stored(reference()), &bodies(Err(StoreError::Busy))),
        Err(answer::AnswerUnread::Read(StoreError::Busy))
    );
}
#[test]
fn unread_refused_answers_still_report_the_recorded_outcome() {
    let r = display::recorded(
        OutcomeKind::Refused,
        &Err(answer::AnswerUnread::Malformed),
        &ProjectId("P".into()),
        false,
    );
    assert_eq!(r.code, 1);
    assert_eq!(
        text(&r),
        "the outcome was recorded as refused; its answer could not be read: malformed answer"
    );
}

fn outcome(done: bool) -> Outcome {
    Outcome {
        kind: if done {
            OutcomeKind::Done
        } else {
            OutcomeKind::Refused
        },
        answer: Answer::Inline(if done {
            json!({"anchored":{"seq":5,"head":"a","tag":"tag","remote":"o","observed_at":"T"}})
        } else {
            json!({"failed":{"reason":"denied"}})
        }),
    }
}
fn anchor_render(outcome: AnchorOutcome, renewal_errors: Vec<StoreError>) -> display::Render {
    let answer = match &outcome {
        AnchorOutcome::Recorded { outcome, .. }
        | AnchorOutcome::Refused { outcome, .. }
        | AnchorOutcome::LateReplay { outcome, .. }
        | AnchorOutcome::Replayed(outcome) => Some(answer::answer_value(
            &outcome.answer,
            &bodies(Err(StoreError::Busy)),
        )),
        _ => None,
    };
    display::anchor(
        &AnchorReport {
            outcome,
            renewal_errors,
            payload_faults: vec![],
        },
        answer.as_ref(),
        &ProjectId("P".into()),
        "REQ",
    )
}
fn head() -> Head {
    Head {
        seq: 7,
        hash: Hash([7; 32]),
    }
}
fn claim() -> Claim {
    Claim {
        id: ClaimId {
            kind: CommandKind("anchor.push".into()),
            request_id: RequestId("held".into()),
        },
        seq: 6,
        claimed_at: "T".into(),
        intent: json!({}),
        scope: vec!["anchor".into()],
        owner: ClaimOwner {
            process: "pid".into(),
            host_session: "cli".into(),
            started_at: "T".into(),
        },
        lease_renewed_at: None,
        awaiting_owner: None,
    }
}
#[test]
fn a_recorded_anchor_done_is_success() {
    let r = anchor_render(
        AnchorOutcome::Recorded {
            outcome: outcome(true),
            head: head(),
        },
        vec![],
    );
    assert_eq!(r.code, 0);
    assert_eq!(
        text(&r),
        "anchored sequence 5 (a) as tag on o, confirmed at T"
    );
}
#[test]
fn a_recorded_anchor_refusal_is_a_finding() {
    let r = anchor_render(
        AnchorOutcome::Recorded {
            outcome: outcome(false),
            head: head(),
        },
        vec![],
    );
    assert_eq!(r.code, 1);
    assert_eq!(text(&r), "anchor failed: denied");
}
#[test]
fn a_late_push_does_not_replace_the_reconciled_outcome() {
    let r = anchor_render(
        AnchorOutcome::LateReplay {
            outcome: outcome(false),
            pushed: true,
        },
        vec![],
    );
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("anchor failed: denied\nthis run's push reported the tag landed; it may have landed after reconciliation found it absent"));
}
#[test]
fn a_late_failed_push_is_not_reported_as_landed() {
    let r = anchor_render(
        AnchorOutcome::LateReplay {
            outcome: outcome(false),
            pushed: false,
        },
        vec![],
    );
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("this run's push did not report the tag landed"));
}
#[test]
fn a_replayed_refusal_remains_refused() {
    let r = anchor_render(AnchorOutcome::Replayed(outcome(false)), vec![]);
    assert_eq!(r.code, 1);
    assert_eq!(
        text(&r),
        "request REQ was answered before:\nanchor failed: denied"
    );
}
#[test]
fn a_replayed_success_remains_successful() {
    let r = anchor_render(AnchorOutcome::Replayed(outcome(true)), vec![]);
    assert_eq!(r.code, 0);
    assert!(text(&r).contains("request REQ was answered before:\nanchored sequence 5"));
}
#[test]
fn an_open_claim_is_not_reported_as_a_push() {
    let r = anchor_render(AnchorOutcome::InProgress(claim()), vec![]);
    assert_eq!(r.code, 1);
    assert_eq!(
        text(&r),
        "request held already holds an open anchor claim since T; nothing more was done"
    );
}
#[test]
fn a_blocked_anchor_names_its_holder() {
    let r = anchor_render(
        AnchorOutcome::Blocked(Block {
            claim: claim().id,
            state: ClaimState::Active,
        }),
        vec![],
    );
    assert_eq!(r.code, 1);
    assert_eq!(text(&r), "anchor blocked by request held (active)");
}
#[test]
fn an_unknown_remote_leaves_the_claim_visible() {
    let r = anchor_render(AnchorOutcome::RemoteUnknown(claim().id), vec![]);
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("an interrupted anchor claim held could not be reconciled"));
}
#[test]
fn an_empty_project_refusal_names_the_absent_events() {
    let r = anchor_render(
        AnchorOutcome::Refused {
            outcome: Outcome {
                kind: OutcomeKind::Refused,
                answer: Answer::Inline(json!({"refused":"empty-chain"})),
            },
            head: head(),
        },
        vec![],
    );
    assert_eq!(r.code, 1);
    assert_eq!(text(&r), "nothing to anchor: the project has no events");
}
fn refused_with(answer: serde_json::Value) -> Outcome {
    Outcome {
        kind: OutcomeKind::Refused,
        answer: Answer::Inline(answer),
    }
}
const NO_REMOTE_LINE: &str = "the project sets no git.remote; set it in baley.toml and commit it, then baley anchor can push";
#[test]
fn a_no_remote_refusal_names_git_remote_and_is_not_a_malformed_answer() {
    let r = anchor_render(
        AnchorOutcome::Refused {
            outcome: refused_with(json!({"refused":"no-remote"})),
            head: head(),
        },
        vec![],
    );
    assert_eq!(r.code, 1);
    assert!(r.error);
    assert_eq!(text(&r), NO_REMOTE_LINE);
    assert!(!text(&r).contains("malformed"));
}
#[test]
fn a_replayed_no_remote_refusal_names_git_remote_after_the_replay_line() {
    let r = anchor_render(
        AnchorOutcome::Replayed(refused_with(json!({"refused":"no-remote"}))),
        vec![],
    );
    assert_eq!(r.code, 1);
    assert_eq!(
        text(&r),
        format!("request REQ was answered before:\n{NO_REMOTE_LINE}")
    );
}
#[test]
fn an_unknown_refusal_code_is_named_not_called_malformed() {
    let r = anchor_render(
        AnchorOutcome::Refused {
            outcome: refused_with(json!({"refused":"later-code"})),
            head: head(),
        },
        vec![],
    );
    assert_eq!(r.code, 1);
    assert_eq!(text(&r), "anchor refused: later-code");
}
#[test]
fn a_lost_reconciliation_is_a_finding() {
    let r = anchor_render(AnchorOutcome::ReconcileLost(StoreError::Busy), vec![]);
    assert_eq!(r.code, 1);
    assert_eq!(text(&r), "anchor reconciliation lost: store busy, retry");
}
#[test]
fn renewal_errors_do_not_cancel_a_successful_anchor() {
    let r = anchor_render(
        AnchorOutcome::Recorded {
            outcome: outcome(true),
            head: head(),
        },
        vec![StoreError::Busy],
    );
    assert_eq!(r.code, 0);
    assert!(text(&r).contains("warning: lease renewal failed: store busy, retry"));
}
#[test]
fn unknown_project_refusal_is_plain() {
    assert_eq!(
        text(&display::store_error(
            &StoreError::Refused(Refusal::UnknownProject(ProjectId("P".into()))),
            None
        )),
        "project P is not in the ledger"
    );
}
#[test]
fn existing_export_refusal_explains_the_new_directory() {
    assert_eq!(
        text(&display::store_error(
            &StoreError::Refused(Refusal::TargetExists("/target".into())),
            None
        )),
        "/target already exists; export creates a new directory"
    );
}
#[test]
fn nothing_to_purge_names_the_project_and_hash() {
    assert_eq!(
        text(&display::store_error(
            &StoreError::Refused(Refusal::NothingToPurge(Hash([1; 32]))),
            Some("P")
        )),
        format!(
            "project P holds no reference to {} left to purge; nothing was purged",
            "01".repeat(32)
        )
    );
}
#[test]
fn busy_refusal_names_the_retry() {
    assert_eq!(
        text(&display::store_error(&StoreError::Busy, None)),
        "the ledger is busy; run the command again"
    );
}
#[test]
fn read_only_refusal_names_the_needed_epoch() {
    assert_eq!(
        text(&display::store_error(
            &StoreError::ReadOnly { needed_epoch: 3 },
            None
        )),
        "the ledger is read-only for this build; a build at epoch 3 is needed"
    );
}

#[test]
fn unsafe_home_renders_every_fix_as_a_refusal_not_debug() {
    let error = StoreError::Refused(Refusal::UnsafeHome(vec![
        HomeFault {
            path: "/h".into(),
            target: FaultTarget::Home,
            problem: HomeProblem::Mode {
                mode: 0o755,
                allowed: 0o700,
            },
        },
        HomeFault {
            path: "/h/baley.db".into(),
            target: FaultTarget::File,
            problem: HomeProblem::Link,
        },
    ]));
    let rendered = display::store_error(&error, None);
    assert_eq!(
        rendered.lines,
        [
            "unsafe-home: the ledger's home is not safe to open",
            "/h has mode 0755, which allows more than 0700 (fix: chmod 700 /h)",
            "/h/baley.db is a symbolic link (fix: replace the link with the real file)",
        ]
    );
    assert_eq!(rendered.code, 2);
    assert!(rendered.error);
}

const CHECKOUT_FILE: &str = "/r/baley.toml";
const PURGE_HINT: &str = "a purge run outside a checkout of the project records policy version 0";
fn checkout_file(text: &str) -> policy::SettingsFile {
    crate::settings::file(
        std::path::Path::new(CHECKOUT_FILE),
        text.as_bytes().to_vec(),
    )
}
#[test]
fn purge_does_not_take_a_checkout_file_without_an_id_as_no_project() {
    let path = std::path::Path::new(CHECKOUT_FILE);
    let bad_id = "[project]\nid = \"0B5C1F6E-2A7D-4C3E-9F10-5A6B7C8D9E0F\"\nname = \"r\"\n";
    let observations = [
        (
            "not TOML",
            crate::init::observe_file(Ok(Some(checkout_file("[project\nid = ")))),
        ),
        (
            "a bad id",
            crate::init::observe_file(Ok(Some(checkout_file(bad_id)))),
        ),
        (
            "removed since the walk",
            crate::init::observe_file(Ok(None)),
        ),
    ];
    for (case, observed) in observations {
        let refusal = commands::purge_project(path, observed).unwrap_err();
        assert!(
            refusal.starts_with("config-unavailable: "),
            "{case}: {refusal}"
        );
        assert!(refusal.contains(CHECKOUT_FILE), "{case}: {refusal}");
        assert!(refusal.contains(PURGE_HINT), "{case}: {refusal}");
    }
}
#[test]
fn purge_takes_the_id_of_a_valid_checkout_file() {
    let id = "0b5c1f6e-2a7d-4c3e-9f10-5a6b7c8d9e0f";
    let bytes = policy::render_project(id, "r", None).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let observed = crate::init::observe_file(Ok(Some(checkout_file(&text))));
    let path = std::path::Path::new(CHECKOUT_FILE);
    assert_eq!(commands::purge_project(path, observed), Ok(id.to_string()));
}
#[test]
fn purge_command_does_not_keep_policy_version_0_in_its_digest_or_itself() {
    let hashes = [Hash([0x0a; 32]), Hash([0xab; 32])];
    let command = commands::purge_command(
        &ProjectId("P".into()),
        7,
        &hashes,
        "leaked",
        RequestId("00000000-0000-4000-8000-000000000001".into()),
        "2026-10-01T10:00:00Z",
    )
    .unwrap();
    assert_eq!(command.policy_version, 7);
    let expected = request_digest(&json!({
        "kind": "payload.purge",
        "project": "P",
        "actor": "owner",
        "policy_version": 7,
        "hashes": ["0a".repeat(32), "ab".repeat(32)],
        "reason": "leaked",
        "scope": [],
    }))
    .unwrap();
    assert_eq!(command.digest, expected);
    assert_eq!(command.actor, Actor::Owner);
    assert_eq!(command.kind, CommandKind("payload.purge".into()));
}

const PROJECT_ID: &str = "0b5c1f6e-2a7d-4c3e-9f10-5a6b7c8d9e0f";
const GLOBAL_FILE: &str = "/c/config.toml";
type Observed = Result<Option<policy::ProjectIdentity>, policy::Unavailable>;
fn valid_id() -> Observed {
    let bytes = policy::render_project(PROJECT_ID, "r", None).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    crate::init::observe_file(Ok(Some(checkout_file(&text))))
}
fn head_copy(text: &str) -> Option<Result<crate::committed::Committed, policy::Unavailable>> {
    Some(Ok(crate::committed::Committed {
        layer: Some(checkout_file(text)),
        pending: None,
    }))
}
fn global_file(text: &str) -> Result<Option<policy::SettingsFile>, policy::Unavailable> {
    Ok(Some(crate::settings::file(
        std::path::Path::new(GLOBAL_FILE),
        text.as_bytes().to_vec(),
    )))
}
fn managed(
    id: Observed,
    global: Result<Option<policy::SettingsFile>, policy::Unavailable>,
    head: Option<Result<crate::committed::Committed, policy::Unavailable>>,
) -> command_plan::Settings {
    command_plan::Settings {
        project_file: Some(command_plan::ProjectFile {
            path: CHECKOUT_FILE.into(),
            id,
        }),
        policy: crate::policy_step::build(&crate::policy_step::Reads { global, head }),
    }
}
fn origin_settings() -> command_plan::Settings {
    managed(
        valid_id(),
        Ok(None),
        head_copy("[git]\nremote = \"origin\"\n"),
    )
}
fn unset_settings() -> command_plan::Settings {
    managed(valid_id(), Ok(None), head_copy(""))
}
fn anchor_next(
    settings: Option<&command_plan::Settings>,
    check: Option<&Result<(), String>>,
    named: Option<&str>,
) -> Result<Vec<command_plan::Op>, String> {
    command_plan::next(&command_plan::Facts {
        command: command_plan::Verb::Anchor { named },
        settings,
        remote_check: check,
    })
}
fn anchors(remote: Option<&str>) -> command_plan::Op {
    command_plan::Op::Anchor {
        project: PROJECT_ID.into(),
        remote: remote.map(Into::into),
    }
}
#[test]
fn anchor_parses_with_and_without_a_project() {
    assert!(matches!(
        parse(&["anchor"]).unwrap(),
        LedgerCommand::Anchor { project: None }
    ));
    assert!(
        matches!(parse(&["anchor", "P"]).unwrap(), LedgerCommand::Anchor { project: Some(p) } if p == "P")
    );
}
#[test]
fn anchor_no_longer_takes_a_remote_flag() {
    assert_eq!(
        parse(&["anchor", "P", "--remote", "o"])
            .unwrap_err()
            .exit_code(),
        2
    );
}
#[test]
fn a_policy_names_the_remote_in_heads_copy_and_none_when_unset() {
    let named = origin_settings();
    assert_eq!(
        anchor_plan::remote_of(named.policy.as_ref().unwrap()),
        Some("origin".to_string())
    );
    let unset = unset_settings();
    assert_eq!(anchor_plan::remote_of(unset.policy.as_ref().unwrap()), None);
}
#[test]
fn a_name_equal_to_the_discovered_project_is_taken_and_a_missing_name_takes_it_too() {
    assert_eq!(
        anchor_plan::judge_target(Some("P"), Some("P")),
        Ok(Some("P".into()))
    );
    assert_eq!(
        anchor_plan::judge_target(Some("P"), None),
        Ok(Some("P".into()))
    );
}
#[test]
fn a_name_that_differs_from_the_discovered_project_is_refused_naming_both() {
    assert_eq!(
        anchor_plan::judge_target(Some("P"), Some("Q")),
        Err(anchor_plan::TargetRefusal::Differs {
            named: "Q".into(),
            discovered: "P".into()
        })
    );
}
#[test]
fn a_name_with_no_project_discovered_is_refused_and_no_name_gives_no_project() {
    assert_eq!(
        anchor_plan::judge_target(None, Some("Q")),
        Err(anchor_plan::TargetRefusal::NoneDiscovered { named: "Q".into() })
    );
    assert_eq!(anchor_plan::judge_target(None, None), Ok(None));
}
#[test]
fn anchor_reads_the_settings_before_it_requests_anything_else() {
    assert_eq!(
        anchor_next(None, None, None),
        Ok(vec![command_plan::Op::ReadSettings])
    );
}
#[test]
fn anchor_checks_a_set_remote_before_it_requests_the_step() {
    assert_eq!(
        anchor_next(Some(&origin_settings()), None, None),
        Ok(vec![command_plan::Op::CheckRemote("origin".into())])
    );
}
#[test]
fn anchor_requests_the_step_then_the_anchor_on_the_remote_once_it_is_checked() {
    assert_eq!(
        anchor_next(Some(&origin_settings()), Some(&Ok(())), None),
        Ok(vec![command_plan::Op::Step, anchors(Some("origin"))])
    );
}
#[test]
fn an_unconfigured_remote_refuses_anchor_with_no_operation() {
    let refused = Err("remote origin is not configured in the git repository at /r".to_string());
    assert_eq!(
        anchor_next(Some(&origin_settings()), Some(&refused), None),
        refused.map(|()| vec![])
    );
}
#[test]
fn anchor_with_no_remote_set_requests_the_step_and_an_anchor_with_none_and_no_check() {
    assert_eq!(
        anchor_next(Some(&unset_settings()), None, None),
        Ok(vec![command_plan::Op::Step, anchors(None)])
    );
}
#[test]
fn anchor_refuses_an_invalid_global_file_naming_it_and_requests_nothing() {
    let settings = managed(
        valid_id(),
        global_file("escalate_on_failure = [\n"),
        head_copy("[git]\nremote = \"origin\"\n"),
    );
    let refusal = anchor_next(Some(&settings), None, None).unwrap_err();
    assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
    assert!(refusal.contains(GLOBAL_FILE), "{refusal}");
}
#[test]
fn anchor_refuses_a_wrong_typed_value_in_heads_copy_naming_the_file() {
    let settings = managed(
        valid_id(),
        Ok(None),
        head_copy("escalate_on_failure = \"yes\"\n"),
    );
    let refusal = anchor_next(Some(&settings), None, None).unwrap_err();
    assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
    assert!(refusal.contains(CHECKOUT_FILE), "{refusal}");
}
#[test]
fn anchor_does_not_take_a_checkout_file_without_an_id_as_no_project() {
    let bad_id = "[project]\nid = \"0B5C1F6E-2A7D-4C3E-9F10-5A6B7C8D9E0F\"\nname = \"r\"\n";
    let observations = [
        (
            "not TOML",
            crate::init::observe_file(Ok(Some(checkout_file("[project\nid = ")))),
        ),
        (
            "a bad id",
            crate::init::observe_file(Ok(Some(checkout_file(bad_id)))),
        ),
        (
            "removed since the walk",
            crate::init::observe_file(Ok(None)),
        ),
    ];
    for (case, observed) in observations {
        let settings = managed(observed, Ok(None), head_copy(""));
        let refusal = anchor_next(Some(&settings), None, None).unwrap_err();
        assert!(
            refusal.starts_with("config-unavailable: "),
            "{case}: {refusal}"
        );
        assert!(refusal.contains(CHECKOUT_FILE), "{case}: {refusal}");
    }
}
#[test]
fn anchor_outside_a_checkout_is_refused_with_no_operation() {
    let settings = command_plan::Settings {
        project_file: None,
        policy: crate::policy_step::build(&crate::policy_step::Reads {
            global: Ok(None),
            head: None,
        }),
    };
    let refusal = anchor_next(Some(&settings), None, None).unwrap_err();
    assert!(refusal.contains("no baley.toml was found"), "{refusal}");
    assert!(refusal.contains("baley anchor"), "{refusal}");
}
#[test]
fn anchor_refuses_a_named_project_that_is_not_the_checkouts_and_requests_nothing() {
    let other = "7c1f0a52-3d4e-4b6a-8c9d-1e2f3a4b5c6d";
    let refusal = anchor_next(Some(&origin_settings()), None, Some(other)).unwrap_err();
    assert!(refusal.contains(other), "{refusal}");
    assert!(refusal.contains(PROJECT_ID), "{refusal}");
    assert_eq!(
        anchor_next(Some(&origin_settings()), None, Some(PROJECT_ID)),
        Ok(vec![command_plan::Op::CheckRemote("origin".into())])
    );
}
fn ack_next(
    settings: Option<&command_plan::Settings>,
    check: Option<&Result<(), String>>,
) -> Result<Vec<command_plan::Op>, String> {
    command_plan::next(&command_plan::Facts {
        command: command_plan::Verb::AcknowledgeRestore { named: None },
        settings,
        remote_check: check,
    })
}
#[test]
fn acknowledge_restore_with_no_git_remote_refuses_naming_it_with_no_operation_and_no_check() {
    let refusal = ack_next(Some(&unset_settings()), None).unwrap_err();
    assert!(refusal.contains("git.remote"), "{refusal}");
    assert!(refusal.contains("baley.toml"), "{refusal}");
}
#[test]
fn acknowledge_restore_checks_the_remote_then_requests_the_step_and_the_acknowledgement() {
    assert_eq!(
        ack_next(Some(&origin_settings()), None),
        Ok(vec![command_plan::Op::CheckRemote("origin".into())])
    );
    assert_eq!(
        ack_next(Some(&origin_settings()), Some(&Ok(()))),
        Ok(vec![
            command_plan::Op::Step,
            command_plan::Op::Acknowledge {
                project: PROJECT_ID.into(),
                remote: "origin".into()
            }
        ])
    );
}
#[test]
fn acknowledge_restore_refuses_an_invalid_settings_file_with_config_unavailable() {
    let settings = managed(
        valid_id(),
        global_file("escalate_on_failure = [\n"),
        head_copy(""),
    );
    let refusal = ack_next(Some(&settings), None).unwrap_err();
    assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
    assert!(refusal.contains(GLOBAL_FILE), "{refusal}");
}
#[test]
fn the_acknowledgement_carries_the_step_version_and_remote() {
    let request = commands::acknowledge_request(
        &ProjectId("P".into()),
        "origin",
        7,
        RequestId("00000000-0000-4000-8000-000000000001".into()),
    );
    assert_eq!(request.policy_version, 7);
    assert_eq!(request.remote, "origin");
    assert_eq!(request.actor, Actor::Owner);
}
#[test]
fn a_blocked_acknowledgement_tells_the_owner_to_run_anchor_with_no_remote_flag() {
    let blocked = AcknowledgeRestoreError::Store(StoreError::Blocked(Block {
        claim: claim().id,
        state: ClaimState::Interrupted,
    }));
    let r = display::acknowledgement_error(&blocked, "P");
    assert_eq!(r.code, 2);
    assert!(text(&r).contains("an anchor claim held is interrupted"));
    assert!(text(&r).contains("run baley anchor first from this checkout"));
    assert!(!text(&r).contains("--remote"));
}
fn verify_next(
    form: command_plan::Form<'_>,
    settings: Option<&command_plan::Settings>,
    check: Option<&Result<(), String>>,
) -> Result<Vec<command_plan::Op>, String> {
    command_plan::next(&command_plan::Facts {
        command: command_plan::Verb::Verify(form),
        settings,
        remote_check: check,
    })
}
fn anchored() -> command_plan::Form<'static> {
    command_plan::Form::Anchored { named: None }
}
fn verifies(remote: Option<&str>) -> command_plan::Op {
    command_plan::Op::Verify {
        project: PROJECT_ID.into(),
        remote: remote.map(Into::into),
    }
}
#[test]
fn anchored_verify_never_requests_the_policy_step_at_any_stage() {
    let origin = origin_settings();
    let unset = unset_settings();
    let stages = [
        verify_next(anchored(), None, None),
        verify_next(anchored(), Some(&origin), None),
        verify_next(anchored(), Some(&origin), Some(&Ok(()))),
        verify_next(anchored(), Some(&unset), None),
    ];
    for stage in stages {
        assert!(!stage.unwrap().contains(&command_plan::Op::Step));
    }
}
#[test]
fn anchored_verify_refuses_an_invalid_settings_file_but_the_flagged_forms_do_not() {
    let settings = managed(
        valid_id(),
        global_file("escalate_on_failure = [\n"),
        head_copy(""),
    );
    let refusal = verify_next(anchored(), Some(&settings), None).unwrap_err();
    assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
    assert!(refusal.contains(GLOBAL_FILE), "{refusal}");
    assert_eq!(
        verify_next(command_plan::Form::LocalOnly("P"), Some(&settings), None),
        Ok(vec![command_plan::Op::Verify {
            project: "P".into(),
            remote: None
        }])
    );
    assert_eq!(
        verify_next(command_plan::Form::Views("P"), Some(&settings), None),
        Ok(vec![command_plan::Op::Views("P".into())])
    );
}
#[test]
fn the_flagged_verify_forms_request_no_settings_read_before_any_facts() {
    assert_eq!(
        verify_next(command_plan::Form::LocalOnly("P"), None, None),
        Ok(vec![command_plan::Op::Verify {
            project: "P".into(),
            remote: None
        }])
    );
    assert_eq!(
        verify_next(command_plan::Form::Views("P"), None, None),
        Ok(vec![command_plan::Op::Views("P".into())])
    );
}
#[test]
fn anchored_verify_outside_a_checkout_points_to_the_local_only_form() {
    let outside = command_plan::Settings {
        project_file: None,
        policy: crate::policy_step::build(&crate::policy_step::Reads {
            global: Ok(None),
            head: None,
        }),
    };
    let none = verify_next(anchored(), Some(&outside), None).unwrap_err();
    assert!(none.contains("verify --local-only <project>"), "{none}");
    let named = command_plan::Form::Anchored { named: Some("P") };
    let refusal = verify_next(named, Some(&outside), None).unwrap_err();
    assert!(refusal.contains("verify --local-only P"), "{refusal}");
}
#[test]
fn anchored_verify_with_no_git_remote_verifies_locally_with_no_check() {
    assert_eq!(
        verify_next(anchored(), Some(&unset_settings()), None),
        Ok(vec![verifies(None)])
    );
}
#[test]
fn anchored_verify_checks_the_remote_then_verifies_against_it() {
    assert_eq!(
        verify_next(anchored(), Some(&origin_settings()), None),
        Ok(vec![command_plan::Op::CheckRemote("origin".into())])
    );
    assert_eq!(
        verify_next(anchored(), Some(&origin_settings()), Some(&Ok(()))),
        Ok(vec![verifies(Some("origin"))])
    );
}
#[test]
fn the_anchor_request_carries_the_step_version_and_remote_in_its_digest() {
    let owner = || ClaimOwner {
        process: "1".into(),
        host_session: "cli".into(),
        started_at: "T".into(),
    };
    let id = |n: &str| RequestId(format!("00000000-0000-4000-8000-00000000000{n}"));
    let build = |version| {
        commands::anchor_request(
            &ProjectId("P".into()),
            Some("origin"),
            version,
            id("1"),
            id("2"),
            owner(),
        )
    };
    let request = build(7);
    assert_eq!(request.policy_version, 7);
    assert_eq!(request.remote.as_deref(), Some("origin"));
    assert_ne!(request.digest().unwrap(), build(0).digest().unwrap());
}

/// A stream whose every write fails with one error kind.
struct Failing(std::io::ErrorKind);
impl std::io::Write for Failing {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(self.0.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn report(code: u8) -> display::Render {
    display::Render {
        lines: vec!["a".into(), "b".into()],
        code,
        error: false,
    }
}
#[test]
fn a_closed_reader_is_not_turned_into_a_failure_or_a_panic() {
    let closed = || Failing(std::io::ErrorKind::BrokenPipe);
    for code in [0, 1, 3] {
        assert_eq!(
            display::emit(&report(code), &mut closed(), &mut Vec::new()),
            code
        );
    }
    let refusal = display::Render::refusal("no");
    assert_eq!(display::emit(&refusal, &mut Vec::new(), &mut closed()), 2);
}
#[test]
fn a_failed_write_is_not_reported_as_success() {
    let full = || Failing(std::io::ErrorKind::Other);
    assert_eq!(display::emit(&report(0), &mut full(), &mut Vec::new()), 1);
    assert_eq!(display::emit(&report(3), &mut full(), &mut Vec::new()), 3);
    let refusal = display::Render::refusal("no");
    assert_eq!(display::emit(&refusal, &mut Vec::new(), &mut full()), 2);
}
#[test]
fn refusal_lines_do_not_reach_stdout_or_lose_their_prefix() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(display::emit(&report(0), &mut out, &mut err), 0);
    assert_eq!((out.as_slice(), err.as_slice()), (&b"a\nb\n"[..], &b""[..]));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    display::emit(&display::Render::refusal("no"), &mut out, &mut err);
    assert_eq!(
        (out.as_slice(), err.as_slice()),
        (&b""[..], &b"baley: no\n"[..])
    );
}
#[test]
fn a_closed_reader_does_not_stop_a_command_at_its_request_line() {
    let mut out = Vec::new();
    display::request_line(&mut out, "R");
    assert_eq!(out, b"request R\n");
    display::request_line(&mut Failing(std::io::ErrorKind::BrokenPipe), "R");
}
