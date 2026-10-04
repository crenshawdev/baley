//! The thread that runs accepted decisions, one at a time, in queue order.
//!
//! Baley owns this worker. Decisions are synchronous code, so they run here and
//! never on rmcp's per-request tasks, which a client's cancel can drop in the
//! middle of a write. A caller that stops waiting loses only its answer: the
//! decision still runs to its end, so a write reaches its transaction
//! boundary. The worker holds no timer, starts no checkpoint and touches no
//! store. The shutdown sequence in [`super::lifecycle`] drives it through
//! [`Worker::close`], [`Worker::drained`] and [`Worker::abandon`].

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use rmcp::ErrorData;
use rmcp::model::CallToolResult;
use tokio::sync::{oneshot, watch};

use super::admission::{Admission, admit};
use super::client::Selection;
use super::gate::Admitted;
use super::queue::{Placed, Queue};

/// What a caller gets back for an accepted call: the decision's result, or a
/// JSON-RPC internal error when the decision panicked.
pub type Answer = Result<CallToolResult, ErrorData>;

type Run = Box<dyn FnOnce(Admitted) -> CallToolResult + Send>;

/// One accepted decision and where its answer goes.
struct Job {
    admitted: Admitted,
    run: Run,
    reply: oneshot::Sender<Answer>,
}

struct State {
    queue: Queue<Job>,
    // The job the queue promoted that the worker has not picked up yet.
    start: Option<Job>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    drained: watch::Sender<bool>,
}

impl State {
    /// Closes the queue and takes every decision that has not begun: the
    /// waiting ones, and the one the queue promoted that the worker has not
    /// picked up yet, whose slot is freed so the queue can drain.
    fn abandon(&mut self) -> (Vec<Job>, Option<Job>) {
        let waiting = self.queue.abandon();
        let unstarted = self.start.take();
        if unstarted.is_some() {
            self.queue.finish();
        }
        (waiting, unstarted)
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What submitting a call came to.
#[derive(Debug)]
pub enum Submission {
    /// Answer now, as a successful tool result. Nothing was queued.
    Answer(CallToolResult),
    /// Fail the request as a protocol error. Nothing was queued.
    ProtocolError(ErrorData),
    /// The call was accepted. Its answer arrives on this receiver. The sender
    /// is dropped without an answer only when the call was abandoned unstarted.
    Accepted(oneshot::Receiver<Answer>),
}

/// The worker's handle. Dropping it closes admission.
pub struct Worker {
    shared: Arc<Shared>,
}

impl Worker {
    /// Starts the worker thread.
    pub fn start() -> std::io::Result<Self> {
        let (drained, _) = watch::channel(true);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: Queue::new(),
                start: None,
            }),
            wake: Condvar::new(),
            drained,
        });
        let thread = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("baley-decisions".into())
            .spawn(move || work(&thread))?;
        Ok(Self { shared })
    }

    /// Admits one call and, when accepted, queues `run` to decide it. `run`
    /// gets what the gate admitted. `raw_bytes` is the call's whole frame size.
    pub fn submit(
        &self,
        tool: &str,
        selection: &Selection,
        spelling: Option<&str>,
        raw_bytes: usize,
        run: impl FnOnce(Admitted) -> CallToolResult + Send + 'static,
    ) -> Submission {
        let (reply, receiver) = oneshot::channel();
        let mut state = self.shared.lock();
        let admission = admit(
            tool,
            selection,
            spelling,
            raw_bytes,
            &mut state.queue,
            |admitted| Job {
                admitted,
                run: Box::new(run),
                reply,
            },
        );
        match admission {
            Admission::Answer(result) => Submission::Answer(result),
            Admission::ProtocolError(error) => Submission::ProtocolError(error),
            Admission::Accepted(placed) => {
                if let Placed::Started(job) = placed {
                    state.start = Some(job);
                    self.shared.wake.notify_one();
                }
                self.shared.drained.send_replace(false);
                Submission::Accepted(receiver)
            }
        }
    }

    /// Stops admitting. Accepted calls still run.
    pub fn close(&self) {
        let mut state = self.shared.lock();
        state.queue.close();
        self.shared.drained.send_replace(state.queue.is_drained());
        self.shared.wake.notify_one();
    }

    /// Closes, and drops every waiting decision unstarted so the worker starts
    /// nothing more. A decision already running is not interrupted: it reaches
    /// its end, or is cut off when the process exits and SQLite rolls an open
    /// transaction back.
    pub fn abandon(&self) {
        let (waiting, unstarted) = {
            let mut state = self.shared.lock();
            let (waiting, unstarted) = state.abandon();
            self.shared.drained.send_replace(state.queue.is_drained());
            self.shared.wake.notify_one();
            (waiting, unstarted)
        };
        drop((waiting, unstarted));
    }

    /// Whether nothing is running and nothing waits. It turns true only after
    /// the last decision's answer was sent, so an orchestrator that awaits it
    /// under a deadline does not outrun an answer.
    pub fn drained(&self) -> watch::Receiver<bool> {
        self.shared.drained.subscribe()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.close();
    }
}

