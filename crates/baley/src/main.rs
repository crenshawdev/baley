//! The `baley` binary. In Build 1 it is the command-line surface over the
//! evidence ledger; the MCP server and the guard move here in later slices.
use clap::Parser;

/// Baley keeps the evidence of AI-assisted work in one hash-chained ledger.
#[derive(Parser)]
#[command(name = "baley", version)]
struct Cli {}

fn main() {
    Cli::parse();
}
