//! Whether one resolved path lies inside, or contains, a protected folder.
//! Comparing components alone would miss a case-variant spelling on a
//! case-insensitive volume, so the (device, inode) identity of existing
//! ancestors is compared too.

use super::resolve::{Lookup, ResolveFailure};
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::Path;

type Identity = (u64, u64);

/// Whether `path` is `folder` or lies inside it.
///
/// It compares components, so `/h/baley-old` is not inside `/h/baley`, and
/// then compares identities: `path`, or one of its existing ancestors, has
/// the folder's identity. A folder that does not exist yet is judged by its
/// deepest existing ancestor and the components missing below it. Those
/// compare ASCII case-insensitively, since a differently cased spelling of a
/// folder not yet created becomes that folder on a case-insensitive volume.
/// On a case-sensitive one that refuses a sibling it need not, which is the
/// accepted cost.
pub fn is_inside(path: &Path, folder: &Path, fs: &dyn Lookup) -> Result<bool, ResolveFailure> {
    if path.starts_with(folder) {
        return Ok(true);
    }
    let Some((anchor, missing)) = deepest_existing(folder, fs)? else {
        return Ok(false);
    };
    for ancestor in path.ancestors() {
        if identity(ancestor, fs)? != Some(anchor) {
            continue;
        }
        if let Ok(rest) = path.strip_prefix(ancestor)
            && begins_with(rest, &missing)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `path` is `folder` or contains it: the folder, or one of its
/// existing ancestors, is `path` by component or by identity.
pub fn contains(path: &Path, folder: &Path, fs: &dyn Lookup) -> Result<bool, ResolveFailure> {
    if folder.starts_with(path) {
        return Ok(true);
    }
    let Some(wanted) = identity(path, fs)? else {
        return Ok(false);
    };
    for ancestor in folder.ancestors() {
        if identity(ancestor, fs)? == Some(wanted) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The identity of the deepest existing ancestor of `folder` (the folder
/// itself when it exists), and the components below it that do not exist.
fn deepest_existing<'a>(
    folder: &'a Path,
    fs: &dyn Lookup,
) -> Result<Option<(Identity, Vec<&'a OsStr>)>, ResolveFailure> {
    for ancestor in folder.ancestors() {
        if let Some(found) = identity(ancestor, fs)? {
            let missing = folder
                .strip_prefix(ancestor)
                .map(|rest| rest.components().map(|part| part.as_os_str()).collect())
                .unwrap_or_default();
            return Ok(Some((found, missing)));
        }
    }
    Ok(None)
}

/// Whether `rest` starts with the `missing` components, ignoring ASCII case.
fn begins_with(rest: &Path, missing: &[&OsStr]) -> bool {
    let mut parts = rest.components().map(|part| part.as_os_str());
    missing.iter().all(|wanted| {
        parts
            .next()
            .is_some_and(|part| part.eq_ignore_ascii_case(wanted))
    })
}

/// The identity of an existing path, or `None` when nothing is there.
fn identity(path: &Path, fs: &dyn Lookup) -> Result<Option<Identity>, ResolveFailure> {
    match fs.metadata(path) {
        Ok(entry) => Ok(Some(entry.identity)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ResolveFailure::Identity(error.to_string())),
    }
}
