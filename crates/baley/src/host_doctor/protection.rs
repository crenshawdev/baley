//! What the supplied settings and hook documents configure: the coverage
//! judge run over each pair, with the placement map's own protected list.
//!
//! The verdicts describe configuration in the documents given. They are not
//! proof that Claude Code enforces them, and other settings files can change
//! what applies.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Content, Document};
use crate::folders::Folders;
use crate::host_artifacts::coverage::{
    self, Coverage, Inputs, Mechanism, Tool, is_guard, matcher_names,
};
use crate::host_artifacts::hook;
use crate::host_artifacts::placement::{Artifact, PlacementMap};

/// Coverage judged over a settings document and a hook document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// The document the sandbox and the deny rules were read from.
    pub settings: PathBuf,
    /// The document the hooks were read from; the settings document's path
    /// when one file holds both.
    pub hook: PathBuf,
    /// The coverage judge's verdicts.
    pub coverage: Coverage,
}

/// Why coverage was not judged. Neither reason is a gap in itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotJudged {
    /// The settings placement, the hook placement or both are unknown, so
    /// there is no document to judge. Nothing is judged against an empty or
    /// invented one.
    Unknown(Vec<Artifact>),
    /// A document could not be used. Its fault is reported with the
    /// artifact.
    Unusable,
}

/// Runs the coverage judge once over the settings and hook documents of the
/// map. The write-only list is the map's protected paths, taken whole, then
/// the discovered checkout's `baley.toml` when there is one. `unsupported`
/// names the mechanisms this machine cannot carry.
pub fn judge(
    map: &PlacementMap,
    documents: &[Document],
    folders: &Folders,
    checkout_file: Option<&Path>,
    unsupported: &[Mechanism],
) -> Result<Judged, NotJudged> {
    let holding = |artifact: &Artifact| {
        documents
            .iter()
            .find(|document| document.artifacts.contains(artifact))
    };
    let (settings, hook) = (holding(&Artifact::Settings), holding(&Artifact::Hook));
    let unknown: Vec<Artifact> = [
        (Artifact::Settings, settings.is_none()),
        (Artifact::Hook, hook.is_none()),
    ]
    .into_iter()
    .filter_map(|(artifact, missing)| missing.then_some(artifact))
    .collect();
    let (Some(settings), Some(hook)) = (settings, hook) else {
        return Err(NotJudged::Unknown(unknown));
    };
    let (Content::Object(settings_value), Content::Object(hook_value)) =
        (&settings.content, &hook.content)
    else {
        return Err(NotJudged::Unusable);
    };

    let mut write_only = map.protected_paths();
    write_only.extend(checkout_file.map(Path::to_path_buf));
    let coverage = coverage::judge(&Inputs {
        settings: settings_value,
        hook: hook_value,
        folders,
        executable: map.executable(),
        write_only: &write_only,
        unsupported,
    });
    Ok(Judged {
        settings: settings.path.clone(),
        hook: hook.path.clone(),
        coverage,
    })
}

/// The nine-tool check over the hook document: which tools of the hook's
/// matcher have no `PreToolUse` item that runs the guard for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NineTools {
    /// The document the hooks were read from.
    pub hook: PathBuf,
    /// The tools no item runs the guard for, in matcher order. Empty when
    /// all nine are guarded.
    pub missing: Vec<Tool>,
    /// The document that sets `disableAllHooks` true, the hook document
    /// first, then the settings document when its placement is known. Claude
    /// Code then runs no hook, so no item guards any tool whatever the
    /// matchers say.
    pub disabled_in: Option<PathBuf>,
}

