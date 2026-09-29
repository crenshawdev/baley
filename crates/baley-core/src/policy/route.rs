//! Route resolution: one role's model and rung for one dispatch, from the
//! effective policy, the attempt and the host's catalog and rung map
//! (design 0003 section 6, CFG-R13 to CFG-R17).

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use super::merge::{EffectivePolicy, Layer, Source};
use super::schema::{Host, Role, Rung};

/// The code of a stored model name the host's catalog does not accept.
pub const UNKNOWN_MODEL: &str = "unknown-model";

/// The model names a host accepts and the catalog version they come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedNames {
    /// Every accepted name.
    pub names: BTreeSet<String>,
    /// The catalog version the names were read at.
    pub version: u64,
}

/// A host's own effort value for each of Baley's rungs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RungMap {
    efforts: [String; 5],
}
impl RungMap {
    /// The host's values in `Rung::ALL` order, lowest first.
    pub fn new(efforts: [String; 5]) -> RungMap {
        RungMap { efforts }
    }

    /// The host's value for `rung`.
    pub fn host_effort(&self, rung: Rung) -> &str {
        &self.efforts[rung as usize]
    }
}

/// Everything one route is resolved from.
#[derive(Debug, Clone)]
pub struct RouteRequest<'a> {
    /// The effective policy, built for `host`.
    pub policy: &'a EffectivePolicy,
    /// The version of that policy, which the route carries.
    pub policy_version: u64,
    /// The role dispatched.
    pub role: Role,
    /// The host the work order goes to.
    pub host: Host,
    /// The attempt number, 1 for the first run.
    pub attempt: NonZeroU32,
    /// The host's accepted model names.
    pub catalog: &'a AcceptedNames,
    /// The host's rung map.
    pub rungs: &'a RungMap,
}

/// The setting, layer and file that decided part of a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingSource {
    /// The setting's name, such as `roles.planner.effort`.
    pub setting: String,
    /// The layer its value came from.
    pub layer: Layer,
    /// The file, `None` for a default.
    pub file: Option<PathBuf>,
}

/// How one role runs for one dispatch (design 0003 section 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// The role.
    pub role: Role,
    /// The model passed to the host, `None` for the session's own.
    pub model: Option<String>,
    /// The stored rung.
    pub starting_rung: Rung,
    /// The rung run, after any escalation.
    pub rung: Rung,
    /// The attempt number.
    pub attempt: u32,
    /// Whether the rung moved.
    pub escalated: bool,
    /// The host's own effort value for `rung`.
    pub host_effort: String,
    /// Where the starting rung came from.
    pub effort_source: SettingSource,
    /// Where the model came from.
    pub model_source: SettingSource,
    /// The policy version in force.
    pub policy_version: u64,
    /// The catalog version the model was checked against.
    pub catalog_version: u64,
    /// One plain sentence per decision.
    pub reasons: Vec<String>,
}

/// Why no route could be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteRefusal {
    /// The stored model name is not in the host's catalog (CFG-R14).
    UnknownModel {
        /// The setting that holds it.
        setting: String,
        /// The stored name.
        model: String,
        /// The layer it came from.
        layer: Layer,
        /// The file it came from.
        file: Option<PathBuf>,
        /// The host whose catalog refused it.
        host: Host,
        /// Every name the catalog accepts, sorted.
        accepted: Vec<String>,
    },
}
impl RouteRefusal {
    /// The stable refusal code.
    pub fn code(&self) -> &'static str {
        match self {
            RouteRefusal::UnknownModel { .. } => UNKNOWN_MODEL,
        }
    }
}
impl fmt::Display for RouteRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteRefusal::UnknownModel {
                setting,
                model,
                layer,
                file,
                host,
                accepted,
            } => {
                let accepted = if accepted.is_empty() {
                    "none".to_owned()
                } else {
                    accepted.join(", ")
                };
                write!(
                    f,
                    "{}: {setting} is {model} from {}, which the {} catalog does not accept; accepted: {accepted}",
                    self.code(),
                    place(*layer, file.as_deref(), *host),
                    host.name()
                )
            }
        }
    }
}

