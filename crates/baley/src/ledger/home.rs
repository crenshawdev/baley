//! The explicit home used until platform discovery arrives.
use super::CliRefusal;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Requires an explicit existing home directory.
pub(super) fn home_from(value: Option<OsString>) -> Result<PathBuf, CliRefusal> {
    let home = value
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            CliRefusal(
                "BALEY_HOME is not set; this build opens the ledger only at $BALEY_HOME/baley.db"
                    .into(),
            )
        })?;
    if !home.is_dir() {
        return Err(CliRefusal(format!(
            "BALEY_HOME is {}, which is not a directory",
            home.display()
        )));
    }
    Ok(home)
}
/// Refuses a home without an existing ledger.
pub(super) fn ledger_file(home: &Path) -> Result<PathBuf, CliRefusal> {
    let file = home.join("baley.db");
    if !file.is_file() {
        return Err(CliRefusal(format!(
            "no ledger at {}; this build does not create one",
            file.display()
        )));
    }
    Ok(file)
}
