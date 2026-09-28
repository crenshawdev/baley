//! Owner commands over the evidence ledger.
mod answer;
mod args;
mod clock;
mod commands;
mod display;
mod forge;
mod home;
mod ids;
mod open;
mod remotes;
#[cfg(test)]
mod tests;
mod ticker;
mod trace;

pub use args::LedgerCommand;
use baley_store_sqlite::SqliteStore;
use std::{fmt, process::ExitCode, sync::Arc};

#[derive(Debug, PartialEq, Eq)]
struct CliRefusal(String);
impl fmt::Display for CliRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "baley: {}", self.0)
    }
}

/// Opens the explicit ledger home and runs one synchronous owner command.
pub fn run(command: LedgerCommand) -> ExitCode {
    let started_at = clock::SystemClock::now();
    let result = (|| {
        let home = home::home_from(std::env::var_os("BALEY_HOME"))
            .map_err(|e| display::Render::refusal(e.0))?;
        home::ledger_file(&home).map_err(|e| display::Render::refusal(e.0))?;
        let cwd = std::env::current_dir().map_err(|e| {
            display::store_error(&baley_store::StoreError::Unavailable(e.to_string()), None)
        })?;
        let store = Arc::new(
            SqliteStore::open(&home, &started_at, open::options())
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
