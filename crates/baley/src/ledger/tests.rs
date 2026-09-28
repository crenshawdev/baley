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
rejects!(verify_requires_a_witness_choice, &["verify", "P"]);
rejects!(
    verify_cannot_combine_remote_and_local,
    &["verify", "P", "--remote", "o", "--local-only"]
);
rejects!(
    verify_views_cannot_fetch,
    &["verify", "P", "--views", "--remote", "o"]
);
rejects!(export_requires_a_destination, &["export", "P"]);
rejects!(purge_requires_a_reason, &["purge", "P", &"ab".repeat(32)]);
rejects!(purge_requires_hashes, &["purge", "P", "--reason", "r"]);
rejects!(anchor_requires_a_remote, &["anchor", "P"]);
#[test]
fn verify_binds_project_and_remote() {
    assert!(
        matches!(parse(&["verify","P","--remote","origin"]).unwrap(),LedgerCommand::Verify { project,remote:Some(remote),views:false,local_only:false } if project == "P" && remote == "origin")
    );
}
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
fn acknowledge_restore_is_on_the_surface() {
    assert!(
        matches!(parse(&["acknowledge-restore","P","--remote","o"]).unwrap(),LedgerCommand::AcknowledgeRestore { project,remote } if project == "P" && remote == "o")
    );
}

const UNSET: &str =
    "baley: BALEY_HOME is not set; this build opens the ledger only at $BALEY_HOME/baley.db";
#[test]
fn an_unset_home_cannot_choose_a_default() {
    assert_eq!(home::home_from(None).unwrap_err().to_string(), UNSET);
}
#[test]
fn an_empty_home_cannot_choose_a_default() {
    assert_eq!(
        home::home_from(Some("".into())).unwrap_err().to_string(),
        UNSET
    );
}
#[test]
fn a_file_is_not_a_home() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, "").unwrap();
    assert_eq!(
        home::home_from(Some(file.clone().into()))
            .unwrap_err()
            .to_string(),
        format!(
            "baley: BALEY_HOME is {}, which is not a directory",
            file.display()
        )
    );
}
#[test]
fn a_missing_ledger_cannot_be_created_by_a_typo() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        home::ledger_file(dir.path()).unwrap_err().to_string(),
        format!(
            "baley: no ledger at {}/baley.db; this build does not create one",
            dir.path().display()
        )
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
    let r = display::anchor_error(&StoreError::Busy, "REQ", "P", "origin");
    assert_eq!(r.code, 3);
    assert!(text(&r).contains("anchor request REQ failed"));
    assert!(text(&r).contains("baley anchor P --remote origin reconciles"));
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
    assert!(text(&r).contains("acknowledged restore at 1"));
}
#[test]
fn view_differences_are_named() {
    let r = display::views(&ViewsReport {
        checked_seq: 9,
        differing: vec![("request".into(), DocKey(vec![KeyValue::Text("R".into())]))],
    });
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("request DocKey([Text(\"R\")])"));
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
        "o",
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
fn uuid_sets_version_and_variant_bits() {
    assert_eq!(
        ids::uuid_v4_text([0; 16]),
        "00000000-0000-4000-8000-000000000000"
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
    assert_eq!(text(&r), "anchor blocked by request held (Active)");
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
                answer: Answer::Inline(json!({"reason":"empty-chain"})),
            },
            head: head(),
        },
        vec![],
    );
    assert_eq!(r.code, 1);
    assert_eq!(text(&r), "nothing to anchor: the project has no events");
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
