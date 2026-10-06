//! HEAD's copy of the project file, read through git (design 0003, CFG-R3;
//! EVD-R17, file half). The project's settings come from this copy, so an
//! edit nobody has committed cannot change the policy agents run under.
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

use baley_core::policy::{Fault, SettingsFile, Unavailable};

use crate::git_process::{self, Caller};
use crate::guard_budget::Budget;
use crate::process::{Launch, Output, Process};
use crate::settings;

/// HEAD's copy of the project file, and what the owner should know about the
/// working-tree file beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// The project layer: HEAD's bytes at the working-tree path, or `None`
    /// when HEAD is unborn or holds no copy.
    pub layer: Option<SettingsFile>,
    /// Set when the working-tree file holds settings not yet in force. A note
    /// about the file, never a policy diagnostic.
    pub pending: Option<Pending>,
}

/// The working-tree file holds settings that apply only once committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    /// HEAD holds no copy of the file.
    Absent {
        /// The working-tree file.
        path: PathBuf,
    },
    /// The working-tree file's bytes differ from HEAD's copy.
    Differs {
        /// The working-tree file.
        path: PathBuf,
    },
}

impl fmt::Display for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent { path } => write!(
                f,
                "{} is not committed at HEAD, so its settings apply once committed",
                path.display()
            ),
            Self::Differs { path } => write!(
                f,
                "{} differs from HEAD's copy, so its changes apply once committed",
                path.display()
            ),
        }
    }
}

/// Reads HEAD's copy of `working`, the project file as `settings::read`
/// returned it, from the repository at `root`. An unborn HEAD or a file absent
/// at HEAD is an empty layer; any other failure is `config-unavailable`
/// naming the working-tree file. The policy step's gatherer is its caller.
pub fn read(
    root: &Path,
    working: &SettingsFile,
    process: &mut dyn Process,
) -> Result<Committed, Unavailable> {
    sequence(root, working, &mut |args| {
        let launch = git(git_process::launch(Caller::ProjectHead), root, args);
        let ran = git_process::run(&launch, process);
        Ok((launch, ran))
    })
}

/// Bytes kept of each stream of a guard launch. A project file larger than
/// this is refused as incomplete output.
const GUARD_OUTPUT_LIMIT: usize = 1 << 20;

/// Bytes kept of git's first stderr line in a guard read's error, so the
/// error cannot carry a flood of output or any later line.
const GUARD_EXCERPT_LIMIT: usize = 256;

/// Reads HEAD's copy of `working` as [`read`] does, on the guard's budget.
///
/// Each of its up to three launches runs under the guard's own caller on the
/// time `budget` grants it and is charged for the time it took, so together
/// they share git's one allowance with the branch lookup. With no time left
/// nothing launches and the read is refused. Each stream keeps at most
/// `GUARD_OUTPUT_LIMIT` bytes (1 MiB), and an error keeps at most
/// `GUARD_EXCERPT_LIMIT` bytes (256) of git's first stderr line.
///
/// It has no caller yet: Build 3's recording guard reads it after the branch
/// lookup, on the same budget.
pub fn read_for_guard(
    root: &Path,
    working: &SettingsFile,
    process: &mut dyn Process,
    budget: &mut Budget,
) -> Result<Committed, Unavailable> {
    sequence(root, working, &mut |args| {
        let grant = budget
            .git()
            .ok_or_else(|| "the guard's git time is spent".to_owned())?;
        let launch = git(
            git_process::guard_launch(Caller::GuardProjectHead, &grant),
            root,
            args,
        )
        .limit(GUARD_OUTPUT_LIMIT);
        let ran = git_process::run(&launch, process).map(first_line_excerpt);
        budget.charge_git(grant);
        Ok((launch, ran))
    })
}

/// Starts one git launch of the sequence with these arguments and runs it,
/// or says why it could not start one.
type Step<'a> = dyn FnMut(&[&OsStr]) -> Result<(Launch, Ran), String> + 'a;

