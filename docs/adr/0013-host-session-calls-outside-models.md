# 0013: Let the host session call outside models, never Baley

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0002: System design](../design/0002-system-design.md), [0008: Review](../design/0008-review.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

Reviews and model detection reach outside providers (OpenAI, Gemini, DeepSeek, Anthropic by API). Today's engine calls the review providers itself over HTTPS with keys it reads from a file, from inside a tool call that can run for minutes and freezes the server's single queue while it waits.

Baley's parties are the owner, Baley and the model (0002). Baley decides which review runs and builds the prompt; the judgment is the model's. A call to an outside model is engineering judgment being sought, not a process decision.

## Decision drivers

- Responsibility stays with the party that acts: Baley decides, models judge.
- The server never blocks on a network call it does not control.
- No owner is forced to hand Baley a key; a provider's own command-line login must work.
- Keys never enter a conversation or transcript.

## Considered options

1. Baley calls providers itself (today)
2. Baley calls providers in a background task and the session polls
3. The host session makes every outside call with the complete prompt and request Baley built; keys reach the call only through `baley exec --key`

## Decision

Chosen option: **3**. Baley decides whether an outside review runs and with which providers, builds the complete prompt and request as a work order, and hands it to the host session. The session makes the call, through the provider's own login or through `baley exec --key <provider> -- <command>`, which sets the key in that one process's environment and replaces every occurrence of it in the output before the model sees it, and returns the typed findings and the provider's reported model and usage. Baley has no port to outside models. The one exception is model detection (0003): a list call with a stored key, no prompt, no content, made by Baley because it is Baley's own bookkeeping.

## Consequences

### Positive

- The server never waits on the network; the session does, under its own host's rules.
- An owner with a command-line login and no key is a first-class user.
- Every outside call is a recorded work order with a recorded result, like a worker's.

### Negative

- The session must relay the call faithfully; the record checks that what came back matches the request it built.
- `baley exec` is a new command with a redaction rule to get right.

### Follow-up

- `baley exec --key` and its redaction are built with the key store (0003).
- Provider adapters (request shape per provider) move from the server into the work order composer.

## Options in detail

### Baley calls providers itself

Simplest to wire, but the server blocks, the keys must live where Baley can read them as a file, and Baley becomes the party that judged. Fails three drivers.

### Background task and polling

Removes the blocking, keeps the key handling and the responsibility problem. Fails two drivers.

### Host session calls (chosen)

The session already relays work orders to workers; an outside call is one more. Keys stay in Baley's store and are injected per process.
