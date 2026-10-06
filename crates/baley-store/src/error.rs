//! What can go wrong at the port, and what the caller does about it.
//!
//! None of these is a domain outcome. A domain outcome, success or a real
//! refusal, is recorded as `command.completed` and comes back as `Ok`
//! (design 0001, Commands). A failed command records nothing: the caller
//! re-reads and retries with the same request id, waits, or stops.
//! `StoreError` says which errors can follow a change that did commit.

use std::fmt;
use std::path::PathBuf;

use crate::chain::Head;
use crate::claim::{Block, ClaimId};
use crate::command::{Absence, StreamName};
use crate::event::{Hash, ProjectId, RequestId};
use crate::ledger::VerifyReport;
use crate::payload::PayloadReference;
use crate::view::DocKey;

/// Why a port operation did not return its result. A failed command
/// records nothing. An operation that first brought a project's views
/// forward to this binary's (a read, a command, `verify_views`) may have
/// committed that forward rebuild before a later error. `CleanupFailed`
/// follows a rebuild whose generation is already live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// An open claim holds a token needed by this command. Nothing was recorded.
    Blocked(Block),
    /// Something the decision depended on changed between the caller's slow
    /// work and the transaction (EVD-R7). Re-read and retry with the same
    /// request id.
    Stale(StaleInput),
    /// The writer queue or the database lock was not free within the
    /// backstop timeout. Retry later.
    Busy,
    /// The store's compatibility epoch is newer than this binary's. Reads
    /// still work; writes need a binary at `needed_epoch` or later.
    ReadOnly { needed_epoch: u32 },
    /// The port refused the request itself.
    Refused(Refusal),
    /// A projector could not apply an event, so the whole command was
    /// rolled back (EVD-R5).
    Projector {
        view: String,
        seq: u64,
        message: String,
    },
    /// The engine failed: I/O, a full disk, a corrupt file.
    Unavailable(String),
    /// A rebuild made `live_generation` live, and then removing the old
    /// generation failed with `cause`. The flip stands: reads and commands
    /// use the new generation, and the next rebuild removes what is left.
    CleanupFailed {
        project: ProjectId,
        live_generation: u64,
        cause: Box<StoreError>,
    },
    /// A rebuild or view verification that never finished, as after a
    /// crash, left `generation` behind with its marker, or a verification
    /// could not remove its own scratch `generation`. Views cannot be
    /// verified until a rebuild removes it. It is not the live generation:
    /// a marker naming that one is `LiveGenerationProtected`.
    UnfinishedGeneration { project: ProjectId, generation: u64 },
    /// The project's building marker names `generation`, which is its live
    /// generation, as when the marker is damaged. A rebuild refuses rather
    /// than remove it, and a view verification refuses rather than report
    /// it unfinished. Nothing was removed, and the live views stand; the
    /// project's rebuilds and view verifications refuse the same way until
    /// the marker is repaired.
    LiveGenerationProtected { project: ProjectId, generation: u64 },
    /// A store opened for bounded access, as the guard's is, found the
    /// project's live views behind this binary's. It answers this instead
    /// of rebuilding them inline or waiting for the maintenance lock, and
    /// nothing was recorded. A normal open, or `baley rebuild <project>`,
    /// brings them forward.
    NeedsRebuild { project: ProjectId },
}

/// The input that moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleInput {
    /// The project head changed after verification.
    Head {
        seen: Option<Head>,
        now: Option<Head>,
    },
    /// A view document is not at the sequence the caller saw. `None` on
    /// either side means absent.
    Document {
        view: String,
        key: DocKey,
        seen: Option<u64>,
        now: Option<u64>,
    },
    /// Something the caller saw absent now exists.
    Absence(Absence),
    /// A git fact of a checkout moved.
    Git {
        checkout: String,
        fact: GitFact,
        seen: String,
        now: String,
    },
    /// The claim is no longer open to its owner because another process
    /// completed or reconciled it, or because it awaits the owner.
    Claim(ClaimId),
    /// A live claim cannot be reconciled yet.
    ClaimActive(ClaimId),
    /// `expect` named a stream version the stream has moved past.
    StreamVersion {
        stream: StreamName,
        expected: u64,
        actual: u64,
    },
}

