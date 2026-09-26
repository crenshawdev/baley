# Baley

Baley keeps AI coding agents accountable to the person who answers for their work. You approve the plan, the agents do the work, and Baley records what they did and the proof that it works. It refuses to let the work move forward on a claim nobody has proven.

> **Status: designed, now being built. Not ready to use.** The repository is public so the work can be followed as it happens. Nothing here is released, and the internal names (the crate and binary are still called `cadence`) change before the first release.

## What it does

- **You approve before anything runs.** Work is a backlog of stories, each with the acceptance criteria you agreed, committed to sprints. A sprint's plan is written, checked and approved by you before any agent acts on it.
- **Every step leaves evidence.** Which agent did what, against which commit, which tests ran and what they printed, which verdict was reached and why.
- **The next step has to be earned.** Baley decides what may happen next from the recorded evidence. If the proof for a step is missing, it refuses and names what is missing.
- **Plans and diffs get attacked, not just approved.** Reviews come from the reviewers you configure: the host's own subagent, and outside providers (OpenAI, Gemini, DeepSeek) if you want them. Every finding is checked against the code and brought to you; you rule on each one, and Baley never acts on a finding by itself.
- **Your branch policy holds.** A hook checks every commit and push an agent attempts against the branches you protect, and a plan's file lease is enforced when a task closes.

## What it refuses to do

- Accept "done" without a recorded proof.
- Let an agent skip a step or approve its own work.
- Send anything anywhere except to the review providers you configure. No telemetry.

## How it works

- One Rust binary, run as one server per user. The host (Claude Code or Codex) talks to it as an MCP server over stdio or HTTP; a hook runs before each of the host's tool calls; a command line serves the owner.
- The binary owns the process. The model does the engineering; Baley decides every process step from the record and the settings, builds each agent's complete work order, runs the tests itself, and owns the rules about what happens next.
- Records are an append-only, hash-chained event ledger in SQLite, kept outside the repository and anchored on the forge, so a rewrite is detectable. Settings are two TOML files, one for you and one committed with the project. API keys are stored encrypted, never in a file or an environment variable.
- The whole design is written before the code: one document per process area under [docs/design](docs/design/), with the decisions in [docs/adr](docs/adr/).

## Following the work

- **What is being built:** the [milestones](https://github.com/crenshawdev/baley/milestones), each a theme of the design, and the build issues under them.
- **How it is designed:** every significant change starts as a design document with requirements and diagrams, and each architectural decision is kept as a decision record. See [the design process](docs/design/README.md), [design documents](docs/design/) and [decision records](docs/adr/).
- **How it is built:** every change reaches `main` through a pull request with passing CI (tests, clippy, cargo-deny), and signed commits are required.

## Issues and contributions

Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

## Sponsoring

Baley is open source, built by one person, and sold to no one. If it is useful to you, [GitHub Sponsors](https://github.com/sponsors/crenshawdev) is the way to support it.

## Lineage

Baley grew out of [Cadence](https://github.com/crenshawdev/cadence). History before the `baley-start` tag is Cadence's.

## License

[MIT](LICENSE)
