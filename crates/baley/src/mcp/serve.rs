//! One session's start and end.
//!
//! Start gathers the project and the store and serves the handler over stdio.
//! The one opened ledger goes to the handler for every project call and stays
//! here for the exit checkpoint.
//! End is driven by [`super::lifecycle`]: input ending, SIGINT or SIGTERM
//! closes admission, accepted work gets up to ten seconds, and exactly one
//! checkpoint attempt follows. Nothing here runs a checkpoint or a timer while
//! the connection is open. The orchestration is gathering and has no unit test.
//! The list of end signals and the words for stderr have their own tests.

use std::future::poll_fn;
use std::sync::Arc;
use std::task::Poll;
use std::time::Instant;

use baley_store_sqlite::{ExitCheckpoint, SkipReason, StartupHealth};
use rmcp::ServiceExt;
use rmcp::service::ServerInitializeError;
#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::task::{JoinError, JoinHandle};

use super::context::{Observation, judge};
use super::handler::{SessionHandler, SessionLedger};
use super::lifecycle::{Action, Event, State, step};
use super::transport::{InputEnd, StdioTransport};
use super::worker::Worker;
use crate::folders::{Environment, Folders, Platform};
use crate::ledger::{clock::SystemClock, open};
use crate::store::writer::SERVER_DRAIN_BOUND;

/// The line for the one checkpoint attempt. `None` means the ledger was never
/// opened, so there was nothing to attempt. No line says the log was
/// shortened: a passive checkpoint only copies frames into the database file.
pub fn checkpoint_line(outcome: Option<&ExitCheckpoint>) -> String {
    match outcome {
        None => "baley: exit checkpoint skipped, the ledger was not opened".to_owned(),
        Some(ExitCheckpoint::Skipped(SkipReason::Fenced)) => {
            "baley: exit checkpoint skipped, the ledger failed its startup check".to_owned()
        }
        Some(ExitCheckpoint::Skipped(SkipReason::EpochNotThisBinary)) => {
            "baley: exit checkpoint skipped, the ledger belongs to another binary's epoch and is read-only here"
                .to_owned()
        }
        Some(ExitCheckpoint::Complete) => {
            "baley: exit checkpoint complete, every logged change is in the database file"
                .to_owned()
        }
        Some(ExitCheckpoint::Incomplete) => {
            "baley: exit checkpoint incomplete, some logged changes stay in the log until SQLite folds them in"
                .to_owned()
        }
        Some(ExitCheckpoint::Unavailable) => {
            "baley: exit checkpoint unavailable, the database was in use, so SQLite will fold the log in later"
                .to_owned()
        }
        Some(ExitCheckpoint::Error(text)) => format!("baley: exit checkpoint error: {text}"),
    }
}

/// The line for work still open when the drain bound passed.
pub fn abandoned_line() -> String {
    format!(
        "baley: work still open after {} seconds was left to SQLite's rollback",
        SERVER_DRAIN_BOUND.as_secs()
    )
}

/// Opens the per-user ledger for the server and returns it with the config
/// folder found on the way. A failure leaves the server running without one:
/// a project call then answers `failed` `ledger-unavailable`, the calls that
/// need no project keep answering, and Claude Code does not restart a stdio
/// server that exits.
fn open_store() -> Option<SessionLedger> {
    let folders = match Folders::resolve(Platform::current(), &Environment::read()) {
        Ok(folders) => folders,
        Err(refusal) => {
            eprintln!("baley: the ledger home cannot be found: {refusal}");
            return None;
        }
    };
    let store = match open::store(&folders.home, &SystemClock::now(), open::server_options()) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("baley: the ledger cannot be opened: {error}");
            return None;
        }
    };
    if let StartupHealth::Unhealthy { report } = store.startup_health() {
        eprintln!("baley: the ledger failed its startup check, so writes are refused: {report}");
    }
    Some(SessionLedger {
        store,
        config: folders.config,
    })
}

/// The signals that end a session the way input ending does. Claude Code
/// stops a stdio server with SIGINT when its session ends. Any other signal
/// keeps its default action.
#[cfg(unix)]
const END_SIGNALS: [SignalKind; 2] = [SignalKind::interrupt(), SignalKind::terminate()];

#[cfg(unix)]
fn listen_for_end() -> std::io::Result<Vec<Signal>> {
    END_SIGNALS.into_iter().map(signal).collect()
}

