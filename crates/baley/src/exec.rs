//! Runs an owner command with one key and redacts its output.
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitCode, ExitStatus};

use aho_corasick::AhoCorasick;
use clap::Args;

use crate::folders::{Environment, FolderRefusal, Folders, Platform};
use crate::keys::{self, Key, Keys, KeysRefusal};
use crate::process::{Launch, Process, System};

/// Arguments for running one command with a provider key.
#[derive(Args, Debug, Clone)]
pub struct ExecArgs {
    /// The key's name exactly as written in keys.env.
    #[arg(long, value_name = "NAME")]
    pub key: String,
    /// The command and its arguments, after --.
    #[arg(last = true, required = true, value_name = "COMMAND")]
    pub command: Vec<OsString>,
}

#[derive(Debug)]
enum ExecError {
    Folder(FolderRefusal),
    Keys(KeysRefusal),
    NotStarted {
        program: OsString,
        error: String,
    },
    WaitFailed {
        program: OsString,
        error: String,
    },
    NotForwarded {
        program: OsString,
        stream: &'static str,
        error: String,
    },
}
impl ExecError {
    fn exit_code(&self) -> u8 {
        match self {
            Self::Folder(_) | Self::Keys(_) | Self::NotStarted { .. } => 2,
            Self::WaitFailed { .. } | Self::NotForwarded { .. } => 3,
        }
    }
}
impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Folder(error) => error.fmt(f),
            Self::Keys(error) => error.fmt(f),
            Self::NotStarted { program, error } => write!(
                f,
                "command-not-started: cannot start {}: {error}",
                program.to_string_lossy()
            ),
            Self::WaitFailed { program, error } => write!(
                f,
                "waiting for {} failed: {error}; its exit status is unknown",
                program.to_string_lossy()
            ),
            Self::NotForwarded {
                program,
                stream,
                error,
            } => write!(
                f,
                "forwarding {}'s {stream} failed: {error}; output after that point was not delivered",
                program.to_string_lossy()
            ),
        }
    }
}

fn find<'k>(keys: &'k Result<Keys, KeysRefusal>, name: &str) -> Result<&'k Key, ExecError> {
    keys.as_ref()
        .map_err(|error| ExecError::Keys(error.clone()))?
        .get(name)
        .map_err(ExecError::Keys)
}

fn launch_for(key: &Key, command: &[OsString]) -> Launch {
    Launch::owner_command(&command[0])
        .args(&command[1..])
        .env(key.name(), key.expose())
}

/// The search pattern stays private and is never formatted.
pub(crate) struct Redactor {
    searcher: AhoCorasick,
    placeholder: String,
}
impl Redactor {
    fn new(key: &Key) -> Self {
        Self {
            searcher: AhoCorasick::new([key.expose()]).expect("one non-empty key pattern builds"),
            placeholder: key.placeholder(),
        }
    }

    fn copy(&self, from: impl Read, mut to: impl Write) -> io::Result<()> {
        self.searcher
            .try_stream_replace_all(from, &mut to, &[&self.placeholder])?;
        to.flush()
    }
}

struct Flushing<W>(W);
impl<W: Write> Write for Flushing<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write_all(bytes)?;
        self.0.flush()?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn outcome(
    program: &OsStr,
    status: ExitStatus,
    stdout: io::Result<()>,
    stderr: io::Result<()>,
) -> Result<ExitStatus, ExecError> {
    for (stream, result) in [("stdout", stdout), ("stderr", stderr)] {
        if let Err(error) = result
            && error.kind() != io::ErrorKind::BrokenPipe
        {
            return Err(ExecError::NotForwarded {
                program: program.into(),
                stream,
                error: error.to_string(),
            });
        }
    }
    Ok(status)
}

fn execute(
    process: &mut dyn Process,
    launch: &Launch,
    redactor: &Redactor,
) -> Result<ExitStatus, ExecError> {
    let mut child = process
        .start(launch)
        .map_err(|error| ExecError::NotStarted {
            program: launch.program.clone(),
            error: error.to_string(),
        })?;
    let stdout = child.stdout().expect("piped stdout");
    let stderr = child.stderr().expect("piped stderr");
    std::thread::scope(|scope| {
        let out = scope.spawn(move || redactor.copy(stdout, Flushing(io::stdout())));
        let err = scope.spawn(move || redactor.copy(stderr, Flushing(io::stderr())));
        let status = child.wait();
        let stdout = out.join().expect("stdout reader");
        let stderr = err.join().expect("stderr reader");
        let status = status.map_err(|error| ExecError::WaitFailed {
            program: launch.program.clone(),
            error: error.to_string(),
        })?;
        outcome(&launch.program, status, stdout, stderr)
    })
}

