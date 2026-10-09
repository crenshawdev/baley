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
//! `recall::history` and `committed` add `GIT_NO_LAZY_FETCH` that no other
//! git call sets.
//! Fixing one environment here would change behavior at those sites.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// What to run, and how to hold it while it runs.
#[derive(Clone, PartialEq, Eq)]
pub struct Launch {
    origin: Origin,
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
    /// The timeout a guard budget grant gave this launch. Only `granted_git`
    /// sets it, so the validator can tell a granted time from one set by hand.
    granted: Option<Duration>,
    /// Put the child in its own process group, and kill that group once the
    /// child is reaped, so a descendant cannot hold the pipes open.
    pub own_group: bool,
    /// Kill the child when the process that started it dies.
    pub die_with_parent: bool,
    /// Let the child use this process's own streams, except outputs sent to
    /// null. Nothing is captured, so the output carries no bytes.
    pub inherit: bool,
    /// The child reads this process's stdin without changing its output streams.
    /// Independent of `inherit`; when set, `stdin` bytes are not written.
    pub inherit_stdin: bool,
    /// Send stdout and stderr to the null device, even when `inherit` is set.
    pub null_output: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Origin {
    Baley,
    Git(crate::git_process::Registration),
    Owner,
}

impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let env: Vec<_> = self
            .env
            .iter()
            .map(|(name, value)| {
                (
                    name,
                    if value.is_some() {
                        "[redacted]"
                    } else {
                        "unset"
                    },
                )
            })
            .collect();
        f.debug_struct("Launch")
            .field("origin", &self.origin)
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("stdin", &self.stdin)
            .field("env", &env)
            .field("limit", &self.limit)
            .field("timeout", &self.timeout)
            .field("granted", &self.granted)
            .field("own_group", &self.own_group)
            .field("die_with_parent", &self.die_with_parent)
            .field("inherit", &self.inherit)
            .field("inherit_stdin", &self.inherit_stdin)
            .field("null_output", &self.null_output)
            .finish()
    }
}

impl Launch {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            origin: Origin::Baley,
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            cwd: None,
            stdin: None,
            env: Vec::new(),
            limit: usize::MAX,
            timeout: None,
            granted: None,
            own_group: false,
            die_with_parent: false,
            inherit: false,
            inherit_stdin: false,
            null_output: false,
        }
    }

    pub fn cwd(mut self, cwd: impl AsRef<Path>) -> Self {
        self.cwd = Some(cwd.as_ref().to_owned());
        self
    }

    pub(crate) fn registered_git(registration: crate::git_process::Registration) -> Self {
        let mut launch = Self::new("git");
        launch.origin = Origin::Git(registration);
        launch
    }

    /// A guard caller's git launch on the time `grant` gave it. The grant is
    /// kept beside `timeout`, so a time changed by hand afterwards no longer
    /// matches it and the validator refuses the launch.
    pub(crate) fn granted_git(
        registration: crate::git_process::Registration,
        grant: &crate::guard_budget::GitGrant,
    ) -> Self {
        let mut launch = Self::registered_git(registration).timeout(grant.timeout());
        launch.granted = Some(grant.timeout());
        launch
    }

    /// A command the owner chose through `baley exec`, built only by `exec::launch_for`.
    /// Baley's own programs never use this constructor.
    pub(crate) fn owner_command(program: impl AsRef<OsStr>) -> Self {
        Self {
            origin: Origin::Owner,
            inherit_stdin: true,
            ..Self::new(program)
        }
    }

    pub(crate) fn git_caller(&self) -> Option<crate::git_process::Caller> {
        match &self.origin {
            Origin::Git(registration) => Some(registration.caller()),
            _ => None,
        }
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
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    pub fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.to_owned(), Some(value.as_ref().to_owned())));
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

    /// Discard both output streams so a detached child has no unread pipes.
    pub fn null_output(mut self) -> Self {
        self.null_output = true;
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

/// An immutable launch that passed the process construction gate.
#[derive(Debug)]
pub struct ValidatedLaunch<'a>(&'a Launch);

