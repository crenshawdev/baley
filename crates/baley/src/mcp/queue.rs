//! The session queue's admission and release, as plain state.
//!
//! One queue serves the whole session. The main session and every subagent
//! share it, and the state holds no caller or agent identity, so no one can
//! have a share of their own. It runs one decision and holds four waiting, in
//! arrival order. The byte cap counts the running decision's raw bytes as well
//! as the waiting ones: with four waiting calls of 4 MiB the waiting bytes
//! alone could never pass 16 MiB, so the cap would bind nothing.
//!
//! Nothing here starts a thread, waits or reads a clock. The worker owns the
//! locking and calls these transitions.

use std::collections::VecDeque;

/// How many decisions may wait behind the running one.
pub const WAITING_SLOTS: usize = 4;

/// The most raw frame bytes the running decision and the waiting ones may hold
/// together.
pub const BYTE_CAP: usize = 16_777_216;

/// Where an accepted call went.
#[derive(Debug, PartialEq, Eq)]
pub enum Placed<T> {
    /// Nothing was running, so this call runs now. The caller must start it.
    Started(T),
    /// It waits its turn behind the others.
    Queued,
}

/// Why a call was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Every waiting slot is full, or the bytes would pass [`BYTE_CAP`].
    Overloaded,
    /// The queue no longer admits calls.
    Closed,
}