/// Why the port refused a request outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The home or a store file is unsafe. Contains every known fault, never empty.
    UnsafeHome(Vec<HomeFault>),
    /// Doctor has no remote check for this project.
    MissingAnchorCheck(ProjectId),
    /// An export target already exists, including a symbolic link.
    TargetExists(PathBuf),
    /// The exported copy failed its own verification.
    ExportUnverified {
        project: ProjectId,
        report: Box<VerifyReport>,
    },
    /// No such project in this store.
    UnknownProject(ProjectId),
    /// The project exists already.
    ProjectExists(ProjectId),
    /// This binary must not write to the project: it holds an event type or
    /// version this binary cannot read, or its live views were built by a
    /// newer projector or view set, which refuses this binary's reads of
    /// those views as well (EVD-R19). History and payload reads still work.
    ProjectReadOnly { project: ProjectId, reason: String },
    /// The request id was used before for this project and command kind
    /// with a different digest (EVD-R6).
    RequestDigestMismatch { request_id: RequestId },
    /// No view of that name is registered.
    UnknownView(String),
    /// The view declares no index of that name; `find` never scans.
    UndeclaredIndex { view: String, index: String },
    /// A key or index value does not fit the declared fields. At open, also
    /// a view declared badly, or a view spec or view set changed under a
    /// version already recorded: `view` then names the view whose spec
    /// changed, or one that is in only one of the two sets.
    MalformedKey { view: String, reason: String },
    /// No payload with that hash is stored.
    UnknownPayload(Hash),
    /// Bytes whose hash was reduced or purged cannot be stored again: the
    /// new reference would point at a body that is gone, and storing it anew
    /// would bring the purged content back for the old references.
    PayloadTombstoned(Hash),
    /// The reference is outside the command's project, absent, not `output`,
    /// or released; or its body is absent or at most 128 KiB.
    NotReducible(PayloadReference),
    /// The project has no reference to this hash, or has released all of
    /// them and every excerpt its own reductions attached.
    NothingToPurge(Hash),
    /// A cursor not issued for this query, project, generation and view
    /// version, or not a cursor at all.
    InvalidCursor,
    /// The event cannot be sealed: see the reason.
    InvalidEvent(String),
    /// A decision appended an event type or version this binary cannot
    /// read, so it would fence its own project.
    UnreadableType { type_name: String, version: u32 },
    /// A decision stored a payload that no event of its command attaches.
    /// The body would sit outside the chain, where no reference names it
    /// and no purge reaches it.
    UnattachedPayload(Hash),
    /// No claim with that id was ever taken in the project.
    UnknownClaim(ClaimId),
    /// The owner supplied does not hold the claim.
    NotClaimOwner(ClaimId),
    /// The renewal time is malformed or earlier than the claim's floor.
    LeaseTime {
        /// The claim being renewed.
        claim: ClaimId,
        /// The supplied renewal time.
        at: String,
        /// The earliest permitted renewal time.
        floor: String,
    },
    /// The claim is held for an owner resolution.
    AwaitingOwner(ClaimId),
    /// A record step did not carry the scope it claimed under.
    ScopeMismatch {
        /// The claim being completed.
        claim: ClaimId,
        /// The record step's scope.
        supplied: Vec<String>,
        /// The scope taken at claim time.
        claimed: Vec<String>,
    },
    /// An owner resolution must be attributed to the owner.
    NotOwner,
    /// The file was created by a build with another epoch-1 schema.
    SchemaChanged {
        /// The ledger created with another schema text.
        path: PathBuf,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(Refusal::UnsafeHome(faults)) => {
                write!(f, "refused: {UNSAFE_HOME}: ")?;
                for (index, fault) in faults.iter().enumerate() {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{fault}")?;
                }
                Ok(())
            }
            Self::Blocked(block) => {
                write!(f, "blocked by claim {:?} ({:?})", block.claim, block.state)
            }
            Self::Stale(input) => write!(f, "stale input, re-read and retry: {input:?}"),
            Self::Busy => f.write_str("store busy, retry"),
            Self::ReadOnly { needed_epoch } => {
                write!(
                    f,
                    "store is read-only for this binary; epoch {needed_epoch} is needed"
                )
            }
            Self::Refused(Refusal::SchemaChanged { path }) => write!(
                f,
                "refused: {} was written by a build with another epoch-1 schema; delete this pre-release ledger and create a fresh one",
                path.display()
            ),
            Self::Refused(Refusal::MissingAnchorCheck(project)) => write!(
                f,
                "refused: no anchor check was supplied for project {}",
                project.0
            ),
            Self::Refused(Refusal::TargetExists(path)) => write!(
                f,
                "refused: export target already exists: {}",
                path.display()
            ),
            Self::Refused(Refusal::ExportUnverified { project, report }) => write!(
                f,
                "refused: export of project {} did not verify: {report:?}",
                project.0
            ),
            Self::Refused(refusal) => write!(f, "refused: {refusal:?}"),
            Self::Projector { view, seq, message } => {
                write!(f, "projector for {view} failed at event {seq}: {message}")
            }
            Self::Unavailable(reason) => write!(f, "store unavailable: {reason}"),
            Self::CleanupFailed {
                project,
                live_generation,
                cause,
            } => write!(
                f,
                "project {} now reads generation {live_generation}; removing the old generation failed and is left for the next rebuild: {cause}",
                project.0
            ),
            Self::UnfinishedGeneration {
                project,
                generation,
            } => write!(
                f,
                "project {} holds generation {generation} of a rebuild or view verification that never finished; rebuild the project first",
                project.0
            ),
            Self::LiveGenerationProtected {
                project,
                generation,
            } => write!(
                f,
                "project {}'s building marker names generation {generation}, which is live; nothing was removed",
                project.0
            ),
            Self::NeedsRebuild { project } => write!(
                f,
                "project {}'s views need a rebuild by this binary; run baley rebuild {}",
                project.0, project.0
            ),
        }
    }
}