/// Where a value came from, in words.
fn place(layer: Layer, file: Option<&Path>, host: Host) -> String {
    let path = file.map_or_else(String::new, |path| path.display().to_string());
    match layer {
        Layer::Default => "the built-in default".to_owned(),
        Layer::Global => format!("the global file {path}"),
        Layer::GlobalHost => format!("the global file's {} section {path}", host.name()),
        Layer::Project => format!("the project file {path}"),
        Layer::ProjectHost => format!("the project file's {} section {path}", host.name()),
    }
}

fn setting_source(setting: String, source: &Source) -> SettingSource {
    SettingSource {
        setting,
        layer: source.layer,
        file: source.file.as_ref().map(|file| file.path.clone()),
    }
}

/// Resolves one route, or refuses a stored model the host does not accept.
///
/// The rung is the role's stored effort, one rung up on any attempt after
/// the first when `escalate_on_failure` is true, capped at `max` and held
/// there (CFG-R16).
pub fn resolve_route(request: &RouteRequest<'_>) -> Result<Route, RouteRefusal> {
    let RouteRequest {
        policy,
        policy_version,
        role,
        host,
        attempt,
        catalog,
        rungs,
    } = *request;
    let (model, model_from) = policy.model(role);
    let (starting, effort_from) = policy.effort(role);
    let (escalate, escalate_from) = policy.escalate_on_failure();
    let where_from = |source: &Source| {
        let file = source.file.as_ref().map(|file| file.path.as_path());
        place(source.layer, file, host)
    };
    let model_setting = format!("roles.{}.model", role.name());
    let effort_setting = format!("roles.{}.effort", role.name());
    if let Some(name) = model
        && !catalog.names.contains(name)
    {
        return Err(RouteRefusal::UnknownModel {
            setting: model_setting,
            model: name.to_owned(),
            layer: model_from.layer,
            file: model_from.file.as_ref().map(|file| file.path.clone()),
            host,
            accepted: catalog.names.iter().cloned().collect(),
        });
    }
    let attempt = attempt.get();
    let rung = if escalate && attempt > 1 {
        starting.up()
    } else {
        starting
    };
    let escalated = rung != starting;
    let host_effort = rungs.host_effort(rung).to_owned();

    let mut reasons = vec![format!(
        "{effort_setting} is {} from {}",
        starting.name(),
        where_from(effort_from)
    )];
    reasons.push(match (model, model_from.layer) {
        (Some(name), _) => format!("{model_setting} is {name} from {}", where_from(model_from)),
        (None, Layer::Default) => {
            format!("{model_setting} is not set; the host session's model is used")
        }
        (None, _) => format!(
            "{model_setting} is not set from {}; the host session's model is used",
            where_from(model_from)
        ),
    });
    let switch = format!(
        "attempt {attempt} with escalate_on_failure {escalate} from {}",
        where_from(escalate_from)
    );
    reasons.push(if attempt == 1 {
        "attempt 1 runs at the starting rung".to_owned()
    } else if !escalate {
        format!("{switch}: {} kept", starting.name())
    } else if !escalated {
        format!("{switch}: {} is the top rung, held there", starting.name())
    } else if attempt == 2 {
        format!(
            "{switch}: one rung up from {} to {}",
            starting.name(),
            rung.name()
        )
    } else {
        format!(
            "{switch}: held at {}, one rung above {}",
            rung.name(),
            starting.name()
        )
    });
    reasons.push(format!(
        "{} runs rung {} as {host_effort}",
        host.name(),
        rung.name()
    ));

    Ok(Route {
        role,
        model: model.map(str::to_owned),
        starting_rung: starting,
        rung,
        attempt,
        escalated,
        host_effort,
        effort_source: setting_source(effort_setting, effort_from),
        model_source: setting_source(model_setting, model_from),
        policy_version,
        catalog_version: catalog.version,
        reasons,
    })
}
