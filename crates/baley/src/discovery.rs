//! Finding the checkout's project from a working directory (design 0003,
//! CFG-R3 and CFG-R4): the nearest `baley.toml` at or below the repository
//! root, never one above it.
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
    /// Whether the folder holds a regular `baley.toml`.
    pub has_project_file: bool,
    /// Whether the folder holds a `.git` entry of any kind.
    pub has_git: bool,
}

/// Where a working directory stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discovery {
    /// A project: the folder of the nearest `baley.toml` and the repository root.
    Managed {
        /// The folder holding the project file.
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

/// Observes every ancestor of the working directory, nearest first, for
/// `discover`. `baley init`, `purge`, `config set`, `config show`,
/// `config interview`, `anchor`, `acknowledge-restore`, anchored `verify`
/// and `doctor` are its callers.
///
/// The directory is canonicalized first, so the root is the checkout's
/// canonical path. `baley.toml` is followed through a link, as settings reads
/// are; `.git` is only checked for presence, never followed or read, so a
/// linked worktree's `.git` file marks a root as a directory does. A directory
/// that cannot be canonicalized is an error, not evidence of no repository.
pub fn ancestors(cwd: &Path) -> io::Result<Vec<Ancestor>> {
    let cwd = fs::canonicalize(cwd)?;
    Ok(cwd
        .ancestors()
        .map(|path| Ancestor {
            path: path.into(),
            has_project_file: fs::metadata(path.join(PROJECT_FILE))
                .is_ok_and(|meta| meta.is_file()),
            has_git: fs::symlink_metadata(path.join(".git")).is_ok(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

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
