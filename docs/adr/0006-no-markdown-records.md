# 0006: Keep every operational record in the ledger

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

The binary renders Markdown copies of its records into `.planning/` and reads Markdown as input: phases exist only as lines in `ROADMAP.md`, execution parses its tasks out of `PLAN-k.md`, requirements live in a table in `REQUIREMENTS.md`, and two features read old Markdown out of git history. Each file is a second source of truth that must be kept in agreement with the store, and the store's transaction journal exists largely to write those files atomically with it.

## Decision drivers

- One source of truth for everything that controls or informs Baley's operation.
- The owner and the agents can still read any record.
- Files that other programs must read keep their formats.

## Considered options

1. Keep Markdown as rendered copies and inputs (the current model)
2. Keep Markdown as write-only exports, rendered on every change
3. Everything operational in the ledger; files only by explicit export

## Decision

Chosen option: **3**. Everything that controls or informs Baley's operation lives in the ledger: roadmap, requirements, context, plans, summaries, verification, captures, diagnostics. The owner reads records with `baley show` and writes a file only with an explicit `baley export`, which is never read back. Agents read records through the MCP `document` query. Three kinds of file stay files, because their readers are not Baley:

- files the host itself loads (skills and agent definitions), whose source is compiled into the binary;
- documents for people in the repository (design documents, decision records, the README);
- the committed project file (ADR 0004).

## Consequences

### Positive

- No copies to keep in agreement, and no multi-file transactions.
- Execution reads typed plans, not parsed Markdown.
- The roadmap becomes data with operations, which supplies the missing "add a phase" command.
- Nothing is deleted when a milestone closes, so history needs no git archaeology.

### Negative

- Records are no longer browsable as files in the repository; reading them takes a command.
- Every place that reads or writes Markdown today must be replaced, which is a large part of the migration.

## Options in detail

### Option 1: current model

Two sources of truth and the machinery to reconcile them.

### Option 2: write-only exports on every change

Removes parsing, but keeps a copy that goes stale the moment someone edits it, and keeps Baley writing into the working tree.

### Option 3: ledger only

One source of truth; reading is a command away.