/// The queue's state. `T` is whatever the worker runs for one call.
#[derive(Debug)]
pub struct Queue<T> {
    running: Option<usize>,
    waiting: VecDeque<(T, usize)>,
    bytes: usize,
    closed: bool,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Queue<T> {
    /// An empty, open queue.
    pub fn new() -> Self {
        Self {
            running: None,
            waiting: VecDeque::new(),
            bytes: 0,
            closed: false,
        }
    }

    /// Takes a call of `bytes` raw frame bytes, or says why not. A refused
    /// call changes nothing. A call that brings the total to exactly
    /// [`BYTE_CAP`] is taken.
    pub fn admit(&mut self, item: T, bytes: usize) -> Result<Placed<T>, Refused> {
        if self.closed {
            return Err(Refused::Closed);
        }
        let slot_free = self.running.is_none() || self.waiting.len() < WAITING_SLOTS;
        if !slot_free || self.bytes.saturating_add(bytes) > BYTE_CAP {
            return Err(Refused::Overloaded);
        }
        self.bytes += bytes;
        if self.running.is_none() {
            self.running = Some(bytes);
            return Ok(Placed::Started(item));
        }
        self.waiting.push_back((item, bytes));
        Ok(Placed::Queued)
    }

    /// The running decision ended, however it ended. Frees its slot and bytes
    /// and promotes the oldest waiting call, which the caller must now start.
    pub fn finish(&mut self) -> Option<T> {
        self.bytes -= self.running.take()?;
        let (item, bytes) = self.waiting.pop_front()?;
        self.running = Some(bytes);
        Some(item)
    }

    /// Stops admitting. Calls already accepted still run and finish.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Closes, and drops every waiting call unstarted, returning them. The
    /// running decision is left alone.
    pub fn abandon(&mut self) -> Vec<T> {
        self.closed = true;
        let dropped: Vec<(T, usize)> = self.waiting.drain(..).collect();
        self.bytes -= dropped.iter().map(|(_, bytes)| bytes).sum::<usize>();
        dropped.into_iter().map(|(item, _)| item).collect()
    }

    /// Whether the queue refuses new calls.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Whether nothing is running and nothing waits.
    pub fn is_drained(&self) -> bool {
        self.running.is_none() && self.waiting.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB4: usize = 4 * 1024 * 1024;

    fn admit_all(queue: &mut Queue<u32>, count: u32, bytes: usize) -> Vec<bool> {
        (0..count).map(|n| queue.admit(n, bytes).is_ok()).collect()
    }

    #[test]
    fn a_sixth_call_is_accepted_while_five_are_held() {
        let mut queue = Queue::new();
        assert_eq!(admit_all(&mut queue, 5, 10), [true; 5]);
        assert_eq!(queue.admit(5, 10), Err(Refused::Overloaded));
    }

    #[test]
    fn a_one_byte_call_is_accepted_when_a_slot_is_free_but_the_bytes_are_spent() {
        let mut queue = Queue::new();
        assert_eq!(admit_all(&mut queue, 4, MIB4), [true; 4]);
        assert_eq!(queue.admit(9, 1), Err(Refused::Overloaded));
    }

    #[test]
    fn a_call_that_brings_the_total_to_exactly_the_cap_is_refused() {
        let mut queue = Queue::new();
        assert_eq!(admit_all(&mut queue, 3, MIB4), [true; 3]);
        assert!(queue.admit(3, MIB4).is_ok());
        let mut queue = Queue::new();
        assert!(queue.admit(0, BYTE_CAP).is_ok());
        let mut queue = Queue::new();
        assert!(queue.admit(0, BYTE_CAP + 1).is_err());
    }

    #[test]
    fn a_refused_call_still_holds_its_bytes() {
        let mut queue = Queue::new();
        admit_all(&mut queue, 3, MIB4);
        assert_eq!(queue.admit(7, 5 * MIB4), Err(Refused::Overloaded));
        assert!(queue.admit(8, MIB4).is_ok());
    }

    #[test]
    fn waiting_calls_are_promoted_out_of_arrival_order() {
        let mut queue = Queue::new();
        assert_eq!(queue.admit(0, 1), Ok(Placed::Started(0)));
        for n in 1..=4 {
            assert_eq!(queue.admit(n, 1), Ok(Placed::Queued));
        }
        let promoted: Vec<u32> = std::iter::from_fn(|| queue.finish()).collect();
        assert_eq!(promoted, [1, 2, 3, 4]);
        assert!(queue.is_drained());
    }

    #[test]
    fn two_decisions_are_ever_running() {
        let mut queue = Queue::new();
        let mut started = 0;
        for n in 0..5 {
            if let Ok(Placed::Started(_)) = queue.admit(n, 1) {
                started += 1;
            }
        }
        assert_eq!(started, 1);
        for _ in 0..4 {
            assert!(queue.finish().is_some());
        }
        assert_eq!(queue.finish(), None);
    }

    #[test]
    fn finishing_does_not_free_the_slot_and_bytes() {
        let mut queue = Queue::new();
        assert_eq!(admit_all(&mut queue, 4, MIB4), [true; 4]);
        while queue.finish().is_some() {}
        assert!(queue.is_drained());
        assert_eq!(admit_all(&mut queue, 4, MIB4), [true; 4]);
        let mut queue = Queue::new();
        admit_all(&mut queue, 5, 1);
        while queue.finish().is_some() {}
        assert_eq!(admit_all(&mut queue, 5, 1), [true; 5]);
    }

    #[test]
    fn a_finish_with_nothing_running_frees_bytes_it_never_held() {
        let mut queue = Queue::new();
        assert_eq!(queue.finish(), None);
        assert!(queue.admit(0, BYTE_CAP).is_ok());
        assert_eq!(queue.admit(1, 1), Err(Refused::Overloaded));
    }

    #[test]
    fn a_closed_queue_accepts_a_call() {
        let mut queue = Queue::new();
        queue.admit(0, 1).unwrap();
        queue.admit(1, 1).unwrap();
        queue.close();
        assert_eq!(queue.admit(2, 1), Err(Refused::Closed));
        assert!(queue.is_closed());
        assert_eq!(queue.finish(), Some(1));
        assert_eq!(queue.finish(), None);
        assert_eq!(queue.admit(3, 1), Err(Refused::Closed));
    }

    #[test]
    fn calls_from_different_agents_each_get_their_own_five_slots() {
        let mut queue: Queue<(&str, u32)> = Queue::new();
        let mine: Vec<bool> = (0..5)
            .map(|n| queue.admit(("main", n), 1).is_ok())
            .collect();
        let theirs: Vec<bool> = (0..5)
            .map(|n| queue.admit(("subagent", n), 1).is_ok())
            .collect();
        assert_eq!(mine, [true; 5]);
        assert_eq!(theirs, [false; 5]);
    }

    #[test]
    fn abandoning_leaves_a_waiting_call_to_start_and_its_bytes_held() {
        let mut queue = Queue::new();
        assert_eq!(admit_all(&mut queue, 4, MIB4), [true; 4]);
        assert_eq!(queue.abandon(), [1, 2, 3]);
        assert!(!queue.is_drained());
        assert_eq!(queue.finish(), None);
        assert!(queue.is_drained());
        assert_eq!(queue.admit(9, 1), Err(Refused::Closed));
        let mut queue = Queue::new();
        queue.admit(0, MIB4).unwrap();
        queue.admit(1, 3 * MIB4).unwrap();
        queue.abandon();
        assert_eq!(queue.bytes, MIB4);
    }
}
