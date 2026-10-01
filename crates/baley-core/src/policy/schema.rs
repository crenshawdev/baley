//! The settings schema as data: each setting Baley reads, with its kind,
//! default, scope and owning design (design 0003 section 9).

use std::collections::BTreeSet;
use std::sync::OnceLock;

/// A role Baley routes (design 0003, CFG-R12), in the design's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Writes plans.
    Planner,
    /// Refines a phase: asks the owner questions and drafts truths.
    Analyzer,
    /// Checks a plan.
    Checker,
    /// Carries out a task.
    Executor,
    /// Verifies finished work.
    Verifier,
    /// Reviews plans and changes.
    Reviewer,
}
impl Role {
    /// Every role, in the design's order.
    pub const ALL: [Role; 6] = [
        Role::Planner,
        Role::Analyzer,
        Role::Checker,
        Role::Executor,
        Role::Verifier,
        Role::Reviewer,
    ];

    /// The name settings use, as in `roles.planner.effort`.
    pub fn name(self) -> &'static str {
        match self {
            Role::Planner => "planner",
            Role::Analyzer => "analyzer",
            Role::Checker => "checker",
            Role::Executor => "executor",
            Role::Verifier => "verifier",
            Role::Reviewer => "reviewer",
        }
    }

    /// The role with exactly this name, if any.
    pub fn parse(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.name() == name)
    }
}

/// One of Baley's five effort levels, ordered from least to most effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rung {
    /// The least effort.
    Low,
    /// The second rung.
    Medium,
    /// The third rung.
    High,
    /// The fourth rung.
    Xhigh,
    /// The top rung.
    Max,
}
impl Rung {
    /// Every rung, lowest first.
    pub const ALL: [Rung; 5] = [Rung::Low, Rung::Medium, Rung::High, Rung::Xhigh, Rung::Max];

    /// The name a settings file writes.
    pub fn name(self) -> &'static str {
        match self {
            Rung::Low => "low",
            Rung::Medium => "medium",
            Rung::High => "high",
            Rung::Xhigh => "xhigh",
            Rung::Max => "max",
        }
    }

    /// The rung with exactly this name, if any; case is not folded.
    pub fn parse(name: &str) -> Option<Rung> {
        Rung::ALL.into_iter().find(|rung| rung.name() == name)
    }

    /// The next rung up; `Max` stays `Max`.
    pub fn up(self) -> Rung {
        match self {
            Rung::Low => Rung::Medium,
            Rung::Medium => Rung::High,
            Rung::High => Rung::Xhigh,
            Rung::Xhigh | Rung::Max => Rung::Max,
        }
    }
}

/// A host whose section a settings file may hold (design 0012 section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Host {
    /// Claude Code.
    ClaudeCode,
    /// Codex.
    Codex,
}
impl Host {
    /// Every host Baley knows.
    pub const ALL: [Host; 2] = [Host::ClaudeCode, Host::Codex];

    /// The name in `[host.<name>]`.
    pub fn name(self) -> &'static str {
        match self {
            Host::ClaudeCode => "claude-code",
            Host::Codex => "codex",
        }
    }

    /// The host with exactly this name, if any.
    pub fn parse(name: &str) -> Option<Host> {
        Host::ALL.into_iter().find(|host| host.name() == name)
    }
}

/// Which files may set a setting (CFG-R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only the global file.
    Global,
    /// Only the project file.
    Project,
    /// Either file.
    Both,
}

/// The type or grammar a setting's value must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `true` or `false`.
    Bool,
    /// One of the five rung names.
    Rung,
    /// A non-empty string; whether a host accepts it is the catalog's question.
    ModelName,
    /// A non-empty string naming a git remote. Whether the repository has that
    /// remote is the forge's exact-name check, not the schema's.
    RemoteName,
}

/// A setting's built-in value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Default {
    /// No value: the setting is absent unless a file writes it.
    Absent,
    /// A boolean.
    Bool(bool),
    /// A rung.
    Rung(Rung),
}
impl Default {
    fn fits(self, kind: Kind) -> bool {
        matches!(
            (self, kind),
            (Default::Absent, _) | (Default::Bool(_), Kind::Bool) | (Default::Rung(_), Kind::Rung)
        )
    }
}

/// One setting in the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The dotted name, such as `roles.planner.effort`.
    pub name: String,
    /// The type or grammar of its value.
    pub kind: Kind,
    /// The value when no file writes it.
    pub default: Default,
    /// Which files may set it.
    pub scope: Scope,
    /// The design document that owns its meaning, such as `0003`.
    pub owner: &'static str,
}

/// Every setting Baley reads; a setting enters it with its first reader (CFG-R7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    entries: Vec<Entry>,
}
impl Schema {
    /// Builds a schema from program data.
    ///
    /// # Panics
    ///
    /// On a repeated name or a default that does not fit its entry's kind:
    /// the schema is compiled in, so either is a bug, not an input.
    pub fn new(entries: Vec<Entry>) -> Schema {
        let mut names = BTreeSet::new();
        for entry in &entries {
            assert!(
                names.insert(entry.name.as_str()),
                "the schema names {} twice",
                entry.name
            );
            assert!(
                entry.default.fits(entry.kind),
                "the default of {} does not fit its kind",
                entry.name
            );
        }
        Schema { entries }
    }

    /// Build 2's schema: each role's model and effort, `escalate_on_failure`
    /// and `git.remote`.
    pub fn standard() -> &'static Schema {
        static STANDARD: OnceLock<Schema> = OnceLock::new();
        STANDARD.get_or_init(|| {
            let mut entries = Vec::new();
            for role in Role::ALL {
                entries.push(Entry {
                    name: format!("roles.{}.model", role.name()),
                    kind: Kind::ModelName,
                    default: Default::Absent,
                    scope: Scope::Both,
                    owner: "0003",
                });
                let effort = match role {
                    Role::Reviewer => Rung::Medium,
                    Role::Checker => Rung::Low,
                    _ => Rung::High,
                };
                entries.push(Entry {
                    name: format!("roles.{}.effort", role.name()),
                    kind: Kind::Rung,
                    default: Default::Rung(effort),
                    scope: Scope::Both,
                    owner: "0003",
                });
            }
            entries.push(Entry {
                name: "escalate_on_failure".into(),
                kind: Kind::Bool,
                default: Default::Bool(false),
                scope: Scope::Both,
                owner: "0003",
            });
            entries.push(Entry {
                name: "git.remote".into(),
                kind: Kind::RemoteName,
                default: Default::Absent,
                scope: Scope::Project,
                owner: "0001",
            });
            Schema::new(entries)
        })
    }

    /// The entry with exactly this name, if any.
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Every entry, in the order the schema was built.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}
