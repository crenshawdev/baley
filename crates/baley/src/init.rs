//! `baley init`: ties a repository to a ledger project (design 0001, EVD-R17).
//! The judges here take plain values; the command gathers them.
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use baley_core::checkout::Checkout;
use baley_core::policy::recorded::{RecordedPolicy, recorded_policy};
use baley_core::policy::{
    EffectivePolicy, ProjectIdentity, SettingsFile, Unavailable, read_project, render_project,
};
use baley_core::{PROJECT_INITIALIZED, PROJECT_INITIALIZED_VERSION};
use baley_store::{
    Actor, Admin, Command, CommandKind, Decision, EVENT_PAGE_BOUND, EventMatch, Ledger, NewEvent,
    Observed, OutcomeKind, PageRequest, ProjectId, Refusal, RequestId, StoreError, StreamName,
    request_digest,
};
use clap::Args;
use serde_json::json;

use crate::checkout::{self, EntryError};
use crate::detection::{Trigger, detect_blocking};
use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::folders::{Environment, Folders, Platform};
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::ledger::open;
use crate::policy_step::{self, Reads};
use crate::{keys, replace, settings};

/// The working directory is not inside a git repository.
pub const NOT_A_REPOSITORY: &str = "not-a-repository";
/// The working directory is inside a repository but not at its root.
pub const NOT_REPOSITORY_ROOT: &str = "not-repository-root";
/// No project file names the project and the root has no usable folder name.
pub const PROJECT_NAME_REQUIRED: &str = "project-name-required";

/// Why `baley init` refused before writing anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitRefusal {
    /// No ancestor of the working directory holds `.git`.
    NotARepository,
    /// The working directory is below the repository root.
    NotRepositoryRoot {
        /// The repository root.
        root: PathBuf,
    },
    /// No `--name` was given and the root's folder name is missing or not UTF-8.
    NameRequired {
        /// The repository root.
        root: PathBuf,
    },
}

impl InitRefusal {
    /// The stable refusal code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotARepository => NOT_A_REPOSITORY,
            Self::NotRepositoryRoot { .. } => NOT_REPOSITORY_ROOT,
            Self::NameRequired { .. } => PROJECT_NAME_REQUIRED,
        }
    }
}

impl fmt::Display for InitRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.code())?;
        match self {
            Self::NotARepository => write!(
                f,
                "the working directory is not inside a git repository \
                 (fix: run baley init at the root of a git repository)"
            ),
            Self::NotRepositoryRoot { root } => write!(
                f,
                "baley init runs at the repository root, {} (fix: run baley init there)",
                root.display()
            ),
            Self::NameRequired { root } => write!(
                f,
                "the repository root {} has no folder name to use as the project's name \
                 (fix: run baley init --name <name>)",
                root.display()
            ),
        }
    }
}

/// The repository root when the working directory is that root. `cwd` is
/// canonical, as the root from `discovery::ancestors` is.
pub fn locate(discovery: &Discovery, cwd: &Path) -> Result<PathBuf, InitRefusal> {
    match discovery {
        Discovery::Outside => Err(InitRefusal::NotARepository),
        Discovery::Managed { root, .. } | Discovery::Unmanaged { root }
            if root.as_path() != cwd =>
        {
            Err(InitRefusal::NotRepositoryRoot { root: root.clone() })
        }
        Discovery::Managed { root, .. } | Discovery::Unmanaged { root } => Ok(root.clone()),
    }
}

/// The project's name, and a note for the owner when `--name` was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    /// The name the file, the project and `project.initialized` carry.
    pub name: String,
    /// Says `--name` was not applied because the file already names the project.
    pub note: Option<String>,
}

/// Chooses the project's name. An existing file's name always wins: init
/// never renames a project, and `--new-id` keeps the name too. Otherwise `--name` as given, empty included,
/// then the root's folder name. A name nobody chose is never made up: no
/// lossy conversion and no empty default.
pub fn name(
    root: &Path,
    given: Option<&str>,
    file: Option<&ProjectIdentity>,
) -> Result<Naming, InitRefusal> {
    if let Some(file) = file {
        let note = given.filter(|given| *given != file.name).map(|_| {
            // Debug quoting keeps control bytes in a committed name off the terminal.
            format!(
                "--name was not applied: {PROJECT_FILE} already names the project {:?}",
                file.name
            )
        });
        return Ok(Naming {
            name: file.name.clone(),
            note,
        });
    }
    let name = match given {
        Some(given) => given,
        None => root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| InitRefusal::NameRequired { root: root.into() })?,
    };
    Ok(Naming {
        name: name.into(),
        note: None,
    })
}

/// A fresh project id: a lower-case hyphenated UUID version 4, the one form
/// `read_project` accepts.
pub fn fresh_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// What a run does to `baley.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    /// There is no file: write a new one with the fresh id.
    Write,
    /// `--new-id` over a file: replace it with the fresh id, keeping its name
    /// and every other setting.
    Replace,
    /// Leave the file as it is.
    Keep,
}

/// The project a run acts on, and what it does to the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acting {
    /// The id and name the project in the ledger carries.
    pub identity: ProjectIdentity,
    /// What happens to `baley.toml`.
    pub file: FileAction,
}

/// Decides which project this run acts on from the file's identity,
/// `--new-id`, the name chosen for a new file and a fresh id. With no file, or
/// with `--new-id` over one, the run acts on the fresh id, and the file's own
/// name is kept in the second case. The file's old id is never acted on, so
/// `--new-id` leaves that project as it is.
pub fn acting(
    file: Option<&ProjectIdentity>,
    new_id: bool,
    chosen_name: &str,
    fresh_id: &str,
) -> Acting {
    match file {
        None => Acting {
            identity: ProjectIdentity {
                id: fresh_id.into(),
                name: chosen_name.into(),
            },
            file: FileAction::Write,
        },
        Some(file) if new_id => Acting {
            identity: ProjectIdentity {
                id: fresh_id.into(),
                name: file.name.clone(),
            },
            file: FileAction::Replace,
        },
        Some(file) => Acting {
            identity: file.clone(),
            file: FileAction::Keep,
        },
    }
}

/// The bytes the run writes to `baley.toml`, `None` when it keeps the file.
/// A replaced file is rendered from the working-tree file as read, so every
/// other table, unknown keys and the other `[project]` keys stay (comments and
/// key order do not, ADR 0027).
pub fn file_bytes(
    acting: &Acting,
    working: Option<&SettingsFile>,
) -> Result<Option<Vec<u8>>, Unavailable> {
    let base = match acting.file {
        FileAction::Keep => return Ok(None),
        FileAction::Write => None,
        FileAction::Replace => working,
    };
    render_project(&acting.identity.id, &acting.identity.name, base).map(Some)
}

