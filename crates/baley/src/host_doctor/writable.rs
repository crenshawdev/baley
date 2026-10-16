//! Whether each folder `baley install` writes into is writable (design 0012
//! section 5, Refusals).
//!
//! The locations come from the placement map, so an installation under
//! `CLAUDE_CONFIG_DIR` has its own folders checked. Only an installed machine
//! has them: with no install record the map places no versions folder and
//! the list is empty.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::host_artifacts::placement::{Artifact, PlacementMap};

/// What asking about one location found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// It is a folder this user may write into and search.
    Writable,
    /// It exists and is not a folder this user may write into and search, with
    /// the cause.
    NotWritable(String),
    /// Nothing is there yet.
    Absent,
}

/// The folders install writes into, each once: the stable path's folder, the
/// versions folder, each placed stub's folder, the folder of the placed
/// settings file and the folder of the placed registration file. None when
/// the map places no versions folder, which only an install record does.
pub fn locations(map: &PlacementMap) -> Vec<PathBuf> {
    let Some(versions) = map.write_only_folders().into_iter().next() else {
        return Vec::new();
    };
    let files = map.expected_files();
    let folders_of = |wanted: fn(&Artifact) -> bool| {
        files
            .iter()
            .filter(move |file| wanted(&file.artifact))
            .filter_map(|file| file.path.parent().map(Path::to_path_buf))
    };
    let mut found: Vec<PathBuf> = Vec::new();
    let stable = Path::new(map.executable().as_str()).parent();
    let candidates = stable
        .map(Path::to_path_buf)
        .into_iter()
        .chain([versions])
        .chain(folders_of(|artifact| matches!(artifact, Artifact::Stub(_))))
        .chain(folders_of(|artifact| {
            matches!(artifact, Artifact::Settings | Artifact::Hook)
        }))
        .chain(folders_of(|artifact| *artifact == Artifact::Registration));
    for folder in candidates {
        if !found.contains(&folder) {
            found.push(folder);
        }
    }
    found
}

/// What the filesystem said about one location, before any rule is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// Nothing is there yet.
    Absent,
    /// The system would not say what is there, with its text.
    Unknown(String),
    /// Something is there.
    Present {
        /// Whether it is a folder, following a link.
        folder: bool,
        /// The system's text when this user may not write into it.
        write: Option<String>,
        /// The system's text when this user may not search it.
        search: Option<String>,
    },
}

/// Asks the filesystem about `location`: whether it is a folder and, with
/// `access`, whether this user may write and search it. Owns no rule and has
/// no unit test of its own.
pub fn gather(location: &Path) -> Observed {
    let Ok(text) = CString::new(location.as_os_str().as_bytes()) else {
        return Observed::Unknown("the path holds a NUL byte".to_owned());
    };
    let metadata = match std::fs::metadata(location) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Observed::Absent,
        Err(error) => return Observed::Unknown(error.to_string()),
    };
    let denied = |mode: libc::c_int| {
        // SAFETY: `text` is a valid NUL-terminated string that outlives the call.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        let allowed = unsafe { libc::access(text.as_ptr(), mode) } == 0;
        (!allowed).then(|| io::Error::last_os_error().to_string())
    };
    Observed::Present {
        folder: metadata.is_dir(),
        write: denied(libc::W_OK),
        search: denied(libc::X_OK),
    }
}

/// The rule: a location install can create things in is a folder this user
/// may both write into and search. One that exists and is not is not
/// writable, whatever `access(W_OK)` alone says. An absent one is not a
/// failure, since install creates it.
pub fn judge(observed: Observed) -> Seen {
    match observed {
        Observed::Absent => Seen::Absent,
        Observed::Unknown(cause) => Seen::NotWritable(cause),
        Observed::Present { folder: false, .. } => {
            Seen::NotWritable("it exists but is not a folder".to_owned())
        }
        Observed::Present {
            write: Some(cause), ..
        } => Seen::NotWritable(cause),
        Observed::Present {
            search: Some(cause),
            ..
        } => Seen::NotWritable(cause),
        Observed::Present { .. } => Seen::Writable,
    }
}

