# 0032: Drop Gemini from the model catalog and detection

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-30 |
| Deciders | John Crenshaw |
| Design document | [0003: Configuration and routing](../design/0003-configuration-and-routing.md) |
| Supersedes | [0027](0027-vendor-folders-and-plain-keys.md), in part: the compiled key-name table names `OPENAI_API_KEY` and `DEEPSEEK_API_KEY` only |
| Superseded by | [0039](0039-session-owned-provider-credentials.md), in part: detection looks up provider keys |

## Context and problem

Detection refreshes each provider's model catalog from the provider's list endpoint (design 0003, CFG-R20). It was first built for OpenAI, Gemini and DeepSeek. Gemini differed from the other two in every part of the request: its key went in an `x-goog-api-key` header, its list came in pages joined by a `nextPageToken` that the next request sent back as a `pageToken` query parameter, the lister needed a bound of 20 pages, and a bad key answered 400 with the reason `API_KEY_INVALID` instead of 401. OpenAI and DeepSeek each answer their whole list in one response, with the key in an `authorization: Bearer` header.

After those parts were built, and before the design was updated to match, Gemini was dropped as a provider Baley supports. The question is how far the removal goes. The model catalog, the key-name table ADR 0027 compiled into Baley, the `baley models` commands and detection all name Gemini. So does the inherited review engine, which has its own Gemini module and `review.providers.gemini.*` settings, and which Build 4 replaces.

## Decision drivers

- Every provider the catalog names is one detection fills.
- No code path is kept for a provider nothing uses.
- The fewest ways a key, or anything a provider sent, can reach a URL or a `Debug` print.
- Build 2 does not change the inherited review engine Build 4 replaces.

## Considered options

1. Remove Gemini from the catalog and detection only
2. Keep a Gemini catalog for owner entries alone
3. Keep the page loop for a future provider that pages
4. Also remove the review engine's Gemini module now

## Decision

Chosen option: **1**. The providers are OpenAI and DeepSeek. Every `baley models` command refuses `gemini` as `unknown-provider`, as it refuses any unknown name, and `baley models update gemini` refuses before any store opens. The Gemini list parser, the `API_KEY_INVALID` rule, the `x-goog-api-key` header and the `GEMINI_API_KEY` row of the key-name table are gone. Since neither remaining provider pages, the continuation and the page bound are gone too: one request per provider is the whole listing, and the 4 MiB body bound is the only bound. The provider type and the key-name table keep their shape. The review engine keeps its Gemini module until Build 4 replaces the engine.

## Consequences

### Positive

- Nothing a provider sends is carried into a later request, so the key header is the only part of a list request that holds a secret, and no token can reach a URL or a request's `Debug`.
- One request per provider, and a lister with no loop over pages.
- The catalog names only providers detection fills.

### Negative

- Detection no longer reads a `GEMINI_API_KEY` line in `keys.env`. Such a line is read only when `baley exec --key GEMINI_API_KEY` injects it into one command (CFG-R27).
- The review engine, design 0008, the Outside reviewers system of design 0002 and the C4 model, and ADR 0013's provider list still name Gemini until Build 4.
- A ledger event naming a `gemini` catalog would be refused by the catalog projector. None exists: Baley has no release, and every store so far is a test's.

### Follow-up

- Build 4 replaces the review engine and its Gemini module.
- Adding a provider later takes a provider variant, a key-name row, an endpoint row in the list request, a parser arm if its list shape differs, and hint rows.

## Options in detail

### Remove Gemini from the catalog and detection only (chosen)

Meets every driver. The removal stays inside the catalog, the key-name table, the `baley models` commands and detection, and the code left is exactly what the two remaining providers use. The review engine is left to the build that replaces it.

### Keep a Gemini catalog for owner entries alone

The owner could still add names to a `gemini` catalog by hand. No detection would fill it and no host routes to it, so it would be a catalog the design has to explain and nothing reads. It fails the first two drivers.

### Keep the page loop for a future provider that pages

The continuation, the page bound and the loop would stay for a provider nobody has named. CFG-R23 would describe a bound nothing reaches, and the continuation is the one place where text from a provider's response was placed in the next request's URL. It fails the second and third drivers.

### Also remove the review engine's Gemini module now

Build 4 replaces the review engine, and the detection task lists no review change. Removing the module now would change code about to be replaced, with its own settings and tests, for no gain in detection. It fails the fourth driver.
