# 0019: Run every configured reviewer, adjudicate in the host session, and let the owner rule on each finding

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0008: Review](../design/0008-review.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

Cadence had three review modes (single, panel, adjudicated), gates that waited on a settlement nothing produced, and consult as a second paid path to the same providers. Baley's parties are owner, Baley and model; the host session has two jobs, relay and adjudicate (0002). The owner's rule for the product: every finding goes to the owner, the owner rules, Baley never applies a finding, reruns a review or re-plans on its own.

## Decision drivers

- Every configured voice is heard and a missing one is visible.
- The owner rules on verified claims in plain words, never on raw output.
- A fix goes through the same gates as any change.
- One mechanism for every critique.

## Considered options

1. Keep the three modes and add a ruling step
2. Every reviewer runs; the host session adjudicates; the owner rules; a fix produces a plan revision or a gap plan; on-demand kinds under one command

## Decision

Chosen option: **2**. `review.mode` is removed: every reviewer in `review.reviewers` runs on every triggered review. Before any finding reaches the owner, the host session checks it against the code, drops what does not hold with the reason recorded, merges duplicates and brings each survivor with fix options. The owner rules `fix`, `track` or `dismiss` on each; the ruling clears the gate; a `fix` produces a plan revision (plan review) or a gap plan (diff or risk review) and grants one more round, never a third. Minimalism, decision and diagnosis reviews are kinds of the one `review` command with no gate; consult is replaced by the diagnosis kind. Tracked findings are filed on the forge only on the owner's word, with a fingerprint lookup before each create.

## Consequences

### Positive

- One review path, one findings shape, one ruling record, for triggered and on-demand reviews alike.
- Gates clear by a recorded ruling, never by a settlement nothing writes.
- The owner sees fewer, verified findings.

### Negative

- The host session does real work in adjudication; its instructions must hold it to checking, not deciding.
- Two review rounds at most may leave a finding tracked rather than fixed; that is the owner's call.

### Follow-up

- Filing (GitHub first) is built after the ruling record exists.

## Options in detail

### Three modes plus a ruling step

Keeps a setting that no longer changes behaviour once every review is adjudicated and ruled.

### Every reviewer runs, adjudicate, rule (chosen)

One fewer setting, one path, the owner's rule for the product written as mechanism.
