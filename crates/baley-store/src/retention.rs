//! Store-owned retention events and the excerpt range rule.

use std::ops::Range;

use serde_json::{Value, json};

use crate::{Hash, PayloadRef};

/// The reduction event type.
pub const PAYLOAD_REDUCED: &str = "payload.reduced";
/// The reduction event version.
pub const PAYLOAD_REDUCED_VERSION: u32 = 1;
/// The purge event type.
pub const PAYLOAD_PURGED: &str = "payload.purged";
/// The purge event version.
pub const PAYLOAD_PURGED_VERSION: u32 = 1;
/// The stream for store-owned retention events.
pub const RETENTION_STREAM: &str = "retention";
/// Bytes kept at each edge of a reduced output.
pub const EXCERPT_EDGE: u64 = 65_536;

/// The two disjoint ranges retained when an excerpt saves space.
pub fn kept_ranges(bytes: u64) -> Option<[Range<u64>; 2]> {
    (bytes > EXCERPT_EDGE * 2).then(|| [0..EXCERPT_EDGE, bytes - EXCERPT_EDGE..bytes])
}

/// The data carried by `payload.reduced`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReducedEvent {
    /// The sequence of the original reference's event.
    pub seq: u64,
    /// The original body's hash.
    pub original: Hash,
    /// The original body's decoded length.
    pub original_bytes: u64,
    /// The excerpt attached to this event.
    pub excerpt: PayloadRef,
    /// The byte ranges retained in the excerpt.
    pub kept: [Range<u64>; 2],
}

impl ReducedEvent {
    /// Builds the event payload with the excerpt as its only reference object.
    pub fn to_value(&self) -> Value {
        json!({"reference": {"seq": self.seq, "hash": self.original.to_hex()},
            "original_bytes": self.original_bytes, "excerpt": self.excerpt.to_value(),
            "kept": [[self.kept[0].start, self.kept[0].end],
                     [self.kept[1].start, self.kept[1].end]]})
    }

    /// Reads a reduction event payload.
    pub fn from_value(value: &Value) -> Option<Self> {
        let reference = value.get("reference")?;
        let pair = |index: usize| -> Option<Range<u64>> {
            let values = value.get("kept")?.as_array()?.get(index)?.as_array()?;
            if values.len() != 2 {
                return None;
            }
            Some(values[0].as_u64()?..values[1].as_u64()?)
        };
        Some(Self {
            seq: reference.get("seq")?.as_u64()?,
            original: Hash::from_hex(reference.get("hash")?.as_str()?)?,
            original_bytes: value.get("original_bytes")?.as_u64()?,
            excerpt: PayloadRef::from_value(value.get("excerpt")?)?,
            kept: [pair(0)?, pair(1)?],
        })
    }
}

/// The data carried by `payload.purged`; hash lists and released pairs are sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgedEvent {
    /// The hashes named by the command.
    pub requested: Vec<Hash>,
    /// Source sequence and hash of each reference this project released.
    pub released: Vec<(u64, Hash)>,
    /// Hashes whose bodies this purge removed.
    pub removed: Vec<Hash>,
    /// Released hashes still required, plus requested originals with shared excerpts.
    pub shared: Vec<Hash>,
    /// The reason kept in each purge tombstone.
    pub reason: String,
}

impl PurgedEvent {
    /// Builds the purge event payload without payload reference objects.
    pub fn to_value(&self) -> Value {
        let hex = |hashes: &[Hash]| hashes.iter().map(|hash| hash.to_hex()).collect::<Vec<_>>();
        let released = self
            .released
            .iter()
            .map(|(seq, hash)| json!([seq, hash.to_hex()]))
            .collect::<Vec<_>>();
        json!({"requested": hex(&self.requested), "released": released,
            "removed": hex(&self.removed), "shared": hex(&self.shared), "reason": self.reason})
    }

    /// Reads a purge event payload.
    pub fn from_value(value: &Value) -> Option<Self> {
        let hashes = |name: &str| -> Option<Vec<Hash>> {
            value
                .get(name)?
                .as_array()?
                .iter()
                .map(|item| Hash::from_hex(item.as_str()?))
                .collect()
        };
        Some(Self {
            requested: hashes("requested")?,
            released: value
                .get("released")?
                .as_array()?
                .iter()
                .map(|item| {
                    let pair = item.as_array()?;
                    if pair.len() != 2 {
                        return None;
                    }
                    Some((pair[0].as_u64()?, Hash::from_hex(pair[1].as_str()?)?))
                })
                .collect::<Option<Vec<_>>>()?,
            removed: hashes("removed")?,
            shared: hashes("shared")?,
            reason: value.get("reason")?.as_str()?.to_owned(),
        })
    }
}
