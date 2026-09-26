# 0015: Keep settings in TOML: one global file, one project file, host sections

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-26 |
| Deciders | John Crenshaw |
| Design document | [0002: System design](../design/0002-system-design.md), [0003: Configuration and routing](../design/0003-configuration-and-routing.md) |
| Supersedes |  |
| Superseded by | |

## Context and problem

Today's engine reads two JSON files, one under the repository's `.planning` directory and one under the home directory, on every request. Baley removes `.planning` and every Markdown and JSON record; only the ledger holds records. Settings are not records: the owner writes them, reviews them and commits the project's to the repository. A hand-edited file needs comments and a format people already read. Some settings differ per host (which models exist on Claude Code and on Codex), some belong to the repository (branches, forge), and some to the user (roles, keys).

## Decision drivers

- The owner sets policy in a file a person can read, comment and diff.
- Repository facts travel with the repository; user choices stay with the user.
- Per-host differences have a home without a third file.
- The model never reads or writes a settings file; Baley records the settings it acted under.

## Considered options

1. Settings as ledger records edited only through commands
2. JSON files as today
3. TOML: one global file per user, one committed project file, `[host.<name>]` sections in each

## Decision

Chosen option: **3**. Settings are TOML in two files: `~/.config/baley/baley.toml` (Linux) or `~/Library/Application Support/baley/baley.toml` (macOS) for everything shared across projects, and `baley.toml` at the repository root, committed, for the project (ADR 0004). Each setting has a scope; branch, forge and repository settings and the test command are project-only. A `[host.<name>]` section in either file applies only when that host is connected. Layers merge defaults, global, global host section, project, project host section. Baley's command line and interview write the files; the guard refuses agent writes; the ledger records `policy.effective` whenever the merged result changes, and every command records the policy version it ran under.

## Consequences

### Positive

- Familiar, reviewable settings, with comments, in the repository where they apply.
- Per-host models and effort without a third file.
- The record says what Baley acted under, whatever the files say now.

### Negative

- Two files to find, and a scope rule to enforce at write time.
- A committed project file is readable by agents in the repository (writes are refused; reads are not secret).

### Follow-up

- The settings interview writes both files; `baley doctor` reports scope diagnostics.

## Options in detail

### Settings as ledger records

One store for everything, but the owner cannot review or commit policy with the code, and a fresh clone has no policy until the owner re-enters it. Fails the first two drivers.

### JSON files

No comments, and the two locations were tied to `.planning`, which is gone. Fails the first driver.

### TOML, global and project, host sections (chosen)

Comments, diffs, commits, scopes and per-host sections in one format.
