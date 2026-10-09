//! Where Baley's binary lives and which version the stable path runs
//! (design 0012 section 5).
//!
//! The stable path `~/.local/bin/baley` is a symbolic link into the versions
//! folder, and a staged version's binary is `<versions>/<version>/baley`.
//! Every path comes from `HOME` alone, so Baley's own home and the XDG
//! variables move nothing. The installation's identity is the stable path as
//! text, never the file it points at, so re-pointing the link keeps one
//! identity and one daily claim.
//!
//! The gatherers at the end only observe. The rules, which observation means
//! which case, are in the judges above them.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::folders::Environment;

use super::version::Version;

/// The paths of one installation, all derived from `HOME`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    stable: String,
    versions: String,
}

/// Why `HOME` cannot place the installation, with what to set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeRefusal {
    /// `HOME` is not set.
    Unset,
    /// `HOME` is set but empty.
    Empty,
    /// `HOME` is not an absolute path.
    Relative(String),
    /// `HOME` contains `..`, whose meaning depends on folder links.
    ParentComponent(String),
    /// `HOME` is not valid UTF-8, so its text cannot be a stable identity.
    NotUtf8(String),
}

impl fmt::Display for HomeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HomeRefusal::Unset => f.write_str("HOME is not set; set it to your home folder"),
            HomeRefusal::Empty => {
                f.write_str("HOME is empty; set it to the absolute path of your home folder")
            }
            HomeRefusal::Relative(value) => write!(
                f,
                "HOME is \"{}\", which is not an absolute path; set it to the absolute path of your home folder",
                value.escape_debug()
            ),
            HomeRefusal::ParentComponent(value) => write!(
                f,
                "HOME is \"{}\", which contains ..; set it to the absolute path of your home folder with no .. in it",
                value.escape_debug()
            ),
            HomeRefusal::NotUtf8(lossy) => write!(
                f,
                "HOME is \"{}\" and is not valid UTF-8; set it to a path that is",
                lossy.escape_debug()
            ),
        }
    }
}

impl std::error::Error for HomeRefusal {}

impl Layout {
    /// The layout for the `HOME` the environment supplies. Empty and `.`
    /// components are removed, including repeated and trailing slashes, so
    /// one home gives one spelling of every path and installation identity.
    /// A `..` component is refused because its meaning depends on folder links.
    pub fn resolve(env: &Environment) -> Result<Layout, HomeRefusal> {
        let value = env.home.as_ref().ok_or(HomeRefusal::Unset)?;
        if value.is_empty() {
            return Err(HomeRefusal::Empty);
        }
        let Some(home) = value.to_str() else {
            return Err(HomeRefusal::NotUtf8(value.to_string_lossy().into_owned()));
        };
        if !home.starts_with('/') {
            return Err(HomeRefusal::Relative(home.to_owned()));
        }
        if home.split('/').any(|component| component == "..") {
            return Err(HomeRefusal::ParentComponent(home.to_owned()));
        }
        let home = format!(
            "/{}",
            home.split('/')
                .filter(|component| !component.is_empty() && *component != ".")
                .collect::<Vec<_>>()
                .join("/")
        );
        let home = home.trim_end_matches('/');
        Ok(Layout {
            stable: format!("{home}/.local/bin/baley"),
            versions: format!("{home}/.local/lib/crenshawdev/baley/versions"),
        })
    }

    /// The stable path, `~/.local/bin/baley`.
    pub fn stable_path(&self) -> &Path {
        Path::new(&self.stable)
    }

    /// The folder that holds one subfolder per staged version.
    pub fn versions_folder(&self) -> &Path {
        Path::new(&self.versions)
    }

    /// The installation's identity: the stable path as text, home expanded,
    /// the link not followed.
    pub fn installation(&self) -> &str {
        &self.stable
    }