/// Maps a child's exit or signal to the command-line status.
pub(crate) fn exit_code(status: ExitStatus) -> u8 {
    status.code().map_or_else(
        || status.signal().map_or(1, |signal| (128 + signal) as u8),
        |code| code as u8,
    )
}

/// Writes the error line and returns its exit code. A stderr that cannot be
/// written must not turn a refusal into a panic, so the write error is dropped.
fn report(error: &ExecError, to: &mut impl Write) -> u8 {
    let _ = writeln!(to, "baley: {error}");
    error.exit_code()
}

/// Reads the owner's keys, starts the command and forwards its redacted streams.
pub fn run(args: ExecArgs) -> ExitCode {
    let result = (|| {
        let folders = Folders::resolve(Platform::current(), &Environment::read())
            .map_err(ExecError::Folder)?;
        let keys = keys::load(&folders.config);
        let key = find(&keys, &args.key)?;
        let launch = launch_for(key, &args.command);
        let redactor = Redactor::new(key);
        execute(&mut System, &launch, &redactor)
    })();
    match result {
        Ok(status) => ExitCode::from(exit_code(status)),
        Err(error) => ExitCode::from(report(&error, &mut io::stderr())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{LineFault, LineProblem};
    use crate::process::{Output, validate_launch};
    use std::path::Path;

    fn fixture() -> Keys {
        Keys::parsed(Path::new("/c/keys.env"), b"A=SENTINEL-7c1e")
    }

    struct OneByte<'a>(&'a [u8]);
    impl Read for OneByte<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let len = bytes.len().min(1);
            self.0.read(&mut bytes[..len])
        }
    }

    fn redact(bytes: &[u8]) -> Vec<u8> {
        let keys = fixture();
        let mut output = Vec::new();
        Redactor::new(keys.get("A").unwrap())
            .copy(OneByte(bytes), &mut output)
            .unwrap();
        output
    }

    #[test]
    fn key_split_across_reads_is_still_replaced() {
        assert_eq!(redact(b"xxSENTINEL-7c1eyy"), b"xx[baley:A]yy");
    }

    #[test]
    fn adjacent_keys_are_each_replaced() {
        assert_eq!(redact(b"SENTINEL-7c1eSENTINEL-7c1e"), b"[baley:A][baley:A]");
    }

    #[test]
    fn trailing_partial_key_is_passed_on_not_dropped() {
        assert_eq!(redact(b"abSENTINEL-7c"), b"abSENTINEL-7c");
    }

    #[test]
    fn output_without_the_key_passes_through_byte_for_byte() {
        assert_eq!(redact(b"\xffx\ry"), b"\xffx\ry");
    }

    #[derive(Default)]
    struct Writes {
        calls: Vec<Vec<u8>>,
    }
    impl Write for Writes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls.push(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.calls.push(Vec::new());
            Ok(())
        }
    }

    #[test]
    fn each_write_reaches_the_stream_without_waiting_for_a_newline() {
        let mut writer = Flushing(Writes::default());
        assert_eq!(writer.write(b"prompt").unwrap(), 6);
        assert_eq!(writer.0.calls, [b"prompt".to_vec(), vec![]]);
        assert_eq!(writer.write(b"> ").unwrap(), 2);
        assert_eq!(
            writer.0.calls,
            [b"prompt".to_vec(), vec![], b"> ".to_vec(), vec![]]
        );
    }

    #[test]
    fn launch_puts_the_key_in_the_child_environment_under_its_name() {
        let keys = fixture();
        let launch = launch_for(
            keys.get("A").unwrap(),
            &["git".into(), "status".into(), "--short".into()],
        );
        assert_eq!(launch.program, "git");
        assert_eq!(launch.args, ["status", "--short"]);
        assert_eq!(launch.env, [("A".into(), Some("SENTINEL-7c1e".into()))]);
        assert!(launch.inherit_stdin);
        assert_eq!(launch.stdin, None);
        assert_eq!(launch.cwd, None);
        assert_eq!(launch.timeout, None);
        assert_eq!(launch.limit, usize::MAX);
        assert!(!launch.own_group);
        assert!(!launch.die_with_parent);
        assert!(!launch.inherit);
        assert!(validate_launch(&launch).is_ok());
    }

    #[test]
    fn every_keys_refusal_stops_the_command_with_its_own_code() {
        for refusal in [
            KeysRefusal::NotRegular {
                path: "/c/keys.env".into(),
            },
            KeysRefusal::Exposed {
                path: "/c/keys.env".into(),
                user: 1000,
                owner: None,
                mode: Some(0o644),
            },
            KeysRefusal::Unreadable {
                path: "/c/keys.env".into(),
                error: "denied".into(),
            },
            KeysRefusal::Invalid {
                path: "/c/keys.env".into(),
                faults: vec![LineFault {
                    line: 2,
                    problem: LineProblem::EmptyValue,
                }],
            },
        ] {
            let result = Err(refusal.clone());
            let ExecError::Keys(actual) = find(&result, "A").unwrap_err() else {
                panic!("keys refusal lost");
            };
            assert_eq!(actual, refusal);
            assert_eq!(actual.code(), refusal.code());
        }
        let keys = Ok(fixture());
        assert_eq!(find(&keys, "A").unwrap().name(), "A");
        for name in ["B", "a", "", " A"] {
            let ExecError::Keys(actual) = find(&keys, name).unwrap_err() else {
                panic!("missing key refusal lost");
            };
            assert_eq!(actual.code(), "no-such-key");
        }
    }

    #[test]
    fn refusals_render_with_their_code_and_exit_status() {
        let cases = [
            (
                ExecError::Folder(FolderRefusal::BaleyHomeEmpty),
                "baley: baley-home-invalid: BALEY_HOME is set but empty; set it to an absolute path, or unset it to use Baley's own folders",
                2,
            ),
            (
                ExecError::Keys(KeysRefusal::Exposed {
                    path: "/c/keys.env".into(),
                    user: 1000,
                    owner: None,
                    mode: Some(0o644),
                }),
                "baley: keys-file-exposed: /c/keys.env has mode 0644, so its group or others can read it (fix: chmod 600 /c/keys.env)",
                2,
            ),
            (
                ExecError::NotStarted {
                    program: "no-such-program".into(),
                    error: io::Error::from_raw_os_error(2).to_string(),
                },
                "baley: command-not-started: cannot start no-such-program: No such file or directory (os error 2)",
                2,
            ),
            (
                ExecError::WaitFailed {
                    program: "cmd".into(),
                    error: "wait error".into(),
                },
                "baley: waiting for cmd failed: wait error; its exit status is unknown",
                3,
            ),
            (
                ExecError::NotForwarded {
                    program: "cmd".into(),
                    stream: "stdout",
                    error: io::Error::from_raw_os_error(28).to_string(),
                },
                "baley: forwarding cmd's stdout failed: No space left on device (os error 28); output after that point was not delivered",
                3,
            ),
        ];
        for (error, expected, code) in cases {
            assert_eq!(format!("baley: {error}"), expected);
            assert_eq!(error.exit_code(), code);
        }
    }

    #[test]
    fn lost_output_is_reported_but_a_closed_reader_is_not() {
        let status = Output::exited(0, "", "").status;
        let error = outcome(
            OsStr::new("cmd"),
            status,
            Err(io::ErrorKind::StorageFull.into()),
            Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.exit_code(), 3);
        assert!(matches!(
            error,
            ExecError::NotForwarded {
                stream: "stdout",
                ..
            }
        ));
        let error = outcome(
            OsStr::new("cmd"),
            status,
            Ok(()),
            Err(io::ErrorKind::Other.into()),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ExecError::NotForwarded {
                stream: "stderr",
                ..
            }
        ));
        for (stdout, stderr) in [
            (Err(io::ErrorKind::BrokenPipe.into()), Ok(())),
            (Ok(()), Err(io::ErrorKind::BrokenPipe.into())),
        ] {
            assert_eq!(
                exit_code(outcome(OsStr::new("cmd"), status, stdout, stderr).unwrap()),
                0
            );
        }
        let status = Output::exited(3, "", "").status;
        assert_eq!(
            exit_code(outcome(OsStr::new("cmd"), status, Ok(()), Ok(())).unwrap()),
            3
        );
    }

    struct Full;
    impl Write for Full {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::StorageFull.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_failed_write_ends_the_copy_with_its_error() {
        let keys = fixture();
        let error = Redactor::new(keys.get("A").unwrap())
            .copy(b"xx".as_slice(), Flushing(Full))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    }

    #[test]
    fn unwritable_stderr_does_not_replace_the_error_exit_code() {
        let refusal = ExecError::Folder(FolderRefusal::BaleyHomeEmpty);
        assert_eq!(report(&refusal, &mut Full), 2);
        let lost = ExecError::WaitFailed {
            program: "cmd".into(),
            error: "wait error".into(),
        };
        assert_eq!(report(&lost, &mut Full), 3);
    }

    #[test]
    fn child_code_passes_through_and_signal_deaths_are_mapped() {
        for code in [0, 3, 255] {
            assert_eq!(exit_code(Output::exited(code, "", "").status), code as u8);
        }
        assert_eq!(exit_code(Output::signaled(9).status), 137);
        assert_eq!(exit_code(Output::signaled(15).status), 143);
    }
}
