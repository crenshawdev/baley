//! The writer queue, the maintenance lock and the time batched maintenance
//! runs on (design 0001, Processes and concurrency).
//!
//! Every write, maintenance included, takes a blocking exclusive lock on
//! `<home>/baley.db.writer` before `BEGIN IMMEDIATE`. The kernel parks
//! waiting writers and wakes them in turn, where SQLite's busy handler
//! sleeps and retries and starved writers for seconds under load. A rebuild
//! or a view verification also holds `<home>/baley.db.maintenance` from
//! start to end, so two of them never interleave, while commands, which do
//! not take it, go on between their batches. `File::lock` is `flock` on
//! Unix.
//!
//! A store opened for the guard never blocks on either lock. It tries the
//! in-process lock and then `File::try_lock`, `flock` with `LOCK_NB`, and
//! tries again after a short pause until its storage time is spent, when it
//! answers `StoreError::Busy`.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::time::{Duration, Instant};

/// An exclusive lock shared by the threads of this process and by other
/// processes: the writer queue or the maintenance lock.
pub(crate) struct FileLock {
    file: File,
    /// `flock` belongs to the open file, so two threads of one store would
    /// both hold it at once, and either could end the other's turn. This
    /// lock makes them take turns inside the process first.
    local: Mutex<()>,
    /// Run each time a turn has been released, so a test can act at that
    /// instant through another store.
    #[cfg(test)]
    on_release: Mutex<Option<Box<dyn FnMut() + Send>>>,
}

/// Holds the lock until dropped.
pub(crate) struct Turn<'a> {
    lock: &'a FileLock,
    local: Option<MutexGuard<'a, ()>>,
}

impl FileLock {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .mode(0o600)
            .truncate(false)
            .write(true)
            .open(path)?;
        Ok(Self {
            file,
            local: Mutex::new(()),
            #[cfg(test)]
            on_release: Mutex::new(None),
        })
    }

    /// Blocks until this thread's turn, in this process and then across
    /// processes. There is no timeout: the holder always finishes.
    pub(crate) fn wait(&self) -> io::Result<Turn<'_>> {
        // A panic during an earlier turn leaves nothing to repair here.
        let local = self.local.lock().unwrap_or_else(PoisonError::into_inner);
        self.file.lock()?;
        Ok(Turn {
            lock: self,
            local: Some(local),
        })
    }

    /// This thread's turn if it is free now, in this process and then across
    /// processes, or `None` when another holds it. It never waits.
    pub(crate) fn try_wait(&self) -> io::Result<Option<Turn<'_>>> {
        let local = match self.local.try_lock() {
            Ok(local) => local,
            // A panic during an earlier turn leaves nothing to repair here.
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Ok(None),
        };
        match self.file.try_lock() {
            Ok(()) => Ok(Some(Turn {
                lock: self,
                local: Some(local),
            })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error),
        }
    }

    /// Runs `action` each time a turn of this lock has been released, in
    /// the releasing thread, before it goes on.
    #[cfg(test)]
    pub(crate) fn at_release(&self, action: impl FnMut() + Send + 'static) {
        *self
            .on_release
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(action));
    }

    #[cfg(test)]
    fn released(&self) {
        let slot = || {
            self.on_release
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        // Taken out while it runs, so an action that takes and releases
        // this lock again does not wait on itself.
        let taken = slot().take();
        if let Some(mut action) = taken {
            action();
            slot().get_or_insert(action);
        }
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        // Released before the in-process lock. A failed unlock leaves
        // nothing to do: closing the file releases it.
        let _ = self.lock.file.unlock();
        drop(self.local.take());
        #[cfg(test)]
        self.lock.released();
    }
}

/// The most events one replay batch applies.
pub(crate) const BATCH_EVENTS: usize = 200;

/// The most document rows one cleanup turn removes.
pub(crate) const BATCH_ROWS: usize = 200;

/// Once a batch has held the queue this long it stops before its next event
/// or table.
pub(crate) const BATCH_TIME: Duration = Duration::from_millis(15);

/// The clock and the pause every rebuild, cleanup and view verification
/// batch runs on. A store holds one, so tests supply their own and no store
/// test reads a live clock or sleeps.
pub trait Timing: Send + Sync {
    /// A monotonic reading: the time since some fixed origin.
    fn now(&self) -> Duration;

    /// Waits for `duration`, called after a batch released the queue.
    fn pause(&self, duration: Duration);
}

