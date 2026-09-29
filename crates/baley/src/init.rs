//! `baley init`: ties a repository to a ledger project (design 0001, EVD-R17).
//! The judges here take plain values; the command gathers them.
use std::fmt;
use std::path::{Path, PathBuf};

use baley_core::policy::{ProjectIdentity, Unavailable, render_project};

use crate::discovery::{Discovery, PROJECT_FILE};
use crate::replace;

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
}
