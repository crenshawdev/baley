//! The tables compiled into the binary: each host's aliases and the hint
//! table (design 0003, CFG-R19 and CFG-R20).

use super::Provider::{DeepSeek, Gemini, OpenAi};
use super::Tier::{Balanced, Cheap, Flagship};
use super::{Provider, Tier};
use crate::policy::Host;

/// The short names a host resolves itself. They live only here, never in the
/// view, so a later binary's alias change reaches every catalog at once.
/// Codex has none until its host adapter fills them in.
pub fn host_aliases(host: Host) -> &'static [&'static str] {
    match host {
        Host::ClaudeCode => &["opus", "sonnet", "haiku", "fable"],
        Host::Codex => &[],
    }
}

/// The hint table's version. Raise it whenever any row changes: seeding
/// records the table again exactly when this number differs from the latest
/// recorded one.
pub const HINT_VERSION: u64 = 1;

/// A hint for one exact model id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HintRow {
    /// The provider that serves the id.
    pub provider: Provider,
    /// The model id, as the provider's list endpoint returns it.
    pub id: &'static str,
    /// The id's tier.
    pub tier: Tier,
    /// Whether the model accepts high effort.
    pub high_effort: bool,
}

/// A hint for every id that starts with a prefix and no exact row names.
/// Detection uses these; seeding never records them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefixRow {
    /// The provider that serves the ids.
    pub provider: Provider,
    /// The start of the ids it tags.
    pub prefix: &'static str,
    /// Their tier.
    pub tier: Tier,
    /// Whether they accept high effort.
    pub high_effort: bool,
}

const fn exact(provider: Provider, id: &'static str, tier: Tier, high_effort: bool) -> HintRow {
    HintRow {
        provider,
        id,
        tier,
        high_effort,
    }
}

const fn prefix(
    provider: Provider,
    prefix: &'static str,
    tier: Tier,
    high_effort: bool,
) -> PrefixRow {
    PrefixRow {
        provider,
        prefix,
        tier,
        high_effort,
    }
}

/// The exact-id rows, from each provider's published model list.
pub const EXACT_HINTS: &[HintRow] = &[
    exact(OpenAi, "gpt-6-astra", Flagship, true),
    exact(OpenAi, "gpt-6.1-sol", Balanced, true),
    exact(OpenAi, "gpt-6-luna", Cheap, true),
    exact(Gemini, "gemini-3.8-flash", Flagship, true),
    exact(Gemini, "gemini-3.1-pro-preview", Flagship, true),
    exact(Gemini, "gemini-3.7-flash", Balanced, true),
    exact(Gemini, "gemini-3.6-flash", Balanced, true),
    exact(Gemini, "gemini-3.5-flash", Balanced, true),
    exact(Gemini, "gemini-3.5-flash-lite", Cheap, true),
    exact(Gemini, "gemini-3.1-flash-lite", Cheap, false),
    exact(DeepSeek, "deepseek-v4-pro", Flagship, true),
    exact(DeepSeek, "deepseek-flash", Cheap, true),
];

/// The prefix rows, for dated snapshots and retired names of the same
/// families.
pub const PREFIX_HINTS: &[PrefixRow] = &[
    prefix(OpenAi, "gpt-6-astra", Flagship, true),
    prefix(OpenAi, "gpt-6.1-sol", Balanced, true),
    prefix(OpenAi, "gpt-6-sol", Balanced, true),
    prefix(OpenAi, "gpt-6-luna", Cheap, true),
    prefix(Gemini, "gemini-3.1-pro", Flagship, true),
    prefix(Gemini, "gemini-3.5-flash-lite", Cheap, true),
    prefix(DeepSeek, "deepseek-v4-pro", Flagship, true),
    prefix(DeepSeek, "deepseek-v4-flash", Cheap, true),
];
