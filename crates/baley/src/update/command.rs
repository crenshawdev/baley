//! Foreground update gathering and wiring (design 0012 sections 5 and 6).

use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use baley_core::catalog::USER_PROJECT;
use baley_core::policy::{FileLayer, Schema, Value, merge, parse_layer};
use baley_core::with_heartbeat;
use baley_store::{ClaimId, ClaimState, CommandKind, Ledger, ProjectId, StoreError};
use baley_store_sqlite::SqliteStore;
use clap::{Args, Subcommand};

use crate::folders::{Environment, Folders, Platform};
use crate::ledger::clock::SystemClock;
use crate::ledger::commands::new_request_id;
use crate::ledger::display::{self, Render};
use crate::ledger::open;
use crate::ledger::ticker::ThreadTicker;
use crate::process::Launch;
use crate::{models, settings};

use super::claim::{self, Check, FetchPermit, Gate, Trigger};
use super::deliver::{self, Failure, Step};
use super::events::{CheckOutcome, FailureCode};
use super::fetch::{self, HttpFetcher};
use super::installation::{self, Active, Layout};
use super::manifest;
use super::receipt::{self, BeforeClaim};
use super::reconcile::{self, ReconcileStep};
use super::record::{self, CheckResult};
use super::seed;
use super::version::{self, Selection};

/// A manual check unless a further update command is supplied.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// An update operation, absent for the foreground check.
    #[command(subcommand)]
    pub command: Option<UpdateCommand>,
}

/// Further operations on an installation's update state.
#[derive(Debug, Subcommand)]
pub enum UpdateCommand {
    /// Record this binary's compiled catalog seed.
    #[command(hide = true)]
    Seed,
}

/// Runs the foreground check or the newly active binary's seed command.
pub fn run(args: UpdateArgs) -> ExitCode {
    match args.command {
        Some(UpdateCommand::Seed) => seed::run(),
        None => {
            let render = manual().unwrap_or_else(|error| error);
            ExitCode::from(display::emit(
                &render,
                &mut io::stdout().lock(),
                &mut io::stderr().lock(),
            ))
        }
    }
}

fn manual() -> Result<Render, Render> {
    let at = SystemClock::now();
    let env = Environment::read();
    let layout =
        Layout::resolve(&env).map_err(|error| receipt::before_claim(&BeforeClaim::Home(error)))?;
    let folders = Folders::resolve(Platform::current(), &env)
        .map_err(|error| Render::refusal(error.to_string()))?;
    let schema = Schema::standard();
    let global = settings::read(&settings::global_path(&folders))
        .and_then(|file| {
            file.as_ref()
                .map(|file| parse_layer(file, FileLayer::Global, schema))
                .transpose()
        })
        .map_err(|error| receipt::before_claim(&BeforeClaim::Settings(error)))?;
    let policy = merge(schema, None, global.as_ref(), None);
    let Some(Value::HttpsAddress(source)) = &policy.settings["updates.source"].value else {
        return Err(receipt::before_claim(&BeforeClaim::SourceUnset));
    };
    let store = Arc::new(
        open::store(&folders.home, &at, open::options())
            .map_err(|error| display::store_error(&error, None))?,
    );
    models::create_user(store.as_ref(), &at)
        .map_err(|error| display::store_error(&error, Some(USER_PROJECT)))?;
    let mut check = Check {
        installation: layout.installation().into(),
        trigger: Trigger::Manual,
        request_id: new_request_id(),
        owner: claim::claim_owner(std::process::id(), &at),
        at,
    };
    let mut gate = claim::gate(claim::claim_check(store.as_ref(), &check));
    if let Gate::NeedsReconciliation { holder } = &gate {
        gate = match reconcile_claim(store.as_ref(), &layout, holder, &check.at) {
            Ok(ReconcileStep::Resolved) => {
                check.request_id = new_request_id();
                claim::gate(claim::claim_check(store.as_ref(), &check))
            }
            Ok(ReconcileStep::AwaitingOwner) => Gate::Busy {
                holder: holder.clone(),
                state: ClaimState::AwaitingOwner,
            },
            Err(error) => claim::gate(Err(error)),
        };
    }
    match gate {
        Gate::Fetch(permit) => Ok(claimed_check(store, &layout, source, &check, &permit)),
        stopped => Err(
            receipt::at_claim(layout.installation(), &check.at[..10], &stopped)
                .expect("a stopped gate has a receipt"),
        ),
    }
}