/// Writes `baley.toml` at the root and returns its path. `digest` is the
/// working-tree file's as read, or none for a new file, so `replace` refuses a
/// file that appeared or changed since it was read, and a link. A new file
/// gets the umask's mode.
pub fn write_project_file(
    root: &Path,
    bytes: &[u8],
    digest: Option<&str>,
) -> Result<PathBuf, replace::Failure> {
    let path = root.join(PROJECT_FILE);
    replace::replace(&path, bytes, digest)?;
    Ok(path)
}

/// The command kind `baley init` records its request under.
pub const INIT_COMMAND: &str = "project.init";
/// The stream `project.initialized` goes to.
pub const PROJECT_STREAM: &str = "project";

/// Creates the project in the ledger. True when this run created it, false
/// when it was already there, as after an interrupted init or a second one.
pub fn create_project(
    store: &impl Admin,
    identity: &ProjectIdentity,
    at: &str,
) -> Result<bool, StoreError> {
    let project = ProjectId(identity.id.clone());
    match store.create_project(&project, &identity.name, at) {
        Ok(()) => Ok(true),
        Err(StoreError::Refused(Refusal::ProjectExists(_))) => Ok(false),
        Err(error) => Err(error),
    }
}

/// The command `baley init` records its request under: Owner's, at policy
/// version 0, with the project's name in the digest.
fn init_command(
    identity: &ProjectIdentity,
    request_id: RequestId,
    at: &str,
) -> Result<Command, StoreError> {
    let actor = Actor::Owner;
    let digest = request_digest(&json!({
        "kind": INIT_COMMAND,
        "project": identity.id,
        "actor": actor.as_str(),
        "policy_version": 0,
        "name": identity.name,
        "scope": [],
    }))
    .map_err(|error| StoreError::Refused(Refusal::InvalidEvent(error.to_string())))?;
    Ok(Command {
        project: ProjectId(identity.id.clone()),
        kind: CommandKind(INIT_COMMAND.into()),
        request_id,
        digest,
        scope: vec![],
        policy_version: 0,
        recorded_at: at.into(),
        actor,
        caller: None,
    })
}

/// The `project.initialized` event for `identity`.
fn initialized_event(identity: &ProjectIdentity) -> NewEvent {
    NewEvent {
        stream: StreamName(PROJECT_STREAM.into()),
        type_name: PROJECT_INITIALIZED.into(),
        type_version: PROJECT_INITIALIZED_VERSION,
        git: None,
        payload: json!({ "name": identity.name }),
        attachments: vec![],
    }
}

/// Records `project.initialized` once. True when this run appended it. The
/// check runs inside the transaction, so a racing second init records only
/// its `command.completed`.
///
/// This is the record for a chain that already holds events, after checkout
/// admission. [`record_initialized_on_empty`] is the one before it.
pub fn record_initialized(
    store: &impl Ledger,
    identity: &ProjectIdentity,
    request_id: RequestId,
    at: &str,
) -> Result<bool, StoreError> {
    let command = init_command(identity, request_id, at)?;
    let initialized = EventMatch {
        type_name: PROJECT_INITIALIZED.into(),
        stream: Some(StreamName(PROJECT_STREAM.into())),
        fields: BTreeMap::new(),
    };
    let mut appended = false;
    store.transact(&command, &mut |tx| {
        appended = !tx.event_exists(&initialized)?;
        if appended {
            tx.append(initialized_event(identity))?;
        }
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({ "recorded": appended }),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    })?;
    Ok(appended)
}

/// What [`record_initialized_on_empty`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyChain {
    /// The chain was empty and `project.initialized` is now its first event.
    Recorded,
    /// The chain already held events, so nothing was recorded.
    FoundEvents,
}

/// Records `project.initialized` as the first event of an empty chain, before
/// checkout admission. The emptiness is read with `Transaction::head` in the
/// transaction that appends, so a checkout admitted since init looked is seen.
///
/// A chain holding events records nothing at all, not even `command.completed`:
/// the decision returns an error, as checkout admission's does, and the
/// finding is kept in a variable this function owns. The caller reads the
/// finding, never that error.
pub fn record_initialized_on_empty(
    store: &impl Ledger,
    identity: &ProjectIdentity,
    request_id: RequestId,
    at: &str,
) -> Result<EmptyChain, StoreError> {
    let command = init_command(identity, request_id, at)?;
    let mut found_events = false;
    let result = store.transact(&command, &mut |tx| {
        if tx.head()?.is_some() {
            found_events = true;
            return Err(StoreError::Refused(Refusal::InvalidEvent(
                "the project's chain already holds events".into(),
            )));
        }
        tx.append(initialized_event(identity))?;
        Ok(Decision {
            kind: OutcomeKind::Done,
            answer: json!({ "recorded": true }),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        })
    });
    if found_events {
        return Ok(EmptyChain::FoundEvents);
    }
    result?;
    Ok(EmptyChain::Recorded)
}

/// One action of `baley init`, in the order the plan lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Write a new `baley.toml` with a fresh id.
    WriteFile,
    /// Replace `baley.toml` with one holding a fresh id.
    ReplaceFile,
    /// Create the project in the ledger.
    CreateProject,
    /// Record `project.initialized` as the first event of an empty chain, which
    /// comes before checkout admission.
    RecordInitializedOnEmpty,
    /// Run checkout admission for this checkout.
    AdmitCheckout,
    /// Record a missing `project.initialized` on a chain that already holds
    /// events, which comes after checkout admission.
    RecordInitialized,
}

/// What the ledger holds for the file's project, read outside any transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LedgerObservation {
    /// The project is in the ledger.
    pub project: bool,
    /// Its `project` stream holds a `project.initialized`.
    pub initialized: bool,
    /// Its chain holds at least one event of any kind. False for a project
    /// absent from the ledger.
    pub events: bool,
}

