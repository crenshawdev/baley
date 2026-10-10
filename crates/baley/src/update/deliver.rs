//! Verified staging beside old versions and activation through the stable
//! link (design 0012 section 5).

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use baley_store::RequestId;
use sha2::{Digest, Sha256};

use super::events::FailureCode;
use super::fetch;
use super::installation::{
    Active, Layout, StablePath, gather_children, gather_stable, judge_active, versions_present,
};
use super::manifest::{Manifest, VerifiedDownload, verify_download};
use super::version::Version;

/// The part of delivery that refused the update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Obtaining the binary's bytes.
    Download,
    /// Checking those bytes against the manifest.
    Verification,
    /// Placing the verified binary beside old versions.
    Staging,
    /// Re-pointing the stable link.
    Activation,
}

impl Step {
    /// The step's name in a receipt.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Verification => "verification",
            Self::Staging => "staging",
            Self::Activation => "activation",
        }
    }
}

/// A refused delivery, including any version left staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The failed step.
    pub step: Step,
    /// The code to record.
    pub code: FailureCode,
    /// What prevented the step, including the path when an operation failed.
    pub cause: String,
    /// The offered version, only when staging finished.
    pub staged_version: Option<Version>,
}

/// A completed activation and the versions kept beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activated {
    /// The version now reached through the stable path.
    pub active_version: Version,
    /// The version placed or found intact in the versions folder.
    pub staged_version: Version,
    /// All versions present after staging, lowest first.
    pub kept_versions: Vec<Version>,
}

