//! Host selection and gathering for `baley install`.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::{fs, io};

use baley_core::catalog::USER_PROJECT;
use baley_core::policy::{FileLayer, Host, Schema, Value, merge, parse_layer};
use clap::Args;

use crate::folders::{Environment, Folders, Platform};
use crate::host_artifacts::{installed, stubs};
use crate::host_doctor::placed;
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::ledger::open;
use crate::update::installation::{self, Active};
use crate::{models, settings};

use super::{apply, event, plan, receipt, record};

/// The host whose user-level wiring install places.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// The host name, defaulting to Claude Code.
    #[arg(long, value_name = "name", default_value = "claude-code")]
    pub host: String,
}

/// Runs installation gathering and emits its receipt or refusal.
pub fn run(args: InstallArgs) -> ExitCode {
    let render = install(args).unwrap_or_else(|error| error);
    ExitCode::from(display::emit(
        &render,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ))
}

fn select_host(name: &str) -> Result<Host, Render> {
    Host::parse(name).ok_or_else(|| {
        let supported = Host::ALL.map(Host::name).join(", ");
        Render::refusal(format!(
            "unknown-host: {name} is not a host Baley supports; supported: {supported}"
        ))
    })
}

fn install(args: InstallArgs) -> Result<Render, Render> {
    let Host::ClaudeCode = select_host(&args.host)?;
    let env = Environment::read();
    let claude_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
    let folders = Folders::resolve(Platform::current(), &env)
        .map_err(|error| Render::refusal(error.to_string()))?;
    let manifest = stubs::manifest(&stubs::front_doors()).expect("unique compiled front doors");
    let installed = installed::resolve(&env, claude_config_dir, &manifest)
        .map_err(|error| Render::refusal(error.to_string()))?;
    let at = SystemClock::now();
    let store = open::store(&folders.home, &at, open::options())
        .map_err(|error| display::store_error(&error, None))?;
    let latest = record::latest(&store, args.host.as_str())
        .map_err(|error| display::store_error(&error, Some(USER_PROJECT)))?;
    let latest_payload = latest.as_ref().map(|latest| &latest.payload);
    let mut observations = BTreeMap::new();
    for file in installed.placements.expected_files() {
        if file.stub.is_some() {
            observations
                .entry(file.path.to_path_buf())
                .or_insert_with(|| plan::Observation {
                    state: placed::read(file.path),
                    symbolic_link: fs::symlink_metadata(file.path)
                        .is_ok_and(|meta| meta.is_symlink()),
                });
        }
    }
    let plan = plan::judge(&installed.placements, latest_payload, &observations)?;
    let applied = plan::applied(&plan, &apply::run(&plan));
    let seed = models::seed(&store, new_request_id(), &SystemClock::now());

    let schema = Schema::standard();
    let global = settings::read(&settings::global_path(&folders)).and_then(|file| {
        file.as_ref()
            .map(|file| parse_layer(file, FileLayer::Global, schema))
            .transpose()
    });
    let auto = global.ok().and_then(|global| {
        let policy = merge(schema, None, global.as_ref(), None);
        match policy.settings["updates.auto"].value {
            Some(Value::Bool(value)) => Some(value),
            _ => None,
        }
    });
    let staged_version = installation::gather_stable(&installed.layout)
        .ok()
        .and_then(
            |seen| match installation::judge_active(&installed.layout, &seen) {
                Active::Version(version) => Some(version.to_string()),
                Active::NotManaged(_) => None,
            },
        );
    let facts = plan::facts(
        env!("CARGO_PKG_VERSION"),
        &installed.placements,
        latest_payload,
        &applied,
        &seed,
        event::Updates {
            auto,
            staged_version,
        },
    );
    let recorded = record::write(
        &store,
        &facts,
        latest.as_ref().map(|latest| latest.seq),
        new_request_id(),
        &SystemClock::now(),
    );
    Ok(receipt::render_applied(
        env!("CARGO_PKG_VERSION"),
        installed.layout.stable_path(),
        &installed.placements,
        &applied,
        &seed,
        &recorded,
    ))
}

#[cfg(test)]
mod tests {
    use baley_core::policy::Host;

    use super::select_host;

    #[test]
    fn an_install_for_another_host_not_refused_unknown_host_is_caught() {
        for name in ["codex", "Claude-Code"] {
            let refused = select_host(name).expect_err("unsupported host");
            assert_eq!(
                refused.lines,
                [format!(
                    "unknown-host: {name} is not a host Baley supports; supported: claude-code"
                )]
            );
            assert_eq!(refused.code, 2);
            assert!(refused.error);
        }
        assert_eq!(select_host("claude-code").unwrap(), Host::ClaudeCode);
    }
}
