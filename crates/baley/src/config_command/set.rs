//! `baley config set`: one function that judges every pair, checks the files,
//! writes one file and prints what it did (design 0003 section 5, CFG-R4,
//! CFG-R5, CFG-R7, CFG-R9). The judges live in `baley_core`; this file
//! gathers what they judge and performs the write.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use baley_core::catalog::{
    Catalog, MODEL_CATALOG_VIEW, USER_PROJECT, accepted_names, catalog_key, state_key,
};
use baley_core::policy::config_command::{
    Place, VersionSource, changed_pairs, choose_outcome, collapse_repeats, judge_models,
    judge_pairs, needs_catalog, render_file, render_set,
};
use baley_core::policy::recorded::{POLICY_VIEW, PathNotUtf8, policy_key, recorded_policy};
use baley_core::policy::{
    AcceptedNames, Fault, FileLayer, Host, ParsedLayer, Schema, SettingsFile, Unavailable,
    parse_layer,
};
use baley_store::{Admin, Ledger, ProjectId, RequestId, StoreError, Views};

use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::folders::{self, Environment, Folders, Platform};
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::ledger::open;
use crate::policy_step::{self, Reads};
use crate::{init, models, replace, settings};

/// Writes `pairs` into the file `layer` names and returns what the command
/// prints. Every pair is judged before anything is written, then one file
/// write follows. `baley config interview` calls this too.
pub(crate) fn set(layer: FileLayer, host: Option<Host>, pairs: &[(String, String)]) -> Render {
    attempt(layer, host, pairs).unwrap_or_else(|render| render)
}

fn refusal(text: impl ToString) -> Render {
    Render::refusal(text.to_string())
}

fn attempt(
    layer: FileLayer,
    host: Option<Host>,
    pairs: &[(String, String)],
) -> Result<Render, Render> {
    let folders = Folders::resolve(Platform::current(), &Environment::read()).map_err(refusal)?;
    let unavailable =
        |error: io::Error| display::store_error(&StoreError::Unavailable(error.to_string()), None);
    let cwd = std::env::current_dir().map_err(unavailable)?;
    let ancestors = discovery::ancestors(&cwd).map_err(unavailable)?;
    let found = match discovery::discover(&ancestors) {
        Discovery::Managed { folder, root } => Some((folder, root)),
        Discovery::Unmanaged { .. } | Discovery::Outside => None,
    };

    let schema = Schema::standard();
    let named: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let typed = judge_pairs(schema, layer, found.is_some(), host, &named).map_err(refusal)?;

    let global_path = settings::global_path(&folders);
    let (project, reads) = match found {
        Some((folder, root)) => {
            let working = settings::read(&folder.join(PROJECT_FILE));
            let head = working.as_ref().ok().and_then(Option::as_ref);
            let reads = policy_step::gather(&folders.config, &root, head);
            (
                Some(ProjectSeen {
                    folder,
                    root,
                    working,
                }),
                reads,
            )
        }
        None => {
            let global = settings::read(&global_path);
            (None, Reads { global, head: None })
        }
    };
    let prepared =
        prepare(layer, &global_path, project.as_ref(), &reads, schema).map_err(refusal)?;
    refuse_link(&prepared.target).map_err(refusal)?;

    // The store opens before the write, so an unsafe ledger refuses while the
    // file is still as it was.
    let at = SystemClock::now();
    let mut store = None;
    let mut place = Place::OutsideProject;
    if let Some(project) = &prepared.project {
        let opened = open::store(&folders.home, &at, open::options())
            .map_err(|error| display::store_error(&error, None))?;
        let failed = |error: StoreError| display::store_error(&error, Some(&project.id));
        place = if in_ledger(&opened, &project.id).map_err(failed)? {
            Place::LedgeredProject
        } else {
            Place::ProjectNotInLedger
        };
        store = Some(opened);
    }
    if needs_catalog(&typed) {
        let seeded = match store.take() {
            Some(store) => store,
            None => open::store(&folders.home, &at, open::options())
                .map_err(|error| display::store_error(&error, None))?,
        };
        let failed = |error: StoreError| display::store_error(&error, None);
        models::seed(&seeded, new_request_id(), &at).map_err(failed)?;
        let accepted = accepted_by_host(&seeded).map_err(failed)?;
        judge_models(&typed, host, &accepted).map_err(refusal)?;
        store = Some(seeded);
    }
    let ledgered = match (place, &store, &prepared.project) {
        (Place::LedgeredProject, Some(store), Some(project)) => {
            let checkout = project.root.to_str().ok_or_else(|| {
                refusal(PathNotUtf8 {
                    path: project.root.clone(),
                })
            })?;
            Some((store, project, checkout))
        }
        _ => None,
    };

    let typed = collapse_repeats(typed);
    let changed = changed_pairs(prepared.current.as_ref(), &typed);
    let outcome = choose_outcome(!changed.is_empty(), place);
    if outcome.write {
        let bytes =
            render_file(schema, layer, prepared.base.as_ref(), &changed).map_err(refusal)?;
        let digest = prepared.base.as_ref().map(|file| file.digest.as_str());
        let config = (layer == FileLayer::Global).then_some(folders.config.as_path());
        write_file(&prepared.target, &bytes, digest, config)
            .map_err(|failure| render_failure(&failure))?;
    }
    let version = match (outcome.version, ledgered) {
        (VersionSource::Step, Some((store, project, _))) => {
            // A failure from here on leaves the change in place, so it says so.
            let step = || -> Result<u64, Render> {
                let reads = regather(&folders.config, project).map_err(refusal)?;
                record_policy(store, project, &reads, new_request_id(), &at)
            };
            step().map_err(|mut render| {
                let note = format!("wrote {}, and the change stands", prepared.target.display());
                render.lines.insert(0, note);
                render
            })?
        }
        (VersionSource::Stored, Some((store, project, checkout))) => {
            let id = ProjectId(project.id.clone());
            stored_version(store, &id, checkout)
                .map_err(|error| display::store_error(&error, Some(&project.id)))?
        }
        _ => 0,
    };
    Ok(Render {
        lines: render_set(layer, &prepared.target, &changed, &outcome, version),
        code: 0,
        error: false,
    })
}