fn work(shared: &Shared) {
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if let Some(job) = state.start.take() {
                    break job;
                }
                if state.queue.is_closed() && state.queue.is_drained() {
                    return;
                }
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let Job {
            admitted,
            run,
            reply,
        } = job;
        let answer = catch_unwind(AssertUnwindSafe(move || run(admitted))).map_err(|_| {
            // A panic is a defect, not a store failure, so it is not a `failed` answer.
            ErrorData::internal_error("the decision failed unexpectedly", None)
        });
        // Free the slot before the answer goes out, so a caller that submits
        // again the moment it is answered finds the room back.
        {
            let mut state = shared.lock();
            state.start = state.queue.finish();
        }
        // A caller that stopped waiting dropped its receiver. The decision has
        // run, so the answer is simply not wanted.
        let _ = reply.send(answer);
        let state = shared.lock();
        shared.drained.send_replace(state.queue.is_drained());
    }
}

#[cfg(test)]
mod tests {
    use super::super::client::Reported;
    use super::*;
    use baley_core::policy::Host;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    fn supported() -> Selection {
        Selection::Supported {
            host: Host::ClaudeCode,
            client_version: "2.1.287".into(),
        }
    }

    fn result(label: u32) -> CallToolResult {
        CallToolResult::structured(json!({"decision": label}))
    }

    fn label_of(answer: Answer) -> u32 {
        let value = answer.expect("a decision answer").structured_content;
        u32::try_from(value.expect("structured")["decision"].as_u64().unwrap()).unwrap()
    }

    /// A decision that says it started, then waits for the test to release it.
    struct Held {
        started: mpsc::Receiver<u32>,
        release: mpsc::Sender<()>,
    }

    fn held(
        label: u32,
        log: &Arc<Mutex<Vec<String>>>,
    ) -> (
        impl FnOnce(Admitted) -> CallToolResult + Send + 'static,
        Held,
    ) {
        let (started_tx, started) = mpsc::channel();
        let (run, release) = held_on(label, log, started_tx);
        (run, Held { started, release })
    }

    /// Like [`held`], reporting its start on a channel the test shares.
    fn held_on(
        label: u32,
        log: &Arc<Mutex<Vec<String>>>,
        started_tx: mpsc::Sender<u32>,
    ) -> (
        impl FnOnce(Admitted) -> CallToolResult + Send + 'static,
        mpsc::Sender<()>,
    ) {
        let (release, release_rx) = mpsc::channel::<()>();
        let log = Arc::clone(log);
        let run = move |_: Admitted| {
            log.lock().unwrap().push(format!("start {label}"));
            started_tx.send(label).unwrap();
            release_rx.recv().unwrap();
            log.lock().unwrap().push(format!("end {label}"));
            result(label)
        };
        (run, release)
    }

    /// The next value, or a failure after a bound, so a worker that stopped
    /// making progress fails the test instead of hanging it.
    fn next<T>(channel: &mpsc::Receiver<T>) -> T {
        channel
            .recv_timeout(std::time::Duration::from_secs(15))
            .expect("the worker made no progress")
    }

