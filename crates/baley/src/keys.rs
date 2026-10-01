//! Provider keys read from the owner's file, with pure exposure and grammar
//! checks, and the model-list request that carries a key in its header.
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use baley_core::catalog::Provider;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::{Method, Request, Url};

/// The file exposes its keys to another user.
pub const KEYS_FILE_EXPOSED: &str = "keys-file-exposed";
/// The file holds one or more invalid lines.
pub const KEYS_FILE_INVALID: &str = "keys-file-invalid";
/// The path is not a regular file or could not be read.
pub const KEYS_FILE_UNREADABLE: &str = "keys-file-unreadable";
/// The requested name has no key in the file.
pub const NO_SUCH_KEY: &str = "no-such-key";

/// One provider key read from `keys.env`, printed only as its placeholder.
pub struct Key {
    name: String,
    value: String,
}
impl Key {
    /// The exact name written in the file.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The replacement used when redacting the value from output.
    pub fn placeholder(&self) -> String {
        format!("[baley:{}]", self.name)
    }

    /// The value for the child environment and the redactor in `baley exec`,
    /// and for the key header of [`list_request`].
    pub(crate) fn expose(&self) -> &str {
        &self.value
    }
}
impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.placeholder())
    }
}

/// A key whose bytes cannot be sent in a header. The file grammar refuses
/// every control byte, so no accepted key gives this, and it holds no text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyNotSendable;

/// The GET for `provider`'s model list. The key goes only in the
/// provider's key header, marked sensitive so a `Debug` of the request
/// prints `Sensitive` for it, and never in the URL.
pub(crate) fn list_request(provider: Provider, key: &Key) -> Result<Request, KeyNotSendable> {
    let (url, header, value) = match provider {
        Provider::OpenAi => (
            "https://api.openai.com/v1/models",
            AUTHORIZATION,
            format!("Bearer {}", key.expose()),
        ),
        Provider::DeepSeek => (
            "https://api.deepseek.com/models",
            AUTHORIZATION,
            format!("Bearer {}", key.expose()),
        ),
    };
    let url = Url::parse(url).expect("a compiled list URL parses");
    let mut value = HeaderValue::from_str(&value).map_err(|_| KeyNotSendable)?;
    value.set_sensitive(true);
    let mut request = Request::new(Method::GET, url);
    request.headers_mut().insert(header, value);
    Ok(request)
}

/// The accepted keys and the configured path they came from.
#[derive(Debug)]
pub struct Keys {
    path: PathBuf,
    missing: bool,
    keys: Vec<Key>,
}
impl Keys {
    /// Parses supplied bytes for tests without reading a file.
    #[cfg(test)]
    pub(crate) fn parsed(path: &Path, bytes: &[u8]) -> Keys {
        Keys {
            path: path.into(),
            missing: false,
            keys: parse(bytes).expect("valid keys fixture"),
        }
    }

    /// Finds a key by exact name, without case folding or trimming.
    pub fn get(&self, name: &str) -> Result<&Key, KeysRefusal> {
        self.keys
            .iter()
            .find(|key| key.name == name)
            .ok_or_else(|| KeysRefusal::NoSuchKey {
                name: name.into(),
                path: self.path.clone(),
                missing: self.missing,
            })
    }
}

/// A grammar fault that carries no text from the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineFault {
    /// The one-based physical line number.
    pub line: usize,
    /// Why the line was refused.
    pub problem: LineProblem,
}

/// A fixed description of a refused line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineProblem {
    /// The first line starts with a UTF-8 byte-order mark.
    ByteOrderMark,
    /// A non-comment line is not UTF-8.
    NotUtf8,
    /// The line does not follow the keys grammar.
    Malformed,
    /// The value has no bytes to redact.
    EmptyValue,
    /// The value holds a forbidden control byte.
    ControlCharacter,
    /// An earlier accepted line used this name.
    Repeats {
        /// The first accepted line with the name.
        first: usize,
    },
}

