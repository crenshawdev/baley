//! What a set does once its pairs are judged: whether it writes, whether the
//! policy step follows, and where the printed version comes from (design 0003
//! sections 3 and 5, build-2-plan decision 18).

use std::path::Path;

use super::file::value_text;
use super::set::TypedPair;
use crate::policy::FileLayer;

/// Where a set runs, as the binary found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Outside any project. Only a global set reaches this, since a
    /// project-file set is refused there.
    OutsideProject,
    /// In a project whose id is not in this machine's ledger, such as a fresh
    /// clone that has not run `baley init`.
    ProjectNotInLedger,
    /// In a project that is in this machine's ledger.
    LedgeredProject,
}

/// Where the version a set prints comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSource {
    /// The policy step's return, run after the write.
    Step,
    /// The stored `policy` record for the checkout and the command line's
    /// host, 0 when none is stored. A ledgered checkout's version in force
    /// is the stored record's sequence.
    Stored,
    /// 0: outside a project no recorded policy applies (D-05).
    OutsideProject,
    /// 0: the project is not in this machine's ledger, so no recorded policy
    /// applies and `baley init` records it (D-04).
    NotInLedger,
}

/// A set's choice of write, step and version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetOutcome {
    /// Whether the file is written.
    pub write: bool,
    /// Whether the policy step runs after the write.
    pub run_step: bool,
    /// Where the printed version comes from.
    pub version: VersionSource,
}

/// Chooses a set's outcome from whether it changes the file and where it runs
/// (D-03, D-04, D-05, D-15).
///
/// A set that changes nothing writes nothing and runs no step. A change is
/// written everywhere, and the step follows only in a ledgered project,
/// because only a project in the ledger has a chain to append to.
pub fn choose_outcome(changes_file: bool, place: Place) -> SetOutcome {
    let (run_step, version) = match (changes_file, place) {
        (true, Place::LedgeredProject) => (true, VersionSource::Step),
        (false, Place::LedgeredProject) => (false, VersionSource::Stored),
        (_, Place::ProjectNotInLedger) => (false, VersionSource::NotInLedger),
        (_, Place::OutsideProject) => (false, VersionSource::OutsideProject),
    };
    SetOutcome {
        write: changes_file,
        run_step,
        version,
    }
}

/// The lines `set` prints, in order (design 0003 section 5, CONF-03).
///
/// `changed` is the pairs the file did not already hold, empty for a no-op.
/// A project-file set that changed the file names the file, each changed
/// setting with its new value and the host section it went to, that the change
/// applies at the next commit, and the version in force. A global set names
/// each changed setting, the file and the version, and never says commit,
/// since a global edit applies as soon as it is written. A no-op says nothing
/// changed. A version 0 is followed by its meaning, which depends on where the
/// set ran (D-04, D-05).
pub fn render_set(
    layer: FileLayer,
    path: &Path,
    changed: &[TypedPair],
    outcome: &SetOutcome,
    version: u64,
) -> Vec<String> {
    let wrote = format!("wrote {}", path.display());
    let settings = changed.iter().map(|pair| {
        let section = pair
            .host
            .map_or_else(String::new, |host| format!("[host.{}] ", host.name()));
        format!("set {section}{} = {}", pair.name, value_text(&pair.value))
    });
    let mut lines: Vec<String> = Vec::new();
    match (changed.is_empty(), layer) {
        (true, _) => lines.push(format!(
            "nothing changed: {} already holds every value given",
            path.display()
        )),
        (false, FileLayer::Project) => {
            lines.push(wrote);
            lines.extend(settings);
            lines.push("The change applies at the next commit.".to_owned());
        }
        (false, FileLayer::Global) => {
            lines.extend(settings);
            lines.push(wrote);
        }
    }
    lines.push(match (version, outcome.version) {
        (0, VersionSource::OutsideProject) => {
            "policy version 0: no recorded policy applies outside a project".to_owned()
        }
        (0, VersionSource::NotInLedger) => "policy version 0: no recorded policy applies to \
             this project on this machine, and baley init records it"
            .to_owned(),
        (0, VersionSource::Step | VersionSource::Stored) => {
            "policy version 0: no recorded policy applies".to_owned()
        }
        (version, _) => format!("policy version {version}"),
    });
    lines
}
