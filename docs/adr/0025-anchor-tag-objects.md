# 0025: Point anchor tags at the empty tree

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-27 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

[ADR 0007](0007-forge-anchors.md) anchors a project's ledger head on its forge as an immutable annotated tag. The annotation names a ledger sequence and hash. These facts have no necessary relationship to a source commit, but a git tag object must point at an object and carry a tagger identity and time.

The CLI must push and fetch anchors without creating local tag references or transferring source history. The tagger must identify the tool that made the anchor without depending on the owner's git identity settings.

## Decision drivers

- Keep the outside witness immutable under the forge's tag rules.
- Keep ledger anchors out of source version descriptions.
- Transfer only the objects needed for an anchor.
- Work in a repository with no source commits or configured user identity.

## Considered options

1. Annotated tag on the empty tree, with a fixed Baley tagger.
2. Annotated tag on the current source commit, with the owner's tagger.
3. Synthetic commit carrying the anchor, with a Baley tagger.

## Decision

Chosen option: **annotated tag on the empty tree**, with tagger `Baley <baley@localhost>`, the push's Unix time and offset `+0000`. The annotation remains the core's canonical ledger-head JSON. This follows ADR 0007's tag name and immutability policy.

The binary first requires a configured remote and resolves its push destination with `git remote get-url --push --all REMOTE`. Exactly one non-empty URL is required. An empty, multiple or failed lookup is unreachable and stops before any object write. Git applies `pushurl` and `pushInsteadOf` during this lookup.

The binary writes the empty tree with `git hash-object -t tree -w --stdin`, writes the tag object with `git mktag`, and pushes its object id with `git push --porcelain --no-verify PUSH_URL SHA:refs/tags/TAG`. Pushing to the URL avoids the named remote's fetch mappings, which can otherwise create local tags after a push. It creates no local ref. Fetch disables tag following, configured ref mappings and `FETCH_HEAD` writes, then reads the fetched object with `git cat-file tag`.

## Consequences

### Positive

- A tree tag is not a source commit candidate for `git describe`.
- Fetch transfers the tag and empty tree, without commit history.
- Anchoring works before the repository has a source commit and does not read user identity settings.
- No local tag ref competes with the remote witness.

### Negative

- Settings keyed to the remote's name, including `remote.<name>.push` and `remote.<name>.receivepack`, do not apply to the anchor push. A remote with several push URLs cannot anchor.
- A forge's presentation of a tag on a tree may be less useful than its presentation of a release tag on a commit.
- The tagger names Baley, not a cryptographically authenticated person. The forge's access and immutability rules still matter.
- Objects are written into the checkout's object store even though no local ref is retained.

### Follow-up

- The first owner push goes to a throwaway repository to inspect how the forge presents the tree tag. A local bare-repository check does not establish GitHub's presentation.
- Slice 2 checks the forge's tag ruleset when a project starts.

## Options in detail

### Annotated tag on the empty tree

The empty tree is a fixed object in each git object format. A tag points at it and carries the ledger head in its annotation. It keeps the witness independent of source history and requires only two objects to fetch.

### Annotated tag on the current source commit

This looks familiar in forge release views, but couples a ledger checkpoint to whichever source commit happens to be checked out. Fetching it can transfer history, and local refs would affect source version descriptions. It also needs a commit and an identity choice unrelated to the ledger evidence.

### Synthetic commit carrying the anchor

A synthetic commit can carry arbitrary data, but adds an object and a commit identity without improving the witness. It introduces commit semantics where the design needs only an immutable annotation.
