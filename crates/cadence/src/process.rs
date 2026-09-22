//! One boundary for starting an external program.
//!
//! Every spawn goes through `Process`, so a check can hand the code a recorded
//! fake instead of running real git, gpg or `sh`. `System` is the only
//! implementation that starts a child. `Recorded` is the fake: scripted outputs
//! in, the launches it was given out.
//!
//! The environment travels in the `Launch` rather than being fixed here,
//! because the call sites disagree: `rail::git::run` removes
//! `GIT_LITERAL_PATHSPECS` where `pause::git::run` sets it to 1, and
//! `recall::history` adds `GIT_NO_LAZY_FETCH` that no other git call sets.
//! Fixing one environment here would change behavior at those sites.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// What to run, and how to hold it while it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Where the child runs. `None` leaves it in this process's own directory,
    /// which is what a `git -C <dir>` call site relies on.
    pub cwd: Option<PathBuf>,
    /// Bytes written to the child's stdin, which is then closed.
    pub stdin: Option<Vec<u8>>,
    /// Environment for the child. `None` removes the variable.
    pub env: Vec<(String, Option<OsString>)>,
    /// Bytes kept per stream. The rest is read and dropped, and the matching
    /// `complete` flag on the output is false. Unbounded unless a caller that
    /// must not be flooded by a child sets it.
    pub limit: usize,
    /// Kill the child once it has run this long.
    pub timeout: Option<Duration>,
    /// Put the child in its own process group, and kill that group once the
    /// child is reaped, so a descendant cannot hold the pipes open.
    pub own_group: bool,
    /// Kill the child when the process that started it dies.
    pub die_with_parent: bool,
    /// Let the child use this process's own stdin, stdout and stderr. Nothing
    /// is captured, so the output carries no bytes.
    pub inherit: bool,
}

impl Launch {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            cwd: None,
            stdin: None,
            env: Vec::new(),
            limit: usize::MAX,
            timeout: None,
            own_group: false,
            die_with_parent: false,
            inherit: false,
        }
    }

    pub fn cwd(mut self, cwd: impl AsRef<Path>) -> Self {
        self.cwd = Some(cwd.as_ref().to_owned());
        self
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args.extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    pub fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.env.push((key.to_owned(), Some(value.as_ref().to_owned())));
        self
    }

    pub fn unset(mut self, key: &str) -> Self {
        self.env.push((key.to_owned(), None));
        self
    }

    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self
    }

    pub fn limit(mut self, bytes: usize) -> Self {
        self.limit = bytes;
        self
    }

    pub fn timeout(mut self, after: Duration) -> Self {
        self.timeout = Some(after);
        self
    }

    pub fn own_group(mut self) -> Self {
        self.own_group = true;
        self
    }

    pub fn die_with_parent(mut self) -> Self {
        self.die_with_parent = true;
        self
    }

    pub fn inherit(mut self) -> Self {
        self.inherit = true;
        self
    }

    /// The arguments as they read in a message, for a caller's error text.
    pub fn argument_text(&self) -> String {
        self.args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// What a finished child left behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// False when the stream was longer than the launch's limit.
    pub stdout_complete: bool,
    pub stderr_complete: bool,
}

impl Output {
    /// An exit with this code and these streams, for a fake's script.
    pub fn exited(code: i32, stdout: impl Into<Vec<u8>>, stderr: impl Into<Vec<u8>>) -> Self {
        Self {
            status: ExitStatus::from_raw(code << 8),
            stdout: stdout.into(),
            stderr: stderr.into(),
            stdout_complete: true,
            stderr_complete: true,
        }
    }

    /// A death by signal, for a fake's script.
    pub fn signaled(signal: i32) -> Self {
        Self {
            status: ExitStatus::from_raw(signal),
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_complete: true,
            stderr_complete: true,
        }
    }

    pub fn success(&self) -> bool {
        self.status.success()
    }

    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    pub fn signal(&self) -> Option<i32> {
        self.status.signal()
    }

    pub fn complete(&self) -> bool {
        self.stdout_complete && self.stderr_complete
    }
}

/// The one way to start an external program.
pub trait Process {
    fn run(&mut self, launch: &Launch) -> std::io::Result<Output>;

    /// Start a child and hand it back while it runs, for the one caller that
    /// reads a stream as it arrives rather than after the fact. Only `System`
    /// offers this; a fake refuses it unless it scripts children of its own.
    fn start(&mut self, launch: &Launch) -> std::io::Result<Box<dyn Child>> {
        let _ = launch;
        Err(std::io::Error::other("this process does not start children"))
    }
}

/// A child that is still running.
pub trait Child {
    /// Taken once; the caller reads it on its own thread.
    fn stdout(&mut self) -> Option<Box<dyn Read + Send>>;
    fn stderr(&mut self) -> Option<Box<dyn Read + Send>>;
    fn wait(&mut self) -> std::io::Result<ExitStatus>;
    fn kill(&mut self) -> std::io::Result<()>;
}

/// Starts real children.
#[derive(Clone, Copy, Debug, Default)]
pub struct System;

