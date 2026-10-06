//! The guard's one time budget (design 0010, GRD-R14).
//!
//! One guard call has to answer inside the host's hook timeout, the
//! `"timeout": 10` that `hooks/hooks.json` gives `baley guard`. The budget
//! starts when the guard starts, before it reads its input. Git and storage
//! waits each have one allowance for the whole call, and no step runs past the
//! end of the work time, so the reserve after it is left for killing and
//! reaping a child and writing the answer. Each grant is decided from the time
//! elapsed since the start and the time already spent, so no step resets the
//! total.
//!
//! The input is bounded by size, not time: at most
//! [`crate::hook_input::MAX_INPUT_BYTES`] bytes, read inside the work time.
//!
//! Nothing here reads a clock except the reading function the caller supplies
//! when it builds a [`Budget`].

use std::time::{Duration, Instant};

/// The host's hook timeout for `baley guard`, from `hooks/hooks.json`.
pub const HOST_TIMEOUT: Duration = Duration::from_secs(10);
/// Work ends this long after the guard starts.
pub const WORK: Duration = Duration::from_secs(8);
/// All of one guard call's git launches together run at most this long.
pub const GIT: Duration = Duration::from_secs(5);
/// All of one guard call's storage waits together last at most this long:
/// the cap the SQLite adapter enforces on a guard store.
pub const STORAGE: Duration = baley_store_sqlite::GUARD_STORAGE_CAP;
/// The least of the host timeout left after work, for killing and reaping a
/// child and writing the answer.
pub const MIN_RESERVE: Duration = Duration::from_secs(1);

/// The timeout for the guard's next git launch: the smaller of git's time left
/// and the work time left. `None` once either is spent, because a zero timeout
/// would still start git and kill it at once.
pub fn git_grant(elapsed: Duration, git_spent: Duration) -> Option<Duration> {
    let left = GIT
        .saturating_sub(git_spent)
        .min(WORK.saturating_sub(elapsed));
    (!left.is_zero()).then_some(left)
}

/// The storage wait the guard may still spend: the smaller of storage time
/// left and work time left, zero once either is spent.
pub fn storage_time(elapsed: Duration, storage_spent: Duration) -> Duration {
    STORAGE
        .saturating_sub(storage_spent)
        .min(WORK.saturating_sub(elapsed))
}

/// Time granted to one guard git launch. Only [`Budget::git`] makes one, so a
/// guard launch never runs on a duration its caller picked.
#[derive(Debug, PartialEq, Eq)]
pub struct GitGrant {
    timeout: Duration,
    granted_at: Duration,
}

impl GitGrant {
    /// How long the launch may run.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// One guard call's budget: what git and storage have spent, and the clock the
/// time since the start is read from.
pub struct Budget {
    elapsed: Box<dyn FnMut() -> Duration>,
    git_spent: Duration,
    storage_spent: Duration,
}

impl Budget {
    /// The budget of a guard call starting now, read from the monotonic clock.
    pub fn start() -> Self {
        let started = Instant::now();
        Self::with_clock(move || started.elapsed())
    }

    /// A budget whose time since the start is whatever `elapsed` reads.
    pub fn with_clock(elapsed: impl FnMut() -> Duration + 'static) -> Self {
        Self {
            elapsed: Box::new(elapsed),
            git_spent: Duration::ZERO,
            storage_spent: Duration::ZERO,
        }
    }

    /// Time for the next git launch, or `None` once git's or the work's time
    /// is spent.
    pub fn git(&mut self) -> Option<GitGrant> {
        let now = (self.elapsed)();
        git_grant(now, self.git_spent).map(|timeout| GitGrant {
            timeout,
            granted_at: now,
        })
    }

    /// Charges git with the time since `grant` was given, however the launch
    /// ended.
    pub fn charge_git(&mut self, grant: GitGrant) {
        self.git_spent += (self.elapsed)().saturating_sub(grant.granted_at);
    }

    /// The storage wait still allowed.
    pub fn storage(&mut self) -> Duration {
        storage_time((self.elapsed)(), self.storage_spent)
    }

    /// Charges storage with a wait it measured.
    pub fn charge_storage(&mut self, waited: Duration) {
        self.storage_spent += waited;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn the_first_git_launch_gets_five_seconds_not_the_old_nine() {
        assert_eq!(git_grant(ms(0), ms(0)), Some(ms(5_000)));
    }

    #[test]
    fn late_git_gets_only_the_work_time_left_not_its_whole_allowance() {
        assert_eq!(git_grant(ms(6_000), ms(0)), Some(ms(2_000)));
    }

    #[test]
    fn git_time_already_spent_is_not_granted_again() {
        assert_eq!(git_grant(ms(4_500), ms(4_000)), Some(ms(1_000)));
    }

    #[test]
    fn spent_git_or_work_time_launches_nothing_rather_than_a_zero_timeout() {
        for (elapsed, spent) in [(5_000, 5_000), (6_000, 7_000), (8_000, 0), (9_500, 0)] {
            assert_eq!(
                git_grant(ms(elapsed), ms(spent)),
                None,
                "{elapsed} ms elapsed, {spent} ms spent"
            );
        }
    }

    #[test]
    fn no_git_grant_runs_into_the_reserve() {
        assert!(WORK + MIN_RESERVE <= HOST_TIMEOUT);
        for elapsed in (0..=10_000).step_by(250) {
            for spent in (0..=6_000).step_by(250) {
                let Some(grant) = git_grant(ms(elapsed), ms(spent)) else {
                    continue;
                };
                let end = ms(elapsed) + grant;
                assert!(end <= ms(8_000), "{elapsed} + {grant:?} runs past work");
                assert!(
                    ms(10_000) - end >= ms(1_000),
                    "{elapsed} + {grant:?} leaves under a second"
                );
                assert!(grant <= ms(5_000), "{grant:?} is over git's allowance");
            }
        }
    }

    #[test]
    fn storage_never_outlasts_its_allowance_or_the_work_time() {
        assert_eq!(storage_time(ms(0), ms(0)), ms(2_000));
        assert_eq!(storage_time(ms(7_500), ms(0)), ms(500));
        assert_eq!(storage_time(ms(1_000), ms(2_000)), ms(0));
        assert_eq!(storage_time(ms(8_500), ms(0)), ms(0));
    }

    #[test]
    fn a_second_launch_is_charged_for_the_first_not_reset() {
        let now = Rc::new(Cell::new(ms(0)));
        let clock = Rc::clone(&now);
        let mut budget = Budget::with_clock(move || clock.get());

        let first = budget.git().expect("a grant at the start");
        assert_eq!(first.timeout(), ms(5_000));
        now.set(ms(2_000));
        budget.charge_git(first);

        let second = budget.git().expect("time is left");
        assert_eq!(second.timeout(), ms(3_000));
    }
}
