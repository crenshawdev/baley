//! Which confirmed writes run after the owner's final `yes` (design 0003
//! section 5, Confirmed writes). It is one pure plan over the owner's answers
//! and what the gatherer read, in the order the writes run: the acknowledgement,
//! the API-host allowance, the reviewer list, then the settings. Each step is
//! planned only when it has something to change.

use std::path::PathBuf;

use baley_core::catalog::Provider;
use baley_core::policy::config_command::TypedPair;
use baley_store::NewEvent;
use serde_json::Value;

use super::providers::{Acknowledge, Answer};
use crate::config_command::reviewers;
use crate::install::hosts::{self, Allowance, ClaudeSettings};
use crate::providers::record;

/// What the acknowledgement step does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Ack {
    /// The providers were confirmed, and the `providers` stream is not read
    /// yet, which is so until the owner's final `yes`.
    Requested(Acknowledge),
    /// No stored acknowledgement covers the providers, so this event records
    /// one.
    Record(Acknowledge, Box<NewEvent>),
    /// A current stored acknowledgement already covers the providers.
    Reused(Acknowledge),
}

impl Ack {
    /// The providers and the owner the acknowledgement is for.
    pub(super) fn who(&self) -> &Acknowledge {
        match self {
            Ack::Requested(who) | Ack::Record(who, _) | Ack::Reused(who) => who,
        }
    }
}

/// The host step: the allowance to write into Claude Code's settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hosts {
    /// The settings file's path.
    pub(super) path: PathBuf,
    /// The decided document, its bytes and the hosts it adds.
    pub(super) allowance: Allowance,
}

/// The `providers` stream as it was read after the final `yes`.
pub(super) struct Stream<'a> {
    /// Each stored `providers.acknowledged` payload.
    pub(super) payloads: &'a [Value],
    /// The time a new acknowledgement is recorded at.
    pub(super) at: &'a str,
}

/// What the plan is judged from.
pub(super) struct Input<'a> {
    /// The owner's answer to the provider question.
    pub(super) answer: &'a Answer,
    /// The confirmed providers and the owner's name, when providers were chosen.
    pub(super) acknowledgement: Option<&'a Acknowledge>,
    /// The global file's top-level `review.reviewers` as written, or none.
    pub(super) stored: Option<&'a [String]>,
    /// Claude Code's settings file as read, or why it could not be.
    pub(super) claude: &'a Result<ClaudeSettings, String>,
    /// The `providers` stream, once read; none before the final `yes`.
    pub(super) stream: Option<Stream<'a>>,
    /// The non-blank settings answers.
    pub(super) settings: &'a [(String, String)],
}

/// The steps to run, each present only when it has something to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    /// Step 1.
    pub(super) acknowledgement: Option<Ack>,
    /// Step 2.
    pub(super) hosts: Option<Hosts>,
    /// Step 3: the whole `review.reviewers` list to write into the global file.
    pub(super) reviewers: Option<TypedPair>,
    /// Step 4: the settings answers, which `config set` judges for itself.
    pub(super) settings: Vec<(String, String)>,
}

impl Plan {
    /// Whether any step still writes something: a reused acknowledgement does
    /// not.
    pub(super) fn writes(&self) -> bool {
        matches!(
            self.acknowledgement,
            Some(Ack::Requested(_) | Ack::Record(..))
        ) || self.hosts.is_some()
            || self.reviewers.is_some()
            || !self.settings.is_empty()
    }
}

/// Why the interview stops before its summary, as the line the owner sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Refused(pub(super) String);

/// Judges the acknowledgement against the stream read after the final `yes`:
/// recorded as a new event, or reused when a stored one covers it.
pub(super) fn settle(acknowledgement: &Acknowledge, stream: &Stream<'_>) -> Ack {
    match record::judge(
        stream.payloads,
        &acknowledgement.providers,
        &acknowledgement.owner,
        stream.at,
    ) {
        Some(event) => Ack::Record(acknowledgement.clone(), Box::new(event)),
        None => Ack::Reused(acknowledgement.clone()),
    }
}

