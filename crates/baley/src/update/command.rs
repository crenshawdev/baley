//! Manual and detached update gathering and wiring (design 0012 sections 5 and 6).

use std::io;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use baley_core::catalog::USER_PROJECT;
use baley_core::policy::{FileLayer, Schema, Value, merge, parse_layer};
use baley_core::with_heartbeat;
use baley_store::{Claim, ClaimId, ClaimState, CommandKind, Ledger, ProjectId, StoreError};
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
use super::events::FailureCode;
use super::fetch::{self, HttpFetcher};
use super::installation::{self, Active, Layout};
use super::manifest;
use super::receipt::{self, BeforeClaim};
use super::reconcile::{self, ReconcileStep};
use super::record;
use super::seed;
use super::version::{self, Selection, Version};

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
    /// Run the daily check started by the server.
    #[command(hide = true)]
    Detached,
    /// Record this binary's compiled catalog seed.
    #[command(hide = true)]
    Seed,
    /// Clear an update check held for the owner, from what is installed now.
    Resolve,
}

/// Runs an update check, catalog seed or owner's recovery command.
pub fn run(args: UpdateArgs) -> ExitCode {
    let render = match args.command {
        Some(UpdateCommand::Detached) => check(Trigger::Detached),
        Some(UpdateCommand::Seed) => return seed::run(),
        Some(UpdateCommand::Resolve) => resolve(),
        None => check(Trigger::Manual),
    }
    .unwrap_or_else(|error| error);
    ExitCode::from(display::emit(
        &render,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ))
}

fn resolve() -> Result<Render, Render> {
    let at = SystemClock::now();
    let env = Environment::read();
    let layout =
        Layout::resolve(&env).map_err(|error| receipt::before_claim(&BeforeClaim::Home(error)))?;
    let folders = Folders::resolve(Platform::current(), &env)
        .map_err(|error| Render::refusal(error.to_string()))?;
    let store = open::store(&folders.home, &at, open::options())
        .map_err(|error| display::store_error(&error, None))?;
    models::create_user(&store, &at)
        .map_err(|error| display::store_error(&error, Some(USER_PROJECT)))?;
    let stable = installation::gather_stable(&layout).map_err(|error| {
        display::store_error(
            &StoreError::Unavailable(format!("{}: {error}", layout.stable_path().display())),
            Some(USER_PROJECT),
        )
    })?;
    let children = installation::gather_children(&layout).map_err(|error| {
        display::store_error(
            &StoreError::Unavailable(format!("{}: {error}", layout.versions_folder().display())),
            Some(USER_PROJECT),
        )
    })?;
    Ok(reconcile::resolve_receipt(
        layout.installation(),
        reconcile::resolve_from_observation(
            &store,
            &layout,
            &stable,
            &children,
            new_request_id(),
            &at,
        ),
    ))
}

fn check(trigger: Trigger) -> Result<Render, Render> {
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
        trigger,
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
    let renewal_failure = Arc::new(Mutex::new(None));
    let renew = {
        let store = Arc::clone(&store);
        let renewal_failure = Arc::clone(&renewal_failure);
        let project = ProjectId(USER_PROJECT.into());
        let claim = ClaimId {
            kind: CommandKind(claim::UPDATE_CHECK.into()),
            request_id: check.request_id.clone(),
        };
        let owner = check.owner.clone();
        move |_at: &str| {
            let mut failure = renewal_failure
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            // Read time under the lock so two renewals cannot arrive backwards.
            let result = store.renew_lease(&project, &claim, &owner, &SystemClock::now());
            if let Err(error) = &result {
                failure.get_or_insert_with(|| error.clone());
            }
            result
        }
    };
    let mut ticker = ThreadTicker::new(Arc::new(SystemClock::now));
    let ((result, launch), renewal_errors) = with_heartbeat(renew, &check.at, &mut ticker, || {
        gather_check(layout, source, check, permit, |step, version| {
            let failure = renewal_failure
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let observation = gather_lease(store.as_ref(), check, failure.clone());
            lease_decision(check, permit.claim_seq(), step, version, &observation)
        })
    });
    let recorded = result.record_result();
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
    mut before_step: impl FnMut(Step, Version) -> Result<(), Failure>,
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
                let delivery = deliver::deliver(
                    layout,
                    &start,
                    &manifest,
                    download,
                    &check.request_id,
                    |step| before_step(step, manifest.version),
                );
                let launch = seed::launch_for(layout, &delivery);
                Ok((receipt::Check::Activated(delivery?), launch))
            }
        }
    })();
    gathered.unwrap_or_else(|mut failure| {
        let fresh = if failure.step == Step::Activation {
            match installation::gather_stable(layout) {
                Ok(seen) => Some(installation::judge_active(layout, &seen)),
                Err(error) => {
                    failure
                        .cause
                        .push_str(&format!("; could not observe the stable path: {error}"));
                    active_version = None;
                    None
                }
            }
        } else {
            None
        };
        (receipt::Check::failed(active_version, failure, fresh), None)
    })
}

