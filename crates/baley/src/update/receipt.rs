//! Plain update receipts over supplied results (design 0012 section 5).

use baley_core::policy::Unavailable;
use baley_store::{ClaimId, ClaimState, Recorded, StoreError};

use crate::ledger::display::{self, Render};

use super::claim::{CHECK_BUSY, Gate, NEEDS_RECONCILIATION, NOT_DUE};
use super::deliver::{Activated, Failure};
use super::events::CheckOutcome;
use super::installation::HomeRefusal;
use super::seed;
use super::version::Version;

/// The check's result before recording, including facts the receipt needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The source offered no higher version.
    Current {
        /// The version still reached through the stable path.
        active_version: Version,
        /// The version the source offered.
        offered_version: Version,
    },
    /// Delivery staged and activated a version.
    Activated(Activated),
    /// The check ended before activation succeeded.
    Failed {
        /// The version active before the failed step, when known.
        active_version: Option<Version>,
        /// The step, code, cause and any staged version.
        failure: Failure,
    },
}

/// What the owner is told after the attempt to record a claimed check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The offered version was not higher, and the check was recorded.
    Current {
        /// The version still reached through the stable path.
        active_version: Version,
        /// The version the source offered.
        offered_version: Version,
    },
    /// Activation succeeded and the check was recorded.
    Staged {
        /// The versions left by delivery.
        activated: Activated,
        /// The new binary's seed result, absent when no launch ran.
        seed: Option<seed::Outcome>,
    },
    /// A named step failed and the failure was recorded.
    Failed {
        /// The version active before the failed step, when known.
        active_version: Option<Version>,
        /// The step, code, cause and any staged version.
        failure: Failure,
    },
    /// The effects stand, but completion did not confirm the record.
    Incomplete {
        /// The newly active version after activation, otherwise the prior one.
        active_version: Option<Version>,
        /// A successful activation, absent when no activation succeeded.
        activated: Option<Activated>,
        /// The new binary's seed result, absent when no launch ran.
        seed: Option<seed::Outcome>,
        /// Why completion failed.
        cause: String,
    },
}

/// Chooses the receipt from effects and completion separately, so a record
/// failure cannot undo an activation in the owner's report.
pub fn outcome(
    check: Check,
    completion: Result<Recorded, StoreError>,
    seed: Option<seed::Outcome>,
) -> Outcome {
    if let Err(error) = completion {
        let (active_version, activated) = match check {
            Check::Current { active_version, .. } => (Some(active_version), None),
            Check::Activated(activated) => (Some(activated.active_version), Some(activated)),
            Check::Failed { active_version, .. } => (active_version, None),
        };
        return Outcome::Incomplete {
            active_version,
            seed: activated.as_ref().and(seed),
            activated,
            cause: error.to_string(),
        };
    }
    match check {
        Check::Current {
            active_version,
            offered_version,
        } => Outcome::Current {
            active_version,
            offered_version,
        },
        Check::Activated(activated) => Outcome::Staged { activated, seed },
        Check::Failed {
            active_version,
            failure,
        } => Outcome::Failed {
            active_version,
            failure,
        },
    }
}

/// Why an update stopped before claiming the installation.
#[derive(Debug)]
pub enum BeforeClaim {
    /// No development artifact source has been configured.
    SourceUnset,
    /// HOME cannot identify the installation.
    Home(HomeRefusal),
    /// The global settings could not be read or parsed.
    Settings(Unavailable),
}

/// Keeps early refusals in the command's usage exit class.
#[allow(
    dead_code,
    reason = "Plan 2 Task 7 wires the command to these receipts"
)]
pub(crate) fn before_claim(reason: &BeforeClaim) -> Render {
    Render::refusal(match reason {
        BeforeClaim::SourceUnset => "update-source-unset: updates.source is not set; set it with baley config set --global updates.source=https://...".into(),
        BeforeClaim::Home(error) => error.to_string(),
        BeforeClaim::Settings(error) => error.to_string(),
    })
}