#[derive(Debug, PartialEq, Eq)]
enum StagedPath {
    Nothing,
    RegularFile { sha256: String, owner_runs: bool },
    Directory,
    SymbolicLink,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
enum Operation<'a> {
    CreateFolder { path: PathBuf, mode: u32 },
    Write { path: PathBuf, bytes: &'a [u8] },
    SetMode { path: PathBuf, mode: u32 },
    Sync { path: PathBuf },
    SyncFolder { path: PathBuf },
    Link { path: PathBuf, target: PathBuf },
    Rename { from: PathBuf, to: PathBuf },
    Exchange { from: PathBuf, to: PathBuf },
    Remove { path: PathBuf },
}

fn temporary_name(attempt: &RequestId) -> String {
    // Encoding keeps every supplied attempt inside one path component.
    format!(".baley-{}.tmp", hex(attempt.0.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn staging_decision<'a>(
    layout: &Layout,
    download: &'a VerifiedDownload,
    seen: &StagedPath,
    attempt: &RequestId,
) -> Result<Vec<Operation<'a>>, Failure> {
    let path = layout.staged_binary(download.version());
    match seen {
        StagedPath::Nothing => {}
        StagedPath::RegularFile { sha256, owner_runs }
            if *owner_runs && *sha256 == hex(&Sha256::digest(download.bytes())) =>
        {
            return Ok(Vec::new());
        }
        _ => {
            return Err(Failure {
                step: Step::Staging,
                code: FailureCode::StagingConflict,
                cause: format!(
                    "{} already holds something other than this runnable binary",
                    path.display()
                ),
                staged_version: None,
            });
        }
    }
    let folder = path.parent().expect("a staged binary has a version folder");
    let temporary = folder.join(temporary_name(attempt));
    Ok(vec![
        Operation::CreateFolder {
            path: folder.into(),
            mode: 0o755,
        },
        Operation::Write {
            path: temporary.clone(),
            bytes: download.bytes(),
        },
        Operation::SetMode {
            path: temporary.clone(),
            mode: 0o755,
        },
        Operation::Sync {
            path: temporary.clone(),
        },
        Operation::Rename {
            from: temporary,
            to: path.clone(),
        },
        Operation::SyncFolder {
            path: folder.into(),
        },
        Operation::SyncFolder {
            path: layout.versions_folder().into(),
        },
    ])
}

fn activation_decision(
    layout: &Layout,
    start: &StablePath,
    fresh: &StablePath,
    staged: Version,
    attempt: &RequestId,
) -> Result<Vec<Operation<'static>>, Failure> {
    if fresh != start || !matches!(judge_active(layout, fresh), Active::Version(_)) {
        return Err(Failure {
            step: Step::Activation,
            code: FailureCode::ActivationConflict,
            cause: format!(
                "{} is no longer the managed link observed at the start",
                layout.stable_path().display()
            ),
            staged_version: Some(staged),
        });
    }
    let temporary = layout
        .stable_path()
        .parent()
        .expect("the stable path has a parent")
        .join(temporary_name(attempt));
    Ok(vec![
        Operation::Link {
            path: temporary.clone(),
            target: layout.staged_binary(staged),
        },
        Operation::Exchange {
            from: temporary,
            to: layout.stable_path().into(),
        },
        Operation::SyncFolder {
            path: layout.stable_path().parent().expect("stable folder").into(),
        },
    ])
}

/// Verifies, stages and activates a selected version under one claim attempt.
/// `start` is the managed link observed before fetching. The caller selects
/// the offered version before passing its manifest and download here.
/// `before_step` confirms the claim immediately before staging and activation.
/// Activation exchanges links atomically on Linux and macOS and restores an
/// unexpected occupant. A failed restoration keeps its temporary entry for
/// inspection. Directory syncs precede success.
pub fn deliver(
    layout: &Layout,
    start: &StablePath,
    manifest: &Manifest,
    download: fetch::Observation,
    attempt: &RequestId,
    mut before_step: impl FnMut(Step) -> Result<(), Failure>,
) -> Result<Activated, Failure> {
    let bytes = fetch::interpret(download).map_err(|error| Failure {
        step: Step::Download,
        code: error.code(),
        cause: format!("{}: {}", error.address, error.cause),
        staged_version: None,
    })?;
    let verified = verify_download(bytes, manifest).map_err(|error| Failure {
        step: Step::Verification,
        code: FailureCode::ChecksumMismatch,
        cause: error.to_string(),
        staged_version: None,
    })?;
    let version = verified.version();
    let path = layout.staged_binary(version);
    let seen =
        gather_staged(&path).map_err(|error| not_writable(Step::Staging, None, &path, error))?;
    let operations = staging_decision(layout, &verified, &seen, attempt)?;
    before_step(Step::Staging)?;
    apply_step(&operations, Step::Staging, None)?;

    // Read before activation so a listing failure cannot hide an active update.
    let children = gather_children(layout).map_err(|error| {
        not_writable(
            Step::Staging,
            Some(version),
            layout.versions_folder(),
            error,
        )
    })?;
    let kept_versions = versions_present(&children);
    let fresh = gather_stable(layout).map_err(|error| {
        not_writable(Step::Activation, Some(version), layout.stable_path(), error)
    })?;
    let operations = activation_decision(layout, start, &fresh, version, attempt)?;
    before_step(Step::Activation)?;
    apply_activation(layout, start, version, attempt, &operations)?;
    Ok(Activated {
        active_version: version,
        staged_version: version,
        kept_versions,
    })
}

fn not_writable(
    step: Step,
    staged_version: Option<Version>,
    path: &Path,
    error: io::Error,
) -> Failure {
    Failure {
        step,
        code: FailureCode::NotWritable,
        cause: format!("{}: {error}", path.display()),
        staged_version,
    }
}

fn apply_step(
    operations: &[Operation<'_>],
    step: Step,
    staged: Option<Version>,
) -> Result<(), Failure> {
    apply(operations).map_err(|error| clean_failed_apply(error, step, staged))
}

fn clean_failed_apply(error: ApplyFailure, step: Step, staged: Option<Version>) -> Failure {
    let mut failure = not_writable(step, staged, &error.path, error.cause);
    if let Some(temporary) = error.temporary
        && let Err(error) = fs::remove_file(&temporary)
    {
        failure.cause.push_str(&format!(
            "; could not remove temporary {}: {error}",
            temporary.display()
        ));
    }
    failure
}

fn restore_operations(layout: &Layout, temporary: &Path) -> Vec<Operation<'static>> {
    vec![
        Operation::Exchange {
            from: temporary.into(),
            to: layout.stable_path().into(),
        },
        Operation::SyncFolder {
            path: layout.stable_path().parent().expect("stable folder").into(),
        },
        Operation::Remove {
            path: temporary.into(),
        },
    ]
}

fn swapped_decision(
    layout: &Layout,
    start: &StablePath,
    swapped_target: Option<&Path>,
    temporary: &Path,
) -> (Vec<Operation<'static>>, bool) {
    let matches = matches!(start, StablePath::Link { target, .. }
        if swapped_target == Some(target.as_path()))
        && matches!(judge_active(layout, start), Active::Version(_));
    if !matches {
        return (restore_operations(layout, temporary), true);
    }
    (
        vec![Operation::Remove {
            path: temporary.into(),
        }],
        false,
    )
}

fn apply_activation(
    layout: &Layout,
    start: &StablePath,
    staged: Version,
    attempt: &RequestId,
    operations: &[Operation<'_>],
) -> Result<(), Failure> {
    let temporary = layout
        .stable_path()
        .parent()
        .expect("stable folder")
        .join(temporary_name(attempt));
    if let Err(error) = apply(operations) {
        let exchanged = error.exchanged;
        let mut failure = clean_failed_apply(error, Step::Activation, Some(staged));
        if exchanged {
            restore_after_error(layout, &temporary, &mut failure);
        }
        return Err(failure);
    }
    let swapped = fs::read_link(&temporary);
    let (finish, conflict) = swapped_decision(layout, start, swapped.as_deref().ok(), &temporary);
    if let Err(error) = apply(&finish) {
        let mut failure = clean_failed_apply(error, Step::Activation, Some(staged));
        if !conflict {
            restore_after_error(layout, &temporary, &mut failure);
        }
        return Err(failure);
    }
    if conflict {
        return Err(Failure {
            step: Step::Activation,
            code: FailureCode::ActivationConflict,
            cause: format!(
                "{} changed before activation; its occupant was restored",
                layout.stable_path().display()
            ),
            staged_version: Some(staged),
        });
    }
    Ok(())
}

fn restore_after_error(layout: &Layout, temporary: &Path, failure: &mut Failure) {
    if let Err(error) = apply(&restore_operations(layout, temporary)) {
        failure.cause.push_str(&format!(
            "; restoration failed at {}: {}; inspect the retained entry at {}",
            error.path.display(),
            error.cause,
            temporary.display()
        ));
    }
}

fn exchange(from: &Path, to: &Path) -> io::Result<()> {
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // Both C strings remain alive through the call and contain no NUL bytes.
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let result = {
        let _ = (from, to);
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic link exchange is unavailable",
        ));
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn gather_staged(path: &Path) -> io::Result<StagedPath> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(StagedPath::Nothing),
        Err(error) => return Err(error),
    };
    let kind = metadata.file_type();
    if kind.is_symlink() {
        return Ok(StagedPath::SymbolicLink);
    }
    if kind.is_dir() {
        return Ok(StagedPath::Directory);
    }
    if !kind.is_file() {
        return Ok(StagedPath::Other);
    }
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(StagedPath::RegularFile {
        sha256: hex(&hash.finalize()),
        owner_runs: metadata.permissions().mode() & 0o100 != 0,
    })
}

