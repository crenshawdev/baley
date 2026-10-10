//! Host selection and gathering for `baley install`.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::ExitCode;

use baley_core::catalog::USER_PROJECT;
use baley_core::policy::{FileLayer, Host, Schema, Value, merge, parse_layer};
use clap::Args;

use crate::folders::{self, Environment, Folders, Platform};
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

/// The file in Baley's home that the one running install holds.
const LOCK_FILE: &str = "install.lock";

/// What trying to take the install lock found, when it was not taken.
#[derive(Debug)]
enum Observed {
    /// Another install holds the lock.
    Held,
    /// The lock file could not be created, opened or locked.
    Unusable(String),
}

/// The install lock, held for as long as this value lives. Closing the file
/// releases it, so an install that crashes never leaves the lock behind.
#[derive(Debug)]
struct InstallLock {
    _file: File,
}

/// Judges one attempt to lock the file: a held lock is another install
/// running, anything else is a lock that cannot be used.
fn observe(attempt: Result<(), TryLockError>) -> Result<(), Observed> {
    match attempt {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(Observed::Held),
        Err(TryLockError::Error(error)) => Err(Observed::Unusable(error.to_string())),
    }
}

/// The refusal for a lock that was not taken.
fn refusal(observed: &Observed, path: &Path) -> Render {
    match observed {
        Observed::Held => Render::refusal(
            "install-running: another baley install is running; try again when it has finished",
        ),
        Observed::Unusable(cause) => Render::refusal(format!(
            "install-lock-unavailable: cannot take the install lock at {}: {cause}",
            path.display()
        )),
    }
}

/// Takes the install lock without waiting. Two installs overlapping would
/// each plan from the same record and could leave the record naming hashes
/// the files no longer hold, so the second one is refused before it reads
/// anything. The lock sits in Baley's own home, never in Claude's folder.
fn acquire(home: &Path) -> Result<InstallLock, Render> {
    let path = home.join(LOCK_FILE);
    let unusable = |cause: io::Error| refusal(&Observed::Unusable(cause.to_string()), &path);
    folders::create_private(home).map_err(unusable)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(unusable)?;
    observe(file.try_lock()).map_err(|observed| refusal(&observed, &path))?;
    Ok(InstallLock { _file: file })
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
    // Held to the end of the run, record included.
    let _lock = acquire(&folders.home)?;
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
    use std::fs::TryLockError;
    use std::io;
    use std::path::Path;

    use baley_core::policy::Host;

    use super::{Observed, acquire, observe, refusal, select_host};

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

    #[test]
    fn a_second_install_started_while_one_holds_the_lock_not_refused_is_caught() {
        let home = tempfile::tempdir().unwrap();
        let first = acquire(home.path()).expect("the first install takes the lock");

        let refused = acquire(home.path()).expect_err("a second install while the first runs");
        assert_eq!(
            refused.lines,
            ["install-running: another baley install is running; try again when it has finished"]
        );
        assert_eq!(refused.code, 2);
        assert!(refused.error);

        drop(first);
        acquire(home.path()).expect("the lock is free once the first install ends");
    }

    #[test]
    fn a_lock_that_cannot_be_taken_reported_as_a_running_install_is_caught() {
        assert!(observe(Ok(())).is_ok());
        assert!(matches!(
            observe(Err(TryLockError::WouldBlock)),
            Err(Observed::Held)
        ));
        let failed = io::Error::from(io::ErrorKind::PermissionDenied);
        let observed = observe(Err(TryLockError::Error(failed))).expect_err("an io failure");

        let refused = refusal(&observed, Path::new("/h/install.lock"));
        assert_eq!(refused.lines.len(), 1);
        let line = &refused.lines[0];
        assert!(line.contains("/h/install.lock"), "{line}");
        assert!(line.contains("permission denied"), "{line}");
        assert!(!line.contains("another baley install is running"), "{line}");
        assert_eq!(refused.code, 2);
        assert!(refused.error);
    }
}
