//! The newly active binary records its own catalog (design 0012 section 5).
//! The updater runs this after the heartbeat stops and claim completion
//! returns, whatever its result, so a newer view set cannot fence its record.

use std::io;
use std::process::ExitCode;
use std::time::Duration;

use baley_core::catalog::{HINT_VERSION, USER_PROJECT};

use crate::folders::{Environment, Folders, Platform};
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::ledger::open;
use crate::models;
use crate::process::{Launch, Output, Process};

use super::deliver::{Activated, Failure};
use super::installation::Layout;

/// The catalog seed's result, used only by the update receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The child confirmed that its compiled seed is recorded.
    Recorded,
    /// The child could not confirm its seed.
    NotRecorded {
        /// The exit, signal, timeout or start error.
        cause: String,
    },
}

/// Requests the staged binary's own seed only after successful activation.
pub fn launch_for(layout: &Layout, delivery: &Result<Activated, Failure>) -> Option<Launch> {
    let activated = delivery.as_ref().ok()?;
    Some(
        Launch::new(layout.staged_binary(activated.staged_version))
            .args(["update", "seed"])
            .own_group()
            .timeout(Duration::from_secs(60))
            .limit(4 * 1024),
    )
}

/// Interprets the child's result without reading a store or starting a process.
pub fn interpret(result: io::Result<Output>) -> Outcome {
    let cause = match result {
        Ok(output) if output.success() => return Outcome::Recorded,
        Ok(output) => match output.code() {
            Some(code) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let first = stderr.lines().next().unwrap_or_default();
                if first.is_empty() {
                    format!("exit status {code}")
                } else {
                    format!("exit status {code}: {first}")
                }
            }
            None => match output.signal() {
                Some(signal) => format!("signal {signal}"),
                None => "process ended without an exit status".into(),
            },
        },
        Err(error) if error.kind() == io::ErrorKind::TimedOut => format!("timed out: {error}"),
        Err(error) => format!("could not start catalog seed: {error}"),
    };
    Outcome::NotRecorded { cause }
}

/// Runs the requested child. The caller must stop renewal and attempt claim
/// completion first, even when completion fails, before launching this seed.
pub fn gather(launch: &Launch) -> io::Result<Output> {
    crate::process::System.run(launch)
}

/// Runs the hidden `baley update seed` command with this binary's own table.
pub fn run() -> ExitCode {
    let render = seed_compiled().unwrap_or_else(|error| error);
    ExitCode::from(display::emit(
        &render,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ))
}

fn seed_compiled() -> Result<Render, Render> {
    let folders = Folders::resolve(Platform::current(), &Environment::read())
        .map_err(|error| Render::refusal(error.to_string()))?;
    let at = SystemClock::now();
    let store = open::store(&folders.home, &at, open::options())
        .map_err(|error| display::store_error(&error, None))?;
    let recorded = models::seed(&store, new_request_id(), &at)
        .map_err(|error| display::store_error(&error, Some(USER_PROJECT)))?;
    let line = if recorded {
        format!("seeded the model catalog with hint table version {HINT_VERSION}")
    } else {
        format!("the model catalog seed with hint table version {HINT_VERSION} is already recorded")
    };
    Ok(Render::line(line, 0))
}

#[cfg(test)]
mod tests {
    use super::{Outcome, interpret, launch_for};
    use crate::folders::Environment;
    use crate::process::{Output, StdioPlan, Stream, stdio_plan};
    use crate::update::deliver::{Activated, Failure, Step};
    use crate::update::events::FailureCode;
    use crate::update::installation::Layout;
    use crate::update::version::Version;
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::time::Duration;

    #[test]
    fn a_seed_run_before_activation_or_by_the_old_binary_is_caught() {
        let layout = Layout::resolve(&Environment {
            home: Some("/home/o".into()),
            ..Environment::default()
        })
        .unwrap();
        let old = Version::parse("0.1.0").unwrap();
        let new = Version::parse("0.2.0").unwrap();
        let activated = Ok(Activated {
            active_version: new,
            staged_version: new,
            kept_versions: vec![old, new],
        });

        let launch = launch_for(&layout, &activated).expect("the active version seeds");

        assert_eq!(
            launch.program,
            OsStr::new("/home/o/.local/lib/crenshawdev/baley/versions/0.2.0/baley")
        );
        assert_eq!(
            launch.args,
            [OsString::from("update"), OsString::from("seed")]
        );
        assert!(launch.own_group);
        assert_eq!(launch.timeout, Some(Duration::from_secs(60)));
        assert_eq!(launch.limit, 4096);
        assert!(!launch.inherit);
        assert!(!launch.inherit_stdin);
        assert_eq!(
            stdio_plan(&launch),
            StdioPlan {
                stdin: Stream::Null,
                stdout: Stream::Piped,
                stderr: Stream::Piped,
            }
        );

        for failure in [
            Failure {
                step: Step::Activation,
                code: FailureCode::ActivationConflict,
                cause: "the stable path changed".into(),
                staged_version: Some(new),
            },
            Failure {
                step: Step::Staging,
                code: FailureCode::StagingConflict,
                cause: "the staged path holds another file".into(),
                staged_version: None,
            },
        ] {
            assert_eq!(launch_for(&layout, &Err(failure)), None);
        }
    }

    #[test]
    fn a_failed_seed_run_reported_as_recorded_is_caught() {
        assert_eq!(
            interpret(Ok(Output::exited(
                0,
                "seeded the model catalog with hint table version 3\n",
                ""
            ))),
            Outcome::Recorded
        );

        for (result, cause) in [
            (
                Ok(Output::exited(1, "", "store busy, retry\nsecond line\n")),
                "exit status 1: store busy, retry",
            ),
            (Ok(Output::signaled(9)), "signal 9"),
            (
                Err(io::Error::new(io::ErrorKind::NotFound, "binary missing")),
                "could not start catalog seed: binary missing",
            ),
            (
                Err(io::Error::new(io::ErrorKind::TimedOut, "deadline reached")),
                "timed out: deadline reached",
            ),
        ] {
            assert_eq!(
                interpret(result),
                Outcome::NotRecorded {
                    cause: cause.into()
                }
            );
        }
    }
}
