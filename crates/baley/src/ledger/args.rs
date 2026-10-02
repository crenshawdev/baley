//! Arguments for owner operations on the ledger.
use baley_store::Hash;
use clap::Subcommand;
use std::path::PathBuf;

/// Ledger commands, beside the inherited command surface.
#[derive(Debug, Clone, Subcommand)]
pub enum LedgerCommand {
    /// Check the chain. With no flag it checks the checkout's project against
    /// the remote its `git.remote` names, or locally when none is set.
    /// `--local-only` and `--views` name the project and need no checkout.
    Verify {
        /// Ledger project id; required with `--local-only` or `--views`.
        project: Option<String>,
        /// Check the named project without a remote witness.
        #[arg(long, requires = "project", conflicts_with = "views")]
        local_only: bool,
        /// Compare the named project's views with a replay.
        #[arg(long, requires = "project")]
        views: bool,
    },
    /// Report the health of every project. The checkout's project is checked
    /// against the remote its `git.remote` names, every other one locally.
    Doctor,
    /// Export one project into a new standalone home.
    Export {
        /// Ledger project id.
        project: String,
        /// New directory for the export.
        #[arg(long)]
        to: PathBuf,
    },
    /// Remove explicitly named payload bodies.
    Purge {
        /// Ledger project id.
        project: String,
        /// SHA-256 hashes of the bodies to purge.
        #[arg(required = true, num_args = 1.., value_parser = payload_hash)]
        hashes: Vec<Hash>,
        /// Why these bodies must be removed.
        #[arg(long, value_parser = purge_reason)]
        reason: String,
    },
    /// Retry an incomplete purge scrub.
    Scrub,
    /// Rebuild all views of a project.
    Rebuild {
        /// Ledger project id.
        project: String,
    },
    /// Anchor the checkout's project on the remote its `git.remote` names.
    Anchor {
        /// Ledger project id; only the checkout's own project is accepted.
        project: Option<String>,
    },
    /// Accept a restored chain behind the anchor on the remote its
    /// `git.remote` names.
    AcknowledgeRestore {
        /// Ledger project id; only the checkout's own project is accepted.
        project: Option<String>,
    },
}

fn payload_hash(text: &str) -> Result<Hash, String> {
    Hash::from_hex(text).ok_or_else(|| "a payload hash is 64 hex digits".into())
}
fn purge_reason(text: &str) -> Result<String, String> {
    if text.is_empty() {
        Err("a purge needs a non-empty --reason".into())
    } else {
        Ok(text.into())
    }
}