struct ApplyFailure {
    path: PathBuf,
    cause: io::Error,
    temporary: Option<PathBuf>,
    exchanged: bool,
}

fn apply(operations: &[Operation<'_>]) -> Result<(), ApplyFailure> {
    let mut temporary = None;
    let mut exchanged = false;
    for operation in operations {
        let (path, result) = match operation {
            Operation::CreateFolder { path, mode } => (path, create_folder(path, *mode)),
            Operation::Write { path, bytes } => {
                let result = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .and_then(|mut file| {
                        temporary = Some(path.clone());
                        file.write_all(bytes)
                    });
                (path, result)
            }
            Operation::SetMode { path, mode } => (
                path,
                fs::set_permissions(path, fs::Permissions::from_mode(*mode)),
            ),
            Operation::Sync { path } => (
                path,
                OpenOptions::new()
                    .write(true)
                    .open(path)
                    .and_then(|file| file.sync_all()),
            ),
            Operation::SyncFolder { path } => {
                (path, File::open(path).and_then(|folder| folder.sync_all()))
            }
            Operation::Link { path, target } => {
                let result = symlink(target, path).map(|()| temporary = Some(path.clone()));
                (path, result)
            }
            Operation::Rename { from, to } => {
                let result = fs::rename(from, to).map(|()| temporary = None);
                (to, result)
            }
            Operation::Exchange { from, to } => {
                let result = exchange(from, to).map(|()| {
                    // The temporary name now holds someone else's entry.
                    temporary = None;
                    exchanged = true;
                });
                (to, result)
            }
            Operation::Remove { path } => (path, fs::remove_file(path)),
        };
        if let Err(cause) = result {
            return Err(ApplyFailure {
                path: path.clone(),
                cause,
                temporary,
                exchanged,
            });
        }
    }
    Ok(())
}

fn create_folder(path: &Path, mode: u32) -> io::Result<()> {
    match fs::DirBuilder::new().mode(mode).create(path) {
        // A restrictive umask must not keep other users from running the binary.
        Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(mode)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or(error)?;
            create_folder(parent, mode)?;
            create_folder(path, mode)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::Environment;
    use crate::update::installation::{Active, gather_stable, judge_active};
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    const ATTEMPT: &str = "00000000-0000-4000-8000-0000000000cc";
    const NEW_DIGEST: &str = "11507a0e2f5e69d5dfa40a62a1bd7b6ee57e6bcd85c67c9b8431b36fff21c437";

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn layout(home: &Path) -> Layout {
        Layout::resolve(&Environment {
            home: Some(home.into()),
            ..Environment::default()
        })
        .unwrap()
    }

    fn manifest() -> Manifest {
        Manifest {
            version: v("0.2.0"),
            sha256: NEW_DIGEST.into(),
        }
    }

    fn download(bytes: &[u8]) -> fetch::Observation {
        fetch::Observation {
            address: "https://dl.example/dev/linux-x86_64/baley".into(),
            transport_error: None,
            status: Some(200),
            body: bytes.to_vec(),
            bound: 268_435_456,
            exceeded_bound: false,
        }
    }

    fn installed() -> (tempfile::TempDir, Layout, StablePath) {
        let home = tempfile::tempdir().unwrap();
        let layout = layout(home.path());
        let old = layout.staged_binary(v("0.1.0"));
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, b"old").unwrap();
        fs::create_dir_all(layout.stable_path().parent().unwrap()).unwrap();
        symlink(&old, layout.stable_path()).unwrap();
        let start = gather_stable(&layout).unwrap();
        assert_eq!(judge_active(&layout, &start), Active::Version(v("0.1.0")));
        (home, layout, start)
    }

    fn children(path: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn no_temporary_names(layout: &Layout, newer: bool) {
        assert_eq!(children(layout.stable_path().parent().unwrap()), ["baley"]);
        assert_eq!(
            children(layout.versions_folder()),
            if newer {
                vec!["0.1.0", "0.2.0"]
            } else {
                vec!["0.1.0"]
            }
        );
        for version in if newer {
            vec!["0.1.0", "0.2.0"]
        } else {
            vec!["0.1.0"]
        } {
            assert_eq!(
                children(layout.staged_binary(v(version)).parent().unwrap()),
                ["baley"]
            );
        }
    }

    #[test]
    fn a_failed_step_that_moves_the_stable_path_or_goes_unnamed_is_caught() {
        for (step, code, staged) in [
            ("download", "update-network-unavailable", None),
            ("verification", "update-checksum-mismatch", None),
            ("staging", "update-staging-conflict", None),
            ("activation", "update-activation-conflict", Some(v("0.2.0"))),
        ] {
            let (home, layout, start) = installed();
            let new = layout.staged_binary(v("0.2.0"));
            let mut observation = download(b"new");
            match step {
                "download" => {
                    observation.transport_error = Some("connection refused".into());
                    observation.status = None;
                    observation.body.clear();
                }
                "verification" => observation.body = b"NEW".to_vec(),
                "staging" => fs::create_dir_all(&new).unwrap(),
                "activation" => {
                    fs::rename(layout.stable_path(), home.path().join("original-link")).unwrap();
                    fs::write(layout.stable_path(), b"owner").unwrap();
                }
                _ => unreachable!(),
            }
            let before = gather_stable(&layout).unwrap();
            let failure = deliver(
                &layout,
                &start,
                &manifest(),
                observation,
                &RequestId(ATTEMPT.into()),
                |_| Ok(()),
            )
            .expect_err(step);
            assert_eq!(failure.step.as_str(), step);
            assert_eq!(failure.code.as_str(), code);
            assert_eq!(failure.staged_version, staged);
            assert!(!failure.cause.is_empty());
            assert_eq!(gather_stable(&layout).unwrap(), before);
            assert_eq!(fs::read(layout.staged_binary(v("0.1.0"))).unwrap(), b"old");
            if step == "activation" {
                assert_eq!(fs::read(layout.stable_path()).unwrap(), b"owner");
                assert_eq!(fs::read(&new).unwrap(), b"new");
            } else {
                assert_eq!(
                    fs::read_link(layout.stable_path()).unwrap(),
                    layout.staged_binary(v("0.1.0"))
                );
            }
            if step == "staging" {
                assert!(new.is_dir());
            }
            if matches!(step, "download" | "verification") {
                assert!(!new.exists());
            }
            no_temporary_names(&layout, matches!(step, "staging" | "activation"));
        }
    }

    #[test]
    fn a_different_file_at_a_staged_version_overwritten_is_caught() {
        let layout = layout(Path::new("/home/o"));
        let verified = verify_download(b"new".to_vec(), &manifest()).unwrap();
        let attempt = RequestId(ATTEMPT.into());
        let write = staging_decision(&layout, &verified, &StagedPath::Nothing, &attempt).unwrap();
        assert!(matches!(
            write.as_slice(),
            [
                Operation::CreateFolder { .. },
                Operation::Write { bytes: b"new", .. },
                ..
            ]
        ));
        let keep = staging_decision(
            &layout,
            &verified,
            &StagedPath::RegularFile {
                sha256: NEW_DIGEST.into(),
                owner_runs: true,
            },
            &attempt,
        )
        .unwrap();
        assert!(keep.is_empty());
        for seen in [
            StagedPath::RegularFile {
                sha256: "00".repeat(32),
                owner_runs: true,
            },
            StagedPath::Directory,
            StagedPath::SymbolicLink,
        ] {
            let failure =
                staging_decision(&layout, &verified, &seen, &attempt).expect_err("occupied");
            assert_eq!(failure.step, Step::Staging);
            assert_eq!(failure.code.as_str(), "update-staging-conflict");
            assert_eq!(failure.staged_version, None);
            assert!(
                failure
                    .cause
                    .contains("/home/o/.local/lib/crenshawdev/baley/versions/0.2.0/baley")
            );
        }
    }

    fn managed(layout: &Layout, version: &str) -> StablePath {
        StablePath::Link {
            target: layout.staged_binary(v(version)),
            followed: super::super::installation::Followed::RegularFile,
        }
    }

    #[test]
    fn an_activation_over_an_owner_file_or_a_moved_link_is_caught() {
        let layout = layout(Path::new("/home/o"));
        let start = managed(&layout, "0.1.0");
        let attempt = RequestId(ATTEMPT.into());
        let activate = activation_decision(&layout, &start, &start, v("0.2.0"), &attempt).unwrap();
        assert!(matches!(
            activate.as_slice(),
            [
                Operation::Link { .. },
                Operation::Exchange { .. },
                Operation::SyncFolder { .. }
            ]
        ));
        for fresh in [
            managed(&layout, "0.3.0"),
            StablePath::NotALink,
            StablePath::Nothing,
        ] {
            let failure = activation_decision(&layout, &start, &fresh, v("0.2.0"), &attempt)
                .expect_err("changed stable path");
            assert_eq!(failure.step, Step::Activation);
            assert_eq!(failure.code.as_str(), "update-activation-conflict");
            assert_eq!(failure.staged_version, Some(v("0.2.0")));
            assert!(failure.cause.contains("/home/o/.local/bin/baley"));
        }
    }

    #[test]
    fn a_staged_binary_left_without_run_permission_is_caught() {
        let layout = layout(Path::new("/home/o"));
        let verified = verify_download(b"new".to_vec(), &manifest()).unwrap();
        let folder = Path::new("/home/o/.local/lib/crenshawdev/baley/versions/0.2.0");
        let binary = folder.join("baley");
        let mut names = Vec::new();
        for attempt in [ATTEMPT, "00000000-0000-4000-8000-0000000000dd"] {
            let operations = staging_decision(
                &layout,
                &verified,
                &StagedPath::Nothing,
                &RequestId(attempt.into()),
            )
            .unwrap();
            let Operation::Write {
                path: temporary, ..
            } = &operations[1]
            else {
                panic!("the second operation must write the temporary binary");
            };
            assert_eq!(temporary.parent(), Some(folder));
            assert_ne!(temporary, &binary);
            assert_eq!(
                operations,
                vec![
                    Operation::CreateFolder {
                        path: folder.into(),
                        mode: 0o755
                    },
                    Operation::Write {
                        path: temporary.clone(),
                        bytes: b"new"
                    },
                    Operation::SetMode {
                        path: temporary.clone(),
                        mode: 0o755
                    },
                    Operation::Sync {
                        path: temporary.clone()
                    },
                    Operation::Rename {
                        from: temporary.clone(),
                        to: binary.clone()
                    },
                    Operation::SyncFolder {
                        path: folder.into()
                    },
                    Operation::SyncFolder {
                        path: layout.versions_folder().into()
                    },
                ]
            );
            names.push(temporary.clone());
        }
        assert_ne!(names[0], names[1]);
    }

    #[test]
    fn a_stable_path_removed_before_its_new_link_is_in_place_is_caught() {
        let layout = layout(Path::new("/home/o"));
        let start = managed(&layout, "0.1.0");
        let folder = Path::new("/home/o/.local/bin");
        let stable = folder.join("baley");
        let mut names = Vec::new();
        for attempt in [ATTEMPT, "00000000-0000-4000-8000-0000000000dd"] {
            let operations = activation_decision(
                &layout,
                &start,
                &start,
                v("0.2.0"),
                &RequestId(attempt.into()),
            )
            .unwrap();
            let Operation::Link {
                path: temporary, ..
            } = &operations[0]
            else {
                panic!("the first operation must create the temporary link");
            };
            assert_eq!(temporary.parent(), Some(folder));
            assert_ne!(temporary, &stable);
            assert_eq!(
                operations,
                vec![
                    Operation::Link {
                        path: temporary.clone(),
                        target: "/home/o/.local/lib/crenshawdev/baley/versions/0.2.0/baley".into(),
                    },
                    Operation::Exchange {
                        from: temporary.clone(),
                        to: stable.clone()
                    },
                    Operation::SyncFolder {
                        path: folder.into()
                    },
                ]
            );
            names.push(temporary.clone());
        }
        assert_ne!(names[0], names[1]);
    }

    #[test]
    fn an_old_version_removed_or_the_link_left_relative_by_an_update_is_caught() {
        let (_home, layout, start) = installed();
        let result = deliver(
            &layout,
            &start,
            &manifest(),
            download(b"new"),
            &RequestId(ATTEMPT.into()),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            result,
            Activated {
                active_version: v("0.2.0"),
                staged_version: v("0.2.0"),
                kept_versions: vec![v("0.1.0"), v("0.2.0")],
            }
        );
        assert_eq!(fs::read(layout.staged_binary(v("0.1.0"))).unwrap(), b"old");
        assert_eq!(fs::read(layout.staged_binary(v("0.2.0"))).unwrap(), b"new");
        let target = fs::read_link(layout.stable_path()).unwrap();
        assert!(target.is_absolute());
        assert_eq!(target, layout.staged_binary(v("0.2.0")));
        no_temporary_names(&layout, true);
    }

    #[test]
    fn a_staged_file_that_matches_its_digest_but_cannot_run_activated_is_caught() {
        let layout = layout(Path::new("/home/o"));
        let verified = verify_download(b"new".to_vec(), &manifest()).unwrap();
        let attempt = RequestId(ATTEMPT.into());
        let seen = StagedPath::RegularFile {
            sha256: NEW_DIGEST.into(),
            owner_runs: false,
        };
        let failure = staging_decision(&layout, &verified, &seen, &attempt)
            .expect_err("a digest match without owner execute permission must be refused");
        assert_eq!(failure.step, Step::Staging);
        assert_eq!(failure.code, FailureCode::StagingConflict);
        assert_eq!(failure.staged_version, None);
        let runnable = StagedPath::RegularFile {
            sha256: NEW_DIGEST.into(),
            owner_runs: true,
        };
        assert!(
            staging_decision(&layout, &verified, &runnable, &attempt)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_owner_replacement_discarded_after_the_activation_swap_is_caught() {
        let layout = layout(Path::new("/home/o"));
        let start = managed(&layout, "0.1.0");
        let temporary = Path::new("/home/o/.local/bin/.attempt");
        let moved = layout.staged_binary(v("0.3.0"));
        for swapped in [None, Some(moved.as_path())] {
            let (operations, conflict) = swapped_decision(&layout, &start, swapped, temporary);
            assert!(conflict, "an unexpected swapped occupant must be restored");
            assert_eq!(
                operations,
                vec![
                    Operation::Exchange {
                        from: temporary.into(),
                        to: layout.stable_path().into()
                    },
                    Operation::SyncFolder {
                        path: "/home/o/.local/bin".into()
                    },
                    Operation::Remove {
                        path: temporary.into()
                    },
                ]
            );
        }
        let old = layout.staged_binary(v("0.1.0"));
        let (operations, conflict) = swapped_decision(&layout, &start, Some(&old), temporary);
        assert!(!conflict);
        assert_eq!(
            operations,
            vec![Operation::Remove {
                path: temporary.into()
            }]
        );
    }

    #[test]
    fn publication_or_activation_without_a_following_directory_sync_is_caught() {
        let layout = layout(Path::new("/home/o"));
        let verified = verify_download(b"new".to_vec(), &manifest()).unwrap();
        let attempt = RequestId(ATTEMPT.into());
        let staging = staging_decision(&layout, &verified, &StagedPath::Nothing, &attempt).unwrap();
        assert!(
            matches!(staging.as_slice(), [
            Operation::CreateFolder { .. }, Operation::Write { .. },
            Operation::SetMode { .. }, Operation::Sync { .. },
            Operation::Rename { to, .. }, Operation::SyncFolder { path },
            Operation::SyncFolder { path: parent },
        ] if to == Path::new("/home/o/.local/lib/crenshawdev/baley/versions/0.2.0/baley")
            && path == Path::new("/home/o/.local/lib/crenshawdev/baley/versions/0.2.0")
            && parent == Path::new("/home/o/.local/lib/crenshawdev/baley/versions")),
            "publication must sync the version folder after its rename"
        );
        let start = managed(&layout, "0.1.0");
        let activation =
            activation_decision(&layout, &start, &start, v("0.2.0"), &attempt).unwrap();
        assert!(
            matches!(activation.as_slice(), [
            Operation::Link { .. }, Operation::Exchange { to, .. }, Operation::SyncFolder { path },
        ] if to == Path::new("/home/o/.local/bin/baley") && path == Path::new("/home/o/.local/bin")),
            "activation must sync the stable folder after its exchange"
        );
    }
}
