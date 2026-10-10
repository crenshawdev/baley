//! `baley install`: Claude Code's wiring at the stable path, from the content
//! `host_artifacts` renders, with ownership evidence in the ledger before
//! any replacement. Gathering is kept apart from judging.
//! One install runs at a time: the command holds an exclusive lock in Baley's
//! own home from reading the latest record to recording the result.
//! Stubs are applied and recorded here. Phase 15 plan 2 adds the registration,
//! hook and settings writes through the same plan and ownership facts.

/// Planned stub writes and their filesystem observations.
pub mod apply;
/// Command arguments, host selection and installation gathering.
pub mod command;
/// Installation ownership facts and the `install.recorded` event.
pub mod event;
/// Ordered artifact writes or a refusal from supplied file observations.
pub mod plan;
/// The install outcome and the owner's next steps, rendered from supplied facts.
pub mod receipt;
/// Recording and reading installation ownership evidence in the ledger.
pub mod record;
/// Claude Code's user settings file read, composed into and judged.
pub mod settings;
/// Stub ownership judged from observed bytes and the latest install record.
pub mod stubs;
/// The latest installation ownership record for each host.
pub mod view;

/// Shared inputs for the install tests: the placements derived from
/// `HOME` `/home/o`, Baley's folders and the sandbox programs found or not.
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::folders::{Environment, Folders};
    use crate::host_artifacts::installed::{self, Installed};
    use crate::host_artifacts::stubs;

    /// The placements for `HOME` `/home/o` with `CLAUDE_CONFIG_DIR` unset.
    pub fn installed() -> Installed {
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let env = Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        };
        installed::resolve(&env, None, &manifest).unwrap()
    }

    /// Baley's data and configuration folders under `/home/o`.
    pub fn folders() -> Folders {
        Folders {
            home: "/home/o/.local/share/crenshawdev/baley".into(),
            config: "/home/o/.config/crenshawdev/baley".into(),
        }
    }
}
