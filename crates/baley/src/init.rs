//! `baley init`: ties a repository to a ledger project (design 0001, EVD-R17).
//! The judges here take plain values; the command gathers them.
use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use baley_core::policy::recorded::{RecordedPolicy, recorded_policy};
use baley_core::policy::{
    ProjectIdentity, SettingsFile, Unavailable, read_project, render_project,
};
use baley_core::{PROJECT_INITIALIZED, PROJECT_INITIALIZED_VERSION};
use baley_store::{
    Actor, Admin, Command, CommandKind, Decision, EVENT_PAGE_BOUND, EventMatch, Ledger, NewEvent,
    Observed, OutcomeKind, PageRequest, ProjectId, Refusal, RequestId, StoreError, StreamName,
    request_digest,
};
use clap::Args;
use serde_json::json;

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

/// Chooses the project's name. An existing file's name always wins, since
/// the file is never changed. Otherwise `--name` as given, empty included,
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

/// A new project: a fresh id, the chosen name, and the bytes of a project file
/// naming both. The id is a lower-case hyphenated UUID version 4, the one form
/// `read_project` accepts.
pub fn new_project(name: &str) -> Result<(ProjectIdentity, Vec<u8>), Unavailable> {
    let id = uuid::Uuid::new_v4().to_string();
    let bytes = render_project(&id, name, None)?;
    let identity = ProjectIdentity {
        id,
        name: name.into(),
    };
    Ok((identity, bytes))
}

