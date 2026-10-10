//! The daily `update.check` claim and the gate in front of every fetch
//! (design 0012 section 6 Daily claim, design 0001 Daily update claim).
//!
//! A check is a command in the per-user `user` project, claimed through the
//! ordinary claim step so no new port operation exists. Its identity is the
//! installation and the UTC day, never the hour: two attempts on one day for
//! one actor share a request digest. A manual `baley update` always claims.
//! The detached check asks the ledger first whether that installation and day
//! were claimed, by anyone, and is refused `update-check-not-due` when so.
//!
//! The gate turns a claim result into the next step. Only a new claim yields
//! a [`FetchPermit`], and the permit has no other way to be made, so the
//! fetchers that take one cannot be reached without a claim.

use std::collections::BTreeMap;

use baley_store::{
    Actor, Answer, Block, COMMAND_CLAIMED, ClaimDecision, ClaimId, ClaimOwner, ClaimState, Claimed,
    Command, CommandKind, Decision, EventMatch, Ledger, Observed, OutcomeKind, ProjectId, Refusal,
    RequestId, StoreError, Transaction, UtcInstant, command_stream, request_digest,
};
use serde_json::{Value, json};

/// The command kind of an update check.
pub const UPDATE_CHECK: &str = "update.check";
/// The code of a detached check refused because its day was already claimed.
pub const NOT_DUE: &str = "update-check-not-due";
/// The code of a check that found another check still holding the installation.
pub const CHECK_BUSY: &str = "update-check-busy";
/// The code of a check that found an interrupted check holding the installation.
pub const NEEDS_RECONCILIATION: &str = "update-check-needs-reconciliation";
/// The code reported for a refusal whose recorded answer carries none.
const REFUSED: &str = "update-check-refused";
/// The project every update record lives in.
const USER: &str = "user";

/// Who asked for the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// `baley update`, run by the owner. Always claims.
    Manual,
    /// The detached daily check `baley serve` starts. Claims only on a day
    /// nobody claimed for the installation.
    Detached,
}

/// One check to claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// The installation, the stable path as text.
    pub installation: String,
    /// Who asked.
    pub trigger: Trigger,
    /// A fresh request id.
    pub request_id: RequestId,
    /// The supplied UTC time: the claim's time and the source of its day.
    pub at: String,
    /// The process that acts.
    pub owner: ClaimOwner,
}

/// The owner of a command-line claim: the process id as decimal text and the
/// `cli` host session, so a server claim is never mistaken for it.
pub fn claim_owner(process: u32, started_at: &str) -> ClaimOwner {
    ClaimOwner {
        process: process.to_string(),
        host_session: "cli".into(),
        started_at: started_at.into(),
    }
}

fn invalid(error: impl std::fmt::Display) -> StoreError {
    StoreError::Refused(Refusal::InvalidEvent(error.to_string()))
}

impl Check {
    /// The UTC calendar date of the supplied time, `YYYY-MM-DD`.
    fn day(&self) -> Result<&str, StoreError> {
        UtcInstant::parse(&self.at).map_err(invalid)?;
        // A parsed instant always opens with its date.
        Ok(&self.at[..10])
    }

    /// The claim's intent, `{installation, day}`: what the existing-claim
    /// match compares and what a reader of the ledger sees.
    pub fn intent(&self) -> Result<Value, StoreError> {
        Ok(json!({"installation": self.installation, "day": self.day()?}))
    }

    fn actor(&self) -> Actor {
        match self.trigger {
            Trigger::Manual => Actor::Owner,
            Trigger::Detached => Actor::Baley,
        }
    }

    /// The command: kind `update.check` in `user`, scope the one token
    /// `update/<installation>`, policy version 0 and no caller. The digest
    /// covers the kind, project, actor, policy version, intent and scope and
    /// nothing else, so the hour never changes it.
    pub fn command(&self) -> Result<Command, StoreError> {
        let scope = format!("update/{}", self.installation);
        let actor = self.actor();
        let digest = request_digest(&json!({
            "kind": UPDATE_CHECK,
            "project": USER,
            "actor": actor.as_str(),
            "policy_version": 0,
            "intent": self.intent()?,
            "scope": [scope],
        }))
        .map_err(invalid)?;
        Ok(Command {
            project: ProjectId(USER.into()),
            kind: CommandKind(UPDATE_CHECK.into()),
            request_id: self.request_id.clone(),
            digest,
            scope: vec![scope],
            policy_version: 0,
            recorded_at: self.at.clone(),
            actor,
            caller: None,
        })
    }
}