impl std::error::Error for StoreError {}

/// The stable code of an unsafe-home refusal.
pub const UNSAFE_HOME: &str = "unsafe-home";

/// One unsafe path and the repair its caller can make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeFault {
    /// The real path inspected by the adapter.
    pub path: PathBuf,
    /// Whether this is the ledger home or a store file.
    pub target: FaultTarget,
    /// What prevents opening it safely.
    pub problem: HomeProblem,
}

/// The kind of path whose safety is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultTarget {
    /// The folder holding the store.
    Home,
    /// A database, log, shared-memory or lock file.
    File,
}

/// A fault found before opening any store file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeProblem {
    /// The final path component is a symbolic link.
    Link,
    /// The home is not a folder.
    NotAFolder,
    /// A store file is not a regular file.
    NotAFile,
    /// The path belongs to another user.
    Owner {
        /// The path's observed owner.
        owner: u32,
        /// The process's effective user id.
        user: u32,
    },
    /// The permission bits grant more than the allowed access.
    Mode {
        /// Observed mode, including special bits.
        mode: u32,
        /// The maximum permission bits for this path.
        allowed: u32,
    },
}

impl fmt::Display for HomeFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ", self.path.display())?;
        match self.problem {
            HomeProblem::Link => write!(
                f,
                "is a symbolic link (fix: replace the link with the real {})",
                match self.target {
                    FaultTarget::Home => "folder",
                    FaultTarget::File => "file",
                }
            ),
            HomeProblem::NotAFolder => f.write_str("is not a folder (fix: remove or rename it)"),
            HomeProblem::NotAFile => {
                f.write_str("is not a regular file (fix: remove or rename it)")
            }
            HomeProblem::Owner { owner, user } => write!(
                f,
                "is owned by user id {owner}, not by this user ({user}) (fix: {})",
                fix(&format!("chown {user}"), &self.path)
            ),
            HomeProblem::Mode { mode, allowed } => write!(
                f,
                "has mode {mode:04o}, which allows more than {allowed:04o} (fix: {})",
                fix(&format!("chmod {allowed:o}"), &self.path)
            ),
        }
    }
}