/// A refusal containing facts and line numbers, never a line's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeysRefusal {
    /// The configured path names something other than a regular file.
    NotRegular {
        /// The configured path, without canonicalization.
        path: PathBuf,
    },
    /// Every known ownership or read-mode fault.
    Exposed {
        /// The configured path.
        path: PathBuf,
        /// The effective user id, for the ownership fix.
        user: u32,
        /// The actual owner, only when it differs from the effective user.
        owner: Option<u32>,
        /// The actual mode, only when group or others can read.
        mode: Option<u32>,
    },
    /// Reading failed without a known kind or exposure fault.
    Unreadable {
        /// The configured path.
        path: PathBuf,
        /// The operating system's error text.
        error: String,
    },
    /// All invalid lines in file order.
    Invalid {
        /// The configured path.
        path: PathBuf,
        /// Faults with no file text.
        faults: Vec<LineFault>,
    },
    /// A caller-supplied name has no accepted key.
    NoSuchKey {
        /// The name supplied by the caller, never taken from a refused line.
        name: String,
        /// The configured path.
        path: PathBuf,
        /// Whether opening the file reported it missing.
        missing: bool,
    },
}
impl KeysRefusal {
    /// The stable refusal code for command rendering and detection.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotRegular { .. } | Self::Unreadable { .. } => KEYS_FILE_UNREADABLE,
            Self::Exposed { .. } => KEYS_FILE_EXPOSED,
            Self::Invalid { .. } => KEYS_FILE_INVALID,
            Self::NoSuchKey { .. } => NO_SUCH_KEY,
        }
    }
}
impl fmt::Display for KeysRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.code())?;
        match self {
            Self::NotRegular { path } => write!(
                f,
                "{} is not a regular file (fix: replace it with a regular file of NAME=value lines)",
                path.display()
            ),
            Self::Exposed {
                path,
                user,
                owner,
                mode,
            } => {
                if let Some(owner) = owner {
                    write!(
                        f,
                        "{} is owned by user id {owner}, not by this user ({user}) (fix: {})",
                        path.display(),
                        fix(&format!("chown {user}"), path)
                    )?;
                }
                if let Some(mode) = mode {
                    if owner.is_some() {
                        f.write_str("; ")?;
                    }
                    write!(
                        f,
                        "{} has mode {mode:04o}, so its group or others can read it (fix: {})",
                        path.display(),
                        fix("chmod 600", path)
                    )?;
                }
                Ok(())
            }
            Self::Unreadable { path, error } => {
                write!(f, "cannot read {}: {error}", path.display())
            }
            Self::Invalid { path, faults } => {
                write!(f, "{}: ", path.display())?;
                for (index, fault) in faults.iter().enumerate() {
                    if index != 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "line {} ", fault.line)?;
                    match fault.problem {
                        LineProblem::ByteOrderMark => {
                            f.write_str("starts with a byte-order mark")?
                        }
                        LineProblem::NotUtf8 => f.write_str("is not UTF-8 text")?,
                        LineProblem::Malformed => {
                            f.write_str("is not a NAME=value line, a comment or blank")?
                        }
                        LineProblem::EmptyValue => f.write_str("has an empty value")?,
                        LineProblem::ControlCharacter => {
                            f.write_str("holds a control character")?
                        }
                        LineProblem::Repeats { first } => {
                            write!(f, "repeats the name on line {first}")?
                        }
                    }
                }
                f.write_str(" (fix: correct each named line)")
            }
            Self::NoSuchKey {
                name,
                path,
                missing,
            } => {
                write!(f, "{name} is not in {}", path.display())?;
                if *missing {
                    write!(
                        f,
                        ", which does not exist (fix: create it with mode 600 and add a {name}=... line)"
                    )
                } else {
                    write!(f, " (fix: add a {name}=... line to it)")
                }
            }
        }
    }
}

// No one quoting of raw bytes works in bash, zsh, dash and fish alike.
fn fix(command: &str, path: &Path) -> String {
    match path.to_str() {
        Some(text)
            if !text.is_empty()
                && text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/._-+@%:,=".contains(&b)) =>
        {
            format!("{command} {text}")
        }
        Some(text) => format!("{command} '{}'", text.replace('\'', "'\\''")),
        None => format!(
            "{command} on this path, whose name is not UTF-8 and cannot be written as a command"
        ),
    }
}