/// Renders a stopped claim gate. A fetch permit has no receipt yet.
#[allow(
    dead_code,
    reason = "Plan 2 Task 7 wires the command to these receipts"
)]
pub(crate) fn at_claim(installation: &str, day: &str, gate: &Gate) -> Option<Render> {
    let text = match gate {
        Gate::Fetch(_) => return None,
        Gate::Refused { code } if code == NOT_DUE => format!(
            "{NOT_DUE}: an update check for {installation} was already claimed on {day}; a manual baley update may still run"
        ),
        Gate::Refused { code } => {
            format!("{code}: the update check for {installation} was refused")
        }
        Gate::Busy { holder, state } => busy(installation, holder, *state),
        Gate::InProgress(holder) => busy(installation, holder, ClaimState::Active),
        Gate::NeedsReconciliation { holder } => format!(
            "{NEEDS_RECONCILIATION}: update check {} for {installation} is interrupted; the next update check reconciles it",
            holder.request_id.0
        ),
        Gate::Replayed => format!("the update check for {installation} was already completed"),
        Gate::Failed(error) => {
            let mut refusal = Render::refusal("");
            refusal.lines = display::store_error(error, Some("user")).lines;
            return Some(refusal);
        }
    };
    Some(Render::refusal(text))
}

fn busy(installation: &str, holder: &ClaimId, state: ClaimState) -> String {
    let state_text = match state {
        ClaimState::Active => "active",
        ClaimState::Interrupted => "interrupted",
        ClaimState::AwaitingOwner => "awaiting the owner",
    };
    let fix = if state == ClaimState::AwaitingOwner {
        "; run baley update resolve"
    } else {
        ""
    };
    format!(
        "{CHECK_BUSY}: update check {} for {installation} is {state_text}{fix}",
        holder.request_id.0
    )
}

/// Renders a claimed check without reading the installation or the ledger.
#[allow(
    dead_code,
    reason = "Plan 2 Task 7 wires the command to these receipts"
)]
pub(crate) fn render(
    installation: &str,
    outcome: &Outcome,
    renewal_errors: &[StoreError],
) -> Render {
    let mut rendered = match outcome {
        Outcome::Current {
            active_version,
            offered_version,
        } => Render {
            lines: vec![
                format!("update outcome: {}", CheckOutcome::Current.as_str()),
                format!("active version at {installation}: {active_version}"),
                format!("offered version: {offered_version}"),
                "nothing staged".into(),
            ],
            code: 0,
            error: false,
        },
        Outcome::Staged { activated, seed } => Render {
            lines: staged_lines(installation, activated, seed.as_ref()),
            code: 0,
            error: false,
        },
        Outcome::Failed {
            active_version,
            failure,
        } => Render {
            lines: vec![
                format!(
                    "update failed at {}: {}: {}",
                    failure.step.as_str(),
                    failure.code.as_str(),
                    failure.cause
                ),
                unchanged_line(installation, *active_version),
            ],
            code: 1,
            error: true,
        },
        Outcome::Incomplete {
            active_version,
            activated,
            seed,
            cause,
        } => {
            let mut lines = match activated {
                Some(activated) => staged_lines(installation, activated, seed.as_ref()),
                None => vec![unchanged_line(installation, *active_version)],
            };
            lines.push(format!(
                "the record of this check is incomplete: {cause}; the next update check reconciles it"
            ));
            Render {
                lines,
                code: 1,
                error: true,
            }
        }
    };
    if let Some(first) = renewal_errors.first() {
        rendered.lines.push(format!(
            "{} lease renewals failed; first error: {first}",
            renewal_errors.len()
        ));
    }
    rendered
}

fn unchanged_line(installation: &str, active_version: Option<Version>) -> String {
    match active_version {
        Some(version) => {
            format!("{installation} still runs {version}; the active version is unchanged")
        }
        None => format!("{installation} is unchanged; no managed active version was known"),
    }
}

