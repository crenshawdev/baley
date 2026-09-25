//! The event type registry: which types this binary knows, at which payload
//! version, and how an older version is read (design 0001, Events; EVD-R19).
//!
//! Old events are never rewritten. An upcaster turns a stored payload of
//! version v into the shape of version v+1, in memory, at read time. An
//! event whose type or version the binary does not know makes its project
//! read-only for this binary, and only that project.

use std::collections::{BTreeMap, btree_map::Entry};
use std::fmt;

use serde_json::Value;

use baley_store::{Event, EventSchema, ProjectId};

/// Why an upcaster could not read a stored payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpcastError(pub String);

/// Turns a payload of one version into the shape of the next.
pub type Upcaster = fn(Value) -> Result<Value, UpcastError>;

/// One event type's current version and the way up from each older one.
struct TypeSpec {
    current: u32,
    /// Keyed by the version the upcaster reads; it writes that version + 1.
    upcasters: BTreeMap<u32, Upcaster>,
}

/// Why a type could not be registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Registered twice.
    Duplicate { type_name: String },
    /// Versions start at 1.
    ZeroVersion { type_name: String },
    /// Some version below the current one has no way up.
    MissingUpcaster { type_name: String, from: u32 },
    /// An upcaster for the current version or beyond.
    StrayUpcaster { type_name: String, from: u32 },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { type_name } => write!(f, "{type_name} is registered twice"),
            Self::ZeroVersion { type_name } => write!(f, "{type_name}: versions start at 1"),
            Self::MissingUpcaster { type_name, from } => {
                write!(f, "{type_name} has no upcaster from version {from}")
            }
            Self::StrayUpcaster { type_name, from } => {
                write!(
                    f,
                    "{type_name} has an upcaster from version {from}, at or past its current version"
                )
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Why a project is read-only for this binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceReason {
    UnknownType {
        type_name: String,
    },
    /// A version this binary has no reading for: newer than its current
    /// one, or zero.
    UnknownVersion {
        type_name: String,
        version: u32,
        current: u32,
    },
    /// A known older version whose upcaster refused the stored payload.
    UpcastFailed {
        type_name: String,
        from: u32,
        error: UpcastError,
    },
}

/// A project this binary must not write to, and the event that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    pub project_id: ProjectId,
    pub seq: u64,
    pub reason: FenceReason,
}

impl fmt::Display for Fence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "project {} is read-only for this binary: event {} ",
            self.project_id.0, self.seq
        )?;
        match &self.reason {
            FenceReason::UnknownType { type_name } => write!(f, "has the unknown type {type_name}"),
            FenceReason::UnknownVersion {
                type_name,
                version,
                current,
            } => {
                write!(
                    f,
                    "is {type_name} version {version}; this binary reads up to {current}"
                )
            }
            FenceReason::UpcastFailed {
                type_name,
                from,
                error,
            } => {
                write!(
                    f,
                    "is {type_name} version {from} and could not be read: {}",
                    error.0
                )
            }
        }
    }
}

/// A stored event's payload in the current shape of its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub type_name: String,
    pub version: u32,
    pub payload: Value,
}

/// The types this binary knows.
#[derive(Default)]
pub struct Registry {
    types: BTreeMap<String, TypeSpec>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares `type_name` at `current`, with one upcaster for every
    /// version from 1 to `current - 1`, each keyed by the version it reads.
    pub fn register(
        &mut self,
        type_name: &str,
        current: u32,
        upcasters: impl IntoIterator<Item = (u32, Upcaster)>,
    ) -> Result<(), RegistryError> {
        let name = || type_name.to_owned();
        if current == 0 {
            return Err(RegistryError::ZeroVersion { type_name: name() });
        }
        let upcasters: BTreeMap<u32, Upcaster> = upcasters.into_iter().collect();
        for from in 1..current {
            if !upcasters.contains_key(&from) {
                return Err(RegistryError::MissingUpcaster {
                    type_name: name(),
                    from,
                });
            }
        }
        if let Some(&from) = upcasters.keys().find(|&&from| from == 0 || from >= current) {
            return Err(RegistryError::StrayUpcaster {
                type_name: name(),
                from,
            });
        }
        match self.types.entry(type_name.to_owned()) {
            Entry::Occupied(_) => Err(RegistryError::Duplicate { type_name: name() }),
            Entry::Vacant(slot) => {
                slot.insert(TypeSpec { current, upcasters });
                Ok(())
            }
        }
    }

