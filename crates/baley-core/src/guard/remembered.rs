//! The remembered policy (design 0010, GRD-R7): the denials of the last
//! complete policy, kept per session project, target checkout and host so a
//! torn settings file cannot open the door.

/// What a complete policy leaves behind for torn settings: its denials and
/// nothing that allows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DenialParts {
    /// Whether `git.on_protected` was `refuse`.
    pub refuse: bool,
    /// Whether `git.guard_hard_fail` was on.
    pub hard_fail: bool,
    /// `git.protected_branches`, kept only when a denial is on, since only a
    /// denial needs to prove a branch protected. Empty otherwise.
    pub protected_branches: Vec<String>,
}