/// The fix as a command, or a plain instruction when the path is not UTF-8:
/// no one quoting of raw bytes works in bash, zsh, dash and fish alike.
fn fix(command: &str, path: &std::path::Path) -> String {
    match path.to_str() {
        Some(text)
            if !text.is_empty()
                && text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/._-+@%:,=".contains(&b)) =>
        {
            format!("{command} {text}")
        }
        Some(text) => format!("{command} '{}'", text.replace('\'', "'\\''")),
        None => format!(
            "{command} on this path, whose name is not UTF-8 and cannot be written as a command"
        ),
    }
}

/// Which git fact moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFact {
    Head,
    Index,
}

#[cfg(test)]
mod home_tests {
    use super::*;

    #[test]
    fn mode_fix_quotes_application_support_as_one_argument() {
        let fault = HomeFault {
            path: "/Users/o/Library/Application Support/crenshawdev/baley".into(),
            target: FaultTarget::Home,
            problem: HomeProblem::Mode {
                mode: 0o750,
                allowed: 0o700,
            },
        };
        assert_eq!(
            fault.to_string(),
            "/Users/o/Library/Application Support/crenshawdev/baley has mode 0750, which allows more than 0700 (fix: chmod 700 '/Users/o/Library/Application Support/crenshawdev/baley')"
        );
    }
    #[test]
    fn repair_path_quotes_do_not_end_the_shell_argument() {
        let fault = HomeFault {
            path: "/h/o'ne/baley.db".into(),
            target: FaultTarget::File,
            problem: HomeProblem::Owner {
                owner: 0,
                user: 1000,
            },
        };
        assert_eq!(
            fault.to_string(),
            "/h/o'ne/baley.db is owned by user id 0, not by this user (1000) (fix: chown 1000 '/h/o'\\''ne/baley.db')"
        );
    }
    #[test]
    fn non_utf8_repair_path_is_not_given_as_a_wrong_command() {
        use std::os::unix::ffi::OsStrExt;
        let fault = HomeFault {
            path: std::ffi::OsStr::from_bytes(b"/h/\xffhome").into(),
            target: FaultTarget::Home,
            problem: HomeProblem::Mode {
                mode: 0o755,
                allowed: 0o700,
            },
        };
        assert!(fault.to_string().ends_with(
            "(fix: chmod 700 on this path, whose name is not UTF-8 and cannot be written as a command)"
        ));
    }
    #[test]
    fn unsafe_home_display_does_not_drop_faults_or_use_debug() {
        let faults = vec![
            HomeFault {
                path: "/h".into(),
                target: FaultTarget::Home,
                problem: HomeProblem::Link,
            },
            HomeFault {
                path: "/h/baley.db".into(),
                target: FaultTarget::File,
                problem: HomeProblem::NotAFile,
            },
        ];
        assert_eq!(
            StoreError::Refused(Refusal::UnsafeHome(faults)).to_string(),
            "refused: unsafe-home: /h is a symbolic link (fix: replace the link with the real folder); /h/baley.db is not a regular file (fix: remove or rename it)"
        );
    }

    #[test]
    fn the_needs_rebuild_text_leaves_out_the_project_or_the_remedy() {
        let project = ProjectId("6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f".into());
        assert_eq!(
            StoreError::NeedsRebuild { project }.to_string(),
            "project 6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f's views need a rebuild by this binary; run baley rebuild 6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f"
        );
    }
}
