//! Whole-file replacement of a settings file (design 0003, CFG-R9): the new
//! bytes are renamed over the old file, and the write is refused when the
//! file on disk is not the one that was read.
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::settings;

/// The file on disk is not the one that was read.
pub const CONFIG_CONFLICT: &str = "config-conflict";

/// Why a replacement was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// The file changed on disk after it was read.
    Changed {
        /// The target file.
        path: PathBuf,
    },
    /// The target is a symbolic link, which Baley never writes through.
    Link {
        /// The link.
        path: PathBuf,
    },
}

impl Conflict {
    /// The stable refusal code.
    pub fn code(&self) -> &'static str {
        CONFIG_CONFLICT
    }
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.code())?;
        match self {
            Self::Changed { path } => write!(
                f,
                "{} changed on disk after it was read (fix: run the command again)",
                path.display()
            ),
            Self::Link { path } => write!(
                f,
                "{} is a symbolic link, which Baley does not write through \
                 (fix: edit the file it points to by hand, or replace the link with a regular file)",
                path.display()
            ),
        }
    }
}

/// Proceeds only when the file on disk is the one that was read: the same
/// digest, or no file then and none now. `None` is no file, never a wildcard.
pub fn decide(path: &Path, read: Option<&str>, on_disk: Option<&str>) -> Result<(), Conflict> {
    if read == on_disk {
        Ok(())
    } else {
        Err(Conflict::Changed { path: path.into() })
    }
}

/// A replacement that did not happen as asked.
#[derive(Debug)]
pub enum Failure {
    /// Refused before the rename; the target is as it was.
    Refused(Conflict),
    /// An I/O error at or before the rename; the target is as it was.
    Unchanged {
        /// The target file.
        path: PathBuf,
        /// The operating system's error.
        cause: io::Error,
    },
    /// The folder could not be synced after the rename: the target holds the
    /// new bytes, and the change may not survive a crash.
    Unsynced {
        /// The target file.
        path: PathBuf,
        /// The operating system's error.
        cause: io::Error,
    },
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(conflict) => conflict.fmt(f),
            Self::Unchanged { path, cause } => write!(
                f,
                "cannot replace {}: {cause}; the file is unchanged",
                path.display()
            ),
            Self::Unsynced { path, cause } => write!(
                f,
                "{} holds the new contents, but its folder could not be synced, \
                 so the change may not survive a crash: {cause}",
                path.display()
            ),
        }
    }
}

impl From<Conflict> for Failure {
    fn from(conflict: Conflict) -> Self {
        Self::Refused(conflict)
    }
}

/// Replaces `target` whole with `bytes`, given the digest of the file as it
/// was read (`None` when there was no file). The new bytes go to a temporary
/// file in the same folder, with the target's mode, and are renamed over the
/// target only if the file on disk is still the one read. A link is never
/// written through. Creates no folder.
pub fn replace(target: &Path, bytes: &[u8], read: Option<&str>) -> Result<(), Failure> {
    let unchanged = |cause| Failure::Unchanged {
        path: target.into(),
        cause,
    };
    let folder = match target.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    let name = target
        .file_name()
        .ok_or_else(|| unchanged(io::Error::new(io::ErrorKind::InvalidInput, "no file name")))?;
    let temporary = folder.join(temporary_name(name));
    // `create_new` never opens a file someone else made at this name.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(unchanged)?;

    let staged = (|| {
        // A new target keeps the umask's default mode.
        if let Ok(meta) = fs::symlink_metadata(target)
            && meta.is_file()
        {
            file.set_permissions(meta.permissions())
                .map_err(unchanged)?;
        }
        file.write_all(bytes).map_err(unchanged)?;
        file.sync_all().map_err(unchanged)?;
        // Just before the rename, which narrows the window a concurrent writer
        // has but cannot close it.
        decide(target, read, on_disk(target)?.as_deref())?;
        fs::rename(&temporary, target).map_err(unchanged)
    })();
    if let Err(failure) = staged {
        let _ = fs::remove_file(&temporary);
        return Err(failure);
    }
    fs::File::open(folder)
        .and_then(|folder| folder.sync_all())
        .map_err(|cause| Failure::Unsynced {
            path: target.into(),
            cause,
        })
}

