//! The settings files as bytes: where the global file is, and a reader that
//! gathers a file and judges what it found (design 0003, CFG-R2, CFG-R9),
//! with a capped form of it for the guard.
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use baley_core::policy::{Fault, SettingsFile, Unavailable};
use sha2::{Digest, Sha256};

use crate::folders::Folders;

/// The global file's name in the config folder.
pub const GLOBAL_FILE: &str = "config.toml";

/// The global file: `config.toml` in the config folder.
pub fn global_path(folders: &Folders) -> PathBuf {
    folders.config.join(GLOBAL_FILE)
}

/// A settings file from its path and exact bytes, with the lower-case hex
/// SHA-256 of the bytes as its digest.
pub fn file(path: &Path, bytes: Vec<u8>) -> SettingsFile {
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    SettingsFile {
        path: path.into(),
        bytes,
        digest,
    }
}

/// Reads one settings file: `None` when it does not exist, `config-unavailable`
/// when it exists but is not a regular file or cannot be read.
pub fn read(path: &Path) -> Result<Option<SettingsFile>, Unavailable> {
    judge(path, gather(path))
}

/// The most bytes the guard takes of one settings file, and of each stream
/// of its git reads of HEAD's copy: 1 MiB.
pub const GUARD_LIMIT: usize = 1 << 20;

/// Reads one settings file for the guard as [`read`] does, but takes at most
/// [`GUARD_LIMIT`] bytes. A larger file is `config-unavailable` naming it, so
/// the guard judges it torn rather than wait on it or read part of it.
pub fn read_for_guard(path: &Path) -> Result<Option<SettingsFile>, Unavailable> {
    judge_for_guard(path, gather_for_guard(path))
}

/// As [`gather`], reading one byte past [`GUARD_LIMIT`] so the judge can
/// tell a file over the cap from one exactly at it.
fn gather_for_guard(path: &Path) -> Seen {
    gather_up_to(path, GUARD_LIMIT as u64 + 1)
}

/// What one read of a settings path found.
pub(crate) enum Seen {
    NotOpened {
        cause: std::io::Error,
        link: std::io::Result<bool>,
    },
    NotRegular,
    Unreadable {
        cause: String,
    },
    Opened(Vec<u8>),
}

/// Opens the path, following a link, and reads the file if it is a regular one.
pub(crate) fn gather(path: &Path) -> Seen {
    gather_up_to(path, u64::MAX)
}

/// As [`gather`], reading at most `limit` bytes.
fn gather_up_to(path: &Path, limit: u64) -> Seen {
    // Non-blocking, so a FIFO with no writer cannot hang the open before its
    // kind is judged; the flag changes nothing for a regular file.
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) => {
            return Seen::NotOpened {
                cause: error,
                link: std::fs::symlink_metadata(path).map(|meta| meta.file_type().is_symlink()),
            };
        }
    };
    match file.metadata() {
        Err(error) => Seen::Unreadable {
            cause: error.to_string(),
        },
        Ok(meta) if !meta.is_file() => Seen::NotRegular,
        Ok(_) => {
            let mut bytes = Vec::new();
            match file.take(limit).read_to_end(&mut bytes) {
                Ok(_) => Seen::Opened(bytes),
                Err(error) => Seen::Unreadable {
                    cause: error.to_string(),
                },
            }
        }
    }
}

/// A missing file is no layer; a file Baley cannot read leaves the policy
/// unavailable, since the owner's settings are in it.
pub(crate) fn judge(path: &Path, seen: Seen) -> Result<Option<SettingsFile>, Unavailable> {
    let unavailable = |fault| Unavailable {
        path: path.into(),
        fault,
    };
    match seen {
        Seen::NotOpened { cause, link } => {
            let missing = cause.kind() == std::io::ErrorKind::NotFound;
            match link {
                Err(error) if missing && error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Ok(true) if missing => Err(unavailable(Fault::Unreadable {
                    cause: "it is a link to a missing file".into(),
                })),
                _ => Err(unavailable(Fault::Unreadable {
                    cause: cause.to_string(),
                })),
            }
        }
        Seen::NotRegular => Err(unavailable(Fault::NotRegular)),
        Seen::Unreadable { cause } => Err(unavailable(Fault::Unreadable { cause })),
        Seen::Opened(bytes) => Ok(Some(file(path, bytes))),
    }
}

