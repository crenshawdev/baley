//! Plain reports and exit classes for owner commands.
use super::answer::AnswerUnread;
use baley_core::{AcknowledgeRestoreError, AnchorOutcome, AnchorReport, Verification};
use baley_store::*;
use serde_json::Value;

/// The log warning threshold: 1,000 pages of 8 KiB.
pub(super) const LOG_WARNING_BYTES: u64 = 8_192_000;

#[derive(Debug)]
/// Plain output and its exit classification.
pub(crate) struct Render {
    /// Lines printed in order.
    pub(crate) lines: Vec<String>,
    /// The command's exit status.
    pub(crate) code: u8,
    /// Whether these lines describe a refusal or failure.
    pub(crate) error: bool,
}
impl Render {
    /// Builds one result line.
    pub(crate) fn line(text: impl Into<String>, code: u8) -> Self {
        Self {
            lines: vec![text.into()],
            code,
            error: false,
        }
    }
    /// Builds a command refusal.
    pub(crate) fn refusal(text: impl Into<String>) -> Self {
        Self {
            lines: vec![text.into()],
            code: 2,
            error: true,
        }
    }
}
/// Writes `render` and returns the exit code: refusal lines to `err` with the
/// `baley: ` prefix, the rest to `out`. A reader that closes early, as `head`
/// does, ends the output and keeps the command's code, since the command
/// already ran. Any other failed write exits 1 or the command's own failure.
pub(crate) fn emit(
    render: &Render,
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> u8 {
    let (stream, prefix): (&mut dyn std::io::Write, &str) = if render.error {
        (err, "baley: ")
    } else {
        (out, "")
    };
    let written = render
        .lines
        .iter()
        .try_for_each(|line| writeln!(stream, "{prefix}{line}"))
        .and_then(|()| stream.flush());
    match written {
        Err(error) if error.kind() != std::io::ErrorKind::BrokenPipe => render.code.max(1),
        _ => render.code,
    }
}
/// Prints a command's request id before it runs. A failed write never stops
/// the command; its report meets the same stream at the end.
pub(super) fn request_line(out: &mut impl std::io::Write, request_id: &str) {
    let _ = writeln!(out, "request {request_id}");
}
/// Classifies store errors independently of prior commits.
pub(super) fn exit_class(error: &StoreError) -> u8 {
    match error {
        StoreError::CleanupFailed { .. }
        | StoreError::Refused(Refusal::ExportUnverified { .. }) => 1,
        StoreError::Refused(_) | StoreError::Blocked(_) => 2,
        _ => 3,
    }
}
/// Renders a store failure with the owner's recovery text.
pub(crate) fn store_error(error: &StoreError, project: Option<&str>) -> Render {
    if let StoreError::Refused(Refusal::UnsafeHome(faults)) = error {
        let mut lines = vec![format!(
            "{UNSAFE_HOME}: the ledger's home is not safe to open"
        )];
        lines.extend(faults.iter().map(ToString::to_string));
        return Render {
            lines,
            code: exit_class(error),
            error: true,
        };
    }
    if let StoreError::Refused(Refusal::ExportUnverified { report, .. }) = error {
        let mut rendered = verify_report(report);
        rendered
            .lines
            .insert(0, "export did not verify; target removed".into());
        rendered.code = 1;
        return rendered;
    }
    let text = match error {
        StoreError::Busy => "the ledger is busy; run the command again".into(),
        StoreError::ReadOnly { needed_epoch } => format!(
            "the ledger is read-only for this build; a build at epoch {needed_epoch} is needed"
        ),
        StoreError::Refused(Refusal::UnknownProject(p)) => {
            format!("project {} is not in the ledger", p.0)
        }
        StoreError::Refused(Refusal::TargetExists(path)) => format!(
            "{} already exists; export creates a new directory",
            path.display()
        ),
        StoreError::Refused(Refusal::NothingToPurge(hash)) => format!(
            "project {} holds no reference to {} left to purge; nothing was purged",
            project.unwrap_or("unknown"),
            hash.to_hex()
        ),
        _ => error.to_string(),
    };
    Render {
        lines: vec![text],
        code: exit_class(error),
        error: true,
    }
}
/// Preserves uncertainty after an anchor claim or push.
pub(super) fn anchor_error(error: &StoreError, request: &str, remote: Option<&str>) -> Render {
    let reached = remote.map_or(String::new(), |remote| {
        format!(" and its tag may have reached {remote}")
    });
    Render {
        lines: vec![
            format!("anchor request {request} failed: {error}"),
            format!(
                "its claim may already be recorded{reached}; once the claim's lease has expired (60 s), running baley anchor again from this checkout reconciles it from the remote"
            ),
        ],
        code: exit_class(error),
        error: true,
    }
}
/// Describes the supplied remote observation.
pub(super) fn check_text(check: &AnchorCheck) -> String {
    match check {
        AnchorCheck::Remote(a) => format!("anchor at {} {}", a.seq, a.hash.to_hex()),
        AnchorCheck::RemoteAbsent => "no anchor yet".into(),
        AnchorCheck::RemoteUnreachable => "unreachable, checked locally only".into(),
        AnchorCheck::RemoteMalformed(tag) => {
            format!("latest tag {tag} is not an anchor, checked locally only")
        }
        AnchorCheck::LocalOnly => "local only".into(),
    }
}
fn head_text(head: Option<&Head>) -> String {
    head.map_or_else(
        || "empty".into(),
        |h| format!("{} {}", h.seq, h.hash.to_hex()),
    )
}
fn payload_text(fault: &PayloadFault) -> String {
    match fault {
        PayloadFault::Missing(h) => format!("missing payload {}", h.to_hex()),
        PayloadFault::Corrupt(h) => format!("corrupt payload {}", h.to_hex()),
    }
}
/// Renders chain, body and local-anchor findings.
pub(super) fn verify_report(report: &VerifyReport) -> Render {
    let mut lines = vec![format!(
        "chain head {}",
        head_text(report.chain.head.as_ref())
    )];
    lines.push(match &report.chain.anchor {
        AnchorVerdict::Matches => "matches the remote anchor".into(),
        AnchorVerdict::NoAnchor => "not compared with a remote anchor".into(),
        AnchorVerdict::Truncated { anchored, head } => {
            format!("truncated: remote anchor at {anchored}, chain ends at {head}")
        }
        AnchorVerdict::Rewritten {
            seq,
            anchored,
            found,
        } => format!(
            "rewritten at {seq}: anchored {}, found {}",
            anchored.to_hex(),
            found.to_hex()
        ),
        AnchorVerdict::Unchecked { anchored } => format!("unchecked: remote anchor at {anchored}"),
        AnchorVerdict::Acknowledged {
            anchored,
            restored,
            acknowledged_seq,
        } => format!(
            "acknowledged at {acknowledged_seq}: restore {} behind remote anchor {anchored}",
            head_text(restored.as_ref())
        ),
    });
    if let Some(broken) = &report.chain.first_break {
        lines.push(format!(
            "first break at {}: {}",
            broken.seq,
            break_text(&broken.kind)
        ));
    }
    if let Some(range) = &report.chain.unanchored {
        lines.push(format!(
            "unanchored sequences {} to {}",
            range.start(),
            range.end()
        ));
    }
    if let Some(at) = &report.chain.age_unanchored_since {
        lines.push(format!("work unanchored since {at}"));
    }
    let shown = match &report.chain.anchor {
        AnchorVerdict::Acknowledged {
            acknowledged_seq, ..
        } => Some(*acknowledged_seq),
        _ => None,
    };
    // The verdict line already names the acknowledgement it rests on.
    for restore in report
        .chain
        .acknowledged_restores
        .iter()
        .filter(|r| Some(r.seq) != shown)
    {
        lines.push(format!(
            "acknowledged restore at {}: {} behind {} {}",
            restore.seq,
            head_text(restore.restored.as_ref()),
            restore.anchor.seq,
            restore.anchor.hash.to_hex()
        ));
    }
    lines.push(format!(
        "bodies checked {}, tombstones {}",
        report.bodies_checked, report.tombstones_checked
    ));
    lines.extend(report.payloads.iter().map(payload_text));
    if let Some(row) = &report.stored_anchor {
        lines.push(format!(
            "local anchor {} {} as {}, confirmed at {}",
            row.anchor.seq,
            row.anchor.hash.to_hex(),
            row.tag,
            row.pushed_at
        ));
    }
    lines.push(format!(
        "local anchor row: {}",
        comparison_text(report.stored_anchor_comparison)
    ));
    let good = report.chain.is_intact()
        && report.payloads.is_empty()
        && !matches!(
            report.stored_anchor_comparison,
            StoredAnchorComparison::Conflict | StoredAnchorComparison::LocalAhead
        );
    Render {
        lines,
        code: u8::from(!good),
        error: false,
    }
}
/// Includes the fetch status and time beside verification.
pub(super) fn verification(report: &Verification, remote: Option<&str>) -> Render {
    let mut result = verify_report(&report.report);
    let prefix = remote.map_or(String::new(), |r| format!("remote {r}: "));
    result
        .lines
        .insert(0, format!("{prefix}{}", check_text(&report.status)));
    result
        .lines
        .insert(1, format!("checked at {}", report.checked_at));
    if matches!(
        report.status,
        AnchorCheck::RemoteUnreachable | AnchorCheck::RemoteMalformed(_)
    ) {
        result.code = 1;
    }
    result
}
/// Renders each differing view document.
pub(super) fn views(report: &ViewsReport) -> Render {
    let mut result = Render::line(
        format!("views checked at sequence {}", report.checked_seq),
        u8::from(!report.differing.is_empty()),
    );
    if report.differing.is_empty() {
        result.lines.push("no differences".into());
    }
    for (view, key) in &report.differing {
        result.lines.push(format!("{view} {}", key_text(key)));
    }
    result
}
/// Reports every project even when another project has failed.
pub(super) fn doctor(health: &Health, projects: &[(ProjectId, String)]) -> Render {
    let mut result = Render::line(format!("epoch {}", health.epoch), 0);
    if let Some(at) = &health.scrub_pending {
        result.lines.push(format!("scrub pending since {at}"));
        result.code = 1;
    }
    for row in &health.integrity {
        result.lines.push(format!("integrity: {row}"));
    }
    if health.integrity != ["ok"] {
        result.code = 1;
    }
    result.lines.push(format!(
        "database {} bytes, log {} bytes",
        health.database_bytes, health.log_bytes
    ));
    if health.log_bytes > LOG_WARNING_BYTES {
        result.lines.push(format!("warning: the write-ahead log is {} bytes, more than 8,192,000 (1,000 pages); find the reader that keeps it from checkpointing", health.log_bytes));
        result.code = 1;
    }
    for p in &health.projects {
        let name = projects
            .iter()
            .find(|(id, _)| id == &p.project)
            .map_or("", |(_, name)| name.as_str());
        result.lines.push(format!(
            "project {} ({name}): {}",
            p.project.0,
            check_text(&p.check)
        ));
        match &p.verify {
            Ok(report) => {
                let v = verify_report(report);
                result.lines.extend(v.lines);
                result.code |= v.code;
            }
            Err(error) => {
                result.lines.push(format!("verify failed: {error}"));
                result.code = 1;
            }
        }
        if let Some(row) = &p.remote_absent_local_row {
            result.lines.push(format!(
                "remote absent, local anchor {} {} as {}",
                row.anchor.seq,
                row.anchor.hash.to_hex(),
                row.tag
            ));
            result.code = 1;
        }
        let age = match &p.unanchored {
            UnanchoredAge::None => "none".into(),
            UnanchoredAge::Unchecked => {
                result.code = 1;
                "unchecked".into()
            }
            UnanchoredAge::Since { since, warning } => {
                if *warning {
                    result.code = 1;
                }
                format!(
                    "since {since}{}",
                    if *warning { ", more than a day" } else { "" }
                )
            }
        };
        result.lines.push(format!("unanchored age: {age}"));
        match &p.raw_views {
            Err(e) => {
                result.lines.push(format!("view versions: {e}"));
                result.code = 1;
            }
            Ok(raw) => {
                result.lines.push(format!(
                    "view set {}, binary {}",
                    version_text(raw.view_set.0),
                    raw.view_set.1
                ));
                if raw.view_set.0.is_some_and(|v| v > raw.view_set.1) {
                    result.code = 1;
                }
                for v in &raw.views {
                    result.lines.push(format!(
                        "view {}: live {}, binary {}",
                        v.view,
                        version_text(v.live_version),
                        v.binary_version
                    ));
                    if v.live_version.is_some_and(|live| live > v.binary_version) {
                        result.code = 1;
                    }
                }
                if let Some(b) = &raw.building {
                    result.lines.push(format!(
                        "building generation {}, applied {}, lag {}",
                        b.generation, b.applied_seq, b.lag
                    ));
                    result.code = 1;
                }
            }
        }
        match &p.views_check {
            Ok(v) => {
                let v = views(v);
                result.lines.extend(v.lines);
                result.code |= v.code;
            }
            Err(e) => {
                result.lines.push(format!("views check: {e}"));
                result.code = 1;
            }
        }
        match &p.claims {
            Ok(c) => {
                result.lines.push(format!(
                    "claims: {} active, {} interrupted, {} awaiting owner",
                    c.active, c.interrupted, c.awaiting_owner
                ));
                if c.interrupted > 0 || c.awaiting_owner > 0 {
                    result.code = 1;
                }
            }
            Err(e) => {
                result.lines.push(format!("claims: {e}"));
                result.code = 1;
            }
        }
    }
    result
}
/// Reports body removals, shared references and recovery.
pub(super) fn purge(report: &PurgeReport) -> Render {
    let mut result = Render {
        lines: Vec::new(),
        code: u8::from(!report.scrubbed),
        error: false,
    };
    for (_, seq) in &report.recorded {
        result.lines.push(format!("recorded at sequence {seq}"));
    }
    for hash in &report.purged {
        result.lines.push(format!("purged {}", hash.to_hex()));
    }
    for hash in &report.shared {
        result.lines.push(format!(
            "kept {} because another reference still requires it",
            hash.to_hex()
        ));
    }
    for path in &report.unreachable {
        result.lines.push(format!(
            "export already received a removed or shared body: {}",
            path.display()
        ));
    }
    result.lines.push(
        if report.scrubbed {
            "scrub complete"
        } else {
            "scrub incomplete: close any reader of the ledger and run baley scrub"
        }
        .into(),
    );
    result.lines.push("A secret that already reached a review provider, an export or any other system must still be rotated.".into());
    result
}
/// Reports the verified standalone export.
pub(super) fn export(project: &str, report: &ExportReport) -> Render {
    Render::line(
        format!(
            "exported project {project} to {}, head {}, verified",
            report.target.display(),
            head_text(report.head.as_ref())
        ),
        0,
    )
}
fn unread(kind: OutcomeKind, error: &AnswerUnread) -> Render {
    let code = if matches!(error, AnswerUnread::Read(_)) {
        3
    } else {
        u8::from(kind == OutcomeKind::Refused)
    };
    let why = match error {
        AnswerUnread::Gone(status) => status_text(status),
        AnswerUnread::Malformed => "malformed answer".into(),
        AnswerUnread::Read(error) => error.to_string(),
    };
    let mut result = Render::line(
        format!(
            "the outcome was recorded as {}; its answer could not be read: {why}",
            if kind == OutcomeKind::Done {
                "done"
            } else {
                "refused"
            }
        ),
        code,
    );
    result.error = true;
    result
}
/// The line for a refused anchor's answer. A failed push records its reason
/// under `failed` or at the top, and the claim decision records the code it
/// refused on under `refused`.
fn anchor_refusal_text(answer: &Value) -> Option<String> {
    let reason = answer
        .get("failed")
        .and_then(|v| v.get("reason"))
        .or_else(|| answer.get("reason"))
        .and_then(Value::as_str);
    if let Some(reason) = reason {
        return Some(format!("anchor failed: {reason}"));
    }
    Some(match answer.get("refused")?.as_str()? {
        "empty-chain" => "nothing to anchor: the project has no events".into(),
        "no-remote" => "the project sets no git.remote; set it in baley.toml and commit it, then baley anchor can push".into(),
        code => format!("anchor refused: {code}"),
    })
}
/// Renders a loaded answer while preserving its recorded outcome.
pub(super) fn recorded(
    kind: OutcomeKind,
    answer: &Result<Value, AnswerUnread>,
    project: &ProjectId,
    acknowledge: bool,
) -> Render {
    let value = match answer {
        Ok(value) => value,
        Err(e) => return unread(kind, e),
    };
    let text = if acknowledge {
        if kind == OutcomeKind::Done {
            value.get("acknowledged").and_then(|v| Some(format!("acknowledged: the restored chain is accepted behind remote anchor {} ({} {}); anchoring may resume", anchor_tag(project, v.get("seq")?.as_u64()?), v.get("seq")?.as_u64()?, v.get("head")?.as_str()?)))
        } else {
            value
                .get("reason")
                .and_then(Value::as_str)
                .map(|r| format!("not acknowledged: {r}"))
        }
    } else if kind == OutcomeKind::Done {
        value.get("anchored").and_then(|v| {
            Some(format!(
                "anchored sequence {} ({}) as {} on {}, confirmed at {}",
                v.get("seq")?.as_u64()?,
                v.get("head")?.as_str()?,
                v.get("tag")?.as_str()?,
                v.get("remote")?.as_str()?,
                v.get("observed_at")?.as_str()?
            ))
        })
    } else {
        anchor_refusal_text(value)
    };
    text.map_or_else(
        || unread(kind, &AnswerUnread::Malformed),
        |text| {
            let mut result = Render::line(text, u8::from(kind == OutcomeKind::Refused));
            result.error = kind == OutcomeKind::Refused;
            result
        },
    )
}
/// Renders every anchor outcome and its accompanying diagnostics.
pub(super) fn anchor(
    report: &AnchorReport,
    answer: Option<&Result<Value, AnswerUnread>>,
    project: &ProjectId,
    request: &str,
) -> Render {
    let render = |o: &Outcome| {
        recorded(
            o.kind,
            answer.expect("recorded outcome has loaded answer"),
            project,
            false,
        )
    };
    let mut result = match &report.outcome {
        AnchorOutcome::Recorded { outcome, .. } => render(outcome),
        AnchorOutcome::Refused { outcome, .. } => render(outcome),
        AnchorOutcome::LateReplay { outcome, pushed } => {
            let mut r = render(outcome);
            r.lines.push(if *pushed { "this run's push reported the tag landed; it may have landed after reconciliation found it absent" } else { "this run's push did not report the tag landed" }.into());
            r.code = r.code.max(1);
            r
        }
        AnchorOutcome::Replayed(o) => {
            let mut r = render(o);
            r.lines
                .insert(0, format!("request {request} was answered before:"));
            r
        }
        AnchorOutcome::InProgress(c) => Render::line(
            format!(
                "request {} already holds an open anchor claim since {}; nothing more was done",
                c.id.request_id.0, c.claimed_at
            ),
            1,
        ),
        AnchorOutcome::Blocked(b) => Render::line(
            format!(
                "anchor blocked by request {} ({})",
                b.claim.request_id.0,
                claim_state_text(b.state)
            ),
            1,
        ),
        AnchorOutcome::RemoteUnknown(c) => Render::line(
            format!(
                "an interrupted anchor claim {} could not be reconciled because the remote was unreachable; nothing was recorded; run anchor again when it is reachable",
                c.request_id.0
            ),
            1,
        ),
        AnchorOutcome::ReconcileLost(e) => {
            Render::line(format!("anchor reconciliation lost: {e}"), 1)
        }
    };
    for e in &report.renewal_errors {
        result
            .lines
            .push(format!("warning: lease renewal failed: {e}"));
    }
    result
        .lines
        .extend(report.payload_faults.iter().map(payload_text));
    result
}
/// Names acknowledgement refusals and their recovery.
pub(super) fn acknowledgement_error(error: &AcknowledgeRestoreError, project: &str) -> Render {
    match error {
        AcknowledgeRestoreError::NothingToAcknowledge(check) => Render::refusal(format!("nothing to acknowledge: {}",check_text(check))),
        AcknowledgeRestoreError::Store(StoreError::Blocked(b)) => Render::refusal(format!("an anchor claim {} is {}; run baley anchor first from this checkout to reconcile it", b.claim.request_id.0,claim_state_text(b.state))),
        AcknowledgeRestoreError::Store(StoreError::Stale(StaleInput::Head { .. })) => Render { lines: vec!["the chain moved after it was verified; nothing was recorded, run the command again".into()],code:3,error:true },
        AcknowledgeRestoreError::Store(e) => store_error(e,Some(project)),
    }
}

/// Owner wording for the local anchor row against the remote anchor.
pub(super) fn comparison_text(comparison: StoredAnchorComparison) -> &'static str {
    match comparison {
        StoredAnchorComparison::NotCompared => "not compared with a remote anchor",
        StoredAnchorComparison::Matches => "matches the remote anchor",
        StoredAnchorComparison::MissingLocal => "missing, although the remote holds an anchor",
        StoredAnchorComparison::LocalBehind => "older than the remote anchor",
        StoredAnchorComparison::LocalAhead => "newer than the remote anchor",
        StoredAnchorComparison::Conflict => {
            "names the remote anchor's sequence with another hash or tag"
        }
    }
}
/// A stored version, or none when nothing is stamped yet.
pub(super) fn version_text(version: Option<u32>) -> String {
    version.map_or_else(|| "none".into(), |v| v.to_string())
}
/// Owner wording for an open claim's state.
pub(super) fn claim_state_text(state: ClaimState) -> &'static str {
    match state {
        ClaimState::Active => "active",
        ClaimState::Interrupted => "interrupted",
        ClaimState::AwaitingOwner => "awaiting the owner",
    }
}
/// Owner wording for what is wrong at a chain break.
pub(super) fn break_text(kind: &BreakKind) -> String {
    let hex = |h: &Option<Hash>| h.as_ref().map_or_else(|| "none".into(), |h| h.to_hex());
    match kind {
        BreakKind::Sequence { found } => format!("the row there carries sequence {found}"),
        BreakKind::Project { expected } => {
            format!("the event names a project other than {}", expected.0)
        }
        BreakKind::PrevHash { expected, found } => format!(
            "previous hash {} where {} was expected",
            hex(found),
            hex(expected)
        ),
        BreakKind::Hash { expected, found } => format!(
            "hash {} where {} was expected",
            found.to_hex(),
            expected.to_hex()
        ),
        BreakKind::Canonical(error) => format!("no canonical form: {error}"),
    }
}
/// Owner wording for a body that is no longer present.
pub(super) fn status_text(status: &PayloadStatus) -> String {
    match status {
        PayloadStatus::Present { bytes } => format!("present, {bytes} bytes"),
        PayloadStatus::Reduced { .. } => "reduced to an excerpt".into(),
        PayloadStatus::Purged { reason } => format!("purged: {reason}"),
    }
}
/// A document key as its field values, in declared order.
pub(super) fn key_text(key: &DocKey) -> String {
    key.0
        .iter()
        .map(|v| match v {
            KeyValue::Text(t) => format!("{t:?}"),
            KeyValue::Integer(i) => i.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