/// The claim decision inside the claim transaction. A manual check always
/// claims. A detached check first asks the ledger for any recorded claim of
/// the same installation and day, completed or failed, manual or detached,
/// and refuses on the merits when one exists, recording no claim.
fn decide(tx: &mut dyn Transaction, check: &Check) -> Result<ClaimDecision, StoreError> {
    let intent = check.intent()?;
    if check.trigger == Trigger::Detached {
        let kind = CommandKind(UPDATE_CHECK.into());
        let claimed = EventMatch {
            type_name: COMMAND_CLAIMED.into(),
            stream: Some(command_stream(&kind)),
            fields: BTreeMap::from([
                ("kind".to_owned(), json!(UPDATE_CHECK)),
                ("intent".to_owned(), intent.clone()),
            ]),
        };
        if tx.event_exists(&claimed)? {
            return Ok(ClaimDecision::Refuse(Decision {
                kind: OutcomeKind::Refused,
                answer: json!({
                    "code": NOT_DUE,
                    "installation": check.installation,
                    "day": check.day()?,
                }),
                sensitive: false,
                observed: Observed::default(),
                git: None,
            }));
        }
    }
    Ok(ClaimDecision::Claim {
        intent,
        owner: check.owner.clone(),
        git: None,
        observed: Observed::default(),
    })
}

/// Takes the claim for `check`. The result goes to [`gate`].
pub fn claim_check(ledger: &dyn Ledger, check: &Check) -> Result<Claimed, StoreError> {
    let command = check.command()?;
    ledger.claim(&command, &mut |tx| decide(tx, check))
}

/// Proof that this check holds a new claim. Only [`gate`] makes one, in its
/// `Claimed::New` arm: the field is private and the type has no constructor,
/// no `Default` and no `Clone`, so a fetcher that asks for a permit cannot
/// be called before the ledger shows the claim.
#[derive(Debug, PartialEq, Eq)]
pub struct FetchPermit {
    claim_seq: u64,
}

impl FetchPermit {
    /// The sequence of the claim's `command.claimed` event.
    pub fn claim_seq(&self) -> u64 {
        self.claim_seq
    }
}

/// What the check does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Gate {
    /// The claim is new: fetch, and record under this claim.
    Fetch(FetchPermit),
    /// The claim decision refused before any effect, with its recorded code.
    Refused {
        /// The recorded code, such as `update-check-not-due`.
        code: String,
    },
    /// This request already holds an open claim. Nothing is fetched again.
    InProgress(ClaimId),
    /// This request was completed before. Nothing is fetched again.
    Replayed,
    /// Another check holds the installation and is live or held for the owner.
    Busy {
        /// The holder.
        holder: ClaimId,
        /// Its state: active, or awaiting the owner.
        state: ClaimState,
    },
    /// An interrupted check holds the installation. It must be reconciled
    /// before another can claim.
    NeedsReconciliation {
        /// The holder.
        holder: ClaimId,
    },
    /// Any other store error, passed on as it came.
    Failed(StoreError),
}

