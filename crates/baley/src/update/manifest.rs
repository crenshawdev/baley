//! The development artifact: a plain `baley` executable and a small manifest
//! that names its version and SHA-256 (design 0012 section 5).
//!
//! For the running platform the source holds two files:
//! `<source>/<os>-<arch>/manifest` and `<source>/<os>-<arch>/baley`. The
//! manifest is UTF-8 text of exactly two lines, each ending in a newline:
//!
//! ```text
//! version 0.2.0
//! sha256 <64 lowercase hex digits>
//! ```
//!
//! These artifacts are unsigned. [`verify_download`] checks the bytes against
//! the manifest's digest and nothing else.

use std::fmt;

use sha2::{Digest, Sha256};

use super::version::{InvalidVersion, Version};

/// Where the two files of one platform are fetched from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addresses {
    /// The manifest.
    pub manifest: String,
    /// The executable.
    pub binary: String,
}

/// The addresses of one platform's files under a source. A trailing `/` on
/// the source is dropped so the addresses never hold a double slash.
pub fn addresses(source: &str, os: &str, arch: &str) -> Addresses {
    let folder = format!("{}/{os}-{arch}", source.trim_end_matches('/'));
    Addresses {
        manifest: format!("{folder}/manifest"),
        binary: format!("{folder}/baley"),
    }
}

/// A parsed manifest: the version on offer and the digest its bytes must have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The offered version.
    pub version: Version,
    /// The SHA-256 of the executable, 64 lowercase hex digits.
    pub sha256: String,
}

/// Why manifest bytes are not a manifest. Lines count from one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestFault {
    /// The bytes are not UTF-8.
    NotUtf8,
    /// The manifest ends before this line.
    MissingLine(usize),
    /// This line does not end in a newline.
    MissingNewline(usize),
    /// A line after the second.
    ExtraLine(usize),
    /// This line should start with the given key and a space.
    WrongKey {
        /// The line.
        line: usize,
        /// The key it should start with.
        key: &'static str,
    },
    /// The version on line 1 is not a version.
    BadVersion(InvalidVersion),
    /// The digest on line 2 is not 64 lowercase hex digits.
    BadDigest,
}

impl fmt::Display for ManifestFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestFault::NotUtf8 => f.write_str("the manifest is not UTF-8"),
            ManifestFault::MissingLine(line) => write!(f, "line {line} is missing"),
            ManifestFault::MissingNewline(line) => {
                write!(f, "line {line} does not end in a newline")
            }
            ManifestFault::ExtraLine(line) => {
                write!(
                    f,
                    "line {line} should not be there; the manifest has two lines"
                )
            }
            ManifestFault::WrongKey { line, key } => {
                write!(f, "line {line} should start with \"{key} \"")
            }
            ManifestFault::BadVersion(error) => write!(f, "line 1: {error}"),
            ManifestFault::BadDigest => {
                f.write_str("line 2: the digest is not 64 lowercase hex digits")
            }
        }
    }
}

impl std::error::Error for ManifestFault {}

/// Reads manifest bytes. Anything but the exact two-line form is refused, so
/// a manifest that another reader (the install script) would read differently
/// is never accepted here.
pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, ManifestFault> {
    let text = std::str::from_utf8(bytes).map_err(|_| ManifestFault::NotUtf8)?;
    let mut lines = text.split_inclusive('\n');
    let mut line = |number: usize, key: &'static str| -> Result<&str, ManifestFault> {
        let raw = lines.next().ok_or(ManifestFault::MissingLine(number))?;
        let body = raw
            .strip_suffix('\n')
            .ok_or(ManifestFault::MissingNewline(number))?;
        body.strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(' '))
            .ok_or(ManifestFault::WrongKey { line: number, key })
    };
    let version = line(1, "version")?;
    let digest = line(2, "sha256")?;
    if lines.next().is_some() {
        return Err(ManifestFault::ExtraLine(3));
    }
    let version = Version::parse(version).map_err(ManifestFault::BadVersion)?;
    let is_digest = digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !is_digest {
        return Err(ManifestFault::BadDigest);
    }
    Ok(Manifest {
        version,
        sha256: digest.to_owned(),
    })
}

/// Downloaded bytes whose SHA-256 matched their manifest. Only
/// [`verify_download`] makes one, so staging cannot be handed bytes that
/// skipped the check.
#[derive(Debug, PartialEq, Eq)]
pub struct VerifiedDownload {
    bytes: Vec<u8>,
    version: Version,
}

impl VerifiedDownload {
    /// The verified executable.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The version the manifest named for these bytes.
    pub fn version(&self) -> Version {
        self.version
    }
}