impl ValidatedLaunch<'_> {
    pub fn descriptor(&self) -> &Launch {
        self.0
    }
}

/// Validate borrowed launch material before a Command or recorded observation
/// can be obtained. The borrow prevents mutation until the consumer is done.
pub fn validate_launch(launch: &Launch) -> std::io::Result<ValidatedLaunch<'_>> {
    if launch.origin == Origin::Owner {
        return Ok(ValidatedLaunch(launch));
    }
    let is_git = Path::new(&launch.program).file_name() == Some(OsStr::new("git"));
    match (is_git, launch.git_caller()) {
        (true, Some(caller)) => {
            use crate::git_process::Deadline;
            let timed = match crate::git_process::deadline(caller) {
                Deadline::Exact(deadline) => launch.timeout == Some(deadline),
                // Only the time a grant gave counts, so a timeout set by hand
                // cannot skip the budget. A zero timeout would still start
                // git, only to kill it.
                Deadline::Guard(cap) => launch.granted.is_some_and(|granted| {
                    launch.timeout == Some(granted) && !granted.is_zero() && granted <= cap
                }),
            };
            if !timed || !launch.own_group {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "registered git launch requires its caller deadline and owned process group",
                ));
            }
        }
        (true, None) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "git launch requires a registered caller",
            ));
        }
        (false, Some(_)) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "registered git launch cannot change executable identity",
            ));
        }
        (false, None) => {}
    }
    Ok(ValidatedLaunch(launch))
}

/// The source or destination of one child stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// Use this process's stream.
    Inherit,
    /// Connect a pipe to Baley.
    Piped,
    /// Connect the null device.
    Null,
}
impl Stream {
    fn stdio(self) -> Stdio {
        match self {
            Self::Inherit => Stdio::inherit(),
            Self::Piped => Stdio::piped(),
            Self::Null => Stdio::null(),
        }
    }
}

/// The three streams requested for a launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StdioPlan {
    /// The child's input source.
    pub stdin: Stream,
    /// The child's standard output destination.
    pub stdout: Stream,
    /// The child's standard error destination.
    pub stderr: Stream,
}

