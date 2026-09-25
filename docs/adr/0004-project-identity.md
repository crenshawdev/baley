# 0004: Identify projects by a committed project file

| | |
|---|---|
| Status | Proposed |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

With the ledger outside the checkout (ADR 0003), each checkout must be mapped to its project. The current store binds records to the device and inode numbers of the directories above it, which ties them to one physical location: they cannot move, be restored or be rebuilt elsewhere. The guard hook also needs a way to tell that a directory is managed, which today is the presence of `.planning/`.

## Decision drivers

- Every clone and worktree of a project maps to the same project.
- No record depends on where the checkout is on disk.
- Project policy (reviewers, routing, protected branches) reaches every clone and is reviewed when it changes.
- A cheap, local test for "is this a managed project".

## Considered options

1. Filesystem identity of the checkout (the current model)
2. The repository's root commit hash, with or without the remote URL
3. A random project id in a committed project file

## Decision

Chosen option: **3, a committed project file**, `baley.toml` at the repository root, holding a random project id (UUID version 4), the project name and the project's policy. `baley init` creates the project and the file; the owner commits it. Baley finds a checkout's project by walking up from the working directory to the first directory with the project file, as git finds a repository. The guard uses the same discovery. Checkouts are recorded with path, root commit and remote URL for diagnosis only. Two checkouts with different remotes claiming one id are refused until one is given a new id. The project id is an identity, not an authorization. The effective policy (defaults, the owner's user configuration and the project file, merged with today's precedence) is recorded in full as `policy.effective` whenever any layer changes, every command records the policy version it ran under, and each checkout runs under the project file at its own HEAD.

## Consequences

### Positive

- Records bind to a logical id; the checkout can live anywhere.
- Policy is versioned and reviewed with the code, and the full effective policy at every point is in the ledger.
- The guard's test for a managed project is one file lookup.

### Negative

- One file in the working tree, which the owner commits.
- Forks copy the id. Detected through differing remotes and resolved by giving the fork a new id.

## Options in detail

### Option 1: filesystem identity

What exists today. Records cannot survive a move, a restore or a new machine.

### Option 2: root commit hash

Needs no file and is shared by every clone, but also by every fork, with no way to split them, and does not exist until the first commit. It carries no policy, so a separate committed file would still be needed.

### Option 3: committed project file

One small file gives identity, policy and the managed-project marker together.
