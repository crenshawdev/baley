# 0039: Leave provider credentials and calls with the session

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-06 |
| Deciders | John Crenshaw |
| Design document | [0002: System design](../design/0002-system-design.md), [0003: Configuration and routing](../design/0003-configuration-and-routing.md), [0008: Review](../design/0008-review.md), [0012: Host interface](../design/0012-host-interface.md) |
| Supersedes | [0013](0013-host-session-calls-outside-models.md), in part: the `baley exec --key` key path, provider command-line login, the model-detection exception and the session returning typed findings (Baley now parses the raw response); [0016](0016-key-store.md), in part: Baley holds provider credentials; [0027](0027-vendor-folders-and-plain-keys.md), in part: `keys.env`, key injection, redaction and authenticated detection; [0033](0033-host-security-bar.md), in part: provider keys stay out of agents' reach; [0028](0028-one-http-stack.md), in part: the model lister uses Baley's HTTP client; [0032](0032-gemini-is-not-a-provider.md), in part: detection looks up provider keys |
| Superseded by | |

## Context and problem

[ADR 0013](0013-host-session-calls-outside-models.md) gives outside reviews to the host session, but has Baley inject the credential through `baley exec --key` and make model-list calls itself. [ADR 0027](0027-vendor-folders-and-plain-keys.md) puts those credentials in the config folder as `keys.env`. [ADR 0033](0033-host-security-bar.md) denies session commands reads of that folder. A session command cannot use that key path while the sandbox enforces the stated boundary.

Letting Baley send reviews would change who acts on a work order. Letting session commands read a separate key folder would keep Baley handling credentials while losing the promise that agents cannot read them. A provider's headless command-line agent also needs its own writable state and can read beyond the review material under the owner's provider-tool instructions. The provider API accepts the bounded request Baley already builds.

The same separation can serve model lists: Baley needs the returned list, not the credential used to fetch it. Its shipped seed and owner catalog entries already provide models without a network call.

## Decision drivers

- Baley never reads, stores or sends an API key.
- Baley never calls a model. The host session performs every outside review call.
- Catalog setup works without a provider request.
- The owner is told what environment credentials and outside reviews expose, and explicitly acknowledges it.

## Considered options

1. Session API calls using the owner's environment credentials
2. Session calls through `baley exec --key` with a readable key folder
3. Headless provider command-line agents
4. A Baley command that sends the review itself

## Decision

Chosen option: **1**. Remove `keys.env` and the `baley exec --key` credential wrapper. The `baley exec` execution command group remains as designed in [0006](../design/0006-execution.md). Owners keep API keys in their own environment, such as `OPENAI_API_KEY` and `DEEPSEEK_API_KEY`. Baley has no key reader, key store, credential lookup or output scrubber.

**Outside reviews.** Release 1 uses provider APIs only. Baley builds the complete request and a work order naming the provider address, authentication header and environment variable name. The session sends the request with the owner's key and returns the raw response through `review return`. Baley parses and validates the findings, model and usage against the work order. The host reviewer remains a Claude Code subagent. Provider command-line agents are outside release 1. ADR 0013's rule that Baley never sends a model request stands.

**Models.** Claude Code resolves the compiled host aliases `opus`, `sonnet`, `haiku` and `fable`. API reviewer tiers use the shipped seed recorded at install and every upgrade, owner entries through `baley models add`, and imported provider lists. An explicit tier-model setting wins. Otherwise the owner's catalog entry for the tier wins; absent one, choose the newest accepted catalog id tagged for that tier by `created` date, then hint order. Build 4 records the date and hint-order inputs needed to reproduce that selection, including how absent dates and multiple owner entries are resolved. After a model-not-found or deprecated-model response, a work order tells the session to fetch the provider list with its environment key and return the raw answer. In a terminal, `baley models update` prints the fetch command for the owner to pipe into `baley models import <provider>`. Install and `baley init` use the seed without detection calls. Baley's pure list parsing, classification, tagging, diff and event code stays. Its HTTPS lister and key-name lookup go.

**Sandbox.** Installation allows the chosen providers' API hosts. It creates no key folder and grants no `~/.codex` exception. Denials over Baley's home and config folder still protect the ledger and settings. They do not protect credentials in the session's environment.

**Warning and acknowledgement.** The README, documentation and interview state these risks plainly: Baley never reads your keys. Keys live in your environment, where every program Claude Code starts can see them, agents and subagents included. Nothing scrubs a key a command prints. Your code goes to the chosen provider under its terms. The interview asks which providers to use, shows the warning and requires a typed confirmation. It records the selected providers, warning version and acknowledgement in the ledger and asks again when the warning changes. No setting or alternative entry point bypasses that acknowledgement. Install takes defaults with outside providers disabled, never an automatic acknowledgement.

## Consequences

### Positive

- Baley has no credential copy to secure and no key injection or redaction path to maintain.
- The session performs reviews and model-list requests under the same boundary.
- Seeded tiers and owner entries work from first use without a key or network request.
- The owner makes a recorded choice about provider access and its risks.

### Negative

- Environment keys are visible to programs the session starts. A printed key can enter a transcript or returned content, and Baley cannot promise to recognize or scrub it.
- Review material leaves the machine for the chosen provider under that provider's terms.
- Owners using only a provider's subscription login cannot use it for outside reviews in release 1.
- A fetched answer is relayed by the session. Parsing and validation do not prove which network request the session actually sent.

### Follow-up

- The owner must assign the build that removes the built `keys.env` reader (`crates/baley/src/keys.rs`), `baley exec --key` (`crates/baley/src/exec.rs`) and HTTPS lister and key lookup (`crates/baley/src/detection/`). No build is assigned by this decision.
- Build 3 T16 owns provider selection, the warning and typed ledger acknowledgement in the interview, including the `review.reviewers` reader and writer that setup needs. Build 4 owns `baley models import`, fetch work orders and their session return, and acknowledgement checks before review or list-fetch work orders. The pure catalog code in `crates/baley-core/src/catalog/detection/` stays.
- Build 4's review work orders and return parser follow REV-R4. Build 3's installer follows HST-R17. Neither assignment silently takes ownership of removing the built credential paths.

## Options in detail

### Session API calls with environment credentials (chosen)

Keeps credentials with the owner and calls with the session. Baley supplies and checks the material, request and returned answer. It gives up any claim that agents cannot see the credential or that output is scrubbed.

### Session calls through a readable key folder

Makes `baley exec --key` work under the sandbox, but Baley still reads credentials and agents can read the folder. It fails the no-key-handling driver without preserving the earlier protection promise.

### Headless provider command-line agents

Supports subscription login but introduces a second agent environment, writable provider state and instructions outside the review work order. It is not part of release 1.

### Baley sends the review

Avoids the session's sandbox problem by moving the call across the boundary. It requires Baley to handle a key and call a model, failing both responsibility drivers.
