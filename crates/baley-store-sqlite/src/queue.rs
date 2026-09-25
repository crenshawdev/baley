//! The writer queue (design 0001, Processes and concurrency).
//!
//! Every write, maintenance included, takes a blocking exclusive lock on
//! `<home>/baley.db.writer` before `BEGIN IMMEDIATE`. The kernel parks
//! waiting writers and wakes them in turn, where SQLite's busy handler
//! sleeps and retries and starved writers for seconds under load.
//! `File::lock` is `flock` on Unix.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

pub(crate) struct WriterQueue {
    file: File,
    /// `flock` belongs to the open file, so two threads of one store would
    /// both hold it at once, and either could end the other's turn. This
    /// lock makes them take turns inside the process first.
    local: Mutex<()>,
}

/// Holds the queue until dropped.
pub(crate) struct QueueTurn<'a> {
    file: &'a File,
    _local: MutexGuard<'a, ()>,
}

impl WriterQueue {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        Ok(Self {
            file,
            local: Mutex::new(()),
        })
    }

    /// Blocks until this thread's turn to write, in this process and then
    /// across processes.
    pub(crate) fn wait(&self) -> io::Result<QueueTurn<'_>> {
        // A panic during an earlier turn leaves nothing to repair here.
        let local = self.local.lock().unwrap_or_else(PoisonError::into_inner);
        self.file.lock()?;
        Ok(QueueTurn {
            file: &self.file,
            _local: local,
        })
    }
}

impl Drop for QueueTurn<'_> {
    fn drop(&mut self) {
        // Released before the in-process lock, which drops after this. A
        // failed unlock leaves nothing to do: closing the file releases it.
        let _ = self.file.unlock();
    }
}