/// The cause when `seen` is a failed check: a location that exists and this
/// user may not write into. An absent one is not, since install creates it.
pub fn failure(seen: &Seen) -> Option<&str> {
    match seen {
        Seen::NotWritable(cause) => Some(cause),
        Seen::Writable | Seen::Absent => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::installed_record;
    use super::super::installed_placements;
    use super::*;
    use crate::folders::Environment;
    use crate::host_artifacts::{installed, stubs};

    fn paths(found: &[PathBuf]) -> Vec<&str> {
        found.iter().map(|path| path.to_str().unwrap()).collect()
    }

    #[test]
    fn an_installed_folder_left_out_of_the_writability_check_or_checked_twice_is_caught() {
        let home = installed_placements(
            Ok(crate::install::fixtures::installed()),
            Ok(Some(installed_record())),
        )
        .unwrap();
        assert_eq!(
            paths(&locations(&home)),
            [
                "/home/o/.local/bin",
                "/home/o/.local/lib/crenshawdev/baley/versions",
                "/home/o/.claude/skills/bal-capture",
                "/home/o/.claude/skills/bal-help",
                "/home/o/.claude",
                "/home/o",
            ]
        );

        let never =
            installed_placements(Ok(crate::install::fixtures::installed()), Ok(None)).unwrap();
        assert_eq!(locations(&never), Vec::<PathBuf>::new());

        let env = Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        };
        let manifest = stubs::manifest(&stubs::front_doors()).unwrap();
        let moved = installed::resolve(&env, Some("/srv/claude".into()), &manifest).unwrap();
        let moved = installed_placements(Ok(moved), Ok(Some(installed_record()))).unwrap();
        let found = locations(&moved);
        assert_eq!(
            paths(&found),
            [
                "/home/o/.local/bin",
                "/home/o/.local/lib/crenshawdev/baley/versions",
                "/srv/claude/skills/bal-capture",
                "/srv/claude/skills/bal-help",
                "/srv/claude",
            ]
        );
        for default in ["/home/o/.claude", "/home/o"] {
            assert!(!paths(&found).contains(&default), "{default}");
        }
    }

    #[test]
    fn a_location_install_cannot_create_in_judged_writable_is_caught() {
        let present = |folder: bool, write: Option<&str>, search: Option<&str>| Observed::Present {
            folder,
            write: write.map(str::to_owned),
            search: search.map(str::to_owned),
        };
        assert_eq!(judge(present(true, None, None)), Seen::Writable);
        // A folder without search permission passes `access(W_OK)` yet holds nothing new.
        let no_search = "Permission denied (os error 13)";
        assert_eq!(
            judge(present(true, None, Some(no_search))),
            Seen::NotWritable(no_search.to_owned())
        );
        // A regular file where a folder should be passes `access(W_OK)` too.
        assert_eq!(
            judge(present(false, None, None)),
            Seen::NotWritable("it exists but is not a folder".to_owned())
        );
        let no_write = "Read-only file system (os error 30)";
        assert_eq!(
            judge(present(true, Some(no_write), None)),
            Seen::NotWritable(no_write.to_owned())
        );
        assert_eq!(judge(Observed::Absent), Seen::Absent);
        assert_eq!(
            judge(Observed::Unknown("denied".to_owned())),
            Seen::NotWritable("denied".to_owned())
        );
    }

    #[test]
    fn an_absent_location_raised_as_unwritable_is_caught() {
        let cause = "Read-only file system (os error 30)";
        assert_eq!(failure(&Seen::Writable), None);
        assert_eq!(failure(&Seen::Absent), None);
        assert_eq!(failure(&Seen::NotWritable(cause.to_owned())), Some(cause));
    }
}
