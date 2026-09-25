# 0009: Serve instructions from the binary; files on disk are stubs

| | |
|---|---|
| Status | Accepted |
| Date | 2026-09-25 |
| Deciders | John Crenshaw |
| Design document | [0001: The evidence ledger](../design/0001-evidence-ledger.md) |
| Supersedes | |
| Superseded by | |

## Context and problem

ADR 0006 keeps two kinds of file outside the ledger because their readers are not Baley: documents for people, and the files a host loads itself, skills and agent definitions. It says their source is compiled into the binary. It does not say how much of them is on disk.

Today all of it is. The binary renders twenty-four `SKILL.md` files from compiled text through its `*-instructions` commands, and two more are kept by hand. Together they are 304 KiB and 3,917 lines of prose; the planner's file alone is 1,251 lines. The prose is correct the moment it is rendered and drifts from then on: a user who upgrades the binary keeps last version's instructions until something renders them again, and a bug report can name a prompt the maintainer no longer ships. The same rule that refused user-edited prompts, that a report must mean the prompt the maintainer runs, is broken from the other side by a stale copy.

The files also invite the wrong kind of reading. A model with a folder of long Markdown searches it, and every search is tokens spent finding text the binary could have handed over by name. The read layer already answers this for records: a bounded part comes back by identity and nothing is scanned. Instructions are the one surface the model still reads from disk.

## Decision drivers

- One source of every instruction the model sees, versioned with the code that enforces it.
- A file on disk can never be out of date with the running binary.
- The model asks for what it needs and receives exactly that; it never searches for instructions.
- Whatever a host requires on disk stays, and stays as small as the host allows.
- The ledger can say which instructions a dispatch ran under.

## Considered options

1. Render the full instructions to disk (the current model)
2. Keep the instructions in the ledger
3. Serve the instructions from the binary; render only a stub

## Decision

Chosen option: **3**. Every instruction the model sees is compiled into the binary and served over the MCP connection as addressed, bounded parts, the way records are served today. The only Markdown a host loads is a stub: the frontmatter the host reads to list and start a command, and one line naming the query that returns the command's instructions. The stub carries no instruction of its own. The binary renders every byte of it, frontmatter included, from its command table; nobody edits a stub, and a stub that grows a paragraph is a defect.

Stubs are written at install by the plugin or by `baley init`, not committed. Where a host can only load a skill from a committed directory, the committed stub is checked in CI against the binary's rendering and fails the build on a single byte of difference.

Each served instruction carries an identity, a version and a hash of its text. Every dispatch and front-door call records that hash in the ledger, so the ledger answers "what was this agent told" and the binary of that version reproduces the text. The ledger holds the fact that a prompt was used, never the prompt.

## Consequences

### Positive

- One instruction source. A wording change is a Rust diff, reviewed once, shipped with the binary that enforces it.
- Nothing on disk to go stale. The prose is whatever the running binary says it is.
- The model makes one call and gets the text by name, in parts sized for reading, not a folder to search.
- The repository loses 304 KiB of rendered prose and the machinery that checks it.
- A dispatch's instructions are provable after the fact from the ledger and the binary version.

### Negative

- Each front door costs one tool call before the model can act, on every invocation.
- The model obeys text that arrived as a tool result rather than as the skill body. The binary's refusals carry the discipline either way, as they do today for the executor and verifier contracts.
- A wording change still needs a release. That price was accepted when prompts were compiled in and it stays accepted.

### Follow-up

- Confirm per host whether a skill can be installed from a rendered directory or must be committed. The answer decides which of the two stub paths each host takes. Recorded in the host matrix of design 0001 at the next run.
- The instruction identity, version and hash become an event field in the lifecycle slice, where the first Baley front doors are built.
- The twenty-six rendered and hand-kept files leave with the old crate in the removal slice.

## Options in detail

### Option 1: render the full instructions to disk

What the binary does today. One source, but a copy on every machine, and copies drift. The model has the whole file in front of it and reads or searches it as it likes. Fails the drift driver and the no-searching driver.

### Option 2: keep the instructions in the ledger

The store is the project's evidence; instructions are the program. Put them in SQLite and the store becomes editable prompt text, which is the user override refused in the compiled-in decision, reached through a database editor instead of a file. The prose would version with the store instead of with the code that enforces it, so an upgrade migrates rows of Markdown. And every fresh store would be seeded from the binary anyway, which makes the binary the source and the store a copy. Fails the one-source driver. What survives from this option is the provenance row: the ledger records which instructions were used, by hash.

### Option 3: serve from the binary, stub on disk

One source, nothing to drift, text by name in bounded parts, and a host still finds the command. Costs one call per invocation.
