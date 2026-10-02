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
fn doctor_takes_no_flags_and_parses_bare() {
    assert!(matches!(parse(&["doctor"]).unwrap(), LedgerCommand::Doctor));
    assert_eq!(exit_code(&["doctor", "--remote", "P=o"]), 2);
    assert_eq!(exit_code(&["doctor", "--local-only", "Q"]), 2);
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
    assert!(anchor_plan::configured("dead\norigin\n", "origin"));
}
#[test]
fn a_remote_prefix_is_not_configured() {
    assert!(!anchor_plan::configured("origin\n", "orig"));
}
fn projects() -> Vec<(ProjectId, String)> {
    vec![
        (ProjectId("P".into()), "One".into()),
        (ProjectId("Q".into()), "Two".into()),
    ]
}

fn no_reasons() -> std::collections::BTreeMap<ProjectId, anchor_plan::LocalReason> {
    std::collections::BTreeMap::new()
}
fn named_remote() -> anchor_plan::RemoteState {
    anchor_plan::RemoteState::Name("origin".into())
}
fn no_faults() -> command_plan::DoctorSettings {
    command_plan::DoctorSettings {
        discovered: None,
        remote: anchor_plan::RemoteState::NotSet,
        faults: vec![],
        diagnostics: vec![],
        pending: None,
    }
}
fn ledger(ids: &[&str]) -> Vec<(ProjectId, String)> {
    ids.iter()
        .map(|id| (ProjectId((*id).into()), format!("{id} name")))
        .collect()
}
fn check_of<'a>(
    plan: &'a anchor_plan::DoctorChecks,
    id: &str,
) -> Option<&'a anchor_plan::CheckAgainst> {
    plan.checks
        .iter()
        .find(|(project, _)| project.0 == id)
        .map(|(_, against)| against)
}
const NOT_DISCOVERED: anchor_plan::CheckAgainst =
    anchor_plan::CheckAgainst::Local(anchor_plan::LocalReason::NotDiscovered);
