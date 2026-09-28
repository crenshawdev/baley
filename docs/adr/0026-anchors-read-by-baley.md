# 0026: Anchors are read by Baley, and a missing tag ruleset is reported

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-27 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | [0007](0007-forge-anchors.md) in part |
| Superseded by | |

## Context and problem

[ADR 0007](0007-forge-anchors.md) lists as a benefit that anchors are visible, dated and ordered in the forge's own history, and it requires a tag ruleset that forbids moving or deleting tags. [ADR 0025](0025-anchor-tag-objects.md) points anchor tags at the empty tree.

The first anchor pushed to GitHub (git 2.55, 2026-09-27) showed two things. GitHub's web interface does not list a tag that points at a tree: the repository's Tags page omits it and its release URL returns 404, while an annotated tag on a commit in the same repository is listed. Git and the REST API still return the tag, its tagger and its annotation. Second, GitHub refuses to create a ruleset on a private repository without a paid plan (HTTP 403). On a public repository the ruleset held: moving and deleting an anchor tag were both rejected, and a new anchor still pushed.

## Decision drivers

- The anchor exists so that `verify` can compare the local chain with a witness outside the machine.
- Keep the empty-tree tag of ADR 0025 and its independence from source history.
- A project on a plan that cannot hold a ruleset still gains from an outside witness.

## Considered options

1. Anchors are read by Baley through git; a missing ruleset is reported and anchoring goes on.
2. Move anchors to a synthetic commit so the forge's web interface lists them.
3. Refuse to anchor where the forge cannot hold a tag ruleset.

## Decision

Chosen option: **1**. Anchors are for Baley to read through git, not for people to browse on the forge. The benefit in ADR 0007 that anchors are visible in the forge's own history no longer holds and is not pursued. ADR 0025's empty-tree tag stays.

A tag ruleset is still wanted on every anchored repository. Where the forge cannot hold one, as on a private GitHub repository without a paid plan, Baley still anchors, and the project start check and `doctor` report the tags as unprotected. This replaces ADR 0007's requirement that the ruleset be in place.

## Consequences

### Positive

- Private repositories on free plans get an outside witness instead of none.
- Anchors stay out of source history, release views and version descriptions.

### Negative

- The owner cannot see anchors on the forge's web pages. `git ls-remote --tags` and the forge's API show them.
- On a repository without a ruleset, anyone with push access can move or delete an anchor. Verification then reports a malformed, missing or older anchor, never a silent pass, but the witness is weaker than ADR 0007 intended.

### Follow-up

- The project start check (PRJ-R8) and `doctor` report whether the tag ruleset is in place, and name an unprotected repository as such. Both arrive with the slice that builds PRJ-R8.

## Options in detail

### Option 1: read by Baley, report a missing ruleset

Keeps the anchor format and every built command unchanged. The cost is visibility and, on some plans, protection, and both are stated where the owner looks.

### Option 2: synthetic commit

GitHub lists tags on commits. A commit per anchor would have to live on some ref or be reachable only through its tag, adds history to fetch, and reopens the choices ADR 0025 settled.

### Option 3: refuse without a ruleset

Keeps ADR 0007's guarantee intact where anchoring happens, but leaves private projects on free plans with only the local chain, which protects against accidents and nothing more.
