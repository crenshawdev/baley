//! Installed Claude Code paths derived from supplied HOME, CLAUDE_CONFIG_DIR
//! and a stub manifest. The guard, install and the doctor share this
//! derivation so they agree on every path. Nothing is read or created.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::folders::Environment;
use crate::update::installation::{HomeRefusal, Layout};

use super::executable::{Executable, PathFault, judge};
use super::placement::{Placement, PlacementMap, PlacementRefusal};
use super::stubs::Entry;

/// The host files and installation paths, whether or not install has run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Claude Code's configuration folder, holding settings and skills.
    pub claude_folder: PathBuf,
    /// The one document holding the hook and security settings.
    pub settings_file: PathBuf,
    /// The user-scope MCP registration file.
    pub registration_file: PathBuf,
    /// The stable executable and versions folder from the same HOME.
    pub layout: Layout,
    /// Every stub and JSON artifact, with the executable and versions folder.
    pub placements: PlacementMap,
}

/// Why the supplied values cannot place the installed artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// HOME cannot place the stable executable.
    Home(HomeRefusal),
    /// CLAUDE_CONFIG_DIR is set to an unusable path.
    ConfigDirectory(PathFault),
    /// The derived artifact paths cannot form a placement map.
    Placement(PlacementRefusal),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Home(refusal) => refusal.fmt(f),
            Self::ConfigDirectory(fault) => write!(
                f,
                "CLAUDE_CONFIG_DIR {fault}; set it to a non-empty absolute UTF-8 path for Claude Code's configuration folder, or unset it to use HOME/.claude"
            ),
            Self::Placement(refusal) => refusal.fmt(f),
        }
    }
}

