//! The guard's settings as the judge reads them (design 0010, GRD-R5 and
//! GRD-R7).

use crate::policy::{EffectivePolicy, OnProtected, Unavailable, Value};

/// What the three guard settings say, read from a complete effective policy.
///
/// The same type is the remembered policy: the settings of the last complete
/// policy, which the hook supplies when the current files are torn. The judge
/// uses a remembered policy only to deny, never to allow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardSettings {
    /// `git.protected_branches`, in file order. May be empty.
    pub protected_branches: Vec<String>,
    /// `git.on_protected`.
    pub on_protected: OnProtected,
    /// `git.guard_hard_fail`.
    pub hard_fail: bool,
}
impl GuardSettings {
    /// Reads the three settings from `policy`.
    ///
    /// # Panics
    ///
    /// When the policy was merged over a schema without `git.protected_branches`,
    /// `git.on_protected` or `git.guard_hard_fail`.
    pub fn from_policy(policy: &EffectivePolicy) -> GuardSettings {
        let value = |name: &str| {
            policy
                .settings
                .get(name)
                .and_then(|effective| effective.value.as_ref())
        };
        let Some(Value::BranchList(protected_branches)) = value("git.protected_branches") else {
            panic!("git.protected_branches holds no branch list");
        };
        let Some(Value::OnProtected(on_protected)) = value("git.on_protected") else {
            panic!("git.on_protected holds no on-protected value");
        };
        let Some(Value::Bool(hard_fail)) = value("git.guard_hard_fail") else {
            panic!("git.guard_hard_fail holds no boolean");
        };
        GuardSettings {
            protected_branches: protected_branches.clone(),
            on_protected: *on_protected,
            hard_fail: *hard_fail,
        }
    }
}

/// The settings the judge is given for one call.
///
/// A readable layer is never merged into a partial policy. Either every file
/// gave a complete policy, or one of them is torn and the call is judged on
/// the torn file alone. An absent global file is valid and an absent project
/// file means no project, so neither is a torn input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsInput {
    /// The typed settings of a complete effective policy.
    Complete(GuardSettings),
    /// The first settings file that gave no policy.
    Torn(Unavailable),
}
