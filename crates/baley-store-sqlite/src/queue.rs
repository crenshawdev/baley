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

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
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