/// Reads both settings files and HEAD's copy again after the write, so the
/// step records the policy now in force. A project file that is gone or
/// unreadable leaves no policy to record.
fn regather(config: &Path, project: &Project) -> Result<Reads, Unavailable> {
    let path = project.folder.join(PROJECT_FILE);
    let working = settings::read(&path)?.ok_or_else(|| Unavailable {
        path,
        fault: Fault::Unreadable {
            cause: "the file was not found".to_owned(),
        },
    })?;
    Ok(policy_step::gather(config, &project.root, Some(&working)))
}

/// Each host's accepted model names from the `user` project's `model_catalog`
/// view: its compiled aliases and the entries in its own catalog. A provider's
/// catalog is never read. One snapshot, so the names agree with each other.
fn accepted_by_host(store: &impl Views) -> Result<BTreeMap<Host, AcceptedNames>, StoreError> {
    let keys: Vec<_> = std::iter::once(state_key())
        .chain(
            Host::ALL
                .iter()
                .map(|host| catalog_key(Catalog::Host(*host))),
        )
        .collect();
    let mut bodies = store
        .get_many(&ProjectId(USER_PROJECT.into()), MODEL_CATALOG_VIEW, &keys)?
        .into_iter()
        .map(|document| document.map(|document| document.body));
    let state = bodies.next().flatten();
    let mut accepted = BTreeMap::new();
    for (host, body) in Host::ALL.into_iter().zip(bodies) {
        let names = accepted_names(host.name(), body.as_ref(), state.as_ref())
            .map_err(|error| StoreError::Unavailable(error.to_string()))?;
        accepted.insert(host, names);
    }
    Ok(accepted)
}

/// Whether the ledger lists the project. Nothing is created, since only
/// `baley init` puts a project in the ledger.
fn in_ledger(store: &impl Admin, id: &str) -> Result<bool, StoreError> {
    Ok(store.projects()?.iter().any(|(known, _)| known.0 == id))
}

/// The version of the policy recorded for the checkout and the command line,
/// 0 when none is recorded.
fn stored_version(
    store: &impl Views,
    project: &ProjectId,
    checkout: &str,
) -> Result<u64, StoreError> {
    let document = store.get(project, POLICY_VIEW, &policy_key(checkout, None))?;
    Ok(document
        .and_then(|document| document.body.get("version")?.as_u64())
        .unwrap_or(0))
}

