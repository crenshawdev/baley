//! The plan of the ledger commands that read the checkout's settings. From
//! the facts gathered so far it gives the operations the command requests
//! next, in order, or its refusal. Pure: the wiring performs each operation
//! and asks again, so a refusal always comes before the operation after it.
use super::anchor_plan::{self, TargetRefusal};
use baley_core::policy::{CONFIG_UNAVAILABLE, EffectivePolicy, ProjectIdentity, Unavailable};
use std::path::{Path, PathBuf};

/// A ledger command the plan covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verb<'a> {
    /// `baley anchor [PROJECT]`.
    Anchor {
        /// The project the owner named.
        named: Option<&'a str>,
    },
}
impl<'a> Verb<'a> {
    fn name(self) -> &'static str {
        match self {
            Self::Anchor { .. } => "anchor",
        }
    }
    fn named(self) -> Option<&'a str> {
        match self {
            Self::Anchor { named } => named,
        }
    }
}

/// A managed checkout's project file as it was observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectFile {
    /// The working-tree `baley.toml`.
    pub(super) path: PathBuf,
    /// `init::observe_file`'s observation of it.
    pub(super) id: Result<Option<ProjectIdentity>, Unavailable>,
}

/// The checkout's settings once read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Settings {
    /// The project file of a managed checkout, `None` outside one.
    pub(super) project_file: Option<ProjectFile>,
    /// The policy built from the two settings files, or what stops it.
    pub(super) policy: Result<EffectivePolicy, Unavailable>,
}
impl Settings {
    /// The id of the discovered project, or the refusal for a managed
    /// checkout whose file yields none. Such a file is never taken as no
    /// project, since the command would then act on the wrong chain.
    fn discovered(&self) -> Result<Option<String>, String> {
        self.project_file
            .as_ref()
            .map(|file| project_id(&file.path, &file.id))
            .transpose()
    }
}

/// The project id of the file at `path` as observed. `Ok(None)` means the
/// file was removed since discovery walked past it.
pub(super) fn project_id(
    path: &Path,
    observed: &Result<Option<ProjectIdentity>, Unavailable>,
) -> Result<String, String> {
    match observed {
        Ok(Some(identity)) => Ok(identity.id.clone()),
        Ok(None) => Err(format!(
            "{CONFIG_UNAVAILABLE}: {} was not found",
            path.display()
        )),
        Err(unavailable) => Err(unavailable.to_string()),
    }
}

/// What is known so far.
#[derive(Debug, Clone, Copy)]
pub(super) struct Facts<'a> {
    /// The command.
    pub(super) command: Verb<'a>,
    /// The checkout's settings, once read.
    pub(super) settings: Option<&'a Settings>,
    /// The exact-name check's verdict for the remote, once made.
    pub(super) remote_check: Option<&'a Result<(), String>>,
}

/// One operation the wiring performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Op {
    /// Find the project and read both settings files.
    ReadSettings,
    /// Check that git lists the remote by exactly this name.
    CheckRemote(String),
    /// Run the policy step for the checkout.
    Step,
    /// Anchor the project on the remote, or on none.
    Anchor {
        /// The project to anchor.
        project: String,
        /// The remote its `git.remote` names.
        remote: Option<String>,
    },
}

fn target_text(command: &str, refusal: &TargetRefusal) -> String {
    match refusal {
        TargetRefusal::Differs { named, discovered } => format!(
            "project {named} is not the project of this checkout ({discovered}); run baley {command} from a checkout of {named}"
        ),
        TargetRefusal::NoneDiscovered { named } => format!(
            "no baley.toml was found from this directory; run baley {command} from a checkout of {named}"
        ),
    }
}

/// The operations to request next, or the refusal. Every refusal comes
/// before the policy step, so a refused command records nothing.
pub(super) fn next(facts: &Facts) -> Result<Vec<Op>, String> {
    let command = facts.command;
    let Some(settings) = facts.settings else {
        return Ok(vec![Op::ReadSettings]);
    };
    let discovered = settings.discovered()?;
    let project = anchor_plan::judge_target(discovered.as_deref(), command.named())
        .map_err(|refusal| target_text(command.name(), &refusal))?
        .ok_or_else(|| {
            format!(
                "no baley.toml was found from this directory; baley {} runs from a checkout of the project",
                command.name()
            )
        })?;
    let policy = settings.policy.as_ref().map_err(ToString::to_string)?;
    let remote = anchor_plan::remote_of(policy);
    if let Some(name) = &remote {
        match facts.remote_check {
            None => return Ok(vec![Op::CheckRemote(name.clone())]),
            Some(Err(refusal)) => return Err(refusal.clone()),
            Some(Ok(())) => {}
        }
    }
    // Owner: T13 (phase 9). Its admission of the checkout is requested here,
    // between the settings read and the policy step.
    Ok(vec![Op::Step, Op::Anchor { project, remote }])
}