/// `rev-parse`, then `ls-tree`, then `cat-file`, each judged before the next
/// runs.
fn sequence(
    root: &Path,
    working: &SettingsFile,
    run: &mut Step<'_>,
) -> Result<Committed, Unavailable> {
    let refuse = |cause: String| Unavailable {
        path: working.path.clone(),
        fault: Fault::Unreadable {
            cause: format!("HEAD's copy: {cause}"),
        },
    };
    let relative = working.path.strip_prefix(root).map_err(|_| {
        refuse(format!(
            "the file is not inside the repository at {}",
            root.display()
        ))
    })?;

    let (head, ran) =
        run(&["rev-parse", "--verify", "-q", "HEAD"].map(OsStr::new)).map_err(refuse)?;
    if !born(&head, ran).map_err(refuse)? {
        return Ok(Committed {
            layer: None,
            pending: None,
        });
    }

    let (tree, ran) = run(&[
        OsStr::new("ls-tree"),
        OsStr::new("HEAD"),
        OsStr::new("--"),
        relative.as_os_str(),
    ])
    .map_err(refuse)?;
    let Some(oid) = entry(&tree, ran).map_err(refuse)? else {
        return Ok(Committed {
            layer: None,
            pending: Some(Pending::Absent {
                path: working.path.clone(),
            }),
        });
    };

    let (cat, ran) =
        run(&[OsStr::new("cat-file"), OsStr::new("blob"), OsStr::new(&oid)]).map_err(refuse)?;
    let bytes = blob(&cat, ran).map_err(refuse)?;
    // The digest is of the bytes, never the object id, so both file layers
    // carry one digest kind.
    let layer = settings::file(&working.path, bytes);
    Ok(Committed {
        pending: differs(working, &layer),
        layer: Some(layer),
    })
}

/// Keeps only the start of git's first stderr line, cut on a character
/// boundary, for a guard read's error text.
fn first_line_excerpt(mut output: Output) -> Output {
    let first = output.stderr.split(|&b| b == b'\n').next().unwrap_or(&[]);
    let text = String::from_utf8_lossy(first);
    let end = text.floor_char_boundary(GUARD_EXCERPT_LIMIT);
    output.stderr = text.as_bytes()[..end].to_vec();
    output
}

/// The note for a working-tree file whose bytes are not HEAD's. Both digests
/// are SHA-256 of the bytes, so comparing them compares the bytes.
fn differs(working: &SettingsFile, head: &SettingsFile) -> Option<Pending> {
    (working.digest != head.digest).then(|| Pending::Differs {
        path: working.path.clone(),
    })
}

/// One git launch at the root, read-only: no index lock, no lazy fetch from a
/// promisor remote, a path that matches only itself, and no prompt.
fn git(launch: Launch, root: &Path, args: &[&OsStr]) -> Launch {
    launch
        .cwd(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
}

pub(crate) type Ran = Result<Output, git_process::Error>;

/// `rev-parse --verify -q HEAD`: exit 0 is a commit, exit 1 an unborn HEAD.
pub(crate) fn born(launch: &Launch, ran: Ran) -> Result<bool, String> {
    let output = finished(launch, ran)?;
    match output.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(failed(launch, &output)),
    }
}

/// `ls-tree HEAD -- <path>`: no output is a file absent at HEAD; one regular
/// blob entry gives its object id; anything else refuses.
fn entry(launch: &Launch, ran: Ran) -> Result<Option<String>, String> {
    let output = finished(launch, ran)?;
    if !output.success() {
        return Err(failed(launch, &output));
    }
    if !output.stdout_complete {
        return Err(format!("{} gave incomplete output", command(launch)));
    }
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let unreadable = || format!("{} gave output that is not one entry", command(launch));
    let line = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
    if line.contains(&b'\n') {
        return Err(unreadable());
    }
    // `<mode> <type> <oid>` TAB `<path>`; the path is the one asked for.
    let tab = line
        .iter()
        .position(|&b| b == b'\t')
        .ok_or_else(unreadable)?;
    let fields = std::str::from_utf8(&line[..tab]).map_err(|_| unreadable())?;
    let [mode, kind, oid] = fields.split(' ').collect::<Vec<_>>()[..] else {
        return Err(unreadable());
    };
    let octal = mode.len() == 6 && mode.bytes().all(|b| (b'0'..=b'7').contains(&b));
    let named = !kind.is_empty() && kind.bytes().all(|b| b.is_ascii_lowercase());
    let hex = matches!(oid.len(), 40 | 64)
        && oid
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !(octal && named && hex) {
        return Err(unreadable());
    }
    // Mode 120000 is a link and its blob is the link's target text.
    if kind != "blob" || !mode.starts_with("100") {
        return Err(format!(
            "HEAD holds a {kind} with mode {mode} there, not a regular file"
        ));
    }
    Ok(Some(oid.into()))
}