/// Builds the policy from `reads` and runs the policy step for the checkout
/// at the project's root, never the project file's folder, so a nested project file keys
/// its checkout by the repository root. Returns the version in force.
fn record_policy(
    store: &(impl Admin + Views + Ledger),
    project: &Project,
    reads: &Reads,
    request_id: RequestId,
    at: &str,
) -> Result<u64, Render> {
    let policy = policy_step::build(reads).map_err(refusal)?;
    let recorded = recorded_policy(&project.root, &policy).map_err(refusal)?;
    policy_step::step(
        store,
        &ProjectId(project.id.clone()),
        &recorded,
        request_id,
        at,
    )
    .map_err(|error| display::store_error(&error, Some(&project.id)))
}

/// The project the working directory is in, with its project file as read.
#[derive(Debug, Clone)]
struct ProjectSeen {
    /// The folder holding `baley.toml`.
    folder: PathBuf,
    /// The repository root.
    root: PathBuf,
    /// The working-tree `baley.toml` as `settings::read` returned it.
    working: Result<Option<SettingsFile>, Unavailable>,
}

/// The project a set runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Project {
    /// The folder holding the project file.
    folder: PathBuf,
    /// The id the working-tree file names.
    id: String,
    /// The repository root, the checkout the policy step records.
    root: PathBuf,
}

/// What the checks before a write found: the file to write and its base.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Prepared {
    /// The file the set writes.
    target: PathBuf,
    /// The target as it was read, the bytes the new file is rendered from and
    /// the digest the write is checked against. `None` for a first write.
    base: Option<SettingsFile>,
    /// The base parsed as the target's layer, for the no-op judge.
    current: Option<ParsedLayer>,
    /// The project, when the set runs in one.
    project: Option<Project>,
}

/// The id the working-tree project file names, as a project set needs it. A
/// read that finds no file is refused as not found, since a set in a project
/// rewrites a file that exists, and a file with a missing or malformed id is
/// refused. `baley config interview` runs it before its first question.
pub(super) fn working_tree_id(
    folder: &Path,
    working: Result<Option<SettingsFile>, Unavailable>,
) -> Result<String, Unavailable> {
    let identity = init::observe_file(working)?.ok_or_else(|| Unavailable {
        path: folder.join(PROJECT_FILE),
        fault: Fault::Unreadable {
            cause: "the file was not found".to_owned(),
        },
    })?;
    Ok(identity.id)
}

/// Judges the supplied reads before anything is written, as `init::prepare`
/// does. In order: the working-tree file's read and id, the policy from the
/// reads (global, then HEAD's copy), then the target as its layer. The first
/// refusal is returned.
///
/// A project-file set is based on the working-tree file, never on HEAD's
/// copy: HEAD's digest would refuse every file with uncommitted edits, and
/// HEAD's bytes would throw those edits away.
fn prepare(
    layer: FileLayer,
    global_path: &Path,
    project: Option<&ProjectSeen>,
    reads: &Reads,
    schema: &Schema,
) -> Result<Prepared, Unavailable> {
    let seen = match project {
        Some(seen) => {
            let id = working_tree_id(&seen.folder, seen.working.clone())?;
            Some((seen, id))
        }
        None => None,
    };
    policy_step::build(reads)?;
    let (target, base) = match (layer, &seen) {
        (FileLayer::Project, Some((seen, _))) => {
            (seen.folder.join(PROJECT_FILE), seen.working.clone()?)
        }
        _ => (global_path.to_owned(), reads.global.clone()?),
    };
    let current = base
        .as_ref()
        .map(|file| parse_layer(file, layer, schema))
        .transpose()?;
    Ok(Prepared {
        target,
        base,
        current,
        project: seen.map(|(seen, id)| Project {
            id,
            folder: seen.folder.clone(),
            root: seen.root.clone(),
        }),
    })
}

/// Refuses a target that is a symbolic link, so a set that changes nothing
/// refuses on one as a changing set does. A missing file and a regular file
/// pass, and `replace` makes its own check at the write.
fn refuse_link(target: &Path) -> Result<(), replace::Conflict> {
    match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => Err(replace::Conflict::Link {
            path: target.to_owned(),
        }),
        _ => Ok(()),
    }
}