/// Checks that the guard runs before every tool of `hook::MATCHER`. The
/// coverage judge credits Bash, Monitor and PowerShell to the sandbox and
/// never asks whether the guard also runs for them, so this is its own
/// judgement. A matcher in one item never borrows another item's guard, and
/// a handler counts only when the coverage judge accepts it whole. A
/// `disableAllHooks` of true in the hook document, or in the settings
/// document when one is placed, is reported with the document and does not
/// change which tools the items name. Nothing is judged when the hook
/// placement is unknown or its document unusable.
pub fn nine_tools(map: &PlacementMap, documents: &[Document]) -> Option<NineTools> {
    let document = documents
        .iter()
        .find(|document| document.artifacts.contains(&Artifact::Hook))?;
    let Content::Object(value) = &document.content else {
        return None;
    };
    let command = hook::command(map.executable());
    let items: &[Value] = value
        .pointer(&format!("/hooks/{}", hook::EVENT))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let guarded = |tool: Tool| {
        items.iter().any(|item| {
            matcher_names(item.get("matcher"), tool)
                && item
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.iter().any(|handler| is_guard(handler, &command))
                    })
        })
    };
    let settings = documents
        .iter()
        .find(|document| document.artifacts.contains(&Artifact::Settings));
    let disabled_in = [Some(document), settings]
        .into_iter()
        .flatten()
        .find(|document| {
            matches!(&document.content, Content::Object(value)
                if value.get("disableAllHooks") == Some(&Value::Bool(true)))
        })
        .map(|document| document.path.clone());
    Some(NineTools {
        hook: document.path.clone(),
        disabled_in,
        missing: Tool::ALL
            .into_iter()
            .filter(|tool| !guarded(*tool))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::fixtures::*;
    use super::*;
    use crate::host_artifacts::compose::compose;
    use crate::host_artifacts::coverage::Verdict;
    use crate::host_artifacts::{hook, security};

    fn document(path: &str, artifacts: &[Artifact], content: Value) -> Document {
        Document {
            path: path.into(),
            artifacts: artifacts.to_vec(),
            content: Content::Object(content),
        }
    }

    #[test]
    fn a_coverage_judged_without_the_settings_or_hook_document_is_caught() {
        let exe = executable();
        let proposal = security::propose(&folders(), &exe, &[SETTINGS.into()]).settings;
        let hook_only = map(None, None, None, Some(HOOKS), None);
        let hook_doc = document(HOOKS, &[Artifact::Hook], hook::render(&exe));
        assert_eq!(
            judge(&hook_only, &[hook_doc], &folders(), None, &[]),
            Err(NotJudged::Unknown(vec![Artifact::Settings]))
        );
        let settings_only = map(None, None, None, None, Some(SETTINGS));
        let settings_doc = document(SETTINGS, &[Artifact::Settings], proposal);
        assert_eq!(
            judge(&settings_only, &[settings_doc], &folders(), None, &[]),
            Err(NotJudged::Unknown(vec![Artifact::Hook]))
        );
    }

    #[test]
    fn the_discovered_checkouts_baley_toml_left_out_of_the_write_only_files_is_caught() {
        let exe = executable();
        let toml = "/work/a/baley.toml";
        let map = map(
            None,
            Some(HELP),
            Some(REGISTRATION),
            Some(SETTINGS),
            Some(SETTINGS),
        );
        let write_only: Vec<PathBuf> = [HELP, REGISTRATION, SETTINGS, toml]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let proposal = security::propose(&folders(), &exe, &write_only).settings;
        let composed = compose(None, &[proposal, hook::render(&exe)], &folders(), &exe).document;
        let documents = [document(
            SETTINGS,
            &[Artifact::Hook, Artifact::Settings],
            composed,
        )];

        let judged = judge(&map, &documents, &folders(), Some(Path::new(toml)), &[]).unwrap();
        let files: Vec<&Path> = judged
            .coverage
            .files
            .iter()
            .map(|file| file.path.as_path())
            .collect();
        assert_eq!(
            files,
            [HELP, REGISTRATION, SETTINGS, EXECUTABLE, toml].map(Path::new)
        );
        for file in &judged.coverage.files {
            assert!(
                matches!(file.write, Verdict::Covered { .. }),
                "{:?}",
                file.path
            );
        }
        assert_eq!(judged.settings, PathBuf::from(SETTINGS));
    }
    /// A hook document holding the given `PreToolUse` items, each as a
    /// matcher and the command it runs.
    fn hook_items(items: &[(&str, &str)]) -> Document {
        let handler_for = |command: &str| {
            let mut handler =
                hook::render(&executable())["hooks"][hook::EVENT][0]["hooks"][0].clone();
            handler["command"] = command.into();
            handler
        };
        let items: Vec<Value> = items
            .iter()
            .map(|(matcher, command)| json!({"matcher": matcher, "hooks": [handler_for(command)]}))
            .collect();
        document(
            HOOKS,
            &[Artifact::Hook],
            json!({"hooks": {hook::EVENT: items}}),
        )
    }

    fn missing_tools(document: Document) -> Vec<&'static str> {
        let map = map(None, None, None, Some(HOOKS), None);
        nine_tools(&map, &[document])
            .expect("the hook document is usable")
            .missing
            .into_iter()
            .map(Tool::name)
            .collect()
    }

    #[test]
    fn a_guard_split_across_two_hook_items_reported_as_missing_tools_is_caught() {
        let guard = hook::command(&executable());
        let split = hook_items(&[
            ("Bash|Monitor|PowerShell", &guard),
            ("Read|Grep|Glob|Write|Edit|NotebookEdit", &guard),
        ]);
        assert_eq!(missing_tools(split), Vec::<&str>::new());

        let borrowed = hook_items(&[
            ("Read|Grep|Glob|Write|Edit|NotebookEdit", &guard),
            ("Bash|Monitor|PowerShell", "/bin/echo"),
        ]);
        assert_eq!(missing_tools(borrowed), ["Bash", "Monitor", "PowerShell"]);
    }

    #[test]
    fn a_guard_counted_as_running_while_disable_all_hooks_is_true_is_caught() {
        let disabled_in = |documents: &[Document]| {
            let map = map(None, None, None, Some(HOOKS), Some(SETTINGS));
            nine_tools(&map, documents).unwrap().disabled_in
        };
        let hooks = |disabled: Value| {
            let mut document = hook::render(&executable());
            document["disableAllHooks"] = disabled;
            document
        };
        let settings = |content: Value| document(SETTINGS, &[Artifact::Settings], content);
        let hook_doc = |content: Value| document(HOOKS, &[Artifact::Hook], content);

        assert_eq!(disabled_in(&[hook_doc(hook::render(&executable()))]), None);
        assert_eq!(
            disabled_in(&[hook_doc(hooks(json!(false)))]),
            None,
            "only true disables"
        );
        assert_eq!(
            disabled_in(&[hook_doc(hooks(json!(true)))]),
            Some(PathBuf::from(HOOKS))
        );
        assert_eq!(
            disabled_in(&[
                hook_doc(hook::render(&executable())),
                settings(json!({"disableAllHooks": true})),
            ]),
            Some(PathBuf::from(SETTINGS))
        );
    }

    #[test]
    fn a_tool_named_by_a_pattern_or_a_look_alike_guard_counted_as_guarded_is_caught() {
        let guard = hook::command(&executable());
        let all_nine = [
            "Bash",
            "Monitor",
            "PowerShell",
            "Read",
            "Grep",
            "Glob",
            "Write",
            "Edit",
            "NotebookEdit",
        ];
        let echo_guard = format!("/bin/echo {guard}");
        for item in [
            ("Ba.*", guard.as_str()),
            (hook::MATCHER, "/bin/echo guard"),
            (hook::MATCHER, echo_guard.as_str()),
        ] {
            assert_eq!(missing_tools(hook_items(&[item])), all_nine, "{item:?}");
        }
    }
}