/// Reads and judges `keys.env` in the resolved configuration folder.
pub fn load(config: &Path) -> Result<Keys, KeysRefusal> {
    judge(gather(&config.join("keys.env")))
}

/// Observations from one read, kept separate from the exposure decision.
pub(crate) struct Gathered {
    path: PathBuf,
    user: u32,
    seen: Seen,
}
enum Seen {
    Missing,
    Unreadable { error: String, meta: Option<Meta> },
    Opened { meta: Meta, bytes: Option<Vec<u8>> },
}
struct Meta {
    kind: Kind,
    owner: u32,
    mode: u32,
}
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    File,
    Folder,
    Other,
}

fn metadata(meta: fs::Metadata) -> Meta {
    Meta {
        kind: if meta.is_file() {
            Kind::File
        } else if meta.is_dir() {
            Kind::Folder
        } else {
            Kind::Other
        },
        owner: meta.uid(),
        mode: meta.mode() & 0o7777,
    }
}

fn missing_open(kind: io::ErrorKind) -> bool {
    kind == io::ErrorKind::NotFound
}

/// Opens the configured path and gathers facts from the opened file.
pub(crate) fn gather(path: &Path) -> Gathered {
    let seen = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Err(error) if missing_open(error.kind()) => Seen::Missing,
        Err(error) => Seen::Unreadable {
            error: error.to_string(),
            meta: fs::metadata(path).ok().map(metadata),
        },
        Ok(mut file) => match file.metadata() {
            Err(error) => Seen::Unreadable {
                error: error.to_string(),
                meta: None,
            },
            Ok(meta) => {
                let meta = metadata(meta);
                if meta.kind == Kind::File {
                    let mut bytes = Vec::new();
                    match file.read_to_end(&mut bytes) {
                        Ok(_) => Seen::Opened {
                            meta,
                            bytes: Some(bytes),
                        },
                        Err(error) => Seen::Unreadable {
                            error: error.to_string(),
                            meta: Some(meta),
                        },
                    }
                } else {
                    Seen::Opened { meta, bytes: None }
                }
            }
        },
    };
    // SAFETY: geteuid has no preconditions and cannot fail.
    let user = unsafe { libc::geteuid() };
    Gathered {
        path: path.into(),
        user,
        seen,
    }
}

fn judge(gathered: Gathered) -> Result<Keys, KeysRefusal> {
    let Gathered { path, user, seen } = gathered;
    let meta = match &seen {
        Seen::Missing => {
            return Ok(Keys {
                path,
                missing: true,
                keys: Vec::new(),
            });
        }
        Seen::Unreadable { meta, .. } => meta.as_ref(),
        Seen::Opened { meta, .. } => Some(meta),
    };
    if let Some(meta) = meta {
        if meta.kind != Kind::File {
            return Err(KeysRefusal::NotRegular { path });
        }
        let owner = (meta.owner != user).then_some(meta.owner);
        let mode = (meta.mode & 0o044 != 0).then_some(meta.mode);
        if owner.is_some() || mode.is_some() {
            return Err(KeysRefusal::Exposed {
                path,
                user,
                owner,
                mode,
            });
        }
    }
    match seen {
        Seen::Unreadable { error, .. } => Err(KeysRefusal::Unreadable { path, error }),
        Seen::Opened {
            bytes: Some(bytes), ..
        } => {
            let keys = parse(&bytes).map_err(|faults| KeysRefusal::Invalid {
                path: path.clone(),
                faults,
            })?;
            Ok(Keys {
                path,
                missing: false,
                keys,
            })
        }
        _ => unreachable!("regular opened files have bytes, and missing files returned above"),
    }
}