/// Derives installed paths without reading the environment or filesystem.
/// Only HOME is used from `env`. A set configuration override must be
/// non-empty, absolute and UTF-8, and its trailing slashes are removed.
/// HOME must be usable even with an override, since it places the binary.
pub fn resolve(
    env: &Environment,
    claude_config_dir: Option<OsString>,
    manifest: &[Entry],
) -> Result<Installed, Refusal> {
    let layout = Layout::resolve(env).map_err(Refusal::Home)?;
    let (claude_folder, registration_file) = match claude_config_dir {
        Some(value) => {
            let text = judge(&value).map_err(Refusal::ConfigDirectory)?;
            let folder = text.trim_end_matches('/');
            let folder = PathBuf::from(if folder.is_empty() { "/" } else { folder });
            let registration = folder.join(".claude.json");
            (folder, registration)
        }
        None => (
            layout.home_folder().join(".claude"),
            layout.home_folder().join(".claude.json"),
        ),
    };
    let settings_file = claude_folder.join("settings.json");
    let stubs = manifest
        .iter()
        .map(|entry| {
            let path = claude_folder
                .join("skills")
                .join(&entry.identity)
                .join("SKILL.md");
            (entry.clone(), Placement::At(path))
        })
        .collect();
    let executable =
        Executable::new(layout.stable_path()).expect("the layout's stable path is absolute UTF-8");
    let placements = PlacementMap::new(
        executable,
        stubs,
        Placement::At(registration_file.clone()),
        Placement::At(settings_file.clone()),
        Placement::At(settings_file.clone()),
        Placement::At(layout.versions_folder().to_path_buf()),
    )
    .map_err(Refusal::Placement)?;
    Ok(Installed {
        claude_folder,
        settings_file,
        registration_file,
        layout,
        placements,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    use super::{Refusal, resolve};
    use crate::folders::Environment;
    use crate::host_artifacts::executable::PathFault;
    use crate::host_artifacts::placement::{Artifact, Document};
    use crate::host_artifacts::stubs::{front_doors, manifest};
    use crate::update::installation::HomeRefusal;

    #[test]
    fn a_claude_target_or_protected_file_outside_claude_config_dir_is_caught() {
        let env = Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        };
        let entries = manifest(&front_doors()).unwrap();
        for (value, folder) in [
            ("/c/claude", "/c/claude"),
            ("/c/claude/", "/c/claude"),
            ("/home/o/my claude", "/home/o/my claude"),
        ] {
            let installed = resolve(&env, Some(value.into()), &entries).unwrap();
            let settings = format!("{folder}/settings.json");
            let registration = format!("{folder}/.claude.json");
            assert_eq!(installed.claude_folder.as_os_str(), folder);
            assert_eq!(installed.settings_file.as_os_str(), settings.as_str());
            assert_eq!(
                installed.registration_file.as_os_str(),
                registration.as_str()
            );
            let files: Vec<_> = installed
                .placements
                .expected_files()
                .into_iter()
                .map(|file| (file.artifact, file.path.to_path_buf()))
                .collect();
            assert_eq!(
                files,
                [
                    (
                        Artifact::Stub("bal-capture".into()),
                        PathBuf::from(format!("{folder}/skills/bal-capture/SKILL.md")),
                    ),
                    (
                        Artifact::Stub("bal-help".into()),
                        PathBuf::from(format!("{folder}/skills/bal-help/SKILL.md")),
                    ),
                    (Artifact::Registration, PathBuf::from(registration)),
                    (Artifact::Hook, PathBuf::from(&settings)),
                    (Artifact::Settings, PathBuf::from(settings)),
                ]
            );
            assert_eq!(
                installed.placements.executable().as_str(),
                "/home/o/.local/bin/baley"
            );
            for path in installed.placements.protected_paths() {
                if path != Path::new("/home/o/.local/bin/baley") {
                    assert!(path.to_str().unwrap().starts_with(&format!("{folder}/")));
                }
            }
        }
    }

    #[test]
    fn a_claude_config_dir_that_is_empty_relative_or_not_utf8_used_as_a_folder_is_caught() {
        let env = Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        };
        let entries = manifest(&front_doors()).unwrap();
        for (value, fault, cause) in [
            (OsString::new(), PathFault::Empty, "is empty"),
            ("claude".into(), PathFault::Relative, "is not absolute"),
            (
                OsString::from_vec(b"/c/\xff".to_vec()),
                PathFault::NotUtf8,
                "is not UTF-8",
            ),
        ] {
            let refusal = resolve(&env, Some(value), &entries).unwrap_err();
            assert_eq!(refusal, Refusal::ConfigDirectory(fault));
            let text = refusal.to_string();
            assert!(
                text.starts_with(&format!("CLAUDE_CONFIG_DIR {cause};")),
                "{text}"
            );
            assert!(
                text.contains("set it to a non-empty absolute UTF-8 path"),
                "{text}"
            );
        }
    }

    #[test]
    fn a_default_claude_folder_moved_by_baley_home_or_xdg_is_caught() {
        let entries = manifest(&front_doors()).unwrap();
        for env in [
            Environment {
                home: Some("/home/o".into()),
                baley_home: Some("/b".into()),
                xdg_config_home: Some("/x/c".into()),
                xdg_data_home: Some("/x/d".into()),
            },
            Environment {
                home: Some("/home/o/".into()),
                ..Environment::default()
            },
        ] {
            let installed = resolve(&env, None, &entries).unwrap();
            assert_eq!(installed.claude_folder.as_os_str(), "/home/o/.claude");
            assert_eq!(
                installed.settings_file.as_os_str(),
                "/home/o/.claude/settings.json"
            );
            assert_eq!(
                installed.registration_file.as_os_str(),
                "/home/o/.claude.json"
            );
            let stubs: Vec<_> = installed
                .placements
                .expected_files()
                .into_iter()
                .filter(|file| file.stub.is_some())
                .map(|file| file.path.as_os_str())
                .collect();
            assert_eq!(
                stubs,
                [
                    "/home/o/.claude/skills/bal-capture/SKILL.md",
                    "/home/o/.claude/skills/bal-help/SKILL.md",
                ]
            );
            assert_eq!(installed.layout.home_folder().as_os_str(), "/home/o");
            assert_eq!(
                installed.layout.stable_path().as_os_str(),
                "/home/o/.local/bin/baley"
            );
            assert_eq!(
                installed.placements.executable().as_str(),
                "/home/o/.local/bin/baley"
            );
            assert_eq!(
                installed.layout.versions_folder().as_os_str(),
                "/home/o/.local/lib/crenshawdev/baley/versions"
            );
        }
    }

    #[test]
    fn an_unusable_home_given_claude_placements_is_caught() {
        let entries = manifest(&front_doors()).unwrap();
        for (home, expected, text) in [
            (
                None,
                HomeRefusal::Unset,
                "HOME is not set; set it to your home folder",
            ),
            (
                Some("home/o".into()),
                HomeRefusal::Relative("home/o".into()),
                "HOME is \"home/o\", which is not an absolute path; set it to the absolute path of your home folder",
            ),
        ] {
            let env = Environment {
                home,
                ..Environment::default()
            };
            let refusal = resolve(&env, Some("/c/claude".into()), &entries).unwrap_err();
            assert_eq!(refusal, Refusal::Home(expected));
            assert_eq!(refusal.to_string(), text);
        }
    }

    #[test]
    fn a_placement_map_with_an_unknown_artifact_or_no_versions_folder_is_caught() {
        let env = Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        };
        let entries = manifest(&front_doors()).unwrap();
        let installed = resolve(&env, None, &entries).unwrap();
        let map = &installed.placements;
        assert!(map.not_installed().is_empty());
        assert_eq!(
            map.write_only_folders(),
            [PathBuf::from(
                "/home/o/.local/lib/crenshawdev/baley/versions"
            )]
        );
        assert_eq!(
            map.documents(),
            [
                Document {
                    path: Path::new("/home/o/.claude.json"),
                    artifacts: vec![Artifact::Registration],
                },
                Document {
                    path: Path::new("/home/o/.claude/settings.json"),
                    artifacts: vec![Artifact::Hook, Artifact::Settings],
                },
            ]
        );
        assert_eq!(
            map.protected_paths(),
            [
                PathBuf::from("/home/o/.claude/skills/bal-capture/SKILL.md"),
                PathBuf::from("/home/o/.claude/skills/bal-help/SKILL.md"),
                PathBuf::from("/home/o/.claude.json"),
                PathBuf::from("/home/o/.claude/settings.json"),
                PathBuf::from("/home/o/.local/bin/baley"),
            ]
        );

        let extra = manifest(&[("bal-extra", "Extra")]).unwrap();
        let installed = resolve(&env, None, &extra).unwrap();
        assert_eq!(
            installed.placements.protected_paths(),
            [
                PathBuf::from("/home/o/.claude/skills/bal-extra/SKILL.md"),
                PathBuf::from("/home/o/.claude.json"),
                PathBuf::from("/home/o/.claude/settings.json"),
                PathBuf::from("/home/o/.local/bin/baley"),
            ]
        );
    }
}
