//! Platform folders resolved from supplied environment values.
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::{ffi::OsStrExt, fs::DirBuilderExt, fs::PermissionsExt};
use std::path::{Path, PathBuf};

/// The platform whose folder conventions apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// XDG configuration and data folders.
    Linux,
    /// One Application Support folder.
    MacOs,
}
impl Platform {
    /// Reads the build target's platform.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Values exactly as supplied, with `None` for an unset variable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    /// The override for both folders.
    pub baley_home: Option<OsString>,
    /// Linux's configuration root.
    pub xdg_config_home: Option<OsString>,
    /// Linux's data root.
    pub xdg_data_home: Option<OsString>,
    /// The user's home for platform defaults.
    pub home: Option<OsString>,
}
impl Environment {
    /// Reads the four folder variables without converting their bytes.
    pub fn read() -> Self {
        Self {
            baley_home: std::env::var_os("BALEY_HOME"),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME"),
            xdg_data_home: std::env::var_os("XDG_DATA_HOME"),
            home: std::env::var_os("HOME"),
        }
    }
}

/// Baley's configuration folder and ledger home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    /// Holds `config.toml` and `keys.env`.
    pub config: PathBuf,
    /// Holds the ledger and its store files.
    pub home: PathBuf,
}

/// A supplied location cannot safely select Baley's folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderRefusal {
    /// An empty override must never fall back to the user's ledger.
    BaleyHomeEmpty,
    /// The override is not absolute.
    BaleyHomeRelative(OsString),
    /// The override does not end in a folder name.
    BaleyHomeUnnamed(OsString),
    /// A platform default needs HOME, but it is unset.
    UserHomeUnset,
    /// A platform default needs HOME, but it is empty.
    UserHomeEmpty,
    /// A platform default needs HOME, but it is relative.
    UserHomeRelative(OsString),
}

impl Folders {
    /// Resolves both folders together without reading the environment or filesystem.
    pub fn resolve(platform: Platform, env: &Environment) -> Result<Self, FolderRefusal> {
        if let Some(value) = &env.baley_home {
            let path = Path::new(value);
            if value.is_empty() {
                return Err(FolderRefusal::BaleyHomeEmpty);
            }
            if !path.is_absolute() {
                return Err(FolderRefusal::BaleyHomeRelative(value.clone()));
            }
            // Path components discard a trailing dot, so inspect the original bytes.
            let last = value
                .as_bytes()
                .rsplit(|b| *b == b'/')
                .find(|s| !s.is_empty());
            if path.file_name().is_none() || last == Some(b".".as_slice()) {
                return Err(FolderRefusal::BaleyHomeUnnamed(value.clone()));
            }
            return Ok(Self {
                config: path.into(),
                home: path.into(),
            });
        }
        let user_home = || -> Result<&Path, FolderRefusal> {
            let value = env.home.as_ref().ok_or(FolderRefusal::UserHomeUnset)?;
            if value.is_empty() {
                return Err(FolderRefusal::UserHomeEmpty);
            }
            let path = Path::new(value);
            if !path.is_absolute() {
                return Err(FolderRefusal::UserHomeRelative(value.clone()));
            }
            Ok(path)
        };
        if platform == Platform::MacOs {
            let home = user_home()?.join("Library/Application Support/crenshawdev/baley");
            return Ok(Self {
                config: home.clone(),
                home,
            });
        }
        let root = |value: &Option<OsString>, fallback: &str| -> Result<PathBuf, FolderRefusal> {
            match value.as_deref().map(Path::new).filter(|p| p.is_absolute()) {
                Some(path) => Ok(path.to_path_buf()),
                None => Ok(user_home()?.join(fallback)),
            }
        };
        Ok(Self {
            config: root(&env.xdg_config_home, ".config")?.join("crenshawdev/baley"),
            home: root(&env.xdg_data_home, ".local/share")?.join("crenshawdev/baley"),
        })
    }
}

