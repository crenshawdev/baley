//! Claude Code's user settings file for `baley install`: what a read of it
//! found, which older Baley entries leave it on the install record's
//! evidence, and what Baley's entries compose into it. Nothing here reads or
//! writes a file.

use std::path::Path;

use serde_json::{Map, Value};

use crate::folders::Folders;
use crate::host_artifacts::compose::{Conflict, compose};
use crate::host_artifacts::hook;
use crate::host_artifacts::placement::PlacementMap;
use crate::host_artifacts::security::{Proposal, Unrendered, propose};
use crate::host_doctor::placed::{self, FileState};
use crate::replace;
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

/// An owner value that Baley's own secure value replaced.
#[derive(Debug, Clone, PartialEq)]
pub struct Replaced {
    /// The setting's dotted key.
    pub key: String,
    /// The value the owner's file held.
    pub was: Value,
}

/// What the settings decision reads.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The settings file's path, for the words of a refusal or a gap.
    pub path: &'a Path,
    /// The file as read.
    pub current: &'a Current,
    /// The latest install record, when there is one.
    pub latest: Option<&'a Value>,
    /// Where every artifact goes, and the stable executable.
    pub placements: &'a PlacementMap,
    /// Baley's data and configuration folders.
    pub folders: &'a Folders,
}

/// What install does with the settings file.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// The bytes to write, or none when the composed document equals the one
    /// read, whatever its formatting.
    pub bytes: Option<Vec<u8>>,
    /// SHA-256 of the bytes read, or none when the file was absent.
    pub read_digest: Option<String>,
    /// The document as read.
    pub read: Value,
    /// The document after removal and composition.
    pub document: Value,
    /// Owner values Baley's secure values replaced, to report.
    pub replaced: Vec<Replaced>,
    /// Each setting that leaves protection off, with its fix.
    pub gaps: Vec<String>,
    /// This binary's entries, for the install record.
    pub ours: Ours,
}

/// The refusal text for a replacement that was refused, under install's
/// own code instead of the replacement's.
pub fn replace_refusal(conflict: &replace::Conflict) -> String {
    let text = conflict.to_string();
    let text = text
        .strip_prefix(&format!("{}: ", conflict.code()))
        .unwrap_or(&text);
    format!("install-settings-conflict: {text}")
}

fn secure(setting: &str) -> bool {
    setting != "sandbox.allowUnsandboxedCommands"
}

fn gap(file: &Path, conflict: &Conflict) -> Option<String> {
    let file = file.display();
    Some(match conflict {
        Conflict::Disabled("disableAllHooks") => format!(
            "disableAllHooks is true in {file}, so no hook runs the guard; remove it or set it to false"
        ),
        Conflict::Disabled("sandbox.filesystem.disabled") => format!(
            "sandbox.filesystem.disabled is true in {file}, so the sandbox's filesystem rules do not apply; remove it or set it to false"
        ),
        Conflict::Disabled(setting) => format!(
            "{setting} in {file} is not its secure value; set it to {}",
            secure(setting)
        ),
        Conflict::Reopened { list, entry } => format!(
            "`{entry}` is listed in sandbox.filesystem.{list} in {file}, which re-opens a Baley folder; remove it"
        ),
        Conflict::Unjudged { list, entry } => format!(
            "`{entry}` is listed in sandbox.filesystem.{list} in {file} and is not an absolute path, so whether it re-opens a Baley folder cannot be judged; write it as an absolute path outside Baley's folders or remove it"
        ),
        Conflict::Excluded(entry) => format!(
            "`{entry}` is listed in sandbox.excludedCommands in {file}, so it runs outside the sandbox; remove it"
        ),
        _ => return None,
    })
}

fn unrendered_gap(file: &Path, unrendered: &Unrendered) -> String {
    let fix = match unrendered {
        Unrendered::MissingPrerequisite { .. } => "use an absolute UTF-8 path",
        Unrendered::UnsupportedCharacter { .. } => "use a path without that character",
    };
    format!(
        "{unrendered}, so the deny rules in {} leave it unprotected; {fix}",
        file.display()
    )
}

fn refusal(file: &Path, conflict: &Conflict) -> Option<String> {
    let file = file.display();
    Some(match conflict {
        Conflict::NotComposable(key) => format!(
            "install-settings-conflict: {file} holds `{key}` as a kind of value Baley cannot add its entries to; fix it by hand, then run baley install again"
        ),
        Conflict::OtherGuardHook(item) => format!(
            "install-ownership-conflict: the guard hook at {file} holds a PreToolUse item that runs another guard command than the one Baley wrote: {item}; remove it, then run baley install again"
        ),
        Conflict::OtherRegistration(entry) => format!(
            "install-ownership-conflict: mcpServers.baley at {file} holds an entry that is not the one Baley wrote: {entry}; remove it, then run baley install again"
        ),
        _ => return None,
    })
}

