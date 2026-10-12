//! Owner commands over the evidence ledger.
pub(crate) mod anchor_plan;
pub(crate) mod answer;
mod args;
pub(crate) mod clock;
mod command_plan;
pub(crate) mod commands;
pub(crate) mod display;
pub(crate) mod forge;
pub(crate) mod open;
#[cfg(test)]
mod tests;
pub(crate) mod ticker;
mod trace;

use crate::folders::{Environment, Folders, Platform};
use crate::install::command::select_host;
pub use args::LedgerCommand;
use baley_core::policy::Host;
use std::{fmt, process::ExitCode, sync::Arc};

#[derive(Debug, PartialEq, Eq)]
struct CliRefusal(String);
impl fmt::Display for CliRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "baley: {}", self.0)
    }
}

/// Resolves and opens the ledger home and runs one synchronous owner command.
pub fn run(command: LedgerCommand) -> ExitCode {
    let started_at = clock::SystemClock::now();
    let result = begin(&command, || {
        let folders = Folders::resolve(Platform::current(), &Environment::read())
            .map_err(|e| display::Render::refusal(e.to_string()))?;
        let cwd = std::env::current_dir().map_err(|e| {
            display::store_error(&baley_store::StoreError::Unavailable(e.to_string()), None)
        })?;
        let store = Arc::new(
            open::store(&folders.home, &started_at, open::options())
                .map_err(|e| display::store_error(&e, None))?,
        );
        Ok((folders, cwd, store))
    })
    .map(|(host, (folders, cwd, store))| {
        commands::dispatch(command, host, store, cwd, &folders, started_at)
    })
    .unwrap_or_else(|e| e);
    ExitCode::from(display::emit(
        &result,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
}

/// Selects the host a doctor run judges, then runs `open`, which resolves
/// Baley's folders and opens the store. An unknown host is refused before
/// `open` runs, so nothing is created, opened or brought current for it. Any
/// other command selects no host.
fn begin<T>(
    command: &LedgerCommand,
    open: impl FnOnce() -> Result<T, display::Render>,
) -> Result<(Option<Host>, T), display::Render> {
    let host = match command {
        LedgerCommand::Doctor { host } => Some(select_host(host)?),
        _ => None,
    };
    Ok((host, open()?))
}