fn parse(bytes: &[u8]) -> Result<Vec<Key>, Vec<LineFault>> {
    let mut keys: Vec<Key> = Vec::new();
    let mut lines = Vec::new();
    let mut faults = Vec::new();
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    for (index, raw) in bytes.split(|b| *b == b'\n').enumerate() {
        let line = index + 1;
        let end = raw
            .iter()
            .rposition(|b| !matches!(b, b' ' | b'\t' | b'\r'))
            .map_or(0, |i| i + 1);
        let raw = &raw[..end];
        match parse_line(raw, line) {
            Ok(None) => {}
            Ok(Some(key)) => {
                if let Some(index) = keys.iter().position(|previous| previous.name == key.name) {
                    faults.push(LineFault {
                        line,
                        problem: LineProblem::Repeats {
                            first: lines[index],
                        },
                    });
                } else {
                    keys.push(key);
                    lines.push(line);
                }
            }
            Err(problem) => faults.push(LineFault { line, problem }),
        }
    }
    if faults.is_empty() {
        Ok(keys)
    } else {
        Err(faults)
    }
}

fn parse_line(raw: &[u8], line: usize) -> Result<Option<Key>, LineProblem> {
    if line == 1 && raw.starts_with(b"\xef\xbb\xbf") {
        return Err(LineProblem::ByteOrderMark);
    }
    if raw.is_empty() || raw[0] == b'#' {
        return Ok(None);
    }
    let text = std::str::from_utf8(raw).map_err(|_| LineProblem::NotUtf8)?;
    let text = text.strip_prefix("export ").unwrap_or(text);
    let (name, value) = text.split_once('=').ok_or(LineProblem::Malformed)?;
    if name.is_empty()
        || !name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i != 0 && b.is_ascii_digit()))
    {
        return Err(LineProblem::Malformed);
    }
    let quoted = matches!(value.as_bytes().first(), Some(b'"' | b'\''));
    let value = if quoted {
        if value.len() < 2 || value.as_bytes().first() != value.as_bytes().last() {
            return Err(LineProblem::Malformed);
        }
        &value[1..value.len() - 1]
    } else {
        value
    };
    if value.is_empty() {
        return Err(LineProblem::EmptyValue);
    }
    if !quoted && value.bytes().any(|b| b == b' ' || b == b'\t') {
        return Err(LineProblem::Malformed);
    }
    if value
        .bytes()
        .any(|b| (b <= 0x1f || b == 0x7f) && !(quoted && b == b'\t'))
    {
        return Err(LineProblem::ControlCharacter);
    }
    Ok(Some(Key {
        name: name.into(),
        value: value.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::{ffi::OsStringExt, fs::symlink};

    fn observed(seen: Seen) -> Gathered {
        Gathered {
            path: "/c/keys.env".into(),
            user: 1000,
            seen,
        }
    }
    fn file(owner: u32, mode: u32) -> Meta {
        Meta {
            kind: Kind::File,
            owner,
            mode,
        }
    }
    fn opened(owner: u32, mode: u32, bytes: &[u8]) -> Gathered {
        observed(Seen::Opened {
            meta: file(owner, mode),
            bytes: Some(bytes.into()),
        })
    }
    fn fault(line: usize, problem: LineProblem) -> LineFault {
        LineFault { line, problem }
    }

    #[test]
    fn group_readable_keys_file_is_not_accepted() {
        let refusal = judge(opened(1000, 0o640, b"A=v")).unwrap_err();
        assert_eq!(refusal.code(), KEYS_FILE_EXPOSED);
        assert_eq!(
            refusal.to_string(),
            "keys-file-exposed: /c/keys.env has mode 0640, so its group or others can read it (fix: chmod 600 /c/keys.env)"
        );
    }
    #[test]
    fn other_readable_keys_file_is_not_accepted() {
        let refusal = judge(opened(1000, 0o604, b"A=v")).unwrap_err();
        assert_eq!(refusal.code(), KEYS_FILE_EXPOSED);
        assert_eq!(
            refusal.to_string(),
            "keys-file-exposed: /c/keys.env has mode 0604, so its group or others can read it (fix: chmod 600 /c/keys.env)"
        );
    }
    #[test]
    fn owner_only_modes_are_not_compared_for_equality() {
        for mode in [0o600, 0o400] {
            assert!(judge(opened(1000, mode, b"A=v")).is_ok());
        }
    }
    #[test]
    fn store_file_mask_is_not_applied_to_keys() {
        for mode in [0o700, 0o620, 0o602, 0o7600] {
            assert!(judge(opened(1000, mode, b"A=v")).is_ok());
        }
    }
    #[test]
    fn another_users_keys_file_is_not_accepted() {
        let refusal = judge(opened(1001, 0o600, b"A=v")).unwrap_err();
        assert_eq!(refusal.code(), KEYS_FILE_EXPOSED);
        assert_eq!(
            refusal.to_string(),
            "keys-file-exposed: /c/keys.env is owned by user id 1001, not by this user (1000) (fix: chown 1000 /c/keys.env)"
        );
    }
    #[test]
    fn wrong_owner_does_not_hide_readable_mode() {
        let refusal = judge(opened(1001, 0o644, b"A=v")).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "keys-file-exposed: /c/keys.env is owned by user id 1001, not by this user (1000) (fix: chown 1000 /c/keys.env); /c/keys.env has mode 0644, so its group or others can read it (fix: chmod 600 /c/keys.env)"
        );
    }
    #[test]
    fn non_regular_keys_file_is_not_read_as_no_keys() {
        for kind in [Kind::Folder, Kind::Other] {
            let refusal = judge(observed(Seen::Opened {
                meta: Meta {
                    kind,
                    owner: 1001,
                    mode: 0o644,
                },
                bytes: None,
            }))
            .unwrap_err();
            assert_eq!(refusal.code(), KEYS_FILE_UNREADABLE);
            assert_eq!(
                refusal.to_string(),
                "keys-file-unreadable: /c/keys.env is not a regular file (fix: replace it with a regular file of NAME=value lines)"
            );
        }
    }
    #[test]
    fn known_exposure_wins_over_an_unreadable_file() {
        let refusal = judge(observed(Seen::Unreadable {
            error: "permission denied".into(),
            meta: Some(file(1001, 0o600)),
        }))
        .unwrap_err();
        assert_eq!(refusal.code(), KEYS_FILE_EXPOSED);
        assert!(refusal.to_string().contains("chown 1000 /c/keys.env"));
        for meta in [None, Some(file(1000, 0o600))] {
            let refusal = judge(observed(Seen::Unreadable {
                error: "permission denied".into(),
                meta,
            }))
            .unwrap_err();
            assert_eq!(refusal.code(), KEYS_FILE_UNREADABLE);
            assert_eq!(
                refusal.to_string(),
                "keys-file-unreadable: cannot read /c/keys.env: permission denied"
            );
        }
    }
    #[test]
    fn exposed_file_is_refused_before_its_lines_are_judged() {
        let refusal = judge(opened(1000, 0o644, b"SENTINEL-4f9c")).unwrap_err();
        assert_eq!(refusal.code(), KEYS_FILE_EXPOSED);
        assert!(!refusal.to_string().contains("SENTINEL-4f9c"));
    }
    #[test]
    fn fix_names_the_same_path_in_every_shell() {
        assert_eq!(
            fix("chmod 600", Path::new("/c/Application Support/keys.env")),
            "chmod 600 '/c/Application Support/keys.env'"
        );
        assert_eq!(
            fix("chown 1000", Path::new("/c/o'ne/keys.env")),
            "chown 1000 '/c/o'\\''ne/keys.env'"
        );
        let raw = PathBuf::from(OsString::from_vec(b"/c/\xff/keys.env".to_vec()));
        assert_eq!(
            fix("chmod 600", &raw),
            "chmod 600 on this path, whose name is not UTF-8 and cannot be written as a command"
        );
    }
    #[test]
    fn missing_keys_file_is_no_keys_not_a_refusal() {
        let keys = judge(observed(Seen::Missing)).unwrap();
        let refusal = keys.get("OPENAI_API_KEY").unwrap_err();
        assert_eq!(refusal.code(), NO_SUCH_KEY);
        assert_eq!(
            refusal.to_string(),
            "no-such-key: OPENAI_API_KEY is not in /c/keys.env, which does not exist (fix: create it with mode 600 and add a OPENAI_API_KEY=... line)"
        );
    }
    #[test]
    fn open_errors_other_than_not_found_are_not_taken_as_no_keys() {
        assert!(missing_open(io::ErrorKind::NotFound));
        assert!(!missing_open(io::ErrorKind::PermissionDenied));
        assert!(!missing_open(io::ErrorKind::NotADirectory));
    }
    #[test]
    fn every_written_form_reads_to_its_value() {
        for (input, name, value) in [
            ("OPENAI_API_KEY=sk-abc", "OPENAI_API_KEY", "sk-abc"),
            ("export OPENAI_API_KEY=sk-abc", "OPENAI_API_KEY", "sk-abc"),
            ("A=\"sk abc\"", "A", "sk abc"),
            ("A='sk\"abc'", "A", "sk\"abc"),
            ("A=sk-abc  \t", "A", "sk-abc"),
            ("A=sk-abc\r\n", "A", "sk-abc"),
            ("A=\"sk-abc\"  ", "A", "sk-abc"),
            ("A=a=b", "A", "a=b"),
            ("A=\"a\"b\"", "A", "a\"b"),
            ("A=abc\"", "A", "abc\""),
            ("A=a\\b", "A", "a\\b"),
            ("A=\"a\tb\"", "A", "a\tb"),
            ("A='a\tb'", "A", "a\tb"),
            ("export=1", "export", "1"),
            ("_A1=v", "_A1", "v"),
            ("A=é", "A", "é"),
        ] {
            let keys = parse(input.as_bytes()).unwrap();
            assert_eq!(keys.len(), 1);
            assert_eq!((&*keys[0].name, &*keys[0].value), (name, value));
        }
        assert!(parse(b"# anything\n\n \t\r\n").unwrap().is_empty());
        let keys = parse(b"Z=1\nA=2\na=3\n").unwrap();
        assert_eq!(
            keys.iter().map(Key::name).collect::<Vec<_>>(),
            ["Z", "A", "a"]
        );
    }
    #[test]
    fn comment_and_blank_lines_still_count_as_lines() {
        assert_eq!(
            parse(b"# one\n# two\n\ninvalid").unwrap_err(),
            [fault(4, LineProblem::Malformed)]
        );
    }
    #[test]
    fn repeated_name_is_not_resolved_by_the_last_line() {
        assert_eq!(
            parse(b"# c\nA=1\n\n# d\nexport A=2\nA=3").unwrap_err(),
            [
                fault(5, LineProblem::Repeats { first: 2 }),
                fault(6, LineProblem::Repeats { first: 2 })
            ]
        );
        assert_eq!(
            parse(b"A=\nA=v\nA=w").unwrap_err(),
            [
                fault(1, LineProblem::EmptyValue),
                fault(3, LineProblem::Repeats { first: 2 })
            ]
        );
    }
    #[test]
    fn empty_value_is_not_accepted_as_a_key() {
        assert_eq!(
            parse(b"A=\nA=\"\"\nA=''\nA=   ").unwrap_err(),
            (1..=4)
                .map(|line| fault(line, LineProblem::EmptyValue))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn malformed_lines_are_refused_not_skipped() {
        for input in [
            " A=v",
            "  # c",
            "A =v",
            "1A=v",
            "A-B=v",
            "A",
            "=v",
            "export  A=v",
            "export A",
            "A=\"v",
            "A='v\"",
            "A=\"",
            "SENTINEL-4f9c",
            "A=a b",
            "A=a\tb",
            "A= abc",
            "A=abc # note",
            "A=\"v\"suffix",
            "é=v",
        ] {
            assert_eq!(
                parse(input.as_bytes()).unwrap_err(),
                [fault(1, LineProblem::Malformed)]
            );
        }
    }
    #[test]
    fn every_refused_line_is_named_not_only_the_first() {
        let faults = parse(b"bad\nA=\nB=x\ry").unwrap_err();
        assert_eq!(
            faults,
            [
                fault(1, LineProblem::Malformed),
                fault(2, LineProblem::EmptyValue),
                fault(3, LineProblem::ControlCharacter)
            ]
        );
        let refusal = KeysRefusal::Invalid {
            path: "/c/keys.env".into(),
            faults,
        };
        assert_eq!(refusal.code(), KEYS_FILE_INVALID);
        assert_eq!(
            refusal.to_string(),
            "keys-file-invalid: /c/keys.env: line 1 is not a NAME=value line, a comment or blank; line 2 has an empty value; line 3 holds a control character (fix: correct each named line)"
        );
    }
    #[test]
    fn refusal_text_never_holds_a_line() {
        // A valid-name sentinel also catches leaks from the name position.
        for (input, text) in [
            (
                b"\xef\xbb\xbfA=SENTINEL_4f9c".as_slice(),
                "line 1 starts with a byte-order mark",
            ),
            (b"A=SENTINEL_4f9c\xe9", "line 1 is not UTF-8 text"),
            (
                b"SENTINEL_4f9c",
                "line 1 is not a NAME=value line, a comment or blank",
            ),
            (b"SENTINEL_4f9c=", "line 1 has an empty value"),
            (b"A=SENTINEL_4f9c\x00", "line 1 holds a control character"),
            (
                b"SENTINEL_4f9c=v\nSENTINEL_4f9c=w",
                "line 2 repeats the name on line 1",
            ),
        ] {
            let refusal = KeysRefusal::Invalid {
                path: "/c/keys.env".into(),
                faults: parse(input).unwrap_err(),
            };
            assert_eq!(
                refusal.to_string(),
                format!("keys-file-invalid: /c/keys.env: {text} (fix: correct each named line)")
            );
            assert!(!format!("{refusal:?}").contains("SENTINEL_4f9c"));
        }
    }
    #[test]
    fn text_that_is_not_utf8_is_not_read_as_a_key() {
        assert_eq!(
            parse(b"A=\xe9").unwrap_err(),
            [fault(1, LineProblem::NotUtf8)]
        );
        assert!(parse(b"# A=\xe9").unwrap().is_empty());
    }
    #[test]
    fn unquoted_white_space_is_refused_not_read_as_a_shell_would() {
        assert_eq!(
            parse(b"A=a b\nA=a\tb\nA= abc").unwrap_err(),
            (1..=3)
                .map(|line| fault(line, LineProblem::Malformed))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn carriage_return_inside_a_line_is_not_part_of_a_key() {
        for input in [b"A=x\rB=y".as_slice(), b"A=\"x\ry\""] {
            assert_eq!(
                parse(input).unwrap_err(),
                [fault(1, LineProblem::ControlCharacter)]
            );
        }
        for byte in (0..=0x1f)
            .chain([0x7f])
            .filter(|b| !matches!(b, b'\n' | b'\t'))
        {
            assert_eq!(
                parse(&[b'A', b'=', b'"', b'x', byte, b'y', b'"']).unwrap_err(),
                [fault(1, LineProblem::ControlCharacter)]
            );
        }
        for byte in [0, 0x0b, 0x0c, 0x1f, 0x7f] {
            assert_eq!(
                parse(&[b'A', b'=', b'x', byte]).unwrap_err(),
                [fault(1, LineProblem::ControlCharacter)]
            );
        }
    }
    #[test]
    fn byte_order_mark_is_named_not_hidden_in_the_name() {
        assert_eq!(
            parse(b"\xef\xbb\xbfA=v").unwrap_err(),
            [fault(1, LineProblem::ByteOrderMark)]
        );
        assert_eq!(
            parse(b"# c\n\xef\xbb\xbfA=v").unwrap_err(),
            [fault(2, LineProblem::Malformed)]
        );
    }
    #[test]
    fn key_debug_shows_its_placeholder_not_its_value() {
        let key = Key {
            name: "OPENAI_API_KEY".into(),
            value: "SENTINEL-4f9c".into(),
        };
        assert_eq!(key.placeholder(), "[baley:OPENAI_API_KEY]");
        assert_eq!(format!("{key:?}"), "[baley:OPENAI_API_KEY]");
        let keys = Keys {
            path: "/c/keys.env".into(),
            missing: false,
            keys: vec![key],
        };
        assert!(!format!("{keys:?}").contains("SENTINEL-4f9c"));
    }
    #[test]
    fn lookup_is_by_exact_name() {
        let keys = Keys {
            path: "/c/keys.env".into(),
            missing: false,
            keys: vec![Key {
                name: "OPENAI_API_KEY".into(),
                value: "v".into(),
            }],
        };
        assert_eq!(keys.get("OPENAI_API_KEY").unwrap().name(), "OPENAI_API_KEY");
        for name in [
            "openai_api_key",
            "OPENAI_API",
            "OPENAI_API_KEY_2",
            " OPENAI_API_KEY",
        ] {
            let refusal = keys.get(name).unwrap_err();
            assert_eq!(refusal.code(), NO_SUCH_KEY);
            assert_eq!(
                refusal.to_string(),
                format!(
                    "no-such-key: {name} is not in /c/keys.env (fix: add a {name}=... line to it)"
                )
            );
        }
    }
    const SENTINEL: &str = "sk-SENTINEL-7d31";

    fn sentinel_key(name: &str) -> Keys {
        Keys::parsed(
            Path::new("/c/keys.env"),
            format!("{name}={SENTINEL}").as_bytes(),
        )
    }

    /// Asserts the request is a GET to exactly `origin` and `path`, with
    /// the sentinel only in `header` as `value`, marked sensitive, and
    /// nowhere in the URL or the request's `Debug`.
    fn assert_key_only_in_header(
        request: &Request,
        origin: &str,
        path: &str,
        header: &str,
        value: &str,
    ) {
        assert_eq!(request.method(), Method::GET);
        let url = request.url();
        assert_eq!(url.scheme(), "https");
        assert_eq!(format!("https://{}", url.host_str().unwrap()), origin);
        assert_eq!(url.port(), None);
        assert_eq!(url.path(), path);
        let sent = request.headers().get(header).unwrap();
        assert_eq!(sent.as_bytes(), value.as_bytes());
        assert!(sent.is_sensitive());
        for (name, other) in request.headers() {
            if name.as_str() != header {
                let text = String::from_utf8_lossy(other.as_bytes());
                assert!(!text.contains(SENTINEL), "{name} holds the key");
            }
        }
        assert!(!url.as_str().contains(SENTINEL));
        let debug = format!("{request:?}");
        assert!(!debug.contains(SENTINEL), "{debug}");
        assert!(debug.contains("Sensitive"), "{debug}");
    }

    #[test]
    fn an_openai_list_request_carries_the_key_only_in_a_sensitive_bearer_header() {
        let keys = sentinel_key("OPENAI_API_KEY");
        let key = keys.get("OPENAI_API_KEY").unwrap();
        let request = list_request(Provider::OpenAi, key).unwrap();
        assert_key_only_in_header(
            &request,
            "https://api.openai.com",
            "/v1/models",
            "authorization",
            &format!("Bearer {SENTINEL}"),
        );
        assert_eq!(request.url().query(), None);
    }

    #[test]
    fn a_deepseek_list_request_carries_the_key_only_in_a_sensitive_bearer_header() {
        let keys = sentinel_key("DEEPSEEK_API_KEY");
        let key = keys.get("DEEPSEEK_API_KEY").unwrap();
        let request = list_request(Provider::DeepSeek, key).unwrap();
        assert_key_only_in_header(
            &request,
            "https://api.deepseek.com",
            "/models",
            "authorization",
            &format!("Bearer {SENTINEL}"),
        );
        assert_eq!(request.url().query(), None);
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused_without_its_text() {
        let key = Key {
            name: "OPENAI_API_KEY".into(),
            value: format!("{SENTINEL}\n"),
        };
        let refused = list_request(Provider::OpenAi, &key).unwrap_err();
        assert!(!format!("{refused:?}").contains(SENTINEL));
    }

    #[test]
    fn gatherer_reads_keys_through_a_symbolic_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.env");
        let link = dir.path().join("keys.env");
        fs::write(&real, b"A=v").unwrap();
        symlink(&real, &link).unwrap();
        let gathered = gather(&link);
        assert_eq!(gathered.path, link);
        let Seen::Opened { meta, bytes } = gathered.seen else {
            panic!("the link must be followed")
        };
        assert_eq!(meta.kind, Kind::File);
        assert_eq!(bytes.as_deref(), Some(b"A=v".as_slice()));
    }
}
