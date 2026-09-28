//! Gather filesystem facts separately from the rules for opening a home.
use baley_store::{FaultTarget, HomeFault, HomeProblem, StoreError};
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const FILES: [&str; 5] = [
    "baley.db",
    "baley.db-wal",
    "baley.db-shm",
    "baley.db.writer",
    "baley.db.maintenance",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Link,
    Folder,
    File,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    kind: Kind,
    owner: u32,
    mode: u32,
}

enum Seen {
    Entry(Entry),
    Unreadable(String),
}

/// The home and file observations, in layout order.
pub(crate) struct Observed {
    home: PathBuf,
    home_entry: Entry,
    files: Vec<(PathBuf, Seen)>,
    user: u32,
}

/// The complete safety decision, retaining inspection failures.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Judgement {
    /// Every inspected path is safe, and none was unreadable.
    Safe,
    /// Known faults take precedence over unreadable files.
    Unsafe(Vec<HomeFault>),
    /// No fault is known, but inspection could not finish.
    Unreadable {
        /// The first unreadable file in layout order.
        path: PathBuf,
        /// The inspection error's text.
        error: String,
    },
}

/// Inspects the final component without following it, following ancestor links.
pub(crate) fn gather(home: &Path) -> Result<Observed, StoreError> {
    let home_error = |error: io::Error| {
        if error.kind() == io::ErrorKind::NotFound {
            StoreError::Unavailable(format!("{} does not exist", home.display()))
        } else {
            StoreError::Unavailable(format!("cannot inspect {}: {error}", home.display()))
        }
    };
    let real = match home.file_name() {
        Some(name) => {
            let parent = home
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            parent.canonicalize().map_err(home_error)?.join(name)
        }
        None => home.canonicalize().map_err(home_error)?,
    };
    let home_entry = entry(fs::symlink_metadata(&real).map_err(home_error)?);
    let mut files = Vec::new();
    if home_entry.kind == Kind::Folder {
        for name in FILES {
            let path = real.join(name);
            let seen = match fs::symlink_metadata(&path) {
                Ok(metadata) => Seen::Entry(entry(metadata)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => Seen::Unreadable(error.to_string()),
            };
            files.push((path, seen));
        }
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    let user = unsafe { libc::geteuid() };
    Ok(Observed {
        home: real,
        home_entry,
        files,
        user,
    })
}

fn entry(metadata: fs::Metadata) -> Entry {
    let kind = metadata.file_type();
    Entry {
        kind: if kind.is_symlink() {
            Kind::Link
        } else if kind.is_dir() {
            Kind::Folder
        } else if kind.is_file() {
            Kind::File
        } else {
            Kind::Other
        },
        owner: metadata.uid(),
        mode: metadata.mode() & 0o7777,
    }
}

/// Judges only supplied facts, reporting every known fault in order.
pub(crate) fn judge(observed: &Observed) -> Judgement {
    let mut faults = Vec::new();
    judge_entry(
        &observed.home,
        observed.home_entry,
        FaultTarget::Home,
        observed.user,
        &mut faults,
    );
    let mut unreadable = None;
    for (path, seen) in &observed.files {
        match seen {
            Seen::Entry(entry) => {
                judge_entry(path, *entry, FaultTarget::File, observed.user, &mut faults)
            }
            Seen::Unreadable(error) => {
                unreadable.get_or_insert_with(|| Judgement::Unreadable {
                    path: path.clone(),
                    error: error.clone(),
                });
            }
        }
    }
    if !faults.is_empty() {
        Judgement::Unsafe(faults)
    } else {
        unreadable.unwrap_or(Judgement::Safe)
    }
}

fn judge_entry(
    path: &Path,
    entry: Entry,
    target: FaultTarget,
    user: u32,
    faults: &mut Vec<HomeFault>,
) {
    let mut fault = |problem| {
        faults.push(HomeFault {
            path: path.into(),
            target,
            problem,
        })
    };
    let (kind, allowed, wrong_kind) = match target {
        FaultTarget::Home => (Kind::Folder, 0o700, HomeProblem::NotAFolder),
        FaultTarget::File => (Kind::File, 0o600, HomeProblem::NotAFile),
    };
    if entry.kind == Kind::Link {
        fault(HomeProblem::Link);
        return;
    }
    if entry.kind != kind {
        fault(wrong_kind);
        return;
    }
    if entry.owner != user {
        fault(HomeProblem::Owner {
            owner: entry.owner,
            user,
        });
    }
    if entry.mode & (0o777 & !allowed) != 0 {
        fault(HomeProblem::Mode {
            mode: entry.mode,
            allowed,
        });
    }
}

/// Makes a private home or a root for homes owned by one test.
#[cfg(test)]
pub(crate) fn private_folder() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .expect("private folder")
}

/// Creates a database before a test's raw SQLite connection opens it.
#[cfg(test)]
pub(crate) fn private_file(path: &Path) {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .expect("private file");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed() -> Observed {
        Observed {
            home: "/h/baley".into(),
            home_entry: Entry {
                kind: Kind::Folder,
                owner: 1000,
                mode: 0o700,
            },
            files: FILES
                .iter()
                .map(|name| {
                    (
                        Path::new("/h/baley").join(name),
                        Seen::Entry(Entry {
                            kind: Kind::File,
                            owner: 1000,
                            mode: 0o600,
                        }),
                    )
                })
                .collect(),
            user: 1000,
        }
    }
    fn file(o: &mut Observed, index: usize) -> &mut Entry {
        let Seen::Entry(entry) = &mut o.files[index].1 else {
            panic!("entry")
        };
        entry
    }
    fn fault(name: &str, target: FaultTarget, problem: HomeProblem) -> HomeFault {
        HomeFault {
            path: if name.is_empty() {
                PathBuf::from("/h/baley")
            } else {
                Path::new("/h/baley").join(name)
            },
            target,
            problem,
        }
    }
    #[test]
    fn safe_home_and_all_five_files_are_not_refused() {
        assert_eq!(judge(&observed()), Judgement::Safe);
    }
    #[test]
    fn narrower_permissions_are_not_compared_for_equality() {
        let mut o = observed();
        o.home_entry.mode = 0o500;
        file(&mut o, 0).mode = 0o400;
        assert_eq!(judge(&o), Judgement::Safe);
    }
    #[test]
    fn special_bits_do_not_get_a_permission_fix_that_cannot_clear_them() {
        let mut o = observed();
        o.home_entry.mode = 0o2700;
        assert_eq!(judge(&o), Judgement::Safe);
    }
    #[test]
    fn another_users_home_is_not_accepted() {
        let mut o = observed();
        o.home_entry.owner = 1001;
        let expected = fault(
            "",
            FaultTarget::Home,
            HomeProblem::Owner {
                owner: 1001,
                user: 1000,
            },
        );
        assert_eq!(
            expected.to_string(),
            "/h/baley is owned by user id 1001, not by this user (1000) (fix: chown 1000 /h/baley)"
        );
        assert_eq!(judge(&o), Judgement::Unsafe(vec![expected]));
    }
    #[test]
    fn database_ownership_is_not_skipped() {
        let mut o = observed();
        file(&mut o, 0).owner = 0;
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![fault(
                "baley.db",
                FaultTarget::File,
                HomeProblem::Owner {
                    owner: 0,
                    user: 1000
                }
            )])
        );
    }
    #[test]
    fn group_access_to_home_is_not_accepted() {
        let mut o = observed();
        o.home_entry.mode = 0o750;
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![fault(
                "",
                FaultTarget::Home,
                HomeProblem::Mode {
                    mode: 0o750,
                    allowed: 0o700
                }
            )])
        );
    }
    #[test]
    fn logs_and_locks_do_not_use_the_home_permission_mask() {
        let mut o = observed();
        file(&mut o, 1).mode = 0o640;
        file(&mut o, 3).mode = 0o700;
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![
                fault(
                    "baley.db-wal",
                    FaultTarget::File,
                    HomeProblem::Mode {
                        mode: 0o640,
                        allowed: 0o600
                    }
                ),
                fault(
                    "baley.db.writer",
                    FaultTarget::File,
                    HomeProblem::Mode {
                        mode: 0o700,
                        allowed: 0o600
                    }
                ),
            ])
        );
    }
    #[test]
    fn linked_home_does_not_judge_the_links_owner_or_mode() {
        let mut o = observed();
        o.home_entry = Entry {
            kind: Kind::Link,
            owner: 1001,
            mode: 0o777,
        };
        o.files.clear();
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![fault("", FaultTarget::Home, HomeProblem::Link)])
        );
    }
    #[test]
    fn linked_database_is_not_accepted() {
        let mut o = observed();
        *file(&mut o, 0) = Entry {
            kind: Kind::Link,
            owner: 0,
            mode: 0o777,
        };
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![fault(
                "baley.db",
                FaultTarget::File,
                HomeProblem::Link
            )])
        );
    }
    #[test]
    fn file_home_is_not_accepted_as_a_folder() {
        let mut o = observed();
        o.home_entry.kind = Kind::File;
        o.files.clear();
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![fault("", FaultTarget::Home, HomeProblem::NotAFolder)])
        );
    }
    #[test]
    fn non_regular_database_is_not_accepted_as_a_file() {
        for kind in [Kind::Folder, Kind::Other] {
            let mut o = observed();
            file(&mut o, 0).kind = kind;
            assert_eq!(
                judge(&o),
                Judgement::Unsafe(vec![fault(
                    "baley.db",
                    FaultTarget::File,
                    HomeProblem::NotAFile
                )])
            );
        }
    }
    #[test]
    fn faults_are_not_lost_after_the_first_unsafe_path() {
        let mut o = observed();
        o.home_entry.mode = 0o755;
        file(&mut o, 0).mode = 0o644;
        file(&mut o, 2).owner = 1001;
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![
                fault(
                    "",
                    FaultTarget::Home,
                    HomeProblem::Mode {
                        mode: 0o755,
                        allowed: 0o700
                    }
                ),
                fault(
                    "baley.db",
                    FaultTarget::File,
                    HomeProblem::Mode {
                        mode: 0o644,
                        allowed: 0o600
                    }
                ),
                fault(
                    "baley.db-shm",
                    FaultTarget::File,
                    HomeProblem::Owner {
                        owner: 1001,
                        user: 1000
                    }
                ),
            ])
        );
    }
    #[test]
    fn wrong_owner_does_not_hide_wrong_mode_on_the_same_path() {
        let mut o = observed();
        o.home_entry.owner = 1001;
        o.home_entry.mode = 0o755;
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![
                fault(
                    "",
                    FaultTarget::Home,
                    HomeProblem::Owner {
                        owner: 1001,
                        user: 1000
                    }
                ),
                fault(
                    "",
                    FaultTarget::Home,
                    HomeProblem::Mode {
                        mode: 0o755,
                        allowed: 0o700
                    }
                ),
            ])
        );
    }
    #[test]
    fn unreadable_file_does_not_hide_a_known_home_fix() {
        let mut o = observed();
        o.home_entry.mode = 0o644;
        o.files[0].1 = Seen::Unreadable("denied".into());
        assert_eq!(
            judge(&o),
            Judgement::Unsafe(vec![fault(
                "",
                FaultTarget::Home,
                HomeProblem::Mode {
                    mode: 0o644,
                    allowed: 0o700
                }
            )])
        );
    }
    #[test]
    fn unreadable_files_do_not_pass_as_absent_or_safe() {
        let mut o = observed();
        o.files[0].1 = Seen::Unreadable("first error".into());
        o.files[1].1 = Seen::Unreadable("second error".into());
        assert_eq!(
            judge(&o),
            Judgement::Unreadable {
                path: "/h/baley/baley.db".into(),
                error: "first error".into()
            }
        );
    }

    fn open(home: &Path) -> Result<crate::SqliteStore, StoreError> {
        crate::SqliteStore::open(
            home,
            "2026-09-28T00:00:00Z",
            crate::Options {
                timing: crate::queue::scripted::Scripted::still(),
                ..crate::Options::default()
            },
        )
    }
    #[test]
    fn linked_home_is_refused_before_any_file_is_created() {
        let root = private_folder();
        let real = root.path().join("real");
        use std::os::unix::fs::{DirBuilderExt, symlink};
        fs::DirBuilder::new().mode(0o700).create(&real).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        assert_eq!(
            open(&link).err(),
            Some(StoreError::Refused(baley_store::Refusal::UnsafeHome(vec![
                HomeFault {
                    path: root.path().canonicalize().unwrap().join("link"),
                    target: FaultTarget::Home,
                    problem: HomeProblem::Link
                }
            ])))
        );
        assert_eq!(fs::read_dir(&real).unwrap().count(), 0);
    }
    #[test]
    fn linked_database_is_refused_before_lock_files_are_created() {
        let root = private_folder();
        let home = root.path().join("home");
        use std::os::unix::fs::{DirBuilderExt, symlink};
        fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
        let target = root.path().join("target");
        private_file(&target);
        symlink(&target, home.join("baley.db")).unwrap();
        assert_eq!(
            open(&home).err(),
            Some(StoreError::Refused(baley_store::Refusal::UnsafeHome(vec![
                HomeFault {
                    path: home.canonicalize().unwrap().join("baley.db"),
                    target: FaultTarget::File,
                    problem: HomeProblem::Link
                }
            ])))
        );
        assert!(!home.join("baley.db.writer").exists());
    }
    #[test]
    fn existing_database_is_not_an_error_when_creating_only_a_missing_one() {
        use std::io;
        assert_eq!(
            crate::store::created_or_present(Err(io::Error::from(io::ErrorKind::AlreadyExists))),
            Ok(())
        );
        assert!(
            crate::store::created_or_present(Err(io::Error::from(io::ErrorKind::PermissionDenied)))
                .is_err()
        );
    }
    #[test]
    fn linked_parent_does_not_refuse_a_real_private_home() {
        let root = private_folder();
        let real = root.path().join("real");
        use std::os::unix::fs::{DirBuilderExt, symlink};
        fs::create_dir(&real).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(real.join("home"))
            .unwrap();
        let via = root.path().join("via");
        symlink(&real, &via).unwrap();
        assert!(open(&via.join("home")).is_ok());
    }
}