/// `cat-file blob <oid>`: the exact bytes.
fn blob(launch: &Launch, ran: Ran) -> Result<Vec<u8>, String> {
    let output = finished(launch, ran)?;
    if !output.success() {
        return Err(failed(launch, &output));
    }
    if !output.stdout_complete {
        return Err(format!("{} gave incomplete output", command(launch)));
    }
    Ok(output.stdout)
}

/// A git that could not start or ran out of time.
pub(crate) fn finished(launch: &Launch, ran: Ran) -> Result<Output, String> {
    ran.map_err(|error| match error {
        git_process::Error::Limit(limit) => limit.to_string(),
        git_process::Error::Io(error) => format!("{} could not run: {error}", command(launch)),
    })
}

/// A git that exited with an unexpected code or died by a signal.
pub(crate) fn failed(launch: &Launch, output: &Output) -> String {
    let Some(code) = output.code() else {
        let signal = output.signal().unwrap_or_default();
        return format!("{} was killed by signal {signal}", command(launch));
    };
    let first = output.stderr.split(|&b| b == b'\n').next().unwrap_or(&[]);
    // Control characters from the repository never reach the owner's terminal.
    let stderr: String = String::from_utf8_lossy(first)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if stderr.is_empty() {
        format!("{} exited with code {code}", command(launch))
    } else {
        format!("{} exited with code {code}: {stderr}", command(launch))
    }
}