/// The bytes' digest differs from the manifest's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumMismatch {
    /// The digest the manifest names.
    pub expected: String,
    /// The digest of the downloaded bytes.
    pub actual: String,
}

impl fmt::Display for ChecksumMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the download's SHA-256 is {}, but the manifest names {}",
            self.actual, self.expected
        )
    }
}

impl std::error::Error for ChecksumMismatch {}

/// Checks downloaded bytes against the manifest's SHA-256 and returns them as
/// a [`VerifiedDownload`], or the two digests when they differ. Nothing skips
/// this: no flag, setting or environment variable.
///
/// This checks unsigned development artifacts only. Issue #14 replaces its
/// body with verification of a signed checksum manifest before any release.
pub fn verify_download(
    bytes: Vec<u8>,
    manifest: &Manifest,
) -> Result<VerifiedDownload, ChecksumMismatch> {
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != manifest.sha256 {
        return Err(ChecksumMismatch {
            expected: manifest.sha256.clone(),
            actual,
        });
    }
    Ok(VerifiedDownload {
        bytes,
        version: manifest.version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_of(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn a_malformed_manifest_read_as_valid_is_caught() {
        let digest = "0123456789abcdef".repeat(4);
        let good = format!("version 0.2.0\nsha256 {digest}\n");
        assert_eq!(
            parse_manifest(good.as_bytes()),
            Ok(Manifest {
                version: Version::parse("0.2.0").unwrap(),
                sha256: digest.clone(),
            })
        );
        // (what is wrong, the bytes, what the refusal must name)
        let refused: Vec<(&str, Vec<u8>, &str)> = vec![
            (
                "no final newline",
                good.trim_end().as_bytes().to_vec(),
                "line 2 does not end in a newline",
            ),
            (
                "lines swapped",
                format!("sha256 {digest}\nversion 0.2.0\n").into_bytes(),
                "line 1 should start with \"version \"",
            ),
            (
                "a third line",
                format!("{good}os linux\n").into_bytes(),
                "line 3 should not be there",
            ),
            (
                "an uppercase digest",
                format!("version 0.2.0\nsha256 {}\n", digest.to_uppercase()).into_bytes(),
                "line 2: the digest",
            ),
            (
                "63 hex digits",
                format!("version 0.2.0\nsha256 {}\n", &digest[1..]).into_bytes(),
                "line 2: the digest",
            ),
            (
                "a two-part version",
                format!("version 0.2\nsha256 {digest}\n").into_bytes(),
                "line 1:",
            ),
            (
                "carriage returns",
                format!("version 0.2.0\r\nsha256 {digest}\r\n").into_bytes(),
                "line 1:",
            ),
            ("one byte that is not UTF-8", vec![0xff], "not UTF-8"),
        ];
        for (what, bytes, names) in refused {
            let fault = parse_manifest(&bytes).expect_err(what).to_string();
            assert!(fault.contains(names), "{what}: {fault}");
        }
    }

    #[test]
    fn a_download_accepted_with_another_digest_is_caught() {
        let manifest = Manifest {
            version: Version::parse("0.2.0").unwrap(),
            sha256: digest_of(b"baley"),
        };
        let verified = verify_download(b"baley".to_vec(), &manifest).expect("matching bytes");
        assert_eq!(verified.bytes(), b"baley");
        assert_eq!(verified.version(), manifest.version);

        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        for (bytes, actual) in [
            (b"baleY".to_vec(), digest_of(b"baleY")),
            (Vec::new(), empty.to_owned()),
        ] {
            let mismatch = verify_download(bytes, &manifest).expect_err("other bytes");
            assert_eq!(
                mismatch,
                ChecksumMismatch {
                    expected: manifest.sha256.clone(),
                    actual: actual.clone(),
                }
            );
            let text = mismatch.to_string();
            assert!(
                text.contains(&actual) && text.contains(&manifest.sha256),
                "{text}"
            );
        }
    }

    #[test]
    fn a_platform_file_fetched_from_the_wrong_address_is_caught() {
        for source in ["https://dl.example/dev", "https://dl.example/dev/"] {
            assert_eq!(
                addresses(source, "linux", "x86_64"),
                Addresses {
                    manifest: "https://dl.example/dev/linux-x86_64/manifest".into(),
                    binary: "https://dl.example/dev/linux-x86_64/baley".into(),
                },
                "{source}"
            );
        }
        assert_eq!(
            addresses("https://dl.example/dev", "macos", "aarch64"),
            Addresses {
                manifest: "https://dl.example/dev/macos-aarch64/manifest".into(),
                binary: "https://dl.example/dev/macos-aarch64/baley".into(),
            }
        );
    }
}
