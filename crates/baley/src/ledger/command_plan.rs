//! The plan of the ledger commands that read the checkout's settings. From
//! the facts gathered so far it gives the operations the command requests
//! next, in order, or its refusal. Pure: the wiring performs each operation
//! and asks again, so a refusal always comes before the operation after it.
use super::anchor_plan::{self, RemoteState, TargetRefusal};
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
    /// `baley acknowledge-restore [PROJECT]`.
    AcknowledgeRestore {
        /// The project the owner named.
        named: Option<&'a str>,
    },
    /// `baley verify`, in one of its three forms.
    Verify(Form<'a>),
}

/// The forms of `baley verify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Form<'a> {
    /// No flag: the checkout's project against its `git.remote`.
    Anchored {
        /// The project the owner named.
        named: Option<&'a str>,
    },
    /// `--local-only PROJECT`.
    LocalOnly(&'a str),
    /// `--views PROJECT`.
    Views(&'a str),
}

/// The form the arguments name. The parser already refuses a flag with no
/// project and both flags together, so the refusal here is a second guard.
pub(super) fn verify_form(
    project: Option<&str>,
    local_only: bool,
    views: bool,
) -> Result<Form<'_>, String> {
    match (project, local_only, views) {
        (named, false, false) => Ok(Form::Anchored { named }),
        (Some(project), true, false) => Ok(Form::LocalOnly(project)),
        (Some(project), false, true) => Ok(Form::Views(project)),
        _ => Err("verify takes --local-only or --views, not both, and each needs a project".into()),
    }
}
impl<'a> Verb<'a> {
    fn name(self) -> &'static str {
        match self {
            Self::Anchor { .. } => "anchor",
            Self::AcknowledgeRestore { .. } => "acknowledge-restore",
            Self::Verify(_) => "verify",
        }
    }
    fn named(self) -> Option<&'a str> {
        match self {
            Self::Anchor { named }
            | Self::AcknowledgeRestore { named }
            | Self::Verify(Form::Anchored { named }) => named,
            Self::Verify(Form::LocalOnly(project) | Form::Views(project)) => Some(project),
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

/// What `doctor` makes of the checkout's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DoctorSettings {
    /// The discovered project's id, `None` outside a checkout or when the
    /// project file yields no id.
    pub(super) discovered: Option<String>,
    /// The discovered project's `git.remote`.
    pub(super) remote: RemoteState,
    /// Each settings problem as its `config-unavailable` text, which names
    /// the file. `doctor` reports them and keeps checking.
    pub(super) faults: Vec<String>,
}

/// Judges the gathered settings for `doctor`. A problem is a finding, not a
/// refusal, so one bad file never hides the store's own findings. The policy
/// cannot be built from an invalid file, so the remote is then unknown: none
/// is taken from HEAD's copy alone.
pub(super) fn doctor_settings(settings: &Settings) -> DoctorSettings {
    let mut faults = Vec::new();
    let discovered = settings.discovered().unwrap_or_else(|fault| {
        faults.push(fault);
        None
    });
    let remote = match &settings.policy {
        Ok(policy) => discovered
            .as_ref()
            .and_then(|_| anchor_plan::remote_of(policy))
            .map_or(RemoteState::NotSet, RemoteState::Name),
        Err(unavailable) => {
            faults.push(unavailable.to_string());
            RemoteState::Unknown
        }
    };
    DoctorSettings {
        discovered,
        remote,
        faults,
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
    /// Verify the project against the remote, or locally when there is none.
    Verify {
        /// The project to verify.
        project: String,
        /// The remote its `git.remote` names.
        remote: Option<String>,
    },
    /// Compare the project's views with a replay.
    Views(String),
    /// Accept the restored chain of the project behind the anchor on the
    /// remote.
    Acknowledge {
        /// The project whose restore is accepted.
        project: String,
        /// The remote holding the anchor.
        remote: String,
    },
}

/// Where the owner goes instead of the checkout, for `project`.
fn instead(command: Verb, project: &str) -> String {
    match command {
        Verb::Verify(_) => {
            format!("run baley verify --local-only {project} to check without a checkout")
        }
        _ => format!("run baley {} from a checkout of {project}", command.name()),
    }
}

fn target_text(command: Verb, refusal: &TargetRefusal) -> String {
    match refusal {
        TargetRefusal::Differs { named, discovered } => format!(
            "project {named} is not the project of this checkout ({discovered}); {}",
            instead(command, named)
        ),
        TargetRefusal::NoneDiscovered { named } => format!(
            "no baley.toml was found from this directory; {}",
            instead(command, named)
        ),
    }
}

fn no_project_text(command: Verb) -> String {
    match command {
        Verb::Verify(_) => format!(
            "no baley.toml was found from this directory; {}",
            instead(command, "<project>")
        ),
        _ => format!(
            "no baley.toml was found from this directory; baley {} runs from a checkout of the project",
            command.name()
        ),
    }
}

/// The operations to request next, or the refusal. Every refusal comes
/// before the policy step, so a refused command records nothing.
pub(super) fn next(facts: &Facts) -> Result<Vec<Op>, String> {
    let command = facts.command;
    // The two flagged forms read no settings file, so no file can refuse them.
    match command {
        Verb::Verify(Form::LocalOnly(project)) => {
            return Ok(vec![Op::Verify {
                project: project.into(),
                remote: None,
            }]);
        }
        Verb::Verify(Form::Views(project)) => return Ok(vec![Op::Views(project.into())]),
        _ => {}
    }
    let Some(settings) = facts.settings else {
        return Ok(vec![Op::ReadSettings]);
    };
    let discovered = settings.discovered()?;
    let project = anchor_plan::judge_target(discovered.as_deref(), command.named())
        .map_err(|refusal| target_text(command, &refusal))?
        .ok_or_else(|| no_project_text(command))?;
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
    let last = match (command, remote) {
        // Anchored verify only reads: it requests no policy step and appends
        // nothing.
        (Verb::Verify(_), remote) => return Ok(vec![Op::Verify { project, remote }]),
        (Verb::Anchor { .. }, remote) => Op::Anchor { project, remote },
        (Verb::AcknowledgeRestore { .. }, Some(remote)) => Op::Acknowledge { project, remote },
        // The core's acknowledgement holds a remote name, not an absence, so
        // this refuses here rather than record a refusal of its own.
        (Verb::AcknowledgeRestore { .. }, None) => {
            return Err("git.remote is not set; acknowledge-restore needs the remote that holds the anchor, so set it in baley.toml and commit it".into());
        }
    };
    Ok(vec![Op::Step, last])
}