/// Decides which actions this run takes, from what was observed before any
/// of them.
///
/// There are two orders, and they turn on whether the chain holds events. On
/// an empty chain `project.initialized` is recorded first and checkout
/// admission follows. On a chain that already holds events, checkout
/// admission comes first and a missing `project.initialized` follows, so a
/// fork's plain init is refused before it appends anything to the chain it
/// does not own. A finished init plans checkout admission alone, which
/// appends nothing while the checkout is unchanged. `--new-id` over a file
/// replaces its id, then takes the empty-chain order: the fresh project has
/// no chain yet.
///
/// The policy step, like detection, is not a step of the plan: on every run
/// that passes the refusals it runs after the steps and before detection,
/// and it appends nothing while the policy is unchanged.
pub fn plan(file: FileAction, ledger: LedgerObservation) -> Vec<Step> {
    match (file, ledger) {
        // A new id has nothing to look up, and its chain is empty.
        (FileAction::Write, _) => vec![
            Step::WriteFile,
            Step::CreateProject,
            Step::RecordInitializedOnEmpty,
            Step::AdmitCheckout,
        ],
        // The same whatever the file's old id holds in the ledger.
        (FileAction::Replace, _) => vec![
            Step::ReplaceFile,
            Step::CreateProject,
            Step::RecordInitializedOnEmpty,
            Step::AdmitCheckout,
        ],
        (FileAction::Keep, LedgerObservation { project: false, .. }) => vec![
            Step::CreateProject,
            Step::RecordInitializedOnEmpty,
            Step::AdmitCheckout,
        ],
        (FileAction::Keep, LedgerObservation { events: false, .. }) => {
            vec![Step::RecordInitializedOnEmpty, Step::AdmitCheckout]
        }
        (
            FileAction::Keep,
            LedgerObservation {
                initialized: false, ..
            },
        ) => vec![Step::AdmitCheckout, Step::RecordInitialized],
        (FileAction::Keep, _) => vec![Step::AdmitCheckout],
    }
}

/// The project file as the plan sees it: absent only when there is no file.
/// A file that cannot be read, is not TOML or holds a bad id is refused,
/// never planned over as absent.
pub fn observe_file(
    read: Result<Option<SettingsFile>, Unavailable>,
) -> Result<Option<ProjectIdentity>, Unavailable> {
    read?.as_ref().map(read_project).transpose()
}

/// What init's steps need, judged before anything is written or opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The working-tree file's project, `None` when there is no file.
    pub existing: Option<ProjectIdentity>,
    /// The project's name.
    pub naming: Naming,
    /// The checkout and the policy the step records.
    pub recorded: RecordedPolicy,
    /// The policy `recorded` was built from, which names the remote the
    /// checkout's facts are gathered from.
    pub policy: EffectivePolicy,
}

/// Judges init's observations before the store opens or a file is written:
/// the working-tree file's read, `--name`, the policy from `reads`, then the
/// root as the recorded policy holds it. Returns the first refusal's text.
pub fn prepare(
    root: &Path,
    given: Option<&str>,
    file: Result<Option<SettingsFile>, Unavailable>,
    reads: &Reads,
) -> Result<Prepared, String> {
    let existing = observe_file(file).map_err(|e| e.to_string())?;
    let naming = name(root, given, existing.as_ref()).map_err(|e| e.to_string())?;
    let policy = policy_step::build(reads, None).map_err(|e| e.to_string())?;
    let recorded = recorded_policy(root, &policy).map_err(|e| e.to_string())?;
    Ok(Prepared {
        existing,
        naming,
        recorded,
        policy,
    })
}

/// Reads whether the project is in the ledger and, when it is, whether its
/// chain holds any event and whether its `project` stream holds a
/// `project.initialized`, following every page.
pub fn observe_ledger(
    store: &(impl Admin + Ledger),
    id: &str,
) -> Result<LedgerObservation, StoreError> {
    let project = ProjectId(id.into());
    if !store.projects()?.iter().any(|(known, _)| *known == project) {
        return Ok(LedgerObservation::default());
    }
    let events = store.head(&project)?.is_some();
    let stream = StreamName(PROJECT_STREAM.into());
    let mut after = None;
    loop {
        let limit = EVENT_PAGE_BOUND;
        let page = store.stream(&project, &stream, 1, PageRequest { limit, after })?;
        let initialized = page
            .items
            .iter()
            .any(|event| event.type_name == PROJECT_INITIALIZED);
        if initialized || page.next.is_none() {
            return Ok(LedgerObservation {
                project: true,
                initialized,
                events,
            });
        }
        after = page.next;
    }
}

/// Arguments for `baley init`.
#[derive(Args, Debug, Clone)]
pub struct InitArgs {
    /// The project's name for a new baley.toml. Defaults to the repository
    /// folder's name. An existing file's name is never changed.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// Writes a new project id into baley.toml, keeping every other setting,
    /// creates that project, and leaves the old project as it is. Use it in a
    /// fork or a copy that must not share the original's project.
    #[arg(long)]
    pub new_id: bool,
}

