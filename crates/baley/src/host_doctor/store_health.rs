//! What the store's own health tells the host section: whether this binary
//! can write the ledger, and which folders the ledger and the settings live
//! in.
//!
//! Everything here is judged from values the doctor already holds: the
//! [`Health`] that the store's doctor returned, the resolved [`Folders`] and
//! this binary's compatibility epoch. Nothing is read or written. The home
//! folder was opened with its ownership, mode and link checks before the
//! doctor ran, since the open refuses an unsafe home. The config folder gets
//! no mode, owner or link judgement, and its `keys` file is never opened.

use std::path::PathBuf;

use baley_store::Health;
use baley_store_sqlite::EPOCH;

use crate::folders::Folders;

/// What the stored epoch means for this binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    /// The ledger is at this binary's epoch.
    Matches,
    /// A newer Baley wrote the ledger: this binary can only read it.
    Newer {
        /// The epoch the ledger holds, which a writing build must have.
        epoch: u32,
    },
}

/// The store's health as the host section needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// The stored epoch against this binary's.
    pub compatibility: Compatibility,
    /// The ledger home, which opened with its checks passed.
    pub home: PathBuf,
    /// The configuration folder.
    pub config: PathBuf,
    /// Whether the two are one folder, as `BALEY_HOME` and macOS make them.
    pub one_folder: bool,
}

/// Judges the stored epoch and the folders. An epoch below this binary's
/// never reaches the doctor, because opening the store refuses it, so a
/// stored epoch that is not newer is this binary's own. The folders are one
/// folder only when the two paths are the same path; a config folder inside
/// the home is another folder.
pub fn judge(health: &Health, folders: &Folders) -> Judged {
    let compatibility = if health.epoch > EPOCH {
        Compatibility::Newer {
            epoch: health.epoch,
        }
    } else {
        Compatibility::Matches
    };
    Judged {
        compatibility,
        home: folders.home.clone(),
        config: folders.config.clone(),
        one_folder: folders.home == folders.config,
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::clean_health;
    use super::*;

    fn folders(home: &str, config: &str) -> Folders {
        Folders {
            home: home.into(),
            config: config.into(),
        }
    }

    #[test]
    fn the_binarys_own_epoch_reported_as_read_only_is_caught() {
        let mut health = clean_health();
        health.epoch = EPOCH;
        assert_eq!(
            judge(&health, &folders("/h", "/c")).compatibility,
            Compatibility::Matches
        );
        health.epoch = EPOCH + 1;
        assert_eq!(
            judge(&health, &folders("/h", "/c")).compatibility,
            Compatibility::Newer { epoch: EPOCH + 1 }
        );
    }

    #[test]
    fn a_config_folder_shared_with_the_home_left_unremarked_is_caught() {
        let health = clean_health();
        for (home, config, shared) in [
            ("/h", "/h", true),
            ("/h", "/c", false),
            ("/h", "/h/config", false),
        ] {
            assert_eq!(
                judge(&health, &folders(home, config)).one_folder,
                shared,
                "home {home}, config {config}"
            );
        }
    }
}
