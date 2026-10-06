//! The executable the hook and the registration run, judged once (D-12).
//!
//! The judge reads nothing from disk. Whether the file exists, is a file or
//! can be executed is the doctor's observation (Build 3 T13), not this
//! type's. Every renderer takes the judged type, so the rule is applied in
//! one place and a bad path never reaches a rendered artifact.

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

/// Why a supplied path cannot be used as given (D-12, D-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFault {
    /// The path is empty.
    Empty,
    /// The path is not UTF-8, so it cannot be written into JSON or a shell
    /// string as given.
    NotUtf8,
    /// The path is relative, so what it names would depend on the host's
    /// working directory.
    Relative,
}

impl fmt::Display for PathFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathFault::Empty => "is empty",
            PathFault::NotUtf8 => "is not UTF-8",
            PathFault::Relative => "is not absolute",
        })
    }
}

/// Judges one supplied path: it must be non-empty, UTF-8 and absolute. The
/// first fault found is reported. The placement map applies the same rule
/// to every supplied path.
pub fn judge(path: &OsStr) -> Result<String, PathFault> {
    if path.is_empty() {
        return Err(PathFault::Empty);
    }
    let text = path.to_str().ok_or(PathFault::NotUtf8)?;
    if !Path::new(text).is_absolute() {
        return Err(PathFault::Relative);
    }
    Ok(text.to_owned())
}

/// A prerequisite the caller did not meet: the executable path it supplied
/// has a fault, so nothing that runs it can be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPrerequisite {
    /// What is wrong with the supplied path.
    pub fault: PathFault,
}

impl fmt::Display for MissingPrerequisite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the executable path {}", self.fault)
    }
}

/// The supplied executable, judged absolute and UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executable(String);

impl Executable {
    /// Judges a supplied path as the executable.
    pub fn new(path: impl AsRef<OsStr>) -> Result<Self, MissingPrerequisite> {
        judge(path.as_ref())
            .map(Executable)
            .map_err(|fault| MissingPrerequisite { fault })
    }

    /// The path as it was supplied.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    #[test]
    fn a_relative_empty_or_non_utf8_executable_taken_as_a_met_prerequisite_is_caught() {
        for (path, fault) in [
            (OsStr::new("baley"), PathFault::Relative),
            (OsStr::new("bin/baley"), PathFault::Relative),
            (OsStr::new(""), PathFault::Empty),
            (OsStr::from_bytes(b"/p\xff/baley"), PathFault::NotUtf8),
        ] {
            assert_eq!(
                Executable::new(path),
                Err(MissingPrerequisite { fault }),
                "{path:?}"
            );
        }
        let accepted = Executable::new("/home/o w/baley").unwrap();
        assert_eq!(accepted.as_str(), "/home/o w/baley");
    }
}