pub(crate) fn command(launch: &Launch) -> String {
    format!("git {}", launch.argument_text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Recorded;

    const ROOT: &str = "/r";
    const HEAD: &str = "5f3a1c0e9b7d2a4f6e8c1b3d5a7f9e2c4b6d8a0f\n";
    const OID: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    fn working(path: &str) -> SettingsFile {
        settings::file(Path::new(path), b"[project]\nname = \"w\"\n".to_vec())
    }

    fn read_at(path: &str, fake: &mut Recorded) -> Result<Committed, Unavailable> {
        let result = read(Path::new(ROOT), &working(path), fake);
        for launch in fake.launches() {
            assert_eq!(launch.git_caller(), Some(Caller::ProjectHead), "{launch:?}");
            assert_eq!(launch.cwd.as_deref(), Some(Path::new(ROOT)), "{launch:?}");
        }
        result
    }

    fn cause(refusal: &Unavailable) -> &str {
        assert_eq!(refusal.path, Path::new("/r/baley.toml"));
        let Fault::Unreadable { cause } = &refusal.fault else {
            panic!("expected an unreadable fault: {refusal:?}");
        };
        cause
    }

    #[test]
    fn an_unborn_head_is_an_empty_layer_and_ls_tree_never_runs() {
        let mut fake = Recorded::new().fail(1, "");
        let committed = read_at("/r/baley.toml", &mut fake).unwrap();
        assert_eq!(
            committed,
            Committed {
                layer: None,
                pending: None,
            }
        );
        assert_eq!(fake.arguments(), [["rev-parse", "--verify", "-q", "HEAD"]]);
    }

    #[test]
    fn a_rev_parse_failure_is_not_read_as_an_unborn_head() {
        let mut fake = Recorded::new().fail(
            128,
            "fatal: not a git repository (or any of the parent directories): .git\n",
        );
        let refusal = read_at("/r/baley.toml", &mut fake).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "config-unavailable: cannot read /r/baley.toml: HEAD's copy: \
             git rev-parse --verify -q HEAD exited with code 128: \
             fatal: not a git repository (or any of the parent directories): .git"
        );
        assert_eq!(fake.launches().len(), 1);
    }

    #[test]
    fn a_file_absent_at_head_is_an_empty_layer_with_its_note() {
        let mut fake = Recorded::new().out(HEAD).out("");
        let committed = read_at("/r/baley.toml", &mut fake).unwrap();
        assert_eq!(committed.layer, None);
        let pending = committed.pending.unwrap();
        assert_eq!(
            pending,
            Pending::Absent {
                path: "/r/baley.toml".into()
            }
        );
        assert_eq!(
            pending.to_string(),
            "/r/baley.toml is not committed at HEAD, so its settings apply once committed"
        );
        assert_eq!(fake.arguments()[1], ["ls-tree", "HEAD", "--", "baley.toml"]);
    }

    #[test]
    fn an_ls_tree_failure_is_not_taken_as_an_empty_layer() {
        let mut fake = Recorded::new()
            .out(HEAD)
            .fail(128, "fatal: unable to read tree 5f3a1c0e\n");
        let refusal = read_at("/r/baley.toml", &mut fake).unwrap_err();
        assert_eq!(
            cause(&refusal),
            "HEAD's copy: git ls-tree HEAD -- baley.toml exited with code 128: \
             fatal: unable to read tree 5f3a1c0e"
        );
    }

    #[test]
    fn a_folder_a_link_or_a_submodule_at_head_is_refused_without_reading_it() {
        for (line, kind, mode) in [
            ("040000 tree", "tree", "040000"),
            ("120000 blob", "blob", "120000"),
            ("160000 commit", "commit", "160000"),
        ] {
            let mut fake = Recorded::new()
                .out(HEAD)
                .out(format!("{line} {OID}\tbaley.toml\n"));
            let refusal = read_at("/r/baley.toml", &mut fake).unwrap_err();
            assert_eq!(
                cause(&refusal),
                format!(
                    "HEAD's copy: HEAD holds a {kind} with mode {mode} there, not a regular file"
                )
            );
            assert_eq!(fake.launches().len(), 2, "{line}");
        }
    }

    #[test]
    fn an_ls_tree_answer_that_is_not_one_entry_is_refused() {
        for stdout in [
            format!("100644 blob {OID}\tbaley.toml\n100644 blob {OID}\tbaley.toml\n"),
            format!("100644 blob {}\tbaley.toml\n", &OID[..39]),
            "\n".into(),
        ] {
            let mut fake = Recorded::new().out(HEAD).out(stdout.clone());
            let refusal = read_at("/r/baley.toml", &mut fake).unwrap_err();
            assert_eq!(
                cause(&refusal),
                "HEAD's copy: git ls-tree HEAD -- baley.toml gave output that is not one entry",
                "{stdout:?}"
            );
            assert_eq!(fake.launches().len(), 2, "{stdout:?}");
        }
    }

    #[test]
    fn a_regular_blob_is_the_layer_by_its_bytes_not_its_object_id() {
        for mode in ["100644", "100755"] {
            let mut fake = Recorded::new()
                .out(HEAD)
                .out(format!("{mode} blob {OID}\tbaley.toml\n"))
                .out("abc");
            let committed = read_at("/r/baley.toml", &mut fake).unwrap();
            assert_eq!(
                committed.layer,
                Some(SettingsFile {
                    path: "/r/baley.toml".into(),
                    bytes: b"abc".to_vec(),
                    // Fixed from `sha256sum`, not from this code.
                    digest: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                        .into(),
                })
            );
            assert_eq!(fake.arguments()[2], ["cat-file", "blob", OID]);
        }
    }

    #[test]
    fn a_cat_file_killed_by_a_signal_is_refused_not_read_as_empty() {
        let mut fake = Recorded::new()
            .out(HEAD)
            .out(format!("100644 blob {OID}\tbaley.toml\n"))
            .answer(Output::signaled(9));
        let refusal = read_at("/r/baley.toml", &mut fake).unwrap_err();
        assert_eq!(
            cause(&refusal),
            format!("HEAD's copy: git cat-file blob {OID} was killed by signal 9")
        );
    }

    #[test]
    fn a_git_that_cannot_start_is_refused_not_read_as_unborn() {
        let mut fake = Recorded::new().unavailable(std::io::Error::from_raw_os_error(2));
        let refusal = read_at("/r/baley.toml", &mut fake).unwrap_err();
        assert_eq!(
            cause(&refusal),
            "HEAD's copy: git rev-parse --verify -q HEAD could not run: \
             No such file or directory (os error 2)"
        );
    }

    #[test]
    fn a_file_in_a_subfolder_is_named_to_ls_tree_from_the_root_not_by_its_name() {
        let mut fake = Recorded::new().out(HEAD).out("");
        let committed = read_at("/r/sub/baley.toml", &mut fake).unwrap();
        assert_eq!(committed.layer, None);
        assert_eq!(
            fake.arguments()[1],
            ["ls-tree", "HEAD", "--", "sub/baley.toml"]
        );
    }

    #[test]
    fn differing_bytes_give_the_note_that_changes_apply_once_committed() {
        let working = settings::file(Path::new("/r/baley.toml"), b"a".to_vec());
        let head = settings::file(Path::new("/r/baley.toml"), b"b".to_vec());
        let pending = differs(&working, &head).unwrap();
        assert_eq!(
            pending.to_string(),
            "/r/baley.toml differs from HEAD's copy, so its changes apply once committed"
        );
    }

    #[test]
    fn identical_bytes_give_no_note_rather_than_one_always_shown() {
        let working = settings::file(Path::new("/r/baley.toml"), b"a".to_vec());
        let head = settings::file(Path::new("/r/baley.toml"), b"a".to_vec());
        assert_eq!(differs(&working, &head), None);
    }

    #[test]
    fn the_reader_compares_heads_bytes_with_the_working_tree_files() {
        let differing = b"abc".to_vec();
        let equal = working("/r/baley.toml").bytes;
        for (blob, pending) in [
            (
                differing,
                Some(Pending::Differs {
                    path: "/r/baley.toml".into(),
                }),
            ),
            (equal, None),
        ] {
            let mut fake = Recorded::new()
                .out(HEAD)
                .out(format!("100644 blob {OID}\tbaley.toml\n"))
                .out(blob);
            let committed = read_at("/r/baley.toml", &mut fake).unwrap();
            assert_eq!(committed.pending, pending);
        }
    }

    /// The guard tests' clock: each launch the fake runs takes `step`.
    struct Ticking {
        recorded: Recorded,
        now: std::rc::Rc<std::cell::Cell<std::time::Duration>>,
        step: std::time::Duration,
    }

    impl Process for Ticking {
        fn run(&mut self, launch: &Launch) -> std::io::Result<Output> {
            self.now.set(self.now.get() + self.step);
            self.recorded.run(launch)
        }
    }

    fn secs(seconds: u64) -> std::time::Duration {
        std::time::Duration::from_secs(seconds)
    }

    /// A budget whose clock stands still at `at`.
    fn budget_at(at: std::time::Duration) -> Budget {
        Budget::with_clock(move || at)
    }

    fn guard_read(fake: &mut dyn Process, budget: &mut Budget) -> Result<Committed, Unavailable> {
        read_for_guard(Path::new(ROOT), &working("/r/baley.toml"), fake, budget)
    }

    #[test]
    fn a_guard_read_launches_as_the_guard_on_one_shrinking_allowance_never_as_project_head() {
        let now = std::rc::Rc::new(std::cell::Cell::new(secs(0)));
        let clock = std::rc::Rc::clone(&now);
        // The branch lookup before it used nothing.
        let mut budget = Budget::with_clock(move || clock.get());
        let mut fake = Ticking {
            recorded: Recorded::new()
                .out(HEAD)
                .out(format!("100644 blob {OID}\tbaley.toml\n"))
                .out("abc"),
            now,
            step: secs(2),
        };

        guard_read(&mut fake, &mut budget).unwrap();

        let launches = fake.recorded.launches();
        let timeouts: Vec<_> = launches.iter().map(|launch| launch.timeout).collect();
        assert_eq!(timeouts, [Some(secs(5)), Some(secs(3)), Some(secs(1))]);
        for launch in launches {
            assert_eq!(
                launch.git_caller(),
                Some(Caller::GuardProjectHead),
                "{launch:?}"
            );
            assert!(launch.own_group, "{launch:?}");
            assert_eq!(launch.cwd.as_deref(), Some(Path::new(ROOT)), "{launch:?}");
            assert_eq!(launch.limit, 1 << 20, "{launch:?}");
            assert_eq!(
                launch.env,
                [
                    ("GIT_OPTIONAL_LOCKS".to_owned(), Some("0".into())),
                    ("GIT_NO_LAZY_FETCH".to_owned(), Some("1".into())),
                    ("GIT_LITERAL_PATHSPECS".to_owned(), Some("1".into())),
                    ("GIT_TERMINAL_PROMPT".to_owned(), Some("0".into())),
                ]
            );
        }
    }

    #[test]
    fn spent_git_time_refuses_the_guard_read_rather_than_launch() {
        let now = std::rc::Rc::new(std::cell::Cell::new(secs(0)));
        let clock = std::rc::Rc::clone(&now);
        let mut budget = Budget::with_clock(move || clock.get());
        let lookup = budget.git().expect("a grant at the start");
        now.set(secs(5));
        budget.charge_git(lookup);
        // No scripted answer: a launch would panic.
        let mut fake = Recorded::new();

        let refusal = guard_read(&mut fake, &mut budget).unwrap_err();

        assert_eq!(
            cause(&refusal),
            "HEAD's copy: the guard's git time is spent"
        );
        assert!(fake.launches().is_empty());
    }

    #[test]
    fn a_guard_read_error_keeps_only_a_bounded_start_of_gits_first_stderr_line() {
        let prefix = format!("HEAD's copy: git cat-file blob {OID} exited with code 128: ");
        let flood = format!("{}\ngit commit -m SENTINEL\n", "x".repeat(1 << 20));
        // A three-byte character straddles byte 256, so the cut falls before it.
        let wide = "€".repeat(100);
        for (stderr, excerpt) in [(flood, "x".repeat(256)), (wide, "€".repeat(85))] {
            let mut fake = Recorded::new()
                .out(HEAD)
                .out(format!("100644 blob {OID}\tbaley.toml\n"))
                .fail(128, stderr);

            let refusal = guard_read(&mut fake, &mut budget_at(secs(0))).unwrap_err();

            let cause = cause(&refusal);
            assert!(!cause.contains("SENTINEL"));
            assert_eq!(cause.strip_prefix(prefix.as_str()), Some(excerpt.as_str()));
        }
    }

    #[test]
    fn a_guard_read_timeout_states_the_time_it_was_granted() {
        // 5.6 s in, the work time left is 2.4 s, under git's 5 s.
        let mut fake = Recorded::new().unavailable(std::io::ErrorKind::TimedOut.into());

        let refusal = guard_read(
            &mut fake,
            &mut budget_at(std::time::Duration::from_millis(5_600)),
        )
        .unwrap_err();

        assert_eq!(
            cause(&refusal),
            "HEAD's copy: git rev-parse --verify -q HEAD exceeded git deadline of 2.4 seconds"
        );
    }

    #[test]
    fn a_guard_read_takes_an_unborn_head_as_empty_and_a_regular_blob_as_the_layer() {
        let mut unborn = Recorded::new().fail(1, "");
        assert_eq!(
            guard_read(&mut unborn, &mut budget_at(secs(0))).unwrap(),
            Committed {
                layer: None,
                pending: None,
            }
        );

        let mut regular = Recorded::new()
            .out(HEAD)
            .out(format!("100644 blob {OID}\tbaley.toml\n"))
            .out("abc");
        let committed = guard_read(&mut regular, &mut budget_at(secs(0))).unwrap();
        assert_eq!(
            committed.layer.map(|layer| layer.bytes),
            Some(b"abc".to_vec())
        );
    }
}
