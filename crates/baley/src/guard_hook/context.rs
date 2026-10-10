//! Where the hook runs: the session project from
//! `CLAUDE_PROJECT_DIR`, the checkout the hook's cwd is in, Baley's folders
//! and the installed paths derived from HOME and CLAUDE_CONFIG_DIR. The cwd
//! never stands in for the project, so a `/cd` never changes whose policy
//! applies.

use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::folders::{Environment, FolderRefusal, Folders, Platform};
use crate::host_artifacts::{installed, stubs};
use crate::mcp::context::{DirectoryFault, ProjectContext, project_context};
use crate::protected_paths::ProtectedPaths;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

/// The session project, bound because its directory is in a managed
/// checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Bound {
    /// The folder holding the project's `baley.toml`.
    pub folder: PathBuf,
    /// The canonical repository root.
    pub root: PathBuf,
}

/// What the hook knows about where it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HookContext {
    /// `CLAUDE_PROJECT_DIR` as given whenever it is set, valid or not, and
    /// `None` when unset. Never canonicalized, so a record carries the host's
    /// own text. A value that is not UTF-8 cannot be recorded as given.
    pub project_directory: Result<Option<String>, DirectoryFault>,
    /// The session project, or `None` when nothing is bound.
    pub project: Option<Bound>,
    /// The root of the cwd's checkout, used for remembered denials when a
    /// commit has no redirect. A redirected commit supplies its own walk.
    pub checkout: Option<PathBuf>,
    /// The paths path tools are judged against, or why Baley's folders or
    /// installed artifact paths could not be resolved.
    pub protected: Result<ProtectedPaths, Refusal>,
}

/// Why the guard cannot establish the complete set of protected paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Refusal {
    /// Baley's ledger home or configuration folder could not be resolved.
    Folders(FolderRefusal),
    /// The installed Claude files or executable could not be placed.
    Placements(installed::Refusal),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Folders(refusal) => refusal.fmt(f),
            Self::Placements(refusal) => refusal.fmt(f),
        }
    }
}

/// A directory's discovery, or why its walk failed. A failed walk is not
/// evidence of no repository.
pub(super) type Walk = Result<Discovery, String>;

/// Reads the environment and walks the project directory and the cwd. It
/// owns no policy, so it has no unit test.
pub(super) fn gather(cwd: &str) -> HookContext {
    let project_dir = std::env::var_os("CLAUDE_PROJECT_DIR");
    let is_directory = project_dir
        .as_deref()
        .is_some_and(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_dir()));
    let project = project_context(project_dir.as_deref(), is_directory);
    let project_walk = match &project {
        ProjectContext::Valid(directory) => Some(walk(Path::new(directory))),
        ProjectContext::Missing | ProjectContext::Invalid(_) => None,
    };
    let env = Environment::read();
    let folders = Folders::resolve(Platform::current(), &env);
    let claude_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
    let manifest = stubs::manifest(&stubs::front_doors()).expect("unique compiled front doors");
    let installed = installed::resolve(&env, claude_config_dir, &manifest);
    judge(
        project_dir.as_deref(),
        &project,
        project_walk,
        walk(Path::new(cwd)),
        folders,
        installed.map(Some),
    )
}

/// Walks a directory for the session context or a commit's target checkout.
pub(super) fn walk(directory: &Path) -> Walk {
    // A relative path would be walked from this process's own directory.
    if !directory.is_absolute() {
        return Err(format!("{} is not an absolute path", directory.display()));
    }
    discovery::ancestors(directory)
        .map(|ancestors| discovery::discover(&ancestors))
        .map_err(|error| format!("{} cannot be walked: {error}", directory.display()))
}