    /// The binary of a staged version, `<versions>/<version>/baley`.
    pub fn staged_binary(&self, version: Version) -> PathBuf {
        PathBuf::from(format!("{}/{version}/baley", self.versions))
    }
}

/// What a look at the stable path found, without judging it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StablePath {
    /// Nothing is at the stable path.
    Nothing,
    /// A regular file or a folder is there, not a link.
    NotALink,
    /// A symbolic link, with its target text as stored and what following it
    /// found.
    Link {
        /// The link's stored target, exactly as `read_link` gave it.
        target: PathBuf,
        /// What the target is once followed.
        followed: Followed,
    },
}

/// What following a link to its target found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// Nothing is there: the link dangles.
    Missing,
    /// A regular file.
    RegularFile,
    /// Anything else, such as a folder.
    Other,
}

/// What occupies the stable path when Baley manages no version there. A
/// receipt names it so the owner can see what an update left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupant {
    /// Nothing is at the stable path.
    Nothing,
    /// A file or folder that is not a link, such as the owner's own build.
    NotALink,
    /// A link that is relative, or points outside the versions folder.
    LinkOutsideVersions,
    /// A link into the versions folder whose folder name is not a version.
    LinkToNonVersion,
    /// A link into a version's folder whose file is not named `baley`.
    LinkToOtherFile,
    /// A link into the versions folder whose target is gone.
    DanglingLink,
    /// A link into the versions folder whose target is not a regular file.
    LinkToNonFile,
}

/// The version the stable path runs, or why Baley manages none there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Active {
    /// The stable path links to exactly this staged version's binary.
    Version(Version),
    /// Baley manages nothing at the stable path.
    NotManaged(Occupant),
}

/// Judges an observation of the stable path against the layout.
///
/// A version is active only when the link's stored target is, as text,
/// `<versions>/<version>/baley` under this layout's own versions folder, and
/// following it finds a regular file. A link judged by its last folder names
/// alone would take someone else's `versions/0.2.0/baley` for ours.
pub fn judge_active(layout: &Layout, seen: &StablePath) -> Active {
    let (target, followed) = match seen {
        StablePath::Nothing => return Active::NotManaged(Occupant::Nothing),
        StablePath::NotALink => return Active::NotManaged(Occupant::NotALink),
        StablePath::Link { target, followed } => (target, *followed),
    };
    let rest = target
        .to_str()
        .and_then(|text| text.strip_prefix(layout.versions.as_str()))
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty());
    let Some(rest) = rest else {
        return Active::NotManaged(Occupant::LinkOutsideVersions);
    };
    let (folder, file) = rest.split_once('/').unwrap_or((rest, ""));
    let Ok(version) = Version::parse(folder) else {
        return Active::NotManaged(Occupant::LinkToNonVersion);
    };
    if file != "baley" {
        return Active::NotManaged(Occupant::LinkToOtherFile);
    }
    match followed {
        Followed::Missing => Active::NotManaged(Occupant::DanglingLink),
        Followed::Other => Active::NotManaged(Occupant::LinkToNonFile),
        Followed::RegularFile => Active::Version(version),
    }
}

/// One child of the versions folder as the gatherer saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Child {
    /// The entry's name.
    pub name: OsString,
    /// Whether the entry is a folder.
    pub is_folder: bool,
    /// Whether the entry holds a regular file named `baley`.
    pub holds_binary: bool,
}

/// The versions the folder holds, lowest first. A child counts only when it
/// is a folder named as a version and holds a regular `baley`; a partial
/// download, an empty folder or an unrelated entry is ignored.
pub fn versions_present(children: &[Child]) -> Vec<Version> {
    let mut found: Vec<Version> = children
        .iter()
        .filter(|child| child.is_folder && child.holds_binary)
        .filter_map(|child| child.name.to_str())
        .filter_map(|name| Version::parse(name).ok())
        .collect();
    found.sort();
    found
}