/// Plans the confirmed writes. A Claude settings file that cannot be read or
/// cannot take the hosts refuses the whole interview when a provider was
/// chosen, so nothing has been written when the owner is told. Without a
/// provider the file is not consulted.
pub(super) fn plan(input: &Input<'_>) -> Result<Plan, Refused> {
    let chosen: &[Provider] = match input.answer {
        Answer::Choose(chosen) => chosen,
        Answer::Keep => &[],
    };
    let hosts = if chosen.is_empty() {
        None
    } else {
        let claude = input
            .claude
            .as_ref()
            .map_err(|line| Refused(line.clone()))?;
        hosts::judge(&claude.path, &claude.current, chosen)
            .map_err(Refused)?
            .map(|allowance| Hosts {
                path: claude.path.clone(),
                allowance,
            })
    };
    let acknowledgement = input
        .acknowledgement
        .map(|acknowledgement| match &input.stream {
            None => Ack::Requested(acknowledgement.clone()),
            Some(stream) => settle(acknowledgement, stream),
        });
    let choice = match input.answer {
        Answer::Choose(chosen) => Some(chosen.as_slice()),
        Answer::Keep => None,
    };
    Ok(Plan {
        acknowledgement,
        hosts,
        reviewers: reviewers::decide(input.stored, choice),
        settings: input.settings.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::install::settings::Current;
    use crate::providers::fixtures::{DESIGN_WARNING, sha256_hex};

    const AT: &str = "2026-10-10T09:00:00Z";
    const SETTINGS: &str = "/home/o/.claude/settings.json";

    fn claude(document: Value) -> Result<ClaudeSettings, String> {
        Ok(ClaudeSettings {
            path: SETTINGS.into(),
            current: Current {
                document,
                symbolic_link: false,
                read_digest: Some("read".to_owned()),
            },
        })
    }

    fn ada(providers: &[Provider]) -> Acknowledge {
        Acknowledge {
            providers: providers.to_vec(),
            owner: "Ada Owner".to_owned(),
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn effort_max() -> Vec<(String, String)> {
        vec![("roles.planner.effort".to_owned(), "max".to_owned())]
    }

    /// A stored acknowledgement of `providers` at the current warning.
    fn stored_ack(providers: &[&str]) -> Value {
        json!({
            "providers": providers,
            "warning_version": 1,
            "warning_sha256": sha256_hex(DESIGN_WARNING),
            "owner": "Ada Owner",
            "acknowledged_at": "2026-10-01T09:00:00Z",
        })
    }

    #[test]
    fn a_rerun_of_a_completed_interview_that_writes_or_acknowledges_again_is_caught() {
        let answer = Answer::Choose(vec![Provider::OpenAi]);
        let acknowledgement = ada(&[Provider::OpenAi]);
        let stored = names(&["host", "openai"]);
        let claude =
            claude(json!({"sandbox": {"network": {"allowedDomains": ["api.openai.com"]}}}));
        let payloads = [stored_ack(&["openai"])];
        let settings = effort_max();

        let plan = plan(&Input {
            answer: &answer,
            acknowledgement: Some(&acknowledgement),
            stored: Some(&stored),
            claude: &claude,
            stream: Some(Stream {
                payloads: &payloads,
                at: AT,
            }),
            settings: &settings,
        })
        .unwrap();

        assert_eq!(plan.acknowledgement, Some(Ack::Reused(acknowledgement)));
        assert_eq!(plan.hosts, None);
        assert_eq!(plan.reviewers, None);
        assert_eq!(plan.settings, effort_max());
    }
    /// The plan before the final `yes`, with no settings answers unless given.
    fn before_yes(
        answer: &Answer,
        stored: Option<&[String]>,
        claude: &Result<ClaudeSettings, String>,
        settings: &[(String, String)],
    ) -> Result<Plan, Refused> {
        let providers = match answer {
            Answer::Choose(chosen) if !chosen.is_empty() => Some(ada(chosen)),
            _ => None,
        };
        plan(&Input {
            answer,
            acknowledgement: providers.as_ref(),
            stored,
            claude,
            stream: None,
            settings,
        })
    }

    #[test]
    fn a_blank_provider_answer_planning_a_provider_write_is_caught() {
        let stored = names(&["host", "openai"]);
        let claude = claude(json!({"model": "opus"}));
        let settings = effort_max();

        let plan = before_yes(&Answer::Keep, Some(&stored), &claude, &settings).unwrap();

        assert_eq!(plan.acknowledgement, None);
        assert_eq!(plan.hosts, None);
        assert_eq!(plan.reviewers, None);
        assert_eq!(plan.settings, effort_max());
    }

    #[test]
    fn an_explicit_none_that_leaves_the_stored_providers_is_caught() {
        let claude = claude(json!({}));
        let none = Answer::Choose(Vec::new());
        for (stored, expected) in [
            (names(&["host", "openai"]), names(&["host"])),
            (names(&["openai"]), Vec::new()),
        ] {
            let plan = before_yes(&none, Some(&stored), &claude, &[]).unwrap();

            let Some(pair) = &plan.reviewers else {
                panic!("no reviewer step for {stored:?}");
            };
            assert_eq!(pair.name, "review.reviewers");
            assert_eq!(
                pair.value,
                baley_core::policy::Value::ReviewerList(expected),
                "{stored:?}"
            );
            assert_eq!(plan.acknowledgement, None, "{stored:?}");
            assert_eq!(plan.hosts, None, "{stored:?}");
            assert_eq!(plan.settings, Vec::new(), "{stored:?}");
        }
    }

    #[test]
    fn a_settings_file_that_cannot_take_hosts_reaching_the_summary_is_caught() {
        let refusal = "install-settings-conflict: /home/o/.claude/settings.json is JSON but not an object; fix it by hand, then run baley install again".to_owned();
        let unreadable: Result<ClaudeSettings, String> = Err(refusal.clone());
        let openai = Answer::Choose(vec![Provider::OpenAi]);

        let refused = before_yes(&openai, None, &unreadable, &[]);
        assert_eq!(refused, Err(Refused(refusal.clone())));

        let object = claude(json!({"sandbox": {"network": {"allowedDomains": {"a": 1}}}}));
        let Err(Refused(line)) = before_yes(&openai, None, &object, &[]) else {
            panic!("an allowedDomains object was planned");
        };
        assert!(line.starts_with("install-settings-conflict: "), "{line}");

        let blank = before_yes(&Answer::Keep, None, &unreadable, &[]).unwrap();
        assert_eq!(blank.acknowledgement, None);
        assert_eq!(blank.hosts, None);
        assert_eq!(blank.reviewers, None);

        let stored = names(&["host", "openai"]);
        let none = Answer::Choose(Vec::new());
        let only_list = before_yes(&none, Some(&stored), &unreadable, &[]).unwrap();
        assert_eq!(only_list.acknowledgement, None);
        assert_eq!(only_list.hosts, None);
        assert!(only_list.reviewers.is_some());
    }
}