/// Judges the hook's context. The project is bound only when
/// `CLAUDE_PROJECT_DIR` is valid and its walk finds a managed checkout. The
/// cwd's walk gives the checkout and never the project.
///
/// The protected files are the bound project's `baley.toml` and the one that
/// binds the cwd's checkout: its project file when managed, or the root's
/// missing `baley.toml` when unmanaged, so an agent cannot create the file
/// that would bind it. Supplied installed placements follow, each file once
/// in first-seen order, with the versions folder protected against writes.
/// A folder or placement refusal leaves no protected paths to judge against.
pub(super) fn judge(
    project_dir: Option<&OsStr>,
    project: &ProjectContext,
    project_walk: Option<Walk>,
    cwd_walk: Walk,
    folders: Result<Folders, FolderRefusal>,
    installed: Result<Option<installed::Installed>, installed::Refusal>,
) -> HookContext {
    let project = match (project, project_walk) {
        (ProjectContext::Valid(_), Some(Ok(Discovery::Managed { folder, root }))) => {
            Some(Bound { folder, root })
        }
        _ => None,
    };
    let (checkout, checkout_file) = match cwd_walk {
        Ok(Discovery::Managed { folder, root }) => (Some(root), Some(folder.join(PROJECT_FILE))),
        Ok(Discovery::Unmanaged { root }) => {
            let file = root.join(PROJECT_FILE);
            (Some(root), Some(file))
        }
        Ok(Discovery::Outside) | Err(_) => (None, None),
    };
    let mut files: Vec<PathBuf> = project
        .iter()
        .map(|bound| bound.folder.join(PROJECT_FILE))
        .collect();
    if let Some(file) = checkout_file
        && !files.contains(&file)
    {
        files.push(file);
    }
    let project_directory = project_dir
        .map(|text| {
            text.to_str()
                .map(str::to_owned)
                .ok_or(DirectoryFault::NotUtf8)
        })
        .transpose();
    HookContext {
        project_directory,
        project,
        checkout,
        protected: folders.map_err(Refusal::Folders).and_then(|folders| {
            let installed = installed.map_err(Refusal::Placements)?;
            let mut write_only_folders = Vec::new();
            if let Some(installed) = installed {
                for file in installed.placements.protected_paths() {
                    if !files.contains(&file) {
                        files.push(file);
                    }
                }
                write_only_folders = installed.placements.write_only_folders();
            }
            Ok(ProtectedPaths {
                home: folders.home,
                config: folders.config,
                files,
                write_only_folders,
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> Result<Folders, FolderRefusal> {
        Ok(Folders {
            config: "/u/.config/crenshawdev/baley".into(),
            home: "/u/.local/share/crenshawdev/baley".into(),
        })
    }

    fn managed(path: &str) -> Walk {
        Ok(Discovery::Managed {
            folder: path.into(),
            root: path.into(),
        })
    }

    fn unmanaged(path: &str) -> Walk {
        Ok(Discovery::Unmanaged { root: path.into() })
    }

    fn protecting(files: &[&str]) -> Result<ProtectedPaths, Refusal> {
        Ok(ProtectedPaths {
            home: "/u/.local/share/crenshawdev/baley".into(),
            config: "/u/.config/crenshawdev/baley".into(),
            files: files.iter().map(PathBuf::from).collect(),
            write_only_folders: Vec::new(),
        })
    }

    fn bound(path: &str) -> Option<Bound> {
        Some(Bound {
            folder: path.into(),
            root: path.into(),
        })
    }

    #[test]
    fn the_installed_placements_left_off_or_repeated_in_the_protected_list_is_caught() {
        use crate::host_artifacts::{installed, stubs};

        let env = Environment {
            home: Some("/u".into()),
            ..Environment::default()
        };
        let installed =
            installed::resolve(&env, None, &stubs::manifest(&stubs::front_doors()).unwrap())
                .unwrap();
        let placed = [
            "/u/.claude/skills/bal-capture/SKILL.md",
            "/u/.claude/skills/bal-help/SKILL.md",
            "/u/.claude.json",
            "/u/.claude/settings.json",
            "/u/.local/bin/baley",
        ];
        for (given, project, project_walk, cwd_walk, project_files) in [
            (
                Some(OsStr::new("/p")),
                ProjectContext::Valid("/p".into()),
                Some(managed("/p")),
                managed("/q"),
                vec!["/p/baley.toml", "/q/baley.toml"],
            ),
            (
                None,
                ProjectContext::Missing,
                None,
                Ok(Discovery::Outside),
                vec![],
            ),
        ] {
            let context = judge(
                given,
                &project,
                project_walk,
                cwd_walk,
                folders(),
                Ok(Some(installed.clone())),
            );
            let protected = context.protected.unwrap();
            let expected: Vec<PathBuf> = project_files
                .into_iter()
                .chain(placed)
                .map(PathBuf::from)
                .collect();
            assert_eq!(protected.files, expected);
            assert_eq!(
                protected.write_only_folders,
                [PathBuf::from("/u/.local/lib/crenshawdev/baley/versions")]
            );
        }
    }

    #[test]
    fn the_cwd_taking_over_the_session_project_is_caught() {
        let context = judge(
            Some(OsStr::new("/p")),
            &ProjectContext::Valid("/p".into()),
            Some(managed("/p")),
            managed("/q"),
            folders(),
            Ok(None),
        );
        assert_eq!(
            context,
            HookContext {
                project_directory: Ok(Some("/p".into())),
                project: bound("/p"),
                checkout: Some("/q".into()),
                protected: protecting(&["/p/baley.toml", "/q/baley.toml"]),
            }
        );
    }

    #[test]
    fn the_cwd_standing_in_for_a_missing_or_invalid_project_or_losing_its_protection_is_caught() {
        for (given, project) in [
            (None, ProjectContext::Missing),
            (Some("p"), ProjectContext::Invalid(DirectoryFault::Relative)),
            (
                Some("/p/file"),
                ProjectContext::Invalid(DirectoryFault::NotADirectory),
            ),
        ] {
            let context = judge(
                given.map(OsStr::new),
                &project,
                None,
                managed("/q"),
                folders(),
                Ok(None),
            );
            assert_eq!(
                context,
                HookContext {
                    project_directory: Ok(given.map(str::to_owned)),
                    project: None,
                    checkout: Some("/q".into()),
                    protected: protecting(&["/q/baley.toml"]),
                },
                "{project:?}"
            );
        }
    }

    #[test]
    fn a_project_directory_that_is_not_utf8_read_as_unset_is_caught() {
        use std::os::unix::ffi::OsStrExt;

        let context = judge(
            Some(OsStr::from_bytes(b"/p\xff")),
            &ProjectContext::Invalid(DirectoryFault::NotUtf8),
            None,
            managed("/q"),
            folders(),
            Ok(None),
        );
        assert_eq!(context.project_directory, Err(DirectoryFault::NotUtf8));
        assert_eq!(context.project, None);
    }

    #[test]
    fn an_unmanaged_project_directory_taken_as_bound_is_caught() {
        let context = judge(
            Some(OsStr::new("/p")),
            &ProjectContext::Valid("/p".into()),
            Some(unmanaged("/p")),
            managed("/q"),
            folders(),
            Ok(None),
        );
        assert_eq!(context.project, None);
        assert_eq!(context.checkout, Some("/q".into()));
    }

    #[test]
    fn a_missing_checkout_file_left_off_the_protected_list_is_caught() {
        let context = judge(
            Some(OsStr::new("/p")),
            &ProjectContext::Valid("/p".into()),
            Some(managed("/p")),
            unmanaged("/q"),
            folders(),
            Ok(None),
        );
        assert_eq!(context.project, bound("/p"));
        assert_eq!(context.checkout, Some("/q".into()));
        assert_eq!(
            context.protected,
            protecting(&["/p/baley.toml", "/q/baley.toml"])
        );
    }

    #[test]
    fn a_folders_refusal_replaced_by_made_up_folders_is_caught() {
        let context = judge(
            Some(OsStr::new("/p")),
            &ProjectContext::Valid("/p".into()),
            Some(managed("/p")),
            managed("/q"),
            Err(FolderRefusal::UserHomeUnset),
            Ok(None),
        );
        assert_eq!(
            context,
            HookContext {
                project_directory: Ok(Some("/p".into())),
                project: bound("/p"),
                checkout: Some("/q".into()),
                protected: Err(Refusal::Folders(FolderRefusal::UserHomeUnset)),
            }
        );
    }
}