fn staged_lines(
    installation: &str,
    activated: &Activated,
    seed: Option<&seed::Outcome>,
) -> Vec<String> {
    let version = activated.active_version;
    let seed_line = match seed {
        Some(seed::Outcome::Recorded) => format!("the catalog seed of {version} was recorded"),
        Some(seed::Outcome::NotRecorded { cause }) => {
            format!("the catalog seed of {version} was not recorded: {cause}")
        }
        None => format!("the catalog seed of {version} was not recorded: no seed was launched"),
    };
    vec![
        format!("update outcome: {}", CheckOutcome::Staged.as_str()),
        format!("staged version: {}", activated.staged_version),
        format!("{version} is now the active version at {installation}"),
        format!(
            "new Claude Code sessions start {version}; running sessions keep the version they started with"
        ),
        seed_line,
        format!(
            "kept versions: {}",
            activated
                .kept_versions
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::deliver::Step;
    use crate::update::events::FailureCode;
    use baley_store::{Answer, CommandKind, Hash, Head, OutcomeKind, RequestId};
    use serde_json::json;

    const INSTALLATION: &str = "/home/o/.local/bin/baley";

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn activated() -> Activated {
        Activated {
            active_version: v("0.2.0"),
            staged_version: v("0.2.0"),
            kept_versions: vec![v("0.1.0"), v("0.2.0")],
        }
    }

    #[test]
    fn a_record_failure_after_activation_reported_as_the_old_version_is_caught() {
        let outcome = Outcome::Incomplete {
            active_version: Some(v("0.2.0")),
            activated: Some(activated()),
            seed: Some(seed::Outcome::Recorded),
            cause: StoreError::Unavailable("database or disk is full".into()).to_string(),
        };

        let receipt = render(INSTALLATION, &outcome, &[]);

        assert!(
            receipt.lines.iter().any(|line| {
                line == "0.2.0 is now the active version at /home/o/.local/bin/baley"
            }),
            "{:?}",
            receipt.lines
        );
        assert!(
            receipt.lines.iter().any(|line| {
                line.contains("the record of this check is incomplete")
                    && line.contains("database or disk is full")
                    && line.contains("the next update check reconciles it")
            }),
            "{:?}",
            receipt.lines
        );
        assert!(
            !receipt
                .lines
                .iter()
                .any(|line| { line.contains("active") && line.contains("0.1.0") })
        );
        assert_eq!(receipt.code, 1);
        assert!(receipt.error);
    }

    #[test]
    fn an_activation_receipt_missing_its_versions_effect_or_seed_is_caught() {
        for (seed, seed_line) in [
            (
                seed::Outcome::Recorded,
                "the catalog seed of 0.2.0 was recorded",
            ),
            (
                seed::Outcome::NotRecorded {
                    cause: "exit status 1: store busy, retry".into(),
                },
                "the catalog seed of 0.2.0 was not recorded: exit status 1: store busy, retry",
            ),
        ] {
            let outcome = Outcome::Staged {
                activated: activated(),
                seed: Some(seed),
            };

            let receipt = render(INSTALLATION, &outcome, &[]);

            assert_eq!(
                receipt.lines,
                [
                    "update outcome: staged",
                    "staged version: 0.2.0",
                    "0.2.0 is now the active version at /home/o/.local/bin/baley",
                    "new Claude Code sessions start 0.2.0; running sessions keep the version they started with",
                    seed_line,
                    "kept versions: 0.1.0, 0.2.0",
                ]
            );
            assert_eq!(receipt.code, 0);
            assert!(!receipt.error);
        }
    }

    fn verification_failure() -> Failure {
        Failure {
            step: Step::Verification,
            code: FailureCode::ChecksumMismatch,
            cause: "the download's SHA-256 differs from the manifest's".into(),
            staged_version: None,
        }
    }

    #[test]
    fn a_refusal_without_its_reason_or_fix_is_caught() {
        let source = before_claim(&BeforeClaim::SourceUnset);
        assert_eq!(source.code, 2);
        assert!(source.error);
        let text = source.lines.join("\n");
        assert!(text.contains("update-source-unset"));
        assert!(text.contains("updates.source"));
        assert!(text.contains("baley config set --global updates.source=https://..."));

        for (state, state_text, recovery) in [
            (ClaimState::AwaitingOwner, "awaiting the owner", true),
            (ClaimState::Active, "active", false),
        ] {
            let gate = Gate::Busy {
                holder: ClaimId {
                    kind: CommandKind("update.check".into()),
                    request_id: RequestId("00000000-0000-4000-8000-0000000000aa".into()),
                },
                state,
            };
            let busy = at_claim(INSTALLATION, "2026-10-08", &gate).unwrap();
            assert_eq!(busy.code, 2);
            assert!(busy.error);
            let text = busy.lines.join("\n");
            assert!(text.contains("update-check-busy"));
            assert!(text.contains(state_text));
            assert!(text.contains(INSTALLATION));
            assert_eq!(text.contains("baley update resolve"), recovery);
        }

        let not_due = at_claim(
            INSTALLATION,
            "2026-10-08",
            &Gate::Refused {
                code: "update-check-not-due".into(),
            },
        )
        .unwrap();
        assert_eq!(not_due.code, 2);
        assert!(not_due.error);
        let text = not_due.lines.join("\n");
        assert!(text.contains("update-check-not-due"));
        assert!(text.contains(INSTALLATION));
        assert!(text.contains("2026-10-08"));
        assert!(text.contains("a manual baley update may still run"));

        let failed = render(
            INSTALLATION,
            &Outcome::Failed {
                active_version: Some(v("0.1.0")),
                failure: verification_failure(),
            },
            &[],
        );
        assert_eq!(failed.code, 1);
        assert!(failed.error);
        assert_eq!(
            failed.lines,
            [
                "update failed at verification: update-checksum-mismatch: the download's SHA-256 differs from the manifest's",
                "/home/o/.local/bin/baley still runs 0.1.0; the active version is unchanged",
            ]
        );
    }

    fn completed(kind: OutcomeKind) -> Result<Recorded, StoreError> {
        Ok(Recorded::New {
            outcome: baley_store::Outcome {
                kind,
                answer: Answer::Inline(json!({})),
            },
            head: Head {
                seq: 14,
                hash: Hash([0; 32]),
            },
        })
    }

    #[test]
    fn a_completion_failure_after_activation_receipted_as_a_success_or_the_old_version_is_caught() {
        let completion_error = || Err(StoreError::Unavailable("database or disk is full".into()));
        let recorded = outcome(
            Check::Activated(activated()),
            completed(OutcomeKind::Done),
            Some(seed::Outcome::Recorded),
        );
        assert_eq!(
            recorded,
            Outcome::Staged {
                activated: Activated {
                    active_version: v("0.2.0"),
                    staged_version: v("0.2.0"),
                    kept_versions: vec![v("0.1.0"), v("0.2.0")],
                },
                seed: Some(seed::Outcome::Recorded),
            }
        );

        let incomplete = outcome(
            Check::Activated(activated()),
            completion_error(),
            Some(seed::Outcome::Recorded),
        );
        assert_eq!(
            incomplete,
            Outcome::Incomplete {
                active_version: Some(v("0.2.0")),
                activated: Some(Activated {
                    active_version: v("0.2.0"),
                    staged_version: v("0.2.0"),
                    kept_versions: vec![v("0.1.0"), v("0.2.0")],
                }),
                seed: Some(seed::Outcome::Recorded),
                cause: "store unavailable: database or disk is full".into(),
            }
        );

        let failed = Check::Failed {
            active_version: Some(v("0.1.0")),
            failure: verification_failure(),
        };
        let recorded_failure = outcome(failed.clone(), completed(OutcomeKind::Refused), None);
        assert_eq!(
            recorded_failure,
            Outcome::Failed {
                active_version: Some(v("0.1.0")),
                failure: Failure {
                    step: Step::Verification,
                    code: FailureCode::ChecksumMismatch,
                    cause: "the download's SHA-256 differs from the manifest's".into(),
                    staged_version: None,
                },
            }
        );

        let incomplete_failure = outcome(failed, completion_error(), None);
        assert_eq!(
            incomplete_failure,
            Outcome::Incomplete {
                active_version: Some(v("0.1.0")),
                activated: None,
                seed: None,
                cause: "store unavailable: database or disk is full".into(),
            }
        );
    }
}
