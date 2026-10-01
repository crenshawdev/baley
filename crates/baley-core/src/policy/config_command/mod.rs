//! The pure half of `baley config show` and `baley config set` (design 0003
//! section 5): the judges of `set`'s arguments, the rendering of a settings
//! file whole, the choice of write and policy step, `set`'s output text, and
//! `show`'s refusals and report.
//!
//! Nothing here reads a file, the environment, git, the store or a clock: the
//! binary supplies every file, path, accepted model name and version. Every
//! judge takes the schema as a parameter, so a test can hold settings the
//! standard schema does not.

mod file;
mod set;

#[cfg(test)]
mod tests;

pub use file::render_file;
pub use set::{SetRefusal, TypedPair, collapse_repeats, judge_models, judge_pairs, needs_catalog};

/// The code of a name the schema does not hold.
pub const UNKNOWN_SETTING: &str = "unknown-setting";

/// The code of a value its setting's kind refuses.
pub const INVALID_VALUE: &str = "invalid-value";

/// The code of a setting whose scope excludes the file asked for.
pub const WRONG_LAYER: &str = "wrong-layer";

/// The code of a project-file action outside a project.
pub const NOT_A_PROJECT: &str = "not-a-project";
