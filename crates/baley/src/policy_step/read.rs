//! The policy's reads: gathering the global file and HEAD's copy of the
//! project file, and building the effective policy from them.

use std::path::Path;

use baley_core::policy::{
    EffectivePolicy, FileLayer, Host, Schema, SettingsFile, Unavailable, merge, parse_layer,
};

use crate::committed::{self, Committed};
use crate::process::Process;
use crate::settings;

/// What the gatherer found, for [`build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reads {
    /// The global file's read, as `settings::read` returns it.
    pub global: Result<Option<SettingsFile>, Unavailable>,
    /// HEAD's copy of the working-tree project file, as `committed::read`
    /// returns it. `None` when there is no working-tree file.
    pub head: Option<Result<Committed, Unavailable>>,
}

/// Reads the global file in the `config` folder and, when a working-tree
/// project file is given, HEAD's copy of it from the repository at `root`.
/// The caller supplies the process git runs through: the command line passes
/// the system's, and the session server passes its own.
pub fn gather(
    config: &Path,
    root: &Path,
    working: Option<&SettingsFile>,
    process: &mut dyn Process,
) -> Reads {
    let global = settings::read(&config.join(settings::GLOBAL_FILE));
    let head = working.map(|working| committed::read(root, working, process));
    Reads { global, head }
}

