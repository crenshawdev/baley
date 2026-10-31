//! Work orders' pure half (design 0003 section 6, design 0012): the route
//! record an order carries, the two work-order events, the `work_order`
//! view, the retry judge and the work-order refusal codes.
//!
//! Nothing here reads a store, a file or a clock: the binary supplies every
//! value it judges.

mod event;
mod retry;
mod route;
mod view;

pub use event::{
    IssuedOrder, OrderInput, START_SUBJECT, WORK_ORDER_FAILED, WORK_ORDER_FAILED_VERSION,
    WORK_ORDER_ISSUED, WORK_ORDER_ISSUED_VERSION, agent_name, failed_payload, issued_payload,
    order_stream, phase_subject, register_work_order_events, story_subject, subject_phase,
    subject_story,
};
pub use retry::{
    ANALYZER_ORDER_PENDING, MATERIAL_UNAVAILABLE, NO_SUCH_WORK_ORDER, Retry, RetryRefusal,
    START_ORDER_PENDING, WORK_ORDER_NOT_FAILED, WORK_ORDER_NOT_PENDING, WORK_ORDER_RETRIED,
    WRONG_WORK_ORDER, judge_retry,
};
pub use route::route_record;
pub use view::{
    ANSWERED, FAILED, PENDING, WORK_ORDER_LIVE_INDEX, WORK_ORDER_VIEW, WorkOrderProjector,
    work_order_key, work_order_spec,
};
