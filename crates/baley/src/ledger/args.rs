//! Arguments for owner operations on the ledger.
use baley_store::Hash;
use clap::{ArgGroup, Subcommand};
use std::path::PathBuf;

/// Ledger commands, beside the inherited command surface.
#[derive(Debug, Clone, Subcommand)]
pub enum LedgerCommand {
    /// Check the chain against a remote, locally, or compare replayed views.
    #[command(group(ArgGroup::new("check").required(true).args(["remote", "local_only", "views"])))]
    Verify {
        /// Ledger project id.
        project: String,
        /// Configured git remote.
        #[arg(long)]
        remote: Option<String>,
        /// Check without a remote witness.
        #[arg(long)]
        local_only: bool,
        /// Compare views with a replay.
        #[arg(long)]
        views: bool,
    },
    /// Report the health of every project.
    Doctor {
        /// Project and configured remote, PROJECT=REMOTE.
        #[arg(long, value_parser = project_remote)]
        remote: Vec<(String, String)>,
        /// Project to check without a remote witness.
        #[arg(long)]
        local_only: Vec<String>,
    },
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
    /// Accept a restored chain behind its remote anchor.
    AcknowledgeRestore {
        /// Ledger project id.
        project: String,
        /// Configured git remote.
        #[arg(long)]
        remote: String,
    },
}

fn payload_hash(text: &str) -> Result<Hash, String> {
    Hash::from_hex(text).ok_or_else(|| "a payload hash is 64 hex digits".into())
}
fn project_remote(text: &str) -> Result<(String, String), String> {
    text.split_once('=')
        .filter(|(p, r)| !p.is_empty() && !r.is_empty())
        .map(|(p, r)| (p.into(), r.into()))
        .ok_or_else(|| "expected PROJECT=REMOTE".into())
}
fn purge_reason(text: &str) -> Result<String, String> {
    if text.is_empty() {
        Err("a purge needs a non-empty --reason".into())
    } else {
        Ok(text.into())
    }
}
