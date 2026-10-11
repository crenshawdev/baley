//! The summary the owner reads before the final question (design 0003
//! section 5, Confirmed writes). It is pure over the planned steps and the
//! paths they write to, and it names every file those steps can change or
//! create at its resolved path.

use std::path::{Path, PathBuf};

use baley_core::policy::config_command::TypedPair;
use baley_core::policy::{FileLayer, Host, Value};

use super::writes::{Ack, Plan};
use crate::providers::warning;

/// Where the planned writes land.
pub(super) struct Where<'a> {
    /// Baley's home, which holds the ledger.
    pub(super) home: &'a Path,
    /// The global settings file.
    pub(super) global: &'a Path,
    /// The file the settings answers are written to.
    pub(super) target: &'a Path,
    /// Which file that is.
    pub(super) layer: FileLayer,
    /// The host section the settings answers go to, `None` for the top level.
    pub(super) host: Option<Host>,
    /// Whether the working directory is in a managed project.
    pub(super) in_project: bool,
}

/// The ledger's files in Baley's home, as the store names them.
const LEDGER_FILES: [&str; 5] = [
    "baley.db",
    "baley.db-wal",
    "baley.db-shm",
    "baley.db.writer",
    "baley.db.maintenance",
];

/// A temporary file as `replace` names it, with the process id and the counter
/// left as placeholders since neither is known before the write.
fn temporary(file: &Path) -> PathBuf {
    let name = file.file_name().map(|name| name.to_string_lossy());
    let name = format!(".{}.<process id>-<n>.tmp", name.as_deref().unwrap_or(""));
    file.parent()
        .map_or_else(|| PathBuf::from(&name), |folder| folder.join(&name))
}

