# Baley

Baley keeps AI coding agents accountable to the person who answers for their work. You approve the plan, the agents do the work, and Baley records what they did and the proof that it works. It refuses to let the work move forward on a claim nobody has proven.

> **Status: under active redesign. Not ready to use.** The repository is public so the work can be followed as it happens. Nothing here is released, and the internal names (the crate and binary are still called `cadence`) change during the redesign.

## What it does

- **You approve before anything runs.** A plan is written, reviewed and approved by you before any agent acts on it.
- **Every step leaves evidence.** Which agent did what, against which commit, which tests ran and what they printed, which verdict was reached and why.
- **The next step has to be earned.** Baley decides what may happen next from the recorded evidence. If the proof for a step is missing, it refuses and names what is missing.
- **Plans and diffs get attacked, not just approved.** Reviews come from a model of a different family than the one that wrote the work, or from outside providers (OpenAI, Gemini, DeepSeek).
- **Your branch policy holds.** A hook checks every git command an agent runs against the branches you protect.

## What it refuses to do

- Accept "done" without a recorded proof.
- Let an agent skip a step or approve its own work.
- Send anything anywhere except to the review providers you configure. No telemetry.

## How it works

- One Rust binary. The host (Claude Code or Codex) talks to it as an MCP server over stdio; a hook runs before each of the host's tool calls; a CLI serves the owner.
- The binary owns the process. The model does the engineering; Baley owns the record and the rules about what happens next.
- Records are moving to an append-only, hash-chained event ledger in SQLite, kept outside the repository. The design is under review in [pull request #6](https://github.com/crenshawdev/baley/pull/6).

## Following the work

- **What is being built:** the [Evidence](https://github.com/crenshawdev/baley/milestone/1) milestone.
- **How it is designed:** every significant change starts as a design document with requirements and diagrams, and each architectural decision is kept as a decision record. See [the design process](docs/design/README.md), [design documents](docs/design/) and [decision records](docs/adr/).
- **How it is built:** every change reaches `main` through a pull request with passing CI (tests, clippy, cargo-deny), and signed commits are required.

## Issues and contributions

Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

## Lineage

Baley grew out of [Cadence](https://github.com/crenshawdev/cadence). History before the `baley-start` tag is Cadence's.

## License

[MIT](LICENSE)