/// As [`judge`], refusing a file past [`GUARD_LIMIT`].
fn judge_for_guard(path: &Path, seen: Seen) -> Result<Option<SettingsFile>, Unavailable> {
    match seen {
        Seen::Opened(bytes) if bytes.len() > GUARD_LIMIT => Err(Unavailable {
            path: path.into(),
            fault: Fault::Unreadable {
                cause: format!("it is over the guard's {GUARD_LIMIT}-byte limit"),
            },
        }),
        seen => judge(path, seen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const PATH: &str = "/c/config.toml";

    #[test]
    fn a_missing_file_is_no_layer_and_an_unreadable_one_is_unavailable() {
        let path = Path::new(PATH);
        assert_eq!(
            judge(
                path,
                Seen::NotOpened {
                    cause: std::io::ErrorKind::NotFound.into(),
                    link: Err(std::io::ErrorKind::NotFound.into()),
                }
            ),
            Ok(None)
        );

        let refusal = judge(path, Seen::NotRegular).unwrap_err();
        assert_eq!(
            refusal,
            Unavailable {
                path: PATH.into(),
                fault: Fault::NotRegular,
            }
        );
        assert_eq!(
            refusal.to_string(),
            "config-unavailable: /c/config.toml is not a regular file"
        );

        let cause = "Permission denied (os error 13)";
        let refusal = judge(
            path,
            Seen::Unreadable {
                cause: cause.into(),
            },
        )
        .unwrap_err();
        assert_eq!(
            refusal.fault,
            Fault::Unreadable {
                cause: cause.into()
            }
        );
        assert_eq!(
            refusal.to_string(),
            "config-unavailable: cannot read /c/config.toml: Permission denied (os error 13)"
        );
    }

    #[test]
    fn a_guard_judge_with_no_byte_cap_or_one_off_by_one_is_caught() {
        let path = Path::new(PATH);
        let at_cap = judge_for_guard(path, Seen::Opened(vec![b'#'; 1_048_576]));
        assert_eq!(at_cap.unwrap().unwrap().bytes.len(), 1_048_576);

        let refusal = judge_for_guard(path, Seen::Opened(vec![b'#'; 1_048_577])).unwrap_err();
        assert_eq!(refusal.path, path);
        assert_eq!(
            refusal.to_string(),
            "config-unavailable: cannot read /c/config.toml: it is over the guard's 1048576-byte limit"
        );
    }

    #[test]
    fn the_digest_is_the_sha256_of_the_bytes() {
        // Fixed from `sha256sum`, not from this code.
        for (bytes, digest) in [
            (
                b"abc".as_slice(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"".as_slice(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc\n".as_slice(),
                "edeaaff3f1774ad2888673770c6d64097e391bc362d7d6fb34982ddf0efd18cb",
            ),
        ] {
            let read = judge(Path::new(PATH), Seen::Opened(bytes.to_vec()))
                .unwrap()
                .unwrap();
            assert_eq!(
                read,
                SettingsFile {
                    path: PATH.into(),
                    bytes: bytes.to_vec(),
                    digest: digest.into(),
                }
            );
        }
    }

    #[test]
    fn the_global_file_is_config_toml_in_the_config_folder() {
        let folders = Folders {
            config: "/c".into(),
            home: "/h".into(),
        };
        assert_eq!(global_path(&folders), Path::new("/c/config.toml"));
    }

    #[test]
    fn read_follows_a_link_and_returns_the_targets_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("settings.toml");
        let link = dir.path().join("config.toml");
        std::fs::write(&target, b"abc").unwrap();
        symlink(&target, &link).unwrap();

        let found = read(&link).unwrap().unwrap();
        assert_eq!(found.path, link);
        assert_eq!(found.bytes, b"abc");
        assert_eq!(
            found.digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(read(&dir.path().join("absent")), Ok(None));
        assert_eq!(read(dir.path()).unwrap_err().fault, Fault::NotRegular);
    }

    #[test]
    fn a_dangling_settings_link_is_not_reported_as_a_missing_layer() {
        let path = Path::new(PATH);
        let refusal = judge(
            path,
            Seen::NotOpened {
                cause: std::io::ErrorKind::NotFound.into(),
                link: Ok(true),
            },
        )
        .unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "config-unavailable: cannot read /c/config.toml: it is a link to a missing file"
        );
    }

    #[test]
    fn readers_do_not_lose_a_dangling_settings_link() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("baley.toml");
        symlink("missing.toml", &path).unwrap();
        for result in [read(&path), read_for_guard(&path)] {
            let refusal = result.unwrap_err();
            assert_eq!(refusal.path, path);
            assert_eq!(
                refusal.fault,
                Fault::Unreadable {
                    cause: "it is a link to a missing file".into(),
                }
            );
        }
    }

    #[test]
    fn a_guard_gather_reading_a_large_file_whole_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let large = dir.path().join("config.toml");
        std::fs::write(&large, vec![b'#'; GUARD_LIMIT * 2]).unwrap();
        let Seen::Opened(bytes) = gather_for_guard(&large) else {
            panic!("opened");
        };
        assert_eq!(bytes.len(), GUARD_LIMIT + 1);
    }
}