struct LeaseObservation {
    previous_error: Option<StoreError>,
    renewal: Result<(), StoreError>,
    claims: Result<Vec<Claim>, StoreError>,
}

fn gather_lease(
    store: &dyn Ledger,
    check: &Check,
    previous_error: Option<StoreError>,
) -> LeaseObservation {
    let project = ProjectId(USER_PROJECT.into());
    let claim = ClaimId {
        kind: CommandKind(claim::UPDATE_CHECK.into()),
        request_id: check.request_id.clone(),
    };
    LeaseObservation {
        previous_error,
        renewal: store.renew_lease(&project, &claim, &check.owner, &SystemClock::now()),
        claims: store.open_claims(&project),
    }
}

fn lease_decision(
    check: &Check,
    seq: u64,
    step: Step,
    version: Version,
    seen: &LeaseObservation,
) -> Result<(), Failure> {
    let cause = if let Some(error) = seen
        .previous_error
        .as_ref()
        .or(seen.renewal.as_ref().err())
        .or(seen.claims.as_ref().err())
    {
        Some(error.to_string())
    } else {
        let held = seen.claims.as_ref().is_ok_and(|claims| {
            claims.iter().any(|held| {
                held.id.kind.0 == claim::UPDATE_CHECK
                    && held.id.request_id == check.request_id
                    && held.seq == seq
                    && held.owner == check.owner
                    && held.awaiting_owner.is_none()
                    && held.scope == [format!("update/{}", check.installation)]
            })
        });
        (!held).then(|| {
            "the update claim is closed, held for the owner or held by another check".into()
        })
    };
    match cause {
        None => Ok(()),
        Some(cause) => Err(Failure {
            step,
            code: FailureCode::Interrupted,
            cause: format!(
                "claim {} is no longer confirmed: {cause}",
                check.request_id.0
            ),
            staged_version: (step == Step::Activation).then_some(version),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baley_store::RequestId;
    use serde_json::json;

    #[test]
    fn a_lost_claim_allowed_to_stage_or_activate_is_caught() {
        let check = Check {
            installation: "/home/o/.local/bin/baley".into(),
            trigger: Trigger::Manual,
            request_id: RequestId("00000000-0000-4000-8000-0000000000aa".into()),
            at: "2026-10-08T09:00:00Z".into(),
            owner: claim::claim_owner(4242, "2026-10-08T09:00:00Z"),
        };
        let held = Claim {
            id: ClaimId {
                kind: CommandKind("update.check".into()),
                request_id: check.request_id.clone(),
            },
            seq: 7,
            claimed_at: check.at.clone(),
            intent: json!({"installation": check.installation, "day": "2026-10-08"}),
            scope: vec!["update//home/o/.local/bin/baley".into()],
            owner: check.owner.clone(),
            lease_renewed_at: Some("2026-10-08T09:00:10Z".into()),
            awaiting_owner: None,
        };
        let observation = || LeaseObservation {
            previous_error: None,
            renewal: Ok(()),
            claims: Ok(vec![held.clone()]),
        };
        let version = Version::parse("0.2.0").unwrap();
        for step in [Step::Staging, Step::Activation] {
            assert_eq!(
                lease_decision(&check, 7, step, version, &observation()),
                Ok(())
            );
            for defect in [
                "renewal",
                "heartbeat",
                "closed",
                "request",
                "sequence",
                "owner",
                "held",
                "scope",
                "read",
            ] {
                let mut seen = observation();
                match defect {
                    "renewal" => seen.renewal = Err(StoreError::Busy),
                    "heartbeat" => seen.previous_error = Some(StoreError::Busy),
                    "closed" => seen.claims = Ok(vec![]),
                    "read" => seen.claims = Err(StoreError::Busy),
                    _ => {
                        let held = &mut seen.claims.as_mut().unwrap()[0];
                        match defect {
                            "request" => held.id.request_id = RequestId("another-request".into()),
                            "sequence" => held.seq = 8,
                            "owner" => held.owner.process = "5555".into(),
                            "held" => held.awaiting_owner = Some(9),
                            "scope" => held.scope = vec!["update/another-installation".into()],
                            _ => unreachable!(),
                        }
                    }
                }
                let failure = lease_decision(&check, 7, step, version, &seen)
                    .expect_err("a lost or unconfirmed claim must fence the next filesystem step");
                assert_eq!(failure.step, step, "{defect}");
                assert_eq!(failure.code, FailureCode::Interrupted, "{defect}");
                assert_eq!(
                    failure.staged_version,
                    (step == Step::Activation).then_some(version)
                );
                assert!(failure.cause.contains(&check.request_id.0));
            }
        }
    }
}
