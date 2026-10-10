//! The write decision for Write, Edit and NotebookEdit (design 0010, GRD-R11).
//! It judges the target path alone, so it gives the same answer whether or
//! not a project is bound and from any working directory.

use super::contain::{is_inside, names_missing_file};
use super::resolve::{
    Lookup, ResolveFailure, canonical_cwd, resolve_existing_prefix, resolve_under,
};
use baley_core::guard::Answer;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// What the guard protects, supplied by the caller as absolute paths.
///
/// Every entry must be absolute. A relative entry, or one that cannot be
/// resolved, makes the decision deny, so a bad list fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedPaths {
    /// Baley's ledger home folder, protected whole.
    pub home: PathBuf,
    /// Baley's configuration folder, protected whole.
    pub config: PathBuf,
    /// Files protected one by one: the session project's `baley.toml`, the
    /// `baley.toml` of the checkout the hook's cwd is in, and the installed
    /// stubs, settings file, registration file and executable. Any other
    /// file named `baley.toml` is not on the list.
    pub files: Vec<PathBuf>,
    /// Folders protected against writes only, such as the folder of staged
    /// versions: a write inside one is denied, a read is not, because the
    /// binaries inside must stay readable and runnable. Build 3 T15 fills it
    /// from the placement map's write-only folders, beside `files`.
    pub write_only_folders: Vec<PathBuf>,
}

/// What the write decision knows about the dispatch that is running.
///
/// Build 3 has one value. Build 5 adds what a lease names and the deny for a
/// write outside it (EXE-R8, and ADR 0033's check at close), so no lease
/// grammar is defined here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lease {
    /// No dispatch is active, so no lease narrows or widens a write.
    NoActiveDispatch,
}

/// Judges one write target: a `file_path` or a `notebook_path`.
///
/// It denies a target inside or equal to the home or config folder or to a
/// write-only folder (the staged versions), and a target that is the same
/// destination as a protected file by path or by identity. Reads are not
/// judged here, so a write-only folder stays readable. While a protected file
/// does not exist, a spelling that would create it on a case-insensitive
/// volume is that destination too. It resolves both the spelling as given
/// and the spelling with each backslash read as a slash, and denies when
/// either lands on a protected path or when a path cannot be resolved.
pub fn write_answer(
    cwd: &str,
    target: &str,
    protected: &ProtectedPaths,
    lease: &Lease,
    fs: &dyn Lookup,
) -> Answer {
    match lease {
        Lease::NoActiveDispatch => {}
    }
    match protected_by(cwd, target, protected, fs) {
        Ok(None) => Answer::Pass,
        Ok(Some(reason)) => Answer::Deny(reason),
        Err(failure) => Answer::Deny(format!(
            "Baley cannot tell whether {target} is a protected path ({failure}), so the write is refused"
        )),
    }
}

/// Why the target is protected, or `None` when it is not.
fn protected_by(
    cwd: &str,
    target: &str,
    protected: &ProtectedPaths,
    fs: &dyn Lookup,
) -> Result<Option<String>, ResolveFailure> {
    let cwd = canonical_cwd(cwd, fs)?;
    // A POSIX backslash can be a real name, so the native spelling goes first.
    for spelling in [target.to_owned(), target.replace('\\', "/")] {
        let resolved = resolve_under(&cwd, &spelling, fs)?;
        for (name, folder) in [("home", &protected.home), ("config", &protected.config)] {
            let folder = resolve_entry(folder, fs)?;
            if is_inside(&resolved, &folder, fs)? {
                return Ok(Some(format!(
                    "Baley protects its {name} folder ({}), and {} is inside it, so it is not changed through a tool call",
                    folder.display(),
                    resolved.display()
                )));
            }
        }
        for folder in &protected.write_only_folders {
            let folder = resolve_entry(folder, fs)?;
            if is_inside(&resolved, &folder, fs)? {
                return Ok(Some(format!(
                    "Baley keeps its staged versions in {}, and {} is inside it, so they are not changed through a tool call",
                    folder.display(),
                    resolved.display()
                )));
            }
        }
        for file in &protected.files {
            let file = resolve_entry(file, fs)?;
            if same_destination(&resolved, &file, fs)? {
                return Ok(Some(format!(
                    "Baley protects {}, and {} is the same file, so it is not changed through a tool call",
                    file.display(),
                    resolved.display()
                )));
            }
        }
    }
    Ok(None)
}

/// A protected entry as its canonical path, whether or not it exists yet.
pub(super) fn resolve_entry(entry: &Path, fs: &dyn Lookup) -> Result<PathBuf, ResolveFailure> {
    if !entry.is_absolute() {
        return Err(ResolveFailure::NotAbsolute);
    }
    resolve_existing_prefix(entry, fs)
}

/// Whether two resolved paths are one file: the same path, the same
/// (device, inode) when both exist, or, when the protected `destination`
/// does not exist yet, a spelling that would create it on a case-insensitive
/// volume (see [`names_missing_file`]).
fn same_destination(
    target: &Path,
    destination: &Path,
    fs: &dyn Lookup,
) -> Result<bool, ResolveFailure> {
    if target == destination {
        return Ok(true);
    }
    let identity = |path: &Path| match fs.metadata(path) {
        Ok(entry) => Ok(Some(entry.identity)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ResolveFailure::Identity(error.to_string())),
    };
    match (identity(target)?, identity(destination)?) {
        (Some(left), Some(right)) => Ok(left == right),
        (_, None) => names_missing_file(target, destination, fs),
        (None, Some(_)) => Ok(false),
    }
}
