//! Whole-file replacement of a settings file (design 0003, CFG-R9): the new
//! bytes are renamed over the old file, and the write is refused when the
//! file on disk is not the one that was read.
use std::fmt;
use std::path::{Path, PathBuf};

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
}