/// Adds `path` unless it is listed already, so a file two steps share is named
/// once.
fn add(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

/// The files the planned steps can change or create, and the folders that may
/// be created for them. A step that is not planned adds nothing.
fn files(plan: &Plan, at: &Where<'_>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut files, mut folders) = (Vec::new(), Vec::new());
    let settings = !plan.settings.is_empty();
    // The ledger opens for the acknowledgement, for a catalog seed when a
    // model is set, and for the policy step of a write in a project.
    let ledger = plan.acknowledgement.is_some()
        || plan
            .settings
            .iter()
            .any(|(name, _)| name.ends_with(".model"))
        || (at.in_project && (plan.reviewers.is_some() || settings));
    if ledger {
        for name in LEDGER_FILES {
            add(&mut files, at.home.join(name));
        }
        add(&mut folders, at.home.to_path_buf());
    }
    if let Some(hosts) = &plan.hosts {
        add(&mut files, hosts.path.clone());
        add(&mut files, temporary(&hosts.path));
        if let Some(folder) = hosts.path.parent() {
            add(&mut folders, folder.to_path_buf());
        }
    }
    if plan.reviewers.is_some() || (settings && at.layer == FileLayer::Global) {
        add(&mut files, at.global.to_path_buf());
        add(&mut files, temporary(at.global));
        if let Some(folder) = at.global.parent() {
            add(&mut folders, folder.to_path_buf());
        }
    }
    if settings && at.layer == FileLayer::Project {
        add(&mut files, at.target.to_path_buf());
        add(&mut files, temporary(at.target));
    }
    (files, folders)
}

/// `["host", "openai"]`, as the list is written to the file.
fn list_text(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
    format!("[{}]", quoted.join(", "))
}

/// The summary's lines in order: the providers and their warning version, the
/// API hosts, the reviewer list, every file and folder the writes can touch,
/// and last the settings block when there are settings answers.
pub(super) fn lines(plan: &Plan, at: &Where<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(Ack::Requested(who) | Ack::Record(who, _) | Ack::Reused(who)) =
        &plan.acknowledgement
    {
        let names: Vec<&str> = who
            .providers
            .iter()
            .map(|provider| provider.name())
            .collect();
        lines.push(format!(
            "Outside review providers confirmed: {} (warning version {})",
            names.join(", "),
            warning::WARNING_VERSION
        ));
    }
    if let Some(hosts) = &plan.hosts {
        lines.push(format!(
            "API hosts to allow in {} (sandbox.network.allowedDomains): {}",
            hosts.path.display(),
            hosts.allowance.added.join(", ")
        ));
    }
    if let Some(TypedPair {
        value: Value::ReviewerList(names),
        ..
    }) = &plan.reviewers
    {
        lines.push(format!(
            "review.reviewers will be {} in {}",
            list_text(names),
            at.global.display()
        ));
    }
    let (files, folders) = files(plan, at);
    if !files.is_empty() {
        lines.push("Files these writes can change or create:".to_owned());
        lines.extend(files.iter().map(|file| format!("  {}", file.display())));
        lines.push(
            "  (<process id> and <n> stand for numbers chosen when the file is written)".to_owned(),
        );
        lines.push("Folders that may be created:".to_owned());
        lines.extend(
            folders
                .iter()
                .map(|folder| format!("  {}", folder.display())),
        );
    }
    if !plan.settings.is_empty() {
        let section = at
            .host
            .map_or_else(String::new, |host| format!(" [host.{}]", host.name()));
        lines.push(format!(
            "The interview will write to {}{section}",
            at.target.display()
        ));
        lines.extend(
            plan.settings
                .iter()
                .map(|(name, value)| format!("  {name}={value}")),
        );
    }
    lines
}

#[cfg(test)]
mod tests {
    use baley_core::catalog::Provider;
    use baley_core::policy::Value;
    use baley_core::policy::config_command::TypedPair;
    use serde_json::json;

    use super::*;
    use crate::config_command::interview::providers::Acknowledge;
    use crate::config_command::interview::writes::{Ack, Hosts};
    use crate::install::hosts::Allowance;

    const HOME: &str = "/u/.local/share/crenshawdev/baley";
    const GLOBAL: &str = "/u/.config/crenshawdev/baley/config.toml";
    const CLAUDE: &str = "/u/.claude/settings.json";

    fn acknowledgement(providers: &[Provider]) -> Ack {
        Ack::Requested(Acknowledge {
            providers: providers.to_vec(),
            owner: "Ada Owner".to_owned(),
        })
    }

    fn hosts(added: &[&'static str]) -> Hosts {
        Hosts {
            path: CLAUDE.into(),
            allowance: Allowance {
                document: json!({}),
                bytes: Vec::new(),
                read_digest: None,
                added: added.to_vec(),
            },
        }
    }

    fn list(names: &[&str]) -> TypedPair {
        TypedPair {
            name: "review.reviewers".to_owned(),
            host: None,
            value: Value::ReviewerList(names.iter().map(|name| (*name).to_owned()).collect()),
        }
    }

    fn answer(name: &str, value: &str) -> Vec<(String, String)> {
        vec![(name.to_owned(), value.to_owned())]
    }

    fn provider_steps() -> Plan {
        Plan {
            acknowledgement: Some(acknowledgement(&[Provider::OpenAi])),
            hosts: Some(hosts(&["api.openai.com"])),
            reviewers: Some(list(&["host", "openai"])),
            settings: Vec::new(),
        }
    }

    fn global_at() -> Where<'static> {
        Where {
            home: Path::new(HOME),
            global: Path::new(GLOBAL),
            target: Path::new(GLOBAL),
            layer: FileLayer::Global,
            host: None,
            in_project: false,
        }
    }

    /// How many lines are exactly `path`, indented as the files block does.
    fn listed(lines: &[String], path: impl AsRef<Path>) -> usize {
        let line = format!("  {}", path.as_ref().display());
        lines.iter().filter(|candidate| **candidate == line).count()
    }

    fn folder_of(path: &str) -> PathBuf {
        Path::new(path).parent().unwrap().to_path_buf()
    }

    #[test]
    fn a_file_the_confirmed_writes_can_change_missing_from_the_summary_is_caught() {
        let mut plan = provider_steps();
        plan.settings = answer("roles.planner.effort", "max");

        let lines = lines(&plan, &global_at());

        for ledger in [
            "baley.db",
            "baley.db-wal",
            "baley.db-shm",
            "baley.db.writer",
            "baley.db.maintenance",
        ] {
            assert_eq!(listed(&lines, Path::new(HOME).join(ledger)), 1, "{ledger}");
        }
        assert_eq!(listed(&lines, CLAUDE), 1);
        assert_eq!(
            listed(&lines, "/u/.claude/.settings.json.<process id>-<n>.tmp"),
            1
        );
        assert_eq!(listed(&lines, GLOBAL), 1);
        assert_eq!(
            listed(
                &lines,
                "/u/.config/crenshawdev/baley/.config.toml.<process id>-<n>.tmp"
            ),
            1
        );
        for folder in [Path::new(HOME), &folder_of(GLOBAL), &folder_of(CLAUDE)] {
            assert_eq!(listed(&lines, folder), 1, "{}", folder.display());
        }

        let mut plan = provider_steps();
        plan.settings = answer("roles.planner.effort", "max");
        let in_project = Where {
            target: Path::new("/r/baley.toml"),
            layer: FileLayer::Project,
            in_project: true,
            ..global_at()
        };

        let lines = super::lines(&plan, &in_project);

        assert_eq!(listed(&lines, "/r/baley.toml"), 1);
        assert_eq!(listed(&lines, "/r/.baley.toml.<process id>-<n>.tmp"), 1);
        assert_eq!(listed(&lines, GLOBAL), 1);
    }
    /// Whether some line holds every one of `parts`.
    fn names_together(lines: &[String], parts: &[&str]) -> bool {
        lines
            .iter()
            .any(|line| parts.iter().all(|part| line.contains(part)))
    }

    #[test]
    fn a_summary_missing_the_hosts_or_reviewer_list_it_will_write_is_caught() {
        let mut plan = provider_steps();
        plan.settings = answer("roles.planner.effort", "max");

        let lines = lines(&plan, &global_at());

        assert!(names_together(&lines, &["openai", "warning version 1"]));
        assert!(names_together(&lines, &["api.openai.com", CLAUDE]));
        assert!(names_together(&lines, &["[\"host\", \"openai\"]", GLOBAL]));
        assert!(lines.contains(&"  roles.planner.effort=max".to_owned()));

        let plan = Plan {
            acknowledgement: Some(acknowledgement(&Provider::ALL)),
            hosts: Some(hosts(&["api.openai.com", "api.deepseek.com"])),
            reviewers: Some(list(&["host", "openai", "deepseek"])),
            settings: Vec::new(),
        };

        let lines = super::lines(&plan, &global_at());

        assert!(names_together(
            &lines,
            &["openai", "deepseek", "warning version 1"]
        ));
        assert!(names_together(
            &lines,
            &["api.openai.com", "api.deepseek.com", CLAUDE]
        ));
        assert!(names_together(
            &lines,
            &["[\"host\", \"openai\", \"deepseek\"]", GLOBAL]
        ));
    }

    #[test]
    fn a_file_listed_for_a_step_that_will_not_run_is_caught() {
        let ledger = |lines: &[String]| {
            [
                "baley.db",
                "baley.db-wal",
                "baley.db-shm",
                "baley.db.writer",
                "baley.db.maintenance",
            ]
            .into_iter()
            .filter(|name| listed(lines, Path::new(HOME).join(name)) == 1)
            .count()
        };
        let only = |settings: Vec<(String, String)>, hosts: Option<Hosts>| Plan {
            acknowledgement: None,
            hosts,
            reviewers: None,
            settings,
        };

        let rung = lines(
            &only(answer("roles.planner.effort", "max"), None),
            &global_at(),
        );
        assert_eq!(listed(&rung, GLOBAL), 1);
        assert_eq!(
            listed(
                &rung,
                "/u/.config/crenshawdev/baley/.config.toml.<process id>-<n>.tmp"
            ),
            1
        );
        assert_eq!(listed(&rung, folder_of(GLOBAL)), 1);
        assert_eq!(ledger(&rung), 0);
        assert_eq!(listed(&rung, CLAUDE), 0);
        assert_eq!(listed(&rung, HOME), 0);

        let host_only = lines(
            &only(Vec::new(), Some(hosts(&["api.openai.com"]))),
            &global_at(),
        );
        assert_eq!(listed(&host_only, CLAUDE), 1);
        assert_eq!(
            listed(&host_only, "/u/.claude/.settings.json.<process id>-<n>.tmp"),
            1
        );
        assert_eq!(listed(&host_only, folder_of(CLAUDE)), 1);
        assert_eq!(listed(&host_only, GLOBAL), 0);
        assert_eq!(ledger(&host_only), 0);

        let model = lines(
            &only(answer("roles.planner.model", "sonnet"), None),
            &global_at(),
        );
        assert_eq!(ledger(&model), 5);
        assert_eq!(listed(&model, HOME), 1);
        assert_eq!(listed(&model, GLOBAL), 1);

        let project = Where {
            target: Path::new("/r/baley.toml"),
            layer: FileLayer::Project,
            in_project: true,
            ..global_at()
        };
        let in_project = lines(&only(answer("roles.planner.effort", "max"), None), &project);
        assert_eq!(ledger(&in_project), 5);
        assert_eq!(listed(&in_project, HOME), 1);
        assert_eq!(listed(&in_project, "/r/baley.toml"), 1);
        assert_eq!(
            listed(&in_project, "/r/.baley.toml.<process id>-<n>.tmp"),
            1
        );
        assert_eq!(listed(&in_project, GLOBAL), 0);
    }
}
