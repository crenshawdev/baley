//! Host selection and gathering for `baley install`.

use std::io;
use std::process::ExitCode;

use baley_core::policy::Host;
use clap::Args;

use crate::folders::{Environment, Folders, Platform};
use crate::host_artifacts::{installed, stubs};
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::ledger::open;
use crate::models;

use super::receipt::{self, ArtifactOutcome};

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
    let seed = models::seed(&store, new_request_id(), &SystemClock::now());
    // Phase 15 plan 1 adds stub writes in tasks 6 and 7. Plan 2 adds JSON writes.
    let outcomes: Vec<_> = installed
        .placements
        .expected_files()
        .into_iter()
        .map(|file| (file.artifact, ArtifactOutcome::NotWritten))
        .collect();
    Ok(receipt::render(
        env!("CARGO_PKG_VERSION"),
        installed.layout.stable_path(),
        &installed.placements,
        &outcomes,
        &seed,
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
