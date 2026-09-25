# 0003: Keep one ledger database per user, outside any checkout

| | |
|---|---|
| Status | Proposed |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

The current store lives in each checkout's `.planning/` directory and is gitignored. Deleting or re-cloning the checkout deletes the records, each git worktree gets its own store, and the directory clutters the working tree. The ledger is Baley's historical record, so where it lives decides how long that record survives.

## Decision drivers

- The record must outlive any checkout.
- All worktrees and clones of one project must see one history.
- Nothing of Baley's in the working tree beyond one committed file.
- Simple backup, migration and cross-project queries.
- Development builds and tests must never touch the owner's real records.

## Considered options

1. In the checkout, committed (`.baley/` tracked in git)
2. In the checkout, ignored (`.baley/` in `.gitignore`)
3. Inside the repository's git directory (`.git/baley/`)
4. One database per project in the user's data directory
5. One database per user for all projects, in the user's data directory

## Decision

Chosen option: **5, one database per user**, at `<data dir>/baley/baley.db`: `$XDG_DATA_HOME/baley` or `~/.local/share/baley` on Linux, `~/Library/Application Support/baley` on macOS, `%APPDATA%\baley` on Windows. `BALEY_HOME` overrides it for development builds and tests. The database carries a schema version; a binary that finds a newer schema opens it read-only, and migrations run in one transaction after an automatic backup.

## Consequences

### Positive

- History survives deleting, moving or re-cloning a checkout.
- Every clone and worktree of a project maps to one history through its project id (ADR 0004).
- One file to back up and one schema to migrate.
- Questions across projects (what is in flight everywhere, routing learned across projects) are single queries.

### Negative

- One file holds every project, so corruption would affect all of them. Mitigated by `quick_check` on open, `integrity_check` and verified backups, and per-project hash chains that let one project be exported and verified alone.
- All binaries on the machine share one schema. Handled by the read-only rule for newer schemas and by `BALEY_HOME` for development builds.
- A cloud session on another machine has no history. The same is true of every local option; transport is future work.
- The MCP server and guard hook must be allowed to write outside the repository on both supported hosts. To be confirmed before acceptance.

## Options in detail

### Option 1: committed in the checkout

Records travel with the code, but a database file cannot be diffed or merged, branches conflict, the repository grows, and material sent to reviewers would enter git history.

### Option 2: ignored in the checkout

Easy to find, but still in the working tree, one store per worktree, deleted with the clone and by `git clean -x`.

### Option 3: inside `.git/`

Never committed, shared by all worktrees through the common git directory, no identity problem. Deleted with the clone, which is disqualifying for a historical record.

### Option 4: one database per project in the data directory

Contains the blast radius and makes export a file copy. Loses cross-project queries, multiplies migrations and backups, and still needs project identity. Per-project chains and `export` recover its main advantage within option 5.

### Option 5: one database per user

Outlives checkouts, serves every worktree, keeps the tree clean, and makes the owner's control plane one place.