impl fmt::Display for FolderRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BaleyHomeEmpty => f.write_str("baley-home-invalid: BALEY_HOME is set but empty; set it to an absolute path, or unset it to use Baley's own folders"),
            Self::BaleyHomeRelative(value) => write!(f, "baley-home-invalid: BALEY_HOME is {}, which is not an absolute path; set it to an absolute path", value.to_string_lossy()),
            Self::BaleyHomeUnnamed(value) => write!(f, "baley-home-invalid: BALEY_HOME is {}, which does not end in a folder name; set it to an absolute path ending in the home folder's name", value.to_string_lossy()),
            refusal => {
                f.write_str("user-home-invalid: HOME ")?;
                match refusal {
                    Self::UserHomeUnset => f.write_str("is not set")?,
                    Self::UserHomeEmpty => f.write_str("is set but empty")?,
                    Self::UserHomeRelative(value) => write!(f, "is {}, which is not an absolute path", value.to_string_lossy())?,
                    _ => unreachable!(),
                }
                f.write_str("; Baley needs it to find its folders; set HOME, or set BALEY_HOME to an absolute path")
            }
        }
    }
}

/// Creates a private folder and default-mode parents, leaving existing paths unchanged.
pub fn create_private(path: &Path) -> io::Result<()> {
    let existed = fs::symlink_metadata(path).is_ok();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let created = DirBuilder::new().mode(0o700).create(path);
    match after_create(existed, created)? {
        Created::Done => Ok(()),
        Created::ByItsParents => fs::set_permissions(path, fs::Permissions::from_mode(0o700)),
    }
}

/// What creating a home left to do.
#[derive(Debug, PartialEq, Eq)]
enum Created {
    Done,
    /// A `..` in the path let creating the parents make the home itself, with
    /// the default mode, so it is new and must still be made private.
    ByItsParents,
}

