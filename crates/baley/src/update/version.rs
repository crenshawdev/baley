//! Version order for staged and offered binaries (design 0012 section 5).
//!
//! A version is `major.minor.patch`, compared part by part as numbers, so
//! `0.10.0` is above `0.9.0`. The crate is small enough that no semver
//! library is worth its place in the lockfile.

use std::fmt;

/// A `major.minor.patch` version. The derived order compares the parts as
/// numbers, major first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// Reads a version from exactly three dot-separated decimal numbers.
    ///
    /// Each part must fit `u64`, carry no sign and have no leading zero
    /// except a lone `0`, and nothing may come before, between or after the
    /// parts. Leading zeros are refused so one version has one folder name.
    pub fn parse(text: &str) -> Result<Version, InvalidVersion> {
        let invalid = |reason| InvalidVersion {
            text: text.to_owned(),
            reason,
        };
        let parts: Vec<&str> = text.split('.').collect();
        let [major, minor, patch] = parts[..] else {
            return Err(invalid("it needs exactly three parts, major.minor.patch"));
        };
        let part = |part: &str| -> Result<u64, &'static str> {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("each part must be decimal digits only");
            }
            if part.len() > 1 && part.starts_with('0') {
                return Err("a part must not start with a zero");
            }
            part.parse().map_err(|_| "a part is too large")
        };
        Ok(Version {
            major: part(major).map_err(invalid)?,
            minor: part(minor).map_err(invalid)?,
            patch: part(patch).map_err(invalid)?,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Text that is not a version, with what is wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidVersion {
    text: String,
    reason: &'static str,
}

impl fmt::Display for InvalidVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "\"{}\" is not a version: {}",
            self.text.escape_debug(),
            self.reason
        )
    }
}

impl std::error::Error for InvalidVersion {}

/// What a check does with the version a source offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// The offered version is higher: stage it beside the active one.
    Stage {
        /// The version the stable path runs.
        active: Version,
        /// The higher version the source offers.
        offered: Version,
    },
    /// The offered version is the same or lower. A lower one means the source
    /// is behind, never that the installation should move back.
    Current {
        /// The version the stable path runs.
        active: Version,
        /// The version the source offers.
        offered: Version,
    },
}

/// Chooses between staging the offered version and calling the installation
/// current: only a strictly higher offer is staged.
pub fn select(active: Version, offered: Version) -> Selection {
    if offered > active {
        Selection::Stage { active, offered }
    } else {
        Selection::Current { active, offered }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).expect(text)
    }

    #[test]
    fn an_offered_version_staged_when_not_higher_or_compared_as_text_is_caught() {
        // (active, offered, stage the offered version)
        let cases = [
            ("0.1.0", "0.1.1", true),
            ("0.1.0", "0.2.0", true),
            ("0.1.9", "1.0.0", true),
            ("0.9.0", "0.10.0", true),
            ("1.2.3", "1.2.3", false),
            ("0.2.0", "0.1.9", false),
            ("1.0.0", "0.99.99", false),
            ("0.1.10", "0.1.9", false),
        ];
        for (active, offered, stage) in cases {
            let (active, offered) = (v(active), v(offered));
            let expected = if stage {
                Selection::Stage { active, offered }
            } else {
                Selection::Current { active, offered }
            };
            assert_eq!(select(active, offered), expected, "{active} then {offered}");
        }
    }

    #[test]
    fn a_malformed_version_read_as_valid_is_caught() {
        for text in ["0.1.0", "10.20.30", "18446744073709551615.0.0"] {
            assert_eq!(v(text).to_string(), text);
        }
        for text in [
            "1.2",
            "1.2.3.4",
            "v1.2.3",
            "01.2.3",
            "1.02.3",
            "1.2.3-rc.1",
            "+1.2.3",
            " 1.2.3",
            "1.2.3 ",
            "",
            "18446744073709551616.0.0",
            "1..3",
        ] {
            assert!(Version::parse(text).is_err(), "{text:?} was accepted");
        }
    }
}
