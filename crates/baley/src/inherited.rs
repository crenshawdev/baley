//! The inherited engine: the old per-project services and the resident that
//! ran them. Production no longer reaches any of it since the per-session
//! stdio server took over. It stays compiled so its tests keep running, and
//! Build 9 deletes it.

#[allow(dead_code)]
#[path = "recall/mod.rs"]
pub mod recall;

#[allow(dead_code)]
#[path = "derivation_service.rs"]
pub mod derivation_service;

#[allow(dead_code)]
#[path = "evidence_service.rs"]
pub mod evidence_service;
#[cfg(test)]
#[path = "evidence_service_tests.rs"]
mod evidence_service_tests;

#[allow(dead_code)]
#[path = "next_action_service.rs"]
pub mod next_action_service;

#[path = "debug_service.rs"]
pub mod debug_service;
#[path = "landing_service.rs"]
pub mod landing_service;
#[path = "milestone_service.rs"]
pub mod milestone_service;
#[cfg(test)]
#[path = "next_action_service_tests.rs"]
mod next_action_service_tests;
#[path = "progress_service.rs"]
pub mod progress_service;
#[path = "spike_service.rs"]
pub mod spike_service;
#[path = "suggest_service.rs"]
pub mod suggest_service;
#[path = "task_service.rs"]
pub mod task_service;
#[path = "undo_service.rs"]
pub mod undo_service;
#[path = "why_service.rs"]
pub mod why_service;

#[allow(dead_code)]
#[path = "pause_service.rs"]
pub mod pause_service;

#[path = "execution_runner_service.rs"]
pub mod execution_runner_service;
#[allow(dead_code)]
#[path = "execution_service.rs"]
pub mod execution_service;
#[cfg(test)]
#[path = "execution_service_tests.rs"]
mod execution_service_tests;

#[path = "rail_service.rs"]
pub mod rail_service;

#[path = "config_service_binary.rs"]
pub mod config_service;

#[path = "review_service.rs"]
pub mod review_service;

#[path = "context_service.rs"]
pub mod context_service;

#[path = "capture_service.rs"]
pub mod capture_service;
#[path = "plan_service.rs"]
pub mod plan_service;
#[path = "read_service.rs"]
pub mod read_service;
#[path = "verification_service.rs"]
pub mod verification_service;
