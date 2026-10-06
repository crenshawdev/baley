# 0038: Install with one command and update only by choice

| | |
|---|---|
| Status | Accepted |
| Date | 2026-10-06 |
| Deciders | John Crenshaw |
| Design document | [0012: Host interface](../design/0012-host-interface.md), [0015: Repository upkeep and the build gate](../design/0015-repository-upkeep-and-build-gate.md) |
| Supersedes | [0009](0009-served-instructions.md), in part: the plugin or `baley init` writes the stubs |
| Superseded by | |

## Context and problem

Claude Code starts one Baley server per session over stdio ([ADR 0034](0034-one-server-per-session.md)). A machine also needs an MCP registration, a pre-tool hook, instruction stubs, sandbox settings and file-tool deny rules. Those parts must agree on the binary and on the folders they protect. A binary download alone does not install Baley.

[ADR 0009](0009-served-instructions.md) leaves stub placement to a plugin or `baley init`. [Design 0012](../design/0012-host-interface.md) gives that work to `baley install`. A plugin cannot carry all the sandbox and permission settings needed by the [host security bar](0033-host-security-bar.md). Delivery needs one owner action and one writer of the complete configuration.

An update must preserve a running session's binary and the guard's bounded response time. A release archive's checksum alone does not authenticate its publisher, and a provenance attestation is not a signature the installed binary can verify by itself.

## Decision drivers

- One command installs the binary and all host wiring.
- No package runtime or plugin is needed to run a Rust binary.
- Updates are the owner's choice and never spend the guard's time budget.
- Downloads are authenticated before activation.
- A running session keeps the version it started with.

## Considered options

1. An installer script, with the binary writing all wiring
2. An npm package that launches the platform binary
3. A Claude Code plugin carrying registration, hooks and stubs

## Decision

Chosen option: **1**. The owner runs one installer command. The script downloads a release, verifies its signature and checksum, places the binary behind the stable path `~/.local/bin/baley`, and invokes `baley install`. There is no npm dependency and no Claude Code plugin.

**Wiring.** `baley install` writes the user-level MCP entry, the pre-tool hook, the stubs, the sandbox configuration and the `Read` and `Edit` deny rules. Every executable reference uses the same absolute stable path. It preserves separately owned registrations and settings, including Cadence's. It resolves Baley's folders once for the server, hook and deny paths and records what it installed. `baley doctor` checks the installed result. There is no session-start hook.

**Setup.** Install takes the settings interview's defaults. The interview remains available through `baley config interview`. Outside providers remain disabled until the owner chooses them and types the risk acknowledgement required by [ADR 0039](0039-session-owned-provider-credentials.md). Install never supplies that confirmation. Missing system sandbox prerequisites are reported with the fix, and the receipt names the need for a new Claude Code session and per-checkout `baley init`.

**Updates.** Automatic updates exist and are off by default. Once enabled, `baley serve` starts a detached process that checks at most once a day across sessions. Neither that process nor its network waits run inside the guard hook or its budget. A download's signature and checksum are verified before a new version is staged beside the old one behind the stable path. Activation applies to new sessions only. Existing sessions keep their version, and the old binary remains available to them. Stub and hook activation must preserve that same boundary. A manual `baley update` is always available and uses the same verification and activation rules.

Release verification uses a signed checksum manifest. Verification probably needs a new dependency. The signature format and verification dependency are not selected here. A build task must name any new dependency before adding it.

## Consequences

### Positive

- One command produces the whole installation, with one binary responsible for its contents.
- Stable executable paths keep host registrations valid across releases.
- Updates cannot hold up a tool call, and an owner can keep automatic network checks off.
- The same verification protects a fresh install and an update.

### Negative

- Baley must merge host settings safely and distinguish its own artifacts from somebody else's.
- Keeping old versions and activating hooks and stubs for new sessions costs more than replacing one file. Hooks start anew per call, so changing a link alone cannot establish the session boundary.
- Once opted in, update checks make network calls without a prompt for each check.
- Under ADR 0034, an old server can become read-only when a newer binary raises the ledger's compatibility epoch.

### Follow-up

- Build 3 T14 to T17 ([#24](https://github.com/crenshawdev/baley/issues/24)) implement delivery, artifact application, setup and installed qualification under HST-R12 and HST-R17. They are no longer held for a delivery choice.
- The release design ([#14](https://github.com/crenshawdev/baley/issues/14)) specifies publication and trust for the signed checksum manifest. The implementing task names its verification dependency, if any.
- T14 and T15 specify and qualify how fresh hook processes and stub reads retain the session's version during activation. A stable link alone is insufficient evidence.

## Options in detail

### Installer script and binary-written wiring (chosen)

Meets the one-command requirement without a second runtime. The binary can render and check everything the host loads, including the settings a plugin cannot supply. It owns safe merging and version activation.

### npm package

A thin package can select a platform binary, but adds Node and npm to installation and updates without removing the binary's duty to write sandbox and permission settings.

### Claude Code plugin

Can carry registration, hooks and stubs, but still needs the binary to write the remaining settings. Versioned plugin paths also complicate activation during a running session. It adds a second carrier without completing installation.