/// The production timing: the monotonic clock, and a sleep.
#[derive(Debug, Default, Clone, Copy)]
pub struct Monotonic;

impl Timing for Monotonic {
    fn now(&self) -> Duration {
        // The origin is fixed at the first reading, so building a store
        // reads no clock.
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed()
    }

    fn pause(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// How long to pause after a batch that held the queue for `held`: as long
/// again, so a waiting writer gets its turn before the next batch, which
/// the lock alone does not promise (design 0001, Performance).
pub(crate) fn pause_for(held: Duration) -> Duration {
    held
}

/// How long a bounded acquisition pauses before it tries a taken lock
/// again: the 10 ms the process port polls a child's exit at, short beside
/// the guard's 2 s of storage time.
pub(crate) const RETRY_PAUSE: Duration = Duration::from_millis(10);

/// What an acquisition that does not hold its lock yet does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Next {
    /// Take it with the blocking call, as a normal store always has.
    Block,
    /// Try it once without waiting.
    Try,
    /// Pause this long, then decide again.
    Retry(Duration),
    /// The storage time is spent: answer `StoreError::Busy` and try no more.
    Busy,
}

/// The next step of an acquisition. `blocked` says whether its latest try
/// found the lock taken, false before the first try and after a pause.
/// `remaining` is the storage time left, `None` on a normal store, which
/// waits as long as it takes and is never busy. A bounded store with no time
/// left tries nothing, so no new turn begins after its deadline.
pub(crate) fn next(blocked: bool, remaining: Option<Duration>) -> Next {
    match remaining {
        None => Next::Block,
        Some(remaining) if remaining.is_zero() => Next::Busy,
        Some(remaining) if blocked => Next::Retry(RETRY_PAUSE.min(remaining)),
        Some(_) => Next::Try,
    }
}

/// What an acquisition came to.
pub(crate) enum Taken<T> {
    /// The lock, taken by a nonblocking try.
    Held(T),
    /// A normal store: take it with the blocking call.
    Block,
    /// The storage time ran out first.
    Busy,
}

/// Takes a lock through `attempt`, a nonblocking try that gives `None` when
/// the lock is taken, as `next` directs. `deadline` is the reading of
/// `timing` after which a bounded store tries no more. A normal store passes
/// `None`, reads no clock, makes no try and is told to block.
pub(crate) fn acquire<T, E>(
    deadline: Option<Duration>,
    timing: &dyn Timing,
    mut attempt: impl FnMut() -> Result<Option<T>, E>,
) -> Result<Taken<T>, E> {
    let mut blocked = false;
    loop {
        let remaining = deadline.map(|deadline| deadline.saturating_sub(timing.now()));
        match next(blocked, remaining) {
            Next::Block => return Ok(Taken::Block),
            Next::Busy => return Ok(Taken::Busy),
            Next::Retry(pause) => {
                timing.pause(pause);
                blocked = false;
            }
            Next::Try => match attempt()? {
                Some(held) => return Ok(Taken::Held(held)),
                None => blocked = true,
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod scripted {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use super::Timing;

    /// Readings the test supplies and the pauses the store asked for. Once
    /// the supplied readings run out, each reading advances the last one by
    /// `step`.
    pub(crate) struct Scripted {
        readings: Mutex<VecDeque<Duration>>,
        last: Mutex<Duration>,
        step: Duration,
        pauses: Mutex<Vec<Duration>>,
        /// Run in every pause, where the store holds no queue turn, so a
        /// test can act between two batches it cannot otherwise reach.
        during_pause: Mutex<Option<Box<dyn FnMut() + Send>>>,
    }

    impl Scripted {
        /// Time that stands still, so every batch runs to its event bound.
        pub(crate) fn still() -> Arc<Self> {
            Self::stepping(Duration::ZERO)
        }

        /// Each reading `step` past the last, so a step of the batch bound
        /// or more ends every batch after its first event.
        pub(crate) fn stepping(step: Duration) -> Arc<Self> {
            Arc::new(Self {
                readings: Mutex::new(VecDeque::new()),
                last: Mutex::new(Duration::ZERO),
                step,
                pauses: Mutex::new(Vec::new()),
                during_pause: Mutex::new(None),
            })
        }

        /// Runs `action` once, in the next pause.
        pub(crate) fn at_next_pause(&self, action: impl FnOnce() + Send + 'static) {
            let mut action = Some(action);
            self.at_every_pause(move || {
                if let Some(action) = action.take() {
                    action();
                }
            });
        }

        /// Runs `action` in every pause from now on.
        pub(crate) fn at_every_pause(&self, action: impl FnMut() + Send + 'static) {
            *self
                .during_pause
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(action));
        }

        /// The next readings, in order.
        pub(crate) fn script(&self, readings: &[Duration]) {
            self.readings
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(readings);
        }

        /// Every pause asked for so far.
        pub(crate) fn pauses(&self) -> Vec<Duration> {
            self.pauses
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl Timing for Scripted {
        fn now(&self) -> Duration {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            let next = self
                .readings
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or(*last + self.step);
            *last = next;
            next
        }

        fn pause(&self, duration: Duration) {
            self.pauses
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(duration);
            let slot = || {
                self.during_pause
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
            };
            // Taken out while it runs, so an action that pauses this timing
            // again does not wait on itself.
            let taken = slot().take();
            if let Some(mut action) = taken {
                action();
                slot().get_or_insert(action);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::convert::Infallible;
    use std::time::Duration;

    use super::scripted::Scripted;
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// A try that finds the lock taken `taken` times, then takes it, and
    /// fails the test past `allowed` calls, so unbounded retries fail
    /// instead of hanging.
    fn scripted_try(
        calls: &Cell<u32>,
        taken: u32,
        allowed: u32,
    ) -> impl FnMut() -> Result<Option<()>, Infallible> + '_ {
        move || {
            calls.set(calls.get() + 1);
            assert!(
                calls.get() <= allowed,
                "try {} past the {allowed} allowed",
                calls.get()
            );
            Ok((calls.get() > taken).then_some(()))
        }
    }

    #[test]
    fn a_bounded_wait_retries_past_its_storage_time() {
        let timing = Scripted::stepping(ms(400));
        timing.script(&[Duration::ZERO]);
        let calls = Cell::new(0);
        // Readings 0, 0.4, 0.8 and 1.2 s have time left, so at most four
        // tries; 1.6 s is the first reading at or past the 1.5 s deadline.
        let taken = acquire(
            Some(ms(1_500)),
            timing.as_ref(),
            scripted_try(&calls, u32::MAX, 4),
        );
        assert!(matches!(taken, Ok(Taken::Busy)));
        assert_eq!(timing.now(), ms(2_000), "read past the first spent reading");
        assert!(calls.get() >= 1);
        let pauses = timing.pauses();
        assert!(!pauses.is_empty());
        // 0.3 s is the least time left at any reading before the deadline.
        assert!(pauses.iter().all(|pause| *pause <= ms(300)), "{pauses:?}");
    }

    #[test]
    fn a_retry_pause_outlasts_the_time_left() {
        assert_eq!(next(true, Some(ms(4))), Next::Retry(ms(4)));
        assert_eq!(next(true, Some(ms(1_000))), Next::Retry(RETRY_PAUSE));
    }

    #[test]
    fn a_lock_freed_on_the_second_try_is_not_taken() {
        let timing = Scripted::still();
        let calls = Cell::new(0);
        let taken = acquire(Some(ms(1_500)), timing.as_ref(), scripted_try(&calls, 1, 2));
        assert!(matches!(taken, Ok(Taken::Held(()))));
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn a_spent_storage_time_still_tries_the_lock() {
        let timing = Scripted::still();
        timing.script(&[ms(1_500)]);
        let calls = Cell::new(0);
        let taken = acquire(Some(ms(1_500)), timing.as_ref(), scripted_try(&calls, 0, 0));
        assert!(matches!(taken, Ok(Taken::Busy)));
        assert_eq!(next(false, Some(Duration::ZERO)), Next::Busy);
    }

    #[test]
    fn a_normal_store_answers_busy_instead_of_waiting() {
        // Given the same would-block outcomes a bounded wait gives up on.
        assert_eq!(next(true, None), Next::Block);
        assert_eq!(next(false, None), Next::Block);
        let timing = Scripted::still();
        timing.script(&[ms(11)]);
        let calls = Cell::new(0);
        let taken = acquire(None, timing.as_ref(), scripted_try(&calls, u32::MAX, 0));
        assert!(matches!(taken, Ok(Taken::Block)));
        assert_eq!(timing.now(), ms(11), "a normal store read the clock");
    }
}
