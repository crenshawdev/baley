//! The sandbox and permission proposal that keeps agents out of Baley's home
//! and config folder, off the files Baley places and out of the folder of
//! staged versions (design 0012 section 5, ADR 0033; D-13, D-14, D-15, D-19,
//! D-20).
//!
//! The proposal is content, not a write. `baley install` merges it into
//! Claude Code's settings in Build 3 T15, and T12 applies it by hand. It reads
//! nothing: every path is a supplied value, and a path that cannot be written
//! as a rule is reported beside the content instead of rendered.
//!
//! It holds no `sandbox.network` key. Allowing the owner's chosen providers'
//! API hosts is T15's: once the key ADR and the delivery ADR record the host
//! list, T15 adds it to [`propose`]'s inputs and renders it as
//! `sandbox.network.allowedDomains`.
//!
//! Host facts the keys and spellings depend on, confirmed on 2026-10-06
//! against Claude Code's published settings reference, sandboxing and
//! permissions pages and the configuration schema bundled in Claude Code
//! 2.1.292:
//! - `enabled`, `failIfUnavailable` and `allowUnsandboxedCommands` sit
//!   directly under `sandbox`, and `denyRead`, `denyWrite`, `allowRead` and
//!   `allowWrite` under `sandbox.filesystem`. A key under another parent is
//!   ignored.
//! - Sandbox list entries are ordinary absolute paths. `Read` and `Edit`
//!   rules are gitignore patterns in which `//` starts an absolute path and a
//!   single `/` is relative to the settings file.
//! - `Edit` rules cover every built-in tool that edits files, Write and
//!   NotebookEdit included. A `Write(...)` or `NotebookEdit(...)` path rule
//!   is accepted and never consulted.
//! - Parentheses and inner spaces in a rule path are literal. Hand-written
//!   rules are not escaped, so `*`, `?`, `[` and `\` keep their pattern
//!   meaning and a trailing space is dropped, and on Linux a write-list entry
//!   holding `*`, `?` or `[` is skipped. No escape works in both places, so a
//!   path holding one is reported, not rendered.
//! - The sandbox covers Bash, PowerShell and Monitor commands and the
//!   processes they start. Hooks and local MCP servers run outside it, which
//!   is why the executable needs no exclusion and gets none.
//! - On Linux the sandbox holds a write denial on a file that does not exist
//!   yet, so a path can be protected before anything is placed there.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::executable::{Executable, PathFault, judge};
use crate::folders::Folders;

/// Why a supplied path got no entry in the proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unrendered {
    /// The path is not absolute UTF-8, so it is a missing prerequisite
    /// (D-20) and no rule names it.
    MissingPrerequisite {
        /// The path as supplied.
        path: PathBuf,
        /// What is wrong with it.
        fault: PathFault,
    },
    /// The path holds a character Claude Code would not read literally, and
    /// its docs give no escape: an unsupported mechanism for that path.
    UnsupportedCharacter {
        /// The path as supplied.
        path: String,
        /// The first such character, or a space when the path ends in one.
        character: char,
    },
}

impl fmt::Display for Unrendered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unrendered::MissingPrerequisite { path, fault } => {
                write!(f, "the path {} {fault}", path.display())
            }
            Unrendered::UnsupportedCharacter {
                path,
                character: ' ',
            } => write!(
                f,
                "the path {path} ends in a space, which a deny rule would drop"
            ),
            Unrendered::UnsupportedCharacter { path, character } => write!(
                f,
                "the path {path} holds `{character}`, which a deny rule would read as a pattern"
            ),
        }
    }
}

/// Judges one path for the proposal: absolute UTF-8 (D-20), with no
/// character Claude Code would read as pattern syntax and no trailing space.
/// The coverage judge applies the same rule, so a path the proposal cannot
/// render is never judged covered.
pub fn rule_path(path: &Path) -> Result<String, Unrendered> {
    let text = judge(path.as_os_str()).map_err(|fault| Unrendered::MissingPrerequisite {
        path: path.to_path_buf(),
        fault,
    })?;
    let pattern = text.chars().find(|c| matches!(c, '*' | '?' | '[' | '\\'));
    let trailing = text.ends_with(' ').then_some(' ');
    match pattern.or(trailing) {
        Some(character) => Err(Unrendered::UnsupportedCharacter {
            path: text,
            character,
        }),
        None => Ok(text),
    }
}

/// The `Read` deny rule for everything inside a folder. `path` is absolute,
/// so the extra `/` makes the rule's `//` absolute form.
pub fn read_folder_rule(path: &str) -> String {
    format!("Read(/{path}/**)")
}

