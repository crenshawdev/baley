//! What the supplied settings and hook documents configure: the coverage
//! judge run over each pair, with the placement map's own protected list.
//!
//! The verdicts describe configuration in the documents given. They are not
//! proof that Claude Code enforces them, and other settings files can change
//! what applies.

use std::path::{Path, PathBuf};

use super::{Content, Document};
use crate::folders::Folders;
use crate::host_artifacts::coverage::{self, Coverage, Inputs, Mechanism};
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

#[cfg(test)]
mod tests {
    use serde_json::Value;

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
}