fn reconcile_claim(
    store: &dyn Ledger,
    layout: &Layout,
    holder: &ClaimId,
    at: &str,
) -> Result<ReconcileStep, StoreError> {
    let stable = installation::gather_stable(layout).map_err(|error| {
        StoreError::Unavailable(format!("{}: {error}", layout.stable_path().display()))
    })?;
    let children = installation::gather_children(layout).map_err(|error| {
        StoreError::Unavailable(format!("{}: {error}", layout.versions_folder().display()))
    })?;
    reconcile::reconcile_from_observation(
        store,
        layout,
        holder,
        &stable,
        &children,
        new_request_id(),
        at,
    )
}

fn claimed_check(
    store: Arc<SqliteStore>,
    layout: &Layout,
    source: &str,
    check: &Check,
    permit: &FetchPermit,
) -> Render {
    let renew = {
        let store = Arc::clone(&store);
        let project = ProjectId(USER_PROJECT.into());
        let claim = ClaimId {
            kind: CommandKind(claim::UPDATE_CHECK.into()),
            request_id: check.request_id.clone(),
        };
        let owner = check.owner.clone();
        move |at: &str| store.renew_lease(&project, &claim, &owner, at)
    };
    let mut ticker = ThreadTicker::new(Arc::new(SystemClock::now));
    let ((result, launch), renewal_errors) = with_heartbeat(renew, &check.at, &mut ticker, || {
        gather_check(layout, source, check, permit)
    });
    let recorded = match &result {
        receipt::Check::Current { active_version, .. } => CheckResult {
            outcome: Ok(CheckOutcome::Current),
            active_version: Some(*active_version),
            staged_version: None,
        },
        receipt::Check::Activated(activated) => CheckResult {
            outcome: Ok(CheckOutcome::Staged),
            active_version: Some(activated.active_version),
            staged_version: Some(activated.staged_version),
        },
        receipt::Check::Failed {
            active_version,
            failure,
        } => CheckResult {
            outcome: Err(failure.code),
            active_version: *active_version,
            staged_version: failure.staged_version,
        },
    };
    let completion = record::complete(
        store.as_ref(),
        check,
        permit.claim_seq(),
        &recorded,
        &SystemClock::now(),
    );
    // The new binary may raise the view set, so completion must return first.
    let seeded = launch
        .as_ref()
        .map(|launch| seed::interpret(seed::gather(launch)));
    let outcome = receipt::outcome(result, completion, seeded);
    receipt::render(layout.installation(), &outcome, &renewal_errors)
}

fn gather_check(
    layout: &Layout,
    source: &str,
    check: &Check,
    permit: &FetchPermit,
) -> (receipt::Check, Option<Launch>) {
    let mut active_version = None;
    let gathered = (|| {
        let start = installation::gather_stable(layout).map_err(|error| Failure {
            step: Step::Activation,
            code: FailureCode::NotWritable,
            cause: format!("{}: {error}", layout.stable_path().display()),
            staged_version: None,
        })?;
        let active = match installation::judge_active(layout, &start) {
            Active::Version(version) => version,
            Active::NotManaged(_) => {
                return Err(Failure {
                    step: Step::Activation,
                    code: FailureCode::NotInstalled,
                    cause: format!(
                        "{} is not a managed installation link",
                        layout.installation()
                    ),
                    staged_version: None,
                });
            }
        };
        active_version = Some(active);
        let addresses = manifest::addresses(source, std::env::consts::OS, std::env::consts::ARCH);
        let fetcher = HttpFetcher::new();
        let bytes =
            fetch::interpret(fetcher.manifest(permit, &addresses)).map_err(|error| Failure {
                step: Step::Download,
                code: error.code(),
                cause: format!("{}: {}", error.address, error.cause),
                staged_version: None,
            })?;
        let manifest = manifest::parse_manifest(&bytes).map_err(|error| Failure {
            step: Step::Verification,
            code: FailureCode::ManifestInvalid,
            cause: format!("{}: {error}", addresses.manifest),
            staged_version: None,
        })?;
        match version::select(active, manifest.version) {
            Selection::Current { active, offered } => Ok((
                receipt::Check::Current {
                    active_version: active,
                    offered_version: offered,
                },
                None,
            )),
            Selection::Stage { .. } => {
                let download = fetcher.binary(permit, &addresses);
                let delivery =
                    deliver::deliver(layout, &start, &manifest, download, &check.request_id);
                let launch = seed::launch_for(layout, &delivery);
                Ok((receipt::Check::Activated(delivery?), launch))
            }
        }
    })();
    gathered.unwrap_or_else(|failure| {
        (
            receipt::Check::Failed {
                active_version,
                failure,
            },
            None,
        )
    })
}