fn after_create(existed: bool, created: io::Result<()>) -> io::Result<Created> {
    match created {
        Ok(()) => Ok(Created::Done),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && existed => Ok(Created::Done),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(Created::ByItsParents),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn home_made_by_its_own_parents_is_still_made_private() {
        let exists = || Err(io::Error::from(io::ErrorKind::AlreadyExists));
        assert_eq!(
            after_create(false, exists()).unwrap(),
            Created::ByItsParents
        );
    }
    #[test]
    fn existing_home_is_left_unchanged() {
        let exists = || Err(io::Error::from(io::ErrorKind::AlreadyExists));
        assert_eq!(after_create(true, exists()).unwrap(), Created::Done);
    }
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn env(config: Option<&str>, data: Option<&str>, home: Option<&str>) -> Environment {
        Environment {
            baley_home: None,
            xdg_config_home: config.map(Into::into),
            xdg_data_home: data.map(Into::into),
            home: home.map(Into::into),
        }
    }
    fn expected(config: &str, home: &str) -> Result<Folders, FolderRefusal> {
        Ok(Folders {
            config: config.into(),
            home: home.into(),
        })
    }
    #[test]
    fn linux_does_not_swap_or_drop_xdg_vendor_folders() {
        assert_eq!(
            Folders::resolve(
                Platform::Linux,
                &env(Some("/x/c"), Some("/x/d"), Some("/h"))
            ),
            expected("/x/c/crenshawdev/baley", "/x/d/crenshawdev/baley")
        );
    }
    #[test]
    fn empty_xdg_values_do_not_make_relative_folders() {
        assert_eq!(
            Folders::resolve(Platform::Linux, &env(Some(""), Some(""), Some("/h"))),
            expected(
                "/h/.config/crenshawdev/baley",
                "/h/.local/share/crenshawdev/baley"
            )
        );
    }
    #[test]
    fn relative_xdg_values_do_not_override_defaults() {
        assert_eq!(
            Folders::resolve(
                Platform::Linux,
                &env(Some("rel/dir"), Some("rel/dir"), Some("/h"))
            ),
            expected(
                "/h/.config/crenshawdev/baley",
                "/h/.local/share/crenshawdev/baley"
            )
        );
    }
    #[test]
    fn absolute_xdg_roots_do_not_require_home() {
        assert_eq!(
            Folders::resolve(Platform::Linux, &env(Some("/x/c"), Some("/x/d"), None)),
            expected("/x/c/crenshawdev/baley", "/x/d/crenshawdev/baley")
        );
    }
    #[test]
    fn invalid_data_root_does_not_discard_config_root() {
        assert_eq!(
            Folders::resolve(Platform::Linux, &env(Some("/x/c"), Some("rel"), Some("/h"))),
            expected(
                "/x/c/crenshawdev/baley",
                "/h/.local/share/crenshawdev/baley"
            )
        );
    }
    #[test]
    fn invalid_config_root_does_not_discard_data_root() {
        assert_eq!(
            Folders::resolve(Platform::Linux, &env(Some(""), Some("/x/d"), Some("/h"))),
            expected("/h/.config/crenshawdev/baley", "/x/d/crenshawdev/baley")
        );
    }
    #[test]
    fn one_xdg_fallback_still_requires_home() {
        assert_eq!(
            Folders::resolve(Platform::Linux, &env(Some("/x/c"), None, None)),
            Err(FolderRefusal::UserHomeUnset)
        );
    }
    #[test]
    fn unnamed_override_cannot_reuse_a_default_mode_parent() {
        for value in ["/", "/b/.", "/b/.///", "/a/b/.."] {
            let mut env = env(None, None, None);
            env.baley_home = Some(value.into());
            let refusal = Folders::resolve(Platform::Linux, &env).unwrap_err();
            assert_eq!(refusal, FolderRefusal::BaleyHomeUnnamed(value.into()));
            assert_eq!(
                refusal.to_string(),
                format!(
                    "baley-home-invalid: BALEY_HOME is {value}, which does not end in a folder name; set it to an absolute path ending in the home folder's name"
                )
            );
        }
    }
    #[test]
    fn raw_non_utf8_trailing_dot_is_not_normalized_away() {
        let value = OsString::from_vec(b"/b/\xff/.".to_vec());
        let mut env = env(None, None, None);
        env.baley_home = Some(value.clone());
        assert_eq!(
            Folders::resolve(Platform::Linux, &env),
            Err(FolderRefusal::BaleyHomeUnnamed(value))
        );
    }
    #[test]
    fn non_utf8_named_override_is_preserved() {
        let value = OsString::from_vec(b"/b/\xff".to_vec());
        let mut env = env(None, None, None);
        env.baley_home = Some(value.clone());
        assert_eq!(
            Folders::resolve(Platform::Linux, &env),
            Ok(Folders {
                config: value.clone().into(),
                home: value.into()
            })
        );
    }
    #[test]
    fn macos_does_not_use_xdg_or_separate_config_from_data() {
        assert_eq!(
            Folders::resolve(
                Platform::MacOs,
                &env(Some("/x/c"), Some("/x/d"), Some("/h"))
            ),
            expected(
                "/h/Library/Application Support/crenshawdev/baley",
                "/h/Library/Application Support/crenshawdev/baley"
            )
        );
    }
    #[test]
    fn override_keeps_config_and_keys_beside_the_ledger() {
        for platform in [Platform::Linux, Platform::MacOs] {
            let mut env = env(Some("/x/c"), Some("/x/d"), None);
            env.baley_home = Some("/b".into());
            assert_eq!(Folders::resolve(platform, &env), expected("/b", "/b"));
        }
    }
    #[test]
    fn empty_override_never_falls_back_to_the_owners_ledger() {
        let mut env = env(None, None, Some("/h"));
        env.baley_home = Some("".into());
        let refusal = Folders::resolve(Platform::Linux, &env).unwrap_err();
        assert_eq!(refusal, FolderRefusal::BaleyHomeEmpty);
        assert_eq!(
            refusal.to_string(),
            "baley-home-invalid: BALEY_HOME is set but empty; set it to an absolute path, or unset it to use Baley's own folders"
        );
    }
    #[test]
    fn relative_override_cannot_create_a_ledger_in_the_checkout() {
        for value in ["dev/home", "~/x"] {
            let mut env = env(None, None, Some("/h"));
            env.baley_home = Some(value.into());
            let refusal = Folders::resolve(Platform::Linux, &env).unwrap_err();
            assert_eq!(refusal, FolderRefusal::BaleyHomeRelative(value.into()));
            assert_eq!(
                refusal.to_string(),
                format!(
                    "baley-home-invalid: BALEY_HOME is {value}, which is not an absolute path; set it to an absolute path"
                )
            );
        }
    }
    #[test]
    fn unusable_user_home_cannot_supply_platform_defaults() {
        for platform in [Platform::Linux, Platform::MacOs] {
            for (value, expected, message) in [
                (None, FolderRefusal::UserHomeUnset, "is not set"),
                (Some(""), FolderRefusal::UserHomeEmpty, "is set but empty"),
                (
                    Some("rel"),
                    FolderRefusal::UserHomeRelative("rel".into()),
                    "is rel, which is not an absolute path",
                ),
            ] {
                let refusal = Folders::resolve(platform, &env(None, None, value)).unwrap_err();
                assert_eq!(refusal, expected);
                assert_eq!(
                    refusal.to_string(),
                    format!(
                        "user-home-invalid: HOME {message}; Baley needs it to find its folders; set HOME, or set BALEY_HOME to an absolute path"
                    )
                );
            }
        }
    }
}