impl System {
    fn command(launch: &Launch) -> Command {
        let mut command = Command::new(&launch.program);
        command.args(&launch.args);
        if let Some(cwd) = &launch.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &launch.env {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        if launch.inherit {
            command.stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
        } else {
            command
                .stdin(if launch.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }
        if launch.own_group {
            command.process_group(0);
        }
        if launch.die_with_parent {
            let owner = std::process::id() as libc::pid_t;
            // Safety: the closure runs between fork and exec in the child and
            // calls only async-signal-safe functions.
            unsafe {
                command.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() != owner {
                        return Err(std::io::Error::other("the owner exited before the spawn"));
                    }
                    Ok(())
                })
            };
        }

        command
    }
}

impl Process for System {
    fn run(&mut self, launch: &Launch) -> std::io::Result<Output> {
        let mut command = Self::command(launch);
        let mut child = command.spawn()?;
        let out_stream = child.stdout.take();
        let err_stream = child.stderr.take();
        let mut input = child.stdin.take();
        let limit = launch.limit;

        std::thread::scope(|scope| {
            let out = out_stream.map(|stream| scope.spawn(move || bounded(stream, limit)));
            let err = err_stream.map(|stream| scope.spawn(move || bounded(stream, limit)));
            // Written after the readers start, so a child that answers while it
            // is still being fed cannot fill a pipe and stall both sides.
            if let Some(bytes) = &launch.stdin
                && let Some(mut stdin) = input.take()
            {
                stdin.write_all(bytes)?;
            }
            drop(input);

            let status = match launch.timeout {
                None => child.wait()?,
                Some(limit) => {
                    let started = Instant::now();
                    loop {
                        if let Some(status) = child.try_wait()? {
                            break status;
                        }
                        if started.elapsed() >= limit {
                            child.kill()?;
                            break child.wait()?;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            };
            if launch.own_group {
                // Close the pipes a descendant still holds, so the readers end.
                // Safety: a kill on a process group this call created.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            }

            let (stdout, stdout_complete) = match out {
                Some(handle) => handle.join().expect("stdout reader")?,
                None => (Vec::new(), true),
            };
            let (stderr, stderr_complete) = match err {
                Some(handle) => handle.join().expect("stderr reader")?,
                None => (Vec::new(), true),
            };
            Ok(Output { status, stdout, stderr, stdout_complete, stderr_complete })
        })
    }

    fn start(&mut self, launch: &Launch) -> std::io::Result<Box<dyn Child>> {
        Ok(Box::new(SystemChild(Self::command(launch).spawn()?)))
    }
}

/// A real child of this process.
struct SystemChild(std::process::Child);

impl Child for SystemChild {
    fn stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.0.stdout.take().map(|stream| Box::new(stream) as Box<dyn Read + Send>)
    }

    fn stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        self.0.stderr.take().map(|stream| Box::new(stream) as Box<dyn Read + Send>)
    }

    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.0.wait()
    }

    fn kill(&mut self) -> std::io::Result<()> {
        self.0.kill()
    }
}

/// Read a stream whole, keeping at most `limit` bytes of it.
fn bounded(mut stream: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut complete = true;
    let mut chunk = [0; 8192];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok((bytes, complete));
        }
        let keep = read.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[..keep]);
        complete &= keep == read;
    }
}

/// The fake: scripted outputs in, the launches it was given out.
#[derive(Debug, Default)]
pub struct Recorded {
    scripted: std::collections::VecDeque<std::io::Result<Output>>,
    launches: Vec<Launch>,
}

impl Recorded {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an exit with code 0 and this standard output.
    pub fn out(self, stdout: impl Into<Vec<u8>>) -> Self {
        self.answer(Output::exited(0, stdout, ""))
    }

    /// Queue an exit with this code and this standard error.
    pub fn fail(self, code: i32, stderr: impl Into<Vec<u8>>) -> Self {
        self.answer(Output::exited(code, "", stderr))
    }

    /// Queue an output of the caller's own making.
    pub fn answer(mut self, output: Output) -> Self {
        self.scripted.push_back(Ok(output));
        self
    }

    /// Queue a failure to start the program at all.
    pub fn unavailable(mut self, error: std::io::Error) -> Self {
        self.scripted.push_back(Err(error));
        self
    }

    /// Every launch this fake was given, in order.
    pub fn launches(&self) -> &[Launch] {
        &self.launches
    }

    /// The one launch this fake was given; panics when it was given any other
    /// number, which is the assertion a check usually wants.
    pub fn launch(&self) -> &Launch {
        assert_eq!(self.launches.len(), 1, "expected one launch: {:?}", self.launches);
        &self.launches[0]
    }

    /// The arguments of each launch, which is what most checks read.
    pub fn arguments(&self) -> Vec<Vec<String>> {
        self.launches
            .iter()
            .map(|launch| {
                launch.args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
            })
            .collect()
    }
}

impl Process for Recorded {
    fn run(&mut self, launch: &Launch) -> std::io::Result<Output> {
        self.launches.push(launch.clone());
        self.scripted
            .pop_front()
            .unwrap_or_else(|| panic!("no scripted answer for {:?} {:?}", launch.program, launch.args))
    }
}

#[cfg(test)]
mod tests;
