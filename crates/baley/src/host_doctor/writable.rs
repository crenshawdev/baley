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
    /// This user may write into it.
    Writable,
    /// It exists and this user may not write into it, with the system's text.
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

/// Asks the filesystem whether this user may write into `location`, with
/// `access(W_OK)`. Owns no rule and has no unit test of its own.
pub fn gather(location: &Path) -> Seen {
    let Ok(text) = CString::new(location.as_os_str().as_bytes()) else {
        return Seen::NotWritable("the path holds a NUL byte".to_owned());
    };
    // SAFETY: `text` is a valid NUL-terminated string that outlives the call.
    if unsafe { libc::access(text.as_ptr(), libc::W_OK) } == 0 {
        return Seen::Writable;
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::NotFound {
        Seen::Absent
    } else {
        Seen::NotWritable(error.to_string())
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
    fn an_absent_location_raised_as_unwritable_is_caught() {
        let cause = "Read-only file system (os error 30)";
        assert_eq!(failure(&Seen::Writable), None);
        assert_eq!(failure(&Seen::Absent), None);
        assert_eq!(failure(&Seen::NotWritable(cause.to_owned())), Some(cause));
    }
}
