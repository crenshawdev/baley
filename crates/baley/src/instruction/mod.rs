//! The compiled instruction registry: every instruction Baley serves to a
//! model, found by identity. The table is static data inside the binary, with
//! no disk loader and no override, so the text a model reads is the text that
//! was built and reviewed, and its version and hash can be recorded as evidence.
//!
//! An identity is a short name, the same one the help table gives the command,
//! and is never a path. A front door whose text is not built yet is registered
//! as unavailable with the build that owns it, so a caller learns where it
//! went instead of reading a stand-in.

use baley_store::InstructionEvidence;
use serde_json::{Value, json};

use crate::envelope::Refusal;
use crate::help::front_door;

pub mod capture;
pub mod read_contract;

/// The served text of one instruction, pinned in source.
#[derive(Debug, PartialEq, Eq)]
pub struct Text {
    /// Bumped by hand whenever the body changes, so the ledger can tell texts apart.
    pub version: &'static str,
    /// Lowercase hex SHA-256 of exactly the bytes served, written as a literal.
    pub hash: &'static str,
    /// The text a model reads.
    pub body: &'static str,
}

/// One registered instruction.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    /// The short name a caller asks for.
    pub identity: &'static str,
    /// The build that owns this instruction, whether or not its text exists yet.
    pub build: u32,
    /// The served text, or none while the owning build has not added it.
    pub text: Option<Text>,
}

/// What the registry holds for one identity.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    /// The instruction has a text and is served.
    Served {
        /// The registered entry.
        entry: &'static Entry,
        /// Its pinned text.
        text: &'static Text,
    },
    /// The identity is registered, but its text is not built yet.
    Unavailable {
        /// The registered identity.
        identity: &'static str,
        /// The build that adds the text.
        build: u32,
    },
    /// No entry has this identity.
    Unknown,
}

/// Every instruction, in the help table's order. A build adds its texts here,
/// beside the operations it ships, by giving the entry a `text`.
pub static ENTRIES: &[Entry] = &[
    // Build 4 adds this text beside its operations.
    Entry {
        identity: "bal-context",
        build: 4,
        text: None,
    },
    // Build 4 adds this text beside its operations.
    Entry {
        identity: "bal-plan",
        build: 4,
        text: None,
    },
    // Build 5 adds this text beside its operations.
    Entry {
        identity: "bal-execute",
        build: 5,
        text: None,
    },
    // Build 5 adds this text beside its operations.
    Entry {
        identity: "bal-verify",
        build: 5,
        text: None,
    },
    // Build 7 adds this text beside its operations.
    Entry {
        identity: "bal-progress",
        build: 7,
        text: None,
    },
    // Build 8 adds this text beside its operations.
    Entry {
        identity: "bal-task",
        build: 8,
        text: None,
    },
    // Build 4 adds this text beside its operations.
    Entry {
        identity: "bal-review",
        build: 4,
        text: None,
    },
    // Build 4 adds this text beside its operations.
    Entry {
        identity: "bal-plan-review",
        build: 4,
        text: None,
    },
    // Build 4 adds this text beside its operations.
    Entry {
        identity: "bal-decision-review",
        build: 4,
        text: None,
    },
    // Build 4 adds this text beside its operations.
    Entry {
        identity: "bal-minimalism-review",
        build: 4,
        text: None,
    },
    // Build 8 adds this text beside its operations.
    Entry {
        identity: "bal-debug",
        build: 8,
        text: None,
    },
    // Build 5 adds this text beside its operations.
    Entry {
        identity: "bal-coverage",
        build: 5,
        text: None,
    },
    // Build 5 adds this text beside its operations.
    Entry {
        identity: "bal-audit",
        build: 5,
        text: None,
    },
    // Build 6 adds this text beside its operations.
    Entry {
        identity: "bal-land",
        build: 6,
        text: None,
    },
    // Build 6 adds this text beside its operations.
    Entry {
        identity: "bal-milestone",
        build: 6,
        text: None,
    },
    // Build 6 adds this text beside its operations.
    Entry {
        identity: "bal-undo",
        build: 6,
        text: None,
    },
    // Build 3 serves the capture text beside the `capture` operation it drives.
    Entry {
        identity: capture::IDENTITY,
        build: 3,
        text: Some(Text {
            version: capture::VERSION,
            hash: capture::HASH,
            body: capture::TEXT,
        }),
    },
    // Build 3 serves the help front door, whose words the help area owns.
    Entry {
        identity: front_door::IDENTITY,
        build: 3,
        text: Some(Text {
            version: front_door::VERSION,
            hash: front_door::HASH,
            body: front_door::TEXT,
        }),
    },
    // Build 8 adds this text beside its operations.
    Entry {
        identity: "bal-spike",
        build: 8,
        text: None,
    },
    // Build 7 adds this text beside its operations.
    Entry {
        identity: "bal-suggest",
        build: 7,
        text: None,
    },
    // Build 8 adds this text beside its operations.
    Entry {
        identity: "bal-why",
        build: 8,
        text: None,
    },
    // Build 3 serves the read contract, which belongs to the host interface.
    Entry {
        identity: read_contract::IDENTITY,
        build: 3,
        text: Some(Text {
            version: read_contract::VERSION,
            hash: read_contract::HASH,
            body: read_contract::TEXT,
        }),
    },
];

/// Looks an identity up by exact match. The identity is text compared with the
/// table, never a path, and nothing is read from disk or the environment.
pub fn lookup(identity: &str) -> Lookup {
    match ENTRIES.iter().find(|entry| entry.identity == identity) {
        Some(
            entry @ Entry {
                text: Some(text), ..
            },
        ) => Lookup::Served { entry, text },
        Some(entry) => Lookup::Unavailable {
            identity: entry.identity,
            build: entry.build,
        },
        None => Lookup::Unknown,
    }
}

/// Turns the identity a session sent into the evidence a write records, or the
/// refusal a lookup gives. The version and hash come from the registry alone, so
/// the ledger never records a claim the binary did not serve.
///
/// `capture` attaches the evidence to the caller it prepares under, so every
/// event of that call carries it. The gate's caller carries none.
pub fn evidence(identity: &str) -> Result<InstructionEvidence, Value> {
    match lookup(identity) {
        Lookup::Served { entry, text } => Ok(InstructionEvidence::new(
            entry.identity,
            text.version,
            text.hash,
        )
        .expect("a compiled entry fits the caller record's limits, and a test checks every one")),
        Lookup::Unavailable { identity, build } => Err(unavailable(identity, build)),
        Lookup::Unknown => Err(unknown()),
    }
}

/// The refusal for a registered identity whose text a later build adds.
pub fn unavailable(identity: &str, build: u32) -> Value {
    Refusal::new(
        "instruction-unavailable",
        format!("instruction `{identity}` is not served by this server, Build {build} adds it"),
    )
    .slot("identity")
    .details(json!({ "build": build }))
    .value()
}

/// The refusal for text that is no registered identity. It echoes none of the
/// text, so a long or odd identity cannot grow the answer.
pub fn unknown() -> Value {
    Refusal::new(
        "unknown-instruction",
        "no instruction has that identity, ask help for the names",
    )
    .slot("identity")
    .value()
}

#[cfg(test)]
mod tests;