/// A name in the target's folder that is the target's name with more around
/// it, so it can never be the target, and unique within this process.
fn temporary_name(name: &OsStr) -> OsString {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    temporary
}

/// The digest of the target as it is now, without following a link.
fn on_disk(target: &Path) -> Result<Option<String>, Failure> {
    let unchanged = |cause| Failure::Unchanged {
        path: target.into(),
        cause,
    };
    // Non-blocking, so a FIFO put in its place cannot hang the open.
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(target);
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            if fs::symlink_metadata(target).is_ok_and(|meta| meta.is_symlink()) {
                return Err(Conflict::Link {
                    path: target.into(),
                }
                .into());
            }
            return Err(unchanged(error));
        }
    };
    if !file.metadata().map_err(unchanged)?.is_file() {
        return Err(unchanged(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        )));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(unchanged)?;
    Ok(Some(settings::file(target, bytes).digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "/c/config.toml";
    const A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn changed() -> Result<(), Conflict> {
        Err(Conflict::Changed { path: PATH.into() })
    }

    #[test]
    fn a_file_changed_since_it_was_read_is_not_overwritten() {
        let refusal = decide(Path::new(PATH), Some(A), Some(B)).unwrap_err();
        assert_eq!(refusal.code(), "config-conflict");
        assert_eq!(
            refusal.to_string(),
            "config-conflict: /c/config.toml changed on disk after it was read \
             (fix: run the command again)"
        );
    }

    #[test]
    fn a_file_that_appeared_after_none_was_read_is_not_overwritten() {
        assert_eq!(decide(Path::new(PATH), None, Some(A)), changed());
    }

    #[test]
    fn a_file_deleted_after_it_was_read_is_not_silently_recreated() {
        assert_eq!(decide(Path::new(PATH), Some(A), None), changed());
    }

    #[test]
    fn the_file_that_was_read_is_replaced() {
        assert_eq!(decide(Path::new(PATH), Some(A), Some(A)), Ok(()));
    }

    #[test]
    fn a_first_write_where_no_file_was_or_is_is_not_refused() {
        assert_eq!(decide(Path::new(PATH), None, None), Ok(()));
    }

    // `A` is the SHA-256 of "abc" and `B` of no bytes, fixed from `sha256sum`.

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replacing_a_file_leaves_the_new_bytes_and_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        fs::write(&target, b"abc").unwrap();

        replace(&target, b"new", Some(A)).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(names(dir.path()), ["config.toml"]);
    }

    #[test]
    fn a_target_that_does_not_exist_is_created_when_none_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");

        replace(&target, b"new", None).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(names(dir.path()), ["config.toml"]);
    }

    #[test]
    fn a_linked_target_is_refused_and_neither_the_link_nor_its_file_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        let link = dir.path().join("config.toml");
        fs::write(&real, b"abc").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let Err(Failure::Refused(refusal)) = replace(&link, b"new", Some(A)) else {
            panic!("expected a refusal");
        };
        assert_eq!(refusal, Conflict::Link { path: link.clone() });
        assert_eq!(
            refusal.to_string(),
            format!(
                "config-conflict: {} is a symbolic link, which Baley does not write through \
                 (fix: edit the file it points to by hand, or replace the link with a regular file)",
                link.display()
            )
        );
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), real);
        assert_eq!(fs::read(&real).unwrap(), b"abc");
        assert_eq!(names(dir.path()), ["config.toml", "real.toml"]);
    }

    #[test]
    fn a_stale_digest_is_refused_and_the_target_keeps_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        fs::write(&target, b"abc").unwrap();

        let Err(Failure::Refused(refusal)) = replace(&target, b"new", Some(B)) else {
            panic!("expected a refusal");
        };
        assert_eq!(
            refusal,
            Conflict::Changed {
                path: target.clone()
            }
        );
        assert_eq!(fs::read(&target).unwrap(), b"abc");
        assert_eq!(names(dir.path()), ["config.toml"]);
    }
}
