//! The read decision for Read, Grep and Glob (design 0010, GRD-R11). A read
//! is refused when what it reaches lies inside, or holds, Baley's home or
//! config folder. Protected files such as `baley.toml` are not read-protected.

use super::contain::{contains, is_inside};
use super::resolve::{Lookup, ResolveFailure, canonical_cwd, resolve_under};
use super::write::{ProtectedPaths, resolve_entry};
use baley_core::guard::Answer;

/// Characters that can make a pattern component match more than its own
/// text. Counting one too many only shortens the fixed prefix, which widens
/// what is checked, so a wrong member of this set fails closed.
const WILDCARDS: [char; 5] = ['*', '?', '[', '{', '\\'];

/// Judges one read.
///
/// `path` is Read's `file_path`, or Grep's or Glob's `path`. Without one the
/// search starts in the hook's `cwd`. `pattern` is Glob's `pattern` or Grep's
/// `glob`: the folders it names before its first wildcard are checked the same
/// way, and a `..` after a wildcard refuses the call, since where it leads
/// cannot be told. A target that cannot be resolved is refused.
pub fn read_answer(
    cwd: &str,
    path: Option<&str>,
    pattern: Option<&str>,
    protected: &ProtectedPaths,
    fs: &dyn Lookup,
) -> Answer {
    match refused_by(cwd, path, pattern, protected, fs) {
        Ok(None) => Answer::Pass,
        Ok(Some(reason)) => Answer::Deny(reason),
        Err(failure) => Answer::Deny(format!(
            "Baley cannot tell whether this read reaches its protected folders ({failure}), so it is refused"
        )),
    }
}

/// Why the read is refused, or `None` when it is not.
fn refused_by(
    cwd: &str,
    path: Option<&str>,
    pattern: Option<&str>,
    protected: &ProtectedPaths,
    fs: &dyn Lookup,
) -> Result<Option<String>, ResolveFailure> {
    let cwd = canonical_cwd(cwd, fs)?;
    let target = match path {
        Some(path) => resolve_under(&cwd, path, fs)?,
        None => cwd,
    };
    let mut reached = vec![target.clone()];
    if let Some(pattern) = pattern {
        match fixed_prefix(pattern) {
            Fixed::Escapes => {
                return Ok(Some(format!(
                    "the pattern {pattern} steps up a folder after a wildcard, so where it reaches cannot be told, and the read is refused"
                )));
            }
            Fixed::Folders(Some(fixed)) => reached.push(resolve_under(&target, &fixed, fs)?),
            Fixed::Folders(None) => {}
        }
    }
    for resolved in &reached {
        for (name, folder) in [("home", &protected.home), ("config", &protected.config)] {
            let folder = resolve_entry(folder, fs)?;
            if is_inside(resolved, &folder, fs)? || contains(resolved, &folder, fs)? {
                return Ok(Some(format!(
                    "Baley protects its {name} folder ({}), and this read reaches {}, which is inside it or holds it, so it is refused",
                    folder.display(),
                    resolved.display()
                )));
            }
        }
    }
    Ok(None)
}

/// What a pattern's fixed folders come to.
#[derive(Debug, PartialEq, Eq)]
enum Fixed {
    /// The components before the first wildcard, joined, or none when the
    /// pattern is relative and starts with a wildcard.
    Folders(Option<String>),
    /// A `..` follows a wildcard.
    Escapes,
}

/// Splits a pattern on `/` and keeps the components before the first one that
/// holds a wildcard. A pattern with no wildcard is fixed throughout.
fn fixed_prefix(pattern: &str) -> Fixed {
    let parts: Vec<&str> = pattern.split('/').collect();
    let wild = parts
        .iter()
        .position(|part| part.contains(WILDCARDS))
        .unwrap_or(parts.len());
    if parts[wild..].contains(&"..") {
        return Fixed::Escapes;
    }
    let fixed = parts[..wild].join("/");
    Fixed::Folders(match (fixed.is_empty(), pattern.starts_with('/')) {
        (false, _) => Some(fixed),
        (true, true) => Some("/".to_owned()),
        (true, false) => None,
    })
}