/// Resolves when any end signal arrives.
#[cfg(unix)]
async fn signalled(signals: &mut [Signal]) {
    poll_fn(|context| {
        if signals
            .iter_mut()
            .any(|signal| signal.poll_recv(context).is_ready())
        {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

/// Whether the service task did well, judged from what is already known. A
/// service that failed to start drops the transport, which also wakes the input
/// watch, so the failure can arrive second and must still be read. A task that
/// is still running is never awaited: it can sit in stdin's blocking read, and
/// the drain bound must not wait on it.
async fn service_went_well(
    read: Option<Result<bool, JoinError>>,
    service: JoinHandle<bool>,
) -> bool {
    let result = match read {
        Some(result) => Some(result),
        None if service.is_finished() => Some(service.await),
        None => None,
    };
    result.is_none_or(|result| result.unwrap_or(false))
}

/// Serves one session on stdin and stdout until input ends or SIGINT or SIGTERM
/// arrives, then drains and makes the one checkpoint attempt. Returns whether
/// the run ended cleanly: the service started, input did not fail and the
/// drain finished inside its bound. The checkpoint's outcome does not change
/// that, since the attempt is best effort.
pub async fn run() -> bool {
    let mut signals = match listen_for_end() {
        Ok(signals) => signals,
        Err(error) => {
            eprintln!("baley: cannot listen for SIGINT or SIGTERM: {error}");
            return false;
        }
    };
    let session = Arc::new(judge(Observation::gather()));
    for note in &session.notes {
        eprintln!("baley: {note}");
    }
    let ledger = open_store().map(Arc::new);
    let worker = match Worker::start() {
        Ok(worker) => Arc::new(worker),
        Err(error) => {
            eprintln!("baley: cannot start the decision worker: {error}");
            return false;
        }
    };
    let handler = SessionHandler::new(session, Arc::clone(&worker), ledger.clone());
    let (transport, mut control) = StdioTransport::new(tokio::io::stdin(), tokio::io::stdout());
    let mut service = tokio::spawn(async move {
        match handler.serve(transport).await {
            Ok(service) => service.waiting().await.is_ok(),
            Err(ServerInitializeError::ConnectionClosed(_) | ServerInitializeError::Cancelled) => {
                true
            }
            Err(error) => {
                eprintln!("baley: failed to start the MCP server: {error}");
                false
            }
        }
    });

    let mut clean = true;
    let mut read = None;
    let first = tokio::select! {
        _ = control.input_ended() => Event::InputEnded,
        _ = signalled(&mut signals) => Event::Terminate,
        finished = &mut service => {
            read = Some(finished);
            Event::InputEnded
        }
    };
    clean &= service_went_well(read, service).await;
    clean &= control.ended() != Some(InputEnd::Failed);

    let began = Instant::now();
    let mut drained = worker.drained();
    let (mut state, mut actions) = step(State::Serving, first);
    loop {
        for action in std::mem::take(&mut actions) {
            match action {
                Action::StopAdmission => {
                    control.close_admission();
                    worker.close();
                }
                Action::Wait => {}
                Action::AbandonWaiting => worker.abandon(),
                Action::AttemptCheckpoint { abandoned } => {
                    if abandoned {
                        clean = false;
                        eprintln!("{}", abandoned_line());
                    }
                    let outcome = ledger
                        .as_ref()
                        .map(|ledger| ledger.store.checkpoint_at_exit());
                    eprintln!("{}", checkpoint_line(outcome.as_ref()));
                }
                Action::Exit => return clean,
            }
        }
        let remaining = SERVER_DRAIN_BOUND.saturating_sub(began.elapsed());
        let event = tokio::select! {
            _ = drained.wait_for(|drained| *drained) => Event::QueueDrained,
            _ = tokio::time::sleep(remaining) => Event::Elapsed(began.elapsed()),
        };
        (state, actions) = step(state, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn every_outcome() -> Vec<Option<ExitCheckpoint>> {
        vec![
            None,
            Some(ExitCheckpoint::Skipped(SkipReason::Fenced)),
            Some(ExitCheckpoint::Skipped(SkipReason::EpochNotThisBinary)),
            Some(ExitCheckpoint::Complete),
            Some(ExitCheckpoint::Incomplete),
            Some(ExitCheckpoint::Unavailable),
            Some(ExitCheckpoint::Error("disk full".into())),
        ]
    }

    #[test]
    fn each_outcome_line_names_its_own_outcome() {
        let named = [
            (None, "skipped"),
            (Some(ExitCheckpoint::Skipped(SkipReason::Fenced)), "skipped"),
            (
                Some(ExitCheckpoint::Skipped(SkipReason::EpochNotThisBinary)),
                "skipped",
            ),
            (Some(ExitCheckpoint::Complete), "complete"),
            (Some(ExitCheckpoint::Incomplete), "incomplete"),
            (Some(ExitCheckpoint::Unavailable), "unavailable"),
            (Some(ExitCheckpoint::Error("disk full".into())), "error"),
        ];
        for (outcome, word) in named {
            let line = checkpoint_line(outcome.as_ref());
            assert!(
                line.starts_with("baley: exit checkpoint ") && line.contains(word),
                "{outcome:?}: {line}"
            );
        }
        let error = checkpoint_line(Some(&ExitCheckpoint::Error("disk full".into())));
        assert!(error.contains("disk full"), "{error}");
    }

    #[test]
    fn no_line_says_the_log_was_truncated_emptied_or_reset() {
        for outcome in every_outcome() {
            let line = checkpoint_line(outcome.as_ref()).to_lowercase();
            for word in ["truncat", "empti", "reset", "cleared"] {
                assert!(!line.contains(word), "{outcome:?}: {line}");
            }
        }
        let line = abandoned_line().to_lowercase();
        assert!(
            !line.contains("truncat") && !line.contains("reset"),
            "{line}"
        );
    }

    #[test]
    fn a_line_for_an_attempt_that_did_not_finish_never_says_it_completed() {
        for outcome in [
            ExitCheckpoint::Incomplete,
            ExitCheckpoint::Unavailable,
            ExitCheckpoint::Error("disk full".into()),
            ExitCheckpoint::Skipped(SkipReason::Fenced),
            ExitCheckpoint::Skipped(SkipReason::EpochNotThisBinary),
        ] {
            let line = checkpoint_line(Some(&outcome));
            assert!(!line.contains("completed"), "{outcome:?}: {line}");
            let stripped = line.replace("incomplete", "");
            assert!(!stripped.contains("complete"), "{outcome:?}: {line}");
        }
        assert!(checkpoint_line(None).find("complete").is_none());
    }

    async fn finished(task: JoinHandle<bool>) -> JoinHandle<bool> {
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        task
    }

    #[tokio::test]
    async fn a_service_that_already_failed_is_read_even_when_input_ending_won() {
        let failed = finished(tokio::spawn(async { false })).await;
        assert!(!service_went_well(None, failed).await);
    }

    #[tokio::test]
    async fn a_service_that_already_stopped_cleanly_is_clean() {
        let stopped = finished(tokio::spawn(async { true })).await;
        assert!(service_went_well(None, stopped).await);
    }

    #[tokio::test]
    async fn a_service_that_panicked_is_not_clean() {
        let panicked = finished(tokio::spawn(async { panic!("service task") })).await;
        assert!(!service_went_well(None, panicked).await);
    }

    #[tokio::test]
    async fn a_result_the_select_already_read_decides_without_the_task() {
        let task = tokio::spawn(std::future::pending::<bool>());
        assert!(!service_went_well(Some(Ok(false)), task).await);
        let task = tokio::spawn(std::future::pending::<bool>());
        assert!(service_went_well(Some(Ok(true)), task).await);
    }

    #[tokio::test]
    async fn a_service_still_running_is_not_waited_for() {
        // It stands for a service blocked in stdin's read. One poll must finish.
        let running = tokio::spawn(std::future::pending::<bool>());
        let judged = std::pin::pin!(service_went_well(None, running));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match judged.poll(&mut context) {
            std::task::Poll::Ready(went_well) => assert!(went_well),
            std::task::Poll::Pending => panic!("judging waited for a running service"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn sigint_is_left_to_its_default_action_and_kills_the_server_before_its_exit_checkpoint() {
        // Claude Code 2.1.294 stops a stdio server with SIGINT at /exit.
        assert!(END_SIGNALS.contains(&SignalKind::interrupt()));
        assert!(END_SIGNALS.contains(&SignalKind::terminate()));
    }

    #[test]
    fn the_abandoned_drain_line_names_the_ten_second_bound() {
        assert_eq!(SERVER_DRAIN_BOUND, Duration::from_secs(10));
        assert!(
            abandoned_line().contains("10 seconds"),
            "{}",
            abandoned_line()
        );
    }
}