    /// The current version of a known type.
    pub fn current_version(&self, type_name: &str) -> Option<u32> {
        self.types.get(type_name).map(|spec| spec.current)
    }

    /// Reads a stored event's payload in its type's current shape. The
    /// event itself is untouched: the ledger keeps what was recorded. A
    /// type or version this binary cannot read is a [`Fence`] on the
    /// event's project.
    pub fn read(&self, event: &Event) -> Result<Current, Fence> {
        let fence = |reason| Fence {
            project_id: event.project_id.clone(),
            seq: event.seq,
            reason,
        };
        let Some(spec) = self.types.get(&event.type_name) else {
            return Err(fence(FenceReason::UnknownType {
                type_name: event.type_name.clone(),
            }));
        };
        if event.type_version == 0 || event.type_version > spec.current {
            return Err(fence(FenceReason::UnknownVersion {
                type_name: event.type_name.clone(),
                version: event.type_version,
                current: spec.current,
            }));
        }
        let mut payload = event.payload.clone();
        for from in event.type_version..spec.current {
            let upcaster = spec
                .upcasters
                .get(&from)
                .expect("register checked every version");
            payload = upcaster(payload).map_err(|error| {
                fence(FenceReason::UpcastFailed {
                    type_name: event.type_name.clone(),
                    from,
                    error,
                })
            })?;
        }
        Ok(Current {
            type_name: event.type_name.clone(),
            version: spec.current,
            payload,
        })
    }

    /// The projects among `events` this binary must not write to, each with
    /// its lowest-sequence event that fences it, in whatever order the
    /// events arrive. Projects whose every event reads are absent.
    pub fn fenced_projects<'a>(
        &self,
        events: impl IntoIterator<Item = &'a Event>,
    ) -> BTreeMap<ProjectId, Fence> {
        let mut fences: BTreeMap<ProjectId, Fence> = BTreeMap::new();
        for event in events {
            if fences
                .get(&event.project_id)
                .is_some_and(|fence| fence.seq <= event.seq)
            {
                continue;
            }
            if let Err(fence) = self.read(event) {
                fences.insert(event.project_id.clone(), fence);
            }
        }
        fences
    }
}

