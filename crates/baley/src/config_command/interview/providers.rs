//! The provider stage of the interview (design 0003 section 5, Provider
//! selection and Provider confirmation). It is pure over the input and output
//! it is given, and its one seam is the process that reads git's `user.name`.

use std::io::{self, BufRead, Write};

use baley_core::catalog::Provider;

use super::super::reviewers::Selection;
use super::{accepts, read_answer};
use crate::ledger::display::Render;
use crate::process::Process;
use crate::providers::identity;
use crate::providers::warning::{WARNING_TEXT, WARNING_VERSION};

/// The providers the owner explicitly confirmed and the name to record them
/// under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Acknowledge {
    /// The confirmed providers, each once, in catalog order.
    pub(super) providers: Vec<Provider>,
    /// git's `user.name`.
    pub(super) owner: String,
}

/// A refusal before the summary, compared by what it prints and how it exits.
#[derive(Debug)]
pub(super) struct Refusal(pub(super) Render);

impl PartialEq for Refusal {
    fn eq(&self, other: &Refusal) -> bool {
        (&self.0.lines, self.0.code, self.0.error) == (&other.0.lines, other.0.code, other.0.error)
    }
}
impl Eq for Refusal {}

/// How the provider stage ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Staged {
    /// A warning was not accepted or the input ended, so the interview stops.
    Declined,
    /// The owner identity is missing, so the interview stops before its summary.
    Refused(Refusal),
    /// Every warning shown was accepted.
    Ready(Ready),
}

/// The owner's answer to the provider question and, when providers were
/// chosen, what to acknowledge.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Ready {
    /// What the owner answered.
    pub(super) answer: Answer,
    /// The confirmed providers with the owner's name, `None` when no provider
    /// was chosen explicitly.
    pub(super) acknowledgement: Option<Acknowledge>,
}

/// What the owner answered to the provider question.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Answer {
    /// Blank: keep the stored selection and request no acknowledgement.
    Keep,
    /// An explicit choice: each provider once, in catalog order. Empty is the
    /// explicit choice of none.
    Choose(Vec<Provider>),
}

/// Reads an answer to the provider question. Names are `openai` and
/// `deepseek`, separated by commas, spaces or both, and `none` stands alone.
/// Case is not folded, so `OpenAI` is not a name, and `host` is the host's own
/// reviewer, not an outside provider. A refusal is the line that asks again.
pub(super) fn parse_answer(text: &str) -> Result<Answer, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Answer::Keep);
    }
    if text == "none" {
        return Ok(Answer::Choose(Vec::new()));
    }
    let mut named = Vec::new();
    for word in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|word| !word.is_empty())
    {
        match Provider::parse(word) {
            Some(provider) => named.push(provider),
            None => return Err(refusal(text)),
        }
    }
    if named.is_empty() {
        return Err(refusal(text));
    }
    Ok(Answer::Choose(
        Provider::ALL
            .into_iter()
            .filter(|provider| named.contains(provider))
            .collect(),
    ))
}

fn refusal(text: &str) -> String {
    format!(
        "\"{}\" is not an answer; answer openai, deepseek, both separated by a comma, none, or press Enter to keep the stored choice",
        text.escape_debug()
    )
}

/// Asks the provider question, shows each chosen provider's warning and takes
/// its typed `yes`, then reads the owner identity.
///
/// Only an explicit choice of a provider asks for its warning, and it does so
/// even when the stored selection already holds that provider. Any answer but
/// `yes`, a blank or ended input declines the whole interview without asking
/// again.
pub(super) fn stage(
    stored: &Selection,
    process: &mut dyn Process,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<Staged> {
    writeln!(
        output,
        "The next question chooses the outside review providers: answer openai, deepseek, both separated by a comma, none for no outside provider, or press Enter to keep the stored choice."
    )?;
    let chosen = loop {
        write!(
            output,
            "outside providers are {} from {}: ",
            stored.text(),
            stored.layer
        )?;
        output.flush()?;
        let Some(answer) = read_answer(input)? else {
            // The prompt holds the line, so end it before anything else prints.
            writeln!(output)?;
            return Ok(Staged::Declined);
        };
        match parse_answer(&answer) {
            Ok(Answer::Keep) => {
                return Ok(Staged::Ready(Ready {
                    answer: Answer::Keep,
                    acknowledgement: None,
                }));
            }
            Ok(Answer::Choose(chosen)) => break chosen,
            Err(line) => writeln!(output, "{line}")?,
        }
    };
    if chosen.is_empty() {
        return Ok(Staged::Ready(Ready {
            answer: Answer::Choose(chosen),
            acknowledgement: None,
        }));
    }
    for provider in &chosen {
        let name = provider.name();
        writeln!(output, "Warning for {name} (version {WARNING_VERSION}):")?;
        writeln!(output, "{WARNING_TEXT}")?;
        write!(output, "Type yes to accept this warning for {name}: ")?;
        output.flush()?;
        let answer = read_answer(input)?;
        if answer.is_none() {
            writeln!(output)?;
        }
        if !accepts(answer.as_deref()) {
            return Ok(Staged::Declined);
        }
    }
    Ok(match identity::read(process) {
        Ok(owner) => Staged::Ready(Ready {
            acknowledgement: Some(Acknowledge {
                providers: chosen.clone(),
                owner,
            }),
            answer: Answer::Choose(chosen),
        }),
        Err(render) => Staged::Refused(Refusal(render)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_answer_read_loosely_or_a_repeat_counted_twice_is_caught() {
        let open = Answer::Choose(vec![Provider::OpenAi]);
        let both = Answer::Choose(vec![Provider::OpenAi, Provider::DeepSeek]);
        for (text, expected) in [
            ("openai", open),
            ("deepseek, openai", both),
            (
                "openai,deepseek",
                Answer::Choose(vec![Provider::OpenAi, Provider::DeepSeek]),
            ),
            ("openai openai", Answer::Choose(vec![Provider::OpenAi])),
            ("none", Answer::Choose(Vec::new())),
            ("", Answer::Keep),
        ] {
            assert_eq!(parse_answer(text), Ok(expected), "{text:?}");
        }
        for text in [
            "OpenAI",
            "host",
            "none, openai",
            "anthropic",
            "openai;deepseek",
        ] {
            let line = parse_answer(text).expect_err(text);
            for word in ["openai", "deepseek", "none"] {
                assert!(line.contains(word), "{text:?}: {line}");
            }
        }
    }
}