/// Runs `baley init` in the working directory and prints what it did.
pub fn run(args: InitArgs) -> ExitCode {
    let started_at = SystemClock::now();
    let result = initialize(&args, &started_at).unwrap_or_else(|e| e);
    ExitCode::from(display::emit(
        &result,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
}

fn initialize(args: &InitArgs, started_at: &str) -> Result<Render, Render> {
    let refuse = |refusal: &dyn fmt::Display| Render::refusal(refusal.to_string());
    let unavailable =
        |error: io::Error| display::store_error(&StoreError::Unavailable(error.to_string()), None);
    let folders =
        Folders::resolve(Platform::current(), &Environment::read()).map_err(|e| refuse(&e))?;
    let cwd = std::env::current_dir().map_err(unavailable)?;
    let ancestors = discovery::ancestors(&cwd).map_err(unavailable)?;
    let canonical = ancestors
        .first()
        .map_or(cwd.as_path(), |a| a.path.as_path());
    let root = locate(&discovery::discover(&ancestors), canonical).map_err(|e| refuse(&e))?;
    let path = root.join(PROJECT_FILE);
    let file = settings::read(&path);
    let working = file.as_ref().ok().and_then(Option::as_ref).cloned();
    let reads = policy_step::gather(
        &folders.config,
        &root,
        working.as_ref(),
        &mut crate::process::System,
    );
    let Prepared {
        existing,
        naming,
        recorded,
        policy,
    } = prepare(&root, args.name.as_deref(), file, &reads).map_err(Render::refusal)?;
    // Gathered before anything is written or opened, so a remote the policy
    // names and the checkout lacks, or a git failure, refuses like init's
    // other refusals.
    let facts =
        checkout::gather(&root, &policy, &mut crate::process::System).map_err(Render::refusal)?;
    let checkout = Checkout {
        path: recorded.checkout.clone(),
        root_commit: facts.root_commit,
        remote_url: facts.remote_url,
    };
    let action = acting(existing.as_ref(), args.new_id, &naming.name, &fresh_id());
    let mut new_bytes = file_bytes(&action, working.as_ref()).map_err(|e| refuse(&e))?;

    // Nothing above opens the store, so a refusal leaves no ledger home behind.
    let store = open::store(&folders.home, started_at, open::options())
        .map_err(|e| display::store_error(&e, None))?;
    // A new id is in no ledger, and a replaced id is never looked up, so only
    // a file that is kept has its project observed.
    let observed = match action.file {
        FileAction::Keep => observe_ledger(&store, &action.identity.id)
            .map_err(|e| display::store_error(&e, Some(&action.identity.id)))?,
        FileAction::Write | FileAction::Replace => LedgerObservation::default(),
    };
    let mut steps: VecDeque<Step> = plan(action.file, observed).into();
    let mut lines: Vec<String> = naming.note.into_iter().collect();
    let mut acted = false;
    let identity = action.identity;
    let write_failure = |failure: replace::Failure| match failure {
        replace::Failure::Refused(conflict) => refuse(&conflict),
        other => Render {
            lines: vec![other.to_string()],
            code: 3,
            error: true,
        },
    };
    let failed = |e: StoreError| display::store_error(&e, Some(&identity.id));
    while let Some(step) = steps.pop_front() {
        match step {
            Step::WriteFile | Step::ReplaceFile => {
                let bytes = new_bytes
                    .take()
                    .ok_or_else(|| refuse(&"init planned a second write"))?;
                // A replaced file is checked against the working-tree digest.
                let digest = working.as_ref().map(|file| file.digest.as_str());
                if step == Step::ReplaceFile {
                    write_project_file(&root, &bytes, digest).map_err(write_failure)?;
                    lines.push(format!(
                        "wrote {} with new project id {} (commit this file)",
                        path.display(),
                        identity.id
                    ));
                } else {
                    write_project_file(&root, &bytes, None).map_err(write_failure)?;
                    lines.push(format!("wrote {} (commit this file)", path.display()));
                }
                acted = true;
            }
            Step::CreateProject => {
                if create_project(&store, &identity, started_at).map_err(failed)? {
                    lines.push(format!(
                        "created project {} in the ledger at {}",
                        identity.id,
                        folders.home.display()
                    ));
                    acted = true;
                }
            }
            Step::RecordInitializedOnEmpty => {
                match record_initialized_on_empty(&store, &identity, new_request_id(), started_at)
                    .map_err(failed)?
                {
                    EmptyChain::Recorded => {
                        lines.push(initialized_line(&identity));
                        acted = true;
                    }
                    // A checkout got there first: go on in the other order.
                    // Checkout admission is the plan's last step here, so the
                    // missing record queues behind it.
                    EmptyChain::FoundEvents => steps.push_back(Step::RecordInitialized),
                }
            }
            Step::AdmitCheckout => {
                let project = ProjectId(identity.id.clone());
                checkout::admit(
                    &store,
                    &project,
                    &checkout,
                    new_request_id(),
                    started_at,
                    None,
                )
                .map_err(|e| display::entry_error(&EntryError::Admit(e), &identity.id))?;
            }
            Step::RecordInitialized => {
                if record_initialized(&store, &identity, new_request_id(), started_at)
                    .map_err(failed)?
                {
                    lines.push(initialized_line(&identity));
                    acted = true;
                }
            }
        }
    }
    // Outside the plan, so a rerun finishes a crash between
    // `project.initialized` and the first `policy.effective`. It prints
    // nothing, and init's own events keep policy version 0.
    let project = ProjectId(identity.id.clone());
    policy_step::step(
        &store,
        &project,
        &recorded,
        new_request_id(),
        started_at,
        None,
    )
    .map_err(failed)?;
    if !acted {
        lines.push(format!(
            "already initialized: {} names project {}, and the ledger at {} holds it",
            path.display(),
            identity.id,
            folders.home.display()
        ));
    }
    // Silent whatever it finds: its outcome never changes what init prints
    // or returns. The providers run side by side, so a dead network costs
    // one request timeout.
    let keys = keys::load(&folders.config);
    let _ = detect_blocking(&store, keys.as_ref(), &Trigger::Automatic, started_at);
    Ok(Render {
        lines,
        code: 0,
        error: false,
    })
}

/// The line printed when this run appended `project.initialized`.
fn initialized_line(identity: &ProjectIdentity) -> String {
    format!("recorded project.initialized for project {}", identity.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    fn identity(name: &str) -> ProjectIdentity {
        ProjectIdentity {
            id: "0b5c1f6e-2a7d-4c3e-9f10-5a6b7c8d9e0f".into(),
            name: name.into(),
        }
    }

    #[test]
    fn outside_a_repository_is_refused_not_taken_as_a_root() {
        let refusal = locate(&Discovery::Outside, Path::new("/w")).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "not-a-repository: the working directory is not inside a git repository \
             (fix: run baley init at the root of a git repository)"
        );
    }

    #[test]
    fn a_subdirectory_is_refused_and_names_the_root() {
        let discovery = Discovery::Unmanaged { root: "/r".into() };
        let refusal = locate(&discovery, Path::new("/r/sub")).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "not-repository-root: baley init runs at the repository root, /r \
             (fix: run baley init there)"
        );
    }

    #[test]
    fn a_subdirectory_is_refused_even_when_a_project_file_exists() {
        let discovery = Discovery::Managed {
            folder: "/r".into(),
            root: "/r".into(),
        };
        assert_eq!(
            locate(&discovery, Path::new("/r/sub")),
            Err(InitRefusal::NotRepositoryRoot { root: "/r".into() })
        );
    }

    #[test]
    fn the_root_itself_is_not_refused() {
        let root = Path::new("/r");
        let unmanaged = Discovery::Unmanaged { root: root.into() };
        let managed = Discovery::Managed {
            folder: root.into(),
            root: root.into(),
        };
        assert_eq!(locate(&unmanaged, root), Ok(root.into()));
        assert_eq!(locate(&managed, root), Ok(root.into()));
    }

    #[test]
    fn the_default_name_is_the_root_folder_not_the_full_path() {
        let naming = name(Path::new("/w/sample"), None, None).unwrap();
        assert_eq!(naming.name, "sample");
        assert_eq!(naming.note, None);
    }

    #[test]
    fn a_given_name_is_not_ignored_without_a_file() {
        let naming = name(Path::new("/w/sample"), Some("chosen"), None).unwrap();
        assert_eq!(naming.name, "chosen");
    }

    #[test]
    fn a_root_with_no_folder_name_refuses_rather_than_naming_it_empty() {
        let refusal = name(Path::new("/"), None, None).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "project-name-required: the repository root / has no folder name to use as \
             the project's name (fix: run baley init --name <name>)"
        );
    }

    #[test]
    fn a_folder_name_that_is_not_utf8_refuses_rather_than_converting_lossily() {
        let root = Path::new(OsStr::from_bytes(b"/w/caf\xe9"));
        assert_eq!(
            name(root, None, None),
            Err(InitRefusal::NameRequired { root: root.into() })
        );
    }

    #[test]
    fn a_differing_name_is_not_applied_over_the_file_and_says_so() {
        let naming = name(Path::new("/w/r"), Some("other"), Some(&identity("kept"))).unwrap();
        assert_eq!(naming.name, "kept");
        assert_eq!(
            naming.note.as_deref(),
            Some("--name was not applied: baley.toml already names the project \"kept\"")
        );
    }

    #[test]
    fn no_note_is_shown_when_nothing_was_ignored() {
        let file = identity("kept");
        for given in [Some("kept"), None] {
            let naming = name(Path::new("/w/r"), given, Some(&file)).unwrap();
            assert_eq!(naming.name, "kept");
            assert_eq!(naming.note, None, "given {given:?}");
        }
    }

    const FRESH: &str = "7d1e9c3a-5b2f-4a68-8c0d-1e2f3a4b5c6d";

    #[test]
    fn a_new_project_file_reads_back_the_id_and_name_it_was_rendered_from() {
        let action = acting(None, false, "sample", FRESH);
        let bytes = file_bytes(&action, None).unwrap().unwrap();
        let file = crate::settings::file(Path::new("/r/baley.toml"), bytes);
        let read = baley_core::policy::read_project(&file).unwrap();
        assert_eq!(read, action.identity);
        assert_eq!(read.id, FRESH);
        assert_eq!(read.name, "sample");
    }

    #[derive(clap::Parser)]
    struct Cli {
        #[command(flatten)]
        init: InitArgs,
    }

    #[test]
    fn the_new_id_flag_is_not_ignored_and_is_off_without_it() {
        use clap::Parser;
        let with = Cli::try_parse_from(["init", "--new-id"]).unwrap();
        assert!(with.init.new_id);
        let plain = Cli::try_parse_from(["init"]).unwrap();
        assert!(!plain.init.new_id);
    }

    /// A working file with every kind of content a new id must carry over.
    const WORKING_TEXT: &str = "escalate_on_failure = true\n\
        [host.claude-code.roles.checker]\n\
        effort = \"xhigh\"\n\
        [review]\n\
        depth = 3\n\
        [project]\n\
        id = \"0b5c1f6e-2a7d-4c3e-9f10-5a6b7c8d9e0f\"\n\
        name = \"kept\"\n\
        owner = \"someone\"\n";

    /// What the layer parser makes of a project file, positions aside.
    fn layer_of(text: &str) -> (Vec<String>, Vec<String>) {
        let file = crate::settings::file(Path::new(WORKING), text.as_bytes().to_vec());
        let parsed = baley_core::policy::parse_layer(
            &file,
            baley_core::policy::FileLayer::Project,
            baley_core::policy::Schema::standard(),
        )
        .unwrap();
        let values = parsed
            .values
            .iter()
            .map(|w| format!("{} {:?} {:?}", w.name, w.host, w.value))
            .collect();
        let ignored = parsed
            .diagnostics
            .iter()
            .map(|d| format!("{} {:?}", d.name, d.kind))
            .collect();
        (values, ignored)
    }

    #[test]
    fn a_new_id_over_a_file_loses_no_name_setting_host_table_or_unknown_table() {
        let working = settings_file(WORKING, WORKING_TEXT);
        let old = observe_file(Ok(Some(working.clone()))).unwrap().unwrap();
        let action = acting(Some(&old), true, "ignored", FRESH);

        let bytes = file_bytes(&action, Some(&working)).unwrap().unwrap();

        let after = String::from_utf8(bytes).unwrap();
        let read = read_project(&settings_file(WORKING, &after)).unwrap();
        assert!(baley_core::policy::is_project_id(&read.id), "{after}");
        assert_ne!(read.id, old.id);
        assert_eq!(read.name, "kept");
        assert!(after.contains("owner = \"someone\""), "{after}");
        let (values_before, ignored_before) = layer_of(WORKING_TEXT);
        let (values_after, ignored_after) = layer_of(&after);
        assert!(!values_before.is_empty());
        assert_eq!(values_after, values_before, "{after}");
        assert!(ignored_before.iter().any(|d| d.starts_with("review.depth")));
        assert_eq!(ignored_after, ignored_before, "{after}");
    }

    #[test]
    fn a_new_id_over_a_file_acts_on_the_fresh_id_and_replaces_the_file_for_every_observation() {
        let old = identity("kept");
        let action = acting(Some(&old), true, "ignored", FRESH);

        assert_eq!(action.identity.id, FRESH);
        assert_ne!(action.identity.id, old.id);
        assert_eq!(action.identity.name, "kept");
        assert_eq!(action.file, FileAction::Replace);
        for observed in ALL {
            assert_eq!(
                plan(action.file, observed),
                vec![
                    Step::ReplaceFile,
                    Step::CreateProject,
                    Step::RecordInitializedOnEmpty,
                    Step::AdmitCheckout
                ],
                "observed {observed:?}"
            );
        }
    }

    #[test]
    fn a_file_without_the_flag_is_kept_with_its_own_id() {
        let old = identity("kept");
        let action = acting(Some(&old), false, "ignored", FRESH);
        assert_eq!(action.identity, old);
        assert_eq!(action.file, FileAction::Keep);
        assert_eq!(file_bytes(&action, None), Ok(None));
    }

    #[test]
    fn a_new_id_with_no_file_is_plain_init_not_a_replace_or_a_refusal() {
        let flagged = acting(None, true, "sample", FRESH);
        let plain = acting(None, false, "sample", FRESH);
        assert_eq!(flagged, plain);
        assert_eq!(flagged.file, FileAction::Write);
        for observed in ALL {
            assert_eq!(
                plan(flagged.file, observed),
                plan(FileAction::Write, observed)
            );
        }
    }

    #[test]
    fn a_new_id_with_a_differing_name_keeps_the_files_name_and_says_so() {
        let old = identity("kept");
        let naming = name(Path::new("/w/r"), Some("other"), Some(&old)).unwrap();
        let action = acting(Some(&old), true, &naming.name, FRESH);

        assert_eq!(action.identity.name, "kept");
        assert_eq!(
            naming.note.as_deref(),
            Some("--name was not applied: baley.toml already names the project \"kept\"")
        );
    }

    #[test]
    fn a_nameless_root_is_not_refused_when_the_file_names_the_project() {
        let naming = name(Path::new("/"), None, Some(&identity("kept"))).unwrap();
        assert_eq!(
            naming,
            Naming {
                name: "kept".into(),
                note: None,
            }
        );
    }

    const T0: &str = "2026-09-29T10:00:00Z";
    const T1: &str = "2026-09-29T10:00:01Z";

    /// A real store in a private home under a fresh temporary directory.
    fn store() -> (tempfile::TempDir, baley_store_sqlite::SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = crate::ledger::open::store(&home, T0, crate::ledger::open::options()).unwrap();
        (dir, store)
    }

    fn request(n: u8) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-0000000000{n:02}"))
    }

    fn events(
        store: &baley_store_sqlite::SqliteStore,
        id: &str,
        stream: &str,
    ) -> Vec<baley_store::Event> {
        let page = baley_store::PageRequest {
            limit: 100,
            after: None,
        };
        let project = ProjectId(id.into());
        store
            .stream(&project, &StreamName(stream.into()), 1, page)
            .unwrap()
            .items
    }

    #[test]
    fn a_first_init_creates_the_project_and_records_one_initialized_event_as_specified() {
        let (_dir, store) = store();
        let project = identity("sample");
        assert!(create_project(&store, &project, T0).unwrap());
        assert!(record_initialized(&store, &project, request(1), T1).unwrap());

        assert_eq!(
            store.projects().unwrap(),
            vec![(ProjectId(project.id.clone()), "sample".to_string())]
        );
        let recorded = events(&store, &project.id, "project");
        assert_eq!(recorded.len(), 1);
        let event = &recorded[0];
        assert_eq!(event.type_name, "project.initialized");
        assert_eq!(event.type_version, 1);
        assert_eq!(event.payload, serde_json::json!({ "name": "sample" }));
        assert_eq!(event.policy_version, 0);
        assert_eq!(event.actor, Actor::Owner);
        let completed = events(&store, &project.id, "command/project.init");
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].type_name, "command.completed");
    }

    #[test]
    fn a_second_record_does_not_duplicate_project_initialized() {
        let (_dir, store) = store();
        let project = identity("sample");
        let id = ProjectId(project.id.clone());
        store.create_project(&id, "sample", T0).unwrap();
        assert!(record_initialized(&store, &project, request(1), T1).unwrap());
        let before = store.head(&id).unwrap().unwrap().seq;

        assert!(!record_initialized(&store, &project, request(2), T1).unwrap());

        let initialized: Vec<_> = events(&store, &project.id, "project")
            .into_iter()
            .filter(|event| event.type_name == "project.initialized")
            .collect();
        assert_eq!(initialized.len(), 1);
        assert_eq!(store.head(&id).unwrap().unwrap().seq, before + 1);
        let completed = events(&store, &project.id, "command/project.init");
        assert_eq!(completed.len(), 2);
        assert_eq!(completed[1].seq, before + 1);
    }

    #[test]
    fn an_existing_project_is_taken_as_present_not_as_a_failure() {
        let (_dir, store) = store();
        let project = identity("sample");
        store
            .create_project(&ProjectId(project.id.clone()), "sample", T0)
            .unwrap();

        assert!(!create_project(&store, &project, T1).unwrap());
        assert!(record_initialized(&store, &project, request(1), T1).unwrap());
        assert_eq!(events(&store, &project.id, "project").len(), 1);
    }

    const ALL: [LedgerObservation; 4] = [
        LedgerObservation {
            project: false,
            initialized: false,
            events: false,
        },
        LedgerObservation {
            project: true,
            initialized: false,
            events: false,
        },
        LedgerObservation {
            project: true,
            initialized: false,
            events: true,
        },
        LedgerObservation {
            project: true,
            initialized: true,
            events: true,
        },
    ];

    #[test]
    fn a_first_run_records_on_the_empty_chain_before_it_admits_the_checkout() {
        for observed in ALL {
            assert_eq!(
                plan(FileAction::Write, observed),
                vec![
                    Step::WriteFile,
                    Step::CreateProject,
                    Step::RecordInitializedOnEmpty,
                    Step::AdmitCheckout
                ],
                "observed {observed:?}"
            );
        }
    }

    #[test]
    fn a_file_without_a_project_is_not_taken_as_a_finished_init() {
        let observed = LedgerObservation::default();
        assert_eq!(
            plan(FileAction::Keep, observed),
            vec![
                Step::CreateProject,
                Step::RecordInitializedOnEmpty,
                Step::AdmitCheckout
            ]
        );
    }

    #[test]
    fn a_project_without_events_records_first_and_is_not_admitted_before_it() {
        let observed = LedgerObservation {
            project: true,
            initialized: false,
            events: false,
        };
        assert_eq!(
            plan(FileAction::Keep, observed),
            vec![Step::RecordInitializedOnEmpty, Step::AdmitCheckout]
        );
    }

    #[test]
    fn a_project_with_events_and_no_initialized_admits_before_the_missing_record() {
        let observed = LedgerObservation {
            project: true,
            initialized: false,
            events: true,
        };
        assert_eq!(
            plan(FileAction::Keep, observed),
            vec![Step::AdmitCheckout, Step::RecordInitialized]
        );
    }

    #[test]
    fn a_rerun_on_a_finished_init_admits_the_checkout_and_nothing_else() {
        let observed = LedgerObservation {
            project: true,
            initialized: true,
            events: true,
        };
        assert_eq!(plan(FileAction::Keep, observed), vec![Step::AdmitCheckout]);
    }

    #[test]
    fn a_bad_project_file_is_refused_naming_it_not_planned_as_absent() {
        let path = Path::new("/r/baley.toml");
        let bad = [
            b"[project\nid = ".to_vec(),
            b"[project]\nid = \"0B5C1F6E-2A7D-4C3E-9F10-5A6B7C8D9E0F\"\nname = \"r\"\n".to_vec(),
        ];
        for bytes in bad {
            let refusal = observe_file(Ok(Some(crate::settings::file(path, bytes)))).unwrap_err();
            assert_eq!(refusal.path, path);
            assert!(
                refusal.to_string().starts_with("config-unavailable: "),
                "{refusal}"
            );
        }
        assert_eq!(observe_file(Ok(None)), Ok(None));
    }

    #[test]
    fn the_ledger_observation_sees_the_recorded_event_so_a_rerun_appends_nothing() {
        let (_dir, store) = store();
        let project = identity("sample");
        assert_eq!(
            observe_ledger(&store, &project.id).unwrap(),
            LedgerObservation {
                project: false,
                initialized: false,
                events: false,
            }
        );
        create_project(&store, &project, T0).unwrap();
        assert_eq!(
            observe_ledger(&store, &project.id).unwrap(),
            LedgerObservation {
                project: true,
                initialized: false,
                events: false,
            }
        );
        record_initialized(&store, &project, request(1), T1).unwrap();
        let observed = observe_ledger(&store, &project.id).unwrap();
        assert_eq!(
            observed,
            LedgerObservation {
                project: true,
                initialized: true,
                events: true,
            }
        );
        assert_eq!(plan(FileAction::Keep, observed), vec![Step::AdmitCheckout]);
    }

    const ROOT_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    /// Another remote's checkout, admitted through checkout admission's store
    /// step, as a second clone of the project would be.
    fn admit_other_remote(store: &(impl Ledger + baley_store::Views), project: &ProjectIdentity) {
        let checkout = baley_core::checkout::Checkout {
            path: "/w/other".into(),
            root_commit: Some(ROOT_COMMIT.into()),
            remote_url: Some("https://example.com/other.git".into()),
        };
        crate::checkout::admit(
            store,
            &ProjectId(project.id.clone()),
            &checkout,
            request(7),
            T1,
            None,
        )
        .unwrap();
    }

    fn head_seq(store: &impl Ledger, project: &ProjectIdentity) -> Option<u64> {
        store
            .head(&ProjectId(project.id.clone()))
            .unwrap()
            .map(|head| head.seq)
    }

    fn type_names(
        store: &baley_store_sqlite::SqliteStore,
        project: &ProjectIdentity,
    ) -> Vec<String> {
        events(store, &project.id, "project")
            .into_iter()
            .map(|event| event.type_name)
            .collect()
    }

    #[test]
    fn an_observation_that_reports_events_for_a_fresh_project_or_none_after_a_checkout_is_wrong() {
        let (_dir, store) = store();
        let project = identity("sample");
        create_project(&store, &project, T0).unwrap();
        assert!(!observe_ledger(&store, &project.id).unwrap().events);

        admit_other_remote(&store, &project);

        let observed = observe_ledger(&store, &project.id).unwrap();
        assert!(observed.events, "a checkout.seen is an event");
        assert!(
            !observed.initialized,
            "checkout.seen is not project.initialized"
        );
    }

    #[test]
    fn an_empty_chain_gets_one_project_initialized_as_specified_not_a_skipped_record() {
        let (_dir, store) = store();
        let project = identity("sample");
        create_project(&store, &project, T0).unwrap();

        let found = record_initialized_on_empty(&store, &project, request(1), T1).unwrap();

        assert_eq!(found, EmptyChain::Recorded);
        let recorded = events(&store, &project.id, "project");
        assert_eq!(recorded.len(), 1);
        let event = &recorded[0];
        assert_eq!(event.type_name, "project.initialized");
        assert_eq!(event.policy_version, 0);
        assert_eq!(event.actor, Actor::Owner);
        assert_eq!(event.payload, serde_json::json!({ "name": "sample" }));
        let completed = events(&store, &project.id, "command/project.init");
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].type_name, "command.completed");
    }

    #[test]
    fn a_chain_holding_another_remotes_checkout_is_not_given_project_initialized_or_a_command() {
        let (_dir, store) = store();
        let project = identity("sample");
        create_project(&store, &project, T0).unwrap();
        admit_other_remote(&store, &project);
        let before = head_seq(&store, &project);

        let found = record_initialized_on_empty(&store, &project, request(1), T1).unwrap();

        assert_eq!(found, EmptyChain::FoundEvents);
        assert_eq!(head_seq(&store, &project), before);
        assert!(!type_names(&store, &project).contains(&"project.initialized".to_string()));
        assert!(events(&store, &project.id, "command/project.init").is_empty());
    }

    /// The real store, with another remote's checkout admitted on the first
    /// `transact` only, after the caller looked and before its command runs.
    struct AdmitsFirst<'a> {
        store: &'a baley_store_sqlite::SqliteStore,
        project: &'a ProjectIdentity,
        raced: std::cell::Cell<bool>,
    }

    impl Ledger for AdmitsFirst<'_> {
        fn transact(
            &self,
            command: &Command,
            decide: &mut baley_store::Decide<'_>,
        ) -> Result<baley_store::Recorded, StoreError> {
            if !self.raced.replace(true) {
                admit_other_remote(self.store, self.project);
            }
            Ledger::transact(self.store, command, decide)
        }

        fn claim(
            &self,
            command: &Command,
            decide: &mut baley_store::DecideClaim<'_>,
        ) -> Result<baley_store::Claimed, StoreError> {
            Ledger::claim(self.store, command, decide)
        }

        fn renew_lease(
            &self,
            project: &ProjectId,
            claim: &baley_store::ClaimId,
            owner: &baley_store::ClaimOwner,
            at: &str,
        ) -> Result<(), StoreError> {
            Ledger::renew_lease(self.store, project, claim, owner, at)
        }

        fn complete(
            &self,
            command: &Command,
            owner: &baley_store::ClaimOwner,
            decide: &mut baley_store::Decide<'_>,
        ) -> Result<baley_store::Recorded, StoreError> {
            Ledger::complete(self.store, command, owner, decide)
        }

        fn reconcile(
            &self,
            command: &Command,
            claim: &baley_store::ClaimId,
            authority: baley_store::ReconcileAuthority,
            decide: &mut baley_store::DecideReconcile<'_>,
        ) -> Result<baley_store::Recorded, StoreError> {
            Ledger::reconcile(self.store, command, claim, authority, decide)
        }

        fn open_claims(&self, project: &ProjectId) -> Result<Vec<baley_store::Claim>, StoreError> {
            Ledger::open_claims(self.store, project)
        }

        fn stream(
            &self,
            project: &ProjectId,
            stream: &StreamName,
            from_version: u64,
            page: PageRequest,
        ) -> Result<baley_store::Page<baley_store::Event>, StoreError> {
            Ledger::stream(self.store, project, stream, from_version, page)
        }

        fn history(
            &self,
            project: &ProjectId,
            range: std::ops::RangeInclusive<u64>,
            filter: &baley_store::HistoryFilter,
            page: PageRequest,
        ) -> Result<baley_store::Page<baley_store::Event>, StoreError> {
            Ledger::history(self.store, project, range, filter, page)
        }

        fn head(&self, project: &ProjectId) -> Result<Option<baley_store::Head>, StoreError> {
            Ledger::head(self.store, project)
        }

        fn verify(
            &self,
            project: &ProjectId,
            anchor: Option<&baley_store::Anchor>,
        ) -> Result<baley_store::VerifyReport, StoreError> {
            Ledger::verify(self.store, project, anchor)
        }
    }

    #[test]
    fn a_checkout_admitted_after_the_chain_was_read_empty_is_not_followed_by_project_initialized() {
        let (_dir, store) = store();
        let project = identity("sample");
        create_project(&store, &project, T0).unwrap();
        assert!(
            !observe_ledger(&store, &project.id).unwrap().events,
            "the chain is empty when read"
        );
        let racing = AdmitsFirst {
            store: &store,
            project: &project,
            raced: std::cell::Cell::new(false),
        };

        let found = record_initialized_on_empty(&racing, &project, request(1), T1).unwrap();

        assert_eq!(found, EmptyChain::FoundEvents);
        assert!(racing.raced.get());
        let seen = events(&store, &project.id, "project");
        assert_eq!(seen.len(), 1, "only the racer's checkout.seen");
        assert_eq!(seen[0].type_name, "checkout.seen");
        let admitted = events(&store, &project.id, "command/checkout.admit");
        assert_eq!(
            head_seq(&store, &project),
            admitted.last().map(|event| event.seq)
        );
        assert!(events(&store, &project.id, "command/project.init").is_empty());
    }

    const ID: &str = "0b5c1f6e-2a7d-4c3e-9f10-5a6b7c8d9e0f";
    const GLOBAL: &str = "/c/config.toml";
    const WORKING: &str = "/r/baley.toml";

    fn settings_file(path: &str, text: &str) -> SettingsFile {
        crate::settings::file(Path::new(path), text.as_bytes().to_vec())
    }

    /// A valid project file naming `kept`, with `settings` above its table.
    fn project_text(settings: &str) -> String {
        let table = render_project(ID, "kept", None).unwrap();
        format!("{settings}{}", String::from_utf8(table).unwrap())
    }

    fn reads(global: Option<&str>, head: Option<Result<&str, Unavailable>>) -> Reads {
        Reads {
            global: Ok(global.map(|text| settings_file(GLOBAL, text))),
            head: head.map(|read| {
                read.map(|text| crate::committed::Committed {
                    layer: Some(settings_file(WORKING, text)),
                    pending: None,
                })
            }),
        }
    }

    fn unreadable(path: &str, cause: &str) -> Unavailable {
        Unavailable {
            path: path.into(),
            fault: baley_core::policy::Fault::Unreadable {
                cause: cause.into(),
            },
        }
    }

    #[test]
    fn a_first_init_with_an_invalid_global_file_is_refused_before_the_file_is_written() {
        let root = Path::new("/r");
        let invalid = reads(Some("escalate_on_failure = [\n"), None);
        let refusal = prepare(root, None, Ok(None), &invalid).unwrap_err();
        assert!(
            refusal.starts_with("config-unavailable: /c/config.toml:"),
            "{refusal}"
        );

        // The same observations with a valid global file plan the write.
        let valid = reads(Some("escalate_on_failure = true\n"), None);
        let prepared = prepare(root, None, Ok(None), &valid).unwrap();
        assert_eq!(prepared.existing, None);
        let action = acting(
            prepared.existing.as_ref(),
            false,
            &prepared.naming.name,
            FRESH,
        );
        let steps = plan(action.file, LedgerObservation::default());
        assert_eq!(steps.first(), Some(&Step::WriteFile));
    }

    #[test]
    fn a_wrong_type_in_heads_copy_is_refused_as_heads_not_the_working_trees() {
        let working = project_text("");
        let head = project_text("escalate_on_failure = \"yes\"\n");
        let observed = reads(None, Some(Ok(&head)));
        let file = Ok(Some(settings_file(WORKING, &working)));
        let refusal = prepare(Path::new("/r"), None, file, &observed).unwrap_err();
        assert!(
            refusal.starts_with("config-unavailable: HEAD's copy of /r/baley.toml:1:"),
            "{refusal}"
        );
    }

    #[test]
    fn a_failed_read_of_heads_copy_is_refused_with_its_cause() {
        let failed = unreadable(WORKING, "HEAD's copy: git exited with status 128");
        let observed = reads(None, Some(Err(failed.clone())));
        let file = Ok(Some(settings_file(WORKING, &project_text(""))));
        let refusal = prepare(Path::new("/r"), None, file, &observed).unwrap_err();
        assert_eq!(refusal, failed.to_string());
    }

    #[test]
    fn the_global_file_is_not_judged_before_an_invalid_working_tree_file() {
        let observed = reads(Some("escalate_on_failure = [\n"), None);
        let file = Ok(Some(settings_file(WORKING, "[project\nid = ")));
        let refusal = prepare(Path::new("/r"), None, file, &observed).unwrap_err();
        assert!(
            refusal.starts_with("config-unavailable: /r/baley.toml:"),
            "{refusal}"
        );
    }

    #[test]
    fn a_root_that_is_not_utf8_is_refused_rather_than_recorded_lossily() {
        let root = Path::new(OsStr::from_bytes(b"/w/caf\xe9"));
        let refusal = prepare(root, Some("cafe"), Ok(None), &reads(None, None)).unwrap_err();
        assert!(refusal.starts_with("config-unavailable: "), "{refusal}");
        assert!(
            refusal.contains(&format!("{} is not UTF-8", root.display())),
            "{refusal}"
        );
    }

    #[test]
    fn valid_observations_do_not_lose_the_identity_naming_or_either_files_source() {
        let head = project_text("roles.reviewer.effort = \"high\"\n");
        let observed = reads(Some("escalate_on_failure = true\n"), Some(Ok(&head)));
        let file = Ok(Some(settings_file(WORKING, &head)));

        let prepared = prepare(Path::new("/r"), Some("other"), file, &observed).unwrap();

        assert_eq!(prepared.existing, Some(identity("kept")));
        assert_eq!(prepared.naming.name, "kept");
        let payload = baley_core::policy::recorded::effective_payload(&prepared.recorded, ID, 0);
        assert_eq!(payload["checkout"], "/r");
        assert_eq!(payload["sources"]["escalate_on_failure"]["path"], GLOBAL);
        assert_eq!(payload["sources"]["roles.reviewer.effort"]["path"], WORKING);
        assert_eq!(payload["values"]["roles.reviewer.effort"], "high");
    }
}
