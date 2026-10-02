//! Checkout admission's pure half (design 0001 Project identity and policy,
//! EVD-R17, ADR 0004): the `checkout.seen` event, the `checkout` view that
//! keeps the latest one per path, the fork judgement and the checkout
//! admission plan. "Admission" alone means phase admission in this project,
//! so the word here is always "checkout admission".
//!
//! Nothing here reads git, a file, the store or a clock: the binary supplies
//! every checkout, row and stored document it judges.

mod event;
mod view;

#[cfg(test)]
mod tests;

pub use event::{
    CHECKOUT_SEEN, CHECKOUT_SEEN_VERSION, Checkout, register_checkout_events, seen_payload,
};
pub use view::{CHECKOUT_VIEW, CheckoutProjector, checkout_key, checkout_spec};
