//! The checkout admission plan: what one admitting command requests, and in
//! what order, from supplied observations (design 0001 Project identity and
//! policy, EVD-R17).

use serde_json::Value;

use super::event::{Checkout, seen_payload};
use super::judge::{CheckoutVerdict, ProjectIdConflict, judge_checkout};

/// One request of an admitting command, in the order it is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutOperation {
    /// Record one `checkout.seen` with this payload.
    RecordSeen(Value),
    /// Run the policy step.
    PolicyStep,
    /// Run the command's own events.
    Command,
}

/// What a checkout admission requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutAction {
    /// A fork. Nothing is requested and nothing is recorded in the project.
    Refuse(ProjectIdConflict),
    /// Make these requests in this order.
    Proceed(Vec<CheckoutOperation>),
}

/// A checkout admission's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutPlan {
    /// The policy version the checkout admission command carries: the one
    /// the next policy record replaces, or 0 when none is stored. It rides a
    /// refusal too, because the store step opens its transaction and builds
    /// the command before the judgement is made again inside it.
    pub policy_version: u64,
    /// Whether to refuse, or what to request.
    pub action: CheckoutAction,
}

/// Plans the checkout admission of `checkout` against `rows`, the bodies of
/// the project's `checkout` rows, and `stored_policy`, the body of the
/// `policy` document stored for this path and no host, if any.
///
/// Checkout admission performs the `checkout.seen` request in one
/// transaction with the judgement, so two checkouts admitted at once are
/// judged in queue order. Each caller then runs the policy step and its own
/// command in this order: the binary's ledger commands and `baley init`, and
/// the session server's write preparation for each project write. The guard
/// never runs checkout admission. A refusal records nothing in the project, and
/// is never a recorded refusal.
///
/// The order is `checkout.seen` only when the checkout is new or changed,
/// then the policy step, then the command. The version is the stored
/// body's positive `version`, read as the policy step reads it, or 0.
pub fn plan_checkout_admission(
    checkout: &Checkout,
    rows: &[Value],
    stored_policy: Option<&Value>,
) -> CheckoutPlan {
    let policy_version = stored_policy
        .and_then(|body| body.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let action = match judge_checkout(checkout, rows) {
        CheckoutVerdict::Conflict(conflict) => CheckoutAction::Refuse(conflict),
        CheckoutVerdict::Unchanged => CheckoutAction::Proceed(vec![
            CheckoutOperation::PolicyStep,
            CheckoutOperation::Command,
        ]),
        CheckoutVerdict::Record => CheckoutAction::Proceed(vec![
            CheckoutOperation::RecordSeen(seen_payload(checkout)),
            CheckoutOperation::PolicyStep,
            CheckoutOperation::Command,
        ]),
    };
    CheckoutPlan {
        policy_version,
        action,
    }
}
