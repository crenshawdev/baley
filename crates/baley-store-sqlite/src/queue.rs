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

pub(crate) struct WriterQueue {
    file: File,
}

/// Holds the queue until dropped.
pub(crate) struct QueueTurn<'a> {
    file: &'a File,
}

impl WriterQueue {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        Ok(Self { file })
    }

    /// Blocks until this store's turn to write.
    pub(crate) fn wait(&self) -> io::Result<QueueTurn<'_>> {
        self.file.lock()?;
        Ok(QueueTurn { file: &self.file })
    }
}

impl Drop for QueueTurn<'_> {
    fn drop(&mut self) {
        // Closing the file would release it too; the store keeps the file
        // open, so the turn ends here. A failed unlock leaves nothing to do.
        let _ = self.file.unlock();
    }
}