/// Uses one generation of the link for both its target text and followed facts.
fn followed_path(link: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        link.parent().unwrap_or(Path::new("")).join(target)
    }
}

/// Looks at the stable path without following it, reads its target once,
/// then reads the followed facts from that captured target.
/// Owns no rule: [`judge_active`] reads the result.
pub fn gather_stable(layout: &Layout) -> io::Result<StablePath> {
    let path = layout.stable_path();
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(StablePath::Nothing),
        Err(error) => return Err(error),
    };
    if !meta.file_type().is_symlink() {
        return Ok(StablePath::NotALink);
    }
    let target = fs::read_link(path)?;
    let followed = match fs::metadata(followed_path(path, &target)) {
        Ok(meta) if meta.is_file() => Followed::RegularFile,
        Ok(_) => Followed::Other,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Followed::Missing,
        Err(error) => return Err(error),
    };
    Ok(StablePath::Link { target, followed })
}

/// Lists the versions folder's children. A folder that does not exist yet
/// lists nothing. Owns no rule: [`versions_present`] reads the result.
pub fn gather_children(layout: &Layout) -> io::Result<Vec<Child>> {
    let entries = match fs::read_dir(layout.versions_folder()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut children = Vec::new();
    for entry in entries {
        let entry = entry?;
        let is_folder = entry.file_type()?.is_dir();
        let holds_binary = is_folder
            && fs::symlink_metadata(entry.path().join("baley"))
                .is_ok_and(|meta| meta.file_type().is_file());
        children.push(Child {
            name: entry.file_name(),
            is_folder,
            holds_binary,
        });
    }
    Ok(children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    const VERSIONS: &str = "/home/o/.local/lib/crenshawdev/baley/versions";

    fn env(home: Option<OsString>) -> Environment {
        Environment {
            home,
            ..Environment::default()
        }
    }

    fn layout() -> Layout {
        Layout::resolve(&env(Some("/home/o".into()))).unwrap()
    }

    fn link(target: &str, followed: Followed) -> StablePath {
        StablePath::Link {
            target: target.into(),
            followed,
        }
    }

    #[test]
    fn a_stable_path_moved_by_baley_home_or_xdg_or_spelled_twice_is_caught() {
        let moved = Environment {
            baley_home: Some("/b".into()),
            xdg_config_home: Some("/x/c".into()),
            xdg_data_home: Some("/x/d".into()),
            home: Some("/home/o".into()),
        };
        for env in [moved, env(Some("/home/o/".into()))] {
            let layout = Layout::resolve(&env).unwrap();
            assert_eq!(layout.stable_path(), Path::new("/home/o/.local/bin/baley"));
            assert_eq!(layout.versions_folder(), Path::new(VERSIONS));
            assert_eq!(layout.installation(), "/home/o/.local/bin/baley");
            assert_eq!(
                layout.staged_binary(Version::parse("0.2.0").unwrap()),
                Path::new("/home/o/.local/lib/crenshawdev/baley/versions/0.2.0/baley")
            );
        }
    }

    #[test]
    fn a_home_spelled_with_dot_components_or_repeated_slashes_giving_a_second_identity_is_caught() {
        for home in [
            "/home/o/.",
            "/home/o//",
            "/home//o",
            "/home/./o/./",
            "/./home/o",
        ] {
            let layout = Layout::resolve(&env(Some(home.into()))).unwrap();
            assert_eq!(layout.stable_path().as_os_str(), "/home/o/.local/bin/baley");
            assert_eq!(layout.versions_folder().as_os_str(), VERSIONS);
            assert_eq!(layout.installation(), "/home/o/.local/bin/baley");
        }
    }

    #[test]
    fn an_unusable_home_given_a_stable_path_is_caught() {
        let cases = [
            (None, HomeRefusal::Unset),
            (Some(OsString::new()), HomeRefusal::Empty),
            (
                Some("home/o".into()),
                HomeRefusal::Relative("home/o".into()),
            ),
            (
                Some(OsString::from_vec(b"/home/\xff".to_vec())),
                HomeRefusal::NotUtf8("/home/\u{fffd}".into()),
            ),
            (
                Some("/home/o/../p".into()),
                HomeRefusal::ParentComponent("/home/o/../p".into()),
            ),
            (
                Some("/home/o/..".into()),
                HomeRefusal::ParentComponent("/home/o/..".into()),
            ),
        ];
        for (home, refusal) in cases {
            let got = Layout::resolve(&env(home)).expect_err("HOME is unusable");
            assert_eq!(got, refusal);
            assert!(got.to_string().starts_with("HOME "), "{got}");
        }
    }

    #[test]
    fn a_relative_link_target_followed_from_the_wrong_folder_is_caught() {
        let link = Path::new("/home/o/.local/bin/baley");
        assert_eq!(
            followed_path(
                link,
                Path::new("../lib/crenshawdev/baley/versions/0.2.0/baley")
            )
            .as_os_str(),
            "/home/o/.local/bin/../lib/crenshawdev/baley/versions/0.2.0/baley"
        );
        let target = Path::new("/home/o/.local/lib/crenshawdev/baley/versions/0.2.0/baley");
        assert_eq!(followed_path(link, target).as_os_str(), target.as_os_str());
    }

    #[test]
    fn an_unmanaged_stable_path_read_as_an_active_version_is_caught() {
        let staged = format!("{VERSIONS}/0.2.0/baley");
        let cases = [
            (StablePath::Nothing, Active::NotManaged(Occupant::Nothing)),
            (StablePath::NotALink, Active::NotManaged(Occupant::NotALink)),
            (
                link(&staged, Followed::RegularFile),
                Active::Version(Version::parse("0.2.0").unwrap()),
            ),
            (
                link(&staged, Followed::Missing),
                Active::NotManaged(Occupant::DanglingLink),
            ),
            (
                link("/usr/local/bin/baley", Followed::RegularFile),
                Active::NotManaged(Occupant::LinkOutsideVersions),
            ),
            (
                link("/opt/mybuild/versions/0.2.0/baley", Followed::RegularFile),
                Active::NotManaged(Occupant::LinkOutsideVersions),
            ),
            (
                link(
                    "../lib/crenshawdev/baley/versions/0.2.0/baley",
                    Followed::RegularFile,
                ),
                Active::NotManaged(Occupant::LinkOutsideVersions),
            ),
            (
                link(&format!("{VERSIONS}/v0.2.0/baley"), Followed::RegularFile),
                Active::NotManaged(Occupant::LinkToNonVersion),
            ),
            (
                link(
                    &format!("{VERSIONS}/0.2.0/baley-old"),
                    Followed::RegularFile,
                ),
                Active::NotManaged(Occupant::LinkToOtherFile),
            ),
            (
                link(&staged, Followed::Other),
                Active::NotManaged(Occupant::LinkToNonFile),
            ),
        ];
        for (seen, expected) in cases {
            assert_eq!(judge_active(&layout(), &seen), expected, "{seen:?}");
        }
    }

    #[test]
    fn a_stray_entry_in_the_versions_folder_counted_as_a_version_is_caught() {
        let child = |name: &str, is_folder, holds_binary| Child {
            name: name.into(),
            is_folder,
            holds_binary,
        };
        let listing = [
            child("0.10.0", true, true),
            child(".baley-0.4.0.partial", false, false),
            child("0.1.0", true, true),
            child("0.2.0", true, false),
            child("0.9.0", true, true),
            child("0.3.0", true, false),
            child("notes", true, true),
        ];
        let expected: Vec<Version> = ["0.1.0", "0.9.0", "0.10.0"]
            .into_iter()
            .map(|text| Version::parse(text).unwrap())
            .collect();
        assert_eq!(versions_present(&listing), expected);
    }
}
