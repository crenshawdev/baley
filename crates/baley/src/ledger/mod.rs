//! Owner commands over the evidence ledger.
mod answer;
mod args;
mod clock;
mod commands;
mod display;
mod forge;
pub(crate) mod open;
mod remotes;
#[cfg(test)]
mod tests;
mod ticker;
mod trace;

use crate::folders::{Environment, Folders, Platform};
pub use args::LedgerCommand;
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
    let result = (|| {
        let folders = Folders::resolve(Platform::current(), &Environment::read())
            .map_err(|e| display::Render::refusal(e.to_string()))?;
        let cwd = std::env::current_dir().map_err(|e| {
            display::store_error(&baley_store::StoreError::Unavailable(e.to_string()), None)
        })?;
        let store = Arc::new(
            open::store(&folders.home, &started_at, open::options())
                .map_err(|e| display::store_error(&e, None))?,
        );
        Ok(commands::dispatch(command, store, cwd, started_at))
    })()
    .unwrap_or_else(|e| e);
    for line in result.lines {
        if result.error {
            eprintln!("baley: {line}");
        } else {
            println!("{line}");
        }
    }
    ExitCode::from(result.code)
}