/// Selects streams without constructing an operating-system command.
pub fn stdio_plan(launch: &Launch) -> StdioPlan {
    let output = if launch.null_output {
        Stream::Null
    } else if launch.inherit {
        Stream::Inherit
    } else {
        Stream::Piped
    };
    StdioPlan {
        stdin: if launch.inherit || launch.inherit_stdin {
            Stream::Inherit
        } else if launch.stdin.is_some() {
            Stream::Piped
        } else {
            Stream::Null
        },
        stdout: output,
        stderr: output,
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

    /// Start a child and hand it back while it runs, for callers that
    /// read a stream as it arrives rather than after the fact. Only `System`
    /// offers this; a fake refuses it unless it scripts children of its own.
    fn start(&mut self, launch: &Launch) -> std::io::Result<Box<dyn Child>> {
        let _ = launch;
        Err(std::io::Error::other(
            "this process does not start children",
        ))
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
    fn command(validated: &ValidatedLaunch<'_>) -> Command {
        let launch = validated.descriptor();
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
        let plan = stdio_plan(launch);
        command
            .stdin(plan.stdin.stdio())
            .stdout(plan.stdout.stdio())
            .stderr(plan.stderr.stdio());
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
        let mut child = SystemChild::spawn(launch)?;
        let out_stream = child.stdout();
        let err_stream = child.stderr();
        let limit = launch.limit;
        std::thread::scope(|scope| {
            let out = out_stream.map(|stream| scope.spawn(move || bounded(stream, limit)));
            let err = err_stream.map(|stream| scope.spawn(move || bounded(stream, limit)));
            let status = child.wait();
            // Always join the drains after cleanup, including on timeout.
            let stdout = out
                .map(|handle| handle.join().expect("stdout reader"))
                .transpose();
            let stderr = err
                .map(|handle| handle.join().expect("stderr reader"))
                .transpose();
            let status = status?;
            let (stdout, stdout_complete) = stdout?.unwrap_or((Vec::new(), true));
            let (stderr, stderr_complete) = stderr?.unwrap_or((Vec::new(), true));
            Ok(Output {
                status,
                stdout,
                stderr,
                stdout_complete,
                stderr_complete,
            })
        })
    }

    fn start(&mut self, launch: &Launch) -> std::io::Result<Box<dyn Child>> {
        Ok(Box::new(SystemChild::spawn(launch)?))
    }
}

/// Signaling and waiting remain observations gathered by SystemChild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadlineAction {
    Wait,
    KillAndReap,
}

pub fn deadline_action(timeout: Option<Duration>, elapsed: Duration) -> DeadlineAction {
    if timeout.is_some_and(|limit| elapsed >= limit) {
        DeadlineAction::KillAndReap
    } else {
        DeadlineAction::Wait
    }
}

/// A real child of this process, including its stdin worker and deadline.
struct SystemChild {
    child: std::process::Child,
    started: Instant,
    timeout: Option<Duration>,
    own_group: bool,
    input: Option<std::thread::JoinHandle<std::io::Result<()>>>,
}

impl SystemChild {
    fn spawn(launch: &Launch) -> std::io::Result<Self> {
        let validated = validate_launch(launch)?;
        let launch = validated.descriptor();
        let mut command = System::command(&validated);
        // Includes spawn and all stdin delivery, not just the wait.
        let started = Instant::now();
        let mut child = command.spawn()?;
        let input = child
            .stdin
            .take()
            .zip(launch.stdin.clone())
            .map(|(mut stdin, bytes)| std::thread::spawn(move || stdin.write_all(&bytes)));
        Ok(Self {
            child,
            started,
            timeout: launch.timeout,
            own_group: launch.own_group,
            input,
        })
    }

    fn kill_group(&self) {
        if self.own_group {
            // Safety: this group was created for this owned child.
            unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL) };
        }
    }

    fn observe_exit(&mut self) -> std::io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if deadline_action(self.timeout, self.started.elapsed()) == DeadlineAction::KillAndReap
            {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            if self.timeout.is_none() {
                return self.child.wait();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Child for SystemChild {
    fn stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.child
            .stdout
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Read + Send>)
    }

    fn stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        self.child
            .stderr
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Read + Send>)
    }

    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.observe_exit();
        // Kill the group before joining any pipe worker. A descendant may
        // retain either end even after the immediate child has exited.
        self.kill_group();
        if status.is_err() {
            let _ = self.child.kill();
            self.child.wait()?;
        }
        let input = self
            .input
            .take()
            .map(|handle| handle.join().expect("stdin writer"))
            .transpose();
        // The timeout observation takes precedence over cleanup's broken pipe.
        let status = status?;
        input?;
        Ok(status)
    }

    fn kill(&mut self) -> std::io::Result<()> {
        self.kill_group();
        self.child.kill()
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
        assert_eq!(
            self.launches.len(),
            1,
            "expected one launch: {:?}",
            self.launches
        );
        &self.launches[0]
    }

    /// The arguments of each launch, which is what most checks read.
    pub fn arguments(&self) -> Vec<Vec<String>> {
        self.launches
            .iter()
            .map(|launch| {
                launch
                    .args
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect()
            })
            .collect()
    }
}

impl Process for Recorded {
    fn run(&mut self, launch: &Launch) -> std::io::Result<Output> {
        let validated = validate_launch(launch)?;
        let launch = validated.descriptor();
        self.launches.push(launch.clone());
        self.scripted.pop_front().unwrap_or_else(|| {
            panic!(
                "no scripted answer for {:?} {:?}",
                launch.program, launch.args
            )
        })
    }
}

#[cfg(test)]
mod tests;
