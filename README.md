# Baley

Baley keeps AI coding agents accountable to the person who answers for their work. You approve the plan, the agents do the work, and Baley records what they did and the proof that it works. It refuses to let the work move forward on a claim nobody has proven.

> **Status: designed, now being built. Not ready to use.** The repository is public so the work can be followed as it happens. Nothing here is released. The binary crate holds the inherited code until later builds replace it.

## What it does

- **You approve before anything runs.** Work is a backlog of stories, each with the acceptance criteria you agreed, committed to phases. A phase's plan is written, checked and approved by you before any agent acts on it.
- **Every step leaves evidence.** Which agent did what, against which commit, which tests ran and what they printed, which verdict was reached and why.
- **The next step has to be earned.** Baley decides what may happen next from the recorded evidence. If the proof for a step is missing, it refuses and names what is missing.
- **Plans and diffs get attacked, not just approved.** Reviews come from the reviewers you configure: the host's own subagent, and outside providers (OpenAI, DeepSeek) if you want them. Every finding is checked against the code and brought to you; you rule on each one, and Baley never acts on a finding by itself.
- **Your branch policy holds.** A hook checks every commit and push an agent attempts against the branches you protect, and a plan's file lease is enforced when a task closes.

## What it is designed to refuse

- Accept "done" without a recorded proof.
- Let an agent skip a step or approve its own work.
- Read, store or send your API keys, or call a model itself. The session sends outside reviews. No telemetry.

## How it works

- One Rust binary. Each Claude Code session starts it as an MCP server over stdio; a hook runs before each of its tool calls; a command line serves the owner. Codex support has been removed because its sandboxing and execution controls do not meet Baley's requirements. Support may return when those requirements are met. Baley will not lower its security defaults to accommodate a host.
- The binary owns the process. The model does the engineering; Baley decides every process step from the record and the settings, builds each agent's complete work order, runs the tests itself, and owns the rules about what happens next.
- Records are an append-only, hash-chained event ledger in SQLite, kept outside the repository and anchored on the forge, so a rewrite is detectable. Settings are two TOML files, one for you and one committed with the project. The design keeps API keys in your environment; Baley builds a request, the session sends it and returns the raw response, and Baley parses and checks it.
- The whole design is written before the code: one document per process area under [docs/design](docs/design/), with the decisions in [docs/adr](docs/adr/).

## Installing and updating

The development install script [`install.sh`](install.sh) and manual `baley update` are built for unsigned development artifacts only. Both check the downloaded binary's SHA-256 against an unsigned manifest before staging it behind `~/.local/bin/baley`. [#14](https://github.com/crenshawdev/baley/issues/14) adds and qualifies signature verification before any release. The script ends by running `baley install` through `~/.local/bin/baley`. Install writes Claude Code's wiring: the `baley` MCP server registration in `~/.claude.json`, made through `claude mcp add-json --scope user` so that Claude Code stays the only writer of that file, the pre-tool hook, the sandbox settings and the `Read` and `Edit` deny rules in `~/.claude/settings.json`, and one skill stub per front door under `~/.claude/skills/`. When `CLAUDE_CONFIG_DIR` is set, all three live under that folder instead. Install keeps your own settings and replaces an artifact only when it can show Baley wrote it. It refuses, and changes none of your files, when an artifact under Baley's name is not provably Baley's (`install-ownership-conflict`) or the settings file cannot be merged (`install-settings-conflict`). Every run prints a receipt that says whether the install is complete or partial. It is partial, and the script exits non-zero, when one of your settings weakens the protection, when `bwrap` or `socat` is missing on Linux, or when nothing runnable sits at `~/.local/bin/baley`. The receipt names each gap and the fix, and you run install again afterward. The sandbox settings apply to the Claude Code sessions that read that settings file, including projects Baley does not manage. No npm or plugin is needed. Install takes settings defaults and writes nothing to `config.toml`; `baley config interview` remains available later. A new Claude Code session loads the wiring, and each checkout needs `baley init`.

Automatic updates are opt-in and off by default. Enable them with `baley config set --global updates.auto=true`. At startup, `baley serve` reads that setting once and starts a detached check without waiting for it. The check runs at most once per UTC day per installation, checks the download's SHA-256 against the unsigned manifest, and stages a newer version beside the old ones. Set the development source with `baley config set --global updates.source=https://...`; there is no default source. New sessions start the new server; running servers keep their version. The hook uses the new version at its next call and must work with an older running server and its ledger state. The updater contacts the configured source without sending project content or provider credentials. The guard never checks for updates or waits for them. Manual `baley update` can run even when that day's check has already run. See [delivery and updates](docs/adr/0038-installer-and-opt-in-updates.md).

## API keys and outside-review risks

The design requires that Baley never read, store or send your API keys. You keep them in your environment, where every program Claude Code starts can see them, agents and subagents included. Nothing scrubs a key a command prints. Your code goes to the review provider you choose under its terms.

Release 1 outside reviews use provider APIs only. Baley builds the request and names the address, header and environment variable; the session sends it with your key and returns the raw response for Baley to parse and check. The same boundary applies to model-list refreshes. Provider command-line agents are outside this release.

Before an outside provider is enabled, the interview asks which providers to use, states these risks and requires a typed confirmation. It records the acknowledgement in the ledger and asks again when the warning changes. A fresh install enables no outside provider; a reinstall keeps acknowledged choices. Install never acknowledges a warning for you.

This boundary is not built yet. The current binary still contains the `keys.env` reader, `baley exec --key` and the HTTPS model lister. Build 4 (#25) removes those paths. The parked review engine also reads environment keys and `providers.env` and makes provider calls; that code goes with the parked engine in Build 9. See [the credential decision](docs/adr/0039-session-owned-provider-credentials.md) and [configuration build status](docs/design/0003-configuration-and-routing.md#11-build-status).

## Following the work

- **What is being built:** the [milestones](https://github.com/crenshawdev/baley/milestones), each a theme of the design, and the build issues under them.
- **In what order:** the [roadmap](docs/roadmap.md), from the build under way to the first release, updated by each build pull request.
- **How it is designed:** every significant change starts as a design document with requirements and diagrams, and each architectural decision is kept as a decision record. See [the design process](docs/design/README.md), [design documents](docs/design/), [decision records](docs/adr/) and [the glossary](CONTEXT.md), which gives one term for each concept.
- **How it is built:** every change reaches `main` through a pull request with passing required checks (`cargo-test`, `clippy`, `cargo-deny`, `fmt`), and signed commits are required.
- **Talk about the work:** join the project's [Discord server](https://jcrenshaw.dev/discord).

## Issues and contributions

Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

## Sponsoring

Baley is open source, built by one person, and sold to no one. If it is useful to you, [GitHub Sponsors](https://github.com/sponsors/crenshawdev) is the way to support it.

## Lineage

Baley grew out of [Cadence](https://github.com/crenshawdev/cadence). History before the `baley-start` tag is Cadence's.

## License

[MIT](LICENSE)
