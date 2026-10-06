//! Where the hook runs (design 0010, GRD-R2): the session project from
//! `CLAUDE_PROJECT_DIR`, the checkout the hook's cwd is in, Baley's folders
//! and the paths the guard protects. The cwd never stands in for the project,
//! so a `/cd` never changes whose policy applies.

use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::folders::{Environment, FolderRefusal, Folders, Platform};
use crate::mcp::context::{DirectoryFault, ProjectContext, project_context};
use crate::protected_paths::ProtectedPaths;
use std::ffi::OsStr;
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
    /// The root of the checkout the cwd is in, whose branch the hook reads,
    /// and the checkout half of the remembered-policy key.
    pub checkout: Option<PathBuf>,
    /// The paths path tools are judged against, or why Baley's folders could
    /// not be resolved.
    pub protected: Result<ProtectedPaths, FolderRefusal>,
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
    let folders = Folders::resolve(Platform::current(), &Environment::read());
    judge(
        project_dir.as_deref(),
        &project,
        project_walk,
        walk(Path::new(cwd)),
        folders,
    )
}

fn walk(directory: &Path) -> Walk {
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
/// that would bind it. Stubs are not listed yet.
pub(super) fn judge(
    project_dir: Option<&OsStr>,
    project: &ProjectContext,
    project_walk: Option<Walk>,
    cwd_walk: Walk,
    folders: Result<Folders, FolderRefusal>,
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
        protected: folders.map(|folders| ProtectedPaths {
            home: folders.home,
            config: folders.config,
            files,
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

    fn protecting(files: &[&str]) -> Result<ProtectedPaths, FolderRefusal> {
        Ok(ProtectedPaths {
            home: "/u/.local/share/crenshawdev/baley".into(),
            config: "/u/.config/crenshawdev/baley".into(),
            files: files.iter().map(PathBuf::from).collect(),
        })
    }

    fn bound(path: &str) -> Option<Bound> {
        Some(Bound {
            folder: path.into(),
            root: path.into(),
        })
    }

    #[test]
    fn the_cwd_taking_over_the_session_project_is_caught() {
        let context = judge(
            Some(OsStr::new("/p")),
            &ProjectContext::Valid("/p".into()),
            Some(managed("/p")),
            managed("/q"),
            folders(),
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
        );
        assert_eq!(
            context,
            HookContext {
                project_directory: Ok(Some("/p".into())),
                project: bound("/p"),
                checkout: Some("/q".into()),
                protected: Err(FolderRefusal::UserHomeUnset),
            }
        );
    }
}
