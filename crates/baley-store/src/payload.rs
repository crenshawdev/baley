//! Payloads and references (design 0001, Payloads, retention and purge;
//! EVD-R11, R14).

use std::io::Read;
use std::ops::Range;

use serde_json::{Value, json};

use crate::error::StoreError;
use crate::event::{Hash, ProjectId};

/// How long a reference keeps its body, by default (the project file may
/// change any of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RetentionClass {
    /// Plan and context text, verdict detail: the life of the project.
    Record,
    /// Test and command output: until its milestone closes, then reduced.
    Output,
    /// Review material, prompts: 90 days after its review closes.
    Material,
}

impl RetentionClass {
    /// The name events and the store write.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::Output => "output",
            Self::Material => "material",
        }
    }

    /// The class of that name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "record" => Some(Self::Record),
            "output" => Some(Self::Output),
            "material" => Some(Self::Material),
            _ => None,
        }
    }
}

/// One event's use of a payload, as the event's payload carries it:
/// `{ "payload": "<sha256>", "bytes": n, "class": "<class>" }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadRef {
    /// SHA-256 of the uncompressed bytes.
    pub hash: Hash,
    pub bytes: u64,
    pub class: RetentionClass,
}

impl PayloadRef {
    /// The reference object an event's payload carries.
    pub fn to_value(&self) -> Value {
        json!({
            "payload": self.hash.to_hex(),
            "bytes": self.bytes,
            "class": self.class.as_str(),
        })
    }

    /// The reference in a reference object, or `None` if it is not one.
    pub fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.len() != 3 {
            return None;
        }
        Some(Self {
            hash: Hash::from_hex(object.get("payload")?.as_str()?)?,
            bytes: object.get("bytes")?.as_u64()?,
            class: RetentionClass::parse(object.get("class")?.as_str()?)?,
        })
    }
}

/// One reference's identity: the event that made it and the body it
/// names. Retention decisions apply to references, never to a hash alone.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PayloadReference {
    pub project: ProjectId,
    pub seq: u64,
    pub hash: Hash,
}

/// A payload's state. Reduced and purged payloads answer with this, not
/// with an error; the hash, length and references remain and the chain
/// still verifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadStatus {
    Present {
        bytes: u64,
    },
    /// The body was replaced by an excerpt stored as its own payload.
    Reduced {
        excerpt: PayloadRef,
        /// The byte ranges of the original the excerpt keeps.
        kept: Vec<Range<u64>>,
    },
    Purged {
        reason: String,
    },
}

/// A body to read, or the state that stands in for it.
pub enum PayloadBody<'a> {
    /// The uncompressed bytes, streamed; never loaded whole.
    Present(Box<dyn Read + 'a>),
    Gone(PayloadStatus),
}

/// Reading payloads by hash. Writes go through `Transaction::put_payload`.
pub trait Payloads {
    fn open(&self, hash: &Hash) -> Result<PayloadBody<'_>, StoreError>;
    fn status(&self, hash: &Hash) -> Result<PayloadStatus, StoreError>;
}
