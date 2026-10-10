//! Claude Code's user settings file for `baley install`: what a read of it
//! found, which older Baley entries leave it on the install record's
//! evidence, and what Baley's entries compose into it. Nothing here reads or
//! writes a file.

use std::path::Path;

use serde_json::{Map, Value};

use crate::folders::Folders;
use crate::host_artifacts::hook;
use crate::host_artifacts::placement::PlacementMap;
use crate::host_artifacts::security::{Proposal, propose};
use crate::host_doctor::placed::{self, FileState};
use crate::store::model::digest;

use super::plan::Observation;

/// The settings file as a read found it, ready to compose into.
#[derive(Debug, Clone, PartialEq)]
pub struct Current {
    /// The JSON object the file holds, or an empty one when it is absent.
    pub document: Value,
    /// Whether the settings path is itself a symbolic link. A link whose
    /// target reads as an object is accepted here and refused only when a
    /// write is needed.
    pub symbolic_link: bool,
    /// SHA-256 of the bytes read, or none when the file is absent.
    pub read_digest: Option<String>,
}

/// The refusal for a settings file that cannot be composed into.
fn conflict(path: &Path, cause: &str) -> String {
    format!(
        "install-settings-conflict: {} {cause}; fix it by hand, then run baley install again",
        path.display()
    )
}

/// Judges what the read found. An absent file is an empty document, and a
/// file that is not a JSON object, or is not a readable file, is refused:
/// it is never read as an empty one, since composing into it would write
/// over the owner's file.
pub fn current(path: &Path, seen: &Observation) -> Result<Current, String> {
    match &seen.state {
        FileState::Absent => Ok(Current {
            document: Value::Object(Map::new()),
            symbolic_link: seen.symbolic_link,
            read_digest: None,
        }),
        FileState::Fault(fault) => Err(conflict(path, &fault.to_string())),
        FileState::Bytes(bytes) => {
            let document =
                placed::document(bytes).map_err(|fault| conflict(path, &fault.to_string()))?;
            Ok(Current {
                document,
                symbolic_link: seen.symbolic_link,
                read_digest: Some(digest(bytes)),
            })
        }
    }
}