/// Why the write did not happen as asked.
#[derive(Debug)]
enum WriteFailure {
    /// The config folder could not be created.
    Folder {
        /// The folder.
        folder: PathBuf,
        /// The operating system's error.
        cause: io::Error,
    },
    /// `replace` refused or failed.
    Replace(replace::Failure),
}

/// Replaces `target` whole with `bytes`, given the digest of the file as it
/// was read. A global set also creates the config folder when it is missing.
/// The target is never followed, so `replace` refuses a link.
fn write_file(
    target: &Path,
    bytes: &[u8],
    digest: Option<&str>,
    config_folder: Option<&Path>,
) -> Result<(), WriteFailure> {
    if let Some(folder) = config_folder {
        folders::create_private(folder).map_err(|cause| WriteFailure::Folder {
            folder: folder.to_owned(),
            cause,
        })?;
    }
    replace::replace(target, bytes, digest).map_err(WriteFailure::Replace)
}

/// A refusal exits 2 and any other write failure exits 3, as `init` maps it.
fn render_failure(failure: &WriteFailure) -> Render {
    let failed = |text: String| Render {
        lines: vec![text],
        code: 3,
        error: true,
    };
    match failure {
        WriteFailure::Replace(replace::Failure::Refused(conflict)) => refusal(conflict),
        WriteFailure::Replace(other) => failed(other.to_string()),
        WriteFailure::Folder { folder, cause } => {
            failed(format!("cannot create {}: {cause}", folder.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    const T0: &str = "2026-10-01T10:00:00Z";
    const T1: &str = "2026-10-01T10:00:01Z";

    fn request(n: u8) -> RequestId {
        RequestId(format!("00000000-0000-4000-8000-0000000000{n:02}"))
    }

    fn store() -> (tempfile::TempDir, baley_store_sqlite::SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = open::store(&home, T0, open::options()).unwrap();
        (dir, store)
    }

    fn recorded_events(store: &baley_store_sqlite::SqliteStore) -> Vec<baley_store::Event> {
        let page = baley_store::PageRequest {
            limit: 100,
            after: None,
        };
        let stream = baley_store::StreamName("project".into());
        store
            .stream(&ProjectId(ID.into()), &stream, 1, page)
            .unwrap()
            .items
            .into_iter()
            .filter(|event| event.type_name == "policy.effective")
            .collect()
    }

    fn found() -> Project {
        Project {
            id: ID.into(),
            folder: FOLDER.into(),
            root: ROOT.into(),
        }
    }

    /// Supplied reads for a project file in `/r/app`, with no git.
    fn nested_reads() -> Reads {
        reads(None, Some(PROJECT_TEXT))
    }

    #[test]
    fn membership_reads_the_ledger_without_creating_the_project() {
        let (_dir, store) = store();
        store
            .create_project(&ProjectId(ID.into()), "sample", T0)
            .unwrap();
        let before = store.projects().unwrap();

        assert!(in_ledger(&store, ID).unwrap());
        assert!(!in_ledger(&store, "0b5c1f6e-2a7d-4c3e-9f10-5a6b7c8d9e0f").unwrap());

        assert_eq!(store.projects().unwrap(), before);
    }

    #[test]
    fn the_stored_version_is_the_recorded_event_of_that_checkout_only() {
        let (_dir, store) = store();
        let id = ProjectId(ID.into());
        store.create_project(&id, "sample", T0).unwrap();
        let project = found();
        let version = record_policy(&store, &project, &nested_reads(), request(1), T1).unwrap();
        let events = recorded_events(&store);
        assert_eq!(events.len(), 1);
        assert_eq!(version, events[0].seq);

        assert_eq!(stored_version(&store, &id, ROOT).unwrap(), events[0].seq);
        assert_eq!(stored_version(&store, &id, "/w/other").unwrap(), 0);
    }

    #[test]
    fn each_hosts_names_come_from_its_own_catalog_after_seeding() {
        let (_dir, store) = store();
        models::seed(&store, request(1), T0).unwrap();
        models::change(
            &store,
            Catalog::Host(Host::Codex),
            "gpt-owner-added",
            baley_core::catalog::OwnerChange::Added(None),
            request(2),
            T1,
        )
        .unwrap();

        let accepted = accepted_by_host(&store).unwrap();

        let claude = &accepted[&Host::ClaudeCode].names;
        let codex = &accepted[&Host::Codex].names;
        assert!(claude.contains("opus"), "{claude:?}");
        assert!(!codex.contains("opus"), "{codex:?}");
        assert!(codex.contains("gpt-owner-added"), "{codex:?}");
        assert!(!claude.contains("gpt-owner-added"), "{claude:?}");
    }

    #[test]
    fn the_step_keys_its_checkout_by_the_repository_root_not_the_project_folder() {
        let (_dir, store) = store();
        store
            .create_project(&ProjectId(ID.into()), "sample", T0)
            .unwrap();

        let version = record_policy(&store, &found(), &nested_reads(), request(1), T1).unwrap();

        let events = recorded_events(&store);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["checkout"], "/r");
        assert_eq!(version, events[0].seq);
    }

    const GLOBAL: &str = "/c/config.toml";
    const FOLDER: &str = "/r/app";
    const ROOT: &str = "/r";
    const ID: &str = "6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f";
    const PROJECT_TEXT: &str =
        "[project]\nid = \"6f1c2a4e-8b1d-4c3a-9e2f-0a5b7c9d1e3f\"\nname = \"sample\"\n";

    fn file(path: &str, text: &str) -> SettingsFile {
        settings::file(Path::new(path), text.as_bytes().to_vec())
    }

    fn working(text: &str) -> Result<Option<SettingsFile>, Unavailable> {
        Ok(Some(file("/r/app/baley.toml", text)))
    }

    fn seen(working: Result<Option<SettingsFile>, Unavailable>) -> ProjectSeen {
        ProjectSeen {
            folder: FOLDER.into(),
            root: ROOT.into(),
            working,
        }
    }

    fn reads(global: Option<&str>, head: Option<&str>) -> Reads {
        Reads {
            global: Ok(global.map(|text| file(GLOBAL, text))),
            head: Some(Ok(crate::committed::Committed {
                layer: head.map(|text| file("/r/app/baley.toml", text)),
                pending: None,
            })),
        }
    }

    fn prepare_in_project(
        layer: FileLayer,
        seen: &ProjectSeen,
        reads: &Reads,
    ) -> Result<Prepared, Unavailable> {
        prepare(
            layer,
            Path::new(GLOBAL),
            Some(seen),
            reads,
            Schema::standard(),
        )
    }

    #[test]
    fn a_project_file_set_is_based_on_the_working_tree_not_heads_copy() {
        let edited = format!("escalate_on_failure = true\n{PROJECT_TEXT}");
        let seen = seen(working(&edited));

        let prepared =
            prepare_in_project(FileLayer::Project, &seen, &reads(None, Some(PROJECT_TEXT)))
                .unwrap();

        let base = prepared.base.unwrap();
        assert_eq!(base.bytes, edited.as_bytes());
        assert_eq!(base.digest, file("/r/app/baley.toml", &edited).digest);
        assert_ne!(base.digest, file("/r/app/baley.toml", PROJECT_TEXT).digest);
        assert_eq!(prepared.target, Path::new("/r/app/baley.toml"));
        assert_eq!(
            prepared.project,
            Some(Project {
                folder: FOLDER.into(),
                id: ID.into(),
                root: ROOT.into()
            })
        );
    }

    #[test]
    fn a_global_set_in_a_project_is_refused_for_a_wrong_type_in_heads_copy() {
        let seen = seen(working(PROJECT_TEXT));
        let head = format!("escalate_on_failure = \"yes\"\n{PROJECT_TEXT}");

        let refusal = prepare_in_project(
            FileLayer::Global,
            &seen,
            &reads(Some("escalate_on_failure = true\n"), Some(&head)),
        )
        .unwrap_err();

        let text = refusal.to_string();
        assert!(
            text.starts_with("config-unavailable: HEAD's copy of /r/app/baley.toml:1:"),
            "{text}"
        );
    }

    #[test]
    fn a_project_file_set_is_refused_for_a_wrong_type_in_the_working_tree() {
        let edited = format!("escalate_on_failure = \"yes\"\n{PROJECT_TEXT}");
        let seen = seen(working(&edited));

        let refusal =
            prepare_in_project(FileLayer::Project, &seen, &reads(None, Some(PROJECT_TEXT)))
                .unwrap_err();

        let text = refusal.to_string();
        assert!(
            text.starts_with("config-unavailable: /r/app/baley.toml:1:"),
            "{text}"
        );
    }

    #[test]
    fn a_working_tree_file_with_no_valid_id_is_refused_before_an_unreadable_global_file() {
        let seen = seen(working("[project]\nname = \"sample\"\n"));
        let mut reads = reads(None, None);
        reads.global = Err(Unavailable {
            path: GLOBAL.into(),
            fault: Fault::Unreadable {
                cause: "Permission denied (os error 13)".into(),
            },
        });

        let refusal = prepare_in_project(FileLayer::Global, &seen, &reads).unwrap_err();

        assert_eq!(refusal.path, Path::new("/r/app/baley.toml"));
    }

    #[test]
    fn a_working_tree_read_of_no_file_in_a_project_is_refused_not_a_new_file() {
        let seen = seen(Ok(None));

        let refusal =
            prepare_in_project(FileLayer::Project, &seen, &reads(None, None)).unwrap_err();

        let text = refusal.to_string();
        assert!(text.starts_with("config-unavailable:"), "{text}");
        assert!(
            text.contains("/r/app/baley.toml") && text.contains("not found"),
            "{text}"
        );
    }

    #[test]
    fn a_missing_config_folder_is_created_and_the_file_written() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("a").join("config");
        let target = folder.join("config.toml");

        write_file(&target, b"x = 1\n", None, Some(&folder)).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"x = 1\n");
    }

    #[test]
    fn a_linked_target_is_refused_not_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        let link = dir.path().join("config.toml");
        std::fs::write(&real, b"old = 1\n").unwrap();
        symlink(&real, &link).unwrap();
        let read = settings::read(&link).unwrap().unwrap();
        assert_eq!(read.bytes, b"old = 1\n");

        let failure = write_file(&link, b"new = 2\n", Some(&read.digest), None).unwrap_err();

        assert!(
            matches!(
                failure,
                WriteFailure::Replace(replace::Failure::Refused(replace::Conflict::Link { .. }))
            ),
            "{failure:?}"
        );
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&real).unwrap(), b"old = 1\n");
    }

    #[test]
    fn a_file_changed_after_it_was_read_is_refused_and_keeps_the_new_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        std::fs::write(&target, b"old = 1\n").unwrap();
        let read = settings::read(&target).unwrap().unwrap();
        std::fs::write(&target, b"other = 3\n").unwrap();

        let failure = write_file(&target, b"new = 2\n", Some(&read.digest), None).unwrap_err();

        assert!(
            matches!(
                failure,
                WriteFailure::Replace(replace::Failure::Refused(replace::Conflict::Changed { .. }))
            ),
            "{failure:?}"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"other = 3\n");
    }

    #[test]
    fn an_unchanged_regular_file_is_replaced_with_its_own_digest() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        std::fs::write(&target, b"old = 1\n").unwrap();
        let read = settings::read(&target).unwrap().unwrap();

        write_file(&target, b"new = 2\n", Some(&read.digest), None).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new = 2\n");
    }

    #[test]
    fn a_link_is_refused_by_the_look_and_a_regular_or_missing_file_passes() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        let link = dir.path().join("linked.toml");
        std::fs::write(&real, b"a = 1\n").unwrap();
        symlink(&real, &link).unwrap();

        assert!(matches!(
            refuse_link(&link),
            Err(replace::Conflict::Link { .. })
        ));
        assert!(refuse_link(&real).is_ok());
        assert!(refuse_link(&dir.path().join("missing.toml")).is_ok());
    }

    #[test]
    fn a_refused_conflict_exits_2_and_an_unchanged_failure_exits_3() {
        let refused =
            WriteFailure::Replace(replace::Failure::Refused(replace::Conflict::Changed {
                path: "/c/config.toml".into(),
            }));
        let render = render_failure(&refused);
        assert_eq!((render.code, render.error), (2, true));
        assert!(
            render.lines[0].starts_with("config-conflict:"),
            "{:?}",
            render.lines
        );

        let unchanged = WriteFailure::Replace(replace::Failure::Unchanged {
            path: "/c/config.toml".into(),
            cause: io::Error::other("disk full"),
        });
        let render = render_failure(&unchanged);
        assert_eq!((render.code, render.error), (3, true));
        assert!(render.lines[0].contains("disk full"), "{:?}", render.lines);
    }
}