    fn quick(label: u32) -> impl FnOnce(Admitted) -> CallToolResult + Send + 'static {
        move |_| result(label)
    }

    fn put(
        worker: &Worker,
        run: impl FnOnce(Admitted) -> CallToolResult + Send + 'static,
    ) -> Submission {
        worker.submit("baley_version", &supported(), None, 10, run)
    }

    fn receiver(submission: Submission) -> oneshot::Receiver<Answer> {
        match submission {
            Submission::Accepted(receiver) => receiver,
            other => panic!("expected the call to be accepted, got {other:?}"),
        }
    }

    fn log() -> Arc<Mutex<Vec<String>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    #[test]
    fn a_second_decision_starts_while_the_first_is_held_open() {
        let log = log();
        let worker = Worker::start().unwrap();
        let (a, hold_a) = held(1, &log);
        let first = receiver(put(&worker, a));
        assert_eq!(next(&hold_a.started), 1);
        let (b, hold_b) = held(2, &log);
        let second = receiver(put(&worker, b));
        hold_a.release.send(()).unwrap();
        assert_eq!(next(&hold_b.started), 2);
        hold_b.release.send(()).unwrap();
        assert_eq!(label_of(first.blocking_recv().unwrap()), 1);
        assert_eq!(label_of(second.blocking_recv().unwrap()), 2);
        assert_eq!(
            *log.lock().unwrap(),
            ["start 1", "end 1", "start 2", "end 2"]
        );
    }

    #[test]
    fn decisions_start_out_of_submission_order() {
        let log = log();
        let worker = Worker::start().unwrap();
        let (started_tx, started) = mpsc::channel();
        let mut releases = Vec::new();
        let mut receivers = Vec::new();
        for label in 0..=4 {
            let (run, release) = held_on(label, &log, started_tx.clone());
            receivers.push(receiver(put(&worker, run)));
            releases.push(release);
            if label == 0 {
                assert_eq!(next(&started), 0);
            }
        }
        releases[0].send(()).unwrap();
        for expected in 1..=4 {
            assert_eq!(next(&started), expected);
            releases[expected as usize].send(()).unwrap();
        }
        for (label, receiver) in receivers.into_iter().enumerate() {
            assert_eq!(label_of(receiver.blocking_recv().unwrap()) as usize, label);
        }
        assert_eq!(
            *log.lock().unwrap(),
            [
                "start 0", "end 0", "start 1", "end 1", "start 2", "end 2", "start 3", "end 3",
                "start 4", "end 4"
            ]
        );
    }

    #[test]
    fn the_five_slot_capacity_is_not_back_after_a_decision_panics() {
        let log = log();
        let worker = Worker::start().unwrap();
        let panicked = receiver(put(&worker, |_| panic!("a defect in a decision")));
        let error = panicked
            .blocking_recv()
            .unwrap()
            .expect_err("an internal error");
        assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        let (a, hold_a) = held(0, &log);
        let mut receivers = vec![receiver(put(&worker, a))];
        for label in 1..5 {
            receivers.push(receiver(put(&worker, quick(label))));
        }
        let Submission::Answer(overloaded) = put(&worker, quick(9)) else {
            panic!("a sixth call is refused");
        };
        assert_eq!(
            overloaded.structured_content.unwrap()["code"],
            "server-overloaded"
        );
        hold_a.release.send(()).unwrap();
        for (label, receiver) in receivers.into_iter().enumerate() {
            assert_eq!(label_of(receiver.blocking_recv().unwrap()) as usize, label);
        }
    }

    #[test]
    fn a_decision_after_a_panic_never_runs() {
        let worker = Worker::start().unwrap();
        let first = receiver(put(&worker, |_| panic!("a defect in a decision")));
        let second = receiver(put(&worker, quick(7)));
        assert!(first.blocking_recv().unwrap().is_err());
        assert_eq!(label_of(second.blocking_recv().unwrap()), 7);
    }

    #[test]
    fn a_decision_whose_receiver_the_test_dropped_does_not_run_or_leaves_its_slot_held() {
        let log = log();
        let worker = Worker::start().unwrap();
        let (a, hold_a) = held(0, &log);
        let running = receiver(put(&worker, a));
        assert_eq!(next(&hold_a.started), 0);
        let (ran_tx, ran) = mpsc::channel();
        let waiting = receiver(put(&worker, move |_| {
            ran_tx.send(()).unwrap();
            result(1)
        }));
        drop(running);
        drop(waiting);
        hold_a.release.send(()).unwrap();
        next(&ran);
        let mut again = Vec::new();
        for label in 0..5 {
            let run: Box<dyn FnOnce(Admitted) -> CallToolResult + Send> = if label == 0 {
                let (run, hold) = held(10, &log);
                again.push(hold);
                Box::new(run)
            } else {
                Box::new(quick(label))
            };
            assert!(
                matches!(put(&worker, run), Submission::Accepted(_)),
                "{label}"
            );
        }
        again[0].release.send(()).unwrap();
    }

    #[test]
    fn a_submission_after_close_is_accepted() {
        let worker = Worker::start().unwrap();
        worker.close();
        let Submission::Answer(answer) = put(&worker, quick(1)) else {
            panic!("a closed worker answers instead of accepting");
        };
        let value = answer.structured_content.unwrap();
        assert_eq!(value["code"], "server-overloaded");
        assert_eq!(value["retryable"], true);
    }

    #[test]
    fn an_unsupported_client_never_reaches_the_worker() {
        let worker = Worker::start().unwrap();
        let unsupported = Selection::UnknownHost {
            name: Some(Reported::Text("codex".into())),
            version: None,
        };
        let ran = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&ran);
        let submission = worker.submit("baley_version", &unsupported, None, 10, move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            result(1)
        });
        assert!(matches!(submission, Submission::Answer(_)));
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        assert!(*worker.drained().borrow());
    }

    #[tokio::test]
    async fn after_abandoning_a_waiting_decision_starts_when_the_running_one_is_released() {
        let log = log();
        let worker = Worker::start().unwrap();
        let (a, hold_a) = held(0, &log);
        let running = receiver(put(&worker, a));
        assert_eq!(next(&hold_a.started), 0);
        let (b, hold_b) = held(1, &log);
        let waiting = receiver(put(&worker, b));
        worker.abandon();
        hold_a.release.send(()).unwrap();
        assert_eq!(label_of(running.await.unwrap()), 0);
        worker.drained().wait_for(|drained| *drained).await.unwrap();
        assert!(hold_b.started.try_recv().is_err());
        assert!(waiting.await.is_err());
        assert_eq!(*log.lock().unwrap(), ["start 0", "end 0"]);
    }

    fn job(label: u32) -> Job {
        let (reply, _receiver) = oneshot::channel();
        Job {
            admitted: Admitted {
                called: super::super::gate::Called::Version,
                host: Host::ClaudeCode,
                client_version: "2.1.287".into(),
                needs_project: false,
            },
            run: Box::new(move |_| result(label)),
            reply,
        }
    }

    #[test]
    fn abandoning_leaves_the_queue_held_by_a_job_the_worker_never_picked_up() {
        let mut state = State {
            queue: Queue::new(),
            start: None,
        };
        let Ok(Placed::Started(first)) = state.queue.admit(job(0), 10) else {
            panic!("an idle queue starts the job");
        };
        state.start = Some(first);
        assert!(matches!(state.queue.admit(job(1), 10), Ok(Placed::Queued)));
        let (waiting, unstarted) = state.abandon();
        assert_eq!(waiting.len(), 1);
        assert!(unstarted.is_some());
        assert!(state.start.is_none());
        assert!(state.queue.is_closed());
        assert!(state.queue.is_drained());
    }

    #[tokio::test]
    async fn the_drained_signal_fires_while_a_decision_is_still_running() {
        let log = log();
        let worker = Worker::start().unwrap();
        let mut drained = worker.drained();
        assert!(*drained.borrow());
        let (a, hold_a) = held(0, &log);
        let running = receiver(put(&worker, a));
        assert_eq!(next(&hold_a.started), 0);
        assert!(!*drained.borrow_and_update());
        worker.close();
        assert!(!*drained.borrow());
        hold_a.release.send(()).unwrap();
        assert_eq!(label_of(running.await.unwrap()), 0);
        drained.wait_for(|drained| *drained).await.unwrap();
    }
}
