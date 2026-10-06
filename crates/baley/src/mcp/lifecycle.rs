//! Shutdown as transitions over supplied events and elapsed time.
//!
//! The server never asks for a checkpoint while its connection is open: there
//! is no idle checkpoint and no timer on a live session. Input ending or a
//! terminate signal is the one thing that starts shutdown. Admission stops,
//! accepted work gets up to [`SERVER_DRAIN_BOUND`], and exactly one checkpoint
//! attempt follows. Waiting work is abandoned at the bound, before the
//! checkpoint, so nothing starts during it. The decision still running at the
//! bound is left to SQLite's rollback. A second end signal changes nothing.
//!
//! Nothing here reads a clock, sleeps or touches the store. The caller supplies
//! how long has passed since the first end event and carries out each action.

use std::time::Duration;

use crate::store::writer::SERVER_DRAIN_BOUND;

/// Where shutdown stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The connection is open and the server answers calls.
    Serving,
    /// Admission is closed and accepted work is finishing.
    Draining,
    /// The checkpoint attempt was asked for and the process exits.
    Done,
}

/// Something that happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The client closed its end of the input.
    InputEnded,
    /// The process was told to terminate.
    Terminate,
    /// No decision is running or waiting, and no answer remains unsent. The
    /// caller sends it as soon as the worker reports that state.
    QueueDrained,
    /// This long has passed since the first end event.
    Elapsed(Duration),
}

/// What the caller must do, in the order given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Refuse every call from now on.
    StopAdmission,
    /// Keep waiting for the queue to drain or the bound to pass.
    Wait,
    /// Drop every waiting decision unstarted.
    AbandonWaiting,
    /// Make the one exit checkpoint attempt.
    AttemptCheckpoint {
        /// True when work was still open at the bound and was cut off.
        abandoned: bool,
    },
    /// End the process.
    Exit,
}