#[test]
fn doctor_gives_only_the_discovered_project_the_remote_and_user_is_local() {
    let plan = anchor_plan::doctor_checks(Some("P"), &named_remote(), &ledger(&["P", "Q", "user"]));
    assert_eq!(
        check_of(&plan, "P"),
        Some(&anchor_plan::CheckAgainst::Remote("origin".into()))
    );
    assert_eq!(check_of(&plan, "Q"), Some(&NOT_DISCOVERED));
    assert_eq!(check_of(&plan, "user"), Some(&NOT_DISCOVERED));
    assert_eq!(plan.validate.as_deref(), Some("origin"));
}
#[test]
fn doctor_checks_a_discovered_project_with_no_remote_locally_for_want_of_a_forge_remote() {
    let plan = anchor_plan::doctor_checks(
        Some("P"),
        &anchor_plan::RemoteState::NotSet,
        &ledger(&["P", "Q"]),
    );
    assert_eq!(
        check_of(&plan, "P"),
        Some(&anchor_plan::CheckAgainst::Local(
            anchor_plan::LocalReason::NoForgeRemote
        ))
    );
    assert_eq!(plan.validate, None);
}
#[test]
fn doctor_gives_a_discovered_project_missing_from_the_ledger_no_check() {
    let plan = anchor_plan::doctor_checks(
        Some("Z"),
        &anchor_plan::RemoteState::NotSet,
        &ledger(&["P", "Q"]),
    );
    assert_eq!(check_of(&plan, "Z"), None);
    assert_eq!(plan.checks.len(), 2);
    assert_eq!(check_of(&plan, "P"), Some(&NOT_DISCOVERED));
    assert_eq!(check_of(&plan, "Q"), Some(&NOT_DISCOVERED));
}
#[test]
fn doctor_still_validates_the_remote_of_a_discovered_project_missing_from_the_ledger() {
    let plan = anchor_plan::doctor_checks(Some("Z"), &named_remote(), &ledger(&["P", "Q"]));
    assert_eq!(plan.validate.as_deref(), Some("origin"));
    assert_eq!(check_of(&plan, "Z"), None);
}
#[test]
fn doctor_from_outside_a_checkout_checks_every_project_locally_and_validates_nothing() {
    let plan = anchor_plan::doctor_checks(
        None,
        &anchor_plan::RemoteState::NotSet,
        &ledger(&["P", "Q", "user"]),
    );
    for id in ["P", "Q", "user"] {
        assert_eq!(check_of(&plan, id), Some(&NOT_DISCOVERED), "{id}");
    }
    assert_eq!(plan.validate, None);
}
#[test]
fn doctor_labels_a_project_not_checked_from_this_directory() {
    let reasons = std::collections::BTreeMap::from([(
        ProjectId("P".into()),
        anchor_plan::LocalReason::NotDiscovered,
    )]);
    let r = text(&display::doctor(
        &health(),
        &projects(),
        &reasons,
        &no_faults(),
    ));
    assert!(r.contains("project P (One): not checked against a remote from this directory"));
}
#[test]
fn doctor_labels_a_checkout_without_git_remote_apart_from_one_not_discovered() {
    let reasons = std::collections::BTreeMap::from([(
        ProjectId("P".into()),
        anchor_plan::LocalReason::NoForgeRemote,
    )]);
    let r = text(&display::doctor(
        &health(),
        &projects(),
        &reasons,
        &no_faults(),
    ));
    assert!(r.contains("project P (One): local only, no forge remote (git.remote is not set)"));
    assert!(!r.contains("not checked against a remote from this directory"));
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
fn restore_at(seq: u64) -> AcknowledgedRestore {
    AcknowledgedRestore {
        seq,
        anchor: Anchor {
            seq: 60,
            hash: Hash([6; 32]),
        },
        restored: None,
    }
}
fn with_restores(seqs: &[u64]) -> VerifyReport {
    let mut report = clean();
    report.chain.acknowledged_restores = seqs.iter().map(|s| restore_at(*s)).collect();
    report
}
#[test]
fn a_later_matching_anchor_does_not_hide_the_purge_warning() {
    let report = with_restores(&[1]);
    assert!(matches!(report.chain.anchor, AnchorVerdict::Matches));
    let r = display::verify_report(&report);
    assert_eq!(r.code, 0);
    assert_eq!(warnings(&r), 1);
}
#[test]
fn the_purge_warning_prints_once_per_report_not_once_per_acknowledgement() {
    let mut report = with_restores(&[1, 2]);
    report.chain.anchor = AnchorVerdict::Acknowledged {
        anchored: 60,
        restored: None,
        acknowledged_seq: 2,
    };
    let r = display::verify_report(&report);
    assert_eq!(r.code, 0);
    assert_eq!(warnings(&r), 1);
}
#[test]
fn an_acknowledged_verdict_with_no_listed_restore_still_prints_the_purge_warning() {
    let mut report = clean();
    report.chain.anchor = AnchorVerdict::Acknowledged {
        anchored: 60,
        restored: None,
        acknowledged_seq: 1,
    };
    assert!(report.chain.acknowledged_restores.is_empty());
    let r = display::verify_report(&report);
    assert_eq!(r.code, 0);
    assert_eq!(warnings(&r), 1);
}
#[test]
fn a_local_only_check_still_prints_the_purge_warning() {
    let mut report = with_restores(&[1]);
    report.chain.anchor = AnchorVerdict::NoAnchor;
    let r = display::verification(
        &Verification {
            status: AnchorCheck::LocalOnly,
            report,
            checked_at: "T".into(),
        },
        None,
    );
    assert_eq!(r.code, 0);
    assert_eq!(warnings(&r), 1);
}
#[test]
fn an_unacknowledged_truncation_or_rewrite_prints_no_purge_warning() {
    for verdict in [
        AnchorVerdict::Truncated {
            anchored: 60,
            head: 50,
        },
        AnchorVerdict::Rewritten {
            seq: 40,
            anchored: Hash([1; 32]),
            found: Hash([2; 32]),
        },
    ] {
        let mut report = clean();
        report.chain.anchor = verdict;
        let r = display::verify_report(&report);
        assert_eq!(r.code, 1);
        assert_eq!(warnings(&r), 0);
    }
}
#[test]
fn a_report_with_no_accepted_gap_prints_no_purge_warning() {
    for verdict in [AnchorVerdict::Matches, AnchorVerdict::NoAnchor] {
        let mut report = clean();
        report.chain.anchor = verdict;
        let r = display::verify_report(&report);
        assert_eq!(r.code, 0);
        assert_eq!(warnings(&r), 0);
    }
}
#[test]
fn doctor_prints_the_purge_warning_once_for_the_project_that_lists_a_restore() {
    let mut h = health();
    let mut other = h.projects[0].clone();
    other.project = ProjectId("Q".into());
    h.projects[0].verify = Ok(with_restores(&[1]));
    h.projects.push(other);
    let r = display::doctor(&h, &projects(), &no_reasons(), &no_faults());
    assert_eq!(r.code, 0);
    assert_eq!(warnings(&r), 1);
}
#[test]
fn a_failed_export_prints_the_purge_warning_and_exits_one() {
    let r = display::store_error(
        &StoreError::Refused(Refusal::ExportUnverified {
            project: ProjectId("P".into()),
            report: Box::new(with_restores(&[1])),
        }),
        Some("P"),
    );
    assert_eq!(r.code, 1);
    assert_eq!(warnings(&r), 1);
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
    let r = text(&display::doctor(
        &h,
        &projects(),
        &no_reasons(),
        &no_faults(),
    ));
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
    let r = display::doctor(&h, &projects(), &no_reasons(), &no_faults());
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
    let r = display::doctor(&h, &projects(), &no_reasons(), &no_faults());
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
    let r = display::doctor(&h, &projects(), &no_reasons(), &no_faults());
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
    let r = display::doctor(&h, &projects(), &no_reasons(), &no_faults());
    assert_eq!(r.code, 1);
    assert!(text(&r).contains("remote absent, local anchor 7"));
}
#[test]
fn a_clean_doctor_report_succeeds() {
    assert_eq!(
        display::doctor(&health(), &projects(), &no_reasons(), &no_faults()).code,
        0
    );
}
#[test]
fn the_log_threshold_itself_is_not_a_warning() {
    let mut h = health();
    h.log_bytes = 8_192_000;
    let r = display::doctor(&h, &projects(), &no_reasons(), &no_faults());
    assert_eq!(r.code, 0);
    assert!(!text(&r).contains("warning:"));
}
#[test]
fn one_byte_above_the_log_threshold_warns() {
    let mut h = health();
    h.log_bytes = 8_192_001;
    let r = display::doctor(&h, &projects(), &no_reasons(), &no_faults());
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
const WARNING_MARK: &str = "purges recorded in history this copy lacks may be missing";
fn warnings(r: &display::Render) -> usize {
    text(r).matches(WARNING_MARK).count()
}
fn ack_answer() -> serde_json::Value {
    json!({"acknowledged":{"seq":60,"head":"ab".repeat(32)}})
}
fn ack_line() -> String {
    format!(
        "acknowledged: the restored chain is accepted behind remote anchor baley-anchor/P/60 (60 {}); anchoring may resume",
        "ab".repeat(32)
    )
}
fn acknowledged_outcome(answer: Answer) -> Outcome {
    Outcome {
        kind: OutcomeKind::Done,
        answer,
    }
}
#[test]
fn acknowledgement_prints_the_remote_identity() {
    let r = display::recorded(
        OutcomeKind::Done,
        &Ok(ack_answer()),
        &ProjectId("P".into()),
        true,
    );
    assert_eq!(r.code, 0);
    assert_eq!(r.lines[0], ack_line());
}
#[test]
fn a_new_acknowledgement_prints_the_purge_warning() {
    let r = display::recorded(
        OutcomeKind::Done,
        &Ok(ack_answer()),
        &ProjectId("P".into()),
        true,
    );
    assert_eq!(r.code, 0);
    assert!(!r.error);
    assert_eq!(warnings(&r), 1);
}
#[test]
fn a_replayed_acknowledgement_still_prints_the_purge_warning() {
    let r = display::acknowledgement(
        &Recorded::Replayed {
            outcome: acknowledged_outcome(Answer::Inline(ack_answer())),
        },
        &ProjectId("P".into()),
        &bodies(Err(StoreError::Busy)),
    );
    assert_eq!(r.code, 0);
    assert_eq!(r.lines[0], ack_line());
    assert_eq!(warnings(&r), 1);
}
#[test]
fn a_replayed_acknowledgement_renders_as_a_new_one_does() {
    let project = ProjectId("P".into());
    let payloads = bodies(Err(StoreError::Busy));
    let outcome = acknowledged_outcome(Answer::Inline(ack_answer()));
    let new = display::acknowledgement(
        &Recorded::New {
            outcome: outcome.clone(),
            head: head(),
        },
        &project,
        &payloads,
    );
    let replayed = display::acknowledgement(&Recorded::Replayed { outcome }, &project, &payloads);
    assert_eq!(replayed.lines, new.lines);
    assert_eq!(replayed.code, new.code);
    assert_eq!(replayed.error, new.error);
}
#[test]
fn an_unreadable_done_acknowledgement_keeps_its_code_and_the_purge_warning() {
    let project = ProjectId("P".into());
    let cases = [
        (
            Err(answer::AnswerUnread::Gone(PayloadStatus::Purged {
                reason: "secret".into(),
            })),
            0,
        ),
        (Err(answer::AnswerUnread::Malformed), 0),
        (Err(answer::AnswerUnread::Read(StoreError::Busy)), 3),
        (Ok(json!({"unrelated": 1})), 0),
    ];
    for (answer, code) in cases {
        let r = display::recorded(OutcomeKind::Done, &answer, &project, true);
        assert_eq!(r.code, code, "{answer:?}");
        assert!(r.error, "{answer:?}");
        assert_eq!(warnings(&r), 1, "{answer:?}");
    }
}
#[test]
fn an_unreadable_refused_acknowledgement_prints_no_purge_warning() {
    let r = display::recorded(
        OutcomeKind::Refused,
        &Err(answer::AnswerUnread::Malformed),
        &ProjectId("P".into()),
        true,
    );
    assert_eq!(r.code, 1);
    assert_eq!(warnings(&r), 0);
}
#[test]
fn the_purge_warning_tells_the_owner_what_a_restore_cannot_undo() {
    let r = display::recorded(
        OutcomeKind::Done,
        &Ok(ack_answer()),
        &ProjectId("P".into()),
        true,
    );
    let t = text(&r);
    assert!(t.contains("bodies purged there may have reappeared"));
    assert!(t.contains("Every secret that was in such a body must be rotated"));
    assert!(t.contains("whether they came after the copy was taken or on a branch it replaced"));
    assert!(t.contains(
        "run baley purge <project> <hash>... --reason <text> for each project, from records kept outside the store"
    ));
    assert!(t.contains("Review first any hash with nothing left to release, because one such hash refuses the whole request"));
    assert!(t.contains("\"kept ... because another reference still requires it\""));
    assert!(!t.contains("history is complete"));
}
#[test]
fn a_head_that_moved_during_acknowledgement_keeps_code_three_and_no_purge_warning() {
    let r = display::acknowledgement_error(
        &AcknowledgeRestoreError::Store(StoreError::Stale(StaleInput::Head {
            seen: Some(head()),
            now: None,
        })),
        "P",
    );
    assert_eq!(r.code, 3);
    assert!(r.error);
    assert_eq!(warnings(&r), 0);
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
    let reads = crate::policy_step::Reads { global, head };
    command_plan::Settings {
        project_file: Some(command_plan::ProjectFile {
            path: CHECKOUT_FILE.into(),
            id,
        }),
        policy: crate::policy_step::build(&reads),
        pending: command_plan::pending_note(&reads),
    }
}
fn outside_settings(
    global: Result<Option<policy::SettingsFile>, policy::Unavailable>,
) -> command_plan::Settings {
    let reads = crate::policy_step::Reads { global, head: None };
    command_plan::Settings {
        project_file: None,
        policy: crate::policy_step::build(&reads),
        pending: command_plan::pending_note(&reads),
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
fn anchor_requests_checkout_admission_then_the_step_then_the_anchor_on_the_remote_once_it_is_checked()
 {
    assert_eq!(
        anchor_next(Some(&origin_settings()), Some(&Ok(())), None),
        Ok(vec![
            command_plan::Op::AdmitCheckout,
            command_plan::Op::Step,
            anchors(Some("origin"))
        ])
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
fn anchor_with_no_remote_set_requests_checkout_admission_the_step_and_an_anchor_with_none_and_no_check()
 {
    assert_eq!(
        anchor_next(Some(&unset_settings()), None, None),
        Ok(vec![
            command_plan::Op::AdmitCheckout,
            command_plan::Op::Step,
            anchors(None)
        ])
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
    let settings = outside_settings(Ok(None));
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
fn acknowledge_restore_checks_the_remote_then_requests_checkout_admission_the_step_and_the_acknowledgement()
 {
    assert_eq!(
        ack_next(Some(&origin_settings()), None),
        Ok(vec![command_plan::Op::CheckRemote("origin".into())])
    );
    assert_eq!(
        ack_next(Some(&origin_settings()), Some(&Ok(()))),
        Ok(vec![
            command_plan::Op::AdmitCheckout,
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
fn anchored_verify_stages() -> Vec<Result<Vec<command_plan::Op>, String>> {
    let origin = origin_settings();
    let unset = unset_settings();
    vec![
        verify_next(anchored(), None, None),
        verify_next(anchored(), Some(&origin), None),
        verify_next(anchored(), Some(&origin), Some(&Ok(()))),
        verify_next(anchored(), Some(&unset), None),
    ]
}
#[test]
fn anchored_verify_never_requests_the_policy_step_at_any_stage() {
    for stage in anchored_verify_stages() {
        assert!(!stage.unwrap().contains(&command_plan::Op::Step));
    }
}
#[test]
fn anchored_verify_never_requests_checkout_admission_at_any_stage() {
    for stage in anchored_verify_stages() {
        assert!(!stage.unwrap().contains(&command_plan::Op::AdmitCheckout));
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
    let outside = outside_settings(Ok(None));
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
fn a_checkout_file_without_an_id_is_a_fault_naming_it_and_no_discovered_project_in_doctor() {
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
        let judged = command_plan::doctor_settings(&managed(observed, Ok(None), head_copy("")));
        assert_eq!(judged.discovered, None, "{case}");
        assert_eq!(judged.faults.len(), 1, "{case}: {:?}", judged.faults);
        assert!(
            judged.faults[0].contains(CHECKOUT_FILE),
            "{case}: {:?}",
            judged.faults
        );
    }
}
#[test]
fn an_invalid_global_file_is_a_fault_with_the_remote_unknown_even_when_heads_copy_sets_one() {
    let settings = managed(
        valid_id(),
        global_file("escalate_on_failure = [\n"),
        head_copy("[git]\nremote = \"origin\"\n"),
    );
    let judged = command_plan::doctor_settings(&settings);
    assert_eq!(judged.discovered.as_deref(), Some(PROJECT_ID));
    assert_eq!(judged.remote, anchor_plan::RemoteState::Unknown);
    assert_eq!(judged.faults.len(), 1);
    assert!(
        judged.faults[0].contains(GLOBAL_FILE),
        "{:?}",
        judged.faults
    );
}
#[test]
fn an_unreadable_global_file_and_a_wrong_typed_heads_copy_are_each_a_fault() {
    let unreadable = policy::Unavailable {
        path: GLOBAL_FILE.into(),
        fault: policy::Fault::Unreadable {
            cause: "Permission denied (os error 13)".into(),
        },
    };
    let judged =
        command_plan::doctor_settings(&managed(valid_id(), Err(unreadable), head_copy("")));
    assert_eq!(judged.faults.len(), 1);
    assert!(
        judged.faults[0].contains("Permission denied"),
        "{:?}",
        judged.faults
    );
    let wrong = managed(
        valid_id(),
        Ok(None),
        head_copy("escalate_on_failure = \"yes\"\n"),
    );
    let judged = command_plan::doctor_settings(&wrong);
    assert_eq!(judged.remote, anchor_plan::RemoteState::Unknown);
    assert_eq!(judged.faults.len(), 1);
    assert!(
        judged.faults[0].contains(CHECKOUT_FILE),
        "{:?}",
        judged.faults
    );
}
#[test]
fn outside_a_checkout_an_invalid_global_file_is_a_fault_in_doctor() {
    let outside = outside_settings(global_file("escalate_on_failure = [\n"));
    let judged = command_plan::doctor_settings(&outside);
    assert_eq!(judged.discovered, None);
    assert_eq!(judged.faults.len(), 1);
    assert!(
        judged.faults[0].contains(GLOBAL_FILE),
        "{:?}",
        judged.faults
    );
}
#[test]
fn clean_settings_give_doctor_no_fault_and_the_remote_from_git_remote() {
    let judged = command_plan::doctor_settings(&origin_settings());
    assert_eq!(judged.faults, Vec::<String>::new());
    assert_eq!(judged.discovered.as_deref(), Some(PROJECT_ID));
    assert_eq!(judged.remote, named_remote());
    let unset = command_plan::doctor_settings(&unset_settings());
    assert_eq!(unset.remote, anchor_plan::RemoteState::NotSet);
}
#[test]
fn doctor_checks_a_discovered_project_with_unreadable_settings_locally_for_that_reason() {
    let plan = anchor_plan::doctor_checks(
        Some("P"),
        &anchor_plan::RemoteState::Unknown,
        &ledger(&["P", "Q"]),
    );
    assert_eq!(
        check_of(&plan, "P"),
        Some(&anchor_plan::CheckAgainst::Local(
            anchor_plan::LocalReason::SettingsUnreadable
        ))
    );
    assert_eq!(plan.validate, None);
}
#[test]
fn a_settings_fault_is_listed_with_its_file_sets_exit_one_and_keeps_every_project() {
    let mut h = health();
    let mut q = h.projects[0].clone();
    q.project = ProjectId("Q".into());
    h.projects.push(q);
    let faulted = command_plan::DoctorSettings {
        faults: vec![format!(
            "config-unavailable: {GLOBAL_FILE}:1:9: roles is an integer"
        )],
        ..no_faults()
    };
    let r = display::doctor(&h, &projects(), &no_reasons(), &faulted);
    assert_eq!(r.code, 1);
    assert!(text(&r).contains(&format!(
        "settings finding: config-unavailable: {GLOBAL_FILE}:1:9"
    )));
    assert!(text(&r).contains("project P (One)"));
    assert!(text(&r).contains("project Q (Two)"));
    assert_eq!(
        display::doctor(&h, &projects(), &no_reasons(), &no_faults()).code,
        0
    );
}
#[test]
fn doctor_labels_a_project_whose_settings_could_not_be_read() {
    let reasons = std::collections::BTreeMap::from([(
        ProjectId("P".into()),
        anchor_plan::LocalReason::SettingsUnreadable,
    )]);
    let r = text(&display::doctor(
        &health(),
        &projects(),
        &reasons,
        &no_faults(),
    ));
    assert!(r.contains("project P (One): local only, the settings could not be read"));
}
fn differs() -> crate::committed::Pending {
    crate::committed::Pending::Differs {
        path: CHECKOUT_FILE.into(),
    }
}
fn head_pending(
    text: &str,
    pending: crate::committed::Pending,
) -> Option<Result<crate::committed::Committed, policy::Unavailable>> {
    Some(Ok(crate::committed::Committed {
        layer: Some(checkout_file(text)),
        pending: Some(pending),
    }))
}
const IGNORED_GLOBAL: &str = "unknown = 1\n[git]\nremote = \"origin\"\n";
const IGNORED_HEAD: &str = "[host.cursor]\nescalate_on_failure = 3\n";
fn ignoring_settings() -> command_plan::Settings {
    managed(
        valid_id(),
        global_file(IGNORED_GLOBAL),
        head_pending(IGNORED_HEAD, differs()),
    )
}
#[test]
fn doctor_passes_the_policys_diagnostics_global_file_first_then_heads_copy() {
    let judged = command_plan::doctor_settings(&ignoring_settings());
    let lines: Vec<String> = judged.diagnostics.iter().map(ToString::to_string).collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[0].starts_with(GLOBAL_FILE) && lines[0].contains("unknown is not a setting"),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with(GLOBAL_FILE)
            && lines[1]
                .contains("git.remote is a project setting and was ignored in the global file"),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with(CHECKOUT_FILE)
            && lines[2].contains("host.cursor is not a host Baley knows"),
        "{lines:?}"
    );
    assert_eq!(judged.faults, Vec::<String>::new());
}
#[test]
fn doctor_passes_the_note_of_heads_copy_even_when_a_fault_stops_the_policy() {
    let judged = command_plan::doctor_settings(&ignoring_settings());
    assert_eq!(judged.pending, Some(differs()));
    let faulted = managed(
        valid_id(),
        Ok(None),
        head_pending("escalate_on_failure = \"yes\"\n", differs()),
    );
    let judged = command_plan::doctor_settings(&faulted);
    assert_eq!(judged.faults.len(), 1);
    assert_eq!(judged.diagnostics, vec![]);
    assert_eq!(judged.pending, Some(differs()));
}
#[test]
fn no_note_is_passed_when_heads_copy_was_not_read_or_failed() {
    let none = crate::policy_step::Reads {
        global: Ok(None),
        head: None,
    };
    assert_eq!(command_plan::pending_note(&none), None);
    let failed = crate::policy_step::Reads {
        global: Ok(None),
        head: Some(Err(policy::Unavailable {
            path: CHECKOUT_FILE.into(),
            fault: policy::Fault::Unreadable {
                cause: "git failed".into(),
            },
        })),
    };
    assert_eq!(command_plan::pending_note(&failed), None);
}
#[test]
fn doctor_prints_each_diagnostic_with_its_file_then_the_pending_note_before_any_project() {
    let judged = command_plan::doctor_settings(&ignoring_settings());
    let r = display::doctor(&health(), &projects(), &no_reasons(), &judged);
    let at = |needle: &str| {
        r.lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line holds {needle}: {:?}", r.lines))
    };
    let (unknown, scope, host) = (
        at("unknown is not a setting"),
        at("git.remote is a project setting"),
        at("host.cursor"),
    );
    let note = at(&format!(
        "{CHECKOUT_FILE} differs from HEAD's copy, so its changes apply once committed"
    ));
    assert!(at("database ") < unknown);
    assert!(unknown < scope && scope < host && host < note);
    assert!(note < at("project P (One)"));
}
#[test]
fn doctor_diagnostics_and_the_pending_note_alone_leave_the_exit_code_at_zero() {
    let judged = command_plan::doctor_settings(&ignoring_settings());
    assert_eq!(
        display::doctor(&health(), &projects(), &no_reasons(), &judged).code,
        0
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
