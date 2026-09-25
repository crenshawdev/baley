//! The event: an envelope, a payload and its place in a project's chain
//! (design 0001, Events).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::canonical::{CanonicalError, canonical_json};
use crate::chain::chain_hash;

/// The project an event belongs to: a UUID v4 in its text form. An identity,
/// not an authorization (design 0001).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(pub String);

/// The command that recorded an event: the caller's fresh UUID (EVD-R6).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub String);

/// A SHA-256 digest. Written as 64 lower-case hex digits wherever it is
/// text; the chain formula uses its raw 32 bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// The digest from its 64 hex digits, either case.
    pub fn from_hex(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (index, pair) in bytes.chunks(2).enumerate() {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            out[index] = (high * 16 + low) as u8;
        }
        Some(Self(out))
    }

    /// The 64 lower-case hex digits.
    pub fn to_hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.to_hex())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Hash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_hex(&text).ok_or_else(|| serde::de::Error::custom("a hash is 64 hex digits"))
    }
}

/// Why text cannot name an actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorError {
    Empty,
    /// `owner` and `baley` name the reserved actors, never an agent role.
    Reserved(String),
}

impl fmt::Display for ActorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("an actor is never empty"),
            Self::Reserved(name) => write!(f, "{name} is a reserved actor, not an agent role"),
        }
    }
}

impl std::error::Error for ActorError {}

/// An agent role such as `daneel:executor`: never empty, never a reserved
/// actor's name, so an agent can never be recorded as the owner.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentRole(String);

impl AgentRole {
    pub fn new(role: &str) -> Result<Self, ActorError> {
        match role {
            "" => Err(ActorError::Empty),
            "owner" | "baley" => Err(ActorError::Reserved(role.to_owned())),
            role => Ok(Self(role.to_owned())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who recorded an event: the owner, an agent role, or Baley itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Actor {
    Owner,
    Baley,
    Agent(AgentRole),
}

impl Actor {
    /// The text form the envelope carries.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Owner => "owner",
            Self::Baley => "baley",
            Self::Agent(role) => role.as_str(),
        }
    }

    /// The actor named by its text form.
    pub fn parse(text: &str) -> Result<Self, ActorError> {
        match text {
            "owner" => Ok(Self::Owner),
            "baley" => Ok(Self::Baley),
            role => AgentRole::new(role).map(Self::Agent),
        }
    }
}

impl Serialize for Actor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Actor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// The git facts an event depends on (EVD-R4): the commit and tree it saw,
/// and the checkout it saw them in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFacts {
    pub commit: String,
    pub tree: String,
    pub checkout: String,
}

/// An event before the ledger has placed it: everything the command decides,
/// nothing the chain decides. [`Event::seal`] adds the sequence, the previous
/// hash and the hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDraft {
    pub stream: String,
    pub stream_version: u64,
    pub type_name: String,
    pub type_version: u32,
    pub actor: Actor,
    /// UTC, RFC 3339, second or finer precision, `Z` suffix. Text because
    /// its bytes are hashed.
    pub recorded_at: String,
    pub request_id: RequestId,
    /// Absent when the event depends on no git state.
    pub git: Option<GitFacts>,
    pub policy_version: u64,
    /// Canonical-JSON-able content: integers within ±(2^53 − 1), no floats.
    pub payload: Value,
}

/// One recorded event: the envelope fields of the design's table, the
/// payload, and the chain fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub project_id: ProjectId,
    /// Project sequence, from 1, no gaps.
    pub seq: u64,
    pub stream: String,
    pub stream_version: u64,
    pub type_name: String,
    pub type_version: u32,
    pub actor: Actor,
    pub recorded_at: String,
    pub request_id: RequestId,
    pub git: Option<GitFacts>,
    pub policy_version: u64,
    pub payload: Value,
    /// `None` only at sequence 1.
    pub prev_hash: Option<Hash>,
    pub hash: Hash,
}