/// Maps a claim result to the next step. Only `Claimed::New` fetches.
pub fn gate(result: Result<Claimed, StoreError>) -> Gate {
    match result {
        Ok(Claimed::New { seq }) => Gate::Fetch(FetchPermit { claim_seq: seq }),
        Ok(Claimed::Refused { outcome, .. }) => {
            let code = match outcome.answer {
                Answer::Inline(answer) => answer
                    .get("code")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                _ => None,
            };
            Gate::Refused {
                code: code.unwrap_or_else(|| REFUSED.into()),
            }
        }
        Ok(Claimed::InProgress(claim)) => Gate::InProgress(claim.id),
        Ok(Claimed::Replayed(_)) => Gate::Replayed,
        Err(StoreError::Blocked(Block { claim, state })) => match state {
            ClaimState::Interrupted => Gate::NeedsReconciliation { holder: claim },
            ClaimState::Active | ClaimState::AwaitingOwner => Gate::Busy {
                holder: claim,
                state,
            },
        },
        Err(error) => Gate::Failed(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::open::{options, store};
    use crate::models;
    use baley_store::{
        Answer, Block, ClaimId, ClaimState, Claimed, CommandKind, Decision, Hash, Head, Observed,
        Outcome, OutcomeKind, RequestId, StoreError,
    };
    use serde_json::json;

    const INSTALLATION: &str = "/home/o/.local/bin/baley";

    fn check(installation: &str, trigger: Trigger, n: u8, at: &str) -> Check {
        Check {
            installation: installation.into(),
            trigger,
            request_id: RequestId(format!("00000000-0000-4000-8000-0000000000{n:02x}")),
            at: at.into(),
            owner: claim_owner(4242, at),
        }
    }

    fn done() -> Decision {
        Decision {
            kind: OutcomeKind::Done,
            answer: json!({}),
            sensitive: false,
            observed: Observed::default(),
            git: None,
        }
    }

    #[test]
    fn a_claim_whose_identity_drifts_with_the_hour_or_the_actor_is_caught() {
        let commands: Vec<_> = [
            (Trigger::Manual, "2026-10-08T00:00:00Z"),
            (Trigger::Manual, "2026-10-08T23:59:59.999Z"),
            (Trigger::Detached, "2026-10-08T12:00:00Z"),
            (Trigger::Manual, "2026-10-09T00:00:00Z"),
        ]
        .into_iter()
        .enumerate()
        .map(|(n, (trigger, at))| {
            check(INSTALLATION, trigger, n as u8, at)
                .command()
                .expect("a valid time")
        })
        .collect();
        for command in &commands {
            assert_eq!(command.kind, CommandKind("update.check".into()));
            assert_eq!(command.project.0, "user");
            assert_eq!(command.scope, ["update//home/o/.local/bin/baley"]);
            assert_eq!(command.policy_version, 0);
            assert_eq!(command.caller, None);
        }
        let actors: Vec<&str> = commands.iter().map(|c| c.actor.as_str()).collect();
        assert_eq!(actors, ["owner", "owner", "baley", "owner"]);
        let intents: Vec<_> = [
            (Trigger::Manual, "2026-10-08T00:00:00Z"),
            (Trigger::Manual, "2026-10-08T23:59:59.999Z"),
            (Trigger::Detached, "2026-10-08T12:00:00Z"),
            (Trigger::Manual, "2026-10-09T00:00:00Z"),
        ]
        .into_iter()
        .map(|(trigger, at)| check(INSTALLATION, trigger, 0, at).intent().unwrap())
        .collect();
        let on = |day: &str| json!({"installation": INSTALLATION, "day": day});
        assert_eq!(
            intents,
            [
                on("2026-10-08"),
                on("2026-10-08"),
                on("2026-10-08"),
                on("2026-10-09")
            ]
        );
        assert_eq!(commands[0].digest, commands[1].digest);
        assert_ne!(commands[0].digest, commands[2].digest);
        assert_ne!(commands[0].digest, commands[3].digest);
    }

    #[test]
    fn a_claim_owner_not_marked_as_the_command_line_is_caught() {
        let owner = claim_owner(4242, "2026-10-08T09:00:00Z");
        assert_eq!(owner.process, "4242");
        assert_eq!(owner.host_session, "cli");
        assert_eq!(owner.started_at, "2026-10-08T09:00:00Z");
    }

    #[test]
    fn a_manual_check_refused_as_not_due_or_a_detached_check_let_through_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = store(&dir.path().join("home"), "2026-10-08T08:00:00Z", options()).unwrap();
        models::create_user(&ledger, "2026-10-08T08:00:00Z").unwrap();
        let run = |check: Check| {
            let claimed = claim_check(&ledger, &check).expect("the claim step runs");
            if matches!(claimed, Claimed::New { .. }) {
                ledger
                    .complete(&check.command().unwrap(), &check.owner, &mut |_| Ok(done()))
                    .expect("the owner completes");
            }
            claimed
        };
        let other = "/home/p/.local/bin/baley";

        let first = run(check(
            INSTALLATION,
            Trigger::Manual,
            1,
            "2026-10-08T09:00:00Z",
        ));
        assert!(matches!(first, Claimed::New { .. }), "{first:?}");

        let detached = run(check(
            INSTALLATION,
            Trigger::Detached,
            2,
            "2026-10-08T10:00:00Z",
        ));
        let Claimed::Refused { outcome, .. } = detached else {
            panic!("a detached check on a claimed day was not refused: {detached:?}");
        };
        assert_eq!(outcome.kind, OutcomeKind::Refused);
        let Answer::Inline(answer) = outcome.answer else {
            panic!("the refusal is recorded inline");
        };
        assert_eq!(answer["code"], "update-check-not-due");
        assert_eq!(answer["installation"], INSTALLATION);
        assert_eq!(answer["day"], "2026-10-08");

        let manual = run(check(
            INSTALLATION,
            Trigger::Manual,
            3,
            "2026-10-08T11:00:00Z",
        ));
        assert!(matches!(manual, Claimed::New { .. }), "{manual:?}");

        let elsewhere = run(check(other, Trigger::Detached, 4, "2026-10-08T12:00:00Z"));
        assert!(matches!(elsewhere, Claimed::New { .. }), "{elsewhere:?}");

        let next_day = run(check(
            INSTALLATION,
            Trigger::Detached,
            5,
            "2026-10-09T00:00:00Z",
        ));
        assert!(matches!(next_day, Claimed::New { .. }), "{next_day:?}");
    }

    fn claim(n: u8) -> baley_store::Claim {
        baley_store::Claim {
            id: ClaimId {
                kind: CommandKind("update.check".into()),
                request_id: RequestId(format!("00000000-0000-4000-8000-0000000000{n:02x}")),
            },
            seq: 4,
            claimed_at: "2026-10-08T09:00:00Z".into(),
            intent: json!({}),
            scope: vec![],
            owner: claim_owner(1, "2026-10-08T09:00:00Z"),
            lease_renewed_at: None,
            awaiting_owner: None,
        }
    }

    fn outcome(answer: serde_json::Value) -> Outcome {
        Outcome {
            kind: OutcomeKind::Refused,
            answer: Answer::Inline(answer),
        }
    }

    fn block(state: ClaimState) -> StoreError {
        StoreError::Blocked(Block {
            claim: claim(9).id,
            state,
        })
    }

    #[test]
    fn a_fetch_allowed_without_a_new_claim_is_caught() {
        let Gate::Fetch(permit) = gate(Ok(Claimed::New { seq: 7 })) else {
            panic!("a new claim did not permit a fetch");
        };
        assert_eq!(permit.claim_seq(), 7);

        let refused = gate(Ok(Claimed::Refused {
            outcome: outcome(json!({"code": "update-check-not-due"})),
            head: Head {
                seq: 5,
                hash: Hash([0; 32]),
            },
        }));
        assert_eq!(
            refused,
            Gate::Refused {
                code: "update-check-not-due".into()
            }
        );
        assert_eq!(
            gate(Ok(Claimed::InProgress(claim(1)))),
            Gate::InProgress(claim(1).id)
        );
        assert_eq!(
            gate(Ok(Claimed::Replayed(outcome(json!({}))))),
            Gate::Replayed
        );
        assert_eq!(
            gate(Err(block(ClaimState::Active))),
            Gate::Busy {
                holder: claim(9).id,
                state: ClaimState::Active
            }
        );
        assert_eq!(
            gate(Err(block(ClaimState::AwaitingOwner))),
            Gate::Busy {
                holder: claim(9).id,
                state: ClaimState::AwaitingOwner
            }
        );
        assert_eq!(
            gate(Err(block(ClaimState::Interrupted))),
            Gate::NeedsReconciliation {
                holder: claim(9).id
            }
        );
        assert_eq!(gate(Err(StoreError::Busy)), Gate::Failed(StoreError::Busy));
    }
}