/// The store fences a project holding an event this binary cannot read:
/// an unknown type, version zero, or a version newer than the registered
/// one. Older versions read through their upcasters.
impl EventSchema for Registry {
    fn reads(&self, type_name: &str, version: u32) -> bool {
        self.current_version(type_name)
            .is_some_and(|current| (1..=current).contains(&version))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use baley_store::{Actor, Hash, RequestId};

    fn event(project: &str, seq: u64, type_name: &str, type_version: u32, payload: Value) -> Event {
        Event {
            project_id: ProjectId(project.into()),
            seq,
            stream: "phase/1".into(),
            stream_version: seq,
            type_name: type_name.into(),
            type_version,
            actor: Actor::Owner,
            recorded_at: "2026-09-25T18:00:00Z".into(),
            request_id: RequestId("00000000-0000-4000-8000-000000000001".into()),
            git: None,
            policy_version: 1,
            payload,
            prev_hash: None,
            hash: Hash([0; 32]),
        }
    }

    // Version 1 of phase.declared carried `name`; version 2 calls it
    // `title` and adds `slug`.
    fn v1_to_v2(mut payload: Value) -> Result<Value, UpcastError> {
        let object = payload
            .as_object_mut()
            .ok_or(UpcastError("payload is not an object".into()))?;
        let name = object.remove("name").ok_or(UpcastError("no name".into()))?;
        let slug = name
            .as_str()
            .map(|name| name.to_lowercase().replace(' ', "-"));
        object.insert("title".into(), name);
        object.insert("slug".into(), slug.map_or(Value::Null, Value::String));
        Ok(payload)
    }

    fn v2_to_v3(mut payload: Value) -> Result<Value, UpcastError> {
        let object = payload
            .as_object_mut()
            .ok_or(UpcastError("payload is not an object".into()))?;
        object.insert("goal".into(), Value::Null);
        Ok(payload)
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry
            .register(
                "phase.declared",
                3,
                [(1, v1_to_v2 as Upcaster), (2, v2_to_v3)],
            )
            .expect("register");
        registry
            .register("phase.completed", 1, [])
            .expect("register");
        registry
    }

    // A version-1 payload passes through both upcasters and comes out in
    // version 3's shape, while the stored event is exactly as it was.
    // Catches an upcaster chain that skips a step or a read that rewrites
    // the event.
    #[test]
    fn an_old_payload_reads_in_the_current_shape_and_the_event_is_untouched() {
        let stored = event(
            "p",
            1,
            "phase.declared",
            1,
            json!({"phase": 1, "name": "The Store"}),
        );
        let before = stored.clone();
        let current = registry().read(&stored).expect("reads");
        assert_eq!(
            current,
            Current {
                type_name: "phase.declared".into(),
                version: 3,
                payload: json!({"phase": 1, "title": "The Store", "slug": "the-store", "goal": null}),
            }
        );
        assert_eq!(stored, before);
        assert_eq!(stored.type_version, 1);
    }

    // A payload already at the current version passes through no upcaster.
    // Catches a read that always starts from version 1.
    #[test]
    fn a_current_payload_reads_as_it_is() {
        let stored = event(
            "p",
            1,
            "phase.declared",
            3,
            json!({"phase": 2, "title": "Identity", "slug": "identity", "goal": "x"}),
        );
        let current = registry().read(&stored).expect("reads");
        assert_eq!(current.payload, stored.payload);
        assert_eq!(current.version, 3);
    }

    // An unknown type fences the event's project and names the event.
    // Catches a fence without a location.
    #[test]
    fn an_unknown_type_fences_its_project() {
        let stored = event("p", 7, "phase.forgotten", 1, json!({}));
        assert_eq!(
            registry().read(&stored),
            Err(Fence {
                project_id: ProjectId("p".into()),
                seq: 7,
                reason: FenceReason::UnknownType {
                    type_name: "phase.forgotten".into()
                },
            })
        );
    }

    // A version newer than this binary's fences the project: a newer
    // binary wrote it. Catches a read that upcasts "downwards" or reads the
    // payload as is.
    #[test]
    fn a_newer_version_fences_its_project() {
        let stored = event("p", 2, "phase.declared", 4, json!({}));
        assert_eq!(
            registry().read(&stored).unwrap_err().reason,
            FenceReason::UnknownVersion {
                type_name: "phase.declared".into(),
                version: 4,
                current: 3
            }
        );
    }

    // An upcaster that cannot read what was stored fences the project
    // rather than inventing a payload. Catches an upcaster error swallowed
    // into a default.
    #[test]
    fn a_refused_upcast_fences_its_project() {
        let stored = event("p", 3, "phase.declared", 1, json!({"phase": 1}));
        assert_eq!(
            registry().read(&stored).unwrap_err().reason,
            FenceReason::UpcastFailed {
                type_name: "phase.declared".into(),
                from: 1,
                error: UpcastError("no name".into())
            }
        );
    }

    // Over events from two projects, an unknown type in one leaves the
    // other writable. Catches a fence that stops every project.
    #[test]
    fn an_unknown_type_fences_only_its_own_project() {
        let events = [
            event("alpha", 1, "phase.declared", 3, json!({})),
            event("beta", 1, "phase.declared", 3, json!({})),
            event("beta", 2, "phase.forgotten", 1, json!({})),
            event("alpha", 2, "phase.completed", 1, json!({})),
            event("beta", 3, "phase.other", 1, json!({})),
        ];
        let fences = registry().fenced_projects(&events);
        assert_eq!(
            fences.keys().collect::<Vec<_>>(),
            [&ProjectId("beta".into())]
        );
    }

    // Fed a project's unreadable events highest sequence first, the fence
    // still names the lowest. Catches a fence that keeps whichever
    // offending event arrived first.
    #[test]
    fn the_fence_names_the_lowest_offending_sequence() {
        let events = [
            event("beta", 3, "phase.other", 1, json!({})),
            event("beta", 2, "phase.forgotten", 1, json!({})),
        ];
        let fences = registry().fenced_projects(&events);
        let fence = &fences[&ProjectId("beta".into())];
        assert_eq!(fence.seq, 2);
        assert_eq!(
            fence.reason,
            FenceReason::UnknownType {
                type_name: "phase.forgotten".into()
            }
        );
    }

    // A type whose upcasters skip a version is refused at registration.
    // Catches a registry that lets `read` reach a missing upcaster.
    #[test]
    fn registration_refuses_a_gap_in_the_upcasters() {
        assert_eq!(
            Registry::new().register("a", 3, [(1, v1_to_v2 as Upcaster)]),
            Err(RegistryError::MissingUpcaster {
                type_name: "a".into(),
                from: 2
            })
        );
    }

    // An upcaster from the current version or beyond has nowhere to go.
    // Catches a ladder that reads past the current shape.
    #[test]
    fn registration_refuses_an_upcaster_at_the_current_version() {
        assert_eq!(
            Registry::new().register("a", 1, [(1, v1_to_v2 as Upcaster)]),
            Err(RegistryError::StrayUpcaster {
                type_name: "a".into(),
                from: 1
            })
        );
    }

    // Versions start at 1. Catches a type registered at a version no
    // event can carry.
    #[test]
    fn registration_refuses_version_zero() {
        assert_eq!(
            Registry::new().register("a", 0, []),
            Err(RegistryError::ZeroVersion {
                type_name: "a".into()
            })
        );
    }

    // A type registered twice is refused and the first registration
    // stands. Catches a second registration silently replacing the ladder.
    #[test]
    fn registration_refuses_a_repeat() {
        let mut registry = Registry::new();
        registry
            .register("a", 2, [(1, v1_to_v2 as Upcaster)])
            .expect("first");
        assert_eq!(
            registry.register("a", 1, []),
            Err(RegistryError::Duplicate {
                type_name: "a".into()
            })
        );
        assert_eq!(registry.current_version("a"), Some(2));
    }

    // A registered type reports its current version and an unregistered
    // one reports none. Catches a lookup that defaults to version 1.
    #[test]
    fn current_version_is_known_only_for_registered_types() {
        let registry = registry();
        assert_eq!(registry.current_version("phase.declared"), Some(3));
        assert_eq!(registry.current_version("phase.forgotten"), None);
    }

    // The schema the store fences with reads every version from 1 to the
    // current one and nothing else, agreeing with `read`. Catches a check
    // that admits a newer version, version zero or an unknown type, which
    // would let this binary write to a project it cannot read.
    #[test]
    fn the_store_schema_reads_exactly_the_registered_versions() {
        let registry = registry();
        assert!(registry.reads("phase.declared", 1));
        assert!(registry.reads("phase.declared", 3));
        assert!(!registry.reads("phase.declared", 4));
        assert!(!registry.reads("phase.declared", 0));
        assert!(!registry.reads("phase.forgotten", 1));
    }
}
