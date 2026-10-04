//! Resolves a path spelling against the hook's working directory into the
//! canonical path of its deepest existing prefix plus the missing components
//! below it. Gathering sits behind [`Lookup`]; the walk judges what it sees.

use std::fmt;
use std::io::{Error, ErrorKind};
use std::path::{Component, Path, PathBuf};

/// What the filesystem answers about one path, asked one lookup at a time.
pub trait Lookup {
    /// As `std::fs::metadata`: follows a symlink.
    fn metadata(&self, path: &Path) -> std::io::Result<Entry>;
    /// As `std::fs::symlink_metadata`: whether anything, a symlink included,
    /// is at the path.
    fn present(&self, path: &Path) -> std::io::Result<()>;
    /// As `std::fs::canonicalize`.
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
    /// As `std::path::absolute`: a relative path is taken from the process's
    /// directory.
    fn absolute(&self, path: &Path) -> std::io::Result<PathBuf>;
}

/// What a followed lookup says about one path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Whether the path is a directory.
    pub directory: bool,
    /// Device and inode: two paths with one identity are one file.
    pub identity: (u64, u64),
}

/// The real filesystem.
pub struct Disk;

impl Lookup for Disk {
    fn metadata(&self, path: &Path) -> std::io::Result<Entry> {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).map(|metadata| Entry {
            directory: metadata.is_dir(),
            identity: (metadata.dev(), metadata.ino()),
        })
    }

    fn present(&self, path: &Path) -> std::io::Result<()> {
        std::fs::symlink_metadata(path).map(|_| ())
    }

    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn absolute(&self, path: &Path) -> std::io::Result<PathBuf> {
        std::path::absolute(path)
    }
}

/// Which of the two inputs a refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The hook's working directory.
    Cwd,
    /// The path spelling.
    Target,
}

impl fmt::Display for Part {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(match self {
            Part::Cwd => "cwd",
            Part::Target => "target",
        })
    }
}

/// Why a path cannot be resolved safely. A decision that meets one of these
/// refuses, since a path it cannot place is a path it cannot clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveFailure {
    /// The value is empty or holds a control byte.
    BadText(Part),
    /// The working directory is not absolute.
    RelativeCwd,
    /// The working directory could not be resolved.
    CwdUnresolved(String),
    /// The working directory is not a directory.
    CwdNotDirectory,
    /// The spelling starts `//` or with a drive letter, which different
    /// systems read differently.
    AmbiguousPrefix,
    /// A parent step leaves the filesystem root.
    AboveRoot,
    /// A component below a file.
    NonDirectoryParent,
    /// A parent folder could not be inspected.
    Parent(String),
    /// Whether a component exists could not be told.
    Present(String),
    /// An existing component could not be resolved.
    Canonicalize(String),
    /// The identity of an existing path could not be read.
    Identity(String),
    /// A supplied protected path is not absolute.
    NotAbsolute,
}

impl fmt::Display for ResolveFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ResolveFailure::BadText(part) => {
                write!(formatter, "the {part} is empty or contains control bytes")
            }
            ResolveFailure::RelativeCwd => formatter.write_str("the cwd is not an absolute path"),
            ResolveFailure::CwdUnresolved(error) => {
                write!(formatter, "the cwd cannot be resolved: {error}")
            }
            ResolveFailure::CwdNotDirectory => formatter.write_str("the cwd is not a directory"),
            ResolveFailure::AmbiguousPrefix => {
                formatter.write_str("the path starts with an ambiguous prefix")
            }
            ResolveFailure::AboveRoot => {
                formatter.write_str("the path steps above the filesystem root")
            }
            ResolveFailure::NonDirectoryParent => {
                formatter.write_str("the path goes through a file")
            }
            ResolveFailure::Parent(error) => {
                write!(
                    formatter,
                    "a parent folder cannot be inspected safely: {error}"
                )
            }
            ResolveFailure::Present(error) => {
                write!(
                    formatter,
                    "a path component cannot be inspected safely: {error}"
                )
            }
            ResolveFailure::Canonicalize(error) => {
                write!(formatter, "a path component cannot be resolved: {error}")
            }
            ResolveFailure::Identity(error) => {
                write!(formatter, "a path cannot be identified safely: {error}")
            }
            ResolveFailure::NotAbsolute => formatter.write_str("a protected path is not absolute"),
        }
    }
}

/// Resolves `target` against the hook's `cwd`: the canonical path of the
/// deepest existing prefix, with the missing components after it as spelled.
pub fn resolve_target(cwd: &str, target: &str, fs: &dyn Lookup) -> Result<PathBuf, ResolveFailure> {
    let cwd = canonical_cwd(cwd, fs)?;
    resolve_under(&cwd, target, fs)
}

/// The working directory as a canonical, existing directory.
pub fn canonical_cwd(cwd: &str, fs: &dyn Lookup) -> Result<PathBuf, ResolveFailure> {
    validate_text(cwd, Part::Cwd)?;
    let cwd = Path::new(cwd);
    if !cwd.is_absolute() {
        return Err(ResolveFailure::RelativeCwd);
    }
    let cwd = fs
        .canonicalize(cwd)
        .map_err(|error| ResolveFailure::CwdUnresolved(error.to_string()))?;
    if !fs.metadata(&cwd).is_ok_and(|entry| entry.directory) {
        return Err(ResolveFailure::CwdNotDirectory);
    }
    Ok(cwd)
}

/// Resolves `spelling` from `base`, an absolute directory. An absolute
/// spelling ignores `base`.
pub fn resolve_under(
    base: &Path,
    spelling: &str,
    fs: &dyn Lookup,
) -> Result<PathBuf, ResolveFailure> {
    validate_text(spelling, Part::Target)?;
    if spelling.starts_with("//") || spelling.as_bytes().get(1).is_some_and(|byte| *byte == b':') {
        return Err(ResolveFailure::AmbiguousPrefix);
    }
    let supplied = Path::new(spelling);
    let joined = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        base.join(supplied)
    };
    resolve_existing_prefix(&joined, fs)
}

fn validate_text(value: &str, part: Part) -> Result<(), ResolveFailure> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(ResolveFailure::BadText(part));
    }
    Ok(())
}

/// The canonical path of the deepest existing prefix of an absolute path, with
/// the missing components after it. A parent step is taken against what the
/// prefix resolved to, so a step after a linked folder leaves the link's
/// target and not the spelled folder.
pub fn resolve_existing_prefix(path: &Path, fs: &dyn Lookup) -> Result<PathBuf, ResolveFailure> {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) => return Err(ResolveFailure::AmbiguousPrefix),
            Component::RootDir => resolved.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() {
                    return Err(ResolveFailure::AboveRoot);
                }
            }
            Component::Normal(value) => {
                match fs.metadata(&resolved) {
                    Ok(entry) if !entry.directory => {
                        return Err(ResolveFailure::NonDirectoryParent);
                    }
                    Ok(_) => {}
                    Err(error) if is_missing(&error) => {}
                    Err(error) => return Err(ResolveFailure::Parent(error.to_string())),
                }
                resolved.push(value);
                match fs.present(&resolved) {
                    Ok(()) => {
                        resolved = fs
                            .canonicalize(&resolved)
                            .map_err(|error| ResolveFailure::Canonicalize(error.to_string()))?;
                    }
                    Err(error) if is_missing(&error) => {}
                    Err(error) => return Err(ResolveFailure::Present(error.to_string())),
                }
            }
        }
    }
    Ok(resolved)
}

fn is_missing(error: &Error) -> bool {
    error.kind() == ErrorKind::NotFound
}
