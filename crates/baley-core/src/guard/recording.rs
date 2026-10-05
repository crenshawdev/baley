//! The audit precondition (design 0010, GRD-R9): a decision that asks the
//! owner must be recorded first, because an ask that cannot be replayed or
//! audited is not a gate.

use super::answer::Answer;
use super::reason;

/// Whether the hook can record the answer it is about to give. The hook
/// supplies it; nothing here reads or writes a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditPrecondition {
    /// The decision can be recorded.
    Recorded,
    /// The decision cannot be recorded: the store could not record, the views
    /// need a rebuild, or the call carries no `tool_use_id` to replay by. A
    /// missing identity is unrecordable and is never replaced by an invented
    /// one.
    Unrecordable,
}

/// The final answer once the audit precondition is known.
///
/// A recorded answer is unchanged, so a recorded ask about torn settings
/// stays an ask. When the decision is unrecordable, an ask becomes a deny that
/// says so and keeps the original reason, a deny stays the same deny, a pass
/// on failure stays a pass on failure and a plain pass is untouched.
pub fn record_answer(answer: Answer, precondition: AuditPrecondition) -> Answer {
    match (precondition, answer) {
        (AuditPrecondition::Unrecordable, Answer::Ask(original)) => {
            Answer::Deny(reason::could_not_record(&original))
        }
        (_, answer) => answer,
    }
}
