# Baley

Baley keeps AI coding agents accountable to the person who answers for their work. You approve the plan, the agents do the work, and Baley records what they did and the proof that it works. It refuses to let the work move forward on a claim nobody has proven.

> **Status: designed, now being built. Not ready to use.** The repository is public so the work can be followed as it happens. Nothing here is released. The binary crate holds the inherited code until later builds replace it. Where today's code still differs from the design is listed under [Current state](#current-state).

## Trust

Baley collects nothing about you, sends no telemetry, and sends data only to services you configure. Because that guarantee depends on the operating system underneath, the same standard decides which platforms Baley supports. Linux and macOS meet it. Windows fails it on privacy and on security and is not supported. A supported platform that stops meeting the standard loses support. The criteria and reasoning are in [TRUST.md](TRUST.md).

**What leaves your machine.** Anchors: when you run `baley anchor`, a git tag holding the ledger's latest sequence number and hash is pushed to the remote `git.remote` names, and `baley verify`, `baley doctor` and `baley acknowledge-restore` read it back from there. Downloads from the update source you name: `install.sh` from the source you pass it, `baley update` whenever you run it, and a daily check only if you set `updates.auto=true`. Model lists: `baley models update` prints a `curl` command for each provider you acknowledged, and you run it with the key in your environment. Outside reviews, once Build 4 adds them: requests to the providers you choose, sent by your Claude Code session with your key. Nothing else.

## What it is designed to do

- **You approve before anything runs.** Work is a backlog of stories, each with the acceptance criteria you agreed, committed to phases. A phase's plan is written, checked and approved by you before any agent acts on it.
- **Every step leaves evidence.** Which agent did what, against which commit, which tests ran and what they printed, which verdict was reached and why.
- **The next step has to be earned.** Baley decides what may happen next from the recorded evidence. If the proof for a step is missing, it refuses and names what is missing.
- **Plans and diffs get attacked, not just approved.** Reviews come from the reviewers you configure: the host's own subagent, and outside providers (OpenAI, DeepSeek) if you want them. Every finding is checked against the code and brought to you; you rule on each one, and Baley never acts on a finding by itself.
- **Your branch policy holds.** A hook checks every commit and push an agent attempts against the branches you protect, and a plan's file lease is enforced when a task closes.

## What it is designed to refuse

- Accept "done" without a recorded proof.
- Let an agent skip a step or approve its own work.
- Read, store or send your API keys, or call a model itself. The session sends outside reviews. No telemetry.

## Current state

- Installs use unsigned development artifacts. `install.sh` and `baley update` check SHA-256 against an unsigned manifest, and signature verification comes before any release ([#14](https://github.com/jcrenshaw-dev/baley/issues/14)). The design signs each release's manifests with an OpenSSH Ed25519 key that stays on the maintainer's machine, and both will verify that signature against the public key they carry.
- The parked review engine is still compiled in. No command or tool reaches it, but it reads environment keys and `providers.env` and can call providers. Build 9 ([#30](https://github.com/jcrenshaw-dev/baley/issues/30)) deletes it.
- `baley models update` prints a fetch only for an acknowledged provider. The same check before a review, including for hand-edited settings, comes with Build 4 ([#25](https://github.com/jcrenshaw-dev/baley/issues/25)).
- Outside reviews of plans and diffs are not built yet. Build 4 ([#25](https://github.com/jcrenshaw-dev/baley/issues/25)) adds them, along with phase planning and plan approval.
- A plan's file lease is not yet enforced when a task closes. Build 5 ([#26](https://github.com/jcrenshaw-dev/baley/issues/26)) enforces it.
- Anchors are pushed only when you run `baley anchor`. Build 6 ([#27](https://github.com/jcrenshaw-dev/baley/issues/27)) anchors automatically.
- macOS support is written but has not been run or qualified, and CI runs on Linux only ([#24](https://github.com/jcrenshaw-dev/baley/issues/24)).

The [roadmap](docs/roadmap.md) carries everything else, build by build.

## How it works

- One Rust binary. Each Claude Code session starts it as an MCP server over stdio; a hook runs before each of its tool calls; a command line serves the owner.
- The binary owns the process. The model does the engineering; Baley decides every process step from the record and the settings, builds each agent's complete work order, runs the tests itself, and owns the rules about what happens next.
- Records are an append-only, hash-chained event ledger in SQLite, kept outside the repository and anchored on the forge, so a rewrite is detectable. Settings are two TOML files, one for you and one committed with the project. The design keeps API keys in your environment; Baley builds a request, the session sends it and returns the raw response, and Baley parses and checks it.
- The whole design is written before the code: one document per process area under [docs/design](docs/design/), with the decisions in [docs/adr](docs/adr/).

## Hosts and platforms

Claude Code only. Linux is the platform it has run on, and its sandbox needs `bwrap` and `socat`. macOS support is written but not yet run or qualified.

Codex support has been removed because its sandboxing and execution controls do not meet Baley's requirements. Support may return when those requirements are met. Baley will not lower its security defaults to accommodate a host.

## Installing and updating

`install.sh` downloads the binary from the source you pass it, places it behind `~/.local/bin/baley` and runs `baley install`, which writes Claude Code's wiring. Each checkout then needs `baley init`. Install keeps your own settings, refuses to change anything it cannot prove it wrote, and prints a receipt that says whether the install is complete or partial.

Updates are opt-in and off by default, and there is no default source. Name one with `baley config set --global updates.source=https://...`. `baley update` checks for a newer version and installs it whenever you run it. With `baley config set --global updates.auto=true`, `baley serve` also checks at most once a day, without waiting for the result. A new version is staged beside the old one: new sessions start it, and running sessions keep the version they started with.

The details, including what install writes and when it refuses, are in [docs/install.md](docs/install.md).

## Outside reviews and your API keys

Keys live in your environment, where every program Claude Code starts can see them, agents and subagents included. Nothing scrubs a key a command prints. Your code goes to the chosen provider under its terms.

A fresh install enables no outside provider. `baley config interview` adds one only after you accept that warning, and Build 4 ([#25](https://github.com/jcrenshaw-dev/baley/issues/25)) adds the same check before every review, including for hand-edited settings. The details are in [docs/outside-reviews.md](docs/outside-reviews.md).

## How it is designed and built

- **What is being built:** the [milestones](https://github.com/jcrenshaw-dev/baley/milestones), each a theme of the design, and the build issues under them.
- **In what order:** the [roadmap](docs/roadmap.md), from the build under way to the first release, updated by each build pull request.
- **How it is designed:** every significant change starts as a design document with requirements and diagrams, and each architectural decision is kept as a decision record. See [Baley: product requirements](docs/design/prd/baley.md), [0002: System design](docs/design/0002-system-design.md), [the design process](docs/design/README.md), [design documents](docs/design/), [decision records](docs/adr/) and [the glossary](CONTEXT.md), which gives one term for each concept.
- **How it is built:** every change reaches `main` through a pull request with passing required checks (`cargo-test`, `clippy`, `cargo-deny`, `fmt`).
- **Talk about the work:** join the project's [Discord server](https://jcrenshaw.dev/discord).

## Issues and contributions

Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

## Sponsoring

Baley is open source, built by one person, and sold to no one. If it is useful to you, [GitHub Sponsors](https://github.com/sponsors/crenshawdev) is the way to support it.

## Lineage

Baley grew out of [Cadence](https://github.com/crenshawdev/cadence). History before the `baley-start` tag is Cadence's.

## License

[MIT](LICENSE)