/// Why a draft cannot be placed in a chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealError {
    /// Sequences start at 1.
    ZeroSequence,
    /// Sequence 1 has no predecessor and every later sequence has one;
    /// this pair has it the other way round.
    Link {
        seq: u64,
        has_prev: bool,
    },
    Canonical(CanonicalError),
}

impl fmt::Display for SealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSequence => f.write_str("sequences start at 1"),
            Self::Link { seq: 1, .. } => f.write_str("sequence 1 has no predecessor"),
            Self::Link { seq, .. } => write!(f, "sequence {seq} needs its predecessor's hash"),
            Self::Canonical(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SealError {}

impl Event {
    /// Places a draft in a project's chain at `seq` after the event whose
    /// hash is `prev`, computing its hash from the design's formula. `prev`
    /// is `None` at sequence 1 and present everywhere else, or the draft is
    /// refused, so a sealed event never carries the wrong formula.
    pub fn seal(
        project_id: ProjectId,
        seq: u64,
        prev: Option<Hash>,
        draft: EventDraft,
    ) -> Result<Self, SealError> {
        if seq == 0 {
            return Err(SealError::ZeroSequence);
        }
        if (seq == 1) == prev.is_some() {
            return Err(SealError::Link {
                seq,
                has_prev: prev.is_some(),
            });
        }
        let mut event = Self {
            project_id,
            seq,
            stream: draft.stream,
            stream_version: draft.stream_version,
            type_name: draft.type_name,
            type_version: draft.type_version,
            actor: draft.actor,
            recorded_at: draft.recorded_at,
            request_id: draft.request_id,
            git: draft.git,
            policy_version: draft.policy_version,
            payload: draft.payload,
            prev_hash: prev,
            hash: Hash([0; 32]),
        };
        event.hash = event.compute_hash().map_err(SealError::Canonical)?;
        Ok(event)
    }

    /// The hash the design's formula gives this event from its own fields,
    /// ignoring the stored `hash`. Verification compares the two.
    pub fn compute_hash(&self) -> Result<Hash, CanonicalError> {
        let envelope = self.canonical_envelope()?;
        let payload = canonical_json(&self.payload)?;
        Ok(chain_hash(
            self.prev_hash.as_ref(),
            &self.project_id,
            &envelope,
            &payload,
        ))
    }

    /// The envelope as the design's table lists it, without `hash`, as a
    /// JSON object: `git` is left out when absent, `prev_hash` is `null` at
    /// sequence 1, hashes are hex text.
    pub fn envelope(&self) -> Value {
        let mut envelope = Map::new();
        envelope.insert(
            "project_id".into(),
            Value::String(self.project_id.0.clone()),
        );
        envelope.insert("seq".into(), Value::from(self.seq));
        envelope.insert("stream".into(), Value::String(self.stream.clone()));
        envelope.insert("stream_version".into(), Value::from(self.stream_version));
        envelope.insert("type".into(), Value::String(self.type_name.clone()));
        envelope.insert("type_version".into(), Value::from(self.type_version));
        envelope.insert(
            "actor".into(),
            Value::String(self.actor.as_str().to_owned()),
        );
        envelope.insert(
            "recorded_at".into(),
            Value::String(self.recorded_at.clone()),
        );
        envelope.insert(
            "request_id".into(),
            Value::String(self.request_id.0.clone()),
        );
        if let Some(git) = &self.git {
            let mut facts = Map::new();
            facts.insert("commit".into(), Value::String(git.commit.clone()));
            facts.insert("tree".into(), Value::String(git.tree.clone()));
            facts.insert("checkout".into(), Value::String(git.checkout.clone()));
            envelope.insert("git".into(), Value::Object(facts));
        }
        envelope.insert("policy_version".into(), Value::from(self.policy_version));
        envelope.insert(
            "prev_hash".into(),
            match self.prev_hash {
                Some(hash) => Value::String(hash.to_hex()),
                None => Value::Null,
            },
        );
        Value::Object(envelope)
    }

    /// The canonical bytes of [`Event::envelope`].
    pub fn canonical_envelope(&self) -> Result<Vec<u8>, CanonicalError> {
        canonical_json(&self.envelope())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A hash round-trips through its hex text, and text that is not 64 hex
    // digits is refused. Catches an odd-length or upper-case-only decoder.
    #[test]
    fn hash_hex_round_trips_and_rejects_other_text() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0xab;
        bytes[31] = 0x01;
        let hash = Hash(bytes);
        let hex = hash.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("ab00") && hex.ends_with("0001"));
        assert_eq!(Hash::from_hex(&hex), Some(hash));
        assert_eq!(Hash::from_hex(&hex.to_uppercase()), Some(hash));
        assert_eq!(Hash::from_hex(&hex[1..]), None);
        assert_eq!(Hash::from_hex(&format!("zz{}", &hex[2..])), None);
    }

    // Each actor form survives a round trip through the JSON the envelope
    // carries, landing on the variant it started as. Catches an agent role
    // read back as the owner or Baley.
    #[test]
    fn actors_round_trip_through_their_text() {
        let role = AgentRole::new("daneel:executor").expect("role");
        for (actor, text) in [
            (Actor::Owner, "\"owner\""),
            (Actor::Baley, "\"baley\""),
            (Actor::Agent(role), "\"daneel:executor\""),
        ] {
            assert_eq!(serde_json::to_string(&actor).expect("serialize"), text);
            assert_eq!(
                serde_json::from_str::<Actor>(text).expect("deserialize"),
                actor
            );
        }
    }

    // An agent role cannot take a reserved actor's name, so no agent is
    // ever recorded as the owner. Catches attribution lost at construction.
    #[test]
    fn an_agent_role_cannot_take_a_reserved_name() {
        assert_eq!(
            AgentRole::new("owner"),
            Err(ActorError::Reserved("owner".into()))
        );
        assert_eq!(
            AgentRole::new("baley"),
            Err(ActorError::Reserved("baley".into()))
        );
    }

    // Empty text names no actor, whether built as a role or read from an
    // envelope. Catches an event recorded with no one behind it.
    #[test]
    fn an_empty_actor_is_refused() {
        assert_eq!(AgentRole::new(""), Err(ActorError::Empty));
        assert_eq!(Actor::parse(""), Err(ActorError::Empty));
        assert!(serde_json::from_str::<Actor>("\"\"").is_err());
    }

    fn draft() -> EventDraft {
        EventDraft {
            stream: "phase/1".into(),
            stream_version: 1,
            type_name: "phase.declared".into(),
            type_version: 1,
            actor: Actor::Owner,
            recorded_at: "2026-09-25T18:00:00Z".into(),
            request_id: RequestId("00000000-0000-4000-8000-000000000001".into()),
            git: None,
            policy_version: 1,
            payload: serde_json::json!({}),
        }
    }

    // Sequence 1 with a predecessor, or a later sequence without one, is
    // refused before any hash is computed. Catches a sealed event hashed
    // with the other formula.
    #[test]
    fn seal_refuses_a_sequence_and_predecessor_that_disagree() {
        let project = || ProjectId("p".into());
        assert_eq!(
            Event::seal(project(), 1, Some(Hash([7; 32])), draft()),
            Err(SealError::Link {
                seq: 1,
                has_prev: true
            })
        );
        assert_eq!(
            Event::seal(project(), 2, None, draft()),
            Err(SealError::Link {
                seq: 2,
                has_prev: false
            })
        );
    }

    // Sequence 0 does not exist. Catches a chain that starts one early.
    #[test]
    fn seal_refuses_sequence_zero() {
        assert_eq!(
            Event::seal(ProjectId("p".into()), 0, None, draft()),
            Err(SealError::ZeroSequence)
        );
    }
}