/// Decides what happens to the settings file: removes older Baley entries
/// the record shows, composes this binary's hook and proposal in once, and
/// sorts what composition found. A document that cannot be merged, or holds
/// a guard hook or registration the record does not show Baley wrote, is
/// refused with every line. A setting that leaves protection off is kept and
/// becomes a gap, and Baley's own secure value replacing an owner's is
/// written and reported. A settings path that is a link is refused only when
/// a write is needed.
pub fn judge(input: &Input<'_>) -> Result<Decision, Vec<String>> {
    let ours = ours(input.placements, input.folders);
    let mut document = input.current.document.clone();
    remove_recorded(&mut document, input.latest, &ours, false);
    let executable = input.placements.executable();
    let hook = hook::render(executable);
    let composed = compose(
        Some(&document),
        &[ours.proposal.settings.clone(), hook],
        input.folders,
        executable,
    );
    let (mut refusals, mut replaced, mut gaps) = (Vec::new(), Vec::new(), Vec::new());
    for conflict in &composed.conflicts {
        if let Some(line) = refusal(input.path, conflict) {
            refusals.push(line);
        } else if let Some(line) = gap(input.path, conflict) {
            gaps.push(line);
        } else if let Conflict::Replaced { key, was } = conflict {
            replaced.push(Replaced {
                key: key.clone(),
                was: was.clone(),
            });
        }
    }
    gaps.extend(
        ours.proposal
            .unrendered
            .iter()
            .map(|unrendered| unrendered_gap(input.path, unrendered)),
    );
    if !refusals.is_empty() {
        return Err(refusals);
    }
    let bytes = (composed.document != input.current.document).then(|| {
        let mut bytes =
            serde_json::to_vec_pretty(&composed.document).expect("a JSON document serializes");
        bytes.push(b'\n');
        bytes
    });
    if bytes.is_some() && input.current.symbolic_link {
        let link = replace::Conflict::Link {
            path: input.path.to_path_buf(),
        };
        return Err(vec![replace_refusal(&link)]);
    }
    Ok(Decision {
        bytes,
        read_digest: input.current.read_digest.clone(),
        read: input.current.document.clone(),
        document: composed.document,
        replaced,
        gaps,
        ours,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::host_doctor::placed::Fault;
    use crate::install::fixtures::{complete_settings, folders, four_spaces, hook_item, installed};

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

    fn read(document: Value) -> Current {
        Current {
            document,
            symbolic_link: false,
            read_digest: Some("a".repeat(64)),
        }
    }

    fn decide_in(
        current: &Current,
        latest: Option<&Value>,
        folders: &Folders,
    ) -> Result<Decision, Vec<String>> {
        let installed = installed();
        judge(&Input {
            path: Path::new(PATH),
            current,
            latest,
            placements: &installed.placements,
            folders,
        })
    }

    fn decide(current: &Current, latest: Option<&Value>) -> Result<Decision, Vec<String>> {
        decide_in(current, latest, &folders())
    }

    fn gap_with<'d>(decision: &'d Decision, parts: &[&str]) -> &'d str {
        decision
            .gaps
            .iter()
            .find(|gap| parts.iter().all(|part| gap.contains(part)))
            .unwrap_or_else(|| panic!("no gap holds {parts:?} in {:?}", decision.gaps))
    }

    #[test]
    fn an_owner_setting_lost_or_a_baley_entry_missing_from_the_composed_settings_is_caught() {
        let owner = json!({
            "model": "opus",
            "env": {"FOO": "1"},
            "permissions": {"allow": ["Bash(ls:*)"]},
        });

        let decision = decide(&read(owner), None).expect("an unrelated document composes");

        let document = &decision.document;
        let keys: Vec<&str> = document
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys[..3], ["model", "env", "permissions"]);
        assert_eq!(document["model"], "opus");
        assert_eq!(document["env"], json!({"FOO": "1"}));
        assert_eq!(document["permissions"]["allow"], json!(["Bash(ls:*)"]));
        assert_eq!(
            document["hooks"]["PreToolUse"],
            json!([{
                "matcher": "Bash|Monitor|PowerShell|Read|Grep|Glob|Write|Edit|NotebookEdit",
                "hooks": [{
                    "type": "command",
                    "command": "'/home/o/.local/bin/baley' guard",
                    "timeout": 10,
                }],
            }])
        );
        let sandbox = &document["sandbox"];
        assert_eq!(sandbox["enabled"], true);
        assert_eq!(sandbox["failIfUnavailable"], true);
        assert_eq!(sandbox["allowUnsandboxedCommands"], false);
        assert_eq!(
            sandbox["filesystem"]["denyRead"],
            json!([
                "/home/o/.local/share/crenshawdev/baley",
                "/home/o/.config/crenshawdev/baley"
            ])
        );
        assert_eq!(
            sandbox["filesystem"]["denyWrite"],
            json!([
                "/home/o/.local/share/crenshawdev/baley",
                "/home/o/.config/crenshawdev/baley",
                "/home/o/.local/lib/crenshawdev/baley/versions",
                "/home/o/.claude/skills/bal-capture/SKILL.md",
                "/home/o/.claude/skills/bal-help/SKILL.md",
                "/home/o/.claude.json",
                "/home/o/.claude/settings.json",
                "/home/o/.local/bin/baley",
            ])
        );
        assert_eq!(
            document["permissions"]["deny"],
            json!([
                "Read(//home/o/.local/share/crenshawdev/baley/**)",
                "Edit(//home/o/.local/share/crenshawdev/baley/**)",
                "Read(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.config/crenshawdev/baley/**)",
                "Edit(//home/o/.local/lib/crenshawdev/baley/versions/**)",
                "Edit(//home/o/.claude/skills/bal-capture/SKILL.md)",
                "Edit(//home/o/.claude/skills/bal-help/SKILL.md)",
                "Edit(//home/o/.claude.json)",
                "Edit(//home/o/.claude/settings.json)",
                "Edit(//home/o/.local/bin/baley)",
            ])
        );
        assert!(decision.gaps.is_empty(), "{:?}", decision.gaps);
        assert!(decision.replaced.is_empty());
        let bytes = decision.bytes.as_ref().expect("a write is needed");
        assert!(bytes.ends_with(b"\n"));
        assert_eq!(serde_json::from_slice::<Value>(bytes).unwrap(), *document);
    }

    #[test]
    fn an_owner_setting_that_weakens_protection_written_as_complete_is_caught() {
        let owner = json!({
            "disableAllHooks": true,
            "sandbox": {
                "excludedCommands": ["docker"],
                "filesystem": {"allowWrite": [
                    "/home/o/.config/crenshawdev/baley/x",
                    "~/scratch",
                ]},
            },
        });

        let decision = decide(&read(owner), None).expect("written around, never refused");

        let document = &decision.document;
        assert_eq!(document["disableAllHooks"], true);
        assert_eq!(document["sandbox"]["excludedCommands"], json!(["docker"]));
        assert_eq!(
            document["sandbox"]["filesystem"]["allowWrite"],
            json!(["/home/o/.config/crenshawdev/baley/x", "~/scratch"])
        );
        assert_eq!(decision.gaps.len(), 4, "{:?}", decision.gaps);
        let hooks = gap_with(&decision, &["disableAllHooks", PATH]);
        assert!(hooks.contains("remove it or set it to false"), "{hooks}");
        let docker = gap_with(&decision, &["docker", "sandbox.excludedCommands", PATH]);
        assert!(docker.contains("remove it"), "{docker}");
        let reopened = gap_with(
            &decision,
            &[
                "/home/o/.config/crenshawdev/baley/x",
                "sandbox.filesystem.allowWrite",
                PATH,
            ],
        );
        assert!(reopened.contains("remove it"), "{reopened}");
        let relative = gap_with(
            &decision,
            &["~/scratch", "sandbox.filesystem.allowWrite", PATH],
        );
        assert!(
            relative.contains("write it as an absolute path outside Baley's folders or remove it"),
            "{relative}"
        );
    }

    #[test]
    fn a_replaced_sandbox_switch_reported_as_a_gap_or_left_unreported_is_caught() {
        let owner = json!({"sandbox": {"enabled": false, "failIfUnavailable": false}});

        let decision = decide(&read(owner), None).expect("Baley's secure values are written");

        assert_eq!(decision.document["sandbox"]["enabled"], true);
        assert_eq!(decision.document["sandbox"]["failIfUnavailable"], true);
        let replaced: Vec<(&str, &Value)> = decision
            .replaced
            .iter()
            .map(|replaced| (replaced.key.as_str(), &replaced.was))
            .collect();
        assert_eq!(
            replaced,
            [
                ("sandbox.enabled", &json!(false)),
                ("sandbox.failIfUnavailable", &json!(false)),
            ]
        );
        assert!(decision.gaps.is_empty(), "{:?}", decision.gaps);
    }

    #[test]
    fn a_settings_document_that_cannot_be_merged_written_anyway_is_caught() {
        for (owner, code, names) in [
            (json!({"hooks": []}), "install-settings-conflict", "hooks"),
            (
                json!({"permissions": "all"}),
                "install-settings-conflict",
                "permissions",
            ),
            (
                json!({"mcpServers": {"baley": {"command": "/opt/x/baley", "args": ["serve"]}}}),
                "install-ownership-conflict",
                "mcpServers.baley",
            ),
            (
                json!({"permissions": {"deny": "all"}}),
                "install-settings-conflict",
                "permissions.deny",
            ),
        ] {
            let refusal = decide(&read(owner.clone()), None)
                .expect_err("a document Baley's entries cannot be added to is refused");
            assert_eq!(refusal.len(), 1, "{refusal:?}");
            assert!(refusal[0].starts_with(&format!("{code}: ")), "{refusal:?}");
            assert!(refusal[0].contains(PATH), "{refusal:?}");
            assert!(refusal[0].contains(names), "{refusal:?}");
        }
    }

    #[test]
    fn an_unchanged_settings_file_rewritten_is_caught() {
        let complete = complete_settings();
        let seen = |document: &Value| Observation {
            state: FileState::Bytes(four_spaces(document)),
            symbolic_link: false,
        };
        let decide_seen = |document: &Value| {
            let current = current(Path::new(PATH), &seen(document)).unwrap();
            decide(&current, None).expect("composes")
        };

        let unchanged = decide_seen(&complete);
        assert_eq!(unchanged.bytes, None);

        let mut lost_rule = complete.clone();
        lost_rule["permissions"]["deny"]
            .as_array_mut()
            .unwrap()
            .retain(|rule| rule != "Edit(//home/o/.claude.json)");
        let restored = decide_seen(&lost_rule);
        assert!(restored.bytes.is_some());
        assert!(
            restored.document["permissions"]["deny"]
                .as_array()
                .unwrap()
                .contains(&json!("Edit(//home/o/.claude.json)"))
        );

        let mut lost_hook = complete;
        lost_hook["hooks"]["PreToolUse"] = json!([]);
        let restored = decide_seen(&lost_hook);
        assert!(restored.bytes.is_some());
        assert_eq!(
            restored.document["hooks"]["PreToolUse"],
            json!([hook_item()])
        );
    }

    #[test]
    fn a_settings_link_written_through_is_caught() {
        let linked = |document: Value| Current {
            symbolic_link: true,
            ..read(document)
        };

        let refusal = decide(&linked(json!({"model": "opus"})), None)
            .expect_err("a write through a link is refused");
        assert_eq!(refusal.len(), 1, "{refusal:?}");
        assert!(
            refusal[0].starts_with("install-settings-conflict: "),
            "{refusal:?}"
        );
        assert!(refusal[0].contains(PATH), "{refusal:?}");
        assert!(refusal[0].contains("symbolic link"), "{refusal:?}");

        let unchanged = decide(&linked(complete_settings()), None)
            .expect("a link whose target holds everything needs no write");
        assert_eq!(unchanged.bytes, None);
    }

    #[test]
    fn a_folder_baley_cannot_write_as_a_rule_left_out_of_the_gaps_is_caught() {
        let owner = read(json!({}));
        let broken_home = Folders {
            home: PathBuf::from("/home/o/b[1]/data"),
            ..folders()
        };

        let decision = decide_in(&owner, None, &broken_home).expect("composes");

        assert_eq!(decision.gaps.len(), 1, "{:?}", decision.gaps);
        gap_with(&decision, &["/home/o/b[1]/data", "["]);
        let written = decision.document.to_string();
        assert!(!written.contains("b[1]"), "{written}");
        assert!(written.contains("/home/o/.config/crenshawdev/baley"));

        let broken_both = Folders {
            config: PathBuf::from("/home/o/c?/config"),
            ..broken_home
        };
        let decision = decide_in(&owner, None, &broken_both).expect("composes");
        assert_eq!(decision.gaps.len(), 2, "{:?}", decision.gaps);
        gap_with(&decision, &["/home/o/b[1]/data", "["]);
        gap_with(&decision, &["/home/o/c?/config", "?"]);
        let written = decision.document.to_string();
        assert!(
            !written.contains("b[1]") && !written.contains("c?"),
            "{written}"
        );
    }
}
