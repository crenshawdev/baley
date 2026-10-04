//! `baley config show`: gathers both settings files and HEAD's copy, then
//! prints every setting with its layer and diagnostics (design 0003 section
//! 5). It opens no store and runs no policy step.

use std::io;
use std::path::Path;

use baley_core::policy::config_command::{ShowRequest, judge_show, render_show};
use baley_core::policy::{Host, Schema, SettingsFile, Unavailable};
use baley_store::StoreError;

use crate::discovery::{self, Discovery, PROJECT_FILE};
use crate::folders::{Environment, Folders, Platform};
use crate::ledger::display::{self, Render};
use crate::policy_step::{self, Reads};
use crate::settings;

/// Reports the settings asked for, every setting when `names` is empty.
pub(crate) fn show(host: Option<Host>, names: &[String]) -> Render {
    attempt(host, names).unwrap_or_else(|render| render)
}

fn attempt(host: Option<Host>, names: &[String]) -> Result<Render, Render> {
    let folders = Folders::resolve(Platform::current(), &Environment::read())
        .map_err(|refusal| Render::refusal(refusal.to_string()))?;
    let unavailable =
        |error: io::Error| display::store_error(&StoreError::Unavailable(error.to_string()), None);
    let cwd = std::env::current_dir().map_err(unavailable)?;
    let ancestors = discovery::ancestors(&cwd).map_err(unavailable)?;
    let global_path = settings::global_path(&folders);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();

    let schema = Schema::standard();
    Ok(match discovery::discover(&ancestors) {
        Discovery::Managed { folder, root } => {
            let path = folder.join(PROJECT_FILE);
            let working = settings::read(&path);
            let head = working.as_ref().ok().and_then(Option::as_ref);
            let reads =
                policy_step::gather(&folders.config, &root, head, &mut crate::process::System);
            report(
                schema,
                host,
                &names,
                &global_path,
                Some(&path),
                working,
                reads,
            )
        }
        Discovery::Unmanaged { .. } | Discovery::Outside => {
            let global = settings::read(&global_path);
            let reads = Reads { global, head: None };
            report(schema, host, &names, &global_path, None, Ok(None), reads)
        }
    })
}

/// Judges the supplied reads and renders the report. A refusal is exit 2 and
/// the report exit 0. HEAD's read becomes the judge's input as follows: no
/// read is no file, a copy is its layer, and a failed read stays its refusal.
/// The pending note is the copy's own text.
fn report(
    schema: &Schema,
    host: Option<Host>,
    names: &[&str],
    global_path: &Path,
    project_path: Option<&Path>,
    working: Result<Option<SettingsFile>, Unavailable>,
    reads: Reads,
) -> Render {
    let mut pending = None;
    let head = match reads.head {
        None => Ok(None),
        Some(Ok(committed)) => {
            pending = committed.pending.map(|note| note.to_string());
            Ok(committed.layer)
        }
        Some(Err(refusal)) => Err(refusal),
    };
    let in_project = project_path.is_some();
    match judge_show(schema, names, in_project, reads.global, working, head) {
        Err(refusal) => Render::refusal(refusal.to_string()),
        Ok(layers) => Render {
            lines: render_show(&ShowRequest {
                schema,
                host,
                names,
                layers: &layers,
                pending: pending.as_deref(),
                global_path,
                project_path,
            }),
            code: 0,
            error: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::committed::{Committed, Pending};
    use baley_core::policy::Fault;

    const GLOBAL: &str = "/c/config.toml";
    const PROJECT: &str = "/r/baley.toml";

    fn file(path: &str, text: &str) -> SettingsFile {
        settings::file(Path::new(path), text.as_bytes().to_vec())
    }

    fn render(working: &str, head: Result<Committed, Unavailable>) -> Render {
        let reads = Reads {
            global: Ok(None),
            head: Some(head),
        };
        report(
            Schema::standard(),
            None,
            &["escalate_on_failure"],
            Path::new(GLOBAL),
            Some(Path::new(PROJECT)),
            Ok(Some(file(PROJECT, working))),
            reads,
        )
    }

    #[test]
    fn a_head_read_that_failed_is_refused_with_its_own_text() {
        let failed = Unavailable {
            path: PROJECT.into(),
            fault: Fault::Unreadable {
                cause: "HEAD's copy: git is not installed".into(),
            },
        };

        let render = render("escalate_on_failure = true\n", Err(failed.clone()));

        assert_eq!((render.code, render.error), (2, true));
        assert_eq!(render.lines, [failed.to_string()]);
    }

    #[test]
    fn a_differing_head_value_is_effective_and_the_pending_note_is_printed() {
        let head = Committed {
            layer: Some(file(PROJECT, "escalate_on_failure = false\n")),
            pending: Some(Pending::Differs {
                path: PROJECT.into(),
            }),
        };

        let render = render("escalate_on_failure = true\n", Ok(head));

        assert_eq!((render.code, render.error), (0, false));
        let text = render.lines.join("\n");
        let note = Pending::Differs {
            path: PROJECT.into(),
        }
        .to_string();
        assert!(text.contains(&note), "{text}");
        assert!(text.contains("effective: false from project"), "{text}");
        assert!(text.contains("project: true (HEAD has false"), "{text}");
    }
}