/// The effective policy from `reads`, or the first refusal: the global
/// read's, then HEAD's, then the global file's parse, then the project
/// file's. The project layer is HEAD's copy alone, so the pending note and
/// any uncommitted edit never reach the policy. A fault in HEAD's copy is
/// labelled as HEAD's, since it carries the working-tree file's path.
///
/// The host is the caller's: none for the command line, which then applies
/// no `[host.*]` section and names no host, and the session's host for the
/// server.
pub fn build(reads: &Reads, host: Option<Host>) -> Result<EffectivePolicy, Unavailable> {
    let global = reads.global.as_ref().map_err(Clone::clone)?;
    let project = match &reads.head {
        None => None,
        Some(head) => head.as_ref().map_err(Clone::clone)?.layer.as_ref(),
    };
    let schema = Schema::standard();
    let global = global
        .as_ref()
        .map(|file| parse_layer(file, FileLayer::Global, schema))
        .transpose()?;
    let project = project
        .map(|file| parse_layer(file, FileLayer::Project, schema).map_err(Unavailable::at_head))
        .transpose()?;
    Ok(merge(schema, host, global.as_ref(), project.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::committed::Pending;
    use baley_core::policy::{Fault, Layer, Value};

    const GLOBAL: &str = "/c/config.toml";
    const PROJECT: &str = "/r/baley.toml";

    fn file(path: &str, text: &str) -> SettingsFile {
        settings::file(Path::new(path), text.as_bytes().to_vec())
    }

    fn head(text: &str, pending: Option<Pending>) -> Option<Result<Committed, Unavailable>> {
        Some(Ok(Committed {
            layer: Some(file(PROJECT, text)),
            pending,
        }))
    }

    fn unreadable(path: &str, cause: &str) -> Unavailable {
        Unavailable {
            path: path.into(),
            fault: Fault::Unreadable {
                cause: cause.into(),
            },
        }
    }

    #[test]
    fn a_global_file_that_does_not_parse_or_cannot_be_read_is_refused_not_skipped() {
        let reads = Reads {
            global: Ok(Some(file(GLOBAL, "escalate_on_failure = [\n"))),
            head: None,
        };
        let refusal = build(&reads, None).unwrap_err();
        assert_eq!(refusal.path, Path::new(GLOBAL));
        assert!(
            refusal
                .to_string()
                .starts_with("config-unavailable: /c/config.toml:1:"),
            "{refusal}"
        );

        let failed = unreadable(GLOBAL, "Permission denied (os error 13)");
        let reads = Reads {
            global: Err(failed.clone()),
            head: None,
        };
        let refusal = build(&reads, None).unwrap_err();
        assert_eq!(refusal, failed);
        assert_eq!(
            refusal.to_string(),
            "config-unavailable: cannot read /c/config.toml: Permission denied (os error 13)"
        );
    }

    #[test]
    fn a_wrong_type_in_heads_copy_is_refused_as_heads_not_the_working_trees() {
        let reads = Reads {
            global: Ok(None),
            head: head("escalate_on_failure = \"yes\"\n", None),
        };
        let refusal = build(&reads, None).unwrap_err();
        assert_eq!(refusal.path, Path::new(PROJECT));
        assert!(
            matches!(
                &refusal.fault,
                Fault::AtHead { fault } if matches!(**fault, Fault::WrongType { line: 1, column, .. } if column > 0)
            ),
            "{:?}",
            refusal.fault
        );
        let text = refusal.to_string();
        assert!(
            text.starts_with("config-unavailable: HEAD's copy of /r/baley.toml:1:"),
            "{text}"
        );
        assert!(text.contains("escalate_on_failure"), "{text}");
    }

    #[test]
    fn a_failed_read_of_heads_copy_is_refused_with_its_cause_not_read_as_no_layer() {
        let failed = unreadable(PROJECT, "HEAD's copy: git exited with status 128");
        let reads = Reads {
            global: Ok(None),
            head: Some(Err(failed.clone())),
        };
        let refusal = build(&reads, None).unwrap_err();
        assert_eq!(refusal, failed);
        assert!(
            refusal
                .to_string()
                .ends_with("HEAD's copy: git exited with status 128"),
            "{refusal}"
        );
    }

    #[test]
    fn heads_refusal_is_not_named_before_the_global_files() {
        let global = unreadable(GLOBAL, "Permission denied (os error 13)");
        let reads = Reads {
            global: Err(global.clone()),
            head: Some(Err(unreadable(PROJECT, "HEAD's copy: git failed"))),
        };
        assert_eq!(build(&reads, None).unwrap_err(), global);

        let reads = Reads {
            global: Ok(Some(file(GLOBAL, "escalate_on_failure = [\n"))),
            head: head("escalate_on_failure = \"yes\"\n", None),
        };
        assert_eq!(build(&reads, None).unwrap_err().path, Path::new(GLOBAL));
    }

    #[test]
    fn the_policy_takes_heads_value_and_the_pending_note_changes_nothing() {
        // HEAD sets the value away from its default of false. The working
        // tree, which says false, is only ever the note.
        let committed = "escalate_on_failure = true\n";
        let differs = Pending::Differs {
            path: PROJECT.into(),
        };
        let noted = build(
            &Reads {
                global: Ok(None),
                head: head(committed, Some(differs)),
            },
            None,
        )
        .unwrap();
        let clean = build(
            &Reads {
                global: Ok(None),
                head: head(committed, None),
            },
            None,
        )
        .unwrap();

        let setting = &noted.settings["escalate_on_failure"];
        assert_eq!(setting.value, Some(Value::Bool(true)));
        assert_eq!(setting.source.layer, Layer::Project);
        assert_eq!(
            setting.source.file.as_ref().unwrap().digest,
            file(PROJECT, committed).digest
        );
        assert_eq!(noted, clean);
    }

    #[test]
    fn no_working_tree_file_gives_a_policy_with_no_project_layer() {
        let policy = build(
            &Reads {
                global: Ok(Some(file(GLOBAL, "escalate_on_failure = true\n"))),
                head: None,
            },
            None,
        )
        .unwrap();
        assert_eq!(policy.project, None);
        assert_eq!(policy.host, None);
        let setting = &policy.settings["escalate_on_failure"];
        assert_eq!(setting.value, Some(Value::Bool(true)));
        assert_eq!(setting.source.layer, Layer::Global);
    }

    // The file sets the value at the top level and again in its host section,
    // so each build below shows which of the two it applied.
    const SECTIONED: &str =
        "escalate_on_failure = false\n[host.claude-code]\nescalate_on_failure = true\n";

    fn sectioned() -> Reads {
        Reads {
            global: Ok(None),
            head: head(SECTIONED, None),
        }
    }

    #[test]
    fn a_claude_code_build_applies_the_host_section_and_names_the_host() {
        let policy = build(&sectioned(), Some(Host::ClaudeCode)).unwrap();
        assert_eq!(policy.host, Some(Host::ClaudeCode));
        let setting = &policy.settings["escalate_on_failure"];
        assert_eq!(setting.value, Some(Value::Bool(true)));
        assert_eq!(setting.source.layer, Layer::ProjectHost);
    }

    #[test]
    fn a_build_with_no_host_keeps_the_top_level_value_and_names_no_host() {
        let policy = build(&sectioned(), None).unwrap();
        assert_eq!(policy.host, None);
        let setting = &policy.settings["escalate_on_failure"];
        assert_eq!(setting.value, Some(Value::Bool(false)));
        assert_eq!(setting.source.layer, Layer::Project);
    }
}