/// This binary's entries in the settings document: the hook item and the
/// three lists the sandbox and permission proposal renders, with the paths
/// the proposal could not render.
#[derive(Debug, Clone, PartialEq)]
pub struct Ours {
    /// The `PreToolUse` item that runs the guard at the stable path.
    pub hook_item: Value,
    /// The proposal composition merges in.
    pub proposal: Proposal,
    /// The `permissions.deny` rules.
    pub permissions_deny: Vec<String>,
    /// The `sandbox.filesystem.denyRead` entries.
    pub deny_read: Vec<String>,
    /// The `sandbox.filesystem.denyWrite` entries.
    pub deny_write: Vec<String>,
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Renders this binary's entries from the placement map and Baley's
/// folders: the stable path is the executable, the map's protected paths are
/// write-only files and its write-only folders are the versions folder.
pub fn ours(placements: &PlacementMap, folders: &Folders) -> Ours {
    let executable = placements.executable();
    let proposal = propose(
        folders,
        executable,
        &placements.protected_paths(),
        &placements.write_only_folders(),
    );
    let hook_item = hook::render(executable)["hooks"][hook::EVENT][0].clone();
    let settings = &proposal.settings;
    Ours {
        permissions_deny: strings(settings.pointer("/permissions/deny")),
        deny_read: strings(settings.pointer("/sandbox/filesystem/denyRead")),
        deny_write: strings(settings.pointer("/sandbox/filesystem/denyWrite")),
        hook_item,
        proposal,
    }
}

/// Removes an older Baley entry on the install record's evidence only: the
/// hook item the record holds when it differs from this binary's, and each
/// string the record lists under `permissions_deny`, `deny_read` or
/// `deny_write` that this binary's lists do not hold. Whole values are
/// compared, never a pattern. No key goes and no other entry, so an owner's
/// own items, rules and switches stay. With `keep_sandbox` the sandbox block
/// is left exactly as it was.
pub fn remove_recorded(
    document: &mut Value,
    latest: Option<&Value>,
    ours: &Ours,
    keep_sandbox: bool,
) {
    let Some(latest) = latest else { return };
    if let Some(item) = latest.pointer("/registered/hook/item")
        && *item != ours.hook_item
        && let Some(items) = document
            .pointer_mut(&format!("/hooks/{}", hook::EVENT))
            .and_then(Value::as_array_mut)
    {
        items.retain(|existing| existing != item);
    }
    let mut lists = vec![(
        "/permissions/deny",
        "/sandbox/permissions_deny",
        &ours.permissions_deny,
    )];
    if !keep_sandbox {
        lists.push((
            "/sandbox/filesystem/denyRead",
            "/sandbox/deny_read",
            &ours.deny_read,
        ));
        lists.push((
            "/sandbox/filesystem/denyWrite",
            "/sandbox/deny_write",
            &ours.deny_write,
        ));
    }
    for (pointer, recorded, ours) in lists {
        let recorded = strings(latest.pointer(recorded));
        if let Some(list) = document.pointer_mut(pointer).and_then(Value::as_array_mut) {
            list.retain(|entry| {
                entry.as_str().is_none_or(|entry| {
                    !recorded.iter().any(|old| old == entry)
                        || ours.iter().any(|ours| ours == entry)
                })
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::host_doctor::placed::{Fault, FileState};
    use crate::install::plan::Observation;

    const PATH: &str = "/home/o/.claude/settings.json";

    fn seen(state: FileState) -> Observation {
        Observation {
            state,
            symbolic_link: false,
        }
    }

    #[test]
    fn an_unparseable_settings_file_composed_into_is_caught() {
        let bytes = br#"{"model": "opus","#.to_vec();
        let refusal = current(Path::new(PATH), &seen(FileState::Bytes(bytes)))
            .expect_err("bytes that are not JSON give no document to compose into");
        assert!(
            refusal.starts_with("install-settings-conflict: "),
            "{refusal}"
        );
        assert!(refusal.contains(PATH), "{refusal}");
        assert!(refusal.contains("is not JSON"), "{refusal}");
        assert_eq!(refusal.lines().count(), 1);
    }

    #[test]
    fn a_settings_file_that_is_not_an_object_or_not_a_file_composed_into_is_caught() {
        for (state, cause) in [
            (
                FileState::Bytes(b"[1]".to_vec()),
                "is JSON but not an object",
            ),
            (
                FileState::Bytes(br#""x""#.to_vec()),
                "is JSON but not an object",
            ),
            (FileState::Fault(Fault::NotRegular), "is not a regular file"),
            (
                FileState::Fault(Fault::DanglingLink),
                "is a link to a missing file",
            ),
            (
                FileState::Fault(Fault::Unreadable("Permission denied".into())),
                "Permission denied",
            ),
        ] {
            let refusal = current(Path::new(PATH), &seen(state.clone()))
                .expect_err("a file that is not a settings document is refused");
            assert!(
                refusal.starts_with("install-settings-conflict: "),
                "{refusal}"
            );
            assert!(refusal.contains(PATH), "{refusal}");
            assert!(refusal.contains(cause), "{refusal}");
        }
        let empty = current(Path::new(PATH), &seen(FileState::Absent)).expect("absent is empty");
        assert_eq!(empty.document, serde_json::json!({}));
        assert_eq!(empty.read_digest, None);
    }
}

#[cfg(test)]
mod removal_tests {
    use serde_json::json;

    use super::*;
    use crate::install::fixtures::{folders, installed};

    const OLD_HOOK_COMMAND: &str = "'/home/o/.local/bin/baley' guard";

    fn old_hook() -> Value {
        json!({
            "matcher": "Bash|Write|Edit",
            "hooks": [{"type": "command", "command": OLD_HOOK_COMMAND, "timeout": 5}],
        })
    }

    fn owner_hook() -> Value {
        json!({"matcher": "Bash", "hooks": [{"type": "command", "command": "echo hi"}]})
    }

    #[test]
    fn an_older_baley_hook_or_deny_entry_left_beside_the_new_one_is_caught() {
        let record = json!({
            "registered": {"hook": {"path": "/home/o/.claude/settings.json", "item": old_hook()}},
            "sandbox": {
                "permissions_deny": ["Edit(//home/o/old/**)", "Edit(//home/o/.local/bin/baley)"],
                "deny_read": ["/home/o/old"],
                "deny_write": ["/home/o/old"],
            },
        });
        let mut document = json!({
            "hooks": {"PreToolUse": [old_hook(), owner_hook()]},
            "permissions": {"deny": [
                "Edit(//home/o/old/**)",
                "Read(./secrets/**)",
                "Edit(//home/o/.local/bin/baley)",
            ]},
            "sandbox": {"filesystem": {
                "denyRead": ["/home/o/old"],
                "denyWrite": ["/home/o/old"],
            }},
        });
        let installed = installed();
        let ours = ours(&installed.placements, &folders());

        remove_recorded(&mut document, Some(&record), &ours, false);

        assert_eq!(document["hooks"]["PreToolUse"], json!([owner_hook()]));
        assert_eq!(
            document["permissions"]["deny"],
            json!(["Read(./secrets/**)", "Edit(//home/o/.local/bin/baley)"])
        );
        assert_eq!(document["sandbox"]["filesystem"]["denyRead"], json!([]));
        assert_eq!(document["sandbox"]["filesystem"]["denyWrite"], json!([]));
    }

    #[test]
    fn an_entry_removed_because_it_looks_like_baleys_is_caught() {
        let document = json!({
            "hooks": {"PreToolUse": [{
                "matcher": "Bash",
                "hooks": [{"type": "command", "command": OLD_HOOK_COMMAND}],
            }]},
            "permissions": {"deny": ["Edit(//home/o/.config/crenshawdev/baley-old/**)"]},
        });
        let record = json!({
            "registered": {"hook": {"path": "/home/o/.claude/settings.json", "item": old_hook()}},
            "sandbox": {
                "permissions_deny": ["Edit(//home/o/old/**)"],
                "deny_read": [],
                "deny_write": ["/home/o/old"],
            },
        });
        let installed = installed();
        let ours = ours(&installed.placements, &folders());

        for latest in [None, Some(&record)] {
            let mut seen = document.clone();
            remove_recorded(&mut seen, latest, &ours, false);
            assert_eq!(seen, document);
        }
    }
}
