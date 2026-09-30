//! Detection: refreshes the OpenAI, Gemini and DeepSeek catalogs from each
//! provider's model-list endpoint (design 0003 section 6, CFG-R20,
//! CFG-R21). The judging lives in `baley_core::catalog::detection`; this
//! module gathers each listing with its key and records the outcome.

mod lister;

pub use lister::{HttpLister, ModelLister};