/// The `Edit` deny rule for everything inside a folder.
pub fn edit_folder_rule(path: &str) -> String {
    format!("Edit(/{path}/**)")
}

/// The `Edit` deny rule for one file.
pub fn edit_file_rule(path: &str) -> String {
    format!("Edit(/{path})")
}

/// The settings content and every supplied path it could not render.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// The `sandbox` and `permissions` content, ready for composition.
    pub settings: Value,
    /// The paths that got no entry, each with why. The content is still
    /// rendered for every other path.
    pub unrendered: Vec<Unrendered>,
}

fn push_unique<T: PartialEq>(list: &mut Vec<T>, item: T) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// The proposal over the resolved folders, the executable, the supplied
/// write-only files (the `baley.toml` paths the caller names and the
/// placement map's protected list) and the write-only folders (the
/// placement map's [`super::placement::PlacementMap::write_only_folders`]).
///
/// Both Baley folders are denied to sandboxed reads and writes and get a
/// `Read` and an `Edit` rule. Each write-only folder, such as the folder of
/// staged versions, is denied to sandboxed writes and gets an `Edit` folder
/// rule, listed after the Baley folders and before the files, and nothing
/// denies its reads, because sandboxed Bash must run the binaries inside it.
/// A folder entry protects what is inside it, which a file entry would not.
/// Each write-only file, and the executable whether or not the list already
/// holds it (D-25), is denied to sandboxed writes and gets an `Edit` rule,
/// and nothing denies its reads (D-15). One folder for home and config gives
/// one set of entries (D-13), and no list holds an entry twice. Nothing
/// excludes a command or re-opens a path (D-19).
pub fn propose(
    folders: &Folders,
    executable: &Executable,
    write_only: &[PathBuf],
    write_only_folders: &[PathBuf],
) -> Proposal {
    let mut unrendered = Vec::new();
    let mut folder_paths: Vec<String> = Vec::new();
    for folder in [&folders.home, &folders.config] {
        match rule_path(folder) {
            Ok(path) => push_unique(&mut folder_paths, path),
            Err(report) => push_unique(&mut unrendered, report),
        }
    }
    let mut write_only_folder_paths: Vec<String> = Vec::new();
    for folder in write_only_folders {
        match rule_path(folder) {
            Ok(path) => push_unique(&mut write_only_folder_paths, path),
            Err(report) => push_unique(&mut unrendered, report),
        }
    }
    let mut files: Vec<String> = Vec::new();
    let supplied = write_only.iter().map(PathBuf::as_path);
    for file in supplied.chain([Path::new(executable.as_str())]) {
        match rule_path(file) {
            Ok(path) => push_unique(&mut files, path),
            Err(report) => push_unique(&mut unrendered, report),
        }
    }

    let mut deny_write = folder_paths.clone();
    let mut rules = Vec::new();
    for folder in &folder_paths {
        rules.push(read_folder_rule(folder));
        rules.push(edit_folder_rule(folder));
    }
    for folder in write_only_folder_paths {
        push_unique(&mut rules, edit_folder_rule(&folder));
        push_unique(&mut deny_write, folder);
    }
    for file in files {
        push_unique(&mut rules, edit_file_rule(&file));
        push_unique(&mut deny_write, file);
    }

    let settings = json!({
        "sandbox": {
            "enabled": true,
            "failIfUnavailable": true,
            "allowUnsandboxedCommands": false,
            "filesystem": {
                "denyRead": folder_paths,
                "denyWrite": deny_write,
            },
        },
        "permissions": {
            "deny": rules,
        },
    });
    Proposal {
        settings,
        unrendered,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    const HOME: &str = "/home/o/.local/share/crenshawdev/baley";
    const CONFIG: &str = "/home/o/.config/crenshawdev/baley";
    const EXECUTABLE: &str = "/home/o/.local/bin/baley";
    const SETTINGS: &str = "/home/o/.claude/settings.json";

    fn folders(home: &str, config: &str) -> Folders {
        Folders {
            home: home.into(),
            config: config.into(),
        }
    }

    fn proposal(folders: &Folders, write_only: &[&str]) -> Proposal {
        let write_only: Vec<PathBuf> = write_only.iter().map(PathBuf::from).collect();
        propose(
            folders,
            &Executable::new(EXECUTABLE).unwrap(),
            &write_only,
            &[],
        )
    }

    fn strings(value: &Value) -> Vec<&str> {
        value
            .as_array()
            .expect("an array")
            .iter()
            .map(|item| item.as_str().expect("a string"))
            .collect()
    }

    #[test]
    fn home_only_protection_missing_the_config_folder_is_caught() {
        // Expected lists follow D-14 and design 0012 section 5.
        let ours = proposal(&folders(HOME, CONFIG), &[]);
        assert!(ours.unrendered.is_empty());
        let filesystem = &ours.settings["sandbox"]["filesystem"];
        assert_eq!(strings(&filesystem["denyRead"]), [HOME, CONFIG]);
        assert_eq!(
            strings(&filesystem["denyWrite"]),
            [HOME, CONFIG, EXECUTABLE]
        );
        assert_eq!(
            strings(&ours.settings["permissions"]["deny"]),
            [
                "Read(//home/o/.local/share/crenshawdev/baley/**)",
                "Edit(//home/o/.local/share/crenshawdev/baley/**)",
                "Read(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.local/bin/baley)",
            ]
        );
    }

    #[test]
    fn a_misspelled_rule_or_a_hook_or_server_sandboxed_by_exclusion_is_caught() {
        let ours = proposal(&folders(HOME, CONFIG), &[SETTINGS]).settings;
        let sandbox = &ours["sandbox"];
        assert_eq!(sandbox["enabled"], true);
        assert_eq!(sandbox["failIfUnavailable"], true);
        assert_eq!(sandbox["allowUnsandboxedCommands"], false);
        let keys: Vec<&String> = sandbox.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "enabled",
                "failIfUnavailable",
                "allowUnsandboxedCommands",
                "filesystem"
            ]
        );
        let lists: Vec<&String> = sandbox["filesystem"].as_object().unwrap().keys().collect();
        assert_eq!(lists, ["denyRead", "denyWrite"]);
        let top: Vec<&String> = ours.as_object().unwrap().keys().collect();
        assert_eq!(top, ["sandbox", "permissions"]);
        let permissions: Vec<&String> = ours["permissions"].as_object().unwrap().keys().collect();
        assert_eq!(permissions, ["deny"]);
        for rule in strings(&ours["permissions"]["deny"]) {
            assert!(
                rule.starts_with("Read(//") || rule.starts_with("Edit(//"),
                "{rule}"
            );
        }
        let text = ours.to_string();
        assert_eq!(text.matches(EXECUTABLE).count(), 2, "{text}");
    }

    #[test]
    fn write_only_protection_widened_to_a_read_rule_is_caught() {
        let toml = "/work/project/baley.toml";
        let ours = proposal(&folders(HOME, CONFIG), &[toml, SETTINGS]);
        let filesystem = &ours.settings["sandbox"]["filesystem"];
        let deny_write = strings(&filesystem["denyWrite"]);
        assert_eq!(deny_write, [HOME, CONFIG, toml, SETTINGS, EXECUTABLE]);
        let rules = strings(&ours.settings["permissions"]["deny"]);
        for file in [toml, SETTINGS, EXECUTABLE] {
            assert!(!strings(&filesystem["denyRead"]).contains(&file), "{file}");
            assert!(rules.contains(&format!("Edit(/{file})").as_str()), "{file}");
            assert!(
                !rules
                    .iter()
                    .any(|rule| rule.starts_with("Read(") && rule.contains(file)),
                "{file}"
            );
        }
    }

    #[test]
    fn one_folder_for_home_and_config_listed_twice_is_caught() {
        let ours = proposal(&folders(HOME, HOME), &[EXECUTABLE, SETTINGS, SETTINGS]);
        let filesystem = &ours.settings["sandbox"]["filesystem"];
        assert_eq!(strings(&filesystem["denyRead"]), [HOME]);
        assert_eq!(
            strings(&filesystem["denyWrite"]),
            [HOME, EXECUTABLE, SETTINGS]
        );
        assert_eq!(
            strings(&ours.settings["permissions"]["deny"]),
            [
                "Read(//home/o/.local/share/crenshawdev/baley/**)",
                "Edit(//home/o/.local/share/crenshawdev/baley/**)",
                "Edit(//home/o/.local/bin/baley)",
                "Edit(//home/o/.claude/settings.json)",
            ]
        );
    }

    #[test]
    fn a_parenthesis_or_inner_space_reported_or_escaped_instead_of_written_as_spelled_is_caught() {
        // Claude Code reads parentheses and inner spaces in a rule path
        // literally, so the rule names the path as spelled.
        let home = "/home/o w/data (2024)/baley";
        let ours = proposal(&folders(home, CONFIG), &[]);
        assert!(ours.unrendered.is_empty(), "{:?}", ours.unrendered);
        assert_eq!(
            strings(&ours.settings["sandbox"]["filesystem"]["denyRead"])[0],
            home
        );
        let rules = strings(&ours.settings["permissions"]["deny"]);
        assert_eq!(rules[0], "Read(//home/o w/data (2024)/baley/**)");
        assert_eq!(rules[1], "Edit(//home/o w/data (2024)/baley/**)");
    }

    #[test]
    fn a_pattern_character_or_trailing_space_rendered_as_a_rule_that_matches_something_else_is_caught()
     {
        for (path, character) in [
            ("/srv/[x]/baley.toml", '['),
            ("/srv/a*/baley.toml", '*'),
            ("/srv/a?/baley.toml", '?'),
            (r"/srv/a\b/baley.toml", '\\'),
            ("/srv/a/baley.toml ", ' '),
        ] {
            let ours = proposal(&folders(HOME, CONFIG), &[path]);
            assert_eq!(
                ours.unrendered,
                [Unrendered::UnsupportedCharacter {
                    path: path.to_owned(),
                    character,
                }],
                "{path}"
            );
            assert!(!ours.settings.to_string().contains("/srv/"), "{path}");
            let deny_write = strings(&ours.settings["sandbox"]["filesystem"]["denyWrite"]);
            assert_eq!(deny_write, [HOME, CONFIG, EXECUTABLE], "{path}");
        }
    }

    #[test]
    fn protection_rendered_for_a_relative_or_lossily_converted_path_is_caught() {
        let relative = proposal(&folders("baley", CONFIG), &[]);
        assert_eq!(
            relative.unrendered,
            [Unrendered::MissingPrerequisite {
                path: "baley".into(),
                fault: PathFault::Relative,
            }]
        );
        let filesystem = &relative.settings["sandbox"]["filesystem"];
        assert_eq!(strings(&filesystem["denyRead"]), [CONFIG]);
        assert_eq!(strings(&filesystem["denyWrite"]), [CONFIG, EXECUTABLE]);
        assert_eq!(
            strings(&relative.settings["permissions"]["deny"]),
            [
                "Read(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.local/bin/baley)",
            ]
        );

        let bytes = PathBuf::from(OsStr::from_bytes(b"/p\xff/baley.toml"));
        let ours = propose(
            &folders(HOME, CONFIG),
            &Executable::new(EXECUTABLE).unwrap(),
            std::slice::from_ref(&bytes),
            &[],
        );
        assert_eq!(
            ours.unrendered,
            [Unrendered::MissingPrerequisite {
                path: bytes,
                fault: PathFault::NotUtf8,
            }]
        );
        let deny_write = strings(&ours.settings["sandbox"]["filesystem"]["denyWrite"]);
        assert_eq!(deny_write, [HOME, CONFIG, EXECUTABLE]);
        assert!(!ours.settings.to_string().contains("baley.toml"));
    }

    const VERSIONS_FOLDER: &str = "/home/o/.local/lib/crenshawdev/baley/versions";

    fn with_write_only_folders(listed: &[&str]) -> Proposal {
        let listed: Vec<PathBuf> = listed.iter().map(PathBuf::from).collect();
        propose(
            &folders(HOME, CONFIG),
            &Executable::new(EXECUTABLE).unwrap(),
            &[],
            &listed,
        )
    }

    #[test]
    fn a_versions_folder_rendered_as_a_file_rule_or_given_read_protection_is_caught() {
        let ours = with_write_only_folders(&[VERSIONS_FOLDER]);
        assert!(ours.unrendered.is_empty(), "{:?}", ours.unrendered);
        let filesystem = &ours.settings["sandbox"]["filesystem"];
        assert_eq!(strings(&filesystem["denyRead"]), [HOME, CONFIG]);
        assert_eq!(
            strings(&filesystem["denyWrite"]),
            [HOME, CONFIG, VERSIONS_FOLDER, EXECUTABLE]
        );
        assert_eq!(
            strings(&ours.settings["permissions"]["deny"]),
            [
                "Read(//home/o/.local/share/crenshawdev/baley/**)",
                "Edit(//home/o/.local/share/crenshawdev/baley/**)",
                "Read(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.local/lib/crenshawdev/baley/versions/**)",
                "Edit(//home/o/.local/bin/baley)",
            ]
        );
    }

    #[test]
    fn a_versions_folder_with_a_pattern_character_rendered_is_caught() {
        let ours = with_write_only_folders(&["/home/o/v[1]/versions"]);
        assert_eq!(
            ours.unrendered,
            [Unrendered::UnsupportedCharacter {
                path: "/home/o/v[1]/versions".to_owned(),
                character: '[',
            }]
        );
        assert!(!ours.settings.to_string().contains("/home/o/v[1]"));
    }
}
