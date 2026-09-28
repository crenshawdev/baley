//! Lease renewals scheduled from monotonic deadlines.
use baley_core::{TickGuard, Ticker};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// A supplied tick time or a stop observation.
pub(super) enum Wake {
    Tick(String),
    Stop,
}
/// The wait observation consumed by the ticker loop.
pub(super) trait Pace {
    fn wait(&mut self, interval: Duration) -> Wake;
}
/// Delivers supplied times until the pace stops.
pub(super) fn run_ticks(
    mut pace: impl Pace,
    interval: Duration,
    mut tick: Box<dyn FnMut(String) + Send>,
) {
    while let Wake::Tick(at) = pace.wait(interval) {
        tick(at);
    }
}
struct DeadlinePace {
    stop: Receiver<()>,
    next: Instant,
    clock: Arc<dyn Fn() -> String + Send + Sync>,
}
impl Pace for DeadlinePace {
    fn wait(&mut self, interval: Duration) -> Wake {
        match self
            .stop
            .recv_timeout(self.next.saturating_duration_since(Instant::now()))
        {
            Err(RecvTimeoutError::Timeout) => {
                self.next += interval;
                Wake::Tick((self.clock)())
            }
            _ => Wake::Stop,
        }
    }
}
/// Schedules lease renewals on one thread.
pub(super) struct ThreadTicker {
    clock: Arc<dyn Fn() -> String + Send + Sync>,
}
impl ThreadTicker {
    /// Builds the edge adapter from its supplied dependencies.
    pub(super) fn new(clock: Arc<dyn Fn() -> String + Send + Sync>) -> Self {
        Self { clock }
    }
}
// Keeps the stop decision testable without relying on thread scheduling.
trait Worker {
    fn join(self);
}
impl Worker for JoinHandle<()> {
    fn join(self) {
        let _ = JoinHandle::join(self);
    }
}
struct ThreadGuard<W = JoinHandle<()>> {
    stop: Sender<()>,
    thread: W,
}
impl<W: Worker> TickGuard for ThreadGuard<W> {
    fn stop(self: Box<Self>) {
        let _ = self.stop.send(());
        self.thread.join();
    }
}
/// The core asks for whole seconds; the pace waits a `Duration`.
fn interval(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}
impl Ticker for ThreadTicker {
    fn start(
        &mut self,
        interval_seconds: u64,
        tick: Box<dyn FnMut(String) + Send + 'static>,
    ) -> Box<dyn TickGuard> {
        let interval = interval(interval_seconds);
        let (stop, receiver) = mpsc::channel();
        let pace = DeadlinePace {
            stop: receiver,
            next: Instant::now() + interval,
            clock: self.clock.clone(),
        };
        Box::new(ThreadGuard {
            stop,
            thread: thread::spawn(move || run_ticks(pace, interval, tick)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Script {
        wakes: std::collections::VecDeque<Wake>,
        intervals: Arc<Mutex<Vec<Duration>>>,
    }
    impl Pace for Script {
        fn wait(&mut self, interval: Duration) -> Wake {
            self.intervals.lock().unwrap().push(interval);
            self.wakes.pop_front().unwrap_or(Wake::Stop)
        }
    }
    #[test]
    fn ticks_carry_each_supplied_time_in_order() {
        let times = Arc::new(Mutex::new(vec![]));
        let saved = times.clone();
        run_ticks(
            Script {
                wakes: [
                    Wake::Tick("one".into()),
                    Wake::Tick("two".into()),
                    Wake::Stop,
                ]
                .into(),
                intervals: Arc::default(),
            },
            Duration::from_secs(10),
            Box::new(move |at| saved.lock().unwrap().push(at)),
        );
        assert_eq!(*times.lock().unwrap(), ["one", "two"]);
    }
    #[test]
    fn the_pace_is_not_given_a_different_interval() {
        let intervals = Arc::default();
        run_ticks(
            Script {
                wakes: [Wake::Stop].into(),
                intervals: Arc::clone(&intervals),
            },
            Duration::from_secs(10),
            Box::new(|_| panic!("no tick")),
        );
        assert_eq!(*intervals.lock().unwrap(), [Duration::from_secs(10)]);
    }
    #[test]
    fn start_cannot_read_seconds_as_milliseconds() {
        assert_eq!(interval(10), Duration::from_millis(10_000));
    }
    struct BlockingPace {
        stop: Receiver<()>,
        first: bool,
    }
    impl Pace for BlockingPace {
        fn wait(&mut self, _: Duration) -> Wake {
            if self.first {
                self.first = false;
                return Wake::Tick("one".into());
            }
            self.stop.recv().unwrap();
            Wake::Stop
        }
    }
    struct Dropped(Sender<()>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    #[test]
    fn stopping_joins_and_drops_the_tick_before_returning() {
        let (stop, stopped) = mpsc::channel();
        let (dropped, drop_seen) = mpsc::channel();
        let (entered, entry) = mpsc::channel();
        let token = Dropped(dropped);
        let thread = thread::spawn(move || {
            run_ticks(
                BlockingPace {
                    stop: stopped,
                    first: true,
                },
                Duration::from_secs(10),
                Box::new(move |at| {
                    let _keep = &token;
                    entered.send(at).unwrap();
                }),
            )
        });
        assert_eq!(entry.recv().unwrap(), "one");
        let after_stop = stop.clone();
        Box::new(ThreadGuard { stop, thread }).stop();
        assert!(
            drop_seen.try_recv().is_ok(),
            "stop returned before dropping the tick"
        );
        assert!(after_stop.send(()).is_err());
        assert!(entry.try_recv().is_err());
    }
    #[test]
    fn stopping_cannot_omit_join_after_sending_stop() {
        struct ObservedWorker {
            stop: Receiver<()>,
            joined: Arc<Mutex<bool>>,
        }
        impl Worker for ObservedWorker {
            fn join(self) {
                assert_eq!(self.stop.try_recv(), Ok(()));
                *self.joined.lock().unwrap() = true;
            }
        }
        let (stop, receiver) = mpsc::channel();
        let joined = Arc::new(Mutex::new(false));
        Box::new(ThreadGuard {
            stop,
            thread: ObservedWorker {
                stop: receiver,
                joined: joined.clone(),
            },
        })
        .stop();
        assert!(*joined.lock().unwrap(), "stop returned without joining");
    }
}