/// Writes a new `baley.toml` at the root and returns its path. Only an absent
/// file is written, so the expected digest is none: `replace` refuses a file
/// that appeared since it was read, and a link. The file gets the umask's mode.
pub fn write_project_file(root: &Path, bytes: &[u8]) -> Result<PathBuf, replace::Failure> {
    let path = root.join(PROJECT_FILE);
    replace::replace(&path, bytes, None)?;
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

/// Records `project.initialized` once. True when this run appended it. The
/// check runs inside the transaction, so a racing second init records only
/// its `command.completed`.
pub fn record_initialized(
    store: &impl Ledger,
    identity: &ProjectIdentity,
    request_id: RequestId,
    at: &str,
) -> Result<bool, StoreError> {
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
    let command = Command {
        project: ProjectId(identity.id.clone()),
        kind: CommandKind(INIT_COMMAND.into()),
        request_id,
        digest,
        scope: vec![],
        policy_version: 0,
        recorded_at: at.into(),
        actor,
    };
    let initialized = EventMatch {
        type_name: PROJECT_INITIALIZED.into(),
        stream: Some(StreamName(PROJECT_STREAM.into())),
        fields: BTreeMap::new(),
    };
    let mut appended = false;
    store.transact(&command, &mut |tx| {
        appended = !tx.event_exists(&initialized)?;
        if appended {
            tx.append(NewEvent {
                stream: StreamName(PROJECT_STREAM.into()),
                type_name: PROJECT_INITIALIZED.into(),
                type_version: PROJECT_INITIALIZED_VERSION,
                git: None,
                payload: json!({ "name": identity.name }),
                attachments: vec![],
            })?;
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

/// One action of `baley init`, in the order the plan lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Write a new `baley.toml` with a fresh id.
    WriteFile,
    /// Create the project in the ledger.
    CreateProject,
    /// Record `project.initialized`.
    RecordInitialized,
}

/// What the ledger holds for the file's project, read outside any transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LedgerObservation {
    /// The project is in the ledger.
    pub project: bool,
    /// Its `project` stream holds a `project.initialized`.
    pub initialized: bool,
}

/// Decides which actions this run takes, from what was observed before any
/// of them. A fully initialized checkout plans nothing, so a rerun appends
/// nothing to the chain.
///
/// The policy step, like detection, is not a step of the plan: on every run
/// that passes the refusals it runs after the steps and before detection,
/// and it appends nothing while the policy is unchanged. Checkout admission
/// with `--new-id` (T13) is later work here.
pub fn plan(file: Option<&ProjectIdentity>, ledger: LedgerObservation) -> Vec<Step> {
    match (file, ledger) {
        // A new id has nothing to look up.
        (None, _) => vec![
            Step::WriteFile,
            Step::CreateProject,
            Step::RecordInitialized,
        ],
        (Some(_), LedgerObservation { project: false, .. }) => {
            vec![Step::CreateProject, Step::RecordInitialized]
        }
        (
            Some(_),
            LedgerObservation {
                initialized: false, ..
            },
        ) => vec![Step::RecordInitialized],
        (Some(_), _) => vec![],
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
    let policy = policy_step::build(reads).map_err(|e| e.to_string())?;
    let recorded = recorded_policy(root, &policy).map_err(|e| e.to_string())?;
    Ok(Prepared {
        existing,
        naming,
        recorded,
    })
}

/// Reads whether the project is in the ledger and, when it is, whether its
/// `project` stream holds a `project.initialized`, following every page.
pub fn observe_ledger(
    store: &(impl Admin + Ledger),
    id: &str,
) -> Result<LedgerObservation, StoreError> {
    let project = ProjectId(id.into());
    if !store.projects()?.iter().any(|(known, _)| *known == project) {
        return Ok(LedgerObservation::default());
    }
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
    let working = file.as_ref().ok().and_then(Option::as_ref);
    let reads = policy_step::gather(&folders.config, &root, working);
    let Prepared {
        existing,
        naming,
        recorded,
    } = prepare(&root, args.name.as_deref(), file, &reads).map_err(Render::refusal)?;

    // Nothing above opens the store, so a refusal leaves no ledger home behind.
    let store = open::store(&folders.home, started_at, open::options())
        .map_err(|e| display::store_error(&e, None))?;
    // A new id is in no ledger, so only an existing file's project is looked up.
    let observed = match &existing {
        Some(identity) => observe_ledger(&store, &identity.id)
            .map_err(|e| display::store_error(&e, Some(&identity.id)))?,
        None => LedgerObservation::default(),
    };
    let steps = plan(existing.as_ref(), observed);
    let mut lines: Vec<String> = naming.note.into_iter().collect();
    let mut acted = false;
    let identity = match existing {
        Some(identity) => identity,
        // The plan writes the file exactly when there is none.
        None => {
            let (identity, bytes) = new_project(&naming.name).map_err(|e| refuse(&e))?;
            write_project_file(&root, &bytes).map_err(|failure| match failure {
                replace::Failure::Refused(conflict) => refuse(&conflict),
                other => Render {
                    lines: vec![other.to_string()],
                    code: 3,
                    error: true,
                },
            })?;
            lines.push(format!("wrote {} (commit this file)", path.display()));
            acted = true;
            identity
        }
    };
    let failed = |e: StoreError| display::store_error(&e, Some(&identity.id));
    if steps.contains(&Step::CreateProject)
        && create_project(&store, &identity, started_at).map_err(failed)?
    {
        lines.push(format!(
            "created project {} in the ledger at {}",
            identity.id,
            folders.home.display()
        ));
        acted = true;
    }
    if steps.contains(&Step::RecordInitialized)
        && record_initialized(&store, &identity, new_request_id(), started_at).map_err(failed)?
    {
        lines.push(format!(
            "recorded project.initialized for project {}",
            identity.id
        ));
        acted = true;
    }
    // Outside the plan, so a rerun finishes a crash between
    // `project.initialized` and the first `policy.effective`. It prints
    // nothing, and init's own events keep policy version 0.
    let project = ProjectId(identity.id.clone());
    policy_step::step(&store, &project, &recorded, new_request_id(), started_at).map_err(failed)?;
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

    #[test]
    fn a_new_project_file_reads_back_the_id_and_name_it_was_rendered_from() {
        let (identity, bytes) = new_project("sample").unwrap();
        let file = crate::settings::file(Path::new("/r/baley.toml"), bytes);
        let read = baley_core::policy::read_project(&file).unwrap();
        assert_eq!(read, identity);
        assert_eq!(read.name, "sample");
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

    const ALL: [LedgerObservation; 2] = [
        LedgerObservation {
            project: false,
            initialized: false,
        },
        LedgerObservation {
            project: true,
            initialized: true,
        },
    ];

    #[test]
    fn a_first_run_plans_write_create_record_in_that_order() {
        for observed in ALL {
            assert_eq!(
                plan(None, observed),
                vec![
                    Step::WriteFile,
                    Step::CreateProject,
                    Step::RecordInitialized
                ],
                "observed {observed:?}"
            );
        }
    }

    #[test]
    fn a_file_without_a_project_is_not_taken_as_a_finished_init() {
        let observed = LedgerObservation::default();
        assert_eq!(
            plan(Some(&identity("kept")), observed),
            vec![Step::CreateProject, Step::RecordInitialized]
        );
    }

    #[test]
    fn a_project_row_without_its_event_is_not_taken_as_a_finished_init() {
        let observed = LedgerObservation {
            project: true,
            initialized: false,
        };
        assert_eq!(
            plan(Some(&identity("kept")), observed),
            vec![Step::RecordInitialized]
        );
    }

    #[test]
    fn a_rerun_on_a_finished_init_plans_nothing() {
        let observed = LedgerObservation {
            project: true,
            initialized: true,
        };
        assert_eq!(plan(Some(&identity("kept")), observed), vec![]);
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
            }
        );
        create_project(&store, &project, T0).unwrap();
        assert_eq!(
            observe_ledger(&store, &project.id).unwrap(),
            LedgerObservation {
                project: true,
                initialized: false,
            }
        );
        record_initialized(&store, &project, request(1), T1).unwrap();
        let observed = observe_ledger(&store, &project.id).unwrap();
        assert_eq!(
            observed,
            LedgerObservation {
                project: true,
                initialized: true,
            }
        );
        assert_eq!(plan(Some(&project), observed), vec![]);
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
        let steps = plan(prepared.existing.as_ref(), LedgerObservation::default());
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