/// Applies one event. An event that changes nothing returns no actions.
pub fn step(state: State, event: Event) -> (State, Vec<Action>) {
    match (state, event) {
        (State::Serving, Event::InputEnded | Event::Terminate) => {
            (State::Draining, vec![Action::StopAdmission, Action::Wait])
        }
        (State::Draining, Event::QueueDrained) => (
            State::Done,
            vec![Action::AttemptCheckpoint { abandoned: false }, Action::Exit],
        ),
        (State::Draining, Event::Elapsed(elapsed)) if elapsed >= SERVER_DRAIN_BOUND => (
            State::Done,
            vec![
                Action::AbandonWaiting,
                Action::AttemptCheckpoint { abandoned: true },
                Action::Exit,
            ],
        ),
        (State::Draining, Event::Elapsed(_)) => (State::Draining, vec![Action::Wait]),
        (state, _) => (state, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUND_LESS_ONE_MS: Duration = Duration::from_millis(9_999);

    fn draining() -> State {
        step(State::Serving, Event::InputEnded).0
    }

    fn run(events: &[Event]) -> Vec<Action> {
        let mut state = State::Serving;
        let mut all = Vec::new();
        for event in events {
            let (next, actions) = step(state, *event);
            state = next;
            all.extend(actions);
        }
        all
    }

    fn checkpoints(actions: &[Action]) -> usize {
        actions
            .iter()
            .filter(|action| matches!(action, Action::AttemptCheckpoint { .. }))
            .count()
    }

    #[test]
    fn serving_with_an_empty_queue_over_any_elapsed_time_yields_a_checkpoint() {
        for secs in [0, 10, 600, 1_800, 86_400] {
            let (state, actions) = step(State::Serving, Event::Elapsed(Duration::from_secs(secs)));
            assert_eq!((state, actions), (State::Serving, vec![]), "{secs}s");
        }
        let (state, actions) = step(State::Serving, Event::QueueDrained);
        assert_eq!((state, actions), (State::Serving, vec![]));
    }

    #[test]
    fn input_ended_stops_admission_and_waits_instead_of_checkpointing() {
        for end in [Event::InputEnded, Event::Terminate] {
            let (state, actions) = step(State::Serving, end);
            assert_eq!(state, State::Draining);
            assert_eq!(actions, [Action::StopAdmission, Action::Wait]);
        }
    }

    #[test]
    fn input_ended_with_accepted_work_checkpoints_before_the_queue_drains() {
        let actions = run(&[
            Event::InputEnded,
            Event::Elapsed(Duration::ZERO),
            Event::Elapsed(Duration::from_secs(5)),
        ]);
        assert_eq!(checkpoints(&actions), 0);
        assert_eq!(
            actions,
            [
                Action::StopAdmission,
                Action::Wait,
                Action::Wait,
                Action::Wait
            ]
        );
    }

    #[test]
    fn draining_just_under_the_bound_with_work_open_yields_a_checkpoint() {
        let (state, actions) = step(draining(), Event::Elapsed(BOUND_LESS_ONE_MS));
        assert_eq!(state, State::Draining);
        assert_eq!(actions, [Action::Wait]);
    }

    #[test]
    fn at_the_bound_with_work_open_it_waits_instead_of_checkpointing_with_abandonment_noted() {
        let (state, actions) = step(draining(), Event::Elapsed(SERVER_DRAIN_BOUND));
        assert_eq!(state, State::Done);
        assert!(actions.contains(&Action::AttemptCheckpoint { abandoned: true }));
        assert_eq!(actions.last(), Some(&Action::Exit));
    }

    #[test]
    fn at_the_bound_the_checkpoint_is_asked_for_without_abandoning_the_waiting_work_first() {
        let (_, actions) = step(draining(), Event::Elapsed(SERVER_DRAIN_BOUND));
        let abandon = actions.iter().position(|a| *a == Action::AbandonWaiting);
        let checkpoint = actions
            .iter()
            .position(|a| matches!(a, Action::AttemptCheckpoint { .. }));
        assert!(abandon.is_some());
        assert!(abandon < checkpoint, "{actions:?}");
        let (_, long) = step(draining(), Event::Elapsed(Duration::from_secs(60)));
        assert_eq!(long, actions);
    }

    #[test]
    fn a_drain_that_finishes_in_time_reports_nothing_abandoned() {
        let (state, actions) = step(draining(), Event::QueueDrained);
        assert_eq!(state, State::Done);
        assert_eq!(
            actions,
            [Action::AttemptCheckpoint { abandoned: false }, Action::Exit]
        );
    }

    #[test]
    fn a_terminate_signal_after_input_ended_yields_a_second_checkpoint_attempt() {
        let sequences: [&[Event]; 4] = [
            &[
                Event::InputEnded,
                Event::Terminate,
                Event::QueueDrained,
                Event::Terminate,
                Event::InputEnded,
                Event::QueueDrained,
            ],
            &[
                Event::Terminate,
                Event::InputEnded,
                Event::Elapsed(SERVER_DRAIN_BOUND),
                Event::Terminate,
                Event::Elapsed(SERVER_DRAIN_BOUND),
                Event::QueueDrained,
            ],
            &[
                Event::InputEnded,
                Event::Elapsed(SERVER_DRAIN_BOUND),
                Event::QueueDrained,
            ],
            &[
                Event::InputEnded,
                Event::QueueDrained,
                Event::Elapsed(SERVER_DRAIN_BOUND),
            ],
        ];
        for events in sequences {
            assert_eq!(checkpoints(&run(events)), 1, "{events:?}");
        }
    }

    #[test]
    fn a_second_end_signal_while_draining_changes_nothing() {
        for again in [Event::InputEnded, Event::Terminate] {
            assert_eq!(step(draining(), again), (State::Draining, vec![]));
        }
    }

    #[test]
    fn nothing_is_asked_for_after_the_process_is_done() {
        for event in [
            Event::InputEnded,
            Event::Terminate,
            Event::QueueDrained,
            Event::Elapsed(Duration::from_secs(99)),
        ] {
            assert_eq!(step(State::Done, event), (State::Done, vec![]));
        }
    }

    #[test]
    fn input_ended_with_an_empty_queue_does_not_go_straight_to_the_one_checkpoint_attempt() {
        let actions = run(&[Event::InputEnded, Event::QueueDrained]);
        assert_eq!(
            actions,
            [
                Action::StopAdmission,
                Action::Wait,
                Action::AttemptCheckpoint { abandoned: false },
                Action::Exit
            ]
        );
    }
}
