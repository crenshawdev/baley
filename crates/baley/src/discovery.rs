//! Finding the checkout's project from a starting directory (design 0003,
//! CFG-R3 and CFG-R4): the nearest `baley.toml` at or below the repository
//! root, never one above it. The command line starts from its working
//! directory. The session server starts from the session's
//! `CLAUDE_PROJECT_DIR` on every project call.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The project file's name in a project's folder.
pub const PROJECT_FILE: &str = "baley.toml";

/// One ancestor of the working directory as it was observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ancestor {
    /// The folder.
    pub path: PathBuf,
    /// Whether `baley.toml` is present or its presence cannot be determined.
    pub has_project_file: bool,
    /// Whether the folder holds a `.git` entry of any kind.
    pub has_git: bool,
}

/// Where a working directory stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discovery {
    /// A project: the folder of the nearest `baley.toml` and the repository root.
    Managed {
        /// The folder holding the nearest `baley.toml` entry, even when it
        /// cannot be read as a settings file.
        folder: PathBuf,
        /// The repository root.
        root: PathBuf,
    },
    /// A repository with no project file at or below the working directory.
    Unmanaged {
        /// The repository root.
        root: PathBuf,
    },
    /// No ancestor holds `.git`.
    Outside,
}

/// Judges the ancestors of a working directory, nearest first. The first
/// ancestor holding `.git` is the root; a file above it never counts, and an
/// inner file is never merged with an outer one.
pub fn discover(ancestors: &[Ancestor]) -> Discovery {
    let mut nearest = None;
    for ancestor in ancestors {
        // The root's own file counts, so it is taken before `.git` stops the walk.
        if nearest.is_none() && ancestor.has_project_file {
            nearest = Some(ancestor.path.clone());
        }
        if ancestor.has_git {
            let root = ancestor.path.clone();
            return match nearest {
                Some(folder) => Discovery::Managed { folder, root },
                None => Discovery::Unmanaged { root },
            };
        }
    }
    Discovery::Outside
}

/// Observes every ancestor of the starting directory, nearest first, for
/// `discover`. `baley init`, `purge`, `config set`, `config show`,
/// `config interview`, `anchor`, `acknowledge-restore`, anchored `verify`
/// and `doctor` are its callers, and so is the session server's preparation
/// of a project call.
///
/// The directory is canonicalized first, so the root is the checkout's
/// canonical path. Both names are checked without following links. Any entry
/// or error other than not-found counts as present, so an unusable settings
/// file cannot hand the project to an outer file. A directory
/// that cannot be canonicalized is an error, not evidence of no repository.
pub fn ancestors(cwd: &Path) -> io::Result<Vec<Ancestor>> {
    let cwd = fs::canonicalize(cwd)?;
    Ok(cwd.ancestors().map(observe).collect())
}

/// Observes just this folder, so the walk's inputs can be gathered separately.
fn observe(path: &Path) -> Ancestor {
    Ancestor {
        path: path.into(),
        has_project_file: entry_present(
            fs::symlink_metadata(path.join(PROJECT_FILE)).map_err(|error| error.kind()),
        ),
        has_git: entry_present(
            fs::symlink_metadata(path.join(".git")).map_err(|error| error.kind()),
        ),
    }
}

/// Only not-found proves absence. The entry's kind never changes discovery.
fn entry_present<T>(observed: Result<T, io::ErrorKind>) -> bool {
    !matches!(observed, Err(io::ErrorKind::NotFound))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_not_found_is_absent_and_unusable_entries_still_bind() {
        assert!(!entry_present::<()>(Err(io::ErrorKind::NotFound)));
        for kind in ["regular file", "directory", "dangling link"] {
            assert!(entry_present(Ok(kind)), "{kind}");
        }
        for error in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            assert!(entry_present::<()>(Err(error)), "{error:?}");
        }
    }

    #[test]
    fn a_directory_named_baley_toml_does_not_fall_back_to_the_outer_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(PROJECT_FILE), b"outer").unwrap();
        fs::create_dir_all(root.join("a/baley.toml")).unwrap();

        assert_eq!(
            discover(&[observe(&root.join("a")), observe(&root)]),
            Discovery::Managed {
                folder: root.join("a"),
                root,
            }
        );
    }

    #[test]
    fn a_symlink_to_a_regular_project_file_still_binds() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join("settings.toml"), b"project").unwrap();
        std::os::unix::fs::symlink("settings.toml", root.join(PROJECT_FILE)).unwrap();

        assert_eq!(
            discover(&[observe(&root)]),
            Discovery::Managed {
                folder: root.clone(),
                root,
            }
        );
    }

    fn at(path: &str, has_project_file: bool, has_git: bool) -> Ancestor {
        Ancestor {
            path: path.into(),
            has_project_file,
            has_git,
        }
    }

    #[test]
    fn a_subdirectory_does_not_miss_the_file_at_the_root() {
        let ancestors = [
            at("/r/a/b", false, false),
            at("/r/a", false, false),
            at("/r", true, true),
            at("/", false, false),
        ];
        assert_eq!(
            discover(&ancestors),
            Discovery::Managed {
                folder: "/r".into(),
                root: "/r".into(),
            }
        );
    }

    #[test]
    fn the_nearer_file_wins_and_the_outer_one_is_not_taken() {
        let ancestors = [
            at("/r/a/b", false, false),
            at("/r/a", true, false),
            at("/r", true, true),
            at("/", false, false),
        ];
        assert_eq!(
            discover(&ancestors),
            Discovery::Managed {
                folder: "/r/a".into(),
                root: "/r".into(),
            }
        );
    }

    #[test]
    fn the_walk_does_not_pass_the_first_git_to_a_file_above_it() {
        // An outer repository above holds a file; the inner root still has none.
        let ancestors = [
            at("/h/o/r/sub", false, false),
            at("/h/o/r", false, true),
            at("/h/o", true, false),
            at("/h", false, true),
            at("/", false, false),
        ];
        assert_eq!(
            discover(&ancestors),
            Discovery::Unmanaged {
                root: "/h/o/r".into(),
            }
        );
    }

    #[test]
    fn a_repository_without_a_file_is_unmanaged_not_outside() {
        let ancestors = [
            at("/r/a", false, false),
            at("/r", false, true),
            at("/", false, false),
        ];
        assert_eq!(
            discover(&ancestors),
            Discovery::Unmanaged { root: "/r".into() }
        );
    }

    #[test]
    fn a_file_without_a_repository_is_outside_not_managed() {
        let ancestors = [
            at("/p/a", false, false),
            at("/p", true, false),
            at("/", false, false),
        ];
        assert_eq!(discover(&ancestors), Discovery::Outside);
    }

    #[test]
    fn the_roots_own_file_is_not_skipped_when_git_sits_beside_it() {
        let ancestors = [at("/r", true, true), at("/", false, false)];
        assert_eq!(
            discover(&ancestors),
            Discovery::Managed {
                folder: "/r".into(),
                root: "/r".into(),
            }
        );
    }
}
